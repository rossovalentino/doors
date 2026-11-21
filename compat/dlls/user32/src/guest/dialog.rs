//! Dialogs: the #32770 class and DefDlgProcW, template-driven creation
//! (DIALOG/DIALOGEX from resources or memory), modal loops with disabled
//! owners, IsDialogMessageW keyboard navigation, dialog-item helpers and
//! MessageBoxW.
use super::{
    CreateFontIndirectW, DialogState, GetStockObject, GetTextMetricsW, SelectObject, call, g, hwnd,
    hwnd_or_error, menu, msg, paint, tid, with, wnd,
};
use alloc::vec::Vec;
use door_windows_gui::dlgnav::{self, Control as NavControl, Nav};
use door_windows_gui::handles::Handle;
use door_windows_gui::msg::{bm, bn, dm, ds, id, mb, sw, wm, ws, ws_ex};
use door_windows_gui::resource::{DialogTemplate, NameOrId, control_class};
use door_windows_gui::units::{DialogUnits, mul_div};
use door_windows_gui::{Rect, win32_error as e};

/// DLGWINDOWEXTRA: DWLP_MSGRESULT (0), DWLP_DLGPROC (after the LRESULT)
/// and DWLP_USER (after the DLGPROC): 0, 8 and 16 on x64, 0, 4 and 8 on x86
/// (P17), each a pointer wide.
pub(crate) const DLGWINDOWEXTRA: u32 = 30;
const DWLP_MSGRESULT: i32 = 0;
const DWLP_DLGPROC: i32 = super::structs::LONG_PTR as i32;
const DC_HASDEFID: u32 = 0x534b;

fn fail<T: Default>(code: u32) -> T {
    g::set_last_error(code);
    T::default()
}

fn dlgproc_of(h: Handle) -> u64 {
    with(|u| {
        u.windows
            .get(h)
            .filter(|w| w.extra.len() >= 2 * super::structs::LONG_PTR)
            .map_or(0, |w| {
                super::structs::get(
                    &w.extra[DWLP_DLGPROC as usize..],
                    &[door_windows_abi::layout::Field::Pointer],
                    0,
                )
            })
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn DefDlgProcW(
    hwnd_value: usize,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    let wparam = door_windows_abi::widen(wparam);
    let lparam = lparam as i64;
    ((move || -> i64 {
        let Some(h) = hwnd(hwnd_value) else { return 0 };
        let procedure = dlgproc_of(h);
        if procedure != 0 {
            wnd::SetWindowLongPtrW(door_windows_abi::narrow(hwnd_value), DWLP_MSGRESULT, 0);
            let r = call(procedure, hwnd_value, message, wparam, lparam);
            if r != 0 {
                // Messages whose result the dialog procedure returns directly.
                return if matches!(
                    message,
                    wm::INITDIALOG | 0x0132..=0x0138 | 0x002e | 0x002f | 0x0039 | 0x0037
                ) {
                    r
                } else {
                    wnd::GetWindowLongPtrW(door_windows_abi::narrow(hwnd_value), DWLP_MSGRESULT)
                        as i64
                };
            }
        }
        match message {
            wm::INITDIALOG => 0,
            wm::CLOSE => {
                let cancel = GetDlgItem(door_windows_abi::narrow(hwnd_value), id::CANCEL as i32);
                msg::send(
                    hwnd_value,
                    wm::COMMAND,
                    u64::from(id::CANCEL),
                    cancel as i64,
                );
                0
            }
            _ if message == dm::GETDEFID => {
                let d = with(|u| u.dialog(h).map_or(0, |d| d.default_id));
                if d == 0 {
                    0
                } else {
                    (d | DC_HASDEFID << 16) as i64
                }
            }
            _ if message == dm::SETDEFID => {
                with(|u| {
                    if let Some(d) = u.dialog(h) {
                        d.default_id = wparam as u32;
                    }
                });
                1
            }
            wm::ACTIVATE => {
                if wparam & 0xffff != 0 {
                    // A control that already has the focus (set by WM_INITDIALOG or
                    // the user) keeps it; otherwise the saved one, else the first
                    // tab stop, as Windows restores a dialog's focus.
                    let me = tid();
                    let inside = with(|u| {
                        u.thread(me)
                            .focus
                            .is_some_and(|f| f != h && u.windows.is_descendant(h, f))
                    });
                    let saved = with(|u| u.dialog(h).and_then(|d| d.focus_saved));
                    let target = saved
                        .filter(|f| hwnd(f.value()).is_some())
                        .or_else(|| first_tab(h));
                    if !inside && let Some(t) = target {
                        wnd::set_focus(Some(t));
                    }
                } else {
                    let me = tid();
                    let focus = with(|u| u.thread(me).focus);
                    with(|u| {
                        if let Some(d) = u.dialog(h) {
                            d.focus_saved = focus;
                        }
                    });
                }
                0
            }
            wm::SETFOCUS => {
                let saved = with(|u| u.dialog(h).and_then(|d| d.focus_saved));
                if let Some(t) = saved
                    .filter(|f| hwnd(f.value()).is_some())
                    .or_else(|| first_tab(h))
                {
                    wnd::set_focus(Some(t));
                }
                0
            }
            wm::NEXTDLGCTL => {
                let target = if lparam & 0xffff != 0 {
                    hwnd(wparam)
                } else {
                    let me = tid();
                    let focus = with(|u| u.thread(me).focus);
                    let controls = nav_controls(h);
                    let current = focus.and_then(|f| controls.iter().position(|(c, _)| *c == f));
                    let list: Vec<NavControl> = controls.iter().map(|(_, c)| *c).collect();
                    dlgnav::next_tab(&list, current, wparam != 0).map(|i| controls[i].0)
                };
                if let Some(t) = target {
                    wnd::set_focus(Some(t));
                }
                0
            }
            _ => {
                (wnd::DefWindowProcW(
                    door_windows_abi::narrow(hwnd_value),
                    message,
                    door_windows_abi::narrow(wparam),
                    (lparam) as isize,
                )) as i64
            }
        }
    })()) as isize
}

fn first_tab(h: Handle) -> Option<Handle> {
    let controls = nav_controls(h);
    let list: Vec<NavControl> = controls.iter().map(|(_, c)| *c).collect();
    dlgnav::first_tab(&list).map(|i| controls[i].0)
}

/// The dialog's direct children as navigation entries.
fn nav_controls(h: Handle) -> Vec<(Handle, NavControl)> {
    let kids = with(|u| u.windows.get(h).map(|w| w.children.clone())).unwrap_or_default();
    kids.into_iter()
        .filter_map(|c| {
            let (style, idv, visible, enabled, text) = with(|u| {
                let w = u.windows.get(c)?;
                Some((w.style, w.id, w.visible(), w.enabled(), w.text.clone()))
            })?;
            let code = msg::send(c.value(), wm::GETDLGCODE, 0, 0) as u32;
            Some((
                c,
                NavControl {
                    id: idv as u32,
                    style,
                    visible,
                    enabled,
                    code,
                    mnemonic: door_windows_gui::menu::mnemonic(&text),
                },
            ))
        })
        .collect()
}

#[unsafe(no_mangle)]
pub extern "system" fn IsDialogMessageW(dialog: usize, message: usize) -> i32 {
    let dialog = door_windows_abi::widen(dialog);
    let message = door_windows_abi::widen(message);
    {
        let (Some(d), Some(m)) = (hwnd(dialog), msg::read_msg(message)) else {
            return 0;
        };
        let Some(target) = hwnd(m.hwnd) else { return 0 };
        if target != d && !with(|u| u.windows.is_descendant(d, target)) {
            return 0;
        }
        let navigation = matches!(m.message, wm::KEYDOWN | wm::CHAR | wm::SYSCHAR);
        if navigation {
            let controls = nav_controls(d);
            let me = tid();
            let focus = with(|u| u.thread(me).focus);
            let current = focus.and_then(|f| {
                controls
                    .iter()
                    .position(|(c, _)| *c == f || with(|u| u.windows.is_descendant(*c, f)))
            });
            let list: Vec<NavControl> = controls.iter().map(|(_, c)| *c).collect();
            let shift = with(|u| {
                u.queues
                    .key_state(super::tid())
                    .down(door_windows_gui::keyboard::vk::SHIFT)
            });
            let default_id = (msg::send(dialog, dm::GETDEFID, 0, 0) & 0xffff) as u32;
            match dlgnav::translate(
                &list,
                current,
                m.message,
                m.wparam as u32,
                shift,
                default_id,
            ) {
                Nav::Pass => (),
                Nav::Handled => return 1,
                Nav::Focus(i) => {
                    let c = controls[i].0;
                    wnd::set_focus(Some(c));
                    if list[i].code & door_windows_gui::msg::dlgc::HASSETSEL != 0 {
                        msg::send(c.value(), door_windows_gui::msg::em::SETSEL, 0, -1);
                    }
                    return 1;
                }
                Nav::Click(i) => {
                    let c = controls[i].0;
                    wnd::set_focus(Some(c));
                    msg::send(c.value(), bm::CLICK, 0, 0);
                    return 1;
                }
                Nav::Command(idv) => {
                    let ctl = GetDlgItem(door_windows_abi::narrow(dialog), idv as i32);
                    msg::send(
                        dialog,
                        wm::COMMAND,
                        super::makelong(idv, bn::CLICKED),
                        ctl as i64,
                    );
                    return 1;
                }
            }
        }
        msg::TranslateMessage(door_windows_abi::narrow(message));
        msg::DispatchMessageW(door_windows_abi::narrow(message));
        1
    }
}

fn find_item(dialog: Handle, idv: i32) -> Option<Handle> {
    with(|u| {
        let kids = u.windows.get(dialog)?.children.clone();
        kids.iter()
            .copied()
            .find(|&c| u.windows.get(c).is_some_and(|w| w.id as i32 == idv))
            .or_else(|| {
                kids.iter().copied().find(|&c| {
                    u.windows
                        .get(c)
                        .is_some_and(|w| w.id & 0xffff == idv as u64 & 0xffff)
                })
            })
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn GetDlgItem(dialog: usize, idv: i32) -> usize {
    let dialog = door_windows_abi::widen(dialog);
    door_windows_abi::narrow((move || -> u64 {
        let Some(d) = hwnd_or_error(dialog) else {
            return 0;
        };
        find_item(d, idv).map_or_else(|| fail(1421), Handle::value)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetDlgItemTextW(dialog: usize, idv: i32, out: usize, capacity: i32) -> u32 {
    let dialog = door_windows_abi::widen(dialog);
    let out = door_windows_abi::widen(out);
    {
        let item = GetDlgItem(door_windows_abi::narrow(dialog), idv);
        if item == 0 {
            if capacity > 0 {
                g::write_wide(out, capacity as usize, &[]);
            }
            return 0;
        }
        wnd::GetWindowTextW(item, door_windows_abi::narrow(out), capacity) as u32
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SetDlgItemTextW(dialog: usize, idv: i32, text: usize) -> i32 {
    let dialog = door_windows_abi::widen(dialog);
    let text = door_windows_abi::widen(text);
    {
        let item = GetDlgItem(door_windows_abi::narrow(dialog), idv);
        if item == 0 {
            return 0;
        }
        wnd::SetWindowTextW(item, door_windows_abi::narrow(text))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn SendDlgItemMessageW(
    dialog: usize,
    idv: i32,
    message: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    let dialog = door_windows_abi::widen(dialog);
    let wparam = door_windows_abi::widen(wparam);
    let lparam = lparam as i64;
    ((move || -> i64 {
        let item = GetDlgItem(door_windows_abi::narrow(dialog), idv);
        if item == 0 {
            return 0;
        }
        (msg::SendMessageW(
            item,
            message,
            door_windows_abi::narrow(wparam),
            (lparam) as isize,
        )) as i64
    })()) as isize
}
#[unsafe(no_mangle)]
pub extern "system" fn CheckDlgButton(dialog: usize, idv: i32, check: u32) -> i32 {
    let dialog = door_windows_abi::widen(dialog);
    {
        let item = GetDlgItem(door_windows_abi::narrow(dialog), idv);
        if item == 0 {
            return 0;
        }
        msg::SendMessageW(
            item,
            bm::SETCHECK,
            door_windows_abi::narrow(u64::from(check)),
            0,
        );
        1
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn IsDlgButtonChecked(dialog: usize, idv: i32) -> u32 {
    let dialog = door_windows_abi::widen(dialog);
    {
        let item = GetDlgItem(door_windows_abi::narrow(dialog), idv);
        if item == 0 {
            return 0;
        }
        msg::SendMessageW(item, bm::GETCHECK, 0, 0) as u32
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn EndDialog(dialog: usize, result: isize) -> i32 {
    let dialog = door_windows_abi::widen(dialog);
    let result = result as i64;
    {
        let Some(d) = hwnd_or_error(dialog) else {
            return 0;
        };
        let modal = with(|u| {
            let s = u.dialog(d)?;
            s.ended = true;
            s.result = result;
            Some(s.modal)
        });
        match modal {
            Some(true) => {
                // The modal loop hides and destroys the dialog.
                if let Some(t) = with(|u| u.window_thread(d)) {
                    with(|u| u.wake(t));
                }
                1
            }
            Some(false) => {
                wnd::show(d, sw::HIDE);
                1
            }
            None => fail(e::INVALID_WINDOW_HANDLE),
        }
    }
}

/// System font for dialogs without DS_SETFONT, and dialog base units.
fn dialog_font(font: Option<&door_windows_gui::resource::DialogFont>) -> u64 {
    let Some(f) = font else {
        return unsafe {
            door_windows_abi::widen(GetStockObject(
                door_windows_gui::msg::stock::DEFAULT_GUI_FONT,
            ))
        };
    };
    let mut lf = [0u8; 92];
    let height = -mul_div(i32::from(f.point_size), 96, 72);
    lf[..4].copy_from_slice(&height.to_le_bytes());
    lf[16..20]
        .copy_from_slice(&i32::from(if f.weight == 0 { 400 } else { f.weight }).to_le_bytes());
    lf[20] = u8::from(f.italic);
    lf[23] = f.charset;
    for (i, u) in f.face.iter().take(31).enumerate() {
        lf[28 + 2 * i..30 + 2 * i].copy_from_slice(&u.to_le_bytes());
    }
    let h = unsafe { CreateFontIndirectW(door_windows_abi::narrow(lf.as_ptr() as u64)) };
    if h == 0 {
        unsafe {
            door_windows_abi::widen(GetStockObject(
                door_windows_gui::msg::stock::DEFAULT_GUI_FONT,
            ))
        }
    } else {
        door_windows_abi::widen(h)
    }
}
fn base_units(font: u64) -> DialogUnits {
    let hdc = paint::GetDC(0);
    unsafe {
        SelectObject(
            door_windows_abi::narrow(door_windows_abi::widen(hdc)),
            door_windows_abi::narrow(font),
        )
    };
    let sample: Vec<u16> = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz"
        .encode_utf16()
        .collect();
    let (w, _) = paint::text_width(door_windows_abi::widen(hdc), &sample);
    let mut tm = [0u8; 60];
    unsafe {
        GetTextMetricsW(
            door_windows_abi::narrow(door_windows_abi::widen(hdc)),
            door_windows_abi::narrow(tm.as_mut_ptr() as u64),
        )
    };
    paint::ReleaseDC(0, hdc);
    let height = i32::from_le_bytes(tm[..4].try_into().unwrap());
    DialogUnits {
        base_x: if w > 0 { (w / 26 + 1) / 2 } else { 7 },
        base_y: if height > 0 { height } else { 15 },
    }
}

/// Create a dialog from a parsed template (DialogBox*/CreateDialog*).
fn create_dialog(
    instance: u64,
    t: &DialogTemplate,
    owner: u64,
    procedure: u64,
    param: i64,
    modal: bool,
) -> Option<Handle> {
    let font = dialog_font(t.font.as_ref());
    let units = base_units(font);
    let client = units.map(Rect::from_size(0, 0, i32::from(t.cx), i32::from(t.cy)));
    let child = t.style & ws::CHILD != 0;
    let mut style = t.style & !ws::VISIBLE;
    if t.style & ds::MODALFRAME != 0 && !child {
        style |= ws::DLGFRAME;
    }
    let insets = door_windows_gui::metrics::insets(style, t.ex_style, false, child);
    let (w, h) = (
        client.width() + insets.left + insets.right,
        client.height() + insets.top + insets.bottom,
    );
    let owner_h = hwnd(owner);
    let (x, y) = {
        let at = units.map(Rect::from_size(i32::from(t.x), i32::from(t.y), 0, 0));
        match owner_h {
            Some(_) if child => (at.left, at.top),
            Some(o) if t.style & (ds::CENTER | ds::CENTERMOUSE) != 0 || (t.x == 0 && t.y == 0) => {
                let r = with(|u| u.windows.screen_rect(o));
                (r.left + (r.width() - w) / 2, r.top + (r.height() - h) / 2)
            }
            Some(o) => {
                let origin = with(|u| u.windows.client_origin(o));
                (origin.x + at.left, origin.y + at.top)
            }
            None => {
                let (sw_, sh) = with(|u| u.screen);
                (((sw_ - w) / 2).max(0), ((sh - h) / 3).max(0))
            }
        }
    };
    let class: Vec<u16> = match &t.class {
        NameOrId::Name(n) => n.iter().copied().chain(core::iter::once(0)).collect(),
        _ => "#32770\0".encode_utf16().collect(),
    };
    let title: Vec<u16> = t.title.iter().copied().chain(core::iter::once(0)).collect();
    let menu_handle = match &t.menu {
        NameOrId::None => 0,
        NameOrId::Id(m) => menu::LoadMenuW(
            door_windows_abi::narrow(instance),
            door_windows_abi::narrow(u64::from(*m)),
        ),
        NameOrId::Name(n) => {
            let z: Vec<u16> = n.iter().copied().chain(core::iter::once(0)).collect();
            menu::LoadMenuW(
                door_windows_abi::narrow(instance),
                door_windows_abi::narrow(z.as_ptr() as u64),
            )
        }
    };
    let hv = wnd::CreateWindowExW(
        t.ex_style
            | if t.style & ds::MODALFRAME != 0 {
                ws_ex::DLGMODALFRAME
            } else {
                0
            },
        door_windows_abi::narrow(class.as_ptr() as u64),
        door_windows_abi::narrow(title.as_ptr() as u64),
        style,
        x,
        y,
        w,
        h,
        door_windows_abi::narrow(owner),
        menu_handle,
        door_windows_abi::narrow(instance),
        door_windows_abi::narrow(param as u64),
    );
    let d = hwnd(door_windows_abi::widen(hv))?;
    with(|u| {
        u.dialogs.push((
            d,
            DialogState {
                ended: false,
                result: 0,
                default_id: 0,
                modal,
                focus_saved: None,
            },
        ));
    });
    wnd::SetWindowLongPtrW(hv, DWLP_DLGPROC, door_windows_abi::narrow(procedure));
    msg::send(door_windows_abi::widen(hv), wm::SETFONT, font, 0);
    for item in &t.items {
        let class_name: Vec<u16> = match &item.class {
            NameOrId::Id(o) => control_class(*o)
                .unwrap_or("Static")
                .encode_utf16()
                .chain(core::iter::once(0))
                .collect(),
            NameOrId::Name(n) => n.iter().copied().chain(core::iter::once(0)).collect(),
            NameOrId::None => "Static\0".encode_utf16().collect(),
        };
        let text: Vec<u16> = match &item.title {
            NameOrId::Name(n) => n.iter().copied().chain(core::iter::once(0)).collect(),
            _ => alloc::vec![0],
        };
        let r = units.map(Rect::from_size(
            i32::from(item.x),
            i32::from(item.y),
            i32::from(item.cx),
            i32::from(item.cy),
        ));
        let data = if item.extra.is_empty() {
            0
        } else {
            item.extra.as_ptr() as u64
        };
        let c = wnd::CreateWindowExW(
            item.ex_style | ws_ex::NOPARENTNOTIFY,
            door_windows_abi::narrow(class_name.as_ptr() as u64),
            door_windows_abi::narrow(text.as_ptr() as u64),
            item.style | ws::CHILD,
            r.left,
            r.top,
            r.width(),
            r.height(),
            hv,
            door_windows_abi::narrow(u64::from(item.id)),
            door_windows_abi::narrow(instance),
            door_windows_abi::narrow(data),
        );
        if c == 0 {
            continue;
        }
        msg::send(door_windows_abi::widen(c), wm::SETFONT, font, 0);
        if item.style & 0xf == door_windows_gui::msg::bs::DEFPUSHBUTTON
            && matches!(&item.class, NameOrId::Id(0x80))
        {
            with(|u| {
                if let Some(s) = u.dialog(d) {
                    s.default_id = item.id;
                }
            });
        }
    }
    let focus = first_tab(d);
    let set = msg::send(
        door_windows_abi::widen(hv),
        wm::INITDIALOG,
        focus.map_or(0, Handle::value),
        param,
    );
    hwnd(door_windows_abi::widen(hv))?;
    if set != 0
        && let Some(f) = focus
    {
        wnd::set_focus(Some(f));
    }
    if t.style & ws::VISIBLE != 0 && !modal {
        wnd::show(d, sw::SHOWNORMAL);
    }
    Some(d)
}

/// Run a modal dialog to completion and destroy it.
fn run_modal(d: Handle, owner: u64) -> i64 {
    let me = tid();
    let owner_h = hwnd(owner).map(|o| with(|u| u.windows.root(o)));
    let owner_was_enabled =
        owner_h.is_some_and(|o| with(|u| u.windows.get(o).is_some_and(|w| w.enabled())));
    if let Some(o) = owner_h.filter(|_| owner_was_enabled) {
        wnd::EnableWindow(door_windows_abi::narrow(o.value()), 0);
    }
    wnd::show(d, sw::SHOWNORMAL);
    // The window service refused the dialog (per-process window quota):
    // fail with -1 and give the owner back, never a hidden modal loop over a
    // disabled owner.
    if hwnd(d.value()).is_some() && !super::server::has_native(d) {
        if let Some(o) = owner_h.filter(|_| owner_was_enabled) {
            wnd::EnableWindow(door_windows_abi::narrow(o.value()), 1);
        }
        wnd::DestroyWindow(door_windows_abi::narrow(d.value()));
        g::set_last_error(door_windows_gui::win32_error::NOT_ENOUGH_QUOTA);
        return -1;
    }
    paint::update_now(d);
    let mut quit = None;
    loop {
        let ended = with(|u| u.dialog(d).is_none_or(|s| s.ended));
        if ended || hwnd(d.value()).is_none() {
            break;
        }
        super::server::pump();
        msg::process_pending(me);
        match msg::retrieve(me, 0, 0, 0, true) {
            Ok(Some(m)) => {
                if m.message == wm::QUIT {
                    quit = Some(m.wparam as i32);
                    break;
                }
                let b = msg::msg_bytes(&m);
                if IsDialogMessageW(
                    door_windows_abi::narrow(d.value()),
                    door_windows_abi::narrow(b.as_ptr() as u64),
                ) == 0
                {
                    msg::TranslateMessage(door_windows_abi::narrow(b.as_ptr() as u64));
                    msg::DispatchMessageW(door_windows_abi::narrow(b.as_ptr() as u64));
                }
            }
            Ok(None) => {
                let due = with(|u| u.queues.next_due(me)).map_or(8, |t| t.saturating_sub(g::now()));
                msg::idle_wait(me, due);
            }
            Err(_) => break,
        }
    }
    let result = with(|u| u.dialog(d).map_or(-1, |s| s.result));
    if let Some(o) = owner_h.filter(|_| owner_was_enabled) {
        wnd::EnableWindow(door_windows_abi::narrow(o.value()), 1);
        wnd::SetActiveWindow(door_windows_abi::narrow(o.value()));
        wnd::SetForegroundWindow(door_windows_abi::narrow(o.value()));
    }
    if hwnd(d.value()).is_some() {
        wnd::DestroyWindow(door_windows_abi::narrow(d.value()));
    }
    if let Some(code) = quit {
        msg::PostQuitMessage(code);
    }
    result
}

fn template_from(source: u64) -> Option<DialogTemplate> {
    let mut bytes = Vec::new();
    for chunk in 0..64u64 {
        let mut b = [0u8; 1024];
        if !g::read(source + chunk * 1024, &mut b) {
            // The last page may end early: retry the remainder bytewise.
            for i in 0..1024u64 {
                let mut one = [0u8; 1];
                if !g::read(source + chunk * 1024 + i, &mut one) {
                    break;
                }
                bytes.push(one[0]);
            }
            return DialogTemplate::parse(&bytes).ok();
        }
        bytes.extend_from_slice(&b);
        if let Ok(t) = DialogTemplate::parse(&bytes) {
            return Some(t);
        }
    }
    None
}

#[unsafe(no_mangle)]
pub extern "system" fn DialogBoxIndirectParamW(
    instance: usize,
    template: usize,
    owner: usize,
    procedure: usize,
    param: isize,
) -> isize {
    let instance = door_windows_abi::widen(instance);
    let template = door_windows_abi::widen(template);
    let owner = door_windows_abi::widen(owner);
    let procedure = door_windows_abi::widen(procedure);
    let param = param as i64;
    ((move || -> i64 {
        let Some(t) = template_from(template) else {
            return fail::<i64>(e::INVALID_PARAMETER) - 1;
        };
        match create_dialog(instance, &t, owner, procedure, param, true) {
            Some(d) => run_modal(d, owner),
            None => -1,
        }
    })()) as isize
}
#[unsafe(no_mangle)]
pub extern "system" fn DialogBoxParamW(
    instance: usize,
    name: usize,
    owner: usize,
    procedure: usize,
    param: isize,
) -> isize {
    let instance = door_windows_abi::widen(instance);
    let name = door_windows_abi::widen(name);
    let owner = door_windows_abi::widen(owner);
    let procedure = door_windows_abi::widen(procedure);
    let param = param as i64;
    ((move || -> i64 {
        let Some(bytes) = menu::resource_bytes(instance, menu::RT_DIALOG, name) else {
            g::set_last_error(e::RESOURCE_NAME_NOT_FOUND);
            return -1;
        };
        let Ok(t) = DialogTemplate::parse(&bytes) else {
            g::set_last_error(e::INVALID_PARAMETER);
            return -1;
        };
        match create_dialog(instance, &t, owner, procedure, param, true) {
            Some(d) => run_modal(d, owner),
            None => -1,
        }
    })()) as isize
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateDialogIndirectParamW(
    instance: usize,
    template: usize,
    owner: usize,
    procedure: usize,
    param: isize,
) -> usize {
    let instance = door_windows_abi::widen(instance);
    let template = door_windows_abi::widen(template);
    let owner = door_windows_abi::widen(owner);
    let procedure = door_windows_abi::widen(procedure);
    let param = param as i64;
    door_windows_abi::narrow((move || -> u64 {
        let Some(t) = template_from(template) else {
            return fail(e::INVALID_PARAMETER);
        };
        create_dialog(instance, &t, owner, procedure, param, false).map_or(0, Handle::value)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateDialogParamW(
    instance: usize,
    name: usize,
    owner: usize,
    procedure: usize,
    param: isize,
) -> usize {
    let instance = door_windows_abi::widen(instance);
    let name = door_windows_abi::widen(name);
    let owner = door_windows_abi::widen(owner);
    let procedure = door_windows_abi::widen(procedure);
    let param = param as i64;
    door_windows_abi::narrow((move || -> u64 {
        let Some(bytes) = menu::resource_bytes(instance, menu::RT_DIALOG, name) else {
            return fail(e::RESOURCE_NAME_NOT_FOUND);
        };
        let Ok(t) = DialogTemplate::parse(&bytes) else {
            return fail(e::INVALID_PARAMETER);
        };
        create_dialog(instance, &t, owner, procedure, param, false).map_or(0, Handle::value)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetDialogBaseUnits() -> i32 {
    let u = base_units(unsafe {
        door_windows_abi::widen(GetStockObject(
            door_windows_gui::msg::stock::DEFAULT_GUI_FONT,
        ))
    });
    (u.base_x & 0xffff) | (u.base_y << 16)
}
#[unsafe(no_mangle)]
pub extern "system" fn MapDialogRect(dialog: usize, rect: usize) -> i32 {
    let dialog = door_windows_abi::widen(dialog);
    let rect = door_windows_abi::widen(rect);
    {
        let Some(d) = hwnd_or_error(dialog) else {
            return 0;
        };
        let font = with(|u| u.windows.get(d).map_or(0, |w| w.font));
        let units = base_units(if font == 0 {
            unsafe {
                door_windows_abi::widen(GetStockObject(
                    door_windows_gui::msg::stock::DEFAULT_GUI_FONT,
                ))
            }
        } else {
            font
        });
        let Some([l, t, r, b]) = g::read_i32s::<4>(rect) else {
            return fail(e::INVALID_PARAMETER);
        };
        let m = units.map(Rect::new(l, t, r, b));
        i32::from(g::write_i32s(rect, &[m.left, m.top, m.right, m.bottom]))
    }
}

// ---- MessageBoxW -------------------------------------------------------------------------

extern "system" fn message_box_proc(
    hwnd_value: usize,
    message: u32,
    wparam: usize,
    _lparam: isize,
) -> isize {
    let hwnd_value = door_windows_abi::widen(hwnd_value);
    let wparam = door_windows_abi::widen(wparam);
    ((move || -> i64 {
        match message {
            wm::INITDIALOG => 1,
            wm::COMMAND => {
                let idv = (wparam & 0xffff) as u32;
                let has = |x: u32| GetDlgItem(door_windows_abi::narrow(hwnd_value), x as i32) != 0;
                let result = if idv == id::CANCEL && !has(id::CANCEL) {
                    // Escape in a box without Cancel chooses its only button.
                    if has(id::OK) { id::OK } else { return 1 }
                } else if has(idv) {
                    idv
                } else {
                    return 0;
                };
                EndDialog(
                    door_windows_abi::narrow(hwnd_value),
                    (i64::from(result)) as isize,
                );
                1
            }
            _ => 0,
        }
    })()) as isize
}

fn button_set(kind: u32) -> &'static [(u32, &'static str)] {
    match kind & 0xf {
        mb::OKCANCEL => &[(id::OK, "OK"), (id::CANCEL, "Cancel")],
        mb::ABORTRETRYIGNORE => &[
            (id::ABORT, "&Abort"),
            (id::RETRY, "&Retry"),
            (id::IGNORE, "&Ignore"),
        ],
        mb::YESNOCANCEL => &[(id::YES, "&Yes"), (id::NO, "&No"), (id::CANCEL, "Cancel")],
        mb::YESNO => &[(id::YES, "&Yes"), (id::NO, "&No")],
        mb::RETRYCANCEL => &[(id::RETRY, "&Retry"), (id::CANCEL, "Cancel")],
        6 => &[
            (id::CANCEL, "Cancel"),
            (id::TRYAGAIN, "&Try Again"),
            (id::CONTINUE, "&Continue"),
        ],
        // As on Windows, the single OK button of MB_OK has the identifier
        // IDCANCEL (so Esc presses it); MessageBoxW still returns IDOK.
        _ => &[(id::CANCEL, "OK")],
    }
}

fn push_item(
    b: &mut Vec<u8>,
    style: u32,
    r: (i16, i16, i16, i16),
    idv: u32,
    class: u16,
    text: &[u16],
) {
    while !b.len().is_multiple_of(4) {
        b.push(0);
    }
    for v in [0u32, 0, style | ws::CHILD | ws::VISIBLE] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for v in [r.0, r.1, r.2, r.3] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&idv.to_le_bytes());
    b.extend_from_slice(&[0xff, 0xff]);
    b.extend_from_slice(&class.to_le_bytes());
    for u in text {
        b.extend_from_slice(&u.to_le_bytes());
    }
    b.extend_from_slice(&[0, 0, 0, 0]);
}

#[unsafe(no_mangle)]
pub extern "system" fn MessageBoxW(owner: usize, text: usize, caption: usize, kind: u32) -> i32 {
    let owner = door_windows_abi::widen(owner);
    let text = door_windows_abi::widen(text);
    let caption = door_windows_abi::widen(caption);
    {
        let saved = g::last_error();
        let body = if text == 0 {
            Some(Vec::new())
        } else {
            g::read_wide(text, 32768)
        };
        let title = if caption == 0 {
            Some("Error".encode_utf16().collect())
        } else {
            g::read_wide(caption, 1024)
        };
        let (Some(body), Some(title)) = (body, title) else {
            return fail(e::INVALID_PARAMETER);
        };
        // Measure the message in dialog units of the dialog font.
        let font = unsafe { GetStockObject(door_windows_gui::msg::stock::DEFAULT_GUI_FONT) };
        let units = base_units(door_windows_abi::widen(font));
        let hdc = paint::GetDC(0);
        unsafe { SelectObject(door_windows_abi::narrow(door_windows_abi::widen(hdc)), font) };
        let mut r = Rect::new(0, 0, 360, 0);
        paint::draw_text(door_windows_abi::widen(hdc), &body, &mut r, 0x410 | 0x800);
        paint::ReleaseDC(0, hdc);
        let to_dlu_x = |px: i32| (px * 4 / units.base_x.max(1)) as i16;
        let to_dlu_y = |px: i32| (px * 8 / units.base_y.max(1)) as i16;
        let buttons = button_set(kind);
        let text_w = to_dlu_x(r.width().max(60)) + 4;
        let text_h = to_dlu_y(r.height().max(units.base_y)) + 2;
        let buttons_w = buttons.len() as i16 * 54;
        let width = (text_w + 20).max(buttons_w + 14).min(400);
        let height = 10 + text_h + 10 + 14 + 8;
        let mut b = Vec::new();
        for v in [1u16, 0xffff] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let style = ws::POPUP | ws::CAPTION | ws::SYSMENU | ds::MODALFRAME | 0x40;
        for v in [0u32, 0, style] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&((buttons.len() + 1) as u16).to_le_bytes());
        for v in [0i16, 0, width, height] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&[0, 0, 0, 0]);
        for u in title.iter().take(255).chain(core::iter::once(&0)) {
            b.extend_from_slice(&u.to_le_bytes());
        }
        b.extend_from_slice(&9u16.to_le_bytes());
        b.extend_from_slice(&400u16.to_le_bytes());
        b.extend_from_slice(&[0, 1]);
        for u in "Segoe UI\0".encode_utf16() {
            b.extend_from_slice(&u.to_le_bytes());
        }
        push_item(
            &mut b,
            door_windows_gui::msg::ss::NOPREFIX,
            (10, 10, text_w, text_h),
            0xffff,
            0x82,
            &body,
        );
        let default = ((kind >> 8) & 0xf) as usize;
        let mut x = width - 7 - buttons_w;
        for (i, (idv, label)) in buttons.iter().enumerate() {
            let units16: Vec<u16> = label.encode_utf16().collect();
            let bstyle = if i == default.min(buttons.len() - 1) {
                door_windows_gui::msg::bs::DEFPUSHBUTTON
            } else {
                door_windows_gui::msg::bs::PUSHBUTTON
            } | ws::TABSTOP
                | if i == 0 { ws::GROUP } else { 0 };
            push_item(
                &mut b,
                bstyle,
                (x + 4, height - 22, 50, 14),
                *idv,
                0x80,
                &units16,
            );
            x += 54;
        }
        // The box sounds its icon's scheme event as it appears; without an
        // icon (or with MB_USERICON) the default beep.
        super::misc::beep(match kind & 0xf0 {
            icon @ (0x10 | 0x20 | 0x30 | 0x40) => icon,
            _ => 0,
        });
        let owner_root = hwnd(owner).map_or(0, |o| with(|u| u.windows.root(o)).value());
        let result = DialogBoxIndirectParamW(
            0,
            door_windows_abi::narrow(b.as_ptr() as u64),
            door_windows_abi::narrow(owner_root),
            door_windows_abi::narrow(message_box_proc as *const () as u64),
            0,
        );
        // The box could not be created (window quota): 0 with the error set.
        if result == -1 {
            return 0;
        }
        g::set_last_error(saved);
        if kind & 0xf == 0 && result == (i64::from(id::CANCEL)) as isize {
            return id::OK as i32;
        }
        result as i32
    }
}

const _: () = assert!(sw::SHOWNORMAL == 1);
