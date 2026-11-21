//! Actual Files rename input, durable namespace and independent two-boot audit.
use super::*;
// Window points of Explorer, logical (times `scale`; the accelerated profile
// is the same 1280x720 desktop at density 3). Explorer stays at its initial
// placement, 896x524 at (192, 74) (`graphics::initial_window`), and draws its
// own caption, so `at` adds that origin from `window_points`. Selecting an item
// with the keyboard opens its details pane. The points come from Explorer's
// layout and are asserted by desktop/workspace/src/files/scenario_points.rs; the
// resize grips by `tests` below.
/// Bottom-right resize grip, 3 DIP inside the corner, at 896x524.
const CORNER: (i32, i32) = (893, 521);
/// The grip at 400x240 (one Details row, no Large slot) and 400x340 (one
/// Large slot).
const SMALL_CORNER: (i32, i32) = (397, 237);
const TALL_CORNER: (i32, i32) = (397, 337);
/// The grip at the 240 DIP minimum width: rows remain, no name editor fits.
const NARROW_CORNER: (i32, i32) = (237, 521);
/// Light inline-rename border (top edge) in Details row 0 and in the first
/// Large slot.
const EDITOR_TOP: (i32, i32) = (240, 170);
const LARGE_EDITOR_TOP: (i32, i32) = (214, 246);
const EDITOR_BORDER: [u8; 3] = [160; 3];
/// Command bar Rename, and Rename in the context menu's inline icon row.
const RENAME_COMMAND: (i32, i32) = (274, 111);
// The labelled 26H2 icon row (56 px buttons on a 58 px pitch); pinned by
// files::scenario_points::file_mutations_rename_points_follow_the_explorer_layout.
const RENAME_ICON: (i32, i32) = (329, 180);
/// Dark editor: caret at the start of the name, and the fill left of it.
const CARET: (i32, i32) = (199, 182);
const CARET_LEFT: (i32, i32) = (198, 182);
/// Physical screen point of window point `(x, y)`.
fn at((x, y): (i32, i32), scale: i32) -> Result<(i32, i32), String> {
    let (ox, oy) = window_points::origin(window_points::App::Files)?;
    Ok(((ox + x) * scale, (oy + y) * scale))
}
fn probe(point: (i32, i32), scale: i32, color: [u8; 3]) -> Result<(usize, usize, [u8; 3]), String> {
    let (x, y) = at(point, scale)?;
    Ok((x as usize, y as usize, color))
}
fn wait_new(run: &Run, record: &Value, offset: usize, marker: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let text = serial(&run.root, record)?;
        if text.get(offset..).unwrap_or("").contains(marker) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("fresh Files operation missing: {marker}"));
        }
        thread::sleep(Duration::from_millis(80));
    }
}
fn action(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    keys: &[&str],
    marker: &str,
) -> Result<(), String> {
    let offset = serial(&run.root, record)?.len();
    key(c, keys)?;
    wait_new(run, record, offset, marker)
}
fn type_edit(c: &mut qmp::Client, text: &str) -> Result<(), String> {
    for ch in text.chars() {
        c.execute(
            "send-key",
            json!({"keys":session::keys(ch)?,"hold-time":20}),
        )?;
        thread::sleep(Duration::from_millis(70));
    }
    Ok(())
}
fn address(run: &Run, record: &Value, c: &mut qmp::Client) -> Result<(), String> {
    key(c, &["ctrl", "l"])?;
    type_edit(c, r"C:\Users\1001\notes")?;
    action(
        run,
        record,
        c,
        &["ret"],
        "FILES ADDRESS path=/users/1001/notes history=",
    )?;
    // As in Windows, navigation selects nothing; Home selects the first item.
    key(c, &["home"])
}
fn shutdown_clean(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    p: &mut (i32, i32),
    scale: i32,
) -> Result<(), String> {
    let _ = (p, scale);
    key(c, &["esc"])?;
    start_shut_down(c)?;
    wait_serial(
        &run.root,
        record,
        "DOOR_P08 CLEANUP PASS",
        Duration::from_secs(25),
    )?;
    let text = serial(&run.root, record)?;
    if !text.contains("leaked=0") || text.contains("APP FAILED") || text.contains("SERVER FAILED") {
        return Err("Files mutation lifecycle or cleanup failed".into());
    }
    Ok(())
}
fn disk_audit(disk: &std::path::Path, final_state: bool) -> Result<Value, String> {
    let bytes = fs::read(disk).map_err(|e| e.to_string())?;
    let journal = crate::persistence::journal::inspect(&bytes)?;
    if journal.selected().committed {
        return Err("Files shutdown left committed journal".into());
    }
    let tree = crate::persistence::tree::inspect(&journal.replayed_home(&bytes)?)?;
    let catalog = crate::persistence::catalog::decode(&tree)?.ok_or("Files catalog missing")?;
    let paths = catalog.paths()?;
    let path = if final_state {
        "/users/1001/notes/Renamed.txt"
    } else {
        "/users/1001/notes/desktop.txt"
    };
    let node = paths
        .get(path)
        .ok_or("GUI rename did not persist its actual new name")?;
    let file = tree
        .files
        .get(&format!("obj{:016x}", node.id))
        .ok_or("renamed file backing absent")?;
    if node.kind != 1 || node.owner != 1001 || file.bytes != b"Rename preserves file bytes\n" {
        return Err("GUI rename changed identity owner/type or file content".into());
    }
    for absent in if final_state {
        &[
            "desktop.txt",
            "CON.txt",
            "Kept.txt",
            "New folder 1",
            "Archives",
            "Classic folder",
        ][..]
    } else {
        &[]
    } {
        if paths.contains_key(&format!("/users/1001/notes/{absent}")) {
            return Err(format!("unexpected committed rename: {absent}"));
        }
    }
    let folder = paths.get("/users/1001/notes/Final folder");
    if final_state
        && !folder.is_some_and(|folder| {
            folder.kind == 2 && folder.parent == node.parent && folder.owner == 1001
        })
    {
        return Err("renamed folder identity/type/parent".into());
    }
    Ok(
        json!({"disk_sha256":file_hash(disk)?,"object":node.id,"inode":node.inode,"generation":node.generation,"folder_object":folder.map(|folder|folder.id),"path":path,"content_bytes":file.bytes.len(),"journal_clean":true,"independent_namespace_and_allocation_audit":true}),
    )
}
fn snapshot_before_rename(
    run: &Run,
    c: &mut qmp::Client,
    disk: &std::path::Path,
) -> Result<Value, String> {
    // Notes' durable-save reply and Files' initial directory flush have finished.
    // Pause this identified private VM while retaining the original disk bytes.
    c.execute("stop", json!({}))?;
    let retained = run.directory.join("desktop-files-original.raw");
    let copied = fs::copy(disk, &retained).map_err(|e| e.to_string());
    let resumed = c.execute("cont", json!({}));
    copied?;
    resumed?;
    let mut original = disk_audit(&retained, false)?;
    original["disk"] = json!(relative(&run.root, &retained)?);
    Ok(original)
}
fn dark_and_classic(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    _p: &mut (i32, i32),
    scale: i32,
) -> Result<(), String> {
    resolution::activate(&run.root, record, c, 2)?;
    let before = serial(&run.root, record)?.len();
    settings_mode(c, true)?;
    wait_new(run, record, before, "SETTINGS SAVED theme=1")?;
    wait_new(run, record, before, "APPEARANCE applied=1")?;
    resolution::activate(&run.root, record, c, 0)?;
    action(run, record, c, &["f2"], "FILES RENAME BEGIN")?;
    key(c, &["home"])?;
    scene::capture_settled(
        run,
        c,
        "files-dark-rename-caret",
        &[
            // Sample inside the shared ink-height caret; its old line-box top
            // is now intentional breathing room above the reference glyphs.
            // Explorer's dark editor fill is its dark surface (25).
            probe(CARET, scale, [170; 3])?,
            probe(CARET_LEFT, scale, [25; 3])?,
        ],
    )?;
    key(c, &["esc"])?;
    key(c, &["shift", "f10"])?;
    scene::capture_settled(run, c, "files-dark-inline-context", &[])?;
    key(c, &["esc"])?;
    resolution::activate(&run.root, record, c, 2)?;
    key(c, &["ctrl", "f"])?;
    type_line(c, "context menu")?;
    key(c, &["esc"])?;
    // 26H2 Context menu order: File actions, Show more options, Properties
    // placement, then "Arrange primary actions inline" (4th Tab stop); see
    // `file_mutations_scenario_inline_actions_key_sequence` in
    // desktop/workspace/tests/settings_reference_painter.rs.
    for _ in 0..4 {
        key(c, &["tab"])?;
    }
    action(run, record, c, &["spc"], "SETTINGS APPLIED controls=32")?;
    resolution::activate(&run.root, record, c, 0)?;
    key(c, &["shift", "f10"])?;
    scene::capture_settled(run, c, "files-dark-classic-context", &[])?;
    // Inline actions off: the compact commands are labelled rows above the
    // tail. Up wraps to Customize menu, then Show more options, Delete and
    // Rename (disabled Share and Terminal are skipped); asserted by
    // `inline_off_menu_reaches_rename_with_the_scenario_keys`.
    for _ in 0..4 {
        key(c, &["up"])?;
    }
    action(run, record, c, &["ret"], "FILES RENAME BEGIN")?;
    type_edit(c, "Classic folder")?;
    action(run, record, c, &["ret"], "FILES RENAME COMMITTED")?;
    action(run, record, c, &["f2"], "FILES RENAME BEGIN")?;
    type_edit(c, "Final folder")?;
    action(run, record, c, &["ret"], "FILES RENAME COMMITTED")?;
    resolution::activate(&run.root, record, c, 2)?;
    action(run, record, c, &["spc"], "SETTINGS APPLIED controls=48")?;
    resolution::activate(&run.root, record, c, 0)?;
    scene::capture_settled(run, c, "files-dark-renamed", &[])?;
    Ok(())
}
fn resize_and_view(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    p: &mut (i32, i32),
    scale: i32,
) -> Result<(), String> {
    // Three DIP inside each edge reaches the rounded corner's actual coverage
    // while still hitting both resize borders. The extreme last pixel is clear.
    key(c, &["end"])?; // Text file follows the folder in Details.
    action(run, record, c, &["f2"], "FILES RENAME BEGIN")?;
    // 400x240: one Details row, so the editing second item is scrolled in.
    scene::drag(c, p, at(CORNER, scale)?, at(SMALL_CORNER, scale)?)?;
    scene::capture_settled(
        run,
        c,
        "files-rename-resize-revealed",
        &[probe(EDITOR_TOP, scale, EDITOR_BORDER)?],
    )?;
    key(c, &["esc"])?;
    scene::drag(c, p, at(SMALL_CORNER, scale)?, at(TALL_CORNER, scale)?)?;
    key(c, &["ctrl", "shift", "2"])?; // One icon slot; selected second item must be revealed.
    action(run, record, c, &["f2"], "FILES RENAME BEGIN")?;
    scene::capture_settled(
        run,
        c,
        "files-rename-view-revealed",
        // The 20 DIP editor is centered on the Large tile's label.
        &[probe(LARGE_EDITOR_TOP, scale, EDITOR_BORDER)?],
    )?;
    let before = serial(&run.root, record)?.len();
    scene::drag(c, p, at(TALL_CORNER, scale)?, at(SMALL_CORNER, scale)?)?;
    wait_new(run, record, before, "FILES RENAME CANCEL layout")?;
    scene::capture_settled(run, c, "files-rename-zero-capacity-cancel", &[])?;
    action(run, record, c, &["f2"], "FILES RENAME REJECT not_visible")?;
    scene::drag(c, p, at(SMALL_CORNER, scale)?, at(CORNER, scale)?)?;
    key(c, &["ctrl", "shift", "6"])?;
    key(c, &["home"])?;
    action(run, record, c, &["f2"], "FILES RENAME BEGIN")?;
    let before = serial(&run.root, record)?.len();
    // The window service's resize minimum is 240 DIP, above the compositor's
    // 160 DIP floor; Explorer's rows remain but the name editor cannot fit.
    scene::drag(c, p, at(CORNER, scale)?, at(NARROW_CORNER, scale)?)?;
    wait_new(run, record, before, "FILES RENAME CANCEL layout")?;
    scene::capture_settled(run, c, "files-rename-narrow-cancel", &[])?;
    action(run, record, c, &["f2"], "FILES RENAME REJECT not_visible")?;
    scene::drag(c, p, at(NARROW_CORNER, scale)?, at(CORNER, scale)?)?;
    Ok(())
}
pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    verify_backend(run, image, false)
}
pub(super) fn verify_accelerated(run: &mut Run, image: &str) -> Result<(), String> {
    verify_backend(run, image, true)
}
fn launch(
    run: &mut Run,
    image: &str,
    disk: &std::path::Path,
    accelerated: bool,
) -> Result<Value, String> {
    if accelerated {
        session::launch_desktop_accelerated(run, image, disk, false, "3840x2160")
    } else {
        session::launch_desktop_wide(run, image, disk, false)
    }
}
fn verify_backend(run: &mut Run, image: &str, accelerated: bool) -> Result<(), String> {
    let scale = if accelerated { 3 } else { 1 };
    let disk = run.directory.join("desktop-file-mutations.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut first = launch(run, image, &disk, accelerated)?;
    let mut original = Value::Null;
    let result: Result<(), String> = (|| {
        let mut c = connection(&first)?;
        let mut p = (640 * scale, 360 * scale);
        resolution::activate(&run.root, &first, &mut c, 1)?;
        type_line(&mut c, "Rename preserves file bytes")?;
        action(run, &first, &mut c, &["ctrl", "s"], "DOOR_P11 NOTES SAVED")?;
        action(run, &first, &mut c, &["ctrl", "w"], "DOOR_P11 CLIENT_EXIT")?;
        resolution::activate(&run.root, &first, &mut c, 0)?;
        address(run, &first, &mut c)?;
        original = snapshot_before_rename(run, &mut c, &disk)?;
        action(run, &first, &mut c, &["f2"], "FILES RENAME BEGIN")?;
        let text = serial(&run.root, &first)?;
        let observed = text
            .lines()
            .rev()
            .find(|line| line.contains("FILES RENAME BEGIN "))
            .and_then(|line| {
                line.split_whitespace()
                    .find_map(|word| word.strip_prefix("object="))
            })
            .and_then(|id| id.parse::<u64>().ok())
            .ok_or("original observed rename object missing")?;
        if original["object"] != observed {
            return Err("GUI selection did not bind the original disk object".into());
        }
        scene::capture_settled(
            run,
            &mut c,
            "files-rename-editor",
            // The editor's light neutral border (160; it maps to 65 in dark
            // mode). Keep an exact native border probe.
            &[probe(EDITOR_TOP, scale, EDITOR_BORDER)?],
        )?;
        type_edit(&mut c, "CON")?;
        action(
            run,
            &first,
            &mut c,
            &["ret"],
            "FILES RENAME REJECT invalid_name",
        )?;
        scene::capture_settled(run, &mut c, "files-rename-invalid", &[])?;
        key(&mut c, &["esc"])?;
        action(run, &first, &mut c, &["f2"], "FILES RENAME BEGIN")?;
        type_edit(&mut c, "Kept")?;
        key(&mut c, &["esc"])?;
        action(run, &first, &mut c, &["f2"], "FILES RENAME BEGIN")?;
        type_edit(&mut c, "Renamed")?;
        action(run, &first, &mut c, &["ret"], "FILES RENAME COMMITTED")?;
        scene::capture_settled(run, &mut c, "files-renamed-file", &[])?;
        key(&mut c, &["ctrl", "shift", "n"])?;
        // Files retains the prior selection through refresh. Home explicitly
        // selects the directory before testing directory rename and collision.
        key(&mut c, &["home"])?;
        action(run, &first, &mut c, &["f2"], "FILES RENAME BEGIN")?;
        type_edit(&mut c, "renamed.txt")?;
        action(run, &first, &mut c, &["ret"], "FILES RENAME REJECT")?;
        scene::capture_settled(run, &mut c, "files-rename-collision", &[])?;
        key(&mut c, &["esc"])?;
        let before = serial(&run.root, &first)?.len();
        let (x, y) = at(RENAME_COMMAND, scale)?;
        click(&mut c, &mut p, x, y)?;
        wait_new(run, &first, before, "FILES RENAME BEGIN")?;
        type_edit(&mut c, "Archives")?;
        action(run, &first, &mut c, &["ret"], "FILES RENAME COMMITTED")?;
        key(&mut c, &["shift", "f10"])?;
        scene::capture_settled(run, &mut c, "files-rename-inline-menu", &[])?;
        let before = serial(&run.root, &first)?.len();
        let (x, y) = at(RENAME_ICON, scale)?;
        click(&mut c, &mut p, x, y)?;
        wait_new(run, &first, before, "FILES RENAME BEGIN")?;
        type_edit(&mut c, "Final folder")?;
        action(run, &first, &mut c, &["ret"], "FILES RENAME COMMITTED")?;
        scene::capture_settled(run, &mut c, "files-renamed-folder", &[])?;
        resize_and_view(run, &first, &mut c, &mut p, scale)?;
        dark_and_classic(run, &first, &mut c, &mut p, scale)?;
        shutdown_clean(run, &first, &mut c, &mut p, scale)
    })();
    if result.is_err()
        && let Ok(mut c) = connection(&first)
    {
        let _ = c.execute("quit", json!({}));
    }
    result?;
    seal_serial(&run.root, &mut first)?;
    let saved_disk = run.directory.join("desktop-files-first-shutdown.raw");
    fs::copy(&disk, &saved_disk).map_err(|e| e.to_string())?;
    let mut saved = disk_audit(&saved_disk, true)?;
    saved["disk"] = json!(relative(&run.root, &saved_disk)?);
    for field in ["object", "inode", "generation"] {
        if original[field] != saved[field] {
            return Err(format!(
                "GUI rename replaced the original backing identity: {field}"
            ));
        }
    }
    let mut second = launch(run, image, &disk, accelerated)?;
    let result: Result<(), String> = (|| {
        let mut c = connection(&second)?;
        let mut p = (640 * scale, 360 * scale);
        resolution::activate(&run.root, &second, &mut c, 0)?;
        address(run, &second, &mut c)?;
        scene::capture_settled(run, &mut c, "files-renamed-after-reboot", &[])?;
        key(&mut c, &["end"])?; // directory first, then the renamed text file.
        key(&mut c, &["ret"])?;
        scene::capture_settled(run, &mut c, "files-renamed-preview-after-reboot", &[])?;
        shutdown_clean(run, &second, &mut c, &mut p, scale)
    })();
    if result.is_err()
        && let Ok(mut c) = connection(&second)
    {
        let _ = c.execute("quit", json!({}));
    }
    result?;
    seal_serial(&run.root, &mut second)?;
    let mut reopened = disk_audit(&disk, true)?;
    reopened["disk"] = json!(relative(&run.root, &disk)?);
    for field in ["object", "inode", "generation", "folder_object"] {
        if saved[field] != reopened[field] {
            return Err(format!("rename identity changed after reboot: {field}"));
        }
    }
    run.check("desktop-files-checked-rename", json!({"scale":scale,"accelerated":accelerated,"first":first,"second":second,"original":original,"saved":saved,"reopened":reopened,"entrypoints":["F2","command bar","inline context","classic row"],"themes":["light","dark"],"reserved_name_rejected":true,"casefold_collision_rejected":true,"escape_cancels":true,"file_extension_preserved":true,"separate_boots":2,"scope":"real same-parent checked rename; not Recycle Bin/copy/move or complete Windows compatibility"}));
    Ok(())
}
/// The resize edges a press at `point` grabs on a normal Explorer window with
/// outer rect `rect`, or None when it is not the window's resize border. Uses
/// the real window authority, as `window_points` does.
#[cfg(test)]
pub(super) fn resize_edges(rect: (i32, i32, u32, u32), point: (i32, i32)) -> Option<u8> {
    use door_window::graphics as g;
    let mut engine = g::Engine::new(1280, 720, g::Theme::Light, g::Limits::default()).ok()?;
    engine
        .set_work_area(door_window::taskbar::Layout::new(1280, 720).work_area())
        .ok()?;
    let owner = g::Owner {
        pid: 7,
        session: 1,
        epoch: 1,
    };
    let outer = g::Rect::new(rect.0, rect.1, rect.2, rect.3).ok()?;
    let caption = g::CaptionLayout {
        tabbed: true,
        height: 40,
        max_width: 2048,
        side_margin: 8,
    };
    let id = engine.create_client_caption(owner, outer, caption).ok()?;
    (engine.hit_test(point.0, point.1) == Some((id, g::HitRegion::Resize)))
        .then(|| g::CursorIcon::resize_edges_scaled(outer, point.0, point.1, g::Scale::ONE))
}
#[cfg(test)]
mod tests {
    use super::*;
    const RIGHT: u8 = 2;
    const BOTTOM: u8 = 8;
    /// Each drag starts on the bottom-right grip of the size the previous one
    /// left, and ends where the grip lands at the intended size.
    #[test]
    fn resize_grips_follow_the_initial_placement_and_each_size() {
        let initial = door_window::graphics::initial_window((1280, 720), (1000, 640), 0, None);
        assert_eq!(initial, (192, 74, 896, 524));
        assert_eq!(
            window_points::origin(window_points::App::Files),
            Ok((192, 74))
        );
        let grip = |(w, h): (u32, u32)| (w as i32 - 3, h as i32 - 3);
        for (size, point) in [
            ((896, 524), CORNER),
            ((400, 240), SMALL_CORNER),
            ((400, 340), TALL_CORNER),
            ((240, 524), NARROW_CORNER),
        ] {
            assert_eq!(grip(size), point, "{size:?}");
            assert_eq!(
                resize_edges((192, 74, size.0, size.1), at(point, 1).unwrap()),
                Some(RIGHT | BOTTOM),
                "{size:?}"
            );
        }
        // The other points are inside the window at every size they are used.
        // The Details editor at 400x240, the Large one at 400x340; command,
        // menu and caret points are used at 896x524 and also fit 400x240.
        for (point, (w, h)) in [
            (EDITOR_TOP, (400, 240)),
            (LARGE_EDITOR_TOP, (400, 340)),
            (RENAME_COMMAND, (400, 240)),
            (RENAME_ICON, (400, 240)),
            (CARET, (400, 240)),
        ] {
            assert!(point.0 < w && point.1 < h, "{point:?}");
        }
    }
}
