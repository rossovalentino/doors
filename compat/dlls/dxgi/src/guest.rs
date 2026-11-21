//! dxgi.dll exports.
use crate::DXGI_ERROR_UNSUPPORTED;

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

const E_INVALIDARG: u32 = 0x8007_0057;
const E_NOINTERFACE: u32 = 0x8000_4002;
fn clear(out: usize) {
    if out != 0 {
        // SAFETY: the caller's interface out-pointer.
        unsafe { (out as *mut usize).write_unaligned(0) };
    }
}
fn factory(out: usize) -> u32 {
    if out == 0 {
        return E_INVALIDARG;
    }
    clear(out);
    DXGI_ERROR_UNSUPPORTED
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateDXGIFactory(_iid: usize, out: usize) -> u32 {
    factory(out)
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateDXGIFactory1(_iid: usize, out: usize) -> u32 {
    factory(out)
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateDXGIFactory2(_flags: u32, _iid: usize, out: usize) -> u32 {
    factory(out)
}
#[unsafe(no_mangle)]
pub extern "system" fn DXGIGetDebugInterface1(_flags: u32, _iid: usize, out: usize) -> u32 {
    clear(out);
    E_NOINTERFACE
}
#[unsafe(no_mangle)]
pub extern "system" fn DXGIDeclareAdapterRemovalSupport() -> u32 {
    0
}
