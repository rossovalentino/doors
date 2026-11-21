//! VirtIO sound class (modern PCI, polled; P15): the control queue for PCM
//! stream information and commands, the transmit queue carrying one output
//! stream's periods of signed 16-bit stereo at 48 kHz, and the receive queue
//! returning one input stream's periods in the same format while capture
//! runs (prepared on start, released on stop). The event queue stays
//! disabled.
use super::{
    Error, Result,
    pci::Function,
    virtio_pci::{self as io, QueueLayout, Transport},
};
use crate::mm::MappedRegion;
use core::sync::atomic::{Ordering, fence};

pub use super::virtio_pci::Resources;
pub const RATE: u32 = 48_000;
pub const CHANNELS: u8 = 2;
/// One period: 10 ms of 16-bit stereo.
pub const PERIOD: usize = 1920;
/// Periods the device may hold (160 ms of queued sound).
pub const PERIODS: u16 = 16;
/// VERSION_1 only: virtio-sound defines no device features.
const REQUIRED: u64 = 1 << 32;
/// Spins between clock reads while polling: the clock is the PM timer, a
/// device register that traps into the hypervisor on every read.
const CLOCK_EVERY: usize = 1024;
const REJECTED: u64 = 0;
const CONTROL: QueueLayout = QueueLayout {
    index: 0,
    size: 4,
    desc: 0,
    avail: 1024,
    used: 2048,
};
const TX: QueueLayout = QueueLayout {
    index: 2,
    size: 2 * PERIODS,
    desc: 4096,
    avail: 5120,
    used: 6144,
};
/// Control request and response buffers.
const REQUEST: u64 = 8192;
const RESPONSE: u64 = 8192 + 512;
/// Transmit slots: `virtio_snd_pcm_xfer` (stream id), the period, then the
/// device-written `virtio_snd_pcm_status` at `STATUS`.
const SLOTS: u64 = 12288;
const SLOT: u64 = 2048;
const STATUS: u64 = 1936;
/// Capture periods the driver offers the device at once (160 ms).
pub const CAPTURE_PERIODS: u16 = 16;
const RX: QueueLayout = QueueLayout {
    index: 3,
    size: 2 * CAPTURE_PERIODS,
    desc: 45056,
    avail: 46080,
    used: 47104,
};
/// Capture slots: the `virtio_snd_pcm_xfer` header (stream id) at 0, then the
/// device-written period at `CAPTURE_DATA` and its status right after it.
const CAPTURE_SLOTS: u64 = 49152;
const CAPTURE_DATA: u64 = 16;
const CAPTURE_STATUS: u64 = CAPTURE_DATA + PERIOD as u64;
const _: () = assert!(CAPTURE_STATUS + 8 <= SLOT);
/// DMA region: the control and transmit rings, the control buffers, the
/// transmit slots, the receive ring, then the capture slots.
pub const DMA_PAGES: u64 = (CAPTURE_SLOTS + CAPTURE_PERIODS as u64 * SLOT) / 4096;

const R_PCM_INFO: u32 = 0x0100;
const R_PCM_SET_PARAMS: u32 = 0x0101;
const R_PCM_PREPARE: u32 = 0x0102;
const R_PCM_RELEASE: u32 = 0x0103;
const R_PCM_START: u32 = 0x0104;
const R_PCM_STOP: u32 = 0x0105;
const S_OK: u32 = 0x8000;
const FORMAT_S16: u8 = 5;
const RATE_48000: u8 = 7;
const OUTPUT: u8 = 0;
const INPUT: u8 = 1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Statistics {
    /// Periods the device finished playing.
    pub played: u64,
    /// Periods the device returned with an error status.
    pub failed: u64,
    /// Writes refused because every period was queued.
    pub busy: u64,
    /// Capture periods received, and received with an error status (read
    /// as silence).
    pub captured: u64,
    pub capture_failed: u64,
}

/// Split-ring bookkeeping over a DMA region (virtual and physical bases),
/// kept apart from the transport so host tests can play the device.
struct Rings {
    virt: u64,
    phys: u64,
    ctl_avail: u16,
    ctl_used: u16,
    /// Periods offered and reclaimed (slot = count % PERIODS).
    tx_avail: u16,
    tx_used: u16,
    /// Capture periods offered and taken back (slot = count % CAPTURE_PERIODS).
    rx_avail: u16,
    rx_used: u16,
}
impl Rings {
    fn descriptor(
        &self,
        q: QueueLayout,
        index: u16,
        address: u64,
        bytes: u32,
        flags: u16,
        next: u16,
    ) {
        let at = self.virt + q.desc + u64::from(index) * 16;
        io::write64(at, address);
        io::write32(at + 8, bytes);
        io::write16(at + 12, flags);
        io::write16(at + 14, next);
    }
    fn publish(&self, q: QueueLayout, idx: u16, head: u16) {
        io::write16(self.virt + q.avail + 4 + u64::from(idx % q.size) * 2, head);
        fence(Ordering::SeqCst);
        io::write16(self.virt + q.avail + 2, idx.wrapping_add(1));
    }
    fn used_idx(&self, q: QueueLayout) -> u16 {
        let idx = io::read16(self.virt + q.used + 2);
        fence(Ordering::SeqCst);
        idx
    }
    fn used_id(&self, q: QueueLayout, idx: u16) -> u32 {
        io::read32(self.virt + q.used + 4 + u64::from(idx % q.size) * 8)
    }
    /// Offer `request` with a `response_bytes` reply buffer on the control
    /// queue (descriptors 0 and 1); the caller notifies and waits.
    fn post_control(&mut self, request: &[u8], response_bytes: u32) {
        for (i, b) in request.iter().enumerate() {
            io::write8(self.virt + REQUEST + i as u64, *b);
        }
        for i in 0..u64::from(response_bytes) {
            io::write8(self.virt + RESPONSE + i, 0);
        }
        self.descriptor(CONTROL, 0, self.phys + REQUEST, request.len() as u32, 1, 1);
        self.descriptor(CONTROL, 1, self.phys + RESPONSE, response_bytes, 2, 0);
        self.publish(CONTROL, self.ctl_avail, 0);
        self.ctl_avail = self.ctl_avail.wrapping_add(1);
    }
    /// The answered control request's status, None while pending.
    fn control_done(&mut self) -> Option<u32> {
        if self.used_idx(CONTROL) == self.ctl_used {
            return None;
        }
        self.ctl_used = self.ctl_used.wrapping_add(1);
        Some(io::read32(self.virt + RESPONSE))
    }
    /// Reclaim finished periods: (played, failed). A completion out of
    /// order is a device fault.
    fn reclaim(&mut self) -> Result<(u64, u64)> {
        let (mut played, mut failed) = (0, 0);
        // Every period is one used entry, so `tx_used` also counts the
        // used entries already taken.
        let idx = self.used_idx(TX);
        while self.tx_used != self.tx_avail && self.tx_used != idx {
            let expected = u32::from(self.tx_used % PERIODS) * 2;
            if self.used_id(TX, self.tx_used) != expected {
                return Err(Error::Malformed);
            }
            let slot = u64::from(self.tx_used % PERIODS);
            if io::read32(self.virt + SLOTS + slot * SLOT + STATUS) == S_OK {
                played += 1;
            } else {
                failed += 1;
            }
            self.tx_used = self.tx_used.wrapping_add(1);
        }
        Ok((played, failed))
    }
    fn queued(&self) -> u16 {
        self.tx_avail.wrapping_sub(self.tx_used)
    }
    /// Offer the next capture slot to `stream`: the readable header, then the
    /// writable period and status.
    fn offer_capture(&mut self, stream: u32) {
        let slot = self.rx_avail % CAPTURE_PERIODS;
        let base = CAPTURE_SLOTS + u64::from(slot) * SLOT;
        io::write32(self.virt + base, stream);
        io::write32(self.virt + base + CAPTURE_STATUS, 0);
        let head = slot * 2;
        self.descriptor(RX, head, self.phys + base, 4, 1, head + 1);
        self.descriptor(
            RX,
            head + 1,
            self.phys + base + CAPTURE_DATA,
            PERIOD as u32 + 8,
            2,
            0,
        );
        self.publish(RX, self.rx_avail, head);
        self.rx_avail = self.rx_avail.wrapping_add(1);
    }
    fn capture_outstanding(&self) -> u16 {
        self.rx_avail.wrapping_sub(self.rx_used)
    }
    /// The oldest returned capture period, copied to `out` (zero-filled past
    /// what the device wrote): Some(true) when its status was OK. None while
    /// the device holds it; a completion out of order is a device fault.
    fn take_capture(&mut self, out: &mut [u8]) -> Result<Option<bool>> {
        if out.len() != PERIOD {
            return Err(Error::Bounds);
        }
        if self.rx_used == self.rx_avail || self.rx_used == self.used_idx(RX) {
            return Ok(None);
        }
        let slot = self.rx_used % CAPTURE_PERIODS;
        if self.used_id(RX, self.rx_used) != u32::from(slot) * 2 {
            return Err(Error::Malformed);
        }
        let written =
            io::read32(self.virt + RX.used + 4 + u64::from(self.rx_used % RX.size) * 8 + 4)
                as usize;
        let data = written.saturating_sub(8).min(PERIOD);
        let base = CAPTURE_SLOTS + u64::from(slot) * SLOT;
        for (i, b) in out.iter_mut().enumerate() {
            *b = if i < data {
                io::read8(self.virt + base + CAPTURE_DATA + i as u64)
            } else {
                0
            };
        }
        let ok = io::read32(self.virt + base + CAPTURE_STATUS) == S_OK;
        self.rx_used = self.rx_used.wrapping_add(1);
        Ok(Some(ok))
    }
    /// Queue one period for `stream`.
    fn queue(&mut self, stream: u32, pcm: &[u8]) -> Result<()> {
        if pcm.len() != PERIOD {
            return Err(Error::Bounds);
        }
        if self.queued() >= PERIODS {
            return Err(Error::QueueFull);
        }
        let slot = self.tx_avail % PERIODS;
        let base = SLOTS + u64::from(slot) * SLOT;
        io::write32(self.virt + base, stream);
        for (i, b) in pcm.iter().enumerate() {
            io::write8(self.virt + base + 4 + i as u64, *b);
        }
        io::write32(self.virt + base + STATUS, 0);
        let head = slot * 2;
        self.descriptor(TX, head, self.phys + base, 4 + PERIOD as u32, 1, head + 1);
        self.descriptor(TX, head + 1, self.phys + base + STATUS, 8, 2, 0);
        self.publish(TX, self.tx_avail, head);
        self.tx_avail = self.tx_avail.wrapping_add(1);
        Ok(())
    }
}

pub struct VirtioSound {
    transport: Transport,
    rings: Rings,
    notify: [u64; 3],
    stream: u32,
    running: bool,
    /// The input stream used for capture, when the device has one.
    input: Option<u32>,
    capturing: bool,
    statistics: Statistics,
}
impl VirtioSound {
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
        let mut notify = [0u64; 3];
        if let Err(error) =
            transport.initialize_queues(REQUIRED, REJECTED, &[CONTROL, TX, RX], &mut notify)
        {
            let _ = transport.reset();
            return Err(error);
        }
        let streams = io::read32(transport.device_config() + 4).min(8);
        let mut sound = Self {
            rings: Rings {
                virt: transport.virtual_dma(),
                phys: transport.physical_dma(),
                ctl_avail: 0,
                ctl_used: 0,
                tx_avail: 0,
                tx_used: 0,
                rx_avail: 0,
                rx_used: 0,
            },
            transport,
            notify,
            stream: 0,
            running: false,
            input: None,
            capturing: false,
            statistics: Statistics::default(),
        };
        match sound.open_output(streams) {
            Ok(()) => Ok(sound),
            Err(error) => {
                let _ = sound.transport.reset();
                Err(error)
            }
        }
    }
    /// Pick the first output stream that plays S16 stereo at 48 kHz, set
    /// its parameters and prepare it; remember the first input stream that
    /// records the same format (capture is optional).
    fn open_output(&mut self, streams: u32) -> Result<()> {
        let mut query = [0u8; 16];
        query[..4].copy_from_slice(&R_PCM_INFO.to_le_bytes());
        query[8..12].copy_from_slice(&streams.to_le_bytes());
        query[12..16].copy_from_slice(&32u32.to_le_bytes());
        self.command(&query, 4 + 32 * streams)?;
        let usable = |sound: &Self, i: u32, direction: u8| {
            let at = sound.rings.virt + RESPONSE + 4 + u64::from(i) * 32;
            let formats = u64::from(io::read32(at + 8)) | u64::from(io::read32(at + 12)) << 32;
            let rates = u64::from(io::read32(at + 16)) | u64::from(io::read32(at + 20)) << 32;
            io::read8(at + 24) == direction
                && formats & 1 << FORMAT_S16 != 0
                && rates & 1 << RATE_48000 != 0
                && (io::read8(at + 25)..=io::read8(at + 26)).contains(&CHANNELS)
        };
        let stream = (0..streams)
            .find(|i| usable(self, *i, OUTPUT))
            .ok_or(Error::Unsupported)?;
        self.input = (0..streams).find(|i| usable(self, *i, INPUT));
        self.stream = stream;
        self.set_params(stream, PERIODS)?;
        self.stream_command(R_PCM_PREPARE)
    }
    /// S16 stereo at 48 kHz in `periods` periods of 10 ms.
    fn set_params(&mut self, stream: u32, periods: u16) -> Result<()> {
        let mut params = [0u8; 24];
        params[..4].copy_from_slice(&R_PCM_SET_PARAMS.to_le_bytes());
        params[4..8].copy_from_slice(&stream.to_le_bytes());
        params[8..12].copy_from_slice(&(u32::from(periods) * PERIOD as u32).to_le_bytes());
        params[12..16].copy_from_slice(&(PERIOD as u32).to_le_bytes());
        params[20] = CHANNELS;
        params[21] = FORMAT_S16;
        params[22] = RATE_48000;
        self.command(&params, 4)
    }
    fn stream_command_for(&mut self, code: u32, stream: u32) -> Result<()> {
        let mut request = [0u8; 8];
        request[..4].copy_from_slice(&code.to_le_bytes());
        request[4..].copy_from_slice(&stream.to_le_bytes());
        self.command(&request, 4)
    }
    fn stream_command(&mut self, code: u32) -> Result<()> {
        self.stream_command_for(code, self.stream)
    }
    /// One synchronous control request (polled with a deadline).
    fn command(&mut self, request: &[u8], response: u32) -> Result<()> {
        self.transport.check()?;
        self.rings.post_control(request, response);
        self.transport.notify_queue(self.notify[0], CONTROL.index);
        let start = self.transport.clock();
        let mut spins = 0usize;
        loop {
            if let Some(status) = self.rings.control_done() {
                return if status == S_OK {
                    Ok(())
                } else {
                    Err(Error::Unsupported)
                };
            }
            spins += 1;
            if spins.is_multiple_of(CLOCK_EVERY) && self.transport.expired(start, spins) {
                return Err(Error::Timeout);
            }
            core::hint::spin_loop();
        }
    }
    pub fn address(&self) -> super::pci::Address {
        self.transport.address()
    }
    pub fn statistics(&self) -> Statistics {
        self.statistics
    }
    pub fn running(&self) -> bool {
        self.running
    }
    /// Periods queued and not yet played (after reclaiming finished ones).
    pub fn queued(&mut self) -> Result<u16> {
        self.transport.check()?;
        let (played, failed) = self.rings.reclaim()?;
        self.statistics.played += played;
        self.statistics.failed += failed;
        Ok(self.rings.queued())
    }
    /// Queue one period (`PERIOD` bytes of S16LE stereo).
    pub fn write(&mut self, pcm: &[u8]) -> Result<()> {
        self.queued()?;
        match self.rings.queue(self.stream, pcm) {
            Ok(()) => {
                self.transport.notify_queue(self.notify[1], TX.index);
                Ok(())
            }
            Err(Error::QueueFull) => {
                self.statistics.busy += 1;
                Err(Error::QueueFull)
            }
            Err(e) => Err(e),
        }
    }
    pub fn start(&mut self) -> Result<()> {
        if !self.running {
            self.stream_command(R_PCM_START)?;
            self.running = true;
        }
        Ok(())
    }
    pub fn stop(&mut self) -> Result<()> {
        if self.running {
            self.stream_command(R_PCM_STOP)?;
            self.running = false;
        }
        Ok(())
    }
    /// The device has an input stream to capture from.
    pub fn capture_present(&self) -> bool {
        self.input.is_some()
    }
    pub fn capturing(&self) -> bool {
        self.capturing
    }
    /// Prepare the input stream, offer every capture slot and start it.
    pub fn capture_start(&mut self) -> Result<()> {
        let stream = self.input.ok_or(Error::Unsupported)?;
        if self.capturing {
            return Ok(());
        }
        self.transport.check()?;
        self.set_params(stream, CAPTURE_PERIODS)?;
        self.stream_command_for(R_PCM_PREPARE, stream)?;
        while self.rings.capture_outstanding() < CAPTURE_PERIODS {
            self.rings.offer_capture(stream);
        }
        self.transport.notify_queue(self.notify[2], RX.index);
        self.stream_command_for(R_PCM_START, stream)?;
        self.capturing = true;
        Ok(())
    }
    /// Stop and release the input stream; the device returns every capture
    /// period it held, which are dropped.
    pub fn capture_stop(&mut self) -> Result<()> {
        let Some(stream) = self.input.filter(|_| self.capturing) else {
            return Ok(());
        };
        self.capturing = false;
        self.stream_command_for(R_PCM_STOP, stream)?;
        self.stream_command_for(R_PCM_RELEASE, stream)?;
        let mut scratch = [0u8; PERIOD];
        let start = self.transport.clock();
        let mut spins = 0usize;
        while self.rings.capture_outstanding() != 0 {
            if self.rings.take_capture(&mut scratch)?.is_none() {
                spins += 1;
                if spins.is_multiple_of(CLOCK_EVERY) && self.transport.expired(start, spins) {
                    // Capture stays off: slots the device kept are never offered again.
                    self.input = None;
                    return Err(Error::Timeout);
                }
                core::hint::spin_loop();
            }
        }
        Ok(())
    }
    /// The next captured period (`PERIOD` bytes of S16LE stereo) into `out`:
    /// false while none is waiting or capture is stopped. A period the device
    /// returned with an error status reads as silence and is counted.
    pub fn capture_read(&mut self, out: &mut [u8]) -> Result<bool> {
        let Some(stream) = self.input.filter(|_| self.capturing) else {
            return Ok(false);
        };
        self.transport.check()?;
        match self.rings.take_capture(out)? {
            None => Ok(false),
            Some(ok) => {
                if ok {
                    self.statistics.captured += 1;
                } else {
                    self.statistics.capture_failed += 1;
                    out.fill(0);
                }
                self.rings.offer_capture(stream);
                self.transport.notify_queue(self.notify[2], RX.index);
                Ok(true)
            }
        }
    }
    pub fn shutdown(mut self) -> Result<Resources> {
        let _ = self.capture_stop();
        let _ = self.stop();
        let _ = self.stream_command(R_PCM_RELEASE);
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
            phys: 0x2000_0000,
            ctl_avail: 0,
            ctl_used: 0,
            tx_avail: 0,
            tx_used: 0,
            rx_avail: 0,
            rx_used: 0,
        }
    }
    fn device_used_len(base: u64, q: QueueLayout, idx: u16, id: u32, len: u32) {
        let at = base + q.used + 4 + u64::from(idx % q.size) * 8;
        io::write32(at, id);
        io::write32(at + 4, len);
        io::write16(base + q.used + 2, idx.wrapping_add(1));
    }
    fn device_used(base: u64, q: QueueLayout, idx: u16, id: u32) {
        let at = base + q.used + 4 + u64::from(idx % q.size) * 8;
        io::write32(at, id);
        io::write32(at + 4, 8);
        io::write16(base + q.used + 2, idx.wrapping_add(1));
    }
    #[test]
    fn control_requests_chain_a_readable_request_to_a_writable_reply() {
        let (_keep, base) = region();
        let mut r = rings(base);
        r.post_control(&[4, 1, 0, 0, 7, 0, 0, 0], 4);
        // Descriptor 0: the 8-byte request, NEXT to descriptor 1.
        assert_eq!(io::read32(base + CONTROL.desc + 8), 8);
        assert_eq!(io::read16(base + CONTROL.desc + 12), 1);
        assert_eq!(io::read16(base + CONTROL.desc + 14), 1);
        // Descriptor 1: the device-writable 4-byte status.
        assert_eq!(io::read32(base + CONTROL.desc + 16 + 8), 4);
        assert_eq!(io::read16(base + CONTROL.desc + 16 + 12), 2);
        assert_eq!(r.control_done(), None, "pending until the device answers");
        io::write32(base + RESPONSE, S_OK);
        device_used(base, CONTROL, 0, 0);
        assert_eq!(r.control_done(), Some(S_OK));
    }
    #[test]
    fn periods_queue_in_order_until_the_device_is_full_and_reclaim_in_order() {
        let (_keep, base) = region();
        let mut r = rings(base);
        assert_eq!(
            r.queue(0, &[0; 100]),
            Err(Error::Bounds),
            "whole periods only"
        );
        let period = [0x5au8; PERIOD];
        for _ in 0..PERIODS {
            r.queue(3, &period).unwrap();
        }
        assert_eq!(r.queue(3, &period), Err(Error::QueueFull));
        assert_eq!(r.queued(), PERIODS);
        // Slot 0: stream id, then the PCM; its status after the period.
        assert_eq!(io::read32(base + SLOTS), 3);
        assert_eq!(io::read8(base + SLOTS + 4), 0x5a);
        assert_eq!(io::read32(base + TX.desc + 8), 4 + PERIOD as u32);
        assert_eq!(io::read16(base + TX.desc + 12), 1);
        let low = u64::from(io::read32(base + TX.desc + 16));
        let high = u64::from(io::read32(base + TX.desc + 20));
        assert_eq!(low | high << 32, 0x2000_0000 + SLOTS + STATUS);
        // The device plays two periods: one fine, one failed.
        io::write32(base + SLOTS + STATUS, S_OK);
        device_used(base, TX, 0, 0);
        io::write32(base + SLOTS + SLOT + STATUS, 0x8003);
        device_used(base, TX, 1, 2);
        assert_eq!(r.reclaim(), Ok((1, 1)));
        assert_eq!(r.queued(), PERIODS - 2);
        r.queue(3, &period).unwrap();
        // A completion out of order is a device fault.
        device_used(base, TX, 2, 8);
        assert_eq!(r.reclaim(), Err(Error::Malformed));
    }
    #[test]
    fn capture_periods_are_offered_and_taken_back_in_order() {
        let (_keep, base) = region();
        let mut r = rings(base);
        for _ in 0..CAPTURE_PERIODS {
            r.offer_capture(1);
        }
        assert_eq!(r.capture_outstanding(), CAPTURE_PERIODS);
        // Slot 0: the 4-byte header (stream 1), then the writable period
        // and status in one descriptor.
        assert_eq!(io::read32(base + CAPTURE_SLOTS), 1);
        assert_eq!(io::read32(base + RX.desc + 8), 4);
        assert_eq!(io::read16(base + RX.desc + 12), 1);
        assert_eq!(io::read32(base + RX.desc + 16 + 8), PERIOD as u32 + 8);
        assert_eq!(io::read16(base + RX.desc + 16 + 12), 2);
        let low = u64::from(io::read32(base + RX.desc + 16));
        let high = u64::from(io::read32(base + RX.desc + 20));
        assert_eq!(low | high << 32, 0x2000_0000 + CAPTURE_SLOTS + CAPTURE_DATA);
        let mut out = [0xffu8; PERIOD];
        assert_eq!(
            r.take_capture(&mut out),
            Ok(None),
            "pending until the device returns it"
        );
        assert_eq!(r.take_capture(&mut [0; 4]), Err(Error::Bounds));
        // The device fills slot 0 completely and slot 1 with half a period
        // and an error status.
        for i in 0..PERIOD as u64 {
            io::write8(base + CAPTURE_SLOTS + CAPTURE_DATA + i, 7);
        }
        io::write32(base + CAPTURE_SLOTS + CAPTURE_STATUS, S_OK);
        device_used_len(base, RX, 0, 0, PERIOD as u32 + 8);
        assert_eq!(r.take_capture(&mut out), Ok(Some(true)));
        assert!(out.iter().all(|b| *b == 7));
        for i in 0..PERIOD as u64 / 2 {
            io::write8(base + CAPTURE_SLOTS + SLOT + CAPTURE_DATA + i, 9);
        }
        io::write32(base + CAPTURE_SLOTS + SLOT + CAPTURE_STATUS, 0x8003);
        device_used_len(base, RX, 1, 2, PERIOD as u32 / 2 + 8);
        assert_eq!(r.take_capture(&mut out), Ok(Some(false)));
        assert!(
            out[..PERIOD / 2].iter().all(|b| *b == 9) && out[PERIOD / 2..].iter().all(|b| *b == 0)
        );
        // Re-offering reuses the oldest slot.
        r.offer_capture(1);
        assert_eq!(
            io::read16(base + RX.avail + 4 + u64::from(16 % RX.size) * 2),
            0
        );
        // A completion out of order is a device fault.
        device_used_len(base, RX, 2, 8, 8);
        assert_eq!(r.take_capture(&mut out), Err(Error::Malformed));
    }
    #[test]
    fn layout_fits_the_dma_region() {
        assert!(CONTROL.used + 6 + 8 * u64::from(CONTROL.size) <= TX.desc);
        assert!(TX.used + 6 + 8 * u64::from(TX.size) <= REQUEST);
        assert!(STATUS >= 4 + PERIOD as u64 && STATUS + 8 <= SLOT);
        assert_eq!(SLOTS + u64::from(PERIODS) * SLOT, RX.desc);
        assert!(RX.used + 6 + 8 * u64::from(RX.size) <= CAPTURE_SLOTS);
        assert_eq!(
            DMA_PAGES * 4096,
            CAPTURE_SLOTS + u64::from(CAPTURE_PERIODS) * SLOT
        );
    }
}
