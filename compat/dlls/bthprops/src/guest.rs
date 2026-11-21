//! bthprops exports.

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

/// Clears the caller's out-pointer, if any.
fn clear(out: usize) {
    if out != 0 {
        // SAFETY: the caller's pointer-sized out-parameter.
        unsafe { (out as *mut usize).write_unaligned(0) };
    }
}
#[link(name = "kernel32", kind = "dylib")]
unsafe extern "system" {
    fn SetLastError(code: u32);
}
fn fail<T: Default>(code: u32) -> T {
    unsafe { SetLastError(code) };
    T::default()
}
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_NO_MORE_ITEMS: u32 = 259;
const ERROR_NOT_FOUND: u32 = 1168;
#[unsafe(no_mangle)]
pub extern "system" fn BluetoothFindFirstRadio(_parameters: usize, radio: usize) -> usize {
    clear(radio);
    fail(ERROR_NO_MORE_ITEMS)
}
#[unsafe(no_mangle)]
pub extern "system" fn BluetoothFindNextRadio(_find: usize, radio: usize) -> i32 {
    clear(radio);
    fail(ERROR_NO_MORE_ITEMS)
}
#[unsafe(no_mangle)]
pub extern "system" fn BluetoothFindRadioClose(_find: usize) -> i32 {
    fail(ERROR_INVALID_HANDLE)
}
#[unsafe(no_mangle)]
pub extern "system" fn BluetoothFindFirstDevice(_parameters: usize, _info: usize) -> usize {
    fail(ERROR_NO_MORE_ITEMS)
}
#[unsafe(no_mangle)]
pub extern "system" fn BluetoothFindNextDevice(_find: usize, _info: usize) -> i32 {
    fail(ERROR_NO_MORE_ITEMS)
}
#[unsafe(no_mangle)]
pub extern "system" fn BluetoothFindDeviceClose(_find: usize) -> i32 {
    fail(ERROR_INVALID_HANDLE)
}
#[unsafe(no_mangle)]
pub extern "system" fn BluetoothGetRadioInfo(_radio: usize, _info: usize) -> u32 {
    ERROR_INVALID_HANDLE
}
#[unsafe(no_mangle)]
pub extern "system" fn BluetoothIsConnectable(_radio: usize) -> i32 {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn BluetoothEnableDiscovery(_radio: usize, _enabled: i32) -> i32 {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn BluetoothEnableIncomingConnections(_radio: usize, _enabled: i32) -> i32 {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn BluetoothSdpGetAttributeValue(
    _record: usize,
    _size: u32,
    _id: u16,
    _data: usize,
) -> u32 {
    ERROR_NOT_FOUND
}
#[unsafe(no_mangle)]
pub extern "system" fn BluetoothSdpGetContainerElementData(
    _stream: usize,
    _size: u32,
    _element: usize,
    _data: usize,
) -> u32 {
    ERROR_INVALID_HANDLE
}
