//! The rest of user32's surface Chromium and Electron call (P18 M7): the
//! display (one monitor: the session's screen and work area), window
//! properties, enumeration and lookup, placement, layered windows, the
//! clipboard (process-local, as `User::clipboard` is), hooks and WinEvents
//! (registered; Doors raises none), keyboard, mouse and pointer queries,
//! raw input (no devices registered), power and device notifications,
//! window stations and desktops (the interactive WinSta0\Default), and
//! assorted helpers.
use super::{hwnd, hwnd_or_error, tid, with};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering};
use door_windows_abi::{narrow, widen};
use door_windows_gui::handles::Handle;
use door_windows_gui::msg::{swp, ws, ws_ex};
use door_windows_gui::{Rect, metrics, win32_error as e};
use door_windows_gui_guest as g;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorGetCurrentProcessId() -> u32;
    fn RtlAllocateHeap(heap: usize, flags: u32, bytes: usize) -> usize;
    fn RtlSizeHeap(heap: usize, flags: u32, pointer: usize) -> usize;
    fn DoorGetProcessHeap() -> u64;
}
#[link(name = "gdi32", kind = "dylib")]
unsafe extern "system" {
    fn BitBlt(
        dc: usize,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        source: usize,
        sx: i32,
        sy: i32,
        rop: u32,
    ) -> i32;
    fn DeleteObject(object: usize) -> i32;
}

const POINTER: usize = core::mem::size_of::<usize>();
const ERROR_INVALID_MONITOR_HANDLE: u32 = 1461;
const ERROR_NOACCESS: u32 = 998;
const ERROR_NOT_FOUND: u32 = 1168;
/// The one display's HMONITOR.
pub(crate) const MONITOR: u64 = 0x0001_0001;
/// GetDesktopWindow's value (no Doors window has it).
const DESKTOP: u64 = 0x0001_0010;
/// The window station and desktop pseudo handles.
const STATION: u64 = 0x0000_0FF0;
const DESKTOP_OBJECT: u64 = 0x0000_0FF4;

fn fail<T: Default>(code: u32) -> T {
    g::set_last_error(code);
    T::default()
}
fn read_wide_arg(pointer: u64) -> Option<Vec<u16>> {
    // A string argument, or an atom (MAKEINTATOM: the high bits are zero).
    if pointer >> 16 == 0 {
        None
    } else {
        g::read_wide(pointer, 1024)
    }
}
fn upper(units: &[u16]) -> Vec<u16> {
    units
        .iter()
        .map(|u| {
            if (u16::from(b'a')..=u16::from(b'z')).contains(u) {
                u - 32
            } else {
                *u
            }
        })
        .collect()
}
/// A process-local lock-protected table.
struct Store<T>(g::Lock<Vec<T>>);
impl<T> Store<T> {
    const fn new() -> Self {
        Self(g::Lock::new(Vec::new()))
    }
}

// ---- the display -------------------------------------------------------------------------
fn screen() -> (Rect, Rect) {
    with(|u| (Rect::new(0, 0, u.screen.0, u.screen.1), u.work))
}
fn rect_of(pointer: u64) -> Option<Rect> {
    g::read_i32s::<4>(pointer).map(|[l, t, r, b]| Rect::new(l, t, r, b))
}
/// MONITOR_DEFAULTTONULL (0) gives no monitor for a point or window off
/// the display; the other flags give the display.
fn monitor_or_null(on_display: bool, flags: u32) -> usize {
    narrow(if on_display || flags != 0 { MONITOR } else { 0 })
}
#[unsafe(no_mangle)]
pub extern "system" fn MonitorFromWindow(hwnd_value: usize, flags: u32) -> usize {
    let v = widen(hwnd_value);
    let on = v == DESKTOP
        || hwnd(v).is_some_and(|h| {
            let r = with(|u| u.windows.screen_rect(h));
            !r.intersect(screen().0).is_empty()
        });
    monitor_or_null(on, flags)
}
/// MonitorFromPoint(POINT by value).
#[unsafe(no_mangle)]
pub extern "system" fn MonitorFromPoint(point: u64, flags: u32) -> usize {
    let (x, y) = (point as u32 as i32, (point >> 32) as u32 as i32);
    let s = screen().0;
    monitor_or_null(
        x >= s.left && x < s.right && y >= s.top && y < s.bottom,
        flags,
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn MonitorFromRect(rect: usize, flags: u32) -> usize {
    let on = rect_of(widen(rect)).is_some_and(|r| !r.intersect(screen().0).is_empty());
    monitor_or_null(on, flags)
}
const DEVICE: &str = "\\\\.\\DISPLAY1";
/// GetMonitorInfo: MONITORINFO (40 bytes) or MONITORINFOEX (+ the
/// device name: 32 WCHARs or 32 chars).
fn monitor_info(monitor: usize, out: usize, wide: bool) -> i32 {
    let (monitor, out) = (widen(monitor), widen(out));
    if monitor != MONITOR {
        return fail(ERROR_INVALID_MONITOR_HANDLE);
    }
    let Some(size) = g::read_u32(out) else {
        return fail(ERROR_NOACCESS);
    };
    let ex = if wide { 104 } else { 72 };
    if size != 40 && size != ex {
        return fail(e::INVALID_PARAMETER);
    }
    let (s, w) = screen();
    let mut b = alloc::vec![0u8; size as usize];
    b[..4].copy_from_slice(&size.to_le_bytes());
    for (i, v) in [
        s.left, s.top, s.right, s.bottom, w.left, w.top, w.right, w.bottom,
    ]
    .iter()
    .enumerate()
    {
        b[4 + 4 * i..8 + 4 * i].copy_from_slice(&v.to_le_bytes());
    }
    // MONITORINFOF_PRIMARY.
    b[36..40].copy_from_slice(&1u32.to_le_bytes());
    if size == ex {
        for (i, c) in DEVICE.bytes().enumerate() {
            if wide {
                b[40 + 2 * i] = c;
            } else {
                b[40 + i] = c;
            }
        }
    }
    i32::from(g::write(out, &b))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetMonitorInfoW(monitor: usize, out: usize) -> i32 {
    monitor_info(monitor, out, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetMonitorInfoA(monitor: usize, out: usize) -> i32 {
    monitor_info(monitor, out, false)
}
type MonitorProc = extern "system" fn(usize, usize, usize, isize) -> i32;
/// EnumDisplayMonitors: the one monitor (clipped by `clip` when given).
#[unsafe(no_mangle)]
pub extern "system" fn EnumDisplayMonitors(
    dc: usize,
    clip: usize,
    procedure: usize,
    data: isize,
) -> i32 {
    if procedure == 0 {
        return fail(e::INVALID_PARAMETER);
    }
    let mut r = screen().0;
    if let Some(c) = rect_of(widen(clip)) {
        r = r.intersect(c);
        if r.is_empty() {
            return 1;
        }
    }
    let rect = [r.left, r.top, r.right, r.bottom];
    // SAFETY: the application's MONITORENUMPROC, called with no lock held.
    let f: MonitorProc = unsafe { core::mem::transmute(procedure) };
    f(narrow(MONITOR), dc, rect.as_ptr() as usize, data);
    1
}
/// EnumDisplaySettings(Ex)W: the current mode (DEVMODEW, 220 bytes).
fn display_settings(device: usize, mode: u32, out: usize) -> i32 {
    let out = widen(out);
    if let Some(name) = read_wide_arg(widen(device))
        && upper(&name) != DEVICE.encode_utf16().collect::<Vec<_>>()
    {
        return 0;
    }
    // ENUM_CURRENT_SETTINGS, ENUM_REGISTRY_SETTINGS, or mode 0.
    if !matches!(mode, 0 | 0xFFFF_FFFF | 0xFFFF_FFFE) {
        return 0;
    }
    let (s, _) = screen();
    let mut b = [0u8; 220];
    for (i, c) in DEVICE.encode_utf16().enumerate().skip(4) {
        b[2 * (i - 4)..2 * (i - 4) + 2].copy_from_slice(&c.to_le_bytes());
    }
    b[64..66].copy_from_slice(&0x0401u16.to_le_bytes());
    b[68..70].copy_from_slice(&220u16.to_le_bytes());
    // DM_POSITION | DM_DISPLAYORIENTATION | DM_BITSPERPEL | DM_PELSWIDTH
    // | DM_PELSHEIGHT | DM_DISPLAYFLAGS | DM_DISPLAYFREQUENCY.
    b[72..76].copy_from_slice(&0x005C_00A0u32.to_le_bytes());
    b[168..172].copy_from_slice(&32u32.to_le_bytes());
    b[172..176].copy_from_slice(&(s.width() as u32).to_le_bytes());
    b[176..180].copy_from_slice(&(s.height() as u32).to_le_bytes());
    b[184..188].copy_from_slice(&60u32.to_le_bytes());
    // Callers set dmSize and may pass a shorter DEVMODE; write what fits.
    let size = g::read_u32(out + 68)
        .map_or(220, |v| (v & 0xFFFF) as usize)
        .clamp(96, 220);
    i32::from(g::write(out, &b[..size]))
}
#[unsafe(no_mangle)]
pub extern "system" fn EnumDisplaySettingsW(device: usize, mode: u32, out: usize) -> i32 {
    display_settings(device, mode, out)
}
#[unsafe(no_mangle)]
pub extern "system" fn EnumDisplaySettingsExW(
    device: usize,
    mode: u32,
    out: usize,
    _flags: u32,
) -> i32 {
    display_settings(device, mode, out)
}
/// EnumDisplayDevicesW: the adapter `\\.\DISPLAY1`, or (for it) its
/// monitor (DISPLAY_DEVICEW, 840 bytes).
#[unsafe(no_mangle)]
pub extern "system" fn EnumDisplayDevicesW(
    device: usize,
    index: u32,
    out: usize,
    _flags: u32,
) -> i32 {
    let out = widen(out);
    if index != 0 {
        return 0;
    }
    let monitor = read_wide_arg(widen(device)).is_some();
    let (name, description, state, id) = if monitor {
        (
            "\\\\.\\DISPLAY1\\Monitor0",
            "Generic PnP Monitor",
            0x3u32,
            "MONITOR\\Default_Monitor",
        )
    } else {
        (
            DEVICE,
            "Microsoft Basic Display Adapter",
            0x5u32,
            "PCI\\VEN_1234&DEV_1111",
        )
    };
    let mut b = [0u8; 840];
    let put = |b: &mut [u8; 840], at: usize, text: &str| {
        for (i, c) in text.encode_utf16().enumerate() {
            b[at + 2 * i..at + 2 * i + 2].copy_from_slice(&c.to_le_bytes());
        }
    };
    b[..4].copy_from_slice(&840u32.to_le_bytes());
    put(&mut b, 4, name);
    put(&mut b, 68, description);
    b[324..328].copy_from_slice(&state.to_le_bytes());
    put(&mut b, 328, id);
    i32::from(g::write(out, &b))
}
/// The DisplayConfig API: not offered (callers use EnumDisplayDevices and
/// the monitor API).
#[unsafe(no_mangle)]
pub extern "system" fn GetDisplayConfigBufferSizes(_flags: u32, paths: usize, modes: usize) -> i32 {
    g::write_u32(widen(paths), 0);
    g::write_u32(widen(modes), 0);
    e::NOT_SUPPORTED as i32
}
#[unsafe(no_mangle)]
pub extern "system" fn QueryDisplayConfig(
    _flags: u32,
    _paths: usize,
    _path_info: usize,
    _modes: usize,
    _mode_info: usize,
    _topology: usize,
) -> i32 {
    e::NOT_SUPPORTED as i32
}
#[unsafe(no_mangle)]
pub extern "system" fn DisplayConfigGetDeviceInfo(_packet: usize) -> i32 {
    e::NOT_SUPPORTED as i32
}

// ---- window properties ----------------------------------------------------------------------
/// A property name: an atom or (upper-cased) text.
#[derive(PartialEq, Eq)]
enum Key {
    Atom(u16),
    Name(Vec<u16>),
}
fn key(pointer: u64) -> Option<Key> {
    if pointer >> 16 == 0 {
        (pointer != 0).then_some(Key::Atom(pointer as u16))
    } else {
        g::read_wide(pointer, 256).map(|n| Key::Name(upper(&n)))
    }
}
static PROPS: Store<(Handle, Key, u64)> = Store::new();
#[unsafe(no_mangle)]
pub extern "system" fn SetPropW(hwnd_value: usize, name: usize, data: usize) -> i32 {
    let (Some(h), Some(k)) = (hwnd_or_error(widen(hwnd_value)), key(widen(name))) else {
        return 0;
    };
    let mut props = PROPS.0.lock();
    match props.iter_mut().find(|(w, n, _)| *w == h && *n == k) {
        Some(p) => p.2 = widen(data),
        None => props.push((h, k, widen(data))),
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetPropW(hwnd_value: usize, name: usize) -> usize {
    let (Some(h), Some(k)) = (hwnd(widen(hwnd_value)), key(widen(name))) else {
        return 0;
    };
    narrow(
        PROPS
            .0
            .lock()
            .iter()
            .find(|(w, n, _)| *w == h && *n == k)
            .map_or(0, |p| p.2),
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn RemovePropW(hwnd_value: usize, name: usize) -> usize {
    let (Some(h), Some(k)) = (hwnd(widen(hwnd_value)), key(widen(name))) else {
        return 0;
    };
    let mut props = PROPS.0.lock();
    match props.iter().position(|(w, n, _)| *w == h && *n == k) {
        Some(i) => narrow(props.remove(i).2),
        None => 0,
    }
}
/// A window's properties go with it.
pub(crate) fn window_destroyed(h: Handle) {
    PROPS.0.lock().retain(|(w, _, _)| *w != h);
    LAYERED.0.lock().retain(|(w, _)| *w != h);
}

// ---- finding and enumerating windows ---------------------------------------------------------
type EnumProc = extern "system" fn(usize, isize) -> i32;
/// EnumWindows: the top-level windows, front to back.
#[unsafe(no_mangle)]
pub extern "system" fn EnumWindows(procedure: usize, data: isize) -> i32 {
    if procedure == 0 {
        return fail(e::INVALID_PARAMETER);
    }
    let windows: Vec<u64> = with(|u| u.windows.top.iter().map(|h| h.value()).collect());
    // SAFETY: the application's WNDENUMPROC, called with no lock held.
    let f: EnumProc = unsafe { core::mem::transmute(procedure) };
    for w in windows {
        if hwnd(w).is_some() && f(narrow(w), data) == 0 {
            break;
        }
    }
    1
}
/// Whether window `h` has class `class` (a name or an atom) and title.
fn matches(h: Handle, class: u64, title: u64) -> bool {
    let wanted_class = read_wide_arg(class).map(|c| upper(&c));
    let wanted_title = if title == 0 {
        None
    } else {
        g::read_wide(title, 1024)
    };
    with(|u| {
        let Some(w) = u.windows.get(h) else {
            return false;
        };
        let class_ok = match (&wanted_class, class) {
            (_, 0) => true,
            (Some(name), _) => u
                .classes
                .by_atom(w.class)
                .and_then(|i| u.classes.get(i))
                .is_some_and(|c| upper(&c.name) == *name),
            (None, atom) => u64::from(w.class) == atom,
        };
        class_ok && wanted_title.as_ref().is_none_or(|t| *t == w.text)
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn FindWindowExW(
    parent: usize,
    after: usize,
    class: usize,
    title: usize,
) -> usize {
    let (parent, after) = (widen(parent), widen(after));
    let candidates: Vec<Handle> = with(|u| match u.windows.resolve(parent) {
        Some(p) if parent != DESKTOP => u
            .windows
            .get(p)
            .map(|w| w.children.clone())
            .unwrap_or_default(),
        _ => u.windows.top.clone(),
    });
    let start = match hwnd(after) {
        Some(a) => candidates
            .iter()
            .position(|c| *c == a)
            .map_or(candidates.len(), |i| i + 1),
        None => 0,
    };
    for &c in &candidates[start.min(candidates.len())..] {
        if matches(c, widen(class), widen(title)) {
            return narrow(c.value());
        }
    }
    fail(e::FILE_NOT_FOUND)
}
#[unsafe(no_mangle)]
pub extern "system" fn FindWindowW(class: usize, title: usize) -> usize {
    FindWindowExW(0, 0, class, title)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetDesktopWindow() -> usize {
    narrow(DESKTOP)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetWindowThreadProcessId(hwnd_value: usize, process: usize) -> u32 {
    let Some(h) = hwnd_or_error(widen(hwnd_value)) else {
        return 0;
    };
    let thread = with(|u| u.windows.get(h).map_or(0, |w| w.thread));
    if process != 0 {
        g::write_u32(widen(process), unsafe { DoorGetCurrentProcessId() });
    }
    thread
}
/// WindowFromPoint(POINT by value): the deepest visible window there.
#[unsafe(no_mangle)]
pub extern "system" fn WindowFromPoint(point: u64) -> usize {
    let p = door_windows_gui::Point::new(point as u32 as i32, (point >> 32) as u32 as i32);
    narrow(with(|u| {
        let mut found = None;
        for &t in &u.windows.top {
            if u.windows.really_visible(t) && u.windows.screen_rect(t).contains(p) {
                found = Some(t);
                break;
            }
        }
        let mut current = found;
        while let Some(c) = current {
            let next = u.windows.get(c).and_then(|w| {
                w.children
                    .iter()
                    .copied()
                    .find(|&k| u.windows.really_visible(k) && u.windows.screen_rect(k).contains(p))
            });
            match next {
                Some(n) => current = Some(n),
                None => break,
            }
        }
        current.map_or(0, |h| h.value())
    }))
}
#[unsafe(no_mangle)]
pub extern "system" fn WindowFromDC(_dc: usize) -> usize {
    0
}

// ---- window state ---------------------------------------------------------------------------
fn style(hwnd_value: usize) -> u32 {
    hwnd(widen(hwnd_value)).map_or(0, |h| with(|u| u.windows.get(h).map_or(0, |w| w.style)))
}
#[unsafe(no_mangle)]
pub extern "system" fn IsIconic(hwnd_value: usize) -> i32 {
    i32::from(style(hwnd_value) & ws::MINIMIZE != 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn IsZoomed(hwnd_value: usize) -> i32 {
    i32::from(style(hwnd_value) & ws::MAXIMIZE != 0)
}
/// GetWindowPlacement (WINDOWPLACEMENT, 44 bytes): the window's rectangle
/// and show state.
#[unsafe(no_mangle)]
pub extern "system" fn GetWindowPlacement(hwnd_value: usize, out: usize) -> i32 {
    let Some(h) = hwnd_or_error(widen(hwnd_value)) else {
        return 0;
    };
    let (r, s) = with(|u| {
        (
            u.windows.screen_rect(h),
            u.windows.get(h).map_or(0, |w| w.style),
        )
    });
    let show = if s & ws::MINIMIZE != 0 {
        2
    } else if s & ws::MAXIMIZE != 0 {
        3
    } else {
        1
    };
    i32::from(g::write_i32s(
        widen(out),
        &[
            44, 0, show, -1, -1, -1, -1, r.left, r.top, r.right, r.bottom,
        ],
    ))
}
#[unsafe(no_mangle)]
pub extern "system" fn SetWindowPlacement(hwnd_value: usize, placement: usize) -> i32 {
    let Some(h) = hwnd_or_error(widen(hwnd_value)) else {
        return 0;
    };
    let Some(p) = g::read_i32s::<11>(widen(placement)) else {
        return fail(ERROR_NOACCESS);
    };
    super::wnd::set_window_pos(
        h,
        p[7],
        p[8],
        p[9] - p[7],
        p[10] - p[8],
        swp::NOZORDER | swp::NOACTIVATE,
    );
    super::wnd::show(h, p[2]);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn BringWindowToTop(hwnd_value: usize) -> i32 {
    let Some(h) = hwnd_or_error(widen(hwnd_value)) else {
        return 0;
    };
    i32::from(super::wnd::set_window_pos(
        h,
        0,
        0,
        0,
        0,
        swp::NOMOVE | swp::NOSIZE,
    ))
}
/// SetParent: the child moves under `parent` (or to the top level).
#[unsafe(no_mangle)]
pub extern "system" fn SetParent(child: usize, parent: usize) -> usize {
    let Some(c) = hwnd_or_error(widen(child)) else {
        return 0;
    };
    let new = widen(parent);
    let p = if new == 0 || new == DESKTOP {
        None
    } else {
        hwnd(new)
    };
    if new != 0 && new != DESKTOP && p.is_none() {
        return fail(e::INVALID_WINDOW_HANDLE);
    }
    narrow(with(|u| {
        let old = u.windows.get(c).and_then(|w| w.parent);
        if let Some(o) = old
            && let Some(w) = u.windows.get_mut(o)
        {
            w.children.retain(|k| *k != c);
        }
        u.windows.top.retain(|k| *k != c);
        match p {
            Some(p) => {
                if let Some(w) = u.windows.get_mut(p) {
                    w.children.insert(0, c);
                }
            }
            None => u.windows.top.insert(0, c),
        }
        if let Some(w) = u.windows.get_mut(c) {
            w.parent = p;
        }
        old.map_or(DESKTOP, |o| o.value())
    }))
}
/// No system menu is drawn by Doors' caption buttons.
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemMenu(_hwnd: usize, _revert: i32) -> usize {
    0
}
/// SetWindowRgn: the system owns the region (deleted here); windows stay
/// rectangular.
#[unsafe(no_mangle)]
pub extern "system" fn SetWindowRgn(hwnd_value: usize, region: usize, _redraw: i32) -> i32 {
    if hwnd_or_error(widen(hwnd_value)).is_none() {
        return 0;
    }
    if region != 0 {
        unsafe { DeleteObject(region) };
    }
    1
}
/// GetWindowRgn: ERROR (no region).
#[unsafe(no_mangle)]
pub extern "system" fn GetWindowRgn(_hwnd: usize, _region: usize) -> i32 {
    0
}
static LAYERED: Store<(Handle, (u32, u8, u32))> = Store::new();
#[unsafe(no_mangle)]
pub extern "system" fn SetLayeredWindowAttributes(
    hwnd_value: usize,
    key: u32,
    alpha: u8,
    flags: u32,
) -> i32 {
    let Some(h) = hwnd_or_error(widen(hwnd_value)) else {
        return 0;
    };
    let mut l = LAYERED.0.lock();
    l.retain(|(w, _)| *w != h);
    l.push((h, (key, alpha, flags)));
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetLayeredWindowAttributes(
    hwnd_value: usize,
    key: usize,
    alpha: usize,
    flags: usize,
) -> i32 {
    let Some(h) = hwnd_or_error(widen(hwnd_value)) else {
        return 0;
    };
    let Some((k, a, f)) = LAYERED
        .0
        .lock()
        .iter()
        .find(|(w, _)| *w == h)
        .map(|(_, v)| *v)
    else {
        return 0;
    };
    g::write_u32(widen(key), k);
    if alpha != 0 {
        g::write(widen(alpha), &[a]);
    }
    g::write_u32(widen(flags), f);
    1
}
/// UpdateLayeredWindow: the window moves and resizes to `position` and
/// `size`, then shows `source` (from `source_point`) as its content.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn UpdateLayeredWindow(
    hwnd_value: usize,
    _destination: usize,
    position: usize,
    size: usize,
    source: usize,
    source_point: usize,
    _key: u32,
    _blend: usize,
    _flags: u32,
) -> i32 {
    let Some(h) = hwnd_or_error(widen(hwnd_value)) else {
        return 0;
    };
    let at = g::read_i32s::<2>(widen(position));
    let extent = g::read_i32s::<2>(widen(size));
    let r = with(|u| u.windows.screen_rect(h));
    let (x, y) = at.map_or((r.left, r.top), |[x, y]| (x, y));
    let (w, hh) = extent.map_or((r.width(), r.height()), |[w, hh]| (w, hh));
    let mut flags = swp::NOZORDER | swp::NOACTIVATE;
    if at.is_none() {
        flags |= swp::NOMOVE;
    }
    if extent.is_none() {
        flags |= swp::NOSIZE;
    }
    super::wnd::set_window_pos(h, x, y, w, hh, flags);
    if source != 0 {
        let [sx, sy] = g::read_i32s::<2>(widen(source_point)).unwrap_or([0, 0]);
        let dc = super::paint::GetDC(hwnd_value);
        if dc != 0 {
            // SRCCOPY.
            unsafe { BitBlt(dc, 0, 0, w, hh, source, sx, sy, 0x00CC_0020) };
            super::paint::ReleaseDC(hwnd_value, dc);
        }
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn UpdateLayeredWindowIndirect(hwnd_value: usize, info: usize) -> i32 {
    // UPDATELAYEREDWINDOWINFO { cbSize, hdcDst, pptDst, psize, hdcSrc,
    // pptSrc, crKey, pblend, dwFlags, prcDirty }.
    let info = widen(info);
    let p = |i: usize| g::read_pointer(info + (POINTER * i) as u64).unwrap_or(0);
    UpdateLayeredWindow(
        hwnd_value,
        narrow(p(1)),
        narrow(p(2)),
        narrow(p(3)),
        narrow(p(4)),
        narrow(p(5)),
        0,
        narrow(p(7)),
        0,
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn PrintWindow(_hwnd: usize, _dc: usize, _flags: u32) -> i32 {
    0
}
static AFFINITY: AtomicU32 = AtomicU32::new(0);
#[unsafe(no_mangle)]
pub extern "system" fn SetWindowDisplayAffinity(hwnd_value: usize, affinity: u32) -> i32 {
    if hwnd_or_error(widen(hwnd_value)).is_none() {
        return 0;
    }
    AFFINITY.store(affinity, Ordering::Relaxed);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetWindowDisplayAffinity(hwnd_value: usize, affinity: usize) -> i32 {
    if hwnd_or_error(widen(hwnd_value)).is_none() {
        return 0;
    }
    i32::from(g::write_u32(
        widen(affinity),
        AFFINITY.load(Ordering::Relaxed),
    ))
}
#[unsafe(no_mangle)]
pub extern "system" fn IsWindowArranged(_hwnd: usize) -> i32 {
    0
}
/// AdjustWindowRect(Ex)(ForDpi): the window rectangle for a client one,
/// with Doors' non-client insets.
fn adjust(rect: usize, style: u32, menu: i32, ex_style: u32) -> i32 {
    let rect = widen(rect);
    let Some(r) = rect_of(rect) else {
        return fail(ERROR_NOACCESS);
    };
    let i = metrics::insets(style, ex_style, menu != 0, style & ws::CHILD != 0);
    i32::from(g::write_i32s(
        rect,
        &[
            r.left - i.left,
            r.top - i.top,
            r.right + i.right,
            r.bottom + i.bottom,
        ],
    ))
}
#[unsafe(no_mangle)]
pub extern "system" fn AdjustWindowRectEx(
    rect: usize,
    style: u32,
    menu: i32,
    ex_style: u32,
) -> i32 {
    adjust(rect, style, menu, ex_style)
}
#[unsafe(no_mangle)]
pub extern "system" fn AdjustWindowRect(rect: usize, style: u32, menu: i32) -> i32 {
    adjust(rect, style, menu, 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn AdjustWindowRectExForDpi(
    rect: usize,
    style: u32,
    menu: i32,
    ex_style: u32,
    _dpi: u32,
) -> i32 {
    adjust(rect, style, menu, ex_style)
}
#[unsafe(no_mangle)]
pub extern "system" fn AllowSetForegroundWindow(_process: u32) -> i32 {
    1
}
/// ChangeWindowMessageFilterEx: every message is allowed (one integrity
/// level per session); CHANGEFILTERSTRUCT.ExtStatus is MSGFLTINFO_NONE.
#[unsafe(no_mangle)]
pub extern "system" fn ChangeWindowMessageFilterEx(
    _hwnd: usize,
    _message: u32,
    _action: u32,
    status: usize,
) -> i32 {
    let status = widen(status);
    if status != 0 {
        g::write_u32(status + 4, 0);
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn ChangeWindowMessageFilter(_message: u32, _flag: u32) -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetProcessDPIAware() -> i32 {
    super::dpi::declare_system_aware();
    1
}

// ---- registered messages and clipboard formats ---------------------------------------------
/// RegisterWindowMessage and RegisterClipboardFormat share Windows' atom
/// range 0xC000-0xFFFF (case-insensitive names, process-local here).
static REGISTERED: Store<Vec<u16>> = Store::new();
fn register_name(name: Vec<u16>) -> u32 {
    if name.is_empty() {
        return fail(e::INVALID_PARAMETER);
    }
    let key = upper(&name);
    let mut table = REGISTERED.0.lock();
    let index = match table.iter().position(|n| upper(n) == key) {
        Some(i) => i,
        None => {
            if table.len() >= 0x3FFF {
                return fail(e::NOT_ENOUGH_MEMORY);
            }
            table.push(name);
            table.len() - 1
        }
    };
    0xC000 + index as u32
}
fn registered_name(id: u32) -> Option<Vec<u16>> {
    let i = id.checked_sub(0xC000)? as usize;
    REGISTERED.0.lock().get(i).cloned()
}
#[unsafe(no_mangle)]
pub extern "system" fn RegisterWindowMessageW(name: usize) -> u32 {
    g::read_wide(widen(name), 255).map_or_else(|| fail(e::INVALID_PARAMETER), register_name)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegisterWindowMessageA(name: usize) -> u32 {
    g::read_bytes(widen(name), 255)
        .map(|b| {
            b.iter()
                .take_while(|c| **c != 0)
                .map(|c| u16::from(*c))
                .collect()
        })
        .map_or_else(|| fail(e::INVALID_PARAMETER), register_name)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegisterClipboardFormatW(name: usize) -> u32 {
    RegisterWindowMessageW(name)
}

// ---- the clipboard ------------------------------------------------------------------------
/// The clipboard's formats and their bytes; CF_UNICODETEXT mirrors
/// `User::clipboard`, which the edit controls use.
struct Clipboard {
    owner: Option<u32>,
    formats: Vec<(u32, Vec<u8>)>,
    listeners: Vec<u64>,
}
static CLIPBOARD: g::Lock<Clipboard> = g::Lock::new(Clipboard {
    owner: None,
    formats: Vec::new(),
    listeners: Vec::new(),
});
static SEQUENCE: AtomicU32 = AtomicU32::new(1);
const CF_TEXT: u32 = 1;
const CF_UNICODETEXT: u32 = 13;
const WM_CLIPBOARDUPDATE: u32 = 0x031D;
fn changed() {
    SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let listeners = CLIPBOARD.lock().listeners.clone();
    for l in listeners {
        let _ = super::msg::post(l, WM_CLIPBOARDUPDATE, 0, 0);
    }
}
/// The edit controls' text joins the formats (it may have changed since).
fn sync_text(c: &mut Clipboard) {
    let text = with(|u| u.clipboard.clone());
    c.formats
        .retain(|(f, _)| *f != CF_UNICODETEXT && *f != CF_TEXT);
    if !text.is_empty() {
        let mut bytes: Vec<u8> = text.iter().flat_map(|u| u.to_le_bytes()).collect();
        bytes.extend([0, 0]);
        c.formats.push((CF_UNICODETEXT, bytes));
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenClipboard(_owner: usize) -> i32 {
    let mut c = CLIPBOARD.lock();
    match c.owner {
        Some(t) if t != tid() => fail(e::ACCESS_DENIED),
        _ => {
            c.owner = Some(tid());
            1
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CloseClipboard() -> i32 {
    let mut c = CLIPBOARD.lock();
    if c.owner != Some(tid()) {
        return fail(e::CLIPBOARD_NOT_OPEN);
    }
    c.owner = None;
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn EmptyClipboard() -> i32 {
    {
        let mut c = CLIPBOARD.lock();
        if c.owner != Some(tid()) {
            return fail(e::CLIPBOARD_NOT_OPEN);
        }
        c.formats.clear();
    }
    with(|u| u.clipboard.clear());
    changed();
    1
}
/// SetClipboardData: the HGLOBAL's bytes (a process heap block) are copied
/// in; the system owns the handle afterwards.
#[unsafe(no_mangle)]
pub extern "system" fn SetClipboardData(format: u32, data: usize) -> usize {
    if CLIPBOARD.lock().owner != Some(tid()) {
        return fail(e::CLIPBOARD_NOT_OPEN);
    }
    let block = widen(data);
    let bytes = if block == 0 {
        Vec::new()
    } else {
        let size = unsafe { RtlSizeHeap(narrow(DoorGetProcessHeap()), 0, data) };
        if size == usize::MAX {
            return fail(e::INVALID_HANDLE);
        }
        g::read_bytes(block, size).unwrap_or_default()
    };
    if format == CF_UNICODETEXT {
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|u| *u != 0)
            .collect();
        with(|u| u.clipboard = units);
    }
    {
        let mut c = CLIPBOARD.lock();
        c.formats.retain(|(f, _)| *f != format);
        c.formats.push((format, bytes));
    }
    changed();
    data
}
/// GetClipboardData: a process heap copy (GlobalLock returns its address).
#[unsafe(no_mangle)]
pub extern "system" fn GetClipboardData(format: u32) -> usize {
    let mut c = CLIPBOARD.lock();
    if c.owner != Some(tid()) {
        return fail(e::CLIPBOARD_NOT_OPEN);
    }
    sync_text(&mut c);
    let bytes = match (format, c.formats.iter().find(|(f, _)| *f == format)) {
        (_, Some((_, b))) => b.clone(),
        // CF_TEXT from the Unicode text (ASCII; others become '?').
        (CF_TEXT, None) => match c.formats.iter().find(|(f, _)| *f == CF_UNICODETEXT) {
            Some((_, b)) => b
                .chunks_exact(2)
                .map(|p| u16::from_le_bytes([p[0], p[1]]))
                .map(|u| if u < 0x80 { u as u8 } else { b'?' })
                .collect(),
            None => return fail(e::CLIPBOARD_NOT_OPEN),
        },
        _ => return fail(ERROR_NOT_FOUND),
    };
    drop(c);
    let block = unsafe { RtlAllocateHeap(narrow(DoorGetProcessHeap()), 0, bytes.len().max(1)) };
    if block != 0 {
        g::write(widen(block), &bytes);
    }
    block
}
#[unsafe(no_mangle)]
pub extern "system" fn IsClipboardFormatAvailable(format: u32) -> i32 {
    let mut c = CLIPBOARD.lock();
    sync_text(&mut c);
    let unicode = c.formats.iter().any(|(f, _)| *f == CF_UNICODETEXT);
    i32::from(c.formats.iter().any(|(f, _)| *f == format) || (format == CF_TEXT && unicode))
}
#[unsafe(no_mangle)]
pub extern "system" fn EnumClipboardFormats(previous: u32) -> u32 {
    let mut c = CLIPBOARD.lock();
    if c.owner != Some(tid()) {
        return fail(e::CLIPBOARD_NOT_OPEN);
    }
    sync_text(&mut c);
    let formats: Vec<u32> = c.formats.iter().map(|(f, _)| *f).collect();
    let next = if previous == 0 {
        formats.first().copied()
    } else {
        formats
            .iter()
            .position(|f| *f == previous)
            .and_then(|i| formats.get(i + 1).copied())
    };
    g::set_last_error(0);
    next.unwrap_or(0)
}
#[unsafe(no_mangle)]
pub extern "system" fn CountClipboardFormats() -> i32 {
    let mut c = CLIPBOARD.lock();
    sync_text(&mut c);
    c.formats.len() as i32
}
#[unsafe(no_mangle)]
pub extern "system" fn GetClipboardFormatNameW(format: u32, out: usize, capacity: i32) -> i32 {
    let Some(name) = registered_name(format) else {
        return fail(e::INVALID_PARAMETER);
    };
    g::write_wide(widen(out), capacity.max(0) as usize, &name).map_or(0, |n| n as i32)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetClipboardSequenceNumber() -> u32 {
    SEQUENCE.load(Ordering::Relaxed)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetClipboardOwner() -> usize {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn AddClipboardFormatListener(hwnd_value: usize) -> i32 {
    if hwnd_or_error(widen(hwnd_value)).is_none() {
        return 0;
    }
    let mut c = CLIPBOARD.lock();
    if !c.listeners.contains(&widen(hwnd_value)) {
        c.listeners.push(widen(hwnd_value));
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn RemoveClipboardFormatListener(hwnd_value: usize) -> i32 {
    let mut c = CLIPBOARD.lock();
    let before = c.listeners.len();
    c.listeners.retain(|l| *l != widen(hwnd_value));
    i32::from(c.listeners.len() < before)
}

// ---- hooks and WinEvents --------------------------------------------------------------------
/// Hooks and WinEvent hooks are registered and removed; Doors' input and
/// window events do not run them.
static NEXT_HOOK: AtomicU64 = AtomicU64::new(0x0003_0000);
static HOOKS: Store<u64> = Store::new();
fn new_hook() -> usize {
    let h = NEXT_HOOK.fetch_add(4, Ordering::Relaxed);
    HOOKS.0.lock().push(h);
    narrow(h)
}
fn remove_hook(h: usize) -> i32 {
    let mut hooks = HOOKS.0.lock();
    let before = hooks.len();
    hooks.retain(|x| *x != widen(h));
    if hooks.len() < before {
        1
    } else {
        drop(hooks);
        fail(e::INVALID_HOOK_HANDLE)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetWindowsHookExW(
    kind: i32,
    procedure: usize,
    _module: usize,
    _thread: u32,
) -> usize {
    if procedure == 0 || !(-1..=14).contains(&kind) {
        return fail(e::INVALID_PARAMETER);
    }
    new_hook()
}
#[unsafe(no_mangle)]
pub extern "system" fn SetWindowsHookExA(
    kind: i32,
    procedure: usize,
    module: usize,
    thread: u32,
) -> usize {
    SetWindowsHookExW(kind, procedure, module, thread)
}
#[unsafe(no_mangle)]
pub extern "system" fn UnhookWindowsHookEx(hook: usize) -> i32 {
    remove_hook(hook)
}
#[unsafe(no_mangle)]
pub extern "system" fn CallNextHookEx(
    _hook: usize,
    _code: i32,
    _wparam: usize,
    _lparam: isize,
) -> isize {
    0
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn SetWinEventHook(
    _min: u32,
    _max: u32,
    _module: usize,
    procedure: usize,
    _process: u32,
    _thread: u32,
    _flags: u32,
) -> usize {
    if procedure == 0 {
        return fail(e::INVALID_PARAMETER);
    }
    new_hook()
}
#[unsafe(no_mangle)]
pub extern "system" fn UnhookWinEvent(hook: usize) -> i32 {
    remove_hook(hook)
}
#[unsafe(no_mangle)]
pub extern "system" fn NotifyWinEvent(_event: u32, _hwnd: usize, _object: i32, _child: i32) {}

// ---- keyboard, mouse and pointer --------------------------------------------------------------
#[unsafe(no_mangle)]
pub extern "system" fn GetKeyboardState(out: usize) -> i32 {
    let mut state = [0u8; 256];
    for (vk, slot) in state.iter_mut().enumerate() {
        let s = super::misc::GetKeyState(vk as i32) as u16;
        *slot = (if s & 0x8000 != 0 { 0x80 } else { 0 }) | (s & 1) as u8;
    }
    i32::from(g::write(widen(out), &state))
}
/// SetKeyboardState: the thread's key state is the device's (accepted).
#[unsafe(no_mangle)]
pub extern "system" fn SetKeyboardState(state: usize) -> i32 {
    i32::from(state != 0)
}
/// SendInput: synthesized input is refused (no input injection path);
/// UIPI's answer.
#[unsafe(no_mangle)]
pub extern "system" fn SendInput(_count: u32, _inputs: usize, _size: i32) -> u32 {
    fail(e::ACCESS_DENIED)
}
/// TrackMouseEvent: TME_LEAVE requests WM_MOUSELEAVE when the pointer
/// moves to another window; TME_HOVER is recorded; TME_QUERY reports.
static TRACKED: AtomicU64 = AtomicU64::new(0);
static TRACK_FLAGS: AtomicU32 = AtomicU32::new(0);
#[unsafe(no_mangle)]
pub extern "system" fn TrackMouseEvent(event: usize) -> i32 {
    let event = widen(event);
    let (Some(size), Some(flags)) = (g::read_u32(event), g::read_u32(event + 4)) else {
        return fail(ERROR_NOACCESS);
    };
    if size as usize != 16 + POINTER {
        return fail(e::INVALID_PARAMETER);
    }
    let target = g::read_pointer(event + 8).unwrap_or(0);
    const TME_QUERY: u32 = 0x4000_0000;
    const TME_CANCEL: u32 = 0x8000_0000;
    if flags & TME_QUERY != 0 {
        g::write_u32(event + 4, TRACK_FLAGS.load(Ordering::Relaxed));
        g::write_pointer(event + 8, TRACKED.load(Ordering::Relaxed));
        return 1;
    }
    if hwnd_or_error(target).is_none() {
        return 0;
    }
    if flags & TME_CANCEL != 0 {
        if TRACKED.load(Ordering::Relaxed) == target {
            TRACK_FLAGS.fetch_and(!flags, Ordering::Relaxed);
        }
    } else {
        TRACKED.store(target, Ordering::Relaxed);
        TRACK_FLAGS.store(flags & 0x13, Ordering::Relaxed);
    }
    1
}
/// The pointer is now over `h`: a tracked window it left gets
/// WM_MOUSELEAVE (or WM_NCMOUSELEAVE for TME_NONCLIENT).
pub(crate) fn pointer_over(h: Handle) {
    let tracked = TRACKED.load(Ordering::Relaxed);
    let flags = TRACK_FLAGS.load(Ordering::Relaxed);
    if tracked == 0 || tracked == h.value() || flags & 2 == 0 {
        return;
    }
    TRACKED.store(0, Ordering::Relaxed);
    TRACK_FLAGS.store(0, Ordering::Relaxed);
    let message = if flags & 0x10 != 0 { 0x02A2 } else { 0x02A3 };
    let _ = super::msg::post(tracked, message, 0, 0);
}
/// GetCursorInfo (CURSORINFO: cbSize, flags, hCursor, ptScreenPos).
#[unsafe(no_mangle)]
pub extern "system" fn GetCursorInfo(info: usize) -> i32 {
    let info = widen(info);
    // CURSORINFO: 24 bytes on x64, 20 on x86.
    let expected = if POINTER == 8 { 24 } else { 20 };
    if g::read_u32(info) != Some(expected) {
        return fail(e::INVALID_PARAMETER);
    }
    let p = with(|u| u.last_click.3);
    let showing = u32::from(CURSOR_COUNT.load(Ordering::Relaxed) >= 0);
    let cursor = super::misc::current_cursor();
    g::write_u32(info + 4, showing);
    g::write_pointer(info + 8, cursor);
    i32::from(g::write_i32s(info + 8 + POINTER as u64, &[p.x, p.y]))
}
static CURSOR_COUNT: AtomicI32 = AtomicI32::new(0);
#[unsafe(no_mangle)]
pub extern "system" fn ShowCursor(show: i32) -> i32 {
    if show != 0 {
        CURSOR_COUNT.fetch_add(1, Ordering::Relaxed) + 1
    } else {
        CURSOR_COUNT.fetch_sub(1, Ordering::Relaxed) - 1
    }
}
/// SetCursorPos / ClipCursor: the pointer is the host's; the request is
/// accepted without moving it.
#[unsafe(no_mangle)]
pub extern "system" fn SetCursorPos(_x: i32, _y: i32) -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn ClipCursor(_rect: usize) -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetClipCursor(rect: usize) -> i32 {
    let s = screen().0;
    i32::from(g::write_i32s(
        widen(rect),
        &[s.left, s.top, s.right, s.bottom],
    ))
}
/// GetLastInputInfo: input is seen as current (Doors has no idle timer
/// for programs).
#[unsafe(no_mangle)]
pub extern "system" fn GetLastInputInfo(info: usize) -> i32 {
    let info = widen(info);
    if g::read_u32(info) != Some(8) {
        return fail(e::INVALID_PARAMETER);
    }
    i32::from(g::write_u32(info + 4, (g::now() / 10_000) as u32))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetMessagePos() -> u32 {
    let p = with(|u| u.last_click.3);
    (p.x as u16 as u32) | ((p.y as u16 as u32) << 16)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetMessageExtraInfo() -> isize {
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCaretBlinkTime() -> u32 {
    530
}
#[unsafe(no_mangle)]
pub extern "system" fn GetKeyboardLayoutNameW(out: usize) -> i32 {
    i32::from(
        g::write_wide(
            widen(out),
            9,
            &"00000409".encode_utf16().collect::<Vec<_>>(),
        )
        .is_some(),
    )
}
/// Raw input: registration succeeds, but Doors routes no WM_INPUT.
#[unsafe(no_mangle)]
pub extern "system" fn RegisterRawInputDevices(devices: usize, count: u32, _size: u32) -> i32 {
    i32::from(devices != 0 && count != 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetRawInputData(
    _input: usize,
    _command: u32,
    _data: usize,
    _size: usize,
    _header: u32,
) -> u32 {
    fail::<u32>(e::INVALID_HANDLE).wrapping_sub(1)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetRawInputDeviceList(_list: usize, count: usize, _size: u32) -> u32 {
    g::write_u32(widen(count), 0);
    0
}
#[unsafe(no_mangle)]
pub extern "system" fn GetRawInputDeviceInfoW(
    _device: usize,
    _command: u32,
    _data: usize,
    _size: usize,
) -> u32 {
    fail::<u32>(e::INVALID_HANDLE).wrapping_sub(1)
}
#[unsafe(no_mangle)]
pub extern "system" fn DefRawInputProc(_inputs: usize, _count: i32, _header: u32) -> isize {
    0
}
/// Pointer input: the mouse (PT_MOUSE) and no pen or touch devices.
#[unsafe(no_mangle)]
pub extern "system" fn GetPointerType(_pointer: u32, kind: usize) -> i32 {
    i32::from(g::write_u32(widen(kind), 4))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetPointerDevices(count: usize, _devices: usize) -> i32 {
    i32::from(g::write_u32(widen(count), 0))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetPointerDevice(_device: usize, _info: usize) -> i32 {
    fail(e::INVALID_PARAMETER)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetPointerPenInfo(_pointer: u32, _info: usize) -> i32 {
    fail(e::INVALID_PARAMETER)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegisterPointerDeviceNotifications(_hwnd: usize, _range: i32) -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn CloseTouchInputHandle(_input: usize) -> i32 {
    1
}

// ---- notifications and hot keys -----------------------------------------------------------------
static NEXT_NOTIFY: AtomicU64 = AtomicU64::new(0x0004_0000);
fn notification() -> usize {
    narrow(NEXT_NOTIFY.fetch_add(4, Ordering::Relaxed))
}
#[unsafe(no_mangle)]
pub extern "system" fn RegisterPowerSettingNotification(
    _recipient: usize,
    _setting: usize,
    _flags: u32,
) -> usize {
    notification()
}
#[unsafe(no_mangle)]
pub extern "system" fn UnregisterPowerSettingNotification(handle: usize) -> i32 {
    i32::from(handle != 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegisterSuspendResumeNotification(_recipient: usize, _flags: u32) -> usize {
    notification()
}
#[unsafe(no_mangle)]
pub extern "system" fn UnregisterSuspendResumeNotification(handle: usize) -> i32 {
    i32::from(handle != 0)
}
#[unsafe(no_mangle)]
pub extern "system" fn RegisterDeviceNotificationW(
    _recipient: usize,
    _filter: usize,
    _flags: u32,
) -> usize {
    notification()
}
#[unsafe(no_mangle)]
pub extern "system" fn UnregisterDeviceNotification(handle: usize) -> i32 {
    i32::from(handle != 0)
}
/// Global hot keys are Doors' shell's; registration is accepted and
/// WM_HOTKEY is not delivered.
#[unsafe(no_mangle)]
pub extern "system" fn RegisterHotKey(_hwnd: usize, _id: i32, _modifiers: u32, _key: u32) -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn UnregisterHotKey(_hwnd: usize, _id: i32) -> i32 {
    1
}

// ---- window stations and desktops ----------------------------------------------------------
#[unsafe(no_mangle)]
pub extern "system" fn GetProcessWindowStation() -> usize {
    narrow(STATION)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadDesktop(_thread: u32) -> usize {
    narrow(DESKTOP_OBJECT)
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenInputDesktop(_flags: u32, _inherit: i32, _access: u32) -> usize {
    narrow(DESKTOP_OBJECT)
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenDesktopW(
    _name: usize,
    _flags: u32,
    _inherit: i32,
    _access: u32,
) -> usize {
    narrow(DESKTOP_OBJECT)
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateDesktopW(
    _name: usize,
    _device: usize,
    _mode: usize,
    _flags: u32,
    _access: u32,
    _security: usize,
) -> usize {
    narrow(DESKTOP_OBJECT)
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateWindowStationW(
    _name: usize,
    _flags: u32,
    _access: u32,
    _security: usize,
) -> usize {
    narrow(STATION)
}
#[unsafe(no_mangle)]
pub extern "system" fn CloseDesktop(_desktop: usize) -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn CloseWindowStation(_station: usize) -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetProcessWindowStation(_station: usize) -> i32 {
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SetThreadDesktop(_desktop: usize) -> i32 {
    1
}
/// GetUserObjectInformationW: UOI_FLAGS (visible), UOI_NAME (WinSta0 or
/// Default), UOI_TYPE, UOI_IO (the input desktop).
#[unsafe(no_mangle)]
pub extern "system" fn GetUserObjectInformationW(
    object: usize,
    index: i32,
    out: usize,
    size: u32,
    needed: usize,
) -> i32 {
    let station = widen(object) == STATION;
    let text = |s: &str| -> Vec<u8> {
        s.encode_utf16()
            .chain([0])
            .flat_map(|u| u.to_le_bytes())
            .collect()
    };
    let value: Vec<u8> = match index {
        1 => {
            let mut b = alloc::vec![0u8; 12];
            b[8..12].copy_from_slice(&1u32.to_le_bytes());
            b
        }
        2 => text(if station { "WinSta0" } else { "Default" }),
        3 => text(if station { "WindowStation" } else { "Desktop" }),
        6 => 1u32.to_le_bytes().to_vec(),
        _ => return fail(e::INVALID_PARAMETER),
    };
    g::write_u32(widen(needed), value.len() as u32);
    if (size as usize) < value.len() {
        return fail(e::INSUFFICIENT_BUFFER);
    }
    i32::from(g::write(widen(out), &value))
}

// ---- text and drawing helpers --------------------------------------------------------------------
fn upper_unit(u: u16) -> u16 {
    char::from_u32(u32::from(u))
        .and_then(|c| {
            let mut up = c.to_uppercase();
            let first = up.next()?;
            (up.next().is_none() && (first as u32) <= 0xFFFF).then_some(first as u16)
        })
        .unwrap_or(u)
}
/// CharUpperW: one character (the high word zero) or a string in place.
#[unsafe(no_mangle)]
pub extern "system" fn CharUpperW(text: usize) -> usize {
    let t = widen(text);
    if t >> 16 == 0 {
        return narrow(u64::from(upper_unit(t as u16)));
    }
    let Some(units) = g::read_wide(t, 1 << 20) else {
        return text;
    };
    let up: Vec<u16> = units.iter().map(|u| upper_unit(*u)).collect();
    g::write_wide(t, up.len() + 1, &up);
    text
}
#[unsafe(no_mangle)]
pub extern "system" fn CharUpperA(text: usize) -> usize {
    let t = widen(text);
    if t >> 16 == 0 {
        return narrow(u64::from((t as u8).to_ascii_uppercase()));
    }
    if let Some(bytes) = g::read_bytes(t, 1 << 16) {
        let n = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        let up: Vec<u8> = bytes[..n].iter().map(u8::to_ascii_uppercase).collect();
        g::write(t, &up);
    }
    text
}
fn fill(dc: usize, r: Rect, color: i32) {
    let brush = unsafe { super::DoorGdiSysColorBrush(color) };
    let rect = [r.left, r.top, r.right, r.bottom];
    unsafe { super::DoorGdiFillRect(widen(dc), rect.as_ptr() as u64, brush) };
}
/// DrawEdge: the 3-D border of `rect` (BDR_RAISED/SUNKEN outer and inner
/// lines), BF_ADJUST shrinking `rect` to its interior.
#[unsafe(no_mangle)]
pub extern "system" fn DrawEdge(dc: usize, rect: usize, edge: u32, flags: u32) -> i32 {
    let Some(mut r) = rect_of(widen(rect)) else {
        return 0;
    };
    // COLOR_3DHILIGHT 20, COLOR_3DSHADOW 16, COLOR_3DLIGHT 22, COLOR_3DDKSHADOW 21.
    let raised_outer = edge & 0x1 != 0;
    let lines = [
        (
            edge & 0x3 != 0,
            if raised_outer { (22, 21) } else { (16, 20) },
        ),
        (
            edge & 0xC != 0,
            if edge & 0x4 != 0 { (20, 16) } else { (21, 22) },
        ),
    ];
    for (present, (light, dark)) in lines {
        if !present || r.is_empty() {
            continue;
        }
        if flags & 0x1 != 0 {
            fill(dc, Rect::new(r.left, r.top, r.left + 1, r.bottom), light);
        }
        if flags & 0x2 != 0 {
            fill(dc, Rect::new(r.left, r.top, r.right, r.top + 1), light);
        }
        if flags & 0x4 != 0 {
            fill(dc, Rect::new(r.right - 1, r.top, r.right, r.bottom), dark);
        }
        if flags & 0x8 != 0 {
            fill(dc, Rect::new(r.left, r.bottom - 1, r.right, r.bottom), dark);
        }
        r = Rect::new(r.left + 1, r.top + 1, r.right - 1, r.bottom - 1);
    }
    // BF_MIDDLE fills the interior.
    if flags & 0x800 != 0 {
        fill(dc, r, 15);
    }
    if flags & 0x2000 != 0 {
        g::write_i32s(widen(rect), &[r.left, r.top, r.right, r.bottom]);
    }
    1
}
/// DrawFocusRect: a 1-pixel inverted frame.
#[unsafe(no_mangle)]
pub extern "system" fn DrawFocusRect(dc: usize, rect: usize) -> i32 {
    let Some(r) = rect_of(widen(rect)) else {
        return 0;
    };
    for part in [
        Rect::new(r.left, r.top, r.right, r.top + 1),
        Rect::new(r.left, r.bottom - 1, r.right, r.bottom),
        Rect::new(r.left, r.top + 1, r.left + 1, r.bottom - 1),
        Rect::new(r.right - 1, r.top + 1, r.right, r.bottom - 1),
    ] {
        invert(dc, part);
    }
    1
}
fn invert(dc: usize, r: Rect) {
    // DSTINVERT.
    unsafe {
        BitBlt(
            dc,
            r.left,
            r.top,
            r.width(),
            r.height(),
            0,
            0,
            0,
            0x0055_0009,
        )
    };
}
#[unsafe(no_mangle)]
pub extern "system" fn InvertRect(dc: usize, rect: usize) -> i32 {
    match rect_of(widen(rect)) {
        Some(r) => {
            invert(dc, r);
            1
        }
        None => 0,
    }
}
/// DrawFrameControl: buttons, captions and scroll arrows as raised faces.
#[unsafe(no_mangle)]
pub extern "system" fn DrawFrameControl(dc: usize, rect: usize, _kind: u32, state: u32) -> i32 {
    let Some(r) = rect_of(widen(rect)) else {
        return 0;
    };
    fill(dc, r, 15);
    // DFCS_PUSHED (0x200) draws sunken.
    let mut edge = [r.left, r.top, r.right, r.bottom];
    let style = if state & 0x200 != 0 { 0xA } else { 0x5 };
    DrawEdge(dc, edge.as_mut_ptr() as usize, style, 0xF);
    1
}

// ---- classes and windows (ANSI forms) --------------------------------------------------------
fn ansi_to_wide(pointer: u64) -> Option<Vec<u16>> {
    if pointer >> 16 == 0 {
        return None;
    }
    let bytes = g::read_bytes(pointer, 1024)?;
    let n = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let mut w: Vec<u16> = bytes[..n].iter().map(|b| u16::from(*b)).collect();
    w.push(0);
    Some(w)
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn CreateWindowExA(
    ex_style: u32,
    class: usize,
    name: usize,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    parent: usize,
    menu: usize,
    instance: usize,
    param: usize,
) -> usize {
    let class_w = ansi_to_wide(widen(class));
    let name_w = ansi_to_wide(widen(name));
    super::wnd::CreateWindowExW(
        ex_style,
        class_w.as_ref().map_or(class, |c| c.as_ptr() as usize),
        name_w.as_ref().map_or(name, |n| n.as_ptr() as usize),
        style,
        x,
        y,
        width,
        height,
        parent,
        menu,
        instance,
        param,
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn UnregisterClassA(class: usize, instance: usize) -> i32 {
    let w = ansi_to_wide(widen(class));
    super::wnd::UnregisterClassW(w.as_ref().map_or(class, |c| c.as_ptr() as usize), instance)
}
/// GetClassInfoW: WNDCLASSW from GetClassInfoExW's WNDCLASSEXW (which
/// adds cbSize before and hIconSm after the same fields).
#[unsafe(no_mangle)]
pub extern "system" fn GetClassInfoW(instance: usize, class: usize, out: usize) -> i32 {
    let ex_size = if POINTER == 8 { 80 } else { 48 };
    let mut ex = [0u8; 80];
    ex[..4].copy_from_slice(&(ex_size as u32).to_le_bytes());
    if super::wnd::GetClassInfoExW(instance, class, ex.as_mut_ptr() as usize) == 0 {
        return 0;
    }
    // WNDCLASSEXW is { cbSize, style, lpfnWndProc, ... hIconSm };
    // WNDCLASSW is { style, lpfnWndProc, ... }. On x86 the shared fields
    // are contiguous; on x64 lpfnWndProc sits at 8 in both.
    let mut w = [0u8; 72];
    let bytes = if POINTER == 8 {
        w[..4].copy_from_slice(&ex[4..8]);
        w[8..72].copy_from_slice(&ex[8..72]);
        &w[..72]
    } else {
        w[..40].copy_from_slice(&ex[4..44]);
        &w[..40]
    };
    i32::from(g::write(widen(out), bytes))
}

// ---- menus ---------------------------------------------------------------------------------------
#[unsafe(no_mangle)]
pub extern "system" fn SetMenuDefaultItem(_menu: usize, _item: u32, _by_position: u32) -> i32 {
    1
}
/// TrackPopupMenu: TrackPopupMenuEx without exclusion rectangle.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn TrackPopupMenu(
    menu: usize,
    flags: u32,
    x: i32,
    y: i32,
    _reserved: i32,
    hwnd_value: usize,
    _rect: usize,
) -> i32 {
    super::menu::track_popup(menu, flags, x, y, hwnd_value)
}
/// GetMenuItemInfoW: MENUITEMINFOW's state, ID, submenu and text.
#[unsafe(no_mangle)]
pub extern "system" fn GetMenuItemInfoW(
    menu: usize,
    item: u32,
    by_position: i32,
    info: usize,
) -> i32 {
    super::menu::item_info(menu, item, by_position != 0, info)
}

const _: () = {
    let _ = ws_ex::LAYERED;
};
