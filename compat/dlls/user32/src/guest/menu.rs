//! Menus, accelerators and string/menu/accelerator resources, the menu bar
//! (drawn in the non-client band below the native caption) and modal menu
//! tracking. Popup menus are drawn inside the owning top-level surface.
use super::{
    GetStockObject, SelectObject, SetBkMode, SetTextColor, g, hwnd, hwnd_or_error, msg, paint, tid,
    with,
};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use door_windows_gui::handles::Handle;
use door_windows_gui::menu::{Action, Tracker, display, layout_bar, mnemonic};
use door_windows_gui::metrics;
use door_windows_gui::msg::{color, mf, wm};
use door_windows_gui::resource;
use door_windows_gui::{Point, Rect, win32_error as e};

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorFindResource(module: u64, kind: u64, name: u64, language: u16) -> u64;
    fn DoorLoadResource(module: u64, handle: u64) -> u64;
    fn DoorSizeofResource(module: u64, handle: u64) -> u32;
}

const RT_MENU: u64 = 4;
const RT_STRING: u64 = 6;
const RT_ACCELERATOR: u64 = 9;
pub(crate) const RT_DIALOG: u64 = 5;

/// Resource bytes of (module, type, name), captured through checked copy.
pub(crate) fn resource_bytes(module: u64, kind: u64, name: u64) -> Option<Vec<u8>> {
    // SAFETY: ntdll validates the module and captures name pointers.
    let handle = unsafe { DoorFindResource(module, kind, name, 0) };
    if handle == 0 {
        return None;
    }
    let (address, size) = unsafe {
        (
            DoorLoadResource(module, handle),
            DoorSizeofResource(module, handle),
        )
    };
    if address == 0 || size == 0 || size > 1 << 20 {
        return None;
    }
    g::read_bytes(address, size as usize)
}

fn fail<T: Default>(code: u32) -> T {
    g::set_last_error(code);
    T::default()
}
fn menu_of(value: u64) -> Option<Handle> {
    let h = with(|u| u.menus.resolve(value));
    if h.is_none() {
        g::set_last_error(e::INVALID_MENU_HANDLE);
    }
    h
}

#[unsafe(no_mangle)]
pub extern "system" fn CreateMenu() -> usize {
    door_windows_abi::narrow((move || -> u64 {
        with(|u| u.menus.create(false)).map_or_else(|_| fail(e::NOT_ENOUGH_MEMORY), Handle::value)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CreatePopupMenu() -> usize {
    door_windows_abi::narrow((move || -> u64 {
        with(|u| u.menus.create(true)).map_or_else(|_| fail(e::NOT_ENOUGH_MEMORY), Handle::value)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn DestroyMenu(menu: usize) -> i32 {
    let menu = door_windows_abi::widen(menu);
    {
        let Some(h) = menu_of(menu) else { return 0 };
        i32::from(with(|u| u.menus.destroy(h)).is_ok())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn IsMenu(menu: usize) -> i32 {
    let menu = door_windows_abi::widen(menu);
    i32::from(with(|u| u.menus.resolve(menu)).is_some())
}

fn item_text(flags: u32, text: u64) -> Option<Vec<u16>> {
    if flags & (mf::SEPARATOR | mf::BITMAP | mf::OWNERDRAW) != 0 || text == 0 {
        return Some(Vec::new());
    }
    g::read_wide(text, 4096)
}
fn insert(menu: u64, position: Option<(u32, bool)>, flags: u32, id: u64, text: u64) -> i32 {
    let Some(h) = menu_of(menu) else { return 0 };
    let Some(t) = item_text(flags, text) else {
        return fail(e::INVALID_PARAMETER);
    };
    match with(|u| u.menus.insert(h, position, flags, id, t)) {
        Ok(()) => 1,
        Err(err) => fail(err.win32()),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn AppendMenuW(menu: usize, flags: u32, id: usize, text: usize) -> i32 {
    let menu = door_windows_abi::widen(menu);
    let id = door_windows_abi::widen(id);
    let text = door_windows_abi::widen(text);
    insert(menu, None, flags, id, text)
}
#[unsafe(no_mangle)]
pub extern "system" fn InsertMenuW(
    menu: usize,
    position: u32,
    flags: u32,
    id: usize,
    text: usize,
) -> i32 {
    let menu = door_windows_abi::widen(menu);
    let id = door_windows_abi::widen(id);
    let text = door_windows_abi::widen(text);
    {
        let at = if position == 0xffff_ffff && flags & mf::BYPOSITION != 0 {
            None
        } else {
            Some((position, flags & mf::BYPOSITION != 0))
        };
        insert(menu, at, flags & !mf::BYPOSITION, id, text)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ModifyMenuW(
    menu: usize,
    position: u32,
    flags: u32,
    id: usize,
    text: usize,
) -> i32 {
    let menu = door_windows_abi::widen(menu);
    let id = door_windows_abi::widen(id);
    let text = door_windows_abi::widen(text);
    {
        let Some(h) = menu_of(menu) else { return 0 };
        let by_position = flags & mf::BYPOSITION != 0;
        let Some(t) = item_text(flags, text) else {
            return fail(e::INVALID_PARAMETER);
        };
        let ok = with(|u| {
            let (m, i) = u.menus.find(h, position, by_position)?;
            let item = u.menus.get_mut(m)?.items.get_mut(i)?;
            item.text = t;
            item.kind = flags & (mf::SEPARATOR | mf::OWNERDRAW | mf::MENUBARBREAK | mf::MENUBREAK);
            item.state = flags & (mf::GRAYED | mf::DISABLED | mf::CHECKED);
            if flags & mf::POPUP == 0 {
                item.id = id as u32;
            }
            Some(())
        });
        if ok.is_some() {
            1
        } else {
            fail(e::INVALID_PARAMETER)
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn RemoveMenu(menu: usize, position: u32, flags: u32) -> i32 {
    let menu = door_windows_abi::widen(menu);
    {
        let Some(h) = menu_of(menu) else { return 0 };
        i32::from(
            with(|u| {
                u.menus
                    .remove(h, position, flags & mf::BYPOSITION != 0, false)
            })
            .is_ok(),
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DeleteMenu(menu: usize, position: u32, flags: u32) -> i32 {
    let menu = door_windows_abi::widen(menu);
    {
        let Some(h) = menu_of(menu) else { return 0 };
        i32::from(
            with(|u| {
                u.menus
                    .remove(h, position, flags & mf::BYPOSITION != 0, true)
            })
            .is_ok(),
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetMenuItemCount(menu: usize) -> i32 {
    let menu = door_windows_abi::widen(menu);
    {
        let Some(h) = menu_of(menu) else { return -1 };
        with(|u| u.menus.count(h)).map_or(-1, |n| n as i32)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetMenuItemID(menu: usize, position: i32) -> u32 {
    let menu = door_windows_abi::widen(menu);
    {
        let Some(h) = menu_of(menu) else {
            return 0xffff_ffff;
        };
        with(|u| u.menus.item_id(h, position as u32))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSubMenu(menu: usize, position: i32) -> usize {
    let menu = door_windows_abi::widen(menu);
    door_windows_abi::narrow((move || -> u64 {
        let Some(h) = menu_of(menu) else { return 0 };
        with(|u| u.menus.submenu(h, position as u32)).map_or(0, Handle::value)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetMenuStringW(
    menu: usize,
    item: u32,
    out: usize,
    capacity: i32,
    flags: u32,
) -> i32 {
    let menu = door_windows_abi::widen(menu);
    let out = door_windows_abi::widen(out);
    {
        let Some(h) = menu_of(menu) else { return 0 };
        let Some(text) = with(|u| {
            u.menus
                .item(h, item, flags & mf::BYPOSITION != 0)
                .map(|i| i.text.clone())
        }) else {
            return 0;
        };
        if out == 0 || capacity <= 0 {
            return text.len() as i32;
        }
        g::write_wide(out, capacity as usize, &text).map_or(0, |n| n as i32)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetMenuState(menu: usize, item: u32, flags: u32) -> u32 {
    let menu = door_windows_abi::widen(menu);
    {
        let Some(h) = menu_of(menu) else {
            return 0xffff_ffff;
        };
        with(|u| u.menus.state(h, item, flags & mf::BYPOSITION != 0))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn CheckMenuItem(menu: usize, item: u32, flags: u32) -> u32 {
    let menu = door_windows_abi::widen(menu);
    {
        let Some(h) = menu_of(menu) else {
            return 0xffff_ffff;
        };
        with(|u| u.menus.check(h, item, flags))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn EnableMenuItem(menu: usize, item: u32, flags: u32) -> u32 {
    let menu = door_windows_abi::widen(menu);
    {
        let Some(h) = menu_of(menu) else {
            return 0xffff_ffff;
        };
        with(|u| u.menus.enable(h, item, flags))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetMenu(hwnd_value: usize) -> usize {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    door_windows_abi::narrow((move || -> u64 {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        with(|u| {
            u.windows
                .get(h)
                .filter(|w| !w.is_child())
                .map_or(0, |w| w.id)
        })
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SetMenu(hwnd_value: usize, menu: usize) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    let menu = door_windows_abi::widen(menu);
    {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        if menu != 0 && menu_of(menu).is_none() {
            return 0;
        }
        with(|u| {
            if let Some(w) = u.windows.get_mut(h)
                && !w.is_child()
            {
                w.id = menu;
                w.frame_dirty = true;
            }
        });
        1
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DrawMenuBar(hwnd_value: usize) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    {
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        let hdc = paint::GetWindowDC(door_windows_abi::narrow(hwnd_value));
        if hdc != 0 {
            draw_bar(h, door_windows_abi::widen(hdc));
            paint::ReleaseDC(door_windows_abi::narrow(hwnd_value), hdc);
        }
        1
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn LoadMenuW(instance: usize, name: usize) -> usize {
    let instance = door_windows_abi::widen(instance);
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 {
        let Some(bytes) = resource_bytes(instance, RT_MENU, name) else {
            return fail(e::RESOURCE_NAME_NOT_FOUND);
        };
        LoadMenuIndirectBytes(&bytes)
    })())
}
#[allow(non_snake_case)]
fn LoadMenuIndirectBytes(bytes: &[u8]) -> u64 {
    match resource::parse_menu(bytes) {
        Ok(nodes) => with(|u| u.menus.from_template(&nodes, false))
            .map_or_else(|err| fail(err.win32()), Handle::value),
        Err(err) => fail(err.win32()),
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn LoadMenuIndirectW(template: usize) -> usize {
    let template = door_windows_abi::widen(template);
    door_windows_abi::narrow((move || -> u64 {
        // MENU/MENUEX templates are self-delimiting; capture a bounded prefix.
        let mut bytes = Vec::new();
        for chunk in 0..64u64 {
            let mut b = [0u8; 1024];
            if !g::read(template + chunk * 1024, &mut b) {
                break;
            }
            bytes.extend_from_slice(&b);
            if resource::parse_menu(&bytes).is_ok() {
                break;
            }
        }
        LoadMenuIndirectBytes(&bytes)
    })())
}

#[unsafe(no_mangle)]
pub extern "system" fn LoadStringW(instance: usize, id: u32, out: usize, capacity: i32) -> i32 {
    let instance = door_windows_abi::widen(instance);
    let out = door_windows_abi::widen(out);
    {
        let block = u64::from((id >> 4) + 1);
        let Some(bytes) = resource_bytes(instance, RT_STRING, block) else {
            return fail(e::RESOURCE_NAME_NOT_FOUND);
        };
        let text = match resource::string_from_block(&bytes, (id & 15) as usize) {
            Ok(Some(t)) => t,
            Ok(None) => return fail(e::RESOURCE_NAME_NOT_FOUND),
            Err(err) => return fail(err.win32()),
        };
        if capacity == 0 {
            // Read-only pointer form: point at the string inside the image.
            // SAFETY: as in resource_bytes.
            let handle = unsafe { DoorFindResource(instance, RT_STRING, block, 0) };
            let address = unsafe { DoorLoadResource(instance, handle) };
            let mut at = 0usize;
            for _ in 0..(id & 15) {
                let len = u16::from_le_bytes([bytes[at], bytes[at + 1]]) as usize;
                at += 2 + 2 * len;
            }
            g::write_pointer(out, address + at as u64 + 2);
            return text.len() as i32;
        }
        if capacity < 0 {
            return fail(e::INVALID_PARAMETER);
        }
        g::write_wide(out, capacity as usize, &text).map_or(0, |n| n as i32)
    }
}

// ---- accelerators ---------------------------------------------------------------------

fn accel_table(list: Vec<resource::Accel>) -> u64 {
    with(|u| u.accels.insert(list)).map_or_else(|_| fail(e::NOT_ENOUGH_MEMORY), Handle::value)
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateAcceleratorTableW(entries: usize, count: i32) -> usize {
    let entries = door_windows_abi::widen(entries);
    door_windows_abi::narrow((move || -> u64 {
        if !(1..=resource::MAX_ITEMS as i32).contains(&count) {
            return fail(e::INVALID_PARAMETER);
        }
        let Some(bytes) = g::read_bytes(entries, count as usize * 6) else {
            return fail(e::INVALID_PARAMETER);
        };
        match resource::parse_accel_array(&bytes) {
            Ok(list) => accel_table(list),
            Err(err) => fail(err.win32()),
        }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn LoadAcceleratorsW(instance: usize, name: usize) -> usize {
    let instance = door_windows_abi::widen(instance);
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 {
        let Some(bytes) = resource_bytes(instance, RT_ACCELERATOR, name) else {
            return fail(e::RESOURCE_NAME_NOT_FOUND);
        };
        match resource::parse_accelerators(&bytes) {
            Ok(list) => accel_table(list),
            Err(err) => fail(err.win32()),
        }
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn DestroyAcceleratorTable(table: usize) -> i32 {
    let table = door_windows_abi::widen(table);
    i32::from(with(|u| u.accels.resolve(table).and_then(|h| u.accels.remove(h))).is_some())
}
#[unsafe(no_mangle)]
pub extern "system" fn CopyAcceleratorTableW(table: usize, out: usize, count: i32) -> i32 {
    let table = door_windows_abi::widen(table);
    let out = door_windows_abi::widen(out);
    {
        let Some(list) = with(|u| {
            u.accels
                .resolve(table)
                .and_then(|h| u.accels.get(h).cloned())
        }) else {
            return fail(e::INVALID_ACCEL_HANDLE);
        };
        if out == 0 {
            return list.len() as i32;
        }
        let n = list.len().min(count.max(0) as usize);
        let mut b = Vec::with_capacity(n * 6);
        for a in &list[..n] {
            b.extend_from_slice(&[a.virt, 0]);
            b.extend_from_slice(&a.key.to_le_bytes());
            b.extend_from_slice(&a.cmd.to_le_bytes());
        }
        if !g::write(out, &b) {
            return fail(e::INVALID_PARAMETER);
        }
        n as i32
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn TranslateAcceleratorW(
    hwnd_value: usize,
    table: usize,
    message: usize,
) -> i32 {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    let table = door_windows_abi::widen(table);
    let message = door_windows_abi::widen(message);
    {
        let Some(m) = msg::read_msg(message) else {
            return fail(e::INVALID_PARAMETER);
        };
        let Some(h) = hwnd_or_error(hwnd_value) else {
            return 0;
        };
        let found = with(|u| {
            let list = u.accels.resolve(table).and_then(|t| u.accels.get(t))?;
            door_windows_gui::keyboard::match_accelerator(
                list,
                m.message,
                m.wparam,
                u.queues.key_state(super::tid()),
            )
        });
        let Some(cmd) = found else { return 0 };
        // A disabled menu item consumes its accelerator without a command.
        let disabled = with(|u| {
            let bar = u.windows.get(h).filter(|w| !w.is_child()).map(|w| w.id)?;
            let menu = u.menus.resolve(bar)?;
            let state = u.menus.state(menu, u32::from(cmd), false);
            Some(state != 0xffff_ffff && state & (mf::GRAYED | mf::DISABLED) != 0)
        })
        .unwrap_or(false);
        if !disabled {
            msg::send(hwnd_value, wm::COMMAND, u64::from(cmd) | 1 << 16, 0);
        }
        1
    }
}

// ---- menu bar and tracking ------------------------------------------------------------

static TRACKING: AtomicBool = AtomicBool::new(false);
static CANCEL: AtomicBool = AtomicBool::new(false);

pub(crate) fn cancel_tracking() {
    if TRACKING.load(Ordering::Relaxed) {
        CANCEL.store(true, Ordering::Relaxed);
    }
}

/// Menu-bar item rectangles in window coordinates.
fn bar_items(h: Handle, hdc: u64) -> (Option<Handle>, Vec<Rect>, Rect) {
    let info = with(|u| {
        let w = u.windows.get(h)?;
        let menu = u.menus.resolve(w.id)?;
        let texts: Vec<Vec<u16>> = u
            .menus
            .get(menu)?
            .items
            .iter()
            .map(|i| display(&i.text).0)
            .collect();
        Some((menu, texts, w.rect, w.style))
    });
    let Some((menu, texts, rect, style)) = info else {
        return (None, Vec::new(), Rect::EMPTY);
    };
    let frame = metrics::frame(style);
    let bar = Rect::new(
        frame,
        metrics::CAPTION,
        rect.width() - frame,
        metrics::CAPTION + metrics::MENU_HEIGHT,
    );
    let widths: Vec<i32> = texts.iter().map(|t| paint::text_width(hdc, t).0).collect();
    let rects = layout_bar(&widths, bar.width(), metrics::MENU_HEIGHT)
        .into_iter()
        .map(|r| r.offset(bar.left, bar.top))
        .collect();
    (Some(menu), rects, bar)
}

fn select_ui_font(hdc: u64) {
    unsafe {
        SelectObject(
            door_windows_abi::narrow(hdc),
            GetStockObject(door_windows_gui::msg::stock::DEFAULT_GUI_FONT),
        );
        SetBkMode(
            door_windows_abi::narrow(hdc),
            door_windows_gui::msg::bk::TRANSPARENT,
        );
    }
}

static HOT: g::Lock<Option<(u64, usize)>> = g::Lock::new(None);

pub(crate) fn draw_bar(h: Handle, hdc: u64) {
    select_ui_font(hdc);
    let (menu, rects, bar) = bar_items(h, hdc);
    let Some(menu) = menu else { return };
    paint::fill(hdc, bar, (color::MENUBAR + 1) as u64);
    let hot = *HOT.lock();
    let items = with(|u| u.menus.get(menu).map(|m| m.items.clone())).unwrap_or_default();
    for (i, (r, item)) in rects.iter().zip(items.iter()).enumerate() {
        let selected = hot == Some((h.value(), i));
        if selected {
            paint::fill(hdc, *r, (color::MENUHILIGHT + 1) as u64);
        }
        let text_color = if item.enabled() {
            color::MENUTEXT
        } else {
            color::GRAYTEXT
        };
        unsafe {
            SetTextColor(
                door_windows_abi::narrow(hdc),
                metrics::SYS_COLORS[text_color as usize],
            )
        };
        let mut rr = *r;
        paint::draw_text(hdc, &item.text, &mut rr, 0x25);
    }
}

/// Draw an open popup menu at window position `at`; returns its rectangle
/// and item rectangles (window coordinates).
fn draw_popup(h: Handle, menu: Handle, at: Point, selected: Option<usize>) -> (Rect, Vec<Rect>) {
    let hdc = paint::GetWindowDC(door_windows_abi::narrow(h.value()));
    if hdc == 0 {
        return (Rect::EMPTY, Vec::new());
    }
    select_ui_font(door_windows_abi::widen(hdc));
    let items = with(|u| u.menus.get(menu).map(|m| m.items.clone())).unwrap_or_default();
    let row = 22;
    let mut label_w = 0;
    let mut accel_w = 0;
    for it in &items {
        let (label, accel, _) = display(&it.text);
        label_w = label_w.max(paint::text_width(door_windows_abi::widen(hdc), &label).0);
        accel_w = accel_w.max(paint::text_width(door_windows_abi::widen(hdc), &accel).0);
    }
    let width = 28 + label_w + 24 + accel_w + 16;
    let mut y = at.y + 2;
    let mut rects = Vec::new();
    for it in &items {
        let hgt = if it.separator() { 7 } else { row };
        rects.push(Rect::new(at.x + 2, y, at.x + width - 2, y + hgt));
        y += hgt;
    }
    let outer = Rect::new(at.x, at.y, at.x + width, y + 2);
    paint::fill(
        door_windows_abi::widen(hdc),
        outer,
        (color::MENU + 1) as u64,
    );
    paint::frame(
        door_windows_abi::widen(hdc),
        outer,
        (color::BTNSHADOW + 1) as u64,
    );
    for (i, (r, it)) in rects.iter().zip(items.iter()).enumerate() {
        if it.separator() {
            paint::fill(
                door_windows_abi::widen(hdc),
                Rect::new(r.left + 4, r.top + 3, r.right - 4, r.top + 4),
                (color::BTNSHADOW + 1) as u64,
            );
            continue;
        }
        if selected == Some(i) {
            paint::fill(
                door_windows_abi::widen(hdc),
                *r,
                (color::MENUHILIGHT + 1) as u64,
            );
        }
        let c = if it.enabled() {
            color::MENUTEXT
        } else {
            color::GRAYTEXT
        };
        unsafe {
            SetTextColor(
                door_windows_abi::narrow(door_windows_abi::widen(hdc)),
                metrics::SYS_COLORS[c as usize],
            )
        };
        let (label, accel, _) = display(&it.text);
        let mut lr = Rect::new(r.left + 26, r.top, r.right, r.bottom);
        paint::draw_text(door_windows_abi::widen(hdc), &label, &mut lr, 0x24 | 0x800);
        let mut ar = Rect::new(r.left, r.top, r.right - 12, r.bottom);
        paint::draw_text(door_windows_abi::widen(hdc), &accel, &mut ar, 0x26 | 0x800);
        if it.state & mf::CHECKED != 0 {
            let mut cr = Rect::new(r.left + 6, r.top, r.left + 24, r.bottom);
            paint::draw_text(door_windows_abi::widen(hdc), &[0x2713], &mut cr, 0x25);
        }
    }
    paint::ReleaseDC(door_windows_abi::narrow(h.value()), hdc);
    (outer, rects)
}

fn redraw_window(h: Handle) {
    with(|u| {
        u.windows.invalidate(h, None, true);
        for x in u.windows.subtree(h) {
            if let Some(w) = u.windows.get_mut(x) {
                w.frame_dirty = true;
            }
        }
    });
    paint::update_now(h);
    let hdc = paint::GetWindowDC(door_windows_abi::narrow(h.value()));
    if hdc != 0 {
        draw_bar(h, door_windows_abi::widen(hdc));
        paint::ReleaseDC(door_windows_abi::narrow(h.value()), hdc);
    }
}

/// Modal menu loop over the bar of `h`, starting at `bar` (None = keyboard
/// activation with nothing open). Returns when a command was chosen or the
/// menu closed.
fn track(h: Handle, start: Option<usize>, open: bool) {
    if TRACKING.swap(true, Ordering::AcqRel) {
        return;
    }
    CANCEL.store(false, Ordering::Relaxed);
    let me = tid();
    let Some(bar) = with(|u| u.windows.get(h).and_then(|w| u.menus.resolve(w.id))) else {
        TRACKING.store(false, Ordering::Release);
        return;
    };
    msg::send(h.value(), wm::ENTERMENULOOP, 0, 0);
    msg::send(h.value(), wm::INITMENU, bar.value(), 0);
    let mut tracker = Tracker::new(start.unwrap_or(0));
    let mut popup_rects: Vec<Rect> = Vec::new();
    let mut popup_outer = Rect::EMPTY;
    let mut command = None;
    let refresh = |tracker: &mut Tracker, popup_rects: &mut Vec<Rect>, popup_outer: &mut Rect| {
        *HOT.lock() = Some((h.value(), tracker.bar));
        redraw_window(h);
        popup_rects.clear();
        *popup_outer = Rect::EMPTY;
        if let Some(&(menu, selected)) = tracker.popups.last() {
            let hdc = paint::GetWindowDC(door_windows_abi::narrow(h.value()));
            let rects = bar_items(h, door_windows_abi::widen(hdc)).1;
            paint::ReleaseDC(door_windows_abi::narrow(h.value()), hdc);
            let anchor = rects
                .get(tracker.bar)
                .map_or(Point::new(0, 0), |r| Point::new(r.left, r.bottom));
            let (outer, rs) = draw_popup(h, menu, anchor, selected);
            *popup_outer = outer;
            *popup_rects = rs;
        }
    };
    if open && let Action::Opened(sub) = with(|u| tracker.open(&u.menus, bar)) {
        msg::send(
            h.value(),
            wm::INITMENUPOPUP,
            sub.value(),
            tracker.bar as i64,
        );
    }
    refresh(&mut tracker, &mut popup_rects, &mut popup_outer);
    let origin = with(|u| u.windows.get(h).map(|w| w.rect.top_left())).unwrap_or_default();
    while command.is_none() && !CANCEL.load(Ordering::Relaxed) && hwnd(h.value()).is_some() {
        server::pump();
        msg::process_pending(me);
        let Ok(next) = msg::retrieve(me, 0, 0, 0, true) else {
            break;
        };
        let Some(m) = next else {
            msg::idle_wait(me, 8);
            continue;
        };

        let action = match m.message {
            wm::KEYDOWN | wm::SYSKEYDOWN => with(|u| {
                // Windows' menu loop translates keys too: a letter that is
                // not a navigation key selects by mnemonic (File > E&xit).
                match tracker.key(&u.menus, bar, m.wparam as u8, None) {
                    Action::None => {
                        let (shift, ctrl, alt) = u.queues.key_state(super::tid()).modifiers();
                        let caps = u
                            .queues
                            .key_state(super::tid())
                            .toggled(door_windows_gui::keyboard::vk::CAPITAL);
                        door_windows_gui::keyboard::to_char(m.wparam as u8, shift, caps, ctrl, alt)
                            .filter(|c| *c >= 0x20)
                            .map_or(Action::None, |c| tracker.key(&u.menus, bar, 0, Some(c)))
                    }
                    action => action,
                }
            }),
            wm::CHAR | wm::SYSCHAR => {
                with(|u| tracker.key(&u.menus, bar, 0, Some(m.wparam as u16)))
            }
            wm::LBUTTONDOWN
            | wm::NCLBUTTONDOWN
            | wm::LBUTTONUP
            | wm::MOUSEMOVE
            | wm::NCMOUSEMOVE => {
                let raw = Point::from_lparam(m.lparam as isize);
                let screen = if matches!(m.message, wm::NCLBUTTONDOWN | wm::NCMOUSEMOVE) {
                    raw
                } else {
                    let o = with(|u| super::hwnd(m.hwnd).map(|x| u.windows.client_origin(x)))
                        .unwrap_or_default();
                    Point::new(raw.x + o.x, raw.y + o.y)
                };
                let p = Point::new(screen.x - origin.x, screen.y - origin.y);
                let hdc = paint::GetWindowDC(door_windows_abi::narrow(h.value()));
                let bar_rects = bar_items(h, door_windows_abi::widen(hdc)).1;
                paint::ReleaseDC(door_windows_abi::narrow(h.value()), hdc);
                let press = matches!(m.message, wm::LBUTTONDOWN | wm::NCLBUTTONDOWN);
                let release = m.message == wm::LBUTTONUP;
                if let Some(i) = popup_rects.iter().position(|r| r.contains(p)) {
                    if let Some(last) = tracker.popups.last_mut() {
                        last.1 = Some(i);
                    }
                    if release || press {
                        with(|u| {
                            tracker.key(&u.menus, bar, door_windows_gui::keyboard::vk::RETURN, None)
                        })
                    } else {
                        Action::Redraw
                    }
                } else if let Some(i) = bar_rects.iter().position(|r| r.contains(p)) {
                    if press || (!tracker.popups.is_empty() && i != tracker.bar) {
                        tracker.bar = i;
                        with(|u| tracker.open(&u.menus, bar))
                    } else {
                        Action::None
                    }
                } else if press && !popup_outer.contains(p) {
                    Action::Close
                } else {
                    Action::None
                }
            }
            _ => {
                // Everything else is dispatched normally during the loop.
                let b = msg::msg_bytes(&m);
                msg::DispatchMessageW(door_windows_abi::narrow(b.as_ptr() as u64));
                Action::None
            }
        };
        match action {
            Action::Command(id) => command = Some(id),
            Action::Close => break,
            Action::Opened(sub) => {
                msg::send(
                    h.value(),
                    wm::INITMENUPOPUP,
                    sub.value(),
                    tracker.bar as i64,
                );
                refresh(&mut tracker, &mut popup_rects, &mut popup_outer);
            }
            Action::Redraw => refresh(&mut tracker, &mut popup_rects, &mut popup_outer),
            Action::None => (),
        }
    }
    *HOT.lock() = None;
    redraw_window(h);
    msg::send(h.value(), wm::EXITMENULOOP, 0, 0);
    TRACKING.store(false, Ordering::Release);
    if let Some(id) = command {
        let _ = msg::post(h.value(), wm::COMMAND, u64::from(id), 0);
    }
}

pub(crate) fn track_mouse(h: Handle, screen: Point) {
    let hdc = paint::GetWindowDC(door_windows_abi::narrow(h.value()));
    let origin = with(|u| u.windows.get(h).map(|w| w.rect.top_left())).unwrap_or_default();
    let rects = bar_items(h, door_windows_abi::widen(hdc)).1;
    paint::ReleaseDC(door_windows_abi::narrow(h.value()), hdc);
    let p = Point::new(screen.x - origin.x, screen.y - origin.y);
    if let Some(i) = rects.iter().position(|r| r.contains(p)) {
        track(h, Some(i), true);
    }
}
pub(crate) fn track_keyboard(h: Handle, ch: u16) {
    if ch == 0 {
        track(h, None, false);
        return;
    }
    let target = with(|u| {
        let bar = u.windows.get(h).and_then(|w| u.menus.resolve(w.id))?;
        let c = door_windows_gui::geom::wide::fold(ch);
        u.menus
            .get(bar)?
            .items
            .iter()
            .position(|i| mnemonic(&i.text) == Some(c))
    });
    if let Some(i) = target {
        track(h, Some(i), true);
    }
}

use super::server;

/// TrackPopupMenu(Ex): popup menus are not shown (Doors draws menu bars
/// only); no item is chosen.
pub(crate) fn track_popup(_menu: usize, _flags: u32, _x: i32, _y: i32, _hwnd: usize) -> i32 {
    0
}
/// GetMenuItemInfoW: MENUITEMINFOW { cbSize, fMask, fType, fState, wID,
/// hSubMenu, hbmpChecked, hbmpUnchecked, dwItemData, dwTypeData, cch,
/// hbmpItem } for the fields fMask names.
pub(crate) fn item_info(menu: usize, item: u32, by_position: bool, info: usize) -> i32 {
    let info = door_windows_abi::widen(info);
    let Some(h) = menu_of(door_windows_abi::widen(menu)) else {
        return 0;
    };
    let Some((kind, state, id, text, submenu, data)) = with(|u| {
        u.menus.item(h, item, by_position).map(|i| {
            (
                i.kind,
                i.state,
                i.id,
                i.text.clone(),
                i.submenu.map_or(0, Handle::value),
                i.data,
            )
        })
    }) else {
        g::set_last_error(door_windows_gui::win32_error::INVALID_PARAMETER);
        return 0;
    };
    let p = core::mem::size_of::<usize>() as u64;
    let Some(mask) = g::read_u32(info + 4) else {
        return 0;
    };
    // MIIM_STATE 1, MIIM_ID 2, MIIM_SUBMENU 4, MIIM_DATA 0x20,
    // MIIM_TYPE 0x10, MIIM_STRING 0x40, MIIM_FTYPE 0x100.
    let at_submenu = 4 * 4 + 4 + if p == 8 { 4 } else { 0 };
    let at_data = at_submenu + 3 * p;
    let at_text = at_data + p;
    let at_cch = at_text + p;
    if mask & 0x110 != 0 {
        g::write_u32(info + 8, kind);
    }
    if mask & 1 != 0 {
        g::write_u32(info + 12, state);
    }
    if mask & 2 != 0 {
        g::write_u32(info + 16, id);
    }
    if mask & 4 != 0 {
        g::write_pointer(info + at_submenu, submenu);
    }
    if mask & 0x20 != 0 {
        g::write_pointer(info + at_data, data);
    }
    if mask & 0x50 != 0 {
        let buffer = g::read_pointer(info + at_text).unwrap_or(0);
        let capacity = g::read_u32(info + at_cch).unwrap_or(0) as usize;
        if buffer != 0 && capacity > 0 {
            g::write_wide(buffer, capacity, &text);
        }
        g::write_u32(info + at_cch, text.len() as u32);
    }
    1
}
