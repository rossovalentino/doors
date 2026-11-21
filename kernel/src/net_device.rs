//! Kernel ownership bridge for the optional network interface (P15): the
//! modern VirtIO NIC, polled like the data disk. Only door-network reaches
//! it (`platform_device::net_*`); MMIO and DMA never outlive close.
use crate::process;
use door_boot_protocol::BootInfo;
use door_kernel::{
    drivers::{
        pci,
        virtio_net::{self, Statistics, VirtioNet},
    },
    mm::{self, FrameStats, MappedRegion, PageCount, Permissions, PhysicalAddress, VirtualAddress},
};

const DMA_BASE: u64 = 0xffff_ffff_ba00_0000;
const BAR_BASE: u64 = 0xffff_ffff_b900_0000;
const BAR_STRIDE: u64 = 0x0010_0000;

pub struct NetSession {
    device: VirtioNet,
    baseline: FrameStats,
}
impl NetSession {
    /// The NIC when the machine has one (None without a modern VirtIO
    /// network function, or when it fails negotiation: the network service
    /// then reports no adapter).
    pub fn open(info: &BootInfo<'_>) -> Option<Self> {
        let function = unsafe { pci::discover(pci::NET) }.ok()?;
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
            let backing = frames
                .allocate(PageCount::new(virtio_net::DMA_PAGES).unwrap(), 1)
                .unwrap();
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
        match unsafe { VirtioNet::initialize(function, bars, dma, clock_millis) } {
            Ok(device) => Some(Self { device, baseline }),
            Err(_) => None,
        }
    }
    pub fn mac(&self) -> [u8; 6] {
        self.device.mac()
    }
    pub fn link_up(&self) -> bool {
        self.device.link_up()
    }
    pub fn statistics(&self) -> Statistics {
        self.device.statistics()
    }
    pub fn send(&mut self, frame: &[u8]) -> door_kernel::drivers::Result<()> {
        self.device.send(frame)
    }
    pub fn receive(&mut self, out: &mut [u8]) -> door_kernel::drivers::Result<Option<usize>> {
        self.device.receive(out)
    }
    pub fn close(self) {
        let resources = self
            .device
            .shutdown()
            .expect("acknowledged NIC DMA quiescence");
        process::with_kernel_memory(|frames, tables| {
            for bar in resources.bars.into_iter().flatten() {
                assert!(unsafe { tables.unmap(bar) }.unwrap().is_none());
            }
            let backing = unsafe { tables.unmap(resources.dma) }.unwrap().unwrap();
            assert_eq!(backing.pages().get(), virtio_net::DMA_PAGES);
            frames.free(backing).unwrap();
            assert_eq!(frames.stats(), self.baseline);
        });
    }
}
/// The PM timer's milliseconds (polled with interrupts disabled).
fn clock_millis() -> u64 {
    crate::arch::pm_millis()
}
