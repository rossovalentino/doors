//! Bounded process-local dynamic TLS index ownership. Values live in separate
//! per-thread storage, tagged by allocation epoch so reuse observes zero.
use core::sync::atomic::{AtomicU64, Ordering};

pub const SLOTS: usize = 64;
pub const FS_OFFSET: usize = 1024;
pub const SLOT_BYTES: usize = 16;
pub const OUT_OF_INDEXES: u32 = u32::MAX;
const _: () = assert!(FS_OFFSET + SLOTS * SLOT_BYTES <= crate::THREAD_TEB_FS_OFFSET);
pub struct Indexes {
    slots: [AtomicU64; SLOTS],
}
impl Default for Indexes {
    fn default() -> Self {
        Self::new()
    }
}
impl Indexes {
    pub const fn new() -> Self {
        Self {
            slots: [const { AtomicU64::new(0) }; SLOTS],
        }
    }
    pub fn allocate(&self) -> Option<u32> {
        for (index, slot) in self.slots.iter().enumerate() {
            let observed = slot.load(Ordering::Acquire);
            if observed & 1 == 0
                && observed < u64::MAX - 1
                && slot
                    .compare_exchange(observed, observed + 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                return Some(index as u32);
            }
        }
        None
    }
    pub fn free(&self, index: u32) -> bool {
        let Some(slot) = self.slots.get(index as usize) else {
            return false;
        };
        let observed = slot.load(Ordering::Acquire);
        observed & 1 != 0
            && observed < u64::MAX
            && slot
                .compare_exchange(observed, observed + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
    }
    /// Range validation follows the minimum64-index Windows fast-path contract.
    /// Unallocated numeric indexes have epoch0; callers own allocation discipline.
    pub fn epoch(&self, index: u32) -> Option<u64> {
        self.slots.get(index as usize).map(|slot| {
            let value = slot.load(Ordering::Acquire);
            if value & 1 != 0 { value } else { 0 }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocation_exhaustion_reuse_and_process_isolation() {
        let indexes = Indexes::new();
        let other = Indexes::new();
        for expected in 0..SLOTS {
            assert_eq!(indexes.allocate(), Some(expected as u32));
        }
        assert_eq!(indexes.allocate(), None);
        let epoch = indexes.epoch(7).unwrap();
        assert!(indexes.free(7));
        assert!(!indexes.free(7));
        assert_eq!(indexes.epoch(7), Some(0));
        assert_eq!(indexes.allocate(), Some(7));
        assert_ne!(indexes.epoch(7), Some(epoch));
        assert_eq!(other.epoch(7), Some(0));
        assert_eq!(indexes.epoch(64), None);
        assert!(!indexes.free(u32::MAX));
    }
    #[test]
    fn exhausted_generation_is_never_wrapped_into_a_stale_value() {
        let indexes = Indexes::new();
        for slot in &indexes.slots {
            slot.store(u64::MAX - 1, Ordering::Relaxed);
        }
        assert_eq!(indexes.allocate(), None);
        indexes.slots[0].store(u64::MAX, Ordering::Relaxed);
        assert!(!indexes.free(0));
    }
}
