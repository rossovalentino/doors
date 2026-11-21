//! Windows sections (P18): CreateFileMapping, MapViewOfFile(Ex/3),
//! UnmapViewOfFile(Ex/2) and commits and protections inside views, with
//! Windows semantics: a section's pages are shared by every view of it in
//! every process (CoreCLR's double-mapped code heap writes through one
//! view and executes through another; Chromium's processes share memory
//! through sections), pages are allocated from the page pool only when
//! committed and first mapped, and a section lives while a handle or a
//! view of it does.
//!
//! Views are paged reservations of type MEM_MAPPED; their page-table
//! entries point at the section's pages and are filled when the view is
//! mapped and whenever a page of the section is committed through any of
//! its views, so the kernel's user-copy checks and VirtualQuery see views
//! exactly as they see private memory.
use super::*;
use door_kernel::mm::{PoolPages, Protection, SectionPages};
use door_windows_abi::memory::{RESERVE_TOP_DOWN, section};

/// Sections the machine holds at once (with handles or views).
pub(super) const MAX_WIN_SECTIONS: usize = 1024;
/// Views mapped at once, all processes together.
pub(super) const MAX_WIN_VIEWS: usize = 4096;

pub(super) struct WinSection {
    key: u64,
    pages: SectionPages,
    /// The section's protection (CreateFileMapping flProtect): no view or
    /// commit may exceed it.
    protection: Protection,
    views: u32,
    /// The last handle closed: freed with its last view.
    retired: bool,
}
#[derive(Clone, Copy)]
pub(super) struct WinView {
    slot: usize,
    base: u64,
    pages: u64,
    section: usize,
    /// First section page the view maps.
    offset: u64,
    /// The protection the view was mapped with (pages committed later
    /// through another view appear with it).
    protection: Protection,
    /// Whether the handle allowed writing (a read-only handle's views never
    /// become writable).
    may_write: bool,
}

/// Writes that reach the section (copy-on-write views write their copies).
const fn shared_write(p: Protection) -> bool {
    matches!(p, Protection::ReadWrite | Protection::ExecuteReadWrite)
}
/// Whether pages of a section with `section` protection may be mapped or
/// protected with `wanted`: shared writing needs a writable section and a
/// handle with WRITE, execution an executable section; copy on write only
/// needs reading (Windows' rule for FILE_MAP_COPY).
const fn allowed(section: Protection, wanted: Protection, may_write: bool) -> bool {
    (!shared_write(wanted) || (shared_write(section) && may_write))
        && (!wanted.executable() || section.executable())
}
fn protection_of(value: u64) -> Result<(Protection, bool)> {
    u32::try_from(value)
        .ok()
        .and_then(Protection::from_windows)
        .ok_or(Status::InvalidArgument)
}

impl Vm {
    fn win_section_index(&self, key: u64) -> Result<usize> {
        self.win_sections
            .iter()
            .position(|s| s.as_ref().is_some_and(|s| s.key == key))
            .ok_or(Status::NotFound)
    }
    /// Pool pages every section holds (part of the machine's commit).
    pub(super) fn section_bytes(&self) -> u64 {
        self.section_bytes
    }
    /// CreateFileMapping's backing for executive `key`: `bytes` (page
    /// multiple), `flags` (`section::RESERVE`: SEC_RESERVE, pages committed
    /// later through views), `protect` its PAGE_* protection.
    pub fn win_section_create(
        &mut self,
        key: u64,
        bytes: u64,
        flags: u64,
        protect: u64,
    ) -> Result<()> {
        if flags & !section::RESERVE != 0 || bytes == 0 || !bytes.is_multiple_of(PAGE_BYTES) {
            return Err(Status::InvalidArgument);
        }
        let (protection, guard) = protection_of(protect)?;
        if guard || protection == Protection::NoAccess {
            return Err(Status::InvalidArgument);
        }
        let reserve = flags & section::RESERVE != 0;
        // A SEC_COMMIT section is charged in full when created, as on
        // Windows; its pages are still allocated only when mapped.
        if !reserve && bytes > windows_memory::COMMIT_LIMIT {
            return Err(Status::ResourceExhausted);
        }
        let pages =
            SectionPages::new(bytes / PAGE_BYTES, !reserve).map_err(|_| Status::InvalidArgument)?;
        if self.win_sections.iter().flatten().any(|s| s.key == key) {
            return Err(Status::AlreadyExists);
        }
        let slot = self
            .win_sections
            .iter()
            .position(Option::is_none)
            .ok_or(Status::ResourceExhausted)?;
        self.win_sections[slot] = Some(WinSection {
            key,
            pages,
            protection,
            views: 0,
            retired: false,
        });
        Ok(())
    }
    /// The executive dropped the last handle of `key`: the section goes now
    /// or with its last view.
    pub fn win_section_retire(&mut self, key: u64) -> Result<()> {
        let index = self.win_section_index(key)?;
        let section = self.win_sections[index].as_mut().expect("found section");
        section.retired = true;
        if section.views == 0 {
            self.win_section_free(index);
        }
        Ok(())
    }
    fn win_section_free(&mut self, index: usize) {
        let mut section = self.win_sections[index].take().expect("section to free");
        let mut memory = PoolPages {
            pool: &mut self.pool,
            frames: &mut self.frames,
            kernel: &mut self.tables,
        };
        // No view maps a page any more and every unmap retired its
        // translations on every CPU.
        let freed = unsafe { section.pages.release(&mut memory) }.expect("section page teardown");
        self.section_bytes = self.section_bytes.saturating_sub(freed * PAGE_BYTES);
    }
    /// Whether one more pool page fits the machine's commit limit.
    fn section_budget(&self) -> Result<()> {
        let used: u64 = self.paged_bytes.iter().sum::<u64>() + self.section_bytes;
        if used + 8 * PAGE_BYTES > windows_memory::COMMIT_LIMIT {
            return Err(Status::ResourceExhausted);
        }
        Ok(())
    }
    /// The physical page behind section page `index`, allocated on first
    /// use; `NotCommitted` (BadAddress) for a reserved page.
    fn section_page(&mut self, section: usize, index: u64) -> Result<u64> {
        let state = {
            let s = self.win_sections[section]
                .as_mut()
                .ok_or(Status::NotFound)?;
            let mut memory = PoolPages {
                pool: &mut self.pool,
                frames: &mut self.frames,
                kernel: &mut self.tables,
            };
            unsafe { s.pages.state(&mut memory, index) }.map_err(status)?
        };
        if let door_kernel::mm::PageState::Present(physical) = state {
            return Ok(physical);
        }
        self.section_budget()?;
        let s = self.win_sections[section]
            .as_mut()
            .ok_or(Status::NotFound)?;
        let before = s.pages.held_pages();
        let mut memory = PoolPages {
            pool: &mut self.pool,
            frames: &mut self.frames,
            kernel: &mut self.tables,
        };
        let physical = unsafe { s.pages.page(&mut memory, index) }.map_err(status)?;
        self.section_bytes += (s.pages.held_pages() - before) * PAGE_BYTES;
        Ok(physical)
    }
    /// MapViewOfFile: a view of `pages_key`'s section in `slot` at
    /// `address` (0: anywhere; `RESERVE_TOP_DOWN` in `flags` from the top),
    /// from byte `offset` (64 KiB multiple) for `bytes` (0: to the end),
    /// with `protect`. `may_write`: the handle grants WRITE.
    /// `section::REPLACE` maps over an empty private reservation of exactly
    /// that range (MEM_REPLACE_PLACEHOLDER). The view's base.
    #[allow(clippy::too_many_arguments)]
    pub fn win_section_map(
        &mut self,
        slot: usize,
        key: u64,
        address: u64,
        offset: u64,
        bytes: u64,
        protect: u64,
        flags: u64,
        may_write: bool,
    ) -> Result<u64> {
        // A view replacing a placeholder (an image part) may start at any
        // page of the section; others at 64 KiB multiples.
        let alignment = if flags & section::REPLACE != 0 {
            PAGE_BYTES
        } else {
            mm::GRANULARITY
        };
        if flags & !(RESERVE_TOP_DOWN | section::REPLACE | section::IMAGE) != 0
            || !offset.is_multiple_of(alignment)
        {
            return Err(Status::InvalidArgument);
        }
        let (protection, guard) = protection_of(protect)?;
        if guard {
            return Err(Status::InvalidArgument);
        }
        let index = self.win_section_index(key)?;
        let (section_pages, section_protection) = {
            let s = self.win_sections[index].as_ref().expect("found section");
            if s.retired {
                return Err(Status::NotFound);
            }
            (s.pages.pages(), s.protection)
        };
        if protection != Protection::NoAccess && !allowed(section_protection, protection, may_write)
        {
            return Err(Status::AccessDenied);
        }
        let first = offset / PAGE_BYTES;
        if first >= section_pages {
            return Err(Status::InvalidArgument);
        }
        let count = if bytes == 0 {
            section_pages - first
        } else {
            bytes.div_ceil(PAGE_BYTES)
        };
        if count == 0 || first + count > section_pages {
            return Err(Status::InvalidArgument);
        }
        let view_slot = self
            .win_views
            .iter()
            .position(Option::is_none)
            .ok_or(Status::ResourceExhausted)?;
        let base = if flags & section::REPLACE != 0 {
            // MEM_REPLACE_PLACEHOLDER: the placeholder of exactly the view's
            // range becomes the view.
            self.spaces[slot]
                .as_mut()
                .ok_or(Status::NotFound)?
                .paged_replace_placeholder(address, count * PAGE_BYTES, protect as u32, mm::MAPPED)
                .map_err(|_| Status::InvalidArgument)?;
            address
        } else {
            self.reserve_range(
                slot,
                address,
                count * PAGE_BYTES,
                flags & RESERVE_TOP_DOWN,
                protect,
                windows_memory::Reserve::View,
            )?
        };
        if flags & section::IMAGE != 0 {
            // An image section's view reads as MEM_IMAGE.
            self.spaces[slot]
                .as_mut()
                .ok_or(Status::NotFound)?
                .paged_mark_image(base)
                .map_err(status)?;
        }
        self.win_views[view_slot] = Some(WinView {
            slot,
            base,
            pages: count,
            section: index,
            offset: first,
            protection,
            may_write,
        });
        self.win_sections[index]
            .as_mut()
            .expect("found section")
            .views += 1;
        // Every committed page of the range, mapped now.
        if protection != Protection::NoAccess {
            self.ensure_tables(slot, base, count * PAGE_BYTES);
            for page in 0..count {
                let state = {
                    let s = self.win_sections[index].as_mut().expect("found section");
                    let mut memory = PoolPages {
                        pool: &mut self.pool,
                        frames: &mut self.frames,
                        kernel: &mut self.tables,
                    };
                    unsafe { s.pages.state(&mut memory, first + page) }
                };
                let result = match state {
                    Ok(door_kernel::mm::PageState::Reserved) => Ok(()),
                    Ok(_) => self.section_page(index, first + page).and_then(|physical| {
                        self.spaces[slot]
                            .as_mut()
                            .ok_or(Status::NotFound)?
                            .view_install(base + page * PAGE_BYTES, physical, protection)
                            .map_err(status)
                    }),
                    Err(error) => Err(status(error)),
                };
                if let Err(error) = result {
                    let _ = self.win_section_unmap(slot, base, 0);
                    return Err(error);
                }
            }
        }
        Ok(base)
    }
    /// UnmapViewOfFile of the view holding `address`; `section::PRESERVE`
    /// in `flags` leaves an empty placeholder reservation in its place
    /// (MEM_PRESERVE_PLACEHOLDER).
    pub fn win_section_unmap(&mut self, slot: usize, address: u64, flags: u64) -> Result<()> {
        if flags & !section::PRESERVE != 0 {
            return Err(Status::InvalidArgument);
        }
        let index = self
            .win_views
            .iter()
            .position(|v| {
                v.is_some_and(|v| {
                    v.slot == slot && address >= v.base && address < v.base + v.pages * PAGE_BYTES
                })
            })
            .ok_or(Status::InvalidArgument)?;
        let view = self.win_views[index].expect("found view");
        let space = self.spaces[slot].as_mut().ok_or(Status::NotFound)?;
        // Copies the view made (copy on write) go back with it.
        let mut pages = PoolPages {
            pool: &mut self.pool,
            frames: &mut self.frames,
            kernel: &mut self.tables,
        };
        let charge = &mut self.paged_bytes[slot];
        if flags & section::PRESERVE != 0 {
            space.view_to_placeholder(view.base, &mut pages, charge)
        } else {
            space.view_unmap(view.base, &mut pages, charge)
        }
        .map_err(status)?;
        self.win_views[index] = None;
        let s = self.win_sections[view.section]
            .as_mut()
            .expect("viewed section");
        s.views -= 1;
        if s.views == 0 && s.retired {
            self.win_section_free(view.section);
        }
        Ok(())
    }
    /// Every view of `slot` unmapped (process teardown).
    pub(super) fn win_views_drop(&mut self, slot: usize) {
        while let Some(view) = self
            .win_views
            .iter()
            .flatten()
            .find(|v| v.slot == slot)
            .copied()
        {
            self.win_section_unmap(slot, view.base, 0)
                .expect("process view teardown");
        }
    }
    /// The view of `slot` holding `address`.
    fn view_at(&self, slot: usize, address: u64) -> Option<WinView> {
        self.win_views
            .iter()
            .flatten()
            .find(|v| {
                v.slot == slot && address >= v.base && address < v.base + v.pages * PAGE_BYTES
            })
            .copied()
    }
    /// MEM_COMMIT inside a view: the section's pages of the range are
    /// committed (SEC_RESERVE) and mapped in this view with `protect` and
    /// in every other view of them with that view's protection.
    pub(super) fn view_commit(
        &mut self,
        slot: usize,
        address: u64,
        bytes: u64,
        protect: u64,
    ) -> Result<u64> {
        let (protection, guard) = protection_of(protect)?;
        if bytes == 0 || !address.is_multiple_of(PAGE_BYTES) || !bytes.is_multiple_of(PAGE_BYTES) {
            return Err(Status::InvalidArgument);
        }
        let view = self.view_at(slot, address).ok_or(Status::BadAddress)?;
        if address + bytes > view.base + view.pages * PAGE_BYTES {
            return Err(Status::BadAddress);
        }
        let section_protection = self.win_sections[view.section]
            .as_ref()
            .expect("viewed section")
            .protection;
        if guard || !allowed(section_protection, protection, view.may_write) {
            return Err(Status::AccessDenied);
        }
        let mut cursor = address;
        while cursor < address + bytes {
            let index = view.offset + (cursor - view.base) / PAGE_BYTES;
            {
                let s = self.win_sections[view.section]
                    .as_mut()
                    .expect("viewed section");
                let before = s.pages.held_pages();
                let mut memory = PoolPages {
                    pool: &mut self.pool,
                    frames: &mut self.frames,
                    kernel: &mut self.tables,
                };
                unsafe { s.pages.commit(&mut memory, index) }.map_err(status)?;
                self.section_bytes += (s.pages.held_pages() - before) * PAGE_BYTES;
            }
            let physical = self.section_page(view.section, index)?;
            // This view with the protection asked for, the others with
            // their own.
            for i in 0..MAX_WIN_VIEWS {
                let Some(other) = self.win_views[i] else {
                    continue;
                };
                if other.section != view.section
                    || index < other.offset
                    || index >= other.offset + other.pages
                    || other.protection == Protection::NoAccess && other.base != view.base
                {
                    continue;
                }
                let at = other.base + (index - other.offset) * PAGE_BYTES;
                let mine = other.slot == slot && other.base == view.base;
                let wanted = if mine { protection } else { other.protection };
                self.ensure_tables(other.slot, at, PAGE_BYTES);
                let space = self.spaces[other.slot].as_mut().ok_or(Status::NotFound)?;
                if !mine && space.query(at).is_ok_and(|q| q.state == mm::mem::COMMIT) {
                    continue;
                }
                space.view_install(at, physical, wanted).map_err(status)?;
            }
            cursor += PAGE_BYTES;
        }
        Ok(address)
    }
    /// VirtualProtect inside a view: within what the section and the
    /// handle allow; every page must be committed.
    pub(super) fn view_protect(
        &mut self,
        slot: usize,
        address: u64,
        bytes: u64,
        protect: u64,
    ) -> Result<u64> {
        let (protection, guard) = protection_of(protect)?;
        let view = self.view_at(slot, address).ok_or(Status::BadAddress)?;
        let section_protection = self.win_sections[view.section]
            .as_ref()
            .expect("viewed section")
            .protection;
        if protection != Protection::NoAccess
            && !allowed(section_protection, protection, view.may_write)
        {
            return Err(Status::AccessDenied);
        }
        self.spaces[slot]
            .as_mut()
            .ok_or(Status::NotFound)?
            .paged_protect(address, bytes, protection, guard)
            .map(u64::from)
            .map_err(|error| match error {
                mm::Error::NotCommitted | mm::Error::Address => Status::BadAddress,
                other => status(other),
            })
    }
    /// Whether `address` of `slot` is inside a section view.
    pub(super) fn in_view(&self, slot: usize, address: u64) -> bool {
        self.view_at(slot, address).is_some()
    }
}

/// Zeroes for reading section pages that were never touched.
static ZERO_PAGE: [u8; PAGE_BYTES as usize] = [0; PAGE_BYTES as usize];

impl Vm {
    /// SECTION_SNAPSHOT and SECTION_FILL on a Windows section: `bytes` of
    /// section `key` from byte `offset` copied to (`fill` false) or from
    /// (`fill` true) the user memory of `slot` at `user`, which the caller
    /// validated. Filling allocates the section's committed pages; a
    /// reserved page is BadAddress. A page never touched reads as zeroes.
    pub fn win_section_copy(
        &mut self,
        slot: usize,
        key: u64,
        offset: u64,
        user: u64,
        bytes: usize,
        fill: bool,
    ) -> Result<()> {
        let index = self.win_section_index(key)?;
        let size = self.win_sections[index]
            .as_ref()
            .expect("found section")
            .pages
            .pages()
            * PAGE_BYTES;
        if offset
            .checked_add(bytes as u64)
            .is_none_or(|end| end > size)
        {
            return Err(Status::BadAddress);
        }
        let cr3 = self.cr3(slot)?;
        let mut done = 0usize;
        while done < bytes {
            let at = offset + done as u64;
            let within = (at % PAGE_BYTES) as usize;
            let chunk = (PAGE_BYTES as usize - within).min(bytes - done);
            let user_at = user + done as u64;
            let physical = if fill {
                Some(self.section_page(index, at / PAGE_BYTES)?)
            } else {
                let s = self.win_sections[index].as_mut().expect("found section");
                let mut memory = PoolPages {
                    pool: &mut self.pool,
                    frames: &mut self.frames,
                    kernel: &mut self.tables,
                };
                match unsafe { s.pages.state(&mut memory, at / PAGE_BYTES) }.map_err(status)? {
                    door_kernel::mm::PageState::Present(physical) => Some(physical),
                    _ => None,
                }
            };
            // SAFETY: the VM guard is held; the alias maps a section page
            // the section owns until it is freed under this guard; the user
            // range was validated by the caller.
            let copied = unsafe {
                match physical {
                    Some(physical) => {
                        let alias =
                            self.pool.alias(physical).ok_or(Status::IoFailure)? + within as u64;
                        if fill {
                            arch::copy_user_to_shared(cr3, user_at, alias, chunk)
                        } else {
                            arch::copy_shared_to_user(cr3, user_at, alias, chunk)
                        }
                    }
                    None => arch::copy_to_user(cr3, user_at, &ZERO_PAGE[..chunk]),
                }
            }
            .map_err(|_| Status::BadAddress)?;
            if copied != chunk {
                return Err(Status::BadAddress);
            }
            done += chunk;
        }
        Ok(())
    }
}
