//! Summary information, sessions (MsiOpenPackage and the session a custom
//! action is given), product queries over the Installer's records, and
//! installation: MsiInstallProduct, MsiConfigureProduct(Ex),
//! MsiReinstallProduct with the internal UI level and the external UI
//! handler set for this process.
use super::*;
use door_msi::engine::level;
use door_msi::execute::Target;
use door_msi::install::Machine as Layout;
use door_msi::install::Root;
use target::Machine;
use ui::Interface;

// ---- settings ---------------------------------------------------------------------

pub(crate) struct Settings {
    pub ui_level: u32,
    pub passive: bool,
    pub external: Option<(u64, u32, u64)>,
    pub log: Option<String>,
}
pub(crate) static SETTINGS: Lock<Settings> = Lock::new(Settings {
    ui_level: level::BASIC,
    passive: false,
    external: None,
    log: None,
});

/// MsiSetInternalUI: the previous level (INSTALLUILEVEL_NOCHANGE keeps it).
#[unsafe(no_mangle)]
pub extern "system" fn MsiSetInternalUI(ui_level: u32, window: usize) -> u32 {
    let window = door_windows_abi::widen(window);
    {
        let _ = window;
        SETTINGS.with(|s| {
            let old = s.ui_level;
            let base = ui_level & 0xf;
            if base != 0 {
                s.ui_level = if base == 1 {
                    level::BASIC
                } else {
                    base.clamp(level::NONE, level::FULL)
                };
                // INSTALLUILEVEL_PROGRESSONLY / HIDECANCEL: no Cancel.
                s.passive = ui_level & 0x60 != 0;
            }
            old
        })
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiSetExternalUIW(handler: usize, filter: u32, context: usize) -> usize {
    let handler = door_windows_abi::widen(handler);
    let context = door_windows_abi::widen(context);
    door_windows_abi::narrow((move || -> u64 {
        SETTINGS.with(|s| {
            let old = s.external.map_or(0, |e| e.0);
            s.external = (handler != 0).then_some((handler, filter, context));
            old
        })
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiEnableLogW(mode: u32, path: usize, attributes: u32) -> u32 {
    let path = door_windows_abi::widen(path);
    {
        let _ = attributes;
        let path = match wide(path) {
            Ok(p) => p,
            Err(e) => return e,
        };
        SETTINGS.with(|s| {
            s.log = if mode == 0 {
                None
            } else {
                path.map(|p| absolute(&p))
            }
        });
        error::SUCCESS
    }
}

// ---- summary information --------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn MsiGetSummaryInformationW(
    database: u32,
    path: usize,
    updates: u32,
    out: usize,
) -> u32 {
    let path = door_windows_abi::widen(path);
    let out = door_windows_abi::widen(out);
    {
        let _ = updates;
        let bytes = if database != 0 {
            match database_bytes(database) {
                Some(b) => b,
                None => return error::INVALID_HANDLE,
            }
        } else {
            let Ok(Some(p)) = wide(path) else {
                return error::INVALID_PARAMETER;
            };
            match load(&absolute(&p)) {
                Ok(b) => Arc::new(b),
                Err(e) => return e,
            }
        };
        let Ok(db) = door_msi::Database::open(&bytes) else {
            return error::OPEN_FAILED;
        };
        let properties = db.summary().map(|s| s.properties).unwrap_or_default();
        let h = insert(Object::Summary(properties));
        if write_u32(out, h).is_err() {
            close(h);
            return error::INVALID_PARAMETER;
        }
        error::SUCCESS
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiSummaryInfoGetPropertyCount(summary: u32, out: usize) -> u32 {
    let out = door_windows_abi::widen(out);
    {
        let Some(n) = with_object(summary, |o| match o {
            Object::Summary(p) => Some(p.len() as u32),
            _ => None,
        })
        .flatten() else {
            return error::INVALID_HANDLE;
        };
        write_u32(out, n).map_or(error::INVALID_PARAMETER, |()| error::SUCCESS)
    }
}
/// MsiSummaryInfoGetPropertyW: VT_I2 2, VT_I4 3, VT_LPSTR 30, VT_FILETIME
/// 64, VT_EMPTY 0.
#[unsafe(no_mangle)]
pub extern "system" fn MsiSummaryInfoGetPropertyW(
    summary: u32,
    id: u32,
    kind: usize,
    integer: usize,
    filetime: usize,
    buffer: usize,
    length: usize,
) -> u32 {
    let kind = door_windows_abi::widen(kind);
    let integer = door_windows_abi::widen(integer);
    let filetime = door_windows_abi::widen(filetime);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    {
        let Some(value) = with_object(summary, |o| match o {
            Object::Summary(p) => Some(p.iter().find(|x| x.0 == id).map(|x| x.1.clone())),
            _ => None,
        })
        .flatten() else {
            return error::INVALID_HANDLE;
        };
        let set_kind = |k: u32| {
            if kind != 0 {
                let _ = write_u32(kind, k);
            }
        };
        match value {
            None => {
                set_kind(0);
                error::SUCCESS
            }
            Some(Property::Int(v)) => {
                set_kind(if id == 1 || id == 19 { 2 } else { 3 });
                if integer != 0 {
                    let _ = write_u32(integer, v as u32);
                }
                error::SUCCESS
            }
            Some(Property::Time(t)) => {
                set_kind(64);
                if filetime != 0 {
                    let _ = write(filetime, &t.to_le_bytes());
                }
                error::SUCCESS
            }
            Some(Property::Text(t)) => {
                set_kind(30);
                out_wide(&t, buffer, length)
            }
        }
    }
}

// ---- sessions --------------------------------------------------------------------

fn layout() -> Layout {
    let env = |name: &str| -> String {
        let n = utf16z(name);
        let mut buffer = [0u16; 512];
        let len = unsafe {
            DoorGetEnvironmentVariableW(
                n.as_ptr() as u64,
                buffer.as_mut_ptr() as u64,
                buffer.len() as u32,
            )
        } as usize;
        String::from_utf16_lossy(&buffer[..len.min(buffer.len())])
    };
    let mut profile = env("USERPROFILE");
    if profile.is_empty() {
        profile = String::from("C:\\Users\\Public");
    }
    if !profile.ends_with('\\') {
        profile.push('\\');
    }
    let user = env("USERNAME");
    let computer = env("COMPUTERNAME");
    Layout::doors(&profile, &user, &computer, true)
}

fn open_package(path: Option<String>, out: u64) -> u32 {
    let Some(path) = path else {
        return error::INVALID_PARAMETER;
    };
    // "#<handle>": an open database.
    let (bytes, path) = if let Some(h) = path.strip_prefix('#') {
        let Ok(h) = h.parse::<u32>() else {
            return error::INVALID_HANDLE;
        };
        let Some(found) = with_object(h, |o| match o {
            Object::Database(p) => Some((p.bytes.clone(), p.path.clone())),
            _ => None,
        })
        .flatten() else {
            return error::INVALID_HANDLE;
        };
        found
    } else {
        let p = absolute(&path);
        match load(&p) {
            Ok(b) => (Arc::new(b), p),
            Err(_) => return code::PACKAGE_OPEN_FAILED,
        }
    };
    let Ok(db) = door_msi::Database::open(&bytes) else {
        return code::BAD_PACKAGE;
    };
    let mut target = Machine::new();
    let state = Session::new(
        &db,
        &mut target,
        &path,
        Request::Install,
        layout(),
        &[],
        level::NONE,
    )
    .state;
    let h = insert(Object::Package(Box::new((Package { bytes, path }, state))));
    if write_u32(out, h).is_err() {
        close(h);
        return error::INVALID_PARAMETER;
    }
    error::SUCCESS
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiOpenPackageW(path: usize, out: usize) -> u32 {
    let path = door_windows_abi::widen(path);
    let out = door_windows_abi::widen(out);
    {
        match wide(path) {
            Ok(p) => open_package(p, out),
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiOpenPackageExW(path: usize, options: u32, out: usize) -> u32 {
    let path = door_windows_abi::widen(path);
    let out = door_windows_abi::widen(out);
    {
        let _ = options;
        MsiOpenPackageW(
            door_windows_abi::narrow(path),
            door_windows_abi::narrow(out),
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiGetPropertyW(
    session: u32,
    name: usize,
    buffer: usize,
    length: usize,
) -> u32 {
    let name = door_windows_abi::widen(name);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    {
        let Ok(Some(name)) = wide(name) else {
            return error::INVALID_PARAMETER;
        };
        match with_state(session, |s| String::from(s.property(&name))) {
            Some(v) => out_wide(&v, buffer, length),
            None => error::INVALID_HANDLE,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiGetPropertyA(
    session: u32,
    name: usize,
    buffer: usize,
    length: usize,
) -> u32 {
    let name = door_windows_abi::widen(name);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    {
        let Ok(Some(name)) = ansi(name) else {
            return error::INVALID_PARAMETER;
        };
        match with_state(session, |s| String::from(s.property(&name))) {
            Some(v) => out_ansi(&v, buffer, length),
            None => error::INVALID_HANDLE,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiSetPropertyW(session: u32, name: usize, value: usize) -> u32 {
    let name = door_windows_abi::widen(name);
    let value = door_windows_abi::widen(value);
    {
        let Ok(Some(name)) = wide(name) else {
            return error::INVALID_PARAMETER;
        };
        let value = wide(value).ok().flatten().unwrap_or_default();
        if name.is_empty() {
            return error::INVALID_PARAMETER;
        }
        with_state(session, |s| s.set_property(&name, &value))
            .map_or(error::INVALID_HANDLE, |()| error::SUCCESS)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiSetPropertyA(session: u32, name: usize, value: usize) -> u32 {
    let name = door_windows_abi::widen(name);
    let value = door_windows_abi::widen(value);
    {
        let Ok(Some(name)) = ansi(name) else {
            return error::INVALID_PARAMETER;
        };
        let value = ansi(value).ok().flatten().unwrap_or_default();
        with_state(session, |s| s.set_property(&name, &value))
            .map_or(error::INVALID_HANDLE, |()| error::SUCCESS)
    }
}
/// MSICONDITION_FALSE 0, TRUE 1, NONE 2 (empty), ERROR 3.
#[unsafe(no_mangle)]
pub extern "system" fn MsiEvaluateConditionW(session: u32, condition: usize) -> u32 {
    let condition = door_windows_abi::widen(condition);
    {
        let Ok(text) = wide(condition) else {
            return 3;
        };
        let text = text.unwrap_or_default();
        if text.trim().is_empty() {
            return 2;
        }
        with_state(session, |s| u32::from(s.evaluate(&text))).unwrap_or(3)
    }
}
/// MsiGetMode: MSIRUNMODE_MAINTENANCE 2, REBOOTATEND 6, SCHEDULED 16,
/// ROLLBACK 17, COMMIT 18.
#[unsafe(no_mangle)]
pub extern "system" fn MsiGetMode(session: u32, mode: u32) -> i32 {
    with_state(session, |s| match mode {
        2 => !s.property("Installed").is_empty(),
        6 => s.reboot,
        16 => s.script && !s.property("CustomActionData").is_empty(),
        _ => false,
    })
    .map_or(0, i32::from)
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiGetLanguage(session: u32) -> u16 {
    with_state(session, |s| {
        s.property("ProductLanguage").parse().unwrap_or(0)
    })
    .unwrap_or(0)
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiGetTargetPathW(
    session: u32,
    folder: usize,
    buffer: usize,
    length: usize,
) -> u32 {
    let folder = door_windows_abi::widen(folder);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    {
        let Ok(Some(folder)) = wide(folder) else {
            return error::INVALID_PARAMETER;
        };
        let path = with_state(session, |s| {
            let p = s.property(&folder);
            if !p.is_empty() {
                return Some(String::from(p));
            }
            s.plan.directory(&folder).map(String::from)
        });
        match path {
            None => error::INVALID_HANDLE,
            Some(None) => 267, // ERROR_DIRECTORY
            Some(Some(p)) => out_wide(&p, buffer, length),
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiSetTargetPathW(session: u32, folder: usize, path: usize) -> u32 {
    let folder = door_windows_abi::widen(folder);
    let path = door_windows_abi::widen(path);
    {
        let (Ok(Some(folder)), Ok(Some(mut path))) = (wide(folder), wide(path)) else {
            return error::INVALID_PARAMETER;
        };
        if !path.ends_with('\\') {
            path.push('\\');
        }
        with_state(session, |s| s.set_property(&folder, &path))
            .map_or(error::INVALID_HANDLE, |()| error::SUCCESS)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiGetSourcePathW(
    session: u32,
    folder: usize,
    buffer: usize,
    length: usize,
) -> u32 {
    let folder = door_windows_abi::widen(folder);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    {
        let _ = folder;
        match with_state(session, |s| String::from(s.property("SourceDir"))) {
            Some(p) => out_wide(&p, buffer, length),
            None => error::INVALID_HANDLE,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiGetActiveDatabase(session: u32) -> u32 {
    let package = with_object(session, |o| match o {
        Object::Package(b) => Some((b.0.bytes.clone(), b.0.path.clone())),
        Object::Action(p) => Some((Arc::new(Vec::new()), unsafe { (**p).package.clone() })),
        _ => None,
    })
    .flatten();
    let Some((bytes, path)) = package else {
        return 0;
    };
    let bytes = if bytes.is_empty() {
        match load(&path) {
            Ok(b) => Arc::new(b),
            Err(_) => return 0,
        }
    } else {
        bytes
    };
    insert(Object::Database(Package { bytes, path }))
}
/// MsiFormatRecordW: `[n]` fields of the record, then (with a session)
/// properties and the rest of MsiFormatRecord's syntax.
#[unsafe(no_mangle)]
pub extern "system" fn MsiFormatRecordW(
    session: u32,
    record: u32,
    buffer: usize,
    length: usize,
) -> u32 {
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    {
        let Some(fields) = record_fields(record) else {
            return error::INVALID_HANDLE;
        };
        let template = fields.first().map(Field::text).unwrap_or_default();
        let mut text = if template.is_empty() {
            // Without a template: "1: value 2: value ...".
            let mut s = String::new();
            for (i, f) in fields.iter().enumerate().skip(1) {
                if !s.is_empty() {
                    s.push(' ');
                }
                s.push_str(&alloc::format!("{i}: {}", f.text()));
            }
            s
        } else {
            let mut t = template.clone();
            for (i, f) in fields.iter().enumerate().skip(1).rev() {
                t = t.replace(&alloc::format!("[{i}]"), &f.text());
            }
            t
        };
        if session != 0
            && let Some(formatted) = with_state(session, |s| s.format(&text))
        {
            text = formatted;
        }
        out_wide(&text, buffer, length)
    }
}
/// MsiProcessMessage from a custom action: information goes to the log;
/// errors, warnings and user messages to a message box when the UI shows
/// them.
#[unsafe(no_mangle)]
pub extern "system" fn MsiProcessMessage(session: u32, kind: u32, record: u32) -> i32 {
    let Some(fields) = record_fields(record) else {
        return -1;
    };
    let template = fields.first().map(Field::text).unwrap_or_default();
    let mut text = template.clone();
    for (i, f) in fields.iter().enumerate().skip(1).rev() {
        text = text.replace(&alloc::format!("[{i}]"), &f.text());
    }
    if template.is_empty() {
        text = fields
            .iter()
            .skip(1)
            .map(Field::text)
            .collect::<Vec<_>>()
            .join(" ");
    }
    let Some((product, logged)) = with_state(session, |s| {
        let t = s.format(&text);
        s.log.push(t.clone());
        (String::from(s.property("ProductName")), t)
    }) else {
        return -1;
    };
    super::target::forward(&logged);
    let class = kind & 0xff00_0000;
    if matches!(class, 0x0100_0000 | 0x0200_0000 | 0x0300_0000)
        && SETTINGS.with(|s| s.ui_level) > level::NONE
    {
        let w = utf16z(&logged);
        let c = utf16z(&product);
        #[link(name = "user32", kind = "dylib")]
        unsafe extern "system" {
            fn MessageBoxW(hwnd: usize, text: usize, caption: usize, kind: u32) -> i32;
        }
        return unsafe {
            MessageBoxW(
                0,
                door_windows_abi::narrow(w.as_ptr() as u64),
                door_windows_abi::narrow(c.as_ptr() as u64),
                kind & 0xffff,
            )
        };
    }
    0
}
/// MsiDoActionW: one action in the session (costing, a custom action, a
/// dialog), without UI.
#[unsafe(no_mangle)]
pub extern "system" fn MsiDoActionW(session: u32, action: usize) -> u32 {
    let action = door_windows_abi::widen(action);
    {
        let Ok(Some(action)) = wide(action) else {
            return error::INVALID_PARAMETER;
        };
        let package = with_object(session, |o| match o {
            Object::Package(b) => {
                Some((b.0.bytes.clone(), Box::as_mut(b) as *mut (Package, State)))
            }
            _ => None,
        })
        .flatten();
        let Some((bytes, pointer)) = package else {
            return code::FUNCTION_NOT_CALLED;
        };
        let Ok(db) = door_msi::Database::open(&bytes) else {
            return code::BAD_PACKAGE;
        };
        // SAFETY: the box stays until MsiCloseHandle of this session, which
        // this call does not do.
        let state = core::mem::take(unsafe { &mut (*pointer).1 });
        let mut target = Machine::new();
        let mut s = Session {
            db: &db,
            target: &mut target,
            state,
        };
        let mut ui = Interface::new(level::NONE, true, "");
        let result = s.action(&mut ui, &action);
        unsafe { (*pointer).1 = core::mem::take(&mut s.state) };
        match result {
            Ok(()) => error::SUCCESS,
            Err(code::INSTALL_USEREXIT) => code::INSTALL_USEREXIT,
            Err(code::NO_MORE_ITEMS) => error::NO_MORE_ITEMS,
            Err(_) => code::INSTALL_FAILURE,
        }
    }
}

// ---- product queries -------------------------------------------------------------

/// INSTALLSTATE_DEFAULT (5) for an installed product, INSTALLSTATE_UNKNOWN
/// (-1) otherwise.
#[unsafe(no_mangle)]
pub extern "system" fn MsiQueryProductStateW(product: usize) -> i32 {
    let product = door_windows_abi::widen(product);
    {
        let Ok(Some(p)) = wide(product) else {
            return -2; // INSTALLSTATE_INVALIDARG
        };
        let mut t = Machine::new();
        if records::product(&mut t, &p).is_some() {
            5
        } else {
            -1
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiQueryProductStateA(product: usize) -> i32 {
    let product = door_windows_abi::widen(product);
    {
        let Ok(Some(p)) = ansi(product) else {
            return -2;
        };
        let mut t = Machine::new();
        if records::product(&mut t, &p).is_some() {
            5
        } else {
            -1
        }
    }
}
fn product_info(product: &str, property: &str) -> Result<String, u32> {
    let mut t = Machine::new();
    let record = records::product(&mut t, product).ok_or(error::UNKNOWN_PRODUCT)?;
    if let Some(value) = crate::product_info_value(property) {
        let (root, key) =
            records::product_key(record.all_users, product).ok_or(error::BAD_CONFIGURATION)?;
        return Ok(match t.value(root, &key, value) {
            Some(door_msi::install::RegValue::String(s)) => s,
            Some(door_msi::install::RegValue::Dword(d)) => alloc::format!("{d}"),
            _ => String::new(),
        });
    }
    // The Add/Remove Programs values (InstallLocation, HelpLink, ...).
    let root = if record.all_users {
        door_msi::install::Root::LocalMachine
    } else {
        door_msi::install::Root::CurrentUser
    };
    let key = alloc::format!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{product}");
    let name = match property {
        "InstallLocation" | "HelpLink" | "HelpTelephone" | "URLInfoAbout" | "URLUpdateInfo"
        | "Comments" | "Contact" | "DisplayIcon" => property,
        _ => return Err(error::UNKNOWN_PROPERTY),
    };
    Ok(match t.value(root, &key, name) {
        Some(door_msi::install::RegValue::String(s))
        | Some(door_msi::install::RegValue::Expand(s)) => s,
        _ => String::new(),
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiGetProductInfoW(
    product: usize,
    property: usize,
    buffer: usize,
    length: usize,
) -> u32 {
    let product = door_windows_abi::widen(product);
    let property = door_windows_abi::widen(property);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    {
        let (Ok(Some(p)), Ok(Some(name))) = (wide(product), wide(property)) else {
            return error::INVALID_PARAMETER;
        };
        match product_info(&p, &name) {
            Ok(v) => out_wide(&v, buffer, length),
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiGetProductInfoA(
    product: usize,
    property: usize,
    buffer: usize,
    length: usize,
) -> u32 {
    let product = door_windows_abi::widen(product);
    let property = door_windows_abi::widen(property);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    {
        let (Ok(Some(p)), Ok(Some(name))) = (ansi(product), ansi(property)) else {
            return error::INVALID_PARAMETER;
        };
        match product_info(&p, &name) {
            Ok(v) => out_ansi(&v, buffer, length),
            Err(e) => e,
        }
    }
}
fn out_guid(code: &str, buffer: u64) -> u32 {
    let mut units: Vec<u8> = code.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    units.extend_from_slice(&[0, 0]);
    write(buffer, &units).map_or(error::INVALID_PARAMETER, |()| error::SUCCESS)
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiEnumProductsW(index: u32, buffer: usize) -> u32 {
    let buffer = door_windows_abi::widen(buffer);
    {
        let mut t = Machine::new();
        match records::products(&mut t).get(index as usize) {
            Some(code) => out_guid(code, buffer),
            None => error::NO_MORE_ITEMS,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiEnumRelatedProductsW(
    upgrade: usize,
    reserved: u32,
    index: u32,
    buffer: usize,
) -> u32 {
    let upgrade = door_windows_abi::widen(upgrade);
    let buffer = door_windows_abi::widen(buffer);
    {
        let _ = reserved;
        let Ok(Some(upgrade)) = wide(upgrade) else {
            return error::INVALID_PARAMETER;
        };
        let mut t = Machine::new();
        match records::related(&mut t, &upgrade).get(index as usize) {
            Some(code) => out_guid(code, buffer),
            None => error::NO_MORE_ITEMS,
        }
    }
}
/// INSTALLSTATE_LOCAL (3) with the key path, or INSTALLSTATE_UNKNOWN.
#[unsafe(no_mangle)]
pub extern "system" fn MsiGetComponentPathW(
    product: usize,
    component: usize,
    buffer: usize,
    length: usize,
) -> i32 {
    let product = door_windows_abi::widen(product);
    let component = door_windows_abi::widen(component);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    {
        let (Ok(Some(p)), Ok(Some(c))) = (wide(product), wide(component)) else {
            return -2;
        };
        let mut t = Machine::new();
        let Some(packed) = door_msi::execute::packed_guid(&p) else {
            return -2;
        };
        match records::clients(&mut t, &c)
            .into_iter()
            .find(|x| x.0 == packed)
        {
            Some((_, path)) => {
                if buffer != 0 || length != 0 {
                    let _ = out_wide(&path, buffer, length);
                }
                if key_path_present(&mut t, &path) {
                    3
                } else {
                    2
                }
            }
            None => -1,
        }
    }
}
/// Whether a component's key path is there: a file or folder, or for a
/// registry key path (`nn:\key\name`, 00-03 in the 32-bit view and 20-23
/// in the 64-bit one) the value.
fn key_path_present(t: &mut Machine, path: &str) -> bool {
    let b = path.as_bytes();
    if b.len() > 4 && b[0].is_ascii_digit() && b[1].is_ascii_digit() && &b[2..4] == b":\\" {
        let code = (b[0] - b'0') * 10 + (b[1] - b'0');
        let root = match code % 20 {
            0 => Root::ClassesRoot,
            1 => Root::CurrentUser,
            3 => Root::Users,
            _ => Root::LocalMachine,
        }
        .in_view(code < 20);
        let rest = &path[4..];
        let (key, name) = rest.rsplit_once('\\').unwrap_or((rest, ""));
        return t.value(root, key, name).is_some();
    }
    t.exists(path.trim_end_matches('\\'))
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiLocateComponentW(component: usize, buffer: usize, length: usize) -> i32 {
    let component = door_windows_abi::widen(component);
    let buffer = door_windows_abi::widen(buffer);
    let length = door_windows_abi::widen(length);
    {
        let Ok(Some(c)) = wide(component) else {
            return -2;
        };
        let mut t = Machine::new();
        match records::clients(&mut t, &c)
            .into_iter()
            .find(|x| x.0 != records::PERMANENT_CLIENT)
        {
            Some((_, path)) => {
                let _ = out_wide(&path, buffer, length);
                3
            }
            None => -1,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiGetProductCodeW(component: usize, buffer: usize) -> u32 {
    let component = door_windows_abi::widen(component);
    let buffer = door_windows_abi::widen(buffer);
    {
        let Ok(Some(c)) = wide(component) else {
            return error::INVALID_PARAMETER;
        };
        let mut t = Machine::new();
        match records::clients(&mut t, &c)
            .into_iter()
            .filter_map(|x| door_msi::execute::unpacked_guid(&x.0))
            .next()
        {
            Some(code) => out_guid(&code, buffer),
            None => error::UNKNOWN_COMPONENT,
        }
    }
}
/// MsiGetFileVersionW: a PE file's version as `a.b.c.d`.
#[unsafe(no_mangle)]
pub extern "system" fn MsiGetFileVersionW(
    path: usize,
    version: usize,
    version_length: usize,
    language: usize,
    language_length: usize,
) -> u32 {
    let path = door_windows_abi::widen(path);
    let version = door_windows_abi::widen(version);
    let version_length = door_windows_abi::widen(version_length);
    let language = door_windows_abi::widen(language);
    let language_length = door_windows_abi::widen(language_length);
    {
        let Ok(Some(p)) = wide(path) else {
            return error::INVALID_PARAMETER;
        };
        let mut t = Machine::new();
        let bytes = match t.read_file(&absolute(&p)) {
            Ok(b) => b,
            Err(e) => return e,
        };
        let Some(v) = door_msi::version::of_image(&bytes) else {
            return 1006; // ERROR_FILE_INVALID
        };
        if language_length != 0 {
            let _ = out_wide("1033", language, language_length);
        }
        out_wide(&door_msi::version::text(v), version, version_length)
    }
}

// ---- installation ------------------------------------------------------------------

/// Properties of a command line (`NAME=value`, quotes allowed).
pub(crate) fn command_properties(line: &str) -> Result<Vec<(String, String)>, u32> {
    let mut out = Vec::new();
    for word in engine::split_msiexec_line(line) {
        let Some((k, v)) = word.split_once('=') else {
            return Err(code::INVALID_COMMAND_LINE);
        };
        out.push((String::from(k.trim()), String::from(v)));
    }
    Ok(out)
}

/// The msiexec.exe that runs the package at `path` when it is of the other
/// bitness than this process: Windows Installer runs a 32-bit package (and
/// its 32-bit DLL custom actions) as a 32-bit installation, in
/// SysWOW64\msiexec.exe, and a 64-bit one in System32's (Sysnative from a
/// 32-bit process).
pub(crate) fn foreign_engine(path: &str) -> Option<&'static str> {
    let bytes = load(path).ok()?;
    let db = door_msi::Database::open(&bytes).ok()?;
    match (
        door_msi::install::package_is_32bit(&db),
        cfg!(target_arch = "x86"),
    ) {
        (true, false) => Some("C:\\Windows\\SysWOW64\\msiexec.exe"),
        (false, true) => Some("C:\\Windows\\Sysnative\\msiexec.exe"),
        _ => None,
    }
}
/// Run msiexec.exe `program` with `arguments` and wait for its exit code.
pub(crate) fn run_engine(program: &str, arguments: &str) -> u32 {
    let _ = door_sys::write(alloc::format!("DOOR_P17 MSI ENGINE {program}\n").as_bytes());
    Machine::new()
        .run(&alloc::format!("\"{program}\" {arguments}"), "", true)
        .unwrap_or(code::INSTALL_FAILURE)
}

/// Run `request` on the package at `path` with the internal UI at
/// `ui_level`: Windows Installer's result. A package of the other bitness
/// runs in that msiexec.exe, with the same request (an external UI
/// handler does not follow it).
pub(crate) fn run_request(
    path: &str,
    request: Request,
    properties: &[(String, String)],
    ui_level: u32,
    passive: bool,
    log: Option<String>,
) -> (u32, Vec<(String, String)>) {
    if let Some(program) = foreign_engine(path) {
        let mut line = match &request {
            Request::Install => alloc::format!("/i \"{path}\""),
            Request::Remove => alloc::format!("/x \"{path}\""),
            Request::Repair(mode) => alloc::format!("/f{mode} \"{path}\""),
        };
        line.push_str(match ui_level & 0x7 {
            level::NONE => " /qn",
            level::BASIC => " /qb",
            level::REDUCED => " /qr",
            _ => "",
        });
        // INSTALLUILEVEL_HIDECANCEL, PROGRESSONLY and ENDDIALOG as
        // msiexec's /qb modifiers.
        if ui_level & 0x7 == level::BASIC {
            if ui_level & 0x20 != 0 {
                line.push('!');
            }
            if ui_level & 0x40 != 0 {
                line.push('-');
            } else if ui_level & 0x80 != 0 {
                line.push('+');
            }
        }
        if passive {
            line.push_str(" /passive");
        }
        if let Some(l) = &log {
            line.push_str(&alloc::format!(" /l*v \"{l}\""));
        }
        for (k, v) in properties {
            line.push_str(&alloc::format!(" {k}=\"{}\"", v.replace('"', "\"\"")));
        }
        return (run_engine(program, &line), Vec::new());
    }
    run_request_with(path, None, request, properties, ui_level, passive, log)
}
/// `run_request` for a package whose bytes must hash to `expected` (an
/// elevated msiexec.exe's approved package): the package is read once, and
/// those bytes are what is installed.
pub(crate) fn run_request_with(
    path: &str,
    expected: Option<&[u8; 32]>,
    request: Request,
    properties: &[(String, String)],
    ui_level: u32,
    passive: bool,
    log: Option<String>,
) -> (u32, Vec<(String, String)>) {
    let bytes = match load(path) {
        Ok(b) => Arc::new(b),
        Err(_) => return (code::PACKAGE_OPEN_FAILED, Vec::new()),
    };
    if let Some(expected) = expected {
        use sha2::Digest;
        if sha2::Sha256::digest(bytes.as_slice()).as_slice() != expected.as_slice() {
            let _ = door_sys::write(b"DOOR_P16 MSI ELEVATE package changed\n");
            return (code::BAD_PACKAGE, Vec::new());
        }
    }
    let Ok(db) = door_msi::Database::open(&bytes) else {
        return (code::BAD_PACKAGE, Vec::new());
    };
    let caption = db.property("ProductName").unwrap_or_default();
    let mut target = Machine::new();
    if log.is_some() {
        target.log = Some(Vec::new());
    }
    let mut ui = Interface::new(ui_level, passive, &caption);
    ui.external = SETTINGS.with(|s| s.external);
    ui.package = Some(bytes.clone());
    let started = alloc::format!(
        "DOOR_P16 MSI START package={path} request={:?} ui={ui_level}",
        request
    );
    target.log(&started);
    let (status, public) = {
        let mut session = Session::new(
            &db,
            &mut target,
            path,
            request,
            layout(),
            properties,
            ui_level,
        );
        let status = session.run(&mut ui);
        // Public properties (all upper case) for an elevated continuation.
        let public = if status == code::ELEVATION_REQUIRED {
            session
                .state
                .properties
                .iter()
                .filter(|(k, _)| {
                    k.chars()
                        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
                        && !matches!(
                            k.as_str(),
                            "ROOTDRIVE"
                                | "INSTALLLEVEL"
                                | "UILEVEL"
                                | "ACTION"
                                | "DATABASE"
                                | "SOURCEDIR"
                        )
                        && db.property(k).as_deref() != Some(v_of(&session.state, k))
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        (status, public)
    };
    ui.close();
    target.release();
    target.log(&alloc::format!(
        "DOOR_P16 MSI END status={status} product={caption}"
    ));
    if let (Some(path), Some(lines)) = (log, target.log.take()) {
        let mut text = String::from("=== Verbose logging started ===\r\n");
        for l in lines {
            text.push_str(&l);
            text.push_str("\r\n");
        }
        text.push_str(&alloc::format!(
            "=== Verbose logging stopped: exit code {status} ===\r\n"
        ));
        let mut bytes = vec![0xff, 0xfe];
        bytes.extend(text.encode_utf16().flat_map(|u| u.to_le_bytes()));
        let _ = Machine::new().write_file(&path, &bytes);
    }
    (status, public)
}
fn v_of<'a>(state: &'a State, name: &str) -> &'a str {
    state.property(name)
}
fn settings() -> (u32, bool, Option<String>) {
    SETTINGS.with(|s| (s.ui_level, s.passive, s.log.clone()))
}
/// A request's status for an API caller: a per-machine package without
/// the administrator's token is ERROR_INSTALL_FAILURE's privilege form.
fn api_status(status: u32) -> u32 {
    if status == code::ELEVATION_REQUIRED {
        code::INSUFFICIENT_PRIVILEGES
    } else {
        status
    }
}
fn install_product(package: Option<String>, line: Option<String>) -> u32 {
    let Some(package) = package else {
        return error::INVALID_PARAMETER;
    };
    let properties = match command_properties(&line.unwrap_or_default()) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let get = |k: &str| {
        properties
            .iter()
            .find(|p| p.0.eq_ignore_ascii_case(k))
            .map(|p| p.1.clone())
    };
    let request = if get("REMOVE").is_some_and(|v| v.eq_ignore_ascii_case("ALL")) {
        Request::Remove
    } else if get("REINSTALL").is_some() {
        Request::Repair(get("REINSTALLMODE").unwrap_or_else(|| String::from("omus")))
    } else {
        Request::Install
    };
    let (ui_level, passive, log) = settings();
    api_status(
        run_request(
            &absolute(&package),
            request,
            &properties,
            ui_level,
            passive,
            log,
        )
        .0,
    )
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiInstallProductW(package: usize, command_line: usize) -> u32 {
    let package = door_windows_abi::widen(package);
    let command_line = door_windows_abi::widen(command_line);
    {
        match (wide(package), wide(command_line)) {
            (Ok(p), Ok(l)) => install_product(p, l),
            _ => error::INVALID_PARAMETER,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiInstallProductA(package: usize, command_line: usize) -> u32 {
    let package = door_windows_abi::widen(package);
    let command_line = door_windows_abi::widen(command_line);
    {
        match (ansi(package), ansi(command_line)) {
            (Ok(p), Ok(l)) => install_product(p, l),
            _ => error::INVALID_PARAMETER,
        }
    }
}
/// An installed product's cached package.
pub(crate) fn cached(product: &str) -> Result<String, u32> {
    let mut t = Machine::new();
    let record = records::product(&mut t, product).ok_or(code::UNKNOWN_PRODUCT)?;
    if !t.exists(&record.local_package) {
        return Err(code::INSTALL_SOURCE_ABSENT);
    }
    Ok(record.local_package)
}
fn configure(product: Option<String>, state: i32, line: Option<String>) -> u32 {
    let Some(product) = product else {
        return error::INVALID_PARAMETER;
    };
    let properties = match command_properties(&line.unwrap_or_default()) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let package = match cached(&product) {
        Ok(p) => p,
        Err(e) => return e,
    };
    // INSTALLSTATE_ABSENT removes; LOCAL / DEFAULT configure.
    let request = match state {
        2 => Request::Remove,
        3 | 5 => Request::Install,
        _ => return error::INVALID_PARAMETER,
    };
    let (ui_level, passive, log) = settings();
    api_status(run_request(&package, request, &properties, ui_level, passive, log).0)
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiConfigureProductW(product: usize, install_level: i32, state: i32) -> u32 {
    let product = door_windows_abi::widen(product);
    {
        let _ = install_level;
        match wide(product) {
            Ok(p) => configure(p, state, None),
            Err(e) => e,
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiConfigureProductExW(
    product: usize,
    install_level: i32,
    state: i32,
    command_line: usize,
) -> u32 {
    let product = door_windows_abi::widen(product);
    let command_line = door_windows_abi::widen(command_line);
    {
        let _ = install_level;
        match (wide(product), wide(command_line)) {
            (Ok(p), Ok(l)) => configure(p, state, l),
            _ => error::INVALID_PARAMETER,
        }
    }
}
/// REINSTALLMODE bits as their letters.
pub(crate) fn reinstall_letters(mode: u32) -> String {
    let mut s = String::new();
    for (bit, letter) in [
        (0x2, 'p'),
        (0x4, 'o'),
        (0x8, 'e'),
        (0x10, 'd'),
        (0x20, 'c'),
        (0x40, 'a'),
        (0x80, 'm'),
        (0x100, 'u'),
        (0x200, 's'),
        (0x400, 'v'),
    ] {
        if mode & bit != 0 {
            s.push(letter);
        }
    }
    if s.is_empty() {
        String::from("omus")
    } else {
        s
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn MsiReinstallProductW(product: usize, mode: u32) -> u32 {
    let product = door_windows_abi::widen(product);
    {
        let Ok(Some(product)) = wide(product) else {
            return error::INVALID_PARAMETER;
        };
        let package = match cached(&product) {
            Ok(p) => p,
            Err(e) => return e,
        };
        let (ui_level, passive, log) = settings();
        api_status(
            run_request(
                &package,
                Request::Repair(reinstall_letters(mode)),
                &[],
                ui_level,
                passive,
                log,
            )
            .0,
        )
    }
}
