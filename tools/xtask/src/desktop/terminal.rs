//! Actual Terminal input, captured native pages and independent disk audit.
use super::*;
pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let disk = run.directory.join("terminal-private.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut sessions = Vec::new();
    for boot in 0..2 {
        let mut record =
            session::launch_desktop_accelerated(run, image, &disk, false, "1920x1080")?;
        let result: Result<(), String> = (|| {
            let mut c = connection(&record)?;
            let mut p = (960, 540);
            thread::sleep(Duration::from_secs(2));
            let before = serial(&run.root, &record)?.len();
            // Start opens with focus in its search box; Enter opens Best match.
            key(&mut c, &["meta_l"])?;
            type_line(&mut c, "terminal")?;
            resolution::wait_activation(&run.root, &record, 9, before)?;
            if boot == 0 {
                scene::capture_settled(run, &mut c, "terminal-reference-window", &[])?;
            }
            key(&mut c, &["meta_l", "up"])?;
            if boot == 0 {
                command_result(
                    run,
                    &record,
                    &mut c,
                    "echo rejected > ../../../blocked.txt",
                    false,
                )?;
                for line in [
                    "pwd",
                    "mkdir terminal-test",
                    "cd terminal-test",
                    "echo native-proof > first.txt",
                    "append first.txt second-line",
                    "cat first.txt",
                    "cp first.txt copy.txt",
                    "mv copy.txt final.txt",
                    "stat final.txt",
                    "rm first.txt",
                    "ls",
                    "cd ..",
                ] {
                    command(run, &record, &mut c, line)?;
                }
                scene::capture_settled(run, &mut c, "terminal-home-native", &[])?;
                key(&mut c, &["ctrl", "t"])?;
                command(run, &record, &mut c, "pwd")?;
                key(&mut c, &["ctrl", "shift", "tab"])?;
                key(&mut c, &["ctrl", "f"])?;
                type_line(&mut c, "native-proof")?;
                scene::capture_settled(run, &mut c, "terminal-find", &[])?;
                key(&mut c, &["esc"])?;
                for (y, name) in [
                    (204, "cmd"),
                    (244, "powershell"),
                    (284, "dsl"),
                    (164, "doorsh"),
                ] {
                    click(&mut c, &mut p, 100, y)?;
                    scene::capture_settled(run, &mut c, &format!("terminal-profile-{name}"), &[])?;
                }
                click(&mut c, &mut p, 100, 324)?;
                scene::capture_settled(run, &mut c, "terminal-profiles", &[])?;
                click(&mut c, &mut p, 100, 424)?;
                scene::capture_settled(run, &mut c, "terminal-about", &[])?;
                click(&mut c, &mut p, 100, 384)?;
                click(&mut c, &mut p, 600, 166)?;
                click(&mut c, &mut p, 600, 250)?;
                key(&mut c, &["ctrl", "s"])?;
                scene::capture_settled(run, &mut c, "terminal-settings-saved", &[])?;
                // Live application theme changes must reach the existing Terminal,
                // without changing its separately saved console color scheme.
                for (dark, theme, color) in [(true, 1, [32, 32, 32]), (false, 0, [244, 247, 250])] {
                    resolution::activate(&run.root, &record, &mut c, 2)?;
                    settings_mode(&mut c, dark)?;
                    wait_serial(
                        &run.root,
                        &record,
                        &format!("SETTINGS SAVED theme={theme}"),
                        Duration::from_secs(15),
                    )?;
                    let before = serial(&run.root, &record)?.len();
                    key(&mut c, &["meta_l"])?;
                    type_line(&mut c, "terminal")?;
                    resolution::wait_activation(&run.root, &record, 9, before)?;
                    scene::capture_settled(
                        run,
                        &mut c,
                        if dark {
                            "terminal-settings-dark"
                        } else {
                            "terminal-settings-light"
                        },
                        &[(220, 110, color)],
                    )?;
                }
                // First caption tab: Explorer strip from x 8 (stride <= 294,
                // close box at its right 32 px), so x 250 selects tab 0.
                click(&mut c, &mut p, 250, 28)?;
                key(&mut c, &["ctrl", "w"])?;
                scene::capture_settled(run, &mut c, "terminal-tab-close", &[])?;
            } else {
                command(run, &record, &mut c, "cat terminal-test/final.txt")?;
                scene::capture_settled(run, &mut c, "terminal-reopened-home", &[])?;
                click(&mut c, &mut p, 100, 344)?;
                scene::capture_settled(run, &mut c, "terminal-reopened-settings", &[])?;
            }
            key(&mut c, &["alt", "f4"])?;
            start_shut_down(&mut c)?;
            wait_serial(
                &run.root,
                &record,
                "DOOR_P08 CLEANUP PASS",
                Duration::from_secs(25),
            )?;
            Ok(())
        })();
        if result.is_err()
            && let Ok(mut c) = connection(&record)
        {
            let _ = scene::capture_settled(run, &mut c, "terminal-failure", &[]);
            let _ = c.execute("quit", json!({}));
        }
        result?;
        seal_serial(&run.root, &mut record)?;
        shutdown::retirement_commands(
            &serial(&run.root, &record)?,
            if boot == 0 { &[9, 3] } else { &[9] },
        )?;
        sessions.push(record);
        audit(&disk)?;
    }
    run.check("terminal-native-preview", json!({"sessions":sessions,"audit":audit(&disk)?,"disk":relative(&run.root,&disk)?,"disk_sha256":file_hash(&disk)?,"owner_volume_used":false,"scope":"Native builtins and checked preferences only; no CMD/PowerShell/WSL engine or 1:1 fidelity qualification"}));
    Ok(())
}
fn command(run: &Run, record: &Value, c: &mut qmp::Client, line: &str) -> Result<(), String> {
    command_result(run, record, c, line, true)
}
fn command_result(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    line: &str,
    success: bool,
) -> Result<(), String> {
    let offset = serial(&run.root, record)?.len();
    type_line(c, line)?;
    let expected = format!(
        "TERMINAL COMMAND name={} ok={}",
        line.split_whitespace().next().ok_or("empty test command")?,
        u8::from(success)
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    while !serial(&run.root, record)?
        .get(offset..)
        .unwrap_or("")
        .contains(&expected)
    {
        if Instant::now() >= deadline {
            return Err(format!("Terminal command completion missing: {expected}"));
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
fn audit(disk: &std::path::Path) -> Result<Value, String> {
    let raw = fs::read(disk).map_err(|e| e.to_string())?;
    let journal = crate::persistence::journal::inspect(&raw)?;
    if journal.selected().committed {
        return Err("terminal journal not clean".into());
    }
    let tree = crate::persistence::tree::inspect(&journal.replayed_home(&raw)?)?;
    let catalog = crate::persistence::catalog::decode(&tree)?.ok_or("terminal catalog absent")?;
    let paths = catalog.paths()?;
    let node = paths
        .get("/users/1001/terminal-test/final.txt")
        .ok_or("terminal proof file absent")?;
    let file = tree
        .files
        .get(&format!("obj{:016x}", node.id))
        .ok_or("terminal proof backing absent")?;
    if file.bytes != b"native-proof\nsecond-line\n" {
        return Err("terminal proof bytes differ".into());
    }
    if paths.contains_key("/users/1001/terminal-test/first.txt") {
        return Err("terminal removed file remains".into());
    }
    let preferences_node = paths
        .get("/users/1001/terminal.conf")
        .ok_or("terminal preferences absent")?;
    let preferences = tree
        .files
        .get(&format!("obj{:016x}", preferences_node.id))
        .ok_or("terminal preferences backing absent")?;
    if preferences.bytes != [b'D', b'T', b'R', b'M', 2, 1, 2, 15] {
        return Err("terminal preference bytes differ".into());
    }
    Ok(
        json!({"native_file_bytes":true,"removed_file_absent":true,"preferences_exact":true,"journal_clean":true}),
    )
}
