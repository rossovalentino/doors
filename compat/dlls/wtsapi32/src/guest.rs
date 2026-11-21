//! wtsapi32.dll exports.
use crate::*;
use alloc::vec::Vec;

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "advapi32", kind = "dylib")]
unsafe extern "system" {
    fn GetUserNameW(out: usize, length: usize) -> i32;
}
#[link(name = "kernel32", kind = "dylib")]
unsafe extern "system" {
    fn GetComputerNameW(buffer: usize, size: usize) -> i32;
    fn SetLastError(code: u32);
    fn GetProcessHeap() -> usize;
    fn HeapAlloc(heap: usize, flags: u32, bytes: usize) -> usize;
    fn HeapFree(heap: usize, flags: u32, block: usize) -> i32;
    fn GetCurrentProcessId() -> u32;
    fn ProcessIdToSessionId(process: u32, session: usize) -> i32;
}
#[link(name = "user32", kind = "dylib")]
unsafe extern "system" {
    fn IsWindow(hwnd: usize) -> i32;
}

fn fail(code: u32) -> i32 {
    unsafe { SetLastError(code) };
    0
}
/// WTSRegisterSessionNotification: the window would hear
/// WM_WTSSESSION_CHANGE; Doors' one console session never changes, so
/// nothing is sent.
#[unsafe(no_mangle)]
pub extern "system" fn WTSRegisterSessionNotification(window: usize, flags: u32) -> i32 {
    if flags > 1 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    if unsafe { IsWindow(window) } == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn WTSRegisterSessionNotificationEx(
    _server: usize,
    window: usize,
    flags: u32,
) -> i32 {
    WTSRegisterSessionNotification(window, flags)
}
#[unsafe(no_mangle)]
pub extern "system" fn WTSUnRegisterSessionNotification(window: usize) -> i32 {
    if unsafe { IsWindow(window) } == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn WTSUnRegisterSessionNotificationEx(_server: usize, window: usize) -> i32 {
    WTSUnRegisterSessionNotification(window)
}
fn user_name() -> Vec<u16> {
    let mut user = [0u16; 257];
    let mut length = user.len() as u32;
    if unsafe { GetUserNameW(user.as_mut_ptr() as usize, &mut length as *mut u32 as usize) } == 0 {
        return Vec::new();
    }
    user[..(length as usize).saturating_sub(1)].to_vec()
}
fn computer_name() -> Vec<u16> {
    let mut name = [0u16; 64];
    let mut length = name.len() as u32;
    if unsafe { GetComputerNameW(name.as_mut_ptr() as usize, &mut length as *mut u32 as usize) }
        == 0
    {
        return Vec::new();
    }
    name[..length as usize].to_vec()
}
/// WTSQuerySessionInformationW for the current server and session
/// (this process's console session): a WTSFreeMemory block.
#[unsafe(no_mangle)]
pub extern "system" fn WTSQuerySessionInformationW(
    server: usize,
    session: u32,
    class: u32,
    buffer: usize,
    bytes: usize,
) -> i32 {
    if server != 0 || buffer == 0 || bytes == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let mut own = 0u32;
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut own as *mut u32 as usize) };
    if session != own && session != CURRENT_SESSION {
        return fail(ERROR_FILE_NOT_FOUND);
    }
    let Some(data) = session_information(class, own, &user_name(), &computer_name()) else {
        return fail(ERROR_NOT_SUPPORTED);
    };
    let heap = unsafe { GetProcessHeap() };
    let block = unsafe { HeapAlloc(heap, 0, data.len().max(1)) };
    if block == 0 {
        return fail(ERROR_NOT_ENOUGH_MEMORY);
    }
    // SAFETY: a fresh block of data.len() bytes and the caller's outputs.
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), block as *mut u8, data.len());
        (buffer as *mut usize).write_unaligned(block);
        (bytes as *mut u32).write_unaligned(data.len() as u32);
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn WTSFreeMemory(block: usize) {
    if block != 0 {
        unsafe { HeapFree(GetProcessHeap(), 0, block) };
    }
}
