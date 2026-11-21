//! Kernel ownership bridge for the selected data controller. Filesystem clients
//! share this lifecycle; MMIO and DMA never outlive acknowledged quiescence.
use crate::{arch, process};
use door_boot_protocol::{BootInfo, MAPPING_BYTES, MAPPINGS_KIND, Mapping};
use door_kernel::{
    drivers::{
        pci,
        virtio_block::{SelfTest, VirtioBlock},
    },
    mm::{self, FrameStats, MappedRegion, PageCount, Permissions, PhysicalAddress, VirtualAddress},
    storage::{BlockDevice, Geometry, Result},
};

const DMA_BASE: u64 = 0xffff_ffff_b800_0000;
const BAR_BASE: u64 = 0xffff_ffff_b100_0000;
const BAR_STRIDE: u64 = 0x0100_0000;

#[derive(Clone, Copy)]
pub struct Statistics {
    pub reads: u64,
    pub writes: u64,
    pub flushes: u64,
    pub sampled_remaining: u64,
    pub stack_bytes: u64,
}

/// The trusted boot/service owner opens one selected controller after process VM
/// initialization. A panic retains backing; only `close` returns DMA to the pool.
pub struct Session {
    device: VirtioBlock,
    baseline: FrameStats,
    address: pci::Address,
    stack_base: u64,
    stack_end: u64,
    statistics: Statistics,
}

impl Session {
    pub fn open(info: &BootInfo<'_>) -> Self {
        let function = unsafe { pci::discover_block() }.expect("selected modern VirtIO data disk");
        let address = function.address();
        let baseline = process::with_kernel_memory(|frames, _| frames.stats());
        let (bars, dma) = process::with_kernel_memory(|frames, tables| {
            let bars: [Option<MappedRegion>; 6] = core::array::from_fn(|index| {
                function.bars()[index].map(|bar| {
                    assert!(bar.bytes <= BAR_STRIDE && bar.bytes.is_multiple_of(4096));
                    let physical = PhysicalAddress::new(bar.physical).unwrap();
                    let pages = PageCount::new(bar.bytes / 4096).unwrap();
                    mm::validate_device_extent(info, physical, pages)
                        .expect("exclusive PCI MMIO extent");
                    unsafe {
                        tables.map_device(
                            physical,
                            VirtualAddress::new(BAR_BASE + index as u64 * BAR_STRIDE).unwrap(),
                            pages,
                        )
                    }
                    .unwrap()
                })
            });
            let backing = frames.allocate(PageCount::new(8).unwrap(), 1).unwrap();
            let dma = unsafe {
                tables.map_frames(
                    backing,
                    VirtualAddress::new(DMA_BASE).unwrap(),
                    Permissions::ReadWrite,
                )
            }
            .unwrap();
            (bars, dma)
        });
        arch::enable_timer_service(true);
        arch::start_timer().unwrap();
        arch::enable_interrupts();
        let device = unsafe { VirtioBlock::initialize(function, bars, dma, clock_millis) }
            .expect("VirtIO negotiation and queue");
        let stack = info
            .table(MAPPINGS_KIND)
            .unwrap()
            .chunks_exact(MAPPING_BYTES)
            .map(|bytes| Mapping::decode(bytes).unwrap())
            .find(|mapping| mapping.role == 4)
            .unwrap();
        assert_eq!(stack.bytes, 131072);
        Self {
            device,
            baseline,
            address,
            stack_base: stack.virtual_start,
            stack_end: stack.virtual_start + stack.bytes,
            statistics: Statistics {
                reads: 0,
                writes: 0,
                flushes: 0,
                sampled_remaining: stack.bytes,
                stack_bytes: stack.bytes,
            },
        }
    }

    pub fn address(&self) -> pci::Address {
        self.address
    }
    pub fn features(&self) -> u64 {
        self.device.negotiated_features()
    }
    pub fn statistics(&self) -> Statistics {
        self.statistics
    }
    pub fn selftest(&mut self, scratch_lba: u64) -> door_kernel::drivers::Result<SelfTest> {
        self.device.selftest(scratch_lba)
    }

    #[inline(always)]
    fn sample_stack(&mut self) {
        let pointer: u64;
        unsafe {
            core::arch::asm!("mov {}, rsp", out(reg) pointer, options(nomem, nostack, preserves_flags));
        }
        let (base, end) = if pointer >= self.stack_base && pointer < self.stack_end {
            (self.stack_base, self.stack_end)
        } else {
            process::current_kernel_stack().expect("owned storage syscall stack")
        };
        assert!(pointer >= base && pointer < end);
        let remaining = pointer - base;
        assert!(remaining >= 16384, "storage I/O stack margin");
        self.statistics.sampled_remaining = self.statistics.sampled_remaining.min(remaining);
    }

    pub fn close(self) {
        let resources = self.device.shutdown().expect("acknowledged DMA quiescence");
        arch::disable_interrupts();
        arch::enable_timer_service(false);
        arch::stop_timer();
        process::with_kernel_memory(|frames, tables| {
            for bar in resources.bars.into_iter().flatten() {
                assert!(unsafe { tables.unmap(bar) }.unwrap().is_none());
            }
            let backing = unsafe { tables.unmap(resources.dma) }.unwrap().unwrap();
            assert_eq!(backing.pages().get(), 8);
            frames.free(backing).unwrap();
            assert_eq!(frames.stats(), self.baseline);
        });
    }
}

impl BlockDevice for Session {
    fn geometry(&self) -> Geometry {
        self.device.geometry()
    }
    fn read_sectors(&mut self, lba: u64, bytes: &mut [u8]) -> Result<()> {
        self.sample_stack();
        self.device.read_sectors(lba, bytes)?;
        self.statistics.reads += 1;
        Ok(())
    }
    fn write_sectors(&mut self, lba: u64, bytes: &[u8]) -> Result<()> {
        self.sample_stack();
        self.device.write_sectors(lba, bytes)?;
        self.statistics.writes += 1;
        Ok(())
    }
    fn flush(&mut self) -> Result<()> {
        self.sample_stack();
        self.device.flush()?;
        self.statistics.flushes += 1;
        Ok(())
    }
}
/// The PM timer's milliseconds: block requests poll with interrupts disabled
/// (the platform device lock), where the tick count stands still.
fn clock_millis() -> u64 {
    arch::pm_millis()
}
