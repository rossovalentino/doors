//! P16 SCM-A: a Windows service installed, run and removed through the
//! Service Control Manager Init hosts. Two programs built with the pinned
//! MSVC toolchain and SDK import libraries (no C runtime): `doorsvc.exe`, an
//! ordinary service (StartServiceCtrlDispatcher, RegisterServiceCtrlHandlerEx,
//! SetServiceStatus), and `svcprobe.exe`, which uses advapi32's SCM API
//! (OpenSCManager, CreateService, ChangeServiceConfig2, QueryServiceConfig,
//! StartService, QueryServiceStatusEx, ControlService pause/continue/stop,
//! EnumServicesStatusEx, DeleteService). Run's Ctrl+Shift+Enter opens an
//! elevated Doorsh (User Account Control) where the probe copies the service
//! into C:\Program Files, runs the checks and removes it again. Guest only:
//! installing a service changes the machine, so nothing runs on the
//! reference PC.
use super::p15_connect::{Probe, build, run_probe, stage};
use super::*;

const DIRECTORY: &str = "tests/fixtures/windows-services";
const SERVICE: Probe = Probe {
    name: "doorsvc",
    directory: DIRECTORY,
    libraries: &["kernel32.lib", "AdvAPI32.Lib"],
    modules: &["advapi32.dll", "kernel32.dll"],
};
const SVCPROBE: Probe = Probe {
    name: "svcprobe",
    directory: DIRECTORY,
    libraries: &["kernel32.lib", "AdvAPI32.Lib", "shell32.lib"],
    modules: &["advapi32.dll", "kernel32.dll", "shell32.dll"],
};
const RESOLUTION: &str = "1280x720";
/// Steps svcprobe reports, in order.
const STEPS: [&str; 13] = [
    "install",
    "open-manager",
    "create",
    "configure",
    "start",
    "pause-continue",
    "enumerate",
    "stop",
    "recovery",
    "crash-loop",
    "recovery-program",
    "hung-stop",
    "delete",
];
/// An Application event whose message runs past one IPC packet (the Event
/// Log sends it in parts).
const LONG_EVENT: &str = "SCM-A long event: the Event Log keeps messages longer than one packet, the Event Log keeps messages longer than one packet, the Event Log keeps messages longer than one packet, the Event Log keeps messages longer than one packet, the Event Log keeps messages longer than one packet.";

fn wait_for(run: &Run, record: &Value, before: usize, needle: &str) -> Result<String, String> {
    wait_long(run, record, before, needle, 40)
}
fn wait_long(
    run: &Run,
    record: &Value,
    before: usize,
    needle: &str,
    seconds: u64,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let text = serial(&run.root, record)?;
        // Complete lines only: the guest may be writing the last one.
        let complete = &text[..text.rfind('\n').map_or(0, |end| end + 1)];
        if let Some(line) = complete
            .get(before.min(complete.len())..)
            .unwrap_or("")
            .lines()
            .find(|l| l.contains(needle))
        {
            return Ok(line.to_owned());
        }
        if text.contains("KERNEL_PANIC") || text.contains("APP FAILED") {
            return Err("guest failed; inspect serial".into());
        }
        if Instant::now() >= deadline {
            return Err(format!("{needle} missing"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/scm-a", run.id))?;
    let service = build(run, &target, &SERVICE)?;
    let probe = build(run, &target, &SVCPROBE)?;
    let staged = stage(
        run,
        image,
        &[service.clone(), probe.clone()],
        &target,
        "desktop-scm",
        &[
            "windows/advapi32.dll",
            "windows/ntdll.dll",
            "windows/kernel32.dll",
        ],
    )?;
    let disk = super::win32::fresh_disk(run, "scm-a")?;
    let mut record = session::launch_desktop_accelerated(run, &staged, &disk, false, RESOLUTION)?;
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        thread::sleep(Duration::from_secs(3));
        // SCM-D: a standard (unelevated) Terminal may list services but
        // not change them.
        super::win32::open_terminal(run, &record, &mut c)?;
        thread::sleep(Duration::from_secs(2));
        let denied = run_probe(
            run,
            &record,
            &mut c,
            &probe,
            "svcprobe.exe denied",
            "SCM-D",
            &["manager-denied", "manager-read", "service-denied"],
        )?;
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["alt", "f4"])?;
        super::win32::wait_after(run, &record, before, "CLIENT_EXIT", 15)?;
        thread::sleep(Duration::from_millis(800));
        // Win+R doorsh, Ctrl+Shift+Enter: User Account Control, then the
        // elevated Terminal on a Doorsh tab.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(400));
        for ch in "doorsh".chars() {
            super::administration::type_character(&mut c, ch)?;
        }
        key(&mut c, &["ctrl", "shift", "ret"])?;
        wait_for(run, &record, before, "DOOR_P16 RUN elevated")?;
        wait_for(run, &record, before, "DOOR_P16 UAC PROMPT")?;
        key(&mut c, &["alt", "y"])?;
        wait_for(run, &record, before, "DOOR_P16 ELEVATED TERMINAL launched")?;
        thread::sleep(Duration::from_secs(3));
        // The probe installs the service under C:\Program Files (as an
        // installer would) and removes it again.
        let result = run_probe(
            run,
            &record,
            &mut c,
            &probe,
            "svcprobe.exe",
            "SCM-A",
            &STEPS,
        )?;
        let text = serial(&run.root, &record)?;
        let started = text
            .lines()
            .find(|l| l.contains("DOOR_P16 SCM started name=DoorsProbe"))
            .map(str::to_owned)
            .ok_or("Init never started the service")?;
        // The crash loop: two failures with a restart each (7031), the
        // third without (7034); the recovery program after the next crash;
        // the hung stop handler timed out (7011).
        let mut recovery = Vec::new();
        for needle in [
            "DOOR_P16 SCM failure name=DoorsProbe count=1 event=7031",
            "DOOR_P16 SCM failure name=DoorsProbe count=2 event=7031",
            "DOOR_P16 SCM failure name=DoorsProbe count=3 event=7034",
            "DOOR_P16 SCM recovery program name=DoorsProbe pid=",
            "DOOR_P16 SCM control timeout name=DoorsProbe",
        ] {
            recovery.push(
                text.lines()
                    .find(|l| l.contains(needle))
                    .map(str::to_owned)
                    .ok_or(format!("{needle} missing"))?,
            );
        }
        let restarts = text
            .lines()
            .filter(|l| l.contains("DOOR_P16 SCM restarted name=DoorsProbe"))
            .count();
        if restarts != 2 {
            return Err(format!("{restarts} recovery restarts, 2 expected"));
        }
        let frame = scene::capture_settled(run, &mut c, "scm-a-terminal", &[])?;
        let shot = super::win32::keep(run, &frame, "scm-a-terminal")?;
        // The elevated cmd: the Event Log and the Task Scheduler.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(400));
        for ch in "cmd".chars() {
            super::administration::type_character(&mut c, ch)?;
        }
        key(&mut c, &["ctrl", "shift", "ret"])?;
        wait_for(run, &record, before, "DOOR_P16 UAC PROMPT")?;
        key(&mut c, &["alt", "y"])?;
        // The elevated Terminal is open already: a Command Prompt tab.
        wait_for(run, &record, before, "DOOR_P16 ELEVATED TERMINAL document")?;
        thread::sleep(Duration::from_millis(2500));
        // The service ran as LocalSystem (the service account's session),
        // not as the administrator who started it: SCM-A step 5 passes only
        // when the service recorded SYSTEM, LocalSystem's SID, an elevated
        // token, the systemprofile profile and its start arguments; its note
        // line is kept as evidence.
        let account = super::p15_connect::console_after(
            run,
            &record,
            0,
            "SCM-A note identity account_system=1 localsystem_sid=1 elevated=1 systemprofile=1 argc=3 arguments_ok=1",
            5,
        )
        .map(|_| "SCM-A note identity account_system=1 localsystem_sid=1 elevated=1 systemprofile=1 argc=3 arguments_ok=1")?;
        // The service's state changes, as Windows' event 7036.
        let system = command(
            run,
            &record,
            &mut c,
            "wevtutil qe System /c:24 /rd:true /f:text",
        )?;
        expect(&system, "  Source: Service Control Manager")?;
        expect(
            &system,
            "The Doors Probe Service service entered the stopped state.",
        )?;
        expect(
            &system,
            "The Doors Probe Service service entered the running state.",
        )?;
        expect(
            &system,
            "A timeout (30000 milliseconds) was reached while waiting for a transaction response from the Doors Probe Service service.",
        )?;
        expect(
            &system,
            "The Doors Probe Service service terminated unexpectedly.  It has done this 3 time(s).",
        )?;
        // 7031's text runs past one IPC packet: whole.
        expect(
            &system,
            "The Doors Probe Service service terminated unexpectedly.  It has done this 2 time(s).  The following corrective action will be taken in 1000 milliseconds: Restart the service.",
        )?;
        expect(
            &system,
            "The Doors Probe Service service terminated unexpectedly.  It has done this 1 time(s).  The following corrective action will be taken in 500 milliseconds: Run the configured recovery program.",
        )?;
        let created = command(
            run,
            &record,
            &mut c,
            "eventcreate /t warning /id 42 /so DoorsProbe /d \"SCM-A finished\"",
        )?;
        expect(
            &created,
            "SUCCESS: An event of type 'WARNING' was created in the 'APPLICATION' log with 'DoorsProbe' as the source.",
        )?;
        let application = command(
            run,
            &record,
            &mut c,
            "wevtutil qe Application /c:1 /rd:true /f:text",
        )?;
        expect(&application, "  Source: DoorsProbe")?;
        expect(&application, "  Event ID: 42")?;
        expect(&application, "  Level: Warning")?;
        expect(&application, "SCM-A finished")?;
        // A message longer than one packet, written and read back whole.
        let long = command(
            run,
            &record,
            &mut c,
            &format!("eventcreate /t information /id 43 /so DoorsProbe /d \"{LONG_EVENT}\""),
        )?;
        expect(
            &long,
            "SUCCESS: An event of type 'INFORMATION' was created in the 'APPLICATION' log with 'DoorsProbe' as the source.",
        )?;
        let long_read = command(
            run,
            &record,
            &mut c,
            "wevtutil qe Application /c:1 /rd:true /f:text",
        )?;
        expect(&long_read, "  Event ID: 43")?;
        expect(&long_read, LONG_EVENT)?;
        // A scheduled task: doorsvc.exe started outside the SCM ends with
        // ERROR_FAILED_SERVICE_CONTROLLER_CONNECT (1063).
        let task = command(
            run,
            &record,
            &mut c,
            "schtasks /create /tn DoorsTask /tr /system/bin/doorsvc.exe /sc daily /st 23:59 /f",
        )?;
        expect(
            &task,
            "SUCCESS: The scheduled task \"DoorsTask\" has successfully been created.",
        )?;
        let query = command(run, &record, &mut c, "schtasks /query /tn DoorsTask")?;
        if !query
            .iter()
            .any(|l| l.starts_with("DoorsTask") && l.ends_with("Ready"))
        {
            return Err(format!("schtasks /query: {query:?}"));
        }
        let before = serial(&run.root, &record)?.len();
        let ran = command(run, &record, &mut c, "schtasks /run /tn DoorsTask")?;
        expect(
            &ran,
            "SUCCESS: Attempted to run the scheduled task \"DoorsTask\".",
        )?;
        let exit = wait_for(run, &record, before, "DOOR_P16 TASK exit name=DoorsTask")?;
        if !exit.ends_with("code=1063") {
            return Err(format!("the task ended unexpectedly: {exit}"));
        }
        let deleted = command(run, &record, &mut c, "schtasks /delete /tn DoorsTask /f")?;
        expect(
            &deleted,
            "SUCCESS: The scheduled task \"DoorsTask\" was successfully deleted.",
        )?;
        let frame = scene::capture_settled(run, &mut c, "scm-a-logs-tasks", &[])?;
        let logs = super::win32::keep(run, &frame, "scm-a-logs-tasks")?;
        // Settings > Services (Run: services.msc, behind User Account
        // Control as the Management Console is): a service `sc` created
        // is listed, stopped, beside Doors' own running services.
        let created = command(
            run,
            &record,
            &mut c,
            "sc create DoorsView binPath= C:\\Windows\\System32\\msiexec.exe DisplayName= \"Doors View Service\"",
        )?;
        expect(&created, "[SC] CreateService SUCCESS")?;
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(400));
        type_line(&mut c, "services.msc")?;
        let uac = wait_for(run, &record, before, "DOOR_P16 UAC PROMPT")?;
        if !uac.contains("program=Macrohard Management Console") {
            return Err(format!("services.msc prompt: {uac}"));
        }
        thread::sleep(Duration::from_millis(800));
        key(&mut c, &["alt", "y"])?;
        let listed = wait_for(run, &record, before, "DOOR_P16 SETTINGS SERVICES")?;
        for want in ["Doors View Service: Stopped", "Doors Audio: Running"] {
            if !listed.contains(want) {
                return Err(format!("the Services page lists {listed}"));
            }
        }
        thread::sleep(Duration::from_millis(1500));
        let frame = scene::capture_settled(run, &mut c, "scm-a-settings-services", &[])?;
        let services_shot = super::win32::keep(run, &frame, "scm-a-settings-services")?;
        key(&mut c, &["alt", "f4"])?;
        thread::sleep(Duration::from_millis(1500));
        // Dependencies (DependOnService): StartService refuses a service
        // whose dependency does not exist (1075) or is disabled (1068),
        // starting nothing.
        let mut dependencies = Vec::new();
        for (line, want) in [
            (
                "sc create DoorsDepA binPath= C:\\Windows\\System32\\msiexec.exe depend= DoorsMissing",
                "[SC] CreateService SUCCESS",
            ),
            (
                "sc qc DoorsDepA",
                "        DEPENDENCIES       : DoorsMissing",
            ),
            ("sc start DoorsDepA", "[SC] StartService FAILED 1075:"),
            (
                "sc config DoorsDepA depend= DoorsView",
                "[SC] ChangeServiceConfig SUCCESS",
            ),
            // The other way round closes a cycle: refused.
            (
                "sc config DoorsView depend= DoorsDepA",
                "[SC] ChangeServiceConfig FAILED 1059:",
            ),
            (
                "sc config DoorsView start= disabled",
                "[SC] ChangeServiceConfig SUCCESS",
            ),
            ("sc start DoorsDepA", "[SC] StartService FAILED 1068:"),
            (
                "sc query DoorsView",
                "        STATE              : 1  STOPPED",
            ),
            ("sc delete DoorsDepA", "[SC] DeleteService SUCCESS"),
        ] {
            let out = command(run, &record, &mut c, line)?;
            expect(&out, want)?;
            dependencies.push(json!({"command":line,"output":out}));
        }
        let removed = command(run, &record, &mut c, "sc delete DoorsView")?;
        expect(&removed, "[SC] DeleteService SUCCESS")?;
        Ok(
            json!({"denied":denied,"probe":result,"started":started,"recovery":recovery,"restarts":restarts,
            "screenshot":shot,"system_log":system,"application_log":application,"long_event":long_read,
            "task_exit":exit,"logs_screenshot":logs,"services_page":listed,"services_screenshot":services_shot,"account":account,
            "dependencies":dependencies}),
        )
    })();
    let details = super::win32::quit_on_error(run, &record, result, "scm-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    let reboot = reboot_test(run, &staged, &probe)?;
    run.check("p16-scm-a", json!({"fixtures":[service,probe],"image":staged,"run":details,"session":record,
        "reboot":reboot,
        "scope":"SCM-A: an MSVC-built Windows service and a probe using advapi32's Service Control Manager API (create, configure, start, pause/continue, enumerate, stop, failure actions (restarting the computer refused until the shutdown privilege is enabled), a crash loop restarted twice then left stopped, a recovery program run with the failure count, a stop handler hung past 30 seconds, delete) against Init's SCM from an elevated Doorsh opened with Run's Ctrl+Shift+Enter, and what a standard user is refused (SCM-D); the System log's 7036, 7031 (whole, past one IPC packet), 7034 and 7011 events, eventcreate and wevtutil on the Application log with a message longer than one packet, schtasks create/query/run/delete with the task's exit code, Settings > Services (services.msc behind User Account Control) listing an sc-created service, and sc depend= with StartService refusing a missing (1075) or disabled (1068) dependency; then SCM-R in a second session under TCG (QEMU's WHPX freezes on a guest reset with edk2 firmware, QEMU issue 2402): a crashing service whose failure action restarts the computer, the guest restarting and signing in again (QMP keys) before a clean shutdown"}));
    Ok(())
}
/// SCM-R: a crashing service whose failure action restarts the computer,
/// in its own session on the TCG profile (QEMU's WHPX accelerator freezes
/// when a guest resets on edk2 firmware: QEMU issue 2402). The desktop
/// closes, the machine starts again, the sign-in screen comes back and the
/// second boot shuts down cleanly.
fn reboot_test(run: &mut Run, staged: &str, probe: &Value) -> Result<Value, String> {
    let disk = super::win32::fresh_disk(run, "scm-r")?;
    let mut record = session::launch_desktop_wide(run, staged, &disk, false)?;
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        thread::sleep(Duration::from_secs(5));
        // An elevated Doorsh (the probe needs an administrator's token).
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(800));
        for ch in "doorsh".chars() {
            super::administration::type_character(&mut c, ch)?;
        }
        key(&mut c, &["ctrl", "shift", "ret"])?;
        wait_long(run, &record, before, "DOOR_P16 UAC PROMPT", 120)?;
        thread::sleep(Duration::from_millis(1500));
        key(&mut c, &["alt", "y"])?;
        wait_long(
            run,
            &record,
            before,
            "DOOR_P16 ELEVATED TERMINAL launched",
            120,
        )?;
        thread::sleep(Duration::from_secs(6));
        let before = serial(&run.root, &record)?.len();
        let reboot = run_probe(
            run,
            &record,
            &mut c,
            probe,
            "svcprobe.exe reboot",
            "SCM-R",
            &["create", "privilege", "start"],
        )?;
        let failure = wait_long(
            run,
            &record,
            before,
            "DOOR_P16 SCM failure name=DoorsProbe count=1 event=7031",
            60,
        )?;
        let rebooting = wait_long(
            run,
            &record,
            before,
            "DOOR_P16 SCM reboot name=DoorsProbe",
            60,
        )?;
        let cleanup = wait_long(run, &record, before, "DOOR_P08 CLEANUP PASS", 120)?;
        let second_boot = wait_long(run, &record, before, "DOOR_LOADER v1 START", 120)?;
        let after_boot = serial(&run.root, &record)?
            .rfind("DOOR_LOADER v1 START")
            .ok_or("the second boot's loader line is missing")?;
        let surface = wait_long(run, &record, after_boot, "DOOR_P14 SIGNIN surface=", 240)?;
        thread::sleep(Duration::from_secs(2));
        let text = serial(&run.root, &record)?;
        let keys = crate::desktop::sign_in_keys(&text[after_boot..])
            .ok_or("the second boot offers no sign-in")??;
        for k in &keys {
            c.execute(
                "send-key",
                json!({"keys":[{"type":"qcode","data":k}],"hold-time":60}),
            )?;
            thread::sleep(Duration::from_millis(150));
        }
        let ready = wait_long(run, &record, after_boot, "DOOR_P11 READY", 240)?;
        Ok(
            json!({"probe":reboot,"failure":failure,"rebooting":rebooting,"cleanup":cleanup,
            "second_boot":second_boot,"sign_in":surface,"ready":ready}),
        )
    })();
    let details = super::win32::quit_on_error(run, &record, result, "scm-r-failure")?;
    // The second boot: shut down as `win32::finish` does, waiting for that
    // boot's cleanup.
    let mut c = connection(&record)?;
    thread::sleep(Duration::from_secs(5));
    let before = serial(&run.root, &record)?.len();
    start_shut_down(&mut c)?;
    wait_long(run, &record, before, "DOOR_P08 CLEANUP PASS", 120)?;
    seal_serial(&run.root, &mut record)?;
    Ok(json!({"run":details,"session":record}))
}
/// cmd's TEXT lines for one command line typed at its prompt.
fn command(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    line: &str,
) -> Result<Vec<String>, String> {
    let before = serial(&run.root, record)?.len();
    type_line(c, line)?;
    wait_for(run, record, before, "DOOR_P16 CMD IDLE")?;
    let text = serial(&run.root, record)?;
    let tail = text.get(before.min(text.len())..).unwrap_or("");
    // A MORE line continues the line before it (the terminal sends lines
    // in pieces of 160 bytes).
    let mut lines: Vec<String> = Vec::new();
    for l in tail
        .lines()
        .take_while(|l| !l.contains("DOOR_P16 CMD IDLE"))
    {
        if let Some((_, t)) = l.split_once("DOOR_P16 CMD TEXT ") {
            lines.push(t.trim_end_matches('\r').to_owned());
        } else if let Some((_, t)) = l.split_once("DOOR_P16 CMD MORE ")
            && let Some(last) = lines.last_mut()
        {
            last.push_str(t.trim_end_matches('\r'));
        }
    }
    Ok(lines.into_iter().map(|l| l.trim_end().to_owned()).collect())
}
fn expect(lines: &[String], want: &str) -> Result<(), String> {
    if lines.iter().any(|l| l == want) {
        Ok(())
    } else {
        Err(format!("{want:?} missing from {lines:?}"))
    }
}
