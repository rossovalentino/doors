//! urlmon exports.

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

/// Clears the caller's out-pointer, if any.
fn clear(out: usize) {
    if out != 0 {
        // SAFETY: the caller's pointer-sized out-parameter.
        unsafe { (out as *mut usize).write_unaligned(0) };
    }
}
const E_NOTIMPL: u32 = 0x8000_4001;
#[unsafe(no_mangle)]
pub extern "system" fn CoInternetCreateSecurityManager(
    _provider: usize,
    out: usize,
    _reserved: u32,
) -> u32 {
    clear(out);
    E_NOTIMPL
}
#[unsafe(no_mangle)]
pub extern "system" fn CoInternetCreateZoneManager(
    _provider: usize,
    out: usize,
    _reserved: u32,
) -> u32 {
    clear(out);
    E_NOTIMPL
}
