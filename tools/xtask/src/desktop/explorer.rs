//! Real address editing/resolution over the authenticated guest filesystem.
use super::*;
fn address(
    run: &mut Run,
    c: &mut qmp::Client,
    record: &Value,
    input: &str,
    expected: &str,
) -> Result<(), String> {
    let offset = serial(&run.root, record)?.len();
    key(c, &["ctrl", "l"])?;
    type_line(c, input)?;
    wait_address(run, record, offset, expected)?;
    run.check("desktop-explorer-address", json!({"input":input,"expected":expected,"observed_serial_offset":offset,"authenticated_guest_resolution":true}));
    Ok(())
}
fn wait_address(run: &Run, record: &Value, offset: usize, expected: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let text = serial(&run.root, record)?;
        let tail = text.get(offset..).unwrap_or("");
        if tail.contains(expected) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "new address result missing: {expected}; tail={tail}"
            ));
        }
        thread::sleep(Duration::from_millis(80));
    }
}
pub(super) fn exercise(
    run: &mut Run,
    c: &mut qmp::Client,
    pointer: &mut (i32, i32),
    record: &Value,
) -> Result<(), String> {
    address(
        run,
        c,
        record,
        r"C:\Users\1001\Documents",
        "FILES ADDRESS path=/users/1001/Documents history=",
    )?;
    address(
        run,
        c,
        record,
        r"C:\uSeRs\1001\dOcUmEnTs",
        "FILES ADDRESS path=/users/1001/Documents history=",
    )?;
    address(
        run,
        c,
        record,
        r"..\Downloads",
        "FILES ADDRESS path=/users/1001/Downloads history=",
    )?;
    address(
        run,
        c,
        record,
        r"C:\Windows\System32",
        "FILES ADDRESS path=/compat/Windows/System32 history=",
    )?;
    address(
        run,
        c,
        record,
        r"%USERPROFILE%\Documents",
        "FILES ADDRESS path=/users/1001/Documents history=",
    )?;
    for input in [
        r"C:\Users\1002",
        r"C:\Users\1001\missing\..\Documents",
        r"\\server\share",
    ] {
        address(
            run,
            c,
            record,
            input,
            "FILES ADDRESS REJECT path=/users/1001/Documents history=",
        )?;
        key(c, &["esc"])?;
    }
    key(c, &["ctrl", "l"])?;
    // Repair a real typo in place with Home/Right/Delete, rather than replacing
    // the entire edit buffer. This covers the old append-only address bug.
    type_line_without_enter(c, r"CX:\Users\1001\Documents")?;
    key(c, &["home"])?;
    key(c, &["right"])?;
    key(c, &["delete"])?;
    let offset = serial(&run.root, record)?.len();
    key(c, &["ret"])?;
    wait_address(
        run,
        record,
        offset,
        "FILES ADDRESS path=/users/1001/Documents history=",
    )?;
    scene::capture_settled(run, c, "explorer-windows-address", &[])?;
    // Known folders show a one-segment path (as in Windows), so the profile
    // crumb is reached from a plain folder: This PC > Local Disk (C:) >
    // Users > 1001 > notes. Points are for the maximized 1280x672 window that
    // lifecycle.rs leaves, asserted by
    // desktop/workspace/src/files/scenario_points.rs.
    address(
        run,
        c,
        record,
        r"C:\Users\1001\notes",
        "FILES ADDRESS path=/users/1001/notes history=",
    )?;
    let offset = serial(&run.root, record)?.len();
    click(c, pointer, 611, 64)?; // "1001"
    wait_address(run, record, offset, "FILES BREADCRUMB path=/users/1001\n")?;
    scene::capture_settled(run, c, "explorer-home-breadcrumb", &[])?;
    for (icons, name, point) in [
        (true, "footer-icons", Some((1263, 659))),
        (false, "footer-details", Some((1237, 659))),
        (true, "keyboard-icons", None),
        (false, "keyboard-details", None),
    ] {
        let offset = serial(&run.root, record)?.len();
        if let Some((x, y)) = point {
            click(c, pointer, x, y)?;
        } else {
            key(c, &["ctrl", "shift", if icons { "2" } else { "6" }])?;
        }
        wait_address(
            run,
            record,
            offset,
            &format!(
                "FILES VIEW mode={} path=/users/1001\n",
                if icons { "Large" } else { "Details" }
            ),
        )?;
        // In Large icons the Pictures folder icon (its back plate's tab) spans
        // this point; in Details the same point is blank space in the first
        // row. Check real layout, not just a log.
        scene::capture_settled(
            run,
            c,
            &format!("explorer-{name}"),
            &[(
                800,
                169,
                if icons {
                    [19, 116, 207]
                } else {
                    [255, 255, 255]
                },
            )],
        )?;
    }
    run.check("desktop-explorer-view-controls",json!({"physical_footer_buttons":2,"physical_keyboard_shortcuts":2,"independent_layout_pixels":4,"same_authenticated_directory":true}));
    run.check("desktop-explorer-address-edit",json!({"home_right_delete":true,"successful_physical_return":true,"physical_home_breadcrumb":true,"rejected_paths_preserve_cwd":3,"windows_view":"existing /compat and /users aliases; no fabricated backend"}));
    // Retain the original suite's Downloads fixture and This PC toggle state.
    address(
        run,
        c,
        record,
        "shell:Downloads",
        "FILES ADDRESS path=/users/1001/Downloads history=",
    )
}
fn type_line_without_enter(c: &mut qmp::Client, text: &str) -> Result<(), String> {
    for ch in text.chars() {
        c.execute(
            "send-key",
            json!({"keys":session::keys(ch)?,"hold-time":20}),
        )?;
        thread::sleep(Duration::from_millis(70));
    }
    Ok(())
}
