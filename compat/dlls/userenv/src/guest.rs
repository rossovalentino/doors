//! userenv exports.

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
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
/// HRESULT_FROM_WIN32(ERROR_NOT_SUPPORTED).
const NOT_SUPPORTED: u32 = 0x8007_0032;
#[link(name = "kernel32", kind = "dylib")]
unsafe extern "system" {
    fn GetEnvironmentVariableW(name: usize, buffer: usize, size: u32) -> u32;
    fn GetEnvironmentStringsW() -> usize;
    fn FreeEnvironmentStringsW(block: usize) -> i32;
    fn GetProcessHeap() -> usize;
    fn HeapAlloc(heap: usize, flags: u32, bytes: usize) -> usize;
    fn HeapFree(heap: usize, flags: u32, block: usize) -> i32;
}
/// GetUserProfileDirectoryW: USERPROFILE (every token is the session
/// user's); `size` in characters with the terminator.
#[unsafe(no_mangle)]
pub extern "system" fn GetUserProfileDirectoryW(_token: usize, out: usize, size: usize) -> i32 {
    if size == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let name: [u16; 12] = [85, 83, 69, 82, 80, 82, 79, 70, 73, 76, 69, 0]; // USERPROFILE
    let mut buffer = [0u16; 512];
    let n = unsafe {
        GetEnvironmentVariableW(name.as_ptr() as usize, buffer.as_mut_ptr() as usize, 512)
    } as usize;
    if n == 0 || n >= buffer.len() {
        return fail(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: the caller's DWORD in/out count and buffer.
    let capacity = unsafe { (size as *const u32).read_unaligned() } as usize;
    unsafe { (size as *mut u32).write_unaligned((n + 1) as u32) };
    if out == 0 || capacity < n + 1 {
        return fail(ERROR_INSUFFICIENT_BUFFER);
    }
    unsafe { core::ptr::copy_nonoverlapping(buffer.as_ptr(), out as *mut u16, n + 1) };
    1
}
/// CreateEnvironmentBlock: a copy of this process's environment (the
/// session user's), whether or not `inherit` is set.
#[unsafe(no_mangle)]
pub extern "system" fn CreateEnvironmentBlock(out: usize, _token: usize, _inherit: i32) -> i32 {
    if out == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let block = unsafe { GetEnvironmentStringsW() };
    if block == 0 {
        return fail(ERROR_NOT_ENOUGH_MEMORY);
    }
    // The block ends at the first empty string (two NULs in a row).
    let mut n = 0usize;
    // SAFETY: the environment block GetEnvironmentStrings returned.
    unsafe {
        while !((block as *const u16).add(n).read_unaligned() == 0
            && (n == 0 || (block as *const u16).add(n - 1).read_unaligned() == 0))
        {
            n += 1;
        }
    }
    let units = (n + 1).max(2);
    let copy = unsafe { HeapAlloc(GetProcessHeap(), 8, 2 * units) };
    if copy == 0 {
        unsafe { FreeEnvironmentStringsW(block) };
        return fail(ERROR_NOT_ENOUGH_MEMORY);
    }
    unsafe {
        core::ptr::copy_nonoverlapping(block as *const u16, copy as *mut u16, n + 1);
        FreeEnvironmentStringsW(block);
        (out as *mut usize).write_unaligned(copy);
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn DestroyEnvironmentBlock(block: usize) -> i32 {
    if block == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    unsafe { HeapFree(GetProcessHeap(), 0, block) }
}
#[unsafe(no_mangle)]
pub extern "system" fn DeriveAppContainerSidFromAppContainerName(_name: usize, sid: usize) -> u32 {
    clear(sid);
    NOT_SUPPORTED
}
