//! amsi.dll exports: a context and sessions with no provider; scans are
//! not detected.
use crate::{E_INVALIDARG, NOT_DETECTED, S_OK, SIGNATURE};
use core::sync::atomic::{AtomicU32, Ordering};
use door_windows_abi::{narrow, widen};

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

/// The one context (HAMSICONTEXT): its signature, then nothing a caller
/// reads.
static CONTEXT: [u8; 16] = [
    SIGNATURE[0],
    SIGNATURE[1],
    SIGNATURE[2],
    SIGNATURE[3],
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
    0,
];
/// Session numbers (HAMSISESSION), never 0.
static SESSIONS: AtomicU32 = AtomicU32::new(0);

fn context() -> u64 {
    CONTEXT.as_ptr() as u64
}
fn put(pointer: u64, value: u64, wide: bool) {
    // SAFETY: the caller's out parameter (a handle or an AMSI_RESULT).
    unsafe {
        if wide {
            (pointer as *mut usize).write_unaligned(narrow(value));
        } else {
            (pointer as *mut u32).write_unaligned(value as u32);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn AmsiInitialize(app_name: usize, out: usize) -> u32 {
    let (app_name, out) = (widen(app_name), widen(out));
    if app_name == 0 || out == 0 {
        return E_INVALIDARG;
    }
    put(out, context(), true);
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn AmsiUninitialize(_context: usize) {}
#[unsafe(no_mangle)]
pub extern "system" fn AmsiOpenSession(context_handle: usize, out: usize) -> u32 {
    let (context_handle, out) = (widen(context_handle), widen(out));
    if context_handle != context() || out == 0 {
        return E_INVALIDARG;
    }
    let session = SESSIONS
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1)
        .max(1);
    put(out, u64::from(session), true);
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn AmsiCloseSession(_context: usize, _session: usize) {}
#[unsafe(no_mangle)]
pub extern "system" fn AmsiScanBuffer(
    context_handle: usize,
    buffer: usize,
    length: u32,
    _content_name: usize,
    _session: usize,
    result: usize,
) -> u32 {
    let (context_handle, buffer, result) = (widen(context_handle), widen(buffer), widen(result));
    if context_handle != context() || (buffer == 0 && length != 0) || result == 0 {
        return E_INVALIDARG;
    }
    put(result, u64::from(NOT_DETECTED), false);
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn AmsiScanString(
    context_handle: usize,
    string: usize,
    _content_name: usize,
    _session: usize,
    result: usize,
) -> u32 {
    let (context_handle, string, result) = (widen(context_handle), widen(string), widen(result));
    if context_handle != context() || string == 0 || result == 0 {
        return E_INVALIDARG;
    }
    put(result, u64::from(NOT_DETECTED), false);
    S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn AmsiNotifyOperation(
    context_handle: usize,
    _buffer: usize,
    _length: u32,
    _content_name: usize,
    result: usize,
) -> u32 {
    let (context_handle, result) = (widen(context_handle), widen(result));
    if context_handle != context() || result == 0 {
        return E_INVALIDARG;
    }
    put(result, u64::from(NOT_DETECTED), false);
    S_OK
}
