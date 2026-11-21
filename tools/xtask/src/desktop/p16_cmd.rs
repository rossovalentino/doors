//! P16 cmd.exe: Run > cmd opens Terminal on a Command Prompt tab; typed
//! commands produce cmd.exe's output (read back from the engine's
//! `DOOR_P16 CMD TEXT` lines): echo, SET /A, redirection and TYPE, DIR /B,
//! FOR /L, a batch file with an argument written and run from the prompt,
//! VER, IPCONFIG and a legacy entry point opening Settings; then User
//! Account Control from Run, and File Explorer's Run as administrator.
use super::*;

const RESOLUTION: (u32, u32) = (1280, 720);
/// Wait for `needle` in serial output after `before`.
fn wait_for(run: &Run, record: &Value, before: usize, needle: &str) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(25);
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
        if text.contains("KERNEL_PANIC") || text.contains("APP FAILED") {
            return Err("guest failed; inspect serial".into());
        }
        if Instant::now() >= deadline {
            return Err(format!("{needle} missing"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}
/// Type a command line at the CMD prompt, wait until the engine is idle
/// again, and return the TEXT lines it printed.
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
    Ok(tail
        .lines()
        .take_while(|l| !l.contains("DOOR_P16 CMD IDLE"))
        .filter_map(|l| {
            l.split_once("DOOR_P16 CMD TEXT ")
                .map(|(_, t)| t.trim_end().to_owned())
        })
        .collect())
}
/// File Explorer's address bar to `location`; Home selects the first item.
fn explorer_to(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    location: &str,
    shown: &str,
) -> Result<(), String> {
    let before = serial(&run.root, record)?.len();
    key(c, &["ctrl", "l"])?;
    thread::sleep(Duration::from_millis(300));
    type_line(c, location)?;
    let line = wait_for(run, record, before, "DOOR_P12 FILES ADDRESS")?;
    if !line.contains(shown) {
        return Err(format!("File Explorer went elsewhere: {line}"));
    }
    thread::sleep(Duration::from_millis(800));
    key(c, &["home"])?;
    thread::sleep(Duration::from_millis(300));
    Ok(())
}
/// The selected item's context menu (Shift+F10 focuses Open) > Run as
/// administrator, `row` commands below Open (a batch file lists Open with
/// first); returns the serial offset and the prompt's line.
fn run_as_administrator(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    row: usize,
    path: &str,
) -> Result<(usize, String), String> {
    let before = serial(&run.root, record)?.len();
    key(c, &["shift", "f10"])?;
    thread::sleep(Duration::from_millis(600));
    for _ in 0..row {
        key(c, &["down"])?;
    }
    key(c, &["ret"])?;
    let request = wait_for(run, record, before, "DOOR_P16 FILES RUNAS")?;
    if !request.ends_with(path) {
        return Err(format!("Run as administrator on the wrong item: {request}"));
    }
    Ok((
        before,
        wait_for(run, record, before, "DOOR_P16 UAC PROMPT")?,
    ))
}
fn expect(lines: &[String], want: &str, what: &str) -> Result<(), String> {
    if lines.iter().any(|l| l == want) {
        Ok(())
    } else {
        Err(format!("{what}: expected a line {want:?}, got {lines:?}"))
    }
}
pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let disk = super::win32::fresh_disk(run, "p16-cmd")?;
    let resolution = format!("{}x{}", RESOLUTION.0, RESOLUTION.1);
    let mut record = session::launch_desktop_accelerated(run, image, &disk, false, &resolution)?;
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        thread::sleep(Duration::from_secs(2));
        // Run > cmd: Terminal opens on a Command Prompt tab.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(400));
        type_line(&mut c, "cmd")?;
        wait_for(run, &record, before, "DOCUMENT_OPEN app=9")?;
        // Terminal has the keyboard before the first line is typed.
        super::resolution::wait_activation(&run.root, &record, 9, before)?;
        thread::sleep(Duration::from_millis(2500));
        let mut observed = serde_json::Map::new();
        let echo = command(run, &record, &mut c, "echo hello doors")?;
        expect(&echo, "hello doors", "echo")?;
        let math = command(run, &record, &mut c, "set /a 6*7")?;
        expect(&math, "42", "set /a")?;
        command(run, &record, &mut c, "md p16test")?;
        command(run, &record, &mut c, "cd p16test")?;
        command(run, &record, &mut c, "echo one> a.txt & echo two>>a.txt")?;
        let typed = command(run, &record, &mut c, "type a.txt")?;
        expect(&typed, "one", "type")?;
        expect(&typed, "two", "type")?;
        let listing = command(run, &record, &mut c, "dir /b")?;
        expect(&listing, "a.txt", "dir /b")?;
        let looped = command(run, &record, &mut c, "for /l %i in (1,1,3) do @echo n%i")?;
        for n in ["n1", "n2", "n3"] {
            expect(&looped, n, "for /l")?;
        }
        command(run, &record, &mut c, "echo @echo off> t.bat")?;
        command(run, &record, &mut c, "echo echo batch %1 [%~x0]>> t.bat")?;
        let batch = command(run, &record, &mut c, "t.bat works")?;
        expect(&batch, "batch works [.bat]", "batch file")?;
        let ver = command(run, &record, &mut c, "ver")?;
        expect(&ver, "Macrohard Doors [Version 10.0.26300.1]", "ver")?;
        let ip = command(run, &record, &mut c, "ipconfig")?;
        if !ip
            .iter()
            .any(|l| l.contains("IPv4 Address. . . . . . . . . . . : 10.0.2.15"))
        {
            return Err(format!("ipconfig: no IPv4 line in {ip:?}"));
        }
        let missing = command(run, &record, &mut c, "nothere_cmd")?;
        expect(
            &missing,
            "'nothere_cmd' is not recognized as an internal or external command,",
            "unknown command",
        )?;
        let level = command(run, &record, &mut c, "echo %errorlevel%")?;
        expect(&level, "9009", "errorlevel")?;
        scene::capture_settled(run, &mut c, "p16-cmd", &[])?;
        // F11: Terminal goes full screen over the taskbar, and back.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["f11"])?;
        wait_for(run, &record, before, "DOOR_P16 FULLSCREEN")?;
        thread::sleep(Duration::from_millis(1200));
        scene::capture_settled(run, &mut c, "p16-fullscreen", &[])?;
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["f11"])?;
        let back = wait_for(run, &record, before, "DOOR_P16 FULLSCREEN")?;
        if !back.contains("on=0") {
            return Err(format!("full screen did not end: {back}"));
        }
        thread::sleep(Duration::from_millis(800));
        // A legacy entry point opens its Settings page.
        let before = serial(&run.root, &record)?.len();
        type_line(&mut c, "control printers")?;
        wait_for(run, &record, before, "DOOR_P16 CMD LAUNCH Settings")?;
        wait_for(run, &record, before, "SETTINGS PAGE id=")?;
        thread::sleep(Duration::from_millis(1500));
        // Run > regedit: User Account Control asks first; Alt+Y approves
        // and Settings opens the Registry Editor page.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(400));
        type_line(&mut c, "regedit")?;
        let uac = wait_for(run, &record, before, "DOOR_P16 UAC PROMPT")?;
        if !uac.contains("program=Registry Editor publisher=Macrohard Doors") {
            return Err(format!("unexpected prompt: {uac}"));
        }
        thread::sleep(Duration::from_millis(1000));
        scene::capture_settled(run, &mut c, "p16-uac", &[])?;
        key(&mut c, &["alt", "y"])?;
        wait_for(run, &record, before, "DOOR_P16 UAC ANSWER yes")?;
        wait_for(run, &record, before, "SETTINGS PAGE id=")?;
        thread::sleep(Duration::from_millis(1500));
        // Doors runs at most 13 processes, the session nine of them; Tiny
        // Basic (elevated, waiting for input) and the elevated Terminal
        // below need the room, so Settings closes.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["alt", "f4"])?;
        wait_for(run, &record, before, "CLIENT_EXIT")?;
        thread::sleep(Duration::from_millis(800));
        // Search > doorsh opens Terminal on a Doorsh tab.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l"])?;
        thread::sleep(Duration::from_millis(600));
        type_line(&mut c, "doorsh")?;
        wait_for(run, &record, before, "DOCUMENT_OPEN app=9")?;
        thread::sleep(Duration::from_millis(1500));
        // Doorsh: expressions, a structured pipeline and a cmd.exe tool.
        for (line, want) in [
            ("echo (6 * 7)", "42"),
            ("ls | where type == folder | count", ""),
            ("hostname | lower", ""),
        ] {
            let before = serial(&run.root, &record)?.len();
            type_line(&mut c, line)?;
            wait_for(run, &record, before, "DOOR_P16 DSH IDLE status=0")?;
            if !want.is_empty() {
                let text = serial(&run.root, &record)?;
                let tail = text.get(before.min(text.len())..).unwrap_or("");
                if !tail
                    .lines()
                    .any(|l| l.ends_with(&format!("DOOR_P16 DSH TEXT {want}")))
                {
                    return Err(format!("doorsh: {line} did not print {want}"));
                }
            }
        }
        scene::capture_settled(run, &mut c, "p16-doorsh", &[])?;
        // File Explorer > Run as administrator on a program: the window
        // server verifies it itself (Tiny Basic is unsigned, so the
        // unknown-publisher prompt) and on Alt+Y has Init start exactly the
        // bytes it verified.
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "e"])?;
        resolution::wait_activation(&run.root, &record, 1, before)?;
        explorer_to(run, &record, &mut c, "/system/bin", "path=/system/bin ")?;
        let program = run_as_administrator(run, &record, &mut c, 1, "/system/bin/tinybasic.exe")?;
        let trust = wait_for(
            run,
            &record,
            program.0,
            "DOOR_P16 AUTHENTICODE path=/system/bin/tinybasic.exe",
        )?;
        if !trust.contains("trust=0x800B0100 publisher=Unknown")
            || !program.1.contains("publisher=Unknown")
        {
            return Err(format!(
                "an unsigned program must get the unknown-publisher prompt: {trust} / {}",
                program.1
            ));
        }
        thread::sleep(Duration::from_millis(1000));
        scene::capture_settled(run, &mut c, "p16-runas-unknown-publisher", &[])?;
        key(&mut c, &["alt", "y"])?;
        wait_for(
            run,
            &record,
            program.0,
            "DOOR_P16 ELEVATED LAUNCH image verified",
        )?;
        let started = wait_for(
            run,
            &record,
            program.0,
            "DOOR_P16 ELEVATED LAUNCH started path=/system/bin/tinybasic.exe",
        )?;
        // Run as administrator on a batch file: Doors Command Processor is
        // what elevates, and an administrator Terminal tab runs the file.
        explorer_to(run, &record, &mut c, "~\\p16test", "p16test ")?;
        key(&mut c, &["down"])?; // a.txt, then t.bat
        let script = run_as_administrator(run, &record, &mut c, 2, "/p16test/t.bat")?;
        if !script
            .1
            .contains("program=Doors Command Processor publisher=Macrohard Doors")
        {
            return Err(format!("batch prompt: {}", script.1));
        }
        key(&mut c, &["alt", "y"])?;
        wait_for(run, &record, script.0, "DOOR_P16 ELEVATED SCRIPT")?;
        let ran = wait_for(run, &record, script.0, "DOOR_P16 CMD TEXT batch")?;
        if !ran.contains("[.bat]") {
            return Err(format!("elevated batch output: {ran}"));
        }
        thread::sleep(Duration::from_millis(1500));
        scene::capture_settled(run, &mut c, "p16-runas-batch", &[])?;
        observed.insert("runas_program".into(), json!([trust, program.1, started]));
        observed.insert("runas_batch".into(), json!([script.1, ran]));
        observed.insert("uac".into(), json!(uac));
        start_shut_down(&mut c)?;
        wait_serial(
            &run.root,
            &record,
            "DOOR_P08 CLEANUP PASS",
            Duration::from_secs(40),
        )?;
        observed.insert("echo".into(), json!(echo));
        observed.insert("set_a".into(), json!(math));
        observed.insert("type".into(), json!(typed));
        observed.insert("for".into(), json!(looped));
        observed.insert("batch".into(), json!(batch));
        observed.insert("ver".into(), json!(ver));
        observed.insert("ipconfig".into(), json!(ip));
        Ok(Value::Object(observed))
    })();
    let summary = super::win32::quit_on_error(run, &record, result, "p16-cmd-failure")?;
    seal_serial(&run.root, &mut record)?;
    run.check(
        "p16-cmd-terminal",
        json!({"session":record,"observed":summary,"scope":"Run > cmd opens Terminal's Command Prompt tab; echo, SET /A, redirection with TYPE, DIR /B, FOR /L, a batch file with %1 and %~x0, VER, IPCONFIG, the 9009 not-recognized error, control printers opening Settings, F11 full screen, Run > regedit behind the User Account Control prompt, Start search > doorsh, Doorsh expressions and pipelines, and File Explorer Run as administrator on an unsigned program (server-verified unknown-publisher prompt, Init launch bound to the image hash) and on a batch file (administrator Terminal tab)"}),
    );
    Ok(())
}
