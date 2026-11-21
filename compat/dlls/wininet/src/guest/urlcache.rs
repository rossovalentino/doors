//! The URL cache: `%LOCALAPPDATA%\Microsoft\Windows\INetCache\IE`, four
//! folders of files and an index (`container.dat`, door-windows-inet's
//! `cache::Index`) every process shares under a lock file. HTTP GET
//! responses the request may store are written as they are read;
//! INTERNET_FLAG_FROM_CACHE answers from it; the UrlCache API reads and
//! edits it.
use super::{Text, done, encode, error, fail, read, read_u32, text, unit, write, write_u32};
use alloc::{string::String, vec, vec::Vec};
use door_windows_abi::layout::{self, Field, Field::*};
use door_windows_inet::cache::{Entry, Index, kind};
use door_windows_inet::sync::Lock;

#[link(name = "kernel32", kind = "dylib")]
unsafe extern "system" {
    fn CreateFileW(
        name: usize,
        access: u32,
        share: u32,
        security: usize,
        creation: u32,
        flags: u32,
        template: usize,
    ) -> usize;
    fn ReadFile(file: usize, buffer: usize, bytes: u32, read: usize, overlapped: usize) -> i32;
    fn WriteFile(file: usize, buffer: usize, bytes: u32, written: usize, overlapped: usize) -> i32;
    fn CloseHandle(handle: usize) -> i32;
    fn CreateDirectoryW(name: usize, security: usize) -> i32;
    fn DeleteFileW(name: usize) -> i32;
    fn GetFileAttributesW(name: usize) -> u32;
    fn GetEnvironmentVariableW(name: usize, buffer: usize, size: u32) -> u32;
    fn GetSystemTimeAsFileTime(time: usize);
}
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const FILE_SHARE_READ: u32 = 1;
const FILE_SHARE_DELETE: u32 = 4;
const INVALID_HANDLE_VALUE: u64 = u64::MAX;
const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
/// The largest body stored (larger responses pass uncached).
pub const MAX_STORED: usize = 16 * 1024 * 1024;
/// INTERNET_CACHE_ENTRY_INFO: dwStructSize, lpszSourceUrlName,
/// lpszLocalFileName, CacheEntryType, dwUseCount, dwHitRate, dwSizeLow,
/// dwSizeHigh, the four FILETIMEs (4-aligned), lpHeaderInfo,
/// dwHeaderInfoSize, lpszFileExtension, dwExemptDelta. Its fixed part is
/// 112 bytes on x64 (as measured on Windows), 80 on x86.
const INFO: [Field; 16] = [
    U32,
    Pointer,
    Pointer,
    U32,
    U32,
    U32,
    U32,
    U32,
    Bytes(8),
    Bytes(8),
    Bytes(8),
    Bytes(8),
    Pointer,
    U32,
    Pointer,
    U32,
];
const INFO_BYTES: usize = layout::size(&INFO);
fn info_at(index: usize) -> usize {
    layout::offset(&INFO, index)
}
/// The type of entries stored from HTTP responses, as Windows stores them.
const HTTP_ENTRY: u32 = kind::NORMAL | 0x40;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}
fn now() -> u64 {
    let mut t = 0u64;
    unsafe { GetSystemTimeAsFileTime(door_windows_abi::narrow(&mut t as *mut u64 as u64)) };
    t
}
fn environment(name: &str) -> Option<String> {
    let name = wide(name);
    let mut buffer = vec![0u16; 520];
    let n = unsafe {
        GetEnvironmentVariableW(
            door_windows_abi::narrow(name.as_ptr() as u64),
            door_windows_abi::narrow(buffer.as_mut_ptr() as u64),
            520,
        )
    };
    (n != 0 && (n as usize) < buffer.len()).then(|| String::from_utf16_lossy(&buffer[..n as usize]))
}
/// `%LOCALAPPDATA%\Microsoft\Windows\INetCache\IE`, made when missing.
fn root() -> Option<String> {
    let local = environment("LOCALAPPDATA")
        .or_else(|| environment("USERPROFILE").map(|p| alloc::format!("{p}\\AppData\\Local")))?;
    let mut path = local;
    for part in ["Microsoft", "Windows", "INetCache", "IE"] {
        path.push('\\');
        path.push_str(part);
        let w = wide(&path);
        unsafe { CreateDirectoryW(door_windows_abi::narrow(w.as_ptr() as u64), 0) };
    }
    Some(path)
}
fn read_file(path: &str) -> Option<Vec<u8>> {
    let w = wide(path);
    let file = unsafe {
        CreateFileW(
            door_windows_abi::narrow(w.as_ptr() as u64),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_DELETE,
            0,
            OPEN_EXISTING,
            0,
            0,
        )
    };
    if file == door_windows_abi::narrow(INVALID_HANDLE_VALUE) {
        return None;
    }
    let mut out = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let mut got = 0u32;
        let ok = unsafe {
            ReadFile(
                file,
                door_windows_abi::narrow(chunk.as_mut_ptr() as u64),
                chunk.len() as u32,
                door_windows_abi::narrow(&mut got as *mut u32 as u64),
                0,
            )
        };
        if ok == 0 || got == 0 || out.len() + got as usize > MAX_STORED {
            break;
        }
        out.extend_from_slice(&chunk[..got as usize]);
    }
    unsafe { CloseHandle(file) };
    Some(out)
}
fn write_file(path: &str, data: &[u8]) -> bool {
    let w = wide(path);
    let file = unsafe {
        CreateFileW(
            door_windows_abi::narrow(w.as_ptr() as u64),
            GENERIC_WRITE,
            0,
            0,
            CREATE_ALWAYS,
            0,
            0,
        )
    };
    if file == door_windows_abi::narrow(INVALID_HANDLE_VALUE) {
        return false;
    }
    let mut written = 0u32;
    let ok = data.is_empty()
        || unsafe {
            WriteFile(
                file,
                door_windows_abi::narrow(data.as_ptr() as u64),
                data.len() as u32,
                door_windows_abi::narrow(&mut written as *mut u32 as u64),
                0,
            )
        } != 0
            && written as usize == data.len();
    unsafe { CloseHandle(file) };
    ok
}
fn exists(path: &str) -> bool {
    let w = wide(path);
    unsafe {
        GetFileAttributesW(door_windows_abi::narrow(w.as_ptr() as u64)) != INVALID_FILE_ATTRIBUTES
    }
}
fn delete(path: &str) {
    let w = wide(path);
    unsafe { DeleteFileW(door_windows_abi::narrow(w.as_ptr() as u64)) };
}

/// Threads of this process take turns on the index; it is read before
/// and written after each change.
static LOCAL: Lock<()> = Lock::new(());
/// Run `f` on the index; it saves the index when `f` answers true in its
/// second value.
fn with_index<R>(f: impl FnOnce(&str, &mut Index) -> (R, bool)) -> Option<R> {
    let root = root()?;
    LOCAL.with(|_| {
        let index_path = alloc::format!("{root}\\container.dat");
        let mut index = read_file(&index_path)
            .and_then(|b| Index::decode(&String::from_utf8_lossy(&b)))
            .unwrap_or_else(|| Index::new(now() ^ 0x9e37_79b9_7f4a_7c15));
        for folder in &index.folders {
            let w = wide(&alloc::format!("{root}\\{folder}"));
            unsafe { CreateDirectoryW(door_windows_abi::narrow(w.as_ptr() as u64), 0) };
        }
        let (result, save) = f(&root, &mut index);
        if save {
            write_file(&index_path, index.encode().as_bytes());
        }
        Some(result)
    })
}
/// The full path of an entry's file.
fn full(root: &str, file: &str) -> String {
    alloc::format!("{root}\\{file}")
}
/// A cache path's entry-relative name ("FOLDER\\name"), when it is one.
fn relative(root: &str, path: &str) -> Option<String> {
    let rest = path.strip_prefix(root)?.strip_prefix('\\')?;
    door_windows_inet::cache::safe_file(rest).then(|| String::from(rest))
}

/// The headers a stored response keeps: the status line and what describes
/// the body (as Windows stores them: not Date, Server, the caching and
/// connection headers, or cookies).
pub fn stored_head(raw: &str) -> String {
    const DROPPED: [&str; 14] = [
        "date",
        "server",
        "cache-control",
        "expires",
        "last-modified",
        "etag",
        "age",
        "pragma",
        "connection",
        "keep-alive",
        "transfer-encoding",
        "set-cookie",
        "set-cookie2",
        "vary",
    ];
    let mut out = String::new();
    for (i, line) in raw.split("\r\n").enumerate() {
        if line.is_empty() {
            break;
        }
        let name = line
            .split(':')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if i == 0 || !DROPPED.contains(&name.as_str()) {
            out.push_str(line);
            out.push_str("\r\n");
        }
    }
    out.push_str("\r\n");
    out
}
/// Store a response read to its end: URL, head as received, body, and
/// when it goes stale.
pub fn store(
    url: &str,
    raw_head: &str,
    body: &[u8],
    cache_control: Option<&str>,
    expires: Option<u64>,
) {
    let now = now();
    let extension = String::from(
        url.split(['?', '#'])
            .next()
            .and_then(|p| p.rsplit('/').next())
            .and_then(|last| last.rsplit_once('.').map(|(_, e)| e))
            .unwrap_or(""),
    );
    let head = stored_head(raw_head);
    with_index(|root, index| {
        let folder = (now as usize / 7) % index.folders.len().max(1);
        let name = index.file_name(url, &extension, folder, &|n: &str| exists(&full(root, n)));
        if !write_file(&full(root, &name), body) {
            return ((), false);
        }
        let entry = Entry {
            url: String::from(url),
            file: name,
            kind: HTTP_ENTRY,
            size: body.len() as u64,
            modified: now,
            expires: door_windows_inet::cache::expiry(now, cache_control, expires),
            accessed: now,
            synced: now,
            uses: 0,
            hits: 1,
            extension,
            headers: head.into_bytes(),
            locks: 0,
        };
        match index.commit(entry) {
            Ok(dropped) => {
                for f in dropped {
                    delete(&full(root, &f));
                }
                ((), true)
            }
            Err(_) => ((), false),
        }
    });
}
/// A stored response: its head and body (FROM_CACHE).
pub fn load(url: &str) -> Option<(String, Vec<u8>)> {
    with_index(|root, index| {
        let Some(at) = index.find(url) else {
            return (None, false);
        };
        let e = &mut index.entries[at];
        let body = if e.file.is_empty() {
            Some(Vec::new())
        } else {
            read_file(&full(root, &e.file))
        };
        e.accessed = now();
        e.hits = e.hits.saturating_add(1);
        let head = String::from_utf8_lossy(&e.headers).into_owned();
        (body.map(|b| (head, b)), true)
    })
    .flatten()
}

// ---- INTERNET_CACHE_ENTRY_INFO ---------------------------------------------------------
/// The entry as INTERNET_CACHE_ENTRY_INFO(W/A) with its strings after the
/// fixed part; Err(required size) when `capacity` is short.
fn info_bytes(
    root: &str,
    e: &Entry,
    kind: Text,
    base: u64,
    capacity: usize,
) -> Result<Vec<u8>, usize> {
    let file = if e.file.is_empty() {
        None
    } else {
        Some(full(root, &e.file))
    };
    let head = String::from_utf8_lossy(&e.headers).into_owned();
    let strings: [Option<&str>; 4] = [
        Some(&e.url),
        file.as_deref(),
        Some(&head),
        Some(&e.extension),
    ];
    let mut b = vec![0u8; INFO_BYTES];
    let mut tail: Vec<u8> = Vec::new();
    let mut pointers = [0u64; 4];
    for (i, s) in strings.iter().enumerate() {
        if let Some(s) = s {
            // Strings start 2-aligned (W) after the fixed part.
            while !tail.len().is_multiple_of(2) {
                tail.push(0);
            }
            pointers[i] = base + (INFO_BYTES + tail.len()) as u64;
            tail.extend(encode(s, kind));
        }
    }
    let total = INFO_BYTES + tail.len();
    if total > capacity {
        return Err(total);
    }
    for (index, value) in [
        INFO_BYTES as u64,
        pointers[0],
        pointers[1],
        u64::from(e.kind),
        u64::from(e.uses),
        u64::from(e.hits),
        e.size & 0xffff_ffff,
        e.size >> 32,
    ]
    .into_iter()
    .enumerate()
    {
        layout::set(&mut b, &INFO, index, value);
    }
    for (index, time) in [e.modified, e.expires, e.accessed, e.synced]
        .into_iter()
        .enumerate()
    {
        let at = info_at(8 + index);
        b[at..at + 8].copy_from_slice(&time.to_le_bytes());
    }
    layout::set(&mut b, &INFO, 12, pointers[2]);
    // dwHeaderInfoSize: characters, without the NUL.
    let head_units = match kind {
        Text::Wide => head.encode_utf16().count(),
        Text::Ansi => crate::to_ansi(&head).len(),
    };
    layout::set(&mut b, &INFO, 13, head_units as u64);
    layout::set(&mut b, &INFO, 14, pointers[3]);
    b.extend(tail);
    Ok(b)
}
/// Write an entry's information to (info, size): the size needed when the
/// buffer is short (ERROR_INSUFFICIENT_BUFFER).
fn answer_info(root: &str, e: &Entry, info: u64, size: u64, kind: Text) -> Result<(), u32> {
    let capacity = if size == 0 {
        0
    } else {
        read_u32(size)? as usize
    };
    let capacity = if info == 0 { 0 } else { capacity };
    match info_bytes(root, e, kind, info, capacity) {
        Ok(bytes) => {
            write(info, &bytes)?;
            if size != 0 {
                write_u32(size, bytes.len() as u32)?;
            }
            Ok(())
        }
        Err(needed) => {
            if size != 0 {
                write_u32(size, needed as u32)?;
            }
            Err(error::INSUFFICIENT_BUFFER)
        }
    }
}

fn get_info(url: u64, info: u64, size: u64, kind: Text, lock: bool) -> i32 {
    let result = (|| {
        let url = text(url, u32::MAX, kind)?;
        if url.is_empty() {
            return Err(error::INVALID_PARAMETER);
        }
        with_index(|root, index| {
            let Some(at) = index.find(&url) else {
                return (Err(error::FILE_NOT_FOUND), false);
            };
            let answer = answer_info(root, &index.entries[at], info, size, kind);
            if lock && answer.is_ok() {
                let e = &mut index.entries[at];
                e.locks = e.locks.saturating_add(1);
                e.uses = e.uses.saturating_add(1);
                e.accessed = now();
                return (answer, true);
            }
            (answer, false)
        })
        .unwrap_or(Err(error::FILE_NOT_FOUND))
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetUrlCacheEntryInfoW(url: usize, info: usize, size: usize) -> i32 {
    let url = door_windows_abi::widen(url);
    let info = door_windows_abi::widen(info);
    let size = door_windows_abi::widen(size);
    get_info(url, info, size, Text::Wide, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetUrlCacheEntryInfoA(url: usize, info: usize, size: usize) -> i32 {
    let url = door_windows_abi::widen(url);
    let info = door_windows_abi::widen(info);
    let size = door_windows_abi::widen(size);
    get_info(url, info, size, Text::Ansi, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetUrlCacheEntryInfoExW(
    url: usize,
    info: usize,
    size: usize,
    _redirect: usize,
    _redirect_size: usize,
    _reserved: usize,
    _flags: u32,
) -> i32 {
    let url = door_windows_abi::widen(url);
    let info = door_windows_abi::widen(info);
    let size = door_windows_abi::widen(size);
    get_info(url, info, size, Text::Wide, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetUrlCacheEntryInfoExA(
    url: usize,
    info: usize,
    size: usize,
    _redirect: usize,
    _redirect_size: usize,
    _reserved: usize,
    _flags: u32,
) -> i32 {
    let url = door_windows_abi::widen(url);
    let info = door_windows_abi::widen(info);
    let size = door_windows_abi::widen(size);
    get_info(url, info, size, Text::Ansi, false)
}
/// The entry's information, its file locked until UnlockUrlCacheEntryFile.
#[unsafe(no_mangle)]
pub extern "system" fn RetrieveUrlCacheEntryFileW(
    url: usize,
    info: usize,
    size: usize,
    _reserved: u32,
) -> i32 {
    let url = door_windows_abi::widen(url);
    let info = door_windows_abi::widen(info);
    let size = door_windows_abi::widen(size);
    get_info(url, info, size, Text::Wide, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn RetrieveUrlCacheEntryFileA(
    url: usize,
    info: usize,
    size: usize,
    _reserved: u32,
) -> i32 {
    let url = door_windows_abi::widen(url);
    let info = door_windows_abi::widen(info);
    let size = door_windows_abi::widen(size);
    get_info(url, info, size, Text::Ansi, true)
}
fn unlock(url: u64, kind: Text) -> i32 {
    let result = (|| {
        let url = text(url, u32::MAX, kind)?;
        with_index(|_, index| match index.find(&url) {
            Some(at) if index.entries[at].locks > 0 => {
                index.entries[at].locks -= 1;
                (Ok(()), true)
            }
            Some(_) => (Err(error::NOT_LOCKED), false),
            None => (Err(error::FILE_NOT_FOUND), false),
        })
        .unwrap_or(Err(error::FILE_NOT_FOUND))
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn UnlockUrlCacheEntryFileW(url: usize, _reserved: u32) -> i32 {
    let url = door_windows_abi::widen(url);
    unlock(url, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn UnlockUrlCacheEntryFileA(url: usize, _reserved: u32) -> i32 {
    let url = door_windows_abi::widen(url);
    unlock(url, Text::Ansi)
}
fn delete_entry(url: u64, kind: Text) -> i32 {
    let result = (|| {
        let url = text(url, u32::MAX, kind)?;
        with_index(|root, index| match index.find(&url) {
            Some(at) => {
                let e = index.entries.remove(at);
                if !e.file.is_empty() {
                    delete(&full(root, &e.file));
                }
                (Ok(()), true)
            }
            None => (Err(error::FILE_NOT_FOUND), false),
        })
        .unwrap_or(Err(error::FILE_NOT_FOUND))
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn DeleteUrlCacheEntryW(url: usize) -> i32 {
    let url = door_windows_abi::widen(url);
    delete_entry(url, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn DeleteUrlCacheEntryA(url: usize) -> i32 {
    let url = door_windows_abi::widen(url);
    delete_entry(url, Text::Ansi)
}
#[unsafe(no_mangle)]
pub extern "system" fn DeleteUrlCacheEntry(url: usize) -> i32 {
    let url = door_windows_abi::widen(url);
    delete_entry(url, Text::Ansi)
}

/// CreateUrlCacheEntry: a new file in the cache for `url` (MAX_PATH
/// characters into `out`), created empty so the name stays taken.
fn create_entry(url: u64, _expected: u32, extension: u64, out: u64, kind: Text) -> i32 {
    let result = (|| {
        let url = text(url, u32::MAX, kind)?;
        let extension = text(extension, u32::MAX, kind)?;
        if url.is_empty() || out == 0 {
            return Err(error::INVALID_PARAMETER);
        }
        let path = with_index(|root, index| {
            let folder = (now() as usize / 7) % index.folders.len().max(1);
            let name = index.file_name(&url, &extension, folder, &|n: &str| exists(&full(root, n)));
            let path = full(root, &name);
            (write_file(&path, &[]).then_some(path), false)
        })
        .flatten()
        .ok_or(error::FILE_NOT_FOUND)?;
        let bytes = encode(&path, kind);
        if bytes.len() / unit(kind) as usize > 260 {
            return Err(error::INSUFFICIENT_BUFFER);
        }
        write(out, &bytes)
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateUrlCacheEntryW(
    url: usize,
    expected: u32,
    extension: usize,
    out: usize,
    _reserved: u32,
) -> i32 {
    let url = door_windows_abi::widen(url);
    let extension = door_windows_abi::widen(extension);
    let out = door_windows_abi::widen(out);
    create_entry(url, expected, extension, out, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateUrlCacheEntryA(
    url: usize,
    expected: u32,
    extension: usize,
    out: usize,
    _reserved: u32,
) -> i32 {
    let url = door_windows_abi::widen(url);
    let extension = door_windows_abi::widen(extension);
    let out = door_windows_abi::widen(out);
    create_entry(url, expected, extension, out, Text::Ansi)
}
/// CommitUrlCacheEntry: the file CreateUrlCacheEntry gave (or none) under
/// `url`, with times, type and headers.
#[allow(clippy::too_many_arguments)]
fn commit_entry(
    url: u64,
    file: u64,
    expires: u64,
    modified: u64,
    entry_type: u32,
    header: u64,
    header_size: u32,
    extension: u64,
    kind: Text,
) -> i32 {
    let result = (|| {
        let url = text(url, u32::MAX, kind)?;
        let file = text(file, u32::MAX, kind)?;
        let header = if header == 0 || header_size == 0 {
            String::new()
        } else {
            text(header, header_size, kind)?
        };
        let extension = text(extension, u32::MAX, kind)?;
        if url.is_empty() {
            return Err(error::INVALID_PARAMETER);
        }
        with_index(|root, index| {
            let relative_file = if file.is_empty() {
                Some(String::new())
            } else {
                relative(root, &file)
            };
            let Some(relative_file) = relative_file else {
                return (Err(error::INVALID_PARAMETER), false);
            };
            let size = if relative_file.is_empty() {
                0
            } else {
                match read_file(&full(root, &relative_file)) {
                    Some(b) => b.len() as u64,
                    None => return (Err(error::FILE_NOT_FOUND), false),
                }
            };
            let now = now();
            let entry = Entry {
                url: url.clone(),
                file: relative_file,
                kind: if entry_type == 0 {
                    kind::NORMAL
                } else {
                    entry_type
                },
                size,
                modified,
                expires,
                accessed: now,
                synced: now,
                uses: 0,
                hits: 0,
                extension: extension.clone(),
                headers: header.clone().into_bytes(),
                locks: 0,
            };
            match index.commit(entry) {
                Ok(dropped) => {
                    for f in dropped {
                        delete(&full(root, &f));
                    }
                    (Ok(()), true)
                }
                Err(_) => (Err(error::INVALID_PARAMETER), false),
            }
        })
        .unwrap_or(Err(error::FILE_NOT_FOUND))
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn CommitUrlCacheEntryW(
    url: usize,
    file: usize,
    expires: usize,
    modified: usize,
    entry_type: u32,
    header: usize,
    header_size: u32,
    extension: usize,
    _original: usize,
) -> i32 {
    let url = door_windows_abi::widen(url);
    let file = door_windows_abi::widen(file);
    let expires = door_windows_abi::widen(expires);
    let modified = door_windows_abi::widen(modified);
    let header = door_windows_abi::widen(header);
    let extension = door_windows_abi::widen(extension);
    {
        commit_entry(
            url,
            file,
            expires,
            modified,
            entry_type,
            header,
            header_size,
            extension,
            Text::Wide,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CommitUrlCacheEntryA(
    url: usize,
    file: usize,
    expires: usize,
    modified: usize,
    entry_type: u32,
    header: usize,
    header_size: u32,
    extension: usize,
    _original: usize,
) -> i32 {
    let url = door_windows_abi::widen(url);
    let file = door_windows_abi::widen(file);
    let expires = door_windows_abi::widen(expires);
    let modified = door_windows_abi::widen(modified);
    let header = door_windows_abi::widen(header);
    let extension = door_windows_abi::widen(extension);
    {
        commit_entry(
            url,
            file,
            expires,
            modified,
            entry_type,
            header,
            header_size,
            extension,
            Text::Ansi,
        )
    }
}
/// SetUrlCacheEntryInfo: the fields `control` names (CACHE_ENTRY_*_FC).
fn set_info(url: u64, info: u64, control: u32, kind: Text) -> i32 {
    const ATTRIBUTE: u32 = 0x4;
    const HITRATE: u32 = 0x10;
    const MODTIME: u32 = 0x40;
    const EXPTIME: u32 = 0x80;
    const ACCTIME: u32 = 0x100;
    const SYNCTIME: u32 = 0x200;
    let result = (|| {
        let url = text(url, u32::MAX, kind)?;
        let mut b = [0u8; INFO_BYTES];
        read(info, &mut b)?;
        let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let u64_at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().unwrap());
        with_index(|_, index| {
            let Some(at) = index.find(&url) else {
                return (Err(error::FILE_NOT_FOUND), false);
            };
            let e = &mut index.entries[at];
            if control & ATTRIBUTE != 0 {
                e.kind = u32_at(info_at(3));
            }
            if control & HITRATE != 0 {
                e.hits = u32_at(info_at(5));
            }
            if control & MODTIME != 0 {
                e.modified = u64_at(info_at(8));
            }
            if control & EXPTIME != 0 {
                e.expires = u64_at(info_at(9));
            }
            if control & ACCTIME != 0 {
                e.accessed = u64_at(info_at(10));
            }
            if control & SYNCTIME != 0 {
                e.synced = u64_at(info_at(11));
            }
            (Ok(()), true)
        })
        .unwrap_or(Err(error::FILE_NOT_FOUND))
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetUrlCacheEntryInfoW(url: usize, info: usize, control: u32) -> i32 {
    let url = door_windows_abi::widen(url);
    let info = door_windows_abi::widen(info);
    set_info(url, info, control, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetUrlCacheEntryInfoA(url: usize, info: usize, control: u32) -> i32 {
    let url = door_windows_abi::widen(url);
    let info = door_windows_abi::widen(info);
    set_info(url, info, control, Text::Ansi)
}

// ---- enumeration -----------------------------------------------------------------------
/// FindFirstUrlCacheEntry's snapshot: the URLs, and the next to answer.
type Snapshot = Option<(Vec<String>, usize)>;
static FINDS: Lock<Vec<Snapshot>> = Lock::new(Vec::new());
const FIND_BASE: u64 = 0x00CA_0004;
/// Answer the next entry of `urls` from `at` into (info, size); the
/// position after it, or the error (NO_MORE_ITEMS at the end).
fn next_entry(urls: &[String], at: usize, info: u64, size: u64, kind: Text) -> Result<usize, u32> {
    with_index(|root, index| {
        let mut at = at;
        while at < urls.len() {
            if let Some(i) = index.find(&urls[at]) {
                return (
                    answer_info(root, &index.entries[i], info, size, kind).map(|()| at + 1),
                    false,
                );
            }
            // Deleted since FindFirst.
            at += 1;
        }
        (Err(error::NO_MORE_ITEMS), false)
    })
    .unwrap_or(Err(error::NO_MORE_ITEMS))
}
fn find_first(pattern: u64, info: u64, size: u64, kind: Text) -> u64 {
    let result = (|| {
        let pattern = text(pattern, u32::MAX, kind)?;
        let pattern = (!pattern.is_empty()).then_some(pattern);
        let urls = with_index(|_, index| {
            let urls: Vec<String> = index
                .matching(pattern.as_deref())
                .into_iter()
                .map(|i| index.entries[i].url.clone())
                .collect();
            (urls, false)
        })
        .unwrap_or_default();
        let next = next_entry(&urls, 0, info, size, kind)?;
        let handle = FINDS.with(|f| {
            let index = f.iter().position(Option::is_none).unwrap_or_else(|| {
                f.push(None);
                f.len() - 1
            });
            f[index] = Some((urls, next));
            FIND_BASE + index as u64 * 4
        });
        Ok(handle)
    })();
    match result {
        Ok(h) => {
            unsafe { super::RtlSetLastWin32Error(0) };
            h
        }
        Err(e) => fail(e, 0),
    }
}
fn find_slot(handle: u64) -> Option<usize> {
    handle
        .checked_sub(FIND_BASE)
        .filter(|v| v % 4 == 0)
        .map(|v| (v / 4) as usize)
}
fn find_next(handle: u64, info: u64, size: u64, kind: Text) -> i32 {
    let result = (|| {
        let slot = find_slot(handle).ok_or(error::INVALID_HANDLE)?;
        let (urls, at) = FINDS
            .with(|f| f.get(slot).and_then(|s| s.clone()))
            .ok_or(error::INVALID_HANDLE)?;
        let next = next_entry(&urls, at, info, size, kind)?;
        FINDS.with(|f| {
            if let Some(Some(s)) = f.get_mut(slot) {
                s.1 = next;
            }
        });
        Ok(())
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn FindFirstUrlCacheEntryW(pattern: usize, info: usize, size: usize) -> usize {
    let pattern = door_windows_abi::widen(pattern);
    let info = door_windows_abi::widen(info);
    let size = door_windows_abi::widen(size);
    door_windows_abi::narrow((move || -> u64 {
        find_first(pattern, info, size, Text::Wide)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn FindFirstUrlCacheEntryA(pattern: usize, info: usize, size: usize) -> usize {
    let pattern = door_windows_abi::widen(pattern);
    let info = door_windows_abi::widen(info);
    let size = door_windows_abi::widen(size);
    door_windows_abi::narrow((move || -> u64 {
        find_first(pattern, info, size, Text::Ansi)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn FindNextUrlCacheEntryW(handle: usize, info: usize, size: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let info = door_windows_abi::widen(info);
    let size = door_windows_abi::widen(size);
    find_next(handle, info, size, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn FindNextUrlCacheEntryA(handle: usize, info: usize, size: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let info = door_windows_abi::widen(info);
    let size = door_windows_abi::widen(size);
    find_next(handle, info, size, Text::Ansi)
}
#[unsafe(no_mangle)]
pub extern "system" fn FindCloseUrlCache(handle: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    {
        let closed = find_slot(handle)
            .is_some_and(|slot| FINDS.with(|f| f.get_mut(slot).and_then(Option::take).is_some()));
        if closed {
            done(Ok(()))
        } else {
            fail(error::INVALID_HANDLE, 0)
        }
    }
}
