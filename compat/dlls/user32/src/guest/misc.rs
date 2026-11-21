//! Rectangle helpers, keyboard mapping and state, cursors, system metrics
//! and colors, and resource counts.
use super::{DoorGdiObjectCount, DoorGdiSysColorBrush, g, server, tid, with};
use door_windows_gui::keyboard;
use door_windows_gui::msg::idc;
use door_windows_gui::{Point, Rect, metrics, win32_error as e};

fn rect_in(p: u64) -> Option<Rect> {
    g::read_i32s::<4>(p).map(|[l, t, r, b]| Rect::new(l, t, r, b))
}
fn rect_out(p: u64, r: Rect) -> i32 {
    i32::from(g::write_i32s(p, &[r.left, r.top, r.right, r.bottom]))
}

#[unsafe(no_mangle)]
pub extern "system" fn SetRect(rect: usize, left: i32, top: i32, right: i32, bottom: i32) -> i32 {
    let rect = door_windows_abi::widen(rect);
    rect_out(rect, Rect::new(left, top, right, bottom))
}
#[unsafe(no_mangle)]
pub extern "system" fn SetRectEmpty(rect: usize) -> i32 {
    let rect = door_windows_abi::widen(rect);
    rect_out(rect, Rect::EMPTY)
}
#[unsafe(no_mangle)]
pub extern "system" fn CopyRect(dst: usize, src: usize) -> i32 {
    let dst = door_windows_abi::widen(dst);
    let src = door_windows_abi::widen(src);
    rect_in(src).map_or(0, |r| rect_out(dst, r))
}
#[unsafe(no_mangle)]
pub extern "system" fn InflateRect(rect: usize, dx: i32, dy: i32) -> i32 {
    let rect = door_windows_abi::widen(rect);
    rect_in(rect).map_or(0, |r| rect_out(rect, r.inflate(dx, dy)))
}
#[unsafe(no_mangle)]
pub extern "system" fn OffsetRect(rect: usize, dx: i32, dy: i32) -> i32 {
    let rect = door_windows_abi::widen(rect);
    rect_in(rect).map_or(0, |r| rect_out(rect, r.offset(dx, dy)))
}
#[unsafe(no_mangle)]
pub extern "system" fn IntersectRect(dst: usize, a: usize, b: usize) -> i32 {
    let dst = door_windows_abi::widen(dst);
    let a = door_windows_abi::widen(a);
    let b = door_windows_abi::widen(b);
    {
        let (Some(a), Some(b)) = (rect_in(a), rect_in(b)) else {
            return 0;
        };
        let r = a.intersect(b);
        rect_out(dst, r);
        i32::from(!r.is_empty())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn UnionRect(dst: usize, a: usize, b: usize) -> i32 {
    let dst = door_windows_abi::widen(dst);
    let a = door_windows_abi::widen(a);
    let b = door_windows_abi::widen(b);
    {
        let (Some(a), Some(b)) = (rect_in(a), rect_in(b)) else {
            return 0;
        };
        let r = a.union(b);
        rect_out(dst, r);
        i32::from(!r.is_empty())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn IsRectEmpty(rect: usize) -> i32 {
    let rect = door_windows_abi::widen(rect);
    rect_in(rect).map_or(1, |r| i32::from(r.is_empty()))
}
#[unsafe(no_mangle)]
pub extern "system" fn EqualRect(a: usize, b: usize) -> i32 {
    let a = door_windows_abi::widen(a);
    let b = door_windows_abi::widen(b);
    {
        match (rect_in(a), rect_in(b)) {
            (Some(a), Some(b)) => i32::from(a == b),
            _ => 0,
        }
    }
}
/// PtInRect: the POINT arrives by value in one 64-bit register.
#[unsafe(no_mangle)]
pub extern "system" fn PtInRect(rect: usize, point: usize) -> i32 {
    let rect = door_windows_abi::widen(rect);
    let point = door_windows_abi::widen(point);
    {
        let p = Point::new(point as u32 as i32, (point >> 32) as u32 as i32);
        rect_in(rect).map_or(0, |r| i32::from(r.contains(p)))
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn MapVirtualKeyW(code: u32, kind: u32) -> u32 {
    match kind {
        0 => u32::from(keyboard::vk_to_scan(code as u8)),
        1 => u32::from(keyboard::scan_to_vk(code as u16 & 0xff)),
        2 => keyboard::to_char(code as u8, false, false, false, false).map_or(0, |c| {
            let c = u32::from(c);
            if (u32::from(b'a')..=u32::from(b'z')).contains(&c) {
                c - 32
            } else {
                c
            }
        }),
        3 => {
            let ext = code & 0xff00 == 0xe000;
            u32::from(keyboard::scan_to_vk_ex(
                (code & 0xff) as u16 | if ext { 0x100 } else { 0 },
            ))
        }
        4 => {
            let scan = u32::from(keyboard::vk_to_scan(code as u8));
            if scan != 0 && keyboard::is_extended(code as u8) {
                0xe000 | scan
            } else {
                scan
            }
        }
        _ => {
            g::set_last_error(e::INVALID_PARAMETER);
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetKeyState(key: i32) -> i16 {
    with(|u| u.queues.key_state(tid()).get(key as u8))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetAsyncKeyState(key: i32) -> i16 {
    with(|u| u.async_keys.get(key as u8)) & i16::MIN
}

/// Handle values of the system cursors (kind CURSOR, index = IDC id).
pub(crate) fn cursor_value(id: u16) -> u64 {
    (6u64 << 24) | (1 << 16) | u64::from(id - 32_000)
}
#[unsafe(no_mangle)]
pub extern "system" fn LoadCursorW(instance: usize, name: usize) -> usize {
    let instance = door_windows_abi::widen(instance);
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 {
        if instance == 0 && (u64::from(idc::ARROW)..=32_650).contains(&name) {
            return cursor_value(name as u16);
        }
        if instance == 0 {
            g::set_last_error(e::RESOURCE_NAME_NOT_FOUND);
            return 0;
        }
        // An application cursor resource (drawn as the arrow on screen).
        super::icon::load_cursor_resource(instance, name, false)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn LoadCursorA(instance: usize, name: usize) -> usize {
    let instance = door_windows_abi::widen(instance);
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 {
        if instance == 0 || name >> 16 == 0 {
            return door_windows_abi::widen(LoadCursorW(
                door_windows_abi::narrow(instance),
                door_windows_abi::narrow(name),
            ));
        }
        super::icon::load_cursor_resource(instance, name, true)
    })())
}
static CURSOR: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[unsafe(no_mangle)]
pub extern "system" fn SetCursor(cursor: usize) -> usize {
    let cursor = door_windows_abi::widen(cursor);
    door_windows_abi::narrow((move || -> u64 {
        let old = CURSOR.swap(cursor, core::sync::atomic::Ordering::Relaxed);
        let shape = if cursor == cursor_value(idc::IBEAM) {
            door_service_protocol::window::CURSOR_TEXT
        } else {
            door_service_protocol::window::CURSOR_ARROW
        };
        let me = tid();
        if let Some(id) = with(|u| {
            let a = u.thread(me).active?;
            u.windows.get(a)?.native
        }) {
            server::conn(|c| c.set_cursor(id, shape));
        }
        old
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCursorPos(point: usize) -> i32 {
    let point = door_windows_abi::widen(point);
    {
        let p = with(|u| u.last_click.3);
        i32::from(g::write_i32s(point, &[p.x, p.y]))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetDoubleClickTime() -> u32 {
    500
}
/// The process's door.audio connection, taken out for each call so the
/// lock is never held across IPC.
static AUDIO: g::Lock<Option<door_service_client::audio::Audio>> = g::Lock::new(None);
/// The sound-scheme event Windows plays for a MessageBeep type: `.Default`
/// for MB_OK, 0xFFFFFFFF and unlisted values, SystemHand, SystemExclamation
/// and SystemAsterisk; SystemQuestion (MB_ICONQUESTION) is silent in the
/// Windows 11 scheme.
pub(crate) fn beep_event(kind: u32) -> Option<u32> {
    use door_service_protocol::audio::sound;
    match kind {
        0x10 => Some(sound::CRITICAL_STOP),
        0x20 => None,
        0x30 => Some(sound::EXCLAMATION),
        0x40 => Some(sound::ASTERISK),
        _ => Some(sound::DEFAULT_BEEP),
    }
}
/// Plays a MessageBeep type asynchronously through door.audio. Like
/// Windows without a sound device, a missing service plays nothing and
/// still succeeds.
pub(crate) fn beep(kind: u32) {
    let Some(event) = beep_event(kind) else {
        return;
    };
    let taken = AUDIO.lock().take();
    let mut audio = taken.or_else(|| {
        let registry = g::inherited(door_abi::platform::inherited::REGISTRY);
        door_service_client::audio::Audio::connect(registry).ok()
    });
    if let Some(a) = audio.as_mut()
        && a.play_sound(event)
            .is_err_and(|e| !matches!(e, door_service_client::Error::Service(_)))
    {
        // The connection broke: reconnect on the next beep.
        audio = None;
    }
    let mut slot = AUDIO.lock();
    if slot.is_none() {
        *slot = audio;
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MessageBeep(kind: u32) -> i32 {
    beep(kind);
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn GetSystemMetrics(index: i32) -> i32 {
    let (screen, work) = with(|u| (u.screen, u.work));
    metrics::system_metric(index, screen, work)
}
/// SystemParametersInfoW/A (`door_windows_gui::parameters`): what the
/// action reads is written to `pv` (a structure whose cbSize the caller set
/// to the layout's size, or an older smaller one's); SPI_SET actions are
/// accepted without effect.
fn system_parameters(action: u32, ui: u32, pv: u64, ansi: bool) -> i32 {
    use door_windows_gui::parameters::{Parameter, system_parameter};
    let (screen, work) = with(|u| (u.screen, u.work));
    let pointer = core::mem::size_of::<usize>();
    let Some(answer) = system_parameter(action, ansi, pointer, screen, work) else {
        g::set_last_error(e::INVALID_PARAMETER);
        return 0;
    };
    let written = match answer {
        Parameter::Set => true,
        Parameter::Value(v) => pv != 0 && g::write_u32(pv, v as u32),
        Parameter::Struct {
            mut bytes,
            sized,
            also,
        } => {
            if sized {
                match g::read_u32(pv).map(|n| n as usize) {
                    Some(n)
                        if (n == bytes.len() || Some(n) == also)
                            && (ui == 0 || ui as usize == n) =>
                    {
                        bytes.truncate(n);
                        bytes[..4].copy_from_slice(&(n as u32).to_le_bytes());
                        g::write(pv, &bytes)
                    }
                    _ => false,
                }
            } else {
                pv != 0 && g::write(pv, &bytes)
            }
        }
        Parameter::Text(text) => {
            let capacity = ui as usize;
            if pv == 0 || capacity == 0 {
                false
            } else if ansi {
                let n = text.len().min(capacity - 1);
                let mut bytes = alloc::vec::Vec::from(&text.as_bytes()[..n]);
                bytes.push(0);
                g::write(pv, &bytes)
            } else {
                let units: alloc::vec::Vec<u16> = text.encode_utf16().collect();
                g::write_wide(pv, capacity, &units).is_some()
            }
        }
    };
    if !written {
        g::set_last_error(e::INVALID_PARAMETER);
        return 0;
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn SystemParametersInfoW(action: u32, ui: u32, pv: usize, _ini: u32) -> i32 {
    system_parameters(action, ui, door_windows_abi::widen(pv), false)
}
#[unsafe(no_mangle)]
pub extern "system" fn SystemParametersInfoA(action: u32, ui: u32, pv: usize, _ini: u32) -> i32 {
    system_parameters(action, ui, door_windows_abi::widen(pv), true)
}
/// SystemParametersInfoForDpi: the answers at 100% scale (Doors draws
/// programs at one scale).
#[unsafe(no_mangle)]
pub extern "system" fn SystemParametersInfoForDpi(
    action: u32,
    ui: u32,
    pv: usize,
    _ini: u32,
    _dpi: u32,
) -> i32 {
    system_parameters(action, ui, door_windows_abi::widen(pv), false)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSysColor(index: i32) -> u32 {
    metrics::sys_color(index).unwrap_or(0)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSysColorBrush(index: i32) -> usize {
    door_windows_abi::narrow((move || -> u64 {
        // SAFETY: plain call into gdi32.
        unsafe { DoorGdiSysColorBrush(index) }
    })())
}

/// GetGuiResources: GDI objects (with DCs) or USER objects of this process.
#[unsafe(no_mangle)]
pub extern "system" fn GetGuiResources(_process: usize, flags: u32) -> u32 {
    {
        match flags {
            // SAFETY: plain call into gdi32.
            0 | 2 => unsafe { DoorGdiObjectCount() },
            1 | 4 => with(|u| (u.windows.len() + u.menus.len() + u.accels.len()) as u32),
            _ => {
                g::set_last_error(e::INVALID_PARAMETER);
                0
            }
        }
    }
}

/// FLASHWINFO flags.
const FLASHW_TRAY: u32 = 2;
const FLASHW_TIMERNOFG: u32 = 12;
/// FlashWindowEx: the taskbar button flashes for attention until the window
/// comes to the foreground (door.shell attention). FLASHW_STOP (0) stops
/// it; caption-only flashing has no taskbar effect. Returns whether the
/// window was active before the call, as Windows does.
#[unsafe(no_mangle)]
pub extern "system" fn FlashWindowEx(info: usize) -> i32 {
    let info = door_windows_abi::widen(info);
    {
        // FLASHWINFO: cbSize, hwnd, dwFlags, uCount, dwTimeout.
        const FLASHWINFO: [door_windows_abi::layout::Field; 5] = {
            use door_windows_abi::layout::Field::*;
            [U32, Pointer, U32, U32, U32]
        };
        let at = |index| info + door_windows_abi::layout::offset(&FLASHWINFO, index) as u64;
        let (Some(size), Some(window), Some(flags)) = (
            g::read_u32(info),
            g::read_pointer(at(1)),
            g::read_u32(at(2)),
        ) else {
            g::set_last_error(e::INVALID_PARAMETER);
            return 0;
        };
        if (size as usize) < door_windows_abi::layout::size(&FLASHWINFO) {
            g::set_last_error(e::INVALID_PARAMETER);
            return 0;
        }
        flash(window, flags)
    }
}
/// FlashWindow(hwnd, invert): one flash request (Windows flashes the
/// taskbar button of an inactive window until it is activated).
#[unsafe(no_mangle)]
pub extern "system" fn FlashWindow(hwnd: usize, invert: i32) -> i32 {
    let hwnd = door_windows_abi::widen(hwnd);
    {
        flash(
            hwnd,
            if invert != 0 {
                FLASHW_TRAY | FLASHW_TIMERNOFG
            } else {
                0
            },
        )
    }
}
fn flash(window: u64, flags: u32) -> i32 {
    let Some(h) = super::hwnd_or_error(window) else {
        return 0;
    };
    let was_active = with(|u| u.threads.iter().any(|t| t.active == Some(h)));
    let attention = if flags & FLASHW_TRAY != 0 {
        door_service_protocol::shell::attention::REQUEST
    } else {
        door_service_protocol::shell::attention::STOP
    };
    server::taskbar_state(h, attention, None);
    i32::from(was_active)
}
/// Door taskbar progress hook (the ITaskbarList3::SetProgressState /
/// SetProgressValue pair without COM): `state` is a TBPF_* value
/// (0 none, 1 indeterminate, 2 normal, 4 error, 8 paused) and the value is
/// `completed` of `total`. Returns S_OK (0) or E_FAIL.
#[unsafe(no_mangle)]
pub extern "system" fn DoorSetTaskbarProgress(
    hwnd: u64,
    state: u32,
    completed: u64,
    total: u64,
) -> i32 {
    use door_service_protocol::shell::progress;
    const E_FAIL: i32 = 0x8000_4005_u32 as i32;
    let Some(h) = super::hwnd(hwnd) else {
        return E_FAIL;
    };
    let state = match state {
        0 => progress::NONE,
        1 => progress::INDETERMINATE,
        2 => progress::NORMAL,
        4 => progress::ERROR,
        8 => progress::PAUSED,
        _ => return E_FAIL,
    };
    let value = if total == 0 {
        0
    } else {
        (u128::from(completed.min(total)) * 1000 / u128::from(total)) as u16
    };
    if server::taskbar_state(
        h,
        door_service_protocol::shell::attention::KEEP,
        Some((state, value)),
    ) {
        0
    } else {
        E_FAIL
    }
}

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorGetInheritedHandle(index: u32) -> u64;
}
/// shell32's ShellExecute of a document no Windows association opens
/// (`choose` 0), or its "openas" verb (`choose` 1): File Explorer opens the
/// native `path` with its default app or shows the app picker, as
/// OpenWith.exe does. 0, or a Win32 error.
#[unsafe(no_mangle)]
pub extern "system" fn DoorOpenWith(path: u64, path_len: u32, choose: u32) -> u32 {
    if path == 0
        || path_len == 0
        || path_len as usize >= door_service_protocol::window::MAX_DOCUMENT
    {
        return e::INVALID_PARAMETER;
    }
    // SAFETY: shell32 passes its own buffer, live for this call.
    let path = unsafe { core::slice::from_raw_parts(path as *const u8, path_len as usize) };
    let Ok(path) = core::str::from_utf8(path) else {
        return e::INVALID_PARAMETER;
    };
    use door_service_protocol::window::open_with;
    let mode = if choose != 0 {
        open_with::CHOOSE
    } else {
        open_with::OPEN
    };
    match server::conn(|c| c.open_with(mode, path)) {
        Some(Ok(())) => 0,
        Some(Err(_)) => e::ACCESS_DENIED,
        None => 1460,
    }
}
/// shell32's ShellExecuteEx "runas": asks the window server to verify the
/// program at the native `path` (`image`, its bytes in the caller's memory)
/// and show User Account Control, as consent.exe does. `flags` are
/// `window::elevation` bits. 0, or a Win32 error.
#[unsafe(no_mangle)]
pub extern "system" fn DoorRequestElevation(
    path: u64,
    path_len: u32,
    image: u64,
    image_len: u32,
    flags: u32,
) -> u32 {
    if path == 0
        || path_len == 0
        || path_len as usize > door_service_protocol::window::elevation::MAX_PATH
    {
        return e::INVALID_PARAMETER;
    }
    // SAFETY: shell32 passes its own buffers, live for this call.
    let (path, bytes) = unsafe {
        (
            core::slice::from_raw_parts(path as *const u8, path_len as usize),
            if image == 0 {
                &[][..]
            } else {
                core::slice::from_raw_parts(image as *const u8, image_len as usize)
            },
        )
    };
    let Ok(path) = core::str::from_utf8(path) else {
        return e::INVALID_PARAMETER;
    };
    let directory =
        unsafe { DoorGetInheritedHandle(door_abi::platform::inherited::DIRECTORY as u32) };
    let script = flags & door_service_protocol::window::elevation::SCRIPT != 0;
    let section = if script || bytes.is_empty() {
        None
    } else {
        let size = (bytes.len() as u64).div_ceil(4096) * 4096;
        let address = unsafe { super::DoorReserveSectionRange(size) };
        if address == 0 {
            let _ = door_sys::write(b"DOOR_P16 ELEVATION request no section range\n");
            return e::NOT_ENOUGH_MEMORY;
        }
        let made = door_sys::section_create(size, door_abi::rights::ALL & !door_abi::rights::WAIT)
            .ok()
            .filter(|h| {
                // SAFETY: the reserved range is this section's alone.
                let mapped = unsafe {
                    door_sys::section_map(
                        *h,
                        address,
                        door_abi::rights::READ | door_abi::rights::WRITE,
                    )
                }
                .is_ok();
                if mapped {
                    // SAFETY: freshly mapped, writable, `size` >= `bytes.len()`.
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            bytes.as_ptr(),
                            address as *mut u8,
                            bytes.len(),
                        )
                    };
                    let _ = unsafe { door_sys::section_unmap(address, size) };
                } else {
                    let _ = door_sys::close(*h);
                }
                mapped
            });
        unsafe { super::DoorReleaseSectionRange(address, size) };
        match made {
            Some(h) => Some((h, bytes.len() as u32)),
            None => {
                let _ = door_sys::write(b"DOOR_P16 ELEVATION request no section\n");
                return e::NOT_ENOUGH_MEMORY;
            }
        }
    };
    let sent = server::conn(|c| c.request_elevation(path, flags, section, directory));
    if let Some((h, _)) = section {
        let _ = door_sys::close(h);
    }
    match sent {
        Some(Ok(())) => 0,
        // ERROR_CANCELLED while another prompt is up; access denied when the
        // window server refuses.
        Some(Err(door_service_client::Error::Service(door_service_protocol::Code::WouldBlock))) => {
            1223
        }
        Some(Err(error)) => {
            let _ = door_sys::write(
                alloc::format!("DOOR_P16 ELEVATION request error={error:?}\n").as_bytes(),
            );
            e::ACCESS_DENIED
        }
        None => {
            let _ = door_sys::write(b"DOOR_P16 ELEVATION request no window server\n");
            1460
        }
    }
}
/// The outcome of this process's last Run as administrator of a program
/// (after `DoorRequestElevation` answered 0), as ShellExecuteEx's "runas"
/// returns it: waits while the prompt is open or the program is starting;
/// 0 and the new process's handle (wait and query rights) in `process`
/// when it started, ERROR_CANCELLED when the user answered No,
/// ERROR_ACCESS_DENIED when it could not start.
#[unsafe(no_mangle)]
pub extern "system" fn DoorElevationResult(process: u64) -> u32 {
    use door_service_client::window::Elevation;
    loop {
        match server::conn(|c| c.elevation_status()) {
            Some(Ok(None)) => g::sleep(50),
            Some(Ok(Some(Elevation::Started(handle)))) => {
                if process != 0 && !g::write(process, &handle.to_le_bytes()) {
                    let _ = door_sys::close(handle);
                    return e::INVALID_PARAMETER;
                }
                if process == 0 {
                    let _ = door_sys::close(handle);
                }
                return 0;
            }
            Some(Ok(Some(Elevation::Declined))) => return 1223,
            _ => return e::ACCESS_DENIED,
        }
    }
}

// ---- the keyboard layout (US English, the only one Doors has) -------------------------

/// HKL of the US English layout (language 0x0409, layout 0x0409).
const US_LAYOUT: usize = 0x0409_0409;
/// `GetKeyboardLayout(thread)`: every thread has the US layout.
#[unsafe(no_mangle)]
pub extern "system" fn GetKeyboardLayout(_thread: u32) -> usize {
    US_LAYOUT
}
/// `GetKeyboardLayoutList(count, list)`: the one layout.
#[unsafe(no_mangle)]
pub extern "system" fn GetKeyboardLayoutList(count: i32, list: usize) -> i32 {
    let list = door_windows_abi::widen(list);
    if count > 0 && list != 0 {
        unsafe { (list as *mut usize).write_unaligned(US_LAYOUT) };
    }
    1
}
/// `MapVirtualKeyExW`: `MapVirtualKeyW` in the (US) layout given.
#[unsafe(no_mangle)]
pub extern "system" fn MapVirtualKeyExW(code: u32, kind: u32, _layout: usize) -> u32 {
    MapVirtualKeyW(code, kind)
}
/// The ANSI forms (.NET's default P/Invoke character set looks them up):
/// the US layout's characters are ASCII, so they equal the W forms.
#[unsafe(no_mangle)]
pub extern "system" fn MapVirtualKeyA(code: u32, kind: u32) -> u32 {
    MapVirtualKeyW(code, kind)
}
#[unsafe(no_mangle)]
pub extern "system" fn MapVirtualKeyExA(code: u32, kind: u32, _layout: usize) -> u32 {
    MapVirtualKeyW(code, kind)
}
/// `ToUnicodeEx(vk, scan, keyState[256], buffer, size, flags, layout)`: the
/// character the key types with the modifiers in `keyState` (Shift,
/// Caps Lock, Ctrl, Alt); 1 written, 0 for a key that types nothing.
/// Doors' layout has no dead keys, so nothing is kept between calls.
#[unsafe(no_mangle)]
pub extern "system" fn ToUnicodeEx(
    key: u32,
    scan: u32,
    state: usize,
    buffer: usize,
    size: i32,
    _flags: u32,
    _layout: usize,
) -> i32 {
    let (state, buffer) = (
        door_windows_abi::widen(state),
        door_windows_abi::widen(buffer),
    );
    // A key release (scan bit 15) types nothing.
    if scan & 0x8000 != 0 || state == 0 || buffer == 0 || size < 1 {
        return 0;
    }
    let down = |vk: usize| unsafe { (state as *const u8).add(vk).read() } & 0x80 != 0;
    let toggled = |vk: usize| unsafe { (state as *const u8).add(vk).read() } & 1 != 0;
    let Some(unit) =
        keyboard::to_char(key as u8, down(0x10), toggled(0x14), down(0x11), down(0x12))
    else {
        return 0;
    };
    unsafe { (buffer as *mut u16).write_unaligned(unit) };
    if size > 1 {
        unsafe { (buffer as *mut u16).add(1).write_unaligned(0) };
    }
    1
}
/// `ToUnicode`: `ToUnicodeEx` in the thread's layout.
#[unsafe(no_mangle)]
pub extern "system" fn ToUnicode(
    key: u32,
    scan: u32,
    state: usize,
    buffer: usize,
    size: i32,
    flags: u32,
) -> i32 {
    ToUnicodeEx(key, scan, state, buffer, size, flags, US_LAYOUT)
}
/// `VkKeyScanExW(ch, layout)`: the virtual key in the low byte and the
/// shift state in the high byte (1 Shift, 2 Ctrl, 4 Alt) that type `ch`;
/// -1 when no key does.
#[unsafe(no_mangle)]
pub extern "system" fn VkKeyScanExW(ch: u16, _layout: usize) -> i16 {
    for key in 0x08u8..=0xfe {
        for (shift, ctrl, bits) in [(false, false, 0), (true, false, 1), (false, true, 2)] {
            if keyboard::to_char(key, shift, false, ctrl, false) == Some(ch) {
                return (bits << 8) | i16::from(key);
            }
        }
    }
    -1
}
#[unsafe(no_mangle)]
pub extern "system" fn VkKeyScanW(ch: u16) -> i16 {
    VkKeyScanExW(ch, US_LAYOUT)
}
/// The cursor SetCursor last set (GetCursor, GetCursorInfo).
pub(crate) fn current_cursor() -> u64 {
    CURSOR.load(core::sync::atomic::Ordering::Relaxed)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCursor() -> usize {
    door_windows_abi::narrow(current_cursor())
}
