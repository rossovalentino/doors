//! VirtIO block class, using modern PCI split queues and owned bounce buffers.
//! A write completion is volatile until a subsequent FLUSH completes.
use super::{
    Error, Result,
    pci::Function,
    queue::{QUEUE_SIZE, Queue, SLOTS, Token, completion_count},
    virtio_pci::{self as io, AVAIL, Transport, USED},
};
use crate::{
    mm::MappedRegion,
    storage::{BlockDevice, Geometry},
};
use core::sync::atomic::{Ordering, fence};

pub use super::virtio_pci::Resources;
pub const DMA_PAGES: u64 = 8;
pub const MAX_TRANSFER: usize = 4096;
const REQUIRED: u64 = (1 << 32) | (1 << 9); // VERSION_1 and FLUSH
const READ_ONLY: u64 = 1 << 5;
/// Spins between reads of the device status register and the clock.
const CHECK_EVERY: usize = 1024;
/// A request the device has not completed after this long is a hung device
/// (it is reset). Generous: a flush of a large image on a loaded host takes
/// seconds.
const REQUEST_TIMEOUT_MS: u64 = 30_000;

#[derive(Clone, Copy, Debug, Default)]
pub struct Statistics {
    pub reads: u64,
    pub writes: u64,
    pub flushes: u64,
    pub resets: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct SelfTest {
    pub queue_full: bool,
    pub cancellation: bool,
    pub reset: bool,
    pub stale: bool,
}
pub struct VirtioBlock {
    transport: Transport,
    queue: Queue,
    capacity: u64,
    available: u16,
    used: u16,
    statistics: Statistics,
}
impl VirtioBlock {
    /// # Safety
    /// Caller exclusively owns the function and uncached BAR mappings, which
    /// must match Function::bars(), and coherent contiguous RW/NX DMA backing.
    pub unsafe fn initialize(
        function: Function,
        bars: [Option<MappedRegion>; 6],
        dma: MappedRegion,
        clock: fn() -> u64,
    ) -> Result<Self> {
        let mut transport = unsafe { Transport::new(function, bars, dma, clock) }?;
        if let Err(error) = transport.initialize(REQUIRED, READ_ONLY) {
            let _ = transport.reset();
            return Err(error);
        }
        let capacity = transport.capacity()?;
        Ok(Self {
            transport,
            queue: Queue::new(),
            capacity,
            available: 0,
            used: 0,
            statistics: Statistics::default(),
        })
    }
    pub fn statistics(&self) -> Statistics {
        self.statistics
    }
    pub const fn negotiated_features(&self) -> u64 {
        REQUIRED
    }
    pub fn address(&self) -> super::pci::Address {
        self.transport.address()
    }
    fn header(&self, slot: usize) -> u64 {
        self.transport.virtual_dma() + 2048 + slot as u64 * 32
    }
    fn buffer(&self, slot: usize) -> u64 {
        self.transport.virtual_dma() + 4096 + slot as u64 * 4096
    }
    fn descriptor(&self, index: usize, address: u64, bytes: u32, flags: u16, next: u16) {
        let at = self.transport.virtual_dma() + index as u64 * 16;
        io::write64(at, address);
        io::write32(at + 8, bytes);
        io::write16(at + 12, flags);
        io::write16(at + 14, next);
    }
    fn prepare(&mut self, kind: u32, sector: u64, bytes: usize) -> Result<Token> {
        self.transport.check()?;
        if kind == 4 {
            if bytes != 0 || sector != 0 {
                return Err(Error::Configuration);
            }
        } else {
            if bytes == 0 || bytes > MAX_TRANSFER || !bytes.is_multiple_of(512) {
                return Err(Error::Alignment);
            }
            if sector
                .checked_add((bytes / 512) as u64)
                .is_none_or(|end| end > self.capacity)
            {
                return Err(Error::Bounds);
            }
        }
        let token = self
            .queue
            .reserve(if kind == 0 { bytes as u32 + 1 } else { 1 })?;
        let slot = token.slot;
        let head = slot * 3;
        let header = self.header(slot);
        io::write32(header, kind);
        io::write32(header + 4, 0);
        io::write64(header + 8, sector);
        io::write8(header + 16, 0xff);
        let physical = self.transport.physical_dma();
        self.descriptor(
            head,
            physical + 2048 + slot as u64 * 32,
            16,
            1,
            if bytes == 0 {
                (head + 2) as u16
            } else {
                (head + 1) as u16
            },
        );
        if bytes != 0 {
            self.descriptor(
                head + 1,
                physical + 4096 + slot as u64 * 4096,
                bytes as u32,
                1 | if kind == 0 { 2 } else { 0 },
                (head + 2) as u16,
            );
        }
        self.descriptor(head + 2, physical + 2064 + slot as u64 * 32, 1, 2, 0);
        Ok(token)
    }
    fn publish(&mut self, token: Token) -> Result<()> {
        self.queue.publish(token)?;
        let base = self.transport.virtual_dma() + AVAIL;
        io::write16(
            base + 4 + u64::from(self.available % QUEUE_SIZE) * 2,
            (token.slot * 3) as u16,
        );
        fence(Ordering::Release);
        self.available = self.available.wrapping_add(1);
        io::write16(base + 2, self.available);
        self.transport.notify();
        Ok(())
    }
    fn poll(&mut self, check_device: bool) -> Result<()> {
        if check_device {
            self.transport.check()?;
        }
        let base = self.transport.virtual_dma() + USED;
        let end = io::read16(base + 2);
        completion_count(self.used, end, self.queue.published())?;
        fence(Ordering::Acquire);
        while self.used != end {
            let at = base + 4 + u64::from(self.used % QUEUE_SIZE) * 8;
            let head = io::read32(at);
            let bytes = io::read32(at + 4);
            if !head.is_multiple_of(3) || head / 3 >= SLOTS as u32 {
                return Err(Error::Malformed);
            }
            let status = io::read8(self.header(head as usize / 3) + 16);
            self.queue.complete(head, bytes, status)?;
            self.used = self.used.wrapping_add(1);
        }
        Ok(())
    }
    /// Spin (interrupts are off) until `token` completes. The used ring is in
    /// RAM and is read on every spin. The device status register and the
    /// clock are device registers: under emulation every read traps into the
    /// hypervisor, and read on every spin those traps can hold off the very
    /// thread that completes the request (seen under QEMU TCG as the whole
    /// guest stopping at a disk write). They are read every `CHECK_EVERY`
    /// spins. The time limit is the PM timer's, which runs with interrupts
    /// off; the tick count does not.
    fn wait(&mut self, token: Token) -> Result<()> {
        let start = self.transport.clock();
        let mut spins: usize = 0;
        loop {
            let check = spins.is_multiple_of(CHECK_EVERY);
            if let Err(error) = self.poll(check) {
                self.quiesce()?;
                return Err(error);
            }
            if let Some(result) = self.queue.result(token)? {
                return result;
            }
            if check && self.transport.expired_after(start, REQUEST_TIMEOUT_MS) {
                self.quiesce()?;
                return Err(Error::Timeout);
            }
            spins = spins.wrapping_add(1);
            core::hint::spin_loop();
        }
    }
    fn quiesce(&mut self) -> Result<()> {
        self.queue.stop();
        self.transport.reset()?;
        self.queue.reset_acknowledged();
        self.statistics.resets += 1;
        Ok(())
    }
    fn request(
        &mut self,
        kind: u32,
        sector: u64,
        input: Option<&[u8]>,
        output: Option<&mut [u8]>,
    ) -> Result<()> {
        let bytes = input
            .as_ref()
            .map_or_else(|| output.as_ref().map_or(0, |v| v.len()), |v| v.len());
        let token = self.prepare(kind, sector, bytes)?;
        if let Some(input) = input {
            for (offset, value) in input.iter().enumerate() {
                io::write8(self.buffer(token.slot) + offset as u64, *value);
            }
        }
        self.publish(token)?;
        let result = self.wait(token);
        if result.is_ok() {
            if let Some(output) = output {
                for (offset, value) in output.iter_mut().enumerate() {
                    *value = io::read8(self.buffer(token.slot) + offset as u64);
                }
            }
            match kind {
                0 => self.statistics.reads += 1,
                1 => self.statistics.writes += 1,
                _ => self.statistics.flushes += 1,
            }
        }
        let reclaimed = self.queue.reap(token);
        result.and(reclaimed)
    }
    /// Exercises the actual driver's pending ownership, bounded capacity and a
    /// reset with a published scratch write. Completion is deliberately
    /// indeterminate; caller dedicates this sector outside every partition.
    pub fn selftest(&mut self, scratch_lba: u64) -> Result<SelfTest> {
        let first = self.prepare(0, 0, 512)?;
        let second = self.prepare(1, scratch_lba, 512)?;
        let third = self.prepare(0, 0, 512)?;
        let fourth = self.prepare(0, 0, 512)?;
        let queue_full = self.prepare(0, 0, 512) == Err(Error::QueueFull);
        self.queue.cancel(first)?;
        let cancellation = self.queue.reap(first) == Err(Error::Canceled);
        for offset in 0..512 {
            io::write8(self.buffer(second.slot) + offset, 0);
        }
        self.publish(second)?;
        self.quiesce()?;
        let reset = [second, third, fourth]
            .into_iter()
            .all(|token| self.queue.reap(token) == Err(Error::Reset));
        self.transport.initialize(REQUIRED, READ_ONLY)?;
        self.available = 0;
        self.used = 0;
        self.queue.restart();
        let next = self.prepare(0, 0, 512)?;
        let stale = self.queue.publish(first) == Err(Error::Stale);
        self.queue.cancel(next)?;
        if self.queue.reap(next) != Err(Error::Canceled)
            || !queue_full
            || !cancellation
            || !reset
            || !stale
        {
            return Err(Error::Malformed);
        }
        if self.transport.capacity()? != self.capacity {
            return Err(Error::Configuration);
        }
        Ok(SelfTest {
            queue_full,
            cancellation,
            reset,
            stale,
        })
    }
    /// Stops submissions, observes device reset and bus mastering disabled,
    /// then returns every owned token for explicit mapping/frame reclamation.
    pub fn shutdown(mut self) -> Result<Resources> {
        self.quiesce()?;
        self.transport.shutdown()
    }
}
impl BlockDevice for VirtioBlock {
    fn geometry(&self) -> Geometry {
        Geometry {
            sector_size: 512,
            sectors: self.capacity,
            read_only: false,
            flush_supported: true,
        }
    }
    fn read_sectors(
        &mut self,
        lba: u64,
        output: &mut [u8],
    ) -> core::result::Result<(), crate::storage::Error> {
        crate::storage::check_transfer(self.geometry(), lba, output.len())?;
        for (index, chunk) in output.chunks_mut(MAX_TRANSFER).enumerate() {
            self.request(
                0,
                lba + (index * MAX_TRANSFER / 512) as u64,
                None,
                Some(chunk),
            )
            .map_err(storage_error)?;
        }
        Ok(())
    }
    fn write_sectors(
        &mut self,
        lba: u64,
        input: &[u8],
    ) -> core::result::Result<(), crate::storage::Error> {
        crate::storage::check_transfer(self.geometry(), lba, input.len())?;
        for (index, chunk) in input.chunks(MAX_TRANSFER).enumerate() {
            self.request(
                1,
                lba + (index * MAX_TRANSFER / 512) as u64,
                Some(chunk),
                None,
            )
            .map_err(storage_error)?;
        }
        Ok(())
    }
    fn flush(&mut self) -> core::result::Result<(), crate::storage::Error> {
        self.request(4, 0, None, None).map_err(storage_error)
    }
}
fn storage_error(error: Error) -> crate::storage::Error {
    use crate::storage::Error as E;
    match error {
        Error::Bounds => E::Bounds,
        Error::Alignment => E::Alignment,
        Error::Unsupported => E::Unsupported,
        Error::ShortIo => E::ShortIo,
        Error::Timeout => E::Timeout,
        Error::Removed => E::DeviceLost,
        Error::QueueFull | Error::InFlight => E::Busy,
        Error::Malformed | Error::Configuration => E::Corrupt,
        _ => E::Io,
    }
}
