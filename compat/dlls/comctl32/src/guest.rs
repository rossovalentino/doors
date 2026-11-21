//! PE exports of comctl32: InitCommonControlsEx, the status bar class and
//! image lists.
mod imagelist;
mod taskdialog;
use alloc::vec::Vec;
use door_windows_abi::layout::{self, Field, Field::*};
use door_windows_gui::Rect;
use door_windows_gui::msg::{color, wm};
use door_windows_gui_guest as g;

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "user32", kind = "dylib")]
unsafe extern "system" {
    fn RegisterClassExW(class: usize) -> u16;
    fn DefWindowProcW(hwnd: usize, message: u32, wparam: usize, lparam: isize) -> isize;
    fn SendMessageW(hwnd: usize, message: u32, wparam: usize, lparam: isize) -> isize;
    fn GetParent(hwnd: usize) -> usize;
    fn GetClientRect(hwnd: usize, rect: usize) -> i32;
    fn SetWindowPos(hwnd: usize, after: usize, x: i32, y: i32, cx: i32, cy: i32, flags: u32)
    -> i32;
    fn BeginPaint(hwnd: usize, paint: usize) -> usize;
    fn EndPaint(hwnd: usize, paint: usize) -> i32;
    fn DrawTextW(hdc: usize, text: usize, count: i32, rect: usize, format: u32) -> i32;
    fn FillRect(hdc: usize, rect: usize, brush: usize) -> i32;
    fn GetDlgCtrlID(hwnd: usize) -> i32;
    fn InvalidateRect(hwnd: usize, rect: usize, erase: i32) -> i32;
}
#[link(name = "gdi32", kind = "dylib")]
unsafe extern "system" {
    fn SelectObject(hdc: usize, object: usize) -> usize;
    fn GetStockObject(index: i32) -> usize;
    fn SetBkMode(hdc: usize, mode: i32) -> i32;
    fn SetTextColor(hdc: usize, color: u32) -> u32;
}

const MODULE: u64 =
    door_windows_abi::modules::own::PROVIDERS[door_windows_abi::modules::COMCTL32].base;
const MAX_PARTS: usize = 256;
const HEIGHT: i32 = 22;

const SB_SETTEXTW: u32 = 0x040b;
const SB_GETTEXTW: u32 = 0x040d;
const SB_GETTEXTLENGTHW: u32 = 0x040c;
const SB_SETPARTS: u32 = 0x0404;
const SB_GETPARTS: u32 = 0x0406;
const SB_GETRECT: u32 = 0x040a;
const SB_SIMPLE: u32 = 0x0409;
const NM_CLICK: u32 = 0xffff_fffe;
const NM_DBLCLK: u32 = 0xffff_fffd;
const NM_RCLICK: u32 = 0xffff_fffb;

struct Bar {
    hwnd: u64,
    parts: Vec<i32>,
    texts: Vec<(Vec<u16>, u32)>,
    simple: bool,
}

static BARS: g::Lock<Vec<Bar>> = g::Lock::new(Vec::new());

fn with_bar<R>(hwnd: u64, f: impl FnOnce(&mut Bar) -> R) -> Option<R> {
    let mut bars = BARS.lock();
    bars.iter_mut().find(|b| b.hwnd == hwnd).map(f)
}

/// The structures comctl32 builds, laid out at the build's width (P17):
/// WNDCLASSEXW (cbSize, style, lpfnWndProc, cbClsExtra, cbWndExtra,
/// hInstance, hIcon, hCursor, hbrBackground, lpszMenuName, lpszClassName,
/// hIconSm), NMMOUSE (NMHDR hwndFrom, idFrom, code; dwItemSpec, dwItemData,
/// pt.x, pt.y, dwHitInfo) and PAINTSTRUCT (hdc, fErase, rcPaint, fRestore,
/// fIncUpdate, rgbReserved).
const WNDCLASSEXW: [Field; 12] = [
    U32, U32, Pointer, U32, U32, Pointer, Pointer, Pointer, Pointer, Pointer, Pointer, Pointer,
];
const NMMOUSE: [Field; 8] = [Pointer, Pointer, U32, Pointer, Pointer, U32, U32, Pointer];
const PAINTSTRUCT: [Field; 9] = [Pointer, U32, U32, U32, U32, U32, U32, U32, Bytes(32)];

static REGISTERED: g::Lock<bool> = g::Lock::new(false);

fn register() -> bool {
    let mut done = REGISTERED.lock();
    if *done {
        return true;
    }
    let name: Vec<u16> = "msctls_statusbar32\0".encode_utf16().collect();
    let mut wc = [0u8; layout::size(&WNDCLASSEXW)];
    for (index, value) in [
        (0, wc.len() as u64),
        // CS_DBLCLKS | CS_GLOBALCLASS | CS_HREDRAW | CS_VREDRAW
        (1, 0x0008 | 0x4000 | 0x3),
        (2, status_proc as *const () as u64),
        (5, MODULE),
        (8, (color::BTNFACE + 1) as u64),
        (10, name.as_ptr() as u64),
    ] {
        layout::set(&mut wc, &WNDCLASSEXW, index, value);
    }
    // SAFETY: the class record and name live across the call.
    let ok = unsafe { RegisterClassExW(door_windows_abi::narrow(wc.as_ptr() as u64)) } != 0;
    *done = ok;
    ok
}

#[unsafe(no_mangle)]
pub extern "system" fn InitCommonControlsEx(init: usize) -> i32 {
    let init = door_windows_abi::widen(init);
    {
        let mut b = [0u8; 8];
        if !g::read(init, &mut b) || u32::from_le_bytes(b[..4].try_into().unwrap()) != 8 {
            g::set_last_error(87);
            return 0;
        }
        let classes = u32::from_le_bytes(b[4..8].try_into().unwrap());
        // As on Windows, TRUE only when every requested class is registered:
        // a mask naming any family this subset lacks (ICC_WIN95_CLASSES among
        // them) is refused rather than claimed.
        if !crate::accepts(classes) {
            g::set_last_error(50);
            return 0;
        }
        if classes & crate::ICC_BAR_CLASSES == 0 {
            return 1;
        }
        i32::from(register())
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn InitCommonControls() {
    register();
}

fn client(hwnd: u64) -> Rect {
    let mut r = [0i32; 4];
    // SAFETY: our own output array.
    unsafe {
        GetClientRect(
            door_windows_abi::narrow(hwnd),
            door_windows_abi::narrow(r.as_mut_ptr() as u64),
        )
    };
    Rect::new(r[0], r[1], r[2], r[3])
}

/// Part rectangles: each part ends at its right edge; -1 extends to the
/// client's right edge.
fn part_rects(hwnd: u64) -> Vec<Rect> {
    let c = client(hwnd);
    let parts = with_bar(hwnd, |b| {
        if b.simple {
            alloc::vec![-1]
        } else {
            b.parts.clone()
        }
    })
    .unwrap_or_default();
    let mut left = 0;
    parts
        .iter()
        .map(|&right| {
            let r = if right < 0 {
                c.right
            } else {
                right.min(c.right)
            };
            let rect = Rect::new(left, 0, r.max(left), c.bottom);
            left = r.max(left);
            rect
        })
        .collect()
}

fn notify(hwnd: u64, code: u32, x: i32, y: i32) {
    // SAFETY: plain user32 calls with our own NMMOUSE record.
    let parent = unsafe { GetParent(door_windows_abi::narrow(hwnd)) };
    if parent == 0 {
        return;
    }
    let id = unsafe { GetDlgCtrlID(door_windows_abi::narrow(hwnd)) } as u64;
    let part = part_rects(hwnd)
        .iter()
        .position(|r| r.contains(door_windows_gui::Point::new(x, y)))
        .map_or(u64::MAX, |p| p as u64);
    let mut nm = [0u8; layout::size(&NMMOUSE)];
    for (index, value) in [
        (0, hwnd),
        (1, id),
        (2, u64::from(code)),
        (3, part),
        (5, u64::from(x as u32)),
        (6, u64::from(y as u32)),
    ] {
        layout::set(&mut nm, &NMMOUSE, index, value);
    }
    unsafe {
        SendMessageW(
            parent,
            wm::NOTIFY,
            door_windows_abi::narrow(id),
            (nm.as_ptr() as i64) as isize,
        )
    };
}

fn redraw(hwnd: u64) {
    unsafe { InvalidateRect(door_windows_abi::narrow(hwnd), 0, 1) };
}

extern "system" fn status_proc(hwnd: usize, message: u32, wparam: usize, lparam: isize) -> isize {
    let hwnd = door_windows_abi::widen(hwnd);
    let wparam = door_windows_abi::widen(wparam);
    let lparam = lparam as i64;
    ((move || -> i64 {
        match message {
            wm::NCCREATE => {
                BARS.lock().push(Bar {
                    hwnd,
                    parts: alloc::vec![-1],
                    texts: alloc::vec![(Vec::new(), 0)],
                    simple: false,
                });
                unsafe {
                    (DefWindowProcW(
                        door_windows_abi::narrow(hwnd),
                        message,
                        door_windows_abi::narrow(wparam),
                        (lparam) as isize,
                    )) as i64
                }
            }
            wm::NCDESTROY => {
                BARS.lock().retain(|b| b.hwnd != hwnd);
                unsafe {
                    (DefWindowProcW(
                        door_windows_abi::narrow(hwnd),
                        message,
                        door_windows_abi::narrow(wparam),
                        (lparam) as isize,
                    )) as i64
                }
            }
            wm::SIZE => {
                // The bar keeps itself along the parent's bottom edge.
                let parent = unsafe { GetParent(door_windows_abi::narrow(hwnd)) };
                if parent != 0 {
                    let p = client(door_windows_abi::widen(parent));
                    unsafe {
                        SetWindowPos(
                            door_windows_abi::narrow(hwnd),
                            0,
                            0,
                            p.bottom - HEIGHT,
                            p.right,
                            HEIGHT,
                            0x0004 | 0x0010,
                        )
                    };
                }
                0
            }
            SB_SETPARTS => {
                let count = wparam as usize;
                if count == 0 || count > MAX_PARTS {
                    return 0;
                }
                let Some(bytes) = g::read_bytes(lparam as u64, count * 4) else {
                    return 0;
                };
                let parts: Vec<i32> = bytes
                    .chunks_exact(4)
                    .map(|c| i32::from_le_bytes(c.try_into().unwrap()))
                    .collect();
                with_bar(hwnd, |b| {
                    b.texts.resize(count, (Vec::new(), 0));
                    b.parts = parts;
                });
                redraw(hwnd);
                1
            }
            SB_GETPARTS => {
                let parts = with_bar(hwnd, |b| b.parts.clone()).unwrap_or_default();
                if lparam != 0 {
                    let n = parts.len().min(wparam as usize);
                    let bytes: Vec<u8> = parts[..n].iter().flat_map(|v| v.to_le_bytes()).collect();
                    if !g::write(lparam as u64, &bytes) {
                        return 0;
                    }
                }
                parts.len() as i64
            }
            SB_SETTEXTW => {
                let part = (wparam & 0xff) as usize;
                let flags = (wparam as u32) & 0xff00;
                let text = if lparam == 0 {
                    Some(Vec::new())
                } else {
                    g::read_wide(lparam as u64, 4096)
                };
                let Some(text) = text else { return 0 };
                let ok = with_bar(hwnd, |b| match b.texts.get_mut(part) {
                    Some(slot) => {
                        *slot = (text, flags);
                        true
                    }
                    None => false,
                })
                .unwrap_or(false);
                redraw(hwnd);
                i64::from(ok)
            }
            SB_GETTEXTW | SB_GETTEXTLENGTHW => {
                let part = (wparam & 0xff) as usize;
                let Some((text, flags)) = with_bar(hwnd, |b| b.texts.get(part).cloned()).flatten()
                else {
                    return 0;
                };
                if message == SB_GETTEXTW && lparam != 0 {
                    let mut bytes: Vec<u8> = text.iter().flat_map(|u| u.to_le_bytes()).collect();
                    bytes.extend_from_slice(&[0, 0]);
                    if !g::write(lparam as u64, &bytes) {
                        return 0;
                    }
                }
                ((text.len() as u32 & 0xffff) | (flags << 16)) as i64
            }
            SB_GETRECT => {
                let rects = part_rects(hwnd);
                match rects.get(wparam as usize) {
                    Some(r) => i64::from(g::write_i32s(
                        lparam as u64,
                        &[r.left, r.top, r.right, r.bottom],
                    )),
                    None => 0,
                }
            }
            SB_SIMPLE => {
                with_bar(hwnd, |b| b.simple = wparam != 0);
                redraw(hwnd);
                0
            }
            wm::SETTEXT => {
                let text = if lparam == 0 {
                    Some(Vec::new())
                } else {
                    g::read_wide(lparam as u64, 4096)
                };
                if let Some(t) = text {
                    with_bar(hwnd, |b| b.texts[0] = (t, 0));
                    redraw(hwnd);
                }
                unsafe {
                    (DefWindowProcW(
                        door_windows_abi::narrow(hwnd),
                        message,
                        door_windows_abi::narrow(wparam),
                        (lparam) as isize,
                    )) as i64
                }
            }
            wm::LBUTTONUP | wm::RBUTTONUP | wm::LBUTTONDBLCLK => {
                let p = door_windows_gui::Point::from_lparam(lparam as isize);
                let code = match message {
                    wm::LBUTTONUP => NM_CLICK,
                    wm::RBUTTONUP => NM_RCLICK,
                    _ => NM_DBLCLK,
                };
                notify(hwnd, code, p.x, p.y);
                0
            }
            wm::PAINT => {
                let mut ps = [0u8; layout::size(&PAINTSTRUCT)];
                let hdc = unsafe {
                    BeginPaint(
                        door_windows_abi::narrow(hwnd),
                        door_windows_abi::narrow(ps.as_mut_ptr() as u64),
                    )
                };
                if hdc != 0 {
                    paint(hwnd, door_windows_abi::widen(hdc));
                    unsafe {
                        EndPaint(
                            door_windows_abi::narrow(hwnd),
                            door_windows_abi::narrow(ps.as_ptr() as u64),
                        )
                    };
                }
                0
            }
            _ => unsafe {
                (DefWindowProcW(
                    door_windows_abi::narrow(hwnd),
                    message,
                    door_windows_abi::narrow(wparam),
                    (lparam) as isize,
                )) as i64
            },
        }
    })()) as isize
}

fn paint(hwnd: u64, hdc: u64) {
    let font = unsafe { SendMessageW(door_windows_abi::narrow(hwnd), wm::GETFONT, 0, 0) } as u64;
    unsafe {
        SelectObject(
            door_windows_abi::narrow(hdc),
            if font != 0 {
                door_windows_abi::narrow(font)
            } else {
                GetStockObject(17)
            },
        );
        SetBkMode(door_windows_abi::narrow(hdc), 1);
        SetTextColor(
            door_windows_abi::narrow(hdc),
            door_windows_gui::metrics::SYS_COLORS[color::BTNTEXT as usize],
        );
    }
    let c = client(hwnd);
    let b = c.to_bytes();
    unsafe {
        FillRect(
            door_windows_abi::narrow(hdc),
            door_windows_abi::narrow(b.as_ptr() as u64),
            door_windows_abi::narrow((color::BTNFACE + 1) as u64),
        )
    };
    let top = Rect::new(c.left, c.top, c.right, c.top + 1).to_bytes();
    unsafe {
        FillRect(
            door_windows_abi::narrow(hdc),
            door_windows_abi::narrow(top.as_ptr() as u64),
            door_windows_abi::narrow((color::BTNSHADOW + 1) as u64),
        )
    };
    let texts = with_bar(hwnd, |b| b.texts.clone()).unwrap_or_default();
    for (i, r) in part_rects(hwnd).iter().enumerate() {
        if i > 0 {
            let sep = Rect::new(r.left, r.top + 3, r.left + 1, r.bottom - 3).to_bytes();
            unsafe {
                FillRect(
                    door_windows_abi::narrow(hdc),
                    door_windows_abi::narrow(sep.as_ptr() as u64),
                    door_windows_abi::narrow((color::BTNSHADOW + 1) as u64),
                )
            };
        }
        if let Some((t, _)) = texts.get(i) {
            let tr = Rect::new(r.left + 6, r.top, r.right - 4, r.bottom).to_bytes();
            // DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX
            unsafe {
                DrawTextW(
                    door_windows_abi::narrow(hdc),
                    door_windows_abi::narrow(t.as_ptr() as u64),
                    t.len() as i32,
                    door_windows_abi::narrow(tr.as_ptr() as u64),
                    0x20 | 0x04 | 0x800,
                )
            };
        }
    }
}
