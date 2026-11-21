//! P16 TSCHED-A: the Task Scheduler 2.0 API (taskschd.dll) through COM.
//! `tschedprobe.exe guest` (MSVC-built, Windows SDK, no C runtime) compares
//! what it reads and builds (ITaskService, the root folder, definitions
//! with every trigger, action, setting and principal property, their XML,
//! validation, IDispatch through the type library) with what Windows 11
//! printed (`tschedprobe.expected.h`, from `tschedprobe.exe raw` on the
//! owner's PC, which only reads), then registers a task, runs it (the
//! probe itself, exiting 7), disables and deletes it, which only the guest
//! may do.
use super::p15_connect::{Probe, build, run_probe, stage};
use super::*;

const TSCHEDPROBE: Probe = Probe {
    name: "tschedprobe",
    directory: "tests/fixtures/windows-com",
    libraries: &["kernel32.lib", "Ole32.Lib", "OleAut32.Lib"],
    modules: &["kernel32.dll", "ole32.dll", "oleaut32.dll"],
};
const RESOLUTION: &str = "1280x720";
/// Steps tschedprobe reports, in order.
const STEPS: [&str; 14] = [
    "service",
    "folder",
    "definition",
    "triggers",
    "actions",
    "xml",
    "validate",
    "fullxml",
    "roundtrip",
    "dispatch",
    "register",
    "run",
    "enable",
    "delete",
];

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/tsched-a", run.id))?;
    let fixture = build(run, &target, &TSCHEDPROBE)?;
    let staged = stage(
        run,
        image,
        std::slice::from_ref(&fixture),
        &target,
        "desktop-tsched",
        &[
            "windows/taskschd.dll",
            "windows/ole32.dll",
            "windows/oleaut32.dll",
        ],
    )?;
    let disk = super::win32::fresh_disk(run, "tsched-a")?;
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
            "tschedprobe.exe guest",
            "TSCHED-A",
            &STEPS,
        )?;
        // The run the probe started: Init launched the task's program.
        let launched = super::p15_desktop::line_after(
            run,
            &record,
            0,
            "DOOR_P16 TASK run name=DoorsProbeTask",
        )?;
        let exited = super::p15_desktop::line_after(
            run,
            &record,
            0,
            "DOOR_P16 TASK exit name=DoorsProbeTask code=7",
        )?;
        let frame = scene::capture_settled(run, &mut c, "tsched-a-terminal", &[])?;
        let shot = super::win32::keep(run, &frame, "tsched-a-terminal")?;
        Ok(json!({"probe":probe,"launched":launched,"exited":exited,"screenshot":shot}))
    })();
    let details = super::win32::quit_on_error(run, &record, result, "tsched-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p16-tsched-a", json!({"fixture":fixture,"image":staged,"run":details,"session":record,
        "reference":"Windows 11 26H2 (owner's PC): tschedprobe.exe check 10/10 (it reads the scheduler and builds definitions only; registering a task is guest-only)",
        "scope":"TSCHED-A: an MSVC-built probe using ITaskService, ITaskFolder, ITaskDefinition with registration info, every trigger and action type, settings, principal, the task XML (written and read), RegisterTaskDefinition validation, IDispatch through the TaskScheduler type library, then registering, running, disabling and deleting a task"}));
    Ok(())
}
