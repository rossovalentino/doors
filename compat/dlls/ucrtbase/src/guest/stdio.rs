//! The stdio set (api-ms-win-crt-stdio): low-level file descriptors over
//! Windows handles (`_open`, `_read`, `_write`, `_setmode`...), FILE
//! streams with their buffering and text-mode translation, and the
//! `__stdio_common_v*` entry points every printf and scanf of a /MD
//! program goes through.
use super::{
    AcquireSRWLockExclusive, CURRENT_PROCESS, CloseHandle, CreateFileW, DuplicateHandle,
    FlushFileBuffers, GetConsoleMode, GetFileType, GetStdHandle, ReadFile, ReleaseSRWLockExclusive,
    SetFilePointerEx, WriteConsoleW, WriteFile, convert, errno, heap, invalid_parameter,
    map_os_error, set_errno,
};
use core::sync::atomic::{AtomicBool, Ordering};
use door_crt::{format, scan};

const INVALID: usize = usize::MAX;
const EOF: i32 = -1;
const WEOF: u16 = 0xFFFF;

// _O_* flags.
const O_RDONLY: i32 = 0;
const O_WRONLY: i32 = 1;
const O_RDWR: i32 = 2;
const O_APPEND: i32 = 0x8;
const O_CREAT: i32 = 0x100;
const O_TRUNC: i32 = 0x200;
const O_EXCL: i32 = 0x400;
const O_TEXT: i32 = 0x4000;
const O_BINARY: i32 = 0x8000;
const O_WTEXT: i32 = 0x10000;
const O_U16TEXT: i32 = 0x20000;
const O_U8TEXT: i32 = 0x40000;
const O_NOINHERIT: i32 = 0x80;

/// One file descriptor.
#[derive(Clone, Copy)]
struct Fd {
    handle: usize,
    open: bool,
    /// O_TEXT, O_BINARY, O_U16TEXT, O_U8TEXT, O_WTEXT.
    mode: i32,
    append: bool,
    /// A console (writes go through WriteConsoleW).
    console: bool,
    /// The low-level read hit end of file.
    eof: bool,
}
const MAX_FDS: usize = 2048;
struct Fds {
    lock: AtomicBool,
    table: core::cell::UnsafeCell<[Fd; MAX_FDS]>,
}
unsafe impl Sync for Fds {}
static FDS: Fds = Fds {
    lock: AtomicBool::new(false),
    table: core::cell::UnsafeCell::new(
        [Fd {
            handle: INVALID,
            open: false,
            mode: O_TEXT,
            append: false,
            console: false,
            eof: false,
        }; MAX_FDS],
    ),
};
struct FdGuard;
impl FdGuard {
    fn take() -> Self {
        while FDS
            .lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        Self
    }
    #[allow(clippy::mut_from_ref)]
    fn table(&self) -> &mut [Fd; MAX_FDS] {
        // SAFETY: the guard admits one holder.
        unsafe { &mut *FDS.table.get() }
    }
}
impl Drop for FdGuard {
    fn drop(&mut self) {
        FDS.lock.store(false, Ordering::Release);
    }
}
fn is_console(handle: usize) -> bool {
    let mut mode = 0u32;
    unsafe { GetConsoleMode(handle, &mut mode as *mut u32 as usize) != 0 }
}
/// The descriptor `fd` (0 to 2 are bound to the standard handles on first
/// use).
fn fd(fd: i32) -> Option<Fd> {
    let index = usize::try_from(fd).ok().filter(|i| *i < MAX_FDS)?;
    let g = FdGuard::take();
    let t = g.table();
    if index < 3 && !t[index].open {
        let handle = unsafe { GetStdHandle([-10i32, -11, -12][index] as u32) };
        if handle != 0 && handle != INVALID {
            t[index] = Fd {
                handle,
                open: true,
                mode: if super::runtime::default_binary() {
                    O_BINARY
                } else {
                    O_TEXT
                },
                append: false,
                console: is_console(handle),
                eof: false,
            };
        }
    }
    t[index].open.then_some(t[index])
}
fn set_eof(fd: i32, eof: bool) {
    let g = FdGuard::take();
    if let Some(entry) = g.table().get_mut(fd as usize) {
        entry.eof = eof;
    }
}
fn allocate_fd(handle: usize, mode: i32, append: bool) -> i32 {
    let g = FdGuard::take();
    let t = g.table();
    for (i, entry) in t.iter_mut().enumerate().skip(3) {
        if !entry.open {
            *entry = Fd {
                handle,
                open: true,
                mode,
                append,
                console: is_console(handle),
                eof: false,
            };
            return i as i32;
        }
    }
    set_errno(errno::EMFILE);
    -1
}

#[unsafe(no_mangle)]
pub extern "C" fn _get_osfhandle(descriptor: i32) -> usize {
    match fd(descriptor) {
        Some(f) => f.handle,
        None => {
            set_errno(errno::EBADF);
            INVALID
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn _open_osfhandle(handle: usize, flags: i32) -> i32 {
    let mode = if flags & O_BINARY != 0 {
        O_BINARY
    } else {
        O_TEXT
    };
    allocate_fd(handle, mode, flags & O_APPEND != 0)
}
/// `_wsopen_s`: open `path` with `_O_*` flags.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wsopen_s(
    out: *mut i32,
    path: *const u16,
    flags: i32,
    share: i32,
    _permission: i32,
) -> i32 {
    if out.is_null() || path.is_null() {
        invalid_parameter();
        return errno::EINVAL;
    }
    unsafe { *out = -1 };
    let access = match flags & 3 {
        O_WRONLY => 0x4000_0000,
        O_RDWR => 0xC000_0000,
        _ => 0x8000_0000,
    } | if flags & O_APPEND != 0 { 0x4 } else { 0 };
    let disposition = match (
        flags & O_CREAT != 0,
        flags & O_EXCL != 0,
        flags & O_TRUNC != 0,
    ) {
        (true, true, _) => 1,      // CREATE_NEW
        (true, false, true) => 2,  // CREATE_ALWAYS
        (true, false, false) => 4, // OPEN_ALWAYS
        (false, _, true) => 5,     // TRUNCATE_EXISTING
        _ => 3,                    // OPEN_EXISTING
    };
    let share = match share {
        0x10 => 0,  // _SH_DENYRW
        0x20 => 1,  // _SH_DENYWR: others may read
        0x30 => 2,  // _SH_DENYRD: others may write
        _ => 3 | 4, // _SH_DENYNO
    };
    let handle = unsafe { CreateFileW(path as usize, access, share, 0, disposition, 0x80, 0) };
    if handle == INVALID {
        map_os_error(unsafe { super::GetLastError() });
        return thread_errno();
    }
    let mode = if flags & (O_U16TEXT | O_U8TEXT | O_WTEXT) != 0 {
        flags & (O_U16TEXT | O_U8TEXT | O_WTEXT)
    } else if flags & O_BINARY != 0 || (flags & O_TEXT == 0 && super::runtime::default_binary()) {
        O_BINARY
    } else {
        O_TEXT
    };
    let _ = O_NOINHERIT;
    let descriptor = allocate_fd(handle, mode, flags & O_APPEND != 0);
    if descriptor < 0 {
        unsafe { CloseHandle(handle) };
        return errno::EMFILE;
    }
    unsafe { *out = descriptor };
    0
}
fn thread_errno() -> i32 {
    super::thread().errno
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wopen(path: *const u16, flags: i32, permission: i32) -> i32 {
    let mut fd = -1;
    unsafe { _wsopen_s(&mut fd, path, flags, 0x40, permission) };
    fd
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _sopen_s(
    out: *mut i32,
    path: *const u8,
    flags: i32,
    share: i32,
    permission: i32,
) -> i32 {
    let mut wide = [0u16; 1024];
    if path.is_null()
        || unsafe { convert::mbstowcs(wide.as_mut_ptr(), path, wide.len() - 1) } == usize::MAX
    {
        invalid_parameter();
        return errno::EINVAL;
    }
    unsafe { _wsopen_s(out, wide.as_ptr(), flags, share, permission) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _open(path: *const u8, flags: i32, permission: i32) -> i32 {
    let mut fd = -1;
    unsafe { _sopen_s(&mut fd, path, flags, 0x40, permission) };
    fd
}
#[unsafe(no_mangle)]
pub extern "C" fn _close(descriptor: i32) -> i32 {
    let Some(f) = fd(descriptor) else {
        set_errno(errno::EBADF);
        return -1;
    };
    {
        let g = FdGuard::take();
        g.table()[descriptor as usize].open = false;
    }
    if unsafe { CloseHandle(f.handle) } == 0 && descriptor > 2 {
        map_os_error(unsafe { super::GetLastError() });
        return -1;
    }
    0
}
/// Write `bytes` to the descriptor's handle (console text as UTF-16).
fn os_write(f: &Fd, data: &[u8]) -> Result<usize, ()> {
    if f.console {
        // UTF-8 to UTF-16 for the console.
        let mut units = [0u16; 512];
        let mut n = 0;
        let mut i = 0;
        while i < data.len() {
            let (c, used) = unsafe { convert::decode(data.as_ptr().add(i), data.len() - i) }
                .unwrap_or((0xFFFD, 1));
            let ch = char::from_u32(c).unwrap_or('\u{FFFD}');
            let mut b = [0u16; 2];
            for u in ch.encode_utf16(&mut b) {
                units[n] = *u;
                n += 1;
            }
            i += used;
            if n >= units.len() - 2 || i >= data.len() {
                let mut written = 0u32;
                if unsafe {
                    WriteConsoleW(
                        f.handle,
                        units.as_ptr() as usize,
                        n as u32,
                        &mut written as *mut u32 as usize,
                        0,
                    )
                } == 0
                {
                    return Err(());
                }
                n = 0;
            }
        }
        return Ok(data.len());
    }
    if f.append {
        unsafe { SetFilePointerEx(f.handle, 0, 0, 2) };
    }
    let mut done = 0;
    while done < data.len() {
        let mut written = 0u32;
        let chunk = (data.len() - done).min(1 << 30) as u32;
        if unsafe {
            WriteFile(
                f.handle,
                data.as_ptr().add(done) as usize,
                chunk,
                &mut written as *mut u32 as usize,
                0,
            )
        } == 0
        {
            map_os_error(unsafe { super::GetLastError() });
            return Err(());
        }
        done += written as usize;
        if written == 0 {
            break;
        }
    }
    Ok(done)
}
/// `_write`: text mode turns "\n" into "\r\n".
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _write(descriptor: i32, data: *const u8, bytes: u32) -> i32 {
    let Some(f) = fd(descriptor) else {
        set_errno(errno::EBADF);
        return -1;
    };
    if bytes == 0 {
        return 0;
    }
    if data.is_null() {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return -1;
    }
    let input = unsafe { core::slice::from_raw_parts(data, bytes as usize) };
    if f.mode == O_BINARY {
        return os_write(&f, input).map_or(-1, |n| n as i32);
    }
    let mut out = [0u8; 1024];
    let mut n = 0;
    for &b in input {
        if b == b'\n' {
            out[n] = b'\r';
            n += 1;
        }
        out[n] = b;
        n += 1;
        if n >= out.len() - 2 {
            if os_write(&f, &out[..n]).is_err() {
                return -1;
            }
            n = 0;
        }
    }
    if n > 0 && os_write(&f, &out[..n]).is_err() {
        return -1;
    }
    bytes as i32
}
/// `_read`: text mode turns "\r\n" into "\n"; 0 at end of file.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _read(descriptor: i32, data: *mut u8, bytes: u32) -> i32 {
    let Some(f) = fd(descriptor) else {
        set_errno(errno::EBADF);
        return -1;
    };
    if bytes == 0 {
        return 0;
    }
    let mut read = 0u32;
    if unsafe {
        ReadFile(
            f.handle,
            data as usize,
            bytes,
            &mut read as *mut u32 as usize,
            0,
        )
    } == 0
    {
        let error = unsafe { super::GetLastError() };
        // A closed pipe is end of file.
        if error == 109 {
            set_eof(descriptor, true);
            return 0;
        }
        map_os_error(error);
        return -1;
    }
    if read == 0 {
        set_eof(descriptor, true);
        return 0;
    }
    if f.mode == O_BINARY {
        return read as i32;
    }
    // Collapse CR LF in place.
    let buffer = unsafe { core::slice::from_raw_parts_mut(data, read as usize) };
    let mut out = 0;
    let mut i = 0;
    while i < buffer.len() {
        if buffer[i] == b'\r' && i + 1 < buffer.len() && buffer[i + 1] == b'\n' {
            i += 1;
            continue;
        }
        // Ctrl+Z ends text input from a console.
        if buffer[i] == 0x1A && f.console {
            set_eof(descriptor, true);
            break;
        }
        buffer[out] = buffer[i];
        out += 1;
        i += 1;
    }
    out as i32
}
#[unsafe(no_mangle)]
pub extern "C" fn _lseeki64(descriptor: i32, offset: i64, origin: i32) -> i64 {
    let Some(f) = fd(descriptor) else {
        set_errno(errno::EBADF);
        return -1;
    };
    let mut position = 0i64;
    if unsafe {
        SetFilePointerEx(
            f.handle,
            offset,
            &mut position as *mut i64 as usize,
            origin as u32,
        )
    } == 0
    {
        map_os_error(unsafe { super::GetLastError() });
        return -1;
    }
    set_eof(descriptor, false);
    position
}
#[unsafe(no_mangle)]
pub extern "C" fn _lseek(descriptor: i32, offset: i32, origin: i32) -> i32 {
    let p = _lseeki64(descriptor, i64::from(offset), origin);
    if p > i64::from(i32::MAX) {
        set_errno(errno::EINVAL);
        return -1;
    }
    p as i32
}
#[unsafe(no_mangle)]
pub extern "C" fn _telli64(descriptor: i32) -> i64 {
    _lseeki64(descriptor, 0, 1)
}
#[unsafe(no_mangle)]
pub extern "C" fn _filelengthi64(descriptor: i32) -> i64 {
    let here = _lseeki64(descriptor, 0, 1);
    if here < 0 {
        return -1;
    }
    let end = _lseeki64(descriptor, 0, 2);
    _lseeki64(descriptor, here, 0);
    end
}
#[unsafe(no_mangle)]
pub extern "C" fn _filelength(descriptor: i32) -> i32 {
    _filelengthi64(descriptor) as i32
}
#[unsafe(no_mangle)]
pub extern "C" fn _eof(descriptor: i32) -> i32 {
    let here = _lseeki64(descriptor, 0, 1);
    if here < 0 {
        return -1;
    }
    i32::from(here >= _filelengthi64(descriptor))
}
#[unsafe(no_mangle)]
pub extern "C" fn _commit(descriptor: i32) -> i32 {
    let Some(f) = fd(descriptor) else {
        set_errno(errno::EBADF);
        return -1;
    };
    if unsafe { FlushFileBuffers(f.handle) } == 0 {
        map_os_error(unsafe { super::GetLastError() });
        return -1;
    }
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn _isatty(descriptor: i32) -> i32 {
    match fd(descriptor) {
        // FILE_TYPE_CHAR.
        Some(f) => i32::from(unsafe { GetFileType(f.handle) } == 2),
        None => {
            set_errno(errno::EBADF);
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn _setmode(descriptor: i32, mode: i32) -> i32 {
    let valid = [O_TEXT, O_BINARY, O_WTEXT, O_U16TEXT, O_U8TEXT];
    if !valid.contains(&mode) {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return -1;
    }
    let Some(previous) = fd(descriptor) else {
        set_errno(errno::EBADF);
        return -1;
    };
    let g = FdGuard::take();
    g.table()[descriptor as usize].mode = mode;
    previous.mode
}
#[unsafe(no_mangle)]
pub extern "C" fn _dup(descriptor: i32) -> i32 {
    let Some(f) = fd(descriptor) else {
        set_errno(errno::EBADF);
        return -1;
    };
    let mut copy = 0usize;
    if unsafe {
        DuplicateHandle(
            CURRENT_PROCESS,
            f.handle,
            CURRENT_PROCESS,
            &mut copy as *mut usize as usize,
            0,
            0,
            2,
        )
    } == 0
    {
        map_os_error(unsafe { super::GetLastError() });
        return -1;
    }
    let new = allocate_fd(copy, f.mode, f.append);
    if new < 0 {
        unsafe { CloseHandle(copy) };
    }
    new
}
#[unsafe(no_mangle)]
pub extern "C" fn _dup2(from: i32, to: i32) -> i32 {
    let Some(f) = fd(from) else {
        set_errno(errno::EBADF);
        return -1;
    };
    let Some(index) = usize::try_from(to).ok().filter(|i| *i < MAX_FDS) else {
        set_errno(errno::EBADF);
        return -1;
    };
    let mut copy = 0usize;
    if unsafe {
        DuplicateHandle(
            CURRENT_PROCESS,
            f.handle,
            CURRENT_PROCESS,
            &mut copy as *mut usize as usize,
            0,
            0,
            2,
        )
    } == 0
    {
        map_os_error(unsafe { super::GetLastError() });
        return -1;
    }
    let old = {
        let g = FdGuard::take();
        let t = g.table();
        let old = t[index];
        t[index] = Fd {
            handle: copy,
            open: true,
            mode: f.mode,
            append: f.append,
            console: f.console,
            eof: false,
        };
        old
    };
    if old.open {
        unsafe { CloseHandle(old.handle) };
    }
    0
}

// ---------------------------------------------------------------------------
// FILE streams.

const READ: u32 = 0x1;
const WRITE: u32 = 0x2;
const UPDATE: u32 = 0x4;
const AT_EOF: u32 = 0x10;
const ERROR: u32 = 0x20;
const OPEN: u32 = 0x40;
/// The last operation (a switch between reading and writing flushes).
const READING: u32 = 0x100;
const WRITING: u32 = 0x200;
const UNBUFFERED: u32 = 0x400;
const LINE_BUFFERED: u32 = 0x800;
const BUFFER_BYTES: usize = 4096;

/// A stream (`FILE` is opaque in the UCRT's headers).
#[repr(C)]
pub struct File {
    lock: usize,
    flags: u32,
    fd: i32,
    buffer: *mut u8,
    capacity: usize,
    /// Bytes buffered for writing, or read and not yet consumed.
    used: usize,
    position: usize,
    /// ungetc's character (-1: none).
    ungot: i32,
}
impl File {
    const fn closed() -> Self {
        Self {
            lock: 0,
            flags: 0,
            fd: -1,
            buffer: core::ptr::null_mut(),
            capacity: 0,
            used: 0,
            position: 0,
            ungot: -1,
        }
    }
}
const MAX_STREAMS: usize = 512;
static mut STREAMS: [File; MAX_STREAMS] = [const { File::closed() }; MAX_STREAMS];
static STREAMS_LOCK: AtomicBool = AtomicBool::new(false);
fn streams() -> *mut File {
    core::ptr::addr_of_mut!(STREAMS) as *mut File
}
/// stdin, stdout and stderr, bound on first use.
#[unsafe(no_mangle)]
pub extern "C" fn __acrt_iob_func(index: u32) -> *mut File {
    let index = index.min(2) as usize;
    let file = unsafe { streams().add(index) };
    let f = unsafe { &mut *file };
    if f.flags & OPEN == 0 {
        f.fd = index as i32;
        f.flags = OPEN
            | if index == 0 { READ } else { WRITE }
            | if index == 2 {
                UNBUFFERED
            } else {
                LINE_BUFFERED
            };
        f.ungot = -1;
    }
    file
}
struct Locked(*mut File);
impl Locked {
    fn new(file: *mut File) -> Self {
        unsafe { AcquireSRWLockExclusive(core::ptr::addr_of_mut!((*file).lock) as usize) };
        Self(file)
    }
    #[allow(clippy::mut_from_ref)]
    fn file(&self) -> &mut File {
        unsafe { &mut *self.0 }
    }
}
impl Drop for Locked {
    fn drop(&mut self) {
        unsafe { ReleaseSRWLockExclusive(core::ptr::addr_of_mut!((*self.0).lock) as usize) };
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _lock_file(file: *mut File) {
    unsafe { AcquireSRWLockExclusive(core::ptr::addr_of_mut!((*file).lock) as usize) };
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _unlock_file(file: *mut File) {
    unsafe { ReleaseSRWLockExclusive(core::ptr::addr_of_mut!((*file).lock) as usize) };
}
fn ensure_buffer(f: &mut File) {
    if f.buffer.is_null() && f.flags & UNBUFFERED == 0 {
        let b = heap::malloc(BUFFER_BYTES);
        if !b.is_null() {
            f.buffer = b;
            f.capacity = BUFFER_BYTES;
        } else {
            f.flags |= UNBUFFERED;
        }
    }
}
/// Write out what is buffered for writing.
fn flush_locked(f: &mut File) -> i32 {
    if f.flags & WRITING != 0 && f.used > 0 {
        let data = unsafe { core::slice::from_raw_parts(f.buffer, f.used) };
        f.used = 0;
        if unsafe { _write(f.fd, data.as_ptr(), data.len() as u32) } < 0 {
            f.flags |= ERROR;
            return EOF;
        }
    }
    if f.flags & READING != 0 && f.used > f.position {
        // Unread buffered input: move the descriptor back to the logical
        // position (binary streams; text streams may be off by CRs).
        let unread = (f.used - f.position) as i64;
        _lseeki64(f.fd, -unread, 1);
    }
    if f.flags & READING != 0 {
        f.used = 0;
        f.position = 0;
    }
    f.flags &= !(READING | WRITING);
    0
}
fn put_bytes(f: &mut File, data: &[u8]) -> usize {
    if f.flags & (WRITE | UPDATE) == 0 {
        f.flags |= ERROR;
        set_errno(errno::EBADF);
        return 0;
    }
    if f.flags & READING != 0 {
        flush_locked(f);
    }
    f.flags |= WRITING;
    ensure_buffer(f);
    if f.buffer.is_null() || f.flags & UNBUFFERED != 0 {
        return if unsafe { _write(f.fd, data.as_ptr(), data.len() as u32) } < 0 {
            f.flags |= ERROR;
            0
        } else {
            data.len()
        };
    }
    let mut done = 0;
    while done < data.len() {
        let n = (data.len() - done).min(f.capacity - f.used);
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr().add(done), f.buffer.add(f.used), n) };
        f.used += n;
        done += n;
        if f.used == f.capacity && flush_locked(f) == EOF {
            return done - n;
        }
        if f.used == 0 {
            f.flags |= WRITING;
        }
    }
    if f.flags & LINE_BUFFERED != 0 && data.contains(&b'\n') {
        flush_locked(f);
    }
    done
}
/// Read one byte (EOF at the end or on error).
fn get_byte(f: &mut File) -> i32 {
    if f.ungot >= 0 {
        let c = f.ungot;
        f.ungot = -1;
        return c;
    }
    if f.flags & (READ | UPDATE) == 0 {
        f.flags |= ERROR;
        return EOF;
    }
    if f.flags & WRITING != 0 {
        flush_locked(f);
    }
    f.flags |= READING;
    ensure_buffer(f);
    if f.position >= f.used {
        f.position = 0;
        f.used = 0;
        let mut single = 0u8;
        let (target, room) = if f.buffer.is_null() {
            (&mut single as *mut u8, 1)
        } else {
            (f.buffer, f.capacity)
        };
        let n = unsafe { _read(f.fd, target, room as u32) };
        if n <= 0 {
            f.flags |= if n == 0 { AT_EOF } else { ERROR };
            return EOF;
        }
        if f.buffer.is_null() {
            return i32::from(single);
        }
        f.used = n as usize;
    }
    let c = unsafe { *f.buffer.add(f.position) };
    f.position += 1;
    i32::from(c)
}
/// Flush every open stream (exit, `_flushall`).
pub(crate) fn flush_all() -> i32 {
    let mut count = 0;
    for i in 0..MAX_STREAMS {
        let file = unsafe { streams().add(i) };
        if unsafe { (*file).flags } & OPEN != 0 {
            let l = Locked::new(file);
            flush_locked(l.file());
            count += 1;
        }
    }
    count
}
#[unsafe(no_mangle)]
pub extern "C" fn _flushall() -> i32 {
    flush_all()
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fflush(file: *mut File) -> i32 {
    if file.is_null() {
        flush_all();
        return 0;
    }
    let l = Locked::new(file);
    flush_locked(l.file())
}
/// Parse an fopen mode: (_O_* flags, stream flags).
unsafe fn mode<T: Copy + Into<u32>>(mode: *const T) -> Option<(i32, u32)> {
    if mode.is_null() {
        return None;
    }
    let first: u32 = unsafe { *mode }.into();
    let (mut flags, mut stream) = match first as u8 {
        b'r' => (O_RDONLY, READ),
        b'w' => (O_WRONLY | O_CREAT | O_TRUNC, WRITE),
        b'a' => (O_WRONLY | O_CREAT | O_APPEND, WRITE),
        _ => return None,
    };
    let mut i = 1;
    loop {
        let c: u32 = unsafe { *mode.add(i) }.into();
        match c as u8 {
            0 => break,
            b'+' => {
                flags = (flags & !3) | O_RDWR;
                stream |= UPDATE | READ | WRITE;
            }
            b'b' => flags |= O_BINARY,
            b't' => flags |= O_TEXT,
            b'x' => flags |= O_EXCL,
            b',' => {
                // ",ccs=UTF-8" / "UTF-16LE" / "UNICODE".
                let mut k = i + 1;
                let mut word = [0u8; 16];
                let mut n = 0;
                loop {
                    let c: u32 = unsafe { *mode.add(k) }.into();
                    if c == 0 || n >= word.len() {
                        break;
                    }
                    word[n] = (c as u8).to_ascii_uppercase();
                    n += 1;
                    k += 1;
                }
                let encoding = &word[..n];
                flags |= if encoding.ends_with(b"UTF-8") {
                    O_U8TEXT
                } else if encoding.ends_with(b"UTF-16LE") || encoding.ends_with(b"UNICODE") {
                    O_U16TEXT
                } else {
                    return None;
                };
                break;
            }
            b'c' | b'n' | b'S' | b'R' | b'T' | b'D' | b'N' => {}
            _ => return None,
        }
        i += 1;
    }
    Some((flags, stream))
}
fn new_stream(fd: i32, stream: u32) -> *mut File {
    while STREAMS_LOCK
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    let mut found = core::ptr::null_mut();
    for i in 3..MAX_STREAMS {
        let file = unsafe { streams().add(i) };
        let f = unsafe { &mut *file };
        if f.flags & OPEN == 0 {
            *f = File::closed();
            f.fd = fd;
            f.flags = OPEN | stream;
            found = file;
            break;
        }
    }
    STREAMS_LOCK.store(false, Ordering::Release);
    if found.is_null() {
        set_errno(errno::EMFILE);
    }
    found
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wfsopen(
    path: *const u16,
    mode_text: *const u16,
    share: i32,
) -> *mut File {
    let Some((flags, stream)) = (unsafe { mode(mode_text) }) else {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return core::ptr::null_mut();
    };
    let mut descriptor = -1;
    if unsafe { _wsopen_s(&mut descriptor, path, flags, share, 0x180) } != 0 {
        return core::ptr::null_mut();
    }
    let file = new_stream(descriptor, stream);
    if file.is_null() {
        _close(descriptor);
    }
    file
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wfopen(path: *const u16, mode_text: *const u16) -> *mut File {
    unsafe { _wfsopen(path, mode_text, 0x40) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wfopen_s(
    out: *mut *mut File,
    path: *const u16,
    mode_text: *const u16,
) -> i32 {
    if out.is_null() {
        invalid_parameter();
        return errno::EINVAL;
    }
    let f = unsafe { _wfsopen(path, mode_text, 0x10) };
    unsafe { *out = f };
    if f.is_null() { thread_errno() } else { 0 }
}
/// A heap UTF-16 copy of a UTF-8 string.
unsafe fn widen(s: *const u8) -> Option<*mut u16> {
    if s.is_null() {
        return None;
    }
    let units = unsafe { convert::mbstowcs(core::ptr::null_mut(), s, 0) };
    if units == usize::MAX {
        return None;
    }
    let p = heap::malloc((units + 1) * 2) as *mut u16;
    if !p.is_null() {
        unsafe { convert::mbstowcs(p, s, units + 1) };
    }
    (!p.is_null()).then_some(p)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _fsopen(path: *const u8, mode_text: *const u8, share: i32) -> *mut File {
    let (Some(wpath), Some(wmode)) = (unsafe { widen(path) }, unsafe { widen(mode_text) }) else {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return core::ptr::null_mut();
    };
    let f = unsafe { _wfsopen(wpath, wmode, share) };
    unsafe {
        heap::free(wpath as *mut u8);
        heap::free(wmode as *mut u8);
    }
    f
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fopen(path: *const u8, mode_text: *const u8) -> *mut File {
    unsafe { _fsopen(path, mode_text, 0x40) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fopen_s(
    out: *mut *mut File,
    path: *const u8,
    mode_text: *const u8,
) -> i32 {
    if out.is_null() {
        invalid_parameter();
        return errno::EINVAL;
    }
    let f = unsafe { _fsopen(path, mode_text, 0x10) };
    unsafe { *out = f };
    if f.is_null() { thread_errno() } else { 0 }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _fdopen(descriptor: i32, mode_text: *const u8) -> *mut File {
    let Some((_, stream)) = (unsafe { mode(mode_text) }) else {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return core::ptr::null_mut();
    };
    if fd(descriptor).is_none() {
        set_errno(errno::EBADF);
        return core::ptr::null_mut();
    }
    new_stream(descriptor, stream)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wfdopen(descriptor: i32, mode_text: *const u16) -> *mut File {
    let Some((_, stream)) = (unsafe { mode(mode_text) }) else {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return core::ptr::null_mut();
    };
    if fd(descriptor).is_none() {
        set_errno(errno::EBADF);
        return core::ptr::null_mut();
    }
    new_stream(descriptor, stream)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fclose(file: *mut File) -> i32 {
    if file.is_null() {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return EOF;
    }
    {
        let l = Locked::new(file);
        let f = l.file();
        if f.flags & OPEN == 0 {
            return EOF;
        }
        let r = flush_locked(f);
        let descriptor = f.fd;
        if !f.buffer.is_null() {
            unsafe { heap::free(f.buffer) };
        }
        let lock = f.lock;
        *f = File::closed();
        f.lock = lock;
        if _close(descriptor) < 0 { EOF } else { r }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn _fcloseall() -> i32 {
    let mut n = 0;
    for i in 3..MAX_STREAMS {
        let file = unsafe { streams().add(i) };
        if unsafe { (*file).flags } & OPEN != 0 && unsafe { fclose(file) } == 0 {
            n += 1;
        }
    }
    n
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _fileno(file: *mut File) -> i32 {
    if file.is_null() {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return -1;
    }
    unsafe { (*file).fd }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setvbuf(file: *mut File, buffer: *mut u8, kind: i32, size: usize) -> i32 {
    if file.is_null()
        || !matches!(kind, 0 | 0x40 | 4)
        || (kind != 4 && !(2..=i32::MAX as usize).contains(&size))
    {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return -1;
    }
    let l = Locked::new(file);
    let f = l.file();
    flush_locked(f);
    f.flags &= !(UNBUFFERED | LINE_BUFFERED);
    match kind {
        4 => f.flags |= UNBUFFERED,       // _IONBF
        0x40 => f.flags |= LINE_BUFFERED, // _IOLBF (full buffering in the UCRT)
        _ => {}
    }
    if kind != 4 && !buffer.is_null() {
        // The caller's buffer replaces ours (ours is freed).
        if !f.buffer.is_null() {
            unsafe { heap::free(f.buffer) };
        }
        f.buffer = buffer;
        f.capacity = size;
    }
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setbuf(file: *mut File, buffer: *mut u8) {
    unsafe { setvbuf(file, buffer, if buffer.is_null() { 4 } else { 0 }, 512) };
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fwrite(
    data: *const u8,
    size: usize,
    count: usize,
    file: *mut File,
) -> usize {
    let Some(total) = size.checked_mul(count) else {
        return 0;
    };
    if total == 0 {
        return 0;
    }
    let l = Locked::new(file);
    let done = put_bytes(l.file(), unsafe {
        core::slice::from_raw_parts(data, total)
    });
    done / size
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fread(data: *mut u8, size: usize, count: usize, file: *mut File) -> usize {
    let Some(total) = size.checked_mul(count) else {
        return 0;
    };
    if total == 0 {
        return 0;
    }
    let l = Locked::new(file);
    let f = l.file();
    let mut done = 0;
    while done < total {
        // Large binary reads go straight to the descriptor.
        if f.ungot < 0 && f.position >= f.used && total - done >= BUFFER_BYTES {
            if f.flags & WRITING != 0 {
                flush_locked(f);
            }
            f.flags |= READING;
            let n = unsafe { _read(f.fd, data.add(done), (total - done).min(1 << 30) as u32) };
            if n <= 0 {
                f.flags |= if n == 0 { AT_EOF } else { ERROR };
                break;
            }
            done += n as usize;
            continue;
        }
        let c = get_byte(f);
        if c == EOF {
            break;
        }
        unsafe { *data.add(done) = c as u8 };
        done += 1;
    }
    done / size
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fgetc(file: *mut File) -> i32 {
    let l = Locked::new(file);
    get_byte(l.file())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getc(file: *mut File) -> i32 {
    unsafe { fgetc(file) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _fgetc_nolock(file: *mut File) -> i32 {
    get_byte(unsafe { &mut *file })
}
#[unsafe(no_mangle)]
pub extern "C" fn getchar() -> i32 {
    unsafe { fgetc(__acrt_iob_func(0)) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fputc(c: i32, file: *mut File) -> i32 {
    let l = Locked::new(file);
    if put_bytes(l.file(), &[c as u8]) == 1 {
        c & 0xFF
    } else {
        EOF
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn putc(c: i32, file: *mut File) -> i32 {
    unsafe { fputc(c, file) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _fputc_nolock(c: i32, file: *mut File) -> i32 {
    if put_bytes(unsafe { &mut *file }, &[c as u8]) == 1 {
        c & 0xFF
    } else {
        EOF
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn putchar(c: i32) -> i32 {
    unsafe { fputc(c, __acrt_iob_func(1)) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _fputchar(c: i32) -> i32 {
    putchar(c)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fputs(s: *const u8, file: *mut File) -> i32 {
    let n = unsafe { super::length(s) };
    let l = Locked::new(file);
    if put_bytes(l.file(), unsafe { core::slice::from_raw_parts(s, n) }) == n {
        0
    } else {
        EOF
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn puts(s: *const u8) -> i32 {
    let out = __acrt_iob_func(1);
    if unsafe { fputs(s, out) } == EOF {
        return EOF;
    }
    unsafe { fputc(i32::from(b'\n'), out) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fgets(buffer: *mut u8, count: i32, file: *mut File) -> *mut u8 {
    if buffer.is_null() || count <= 0 {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return core::ptr::null_mut();
    }
    let l = Locked::new(file);
    let f = l.file();
    let mut n = 0usize;
    while n + 1 < count as usize {
        let c = get_byte(f);
        if c == EOF {
            break;
        }
        unsafe { *buffer.add(n) = c as u8 };
        n += 1;
        if c == i32::from(b'\n') {
            break;
        }
    }
    if n == 0 {
        return core::ptr::null_mut();
    }
    unsafe { *buffer.add(n) = 0 };
    buffer
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ungetc(c: i32, file: *mut File) -> i32 {
    if c == EOF || file.is_null() {
        return EOF;
    }
    let l = Locked::new(file);
    let f = l.file();
    f.ungot = c & 0xFF;
    f.flags &= !AT_EOF;
    c & 0xFF
}
/// Is the stream a UTF-16 text stream (`_O_U16TEXT`, `_O_WTEXT`)?
fn wide_stream(f: &File) -> bool {
    fd(f.fd).is_some_and(|d| d.mode == O_U16TEXT || d.mode == O_WTEXT)
}
/// Write the UTF-16 units of `units` to a stream: as UTF-16 for wide
/// streams, as UTF-8 otherwise (the process code page).
fn put_wide(f: &mut File, units: &[u16]) -> bool {
    if wide_stream(f) {
        let mut bytes = [0u8; 512];
        let mut n = 0;
        for &u in units {
            if u == u16::from(b'\n') && fd(f.fd).is_some_and(|d| d.mode == O_WTEXT) {
                bytes[n..n + 2].copy_from_slice(&0x0Du16.to_le_bytes());
                n += 2;
            }
            bytes[n..n + 2].copy_from_slice(&u.to_le_bytes());
            n += 2;
            if n >= bytes.len() - 4 {
                if put_bytes(f, &bytes[..n]) != n {
                    return false;
                }
                n = 0;
            }
        }
        return n == 0 || put_bytes(f, &bytes[..n]) == n;
    }
    let mut bytes = [0u8; 512];
    let mut n = 0;
    let mut i = 0;
    while i < units.len() {
        let (c, used) = convert::wide_char(&units[i..]);
        let mut b = [0u8; 4];
        let mut k = convert::encode(c, &mut b);
        if k == 0 {
            b = [0xEF, 0xBF, 0xBD, 0];
            k = 3;
        }
        bytes[n..n + k].copy_from_slice(&b[..k]);
        n += k;
        i += used.max(1);
        if n >= bytes.len() - 8 {
            if put_bytes(f, &bytes[..n]) != n {
                return false;
            }
            n = 0;
        }
    }
    n == 0 || put_bytes(f, &bytes[..n]) == n
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fputwc(c: u16, file: *mut File) -> u16 {
    let l = Locked::new(file);
    if put_wide(l.file(), &[c]) { c } else { WEOF }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn putwc(c: u16, file: *mut File) -> u16 {
    unsafe { fputwc(c, file) }
}
#[unsafe(no_mangle)]
pub extern "C" fn putwchar(c: u16) -> u16 {
    unsafe { fputwc(c, __acrt_iob_func(1)) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fputws(s: *const u16, file: *mut File) -> i32 {
    let n = unsafe { super::length(s) };
    let l = Locked::new(file);
    if put_wide(l.file(), unsafe { core::slice::from_raw_parts(s, n) }) {
        0
    } else {
        EOF
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _putws(s: *const u16) -> i32 {
    let out = __acrt_iob_func(1);
    if unsafe { fputws(s, out) } == EOF {
        return EOF;
    }
    if unsafe { fputwc(u16::from(b'\n'), out) } == WEOF {
        EOF
    } else {
        0
    }
}
/// Read one wide character: UTF-16 from wide streams, UTF-8 otherwise.
fn get_wide(f: &mut File) -> u16 {
    if wide_stream(f) {
        let a = get_byte(f);
        let b = get_byte(f);
        if a == EOF || b == EOF {
            return WEOF;
        }
        return u16::from_le_bytes([a as u8, b as u8]);
    }
    let first = get_byte(f);
    if first == EOF {
        return WEOF;
    }
    let mut bytes = [first as u8, 0, 0, 0];
    let n = match first as u8 {
        0..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    };
    for b in bytes.iter_mut().take(n).skip(1) {
        let c = get_byte(f);
        if c == EOF {
            return WEOF;
        }
        *b = c as u8;
    }
    match unsafe { convert::decode(bytes.as_ptr(), n) } {
        Some((c, _)) if c < 0x10000 => c as u16,
        _ => 0xFFFD,
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fgetwc(file: *mut File) -> u16 {
    let l = Locked::new(file);
    get_wide(l.file())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getwc(file: *mut File) -> u16 {
    unsafe { fgetwc(file) }
}
#[unsafe(no_mangle)]
pub extern "C" fn getwchar() -> u16 {
    unsafe { fgetwc(__acrt_iob_func(0)) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fgetws(buffer: *mut u16, count: i32, file: *mut File) -> *mut u16 {
    if buffer.is_null() || count <= 0 {
        invalid_parameter();
        return core::ptr::null_mut();
    }
    let l = Locked::new(file);
    let f = l.file();
    let mut n = 0usize;
    while n + 1 < count as usize {
        let c = get_wide(f);
        if c == WEOF {
            break;
        }
        unsafe { *buffer.add(n) = c };
        n += 1;
        if c == u16::from(b'\n') {
            break;
        }
    }
    if n == 0 {
        return core::ptr::null_mut();
    }
    unsafe { *buffer.add(n) = 0 };
    buffer
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ungetwc(c: u16, file: *mut File) -> u16 {
    if c == WEOF || file.is_null() {
        return WEOF;
    }
    let l = Locked::new(file);
    let f = l.file();
    // A wide character pushed back as its first byte only when ASCII.
    if c < 128 {
        f.ungot = i32::from(c);
    }
    f.flags &= !AT_EOF;
    c
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _fseeki64(file: *mut File, offset: i64, origin: i32) -> i32 {
    if file.is_null() || !(0..=2).contains(&origin) {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return -1;
    }
    let l = Locked::new(file);
    let f = l.file();
    // flush_locked rewinds the unread buffered bytes; an ungetc character
    // is one more before the logical position.
    let adjust = i64::from(f.ungot >= 0);
    flush_locked(f);
    f.ungot = -1;
    let offset = if origin == 1 { offset - adjust } else { offset };
    if _lseeki64(f.fd, offset, origin) < 0 {
        return -1;
    }
    f.flags &= !AT_EOF;
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fseek(file: *mut File, offset: i32, origin: i32) -> i32 {
    unsafe { _fseeki64(file, i64::from(offset), origin) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _ftelli64(file: *mut File) -> i64 {
    if file.is_null() {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return -1;
    }
    let l = Locked::new(file);
    let f = l.file();
    let base = _lseeki64(f.fd, 0, 1);
    if base < 0 {
        return -1;
    }
    if f.flags & WRITING != 0 {
        base + f.used as i64
    } else if f.flags & READING != 0 {
        base - (f.used - f.position) as i64 - i64::from(f.ungot >= 0)
    } else {
        base
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ftell(file: *mut File) -> i32 {
    let p = unsafe { _ftelli64(file) };
    if p > i64::from(i32::MAX) {
        set_errno(errno::EINVAL);
        return -1;
    }
    p as i32
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rewind(file: *mut File) {
    unsafe { _fseeki64(file, 0, 0) };
    if !file.is_null() {
        let l = Locked::new(file);
        l.file().flags &= !ERROR;
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fgetpos(file: *mut File, position: *mut i64) -> i32 {
    let p = unsafe { _ftelli64(file) };
    if p < 0 || position.is_null() {
        return -1;
    }
    unsafe { *position = p };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fsetpos(file: *mut File, position: *const i64) -> i32 {
    if position.is_null() {
        return -1;
    }
    unsafe { _fseeki64(file, *position, 0) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn feof(file: *mut File) -> i32 {
    i32::from(!file.is_null() && unsafe { (*file).flags } & AT_EOF != 0)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ferror(file: *mut File) -> i32 {
    i32::from(!file.is_null() && unsafe { (*file).flags } & ERROR != 0)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn clearerr(file: *mut File) {
    if !file.is_null() {
        let l = Locked::new(file);
        l.file().flags &= !(ERROR | AT_EOF);
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn clearerr_s(file: *mut File) -> i32 {
    unsafe { clearerr(file) };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _get_stream_buffer_pointers(
    file: *mut File,
    base: *mut *mut *mut u8,
    pointer: *mut *mut *mut u8,
    count: *mut *mut i32,
) -> i32 {
    let f = unsafe { &mut *file };
    unsafe {
        if !base.is_null() {
            *base = &mut f.buffer;
        }
        if !pointer.is_null() {
            *pointer = &mut f.buffer;
        }
        if !count.is_null() {
            *count = &mut f.used as *mut usize as *mut i32;
        }
    }
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn _getmaxstdio() -> i32 {
    MAX_STREAMS as i32
}
#[unsafe(no_mangle)]
pub extern "C" fn _setmaxstdio(n: i32) -> i32 {
    if (20..=MAX_STREAMS as i32).contains(&n) {
        n
    } else {
        -1
    }
}

// ---------------------------------------------------------------------------
// printf and scanf over a C va_list.

/// A va_list: a pointer to the arguments' slots (8 bytes each on x64; 4,
/// or 8 for 64-bit integers and doubles, on x86).
struct VaList(usize);
impl VaList {
    fn take<T: Copy>(&mut self, slot: usize) -> T {
        let v = unsafe { (self.0 as *const T).read_unaligned() };
        self.0 += slot;
        v
    }
}
impl format::Args for VaList {
    fn int(&mut self, size: usize) -> u64 {
        if cfg!(target_pointer_width = "64") || size == 8 {
            self.take::<u64>(8)
        } else {
            u64::from(self.take::<u32>(4))
        }
    }
    fn double(&mut self) -> f64 {
        self.take::<f64>(8)
    }
    fn pointer(&mut self) -> usize {
        self.take::<usize>(core::mem::size_of::<usize>())
    }
    fn unit(&mut self, pointer: usize, index: usize, wide: bool) -> u16 {
        unsafe {
            if wide {
                *(pointer as *const u16).add(index)
            } else {
                u16::from(*(pointer as *const u8).add(index))
            }
        }
    }
    fn store(&mut self, _pointer: usize, _size: usize, _count: u64) {}
}
impl scan::Store for VaList {
    fn pointer(&mut self) -> usize {
        self.take::<usize>(core::mem::size_of::<usize>())
    }
    fn integer(&mut self, pointer: usize, size: usize, value: u64) {
        unsafe {
            match size {
                1 => *(pointer as *mut u8) = value as u8,
                2 => (pointer as *mut u16).write_unaligned(value as u16),
                8 => (pointer as *mut u64).write_unaligned(value),
                _ => (pointer as *mut u32).write_unaligned(value as u32),
            }
        }
    }
    fn float(&mut self, pointer: usize, double: bool, value: f64) {
        unsafe {
            if double {
                (pointer as *mut f64).write_unaligned(value);
            } else {
                (pointer as *mut f32).write_unaligned(value as f32);
            }
        }
    }
    fn unit(&mut self, pointer: usize, index: usize, wide: bool, unit: u16) -> bool {
        unsafe {
            if wide {
                *(pointer as *mut u16).add(index) = unit;
            } else {
                *(pointer as *mut u8).add(index) = unit as u8;
            }
        }
        true
    }
    fn size(&mut self) -> Option<usize> {
        // The secure functions' buffer sizes are unsigned ints.
        Some(if cfg!(target_pointer_width = "64") {
            self.take::<u64>(8) as u32 as usize
        } else {
            self.take::<u32>(4) as usize
        })
    }
}

/// A NUL-terminated format of units.
unsafe fn units<T: Copy + Into<u32>>(format: *const T, out: &mut [u16]) -> Option<usize> {
    let mut n = 0;
    loop {
        let u: u32 = unsafe { *format.add(n) }.into();
        if u == 0 {
            return Some(n);
        }
        if n >= out.len() {
            return None;
        }
        out[n] = u as u16;
        n += 1;
    }
}
/// The format's units (narrow formats are bytes: UTF-8 passes through).
fn with_format<T: Copy + Into<u32>, R>(format: *const T, f: impl FnOnce(&[u16]) -> R) -> Option<R> {
    if format.is_null() {
        return None;
    }
    let n = unsafe { super::length_of_units(format) };
    let p = heap::malloc((n + 1) * 2) as *mut u16;
    if p.is_null() {
        return None;
    }
    let buffer = unsafe { core::slice::from_raw_parts_mut(p, n + 1) };
    let len = unsafe { units(format, buffer) }?;
    let r = f(&buffer[..len]);
    unsafe { heap::free(p as *mut u8) };
    Some(r)
}

/// Bytes into a stream (narrow printf).
struct StreamSink<'a> {
    file: &'a mut File,
    pending: [u8; 256],
    n: usize,
    failed: bool,
}
impl format::Sink for StreamSink<'_> {
    fn put(&mut self, unit: u16) -> bool {
        self.pending[self.n] = unit as u8;
        self.n += 1;
        if self.n == self.pending.len() {
            self.flush();
        }
        !self.failed
    }
    fn wide(&self) -> bool {
        false
    }
}
impl StreamSink<'_> {
    fn flush(&mut self) {
        if self.n > 0 && put_bytes(self.file, &self.pending[..self.n]) != self.n {
            self.failed = true;
        }
        self.n = 0;
    }
}
/// UTF-16 into a stream (wide printf).
struct WideStreamSink<'a> {
    file: &'a mut File,
    pending: [u16; 256],
    n: usize,
    failed: bool,
}
impl format::Sink for WideStreamSink<'_> {
    fn put(&mut self, unit: u16) -> bool {
        self.pending[self.n] = unit;
        self.n += 1;
        if self.n == self.pending.len() - 1 {
            self.flush();
        }
        !self.failed
    }
    fn wide(&self) -> bool {
        true
    }
}
impl WideStreamSink<'_> {
    fn flush(&mut self) {
        // Keep a surrogate pair together.
        let keep = usize::from(self.n > 0 && (0xD800..0xDC00).contains(&self.pending[self.n - 1]));
        let take = self.n - keep;
        if take > 0 && !put_wide(self.file, &self.pending[..take]) {
            self.failed = true;
        }
        if keep == 1 {
            self.pending[0] = self.pending[self.n - 1];
        }
        self.n = keep;
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vfprintf(
    options: u64,
    file: *mut File,
    format_text: *const u8,
    _locale: usize,
    args: usize,
) -> i32 {
    if file.is_null() || format_text.is_null() {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return -1;
    }
    let l = Locked::new(file);
    let mut sink = StreamSink {
        file: l.file(),
        pending: [0; 256],
        n: 0,
        failed: false,
    };
    let r = with_format(format_text, |f| {
        format::format(f, &mut VaList(args), &mut sink, options)
    });
    sink.flush();
    match r {
        Some(Ok(n)) if !sink.failed => n as i32,
        _ => -1,
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vfprintf_s(
    options: u64,
    file: *mut File,
    format_text: *const u8,
    locale: usize,
    args: usize,
) -> i32 {
    unsafe { __stdio_common_vfprintf(options, file, format_text, locale, args) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vfprintf_p(
    options: u64,
    file: *mut File,
    format_text: *const u8,
    locale: usize,
    args: usize,
) -> i32 {
    unsafe { __stdio_common_vfprintf(options, file, format_text, locale, args) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vfwprintf(
    options: u64,
    file: *mut File,
    format_text: *const u16,
    _locale: usize,
    args: usize,
) -> i32 {
    if file.is_null() || format_text.is_null() {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return -1;
    }
    let l = Locked::new(file);
    let mut sink = WideStreamSink {
        file: l.file(),
        pending: [0; 256],
        n: 0,
        failed: false,
    };
    let r = with_format(format_text, |f| {
        format::format(f, &mut VaList(args), &mut sink, options)
    });
    sink.flush();
    match r {
        Some(Ok(n)) if !sink.failed => n as i32,
        _ => -1,
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vfwprintf_s(
    options: u64,
    file: *mut File,
    format_text: *const u16,
    locale: usize,
    args: usize,
) -> i32 {
    unsafe { __stdio_common_vfwprintf(options, file, format_text, locale, args) }
}

/// A caller's buffer of `capacity` units (narrow bytes or UTF-16).
struct BufferSink<T> {
    buffer: *mut T,
    capacity: usize,
    n: usize,
}
impl format::Sink for BufferSink<u8> {
    fn put(&mut self, unit: u16) -> bool {
        if self.n < self.capacity {
            unsafe { *self.buffer.add(self.n) = unit as u8 };
            self.n += 1;
            true
        } else {
            false
        }
    }
    fn wide(&self) -> bool {
        false
    }
}
impl format::Sink for BufferSink<u16> {
    fn put(&mut self, unit: u16) -> bool {
        if self.n < self.capacity {
            unsafe { *self.buffer.add(self.n) = unit };
            self.n += 1;
            true
        } else {
            false
        }
    }
    fn wide(&self) -> bool {
        true
    }
}
/// The sprintf family's rules on a full buffer (`__stdio_common_vsprintf`
/// and `vswprintf`): the result fits (terminated, its length); a null
/// buffer of zero units (the length needed); otherwise per the options:
/// STANDARD_SNPRINTF_BEHAVIOR truncates, terminates and returns the full
/// length, LEGACY_VSPRINTF_NULL_TERMINATION returns -1 unterminated when
/// exactly full, else -1 with the buffer terminated at its end.
unsafe fn sprintf<T: Copy + Default + Into<u32>>(
    options: u64,
    buffer: *mut T,
    capacity: usize,
    format_text: *const T,
    args: usize,
) -> i32
where
    BufferSink<T>: format::Sink,
{
    if format_text.is_null() || (buffer.is_null() && capacity != 0) {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return -1;
    }
    let mut sink = BufferSink {
        buffer,
        capacity,
        n: 0,
    };
    let Some(Ok(length)) = with_format(format_text, |f| {
        format::format(f, &mut VaList(args), &mut sink, options)
    }) else {
        if !buffer.is_null() && capacity > 0 {
            unsafe { *buffer = T::default() };
        }
        return -1;
    };
    if buffer.is_null() {
        return length as i32;
    }
    if length < capacity {
        unsafe { *buffer.add(length) = T::default() };
        return length as i32;
    }
    if options & format::options::STANDARD_SNPRINTF_BEHAVIOR != 0 {
        if capacity > 0 {
            unsafe { *buffer.add(capacity - 1) = T::default() };
        }
        return length as i32;
    }
    if length == capacity && options & format::options::LEGACY_VSPRINTF_NULL_TERMINATION != 0 {
        return length as i32;
    }
    -1
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vsprintf(
    options: u64,
    buffer: *mut u8,
    capacity: usize,
    format_text: *const u8,
    _locale: usize,
    args: usize,
) -> i32 {
    unsafe { sprintf(options, buffer, capacity, format_text, args) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vswprintf(
    options: u64,
    buffer: *mut u16,
    capacity: usize,
    format_text: *const u16,
    _locale: usize,
    args: usize,
) -> i32 {
    unsafe { sprintf(options, buffer, capacity, format_text, args) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vsprintf_p(
    options: u64,
    buffer: *mut u8,
    capacity: usize,
    format_text: *const u8,
    locale: usize,
    args: usize,
) -> i32 {
    unsafe { __stdio_common_vsprintf(options, buffer, capacity, format_text, locale, args) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vswprintf_p(
    options: u64,
    buffer: *mut u16,
    capacity: usize,
    format_text: *const u16,
    locale: usize,
    args: usize,
) -> i32 {
    unsafe { __stdio_common_vswprintf(options, buffer, capacity, format_text, locale, args) }
}
/// sprintf_s: the result must fit (else the buffer is emptied, EINVAL or
/// ERANGE, -1).
unsafe fn sprintf_s<T: Copy + Default + Into<u32>>(
    options: u64,
    buffer: *mut T,
    capacity: usize,
    format_text: *const T,
    args: usize,
) -> i32
where
    BufferSink<T>: format::Sink,
{
    if buffer.is_null() || capacity == 0 || format_text.is_null() {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return -1;
    }
    let mut sink = BufferSink {
        buffer,
        capacity,
        n: 0,
    };
    match with_format(format_text, |f| {
        format::format(f, &mut VaList(args), &mut sink, options)
    }) {
        Some(Ok(length)) if length < capacity => {
            unsafe { *buffer.add(length) = T::default() };
            length as i32
        }
        Some(Ok(_)) => {
            unsafe { *buffer = T::default() };
            invalid_parameter();
            set_errno(errno::ERANGE);
            -1
        }
        _ => {
            unsafe { *buffer = T::default() };
            invalid_parameter();
            set_errno(errno::EINVAL);
            -1
        }
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vsprintf_s(
    options: u64,
    buffer: *mut u8,
    capacity: usize,
    format_text: *const u8,
    _locale: usize,
    args: usize,
) -> i32 {
    unsafe { sprintf_s(options, buffer, capacity, format_text, args) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vswprintf_s(
    options: u64,
    buffer: *mut u16,
    capacity: usize,
    format_text: *const u16,
    _locale: usize,
    args: usize,
) -> i32 {
    unsafe { sprintf_s(options, buffer, capacity, format_text, args) }
}
/// _snprintf_s: `count` units at most (`_TRUNCATE`: whatever fits, -1
/// when truncated); the buffer is always terminated.
unsafe fn snprintf_s<T: Copy + Default + Into<u32>>(
    options: u64,
    buffer: *mut T,
    capacity: usize,
    count: usize,
    format_text: *const T,
    args: usize,
) -> i32
where
    BufferSink<T>: format::Sink,
{
    if buffer.is_null() || capacity == 0 || format_text.is_null() {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return -1;
    }
    let limit = if count == usize::MAX {
        capacity - 1
    } else {
        count.min(capacity - 1)
    };
    let mut sink = BufferSink {
        buffer,
        capacity: limit,
        n: 0,
    };
    match with_format(format_text, |f| {
        format::format(f, &mut VaList(args), &mut sink, options)
    }) {
        Some(Ok(length)) if length <= limit => {
            unsafe { *buffer.add(length) = T::default() };
            length as i32
        }
        Some(Ok(_)) if count == usize::MAX || count < capacity => {
            unsafe { *buffer.add(limit) = T::default() };
            if count == usize::MAX {
                set_errno(errno::STRUNCATE);
            }
            -1
        }
        Some(Ok(_)) => {
            unsafe { *buffer = T::default() };
            invalid_parameter();
            set_errno(errno::ERANGE);
            -1
        }
        _ => {
            unsafe { *buffer = T::default() };
            set_errno(errno::EINVAL);
            -1
        }
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vsnprintf_s(
    options: u64,
    buffer: *mut u8,
    capacity: usize,
    count: usize,
    format_text: *const u8,
    _locale: usize,
    args: usize,
) -> i32 {
    unsafe { snprintf_s(options, buffer, capacity, count, format_text, args) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vsnwprintf_s(
    options: u64,
    buffer: *mut u16,
    capacity: usize,
    count: usize,
    format_text: *const u16,
    _locale: usize,
    args: usize,
) -> i32 {
    unsafe { snprintf_s(options, buffer, capacity, count, format_text, args) }
}
/// `_CRT_INTERNAL_SCANF_SECURECRT`.
const SCANF_SECURE: u64 = 0x1;
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vsscanf(
    options: u64,
    input: *const u8,
    count: usize,
    format_text: *const u8,
    _locale: usize,
    args: usize,
) -> i32 {
    if input.is_null() || format_text.is_null() {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return -1;
    }
    let length = if count == usize::MAX {
        unsafe { super::length(input) }
    } else {
        unsafe { super::string::strnlen(input, count) }
    };
    let get = |i: usize| {
        if i < length {
            u16::from(unsafe { *input.add(i) })
        } else {
            0
        }
    };
    let secure = options & SCANF_SECURE != 0;
    with_format(format_text, |f| {
        scan::scan(f, &get, &mut VaList(args), false, secure)
    })
    .unwrap_or(-1)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vswscanf(
    options: u64,
    input: *const u16,
    count: usize,
    format_text: *const u16,
    _locale: usize,
    args: usize,
) -> i32 {
    if input.is_null() || format_text.is_null() {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return -1;
    }
    let length = if count == usize::MAX {
        unsafe { super::length(input) }
    } else {
        unsafe { super::string::wcsnlen(input, count) }
    };
    let get = |i: usize| {
        if i < length {
            unsafe { *input.add(i) }
        } else {
            0
        }
    };
    let secure = options & SCANF_SECURE != 0;
    with_format(format_text, |f| {
        scan::scan(f, &get, &mut VaList(args), true, secure)
    })
    .unwrap_or(-1)
}
/// fscanf: a line at a time is enough for the formats programs read from
/// streams (the rest of the line stays buffered for the next read).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vfscanf(
    options: u64,
    file: *mut File,
    format_text: *const u8,
    locale: usize,
    args: usize,
) -> i32 {
    let mut line = [0u8; 4096];
    if unsafe { fgets(line.as_mut_ptr(), line.len() as i32, file) }.is_null() {
        return -1;
    }
    unsafe {
        __stdio_common_vsscanf(
            options,
            line.as_ptr(),
            usize::MAX,
            format_text,
            locale,
            args,
        )
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vfwscanf(
    options: u64,
    file: *mut File,
    format_text: *const u16,
    locale: usize,
    args: usize,
) -> i32 {
    let mut line = [0u16; 4096];
    if unsafe { fgetws(line.as_mut_ptr(), line.len() as i32, file) }.is_null() {
        return -1;
    }
    unsafe {
        __stdio_common_vswscanf(
            options,
            line.as_ptr(),
            usize::MAX,
            format_text,
            locale,
            args,
        )
    }
}
