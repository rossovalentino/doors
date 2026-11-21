//! Standard controls: Button, Static, Edit (over the host-tested EDIT
//! model), ListBox and ScrollBar. They are ordinary window procedures of
//! user32 that applications may subclass; notifications reach the parent
//! through WM_COMMAND exactly as on Windows.
use super::{
    Control, GetStockObject, SelectObject, SetBkColor, SetBkMode, SetTextColor, g, hwnd, msg,
    paint, server, with, wnd,
};
use alloc::vec::Vec;
use door_windows_gui::edit::{Clip, Edit, Notify, Outcome};
use door_windows_gui::handles::Handle;
use door_windows_gui::keyboard::vk;
use door_windows_gui::metrics::SYS_COLORS;
use door_windows_gui::msg::{bm, bn, bs, color, dlgc, em, en, es, lb, lbn, lbs, ss, stn, wm, ws};
use door_windows_gui::{Point, Rect};

pub(crate) struct ListState {
    pub items: Vec<Vec<u16>>,
    pub data: Vec<u64>,
    pub selected: Option<usize>,
    pub top: usize,
}

const ITEM_HEIGHT: i32 = 18;

/// (style, id, parent, client size, font) of a control.
type Info = (u32, u64, Option<Handle>, (i32, i32), u64);
fn info(h: Handle) -> Option<Info> {
    with(|u| {
        let w = u.windows.get(h)?;
        Some((w.style, w.id, w.parent, w.client_size(), w.font))
    })
}

/// WM_COMMAND notification to the parent.
pub(crate) fn notify(h: Handle, code: u32) {
    if let Some((_, id, Some(parent), _, _)) = info(h) {
        msg::send(
            parent.value(),
            wm::COMMAND,
            super::makelong(id as u32, code),
            h.value() as i64,
        );
    }
}

fn redraw(h: Handle) {
    with(|u| {
        u.windows.invalidate(h, None, true);
        if let Some(t) = u.window_thread(h) {
            u.wake(t);
        }
    });
}

/// Select the control font (WM_SETFONT, else the dialog font) into a DC.
fn select_font(hdc: u64, font: u64) {
    let f = if font != 0 {
        font
    } else {
        unsafe {
            door_windows_abi::widen(GetStockObject(
                door_windows_gui::msg::stock::DEFAULT_GUI_FONT,
            ))
        }
    };
    unsafe { SelectObject(door_windows_abi::narrow(hdc), door_windows_abi::narrow(f)) };
}

/// Background brush from the parent's WM_CTLCOLOR* answer.
fn ctl_color(h: Handle, hdc: u64, message: u32) -> u64 {
    let parent = info(h).and_then(|i| i.2);
    match parent {
        Some(p) => msg::send(p.value(), message, hdc, h.value() as i64) as u64,
        None => paint::brush((color::BTNFACE + 1) as u64),
    }
}

fn common(h: Handle, message: u32, wparam: u64, lparam: i64) -> Option<i64> {
    match message {
        wm::SETFONT => {
            with(|u| {
                if let Some(w) = u.windows.get_mut(h) {
                    w.font = wparam;
                }
            });
            if lparam != 0 {
                redraw(h);
            }
            Some(0)
        }
        wm::GETFONT => Some(info(h).map_or(0, |i| i.4) as i64),
        wm::ENABLE => {
            redraw(h);
            Some(0)
        }
        _ => None,
    }
}

// ---- Button ------------------------------------------------------------------------------

fn button_state(h: Handle) -> (u32, bool, bool) {
    with(|u| match u.control(h) {
        Some(Control::Button {
            checked,
            pushed,
            focused,
        }) => (*checked, *pushed, *focused),
        _ => (0, false, false),
    })
}
fn set_button(h: Handle, f: impl FnOnce(&mut u32, &mut bool, &mut bool)) {
    with(|u| {
        if u.control(h).is_none() {
            u.controls.push((
                h,
                Control::Button {
                    checked: 0,
                    pushed: false,
                    focused: false,
                },
            ));
        }
        if let Some(Control::Button {
            checked,
            pushed,
            focused,
        }) = u.control(h)
        {
            f(checked, pushed, focused);
        }
    });
    redraw(h);
}

fn click(h: Handle) {
    let style = info(h).map_or(0, |i| i.0);
    match style & bs::TYPEMASK {
        bs::AUTOCHECKBOX => set_button(h, |c, _, _| *c = u32::from(*c == 0)),
        bs::AUTO3STATE => set_button(h, |c, _, _| *c = (*c + 1) % 3),
        bs::AUTORADIOBUTTON => {
            // Uncheck the other auto radio buttons of the group.
            let group = with(|u| {
                let parent = u.windows.get(h)?.parent?;
                let kids = u.windows.get(parent)?.children.clone();
                let at = kids.iter().position(|&k| k == h)?;
                let mut start = at;
                while start > 0
                    && u.windows
                        .get(kids[start])
                        .is_some_and(|w| w.style & ws::GROUP == 0)
                {
                    start -= 1;
                }
                let mut end = at + 1;
                while end < kids.len()
                    && u.windows
                        .get(kids[end])
                        .is_some_and(|w| w.style & ws::GROUP == 0)
                {
                    end += 1;
                }
                Some(kids[start..end].to_vec())
            })
            .unwrap_or_default();
            for k in group {
                if k != h && info(k).is_some_and(|i| i.0 & bs::TYPEMASK == bs::AUTORADIOBUTTON) {
                    set_button(k, |c, _, _| *c = 0);
                }
            }
            set_button(h, |c, _, _| *c = 1);
        }
        _ => (),
    }
    notify(h, bn::CLICKED);
}

pub(crate) extern "system" fn button_proc(
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
        if let Some(r) = common(h, message, wparam, lparam) {
            return r;
        }
        let style = info(h).map_or(0, |i| i.0);
        let kind = style & bs::TYPEMASK;
        match message {
            wm::CREATE => {
                set_button(h, |_, _, _| ());
                0
            }
            wm::GETDLGCODE => {
                (match kind {
                    bs::PUSHBUTTON => dlgc::UNDEFPUSHBUTTON | dlgc::BUTTON,
                    bs::DEFPUSHBUTTON => dlgc::DEFPUSHBUTTON | dlgc::BUTTON,
                    bs::RADIOBUTTON | bs::AUTORADIOBUTTON => dlgc::RADIOBUTTON | dlgc::BUTTON,
                    bs::GROUPBOX => dlgc::STATIC,
                    _ => dlgc::BUTTON,
                }) as i64
            }
            wm::LBUTTONDOWN | wm::LBUTTONDBLCLK => {
                wnd::SetCapture(door_windows_abi::narrow(hwnd_value));
                wnd::SetFocus(door_windows_abi::narrow(hwnd_value));
                set_button(h, |_, p, _| *p = true);
                0
            }
            wm::LBUTTONUP => {
                let (_, pushed, _) = button_state(h);
                wnd::ReleaseCapture();
                set_button(h, |_, p, _| *p = false);
                let p = Point::from_lparam(lparam as isize);
                let (w, hh) = info(h).map_or((0, 0), |i| i.3);
                if pushed && Rect::new(0, 0, w, hh).contains(p) {
                    click(h);
                }
                0
            }
            wm::KEYDOWN if wparam as u8 == vk::SPACE => {
                set_button(h, |_, p, _| *p = true);
                0
            }
            wm::KEYUP if wparam as u8 == vk::SPACE => {
                let (_, pushed, _) = button_state(h);
                set_button(h, |_, p, _| *p = false);
                if pushed {
                    click(h);
                }
                0
            }
            wm::SETFOCUS | wm::KILLFOCUS => {
                let on = message == wm::SETFOCUS;
                set_button(h, |_, _, f| *f = on);
                if style & bs::NOTIFY != 0 {
                    notify(h, if on { bn::SETFOCUS } else { bn::KILLFOCUS });
                }
                0
            }
            wm::SETTEXT => {
                let r = wnd::DefWindowProcW(
                    door_windows_abi::narrow(hwnd_value),
                    message,
                    door_windows_abi::narrow(wparam),
                    (lparam) as isize,
                );
                redraw(h);
                (r) as i64
            }
            _ if message == bm::CLICK => {
                msg::send(hwnd_value, wm::LBUTTONDOWN, 1, 0);
                msg::send(hwnd_value, wm::LBUTTONUP, 0, 0);
                0
            }
            _ if message == bm::GETCHECK => button_state(h).0 as i64,
            _ if message == bm::SETCHECK => {
                set_button(h, |c, _, _| *c = wparam as u32);
                0
            }
            _ if message == bm::GETSTATE => {
                let (c, p, f) = button_state(h);
                (c as usize | if p { 4 } else { 0 } | if f { 8 } else { 0 }) as i64
            }
            _ if message == bm::SETSTATE => {
                set_button(h, |_, p, _| *p = wparam != 0);
                0
            }
            _ if message == bm::SETSTYLE => {
                with(|u| {
                    if let Some(w) = u.windows.get_mut(h) {
                        w.style = (w.style & 0xffff_0000) | (wparam as u32 & 0xffff);
                    }
                });
                if lparam != 0 {
                    redraw(h);
                }
                0
            }
            wm::PAINT => {
                let mut ps = [0u8; super::structs::PAINTSTRUCT_BYTES];
                let hdc = paint::BeginPaint(
                    door_windows_abi::narrow(hwnd_value),
                    door_windows_abi::narrow(ps.as_mut_ptr() as u64),
                );
                if hdc != 0 {
                    paint_button(h, door_windows_abi::widen(hdc));
                    paint::EndPaint(
                        door_windows_abi::narrow(hwnd_value),
                        door_windows_abi::narrow(ps.as_ptr() as u64),
                    );
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

fn paint_button(h: Handle, hdc: u64) {
    let Some((style, _, _, (w, hh), font)) = info(h) else {
        return;
    };
    let (checked, pushed, focused) = button_state(h);
    let enabled = with(|u| u.windows.get(h).is_some_and(|w| w.enabled()));
    let text = wnd::text_of(h);
    select_font(hdc, font);
    unsafe { SetBkMode(door_windows_abi::narrow(hdc), 1) };
    let fg = SYS_COLORS[if enabled {
        color::BTNTEXT
    } else {
        color::GRAYTEXT
    } as usize];
    unsafe { SetTextColor(door_windows_abi::narrow(hdc), fg) };
    let r = Rect::new(0, 0, w, hh);
    let kind = style & bs::TYPEMASK;
    match kind {
        bs::PUSHBUTTON | bs::DEFPUSHBUTTON | bs::PUSHBOX => {
            let face = if pushed {
                color::LIGHT_3D
            } else {
                color::BTNFACE
            };
            paint::fill(hdc, r, (face + 1) as u64);
            let border = if kind == bs::DEFPUSHBUTTON || focused {
                color::HIGHLIGHT
            } else {
                color::BTNSHADOW
            };
            paint::frame(hdc, r, (border + 1) as u64);
            if kind == bs::DEFPUSHBUTTON {
                paint::frame(hdc, r.inflate(-1, -1), (border + 1) as u64);
            }
            let mut tr = r;
            paint::draw_text(hdc, &text, &mut tr, 0x25);
        }
        bs::GROUPBOX => {
            let brush = ctl_color(h, hdc, 0x0138);
            paint::fill(hdc, r, brush);
            paint::frame(hdc, Rect::new(0, 7, w, hh), (color::BTNSHADOW + 1) as u64);
            let (tw, th) = paint::text_width(hdc, &door_windows_gui::menu::display(&text).0);
            paint::fill(hdc, Rect::new(6, 0, 10 + tw, th), brush);
            let mut tr = Rect::new(8, 0, w, th);
            paint::draw_text(hdc, &text, &mut tr, 0);
        }
        _ => {
            let brush = ctl_color(h, hdc, 0x0138);
            paint::fill(hdc, r, brush);
            let box_ = Rect::from_size(0, (hh - 13) / 2, 13, 13);
            paint::fill(hdc, box_, (color::WINDOW + 1) as u64);
            paint::frame(hdc, box_, (color::BTNSHADOW + 1) as u64);
            if checked != 0 {
                let mark = box_.inflate(-3, -3);
                paint::fill(hdc, mark, (color::HIGHLIGHT + 1) as u64);
            }
            let mut tr = Rect::new(18, 0, w, hh);
            paint::draw_text(hdc, &text, &mut tr, 0x24);
        }
    }
}

// ---- Static --------------------------------------------------------------------------------

pub(crate) extern "system" fn static_proc(
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
        if let Some(r) = common(h, message, wparam, lparam) {
            return r;
        }
        let style = info(h).map_or(0, |i| i.0);
        match message {
            wm::GETDLGCODE => dlgc::STATIC as i64,
            wm::NCHITTEST if style & ss::NOTIFY == 0 => -1,
            wm::LBUTTONDOWN if style & ss::NOTIFY != 0 => {
                notify(h, stn::CLICKED);
                0
            }
            wm::LBUTTONDBLCLK if style & ss::NOTIFY != 0 => {
                notify(h, stn::DBLCLK);
                0
            }
            wm::SETTEXT => {
                let r = wnd::DefWindowProcW(
                    door_windows_abi::narrow(hwnd_value),
                    message,
                    door_windows_abi::narrow(wparam),
                    (lparam) as isize,
                );
                redraw(h);
                (r) as i64
            }
            wm::PAINT => {
                let mut ps = [0u8; super::structs::PAINTSTRUCT_BYTES];
                let hdc = paint::BeginPaint(
                    door_windows_abi::narrow(hwnd_value),
                    door_windows_abi::narrow(ps.as_mut_ptr() as u64),
                );
                if hdc != 0 {
                    let Some((_, _, _, (w, hh), font)) = info(h) else {
                        return 0;
                    };
                    select_font(door_windows_abi::widen(hdc), font);
                    let brush = ctl_color(h, door_windows_abi::widen(hdc), 0x0138);
                    let r = Rect::new(0, 0, w, hh);
                    match style & ss::TYPEMASK {
                        ss::BLACKRECT | ss::GRAYRECT | ss::WHITERECT => {
                            let c = [color::WINDOWFRAME, color::BTNSHADOW, color::WINDOW]
                                [((style & ss::TYPEMASK) - 4) as usize];
                            paint::fill(door_windows_abi::widen(hdc), r, (c + 1) as u64);
                        }
                        ss::ETCHEDHORZ
                        | ss::ETCHEDFRAME
                        | ss::BLACKFRAME
                        | ss::GRAYFRAME
                        | ss::WHITEFRAME => {
                            paint::fill(door_windows_abi::widen(hdc), r, brush);
                            paint::frame(
                                door_windows_abi::widen(hdc),
                                r,
                                (color::BTNSHADOW + 1) as u64,
                            );
                        }
                        kind => {
                            paint::fill(door_windows_abi::widen(hdc), r, brush);
                            unsafe {
                                SetBkMode(door_windows_abi::narrow(door_windows_abi::widen(hdc)), 1)
                            };
                            let enabled = with(|u| u.windows.get(h).is_some_and(|w| w.enabled()));
                            if !enabled {
                                unsafe {
                                    SetTextColor(
                                        door_windows_abi::narrow(door_windows_abi::widen(hdc)),
                                        SYS_COLORS[color::GRAYTEXT as usize],
                                    )
                                };
                            }
                            let align = match kind {
                                ss::CENTER => 0x01,
                                ss::RIGHT => 0x02,
                                _ => 0,
                            };
                            let wrap = if matches!(kind, ss::SIMPLE | ss::LEFTNOWORDWRAP) {
                                0x20
                            } else {
                                0x10
                            };
                            let prefix = if style & ss::NOPREFIX != 0 { 0x800 } else { 0 };
                            let mut tr = r;
                            paint::draw_text(
                                door_windows_abi::widen(hdc),
                                &wnd::text_of(h),
                                &mut tr,
                                align | wrap | prefix,
                            );
                        }
                    }
                    paint::EndPaint(
                        door_windows_abi::narrow(hwnd_value),
                        door_windows_abi::narrow(ps.as_ptr() as u64),
                    );
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

// ---- Edit ------------------------------------------------------------------------------------

fn with_edit<R>(h: Handle, f: impl FnOnce(&mut Edit) -> R) -> Option<R> {
    with(|u| match u.control(h) {
        Some(Control::Edit(e)) => Some(f(e)),
        _ => None,
    })
}
fn emit(h: Handle, o: &Outcome) {
    for n in &o.notify {
        notify(
            h,
            match n {
                Notify::Update => en::UPDATE,
                Notify::Change => en::CHANGE,
                Notify::MaxText => en::MAXTEXT,
            },
        );
    }
    if let Some(c) = o.clipboard {
        clipboard(h, c);
    }
    if !o.notify.is_empty() || o.moved || o.clipboard.is_some() {
        scroll_to_caret(h);
        redraw(h);
    }
}
fn clipboard(h: Handle, c: Clip) {
    match c {
        Clip::Copy | Clip::Cut => {
            let text = with_edit(h, |e| e.selected_text().to_vec()).unwrap_or_default();
            if !text.is_empty() {
                publish_text(h, &text);
                with(|u| u.clipboard = text);
            }
            if c == Clip::Cut {
                let o = with_edit(h, |e| e.replace_selection(&[], true)).unwrap_or_default();
                emit(h, &o);
            }
        }
        Clip::Paste => {
            let text = session_text().unwrap_or_else(|| with(|u| u.clipboard.clone()));
            let o = with_edit(h, |e| e.replace_selection(&text, true)).unwrap_or_default();
            emit(h, &o);
        }
    }
}
/// The session clipboard (door.clipboard, brokered by the window server) is
/// the Windows clipboard: copied text reaches native applications as UTF-8
/// [CF_UNICODETEXT], owned by the edit control's top-level window.
fn publish_text(h: Handle, text: &[u16]) {
    let owner = with(|u| {
        let root = u.windows.root(h);
        u.natives.iter().find(|n| n.hwnd == root).map(|n| n.id)
    })
    .filter(|id| *id != 0);
    let Some(owner) = owner else { return };
    let utf8: Vec<u8> = char::decode_utf16(text.iter().copied())
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect::<alloc::string::String>()
        .into_bytes();
    server::conn(|c| c.clipboard_set(owner, &[(door_service_protocol::clipboard::TEXT, &utf8)]));
}
/// Text on the session clipboard (None: unavailable, then the local copy).
fn session_text() -> Option<Vec<u16>> {
    server::conn(|c| {
        let info = c.clipboard_info().ok()?;
        let size = info.size(door_service_protocol::clipboard::TEXT) as usize;
        if size == 0 || size > 256 * 1024 {
            return None;
        }
        let mut bytes = alloc::vec![0u8; size];
        let n = c
            .clipboard_read_into(&info, door_service_protocol::clipboard::TEXT, &mut bytes)
            .ok()?;
        let text = core::str::from_utf8(&bytes[..n]).ok()?;
        Some(text.encode_utf16().collect())
    })
    .flatten()
}
fn line_height(h: Handle) -> i32 {
    let hdc = paint::GetDC(0);
    select_font(door_windows_abi::widen(hdc), info(h).map_or(0, |i| i.4));
    let (_, ht) = paint::text_width(door_windows_abi::widen(hdc), &[0x20]);
    paint::ReleaseDC(0, hdc);
    ht.max(1)
}
fn scroll_to_caret(h: Handle) {
    let rows = (info(h).map_or(0, |i| i.3.1) - 2) / line_height(h);
    with_edit(h, |e| {
        let line = e.line_from_char(e.caret());
        if line < e.first_line {
            e.first_line = line;
        } else if rows > 0 && line >= e.first_line + rows as usize {
            e.first_line = line + 1 - rows as usize;
        }
    });
}

/// Character index under a client point.
fn char_at(h: Handle, p: Point) -> usize {
    let lh = line_height(h);
    let Some((lines, text, first)) = with_edit(h, |e| (e.lines(), e.text().to_vec(), e.first_line))
    else {
        return 0;
    };
    let line = (first + (p.y.max(0) - 1).max(0) as usize / lh as usize).min(lines.len() - 1);
    let (s, end) = lines[line];
    let hdc = paint::GetDC(0);
    select_font(door_windows_abi::widen(hdc), info(h).map_or(0, |i| i.4));
    let mut adv = alloc::vec![0i32; end - s];
    if end > s {
        unsafe {
            super::DoorGdiTextAdvances(
                door_windows_abi::widen(hdc),
                text[s..].as_ptr() as u64,
                (end - s) as u32,
                adv.as_mut_ptr() as u64,
            )
        };
    }
    paint::ReleaseDC(0, hdc);
    let mut x = 2;
    for (i, a) in adv.iter().enumerate() {
        if p.x < x + a / 2 {
            return s + i;
        }
        x += a;
    }
    end
}

pub(crate) extern "system" fn edit_proc(
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
        if let Some(r) = common(h, message, wparam, lparam) {
            return r;
        }
        match message {
            wm::NCCREATE => {
                let style = info(h).map_or(0, |i| i.0);
                let mut model = Edit::new(style);
                if !model.multiline() {
                    model.limit = door_windows_gui::edit::DEFAULT_LIMIT;
                }
                let name = g::read_pointer(
                    lparam as u64
                        + door_windows_abi::layout::offset(
                            &super::structs::CREATESTRUCTW,
                            super::structs::create::NAME,
                        ) as u64,
                )
                .unwrap_or(0);
                if name >= 0x1_0000
                    && let Some(t) = g::read_wide(name, door_windows_gui::window::MAX_TEXT)
                {
                    model.set_text(&t);
                }
                with(|u| u.controls.push((h, Control::Edit(model))));
                1
            }
            wm::GETDLGCODE => {
                let multi = with_edit(h, |e| e.multiline()).unwrap_or(false);
                (dlgc::WANTCHARS
                    | dlgc::HASSETSEL
                    | dlgc::WANTARROWS
                    | if multi { dlgc::WANTALLKEYS } else { 0 }) as i64
            }
            wm::CHAR => {
                let o = with_edit(h, |e| e.char(wparam as u16)).unwrap_or_default();
                emit(h, &o);
                0
            }
            wm::KEYDOWN => {
                let (shift, ctrl, _) = with(|u| u.queues.key_state(super::tid()).modifiers());
                let o = with_edit(h, |e| e.key(wparam as u8, shift, ctrl)).unwrap_or_default();
                emit(h, &o);
                0
            }
            wm::GETTEXT => {
                let text = with_edit(h, |e| e.text().to_vec()).unwrap_or_default();
                g::write_wide(lparam as u64, wparam as usize, &text).map_or(0, |n| n as i64)
            }
            wm::GETTEXTLENGTH => with_edit(h, |e| e.len()).unwrap_or(0) as i64,
            wm::SETTEXT => {
                let text = if lparam == 0 {
                    Some(Vec::new())
                } else {
                    g::read_wide(lparam as u64, 0x7fff_fffe)
                };
                let Some(text) = text else { return 0 };
                let o = with_edit(h, |e| e.set_text(&text)).unwrap_or_default();
                with(|u| {
                    if let Some(w) = u.windows.get_mut(h) {
                        w.text = text;
                    }
                });
                emit(h, &o);
                redraw(h);
                1
            }
            wm::CUT => {
                clipboard(h, Clip::Cut);
                0
            }
            wm::COPY => {
                clipboard(h, Clip::Copy);
                0
            }
            wm::PASTE => {
                clipboard(h, Clip::Paste);
                0
            }
            wm::CLEAR => {
                let o = with_edit(h, |e| e.replace_selection(&[], true)).unwrap_or_default();
                emit(h, &o);
                0
            }
            wm::UNDO => {
                let o = with_edit(h, |e| e.undo()).unwrap_or_default();
                emit(h, &o);
                1
            }
            wm::SETFOCUS | wm::KILLFOCUS => {
                notify(
                    h,
                    if message == wm::SETFOCUS {
                        en::SETFOCUS
                    } else {
                        en::KILLFOCUS
                    },
                );
                redraw(h);
                0
            }
            wm::LBUTTONDOWN => {
                wnd::SetFocus(door_windows_abi::narrow(hwnd_value));
                wnd::SetCapture(door_windows_abi::narrow(hwnd_value));
                let at = char_at(h, Point::from_lparam(lparam as isize));
                with_edit(h, |e| e.set_selection(at as isize, at as isize));
                redraw(h);
                0
            }
            wm::MOUSEMOVE => {
                if super::wnd::GetCapture() == door_windows_abi::narrow(hwnd_value) {
                    let at = char_at(h, Point::from_lparam(lparam as isize));
                    with_edit(h, |e| {
                        let (s, en_) = e.selection();
                        let anchor = if e.caret() == s { en_ } else { s };
                        e.set_selection(anchor as isize, at as isize);
                    });
                    redraw(h);
                }
                0
            }
            wm::LBUTTONUP => {
                if super::wnd::GetCapture() == door_windows_abi::narrow(hwnd_value) {
                    wnd::ReleaseCapture();
                }
                0
            }
            wm::LBUTTONDBLCLK => {
                let o = with_edit(h, |e| {
                    let c = e.caret();
                    e.set_selection(c as isize, c as isize);
                    let mut o = e.key(vk::LEFT, false, true);
                    let start = e.caret();
                    o.notify.extend(e.key(vk::RIGHT, true, true).notify);
                    let end = e.caret();
                    e.set_selection(start as isize, end as isize);
                    o.moved = true;
                    o
                })
                .unwrap_or_default();
                emit(h, &o);
                0
            }
            wm::PAINT => {
                let mut ps = [0u8; super::structs::PAINTSTRUCT_BYTES];
                let hdc = paint::BeginPaint(
                    door_windows_abi::narrow(hwnd_value),
                    door_windows_abi::narrow(ps.as_mut_ptr() as u64),
                );
                if hdc != 0 {
                    paint_edit(h, door_windows_abi::widen(hdc));
                    paint::EndPaint(
                        door_windows_abi::narrow(hwnd_value),
                        door_windows_abi::narrow(ps.as_ptr() as u64),
                    );
                }
                0
            }
            _ if message == em::GETSEL => {
                let (s, e) = with_edit(h, |e| e.selection()).unwrap_or((0, 0));
                if wparam != 0 {
                    g::write_u32(wparam, s as u32);
                }
                if lparam != 0 {
                    g::write_u32(lparam as u64, e as u32);
                }
                if s > 0xffff || e > 0xffff {
                    -1
                } else {
                    (s | e << 16) as i64
                }
            }
            _ if message == em::SETSEL => {
                with_edit(h, |e| {
                    e.set_selection(wparam as i32 as isize, lparam as i32 as isize)
                });
                scroll_to_caret(h);
                redraw(h);
                0
            }
            _ if message == em::REPLACESEL => {
                let text = g::read_wide(lparam as u64, 0x7fff_fffe).unwrap_or_default();
                let o =
                    with_edit(h, |e| e.replace_selection(&text, wparam != 0)).unwrap_or_default();
                emit(h, &o);
                0
            }
            _ if message == em::GETLINECOUNT => {
                with_edit(h, |e| e.line_count()).unwrap_or(1) as i64
            }
            _ if message == em::LINEINDEX => {
                let line = if wparam as i64 as i32 == -1 {
                    None
                } else {
                    Some(wparam as usize)
                };
                with_edit(h, |e| e.line_index(line))
                    .flatten()
                    .map_or(-1, |v| v as i64)
            }
            _ if message == em::LINELENGTH => {
                let at = if wparam as i64 as i32 == -1 {
                    None
                } else {
                    Some(wparam as usize)
                };
                with_edit(h, |e| e.line_length(at)).unwrap_or(0) as i64
            }
            _ if message == em::LINEFROMCHAR => {
                let at = if wparam as i64 as i32 == -1 {
                    usize::MAX
                } else {
                    wparam as usize
                };
                with_edit(h, |e| e.line_from_char(at)).unwrap_or(0) as i64
            }
            _ if message == em::GETLINE => {
                let Some(cap) = g::read_u32(lparam as u64).map(|v| (v & 0xffff) as usize) else {
                    return 0;
                };
                let line =
                    with_edit(h, |e| e.line_text(wparam as usize).map(<[u16]>::to_vec)).flatten();
                match line {
                    Some(t) => {
                        let n = t.len().min(cap);
                        let bytes: Vec<u8> = t[..n].iter().flat_map(|u| u.to_le_bytes()).collect();
                        if g::write(lparam as u64, &bytes) {
                            n as i64
                        } else {
                            0
                        }
                    }
                    None => 0,
                }
            }
            _ if message == em::GETMODIFY => {
                i64::from(with_edit(h, |e| e.modified).unwrap_or(false))
            }
            _ if message == em::SETMODIFY => {
                with_edit(h, |e| e.modified = wparam != 0);
                0
            }
            _ if message == em::CANUNDO => {
                i64::from(with_edit(h, |e| e.can_undo()).unwrap_or(false))
            }
            _ if message == em::UNDO => {
                let o = with_edit(h, |e| e.undo()).unwrap_or_default();
                let ok = !o.notify.is_empty();
                emit(h, &o);
                i64::from(ok)
            }
            _ if message == em::EMPTYUNDOBUFFER => {
                with_edit(h, |e| e.empty_undo());
                0
            }
            _ if message == em::LIMITTEXT => {
                let limit = if wparam == 0 {
                    door_windows_gui::edit::MAX_LIMIT
                } else {
                    wparam as usize
                };
                with_edit(h, |e| e.limit = limit);
                0
            }
            _ if message == em::GETLIMITTEXT => with_edit(h, |e| e.limit).unwrap_or(0) as i64,
            _ if message == em::SETREADONLY => {
                with_edit(h, |e| {
                    if wparam != 0 {
                        e.style |= es::READONLY;
                    } else {
                        e.style &= !es::READONLY;
                    }
                });
                with(|u| {
                    if let Some(w) = u.windows.get_mut(h) {
                        if wparam != 0 {
                            w.style |= es::READONLY;
                        } else {
                            w.style &= !es::READONLY;
                        }
                    }
                });
                redraw(h);
                1
            }
            _ if message == em::GETFIRSTVISIBLELINE => {
                with_edit(h, |e| e.first_line).unwrap_or(0) as i64
            }
            _ if message == em::SCROLLCARET => {
                scroll_to_caret(h);
                redraw(h);
                1
            }
            _ if message == em::SETPASSWORDCHAR => {
                with_edit(h, |e| e.password_char = wparam as u16);
                redraw(h);
                0
            }
            _ if message == em::GETPASSWORDCHAR => {
                with_edit(h, |e| e.password_char).unwrap_or(0) as i64
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

fn paint_edit(h: Handle, hdc: u64) {
    let Some((style, _, _, (w, hh), font)) = info(h) else {
        return;
    };
    // A multiline edit without ES_AUTOHSCROLL wraps its lines to the
    // client width (soft wrap: painting only, as the text model's lines
    // stay the hard CR-LF lines).
    let wraps = style & door_windows_gui::msg::es::MULTILINE != 0
        && style & door_windows_gui::msg::es::AUTOHSCROLL == 0;
    select_font(hdc, font);
    let brush = ctl_color(h, hdc, 0x0133);
    paint::fill(hdc, Rect::new(0, 0, w, hh), brush);
    let focused = super::wnd::GetFocus() == door_windows_abi::narrow(h.value());
    let Some((lines, text, (s, e), first, caret, pwd)) = with_edit(h, |e| {
        (
            e.lines(),
            e.text().to_vec(),
            e.selection(),
            e.first_line,
            e.caret(),
            e.password_char,
        )
    }) else {
        return;
    };
    let lh = line_height(h);
    unsafe { SetBkMode(door_windows_abi::narrow(hdc), 1) };
    let mut y = 1;
    // Visual rows: each hard line, wrapped when the style asks for it.
    let mut rows: Vec<(usize, usize, usize)> = Vec::new();
    for (i, &(ls, le)) in lines.iter().enumerate().skip(first) {
        if !wraps || le == ls {
            rows.push((i, ls, le));
            continue;
        }
        let mut adv = alloc::vec![0i32; le - ls];
        unsafe {
            super::DoorGdiTextAdvances(
                hdc,
                text[ls..le].as_ptr() as u64,
                (le - ls) as u32,
                adv.as_mut_ptr() as u64,
            )
        };
        for (a, b) in door_windows_gui::edit::wrap_rows(&text[ls..le], &adv, (w - 4).max(1)) {
            rows.push((i, ls + a, ls + b));
        }
    }
    for &(i, ls, le) in &rows {
        if y > hh {
            break;
        }
        let shown: Vec<u16> = text[ls..le]
            .iter()
            .map(|&u| if pwd != 0 { pwd } else { u })
            .collect();
        let mut adv = alloc::vec![0i32; shown.len()];
        if !shown.is_empty() {
            unsafe {
                super::DoorGdiTextAdvances(
                    hdc,
                    shown.as_ptr() as u64,
                    shown.len() as u32,
                    adv.as_mut_ptr() as u64,
                )
            };
        }
        let x_of = |k: usize| 2 + adv[..k - ls].iter().sum::<i32>();
        let (sel_a, sel_b) = (s.clamp(ls, le), e.clamp(ls, le));
        if sel_a < sel_b && focused {
            paint::fill(
                hdc,
                Rect::new(x_of(sel_a), y, x_of(sel_b), y + lh),
                (color::HIGHLIGHT + 1) as u64,
            );
        }
        unsafe {
            SetTextColor(
                door_windows_abi::narrow(hdc),
                SYS_COLORS[color::WINDOWTEXT as usize],
            )
        };
        paint::text_out(hdc, 2, y, &shown);
        if sel_a < sel_b && focused {
            unsafe {
                SetTextColor(
                    door_windows_abi::narrow(hdc),
                    SYS_COLORS[color::HIGHLIGHTTEXT as usize],
                );
                SetBkColor(
                    door_windows_abi::narrow(hdc),
                    SYS_COLORS[color::HIGHLIGHT as usize],
                );
            }
            paint::text_out(hdc, x_of(sel_a), y, &shown[sel_a - ls..sel_b - ls]);
        }
        if focused
            && s == e
            && caret >= ls
            && caret <= le
            // At a soft break the caret belongs to the next row.
            && (caret < le || le == lines[i].1)
        {
            paint::fill(
                hdc,
                Rect::new(x_of(caret), y, x_of(caret) + 1, y + lh),
                (color::WINDOWTEXT + 1) as u64,
            );
        }
        y += lh;
    }
}

// ---- ListBox -------------------------------------------------------------------------------

fn with_list<R>(h: Handle, f: impl FnOnce(&mut ListState) -> R) -> Option<R> {
    with(|u| {
        if u.control(h).is_none() {
            u.controls.push((
                h,
                Control::List(ListState {
                    items: Vec::new(),
                    data: Vec::new(),
                    selected: None,
                    top: 0,
                }),
            ));
        }
        match u.control(h) {
            Some(Control::List(l)) => Some(f(l)),
            _ => None,
        }
    })
}

fn insert_item(l: &mut ListState, at: Option<usize>, text: Vec<u16>, sorted: bool) -> i64 {
    if l.items.len() >= 32_768 {
        return lb::ERRSPACE as i64;
    }
    let index = match at {
        _ if sorted => l
            .items
            .iter()
            .position(|x| {
                door_windows_gui::geom::wide::to_string(x).to_lowercase()
                    > door_windows_gui::geom::wide::to_string(&text).to_lowercase()
            })
            .unwrap_or(l.items.len()),
        Some(i) if i <= l.items.len() => i,
        Some(_) => return lb::ERR as i64,
        None => l.items.len(),
    };
    l.items.insert(index, text);
    l.data.insert(index, 0);
    index as i64
}

pub(crate) extern "system" fn listbox_proc(
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
        if let Some(r) = common(h, message, wparam, lparam) {
            return r;
        }
        let style = info(h).map_or(0, |i| i.0);
        let result = match message {
            wm::CREATE => {
                with_list(h, |_| ());
                0
            }
            wm::GETDLGCODE => (dlgc::WANTARROWS | dlgc::WANTCHARS) as i64,
            _ if message == lb::ADDSTRING || message == lb::INSERTSTRING => {
                let text = g::read_wide(lparam as u64, 4096).unwrap_or_default();
                let at = if message == lb::INSERTSTRING && wparam as i32 >= 0 {
                    Some(wparam as usize)
                } else {
                    None
                };
                with_list(h, |l| {
                    insert_item(
                        l,
                        at,
                        text,
                        message == lb::ADDSTRING && style & lbs::SORT != 0,
                    )
                })
                .unwrap_or(lb::ERR as i64)
            }
            _ if message == lb::DELETESTRING => with_list(h, |l| {
                let i = wparam as usize;
                if i >= l.items.len() {
                    return lb::ERR as i64;
                }
                l.items.remove(i);
                l.data.remove(i);
                if l.selected.is_some_and(|s| s >= l.items.len()) {
                    l.selected = None;
                }
                l.items.len() as i64
            })
            .unwrap_or(lb::ERR as i64),
            _ if message == lb::RESETCONTENT => {
                with_list(h, |l| {
                    l.items.clear();
                    l.data.clear();
                    l.selected = None;
                    l.top = 0;
                });
                0
            }
            _ if message == lb::GETCOUNT => with_list(h, |l| l.items.len() as i64).unwrap_or(0),
            _ if message == lb::GETCURSEL => {
                with_list(h, |l| l.selected.map_or(lb::ERR as i64, |s| s as i64))
                    .unwrap_or(lb::ERR as i64)
            }
            _ if message == lb::SETCURSEL => with_list(h, |l| {
                let i = wparam as i32;
                if i < 0 || i as usize >= l.items.len() {
                    l.selected = None;
                    return lb::ERR as i64;
                }
                l.selected = Some(i as usize);
                i as i64
            })
            .unwrap_or(lb::ERR as i64),
            _ if message == lb::GETTEXTLEN => with_list(h, |l| {
                l.items
                    .get(wparam as usize)
                    .map_or(lb::ERR as i64, |t| t.len() as i64)
            })
            .unwrap_or(lb::ERR as i64),
            _ if message == lb::GETTEXT => {
                let t = with_list(h, |l| l.items.get(wparam as usize).cloned()).flatten();
                match t {
                    Some(t) => {
                        let bytes: Vec<u8> = t
                            .iter()
                            .chain(core::iter::once(&0))
                            .flat_map(|u| u.to_le_bytes())
                            .collect();
                        if g::write(lparam as u64, &bytes) {
                            t.len() as i64
                        } else {
                            lb::ERR as i64
                        }
                    }
                    None => lb::ERR as i64,
                }
            }
            _ if message == lb::GETITEMDATA => with_list(h, |l| {
                l.data
                    .get(wparam as usize)
                    .map_or(lb::ERR as i64, |d| *d as i64)
            })
            .unwrap_or(lb::ERR as i64),
            _ if message == lb::SETITEMDATA => {
                with_list(h, |l| match l.data.get_mut(wparam as usize) {
                    Some(d) => {
                        *d = lparam as u64;
                        0
                    }
                    None => lb::ERR as i64,
                })
                .unwrap_or(lb::ERR as i64)
            }
            _ if message == lb::GETTOPINDEX => with_list(h, |l| l.top as i64).unwrap_or(0),
            _ if message == lb::GETITEMHEIGHT => ITEM_HEIGHT as i64,
            wm::LBUTTONDOWN | wm::LBUTTONDBLCLK => {
                wnd::SetFocus(door_windows_abi::narrow(hwnd_value));
                let y = Point::from_lparam(lparam as isize).y;
                let changed = with_list(h, |l| {
                    let i = l.top + (y.max(0) / ITEM_HEIGHT) as usize;
                    if i < l.items.len() && l.selected != Some(i) {
                        l.selected = Some(i);
                        return true;
                    }
                    false
                })
                .unwrap_or(false);
                if style & lbs::NOTIFY != 0 {
                    if changed {
                        notify(h, lbn::SELCHANGE);
                    }
                    if message == wm::LBUTTONDBLCLK {
                        notify(h, lbn::DBLCLK);
                    }
                }
                0
            }
            wm::KEYDOWN => {
                let changed = with_list(h, |l| {
                    let n = l.items.len();
                    if n == 0 {
                        return false;
                    }
                    let cur = l.selected.unwrap_or(0);
                    let next = match wparam as u8 {
                        vk::UP => cur.saturating_sub(1),
                        vk::DOWN => (cur + 1).min(n - 1),
                        vk::HOME => 0,
                        vk::END => n - 1,
                        _ => return false,
                    };
                    let changed = l.selected != Some(next);
                    l.selected = Some(next);
                    changed
                })
                .unwrap_or(false);
                if changed && style & lbs::NOTIFY != 0 {
                    notify(h, lbn::SELCHANGE);
                }
                0
            }
            wm::PAINT => {
                let mut ps = [0u8; super::structs::PAINTSTRUCT_BYTES];
                let hdc = paint::BeginPaint(
                    door_windows_abi::narrow(hwnd_value),
                    door_windows_abi::narrow(ps.as_mut_ptr() as u64),
                );
                if hdc != 0 {
                    let Some((_, _, _, (w, hh), font)) = info(h) else {
                        return 0;
                    };
                    select_font(door_windows_abi::widen(hdc), font);
                    let brush = ctl_color(h, door_windows_abi::widen(hdc), 0x0134);
                    paint::fill(door_windows_abi::widen(hdc), Rect::new(0, 0, w, hh), brush);
                    unsafe { SetBkMode(door_windows_abi::narrow(door_windows_abi::widen(hdc)), 1) };
                    let (items, sel, top) =
                        with_list(h, |l| (l.items.clone(), l.selected, l.top)).unwrap_or_default();
                    let mut y = 0;
                    for (i, t) in items.iter().enumerate().skip(top) {
                        if y > hh {
                            break;
                        }
                        let selected = sel == Some(i);
                        if selected {
                            paint::fill(
                                door_windows_abi::widen(hdc),
                                Rect::new(0, y, w, y + ITEM_HEIGHT),
                                (color::HIGHLIGHT + 1) as u64,
                            );
                        }
                        let c = if selected {
                            color::HIGHLIGHTTEXT
                        } else {
                            color::WINDOWTEXT
                        };
                        unsafe {
                            SetTextColor(
                                door_windows_abi::narrow(door_windows_abi::widen(hdc)),
                                SYS_COLORS[c as usize],
                            )
                        };
                        let mut r = Rect::new(3, y, w, y + ITEM_HEIGHT);
                        paint::draw_text(door_windows_abi::widen(hdc), t, &mut r, 0x24 | 0x800);
                        y += ITEM_HEIGHT;
                    }
                    paint::EndPaint(
                        door_windows_abi::narrow(hwnd_value),
                        door_windows_abi::narrow(ps.as_ptr() as u64),
                    );
                }
                return 0;
            }
            _ => {
                return (wnd::DefWindowProcW(
                    door_windows_abi::narrow(hwnd_value),
                    message,
                    door_windows_abi::narrow(wparam),
                    (lparam) as isize,
                )) as i64;
            }
        };
        if !matches!(message, wm::GETDLGCODE) && message != lb::GETCOUNT && message != lb::GETCURSEL
        {
            redraw(h);
        }
        result
    })()) as isize
}

// ---- ScrollBar (drawing only; SBS_* scrolling is not implemented) ----------------------------

pub(crate) extern "system" fn scrollbar_proc(
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
        if message == wm::PAINT {
            let mut ps = [0u8; super::structs::PAINTSTRUCT_BYTES];
            let hdc = paint::BeginPaint(
                door_windows_abi::narrow(hwnd_value),
                door_windows_abi::narrow(ps.as_mut_ptr() as u64),
            );
            if hdc != 0 {
                let (w, hh) = info(h).map_or((0, 0), |i| i.3);
                paint::fill(
                    door_windows_abi::widen(hdc),
                    Rect::new(0, 0, w, hh),
                    (color::SCROLLBAR + 1) as u64,
                );
                paint::EndPaint(
                    door_windows_abi::narrow(hwnd_value),
                    door_windows_abi::narrow(ps.as_ptr() as u64),
                );
            }
            return 0;
        }
        (wnd::DefWindowProcW(
            door_windows_abi::narrow(hwnd_value),
            message,
            door_windows_abi::narrow(wparam),
            (lparam) as isize,
        )) as i64
    })()) as isize
}
