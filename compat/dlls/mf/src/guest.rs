//! mf exports.

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
/// MFEnumDeviceSources: no capture devices.
#[unsafe(no_mangle)]
pub extern "system" fn MFEnumDeviceSources(
    _attributes: usize,
    sources: usize,
    count: usize,
) -> u32 {
    clear(sources);
    if count != 0 {
        // SAFETY: the caller's UINT32 out-parameter.
        unsafe { (count as *mut u32).write_unaligned(0) };
    }
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn MFCreateDeviceSource(_attributes: usize, out: usize) -> u32 {
    clear(out);
    E_NOTIMPL
}
