//! Directory handles (P18): CreateFile of a folder with
//! FILE_FLAG_BACKUP_SEMANTICS, as hosts and runtimes open folders to
//! resolve their final paths (GetFinalPathNameByHandle) or query them
//! (GetFileInformationByHandle). A handle keeps the folder's normalized
//! path as storage names it (`C:\a\b`, or `/a/b` for Doors' own folders);
//! the folder itself is looked up again at each query.
use super::guest::{RtlSetLastWin32Error, copy};
use core::{cell::UnsafeCell, sync::atomic::AtomicBool, sync::atomic::Ordering};
use door_windows_abi::tagged::{DIRECTORY, same};

const SLOTS: usize = 64;
const TEXT: usize = super::files::PATH_TEXT;

#[derive(Clone, Copy)]
struct Directory {
    path: [u8; TEXT],
    length: usize,
    /// NtQueryDirectoryFile's position: the last object ID returned.
    cursor: u64,
}
struct Table {
    slots: [Option<Directory>; SLOTS],
    generations: [u32; SLOTS],
}
struct Shared {
    held: AtomicBool,
    table: UnsafeCell<Table>,
}
unsafe impl Sync for Shared {}
static TABLE: Shared = Shared {
    held: AtomicBool::new(false),
    table: UnsafeCell::new(Table {
        slots: [None; SLOTS],
        generations: [0; SLOTS],
    }),
};
fn with<R>(f: impl FnOnce(&mut Table) -> R) -> R {
    while TABLE.held.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    // SAFETY: `held` gives this closure the only access.
    let r = f(unsafe { &mut *TABLE.table.get() });
    TABLE.held.store(false, Ordering::Release);
    r
}
fn failed(error: u32) -> i32 {
    RtlSetLastWin32Error(error);
    0
}
fn find(table: &Table, handle: u64) -> Result<usize, u32> {
    let (slot, generation) = DIRECTORY
        .decode(handle)
        .filter(|&(s, _)| s < SLOTS)
        .ok_or(6u32)?;
    if table.slots[slot].is_none() || !same(table.generations[slot], generation) {
        return Err(6);
    }
    Ok(slot)
}
fn path_of(handle: u64) -> Result<Directory, u32> {
    with(|t| find(t, handle).map(|s| t.slots[s].unwrap()))
}

/// Opens the folder `path` (a NUL-terminated UTF-16 pointer): a directory
/// handle, or u64::MAX with ERROR_FILE_NOT_FOUND / ERROR_PATH_NOT_FOUND,
/// or ERROR_DIRECTORY (267) when `path` is a file (CreateFile then opens
/// the file instead).
#[unsafe(no_mangle)]
pub extern "system" fn DoorOpenDirectory(path: u64) -> u64 {
    let result = super::files::with_native(path, |storage, native| {
        let info = storage
            .metadata(native)
            .map_err(|e| super::files::path_error(storage, native, e))?;
        if !info.directory {
            return Err(267);
        }
        let mut d = Directory {
            path: [0; TEXT],
            length: native.len(),
            cursor: 0,
        };
        d.path[..native.len()].copy_from_slice(native.as_bytes());
        Ok(d)
    })
    .and_then(|d| {
        with(|t| {
            let slot = t.slots.iter().position(Option::is_none).ok_or(4u32)?;
            let generation = t.generations[slot].wrapping_add(1).max(1);
            t.generations[slot] = generation;
            t.slots[slot] = Some(d);
            Ok(DIRECTORY.encode(slot, generation))
        })
    });
    match result {
        Ok(handle) => {
            RtlSetLastWin32Error(0);
            handle
        }
        Err(error) => {
            RtlSetLastWin32Error(error);
            u64::MAX
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorCloseDirectory(handle: u64) -> i32 {
    match with(|t| find(t, handle).map(|s| t.slots[s] = None)) {
        Ok(()) => 1,
        Err(e) => failed(e),
    }
}
/// The folder's path in UTF-16 (`C:\a\b`; `/a/b` for Doors' own folders),
/// as DoorGetFilePath: its length, or the units needed with the NUL.
#[unsafe(no_mangle)]
pub extern "system" fn DoorDirectoryPath(handle: u64, out: u64, capacity: u32) -> u32 {
    let d = match path_of(handle) {
        Ok(d) => d,
        Err(e) => {
            failed(e);
            return 0;
        }
    };
    let Ok(text) = core::str::from_utf8(&d.path[..d.length]) else {
        failed(123);
        return 0;
    };
    let length = text.encode_utf16().count();
    if capacity as usize <= length {
        return length as u32 + 1;
    }
    for (i, unit) in text.encode_utf16().chain([0]).enumerate() {
        if copy(out + 2 * i as u64, &unit as *const u16 as u64, 2).is_err() {
            failed(998);
            return 0;
        }
    }
    length as u32
}
/// As DoorFileInformation, for the folder (looked up again by path).
#[unsafe(no_mangle)]
pub extern "system" fn DoorDirectoryInformation(handle: u64, out: u64) -> i32 {
    let result = path_of(handle).and_then(|d| {
        let text = core::str::from_utf8(&d.path[..d.length]).map_err(|_| 123u32)?;
        super::files::with_storage(|s| s.stat(text).map_err(super::files::service))
    });
    match result {
        Ok((info, times)) => {
            let mut record = [0u8; 48];
            record[8..16].copy_from_slice(&info.object.to_le_bytes());
            record[16..20].copy_from_slice(&0x10u32.to_le_bytes());
            record[20..24].copy_from_slice(&1u32.to_le_bytes());
            record[24..32].copy_from_slice(&times.created.to_le_bytes());
            record[32..40].copy_from_slice(&times.accessed.to_le_bytes());
            record[40..48].copy_from_slice(&times.modified.to_le_bytes());
            match copy(out, record.as_ptr() as u64, 48) {
                Ok(()) => 1,
                Err(_) => failed(998),
            }
        }
        Err(e) => failed(e),
    }
}

// ---- NtQueryDirectoryFile ----------------------------------------------------------------
const STATUS_SUCCESS: u32 = 0;
const STATUS_NO_MORE_FILES: u32 = 0x8000_0006;
const STATUS_BUFFER_OVERFLOW: u32 = 0x8000_0005;
const STATUS_INVALID_HANDLE: u32 = 0xC000_0008;
const STATUS_INVALID_PARAMETER: u32 = 0xC000_000D;
const STATUS_INVALID_INFO_CLASS: u32 = 0xC000_0003;
const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;
const STATUS_BUFFER_TOO_SMALL: u32 = 0xC000_0023;
const STATUS_UNSUCCESSFUL: u32 = 0xC000_0001;

/// Where a class puts the name, and whether it carries the 8.3 name and
/// the file ID (FileDirectoryInformation 1, FileFullDirectoryInformation
/// 2, FileBothDirectoryInformation 3, FileNamesInformation 12,
/// FileIdBothDirectoryInformation 37, FileIdFullDirectoryInformation 38).
fn class_layout(class: u32) -> Option<usize> {
    Some(match class {
        1 => 64,
        2 => 68,
        3 => 94,
        12 => 12,
        37 => 104,
        38 => 80,
        _ => return None,
    })
}
/// One entry's record (name in UTF-16): its length (8-aligned is the
/// caller's job).
fn record(class: u32, entry: &door_service_protocol::files::DirEntry, out: &mut [u8]) -> usize {
    let name_at = class_layout(class).unwrap();
    let name = &entry.name[..entry.name_len];
    let length = name_at + 2 * name.len();
    out[..length].fill(0);
    let put32 = |b: &mut [u8], at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    let put64 = |b: &mut [u8], at: usize, v: u64| b[at..at + 8].copy_from_slice(&v.to_le_bytes());
    let attributes = if entry.info.directory { 0x10 } else { 0x80 };
    if class == 12 {
        put32(out, 8, (2 * name.len()) as u32);
    } else {
        put64(out, 8, entry.times.created);
        put64(out, 16, entry.times.accessed);
        put64(out, 24, entry.times.modified);
        put64(out, 32, entry.times.modified);
        let bytes = if entry.info.directory {
            0
        } else {
            entry.info.bytes
        };
        put64(out, 40, bytes);
        put64(out, 48, bytes.div_ceil(4096) * 4096);
        put32(out, 56, attributes);
        put32(out, 60, (2 * name.len()) as u32);
        match class {
            37 => put64(out, 96, entry.info.object),
            38 => put64(out, 72, entry.info.object),
            _ => (),
        }
    }
    for (i, &u) in name.iter().enumerate() {
        out[name_at + 2 * i..name_at + 2 * i + 2].copy_from_slice(&u.to_le_bytes());
    }
    length
}
/// NtQueryDirectoryFile(handle, event, apc, apc context, IO_STATUS_BLOCK,
/// buffer, length, class, single, file name, restart) on a directory
/// handle: entries in object-ID order, synchronously (the event and APC
/// are not used), every name matching (a pattern other than `*` is not
/// applied).
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn NtQueryDirectoryFile(
    handle: usize,
    _event: usize,
    _apc: usize,
    _apc_context: usize,
    status_block: usize,
    buffer: usize,
    length: u32,
    class: u32,
    single: u8,
    _name: usize,
    restart: u8,
) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let status_block = door_windows_abi::widen(status_block);
    let buffer = door_windows_abi::widen(buffer);
    let Some(name_at) = class_layout(class) else {
        return STATUS_INVALID_INFO_CLASS;
    };
    if buffer == 0 || status_block == 0 {
        return STATUS_INVALID_PARAMETER;
    }
    let Ok(mut d) = path_of(handle) else {
        return STATUS_INVALID_HANDLE;
    };
    if restart != 0 {
        d.cursor = 0;
    }
    let Ok(text) = core::str::from_utf8(&d.path[..d.length]) else {
        return STATUS_UNSUCCESSFUL;
    };
    let mut out = [0u8; 4096];
    let mut used = 0usize;
    let mut last: Option<usize> = None;
    let mut status = STATUS_SUCCESS;
    let capacity = length as usize;
    loop {
        let next =
            super::files::with_storage(|s| s.list(text, d.cursor).map_err(super::files::service));
        let entry = match next {
            Ok(Some(e)) => e,
            Ok(None) => {
                if used == 0 {
                    status = STATUS_NO_MORE_FILES;
                }
                break;
            }
            Err(_) => {
                if used == 0 {
                    status = STATUS_UNSUCCESSFUL;
                }
                break;
            }
        };
        let start = used.next_multiple_of(8);
        let size = name_at + 2 * entry.name_len;
        if start + size > capacity || start + size > out.len() {
            if used == 0 {
                status = if capacity >= name_at {
                    STATUS_BUFFER_OVERFLOW
                } else {
                    STATUS_BUFFER_TOO_SMALL
                };
            }
            break;
        }
        let n = record(class, &entry, &mut out[start..]);
        if let Some(previous) = last {
            out[previous..previous + 4].copy_from_slice(&((start - previous) as u32).to_le_bytes());
        }
        last = Some(start);
        used = start + n;
        d.cursor = entry.info.object;
        if single != 0 {
            break;
        }
    }
    if used > 0 && super::guest::copy_bulk(buffer, out.as_ptr() as u64, used).is_err() {
        return STATUS_ACCESS_VIOLATION;
    }
    with(|t| {
        if let Ok(s) = find(t, handle)
            && let Some(slot) = t.slots[s].as_mut()
        {
            slot.cursor = d.cursor;
        }
    });
    // IO_STATUS_BLOCK: the status (pointer-sized), then the bytes written.
    let pointer = core::mem::size_of::<usize>();
    let mut block = [0u8; 16];
    block[..4].copy_from_slice(&status.to_le_bytes());
    block[pointer..pointer + 4].copy_from_slice(&(used as u32).to_le_bytes());
    let _ = super::guest::copy(status_block, block.as_ptr() as u64, 2 * pointer);
    status
}
