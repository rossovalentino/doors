//! Windows Installer's internal UI: message boxes, the basic UI's progress
//! dialog (/qb, /passive) and the package's own dialogs (the full UI) from
//! its Dialog and Control tables, drawn as Windows Installer draws them:
//! a "MsiDialogCloseClass" window per dialog, push buttons, check boxes,
//! radio buttons and edits as controls, text, bitmaps, lines and progress
//! bars painted by the dialog. Control events (NewDialog, SpawnDialog,
//! EndDialog, DoAction, SetProperty, SetTargetPath, Reset) run in the
//! dialog's loop; subscriptions (SetProgress, ActionText, ActionData,
//! TimeRemaining) follow the installation. An external UI handler
//! (MsiSetExternalUI) sees every message first.
use super::*;
use crate::layout::{self, Control, Dialog, control_attr};
use door_msi::engine::{Ending, Ui, level, msg};

#[link(name = "user32", kind = "dylib")]
unsafe extern "system" {
    fn RegisterClassExW(class: usize) -> u16;
    fn CreateWindowExW(
        ex: u32,
        class: usize,
        name: usize,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: usize,
        menu: usize,
        instance: usize,
        param: usize,
    ) -> usize;
    fn DefWindowProcW(hwnd: usize, message: u32, wparam: usize, lparam: isize) -> isize;
    fn DestroyWindow(hwnd: usize) -> i32;
    fn ShowWindow(hwnd: usize, cmd: i32) -> i32;
    fn UpdateWindow(hwnd: usize) -> i32;
    fn InvalidateRect(hwnd: usize, rect: usize, erase: i32) -> i32;
    fn BeginPaint(hwnd: usize, paint: usize) -> usize;
    fn EndPaint(hwnd: usize, paint: usize) -> i32;
    fn FillRect(dc: usize, rect: usize, brush: usize) -> i32;
    fn DrawTextW(dc: usize, text: usize, count: i32, rect: usize, format: u32) -> i32;
    fn GetSysColorBrush(index: i32) -> usize;
    fn GetSysColor(index: i32) -> u32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetClientRect(hwnd: usize, rect: usize) -> i32;
    fn GetWindowRect(hwnd: usize, rect: usize) -> i32;
    fn MoveWindow(hwnd: usize, x: i32, y: i32, w: i32, h: i32, repaint: i32) -> i32;
    fn SendMessageW(hwnd: usize, message: u32, wparam: usize, lparam: isize) -> isize;
    fn PeekMessageW(msg: usize, hwnd: usize, min: u32, max: u32, remove: u32) -> i32;
    fn GetMessageW(msg: usize, hwnd: usize, min: u32, max: u32) -> i32;
    fn TranslateMessage(msg: usize) -> i32;
    fn DispatchMessageW(msg: usize) -> isize;
    fn MessageBoxW(hwnd: usize, text: usize, caption: usize, kind: u32) -> i32;
    fn EnableWindow(hwnd: usize, enable: i32) -> i32;
    fn SetFocus(hwnd: usize) -> usize;
    fn GetWindowTextW(hwnd: usize, text: usize, capacity: i32) -> i32;
    fn SetForegroundWindow(hwnd: usize) -> i32;
    fn LoadCursorW(instance: usize, name: usize) -> usize;
}
#[link(name = "gdi32", kind = "dylib")]
unsafe extern "system" {
    fn CreateFontW(
        h: i32,
        w: i32,
        esc: i32,
        orient: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike: u32,
        charset: u32,
        out: u32,
        clip: u32,
        quality: u32,
        pitch: u32,
        face: usize,
    ) -> usize;
    fn SelectObject(dc: usize, object: usize) -> usize;
    fn DeleteObject(object: usize) -> i32;
    fn SetBkMode(dc: usize, mode: i32) -> i32;
    fn SetTextColor(dc: usize, color: u32) -> u32;
    fn CreateSolidBrush(color: u32) -> usize;
    fn StretchDIBits(
        dc: usize,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        sx: i32,
        sy: i32,
        sw: i32,
        sh: i32,
        bits: usize,
        info: usize,
        usage: u32,
        rop: u32,
    ) -> i32;
    fn SetStretchBltMode(dc: usize, mode: i32) -> i32;
}

const WM_DESTROY: u32 = 0x2;
const WM_PAINT: u32 = 0xf;
const WM_CLOSE: u32 = 0x10;
const WM_SETFONT: u32 = 0x30;
const WM_KEYDOWN: u32 = 0x100;
const WM_SYSKEYDOWN: u32 = 0x104;
const WM_COMMAND: u32 = 0x111;
const BM_GETCHECK: u32 = 0xf0;
const BM_SETCHECK: u32 = 0xf1;
const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_TABSTOP: u32 = 0x0001_0000;
const WS_CAPTION: u32 = 0x00c0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_BORDER: u32 = 0x0080_0000;
const WS_VSCROLL: u32 = 0x0020_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_EX_CLIENTEDGE: u32 = 0x200;
const WS_EX_DLGMODALFRAME: u32 = 0x1;
const COLOR_BTNFACE: i32 = 15;
const COLOR_WINDOWTEXT: i32 = 8;
const COLOR_GRAYTEXT: i32 = 17;
const DT_WORDBREAK: u32 = 0x10;
const DT_NOPREFIX: u32 = 0x800;
const DT_SINGLELINE: u32 = 0x20;
const DT_END_ELLIPSIS: u32 = 0x8000;
const DT_RIGHT: u32 = 0x2;

/// What the dialog's window procedure saw, for the dialog's loop.
#[derive(Default)]
struct Win {
    hwnd: u64,
    dialog: Dialog,
    /// (control index, its window) for the controls that are windows.
    children: Vec<(usize, u64)>,
    clicked: Vec<usize>,
    closed: bool,
    /// Painted state per control: formatted text, shown, enabled.
    texts: Vec<String>,
    shown: Vec<bool>,
    enabled: Vec<bool>,
    /// Progress bars: (done, total).
    progress: Vec<(u64, u64)>,
    /// Bitmaps by control index: (width, height, BITMAPINFO, bits).
    #[allow(clippy::type_complexity)]
    bitmaps: Vec<(usize, i32, i32, Vec<u8>, Vec<u8>)>,
    /// Fonts by style name.
    fonts: Vec<(String, u64, u32)>,
    default_font: u64,
}
static WINDOWS: Lock<Vec<Win>> = Lock::new(Vec::new());
static CLASS: AtomicU32 = AtomicU32::new(0);
const CHILD_BASE: u64 = 100;

fn rect(r: &[i32; 4]) -> u64 {
    r.as_ptr() as u64
}
fn rect_mut(r: &mut [i32; 4]) -> u64 {
    r.as_mut_ptr() as u64
}
/// "MsiDialogCloseClass", Windows Installer's dialog class.
static CLASS_NAME: [u16; 20] = {
    let s = b"MsiDialogCloseClass\0";
    let mut out = [0u16; 20];
    let mut i = 0;
    while i < 20 {
        out[i] = s[i] as u16;
        i += 1;
    }
    out
};

extern "system" fn procedure(hwnd: usize, message: u32, wparam: usize, lparam: isize) -> isize {
    let hwnd = door_windows_abi::widen(hwnd);
    let wparam = door_windows_abi::widen(wparam);
    let lparam = lparam as i64;
    ((move || -> i64 {
        match message {
            WM_PAINT => {
                paint(hwnd);
                0
            }
            WM_COMMAND => {
                let id = wparam & 0xffff;
                let notify = (wparam >> 16) & 0xffff;
                if id >= CHILD_BASE && (notify == 0 || notify == 0x300) {
                    WINDOWS.with(|w| {
                        if let Some(win) = w.iter_mut().find(|x| x.hwnd == hwnd) {
                            let index = (id - CHILD_BASE) as usize;
                            if notify == 0 {
                                win.clicked.push(index);
                            }
                        }
                    });
                }
                0
            }
            WM_CLOSE => {
                WINDOWS.with(|w| {
                    if let Some(win) = w.iter_mut().find(|x| x.hwnd == hwnd) {
                        win.closed = true;
                    }
                });
                0
            }
            WM_DESTROY => 0,
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

fn register() -> u64 {
    let atom = CLASS.load(Ordering::Acquire);
    let name = &CLASS_NAME;
    if atom == 0 {
        // WNDCLASSEXW: cbSize, style, lpfnWndProc, cbClsExtra, cbWndExtra,
        // hInstance, hIcon, hCursor, hbrBackground, lpszMenuName,
        // lpszClassName, hIconSm (80 bytes on x64, 48 on x86).
        use door_windows_abi::layout as abi_layout;
        const WNDCLASSEXW: [abi_layout::Field; 12] = {
            use abi_layout::Field::*;
            [
                U32, U32, Pointer, U32, U32, Pointer, Pointer, Pointer, Pointer, Pointer, Pointer,
                Pointer,
            ]
        };
        let mut class = [0u8; abi_layout::size(&WNDCLASSEXW)];
        let size = abi_layout::size(&WNDCLASSEXW) as u64;
        abi_layout::set(&mut class, &WNDCLASSEXW, 0, size);
        abi_layout::set(&mut class, &WNDCLASSEXW, 1, 3); // CS_HREDRAW | CS_VREDRAW
        abi_layout::set(&mut class, &WNDCLASSEXW, 2, procedure as *const () as u64);
        let cursor = unsafe { LoadCursorW(0, 32512) };
        abi_layout::set(&mut class, &WNDCLASSEXW, 7, cursor as u64);
        abi_layout::set(&mut class, &WNDCLASSEXW, 8, (COLOR_BTNFACE + 1) as u64);
        abi_layout::set(&mut class, &WNDCLASSEXW, 10, name.as_ptr() as u64);
        let a = unsafe { RegisterClassExW(door_windows_abi::narrow(class.as_ptr() as u64)) };
        CLASS.store(u32::from(a).max(1), Ordering::Release);
    }
    name.as_ptr() as u64
}

/// A font for a TextStyle (or the dialog default).
fn font(style: Option<&layout::TextStyle>) -> (u64, u32) {
    let (face, size, bits, color) = match style {
        Some(s) => (s.face.as_str(), s.size, s.bits, s.color.unwrap_or(u32::MAX)),
        None => ("Segoe UI", 9, 0, u32::MAX),
    };
    let face = utf16z(face);
    let height = -(size * 96 + 36) / 72;
    let f = unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            if bits & 1 != 0 { 700 } else { 400 },
            u32::from(bits & 2 != 0),
            u32::from(bits & 4 != 0),
            u32::from(bits & 8 != 0),
            1,
            0,
            0,
            5,
            0,
            door_windows_abi::narrow(face.as_ptr() as u64),
        )
    };
    (door_windows_abi::widen(f), color)
}

/// A BMP (or the first image of an ICO) as a DIB: (width, height,
/// BITMAPINFO, bits).
fn decode_image(bytes: &[u8]) -> Option<(i32, i32, Vec<u8>, Vec<u8>)> {
    fn get(b: &[u8], at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
    }
    if bytes.get(..2)? == b"BM" {
        let offset = get(bytes, 10)? as usize;
        let info = bytes.get(14..offset)?.to_vec();
        let width = get(bytes, 18)? as i32;
        let height = get(bytes, 22)? as i32;
        return Some((width, height.abs(), info, bytes.get(offset..)?.to_vec()));
    }
    // ICO: the first entry's DIB (32-bit), its XOR rows only.
    if bytes.get(..4)? == [0, 0, 1, 0] {
        let size = get(bytes, 6 + 8)? as usize;
        let at = get(bytes, 6 + 12)? as usize;
        let dib = bytes.get(at..at + size)?;
        let header = get(dib, 0)? as usize;
        let width = get(dib, 4)? as i32;
        let height = get(dib, 8)? as i32 / 2;
        let bpp = u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?);
        if bpp != 32 {
            return None;
        }
        let mut info = dib.get(..header)?.to_vec();
        info[8..12].copy_from_slice(&height.to_le_bytes());
        let pixels = (width * height * 4) as usize;
        return Some((
            width,
            height,
            info,
            dib.get(header..header + pixels)?.to_vec(),
        ));
    }
    None
}

fn paint(hwnd: u64) {
    let mut ps = [0u8; 72];
    let dc = unsafe {
        BeginPaint(
            door_windows_abi::narrow(hwnd),
            door_windows_abi::narrow(ps.as_mut_ptr() as u64),
        )
    };
    if dc == 0 {
        return;
    }
    let mut client = [0i32; 4];
    unsafe {
        GetClientRect(
            door_windows_abi::narrow(hwnd),
            door_windows_abi::narrow(rect_mut(&mut client)),
        )
    };
    unsafe {
        FillRect(
            dc,
            door_windows_abi::narrow(rect(&client)),
            GetSysColorBrush(COLOR_BTNFACE),
        )
    };
    unsafe { SetBkMode(dc, 1) };
    unsafe { SetStretchBltMode(dc, 4) };
    WINDOWS.with(|w| {
        let Some(win) = w.iter().find(|x| x.hwnd == hwnd) else {
            return;
        };
        // Bitmaps first, then lines, progress bars and text over them.
        for (i, c) in win.dialog.controls.iter().enumerate() {
            if !win.shown.get(i).copied().unwrap_or(false) {
                continue;
            }
            let r = [
                layout::pixels(c.x),
                layout::pixels(c.y),
                layout::pixels(c.x + c.width),
                layout::pixels(c.y + c.height),
            ];
            match c.kind.as_str() {
                "Bitmap" | "Icon" => {
                    if let Some((_, w, h, info, bits)) = win.bitmaps.iter().find(|b| b.0 == i) {
                        unsafe {
                            StretchDIBits(
                                dc,
                                r[0],
                                r[1],
                                r[2] - r[0],
                                r[3] - r[1],
                                0,
                                0,
                                *w,
                                *h,
                                door_windows_abi::narrow(bits.as_ptr() as u64),
                                door_windows_abi::narrow(info.as_ptr() as u64),
                                0,
                                0x00cc_0020,
                            )
                        };
                    }
                }
                _ => (),
            }
        }
        for (i, c) in win.dialog.controls.iter().enumerate() {
            if !win.shown.get(i).copied().unwrap_or(false) {
                continue;
            }
            let r = [
                layout::pixels(c.x),
                layout::pixels(c.y),
                layout::pixels(c.x + c.width),
                layout::pixels(c.y + c.height),
            ];
            match c.kind.as_str() {
                "Line" => {
                    let a = [r[0], r[1], r[2], r[1] + 1];
                    let b = [r[0], r[1] + 1, r[2], r[1] + 2];
                    unsafe {
                        let dark = CreateSolidBrush(0x00a0_a0a0);
                        let light = CreateSolidBrush(0x00ff_ffff);
                        FillRect(dc, door_windows_abi::narrow(rect(&a)), dark);
                        FillRect(dc, door_windows_abi::narrow(rect(&b)), light);
                        DeleteObject(dark);
                        DeleteObject(light);
                    }
                }
                "ProgressBar" => {
                    let (done, total) = win.progress.get(i).copied().unwrap_or((0, 0));
                    unsafe {
                        let track = CreateSolidBrush(0x00e6_e6e6);
                        let border = CreateSolidBrush(0x00bc_bcbc);
                        let fill = CreateSolidBrush(0x0025_b006);
                        FillRect(dc, door_windows_abi::narrow(rect(&r)), border);
                        let inner = [r[0] + 1, r[1] + 1, r[2] - 1, r[3] - 1];
                        FillRect(dc, door_windows_abi::narrow(rect(&inner)), track);
                        if total > 0 {
                            let width =
                                ((inner[2] - inner[0]) as u64 * done.min(total) / total) as i32;
                            let bar = [inner[0], inner[1], inner[0] + width, inner[3]];
                            FillRect(dc, door_windows_abi::narrow(rect(&bar)), fill);
                        }
                        DeleteObject(track);
                        DeleteObject(border);
                        DeleteObject(fill);
                    }
                }
                "Text" => {
                    let (style, text) =
                        layout::split_style(win.texts.get(i).map(String::as_str).unwrap_or(""));
                    let (f, color) = style
                        .as_ref()
                        .and_then(|s| win.fonts.iter().find(|x| x.0 == *s))
                        .map(|x| (x.1, x.2))
                        .unwrap_or((win.default_font, u32::MAX));
                    let old = unsafe { SelectObject(dc, door_windows_abi::narrow(f)) };
                    let color = if !win.enabled.get(i).copied().unwrap_or(true) {
                        unsafe { GetSysColor(COLOR_GRAYTEXT) }
                    } else if color == u32::MAX {
                        unsafe { GetSysColor(COLOR_WINDOWTEXT) }
                    } else {
                        color
                    };
                    unsafe { SetTextColor(dc, color) };
                    let mut flags = DT_NOPREFIX;
                    if c.attributes & control_attr::NO_WRAP != 0 {
                        flags |= DT_SINGLELINE | DT_END_ELLIPSIS;
                    } else {
                        flags |= DT_WORDBREAK;
                    }
                    if c.attributes & 0x40 != 0 {
                        flags |= DT_RIGHT;
                    }
                    let w16: Vec<u16> = text.encode_utf16().collect();
                    let mut box_ = r;
                    unsafe {
                        DrawTextW(
                            dc,
                            door_windows_abi::narrow(w16.as_ptr() as u64),
                            w16.len() as i32,
                            door_windows_abi::narrow(rect_mut(&mut box_)),
                            flags,
                        )
                    };
                    unsafe { SelectObject(dc, old) };
                }
                _ => (),
            }
        }
    });
    unsafe {
        EndPaint(
            door_windows_abi::narrow(hwnd),
            door_windows_abi::narrow(ps.as_mut_ptr() as u64),
        )
    };
}

/// Pump the thread's messages without waiting.
pub(crate) fn pump() {
    let mut m = [0u8; 48];
    while unsafe { PeekMessageW(door_windows_abi::narrow(m.as_mut_ptr() as u64), 0, 0, 0, 1) } != 0
    {
        unsafe {
            TranslateMessage(door_windows_abi::narrow(m.as_ptr() as u64));
            DispatchMessageW(door_windows_abi::narrow(m.as_ptr() as u64));
        }
    }
}

/// The internal UI with the session's product name and, for the full UI,
/// the open dialogs.
pub struct Interface {
    pub level: u32,
    pub passive: bool,
    pub caption: String,
    /// MsiSetExternalUI: (handler, filter, context).
    pub external: Option<(u64, u32, u64)>,
    /// The dialog on screen between dialogs: the modeless one (ProgressDlg,
    /// or the basic UI's), or the wizard page that just ended, kept until
    /// the next one is up (as Windows Installer switches pages: msiexec
    /// never has no window, so it keeps the keyboard).
    current: Option<u64>,
    cancel_asked: bool,
    /// Spawned dialogs (SpawnDialog) running over the current page.
    spawned: u32,
    started: bool,
    time_remaining: String,
    /// Properties as the last dialog saw them (for conditions while the
    /// installation runs).
    properties: Vec<(String, String)>,
    /// The package (its dialogs while the installation runs).
    pub package: Option<Arc<Vec<u8>>>,
}
impl Interface {
    pub fn new(level: u32, passive: bool, caption: &str) -> Self {
        Self {
            level,
            passive,
            caption: String::from(caption),
            external: None,
            current: None,
            cancel_asked: false,
            spawned: 0,
            started: false,
            time_remaining: String::new(),
            properties: Vec::new(),
            package: None,
        }
    }
    fn external(&mut self, kind: u32, text: &str) -> Option<i32> {
        let (handler, filter, context) = self.external?;
        let kind_bit = 1u32 << (kind >> 24);
        if filter & kind_bit == 0 {
            return None;
        }
        let w = utf16z(text);
        // SAFETY: INSTALLUI_HANDLERW: int (LPVOID, UINT, LPCWSTR).
        let f: extern "system" fn(usize, u32, usize) -> i32 =
            unsafe { core::mem::transmute(handler as usize) };
        let answer = f(
            door_windows_abi::narrow(context),
            kind,
            door_windows_abi::narrow(w.as_ptr() as u64),
        );
        (answer != 0).then_some(answer)
    }
    /// Close what this UI has on screen.
    pub fn close(&mut self) {
        if let Some(h) = self.current.take() {
            destroy(h);
        }
    }
    /// The basic UI's progress dialog: Windows Installer's own layout.
    fn basic_dialog(&self) -> Dialog {
        let mk = |name: &str,
                  kind: &str,
                  x: i32,
                  y: i32,
                  w: i32,
                  h: i32,
                  attributes: i32,
                  text: &str| Control {
            name: String::from(name),
            kind: String::from(kind),
            x,
            y,
            width: w,
            height: h,
            attributes,
            text: String::from(text),
            ..Control::default()
        };
        let mut controls = vec![
            mk(
                "Info",
                "Text",
                15,
                12,
                250,
                12,
                3 | control_attr::NO_PREFIX,
                &alloc::format!("Please wait while Windows configures {}", self.caption),
            ),
            mk(
                "ActionText",
                "Text",
                15,
                30,
                250,
                12,
                3 | control_attr::NO_WRAP,
                "Gathering required information...",
            ),
            mk("TimeRemaining", "Text", 15, 44, 250, 12, 3, ""),
            mk("ProgressBar", "ProgressBar", 15, 58, 250, 10, 3, ""),
        ];
        controls[1]
            .subscriptions
            .push((String::from("ActionText"), String::from("Text")));
        controls[2]
            .subscriptions
            .push((String::from("TimeRemaining"), String::from("TimeRemaining")));
        controls[3]
            .subscriptions
            .push((String::from("SetProgress"), String::from("Progress")));
        let mut cancel = mk(
            "Cancel",
            "PushButton",
            209,
            76,
            56,
            17,
            if self.passive { 1 } else { 3 },
            "Cancel",
        );
        cancel.events.push(layout::Event {
            event: String::from("SpawnDialog"),
            argument: String::from("#cancel"),
            condition: String::from("1"),
            ordering: 1,
        });
        controls.push(cancel);
        Dialog {
            name: String::from("#basic"),
            h_center: 50,
            v_center: 50,
            width: 280,
            height: 100,
            attributes: layout::dialog_attr::VISIBLE | layout::dialog_attr::MINIMIZE,
            title: self.caption.clone(),
            cancel: String::from("Cancel"),
            controls,
            ..Dialog::default()
        }
    }
    fn update<F: FnMut(&Control, usize, &mut Win) -> bool>(&mut self, mut f: F) {
        let Some(h) = self.current else { return };
        let mut repaint = false;
        WINDOWS.with(|w| {
            if let Some(win) = w.iter_mut().find(|x| x.hwnd == h) {
                let controls = win.dialog.controls.clone();
                for (i, c) in controls.iter().enumerate() {
                    repaint |= f(c, i, win);
                }
            }
        });
        if repaint {
            unsafe { InvalidateRect(door_windows_abi::narrow(h), 0, 0) };
        }
        pump();
    }
    /// The package's cancel dialog (the current dialog's Cancel control
    /// spawns it) while the installation runs: true when it ended with
    /// Exit. Formatting and conditions use the properties the last dialog
    /// saw.
    fn spawn_detached(&mut self, name: &str) -> Option<bool> {
        let bytes = self.package.clone()?;
        let db = door_msi::Database::open(&bytes).ok()?;
        let d = layout::dialog(&db, name)?;
        let styles = layout::text_styles(&db);
        let state = State {
            properties: self.properties.clone(),
            ..State::default()
        };
        let default_style = String::from(state.property("DefaultUIFont"));
        let images = |key: &str| {
            db.stream(&alloc::format!("Binary.{}", state.format(key)))
                .ok()
        };
        let format = |t: &str| state.format(t);
        let condition = |c: &str| state.evaluate(c);
        let property = |p: &str| String::from(state.property(p));
        let hwnd = create(
            &d,
            &styles,
            &default_style,
            &images,
            &format,
            &condition,
            &property,
        );
        if hwnd == 0 {
            return None;
        }
        let exit = loop {
            let mut m = [0u8; 48];
            if unsafe { GetMessageW(door_windows_abi::narrow(m.as_mut_ptr() as u64), 0, 0, 0) } <= 0
            {
                break true;
            }
            unsafe {
                TranslateMessage(door_windows_abi::narrow(m.as_ptr() as u64));
                DispatchMessageW(door_windows_abi::narrow(m.as_ptr() as u64));
            }
            let (clicked, closed) = WINDOWS.with(|w| {
                w.iter_mut()
                    .find(|x| x.hwnd == hwnd)
                    .map(|win| {
                        (
                            core::mem::take(&mut win.clicked),
                            core::mem::take(&mut win.closed),
                        )
                    })
                    .unwrap_or_default()
            });
            if closed {
                break false;
            }
            let ended = clicked.iter().find_map(|i| {
                d.controls[i & 0xfff]
                    .events
                    .iter()
                    .find(|e| e.event == "EndDialog" && state.evaluate(&e.condition))
                    .map(|e| e.argument == "Exit")
            });
            if let Some(exit) = ended {
                break exit;
            }
        };
        destroy(hwnd);
        Some(exit)
    }
    /// The user pressed Cancel during the installation: ask.
    fn confirm_cancel(&mut self) -> bool {
        let spawn = self.current.and_then(|h| {
            WINDOWS.with(|w| {
                let win = w.iter().find(|x| x.hwnd == h)?;
                let cancel = win.dialog.control(&win.dialog.cancel)?;
                cancel
                    .events
                    .iter()
                    .find(|e| e.event == "SpawnDialog" && !e.argument.starts_with('#'))
                    .map(|e| e.argument.clone())
            })
        });
        if let Some(name) = spawn
            && let Some(exit) = self.spawn_detached(&name)
        {
            return exit;
        }
        let text = utf16z(&alloc::format!(
            "Are you sure you want to cancel {} installation?",
            self.caption
        ));
        let caption = utf16z(&self.caption);
        let answer = unsafe {
            MessageBoxW(
                door_windows_abi::narrow(self.current.unwrap_or(0)),
                door_windows_abi::narrow(text.as_ptr() as u64),
                door_windows_abi::narrow(caption.as_ptr() as u64),
                4 | 0x20,
            )
        };
        answer == msg::IDYES
    }
    fn cancel_clicked(&mut self) -> bool {
        let Some(h) = self.current else { return false };
        let mut clicked = Vec::new();
        let mut closed = false;
        WINDOWS.with(|w| {
            if let Some(win) = w.iter_mut().find(|x| x.hwnd == h) {
                clicked = core::mem::take(&mut win.clicked);
                closed = core::mem::take(&mut win.closed);
                let cancel = win.dialog.cancel.clone();
                clicked.retain(|i| win.dialog.controls[*i].name == cancel);
            }
        });
        !clicked.is_empty() || closed
    }
}

/// A page gets the keyboard back (a spawned dialog over it closed): it is
/// activated and its first control focused.
fn reactivate(hwnd: u64) {
    let first = WINDOWS.with(|w| {
        let win = w.iter().find(|x| x.hwnd == hwnd)?;
        let i = win
            .dialog
            .controls
            .iter()
            .position(|c| c.name == win.dialog.first)?;
        win.children.iter().find(|c| c.0 == i).map(|c| c.1)
    });
    if WINDOWS.with(|w| w.iter().any(|x| x.hwnd == hwnd)) {
        unsafe {
            SetForegroundWindow(door_windows_abi::narrow(hwnd));
            if let Some(f) = first {
                SetFocus(door_windows_abi::narrow(f));
            }
        }
    }
}
fn destroy(hwnd: u64) {
    let fonts = WINDOWS.with(|w| {
        let pos = w.iter().position(|x| x.hwnd == hwnd)?;
        let win = w.remove(pos);
        Some(
            win.fonts
                .iter()
                .map(|f| f.1)
                .chain(core::iter::once(win.default_font))
                .collect::<Vec<u64>>(),
        )
    });
    unsafe { DestroyWindow(door_windows_abi::narrow(hwnd)) };
    for f in fonts.unwrap_or_default() {
        unsafe { DeleteObject(door_windows_abi::narrow(f)) };
    }
}

/// Create a dialog's window and its controls. `format` expands control
/// text; `condition` evaluates ControlConditions; `property` reads a
/// control's property.
fn create(
    d: &Dialog,
    styles: &[layout::TextStyle],
    default_style: &str,
    images: &dyn Fn(&str) -> Option<Vec<u8>>,
    format: &dyn Fn(&str) -> String,
    condition: &dyn Fn(&str) -> bool,
    property: &dyn Fn(&str) -> String,
) -> u64 {
    let class = register();
    let title = utf16z(&format(&d.title));
    let width = layout::pixels(d.width);
    let height = layout::pixels(d.height);
    let screen = unsafe { (GetSystemMetrics(0), GetSystemMetrics(1)) };
    let style = WS_CAPTION
        | WS_SYSMENU
        | if d.attributes & layout::dialog_attr::MINIMIZE != 0 {
            WS_MINIMIZEBOX
        } else {
            0
        };
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            door_windows_abi::narrow(class),
            door_windows_abi::narrow(title.as_ptr() as u64),
            style,
            0,
            0,
            width,
            height,
            0,
            0,
            0,
            0,
        )
    };
    if hwnd == 0 {
        return 0;
    }
    // Size the window so its client area is the dialog's.
    let (mut outer, mut inner) = ([0i32; 4], [0i32; 4]);
    unsafe {
        GetWindowRect(hwnd, door_windows_abi::narrow(rect_mut(&mut outer)));
        GetClientRect(hwnd, door_windows_abi::narrow(rect_mut(&mut inner)));
    }
    let w = width + (outer[2] - outer[0]) - (inner[2] - inner[0]);
    let h = height + (outer[3] - outer[1]) - (inner[3] - inner[1]);
    let x = ((screen.0 - w) * d.h_center.clamp(0, 100)) / 100;
    let y = ((screen.1 - h) * d.v_center.clamp(0, 100)) / 100;
    unsafe { MoveWindow(hwnd, x.max(0), y.max(0), w, h, 0) };
    let mut win = Win {
        hwnd: door_windows_abi::widen(hwnd),
        dialog: d.clone(),
        ..Win::default()
    };
    for s in styles {
        let (f, color) = font(Some(s));
        win.fonts.push((s.name.clone(), f, color));
    }
    win.default_font = styles
        .iter()
        .find(|s| s.name == default_style)
        .map_or_else(|| font(None).0, |s| font(Some(s)).0);
    for (i, c) in d.controls.iter().enumerate() {
        let mut shown = c.attributes & control_attr::VISIBLE != 0;
        let mut enabled = c.attributes & control_attr::ENABLED != 0;
        for (action, cond) in &c.conditions {
            if condition(cond) {
                match action.as_str() {
                    "Show" => shown = true,
                    "Hide" => shown = false,
                    "Enable" => enabled = true,
                    "Disable" => enabled = false,
                    _ => (),
                }
            }
        }
        win.shown.push(shown);
        win.enabled.push(enabled);
        win.progress.push((0, 0));
        let text = format(&c.text);
        win.texts.push(text.clone());
        if matches!(c.kind.as_str(), "Bitmap" | "Icon")
            && let Some(bytes) = images(&c.text)
            && let Some((w, h, info, bits)) = decode_image(&bytes)
        {
            win.bitmaps.push((i, w, h, info, bits));
        }
        let (class, mut style, ex, caption): (&str, u32, u32, String) = match c.kind.as_str() {
            "PushButton" => (
                "Button",
                WS_TABSTOP | if c.name == d.default { 1 } else { 0 },
                0,
                layout::split_style(&text).1.into(),
            ),
            "CheckBox" => (
                "Button",
                WS_TABSTOP | 3,
                0,
                layout::split_style(&text).1.into(),
            ),
            "GroupBox" => ("Button", 7, 0, layout::split_style(&text).1.into()),
            "Edit" | "PathEdit" | "MaskedEdit" => (
                "Edit",
                WS_TABSTOP
                    | WS_BORDER
                    | 0x80
                    | if c.attributes & control_attr::PASSWORD != 0 {
                        0x20
                    } else {
                        0
                    },
                WS_EX_CLIENTEDGE,
                property(&c.property),
            ),
            "ScrollableText" => (
                "Edit",
                WS_TABSTOP | WS_BORDER | WS_VSCROLL | 4 | 0x40 | 0x800,
                WS_EX_CLIENTEDGE,
                layout::rtf_text(&c.text),
            ),
            "RadioButtonGroup" => {
                // One button per RadioButton row, the group's value
                // checked.
                let value = property(&c.property);
                for (k, (v, x, y, w, h, t)) in c.radio.iter().enumerate() {
                    let caption = utf16z(layout::split_style(&format(t)).1);
                    let class = utf16z("Button");
                    let b = unsafe {
                        CreateWindowExW(
                            0,
                            door_windows_abi::narrow(class.as_ptr() as u64),
                            door_windows_abi::narrow(caption.as_ptr() as u64),
                            WS_CHILD | WS_TABSTOP | 9 | if shown { WS_VISIBLE } else { 0 },
                            layout::pixels(c.x + x),
                            layout::pixels(c.y + y),
                            layout::pixels(*w),
                            layout::pixels(*h),
                            hwnd,
                            door_windows_abi::narrow(
                                CHILD_BASE + (i as u64) + ((k as u64 + 1) << 12),
                            ),
                            0,
                            0,
                        )
                    };
                    unsafe {
                        SendMessageW(b, WM_SETFONT, door_windows_abi::narrow(win.default_font), 1);
                        if *v == value {
                            SendMessageW(b, BM_SETCHECK, 1, 0);
                        }
                    }
                    win.children
                        .push((i | ((k + 1) << 12), door_windows_abi::widen(b)));
                }
                continue;
            }
            _ => continue,
        };
        if shown {
            style |= WS_VISIBLE;
        }
        let class = utf16z(class);
        let caption = utf16z(&caption);
        let child = unsafe {
            CreateWindowExW(
                ex,
                door_windows_abi::narrow(class.as_ptr() as u64),
                door_windows_abi::narrow(caption.as_ptr() as u64),
                WS_CHILD | style,
                layout::pixels(c.x),
                layout::pixels(c.y),
                layout::pixels(c.width),
                layout::pixels(c.height),
                hwnd,
                door_windows_abi::narrow(CHILD_BASE + i as u64),
                0,
                0,
            )
        };
        let (style_name, _) = layout::split_style(&text);
        let f = style_name
            .and_then(|s| win.fonts.iter().find(|x| x.0 == s).map(|x| x.1))
            .unwrap_or(win.default_font);
        unsafe {
            SendMessageW(child, WM_SETFONT, door_windows_abi::narrow(f), 1);
            if !enabled {
                EnableWindow(child, 0);
            }
            if c.kind == "CheckBox" && !property(&c.property).is_empty() {
                SendMessageW(child, BM_SETCHECK, 1, 0);
            }
        }
        win.children.push((i, door_windows_abi::widen(child)));
    }
    let first = d
        .controls
        .iter()
        .position(|c| c.name == d.first)
        .and_then(|i| win.children.iter().find(|x| x.0 == i).map(|x| x.1));
    WINDOWS.with(|w| w.push(win));
    unsafe {
        ShowWindow(hwnd, 5);
        UpdateWindow(hwnd);
        SetForegroundWindow(hwnd);
        if let Some(f) = first {
            SetFocus(door_windows_abi::narrow(f));
        }
    }
    pump();
    door_windows_abi::widen(hwnd)
}

/// The text of an Edit control.
fn edit_text(hwnd: u64) -> String {
    let mut buffer = [0u16; 1024];
    let n = unsafe {
        GetWindowTextW(
            door_windows_abi::narrow(hwnd),
            door_windows_abi::narrow(buffer.as_mut_ptr() as u64),
            buffer.len() as i32,
        )
    }
    .max(0) as usize;
    String::from_utf16_lossy(&buffer[..n])
}

/// A session's view for a dialog: formatting, conditions and properties
/// read from the engine.
fn session_dialog(session: &Session<'_, '_>, name: &str) -> Option<(Dialog, u64)> {
    let d = layout::dialog(session.db, name)?;
    let styles = layout::text_styles(session.db);
    let default_style = String::from(session.property("DefaultUIFont"));
    let images = |key: &str| {
        session
            .db
            .stream(&alloc::format!("Binary.{}", session.format(key)))
            .ok()
    };
    let format = |t: &str| session.format(t);
    let condition = |c: &str| session.evaluate(c);
    let property = |p: &str| String::from(session.property(p));
    let h = create(
        &d,
        &styles,
        &default_style,
        &images,
        &format,
        &condition,
        &property,
    );
    let _ = door_sys::write(alloc::format!("DOOR_P16 MSI DIALOG name={name}\n").as_bytes());
    Some((d, h))
}

/// Read back what the user typed and checked into the session.
fn collect(session: &mut Session<'_, '_>, hwnd: u64) {
    let (dialog, children) = WINDOWS.with(|w| {
        w.iter()
            .find(|x| x.hwnd == hwnd)
            .map(|win| (win.dialog.clone(), win.children.clone()))
            .unwrap_or_default()
    });
    for (index, child) in children {
        let i = index & 0xfff;
        let Some(c) = dialog.controls.get(i) else {
            continue;
        };
        if c.property.is_empty() {
            continue;
        }
        match c.kind.as_str() {
            "Edit" | "PathEdit" | "MaskedEdit" => {
                let text = edit_text(child);
                session.set_property(&c.property, &text);
            }
            "CheckBox" => {
                let checked =
                    unsafe { SendMessageW(door_windows_abi::narrow(child), BM_GETCHECK, 0, 0) }
                        == 1;
                let value = if checked {
                    door_msi::Database::table(session.db, "CheckBox")
                        .ok()
                        .and_then(|t| {
                            t.find(&c.property)
                                .and_then(|r| t.get(r, "Value").str().map(String::from))
                        })
                        .unwrap_or_else(|| String::from("1"))
                } else {
                    String::new()
                };
                session.set_property(&c.property, &value);
            }
            "RadioButtonGroup" => {
                let k = index >> 12;
                if k > 0
                    && unsafe { SendMessageW(door_windows_abi::narrow(child), BM_GETCHECK, 0, 0) }
                        == 1
                    && let Some(r) = c.radio.get(k - 1)
                {
                    session.set_property(&c.property, &r.0);
                }
            }
            _ => (),
        }
    }
}

/// Re-evaluate ControlConditions after an event.
fn refresh(session: &Session<'_, '_>, hwnd: u64) {
    WINDOWS.with(|w| {
        let Some(win) = w.iter_mut().find(|x| x.hwnd == hwnd) else {
            return;
        };
        let controls = win.dialog.controls.clone();
        for (i, c) in controls.iter().enumerate() {
            if c.conditions.is_empty() {
                continue;
            }
            let (mut shown, mut enabled) = (win.shown[i], win.enabled[i]);
            for (action, cond) in &c.conditions {
                if session.evaluate(cond) {
                    match action.as_str() {
                        "Show" => shown = true,
                        "Hide" => shown = false,
                        "Enable" => enabled = true,
                        "Disable" => enabled = false,
                        _ => (),
                    }
                }
            }
            win.shown[i] = shown;
            win.enabled[i] = enabled;
            for (index, child) in win.children.iter().filter(|x| x.0 & 0xfff == i) {
                let _ = index;
                unsafe {
                    ShowWindow(door_windows_abi::narrow(*child), if shown { 5 } else { 0 });
                    EnableWindow(door_windows_abi::narrow(*child), i32::from(enabled));
                }
            }
        }
    });
    unsafe { InvalidateRect(door_windows_abi::narrow(hwnd), 0, 1) };
}

enum Next {
    Stay,
    End(Ending),
    Replace(String),
}

impl Interface {
    /// Run a dialog's events for a clicked control.
    fn events(&mut self, session: &mut Session<'_, '_>, hwnd: u64, control: &Control) -> Next {
        collect(session, hwnd);
        let mut next = Next::Stay;
        for e in &control.events {
            if !session.evaluate(&e.condition) {
                continue;
            }
            let argument = session.format(&e.argument);
            match e.event.as_str() {
                "EndDialog" => {
                    next = Next::End(match argument.as_str() {
                        "Exit" => Ending::Exit,
                        "Retry" => Ending::Retry,
                        "Ignore" => Ending::Ignore,
                        "ErrorAbort" | "ErrorCancel" | "ErrorNo" => Ending::Exit,
                        _ => Ending::Return,
                    })
                }
                "NewDialog" => next = Next::Replace(argument),
                "SpawnDialog" | "SpawnWaitDialog" => {
                    self.spawned += 1;
                    let ending = self.dialog(session, &argument);
                    self.spawned -= 1;
                    // The page under it has the keyboard again, as its
                    // owner window does on Windows.
                    reactivate(hwnd);
                    if matches!(ending, Ending::Exit) {
                        next = Next::End(Ending::Exit);
                    }
                }
                "DoAction" => {
                    let _ = session.action(self, &argument);
                }
                "SetTargetPath" => {
                    let mut v = String::from(session.property(&argument));
                    if !v.is_empty() && !v.ends_with('\\') {
                        v.push('\\');
                    }
                    session.set_property(&argument, &v);
                    session.recost();
                }
                "Reset" => (),
                "AddLocal" | "AddSource" => {
                    let mut list = String::from(session.property("ADDLOCAL"));
                    if !list.split(',').any(|f| f == argument) {
                        if !list.is_empty() {
                            list.push(',');
                        }
                        list.push_str(&argument);
                    }
                    session.set_property("ADDLOCAL", &list);
                }
                "Remove" => session.set_property("REMOVE", &argument),
                "Reinstall" => session.set_property("REINSTALL", &argument),
                "ReinstallMode" => session.set_property("REINSTALLMODE", &argument),
                "SetInstallLevel" => session.set_property("INSTALLLEVEL", &argument),
                name if name.starts_with('[') && name.ends_with(']') => {
                    let property = &name[1..name.len() - 1];
                    let value = if argument == "{}" {
                        String::new()
                    } else {
                        argument.clone()
                    };
                    session.set_property(property, &value);
                }
                _ => (),
            }
        }
        refresh(session, hwnd);
        next
    }

    fn run_modal(&mut self, session: &mut Session<'_, '_>, name: &str) -> Ending {
        let mut name = String::from(name);
        // A spawned dialog runs over the page that spawned it; a page
        // replaces the one before it once it is up.
        let spawned = self.spawned > 0;
        loop {
            let Some((d, hwnd)) = session_dialog(session, &name) else {
                return Ending::Error;
            };
            if !spawned && let Some(p) = self.current.take() {
                destroy(p);
            }
            let outcome = loop {
                let mut m = [0u8; 48];
                if unsafe { GetMessageW(door_windows_abi::narrow(m.as_mut_ptr() as u64), 0, 0, 0) }
                    <= 0
                {
                    break Next::End(Ending::Exit);
                }
                // MSG: hwnd, message, wParam, lParam, ... (P17: pointer
                // width).
                use door_windows_abi::layout as abi_layout;
                const MSG: [abi_layout::Field; 4] = {
                    use abi_layout::Field::*;
                    [Pointer, U32, Pointer, Pointer]
                };
                let target = abi_layout::get(&m, &MSG, 0);
                let message = abi_layout::get(&m, &MSG, 1) as u32;
                let key = abi_layout::get(&m, &MSG, 2);
                let mut handled = false;
                // Alt+letter: the control whose text marks that letter with
                // "&" (a check box toggles).
                if message == WM_SYSKEYDOWN && (0x41..=0x5a).contains(&key) {
                    let letter = char::from(key as u8).to_ascii_lowercase();
                    let hit = d.controls.iter().position(|c| {
                        let (_, text) = layout::split_style(&c.text);
                        text.to_ascii_lowercase()
                            .contains(&alloc::format!("&{letter}"))
                            && matches!(c.kind.as_str(), "PushButton" | "CheckBox")
                    });
                    if let Some(i) = hit {
                        WINDOWS.with(|w| {
                            if let Some(win) = w.iter_mut().find(|x| x.hwnd == hwnd)
                                && win.enabled.get(i).copied().unwrap_or(false)
                            {
                                if let Some(child) =
                                    win.children.iter().find(|c| c.0 == i).map(|c| c.1)
                                    && win.dialog.controls[i].kind == "CheckBox"
                                {
                                    let checked = unsafe {
                                        SendMessageW(
                                            door_windows_abi::narrow(child),
                                            BM_GETCHECK,
                                            0,
                                            0,
                                        )
                                    } == 1;
                                    unsafe {
                                        SendMessageW(
                                            door_windows_abi::narrow(child),
                                            BM_SETCHECK,
                                            door_windows_abi::narrow(u64::from(!checked)),
                                            0,
                                        )
                                    };
                                }
                                win.clicked.push(i);
                            }
                        });
                        handled = true;
                    }
                }
                if message == WM_KEYDOWN && (key == 0x0d || key == 0x1b) {
                    let mine = target == hwnd
                        || WINDOWS.with(|w| {
                            w.iter()
                                .find(|x| x.hwnd == hwnd)
                                .is_some_and(|win| win.children.iter().any(|c| c.1 == target))
                        });
                    let multiline = WINDOWS.with(|w| {
                        w.iter().find(|x| x.hwnd == hwnd).is_some_and(|win| {
                            win.children
                                .iter()
                                .find(|c| c.1 == target)
                                .is_some_and(|c| {
                                    win.dialog.controls[c.0 & 0xfff].kind == "ScrollableText"
                                })
                        })
                    });
                    if mine && !(multiline && key == 0x0d) {
                        let wanted = if key == 0x0d { &d.default } else { &d.cancel };
                        if let Some(i) = d.controls.iter().position(|c| c.name == *wanted) {
                            WINDOWS.with(|w| {
                                if let Some(win) = w.iter_mut().find(|x| x.hwnd == hwnd)
                                    && win.enabled.get(i).copied().unwrap_or(false)
                                {
                                    win.clicked.push(i);
                                }
                            });
                            handled = true;
                        }
                    }
                }
                if !handled {
                    unsafe {
                        TranslateMessage(door_windows_abi::narrow(m.as_ptr() as u64));
                        DispatchMessageW(door_windows_abi::narrow(m.as_ptr() as u64));
                    }
                }
                let (clicked, closed) = WINDOWS.with(|w| {
                    w.iter_mut()
                        .find(|x| x.hwnd == hwnd)
                        .map(|win| {
                            (
                                core::mem::take(&mut win.clicked),
                                core::mem::take(&mut win.closed),
                            )
                        })
                        .unwrap_or_default()
                });
                let mut result = Next::Stay;
                let mut indices = clicked;
                if closed && let Some(i) = d.controls.iter().position(|c| c.name == d.cancel) {
                    indices.push(i);
                }
                for i in indices {
                    let control = d.controls[i & 0xfff].clone();
                    match self.events(session, hwnd, &control) {
                        Next::Stay => (),
                        other => {
                            result = other;
                            break;
                        }
                    }
                }
                if !matches!(result, Next::Stay) {
                    break result;
                }
            };
            collect(session, hwnd);
            if spawned {
                destroy(hwnd);
            } else {
                // Up until the next page (or the end of the UI) replaces it.
                self.current = Some(hwnd);
            }
            match outcome {
                Next::End(e) => return e,
                Next::Replace(n) => name = n,
                Next::Stay => return Ending::Return,
            }
        }
    }
}

impl Ui for Interface {
    fn level(&self) -> u32 {
        self.level
    }
    fn message(&mut self, kind: u32, text: &str) -> i32 {
        if let Some(answer) = self.external(kind, text) {
            return answer;
        }
        let class = kind & 0xff00_0000;
        match class {
            msg::ACTION_START => {
                let description = text.split_once('\t').map_or(text, |x| x.1);
                if !description.is_empty() {
                    let d = String::from(description);
                    self.update(|c, i, win| {
                        if c.subscriptions.iter().any(|s| s.0 == "ActionText") {
                            win.texts[i] = d.clone();
                            return true;
                        }
                        false
                    });
                }
                0
            }
            msg::ACTION_DATA => {
                let d = String::from(text);
                self.update(|c, i, win| {
                    if c.subscriptions.iter().any(|s| s.0 == "ActionData") {
                        win.texts[i] = d.clone();
                        return true;
                    }
                    false
                });
                0
            }
            msg::ERROR | msg::WARNING | msg::USER | msg::OUT_OF_DISK_SPACE | msg::FATAL_EXIT => {
                if self.level <= level::NONE {
                    return 0;
                }
                // Basic UI: only errors and the restart question are shown.
                let style = kind & 0x00ff_ffff;
                let w = utf16z(text);
                let caption = utf16z(if self.caption.is_empty() {
                    "Doors Installer"
                } else {
                    &self.caption
                });
                unsafe {
                    MessageBoxW(
                        door_windows_abi::narrow(self.current.unwrap_or(0)),
                        door_windows_abi::narrow(w.as_ptr() as u64),
                        door_windows_abi::narrow(caption.as_ptr() as u64),
                        style,
                    )
                }
            }
            _ => 0,
        }
    }
    fn progress(&mut self, done: u64, total: u64) -> bool {
        if let Some(answer) =
            self.external(msg::PROGRESS, &alloc::format!("1: 2 2: {done} 3: {total}"))
            && answer == msg::IDCANCEL
        {
            return false;
        }
        if self.level <= level::NONE {
            return true;
        }
        if self.current.is_none() && self.level == level::BASIC && !self.started {
            self.started = true;
            let d = self.basic_dialog();
            let styles: [layout::TextStyle; 0] = [];
            let none = |_: &str| None;
            let same = |t: &str| String::from(t);
            let yes = |_: &str| true;
            let empty = |_: &str| String::new();
            self.current = Some(create(&d, &styles, "", &none, &same, &yes, &empty));
        }
        let remaining = if total > 0 && done < total {
            alloc::format!(
                "Time remaining: {} seconds",
                ((total - done) / 8_000_000).max(1)
            )
        } else {
            String::new()
        };
        self.time_remaining = remaining.clone();
        self.update(|c, i, win| {
            let mut changed = false;
            if c.subscriptions.iter().any(|s| s.0 == "SetProgress") {
                win.progress[i] = (done, total);
                changed = true;
            }
            if c.subscriptions.iter().any(|s| s.0 == "TimeRemaining") {
                win.texts[i] = remaining.clone();
                changed = true;
            }
            changed
        });
        if !self.passive && self.cancel_clicked() && !self.cancel_asked {
            self.cancel_asked = true;
            let cancel = self.confirm_cancel();
            self.cancel_asked = false;
            if cancel {
                return false;
            }
        }
        true
    }
    fn dialog(&mut self, session: &mut Session<'_, '_>, name: &str) -> Ending {
        if self.level < level::REDUCED {
            return Ending::Skip;
        }
        let Some(d) = layout::dialog(session.db, name) else {
            return Ending::Skip;
        };
        self.caption = String::from(session.property("ProductName"));
        self.properties = session.state.properties.clone();
        if d.modal() {
            return self.run_modal(session, name);
        }
        // A modeless dialog (ProgressDlg) stays up while the sequence goes
        // on; it replaces the page before it once it is up.
        if let Some((_, h)) = session_dialog(session, name)
            && let Some(p) = self.current.replace(h)
        {
            destroy(p);
        }
        Ending::Return
    }
}
