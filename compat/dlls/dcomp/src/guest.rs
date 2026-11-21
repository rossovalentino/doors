//! dcomp.dll exports.
use crate::DXGI_ERROR_UNSUPPORTED;

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

fn clear(out: usize) {
    if out != 0 {
        // SAFETY: the caller's interface out-pointer.
        unsafe { (out as *mut usize).write_unaligned(0) };
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DCompositionCreateDevice(_dxgi: usize, _iid: usize, out: usize) -> u32 {
    clear(out);
    DXGI_ERROR_UNSUPPORTED
}
#[unsafe(no_mangle)]
pub extern "system" fn DCompositionCreateDevice2(
    _rendering: usize,
    _iid: usize,
    out: usize,
) -> u32 {
    clear(out);
    DXGI_ERROR_UNSUPPORTED
}
#[unsafe(no_mangle)]
pub extern "system" fn DCompositionCreateDevice3(
    _rendering: usize,
    _iid: usize,
    out: usize,
) -> u32 {
    clear(out);
    DXGI_ERROR_UNSUPPORTED
}
#[unsafe(no_mangle)]
pub extern "system" fn DCompositionCreateSurfaceHandle(
    _access: u32,
    _security: usize,
    out: usize,
) -> u32 {
    clear(out);
    DXGI_ERROR_UNSUPPORTED
}
