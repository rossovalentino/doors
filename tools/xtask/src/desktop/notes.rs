//! Real editing input and independently audited persistence on private disks.
use super::scene::drag;
use super::*;
// Notes opens at its initial placement, (192, 74, 896, 524) on the 1280x720
// logical desktop of both profiles (70% x 78% of the work area) (`window_points::notes_window`, host
// tested). It paints its own 41 px tabbed caption (Windows 11 26H2 Notepad),
// so client coordinates start at the window origin. Client layout
// (desktop/workspace/src/document_format.rs `Chrome`, used by notes.rs
// `configure`/`draw`): body at (16, 89) with 20 px rows and a wrap width of
// client width - 32, caret ink from client y 91, File menu plate at (4..45, 41..73).
// Text is measured with Notes' monospace layout font.
const CLIENT_X: i32 = 192;
const CLIENT_Y: i32 = 74;
const CLIENT_W: i32 = 896;
const CLIENT_H: i32 = 524;
const TEXT_X: i32 = CLIENT_X + 16;
// Shared font-ink alignment puts the caret at client 91; body hit rows still
// start at 89. Keep this an independently asserted physical pixel, not a value
// copied from the app at runtime.
const CARET_Y: i32 = CLIENT_Y + 91;
/// Pointer row inside the first body line.
const ROW_Y: i32 = CLIENT_Y + 96;
/// Select-all accent probe row, inside the first body line.
const SELECT_Y: i32 = CLIENT_Y + 92;
const LINE_HEIGHT: i32 = 20;
const WRAP_WIDTH: i32 = CLIENT_W - 32;
/// Notes' dark editor body: Notepad's #272727 (notes.rs `Look`).
const NOTES_DARK: u8 = 39;
/// Body crop: through trailing wrap carets (5 px past the wrap edge, 11 px
/// before the right edge) and nine rows.
const BODY_RIGHT: i32 = CLIENT_X + CLIENT_W - 11;
const BODY_BOTTOM: i32 = CLIENT_Y + 271;
/// Logical client point to physical screen point.
fn at(x: i32, y: i32, scale: i32) -> (i32, i32) {
    ((CLIENT_X + x) * scale, (CLIENT_Y + y) * scale)
}
fn pixel_at(x: i32, y: i32, scale: i32) -> (usize, usize) {
    let (x, y) = at(x, y, scale);
    (x as usize, y as usize)
}
/// Full-size modal (896 x 524 client): button `index` origin in client space.
fn button(index: usize) -> (i32, i32, u32, u32) {
    window_points::notes_dialog(CLIENT_W as u32, CLIENT_H as u32).1[index]
}
/// Centre of full-size modal button `index`, physical.
fn button_centre(index: usize, scale: i32) -> (i32, i32) {
    let (x, y, w, h) = button(index);
    at(x + w as i32 / 2, y + h as i32 / 2, scale)
}
/// Focused Save in a full-size modal: 5 px right, 9 px down of its origin.
fn save_focus(scale: i32) -> (usize, usize, [u8; 3]) {
    let (x, y, _, _) = button(0);
    let (x, y) = pixel_at(x + 5, y + 9, scale);
    (x, y, [0, 103, 196])
}
/// The minimum client (240 x 178) keeps the earlier fixture's client offsets.
const MIN_W: i32 = 240;
const MIN_H: i32 = 178;
fn fresh(run: &Run, record: &Value, offset: usize, marker: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if serial(&run.root, record)?
            .get(offset..)
            .unwrap_or("")
            .contains(marker)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("fresh Notes operation missing: {marker}"));
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
    fresh(run, record, offset, marker)
}
fn body(frame: &scene::Pixels, scale: usize) -> Result<Vec<[u8; 3]>, String> {
    let mut pixels = Vec::new();
    for y in CARET_Y as usize * scale..BODY_BOTTOM as usize * scale {
        // Include trailing wrap carets (at most TEXT_X + WRAP_WIDTH), plus
        // nearby padding. A text-only crop can otherwise hide affinity errors.
        for x in TEXT_X as usize * scale..BODY_RIGHT as usize * scale {
            pixels.push(frame.pixel(x, y)?);
        }
    }
    Ok(pixels)
}
fn selected(
    run: &mut Run,
    c: &mut qmp::Client,
    name: &str,
    scale: usize,
    background: u8,
) -> Result<scene::Pixels, String> {
    let frame = scene::capture_settled(
        run,
        c,
        name,
        &[
            (
                (CLIENT_X + 400) as usize * scale,
                SELECT_Y as usize * scale,
                [0, 103, 196],
            ),
            (
                (CLIENT_X + 470) as usize * scale,
                (CLIENT_Y + 158) as usize * scale,
                [background; 3],
            ),
        ],
    )?;
    let mut bright = 0;
    for y in CARET_Y as usize * scale..(CARET_Y + 16) as usize * scale {
        for x in (TEXT_X + 4) as usize * scale..(CLIENT_X + 165) as usize * scale {
            let p = frame.pixel(x, y)?;
            if p[0] > 150 && p[1] > 170 && p[2] > 210 {
                bright += 1;
            }
        }
    }
    if bright < 10 {
        return Err("selected Notes text has no contrasting visible glyph coverage".into());
    }
    Ok(frame)
}
fn checkpoint(
    run: &Run,
    c: &mut qmp::Client,
    disk: &std::path::Path,
    name: &str,
    expected: &[u8],
) -> Result<Value, String> {
    c.execute("stop", json!({}))?;
    let copy = run.directory.join(format!("notes-{name}.raw"));
    let copied = fs::copy(disk, &copy).map_err(|e| e.to_string());
    let resumed = c.execute("cont", json!({}));
    copied?;
    resumed?;
    disk_audit(&copy, expected)
}
fn disk_audit(disk: &std::path::Path, expected: &[u8]) -> Result<Value, String> {
    let mut result = audit(disk, expected)?;
    let bytes = fs::read(disk).map_err(|e| e.to_string())?;
    let journal = crate::persistence::journal::inspect(&bytes)?;
    let tree = crate::persistence::tree::inspect(&journal.replayed_home(&bytes)?)?;
    let catalog = crate::persistence::catalog::decode(&tree)?.ok_or("Notes catalog missing")?;
    let paths = catalog.paths()?;
    let node = paths
        .get("/users/1001/notes/desktop.txt")
        .ok_or("Notes object missing")?;
    if node.kind != 1 || node.owner != 1001 {
        return Err("Notes owner/type changed".into());
    }
    result["object"] = json!(node.id);
    result["inode"] = json!(node.inode);
    result["generation"] = json!(node.generation);
    result["disk"] = json!(disk.to_string_lossy());
    Ok(result)
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
fn shutdown_clean(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    p: &mut (i32, i32),
    scale: i32,
    clients: usize,
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
    // Fixed device/DMA pages, not the number of retired processes or threads.
    wait_serial(&run.root, record, "reclaimed=8", Duration::from_secs(10))?;
    retirement(&serial(&run.root, record)?, clients)
}
/// Kernel records, exact fixture identities and complete cleanup summaries.
fn retirement(text: &str, clients: usize) -> Result<(), String> {
    // Init's windowless Start search host retires with its own exit code.
    let hosts = super::shutdown::search_hosts(text).len();
    // So does the session IME.
    let imes = super::shutdown::ime_hosts(text).len();
    // And the P15 system services (network, audio).
    let nets = super::shutdown::network_hosts(text).len();
    let count = 5 + clients + hosts + imes + nets;
    let summaries: Vec<_> = text
        .lines()
        .enumerate()
        .filter(|(_, l)| l.starts_with("DOOR_P08 CLEANUP "))
        .collect();
    if summaries.len() != 1
        || summaries[0].1
            != format!("DOOR_P08 CLEANUP PASS processes={count} threads={count} leaked=0")
    {
        return Err("Notes exact cleanup counts absent".into());
    }
    let devices: Vec<_> = text
        .lines()
        .enumerate()
        .filter(|(_, l)| l.starts_with("DOOR_P08 DEVICE "))
        .collect();
    if devices.len() != 1
        || devices[0].0 <= summaries[0].0
        || !devices[0].1.starts_with("DOOR_P08 DEVICE PASS ")
        || devices[0].1.split_whitespace().last() != Some("reclaimed=8")
    {
        return Err("Notes complete device reclamation absent".into());
    }
    let mut pids = std::collections::BTreeSet::new();
    let mut instances = std::collections::BTreeSet::new();
    let mut expected = vec![(1, 1, 80), (2, 1, 0), (3, 2, 0), (4, 3, 0), (11, 3, -1)];
    expected.extend(std::iter::repeat_n((12, 1001, 0), clients));
    expected.extend(std::iter::repeat_n(
        (
            super::shutdown::SEARCH_HOST_MANIFEST as _,
            1001,
            super::shutdown::SEARCH_HOST_EXIT as _,
        ),
        hosts,
    ));
    expected.extend(std::iter::repeat_n(
        (16, 1001, super::shutdown::IME_EXIT as _),
        imes,
    ));
    // The P15 system services Init stops with exit -1 (door-network,
    // door-audio).
    let audios = super::shutdown::audio_hosts(text).len();
    expected.extend(std::iter::repeat_n((29, 4, -1), nets - audios));
    expected.extend(std::iter::repeat_n((30, 4, -1), audios));
    for (index, line) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| l.starts_with("DOOR_P08 PROCESS "))
    {
        let p =
            crate::services::scenarios::process(line).ok_or("malformed Notes kernel identity")?;
        let at = expected
            .iter()
            .position(|e| *e == (p.manifest, p.uid, p.exit))
            .ok_or("unexpected Notes retired process")?;
        expected.remove(at);
        // Session processes: workspace clients, the IME and the desktop
        // shell (which hosts Start search).
        let session_process = matches!(p.manifest, 12 | 16)
            || p.manifest == super::shutdown::SEARCH_HOST_MANIFEST as u64;
        let (session, epoch) = if session_process { (1, 1) } else { (0, 0) };
        if index >= summaries[0].0
            || p.pid == 0
            || p.instance == 0
            || !pids.insert(p.pid)
            || !instances.insert(p.instance)
            || p.session != session
            || p.epoch != epoch
        {
            return Err("Notes retired process identity/lifetime mismatch".into());
        }
    }
    if !expected.is_empty()
        || text.contains("APP FAILED")
        || text.contains("SERVER FAILED")
        || text.contains("KERNEL_PANIC")
    {
        return Err("Notes lifecycle missing or failed".into());
    }
    Ok(())
}
fn pointer_navigation(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    p: &mut (i32, i32),
    disk: &std::path::Path,
    scale: i32,
) -> Result<(), String> {
    use door_graphics_text::{FontAtlas, TextStyle};
    // Notes lays out and hit-tests text with its monospace editor font.
    let fonts = FontAtlas::monospace_reference_scaled(scale as u32)
        .map_err(|e| format!("Notes input font: {e:?}"))?;
    let width = |s: &str| {
        fonts
            .measure(s, TextStyle::Regular14)
            .map(|m| m.advance_width as i32)
            .map_err(|e| format!("Notes input metrics: {e:?}"))
    };
    click(c, p, (TEXT_X + width("Original ")?) * scale, ROW_Y * scale)?;
    key(c, &["x"])?;
    action(run, record, c, &["ctrl", "s"], "DOOR_P11 NOTES SAVED")?;
    let inserted = checkpoint(run, c, disk, "pointer-insert", b"Original xtext\n")?;
    key(c, &["ctrl", "z"])?;
    action(run, record, c, &["ctrl", "s"], "DOOR_P11 NOTES SAVED")?;
    key(c, &["ctrl", "end"])?;
    scene::drag(
        c,
        p,
        ((TEXT_X + 1) * scale, ROW_Y * scale),
        at(460, 93, scale),
    )?;
    move_to(c, p, 0, 0)?;
    let dragged = scene::capture_settled(
        run,
        c,
        "notes-pointer-selection",
        &[(
            ((TEXT_X + 1) * scale) as usize,
            (CARET_Y * scale) as usize,
            [0, 103, 196],
        )],
    )?;
    key(c, &["ctrl", "home"])?;
    key(c, &["shift", "end"])?;
    let keyboard = scene::capture_settled(run, c, "notes-keyboard-line-selection", &[])?;
    if body(&dragged, scale as usize)? != body(&keyboard, scale as usize)? {
        return Err("mouse selection differs from actual keyboard line selection".into());
    }
    key(c, &["x"])?;
    action(run, record, c, &["ctrl", "s"], "DOOR_P11 NOTES SAVED")?;
    let replacement = checkpoint(run, c, disk, "drag-replace", b"x\n")?;
    key(c, &["ctrl", "z"])?;
    action(run, record, c, &["ctrl", "s"], "DOOR_P11 NOTES SAVED")?;
    // A trusted overlay must revoke the held client capture. Later hover motion
    // must not continue extending the document after the physical release.
    move_to(c, p, TEXT_X * scale, ROW_Y * scale)?;
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"btn","data":{"down":true,"button":"left"}}]}),
    )?;
    action(
        run,
        record,
        c,
        &["meta_l"],
        "SCENE_PRESENTED app=0 overlay=1",
    )?;
    key(c, &["esc"])?;
    move_to(c, p, 0, 0)?;
    let canceled = scene::capture_settled(
        run,
        c,
        "notes-capture-canceled",
        &[
            (
                pixel_at(470, 158, scale).0,
                pixel_at(470, 158, scale).1,
                [255; 3],
            ),
            (
                (TEXT_X * scale) as usize,
                (CARET_Y * scale) as usize,
                [0, 103, 196],
            ),
        ],
    )?;
    let (hx, hy) = at(200, 113, scale);
    move_to(c, p, hx, hy)?;
    move_to(c, p, 0, 0)?;
    let held = scene::capture_settled(run, c, "notes-canceled-held-motion", &[])?;
    if body(&canceled, scale as usize)? != body(&held, scale as usize)? {
        return Err("Notes continued a held drag after trusted capture cancellation".into());
    }
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"btn","data":{"down":false,"button":"left"}}]}),
    )?;
    move_to(c, p, hx, hy)?;
    move_to(c, p, 0, 0)?;
    let hovered = scene::capture_settled(run, c, "notes-after-cancel-motion", &[])?;
    if body(&canceled, scale as usize)? != body(&hovered, scale as usize)? {
        return Err("Notes drag continued after capture cancellation".into());
    }
    key(c, &["ctrl", "a"])?;
    // Long enough to wrap the 856 px body with the monospace font.
    let wrapped = "W".repeat(160);
    type_line(c, &wrapped)?;
    let count = (1..=160)
        .take_while(|n| width(&wrapped[..*n]).is_ok_and(|w| w <= WRAP_WIDTH))
        .last()
        .ok_or("Notes reference wrap count")?;
    if count == 160 {
        return Err("Notes wrap fixture did not wrap".into());
    }
    key(c, &["ctrl", "home"])?;
    key(c, &["end"])?;
    key(c, &["left"])?;
    key(c, &["right"])?;
    let end = scene::capture_settled(
        run,
        c,
        "notes-wrap-end-caret",
        &[(
            ((TEXT_X + width(&wrapped[..count])?) * scale) as usize,
            (CARET_Y * scale) as usize,
            [0, 103, 196],
        )],
    )?;
    key(c, &["pgdn"])?;
    key(c, &["pgup"])?;
    let paged = scene::capture_settled(run, c, "notes-page-navigation-return", &[])?;
    if body(&end, scale as usize)? != body(&paged, scale as usize)? {
        return Err("page navigation lost the preferred caret column".into());
    }
    key(c, &["right"])?;
    let start = scene::capture_settled(
        run,
        c,
        "notes-wrap-start-caret",
        &[(
            (TEXT_X * scale) as usize,
            ((CARET_Y + LINE_HEIGHT) * scale) as usize,
            [0, 103, 196],
        )],
    )?;
    key(c, &["left"])?;
    let back = scene::capture_settled(run, c, "notes-wrap-arrow-return", &[])?;
    if body(&end, scale as usize)? != body(&back, scale as usize)? {
        return Err("wrap boundary caret did not return to its original visual side".into());
    }
    key(c, &["right"])?;
    key(c, &["x"])?;
    action(run, record, c, &["ctrl", "s"], "DOOR_P11 NOTES SAVED")?;
    let mut expected = wrapped.clone();
    expected.insert(count, 'x');
    expected.push('\n');
    let wrap = checkpoint(run, c, disk, "wrap-boundary", expected.as_bytes())?;
    key(c, &["ctrl", "z"])?;
    let undone = scene::capture_settled(run, c, "notes-wrap-undo-affinity", &[])?;
    if body(&start, scale as usize)? != body(&undone, scale as usize)? {
        return Err("undo lost the caret's wrap affinity".into());
    }
    key(c, &["ctrl", "a"])?;
    for line in ["ABCDE", "x", "ABCDE"] {
        type_line(c, line)?;
    }
    key(c, &["ctrl", "home"])?;
    key(c, &["end"])?;
    key(c, &["down"])?;
    key(c, &["down"])?;
    key(c, &["x"])?;
    action(run, record, c, &["ctrl", "s"], "DOOR_P11 NOTES SAVED")?;
    let vertical = checkpoint(run, c, disk, "vertical-column", b"ABCDE\nx\nABCDEx\n")?;
    key(c, &["ctrl", "home"])?;
    key(c, &["down"])?;
    key(c, &["shift", "down"])?;
    let selected = scene::capture_settled(run, c, "notes-vertical-selection", &[])?;
    key(c, &["delete"])?;
    key(c, &["ctrl", "z"])?;
    key(c, &["ctrl", "shift", "z"])?;
    key(c, &["ctrl", "z"])?;
    let restored = scene::capture_settled(run, c, "notes-vertical-selection-undo", &[])?;
    if body(&selected, scale as usize)? != body(&restored, scale as usize)? {
        return Err("vertical selection undo/redo lost visible state".into());
    }
    for audit in [&replacement, &wrap, &vertical] {
        for field in ["object", "inode", "generation"] {
            if audit[field] != inserted[field] {
                return Err(format!("Notes input changed backing identity: {field}"));
            }
        }
    }
    key(c, &["ctrl", "a"])?;
    type_line(c, "Original text")?;
    action(run, record, c, &["ctrl", "s"], "DOOR_P11 NOTES SAVED")?;
    key(c, &["ctrl", "end"])?;
    move_to(c, p, 0, 0)?;
    run.check("desktop-notes-pointer-navigation",json!({"scale":scale,"pointer_insert":inserted,"drag_replacement":replacement,"wrapped_edit":wrap,"vertical_edit":vertical,"wrap_first_row_bytes":count,"pointer_keyboard_selection_equal":true,"capture_cancel_stops_hover_edit":true,"wrap_visual_caret_and_undo":true,"vertical_preferred_column":true,"scope":"single click, visible-row drag, visual keyboard navigation; no Shift-click, edge autoscroll, horizontal scrolling or Unicode/IME"}));
    Ok(())
}
fn modal_light(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    p: &mut (i32, i32),
    disk: &std::path::Path,
    scale: i32,
) -> Result<(), String> {
    key(c, &["ctrl", "a"])?;
    type_line(c, "Saved by dialog")?;
    action(
        run,
        record,
        c,
        &["ctrl", "o"],
        "DOOR_P12 NOTES PROMPT intent=Reload",
    )?;
    key(c, &["tab"])?;
    key(c, &["shift", "tab"])?;
    let before = serial(&run.root, record)?.len();
    let (sx, sy) = button_centre(0, scale);
    click(c, p, sx, sy)?;
    fresh(
        run,
        record,
        before,
        "DOOR_P12 NOTES PROMPT_DONE intent=Reload choice=0",
    )?;
    let saved = checkpoint(run, c, disk, "dialog-reload-save", b"Saved by dialog\n")?;
    key(c, &["ctrl", "a"])?;
    let expected = selected(run, c, "notes-dialog-saved-selection", scale as usize, 255)?;
    type_line(c, "Discard reload")?;
    action(
        run,
        record,
        c,
        &["ctrl", "o"],
        "DOOR_P12 NOTES PROMPT intent=Reload",
    )?;
    key(c, &["tab"])?;
    action(
        run,
        record,
        c,
        &["ret"],
        "DOOR_P12 NOTES PROMPT_DONE intent=Reload choice=1",
    )?;
    key(c, &["ctrl", "a"])?;
    let reloaded = selected(run, c, "notes-dialog-discard-reloaded", scale as usize, 255)?;
    if body(&expected, scale as usize)? != body(&reloaded, scale as usize)? {
        return Err("Don't save/reopen did not restore exact saved document".into());
    }
    let discarded = checkpoint(run, c, disk, "dialog-reload-discard", b"Saved by dialog\n")?;
    if saved["disk_sha256"] != discarded["disk_sha256"] {
        return Err("Discard reload wrote to the disk".into());
    }
    type_line(c, "Keep on cancel")?;
    // Grab straight edges; the rounded bottom-right corner is outside the hit
    // region and would instead focus the desktop without resizing anything.
    // Right edge (2 px inside the 4 px resize band) to a 240 px client, then
    // the bottom edge to 178 px: the earlier fixture's minimum client, so
    // every client offset below is unchanged.
    drag(
        c,
        p,
        at(CLIENT_W - 2, 396, scale),
        at(MIN_W - 2, 396, scale),
    )?;
    drag(
        c,
        p,
        at(120, CLIENT_H - 2, scale),
        at(120, MIN_H - 2, scale),
    )?;
    move_to(c, p, 0, 0)?;
    let minimum_before = scene::capture_settled(run, c, "notes-minimum-before-prompt", &[])?;
    action(
        run,
        record,
        c,
        &["ctrl", "w"],
        "DOOR_P12 NOTES PROMPT intent=Close",
    )?;
    move_to(c, p, 0, 0)?;
    let narrow = scene::capture_settled(
        run,
        c,
        "notes-minimum-save-dialog",
        &[(
            pixel_at(22, 140, scale).0,
            pixel_at(22, 140, scale).1,
            [0, 103, 196],
        )],
    )?;
    for (name, top, bottom) in [("question", 16, 36), ("warning", 44, 88)] {
        let mut ink = 0;
        for y in (CLIENT_Y + top) * scale..(CLIENT_Y + bottom) * scale {
            for x in (CLIENT_X + 17) * scale..(CLIENT_X + 223) * scale {
                let rgb = narrow.pixel(x as usize, y as usize)?;
                if rgb.iter().all(|v| *v < 120) {
                    ink += 1;
                }
            }
        }
        if ink < 30 * scale as usize {
            return Err(format!("minimum dialog has no readable {name} glyphs"));
        }
    }
    // Cancel while Save is held, then release: the old press must not activate
    // a control in the newly uncovered editor.
    let (hx, hy) = at(40, 146, scale);
    move_to(c, p, hx, hy)?;
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"btn","data":{"down":true,"button":"left"}}]}),
    )?;
    // QMP injection acknowledgement does not establish cross-device delivery
    // order. Prove DOWN reached the modal before injecting keyboard Escape.
    scene::capture_settled(
        run,
        c,
        "notes-minimum-save-held",
        &[(
            pixel_at(22, 140, scale).0,
            pixel_at(22, 140, scale).1,
            [0, 80, 160],
        )],
    )?;
    action(
        run,
        record,
        c,
        &["esc"],
        "DOOR_P12 NOTES PROMPT_CANCEL intent=Close",
    )?;
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"btn","data":{"down":false,"button":"left"}}]}),
    )?;
    action(
        run,
        record,
        c,
        &["ctrl", "w"],
        "DOOR_P12 NOTES PROMPT intent=Close",
    )?;
    // This non-button dialog point overlaps the real underlying File control.
    // A stale press surviving Escape would open its menu on button-up.
    let (fx, fy) = at(20, 44, scale);
    move_to(c, p, fx, fy)?;
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"btn","data":{"down":true,"button":"left"}}]}),
    )?;
    // The non-button press clears keyboard focus. Its neutral border proves
    // the held event was consumed before dismissal; otherwise separate virtio
    // device polls could legally deliver Escape ahead of this mouse packet.
    scene::capture_settled(
        run,
        c,
        "notes-minimum-file-region-held",
        &[(
            pixel_at(16, 146, scale).0,
            pixel_at(16, 146, scale).1,
            [190; 3],
        )],
    )?;
    action(
        run,
        record,
        c,
        &["esc"],
        "DOOR_P12 NOTES PROMPT_CANCEL intent=Close",
    )?;
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"btn","data":{"down":false,"button":"left"}}]}),
    )?;
    move_to(c, p, 0, 0)?;
    let minimum_after = scene::capture_settled(run, c, "notes-minimum-dismiss-release", &[])?;
    for y in CLIENT_Y * scale..(CLIENT_Y + MIN_H) * scale {
        for x in CLIENT_X * scale..(CLIENT_X + MIN_W) * scale {
            if minimum_before.pixel(x as usize, y as usize)?
                != minimum_after.pixel(x as usize, y as usize)?
            {
                return Err(
                    "modal-dismiss release changed the restored client or opened its File menu"
                        .into(),
                );
            }
        }
    }
    drag(
        c,
        p,
        at(120, MIN_H - 2, scale),
        at(120, CLIENT_H - 2, scale),
    )?;
    drag(
        c,
        p,
        at(MIN_W - 2, 396, scale),
        at(CLIENT_W - 2, 396, scale),
    )?;
    action(
        run,
        record,
        c,
        &["ctrl", "w"],
        "DOOR_P12 NOTES PROMPT intent=Close",
    )?;
    // Repeated close keeps one modal; Escape restores the same document.
    // The window server logs each delivered close; Notes must not open a
    // second prompt for it. Notes writes its prompt marker before it hands
    // the first close back to the server, so an Alt+F4 sent in between is
    // the same pending close (nothing to log): press again, as a user
    // would, at most three times.
    let mut delivered = false;
    for _ in 0..3 {
        let before = serial(&run.root, record)?.len();
        key(c, &["alt", "f4"])?;
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline {
            if serial(&run.root, record)?
                .get(before..)
                .unwrap_or("")
                .contains("DOOR_P12 CLOSE REQUEST pid=")
            {
                delivered = true;
                break;
            }
            thread::sleep(Duration::from_millis(80));
        }
        if delivered {
            break;
        }
    }
    if !delivered {
        return Err("fresh Notes operation missing: DOOR_P12 CLOSE REQUEST pid=".into());
    }
    let before = serial(&run.root, record)?.len();
    let (cx, cy) = button_centre(2, scale);
    click(c, p, cx, cy)?;
    fresh(
        run,
        record,
        before,
        "DOOR_P12 NOTES PROMPT_CANCEL intent=Close",
    )?;
    action(run, record, c, &["ctrl", "s"], "DOOR_P11 NOTES SAVED")?;
    let kept = checkpoint(run, c, disk, "dialog-close-cancel", b"Keep on cancel\n")?;
    key(c, &["ctrl", "a"])?;
    type_line(c, "Discard close")?;
    action(
        run,
        record,
        c,
        &["ctrl", "w"],
        "DOOR_P12 NOTES PROMPT intent=Close",
    )?;
    // Focus/capture cancellation must not convert a held Save into a release.
    let (sx, sy) = button_centre(0, scale);
    move_to(c, p, sx, sy)?;
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"btn","data":{"down":true,"button":"left"}}]}),
    )?;
    key(c, &["meta_l"])?;
    key(c, &["esc"])?;
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"btn","data":{"down":false,"button":"left"}}]}),
    )?;
    move_to(c, p, 0, 0)?;
    scene::capture_settled(
        run,
        c,
        "notes-dialog-capture-canceled",
        &[save_focus(scale)],
    )?;
    key(c, &["tab"])?;
    let before = serial(&run.root, record)?.len();
    let (dx, dy) = button_centre(1, scale);
    click(c, p, dx, dy)?;
    fresh(
        run,
        record,
        before,
        "DOOR_P12 NOTES PROMPT_DONE intent=Close choice=1",
    )?;
    fresh(run, record, before, "DOOR_P11 CLIENT_EXIT")?;
    let closed = checkpoint(run, c, disk, "dialog-close-discard", b"Keep on cancel\n")?;
    if kept["disk_sha256"] != closed["disk_sha256"] {
        return Err("Don't save/close wrote to the disk".into());
    }
    resolution::activate(&run.root, record, c, 1)?;
    run.check("desktop-notes-dialog-light-workflows",json!({"scale":scale,"save_reload":saved,"discard_reload":discarded,"cancel_close":kept,"discard_close":closed,"discard_disk_exact":true,"reload_selection_exact":true,"capture_cancel":true,"held_button_escape_release":true,"minimum_client":[240,178],"question_warning_ink_checked":true,"repeat_close":true,"native_extra_notes_instances":1}));
    Ok(())
}
pub(super) fn verify(run: &mut Run, image: &str, accelerated: bool) -> Result<(), String> {
    let scale = if accelerated { 3 } else { 1 };
    let disk = run.directory.join("desktop-notes.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut first = launch(run, image, &disk, accelerated)?;
    let mut guarded = Value::Null;
    let result: Result<(), String> = (|| {
        let mut c = connection(&first)?;
        let mut p = (640 * scale, 360 * scale);
        resolution::activate(&run.root, &first, &mut c, 1)?;
        type_line(&mut c, "Original text")?;
        action(run, &first, &mut c, &["ctrl", "s"], "DOOR_P11 NOTES SAVED")?;
        pointer_navigation(run, &first, &mut c, &mut p, &disk, scale)?;
        let original = scene::capture_settled(
            run,
            &mut c,
            "notes-original",
            &[{
                let (x, y) = pixel_at(470, 158, scale);
                (x, y, [255; 3])
            }],
        )?;
        key(&mut c, &["ctrl", "a"])?;
        let selection = selected(run, &mut c, "notes-light-selection", scale as usize, 255)?;
        key(&mut c, &["x"])?;
        let replaced = scene::capture_settled(run, &mut c, "notes-replaced", &[])?;
        if body(&original, scale as usize)? == body(&replaced, scale as usize)? {
            return Err("selection replacement did not change visible document".into());
        }
        key(&mut c, &["ctrl", "z"])?;
        let undone = selected(run, &mut c, "notes-undo-selection", scale as usize, 255)?;
        if body(&selection, scale as usize)? != body(&undone, scale as usize)? {
            return Err("undo failed to restore exact selected glyphs and selection".into());
        }
        key(&mut c, &["ctrl", "y"])?;
        let redone = scene::capture_settled(run, &mut c, "notes-redo", &[])?;
        if body(&replaced, scale as usize)? != body(&redone, scale as usize)? {
            return Err("redo failed to restore replacement".into());
        }
        // Over the body, and later inside the modal panel at the same
        // panel-relative point as the earlier fixture.
        let (tx, ty) = at(603, 281, scale);
        move_to(&mut c, &mut p, tx, ty)?;
        scene::capture_settled(
            run,
            &mut c,
            "notes-before-modal-text-cursor",
            &[(tx as usize, ty as usize, [0; 3])],
        )?;
        action(
            run,
            &first,
            &mut c,
            &["ctrl", "o"],
            "DOOR_P12 NOTES REOPEN_BLOCKED",
        )?;
        scene::capture_settled(
            run,
            &mut c,
            "notes-reopen-dialog",
            &[
                save_focus(scale),
                if scale == 3 {
                    ((tx + 4) as usize, (ty + 5) as usize, [0; 3])
                } else {
                    (tx as usize, ty as usize, [24, 25, 37])
                },
            ],
        )?;
        // Every editor shortcut and an outside click remain behind the modal.
        for keys in [
            &["x"][..],
            &["ctrl", "s"][..],
            &["ctrl", "w"][..],
            &["shift", "f10"][..],
        ] {
            key(&mut c, keys)?;
        }
        // Outside the modal panel, over the editor body.
        let (ox, oy) = at(20, 308, scale);
        click(&mut c, &mut p, ox, oy)?;
        action(
            run,
            &first,
            &mut c,
            &["esc"],
            "DOOR_P12 NOTES PROMPT_CANCEL intent=Reload",
        )?;
        let blocked = scene::capture_settled(run, &mut c, "notes-reopen-canceled", &[])?;
        if body(&replaced, scale as usize)? != body(&blocked, scale as usize)? {
            return Err("reopen guard discarded unsaved visible edits".into());
        }
        guarded = checkpoint(run, &mut c, &disk, "guarded", b"Original text\n")?;
        key(&mut c, &["ctrl", "z"])?;
        key(&mut c, &["right"])?;
        key(&mut c, &["shift", "left"])?;
        key(&mut c, &["backspace"])?;
        key(&mut c, &["ctrl", "z"])?;
        key(&mut c, &["ctrl", "shift", "z"])?;
        key(&mut c, &["ctrl", "z"])?;
        key(&mut c, &["right"])?;
        let restored = scene::capture_settled(run, &mut c, "notes-restored", &[])?;
        if body(&original, scale as usize)? != body(&restored, scale as usize)? {
            return Err("newline erase/undo/redo did not restore original document".into());
        }
        action(
            run,
            &first,
            &mut c,
            &["ctrl", "o"],
            "DOOR_P12 NOTES REOPENED",
        )?;
        modal_light(run, &first, &mut c, &mut p, &disk, scale)?;
        key(&mut c, &["ctrl", "a"])?;
        type_line(&mut c, "Final text")?;
        action(run, &first, &mut c, &["ctrl", "s"], "DOOR_P11 NOTES SAVED")?;
        resolution::activate(&run.root, &first, &mut c, 2)?;
        let before = serial(&run.root, &first)?.len();
        settings_mode(&mut c, true)?;
        fresh(run, &first, before, "SETTINGS SAVED theme=1")?;
        fresh(run, &first, before, "APPEARANCE applied=1")?;
        resolution::activate(&run.root, &first, &mut c, 1)?;
        key(&mut c, &["ctrl", "a"])?;
        selected(
            run,
            &mut c,
            "notes-dark-selection",
            scale as usize,
            NOTES_DARK,
        )?;
        key(&mut c, &["right"])?;
        key(&mut c, &["ctrl", "a"])?;
        type_line(&mut c, "Saved on close")?;
        action(
            run,
            &first,
            &mut c,
            &["ctrl", "w"],
            "DOOR_P12 NOTES PROMPT intent=Close",
        )?;
        scene::capture_settled(
            run,
            &mut c,
            "notes-dark-save-dialog",
            &[save_focus(scale), {
                let (x, y, _, _) = button(0);
                let (x, y) = pixel_at(x + 2, y + 15, scale);
                (x, y, [255; 3])
            }],
        )?;
        let before = serial(&run.root, &first)?.len();
        key(&mut c, &["ret"])?;
        fresh(
            run,
            &first,
            before,
            "DOOR_P12 NOTES PROMPT_DONE intent=Close choice=0",
        )?;
        fresh(run, &first, before, "DOOR_P11 CLIENT_EXIT")?;
        let close_saved = checkpoint(run, &mut c, &disk, "dialog-close-save", b"Saved on close\n")?;
        resolution::activate(&run.root, &first, &mut c, 1)?;
        key(&mut c, &["ctrl", "a"])?;
        type_line(&mut c, "Final text")?;
        action(run, &first, &mut c, &["ctrl", "s"], "DOOR_P11 NOTES SAVED")?;
        run.check("desktop-notes-dialog-dark-close-save",json!({"scale":scale,"checkpoint":close_saved,"real_close_and_relaunch":true,"explicit_save":true}));
        shutdown_clean(run, &first, &mut c, &mut p, scale, 4)
    })();
    if result.is_err()
        && let Ok(mut c) = connection(&first)
    {
        let _ = c.execute("quit", json!({}));
    }
    result?;
    seal_serial(&run.root, &mut first)?;
    let retained = run.directory.join("notes-first-shutdown.raw");
    fs::copy(&disk, &retained).map_err(|e| e.to_string())?;
    let saved = disk_audit(&retained, b"Final text\n")?;
    let mut second = launch(run, image, &disk, accelerated)?;
    let result: Result<(), String> = (|| {
        let mut c = connection(&second)?;
        let mut p = (640 * scale, 360 * scale);
        resolution::activate(&run.root, &second, &mut c, 1)?;
        key(&mut c, &["ctrl", "a"])?;
        selected(
            run,
            &mut c,
            "notes-reboot-selection",
            scale as usize,
            NOTES_DARK,
        )?;
        key(&mut c, &["right"])?;
        type_line(&mut c, "Reopened")?;
        key(&mut c, &["ctrl", "z"])?;
        key(&mut c, &["ctrl", "y"])?;
        action(run, &second, &mut c, &["ctrl", "s"], "DOOR_P11 NOTES SAVED")?;
        scene::capture_settled(run, &mut c, "notes-reboot-saved", &[])?;
        shutdown_clean(run, &second, &mut c, &mut p, scale, 1)
    })();
    if result.is_err()
        && let Ok(mut c) = connection(&second)
    {
        let _ = c.execute("quit", json!({}));
    }
    result?;
    seal_serial(&run.root, &mut second)?;
    let reopened = disk_audit(&disk, b"Final text\nReopened\n")?;
    for field in ["object", "inode", "generation"] {
        if guarded[field] != saved[field] || saved[field] != reopened[field] {
            return Err(format!(
                "Notes save replaced original backing identity: {field}"
            ));
        }
    }
    run.check("desktop-notes-editing", json!({"scale":scale,"accelerated":accelerated,"first":first,"second":second,"guarded":guarded,"saved":saved,"reopened":reopened,"selection_replacement":true,"undo_redo_exact_visible_body":true,"unsaved_reopen_preserved":true,"separate_boots":2,"themes":["light","dark"],"scope":"bounded ASCII editor, checked durable replacement and explicit modal save/discard/cancel; not Unicode/IME/clipboard or full Windows Notepad fidelity"}));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notes_points_follow_the_real_placement_layout_and_resize_bands() {
        use super::window_points::{App, Desk, notes_dialog, notes_window};
        use door_window::graphics as g;
        // Initial placement on the 1280x720 logical desktop of both profiles.
        assert_eq!(
            notes_window().unwrap(),
            (CLIENT_X, CLIENT_Y, CLIENT_W as u32, CLIENT_H as u32)
        );
        // Client offsets from Notes' layout; desktop/workspace/tests/
        // notes_scenario_points.rs checks them against the real viewport,
        // dialog layout and painter.
        assert_eq!(
            (TEXT_X - CLIENT_X, CARET_Y - CLIENT_Y, ROW_Y - CLIENT_Y),
            (16, 91, 96)
        );
        assert_eq!(SELECT_Y - CLIENT_Y, 92);
        assert_eq!((WRAP_WIDTH, BODY_RIGHT - CLIENT_X), (864, 885));
        assert_eq!(BODY_BOTTOM - CLIENT_Y, 89 + 9 * LINE_HEIGHT + 2);
        assert_eq!(
            notes_dialog(896, 524),
            (
                (224, 146, 448, 232),
                [
                    (248, 322, 128, 32),
                    (384, 322, 128, 32),
                    (520, 322, 128, 32)
                ]
            )
        );
        assert_eq!(
            notes_dialog(240, 178),
            (
                (8, 8, 224, 162),
                [(16, 130, 66, 32), (86, 130, 66, 32), (156, 130, 66, 32)]
            )
        );
        assert_eq!(button(0), (248, 322, 128, 32));
        // Button centres (Save, Don't save, Cancel) at 1x and 3x: the panel
        // is centred in the window, which is centred in the work area.
        assert_eq!(button_centre(0, 1), (504, 412));
        assert_eq!(button_centre(1, 1), (640, 412));
        assert_eq!(button_centre(2, 1), (776, 412));
        assert_eq!(button_centre(2, 3), (2328, 1236));
        assert_eq!(save_focus(3), (1335, 1215, [0, 103, 196]));
        // Every pointer point lands on its target region of the real window.
        let mut desk = Desk::new(1280, 720).unwrap();
        let id = desk.open(App::Notes).unwrap();
        let client =
            |desk: &Desk, p: (i32, i32)| desk.hit(p.0, p.1) == Some((id, g::HitRegion::Client));
        for p in [
            (TEXT_X + 1, ROW_Y),
            at(460, 93, 1),
            at(200, 113, 1),
            at(603, 281, 1),
            at(20, 308, 1),
            button_centre(0, 1),
            button_centre(1, 1),
            button_centre(2, 1),
        ] {
            assert!(client(&desk, p), "{p:?}");
        }
        // The resize drags: right edge to the 240 px minimum, bottom edge to
        // 178 px, then back to the placement; each start is on the 4 px band
        // (12 physical px at 3x) of a straight edge.
        let outer = desk.outer(id).unwrap();
        let start = at(CLIENT_W - 2, 396, 1);
        assert_eq!(g::CursorIcon::resize_edges(outer, start.0, start.1), 2);
        let scaled = g::Rect::new(
            outer.x() * 3,
            outer.y() * 3,
            outer.width() * 3,
            outer.height() * 3,
        )
        .unwrap();
        let s3 = at(CLIENT_W - 2, 396, 3);
        assert_eq!(
            g::CursorIcon::resize_edges_scaled(
                scaled,
                s3.0,
                s3.1,
                g::Scale::from_factor(3).unwrap()
            ),
            2
        );
        let s3 = at(120, CLIENT_H - 2, 3);
        assert_eq!(
            g::CursorIcon::resize_edges_scaled(
                scaled,
                s3.0,
                s3.1,
                g::Scale::from_factor(3).unwrap()
            ),
            8
        );
        let narrow = desk.resize_drag(id, start, at(MIN_W - 2, 396, 1)).unwrap();
        assert_eq!(
            narrow,
            g::Rect::new(CLIENT_X, CLIENT_Y, 240, CLIENT_H as u32).unwrap()
        );
        let minimum = desk
            .resize_drag(id, at(120, CLIENT_H - 2, 1), at(120, MIN_H - 2, 1))
            .unwrap();
        assert_eq!(
            minimum,
            g::Rect::new(CLIENT_X, CLIENT_Y, MIN_W as u32, MIN_H as u32).unwrap()
        );
        // Minimum-client dialog points stay on the client.
        for p in [
            at(22, 140, 1),
            at(40, 146, 1),
            at(16, 146, 1),
            at(20, 44, 1),
        ] {
            assert!(client(&desk, p), "{p:?}");
        }
        desk.resize_drag(id, at(120, MIN_H - 2, 1), at(120, CLIENT_H - 2, 1))
            .unwrap();
        let restored = desk
            .resize_drag(id, at(MIN_W - 2, 396, 1), at(CLIENT_W - 2, 396, 1))
            .unwrap();
        assert_eq!(restored, outer);
    }
    fn fixture(clients: usize) -> String {
        let mut text = String::new();
        for (i, (manifest, uid, exit)) in [(1, 1, 80), (2, 1, 0), (3, 2, 0), (4, 3, 0), (11, 3, -1)]
            .into_iter()
            .chain(std::iter::repeat_n((12, 1001, 0), clients))
            .enumerate()
        {
            let id = i + 1;
            let session = usize::from(manifest == 12);
            text.push_str(&format!("DOOR_P08 PROCESS manifest={manifest} pid={id} instance={id} uid={uid} session={session} epoch={session} exit={exit}\n"));
        }
        let count = 5 + clients;
        text.push_str(&format!("DOOR_P08 CLEANUP PASS processes={count} threads={count} leaked=0\nDOOR_P08 DEVICE PASS reads=1 writes=1 flushes=1 sampled_remaining=1 reclaimed=8\n"));
        text
    }
    #[test]
    fn notes_retirement_requires_the_actual_fixture_set_and_complete_summaries() {
        for clients in [1, 2] {
            let valid = fixture(clients);
            assert!(retirement(&valid, clients).is_ok());
            assert!(retirement(&valid, 3).is_err());
            assert!(
                retirement(
                    &valid.replace(
                        "DOOR_P08 DEVICE PASS",
                        "DOOR_USER pid=1 DOOR_P08 DEVICE PASS"
                    ),
                    clients
                )
                .is_err()
            );
            assert!(retirement(&valid.replace("leaked=0", "leaked=1"), clients).is_err());
            assert!(retirement(&valid.replace("reclaimed=8", "reclaimed=7"), clients).is_err());
            assert!(
                retirement(&valid.replace("reclaimed=", "partial_reclaimed="), clients).is_err()
            );
        }
    }
    #[test]
    fn notes_retirement_rejects_reused_forged_and_late_kernel_identities() {
        let valid = fixture(2);
        for invalid in [
            valid.replace("pid=7", "pid=6"),
            valid.replace("instance=7", "instance=6"),
            valid.replace("uid=1001", "uid=3"),
            valid.replace("session=1", "session=0"),
            valid.replace("exit=0", "exit=-1"),
        ] {
            assert!(retirement(&invalid, 2).is_err());
        }
        let (first, rest) = valid.split_once('\n').unwrap();
        assert!(retirement(&format!("{rest}{first}\n"), 2).is_err());
        assert!(retirement(&format!("{valid}{first}\n"), 2).is_err());
    }
}
