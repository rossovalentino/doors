//! Deferred process stops preserve kernel continuations until their safe point.
use super::{Error, MAX_TASKS, Scheduler, TaskId};
use core::sync::atomic::{AtomicI64, AtomicU64, Ordering};

pub struct StopRequests {
    generations: [AtomicU64; MAX_TASKS],
    codes: [AtomicI64; MAX_TASKS],
}
impl StopRequests {
    pub const fn new() -> Self {
        Self {
            generations: [const { AtomicU64::new(0) }; MAX_TASKS],
            codes: [const { AtomicI64::new(0) }; MAX_TASKS],
        }
    }
    /// Call under the scheduler lock. A stale process task snapshot must not
    /// overwrite a stop already published for a replacement in the same slot.
    /// New tasks retain this request through start; kernel continuations unwind
    /// waits before the runtime terminates them at a safe point.
    pub fn request(&self, scheduler: &mut Scheduler, id: TaskId, code: i64) -> Result<(), Error> {
        scheduler.snapshot(id)?;
        self.codes[id.slot()].store(code, Ordering::Relaxed);
        self.generations[id.slot()].store(id.generation(), Ordering::Release);
        scheduler.abort_wait_for_stop(id)?;
        Ok(())
    }
    /// The caller owns a live task or holds the scheduler lock, preventing its
    /// slot from being recycled while reading its matching generation/code.
    pub fn requested(&self, id: TaskId) -> Option<i64> {
        (self.generations[id.slot()].load(Ordering::Acquire) == id.generation())
            .then(|| self.codes[id.slot()].load(Ordering::Relaxed))
    }
}
impl Default for StopRequests {
    fn default() -> Self {
        Self::new()
    }
}
