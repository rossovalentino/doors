//! Kernel ownership bridge for the optional sound device (P15): the modern
//! VirtIO sound function, polled like the NIC. Only door-audio reaches it
//! (`platform_device::audio_*`); MMIO and DMA never outlive close.
use crate::process;
use door_boot_protocol::BootInfo;
use door_kernel::{
    drivers::{
        pci,
        virtio_sound::{self, Statistics, VirtioSound},
    },
    mm::{self, FrameStats, MappedRegion, PageCount, Permissions, PhysicalAddress, VirtualAddress},
};

const DMA_BASE: u64 = 0xffff_ffff_bc00_0000;
const BAR_BASE: u64 = 0xffff_ffff_bb00_0000;
const BAR_STRIDE: u64 = 0x0010_0000;

pub struct SoundSession {
    device: VirtioSound,
    baseline: FrameStats,
}
impl SoundSession {
    /// The sound device when the machine has one with a 48 kHz stereo
    /// output stream (None otherwise: door-audio then reports no device).
    pub fn open(info: &BootInfo<'_>) -> Option<Self> {
        let function = unsafe { pci::discover(pci::SOUND) }.ok()?;
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
                .allocate(PageCount::new(virtio_sound::DMA_PAGES).unwrap(), 1)
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
        match unsafe { VirtioSound::initialize(function, bars, dma, clock_millis) } {
            Ok(device) => Some(Self { device, baseline }),
            Err(_) => None,
        }
    }
    pub fn statistics(&self) -> Statistics {
        self.device.statistics()
    }
    pub fn running(&self) -> bool {
        self.device.running()
    }
    pub fn queued(&mut self) -> door_kernel::drivers::Result<u16> {
        self.device.queued()
    }
    pub fn write(&mut self, pcm: &[u8]) -> door_kernel::drivers::Result<()> {
        self.device.write(pcm)
    }
    pub fn start(&mut self) -> door_kernel::drivers::Result<()> {
        self.device.start()
    }
    pub fn stop(&mut self) -> door_kernel::drivers::Result<()> {
        self.device.stop()
    }
    pub fn capture_state(&self) -> door_abi::platform::Capture {
        use door_abi::platform::Capture;
        match (self.device.capture_present(), self.device.capturing()) {
            (false, _) => Capture::Absent,
            (true, false) => Capture::Stopped,
            (true, true) => Capture::Running,
        }
    }
    pub fn capture_start(&mut self) -> door_kernel::drivers::Result<()> {
        self.device.capture_start()
    }
    pub fn capture_stop(&mut self) -> door_kernel::drivers::Result<()> {
        self.device.capture_stop()
    }
    pub fn capture_read(&mut self, out: &mut [u8]) -> door_kernel::drivers::Result<bool> {
        self.device.capture_read(out)
    }
    pub fn close(self) {
        let resources = self
            .device
            .shutdown()
            .expect("acknowledged sound DMA quiescence");
        process::with_kernel_memory(|frames, tables| {
            for bar in resources.bars.into_iter().flatten() {
                assert!(unsafe { tables.unmap(bar) }.unwrap().is_none());
            }
            let backing = unsafe { tables.unmap(resources.dma) }.unwrap().unwrap();
            assert_eq!(backing.pages().get(), virtio_sound::DMA_PAGES);
            frames.free(backing).unwrap();
            assert_eq!(frames.stats(), self.baseline);
        });
    }
}
/// The PM timer's milliseconds (polled with interrupts disabled).
fn clock_millis() -> u64 {
    crate::arch::pm_millis()
}
