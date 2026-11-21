//! The ANSI forms of the file and module calls Chromium and Electron import
//! (FindFirstFileExA, GetModuleFileNameA, ...), each over its wide form
//! through the ANSI code page, plus lstrcmp(i), GetTempFileName and the
//! DLL-directory calls (P18 M7, from the measured import tables).
use crate::environment::{UNITS, ansi, fail, widen};
use door_windows_abi::error;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorGetModuleFileNameW(module: u64, buffer: u64, capacity: u32) -> u32;
    fn DoorGetFileAttributesW(path: u64) -> u32;
    fn DoorFindResource(module: u64, kind: u64, name: u64, language: u16) -> u64;
    fn DoorGetTickCount64() -> u64;
}

/// WIN32_FIND_DATAW: attributes and times (44 bytes), then cFileName[260]
/// and cAlternateFileName[14] wide.
const FIND_W: usize = 44 + 2 * 260 + 2 * 14;
/// WIN32_FIND_DATAA: the same 44 bytes, then the names as ANSI.
const FIND_A_NAME: usize = 44;
const FIND_A_ALTERNATE: usize = 44 + 260;

fn narrow_into(units: &[u16], out: &mut [u8]) {
    let n = units
        .iter()
        .position(|&u| u == 0)
        .unwrap_or(units.len())
        .min(out.len() - 1);
    for (b, u) in out.iter_mut().zip(&units[..n]) {
        *b = ansi(*u);
    }
    out[n] = 0;
}
/// A WIN32_FIND_DATAW at `wide` as WIN32_FIND_DATAA at `destination`.
fn find_data_to_ansi(wide: &[u8; FIND_W], destination: u64) {
    let mut out = [0u8; 320];
    out[..44].copy_from_slice(&wide[..44]);
    let units: [u16; 274] =
        core::array::from_fn(|i| u16::from_le_bytes([wide[44 + 2 * i], wide[45 + 2 * i]]));
    narrow_into(&units[..260], &mut out[FIND_A_NAME..FIND_A_NAME + 260]);
    narrow_into(
        &units[260..],
        &mut out[FIND_A_ALTERNATE..FIND_A_ALTERNATE + 14],
    );
    // SAFETY: the caller's WIN32_FIND_DATAA.
    unsafe { core::ptr::copy_nonoverlapping(out.as_ptr(), destination as *mut u8, out.len()) };
}

#[unsafe(no_mangle)]
pub extern "system" fn FindFirstFileExA(
    pattern: usize,
    level: u32,
    destination: usize,
    operation: u32,
    filter: usize,
    flags: u32,
) -> usize {
    let pattern = door_windows_abi::widen(pattern);
    let destination = door_windows_abi::widen(destination);
    let mut units = [0u16; UNITS];
    let Ok(wide_pattern) = widen(pattern, &mut units) else {
        fail(error::INVALID_PARAMETER);
        return usize::MAX;
    };
    let mut data = [0u8; FIND_W];
    let handle = crate::application::FindFirstFileExW(
        wide_pattern as usize,
        level,
        data.as_mut_ptr() as usize,
        operation,
        filter,
        flags,
    );
    if handle != usize::MAX && destination != 0 {
        find_data_to_ansi(&data, destination);
    }
    handle
}
#[unsafe(no_mangle)]
pub extern "system" fn FindFirstFileA(pattern: usize, destination: usize) -> usize {
    FindFirstFileExA(pattern, 0, destination, 0, 0, 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn FindNextFileA(handle: usize, destination: usize) -> i32 {
    let destination = door_windows_abi::widen(destination);
    let mut data = [0u8; FIND_W];
    let ok = crate::application::FindNextFileW(handle, data.as_mut_ptr() as usize);
    if ok != 0 && destination != 0 {
        find_data_to_ansi(&data, destination);
    }
    ok
}
/// A resource name or type: an integer (MAKEINTRESOURCE) as is, else the
/// ANSI string widened into `out`.
fn resource_name(value: u64, out: &mut [u16; UNITS]) -> Result<u64, u32> {
    if value < 0x10000 {
        return Ok(value);
    }
    widen(value, out)
}
#[unsafe(no_mangle)]
pub extern "system" fn FindResourceA(module: usize, name: usize, kind: usize) -> usize {
    let module = door_windows_abi::widen(module);
    let (mut a, mut b) = ([0u16; UNITS], [0u16; UNITS]);
    let (Ok(name), Ok(kind)) = (
        resource_name(door_windows_abi::widen(name), &mut a),
        resource_name(door_windows_abi::widen(kind), &mut b),
    ) else {
        fail(error::INVALID_PARAMETER);
        return 0;
    };
    door_windows_abi::narrow(unsafe { DoorFindResource(module, kind, name, 0) })
}
#[unsafe(no_mangle)]
pub extern "system" fn GetFileAttributesA(path: usize) -> u32 {
    let mut units = [0u16; UNITS];
    match widen(door_windows_abi::widen(path), &mut units) {
        Ok(p) if p != 0 => unsafe { DoorGetFileAttributesW(p) },
        _ => {
            fail(error::INVALID_PARAMETER);
            u32::MAX
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetDiskFreeSpaceA(
    root: usize,
    sectors_per_cluster: usize,
    bytes_per_sector: usize,
    free_clusters: usize,
    total_clusters: usize,
) -> i32 {
    let mut units = [0u16; UNITS];
    let Ok(root) = widen(door_windows_abi::widen(root), &mut units) else {
        return fail(error::INVALID_PARAMETER) as i32;
    };
    crate::machine::GetDiskFreeSpaceW(
        root as usize,
        sectors_per_cluster,
        bytes_per_sector,
        free_clusters,
        total_clusters,
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn GetModuleFileNameA(module: usize, buffer: usize, capacity: u32) -> u32 {
    let module = door_windows_abi::widen(module);
    let buffer = door_windows_abi::widen(buffer);
    let mut wide = [0u16; 1024];
    let n = unsafe { DoorGetModuleFileNameW(module, wide.as_mut_ptr() as u64, 1024) } as usize;
    if n == 0 || capacity == 0 {
        return if capacity == 0 {
            fail(error::INSUFFICIENT_BUFFER)
        } else {
            0
        };
    }
    let stored = n.min(capacity as usize - 1);
    let mut bytes = [0u8; 1025];
    for (b, u) in bytes.iter_mut().zip(&wide[..stored]) {
        *b = ansi(*u);
    }
    bytes[stored] = 0;
    // SAFETY: the caller's buffer of `capacity` bytes.
    unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), buffer as *mut u8, stored + 1) };
    if n >= capacity as usize {
        fail(error::INSUFFICIENT_BUFFER);
        return capacity;
    }
    n as u32
}

// ---- comparison -----------------------------------------------------------------

/// lstrcmp(i)W: CompareString with the user's locale (word sort), as
/// Windows defines them; NULL sorts first.
fn compare_wide(a: usize, b: usize, ignore_case: bool) -> i32 {
    match (a, b) {
        (0, 0) => return 0,
        (0, _) => return -1,
        (_, 0) => return 1,
        _ => {}
    }
    let flags = if ignore_case { 1 } else { 0 };
    let result = crate::nls::CompareStringW(0x400, flags, a, -1, b, -1);
    match result {
        1 => -1,
        3 => 1,
        _ => 0,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn lstrcmpW(a: usize, b: usize) -> i32 {
    compare_wide(a, b, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn lstrcmpiW(a: usize, b: usize) -> i32 {
    compare_wide(a, b, true)
}
fn compare_ansi(a: usize, b: usize, ignore_case: bool) -> i32 {
    let (mut x, mut y) = ([0u16; UNITS], [0u16; UNITS]);
    let (Ok(a), Ok(b)) = (
        widen(door_windows_abi::widen(a), &mut x),
        widen(door_windows_abi::widen(b), &mut y),
    ) else {
        return 0;
    };
    compare_wide(a as usize, b as usize, ignore_case)
}
#[unsafe(no_mangle)]
pub extern "system" fn lstrcmpA(a: usize, b: usize) -> i32 {
    compare_ansi(a, b, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn lstrcmpiA(a: usize, b: usize) -> i32 {
    compare_ansi(a, b, true)
}

// ---- temporary file names -------------------------------------------------------

const FILE_EXISTS: u32 = 80;
fn put_wide(buffer: u64, units: &[u16]) {
    for (i, u) in units.iter().chain([&0]).enumerate() {
        // SAFETY: the caller's MAX_PATH buffer.
        unsafe { (buffer as *mut u16).add(i).write_unaligned(*u) };
    }
}
/// `path\<prefix up to 3><hex>.TMP`: with `unique` 0, the first free name
/// from the tick count, created empty (as Windows does); otherwise that
/// number's name, not created.
fn temp_name(path: &[u16], prefix: &[u16], unique: u32, buffer: u64) -> u32 {
    let hex = |n: u32, out: &mut [u16; 8]| -> usize {
        let mut len = 0;
        let mut started = false;
        for shift in (0..8).rev() {
            let d = (n >> (shift * 4)) & 0xf;
            if d != 0 || started || shift == 0 {
                started = true;
                out[len] = u16::from(b"0123456789ABCDEF"[d as usize]);
                len += 1;
            }
        }
        len
    };
    let build = |n: u32, out: &mut [u16; 300]| -> usize {
        let mut len = 0;
        for &u in path {
            out[len] = u;
            len += 1;
        }
        if len > 0 && out[len - 1] != u16::from(b'\\') && out[len - 1] != u16::from(b'/') {
            out[len] = u16::from(b'\\');
            len += 1;
        }
        for &u in prefix.iter().take(3) {
            out[len] = u;
            len += 1;
        }
        let mut digits = [0u16; 8];
        let d = hex(n & 0xffff, &mut digits);
        out[len..len + d].copy_from_slice(&digits[..d]);
        len += d;
        for u in ".TMP".encode_utf16() {
            out[len] = u;
            len += 1;
        }
        len
    };
    let mut name = [0u16; 300];
    if path.len() + 3 + 4 + 4 + 2 > 259 {
        fail(error::INVALID_PARAMETER);
        return 0;
    }
    if unique != 0 {
        let len = build(unique, &mut name);
        put_wide(buffer, &name[..len]);
        return unique & 0xffff;
    }
    let start = (unsafe { DoorGetTickCount64() } as u32 & 0xffff).max(1);
    for step in 0..0xffff_u32 {
        let n = ((start + step - 1) % 0xffff) + 1;
        let len = build(n, &mut name);
        name[len] = 0;
        let handle = crate::guest::CreateFileW(
            name.as_ptr() as usize,
            0x4000_0000,
            0,
            0,
            1, // CREATE_NEW
            0x80,
            0,
        );
        if handle != usize::MAX {
            crate::guest::CloseHandle(handle);
            put_wide(buffer, &name[..len]);
            return n;
        }
        let code = crate::guest::GetLastError();
        if code != FILE_EXISTS && code != 183 {
            return 0;
        }
    }
    fail(FILE_EXISTS)
}
fn wide_arg(pointer: u64, out: &mut [u16; UNITS]) -> usize {
    if pointer == 0 {
        return 0;
    }
    let mut n = 0;
    while n < UNITS - 1 {
        // SAFETY: the caller's NUL-terminated string.
        let u = unsafe { (pointer as *const u16).add(n).read_unaligned() };
        if u == 0 {
            break;
        }
        out[n] = u;
        n += 1;
    }
    n
}
#[unsafe(no_mangle)]
pub extern "system" fn GetTempFileNameW(
    path: usize,
    prefix: usize,
    unique: u32,
    out: usize,
) -> u32 {
    let (mut p, mut q) = ([0u16; UNITS], [0u16; UNITS]);
    let pn = wide_arg(door_windows_abi::widen(path), &mut p);
    let qn = wide_arg(door_windows_abi::widen(prefix), &mut q);
    temp_name(&p[..pn], &q[..qn], unique, door_windows_abi::widen(out))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetTempFileNameA(
    path: usize,
    prefix: usize,
    unique: u32,
    out: usize,
) -> u32 {
    let (mut p, mut q) = ([0u16; UNITS], [0u16; UNITS]);
    let (Ok(path), Ok(prefix)) = (
        widen(door_windows_abi::widen(path), &mut p),
        widen(door_windows_abi::widen(prefix), &mut q),
    ) else {
        return fail(error::INVALID_PARAMETER);
    };
    let (mut a, mut b) = ([0u16; UNITS], [0u16; UNITS]);
    let an = wide_arg(path, &mut a);
    let bn = wide_arg(prefix, &mut b);
    let mut wide = [0u16; 300];
    let n = temp_name(&a[..an], &b[..bn], unique, wide.as_mut_ptr() as u64);
    if n != 0 {
        let len = wide.iter().position(|&u| u == 0).unwrap_or(0);
        let mut bytes = [0u8; 300];
        narrow_into(&wide[..len + 1], &mut bytes);
        // SAFETY: the caller's MAX_PATH buffer.
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                door_windows_abi::widen(out) as *mut u8,
                len + 1,
            )
        };
    }
    n
}

// ---- DLL directories -------------------------------------------------------------

/// LOAD_LIBRARY_SEARCH_* defaults: accepted (the loader searches the
/// application's folder and System32, as with the secure default).
#[unsafe(no_mangle)]
pub extern "system" fn SetDefaultDllDirectories(flags: u32) -> i32 {
    if flags == 0 || flags & !0x1f00 != 0 {
        return fail(error::INVALID_PARAMETER) as i32;
    }
    1
}
/// AddDllDirectory: a cookie for the folder (the loader's search order does
/// not take extra folders yet; the cookie removes nothing).
#[unsafe(no_mangle)]
pub extern "system" fn AddDllDirectory(path: usize) -> usize {
    if path == 0 {
        fail(error::INVALID_PARAMETER);
        return 0;
    }
    path
}
#[unsafe(no_mangle)]
pub extern "system" fn RemoveDllDirectory(cookie: usize) -> i32 {
    i32::from(cookie != 0)
}
