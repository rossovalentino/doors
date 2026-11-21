//! TaskDialog and TaskDialogIndirect (comctl32 v6 ordinals 344 and 345;
//! Chromium's message boxes). The dialog is an ordinary modal dialog built
//! from a template, laid out like Windows 11's: the main instruction in
//! 12 pt blue, the content, radio buttons and command links on white, and
//! the verification check box and the common and custom buttons on the
//! grey footer band. The callback receives TDN_DIALOG_CONSTRUCTED,
//! TDN_CREATED, TDN_BUTTON_CLICKED (S_FALSE keeps the dialog open),
//! TDN_RADIO_BUTTON_CLICKED, TDN_VERIFICATION_CLICKED, TDN_TIMER and
//! TDN_DESTROYED; the TDM_ messages that click, enable and retitle
//! elements work. Not drawn: icons (the box sounds its icon's event
//! instead), progress bars, hyperlinks (shown as text) and the expando
//! button (expanded information shows only with TDF_EXPANDED_BY_DEFAULT).
use alloc::vec::Vec;
use door_windows_abi::{narrow, widen};
use door_windows_gui_guest as g;

#[link(name = "user32", kind = "dylib")]
unsafe extern "system" {
    fn DialogBoxIndirectParamW(
        instance: usize,
        template: usize,
        owner: usize,
        procedure: usize,
        param: isize,
    ) -> isize;
    fn EndDialog(dialog: usize, result: isize) -> i32;
    fn GetDlgItem(dialog: usize, id: i32) -> usize;
    fn SetDlgItemTextW(dialog: usize, id: i32, text: usize) -> i32;
    fn IsDlgButtonChecked(dialog: usize, id: i32) -> u32;
    fn CheckDlgButton(dialog: usize, id: i32, check: u32) -> i32;
    fn EnableWindow(hwnd: usize, enable: i32) -> i32;
    fn GetDialogBaseUnits() -> i32;
    fn MapDialogRect(dialog: usize, rect: usize) -> i32;
    fn LoadStringW(instance: usize, id: u32, out: usize, capacity: i32) -> i32;
    fn MessageBeep(kind: u32) -> i32;
    fn SetTimer(hwnd: usize, id: usize, elapse: u32, procedure: usize) -> usize;
    fn KillTimer(hwnd: usize, id: usize) -> i32;
    fn GetDC(hwnd: usize) -> usize;
    fn ReleaseDC(hwnd: usize, hdc: usize) -> i32;
    fn GetClientRect(hwnd: usize, rect: usize) -> i32;
    fn FillRect(hdc: usize, rect: usize, brush: usize) -> i32;
    fn DrawTextW(hdc: usize, text: usize, count: i32, rect: usize, format: u32) -> i32;
    fn SendMessageW(hwnd: usize, message: u32, wparam: usize, lparam: isize) -> isize;
}
#[link(name = "gdi32", kind = "dylib")]
unsafe extern "system" {
    fn CreateFontW(
        height: i32,
        width: i32,
        escapement: i32,
        orientation: i32,
        weight: i32,
        italic: u32,
        underline: u32,
        strike: u32,
        charset: u32,
        precision: u32,
        clip: u32,
        quality: u32,
        pitch: u32,
        face: usize,
    ) -> usize;
    fn CreateSolidBrush(color: u32) -> usize;
    fn DeleteObject(object: usize) -> i32;
    fn SelectObject(hdc: usize, object: usize) -> usize;
    fn GetStockObject(index: i32) -> usize;
    fn SetTextColor(hdc: usize, color: u32) -> u32;
    fn SetBkColor(hdc: usize, color: u32) -> u32;
}

const S_OK: u32 = 0;
const S_FALSE: u32 = 1;
const E_INVALIDARG: u32 = 0x8007_0057;
const E_OUTOFMEMORY: u32 = 0x8007_000E;
const E_FAIL: u32 = 0x8000_4005;

// TASKDIALOG_FLAGS.
const TDF_ALLOW_DIALOG_CANCELLATION: u32 = 0x8;
const TDF_USE_COMMAND_LINKS: u32 = 0x10;
const TDF_USE_COMMAND_LINKS_NO_ICON: u32 = 0x20;
const TDF_EXPANDED_BY_DEFAULT: u32 = 0x80;
const TDF_VERIFICATION_FLAG_CHECKED: u32 = 0x100;
const TDF_CALLBACK_TIMER: u32 = 0x800;
const TDF_NO_DEFAULT_RADIO_BUTTON: u32 = 0x4000;

// Notifications.
const TDN_CREATED: u32 = 0;
const TDN_BUTTON_CLICKED: u32 = 2;
const TDN_TIMER: u32 = 4;
const TDN_DESTROYED: u32 = 5;
const TDN_RADIO_BUTTON_CLICKED: u32 = 6;
const TDN_DIALOG_CONSTRUCTED: u32 = 7;
const TDN_VERIFICATION_CLICKED: u32 = 8;

// Messages (WM_USER + n).
const TDM_CLICK_BUTTON: u32 = 0x466;
const TDM_SET_ELEMENT_TEXT: u32 = 0x46C;
const TDM_CLICK_RADIO_BUTTON: u32 = 0x46E;
const TDM_ENABLE_BUTTON: u32 = 0x46F;
const TDM_ENABLE_RADIO_BUTTON: u32 = 0x470;
const TDM_CLICK_VERIFICATION: u32 = 0x471;
const TDM_UPDATE_ELEMENT_TEXT: u32 = 0x472;

const WM_DESTROY: u32 = 0x0002;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_INITDIALOG: u32 = 0x0110;
const WM_COMMAND: u32 = 0x0111;
const WM_TIMER: u32 = 0x0113;
const WM_CTLCOLORBTN: u32 = 0x0135;
const WM_CTLCOLORDLG: u32 = 0x0136;
const WM_CTLCOLORSTATIC: u32 = 0x0138;
const WM_SETFONT: u32 = 0x0030;

// Controls of the template.
const ID_INSTRUCTION: i32 = 0x7F01;
const ID_CONTENT: i32 = 0x7F02;
const ID_EXPANDED: i32 = 0x7F03;
const ID_FOOTER: i32 = 0x7F04;
const ID_VERIFY: i32 = 0x7F05;
const ID_RADIO: i32 = 0x7E00;
const IDCANCEL: i32 = 2;
const TIMER_ID: usize = 0x7444;

/// Main-instruction blue and the footer band's grey (aero.msstyles).
const INSTRUCTION_COLOR: u32 = 0x0099_3300;
const BAND_COLOR: u32 = 0x00F0_F0F0;
const WHITE: u32 = 0x00FF_FFFF;

/// A running task dialog's state, found by its window.
struct Task {
    hwnd: u64,
    callback: u64,
    data: i64,
    flags: u32,
    buttons: Vec<i32>,
    radios: Vec<i32>,
    radio: i32,
    has_verification: bool,
    verified: bool,
    has_cancel: bool,
    /// The footer band's top in dialog units, and its font and brushes.
    band_top: i16,
    font: u64,
    white: u64,
    band: u64,
    started: u64,
}
static TASKS: g::Lock<Vec<Task>> = g::Lock::new(Vec::new());

fn with_task<R>(hwnd: u64, f: impl FnOnce(&mut Task) -> R) -> Option<R> {
    TASKS.lock().iter_mut().find(|t| t.hwnd == hwnd).map(f)
}
fn notify(hwnd: u64, code: u32, wparam: u64, lparam: i64) -> u32 {
    let Some((callback, data)) = with_task(hwnd, |t| (t.callback, t.data)) else {
        return S_OK;
    };
    if callback == 0 {
        return S_OK;
    }
    type Callback = extern "system" fn(usize, u32, usize, isize, isize) -> u32;
    // SAFETY: the application's PFTASKDIALOGCALLBACK, called with no lock.
    let f: Callback = unsafe { core::mem::transmute(callback as usize) };
    f(
        narrow(hwnd),
        code,
        narrow(wparam),
        lparam as isize,
        data as isize,
    )
}

/// A string field: NULL, a resource ID of `instance`, or text.
fn text(instance: u64, pointer: u64) -> Vec<u16> {
    if pointer == 0 {
        return Vec::new();
    }
    if pointer < 0x1_0000 {
        let mut buf = [0u16; 1024];
        let n = unsafe {
            LoadStringW(
                narrow(instance),
                pointer as u32,
                buf.as_mut_ptr() as usize,
                1024,
            )
        };
        return buf[..n.max(0) as usize].to_vec();
    }
    g::read_wide(pointer, 32768).unwrap_or_default()
}

/// The fields of a TASKDIALOGCONFIG (packed: `pshpack1.h`).
struct Config {
    owner: u64,
    instance: u64,
    flags: u32,
    common: u32,
    title: Vec<u16>,
    icon: u64,
    instruction: Vec<u16>,
    content: Vec<u16>,
    buttons: Vec<(i32, Vec<u16>)>,
    default_button: i32,
    radios: Vec<(i32, Vec<u16>)>,
    default_radio: i32,
    verification: Vec<u16>,
    expanded: Vec<u16>,
    footer: Vec<u16>,
    callback: u64,
    data: i64,
    width: u32,
}
fn read_config(at: u64) -> Option<Config> {
    let p = g::POINTER;
    let size = 4 + 2 * p + 8 + 2 * p + 2 * p + 4 + p + 8 + p + 4 + 5 * p + p + 2 * p + 4;
    let b = g::read_bytes(at, size)?;
    let mut o = 0usize;
    let u32_at = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    let ptr_at = |o: usize| {
        if p == 8 {
            u64::from_le_bytes(b[o..o + 8].try_into().unwrap_or([0; 8]))
        } else {
            u64::from(u32_at(o))
        }
    };
    let cb = u32_at(0) as usize;
    if cb != size {
        return None;
    }
    o += 4;
    let owner = ptr_at(o);
    o += p;
    let instance = ptr_at(o);
    o += p;
    let flags = u32_at(o);
    let common = u32_at(o + 4);
    o += 8;
    let title = text(instance, ptr_at(o));
    o += p;
    let icon = ptr_at(o);
    o += p;
    let instruction = text(instance, ptr_at(o));
    o += p;
    let content = text(instance, ptr_at(o));
    o += p;
    let count = u32_at(o) as usize;
    o += 4;
    let list = ptr_at(o);
    o += p;
    let default_button = u32_at(o) as i32;
    o += 4;
    let radio_count = u32_at(o) as usize;
    o += 4;
    let radio_list = ptr_at(o);
    o += p;
    let default_radio = u32_at(o) as i32;
    o += 4;
    let verification = text(instance, ptr_at(o));
    o += p;
    let expanded = text(instance, ptr_at(o));
    o += 3 * p; // expanded and collapsed control texts
    o += p; // footer icon
    let footer = text(instance, ptr_at(o));
    o += p;
    let callback = ptr_at(o);
    o += p;
    let data = if p == 8 {
        ptr_at(o) as i64
    } else {
        i64::from(u32_at(o) as i32)
    };
    o += p;
    let width = u32_at(o);
    // TASKDIALOG_BUTTON: nButtonID, then the (packed) text pointer.
    let entries = |list: u64, count: usize| -> Option<Vec<(i32, Vec<u16>)>> {
        if count > 0 && list == 0 {
            return None;
        }
        let item = 4 + p;
        let raw = g::read_bytes(list, count.checked_mul(item)?)?;
        Some(
            raw.chunks_exact(item)
                .map(|c| {
                    let id = i32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                    let ptr = if p == 8 {
                        u64::from_le_bytes(c[4..12].try_into().unwrap_or([0; 8]))
                    } else {
                        u64::from(u32::from_le_bytes([c[4], c[5], c[6], c[7]]))
                    };
                    (id, text(instance, ptr))
                })
                .collect(),
        )
    };
    let buttons = if count == 0 {
        Vec::new()
    } else {
        entries(list, count.min(64))?
    };
    let radios = if radio_count == 0 {
        Vec::new()
    } else {
        entries(radio_list, radio_count.min(64))?
    };
    Some(Config {
        owner,
        instance,
        flags,
        common,
        title,
        icon,
        instruction,
        content,
        buttons,
        default_button,
        radios,
        default_radio,
        verification,
        expanded,
        footer,
        callback,
        data,
        width,
    })
}

/// TDCBF_* flags in Windows' order with their result IDs and labels.
const COMMON: [(u32, i32, &str); 6] = [
    (0x1, 1, "OK"),
    (0x2, 6, "&Yes"),
    (0x4, 7, "&No"),
    (0x8, 2, "Cancel"),
    (0x10, 4, "&Retry"),
    (0x20, 8, "&Close"),
];

/// A template control: style, rectangle (dialog units), ID, class atom, text.
type Item = (u32, (i16, i16, i16, i16), i32, u16, Vec<u16>);
fn push_item(
    b: &mut Vec<u8>,
    style: u32,
    r: (i16, i16, i16, i16),
    id: i32,
    class: u16,
    text: &[u16],
) {
    while !b.len().is_multiple_of(4) {
        b.push(0);
    }
    const CHILD_VISIBLE: u32 = 0x5000_0000;
    for v in [0u32, 0, style | CHILD_VISIBLE] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    for v in [r.0, r.1, r.2, r.3] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&(id as u32).to_le_bytes());
    b.extend_from_slice(&[0xff, 0xff]);
    b.extend_from_slice(&class.to_le_bytes());
    for u in text {
        b.extend_from_slice(&u.to_le_bytes());
    }
    b.extend_from_slice(&[0, 0, 0, 0]);
}

/// The height in dialog units of `text` wrapped at `width` units in `font`.
fn measure(font: u64, text: &[u16], width: i16, base_x: i32, base_y: i32) -> i16 {
    if text.is_empty() {
        return 0;
    }
    let hdc = unsafe { GetDC(0) };
    let old = unsafe { SelectObject(hdc, narrow(font)) };
    let mut r = [0i32, 0, i32::from(width) * base_x / 4, 0];
    // DT_WORDBREAK | DT_CALCRECT | DT_NOPREFIX.
    unsafe {
        DrawTextW(
            hdc,
            text.as_ptr() as usize,
            text.len() as i32,
            r.as_mut_ptr() as usize,
            0x10 | 0x400 | 0x800,
        )
    };
    unsafe {
        SelectObject(hdc, old);
        ReleaseDC(0, hdc);
    }
    ((r[3] - r[1]).max(base_y) * 8 / base_y.max(1) + 1) as i16
}

fn run(config: &Config, button_out: u64, radio_out: u64, verified_out: u64) -> u32 {
    let units = unsafe { GetDialogBaseUnits() };
    let (base_x, base_y) = ((units & 0xffff).max(1), ((units >> 16) & 0xffff).max(1));
    let face: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
    let font = widen(unsafe {
        CreateFontW(
            -16,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            1,
            0,
            0,
            5,
            0,
            face.as_ptr() as usize,
        )
    });
    let dialog_font = widen(unsafe { GetStockObject(17) });
    let width: i16 = if config.width != 0 {
        (config.width as i16).clamp(120, 600)
    } else {
        240
    };
    let inner = width - 20;
    let command_links = config.flags & (TDF_USE_COMMAND_LINKS | TDF_USE_COMMAND_LINKS_NO_ICON) != 0;
    let mut items: Vec<Item> = Vec::new();
    const SS_NOPREFIX: u32 = 0x80;
    const BS_DEFPUSHBUTTON: u32 = 1;
    const BS_AUTOCHECKBOX: u32 = 3;
    const BS_AUTORADIOBUTTON: u32 = 9;
    const WS_TABSTOP: u32 = 0x0001_0000;
    const WS_GROUP: u32 = 0x0002_0000;
    let mut y: i16 = 10;
    if !config.instruction.is_empty() {
        let h = measure(font, &config.instruction, inner, base_x, base_y);
        items.push((
            SS_NOPREFIX,
            (10, y, inner, h),
            ID_INSTRUCTION,
            0x82,
            config.instruction.clone(),
        ));
        y += h + 6;
    }
    if !config.content.is_empty() {
        let h = measure(dialog_font, &config.content, inner, base_x, base_y);
        items.push((
            SS_NOPREFIX,
            (10, y, inner, h),
            ID_CONTENT,
            0x82,
            config.content.clone(),
        ));
        y += h + 6;
    }
    if !config.expanded.is_empty() && config.flags & TDF_EXPANDED_BY_DEFAULT != 0 {
        let h = measure(dialog_font, &config.expanded, inner, base_x, base_y);
        items.push((
            SS_NOPREFIX,
            (10, y, inner, h),
            ID_EXPANDED,
            0x82,
            config.expanded.clone(),
        ));
        y += h + 6;
    }
    for (i, (_, label)) in config.radios.iter().enumerate() {
        let style = BS_AUTORADIOBUTTON | WS_TABSTOP | if i == 0 { WS_GROUP } else { 0 };
        items.push((
            style,
            (16, y, inner - 6, 10),
            ID_RADIO + i as i32,
            0x80,
            label.clone(),
        ));
        y += 12;
    }
    // The buttons: command links stack on the white area; push buttons
    // (custom first, then common) sit right-aligned on the footer band.
    let mut push: Vec<(i32, Vec<u16>)> = Vec::new();
    let mut ids = Vec::new();
    for (id, label) in &config.buttons {
        ids.push(*id);
        if command_links {
            let title: Vec<u16> = label
                .split(|u| *u == u16::from(b'\n'))
                .next()
                .unwrap_or(&[])
                .to_vec();
            items.push((WS_TABSTOP, (16, y, inner - 6, 20), *id, 0x80, title));
            y += 23;
        } else {
            push.push((*id, label.clone()));
        }
    }
    for (bit, id, label) in COMMON {
        if config.common & bit != 0 {
            ids.push(id);
            push.push((id, label.encode_utf16().collect()));
        }
    }
    if ids.is_empty() {
        ids.push(1);
        push.push((1, "OK".encode_utf16().collect()));
    }
    let default_id = if ids.contains(&config.default_button) {
        config.default_button
    } else {
        ids[0]
    };
    let band_top = y + 4;
    y = band_top + 8;
    let buttons_w = push.len() as i16 * 54;
    let mut x = width - 10 - buttons_w + 4;
    for (i, (id, label)) in push.iter().enumerate() {
        let style = WS_TABSTOP
            | if *id == default_id {
                BS_DEFPUSHBUTTON
            } else {
                0
            }
            | if i == 0 { WS_GROUP } else { 0 };
        items.push((style, (x, y, 50, 14), *id, 0x80, label.clone()));
        x += 54;
    }
    if !config.verification.is_empty() {
        let w = (width - 24 - buttons_w).max(40);
        items.push((
            BS_AUTOCHECKBOX | WS_TABSTOP | WS_GROUP,
            (10, y + 2, w, 10),
            ID_VERIFY,
            0x80,
            config.verification.clone(),
        ));
    }
    y += 14 + 8;
    if !config.footer.is_empty() {
        let h = measure(dialog_font, &config.footer, inner, base_x, base_y);
        items.push((
            SS_NOPREFIX,
            (10, y, inner, h),
            ID_FOOTER,
            0x82,
            config.footer.clone(),
        ));
        y += h + 6;
    }
    let height = y;
    // DLGTEMPLATE: WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME |
    // DS_SETFONT | DS_CENTER.
    let mut b = Vec::new();
    for v in [1u16, 0xffff] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    let style: u32 = 0x8000_0000 | 0x00C0_0000 | 0x0008_0000 | 0x80 | 0x40 | 0x800;
    for v in [0, 0, style] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&(items.len() as u16).to_le_bytes());
    for v in [0i16, 0, width, height] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.extend_from_slice(&[0, 0, 0, 0]);
    let title: Vec<u16> = if config.title.is_empty() {
        "Doors".encode_utf16().collect()
    } else {
        config.title.clone()
    };
    for u in title.iter().take(255).chain(core::iter::once(&0)) {
        b.extend_from_slice(&u.to_le_bytes());
    }
    b.extend_from_slice(&9u16.to_le_bytes());
    b.extend_from_slice(&400u16.to_le_bytes());
    b.extend_from_slice(&[0, 1]);
    for u in face.iter() {
        b.extend_from_slice(&u.to_le_bytes());
    }
    for (style, r, id, class, label) in &items {
        push_item(&mut b, *style, *r, *id, *class, label);
    }
    let radio_ids: Vec<i32> = config.radios.iter().map(|(id, _)| *id).collect();
    let radio = if config.flags & TDF_NO_DEFAULT_RADIO_BUTTON != 0 {
        0
    } else if radio_ids.contains(&config.default_radio) {
        config.default_radio
    } else {
        radio_ids.first().copied().unwrap_or(0)
    };
    let task = Task {
        hwnd: 0,
        callback: config.callback,
        data: config.data,
        flags: config.flags,
        buttons: ids.clone(),
        radios: radio_ids,
        radio,
        has_verification: !config.verification.is_empty(),
        verified: config.flags & TDF_VERIFICATION_FLAG_CHECKED != 0,
        has_cancel: ids.contains(&IDCANCEL),
        band_top,
        font,
        white: widen(unsafe { CreateSolidBrush(WHITE) }),
        band: widen(unsafe { CreateSolidBrush(BAND_COLOR) }),
        started: g::now(),
    };
    // The icon's sound, as MessageBox plays it.
    let sound = match config.icon {
        0xFFFF => 0x30,
        0xFFFE => 0x10,
        0xFFFD => 0x40,
        _ => 0,
    };
    if sound != 0 {
        unsafe { MessageBeep(sound) };
    }
    // One dialog initialises at a time (WM_INITDIALOG runs before
    // DialogBoxIndirectParam returns control): a second, from another
    // thread meanwhile, fails rather than take the first one's state.
    {
        let mut pending = PENDING.lock();
        if pending.is_some() {
            drop(pending);
            release(&task);
            return E_FAIL;
        }
        *pending = Some(task);
    }
    let result = unsafe {
        DialogBoxIndirectParamW(
            narrow(config.instance),
            b.as_ptr() as usize,
            narrow(config.owner),
            dialog_proc as *const () as usize,
            0,
        )
    };
    // The dialog never initialised: drop the pending state.
    if let Some(t) = PENDING.lock().take() {
        release(&t);
    }
    if result == -1 {
        return E_OUTOFMEMORY;
    }
    let (radio, verified) = *LAST_RESULT.lock();
    g::write_u32(button_out, result as u32);
    g::write_u32(radio_out, radio as u32);
    g::write_u32(verified_out, u32::from(verified != 0));
    S_OK
}
/// The state handed from TaskDialogIndirect to WM_INITDIALOG (a modal
/// dialog initialises on the calling thread before any other can start).
static PENDING: g::Lock<Option<Task>> = g::Lock::new(None);
/// The last finished dialog's radio button and verification state.
static LAST_RESULT: g::Lock<(u64, u64)> = g::Lock::new((0, 0));

fn release(t: &Task) {
    unsafe {
        DeleteObject(narrow(t.font));
        DeleteObject(narrow(t.white));
        DeleteObject(narrow(t.band));
    }
}

fn click(hwnd: u64, id: i32) {
    if notify(hwnd, TDN_BUTTON_CLICKED, id as u64, 0) == S_FALSE {
        return;
    }
    unsafe { EndDialog(narrow(hwnd), id as isize) };
}

extern "system" fn dialog_proc(hwnd: usize, message: u32, wparam: usize, lparam: isize) -> isize {
    let h = widen(hwnd);
    let wparam = widen(wparam);
    match message {
        WM_INITDIALOG => {
            let Some(mut task) = PENDING.lock().take() else {
                return 1;
            };
            task.hwnd = h;
            let (font, radios, radio, verified, flags) = (
                task.font,
                task.radios.clone(),
                task.radio,
                task.verified,
                task.flags,
            );
            TASKS.lock().push(task);
            if let Some(control) =
                Some(unsafe { GetDlgItem(hwnd, ID_INSTRUCTION) }).filter(|c| *c != 0)
            {
                unsafe { SendMessageW(control, WM_SETFONT, narrow(font), 0) };
            }
            if let Some(i) = radios.iter().position(|r| *r == radio) {
                unsafe { CheckDlgButton(hwnd, ID_RADIO + i as i32, 1) };
            }
            if verified {
                unsafe { CheckDlgButton(hwnd, ID_VERIFY, 1) };
            }
            if flags & TDF_CALLBACK_TIMER != 0 {
                unsafe { SetTimer(hwnd, TIMER_ID, 200, 0) };
            }
            notify(h, TDN_DIALOG_CONSTRUCTED, 0, 0);
            notify(h, TDN_CREATED, 0, 0);
            1
        }
        WM_ERASEBKGND => {
            let Some((white, band, top)) = with_task(h, |t| (t.white, t.band, t.band_top)) else {
                return 0;
            };
            let mut client = [0i32; 4];
            unsafe { GetClientRect(hwnd, client.as_mut_ptr() as usize) };
            let mut edge = [0i32, i32::from(top), 1, i32::from(top)];
            unsafe { MapDialogRect(hwnd, edge.as_mut_ptr() as usize) };
            let split = edge[1];
            let upper = [client[0], client[1], client[2], split];
            let lower = [client[0], split, client[2], client[3]];
            unsafe {
                FillRect(narrow(wparam), upper.as_ptr() as usize, narrow(white));
                FillRect(narrow(wparam), lower.as_ptr() as usize, narrow(band));
            }
            1
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLORDLG => {
            let child = lparam as usize;
            let id = if child == 0 {
                0
            } else {
                control_id(hwnd, child)
            };
            let Some((white, band)) = with_task(h, |t| (t.white, t.band)) else {
                return 0;
            };
            let on_band = matches!(id, ID_FOOTER | ID_VERIFY)
                || (with_task(h, |t| t.buttons.contains(&id)).unwrap_or(false)
                    && !is_link(hwnd, id));
            let (brush, color) = if message == WM_CTLCOLORDLG || on_band {
                (band, BAND_COLOR)
            } else {
                (white, WHITE)
            };
            unsafe {
                SetBkColor(narrow(wparam), color);
                SetTextColor(
                    narrow(wparam),
                    if id == ID_INSTRUCTION {
                        INSTRUCTION_COLOR
                    } else {
                        0
                    },
                );
            }
            brush as isize
        }
        WM_COMMAND => {
            let id = (wparam & 0xffff) as i32;
            let Some((buttons, radios, cancellable)) = with_task(h, |t| {
                (
                    t.buttons.clone(),
                    t.radios.clone(),
                    t.has_cancel || t.flags & TDF_ALLOW_DIALOG_CANCELLATION != 0,
                )
            }) else {
                return 0;
            };
            if id == ID_VERIFY {
                let checked = unsafe { IsDlgButtonChecked(hwnd, ID_VERIFY) } != 0;
                with_task(h, |t| t.verified = checked);
                notify(h, TDN_VERIFICATION_CLICKED, u64::from(checked), 0);
            } else if (ID_RADIO..ID_RADIO + radios.len() as i32).contains(&id) {
                let chosen = radios[(id - ID_RADIO) as usize];
                with_task(h, |t| t.radio = chosen);
                notify(h, TDN_RADIO_BUTTON_CLICKED, chosen as u64, 0);
            } else if buttons.contains(&id) || (id == IDCANCEL && cancellable) {
                click(h, id);
            }
            1
        }
        WM_TIMER if wparam as usize == TIMER_ID => {
            let started = with_task(h, |t| t.started).unwrap_or(0);
            if notify(h, TDN_TIMER, g::now().saturating_sub(started), 0) == S_FALSE {
                with_task(h, |t| t.started = g::now());
            }
            1
        }
        TDM_CLICK_BUTTON => {
            click(h, wparam as i32);
            1
        }
        TDM_SET_ELEMENT_TEXT | TDM_UPDATE_ELEMENT_TEXT => {
            let id = match wparam {
                0 => ID_CONTENT,
                1 => ID_EXPANDED,
                2 => ID_FOOTER,
                _ => ID_INSTRUCTION,
            };
            unsafe { SetDlgItemTextW(hwnd, id, lparam as usize) };
            1
        }
        TDM_CLICK_RADIO_BUTTON => {
            let radios = with_task(h, |t| t.radios.clone()).unwrap_or_default();
            if let Some(i) = radios.iter().position(|r| *r == wparam as i32) {
                for j in 0..radios.len() {
                    unsafe { CheckDlgButton(hwnd, ID_RADIO + j as i32, u32::from(j == i)) };
                }
                with_task(h, |t| t.radio = radios[i]);
                notify(h, TDN_RADIO_BUTTON_CLICKED, radios[i] as u64, 0);
            }
            1
        }
        TDM_ENABLE_BUTTON => {
            unsafe { EnableWindow(GetDlgItem(hwnd, wparam as i32), i32::from(lparam != 0)) };
            1
        }
        TDM_ENABLE_RADIO_BUTTON => {
            let radios = with_task(h, |t| t.radios.clone()).unwrap_or_default();
            if let Some(i) = radios.iter().position(|r| *r == wparam as i32) {
                unsafe {
                    EnableWindow(
                        GetDlgItem(hwnd, ID_RADIO + i as i32),
                        i32::from(lparam != 0),
                    )
                };
            }
            1
        }
        TDM_CLICK_VERIFICATION => {
            if with_task(h, |t| t.has_verification).unwrap_or(false) {
                unsafe { CheckDlgButton(hwnd, ID_VERIFY, u32::from(wparam != 0)) };
                with_task(h, |t| t.verified = wparam != 0);
                notify(h, TDN_VERIFICATION_CLICKED, u64::from(wparam != 0), 0);
            }
            1
        }
        WM_DESTROY => {
            notify(h, TDN_DESTROYED, 0, 0);
            unsafe { KillTimer(hwnd, TIMER_ID) };
            let mut tasks = TASKS.lock();
            if let Some(i) = tasks.iter().position(|t| t.hwnd == h) {
                let t = tasks.remove(i);
                *LAST_RESULT.lock() = (t.radio as u32 as u64, u64::from(t.verified));
                release(&t);
            }
            0
        }
        _ => 0,
    }
}
fn control_id(dialog: usize, child: usize) -> i32 {
    // The child is one of the template's controls: find its ID by handle.
    for id in [
        ID_INSTRUCTION,
        ID_CONTENT,
        ID_EXPANDED,
        ID_FOOTER,
        ID_VERIFY,
    ] {
        if unsafe { GetDlgItem(dialog, id) } == child {
            return id;
        }
    }
    let ids = with_task(widen(dialog), |t| {
        let mut all = t.buttons.clone();
        all.extend((0..t.radios.len() as i32).map(|i| ID_RADIO + i));
        all
    })
    .unwrap_or_default();
    ids.into_iter()
        .find(|id| unsafe { GetDlgItem(dialog, *id) } == child)
        .unwrap_or(0)
}
/// Is button `id` a command link (on the white area)?
fn is_link(dialog: usize, id: i32) -> bool {
    with_task(widen(dialog), |t| {
        t.flags & (TDF_USE_COMMAND_LINKS | TDF_USE_COMMAND_LINKS_NO_ICON) != 0
            && !COMMON.iter().any(|(_, c, _)| *c == id)
    })
    .unwrap_or(false)
}

#[unsafe(no_mangle)]
pub extern "system" fn TaskDialogIndirect(
    config: usize,
    button: usize,
    radio: usize,
    verified: usize,
) -> u32 {
    let Some(c) = read_config(widen(config)) else {
        return E_INVALIDARG;
    };
    run(&c, widen(button), widen(radio), widen(verified))
}

#[unsafe(no_mangle)]
pub extern "system" fn TaskDialog(
    owner: usize,
    instance: usize,
    title: usize,
    instruction: usize,
    content: usize,
    common: u32,
    icon: usize,
    button: usize,
) -> u32 {
    let instance = widen(instance);
    let c = Config {
        owner: widen(owner),
        instance,
        flags: 0,
        common,
        title: text(instance, widen(title)),
        icon: widen(icon),
        instruction: text(instance, widen(instruction)),
        content: text(instance, widen(content)),
        buttons: Vec::new(),
        default_button: 0,
        radios: Vec::new(),
        default_radio: 0,
        verification: Vec::new(),
        expanded: Vec::new(),
        footer: Vec::new(),
        callback: 0,
        data: 0,
        width: 0,
    };
    run(&c, widen(button), 0, 0)
}
