//! PE exports of comdlg32. The dialog is an ordinary user32 dialog built
//! from an in-memory DIALOGEX template; an Explorer-style hook procedure
//! becomes the dialog procedure of a child page, which receives the CDN_*
//! notifications through WM_NOTIFY and may drive the dialog with CDM_*
//! messages. Files are listed and checked through ntdll's file service.
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use door_windows_abi::layout::{self, Field, Field::*};
use door_windows_gui::ofn::{self, Exists, Outcome, Request, error, flags};
use door_windows_gui_guest as g;

#[global_allocator]
static HEAP: door_windows_abi::allocator::ProcessHeap = door_windows_abi::allocator::ProcessHeap;

#[link(name = "user32", kind = "dylib")]
unsafe extern "system" {
    fn DialogBoxIndirectParamW(
        instance: usize,
        template: usize,
        owner: usize,
        procedure: usize,
        param: isize,
    ) -> isize;
    fn CreateDialogIndirectParamW(
        instance: usize,
        template: usize,
        owner: usize,
        procedure: usize,
        param: isize,
    ) -> usize;
    fn SendMessageW(hwnd: usize, message: u32, wparam: usize, lparam: isize) -> isize;
    fn PostMessageW(hwnd: usize, message: u32, wparam: usize, lparam: isize) -> i32;
    fn GetDlgItem(dialog: usize, id: i32) -> usize;
    fn SetFocus(hwnd: usize) -> usize;
    fn SetDlgItemTextW(dialog: usize, id: i32, text: usize) -> i32;
    fn GetDlgItemTextW(dialog: usize, id: i32, text: usize, capacity: i32) -> u32;
    fn EndDialog(dialog: usize, result: isize) -> i32;
    fn GetWindowLongPtrW(hwnd: usize, index: i32) -> usize;
    fn SetWindowLongPtrW(hwnd: usize, index: i32, value: usize) -> usize;
    fn MessageBoxW(owner: usize, text: usize, caption: usize, kind: u32) -> i32;
}
#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorFindFirstFileW(pattern: u64, destination: u64) -> u64;
    fn DoorFindNextFileW(handle: u64, destination: u64) -> i32;
    fn DoorFindClose(handle: u64) -> i32;
    fn DoorGetFileAttributesW(path: u64) -> u32;
    fn DoorGetCurrentDirectoryW(capacity: u32, buffer: u64) -> u32;
}

const WM_INITDIALOG: u32 = 0x0110;
const WM_COMMAND: u32 = 0x0111;
const WM_NOTIFY: u32 = 0x004e;
const WM_USER: u32 = 0x0400;
const WM_FOLDER_READY: u32 = 0x8000 + 0x3f0;
const WM_PAINT: u32 = 0x000f;
const EM_SETSEL: u32 = 0x00b1;
const CDM_GETSPEC: u32 = WM_USER + 100;
const CDM_GETFILEPATH: u32 = WM_USER + 101;
const CDM_GETFOLDERPATH: u32 = WM_USER + 102;
const CDM_SETCONTROLTEXT: u32 = WM_USER + 104;
const CDM_SETDEFEXT: u32 = WM_USER + 106;
const CDN_INITDONE: u32 = 0u32.wrapping_sub(601);
const CDN_FOLDERCHANGE: u32 = 0u32.wrapping_sub(603);
const CDN_FILEOK: u32 = 0u32.wrapping_sub(606);
const IDOK: u32 = 1;
const IDCANCEL: u32 = 2;
const STC2: u32 = 0x441;
const STC3: u32 = 0x442;
const STC4: u32 = 0x443;
const LST1: u32 = 0x460;
const CMB1: u32 = 0x470;
const EDT1: u32 = 0x480;
const LBN_SELCHANGE: u32 = 1;
const LBN_DBLCLK: u32 = 2;
const LB_ADDSTRING: u32 = 0x0180;
const LB_RESETCONTENT: u32 = 0x0184;
const LB_GETCURSEL: u32 = 0x0188;
const LB_GETTEXT: u32 = 0x0189;
const LB_SETITEMDATA: u32 = 0x019a;
const LB_GETITEMDATA: u32 = 0x0199;
const DWLP_MSGRESULT: i32 = 0;
const DWLP_USER: i32 = 16;
const ATTRIBUTE_DIRECTORY: u32 = 0x10;

/// CommDlgExtendedError is per thread, as on Windows: a bounded table of
/// (thread id, code). Slots holding 0 are free; a thread without a slot
/// reads 0.
const ERROR_SLOTS: usize = 64;
static ERRORS: g::Lock<[(u32, u32); ERROR_SLOTS]> = g::Lock::new([(0, 0); ERROR_SLOTS]);

fn set_error(code: u32) {
    let tid = g::thread_id();
    let mut table = ERRORS.lock();
    if let Some(slot) = table.iter_mut().find(|s| s.0 == tid) {
        slot.1 = code;
    } else if code != 0 {
        // A free slot, else (every slot busy) a fixed victim.
        let i = table
            .iter()
            .position(|s| s.1 == 0)
            .unwrap_or(tid as usize % ERROR_SLOTS);
        table[i] = (tid, code);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn CommDlgExtendedError() -> u32 {
    let tid = g::thread_id();
    ERRORS.lock().iter().find(|s| s.0 == tid).map_or(0, |s| s.1)
}

/// Dialogs running at once (nested and on different threads).
const MAX_DIALOGS: usize = 32;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// One running dialog, found by its `id` (handed to WM_INITDIALOG) until
/// its window exists and by `dialog` afterwards; each invocation owns its
/// own context, so concurrent and nested dialogs never share one.
struct Context {
    id: u64,
    ofn: u64,
    flags: u32,
    save: bool,
    instance: u64,
    file: u64,
    capacity: usize,
    hook: u64,
    default_extension: Vec<u16>,
    filters: Vec<(Vec<u16>, Vec<u16>)>,
    filter: usize,
    directory: Vec<u16>,
    initial_name: Vec<u16>,
    title: Vec<u16>,
    page: u64,
    dialog: u64,
    /// The initial folder view is shown after the dialog first paints.
    folder_pending: bool,
}

static CONTEXTS: g::Lock<Vec<Context>> = g::Lock::new(Vec::new());

fn with_context<R>(dialog: u64, f: impl FnOnce(&mut Context) -> R) -> Option<R> {
    let mut list = CONTEXTS.lock();
    list.iter_mut().find(|c| c.dialog == dialog).map(f)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
fn z(v: &[u16]) -> Vec<u16> {
    v.iter().copied().chain(core::iter::once(0)).collect()
}

fn attributes(path: &[u16]) -> Option<u32> {
    let p = z(path);
    // SAFETY: ntdll captures our NUL-terminated path.
    let a = unsafe { DoorGetFileAttributesW(p.as_ptr() as u64) };
    (a != u32::MAX).then_some(a)
}
fn exists(path: &[u16]) -> Exists {
    match attributes(path) {
        None => Exists::Missing,
        Some(a) if a & ATTRIBUTE_DIRECTORY != 0 => Exists::Directory,
        Some(_) => Exists::File,
    }
}

/// (name, directory) entries of `directory` matching the filter.
fn list(directory: &[u16], patterns: &[u16]) -> Vec<(Vec<u16>, bool)> {
    let mut pattern = directory.to_vec();
    if pattern.last() != Some(&92) {
        pattern.push(92);
    }
    pattern.push(42);
    let p = z(&pattern);
    let mut data = alloc::vec![0u8; 592];
    let mut out = Vec::new();
    // SAFETY: our own pattern and record buffers.
    let handle = unsafe { DoorFindFirstFileW(p.as_ptr() as u64, data.as_mut_ptr() as u64) };
    if handle == u64::MAX || handle == 0 {
        return out;
    }
    loop {
        let attrs = u32::from_le_bytes(data[..4].try_into().unwrap());
        let name: Vec<u16> = data[44..44 + 520]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|&u| u != 0)
            .collect();
        let dir = attrs & ATTRIBUTE_DIRECTORY != 0;
        if !name.is_empty() && (dir || ofn::matches(patterns, &name)) && out.len() < 4096 {
            out.push((name, dir));
        }
        if unsafe { DoorFindNextFileW(handle, data.as_mut_ptr() as u64) } == 0 {
            break;
        }
    }
    unsafe { DoorFindClose(handle) };
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

fn fill_list(dialog: u64) {
    let Some((directory, pattern)) = with_context(dialog, |c| {
        let pattern = c
            .filters
            .get(c.filter)
            .map_or_else(|| wide("*.*"), |f| f.1.clone());
        (c.directory.clone(), pattern)
    }) else {
        return;
    };
    let lst = unsafe { GetDlgItem(door_windows_abi::narrow(dialog), LST1 as i32) };
    unsafe { SendMessageW(lst, LB_RESETCONTENT, 0, 0) };
    for (name, dir) in list(&directory, &pattern) {
        let shown = if dir {
            let mut v = wide("[");
            v.extend_from_slice(&name);
            v.push(93);
            z(&v)
        } else {
            z(&name)
        };
        let i = unsafe { SendMessageW(lst, LB_ADDSTRING, 0, (shown.as_ptr() as i64) as isize) };
        if i >= 0 {
            unsafe {
                SendMessageW(
                    lst,
                    LB_SETITEMDATA,
                    door_windows_abi::narrow(i as u64),
                    (i64::from(dir)) as isize,
                )
            };
        }
    }
    let shown = z(&directory);
    unsafe {
        SetDlgItemTextW(
            door_windows_abi::narrow(dialog),
            STC4 as i32,
            door_windows_abi::narrow(shown.as_ptr() as u64),
        )
    };
}

/// WM_NOTIFY with an OFNOTIFYW to the hook page; returns the hook's verdict.
fn notify(dialog: u64, code: u32) -> i64 {
    let Some((page, ofn)) = with_context(dialog, |c| (c.page, c.ofn)) else {
        return 0;
    };
    if page == 0 {
        return 0;
    }
    let mut note = [0u8; layout::size(&OFNOTIFYW)];
    layout::set(&mut note, &OFNOTIFYW, 0, dialog);
    layout::set(&mut note, &OFNOTIFYW, 2, u64::from(code));
    layout::set(&mut note, &OFNOTIFYW, 3, ofn);
    unsafe {
        SetWindowLongPtrW(door_windows_abi::narrow(page), DWLP_MSGRESULT, 0);
        let r = SendMessageW(
            door_windows_abi::narrow(page),
            WM_NOTIFY,
            0,
            (note.as_ptr() as i64) as isize,
        );
        if r != 0 {
            return (r) as i64;
        }
        GetWindowLongPtrW(door_windows_abi::narrow(page), DWLP_MSGRESULT) as i64
    }
}

fn item_text(dialog: u64, id: u32) -> Vec<u16> {
    let mut buf = alloc::vec![0u16; 1024];
    let n = unsafe {
        GetDlgItemTextW(
            door_windows_abi::narrow(dialog),
            id as i32,
            door_windows_abi::narrow(buf.as_mut_ptr() as u64),
            buf.len() as i32,
        )
    } as usize;
    buf.truncate(n.min(1023));
    buf
}

fn current_path(dialog: u64) -> Option<Vec<u16>> {
    let (dir, name) =
        with_context(dialog, |c| c.directory.clone()).map(|d| (d, item_text(dialog, EDT1)))?;
    ofn::join(&dir, &name)
}

/// OK: resolve the typed name and either finish or stay open.
fn accept(dialog: u64) {
    let typed = item_text(dialog, EDT1);
    let Some((flags_, save, ext, capacity, directory, file, ofn_ptr)) = with_context(dialog, |c| {
        (
            c.flags,
            c.save,
            c.default_extension.clone(),
            c.capacity,
            c.directory.clone(),
            c.file,
            c.ofn,
        )
    }) else {
        return;
    };
    let req = Request {
        flags: flags_,
        save,
        default_extension: (!ext.is_empty()).then_some(&ext[..]),
        capacity,
    };
    match ofn::resolve(&req, &directory, &typed, &exists) {
        Outcome::Accept {
            path,
            file_offset,
            extension_offset,
        } => {
            if save && flags_ & flags::OVERWRITEPROMPT != 0 && exists(&path) == Exists::File {
                let mut text = path.clone();
                text.extend(wide(" already exists.\nDo you want to replace it?"));
                let t = z(&text);
                let caption = z(&wide("Confirm Save As"));
                // MB_YESNO | MB_ICONWARNING
                if unsafe {
                    MessageBoxW(
                        door_windows_abi::narrow(dialog),
                        door_windows_abi::narrow(t.as_ptr() as u64),
                        door_windows_abi::narrow(caption.as_ptr() as u64),
                        0x34,
                    )
                } != 6
                {
                    return;
                }
            }
            // The path is published before CDN_FILEOK so the hook can read it.
            let bytes: Vec<u8> = z(&path).iter().flat_map(|u| u.to_le_bytes()).collect();
            if !g::write(file, &bytes) {
                set_error(error::INVALIDFILENAME);
                unsafe { EndDialog(door_windows_abi::narrow(dialog), 0) };
                return;
            }
            let mut offsets = [0u8; 4];
            offsets[..2].copy_from_slice(&file_offset.to_le_bytes());
            offsets[2..].copy_from_slice(&extension_offset.to_le_bytes());
            g::write(
                ofn_ptr + layout::offset(&OPENFILENAMEW, member::FILE_OFFSET) as u64,
                &offsets,
            );
            if notify(dialog, CDN_FILEOK) != 0 {
                return;
            }
            // lpstrFileTitle receives the file name when requested.
            let at = |index| ofn_ptr + layout::offset(&OPENFILENAMEW, index) as u64;
            if let (Some(title), Some(cap)) = (
                g::read_pointer(at(member::FILE_TITLE)),
                g::read_u32(at(member::MAX_FILE_TITLE)),
            ) && title != 0
                && cap != 0
            {
                g::write_wide(title, cap as usize, &path[file_offset as usize..]);
            }
            set_error(0);
            unsafe { EndDialog(door_windows_abi::narrow(dialog), 1) };
        }
        Outcome::BufferTooSmall { required, .. } => {
            // The first WORD of lpstrFile receives the size needed. The call
            // still reports success (measured on Windows 11).
            g::write(file, &(required.min(0xffff) as u16).to_le_bytes());
            set_error(error::BUFFERTOOSMALL);
            unsafe { EndDialog(door_windows_abi::narrow(dialog), 1) };
        }
        Outcome::Navigate(dir) => {
            with_context(dialog, |c| c.directory = dir);
            let empty = [0u16];
            unsafe {
                SetDlgItemTextW(
                    door_windows_abi::narrow(dialog),
                    EDT1 as i32,
                    door_windows_abi::narrow(empty.as_ptr() as u64),
                )
            };
            fill_list(dialog);
            notify(dialog, CDN_FOLDERCHANGE);
        }
        Outcome::Filter(pattern) => {
            with_context(dialog, |c| {
                c.filters.insert(0, (pattern.clone(), pattern));
                c.filter = 0;
            });
            fill_list(dialog);
        }
        Outcome::NotFound(p) | Outcome::PathNotFound(p) => {
            let mut text = p;
            text.extend(wide(
                "\nFile not found.\nCheck the file name and try again.",
            ));
            let t = z(&text);
            let caption = with_context(dialog, |c| z(&c.title)).unwrap_or_default();
            unsafe {
                MessageBoxW(
                    door_windows_abi::narrow(dialog),
                    door_windows_abi::narrow(t.as_ptr() as u64),
                    door_windows_abi::narrow(caption.as_ptr() as u64),
                    0x30,
                )
            };
        }
        Outcome::InvalidName => {
            if !typed.is_empty() {
                let t = z(&wide("The file name is not valid."));
                let caption = with_context(dialog, |c| z(&c.title)).unwrap_or_default();
                unsafe {
                    MessageBoxW(
                        door_windows_abi::narrow(dialog),
                        door_windows_abi::narrow(t.as_ptr() as u64),
                        door_windows_abi::narrow(caption.as_ptr() as u64),
                        0x30,
                    )
                };
            }
        }
    }
}

extern "system" fn dialog_proc(dialog: usize, message: u32, wparam: usize, lparam: isize) -> isize {
    let dialog = door_windows_abi::widen(dialog);
    let wparam = door_windows_abi::widen(wparam);
    let lparam = lparam as i64;
    ((move || -> i64 {
        match message {
            WM_INITDIALOG => {
                let id = lparam as u64;
                let (name, hook, instance, ofn_ptr, filter_name) = {
                    let mut list = CONTEXTS.lock();
                    let Some(c) = list.iter_mut().find(|c| c.id == id && c.dialog == 0) else {
                        return 0;
                    };
                    c.dialog = dialog;
                    let filter_name = c
                        .filters
                        .get(c.filter)
                        .map(|f| f.0.clone())
                        .unwrap_or_default();
                    (
                        c.initial_name.clone(),
                        c.hook,
                        c.instance,
                        c.ofn,
                        filter_name,
                    )
                };
                unsafe {
                    SetWindowLongPtrW(
                        door_windows_abi::narrow(dialog),
                        DWLP_USER,
                        door_windows_abi::narrow(id),
                    )
                };
                let n = z(&name);
                unsafe {
                    SetDlgItemTextW(
                        door_windows_abi::narrow(dialog),
                        EDT1 as i32,
                        door_windows_abi::narrow(n.as_ptr() as u64),
                    )
                };
                let f = z(&filter_name);
                unsafe {
                    SetDlgItemTextW(
                        door_windows_abi::narrow(dialog),
                        CMB1 as i32,
                        door_windows_abi::narrow(f.as_ptr() as u64),
                    )
                };
                fill_list(dialog);
                if hook != 0 {
                    let page = page_template();
                    // SAFETY: the template lives across the call.
                    let p = unsafe {
                        CreateDialogIndirectParamW(
                            door_windows_abi::narrow(instance),
                            door_windows_abi::narrow(page.as_ptr() as u64),
                            door_windows_abi::narrow(dialog),
                            door_windows_abi::narrow(hook),
                            (ofn_ptr as i64) as isize,
                        )
                    };
                    with_context(dialog, |c| c.page = door_windows_abi::widen(p));
                }
                notify(dialog, CDN_INITDONE);
                // As on Windows, the folder view is browsed once the dialog is on
                // screen: CDN_FOLDERCHANGE follows its first paint, so a hook that
                // answers it never closes a dialog that was never shown.
                with_context(dialog, |c| c.folder_pending = true);
                // The File name box starts with the focus and its text selected
                // (FALSE: this procedure set the focus itself).
                unsafe {
                    let name = GetDlgItem(door_windows_abi::narrow(dialog), EDT1 as i32);
                    SetFocus(name);
                    SendMessageW(name, EM_SETSEL, 0, -1);
                }
                0
            }
            WM_PAINT => {
                if with_context(dialog, |c| core::mem::take(&mut c.folder_pending)) == Some(true) {
                    unsafe {
                        PostMessageW(door_windows_abi::narrow(dialog), WM_FOLDER_READY, 0, 0)
                    };
                }
                0
            }
            WM_FOLDER_READY => {
                notify(dialog, CDN_FOLDERCHANGE);
                1
            }
            CDM_SETCONTROLTEXT => {
                unsafe {
                    SetDlgItemTextW(
                        door_windows_abi::narrow(dialog),
                        wparam as i32,
                        door_windows_abi::narrow(lparam as u64),
                    )
                };
                1
            }
            CDM_GETSPEC | CDM_GETFILEPATH | CDM_GETFOLDERPATH => {
                let text = match message {
                    CDM_GETSPEC => item_text(dialog, EDT1),
                    CDM_GETFILEPATH => current_path(dialog).unwrap_or_default(),
                    _ => with_context(dialog, |c| c.directory.clone()).unwrap_or_default(),
                };
                let needed = text.len() + 1;
                if lparam != 0 && wparam as usize >= needed {
                    g::write_wide(lparam as u64, wparam as usize, &text);
                }
                unsafe {
                    SetWindowLongPtrW(
                        door_windows_abi::narrow(dialog),
                        DWLP_MSGRESULT,
                        door_windows_abi::narrow(needed as u64),
                    )
                };
                1
            }
            CDM_SETDEFEXT => {
                let ext = g::read_wide(lparam as u64, 64).unwrap_or_default();
                with_context(dialog, |c| c.default_extension = ext);
                1
            }
            WM_COMMAND => {
                let (id, code) = ((wparam & 0xffff) as u32, ((wparam >> 16) & 0xffff) as u32);
                match id {
                    IDOK => accept(dialog),
                    IDCANCEL => {
                        set_error(0);
                        unsafe { EndDialog(door_windows_abi::narrow(dialog), 0) };
                    }
                    LST1 if code == LBN_SELCHANGE || code == LBN_DBLCLK => {
                        let lst =
                            unsafe { GetDlgItem(door_windows_abi::narrow(dialog), LST1 as i32) };
                        let i = unsafe { SendMessageW(lst, LB_GETCURSEL, 0, 0) };
                        if i < 0 {
                            return 1;
                        }
                        let dir = unsafe {
                            SendMessageW(lst, LB_GETITEMDATA, door_windows_abi::narrow(i as u64), 0)
                        } != 0;
                        let mut buf = alloc::vec![0u16; 520];
                        let n = unsafe {
                            SendMessageW(
                                lst,
                                LB_GETTEXT,
                                door_windows_abi::narrow(i as u64),
                                (buf.as_mut_ptr() as i64) as isize,
                            )
                        };
                        if n <= 0 {
                            return 1;
                        }
                        buf.truncate(n as usize);
                        if dir {
                            buf = buf[1..buf.len() - 1].to_vec();
                        }
                        let shown = z(&buf);
                        unsafe {
                            SetDlgItemTextW(
                                door_windows_abi::narrow(dialog),
                                EDT1 as i32,
                                door_windows_abi::narrow(shown.as_ptr() as u64),
                            )
                        };
                        if code == LBN_DBLCLK {
                            accept(dialog);
                        }
                    }
                    _ => return 0,
                }
                1
            }
            _ => 0,
        }
    })()) as isize
}

fn push(b: &mut Vec<u8>, v: &[u8]) {
    b.extend_from_slice(v);
}
fn item(b: &mut Vec<u8>, style: u32, r: [i16; 4], id: u32, class: u16, text: &str) {
    while !b.len().is_multiple_of(4) {
        b.push(0);
    }
    for v in [0u32, 0, style | 0x5000_0000] {
        push(b, &v.to_le_bytes());
    }
    for v in r {
        push(b, &v.to_le_bytes());
    }
    push(b, &id.to_le_bytes());
    push(b, &[0xff, 0xff]);
    push(b, &class.to_le_bytes());
    for u in text.encode_utf16().chain(core::iter::once(0)) {
        push(b, &u.to_le_bytes());
    }
    push(b, &[0, 0]);
}
fn header(b: &mut Vec<u8>, style: u32, count: u16, cx: i16, cy: i16, title: &[u16]) {
    push(b, &1u16.to_le_bytes());
    push(b, &0xffffu16.to_le_bytes());
    for v in [0u32, 0, style] {
        push(b, &v.to_le_bytes());
    }
    push(b, &count.to_le_bytes());
    for v in [0i16, 0, cx, cy] {
        push(b, &v.to_le_bytes());
    }
    push(b, &[0, 0, 0, 0]);
    for u in title.iter().take(255).chain(core::iter::once(&0)) {
        push(b, &u.to_le_bytes());
    }
    if style & 0x40 != 0 {
        push(b, &9u16.to_le_bytes());
        push(b, &400u16.to_le_bytes());
        push(b, &[0, 1]);
        for u in "Segoe UI\0".encode_utf16() {
            push(b, &u.to_le_bytes());
        }
    }
}

fn dialog_template(save: bool, title: &[u16]) -> Vec<u8> {
    let mut b = Vec::new();
    // WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME | DS_SETFONT
    header(&mut b, 0x80c8_0080 | 0x40, 9, 280, 196, title);
    item(&mut b, 0, [7, 6, 40, 9], 0xffff, 0x82, "Look &in:");
    item(&mut b, 0x80, [50, 6, 222, 9], STC4, 0x82, "");
    // LBS_NOTIFY | WS_VSCROLL | WS_BORDER | WS_TABSTOP
    item(
        &mut b,
        0x0001 | 0x0020_0000 | 0x0080_0000 | 0x0001_0000,
        [7, 20, 266, 112],
        LST1,
        0x83,
        "",
    );
    item(&mut b, 0, [7, 140, 50, 9], STC3, 0x82, "File &name:");
    // ES_AUTOHSCROLL | WS_BORDER | WS_TABSTOP
    item(
        &mut b,
        0x0080 | 0x0080_0000 | 0x0001_0000,
        [62, 138, 150, 13],
        EDT1,
        0x81,
        "",
    );
    item(
        &mut b,
        0,
        [7, 160, 50, 9],
        STC2,
        0x82,
        if save {
            "Save as &type:"
        } else {
            "Files of &type:"
        },
    );
    item(&mut b, 0x80, [62, 160, 150, 9], CMB1, 0x82, "");
    // BS_DEFPUSHBUTTON | WS_TABSTOP
    item(
        &mut b,
        0x0001 | 0x0001_0000,
        [220, 137, 53, 15],
        IDOK,
        0x80,
        if save { "&Save" } else { "&Open" },
    );
    item(
        &mut b,
        0x0001_0000,
        [220, 157, 53, 15],
        IDCANCEL,
        0x80,
        "Cancel",
    );
    b
}
/// The empty child page that hosts an Explorer-style hook procedure.
fn page_template() -> Vec<u8> {
    let mut b = Vec::new();
    // WS_CHILD | WS_CLIPSIBLINGS | DS_CONTROL | DS_3DLOOK
    header(&mut b, 0x4400_0000 | 0x0400 | 0x0004, 0, 0, 0, &[]);
    b
}

/// OPENFILENAMEW, at the build's width (P17): lStructSize, hwndOwner,
/// hInstance, lpstrFilter, lpstrCustomFilter, nMaxCustFilter, nFilterIndex,
/// lpstrFile, nMaxFile, lpstrFileTitle, nMaxFileTitle, lpstrInitialDir,
/// lpstrTitle, Flags, nFileOffset, nFileExtension, lpstrDefExt, lCustData,
/// lpfnHook, lpTemplateName, pvReserved, dwReserved, FlagsEx.
const OPENFILENAMEW: [Field; 23] = [
    U32, Pointer, Pointer, Pointer, Pointer, U32, U32, Pointer, U32, Pointer, U32, Pointer,
    Pointer, U32, U16, U16, Pointer, Pointer, Pointer, Pointer, Pointer, U32, U32,
];
mod member {
    pub const OWNER: usize = 1;
    pub const INSTANCE: usize = 2;
    pub const FILTER: usize = 3;
    pub const FILTER_INDEX: usize = 6;
    pub const FILE: usize = 7;
    pub const MAX_FILE: usize = 8;
    pub const FILE_TITLE: usize = 9;
    pub const MAX_FILE_TITLE: usize = 10;
    pub const INITIAL_DIR: usize = 11;
    pub const TITLE: usize = 12;
    pub const FLAGS: usize = 13;
    pub const FILE_OFFSET: usize = 14;
    pub const DEF_EXT: usize = 16;
    pub const HOOK: usize = 18;
}
/// OFNOTIFYW: NMHDR (hwndFrom, idFrom, code), lpOFN, pszFile.
const OFNOTIFYW: [Field; 5] = [Pointer, Pointer, U32, Pointer, Pointer];

fn run(ofn_ptr: u64, save: bool) -> i32 {
    set_error(0);
    let Some(size) = g::read_u32(ofn_ptr) else {
        set_error(error::INITIALIZATION);
        return 0;
    };
    // The full structure or OPENFILENAME_SIZE_VERSION_400 (without
    // pvReserved, dwReserved and FlagsEx): 152 or 136 bytes on x64, 88 or
    // 76 on x86 (P17).
    if size as usize != layout::size(&OPENFILENAMEW)
        && size as usize != layout::size(&OPENFILENAMEW[..20])
    {
        set_error(error::STRUCTSIZE);
        return 0;
    }
    let Some(o) = g::read_bytes(ofn_ptr, size as usize) else {
        set_error(error::INITIALIZATION);
        return 0;
    };
    let field = |index| layout::get(&o, &OPENFILENAMEW, index);
    let flags_ = field(member::FLAGS) as u32;
    if flags_ & (flags::ENABLETEMPLATE | flags::ENABLETEMPLATEHANDLE) != 0 {
        set_error(error::NOTEMPLATE);
        return 0;
    }
    if flags_ & flags::ALLOWMULTISELECT != 0 {
        set_error(error::DIALOGFAILURE);
        return 0;
    }
    let file = field(member::FILE);
    let capacity = (field(member::MAX_FILE) as u32) as usize;
    if file == 0 || capacity == 0 {
        set_error(error::INITIALIZATION);
        return 0;
    }
    let read_opt = |p: u64, max: usize| {
        if p == 0 {
            Vec::new()
        } else {
            g::read_wide(p, max).unwrap_or_default()
        }
    };
    // lpstrFilter is a double-NUL-terminated list.
    let mut filter_units = Vec::new();
    let mut at = field(member::FILTER);
    if at != 0 {
        for _ in 0..64 {
            let part = read_opt(at, 512);
            if part.is_empty() {
                break;
            }
            at += (part.len() as u64 + 1) * 2;
            filter_units.extend_from_slice(&part);
            filter_units.push(0);
        }
    }
    filter_units.push(0);
    let filters = ofn::parse_filter(&filter_units);
    let filter = ((field(member::FILTER_INDEX) as u32).max(1) as usize - 1)
        .min(filters.len().saturating_sub(1));
    let initial = read_opt(file, capacity);
    let mut directory = read_opt(field(member::INITIAL_DIR), 1024);
    let mut initial_name = initial.clone();
    if let Some(p) = initial.iter().rposition(|&u| u == 92) {
        let (d, n) = initial.split_at(p);
        if directory.is_empty() {
            directory = d.to_vec();
        }
        initial_name = n[1..].to_vec();
    }
    if directory.is_empty() {
        let mut buf = alloc::vec![0u16; 1024];
        let n = unsafe { DoorGetCurrentDirectoryW(1024, buf.as_mut_ptr() as u64) } as usize;
        buf.truncate(n.min(1023));
        directory = buf;
    }
    while directory.len() > 3 && directory.last() == Some(&92) {
        directory.pop();
    }
    let title = {
        let t = read_opt(field(member::TITLE), 256);
        if t.is_empty() {
            wide(if save { "Save As" } else { "Open" })
        } else {
            t
        }
    };
    let hook = if flags_ & flags::ENABLEHOOK != 0 {
        field(member::HOOK)
    } else {
        0
    };
    let template = dialog_template(save, &title);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let owner = field(member::OWNER);
    {
        let mut list = CONTEXTS.lock();
        if list.len() >= MAX_DIALOGS || list.try_reserve(1).is_err() {
            drop(list);
            set_error(error::DIALOGFAILURE);
            return 0;
        }
        list.push(Context {
            id,
            ofn: ofn_ptr,
            flags: flags_,
            save,
            instance: field(member::INSTANCE),
            file,
            capacity,
            hook,
            default_extension: read_opt(field(member::DEF_EXT), 64),
            filters,
            filter,
            directory,
            initial_name,
            title,
            page: 0,
            dialog: 0,
            folder_pending: false,
        });
    }
    // SAFETY: the template and context outlive the modal call.
    let result = unsafe {
        DialogBoxIndirectParamW(
            0,
            door_windows_abi::narrow(template.as_ptr() as u64),
            door_windows_abi::narrow(owner),
            door_windows_abi::narrow(dialog_proc as *const () as u64),
            (id as i64) as isize,
        )
    };
    // Only this invocation's context goes: other threads' and enclosing
    // dialogs keep theirs.
    CONTEXTS.lock().retain(|c| c.id != id);
    if result < 0 {
        set_error(error::DIALOGFAILURE);
        return 0;
    }
    i32::from(result > 0)
}

#[unsafe(no_mangle)]
pub extern "system" fn GetOpenFileNameW(ofn_ptr: usize) -> i32 {
    let ofn_ptr = door_windows_abi::widen(ofn_ptr);
    run(ofn_ptr, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSaveFileNameW(ofn_ptr: usize) -> i32 {
    let ofn_ptr = door_windows_abi::widen(ofn_ptr);
    run(ofn_ptr, true)
}

const _: () = assert!(STC2 + 1 == STC3 && LB_ADDSTRING == 0x180);

/// PrintDlgExW: Doors has no printers, so (as Windows does on a computer
/// with none installed) the dialog fails with PDERR_NODEFAULTPRN once the
/// PRINTDLGEXW (136 bytes on x64, 84 on x86) is valid.
#[unsafe(no_mangle)]
pub extern "system" fn PrintDlgExW(info: usize) -> u32 {
    const E_FAIL: u32 = 0x8000_4005;
    const E_INVALIDARG: u32 = 0x8007_0057;
    const E_POINTER: u32 = 0x8000_4003;
    const PDERR_NODEFAULTPRN: u32 = 0x1008;
    let info = door_windows_abi::widen(info);
    if info == 0 {
        return E_POINTER;
    }
    let size = if g::POINTER == 8 { 136 } else { 84 };
    if g::read_u32(info) != Some(size) {
        return E_INVALIDARG;
    }
    set_error(PDERR_NODEFAULTPRN);
    E_FAIL
}
