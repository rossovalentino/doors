//! One-time initialization (InitOnce*, Vista's INIT_ONCE) on a
//! pointer-sized word: 0 not started, 1 running (other callers wait on the
//! word's address), or the context with 2 in its low bits once done. The
//! low two bits are reserved (INIT_ONCE_CTX_RESERVED_BITS), so a context
//! keeps the rest. A failed initialization leaves the word at 0 for the
//! next caller to try, as Windows does.
use core::sync::atomic::{AtomicUsize, Ordering};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorWaitOnAddress(address: usize, compare: usize, size: usize, millis: u32) -> i32;
    fn DoorWakeByAddressAll(address: usize);
    fn RtlSetLastWin32Error(value: u32);
}

const RUNNING: usize = 1;
const DONE: usize = 2;
const INIT_ONCE_CHECK_ONLY: u32 = 1;
const INIT_ONCE_ASYNC: u32 = 2;
const INIT_ONCE_INIT_FAILED: u32 = 4;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_GEN_FAILURE: u32 = 31;

fn fail(code: u32) -> i32 {
    unsafe { RtlSetLastWin32Error(code) };
    0
}
fn word<'a>(once: usize) -> &'a AtomicUsize {
    // SAFETY: the caller's INIT_ONCE, a pointer-sized, pointer-aligned word.
    unsafe { &*(once as *const AtomicUsize) }
}
/// Wait while another thread runs the initialization.
fn wait_running(once: usize) {
    let running = RUNNING;
    while word(once).load(Ordering::Acquire) == RUNNING {
        unsafe {
            DoorWaitOnAddress(
                once,
                &running as *const usize as usize,
                core::mem::size_of::<usize>(),
                u32::MAX,
            )
        };
    }
}
fn finish(once: usize, value: usize) {
    word(once).store(value, Ordering::Release);
    unsafe { DoorWakeByAddressAll(once) };
}

#[unsafe(no_mangle)]
pub extern "system" fn InitOnceInitialize(once: usize) {
    if once != 0 {
        word(once).store(0, Ordering::Release);
    }
}
/// `InitOnceExecuteOnce`: `function(once, parameter, &context)` runs on
/// the first call; later and concurrent calls return its result and
/// context.
#[unsafe(no_mangle)]
pub extern "system" fn InitOnceExecuteOnce(
    once: usize,
    function: usize,
    parameter: usize,
    context: usize,
) -> i32 {
    if once == 0 || function == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let function: extern "system" fn(usize, usize, usize) -> i32 =
        unsafe { core::mem::transmute(function) };
    loop {
        let value = word(once).load(Ordering::Acquire);
        if value & 3 == DONE {
            if context != 0 {
                unsafe { (context as *mut usize).write_unaligned(value & !3) };
            }
            return 1;
        }
        if value == RUNNING {
            wait_running(once);
            continue;
        }
        if word(once)
            .compare_exchange(0, RUNNING, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            continue;
        }
        let mut produced = 0usize;
        let ok = function(once, parameter, &mut produced as *mut usize as usize);
        if ok == 0 {
            finish(once, 0);
            return 0;
        }
        finish(once, (produced & !3) | DONE);
        if context != 0 {
            unsafe { (context as *mut usize).write_unaligned(produced & !3) };
        }
        return 1;
    }
}
/// `InitOnceBeginInitialize`: TRUE with `pending` set when the caller is
/// to initialize (and call InitOnceComplete), clear when it already is.
/// INIT_ONCE_CHECK_ONLY never starts it (ERROR_GEN_FAILURE if not done).
/// Asynchronous initialization is not offered here.
#[unsafe(no_mangle)]
pub extern "system" fn InitOnceBeginInitialize(
    once: usize,
    flags: u32,
    pending: usize,
    context: usize,
) -> i32 {
    if once == 0 || pending == 0 || flags & !(INIT_ONCE_CHECK_ONLY | INIT_ONCE_ASYNC) != 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    if flags & INIT_ONCE_ASYNC != 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    loop {
        let value = word(once).load(Ordering::Acquire);
        if value & 3 == DONE {
            unsafe { (pending as *mut i32).write_unaligned(0) };
            if context != 0 {
                unsafe { (context as *mut usize).write_unaligned(value & !3) };
            }
            return 1;
        }
        if flags & INIT_ONCE_CHECK_ONLY != 0 {
            unsafe { (pending as *mut i32).write_unaligned(1) };
            return fail(ERROR_GEN_FAILURE);
        }
        if value == RUNNING {
            wait_running(once);
            continue;
        }
        if word(once)
            .compare_exchange(0, RUNNING, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            unsafe { (pending as *mut i32).write_unaligned(1) };
            return 1;
        }
    }
}
/// `InitOnceComplete`: the initialization that InitOnceBeginInitialize
/// started ends, with its context, or failed (INIT_ONCE_INIT_FAILED) for
/// the next caller to retry.
#[unsafe(no_mangle)]
pub extern "system" fn InitOnceComplete(once: usize, flags: u32, context: usize) -> i32 {
    if once == 0 || flags & !(INIT_ONCE_ASYNC | INIT_ONCE_INIT_FAILED) != 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    if word(once).load(Ordering::Acquire) != RUNNING {
        return fail(ERROR_GEN_FAILURE);
    }
    if flags & INIT_ONCE_INIT_FAILED != 0 {
        finish(once, 0);
    } else {
        finish(once, (context & !3) | DONE);
    }
    1
}
