//! Modern PCI transport, independent of block request formatting.
use super::{Error, Result, pci::Function, queue::QUEUE_SIZE};
use crate::mm::MappedRegion;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{Ordering, fence};

pub const DMA_BYTES: u64 = 8 * 4096;
pub const AVAIL: u64 = 512;
pub const USED: u64 = 1024;
const SPIN_LIMIT: usize = 10_000_000;
const TIMEOUT_MS: u64 = 2000;

fn deadline_expired(start: u64, now: u64, iterations: usize) -> bool {
    iterations >= SPIN_LIMIT || now.wrapping_sub(start) >= TIMEOUT_MS
}

/// Returned only after reset acknowledgement and PCI bus-master disable.
pub struct Resources {
    pub bars: [Option<MappedRegion>; 6],
    pub dma: MappedRegion,
}
/// One virtqueue of a multi-queue device: its index, size and the offsets
/// of its descriptor table, available ring and used ring in the DMA region.
#[derive(Clone, Copy, Debug)]
pub struct QueueLayout {
    pub index: u16,
    pub size: u16,
    pub desc: u64,
    pub avail: u64,
    pub used: u64,
}
pub struct Transport {
    function: Function,
    resources: Resources,
    dma_bytes: u64,
    common: u64,
    device: u64,
    notify: u64,
    clock: fn() -> u64,
    operational: bool,
}
impl Transport {
    /// # Safety
    /// BAR tokens must map exactly the discovered physical ranges uncached.
    /// DMA is exclusive, coherent, contiguous owned RAM, writable and NX. No
    /// other code/device may use these mappings until returned by shutdown.
    pub unsafe fn new(
        function: Function,
        bars: [Option<MappedRegion>; 6],
        dma: MappedRegion,
        clock: fn() -> u64,
    ) -> Result<Self> {
        unsafe { Self::new_sized(function, bars, dma, clock, DMA_BYTES) }
    }
    /// `new` with a DMA region of `dma_bytes` (whole pages) for devices
    /// with several queues and their buffers (network, sound).
    ///
    /// # Safety
    /// As `new`.
    pub unsafe fn new_sized(
        function: Function,
        bars: [Option<MappedRegion>; 6],
        dma: MappedRegion,
        clock: fn() -> u64,
        dma_bytes: u64,
    ) -> Result<Self> {
        if dma.bytes().get() != dma_bytes
            || !dma_bytes.is_multiple_of(4096)
            || dma.physical().is_none()
            || dma.start().get() & 4095 != 0
        {
            return Err(Error::Configuration);
        }
        for (spec, mapping) in function.bars().iter().zip(&bars) {
            match (spec, mapping) {
                (Some(spec), Some(mapping))
                    if mapping.bytes().get() == spec.bytes && mapping.start().get() & 4095 == 0 => {
                }
                (None, None) => {}
                _ => return Err(Error::Configuration),
            }
        }
        let address = |index: usize| {
            let c = function.caps.regions[index];
            bars[c.bar].as_ref().unwrap().start().get() + u64::from(c.offset)
        };
        let (common, device, notify) = (address(0), address(3), address(1));
        function.command(false, true)?;
        Ok(Self {
            function,
            resources: Resources { bars, dma },
            dma_bytes,
            common,
            device,
            notify,
            clock,
            operational: false,
        })
    }
    pub fn virtual_dma(&self) -> u64 {
        self.resources.dma.start().get()
    }
    pub fn physical_dma(&self) -> u64 {
        self.resources.dma.physical().unwrap().get()
    }
    pub fn address(&self) -> super::pci::Address {
        self.function.address()
    }
    pub fn clock(&self) -> u64 {
        (self.clock)()
    }
    pub fn expired(&self, start: u64, iterations: usize) -> bool {
        deadline_expired(start, self.clock(), iterations)
    }
    /// `limit_ms` passed since `start` (a request's own time limit). A clock
    /// that went backwards counts as expired, as in `expired`.
    pub fn expired_after(&self, start: u64, limit_ms: u64) -> bool {
        let now = self.clock();
        now < start || now - start >= limit_ms
    }
    pub fn status(&self) -> u8 {
        read8(self.common + 20)
    }
    pub fn check(&self) -> Result<()> {
        if !self.operational {
            return Err(Error::Removed);
        }
        let status = self.status();
        if status == 0xff || status == 0 {
            Err(Error::Removed)
        } else if status & 0xc0 != 0 || status & 0x0f != 0x0f {
            Err(Error::Io)
        } else {
            Ok(())
        }
    }
    pub fn reset(&mut self) -> Result<()> {
        self.operational = false;
        write8(self.common + 20, 0);
        let start = self.clock();
        for iteration in 0..=SPIN_LIMIT {
            if self.status() == 0 {
                self.function.command(false, true)?;
                fence(Ordering::SeqCst);
                return Ok(());
            }
            if iteration.is_multiple_of(1024) && self.expired(start, iteration) {
                // Even without reset acknowledgement, stop further DMA if the
                // function still responds. Backing is nevertheless retained.
                let _ = self.function.command(false, true);
                return Err(Error::Timeout);
            }
            core::hint::spin_loop();
        }
        Err(Error::Timeout)
    }
    pub fn initialize(&mut self, required: u64, rejected: u64) -> Result<u64> {
        self.reset()?;
        // All DMA memory is driver-owned after acknowledged reset.
        unsafe {
            core::ptr::write_bytes(self.virtual_dma() as *mut u8, 0, self.dma_bytes as usize);
        }
        write8(self.common + 20, 1);
        write8(self.common + 20, 3);
        write32(self.common, 0);
        let low = read32(self.common + 4);
        write32(self.common, 1);
        let high = read32(self.common + 4);
        let offered = (u64::from(high) << 32) | u64::from(low);
        if offered & required != required || offered & rejected != 0 {
            return Err(Error::Unsupported);
        }
        write32(self.common + 8, 0);
        write32(self.common + 12, required as u32);
        write32(self.common + 8, 1);
        write32(self.common + 12, (required >> 32) as u32);
        write8(self.common + 20, 11);
        if self.status() & 8 == 0 {
            return Err(Error::Unsupported);
        }
        if read16(self.common + 18) == 0 {
            return Err(Error::Configuration);
        }
        write16(self.common + 16, 0xffff);
        write16(self.common + 22, 0);
        let size = read16(self.common + 24);
        if size < QUEUE_SIZE || !size.is_power_of_two() || read16(self.common + 28) != 0 {
            return Err(Error::Configuration);
        }
        write16(self.common + 24, QUEUE_SIZE);
        write16(self.common + 26, 0xffff);
        let notify_offset =
            u64::from(read16(self.common + 30)) * u64::from(self.function.caps.multiplier);
        let cap = self.function.caps.regions[1];
        if notify_offset
            .checked_add(2)
            .is_none_or(|end| end > u64::from(cap.length))
        {
            return Err(Error::Configuration);
        }
        let base = self.resources.bars[cap.bar].as_ref().unwrap().start().get();
        self.notify = base + u64::from(cap.offset) + notify_offset;
        write64(self.common + 32, self.physical_dma());
        write64(self.common + 40, self.physical_dma() + AVAIL);
        write64(self.common + 48, self.physical_dma() + USED);
        write16(self.virtual_dma() + AVAIL, 1); // VIRTQ_AVAIL_F_NO_INTERRUPT
        write16(self.common + 28, 1);
        self.function.command(true, true)?;
        write8(self.common + 20, 15);
        self.operational = true;
        self.check()?;
        Ok(offered)
    }
    /// Negotiate `required` features and enable each of `queues` (offsets
    /// into the DMA region); `notify` receives each queue's notify address.
    /// Polled: MSI-X vectors stay unassigned and used-ring interrupts off.
    pub fn initialize_queues(
        &mut self,
        required: u64,
        rejected: u64,
        queues: &[QueueLayout],
        notify: &mut [u64],
    ) -> Result<u64> {
        if notify.len() < queues.len() {
            return Err(Error::Configuration);
        }
        self.reset()?;
        unsafe {
            core::ptr::write_bytes(self.virtual_dma() as *mut u8, 0, self.dma_bytes as usize);
        }
        write8(self.common + 20, 1);
        write8(self.common + 20, 3);
        write32(self.common, 0);
        let low = read32(self.common + 4);
        write32(self.common, 1);
        let high = read32(self.common + 4);
        let offered = (u64::from(high) << 32) | u64::from(low);
        if offered & required != required || offered & rejected != 0 {
            return Err(Error::Unsupported);
        }
        write32(self.common + 8, 0);
        write32(self.common + 12, required as u32);
        write32(self.common + 8, 1);
        write32(self.common + 12, (required >> 32) as u32);
        write8(self.common + 20, 11);
        if self.status() & 8 == 0 {
            return Err(Error::Unsupported);
        }
        write16(self.common + 16, 0xffff);
        let cap = self.function.caps.regions[1];
        let base = self.resources.bars[cap.bar].as_ref().unwrap().start().get();
        for (q, slot) in queues.iter().zip(notify.iter_mut()) {
            let end = q.used + 6 + 8 * u64::from(q.size);
            if q.size == 0
                || !q.size.is_power_of_two()
                || end > self.dma_bytes
                || q.desc % 16 != 0
                || q.avail % 2 != 0
                || q.used % 4 != 0
            {
                return Err(Error::Configuration);
            }
            write16(self.common + 22, q.index);
            let size = read16(self.common + 24);
            if size < q.size || read16(self.common + 28) != 0 {
                return Err(Error::Configuration);
            }
            write16(self.common + 24, q.size);
            write16(self.common + 26, 0xffff);
            let offset =
                u64::from(read16(self.common + 30)) * u64::from(self.function.caps.multiplier);
            if offset
                .checked_add(2)
                .is_none_or(|end| end > u64::from(cap.length))
            {
                return Err(Error::Configuration);
            }
            *slot = base + u64::from(cap.offset) + offset;
            let physical = self.physical_dma();
            write64(self.common + 32, physical + q.desc);
            write64(self.common + 40, physical + q.avail);
            write64(self.common + 48, physical + q.used);
            write16(self.virtual_dma() + q.avail, 1); // VIRTQ_AVAIL_F_NO_INTERRUPT
            write16(self.common + 28, 1);
        }
        self.function.command(true, true)?;
        write8(self.common + 20, 15);
        self.operational = true;
        self.check()?;
        Ok(offered)
    }
    /// The device-specific configuration structure (MAC and link status of
    /// a network device, the stream counts of a sound device).
    pub fn device_config(&self) -> u64 {
        self.device
    }
    /// Notify queue at `address` (from `initialize_queues`).
    pub fn notify_queue(&self, address: u64, index: u16) {
        fence(Ordering::SeqCst);
        write16(address, index);
    }
    pub fn capacity(&self) -> Result<u64> {
        for _ in 0..32 {
            let generation = read8(self.common + 21);
            let low = read32(self.device);
            let high = read32(self.device + 4);
            if generation == read8(self.common + 21) {
                let capacity = (u64::from(high) << 32) | u64::from(low);
                return if capacity == 0 {
                    Err(Error::Configuration)
                } else {
                    Ok(capacity)
                };
            }
        }
        Err(Error::Io)
    }
    pub fn notify(&self) {
        fence(Ordering::SeqCst);
        write16(self.notify, 0);
    }
    pub fn shutdown(mut self) -> Result<Resources> {
        self.reset()?;
        self.function.command(false, false)?;
        Ok(self.resources)
    }
}
// All addresses originate from retained, validated mappings or DMA offsets.
pub fn read8(address: u64) -> u8 {
    unsafe { read_volatile(address as *const u8) }
}
pub fn read16(address: u64) -> u16 {
    unsafe { read_volatile(address as *const u16) }
}
pub fn read32(address: u64) -> u32 {
    unsafe { read_volatile(address as *const u32) }
}
pub fn write8(address: u64, value: u8) {
    unsafe { write_volatile(address as *mut u8, value) }
}
pub fn write16(address: u64, value: u16) {
    unsafe { write_volatile(address as *mut u16, value) }
}
pub fn write32(address: u64, value: u32) {
    unsafe { write_volatile(address as *mut u32, value) }
}
pub fn write64(address: u64, value: u64) {
    write32(address, value as u32);
    write32(address + 4, (value >> 32) as u32);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stalled_clock_cannot_prevent_timeout_and_real_deadline_is_inclusive() {
        assert!(!deadline_expired(100, 2099, 1));
        assert!(deadline_expired(100, 2100, 1));
        assert!(!deadline_expired(100, 100, SPIN_LIMIT - 1));
        assert!(deadline_expired(100, 100, SPIN_LIMIT));
    }
    #[test]
    fn clock_wrap_does_not_extend_a_hardware_request_indefinitely() {
        let start = u64::MAX - 999;
        assert!(!deadline_expired(start, 999, 1));
        assert!(deadline_expired(start, 1000, 1));
        // A faulty backwards clock fails closed rather than extending a lease.
        assert!(deadline_expired(100, 99, 1));
    }
}
