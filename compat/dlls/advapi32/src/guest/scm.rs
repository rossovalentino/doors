//! The Service Control Manager's API: the database in
//! HKLM\SYSTEM\CurrentControlSet\Services (CreateService, DeleteService,
//! QueryServiceConfig(2), ChangeServiceConfig(2), EnumServicesStatus(Ex))
//! and Init's manager for status and control (StartService,
//! ControlService, QueryServiceStatus(Ex)); a service process's side:
//! StartServiceCtrlDispatcher, RegisterServiceCtrlHandler(Ex),
//! SetServiceStatus.
use super::*;
use door_windows_abi::layout::{self, Field::*};
/// QUERY_SERVICE_CONFIG: dwServiceType, dwStartType, dwErrorControl,
/// lpBinaryPathName, lpLoadOrderGroup, dwTagId, lpDependencies,
/// lpServiceStartName, lpDisplayName (64 bytes on x64, 36 on x86).
const QUERY_SERVICE_CONFIG: [layout::Field; 9] = [
    U32, U32, U32, Pointer, Pointer, U32, Pointer, Pointer, Pointer,
];
/// SERVICE_FAILURE_ACTIONS: dwResetPeriod, lpRebootMsg, lpCommand,
/// cActions, lpsaActions.
const FAILURE_ACTIONS: [layout::Field; 5] = [U32, Pointer, Pointer, U32, Pointer];
/// ENUM_SERVICE_STATUS(_PROCESS): lpServiceName, lpDisplayName, then
/// SERVICE_STATUS (7 DWORDs) or SERVICE_STATUS_PROCESS (9).
const ENUM_SERVICE_STATUS: [layout::Field; 3] = [Pointer, Pointer, Bytes(28)];
const ENUM_SERVICE_STATUS_PROCESS: [layout::Field; 3] = [Pointer, Pointer, Bytes(36)];
use alloc::boxed::Box;
use door_service_client::Services;
use door_service_protocol::scm::{self, Status, state};

const SERVICES: &str = "SYSTEM\\CurrentControlSet\\Services\\";
mod code {
    pub const INVALID_HANDLE: u32 = 6;
    pub const INVALID_NAME: u32 = 123;
    pub const SERVICE_EXISTS: u32 = 1073;
    pub const SERVICE_DOES_NOT_EXIST: u32 = 1060;
    pub const FAILED_SERVICE_CONTROLLER_CONNECT: u32 = 1063;
    pub const INVALID_LEVEL: u32 = 124;
    pub const SERVICE_REQUEST_TIMEOUT: u32 = 1053;
}
/// SERVICE_CONFIG_* levels of ChangeServiceConfig2/QueryServiceConfig2.
const CONFIG_DESCRIPTION: u32 = 1;
const CONFIG_FAILURE_ACTIONS: u32 = 2;
const CONFIG_FAILURE_ACTIONS_FLAG: u32 = 4;
/// Failure actions a service keeps (Init's manager holds four).
const MAX_ACTIONS: usize = 4;
/// SERVICE_START: restart actions need it on the handle, as on Windows.
const SERVICE_START: u32 = 0x10;
const SERVICE_NO_CHANGE: u32 = 0xffff_ffff;
const SC_MANAGER_CREATE_SERVICE: u32 = 0x2;
/// Access rights that modify a service (DELETE, SERVICE_CHANGE_CONFIG,
/// SERVICE_START, SERVICE_STOP, SERVICE_PAUSE_CONTINUE, WRITE_DAC,
/// WRITE_OWNER, GENERIC_ALL, GENERIC_WRITE).
const MODIFY: u32 =
    0x0001_0000 | 0x2 | 0x10 | 0x20 | 0x40 | 0x0004_0000 | 0x0008_0000 | 0x1000_0000 | 0x4000_0000;

static MANAGER: Lock<Option<Services>> = Lock::new(None);
fn manager<R>(f: impl FnOnce(&mut Services) -> Result<R, u32>) -> Result<R, u32> {
    MANAGER.with(|m| {
        if m.is_none() {
            let registry =
                unsafe { DoorGetInheritedHandle(door_abi::platform::inherited::REGISTRY as u32) };
            *m = Some(
                Services::connect(registry).map_err(|_| code::FAILED_SERVICE_CONTROLLER_CONNECT)?,
            );
        }
        f(m.as_mut().unwrap())
    })
}

#[derive(Clone)]
enum Handle {
    Manager {
        access: u32,
    },
    Service {
        name: String,
        access: u32,
    },
    /// SERVICE_STATUS_HANDLE of a service this process runs.
    Status {
        name: String,
    },
}
static HANDLES: Lock<Vec<Option<Handle>>> = Lock::new(Vec::new());
const HANDLE_BASE: u64 = 0x0000_0000_00c0_0000;
fn new_handle(h: Handle) -> u64 {
    HANDLES.with(|t| {
        let i = t.iter().position(Option::is_none).unwrap_or_else(|| {
            t.push(None);
            t.len() - 1
        });
        t[i] = Some(h);
        HANDLE_BASE + 8 * i as u64
    })
}
fn handle(h: u64) -> Result<Handle, u32> {
    HANDLES.with(|t| {
        h.checked_sub(HANDLE_BASE)
            .filter(|o| o % 8 == 0)
            .and_then(|o| t.get((o / 8) as usize).cloned().flatten())
            .ok_or(code::INVALID_HANDLE)
    })
}
fn service(h: u64) -> Result<(String, u32), u32> {
    match handle(h)? {
        Handle::Service { name, access } => Ok((name, access)),
        _ => Err(code::INVALID_HANDLE),
    }
}
fn string(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}
fn key_of(name: &str) -> Vec<u16> {
    let mut k: Vec<u16> = SERVICES.encode_utf16().collect();
    k.extend(name.encode_utf16());
    k
}
fn open_key(name: &str, access: u16) -> Open {
    Open {
        root: RegistryRoot::LocalMachine,
        view32: false,
        path: key_of(name),
        access,
    }
}
/// A value of the service's key: REG_SZ/EXPAND_SZ as text, DWORD,
/// MULTI_SZ as its strings.
enum Value {
    Text(String),
    Number(u32),
    Multi(Vec<String>),
    Binary(Vec<u8>),
}
fn values(name: &str) -> Result<Vec<(String, Value)>, u32> {
    with_key(&open_key(name, 1), false, |s, key| {
        let version = s.registry_version(key).unwrap_or(0);
        let mut out = Vec::new();
        for i in 0..64 {
            let Ok(e) = s.registry_enumerate_value(key, i, version) else {
                break;
            };
            let data = e.value.data();
            let units: Vec<u16> = data
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            let v = match e.value.kind {
                1 | 2 => Value::Text(string(units.split(|u| *u == 0).next().unwrap_or(&[]))),
                4 if data.len() >= 4 => {
                    Value::Number(u32::from_le_bytes(data[..4].try_into().unwrap()))
                }
                7 => Value::Multi(
                    units
                        .split(|u| *u == 0)
                        .filter(|s| !s.is_empty())
                        .map(string)
                        .collect(),
                ),
                3 => Value::Binary(data.to_vec()),
                _ => continue,
            };
            out.push((string(e.name.units()), v));
        }
        Ok(out)
    })
    .map_err(|e| {
        if e == error::FILE_NOT_FOUND {
            code::SERVICE_DOES_NOT_EXIST
        } else {
            e
        }
    })
}
fn text_of(v: &[(String, Value)], name: &str) -> String {
    v.iter()
        .find_map(|(n, x)| match x {
            Value::Text(t) if n.eq_ignore_ascii_case(name) => Some(t.clone()),
            _ => None,
        })
        .unwrap_or_default()
}
fn number_of(v: &[(String, Value)], name: &str) -> Option<u32> {
    v.iter().find_map(|(n, x)| match x {
        Value::Number(d) if n.eq_ignore_ascii_case(name) => Some(*d),
        _ => None,
    })
}
fn set_value(key: &Open, name: &str, kind: u8, data: &[u8]) -> Result<(), u32> {
    with_key(key, true, |s, k| {
        let units: Vec<u16> = name.encode_utf16().collect();
        s.registry_set(k, &units, RegistryValue::new(kind, data).map_err(win32)?)
            .map_err(win32)
    })
}
fn wide_bytes(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}
fn multi_bytes(list: &[String]) -> Vec<u8> {
    let mut b = Vec::new();
    for s in list {
        b.extend(wide_bytes(s));
    }
    b.extend_from_slice(&[0, 0]);
    b
}
/// A double-NUL-terminated list (lpDependencies).
fn read_multi(p: u64, wide: bool) -> Result<Vec<String>, u32> {
    let mut out = Vec::new();
    let mut at = p;
    if p == 0 {
        return Ok(out);
    }
    for _ in 0..64 {
        let s = text(at, wide)?;
        if s.is_empty() {
            break;
        }
        at += (s.len() as u64 + 1) * if wide { 2 } else { 1 };
        out.push(string(&s));
    }
    Ok(out)
}
fn elevated() -> bool {
    identity().elevated
}

// ---- the manager ----------------------------------------------------------------
fn open_manager(machine: u64, database: u64, access: u32, wide: bool) -> u64 {
    let r = (|| {
        let machine = string(&text(machine, wide)?);
        let local = string(&computer_name());
        let trimmed = machine.trim_start_matches('\\');
        if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case(&local) && trimmed != "." {
            return Err(1722); // RPC_S_SERVER_UNAVAILABLE
        }
        let database = string(&text(database, wide)?);
        if !database.is_empty() && !database.eq_ignore_ascii_case("ServicesActive") {
            return Err(1065); // ERROR_DATABASE_DOES_NOT_EXIST
        }
        if access & SC_MANAGER_CREATE_SERVICE != 0 && !elevated() {
            return Err(error::ACCESS_DENIED);
        }
        Ok(new_handle(Handle::Manager { access }))
    })();
    r.unwrap_or_else(|e| {
        last_error(e);
        0
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenSCManagerW(machine: usize, database: usize, access: u32) -> usize {
    let machine = door_windows_abi::widen(machine);
    let database = door_windows_abi::widen(database);
    door_windows_abi::narrow((move || -> u64 {
        open_manager(machine, database, access, true)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenSCManagerA(machine: usize, database: usize, access: u32) -> usize {
    let machine = door_windows_abi::widen(machine);
    let database = door_windows_abi::widen(database);
    door_windows_abi::narrow((move || -> u64 {
        open_manager(machine, database, access, false)
    })())
}
/// The service named `name` exists: in the registry, or as a Doors
/// system service the manager reports.
fn exists(name: &str) -> Result<String, u32> {
    if let Some((n, _, _)) = listed()
        .into_iter()
        .find(|(n, _, _)| n.eq_ignore_ascii_case(name))
    {
        return Ok(n);
    }
    values(name).map(|_| String::from(name))
}
fn open_service(scm: u64, name: u64, access: u32, wide: bool) -> u64 {
    let r = (|| {
        if !matches!(handle(scm)?, Handle::Manager { .. }) {
            return Err(code::INVALID_HANDLE);
        }
        let name = string(&text(name, wide)?);
        if scm::name(name.as_bytes()).is_none() {
            return Err(code::INVALID_NAME);
        }
        if access & MODIFY != 0 && !elevated() {
            return Err(error::ACCESS_DENIED);
        }
        let name = exists(&name)?;
        Ok(new_handle(Handle::Service { name, access }))
    })();
    r.unwrap_or_else(|e| {
        last_error(e);
        0
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenServiceW(scm: usize, name: usize, access: u32) -> usize {
    let scm = door_windows_abi::widen(scm);
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 { open_service(scm, name, access, true) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn OpenServiceA(scm: usize, name: usize, access: u32) -> usize {
    let scm = door_windows_abi::widen(scm);
    let name = door_windows_abi::widen(name);
    door_windows_abi::narrow((move || -> u64 { open_service(scm, name, access, false) })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CloseServiceHandle(h: usize) -> i32 {
    let h = door_windows_abi::widen(h);
    {
        let found = HANDLES.with(|t| {
            h.checked_sub(HANDLE_BASE)
                .filter(|o| o % 8 == 0)
                .and_then(|o| t.get_mut((o / 8) as usize))
                .and_then(Option::take)
                .is_some()
        });
        last_error(if found {
            error::SUCCESS
        } else {
            code::INVALID_HANDLE
        })
    }
}
#[allow(clippy::too_many_arguments)]
fn create_service(
    scm: u64,
    name: u64,
    display: u64,
    access: u32,
    kind: u32,
    start: u32,
    error_control: u32,
    path: u64,
    group: u64,
    tag: u64,
    depends: u64,
    account: u64,
    _password: u64,
    wide: bool,
) -> u64 {
    let r = (|| {
        match handle(scm)? {
            Handle::Manager { access } if access & SC_MANAGER_CREATE_SERVICE != 0 => (),
            Handle::Manager { .. } => return Err(error::ACCESS_DENIED),
            _ => return Err(code::INVALID_HANDLE),
        }
        let name = string(&text(name, wide)?);
        if scm::name(name.as_bytes()).is_none() || name.len() > 60 {
            return Err(code::INVALID_NAME);
        }
        let image = string(&text(path, wide)?);
        if image.is_empty() || tag != 0 || !(1..=4).contains(&start) && start != 0 {
            return Err(error::INVALID_PARAMETER);
        }
        if exists(&name).is_ok() {
            return Err(code::SERVICE_EXISTS);
        }
        let display = string(&text(display, wide)?);
        let key = open_key(&name, 3);
        set_value(&key, "Type", 4, &kind.to_le_bytes())?;
        set_value(&key, "Start", 4, &start.to_le_bytes())?;
        set_value(&key, "ErrorControl", 4, &error_control.to_le_bytes())?;
        set_value(&key, "ImagePath", 2, &wide_bytes(&image))?;
        let shown = if display.is_empty() {
            name.clone()
        } else {
            display
        };
        set_value(&key, "DisplayName", 1, &wide_bytes(&shown))?;
        let account = string(&text(account, wide)?);
        let account = if account.is_empty() {
            String::from("LocalSystem")
        } else {
            account
        };
        set_value(&key, "ObjectName", 1, &wide_bytes(&account))?;
        let group = string(&text(group, wide)?);
        if !group.is_empty() {
            set_value(&key, "Group", 1, &wide_bytes(&group))?;
        }
        let depends = read_multi(depends, wide)?;
        if !depends.is_empty() {
            set_value(&key, "DependOnService", 7, &multi_bytes(&depends))?;
        }
        let _ = manager(|m| m.reload(&name));
        Ok(new_handle(Handle::Service { name, access }))
    })();
    r.unwrap_or_else(|e| {
        last_error(e);
        0
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateServiceW(
    scm: usize,
    name: usize,
    display: usize,
    access: u32,
    kind: u32,
    start: u32,
    error_control: u32,
    path: usize,
    group: usize,
    tag: usize,
    depends: usize,
    account: usize,
    password: usize,
) -> usize {
    let scm = door_windows_abi::widen(scm);
    let name = door_windows_abi::widen(name);
    let display = door_windows_abi::widen(display);
    let path = door_windows_abi::widen(path);
    let group = door_windows_abi::widen(group);
    let tag = door_windows_abi::widen(tag);
    let depends = door_windows_abi::widen(depends);
    let account = door_windows_abi::widen(account);
    let password = door_windows_abi::widen(password);
    door_windows_abi::narrow((move || -> u64 {
        create_service(
            scm,
            name,
            display,
            access,
            kind,
            start,
            error_control,
            path,
            group,
            tag,
            depends,
            account,
            password,
            true,
        )
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn CreateServiceA(
    scm: usize,
    name: usize,
    display: usize,
    access: u32,
    kind: u32,
    start: u32,
    error_control: u32,
    path: usize,
    group: usize,
    tag: usize,
    depends: usize,
    account: usize,
    password: usize,
) -> usize {
    let scm = door_windows_abi::widen(scm);
    let name = door_windows_abi::widen(name);
    let display = door_windows_abi::widen(display);
    let path = door_windows_abi::widen(path);
    let group = door_windows_abi::widen(group);
    let tag = door_windows_abi::widen(tag);
    let depends = door_windows_abi::widen(depends);
    let account = door_windows_abi::widen(account);
    let password = door_windows_abi::widen(password);
    door_windows_abi::narrow((move || -> u64 {
        create_service(
            scm,
            name,
            display,
            access,
            kind,
            start,
            error_control,
            path,
            group,
            tag,
            depends,
            account,
            password,
            false,
        )
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn DeleteService(h: usize) -> i32 {
    let h = door_windows_abi::widen(h);
    {
        let r = (|| {
            let (name, access) = service(h)?;
            if access & 0x0001_0000 == 0 && access & 0x1000_0000 == 0 {
                return Err(error::ACCESS_DENIED);
            }
            values(&name)?;
            let open = open_key(&name, 3);
            delete_tree(&open)?;
            // delete_tree empties the key; the key itself goes too, so the
            // manager's reload finds no service of the name.
            with_key(
                &Open {
                    access: 1 | 2 | 4,
                    ..open
                },
                false,
                |s, k| s.registry_delete_key(k).map_err(win32),
            )?;
            let _ = manager(|m| m.reload(&name));
            Ok(())
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}

// ---- status and control ----------------------------------------------------------------
fn put_status(out: u64, s: &Status) -> Result<(), u32> {
    // SERVICE_STATUS: the first seven words of SERVICE_STATUS_PROCESS.
    write(out, &s.encode()[..28])
}
#[unsafe(no_mangle)]
pub extern "system" fn QueryServiceStatus(h: usize, out: usize) -> i32 {
    let h = door_windows_abi::widen(h);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let (name, _) = service(h)?;
            let s = manager(|m| m.query(&name))?;
            put_status(out, &s)
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn QueryServiceStatusEx(
    h: usize,
    level: u32,
    out: usize,
    size: u32,
    needed: usize,
) -> i32 {
    let h = door_windows_abi::widen(h);
    let out = door_windows_abi::widen(out);
    let needed = door_windows_abi::widen(needed);
    {
        let r = (|| {
            if level != 0 {
                return Err(code::INVALID_LEVEL);
            }
            let (name, _) = service(h)?;
            if needed != 0 {
                write_u32(needed, scm::STATUS_BYTES as u32)?;
            }
            if (size as usize) < scm::STATUS_BYTES {
                return Err(error::INSUFFICIENT_BUFFER);
            }
            let s = manager(|m| m.query(&name))?;
            write(out, &s.encode())
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
/// StartService: `argc` strings at `argv` (wide or ANSI) go to the
/// service's ServiceMain after its name, each NUL-terminated
/// (`scm::ARGUMENT_BYTES` in all at most: ERROR_INVALID_PARAMETER beyond).
fn start_service(h: u64, argc: u32, argv: u64, wide: bool) -> i32 {
    let r = (|| {
        let (name, _) = service(h)?;
        let mut arguments = Vec::new();
        if argc != 0 {
            if argv == 0 {
                return Err(error::INVALID_PARAMETER);
            }
            for i in 0..u64::from(argc) {
                let item = read_pointer(argv + i * POINTER as u64)?;
                if item == 0 {
                    return Err(error::INVALID_PARAMETER);
                }
                arguments.extend(string(&text(item, wide)?).bytes());
                arguments.push(0);
                if arguments.len() > scm::ARGUMENT_BYTES {
                    return Err(error::INVALID_PARAMETER);
                }
            }
        }
        manager(|m| m.start_with(&name, &arguments)).map(|_| ())
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn StartServiceW(h: usize, argc: u32, argv: usize) -> i32 {
    let h = door_windows_abi::widen(h);
    let argv = door_windows_abi::widen(argv);
    start_service(h, argc, argv, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn StartServiceA(h: usize, argc: u32, argv: usize) -> i32 {
    let h = door_windows_abi::widen(h);
    let argv = door_windows_abi::widen(argv);
    start_service(h, argc, argv, false)
}
/// ControlService: the control is queued, then (as Windows' waits for the
/// service's handler) this waits until the handler has returned; a handler
/// that does not return in 30 seconds fails the call with
/// ERROR_SERVICE_REQUEST_TIMEOUT (the manager logs event 7011).
#[unsafe(no_mangle)]
pub extern "system" fn ControlService(h: usize, control: u32, out: usize) -> i32 {
    let h = door_windows_abi::widen(h);
    let out = door_windows_abi::widen(out);
    {
        let r = (|| {
            let (name, _) = service(h)?;
            let (mut s, sequence) = manager(|m| m.control_sequenced(&name, control))?;
            let mut outcome = Ok(());
            if sequence != 0 {
                // A little past the manager's own 30 seconds.
                let polls = (scm::CONTROL_TIMEOUT_MS + 2000) / 50;
                outcome = Err(code::SERVICE_REQUEST_TIMEOUT);
                for _ in 0..polls {
                    match manager(|m| m.control_state(&name, sequence))? {
                        scm::CONTROL_DONE_STATE => {
                            outcome = Ok(());
                            break;
                        }
                        scm::CONTROL_TIMED_OUT => break,
                        _ => unsafe { DoorSleep(50) },
                    }
                }
                if let Ok(now) = manager(|m| m.query(&name)) {
                    s = now;
                }
            }
            if out != 0 {
                put_status(out, &s)?;
            }
            outcome
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}

// ---- configuration ----------------------------------------------------------------
/// QUERY_SERVICE_CONFIG (64 bytes on x64) and its strings.
fn query_config(h: u64, out: u64, size: u32, needed: u64, wide: bool) -> i32 {
    let r = (|| {
        let (name, _) = service(h)?;
        let v = values(&name).unwrap_or_default();
        let builtin = v.is_empty();
        let unit = if wide { 2 } else { 1 };
        let encode = |s: &str| -> Vec<u8> {
            if wide {
                wide_bytes(s)
            } else {
                s.bytes().chain([0]).collect()
            }
        };
        let path = text_of(&v, "ImagePath");
        let group = text_of(&v, "Group");
        let account = if builtin {
            String::from("LocalSystem")
        } else {
            text_of(&v, "ObjectName")
        };
        let display = if builtin {
            name.clone()
        } else {
            text_of(&v, "DisplayName")
        };
        let depends: Vec<u8> = {
            let list: Vec<String> = v
                .iter()
                .find_map(|(n, x)| match x {
                    Value::Multi(m) if n.eq_ignore_ascii_case("DependOnService") => Some(m.clone()),
                    _ => None,
                })
                .unwrap_or_default();
            let mut b = Vec::new();
            for s in &list {
                b.extend(encode(s));
            }
            b.extend(core::iter::repeat_n(0u8, unit));
            b
        };
        let strings = [
            encode(&path),
            encode(&group),
            depends,
            encode(&account),
            encode(&display),
        ];
        let fixed = layout::size(&QUERY_SERVICE_CONFIG);
        let total = fixed + strings.iter().map(Vec::len).sum::<usize>();
        if needed != 0 {
            write_u32(needed, total as u32)?;
        }
        if (size as usize) < total {
            return Err(error::INSUFFICIENT_BUFFER);
        }
        let mut b = vec![0u8; fixed];
        let config = &QUERY_SERVICE_CONFIG;
        layout::set(
            &mut b,
            config,
            0,
            u64::from(number_of(&v, "Type").unwrap_or(0x10)),
        );
        layout::set(
            &mut b,
            config,
            1,
            u64::from(number_of(&v, "Start").unwrap_or(2)),
        );
        layout::set(
            &mut b,
            config,
            2,
            u64::from(number_of(&v, "ErrorControl").unwrap_or(1)),
        );
        let mut at = fixed as u64;
        // lpBinaryPathName, lpLoadOrderGroup, (dwTagId), lpDependencies,
        // lpServiceStartName, lpDisplayName.
        for (index, s) in [3usize, 4, 6, 7, 8].into_iter().zip(&strings) {
            layout::set(&mut b, config, index, out + at);
            at += s.len() as u64;
        }
        for s in &strings {
            b.extend_from_slice(s);
        }
        write(out, &b)
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn QueryServiceConfigW(h: usize, out: usize, size: u32, needed: usize) -> i32 {
    let h = door_windows_abi::widen(h);
    let out = door_windows_abi::widen(out);
    let needed = door_windows_abi::widen(needed);
    query_config(h, out, size, needed, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn QueryServiceConfigA(h: usize, out: usize, size: u32, needed: usize) -> i32 {
    let h = door_windows_abi::widen(h);
    let out = door_windows_abi::widen(out);
    let needed = door_windows_abi::widen(needed);
    query_config(h, out, size, needed, false)
}
/// SERVICE_CONFIG_DESCRIPTION (1): SERVICE_DESCRIPTION { lpDescription }.
fn query_config2(h: u64, level: u32, out: u64, size: u32, needed: u64, wide: bool) -> i32 {
    let r = (|| {
        let (name, _) = service(h)?;
        match level {
            CONFIG_DESCRIPTION => (),
            CONFIG_FAILURE_ACTIONS => return query_failure_actions(&name, out, size, needed, wide),
            CONFIG_FAILURE_ACTIONS_FLAG => {
                let flag = values(&name)
                    .ok()
                    .and_then(|v| number_of(&v, "FailureActionsOnNonCrashFailures"))
                    .map_or(0u32, |n| u32::from(n != 0));
                if needed != 0 {
                    write_u32(needed, 4)?;
                }
                if size < 4 {
                    return Err(error::INSUFFICIENT_BUFFER);
                }
                return write_u32(out, flag);
            }
            _ => return Err(code::INVALID_LEVEL),
        }
        let description = values(&name)
            .map(|v| text_of(&v, "Description"))
            .unwrap_or_default();
        let data: Vec<u8> = if description.is_empty() {
            Vec::new()
        } else if wide {
            wide_bytes(&description)
        } else {
            description.bytes().chain([0]).collect()
        };
        // SERVICE_DESCRIPTION: lpDescription, then the text.
        let total = POINTER + data.len();
        if needed != 0 {
            write_u32(needed, total as u32)?;
        }
        if (size as usize) < total {
            return Err(error::INSUFFICIENT_BUFFER);
        }
        let pointer = if data.is_empty() {
            0
        } else {
            out + POINTER as u64
        };
        let mut b = pointer.to_le_bytes()[..POINTER].to_vec();
        b.extend_from_slice(&data);
        write(out, &b)
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
/// SERVICE_FAILURE_ACTIONS (40 bytes on x64, 20 on x86) followed by its
/// actions and its reboot message and command strings, as Windows lays
/// them out.
fn query_failure_actions(
    name: &str,
    out: u64,
    size: u32,
    needed: u64,
    wide: bool,
) -> Result<(), u32> {
    let v = values(name).unwrap_or_default();
    let (reset, actions) = failure_of(&v).unwrap_or((0, Vec::new()));
    let encode = |s: String| -> Vec<u8> {
        if wide {
            wide_bytes(&s)
        } else {
            s.bytes().chain([0]).collect()
        }
    };
    let message = Some(text_of(&v, "RebootMessage"))
        .filter(|s| !s.is_empty())
        .map(encode);
    let command = Some(text_of(&v, "FailureCommand"))
        .filter(|s| !s.is_empty())
        .map(encode);
    let actions_at = layout::size(&FAILURE_ACTIONS) as u64;
    let message_at = actions_at + 8 * actions.len() as u64;
    let command_at = message_at + message.as_ref().map_or(0, |m| m.len() as u64);
    let total = command_at + command.as_ref().map_or(0, |c| c.len() as u64);
    if needed != 0 {
        write_u32(needed, total as u32)?;
    }
    if u64::from(size) < total {
        return Err(error::INSUFFICIENT_BUFFER);
    }
    let mut b = vec![0u8; actions_at as usize];
    let array = if actions.is_empty() {
        0
    } else {
        out + actions_at
    };
    for (index, value) in [
        u64::from(reset),
        message.as_ref().map_or(0, |_| out + message_at),
        command.as_ref().map_or(0, |_| out + command_at),
        actions.len() as u64,
        array,
    ]
    .into_iter()
    .enumerate()
    {
        layout::set(&mut b, &FAILURE_ACTIONS, index, value);
    }
    for (kind, delay) in &actions {
        b.extend_from_slice(&kind.to_le_bytes());
        b.extend_from_slice(&delay.to_le_bytes());
    }
    b.extend(message.unwrap_or_default());
    b.extend(command.unwrap_or_default());
    write(out, &b)
}
#[unsafe(no_mangle)]
pub extern "system" fn QueryServiceConfig2W(
    h: usize,
    level: u32,
    out: usize,
    size: u32,
    needed: usize,
) -> i32 {
    let h = door_windows_abi::widen(h);
    let out = door_windows_abi::widen(out);
    let needed = door_windows_abi::widen(needed);
    query_config2(h, level, out, size, needed, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn QueryServiceConfig2A(
    h: usize,
    level: u32,
    out: usize,
    size: u32,
    needed: usize,
) -> i32 {
    let h = door_windows_abi::widen(h);
    let out = door_windows_abi::widen(out);
    let needed = door_windows_abi::widen(needed);
    query_config2(h, level, out, size, needed, false)
}
#[allow(clippy::too_many_arguments)]
fn change_config(
    h: u64,
    kind: u32,
    start: u32,
    error_control: u32,
    path: u64,
    group: u64,
    tag: u64,
    depends: u64,
    account: u64,
    _password: u64,
    display: u64,
    wide: bool,
) -> i32 {
    let r = (|| {
        let (name, access) = service(h)?;
        if access & 0x2 == 0 && access & 0x1000_0000 == 0 {
            return Err(error::ACCESS_DENIED);
        }
        values(&name)?;
        let key = open_key(&name, 3);
        for (value, n) in [
            (kind, "Type"),
            (start, "Start"),
            (error_control, "ErrorControl"),
        ] {
            if value != SERVICE_NO_CHANGE {
                set_value(&key, n, 4, &value.to_le_bytes())?;
            }
        }
        if tag != 0 {
            return Err(error::INVALID_PARAMETER);
        }
        for (p, n, kind) in [
            (path, "ImagePath", 2u8),
            (group, "Group", 1),
            (account, "ObjectName", 1),
            (display, "DisplayName", 1),
        ] {
            if p != 0 {
                set_value(&key, n, kind, &wide_bytes(&string(&text(p, wide)?)))?;
            }
        }
        if depends != 0 {
            set_value(
                &key,
                "DependOnService",
                7,
                &multi_bytes(&read_multi(depends, wide)?),
            )?;
        }
        let _ = manager(|m| m.reload(&name));
        Ok(())
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn ChangeServiceConfigW(
    h: usize,
    kind: u32,
    start: u32,
    error_control: u32,
    path: usize,
    group: usize,
    tag: usize,
    depends: usize,
    account: usize,
    password: usize,
    display: usize,
) -> i32 {
    let h = door_windows_abi::widen(h);
    let path = door_windows_abi::widen(path);
    let group = door_windows_abi::widen(group);
    let tag = door_windows_abi::widen(tag);
    let depends = door_windows_abi::widen(depends);
    let account = door_windows_abi::widen(account);
    let password = door_windows_abi::widen(password);
    let display = door_windows_abi::widen(display);
    {
        change_config(
            h,
            kind,
            start,
            error_control,
            path,
            group,
            tag,
            depends,
            account,
            password,
            display,
            true,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn ChangeServiceConfigA(
    h: usize,
    kind: u32,
    start: u32,
    error_control: u32,
    path: usize,
    group: usize,
    tag: usize,
    depends: usize,
    account: usize,
    password: usize,
    display: usize,
) -> i32 {
    let h = door_windows_abi::widen(h);
    let path = door_windows_abi::widen(path);
    let group = door_windows_abi::widen(group);
    let tag = door_windows_abi::widen(tag);
    let depends = door_windows_abi::widen(depends);
    let account = door_windows_abi::widen(account);
    let password = door_windows_abi::widen(password);
    let display = door_windows_abi::widen(display);
    {
        change_config(
            h,
            kind,
            start,
            error_control,
            path,
            group,
            tag,
            depends,
            account,
            password,
            display,
            false,
        )
    }
}
/// The registry's FailureActions value for `reset` and `actions` (the
/// layout Windows stores: a 20-byte header, then the actions).
fn failure_bytes(reset: u32, actions: &[(u32, u32)]) -> Vec<u8> {
    let mut b = Vec::with_capacity(20 + 8 * actions.len());
    b.extend_from_slice(&reset.to_le_bytes());
    b.extend_from_slice(&[0u8; 8]);
    b.extend_from_slice(&(actions.len() as u32).to_le_bytes());
    b.extend_from_slice(&20u32.to_le_bytes());
    for (kind, delay) in actions {
        b.extend_from_slice(&kind.to_le_bytes());
        b.extend_from_slice(&delay.to_le_bytes());
    }
    b
}
/// The (reset period, actions) a FailureActions value holds.
fn failure_of(v: &[(String, Value)]) -> Option<(u32, Vec<(u32, u32)>)> {
    let data = v.iter().find_map(|(n, x)| match x {
        Value::Binary(d) if n.eq_ignore_ascii_case("FailureActions") => Some(d),
        _ => None,
    })?;
    let w = |at: usize| {
        data.get(at..at + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let count = (w(12)? as usize).min(MAX_ACTIONS);
    let actions = (0..count)
        .map_while(|i| Some((w(20 + 8 * i)?, w(24 + 8 * i)?)))
        .collect();
    Some((w(0)?, actions))
}
/// Whether the caller's effective token (the thread's when it
/// impersonates) has SeShutdownPrivilege enabled.
fn shutdown_enabled() -> bool {
    const SE_SHUTDOWN: u32 = 19;
    super::object_of(0xFFFF_FFFF_FFFF_FFFA).is_ok_and(|object| {
        super::privileges_of(object)
            .iter()
            .any(|&(luid, attributes)| {
                luid == SE_SHUTDOWN && attributes & crate::PRIVILEGE_ENABLED != 0
            })
    })
}
fn change_config2(h: u64, level: u32, info: u64, wide: bool) -> i32 {
    let r = (|| {
        if !matches!(
            level,
            CONFIG_DESCRIPTION | CONFIG_FAILURE_ACTIONS | CONFIG_FAILURE_ACTIONS_FLAG
        ) {
            return Err(code::INVALID_LEVEL);
        }
        let (name, access) = service(h)?;
        if access & 0x2 == 0 && access & 0x1000_0000 == 0 {
            return Err(error::ACCESS_DENIED);
        }
        values(&name)?;
        let key = open_key(&name, 3);
        match level {
            CONFIG_DESCRIPTION => {
                let description = string(&text(read_pointer(info)?, wide)?);
                set_value(&key, "Description", 1, &wide_bytes(&description))?;
            }
            CONFIG_FAILURE_ACTIONS => {
                // SERVICE_FAILURE_ACTIONS: dwResetPeriod, lpRebootMsg,
                // lpCommand, cActions, lpsaActions.
                let mut f = [0u8; layout::size(&FAILURE_ACTIONS)];
                read(info, &mut f)?;
                let field = |index| layout::get(&f, &FAILURE_ACTIONS, index);
                let (reset, count, array) = (field(0) as u32, field(3) as u32 as usize, field(4));
                for (pointer, value) in [(field(1), "RebootMessage"), (field(2), "FailureCommand")]
                {
                    if pointer != 0 {
                        set_value(&key, value, 1, &wide_bytes(&string(&text(pointer, wide)?)))?;
                    }
                }
                // No actions array: the actions and reset period stay.
                if array != 0 {
                    if count > MAX_ACTIONS {
                        return Err(error::INVALID_PARAMETER);
                    }
                    let mut actions = Vec::with_capacity(count);
                    for i in 0..count {
                        let mut a = [0u8; 8];
                        read(array + 8 * i as u64, &mut a)?;
                        let kind = u32::from_le_bytes(a[..4].try_into().unwrap());
                        let delay = u32::from_le_bytes(a[4..].try_into().unwrap());
                        match kind {
                            scm::action::NONE => (),
                            // Restarting needs SERVICE_START on the handle.
                            scm::action::RESTART if access & (SERVICE_START | 0x1000_0000) == 0 => {
                                return Err(error::ACCESS_DENIED);
                            }
                            // Restarting the computer needs the caller's
                            // shutdown privilege enabled (AdjustTokenPrivileges).
                            scm::action::REBOOT if !shutdown_enabled() => {
                                return Err(error::ACCESS_DENIED);
                            }
                            scm::action::RESTART
                            | scm::action::REBOOT
                            | scm::action::RUN_COMMAND => {}
                            _ => return Err(error::INVALID_PARAMETER),
                        }
                        actions.push((kind, delay));
                    }
                    set_value(&key, "FailureActions", 3, &failure_bytes(reset, &actions))?;
                }
            }
            _ => {
                // SERVICE_FAILURE_ACTIONS_FLAG: fFailureActionsOnNonCrashFailures.
                let mut b = [0u8; 4];
                read(info, &mut b)?;
                let flag = u32::from(u32::from_le_bytes(b) != 0);
                set_value(
                    &key,
                    "FailureActionsOnNonCrashFailures",
                    4,
                    &flag.to_le_bytes(),
                )?;
            }
        }
        let _ = manager(|m| m.reload(&name));
        Ok(())
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn ChangeServiceConfig2W(h: usize, level: u32, info: usize) -> i32 {
    let h = door_windows_abi::widen(h);
    let info = door_windows_abi::widen(info);
    change_config2(h, level, info, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn ChangeServiceConfig2A(h: usize, level: u32, info: usize) -> i32 {
    let h = door_windows_abi::widen(h);
    let info = door_windows_abi::widen(info);
    change_config2(h, level, info, false)
}

// ---- enumeration ----------------------------------------------------------------
/// (name, display name, status) of every service matching `state` (1
/// active, 2 inactive, 3 all).
fn all(state_filter: u32) -> Result<Vec<(String, String, Status)>, u32> {
    let mut names: Vec<String> = with_key(
        &Open {
            root: RegistryRoot::LocalMachine,
            view32: false,
            path: SERVICES.trim_end_matches('\\').encode_utf16().collect(),
            access: 1,
        },
        false,
        |s, key| {
            let version = s.registry_version(key).unwrap_or(0);
            let mut out = Vec::new();
            for i in 0..1024 {
                match s.registry_enumerate_key(key, i, version) {
                    Ok(n) => out.push(string(n.units())),
                    Err(_) => break,
                }
            }
            Ok(out)
        },
    )
    .unwrap_or_default();
    let running = listed();
    for (n, _, _) in &running {
        if !names.iter().any(|x| x.eq_ignore_ascii_case(n)) {
            names.push(n.clone());
        }
    }
    names.sort_by_key(|n| n.to_ascii_lowercase());
    let mut out = Vec::new();
    for n in names {
        let v = values(&n).unwrap_or_default();
        // Win32 services only (drivers are not listed).
        if number_of(&v, "Type").is_some_and(|t| t & 0x30 == 0) {
            continue;
        }
        let status = manager(|m| m.query(&n)).unwrap_or(Status {
            kind: number_of(&v, "Type").unwrap_or(0x10),
            state: state::STOPPED,
            ..Status::default()
        });
        let active = status.state != state::STOPPED;
        if (state_filter == 1 && !active) || (state_filter == 2 && active) {
            continue;
        }
        let display = text_of(&v, "DisplayName");
        let display = if display.is_empty() {
            n.clone()
        } else {
            display
        };
        out.push((n, display, status));
    }
    Ok(out)
}
#[allow(clippy::too_many_arguments)]
fn enum_ex(
    scm: u64,
    level: u32,
    _kind: u32,
    state_filter: u32,
    out: u64,
    size: u32,
    needed: u64,
    returned: u64,
    resume: u64,
    wide: bool,
    extended: bool,
) -> i32 {
    let r = (|| {
        if !matches!(handle(scm)?, Handle::Manager { .. }) {
            return Err(code::INVALID_HANDLE);
        }
        if extended && level != 0 {
            return Err(code::INVALID_LEVEL);
        }
        if !(1..=3).contains(&state_filter) {
            return Err(error::INVALID_PARAMETER);
        }
        let mut start = 0u32;
        if resume != 0 {
            start = read_u32(resume)?;
        }
        let list = all(state_filter)?;
        let list = &list[(start as usize).min(list.len())..];
        // ENUM_SERVICE_STATUS_PROCESS (56 bytes on x64, 44 on x86) or
        // ENUM_SERVICE_STATUS (48, 36): two string pointers, then the status.
        let record = layout::size(if extended {
            &ENUM_SERVICE_STATUS_PROCESS[..]
        } else {
            &ENUM_SERVICE_STATUS[..]
        });
        let status_at = 2 * POINTER;
        let encode = |s: &str| -> Vec<u8> {
            if wide {
                wide_bytes(s)
            } else {
                s.bytes().chain([0]).collect()
            }
        };
        let sizes: Vec<usize> = list
            .iter()
            .map(|(n, d, _)| record + encode(n).len() + encode(d).len())
            .collect();
        let total: usize = sizes.iter().sum();
        let mut fits = 0;
        let mut used = 0;
        for s in &sizes {
            if used + s > size as usize {
                break;
            }
            used += s;
            fits += 1;
        }
        let mut fixed = vec![0u8; fits * record];
        let mut strings = Vec::new();
        let mut at = out + (fits * record) as u64;
        for (i, (n, d, s)) in list[..fits].iter().enumerate() {
            let r = &mut fixed[i * record..(i + 1) * record];
            let (name, display) = (encode(n), encode(d));
            r[..POINTER].copy_from_slice(&at.to_le_bytes()[..POINTER]);
            at += name.len() as u64;
            r[POINTER..2 * POINTER].copy_from_slice(&at.to_le_bytes()[..POINTER]);
            at += display.len() as u64;
            let status = s.encode();
            if extended {
                r[status_at..status_at + 36].copy_from_slice(&status);
            } else {
                r[status_at..status_at + 28].copy_from_slice(&status[..28]);
            }
            strings.extend(name);
            strings.extend(display);
        }
        fixed.extend(strings);
        if fits > 0 {
            write(out, &fixed)?;
        }
        write_u32(returned, fits as u32)?;
        write_u32(needed, (total - used) as u32)?;
        if fits < list.len() {
            if resume != 0 {
                write_u32(resume, start + fits as u32)?;
            }
            return Err(error::MORE_DATA);
        }
        if resume != 0 {
            write_u32(resume, 0)?;
        }
        Ok(())
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn EnumServicesStatusExW(
    scm: usize,
    level: u32,
    kind: u32,
    state: u32,
    out: usize,
    size: u32,
    needed: usize,
    returned: usize,
    resume: usize,
    _group: usize,
) -> i32 {
    let scm = door_windows_abi::widen(scm);
    let out = door_windows_abi::widen(out);
    let needed = door_windows_abi::widen(needed);
    let returned = door_windows_abi::widen(returned);
    let resume = door_windows_abi::widen(resume);
    {
        enum_ex(
            scm, level, kind, state, out, size, needed, returned, resume, true, true,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn EnumServicesStatusExA(
    scm: usize,
    level: u32,
    kind: u32,
    state: u32,
    out: usize,
    size: u32,
    needed: usize,
    returned: usize,
    resume: usize,
    _group: usize,
) -> i32 {
    let scm = door_windows_abi::widen(scm);
    let out = door_windows_abi::widen(out);
    let needed = door_windows_abi::widen(needed);
    let returned = door_windows_abi::widen(returned);
    let resume = door_windows_abi::widen(resume);
    {
        enum_ex(
            scm, level, kind, state, out, size, needed, returned, resume, false, true,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn EnumServicesStatusW(
    scm: usize,
    kind: u32,
    state: u32,
    out: usize,
    size: u32,
    needed: usize,
    returned: usize,
    resume: usize,
) -> i32 {
    let scm = door_windows_abi::widen(scm);
    let out = door_windows_abi::widen(out);
    let needed = door_windows_abi::widen(needed);
    let returned = door_windows_abi::widen(returned);
    let resume = door_windows_abi::widen(resume);
    {
        enum_ex(
            scm, 0, kind, state, out, size, needed, returned, resume, true, false,
        )
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn EnumServicesStatusA(
    scm: usize,
    kind: u32,
    state: u32,
    out: usize,
    size: u32,
    needed: usize,
    returned: usize,
    resume: usize,
) -> i32 {
    let scm = door_windows_abi::widen(scm);
    let out = door_windows_abi::widen(out);
    let needed = door_windows_abi::widen(needed);
    let returned = door_windows_abi::widen(returned);
    let resume = door_windows_abi::widen(resume);
    {
        enum_ex(
            scm, 0, kind, state, out, size, needed, returned, resume, false, false,
        )
    }
}
fn display_name(scm: u64, name: u64, out: u64, length: u64, wide: bool, to_key: bool) -> i32 {
    let r = (|| {
        if !matches!(handle(scm)?, Handle::Manager { .. }) {
            return Err(code::INVALID_HANDLE);
        }
        let wanted = string(&text(name, wide)?);
        let found = all(3)?
            .into_iter()
            .find(|(n, d, _)| {
                if to_key {
                    d.eq_ignore_ascii_case(&wanted)
                } else {
                    n.eq_ignore_ascii_case(&wanted)
                }
            })
            .ok_or(code::SERVICE_DOES_NOT_EXIST)?;
        let answer: Vec<u16> = if to_key { found.0 } else { found.1 }
            .encode_utf16()
            .collect();
        let capacity = read_u32(length)?;
        if answer.len() + 1 > capacity as usize {
            write_u32(length, answer.len() as u32 * if wide { 1 } else { 2 })?;
            return Err(error::INSUFFICIENT_BUFFER);
        }
        put_text(out, capacity, &answer, wide)?;
        write_u32(length, answer.len() as u32)
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn GetServiceDisplayNameW(
    scm: usize,
    name: usize,
    out: usize,
    length: usize,
) -> i32 {
    let scm = door_windows_abi::widen(scm);
    let name = door_windows_abi::widen(name);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    display_name(scm, name, out, length, true, false)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetServiceKeyNameW(
    scm: usize,
    display: usize,
    out: usize,
    length: usize,
) -> i32 {
    let scm = door_windows_abi::widen(scm);
    let display = door_windows_abi::widen(display);
    let out = door_windows_abi::widen(out);
    let length = door_windows_abi::widen(length);
    display_name(scm, display, out, length, true, true)
}

// ---- a service process --------------------------------------------------------------
/// The handler a service registered: (name, LPHANDLER_FUNCTION_EX or
/// LPHANDLER_FUNCTION, extended, context).
static HANDLERS: Lock<Vec<(String, u64, bool, u64)>> = Lock::new(Vec::new());
/// The service this own-process service program runs (set when the
/// dispatcher connects): handlers and status go to it whatever name the
/// program gives, as on Windows.
static SERVICE_NAME: Lock<Option<String>> = Lock::new(None);
#[derive(Clone, Copy)]
struct Entry {
    name: u64,
    main: u64,
}
fn register_handler(name: u64, handler: u64, extended: bool, context: u64, wide: bool) -> u64 {
    let r = (|| {
        let given = string(&text(name, wide)?);
        let name = SERVICE_NAME.with(|s| s.clone()).unwrap_or(given);
        if handler == 0 {
            return Err(error::INVALID_PARAMETER);
        }
        HANDLERS.with(|h| {
            h.retain(|(n, ..)| !n.eq_ignore_ascii_case(&name));
            h.push((name.clone(), handler, extended, context));
        });
        Ok(new_handle(Handle::Status { name }))
    })();
    r.unwrap_or_else(|e| {
        last_error(e);
        0
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn RegisterServiceCtrlHandlerW(name: usize, handler: usize) -> usize {
    let name = door_windows_abi::widen(name);
    let handler = door_windows_abi::widen(handler);
    door_windows_abi::narrow((move || -> u64 {
        register_handler(name, handler, false, 0, true)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn RegisterServiceCtrlHandlerA(name: usize, handler: usize) -> usize {
    let name = door_windows_abi::widen(name);
    let handler = door_windows_abi::widen(handler);
    door_windows_abi::narrow((move || -> u64 {
        register_handler(name, handler, false, 0, false)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn RegisterServiceCtrlHandlerExW(
    name: usize,
    handler: usize,
    context: usize,
) -> usize {
    let name = door_windows_abi::widen(name);
    let handler = door_windows_abi::widen(handler);
    let context = door_windows_abi::widen(context);
    door_windows_abi::narrow((move || -> u64 {
        register_handler(name, handler, true, context, true)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn RegisterServiceCtrlHandlerExA(
    name: usize,
    handler: usize,
    context: usize,
) -> usize {
    let name = door_windows_abi::widen(name);
    let handler = door_windows_abi::widen(handler);
    let context = door_windows_abi::widen(context);
    door_windows_abi::narrow((move || -> u64 {
        register_handler(name, handler, true, context, false)
    })())
}
#[unsafe(no_mangle)]
pub extern "system" fn SetServiceStatus(h: usize, status: usize) -> i32 {
    let h = door_windows_abi::widen(h);
    let status = door_windows_abi::widen(status);
    {
        let r = (|| {
            let Handle::Status { name } = handle(h)? else {
                return Err(code::INVALID_HANDLE);
            };
            let mut b = [0u8; 36];
            read(status, &mut b[..28])?;
            let s = Status::decode(&b).ok_or(error::INVALID_PARAMETER)?;
            manager(|m| m.set_status(&name, &s))
        })();
        last_error(r.err().unwrap_or(error::SUCCESS))
    }
}
fn dispatch(table: u64, wide: bool) -> i32 {
    let r = (|| {
        let mut entries = Vec::new();
        // SERVICE_TABLE_ENTRY: lpServiceName, lpServiceProc.
        let entry_bytes = 2 * POINTER as u64;
        for i in 0..16u64 {
            let name = read_pointer(table + entry_bytes * i)?;
            let main = read_pointer(table + entry_bytes * i + POINTER as u64)?;
            if name == 0 || main == 0 {
                break;
            }
            entries.push(Entry { name, main });
        }
        if entries.is_empty() {
            return Err(error::INVALID_PARAMETER);
        }
        // An own-process service: the first entry is the service Init
        // started this process for.
        let entry = entries[0];
        let name_units = text(entry.name, wide)?;
        let asked = string(&name_units);
        // A name the manager could not even take (empty, too long, with
        // slashes) is sent as none: it is ignored anyway.
        let asked = if scm::name(asked.as_bytes()).is_some() {
            asked
        } else {
            String::new()
        };
        // The table's name is ignored for an own-process service: the
        // manager answers which service this process runs.
        let mut real = [0u8; scm::NAME_BYTES];
        let n = manager(|m| m.register(&asked, &mut real))?;
        let name = String::from(core::str::from_utf8(&real[..n]).map_err(|_| error::INVALID_DATA)?);
        SERVICE_NAME.with(|s| *s = Some(name.clone()));
        // ServiceMain(argc, argv [the service's name, then the arguments
        // of the StartService that launched it]) on its own thread, as
        // Windows runs it; this thread delivers the controls.
        let mut given = [0u8; scm::ARGUMENT_BYTES];
        let n = manager(|m| m.arguments(&name, &mut given)).unwrap_or(0);
        let mut items: Vec<&str> = vec![name.as_str()];
        if let Ok(text) = core::str::from_utf8(&given[..n]) {
            items.extend(text.split_terminator('\0'));
        }
        let mut pointers: Vec<usize> = items
            .iter()
            .map(|item| {
                let address = if wide {
                    let units: Vec<u16> = item.encode_utf16().chain([0]).collect();
                    Box::leak(units.into_boxed_slice()).as_ptr() as u64
                } else {
                    let bytes: Vec<u8> = item.bytes().chain([0]).collect();
                    Box::leak(bytes.into_boxed_slice()).as_ptr() as u64
                };
                door_windows_abi::narrow(address)
            })
            .collect();
        let argc = pointers.len() as u32;
        // LPWSTR argv[] at the program's pointer width (P17), NULL-ended.
        pointers.push(0);
        let argv = Box::leak(pointers.into_boxed_slice());
        let start = Box::leak(Box::new((entry.main, argv.as_ptr() as u64, argc)));
        extern "system" fn service_main(parameter: usize) -> u32 {
            let parameter = door_windows_abi::widen(parameter);
            {
                // SAFETY: the leaked (ServiceMain, argv, argc) above.
                let (main, argv, argc) = unsafe { *(parameter as *const (u64, u64, u32)) };
                // ServiceMain(DWORD argc, LPWSTR *argv).
                let f: extern "system" fn(u32, usize) =
                    unsafe { core::mem::transmute(door_windows_abi::narrow(main)) };
                f(argc, door_windows_abi::narrow(argv));
                0
            }
        }
        let thread = unsafe {
            DoorCreateThreadEx(
                service_main as *const () as u64,
                start as *const _ as u64,
                0,
            )
        };
        if thread == 0 {
            return Err(error::NOT_ENOUGH_MEMORY);
        }
        loop {
            match manager(|m| m.query(&name)) {
                Ok(s) if s.state == state::STOPPED => break,
                Err(_) => break,
                _ => (),
            }
            match manager(|m| m.next_control(&name)) {
                Ok(Some((control, sequence))) => {
                    let handler = HANDLERS.with(|h| {
                        h.iter()
                            .find(|(n, ..)| n.eq_ignore_ascii_case(&name))
                            .map(|(_, f, ext, ctx)| (*f, *ext, *ctx))
                    });
                    if let Some((f, extended, context)) = handler {
                        if extended {
                            // HandlerEx(dwControl, dwEventType, lpEventData,
                            // lpContext).
                            let f: extern "system" fn(u32, u32, usize, usize) -> u32 =
                                unsafe { core::mem::transmute(door_windows_abi::narrow(f)) };
                            f(control, 0, 0, door_windows_abi::narrow(context));
                        } else {
                            let f: extern "system" fn(u32) =
                                unsafe { core::mem::transmute(door_windows_abi::narrow(f)) };
                            f(control);
                        }
                    }
                    // The handler returned: the caller's ControlService
                    // may go on.
                    let _ = manager(|m| m.control_done(&name, sequence));
                }
                Ok(None) => unsafe { DoorSleep(50) },
                Err(_) => break,
            }
        }
        Ok(())
    })();
    last_error(r.err().unwrap_or(error::SUCCESS))
}
#[unsafe(no_mangle)]
pub extern "system" fn StartServiceCtrlDispatcherW(table: usize) -> i32 {
    let table = door_windows_abi::widen(table);
    dispatch(table, true)
}
#[unsafe(no_mangle)]
pub extern "system" fn StartServiceCtrlDispatcherA(table: usize) -> i32 {
    let table = door_windows_abi::widen(table);
    dispatch(table, false)
}
/// The services the manager knows: (name, state, process).
fn listed() -> Vec<(String, u32, u32)> {
    let mut out = Vec::new();
    let _ = manager(|m| m.each(|e| out.push((String::from(e.name()), e.state, e.pid))));
    out
}
#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorCreateThreadEx(entry: u64, argument: u64, id: u64) -> u64;
    fn DoorSleep(millis: u32);
}
