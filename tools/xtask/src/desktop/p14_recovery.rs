//! P14 RECOVER-01 (`cargo xtask test desktop --suite recover-01`, spec §114
//! failure and recovery, GUI-F, RECOVER-01): a desktop shell failure keeps
//! every application, and a new shell incarnation rebuilds its state by
//! authorized enumeration; the existing compositor-restart test is kept.
//!
//! On the shell-diagnostics image variant (Windows fixtures plus
//! `/system/share/shell-diagnostics.conf`, absent from production images),
//! with real input: Notes (native) and the Win32 F2 editor run with their
//! taskbar buttons; the user creates `shell-fault` in the profile from the
//! Terminal, the armed shell crashes, Init records the exit and relaunches
//! it with backoff, the window server reports the role lost (fallback
//! taskbar from its own window list) and the new incarnation connected. The
//! kernel's records prove Notes and F2 never exited and nothing was
//! relaunched; screendumps prove F2's window and its taskbar button survived;
//! the rebuilt taskbar acts on F2 again; the "Desktop restarted"
//! notification is posted. Then the compositor-restart test runs unchanged.
use super::*;
use crate::desktop::{p14_shell as shell, win32};

/// A record field's value. The kernel prefixes user diagnostics with the
/// writer (`DOOR_USER pid=1 DOOR_P14 SHELL LAUNCHED pid=8 ...`), so fields
/// are read after the record's `DOOR_P` marker.
fn number(line: &str, key: &str) -> Option<u64> {
    line.find("DOOR_P")
        .map_or(line, |at| &line[at..])
        .split_whitespace()
        .find_map(|f| f.strip_prefix(key)?.strip_prefix('='))
        .and_then(|v| v.parse().ok())
}
pub(super) fn suite(run: &mut Run, image: &str, test_image: &str) -> Result<Value, String> {
    let (staged, staged_image) = shell::stage(run, image, true)?;
    let disk = win32::fresh_disk(run, "p14-recover")?;
    let mut record =
        session::launch_desktop_accelerated(run, &staged_image, &disk, false, win32::RESOLUTION)?;
    thread::sleep(Duration::from_secs(2));
    let result = (|| -> Result<Value, String> {
        let mut c = connection(&record)?;
        let mut p = (960, 540);
        shell::require_unit_scale(run, &record)?;
        shell::wait_line(run, &record, 0, "DOOR_P14 SHELL DIAGNOSTICS armed=1", 60)?;
        let first = shell::wait_line(run, &record, 0, "DOOR_P14 SHELL LAUNCHED pid=", 60)?;
        let old_pid = number(&first, "pid").ok_or("first shell pid")?;
        shell::wait_line(run, &record, 0, "DOOR_P14 SHELL READY ", 60)?;
        // A native app and the Win32 F2 editor, each with its button.
        let offset = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l"])?;
        thread::sleep(Duration::from_millis(400));
        type_line(&mut c, "notes")?;
        resolution::wait_activation(&run.root, &record, 2, offset)?;
        let notes_line =
            shell::wait_line(run, &record, offset, "DOOR_P12 APP_LAUNCHED app=2 pid=", 20)?;
        let notes = number(&notes_line, "pid").ok_or("Notes pid")?;
        let editor = shell::search_launch(run, &record, &mut c, &staged, "wingui-editor")?;
        shell::wait_line(
            run,
            &record,
            0,
            "DOOR_P14 SHELL TASKBAR groups=1 windows=1",
            30,
        )?;
        thread::sleep(Duration::from_secs(2));
        let (before, before_png) = shell::shot(run, &mut c, "recover-before")?;
        let slot = shell::extra_slot(1);
        if shell::icon_ink(&before, slot)? < 20 {
            return Err("F2 taskbar button absent before the fault".into());
        }
        // The user creates the fault file from the Terminal; the armed shell
        // crashes on its next poll.
        win32::open_terminal(run, &record, &mut c)?;
        let fault = serial(&run.root, &record)?.len();
        win32::terminal(
            run,
            &record,
            &mut c,
            &format!("echo crash > {}/shell-fault", win32::HOME),
        )?;
        shell::wait_line(run, &record, fault, "DOOR_P14 SHELL FAULT_INJECTED", 20)?;
        let exit = shell::wait_line(
            run,
            &record,
            fault,
            &format!("DOOR_P14 SHELL EXIT pid={old_pid} "),
            20,
        )?;
        let lost = shell::wait_line(
            run,
            &record,
            fault,
            &format!("DOOR_P14 SHELL_LOST pid={old_pid} fallback=1"),
            20,
        )?;
        let relaunched = shell::wait_line(run, &record, fault, "DOOR_P14 SHELL LAUNCHED pid=", 30)?;
        let new_pid = number(&relaunched, "pid").ok_or("new shell pid")?;
        if new_pid == old_pid || !relaunched.contains("incarnation=2") {
            return Err(format!(
                "shell was not relaunched as a new incarnation: {relaunched}"
            ));
        }
        let connected = shell::wait_line(
            run,
            &record,
            fault,
            &format!("DOOR_P14 SHELL_CONNECTED pid={new_pid} incarnation=2"),
            30,
        )?;
        if number(&connected, "windows").is_none_or(|n| n < 3) {
            return Err(format!(
                "new shell did not enumerate the live windows: {connected}"
            ));
        }
        let ready = shell::wait_line(run, &record, fault, "DOOR_P14 SHELL READY ", 30)?;
        let ready_at = serial(&run.root, &record)?
            .find(&ready)
            .ok_or("ready line")?;
        shell::wait_line(
            run,
            &record,
            ready_at,
            "DOOR_P14 SHELL TASKBAR groups=1 windows=1",
            30,
        )?;
        let notified = shell::wait_line(run, &record, fault, "DOOR_P14 NOTIFY SYSTEM id=", 10)?;
        shell::wait_line(
            run,
            &record,
            ready_at,
            "DOOR_P14 SHELL NOTICES count=1 toasts=1",
            10,
        )?;
        thread::sleep(Duration::from_secs(1));
        // Applications survived: no retirement record, no relaunch.
        let text = serial(&run.root, &record)?;
        let after = &text[fault..];
        let survivors = win32::processes(&text, fault);
        for (pid, what) in [(notes, "Notes"), (editor, "Win32 F2")] {
            let retired = after
                .lines()
                .any(|l| l.starts_with("DOOR_P08 PROCESS ") && number(l, "pid") == Some(pid));
            if retired || survivors.get(&pid).is_some_and(|p| p.exit.is_some()) {
                return Err(format!("{what} (pid {pid}) exited with the shell"));
            }
        }
        if after.contains("DOOR_P12 APP_LAUNCHED app=2 ") {
            return Err("Notes was relaunched to hide the shell failure".into());
        }
        // The window and its button are still on screen; the new shell's
        // taskbar acts on the same window.
        let (rebuilt, rebuilt_png) = shell::shot(run, &mut c, "recover-rebuilt")?;
        let ink = shell::icon_ink(&rebuilt, slot)?;
        if ink < 20 {
            return Err("F2 taskbar button not rebuilt after the shell restart".into());
        }
        let (x, y) = (
            slot.x() + slot.width() as i32 / 2,
            slot.y() + slot.height() as i32 / 2,
        );
        // F2 to the front first (Terminal is active), then minimize.
        click(&mut c, &mut p, x, y)?;
        thread::sleep(Duration::from_millis(1200));
        let (front, front_png) = shell::shot(run, &mut c, "recover-f2-front")?;
        click(&mut c, &mut p, x, y)?;
        thread::sleep(Duration::from_millis(1200));
        let (minimized, minimized_png) = shell::shot(run, &mut c, "recover-f2-minimized")?;
        let gone = win32::changed(&front, &minimized, shell::F2_CLIENT)?;
        let area = (shell::F2_CLIENT[2] * shell::F2_CLIENT[3]) as usize;
        if gone < area / 4 {
            return Err(format!(
                "rebuilt taskbar did not act on the surviving F2 window ({gone} of {area})"
            ));
        }
        win32::finish(run, &mut record, &mut c)?;
        Ok(json!({
            "first_shell": old_pid,
            "second_shell": new_pid,
            "notes_pid": notes,
            "editor_pid": editor,
            "exit": exit,
            "lost": lost,
            "connected": connected,
            "ready": ready,
            "notified": notified,
            "rebuilt_icon_pixels": ink,
            "minimize_changed": gone,
            "screens": {"before":before_png,"rebuilt":rebuilt_png,"front":front_png,"minimized":minimized_png},
        }))
    })();
    let mut evidence = win32::quit_on_error(run, &record, result, "p14-recover-failure")?;
    evidence["session"] = record;
    // The existing compositor-restart test (adversarial image): native
    // clients reconnect to a new display-service incarnation.
    super::recovery::verify(run, test_image)?;
    evidence["compositor_restart"] =
        json!({"suite":"desktop recovery (P11)","image":test_image,"outcome":"passed"});
    evidence["reference_comparison"] = json!({"state":"not_applicable",
        "scope":"Recovery behavior; Win32 reconnection across a compositor restart is implemented in user32 but not exercised by the adversarial image"});
    Ok(evidence)
}
#[cfg(test)]
mod tests {
    #[test]
    fn fields_are_read_after_the_writer_prefix() {
        let line = "DOOR_USER pid=1 DOOR_P14 SHELL LAUNCHED pid=8 incarnation=1";
        assert_eq!(super::number(line, "pid"), Some(8));
        assert_eq!(super::number(line, "incarnation"), Some(1));
        assert_eq!(
            super::number("DOOR_P14 SHELL EXIT pid=13 code=-101", "pid"),
            Some(13)
        );
        assert_eq!(super::number(line, "missing"), None);
    }
}
