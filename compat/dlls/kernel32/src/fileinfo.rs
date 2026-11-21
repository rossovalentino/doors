//! File handles' identity and metadata (P18): directory handles
//! (CreateFile with FILE_FLAG_BACKUP_SEMANTICS), GetFinalPathNameByHandle,
//! GetFileInformationByHandle(Ex), SetFileInformationByHandle, SetEndOfFile
//! and GetFileTime, over ntdll's file and directory tables.
use door_windows_abi::{self as abi, error, tagged::DIRECTORY};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorOpenDirectory(path: u64) -> u64;
    fn DoorCloseDirectory(handle: u64) -> i32;
    fn DoorDirectoryPath(handle: u64, out: u64, capacity: u32) -> u32;
    fn DoorDirectoryInformation(handle: u64, out: u64) -> i32;
    fn DoorGetFilePath(handle: u64, out: u64, capacity: u32) -> u32;
    fn DoorFileInformation(handle: u64, out: u64) -> i32;
    fn DoorSetEndOfFile(handle: u64, bytes: u64) -> i32;
    fn DoorSetDeleteOnClose(handle: u64, delete: i32) -> i32;
    fn DoorSetFilePointerEx(handle: u64, distance: i64, destination: u64, method: u32) -> i32;
    fn DoorMoveFileExW(existing: u64, new: u64, flags: u32) -> i32;
    fn DoorGetFileAttributesW(path: u64) -> u32;
    fn DoorUserCopy(destination: u64, source: u64, bytes: usize) -> u32;
    fn RtlSetLastWin32Error(code: u32);
    fn RtlGetLastWin32Error() -> u32;
}
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const OPEN_EXISTING: u32 = 3;
const OPEN_ALWAYS: u32 = 4;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
/// The volume serial number Doors reports for C: (GetVolumeInformation).
const VOLUME_SERIAL: u32 = 0xD00D_0C0D;
const PATH: usize = 1024;

fn fail(code: u32) -> i32 {
    unsafe { RtlSetLastWin32Error(code) };
    0
}
fn write(pointer: u64, bytes: &[u8]) -> Result<(), u32> {
    if pointer == 0 {
        return Err(error::INVALID_PARAMETER);
    }
    match unsafe { DoorUserCopy(pointer, bytes.as_ptr() as u64, bytes.len()) } {
        0 => Ok(()),
        _ => Err(error::NOACCESS),
    }
}
fn read(pointer: u64, out: &mut [u8]) -> Result<(), u32> {
    if pointer == 0 {
        return Err(error::INVALID_PARAMETER);
    }
    match unsafe { DoorUserCopy(out.as_mut_ptr() as u64, pointer, out.len()) } {
        0 => Ok(()),
        _ => Err(error::NOACCESS),
    }
}
pub(crate) fn is_directory(handle: u64) -> bool {
    DIRECTORY.is(handle)
}
pub(crate) fn close_directory(handle: u64) -> i32 {
    unsafe { DoorCloseDirectory(handle) }
}
/// CreateFileW's folder case: with FILE_FLAG_BACKUP_SEMANTICS and an
/// existing folder (OPEN_EXISTING / OPEN_ALWAYS), a directory handle.
pub(crate) fn open_directory(path: u64, disposition: u32, flags: u32) -> Option<u64> {
    if flags & FILE_FLAG_BACKUP_SEMANTICS == 0
        || !matches!(disposition, OPEN_EXISTING | OPEN_ALWAYS)
        || path == 0
    {
        return None;
    }
    let attributes = unsafe { DoorGetFileAttributesW(path) };
    if attributes == u32::MAX || attributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return None;
    }
    Some(unsafe { DoorOpenDirectory(path) })
}

/// The record ntdll fills: size, object, attributes, links, creation,
/// access and write times.
struct Information {
    bytes: u64,
    object: u64,
    attributes: u32,
    links: u32,
    created: u64,
    accessed: u64,
    written: u64,
}
fn information(handle: u64) -> Result<Information, u32> {
    let mut r = [0u8; 48];
    let ok = if is_directory(handle) {
        unsafe { DoorDirectoryInformation(handle, r.as_mut_ptr() as u64) }
    } else if abi::file::is_file(handle) {
        unsafe { DoorFileInformation(handle, r.as_mut_ptr() as u64) }
    } else {
        return Err(error::INVALID_HANDLE);
    };
    if ok == 0 {
        return Err(unsafe { RtlGetLastWin32Error() });
    }
    let q = |at: usize| u64::from_le_bytes(r[at..at + 8].try_into().unwrap());
    let d = |at: usize| u32::from_le_bytes(r[at..at + 4].try_into().unwrap());
    Ok(Information {
        bytes: q(0),
        object: q(8),
        attributes: d(16),
        links: d(20),
        created: q(24),
        accessed: q(32),
        written: q(40),
    })
}
/// The handle's path as Windows names it: `C:\...` (a path relative to
/// the current directory joined to it, `.` and `..` resolved), or a Doors
/// folder (`/system/...`) with backslashes.
fn handle_path(handle: u64, out: &mut [u16; PATH]) -> Result<usize, u32> {
    let mut raw = [0u16; PATH];
    let n = if is_directory(handle) {
        unsafe { DoorDirectoryPath(handle, raw.as_mut_ptr() as u64, PATH as u32) }
    } else if abi::file::is_file(handle) {
        unsafe { DoorGetFilePath(handle, raw.as_mut_ptr() as u64, PATH as u32) }
    } else {
        return Err(error::INVALID_HANDLE);
    } as usize;
    if n == 0 || n >= PATH {
        return Err(unsafe { RtlGetLastWin32Error() }.max(1));
    }
    let raw = &raw[..n];
    // A path joined to the current directory is in its native form
    // (`/users/...`), which is C:\Users on Windows.
    let users = raw.len() >= 6
        && raw[0] == u16::from(b'/')
        && raw[1..6]
            .iter()
            .zip(b"users")
            .all(|(&u, &b)| u | 0x20 == u16::from(b))
        && raw.get(6).is_none_or(|&u| u == u16::from(b'/'));
    let mut windows = [0u16; PATH];
    let mut length = 0;
    if users {
        for &u in &[u16::from(b'C'), u16::from(b':')] {
            windows[length] = u;
            length += 1;
        }
    }
    for &u in raw {
        if length + 2 >= PATH {
            return Err(error::INVALID_PARAMETER);
        }
        windows[length] = if u == u16::from(b'/') {
            u16::from(b'\\')
        } else {
            u
        };
        length += 1;
    }
    if users || (length >= 2 && windows[1] == u16::from(b':')) {
        let current: [u16; 3] = [u16::from(b'C'), u16::from(b':'), u16::from(b'\\')];
        return abi::path::full_path(&windows[..length], &current, out)
            .ok_or(error::INVALID_PARAMETER);
    }
    out[..length].copy_from_slice(&windows[..length]);
    Ok(length)
}

/// GetFinalPathNameByHandleW: VOLUME_NAME_DOS (`\\?\C:\...`, the
/// default), VOLUME_NAME_GUID, VOLUME_NAME_NT or VOLUME_NAME_NONE.
#[unsafe(no_mangle)]
pub extern "system" fn GetFinalPathNameByHandleW(
    handle: usize,
    buffer: usize,
    capacity: u32,
    flags: u32,
) -> u32 {
    let handle = abi::widen(handle);
    let buffer = abi::widen(buffer);
    let mut path = [0u16; PATH];
    let length = match handle_path(handle, &mut path) {
        Ok(n) => n,
        Err(e) => return fail(e) as u32,
    };
    if flags & !0xF != 0 {
        return fail(error::INVALID_PARAMETER) as u32;
    }
    let drive = length >= 2 && path[1] == u16::from(b':');
    let rest = if drive {
        &path[2..length]
    } else {
        &path[..length]
    };
    let mut out = [0u16; PATH + 64];
    let mut n = 0;
    let mut push = |text: &[u16], n: &mut usize| {
        for &u in text {
            if *n < out.len() {
                out[*n] = u;
            }
            *n += 1;
        }
    };
    let ascii = |s: &str, buf: &mut [u16; 64]| -> usize {
        for (i, b) in s.bytes().enumerate() {
            buf[i] = u16::from(b);
        }
        s.len()
    };
    let mut prefix = [0u16; 64];
    match flags & 0x7 {
        // VOLUME_NAME_DOS
        0 => {
            let k = ascii("\\\\?\\", &mut prefix);
            push(&prefix[..k], &mut n);
            push(&path[..length], &mut n);
        }
        // VOLUME_NAME_GUID
        1 if drive => {
            let k = ascii(
                "\\\\?\\Volume{d00d0000-0000-4000-8000-000000000c0d}",
                &mut prefix,
            );
            push(&prefix[..k], &mut n);
            push(rest, &mut n);
        }
        // VOLUME_NAME_NT
        2 if drive => {
            let k = ascii("\\Device\\HarddiskVolume3", &mut prefix);
            push(&prefix[..k], &mut n);
            push(rest, &mut n);
        }
        // VOLUME_NAME_NONE
        4 => push(rest, &mut n),
        _ => return fail(error::INVALID_PARAMETER) as u32,
    }
    if n >= out.len() {
        return fail(error::INVALID_PARAMETER) as u32;
    }
    if capacity as usize <= n || buffer == 0 {
        return n as u32 + 1;
    }
    out[n] = 0;
    let bytes: &[u8] =
        unsafe { core::slice::from_raw_parts(out.as_ptr() as *const u8, (n + 1) * 2) };
    match write(buffer, bytes) {
        Ok(()) => n as u32,
        Err(e) => fail(e) as u32,
    }
}
/// GetFinalPathNameByHandleA: the same through the ANSI code page (UTF-8).
#[unsafe(no_mangle)]
pub extern "system" fn GetFinalPathNameByHandleA(
    handle: usize,
    buffer: usize,
    capacity: u32,
    flags: u32,
) -> u32 {
    let mut wide = [0u16; PATH + 64];
    let n = GetFinalPathNameByHandleW(handle, wide.as_mut_ptr() as usize, wide.len() as u32, flags)
        as usize;
    if n == 0 || n >= wide.len() {
        return n as u32;
    }
    let mut text = [0u8; 4 * (PATH + 64)];
    let mut length = 0;
    for c in char::decode_utf16(wide[..n].iter().copied()) {
        let c = c.unwrap_or(char::REPLACEMENT_CHARACTER);
        length += c.encode_utf8(&mut text[length..]).len();
    }
    if capacity as usize <= length || buffer == 0 {
        return length as u32 + 1;
    }
    text[length] = 0;
    match write(abi::widen(buffer), &text[..=length]) {
        Ok(()) => length as u32,
        Err(e) => fail(e) as u32,
    }
}

/// BY_HANDLE_FILE_INFORMATION.
#[unsafe(no_mangle)]
pub extern "system" fn GetFileInformationByHandle(handle: usize, out: usize) -> i32 {
    let i = match information(abi::widen(handle)) {
        Ok(i) => i,
        Err(e) => return fail(e),
    };
    let mut b = [0u8; 52];
    b[..4].copy_from_slice(&i.attributes.to_le_bytes());
    b[4..12].copy_from_slice(&i.created.to_le_bytes());
    b[12..20].copy_from_slice(&i.accessed.to_le_bytes());
    b[20..28].copy_from_slice(&i.written.to_le_bytes());
    b[28..32].copy_from_slice(&VOLUME_SERIAL.to_le_bytes());
    b[32..36].copy_from_slice(&((i.bytes >> 32) as u32).to_le_bytes());
    b[36..40].copy_from_slice(&(i.bytes as u32).to_le_bytes());
    b[40..44].copy_from_slice(&i.links.to_le_bytes());
    b[44..48].copy_from_slice(&((i.object >> 32) as u32).to_le_bytes());
    b[48..52].copy_from_slice(&(i.object as u32).to_le_bytes());
    match write(abi::widen(out), &b) {
        Ok(()) => 1,
        Err(e) => fail(e),
    }
}
/// GetFileInformationByHandleEx: FileBasicInfo (0), FileStandardInfo (1),
/// FileNameInfo (2), FileAttributeTagInfo (9), FileStorageInfo (16),
/// FileAlignmentInfo (17), FileIdInfo (18).
#[unsafe(no_mangle)]
pub extern "system" fn GetFileInformationByHandleEx(
    handle: usize,
    class: u32,
    out: usize,
    bytes: u32,
) -> i32 {
    let handle = abi::widen(handle);
    let out = abi::widen(out);
    let mut b = [0u8; 2 * PATH + 8];
    let length = match class {
        0 | 1 | 9 | 18 => {
            let i = match information(handle) {
                Ok(i) => i,
                Err(e) => return fail(e),
            };
            match class {
                0 => {
                    b[..8].copy_from_slice(&i.created.to_le_bytes());
                    b[8..16].copy_from_slice(&i.accessed.to_le_bytes());
                    b[16..24].copy_from_slice(&i.written.to_le_bytes());
                    b[24..32].copy_from_slice(&i.written.to_le_bytes());
                    b[32..36].copy_from_slice(&i.attributes.to_le_bytes());
                    40
                }
                1 => {
                    let allocated = i.bytes.div_ceil(4096) * 4096;
                    b[..8].copy_from_slice(&allocated.to_le_bytes());
                    b[8..16].copy_from_slice(&i.bytes.to_le_bytes());
                    b[16..20].copy_from_slice(&i.links.to_le_bytes());
                    b[21] = u8::from(i.attributes & FILE_ATTRIBUTE_DIRECTORY != 0);
                    24
                }
                9 => {
                    b[..4].copy_from_slice(&i.attributes.to_le_bytes());
                    8
                }
                _ => {
                    b[..8].copy_from_slice(&u64::from(VOLUME_SERIAL).to_le_bytes());
                    b[8..16].copy_from_slice(&i.object.to_le_bytes());
                    24
                }
            }
        }
        // FileNameInfo: the path without its drive, a byte count first.
        2 => {
            let mut path = [0u16; PATH];
            let n = match handle_path(handle, &mut path) {
                Ok(n) => n,
                Err(e) => return fail(e),
            };
            let skip = if n >= 2 && path[1] == u16::from(b':') {
                2
            } else {
                0
            };
            let name = &path[skip..n];
            b[..4].copy_from_slice(&((name.len() * 2) as u32).to_le_bytes());
            for (k, &u) in name.iter().enumerate() {
                b[4 + 2 * k..6 + 2 * k].copy_from_slice(&u.to_le_bytes());
            }
            let full = 4 + 2 * name.len();
            // A short buffer gets what fits and ERROR_MORE_DATA.
            if (bytes as usize) < full {
                if bytes >= 4 {
                    let _ = write(out, &b[..bytes as usize]);
                }
                return fail(234);
            }
            full
        }
        // FileStorageInfo: 512-byte sectors, 4 KiB physical, aligned.
        16 => {
            for (k, v) in [512u32, 4096, 4096, 4096, 0x1F, 0, 0].iter().enumerate() {
                b[4 * k..4 * k + 4].copy_from_slice(&v.to_le_bytes());
            }
            28
        }
        // FileAlignmentInfo: byte alignment.
        17 => 4,
        _ => return fail(error::INVALID_PARAMETER),
    };
    if (bytes as usize) < length {
        return fail(24);
    }
    match write(out, &b[..length]) {
        Ok(()) => 1,
        Err(e) => fail(e),
    }
}
/// SetFileInformationByHandle: FileBasicInfo (0: attributes and times are
/// kept by storage itself; accepted unchanged), FileRenameInfo (3),
/// FileDispositionInfo (4) and Ex (21), FileAllocationInfo (5, nothing to
/// reserve), FileEndOfFileInfo (6).
#[unsafe(no_mangle)]
pub extern "system" fn SetFileInformationByHandle(
    handle: usize,
    class: u32,
    data: usize,
    bytes: u32,
) -> i32 {
    let handle = abi::widen(handle);
    let data = abi::widen(data);
    if !abi::file::is_file(handle) && !is_directory(handle) {
        return fail(error::INVALID_HANDLE);
    }
    let file = abi::file::is_file(handle);
    match class {
        0 if bytes >= 36 => 1,
        5 if bytes >= 8 => 1,
        6 if bytes >= 8 && file => {
            let mut b = [0u8; 8];
            if let Err(e) = read(data, &mut b) {
                return fail(e);
            }
            unsafe { DoorSetEndOfFile(handle, u64::from_le_bytes(b)) }
        }
        4 if bytes >= 1 && file => {
            let mut b = [0u8; 1];
            if let Err(e) = read(data, &mut b) {
                return fail(e);
            }
            unsafe { DoorSetDeleteOnClose(handle, i32::from(b[0])) }
        }
        // FILE_DISPOSITION_INFO_EX: FILE_DISPOSITION_FLAG_DELETE (1).
        21 if bytes >= 4 && file => {
            let mut b = [0u8; 4];
            if let Err(e) = read(data, &mut b) {
                return fail(e);
            }
            unsafe { DoorSetDeleteOnClose(handle, i32::from(b[0] & 1 != 0)) }
        }
        // FILE_RENAME_INFO: flags (ReplaceIfExists), RootDirectory,
        // FileNameLength, FileName.
        3 if file => rename(handle, data, bytes),
        _ => fail(error::INVALID_PARAMETER),
    }
}
fn rename(handle: u64, data: u64, bytes: u32) -> i32 {
    let p = abi::layout::POINTER as u64;
    let header = 2 * p + 4;
    if u64::from(bytes) < header + 2 {
        return fail(error::INVALID_PARAMETER);
    }
    let mut flags = [0u8; 4];
    let mut root = [0u8; 8];
    let mut length = [0u8; 4];
    if read(data, &mut flags).is_err()
        || read(data + p, &mut root[..p as usize]).is_err()
        || read(data + 2 * p, &mut length).is_err()
    {
        return fail(error::NOACCESS);
    }
    let units = u32::from_le_bytes(length) as usize / 2;
    if u64::from_le_bytes(root) != 0 || units == 0 || units >= PATH {
        return fail(error::INVALID_PARAMETER);
    }
    let mut target = [0u16; PATH];
    let raw = unsafe { core::slice::from_raw_parts_mut(target.as_mut_ptr() as *mut u8, units * 2) };
    if read(data + header, raw).is_err() {
        return fail(error::NOACCESS);
    }
    let mut source = [0u16; PATH];
    let n = match handle_path(handle, &mut source) {
        Ok(n) => n,
        Err(e) => return fail(e),
    };
    source[n] = 0;
    target[units] = 0;
    let replace = u32::from(flags[0] != 0);
    unsafe { DoorMoveFileExW(source.as_ptr() as u64, target.as_ptr() as u64, replace) }
}
/// SetEndOfFile: the file ends at the handle's position.
#[unsafe(no_mangle)]
pub extern "system" fn SetEndOfFile(handle: usize) -> i32 {
    let handle = abi::widen(handle);
    let mut position = 0i64;
    if unsafe { DoorSetFilePointerEx(handle, 0, &mut position as *mut i64 as u64, 1) } == 0 {
        return 0;
    }
    unsafe { DoorSetEndOfFile(handle, position as u64) }
}
/// SetFileTime: storage keeps a file's times itself (as
/// SetFileInformationByHandle's FileBasicInfo does), so a valid file or
/// directory handle is accepted unchanged.
#[unsafe(no_mangle)]
pub extern "system" fn SetFileTime(
    handle: usize,
    _created: usize,
    _accessed: usize,
    _written: usize,
) -> i32 {
    let handle = abi::widen(handle);
    if abi::file::is_file(handle) || is_directory(handle) {
        1
    } else {
        fail(error::INVALID_HANDLE)
    }
}
/// GetFileTime: creation, last access and last write FILETIMEs.
#[unsafe(no_mangle)]
pub extern "system" fn GetFileTime(
    handle: usize,
    created: usize,
    accessed: usize,
    written: usize,
) -> i32 {
    let i = match information(abi::widen(handle)) {
        Ok(i) => i,
        Err(e) => return fail(e),
    };
    for (pointer, value) in [
        (created, i.created),
        (accessed, i.accessed),
        (written, i.written),
    ] {
        let pointer = abi::widen(pointer);
        if pointer != 0
            && let Err(e) = write(pointer, &value.to_le_bytes())
        {
            return fail(e);
        }
    }
    1
}
