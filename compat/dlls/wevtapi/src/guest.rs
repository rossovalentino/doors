//! wevtapi exports.

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "kernel32", kind = "dylib")]
unsafe extern "system" {
    fn SetLastError(code: u32);
}
fn fail<T: Default>(code: u32) -> T {
    unsafe { SetLastError(code) };
    T::default()
}
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_NOT_SUPPORTED: u32 = 50;
const ERROR_NO_MORE_ITEMS: u32 = 259;
const ERROR_EVT_CHANNEL_NOT_FOUND: u32 = 15007;
#[unsafe(no_mangle)]
pub extern "system" fn EvtQuery(
    _session: usize,
    _path: usize,
    _query: usize,
    _flags: u32,
) -> usize {
    fail(ERROR_EVT_CHANNEL_NOT_FOUND)
}
#[unsafe(no_mangle)]
pub extern "system" fn EvtNext(
    _results: usize,
    _count: u32,
    events: usize,
    _timeout: u32,
    _flags: u32,
    returned: usize,
) -> i32 {
    let _ = events;
    if returned != 0 {
        // SAFETY: the caller's DWORD out-parameter.
        unsafe { (returned as *mut u32).write_unaligned(0) };
    }
    fail(ERROR_NO_MORE_ITEMS)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn EvtRender(
    _context: usize,
    _fragment: usize,
    _flags: u32,
    _size: u32,
    _buffer: usize,
    _used: usize,
    _count: usize,
) -> i32 {
    fail(ERROR_INVALID_HANDLE)
}
#[unsafe(no_mangle)]
pub extern "system" fn EvtCreateRenderContext(_count: u32, _paths: usize, _flags: u32) -> usize {
    fail(ERROR_NOT_SUPPORTED)
}
#[unsafe(no_mangle)]
pub extern "system" fn EvtClose(_object: usize) -> i32 {
    fail(ERROR_INVALID_HANDLE)
}
