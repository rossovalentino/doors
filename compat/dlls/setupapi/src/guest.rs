//! setupapi exports.

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
use core::sync::atomic::{AtomicUsize, Ordering};
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_NO_MORE_ITEMS: u32 = 259;
const ERROR_NOT_FOUND: u32 = 1168;
const ERROR_NO_SUCH_DEVINST: u32 = 0xE000_020B;
const ERROR_NO_SUCH_DEVICE_INTERFACE: u32 = 0xE000_0216;
const INVALID_HANDLE_VALUE: usize = usize::MAX;
/// Device information sets: distinct non-null handles, each empty.
static NEXT: AtomicUsize = AtomicUsize::new(0x5E70_0000);
fn new_set() -> usize {
    NEXT.fetch_add(4, Ordering::Relaxed)
}
fn is_set(set: usize) -> bool {
    set != 0 && set != INVALID_HANDLE_VALUE
}
fn empty(set: usize, code: u32) -> i32 {
    fail(if is_set(set) {
        code
    } else {
        ERROR_INVALID_HANDLE
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn SetupDiGetClassDevsW(
    _class: usize,
    _enumerator: usize,
    _parent: usize,
    _flags: u32,
) -> usize {
    new_set()
}
#[unsafe(no_mangle)]
pub extern "system" fn SetupDiGetClassDevsExW(
    _class: usize,
    _enumerator: usize,
    _parent: usize,
    _flags: u32,
    existing: usize,
    _machine: usize,
    _reserved: usize,
) -> usize {
    if is_set(existing) {
        existing
    } else {
        new_set()
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetupDiCreateDeviceInfoList(_class: usize, _parent: usize) -> usize {
    new_set()
}
#[unsafe(no_mangle)]
pub extern "system" fn SetupDiDestroyDeviceInfoList(set: usize) -> i32 {
    if is_set(set) {
        1
    } else {
        fail(ERROR_INVALID_HANDLE)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetupDiEnumDeviceInfo(set: usize, _index: u32, _data: usize) -> i32 {
    empty(set, ERROR_NO_MORE_ITEMS)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetupDiEnumDeviceInterfaces(
    set: usize,
    _device: usize,
    _class: usize,
    _index: u32,
    _data: usize,
) -> i32 {
    empty(set, ERROR_NO_MORE_ITEMS)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetupDiGetDeviceInterfaceDetailW(
    set: usize,
    data: usize,
    _detail: usize,
    _size: u32,
    _required: usize,
    _device: usize,
) -> i32 {
    if data == 0 {
        return empty(set, ERROR_INVALID_PARAMETER);
    }
    empty(set, ERROR_NO_SUCH_DEVICE_INTERFACE)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn SetupDiGetDevicePropertyW(
    set: usize,
    _device: usize,
    _key: usize,
    _kind: usize,
    _buffer: usize,
    _size: u32,
    _required: usize,
    _flags: u32,
) -> i32 {
    empty(set, ERROR_NOT_FOUND)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn SetupDiGetDeviceRegistryPropertyW(
    set: usize,
    _device: usize,
    _property: u32,
    _kind: usize,
    _buffer: usize,
    _size: u32,
    _required: usize,
) -> i32 {
    empty(set, ERROR_NO_SUCH_DEVINST)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetupDiOpenDevRegKey(
    _set: usize,
    _device: usize,
    _scope: u32,
    _profile: u32,
    _kind: u32,
    _access: u32,
) -> usize {
    let _: i32 = fail(ERROR_NO_SUCH_DEVINST);
    INVALID_HANDLE_VALUE
}
#[unsafe(no_mangle)]
pub extern "system" fn SetupDiOpenDeviceInfoW(
    set: usize,
    _id: usize,
    _parent: usize,
    _flags: u32,
    _data: usize,
) -> i32 {
    empty(set, ERROR_NO_SUCH_DEVINST)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetupDiOpenDeviceInterfaceW(
    set: usize,
    _path: usize,
    _flags: u32,
    _data: usize,
) -> i32 {
    empty(set, ERROR_NO_SUCH_DEVICE_INTERFACE)
}
