//! File version information (api-ms-win-core-version, which version.dll
//! forwards to as Windows' version.dll forwards to kernelbase): a file's
//! VS_VERSIONINFO resource (RT_VERSION, id 1) read through a data-file load,
//! copied whole, then queried by key path. GetFileVersionInfoSize asks for
//! twice the resource plus 4 bytes, as Windows does: the copy ends with
//! "FE2X" and the spare half holds what VerQueryValueA converts to ANSI.
use door_windows_abi::{self as abi, version_info};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn RtlSetLastWin32Error(error: u32);
    fn RtlGetLastWin32Error() -> u32;
}

const RT_VERSION: usize = 16;
const VS_VERSION_INFO: usize = 1;
/// LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE.
const DATA_FILE: u32 = 0x2 | 0x20;
const ERROR_INVALID_DATA: u32 = 13;
const ERROR_RESOURCE_TYPE_NOT_FOUND: u32 = 1813;
/// Longest query path or file name taken.
const UNITS: usize = 1024;

fn fail(code: u32) -> i32 {
    unsafe { RtlSetLastWin32Error(code) };
    0
}

/// `f` over the version resource of the module file `path` (wide).
fn with_resource<R>(path: usize, f: impl FnOnce(&[u8]) -> R) -> Result<R, u32> {
    let module = crate::guest::LoadLibraryExW(path, 0, DATA_FILE);
    if module == 0 {
        return Err(unsafe { RtlGetLastWin32Error() });
    }
    let found = crate::application::FindResourceW(module, VS_VERSION_INFO, RT_VERSION);
    let result = if found == 0 {
        Err(ERROR_RESOURCE_TYPE_NOT_FOUND)
    } else {
        let bytes = crate::application::SizeofResource(module, found) as usize;
        let data = crate::application::LoadResource(module, found);
        if data == 0 || bytes < 6 {
            Err(ERROR_RESOURCE_TYPE_NOT_FOUND)
        } else {
            // SAFETY: the data file's image holds `bytes` of resource data.
            Ok(f(unsafe {
                core::slice::from_raw_parts(data as *const u8, bytes)
            }))
        }
    };
    crate::guest::FreeLibrary(module);
    result
}

#[unsafe(no_mangle)]
pub extern "system" fn GetFileVersionInfoSizeExW(_flags: u32, path: usize, handle: usize) -> u32 {
    if handle != 0 {
        // SAFETY: the caller's DWORD.
        unsafe { (handle as *mut u32).write_unaligned(0) };
    }
    match with_resource(path, |resource| resource.len()) {
        Ok(bytes) => (bytes * 2 + 4) as u32,
        Err(code) => fail(code) as u32,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetFileVersionInfoSizeW(path: usize, handle: usize) -> u32 {
    GetFileVersionInfoSizeExW(0, path, handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetFileVersionInfoSizeExA(flags: u32, path: usize, handle: usize) -> u32 {
    let mut units = [0u16; crate::environment::UNITS];
    match crate::environment::widen(abi::widen(path), &mut units) {
        Ok(wide) => GetFileVersionInfoSizeExW(flags, abi::narrow(wide), handle),
        Err(code) => fail(code) as u32,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetFileVersionInfoSizeA(path: usize, handle: usize) -> u32 {
    GetFileVersionInfoSizeExA(0, path, handle)
}

#[unsafe(no_mangle)]
pub extern "system" fn GetFileVersionInfoExW(
    _flags: u32,
    path: usize,
    _handle: u32,
    bytes: u32,
    data: usize,
) -> i32 {
    if data == 0 {
        return fail(ERROR_INVALID_DATA);
    }
    let copied = with_resource(path, |resource| {
        let wanted = version_info::length(resource).unwrap_or(resource.len());
        let n = resource.len().min(bytes as usize);
        // SAFETY: the caller's buffer holds `bytes`.
        unsafe { core::ptr::copy_nonoverlapping(resource.as_ptr(), data as *mut u8, n) };
        // The spare-half marker, when it fits after the resource.
        if bytes as usize > wanted {
            let marker = (bytes as usize - wanted).min(4);
            unsafe {
                core::ptr::copy_nonoverlapping(
                    version_info::SPARE_SIGNATURE.as_ptr(),
                    (data + wanted) as *mut u8,
                    marker,
                )
            };
        }
    });
    match copied {
        Ok(()) => 1,
        Err(code) => fail(code),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetFileVersionInfoW(
    path: usize,
    handle: u32,
    bytes: u32,
    data: usize,
) -> i32 {
    GetFileVersionInfoExW(0, path, handle, bytes, data)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetFileVersionInfoExA(
    flags: u32,
    path: usize,
    handle: u32,
    bytes: u32,
    data: usize,
) -> i32 {
    let mut units = [0u16; crate::environment::UNITS];
    match crate::environment::widen(abi::widen(path), &mut units) {
        Ok(wide) => GetFileVersionInfoExW(flags, abi::narrow(wide), handle, bytes, data),
        Err(code) => fail(code),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetFileVersionInfoA(
    path: usize,
    handle: u32,
    bytes: u32,
    data: usize,
) -> i32 {
    GetFileVersionInfoExA(0, path, handle, bytes, data)
}

/// The copied resource at `block`, bounded by its own length.
fn resource<'a>(block: usize) -> Option<&'a [u8]> {
    if block == 0 {
        return None;
    }
    // SAFETY: a block GetFileVersionInfo filled; its first word is its length.
    let length = usize::from(unsafe { (block as *const u16).read_unaligned() });
    (length >= 6).then(|| unsafe { core::slice::from_raw_parts(block as *const u8, length) })
}
/// Answer a query: the value's address and length into the caller's
/// pointer and UINT.
fn answer(
    block: usize,
    path: &[u16],
    buffer: usize,
    length: usize,
) -> Result<(usize, u32, bool), u32> {
    let resource = resource(block).ok_or(ERROR_INVALID_DATA)?;
    let Some(value) = version_info::query(resource, path) else {
        if length != 0 {
            unsafe { (length as *mut u32).write_unaligned(0) };
        }
        return Err(ERROR_RESOURCE_TYPE_NOT_FOUND);
    };
    let address = block + value.offset;
    if buffer != 0 {
        // SAFETY: the caller's pointer-sized out parameter.
        unsafe { (buffer as *mut usize).write_unaligned(address) };
    }
    if length != 0 {
        unsafe { (length as *mut u32).write_unaligned(u32::from(value.length)) };
    }
    Ok((address, u32::from(value.length), value.text))
}
/// A NUL-terminated wide path, bounded.
fn wide_path(pointer: usize, out: &mut [u16; UNITS]) -> usize {
    if pointer == 0 {
        return 0;
    }
    let mut n = 0;
    while n < UNITS {
        let unit = unsafe { (pointer as *const u16).add(n).read_unaligned() };
        if unit == 0 {
            break;
        }
        out[n] = unit;
        n += 1;
    }
    n
}

#[unsafe(no_mangle)]
pub extern "system" fn VerQueryValueW(
    block: usize,
    path: usize,
    buffer: usize,
    length: usize,
) -> i32 {
    let mut units = [0u16; UNITS];
    let n = wide_path(path, &mut units);
    match answer(block, &units[..n], buffer, length) {
        Ok(_) => 1,
        Err(code) => fail(code),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn VerQueryValueA(
    block: usize,
    path: usize,
    buffer: usize,
    length: usize,
) -> i32 {
    let mut wide = [0u16; crate::environment::UNITS];
    let path = match crate::environment::widen(abi::widen(path), &mut wide) {
        Ok(_) => {
            let n = wide.iter().position(|u| *u == 0).unwrap_or(0);
            &wide[..n]
        }
        Err(code) => return fail(code),
    };
    let Some(whole) = resource(block) else {
        return fail(ERROR_INVALID_DATA);
    };
    let spare = block + whole.len() + 4;
    match answer(block, path, buffer, length) {
        Ok((address, units, true)) => {
            // Text: converted into the spare half at the same offset.
            let target = spare + (address - block);
            for i in 0..units as usize {
                let unit = unsafe { (address as *const u16).add(i).read_unaligned() };
                unsafe { ((target + i) as *mut u8).write(crate::environment::ansi(unit)) };
            }
            if buffer != 0 {
                unsafe { (buffer as *mut usize).write_unaligned(target) };
            }
            1
        }
        Ok(_) => 1,
        Err(code) => fail(code),
    }
}

/// `VerLanguageNameW`: a language's display name, as Windows 11 words it,
/// for the languages Doors carries; the length written.
#[unsafe(no_mangle)]
pub extern "system" fn VerLanguageNameW(language: u32, buffer: usize, capacity: u32) -> u32 {
    let name = match language & 0xffff {
        0x0000 => "Language Neutral",
        0x007f => "Invariant Language (Invariant Country)",
        0x0409 => "English (United States)",
        0x0809 => "English (United Kingdom)",
        _ => return fail(ERROR_RESOURCE_TYPE_NOT_FOUND) as u32,
    };
    let units = name.encode_utf16();
    if buffer == 0 || capacity == 0 {
        return name.len() as u32;
    }
    let mut written = 0;
    for unit in units.take(capacity as usize - 1) {
        unsafe { (buffer as *mut u16).add(written).write_unaligned(unit) };
        written += 1;
    }
    unsafe { (buffer as *mut u16).add(written).write_unaligned(0) };
    written as u32
}
#[unsafe(no_mangle)]
pub extern "system" fn VerLanguageNameA(language: u32, buffer: usize, capacity: u32) -> u32 {
    let mut wide = [0u16; 64];
    let n = VerLanguageNameW(language, wide.as_mut_ptr() as usize, 64) as usize;
    if n == 0 || buffer == 0 || capacity == 0 {
        return n as u32;
    }
    let mut written = 0;
    for unit in wide[..n].iter().take(capacity as usize - 1) {
        unsafe { ((buffer + written) as *mut u8).write(crate::environment::ansi(*unit)) };
        written += 1;
    }
    unsafe { ((buffer + written) as *mut u8).write(0) };
    written as u32
}
