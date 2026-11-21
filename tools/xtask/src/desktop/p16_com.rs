//! P16 COM-A: an ordinary Windows console program (`comprobe.exe`, built
//! with the pinned MSVC toolchain and SDK import libraries, no C runtime)
//! runs from Terminal and uses in-process COM through ole32.dll:
//! apartments, GUID helpers, the task allocator, a class object it
//! registers itself, DirectSound activated from its HKCR\CLSID
//! registration (which Init writes at startup; the program does not import
//! dsound.dll, so COM loads it at run time), LoadLibrary of winmm.dll by
//! path, ProgIDs, an IStream over
//! HGLOBAL, PROPVARIANT copies and the implicit MTA of a second thread.
use super::p15_connect::{Probe, build, run_probe, stage};
use super::*;

const COMPROBE: Probe = Probe {
    name: "comprobe",
    directory: "tests/fixtures/windows-com",
    libraries: &["kernel32.lib", "Ole32.Lib"],
    modules: &["kernel32.dll", "ole32.dll"],
};
const RESOLUTION: &str = "1280x720";
/// Steps comprobe reports, in order.
const STEPS: [&str; 9] = [
    "apartment",
    "guids",
    "task-memory",
    "class-object",
    "directsound",
    "progid",
    "hglobal-stream",
    "propvariant",
    "implicit-mta",
];

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/com-a", run.id))?;
    let fixture = build(run, &target, &COMPROBE)?;
    let staged = stage(
        run,
        image,
        std::slice::from_ref(&fixture),
        &target,
        "desktop-com",
        &[
            "windows/ole32.dll",
            "windows/advapi32.dll",
            "windows/dsound.dll",
        ],
    )?;
    let disk = super::win32::fresh_disk(run, "com-a")?;
    let mut record = session::launch_desktop_accelerated(run, &staged, &disk, false, RESOLUTION)?;
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        // Init registers the system classes before the session starts.
        let classes = super::p15_desktop::line_after(run, &record, 0, "DOOR_P16 CLASSES ")?;
        if !classes.contains("registered") {
            return Err(format!("system COM classes: {classes}"));
        }
        thread::sleep(Duration::from_secs(2));
        super::win32::open_terminal(run, &record, &mut c)?;
        thread::sleep(Duration::from_secs(1));
        let probe = run_probe(
            run,
            &record,
            &mut c,
            &fixture,
            "comprobe.exe",
            "COM-A",
            &STEPS,
        )?;
        let frame = scene::capture_settled(run, &mut c, "com-a-terminal", &[])?;
        let shot = super::win32::keep(run, &frame, "com-a-terminal")?;
        Ok(json!({"probe":probe,"classes":classes,"screenshot":shot}))
    })();
    let details = super::win32::quit_on_error(run, &record, result, "com-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p16-com-a", json!({"fixture":fixture,"image":staged,"run":details,"session":record,
        "scope":"An MSVC-built Windows console program using in-process COM through ole32.dll in the guest: apartments, GUIDs, task memory, registered class objects, DirectSound activation from HKCR, ProgIDs, HGLOBAL streams, PROPVARIANT and the implicit MTA"}));
    Ok(())
}
