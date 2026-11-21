//! Namespace changes: CreateDirectory, RemoveDirectory, DeleteFile,
//! MoveFile(Ex) and CopyFile(Ex), over ntdll's storage connection. The ANSI
//! forms convert their paths through the ANSI code page.
use crate::environment::{UNITS, fail, widen};
use door_windows_abi::error;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorCreateDirectoryW(path: u64) -> i32;
    fn DoorRemoveDirectoryW(path: u64) -> i32;
    fn DoorDeleteFileW(path: u64) -> i32;
    fn DoorMoveFileExW(existing: u64, new: u64, flags: u32) -> i32;
    fn DoorCopyFileW(existing: u64, new: u64, fail_if_exists: i32) -> i32;
}
/// One ANSI path argument widened, or ERROR_INVALID_PARAMETER.
fn ansi(path: u64, f: impl FnOnce(u64) -> i32) -> i32 {
    let mut units = [0u16; UNITS];
    match widen(path, &mut units) {
        Ok(p) if p != 0 => f(p),
        _ => fail(error::INVALID_PARAMETER) as i32,
    }
}
fn ansi2(a: u64, b: u64, f: impl FnOnce(u64, u64) -> i32) -> i32 {
    ansi(a, |a| ansi(b, |b| f(a, b)))
}
/// Security attributes are not applied: the directory inherits its
/// parent's access, as with a null descriptor.
#[unsafe(no_mangle)]
pub extern "system" fn CreateDirectoryW(path: usize, _security: usize) -> i32 {
    let path = door_windows_abi::widen(path);
    unsafe { DoorCreateDirectoryW(path) }
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateDirectoryA(path: usize, _security: usize) -> i32 {
    let path = door_windows_abi::widen(path);
    ansi(path, |p| unsafe { DoorCreateDirectoryW(p) })
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateDirectoryExW(_template: usize, path: usize, _security: usize) -> i32 {
    let path = door_windows_abi::widen(path);
    unsafe { DoorCreateDirectoryW(path) }
}
#[unsafe(no_mangle)]
pub extern "system" fn RemoveDirectoryW(path: usize) -> i32 {
    let path = door_windows_abi::widen(path);
    unsafe { DoorRemoveDirectoryW(path) }
}
#[unsafe(no_mangle)]
pub extern "system" fn RemoveDirectoryA(path: usize) -> i32 {
    let path = door_windows_abi::widen(path);
    ansi(path, |p| unsafe { DoorRemoveDirectoryW(p) })
}
#[unsafe(no_mangle)]
pub extern "system" fn DeleteFileW(path: usize) -> i32 {
    let path = door_windows_abi::widen(path);
    unsafe { DoorDeleteFileW(path) }
}
#[unsafe(no_mangle)]
pub extern "system" fn DeleteFileA(path: usize) -> i32 {
    let path = door_windows_abi::widen(path);
    ansi(path, |p| unsafe { DoorDeleteFileW(p) })
}
#[unsafe(no_mangle)]
pub extern "system" fn MoveFileExW(existing: usize, new: usize, flags: u32) -> i32 {
    let existing = door_windows_abi::widen(existing);
    let new = door_windows_abi::widen(new);
    {
        if new == 0 {
            // A null target deletes at reboot (MOVEFILE_DELAY_UNTIL_REBOOT):
            // only administrators may schedule it.
            return fail(error::ACCESS_DENIED) as i32;
        }
        unsafe { DoorMoveFileExW(existing, new, flags) }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MoveFileW(existing: usize, new: usize) -> i32 {
    let existing = door_windows_abi::widen(existing);
    let new = door_windows_abi::widen(new);
    {
        MoveFileExW(
            door_windows_abi::narrow(existing),
            door_windows_abi::narrow(new),
            2,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MoveFileExA(existing: usize, new: usize, flags: u32) -> i32 {
    let existing = door_windows_abi::widen(existing);
    let new = door_windows_abi::widen(new);
    {
        ansi2(existing, new, |a, b| {
            MoveFileExW(
                door_windows_abi::narrow(a),
                door_windows_abi::narrow(b),
                flags,
            )
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MoveFileA(existing: usize, new: usize) -> i32 {
    let existing = door_windows_abi::widen(existing);
    let new = door_windows_abi::widen(new);
    {
        ansi2(existing, new, |a, b| {
            MoveFileExW(door_windows_abi::narrow(a), door_windows_abi::narrow(b), 2)
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CopyFileW(existing: usize, new: usize, fail_if_exists: i32) -> i32 {
    let existing = door_windows_abi::widen(existing);
    let new = door_windows_abi::widen(new);
    unsafe { DoorCopyFileW(existing, new, fail_if_exists) }
}
#[unsafe(no_mangle)]
pub extern "system" fn CopyFileA(existing: usize, new: usize, fail_if_exists: i32) -> i32 {
    let existing = door_windows_abi::widen(existing);
    let new = door_windows_abi::widen(new);
    {
        ansi2(existing, new, |a, b| unsafe {
            DoorCopyFileW(a, b, fail_if_exists)
        })
    }
}
/// COPY_FILE_FAIL_IF_EXISTS (1) is honoured; the progress routine is not
/// called (the copy is one step).
#[unsafe(no_mangle)]
pub extern "system" fn CopyFileExW(
    existing: usize,
    new: usize,
    _progress: usize,
    _data: usize,
    cancel: usize,
    flags: u32,
) -> i32 {
    let existing = door_windows_abi::widen(existing);
    let new = door_windows_abi::widen(new);
    let cancel = door_windows_abi::widen(cancel);
    {
        let _ = cancel;
        unsafe { DoorCopyFileW(existing, new, i32::from(flags & 1 != 0)) }
    }
}

/// GetFullPathNameW: the path made absolute against the current directory
/// (`door_windows_abi::path`). The units written without the terminator,
/// or the size needed with it when `buffer` is too small.
#[unsafe(no_mangle)]
pub extern "system" fn GetFullPathNameW(
    path: usize,
    capacity: u32,
    buffer: usize,
    file_part: usize,
) -> u32 {
    use door_windows_abi::path;
    let Some(input) = wide_string(path) else {
        return fail(error::INVALID_PARAMETER);
    };
    let mut current = [0u16; 1024];
    let n = crate::application::GetCurrentDirectoryW(
        current.len() as u32,
        current.as_mut_ptr() as usize,
    );
    if n == 0 || n as usize >= current.len() {
        return 0;
    }
    let mut out = [0u16; 4096];
    let Some(length) = path::full_path(input, &current[..n as usize], &mut out) else {
        return fail(error::INVALID_PARAMETER);
    };
    if (capacity as usize) <= length || buffer == 0 {
        return length as u32 + 1;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(out.as_ptr(), buffer as *mut u16, length);
        ((buffer as *mut u16).add(length)).write_unaligned(0);
    }
    if file_part != 0 {
        let at = path::file_part(&out[..length]).map_or(0, |i| buffer + i * 2);
        unsafe { (file_part as *mut usize).write_unaligned(at) };
    }
    length as u32
}
/// GetFullPathNameA: through the ANSI code page both ways.
#[unsafe(no_mangle)]
pub extern "system" fn GetFullPathNameA(
    path: usize,
    capacity: u32,
    buffer: usize,
    file_part: usize,
) -> u32 {
    let mut wide = [0u16; UNITS];
    let Ok(pointer) = widen(door_windows_abi::widen(path), &mut wide) else {
        return fail(error::INVALID_PARAMETER);
    };
    let mut full = [0u16; 4096];
    let n = GetFullPathNameW(
        pointer as usize,
        full.len() as u32,
        full.as_mut_ptr() as usize,
        0,
    );
    if n == 0 || n as usize >= full.len() {
        return n;
    }
    // ANSI is UTF-8 here: the bytes needed, then the copy.
    let text = &full[..n as usize];
    let bytes: usize = char::decode_utf16(text.iter().copied())
        .map(|c| c.map_or(3, |c| c.len_utf8()))
        .sum();
    if (capacity as usize) <= bytes || buffer == 0 {
        return bytes as u32 + 1;
    }
    let mut at = buffer;
    let mut last = buffer;
    for c in char::decode_utf16(text.iter().copied()) {
        let c = c.unwrap_or('\u{fffd}');
        let mut utf8 = [0u8; 4];
        let encoded = c.encode_utf8(&mut utf8);
        unsafe { core::ptr::copy_nonoverlapping(encoded.as_ptr(), at as *mut u8, encoded.len()) };
        at += encoded.len();
        if c == '\\' {
            last = at;
        }
    }
    unsafe { (at as *mut u8).write(0) };
    if file_part != 0 {
        let value = if text.last() == Some(&(b'\\' as u16)) {
            0
        } else {
            last
        };
        unsafe { (file_part as *mut usize).write_unaligned(value) };
    }
    bytes as u32
}
/// The NUL-terminated UTF-16 string at `pointer` (at most 32,767 units).
fn wide_string<'a>(pointer: usize) -> Option<&'a [u16]> {
    if pointer == 0 {
        return None;
    }
    let mut n = 0;
    while n < 32_767 && unsafe { ((pointer as *const u16).add(n)).read_unaligned() } != 0 {
        n += 1;
    }
    Some(unsafe { core::slice::from_raw_parts(pointer as *const u16, n) })
}
