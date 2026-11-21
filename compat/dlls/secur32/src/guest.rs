//! secur32.dll exports: GetUserNameExW/A over advapi32's GetUserNameW and
//! kernel32's GetComputerNameW.
use door_windows_abi::widen;

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
}

/// The SAM-compatible name (or the format's error) into `out`.
fn lookup(format: u32, out: &mut [u16]) -> Result<usize, (u32, usize)> {
    let mut user = [0u16; 257];
    let mut user_len = user.len() as u32;
    let mut computer = [0u16; 64];
    let mut computer_len = computer.len() as u32;
    let ok = unsafe {
        GetUserNameW(
            user.as_mut_ptr() as usize,
            &mut user_len as *mut u32 as usize,
        ) != 0
            && GetComputerNameW(
                computer.as_mut_ptr() as usize,
                &mut computer_len as *mut u32 as usize,
            ) != 0
    };
    if !ok {
        return Err((crate::ERROR_NONE_MAPPED, 0));
    }
    // GetUserNameW counts the terminator; GetComputerNameW does not.
    let user = &user[..(user_len as usize).saturating_sub(1)];
    let computer = &computer[..computer_len as usize];
    crate::name(format, computer, user, out)
}
fn read_size(size: u64) -> Option<u32> {
    if size == 0 {
        return None;
    }
    // SAFETY: the caller's ULONG in/out parameter.
    Some(unsafe { (size as *const u32).read_unaligned() })
}
fn write_size(size: u64, value: u32) {
    // SAFETY: the caller's ULONG in/out parameter.
    unsafe { (size as *mut u32).write_unaligned(value) };
}
/// `GetUserNameExW`: BOOLEAN; `size` in characters (on ERROR_MORE_DATA the
/// characters needed, with the terminator).
#[unsafe(no_mangle)]
pub extern "system" fn GetUserNameExW(format: u32, buffer: usize, size: usize) -> u8 {
    let (buffer, size) = (widen(buffer), widen(size));
    let Some(capacity) = read_size(size) else {
        unsafe { SetLastError(crate::ERROR_INVALID_PARAMETER) };
        return 0;
    };
    let mut out = [0u16; 400];
    let room = (capacity as usize).min(out.len());
    match lookup(format, &mut out[..if buffer == 0 { 0 } else { room }]) {
        Ok(n) => {
            for (i, &u) in out[..=n].iter().enumerate() {
                // SAFETY: the caller's buffer of `capacity` characters.
                unsafe { (buffer as *mut u16).add(i).write_unaligned(u) };
            }
            write_size(size, n as u32);
            1
        }
        Err((code, needed)) => {
            if code == crate::ERROR_MORE_DATA {
                write_size(size, needed as u32);
            }
            unsafe { SetLastError(code) };
            0
        }
    }
}
/// `GetUserNameExA`: the same name in the ANSI code page (ASCII names).
#[unsafe(no_mangle)]
pub extern "system" fn GetUserNameExA(format: u32, buffer: usize, size: usize) -> u8 {
    let (buffer, size) = (widen(buffer), widen(size));
    let Some(capacity) = read_size(size) else {
        unsafe { SetLastError(crate::ERROR_INVALID_PARAMETER) };
        return 0;
    };
    let mut out = [0u16; 400];
    let room = (capacity as usize).min(out.len());
    match lookup(format, &mut out[..if buffer == 0 { 0 } else { room }]) {
        Ok(n) => {
            for (i, &u) in out[..=n].iter().enumerate() {
                let byte = u8::try_from(u).unwrap_or(b'?');
                // SAFETY: the caller's buffer of `capacity` bytes.
                unsafe { (buffer as *mut u8).add(i).write(byte) };
            }
            write_size(size, n as u32);
            1
        }
        Err((code, needed)) => {
            if code == crate::ERROR_MORE_DATA {
                write_size(size, needed as u32);
            }
            unsafe { SetLastError(code) };
            0
        }
    }
}

// ---- SSPI ---------------------------------------------------------------------------------
// Doors has no security support providers (no NTLM, Kerberos, Negotiate
// or Schannel): every package lookup fails with SEC_E_SECPKG_NOT_FOUND,
// so no credentials or contexts exist and every handle is invalid, as on
// a Windows without the provider. Chromium then offers only the HTTP
// authentication schemes it implements itself.
const SEC_E_OK: u32 = 0;
const SEC_E_INVALID_HANDLE: u32 = 0x8009_0301;
const SEC_E_SECPKG_NOT_FOUND: u32 = 0x8009_0305;
#[link(name = "kernel32", kind = "dylib")]
unsafe extern "system" {
    fn GetProcessHeap() -> usize;
    fn HeapFree(heap: usize, flags: u32, block: usize) -> i32;
}
fn clear_pointer(at: usize) {
    if at != 0 {
        // SAFETY: the caller's out-pointer.
        unsafe { (at as *mut usize).write_unaligned(0) };
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn QuerySecurityPackageInfoW(_package: usize, info: usize) -> u32 {
    clear_pointer(info);
    SEC_E_SECPKG_NOT_FOUND
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn AcquireCredentialsHandleW(
    _principal: usize,
    _package: usize,
    _use: u32,
    _logon: usize,
    _auth: usize,
    _get_key: usize,
    _argument: usize,
    _credential: usize,
    _expiry: usize,
) -> u32 {
    SEC_E_SECPKG_NOT_FOUND
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn InitializeSecurityContextW(
    _credential: usize,
    _context: usize,
    _target: usize,
    _requirements: u32,
    _reserved1: u32,
    _representation: u32,
    _input: usize,
    _reserved2: u32,
    _new_context: usize,
    _output: usize,
    _attributes: usize,
    _expiry: usize,
) -> u32 {
    SEC_E_INVALID_HANDLE
}
#[unsafe(no_mangle)]
pub extern "system" fn DeleteSecurityContext(_context: usize) -> u32 {
    SEC_E_INVALID_HANDLE
}
#[unsafe(no_mangle)]
pub extern "system" fn FreeCredentialsHandle(_credential: usize) -> u32 {
    SEC_E_INVALID_HANDLE
}
#[unsafe(no_mangle)]
pub extern "system" fn QueryContextAttributesW(_context: usize, _attribute: u32, _buffer: usize) -> u32 {
    SEC_E_INVALID_HANDLE
}
/// Buffers SSPI returned (none here) are process-heap blocks.
#[unsafe(no_mangle)]
pub extern "system" fn FreeContextBuffer(buffer: usize) -> u32 {
    if buffer != 0 {
        unsafe { HeapFree(GetProcessHeap(), 0, buffer) };
    }
    SEC_E_OK
}
