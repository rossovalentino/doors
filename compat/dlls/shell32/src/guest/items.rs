//! The rest of shell32's surface Chromium and Electron import (P18): the
//! process's explicit AppUserModelID, app bars (the taskbar's position;
//! Doors' taskbar is the only bar), Shell_NotifyIconGetRect (the icon's
//! cell, or the "Show hidden icons" chevron it sits behind), change
//! notification registrations (accepted; SHChangeNotify delivers no
//! events, as Explorer reads the volume itself), SHParseDisplayName,
//! SHOpenFolderAndSelectItems (File Explorer opens the folder; the items
//! are not selected), IShellItem for file-system paths, the desktop's
//! IShellFolder over absolute item ID lists (no enumeration or UI
//! objects), and each window's IPropertyStore (AppUserModel keys and
//! other scalar or string values, kept for the window's lifetime).
use super::*;
use alloc::boxed::Box;
use core::sync::atomic::{AtomicU32, Ordering};

#[link(name = "user32", kind = "dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
    fn SystemParametersInfoW(action: u32, param: u32, data: usize, update: u32) -> i32;
    fn IsWindow(hwnd: usize) -> i32;
    fn DoorNotifyIconRect(hwnd: u64, id: u32, out: u64) -> i32;
}

const E_NOTIMPL: u32 = 0x8000_4001;
const E_NOINTERFACE: u32 = 0x8000_4002;
const E_POINTER: u32 = 0x8000_4003;
const MK_E_NOOBJECT: u32 = 0x8004_01E5;
const FILE_NOT_FOUND: u32 = 0x8007_0002;

// ---- the explicit AppUserModelID -------------------------------------------------------
static APP_ID: Lock<Option<Vec<u16>>> = Lock::new(None);

#[unsafe(no_mangle)]
pub extern "system" fn SetCurrentProcessExplicitAppUserModelID(id: usize) -> u32 {
    match read_wide(door_windows_abi::widen(id), 128) {
        Ok(units) if !units.is_empty() => {
            APP_ID.with(|a| *a = Some(units));
            hr::S_OK
        }
        _ => hr::E_INVALIDARG,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrentProcessExplicitAppUserModelID(out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    if out == 0 {
        return E_POINTER;
    }
    let _ = write_pointer(out, 0);
    let Some(units) = APP_ID.with(|a| a.clone()) else {
        return hr::E_FAIL;
    };
    let bytes: Vec<u8> = units
        .iter()
        .chain([&0])
        .flat_map(|u| u.to_le_bytes())
        .collect();
    match task_copy(&bytes) {
        Ok(p) => {
            let _ = write_pointer(out, p);
            hr::S_OK
        }
        Err(_) => hr::E_OUTOFMEMORY,
    }
}
/// Recent documents: Doors keeps no Recent folder shortcuts.
#[unsafe(no_mangle)]
pub extern "system" fn SHAddToRecentDocs(_flags: u32, _item: usize) {}

// ---- app bars ----------------------------------------------------------------------------
/// The taskbar's rectangle: the screen below (or beside) the work area.
fn taskbar() -> ([i32; 4], u32) {
    let (cx, cy) = unsafe { (GetSystemMetrics(0), GetSystemMetrics(1)) };
    let mut work = [0i32; 4];
    // SPI_GETWORKAREA.
    if unsafe { SystemParametersInfoW(0x30, 0, work.as_mut_ptr() as usize, 0) } == 0 {
        work = [0, 0, cx, cy - 48];
    }
    // ABE_LEFT 0, ABE_TOP 1, ABE_RIGHT 2, ABE_BOTTOM 3.
    if work[1] > 0 {
        ([0, 0, cx, work[1]], 1)
    } else if work[0] > 0 {
        ([0, 0, work[0], cy], 0)
    } else if work[2] < cx {
        ([work[2], 0, cx, cy], 2)
    } else {
        ([0, work[3].min(cy), cx, cy], 3)
    }
}
/// SHAppBarMessage over APPBARDATA (cbSize, hWnd, uCallbackMessage,
/// uEdge, rc, lParam: 48 bytes on x64, 36 on x86). Doors' taskbar is
/// always on top and never hides; programs cannot register bars.
#[unsafe(no_mangle)]
pub extern "system" fn SHAppBarMessage(message: u32, data: usize) -> usize {
    let data = door_windows_abi::widen(data);
    let (edge_at, rect_at) = if POINTER == 8 { (20, 24) } else { (12, 16) };
    match message {
        // ABM_GETSTATE: ABS_ALWAYSONTOP.
        4 => 2,
        // ABM_GETTASKBARPOS.
        5 => {
            let (r, edge) = taskbar();
            let bytes: Vec<u8> = r.iter().flat_map(|v| v.to_le_bytes()).collect();
            usize::from(
                data != 0
                    && write(data + edge_at, &edge.to_le_bytes()).is_ok()
                    && write(data + rect_at, &bytes).is_ok(),
            )
        }
        // ABM_GETAUTOHIDEBAR(EX): no bar hides.
        7 | 11 => 0,
        // ABM_QUERYPOS, ABM_SETPOS: the rectangle stays as asked.
        2 | 3 => 1,
        // ABM_ACTIVATE, ABM_WINDOWPOSCHANGED, ABM_SETSTATE.
        6 | 9 | 10 => 1,
        // ABM_NEW, ABM_REMOVE, ABM_SETAUTOHIDEBAR(EX): no program bars.
        _ => 0,
    }
}

// ---- notification icons -------------------------------------------------------------------
/// Shell_NotifyIconGetRect over NOTIFYICONIDENTIFIER (cbSize, hWnd, uID,
/// guidItem: 40 bytes on x64, 28 on x86); GUID-identified icons are not
/// supported (Doors' icons are window and ID pairs).
#[unsafe(no_mangle)]
pub extern "system" fn Shell_NotifyIconGetRect(identifier: usize, out: usize) -> u32 {
    let (identifier, out) = (
        door_windows_abi::widen(identifier),
        door_windows_abi::widen(out),
    );
    if identifier == 0 || out == 0 {
        return hr::E_INVALIDARG;
    }
    let (size, id_at) = if POINTER == 8 { (40, 16) } else { (28, 8) };
    if read_u32(identifier) != Ok(size) {
        return hr::E_INVALIDARG;
    }
    let (Ok(window), Ok(id)) = (
        read_pointer(identifier + POINTER as u64),
        read_u32(identifier + id_at),
    ) else {
        return hr::E_INVALIDARG;
    };
    if unsafe { DoorNotifyIconRect(window, id, out) } != 0 {
        hr::S_OK
    } else {
        hr::E_FAIL
    }
}

// ---- change notifications ---------------------------------------------------------------
static NEXT_REGISTRATION: AtomicU32 = AtomicU32::new(1);
static REGISTRATIONS: Lock<Vec<u32>> = Lock::new(Vec::new());
/// SHChangeNotifyRegister (ordinal 2): a registration for `window`.
#[unsafe(no_mangle)]
pub extern "system" fn SHChangeNotifyRegister(
    window: usize,
    _sources: i32,
    _events: i32,
    _message: u32,
    _count: i32,
    _entries: usize,
) -> u32 {
    if unsafe { IsWindow(window) } == 0 {
        return 0;
    }
    let id = NEXT_REGISTRATION.fetch_add(1, Ordering::Relaxed);
    REGISTRATIONS.with(|r| r.push(id));
    id
}
/// SHChangeNotifyDeregister (ordinal 4).
#[unsafe(no_mangle)]
pub extern "system" fn SHChangeNotifyDeregister(id: u32) -> i32 {
    REGISTRATIONS.with(|r| match r.iter().position(|x| *x == id) {
        Some(i) => {
            r.remove(i);
            1
        }
        None => 0,
    })
}

// ---- paths and item ID lists ---------------------------------------------------------------
const SFGAO_CANCOPY: u32 = 0x1;
const SFGAO_CANMOVE: u32 = 0x2;
const SFGAO_CANRENAME: u32 = 0x10;
const SFGAO_CANDELETE: u32 = 0x20;
const SFGAO_STREAM: u32 = 0x0040_0000;
const SFGAO_FILESYSANCESTOR: u32 = 0x1000_0000;
const SFGAO_FOLDER: u32 = 0x2000_0000;
const SFGAO_FILESYSTEM: u32 = 0x4000_0000;
/// The SFGAO_ attributes of a file-system path.
fn shell_attributes(path: &str) -> Option<u32> {
    let a = attributes(path)?;
    let common =
        SFGAO_FILESYSTEM | SFGAO_CANCOPY | SFGAO_CANMOVE | SFGAO_CANRENAME | SFGAO_CANDELETE;
    Some(if a & DIRECTORY != 0 {
        common | SFGAO_FOLDER | SFGAO_FILESYSANCESTOR
    } else {
        common | SFGAO_STREAM
    })
}
/// The last part of `path` ("C:\" for a drive root).
fn leaf(path: &str) -> &str {
    let t = path.trim_end_matches('\\');
    match t.rfind('\\') {
        Some(i) if i + 1 < t.len() => &t[i + 1..],
        _ => path,
    }
}
fn parent(path: &str) -> Option<String> {
    let t = path.trim_end_matches('\\');
    let i = t.rfind('\\')?;
    Some(if i <= 2 {
        String::from(&t[..3.min(t.len())])
    } else {
        String::from(&t[..i])
    })
    .filter(|p| p.len() < t.len())
    .map(|p| {
        if p.len() == 2 {
            alloc::format!("{p}\\")
        } else {
            p
        }
    })
}
fn wide_task(text: &str) -> Result<u64, u32> {
    task_copy(&wide_bytes(text)).map_err(|_| hr::E_OUTOFMEMORY)
}
#[unsafe(no_mangle)]
pub extern "system" fn SHParseDisplayName(
    name: usize,
    _bind: usize,
    out: usize,
    wanted: u32,
    attributes_out: usize,
) -> u32 {
    let (name, out) = (door_windows_abi::widen(name), door_windows_abi::widen(out));
    let attributes_out = door_windows_abi::widen(attributes_out);
    if out == 0 {
        return E_POINTER;
    }
    let _ = write_pointer(out, 0);
    let Ok(text) = read_text(name) else {
        return hr::E_INVALIDARG;
    };
    let path = absolute(&text, None);
    let Some(a) = shell_attributes(&path) else {
        return FILE_NOT_FOUND;
    };
    let Some(list) = pidl::from_path(&path, a & SFGAO_FOLDER != 0).and_then(|l| task_copy(&l).ok())
    else {
        return FILE_NOT_FOUND;
    };
    if attributes_out != 0 {
        let _ = write(attributes_out, &(a & wanted).to_le_bytes());
    }
    let _ = write_pointer(out, list);
    hr::S_OK
}
/// File Explorer opens the folder (as for ShellExecute's "open" of a
/// directory); Doors' Explorer does not take a selection.
#[unsafe(no_mangle)]
pub extern "system" fn SHOpenFolderAndSelectItems(
    folder: usize,
    count: u32,
    items: usize,
    _flags: u32,
) -> u32 {
    let folder = door_windows_abi::widen(folder);
    let items = door_windows_abi::widen(items);
    // The folder itself, or (no folder list) the first item's folder.
    let path = list_path(folder).or_else(|| {
        (count > 0)
            .then(|| read_pointer(items).ok())
            .flatten()
            .and_then(list_path)
            .and_then(|p| parent(&p))
    });
    let Some(path) = path.filter(|p| is_directory(p)) else {
        return hr::E_INVALIDARG;
    };
    let request = Execute {
        verb: String::from("open"),
        file: path.clone(),
        parameters: String::new(),
        directory: None,
        keep_process: false,
        no_ui: true,
    };
    match execute(&request) {
        Ok(process) => {
            if let Some(p) = process {
                unsafe { DoorCloseHandle(p) };
            }
            hr::S_OK
        }
        Err(_) => match open_with(&path, false) {
            Ok(()) => hr::S_OK,
            Err(_) => hr::E_FAIL,
        },
    }
}

// ---- COM objects ------------------------------------------------------------------------------
const IID_IUNKNOWN: Guid = guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_ISHELLITEM: Guid = guid(
    0x4382_6D1E,
    0xE718,
    0x42EE,
    [0xBC, 0x55, 0xA1, 0xE2, 0x61, 0xC3, 0x7B, 0xFE],
);
const IID_ISHELLFOLDER: Guid = guid(0x0002_14E6, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IPROPERTYSTORE: Guid = guid(
    0x886D_8EEB,
    0x8CF2,
    0x4446,
    [0x8D, 0x02, 0xCD, 0xBA, 0x1D, 0xBD, 0xCF, 0x99],
);

/// An interface table of `N` functions.
#[repr(transparent)]
struct Vtbl<const N: usize>([*const (); N]);
// SAFETY: the tables are immutable function addresses.
unsafe impl<const N: usize> Sync for Vtbl<N> {}
/// A COM object: its table, reference count and data.
#[repr(C)]
struct Com<T> {
    vtbl: *const (),
    refs: AtomicU32,
    data: T,
}
trait Kind: Sized + 'static {
    const IID: Guid;
    fn vtbl() -> *const ();
}
fn new_object<T: Kind>(data: T) -> u64 {
    Box::into_raw(Box::new(Com {
        vtbl: T::vtbl(),
        refs: AtomicU32::new(1),
        data,
    })) as u64
}
fn data<'a, T: Kind>(this: usize) -> &'a T {
    // SAFETY: only our objects reach our tables.
    unsafe { &(*(this as *const Com<T>)).data }
}
extern "system" fn add_ref<T: Kind>(this: usize) -> u32 {
    unsafe {
        (*(this as *const Com<T>))
            .refs
            .fetch_add(1, Ordering::AcqRel)
            + 1
    }
}
extern "system" fn release<T: Kind>(this: usize) -> u32 {
    let left = unsafe {
        (*(this as *const Com<T>))
            .refs
            .fetch_sub(1, Ordering::AcqRel)
            - 1
    };
    if left == 0 {
        // SAFETY: the last reference to a Box::into_raw object.
        drop(unsafe { Box::from_raw(this as *mut Com<T>) });
    }
    left
}
extern "system" fn query<T: Kind>(this: usize, iid: usize, out: usize) -> u32 {
    let (iid, out) = (door_windows_abi::widen(iid), door_windows_abi::widen(out));
    if out == 0 || iid == 0 {
        return E_POINTER;
    }
    let mut wanted = [0u8; 16];
    if read(iid, &mut wanted).is_err() {
        return E_POINTER;
    }
    if wanted == IID_IUNKNOWN || wanted == T::IID {
        add_ref::<T>(this);
        let _ = write_pointer(out, this as u64);
        hr::S_OK
    } else {
        let _ = write_pointer(out, 0);
        E_NOINTERFACE
    }
}
/// A new object answered as `iid` into `out` (our reference released).
fn hand_out(
    object: u64,
    iid: u64,
    out: u64,
    release_fn: extern "system" fn(usize) -> u32,
    query_fn: extern "system" fn(usize, usize, usize) -> u32,
) -> u32 {
    let r = query_fn(
        door_windows_abi::narrow(object),
        door_windows_abi::narrow(iid),
        door_windows_abi::narrow(out),
    );
    release_fn(door_windows_abi::narrow(object));
    r
}

// ---- IShellItem -----------------------------------------------------------------------------------
struct Item {
    path: String,
}
impl Kind for Item {
    const IID: Guid = IID_ISHELLITEM;
    fn vtbl() -> *const () {
        &ITEM_VTBL as *const _ as *const ()
    }
}
static ITEM_VTBL: Vtbl<8> = Vtbl([
    query::<Item> as *const (),
    add_ref::<Item> as *const (),
    release::<Item> as *const (),
    item_bind as *const (),
    item_parent as *const (),
    item_name as *const (),
    item_attributes as *const (),
    item_compare as *const (),
]);
fn new_item(path: String, iid: u64, out: u64) -> u32 {
    hand_out(
        new_object(Item { path }),
        iid,
        out,
        release::<Item>,
        query::<Item>,
    )
}
extern "system" fn item_bind(
    _this: usize,
    _bind: usize,
    _handler: usize,
    _iid: usize,
    out: usize,
) -> u32 {
    let _ = write_pointer(door_windows_abi::widen(out), 0);
    E_NOTIMPL
}
extern "system" fn item_parent(this: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    if out == 0 {
        return E_POINTER;
    }
    let _ = write_pointer(out, 0);
    match parent(&data::<Item>(this).path) {
        Some(p) => {
            let iid = IID_ISHELLITEM;
            new_item(p, iid.as_ptr() as u64, out)
        }
        None => MK_E_NOOBJECT,
    }
}
/// SIGDN: the parsing, file-system and editing forms are the path; the URL
/// form is a file: URL; every parent-relative or normal display form is
/// the last part.
fn display_name(path: &str, sigdn: u32) -> String {
    match sigdn {
        0x8002_8000 | 0x8004_C000 | 0x8005_8000 => String::from(path),
        0x8006_8000 => alloc::format!("file:///{}", path.replace('\\', "/")),
        _ if path.len() <= 3 => alloc::format!("Local Disk ({})", &path[..2.min(path.len())]),
        _ => String::from(leaf(path)),
    }
}
extern "system" fn item_name(this: usize, sigdn: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    if out == 0 {
        return E_POINTER;
    }
    match wide_task(&display_name(&data::<Item>(this).path, sigdn)) {
        Ok(p) => {
            let _ = write_pointer(out, p);
            hr::S_OK
        }
        Err(e) => {
            let _ = write_pointer(out, 0);
            e
        }
    }
}
extern "system" fn item_attributes(this: usize, mask: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    if out == 0 {
        return E_POINTER;
    }
    let a = shell_attributes(&data::<Item>(this).path).unwrap_or(0) & mask;
    let _ = write(out, &a.to_le_bytes());
    if a == mask { hr::S_OK } else { hr::S_FALSE }
}
extern "system" fn item_compare(this: usize, other: usize, _hint: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    if other == 0 || out == 0 {
        return E_POINTER;
    }
    // Another of our items: by path; a foreign one never matches.
    let theirs = if unsafe { *(other as *const *const ()) } == Item::vtbl() {
        Some(data::<Item>(other).path.to_lowercase())
    } else {
        None
    };
    let mine = data::<Item>(this).path.to_lowercase();
    let order = match &theirs {
        Some(t) => mine.cmp(t) as i32,
        None => 1,
    };
    let _ = write(out, &order.to_le_bytes());
    if order == 0 { hr::S_OK } else { hr::S_FALSE }
}
#[unsafe(no_mangle)]
pub extern "system" fn SHCreateItemFromParsingName(
    path: usize,
    _bind: usize,
    iid: usize,
    out: usize,
) -> u32 {
    let (path, iid, out) = (
        door_windows_abi::widen(path),
        door_windows_abi::widen(iid),
        door_windows_abi::widen(out),
    );
    if out == 0 || iid == 0 {
        return E_POINTER;
    }
    let _ = write_pointer(out, 0);
    let Ok(text) = read_text(path) else {
        return hr::E_INVALIDARG;
    };
    let full = absolute(&text, None);
    if attributes(&full).is_none() {
        return FILE_NOT_FOUND;
    }
    new_item(full, iid, out)
}

// ---- the desktop's IShellFolder -----------------------------------------------------------------
/// A folder: the desktop (`None`, absolute lists) or a file-system folder
/// whose lists are relative to it.
struct ShellFolder {
    base: Option<String>,
}
impl Kind for ShellFolder {
    const IID: Guid = IID_ISHELLFOLDER;
    fn vtbl() -> *const () {
        &FOLDER_VTBL as *const _ as *const ()
    }
}
static FOLDER_VTBL: Vtbl<13> = Vtbl([
    query::<ShellFolder> as *const (),
    add_ref::<ShellFolder> as *const (),
    release::<ShellFolder> as *const (),
    folder_parse as *const (),
    folder_enum as *const (),
    folder_bind as *const (),
    folder_bind as *const (),
    folder_compare as *const (),
    folder_view as *const (),
    folder_attributes as *const (),
    folder_ui as *const (),
    folder_name as *const (),
    folder_set_name as *const (),
]);
/// The path a list names in folder `this`.
fn resolve(this: usize, list: u64) -> Option<String> {
    let base = &data::<ShellFolder>(this).base;
    match base {
        None => list_path(list),
        Some(b) => {
            let bytes = read_list(list)?;
            let items = pidl::items(&bytes)?;
            let mut path = String::from(b.trim_end_matches('\\'));
            for (_, name) in items {
                path.push('\\');
                path.push_str(&name);
            }
            Some(path)
        }
    }
}
extern "system" fn folder_parse(
    this: usize,
    _window: usize,
    _bind: usize,
    name: usize,
    eaten: usize,
    out: usize,
    attributes_io: usize,
) -> u32 {
    let (name, eaten, out) = (
        door_windows_abi::widen(name),
        door_windows_abi::widen(eaten),
        door_windows_abi::widen(out),
    );
    let attributes_io = door_windows_abi::widen(attributes_io);
    if out == 0 || name == 0 {
        return E_POINTER;
    }
    let _ = write_pointer(out, 0);
    let Ok(text) = read_text(name) else {
        return hr::E_INVALIDARG;
    };
    let base = data::<ShellFolder>(this).base.clone();
    let full = absolute(&text, base.as_deref());
    let Some(a) = shell_attributes(&full) else {
        return FILE_NOT_FOUND;
    };
    let Some(mut list) = pidl::from_path(&full, a & SFGAO_FOLDER != 0) else {
        return FILE_NOT_FOUND;
    };
    // Relative to a folder: drop the folder's own items.
    if let Some(b) = &base {
        let skip = pidl::from_path(b, true)
            .and_then(|l| pidl::size(&l))
            .unwrap_or(2)
            - 2;
        if !full.to_lowercase().starts_with(&b.to_lowercase()) || skip > list.len() - 2 {
            return hr::E_INVALIDARG;
        }
        list.drain(..skip);
    }
    let Ok(p) = task_copy(&list) else {
        return hr::E_OUTOFMEMORY;
    };
    if eaten != 0 {
        let _ = write(eaten, &(text.encode_utf16().count() as u32).to_le_bytes());
    }
    if attributes_io != 0
        && let Ok(wanted) = read_u32(attributes_io)
    {
        let _ = write(attributes_io, &(a & wanted).to_le_bytes());
    }
    let _ = write_pointer(out, p);
    hr::S_OK
}
extern "system" fn folder_enum(_this: usize, _window: usize, _flags: u32, out: usize) -> u32 {
    let _ = write_pointer(door_windows_abi::widen(out), 0);
    E_NOTIMPL
}
extern "system" fn folder_bind(
    this: usize,
    list: usize,
    _bind: usize,
    iid: usize,
    out: usize,
) -> u32 {
    let (iid, out) = (door_windows_abi::widen(iid), door_windows_abi::widen(out));
    if out == 0 || iid == 0 {
        return E_POINTER;
    }
    let _ = write_pointer(out, 0);
    match resolve(this, door_windows_abi::widen(list)) {
        Some(p) if is_directory(&p) => hand_out(
            new_object(ShellFolder { base: Some(p) }),
            iid,
            out,
            release::<ShellFolder>,
            query::<ShellFolder>,
        ),
        Some(_) => E_NOINTERFACE,
        None => hr::E_INVALIDARG,
    }
}
extern "system" fn folder_compare(this: usize, _param: isize, a: usize, b: usize) -> u32 {
    let (Some(x), Some(y)) = (
        resolve(this, door_windows_abi::widen(a)),
        resolve(this, door_windows_abi::widen(b)),
    ) else {
        return hr::E_INVALIDARG;
    };
    // MAKE_HRESULT(0, 0, (USHORT)(short)order).
    match x.to_lowercase().cmp(&y.to_lowercase()) {
        core::cmp::Ordering::Less => 0xFFFF,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}
extern "system" fn folder_view(_this: usize, _owner: usize, _iid: usize, out: usize) -> u32 {
    let _ = write_pointer(door_windows_abi::widen(out), 0);
    E_NOTIMPL
}
extern "system" fn folder_attributes(this: usize, count: u32, lists: usize, io: usize) -> u32 {
    let (lists, io) = (door_windows_abi::widen(lists), door_windows_abi::widen(io));
    if io == 0 || (count > 0 && lists == 0) {
        return E_POINTER;
    }
    let Ok(mut a) = read_u32(io) else {
        return E_POINTER;
    };
    for i in 0..u64::from(count.min(4096)) {
        let Ok(list) = read_pointer(lists + i * POINTER as u64) else {
            return E_POINTER;
        };
        a &= resolve(this, list)
            .and_then(|p| shell_attributes(&p))
            .unwrap_or(0);
    }
    let _ = write(io, &a.to_le_bytes());
    hr::S_OK
}
extern "system" fn folder_ui(
    _this: usize,
    _owner: usize,
    _count: u32,
    _lists: usize,
    _iid: usize,
    _reserved: usize,
    out: usize,
) -> u32 {
    let _ = write_pointer(door_windows_abi::widen(out), 0);
    E_NOTIMPL
}
/// GetDisplayNameOf into a STRRET (uType, then the union at the pointer
/// boundary): SHGDN_FORPARSING without SHGDN_INFOLDER is the path, any
/// other form the last part.
extern "system" fn folder_name(this: usize, list: usize, flags: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    if out == 0 {
        return E_POINTER;
    }
    let Some(path) = resolve(this, door_windows_abi::widen(list)) else {
        return hr::E_INVALIDARG;
    };
    let text = if flags & 0x8000 != 0 && flags & 1 == 0 {
        path
    } else {
        display_name(&path, 0)
    };
    match wide_task(&text) {
        Ok(p) => {
            let _ = write(out, &0u32.to_le_bytes());
            let _ = write_pointer(out + POINTER as u64, p);
            hr::S_OK
        }
        Err(e) => e,
    }
}
extern "system" fn folder_set_name(
    _this: usize,
    _window: usize,
    _list: usize,
    _name: usize,
    _flags: u32,
    out: usize,
) -> u32 {
    let _ = write_pointer(door_windows_abi::widen(out), 0);
    E_NOTIMPL
}
#[unsafe(no_mangle)]
pub extern "system" fn SHGetDesktopFolder(out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    if out == 0 {
        return E_POINTER;
    }
    let _ = write_pointer(out, new_object(ShellFolder { base: None }));
    hr::S_OK
}

// ---- windows' property stores -------------------------------------------------------------------
/// A stored PROPVARIANT: its type, an 8-byte scalar, or owned text or a
/// CLSID.
#[derive(Clone)]
enum Value {
    Scalar(u16, [u8; 8]),
    Text(Vec<u16>),
    Clsid([u8; 16]),
}
type Key = [u8; 20];
/// One window's properties.
type Properties = Vec<(Key, Value)>;
static STORES: Lock<Vec<(u64, Properties)>> = Lock::new(Vec::new());
const VT_LPWSTR: u16 = 31;
const VT_CLSID: u16 = 72;
/// VT_EMPTY, VT_NULL, the integer, real, BOOL and FILETIME types.
const SCALARS: [u16; 17] = [0, 1, 2, 3, 4, 5, 6, 7, 10, 11, 16, 17, 18, 19, 20, 21, 64];
struct Store {
    window: u64,
}
impl Kind for Store {
    const IID: Guid = IID_IPROPERTYSTORE;
    fn vtbl() -> *const () {
        &STORE_VTBL as *const _ as *const ()
    }
}
static STORE_VTBL: Vtbl<8> = Vtbl([
    query::<Store> as *const (),
    add_ref::<Store> as *const (),
    release::<Store> as *const (),
    store_count as *const (),
    store_at as *const (),
    store_get as *const (),
    store_set as *const (),
    store_commit as *const (),
]);
fn with_store<R>(this: usize, f: impl FnOnce(&mut Properties) -> R) -> R {
    let window = data::<Store>(this).window;
    STORES.with(|s| {
        let i = match s.iter().position(|(w, _)| *w == window) {
            Some(i) => i,
            None => {
                s.push((window, Vec::new()));
                s.len() - 1
            }
        };
        f(&mut s[i].1)
    })
}
extern "system" fn store_count(this: usize, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    if out == 0 {
        return E_POINTER;
    }
    let n = with_store(this, |v| v.len() as u32);
    let _ = write(out, &n.to_le_bytes());
    hr::S_OK
}
extern "system" fn store_at(this: usize, index: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    if out == 0 {
        return E_POINTER;
    }
    match with_store(this, |v| v.get(index as usize).map(|(k, _)| *k)) {
        Some(k) => {
            let _ = write(out, &k);
            hr::S_OK
        }
        None => hr::E_INVALIDARG,
    }
}
fn read_key(at: u64) -> Option<Key> {
    let mut k = [0u8; 20];
    read(at, &mut k).ok().map(|()| k)
}
/// PROPVARIANT: vt at 0, the value at 8; 16 bytes on x86, 24 on x64.
const PROPVARIANT: usize = if POINTER == 8 { 24 } else { 16 };
extern "system" fn store_get(this: usize, key: usize, out: usize) -> u32 {
    let (key, out) = (door_windows_abi::widen(key), door_windows_abi::widen(out));
    if out == 0 {
        return E_POINTER;
    }
    let Some(k) = read_key(key) else {
        return E_POINTER;
    };
    let mut v = [0u8; PROPVARIANT];
    let value = with_store(this, |s| {
        s.iter().find(|(x, _)| *x == k).map(|(_, v)| v.clone())
    });
    match value {
        None => {}
        Some(Value::Scalar(vt, bytes)) => {
            v[..2].copy_from_slice(&vt.to_le_bytes());
            v[8..16].copy_from_slice(&bytes);
        }
        Some(Value::Text(units)) => {
            let bytes: Vec<u8> = units
                .iter()
                .chain([&0])
                .flat_map(|u| u.to_le_bytes())
                .collect();
            let Ok(p) = task_copy(&bytes) else {
                return hr::E_OUTOFMEMORY;
            };
            v[..2].copy_from_slice(&VT_LPWSTR.to_le_bytes());
            v[8..8 + POINTER].copy_from_slice(&p.to_le_bytes()[..POINTER]);
        }
        Some(Value::Clsid(g)) => {
            let Ok(p) = task_copy(&g) else {
                return hr::E_OUTOFMEMORY;
            };
            v[..2].copy_from_slice(&VT_CLSID.to_le_bytes());
            v[8..8 + POINTER].copy_from_slice(&p.to_le_bytes()[..POINTER]);
        }
    }
    match write(out, &v) {
        Ok(()) => hr::S_OK,
        Err(_) => E_POINTER,
    }
}
extern "system" fn store_set(this: usize, key: usize, value: usize) -> u32 {
    let (key, value) = (door_windows_abi::widen(key), door_windows_abi::widen(value));
    let Some(k) = read_key(key) else {
        return E_POINTER;
    };
    let mut v = [0u8; 16];
    if read(value, &mut v).is_err() {
        return E_POINTER;
    }
    let vt = u16::from_le_bytes([v[0], v[1]]);
    let pointer = || read_pointer(value + 8).unwrap_or(0);
    let stored = match vt {
        0 => None,
        VT_LPWSTR => match read_wide(pointer(), 32767) {
            Ok(units) => Some(Value::Text(units)),
            Err(_) => return hr::E_INVALIDARG,
        },
        VT_CLSID => {
            let mut g = [0u8; 16];
            if read(pointer(), &mut g).is_err() {
                return hr::E_INVALIDARG;
            }
            Some(Value::Clsid(g))
        }
        _ if SCALARS.contains(&vt) => {
            Some(Value::Scalar(vt, v[8..16].try_into().unwrap_or([0; 8])))
        }
        _ => return hr::E_INVALIDARG,
    };
    with_store(this, |s| {
        let at = s.iter().position(|(x, _)| *x == k);
        match (at, stored) {
            (Some(i), Some(value)) => s[i].1 = value,
            (None, Some(value)) => s.push((k, value)),
            (Some(i), None) => {
                s.remove(i);
            }
            (None, None) => {}
        }
    });
    hr::S_OK
}
extern "system" fn store_commit(_this: usize) -> u32 {
    hr::S_OK
}
#[unsafe(no_mangle)]
pub extern "system" fn SHGetPropertyStoreForWindow(window: usize, iid: usize, out: usize) -> u32 {
    let (iid, out) = (door_windows_abi::widen(iid), door_windows_abi::widen(out));
    if out == 0 || iid == 0 {
        return E_POINTER;
    }
    let _ = write_pointer(out, 0);
    if unsafe { IsWindow(window) } == 0 {
        return hr::E_INVALIDARG;
    }
    hand_out(
        new_object(Store {
            window: door_windows_abi::widen(window),
        }),
        iid,
        out,
        release::<Store>,
        query::<Store>,
    )
}
