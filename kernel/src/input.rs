//! Bounded SPSC raw-device queue. IRQ-off BSP timer/VM-progress paths share one
//! producer; the exclusive display syscall consumes. Atomics publish whole slots;
//! overflow is sticky and invalidates the display input epoch instead of losing
//! a key-up silently. Reset requires both sides quiescent (IRQs off on the BSP).
use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicUsize, Ordering};
const CAPACITY: usize = 512;
struct Slot {
    value: AtomicU16,
    time: AtomicU64,
}
impl Slot {
    const fn new() -> Self {
        Self {
            value: AtomicU16::new(0),
            time: AtomicU64::new(0),
        }
    }
}
pub struct RawQueue {
    slots: [Slot; CAPACITY],
    head: AtomicUsize,
    tail: AtomicUsize,
    lost: AtomicBool,
}
impl Default for RawQueue {
    fn default() -> Self {
        Self::new()
    }
}
impl RawQueue {
    pub const fn new() -> Self {
        Self {
            slots: [const { Slot::new() }; CAPACITY],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
            lost: AtomicBool::new(false),
        }
    }
    pub fn push(&self, value: u16, time: u64) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let next = (head + 1) % CAPACITY;
        if next == self.tail.load(Ordering::Acquire) {
            self.lost.store(true, Ordering::Release);
            return false;
        }
        self.slots[head].value.store(value, Ordering::Relaxed);
        self.slots[head].time.store(time, Ordering::Relaxed);
        self.head.store(next, Ordering::Release);
        true
    }
    pub fn pop(&self) -> Result<Option<(u16, u64)>, Overflow> {
        if self.lost.load(Ordering::Acquire) {
            return Err(Overflow);
        }
        let tail = self.tail.load(Ordering::Relaxed);
        if tail == self.head.load(Ordering::Acquire) {
            return Ok(None);
        }
        let value = self.slots[tail].value.load(Ordering::Relaxed);
        let time = self.slots[tail].time.load(Ordering::Relaxed);
        self.tail.store((tail + 1) % CAPACITY, Ordering::Release);
        Ok(Some((value, time)))
    }
    pub fn reset_quiescent(&self) {
        self.head.store(0, Ordering::Relaxed);
        self.tail.store(0, Ordering::Relaxed);
        self.lost.store(false, Ordering::Release);
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overflow;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_slots_survive_wrap_and_overflow_invalidates_epoch() {
        let q = RawQueue::new();
        for batch in 0..8 {
            for n in 0..400 {
                assert!(q.push(n, n as u64 + batch * 400));
            }
            for n in 0..400 {
                assert_eq!(q.pop(), Ok(Some((n, n as u64 + batch * 400))));
            }
            assert_eq!(q.pop(), Ok(None));
        }
        for n in 0..511 {
            assert!(q.push(n, 10));
        }
        assert!(!q.push(0x9e, 11));
        assert_eq!(q.pop(), Err(Overflow));
        q.reset_quiescent();
        assert_eq!(q.pop(), Ok(None));
        assert!(q.push(0x9e, 12));
        assert_eq!(q.pop(), Ok(Some((0x9e, 12))));
    }
}
