//! More of what Chromium, Electron and VS Code import from kernel32 (P18
//! M7, from their import tables): resource-name enumeration, the locale
//! list and UI languages, INI files (GetPrivateProfileString and friends),
//! waitable timers, the volume list, serial-port calls (Doors has no serial
//! ports), package queries (no program runs packaged yet), ReOpenFile and
//! TransactNamedPipe.
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering},
};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorResourceName(module: u64, kind: u64, index: u32, out: u64) -> u64;
    fn DoorGetCurrentThreadId() -> u32;
    fn DoorQueueUserAPCById(function: u64, id: u32, argument: u64) -> u32;
    fn RtlSetLastWin32Error(value: u32);
}

const ERROR_INVALID_HANDLE: u32 = 6;
const ERROR_INVALID_FUNCTION: u32 = 1;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_NO_MORE_FILES: u32 = 18;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_MORE_DATA: u32 = 234;
const APPMODEL_ERROR_NO_PACKAGE: u32 = 15700;
const ERROR_RESOURCE_TYPE_NOT_FOUND: u32 = 1813;

fn fail(code: u32) -> i32 {
    unsafe { RtlSetLastWin32Error(code) };
    0
}
fn put<T: Copy>(pointer: u64, value: T) {
    if pointer != 0 {
        // SAFETY: the caller's out parameter of type T.
        unsafe { (pointer as *mut T).write_unaligned(value) };
    }
}
fn get<T: Copy>(pointer: u64) -> T {
    // SAFETY: the caller's in parameter of type T.
    unsafe { (pointer as *const T).read_unaligned() }
}
/// A NUL-terminated wide string into `out` (bounded): its length.
fn wide(pointer: u64, out: &mut [u16]) -> usize {
    if pointer == 0 {
        return 0;
    }
    let mut n = 0;
    while n < out.len() {
        let u: u16 = get(pointer + 2 * n as u64);
        if u == 0 {
            break;
        }
        out[n] = u;
        n += 1;
    }
    n
}
fn put_units(pointer: u64, units: &[u16]) {
    for (i, u) in units.iter().enumerate() {
        put(pointer + 2 * i as u64, *u);
    }
}

// ---- resources -------------------------------------------------------------------

/// EnumResourceNames(Ex)W: `callback(module, type, name, param)` for each
/// name of the type, until it returns FALSE.
#[unsafe(no_mangle)]
pub extern "system" fn EnumResourceNamesW(
    module: usize,
    kind: usize,
    callback: usize,
    param: usize,
) -> i32 {
    if callback == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let module = door_windows_abi::widen(module);
    let kind = door_windows_abi::widen(kind);
    // SAFETY: the caller's ENUMRESNAMEPROCW.
    let call: extern "system" fn(usize, usize, usize, usize) -> i32 =
        unsafe { core::mem::transmute(callback) };
    let mut name = [0u16; 256];
    let mut index = 0;
    loop {
        let found = unsafe { DoorResourceName(module, kind, index, name.as_mut_ptr() as u64) };
        if found == 0 {
            return if index == 0 {
                fail(ERROR_RESOURCE_TYPE_NOT_FOUND)
            } else {
                1
            };
        }
        if call(
            door_windows_abi::narrow(module),
            door_windows_abi::narrow(kind),
            door_windows_abi::narrow(found),
            param,
        ) == 0
        {
            return 1;
        }
        index += 1;
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn EnumResourceNamesExW(
    module: usize,
    kind: usize,
    callback: usize,
    param: usize,
    _flags: u32,
    _language: u16,
) -> i32 {
    EnumResourceNamesW(module, kind, callback, param)
}

// ---- locales ---------------------------------------------------------------------

/// EnumSystemLocalesW: the installed locales' LCIDs as hex text
/// ("00000409", English (United States)).
#[unsafe(no_mangle)]
pub extern "system" fn EnumSystemLocalesW(callback: usize, _flags: u32) -> i32 {
    if callback == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: the caller's LOCALE_ENUMPROCW.
    let call: extern "system" fn(usize) -> i32 = unsafe { core::mem::transmute(callback) };
    let name: [u16; 9] = core::array::from_fn(|i| u16::from(b"00000409\0"[i]));
    call(name.as_ptr() as usize);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn IsValidLocale(lcid: u32, _flags: u32) -> i32 {
    i32::from(matches!(lcid, 0x409 | 0x7f | 0x400 | 0x800))
}
/// The UI languages a thread, a process or the user prefers: English
/// (United States), as a name ("en-US") or an LCID ("0409"), in a
/// double-NUL list.
fn ui_languages(flags: u32, count: usize, buffer: usize, size: usize) -> i32 {
    let id = flags & 0x4 != 0; // MUI_LANGUAGE_ID
    let text: &[u8] = if id { b"0409\0\0" } else { b"en-US\0\0" };
    let count = door_windows_abi::widen(count);
    let buffer = door_windows_abi::widen(buffer);
    let size = door_windows_abi::widen(size);
    if size == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    put(count, 1u32);
    let capacity: u32 = get(size);
    put(size, text.len() as u32);
    if buffer == 0 {
        return 1;
    }
    if (capacity as usize) < text.len() {
        return fail(ERROR_INSUFFICIENT_BUFFER);
    }
    for (i, b) in text.iter().enumerate() {
        put(buffer + 2 * i as u64, u16::from(*b));
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadPreferredUILanguages(
    flags: u32,
    count: usize,
    buffer: usize,
    size: usize,
) -> i32 {
    ui_languages(flags, count, buffer, size)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessPreferredUILanguages(
    flags: u32,
    count: usize,
    buffer: usize,
    size: usize,
) -> i32 {
    ui_languages(flags, count, buffer, size)
}

// ---- INI files ---------------------------------------------------------------------

/// An INI file read at most (units of text).
const INI_UNITS: usize = 16384;

/// The INI file's text as UTF-16 (UTF-16LE with a BOM, else ANSI/UTF-8):
/// its length, 0 when it cannot be read.
fn read_ini(file: &[u16], out: &mut [u16; INI_UNITS]) -> usize {
    let mut path = [0u16; 300];
    let n = ini_path(file, &mut path);
    let handle = crate::guest::CreateFileW(path.as_ptr() as usize, 0x8000_0000, 3, 0, 3, 0x80, 0);
    if handle == usize::MAX {
        let _ = n;
        return 0;
    }
    let mut bytes = [0u8; 2 * INI_UNITS];
    let mut read = 0u32;
    let ok = crate::guest::ReadFile(
        handle,
        bytes.as_mut_ptr() as usize,
        bytes.len() as u32,
        &mut read as *mut u32 as usize,
        0,
    );
    crate::guest::CloseHandle(handle);
    if ok == 0 {
        return 0;
    }
    let bytes = &bytes[..read as usize];
    if bytes.starts_with(&[0xff, 0xfe]) {
        let units = bytes[2..]
            .chunks_exact(2)
            .map(|p| u16::from_le_bytes([p[0], p[1]]));
        let mut n = 0;
        for u in units.take(out.len()) {
            out[n] = u;
            n += 1;
        }
        return n;
    }
    let text = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let mut n = 0;
    match core::str::from_utf8(text) {
        Ok(s) => {
            for u in s.encode_utf16().take(out.len()) {
                out[n] = u;
                n += 1;
            }
        }
        Err(_) => {
            for &b in text.iter().take(out.len()) {
                out[n] = u16::from(b);
                n += 1;
            }
        }
    }
    n
}
/// A bare INI file name lives in the Windows folder, as Windows maps it.
fn ini_path(file: &[u16], out: &mut [u16; 300]) -> usize {
    let bare = !file
        .iter()
        .any(|&u| u == u16::from(b'\\') || u == u16::from(b'/') || u == u16::from(b':'));
    let mut n = 0;
    if bare {
        for u in "C:\\Windows\\".encode_utf16() {
            out[n] = u;
            n += 1;
        }
    }
    for &u in file.iter().take(298 - n) {
        out[n] = u;
        n += 1;
    }
    out[n] = 0;
    n
}
fn lower(u: u16) -> u16 {
    if (u16::from(b'A')..=u16::from(b'Z')).contains(&u) {
        u + 32
    } else {
        u
    }
}
fn same(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| lower(*x) == lower(*y))
}
fn trim(mut s: &[u16]) -> &[u16] {
    while let [first, rest @ ..] = s {
        if *first == 0x20 || *first == 0x09 {
            s = rest;
        } else {
            break;
        }
    }
    while let [rest @ .., last] = s {
        if *last == 0x20 || *last == 0x09 || *last == 0x0d {
            s = rest;
        } else {
            break;
        }
    }
    s
}
/// Visit each line of the INI text as (section, key, value); a section
/// line reports key `None`.
fn ini_lines(text: &[u16], mut each: impl FnMut(&[u16], Option<(&[u16], &[u16])>) -> bool) {
    let mut section: &[u16] = &[];
    for line in text.split(|&u| u == 0x0a) {
        let line = trim(line);
        if line.is_empty() || line[0] == u16::from(b';') {
            continue;
        }
        if line[0] == u16::from(b'[') {
            if let Some(end) = line.iter().position(|&u| u == u16::from(b']')) {
                section = trim(&line[1..end]);
                if !each(section, None) {
                    return;
                }
            }
            continue;
        }
        let Some(eq) = line.iter().position(|&u| u == u16::from(b'=')) else {
            continue;
        };
        let key = trim(&line[..eq]);
        let mut value = trim(&line[eq + 1..]);
        if value.len() >= 2
            && value[0] == value[value.len() - 1]
            && (value[0] == u16::from(b'"') || value[0] == u16::from(b'\''))
        {
            value = &value[1..value.len() - 1];
        }
        if !each(section, Some((key, value))) {
            return;
        }
    }
}
/// Copy a string, or a double-NUL list (`list`), into the caller's buffer,
/// truncating as Windows does: the units copied, not counting the NUL.
fn deliver(units: &[u16], buffer: u64, size: u32, list: bool) -> u32 {
    if buffer == 0 || size == 0 {
        return 0;
    }
    let size = size as usize;
    if list {
        if units.len() < size {
            put_units(buffer, units);
            put(buffer + 2 * units.len() as u64, 0u16);
            return units.len() as u32;
        }
        if size < 2 {
            put(buffer, 0u16);
            return 0;
        }
        put_units(buffer, &units[..size - 2]);
        put(buffer + 2 * (size - 2) as u64, 0u16);
        put(buffer + 2 * (size - 1) as u64, 0u16);
        return (size - 2) as u32;
    }
    let n = units.len().min(size - 1);
    put_units(buffer, &units[..n]);
    put(buffer + 2 * n as u64, 0u16);
    n as u32
}
#[unsafe(no_mangle)]
pub extern "system" fn GetPrivateProfileStringW(
    section: usize,
    key: usize,
    default: usize,
    buffer: usize,
    size: u32,
    file: usize,
) -> u32 {
    let (section, key, default, buffer, file) = (
        door_windows_abi::widen(section),
        door_windows_abi::widen(key),
        door_windows_abi::widen(default),
        door_windows_abi::widen(buffer),
        door_windows_abi::widen(file),
    );
    let mut path = [0u16; 260];
    let pn = wide(file, &mut path);
    let mut text = [0u16; INI_UNITS];
    let tn = if pn == 0 {
        0
    } else {
        read_ini(&path[..pn], &mut text)
    };
    let (mut s, mut k) = ([0u16; 256], [0u16; 256]);
    let sn = wide(section, &mut s);
    let kn = wide(key, &mut k);
    let mut list = [0u16; INI_UNITS];
    let mut ln = 0;
    let push = |units: &[u16], list: &mut [u16; INI_UNITS], ln: &mut usize| {
        for &u in units.iter().chain([&0]) {
            if *ln < list.len() {
                list[*ln] = u;
                *ln += 1;
            }
        }
    };
    if section == 0 {
        // Every section name.
        ini_lines(&text[..tn], |name, key| {
            if key.is_none() {
                push(name, &mut list, &mut ln);
            }
            true
        });
        return deliver(&list[..ln], buffer, size, true);
    }
    if key == 0 {
        // Every key of the section.
        ini_lines(&text[..tn], |name, key| {
            if let Some((k, _)) = key
                && same(name, &s[..sn])
            {
                push(k, &mut list, &mut ln);
            }
            true
        });
        return deliver(&list[..ln], buffer, size, true);
    }
    let mut found = [0u16; 4096];
    let mut fnn = None;
    ini_lines(&text[..tn], |name, entry| {
        if let Some((k2, v)) = entry
            && same(name, &s[..sn])
            && same(k2, &k[..kn])
        {
            let n = v.len().min(found.len());
            found[..n].copy_from_slice(&v[..n]);
            fnn = Some(n);
            return false;
        }
        true
    });
    match fnn {
        Some(n) => deliver(&found[..n], buffer, size, false),
        None => {
            let mut d = [0u16; 4096];
            let dn = wide(default, &mut d);
            // Trailing blanks of the default go, as Windows trims them.
            let dv = trim(&d[..dn]);
            if pn != 0 && tn == 0 {
                unsafe { RtlSetLastWin32Error(ERROR_FILE_NOT_FOUND) };
            }
            deliver(dv, buffer, size, false)
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetPrivateProfileIntW(
    section: usize,
    key: usize,
    default: i32,
    file: usize,
) -> u32 {
    let mut text = [0u16; 32];
    let default_text = [0u16; 1];
    let n = GetPrivateProfileStringW(
        section,
        key,
        default_text.as_ptr() as usize,
        text.as_mut_ptr() as usize,
        32,
        file,
    ) as usize;
    if n == 0 {
        return default as u32;
    }
    let mut value: u32 = 0;
    let mut digits = 0;
    for &u in &text[..n] {
        match u8::try_from(u) {
            Ok(b @ b'0'..=b'9') => {
                value = value.wrapping_mul(10).wrapping_add(u32::from(b - b'0'));
                digits += 1;
            }
            _ => break,
        }
    }
    if digits == 0 { 0 } else { value }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetProfileStringW(
    section: usize,
    key: usize,
    default: usize,
    buffer: usize,
    size: u32,
) -> u32 {
    let file: [u16; 8] = core::array::from_fn(|i| u16::from(b"win.ini\0"[i]));
    GetPrivateProfileStringW(section, key, default, buffer, size, file.as_ptr() as usize)
}
/// WritePrivateProfileStringW: set (or with a NULL value, delete) a key,
/// or delete a section (NULL key); the file is rewritten as UTF-16LE.
#[unsafe(no_mangle)]
pub extern "system" fn WritePrivateProfileStringW(
    section: usize,
    key: usize,
    value: usize,
    file: usize,
) -> i32 {
    let (section, key, value, file) = (
        door_windows_abi::widen(section),
        door_windows_abi::widen(key),
        door_windows_abi::widen(value),
        door_windows_abi::widen(file),
    );
    if section == 0 || file == 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let mut path = [0u16; 260];
    let pn = wide(file, &mut path);
    let mut text = [0u16; INI_UNITS];
    let tn = read_ini(&path[..pn], &mut text);
    let (mut s, mut k, mut v) = ([0u16; 256], [0u16; 256], [0u16; 4096]);
    let sn = wide(section, &mut s);
    let kn = wide(key, &mut k);
    let vn = wide(value, &mut v);
    // Rebuild the file line by line.
    let mut out = [0u16; INI_UNITS + 4096];
    let mut on = 0;
    let emit = |units: &[u16], out: &mut [u16], on: &mut usize| {
        for &u in units {
            if *on < out.len() {
                out[*on] = u;
                *on += 1;
            }
        }
    };
    let crlf = [0x0d, 0x0a];
    let eq = [u16::from(b'=')];
    let mut in_section = false;
    let mut section_seen = false;
    let mut written = false;
    for line in text[..tn].split(|&u| u == 0x0a) {
        let t = trim(line);
        if t.first() == Some(&u16::from(b'[')) {
            if in_section && !written && key != 0 && value != 0 {
                emit(&k[..kn], &mut out, &mut on);
                emit(&eq, &mut out, &mut on);
                emit(&v[..vn], &mut out, &mut on);
                emit(&crlf, &mut out, &mut on);
                written = true;
            }
            let end = t
                .iter()
                .position(|&u| u == u16::from(b']'))
                .unwrap_or(t.len());
            in_section = same(trim(&t[1..end]), &s[..sn]);
            section_seen |= in_section;
            if in_section && key == 0 {
                continue;
            }
        } else if in_section {
            if key == 0 {
                continue;
            }
            if let Some(e) = t.iter().position(|&u| u == u16::from(b'='))
                && same(trim(&t[..e]), &k[..kn])
            {
                if value != 0 && !written {
                    emit(&k[..kn], &mut out, &mut on);
                    emit(&eq, &mut out, &mut on);
                    emit(&v[..vn], &mut out, &mut on);
                    emit(&crlf, &mut out, &mut on);
                    written = true;
                }
                continue;
            }
        }
        if !line.is_empty() || on != 0 {
            let body = line.strip_suffix(&[0x0d]).unwrap_or(line);
            if !(body.is_empty() && line.is_empty()) {
                emit(body, &mut out, &mut on);
                emit(&crlf, &mut out, &mut on);
            }
        }
    }
    if !written && key != 0 && value != 0 {
        if !section_seen {
            emit(&[u16::from(b'[')], &mut out, &mut on);
            emit(&s[..sn], &mut out, &mut on);
            emit(&[u16::from(b']')], &mut out, &mut on);
            emit(&crlf, &mut out, &mut on);
        } else if in_section {
            // The section ends the file: append to it.
        }
        emit(&k[..kn], &mut out, &mut on);
        emit(&eq, &mut out, &mut on);
        emit(&v[..vn], &mut out, &mut on);
        emit(&crlf, &mut out, &mut on);
    }
    let mut full = [0u16; 300];
    ini_path(&path[..pn], &mut full);
    let handle = crate::guest::CreateFileW(full.as_ptr() as usize, 0x4000_0000, 0, 0, 2, 0x80, 0);
    if handle == usize::MAX {
        return 0;
    }
    let mut bytes = [0u8; 2 * (INI_UNITS + 4096) + 2];
    bytes[..2].copy_from_slice(&[0xff, 0xfe]);
    for (i, u) in out[..on].iter().enumerate() {
        bytes[2 + 2 * i..4 + 2 * i].copy_from_slice(&u.to_le_bytes());
    }
    let mut done = 0u32;
    let ok = crate::guest::WriteFile(
        handle,
        bytes.as_ptr() as usize,
        (2 + 2 * on) as u32,
        &mut done as *mut u32 as usize,
        0,
    );
    crate::guest::CloseHandle(handle);
    ok
}

// ---- waitable timers -----------------------------------------------------------

const TIMERS: usize = 64;
#[derive(Clone, Copy)]
struct Timer {
    /// The event the timer signals (its handle).
    handle: u64,
    pool: u64,
    routine: u64,
    argument: u64,
    thread: u32,
    period: u32,
}
struct Timers {
    held: AtomicBool,
    list: UnsafeCell<[Option<Timer>; TIMERS]>,
}
unsafe impl Sync for Timers {}
static TIMER_TABLE: Timers = Timers {
    held: AtomicBool::new(false),
    list: UnsafeCell::new([None; TIMERS]),
};
fn with_timers<R>(f: impl FnOnce(&mut [Option<Timer>; TIMERS]) -> R) -> R {
    while TIMER_TABLE.held.swap(true, Ordering::Acquire) {
        core::hint::spin_loop();
    }
    // SAFETY: `held` gives this closure the only access.
    let r = f(unsafe { &mut *TIMER_TABLE.list.get() });
    TIMER_TABLE.held.store(false, Ordering::Release);
    r
}
/// CloseHandle of a waitable timer: its pool timer stops.
pub(crate) fn forget_timer(handle: u64) {
    let pool = with_timers(|list| {
        list.iter_mut()
            .find(|t| t.is_some_and(|t| t.handle == handle))
            .and_then(|slot| slot.take())
            .map(|t| t.pool)
    });
    if let Some(pool) = pool {
        crate::threadpool::SetThreadpoolTimer(door_windows_abi::narrow(pool), 0, 0, 0);
        crate::threadpool::CloseThreadpoolTimer(door_windows_abi::narrow(pool));
    }
}
/// The pool timer's callback: the timer is signalled, and its completion
/// routine queued to the thread that set it.
extern "system" fn timer_fired(_instance: usize, context: usize, _timer: usize) {
    let handle = context as u64;
    let timer = with_timers(|list| list.iter().flatten().find(|t| t.handle == handle).copied());
    let Some(timer) = timer else {
        return;
    };
    crate::guest::SetEvent(door_windows_abi::narrow(handle));
    if timer.routine != 0 {
        unsafe {
            DoorQueueUserAPCById(timer_apc as *const () as usize as u64, timer.thread, handle);
        }
    }
}
extern "system" fn timer_apc(context: usize) {
    let handle = context as u64;
    let timer = with_timers(|list| list.iter().flatten().find(|t| t.handle == handle).copied());
    if let Some(timer) = timer
        && timer.routine != 0
    {
        // SAFETY: the routine SetWaitableTimer was given (PTIMERAPCROUTINE).
        let routine: extern "system" fn(usize, u32, u32) =
            unsafe { core::mem::transmute(timer.routine as usize) };
        let mut now = [0u32; 2];
        crate::application::GetSystemTimeAsFileTime(now.as_mut_ptr() as usize);
        routine(timer.argument as usize, now[0], now[1]);
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateWaitableTimerExW(
    attributes: usize,
    name: usize,
    flags: u32,
    _access: u32,
) -> usize {
    // CREATE_WAITABLE_TIMER_MANUAL_RESET.
    let manual = i32::from(flags & 1 != 0);
    let handle = crate::guest::CreateEventW(attributes, manual, 0, name) as u64;
    if handle == 0 {
        return 0;
    }
    let pool = crate::threadpool::CreateThreadpoolTimer(
        timer_fired as *const () as usize,
        door_windows_abi::narrow(handle),
        0,
    ) as u64;
    if pool == 0 {
        crate::guest::CloseHandle(door_windows_abi::narrow(handle));
        return 0;
    }
    let stored = with_timers(|list| {
        list.iter_mut().find(|t| t.is_none()).map(|slot| {
            *slot = Some(Timer {
                handle,
                pool,
                routine: 0,
                argument: 0,
                thread: 0,
                period: 0,
            });
        })
    });
    if stored.is_none() {
        crate::threadpool::CloseThreadpoolTimer(door_windows_abi::narrow(pool));
        crate::guest::CloseHandle(door_windows_abi::narrow(handle));
        fail(8);
        return 0;
    }
    door_windows_abi::narrow(handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateWaitableTimerW(attributes: usize, manual: i32, name: usize) -> usize {
    CreateWaitableTimerExW(attributes, name, u32::from(manual != 0), 0x1f0003)
}
/// SetWaitableTimer: `due` (FILETIME: negative relative, else absolute),
/// then every `period` ms; the completion routine runs as an APC on the
/// calling thread.
#[unsafe(no_mangle)]
pub extern "system" fn SetWaitableTimer(
    timer: usize,
    due: usize,
    period: i32,
    routine: usize,
    argument: usize,
    _resume: i32,
) -> i32 {
    let handle = door_windows_abi::widen(timer);
    let due = door_windows_abi::widen(due);
    if due == 0 || period < 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let pool = with_timers(|list| {
        list.iter_mut()
            .flatten()
            .find(|t| t.handle == handle)
            .map(|t| {
                t.routine = door_windows_abi::widen(routine);
                t.argument = door_windows_abi::widen(argument);
                t.thread = unsafe { DoorGetCurrentThreadId() };
                t.period = period as u32;
                t.pool
            })
    });
    let Some(pool) = pool else {
        return fail(ERROR_INVALID_HANDLE);
    };
    crate::guest::ResetEvent(timer);
    crate::threadpool::SetThreadpoolTimer(
        door_windows_abi::narrow(pool),
        door_windows_abi::narrow(due),
        period as u32,
        0,
    );
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetWaitableTimerEx(
    timer: usize,
    due: usize,
    period: i32,
    routine: usize,
    argument: usize,
    _context: usize,
    _delay: u32,
) -> i32 {
    SetWaitableTimer(timer, due, period, routine, argument, 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn CancelWaitableTimer(timer: usize) -> i32 {
    let handle = door_windows_abi::widen(timer);
    let pool = with_timers(|list| {
        list.iter()
            .flatten()
            .find(|t| t.handle == handle)
            .map(|t| t.pool)
    });
    let Some(pool) = pool else {
        return fail(ERROR_INVALID_HANDLE);
    };
    crate::threadpool::SetThreadpoolTimer(door_windows_abi::narrow(pool), 0, 0, 0);
    1
}

// ---- volumes -----------------------------------------------------------------------

/// Doors' one volume, C:, by its volume GUID path.
const VOLUME: &str = "\\\\?\\Volume{d0025026-0000-0000-0000-100000000000}\\";
fn put_text(buffer: u64, capacity: u32, text: &str, list: bool) -> Result<u32, u32> {
    let extra = if list { 2 } else { 1 };
    let n = text.encode_utf16().count();
    if (capacity as usize) < n + extra {
        return Err(n as u32 + extra as u32);
    }
    for (i, u) in text.encode_utf16().enumerate() {
        put(buffer + 2 * i as u64, u);
    }
    put(buffer + 2 * n as u64, 0u16);
    if list {
        put(buffer + 2 * (n + 1) as u64, 0u16);
    }
    Ok(n as u32 + extra as u32)
}
/// A volume-enumeration handle: one volume, then the end.
static VOLUME_FIND_DONE: AtomicBool = AtomicBool::new(false);
const VOLUME_FIND_HANDLE: usize = 0x766f_6c00;
#[unsafe(no_mangle)]
pub extern "system" fn FindFirstVolumeW(buffer: usize, capacity: u32) -> usize {
    match put_text(door_windows_abi::widen(buffer), capacity, VOLUME, false) {
        Ok(_) => {
            VOLUME_FIND_DONE.store(true, Ordering::Relaxed);
            VOLUME_FIND_HANDLE
        }
        Err(_) => {
            fail(ERROR_INSUFFICIENT_BUFFER);
            usize::MAX
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn FindNextVolumeW(handle: usize, _buffer: usize, _capacity: u32) -> i32 {
    if handle != VOLUME_FIND_HANDLE {
        return fail(ERROR_INVALID_HANDLE);
    }
    fail(ERROR_NO_MORE_FILES)
}
#[unsafe(no_mangle)]
pub extern "system" fn FindVolumeClose(handle: usize) -> i32 {
    i32::from(handle == VOLUME_FIND_HANDLE)
}
fn is_c_root(pointer: u64) -> bool {
    let mut units = [0u16; 8];
    let n = wide(pointer, &mut units);
    let text: [u8; 8] = core::array::from_fn(|i| units[i] as u8);
    matches!(&text[..n], b"C:\\" | b"c:\\" | b"C:" | b"c:")
}
#[unsafe(no_mangle)]
pub extern "system" fn GetVolumeNameForVolumeMountPointW(
    mount: usize,
    buffer: usize,
    capacity: u32,
) -> i32 {
    if !is_c_root(door_windows_abi::widen(mount)) {
        return fail(ERROR_INVALID_PARAMETER);
    }
    match put_text(door_windows_abi::widen(buffer), capacity, VOLUME, false) {
        Ok(_) => 1,
        Err(_) => fail(ERROR_INSUFFICIENT_BUFFER),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetVolumePathNamesForVolumeNameW(
    volume: usize,
    buffer: usize,
    capacity: u32,
    length: usize,
) -> i32 {
    let mut units = [0u16; 64];
    let n = wide(door_windows_abi::widen(volume), &mut units);
    let wanted: [u16; 64] = {
        let mut w = [0u16; 64];
        for (slot, u) in w.iter_mut().zip(VOLUME.encode_utf16()) {
            *slot = u;
        }
        w
    };
    if !same(&units[..n], &wanted[..VOLUME.len()]) {
        return fail(ERROR_INVALID_PARAMETER);
    }
    match put_text(door_windows_abi::widen(buffer), capacity, "C:\\", true) {
        Ok(n) => {
            put(door_windows_abi::widen(length), n);
            1
        }
        Err(n) => {
            put(door_windows_abi::widen(length), n);
            fail(ERROR_MORE_DATA)
        }
    }
}
/// QueryDosDeviceW: C: is the first hard-disk volume; NULL lists the DOS
/// device names.
#[unsafe(no_mangle)]
pub extern "system" fn QueryDosDeviceW(name: usize, buffer: usize, capacity: u32) -> u32 {
    let name = door_windows_abi::widen(name);
    let buffer = door_windows_abi::widen(buffer);
    let (text, list) = if name == 0 {
        ("C:", true)
    } else if is_c_root(name) {
        ("\\Device\\HarddiskVolume1", true)
    } else {
        return fail(ERROR_FILE_NOT_FOUND) as u32;
    };
    match put_text(buffer, capacity, text, list) {
        Ok(n) => n,
        Err(_) => fail(ERROR_INSUFFICIENT_BUFFER) as u32,
    }
}

// ---- serial ports ----------------------------------------------------------------

/// Doors has no serial ports: no handle is a communications device.
fn not_a_port(handle: usize) -> i32 {
    if handle == 0 || handle == usize::MAX {
        fail(ERROR_INVALID_HANDLE)
    } else {
        fail(ERROR_INVALID_FUNCTION)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ClearCommError(handle: usize, _errors: usize, _status: usize) -> i32 {
    not_a_port(handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn EscapeCommFunction(handle: usize, _function: u32) -> i32 {
    not_a_port(handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCommModemStatus(handle: usize, _status: usize) -> i32 {
    not_a_port(handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCommState(handle: usize, _dcb: usize) -> i32 {
    not_a_port(handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetCommState(handle: usize, _dcb: usize) -> i32 {
    not_a_port(handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCommTimeouts(handle: usize, _timeouts: usize) -> i32 {
    not_a_port(handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetCommTimeouts(handle: usize, _timeouts: usize) -> i32 {
    not_a_port(handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn PurgeComm(handle: usize, _flags: u32) -> i32 {
    not_a_port(handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetupComm(handle: usize, _input: u32, _output: u32) -> i32 {
    not_a_port(handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn SetCommMask(handle: usize, _mask: u32) -> i32 {
    not_a_port(handle)
}
#[unsafe(no_mangle)]
pub extern "system" fn WaitCommEvent(handle: usize, _mask: usize, _overlapped: usize) -> i32 {
    not_a_port(handle)
}

// ---- packages ---------------------------------------------------------------------

/// No program runs packaged yet (MSIX is M5): no process has a package
/// identity and no package is installed.
#[unsafe(no_mangle)]
pub extern "system" fn GetPackageFamilyName(_process: usize, _length: usize, _name: usize) -> u32 {
    APPMODEL_ERROR_NO_PACKAGE
}
#[unsafe(no_mangle)]
pub extern "system" fn GetPackagePathByFullName(_name: usize, _length: usize, _path: usize) -> u32 {
    ERROR_NOT_FOUND_PACKAGE
}
const ERROR_NOT_FOUND_PACKAGE: u32 = 1168;
#[unsafe(no_mangle)]
pub extern "system" fn GetPackagesByPackageFamily(
    _family: usize,
    count: usize,
    _names: usize,
    length: usize,
    _buffer: usize,
) -> u32 {
    put(door_windows_abi::widen(count), 0u32);
    put(door_windows_abi::widen(length), 0u32);
    0
}

// ---- files and pipes --------------------------------------------------------------

/// ReOpenFile: another handle to the same file with new access, as a
/// duplicate of the original (Doors keeps one access per open file).
#[unsafe(no_mangle)]
pub extern "system" fn ReOpenFile(handle: usize, _access: u32, _share: u32, _flags: u32) -> usize {
    let mut copy = 0usize;
    let process = usize::MAX;
    if crate::pipes::DuplicateHandle(
        process,
        handle,
        process,
        &mut copy as *mut usize as usize,
        0,
        0,
        2, // DUPLICATE_SAME_ACCESS
    ) == 0
    {
        return usize::MAX;
    }
    copy
}
/// TransactNamedPipe: one message out, the reply in.
#[unsafe(no_mangle)]
pub extern "system" fn TransactNamedPipe(
    pipe: usize,
    input: usize,
    input_size: u32,
    output: usize,
    output_size: u32,
    read: usize,
    overlapped: usize,
) -> i32 {
    if overlapped != 0 {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let mut written = 0u32;
    if crate::guest::WriteFile(
        pipe,
        input,
        input_size,
        &mut written as *mut u32 as usize,
        0,
    ) == 0
    {
        return 0;
    }
    crate::guest::ReadFile(pipe, output, output_size, read, 0)
}

// ---- change notifications ---------------------------------------------------------

/// ReadDirectoryChangesW: Doors' storage does not report changes to a
/// folder yet, as a file system without change notifications answers
/// (ERROR_INVALID_FUNCTION); watchers fall back to polling.
#[unsafe(no_mangle)]
pub extern "system" fn ReadDirectoryChangesW(
    _directory: usize,
    _buffer: usize,
    _length: u32,
    _subtree: i32,
    _filter: u32,
    _returned: usize,
    _overlapped: usize,
    _routine: usize,
) -> i32 {
    fail(ERROR_INVALID_FUNCTION)
}
