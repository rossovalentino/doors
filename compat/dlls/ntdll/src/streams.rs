//! Alternate data streams (`file.txt:stream`): a file's named streams are
//! small files of their own, named by the file's storage object ID
//! (`door_service_protocol::streams`), so they follow it through renames
//! and moves. CreateFile, DeleteFile and CopyFile reach them through
//! `files`; FindFirstStreamW lists them.
use super::files::{PATH_TEXT, PATH_UNITS};
use super::files::{Table, native_path, path_text, read_path, service};
use super::guest::{RtlSetLastWin32Error, copy};
use core::cell::UnsafeCell;
use door_service_client::{Disposition, Storage};
use door_service_protocol::{CHUNK_BYTES, streams};

fn failed(error: u32) -> i32 {
    RtlSetLastWin32Error(error);
    0
}
fn upcase(unit: u16) -> u16 {
    door_pe_loader::resources::upcase(unit)
}
/// `name:stream[:$DATA]` in `path`'s last component: the base path's length
/// and the stream's name (empty for `name::$DATA`, the file itself).
pub(super) fn split(path: &[u16]) -> Result<Option<(usize, &[u16])>, u32> {
    let start = path
        .iter()
        .rposition(|u| matches!(u, 47 | 92))
        .map_or(0, |i| i + 1);
    // A drive's colon ("C:name") is not a stream's.
    let from = if start == 0 && path.len() > 1 && path[1] == 58 {
        2
    } else {
        start
    };
    let Some(colon) = path[from..].iter().position(|u| *u == 58).map(|i| from + i) else {
        return Ok(None);
    };
    if colon == from {
        return Err(123);
    }
    let rest = &path[colon + 1..];
    let (name, kind) = match rest.iter().position(|u| *u == 58) {
        Some(i) => (&rest[..i], Some(&rest[i + 1..])),
        None => (rest, None),
    };
    if let Some(kind) = kind {
        let data = [36u16, 68, 65, 84, 65]; // $DATA
        if kind.len() != data.len() || !kind.iter().zip(data).all(|(a, b)| upcase(*a) == b) {
            return Err(123);
        }
    }
    if name.is_empty() && kind.is_none() {
        return Err(123);
    }
    Ok(Some((colon, name)))
}
/// ASCII stream name `units` as text in `out`.
fn name<'a>(units: &[u16], out: &'a mut [u8; streams::NAME_MAX]) -> Result<&'a str, u32> {
    if units.len() > out.len() {
        return Err(123);
    }
    for (o, u) in out.iter_mut().zip(units) {
        *o = u8::try_from(*u).ok().filter(u8::is_ascii).ok_or(123u32)?;
    }
    let text = core::str::from_utf8(&out[..units.len()]).map_err(|_| 123u32)?;
    if !streams::valid_name(text) {
        return Err(123);
    }
    Ok(text)
}
/// The storage path of stream `stream` of the file at `base` (a path as
/// `native_path` takes it), into `out`; with `create`, a missing file is
/// made first (Windows creates `new.txt` for `new.txt:s`) and so is the
/// streams folder.
pub(super) fn file(
    table: &mut Table,
    base: &[u16],
    stream: &[u16],
    create: bool,
    out: &mut [u8; PATH_TEXT],
) -> Result<usize, u32> {
    let mut buffer = [0u8; streams::NAME_MAX];
    let stream = name(stream, &mut buffer)?;
    let mut text = [0u8; PATH_TEXT];
    let length = native_path(table, base, &mut text)?;
    let base_text = path_text(&text[..length])?;
    let storage = table.storage.as_mut().ok_or(1117u32)?;
    let object = match storage.metadata(base_text) {
        Ok(info) if info.directory => return Err(5),
        Ok(info) => info.object,
        Err(e) => {
            let e = service(e);
            if !(create && e == 2) {
                return Err(e);
            }
            let file = storage
                .open(base_text, Disposition::CreateNew, 3, 7)
                .map_err(service)?;
            let info = storage.info(&file).map_err(service);
            let _ = storage.close(file);
            info?.object
        }
    };
    let uid = super::guest::current_uid();
    if create {
        let mut folder = [0u8; 96];
        for level in 0..streams::FOLDERS.len() {
            if let Some(k) = streams::folder(uid, level, &mut folder) {
                let _ = storage.mkdir(path_text(&folder[..k])?);
            }
        }
    }
    streams::path(uid, object, stream, out).ok_or(206)
}
type Found = [([u8; streams::NAME_MAX], usize, u64); 8];
/// The named streams of storage object `object`: (stream name, bytes).
fn of(storage: &mut Storage, object: u64) -> (Found, usize) {
    let mut found = [([0u8; streams::NAME_MAX], 0usize, 0u64); 8];
    let mut count = 0;
    let uid = super::guest::current_uid();
    let mut folder = [0u8; 96];
    let Some(k) = streams::folder(uid, 1, &mut folder) else {
        return (found, 0);
    };
    let Ok(folder) = path_text(&folder[..k]) else {
        return (found, 0);
    };
    let mut cursor = 0;
    while count < found.len() {
        let Ok(Some(entry)) = storage.list(folder, cursor) else {
            break;
        };
        cursor = entry.cookie;
        let units = &entry.name[..entry.name_len];
        let mut text = [0u8; 64];
        if units.len() > text.len() || units.iter().any(|u| *u > 127) {
            continue;
        }
        for (o, u) in text.iter_mut().zip(units) {
            *o = *u as u8;
        }
        let Ok(text) = core::str::from_utf8(&text[..units.len()]) else {
            continue;
        };
        if let Some(stream) = streams::stream_of(text, object) {
            found[count].0[..stream.len()].copy_from_slice(stream.as_bytes());
            found[count].1 = stream.len();
            found[count].2 = entry.info.bytes;
            count += 1;
        }
    }
    (found, count)
}
/// The stream files of `object`, removed (its file was deleted).
pub(super) fn remove_all(storage: &mut Storage, object: u64) {
    let (found, count) = of(storage, object);
    let uid = super::guest::current_uid();
    for (stream, len, _) in &found[..count] {
        let mut path = [0u8; PATH_TEXT];
        if let Ok(stream) = core::str::from_utf8(&stream[..*len])
            && let Some(n) = streams::path(uid, object, stream, &mut path)
            && let Ok(p) = path_text(&path[..n])
        {
            let _ = storage.remove(p);
        }
    }
}
/// CopyFile copies a file's named streams too, as on NTFS.
pub(super) fn copy_all(storage: &mut Storage, from: u64, to: u64) {
    let (found, count) = of(storage, from);
    let uid = super::guest::current_uid();
    for (stream, len, _) in &found[..count] {
        let Ok(stream) = core::str::from_utf8(&stream[..*len]) else {
            continue;
        };
        let (mut source, mut target) = ([0u8; PATH_TEXT], [0u8; PATH_TEXT]);
        let (Some(a), Some(b)) = (
            streams::path(uid, from, stream, &mut source),
            streams::path(uid, to, stream, &mut target),
        ) else {
            continue;
        };
        let (Ok(a), Ok(b)) = (path_text(&source[..a]), path_text(&target[..b])) else {
            continue;
        };
        let Ok(input) = storage.open(a, Disposition::OpenExisting, 1, 1) else {
            continue;
        };
        if let Ok(output) = storage.open(b, Disposition::CreateAlways, 2, 0) {
            let mut chunk = [0u8; CHUNK_BYTES];
            let mut offset = 0u64;
            while let Ok(n) = storage.read(&input, offset, &mut chunk) {
                if n == 0 || storage.write(&output, offset, &chunk[..n]).is_err() {
                    break;
                }
                offset += n as u64;
            }
            let _ = storage.close(output);
        }
        let _ = storage.close(input);
    }
}

const TAG: door_windows_abi::tagged::Tag = door_windows_abi::tagged::STREAM_SEARCH;
/// One FindFirstStreamW search: its entries (`::$DATA` first), the next one.
struct Search {
    generation: u32,
    live: bool,
    names: [([u8; streams::NAME_MAX], usize, u64); 9],
    count: usize,
    next: usize,
}
impl Search {
    const fn new() -> Self {
        Self {
            generation: 0,
            live: false,
            names: [([0; streams::NAME_MAX], 0, 0); 9],
            count: 0,
            next: 0,
        }
    }
}
struct Searches(UnsafeCell<[Search; 2]>);
unsafe impl Sync for Searches {}
static SEARCHES: Searches = Searches(UnsafeCell::new([const { Search::new() }; 2]));
/// WIN32_FIND_STREAM_DATA: StreamSize, then `:name:$DATA` in 296 units.
fn write_data(destination: u64, stream: &[u8], bytes: u64) -> Result<(), u32> {
    let mut data = [0u8; 8 + 296 * 2];
    data[..8].copy_from_slice(&bytes.to_le_bytes());
    let mut at = 8;
    for b in b":".iter().chain(stream).chain(b":$DATA") {
        data[at..at + 2].copy_from_slice(&u16::from(*b).to_le_bytes());
        at += 2;
    }
    for (index, chunk) in data.chunks(512).enumerate() {
        copy(
            destination + index as u64 * 512,
            chunk.as_ptr() as u64,
            chunk.len(),
        )
        .map_err(|_| 998u32)?;
    }
    Ok(())
}
/// `FindFirstStreamW` (FindStreamInfoStandard): a file's unnamed stream
/// (its data), then each named stream; ERROR_HANDLE_EOF when there is none.
#[unsafe(no_mangle)]
pub extern "system" fn DoorFindFirstStreamW(
    path: u64,
    level: u32,
    destination: u64,
    flags: u32,
) -> u64 {
    let result = super::files::with_table(|table| {
        if level != 0 || flags != 0 {
            return Err(87);
        }
        let mut units = [0u16; PATH_UNITS];
        let length = read_path(path, &mut units)?;
        let mut text = [0u8; PATH_TEXT];
        let n = native_path(table, &units[..length], &mut text)?;
        let storage = table.storage.as_mut().ok_or(1117u32)?;
        let info = storage.metadata(path_text(&text[..n])?).map_err(service)?;
        // SAFETY: the storage lease (held by with_table) serializes searches.
        let searches = unsafe { &mut *SEARCHES.0.get() };
        let index = searches
            .iter()
            .position(|s| !s.live && s.generation != u32::MAX)
            .ok_or(4u32)?;
        let search = &mut searches[index];
        search.count = 0;
        if !info.directory {
            search.names[0] = ([0; streams::NAME_MAX], 0, info.bytes);
            search.count = 1;
        }
        let (found, count) = of(storage, info.object);
        for entry in &found[..count] {
            search.names[search.count] = *entry;
            search.count += 1;
        }
        if search.count == 0 {
            return Err(38); // ERROR_HANDLE_EOF
        }
        let (stream, len, bytes) = search.names[0];
        write_data(destination, &stream[..len], bytes)?;
        search.next = 1;
        search.live = true;
        search.generation += 1;
        Ok(TAG.encode(index, search.generation))
    });
    result.unwrap_or_else(|error| {
        failed(error);
        u64::MAX
    })
}
pub(super) fn is_search(handle: u64) -> bool {
    TAG.is(handle)
}
fn search(handle: u64) -> Result<&'static mut Search, u32> {
    let (index, generation) = TAG.decode(handle).ok_or(6u32)?;
    // SAFETY: callers hold the storage lease.
    let searches = unsafe { &mut *SEARCHES.0.get() };
    let s = searches.get_mut(index).ok_or(6u32)?;
    if !s.live || !door_windows_abi::tagged::same(s.generation, generation) {
        return Err(6);
    }
    Ok(s)
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorFindNextStreamW(handle: u64, destination: u64) -> i32 {
    let result = super::files::with_table(|_| {
        let s = search(handle)?;
        if s.next >= s.count {
            return Err(38);
        }
        let (stream, len, bytes) = s.names[s.next];
        s.next += 1;
        write_data(destination, &stream[..len], bytes)
    });
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}
pub(super) fn close_search(handle: u64) -> Result<(), u32> {
    super::files::with_table(|_| {
        search(handle)?.live = false;
        Ok(())
    })
}
