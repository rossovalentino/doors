//! DPAPI: CryptProtectData/CryptUnprotectData under the user's or the
//! computer's protection key, and CryptProtectMemory. Each key is 256
//! random bits kept at `Software\Doors\DataProtection` (HKCU for the user,
//! HKLM for CRYPTPROTECT_LOCAL_MACHINE), shared by 32- and 64-bit
//! programs; the first program that needs one creates it.
use super::{BAD_BLOB, LOCAL_MACHINE, blob, fail, local_copy, wide_string};
use crate::{E_INVALIDARG, ERROR_INVALID_PARAMETER, ERROR_NOT_ENOUGH_MEMORY, dpapi};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use door_windows_abi::widen;

#[link(name = "advapi32", kind = "dylib")]
unsafe extern "system" {
    fn RegCreateKeyExW(
        key: usize,
        subkey: usize,
        reserved: u32,
        class: usize,
        options: u32,
        access: u32,
        security: usize,
        result: usize,
        disposition: usize,
    ) -> u32;
    fn RegQueryValueExW(
        key: usize,
        name: usize,
        reserved: usize,
        kind: usize,
        data: usize,
        size: usize,
    ) -> u32;
    fn RegSetValueExW(
        key: usize,
        name: usize,
        reserved: u32,
        kind: u32,
        data: usize,
        size: u32,
    ) -> u32;
    fn RegCloseKey(key: usize) -> u32;
}
#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorGenRandom(buffer: u64, length: u32) -> u32;
}

const HKEY_CURRENT_USER: usize = 0x8000_0001u32 as i32 as isize as usize;
const HKEY_LOCAL_MACHINE: usize = 0x8000_0002u32 as i32 as isize as usize;
const KEY_READ: u32 = 0x2_0019;
const KEY_WRITE: u32 = 0x2_0006;
const KEY_WOW64_64KEY: u32 = 0x100;
const REG_BINARY: u32 = 3;
const CRYPTPROTECTMEMORY_CROSS_PROCESS: u32 = 1;
const CRYPTPROTECTMEMORY_SAME_LOGON: u32 = 2;

fn utf16z(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
fn random<const N: usize>() -> Result<[u8; N], u32> {
    let mut out = [0u8; N];
    match unsafe { DoorGenRandom(out.as_mut_ptr() as u64, N as u32) } {
        0 => Ok(out),
        e => Err(e),
    }
}
/// The stored key, or none.
fn query(key: usize, name: &[u16]) -> Option<[u8; 32]> {
    let mut value = [0u8; 32];
    let mut size = 32u32;
    let mut kind = 0u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr() as usize,
            0,
            &mut kind as *mut u32 as usize,
            value.as_mut_ptr() as usize,
            &mut size as *mut u32 as usize,
        )
    };
    (status == 0 && kind == REG_BINARY && size == 32).then_some(value)
}
/// Reads (creating on first use) the protection key of the user or the
/// computer.
fn load(machine: bool) -> Result<[u8; 32], u32> {
    let root = if machine {
        HKEY_LOCAL_MACHINE
    } else {
        HKEY_CURRENT_USER
    };
    let path = utf16z("Software\\Doors\\DataProtection");
    let name = utf16z("ProtectionKey");
    let mut key = 0usize;
    let open = |access: u32, key: &mut usize| unsafe {
        RegCreateKeyExW(
            root,
            path.as_ptr() as usize,
            0,
            0,
            0,
            access | KEY_WOW64_64KEY,
            0,
            key as *mut usize as usize,
            0,
        )
    };
    let writable = open(KEY_READ | KEY_WRITE, &mut key) == 0;
    if !writable {
        let status = open(KEY_READ, &mut key);
        if status != 0 {
            return Err(status);
        }
    }
    let result = match query(key, &name) {
        Some(value) => Ok(value),
        None if writable => random::<32>().and_then(|value| {
            let status = unsafe {
                RegSetValueExW(
                    key,
                    name.as_ptr() as usize,
                    0,
                    REG_BINARY,
                    value.as_ptr() as usize,
                    32,
                )
            };
            // Another program may have stored its key first: use what the
            // registry holds.
            match (status, query(key, &name)) {
                (_, Some(stored)) => Ok(stored),
                (0, None) => Ok(value),
                (e, None) => Err(e),
            }
        }),
        None => Err(crate::E_ACCESSDENIED),
    };
    unsafe { RegCloseKey(key) };
    result
}
struct Cache {
    held: AtomicBool,
    keys: core::cell::UnsafeCell<[Option<[u8; 32]>; 3]>,
}
unsafe impl Sync for Cache {}
static CACHE: Cache = Cache {
    held: AtomicBool::new(false),
    keys: core::cell::UnsafeCell::new([None; 3]),
};
/// Key 0: the user's, 1: the computer's, 2: this process's memory key.
fn key(which: usize) -> Result<[u8; 32], u32> {
    while CACHE.held.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    let keys = unsafe { &mut *CACHE.keys.get() };
    let result = match keys[which] {
        Some(k) => Ok(k),
        None => {
            let loaded = match which {
                0 => load(false),
                1 => load(true),
                _ => random::<32>(),
            };
            if let Ok(k) = loaded {
                keys[which] = Some(k);
            }
            loaded
        }
    };
    CACHE.held.store(false, Ordering::Release);
    result
}
/// Writes a DATA_BLOB { cbData, pbData } holding a LocalAlloc copy.
fn put_blob(out: u64, data: &[u8]) -> Result<(), u32> {
    let block = local_copy(data);
    if block == 0 {
        return Err(ERROR_NOT_ENOUGH_MEMORY);
    }
    unsafe {
        (out as *mut u32).write_unaligned(data.len() as u32);
        ((out + core::mem::size_of::<usize>() as u64) as *mut usize)
            .write_unaligned(block as usize);
    }
    Ok(())
}
fn finish(result: Result<(), u32>) -> i32 {
    match result {
        Ok(()) => 1,
        Err(e) => fail(e),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptProtectData(
    input: usize,
    description: usize,
    entropy: usize,
    _reserved: usize,
    _prompt: usize,
    flags: u32,
    output: usize,
) -> i32 {
    let (input, output) = (widen(input), widen(output));
    finish((|| {
        if input == 0 || output == 0 {
            return Err(E_INVALIDARG);
        }
        let machine = flags & LOCAL_MACHINE != 0;
        let key = key(usize::from(machine))?;
        let description = wide_string(widen(description), 4096);
        let sealed = dpapi::seal(
            &key,
            random::<12>()?,
            flags,
            &description,
            blob(widen(entropy)),
            blob(input),
        )
        .ok_or(BAD_BLOB)?;
        put_blob(output, &sealed)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptUnprotectData(
    input: usize,
    description: usize,
    entropy: usize,
    _reserved: usize,
    _prompt: usize,
    _flags: u32,
    output: usize,
) -> i32 {
    let (input, output, description) = (widen(input), widen(output), widen(description));
    finish((|| {
        if input == 0 || output == 0 {
            return Err(E_INVALIDARG);
        }
        let data = blob(input);
        let sealed_with = dpapi::flags(data).ok_or(BAD_BLOB)?;
        let key = key(usize::from(sealed_with & LOCAL_MACHINE != 0))?;
        let (text, plain) = dpapi::open(&key, blob(widen(entropy)), data).ok_or(BAD_BLOB)?;
        if description != 0 {
            let units: Vec<u8> = text
                .iter()
                .chain(&[0])
                .flat_map(|u| u.to_le_bytes())
                .collect();
            let copy = local_copy(&units);
            if copy == 0 {
                return Err(ERROR_NOT_ENOUGH_MEMORY);
            }
            unsafe { (description as *mut usize).write_unaligned(copy as usize) };
        }
        put_blob(output, &plain)
    })())
}
/// The CryptProtectMemory key for its scope flags.
fn memory_key(flags: u32) -> Result<[u8; 32], u32> {
    use sha2::Digest;
    let base = match flags {
        0 => return key(2),
        CRYPTPROTECTMEMORY_CROSS_PROCESS => key(1)?,
        CRYPTPROTECTMEMORY_SAME_LOGON => key(0)?,
        _ => return Err(ERROR_INVALID_PARAMETER),
    };
    let mut h = sha2::Sha256::new();
    h.update(b"Doors CryptProtectMemory");
    h.update(base);
    Ok(h.finalize().into())
}
fn memory(data: usize, length: u32, flags: u32, encrypt: bool) -> i32 {
    let data = widen(data);
    finish((|| {
        if data == 0 || !length.is_multiple_of(16) {
            return Err(ERROR_INVALID_PARAMETER);
        }
        let key = memory_key(flags)?;
        let bytes = unsafe { core::slice::from_raw_parts_mut(data as *mut u8, length as usize) };
        if dpapi::memory(&key, bytes, encrypt) {
            Ok(())
        } else {
            Err(ERROR_INVALID_PARAMETER)
        }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptProtectMemory(data: usize, length: u32, flags: u32) -> i32 {
    memory(data, length, flags, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptUnprotectMemory(data: usize, length: u32, flags: u32) -> i32 {
    memory(data, length, flags, false)
}
