//! wintrust.dll's exports: WinVerifyTrust(Ex), the CryptCATAdmin functions
//! and CryptCATCatalogInfoFromContext. Files are read through ntdll, the
//! Trusted Root store through advapi32's registry functions, CRLs through
//! WinHTTP (kept in the user's
//! AppData\LocalLow\Microsoft\CryptnetUrlCache\Content, as cryptnet keeps
//! them).
use crate::*;
use alloc::{format, string::String, vec, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};
use door_authenticode::{Error, catalog, revocation};
use door_tls::verify::Anchor;

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32;
    fn DoorCreateFileW(
        path: u64,
        access: u32,
        share: u32,
        security: u64,
        disposition: u32,
        flags: u32,
        template: u64,
    ) -> u64;
    fn DoorReadFile(handle: u64, destination: u64, bytes: u32, read: u64, overlapped: u64) -> i32;
    fn DoorWriteFile(handle: u64, source: u64, bytes: u32, written: u64, overlapped: u64) -> i32;
    fn DoorCloseFile(handle: u64) -> i32;
    fn DoorSetFilePointerEx(handle: u64, distance: i64, destination: u64, method: u32) -> i32;
    fn DoorFindFirstFileW(pattern: u64, destination: u64) -> u64;
    fn DoorFindNextFileW(handle: u64, destination: u64) -> i32;
    fn DoorFindClose(handle: u64) -> i32;
    fn DoorCreateDirectoryW(path: u64) -> i32;
    fn DoorDeleteFileW(path: u64) -> i32;
    fn DoorGetEnvironmentVariableW(name: u64, buffer: u64, size: u32) -> u32;
    fn NtQuerySystemTime(time: usize) -> u32;
    fn RtlSetLastWin32Error(code: u32);
    fn RtlGetLastWin32Error() -> u32;
}
#[link(name = "advapi32", kind = "dylib")]
unsafe extern "system" {
    fn RegOpenKeyExW(key: usize, subkey: usize, options: u32, access: u32, out: usize) -> u32;
    fn RegEnumKeyExW(
        key: usize,
        index: u32,
        name: usize,
        length: usize,
        reserved: usize,
        class: usize,
        class_length: usize,
        time: usize,
    ) -> u32;
    fn RegQueryValueExW(
        key: usize,
        name: usize,
        reserved: usize,
        kind: usize,
        data: usize,
        length: usize,
    ) -> u32;
    fn RegCloseKey(key: usize) -> u32;
}
#[link(name = "winhttp", kind = "dylib")]
unsafe extern "system" {
    fn WinHttpOpen(agent: usize, access: u32, proxy: usize, bypass: usize, flags: u32) -> usize;
    fn WinHttpConnect(session: usize, server: usize, port: u16, reserved: u32) -> usize;
    fn WinHttpOpenRequest(
        connect: usize,
        verb: usize,
        object: usize,
        version: usize,
        referrer: usize,
        types: usize,
        flags: u32,
    ) -> usize;
    fn WinHttpSetTimeouts(
        handle: usize,
        resolve: i32,
        connect: i32,
        send: i32,
        receive: i32,
    ) -> i32;
    fn WinHttpSendRequest(
        request: usize,
        headers: usize,
        length: u32,
        optional: usize,
        optional_length: u32,
        total: u32,
        context: usize,
    ) -> i32;
    fn WinHttpReceiveResponse(request: usize, reserved: usize) -> i32;
    fn WinHttpQueryHeaders(
        request: usize,
        level: u32,
        name: usize,
        buffer: usize,
        length: usize,
        index: usize,
    ) -> i32;
    fn WinHttpReadData(request: usize, buffer: usize, length: u32, read: usize) -> i32;
    fn WinHttpCloseHandle(handle: usize) -> i32;
}

const INVALID_HANDLE: u64 = u64::MAX;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_NOT_FOUND: u32 = 1168;
const ERROR_BAD_FORMAT: u32 = 11;
const ERROR_WRITE_FAULT: u32 = 29;
const TRUST_E_BAD_DIGEST: u32 = 0x8009_6010;
/// CRYPT_E_FILE_ERROR: a subject or catalog that cannot be read.
const CRYPT_E_FILE_ERROR: u32 = 0x8009_2003;
/// NTE_BAD_ALGID: a hash algorithm CryptCATAdminAcquireContext2 lacks.
const NTE_BAD_ALGID: u32 = 0x8009_0008;
const HKEY_LOCAL_MACHINE: u64 = 0xFFFF_FFFF_8000_0002;
const KEY_READ: u32 = 0x2_0019;
/// The largest file or catalog read whole.
const MAX_FILE: usize = 64 << 20;
/// The largest CRL downloaded.
const MAX_CRL: usize = 8 << 20;

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
        // SAFETY: `held` gives this closure exclusive access.
        let r = f(unsafe { &mut *self.value.get() });
        self.held.store(false, Ordering::Release);
        r
    }
}

// ---- application memory --------------------------------------------------
fn read(pointer: u64, out: &mut [u8]) -> Result<(), u32> {
    if out.is_empty() {
        return Ok(());
    }
    if pointer == 0 || unsafe { DoorUserCopy(out.as_mut_ptr() as u64, pointer, out.len()) } != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    Ok(())
}
fn write(pointer: u64, data: &[u8]) -> Result<(), u32> {
    if data.is_empty() {
        return Ok(());
    }
    if pointer == 0 || unsafe { DoorUserCopy(pointer, data.as_ptr() as u64, data.len()) } != 0 {
        return Err(ERROR_INVALID_PARAMETER);
    }
    Ok(())
}
fn u32_at(p: u64) -> Result<u32, u32> {
    let mut b = [0u8; 4];
    read(p, &mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn u64_at(p: u64) -> Result<u64, u32> {
    let mut b = [0u8; 8];
    read(p, &mut b)?;
    Ok(u64::from_le_bytes(b))
}
/// A NUL-terminated UTF-16 string (at most 32767 units).
fn wide(p: u64) -> Result<Vec<u16>, u32> {
    let mut out = Vec::new();
    loop {
        let mut b = [0u8; 2];
        read(p + 2 * out.len() as u64, &mut b)?;
        let u = u16::from_le_bytes(b);
        if u == 0 {
            return Ok(out);
        }
        out.push(u);
        if out.len() > 32767 {
            return Err(ERROR_INVALID_PARAMETER);
        }
    }
}
fn wide0(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}
fn last_error() -> u32 {
    unsafe { RtlGetLastWin32Error() }.max(1)
}
/// Unix seconds now.
fn now() -> i64 {
    let mut t = 0u64;
    unsafe { NtQuerySystemTime(door_windows_abi::narrow(&mut t as *mut u64 as u64)) };
    (t / 10_000_000) as i64 - 11_644_473_600
}
/// A BOOL export's result, the last error set.
fn finish(r: Result<(), u32>) -> i32 {
    match r {
        Ok(()) => {
            unsafe { RtlSetLastWin32Error(0) };
            1
        }
        Err(e) => {
            unsafe { RtlSetLastWin32Error(e) };
            0
        }
    }
}
/// A handle export's result (0 on failure), the last error set.
fn finish_handle(r: Result<u64, u32>) -> u64 {
    match r {
        Ok(h) => {
            unsafe { RtlSetLastWin32Error(0) };
            h
        }
        Err(e) => {
            unsafe { RtlSetLastWin32Error(e) };
            0
        }
    }
}

// ---- files -----------------------------------------------------------------
/// A file by path, whole.
fn read_path(path: &[u16]) -> Result<Vec<u8>, u32> {
    let mut z = path.to_vec();
    z.push(0);
    // GENERIC_READ, FILE_SHARE_READ, OPEN_EXISTING.
    let h = unsafe { DoorCreateFileW(z.as_ptr() as u64, 0x8000_0000, 1, 0, 3, 0x80, 0) };
    if h == INVALID_HANDLE || h == 0 {
        return Err(last_error());
    }
    let r = read_from_start(h);
    unsafe { DoorCloseFile(h) };
    r
}
/// An open file's bytes from its start; its position is kept.
fn read_handle(h: u64) -> Result<Vec<u8>, u32> {
    let mut position = 0i64;
    // FILE_CURRENT.
    if unsafe { DoorSetFilePointerEx(h, 0, &mut position as *mut i64 as u64, 1) } == 0 {
        return Err(last_error());
    }
    let r = read_from_start(h);
    unsafe { DoorSetFilePointerEx(h, position, 0, 0) };
    r
}
fn read_from_start(h: u64) -> Result<Vec<u8>, u32> {
    if unsafe { DoorSetFilePointerEx(h, 0, 0, 0) } == 0 {
        return Err(last_error());
    }
    let mut data = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let mut got = 0u32;
        if unsafe {
            DoorReadFile(
                h,
                chunk.as_mut_ptr() as u64,
                chunk.len() as u32,
                &mut got as *mut u32 as u64,
                0,
            )
        } == 0
        {
            return Err(last_error());
        }
        if got == 0 {
            return Ok(data);
        }
        data.extend_from_slice(&chunk[..got as usize]);
        if data.len() > MAX_FILE {
            return Err(ERROR_BAD_FORMAT);
        }
    }
}
fn write_path(path: &str, data: &[u8]) -> Result<(), u32> {
    // GENERIC_WRITE, CREATE_ALWAYS.
    let h = unsafe { DoorCreateFileW(wide0(path).as_ptr() as u64, 0x4000_0000, 0, 0, 2, 0x80, 0) };
    if h == INVALID_HANDLE || h == 0 {
        return Err(last_error());
    }
    let mut at = 0;
    let mut result = Ok(());
    while at < data.len() {
        let part = (data.len() - at).min(64 * 1024);
        let mut written = 0u32;
        let ok = unsafe {
            DoorWriteFile(
                h,
                data[at..].as_ptr() as u64,
                part as u32,
                &mut written as *mut u32 as u64,
                0,
            )
        };
        if ok == 0 || written == 0 {
            result = Err(ERROR_WRITE_FAULT);
            break;
        }
        at += written as usize;
    }
    unsafe { DoorCloseFile(h) };
    result
}
/// `folder` and the folders above it created (each may exist).
fn make_folders(folder: &str) {
    for (at, _) in folder.match_indices('\\').skip(1) {
        unsafe { DoorCreateDirectoryW(wide0(&folder[..at]).as_ptr() as u64) };
    }
    unsafe { DoorCreateDirectoryW(wide0(folder).as_ptr() as u64) };
}
/// The `*.cat` files of `folder`, by name.
fn catalogs_in(folder: &str) -> Vec<String> {
    let mut names = Vec::new();
    let pattern = wide0(&format!("{folder}\\*.cat"));
    let mut data = [0u8; 592];
    let h = unsafe { DoorFindFirstFileW(pattern.as_ptr() as u64, data.as_mut_ptr() as u64) };
    if h == INVALID_HANDLE || h == 0 {
        return names;
    }
    loop {
        // WIN32_FIND_DATAW: dwFileAttributes, then cFileName at 44.
        let directory = data[0] & 0x10 != 0;
        let units: Vec<u16> = data[44..44 + 520]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        if !directory {
            names.push(String::from_utf16_lossy(&units));
        }
        if unsafe { DoorFindNextFileW(h, data.as_mut_ptr() as u64) } == 0 {
            break;
        }
    }
    unsafe { DoorFindClose(h) };
    names.sort();
    names
}

// ---- trust anchors ---------------------------------------------------------
/// The DER certificates of the machine's Trusted Root store.
fn root_store() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let path = wide0("SOFTWARE\\Microsoft\\SystemCertificates\\ROOT\\Certificates");
    let mut key = 0u64;
    if unsafe {
        RegOpenKeyExW(
            door_windows_abi::narrow(HKEY_LOCAL_MACHINE),
            door_windows_abi::narrow(path.as_ptr() as u64),
            0,
            KEY_READ,
            door_windows_abi::narrow(&mut key as *mut u64 as u64),
        )
    } != 0
    {
        return out;
    }
    let value = wide0("Blob");
    for index in 0..512u32 {
        let mut name = [0u16; 64];
        let mut length = name.len() as u32;
        if unsafe {
            RegEnumKeyExW(
                door_windows_abi::narrow(key),
                index,
                door_windows_abi::narrow(name.as_mut_ptr() as u64),
                door_windows_abi::narrow(&mut length as *mut u32 as u64),
                0,
                0,
                0,
                0,
            )
        } != 0
        {
            break;
        }
        let mut sub = 0u64;
        let mut z = name[..length as usize].to_vec();
        z.push(0);
        if unsafe {
            RegOpenKeyExW(
                door_windows_abi::narrow(key),
                door_windows_abi::narrow(z.as_ptr() as u64),
                0,
                KEY_READ,
                door_windows_abi::narrow(&mut sub as *mut u64 as u64),
            )
        } != 0
        {
            continue;
        }
        let mut size = 0u32;
        unsafe {
            RegQueryValueExW(
                door_windows_abi::narrow(sub),
                door_windows_abi::narrow(value.as_ptr() as u64),
                0,
                0,
                0,
                door_windows_abi::narrow(&mut size as *mut u32 as u64),
            )
        };
        if size > 0 && size < 64 * 1024 {
            let mut blob = vec![0u8; size as usize];
            if unsafe {
                RegQueryValueExW(
                    door_windows_abi::narrow(sub),
                    door_windows_abi::narrow(value.as_ptr() as u64),
                    0,
                    0,
                    door_windows_abi::narrow(blob.as_mut_ptr() as u64),
                    door_windows_abi::narrow(&mut size as *mut u32 as u64),
                )
            } == 0
                && let Some(cert) =
                    door_authenticode::roots::store_certificate(&blob[..size as usize])
            {
                out.push(cert.to_vec());
            }
        }
        unsafe { RegCloseKey(door_windows_abi::narrow(sub)) };
    }
    unsafe { RegCloseKey(door_windows_abi::narrow(key)) };
    out
}
/// Every trust anchor: the Trusted Root store's certificates (`store` holds
/// their bytes), Doors' code-signing roots and the Mozilla roots.
fn anchors(store: &[Vec<u8>]) -> Vec<Anchor<'_>> {
    let mut a = door_authenticode::roots::of(store);
    a.extend(door_authenticode::roots::trusted());
    a
}

// ---- CRLs ------------------------------------------------------------------
/// CRLs from the user's URL cache, else (unless cache-only) over HTTP.
struct Cryptnet {
    cache_only: bool,
}
fn cache_folder() -> Option<String> {
    let mut out = [0u16; 260];
    let name = wide0("USERPROFILE");
    let n =
        unsafe { DoorGetEnvironmentVariableW(name.as_ptr() as u64, out.as_mut_ptr() as u64, 260) }
            as usize;
    (n > 0 && n < 200).then(|| {
        format!(
            "{}\\AppData\\LocalLow\\Microsoft\\CryptnetUrlCache\\Content",
            String::from_utf16_lossy(&out[..n])
        )
    })
}
impl revocation::Source for Cryptnet {
    fn crl(&mut self, url: &str) -> Option<Vec<u8>> {
        let now = now();
        let current = |bytes: &[u8]| {
            revocation::parse(bytes)
                .is_some_and(|c| c.this_update() <= now && c.next_update().is_none_or(|n| n >= now))
        };
        let folder = cache_folder();
        let file = folder.as_ref().map(|f| format!("{f}\\{}", cache_name(url)));
        if let Some(file) = &file {
            let units: Vec<u16> = file.encode_utf16().collect();
            if let Ok(bytes) = read_path(&units)
                && current(&bytes)
            {
                return Some(bytes);
            }
        }
        if self.cache_only {
            return None;
        }
        let bytes = http_get(url)?;
        revocation::parse(&bytes)?;
        if let (Some(folder), Some(file)) = (&folder, &file) {
            make_folders(folder);
            let _ = write_path(file, &bytes);
        }
        Some(bytes)
    }
}
/// GET an http:// URL with WinHTTP under the user's proxy settings, as
/// cryptnet retrieves CRLs (15 seconds at most per step).
fn http_get(url: &str) -> Option<Vec<u8>> {
    let (host, port, path) = split_url(url)?;
    let agent = wide0("Microsoft-CryptoAPI/10.0");
    let get = wide0("GET");
    let host = wide0(host);
    let path = wide0(path);
    unsafe {
        // WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY.
        let session = WinHttpOpen(door_windows_abi::narrow(agent.as_ptr() as u64), 4, 0, 0, 0);
        if session == 0 {
            return None;
        }
        WinHttpSetTimeouts(session, 15_000, 15_000, 15_000, 15_000);
        let connect = WinHttpConnect(
            session,
            door_windows_abi::narrow(host.as_ptr() as u64),
            port,
            0,
        );
        let request = if connect != 0 {
            WinHttpOpenRequest(
                connect,
                door_windows_abi::narrow(get.as_ptr() as u64),
                door_windows_abi::narrow(path.as_ptr() as u64),
                0,
                0,
                0,
                0,
            )
        } else {
            0
        };
        let mut body = None;
        if request != 0
            && WinHttpSendRequest(request, 0, 0, 0, 0, 0, 0) != 0
            && WinHttpReceiveResponse(request, 0) != 0
        {
            // WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER.
            let mut status = 0u32;
            let mut size = 4u32;
            WinHttpQueryHeaders(
                request,
                19 | 0x2000_0000,
                0,
                door_windows_abi::narrow(&mut status as *mut u32 as u64),
                door_windows_abi::narrow(&mut size as *mut u32 as u64),
                0,
            );
            if status == 200 {
                let mut data = Vec::new();
                let mut chunk = vec![0u8; 16 * 1024];
                loop {
                    let mut got = 0u32;
                    if WinHttpReadData(
                        request,
                        door_windows_abi::narrow(chunk.as_mut_ptr() as u64),
                        chunk.len() as u32,
                        door_windows_abi::narrow(&mut got as *mut u32 as u64),
                    ) == 0
                    {
                        break;
                    }
                    if got == 0 {
                        body = Some(data);
                        break;
                    }
                    data.extend_from_slice(&chunk[..got as usize]);
                    if data.len() > MAX_CRL {
                        break;
                    }
                }
            }
        }
        for h in [request, connect, session] {
            if h != 0 {
                WinHttpCloseHandle(h);
            }
        }
        body
    }
}

// ---- WinVerifyTrust --------------------------------------------------------
/// WinVerifyTrust: the trust provider's verdict (0 when trusted).
#[unsafe(no_mangle)]
pub extern "system" fn WinVerifyTrust(_window: usize, action_id: usize, data: usize) -> i32 {
    let action_id = door_windows_abi::widen(action_id);
    let data = door_windows_abi::widen(data);
    verify_trust(action_id, data) as i32
}
#[unsafe(no_mangle)]
pub extern "system" fn WinVerifyTrustEx(window: usize, action_id: usize, data: usize) -> i32 {
    let window = door_windows_abi::widen(window);
    let action_id = door_windows_abi::widen(action_id);
    let data = door_windows_abi::widen(data);
    {
        WinVerifyTrust(
            door_windows_abi::narrow(window),
            door_windows_abi::narrow(action_id),
            door_windows_abi::narrow(data),
        )
    }
}
fn verify_trust(action_id: u64, data: u64) -> u32 {
    let mut guid = [0u8; 16];
    if read(action_id, &mut guid).is_err() {
        return hr::E_INVALIDARG;
    }
    if guid != action::GENERIC_VERIFY_V2 && guid != action::DRIVER_VERIFY {
        return hr::TRUST_E_PROVIDER_UNKNOWN;
    }
    // WINTRUST_DATA (88 bytes on x64).
    let mut d = [0u8; 88];
    if read(data, &mut d[..4]).is_err() {
        return hr::E_INVALIDARG;
    }
    let size = (u32::from_le_bytes([d[0], d[1], d[2], d[3]]) as usize).min(88);
    if size < 48 || read(data, &mut d[..size]).is_err() {
        return hr::E_INVALIDARG;
    }
    let w = |at: usize| u32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]]);
    let q = |at: usize| {
        let mut b = [0u8; 8];
        b.copy_from_slice(&d[at..at + 8]);
        u64::from_le_bytes(b)
    };
    let revocation_checks = w(28);
    let choice = w(32);
    let info = q(40);
    let state = if size >= 52 {
        w(48)
    } else {
        data::STATE_IGNORE
    };
    let flags = if size >= 76 { w(72) } else { 0 };
    if state == data::STATE_CLOSE {
        if size >= 64 {
            let _ = write(data + 56, &0u64.to_le_bytes());
        }
        return hr::S_OK;
    }
    let mut net = Cryptnet {
        cache_only: flags & data::PROV_CACHE_ONLY_URL_RETRIEVAL != 0,
    };
    let store = root_store();
    let anchors = anchors(&store);
    let source: Option<&mut dyn revocation::Source> = if data::revocation(revocation_checks, flags)
    {
        Some(&mut net)
    } else {
        None
    };
    let result = match choice {
        data::CHOICE_FILE => verify_file(info, &anchors, source),
        data::CHOICE_CATALOG => verify_catalog(info, &anchors, source),
        // ERROR_INVALID_PARAMETER itself, not as an HRESULT (measured).
        _ => Err(ERROR_INVALID_PARAMETER),
    };
    // WTD_STATEACTION_VERIFY: a state handle for the later CLOSE.
    if state == data::STATE_VERIFY && size >= 64 {
        let _ = write(data + 56, &1u64.to_le_bytes());
    }
    result.err().unwrap_or(hr::S_OK)
}
/// WINTRUST_FILE_INFO { cbStruct, pcwszFilePath, hFile, pgKnownSubject }.
fn verify_file(
    info: u64,
    anchors: &[Anchor<'_>],
    source: Option<&mut dyn revocation::Source>,
) -> Result<(), u32> {
    let path = u64_at(info + 8).map_err(|_| hr::E_INVALIDARG)?;
    let handle = u64_at(info + 16).map_err(|_| hr::E_INVALIDARG)?;
    let bytes = if handle != 0 && handle != INVALID_HANDLE {
        read_handle(handle)
    } else {
        read_path(&wide(path).map_err(|_| hr::E_INVALIDARG)?)
    }
    .map_err(|_| CRYPT_E_FILE_ERROR)?;
    door_authenticode::verify_full(&bytes, now(), anchors, source)
        .map(|_| ())
        .map_err(Error::hresult)
}
/// WINTRUST_CATALOG_INFO { cbStruct, dwCatalogVersion, pcwszCatalogFilePath,
/// pcwszMemberTag, pcwszMemberFilePath, hMemberFile, pbCalculatedFileHash,
/// cbCalculatedFileHash, pcCatalogContext, hCatAdmin }, checked in
/// Windows' order (measured): the catalog read (CRYPT_E_FILE_ERROR), the
/// member, then the catalog's signature. A calculated hash names the member
/// itself, the tag aside: a SHA-1 hash, or a SHA-256 one under a SHA-256
/// hCatAdmin (TRUST_E_NOSIGNATURE otherwise). Without one, the tag names the
/// member (TRUST_E_NOSIGNATURE when none has it) and the file's hash in the
/// catalog's algorithm must be the member's (TRUST_E_BAD_DIGEST).
fn verify_catalog(
    info: u64,
    anchors: &[Anchor<'_>],
    source: Option<&mut dyn revocation::Source>,
) -> Result<(), u32> {
    let bad = |_| hr::E_INVALIDARG;
    let size = u32_at(info).map_err(bad)?;
    let cat_path = wide(u64_at(info + 8).map_err(bad)?).map_err(bad)?;
    let tag = u64_at(info + 16).map_err(bad)?;
    let member_path = u64_at(info + 24).map_err(bad)?;
    let member_file = u64_at(info + 32).map_err(bad)?;
    let given = u64_at(info + 40).map_err(bad)?;
    let given_length = u32_at(info + 48).map_err(bad)? as usize;
    let cat_admin = if size >= 72 {
        u64_at(info + 64).map_err(bad)?
    } else {
        0
    };
    let cat = read_path(&cat_path).map_err(|_| CRYPT_E_FILE_ERROR)?;
    let list = catalog::read(&cat).ok_or(CRYPT_E_FILE_ERROR)?;
    if given != 0 && given_length > 0 {
        let wanted = if admin(cat_admin, |a| a.sha256) == Some(true) {
            32
        } else {
            20
        };
        let mut hash = vec![0u8; given_length.min(64)];
        read(given, &mut hash).map_err(bad)?;
        if given_length != wanted || !list.contains(&hash) {
            return Err(hr::TRUST_E_NOSIGNATURE);
        }
    } else {
        let bytes = if member_file != 0 && member_file != INVALID_HANDLE {
            read_handle(member_file)
        } else {
            read_path(&wide(member_path).map_err(bad)?)
        }
        .map_err(|_| CRYPT_E_FILE_ERROR)?;
        let hash = list.hash_of(&bytes);
        if tag != 0 {
            let tag = String::from_utf16_lossy(&wide(tag).map_err(bad)?);
            let member = list.by_tag(&tag).ok_or(hr::TRUST_E_NOSIGNATURE)?;
            if !member.matches(&hash) {
                return Err(TRUST_E_BAD_DIGEST);
            }
        } else if !list.contains(&hash) {
            return Err(hr::TRUST_E_NOSIGNATURE);
        }
    }
    catalog::verify(&cat, now(), anchors, source)
        .map(|_| ())
        .map_err(Error::hresult)
}
// ---- CryptCATAdmin ---------------------------------------------------------
struct Admin {
    folder: String,
    sha256: bool,
}
static ADMINS: Lock<Vec<Option<Admin>>> = Lock::new(Vec::new());
/// Open catalog infos: their catalog files' paths.
static INFOS: Lock<Vec<Option<String>>> = Lock::new(Vec::new());
#[cfg(target_pointer_width = "64")]
const ADMIN_BASE: u64 = 0x4341_0000_0000;
/// A 32-bit process's handle values stay below 2 GB (P17: its HANDLEs are
/// 4 bytes, sign-extended), clear of the kernel's and ntdll's.
#[cfg(not(target_pointer_width = "64"))]
const ADMIN_BASE: u64 = 0x5B00_0000;
#[cfg(target_pointer_width = "64")]
const INFO_BASE: u64 = 0x4349_0000_0000;
/// A 32-bit process's handle values stay below 2 GB (P17: its HANDLEs are
/// 4 bytes, sign-extended), clear of the kernel's and ntdll's.
#[cfg(not(target_pointer_width = "64"))]
const INFO_BASE: u64 = 0x5C00_0000;
fn insert<T>(list: &Lock<Vec<Option<T>>>, value: T, base: u64) -> u64 {
    list.with(|l| {
        let i = l.iter().position(Option::is_none).unwrap_or_else(|| {
            l.push(None);
            l.len() - 1
        });
        l[i] = Some(value);
        base + i as u64
    })
}
fn admin<R>(h: u64, f: impl FnOnce(&Admin) -> R) -> Option<R> {
    ADMINS.with(|a| {
        h.checked_sub(ADMIN_BASE)
            .and_then(|i| a.get(i as usize))
            .and_then(Option::as_ref)
            .map(f)
    })
}
fn admin_folder(h: u64) -> Result<String, u32> {
    admin(h, |a| a.folder.clone()).ok_or(ERROR_INVALID_PARAMETER)
}
fn take_info(h: u64) -> Option<String> {
    INFOS.with(|i| {
        h.checked_sub(INFO_BASE)
            .and_then(|x| i.get_mut(x as usize))
            .and_then(Option::take)
    })
}
fn acquire(out: u64, subsystem: u64, algorithm: Option<&[u16]>) -> Result<(), u32> {
    let mut guid = action::DRIVER_VERIFY;
    if subsystem != 0 {
        read(subsystem, &mut guid)?;
    }
    let sha256 = match algorithm.map(String::from_utf16_lossy) {
        None => false,
        Some(a) if a.eq_ignore_ascii_case("SHA1") => false,
        Some(a) if a.eq_ignore_ascii_case("SHA256") => true,
        Some(_) => return Err(NTE_BAD_ALGID),
    };
    let h = insert(
        &ADMINS,
        Admin {
            folder: store_folder(&guid),
            sha256,
        },
        ADMIN_BASE,
    );
    write(out, &h.to_le_bytes())
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptCATAdminAcquireContext(
    out: usize,
    subsystem: usize,
    _flags: u32,
) -> i32 {
    let out = door_windows_abi::widen(out);
    let subsystem = door_windows_abi::widen(subsystem);
    finish(acquire(out, subsystem, None))
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptCATAdminAcquireContext2(
    out: usize,
    subsystem: usize,
    algorithm: usize,
    _policy: usize,
    _flags: u32,
) -> i32 {
    let out = door_windows_abi::widen(out);
    let subsystem = door_windows_abi::widen(subsystem);
    let algorithm = door_windows_abi::widen(algorithm);
    {
        finish((|| {
            let name = if algorithm == 0 {
                None
            } else {
                Some(wide(algorithm)?)
            };
            acquire(out, subsystem, name.as_deref())
        })())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptCATAdminReleaseContext(h: usize, _flags: u32) -> i32 {
    let h = door_windows_abi::widen(h);
    {
        let found = ADMINS.with(|a| {
            h.checked_sub(ADMIN_BASE)
                .and_then(|i| a.get_mut(i as usize))
                .and_then(Option::take)
                .is_some()
        });
        finish(if found {
            Ok(())
        } else {
            Err(ERROR_INVALID_PARAMETER)
        })
    }
}
/// A file's hash as a catalog names it: SHA-1, or SHA-256 for a context of
/// CryptCATAdminAcquireContext2(L"SHA256"). `length` holds the buffer's
/// size and receives the hash's; a NULL buffer only asks the size, which
/// succeeds, while a short one fails with ERROR_INSUFFICIENT_BUFFER
/// (measured).
fn calc(file: u64, length: u64, out: u64, sha256: bool) -> Result<(), u32> {
    if file == 0 || file == INVALID_HANDLE {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let size: u32 = if sha256 { 32 } else { 20 };
    let have = u32_at(length)?;
    write(length, &size.to_le_bytes())?;
    if out == 0 {
        return Ok(());
    }
    if have < size {
        return Err(ERROR_INSUFFICIENT_BUFFER);
    }
    let hash = pick(catalog::file_hashes(&read_handle(file)?), sha256);
    write(out, &hash)
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptCATAdminCalcHashFromFileHandle(
    file: usize,
    length: usize,
    out: usize,
    _flags: u32,
) -> i32 {
    let file = door_windows_abi::widen(file);
    let length = door_windows_abi::widen(length);
    let out = door_windows_abi::widen(out);
    finish(calc(file, length, out, false))
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptCATAdminCalcHashFromFileHandle2(
    h: usize,
    file: usize,
    length: usize,
    out: usize,
    _flags: u32,
) -> i32 {
    let h = door_windows_abi::widen(h);
    let file = door_windows_abi::widen(file);
    let length = door_windows_abi::widen(length);
    let out = door_windows_abi::widen(out);
    {
        finish(match admin(h, |a| a.sha256) {
            Some(sha256) => calc(file, length, out, sha256),
            None => Err(ERROR_INVALID_PARAMETER),
        })
    }
}
/// The next catalog of the context's store naming `hash`, after the one
/// `*previous` holds (released): its HCATINFO, or 0 (ERROR_NOT_FOUND).
#[unsafe(no_mangle)]
pub extern "system" fn CryptCATAdminEnumCatalogFromHash(
    h: usize,
    hash: usize,
    length: u32,
    _flags: u32,
    previous: usize,
) -> usize {
    let h = door_windows_abi::widen(h);
    let hash = door_windows_abi::widen(hash);
    let previous = door_windows_abi::widen(previous);
    door_windows_abi::narrow((move || -> u64 {
        finish_handle((|| {
            let folder = admin_folder(h)?;
            if length == 0 || length > 64 {
                return Err(ERROR_INVALID_PARAMETER);
            }
            let mut wanted = vec![0u8; length as usize];
            read(hash, &mut wanted)?;
            let after = if previous != 0 {
                take_info(u64_at(previous)?)
            } else {
                None
            };
            if previous != 0 {
                write(previous, &0u64.to_le_bytes())?;
            }
            let names = catalogs_in(&folder);
            let start = after
                .as_ref()
                .and_then(|p| {
                    names
                        .iter()
                        .position(|n| p.len() > n.len() && p.ends_with(n.as_str()))
                })
                .map_or(0, |i| i + 1);
            for name in &names[start..] {
                let path = format!("{folder}\\{name}");
                let units: Vec<u16> = path.encode_utf16().collect();
                let Ok(bytes) = read_path(&units) else {
                    continue;
                };
                if catalog::read(&bytes).is_some_and(|list| list.contains(&wanted)) {
                    return Ok(insert(&INFOS, path, INFO_BASE));
                }
            }
            Err(ERROR_NOT_FOUND)
        })())
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptCATAdminReleaseCatalogContext(
    _h: usize,
    info: usize,
    _flags: u32,
) -> i32 {
    let info = door_windows_abi::widen(info);
    finish(take_info(info).map(|_| ()).ok_or(ERROR_INVALID_PARAMETER))
}
/// CATALOG_INFO { cbStruct, wszCatalogFile[MAX_PATH] }.
#[unsafe(no_mangle)]
pub extern "system" fn CryptCATCatalogInfoFromContext(info: usize, out: usize, _flags: u32) -> i32 {
    let info = door_windows_abi::widen(info);
    let out = door_windows_abi::widen(out);
    {
        finish((|| {
            let path = INFOS
                .with(|i| {
                    info.checked_sub(INFO_BASE)
                        .and_then(|x| i.get(x as usize))
                        .and_then(Option::clone)
                })
                .ok_or(ERROR_INVALID_PARAMETER)?;
            let units = wide0(&path);
            if units.len() > 260 {
                return Err(ERROR_INSUFFICIENT_BUFFER);
            }
            let bytes: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
            write(out + 4, &bytes)
        })())
    }
}
/// Copies a catalog into the context's store (C:\Windows\System32, so
/// administrators only) as `base_name` (else its own name): its HCATINFO.
#[unsafe(no_mangle)]
pub extern "system" fn CryptCATAdminAddCatalog(
    h: usize,
    file: usize,
    base_name: usize,
    _flags: u32,
) -> usize {
    let h = door_windows_abi::widen(h);
    let file = door_windows_abi::widen(file);
    let base_name = door_windows_abi::widen(base_name);
    door_windows_abi::narrow((move || -> u64 {
        finish_handle((|| {
            let folder = admin_folder(h)?;
            let source = wide(file)?;
            let bytes = read_path(&source)?;
            if catalog::read(&bytes).is_none() {
                return Err(ERROR_BAD_FORMAT);
            }
            let name = if base_name != 0 {
                String::from_utf16_lossy(&wide(base_name)?)
            } else {
                let s = String::from_utf16_lossy(&source);
                String::from(s.rsplit(['\\', '/']).next().unwrap_or(&s))
            };
            if !plain_name(&name) {
                return Err(ERROR_INVALID_PARAMETER);
            }
            make_folders(&folder);
            let path = format!("{folder}\\{name}");
            write_path(&path, &bytes)?;
            Ok(insert(&INFOS, path, INFO_BASE))
        })())
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CryptCATAdminRemoveCatalog(h: usize, file: usize, _flags: u32) -> i32 {
    let h = door_windows_abi::widen(h);
    let file = door_windows_abi::widen(file);
    {
        finish((|| {
            let folder = admin_folder(h)?;
            let name = String::from_utf16_lossy(&wide(file)?);
            if !plain_name(&name) {
                return Err(ERROR_INVALID_PARAMETER);
            }
            let path = wide0(&format!("{folder}\\{name}"));
            if unsafe { DoorDeleteFileW(path.as_ptr() as u64) } == 0 {
                return Err(last_error());
            }
            Ok(())
        })())
    }
}

// ---- WinTrust helpers -----------------------------------------------------------------

/// `WTHelperProvDataFromStateData(stateData)`: the CRYPT_PROVIDER_DATA a
/// verification kept. Doors' WinVerifyTrust keeps no provider data (its
/// state handle exists only to be closed), so there is none: NULL, which
/// callers read as "no signer information".
#[unsafe(no_mangle)]
pub extern "system" fn WTHelperProvDataFromStateData(_state: usize) -> usize {
    0
}
/// `WTHelperGetProvSignerFromChain`: no provider data, no signer.
#[unsafe(no_mangle)]
pub extern "system" fn WTHelperGetProvSignerFromChain(
    _data: usize,
    _signer: u32,
    _counter_signer: i32,
    _counter_index: u32,
) -> usize {
    0
}
/// `WTHelperGetProvCertFromChain`: no signer, no certificate.
#[unsafe(no_mangle)]
pub extern "system" fn WTHelperGetProvCertFromChain(_signer: usize, _index: u32) -> usize {
    0
}
/// `WTHelperGetFileHash(path, flags, reserved, hash, size, algId)`: the
/// hash a file's embedded signature records; Doors keeps no signer data
/// for it here, so the file reads as unsigned (TRUST_E_NOSIGNATURE).
#[unsafe(no_mangle)]
pub extern "system" fn WTHelperGetFileHash(
    _path: usize,
    _flags: u32,
    _reserved: usize,
    _hash: usize,
    _size: usize,
    _alg: usize,
) -> i32 {
    0x800B_0100u32 as i32
}
