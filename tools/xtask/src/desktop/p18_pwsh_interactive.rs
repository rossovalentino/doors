//! P18 PWSH-B (M1): interactive PowerShell 7 in the unified Terminal, as a
//! user reaches it on Windows 11: Start search for "pwsh" offers
//! PowerShell, and opening it starts Terminal on a PowerShell tab running
//! pwsh.exe in the tab's console grid (no console window of its own).
//! PSReadLine reads the keys typed (ReadConsoleInputW) and draws the prompt
//! and the edited line with VT sequences, asking for the cursor as it
//! writes. Typed commands run and print; tab completion completes a cmdlet
//! name; Up recalls history; Ctrl+C at the prompt cancels the edited line
//! without ending pwsh; `exit` ends pwsh and its tab. Screendumps record the
//! grid (colours, cursor) for the comparison with Windows Terminal.
use super::*;

const TAR: &str = "C:/Windows/System32/tar.exe";
const PWSH_ZIP: &str = "out/acquisitions/powershell-7.7.0/PowerShell-7.7.0-win-x64.zip";
const RESOLUTION: &str = "1280x720";
const DISK_BYTES: u64 = 3 << 30;

/// Wait until the console transcript after `from` contains `needle`.
fn wait_for(
    run: &Run,
    record: &Value,
    from: usize,
    needle: &str,
    seconds: u64,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let text = session::transcript(&run.root, record)?;
        let rest = text.get(from.min(text.len())..).unwrap_or("").to_owned();
        if rest.contains(needle) {
            return Ok(rest);
        }
        if serial(&run.root, record)?.contains("KERNEL_PANIC") {
            return Err(format!("guest panicked; console: {rest}"));
        }
        if Instant::now() >= deadline {
            return Err(format!("{needle:?} did not appear; console: {rest}"));
        }
        thread::sleep(Duration::from_millis(250));
    }
}
/// Wait for a serial line containing `needle` after `from`.
fn wait_serial(
    run: &Run,
    record: &Value,
    from: usize,
    needle: &str,
    seconds: u64,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let log = serial(&run.root, record)?;
        if log
            .get(from.min(log.len())..)
            .unwrap_or("")
            .contains(needle)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("{needle:?} did not appear in the serial log"));
        }
        thread::sleep(Duration::from_millis(250));
    }
}
fn screendump(run: &Run, c: &mut qmp::Client, name: &str) -> Result<String, String> {
    let path = run.directory.join(format!("{name}.ppm"));
    c.execute(
        "screendump",
        json!({"filename":path.to_string_lossy().trim_start_matches("\\\\?\\").replace('\\',"/")}),
    )?;
    thread::sleep(Duration::from_millis(300));
    Ok(path.display().to_string())
}
fn type_text(c: &mut qmp::Client, text: &str) -> Result<(), String> {
    for ch in text.chars() {
        c.execute(
            "send-key",
            json!({"keys":session::keys(ch)?,"hold-time":20}),
        )?;
        thread::sleep(Duration::from_millis(70));
    }
    Ok(())
}

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/pwsh-b", run.id))?;
    let pwsh = target.join("pwsh");
    fs::create_dir_all(&pwsh).map_err(|e| e.to_string())?;
    let native = |p: &std::path::Path| crate::windows::native_path(p);
    let zip = paths::existing(&run.root, PWSH_ZIP)?;
    run.required(TAR, &["-xf", &native(&zip)?, "-C", &native(&pwsh)?])?;
    let disk = run.directory.join("pwsh-b-user.raw");
    crate::persistence::large::write(&disk, DISK_BYTES)?;
    let items = crate::persistence::stage::tree(&pwsh, "/compat/Program Files/PowerShell/7")?;
    let staged = crate::persistence::stage::stage(&disk, &items)?;
    let image = super::p15_connect::stage(
        run,
        image,
        &[],
        &target,
        "desktop-pwsh-b",
        &[
            "windows/ntdll.dll",
            "windows/kernel32.dll",
            "windows/ucrtbase.dll",
            "windows/advapi32.dll",
            "windows/ole32.dll",
            "windows/bcrypt.dll",
            "windows/version.dll",
            "windows/secur32.dll",
            "windows/amsi.dll",
            "windows/wldp.dll",
            "windows/crypt32.dll",
        ],
    )?;
    let mut record = session::launch_desktop_accelerated_v2(run, &image, &disk, false, RESOLUTION)?;
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        thread::sleep(Duration::from_secs(3));
        let mut steps = Vec::new();
        // Start search: "pwsh" is PowerShell; Enter opens Terminal on a
        // PowerShell tab, which starts pwsh.exe in its grid.
        let before = serial(&run.root, &record)?.len();
        let text_before = session::transcript(&run.root, &record)?.len();
        key(&mut c, &["meta_l"])?;
        type_line(&mut c, "pwsh")?;
        super::resolution::wait_activation(&run.root, &record, 9, before)?;
        steps.push("search".to_owned());
        wait_for(run, &record, text_before, "PS ", 900)?;
        thread::sleep(Duration::from_secs(2));
        steps.push("prompt".to_owned());
        let mut shots = vec![screendump(run, &mut c, "pwsh-b-prompt")?];
        // A command and its output.
        let at = session::transcript(&run.root, &record)?.len();
        type_line(&mut c, "'PWSH-B ' + (6 * 7)")?;
        wait_for(run, &record, at, "PWSH-B 42", 180)?;
        steps.push("expression".to_owned());
        // Tab completion of a cmdlet name.
        let at = session::transcript(&run.root, &record)?.len();
        type_text(&mut c, "Get-Dat")?;
        key(&mut c, &["tab"])?;
        thread::sleep(Duration::from_secs(2));
        // .NET custom formats read H as the hour: the marker is quoted.
        type_line(&mut c, " -Year 2026 -Format \"'PWSH-B' yyyy\"")?;
        wait_for(run, &record, at, "PWSH-B 2026", 180)?;
        steps.push("completion".to_owned());
        // History: Up twice recalls the expression.
        let at = session::transcript(&run.root, &record)?.len();
        key(&mut c, &["up"])?;
        key(&mut c, &["up"])?;
        key(&mut c, &["ret"])?;
        wait_for(run, &record, at, "PWSH-B 42", 180)?;
        steps.push("history".to_owned());
        // Ctrl+C at the prompt cancels the line; pwsh stays.
        type_text(&mut c, "abandoned")?;
        key(&mut c, &["ctrl", "c"])?;
        thread::sleep(Duration::from_secs(1));
        let at = session::transcript(&run.root, &record)?.len();
        type_line(&mut c, "'PWSH-B ' + 'alive'")?;
        wait_for(run, &record, at, "PWSH-B alive", 180)?;
        steps.push("cancel".to_owned());
        shots.push(screendump(run, &mut c, "pwsh-b-session")?);
        // exit ends pwsh and its tab.
        let before = serial(&run.root, &record)?.len();
        type_line(&mut c, "exit")?;
        wait_serial(
            run,
            &record,
            before,
            "DOOR_P18 TERMINAL HOSTED name=pwsh.exe exit=0x0",
            120,
        )?;
        steps.push("exit".to_owned());
        Ok(json!({"steps":steps,"screenshots":shots}))
    })();
    let details = super::win32::quit_on_error(run, &record, result, "pwsh-b-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p18-pwsh-b", json!({"image":image,"disk":{"bytes":DISK_BYTES,"staged":staged},
        "powershell":{"zip":PWSH_ZIP,"sha256":file_hash(&zip)?},
        "run":details,"session":record,
        "reference":"the same keys in Windows Terminal running PowerShell 7.7.0 on the owner's PC",
        "scope":"PWSH-B: Start search 'pwsh' opens Terminal on a PowerShell tab; interactive pwsh in the tab's console grid: PSReadLine prompt and rendering, typed commands, tab completion, history, Ctrl+C cancelling the edited line, exit closing the tab"}));
    Ok(())
}
