//! P17 X86-A and X86-B: 32-bit (x86) Windows programs under Doors' WOW64.
//!
//! `x86probe.exe check` (MSVC x86, Windows SDK, no C runtime, /SAFESEH:NO)
//! compares what it reads with what Windows 11 printed for the same bytes
//! (`x86probe.expected.h`, from `x86probe.exe raw` on the owner's PC):
//! console output, its own and kernel32's modules, heap and virtual memory
//! below 2 GB, implicit and dynamic TLS, a thread, x87 and SSE2 arithmetic,
//! the WOW64 system queries (GetSystemInfo, GetNativeSystemInfo,
//! IsWow64Process(2), the system folders) and structured exception
//! handling through a frame it registers at FS:[0] (an access violation and
//! RaiseException, both continued by its handler).
//!
//! `x86gui.exe check` does the same for user32 and gdi32 with hidden
//! windows (`x86gui.expected.h`): the structures and callbacks whose layout
//! differs from x64 (WNDCLASSEXW, CREATESTRUCTW, window and class longs,
//! MSG, cross-thread sends and their marshalled WINDOWPOS, PAINTSTRUCT, DIB
//! sections, BITMAP and LOGBRUSH, controls, timers, menus, a modeless
//! dialog's DWLP slots), each structure followed by a canary.
//!
//! Each process must have loaded the 32-bit DLLs (`windows32/`), never the
//! x64 ones.
use super::p15_connect::{Probe, build_x86, run_probe, stage, stage_with};
use super::*;
use sha2::{Digest, Sha256};

const X86PROBE: Probe = Probe {
    name: "x86probe",
    directory: "tests/fixtures/windows-x86",
    libraries: &["kernel32.lib"],
    modules: &["kernel32.dll"],
};
const X86GUI: Probe = Probe {
    name: "x86gui",
    directory: "tests/fixtures/windows-x86",
    libraries: &["kernel32.lib", "User32.Lib", "Gdi32.Lib"],
    modules: &["gdi32.dll", "kernel32.dll", "user32.dll"],
};
const RESOLUTION: &str = "1280x720";
/// Steps each probe reports, in order.
const STEPS: [&str; 10] = [
    "console", "module", "memory", "tls", "thread", "math", "system", "seh", "unwind", "dll",
];
const GUI_STEPS: [&str; 10] = [
    "class", "create", "message", "thread", "paint", "gdi", "control", "callback", "menu", "dialog",
];
/// The 32-bit DLLs each probe loads, and the x64 ones it must not.
const DLLS32: [&str; 2] = ["windows32/ntdll.dll", "windows32/kernel32.dll"];
const DLLS64: [&str; 2] = ["windows/ntdll.dll", "windows/kernel32.dll"];
const GUI_DLLS32: [&str; 4] = [
    "windows32/ntdll.dll",
    "windows32/kernel32.dll",
    "windows32/user32.dll",
    "windows32/gdi32.dll",
];
const GUI_DLLS64: [&str; 4] = [
    "windows/ntdll.dll",
    "windows/kernel32.dll",
    "windows/user32.dll",
    "windows/gdi32.dll",
];

/// The SHA-256 of each of `paths` in the image's initfs.
fn shipped(run: &Run, image: &str, paths: &[&str]) -> Result<Vec<String>, String> {
    let (base, _) = crate::image::validate_staged(&run.root, image)?;
    let production = fs::read(base.join("esp/Door/initfs.tar")).map_err(|e| e.to_string())?;
    let archive =
        door_initfs::Archive::parse(&production).map_err(|e| format!("desktop initfs {e:?}"))?;
    paths
        .iter()
        .map(|path| {
            let entry = archive.find(path).ok_or(format!(
                "the desktop image does not ship {path}; rebuild it"
            ))?;
            Ok(format!("{:x}", Sha256::digest(entry.data)))
        })
        .collect()
}
/// The probe's process loaded every 32-bit DLL of `wanted` and none of the
/// x64 ones of `refused`: their (path, hash) records.
fn loaded_32_bit(
    name: &str,
    probe: &Value,
    wanted: (&[&str], &[String]),
    refused: (&[&str], &[String]),
) -> Result<Vec<Value>, String> {
    let modules: Vec<&str> = probe["modules"]
        .as_array()
        .ok_or("probe modules")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for (path, hash) in wanted.0.iter().zip(wanted.1) {
        if !modules.contains(&hash.as_str()) {
            return Err(format!("{name} did not load {path} ({hash}): {modules:?}"));
        }
    }
    for (path, hash) in refused.0.iter().zip(refused.1) {
        if modules.contains(&hash.as_str()) {
            return Err(format!("{name} loaded the x64 {path}"));
        }
    }
    Ok(wanted
        .0
        .iter()
        .zip(wanted.1)
        .map(|(p, h)| json!({"path":p,"sha256":h}))
        .collect())
}

/// `x86dll.dll`, X86-A's private DLL: MSVC x86, no C runtime and no
/// imports, linked at the executable's base (0x400000) so the loader must
/// relocate it. Its record and bytes.
fn build_x86_dll(run: &mut Run, target: &std::path::Path) -> Result<(Value, Vec<u8>), String> {
    use crate::windows::native_path;
    let profile = crate::windows::build::toolchain(&run.root)?;
    let msvc = profile["msvc_root"].as_str().ok_or("MSVC root")?.to_owned();
    let sdk = profile["sdk_root"].as_str().ok_or("SDK root")?.to_owned();
    let version = profile["sdk_version"]
        .as_str()
        .ok_or("SDK version")?
        .to_owned();
    let source = "tests/fixtures/windows-x86/x86dll.c";
    let resources = "tests/fixtures/windows-x86/x86dll.rc";
    let compiled = target.join("x86dll.res");
    run.required(
        &format!("{sdk}/bin/{version}/x64/rc.exe"),
        &[
            "/nologo",
            &format!("/I{sdk}/Include/{version}/um"),
            &format!("/I{sdk}/Include/{version}/shared"),
            "/fo",
            &native_path(&compiled)?,
            &native_path(&run.root.join(resources))?,
        ],
    )?;
    let object = target.join("x86dll.obj");
    let dll = target.join("x86dll.dll");
    let map = target.join("x86dll.map");
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
    compile.push(native_path(&run.root.join(source))?);
    run.required(
        &format!("{msvc}/bin/Hostx64/x86/cl.exe"),
        &compile.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    let mut link: Vec<String> = [
        "/NOLOGO",
        "/NODEFAULTLIB",
        "/DLL",
        "/ENTRY:entry",
        "/DYNAMICBASE",
        "/NXCOMPAT",
        "/Brepro",
        "/DEBUG:NONE",
        "/INCREMENTAL:NO",
        "/MANIFEST:NO",
        "/OPT:REF",
        "/OPT:ICF",
        "/MACHINE:X86",
        "/SAFESEH:NO",
        "/BASE:0x400000",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    link.push(format!("/OUT:{}", native_path(&dll)?));
    link.push(format!("/MAP:{}", native_path(&map)?));
    link.push(native_path(&object)?);
    link.push(native_path(&compiled)?);
    run.required(
        &format!("{msvc}/bin/Hostx64/x86/link.exe"),
        &link.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    let pe = crate::windows::pe::inspect(&dll)?;
    if pe["imports"].as_array().is_some_and(|i| !i.is_empty()) {
        return Err("x86dll.dll must import nothing".into());
    }
    let bytes = fs::read(&dll).map_err(|e| e.to_string())?;
    Ok((
        json!({"name":"x86dll","source":source,"source_sha256":file_hash(&run.root.join(source))?,
        "resources":resources,"resources_sha256":file_hash(&run.root.join(resources))?,
        "path":relative(&run.root,&dll)?,"sha256":file_hash(&dll)?,
        "link_map":{"path":relative(&run.root,&map)?,"sha256":file_hash(&map)?},
        "machine":"x86","runtime":"no CRT; /DLL at 0x400000 (relocated); DllMain"}),
        bytes,
    ))
}

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/x86-a", run.id))?;
    let fixture = build_x86(run, &target, &X86PROBE)?;
    let gui = build_x86(run, &target, &X86GUI)?;
    let (private_dll, private_bytes) = build_x86_dll(run, &target)?;
    let dlls32 = shipped(run, image, &DLLS32)?;
    let dlls64 = shipped(run, image, &DLLS64)?;
    let gui32 = shipped(run, image, &GUI_DLLS32)?;
    let gui64 = shipped(run, image, &GUI_DLLS64)?;
    let staged = stage_with(
        run,
        image,
        &[fixture.clone(), gui.clone()],
        &[("x86dll.dll", private_bytes)],
        &target,
        "desktop-x86",
        &GUI_DLLS32,
    )?;
    let disk = super::win32::fresh_disk(run, "x86-a")?;
    let mut record = session::launch_desktop_accelerated(run, &staged, &disk, false, RESOLUTION)?;
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        thread::sleep(Duration::from_secs(3));
        super::win32::open_terminal(run, &record, &mut c)?;
        thread::sleep(Duration::from_secs(2));
        let probe = run_probe(
            run,
            &record,
            &mut c,
            &fixture,
            "x86probe.exe check",
            "X86-A",
            &STEPS,
        )?;
        let loaded = loaded_32_bit("x86probe", &probe, (&DLLS32, &dlls32), (&DLLS64, &dlls64))?;
        let exceptions: Vec<String> = serial(&run.root, &record)?
            .lines()
            .filter(|l| l.contains("DOOR_P17 EXCEPTION"))
            .map(|l| l.trim_end().to_owned())
            .collect();
        if !exceptions.iter().any(|l| l.contains("code=0xc0000005")) {
            return Err(format!(
                "no 32-bit access violation delivered: {exceptions:?}"
            ));
        }
        let gui_probe = run_probe(
            run,
            &record,
            &mut c,
            &gui,
            "x86gui.exe check",
            "X86-B",
            &GUI_STEPS,
        )?;
        let gui_loaded = loaded_32_bit(
            "x86gui",
            &gui_probe,
            (&GUI_DLLS32, &gui32),
            (&GUI_DLLS64, &gui64),
        )?;
        let frame = scene::capture_settled(run, &mut c, "x86-a-terminal", &[])?;
        let shot = super::win32::keep(run, &frame, "x86-a-terminal")?;
        Ok(
            json!({"probe":probe,"dlls32":loaded,"exceptions":exceptions,
            "gui_probe":gui_probe,"gui_dlls32":gui_loaded,"screenshot":shot}),
        )
    })();
    let details = super::win32::quit_on_error(run, &record, result, "x86-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p17-x86-a", json!({"fixture":fixture,"gui_fixture":gui,"private_dll":private_dll,"image":staged,"run":details,"session":record,
        "reference":"Windows 11 26H2 (owner's PC, WOW64): x86probe.exe check 10/10 and x86gui.exe check 10/10; raw output in compat/dlls/kernel32/tests/reference/x86probe-raw.txt and compat/dlls/user32/tests/reference/x86gui-raw.txt",
        "scope":"X86-A: an MSVC-built 32-bit console program (PE32, /SAFESEH:NO) loaded at its preferred base with the 32-bit ntdll.dll and kernel32.dll (i686 builds of the same sources, far calls into the 64-bit system-call gate): WriteConsole, GetModuleHandle/GetModuleFileName for itself and kernel32, HeapAlloc and VirtualAlloc below 2 GB, the implicit TLS template at FS:[0x2C] and TlsAlloc slots, CreateThread with its own TLS, x87 and SSE2 arithmetic through a varargs call, GetSystemInfo (Intel, 586, 2 GB) and GetNativeSystemInfo (x64), IsWow64Process(2) (I386 on AMD64), GetSystemDirectory (System32) and GetSystemWow64Directory (SysWOW64); an access violation delivered by the kernel as an x86 EXCEPTION_RECORD and CONTEXT and RaiseException from kernel32, both handled by an SEH frame at FS:[0] that edits the context and continues; and unwinding: an outer frame that calls RtlUnwind after an inner one passed the exception on, the inner handler running again with EXCEPTION_UNWINDING (as __finally does). X86-B: a 32-bit program over the 32-bit user32.dll and gdi32.dll with hidden windows: RegisterClassExW/GetClassInfoExW (48-byte WNDCLASSEXW), CREATESTRUCTW in WM_NCCREATE/WM_CREATE (a copy of the class name, WS_EX_WINDOWEDGE), GetWindowLong/SetWindowLong on 4-byte extra slots and GWL_WNDPROC, SendMessageW LRESULT and LPARAM signs, PostMessageW/PeekMessageW MSG (28 bytes), SendMessageTimeoutW's DWORD_PTR, cross-thread WM_SETTEXT/WM_GETTEXT and SetWindowPos (marshalled WINDOWPOS), BeginPaint's PAINTSTRUCT, CreateDIBSection's bits pointer, GetObjectW BITMAP (24) and LOGBRUSH (12), button/edit controls with EM_GETSEL, EnumChildWindows and a TIMERPROC, menus, a modeless dialog's WM_INITDIALOG parameter, DWL_USER and DWL_MSGRESULT, each structure followed by an intact canary"}));
    Ok(())
}

const WOWPROBE: Probe = Probe {
    name: "wowprobe",
    directory: "tests/fixtures/windows-x86",
    libraries: &["kernel32.lib", "AdvAPI32.Lib", "shell32.lib"],
    modules: &["advapi32.dll", "kernel32.dll", "shell32.dll"],
};
const WOWCHILD: Probe = Probe {
    name: "wowchild",
    directory: "tests/fixtures/windows-x86",
    libraries: &["kernel32.lib", "AdvAPI32.Lib"],
    modules: &["advapi32.dll", "kernel32.dll"],
};
const WOW_STEPS: [&str; 5] = ["environment", "folders", "registry", "redirection", "child"];
const WOW_DLLS32: [&str; 5] = [
    "windows32/ntdll.dll",
    "windows32/kernel32.dll",
    "windows32/advapi32.dll",
    "windows32/user32.dll",
    "windows32/shell32.dll",
];
const WOW_DLLS64: [&str; 5] = [
    "windows/ntdll.dll",
    "windows/kernel32.dll",
    "windows/advapi32.dll",
    "windows/user32.dll",
    "windows/shell32.dll",
];
/// What the elevated `wowprobe.exe guest` must print after its checks: the
/// 32-bit installer's writes landed (registry, System32, Program Files) and
/// the x64 child found all seven through the 64-bit views (0x7f), the shared App Paths key included.
const INSTALLED: &str = "WOW-C install registry=1 system32=1 sysnative=1 syswow64=1 programfiles=1";
const VERIFIED: &str = "WOW-C verify exit=127";
/// `msiexec /x {unknown} /qn` started by name from the 32-bit program
/// (System32 is SysWOW64 for it): ERROR_UNKNOWN_PRODUCT.
const INSTALLER_BY_NAME: &str = "WOW-C msiexec exit=1605";

/// P17 X86-C: what a 32-bit program sees of WOW64's views, checked against
/// Windows 11 (`wowprobe.expected.h`, from `wowprobe.exe raw` on the
/// owner's PC): the environment (PROCESSOR_ARCHITEW6432, the Program Files
/// variables), SHGetKnownFolderPath's Program Files and system folders, the
/// registry's 32-bit view (WOW6432Node) beside KEY_WOW64_64KEY and a shared
/// key, Wow64Disable/RevertWow64FsRedirection, Sysnative and SysWOW64, and
/// an x64 child (`wowchild.exe`) the 32-bit program starts: IsWow64Process
/// and IsWow64Process2 of it and what it sees of its own environment. Then,
/// from an elevated Terminal, `wowprobe.exe guest` installs as a 32-bit
/// installer would (HKLM\SOFTWARE, C:\Windows\System32, %ProgramFiles%) and
/// the x64 child verifies through the 64-bit views where each landed and
/// removes them.
pub(super) fn verify_wow(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/x86-c", run.id))?;
    let fixture = build_x86(run, &target, &WOWPROBE)?;
    let helper = super::p15_connect::build(run, &target, &WOWCHILD)?;
    let dlls32 = shipped(run, image, &WOW_DLLS32)?;
    let dlls64 = shipped(run, image, &WOW_DLLS64)?;
    let staged = stage(
        run,
        image,
        &[fixture.clone(), helper.clone()],
        &target,
        "desktop-wow64",
        &WOW_DLLS32,
    )?;
    let disk = super::win32::fresh_disk(run, "x86-c")?;
    let mut record = session::launch_desktop_accelerated(run, &staged, &disk, false, RESOLUTION)?;
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        thread::sleep(Duration::from_secs(3));
        super::win32::open_terminal(run, &record, &mut c)?;
        thread::sleep(Duration::from_secs(2));
        let offset = serial(&run.root, &record)?.len();
        let probe = run_probe(
            run,
            &record,
            &mut c,
            &fixture,
            "wowprobe.exe check",
            "WOW-C",
            &WOW_STEPS,
        )?;
        let loaded = loaded_32_bit(
            "wowprobe",
            &probe,
            (&WOW_DLLS32, &dlls32),
            (&WOW_DLLS64, &dlls64),
        )?;
        // The child is an x64 process: its modules are the x64 DLLs.
        let child = super::win32::processes(&serial(&run.root, &record)?, offset)
            .into_values()
            .find(|p| p.modules.first().map(String::as_str) == helper["sha256"].as_str())
            .ok_or("wowchild.exe did not start")?;
        let child_dlls: Vec<Value> = WOW_DLLS64[..3]
            .iter()
            .zip(&dlls64)
            .map(|(path, hash)| {
                if child.modules.contains(hash) {
                    Ok(json!({"path":path,"sha256":hash}))
                } else {
                    Err(format!("wowchild.exe did not load the x64 {path}"))
                }
            })
            .collect::<Result<_, String>>()?;
        if dlls32.iter().any(|h| child.modules.contains(h)) {
            return Err("wowchild.exe loaded a 32-bit DLL".into());
        }
        // Elevated: Doors runs at most 13 processes, so this Terminal
        // closes first.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["alt", "f4"])?;
        super::win32::wait_after(run, &record, before, "CLIENT_EXIT", 15)?;
        thread::sleep(Duration::from_millis(800));
        let uac = super::p15_connect::open_admin_terminal(run, &record, &mut c)?;
        let console = session::transcript(&run.root, &record)?.len();
        let installer = run_probe(
            run,
            &record,
            &mut c,
            &fixture,
            "wowprobe.exe guest",
            "WOW-C",
            &WOW_STEPS,
        )?;
        let text = session::transcript(&run.root, &record)?;
        let output = text.get(console.min(text.len())..).unwrap_or("");
        let written: Vec<&str> = output
            .lines()
            .filter(|l| {
                l.starts_with("WOW-C install")
                    || l.starts_with("WOW-C verify")
                    || l.starts_with("WOW-C msiexec")
            })
            .map(str::trim_end)
            .collect();
        if written != [INSTALLED, VERIFIED, INSTALLER_BY_NAME] {
            return Err(format!("wowprobe.exe guest: {written:?}"));
        }
        let frame = scene::capture_settled(run, &mut c, "x86-c-terminal", &[])?;
        let shot = super::win32::keep(run, &frame, "x86-c-terminal")?;
        Ok(
            json!({"probe":probe,"dlls32":loaded,"child_dlls64":child_dlls,"uac":uac,
            "installer":installer,"written":written,"screenshot":shot}),
        )
    })();
    let details = super::win32::quit_on_error(run, &record, result, "x86-c-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p17-x86-c", json!({"fixture":fixture,"helper":helper,"image":staged,"run":details,"session":record,
        "reference":r"Windows 11 26H2 (owner's PC, WOW64): wowprobe.exe check 5/5 (read only); raw output in compat/dlls/kernel32/tests/reference/wowprobe-raw.txt. The guest mode writes HKLM and C:\Windows and runs only in the guest",
        "scope":r"X86-C: a 32-bit program over the 32-bit kernel32, advapi32 and shell32: PROCESSOR_ARCHITECTURE x86 with PROCESSOR_ARCHITEW6432 AMD64 and the Program Files variables of a WOW64 process; SHGetKnownFolderPath ProgramFiles/ProgramFilesX86/ProgramFilesCommon (x86), ProgramFilesX64 and ProgramFilesCommonX64 refused, System and SystemX86; the registry's default (32-bit, WOW6432Node) and KEY_WOW64_64KEY/KEY_WOW64_32KEY views of ProgramFilesDir and CommonFilesDir and a shared key (App Paths); Wow64DisableWow64FsRedirection/Revert, Sysnative and SysWOW64; an x64 child it creates (IsWow64Process FALSE, IsWow64Process2 UNKNOWN on AMD64, its own 64-bit environment). From an elevated Terminal: a key under HKLM\SOFTWARE (stored in WOW6432Node), a file written to C:\Windows\System32 (stored in SysWOW64, not visible through Sysnative) and to %ProgramFiles% (Program Files (x86)), verified and removed by the x64 child through the 64-bit views; then `msiexec /x {unknown} /qn` started by name (found through SysWOW64) ends with 1605"}));
    Ok(())
}
