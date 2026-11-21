//! Windows dynamic memory (P18 M0b): VirtualAlloc reservations anywhere in
//! the process's window, committed, protected, decommitted and queried page
//! by page, as CoreCLR, V8 and every Win32 heap use them. Reserving costs no
//! RAM; a committed page is one page-pool page, zeroed, mapped only through
//! the process's own page tables (no standing kernel alias, no physically
//! contiguous ranges), so the limits are the RAM found at boot and the
//! process's commit budget, as on real hardware.
use super::{Backing, ProcessSpace, checked_user_range};
use crate::mm::{
    Error, FrameAllocator, PAGE_BYTES, PageTables, Permissions, UserAccess,
    page_pool::PagePool,
    reservations::{Direction, GRANULARITY, MAPPED, Reservation},
    user_table::{Fault, Page, Protection},
};

/// MEMORY_BASIC_INFORMATION's State and Type values.
pub mod mem {
    pub const COMMIT: u32 = 0x1000;
    pub const RESERVE: u32 = 0x2000;
    pub const FREE: u32 = 0x1_0000;
    pub const PRIVATE: u32 = 0x2_0000;
    pub const MAPPED: u32 = 0x4_0000;
    pub const IMAGE: u32 = 0x100_0000;
    /// PAGE_NOACCESS, a free range's protection.
    pub const NOACCESS: u32 = 0x01;
}

/// One VirtualQuery answer: a run of pages from `base` sharing state and
/// protection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Query {
    pub base: u64,
    pub allocation_base: u64,
    pub allocation_protect: u32,
    pub size: u64,
    pub state: u32,
    pub protect: u32,
    pub kind: u32,
}

const fn region_protect(p: Permissions) -> u32 {
    match p {
        Permissions::ReadOnly => 0x02,
        Permissions::ReadWrite => 0x04,
        Permissions::ReadExecute => 0x20,
    }
}
/// A page's (state, protect) for queries.
fn page_state(page: Option<Page>) -> (u32, u32) {
    match page {
        Some(p) => (mem::COMMIT, p.protection.windows(p.guard)),
        None => (mem::RESERVE, 0),
    }
}

impl ProcessSpace {
    /// The reservation holding all of `[start, start + bytes)`.
    fn paged_holding(&self, start: u64, bytes: u64) -> Result<Reservation, Error> {
        if bytes == 0 || !start.is_multiple_of(PAGE_BYTES) || !bytes.is_multiple_of(PAGE_BYTES) {
            return Err(Error::Alignment);
        }
        self.regions
            .paged()
            .holding(start, bytes)
            .ok_or(Error::Address)
    }
    /// `paged_holding` for private or image pages: a section view's pages
    /// belong to its section and are never committed or freed here, and a
    /// placeholder holds no pages at all.
    fn private_holding(&self, start: u64, bytes: u64) -> Result<Reservation, Error> {
        let r = self.paged_holding(start, bytes)?;
        if r.view || r.placeholder {
            return Err(Error::InvalidState);
        }
        Ok(r)
    }
    /// VirtualAlloc2 MEM_RESERVE_PLACEHOLDER: a placeholder reservation,
    /// placed as `paged_reserve` places one.
    pub fn paged_reserve_placeholder(
        &mut self,
        address: Option<u64>,
        bytes: u64,
        window: (u64, u64),
        direction: Direction,
    ) -> Result<u64, Error> {
        let start = self.paged_reserve(address, bytes, 0x01, window, direction)?;
        self.regions.paged_mut().at_mut(start)?.placeholder = true;
        Ok(start)
    }
    /// The placeholder holding `[start, start + bytes)` split so that range
    /// is a placeholder of its own (VirtualFree MEM_RELEASE |
    /// MEM_PRESERVE_PLACEHOLDER inside a placeholder).
    pub fn paged_split_placeholder(&mut self, start: u64, bytes: u64) -> Result<(), Error> {
        self.live()?;
        let r = self.paged_holding(start, bytes)?;
        if !r.placeholder {
            return Err(Error::InvalidState);
        }
        let table = self.regions.paged_mut();
        if start > r.start {
            table.split(r.start, (start - r.start) / PAGE_BYTES)?;
        }
        if start + bytes < r.end() {
            table.split(start, bytes / PAGE_BYTES)?;
        }
        Ok(())
    }
    /// The allocation of exactly `[start, start + bytes)` back to an empty
    /// placeholder (VirtualFree MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER on
    /// an allocation that replaced one).
    ///
    /// # Safety
    /// As `paged_commit`.
    pub unsafe fn paged_to_placeholder(
        &mut self,
        start: u64,
        bytes: u64,
        pages: &mut dyn PageSource,
        charge: &mut u64,
    ) -> Result<(), Error> {
        let r = self.private_holding(start, bytes)?;
        if r.start != start
            || r.end() != start + bytes
            || r.kind != crate::mm::reservations::PRIVATE
        {
            return Err(Error::InvalidState);
        }
        unsafe { self.paged_decommit(start, bytes, pages, charge) }?;
        let entry = self.regions.paged_mut().at_mut(start)?;
        entry.placeholder = true;
        entry.protect = 0x01;
        Ok(())
    }
    /// The placeholder of exactly `[start, start + bytes)` becomes an
    /// ordinary reservation of `kind` (MEM_PRIVATE for VirtualAlloc2
    /// MEM_REPLACE_PLACEHOLDER, MEM_MAPPED for MapViewOfFile3).
    pub fn paged_replace_placeholder(
        &mut self,
        start: u64,
        bytes: u64,
        protect: u32,
        kind: u32,
    ) -> Result<(), Error> {
        self.live()?;
        let r = self.paged_holding(start, bytes)?;
        if !r.placeholder || r.start != start || r.end() != start + bytes {
            return Err(Error::InvalidState);
        }
        let entry = self.regions.paged_mut().at_mut(start)?;
        entry.placeholder = false;
        entry.protect = protect;
        entry.kind = kind;
        // MapViewOfFile3 makes a view of it, VirtualAlloc2 an allocation.
        entry.view = kind == MAPPED;
        Ok(())
    }
    /// MEM_COALESCE_PLACEHOLDERS over exactly `[start, start + bytes)`.
    pub fn paged_coalesce_placeholders(&mut self, start: u64, bytes: u64) -> Result<(), Error> {
        self.live()?;
        let end = start.checked_add(bytes).ok_or(Error::Overflow)?;
        self.regions.paged_mut().coalesce(start, end)
    }
    /// Where a region overlaps `[start, end)`: the end of the last one.
    fn regions_over(&self, start: u64, end: u64) -> Option<u64> {
        self.regions
            .iter()
            .flatten()
            .filter(|r| start < r.start + r.pages * PAGE_BYTES && r.start < end)
            .map(|r| r.start + r.pages * PAGE_BYTES)
            .max()
    }
    /// MEM_RESERVE of `bytes` (page multiple) at `address` (64 KiB aligned)
    /// or, without one, the first free range of `window` in `direction`.
    pub fn paged_reserve(
        &mut self,
        address: Option<u64>,
        bytes: u64,
        protect: u32,
        window: (u64, u64),
        direction: Direction,
    ) -> Result<u64, Error> {
        self.live()?;
        if bytes == 0 || !bytes.is_multiple_of(PAGE_BYTES) {
            return Err(Error::Alignment);
        }
        let start = match address {
            Some(start) => {
                checked_user_range(start, bytes)?;
                if self.regions_over(start, start + bytes).is_some() {
                    return Err(Error::Overlap);
                }
                start
            }
            None => self
                .regions
                .paged()
                .free_range(window.0, window.1, bytes, direction, &|s, e| {
                    self.regions_over(s, e)
                })
                .ok_or(Error::Exhausted)?,
        };
        if !start.is_multiple_of(GRANULARITY) {
            return Err(Error::Alignment);
        }
        self.regions
            .paged_mut()
            .insert(start, bytes / PAGE_BYTES, protect)?;
        Ok(start)
    }
    /// MEM_COMMIT: every page of `[start, start + bytes)` committed with
    /// `protection` (pages already committed keep their contents and take
    /// the new protection). New pages count against `charge`, which may not
    /// pass `budget`. A failure leaves no page newly committed. With `demand`
    /// (VirtualAlloc, as Windows commits) a new page takes no frame until it
    /// is first touched (`paged_materialize`, `Fault::Demand`); without it
    /// (images the kernel fills at once) every new page gets its frame now.
    ///
    /// # Safety
    /// The VM guard and retirement contract hold; `pages` is the VM
    /// owner's page supply.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn paged_commit(
        &mut self,
        start: u64,
        bytes: u64,
        protection: Protection,
        guard: bool,
        demand: bool,
        pages: &mut dyn PageSource,
        charge: &mut u64,
        budget: u64,
    ) -> Result<(), Error> {
        self.live()?;
        self.private_holding(start, bytes)?;
        let end = start + bytes;
        let mut fresh = 0u64;
        let mut address = start;
        while address < end {
            if self.table.page(address)?.is_none() {
                fresh += 1;
            }
            address += PAGE_BYTES;
        }
        let after = charge
            .checked_add(fresh * PAGE_BYTES)
            .filter(|total| *total <= budget)
            .ok_or(Error::Exhausted)?;
        // New pages first (each marked fresh); existing pages are only
        // re-protected once every new page is in place, so a failure undoes
        // exactly this call's pages and changes nothing else.
        let mut address = start;
        let mut result = Ok(());
        while address < end {
            if self.table.page(address)?.is_none() {
                let step = if demand {
                    self.table.commit_demand(address, protection, guard)
                } else {
                    match unsafe { pages.take() } {
                        Ok(physical) => {
                            let committed =
                                self.table.commit_page(address, physical, protection, guard);
                            if committed.is_err() {
                                let _ = unsafe { pages.give(physical) };
                            }
                            committed
                        }
                        Err(error) => Err(error),
                    }
                };
                if let Err(error) = step {
                    result = Err(error);
                    break;
                }
            }
            address += PAGE_BYTES;
        }
        if let Err(error) = result {
            let mut undo = start;
            while undo < address {
                if let Ok(Some(physical)) = self.table.take_fresh(undo) {
                    let _ = unsafe { pages.give(physical) };
                }
                undo += PAGE_BYTES;
            }
            let _ = self.table.reclaim_tables(start, end);
            return Err(error);
        }
        let mut address = start;
        while address < end {
            if self
                .table
                .page(address)?
                .is_some_and(|p| p.protection != protection || p.guard != guard)
            {
                self.table.protect_page(address, protection, guard)?;
            }
            self.table.settle(address)?;
            address += PAGE_BYTES;
        }
        *charge = after;
        Ok(())
    }
    /// MEM_DECOMMIT: the pages of `[start, start + bytes)` back to the pool
    /// (reserved pages stay reserved). Their translations are retired on
    /// every CPU before a page is reused.
    ///
    /// # Safety
    /// As `paged_commit`.
    pub unsafe fn paged_decommit(
        &mut self,
        start: u64,
        bytes: u64,
        pages: &mut dyn PageSource,
        charge: &mut u64,
    ) -> Result<(), Error> {
        self.live()?;
        self.private_holding(start, bytes)?;
        let end = start + bytes;
        let mut address = start;
        while address < end {
            if let Some(page) = self.table.decommit_page(address)? {
                if !page.demand {
                    unsafe { pages.give(page.physical) }?;
                }
                *charge = charge.saturating_sub(PAGE_BYTES);
            }
            address += PAGE_BYTES;
        }
        self.table.reclaim_tables(start, end)
    }
    /// Frames for every demand page of `[start, start + len)` that lies in a
    /// paged reservation (the kernel about to read or write user memory, as
    /// a user touch would supply them). The commit charge was taken at
    /// commit; only physical pages are taken here.
    ///
    /// # Safety
    /// As `paged_commit`.
    pub unsafe fn paged_materialize(
        &mut self,
        start: u64,
        len: u64,
        pages: &mut dyn PageSource,
    ) -> Result<(), Error> {
        self.live()?;
        if len == 0 {
            return Ok(());
        }
        let end = start.checked_add(len).ok_or(Error::Overflow)?;
        let mut page = start & !(PAGE_BYTES - 1);
        while page < end {
            if self.regions.paged().find(page).is_some()
                && self.table.page(page)?.is_some_and(|p| p.demand)
            {
                let physical = unsafe { pages.take() }?;
                if let Err(error) = self.table.materialize(page, physical) {
                    let _ = unsafe { pages.give(physical) };
                    return Err(error);
                }
            }
            page += PAGE_BYTES;
        }
        Ok(())
    }
    /// VirtualProtect on committed pages: every page must be committed
    /// (nothing changes otherwise). Returns the first page's old
    /// protection, as Windows does.
    pub fn paged_protect(
        &mut self,
        start: u64,
        bytes: u64,
        protection: Protection,
        guard: bool,
    ) -> Result<u32, Error> {
        self.live()?;
        self.paged_holding(start, bytes)?;
        let end = start + bytes;
        let mut address = start;
        let mut old = None;
        while address < end {
            let page = self.table.page(address)?.ok_or(Error::NotCommitted)?;
            old.get_or_insert(page.protection.windows(page.guard));
            address += PAGE_BYTES;
        }
        let mut address = start;
        while address < end {
            self.table.protect_page(address, protection, guard)?;
            address += PAGE_BYTES;
        }
        old.ok_or(Error::Empty)
    }
    /// MEM_RELEASE of the reservation starting at `start`.
    ///
    /// # Safety
    /// As `paged_commit`.
    pub unsafe fn paged_release(
        &mut self,
        start: u64,
        pages: &mut dyn PageSource,
        charge: &mut u64,
    ) -> Result<(), Error> {
        self.live()?;
        let r = self
            .regions
            .paged()
            .find(start)
            .filter(|r| r.start == start)
            .ok_or(Error::Address)?;
        if r.view {
            // UnmapViewOfFile, not VirtualFree, ends a view.
            return Err(Error::InvalidState);
        }
        if !r.placeholder {
            unsafe { self.paged_decommit(r.start, r.pages * PAGE_BYTES, pages, charge) }?;
        }
        self.regions.paged_mut().remove(start).map(|_| ())
    }
    /// Every reservation released (process teardown, before `destroy`).
    /// Section views go too; their pages stay with their sections (the VM
    /// owner drops its view records first).
    ///
    /// # Safety
    /// As `paged_commit`; no thread of the process runs.
    pub unsafe fn paged_release_all(
        &mut self,
        pages: &mut dyn PageSource,
        charge: &mut u64,
    ) -> Result<(), Error> {
        loop {
            let next = self.regions.paged().iter().next();
            let Some(r) = next else { break };
            if r.view {
                self.view_unmap(r.start, pages, charge)?;
            } else {
                unsafe { self.paged_release(r.start, pages, charge) }?;
            }
        }
        Ok(())
    }
    /// A reservation for a section view (MEM_MAPPED), placed as
    /// `paged_reserve` places one; nothing is mapped yet.
    pub fn paged_reserve_view(
        &mut self,
        address: Option<u64>,
        bytes: u64,
        protect: u32,
        window: (u64, u64),
        direction: Direction,
    ) -> Result<u64, Error> {
        let start = self.paged_reserve(address, bytes, protect, window, direction)?;
        let entry = self.regions.paged_mut().at_mut(start)?;
        entry.kind = MAPPED;
        entry.view = true;
        Ok(start)
    }
    /// Map the section page `physical` at `address` of a view with
    /// `protection`; a page already mapped there takes `protection`.
    pub fn view_install(
        &mut self,
        address: u64,
        physical: u64,
        protection: Protection,
    ) -> Result<(), Error> {
        self.live()?;
        let r = self.paged_holding(address, PAGE_BYTES)?;
        if !r.view {
            return Err(Error::InvalidState);
        }
        match self.table.page(address)? {
            Some(page) if page.physical == physical => {
                if page.protection != protection || page.guard {
                    self.table.protect_page(address, protection, false)?;
                }
                Ok(())
            }
            Some(_) => Err(Error::InvalidState),
            None => {
                self.table
                    .commit_page(address, physical, protection, false)?;
                self.table.settle(address)
            }
        }
    }
    /// The translations of the view based at `start` removed (retired on
    /// every CPU): section pages stay with the section, copies the view
    /// owns (copy on write) go back to `pages` and off `charge`.
    fn view_clear(
        &mut self,
        start: u64,
        pages: &mut dyn PageSource,
        charge: &mut u64,
    ) -> Result<Reservation, Error> {
        self.live()?;
        let r = self
            .regions
            .paged()
            .find(start)
            .filter(|r| r.start == start && r.view)
            .ok_or(Error::Address)?;
        let end = r.end();
        let mut address = start;
        while address < end {
            let (page, next) = self.table.page_or_gap(address)?;
            if let Some(page) = page {
                self.table.decommit_page(address)?;
                if page.owned {
                    // SAFETY: the translation is gone and retired.
                    unsafe { pages.give(page.physical) }?;
                    *charge = charge.saturating_sub(PAGE_BYTES);
                }
            }
            address = next.min(end);
        }
        self.table.reclaim_tables(start, end)?;
        Ok(r)
    }
    /// Remove the view based at `start` (UnmapViewOfFile): its
    /// translations and its reservation. The view's size in pages.
    pub fn view_unmap(
        &mut self,
        start: u64,
        pages: &mut dyn PageSource,
        charge: &mut u64,
    ) -> Result<u64, Error> {
        let r = self.view_clear(start, pages, charge)?;
        self.regions.paged_mut().remove(start)?;
        Ok(r.pages)
    }
    /// UnmapViewOfFileEx MEM_PRESERVE_PLACEHOLDER: the view based at
    /// `start` becomes an empty placeholder of the same range.
    pub fn view_to_placeholder(
        &mut self,
        start: u64,
        pages: &mut dyn PageSource,
        charge: &mut u64,
    ) -> Result<u64, Error> {
        let r = self.view_clear(start, pages, charge)?;
        let entry = self.regions.paged_mut().at_mut(start)?;
        entry.placeholder = true;
        entry.kind = crate::mm::reservations::PRIVATE;
        entry.view = false;
        entry.protect = 0x01;
        Ok(r.pages)
    }
    /// A user page fault at `address`: an EXECUTE_READWRITE page changing
    /// sides, a guard page's one-shot fault, a write to a WRITECOPY view
    /// page (`paged_copy_on_write`), or a real violation (also for
    /// addresses outside every reservation).
    pub fn paged_fault(&mut self, address: u64, access: UserAccess) -> Result<Fault, Error> {
        self.live()?;
        if self.regions.paged().find(address).is_none() {
            return Ok(Fault::Violation);
        }
        self.table.page_fault(address, access)
    }
    /// Copy on write at `address` (a WRITECOPY or EXECUTE_WRITECOPY view
    /// page): a new page from `memory` with the old one's contents, owned by
    /// the view and charged to `charge` (within `budget`).
    ///
    /// # Safety
    /// As `paged_commit`; `memory` sees every page involved (the pool).
    pub unsafe fn paged_copy_on_write(
        &mut self,
        address: u64,
        memory: &mut dyn crate::mm::NodeMemory,
        charge: &mut u64,
        budget: u64,
    ) -> Result<(), Error> {
        self.live()?;
        let address = address & !(PAGE_BYTES - 1);
        let page = self.table.page(address)?.ok_or(Error::NotCommitted)?;
        if !matches!(
            page.protection,
            Protection::WriteCopy | Protection::ExecuteWriteCopy
        ) {
            return Err(Error::InvalidState);
        }
        let after = charge
            .checked_add(PAGE_BYTES)
            .filter(|total| *total <= budget)
            .ok_or(Error::Exhausted)?;
        let copy = unsafe { memory.take() }?;
        let copied = (|| {
            let from = memory.words(page.physical)?;
            let to = memory.words(copy)?;
            // SAFETY: two distinct pool pages seen through kernel aliases.
            unsafe { core::ptr::copy_nonoverlapping(from, to, 512) };
            self.table.copy_page(address, copy).map(|_| ())
        })();
        if let Err(error) = copied {
            let _ = unsafe { memory.give(copy) };
            return Err(error);
        }
        *charge = after;
        Ok(())
    }
    /// Before the kernel writes into user memory: EXECUTE_READWRITE pages
    /// of the range currently executable become writable (the next fetch
    /// flips them back), so ReadFile into a JIT buffer works as on Windows,
    /// and WRITECOPY view pages are copied as a user write would copy them.
    ///
    /// # Safety
    /// As `paged_copy_on_write`.
    pub unsafe fn paged_prepare_write(
        &mut self,
        address: u64,
        length: u64,
        memory: &mut dyn crate::mm::NodeMemory,
        charge: &mut u64,
        budget: u64,
    ) -> Result<(), Error> {
        self.live()?;
        if length == 0 {
            return Ok(());
        }
        // Demand pages get their frames first (a write touches them).
        unsafe { self.paged_materialize(address, length, memory) }?;
        let end = address.checked_add(length).ok_or(Error::Overflow)?;
        let mut page = address & !(PAGE_BYTES - 1);
        while page < end {
            if self.regions.paged().find(page).is_some()
                && let Some(p) = self.table.page(page)?
                && !p.guard
            {
                match p.protection {
                    Protection::ExecuteReadWrite
                        if self.table.translate(page, UserAccess::Write).is_err() =>
                    {
                        self.table.page_fault(page, UserAccess::Write)?;
                    }
                    Protection::WriteCopy | Protection::ExecuteWriteCopy => {
                        unsafe { self.paged_copy_on_write(page, memory, charge, budget) }?
                    }
                    _ => {}
                }
            }
            page += PAGE_BYTES;
        }
        Ok(())
    }
    /// VirtualQuery at `address`.
    pub fn query(&self, address: u64) -> Result<Query, Error> {
        self.live()?;
        checked_user_range(address, 1)?;
        let base = address & !(PAGE_BYTES - 1);
        if let Some(r) = self.regions.paged().find(base) {
            let first = page_state(self.table.page(base)?);
            let mut cursor = base;
            while cursor < r.end() {
                let (page, next) = self.table.page_or_gap(cursor)?;
                if page_state(page) != first {
                    break;
                }
                cursor = next.min(r.end());
            }
            return Ok(Query {
                base,
                allocation_base: r.start,
                allocation_protect: r.protect,
                size: cursor - base,
                state: first.0,
                protect: first.1,
                kind: r.kind,
            });
        }
        if let Some(r) = self
            .regions
            .iter()
            .flatten()
            .find(|r| base >= r.start && base < r.start + r.pages * PAGE_BYTES)
        {
            let protect = region_protect(r.permissions);
            let committed = r.backing.is_some();
            return Ok(Query {
                base,
                allocation_base: r.start,
                allocation_protect: protect,
                size: r.start + r.pages * PAGE_BYTES - base,
                state: if committed { mem::COMMIT } else { mem::RESERVE },
                protect: if committed { protect } else { 0 },
                kind: if matches!(r.backing, Some(Backing::Shared { .. })) {
                    mem::MAPPED
                } else {
                    mem::PRIVATE
                },
            });
        }
        // Free: up to the next region or reservation.
        let next_region = self
            .regions
            .iter()
            .flatten()
            .map(|r| r.start)
            .filter(|s| *s > base)
            .min();
        let next_reserved = self.regions.paged().next_at_or_after(base).map(|r| r.start);
        let end = [next_region, next_reserved, Some(super::USER_END)]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(super::USER_END);
        Ok(Query {
            base,
            allocation_base: 0,
            allocation_protect: 0,
            size: end - base,
            state: mem::FREE,
            protect: mem::NOACCESS,
            kind: 0,
        })
    }
    /// Mark the reservation based at `start` as a mapped image (ntdll's
    /// loader reserves a DLL, then this).
    pub fn paged_mark_image(&mut self, start: u64) -> Result<(), Error> {
        self.live()?;
        self.regions
            .paged_mut()
            .set_kind(start, crate::mm::reservations::IMAGE)
    }
    /// An image reservation (MEM_IMAGE) over `[start, start + bytes)` for a
    /// mapped executable or DLL; its pages are committed per section.
    pub fn paged_reserve_image(&mut self, start: u64, bytes: u64) -> Result<(), Error> {
        self.live()?;
        if bytes == 0 || !bytes.is_multiple_of(PAGE_BYTES) || !start.is_multiple_of(GRANULARITY) {
            return Err(Error::Alignment);
        }
        checked_user_range(start, bytes)?;
        if self.regions_over(start, start + bytes).is_some() {
            return Err(Error::Overlap);
        }
        // AllocationProtect of an image: PAGE_EXECUTE_WRITECOPY.
        self.regions.paged_mut().insert_kind(
            start,
            bytes / PAGE_BYTES,
            0x80,
            crate::mm::reservations::IMAGE,
        )
    }
    /// Copy `bytes` into committed pages from `address` on, whatever their
    /// protection (the kernel filling an image before it protects it).
    ///
    /// # Safety
    /// The VM guard is held; `pool` owns every committed page.
    pub unsafe fn paged_write(
        &self,
        address: u64,
        bytes: &[u8],
        pool: &PagePool,
    ) -> Result<(), Error> {
        self.live()?;
        let mut done = 0usize;
        while done < bytes.len() {
            let at = address.checked_add(done as u64).ok_or(Error::Overflow)?;
            let page = at & !(PAGE_BYTES - 1);
            let offset = (at - page) as usize;
            let chunk = (PAGE_BYTES as usize - offset).min(bytes.len() - done);
            let found = self.table.page(page)?.ok_or(Error::NotCommitted)?;
            // Images are committed with frames; a demand page cannot be filled
            // through the pool without one.
            if found.demand {
                return Err(Error::NotCommitted);
            }
            let alias = pool.alias(found.physical).ok_or(Error::InvalidState)?;
            // SAFETY: the alias maps this committed pool page RW in the kernel.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    bytes[done..].as_ptr(),
                    (alias + offset as u64) as *mut u8,
                    chunk,
                )
            };
            done += chunk;
        }
        Ok(())
    }
    /// Copy committed memory from `address` on into `out` (the kernel
    /// reading an image it mapped, such as a TLS template after fixups).
    ///
    /// # Safety
    /// As `paged_write`.
    pub unsafe fn paged_read(
        &self,
        address: u64,
        out: &mut [u8],
        pool: &PagePool,
    ) -> Result<(), Error> {
        self.live()?;
        let mut done = 0usize;
        while done < out.len() {
            let at = address.checked_add(done as u64).ok_or(Error::Overflow)?;
            let page = at & !(PAGE_BYTES - 1);
            let offset = (at - page) as usize;
            let chunk = (PAGE_BYTES as usize - offset).min(out.len() - done);
            let found = self.table.page(page)?.ok_or(Error::NotCommitted)?;
            if found.demand {
                // Never touched: it reads as zeroes.
                out[done..done + chunk].fill(0);
                done += chunk;
                continue;
            }
            let alias = pool.alias(found.physical).ok_or(Error::InvalidState)?;
            // SAFETY: the alias maps this committed pool page in the kernel.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    (alias + offset as u64) as *const u8,
                    out[done..].as_mut_ptr(),
                    chunk,
                )
            };
            done += chunk;
        }
        Ok(())
    }
    /// The reservation holding `address`.
    pub fn paged_reservation(&self, address: u64) -> Option<Reservation> {
        self.regions.paged().find(address)
    }
    /// Reservations held (diagnostics, tests).
    pub fn paged_reservations(&self) -> usize {
        self.regions.paged().len()
    }
}

/// Where committed pages come from and go back to: the VM owner's page pool
/// in the kernel (`PoolPages`), a counting model in host tests.
pub trait PageSource {
    /// A zeroed page nobody else maps: its physical address.
    ///
    /// # Safety
    /// The VM guard is held.
    unsafe fn take(&mut self) -> Result<u64, Error>;
    /// Give back a page from `take`.
    ///
    /// # Safety
    /// No mapping of the page remains and its translations are retired on
    /// every CPU.
    unsafe fn give(&mut self, physical: u64) -> Result<(), Error>;
}
/// The kernel's page supply: the page pool over the frame allocator and the
/// kernel's page tables.
pub struct PoolPages<'p, 'f, 'k> {
    pub pool: &'p mut PagePool,
    pub frames: &'p mut FrameAllocator<'f>,
    pub kernel: &'p mut PageTables<'k>,
}
impl PageSource for PoolPages<'_, '_, '_> {
    unsafe fn take(&mut self) -> Result<u64, Error> {
        unsafe { self.pool.allocate(self.frames, self.kernel) }
    }
    unsafe fn give(&mut self, physical: u64) -> Result<(), Error> {
        unsafe { self.pool.free(physical, self.frames, self.kernel) }
    }
}
impl crate::mm::NodeMemory for PoolPages<'_, '_, '_> {
    fn words(&mut self, physical: u64) -> Result<*mut u64, Error> {
        self.pool
            .alias(physical)
            .map(|alias| alias as *mut u64)
            .ok_or(Error::InvalidState)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::super::tests::model;
    use super::*;
    use std::vec::Vec;
    /// Hands out fake physical pages; fails after `limit`.
    struct Fake {
        next: u64,
        limit: usize,
        out: Vec<u64>,
    }
    impl Fake {
        fn new(limit: usize) -> Self {
            Self {
                next: 0x4000_0000,
                limit,
                out: Vec::new(),
            }
        }
    }
    impl PageSource for Fake {
        unsafe fn take(&mut self) -> Result<u64, Error> {
            if self.out.len() >= self.limit {
                return Err(Error::Exhausted);
            }
            self.next += PAGE_BYTES;
            self.out.push(self.next);
            Ok(self.next)
        }
        unsafe fn give(&mut self, physical: u64) -> Result<(), Error> {
            let i = self
                .out
                .iter()
                .position(|p| *p == physical)
                .ok_or(Error::InvalidFree)?;
            self.out.remove(i);
            Ok(())
        }
    }
    const WINDOW: (u64, u64) = (0x1_0000_0000, 0x7FF0_0000_0000);
    const P: u64 = PAGE_BYTES;
    #[test]
    fn reserve_commit_protect_query_release() {
        let (_t, mut s) = model(1);
        let mut pages = Fake::new(1000);
        let mut charge = 0;
        // CoreCLR-sized reservation: free, costs nothing.
        let base = s
            .paged_reserve(None, 1 << 36, 0x01, WINDOW, Direction::BottomUp)
            .unwrap();
        assert_eq!(base, WINDOW.0);
        assert_eq!(pages.out.len(), 0);
        unsafe {
            s.paged_commit(
                base + 16 * P,
                4 * P,
                Protection::ReadWrite,
                false,
                false,
                &mut pages,
                &mut charge,
                u64::MAX,
            )
        }
        .unwrap();
        assert_eq!((pages.out.len(), charge), (4, 4 * P));
        assert!(s.translate(base + 17 * P, UserAccess::Write).is_ok());
        // Queries: reserved run, committed run, reserved tail.
        let q = s.query(base).unwrap();
        assert_eq!(
            (q.state, q.size, q.allocation_base, q.kind),
            (mem::RESERVE, 16 * P, base, mem::PRIVATE)
        );
        let q = s.query(base + 16 * P).unwrap();
        assert_eq!((q.state, q.protect, q.size), (mem::COMMIT, 0x04, 4 * P));
        let q = s.query(base + 20 * P).unwrap();
        assert_eq!((q.state, q.size), (mem::RESERVE, (1 << 36) - 20 * P));
        // JIT: write, then make executable.
        assert_eq!(
            s.paged_protect(base + 16 * P, 2 * P, Protection::ExecuteRead, false),
            Ok(0x04)
        );
        assert!(s.translate(base + 16 * P, UserAccess::Execute).is_ok());
        assert!(s.translate(base + 16 * P, UserAccess::Write).is_err());
        assert_eq!(s.query(base + 16 * P).unwrap().size, 2 * P);
        // Protecting a range with an uncommitted page changes nothing.
        assert_eq!(
            s.paged_protect(base + 19 * P, 2 * P, Protection::ReadOnly, false),
            Err(Error::NotCommitted)
        );
        assert_eq!(s.query(base + 19 * P).unwrap().protect, 0x04);
        // Release returns every page and the charge.
        unsafe { s.paged_release(base, &mut pages, &mut charge) }.unwrap();
        assert_eq!((pages.out.len(), charge, s.paged_reservations()), (0, 0, 0));
        assert_eq!(s.query(base).unwrap().state, mem::FREE);
    }
    #[test]
    fn demand_commits_charge_now_and_take_frames_on_touch() {
        let (_t, mut s) = model(1);
        let mut pages = Fake::new(10);
        let mut charge = 0;
        let base = s
            .paged_reserve(None, 64 * P, 0x04, WINDOW, Direction::BottomUp)
            .unwrap();
        // More than the supply: committing takes the charge, not frames.
        unsafe {
            s.paged_commit(
                base,
                32 * P,
                Protection::ReadWrite,
                false,
                true,
                &mut pages,
                &mut charge,
                u64::MAX,
            )
        }
        .unwrap();
        assert_eq!((pages.out.len(), charge), (0, 32 * P));
        assert_eq!(s.query(base).unwrap().state, mem::COMMIT);
        // A touch (or a kernel copy) supplies frames for its pages only.
        unsafe { s.paged_materialize(base + 3 * P + 5, 2 * P, &mut pages) }.unwrap();
        assert_eq!(pages.out.len(), 3);
        assert!(s.translate(base + 4 * P, UserAccess::Write).is_ok());
        assert!(s.translate(base, UserAccess::Read).is_err());
        // Decommit returns the frames and the whole charge.
        unsafe { s.paged_decommit(base, 32 * P, &mut pages, &mut charge) }.unwrap();
        assert_eq!((pages.out.len(), charge), (0, 0));
        // The budget still bounds the charge.
        let r = unsafe {
            s.paged_commit(
                base,
                32 * P,
                Protection::ReadWrite,
                false,
                true,
                &mut pages,
                &mut charge,
                16 * P,
            )
        };
        assert_eq!(r, Err(Error::Exhausted));
    }
    #[test]
    fn a_failed_commit_takes_back_only_its_own_pages() {
        let (_t, mut s) = model(1);
        let mut pages = Fake::new(3);
        let mut charge = 0;
        let base = s
            .paged_reserve(None, 16 * P, 0x04, WINDOW, Direction::BottomUp)
            .unwrap();
        unsafe {
            s.paged_commit(
                base,
                P,
                Protection::ReadOnly,
                false,
                false,
                &mut pages,
                &mut charge,
                u64::MAX,
            )
        }
        .unwrap();
        // Four pages wanted, three new ones needed, only two left.
        let r = unsafe {
            s.paged_commit(
                base,
                4 * P,
                Protection::ReadWrite,
                false,
                false,
                &mut pages,
                &mut charge,
                u64::MAX,
            )
        };
        assert_eq!(r, Err(Error::Exhausted));
        assert_eq!((pages.out.len(), charge), (1, P));
        // The page committed before keeps its protection.
        assert_eq!(s.query(base).unwrap().protect, 0x02);
        assert_eq!(s.query(base + P).unwrap().state, mem::RESERVE);
        // The budget is checked before anything changes.
        let r = unsafe {
            s.paged_commit(
                base + 4 * P,
                2 * P,
                Protection::ReadWrite,
                false,
                false,
                &mut pages,
                &mut charge,
                2 * P,
            )
        };
        assert_eq!(r, Err(Error::Exhausted));
        assert_eq!(pages.out.len(), 1);
    }
    #[test]
    fn rwx_pages_toggle_and_guard_pages_fire_once() {
        let (_t, mut s) = model(1);
        let mut pages = Fake::new(16);
        let mut charge = 0;
        let base = s
            .paged_reserve(None, 16 * P, 0x40, WINDOW, Direction::TopDown)
            .unwrap();
        assert_eq!(base, (WINDOW.1 - 16 * P) & !(GRANULARITY - 1));
        unsafe {
            s.paged_commit(
                base,
                2 * P,
                Protection::ExecuteReadWrite,
                false,
                false,
                &mut pages,
                &mut charge,
                u64::MAX,
            )
        }
        .unwrap();
        assert_eq!(s.paged_fault(base, UserAccess::Execute), Ok(Fault::Toggled));
        // The kernel writing into it (ReadFile) flips it back.
        let mut memory = crate::mm::section_pages::tests::Memory::new(4);
        unsafe { s.paged_prepare_write(base, 8, &mut memory, &mut charge, u64::MAX) }.unwrap();
        assert!(s.translate(base, UserAccess::Write).is_ok());
        // A guard page faults once, then is an ordinary page.
        unsafe {
            s.paged_commit(
                base + 2 * P,
                P,
                Protection::ReadWrite,
                true,
                false,
                &mut pages,
                &mut charge,
                u64::MAX,
            )
        }
        .unwrap();
        assert_eq!(s.query(base + 2 * P).unwrap().protect, 0x104);
        assert_eq!(
            s.paged_fault(base + 2 * P, UserAccess::Read),
            Ok(Fault::Guard)
        );
        assert_eq!(
            s.paged_fault(base + 2 * P, UserAccess::Read),
            Ok(Fault::Violation)
        );
        assert_eq!(
            s.paged_fault(base + 9 * P, UserAccess::Read),
            Ok(Fault::Violation)
        );
        assert_eq!(
            s.paged_fault(0x1234_0000, UserAccess::Read),
            Ok(Fault::Violation)
        );
        unsafe { s.paged_release_all(&mut pages, &mut charge) }.unwrap();
        assert_eq!(pages.out.len(), 0);
    }
    #[test]
    fn section_views_map_shared_pages_and_never_free_them() {
        let (_t, mut s) = model(1);
        let mut pages = Fake::new(16);
        let mut charge = 0;
        let view = s
            .paged_reserve_view(None, 4 * P, 0x04, WINDOW, Direction::BottomUp)
            .unwrap();
        // A section page (not from this process's supply).
        s.view_install(view + P, 0x9000_0000, Protection::ReadWrite)
            .unwrap();
        assert!(s.translate(view + P, UserAccess::Write).is_ok());
        let q = s.query(view + P).unwrap();
        assert_eq!(
            (q.kind, q.state, q.protect),
            (mem::MAPPED, mem::COMMIT, 0x04)
        );
        assert_eq!(s.query(view).unwrap().state, mem::RESERVE);
        // Re-installing re-protects; another page there is refused.
        s.view_install(view + P, 0x9000_0000, Protection::ReadOnly)
            .unwrap();
        assert!(s.translate(view + P, UserAccess::Write).is_err());
        assert_eq!(
            s.view_install(view + P, 0x9000_1000, Protection::ReadOnly),
            Err(Error::InvalidState)
        );
        // Private operations do not touch views.
        let r = unsafe {
            s.paged_commit(
                view,
                P,
                Protection::ReadWrite,
                false,
                false,
                &mut pages,
                &mut charge,
                u64::MAX,
            )
        };
        assert_eq!(r, Err(Error::InvalidState));
        assert_eq!(
            unsafe { s.paged_decommit(view, P, &mut pages, &mut charge) },
            Err(Error::InvalidState)
        );
        assert_eq!(
            unsafe { s.paged_release(view, &mut pages, &mut charge) },
            Err(Error::InvalidState)
        );
        // A private reservation is not a view.
        let private = s
            .paged_reserve(None, P, 0x04, WINDOW, Direction::BottomUp)
            .unwrap();
        assert_eq!(
            s.view_install(private, 0x9000_0000, Protection::ReadOnly),
            Err(Error::InvalidState)
        );
        assert_eq!(
            s.view_unmap(private, &mut pages, &mut charge),
            Err(Error::Address)
        );
        assert_eq!(s.view_unmap(view, &mut pages, &mut charge), Ok(4));
        assert_eq!(s.query(view).unwrap().state, mem::FREE);
        // Teardown unmaps views without giving their pages to the supply.
        let again = s
            .paged_reserve_view(None, P, 0x02, WINDOW, Direction::TopDown)
            .unwrap();
        s.view_install(again, 0x9000_0000, Protection::ReadOnly)
            .unwrap();
        unsafe { s.paged_release_all(&mut pages, &mut charge) }.unwrap();
        assert_eq!((pages.out.len(), s.paged_reservations()), (0, 0));
    }
    #[test]
    fn writecopy_views_copy_privately_and_free_their_copies() {
        let (_t, mut s) = model(1);
        let mut memory = crate::mm::section_pages::tests::Memory::new(8);
        let mut charge = 0;
        // The section's page, holding data.
        let shared = unsafe { memory.take() }.unwrap();
        memory.pages.get_mut(&shared).unwrap()[0] = 0x1234;
        let view = s
            .paged_reserve_view(None, 2 * P, 0x08, WINDOW, Direction::BottomUp)
            .unwrap();
        s.view_install(view, shared, Protection::WriteCopy).unwrap();
        assert_eq!(
            s.paged_fault(view, UserAccess::Write),
            Ok(Fault::CopyOnWrite)
        );
        // The budget is checked first.
        assert_eq!(
            unsafe { s.paged_copy_on_write(view, &mut memory, &mut charge, 0) },
            Err(Error::Exhausted)
        );
        unsafe { s.paged_copy_on_write(view + 8, &mut memory, &mut charge, u64::MAX) }.unwrap();
        let copy = s.translate(view, UserAccess::Write).unwrap().get() & !(P - 1);
        assert_ne!(copy, shared);
        assert_eq!(memory.pages[&copy][0], 0x1234);
        assert_eq!((charge, s.query(view).unwrap().protect), (P, 0x04));
        // A kernel write copies too, and a copied page is left alone.
        s.view_install(view + P, shared, Protection::WriteCopy)
            .unwrap();
        unsafe { s.paged_prepare_write(view, 2 * P, &mut memory, &mut charge, u64::MAX) }.unwrap();
        assert_eq!(charge, 2 * P);
        // Unmapping frees the copies, never the section's page.
        let mut held = 3;
        assert_eq!(memory.pages.len(), held);
        s.view_unmap(view, &mut memory, &mut charge).unwrap();
        held -= 2;
        assert_eq!((memory.pages.len(), charge), (held, 0));
        assert!(memory.pages.contains_key(&shared));
    }
    #[test]
    fn placeholders_split_replace_and_return() {
        let (_t, mut s) = model(1);
        let mut pages = Fake::new(16);
        let mut charge = 0;
        let base = s
            .paged_reserve_placeholder(None, 16 * P, WINDOW, Direction::BottomUp)
            .unwrap();
        // Nothing is committed in a placeholder.
        let r = unsafe {
            s.paged_commit(
                base,
                P,
                Protection::ReadWrite,
                false,
                false,
                &mut pages,
                &mut charge,
                u64::MAX,
            )
        };
        assert_eq!(r, Err(Error::InvalidState));
        // Image parts: headers, then a view, then the rest.
        s.paged_split_placeholder(base, P).unwrap();
        s.paged_split_placeholder(base + P, 3 * P).unwrap();
        assert_eq!(s.paged_reservations(), 3);
        s.paged_replace_placeholder(base, P, 0x04, crate::mm::reservations::PRIVATE)
            .unwrap();
        unsafe {
            s.paged_commit(
                base,
                P,
                Protection::ReadWrite,
                false,
                false,
                &mut pages,
                &mut charge,
                u64::MAX,
            )
        }
        .unwrap();
        s.paged_replace_placeholder(base + P, 3 * P, 0x20, MAPPED)
            .unwrap();
        s.view_install(base + P, 0x9000_0000, Protection::ExecuteRead)
            .unwrap();
        assert_eq!(s.query(base + P).unwrap().kind, mem::MAPPED);
        // A view must replace a placeholder of exactly its size.
        assert_eq!(
            s.paged_replace_placeholder(base + 4 * P, 2 * P, 0x04, MAPPED),
            Err(Error::InvalidState)
        );
        // Back to placeholders, then one again.
        unsafe { s.paged_to_placeholder(base, P, &mut pages, &mut charge) }.unwrap();
        s.view_to_placeholder(base + P, &mut pages, &mut charge)
            .unwrap();
        assert_eq!((pages.out.len(), charge), (0, 0));
        s.paged_coalesce_placeholders(base, 16 * P).unwrap();
        assert_eq!(s.paged_reservations(), 1);
        unsafe { s.paged_release(base, &mut pages, &mut charge) }.unwrap();
        assert_eq!(s.query(base).unwrap().state, mem::FREE);
    }
    #[test]
    fn reservations_avoid_regions_and_regions_avoid_reservations() {
        let (_t, mut s) = model(1);
        // An image region at the bottom of the window.
        s.reserve(WINDOW.0, crate::mm::PageCount::new(32).unwrap())
            .unwrap();
        let base = s
            .paged_reserve(None, 16 * P, 0x04, WINDOW, Direction::BottomUp)
            .unwrap();
        assert_eq!(base, WINDOW.0 + 2 * GRANULARITY);
        assert_eq!(
            s.paged_reserve(Some(WINDOW.0), 16 * P, 0x04, WINDOW, Direction::BottomUp),
            Err(Error::Overlap)
        );
        assert_eq!(
            s.reserve(base, crate::mm::PageCount::new(1).unwrap())
                .map(|_| ()),
            Err(Error::Overlap)
        );
    }
}
