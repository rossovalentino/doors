//! FTP (INTERNET_SERVICE_FTP and ftp:// URLs), the commands as Windows'
//! WinINet sends them (measured: USER/PASS when connecting, PWD for the
//! current directory, TYPE A + PASV + LIST to find files, TYPE I + PASV +
//! SIZE + RETR to read one, TYPE I + PASV + STOR to write, RNFR/RNTO,
//! DELE, MKD, CWD, RMD). Data connections are passive, to the control
//! connection's own peer (whatever address PASV names, as current clients
//! do; active mode is not offered). A server's refusal is
//! ERROR_INTERNET_EXTENDED_ERROR with its reply in
//! InternetGetLastResponseInfo.
use super::{
    Common, Object, Text, done, encode, error, fail, handle_created, insert, read_u32, session,
    slot, text, write, write_u32,
};
use alloc::{string::String, sync::Arc, vec, vec::Vec};
use core::sync::atomic::AtomicBool;
use door_windows_inet::ftp::{
    Listed, Replies, Reply, extended_passive, listing_line, passive, quoted_path,
};
use door_windows_inet::guest::{Deadline, Stream, resolve};
use door_windows_inet::sync::Lock;

/// ERROR_INTERNET_* and ERROR_FTP_* codes FTP adds.
mod code {
    pub const EXTENDED_ERROR: u32 = 12003;
    pub const INCORRECT_USER_NAME: u32 = 12013;
    pub const INCORRECT_PASSWORD: u32 = 12014;
    pub const TRANSFER_IN_PROGRESS: u32 = 12110;
    pub const NO_MORE_FILES: u32 = 18;
    pub const FILE_EXISTS: u32 = 80;
}
const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
/// FTP_TRANSFER_TYPE_ASCII.
const TRANSFER_ASCII: u32 = 0x1;
/// The control connection's answers and data transfers may take this long.
const TIMEOUT: u32 = 30_000;

pub(super) struct FtpSession {
    control: Stream,
    replies: Replies,
    /// The control connection's peer: data connections go here.
    address: [u8; 4],
    /// A file or find handle holds the data connection.
    busy: bool,
}
pub(super) struct FtpFile {
    session: Arc<Lock<FtpSession>>,
    data: Option<Stream>,
    writing: bool,
    /// SIZE's answer, for FtpGetFileSize.
    size: Option<u64>,
    /// Read to its end.
    ended: bool,
    /// Bytes received and not yet read.
    buffer: Vec<u8>,
}
pub(super) struct FtpFind {
    session: Arc<Lock<FtpSession>>,
    entries: Vec<Listed>,
    at: usize,
}

/// The last reply's text, per process (InternetGetLastResponseInfo).
static LAST: Lock<String> = Lock::new(String::new());
fn remember(reply: &Reply) {
    let text = alloc::format!("{} {}\r\n", reply.code, reply.text);
    LAST.with(|l| *l = text);
}

impl FtpSession {
    fn reply(&mut self) -> Result<Reply, u32> {
        let deadline = Deadline::after(TIMEOUT);
        loop {
            match self.replies.take() {
                Ok(Some(r)) => {
                    remember(&r);
                    return Ok(r);
                }
                Ok(None) => (),
                Err(_) => return Err(error::CONNECTION_ABORTED),
            }
            let bytes = self
                .control
                .read(deadline, &|| false)
                .map_err(|_| error::CONNECTION_ABORTED)?;
            if bytes.is_empty() || self.replies.push(&bytes).is_err() {
                return Err(error::CONNECTION_ABORTED);
            }
        }
    }
    /// Send one command and take its final reply (1xx preliminaries are
    /// left for the caller of a transfer).
    fn command(&mut self, line: &str) -> Result<Reply, u32> {
        let mut bytes = Vec::from(line.as_bytes());
        bytes.extend_from_slice(b"\r\n");
        self.control
            .write_all(&bytes, Deadline::after(TIMEOUT), &|| false)
            .map_err(|_| error::CONNECTION_ABORTED)?;
        self.reply()
    }
    /// A command that must succeed (2xx, or 3xx where `more` allows).
    fn expect(&mut self, line: &str, more: bool) -> Result<Reply, u32> {
        let r = self.command(line)?;
        match r.class() {
            2 => Ok(r),
            3 if more => Ok(r),
            _ => Err(code::EXTENDED_ERROR),
        }
    }
    /// A passive data connection (PASV, or EPSV when the server refuses it).
    fn data(&mut self) -> Result<Stream, u32> {
        let r = self.command("PASV")?;
        let port = if r.code == 227 {
            passive(&r.text).map(|(_, p)| p)
        } else {
            let r = self.command("EPSV")?;
            (r.code == 229).then(|| extended_passive(&r.text)).flatten()
        }
        .ok_or(code::EXTENDED_ERROR)?;
        Stream::connect(self.address, port, Deadline::after(TIMEOUT), &|| false)
            .map_err(|_| error::CANNOT_CONNECT)
    }
    /// Start a transfer: TYPE, a data connection, then `line` (RETR, STOR,
    /// LIST), which must answer 1xx.
    fn transfer(&mut self, ascii: bool, line: &str) -> Result<Stream, u32> {
        self.expect(if ascii { "TYPE A" } else { "TYPE I" }, false)?;
        let data = self.data()?;
        let r = self.command(line)?;
        if r.class() != 1 {
            return Err(code::EXTENDED_ERROR);
        }
        Ok(data)
    }
}

/// Connect and log in (InternetConnect for INTERNET_SERVICE_FTP).
pub(super) fn open_session(
    host: &str,
    port: u16,
    user: &str,
    password: &str,
) -> Result<FtpSession, u32> {
    let address = resolve(host, Deadline::after(TIMEOUT)).map_err(|_| error::NAME_NOT_RESOLVED)?;
    let port = if port == 0 { 21 } else { port };
    let control = Stream::connect(address, port, Deadline::after(TIMEOUT), &|| false)
        .map_err(|_| error::CANNOT_CONNECT)?;
    let mut s = FtpSession {
        control,
        replies: Replies::default(),
        address,
        busy: false,
    };
    let greeting = s.reply()?;
    if greeting.class() != 2 {
        return Err(code::EXTENDED_ERROR);
    }
    let (user, password) = if user.is_empty() {
        (
            "anonymous",
            if password.is_empty() {
                "IEUser@"
            } else {
                password
            },
        )
    } else {
        (user, password)
    };
    let r = s.command(&alloc::format!("USER {user}"))?;
    match r.class() {
        2 => return Ok(s),
        3 => (),
        _ => return Err(code::INCORRECT_USER_NAME),
    }
    let r = s.command(&alloc::format!("PASS {password}"))?;
    if r.class() != 2 {
        return Err(code::INCORRECT_PASSWORD);
    }
    Ok(s)
}

fn ftp_session(handle: u64) -> Result<Arc<Lock<FtpSession>>, u32> {
    slot(handle, |s| match &s.object {
        Object::Ftp(x) => Ok(x.clone()),
        _ => Err(error::INCORRECT_HANDLE_TYPE),
    })
    .unwrap_or(Err(error::INVALID_HANDLE))
}
fn add_handle(parent: u64, object: Object, context: u64) -> u64 {
    let callback = super::common(parent).map_or(0, |c| c.callback);
    let h = insert(
        Common {
            callback,
            context,
            parent,
        },
        object,
        Arc::new(AtomicBool::new(false)),
    );
    handle_created(h);
    h
}
/// The connection handle InternetConnect answers for FTP.
pub(super) fn connect(
    parent: u64,
    host: &str,
    port: u16,
    user: &str,
    password: &str,
    context: u64,
) -> Result<u64, u32> {
    session(parent)?;
    let s = open_session(host, port, user, password)?;
    Ok(add_handle(
        parent,
        Object::Ftp(Arc::new(Lock::new(s))),
        context,
    ))
}

/// A simple command on a connection handle that must succeed.
fn simple(
    handle: u64,
    line: impl FnOnce() -> Result<String, u32>,
    more: bool,
) -> Result<Reply, u32> {
    let s = ftp_session(handle)?;
    let line = line()?;
    s.with(|s| {
        if s.busy {
            return Err(code::TRANSFER_IN_PROGRESS);
        }
        s.expect(&line, more)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn FtpGetCurrentDirectoryW(handle: usize, out: usize, length: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    get_current_directory(handle, out, length, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn FtpGetCurrentDirectoryA(handle: usize, out: usize, length: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    get_current_directory(handle, out, length, Text::Ansi)
}
/// PWD; `length` (characters) counts the terminating NUL on return, as
/// measured on Windows.
fn get_current_directory(handle: u64, out: u64, length: u64, kind: Text) -> i32 {
    let result = (|| {
        let capacity = read_u32(length)? as usize;
        let reply = simple(handle, || Ok(String::from("PWD")), false)?;
        let path = quoted_path(&reply.text).ok_or(code::EXTENDED_ERROR)?;
        let bytes = encode(&path, kind);
        let units = bytes.len() / super::unit(kind) as usize;
        write_u32(length, units as u32)?;
        if units > capacity {
            return Err(error::INSUFFICIENT_BUFFER);
        }
        write(out, &bytes)
    })();
    done(result)
}
macro_rules! path_command {
    ($wide:ident, $ansi:ident, $verb:literal) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn $wide(handle: usize, path: usize) -> i32 {
            let handle = door_windows_abi::widen(handle);
            let path = door_windows_abi::widen(path);
            {
                done(
                    simple(
                        handle,
                        || {
                            Ok(alloc::format!(
                                "{} {}",
                                $verb,
                                text(path, u32::MAX, Text::Wide)?
                            ))
                        },
                        false,
                    )
                    .map(|_| ()),
                )
            }
        }
        #[unsafe(no_mangle)]
        pub extern "system" fn $ansi(handle: usize, path: usize) -> i32 {
            let handle = door_windows_abi::widen(handle);
            let path = door_windows_abi::widen(path);
            {
                done(
                    simple(
                        handle,
                        || {
                            Ok(alloc::format!(
                                "{} {}",
                                $verb,
                                text(path, u32::MAX, Text::Ansi)?
                            ))
                        },
                        false,
                    )
                    .map(|_| ()),
                )
            }
        }
    };
}
path_command!(FtpSetCurrentDirectoryW, FtpSetCurrentDirectoryA, "CWD");
path_command!(FtpCreateDirectoryW, FtpCreateDirectoryA, "MKD");
path_command!(FtpRemoveDirectoryW, FtpRemoveDirectoryA, "RMD");
path_command!(FtpDeleteFileW, FtpDeleteFileA, "DELE");

fn rename(handle: u64, from: u64, to: u64, kind: Text) -> i32 {
    let result = (|| {
        let (from, to) = (text(from, u32::MAX, kind)?, text(to, u32::MAX, kind)?);
        let s = ftp_session(handle)?;
        s.with(|s| {
            if s.busy {
                return Err(code::TRANSFER_IN_PROGRESS);
            }
            let r = s.command(&alloc::format!("RNFR {from}"))?;
            if r.class() != 3 {
                return Err(code::EXTENDED_ERROR);
            }
            s.expect(&alloc::format!("RNTO {to}"), false).map(|_| ())
        })
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn FtpRenameFileW(handle: usize, from: usize, to: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let from = door_windows_abi::widen(from);
    let to = door_windows_abi::widen(to);
    rename(handle, from, to, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn FtpRenameFileA(handle: usize, from: usize, to: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let from = door_windows_abi::widen(from);
    let to = door_windows_abi::widen(to);
    rename(handle, from, to, Text::Ansi)
}

// ---- files -----------------------------------------------------------------------------
/// Open `path` on a session for reading (SIZE, RETR) or writing (STOR).
fn open(
    s: &mut FtpSession,
    path: &str,
    writing: bool,
    ascii: bool,
) -> Result<(Stream, Option<u64>), u32> {
    if s.busy {
        return Err(code::TRANSFER_IN_PROGRESS);
    }
    if writing {
        let data = s.transfer(ascii, &alloc::format!("STOR {path}"))?;
        return Ok((data, None));
    }
    s.expect(if ascii { "TYPE A" } else { "TYPE I" }, false)?;
    let data = s.data()?;
    let size = s
        .command(&alloc::format!("SIZE {path}"))
        .ok()
        .filter(|r| r.code == 213)
        .and_then(|r| r.text.trim().parse().ok());
    let r = s.command(&alloc::format!("RETR {path}"))?;
    if r.class() != 1 {
        return Err(code::EXTENDED_ERROR);
    }
    Ok((data, size))
}
fn open_file(handle: u64, name: u64, access: u32, flags: u32, context: u64, kind: Text) -> u64 {
    let result = (|| {
        let path = text(name, u32::MAX, kind)?;
        let writing = match access {
            GENERIC_READ => false,
            GENERIC_WRITE => true,
            _ => return Err(error::INVALID_PARAMETER),
        };
        let session = ftp_session(handle)?;
        let (data, size) = session.with(|s| {
            let opened = open(s, &path, writing, flags & 0x3 == TRANSFER_ASCII)?;
            s.busy = true;
            Ok::<_, u32>(opened)
        })?;
        let file = FtpFile {
            session,
            data: Some(data),
            writing,
            size,
            ended: false,
            buffer: Vec::new(),
        };
        Ok(add_handle(
            handle,
            Object::FtpFile(Arc::new(Lock::new(file))),
            context,
        ))
    })();
    match result {
        Ok(h) => {
            unsafe { super::RtlSetLastWin32Error(0) };
            h
        }
        Err(e) => fail(e, 0),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn FtpOpenFileW(
    handle: usize,
    name: usize,
    access: u32,
    flags: u32,
    context: usize,
) -> usize {
    let handle = door_windows_abi::widen(handle);
    let name = door_windows_abi::widen(name);
    let context = door_windows_abi::widen(context);
    door_windows_abi::narrow((move || -> u64 {
        open_file(handle, name, access, flags, context, Text::Wide)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn FtpOpenFileA(
    handle: usize,
    name: usize,
    access: u32,
    flags: u32,
    context: usize,
) -> usize {
    let handle = door_windows_abi::widen(handle);
    let name = door_windows_abi::widen(name);
    let context = door_windows_abi::widen(context);
    door_windows_abi::narrow((move || -> u64 {
        open_file(handle, name, access, flags, context, Text::Ansi)
    })())
}
pub(super) fn file(handle: u64) -> Option<Arc<Lock<FtpFile>>> {
    slot(handle, |s| match &s.object {
        Object::FtpFile(f) => Some(f.clone()),
        _ => None,
    })
    .flatten()
}
/// InternetReadFile on an FTP file: up to `length` bytes, 0 at the end.
pub(super) fn read_file(f: &Arc<Lock<FtpFile>>, length: usize) -> Result<Vec<u8>, u32> {
    f.with(|f| {
        if f.writing {
            return Err(error::INVALID_OPERATION);
        }
        while f.buffer.len() < length.max(1) && !f.ended {
            let Some(data) = f.data.as_mut() else {
                break;
            };
            let bytes = data
                .read(Deadline::after(TIMEOUT), &|| false)
                .map_err(|_| error::CONNECTION_ABORTED)?;
            if bytes.is_empty() {
                f.ended = true;
                break;
            }
            f.buffer.extend_from_slice(&bytes);
        }
        // More than asked stays for the next read.
        let n = length.min(f.buffer.len());
        Ok(f.buffer.drain(..n).collect())
    })
}
/// InternetWriteFile on an FTP file.
pub(super) fn write_file(f: &Arc<Lock<FtpFile>>, data: &[u8]) -> Result<(), u32> {
    f.with(|f| {
        if !f.writing {
            return Err(error::INVALID_OPERATION);
        }
        let stream = f.data.as_mut().ok_or(error::CONNECTION_ABORTED)?;
        stream
            .write_all(data, Deadline::after(TIMEOUT), &|| false)
            .map_err(|_| error::CONNECTION_ABORTED)
    })
}
/// Closing an FTP file ends its transfer: the data connection closes and
/// the server's 226 is read.
pub(super) fn close_file(f: &Arc<Lock<FtpFile>>) {
    let session = f.with(|f| {
        if let Some(mut data) = f.data.take() {
            if f.writing {
                data.shutdown();
            }
            drop(data);
        }
        f.session.clone()
    });
    session.with(|s| {
        let _ = s.reply();
        s.busy = false;
    });
}
#[unsafe(no_mangle)]
pub extern "system" fn FtpGetFileSize(handle: usize, high: usize) -> u32 {
    let handle = door_windows_abi::widen(handle);
    let high = door_windows_abi::widen(high);
    {
        let result = (|| {
            let f = file(handle).ok_or(error::INCORRECT_HANDLE_TYPE)?;
            let size = f.with(|f| f.size).ok_or(error::INVALID_OPERATION)?;
            if high != 0 {
                write_u32(high, (size >> 32) as u32)?;
            }
            Ok(size as u32)
        })();
        match result {
            Ok(low) => {
                unsafe { super::RtlSetLastWin32Error(0) };
                low
            }
            Err(e) => fail(e, u32::MAX),
        }
    }
}

// ---- finding ---------------------------------------------------------------------------
/// WIN32_FIND_DATA(W/A) for a listed entry.
fn find_data(entry: &Listed, kind: Text) -> Vec<u8> {
    const DIRECTORY: u32 = 0x10;
    const NORMAL: u32 = 0x80;
    let (name_units, size) = match kind {
        Text::Wide => (260, 592),
        Text::Ansi => (260, 320),
    };
    let mut b = vec![0u8; size];
    let attributes = if entry.directory { DIRECTORY } else { NORMAL };
    b[..4].copy_from_slice(&attributes.to_le_bytes());
    let (y, mo, d, h, mi) = entry.time;
    let year = if y == 0 { 2026 } else { y };
    let filetime = filetime(year, mo, d, h, mi);
    // Creation, access and write times: the listing gives one.
    for at in [4, 12, 20] {
        b[at..at + 8].copy_from_slice(&filetime.to_le_bytes());
    }
    b[28..32].copy_from_slice(&((entry.size >> 32) as u32).to_le_bytes());
    b[32..36].copy_from_slice(&(entry.size as u32).to_le_bytes());
    let name = encode(&entry.name, kind);
    let unit = super::unit(kind) as usize;
    let max = (name_units - 1) * unit;
    let n = name.len().min(max) - (name.len().min(max) % unit);
    b[44..44 + n].copy_from_slice(&name[..n]);
    b
}
/// FILETIME of a civil date (UTC).
fn filetime(year: u16, month: u8, day: u8, hour: u8, minute: u8) -> u64 {
    let days = door_http::days_from_civil(
        i64::from(year),
        u32::from(month.max(1)),
        u32::from(day.max(1)),
    );
    let unix = days * 86_400 + i64::from(hour) * 3600 + i64::from(minute) * 60;
    ((unix + 11_644_473_600).max(0) as u64) * 10_000_000
}
fn find_first(handle: u64, pattern: u64, data: u64, _flags: u32, context: u64, kind: Text) -> u64 {
    let result = (|| {
        let pattern = text(pattern, u32::MAX, kind)?;
        let session = ftp_session(handle)?;
        let listing = session.with(|s| -> Result<Vec<u8>, u32> {
            if s.busy {
                return Err(code::TRANSFER_IN_PROGRESS);
            }
            let mut stream = s.transfer(true, "LIST")?;
            let mut bytes = Vec::new();
            loop {
                let part = stream
                    .read(Deadline::after(TIMEOUT), &|| false)
                    .map_err(|_| error::CONNECTION_ABORTED)?;
                if part.is_empty() || bytes.len() > 4 * 1024 * 1024 {
                    break;
                }
                bytes.extend_from_slice(&part);
            }
            drop(stream);
            s.reply()?;
            Ok(bytes)
        })?;
        let wanted = if pattern.is_empty() || pattern == "*.*" {
            String::from("*")
        } else {
            pattern.rsplit('/').next().unwrap_or(&pattern).into()
        };
        let entries: Vec<Listed> = String::from_utf8_lossy(&listing)
            .lines()
            .filter_map(listing_line)
            .filter(|e| door_pac::shell_match(&e.name, &wanted))
            .collect();
        let first = entries.first().ok_or(code::NO_MORE_FILES)?;
        if data != 0 {
            write(data, &find_data(first, kind))?;
        }
        let find = FtpFind {
            session: session.clone(),
            entries,
            at: 1,
        };
        session.with(|s| s.busy = true);
        Ok(add_handle(
            handle,
            Object::FtpFind(Arc::new(Lock::new(find))),
            context,
        ))
    })();
    match result {
        Ok(h) => {
            unsafe { super::RtlSetLastWin32Error(0) };
            h
        }
        Err(e) => fail(e, 0),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn FtpFindFirstFileW(
    handle: usize,
    pattern: usize,
    data: usize,
    flags: u32,
    context: usize,
) -> usize {
    let handle = door_windows_abi::widen(handle);
    let pattern = door_windows_abi::widen(pattern);
    let data = door_windows_abi::widen(data);
    let context = door_windows_abi::widen(context);
    door_windows_abi::narrow((move || -> u64 {
        find_first(handle, pattern, data, flags, context, Text::Wide)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn FtpFindFirstFileA(
    handle: usize,
    pattern: usize,
    data: usize,
    flags: u32,
    context: usize,
) -> usize {
    let handle = door_windows_abi::widen(handle);
    let pattern = door_windows_abi::widen(pattern);
    let data = door_windows_abi::widen(data);
    let context = door_windows_abi::widen(context);
    door_windows_abi::narrow((move || -> u64 {
        find_first(handle, pattern, data, flags, context, Text::Ansi)
    })())
}
fn find_next(handle: u64, data: u64, kind: Text) -> i32 {
    let result = (|| {
        let find = slot(handle, |s| match &s.object {
            Object::FtpFind(f) => Ok(f.clone()),
            _ => Err(error::INCORRECT_HANDLE_TYPE),
        })
        .unwrap_or(Err(error::INVALID_HANDLE))?;
        find.with(|f| {
            let entry = f.entries.get(f.at).ok_or(code::NO_MORE_FILES)?;
            if data != 0 {
                write(data, &find_data(entry, kind))?;
            }
            f.at += 1;
            Ok(())
        })
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetFindNextFileW(handle: usize, data: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let data = door_windows_abi::widen(data);
    find_next(handle, data, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetFindNextFileA(handle: usize, data: usize) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let data = door_windows_abi::widen(data);
    find_next(handle, data, Text::Ansi)
}
/// Closing a find handle frees its connection for the next transfer.
pub(super) fn close_find(f: &Arc<Lock<FtpFind>>) {
    let session = f.with(|f| f.session.clone());
    session.with(|s| s.busy = false);
}

// ---- whole files -------------------------------------------------------------------------
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
}
const INVALID_HANDLE_VALUE: u64 = u64::MAX;
fn local_wide(path: &str) -> Vec<u16> {
    path.encode_utf16().chain([0]).collect()
}
/// FtpGetFile: the remote file into a new local one.
#[allow(clippy::too_many_arguments)]
fn get_file(
    handle: u64,
    remote: u64,
    local: u64,
    fail_if_exists: i32,
    flags: u32,
    kind: Text,
) -> i32 {
    let result = (|| {
        let (remote, local) = (text(remote, u32::MAX, kind)?, text(local, u32::MAX, kind)?);
        let session = ftp_session(handle)?;
        let path = local_wide(&local);
        // CREATE_NEW (1) when it must not exist, else CREATE_ALWAYS (2).
        let creation = if fail_if_exists != 0 { 1 } else { 2 };
        let out = unsafe {
            CreateFileW(
                door_windows_abi::narrow(path.as_ptr() as u64),
                GENERIC_WRITE,
                0,
                0,
                creation,
                0,
                0,
            )
        };
        if out == door_windows_abi::narrow(INVALID_HANDLE_VALUE) {
            return Err(code::FILE_EXISTS);
        }
        let copied = session.with(|s| -> Result<(), u32> {
            let (mut data, _) = open(s, &remote, false, flags & 0x3 == TRANSFER_ASCII)?;
            loop {
                let part = data
                    .read(Deadline::after(TIMEOUT), &|| false)
                    .map_err(|_| error::CONNECTION_ABORTED)?;
                if part.is_empty() {
                    break;
                }
                let mut written = 0u32;
                if unsafe {
                    WriteFile(
                        out,
                        door_windows_abi::narrow(part.as_ptr() as u64),
                        part.len() as u32,
                        door_windows_abi::narrow(&mut written as *mut u32 as u64),
                        0,
                    )
                } == 0
                {
                    return Err(error::INTERNAL_ERROR);
                }
            }
            drop(data);
            s.reply().map(|_| ())
        });
        unsafe { CloseHandle(out) };
        copied
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn FtpGetFileW(
    handle: usize,
    remote: usize,
    local: usize,
    fail_if_exists: i32,
    _attributes: u32,
    flags: u32,
    _context: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let remote = door_windows_abi::widen(remote);
    let local = door_windows_abi::widen(local);
    get_file(handle, remote, local, fail_if_exists, flags, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn FtpGetFileA(
    handle: usize,
    remote: usize,
    local: usize,
    fail_if_exists: i32,
    _attributes: u32,
    flags: u32,
    _context: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let remote = door_windows_abi::widen(remote);
    let local = door_windows_abi::widen(local);
    get_file(handle, remote, local, fail_if_exists, flags, Text::Ansi)
}
/// FtpPutFile: a local file to the server.
fn put_file(handle: u64, local: u64, remote: u64, flags: u32, kind: Text) -> i32 {
    let result = (|| {
        let (local, remote) = (text(local, u32::MAX, kind)?, text(remote, u32::MAX, kind)?);
        let session = ftp_session(handle)?;
        let path = local_wide(&local);
        let input = unsafe {
            CreateFileW(
                door_windows_abi::narrow(path.as_ptr() as u64),
                GENERIC_READ,
                1,
                0,
                3,
                0,
                0,
            )
        };
        if input == door_windows_abi::narrow(INVALID_HANDLE_VALUE) {
            return Err(error::FILE_NOT_FOUND);
        }
        let sent = session.with(|s| -> Result<(), u32> {
            let (mut data, _) = open(s, &remote, true, flags & 0x3 == TRANSFER_ASCII)?;
            let mut chunk = vec![0u8; 64 * 1024];
            loop {
                let mut got = 0u32;
                if unsafe {
                    ReadFile(
                        input,
                        door_windows_abi::narrow(chunk.as_mut_ptr() as u64),
                        chunk.len() as u32,
                        door_windows_abi::narrow(&mut got as *mut u32 as u64),
                        0,
                    )
                } == 0
                    || got == 0
                {
                    break;
                }
                data.write_all(&chunk[..got as usize], Deadline::after(TIMEOUT), &|| false)
                    .map_err(|_| error::CONNECTION_ABORTED)?;
            }
            data.shutdown();
            drop(data);
            s.reply().map(|_| ())
        });
        unsafe { CloseHandle(input) };
        sent
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn FtpPutFileW(
    handle: usize,
    local: usize,
    remote: usize,
    flags: u32,
    _context: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let local = door_windows_abi::widen(local);
    let remote = door_windows_abi::widen(remote);
    put_file(handle, local, remote, flags, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn FtpPutFileA(
    handle: usize,
    local: usize,
    remote: usize,
    flags: u32,
    _context: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let local = door_windows_abi::widen(local);
    let remote = door_windows_abi::widen(remote);
    put_file(handle, local, remote, flags, Text::Ansi)
}
/// FtpCommand: a raw command on the control connection (no data
/// connection is opened for it).
fn ftp_command(handle: u64, command: u64, out: u64, kind: Text) -> i32 {
    let result = (|| {
        let line = text(command, u32::MAX, kind)?;
        if line.is_empty() || line.contains(['\r', '\n']) {
            return Err(error::INVALID_PARAMETER);
        }
        let s = ftp_session(handle)?;
        let r = s.with(|s| s.command(&line))?;
        // *phFtpCommand: no data connection (a pointer-sized NULL).
        if out != 0 {
            super::write_pointer(out, 0)?;
        }
        if r.class() >= 4 {
            return Err(code::EXTENDED_ERROR);
        }
        Ok(())
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn FtpCommandW(
    handle: usize,
    _expect: i32,
    _flags: u32,
    command: usize,
    _context: usize,
    out: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let command = door_windows_abi::widen(command);
    let out = door_windows_abi::widen(out);
    ftp_command(handle, command, out, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn FtpCommandA(
    handle: usize,
    _expect: i32,
    _flags: u32,
    command: usize,
    _context: usize,
    out: usize,
) -> i32 {
    let handle = door_windows_abi::widen(handle);
    let command = door_windows_abi::widen(command);
    let out = door_windows_abi::widen(out);
    ftp_command(handle, command, out, Text::Ansi)
}
/// InternetGetLastResponseInfo: the last FTP reply (code 0) and its text.
fn last_response(error_out: u64, buffer: u64, length: u64, kind: Text) -> i32 {
    let result = (|| {
        let text = LAST.with(|l| l.clone());
        if error_out != 0 {
            write_u32(error_out, 0)?;
        }
        let bytes = encode(&text, kind);
        let units = bytes.len() / super::unit(kind) as usize;
        let capacity = read_u32(length)? as usize;
        if units > capacity || buffer == 0 {
            write_u32(length, units as u32)?;
            return Err(error::INSUFFICIENT_BUFFER);
        }
        write(buffer, &bytes)?;
        write_u32(length, (units - 1) as u32)
    })();
    done(result)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetGetLastResponseInfoW(
    error_out: usize,
    buffer: usize,
    length: usize,
) -> i32 {
    let error_out = door_windows_abi::widen(error_out);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    last_response(error_out, buffer, length, Text::Wide)
}
#[unsafe(no_mangle)]
pub extern "system" fn InternetGetLastResponseInfoA(
    error_out: usize,
    buffer: usize,
    length: usize,
) -> i32 {
    let error_out = door_windows_abi::widen(error_out);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    last_response(error_out, buffer, length, Text::Ansi)
}

/// InternetOpenUrl for ftp://: a session for the URL and its file open
/// for reading, both behind the one handle answered.
pub(super) fn open_url(parent: u64, url: &str, flags: u32, context: u64) -> Result<u64, u32> {
    session(parent)?;
    let rest = url
        .get(6..)
        .filter(|_| url[..6].eq_ignore_ascii_case("ftp://"))
        .ok_or(error::INVALID_URL)?;
    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    let (credentials, host_port) = match authority.rsplit_once('@') {
        Some((c, h)) => (Some(c), h),
        None => (None, authority),
    };
    let (user, password) = match credentials {
        Some(c) => match c.split_once(':') {
            Some((u, p)) => (String::from(u), String::from(p)),
            None => (String::from(c), String::new()),
        },
        None => (String::new(), String::new()),
    };
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().map_err(|_| error::INVALID_URL)?),
        None => (host_port, 21),
    };
    if host.is_empty() {
        return Err(error::INVALID_URL);
    }
    let mut s = open_session(host, port, &user, &password)?;
    let ascii = flags & 0x3 == TRANSFER_ASCII;
    let (data, size) = open(&mut s, path, false, ascii)?;
    s.busy = true;
    let file = FtpFile {
        session: Arc::new(Lock::new(s)),
        data: Some(data),
        writing: false,
        size,
        ended: false,
        buffer: Vec::new(),
    };
    Ok(add_handle(
        parent,
        Object::FtpFile(Arc::new(Lock::new(file))),
        context,
    ))
}
