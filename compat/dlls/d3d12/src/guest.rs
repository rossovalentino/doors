//! d3d12.dll exports.
use crate::DXGI_ERROR_UNSUPPORTED;

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

const E_NOINTERFACE: u32 = 0x8000_4002;
fn clear(out: usize) {
    if out != 0 {
        // SAFETY: the caller's interface out-pointer.
        unsafe { (out as *mut usize).write_unaligned(0) };
    }
}
const E_NOTIMPL: u32 = 0x8000_4001;
#[unsafe(no_mangle)]
pub extern "system" fn D3D12CreateDevice(
    _adapter: usize,
    _level: u32,
    _iid: usize,
    out: usize,
) -> u32 {
    clear(out);
    DXGI_ERROR_UNSUPPORTED
}
#[unsafe(no_mangle)]
pub extern "system" fn D3D12GetDebugInterface(_iid: usize, out: usize) -> u32 {
    clear(out);
    E_NOINTERFACE
}
#[unsafe(no_mangle)]
pub extern "system" fn D3D12EnableExperimentalFeatures(
    _count: u32,
    _features: usize,
    _data: usize,
    _sizes: usize,
) -> u32 {
    E_NOINTERFACE
}
#[unsafe(no_mangle)]
pub extern "system" fn D3D12GetInterface(_clsid: usize, _iid: usize, out: usize) -> u32 {
    clear(out);
    E_NOINTERFACE
}
#[unsafe(no_mangle)]
pub extern "system" fn D3D12SerializeRootSignature(
    _description: usize,
    _version: u32,
    blob: usize,
    error: usize,
) -> u32 {
    clear(blob);
    clear(error);
    E_NOTIMPL
}
#[unsafe(no_mangle)]
pub extern "system" fn D3D12SerializeVersionedRootSignature(
    _description: usize,
    blob: usize,
    error: usize,
) -> u32 {
    clear(blob);
    clear(error);
    E_NOTIMPL
}
