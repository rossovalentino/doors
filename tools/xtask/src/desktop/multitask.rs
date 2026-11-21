//! Native Win-key chords, edge placement and real workspace membership.
use super::*;
/// Settings' light window background (RGB).
/// Settings' light background: Mica over the default Bloom.
const SETTINGS_BACKGROUND: [u8; 3] = super::mica::settings(0, false);
/// A blank point of File Explorer's Home in its initial window, asserted by
/// `multitask_probe_is_blank_home_content_with_or_without_recent_files`
/// (desktop/workspace/src/files/scenario_points.rs).
const EXPLORER_HOME_BLANK: (i32, i32) = (630, 436);
fn await_capture(run: &Run, record: &Value, needle: &str) -> Result<(), String> {
    wait_serial(&run.root, record, needle, Duration::from_secs(15))
}
pub(super) fn exercise(
    run: &mut Run,
    c: &mut qmp::Client,
    p: &mut (i32, i32),
    record: &Value,
) -> Result<(), String> {
    // Settings and Explorer open at their initial placement; their probes
    // and the caption drag point come from the real placement (`window_points`).
    let (sx, sy) = window_points::settings_background()?;
    let settings = (sx as usize, sy as usize, SETTINGS_BACKGROUND);
    let (ex, ey) = window_points::screen(window_points::App::Files, EXPLORER_HOME_BLANK)?;
    let explorer = (ex as usize, ey as usize, [255, 255, 255]);
    let baseline = scene::capture(run, c, "multitask-empty", &[])?;
    let backdrop = baseline
        .pixel(1000, 52)?
        .map(|channel| (8 + (u32::from(channel) * 231 + 127) / 255) as u8);
    key(c, &["meta_l", "i"])?;
    scene::capture(run, c, "shortcut-settings", &[settings])?;
    key(c, &["meta_l", "left"])?;
    await_capture(run, record, "CAPTURED app=3 width=640 height=672")?;
    scene::capture(
        run,
        c,
        "shortcut-snap-left",
        &[(632, 100, SETTINGS_BACKGROUND)],
    )?;
    key(c, &["meta_l", "down"])?;
    scene::capture(run, c, "shortcut-unsnap", &[settings])?;
    key(c, &["meta_l", "up"])?;
    scene::capture(
        run,
        c,
        "shortcut-maximize",
        &[(1270, 100, SETTINGS_BACKGROUND)],
    )?;
    key(c, &["meta_l", "down"])?;
    scene::capture(run, c, "shortcut-restore", &[settings])?;
    // Settings caption, right of its search box and left of Minimize.
    let (dx, dy) = window_points::settings_drag()?;
    move_to(c, p, dx, dy)?;
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"btn","data":{"down":true,"button":"left"}}]}),
    )?;
    thread::sleep(Duration::from_millis(100));
    move_to(c, p, dx, 0)?;
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"btn","data":{"down":false,"button":"left"}}]}),
    )?;
    scene::capture(
        run,
        c,
        "drag-top-maximize",
        &[(1270, 100, SETTINGS_BACKGROUND)],
    )?;
    key(c, &["meta_l", "down"])?;
    scene::capture(run, c, "drag-top-restored", &[settings])?;
    key(c, &["meta_l", "d"])?;
    scene::capture(
        run,
        c,
        "shortcut-show-desktop",
        &[(
            settings.0,
            settings.1,
            baseline.pixel(settings.0, settings.1)?,
        )],
    )?;
    key(c, &["meta_l", "d"])?;
    scene::capture(run, c, "shortcut-show-desktop-restored", &[settings])?;
    key(c, &["meta_l", "ctrl", "d"])?;
    await_capture(run, record, "count=2")?;
    scene::capture(
        run,
        c,
        "workspace-two-empty",
        &[(1000, 52, baseline.pixel(1000, 52)?)],
    )?;
    key(c, &["meta_l", "e"])?;
    scene::capture(run, c, "workspace-two-files", &[explorer])?;
    key(c, &["meta_l", "tab"])?;
    await_capture(run, record, "TASK_VIEW open=1")?;
    scene::capture(run, c, "task-view-two-desktops", &[(1000, 52, backdrop)])?;
    click(c, p, 65, 598)?;
    scene::capture(run, c, "workspace-one-settings", &[settings])?;
    key(c, &["meta_l", "tab"])?;
    click(c, p, 200, 598)?;
    scene::capture(run, c, "workspace-two-files-restored", &[explorer])?;
    key(c, &["meta_l", "ctrl", "f4"])?;
    await_capture(run, record, "count=1")?;
    key(c, &["meta_l", "i"])?;
    scene::capture(run, c, "workspace-merge-settings", &[settings])?;
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"alt"}}}]}),
    )?;
    c.execute("input-send-event", json!({"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"alt_r"}}}]}))?;
    key(c, &["tab"])?;
    let switching = scene::capture(
        run,
        c,
        "alt-tab-held-real-previews",
        &[(1000, 52, backdrop)],
    )?;
    if switching.pixel(1000, 52)? != backdrop {
        return Err("Alt+Tab did not draw its held switcher".into());
    }
    c.execute("input-send-event", json!({"events":[{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"alt"}}}]}))?;
    // Releasing one physical Alt must preserve the held switcher.
    thread::sleep(Duration::from_millis(500));
    scene::capture(
        run,
        c,
        "alt-tab-second-alt-still-held",
        &[(1000, 52, backdrop)],
    )?;
    c.execute("input-send-event", json!({"events":[{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"alt_r"}}}]}))?;
    scene::capture(run, c, "alt-tab-commits-on-release", &[explorer])?;
    key(c, &["alt", "tab"])?;
    scene::capture(run, c, "alt-tab-mru-returns-settings", &[settings])?;
    key(c, &["alt", "f4"])?;
    thread::sleep(Duration::from_millis(500));
    key(c, &["meta_l", "e"])?;
    key(c, &["alt", "f4"])?;
    thread::sleep(Duration::from_millis(500));
    key(c, &["meta_l", "tab"])?;
    key(c, &["esc"])?;
    run.check("desktop-native-multitasking",json!({"snap_keyboard_and_drag":true,"show_desktop_toggle":true,"actual_desktops_created":2,"workspace_close_moves_existing_apps":true,"task_view_uses_real_snapshots":true,"alt_tab_held_until_release_mru":true,"scope":"bounded native foundation; not all Windows shortcuts or pixel fidelity"}));
    Ok(())
}
