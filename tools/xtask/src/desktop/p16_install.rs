//! P16 INSTALL-A: Windows Installer and EXE installers on Doors.
//!
//! The packages are door-msi's (written by `door_msi::write`, no WiX):
//! Doors Tools 1.0 and 2.0 (a major upgrade), Doors Companion (it shares a
//! component with Doors Tools) and an oversized package. Their program is
//! `toolprobe.exe` and their DLL custom action `caprobe.dll` (both built
//! here with the pinned MSVC toolchain and SDK, no C runtime);
//! `msiprobe.exe` drives msi.dll's API; `setupprobe.exe` is an EXE
//! installer whose manifest asks for administrator rights. All of them
//! reach the guest through the image's system share and Doorsh copies them
//! into the user's folder.
//!
//! Boot 1: as a standard token msiexec refuses the per-machine package
//! (1925) and msiprobe reads the package through msi.dll's database API;
//! an elevated batch file (Run as administrator) then installs through
//! MsiInstallProduct and removes through MsiConfigureProductEx, fails an
//! installation on purpose (everything rolled back, the deferred custom
//! action's marker removed by its rollback action), runs out of disk space
//! (nothing copied), installs Doors Tools 1.0 (the DLL custom action sets a
//! property the registry receives), Doors Companion, upgrades to 2.0 (the
//! user's own file in the program folder kept), refuses a downgrade,
//! repairs a deleted file and removes both products (the shared component
//! leaves with the last of them), installs a package whose service starts
//! with it and goes with it (ServiceInstall, ServiceControl), and a
//! suppressed restart (3010). Then File Explorer opens tools-1.0.msi: once
//! cancelled (the package's cancel dialog, then 1602), then installed:
//! the package's own dialogs (license, Install with the shield), User
//! Account Control for msiexec.exe, the elevated progress and completion
//! dialogs; File Explorer opens setupprobe.exe (User Account Control from
//! its manifest); Settings > Apps > Installed apps lists both and
//! uninstalls them (the EXE's uninstaller through User Account Control,
//! the package through msiexec /x). Last, an installation is cut off by a
//! power loss while its deferred custom action runs.
//!
//! Boot 2 (same disk): the next Windows Installer session rolls the
//! interrupted installation back before its own.
use super::*;
use std::path::Path;

const RESOLUTION: (u32, u32) = (1280, 720);
const DIRECTORY: &str = "tests/fixtures/windows-install";
const TOOLS_1: &str = door_msi::fixtures::TOOLS_1;
const TOOLS_2: &str = door_msi::fixtures::TOOLS_2;
const COMPANION: &str = door_msi::fixtures::COMPANION;
const REGISTRATIONS: &str = door_msi::fixtures::REGISTRATIONS;
const REG_CLSID: &str = door_msi::fixtures::REG_CLSID;
/// Settings > Apps > Installed apps.
const INSTALLED_APPS: usize = 14;
/// Settings maximized at 1280 x 720 (the taskbar takes 48).
const MAXIMIZED: (u32, u32) = (1280, 672);

/// A fixture: its name, SDK libraries, imported modules, link options and
/// whether it is a DLL.
struct Fixture {
    directory: &'static str,
    name: &'static str,
    libraries: &'static [&'static str],
    modules: &'static [&'static str],
    dll: bool,
    manifest: bool,
}
const FIXTURES: [Fixture; 5] = [
    // The service a package installs and starts (SCM-A's service program).
    Fixture {
        name: "doorsvc",
        directory: "tests/fixtures/windows-services",
        libraries: &["kernel32.lib", "AdvAPI32.Lib"],
        modules: &["advapi32.dll", "kernel32.dll"],
        dll: false,
        manifest: false,
    },
    Fixture {
        name: "toolprobe",
        directory: DIRECTORY,
        libraries: &["kernel32.lib"],
        modules: &["kernel32.dll"],
        dll: false,
        manifest: false,
    },
    Fixture {
        name: "caprobe",
        directory: DIRECTORY,
        libraries: &["kernel32.lib", "Msi.Lib"],
        modules: &["msi.dll"],
        dll: true,
        manifest: false,
    },
    Fixture {
        name: "msiprobe",
        directory: DIRECTORY,
        libraries: &["kernel32.lib", "Msi.Lib"],
        modules: &["kernel32.dll", "msi.dll"],
        dll: false,
        manifest: false,
    },
    Fixture {
        name: "setupprobe",
        directory: DIRECTORY,
        libraries: &["kernel32.lib", "AdvAPI32.Lib"],
        modules: &["advapi32.dll", "kernel32.dll"],
        dll: false,
        manifest: true,
    },
];

/// A compiled resource file holding an application manifest that asks for
/// administrator rights (RT_MANIFEST 1), for the linker.
fn manifest_res() -> Vec<u8> {
    let xml = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0"><trustInfo xmlns="urn:schemas-microsoft-com:asm.v3"><security><requestedPrivileges><requestedExecutionLevel level="requireAdministrator" uiAccess="false"></requestedExecutionLevel></requestedPrivileges></security></trustInfo></assembly>"#;
    let mut out = Vec::new();
    let header = |out: &mut Vec<u8>, size: u32, kind: u16, name: u16, flags: u16, language: u16| {
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&32u32.to_le_bytes());
        out.extend_from_slice(&[0xff, 0xff]);
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&[0xff, 0xff]);
        out.extend_from_slice(&name.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&language.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
    };
    header(&mut out, 0, 0, 0, 0, 0);
    header(&mut out, xml.len() as u32, 24, 1, 0x1030, 0x0409);
    out.extend_from_slice(xml);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    out
}

/// Compile and link one fixture; its record and bytes.
fn build_fixture(
    run: &mut Run,
    target: &Path,
    fixture: &Fixture,
) -> Result<(Value, Vec<u8>), String> {
    use crate::windows::native_path;
    let profile = crate::windows::build::toolchain(&run.root)?;
    let msvc = profile["msvc_root"].as_str().ok_or("MSVC root")?.to_owned();
    let sdk = profile["sdk_root"].as_str().ok_or("SDK root")?.to_owned();
    let version = profile["sdk_version"]
        .as_str()
        .ok_or("SDK version")?
        .to_owned();
    let libraries = format!("{sdk}/Lib/{version}/um/x64");
    for library in fixture.libraries {
        let pinned = profile["files"]
            .as_array()
            .ok_or("toolchain files")?
            .iter()
            .any(|f| f["path"].as_str() == Some(&format!("{libraries}/{library}")));
        if !pinned {
            return Err(format!("Windows toolchain does not pin {library}"));
        }
    }
    let name = fixture.name;
    let source = format!("{}/{name}.c", fixture.directory);
    let object = target.join(format!("{name}.obj"));
    let output = target.join(format!(
        "{name}.{}",
        if fixture.dll { "dll" } else { "exe" }
    ));
    let map = target.join(format!("{name}.map"));
    let mut compile: Vec<String> = [
        "/nologo", "/c", "/TC", "/O1", "/GS-", "/Zl", "/W4", "/WX", "/X", "/Brepro",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    compile.push(format!("/I{msvc}/include"));
    for directory in ["um", "shared", "ucrt"] {
        compile.push(format!("/I{sdk}/Include/{version}/{directory}"));
    }
    compile.push(format!("/Fo{}", native_path(&object)?));
    compile.push(native_path(&run.root.join(&source))?);
    run.required(
        &format!("{msvc}/bin/Hostx64/x64/cl.exe"),
        &compile.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    let mut link: Vec<String> = [
        "/NOLOGO",
        "/NODEFAULTLIB",
        "/MACHINE:X64",
        "/DYNAMICBASE",
        "/NXCOMPAT",
        "/Brepro",
        "/DEBUG:NONE",
        "/INCREMENTAL:NO",
        "/MANIFEST:NO",
        "/OPT:REF",
        "/OPT:ICF",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    if fixture.dll {
        link.extend([
            "/DLL".into(),
            "/NOENTRY".into(),
            "/EXPORT:CheckMachine".into(),
            "/BASE:0x180800000".into(),
        ]);
    } else {
        link.extend([
            "/ENTRY:entry".into(),
            "/SUBSYSTEM:CONSOLE,6.02".into(),
            "/BASE:0x140000000".into(),
        ]);
    }
    if fixture.manifest {
        let res = target.join(format!("{name}.res"));
        fs::write(&res, manifest_res()).map_err(|e| e.to_string())?;
        link.push(native_path(&res)?);
    }
    link.push(format!("/OUT:{}", native_path(&output)?));
    link.push(format!("/MAP:{}", native_path(&map)?));
    link.push(native_path(&object)?);
    for library in fixture.libraries {
        link.push(format!("{libraries}/{library}"));
    }
    run.required(
        &format!("{msvc}/bin/Hostx64/x64/link.exe"),
        &link.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    let pe = crate::windows::pe::inspect(&output)?;
    let mut modules: Vec<String> = pe["imports"]
        .as_array()
        .ok_or("fixture imports")?
        .iter()
        .filter_map(|m| m["module"].as_str().map(str::to_ascii_lowercase))
        .collect();
    modules.sort();
    if modules != fixture.modules {
        return Err(format!(
            "{name} imports {modules:?}; expected {:?}",
            fixture.modules
        ));
    }
    let bytes = fs::read(&output).map_err(|e| e.to_string())?;
    if fixture.manifest {
        let image =
            door_image_format::PeImage::parse(&bytes).map_err(|e| format!("{name}: {e:?}"))?;
        let level = image.manifest().map(door_image_format::execution_level);
        if level != Some(door_image_format::ExecutionLevel::RequireAdministrator) {
            return Err(format!("{name}'s manifest reads as {level:?}"));
        }
    }
    Ok((
        json!({"name":name,"source":source,"source_sha256":file_hash(&run.root.join(&source))?,
        "path":relative(&run.root,&output)?,"sha256":file_hash(&output)?,"imports":pe["imports"],
        "runtime":"Windows SDK C; no CRT"}),
        bytes,
    ))
}

/// The elevated batch files the scenario runs (Run as administrator).
fn batch_install() -> String {
    let lines = [
        "@echo off".to_string(),
        r#"cd /d "%USERPROFILE%\p16install""#.into(),
        // A failure part-way: everything is rolled back.
        "msiexec /i tools-1.0.msi /qn FAIL_AT=files".into(),
        "echo failed %errorlevel%".into(),
        r#"if exist "C:\Program Files\Doors Tools" (echo leftover) else (echo clean)"#.into(),
        r#"if exist "C:\ProgramData\Doors Tools Marker.txt" (echo marker left) else (echo marker gone)"#.into(),
        // msi.dll's API: install and remove.
        "msiprobe install tools-1.0.msi".into(),
        "msiprobe remove".into(),
        // Not enough space: nothing is copied.
        "msiexec /i oversized.msi /qn".into(),
        "echo oversized %errorlevel%".into(),
        r#"if exist "C:\Program Files\Doors Oversized" (echo oversized left) else (echo oversized clean)"#.into(),
        // Installation, with the DLL custom action's property.
        "msiexec /i tools-1.0.msi /qn /l*v install.log".into(),
        "echo install %errorlevel%".into(),
        r#"reg query "HKLM\Software\Macrohard Test\Doors Tools" /v CaCheck"#.into(),
        r#"reg query "HKLM\Software\Macrohard Test\Doors Tools" /v Version"#.into(),
        r#"type "C:\ProgramData\Doors Tools Marker.txt""#.into(),
        "echo.".into(),
        r"reg query HKLM\Software\Classes\.doorsnote".into(),
        r#"dir /b "C:\ProgramData\Microsoft\Windows\Start Menu\Programs""#.into(),
        // The installed product runs.
        r#""C:\Program Files\Doors Tools\toolprobe.exe""#.into(),
        "echo run %errorlevel%".into(),
        "msiexec /i companion.msi /qn".into(),
        "echo companion %errorlevel%".into(),
        // The user's own file in the program folder.
        r#"echo mine> "C:\Program Files\Doors Tools\my notes.txt""#.into(),
        // The major upgrade, then a refused downgrade.
        "msiexec /i tools-2.0.msi /qn".into(),
        "echo upgrade %errorlevel%".into(),
        r#"type "C:\Program Files\Doors Tools\readme.txt""#.into(),
        format!(r"reg query HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\{TOOLS_1} /v DisplayName"),
        "echo old entry %errorlevel%".into(),
        "msiexec /i tools-1.0.msi /qn".into(),
        "echo downgrade %errorlevel%".into(),
        // Repair restores a deleted file.
        r#"del "C:\Program Files\Doors Tools\toolprobe.exe""#.into(),
        format!("msiexec /fomus {TOOLS_2} /qn"),
        "echo repair %errorlevel%".into(),
        r#"if exist "C:\Program Files\Doors Tools\toolprobe.exe" (echo repaired) else (echo missing)"#.into(),
        // Removal: the shared component stays with the companion.
        format!("msiexec /x {TOOLS_2} /qn"),
        "echo remove %errorlevel%".into(),
        r#"if exist "C:\Program Files\Common Files\Doors\doors-shared.txt" (echo shared kept) else (echo shared lost)"#.into(),
        r#"type "C:\Program Files\Doors Tools\my notes.txt""#.into(),
        format!("msiexec /x {COMPANION} /qn"),
        "echo companion removed %errorlevel%".into(),
        r#"if exist "C:\Program Files\Common Files\Doors\doors-shared.txt" (echo shared left) else (echo shared removed)"#.into(),
        r#"del "C:\Program Files\Doors Tools\my notes.txt""#.into(),
        r#"rd "C:\Program Files\Doors Tools""#.into(),
        // A package with a service: installed and started, then stopped
        // and deleted with the product.
        "msiexec /i tools-service.msi /qn".into(),
        "echo service install %errorlevel%".into(),
        "sc query DoorsToolsSvc".into(),
        format!("msiexec /x {TOOLS_1} /qn"),
        "echo service remove %errorlevel%".into(),
        "sc query DoorsToolsSvc".into(),
        "echo service gone %errorlevel%".into(),
        // A restart the package schedules, suppressed: 3010.
        "msiexec /i tools-1.0.msi /qn NEEDREBOOT=1 /norestart".into(),
        "echo reboot %errorlevel%".into(),
        format!("msiexec /x {TOOLS_1} /qn"),
        "echo reboot removed %errorlevel%".into(),
        // A 32-bit package: SysWOW64's msiexec installs it, its registry in
        // the 32-bit view (WOW6432Node); a COM class, a font, environment
        // variables, an INI file, a duplicated file; all gone on removal.
        "msiexec /i registrations.msi /qn".into(),
        "echo registrations %errorlevel%".into(),
        format!(r#"reg query "HKLM\SOFTWARE\Classes\WOW6432Node\CLSID\{REG_CLSID}\LocalServer32""#),
        format!(r#"reg query "HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\{REGISTRATIONS}" /v DisplayName"#),
        r#"reg query "HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts" /v "Doors Registration Sans (TrueType)""#.into(),
        r#"reg query "HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment" /v DOORSREG_HOME"#.into(),
        r#"type "C:\Program Files (x86)\Doors Registrations\doorsreg.ini""#.into(),
        r#"type "C:\Program Files (x86)\Doors Registrations\Data\readme-copy.txt""#.into(),
        format!("msiexec /x {REGISTRATIONS} /qn"),
        "echo registrations removed %errorlevel%".into(),
        format!(r#"reg query "HKLM\SOFTWARE\Classes\WOW6432Node\CLSID\{REG_CLSID}""#),
        "echo class gone %errorlevel%".into(),
        r#"reg query "HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment" /v DOORSREG_HOME"#.into(),
        "echo environment gone %errorlevel%".into(),
        "echo install-a done".into(),
    ];
    lines.join("\r\n") + "\r\n"
}
fn batch_hang() -> String {
    [
        "@echo off",
        r#"cd /d "%USERPROFILE%\p16install""#,
        "msiexec /i tools-1.0.msi /qn HANG=1",
        "echo hang returned %errorlevel%",
    ]
    .join("\r\n")
        + "\r\n"
}
fn batch_after() -> String {
    [
        "@echo off".to_string(),
        r#"cd /d "%USERPROFILE%\p16install""#.into(),
        "msiexec /i companion.msi /qn".into(),
        "echo after %errorlevel%".into(),
        r#"if exist "C:\Program Files\Doors Tools\toolprobe.exe" (echo tools left) else (echo tools gone)"#.into(),
        r#"if exist "C:\ProgramData\Doors Tools Marker.txt" (echo marker left) else (echo marker gone)"#.into(),
        r"reg query HKLM\Software\Microsoft\Windows\CurrentVersion\Installer\InProgress".into(),
        "echo in progress %errorlevel%".into(),
        format!("msiexec /x {COMPANION} /qn"),
        "echo after removed %errorlevel%".into(),
        "echo after done".into(),
    ]
    .join("\r\n")
        + "\r\n"
}

/// The files the image's system share carries for the scenario.
#[allow(clippy::type_complexity)]
fn payload(run: &mut Run, target: &Path) -> Result<(Vec<Value>, Vec<(String, Vec<u8>)>), String> {
    let mut records = Vec::new();
    let mut built: Vec<(&str, Vec<u8>)> = Vec::new();
    for fixture in &FIXTURES {
        let (record, bytes) = build_fixture(run, target, fixture)?;
        records.push(record);
        built.push((fixture.name, bytes));
    }
    let get = |n: &str| {
        built
            .iter()
            .find(|b| b.0 == n)
            .map(|b| b.1.clone())
            .unwrap()
    };
    let tools = door_msi::fixtures::Payload {
        tool: get("toolprobe"),
        service: None,
        custom_action_dll: Some(get("caprobe")),
    };
    // Doors Tools 1.0 with its service (ServiceInstall, ServiceControl:
    // started on install, stopped and deleted on removal).
    let with_service = door_msi::fixtures::Payload {
        service: Some(get("doorsvc")),
        ..tools.clone()
    };
    let files = vec![
        (
            "tools-1.0.msi".to_string(),
            door_msi::fixtures::tools(1, &tools),
        ),
        (
            "tools-2.0.msi".to_string(),
            door_msi::fixtures::tools(2, &tools),
        ),
        (
            "tools-service.msi".to_string(),
            door_msi::fixtures::tools(1, &with_service),
        ),
        ("companion.msi".to_string(), door_msi::fixtures::companion()),
        ("oversized.msi".to_string(), door_msi::fixtures::oversized()),
        (
            "registrations.msi".to_string(),
            door_msi::fixtures::registrations(),
        ),
        ("msiprobe.exe".to_string(), get("msiprobe")),
        ("setupprobe.exe".to_string(), get("setupprobe")),
        ("a-install.bat".to_string(), batch_install().into_bytes()),
        ("b-hang.bat".to_string(), batch_hang().into_bytes()),
        ("c-after.bat".to_string(), batch_after().into_bytes()),
    ];
    Ok((records, files))
}

/// The desktop image (msi.dll and msiexec.exe required) with the payload in
/// /system/share.
fn stage(
    run: &mut Run,
    image: &str,
    target: &Path,
    files: &[(String, Vec<u8>)],
) -> Result<String, String> {
    let (base, _) = crate::image::validate_staged(&run.root, image)?;
    let production = fs::read(base.join("esp/Door/initfs.tar")).map_err(|e| e.to_string())?;
    let archive =
        door_initfs::Archive::parse(&production).map_err(|e| format!("desktop initfs {e:?}"))?;
    for required in ["windows/msi.dll", "seed/msiexec.exe"] {
        if archive.find(required).is_none() {
            return Err(format!(
                "the desktop image does not ship {required}; rebuild it"
            ));
        }
    }
    let mut manifest = String::from_utf8(
        archive
            .find("system/install.manifest")
            .ok_or("desktop initfs lacks system/install.manifest")?
            .data
            .to_vec(),
    )
    .map_err(|e| e.to_string())?;
    if !manifest.ends_with('\n') {
        manifest.push('\n');
    }
    let mut edits: Vec<(String, Vec<u8>)> = Vec::new();
    for (name, bytes) in files {
        manifest.push_str(&format!("/system/share/{name} seed/install-a/{name}\n"));
        edits.push((format!("seed/install-a/{name}"), bytes.clone()));
    }
    edits.push(("system/install.manifest".into(), manifest.into_bytes()));
    let edits: Vec<(&str, &[u8])> = edits
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    let initfs = target.join("initfs-install.tar");
    fs::write(
        &initfs,
        crate::initfs::with_files(&production, &edits, &["system/install.manifest"])?,
    )
    .map_err(|e| e.to_string())?;
    session::stage_desktop_variant(
        run,
        &base.join("esp/EFI/BOOT/BOOTX64.EFI"),
        &base.join("esp/Door/kernel.elf"),
        &initfs,
        "install",
    )
}

/// The serial log after `before` has a line containing `needle` within
/// `seconds`.
fn wait_line(
    run: &Run,
    record: &Value,
    before: usize,
    needle: &str,
    seconds: u64,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let text = serial(&run.root, record)?;
        if let Some(line) = text
            .get(before.min(text.len())..)
            .unwrap_or("")
            .lines()
            .find(|l| l.contains(needle))
        {
            return Ok(line.to_owned());
        }
        if text.contains("KERNEL_PANIC") {
            return Err("guest kernel panic; inspect serial".into());
        }
        if Instant::now() >= deadline {
            return Err(format!("{needle} missing"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}
/// What appeared in the console after `before`: cmd's own TEXT lines, then
/// the lines console programs wrote (`DOOR_CONSOLE` records).
fn text_lines(run: &Run, record: &Value, before: usize) -> Result<Vec<String>, String> {
    let text = serial(&run.root, record)?;
    let tail = text.get(before.min(text.len())..).unwrap_or("");
    let mut lines: Vec<String> = tail
        .lines()
        .filter_map(|l| {
            l.split_once("DOOR_P16 CMD TEXT ")
                .map(|(_, t)| t.trim_end().to_owned())
        })
        .collect();
    let mut console = String::new();
    for line in tail.lines() {
        if let Some(quoted) = line
            .strip_prefix("DOOR_CONSOLE ")
            .and_then(|l| l.split_once(" text="))
            .map(|x| x.1)
            && let Ok(t) = serde_json::from_str::<String>(quoted)
        {
            console.push_str(&t);
        }
    }
    lines.extend(
        console
            .lines()
            .map(|l| l.trim_end().to_owned())
            .filter(|l| !l.is_empty()),
    );
    Ok(lines)
}
/// One command line at a cmd prompt: its TEXT lines.
fn command(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    line: &str,
) -> Result<Vec<String>, String> {
    let before = serial(&run.root, record)?.len();
    type_line(c, line)?;
    wait_line(run, record, before, "DOOR_P16 CMD IDLE", 120)?;
    let lines = text_lines(run, record, before)?;
    Ok(lines)
}
/// Each wanted line, in order: a line after the previous match that is it
/// or starts with it.
fn require(lines: &[String], want: &[&str], what: &str) -> Result<(), String> {
    let mut at = 0;
    for w in want {
        match lines[at..]
            .iter()
            .position(|l| l.trim() == *w || l.trim_start().starts_with(w))
        {
            Some(i) => at += i + 1,
            None => {
                return Err(format!(
                    "{what}: {w:?} missing after line {at} of {lines:?}"
                ));
            }
        }
    }
    Ok(())
}
/// File Explorer at ~\p16install, the batch file at `index` (sorted by
/// name) Run as administrator; User Account Control approved.
fn run_elevated(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    index: usize,
    name: &str,
) -> Result<usize, String> {
    let before = serial(&run.root, record)?.len();
    key(c, &["meta_l", "e"])?;
    resolution::wait_activation(&run.root, record, 1, before)?;
    let before = serial(&run.root, record)?.len();
    key(c, &["ctrl", "l"])?;
    thread::sleep(Duration::from_millis(300));
    type_line(c, "~\\p16install")?;
    wait_line(run, record, before, "DOOR_P12 FILES ADDRESS", 20)?;
    thread::sleep(Duration::from_millis(800));
    key(c, &["home"])?;
    for _ in 0..index {
        key(c, &["down"])?;
    }
    thread::sleep(Duration::from_millis(300));
    let before = serial(&run.root, record)?.len();
    key(c, &["shift", "f10"])?;
    thread::sleep(Duration::from_millis(600));
    key(c, &["down"])?;
    key(c, &["down"])?;
    key(c, &["ret"])?;
    let request = wait_line(run, record, before, "DOOR_P16 FILES RUNAS", 20)?;
    if !request.ends_with(&format!("/p16install/{name}")) {
        return Err(format!("Run as administrator on the wrong item: {request}"));
    }
    wait_line(run, record, before, "DOOR_P16 UAC PROMPT", 20)?;
    key(c, &["alt", "y"])?;
    wait_line(run, record, before, "DOOR_P16 ELEVATED SCRIPT", 20)?;
    Ok(before)
}
/// File Explorer's address bar opens `path` (a package or a program).
fn explorer_open(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    path: &str,
) -> Result<usize, String> {
    let before = serial(&run.root, record)?.len();
    key(c, &["meta_l", "e"])?;
    resolution::wait_activation(&run.root, record, 1, before)?;
    thread::sleep(Duration::from_secs(1));
    let before = serial(&run.root, record)?.len();
    key(c, &["ctrl", "l"])?;
    thread::sleep(Duration::from_millis(300));
    type_line(c, path)?;
    Ok(before)
}

/// Settings (maximized) > Installed apps: the first row's "..." >
/// Uninstall, then the flyout's Uninstall.
fn settings_uninstall(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    pointer: &mut (i32, i32),
    name: &str,
) -> Result<usize, String> {
    // "Doors ..." sort before the included apps: row 0.
    let (more, open) =
        door_workspace::settings_catalog::points::installed_app_open(0, MAXIMIZED.0, MAXIMIZED.1);
    let before = serial(&run.root, record)?.len();
    super::click(c, pointer, more.0, more.1)?;
    thread::sleep(Duration::from_millis(500));
    // Modify, Uninstall: the second entry.
    super::click(c, pointer, open.0, open.1 + 36)?;
    thread::sleep(Duration::from_millis(500));
    key(c, &["down"])?;
    key(c, &["ret"])?;
    wait_line(
        run,
        record,
        before,
        &format!("DOOR_P16 SETTINGS UNINSTALL app={name}"),
        20,
    )?;
    Ok(before)
}

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/install-a", run.id))?;
    let (fixtures, files) = payload(run, &target)?;
    let staged = stage(run, image, &target, &files)?;
    let disk = super::win32::fresh_disk(run, "install-a")?;
    let resolution = format!("{}x{}", RESOLUTION.0, RESOLUTION.1);
    let mut record = session::launch_desktop_accelerated(run, &staged, &disk, false, &resolution)?;
    let first: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        thread::sleep(Duration::from_secs(2));
        // Doorsh copies the payload into the user's folder.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l"])?;
        thread::sleep(Duration::from_millis(600));
        type_line(&mut c, "doorsh")?;
        wait_line(run, &record, before, "DOCUMENT_OPEN app=9", 20)?;
        // Terminal has the keyboard before the first line is typed (a key
        // typed before its window activates is lost).
        super::resolution::wait_activation(&run.root, &record, 9, before)?;
        thread::sleep(Duration::from_millis(2500));
        let before = serial(&run.root, &record)?.len();
        type_line(&mut c, "mkdir ~/p16install")?;
        let idle = wait_line(run, &record, before, "DOOR_P16 DSH IDLE", 30)?;
        if !idle.ends_with("status=0") {
            return Err(format!("Doorsh mkdir failed: {idle}"));
        }
        key(&mut c, &["alt", "f4"])?;
        thread::sleep(Duration::from_secs(1));
        // /system is outside the Windows namespace Doorsh and cmd use; File
        // Explorer copies the share (the payload and the image's own seeds)
        // into the user's folder.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "e"])?;
        resolution::wait_activation(&run.root, &record, 1, before)?;
        thread::sleep(Duration::from_secs(1));
        for (location, then) in [
            ("/system/share", ["ctrl", "c"]),
            ("~\\p16install", ["ctrl", "v"]),
        ] {
            let before = serial(&run.root, &record)?.len();
            key(&mut c, &["ctrl", "l"])?;
            thread::sleep(Duration::from_millis(300));
            type_line(&mut c, location)?;
            wait_line(run, &record, before, "DOOR_P12 FILES ADDRESS", 20)?;
            thread::sleep(Duration::from_millis(800));
            if then[1] == "c" {
                let before = serial(&run.root, &record)?.len();
                key(&mut c, &["ctrl", "a"])?;
                let selected = wait_line(run, &record, before, "FILES SELECT count=", 10)?;
                if !selected.contains(&format!("count={}", files.len() + 3)) {
                    return Err(format!("/system/share: {selected}"));
                }
            }
            let before = serial(&run.root, &record)?.len();
            key(&mut c, &then)?;
            if then[1] == "v" {
                let end = wait_line(run, &record, before, "DOOR_P14 FILEOP end", 120)?;
                if !end.contains("state=done") || !end.contains("failed=0") {
                    return Err(format!("copying the payload: {end}"));
                }
            }
            thread::sleep(Duration::from_millis(400));
        }
        key(&mut c, &["alt", "f4"])?;
        thread::sleep(Duration::from_secs(1));
        // cmd as the standard token.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(400));
        type_line(&mut c, "cmd")?;
        wait_line(run, &record, before, "DOCUMENT_OPEN app=9", 20)?;
        thread::sleep(Duration::from_millis(2500));
        command(run, &record, &mut c, "cd p16install")?;
        let api = command(run, &record, &mut c, "msiprobe api tools-1.0.msi")?;
        require(&api, &["INSTALL-A api PASSED"], "msi.dll database API")?;
        command(run, &record, &mut c, "msiexec /i tools-1.0.msi /qn")?;
        let plain = command(run, &record, &mut c, "echo plain %errorlevel%")?;
        require(&plain, &["plain 1925"], "a standard token")?;
        // Doors runs at most 13 processes and the session itself holds
        // nine: Terminal closes so msiexec and the programs its custom
        // actions start fit beside File Explorer and the elevated Terminal.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["alt", "f4"])?;
        wait_line(run, &record, before, "CLIENT_EXIT", 20)?;
        thread::sleep(Duration::from_secs(1));
        // The elevated batch: failures, rollback, API, install, upgrade,
        // repair, removal.
        let before = run_elevated(run, &record, &mut c, 0, "a-install.bat")?;
        wait_line(
            run,
            &record,
            before,
            "DOOR_P16 CMD TEXT install-a done",
            900,
        )?;
        let script = text_lines(run, &record, before)?;
        require(
            &script,
            &[
                "failed 1603",
                "clean",
                "marker gone",
                "oversized 1603",
                "oversized clean",
                "install 0",
                "CaCheck    REG_SZ    passed",
                "Version    REG_SZ    1.0.0",
                "installed",
                "(Default)    REG_SZ    DoorsTools.Note",
                "Doors Tools.lnk",
                "run 0",
                "companion 0",
                "upgrade 0",
                "Doors Tools 2.0",
                "old entry 1",
                "downgrade 1603",
                "repair 0",
                "repaired",
                "remove 0",
                "shared kept",
                "mine",
                "companion removed 0",
                "shared removed",
                "service install 0",
                "SERVICE_NAME: DoorsToolsSvc",
                "STATE              : 4  RUNNING",
                "service remove 0",
                "[SC] EnumQueryServicesStatus:OpenService FAILED 1060:",
                "service gone 1060",
                "reboot 3010",
                "reboot removed 0",
                "registrations 0",
                "(Default)    REG_SZ    C:\\Program Files (x86)\\Doors Registrations\\server.exe /automation",
                "DisplayName    REG_SZ    Doors Registrations",
                "Doors Registration Sans (TrueType)    REG_SZ    doorsreg.ttf",
                "DOORSREG_HOME    REG_SZ    C:\\Program Files (x86)\\Doors Registrations\\",
                "[Settings]",
                "Home=C:\\Program Files (x86)\\Doors Registrations\\",
                "Plugins=core",
                "Doors Registrations",
                "registrations removed 0",
                "class gone 1",
                "environment gone 1",
                "install-a done",
                // The MsiInstallProduct family's probes print after the
                // batch's own lines.
                "INSTALL-A msiapi PASSED",
                "INSTALL-A msiremove PASSED",
            ],
            "INSTALL-A elevated batch",
        )?;
        let serial_text = serial(&run.root, &record)?;
        let tail = serial_text.get(before..).unwrap_or("");
        for want in [
            "DOOR_P16 MSI Rollback",
            "Out of disk space -- Volume: 'C:'",
            "RemoveExistingProducts: removing {3D1C9A57-0001",
            "CaProbe: Doors Tools checked",
            // The 32-bit package went to the 32-bit engine.
            "DOOR_P17 MSI ENGINE C:\\Windows\\SysWOW64\\msiexec.exe",
        ] {
            if !tail.contains(want) {
                return Err(format!("INSTALL-A: serial lacks {want:?}"));
            }
        }
        key(&mut c, &["alt", "f4"])?;
        thread::sleep(Duration::from_secs(1));
        // The full UI, cancelled: Esc is the welcome dialog's Cancel, the
        // package's cancel dialog asks, Yes ends at its UserExit dialog
        // and msiexec exits 1602.
        let before = explorer_open(
            run,
            &record,
            &mut c,
            "C:\\Users\\1001\\p16install\\tools-1.0.msi",
        )?;
        wait_line(
            run,
            &record,
            before,
            "DOOR_P16 MSI DIALOG name=WelcomeEulaDlg",
            30,
        )?;
        thread::sleep(Duration::from_millis(1500));
        key(&mut c, &["esc"])?;
        thread::sleep(Duration::from_millis(1500));
        let asked = scene::capture_settled(run, &mut c, "install-a-cancel", &[])?;
        let asked = super::win32::keep(run, &asked, "install-a-cancel")?;
        key(&mut c, &["alt", "y"])?;
        wait_line(
            run,
            &record,
            before,
            "DOOR_P16 MSI DIALOG name=UserExit",
            30,
        )?;
        thread::sleep(Duration::from_millis(1500));
        let interrupted = scene::capture_settled(run, &mut c, "install-a-interrupted", &[])?;
        let interrupted = super::win32::keep(run, &interrupted, "install-a-interrupted")?;
        key(&mut c, &["ret"])?;
        let cancelled = wait_line(run, &record, before, "DOOR_P16 MSI END status=1602", 30)?;
        thread::sleep(Duration::from_secs(1));
        // The full UI: File Explorer opens the package.
        let before = explorer_open(
            run,
            &record,
            &mut c,
            "C:\\Users\\1001\\p16install\\tools-1.0.msi",
        )?;
        wait_line(run, &record, before, "DOOR_P16 FILES INSTALLER verb=/i", 20)?;
        wait_line(
            run,
            &record,
            before,
            "DOOR_P16 MSI DIALOG name=WelcomeEulaDlg",
            30,
        )?;
        thread::sleep(Duration::from_millis(1500));
        let welcome = scene::capture_settled(run, &mut c, "install-a-welcome", &[])?;
        let welcome = super::win32::keep(run, &welcome, "install-a-welcome")?;
        key(&mut c, &["alt", "a"])?;
        thread::sleep(Duration::from_millis(500));
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["alt", "i"])?;
        wait_line(run, &record, before, "DOOR_P16 MSI ELEVATE handoff", 60)?;
        wait_line(run, &record, before, "DOOR_P16 UAC PROMPT", 30)?;
        thread::sleep(Duration::from_millis(800));
        let uac = scene::capture_settled(run, &mut c, "install-a-uac", &[])?;
        let uac = super::win32::keep(run, &uac, "install-a-uac")?;
        key(&mut c, &["alt", "y"])?;
        wait_line(
            run,
            &record,
            before,
            "DOOR_P16 MSI DIALOG name=ProgressDlg",
            60,
        )?;
        wait_line(
            run,
            &record,
            before,
            "DOOR_P16 MSI DIALOG name=ExitDialog",
            300,
        )?;
        thread::sleep(Duration::from_millis(1500));
        let finished = scene::capture_settled(run, &mut c, "install-a-finished", &[])?;
        let finished = super::win32::keep(run, &finished, "install-a-finished")?;
        key(&mut c, &["ret"])?;
        let full_ui = wait_line(run, &record, before, "DOOR_P16 MSI END status=0", 30)?;
        thread::sleep(Duration::from_secs(1));
        // An EXE installer asks for administrator rights in its manifest.
        let before = explorer_open(
            run,
            &record,
            &mut c,
            "C:\\Users\\1001\\p16install\\setupprobe.exe",
        )?;
        wait_line(
            run,
            &record,
            before,
            "DOOR_P16 FILES ELEVATION REQUIRED",
            20,
        )?;
        wait_line(run, &record, before, "DOOR_P16 UAC PROMPT", 20)?;
        key(&mut c, &["alt", "y"])?;
        let setup = wait_line(run, &record, before, "DOOR_P16 ELEVATED LAUNCH started", 30)?;
        thread::sleep(Duration::from_secs(3));
        key(&mut c, &["alt", "f4"])?;
        thread::sleep(Duration::from_secs(1));
        // Settings > Apps > Installed apps lists both by name; uninstall them.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(400));
        type_line(&mut c, "ms-settings:appsfeatures")?;
        wait_line(
            run,
            &record,
            before,
            &format!("DOOR_P12 SETTINGS PAGE id={INSTALLED_APPS}"),
            20,
        )?;
        thread::sleep(Duration::from_millis(800));
        key(&mut c, &["meta_l", "up"])?;
        thread::sleep(Duration::from_secs(2));
        let listed = scene::capture_settled(run, &mut c, "install-a-installed-apps", &[])?;
        let listed = super::win32::keep(run, &listed, "install-a-installed-apps")?;
        let mut pointer = (640, 360);
        let before = settings_uninstall(run, &record, &mut c, &mut pointer, "Doors Setup Sample")?;
        wait_line(run, &record, before, "DOOR_P16 SETTINGS RUNAS", 20)?;
        wait_line(run, &record, before, "DOOR_P16 UAC PROMPT", 20)?;
        key(&mut c, &["alt", "y"])?;
        wait_line(run, &record, before, "DOOR_P16 ELEVATED LAUNCH started", 30)?;
        thread::sleep(Duration::from_secs(6));
        let before = settings_uninstall(run, &record, &mut c, &mut pointer, "Doors Tools")?;
        wait_line(
            run,
            &record,
            before,
            "DOOR_P16 SETTINGS START path=/compat/Windows/System32/msiexec.exe",
            20,
        )?;
        // "Are you sure you want to uninstall this product?" Yes.
        thread::sleep(Duration::from_secs(2));
        key(&mut c, &["ret"])?;
        wait_line(run, &record, before, "DOOR_P16 MSI ELEVATE handoff", 60)?;
        wait_line(run, &record, before, "DOOR_P16 UAC PROMPT", 30)?;
        key(&mut c, &["alt", "y"])?;
        let removed = wait_line(run, &record, before, "Removal completed successfully", 300)?;
        thread::sleep(Duration::from_secs(4));
        let emptied = scene::capture_settled(run, &mut c, "install-a-uninstalled", &[])?;
        let emptied = super::win32::keep(run, &emptied, "install-a-uninstalled")?;
        key(&mut c, &["alt", "f4"])?;
        thread::sleep(Duration::from_secs(1));
        // The power fails while an installation's custom action runs.
        let before = run_elevated(run, &record, &mut c, 1, "b-hang.bat")?;
        let hung = wait_line(run, &record, before, "Custom action HangInstall runs", 120)?;
        thread::sleep(Duration::from_secs(2));
        let _ = c.execute("quit", json!({}));
        Ok(
            json!({"fixtures":fixtures,"api":api,"plain":plain,"script":script,
            "cancelled":{"prompt":asked,"interrupted":interrupted,"end":cancelled},
            "full_ui":{"welcome":welcome,"uac":uac,"finished":finished,"end":full_ui},
            "setup":setup,"installed_apps":listed,"removed":removed,"uninstalled":emptied,"hung":hung}),
        )
    })();
    let first = super::win32::quit_on_error(run, &record, first, "p16-install-failure")?;
    // The power was cut: no orderly shutdown record to wait for. The
    // emulator lets go of the disk before the next boot opens it.
    let deadline = Instant::now() + Duration::from_secs(15);
    while connection(&record).is_ok() {
        if Instant::now() >= deadline {
            return Err("the emulator did not stop after the power cut".into());
        }
        thread::sleep(Duration::from_millis(200));
    }
    thread::sleep(Duration::from_secs(1));
    crate::evidence::seal_session_serial(&run.root, &mut record)?;
    // Boot 2: the interrupted installation is rolled back first.
    let mut second = session::launch_desktop_accelerated(run, &staged, &disk, false, &resolution)?;
    let after: Result<Value, String> = (|| {
        let mut c = connection(&second)?;
        thread::sleep(Duration::from_secs(3));
        let before = run_elevated(run, &second, &mut c, 2, "c-after.bat")?;
        let rollback = wait_line(
            run,
            &second,
            before,
            "DOOR_P16 MSI ROLLBACK interrupted product={3D1C9A57-0001",
            120,
        )?;
        wait_line(run, &second, before, "DOOR_P16 CMD TEXT after done", 300)?;
        let lines = text_lines(run, &second, before)?;
        require(
            &lines,
            &[
                "after 0",
                "tools gone",
                "marker gone",
                "in progress 1",
                "after removed 0",
            ],
            "INSTALL-A after the interruption",
        )?;
        // Both deferred programs changed the machine outside the script:
        // RecordInstall (undone by its rollback action) and the interrupted
        // HangInstall.
        if !rollback.contains("nontransactional=2") {
            return Err(format!(
                "the custom actions' records are missing: {rollback}"
            ));
        }
        let text = serial(&run.root, &second)?;
        for action in ["HangInstall", "RecordInstall"] {
            if !text.contains(&format!(
                "Rollback could not undo custom action {action}: it changed the system outside the installation script."
            )) {
                return Err(format!("rollback did not name {action}"));
            }
        }
        thread::sleep(Duration::from_secs(1));
        scene::capture_settled(run, &mut c, "install-a-after", &[])?;
        start_shut_down(&mut c)?;
        wait_serial(
            &run.root,
            &second,
            "DOOR_P08 CLEANUP PASS",
            Duration::from_secs(40),
        )?;
        Ok(json!({"rollback":rollback,"lines":lines}))
    })();
    let after = super::win32::quit_on_error(run, &second, after, "p16-install-after-failure")?;
    seal_serial(&run.root, &mut second)?;
    run.check(
        "p16-install-a",
        json!({"sessions":[record, second],"observed":{"boot":first,"after":after},
            "scope":"INSTALL-A: Windows Installer (msi.dll, msiexec.exe) and EXE installers: a standard token refused; msi.dll's database API; MsiInstallProduct and MsiConfigureProductEx; a failed installation rolled back with its rollback custom action; out of disk space; DLL and EXE custom actions; the file association, shortcut and registry; a shared component's client count; a major upgrade keeping the user's file; a refused downgrade; repair; removal; a package installing and starting a service, stopped and deleted with it; a suppressed restart (3010); the package's full UI cancelled (1602) and completed with User Account Control for msiexec.exe; an EXE installer's requireAdministrator manifest; Settings > Installed apps uninstalling both; an installation interrupted by a power loss rolled back by the next session with the nontransactional custom action recorded"}),
    );
    Ok(())
}
