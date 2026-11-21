//! dhcpcsvc exports.

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

const ERROR_NOT_SUPPORTED: u32 = 50;
/// DhcpCApiInitialize: version 2 of the DHCP client API.
#[unsafe(no_mangle)]
pub extern "system" fn DhcpCApiInitialize(version: usize) -> u32 {
    if version != 0 {
        // SAFETY: the caller's DWORD out-parameter.
        unsafe { (version as *mut u32).write_unaligned(2) };
    }
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn DhcpCApiCleanup() {}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn DhcpRequestParams(
    _flags: u32,
    _reserved: usize,
    _adapter: usize,
    _class: usize,
    _send: usize,
    _receive: usize,
    _buffer: usize,
    _size: usize,
    _request: usize,
) -> u32 {
    ERROR_NOT_SUPPORTED
}
