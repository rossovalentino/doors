//! msiexec.exe's work (`DoorMsiexecMain`): its command line, the request
//! it names (/i, /x, /f, by package or product code), the UI level, the
//! log, and Windows Installer's error messages and exit codes. A
//! per-machine package started without the administrator's token asks for
//! it as Windows Installer does: User Account Control for msiexec.exe,
//! which carries on elevated from where this one stopped (the request is
//! handed over in the user's Installer key).
use super::api::{cached, foreign_engine, run_engine, run_request_with};
use super::*;
use door_msi::engine::{CommandLine, HELP, level, parse_command_line, split_msiexec_line};
use door_msi::execute::Target;
use door_msi::install::{RegValue, Root};
use target::Machine;

#[link(name = "user32", kind = "dylib")]
unsafe extern "system" {
    fn MessageBoxW(hwnd: usize, text: usize, caption: usize, kind: u32) -> i32;
    fn DoorRequestElevation(
        path: u64,
        path_len: u32,
        image: u64,
        image_len: u32,
        flags: u32,
    ) -> u32;
    fn DoorElevationResult(process: u64) -> u32;
}
#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn DoorGetModuleFileNameW(module: u64, buffer: u64, size: u32) -> u32;
    fn DoorGetNativePath(path: u64, out: u64, capacity: u32) -> u32;
    fn DoorWaitForSingleObjectEx(handle: u64, milliseconds: u32, alertable: i32) -> u32;
    fn DoorGetExitCodeProcess(handle: u64, code: u64) -> i32;
    fn DoorCloseHandle(handle: u64) -> i32;
    fn DoorLoadLibraryW(name: u64) -> u64;
    fn DoorFreeLibrary(module: u64) -> i32;
    fn RtlGetLastWin32Error() -> u32;
}

/// Where a request waits for its elevated msiexec.exe.
const HANDOFF: &str = "Software\\Microsoft\\Installer\\Handoff";

fn message_box(text: &str, caption: &str, style: u32) -> i32 {
    let t = utf16z(text);
    let c = utf16z(caption);
    unsafe {
        MessageBoxW(
            0,
            door_windows_abi::narrow(t.as_ptr() as u64),
            door_windows_abi::narrow(c.as_ptr() as u64),
            style,
        )
    }
}

/// Windows Installer's message box for a failure it reports itself.
fn report(status: u32, ui: u32, product: &str) {
    if ui <= level::NONE {
        return;
    }
    let text = match status {
        1605 => "This action is only valid for products that are currently installed.",
        1619 => {
            "This installation package could not be opened. Verify that the package exists and that you can access it, or contact the application vendor to verify that this is a valid Doors Installer package."
        }
        1620 => {
            "This installation package could not be opened. Contact the application vendor to verify that this is a valid Doors Installer package."
        }
        1618 => {
            "Another installation is already in progress. Complete that installation before proceeding with this install."
        }
        1925 => {
            "You do not have sufficient privileges to complete this installation for all users of the machine. Log on as administrator and then retry this installation."
        }
        _ => return,
    };
    message_box(
        text,
        if product.is_empty() {
            "Doors Installer"
        } else {
            product
        },
        0x10,
    );
}

fn now_seconds() -> u32 {
    (door_sys::wall_time().unwrap_or(0) / 10_000_000) as u32
}

/// The package's SHA-256 in lower-case hex.
fn package_hash(package: &str) -> Option<String> {
    use sha2::Digest;
    let bytes = load(package).ok()?;
    Some(
        sha2::Sha256::digest(&bytes)
            .iter()
            .map(|b| alloc::format!("{b:02x}"))
            .collect(),
    )
}
/// The consent token the window server gives the elevated msiexec.exe
/// (`window::elevation::PACKAGE`).
fn hex_digest(parts: &[&[u8]]) -> String {
    use sha2::Digest;
    let mut d = sha2::Sha256::new();
    for part in parts {
        d.update(part);
    }
    d.finalize()
        .iter()
        .map(|b| alloc::format!("{b:02x}"))
        .collect()
}
/// What the elevation request carries for the package's hash and the
/// command line (`window::elevation::PACKAGE`): their digest.
fn binding(hash: &str, line: &str) -> String {
    hex_digest(&[hash.as_bytes(), &[0], line.as_bytes()])
}
/// The consent token the display service gives the elevated msiexec.exe.
fn consent_token(package: &str, hash: &str, line: &str) -> String {
    hex_digest(&[
        door_service_protocol::window::elevation::CONSENT_TAG,
        package.as_bytes(),
        &[0],
        binding(hash, line).as_bytes(),
    ])
}
/// Hand the request to an elevated msiexec.exe: the consent prompt for
/// msiexec.exe, bound to this command line and this package's bytes; the
/// elevated copy picks the hand-off up only if it matches what the user
/// approved.
fn elevate(line: &str, ui: u32, package: &str) -> u32 {
    let mut t = Machine::new();
    let s = |v: &str| RegValue::String(String::from(v));
    let Some(hash) = package_hash(package) else {
        return code::PACKAGE_OPEN_FAILED;
    };
    if t.set_value(Root::CurrentUser, HANDOFF, Some("CommandLine"), &s(line))
        .is_err()
        || t.set_value(Root::CurrentUser, HANDOFF, Some("Package"), &s(package))
            .is_err()
        || t.set_value(Root::CurrentUser, HANDOFF, Some("PackageHash"), &s(&hash))
            .is_err()
        || t.set_value(
            Root::CurrentUser,
            HANDOFF,
            Some("Time"),
            &RegValue::Dword(now_seconds()),
        )
        .is_err()
        || t.set_value(
            Root::CurrentUser,
            HANDOFF,
            Some("UILevel"),
            &RegValue::Dword(ui),
        )
        .is_err()
    {
        return code::INSUFFICIENT_PRIVILEGES;
    }
    let mut module = [0u16; 520];
    let n = unsafe { DoorGetModuleFileNameW(0, module.as_mut_ptr() as u64, module.len() as u32) }
        as usize;
    let windows_path = String::from_utf16_lossy(&module[..n.min(module.len())]);
    let Ok(image) = t.read_file(&windows_path) else {
        let _ = door_sys::write(
            alloc::format!("DOOR_P16 MSI ELEVATE image unreadable path={windows_path}\n")
                .as_bytes(),
        );
        return code::INSUFFICIENT_PRIVILEGES;
    };
    let w = utf16z(&windows_path);
    let mut native = [0u8; 600];
    let len = unsafe {
        DoorGetNativePath(
            w.as_ptr() as u64,
            native.as_mut_ptr() as u64,
            native.len() as u32,
        )
    } as usize;
    if len == 0 || len > native.len() {
        return code::INSUFFICIENT_PRIVILEGES;
    }
    // The prompt names the package, as Windows' consent does for an .msi:
    // `program NUL package NUL binding` with the PACKAGE flag.
    let mut request = Vec::from(&native[..len]);
    for part in [package, &binding(&hash, line)] {
        request.push(0);
        request.extend_from_slice(part.as_bytes());
    }
    if request.len() > door_service_protocol::window::elevation::MAX_PATH {
        let _ = door_sys::write(
            alloc::format!(
                "DOOR_P16 MSI ELEVATE request too long bytes={}\n",
                request.len()
            )
            .as_bytes(),
        );
        return code::INVALID_COMMAND_LINE;
    }
    let _ = door_sys::write(b"DOOR_P16 MSI ELEVATE handoff\n");
    match unsafe {
        DoorRequestElevation(
            request.as_ptr() as u64,
            request.len() as u32,
            image.as_ptr() as u64,
            image.len() as u32,
            door_service_protocol::window::elevation::PACKAGE,
        )
    } {
        0 => (),
        1223 => return code::INSTALL_USEREXIT,
        error => {
            let _ = door_sys::write(
                alloc::format!("DOOR_P16 MSI ELEVATE request failed code={error}\n").as_bytes(),
            );
            return code::INSUFFICIENT_PRIVILEGES;
        }
    }
    // The user's answer, then the elevated installation's own result, as
    // Windows' msiexec.exe returns it to its caller.
    let mut process = [0u8; 8];
    match unsafe { DoorElevationResult(process.as_mut_ptr() as u64) } {
        0 => (),
        1223 => return code::INSTALL_USEREXIT,
        error => {
            let _ = door_sys::write(
                alloc::format!("DOOR_P16 MSI ELEVATE result failed code={error}\n").as_bytes(),
            );
            return code::INSUFFICIENT_PRIVILEGES;
        }
    }
    let process = u64::from_le_bytes(process);
    let mut exit = code::INSTALL_FAILURE;
    unsafe {
        DoorWaitForSingleObjectEx(process, u32::MAX, 0);
        if DoorGetExitCodeProcess(process, &mut exit as *mut u32 as u64) == 0 {
            exit = code::INSTALL_FAILURE;
        }
        DoorCloseHandle(process);
    }
    exit
}

/// What a non-elevated msiexec.exe handed over and the user approved: the
/// command line, the package and its SHA-256.
struct Handoff {
    line: String,
    package: String,
    hash: [u8; 32],
}
/// The request handed over by a non-elevated msiexec.exe in the last two
/// minutes (consumed), if it is the one the consent `token` names.
fn take_handoff(token: &str) -> Option<(Handoff, u32)> {
    let mut t = Machine::new();
    let text = |t: &mut Machine, name: &str| match t.value(Root::CurrentUser, HANDOFF, name) {
        Some(RegValue::String(s)) => Some(s),
        _ => None,
    };
    let line = text(&mut t, "CommandLine");
    let package = text(&mut t, "Package");
    let hex = text(&mut t, "PackageHash");
    let (Some(line), Some(package), Some(hex)) = (line, package, hex) else {
        t.delete_key(Root::CurrentUser, HANDOFF);
        return None;
    };
    // The command line is bound by the token, not shown to the user: one
    // line of text, no control characters.
    if consent_token(&package, &hex, &line) != token
        || hex.len() != 64
        || line.is_empty()
        || line.chars().any(char::is_control)
    {
        t.delete_key(Root::CurrentUser, HANDOFF);
        let _ = door_sys::write(b"DOOR_P16 MSI ELEVATE handoff refused\n");
        return None;
    }
    let mut hash = [0u8; 32];
    for (i, b) in hash.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    let time = match t.value(Root::CurrentUser, HANDOFF, "Time") {
        Some(RegValue::Dword(d)) => d,
        _ => 0,
    };
    let ui = match t.value(Root::CurrentUser, HANDOFF, "UILevel") {
        Some(RegValue::Dword(d)) => d,
        _ => level::BASIC,
    };
    t.delete_key(Root::CurrentUser, HANDOFF);
    (now_seconds().saturating_sub(time) <= 120).then_some((
        Handoff {
            line,
            package,
            hash,
        },
        ui,
    ))
}

/// The arguments after the program name.
fn arguments(line: &str) -> String {
    let line = line.trim_start();
    let rest = if let Some(r) = line.strip_prefix('"') {
        r.find('"').map_or("", |i| &r[i + 1..])
    } else {
        line.find([' ', '\t']).map_or("", |i| &line[i..])
    };
    String::from(rest.trim())
}

/// msiexec.exe's main: Windows Installer's exit code.
#[unsafe(no_mangle)]
pub extern "system" fn DoorMsiexecMain(command_line: u64) -> u32 {
    let full = wide(command_line).ok().flatten().unwrap_or_default();
    let mut line = arguments(&full);
    let mut handed: Option<Handoff> = None;
    if line.is_empty() {
        message_box(HELP, "Doors Installer", 0x40);
        return code::INVALID_COMMAND_LINE;
    }
    // Started elevated after the user approved a package: the hand-off the
    // consent token names, nothing else.
    if let Some(token) = line
        .strip_prefix(door_service_protocol::window::elevation::CONSENT_SWITCH)
        .map(String::from)
    {
        match take_handoff(&token) {
            Some((h, _)) => {
                line = h.line.clone();
                handed = Some(h);
            }
            None => return code::BAD_PACKAGE,
        }
    }
    let approved = handed.as_ref().map(|h| (h.package.clone(), h.hash));
    let handed = handed.is_some();
    // A type library of this bitness for an installation of the other
    // (`/DoorsTypeLib r|u m|u "path" "help"`): its HRESULT.
    if !handed && let Some(rest) = line.strip_prefix(super::target::TYPELIB_SWITCH) {
        let words = split_msiexec_line(rest);
        if words.len() != 4 {
            return code::INVALID_COMMAND_LINE;
        }
        let register = words[0] == "r";
        let per_user = words[1] == "u";
        let view32 = cfg!(target_arch = "x86");
        return Machine::new()
            .register_typelib(&words[2], &words[3], per_user, view32, register)
            .err()
            .unwrap_or(0);
    }
    let words = split_msiexec_line(&line);
    let a: CommandLine = match parse_command_line(&words) {
        Ok(a) => a,
        Err(e) => {
            message_box(HELP, "Doors Installer", 0x40);
            return e;
        }
    };
    if a.help {
        message_box(HELP, "Doors Installer", 0x40);
        return code::SUCCESS;
    }
    // /y and /z: a module's DllRegisterServer or DllUnregisterServer; the
    // exit code is its HRESULT (0 when it succeeded). Not with a consent
    // token: an approved hand-off installs its package and nothing else.
    if let Some((register, module)) = &a.self_reg {
        if handed {
            return code::INVALID_COMMAND_LINE;
        }
        let path = absolute(module);
        let w = utf16z(&path);
        let handle = unsafe { DoorLoadLibraryW(w.as_ptr() as u64) };
        let result = if handle == 0 {
            Err(0x8007_0000 | unsafe { RtlGetLastWin32Error() })
        } else {
            let r = super::target::call_registration(handle, *register);
            unsafe { DoorFreeLibrary(handle) };
            r
        };
        let status = result.err().unwrap_or(0);
        if status != 0 && a.ui > level::NONE {
            message_box(
                &alloc::format!(
                    "{path}: {} failed with {status:#010x}.",
                    if *register {
                        "DllRegisterServer"
                    } else {
                        "DllUnregisterServer"
                    }
                ),
                "Doors Installer",
                0x10,
            );
        }
        return status;
    }
    let mut properties = a.properties.clone();
    if handed && a.ui >= level::REDUCED {
        properties.push((String::from("DOORS_UI_HANDOFF"), String::from("1")));
    }
    let log = a.log.as_ref().map(|l| absolute(l));
    let (package, request) = if let Some(p) = &a.install {
        if p.starts_with('{') {
            // /i {ProductCode}: maintenance of the installed product.
            match cached(p) {
                Ok(c) => (c, Request::Install),
                Err(e) => {
                    report(e, a.ui, "");
                    return e;
                }
            }
        } else {
            (absolute(p), Request::Install)
        }
    } else if let Some(p) = &a.remove {
        if p.starts_with('{') {
            match cached(p) {
                Ok(c) => (c, Request::Remove),
                Err(e) => {
                    report(e, a.ui, "");
                    return e;
                }
            }
        } else {
            (absolute(p), Request::Remove)
        }
    } else if let Some((mode, p)) = &a.repair {
        let path = if p.starts_with('{') {
            match cached(p) {
                Ok(c) => c,
                Err(e) => {
                    report(e, a.ui, "");
                    return e;
                }
            }
        } else {
            absolute(p)
        };
        (path, Request::Repair(mode.clone()))
    } else {
        return code::INVALID_COMMAND_LINE;
    };
    // A package of the other bitness: that msiexec.exe runs the same
    // command line (an approved, elevated hand-off runs here).
    if !handed && let Some(program) = foreign_engine(&package) {
        return run_engine(program, &line);
    }
    // msiexec /x with its full UI asks first, then removes with the basic
    // UI, as Windows Installer does.
    let mut ui_level = a.ui;
    if matches!(request, Request::Remove) && a.ui == level::FULL && !handed {
        let answer = message_box(
            "Are you sure you want to uninstall this product?",
            "Doors Installer",
            0x4 | 0x20,
        );
        if answer != 6 {
            return code::INSTALL_USEREXIT;
        }
        ui_level = level::BASIC;
    }
    // The approved package only, with the bytes the user approved.
    if let Some((approved_package, _)) = &approved
        && !approved_package.eq_ignore_ascii_case(&package)
    {
        let _ = door_sys::write(b"DOOR_P16 MSI ELEVATE package mismatch\n");
        return code::BAD_PACKAGE;
    }
    let (status, public) = run_request_with(
        &package,
        approved.as_ref().map(|(_, h)| h),
        request,
        &properties,
        ui_level,
        a.passive,
        log,
    );
    if status == code::ELEVATION_REQUIRED {
        // The rest runs elevated: the same request, with the answers the
        // user gave in the dialogs (the UI sequence ran here) as its
        // properties.
        let mut handoff = line.clone();
        if ui_level != a.ui {
            // The confirmed removal goes on with the basic UI.
            handoff.push_str(" /qb");
        }
        for (k, v) in public {
            if a.properties.iter().any(|p| p.0 == k) {
                continue;
            }
            // msiexec's quoting: a quote inside a value is doubled.
            handoff.push_str(&alloc::format!(" {k}=\"{}\"", v.replace('"', "\"\"")));
        }
        return elevate(&handoff, ui_level, &package);
    }
    let product = door_msi::Database::open(&load(&package).unwrap_or_default())
        .ok()
        .and_then(|db| db.property("ProductName"))
        .unwrap_or_default();
    report(status, a.ui, &product);
    status
}
