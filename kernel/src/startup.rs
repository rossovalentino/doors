//! P03 integration: all allocations originate in transferred conventional RAM;
//! the live heap/stacks/device mapping retain their ownership for this boot.
use core::{
    alloc::Layout,
    cell::UnsafeCell,
    fmt::Write,
    sync::atomic::{AtomicBool, Ordering},
};
use door_boot_debug::Serial;
use door_boot_protocol::{BootInfo, PhysicalRange};
use door_kernel::mm::{
    EarlyHeap, FrameAllocator, MappedRegion, PageCount, PageTables, Permissions, PhysicalAddress,
    VirtualAddress,
};

const TEST_BASE: u64 = 0xffff_ffff_a200_0000;
const HEAP_BASE: u64 = 0xffff_ffff_a100_0000;
const APIC_BASE: u64 = 0xffff_ffff_b000_0000;
struct BootBitmap(UnsafeCell<[u64; 16384]>);
// The single initialization owner is enforced below, before any CPU0 allocation.
unsafe impl Sync for BootBitmap {}
static BITMAP: BootBitmap = BootBitmap(UnsafeCell::new([0; 16384]));
static INITIALIZED: AtomicBool = AtomicBool::new(false);
static HARDWARE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
pub fn hardware_info() -> Result<door_abi::hardware::HardwareInfo, door_abi::Status> {
    let word = HARDWARE.load(Ordering::Acquire);
    if word == 0 {
        return Err(door_abi::Status::Unsupported);
    }
    Ok(door_abi::hardware::HardwareInfo {
        power_profile: (word & 255) as u8,
        processors: (word >> 8) as u16,
    })
}

pub struct Foundations<'a> {
    frames: FrameAllocator<'static>,
    tables: PageTables<'a>,
    _heap: EarlyHeap<'static>,
    _heap_mapping: MappedRegion,
    _stacks: [MappedRegion; 4],
    _apic: MappedRegion,
}

impl<'a> Foundations<'a> {
    /// The boot owner serializes physical allocation and table mutations. P04
    /// installs remote retirement before secondary CPUs can use shared mappings.
    pub(crate) fn memory(&mut self) -> (&mut FrameAllocator<'static>, &mut PageTables<'a>) {
        (&mut self.frames, &mut self.tables)
    }
    pub fn into_memory(self) -> (FrameAllocator<'static>, PageTables<'a>) {
        // Heap, IST and APIC mappings remain reserved for the lifetime of the
        // boot. Their tokens have no Drop reclamation; the VM owner adopts the
        // allocator and page-table registry containing those mappings.
        (self.frames, self.tables)
    }
    pub fn initialize(info: &BootInfo<'a>) -> Self {
        Self::initialize_mode(info, true)
    }
    pub fn initialize_session(info: &BootInfo<'a>) -> Self {
        Self::initialize_mode(info, false)
    }
    fn initialize_mode(info: &BootInfo<'a>, diagnostics: bool) -> Self {
        assert!(
            !INITIALIZED.swap(true, Ordering::AcqRel),
            "memory initialized twice"
        );
        let platform = info.validate_platform().expect("ACPI platform handoff");
        HARDWARE.store(
            u32::from(platform.power_profile) | (u32::from(platform.processor_count) << 8),
            Ordering::Release,
        );
        let mut frames = FrameAllocator::from_boot_info(info, unsafe { &mut *BITMAP.0.get() })
            .expect("physical frame map");
        // The checked P02 mappings still own CR3 and all reserved table backing.
        let mut tables = unsafe { PageTables::from_boot_info(info) }.expect("page table adoption");
        if diagnostics {
            Self::memory_diagnostics(info, &mut frames, &mut tables);
        }
        let stacks = core::array::from_fn(|index| {
            let base = 0xffff_ffff_a000_1000 + index as u64 * 0xa000;
            assert!(
                !tables
                    .is_mapped(VirtualAddress::new(base - 4096).unwrap())
                    .unwrap()
            );
            assert!(
                !tables
                    .is_mapped(VirtualAddress::new(base + 32768).unwrap())
                    .unwrap()
            );
            let backing = frames
                .allocate(PageCount::new(8).unwrap(), 1)
                .expect("IST backing");
            unsafe {
                tables.map_frames(
                    backing,
                    VirtualAddress::new(base).unwrap(),
                    Permissions::ReadWrite,
                )
            }
            .expect("guarded IST mapping")
        });
        unsafe { crate::arch::initialize(core::array::from_fn(|i| stacks[i].end().get())) }
            .expect("owned guarded exception stacks");
        if diagnostics {
            crate::arch::diagnostics().expect("CPU breakpoint/register diagnostics");
        }
        let heap_frames = frames
            .allocate(PageCount::new(128).unwrap(), 1)
            .expect("early heap backing");
        let heap_mapping = unsafe {
            tables.map_frames(
                heap_frames,
                VirtualAddress::new(HEAP_BASE).unwrap(),
                Permissions::ReadWrite,
            )
        }
        .expect("early heap mapping");
        // Exclusive permanently retained RW/NX mapping. No safe API can unmap
        // this private token while the heap has outstanding raw allocation blocks.
        let storage = unsafe {
            core::slice::from_raw_parts_mut(
                heap_mapping.start().get() as *mut u8,
                heap_mapping.bytes().get() as usize,
            )
        };
        let mut heap = EarlyHeap::new(storage);
        if diagnostics {
            let a = heap
                .allocate(Layout::from_size_align(257, 64).unwrap())
                .expect("heap allocation");
            let b = heap
                .allocate(Layout::from_size_align(4096, 4096).unwrap())
                .expect("aligned heap allocation");
            assert_eq!(a.as_ptr() as usize % 64, 0);
            assert_eq!(b.as_ptr() as usize % 4096, 0);
            assert!(a.as_ptr() as usize + a.bytes() <= b.as_ptr() as usize);
            unsafe {
                a.as_ptr().write_volatile(0x5a);
                b.as_ptr().write_volatile(0xa5);
            }
            assert_eq!(unsafe { a.as_ptr().read_volatile() }, 0x5a);
            let used = heap.used_bytes();
            assert!(
                heap.allocate(Layout::from_size_align(heap.capacity_bytes() + 1, 1).unwrap())
                    .is_err()
            );
            assert_eq!(heap.used_bytes(), used);
        }
        let apic = unsafe {
            tables.map_device(
                PhysicalAddress::new(platform.lapic_physical).unwrap(),
                VirtualAddress::new(APIC_BASE).unwrap(),
                PageCount::new(1).unwrap(),
            )
        }
        .expect("uncached LAPIC mapping");
        let mut serial = unsafe { Serial::new() };
        writeln!(serial, "DOOR_P03 MEMORY PASS managed={} free={} allocated={} tables={} heap_bytes={} guarded_ist=4 reserved_reclaimed=0",
            frames.stats().managed_pages, frames.stats().free_pages, frames.stats().allocated_pages, tables.used_table_pages(), heap.capacity_bytes()).ok();
        unsafe { crate::arch::initialize_timer(APIC_BASE, &platform) }.expect("APIC calibration");
        if diagnostics {
            crate::arch::timer_diagnostics().expect("APIC delivery");
        }
        Self {
            frames,
            tables,
            _heap: heap,
            _heap_mapping: heap_mapping,
            _stacks: stacks,
            _apic: apic,
        }
    }

    fn memory_diagnostics(
        info: &BootInfo<'_>,
        frames: &mut FrameAllocator<'_>,
        tables: &mut PageTables<'_>,
    ) {
        let before = frames.stats();
        let aligned = frames
            .allocate(PageCount::new(3).unwrap(), 4)
            .expect("aligned frames");
        assert_eq!(aligned.start().get() % (4 * 4096), 0);
        assert!(info.is_allocatable(
            PhysicalRange::new(aligned.start().get(), aligned.pages().bytes().get(), 4096).unwrap()
        ));
        frames.free(aligned).expect("release unmapped frames");
        assert_eq!(frames.stats(), before);
        assert!(
            frames
                .allocate(PageCount::new(before.free_pages + 1).unwrap(), 1)
                .is_err()
        );
        assert_eq!(frames.stats(), before);
        let backing = frames
            .allocate(PageCount::new(2).unwrap(), 1)
            .expect("test frames");
        let mut mapping = unsafe {
            tables.map_frames(
                backing,
                VirtualAddress::new(TEST_BASE).unwrap(),
                Permissions::ReadWrite,
            )
        }
        .expect("test mapping");
        let pointer = TEST_BASE as *mut u64;
        for index in 0..1024 {
            assert_eq!(unsafe { pointer.add(index).read_volatile() }, 0);
        }
        unsafe {
            pointer.write_volatile(0x1234_5678_abcd_ef00);
        }
        let overlap = frames.allocate(PageCount::new(1).unwrap(), 1).unwrap();
        let failure = unsafe {
            tables.map_frames(
                overlap,
                VirtualAddress::new(TEST_BASE).unwrap(),
                Permissions::ReadWrite,
            )
        }
        .expect_err("overlapping virtual map accepted");
        frames.free(failure.frames).unwrap();
        assert_eq!(unsafe { pointer.read_volatile() }, 0x1234_5678_abcd_ef00);
        unsafe { tables.protect(&mut mapping, Permissions::ReadOnly) }
            .expect("readonly protection");
        assert_eq!(unsafe { pointer.read_volatile() }, 0x1234_5678_abcd_ef00);
        unsafe { tables.protect(&mut mapping, Permissions::ReadWrite) }.unwrap();
        unsafe {
            pointer.write_volatile(u64::MAX);
        }
        let backing = unsafe { tables.unmap(mapping) }
            .expect("unmap and local translation retirement")
            .expect("RAM token");
        assert!(
            !tables
                .is_mapped(VirtualAddress::new(TEST_BASE).unwrap())
                .unwrap()
        );
        // Reusing the same frames cannot expose the previous owner's pattern.
        let mapping = unsafe {
            tables.map_frames(
                backing,
                VirtualAddress::new(TEST_BASE).unwrap(),
                Permissions::ReadWrite,
            )
        }
        .unwrap();
        assert_eq!(unsafe { pointer.read_volatile() }, 0);
        let backing = unsafe { tables.unmap(mapping) }.unwrap().unwrap();
        frames.free(backing).unwrap();
        assert_eq!(frames.stats(), before);
    }

    pub fn deliberate_fault(&mut self, mode: &[u8]) -> ! {
        crate::arch::disable_interrupts();
        if mode == b"trap-ud" {
            crate::arch::deliberate_ud();
        }
        if mode == b"trap-double" {
            for address in [TEST_BASE + 4096, TEST_BASE + 8192] {
                assert!(
                    !self
                        .tables
                        .is_mapped(VirtualAddress::new(address).unwrap())
                        .unwrap()
                );
            }
            // A failed exception-stack push must switch to the separate #DF IST.
            unsafe { crate::arch::deliberate_double_fault(TEST_BASE + 8192) }
        }
        let backing = self
            .frames
            .allocate(PageCount::new(1).unwrap(), 1)
            .expect("fault fixture backing");
        let permission = if mode == b"fault-readonly" {
            Permissions::ReadOnly
        } else {
            Permissions::ReadWrite
        };
        let _mapping = unsafe {
            self.tables
                .map_frames(backing, VirtualAddress::new(TEST_BASE).unwrap(), permission)
        }
        .expect("fault fixture mapping");
        assert!(
            !self
                .tables
                .is_mapped(VirtualAddress::new(TEST_BASE + 4096).unwrap())
                .unwrap()
        );
        match mode {
            b"fault-readonly" => unsafe {
                (TEST_BASE as *mut u64).write_volatile(1);
            },
            b"fault-guard" => unsafe {
                ((TEST_BASE + 4096) as *mut u64).write_volatile(1);
            },
            b"fault-nx" => unsafe {
                core::arch::asm!("jmp {}", in(reg) TEST_BASE, options(noreturn));
            },
            _ => panic!("unknown protection fixture"),
        }
        panic!("hardware protection did not fault")
    }
}
