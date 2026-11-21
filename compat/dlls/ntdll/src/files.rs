//! Win32 file operations use the existing authenticated storage owner.
//! Captured data is owned across IPC; no application pointer becomes a Rust slice.
use super::guest::{RtlSetLastWin32Error, copy};
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
};
use door_service_client::{Disposition, Error, File, Storage};
use door_service_protocol::{CHUNK_BYTES, Code};
/// MAX_PATH: a path of at most 259 UTF-16 units and its NUL.
pub(super) const PATH_UNITS: usize = 260;
/// A path in storage's UTF-8 (`door_service_protocol::PATH_BYTES`).
pub(super) const PATH_TEXT: usize = door_service_protocol::PATH_BYTES;
use door_windows_abi::{error, file as handles};

struct Open {
    file: File,
    position: u64,
    access: u16,
    asynchronous: bool,
    closing: bool,
    /// File mappings writing back through this file (`pin`): CloseHandle
    /// hides the handle and the file stays open until the last unpin.
    pins: u16,
    /// The path it was opened by (GetFinalPathNameByHandle, delete on close).
    path: [u16; PATH_UNITS],
    path_length: u16,
    /// FILE_FLAG_DELETE_ON_CLOSE or FileDispositionInfo.
    delete_on_close: bool,
}
struct Pending {
    request: door_windows_abi::io::Request,
    handle: u64,
    overlapped: u64,
    destination: u64,
    offset: u64,
    length: usize,
    writing: bool,
    data: [u8; 1024],
    event: u64,
    /// The OVERLAPPED event's low bit: no completion packet.
    quiet: bool,
    worker: u64,
    error: u32,
    transferred: u32,
}
impl Pending {
    const fn new() -> Self {
        Self {
            request: door_windows_abi::io::Request::new(),
            handle: 0,
            overlapped: 0,
            destination: 0,
            offset: 0,
            length: 0,
            writing: false,
            data: [0; 1024],
            event: 0,
            quiet: false,
            worker: 0,
            error: 0,
            transferred: 0,
        }
    }
}
struct Slot {
    generation: u32,
    entry: Option<Open>,
}
pub(super) struct Table {
    pub(super) storage: Option<Storage>,
    cwd: [u16; PATH_UNITS],
    cwd_length: usize,
    slots: [Slot; handles::SLOTS],
    pending: Pending,
    finds: [Find; FIND_SLOTS],
    /// The bounce section for bulk file data: (handle, read-write view).
    bounce: Option<(u64, u64)>,
}
struct State {
    owner: AtomicU32,
    table: UnsafeCell<Table>,
}
unsafe impl Sync for State {}
static STATE: State = State {
    owner: AtomicU32::new(0),
    table: UnsafeCell::new(Table {
        storage: None,
        cwd: [0; PATH_UNITS],
        cwd_length: 0,
        pending: Pending::new(),
        finds: [const { Find::new() }; FIND_SLOTS],
        bounce: None,
        slots: [const {
            Slot {
                generation: 0,
                entry: None,
            }
        }; handles::SLOTS],
    }),
};
// Cancellation never waits for the serialized storage IPC lease. Monotonic
// generations prevent a delayed cancel from targeting/reviving a recycled slot.
static LIVE_HANDLES: [AtomicU64; handles::SLOTS] = [const { AtomicU64::new(0) }; handles::SLOTS];
static ACTIVE_GENERATION: AtomicU64 = AtomicU64::new(0);
static ACTIVE_HANDLE: AtomicU64 = AtomicU64::new(0);
static ACTIVE_OVERLAPPED: AtomicU64 = AtomicU64::new(0);
static CANCELLED_GENERATION: AtomicU64 = AtomicU64::new(0);
/// The file table and its storage connection, one thread at a time. A
/// holder may be in a storage request (a bulk read of a large file takes
/// a while), so a waiter gives up only after 30 seconds (ERROR_TIMEOUT),
/// which only a stuck holder reaches.
struct Lease;
impl Lease {
    fn acquire() -> Result<Self, u32> {
        let id = super::threads::DoorGetCurrentThreadId();
        let until = door_sys::time().map_err(|_| 1117u32)?.saturating_add(
            door_abi::platform::ticks_from_millis(30_000).expect("constant duration"),
        );
        loop {
            match STATE
                .owner
                .compare_exchange(0, id, Ordering::Acquire, Ordering::Relaxed)
            {
                Ok(_) => return Ok(Self),
                Err(owner) if owner == id => return Err(1117),
                Err(_) => {
                    if door_sys::time().map_err(|_| 1117u32)? >= until {
                        return Err(1460);
                    }
                    door_sys::yield_now().map_err(|_| 1117u32)?;
                }
            }
        }
    }
    fn value(&mut self) -> &mut Table {
        unsafe { &mut *STATE.table.get() }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        STATE.owner.store(0, Ordering::Release);
    }
}
pub(super) fn service(error: Error) -> u32 {
    match error {
        Error::Service(Code::Denied) => 5,
        Error::Service(Code::NotFound) => 2,
        Error::Service(Code::Exists) => 80,
        Error::Service(Code::InvalidHandle) => 6,
        Error::Service(Code::SharingViolation) => 32,
        Error::Service(Code::DeletePending) => 303,
        Error::Service(Code::Resources) => 8,
        Error::Service(Code::InvalidRequest) => 87,
        Error::Service(Code::Unsupported) => 50,
        Error::Service(Code::StaleInstance | Code::Closing) => 995,
        Error::Transport(door_abi::Status::AccessDenied) => 5,
        _ => 1117,
    }
}
fn failed(error: u32) -> i32 {
    RtlSetLastWin32Error(error);
    0
}
pub fn initialize(registry: u64, cwd: Option<&str>) -> Result<(), u32> {
    let mut lease = Lease::acquire()?;
    if lease.value().storage.is_some() {
        return Err(87);
    }
    if let Some(cwd) = cwd {
        if !cwd.starts_with('/') {
            return Err(87);
        }
        let units = cwd.encode_utf16().count();
        if units >= PATH_UNITS {
            return Err(206);
        }
        for (out, unit) in lease.value().cwd.iter_mut().zip(cwd.encode_utf16()) {
            *out = unit;
        }
        lease.value().cwd_length = units;
    }
    lease.value().storage = Some(Storage::connect(registry).map_err(service)?);
    Ok(())
}
fn slot(table: &Table, handle: u64) -> Result<usize, u32> {
    let (index, generation) = handles::decode(handle).ok_or(6u32)?;
    let slot = &table.slots[index];
    if !door_windows_abi::tagged::same(slot.generation, generation)
        || slot.entry.as_ref().is_none_or(|file| file.closing)
    {
        return Err(6);
    }
    Ok(index)
}
fn count(pointer: u64, value: u32) -> Result<(), u32> {
    copy(pointer, &value as *const u32 as u64, 4).map_err(|_| error::NOACCESS)
}

/// The console's device names (`CONIN$`, `CONOUT$`, `CON`, also as
/// `\\.\` names): the console input or active screen buffer, as Windows
/// opens them for a program attached to a console (PowerShell's host opens
/// CONOUT$ for the active screen buffer). `CON` is the input when opened
/// for reading only, else the output.
fn console_device(units: &[u16], access: u32) -> Option<u64> {
    let mut name = [0u8; 12];
    let units = match units {
        [92, 92, 46, 92, rest @ ..] => rest,
        _ => units,
    };
    if units.len() > name.len() {
        return None;
    }
    for (out, &unit) in name.iter_mut().zip(units) {
        *out = u8::try_from(unit).ok()?.to_ascii_uppercase();
    }
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_WRITE_DATA: u32 = 0x2;
    match &name[..units.len()] {
        b"CONIN$" => Some(door_windows_abi::INPUT_HANDLE),
        b"CONOUT$" => Some(door_windows_abi::OUTPUT_HANDLE),
        b"CON" if access & (GENERIC_WRITE | FILE_WRITE_DATA) == 0 => {
            Some(door_windows_abi::INPUT_HANDLE)
        }
        b"CON" => Some(door_windows_abi::OUTPUT_HANDLE),
        // The NUL device (Node reopens closed standard streams on it).
        b"NUL" => Some(door_windows_abi::NUL_HANDLE),
        _ => None,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorCreateFileW(
    path: u64,
    access: u32,
    share: u32,
    security: u64,
    disposition: u32,
    flags: u32,
    template: u64,
) -> u64 {
    let result = (|| {
        // Security attributes (inheritance, a new file's descriptor) and a
        // template are not applied here; every FILE_ATTRIBUTE_* and
        // FILE_FLAG_* / SECURITY_* bit is accepted. FILE_FLAG_OVERLAPPED
        // makes the handle asynchronous and FILE_FLAG_DELETE_ON_CLOSE removes
        // the file at its last close; caching hints have nothing to change
        // (writes reach storage when WriteFile returns), and Doors has no
        // reparse points to open.
        let _ = (security, template);
        // Windows' access rights: the file-specific and standard rights,
        // the generic ones and MAXIMUM_ALLOWED. ACCESS_SYSTEM_SECURITY needs
        // SeSecurityPrivilege, which Doors' tokens never have enabled.
        if access & 0x0100_0000 != 0 {
            return Err(1314);
        }
        if access & !(0xF000_0000 | 0x0200_0000 | 0x001F_01FF) != 0 || share & !7 != 0 {
            return Err(87);
        }
        let disposition = match disposition {
            1 => Disposition::CreateNew,
            2 => Disposition::CreateAlways,
            3 => Disposition::OpenExisting,
            4 => Disposition::OpenAlways,
            5 => Disposition::TruncateExisting,
            _ => return Err(87),
        };
        let mut units = [0u16; PATH_UNITS];
        let mut length = None;
        for (index, out) in units.iter_mut().enumerate() {
            let pointer = path.checked_add(index as u64 * 2).ok_or(998u32)?;
            let mut bytes = [0u8; 2];
            copy(bytes.as_mut_ptr() as u64, pointer, 2).map_err(|_| 998u32)?;
            *out = u16::from_le_bytes(bytes);
            if *out == 0 {
                length = Some(index);
                break;
            }
        }
        // Past MAX_PATH: ERROR_PATH_NOT_FOUND, as Windows 11 answers.
        let mut length = length.ok_or(3u32)?;
        if length == 0 {
            return Err(123);
        }
        if let Some(console) = console_device(&units[..length], access) {
            return Ok(console);
        }
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        if table.cwd_length != 0 && !matches!(units[0], 47 | 92) && !(length > 1 && units[1] == 58)
        {
            let prefix = table.cwd_length;
            if prefix + 1 + length > units.len() {
                return Err(206);
            }
            units.copy_within(0..length, prefix + 1);
            units[..prefix].copy_from_slice(&table.cwd[..prefix]);
            units[prefix] = 47;
            length += prefix + 1;
        }
        let (opened, opened_length) = (units, length);
        let index = table
            .slots
            .iter()
            .position(|slot| slot.entry.is_none() && slot.generation != u32::MAX)
            .ok_or(8u32)?;
        // Storage's read, write and delete: reading or executing the data
        // (GENERIC_READ/EXECUTE/ALL, FILE_READ_DATA, FILE_EXECUTE), writing
        // it (GENERIC_WRITE/ALL, FILE_WRITE_DATA, FILE_APPEND_DATA), deleting
        // (DELETE, GENERIC_ALL). Attribute, EA and descriptor rights alone
        // open the file for its metadata.
        let rights = u16::from(access & (0xB000_0000 | 0x21) != 0)
            | (u16::from(access & (0x5000_0000 | 0x6) != 0) << 1)
            | (u16::from(access & (0x1000_0000 | 0x1_0000) != 0) << 2);
        // A 32-bit program's System32 is SysWOW64 (P17): a Windows path
        // naming System32 or Sysnative goes through the redirector (the
        // normalized path `native_path` gives) before it is opened.
        #[cfg(target_arch = "x86")]
        if units[0] != 47 && super::wow64::names_system(&units[..length]) {
            let mut text = [0u8; PATH_TEXT];
            if let Ok(n) = native_path(table, &units[..length], &mut text)
                && let Ok(s) = core::str::from_utf8(&text[..n])
                && s.encode_utf16().count() <= units.len()
            {
                let mut count = 0;
                for unit in s.encode_utf16() {
                    units[count] = unit;
                    count += 1;
                }
                length = count;
            }
        }
        // A slash-rooted path names one of Doors' own folders (as
        // GetModuleFileName gives `/system/bin/app.exe`): opened natively.
        // Everything else is a Windows path on C:.
        let mut path = &units[..length];
        let mut sidecar = [0u8; PATH_TEXT];
        let mut stream = None;
        if let Some((base, name)) = super::streams::split(path)? {
            if name.is_empty() {
                // `file::$DATA`: the file itself.
                path = &units[..base];
            } else {
                let create = matches!(
                    disposition,
                    Disposition::CreateNew | Disposition::CreateAlways | Disposition::OpenAlways
                );
                stream = Some(super::streams::file(
                    table,
                    &units[..base],
                    name,
                    create,
                    &mut sidecar,
                )?);
            }
        }
        let native = path.first() == Some(&47)
            && !path.split(|&u| u == 47).nth(1).is_some_and(|first| {
                first.len() == 5
                    && first
                        .iter()
                        .zip(b"users")
                        .all(|(&u, &b)| u | 0x20 == u16::from(b))
            });
        let storage = table.storage.as_mut().ok_or(1117u32)?;
        let (file, created) = if let Some(n) = stream {
            let p = path_text(&sidecar[..n])?;
            let existed = storage.metadata(p).is_ok();
            let file = storage
                .open(p, disposition, rights, share as u16)
                .map_err(service)?;
            (file, !existed)
        } else if native {
            let mut text = [0u8; PATH_TEXT];
            let mut n = 0;
            for character in char::decode_utf16(path.iter().copied()) {
                let mut bytes = [0u8; 4];
                let encoded = character.map_err(|_| 123u32)?.encode_utf8(&mut bytes);
                text.get_mut(n..n + encoded.len())
                    .ok_or(206u32)?
                    .copy_from_slice(encoded.as_bytes());
                n += encoded.len();
            }
            let existed = storage.metadata(path_text(&text[..n])?).is_ok();
            let file = storage
                .open(path_text(&text[..n])?, disposition, rights, share as u16)
                .map_err(service)?;
            (file, !existed)
        } else {
            match storage.open_windows(path, disposition, rights, share as u16) {
                Ok(opened) => opened,
                Err(error) => {
                    let mut native = [0u8; PATH_TEXT];
                    let n = native_path(table, path, &mut native)?;
                    let storage = table.storage.as_mut().ok_or(1117u32)?;
                    return Err(path_error(storage, path_text(&native[..n])?, error));
                }
            }
        };
        table.slots[index].generation += 1;
        table.slots[index].entry = Some(Open {
            file,
            position: 0,
            access: rights,
            asynchronous: flags & 0x4000_0000 != 0,
            closing: false,
            pins: 0,
            path: {
                let mut p = [0u16; PATH_UNITS];
                p[..opened_length].copy_from_slice(&opened[..opened_length]);
                p
            },
            path_length: opened_length as u16,
            delete_on_close: flags & 0x0400_0000 != 0,
        });
        if matches!(
            disposition,
            Disposition::CreateAlways | Disposition::OpenAlways
        ) {
            RtlSetLastWin32Error(if created { 0 } else { 183 });
        }
        let handle =
            handles::encode(index, table.slots[index].generation).expect("checked file slot");
        LIVE_HANDLES[index].store(handle, Ordering::Release);
        Ok(handle)
    })();
    match result {
        Ok(handle) => handle,
        Err(error) => {
            failed(error);
            u64::MAX
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn DoorReadFile(
    handle: u64,
    destination: u64,
    bytes: u32,
    read: u64,
    overlapped: u64,
) -> i32 {
    if handle == door_windows_abi::NUL_HANDLE {
        // NUL: every read is at its end, a success with nothing read.
        return match count(read, 0) {
            Ok(()) => 1,
            Err(error) => failed(error),
        };
    }
    if handle == door_windows_abi::INPUT_HANDLE {
        if overlapped != 0 {
            return failed(50);
        }
        return match super::guest::console_input(destination, bytes, read) {
            Ok(()) => 1,
            Err(error) => failed(error),
        };
    }
    if overlapped != 0 {
        match synchronous_file(handle) {
            Ok(true) => return positional(handle, destination, bytes, read, overlapped, false),
            Ok(false) => {}
            Err(error) => return failed(error),
        }
        return submit(handle, destination, bytes, read, overlapped, false);
    }
    let result = (|| {
        count(read, 0)?;
        if bytes as usize >= BULK_THRESHOLD {
            // Bulk: a storage request per 256 KiB through the bounce section.
            let mut lease = Lease::acquire()?;
            let table = lease.value();
            let index = slot(table, handle)?;
            let file = table.slots[index].entry.as_ref().unwrap();
            if file.asynchronous {
                return Err(87);
            }
            if file.access & door_service_client::READ == 0 {
                return Err(5);
            }
            let position = file.position;
            let n = bulk_read(table, index, position, destination, bytes as usize)?;
            table.slots[index].entry.as_mut().unwrap().position += n as u64;
            return count(read, n as u32);
        }
        let mut data = [0u8; 1024];
        // Validate the complete destination before consuming file data.
        for offset in (0..bytes as u64).step_by(data.len()) {
            let length = (bytes as u64 - offset).min(data.len() as u64) as usize;
            copy(data.as_mut_ptr() as u64, destination + offset, length).map_err(|_| 998u32)?;
            copy(destination + offset, data.as_ptr() as u64, length).map_err(|_| 998u32)?;
        }
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        let index = slot(table, handle)?;
        let file = table.slots[index].entry.as_mut().unwrap();
        if file.asynchronous {
            return Err(87);
        }
        if file.access & door_service_client::READ == 0 {
            return Err(5);
        }
        let storage = table.storage.as_mut().ok_or(1117u32)?;
        let mut total = 0usize;
        'chunks: while total < bytes as usize {
            let wanted = (bytes as usize - total).min(data.len());
            let mut filled = 0;
            while filled < wanted {
                let limit = (wanted - filled).min(CHUNK_BYTES);
                let offset = file
                    .position
                    .checked_add((total + filled) as u64)
                    .ok_or(87u32)?;
                let actual = storage
                    .read(&file.file, offset, &mut data[filled..filled + limit])
                    .map_err(service)?;
                filled += actual;
                if actual < limit {
                    copy(destination + total as u64, data.as_ptr() as u64, filled)
                        .map_err(|_| 998u32)?;
                    total += filled;
                    break 'chunks;
                }
            }
            copy(destination + total as u64, data.as_ptr() as u64, filled).map_err(|_| 998u32)?;
            total += filled;
        }
        count(read, total as u32)?;
        file.position += total as u64;
        Ok(())
    })();
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn DoorWriteFile(
    handle: u64,
    source: u64,
    bytes: u32,
    written: u64,
    overlapped: u64,
) -> i32 {
    if overlapped != 0 {
        match synchronous_file(handle) {
            Ok(true) => return positional(handle, source, bytes, written, overlapped, true),
            Ok(false) => {}
            Err(error) => return failed(error),
        }
        return submit(handle, source, bytes, written, overlapped, true);
    }
    let result = (|| {
        count(written, 0)?;
        if bytes as usize >= BULK_THRESHOLD {
            let mut lease = Lease::acquire()?;
            let table = lease.value();
            let index = slot(table, handle)?;
            let file = table.slots[index].entry.as_ref().unwrap();
            if file.asynchronous {
                return Err(87);
            }
            if file.access & door_service_client::WRITE == 0 {
                return Err(5);
            }
            let position = file.position;
            let n = bulk_write(table, index, position, source, bytes as usize)?;
            let file = table.slots[index].entry.as_mut().unwrap();
            file.position = file.position.checked_add(n as u64).ok_or(87u32)?;
            count(written, n as u32)?;
            if table.cwd_length != 0 {
                table
                    .storage
                    .as_mut()
                    .ok_or(1117u32)?
                    .flush()
                    .map_err(service)?;
            }
            return Ok(());
        }
        let mut data = [0u8; 1024];
        // Capture-check the complete source before the first output effect.
        for offset in (0..bytes as u64).step_by(data.len()) {
            let length = (bytes as u64 - offset).min(data.len() as u64) as usize;
            copy(data.as_mut_ptr() as u64, source + offset, length).map_err(|_| 1784u32)?;
        }
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        let index = slot(table, handle)?;
        let file = table.slots[index].entry.as_mut().unwrap();
        if file.asynchronous {
            return Err(87);
        }
        if file.access & door_service_client::WRITE == 0 {
            return Err(5);
        }
        let storage = table.storage.as_mut().ok_or(1117u32)?;
        let mut total = 0;
        'chunks: while total < bytes as usize {
            let length = (bytes as usize - total).min(data.len());
            copy(data.as_mut_ptr() as u64, source + total as u64, length).map_err(|_| 1784u32)?;
            let mut done = 0;
            while done < length {
                let limit = (length - done).min(CHUNK_BYTES);
                match storage.write(&file.file, file.position, &data[done..done + limit]) {
                    Ok(actual) => {
                        done += actual;
                        file.position = file.position.checked_add(actual as u64).ok_or(87u32)?;
                        if actual < limit {
                            total += done;
                            break 'chunks;
                        }
                    }
                    Err(error) => {
                        count(written, (total + done) as u32)?;
                        return Err(service(error));
                    }
                }
            }
            total += done;
        }
        count(written, total as u32)?;
        // Ordinary applications do not have fixture-managed commit boundaries.
        // Checkpoint each bounded Win32 write before accepting another request.
        if table.cwd_length != 0 {
            storage.flush().map_err(service)?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn DoorFlushFileBuffers(handle: u64) -> i32 {
    if handle == door_windows_abi::NUL_HANDLE {
        return 1;
    }
    let result = (|| {
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        let index = slot(table, handle)?;
        if table.slots[index].entry.as_ref().unwrap().access & door_service_client::WRITE == 0 {
            return Err(5);
        }
        table
            .storage
            .as_mut()
            .ok_or(1117u32)?
            .flush()
            .map_err(service)
    })();
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorCloseFile(handle: u64) -> i32 {
    let result = (|| {
        let worker = {
            let mut lease = Lease::acquire()?;
            let table = lease.value();
            let index = slot_index(&table.slots, handle)?;
            // No new operation can capture this file while its worker is joined
            // outside the lease. A repeated CloseHandle may retry a failed drain.
            table.slots[index].entry.as_mut().unwrap().closing = true;
            if table.pending.handle == handle && table.pending.worker != 0 {
                table.pending.request.cancel();
                table.pending.worker
            } else {
                0
            }
        };
        if worker != 0 {
            retire_worker(worker, true)?;
        }
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        let index = slot_index(&table.slots, handle)?;
        super::completion::forget_handle(handle);
        if table.slots[index].entry.as_ref().unwrap().pins != 0 {
            // A file mapping still writes back through it: the handle is
            // gone (closing), the file closes with the last unpin.
            return Ok(None);
        }
        let file = table.slots[index].entry.take().unwrap();
        LIVE_HANDLES[index].store(0, Ordering::Release);
        table
            .storage
            .as_mut()
            .ok_or(1117u32)?
            .close(file.file)
            .map_err(service)?;
        Ok(file
            .delete_on_close
            .then_some((file.path, file.path_length as usize)))
    })();
    match result {
        Ok(None) => 1,
        Ok(Some((mut path, length))) => {
            // FILE_FLAG_DELETE_ON_CLOSE: the file goes with its last handle.
            path[length.min(PATH_UNITS - 1)] = 0;
            let _ = DoorDeleteFileW(path.as_ptr() as u64);
            RtlSetLastWin32Error(0);
            1
        }
        Err(error) => failed(error),
    }
}

fn native(error: door_abi::Status) -> u32 {
    match error {
        door_abi::Status::BadHandle => 6,
        door_abi::Status::ResourceExhausted => 8,
        door_abi::Status::AccessDenied => 5,
        _ => 1117,
    }
}
fn io_status(error: u32) -> u64 {
    match error {
        0 => 0,
        38 => 0xc000_0011,
        995 => u64::from(door_windows_abi::status::CANCELLED),
        998 => u64::from(door_windows_abi::status::ACCESS_VIOLATION),
        5 => 0xc000_0022,
        _ => u64::from(door_windows_abi::status::IO_DEVICE_ERROR),
    }
}
fn publish(pending: &mut Pending, generation: u64, error: u32, transferred: u32) {
    if !pending.request.finish(generation) {
        door_sys::exit(-330);
    }
    pending.error = error;
    pending.transferred = transferred;
    // OVERLAPPED.Internal and InternalHigh, a pointer each (P17).
    let at = |index| pending.overlapped + super::shapes::offset(&super::shapes::OVERLAPPED, index);
    if super::shapes::write_pointer(at(0), io_status(error)).is_err()
        || super::shapes::write_pointer(at(1), u64::from(transferred)).is_err()
    {
        pending.error = 998;
    }
    if pending.event != 0 {
        let _ = door_sys::signal(pending.event, 1);
        let _ = door_sys::close(pending.event);
        pending.event = 0;
    }
    if !pending.quiet {
        super::completion::queue_completion(
            pending.handle,
            pending.overlapped,
            io_status(pending.error) as u32,
            pending.transferred,
        );
    }
    ACTIVE_GENERATION.store(0, Ordering::Release);
}
fn retire_worker(worker: u64, wait: bool) -> Result<bool, u32> {
    let retained = {
        let mut lease = Lease::acquire()?;
        if lease.value().pending.worker != worker {
            // A previous observer already retired this exact generation.
            return Ok(true);
        }
        // Capture a kernel-owned reference while the metadata lease excludes
        // the observer that closes the original handle. Joining a naked copied
        // handle outside this lease races another observer's retirement.
        door_sys::duplicate(worker, door_abi::rights::WAIT).map_err(native)?
    };
    let outcome = door_sys::thread_join(retained, if wait { u64::MAX } else { 0 });
    let release = door_sys::close(retained);
    release.map_err(native)?;
    match outcome {
        Ok(_) => {}
        Err(door_abi::Status::WouldBlock | door_abi::Status::Timeout) => return Ok(false),
        Err(error) => return Err(native(error)),
    }
    let mut lease = Lease::acquire()?;
    let pending = &mut lease.value().pending;
    // Another caller may already have joined and retired this exact worker.
    if pending.worker == worker {
        door_sys::close(worker).map_err(native)?;
        pending.worker = 0;
        if !pending.request.retire() {
            return Err(1117);
        }
    }
    Ok(true)
}
/// Whether `handle` is a file opened without FILE_FLAG_OVERLAPPED (false
/// for other handles); the file table's lock is waited for, as any file call
/// waits for it.
fn synchronous_file(handle: u64) -> Result<bool, u32> {
    let mut lease = Lease::acquire()?;
    let table = lease.value();
    Ok(slot(table, handle).is_ok_and(|index| {
        table.slots[index]
            .entry
            .as_ref()
            .is_some_and(|file| !file.asynchronous)
    }))
}
/// ReadFile/WriteFile with an OVERLAPPED on a synchronous file handle, as
/// Windows does it: the transfer starts at the OVERLAPPED's offset and
/// completes before the call returns, the file pointer ends after it,
/// Internal and InternalHigh take the status and the bytes moved, and
/// hEvent is set. A read at or past the end is FALSE with ERROR_HANDLE_EOF
/// (.NET's RandomAccess reads and writes files this way).
fn positional(
    handle: u64,
    buffer: u64,
    bytes: u32,
    output: u64,
    overlapped: u64,
    writing: bool,
) -> i32 {
    use super::shapes::{OVERLAPPED, get, set, size};
    let mut raw = [0u8; size(&OVERLAPPED)];
    if copy(raw.as_mut_ptr() as u64, overlapped, raw.len()).is_err() {
        return failed(998);
    }
    let offset = get(&raw, &OVERLAPPED, 2) | (get(&raw, &OVERLAPPED, 3) << 32);
    let placed = (|| {
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        let index = slot(table, handle)?;
        table.slots[index].entry.as_mut().ok_or(6u32)?.position = offset;
        Ok::<(), u32>(())
    })();
    if let Err(error) = placed {
        return failed(error);
    }
    let mut moved = 0u32;
    let done = &mut moved as *mut u32 as u64;
    let ok = if writing {
        DoorWriteFile(handle, buffer, bytes, done, 0)
    } else {
        DoorReadFile(handle, buffer, bytes, done, 0)
    };
    let end_of_file = ok != 0 && !writing && moved == 0 && bytes != 0;
    let status: u64 = if end_of_file {
        0xC000_0011
    } else if ok != 0 {
        0
    } else {
        0xC000_0001
    };
    set(&mut raw, &OVERLAPPED, 0, status);
    set(&mut raw, &OVERLAPPED, 1, u64::from(moved));
    let _ = copy(overlapped, raw.as_ptr() as u64, raw.len());
    if output != 0 {
        let _ = count(output, moved);
    }
    let event = get(&raw, &OVERLAPPED, 4) & !1;
    if event != 0 {
        let _ = door_sys::signal(event, 1);
    }
    if end_of_file {
        return failed(38);
    }
    ok
}
fn submit(
    handle: u64,
    buffer: u64,
    bytes: u32,
    output: u64,
    overlapped: u64,
    writing: bool,
) -> i32 {
    let result = (|| {
        if bytes > 1024 || overlapped & 7 != 0 {
            return Err(87);
        }
        if output != 0 {
            count(output, 0)?;
        }
        // The OVERLAPPED as the program laid it out (P17): Internal and
        // InternalHigh (kept to restore on failure), the 64-bit Offset and
        // hEvent.
        use super::shapes::{OVERLAPPED, get, size};
        let mut raw = [0u8; size(&OVERLAPPED)];
        copy(raw.as_mut_ptr() as u64, overlapped, raw.len()).map_err(|_| 998u32)?;
        let file_offset = get(&raw, &OVERLAPPED, 2) | (get(&raw, &OVERLAPPED, 3) << 32);
        if file_offset == u64::MAX {
            return Err(50);
        }
        let head = 2 * super::shapes::POINTER;
        let mut data = [0u8; 1024];
        if bytes != 0 {
            copy(data.as_mut_ptr() as u64, buffer, bytes as usize).map_err(|_| 998u32)?;
            if !writing {
                copy(buffer, data.as_ptr() as u64, bytes as usize).map_err(|_| 998u32)?;
            }
        }
        let old_worker = {
            let mut lease = Lease::acquire()?;
            let table = lease.value();
            let index = slot(table, handle)?;
            let file = table.slots[index].entry.as_ref().unwrap();
            if !file.asynchronous {
                return Err(50);
            }
            if file.access
                & if writing {
                    door_service_client::WRITE
                } else {
                    door_service_client::READ
                }
                == 0
            {
                return Err(5);
            }
            table.pending.worker
        };
        if old_worker != 0 && !retire_worker(old_worker, false)? {
            return Err(8);
        }
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        slot(table, handle)?;
        if table.pending.request.state != door_windows_abi::io::State::Idle {
            return Err(8);
        }
        // Duplicate retains the kernel object even if the application closes its
        // event handle before terminal completion. Low bit suppresses the
        // completion packet only.
        let h_event = get(&raw, &OVERLAPPED, 4);
        let quiet = h_event & 1 != 0;
        let event = h_event & !1;
        let event = if event == 0 {
            0
        } else {
            let info = door_sys::inspect(event).map_err(native)?;
            if info[0] != door_abi::HandleKind::Event as u64 {
                return Err(6);
            }
            door_sys::duplicate(event, door_abi::rights::WRITE | door_abi::rights::WAIT)
                .map_err(native)?
        };
        // Internal = STATUS_PENDING, InternalHigh = 0.
        let mut status = [0u8; 16];
        status[..4].copy_from_slice(&0x103u32.to_le_bytes());
        if copy(overlapped, status.as_ptr() as u64, head).is_err() {
            if event != 0 {
                let _ = door_sys::close(event);
            }
            return Err(998);
        }
        if event != 0
            && let Err(error) = door_sys::reset(event)
        {
            let _ = door_sys::close(event);
            let _ = copy(overlapped, raw.as_ptr() as u64, head);
            return Err(native(error));
        }
        let pending = &mut table.pending;
        let Some(generation) = pending.request.submit() else {
            if event != 0 {
                let _ = door_sys::close(event);
            }
            let _ = copy(overlapped, raw.as_ptr() as u64, head);
            return Err(8);
        };
        pending.handle = handle;
        pending.overlapped = overlapped;
        pending.destination = buffer;
        pending.offset = file_offset;
        pending.length = bytes as usize;
        pending.writing = writing;
        pending.data = data;
        pending.event = event;
        pending.quiet = quiet;
        pending.error = 997;
        pending.transferred = 0;
        ACTIVE_HANDLE.store(handle, Ordering::Relaxed);
        ACTIVE_OVERLAPPED.store(overlapped, Ordering::Relaxed);
        ACTIVE_GENERATION.store(generation, Ordering::Release);
        match unsafe { door_sys::thread_create(io_worker as *const () as u64, generation) } {
            Ok(worker) => pending.worker = worker,
            Err(error) => {
                ACTIVE_GENERATION.store(0, Ordering::Release);
                pending.request.finish(generation);
                pending.request.retire();
                if pending.event != 0 {
                    let _ = door_sys::close(pending.event);
                    pending.event = 0;
                }
                let _ = copy(overlapped, raw.as_ptr() as u64, head);
                return Err(native(error));
            }
        }
        Err(997)
    })();
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}
#[cfg(target_arch = "x86_64")]
extern "system" fn io_worker(generation: u64) -> ! {
    run_io_worker(generation)
}
/// x86 (P17): the worker thread's argument arrives in ECX.
#[cfg(target_arch = "x86")]
#[unsafe(naked)]
unsafe extern "C" fn io_worker() -> ! {
    core::arch::naked_asm!("push ecx", "call {f}", "ud2", f = sym io_worker32)
}
#[cfg(target_arch = "x86")]
extern "system" fn io_worker32(generation: u32) -> ! {
    run_io_worker(u64::from(generation))
}
fn run_io_worker(generation: u64) -> ! {
    // Private native worker: no application thread notifications or APC queue.
    // It has its own kernel-owned FS/GS and cannot alias the primary lock owner.
    super::arch::mark_thread_exiting();
    let outcome = (|| {
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        if !table.pending.request.start(generation) {
            return Err(1117u32);
        }
        let mut transferred = 0usize;
        let outcome = (|| {
            let pending = &mut table.pending;
            if pending.request.cancelled {
                return Err(995u32);
            }
            let index = slot_index(&table.slots, pending.handle)?;
            let file = table.slots[index].entry.as_ref().unwrap();
            let storage = table.storage.as_mut().ok_or(1117u32)?;
            while transferred < pending.length {
                if CANCELLED_GENERATION.load(Ordering::Acquire) == generation {
                    return Err(995);
                }
                let limit = (pending.length - transferred).min(CHUNK_BYTES);
                let offset = pending
                    .offset
                    .checked_add(transferred as u64)
                    .ok_or(87u32)?;
                let chunk = &mut pending.data[transferred..transferred + limit];
                let actual = if pending.writing {
                    storage.write(&file.file, offset, chunk)
                } else {
                    storage.read(&file.file, offset, chunk)
                }
                .map_err(service)?;
                transferred += actual;
                if actual < limit {
                    break;
                }
            }
            if CANCELLED_GENERATION.load(Ordering::Acquire) == generation {
                return Err(995);
            }
            if !pending.writing && pending.length != 0 && transferred == 0 {
                return Err(38);
            }
            if !pending.writing && transferred != 0 {
                copy(
                    pending.destination,
                    pending.data.as_ptr() as u64,
                    transferred,
                )
                .map_err(|_| 998u32)?;
            }
            Ok(())
        })();
        if outcome.is_err() && !table.pending.writing {
            transferred = 0;
        }
        publish(
            &mut table.pending,
            generation,
            outcome.err().unwrap_or(0),
            transferred as u32,
        );
        Ok(())
    })();
    if outcome.is_err() {
        door_sys::exit(-331);
    }
    door_sys::thread_exit(0)
}
fn slot_index(slots: &[Slot; handles::SLOTS], handle: u64) -> Result<usize, u32> {
    let (index, generation) = handles::decode(handle).ok_or(6u32)?;
    if !door_windows_abi::tagged::same(slots[index].generation, generation)
        || slots[index].entry.is_none()
    {
        return Err(6);
    }
    Ok(index)
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorCancelIoEx(handle: u64, overlapped: u64) -> i32 {
    // Sockets and other handles whose I/O another DLL runs.
    if !handles::is_file(handle) && super::completion::cancel_hooked(handle, overlapped) {
        return 1;
    }
    let result = (|| {
        let (index, _) = handles::decode(handle).ok_or(6u32)?;
        if LIVE_HANDLES[index].load(Ordering::Acquire) != handle {
            return Err(6);
        }
        let generation = ACTIVE_GENERATION.load(Ordering::Acquire);
        if generation == 0
            || ACTIVE_HANDLE.load(Ordering::Relaxed) != handle
            || (overlapped != 0 && ACTIVE_OVERLAPPED.load(Ordering::Relaxed) != overlapped)
        {
            return Err(1168);
        }
        CANCELLED_GENERATION.fetch_max(generation, Ordering::AcqRel);
        if ACTIVE_GENERATION.load(Ordering::Acquire) != generation {
            return Err(1168);
        }
        Ok(())
    })();
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetOverlappedResult(
    handle: u64,
    overlapped: u64,
    output: u64,
    wait: i32,
) -> i32 {
    let result = (|| {
        count(output, 0)?;
        let worker = {
            let mut lease = Lease::acquire()?;
            let table = lease.value();
            slot(table, handle)?;
            if table.pending.handle != handle || table.pending.overlapped != overlapped {
                return Err(87);
            }
            table.pending.worker
        };
        if worker != 0 {
            retire_worker(worker, wait != 0)?;
        }
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        slot(table, handle)?;
        let pending = &table.pending;
        if pending.handle != handle || pending.overlapped != overlapped {
            return Err(87);
        }
        if matches!(
            pending.request.state,
            door_windows_abi::io::State::Queued | door_windows_abi::io::State::Running
        ) {
            return Err(996);
        }
        count(output, pending.transferred)?;
        if pending.error != 0 {
            return Err(pending.error);
        }
        Ok(())
    })();
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}

// P13 directory, attribute, position and current-directory adapters. Paths
// are captured through USER_COPY, resolved against the process directory and
// sent to the authenticated storage owner as native paths; the storage
// service, not this adapter, enforces the user's access.

const FIND_SLOTS: usize = 4;
const FIND_TAG: door_windows_abi::tagged::Tag = door_windows_abi::tagged::FIND;
const FIND_DATA_BYTES: usize = 592;
const ATTRIBUTE_DIRECTORY: u32 = 0x10;
const ATTRIBUTE_NORMAL: u32 = 0x80;
struct Find {
    generation: u32,
    live: bool,
    directory: [u8; PATH_TEXT],
    directory_length: usize,
    pattern: [u16; PATH_UNITS],
    pattern_length: usize,
    cookie: u64,
}
impl Find {
    const fn new() -> Self {
        Self {
            generation: 0,
            live: false,
            directory: [0; PATH_TEXT],
            directory_length: 0,
            pattern: [0; PATH_UNITS],
            pattern_length: 0,
            cookie: 0,
        }
    }
}

/// What a missing path (a storage path) reports: ERROR_FILE_NOT_FOUND
/// when its folder exists, else ERROR_PATH_NOT_FOUND, as Windows tells
/// the two apart.
pub(super) fn missing(storage: &mut Storage, path: &str) -> u32 {
    let parent = path.rfind(['\\', '/']).map_or("", |at| &path[..at]);
    if parent.is_empty() || parent.eq_ignore_ascii_case("C:") {
        return 2;
    }
    match storage.metadata(parent) {
        Ok(info) if info.directory => 2,
        _ => 3,
    }
}
/// error from storage for path, a missing file told apart (missing).
pub(super) fn path_error(storage: &mut Storage, path: &str, error: Error) -> u32 {
    match service(error) {
        2 => missing(storage, path),
        other => other,
    }
}

/// Capture a NUL-terminated UTF-16 path of at most `PATH_UNITS - 1` units
/// (MAX_PATH).
/// `f` with the file table (the storage lease held).
pub(super) fn with_table<R>(f: impl FnOnce(&mut Table) -> Result<R, u32>) -> Result<R, u32> {
    let mut lease = Lease::acquire()?;
    f(lease.value())
}
pub(super) fn read_path(pointer: u64, units: &mut [u16; PATH_UNITS]) -> Result<usize, u32> {
    for (index, unit) in units.iter_mut().enumerate() {
        let at = pointer.checked_add(index as u64 * 2).ok_or(998u32)?;
        let mut bytes = [0u8; 2];
        copy(bytes.as_mut_ptr() as u64, at, 2).map_err(|_| 998u32)?;
        *unit = u16::from_le_bytes(bytes);
        if *unit == 0 {
            return if index == 0 { Err(3) } else { Ok(index) };
        }
    }
    // Past MAX_PATH: ERROR_PATH_NOT_FOUND, as Windows 11 answers.
    Err(3)
}

/// Resolve a path (`C:\a`, `\a`, `a\b`, `/a/b`, `.`/`..`) against the
/// current directory into a normalized absolute path in UTF-8, in one of
/// storage's two forms: a Windows path on C: (`C:\a\b`, resolved the
/// Windows way: ASCII case folded, `C:\Users` is `/users`) for a drive- or
/// backslash-rooted path, or a native path (`/a/b`, exact) for a
/// slash-rooted one, as Doors names its own folders to programs
/// (`/system/bin`). A relative path takes the current directory's form.
pub(super) fn native_path(
    table: &Table,
    path: &[u16],
    out: &mut [u8; PATH_TEXT],
) -> Result<usize, u32> {
    let mut units = path;
    // `\\?\C:\...` (Win32 extended-length, which .NET's CreateDirectory
    // always uses) and `\??\C:\...` (the NT object manager's spelling):
    // the drive path after the prefix. Network paths (`\\?\UNC\`,
    // `\\server\share`) are ERROR_BAD_NETPATH; device paths (`\\.\`)
    // ERROR_INVALID_NAME, as they name no file on C:.
    if units.starts_with(&[92, 92, 63, 92]) || units.starts_with(&[92, 63, 63, 92]) {
        units = &units[4..];
        if units.len() >= 4
            && units[..4]
                .iter()
                .map(|u| u | 0x20)
                .eq([117, 110, 99, 92 | 0x20])
        {
            return Err(53);
        }
        if units.len() < 2 || units[1] != 58 {
            return Err(123);
        }
    } else if units.starts_with(&[92, 92, 46, 92]) {
        return Err(123);
    } else if units.starts_with(&[92, 92]) {
        return Err(53);
    } else if units.starts_with(&[47, 47]) {
        return Err(123);
    }
    let mut drive = false;
    if units.len() >= 2 && units[1] == 58 {
        if !matches!(units[0], 67 | 99) {
            return Err(3);
        }
        units = &units[2..];
        drive = true;
    }
    let absolute = units.first().is_some_and(|unit| matches!(unit, 47 | 92));
    let prefix: &[u16] = if absolute {
        &[]
    } else {
        &table.cwd[..table.cwd_length]
    };
    let native = !drive
        && if absolute {
            units[0] == 47
        } else {
            prefix.first() == Some(&47)
        };
    let (header, separator): (&[u8], u8) = if native {
        (b"/", b'/')
    } else {
        (b"C:\\", b'\\')
    };
    let mut starts = [0usize; 32];
    let mut depth = 0;
    out[..header.len()].copy_from_slice(header);
    let mut length = header.len();
    for part in prefix
        .split(|unit| matches!(unit, 47 | 92))
        .chain(units.split(|unit| matches!(unit, 47 | 92)))
    {
        match part {
            [] | [46] => continue,
            [46, 46] => {
                if depth > 0 {
                    depth -= 1;
                    length = starts[depth];
                }
                continue;
            }
            _ => {}
        }
        if part
            .iter()
            .any(|unit| matches!(unit, 42 | 63 | 58 | 60 | 62 | 124 | 34))
        {
            return Err(123);
        }
        if depth == starts.len() {
            return Err(206);
        }
        starts[depth] = length;
        depth += 1;
        if length > header.len() {
            *out.get_mut(length).ok_or(206u32)? = separator;
            length += 1;
        }
        for character in char::decode_utf16(part.iter().copied()) {
            let character = character.map_err(|_| 123u32)?;
            let mut encoded = [0u8; 4];
            let encoded = character.encode_utf8(&mut encoded).as_bytes();
            out.get_mut(length..length + encoded.len())
                .ok_or(206u32)?
                .copy_from_slice(encoded);
            length += encoded.len();
        }
    }
    // A 32-bit program's System32 is SysWOW64 (P17's file system redirector).
    #[cfg(target_arch = "x86")]
    if !native {
        return super::wow64::redirect(out, length);
    }
    Ok(length)
}

fn upcase(unit: u16) -> u16 {
    door_pe_loader::resources::upcase(unit)
}
/// Case-insensitive `*`/`?` match; `*.*` matches every name, as on Windows.
fn wildcard(pattern: &[u16], name: &[u16]) -> bool {
    let pattern: &[u16] = if pattern == [42, 46, 42] {
        &[42]
    } else {
        pattern
    };
    let (mut p, mut n) = (0, 0);
    let mut resume: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == 63 || upcase(pattern[p]) == upcase(name[n])) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == 42 {
            resume = Some((p, n));
            p += 1;
        } else if let Some((star, at)) = resume {
            p = star + 1;
            n = at + 1;
            resume = Some((star, at + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|unit| *unit == 42)
}

fn attributes(info: &door_service_protocol::files::FileInfo) -> u32 {
    if info.directory {
        ATTRIBUTE_DIRECTORY
    } else {
        ATTRIBUTE_NORMAL
    }
}
pub(super) fn path_text(path: &[u8]) -> Result<&str, u32> {
    core::str::from_utf8(path).map_err(|_| 123u32)
}

fn write_find_data(
    destination: u64,
    entry: &door_service_protocol::files::DirEntry,
) -> Result<(), u32> {
    let mut data = [0u8; FIND_DATA_BYTES];
    data[..4].copy_from_slice(&attributes(&entry.info).to_le_bytes());
    data[4..12].copy_from_slice(&entry.times.created.to_le_bytes());
    data[12..20].copy_from_slice(&entry.times.accessed.to_le_bytes());
    data[20..28].copy_from_slice(&entry.times.modified.to_le_bytes());
    data[28..32].copy_from_slice(&((entry.info.bytes >> 32) as u32).to_le_bytes());
    data[32..36].copy_from_slice(&(entry.info.bytes as u32).to_le_bytes());
    for (index, unit) in entry.name[..entry.name_len].iter().enumerate() {
        data[44 + index * 2..46 + index * 2].copy_from_slice(&unit.to_le_bytes());
    }
    copy(destination, data.as_ptr() as u64, data.len()).map_err(|_| 998u32)
}

fn find_next(table: &mut Table, index: usize, destination: u64) -> Result<(), u32> {
    let storage = table.storage.as_mut().ok_or(1117u32)?;
    let find = &mut table.finds[index];
    let directory = path_text(&find.directory[..find.directory_length])?;
    loop {
        let entry = storage
            .list(directory, find.cookie)
            .map_err(service)?
            .ok_or(18u32)?;
        find.cookie = entry.cookie;
        if wildcard(
            &find.pattern[..find.pattern_length],
            &entry.name[..entry.name_len],
        ) {
            return write_find_data(destination, &entry);
        }
    }
}
fn find_slot(table: &Table, handle: u64) -> Result<usize, u32> {
    let (index, generation) = FIND_TAG.decode(handle).ok_or(6u32)?;
    let find = table.finds.get(index).ok_or(6u32)?;
    if !find.live || !door_windows_abi::tagged::same(find.generation, generation) {
        return Err(6);
    }
    Ok(index)
}

/// `FindFirstFileW`. `.` and `..` entries are not reported.
#[unsafe(no_mangle)]
pub extern "system" fn DoorFindFirstFileW(pattern: u64, destination: u64) -> u64 {
    let result = (|| {
        let mut units = [0u16; PATH_UNITS];
        let length = read_path(pattern, &mut units)?;
        let split = units[..length]
            .iter()
            .rposition(|unit| matches!(unit, 47 | 92))
            .map_or(0, |at| at + 1);
        let (directory, file) = units[..length].split_at(split);
        if file.is_empty() {
            return Err(2);
        }
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        let index = table
            .finds
            .iter()
            .position(|find| !find.live && find.generation != u32::MAX)
            .ok_or(4u32)?; // ERROR_TOO_MANY_OPEN_FILES
        let mut native = [0u8; PATH_TEXT];
        let directory: &[u16] = if directory.is_empty() {
            &[46]
        } else {
            directory
        };
        let native_length = native_path(table, directory, &mut native)?;
        let storage = table.storage.as_mut().ok_or(1117u32)?;
        let info = storage
            .metadata(path_text(&native[..native_length])?)
            .map_err(|error| match service(error) {
                2 => 3,
                other => other,
            })?;
        if !info.directory {
            return Err(267); // ERROR_DIRECTORY
        }
        let find = &mut table.finds[index];
        find.generation += 1;
        find.live = true;
        find.directory = native;
        find.directory_length = native_length;
        find.pattern[..file.len()].copy_from_slice(file);
        find.pattern_length = file.len();
        find.cookie = 0;
        let generation = find.generation;
        if let Err(error) = find_next(table, index, destination) {
            table.finds[index].live = false;
            return Err(if error == 18 { 2 } else { error });
        }
        Ok(FIND_TAG.encode(index, generation))
    })();
    result.unwrap_or_else(|error| {
        failed(error);
        u64::MAX
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorFindNextFileW(handle: u64, destination: u64) -> i32 {
    let result = (|| {
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        let index = find_slot(table, handle)?;
        find_next(table, index, destination)
    })();
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorFindClose(handle: u64) -> i32 {
    if super::streams::is_search(handle) {
        return match super::streams::close_search(handle) {
            Ok(()) => 1,
            Err(error) => failed(error),
        };
    }
    let result = (|| {
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        let index = find_slot(table, handle)?;
        table.finds[index].live = false;
        Ok(())
    })();
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}

type Stat = (
    door_service_protocol::files::FileInfo,
    door_service_protocol::files::FileTimes,
);
fn stat_path(pointer: u64) -> Result<Stat, u32> {
    let mut units = [0u16; PATH_UNITS];
    let length = read_path(pointer, &mut units)?;
    let mut lease = Lease::acquire()?;
    let table = lease.value();
    let mut native = [0u8; PATH_TEXT];
    let native_length = native_path(table, &units[..length], &mut native)?;
    let storage = table.storage.as_mut().ok_or(1117u32)?;
    let path = path_text(&native[..native_length])?;
    storage.stat(path).map_err(|e| path_error(storage, path, e))
}
/// `GetFileAttributesW`: INVALID_FILE_ATTRIBUTES (`u32::MAX`) on failure.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetFileAttributesW(path: u64) -> u32 {
    match stat_path(path) {
        Ok((info, _)) => attributes(&info),
        Err(error) => {
            failed(error);
            u32::MAX
        }
    }
}
/// `GetFileAttributesExW(GetFileExInfoStandard)`: WIN32_FILE_ATTRIBUTE_DATA.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetFileAttributesExW(path: u64, level: u32, destination: u64) -> i32 {
    if level != 0 {
        return failed(87);
    }
    let result = stat_path(path).and_then(|(info, times)| {
        let mut data = [0u8; 36];
        data[..4].copy_from_slice(&attributes(&info).to_le_bytes());
        data[4..12].copy_from_slice(&times.created.to_le_bytes());
        data[12..20].copy_from_slice(&times.accessed.to_le_bytes());
        data[20..28].copy_from_slice(&times.modified.to_le_bytes());
        data[28..32].copy_from_slice(&((info.bytes >> 32) as u32).to_le_bytes());
        data[32..36].copy_from_slice(&(info.bytes as u32).to_le_bytes());
        copy(destination, data.as_ptr() as u64, data.len()).map_err(|_| 998u32)
    });
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}

/// `SetFilePointerEx` (overlapped handles use explicit offsets, but keep a
/// position as Windows does).
#[unsafe(no_mangle)]
pub extern "system" fn DoorSetFilePointerEx(
    handle: u64,
    distance: i64,
    destination: u64,
    method: u32,
) -> i32 {
    let result = (|| {
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        let index = slot(table, handle)?;
        let origin = match method {
            0 => 0,
            1 => table.slots[index].entry.as_ref().unwrap().position,
            2 => {
                let storage = table.storage.as_mut().ok_or(1117u32)?;
                storage
                    .info(&table.slots[index].entry.as_ref().unwrap().file)
                    .map_err(service)?
                    .bytes
            }
            _ => return Err(87),
        };
        let target = i64::try_from(origin)
            .ok()
            .and_then(|origin| origin.checked_add(distance))
            .ok_or(131u32)?;
        if target < 0 {
            return Err(131);
        }
        if destination != 0 {
            copy(destination, &target as *const i64 as u64, 8).map_err(|_| 998u32)?;
        }
        table.slots[index].entry.as_mut().unwrap().position = target as u64;
        Ok(())
    })();
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetFileSizeEx(handle: u64, destination: u64) -> i32 {
    let result = (|| {
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        let index = slot(table, handle)?;
        let storage = table.storage.as_mut().ok_or(1117u32)?;
        let bytes = storage
            .info(&table.slots[index].entry.as_ref().unwrap().file)
            .map_err(service)?
            .bytes;
        copy(destination, &bytes as *const u64 as u64, 8).map_err(|_| 998u32)
    })();
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}

/// An open file's access control list (advapi32's GetSecurityInfo): the
/// `files::Security` record into `out` (`files::SECURITY_BYTES`).
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetFileSecurity(handle: u64, out: u64) -> i32 {
    let result = (|| {
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        let index = slot(table, handle)?;
        let storage = table.storage.as_mut().ok_or(1117u32)?;
        let record = storage
            .file_security(&table.slots[index].entry.as_ref().unwrap().file)
            .map_err(service)?
            .encode();
        copy(out, record.as_ptr() as u64, record.len()).map_err(|_| 998u32)
    })();
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}
/// SetSecurityInfo of an open file: `record` is a `files::Security`.
#[unsafe(no_mangle)]
pub extern "system" fn DoorSetFileSecurity(handle: u64, record: u64) -> i32 {
    use door_service_protocol::files::{SECURITY_BYTES, Security};
    let result = (|| {
        let mut bytes = [0u8; SECURITY_BYTES];
        copy(bytes.as_mut_ptr() as u64, record, SECURITY_BYTES).map_err(|_| 998u32)?;
        let record = Security::decode(&bytes).map_err(|_| 87u32)?;
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        let index = slot(table, handle)?;
        let storage = table.storage.as_mut().ok_or(1117u32)?;
        storage
            .set_file_security(&table.slots[index].entry.as_ref().unwrap().file, &record)
            .map_err(service)
    })();
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}

/// `GetCurrentDirectoryW`: `C:` plus the process directory with Windows
/// separators. Too small a buffer returns the size needed, NUL included.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetCurrentDirectoryW(capacity: u32, buffer: u64) -> u32 {
    let result = (|| {
        let mut text = [0u16; PATH_UNITS + 3];
        let length = {
            let mut lease = Lease::acquire()?;
            let table = lease.value();
            text[0] = 67;
            text[1] = 58;
            let mut length = 2;
            if table.cwd_length == 0 {
                text[2] = 92;
                length = 3;
            }
            for &unit in &table.cwd[..table.cwd_length] {
                text[length] = if unit == 47 { 92 } else { unit };
                length += 1;
            }
            length
        };
        if (capacity as usize) <= length {
            return Ok(length as u32 + 1);
        }
        copy(buffer, text.as_ptr() as u64, (length + 1) * 2).map_err(|_| 998u32)?;
        Ok(length as u32)
    })();
    result.unwrap_or_else(|error| {
        failed(error);
        0
    })
}
/// `SetCurrentDirectoryW`: the directory must exist for this user.
#[unsafe(no_mangle)]
pub extern "system" fn DoorSetCurrentDirectoryW(path: u64) -> i32 {
    let result = (|| {
        let mut units = [0u16; PATH_UNITS];
        let length = read_path(path, &mut units)?;
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        let mut native = [0u8; PATH_TEXT];
        let native_length = native_path(table, &units[..length], &mut native)?;
        let text = path_text(&native[..native_length])?;
        let info = table
            .storage
            .as_mut()
            .ok_or(1117u32)?
            .metadata(text)
            .map_err(service)?;
        if !info.directory {
            return Err(267);
        }
        let mut cwd = [0u16; PATH_UNITS];
        let mut count = 0;
        // Kept without its drive ("\Users\1001"): relative paths extend it
        // and GetCurrentDirectoryW puts "C:" back.
        for unit in text.strip_prefix("C:").unwrap_or(text).encode_utf16() {
            *cwd.get_mut(count).ok_or(206u32)? = unit;
            count += 1;
        }
        if count == cwd.len() {
            return Err(206);
        }
        table.cwd = cwd;
        table.cwd_length = count;
        Ok(())
    })();
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}
/// The computer name (Settings > System > About), ASCII into `out` (at most
/// `capacity` bytes, no terminator) with its length; `ERROR_BUFFER_OVERFLOW`
/// (111) and the needed length when it does not fit.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetComputerName(out: u64, capacity: u32, length: u64) -> u32 {
    let mut lease = match Lease::acquire() {
        Ok(lease) => lease,
        Err(error) => return error,
    };
    let Some(storage) = lease.value().storage.as_mut() else {
        return 1117;
    };
    let name = door_service_client::device_name(storage);
    let text = name.as_str().as_bytes();
    if count(length, text.len() as u32).is_err() {
        return error::NOACCESS;
    }
    if text.len() > capacity as usize {
        return 111;
    }
    copy(out, text.as_ptr() as u64, text.len()).map_or(error::NOACCESS, |()| 0)
}

// ---- namespace changes (CreateDirectory, RemoveDirectory, DeleteFile,
// MoveFileEx, CopyFile) and Windows-to-native path resolution ----

/// Runs `f` with the storage connection and the native form of the Windows
/// path at `pointer`.
/// The process's storage connection.
/// The whole file at the Windows path `pointer`, at most `limit` bytes,
/// read into a block of the process heap (LoadLibrary of a DLL that is not
/// a system DLL): the block's address and length. ERROR_BAD_EXE_FORMAT for
/// an empty or oversized file.
pub(super) fn whole_file(pointer: u64, limit: usize) -> Result<(u64, usize), u32> {
    let mut units = [0u16; PATH_UNITS];
    let length = read_path(pointer, &mut units)?;
    let mut lease = Lease::acquire()?;
    let table = lease.value();
    let mut native = [0u8; PATH_TEXT];
    let native_length = native_path(table, &units[..length], &mut native)?;
    let path = path_text(&native[..native_length])?;
    let (section, view) = bounce(table)?;
    let s = table.storage.as_mut().ok_or(1117u32)?;
    let file = s
        .open(path, Disposition::OpenExisting, 1, 1)
        .map_err(service)?;
    let result = (|| {
        let size = s.info(&file).map_err(service)?.bytes;
        if size == 0 || size > limit as u64 {
            return Err(193);
        }
        let size = size as usize;
        let heap = super::heap::DoorGetProcessHeap();
        let block = super::heap::RtlAllocateHeap(door_windows_abi::narrow(heap), 0, size);
        if block == 0 {
            return Err(8);
        }
        let block = door_windows_abi::widen(block);
        // Through the bounce section: one storage request per 256 KiB.
        let mut done = 0usize;
        let read = (|| {
            while done < size {
                let want = (size - done).min(BOUNCE_BYTES as usize);
                let n = s
                    .read_section(&file, done as u64, section, 0, want)
                    .map_err(service)?;
                if n != want {
                    return Err(30u32);
                }
                super::guest::copy_bulk(block + done as u64, view, n)?;
                done += n;
            }
            Ok(())
        })();
        match read {
            Ok(()) => Ok((block, size)),
            Err(error) => {
                super::heap::RtlFreeHeap(
                    door_windows_abi::narrow(heap),
                    0,
                    door_windows_abi::narrow(block),
                );
                Err(error)
            }
        }
    })();
    let _ = s.close(file);
    result
}
/// Open the file at `pointer` (a Win32 path, NUL-terminated UTF-16) and hand
/// `f` its size (at most `limit`) and a reader that copies `out.len()`
/// bytes from a file offset straight into `out`, one storage request per
/// 256 KiB: the user-mode loader lays images out from their files this way,
/// without a second copy of a large image in memory.
pub(super) fn read_file_with<R>(
    pointer: u64,
    limit: u64,
    f: impl FnOnce(u64, &mut dyn FnMut(u64, &mut [u8]) -> Result<(), u32>) -> Result<R, u32>,
) -> Result<R, u32> {
    let mut units = [0u16; PATH_UNITS];
    let length = read_path(pointer, &mut units)?;
    let mut lease = Lease::acquire()?;
    let table = lease.value();
    let mut native = [0u8; PATH_TEXT];
    let native_length = native_path(table, &units[..length], &mut native)?;
    let path = path_text(&native[..native_length])?;
    let (section, view) = bounce(table)?;
    let s = table.storage.as_mut().ok_or(1117u32)?;
    let file = s
        .open(path, Disposition::OpenExisting, 1, 1)
        .map_err(service)?;
    let result = (|| {
        let size = s.info(&file).map_err(service)?.bytes;
        if size == 0 || size > limit {
            return Err(193);
        }
        let mut read = |offset: u64, out: &mut [u8]| -> Result<(), u32> {
            let mut done = 0usize;
            while done < out.len() {
                let want = (out.len() - done).min(BOUNCE_BYTES as usize);
                let n = s
                    .read_section(&file, offset + done as u64, section, 0, want)
                    .map_err(service)?;
                if n != want {
                    return Err(30u32);
                }
                super::guest::copy_bulk(out[done..].as_mut_ptr() as u64, view, n)?;
                done += n;
            }
            Ok(())
        };
        f(size, &mut read)
    })();
    let _ = s.close(file);
    result
}
pub(super) fn with_storage<R>(f: impl FnOnce(&mut Storage) -> Result<R, u32>) -> Result<R, u32> {
    let mut lease = Lease::acquire()?;
    f(lease.value().storage.as_mut().ok_or(1117u32)?)
}
pub(super) fn with_native<R>(
    pointer: u64,
    f: impl FnOnce(&mut Storage, &str) -> Result<R, u32>,
) -> Result<R, u32> {
    let mut units = [0u16; PATH_UNITS];
    let length = read_path(pointer, &mut units)?;
    let mut lease = Lease::acquire()?;
    let table = lease.value();
    let mut native = [0u8; PATH_TEXT];
    let native_length = native_path(table, &units[..length], &mut native)?;
    let path = path_text(&native[..native_length])?;
    f(table.storage.as_mut().ok_or(1117u32)?, path)
}
fn done(result: Result<(), u32>) -> i32 {
    match result {
        Ok(()) => {
            RtlSetLastWin32Error(0);
            1
        }
        Err(error) => failed(error),
    }
}
/// The storage path of a path (see `native_path`; UTF-8, no terminator,
/// resolved against the current directory), into `out` (at most
/// `capacity` bytes); its length, or 0 with the last error.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetNativePath(path: u64, out: u64, capacity: u32) -> u32 {
    match with_native(path, |storage, resolved| {
        // A Windows spelling becomes the stored native one (an existing
        // path; a new one keeps its Windows spelling, which storage takes).
        let mut spelled = [0u8; PATH_TEXT];
        let native = if resolved.starts_with("C:\\") {
            match native_spelling(storage, resolved, &mut spelled) {
                Ok(n) => path_text(&spelled[..n])?,
                Err(_) => resolved,
            }
        } else {
            resolved
        };
        if native.len() > capacity as usize {
            return Err(error::INSUFFICIENT_BUFFER);
        }
        copy(out, native.as_ptr() as u64, native.len()).map_err(|_| error::NOACCESS)?;
        Ok(native.len() as u32)
    }) {
        Ok(n) => n,
        Err(e) => {
            failed(e);
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DoorCreateDirectoryW(path: u64) -> i32 {
    done(with_native(path, |s, p| {
        s.mkdir(p).map_err(|e| match service(e) {
            80 => 183, // ERROR_ALREADY_EXISTS
            2 => 3,    // the parent is missing: ERROR_PATH_NOT_FOUND
            other => other,
        })
    }))
}
/// RemoveDirectoryW: only an existing, empty directory.
#[unsafe(no_mangle)]
pub extern "system" fn DoorRemoveDirectoryW(path: u64) -> i32 {
    done(with_native(path, |s, p| {
        let info = s.metadata(p).map_err(|e| path_error(s, p, e))?;
        if !info.directory {
            return Err(267); // ERROR_DIRECTORY
        }
        if s.list(p, 0).map_err(service)?.is_some() {
            return Err(145); // ERROR_DIR_NOT_EMPTY
        }
        s.remove(p).map_err(service)
    }))
}
/// DeleteFileW: a file, never a directory (ERROR_ACCESS_DENIED, as Windows).
#[unsafe(no_mangle)]
pub extern "system" fn DoorDeleteFileW(path: u64) -> i32 {
    let mut units = [0u16; PATH_UNITS];
    let length = match read_path(path, &mut units) {
        Ok(n) => n,
        Err(e) => return failed(e),
    };
    if let Ok(Some((base, name))) = super::streams::split(&units[..length])
        && !name.is_empty()
    {
        return done(with_table(|table| {
            let mut sidecar = [0u8; PATH_TEXT];
            let n = super::streams::file(table, &units[..base], name, false, &mut sidecar)?;
            let storage = table.storage.as_mut().ok_or(1117u32)?;
            storage.remove(path_text(&sidecar[..n])?).map_err(service)
        }));
    }
    done(with_native(path, |s, p| {
        let info = s.metadata(p).map_err(|e| path_error(s, p, e))?;
        if info.directory {
            return Err(5);
        }
        s.remove(p).map_err(service)?;
        // Its named streams go with it.
        super::streams::remove_all(s, info.object);
        Ok(())
    }))
}
const MOVEFILE_REPLACE_EXISTING: u32 = 1;
const MOVEFILE_COPY_ALLOWED: u32 = 2;
/// MoveFileExW: rename within the volume; REPLACE_EXISTING removes an
/// existing file target first.
#[unsafe(no_mangle)]
pub extern "system" fn DoorMoveFileExW(existing: u64, new: u64, flags: u32) -> i32 {
    let mut target = [0u8; PATH_TEXT];
    let target_length = match with_native(new, |_, p| {
        target[..p.len()].copy_from_slice(p.as_bytes());
        Ok(p.len())
    }) {
        Ok(n) => n,
        Err(e) => return failed(e),
    };
    if flags & !(MOVEFILE_REPLACE_EXISTING | MOVEFILE_COPY_ALLOWED | 8) != 0 {
        return failed(87);
    }
    done(with_native(existing, |s, p| {
        let to = path_text(&target[..target_length])?;
        if let Ok(info) = s.metadata(to) {
            if flags & MOVEFILE_REPLACE_EXISTING == 0 || info.directory {
                return Err(183);
            }
            s.remove(to).map_err(service)?;
        }
        if let Err(error) = s.metadata(p) {
            return Err(path_error(s, p, error));
        }
        s.rename(p, to).map_err(service)
    }))
}
/// CopyFileW: the bytes of a file into a new (or, unless `fail_if_exists`,
/// replaced) file.
#[unsafe(no_mangle)]
pub extern "system" fn DoorCopyFileW(existing: u64, new: u64, fail_if_exists: i32) -> i32 {
    let mut target = [0u8; PATH_TEXT];
    let target_length = match with_native(new, |_, p| {
        target[..p.len()].copy_from_slice(p.as_bytes());
        Ok(p.len())
    }) {
        Ok(n) => n,
        Err(e) => return failed(e),
    };
    done(with_native(existing, |s, p| {
        let to = path_text(&target[..target_length])?;
        let source = s
            .open(p, Disposition::OpenExisting, 1, 1)
            .map_err(service)?;
        let result = (|| {
            if s.info(&source).map_err(service)?.directory {
                return Err(5);
            }
            let disposition = if fail_if_exists != 0 {
                Disposition::CreateNew
            } else {
                Disposition::CreateAlways
            };
            let target = s
                .open(to, disposition, 2, 0)
                .map_err(|e| match service(e) {
                    80 => 80, // ERROR_FILE_EXISTS
                    other => other,
                })?;
            let copied = (|| {
                let mut chunk = [0u8; CHUNK_BYTES];
                let mut offset = 0u64;
                loop {
                    let n = s.read(&source, offset, &mut chunk).map_err(service)?;
                    if n == 0 {
                        return Ok(());
                    }
                    let mut written = 0;
                    while written < n {
                        let w = s
                            .write(&target, offset + written as u64, &chunk[written..n])
                            .map_err(service)?;
                        if w == 0 {
                            return Err(29); // ERROR_WRITE_FAULT
                        }
                        written += w;
                    }
                    offset += n as u64;
                }
            })();
            let objects = (s.info(&source), s.info(&target));
            let closed = s.close(target).map_err(service);
            if copied.is_ok()
                && let (Ok(from), Ok(to)) = objects
            {
                super::streams::copy_all(s, from.object, to.object);
            }
            copied.and(closed)
        })();
        let closed = s.close(source).map_err(service);
        result.and(closed)
    }))
}

/// CreateProcessW's program search: the native path of `path` (a Windows
/// path, resolved against the current directory) into `out` when it names
/// an existing file this user can see; None when nothing is there.
pub(super) fn existing_file(path: &[u16], out: &mut [u8; PATH_TEXT]) -> Result<Option<usize>, u32> {
    let mut lease = Lease::acquire()?;
    let table = lease.value();
    let length = match native_path(table, path, out) {
        Ok(length) => length,
        Err(123 | 3) => return Ok(None),
        Err(error) => return Err(error),
    };
    let storage = table.storage.as_mut().ok_or(1117u32)?;
    let text = path_text(&out[..length])?;
    match storage.metadata(text) {
        Ok(info) if !info.directory => Ok(Some(length)),
        Ok(_) | Err(Error::Service(Code::NotFound)) => Ok(None),
        Err(error) => Err(service(error)),
    }
}
/// The native current directory (`/` plus its parts) into `out`.
pub(super) fn current_native(out: &mut [u8; PATH_TEXT]) -> Result<usize, u32> {
    let mut lease = Lease::acquire()?;
    native_path(lease.value(), &[], out)
}
/// The native path of a Windows directory `path` into `out`, when it is
/// an existing directory.
pub(super) fn existing_directory(
    path: &[u16],
    out: &mut [u8; PATH_TEXT],
) -> Result<Option<usize>, u32> {
    let mut lease = Lease::acquire()?;
    let table = lease.value();
    let Ok(length) = native_path(table, path, out) else {
        return Ok(None);
    };
    let storage = table.storage.as_mut().ok_or(1117u32)?;
    match storage.metadata(path_text(&out[..length])?) {
        Ok(info) if info.directory => {}
        Ok(_) | Err(Error::Service(Code::NotFound)) => return Ok(None),
        Err(error) => return Err(service(error)),
    }
    if !out[..length].starts_with(b"C:\\") {
        return Ok(Some(length));
    }
    // A Windows spelling: the process it is for starts there, and a
    // process's current directory is native.
    let mut windows = [0u8; PATH_TEXT];
    windows[..length].copy_from_slice(&out[..length]);
    native_spelling(storage, path_text(&windows[..length])?, out).map(Some)
}
/// The native spelling of the existing Windows path `windows` (a folder
/// or a file, `C:\...`): C:\Users is /users, the rest of the drive /compat, and each
/// component takes its stored spelling, matched without regard to ASCII
/// case as Windows names are.
fn native_spelling(
    storage: &mut Storage,
    windows: &str,
    out: &mut [u8; PATH_TEXT],
) -> Result<usize, u32> {
    let mut parts = windows[3..]
        .split('\\')
        .filter(|p| !p.is_empty())
        .peekable();
    let base: &[u8] = if parts
        .peek()
        .is_some_and(|p| p.eq_ignore_ascii_case("users"))
    {
        parts.next();
        b"/users"
    } else {
        b"/compat"
    };
    out[..base.len()].copy_from_slice(base);
    let mut length = base.len();
    for part in parts {
        let typed = length + 1 + part.len();
        if typed > out.len() {
            return Err(206);
        }
        out[length] = b'/';
        out[length + 1..typed].copy_from_slice(part.as_bytes());
        if storage.metadata(path_text(&out[..typed])?).is_ok() {
            length = typed;
            continue;
        }
        // The stored spelling, by listing the parent.
        let parent = path_text(&out[..length])?;
        let mut parent_buffer = [0u8; PATH_TEXT];
        parent_buffer[..parent.len()].copy_from_slice(parent.as_bytes());
        let parent = path_text(&parent_buffer[..length])?;
        let mut cookie = 0;
        let mut found = None;
        for _ in 0..4096 {
            let Some(entry) = storage.list(parent, cookie).map_err(service)? else {
                break;
            };
            if entry.cookie <= cookie {
                break;
            }
            cookie = entry.cookie;
            let name = &entry.name[..entry.name_len];
            let same = name.len() == part.encode_utf16().count()
                && name.iter().zip(part.encode_utf16()).all(|(a, b)| {
                    let fold = |u: u16| {
                        if (u16::from(b'A')..=u16::from(b'Z')).contains(&u) {
                            u + 32
                        } else {
                            u
                        }
                    };
                    fold(*a) == fold(b)
                });
            if same {
                found = Some(entry);
                break;
            }
        }
        let entry = found.ok_or(3u32)?;
        out[length] = b'/';
        let mut at = length + 1;
        for character in char::decode_utf16(entry.name[..entry.name_len].iter().copied()) {
            let mut bytes = [0u8; 4];
            let encoded = character.map_err(|_| 123u32)?.encode_utf8(&mut bytes);
            let end = at + encoded.len();
            if end > out.len() {
                return Err(206);
            }
            out[at..end].copy_from_slice(encoded.as_bytes());
            at = end;
        }
        length = at;
    }
    Ok(length)
}

// ---------------------------------------------------------------------------
// Bulk file data (P18): READ_SECTION / WRITE_SECTION through sections, so a
// read or write moves `files::SECTION_IO_BYTES` per storage request instead
// of `CHUNK_BYTES` per message.

/// The bounce section's size: one storage request's worth.
const BOUNCE_BYTES: u64 = door_service_protocol::files::SECTION_IO_BYTES as u64;
/// Requests at least this large take the bulk path.
const BULK_THRESHOLD: usize = 4096;
/// Rights a section lent to the storage service carries.
pub(super) const SECTION_RIGHTS: u64 = door_abi::rights::QUERY
    | door_abi::rights::DUPLICATE
    | door_abi::rights::TRANSFER
    | door_abi::rights::READ
    | door_abi::rights::WRITE
    | door_abi::rights::MAP;

fn vm(operation: u32, args: [u64; 6]) -> Result<u64, u32> {
    unsafe { door_sys::raw(operation, args) }
        .result()
        .map_err(|_| 8u32)
}
/// The process's bounce section (created on first use, kept mapped).
fn bounce(table: &mut Table) -> Result<(u64, u64), u32> {
    if let Some(bounce) = table.bounce {
        return Ok(bounce);
    }
    let section = vm(
        door_abi::operation::VM_SECTION_CREATE,
        [BOUNCE_BYTES, 0, 0x04, SECTION_RIGHTS, 0, 0],
    )?;
    let view = match vm(
        door_abi::operation::VM_SECTION_MAP,
        [section, 0, 0, 0, 0x04, 0],
    ) {
        Ok(view) => view,
        Err(error) => {
            let _ = door_sys::close(section);
            return Err(error);
        }
    };
    table.bounce = Some((section, view));
    Ok((section, view))
}
/// Read up to `bytes` of the file in slot `index` at `offset` into the
/// caller's memory at `destination` through the bounce section: the count
/// read (short at the end of the file).
fn bulk_read(
    table: &mut Table,
    index: usize,
    offset: u64,
    destination: u64,
    bytes: usize,
) -> Result<usize, u32> {
    let (section, view) = bounce(table)?;
    let storage = table.storage.as_mut().ok_or(1117u32)?;
    let file = &table.slots[index].entry.as_ref().ok_or(6u32)?.file;
    let mut done = 0usize;
    while done < bytes {
        let want = (bytes - done).min(BOUNCE_BYTES as usize);
        let n = storage
            .read_section(file, offset + done as u64, section, 0, want)
            .map_err(service)?;
        super::guest::copy_bulk(destination + done as u64, view, n).map_err(|_| 998u32)?;
        done += n;
        if n < want {
            break;
        }
    }
    Ok(done)
}
/// Write `bytes` from the caller's memory at `source` to the file in slot
/// `index` at `offset` through the bounce section: the count written.
fn bulk_write(
    table: &mut Table,
    index: usize,
    offset: u64,
    source: u64,
    bytes: usize,
) -> Result<usize, u32> {
    let (section, view) = bounce(table)?;
    let storage = table.storage.as_mut().ok_or(1117u32)?;
    let file = &table.slots[index].entry.as_ref().ok_or(6u32)?.file;
    let mut done = 0usize;
    while done < bytes {
        let want = (bytes - done).min(BOUNCE_BYTES as usize);
        super::guest::copy_bulk(view, source + done as u64, want).map_err(|_| 1784u32)?;
        let n = storage
            .write_section(file, offset + done as u64, section, 0, want)
            .map_err(service)?;
        done += n;
        if n < want {
            break;
        }
    }
    Ok(done)
}

// ---------------------------------------------------------------------------
// File mappings (`sections.rs`): the file side of CreateFileMapping.

/// The size and access of the open file `handle`.
pub(super) fn mapping_file(handle: u64) -> Result<(u64, u16), u32> {
    let mut lease = Lease::acquire()?;
    let table = lease.value();
    let index = slot(table, handle)?;
    let open = table.slots[index].entry.as_ref().unwrap();
    let access = open.access;
    let bytes = table
        .storage
        .as_mut()
        .ok_or(1117u32)?
        .info(&open.file)
        .map_err(service)?
        .bytes;
    Ok((bytes, access))
}
/// Grow the file `handle` to `bytes` (CreateFileMapping beyond the end of
/// a writable file extends it, as on Windows).
pub(super) fn mapping_extend(handle: u64, bytes: u64) -> Result<(), u32> {
    let mut lease = Lease::acquire()?;
    let table = lease.value();
    let index = slot(table, handle)?;
    let storage = table.storage.as_mut().ok_or(1117u32)?;
    storage
        .truncate(&table.slots[index].entry.as_ref().unwrap().file, bytes)
        .map_err(service)
}
/// A file mapping writes back through `handle`: keep the file open past
/// CloseHandle until `mapping_unpin`.
pub(super) fn mapping_pin(handle: u64) -> Result<(), u32> {
    let mut lease = Lease::acquire()?;
    let table = lease.value();
    let index = slot(table, handle)?;
    let open = table.slots[index].entry.as_mut().unwrap();
    open.pins = open.pins.checked_add(1).ok_or(8u32)?;
    Ok(())
}
pub(super) fn mapping_unpin(handle: u64) {
    let Ok(mut lease) = Lease::acquire() else {
        return;
    };
    let table = lease.value();
    let Ok(index) = slot_index(&table.slots, handle) else {
        return;
    };
    let open = table.slots[index].entry.as_mut().unwrap();
    open.pins = open.pins.saturating_sub(1);
    if open.pins == 0 && open.closing {
        // CloseHandle came first: close it now.
        let file = table.slots[index].entry.take().unwrap();
        LIVE_HANDLES[index].store(0, Ordering::Release);
        if let Some(storage) = table.storage.as_mut() {
            let _ = storage.close(file.file);
        }
    }
}
/// File bytes `offset..offset + bytes` into `section` at `section_offset`
/// (an open or pinned file): the count read.
pub(super) fn mapping_read(
    handle: u64,
    offset: u64,
    section: u64,
    section_offset: u64,
    bytes: u64,
) -> Result<u64, u32> {
    let mut lease = Lease::acquire()?;
    let table = lease.value();
    let index = slot_index(&table.slots, handle)?;
    let storage = table.storage.as_mut().ok_or(1117u32)?;
    storage
        .read_section(
            &table.slots[index].entry.as_ref().unwrap().file,
            offset,
            section,
            section_offset,
            usize::try_from(bytes).map_err(|_| 8u32)?,
        )
        .map(|n| n as u64)
        .map_err(service)
}
/// `section` bytes from `section_offset` written to the file at `offset`
/// (FlushViewOfFile, UnmapViewOfFile of a shared writable file view).
pub(super) fn mapping_write(
    handle: u64,
    offset: u64,
    section: u64,
    section_offset: u64,
    bytes: u64,
) -> Result<(), u32> {
    let mut lease = Lease::acquire()?;
    let table = lease.value();
    let index = slot_index(&table.slots, handle)?;
    let storage = table.storage.as_mut().ok_or(1117u32)?;
    let file = &table.slots[index].entry.as_ref().unwrap().file;
    let written = storage
        .write_section(
            file,
            offset,
            section,
            section_offset,
            usize::try_from(bytes).map_err(|_| 8u32)?,
        )
        .map_err(service)?;
    if written as u64 != bytes {
        return Err(1784);
    }
    storage.flush().map_err(service)
}
/// Up to `out.len()` bytes of the open file `handle` at `offset` (headers
/// of a SEC_IMAGE file): the count read.
pub(super) fn mapping_peek(handle: u64, offset: u64, out: &mut [u8]) -> Result<usize, u32> {
    let mut lease = Lease::acquire()?;
    let table = lease.value();
    let index = slot(table, handle)?;
    let storage = table.storage.as_mut().ok_or(1117u32)?;
    storage
        .read_full(
            &table.slots[index].entry.as_ref().unwrap().file,
            offset,
            out,
        )
        .map_err(service)
}

/// The path `handle` was opened by, in `out` (`capacity` units): its
/// length without the NUL, or the units needed with it when `out` is too
/// small; 0 with the last error set on failure.
#[unsafe(no_mangle)]
pub extern "system" fn DoorGetFilePath(handle: u64, out: u64, capacity: u32) -> u32 {
    let result = (|| {
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        let index = slot(table, handle)?;
        let open = table.slots[index].entry.as_ref().unwrap();
        let length = open.path_length as usize;
        if (capacity as usize) <= length {
            return Ok(length as u32 + 1);
        }
        let mut units = [0u16; PATH_UNITS + 1];
        units[..length].copy_from_slice(&open.path[..length]);
        copy(out, units.as_ptr() as u64, (length + 1) * 2).map_err(|_| error::NOACCESS)?;
        Ok(length as u32)
    })();
    result.unwrap_or_else(|e| {
        failed(e);
        0
    })
}
/// The file of `handle`, as GetFileInformationByHandle needs it: size,
/// object id, attributes, then creation, access and write FILETIMEs
/// (u64 bytes, u64 object, u32 attributes, u32 links, u64 x 3).
#[unsafe(no_mangle)]
pub extern "system" fn DoorFileInformation(handle: u64, out: u64) -> i32 {
    let result = (|| {
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        let index = slot(table, handle)?;
        let storage = table.storage.as_mut().ok_or(1117u32)?;
        let (info, times) = storage
            .file_stat(&table.slots[index].entry.as_ref().unwrap().file)
            .map_err(service)?;
        let mut record = [0u8; 48];
        record[..8].copy_from_slice(&info.bytes.to_le_bytes());
        record[8..16].copy_from_slice(&info.object.to_le_bytes());
        record[16..20].copy_from_slice(&attributes(&info).to_le_bytes());
        record[20..24].copy_from_slice(&1u32.to_le_bytes());
        record[24..32].copy_from_slice(&times.created.to_le_bytes());
        record[32..40].copy_from_slice(&times.accessed.to_le_bytes());
        record[40..48].copy_from_slice(&times.modified.to_le_bytes());
        copy(out, record.as_ptr() as u64, record.len()).map_err(|_| error::NOACCESS)
    })();
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}
/// SetEndOfFile / FileEndOfFileInfo: the file's size becomes `bytes`
/// (zeros when it grows); the handle's position is unchanged.
#[unsafe(no_mangle)]
pub extern "system" fn DoorSetEndOfFile(handle: u64, bytes: u64) -> i32 {
    let result = (|| {
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        let index = slot(table, handle)?;
        let open = table.slots[index].entry.as_ref().unwrap();
        if open.access & 2 == 0 {
            return Err(5);
        }
        let storage = table.storage.as_mut().ok_or(1117u32)?;
        storage.truncate(&open.file, bytes).map_err(service)
    })();
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}
/// FileDispositionInfo(Ex): delete (or keep) the file at its last close.
#[unsafe(no_mangle)]
pub extern "system" fn DoorSetDeleteOnClose(handle: u64, delete: i32) -> i32 {
    let result = (|| {
        let mut lease = Lease::acquire()?;
        let table = lease.value();
        let index = slot(table, handle)?;
        let open = table.slots[index].entry.as_mut().unwrap();
        // DELETE access is storage's third right.
        if open.access & 4 == 0 {
            return Err(5);
        }
        open.delete_on_close = delete != 0;
        Ok(())
    })();
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}
/// GetDiskFreeSpaceEx's figures for C: (the home volume, from the
/// filesystem's own free-block accounting): total then free bytes, two
/// u64 at `out`.
#[unsafe(no_mangle)]
pub extern "system" fn DoorVolumeUsage(out: u64) -> i32 {
    let result = (|| {
        let mut lease = Lease::acquire()?;
        let storage = lease.value().storage.as_mut().ok_or(1117u32)?;
        let usage = storage.volume_usage().map_err(service)?;
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&usage.total_bytes.to_le_bytes());
        bytes[8..].copy_from_slice(&usage.free_bytes.to_le_bytes());
        copy(out, bytes.as_ptr() as u64, 16).map_err(|_| 998u32)
    })();
    match result {
        Ok(()) => 1,
        Err(error) => failed(error),
    }
}
