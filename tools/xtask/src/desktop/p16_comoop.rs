//! P16 COMOOP-A: out-of-process COM. `comclient.exe` (MSVC-built, Windows
//! SDK, no C runtime) registers `comserver.exe` as a LocalServer32 and its
//! type library per user, activates the class (ole32 asks Init's COM
//! broker, starts the server with -Embedding, and connects to its class
//! endpoint), calls the dual interface through oleaut32's automation
//! marshaler and through IDispatch, takes a callback into its STA during a
//! call, gets RPC_E_WRONG_THREAD from another apartment and a working
//! proxy once marshaled, survives a server crash and activates a new
//! server, then unregisters. Machine-changing (it writes the registry), so
//! guest-only: the expectations are Windows' documented COM behaviour.
use super::p15_connect::{Probe, build, run_probe, stage_with};
use super::*;

const COMCLIENT: Probe = Probe {
    name: "comclient",
    directory: "tests/fixtures/windows-com",
    libraries: &["kernel32.lib", "OleAut32.Lib", "Ole32.Lib", "AdvAPI32.Lib"],
    modules: &["advapi32.dll", "kernel32.dll", "ole32.dll", "oleaut32.dll"],
};
const COMSERVER: Probe = Probe {
    name: "comserver",
    directory: "tests/fixtures/windows-com",
    libraries: &["kernel32.lib", "OleAut32.Lib", "Ole32.Lib", "User32.Lib"],
    modules: &["kernel32.dll", "ole32.dll", "oleaut32.dll", "user32.dll"],
};
const RESOLUTION: &str = "1280x720";
const STEPS: [&str; 11] = [
    "register",
    "activate",
    "calls",
    "inout",
    "errorinfo",
    "dispatch",
    "callback",
    "apartment",
    "crash",
    "relaunch",
    "unregister",
];
const TYPELIB: &str = "tests/fixtures/windows-com/doorsserver.tlb";

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/comoop-a", run.id))?;
    let client = build(run, &target, &COMCLIENT)?;
    let server = build(run, &target, &COMSERVER)?;
    let typelib = fs::read(run.root.join(TYPELIB)).map_err(|e| format!("{TYPELIB}: {e}"))?;
    let staged = stage_with(
        run,
        image,
        &[client.clone(), server.clone()],
        &[("doorsserver.tlb", typelib)],
        &target,
        "desktop-comoop",
        &[
            "windows/ole32.dll",
            "windows/oleaut32.dll",
            "windows/user32.dll",
            "windows/advapi32.dll",
        ],
    )?;
    let disk = super::win32::fresh_disk(run, "comoop-a")?;
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
            &client,
            "comclient.exe",
            "COMOOP-A",
            &STEPS,
        )?;
        let frame = scene::capture_settled(run, &mut c, "comoop-a-terminal", &[])?;
        let shot = super::win32::keep(run, &frame, "comoop-a-terminal")?;
        Ok(json!({"probe":probe,"screenshot":shot}))
    })();
    let details = super::win32::quit_on_error(run, &record, result, "comoop-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p16-comoop-a", json!({"fixtures":[client, server],"image":staged,"run":details,"session":record,
        "reference":"guest-only (registers a LocalServer32 and a type library); expectations from Windows' documented COM behaviour",
        "scope":"COMOOP-A: LocalServer32 activation through Init's COM broker (-Embedding launch, class endpoint), the automation marshaler's proxies and stubs for a dual interface (scalars, BSTR, VARIANT, SAFEARRAY, [in, out], float/double, error info), IDispatch through the proxy (type information, names, Invoke, DISP_E_EXCEPTION), a callback into the client's STA during a call, RPC_E_WRONG_THREAD and CoMarshalInterThreadInterfaceInStream between apartments, a server crash (call failed, server unavailable) and a new server on the next activation"}));
    Ok(())
}
