//! Software Restriction Policies (SAFER): Doors configures none, so every
//! program and script is fully trusted (PowerShell asks before choosing its
//! language mode).
use super::*;
/// The one level Doors has: no software restriction policy is configured,
/// so every program and script identifies as SAFER_LEVELID_FULLYTRUSTED
/// (Unrestricted), and computing a token from that level changes nothing
/// (SAFER_TOKEN_NULL_IF_EQUAL gives no token). PowerShell asks this to
/// learn whether AppLocker or SRP lock its language down.
const SAFER_LEVEL_HANDLE: u64 = 0x5afe_0001;
const SAFER_LEVELID_FULLYTRUSTED: u32 = 0x40000;
#[unsafe(no_mangle)]
pub extern "system" fn SaferIdentifyLevel(
    _count: u32,
    _properties: usize,
    level: usize,
    _reserved: usize,
) -> i32 {
    let level = door_windows_abi::widen(level);
    if level == 0 {
        return last_error(error::INVALID_PARAMETER);
    }
    unsafe { (level as *mut usize).write_unaligned(door_windows_abi::narrow(SAFER_LEVEL_HANDLE)) };
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SaferCreateLevel(
    _scope: u32,
    level_id: u32,
    _open_flags: u32,
    level: usize,
    _reserved: usize,
) -> i32 {
    let level = door_windows_abi::widen(level);
    if level == 0 || level_id != SAFER_LEVELID_FULLYTRUSTED {
        return last_error(error::INVALID_PARAMETER);
    }
    unsafe { (level as *mut usize).write_unaligned(door_windows_abi::narrow(SAFER_LEVEL_HANDLE)) };
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SaferCloseLevel(level: usize) -> i32 {
    if door_windows_abi::widen(level) != SAFER_LEVEL_HANDLE {
        return last_error(error::INVALID_HANDLE);
    }
    1
}
/// `SaferComputeTokenFromLevel`: the fully trusted level restricts nothing,
/// so the new token is the caller's own; with SAFER_TOKEN_NULL_IF_EQUAL (1)
/// none is returned.
#[unsafe(no_mangle)]
pub extern "system" fn SaferComputeTokenFromLevel(
    level: usize,
    _input: usize,
    output: usize,
    flags: u32,
    _reserved: usize,
) -> i32 {
    let output = door_windows_abi::widen(output);
    if door_windows_abi::widen(level) != SAFER_LEVEL_HANDLE || output == 0 {
        return last_error(error::INVALID_PARAMETER);
    }
    if flags & 1 == 0 {
        return last_error(error::NOT_SUPPORTED);
    }
    unsafe { (output as *mut usize).write_unaligned(0) };
    1
}
/// SaferGetLevelInformation(SaferObjectLevelId = 1): the level's ID.
#[unsafe(no_mangle)]
pub extern "system" fn SaferGetLevelInformation(
    level: usize,
    class: u32,
    buffer: usize,
    size: u32,
    returned: usize,
) -> i32 {
    let buffer = door_windows_abi::widen(buffer);
    let returned = door_windows_abi::widen(returned);
    if door_windows_abi::widen(level) != SAFER_LEVEL_HANDLE || class != 1 {
        return last_error(error::INVALID_PARAMETER);
    }
    if returned != 0 {
        unsafe { (returned as *mut u32).write_unaligned(4) };
    }
    if buffer == 0 || size < 4 {
        return last_error(122);
    }
    unsafe { (buffer as *mut u32).write_unaligned(SAFER_LEVELID_FULLYTRUSTED) };
    1
}
