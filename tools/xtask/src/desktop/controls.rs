//! Actual Settings input, accepted RPC diagnostics and composed work-area pixels.
//! Runs at the end of the wide profile; it restores every preference bit (48).
use super::*;

fn page(c: &mut qmp::Client, query: &str) -> Result<(), String> {
    key(c, &["esc"])?;
    key(c, &["ctrl", "f"])?;
    type_line(c, query)?;
    key(c, &["esc"])
}
fn accepted(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    keys: &[&str],
    bits: u32,
    page: &str,
) -> Result<(), String> {
    let before = serial(&run.root, record)?.len();
    key(c, keys)?;
    let marker = format!("SETTINGS CONTROLS bits={bits} page={page}");
    let applied = format!("SETTINGS APPLIED controls={bits}");
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let text = serial(&run.root, record)?;
        if text[before..].contains(&marker) && text[before..].contains(&applied) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("fresh accepted and applied {marker} missing"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}
pub(super) fn exercise(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    p: &mut (i32, i32),
) -> Result<(), String> {
    // Key sequences and points are asserted by the host test
    // `controls_scenario_taskbar_and_context_menu_key_sequences`
    // (desktop/workspace/tests/settings_reference_painter.rs).
    key(c, &["meta_l", "up"])?;
    // Settings search always reloads the page, so Taskbar behaviors starts
    // collapsed: Tab x5 reaches its header (past Taskbar items: Search,
    // Task view), Space expands it, Tab x2 reaches
    // the 26H2 "Taskbar position" combo (Bottom/Top/Left/Right).
    page(c, "taskbar")?;
    for _ in 0..5 {
        key(c, &["tab"])?;
    }
    key(c, &["spc"])?;
    key(c, &["tab"])?;
    key(c, &["tab"])?;
    for edge in 0..4 {
        if edge == 0 {
            // Re-choose Bottom through the list for a fresh marker.
            key(c, &["ret"])?;
            accepted(run, record, c, &["ret"], 48, "Taskbar")?;
        } else {
            // A closed combo steps to the next position (WinUI).
            accepted(run, record, c, &["down"], 48 + edge, "Taskbar")?;
        }
        key(c, &["meta_l", "up"])?;
        move_to(c, p, 1100, 600)?;
        let (work_x, work_y, bar_x, bar_y) = match edge {
            0 => (0, 0, 100, 700),
            1 => (0, 48, 100, 24),
            2 => (48, 0, 24, 360),
            _ => (0, 0, 1256, 360),
        };
        // Light Settings background (Mica over the default Bloom) at window
        // (300, 16), left of the search box.
        let settings = super::mica::settings(0, false);
        let shot = scene::capture_settled(
            run,
            c,
            &format!("controls-edge-{edge}"),
            &[(work_x + 300, work_y + 16, settings)],
        )?;
        let output = (shot.width, shot.height);
        if scene::probe_matches_at(settings, shot.pixel(bar_x, bar_y)?, (bar_x, bar_y), output) {
            return Err("maximized Settings covered the reserved taskbar edge".into());
        }
        key(c, &["meta_l"])?;
        scene::capture_settled(
            run,
            c,
            &format!("controls-start-edge-{edge}"),
            &[start_probe(1280, 720, 48 + edge)?],
        )?;
        key(c, &["esc"])?;
    }
    // Alignment is just above position: Up selects Left (Top on side edges).
    key(c, &["shift", "tab"])?;
    accepted(run, record, c, &["up"], 55, "Taskbar")?;
    // Taskbar size (26H2) is just below position: Up selects Small.
    key(c, &["tab"])?;
    key(c, &["tab"])?;
    accepted(run, record, c, &["up"], 63, "Taskbar")?;
    key(c, &["meta_l"])?;
    scene::capture_settled(
        run,
        c,
        "controls-compact-start-aligned",
        &[start_probe(1280, 720, 63)?],
    )?;
    key(c, &["esc"])?;
    // 26H2 Context menu order: File actions, Show more options, Properties
    // placement (combo), Arrange primary actions inline, classic menu.
    page(c, "context menu")?;
    key(c, &["tab"])?;
    accepted(run, record, c, &["spc"], 63 | (1 << 20), "ContextMenu")?;
    accepted(run, record, c, &["spc"], 63, "ContextMenu")?;
    key(c, &["tab"])?;
    accepted(run, record, c, &["spc"], 63 | (1 << 21), "ContextMenu")?;
    accepted(run, record, c, &["spc"], 63, "ContextMenu")?;
    key(c, &["tab"])?;
    accepted(run, record, c, &["up"], 31, "ContextMenu")?; // Properties at the top.
    key(c, &["tab"])?;
    accepted(run, record, c, &["spc"], 15, "ContextMenu")?; // Inline actions off.
    scene::capture_settled(run, c, "controls-context-properties-off", &[])?;
    accepted(run, record, c, &["spc"], 31, "ContextMenu")?;
    key(c, &["shift", "tab"])?;
    accepted(run, record, c, &["down"], 63, "ContextMenu")?; // Properties at the bottom.
    // Reset through the same controls; Taskbar behaviors is collapsed again.
    page(c, "taskbar")?;
    for _ in 0..5 {
        key(c, &["tab"])?;
    }
    key(c, &["spc"])?;
    key(c, &["tab"])?;
    key(c, &["tab"])?;
    key(c, &["ret"])?; // Position list, current Right.
    for _ in 0..3 {
        key(c, &["up"])?;
    }
    accepted(run, record, c, &["ret"], 60, "Taskbar")?; // Bottom.
    key(c, &["shift", "tab"])?;
    accepted(run, record, c, &["down"], 56, "Taskbar")?; // Center.
    key(c, &["tab"])?;
    key(c, &["tab"])?;
    accepted(run, record, c, &["down"], 48, "Taskbar")?; // Default size.
    scene::capture_settled(run, c, "controls-default-restored", &[])?;
    let before_explorer = serial(&run.root, record)?.len();
    key(c, &["meta_l", "e"])?;
    resolution::wait_activation(&run.root, record, 1, before_explorer)?;
    key(c, &["shift", "f10"])?;
    // Explorer opens to Home with nothing selected, so Shift+F10 opens the
    // background menu; its 26H2 tail: Properties, Show more options, Customize menu;
    // Up wraps from the first command to the last entry.
    for _ in 0..3 {
        key(c, &["up"])?;
    }
    key(c, &["ret"])?;
    wait_serial(
        &run.root,
        record,
        "DOOR_P12 FILES PROPERTIES",
        Duration::from_secs(15),
    )?;
    scene::capture_settled(run, c, "controls-files-properties", &[])?;
    key(c, &["esc"])?;
    key(c, &["shift", "f10"])?;
    key(c, &["up"])?;
    let before = serial(&run.root, record)?.len();
    key(c, &["ret"])?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let text = serial(&run.root, record)?;
        if text[before..].contains("SETTINGS PAGE id=16") {
            break;
        }
        if Instant::now() >= deadline {
            return Err(
                "Explorer Customize menu did not navigate the real Settings singleton".into(),
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
    scene::capture_settled(run, c, "controls-customize-settings", &[])?;
    run.check("desktop-settings-controls",json!({"positions":["bottom","top","left","right"],"maximized_window_reserves_each_edge":true,"start_anchors_captured":4,"compact_and_alignment":true,"context_inline_and_properties_toggles":true,"context_file_actions_and_more_options":true,"taskbar_size_small":true,"files_authenticated_metadata":true,"customize_navigates_real_settings":true,"fresh_accepted_and_applied_diagnostics":true,"restored_bits":48,"scope":"actual supported subset; no pixel fidelity or frame-rate claim"}));
    Ok(())
}
