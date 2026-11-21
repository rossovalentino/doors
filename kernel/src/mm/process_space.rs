//! Bounded native process reservations, private page ownership and shared views.
use super::{
    Error, FrameAllocator, MappedRegion, PAGE_BYTES, PageCount, PageTables, Permissions,
    PhysicalAddress, VirtualAddress,
    reservations::Reservations,
    user_table::{MAX_TABLE_CHUNKS, TABLE_PAGES, UserTable},
};

mod paged;
use core::{
    ptr::NonNull,
    sync::atomic::{AtomicU64, Ordering},
};
pub use paged::{PageSource, PoolPages, Query, mem};

pub const USER_MIN: u64 = 0x1_0000;
pub const USER_END: u64 = 0x8000_0000_0000;
/// Regions per process: a Windows GUI program maps every section of a dozen
/// DLLs, its stacks, heap segments and window sections.
pub const MAX_REGIONS: usize = 160;
const MAX_REGION_PAGES: u64 = 65536;
static NEXT_SPACE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserAccess {
    Read,
    Write,
    Execute,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegionId {
    space: u64,
    generation: u64,
    slot: usize,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegionInfo {
    pub start: u64,
    pub pages: u64,
    pub permissions: Option<Permissions>,
    pub physical: Option<u64>,
    pub kernel_alias: Option<u64>,
    pub shared: bool,
}
enum Backing {
    Private(MappedRegion),
    Shared {
        physical: u64,
        alias: u64,
        maximum: Permissions,
    },
}
impl Backing {
    fn physical(&self) -> u64 {
        match self {
            Self::Private(mapping) => mapping.physical().expect("owned RAM mapping").get(),
            Self::Shared { physical, .. } => *physical,
        }
    }
    fn alias(&self) -> u64 {
        match self {
            Self::Private(mapping) => mapping.start().get(),
            Self::Shared { alias, .. } => *alias,
        }
    }
}
struct Region {
    start: u64,
    pages: u64,
    generation: u64,
    permissions: Permissions,
    backing: Option<Backing>,
}
impl Region {
    fn info(&self) -> RegionInfo {
        RegionInfo {
            start: self.start,
            pages: self.pages,
            permissions: self.backing.as_ref().map(|_| self.permissions),
            physical: self.backing.as_ref().map(Backing::physical),
            kernel_alias: self.backing.as_ref().map(Backing::alias),
            shared: matches!(self.backing, Some(Backing::Shared { .. })),
        }
    }
}

/// Owns every private frame until explicit acknowledged teardown. Drop retains
/// memory; it cannot race a still-running context by implicitly releasing RAM.
/// Callers serialize the whole address space, including validation followed by
/// user copy, under one VM guard. No mutation may race execution in that range.
pub struct ProcessSpace {
    identity: u64,
    generation: u64,
    table: UserTable,
    pool: Option<MappedRegion>,
    regions: Regions,
}
/// A process's region records. They live in the VM owner's static storage
/// (one table per process slot), not in ProcessSpace, which moves by value
/// through kernel stacks.
pub struct RegionTable {
    regions: [Option<Region>; MAX_REGIONS],
    /// Windows dynamic-memory reservations (P18 M0b, `paged`).
    paged: Reservations,
    /// The page-table chunks added after the first (`ensure_tables`): their
    /// kernel mappings, released with the space.
    table_chunks: [Option<MappedRegion>; MAX_TABLE_CHUNKS - 1],
}
impl RegionTable {
    pub const EMPTY: Self = Self {
        regions: [const { None }; MAX_REGIONS],
        paged: Reservations::EMPTY,
        table_chunks: [const { None }; MAX_TABLE_CHUNKS - 1],
    };
    /// Initialize the table at `at` in place, never as a value on the
    /// stack (it is tens of KiB; the boot stack is 128 KiB).
    ///
    /// # Safety
    /// `at` is valid, aligned storage the caller owns exclusively.
    pub unsafe fn initialize_at(at: *mut Self) {
        unsafe {
            for i in 0..MAX_REGIONS {
                core::ptr::addr_of_mut!((*at).regions)
                    .cast::<Option<Region>>()
                    .add(i)
                    .write(None);
            }
            // All-zero is an empty reservation table.
            core::ptr::write_bytes(core::ptr::addr_of_mut!((*at).paged), 0, 1);
            for i in 0..MAX_TABLE_CHUNKS - 1 {
                core::ptr::addr_of_mut!((*at).table_chunks)
                    .cast::<Option<MappedRegion>>()
                    .add(i)
                    .write(None);
            }
        }
    }
}
struct Regions(NonNull<RegionTable>);
impl core::ops::Deref for Regions {
    type Target = [Option<Region>; MAX_REGIONS];
    fn deref(&self) -> &Self::Target {
        // SAFETY: the table outlives the space, which owns it exclusively.
        unsafe { &self.0.as_ref().regions }
    }
}
impl core::ops::DerefMut for Regions {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: as above; &mut self is the only access.
        unsafe { &mut self.0.as_mut().regions }
    }
}
impl Regions {
    fn paged(&self) -> &Reservations {
        // SAFETY: as for Deref.
        unsafe { &self.0.as_ref().paged }
    }
    fn paged_mut(&mut self) -> &mut Reservations {
        // SAFETY: as for DerefMut.
        unsafe { &mut self.0.as_mut().paged }
    }
    fn table_chunk_count(&self) -> usize {
        // SAFETY: as for Deref.
        unsafe { self.0.as_ref().table_chunks.iter().flatten().count() }
    }
    fn table_chunks(&mut self) -> &mut [Option<MappedRegion>; MAX_TABLE_CHUNKS - 1] {
        // SAFETY: as for DerefMut.
        unsafe { &mut self.0.as_mut().table_chunks }
    }
}
// SAFETY: a space and its table move together under the VM guard.
unsafe impl Send for Regions {}
impl ProcessSpace {
    /// Private committed backing only; no shared views, reservations or addresses.
    pub fn private_bytes(&self) -> u64 {
        self.regions
            .iter()
            .flatten()
            .filter(|r| matches!(r.backing, Some(Backing::Private(_))))
            .map(|r| r.pages * PAGE_BYTES)
            .sum()
    }

    /// # Safety
    /// Called under the exclusive VM guard, with IRQs disabled and trusted kernel
    /// CR3 active. The allocator and kernel tables are the sole authorities.
    /// `retire(cr3,address)` synchronously retires non-global user translations
    /// and paging-structure caches on every CPU which could retain them. Address
    /// zero requests whole-root retirement. Timeout must never return. PCID is
    /// disabled; trap entry and return reload CR3. This contract also applies to
    /// all later mutations. New kernel mappings stay under shared upper entries.
    pub unsafe fn new(
        frames: &mut FrameAllocator<'_>,
        kernel: &mut PageTables<'_>,
        table_alias: VirtualAddress,
        retire: fn(u64, u64),
        regions: NonNull<RegionTable>,
    ) -> Result<Self, Error> {
        let identity = NEXT_SPACE
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Error::Exhausted)?;
        let owned = frames.allocate(PageCount::new(TABLE_PAGES as u64)?, 1)?;
        let physical = owned.start().get();
        let pool = match unsafe { kernel.map_frames(owned, table_alias, Permissions::ReadWrite) } {
            Ok(mapping) => mapping,
            Err(failure) => {
                frames.free(failure.frames)?;
                return Err(failure.error);
            }
        };
        let base = NonNull::new(table_alias.get() as *mut u64).ok_or(Error::Address)?;
        if let Err(error) = unsafe { kernel.copy_kernel_root(base.as_ptr()) } {
            unsafe { release_mapping(pool, frames, kernel) };
            return Err(error);
        }
        Ok(Self {
            identity,
            generation: 1,
            table: unsafe { UserTable::new(base, physical, retire) },
            pool: Some(pool),
            regions: {
                let mut r = Regions(regions);
                // Records a destroyed space left behind (already released).
                for slot in r.iter_mut() {
                    *slot = None;
                }
                *r.paged_mut() = Reservations::EMPTY;
                r
            },
        })
    }
    /// Page-table chunks added after the first.
    pub fn extra_table_chunks(&self) -> usize {
        self.regions.table_chunk_count()
    }
    /// The kernel window extra table chunks take (after the first chunk).
    pub const TABLE_WINDOW_BYTES: u64 =
        (MAX_TABLE_CHUNKS as u64 - 1) * TABLE_PAGES as u64 * PAGE_BYTES;
    /// At least `needed` free page-table pages: chunks of TABLE_PAGES are
    /// added (1 MiB of frames each, mapped at `window` + chunk * 1 MiB in
    /// the space's kernel window) until there are, or every chunk slot is
    /// used, adding at most `allowed` chunks (the machine-wide budget left).
    /// Callers ask before an operation that may create tables (a commit, a
    /// view page, an image).
    ///
    /// # Safety
    /// As `new`: the VM guard is exclusive and `window` is this space's
    /// own unmapped range of `TABLE_WINDOW_BYTES`.
    pub unsafe fn ensure_tables(
        &mut self,
        needed: usize,
        frames: &mut FrameAllocator<'_>,
        kernel: &mut PageTables<'_>,
        window: VirtualAddress,
        mut allowed: usize,
    ) -> Result<(), Error> {
        self.live()?;
        while self.table.free_pages() < needed {
            if allowed == 0 {
                return Err(Error::Exhausted);
            }
            allowed -= 1;
            let Some(slot) = self.regions.table_chunks().iter().position(Option::is_none) else {
                return Err(Error::Exhausted);
            };
            let alias =
                VirtualAddress::new(window.get() + slot as u64 * TABLE_PAGES as u64 * PAGE_BYTES)?;
            let owned = frames.allocate(PageCount::new(TABLE_PAGES as u64)?, 1)?;
            let physical = owned.start().get();
            let mapping = match unsafe { kernel.map_frames(owned, alias, Permissions::ReadWrite) } {
                Ok(mapping) => mapping,
                Err(failure) => {
                    frames.free(failure.frames)?;
                    return Err(failure.error);
                }
            };
            let base = NonNull::new(alias.get() as *mut u64).ok_or(Error::Address)?;
            if self.table.add_chunk(base, physical).is_none() {
                unsafe { release_mapping(mapping, frames, kernel) };
                return Err(Error::Exhausted);
            }
            self.regions.table_chunks()[slot] = Some(mapping);
        }
        Ok(())
    }
    pub fn cr3(&self) -> Result<u64, Error> {
        self.live()?;
        Ok(self.table.root())
    }
    pub fn used_table_pages(&self) -> u32 {
        self.table.used_pages()
    }
    fn live(&self) -> Result<(), Error> {
        if self.pool.is_some() {
            Ok(())
        } else {
            Err(Error::InvalidState)
        }
    }
    fn index(&self, id: RegionId) -> Result<usize, Error> {
        self.live()?;
        if id.space != self.identity
            || self
                .regions
                .get(id.slot)
                .and_then(Option::as_ref)
                .is_none_or(|r| r.generation != id.generation)
        {
            return Err(Error::InvalidMapping);
        }
        Ok(id.slot)
    }
    pub fn get(&self, id: RegionId) -> Result<RegionInfo, Error> {
        Ok(self.regions[self.index(id)?]
            .as_ref()
            .expect("checked region")
            .info())
    }
    pub fn region_at(&self, address: u64) -> Option<RegionId> {
        self.regions.iter().enumerate().find_map(|(slot, entry)| {
            let r = entry.as_ref()?;
            (address >= r.start && address < r.start + r.pages * PAGE_BYTES).then_some(RegionId {
                space: self.identity,
                generation: r.generation,
                slot,
            })
        })
    }
    pub fn has_shared_backing(&self, physical: u64) -> bool {
        self.regions.iter().flatten().any(|region| {
            matches!(&region.backing, Some(Backing::Shared { physical: start, .. }) if *start == physical)
        })
    }
    /// Reserve a complete page-aligned extent without exposing any physical RAM.
    /// Guards remain unreserved/uncommitted holes chosen by process policy.
    pub fn reserve(&mut self, start: u64, pages: PageCount) -> Result<RegionId, Error> {
        self.live()?;
        if !start.is_multiple_of(PAGE_BYTES) {
            return Err(Error::Alignment);
        }
        checked_user_range(start, pages.bytes().get())?;
        if pages.get() > MAX_REGION_PAGES {
            return Err(Error::MappingLimit);
        }
        let end = start + pages.bytes().get();
        if self
            .regions
            .iter()
            .flatten()
            .any(|r| start < r.start + r.pages * PAGE_BYTES && r.start < end)
        {
            return Err(Error::Overlap);
        }
        // Nor over a Windows reservation (P18 M0b).
        if self.regions.paged().overlaps(start, end) {
            return Err(Error::Overlap);
        }
        let slot = self
            .regions
            .iter()
            .position(Option::is_none)
            .ok_or(Error::MappingLimit)?;
        let next = self.generation.checked_add(1).ok_or(Error::Exhausted)?;
        let id = RegionId {
            space: self.identity,
            generation: self.generation,
            slot,
        };
        self.regions[slot] = Some(Region {
            start,
            pages: pages.get(),
            generation: self.generation,
            permissions: Permissions::ReadOnly,
            backing: None,
        });
        self.generation = next;
        Ok(id)
    }
    pub fn release(&mut self, id: RegionId) -> Result<(), Error> {
        let index = self.index(id)?;
        if self.regions[index]
            .as_ref()
            .expect("checked region")
            .backing
            .is_some()
        {
            return Err(Error::InvalidState);
        }
        self.regions[index] = None;
        Ok(())
    }
    /// Commit the entire reservation; allocation, zero fill and initial bytes are
    /// complete before any user PTE is published. Failed commits retain only the
    /// original empty reservation. RX user pages have a RO/NX kernel alias.
    ///
    /// # Safety
    /// The constructor's VM/IRQ/retirement contract holds. `alias` is exclusively
    /// reserved in the shared supervisor arena. Initial bytes are immutable and
    /// do not alias this new allocation. No process can run in the changed range.
    pub unsafe fn commit(
        &mut self,
        id: RegionId,
        frames: &mut FrameAllocator<'_>,
        kernel: &mut PageTables<'_>,
        alias: VirtualAddress,
        permissions: Permissions,
        initial: &[u8],
    ) -> Result<(), Error> {
        let index = self.index(id)?;
        let r = self.regions[index].as_ref().expect("checked region");
        if r.backing.is_some() {
            return Err(Error::InvalidState);
        }
        if initial.len() as u64 > r.pages * PAGE_BYTES {
            return Err(Error::Address);
        }
        let (start, pages) = (r.start, r.pages);
        let owned = frames.allocate(PageCount::new(pages)?, 1)?;
        let mut mapping = match unsafe { kernel.map_frames(owned, alias, Permissions::ReadWrite) } {
            Ok(mapping) => mapping,
            Err(failure) => {
                frames.free(failure.frames)?;
                return Err(failure.error);
            }
        };
        // SAFETY: newly owned, zeroed and still user-inaccessible RAM.
        unsafe {
            core::ptr::copy_nonoverlapping(initial.as_ptr(), alias.get() as *mut u8, initial.len())
        };
        if permissions != Permissions::ReadWrite
            && let Err(error) = unsafe { kernel.protect(&mut mapping, Permissions::ReadOnly) }
        {
            unsafe { release_mapping(mapping, frames, kernel) };
            return Err(error);
        }
        if let Err(error) = self.table.map(
            start,
            mapping.physical().expect("RAM").get(),
            pages,
            permissions,
        ) {
            unsafe { release_mapping(mapping, frames, kernel) };
            return Err(error);
        }
        let r = self.regions[index].as_mut().expect("checked region");
        r.backing = Some(Backing::Private(mapping));
        r.permissions = permissions;
        Ok(())
    }
    /// Map externally owned section RAM. Shared executable sections are outside
    /// this initial policy; RW/RO views preserve physical W^X across aliases.
    ///
    /// # Safety
    /// The VM guard and retirement contract hold. The section owner authorizes
    /// `maximum`, retains `backing` and its frames until every view is removed,
    /// and counts this new view atomically with publication. Backing is normal
    /// RAM, never executable through any alias. Failed publication adds no view.
    pub unsafe fn map_shared(
        &mut self,
        id: RegionId,
        backing: &MappedRegion,
        maximum: Permissions,
        permissions: Permissions,
    ) -> Result<(), Error> {
        let index = self.index(id)?;
        let r = self.regions[index].as_ref().expect("checked region");
        if r.backing.is_some() {
            return Err(Error::InvalidState);
        }
        if permissions == Permissions::ReadExecute
            || maximum == Permissions::ReadExecute
            || (permissions == Permissions::ReadWrite && maximum != Permissions::ReadWrite)
        {
            return Err(Error::AccessDenied);
        }
        if r.pages != backing.pages().get() {
            return Err(Error::Address);
        }
        let physical = backing.physical().ok_or(Error::InvalidMapping)?.get();
        self.table.map(r.start, physical, r.pages, permissions)?;
        let r = self.regions[index].as_mut().expect("checked region");
        r.backing = Some(Backing::Shared {
            physical,
            alias: backing.start().get(),
            maximum,
        });
        r.permissions = permissions;
        Ok(())
    }
    /// # Safety
    /// Same VM guard/IRQ-off/retirement contract as commit. No thread can execute
    /// or access the changed extent concurrently, including through its alias.
    pub unsafe fn protect(
        &mut self,
        id: RegionId,
        kernel: &mut PageTables<'_>,
        permissions: Permissions,
    ) -> Result<(), Error> {
        let index = self.index(id)?;
        let r = self.regions[index].as_mut().expect("checked region");
        let backing = r.backing.as_mut().ok_or(Error::NotCommitted)?;
        self.table
            .validate_mapping(r.start, backing.physical(), r.pages, r.permissions)?;
        match backing {
            Backing::Shared { maximum, .. } => {
                if permissions == Permissions::ReadExecute
                    || (permissions == Permissions::ReadWrite && *maximum != Permissions::ReadWrite)
                {
                    return Err(Error::AccessDenied);
                }
            }
            Backing::Private(mapping) => {
                // Remove execute everywhere before making any alias writable.
                if permissions == Permissions::ReadWrite {
                    self.table.protect(r.start, r.pages, Permissions::ReadOnly);
                }
                let alias_permissions = if permissions == Permissions::ReadWrite {
                    Permissions::ReadWrite
                } else {
                    Permissions::ReadOnly
                };
                if let Err(error) = unsafe { kernel.protect(mapping, alias_permissions) } {
                    self.table.protect(r.start, r.pages, r.permissions);
                    return Err(error);
                }
            }
        }
        self.table.protect(r.start, r.pages, permissions);
        r.permissions = permissions;
        Ok(())
    }
    /// Retire all leaves before releasing private backing. Shared backing stays
    /// with its section owner; caller may decrement the view count on success.
    ///
    /// # Safety
    /// VM guard and retirement contracts hold, with no live access or pending
    /// copy to this range. Its executing contexts are quiesced. No scheduler
    /// lock may be held while waiting for remote retirement acknowledgements.
    pub unsafe fn decommit(
        &mut self,
        id: RegionId,
        frames: &mut FrameAllocator<'_>,
        kernel: &mut PageTables<'_>,
    ) -> Result<(), Error> {
        let index = self.index(id)?;
        let r = self.regions[index].as_mut().expect("checked region");
        let backing = r.backing.as_ref().ok_or(Error::NotCommitted)?;
        self.table
            .validate_mapping(r.start, backing.physical(), r.pages, r.permissions)?;
        self.table.remove(r.start, r.pages);
        if let Some(Backing::Private(mapping)) = r.backing.take() {
            unsafe { release_mapping(mapping, frames, kernel) };
        }
        Ok(())
    }
    pub fn translate(&self, address: u64, access: UserAccess) -> Result<PhysicalAddress, Error> {
        self.live()?;
        checked_user_range(address, 1)?;
        let Some(id) = self.region_at(address) else {
            // Windows dynamic memory: its page tables are the record.
            if self.regions.paged().find(address).is_some() {
                return PhysicalAddress::new(self.table.translate(address, access)?);
            }
            return Err(Error::NotCommitted);
        };
        let r = self.regions[id.slot].as_ref().expect("located region");
        if r.backing.is_none() {
            return Err(Error::NotCommitted);
        }
        if !permits(r.permissions, access) {
            return Err(Error::AccessDenied);
        }
        PhysicalAddress::new(self.table.translate(address, access)?)
    }
    /// Validate the whole buffer before a copy. The VM guard must remain held
    /// until the architecture's fault-contained raw copy completes. No user
    /// reference is manufactured. Zero-length buffers still require user bounds.
    pub fn validate_user_range(
        &self,
        address: u64,
        length: usize,
        access: UserAccess,
    ) -> Result<(), Error> {
        self.live()?;
        checked_user_range(address, length as u64)?;
        if length == 0 {
            return Ok(());
        }
        let end = address + length as u64;
        let mut current = address;
        while current < end {
            self.translate(current, access)?;
            current = (current & !(PAGE_BYTES - 1)) + PAGE_BYTES;
        }
        Ok(())
    }
    /// # Safety
    /// Every process context has completed architecture post-switch retirement;
    /// no CPU or pending operation can ever return to this CR3 again. The VM
    /// guard remains exclusive; shared owners still retain all referenced views.
    /// Remote acknowledgements precede both data and page-table frame reuse.
    pub unsafe fn destroy(
        &mut self,
        frames: &mut FrameAllocator<'_>,
        kernel: &mut PageTables<'_>,
    ) -> Result<(), Error> {
        self.live()?;
        for slot in 0..MAX_REGIONS {
            if let Some(r) = &self.regions[slot] {
                let id = RegionId {
                    space: self.identity,
                    generation: r.generation,
                    slot,
                };
                if r.backing.is_some() {
                    unsafe { self.decommit(id, frames, kernel)? };
                }
                self.release(id)?;
            }
        }
        self.table.retire_root();
        let pool = self.pool.take().expect("live pool");
        unsafe { release_mapping(pool, frames, kernel) };
        for chunk in self.regions.table_chunks().iter_mut() {
            if let Some(mapping) = chunk.take() {
                unsafe { release_mapping(mapping, frames, kernel) };
            }
        }
        Ok(())
    }
}
fn permits(permissions: Permissions, access: UserAccess) -> bool {
    access == UserAccess::Read
        || matches!(
            (permissions, access),
            (Permissions::ReadWrite, UserAccess::Write)
                | (Permissions::ReadExecute, UserAccess::Execute)
        )
}
pub(super) fn checked_user_range(start: u64, bytes: u64) -> Result<(), Error> {
    let end = start.checked_add(bytes).ok_or(Error::Overflow)?;
    if !(USER_MIN..USER_END).contains(&start) || end > USER_END {
        return Err(Error::Address);
    }
    Ok(())
}
unsafe fn release_mapping(
    mapping: MappedRegion,
    frames: &mut FrameAllocator<'_>,
    kernel: &mut PageTables<'_>,
) {
    // Losing owned frames on an invariant failure is safer than returning them
    // while a stale mapping survives; fail closed rather than continue teardown.
    let owned = unsafe { kernel.unmap(mapping) }
        .expect("owned kernel mapping teardown")
        .expect("RAM backing");
    frames.free(owned).expect("owned frame teardown");
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::{boxed::Box, vec};
    pub(super) fn model(identity: u64) -> (Box<[u64]>, ProcessSpace) {
        let mut bytes = vec![0u64; TABLE_PAGES * 512].into_boxed_slice();
        let table = unsafe {
            UserTable::new(
                NonNull::new(bytes.as_mut_ptr()).unwrap(),
                0x200000,
                |_, _| {},
            )
        };
        let space = ProcessSpace {
            identity,
            generation: 1,
            table,
            pool: Some(MappedRegion::inert_model_token(
                0xffff_ffff_d400_0000,
                TABLE_PAGES as u64,
            )),
            regions: Regions(NonNull::from(Box::leak(Box::new(RegionTable::EMPTY)))),
        };
        (bytes, space)
    }
    fn commit_model(
        space: &mut ProcessSpace,
        id: RegionId,
        physical: u64,
        permissions: Permissions,
    ) {
        let r = space.regions[id.slot].as_mut().unwrap();
        space
            .table
            .map(r.start, physical, r.pages, permissions)
            .unwrap();
        r.permissions = permissions;
        r.backing = Some(Backing::Shared {
            physical,
            alias: 0xffff_ffff_d500_0000,
            maximum: permissions,
        });
    }
    #[test]
    fn reservations_exclude_overlap_guards_kernel_and_stale_or_foreign_ids() {
        let (_pool, mut space) = model(1);
        let (_other, other) = model(2);
        let pages = PageCount::new(2).unwrap();
        let id = space.reserve(0x400000, pages).unwrap();
        assert_eq!(space.reserve(0x401000, pages), Err(Error::Overlap));
        assert_eq!(space.reserve(USER_END - 4096, pages), Err(Error::Address));
        assert_eq!(
            space.reserve(0xffff_ffff_8000_0000, pages),
            Err(Error::Address)
        );
        assert_eq!(space.reserve(0, pages), Err(Error::Address));
        assert_eq!(space.reserve(0x500001, pages), Err(Error::Alignment));
        assert_eq!(other.get(id), Err(Error::InvalidMapping));
        assert_eq!(
            space.translate(0x400000, UserAccess::Read),
            Err(Error::NotCommitted)
        );
        space.release(id).unwrap();
        let fresh = space.reserve(0x400000, pages).unwrap();
        assert_ne!(fresh, id);
        assert_eq!(space.release(id), Err(Error::InvalidMapping));
        assert_eq!(space.get(fresh).unwrap().pages, 2);
    }
    #[test]
    fn whole_buffer_validation_rejects_guard_crossings_and_effective_rights() {
        let (_pool, mut space) = model(1);
        let id = space.reserve(0x400000, PageCount::new(2).unwrap()).unwrap();
        commit_model(&mut space, id, 0x800000, Permissions::ReadOnly);
        assert_eq!(
            space.validate_user_range(0x400fff, 2, UserAccess::Read),
            Ok(())
        );
        assert_eq!(
            space.validate_user_range(0x401fff, 2, UserAccess::Read),
            Err(Error::NotCommitted)
        );
        assert_eq!(
            space.validate_user_range(0x400000, 1, UserAccess::Write),
            Err(Error::AccessDenied)
        );
        assert_eq!(
            space.validate_user_range(0x400000, 1, UserAccess::Execute),
            Err(Error::AccessDenied)
        );
        assert_eq!(
            space.validate_user_range(u64::MAX, 8, UserAccess::Read),
            Err(Error::Overflow)
        );
        assert_eq!(
            space.validate_user_range(USER_END, 0, UserAccess::Read),
            Err(Error::Address)
        );
        assert_eq!(
            space.validate_user_range(0x600000, 0, UserAccess::Read),
            Ok(())
        );
        assert_eq!(space.release(id), Err(Error::InvalidState));
        assert_eq!(space.get(id).unwrap().physical, Some(0x800000));
        space.table.remove(0x400000, 2);
        assert_eq!(
            space.validate_user_range(0x400000, 1, UserAccess::Read),
            Err(Error::NotCommitted)
        );
    }
    #[test]
    fn reservation_exhaustion_and_generation_wrap_fail_without_losing_entries() {
        let (_pool, mut space) = model(1);
        for index in 0..MAX_REGIONS {
            space
                .reserve(USER_MIN + index as u64 * 8192, PageCount::new(1).unwrap())
                .unwrap();
        }
        assert_eq!(
            space.reserve(0x400000, PageCount::new(1).unwrap()),
            Err(Error::MappingLimit)
        );
        assert!(space.region_at(USER_MIN).is_some());
        assert!(space.region_at(USER_MIN + 4096).is_none());
        let id = space.region_at(USER_MIN).unwrap();
        space.release(id).unwrap();
        space.generation = u64::MAX;
        assert_eq!(
            space.reserve(USER_MIN, PageCount::new(1).unwrap()),
            Err(Error::Exhausted)
        );
        assert!(space.region_at(USER_MIN).is_none());
    }
}
