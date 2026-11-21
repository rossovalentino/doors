//! pdh exports.

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

/// Clears the caller's out-pointer, if any.
fn clear(out: usize) {
    if out != 0 {
        // SAFETY: the caller's pointer-sized out-parameter.
        unsafe { (out as *mut usize).write_unaligned(0) };
    }
}
use core::sync::atomic::{AtomicUsize, Ordering};
const PDH_CSTATUS_NO_OBJECT: u32 = 0xC000_0BB8;
const PDH_INVALID_HANDLE: u32 = 0xC000_0BBC;
const PDH_INVALID_ARGUMENT: u32 = 0xC000_0BBD;
const PDH_NO_DATA: u32 = 0x8000_0BD5;
/// Queries: distinct handles with no counters.
static NEXT: AtomicUsize = AtomicUsize::new(0x9D40_0000);
#[unsafe(no_mangle)]
pub extern "system" fn PdhOpenQueryW(_source: usize, _data: usize, query: usize) -> u32 {
    if query == 0 {
        return PDH_INVALID_ARGUMENT;
    }
    let handle = NEXT.fetch_add(4, Ordering::Relaxed);
    // SAFETY: the caller's PDH_HQUERY out-parameter.
    unsafe { (query as *mut usize).write_unaligned(handle) };
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn PdhAddEnglishCounterW(
    query: usize,
    _path: usize,
    _data: usize,
    counter: usize,
) -> u32 {
    clear(counter);
    if query == 0 {
        PDH_INVALID_HANDLE
    } else {
        PDH_CSTATUS_NO_OBJECT
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn PdhAddCounterW(
    query: usize,
    path: usize,
    data: usize,
    counter: usize,
) -> u32 {
    PdhAddEnglishCounterW(query, path, data, counter)
}
/// No counters were added, so a collection has no data.
#[unsafe(no_mangle)]
pub extern "system" fn PdhCollectQueryData(query: usize) -> u32 {
    if query == 0 {
        PDH_INVALID_HANDLE
    } else {
        PDH_NO_DATA
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn PdhGetFormattedCounterValue(
    _counter: usize,
    _format: u32,
    _kind: usize,
    _value: usize,
) -> u32 {
    PDH_INVALID_HANDLE
}
#[unsafe(no_mangle)]
pub extern "system" fn PdhCloseQuery(query: usize) -> u32 {
    if query == 0 { PDH_INVALID_HANDLE } else { 0 }
}
