//! The program's arguments and environment as the CRT keeps them
//! (`__argc`, `__argv`, `__wargv`, `_environ`, `_wenviron`), built from
//! GetCommandLineW and GetEnvironmentStringsW with Windows' parsing rules,
//! narrow forms in UTF-8; getenv/_putenv over them and the process's own
//! environment block (kept in step, as the CRT does).
use super::{
    FreeEnvironmentStringsW, GetCommandLineW, GetEnvironmentStringsW, SetEnvironmentVariableW,
    convert, errno, heap, invalid_parameter, set_errno,
};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static ARGC: AtomicUsize = AtomicUsize::new(0);
static ARGV: AtomicUsize = AtomicUsize::new(0);
static WARGV: AtomicUsize = AtomicUsize::new(0);
static ENVIRON: AtomicUsize = AtomicUsize::new(0);
static WENVIRON: AtomicUsize = AtomicUsize::new(0);
static INITIAL_ENVIRON: AtomicUsize = AtomicUsize::new(0);
static INITIAL_WENVIRON: AtomicUsize = AtomicUsize::new(0);
static ENV_LOCK: AtomicBool = AtomicBool::new(false);
struct Lock;
impl Lock {
    fn take() -> Self {
        while ENV_LOCK
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        Self
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        ENV_LOCK.store(false, Ordering::Release);
    }
}

/// Split a command line into argument units (Windows' rules, as
/// CommandLineToArgvW and the CRT's startup parse it: the program name
/// up to the next quote or space; then 2n backslashes and a quote are n
/// backslashes and a toggle, 2n+1 backslashes and a quote are n and a
/// literal quote, "" inside quotes is a literal quote).
fn split(line: &[u16], mut each: impl FnMut(&[u16])) {
    let mut i;
    // The program name.
    let mut name_start = 0;
    let name_end;
    if line.first() == Some(&u16::from(b'"')) {
        name_start = 1;
        let mut k = 1;
        while k < line.len() && line[k] != u16::from(b'"') {
            k += 1;
        }
        name_end = k;
        i = (k + 1).min(line.len());
    } else {
        let mut k = 0;
        while k < line.len() && line[k] != u16::from(b' ') && line[k] != u16::from(b'\t') {
            k += 1;
        }
        name_end = k;
        i = k;
    }
    each(&line[name_start..name_end]);
    // An argument is never longer than the line.
    let space = heap::malloc((line.len() + 1) * 2) as *mut u16;
    if space.is_null() {
        return;
    }
    // SAFETY: a fresh block of line.len() + 1 units, freed below.
    let buffer = unsafe { core::slice::from_raw_parts_mut(space, line.len() + 1) };
    loop {
        while i < line.len() && (line[i] == u16::from(b' ') || line[i] == u16::from(b'\t')) {
            i += 1;
        }
        if i >= line.len() {
            break;
        }
        let mut n = 0;
        let mut quoted = false;
        while i < line.len() {
            let c = line[i];
            if !quoted && (c == u16::from(b' ') || c == u16::from(b'\t')) {
                break;
            }
            if c == u16::from(b'\\') {
                let mut slashes = 0;
                while i < line.len() && line[i] == u16::from(b'\\') {
                    slashes += 1;
                    i += 1;
                }
                if i < line.len() && line[i] == u16::from(b'"') {
                    for _ in 0..slashes / 2 {
                        buffer[n] = u16::from(b'\\');
                        n += 1;
                    }
                    if slashes % 2 == 1 {
                        buffer[n] = u16::from(b'"');
                        n += 1;
                        i += 1;
                    }
                } else {
                    for _ in 0..slashes {
                        buffer[n] = u16::from(b'\\');
                        n += 1;
                    }
                }
                continue;
            }
            if c == u16::from(b'"') {
                if quoted && i + 1 < line.len() && line[i + 1] == u16::from(b'"') {
                    buffer[n] = u16::from(b'"');
                    n += 1;
                    i += 2;
                    continue;
                }
                quoted = !quoted;
                i += 1;
                continue;
            }
            buffer[n] = c;
            n += 1;
            i += 1;
        }
        each(&buffer[..n]);
    }
    unsafe { heap::free(space as *mut u8) };
}
fn command_line() -> &'static [u16] {
    let p = unsafe { GetCommandLineW() } as *const u16;
    if p.is_null() {
        return &[];
    }
    unsafe { core::slice::from_raw_parts(p, super::length(p)) }
}
/// A heap copy of `units` with a terminator.
fn wide_copy(units: &[u16]) -> usize {
    let p = heap::malloc((units.len() + 1) * 2) as *mut u16;
    if !p.is_null() {
        unsafe {
            core::ptr::copy_nonoverlapping(units.as_ptr(), p, units.len());
            *p.add(units.len()) = 0;
        }
    }
    p as usize
}
/// A heap UTF-8 copy of `units` with a terminator.
fn narrow_copy(units: &[u16]) -> usize {
    let mut bytes = 0;
    let mut k = 0;
    while k < units.len() {
        let (c, used) = convert::wide_char(&units[k..]);
        let mut b = [0u8; 4];
        bytes += convert::encode(c, &mut b).max(3);
        k += used.max(1);
    }
    let p = heap::malloc(bytes + 1);
    if p.is_null() {
        return 0;
    }
    let mut n = 0;
    let mut k = 0;
    while k < units.len() {
        let (c, used) = convert::wide_char(&units[k..]);
        let mut b = [0u8; 4];
        let mut len = convert::encode(c, &mut b);
        if len == 0 {
            b = [0xEF, 0xBF, 0xBD, 0];
            len = 3;
        }
        unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), p.add(n), len) };
        n += len;
        k += used.max(1);
    }
    unsafe { *p.add(n) = 0 };
    p as usize
}
fn configure_argv(wide: bool) -> i32 {
    let line = command_line();
    let mut count = 0usize;
    split(line, |_| count += 1);
    let table = heap::calloc(count + 1, core::mem::size_of::<usize>()) as *mut usize;
    if table.is_null() {
        set_errno(errno::ENOMEM);
        return errno::ENOMEM;
    }
    let mut i = 0;
    split(line, |arg| {
        unsafe {
            *table.add(i) = if wide {
                wide_copy(arg)
            } else {
                narrow_copy(arg)
            }
        };
        i += 1;
    });
    ARGC.store(count, Ordering::Release);
    if wide {
        WARGV.store(table as usize, Ordering::Release);
    } else {
        ARGV.store(table as usize, Ordering::Release);
    }
    0
}
/// `_configure_narrow_argv` / `_configure_wide_argv` (mode 1: unexpanded,
/// 2: expanded wildcards; wildcards are kept as written).
#[unsafe(no_mangle)]
pub extern "C" fn _configure_narrow_argv(_mode: i32) -> i32 {
    configure_argv(false)
}
#[unsafe(no_mangle)]
pub extern "C" fn _configure_wide_argv(_mode: i32) -> i32 {
    configure_argv(true)
}
fn ensure(wide: bool) {
    let set = if wide { &WARGV } else { &ARGV };
    if set.load(Ordering::Acquire) == 0 {
        configure_argv(wide);
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn __p___argc() -> *mut i32 {
    ensure(false);
    // The count is a usize here; argc is the low int of it.
    ARGC.as_ptr() as *mut i32
}
#[unsafe(no_mangle)]
pub extern "C" fn __p___argv() -> *mut usize {
    ensure(false);
    ARGV.as_ptr()
}
#[unsafe(no_mangle)]
pub extern "C" fn __p___wargv() -> *mut usize {
    ensure(true);
    WARGV.as_ptr()
}
#[unsafe(no_mangle)]
pub extern "C" fn __p__acmdln() -> *mut usize {
    static LINE: AtomicUsize = AtomicUsize::new(0);
    if LINE.load(Ordering::Acquire) == 0 {
        LINE.store(narrow_copy(command_line()), Ordering::Release);
    }
    LINE.as_ptr()
}
#[unsafe(no_mangle)]
pub extern "C" fn __p__wcmdln() -> *mut usize {
    static LINE: AtomicUsize = AtomicUsize::new(0);
    if LINE.load(Ordering::Acquire) == 0 {
        LINE.store(unsafe { GetCommandLineW() }, Ordering::Release);
    }
    LINE.as_ptr()
}

/// Build `_environ` / `_wenviron` from the process's block.
fn build_environment(wide: bool) -> usize {
    let block = unsafe { GetEnvironmentStringsW() } as *const u16;
    if block.is_null() {
        return 0;
    }
    let mut count = 0;
    let mut p = block;
    loop {
        let n = unsafe { super::length(p) };
        if n == 0 {
            break;
        }
        // Entries naming drive directories ("=C:=C:\") are kept out.
        if unsafe { *p } != u16::from(b'=') {
            count += 1;
        }
        p = unsafe { p.add(n + 1) };
    }
    let table = heap::calloc(count + 1, core::mem::size_of::<usize>()) as *mut usize;
    if table.is_null() {
        unsafe { FreeEnvironmentStringsW(block as usize) };
        return 0;
    }
    let mut i = 0;
    let mut p = block;
    loop {
        let n = unsafe { super::length(p) };
        if n == 0 {
            break;
        }
        if unsafe { *p } != u16::from(b'=') {
            let units = unsafe { core::slice::from_raw_parts(p, n) };
            unsafe {
                *table.add(i) = if wide {
                    wide_copy(units)
                } else {
                    narrow_copy(units)
                }
            };
            i += 1;
        }
        p = unsafe { p.add(n + 1) };
    }
    unsafe { FreeEnvironmentStringsW(block as usize) };
    table as usize
}
#[unsafe(no_mangle)]
pub extern "C" fn _initialize_narrow_environment() -> i32 {
    let _l = Lock::take();
    if ENVIRON.load(Ordering::Acquire) == 0 {
        let t = build_environment(false);
        ENVIRON.store(t, Ordering::Release);
        INITIAL_ENVIRON.store(t, Ordering::Release);
    }
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn _initialize_wide_environment() -> i32 {
    let _l = Lock::take();
    if WENVIRON.load(Ordering::Acquire) == 0 {
        let t = build_environment(true);
        WENVIRON.store(t, Ordering::Release);
        INITIAL_WENVIRON.store(t, Ordering::Release);
    }
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn _get_initial_narrow_environment() -> usize {
    _initialize_narrow_environment();
    INITIAL_ENVIRON.load(Ordering::Acquire)
}
#[unsafe(no_mangle)]
pub extern "C" fn _get_initial_wide_environment() -> usize {
    _initialize_wide_environment();
    INITIAL_WENVIRON.load(Ordering::Acquire)
}
#[unsafe(no_mangle)]
pub extern "C" fn __p__environ() -> *mut usize {
    _initialize_narrow_environment();
    ENVIRON.as_ptr()
}
#[unsafe(no_mangle)]
pub extern "C" fn __p__wenviron() -> *mut usize {
    _initialize_wide_environment();
    WENVIRON.as_ptr()
}

/// The value of `name` in a table of "NAME=value" entries (names compare
/// case-insensitively, as on Windows).
unsafe fn find<T: Copy + Into<u32>>(table: usize, name: *const T) -> Option<*const T> {
    if table == 0 {
        return None;
    }
    let n = unsafe { super::length_of_units(name) };
    let mut i = 0;
    loop {
        let entry = unsafe { *(table as *const usize).add(i) } as *const T;
        if entry.is_null() {
            return None;
        }
        let mut k = 0;
        let mut same = true;
        while k < n {
            let (a, b): (u32, u32) = unsafe { ((*entry.add(k)).into(), (*name.add(k)).into()) };
            let fold = |c: u32| if (65..=90).contains(&c) { c + 32 } else { c };
            if a == 0 || fold(a) != fold(b) {
                same = false;
                break;
            }
            k += 1;
        }
        if same && unsafe { (*entry.add(n)).into() } == u32::from(b'=') {
            return Some(unsafe { entry.add(n + 1) });
        }
        i += 1;
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getenv(name: *const u8) -> *const u8 {
    if name.is_null() {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return core::ptr::null();
    }
    _initialize_narrow_environment();
    let _l = Lock::take();
    unsafe { find(ENVIRON.load(Ordering::Acquire), name) }.unwrap_or(core::ptr::null())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wgetenv(name: *const u16) -> *const u16 {
    if name.is_null() {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return core::ptr::null();
    }
    _initialize_wide_environment();
    let _l = Lock::take();
    unsafe { find(WENVIRON.load(Ordering::Acquire), name) }.unwrap_or(core::ptr::null())
}
unsafe fn copy_out<T: Copy + Into<u32> + Default>(
    value: Option<*const T>,
    required: *mut usize,
    buffer: *mut T,
    capacity: usize,
) -> i32 {
    let Some(v) = value else {
        if !required.is_null() {
            unsafe { *required = 0 };
        }
        if !buffer.is_null() && capacity > 0 {
            unsafe { *buffer = T::default() };
        }
        return 0;
    };
    let n = unsafe { super::length_of_units(v) } + 1;
    if !required.is_null() {
        unsafe { *required = n };
    }
    if buffer.is_null() || capacity == 0 {
        return 0;
    }
    if capacity < n {
        unsafe { *buffer = T::default() };
        return errno::ERANGE;
    }
    unsafe { core::ptr::copy_nonoverlapping(v, buffer, n) };
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getenv_s(
    required: *mut usize,
    buffer: *mut u8,
    capacity: usize,
    name: *const u8,
) -> i32 {
    let value = unsafe { getenv(name) };
    unsafe {
        copy_out(
            (!value.is_null()).then_some(value),
            required,
            buffer,
            capacity,
        )
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wgetenv_s(
    required: *mut usize,
    buffer: *mut u16,
    capacity: usize,
    name: *const u16,
) -> i32 {
    let value = unsafe { _wgetenv(name) };
    unsafe {
        copy_out(
            (!value.is_null()).then_some(value),
            required,
            buffer,
            capacity,
        )
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _dupenv_s(out: *mut *mut u8, length: *mut usize, name: *const u8) -> i32 {
    let value = unsafe { getenv(name) };
    unsafe {
        *out = if value.is_null() {
            core::ptr::null_mut()
        } else {
            super::string::_strdup(value)
        };
        if !length.is_null() {
            *length = if value.is_null() {
                0
            } else {
                super::length(value) + 1
            };
        }
    }
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wdupenv_s(
    out: *mut *mut u16,
    length: *mut usize,
    name: *const u16,
) -> i32 {
    let value = unsafe { _wgetenv(name) };
    unsafe {
        *out = if value.is_null() {
            core::ptr::null_mut()
        } else {
            super::string::_wcsdup(value)
        };
        if !length.is_null() {
            *length = if value.is_null() {
                0
            } else {
                super::length(value) + 1
            };
        }
    }
    0
}
/// `_wputenv_s`: set (or with an empty value, remove) a variable in the
/// process's environment, then rebuild the CRT's tables from it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wputenv_s(name: *const u16, value: *const u16) -> i32 {
    if name.is_null() || value.is_null() || unsafe { *name } == 0 {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return errno::EINVAL;
    }
    let empty = unsafe { *value } == 0;
    if unsafe { SetEnvironmentVariableW(name as usize, if empty { 0 } else { value as usize }) }
        == 0
        && !empty
    {
        set_errno(errno::EINVAL);
        return errno::EINVAL;
    }
    refresh();
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _putenv_s(name: *const u8, value: *const u8) -> i32 {
    let (Some(wname), Some(wvalue)) = (unsafe { widened(name) }, unsafe { widened(value) }) else {
        set_errno(errno::EINVAL);
        return errno::EINVAL;
    };
    let result = unsafe { _wputenv_s(wname, wvalue) };
    unsafe {
        heap::free(wname as *mut u8);
        heap::free(wvalue as *mut u8);
    }
    result
}
/// A heap UTF-16 copy of the UTF-8 string `s` (free it with `free`).
unsafe fn widened(s: *const u8) -> Option<*mut u16> {
    if s.is_null() {
        return None;
    }
    let units = unsafe { convert::mbstowcs(core::ptr::null_mut(), s, 0) };
    if units == usize::MAX {
        return None;
    }
    let p = heap::malloc((units + 1) * 2) as *mut u16;
    if p.is_null() {
        return None;
    }
    unsafe { convert::mbstowcs(p, s, units + 1) };
    Some(p)
}
/// `_wputenv("NAME=value")`: -1 on failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wputenv(assignment: *const u16) -> i32 {
    if assignment.is_null() {
        return -1;
    }
    let n = unsafe { super::length(assignment) };
    let s = unsafe { core::slice::from_raw_parts(assignment, n) };
    let Some(eq) = s
        .iter()
        .skip(1)
        .position(|&u| u == u16::from(b'='))
        .map(|p| p + 1)
    else {
        set_errno(errno::EINVAL);
        return -1;
    };
    let mut name = [0u16; 512];
    if eq >= name.len() {
        return -1;
    }
    name[..eq].copy_from_slice(&s[..eq]);
    if unsafe { _wputenv_s(name.as_ptr(), assignment.add(eq + 1)) } == 0 {
        0
    } else {
        -1
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _putenv(assignment: *const u8) -> i32 {
    let Some(wide) = (unsafe { widened(assignment) }) else {
        return -1;
    };
    let result = unsafe { _wputenv(wide) };
    unsafe { heap::free(wide as *mut u8) };
    result
}
/// Rebuild the tables (the old ones stay allocated: pointers into them
/// may still be held, as with the CRT's).
fn refresh() {
    let _l = Lock::take();
    if ENVIRON.load(Ordering::Acquire) != 0 {
        ENVIRON.store(build_environment(false), Ordering::Release);
    }
    if WENVIRON.load(Ordering::Acquire) != 0 {
        WENVIRON.store(build_environment(true), Ordering::Release);
    }
}
