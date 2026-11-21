//! The window side of Shell_NotifyIcon: shell32 hands each request here
//! (`DoorNotifyIcon`); the icon's pixels go to the window server in a
//! read-only section, and clicks in the hidden-icons flyout come back as
//! `NOTIFY_ICON_EVENT`s, which become the callback messages Windows posts
//! (the mouse message in lParam, or NOTIFYICON_VERSION_4's packed form).
use super::{g, server, with};
use alloc::vec::Vec;
use door_service_protocol::window::{self as w, notify};
use door_windows_gui::win32_error as e;

struct Entry {
    hwnd: u64,
    id: u32,
    callback: u32,
}
static ENTRIES: g::Lock<Vec<Entry>> = g::Lock::new(Vec::new());

const WM_CONTEXTMENU: u32 = 0x007b;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_LBUTTONDBLCLK: u32 = 0x0203;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_RBUTTONUP: u32 = 0x0205;
const NIN_SELECT: u32 = 0x0400;
const NIN_KEYSELECT: u32 = 0x0401;

/// The icon's pixels at 32 px in a read-only section for the server.
fn icon_section(icon: u64) -> Option<u64> {
    let (image, _, _) = super::icon::image_of(icon)?;
    let edge = notify::EDGE;
    let image = image.resample(edge, edge)?;
    let bytes: Vec<u8> = image.px.iter().flat_map(|p| p.to_le_bytes()).collect();
    let size = 4096u64.max((bytes.len() as u64).div_ceil(4096) * 4096);
    // SAFETY: the reserved range is this section's alone.
    let address = unsafe { super::DoorReserveSectionRange(size) };
    if address == 0 {
        return None;
    }
    let made = door_sys::section_create(size, door_abi::rights::ALL & !door_abi::rights::WAIT)
        .ok()
        .filter(|h| {
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
                    core::ptr::copy_nonoverlapping(bytes.as_ptr(), address as *mut u8, bytes.len());
                    let _ = door_sys::section_unmap(address, size);
                }
            } else {
                let _ = door_sys::close(*h);
            }
            mapped
        });
    unsafe { super::DoorReleaseSectionRange(address, size) };
    made
}
/// shell32's Shell_NotifyIcon: `op` NIM_*, `flags` NIF_*, the icon (an
/// HICON), the tip (UTF-16, `tip_len` units). 1 on success.
#[allow(clippy::too_many_arguments)]
#[unsafe(no_mangle)]
pub extern "system" fn DoorNotifyIcon(
    op: u32,
    hwnd: u64,
    id: u32,
    flags: u32,
    callback: u32,
    icon: u64,
    tip: u64,
    tip_len: u32,
    version: u32,
) -> i32 {
    if op > u32::from(notify::SET_VERSION) || with(|u| u.windows.resolve(hwnd)).is_none() {
        g::set_last_error(e::INVALID_WINDOW_HANDLE);
        return 0;
    }
    let op = op as u8;
    let tip = if flags & u32::from(notify::TIP) != 0 && tip != 0 {
        let units = g::read_wide_n(tip, tip_len.min(128) as usize).unwrap_or_default();
        let end = units.iter().position(|u| *u == 0).unwrap_or(units.len());
        alloc::string::String::from_utf16_lossy(&units[..end])
    } else {
        alloc::string::String::new()
    };
    let wants_icon = flags & u32::from(notify::ICON) != 0
        && matches!(op, notify::ADD | notify::MODIFY)
        && icon != 0;
    let section = if wants_icon { icon_section(icon) } else { None };
    let sent_flags = (flags as u16 & (notify::MESSAGE | notify::TIP))
        | if section.is_some() { notify::ICON } else { 0 };
    let result = server::conn(|c| {
        c.notify_icon(
            op,
            version.min(4) as u8,
            sent_flags,
            hwnd,
            id,
            callback,
            &tip,
            section,
        )
    });
    if let Some(s) = section {
        let _ = door_sys::close(s);
    }
    if !matches!(result, Some(Ok(()))) {
        g::set_last_error(e::INVALID_PARAMETER);
        return 0;
    }
    let mut entries = ENTRIES.lock();
    let at = entries.iter().position(|x| x.hwnd == hwnd && x.id == id);
    match op {
        notify::ADD => {
            let callback = if flags & u32::from(notify::MESSAGE) != 0 {
                callback
            } else {
                0
            };
            entries.push(Entry { hwnd, id, callback });
        }
        notify::MODIFY if flags & u32::from(notify::MESSAGE) != 0 => {
            if let Some(at) = at {
                entries[at].callback = callback;
            }
        }
        notify::DELETE => {
            if let Some(at) = at {
                entries.remove(at);
            }
        }
        _ => (),
    }
    1
}
/// A click from the hidden-icons flyout: the callback messages Windows
/// posts for it, in order.
pub(crate) fn clicked(e: &w::Event) {
    let (hwnd, id) = (e.value, e.a);
    let Some(callback) = ENTRIES
        .lock()
        .iter()
        .find(|x| x.hwnd == hwnd && x.id == id && x.callback != 0)
        .map(|x| x.callback)
    else {
        return;
    };
    let version = e.detail;
    let messages: &[u32] = match (e.b, version >= 3) {
        (1, true) => &[WM_LBUTTONDOWN, WM_LBUTTONUP, NIN_SELECT],
        (1, false) => &[WM_LBUTTONDOWN, WM_LBUTTONUP],
        (3, _) => &[WM_LBUTTONDBLCLK, WM_LBUTTONUP],
        (2, true) => &[WM_RBUTTONDOWN, WM_RBUTTONUP, WM_CONTEXTMENU],
        (2, false) => &[WM_RBUTTONDOWN, WM_RBUTTONUP],
        (4, true) => &[NIN_KEYSELECT],
        _ => &[],
    };
    // NOTIFYICON_VERSION_4 anchors at the icon's centre.
    let x = e.rect.x + e.rect.width as i32 / 2;
    let y = e.rect.y + e.rect.height as i32 / 2;
    for &m in messages {
        let (wparam, lparam) = if version >= 4 {
            (
                u64::from(x as u16) | u64::from(y as u16) << 16,
                i64::from(m as u16) | i64::from(id as u16) << 16,
            )
        } else {
            (u64::from(id), i64::from(m))
        };
        let _ = super::msg::post(hwnd, callback, wparam, lparam);
    }
}
/// A window's icons go with it (Windows removes them when the program
/// ends; a destroyed window's are no use).
pub(crate) fn forget_window(hwnd: u64) {
    let ids: Vec<u32> = {
        let mut entries = ENTRIES.lock();
        let ids = entries
            .iter()
            .filter(|x| x.hwnd == hwnd)
            .map(|x| x.id)
            .collect();
        entries.retain(|x| x.hwnd != hwnd);
        ids
    };
    for id in ids {
        server::conn(|c| c.notify_icon(notify::DELETE, 0, 0, hwnd, id, 0, "", None));
    }
}

/// Shell_NotifyIconGetRect's half: where notification icon `id` of
/// `hwnd` is on screen (RECT at `out`); 0 when the icon is not this
/// program's.
#[unsafe(no_mangle)]
pub extern "system" fn DoorNotifyIconRect(hwnd: u64, id: u32, out: u64) -> i32 {
    if out == 0 || with(|u| u.windows.resolve(hwnd)).is_none() {
        return 0;
    }
    match server::conn(|c| c.notify_icon_rect(hwnd, id)) {
        Some(Ok(r)) => i32::from(g::write_i32s(out, &r)),
        _ => 0,
    }
}
