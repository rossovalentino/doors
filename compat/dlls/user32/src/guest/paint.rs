//! Painting: update regions, BeginPaint/EndPaint and GetDC on the top-level
//! back buffer (children paint into their top-level's surface through a DC
//! origin and visible region), FillRect, DrawTextW and band commits.
use super::structs;
use super::{
    DeleteDC, DoorGdiCreateDisplayDC, DoorGdiCreateSurfaceDC, DoorGdiFillRect,
    DoorGdiSysColorBrush, ExtTextOutW, GetTextExtentPoint32W, g, hwnd_or_error, msg, server, with,
};
use alloc::vec::Vec;
use door_windows_gui::handles::Handle;
use door_windows_gui::msg::{dt, wm};
use door_windows_gui::region::Region;
use door_windows_gui::{Point, Rect, win32_error as e};

/// Open paint/DCs: (hdc, top-level, surface rows touched).
static OPEN: g::Lock<Vec<(u64, Handle, i32, i32)>> = g::Lock::new(Vec::new());

/// Where a window's client (or whole window) lies on its top-level surface:
/// (back address, width, height, DC origin, visible rects).
struct Target {
    address: u64,
    width: u32,
    height: u32,
    origin: Point,
    visible: Vec<Rect>,
    top: Handle,
}

fn target(h: Handle, whole: bool, clip: Option<&Region>) -> Option<Target> {
    with(|u| {
        let top = u.windows.root(h);
        let n = u.natives.iter().find(|n| n.hwnd == top)?;
        let top_rect = u.windows.get(top)?.rect;
        let surface = Point::new(top_rect.left + n.surface_left, top_rect.top + n.surface_top);
        let (origin, region) = u.windows.surface_target(h, whole, surface, clip)?;
        Some(Target {
            address: n.back.address,
            width: n.back.width,
            height: n.back.height,
            origin,
            visible: region.rects().to_vec(),
            top,
        })
    })
}

fn make_dc(t: &Target) -> u64 {
    let bytes: Vec<u8> = t.visible.iter().flat_map(|r| r.to_bytes()).collect();
    // SAFETY: plain call into gdi32 with our own rectangle array.
    unsafe {
        DoorGdiCreateSurfaceDC(
            t.address,
            t.width,
            t.height,
            t.width,
            t.origin.x,
            t.origin.y,
            bytes.as_ptr() as u64,
            t.visible.len() as u32,
        )
    }
}

fn open_dc(h: Handle, whole: bool, clip: Option<&Region>) -> u64 {
    match target(h, whole, clip) {
        Some(t) => {
            let hdc = make_dc(&t);
            if hdc != 0 {
                let (lo, hi) = t.visible.iter().fold((i32::MAX, i32::MIN), |(a, b), r| {
                    (a.min(r.top), b.max(r.bottom))
                });
                OPEN.lock().push((hdc, t.top, lo, hi));
            }
            hdc
        }
        // Hidden or not yet backed: an information DC that draws nothing.
        None => unsafe { DoorGdiCreateDisplayDC() },
    }
}

/// Release a DC from `open_dc`, committing the rows it could touch.
fn close_dc(hdc: u64) -> bool {
    let entry = {
        let mut open = OPEN.lock();
        open.iter().position(|e| e.0 == hdc).map(|i| open.remove(i))
    };
    // SAFETY: a DC gdi32 created for us.
    let ok = unsafe { DeleteDC(door_windows_abi::narrow(hdc)) } != 0;
    if let Some((_, top, lo, hi)) = entry
        && lo < hi
    {
        server::present(top, Some((lo.max(0) as u32, hi.max(0) as u32)));
    }
    ok
}

/// PAINTSTRUCT: hdc, fErase, rcPaint, fRestore, fIncUpdate, reserved.
#[unsafe(no_mangle)]
pub extern "system" fn BeginPaint(hwnd: usize, out: usize) -> usize {
    let hwnd = door_windows_abi::widen(hwnd);
    let out = door_windows_abi::widen(out);
    door_windows_abi::narrow((move || -> u64 {
        let Some(h) = hwnd_or_error(hwnd) else {
            return 0;
        };
        let frame = with(|u| {
            u.windows
                .get_mut(h)
                .map(|w| core::mem::replace(&mut w.frame_dirty, false))
                .unwrap_or(false)
        });
        if frame {
            msg::send(hwnd, wm::NCPAINT, 1, 0);
        }
        let taken = with(|u| {
            let w = u.windows.get_mut(h)?;
            let update = core::mem::take(&mut w.update);
            let erase = core::mem::replace(&mut w.erase, false);
            Some((update, erase))
        });
        let Some((update, erase)) = taken else {
            return 0;
        };
        let bounds = update.bounds();
        let hdc = open_dc(h, false, Some(&update));
        if hdc == 0 {
            return 0;
        }
        let mut erased = true;
        if erase {
            erased = msg::send(hwnd, wm::ERASEBKGND, hdc, 0) != 0;
        }
        let mut ps = [0u8; structs::PAINTSTRUCT_BYTES];
        structs::set(&mut ps, &structs::PAINTSTRUCT, structs::paint::HDC, hdc);
        structs::set(
            &mut ps,
            &structs::PAINTSTRUCT,
            structs::paint::ERASE,
            u64::from(!erased),
        );
        for (i, v) in [bounds.left, bounds.top, bounds.right, bounds.bottom]
            .iter()
            .enumerate()
        {
            structs::set_i32(&mut ps, &structs::PAINTSTRUCT, structs::paint::LEFT + i, *v);
        }
        if !g::write(out, &ps) {
            close_dc(hdc);
            g::set_last_error(e::INVALID_PARAMETER);
            return 0;
        }
        hdc
    })())
}

#[unsafe(no_mangle)]
pub extern "system" fn EndPaint(_hwnd: usize, ps: usize) -> i32 {
    let ps = door_windows_abi::widen(ps);
    {
        let Some(hdc) = g::read_pointer(ps) else {
            g::set_last_error(e::INVALID_PARAMETER);
            return 0;
        };
        close_dc(hdc);
        1
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetDC(hwnd: usize) -> usize {
    let hwnd = door_windows_abi::widen(hwnd);
    door_windows_abi::narrow((move || -> u64 {
        if hwnd == 0 {
            return unsafe { DoorGdiCreateDisplayDC() };
        }
        let Some(h) = hwnd_or_error(hwnd) else {
            return 0;
        };
        open_dc(h, false, None)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn GetWindowDC(hwnd: usize) -> usize {
    let hwnd = door_windows_abi::widen(hwnd);
    door_windows_abi::narrow((move || -> u64 {
        if hwnd == 0 {
            return unsafe { DoorGdiCreateDisplayDC() };
        }
        let Some(h) = hwnd_or_error(hwnd) else {
            return 0;
        };
        open_dc(h, true, None)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn ReleaseDC(_hwnd: usize, hdc: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    i32::from(close_dc(hdc))
}

fn read_rect(p: u64) -> Option<Rect> {
    g::read_i32s::<4>(p).map(|[l, t, r, b]| Rect::new(l, t, r, b))
}

#[unsafe(no_mangle)]
pub extern "system" fn InvalidateRect(hwnd: usize, rect: usize, erase: i32) -> i32 {
    let hwnd = door_windows_abi::widen(hwnd);
    let rect = door_windows_abi::widen(rect);
    {
        let area = if rect == 0 {
            None
        } else {
            match read_rect(rect) {
                Some(r) => Some(r),
                None => {
                    g::set_last_error(e::INVALID_PARAMETER);
                    return 0;
                }
            }
        };
        if hwnd == 0 {
            with(|u| {
                for t in u.windows.top.clone() {
                    u.windows.invalidate(t, None, erase != 0);
                }
                let threads: Vec<u32> = u.threads.iter().map(|t| t.id).collect();
                for t in threads {
                    u.wake(t);
                }
            });
            return 1;
        }
        let Some(h) = hwnd_or_error(hwnd) else {
            return 0;
        };
        with(|u| {
            u.windows.invalidate(h, area, erase != 0);
            if let Some(t) = u.window_thread(h) {
                u.wake(t);
            }
        });
        1
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn ValidateRect(hwnd: usize, rect: usize) -> i32 {
    let hwnd = door_windows_abi::widen(hwnd);
    let rect = door_windows_abi::widen(rect);
    {
        let area = if rect == 0 { None } else { read_rect(rect) };
        let Some(h) = hwnd_or_error(hwnd) else {
            return 0;
        };
        with(|u| u.windows.validate(h, area));
        1
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetUpdateRect(hwnd: usize, rect: usize, erase: i32) -> i32 {
    let hwnd = door_windows_abi::widen(hwnd);
    let rect = door_windows_abi::widen(rect);
    {
        let Some(h) = hwnd_or_error(hwnd) else {
            return 0;
        };
        let b = with(|u| u.windows.get(h).map(|w| w.update.bounds())).unwrap_or(Rect::EMPTY);
        if rect != 0 && !g::write_i32s(rect, &[b.left, b.top, b.right, b.bottom]) {
            g::set_last_error(e::INVALID_PARAMETER);
            return 0;
        }
        let pending = !b.is_empty();
        if pending && erase != 0 {
            let hdc = open_dc(h, false, None);
            msg::send(hwnd, wm::ERASEBKGND, hdc, 0);
            close_dc(hdc);
            with(|u| {
                if let Some(w) = u.windows.get_mut(h) {
                    w.erase = false;
                }
            });
        }
        i32::from(pending)
    }
}

/// Paint `h` and its descendants now (UpdateWindow / RDW_UPDATENOW).
pub(crate) fn update_now(h: Handle) {
    let list = with(|u| u.windows.subtree(h));
    for x in list {
        if with(|u| u.windows.needs_paint(x) && u.windows.really_visible(x)) {
            msg::send(x.value(), wm::PAINT, 0, 0);
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn UpdateWindow(hwnd: usize) -> i32 {
    let hwnd = door_windows_abi::widen(hwnd);
    {
        let Some(h) = hwnd_or_error(hwnd) else {
            return 0;
        };
        update_now(h);
        1
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn RedrawWindow(hwnd: usize, rect: usize, _region: usize, flags: u32) -> i32 {
    let hwnd = door_windows_abi::widen(hwnd);
    let rect = door_windows_abi::widen(rect);
    {
        const RDW_INVALIDATE: u32 = 0x1;
        const RDW_ERASE: u32 = 0x4;
        const RDW_VALIDATE: u32 = 0x8;
        const RDW_FRAME: u32 = 0x400;
        const RDW_UPDATENOW: u32 = 0x100;
        let Some(h) = hwnd_or_error(hwnd) else {
            return 0;
        };
        let area = if rect == 0 { None } else { read_rect(rect) };
        with(|u| {
            if flags & RDW_INVALIDATE != 0 {
                u.windows.invalidate(h, area, flags & RDW_ERASE != 0);
            }
            if flags & RDW_VALIDATE != 0 {
                u.windows.validate(h, area);
            }
            if flags & RDW_FRAME != 0
                && let Some(w) = u.windows.get_mut(h)
            {
                w.frame_dirty = true;
            }
            if let Some(t) = u.window_thread(h) {
                u.wake(t);
            }
        });
        if flags & RDW_UPDATENOW != 0 {
            update_now(h);
        }
        1
    }
}

/// (COLOR_* + 1) values name system-color brushes.
pub(crate) fn brush(value: u64) -> u64 {
    if (1..=31).contains(&value) {
        unsafe { DoorGdiSysColorBrush(value as i32 - 1) }
    } else {
        value
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn FillRect(hdc: usize, rect: usize, hbrush: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let rect = door_windows_abi::widen(rect);
    let hbrush = door_windows_abi::widen(hbrush);
    unsafe { DoorGdiFillRect(hdc, rect, brush(hbrush)) }
}
/// FillRect over an owned rectangle (controls).
pub(crate) fn fill(hdc: u64, r: Rect, hbrush: u64) {
    let b = r.to_bytes();
    unsafe { DoorGdiFillRect(hdc, b.as_ptr() as u64, brush(hbrush)) };
}
pub(crate) fn frame(hdc: u64, r: Rect, hbrush: u64) {
    fill(hdc, Rect::new(r.left, r.top, r.right, r.top + 1), hbrush);
    fill(
        hdc,
        Rect::new(r.left, r.bottom - 1, r.right, r.bottom),
        hbrush,
    );
    fill(hdc, Rect::new(r.left, r.top, r.left + 1, r.bottom), hbrush);
    fill(
        hdc,
        Rect::new(r.right - 1, r.top, r.right, r.bottom),
        hbrush,
    );
}

#[unsafe(no_mangle)]
pub extern "system" fn FrameRect(hdc: usize, rect: usize, hbrush: usize) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let rect = door_windows_abi::widen(rect);
    let hbrush = door_windows_abi::widen(hbrush);
    {
        match read_rect(rect) {
            Some(r) => {
                frame(hdc, r, hbrush);
                1
            }
            None => 0,
        }
    }
}

/// Width of `text` in the DC's font.
pub(crate) fn text_width(hdc: u64, text: &[u16]) -> (i32, i32) {
    let mut size = [0i32; 2];
    unsafe {
        GetTextExtentPoint32W(
            door_windows_abi::narrow(hdc),
            door_windows_abi::narrow(text.as_ptr() as u64),
            text.len() as i32,
            door_windows_abi::narrow(size.as_mut_ptr() as u64),
        );
    }
    (size[0], size[1])
}
pub(crate) fn text_out(hdc: u64, x: i32, y: i32, text: &[u16]) {
    unsafe {
        ExtTextOutW(
            door_windows_abi::narrow(hdc),
            x,
            y,
            0,
            0,
            door_windows_abi::narrow(text.as_ptr() as u64),
            text.len() as u32,
            0,
        );
    }
}

/// DrawTextW over owned text; returns the text height.
pub(crate) fn draw_text(hdc: u64, text: &[u16], r: &mut Rect, format: u32) -> i32 {
    let prefix = format & dt::NOPREFIX == 0;
    let clean: Vec<u16> = if prefix {
        door_windows_gui::menu::display(text).0
    } else {
        text.to_vec()
    };
    let line_h = text_width(hdc, &[0x20]).1.max(1);
    // Break into lines: hard breaks, then words when DT_WORDBREAK.
    let mut lines: Vec<Vec<u16>> = Vec::new();
    let single = format & dt::SINGLELINE != 0;
    let width = r.width();
    for para in clean.split(|&u| u == 10 && !single) {
        let para: Vec<u16> = para.iter().copied().filter(|&u| u != 13).collect();
        if format & dt::WORDBREAK == 0 || single {
            lines.push(para);
            continue;
        }
        let mut cur: Vec<u16> = Vec::new();
        for word in para.split_inclusive(|&u| u == 32) {
            let mut trial = cur.clone();
            trial.extend_from_slice(word);
            if !cur.is_empty() && text_width(hdc, &trial).0 > width {
                lines.push(core::mem::take(&mut cur));
                cur.extend_from_slice(word);
            } else {
                cur = trial;
            }
        }
        lines.push(cur);
    }
    let total = line_h * lines.len() as i32;
    if format & dt::CALCRECT != 0 {
        let widest = lines
            .iter()
            .map(|l| text_width(hdc, l).0)
            .max()
            .unwrap_or(0);
        r.right = r.left + widest;
        r.bottom = r.top + total;
        return total;
    }
    let mut y = r.top;
    if single {
        if format & dt::VCENTER != 0 {
            y = r.top + (r.height() - line_h) / 2;
        } else if format & dt::BOTTOM != 0 {
            y = r.bottom - line_h;
        }
    }
    for l in &lines {
        let w = text_width(hdc, l).0;
        let x = if format & dt::CENTER != 0 {
            r.left + (width - w) / 2
        } else if format & dt::RIGHT != 0 {
            r.right - w
        } else {
            r.left
        };
        if format & dt::NOCLIP == 0 {
            let b = r.to_bytes();
            unsafe {
                ExtTextOutW(
                    door_windows_abi::narrow(hdc),
                    x,
                    y,
                    4,
                    door_windows_abi::narrow(b.as_ptr() as u64),
                    door_windows_abi::narrow(l.as_ptr() as u64),
                    l.len() as u32,
                    0,
                );
            }
        } else {
            text_out(hdc, x, y, l);
        }
        y += line_h;
    }
    total
}

#[unsafe(no_mangle)]
pub extern "system" fn DrawTextW(
    hdc: usize,
    text: usize,
    count: i32,
    rect: usize,
    format: u32,
) -> i32 {
    let hdc = door_windows_abi::widen(hdc);
    let text = door_windows_abi::widen(text);
    let rect = door_windows_abi::widen(rect);
    {
        let units = if count < 0 {
            g::read_wide(text, 32768)
        } else {
            g::read_wide_n(text, count as usize)
        };
        let (Some(units), Some(mut r)) = (units, read_rect(rect)) else {
            g::set_last_error(e::INVALID_PARAMETER);
            return 0;
        };
        let height = draw_text(hdc, &units, &mut r, format);
        if format & dt::CALCRECT != 0 {
            g::write_i32s(rect, &[r.left, r.top, r.right, r.bottom]);
        }
        height
    }
}
