//! wldp.dll exports: no lockdown, files allowed, classes approved.
use crate::{E_INVALIDARG, E_NOT_FOUND, EXECUTION_ALLOWED, NO_LOCKDOWN, S_OK};
use door_windows_abi::widen;

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

fn put(pointer: u64, value: u32) {
    // SAFETY: the caller's DWORD/BOOL/enum out parameter.
    unsafe { (pointer as *mut u32).write_unaligned(value) };
}

/// `WldpGetLockdownPolicy(hostInformation, lockdownState, flags)`.
#[unsafe(no_mangle)]
pub extern "system" fn WldpGetLockdownPolicy(_host: usize, state: usize, _flags: u32) -> u32 {
    let state = widen(state);
    if state == 0 {
        return E_INVALIDARG;
    }
    put(state, NO_LOCKDOWN);
    S_OK
}
/// `WldpCanExecuteFile(host, options, fileHandle, auditInfo, result)`.
#[unsafe(no_mangle)]
pub extern "system" fn WldpCanExecuteFile(
    _host: usize,
    _options: u32,
    _file: usize,
    _audit: usize,
    result: usize,
) -> u32 {
    let result = widen(result);
    if result == 0 {
        return E_INVALIDARG;
    }
    put(result, EXECUTION_ALLOWED);
    S_OK
}
/// `WldpCanExecuteBuffer(host, options, buffer, size, auditInfo, result)`.
#[unsafe(no_mangle)]
pub extern "system" fn WldpCanExecuteBuffer(
    _host: usize,
    _options: u32,
    _buffer: usize,
    _size: u32,
    _audit: usize,
    result: usize,
) -> u32 {
    let result = widen(result);
    if result == 0 {
        return E_INVALIDARG;
    }
    put(result, EXECUTION_ALLOWED);
    S_OK
}
/// `WldpIsClassInApprovedList(classId, hostInformation, isApproved, flags)`.
#[unsafe(no_mangle)]
pub extern "system" fn WldpIsClassInApprovedList(
    _class: usize,
    _host: usize,
    approved: usize,
    _flags: u32,
) -> u32 {
    let approved = widen(approved);
    if approved == 0 {
        return E_INVALIDARG;
    }
    put(approved, 1);
    S_OK
}
/// `WldpGetApplicationSettingBoolean(id, setting, result)`: no settings.
#[unsafe(no_mangle)]
pub extern "system" fn WldpGetApplicationSettingBoolean(
    _id: usize,
    _setting: usize,
    result: usize,
) -> u32 {
    let result = widen(result);
    if result != 0 {
        put(result, 0);
    }
    E_NOT_FOUND
}
/// `WldpQueryDynamicCodeTrust(fileHandle, baseImage, imageSize)`: trusted.
#[unsafe(no_mangle)]
pub extern "system" fn WldpQueryDynamicCodeTrust(_file: usize, _base: usize, _size: u32) -> u32 {
    S_OK
}
