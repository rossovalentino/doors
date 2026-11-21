//! Four-level 4 KiB supervisor mappings, Intel SDM 325384-081US sections 4.5,
//! 4.6 and 4.10.4. A synchronous retirement hook extends the initial local-only
//! invalidation contract to every online CPU sharing the active CR3.
use super::{
    ByteCount, Error, OwnedFrames, PAGE_BYTES, PageCount, PhysicalAddress, VirtualAddress,
};
use core::sync::atomic::{Ordering, compiler_fence};
use door_boot_protocol::{BootInfo, MAPPING_BYTES, MAPPINGS_KIND, Mapping};

const ADDRESS: u64 = 0x000f_ffff_ffff_f000;
const PRESENT: u64 = 1;
const WRITABLE: u64 = 2;
const NX: u64 = 1 << 63;
const POOL_PAGES: usize = door_boot_protocol::KERNEL_TABLE_POOL_PAGES;
/// Concurrent kernel mapping records. One table holds every kernel alias:
/// boot and device mappings, sections, and each process's table pool, stacks,
/// startup block, TLS, image segments and dynamic regions. The process VM
/// asserts that the declared desktop capacity fits (ADR 0004).
pub const MAX_MAPPINGS: usize = 2048;
const MAX_MAPPING_PAGES: u64 = 65536;
// One contiguous <=256MiB mapping can need129 PTs, two PDs and two
// PDPTs when crossing a PML4 boundary. Rollback storage follows that extent,
// not the entire table pool: kernel stack frames must remain bounded.
pub(super) const MAX_NEW_TABLES: usize = (MAX_MAPPING_PAGES as usize / 512) + 6;

/// Fixed storage for bounded table pools, including pages beyond one CPU word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TableBitmap<const WORDS: usize>([u64; WORDS]);
impl<const WORDS: usize> TableBitmap<WORDS> {
    pub const fn empty() -> Self {
        Self([0; WORDS])
    }
    pub const fn root() -> Self {
        let mut v = Self::empty();
        v.0[0] = 1;
        v
    }
    pub fn contains(&self, index: usize) -> bool {
        self.0[index / 64] & (1u64 << (index % 64)) != 0
    }
    pub fn insert(&mut self, index: usize) {
        self.0[index / 64] |= 1u64 << (index % 64);
    }
    pub fn remove(&mut self, index: usize) {
        self.0[index / 64] &= !(1u64 << (index % 64));
    }
    pub fn count(&self) -> u32 {
        self.0.iter().map(|word| word.count_ones()).sum()
    }
}

fn device_physical_bits() -> Result<u8, Error> {
    // x86-64 implements CPUID. Read the architectural physical width before
    // accepting a 64-bit PCI BAR; unsupported address bits fault in a PTE.
    let maximum = core::arch::x86_64::__cpuid(0x8000_0000).eax;
    if maximum < 0x8000_0008 {
        return Err(Error::Address);
    }
    let bits = core::arch::x86_64::__cpuid(0x8000_0008).eax as u8;
    if !(32..=52).contains(&bits) {
        return Err(Error::Address);
    }
    Ok(bits)
}

fn physical_extent_supported(end: u64, device: bool, bits: u8) -> bool {
    (32..=52).contains(&bits) && end <= (1u64 << if device { bits } else { 32 })
}

/// No writable-executable state is representable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Permissions {
    ReadOnly,
    ReadWrite,
    ReadExecute,
}
impl Permissions {
    const fn flags(self) -> u64 {
        PRESENT
            | match self {
                Self::ReadOnly => NX,
                Self::ReadWrite => NX | WRITABLE,
                Self::ReadExecute => 0,
            }
    }
}

#[derive(Clone, Copy, Default)]
struct Record {
    virtual_start: u64,
    physical: u64,
    pages: u64,
    flags: u64,
    generation: u64,
}

/// Owns a live mapping and, for RAM, its non-cloneable physical-frame token.
/// Dropping intentionally retains backing; explicit unmap retires translations.
#[derive(Debug)]
pub struct MappedRegion {
    start: VirtualAddress,
    pages: PageCount,
    slot: usize,
    generation: u64,
    pool: u64,
    frames: Option<OwnedFrames>,
}
impl MappedRegion {
    #[cfg(test)]
    pub(super) fn inert_model_token(start: u64, pages: u64) -> Self {
        Self {
            start: VirtualAddress::new(start).unwrap(),
            pages: PageCount::new(pages).unwrap(),
            slot: 0,
            generation: 1,
            pool: 0,
            frames: None,
        }
    }
    /// RAM backing identity. Device mappings deliberately have no frame token.
    pub fn physical(&self) -> Option<PhysicalAddress> {
        self.frames.as_ref().map(OwnedFrames::start)
    }
    pub const fn pages(&self) -> PageCount {
        self.pages
    }
    pub const fn start(&self) -> VirtualAddress {
        self.start
    }
    pub const fn bytes(&self) -> ByteCount {
        self.pages.bytes()
    }
    pub fn end(&self) -> VirtualAddress {
        self.start
            .checked_add(self.bytes())
            .expect("validated mapping extent")
    }
}

#[derive(Debug)]
pub struct MapFailure {
    pub error: Error,
    pub frames: OwnedFrames,
}
#[derive(Debug)]
pub struct UnmapFailure {
    pub error: Error,
    pub region: MappedRegion,
}

/// Adopts the reserved, identity-mapped loader table pool. Structural table pages
/// directly linked from a PML4 persist because private roots share those links.
/// Empty lower tables are reusable only after synchronous translation retirement.
/// A failed construction rolls back all newly allocated tables and leaves.
pub struct PageTables<'a> {
    pool: u64,
    pool_pages: usize,
    used: TableBitmap<{ POOL_PAGES.div_ceil(64) }>,
    boot_mappings: &'a [u8],
    /// Out of line (`KERNEL_RECORDS`): at 2,048 records the table would no
    /// longer fit the 128 KiB boot stack beside its by-value moves.
    records: &'a mut [Record; MAX_MAPPINGS],
    next_generation: u64,
    retire: fn(u64),
}
/// The kernel table's mapping records. One `PageTables` exists
/// (`from_boot_info`'s contract), so one static backs it.
struct KernelRecords(core::cell::UnsafeCell<[Record; MAX_MAPPINGS]>);
// SAFETY: only the single kernel PageTables, created once on the boot CPU
// before any other CPU runs, ever borrows the records.
unsafe impl Sync for KernelRecords {}
static KERNEL_RECORDS: KernelRecords = KernelRecords(core::cell::UnsafeCell::new(
    [Record {
        virtual_start: 0,
        physical: 0,
        pages: 0,
        flags: 0,
        generation: 0,
    }; MAX_MAPPINGS],
));
impl<'a> PageTables<'a> {
    pub const fn root_physical(&self) -> u64 {
        self.pool
    }
    /// Copy only supervisor upper-half root entries into a private process root.
    /// Future shared kernel mappings must remain beneath already-present entries.
    ///
    /// # Safety
    /// Destination is an exclusive, writable, aligned zeroed page. Kernel table
    /// ownership and the IRQ-off retirement contract hold for the whole copy.
    pub(super) unsafe fn copy_kernel_root(&self, destination: *mut u64) -> Result<(), Error> {
        self.context()?;
        for index in 256..512 {
            let entry = self.read(self.pool + index * 8);
            if entry & 4 != 0 {
                return Err(Error::InvalidTable);
            }
        }
        for index in 256..512 {
            // SAFETY: caller owns the destination page; source belongs to pool.
            unsafe {
                destination
                    .add(index as usize)
                    .write(self.read(self.pool + index * 8))
            };
        }
        Ok(())
    }
    /// # Safety
    /// BootInfo and live mappings were validated. The whole table pool is mapped
    /// identity RW/NX and exclusively owned. This CPU is the sole online CPU;
    /// interrupts remain disabled while calling mapping operations. No second
    /// PageTables instance or other writer may mutate these structures.
    pub unsafe fn from_boot_info(info: &BootInfo<'a>) -> Result<Self, Error> {
        let boot_mappings = info.table(MAPPINGS_KIND).ok_or(Error::InvalidTable)?;
        let mapping = boot_mappings
            .chunks_exact(MAPPING_BYTES)
            .find_map(|b| Mapping::decode(b).ok().filter(|m| m.role == 5))
            .ok_or(Error::InvalidTable)?;
        if mapping.physical != mapping.virtual_start
            || mapping.permissions != 6
            || mapping.bytes == 0
            || mapping.bytes > POOL_PAGES as u64 * PAGE_BYTES
        {
            return Err(Error::InvalidTable);
        }
        let mut result = Self {
            pool: mapping.physical,
            pool_pages: (mapping.bytes / PAGE_BYTES) as usize,
            used: TableBitmap::empty(),
            boot_mappings,
            // SAFETY: the caller guarantees this is the only PageTables
            // instance, so these records are borrowed exactly once.
            records: unsafe { &mut *KERNEL_RECORDS.0.get() },
            next_generation: 1,
            retire: invalidate,
        };
        result.context()?;
        result.discover(result.pool, 4)?;
        Ok(result)
    }
    pub fn used_table_pages(&self) -> u32 {
        self.used.count()
    }
    /// Live mapping records (at most `MAX_MAPPINGS`), for exhaustion reports.
    pub fn used_mappings(&self) -> usize {
        self.records.iter().filter(|r| r.pages != 0).count()
    }
    /// Install synchronous local and remote translation retirement before APs
    /// can access this address space. It also covers rollback of table links.
    ///
    /// # Safety
    /// The callback must invalidate this CPU and obtain acknowledgements from
    /// every online CPU which can retain this address space before returning.
    /// A timeout must fail closed (never return). CPU online/offline publication
    /// must serialize with its target snapshot. It must not acquire the caller's
    /// page-table lock, and its remote handler must work without scheduler locks.
    /// Every later mutation still requires one exclusive writer with IRQs off.
    pub unsafe fn install_retirement(&mut self, retire: fn(u64)) {
        self.retire = retire;
    }

    /// Promote the reserved AP trampoline after its bytes have been prepared.
    /// This is the only supported change to a loader-owned mapping: one low
    /// identity page transitions from RW/NX to read-only/executable.
    ///
    /// # Safety
    /// The validated startup page contains the complete trampoline, no writable
    /// aliases exist, all APs remain offline, and interrupts are disabled.
    pub unsafe fn seal_startup_code(&mut self, info: &BootInfo<'_>) -> Result<(), Error> {
        self.context()?;
        let startup = info.validate_startup().map_err(|_| Error::InvalidTable)?;
        let mapping = self
            .boot_mappings
            .chunks_exact(MAPPING_BYTES)
            .find_map(|b| {
                Mapping::decode(b)
                    .ok()
                    .filter(|m| m.role == door_boot_protocol::STARTUP_CODE_ROLE)
            })
            .ok_or(Error::InvalidMapping)?;
        if mapping.physical != startup.code_physical
            || mapping.virtual_start != startup.code_physical
            || mapping.bytes != PAGE_BYTES
            || mapping.permissions != 6
        {
            return Err(Error::InvalidMapping);
        }
        let entry = self
            .leaf(startup.code_physical)?
            .ok_or(Error::InvalidMapping)?;
        if self.read(entry) & !(0x20 | 0x40)
            != startup.code_physical | Permissions::ReadWrite.flags()
        {
            return Err(Error::InvalidMapping);
        }
        self.write(
            entry,
            startup.code_physical | Permissions::ReadExecute.flags(),
        );
        (self.retire)(startup.code_physical);
        Ok(())
    }
    pub fn is_mapped(&self, address: VirtualAddress) -> Result<bool, Error> {
        Ok(self.leaf(address.get())?.is_some())
    }

    /// # Safety
    /// The exclusive-writer and installed retirement contract still holds. No
    /// references point into the chosen virtual range; caller owns reservation.
    /// Frames originate from the kernel's sole allocator over the validated boot
    /// map, denote ordinary RAM, and have no aliases or outstanding DMA users.
    /// Newly committed RAM is zeroed through supervisor RW/NX mappings before
    /// its requested protection is exposed. Failure returns original ownership.
    pub unsafe fn map_frames(
        &mut self,
        frames: OwnedFrames,
        start: VirtualAddress,
        permissions: Permissions,
    ) -> Result<MappedRegion, MapFailure> {
        let flags = Permissions::ReadWrite.flags();
        let outcome = self.insert(frames.start(), start, frames.pages(), flags);
        match outcome {
            Err(error) => Err(MapFailure { error, frames }),
            Ok(mut region) => {
                // SAFETY: insert just created this exclusive, writable RAM extent
                // and invalidated stale translations before any memory access.
                unsafe {
                    core::ptr::write_bytes(
                        start.get() as *mut u8,
                        0,
                        frames.pages().bytes().get() as usize,
                    );
                }
                self.set_flags(&mut region, permissions.flags());
                region.frames = Some(frames);
                Ok(region)
            }
        }
    }
    /// # Safety
    /// In addition to exclusive page-table access, caller verified this physical
    /// extent is device MMIO and owns device access. It must never name RAM with
    /// a live cached alias. Device pages are not zeroed or returned to a RAM pool.
    pub unsafe fn map_device(
        &mut self,
        physical: PhysicalAddress,
        start: VirtualAddress,
        pages: PageCount,
    ) -> Result<MappedRegion, Error> {
        self.insert(
            physical,
            start,
            pages,
            Permissions::ReadWrite.flags() | 0x18,
        )
    }
    /// # Safety
    /// All accesses obey the new permissions, interrupts are disabled, and only
    /// the retirement hook covers every CPU. No device DMA relies on this mapping.
    pub unsafe fn protect(
        &mut self,
        region: &mut MappedRegion,
        permissions: Permissions,
    ) -> Result<(), Error> {
        self.context()?;
        let record = self.validate_region(region)?;
        self.validate_leaves(record)?;
        self.set_flags(region, permissions.flags() | (record.flags & 0x18));
        Ok(())
    }
    /// # Safety
    /// There are no remaining pointers, references, executing instructions,
    /// stacks or device users in the region. Exclusive writer and IRQs disabled.
    /// Backing ownership is returned only after all retirement acknowledgements.
    pub unsafe fn unmap(
        &mut self,
        region: MappedRegion,
    ) -> Result<Option<OwnedFrames>, UnmapFailure> {
        let result = self
            .context()
            .and_then(|()| self.validate_region(&region))
            .and_then(|record| self.validate_leaves(record).map(|()| record));
        let record = match result {
            Ok(record) => record,
            Err(error) => return Err(UnmapFailure { error, region }),
        };
        for page in 0..record.pages {
            let address = record.virtual_start + page * PAGE_BYTES;
            let entry = self
                .leaf(address)
                .expect("prevalidated table")
                .expect("prevalidated present leaf");
            self.write(entry, 0);
            (self.retire)(address);
        }
        // One walk per touched 2 MiB range, after all leaves have retired. Keep
        // PML4-linked PDPTs stable for every previously copied process root.
        let end = record.virtual_start + record.pages * PAGE_BYTES;
        let mut address = record.virtual_start;
        while address < end {
            self.reclaim_empty_lower_tables(address);
            let Some(next) = (address | ((1 << 21) - 1)).checked_add(1) else {
                break;
            };
            address = next;
        }
        self.records[region.slot] = Record::default();
        Ok(region.frames)
    }
    fn reclaim_empty_lower_tables(&mut self, address: u64) {
        let root = self.read(self.pool + ((address >> 39) & 511) * 8);
        if root & PRESENT == 0 {
            return;
        }
        let pdpt = root & ADDRESS;
        let pd_link = pdpt + ((address >> 30) & 511) * 8;
        let pd = self.read(pd_link);
        if pd & PRESENT == 0 {
            return;
        }
        let pt_link = (pd & ADDRESS) + ((address >> 21) & 511) * 8;
        let pt = self.read(pt_link);
        if pt & PRESENT != 0 {
            self.reclaim_if_empty(pt_link, pt & ADDRESS, address);
        }
        self.reclaim_if_empty(pd_link, pd & ADDRESS, address);
    }
    fn reclaim_if_empty(&mut self, parent: u64, table: u64, address: u64) {
        let index = self.table_index(table).expect("validated table pool");
        if (0..512).any(|entry| self.read(table + entry * 8) != 0) {
            return;
        }
        self.write(parent, 0);
        // Clearing a leaf alone does not retire a cached structural reference.
        // No allocator may reuse this page until all online CPUs acknowledge.
        (self.retire)(address);
        self.used.remove(index);
    }
    fn context(&self) -> Result<(), Error> {
        #[cfg(not(test))]
        {
            // x86_64 always provides CPUID leaf 1. Check PAT before
            // reading its MSR rather than relying only on firmware defaults.
            if core::arch::x86_64::__cpuid(1).edx & (1 << 16) == 0 {
                return Err(Error::InvalidTable);
            }
            let flags: u64;
            let root: u64;
            let cr0: u64;
            let cr4: u64;
            let pat_low: u32;
            let pat_high: u32;
            let efer_low: u32;
            let efer_high: u32;
            // SAFETY: every public entry which reaches here has a privileged
            // kernel-only safety contract. Host tests use an inert table model.
            unsafe {
                core::arch::asm!("pushfq", "pop {}", out(reg) flags, options(nomem));
                core::arch::asm!("mov {}, cr3", out(reg) root, options(nomem, nostack));
                core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nomem, nostack));
                core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nomem, nostack));
                core::arch::asm!("rdmsr", in("ecx") 0x277u32, out("eax") pat_low, out("edx") pat_high, options(nomem, nostack));
                core::arch::asm!("rdmsr", in("ecx") 0xc000_0080u32, out("eax") efer_low, out("edx") efer_high, options(nomem, nostack));
            }
            let _ = (pat_high, efer_high);
            if flags & (1 << 9) != 0 {
                return Err(Error::InterruptsEnabled);
            }
            if root & ADDRESS != self.pool
                || cr4 & ((1 << 12) | (1 << 17)) != 0
                || cr0 & (1 << 16) == 0
                || efer_low & (1 << 11) == 0
                || pat_low & 0xff != 6
                || pat_low >> 24 != 0
            {
                return Err(Error::InvalidTable);
            }
        }
        Ok(())
    }
    fn table_index(&self, address: u64) -> Result<usize, Error> {
        let offset = address.checked_sub(self.pool).ok_or(Error::InvalidTable)?;
        if !offset.is_multiple_of(PAGE_BYTES) || offset / PAGE_BYTES >= self.pool_pages as u64 {
            return Err(Error::InvalidTable);
        }
        Ok((offset / PAGE_BYTES) as usize)
    }
    fn discover(&mut self, table: u64, level: usize) -> Result<(), Error> {
        let index = self.table_index(table)?;
        if self.used.contains(index) {
            return Err(Error::InvalidTable);
        }
        self.used.insert(index);
        if level == 1 {
            return Ok(());
        }
        for index in 0..512 {
            let entry = self.read(table + index * 8);
            if entry & PRESENT == 0 {
                if entry != 0 {
                    return Err(Error::InvalidTable);
                }
                continue;
            }
            if entry & ((1 << 7) | (1 << 2) | NX) != 0 || entry & WRITABLE == 0 {
                return Err(Error::InvalidTable);
            }
            self.discover(entry & ADDRESS, level - 1)?;
        }
        Ok(())
    }
    fn read(&self, address: u64) -> u64 {
        debug_assert!(
            address >= self.pool
                && address + 8 <= self.pool + self.pool_pages as u64 * PAGE_BYTES
                && address.is_multiple_of(8)
        );
        // SAFETY: callers derive entries only from checked pool table addresses.
        unsafe { core::ptr::read_volatile(address as *const u64) }
    }
    fn write(&mut self, address: u64, value: u64) {
        debug_assert!(
            address >= self.pool
                && address + 8 <= self.pool + self.pool_pages as u64 * PAGE_BYTES
                && address.is_multiple_of(8)
        );
        // SAFETY: pool is exclusively owned and mapped writable for its lifetime.
        unsafe {
            core::ptr::write_volatile(address as *mut u64, value);
        }
        compiler_fence(Ordering::SeqCst);
    }
    fn leaf(&self, address: u64) -> Result<Option<u64>, Error> {
        let mut table = self.pool;
        for shift in [39, 30, 21, 12] {
            self.table_index(table)?;
            let slot = table + ((address >> shift) & 511) * 8;
            let entry = self.read(slot);
            if entry & PRESENT == 0 {
                if entry != 0 {
                    return Err(Error::InvalidTable);
                }
                return Ok(None);
            }
            if shift == 12 {
                return Ok(Some(slot));
            }
            if entry & ((1 << 7) | (1 << 2) | NX) != 0 || entry & WRITABLE == 0 {
                return Err(Error::InvalidTable);
            }
            table = entry & ADDRESS;
        }
        Err(Error::InvalidTable)
    }
    fn insert(
        &mut self,
        physical: PhysicalAddress,
        start: VirtualAddress,
        pages: PageCount,
        flags: u64,
    ) -> Result<MappedRegion, Error> {
        self.context()?;
        if !physical.get().is_multiple_of(PAGE_BYTES) || !start.get().is_multiple_of(PAGE_BYTES) {
            return Err(Error::Alignment);
        }
        if pages.get() > MAX_MAPPING_PAGES {
            return Err(Error::MappingLimit);
        }
        let end = start.checked_add(pages.bytes())?.get();
        let physical_end = physical.checked_add(pages.bytes())?.get();
        let device = flags & 0x18 == 0x18;
        if !physical_extent_supported(
            physical_end,
            device,
            if device { device_physical_bits()? } else { 32 },
        ) {
            return Err(Error::Address);
        }
        // Initial kernel-only arena. Null/low mappings are not a native user ABI.
        if start.get() < 0xffff_8000_0000_0000 {
            return Err(Error::Address);
        }
        let slot = self
            .records
            .iter()
            .position(|r| r.pages == 0)
            .ok_or(Error::MappingLimit)?;
        let generation = self.next_generation;
        let next = generation.checked_add(1).ok_or(Error::Exhausted)?;
        for bytes in self.boot_mappings.chunks_exact(MAPPING_BYTES) {
            let mapping = Mapping::decode(bytes).map_err(|_| Error::InvalidTable)?;
            if overlap(
                physical.get(),
                physical_end,
                mapping.physical,
                mapping.physical + mapping.bytes,
            ) {
                return Err(Error::Overlap);
            }
        }
        for record in self.records.iter() {
            if record.pages != 0
                && (overlap(
                    start.get(),
                    end,
                    record.virtual_start,
                    record.virtual_start + record.pages * PAGE_BYTES,
                ) || overlap(
                    physical.get(),
                    physical_end,
                    record.physical,
                    record.physical + record.pages * PAGE_BYTES,
                ))
            {
                return Err(Error::Overlap);
            }
        }
        for page in 0..pages.get() {
            if self.leaf(start.get() + page * PAGE_BYTES)?.is_some() {
                return Err(Error::Overlap);
            }
        }
        let mut new_links = [(0u64, 0usize); MAX_NEW_TABLES];
        let mut link_count = 0;
        let mut inserted = 0;
        let result = (|| {
            for page in 0..pages.get() {
                let address = start.get() + page * PAGE_BYTES;
                let mut table = self.pool;
                for shift in [39, 30, 21] {
                    let entry_address = table + ((address >> shift) & 511) * 8;
                    let mut entry = self.read(entry_address);
                    if entry == 0 {
                        let index = (0..self.pool_pages)
                            .find(|index| !self.used.contains(*index))
                            .ok_or(Error::Exhausted)?;
                        let new_table = self.pool + index as u64 * PAGE_BYTES;
                        for offset in 0..512 {
                            self.write(new_table + offset * 8, 0);
                        }
                        self.used.insert(index);
                        new_links[link_count] = (entry_address, index);
                        link_count += 1;
                        entry = new_table | PRESENT | WRITABLE;
                        self.write(entry_address, entry);
                    }
                    table = entry & ADDRESS;
                }
                self.write(
                    table + ((address >> 12) & 511) * 8,
                    (physical.get() + page * PAGE_BYTES) | flags,
                );
                (self.retire)(address);
                inserted += 1;
            }
            Ok(())
        })();
        if let Err(error) = result {
            for page in 0..inserted {
                let address = start.get() + page * PAGE_BYTES;
                let entry = self.leaf(address)?.ok_or(Error::InvalidTable)?;
                self.write(entry, 0);
                (self.retire)(address);
            }
            for &(entry, index) in new_links[..link_count].iter().rev() {
                self.write(entry, 0);
                // Flush paging-structure cache even if no leaf was published.
                (self.retire)(start.get() + inserted * PAGE_BYTES);
                self.used.remove(index);
            }
            return Err(error);
        }
        self.records[slot] = Record {
            virtual_start: start.get(),
            physical: physical.get(),
            pages: pages.get(),
            flags,
            generation,
        };
        self.next_generation = next;
        Ok(MappedRegion {
            start,
            pages,
            slot,
            generation,
            pool: self.pool,
            frames: None,
        })
    }
    fn validate_region(&self, region: &MappedRegion) -> Result<Record, Error> {
        let record = *self.records.get(region.slot).ok_or(Error::InvalidMapping)?;
        if region.pool != self.pool
            || record.pages == 0
            || record.generation != region.generation
            || record.virtual_start != region.start.get()
            || record.pages != region.pages.get()
        {
            return Err(Error::InvalidMapping);
        }
        Ok(record)
    }
    fn validate_leaves(&self, record: Record) -> Result<(), Error> {
        for page in 0..record.pages {
            let entry = self
                .leaf(record.virtual_start + page * PAGE_BYTES)?
                .ok_or(Error::InvalidMapping)?;
            let value = self.read(entry);
            // Hardware may set accessed/dirty bits, which are not permissions.
            if value & !(0x20 | 0x40) != (record.physical + page * PAGE_BYTES) | record.flags {
                return Err(Error::InvalidMapping);
            }
        }
        Ok(())
    }
    fn set_flags(&mut self, region: &mut MappedRegion, flags: u64) {
        let record = self.records[region.slot];
        for page in 0..record.pages {
            let address = record.virtual_start + page * PAGE_BYTES;
            let entry = self
                .leaf(address)
                .expect("exclusive validated table")
                .expect("committed mapping");
            self.write(entry, (record.physical + page * PAGE_BYTES) | flags);
            (self.retire)(address);
        }
        self.records[region.slot].flags = flags;
    }
}
fn overlap(a: u64, end_a: u64, b: u64, end_b: u64) -> bool {
    a < end_b && b < end_a
}
fn invalidate(address: u64) {
    compiler_fence(Ordering::SeqCst);
    #[cfg(not(test))]
    // SAFETY: reachable only inside the explicitly privileged mapping API.
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) address, options(nostack, preserves_flags));
    }
    #[cfg(test)]
    let _ = address;
    compiler_fence(Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec;
    #[test]
    fn table_bitmap_tracks_all_pool_words_without_shift_aliases() {
        let mut bitmap = TableBitmap::<8>::empty();
        for i in [0, 63, 64, 127, 128, 255, 256, 511] {
            bitmap.insert(i);
        }
        assert_eq!(bitmap.count(), 8);
        bitmap.remove(128);
        assert!(!bitmap.contains(128));
        assert!(bitmap.contains(0));
        assert!(bitmap.contains(256));
        assert_eq!(bitmap.count(), 7);
    }
    #[test]
    fn concurrent_native_heap_aliases_exceed_the_old_aggregate_pool_and_retire() {
        let mut storage = vec![0; POOL_PAGES * 512 + 511];
        let mut tables = model(&mut storage, POOL_PAGES);
        let mut regions = vec::Vec::new();
        for i in 0..8u64 {
            regions.push(
                tables
                    .insert(
                        pa(0x40000000 + i * 0x08000000),
                        va(0xffff_8000_0000_0000 + i * 0x8000000000),
                        PageCount::new(32768).unwrap(),
                        Permissions::ReadWrite.flags(),
                    )
                    .unwrap(),
            );
        }
        assert!(tables.used_table_pages() > 512);
        for region in regions {
            unsafe {
                tables.unmap(region).unwrap();
            }
        }
        assert!(tables.used_table_pages() <= 25);
    }
    #[test]
    fn maximum_contiguous_mapping_crosses_root_boundary_with_bounded_rollback() {
        let mut storage = vec![0; POOL_PAGES * 512 + 511];
        let mut tables = model(&mut storage, POOL_PAGES);
        let region = tables
            .insert(
                pa(0x40000000),
                va(0xffff_80ff_ffff_f000),
                PageCount::new(MAX_MAPPING_PAGES).unwrap(),
                Permissions::ReadWrite.flags(),
            )
            .unwrap();
        assert_eq!(tables.used_table_pages(), 134);
        unsafe {
            tables.unmap(region).unwrap();
        }
        assert_eq!(
            tables.used_table_pages(),
            3,
            "only copied PML4 parent links remain"
        );
    }
    fn model(storage: &mut [u64], pages: usize) -> PageTables<'static> {
        let base = ((storage.as_mut_ptr() as u64) + 4095) & !4095;
        PageTables {
            pool: base,
            pool_pages: pages,
            used: TableBitmap::root(),
            boot_mappings: &[],
            records: std::boxed::Box::leak(std::boxed::Box::new([Record::default(); MAX_MAPPINGS])),
            next_generation: 1,
            retire: invalidate,
        }
    }
    fn va(value: u64) -> VirtualAddress {
        VirtualAddress::new(value).unwrap()
    }
    fn pa(value: u64) -> PhysicalAddress {
        PhysicalAddress::new(value).unwrap()
    }
    #[test]
    fn wide_device_bars_preserve_ram_limit_and_reclaim_only_mappings() {
        for bits in 32..=52 {
            assert!(physical_extent_supported(1u64 << bits, true, bits));
            assert!(!physical_extent_supported(
                (1u64 << bits) + 4096,
                true,
                bits
            ));
            assert!(physical_extent_supported(1u64 << 32, false, bits));
            assert!(!physical_extent_supported((1u64 << 32) + 4096, false, bits));
        }
        assert!(!physical_extent_supported(4096, true, 53));
        assert!(!physical_extent_supported(4096, true, 31));
        let mut storage = vec![0; 8 * 512 + 511];
        let mut tables = model(&mut storage, 8);
        let start = va(0xffff_ffff_b500_0000);
        let physical = pa(0x1_0000_0000);
        assert!(matches!(
            tables.insert(
                physical,
                start,
                PageCount::new(4).unwrap(),
                Permissions::ReadWrite.flags()
            ),
            Err(Error::Address)
        ));
        assert_eq!(tables.used_table_pages(), 1);
        let mapping =
            unsafe { tables.map_device(physical, start, PageCount::new(4).unwrap()) }.unwrap();
        let leaf = tables.leaf(start.get()).unwrap().unwrap();
        assert_eq!(
            tables.read(leaf),
            physical.get() | Permissions::ReadWrite.flags() | 0x18
        );
        assert!(unsafe { tables.unmap(mapping) }.unwrap().is_none());
        assert!(!tables.is_mapped(start).unwrap());
    }
    #[test]
    fn private_root_copy_omits_low_mappings_and_rejects_user_kernel_roots() {
        let mut storage = vec![0; 4 * 512 + 511];
        let mut tables = model(&mut storage, 4);
        tables.write(tables.pool, 0x10_0007);
        tables.write(tables.pool + 511 * 8, 0x20_0003);
        let mut destination = [0u64; 512];
        unsafe { tables.copy_kernel_root(destination.as_mut_ptr()).unwrap() };
        assert!(destination[..256].iter().all(|entry| *entry == 0));
        assert_eq!(destination[511], 0x20_0003);
        tables.write(tables.pool + 256 * 8, 0x30_0007);
        destination.fill(0);
        assert_eq!(
            unsafe { tables.copy_kernel_root(destination.as_mut_ptr()) },
            Err(Error::InvalidTable)
        );
        assert!(destination.iter().all(|entry| *entry == 0));
    }
    #[test]
    fn installed_retirement_covers_publish_protect_remove_and_rollback() {
        use core::sync::atomic::AtomicUsize;
        static RETIRED: AtomicUsize = AtomicUsize::new(0);
        fn acknowledge(address: u64) {
            assert!(address.is_multiple_of(PAGE_BYTES));
            RETIRED.fetch_add(1, Ordering::SeqCst);
        }
        let mut storage = vec![0; 8 * 512 + 511];
        let mut tables = model(&mut storage, 8);
        unsafe {
            tables.install_retirement(acknowledge);
        }
        let mut region = tables
            .insert(
                pa(0x200000),
                va(0xffff_ffff_a000_0000),
                PageCount::new(2).unwrap(),
                Permissions::ReadWrite.flags(),
            )
            .unwrap();
        assert_eq!(RETIRED.load(Ordering::SeqCst), 2);
        unsafe {
            tables.protect(&mut region, Permissions::ReadOnly).unwrap();
        }
        assert_eq!(RETIRED.load(Ordering::SeqCst), 4);
        unsafe {
            tables.unmap(region).unwrap();
        }
        assert_eq!(RETIRED.load(Ordering::SeqCst), 8);
        let mut small_storage = vec![0; 2 * 512 + 511];
        let mut small = model(&mut small_storage, 2);
        unsafe {
            small.install_retirement(acknowledge);
        }
        assert!(matches!(
            small.insert(
                pa(0x200000),
                va(0xffff_ffff_a000_0000),
                PageCount::new(1).unwrap(),
                Permissions::ReadWrite.flags()
            ),
            Err(Error::Exhausted)
        ));
        assert_eq!(
            RETIRED.load(Ordering::SeqCst),
            9,
            "rollback must retire the parent link before reuse"
        );
    }
    #[test]
    fn expanded_pool_tracks_and_reclaims_bits_above_sixty_three() {
        let mut storage = vec![0; POOL_PAGES * 512 + 511];
        let mut tables = model(&mut storage, POOL_PAGES);
        let mut mappings = vec![];
        for index in 0..72 {
            mappings.push(
                tables
                    .insert(
                        pa(0x200000 + index * PAGE_BYTES),
                        va(0xffff_ffff_a000_0000 + index * (1 << 21)),
                        PageCount::new(1).unwrap(),
                        Permissions::ReadWrite.flags(),
                    )
                    .unwrap(),
            );
        }
        assert_eq!(tables.used_table_pages(), 75);
        for mapping in mappings {
            unsafe {
                tables.unmap(mapping).unwrap();
            }
        }
        assert_eq!(tables.used_table_pages(), 2);
    }
    #[test]
    fn unmap_reuses_lower_tables_without_replacing_shared_root_links() {
        let mut storage = vec![0; 5 * 512 + 511];
        let mut tables = model(&mut storage, 5);
        let mut copied_root = [0; 512];
        for range in 0..128 {
            let address = 0xffff_ffff_a000_0000 + range * (1 << 21);
            let first = tables
                .insert(
                    pa(0x200000),
                    va(address),
                    PageCount::new(1).unwrap(),
                    Permissions::ReadWrite.flags(),
                )
                .unwrap();
            let second = tables
                .insert(
                    pa(0x201000),
                    va(address + PAGE_BYTES),
                    PageCount::new(1).unwrap(),
                    Permissions::ReadOnly.flags(),
                )
                .unwrap();
            if range == 0 {
                unsafe { tables.copy_kernel_root(copied_root.as_mut_ptr()).unwrap() };
            }
            assert_eq!(tables.read(tables.pool + 511 * 8), copied_root[511]);
            unsafe { tables.unmap(first).unwrap() };
            assert!(tables.is_mapped(va(address + PAGE_BYTES)).unwrap());
            assert_eq!(tables.used_table_pages(), 4);
            unsafe { tables.unmap(second).unwrap() };
            assert_eq!(
                tables.used_table_pages(),
                2,
                "only PML4 and shared PDPT remain"
            );
            assert_eq!(tables.read(tables.pool + 511 * 8), copied_root[511]);
        }
    }
    #[test]
    fn record_table_bounds_concurrent_mappings_and_reuses_a_freed_record() {
        let mut storage = vec![0; 8 * 512 + 511];
        let mut tables = model(&mut storage, 8);
        let base = 0xffff_ffff_a000_0000;
        let map = |tables: &mut PageTables<'_>, i: u64| {
            tables.insert(
                pa(0x20_0000 + i * PAGE_BYTES),
                va(base + i * PAGE_BYTES),
                PageCount::new(1).unwrap(),
                Permissions::ReadWrite.flags(),
            )
        };
        let mut live: std::vec::Vec<_> = (0..MAX_MAPPINGS as u64)
            .map(|i| map(&mut tables, i).unwrap())
            .collect();
        assert_eq!(tables.used_mappings(), MAX_MAPPINGS);
        let full = MAX_MAPPINGS as u64;
        assert_eq!(map(&mut tables, full).err(), Some(Error::MappingLimit));
        assert!(!tables.is_mapped(va(base + full * PAGE_BYTES)).unwrap());
        unsafe { tables.unmap(live.pop().unwrap()).unwrap() };
        live.push(map(&mut tables, full).unwrap());
        assert_eq!(tables.used_mappings(), MAX_MAPPINGS);
        for mapping in live {
            unsafe { tables.unmap(mapping).unwrap() };
        }
        assert_eq!(tables.used_mappings(), 0);
    }
    #[test]
    fn map_protect_unmap_preserves_guards_and_rejects_aliases() {
        let mut storage = vec![0; 64 * 512 + 511];
        let mut tables = model(&mut storage, 64);
        let start = va(0xffff_ffff_a000_1000);
        let mut region = tables
            .insert(
                pa(0x200000),
                start,
                PageCount::new(8).unwrap(),
                Permissions::ReadWrite.flags(),
            )
            .unwrap();
        assert!(tables.is_mapped(start).unwrap());
        assert!(!tables.is_mapped(va(start.get() - PAGE_BYTES)).unwrap());
        assert!(!tables.is_mapped(region.end()).unwrap());
        assert!(matches!(
            tables.insert(
                pa(0x300000),
                start,
                PageCount::new(1).unwrap(),
                Permissions::ReadWrite.flags()
            ),
            Err(Error::Overlap)
        ));
        assert!(matches!(
            tables.insert(
                pa(0x200000),
                va(0xffff_ffff_b000_0000),
                PageCount::new(1).unwrap(),
                Permissions::ReadExecute.flags()
            ),
            Err(Error::Overlap)
        ));
        unsafe {
            tables.protect(&mut region, Permissions::ReadOnly).unwrap();
        }
        let leaf = tables.leaf(start.get()).unwrap().unwrap();
        assert_eq!(tables.read(leaf) & (WRITABLE | NX), NX);
        unsafe {
            tables.unmap(region).unwrap();
        }
        assert!(!tables.is_mapped(start).unwrap());
        assert!(
            tables
                .insert(
                    pa(0x200000),
                    start,
                    PageCount::new(8).unwrap(),
                    Permissions::ReadWrite.flags()
                )
                .is_ok()
        );
    }
    #[test]
    fn table_exhaustion_rolls_back_partial_construction() {
        let mut storage = vec![0; 5 * 512 + 511];
        let mut tables = model(&mut storage, 4);
        let start = va(0xffff_ffff_a01f_f000); // crosses a 2MiB table boundary
        assert!(matches!(
            tables.insert(
                pa(0x200000),
                start,
                PageCount::new(2).unwrap(),
                Permissions::ReadWrite.flags()
            ),
            Err(Error::Exhausted)
        ));
        assert_eq!(tables.used_table_pages(), 1);
        assert!(!tables.is_mapped(start).unwrap());
        assert!(
            tables
                .insert(
                    pa(0x200000),
                    start,
                    PageCount::new(1).unwrap(),
                    Permissions::ReadWrite.flags()
                )
                .is_ok()
        );
    }
    #[test]
    fn altered_leaf_is_rejected_before_teardown_and_invalid_extents_do_not_mutate() {
        let mut storage = vec![0; 8 * 512 + 511];
        let mut tables = model(&mut storage, 8);
        let start = va(0xffff_ffff_a000_0000);
        assert!(matches!(
            tables.insert(
                pa(1),
                start,
                PageCount::new(1).unwrap(),
                Permissions::ReadWrite.flags()
            ),
            Err(Error::Alignment)
        ));
        assert!(matches!(
            tables.insert(
                pa(0x1000),
                va(0x1000),
                PageCount::new(1).unwrap(),
                Permissions::ReadWrite.flags()
            ),
            Err(Error::Address)
        ));
        assert_eq!(tables.used_table_pages(), 1);
        let region = tables
            .insert(
                pa(0x200000),
                start,
                PageCount::new(1).unwrap(),
                Permissions::ReadWrite.flags(),
            )
            .unwrap();
        let entry = tables.leaf(start.get()).unwrap().unwrap();
        tables.write(entry, 0x300000 | Permissions::ReadWrite.flags());
        assert_eq!(
            unsafe { tables.unmap(region) }.unwrap_err().error,
            Error::InvalidMapping
        );
        assert!(tables.is_mapped(start).unwrap());
    }
    #[test]
    fn loader_pool_discovery_rejects_cycles_and_external_table_pointers() {
        let mut storage = vec![0; 8 * 512 + 511];
        let mut tables = model(&mut storage, 8);
        tables.write(tables.pool, tables.pool | 3);
        tables.used = TableBitmap::empty();
        assert_eq!(tables.discover(tables.pool, 4), Err(Error::InvalidTable));
        tables.write(tables.pool, 0x1000 | 3);
        tables.used = TableBitmap::empty();
        assert_eq!(tables.discover(tables.pool, 4), Err(Error::InvalidTable));
    }

    #[test]
    fn failed_mapping_returns_frame_authority_without_committing_a_leaf() {
        use super::super::FrameAllocator;
        use door_boot_protocol::{MemoryRegion, Ownership, PhysicalRange};
        let mut bitmap = [0; 8];
        let mut allocator = FrameAllocator::from_regions(
            &[MemoryRegion {
                range: PhysicalRange::new(0x10_0000, PAGE_BYTES, PAGE_BYTES).unwrap(),
                firmware_type: 7,
                firmware_attributes: 0,
                ownership: Ownership::UsableAfterHandoff,
            }],
            &mut bitmap,
        )
        .unwrap();
        let frames = allocator.allocate(PageCount::new(1).unwrap(), 1).unwrap();
        let mut storage = vec![0; 2 * 512 + 511];
        let mut tables = model(&mut storage, 2);
        let start = va(0xffff_ffff_a000_0000);
        let failure =
            unsafe { tables.map_frames(frames, start, Permissions::ReadWrite) }.unwrap_err();
        assert_eq!(failure.error, Error::Exhausted);
        assert_eq!(tables.used_table_pages(), 1);
        assert!(!tables.is_mapped(start).unwrap());
        allocator.free(failure.frames).unwrap();
        assert_eq!(allocator.stats().free_pages, 1);
    }

    #[test]
    fn stale_mapping_token_cannot_change_reused_virtual_extent() {
        let mut storage = vec![0; 8 * 512 + 511];
        let mut tables = model(&mut storage, 8);
        let start = va(0xffff_ffff_a000_0000);
        let region = tables
            .insert(
                pa(0x200000),
                start,
                PageCount::new(1).unwrap(),
                Permissions::ReadWrite.flags(),
            )
            .unwrap();
        let mut stale = MappedRegion {
            start: region.start,
            pages: region.pages,
            slot: region.slot,
            generation: region.generation,
            pool: region.pool,
            frames: None,
        };
        unsafe {
            tables.unmap(region).unwrap();
        }
        let _replacement = tables
            .insert(
                pa(0x300000),
                start,
                PageCount::new(1).unwrap(),
                Permissions::ReadWrite.flags(),
            )
            .unwrap();
        assert_eq!(
            unsafe { tables.protect(&mut stale, Permissions::ReadExecute) },
            Err(Error::InvalidMapping)
        );
    }
}
