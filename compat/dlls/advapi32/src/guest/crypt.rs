//! CryptoAPI: provider contexts, key containers with their user keys,
//! hashes (and HMAC and CBC-MAC), session keys (RC2, RC4, DES, triple DES,
//! AES) and RSA keys: generation, derivation, key blobs, encryption and
//! signatures, as Microsoft's RSA providers do them (`crate::crypt`).
use super::*;
use crate::crypt::{
    self, Hash, flags,
    keys::{self, Asymmetric, Key, Symmetric, calg},
    nte, rsa,
};

struct Provider {
    kind: u32,
    name: &'static str,
    container: String,
    references: u32,
    /// The container's file (None: CRYPT_VERIFYCONTEXT, keys in memory).
    path: Option<String>,
    /// The user keys (AT_KEYEXCHANGE, AT_SIGNATURE).
    exchange: Option<Asymmetric>,
    signature: Option<Asymmetric>,
    /// PP_ENUMALGS(_EX)'s position.
    enumerated: usize,
}
/// A hash and the context that made it (CryptSignHash signs with that
/// context's key).
struct HashObject {
    hash: Hash,
    context: u64,
    kind: u32,
}
static PROVIDERS: Lock<Vec<Option<Provider>>> = Lock::new(Vec::new());
static HASHES: Lock<Vec<Option<HashObject>>> = Lock::new(Vec::new());
/// Context, hash and key handles: tagged indices, never 0.
#[cfg(target_pointer_width = "64")]
const PROVIDER_BASE: u64 = 0x5052_0000_0000;
/// A 32-bit process's handle values stay below 2 GB (P17: its HANDLEs are
/// 4 bytes, sign-extended), clear of the kernel's and ntdll's.
#[cfg(not(target_pointer_width = "64"))]
const PROVIDER_BASE: u64 = 0x5800_0000;
#[cfg(target_pointer_width = "64")]
const HASH_BASE: u64 = 0x4841_0000_0000;
/// A 32-bit process's handle values stay below 2 GB (P17: its HANDLEs are
/// 4 bytes, sign-extended), clear of the kernel's and ntdll's.
#[cfg(not(target_pointer_width = "64"))]
const HASH_BASE: u64 = 0x5900_0000;
const WRITE_FAULT: u32 = 29;
fn index(handle: u64, base: u64) -> Option<usize> {
    handle.checked_sub(base).map(|i| i as usize)
}
fn provider<R>(handle: u64, f: impl FnOnce(&mut Provider) -> R) -> Result<R, u32> {
    PROVIDERS.with(|p| {
        index(handle, PROVIDER_BASE)
            .and_then(|i| p.get_mut(i))
            .and_then(Option::as_mut)
            .map(f)
            .ok_or(error::INVALID_PARAMETER)
    })
}
fn hash_object<R>(
    handle: u64,
    f: impl FnOnce(&mut HashObject) -> Result<R, u32>,
) -> Result<R, u32> {
    HASHES.with(|h| {
        index(handle, HASH_BASE)
            .and_then(|i| h.get_mut(i))
            .and_then(Option::as_mut)
            .ok_or(error::INVALID_PARAMETER)
            .and_then(f)
    })
}
fn hash<R>(handle: u64, f: impl FnOnce(&mut Hash) -> Result<R, u32>) -> Result<R, u32> {
    hash_object(handle, |o| f(&mut o.hash))
}
fn hash_kind(handle: u64) -> Result<u32, u32> {
    hash_object(handle, |o| Ok(o.kind))
}
fn hash_context(handle: u64) -> Result<u64, u32> {
    hash_object(handle, |o| Ok(o.context))
}
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
fn nte(code: u32) -> i32 {
    last_error(code)
}

// ---- key containers -------------------------------------------------------------------
/// A container's file: %APPDATA%\Microsoft\Crypto\RSA\<user SID>\<half the
/// MD5 of the lower-case name> (machine containers in
/// %ProgramData%\Microsoft\Crypto\RSA\MachineKeys), as Windows keeps them.
fn container_file(name: &str, machine: bool) -> Result<(Vec<String>, String), u32> {
    let base = if machine {
        let mut out = [0u16; 128];
        let key: Vec<u16> = "ProgramData".encode_utf16().chain([0]).collect();
        let n = unsafe {
            DoorGetEnvironmentVariableW(key.as_ptr() as u64, out.as_mut_ptr() as u64, 128)
        } as usize;
        if n == 0 || n >= 128 {
            return Err(nte::BAD_KEYSET);
        }
        String::from_utf16_lossy(&out[..n])
    } else {
        let mut out = [0u16; 128];
        let key: Vec<u16> = "APPDATA".encode_utf16().chain([0]).collect();
        let n = unsafe {
            DoorGetEnvironmentVariableW(key.as_ptr() as u64, out.as_mut_ptr() as u64, 128)
        } as usize;
        if n == 0 || n >= 128 {
            return Err(nte::BAD_KEYSET);
        }
        String::from_utf16_lossy(&out[..n])
    };
    let leaf = if machine {
        String::from("MachineKeys")
    } else {
        Sid::user(identity().uid).text()
    };
    let folders = alloc::vec![
        alloc::format!("{base}\\Microsoft"),
        alloc::format!("{base}\\Microsoft\\Crypto"),
        alloc::format!("{base}\\Microsoft\\Crypto\\RSA"),
        alloc::format!("{base}\\Microsoft\\Crypto\\RSA\\{leaf}"),
    ];
    let mut digest =
        Hash::new(crypt::PROV_RSA_FULL, crypt::alg::MD5).map_err(|_| nte::BAD_KEYSET)?;
    let _ = digest.update(name.to_lowercase().as_bytes());
    // Half the MD5 (Windows names the file by the whole hash and the
    // machine's GUID): Doors' file paths hold 95 UTF-16 units, and the
    // profile's RSA folder already takes 78 of them.
    let file: String = digest.value()[..8]
        .iter()
        .map(|b| alloc::format!("{b:02x}"))
        .collect();
    let path = alloc::format!("{}\\{file}", folders[3]);
    Ok((folders, path))
}
fn wide0(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}
fn exists(path: &str) -> bool {
    let attributes = unsafe { DoorGetFileAttributesW(wide0(path).as_ptr() as u64) };
    attributes != u32::MAX
}
/// The container's folders and file; a failure is the file system's error.
fn create(folders: &[String], path: &str) -> Result<(), u32> {
    for f in folders {
        // ERROR_ALREADY_EXISTS is expected for all but new folders.
        if unsafe { DoorCreateDirectoryW(wide0(f).as_ptr() as u64) } == 0 {
            let e = unsafe { RtlGetLastWin32Error() };
            if e != 183 {
                return Err(e);
            }
        }
    }
    // CREATE_NEW, GENERIC_WRITE.
    let h = unsafe { DoorCreateFileW(wide0(path).as_ptr() as u64, 0x4000_0000, 0, 0, 1, 0x80, 0) };
    if h == u64::MAX || h == 0 {
        return Err(unsafe { RtlGetLastWin32Error() }.max(1));
    }
    unsafe { DoorCloseFile(h) };
    Ok(())
}
fn acquire(out: u64, container: u64, name: u64, kind: u32, mode: u32, wide: bool) -> i32 {
    let r = (|| {
        let known = flags::VERIFYCONTEXT
            | flags::NEWKEYSET
            | flags::DELETEKEYSET
            | flags::MACHINE_KEYSET
            | flags::SILENT;
        if mode & !known != 0 {
            return Err(nte::BAD_FLAGS);
        }
        let provider_name = if name == 0 {
            None
        } else {
            Some(String::from_utf16_lossy(&text(name, wide)?))
        };
        let chosen = crypt::provider(provider_name.as_deref(), kind)?;
        let container = if container == 0 {
            user_name()
        } else {
            String::from_utf16_lossy(&text(container, wide)?)
        };
        let machine = mode & flags::MACHINE_KEYSET != 0;
        let mut path_kept = None;
        let mut user_keys = (None, None);
        if mode & flags::VERIFYCONTEXT == 0 {
            let (folders, path) = container_file(&container, machine)?;
            if mode & flags::DELETEKEYSET != 0 {
                if !exists(&path) {
                    return Err(nte::BAD_KEYSET);
                }
                unsafe { DoorDeleteFileW(wide0(&path).as_ptr() as u64) };
                // The handle is not set.
                return Ok(());
            }
            if mode & flags::NEWKEYSET != 0 {
                if exists(&path) {
                    return Err(nte::EXISTS);
                }
                create(&folders, &path)?;
            } else if !exists(&path) {
                return Err(nte::BAD_KEYSET);
            } else {
                user_keys = load_container(&path);
            }
            path_kept = Some(path);
        }
        let handle = store(
            &PROVIDERS,
            Provider {
                kind,
                name: chosen,
                container,
                references: 1,
                path: path_kept,
                exchange: user_keys.0,
                signature: user_keys.1,
                enumerated: 0,
            },
            PROVIDER_BASE,
        );
        write_pointer(out, handle)
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptAcquireContextW(
    out: usize,
    container: usize,
    name: usize,
    kind: u32,
    mode: u32,
) -> i32 {
    let out = door_windows_abi::widen(out);
    let container = door_windows_abi::widen(container);
    let name = door_windows_abi::widen(name);
    acquire(out, container, name, kind, mode, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptAcquireContextA(
    out: usize,
    container: usize,
    name: usize,
    kind: u32,
    mode: u32,
) -> i32 {
    let out = door_windows_abi::widen(out);
    let container = door_windows_abi::widen(container);
    let name = door_windows_abi::widen(name);
    acquire(out, container, name, kind, mode, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptReleaseContext(handle: usize, mode: u32) -> i32 {
    let handle = door_windows_abi::widen(handle);
    {
        if mode != 0 {
            return nte(nte::BAD_FLAGS);
        }
        let r = PROVIDERS.with(|p| {
            let slot = index(handle, PROVIDER_BASE)
                .and_then(|i| p.get_mut(i))
                .filter(|s| s.is_some())
                .ok_or(error::INVALID_PARAMETER)?;
            let last = slot.as_mut().is_some_and(|v| {
                v.references -= 1;
                v.references == 0
            });
            if last {
                *slot = None;
            }
            Ok(())
        });
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptContextAddRef(handle: usize, reserved: usize, mode: u32) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let reserved = door_windows_abi::widen(reserved);
    {
        if reserved != 0 || mode != 0 {
            return last_error(error::INVALID_PARAMETER);
        }
        let r = provider(handle, |p| p.references += 1);
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptGenRandom(handle: usize, length: u32, buffer: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let buffer = door_windows_abi::widen(buffer);
    {
        let r = provider(handle, |_| ()).and_then(|()| {
            if unsafe { DoorGenRandom(buffer, length) } == 0 {
                Ok(())
            } else {
                Err(nte::BAD_KEYSET)
            }
        });
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
/// CryptGetProvParam: PP_ENUMALGS (1) and PP_ENUMALGS_EX (22) one
/// algorithm per call (CRYPT_FIRST starts over; ERROR_NO_MORE_ITEMS at the
/// end), PP_NAME (4), PP_CONTAINER (6) and PP_UNIQUE_CONTAINER (36) as ANSI
/// strings, PP_IMPTYPE (3, software), PP_VERSION (5, 2.0), PP_PROVTYPE (16).
#[unsafe(no_mangle)]
pub extern "system" fn CryptGetProvParam(
    handle: usize,
    param: u32,
    out: usize,
    length: usize,
    mode: u32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    {
        let r = (|| {
            let p = kinds(handle)?;
            let (kind, name, container) =
                provider(handle, |p| (p.kind, p.name, p.container.clone()))?;
            let value: Vec<u8> = match param {
                1 | 22 => {
                    // CRYPT_FIRST (1).
                    let at = provider(handle, |c| {
                        if mode & 1 != 0 {
                            c.enumerated = 0;
                        }
                        c.enumerated
                    })?;
                    let a = keys::algorithms(p).get(at).ok_or(error::NO_MORE_ITEMS)?;
                    let record = if param == 1 {
                        keys::enumalgs(a).to_vec()
                    } else {
                        keys::enumalgs_ex(a).to_vec()
                    };
                    // A length query leaves the position.
                    if out != 0 && read_u32(length)? as usize >= record.len() {
                        provider(handle, |c| c.enumerated = at + 1)?;
                    }
                    record
                }
                4 => name.bytes().chain([0]).collect(),
                6 | 36 => container.bytes().chain([0]).collect(),
                3 => 2u32.to_le_bytes().to_vec(),
                5 => 0x200u32.to_le_bytes().to_vec(),
                16 => kind.to_le_bytes().to_vec(),
                _ => return Err(nte::BAD_TYPE),
            };
            give(out, length, &value)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptCreateHash(
    handle: usize,
    algorithm: u32,
    key: usize,
    mode: u32,
    out: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let key = door_windows_abi::widen(key);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let kind = provider(handle, |p| p.kind)?;
            if mode != 0 {
                return Err(nte::BAD_FLAGS);
            }
            // HMAC and CBC-MAC take a session key; other hashes none.
            let h = match (algorithm, key) {
                (crypt::alg::HMAC, k) if k != 0 => key_symmetric(k, |s| Ok(Hash::hmac(&s.key)))?,
                (crypt::alg::MAC, k) if k != 0 => key_symmetric(k, Hash::mac)?,
                (_, 0) => Hash::new(kind, algorithm)?,
                _ => return Err(nte::BAD_KEY),
            };
            let object = HashObject {
                hash: h,
                context: handle,
                kind,
            };
            write_pointer(out, store(&HASHES, object, HASH_BASE))
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptHashData(handle: usize, data: usize, length: u32, _mode: u32) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let data = door_windows_abi::widen(data);
    {
        let r = (|| {
            let mut chunk = vec![0u8; (length as usize).min(64 * 1024)];
            let mut done = 0u64;
            while done < u64::from(length) {
                let n = (u64::from(length) - done).min(chunk.len() as u64) as usize;
                read(data + done, &mut chunk[..n])?;
                hash(handle, |h| h.update(&chunk[..n]))?;
                done += n as u64;
            }
            if length == 0 {
                hash(handle, |h| h.update(&[]))?;
            }
            Ok(())
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
/// CryptGetHashParam: HP_ALGID (1), HP_HASHVAL (2, which finishes the
/// hash), HP_HASHSIZE (4).
#[unsafe(no_mangle)]
pub extern "system" fn CryptGetHashParam(
    handle: usize,
    param: u32,
    out: usize,
    length: usize,
    _mode: u32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    {
        let r = (|| {
            let value: Vec<u8> = hash(handle, |h| match param {
                1 => Ok(h.algorithm.to_le_bytes().to_vec()),
                2 if out == 0 => Ok(vec![0; h.size()]),
                2 => h.try_value(),
                4 => Ok((h.size() as u32).to_le_bytes().to_vec()),
                _ => Err(nte::BAD_TYPE),
            })?;
            give(out, length, &value)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptDestroyHash(handle: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    {
        let r = HASHES.with(|h| {
            let slot = index(handle, HASH_BASE)
                .and_then(|i| h.get_mut(i))
                .filter(|s| s.is_some())
                .ok_or(error::INVALID_PARAMETER)?;
            *slot = None;
            Ok(())
        });
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptDuplicateHash(
    handle: usize,
    reserved: usize,
    mode: u32,
    out: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let reserved = door_windows_abi::widen(reserved);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            if reserved != 0 || mode != 0 {
                return Err(error::INVALID_PARAMETER);
            }
            let copy = hash_object(handle, |o| {
                Ok(HashObject {
                    hash: o.hash.clone(),
                    context: o.context,
                    kind: o.kind,
                })
            })?;
            write_pointer(out, store(&HASHES, copy, HASH_BASE))
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
// ---- keys -------------------------------------------------------------------------------
struct KeyObject {
    key: Key,
    provider: keys::Provider,
}
static KEYS: Lock<Vec<Option<KeyObject>>> = Lock::new(Vec::new());
#[cfg(target_pointer_width = "64")]
const KEY_BASE: u64 = 0x4B45_0000_0000;
/// A 32-bit process's handle values stay below 2 GB (P17: its HANDLEs are
/// 4 bytes, sign-extended), clear of the kernel's and ntdll's.
#[cfg(not(target_pointer_width = "64"))]
const KEY_BASE: u64 = 0x5A00_0000;
fn key<R>(handle: u64, f: impl FnOnce(&mut KeyObject) -> Result<R, u32>) -> Result<R, u32> {
    KEYS.with(|k| {
        index(handle, KEY_BASE)
            .and_then(|i| k.get_mut(i))
            .and_then(Option::as_mut)
            .ok_or(error::INVALID_PARAMETER)
            .and_then(f)
    })
}
/// `f` with the session key `handle` names (an RSA key: NTE_BAD_KEY).
fn key_symmetric<R>(handle: u64, f: impl FnOnce(&Symmetric) -> Result<R, u32>) -> Result<R, u32> {
    key(handle, |o| match &o.key {
        Key::Symmetric(s) => f(s),
        Key::Asymmetric(_) => Err(nte::BAD_KEY),
    })
}
fn new_key(key: Key, provider: keys::Provider) -> u64 {
    store(&KEYS, KeyObject { key, provider }, KEY_BASE)
}
fn random(buffer: &mut [u8]) {
    if !buffer.is_empty() {
        unsafe { DoorGenRandom(buffer.as_mut_ptr() as u64, buffer.len() as u32) };
    }
}
/// What a context's provider is (Base, AES) for the key layer.
fn kinds(handle: u64) -> Result<keys::Provider, u32> {
    provider(handle, |p| keys::Provider {
        aes: p.kind == crypt::PROV_RSA_AES,
        base: p.name == crypt::BASE_PROVIDER,
    })
}
/// The value-and-length convention of the Crypt*Param and export calls:
/// `length` holds the buffer's size and receives the value's; a null
/// buffer asks for the length only; a short one is ERROR_MORE_DATA.
fn give(out: u64, length: u64, value: &[u8]) -> Result<(), u32> {
    let have = read_u32(length)?;
    write_u32(length, value.len() as u32)?;
    if out == 0 {
        return Ok(());
    }
    if (have as usize) < value.len() {
        return Err(error::MORE_DATA);
    }
    write(out, value)
}

// Containers keep their user keys: `DOORCSP1`, then per key [spec u8]
// [exportable u8][length u32][PRIVATEKEYBLOB body].
const CONTAINER_MAGIC: &[u8; 8] = b"DOORCSP1";
fn load_container(path: &str) -> (Option<Asymmetric>, Option<Asymmetric>) {
    let mut found = (None, None);
    // GENERIC_READ, FILE_SHARE_READ, OPEN_EXISTING.
    let h = unsafe { DoorCreateFileW(wide0(path).as_ptr() as u64, 0x8000_0000, 1, 0, 3, 0x80, 0) };
    if h == u64::MAX || h == 0 {
        return found;
    }
    let mut data = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let mut got = 0u32;
        let ok = unsafe {
            DoorReadFile(
                h,
                chunk.as_mut_ptr() as u64,
                chunk.len() as u32,
                &mut got as *mut u32 as u64,
                0,
            )
        };
        if ok == 0 || got == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..got as usize]);
    }
    unsafe { DoorCloseFile(h) };
    if data.get(..8) != Some(CONTAINER_MAGIC) {
        return found;
    }
    let mut at = 8;
    while at + 6 <= data.len() {
        let (spec, exportable) = (data[at], data[at + 1] != 0);
        let len = u32::from_le_bytes(data[at + 2..at + 6].try_into().unwrap()) as usize;
        let Some(body) = data.get(at + 6..at + 6 + len) else {
            break;
        };
        if let Some(private) = rsa::PrivateKey::from_blob(body) {
            let key = Asymmetric {
                alg: if u32::from(spec) == calg::AT_SIGNATURE {
                    calg::RSA_SIGN
                } else {
                    calg::RSA_KEYX
                },
                public: private.public.clone(),
                private: Some(alloc::boxed::Box::new(private)),
                exportable,
            };
            if u32::from(spec) == calg::AT_SIGNATURE {
                found.1 = Some(key);
            } else {
                found.0 = Some(key);
            }
        }
        at += 6 + len;
    }
    found
}
fn save_container(
    path: &str,
    exchange: &Option<Asymmetric>,
    signature: &Option<Asymmetric>,
) -> Result<(), u32> {
    let mut data = CONTAINER_MAGIC.to_vec();
    for (spec, key) in [
        (calg::AT_KEYEXCHANGE, exchange),
        (calg::AT_SIGNATURE, signature),
    ] {
        if let Some(k) = key
            && let Some(private) = &k.private
        {
            let body = private.blob();
            data.push(spec as u8);
            data.push(u8::from(k.exportable));
            data.extend_from_slice(&(body.len() as u32).to_le_bytes());
            data.extend_from_slice(&body);
        }
    }
    // GENERIC_WRITE, CREATE_ALWAYS.
    let h = unsafe { DoorCreateFileW(wide0(path).as_ptr() as u64, 0x4000_0000, 0, 0, 2, 0x80, 0) };
    if h == u64::MAX || h == 0 {
        return Err(unsafe { RtlGetLastWin32Error() }.max(1));
    }
    let mut written = 0u32;
    let ok = unsafe {
        DoorWriteFile(
            h,
            data.as_ptr() as u64,
            data.len() as u32,
            &mut written as *mut u32 as u64,
            0,
        )
    };
    unsafe { DoorCloseFile(h) };
    if ok == 0 || written as usize != data.len() {
        return Err(WRITE_FAULT);
    }
    Ok(())
}
/// Makes `key` the context's user key of its kind (persisted in a named
/// container).
fn set_user_key(handle: u64, key: &Asymmetric) -> Result<(), u32> {
    let (path, exchange, signature) = provider(handle, |p| {
        if key.alg == calg::RSA_SIGN {
            p.signature = Some(key.clone());
        } else {
            p.exchange = Some(key.clone());
        }
        (p.path.clone(), p.exchange.clone(), p.signature.clone())
    })?;
    match path {
        Some(path) => save_container(&path, &exchange, &signature),
        None => Ok(()),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn CryptGenKey(handle: usize, algorithm: u32, mode: u32, out: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let p = kinds(handle)?;
            let algorithm = match algorithm {
                calg::AT_KEYEXCHANGE => calg::RSA_KEYX,
                calg::AT_SIGNATURE => calg::RSA_SIGN,
                a => a,
            };
            let exportable = mode & keys::flag::EXPORTABLE != 0;
            let bits = keys::key_bits(p, algorithm, mode)?;
            let key = if matches!(algorithm, calg::RSA_KEYX | calg::RSA_SIGN) {
                // Doors makes keys of at most 4096 bits.
                if bits as usize > rsa::MAX_BITS || !bits.is_multiple_of(16) {
                    return Err(nte::BAD_FLAGS);
                }
                let private =
                    rsa::PrivateKey::generate(bits as usize, &mut random).ok_or(nte::BAD_FLAGS)?;
                let pair = Asymmetric {
                    alg: algorithm,
                    public: private.public.clone(),
                    private: Some(alloc::boxed::Box::new(private)),
                    exportable,
                };
                set_user_key(handle, &pair)?;
                Key::Asymmetric(alloc::boxed::Box::new(pair))
            } else {
                let mut bytes = alloc::vec![0u8; keys::key_bytes(algorithm, bits)];
                random(&mut bytes);
                let mut k = Symmetric::new(
                    p,
                    algorithm,
                    &bytes,
                    mode & keys::flag::NO_SALT != 0,
                    exportable,
                );
                if mode & keys::flag::CREATE_SALT != 0 && !k.salt.is_empty() {
                    random(&mut k.salt);
                    k.restart();
                }
                Key::Symmetric(k)
            };
            write_pointer(out, new_key(key, p))
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptDeriveKey(
    handle: usize,
    algorithm: u32,
    base: usize,
    mode: u32,
    out: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let base = door_windows_abi::widen(base);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let p = kinds(handle)?;
            if matches!(algorithm, calg::RSA_KEYX | calg::RSA_SIGN) {
                return Err(nte::BAD_ALGID);
            }
            let bits = keys::key_bits(p, algorithm, mode)?;
            let (hash_algorithm, value) = hash(base, |h| Ok((h.algorithm, h.try_value()?)))?;
            let bytes = keys::derived_bytes(
                hash_algorithm,
                &value,
                algorithm,
                keys::key_bytes(algorithm, bits),
            );
            let mut k = Symmetric::new(
                p,
                algorithm,
                &bytes,
                mode & keys::flag::NO_SALT != 0,
                mode & keys::flag::EXPORTABLE != 0,
            );
            if mode & keys::flag::CREATE_SALT != 0 && !k.salt.is_empty() {
                random(&mut k.salt);
                k.restart();
            }
            write_pointer(out, new_key(Key::Symmetric(k), p))
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
/// BLOBHEADER: bType, bVersion, reserved, aiKeyAlg.
const BLOB_HEADER: usize = 8;
fn blob_header(kind: u8, algorithm: u32) -> Vec<u8> {
    let mut h = alloc::vec![kind, 2, 0, 0];
    h.extend_from_slice(&algorithm.to_le_bytes());
    h
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptImportKey(
    handle: usize,
    data: usize,
    length: u32,
    public: usize,
    mode: u32,
    out: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let data = door_windows_abi::widen(data);
    let public = door_windows_abi::widen(public);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let p = kinds(handle)?;
            if (length as usize) < BLOB_HEADER || length > 64 * 1024 {
                return Err(nte::BAD_DATA);
            }
            let mut blob = alloc::vec![0u8; length as usize];
            read(data, &mut blob)?;
            let kind = blob[0];
            if blob[1] != 2 && blob[1] != 3 {
                return Err(nte::BAD_VER);
            }
            let algorithm = u32::from_le_bytes(blob[4..8].try_into().unwrap());
            let body = &blob[BLOB_HEADER..];
            let exportable = mode & keys::flag::EXPORTABLE != 0;
            let key = match kind {
                keys::blob::PLAINTEXT => {
                    Key::Symmetric(keys::import_plaintext(p, algorithm, body, mode)?)
                }
                keys::blob::SIMPLE => {
                    let exchange = key(public, |o| match &o.key {
                        Key::Asymmetric(a) => Ok(Asymmetric::clone(a)),
                        Key::Symmetric(_) => Err(nte::BAD_KEY),
                    })?;
                    Key::Symmetric(keys::import_simple(p, algorithm, body, &exchange, mode)?)
                }
                keys::blob::PUBLIC => {
                    let public = rsa::PublicKey::from_blob(body).ok_or(nte::BAD_DATA)?;
                    if !matches!(algorithm, calg::RSA_KEYX | calg::RSA_SIGN) {
                        return Err(nte::BAD_ALGID);
                    }
                    Key::Asymmetric(alloc::boxed::Box::new(Asymmetric {
                        alg: algorithm,
                        public,
                        private: None,
                        exportable: true,
                    }))
                }
                keys::blob::PRIVATE => {
                    let private = rsa::PrivateKey::from_blob(body).ok_or(nte::BAD_DATA)?;
                    if !matches!(algorithm, calg::RSA_KEYX | calg::RSA_SIGN) {
                        return Err(nte::BAD_ALGID);
                    }
                    let pair = Asymmetric {
                        alg: algorithm,
                        public: private.public.clone(),
                        private: Some(alloc::boxed::Box::new(private)),
                        exportable,
                    };
                    // An imported key pair is the context's key of its kind.
                    set_user_key(handle, &pair)?;
                    Key::Asymmetric(alloc::boxed::Box::new(pair))
                }
                _ => return Err(nte::BAD_TYPE),
            };
            write_pointer(out, new_key(key, p))
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptExportKey(
    handle: usize,
    exchange: usize,
    kind: u32,
    _mode: u32,
    out: usize,
    length: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let exchange = door_windows_abi::widen(exchange);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    {
        let r = (|| {
            let k = key(handle, |o| Ok(o.key.clone()))?;
            let blob = match (kind as u8, &k) {
                (keys::blob::PLAINTEXT, Key::Symmetric(s)) => {
                    if !s.exportable {
                        return Err(nte::BAD_KEY_STATE);
                    }
                    let mut b = blob_header(keys::blob::PLAINTEXT, s.alg);
                    b.extend(s.plaintext());
                    b
                }
                (keys::blob::SIMPLE, Key::Symmetric(s)) => {
                    if !s.exportable {
                        return Err(nte::BAD_KEY_STATE);
                    }
                    let to = key(exchange, |o| match &o.key {
                        Key::Asymmetric(a) => Ok(Asymmetric::clone(a)),
                        Key::Symmetric(_) => Err(nte::BAD_KEY),
                    })?;
                    let mut encrypted = s.key.clone();
                    keys::rsa_encrypt(&to, &mut encrypted, false, &mut random)?;
                    let mut b = blob_header(keys::blob::SIMPLE, s.alg);
                    b.extend_from_slice(&to.alg.to_le_bytes());
                    b.extend(encrypted);
                    b
                }
                (keys::blob::PUBLIC, Key::Asymmetric(a)) => {
                    let mut b = blob_header(keys::blob::PUBLIC, a.alg);
                    b.extend(a.public.blob());
                    b
                }
                (keys::blob::PRIVATE, Key::Asymmetric(a)) => {
                    let private = a
                        .private
                        .as_ref()
                        .filter(|_| a.exportable)
                        .ok_or(nte::BAD_KEY_STATE)?;
                    let mut b = blob_header(keys::blob::PRIVATE, a.alg);
                    b.extend(private.blob());
                    b
                }
                (
                    keys::blob::PLAINTEXT
                    | keys::blob::SIMPLE
                    | keys::blob::PUBLIC
                    | keys::blob::PRIVATE,
                    _,
                ) => {
                    return Err(nte::BAD_KEY_STATE);
                }
                _ => return Err(nte::BAD_TYPE),
            };
            give(out, length, &blob)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn CryptEncrypt(
    handle: usize,
    with_hash: usize,
    last: i32,
    mode: u32,
    data: usize,
    length: usize,
    size: u32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let with_hash = door_windows_abi::widen(with_hash);
    let data = door_windows_abi::widen(data);
    let length = door_windows_abi::widen(length);
    {
        let r = (|| {
            let len = read_u32(length)? as usize;
            let needed = key(handle, |o| {
                Ok(match &o.key {
                    Key::Symmetric(s) => s.encrypted_len(len, last != 0),
                    Key::Asymmetric(a) => a.public.bytes(),
                })
            })?;
            if data == 0 {
                return write_u32(length, needed as u32);
            }
            if (size as usize) < needed {
                write_u32(length, needed as u32)?;
                return Err(error::MORE_DATA);
            }
            let mut buffer = alloc::vec![0u8; len];
            read(data, &mut buffer)?;
            if with_hash != 0 {
                hash(with_hash, |h| h.update(&buffer))?;
            }
            key(handle, |o| match &mut o.key {
                Key::Symmetric(s) => s.encrypt(&mut buffer, last != 0),
                Key::Asymmetric(a) => {
                    keys::rsa_encrypt(a, &mut buffer, mode & keys::flag::OAEP != 0, &mut random)
                }
            })?;
            write(data, &buffer)?;
            write_u32(length, buffer.len() as u32)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptDecrypt(
    handle: usize,
    with_hash: usize,
    last: i32,
    mode: u32,
    data: usize,
    length: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let with_hash = door_windows_abi::widen(with_hash);
    let data = door_windows_abi::widen(data);
    let length = door_windows_abi::widen(length);
    {
        let r = (|| {
            let len = read_u32(length)? as usize;
            let mut buffer = alloc::vec![0u8; len];
            read(data, &mut buffer)?;
            key(handle, |o| match &mut o.key {
                Key::Symmetric(s) => s.decrypt(&mut buffer, last != 0),
                Key::Asymmetric(a) => {
                    keys::rsa_decrypt(a, &mut buffer, mode & keys::flag::OAEP != 0)
                }
            })?;
            if with_hash != 0 {
                hash(with_hash, |h| h.update(&buffer))?;
            }
            write(data, &buffer)?;
            write_u32(length, buffer.len() as u32)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptDestroyKey(handle: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    {
        let r = KEYS.with(|k| {
            let slot = index(handle, KEY_BASE)
                .and_then(|i| k.get_mut(i))
                .filter(|s| s.is_some())
                .ok_or(error::INVALID_PARAMETER)?;
            *slot = None;
            Ok(())
        });
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptDuplicateKey(
    handle: usize,
    reserved: usize,
    mode: u32,
    out: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let reserved = door_windows_abi::widen(reserved);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            if reserved != 0 || mode != 0 {
                return Err(error::INVALID_PARAMETER);
            }
            let (k, p) = key(handle, |o| Ok((o.key.clone(), o.provider)))?;
            write_pointer(out, new_key(k, p))
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptGetKeyParam(
    handle: usize,
    param: u32,
    out: usize,
    length: usize,
    _mode: u32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    {
        let r = (|| {
            let value = key(handle, |o| o.key.bytes(param))?;
            give(out, length, &value)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptSetKeyParam(handle: usize, param: u32, data: usize, _mode: u32) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let data = door_windows_abi::widen(data);
    {
        let mut b = [0u8; 16];
        let word = |b: &mut [u8; 16]| -> Result<u32, u32> {
            read(data, &mut b[..4])?;
            Ok(u32::from_le_bytes(b[..4].try_into().unwrap()))
        };
        let r = key(handle, |o| {
            let Key::Symmetric(s) = &mut o.key else {
                return Err(nte::BAD_TYPE);
            };
            match param {
                keys::kp::IV => {
                    let n = s.block();
                    read(data, &mut b[..n])?;
                    s.set_iv(&b[..n])
                }
                keys::kp::SALT => s.set_salt(&[]),
                keys::kp::MODE => s.set_mode(word(&mut b)?),
                keys::kp::EFFECTIVE_KEYLEN => s.set_effective(word(&mut b)?),
                keys::kp::PADDING => match word(&mut b)? {
                    1 => Ok(()),
                    _ => Err(nte::BAD_DATA),
                },
                keys::kp::MODE_BITS => match word(&mut b)? {
                    8 => Ok(()),
                    _ => Err(nte::BAD_DATA),
                },
                keys::kp::PERMISSIONS => {
                    let wanted = word(&mut b)?;
                    // Exporting can be withdrawn, never granted.
                    if wanted & 0x4 != 0 && !s.exportable {
                        return Err(nte::BAD_FLAGS);
                    }
                    s.exportable = wanted & 0x4 != 0;
                    Ok(())
                }
                _ => Err(nte::BAD_TYPE),
            }
        });
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptGetUserKey(handle: usize, spec: u32, out: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let p = kinds(handle)?;
            let found = provider(handle, |c| match spec {
                calg::AT_KEYEXCHANGE => c.exchange.clone(),
                calg::AT_SIGNATURE => c.signature.clone(),
                _ => None,
            })?;
            let pair = found.ok_or(nte::NO_KEY)?;
            write_pointer(
                out,
                new_key(Key::Asymmetric(alloc::boxed::Box::new(pair)), p),
            )
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
/// HP_HASHVAL (2) and HP_HMAC_INFO (5: HashAlgid; Windows' default
/// strings only).
#[unsafe(no_mangle)]
pub extern "system" fn CryptSetHashParam(
    handle: usize,
    param: u32,
    data: usize,
    _mode: u32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let data = door_windows_abi::widen(data);
    {
        let r = (|| {
            let kind = hash_kind(handle)?;
            match param {
                2 => {
                    let size = hash(handle, |h| Ok(h.size()))?;
                    let mut value = alloc::vec![0u8; size];
                    read(data, &mut value)?;
                    hash(handle, |h| h.set_value(&value))
                }
                5 => {
                    // HMAC_INFO: HashAlgid, pbInnerString, cbInnerString,
                    // pbOuterString, cbOuterString.
                    use door_windows_abi::layout::{self, Field::*};
                    const HMAC_INFO: [layout::Field; 5] = [U32, Pointer, U32, Pointer, U32];
                    let mut info = [0u8; layout::size(&HMAC_INFO)];
                    read(data, &mut info)?;
                    let algorithm = layout::get(&info, &HMAC_INFO, 0) as u32;
                    let inner = layout::get(&info, &HMAC_INFO, 1);
                    let outer = layout::get(&info, &HMAC_INFO, 3);
                    if inner != 0 || outer != 0 {
                        return Err(nte::NOT_SUPPORTED);
                    }
                    hash(handle, |h| h.set_hmac_info(kind, algorithm))
                }
                _ => Err(nte::BAD_TYPE),
            }
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptHashSessionKey(handle: usize, session: usize, mode: u32) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let session = door_windows_abi::widen(session);
    {
        let r = (|| {
            let mut bytes = key(session, |o| match &o.key {
                Key::Symmetric(s) => Ok(s.key.clone()),
                Key::Asymmetric(_) => Err(nte::BAD_KEY),
            })?;
            // CRYPT_LITTLE_ENDIAN (1) keeps the key's order; by default it is
            // hashed reversed.
            if mode & 1 == 0 {
                bytes.reverse();
            }
            hash(handle, |h| h.update(&bytes))
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
fn sign(handle: u64, spec: u32, mode: u32, out: u64, length: u64) -> i32 {
    let r = (|| {
        let context = hash_context(handle)?;
        let pair = provider(context, |c| match spec {
            calg::AT_KEYEXCHANGE => c.exchange.clone(),
            calg::AT_SIGNATURE => c.signature.clone(),
            _ => None,
        })?
        .ok_or(nte::BAD_KEYSET)?;
        let private = pair.private.as_ref().ok_or(nte::NO_KEY)?;
        let (algorithm, value) = hash(handle, |h| Ok((h.algorithm, h.try_value()?)))?;
        let signature = rsa::sign(
            private,
            algorithm,
            &value,
            mode & keys::flag::NOHASHOID != 0,
        )
        .ok_or(nte::BAD_HASH)?;
        give(out, length, &signature)
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptSignHashW(
    handle: usize,
    spec: u32,
    _description: usize,
    mode: u32,
    out: usize,
    length: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    sign(handle, spec, mode, out, length)
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptSignHashA(
    handle: usize,
    spec: u32,
    _description: usize,
    mode: u32,
    out: usize,
    length: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    sign(handle, spec, mode, out, length)
}
fn verify(handle: u64, signature: u64, length: u32, public: u64, mode: u32) -> i32 {
    let r = (|| {
        let pair = key(public, |o| match &o.key {
            Key::Asymmetric(a) => Ok(Asymmetric::clone(a)),
            Key::Symmetric(_) => Err(nte::BAD_KEY),
        })?;
        let mut sig = alloc::vec![0u8; (length as usize).min(4096)];
        read(signature, &mut sig)?;
        let (algorithm, value) = hash(handle, |h| Ok((h.algorithm, h.try_value()?)))?;
        if rsa::verify(
            &pair.public,
            algorithm,
            &value,
            &sig,
            mode & keys::flag::NOHASHOID != 0,
        ) {
            Ok(())
        } else {
            Err(nte::BAD_SIGNATURE)
        }
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptVerifySignatureW(
    handle: usize,
    signature: usize,
    length: u32,
    public: usize,
    _description: usize,
    mode: u32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let signature = door_windows_abi::widen(signature);
    let public = door_windows_abi::widen(public);
    verify(handle, signature, length, public, mode)
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptVerifySignatureA(
    handle: usize,
    signature: usize,
    length: u32,
    public: usize,
    _description: usize,
    mode: u32,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let signature = door_windows_abi::widen(signature);
    let public = door_windows_abi::widen(public);
    verify(handle, signature, length, public, mode)
}
