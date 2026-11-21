use super::{Error, PAGE_BYTES, PageCount, PhysicalAddress};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use door_boot_protocol::{BootInfo, MemoryRegion, Ownership};

/// Concurrent allocation records. Every mapping of owned RAM holds one, so
/// this tracks `paging::MAX_MAPPINGS` (the process VM asserts both bounds).
/// The page pool (P18) holds one per 4 MiB chunk on top of the process,
/// thread and section aliases.
pub const MAX_ALLOCATIONS: usize = 2048;
const FIRST_MANAGED_FRAME: u64 = 0x10_0000 / PAGE_BYTES;
static NEXT_ALLOCATOR: AtomicU64 = AtomicU64::new(1);
const EMPTY: Record = Record {
    start: 0,
    pages: 0,
    generation: 0,
};
struct BootRecords(core::cell::UnsafeCell<[Record; MAX_ALLOCATIONS]>);
// SAFETY: `from_boot_info` hands the records to one allocator, once.
unsafe impl Sync for BootRecords {}
static BOOT_RECORDS: BootRecords =
    BootRecords(core::cell::UnsafeCell::new([EMPTY; MAX_ALLOCATIONS]));
static BOOT_RECORDS_TAKEN: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Default)]
struct Record {
    start: u64,
    pages: u64,
    generation: u64,
}

/// Exclusive allocation authority. Not cloneable; dropping intentionally retains
/// frames. Free explicitly only after all device/mapping users have retired.
#[derive(Debug)]
pub struct OwnedFrames {
    start: PhysicalAddress,
    pages: PageCount,
    issuer: u64,
    generation: u64,
    slot: usize,
}
impl OwnedFrames {
    pub const fn start(&self) -> PhysicalAddress {
        self.start
    }
    pub const fn pages(&self) -> PageCount {
        self.pages
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameStats {
    pub capacity_pages: u64,
    pub managed_pages: u64,
    pub free_pages: u64,
    pub allocated_pages: u64,
    pub retained_usable_pages: u64,
}

/// Bounded bitmap of initially conventional, explicitly transferred frames.
/// Each set bit is free. Every other bit starts reserved; no firmware reclaim API
/// exists. Allocation records ensure free cannot turn reserved memory into RAM.
/// This is metadata only: the kernel establishes a single allocator over the
/// validated boot map before invoking any unsafe physical-memory access.
pub struct FrameAllocator<'a> {
    free: &'a mut [u64],
    /// Out of line (as the kernel's mapping records): 24 KiB would not
    /// fit the 128 KiB boot stack beside its by-value moves.
    records: &'a mut [Record; MAX_ALLOCATIONS],
    issuer: u64,
    next_generation: u64,
    stats: FrameStats,
}
impl<'a> FrameAllocator<'a> {
    pub fn from_boot_info(info: &BootInfo<'_>, bitmap: &'a mut [u64]) -> Result<Self, Error> {
        // One boot allocator owns the static records.
        if BOOT_RECORDS_TAKEN.swap(true, Ordering::AcqRel) {
            return Err(Error::InvalidState);
        }
        // SAFETY: the flag above hands the records out exactly once.
        let records = unsafe { &mut *BOOT_RECORDS.0.get() };
        Self::initialize(bitmap, records, info.memory_region_count(), |i| {
            info.memory_region(i).map_err(|_| Error::InvalidMemoryMap)
        })
    }
    #[cfg(test)]
    pub fn from_regions(regions: &[MemoryRegion], bitmap: &'a mut [u64]) -> Result<Self, Error> {
        extern crate std;
        let records = std::boxed::Box::leak(std::boxed::Box::new([EMPTY; MAX_ALLOCATIONS]));
        Self::initialize(bitmap, records, regions.len(), |i| Ok(regions[i]))
    }
    fn initialize(
        bitmap: &'a mut [u64],
        records: &'a mut [Record; MAX_ALLOCATIONS],
        count: usize,
        region: impl Fn(usize) -> Result<MemoryRegion, Error>,
    ) -> Result<Self, Error> {
        let capacity = u64::try_from(bitmap.len())
            .map_err(|_| Error::Overflow)?
            .checked_mul(64)
            .ok_or(Error::Overflow)?;
        let limit = capacity.checked_mul(PAGE_BYTES).ok_or(Error::Overflow)?;
        if capacity == 0 || limit > (1 << 52) || count == 0 {
            return Err(Error::InvalidMemoryMap);
        }
        // Validate the complete map before modifying caller-owned metadata.
        for i in 0..count {
            let current = region(i)?;
            let mut encoded = [0u8; 40];
            current
                .encode(&mut encoded)
                .map_err(|_| Error::InvalidMemoryMap)?;
            if current.range.end() > (1 << 52) {
                return Err(Error::InvalidMemoryMap);
            }
            for j in 0..i {
                if current.range.overlaps(region(j)?.range) {
                    return Err(Error::InvalidMemoryMap);
                }
            }
        }
        let issuer = NEXT_ALLOCATOR
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Error::Exhausted)?;
        bitmap.fill(0);
        let mut managed = 0;
        let mut retained = 0;
        for i in 0..count {
            let current = region(i)?;
            if current.ownership != Ownership::UsableAfterHandoff {
                continue;
            }
            // Retain the whole legacy low MiB even if firmware labels some of it
            // conventional. This includes null, low firmware scratch and EBDA.
            let original_begin = current.range.start() / PAGE_BYTES;
            let begin = original_begin.max(FIRST_MANAGED_FRAME);
            let end = current.range.end() / PAGE_BYTES;
            let bounded_end = end.min(capacity);
            retained += (end - original_begin) - bounded_end.saturating_sub(begin);
            for frame in begin..bounded_end {
                bitmap[frame as usize / 64] |= 1 << (frame % 64);
                managed += 1;
            }
        }
        Ok(Self {
            free: bitmap,
            records,
            issuer,
            next_generation: 1,
            stats: FrameStats {
                capacity_pages: capacity,
                managed_pages: managed,
                free_pages: managed,
                allocated_pages: 0,
                retained_usable_pages: retained,
            },
        })
    }
    pub const fn stats(&self) -> FrameStats {
        self.stats
    }
    pub fn allocate(
        &mut self,
        pages: PageCount,
        alignment_pages: u64,
    ) -> Result<OwnedFrames, Error> {
        if !alignment_pages.is_power_of_two() {
            return Err(Error::Alignment);
        }
        let slot = self
            .records
            .iter()
            .position(|r| r.pages == 0)
            .ok_or(Error::AllocationLimit)?;
        let next = self
            .next_generation
            .checked_add(1)
            .ok_or(Error::Exhausted)?;
        if pages.get() > self.stats.free_pages {
            return Err(Error::Exhausted);
        }
        let mut start = alignment_pages;
        while let Some(end) = start
            .checked_add(pages.get())
            .filter(|end| *end <= self.stats.capacity_pages)
        {
            let occupied = (start..end)
                .find(|&frame| self.free[frame as usize / 64] & (1 << (frame % 64)) == 0);
            if let Some(frame) = occupied {
                start = frame.checked_add(alignment_pages).ok_or(Error::Overflow)?
                    & !(alignment_pages - 1);
                continue;
            }
            for frame in start..end {
                self.free[frame as usize / 64] &= !(1 << (frame % 64));
            }
            let generation = self.next_generation;
            self.next_generation = next;
            self.records[slot] = Record {
                start,
                pages: pages.get(),
                generation,
            };
            self.stats.free_pages -= pages.get();
            self.stats.allocated_pages += pages.get();
            return Ok(OwnedFrames {
                start: PhysicalAddress::new(start * PAGE_BYTES)?,
                pages,
                issuer: self.issuer,
                generation,
                slot,
            });
        }
        Err(Error::Exhausted)
    }
    pub fn free(&mut self, frames: OwnedFrames) -> Result<(), Error> {
        let record = self.records.get(frames.slot).ok_or(Error::InvalidFree)?;
        if frames.issuer != self.issuer
            || record.pages != frames.pages.get()
            || record.start * PAGE_BYTES != frames.start.get()
            || record.generation != frames.generation
            || record.pages == 0
        {
            return Err(Error::InvalidFree);
        }
        let begin = record.start;
        let end = begin + record.pages;
        if (begin..end).any(|frame| self.free[frame as usize / 64] & (1 << (frame % 64)) != 0) {
            return Err(Error::InvalidFree);
        }
        for frame in begin..end {
            self.free[frame as usize / 64] |= 1 << (frame % 64);
        }
        self.stats.free_pages += record.pages;
        self.stats.allocated_pages -= record.pages;
        self.records[frames.slot] = Record::default();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use door_boot_protocol::PhysicalRange;
    fn region(start: u64, pages: u64, ownership: Ownership) -> MemoryRegion {
        MemoryRegion {
            range: PhysicalRange::new(
                (start + FIRST_MANAGED_FRAME) * PAGE_BYTES,
                pages * PAGE_BYTES,
                PAGE_BYTES,
            )
            .unwrap(),
            firmware_attributes: 0,
            firmware_type: if ownership == Ownership::UsableAfterHandoff {
                7
            } else if ownership == Ownership::FirmwareRuntime {
                6
            } else {
                4
            },
            ownership,
        }
    }
    fn duplicate(f: &OwnedFrames) -> OwnedFrames {
        OwnedFrames {
            start: f.start,
            pages: f.pages,
            issuer: f.issuer,
            generation: f.generation,
            slot: f.slot,
        }
    }
    #[test]
    fn only_conventional_transferred_frames_are_ever_returned() {
        let regions = [
            region(0, 4, Ownership::UsableAfterHandoff),
            region(4, 4, Ownership::FirmwareRuntime),
            region(8, 4, Ownership::BootData),
            region(12, 4, Ownership::ReclaimableAfterCopy),
            region(16, 4, Ownership::FramebufferMmio),
            region(20, 4, Ownership::KernelModule),
            region(24, 4, Ownership::PermanentlyReserved),
            region(28, 4, Ownership::UsableAfterHandoff),
        ];
        let mut bitmap = [0; 8];
        let mut a = FrameAllocator::from_regions(&regions, &mut bitmap).unwrap();
        assert_eq!(a.stats().free_pages, 8);
        for expected in [0, 1, 2, 3, 28, 29, 30, 31] {
            assert_eq!(
                a.allocate(PageCount::new(1).unwrap(), 1)
                    .unwrap()
                    .start()
                    .get(),
                (expected + FIRST_MANAGED_FRAME) * PAGE_BYTES
            );
        }
        assert!(matches!(
            a.allocate(PageCount::new(1).unwrap(), 1),
            Err(Error::Exhausted)
        ));
    }
    #[test]
    fn fragmentation_alignment_exhaustion_and_reuse() {
        let mut bitmap = [0; 8];
        let mut a = FrameAllocator::from_regions(
            &[region(1, 8, Ownership::UsableAfterHandoff)],
            &mut bitmap,
        )
        .unwrap();
        let first = a.allocate(PageCount::new(3).unwrap(), 4).unwrap();
        assert_eq!(first.start().get(), (4 + FIRST_MANAGED_FRAME) * PAGE_BYTES);
        assert!(matches!(
            a.allocate(PageCount::new(4).unwrap(), 1),
            Err(Error::Exhausted)
        ));
        a.free(first).unwrap();
        let all = a.allocate(PageCount::new(8).unwrap(), 1).unwrap();
        assert_eq!(a.stats().free_pages, 0);
        assert!(matches!(
            a.allocate(PageCount::new(1).unwrap(), 3),
            Err(Error::Alignment)
        ));
        a.free(all).unwrap();
        assert_eq!(a.stats().allocated_pages, 0);
    }
    #[test]
    fn invalid_double_stale_and_foreign_frees_do_not_release_reserved_pages() {
        let mut bitmap = [0; 8];
        let mut other_bitmap = [0; 8];
        let regions = [region(1, 8, Ownership::UsableAfterHandoff)];
        let mut a = FrameAllocator::from_regions(&regions, &mut bitmap).unwrap();
        let mut b = FrameAllocator::from_regions(&regions, &mut other_bitmap).unwrap();
        let first = a.allocate(PageCount::new(2).unwrap(), 1).unwrap();
        let stale = duplicate(&first);
        assert_eq!(b.free(duplicate(&first)), Err(Error::InvalidFree));
        let mut invalid = duplicate(&first);
        invalid.start = PhysicalAddress::new(0).unwrap();
        assert_eq!(a.free(invalid), Err(Error::InvalidFree));
        a.free(first).unwrap();
        assert_eq!(a.free(duplicate(&stale)), Err(Error::InvalidFree));
        let replacement = a.allocate(PageCount::new(2).unwrap(), 1).unwrap();
        assert_eq!(a.free(stale), Err(Error::InvalidFree));
        assert_eq!(a.stats().free_pages, 6);
        a.free(replacement).unwrap();
    }
    #[test]
    fn bounded_capacity_retains_high_frames_and_rejects_malformed_maps() {
        let mut bitmap = [0; 5];
        let a = FrameAllocator::from_regions(
            &[region(32, 96, Ownership::UsableAfterHandoff)],
            &mut bitmap,
        )
        .unwrap();
        assert_eq!(a.stats().free_pages, 32);
        assert_eq!(a.stats().retained_usable_pages, 64);
        let invalid = [
            region(1, 4, Ownership::UsableAfterHandoff),
            region(4, 4, Ownership::BootData),
        ];
        assert!(matches!(
            FrameAllocator::from_regions(&invalid, &mut bitmap),
            Err(Error::InvalidMemoryMap)
        ));
        let mut invalid = region(1, 4, Ownership::UsableAfterHandoff);
        invalid.firmware_attributes = 1 << 63;
        assert!(matches!(
            FrameAllocator::from_regions(&[invalid], &mut bitmap),
            Err(Error::InvalidMemoryMap)
        ));
    }

    #[test]
    fn legacy_low_megabyte_is_retained_even_if_conventional() {
        let mut bitmap = [0; 8];
        let mut whole = region(0, 1, Ownership::UsableAfterHandoff);
        whole.range = PhysicalRange::new(0, 257 * PAGE_BYTES, PAGE_BYTES).unwrap();
        let mut allocator = FrameAllocator::from_regions(&[whole], &mut bitmap).unwrap();
        assert_eq!(allocator.stats().free_pages, 1);
        assert_eq!(allocator.stats().retained_usable_pages, 256);
        assert_eq!(
            allocator
                .allocate(PageCount::new(1).unwrap(), 1)
                .unwrap()
                .start()
                .get(),
            0x10_0000
        );
    }

    #[test]
    fn large_contiguous_allocation_and_metadata_limit_are_fallible() {
        let mut bitmap = [0; 128];
        let mut allocator = FrameAllocator::from_regions(
            &[region(0, 4096, Ownership::UsableAfterHandoff)],
            &mut bitmap,
        )
        .unwrap();
        let large = allocator
            .allocate(PageCount::new(2048).unwrap(), 512)
            .unwrap();
        assert_eq!(large.start().get() % (512 * PAGE_BYTES), 0);
        assert_eq!(allocator.stats().allocated_pages, 2048);
        allocator.free(large).unwrap();
        let mut owned: [Option<OwnedFrames>; MAX_ALLOCATIONS] = core::array::from_fn(|_| None);
        for entry in &mut owned {
            *entry = Some(allocator.allocate(PageCount::new(1).unwrap(), 1).unwrap());
        }
        let before = allocator.stats();
        assert!(matches!(
            allocator.allocate(PageCount::new(1).unwrap(), 1),
            Err(Error::AllocationLimit)
        ));
        assert_eq!(allocator.stats(), before);
        for entry in owned {
            allocator.free(entry.unwrap()).unwrap();
        }
        assert_eq!(
            allocator.stats().free_pages,
            allocator.stats().managed_pages
        );
    }
}
