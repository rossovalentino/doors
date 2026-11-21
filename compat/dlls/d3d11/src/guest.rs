//! d3d11.dll exports.
use crate::DXGI_ERROR_UNSUPPORTED;

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

const E_INVALIDARG: u32 = 0x8007_0057;
fn clear(out: usize) {
    if out != 0 {
        // SAFETY: the caller's interface out-pointer.
        unsafe { (out as *mut usize).write_unaligned(0) };
    }
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn D3D11CreateDevice(
    _adapter: usize,
    _driver: u32,
    _software: usize,
    _flags: u32,
    _levels: usize,
    _count: u32,
    _sdk: u32,
    device: usize,
    level: usize,
    context: usize,
) -> u32 {
    clear(device);
    clear(context);
    if level != 0 {
        // SAFETY: the caller's D3D_FEATURE_LEVEL out-parameter.
        unsafe { (level as *mut u32).write_unaligned(0) };
    }
    DXGI_ERROR_UNSUPPORTED
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn D3D11CreateDeviceAndSwapChain(
    adapter: usize,
    driver: u32,
    software: usize,
    flags: u32,
    levels: usize,
    count: u32,
    sdk: u32,
    _description: usize,
    swap_chain: usize,
    device: usize,
    level: usize,
    context: usize,
) -> u32 {
    clear(swap_chain);
    D3D11CreateDevice(
        adapter, driver, software, flags, levels, count, sdk, device, level, context,
    )
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn D3D11On12CreateDevice(
    _device: usize,
    _flags: u32,
    _levels: usize,
    _level_count: u32,
    _queues: usize,
    _queue_count: u32,
    _node: u32,
    out: usize,
    context: usize,
    level: usize,
) -> u32 {
    clear(out);
    clear(context);
    if level != 0 {
        // SAFETY: the caller's D3D_FEATURE_LEVEL out-parameter.
        unsafe { (level as *mut u32).write_unaligned(0) };
    }
    E_INVALIDARG
}
