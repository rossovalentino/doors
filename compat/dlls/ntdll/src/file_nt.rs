//! The NT file information calls programs make directly (libuv, which Node
//! and Electron run on, requires them at startup): NtQueryInformationFile,
//! NtSetInformationFile and NtQueryVolumeInformationFile over Doors' file,
//! directory and pipe handles, and NtDeviceIoControlFile (no device
//! accepts a control code: there are no AFD socket handles).
use super::directories::{DoorDirectoryInformation, DoorDirectoryPath};
use super::files::{
    DoorFileInformation, DoorGetFilePath, DoorMoveFileExW, DoorSetDeleteOnClose, DoorSetEndOfFile,
    DoorSetFilePointerEx,
};
use super::guest::RtlGetLastWin32Error;
use door_windows_abi as abi;

const SUCCESS: u32 = 0;
const BUFFER_OVERFLOW: u32 = 0x8000_0005;
const INVALID_INFO_CLASS: u32 = 0xC000_0003;
const INFO_LENGTH_MISMATCH: u32 = 0xC000_0004;
const ACCESS_VIOLATION: u32 = 0xC000_0005;
const INVALID_HANDLE: u32 = 0xC000_0008;
const INVALID_PARAMETER: u32 = 0xC000_000D;
const INVALID_DEVICE_REQUEST: u32 = 0xC000_0010;
const ACCESS_DENIED: u32 = 0xC000_0022;
const OBJECT_NAME_NOT_FOUND: u32 = 0xC000_0034;
const OBJECT_NAME_COLLISION: u32 = 0xC000_0035;
const FILE_SYNCHRONOUS_IO_NONALERT: u32 = 0x20;
/// FILE_ALL_ACCESS.
const ALL_ACCESS: u32 = 0x001F_01FF;
/// Kernel32's GetVolumeInformation serial number for C:.
const VOLUME_SERIAL: u32 = 0xD005_2026;
const POINTER: usize = core::mem::size_of::<usize>();
/// Path units a name reply holds.
const NAME_UNITS: usize = 1024;

/// A fixed reply buffer (ntdll has no heap of its own to allocate from).
struct Buf {
    data: [u8; 4352],
    len: usize,
}
impl Buf {
    fn zeroed(len: usize) -> Self {
        Self {
            data: [0; 4352],
            len,
        }
    }
    fn u64(&mut self, at: usize, v: u64) {
        self.data[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, at: usize, v: u32) {
        self.data[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn extend(&mut self, bytes: &[u8]) {
        let n = bytes.len().min(self.data.len() - self.len);
        self.data[self.len..self.len + n].copy_from_slice(&bytes[..n]);
        self.len += n;
    }
    /// A length-prefixed UTF-16 name (FILE_NAME_INFORMATION's shape).
    fn name(&mut self, units: &[u16]) {
        self.extend(&((2 * units.len()) as u32).to_le_bytes());
        for u in units {
            self.extend(&u.to_le_bytes());
        }
    }
    fn bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }
}

/// A handle's file record.
struct Stat {
    bytes: u64,
    object: u64,
    attributes: u32,
    links: u32,
    created: u64,
    accessed: u64,
    written: u64,
    directory: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    Directory,
    Pipe,
}
fn kind(handle: u64) -> Option<Kind> {
    if abi::file::is_file(handle) {
        Some(Kind::File)
    } else if abi::pipe::is_pipe(handle) {
        Some(Kind::Pipe)
    } else {
        // Directory handles: their information call validates them.
        let mut probe = [0u8; 48];
        (DoorDirectoryInformation(handle, probe.as_mut_ptr() as u64) != 0)
            .then_some(Kind::Directory)
    }
}
/// The NTSTATUS of the last Win32 error a Door primitive set.
fn last_status() -> u32 {
    match RtlGetLastWin32Error() {
        2 | 3 => OBJECT_NAME_NOT_FOUND,
        5 => ACCESS_DENIED,
        6 => INVALID_HANDLE,
        80 | 183 => OBJECT_NAME_COLLISION,
        87 => INVALID_PARAMETER,
        _ => 0xC000_0185,
    }
}
fn stat(handle: u64, directory: bool) -> Result<Stat, u32> {
    let mut r = [0u8; 48];
    let ok = if directory {
        DoorDirectoryInformation(handle, r.as_mut_ptr() as u64)
    } else {
        DoorFileInformation(handle, r.as_mut_ptr() as u64)
    };
    if ok == 0 {
        return Err(last_status());
    }
    let q = |at: usize| u64::from_le_bytes(r[at..at + 8].try_into().unwrap());
    let d = |at: usize| u32::from_le_bytes(r[at..at + 4].try_into().unwrap());
    Ok(Stat {
        bytes: q(0),
        object: q(8),
        attributes: d(16),
        links: d(20).max(1),
        created: q(24),
        accessed: q(32),
        written: q(40),
        directory,
    })
}
/// The handle's path without its drive (`\dir\file`), as FileNameInformation
/// reports it: (units, length).
fn name(handle: u64, directory: bool) -> ([u16; NAME_UNITS], usize) {
    let mut units = [0u16; NAME_UNITS];
    let n = if directory {
        DoorDirectoryPath(handle, units.as_mut_ptr() as u64, NAME_UNITS as u32)
    } else {
        DoorGetFilePath(handle, units.as_mut_ptr() as u64, NAME_UNITS as u32)
    } as usize;
    let n = if n >= NAME_UNITS { 0 } else { n };
    if n >= 2 && units[1] == u16::from(b':') {
        units.copy_within(2..n, 0);
        (units, n - 2)
    } else {
        (units, n)
    }
}
fn position(handle: u64) -> u64 {
    let mut at = 0i64;
    DoorSetFilePointerEx(handle, 0, &mut at as *mut i64 as u64, 1);
    at.max(0) as u64
}
fn allocation(s: &Stat) -> u64 {
    s.bytes.div_ceil(4096) * 4096
}
/// FILE_BASIC_INFORMATION at `at` (40 bytes).
fn basic(b: &mut Buf, at: usize, s: &Stat) {
    b.u64(at, s.created);
    b.u64(at + 8, s.accessed);
    b.u64(at + 16, s.written);
    b.u64(at + 24, s.written);
    b.u32(at + 32, s.attributes);
}
/// FILE_STANDARD_INFORMATION at `at` (24 bytes).
fn standard(b: &mut Buf, at: usize, s: &Stat) {
    b.u64(at, allocation(s));
    b.u64(at + 8, s.bytes);
    b.u32(at + 16, s.links);
    b.data[at + 21] = u8::from(s.directory);
}
/// The reply for `class` (FILE_INFORMATION_CLASS) and the smallest buffer
/// it needs; a buffer between that and the whole reply takes a truncated
/// copy (STATUS_BUFFER_OVERFLOW: the name did not fit).
fn query(handle: u64, class: u32) -> Result<(Buf, usize), u32> {
    let k = kind(handle).ok_or(INVALID_HANDLE)?;
    if k == Kind::Pipe {
        return pipe_query(class);
    }
    let directory = k == Kind::Directory;
    let s = stat(handle, directory)?;
    let at = if directory { 0 } else { position(handle) };
    let fixed = |len: usize| Buf::zeroed(len);
    Ok(match class {
        4 => {
            let mut b = fixed(40);
            basic(&mut b, 0, &s);
            (b, 40)
        }
        5 => {
            let mut b = fixed(24);
            standard(&mut b, 0, &s);
            (b, 24)
        }
        6 => {
            let mut b = fixed(8);
            b.u64(0, s.object);
            (b, 8)
        }
        7 | 17 => (fixed(4), 4),
        8 => {
            let mut b = fixed(4);
            b.u32(0, ALL_ACCESS);
            (b, 4)
        }
        9 => {
            let (units, n) = name(handle, directory);
            let mut b = fixed(0);
            b.name(&units[..n]);
            (b, 4)
        }
        14 => {
            let mut b = fixed(8);
            b.u64(0, at);
            (b, 8)
        }
        16 => {
            let mut b = fixed(4);
            b.u32(0, FILE_SYNCHRONOUS_IO_NONALERT);
            (b, 4)
        }
        // FILE_ALL_INFORMATION: basic, standard, internal, EA, access,
        // position, mode, alignment, then the name.
        18 => {
            let mut b = fixed(96);
            basic(&mut b, 0, &s);
            standard(&mut b, 40, &s);
            b.u64(64, s.object);
            b.u32(76, ALL_ACCESS);
            b.u64(80, at);
            b.u32(88, FILE_SYNCHRONOUS_IO_NONALERT);
            let (units, n) = name(handle, directory);
            b.name(&units[..n]);
            (b, 100)
        }
        // FILE_NETWORK_OPEN_INFORMATION.
        34 => {
            let mut b = fixed(56);
            b.u64(0, s.created);
            b.u64(8, s.accessed);
            b.u64(16, s.written);
            b.u64(24, s.written);
            b.u64(32, allocation(&s));
            b.u64(40, s.bytes);
            b.u32(48, s.attributes);
            (b, 56)
        }
        // FILE_ATTRIBUTE_TAG_INFORMATION (no reparse points).
        35 => {
            let mut b = fixed(8);
            b.u32(0, s.attributes);
            (b, 8)
        }
        // FILE_ID_INFORMATION: the volume serial and a 128-bit id.
        59 => {
            let mut b = fixed(24);
            b.u64(0, u64::from(VOLUME_SERIAL));
            b.u64(8, s.object);
            (b, 24)
        }
        // FILE_STAT_INFORMATION.
        68 => {
            let mut b = fixed(72);
            b.u64(0, s.object);
            b.u64(8, s.created);
            b.u64(16, s.accessed);
            b.u64(24, s.written);
            b.u64(32, s.written);
            b.u64(40, allocation(&s));
            b.u64(48, s.bytes);
            b.u32(56, s.attributes);
            b.u32(64, s.links);
            b.u32(68, ALL_ACCESS);
            (b, 72)
        }
        _ => return Err(INVALID_INFO_CLASS),
    })
}
/// Anonymous pipes: synchronous, readable and writable ends; the local
/// pipe information of a connected byte-stream pipe.
fn pipe_query(class: u32) -> Result<(Buf, usize), u32> {
    Ok(match class {
        4 => (Buf::zeroed(40), 40),
        5 => {
            let mut b = Buf::zeroed(24);
            b.u32(16, 1);
            (b, 24)
        }
        // FILE_GENERIC_READ | FILE_GENERIC_WRITE.
        8 => {
            let mut b = Buf::zeroed(4);
            b.u32(0, 0x0012_019F);
            (b, 4)
        }
        16 => {
            let mut b = Buf::zeroed(4);
            b.u32(0, FILE_SYNCHRONOUS_IO_NONALERT);
            (b, 4)
        }
        // FILE_PIPE_LOCAL_INFORMATION: byte stream, one instance, 4 KiB
        // quotas, connected (3), the client end (0).
        24 => {
            let mut b = Buf::zeroed(40);
            b.u32(8, 1);
            b.u32(12, 1);
            b.u32(16, 4096);
            b.u32(24, 4096);
            b.u32(28, 4096);
            b.u32(32, 3);
            (b, 40)
        }
        _ => return Err(INVALID_INFO_CLASS),
    })
}
/// IO_STATUS_BLOCK { Status (a pointer-sized union), Information }.
fn io_status(block: u64, status: u32, information: usize) {
    if block == 0 {
        return;
    }
    let mut b = [0u8; 2 * POINTER];
    b[..4].copy_from_slice(&status.to_le_bytes());
    b[POINTER..2 * POINTER].copy_from_slice(&information.to_le_bytes()[..POINTER]);
    let _ = super::guest::put(block, &b);
}
fn reply(block: u64, out: u64, length: u32, data: &[u8], minimum: usize) -> u32 {
    let length = length as usize;
    if length < minimum {
        io_status(block, INFO_LENGTH_MISMATCH, 0);
        return INFO_LENGTH_MISMATCH;
    }
    let n = data.len().min(length);
    if out == 0 || super::guest::put(out, &data[..n]).is_err() {
        return ACCESS_VIOLATION;
    }
    let status = if n < data.len() {
        BUFFER_OVERFLOW
    } else {
        SUCCESS
    };
    io_status(block, status, n);
    status
}
#[unsafe(no_mangle)]
pub extern "system" fn NtQueryInformationFile(
    handle: usize,
    block: usize,
    out: usize,
    length: u32,
    class: u32,
) -> u32 {
    let (handle, block, out) = (abi::widen(handle), abi::widen(block), abi::widen(out));
    match query(handle, class) {
        Ok((data, minimum)) => reply(block, out, length, data.bytes(), minimum),
        Err(status) => {
            io_status(block, status, 0);
            status
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ZwQueryInformationFile(
    handle: usize,
    block: usize,
    out: usize,
    length: u32,
    class: u32,
) -> u32 {
    NtQueryInformationFile(handle, block, out, length, class)
}

/// The record a set call passes, copied in (at most 4 KiB).
fn record(data: u64, length: u32) -> Result<([u8; 4096], usize), u32> {
    let mut bytes = [0u8; 4096];
    let n = (length as usize).min(bytes.len());
    if n != 0 && super::guest::copy(bytes.as_mut_ptr() as u64, data, n).is_err() {
        return Err(ACCESS_VIOLATION);
    }
    Ok((bytes, n))
}
/// FILE_RENAME_INFORMATION(_EX) { ReplaceIfExists/Flags, RootDirectory,
/// FileNameLength, FileName }: the open file moves to the new name (an NT
/// `\??\C:\...` path or a Win32 one).
fn rename(handle: u64, bytes: &[u8], ex: bool) -> Result<(), u32> {
    let header = 2 * POINTER + 4;
    if bytes.len() < header {
        return Err(INFO_LENGTH_MISMATCH);
    }
    let replace = if ex {
        u32::from_le_bytes(bytes[..4].try_into().unwrap()) & 1
    } else {
        u32::from(bytes[0] != 0)
    };
    let length =
        u32::from_le_bytes(bytes[2 * POINTER..2 * POINTER + 4].try_into().unwrap()) as usize / 2;
    if bytes.len() < header + 2 * length || length == 0 || length >= NAME_UNITS {
        return Err(INVALID_PARAMETER);
    }
    let mut target = [0u16; NAME_UNITS];
    for (i, c) in bytes[header..header + 2 * length]
        .chunks_exact(2)
        .enumerate()
    {
        target[i] = u16::from_le_bytes([c[0], c[1]]);
    }
    let skip = if target[..4] == [0x5C, 0x3F, 0x3F, 0x5C] {
        4
    } else {
        0
    };
    target[length] = 0;
    let mut source = [0u16; NAME_UNITS];
    let n = DoorGetFilePath(handle, source.as_mut_ptr() as u64, NAME_UNITS as u32) as usize;
    if n == 0 || n >= NAME_UNITS {
        return Err(last_status());
    }
    source[n] = 0;
    if DoorMoveFileExW(
        source.as_ptr() as u64,
        target[skip..].as_ptr() as u64,
        replace,
    ) == 0
    {
        return Err(last_status());
    }
    Ok(())
}
fn set(handle: u64, data: u64, length: u32, class: u32) -> Result<(), u32> {
    let k = kind(handle).ok_or(INVALID_HANDLE)?;
    let file = k == Kind::File;
    let (bytes, n) = record(data, length)?;
    let bytes = &bytes[..n];
    let need = |size: usize| {
        if n < size {
            Err(INFO_LENGTH_MISMATCH)
        } else {
            Ok(())
        }
    };
    let ok = |r: i32| if r != 0 { Ok(()) } else { Err(last_status()) };
    match class {
        // Attributes and times are kept by storage itself.
        4 => need(40),
        // FILE_DISPOSITION_INFORMATION { DeleteFile }.
        13 if file => {
            need(1)?;
            ok(DoorSetDeleteOnClose(handle, i32::from(bytes[0] != 0)))
        }
        // FILE_DISPOSITION_INFORMATION_EX { Flags }: DELETE (1).
        64 if file => {
            need(4)?;
            ok(DoorSetDeleteOnClose(handle, i32::from(bytes[0] & 1 != 0)))
        }
        // FILE_POSITION_INFORMATION.
        14 if file => {
            need(8)?;
            let at = i64::from_le_bytes(bytes[..8].try_into().unwrap());
            ok(DoorSetFilePointerEx(handle, at, 0, 0))
        }
        // FILE_END_OF_FILE_INFORMATION; allocation has nothing to reserve.
        20 if file => {
            need(8)?;
            ok(DoorSetEndOfFile(
                handle,
                u64::from_le_bytes(bytes[..8].try_into().unwrap()),
            ))
        }
        19 => need(8),
        16 => need(4),
        10 if file => rename(handle, bytes, false),
        65 if file => rename(handle, bytes, true),
        13 | 64 | 14 | 20 | 10 | 65 => Err(INVALID_PARAMETER),
        _ => Err(INVALID_INFO_CLASS),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn NtSetInformationFile(
    handle: usize,
    block: usize,
    data: usize,
    length: u32,
    class: u32,
) -> u32 {
    let (handle, block, data) = (abi::widen(handle), abi::widen(block), abi::widen(data));
    let status = set(handle, data, length, class).err().unwrap_or(SUCCESS);
    io_status(block, status, 0);
    status
}
#[unsafe(no_mangle)]
pub extern "system" fn ZwSetInformationFile(
    handle: usize,
    block: usize,
    data: usize,
    length: u32,
    class: u32,
) -> u32 {
    NtSetInformationFile(handle, block, data, length, class)
}

/// FS_INFORMATION_CLASS: volume (1), size (3), device (4), attribute (5),
/// full size (7).
fn volume(handle: u64, class: u32) -> Result<(Buf, usize), u32> {
    let pipe = kind(handle).ok_or(INVALID_HANDLE)? == Kind::Pipe;
    Ok(match class {
        // FILE_FS_VOLUME_INFORMATION { CreationTime, SerialNumber,
        // LabelLength, SupportsObjects, Label }: "Windows".
        1 if !pipe => {
            let mut b = Buf::zeroed(18);
            b.u32(8, VOLUME_SERIAL);
            b.u32(12, 14);
            for u in "Windows".encode_utf16() {
                b.extend(&u.to_le_bytes());
            }
            (b, 18)
        }
        // A 64 GiB volume of 4 KiB clusters, half free.
        3 => {
            let mut b = Buf::zeroed(24);
            b.u64(0, 16 << 20);
            b.u64(8, 8 << 20);
            b.u32(16, 8);
            b.u32(20, 512);
            (b, 24)
        }
        7 => {
            let mut b = Buf::zeroed(32);
            b.u64(0, 16 << 20);
            b.u64(8, 8 << 20);
            b.u64(16, 8 << 20);
            b.u32(24, 8);
            b.u32(28, 512);
            (b, 32)
        }
        // FILE_FS_DEVICE_INFORMATION: a disk, or a named-pipe device.
        4 => {
            let mut b = Buf::zeroed(8);
            b.u32(0, if pipe { 0x11 } else { 7 });
            (b, 8)
        }
        // FILE_FS_ATTRIBUTE_INFORMATION: case-preserving Unicode names of
        // up to 255 units on "NTFS".
        5 if !pipe => {
            let mut b = Buf::zeroed(8);
            b.u32(0, 0x0000_0007);
            b.u32(4, 255);
            b.name(&[0x4E, 0x54, 0x46, 0x53]);
            (b, 12)
        }
        _ => return Err(INVALID_INFO_CLASS),
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn NtQueryVolumeInformationFile(
    handle: usize,
    block: usize,
    out: usize,
    length: u32,
    class: u32,
) -> u32 {
    let (handle, block, out) = (abi::widen(handle), abi::widen(block), abi::widen(out));
    match volume(handle, class) {
        Ok((data, minimum)) => reply(block, out, length, data.bytes(), minimum),
        Err(status) => {
            io_status(block, status, 0);
            status
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ZwQueryVolumeInformationFile(
    handle: usize,
    block: usize,
    out: usize,
    length: u32,
    class: u32,
) -> u32 {
    NtQueryVolumeInformationFile(handle, block, out, length, class)
}
/// NtDeviceIoControlFile: no Doors handle is a device that takes control
/// codes (Winsock sockets are not AFD handles), so every request is
/// STATUS_INVALID_DEVICE_REQUEST, as Windows answers an unknown code.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn NtDeviceIoControlFile(
    handle: usize,
    _event: usize,
    _apc: usize,
    _context: usize,
    block: usize,
    _code: u32,
    _input: usize,
    _input_bytes: u32,
    _output: usize,
    _output_bytes: u32,
) -> u32 {
    let status = if kind(abi::widen(handle)).is_some() {
        INVALID_DEVICE_REQUEST
    } else {
        INVALID_HANDLE
    };
    io_status(abi::widen(block), status, 0);
    status
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn NtFsControlFile(
    handle: usize,
    event: usize,
    apc: usize,
    context: usize,
    block: usize,
    code: u32,
    input: usize,
    input_bytes: u32,
    output: usize,
    output_bytes: u32,
) -> u32 {
    NtDeviceIoControlFile(
        handle,
        event,
        apc,
        context,
        block,
        code,
        input,
        input_bytes,
        output,
        output_bytes,
    )
}
