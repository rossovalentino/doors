//! VirtIO network class (modern PCI, polled): one receive and one transmit
//! queue whose 2 KiB buffers live in the driver-owned DMA region. Frames are
//! plain Ethernet II: no checksum, segmentation or mergeable-buffer offload
//! is negotiated, so every header the device writes or reads is zero.
use super::{
    Error, Result,
    pci::Function,
    virtio_pci::{self as io, QueueLayout, Transport},
};
use crate::mm::MappedRegion;
use core::sync::atomic::{Ordering, fence};

pub use super::virtio_pci::Resources;
/// Descriptors per queue.
pub const QUEUE: u16 = 64;
/// Largest Ethernet frame without FCS (no jumbo frames are negotiated).
pub const FRAME: usize = 1514;
/// `virtio_net_hdr` with VERSION_1 (num_buffers included).
const HEADER: usize = 12;
const BUFFER: u64 = 2048;
/// VERSION_1, MAC and STATUS (link state in the device configuration).
const REQUIRED: u64 = (1 << 32) | (1 << 5) | (1 << 16);
/// Offloads Door never asks for (checksum, GSO, control queue).
const REJECTED: u64 = 0;
const RX: QueueLayout = QueueLayout {
    index: 0,
    size: QUEUE,
    desc: 0,
    avail: 1024,
    used: 2048,
};
const TX: QueueLayout = QueueLayout {
    index: 1,
    size: QUEUE,
    desc: 4096,
    avail: 5120,
    used: 6144,
};
const RX_BUFFERS: u64 = 8192;
const TX_BUFFERS: u64 = RX_BUFFERS + QUEUE as u64 * BUFFER;
/// DMA region: both rings, then the receive and transmit buffers.
pub const DMA_PAGES: u64 = (TX_BUFFERS + QUEUE as u64 * BUFFER) / 4096;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Statistics {
    pub sent: u64,
    pub received: u64,
    /// Frames the device returned truncated or oversized.
    pub dropped: u64,
    /// Transmissions refused because every buffer was in flight.
    pub busy: u64,
}

/// Split-ring bookkeeping over a DMA region (virtual and physical bases).
/// Kept separate from the transport so host tests can play the device.
struct Rings {
    virt: u64,
    phys: u64,
    rx_avail: u16,
    rx_used: u16,
    tx_avail: u16,
    tx_used: u16,
}
impl Rings {
    fn descriptor(&self, q: QueueLayout, index: u16, address: u64, bytes: u32, flags: u16) {
        let at = self.virt + q.desc + u64::from(index) * 16;
        io::write64(at, address);
        io::write32(at + 8, bytes);
        io::write16(at + 12, flags);
        io::write16(at + 14, 0);
    }
    fn publish(&self, q: QueueLayout, idx: u16, head: u16) {
        let ring = self.virt + q.avail + 4 + u64::from(idx % q.size) * 2;
        io::write16(ring, head);
        fence(Ordering::SeqCst);
        io::write16(self.virt + q.avail + 2, idx.wrapping_add(1));
    }
    fn used_idx(&self, q: QueueLayout) -> u16 {
        let idx = io::read16(self.virt + q.used + 2);
        fence(Ordering::SeqCst);
        idx
    }
    fn used(&self, q: QueueLayout, idx: u16) -> (u32, u32) {
        let at = self.virt + q.used + 4 + u64::from(idx % q.size) * 8;
        (io::read32(at), io::read32(at + 4))
    }
    /// Offer every receive buffer to the device.
    fn fill_rx(&mut self) {
        for i in 0..QUEUE {
            self.descriptor(
                RX,
                i,
                self.phys + RX_BUFFERS + u64::from(i) * BUFFER,
                BUFFER as u32,
                2, // VIRTQ_DESC_F_WRITE
            );
            self.publish(RX, self.rx_avail, i);
            self.rx_avail = self.rx_avail.wrapping_add(1);
        }
    }
    /// The next received frame copied into `out` (its length), re-offering
    /// the buffer; None when nothing arrived. `Err(Malformed)` for a frame
    /// the device returned outside its buffer (dropped, buffer re-offered).
    fn receive(&mut self, out: &mut [u8]) -> Result<Option<usize>> {
        if self.used_idx(RX) == self.rx_used {
            return Ok(None);
        }
        let (id, bytes) = self.used(RX, self.rx_used);
        self.rx_used = self.rx_used.wrapping_add(1);
        if id >= u32::from(QUEUE) {
            return Err(Error::Malformed);
        }
        let id = id as u16;
        let bytes = bytes as usize;
        let result = if !(HEADER..=HEADER + FRAME).contains(&bytes) {
            Err(Error::Malformed)
        } else if bytes - HEADER > out.len() {
            Err(Error::Bounds)
        } else {
            let source = self.virt + RX_BUFFERS + u64::from(id) * BUFFER + HEADER as u64;
            for (i, b) in out[..bytes - HEADER].iter_mut().enumerate() {
                *b = io::read8(source + i as u64);
            }
            Ok(Some(bytes - HEADER))
        };
        self.publish(RX, self.rx_avail, id);
        self.rx_avail = self.rx_avail.wrapping_add(1);
        result
    }
    /// Reclaim completed transmissions; true when a buffer is free.
    fn tx_room(&mut self) -> bool {
        self.tx_used = self.used_idx(TX).min(self.tx_avail).max(self.tx_used);
        self.tx_avail.wrapping_sub(self.tx_used) < QUEUE
    }
    /// Queue `frame` for transmission (zero header, then the frame).
    fn send(&mut self, frame: &[u8]) -> Result<()> {
        if frame.len() < 14 || frame.len() > FRAME {
            return Err(Error::Bounds);
        }
        if !self.tx_room() {
            return Err(Error::QueueFull);
        }
        let slot = self.tx_avail % QUEUE;
        let buffer = self.virt + TX_BUFFERS + u64::from(slot) * BUFFER;
        for i in 0..HEADER as u64 {
            io::write8(buffer + i, 0);
        }
        for (i, b) in frame.iter().enumerate() {
            io::write8(buffer + HEADER as u64 + i as u64, *b);
        }
        self.descriptor(
            TX,
            slot,
            self.phys + TX_BUFFERS + u64::from(slot) * BUFFER,
            (HEADER + frame.len()) as u32,
            0,
        );
        self.publish(TX, self.tx_avail, slot);
        self.tx_avail = self.tx_avail.wrapping_add(1);
        Ok(())
    }
}

pub struct VirtioNet {
    transport: Transport,
    rings: Rings,
    notify: [u64; 2],
    mac: [u8; 6],
    statistics: Statistics,
}
impl VirtioNet {
    /// # Safety
    /// Caller exclusively owns the function and uncached BAR mappings, which
    /// must match Function::bars(), and coherent contiguous RW/NX DMA backing
    /// of `DMA_PAGES` pages.
    pub unsafe fn initialize(
        function: Function,
        bars: [Option<MappedRegion>; 6],
        dma: MappedRegion,
        clock: fn() -> u64,
    ) -> Result<Self> {
        let mut transport =
            unsafe { Transport::new_sized(function, bars, dma, clock, DMA_PAGES * 4096) }?;
        let mut notify = [0u64; 2];
        if let Err(error) = transport.initialize_queues(REQUIRED, REJECTED, &[RX, TX], &mut notify)
        {
            let _ = transport.reset();
            return Err(error);
        }
        let config = transport.device_config();
        let mut mac = [0u8; 6];
        for (i, b) in mac.iter_mut().enumerate() {
            *b = io::read8(config + i as u64);
        }
        let mut rings = Rings {
            virt: transport.virtual_dma(),
            phys: transport.physical_dma(),
            rx_avail: 0,
            rx_used: 0,
            tx_avail: 0,
            tx_used: 0,
        };
        rings.fill_rx();
        transport.notify_queue(notify[0], RX.index);
        Ok(Self {
            transport,
            rings,
            notify,
            mac,
            statistics: Statistics::default(),
        })
    }
    pub fn mac(&self) -> [u8; 6] {
        self.mac
    }
    /// VIRTIO_NET_S_LINK_UP in the device configuration.
    pub fn link_up(&self) -> bool {
        io::read16(self.transport.device_config() + 6) & 1 != 0
    }
    pub fn statistics(&self) -> Statistics {
        self.statistics
    }
    pub fn address(&self) -> super::pci::Address {
        self.transport.address()
    }
    pub fn send(&mut self, frame: &[u8]) -> Result<()> {
        self.transport.check()?;
        match self.rings.send(frame) {
            Ok(()) => {
                self.transport.notify_queue(self.notify[1], TX.index);
                self.statistics.sent += 1;
                Ok(())
            }
            Err(Error::QueueFull) => {
                self.statistics.busy += 1;
                Err(Error::QueueFull)
            }
            Err(e) => Err(e),
        }
    }
    pub fn receive(&mut self, out: &mut [u8]) -> Result<Option<usize>> {
        self.transport.check()?;
        let result = self.rings.receive(out);
        match result {
            Ok(Some(_)) => {
                self.statistics.received += 1;
                self.transport.notify_queue(self.notify[0], RX.index);
            }
            Err(Error::Malformed) => {
                self.statistics.dropped += 1;
                self.transport.notify_queue(self.notify[0], RX.index);
            }
            _ => (),
        }
        result
    }
    pub fn shutdown(self) -> Result<Resources> {
        self.transport.shutdown()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate std;
    /// A page-aligned host region standing in for DMA memory; the test
    /// plays the device by writing used-ring entries.
    fn region() -> (std::vec::Vec<u64>, u64) {
        let words = (DMA_PAGES * 4096 / 8) as usize;
        let v = std::vec![0u64; words + 512];
        let base = (v.as_ptr() as u64 + 4095) & !4095;
        (v, base)
    }
    fn rings(base: u64) -> Rings {
        Rings {
            virt: base,
            phys: 0x1000_0000,
            rx_avail: 0,
            rx_used: 0,
            tx_avail: 0,
            tx_used: 0,
        }
    }
    fn device_used(base: u64, q: QueueLayout, idx: u16, id: u32, bytes: u32) {
        let at = base + q.used + 4 + u64::from(idx % q.size) * 8;
        io::write32(at, id);
        io::write32(at + 4, bytes);
        io::write16(base + q.used + 2, idx.wrapping_add(1));
    }
    #[test]
    fn receive_copies_frames_after_the_header_and_reoffers_buffers() {
        let (_keep, base) = region();
        let mut r = rings(base);
        r.fill_rx();
        assert_eq!(
            io::read16(base + RX.avail + 2),
            QUEUE,
            "every buffer offered"
        );
        // Descriptor 0: device-writable, 2 KiB, at the first buffer.
        assert_eq!(io::read32(base + 8), BUFFER as u32);
        assert_eq!(io::read16(base + 12), 2);
        let mut out = [0u8; FRAME];
        assert_eq!(r.receive(&mut out), Ok(None));
        // The device fills buffer 5 with a 60-byte frame.
        let buffer = base + RX_BUFFERS + 5 * BUFFER;
        for i in 0..60u64 {
            io::write8(buffer + HEADER as u64 + i, i as u8);
        }
        device_used(base, RX, 0, 5, (HEADER + 60) as u32);
        assert_eq!(r.receive(&mut out), Ok(Some(60)));
        assert_eq!(&out[..4], &[0, 1, 2, 3]);
        assert_eq!(
            io::read16(base + RX.avail + 2),
            QUEUE + 1,
            "buffer re-offered"
        );
        // A length outside the buffer is dropped, and the buffer re-offered.
        device_used(base, RX, 1, 7, 9000);
        assert_eq!(r.receive(&mut out), Err(Error::Malformed));
        device_used(base, RX, 2, 99, 64);
        assert_eq!(r.receive(&mut out), Err(Error::Malformed), "foreign id");
    }
    #[test]
    fn send_bounds_frames_and_reports_a_full_queue() {
        let (_keep, base) = region();
        let mut r = rings(base);
        assert_eq!(r.send(&[0; 10]), Err(Error::Bounds), "runt");
        assert_eq!(r.send(&[0; FRAME + 1]), Err(Error::Bounds));
        let frame = [0xabu8; 64];
        for _ in 0..QUEUE {
            r.send(&frame).unwrap();
        }
        assert_eq!(r.send(&frame), Err(Error::QueueFull));
        // Header zero, frame after it, descriptor length covers both.
        let buffer = base + TX_BUFFERS;
        assert_eq!(io::read8(buffer), 0);
        assert_eq!(io::read8(buffer + HEADER as u64), 0xab);
        assert_eq!(io::read32(base + TX.desc + 8), (HEADER + 64) as u32);
        // The device completes two: two buffers are free again.
        device_used(base, TX, 1, 1, 0);
        r.send(&frame).unwrap();
        r.send(&frame).unwrap();
        assert_eq!(r.send(&frame), Err(Error::QueueFull));
    }
    #[test]
    fn layout_fits_the_dma_region() {
        assert!(TX.used + 6 + 8 * u64::from(QUEUE) <= RX_BUFFERS);
        assert_eq!(DMA_PAGES * 4096, TX_BUFFERS + u64::from(QUEUE) * BUFFER);
    }
}
