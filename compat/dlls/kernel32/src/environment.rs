//! Environment variables: Get/Set/ExpandEnvironmentStrings and
//! GetEnvironmentStrings over ntdll's process block, with the ANSI forms
//! converting through the ANSI code page.
use door_windows_abi::{self as abi, error};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorGetEnvironmentVariableW(name: u64, buffer: u64, size: u32) -> u32;
    fn DoorSetEnvironmentVariableW(name: u64, value: u64) -> i32;
    fn DoorExpandEnvironmentStringsW(source: u64, destination: u64, size: u32) -> u32;
    fn DoorGetEnvironmentStringsW() -> u64;
    fn DoorGetProcessHeap() -> u64;
    fn RtlAllocateHeap(heap: usize, flags: u32, bytes: usize) -> usize;
    fn RtlFreeHeap(heap: usize, flags: u32, pointer: usize) -> u8;
    fn NtWriteVirtualMemory(
        process: usize,
        destination: usize,
        source: usize,
        bytes: usize,
        written: usize,
    ) -> u32;
    fn RtlSetLastWin32Error(value: u32);
}
/// The longest ANSI argument converted on the stack.
pub(crate) const UNITS: usize = 4096;

fn copy(destination: u64, source: u64, bytes: usize) -> Result<(), u32> {
    if bytes == 0 {
        return Ok(());
    }
    for at in (0..bytes).step_by(abi::COPY_BYTES) {
        let n = (bytes - at).min(abi::COPY_BYTES);
        // SAFETY: integer addresses are data to the checked kernel boundary.
        if unsafe {
            NtWriteVirtualMemory(
                door_windows_abi::narrow(abi::CURRENT_PROCESS),
                door_windows_abi::narrow(destination + at as u64),
                door_windows_abi::narrow(source + at as u64),
                n,
                0,
            )
        } != abi::status::SUCCESS
        {
            return Err(error::NOACCESS);
        }
    }
    Ok(())
}
pub(crate) fn fail(code: u32) -> u32 {
    unsafe { RtlSetLastWin32Error(code) };
    0
}
/// Unicode to the ANSI code page (Windows-1252 subset); '?' otherwise.
pub(crate) fn ansi(unit: u16) -> u8 {
    match unit {
        0..=0x7f | 0xa0..=0xff => unit as u8,
        0x20ac => 0x80,
        0x2018 => 0x91,
        0x2019 => 0x92,
        0x201c => 0x93,
        0x201d => 0x94,
        0x2013 => 0x96,
        0x2014 => 0x97,
        _ => b'?',
    }
}
fn wide(byte: u8) -> u16 {
    match byte {
        0x80 => 0x20ac,
        0x91 => 0x2018,
        0x92 => 0x2019,
        0x93 => 0x201c,
        0x94 => 0x201d,
        0x96 => 0x2013,
        0x97 => 0x2014,
        b => u16::from(b),
    }
}
/// A NUL-terminated ANSI argument as a NUL-terminated wide string in `out`.
pub(crate) fn widen(pointer: u64, out: &mut [u16; UNITS]) -> Result<u64, u32> {
    if pointer == 0 {
        return Ok(0);
    }
    let mut i = 0;
    loop {
        let at = pointer + i as u64;
        let n = ((4096 - (at % 4096)) as usize).clamp(1, 256);
        let mut chunk = [0u8; 256];
        copy(chunk.as_mut_ptr() as u64, at, n)?;
        for &b in &chunk[..n] {
            if i + 1 >= UNITS {
                return Err(error::INVALID_PARAMETER);
            }
            if b == 0 {
                out[i] = 0;
                return Ok(out.as_ptr() as u64);
            }
            out[i] = wide(b);
            i += 1;
        }
    }
}
/// `units` as ANSI with a NUL into `buffer` when `size` holds them; the
/// GetEnvironmentVariableA-style count otherwise.
fn narrow(units: &[u16], buffer: u64, size: u32) -> Result<u32, u32> {
    if units.len() + 1 > size as usize || buffer == 0 {
        return Ok(units.len() as u32 + 1);
    }
    let mut bytes = [0u8; UNITS + 1];
    for (b, u) in bytes.iter_mut().zip(units) {
        *b = ansi(*u);
    }
    copy(buffer, bytes.as_ptr() as u64, units.len() + 1)?;
    Ok(units.len() as u32)
}

#[unsafe(no_mangle)]
pub extern "system" fn GetEnvironmentVariableW(name: usize, buffer: usize, size: u32) -> u32 {
    let name = door_windows_abi::widen(name);
    let buffer = door_windows_abi::widen(buffer);
    unsafe { DoorGetEnvironmentVariableW(name, buffer, size) }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetEnvironmentVariableW(name: usize, value: usize) -> i32 {
    let name = door_windows_abi::widen(name);
    let value = door_windows_abi::widen(value);
    unsafe { DoorSetEnvironmentVariableW(name, value) }
}
#[unsafe(no_mangle)]
pub extern "system" fn ExpandEnvironmentStringsW(
    source: usize,
    destination: usize,
    size: u32,
) -> u32 {
    let source = door_windows_abi::widen(source);
    let destination = door_windows_abi::widen(destination);
    unsafe { DoorExpandEnvironmentStringsW(source, destination, size) }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetEnvironmentStringsW() -> usize {
    door_windows_abi::narrow((move || -> u64 { unsafe { DoorGetEnvironmentStringsW() } })())
}
#[unsafe(no_mangle)]
pub extern "system" fn FreeEnvironmentStringsW(block: usize) -> i32 {
    let block = door_windows_abi::widen(block);
    {
        if block == 0 {
            return 0;
        }
        i32::from(
            unsafe {
                RtlFreeHeap(
                    door_windows_abi::narrow(DoorGetProcessHeap()),
                    0,
                    door_windows_abi::narrow(block),
                )
            } != 0,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn FreeEnvironmentStringsA(block: usize) -> i32 {
    let block = door_windows_abi::widen(block);
    FreeEnvironmentStringsW(abi::narrow(block))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetEnvironmentVariableA(name: usize, buffer: usize, size: u32) -> u32 {
    let name = door_windows_abi::widen(name);
    let buffer = door_windows_abi::widen(buffer);
    {
        let mut n = [0u16; UNITS];
        let mut value = [0u16; UNITS];
        let name = match widen(name, &mut n) {
            Ok(0) | Err(_) => return fail(error::INVALID_PARAMETER),
            Ok(p) => p,
        };
        let length =
            unsafe { DoorGetEnvironmentVariableW(name, value.as_mut_ptr() as u64, UNITS as u32) };
        if length == 0 {
            // Not found (the W call set the error) or an empty value.
            if buffer != 0 && size > 0 {
                let _ = copy(buffer, [0u8].as_ptr() as u64, 1);
            }
            return 0;
        }
        if length as usize >= UNITS {
            return length;
        }
        match narrow(&value[..length as usize], buffer, size) {
            Ok(v) => v,
            Err(e) => fail(e),
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetEnvironmentVariableA(name: usize, value: usize) -> i32 {
    let name = door_windows_abi::widen(name);
    let value = door_windows_abi::widen(value);
    {
        let mut n = [0u16; UNITS];
        let mut v = [0u16; UNITS];
        match (widen(name, &mut n), widen(value, &mut v)) {
            (Ok(name), Ok(value)) if name != 0 => unsafe {
                DoorSetEnvironmentVariableW(name, value)
            },
            _ => fail(error::INVALID_PARAMETER) as i32,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ExpandEnvironmentStringsA(
    source: usize,
    destination: usize,
    size: u32,
) -> u32 {
    let source = door_windows_abi::widen(source);
    let destination = door_windows_abi::widen(destination);
    {
        let mut s = [0u16; UNITS];
        let mut out = [0u16; UNITS];
        let source = match widen(source, &mut s) {
            Ok(0) | Err(_) => return fail(error::INVALID_PARAMETER),
            Ok(p) => p,
        };
        let needed =
            unsafe { DoorExpandEnvironmentStringsW(source, out.as_mut_ptr() as u64, UNITS as u32) };
        if needed == 0 || needed as usize > UNITS {
            return needed;
        }
        let units = &out[..needed as usize - 1];
        if destination != 0 && units.len() < size as usize {
            let mut bytes = [0u8; UNITS];
            for (b, u) in bytes.iter_mut().zip(units) {
                *b = ansi(*u);
            }
            if let Err(e) = copy(destination, bytes.as_ptr() as u64, units.len() + 1) {
                return fail(e);
            }
        }
        needed
    }
}
/// GetEnvironmentStrings (GetEnvironmentStringsA): an ANSI process-heap
/// copy of the block.
#[unsafe(no_mangle)]
pub extern "system" fn GetEnvironmentStrings() -> usize {
    door_windows_abi::narrow((move || -> u64 {
        let block = unsafe { DoorGetEnvironmentStringsW() };
        if block == 0 {
            return 0;
        }
        // The block's length: entries until the empty one.
        let mut units = 0usize;
        let mut previous_nul = false;
        loop {
            let mut u = [0u8; 2];
            if copy(u.as_mut_ptr() as u64, block + 2 * units as u64, 2).is_err() {
                FreeEnvironmentStringsW(abi::narrow(block));
                return 0;
            }
            units += 1;
            let nul = u == [0, 0];
            if nul && (previous_nul || units == 1) {
                break;
            }
            previous_nul = nul;
        }
        let heap = unsafe { DoorGetProcessHeap() };
        let out = unsafe { RtlAllocateHeap(door_windows_abi::narrow(heap), 0, units.max(2)) };
        if out != 0 {
            let mut chunk = [0u16; 512];
            let mut bytes = [0u8; 512];
            for start in (0..units).step_by(512) {
                let n = (units - start).min(512);
                if copy(chunk.as_mut_ptr() as u64, block + 2 * start as u64, 2 * n).is_err() {
                    break;
                }
                for (b, u) in bytes.iter_mut().zip(&chunk[..n]) {
                    *b = ansi(*u);
                }
                let _ = copy(
                    door_windows_abi::widen(out + door_windows_abi::narrow(start as u64)),
                    bytes.as_ptr() as u64,
                    n,
                );
            }
            if units == 1 {
                let _ = copy(door_windows_abi::widen(out + 1), [0u8].as_ptr() as u64, 1);
            }
        }
        FreeEnvironmentStringsW(abi::narrow(block));
        door_windows_abi::widen(out)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetEnvironmentStringsA() -> usize {
    GetEnvironmentStrings()
}

/// The temporary directory Windows gives: TMP, else TEMP, else
/// USERPROFILE, else the Windows directory, with a trailing backslash.
fn temp_path() -> ([u16; 520], usize) {
    let mut out = [0u16; 520];
    for name in ["TMP", "TEMP", "USERPROFILE"] {
        let mut key = [0u16; 16];
        for (slot, unit) in key.iter_mut().zip(name.encode_utf16()) {
            *slot = unit;
        }
        let n = unsafe {
            DoorGetEnvironmentVariableW(
                key.as_ptr() as u64,
                out.as_mut_ptr() as u64,
                out.len() as u32 - 1,
            )
        } as usize;
        if n > 0 && n < out.len() - 1 {
            let n = if out[n - 1] == u16::from(b'\\') {
                n
            } else {
                out[n] = u16::from(b'\\');
                n + 1
            };
            return (out, n);
        }
    }
    let n = r"C:\Windows\"
        .encode_utf16()
        .zip(out.iter_mut())
        .map(|(u, slot)| *slot = u)
        .count();
    (out, n)
}
/// `GetTempPathW`: the length without the NUL, or the size needed with it
/// when `buffer` is too small.
#[unsafe(no_mangle)]
pub extern "system" fn GetTempPathW(size: u32, buffer: usize) -> u32 {
    let buffer = door_windows_abi::widen(buffer);
    {
        let (path, n) = temp_path();
        if (size as usize) <= n {
            return n as u32 + 1;
        }
        let mut bytes = [0u8; 1042];
        for (i, unit) in path[..n].iter().chain([&0]).enumerate() {
            bytes[2 * i..2 * i + 2].copy_from_slice(&unit.to_le_bytes());
        }
        match copy(buffer, bytes.as_ptr() as u64, 2 * n + 2) {
            Ok(()) => n as u32,
            Err(_) => 0,
        }
    }
}
/// `GetTempPath2W`: the same directory for a program that is not SYSTEM.
#[unsafe(no_mangle)]
pub extern "system" fn GetTempPath2W(size: u32, buffer: usize) -> u32 {
    let buffer = door_windows_abi::widen(buffer);
    GetTempPathW(size, abi::narrow(buffer))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetTempPathA(size: u32, buffer: usize) -> u32 {
    let buffer = door_windows_abi::widen(buffer);
    {
        let (path, n) = temp_path();
        if (size as usize) <= n {
            return n as u32 + 1;
        }
        let mut out = [0u8; 521];
        for (slot, &unit) in out.iter_mut().zip(&path[..n]) {
            *slot = if unit < 256 { unit as u8 } else { b'?' };
        }
        match copy(buffer, out.as_ptr() as u64, n + 1) {
            Ok(()) => n as u32,
            Err(_) => 0,
        }
    }
}
