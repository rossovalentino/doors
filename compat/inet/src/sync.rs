//! A spin lock for the DLLs' handle tables and request state (no OS
//! mutex is needed for these short critical sections).
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, Ordering};

pub struct Lock<T> {
    held: AtomicBool,
    value: UnsafeCell<T>,
}
unsafe impl<T: Send> Sync for Lock<T> {}
unsafe impl<T: Send> Send for Lock<T> {}
impl<T> Lock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            held: AtomicBool::new(false),
            value: UnsafeCell::new(value),
        }
    }
    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self.held.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        let r = f(unsafe { &mut *self.value.get() });
        self.held.store(false, Ordering::Release);
        r
    }
}
