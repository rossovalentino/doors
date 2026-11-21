//! Token and process calls Chromium's sandbox and its helpers import
//! (P18). Doors has no restricted tokens, integrity levels, anonymous
//! token or named-pipe impersonation: those calls fail with
//! ERROR_NOT_SUPPORTED (or ERROR_CANNOT_IMPERSONATE), so a caller never
//! believes a restriction is in force. A token's owner, primary group and
//! default DACL may be set (they only shape new objects' defaults, which
//! Doors' storage computes itself). CreateProcessAsUser runs the program
//! as the session's user (every Doors token is that user's).
use super::*;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorLoadLibraryW(name: u64) -> u64;
    fn DoorGetProcAddress(module: u64, name: u64) -> u64;
}
const CANNOT_IMPERSONATE: u32 = 1368;

#[unsafe(no_mangle)]
pub extern "system" fn SetTokenInformation(
    token: usize,
    class: u32,
    _info: usize,
    _length: u32,
) -> i32 {
    if let Err(e) = object_of(door_windows_abi::widen(token)) {
        return last_error(e);
    }
    // TokenOwner 4, TokenPrimaryGroup 5, TokenDefaultDacl 6.
    last_error(if matches!(class, 4..=6) {
        error::SUCCESS
    } else {
        error::NOT_SUPPORTED
    })
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn CreateRestrictedToken(
    existing: usize,
    _flags: u32,
    _disable_count: u32,
    _disable: usize,
    _delete_count: u32,
    _delete: usize,
    _restrict_count: u32,
    _restrict: usize,
    out: usize,
) -> i32 {
    let _ = write_pointer(door_windows_abi::widen(out), 0);
    match object_of(door_windows_abi::widen(existing)) {
        Err(e) => last_error(e),
        Ok(_) => last_error(error::NOT_SUPPORTED),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ImpersonateAnonymousToken(_thread: usize) -> i32 {
    last_error(error::NOT_SUPPORTED)
}
#[unsafe(no_mangle)]
pub extern "system" fn ImpersonateNamedPipeClient(_pipe: usize) -> i32 {
    last_error(CANNOT_IMPERSONATE)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn AddConditionalAce(
    _acl: usize,
    _revision: u32,
    _flags: u32,
    _mask: u32,
    _kind: u8,
    _sid: usize,
    _condition: usize,
    _length: usize,
) -> i32 {
    last_error(error::NOT_SUPPORTED)
}
/// The registry has no per-process predefined-key cache to disable.
#[unsafe(no_mangle)]
pub extern "system" fn RegDisablePredefinedCache() -> u32 {
    error::SUCCESS
}
type CreateProcessW =
    extern "system" fn(usize, usize, usize, usize, i32, u32, usize, usize, usize, usize) -> i32;
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn CreateProcessAsUserW(
    token: usize,
    application: usize,
    command_line: usize,
    process_attributes: usize,
    thread_attributes: usize,
    inherit: i32,
    flags: u32,
    environment: usize,
    directory: usize,
    startup: usize,
    information: usize,
) -> i32 {
    if token != 0
        && let Err(e) = object_of(door_windows_abi::widen(token))
    {
        return last_error(e);
    }
    let module: Vec<u16> = "kernel32.dll\0".encode_utf16().collect();
    let name = b"CreateProcessW\0";
    let f = unsafe {
        DoorGetProcAddress(
            DoorLoadLibraryW(module.as_ptr() as u64),
            name.as_ptr() as u64,
        )
    };
    if f == 0 {
        return last_error(error::CALL_NOT_IMPLEMENTED);
    }
    // SAFETY: kernel32's CreateProcessW.
    let create: CreateProcessW = unsafe { core::mem::transmute(door_windows_abi::narrow(f)) };
    create(
        application,
        command_line,
        process_attributes,
        thread_attributes,
        inherit,
        flags,
        environment,
        directory,
        startup,
        information,
    )
}
