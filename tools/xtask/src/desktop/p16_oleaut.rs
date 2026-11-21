//! P16 OLEAUT-A: OLE Automation through oleaut32.dll. `oleautprobe.exe`
//! (MSVC-built, Windows SDK, no C runtime; it imports oleaut32 by ordinal,
//! as OleAut32.Lib does) checks each step's output against what Windows 11
//! 26H2 printed (`oleautprobe.expected.h`): BSTR layout, VARIANT
//! copy/clear/change-type, SAFEARRAY descriptors and elements, dates,
//! error info, the type library MIDL built from doorsprobe.idl (every
//! TYPEATTR, FUNCDESC, VARDESC, name and document string, its dual
//! interface's dispatch and vtable sides, and stdole2 as it reaches it),
//! and IDispatch (ITypeInfo::Invoke through DispInvoke: coercion,
//! defaults, optional and named arguments, property put, exceptions from
//! error info; DispGetIDsOfNames, CreateStdDispatch, DispGetParam).
//! In the guest it adds per-user registration: RegisterTypeLibForUser,
//! QueryPathOfRegTypeLib, LoadRegTypeLib, the Interface key with the
//! automation proxy, UnRegisterTypeLibForUser, stdole2 by GUID.
use super::p15_connect::{Probe, build, run_probe, stage_with};
use super::*;

const OLEAUTPROBE: Probe = Probe {
    name: "oleautprobe",
    directory: "tests/fixtures/windows-com",
    libraries: &[
        "kernel32.lib",
        "OleAut32.Lib",
        "shell32.lib",
        "AdvAPI32.Lib",
    ],
    modules: &[
        "advapi32.dll",
        "kernel32.dll",
        "oleaut32.dll",
        "shell32.dll",
    ],
};
const RESOLUTION: &str = "1280x720";
/// Steps oleautprobe reports, in order.
const STEPS: [&str; 8] = [
    "bstr",
    "variant",
    "safearray",
    "dates",
    "errorinfo",
    "typelib",
    "dispatch",
    "registration",
];
const TYPELIB: &str = "tests/fixtures/windows-com/doorsprobe.tlb";

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/oleaut-a", run.id))?;
    let fixture = build(run, &target, &OLEAUTPROBE)?;
    let typelib = fs::read(run.root.join(TYPELIB)).map_err(|e| format!("{TYPELIB}: {e}"))?;
    let staged = stage_with(
        run,
        image,
        std::slice::from_ref(&fixture),
        &[("doorsprobe.tlb", typelib)],
        &target,
        "desktop-oleaut",
        &[
            "windows/oleaut32.dll",
            "windows/ole32.dll",
            "windows/advapi32.dll",
            "windows/shell32.dll",
        ],
    )?;
    let disk = super::win32::fresh_disk(run, "oleaut-a")?;
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
            "oleautprobe.exe guest /system/bin/doorsprobe.tlb",
            "OLEAUT-A",
            &STEPS,
        )?;
        let frame = scene::capture_settled(run, &mut c, "oleaut-a-terminal", &[])?;
        let shot = super::win32::keep(run, &frame, "oleaut-a-terminal")?;
        Ok(json!({"probe":probe,"screenshot":shot}))
    })();
    let details = super::win32::quit_on_error(run, &record, result, "oleaut-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p16-oleaut-a", json!({"fixture":fixture,"image":staged,"run":details,"session":record,
        "reference":"Windows 11 26H2 (owner's PC): oleautprobe.exe check doorsprobe.tlb 7/7 (registration is guest-only: it writes the registry)",
        "scope":"OLEAUT-A: an MSVC-built probe importing oleaut32 by ordinal checks BSTR, VARIANT, SAFEARRAY, dates and error info, the MIDL type library's TYPEATTR/FUNCDESC/VARDESC/names/documentation (dual interface dispatch and vtable sides, stdole2), IDispatch through ITypeInfo::Invoke/DispInvoke (coercion, defaults, optional and named arguments, property put, exceptions), CreateStdDispatch and DispGetParam against Windows' own output, and per-user type-library registration"}));
    Ok(())
}
