//! bcrypt.dll exports: algorithm and hash handles over `crate`'s objects,
//! the system RNG through ntdll's kernel entropy source.
use crate::{Algorithm, HashObject, Provider, flags, status};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use door_windows_abi::{narrow, widen};

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorGenRandom(buffer: u64, length: u32) -> u32;
}

struct Lock<T> {
    held: AtomicBool,
    value: core::cell::UnsafeCell<T>,
}
unsafe impl<T: Send> Sync for Lock<T> {}
impl<T> Lock<T> {
    const fn new(value: T) -> Self {
        Self {
            held: AtomicBool::new(false),
            value: core::cell::UnsafeCell::new(value),
        }
    }
    fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self.held.swap(true, Ordering::Acquire) {
            core::hint::spin_loop();
        }
        // SAFETY: `held` gives this closure the only access.
        let r = f(unsafe { &mut *self.value.get() });
        self.held.store(false, Ordering::Release);
        r
    }
}
static PROVIDERS: Lock<Vec<Option<Provider>>> = Lock::new(Vec::new());
static HASHES: Lock<Vec<Option<HashObject>>> = Lock::new(Vec::new());
/// Algorithm and hash handles: tagged indices, clear of the
/// pseudo-handles (below 0x100) and of the other DLLs' handle ranges. A
/// 32-bit process's handles stay below 2 GB (4-byte HANDLEs).
#[cfg(target_pointer_width = "64")]
const PROVIDER_BASE: u64 = 0x4243_0000_0000;
#[cfg(not(target_pointer_width = "64"))]
const PROVIDER_BASE: u64 = 0x5a00_0000;
#[cfg(target_pointer_width = "64")]
const HASH_BASE: u64 = 0x4248_0000_0000;
#[cfg(not(target_pointer_width = "64"))]
const HASH_BASE: u64 = 0x5a80_0000;
const SLOTS: u64 = 0x80_0000;

fn store<T>(list: &Lock<Vec<Option<T>>>, value: T, base: u64) -> u64 {
    list.with(|l| {
        let i = match l.iter().position(Option::is_none) {
            Some(i) => {
                l[i] = Some(value);
                i
            }
            None => {
                l.push(Some(value));
                l.len() - 1
            }
        };
        base + i as u64
    })
}
fn index(handle: u64, base: u64) -> Option<usize> {
    handle
        .checked_sub(base)
        .filter(|i| *i < SLOTS)
        .map(|i| i as usize)
}
/// An algorithm handle: opened, or a pseudo-handle.
fn provider(handle: u64) -> Result<Provider, u32> {
    if let Some(p) = Provider::pseudo(handle) {
        return Ok(p);
    }
    PROVIDERS.with(|p| {
        index(handle, PROVIDER_BASE)
            .and_then(|i| p.get(i).copied().flatten())
            .ok_or(status::INVALID_HANDLE)
    })
}
fn hash<R>(handle: u64, f: impl FnOnce(&mut HashObject) -> R) -> Result<R, u32> {
    HASHES.with(|h| {
        index(handle, HASH_BASE)
            .and_then(|i| h.get_mut(i))
            .and_then(Option::as_mut)
            .map(f)
            .ok_or(status::INVALID_HANDLE)
    })
}
/// A NUL-terminated UTF-16 identifier, bounded.
fn wide(pointer: u64, out: &mut [u16; 64]) -> Result<usize, u32> {
    if pointer == 0 {
        return Err(status::INVALID_PARAMETER);
    }
    for (i, slot) in out.iter_mut().enumerate() {
        // SAFETY: the caller's string.
        let unit = unsafe { (pointer as *const u16).add(i).read_unaligned() };
        if unit == 0 {
            return Ok(i);
        }
        *slot = unit;
    }
    Err(status::INVALID_PARAMETER)
}
/// The caller's input bytes.
fn bytes<'a>(pointer: u64, length: u32) -> Result<&'a [u8], u32> {
    if length == 0 {
        return Ok(&[]);
    }
    if pointer == 0 {
        return Err(status::INVALID_PARAMETER);
    }
    // SAFETY: the caller's buffer of `length` bytes.
    Ok(unsafe { core::slice::from_raw_parts(pointer as *const u8, length as usize) })
}
fn bytes_mut<'a>(pointer: u64, length: u32) -> Result<&'a mut [u8], u32> {
    if pointer == 0 && length != 0 {
        return Err(status::INVALID_PARAMETER);
    }
    if length == 0 {
        return Ok(&mut []);
    }
    // SAFETY: the caller's buffer of `length` bytes.
    Ok(unsafe { core::slice::from_raw_parts_mut(pointer as *mut u8, length as usize) })
}
fn put_handle(pointer: u64, value: u64) -> Result<(), u32> {
    if pointer == 0 {
        return Err(status::INVALID_PARAMETER);
    }
    // SAFETY: the caller's pointer-sized out parameter.
    unsafe { (pointer as *mut usize).write_unaligned(narrow(value)) };
    Ok(())
}
fn done(r: Result<(), u32>) -> u32 {
    r.err().unwrap_or(status::SUCCESS)
}

#[unsafe(no_mangle)]
pub extern "system" fn BCryptOpenAlgorithmProvider(
    out: usize,
    algorithm: usize,
    implementation: usize,
    open_flags: u32,
) -> u32 {
    let (out, algorithm, implementation) = (widen(out), widen(algorithm), widen(implementation));
    done((|| {
        let mut name = [0u16; 64];
        let n = wide(algorithm, &mut name)?;
        if implementation != 0 {
            let mut provider_name = [0u16; 64];
            let m = wide(implementation, &mut provider_name)?;
            let primitive: Vec<u16> = "Microsoft Primitive Provider".encode_utf16().collect();
            if provider_name[..m] != primitive[..] {
                return Err(status::NOT_FOUND);
            }
        }
        let opened = Provider::open(&name[..n], open_flags)?;
        put_handle(out, store(&PROVIDERS, opened, PROVIDER_BASE))
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn BCryptCloseAlgorithmProvider(handle: usize, _flags: u32) -> u32 {
    let handle = widen(handle);
    done(PROVIDERS.with(|p| {
        let slot = index(handle, PROVIDER_BASE)
            .and_then(|i| p.get_mut(i))
            .filter(|s| s.is_some())
            .ok_or(status::INVALID_HANDLE)?;
        *slot = None;
        Ok(())
    }))
}
/// BCryptGetProperty on an algorithm or a hash handle.
#[unsafe(no_mangle)]
pub extern "system" fn BCryptGetProperty(
    handle: usize,
    property: usize,
    out: usize,
    capacity: u32,
    result: usize,
    _flags: u32,
) -> u32 {
    let (handle, property, out, result) =
        (widen(handle), widen(property), widen(out), widen(result));
    done((|| {
        let mut name = [0u16; 64];
        let n = wide(property, &mut name)?;
        let p = match provider(handle) {
            Ok(p) => p,
            Err(_) => hash(handle, |h| h.provider)?,
        };
        let value = crate::property(&p, &name[..n])?;
        if result == 0 {
            return Err(status::INVALID_PARAMETER);
        }
        // SAFETY: the caller's ULONG.
        unsafe { (result as *mut u32).write_unaligned(value.len() as u32) };
        if out == 0 {
            return Ok(());
        }
        if (capacity as usize) < value.len() {
            return Err(status::BUFFER_TOO_SMALL);
        }
        bytes_mut(out, value.len() as u32)?.copy_from_slice(&value);
        Ok(())
    })())
}
/// Properties are fixed here (no chaining modes or key lengths to set).
#[unsafe(no_mangle)]
pub extern "system" fn BCryptSetProperty(
    handle: usize,
    _property: usize,
    _input: usize,
    _bytes: u32,
    _flags: u32,
) -> u32 {
    let handle = widen(handle);
    match provider(handle) {
        Ok(_) => status::NOT_SUPPORTED,
        Err(code) => code,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn BCryptCreateHash(
    algorithm: usize,
    out: usize,
    _object: usize,
    _object_bytes: u32,
    secret: usize,
    secret_bytes: u32,
    create_flags: u32,
) -> u32 {
    let (algorithm, out, secret) = (widen(algorithm), widen(out), widen(secret));
    done((|| {
        if create_flags & !flags::HASH_REUSABLE != 0 {
            return Err(status::INVALID_PARAMETER);
        }
        let p = provider(algorithm)?;
        if !p.hmac && secret_bytes != 0 {
            return Err(status::INVALID_PARAMETER);
        }
        let object = HashObject::new(
            p,
            bytes(secret, secret_bytes)?,
            create_flags & flags::HASH_REUSABLE != 0,
        )?;
        put_handle(out, store(&HASHES, object, HASH_BASE))
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn BCryptHashData(
    handle: usize,
    input: usize,
    length: u32,
    _flags: u32,
) -> u32 {
    let (handle, input) = (widen(handle), widen(input));
    done((|| {
        let data = bytes(input, length)?;
        hash(handle, |h| h.update(data))?
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn BCryptFinishHash(
    handle: usize,
    out: usize,
    length: u32,
    _flags: u32,
) -> u32 {
    let (handle, out) = (widen(handle), widen(out));
    done((|| {
        let target = bytes_mut(out, length)?;
        hash(handle, |h| h.finish(target))?
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn BCryptDuplicateHash(
    handle: usize,
    out: usize,
    _object: usize,
    _object_bytes: u32,
    _flags: u32,
) -> u32 {
    let (handle, out) = (widen(handle), widen(out));
    done((|| {
        let copy = hash(handle, |h| h.clone())?;
        put_handle(out, store(&HASHES, copy, HASH_BASE))
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn BCryptDestroyHash(handle: usize) -> u32 {
    let handle = widen(handle);
    done(HASHES.with(|h| {
        let slot = index(handle, HASH_BASE)
            .and_then(|i| h.get_mut(i))
            .filter(|s| s.is_some())
            .ok_or(status::INVALID_HANDLE)?;
        *slot = None;
        Ok(())
    }))
}
/// BCryptHash (Windows 10): create, hash and finish in one call.
#[unsafe(no_mangle)]
pub extern "system" fn BCryptHash(
    algorithm: usize,
    secret: usize,
    secret_bytes: u32,
    input: usize,
    input_bytes: u32,
    out: usize,
    out_bytes: u32,
) -> u32 {
    let (algorithm, secret, input, out) =
        (widen(algorithm), widen(secret), widen(input), widen(out));
    done((|| {
        let p = provider(algorithm)?;
        if !p.hmac && secret_bytes != 0 {
            return Err(status::INVALID_PARAMETER);
        }
        crate::hash_once(
            p,
            bytes(secret, secret_bytes)?,
            bytes(input, input_bytes)?,
            bytes_mut(out, out_bytes)?,
        )
    })())
}
/// BCryptGenRandom: the kernel entropy source, through an RNG handle or
/// BCRYPT_USE_SYSTEM_PREFERRED_RNG with none.
#[unsafe(no_mangle)]
pub extern "system" fn BCryptGenRandom(
    algorithm: usize,
    out: usize,
    length: u32,
    rng_flags: u32,
) -> u32 {
    let (algorithm, out) = (widen(algorithm), widen(out));
    done((|| {
        if rng_flags & flags::USE_SYSTEM_PREFERRED_RNG != 0 {
            if algorithm != 0 {
                return Err(status::INVALID_PARAMETER);
            }
        } else if provider(algorithm)?.algorithm != Algorithm::Rng {
            return Err(status::INVALID_HANDLE);
        }
        if length == 0 {
            return Ok(());
        }
        if out == 0 {
            return Err(status::INVALID_PARAMETER);
        }
        match unsafe { DoorGenRandom(out, length) } {
            0 => Ok(()),
            _ => Err(status::NO_MEMORY),
        }
    })())
}
