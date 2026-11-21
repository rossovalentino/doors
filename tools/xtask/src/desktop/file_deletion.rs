//! Actual safe-default permanent removal, cancellation and two-boot persistence.
use super::*;
// Window points of Explorer, logical (times `scale`; the accelerated profile
// is the same 1280x720 desktop at density 3). Explorer stays at its initial
// placement, 896x524 at (192, 74) (`graphics::initial_window`), and draws its
// own caption, so `at` adds that origin from `window_points`. The removal
// dialog is centred on its whole client. The points come from Explorer's
// layout, asserted by desktop/workspace/src/files/scenario_points.rs; the resize
// grips by `tests`.
/// Cancel's left edge (its keyboard-focus border) and a press on Delete.
const CANCEL_EDGE: (i32, i32) = (452, 335);
const DELETE_PRESS: (i32, i32) = (346, 338);
/// Inside Delete's fill, which shows the pressed accent while held.
const DELETE_FILL: (i32, i32) = (256, 325);
/// Blank dialog panel (dark theme).
const DIALOG_BLANK: (i32, i32) = (621, 275);
/// Resize grips 3 DIP inside the right and bottom edges: at 896x524, then
/// at the 240 DIP minimum width, then at 240x178 (the scenario's minimum
/// client), and back at 896x178.
const RIGHT_GRIP: (i32, i32) = (893, 396);
const MIN_RIGHT: (i32, i32) = (237, 396);
const BOTTOM_GRIP: (i32, i32) = (40, 521);
const MIN_BOTTOM: (i32, i32) = (40, 175);
const MIN_RIGHT_GRIP: (i32, i32) = (237, 156);
const RESTORED_RIGHT: (i32, i32) = (893, 156);
const LOW_BOTTOM_GRIP: (i32, i32) = (260, 175);
const RESTORED_BOTTOM: (i32, i32) = (260, 521);
/// Physical screen point of window point `(x, y)`.
fn at((x, y): (i32, i32), scale: i32) -> Result<(i32, i32), String> {
    let (ox, oy) = window_points::origin(window_points::App::Files)?;
    Ok(((ox + x) * scale, (oy + y) * scale))
}
fn probe(point: (i32, i32), scale: i32, color: [u8; 3]) -> Result<(usize, usize, [u8; 3]), String> {
    let (x, y) = at(point, scale)?;
    Ok((x as usize, y as usize, color))
}
fn fresh(run: &Run, record: &Value, offset: usize, marker: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        if serial(&run.root, record)?
            .get(offset..)
            .unwrap_or("")
            .contains(marker)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("fresh Files deletion marker missing: {marker}"));
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
fn address(run: &Run, record: &Value, c: &mut qmp::Client, notes: bool) -> Result<(), String> {
    key(c, &["ctrl", "l"])?;
    for ch in if notes {
        r"C:\Users\1001\notes"
    } else {
        r"C:\Users\1001"
    }
    .chars()
    {
        c.execute(
            "send-key",
            json!({"keys":session::keys(ch)?,"hold-time":20}),
        )?;
        thread::sleep(Duration::from_millis(70));
    }
    action(
        run,
        record,
        c,
        &["ret"],
        if notes {
            "FILES ADDRESS path=/users/1001/notes history="
        } else {
            "FILES ADDRESS path=/users/1001 history="
        },
    )?;
    // As in Windows, navigation selects nothing; Home selects the first item.
    key(c, &["home"])
}
fn checkpoint(
    run: &Run,
    c: &mut qmp::Client,
    disk: &std::path::Path,
    name: &str,
) -> Result<std::path::PathBuf, String> {
    c.execute("stop", json!({}))?;
    let path = run.directory.join(format!("remove-{name}.raw"));
    let copied = fs::copy(disk, &path).map_err(|e| e.to_string());
    let resumed = c.execute("cont", json!({}));
    copied?;
    resumed?;
    Ok(path)
}
fn removed(
    before: &std::path::Path,
    after: &std::path::Path,
    target: &str,
) -> Result<Value, String> {
    use crate::persistence::{catalog, journal, tree};
    let a = fs::read(before).map_err(|e| e.to_string())?;
    let b = fs::read(after).map_err(|e| e.to_string())?;
    let aj = journal::inspect(&a)?;
    let bj = journal::inspect(&b)?;
    if aj.selected().committed || bj.selected().committed {
        return Err("GUI removal checkpoint dirty journal".into());
    }
    let at = tree::inspect(&aj.replayed_home(&a)?)?;
    let bt = tree::inspect(&bj.replayed_home(&b)?)?;
    let ac = catalog::decode(&at)?.ok_or("GUI removal old catalog")?;
    let bc = catalog::decode(&bt)?.ok_or("GUI removal new catalog")?;
    let ap = ac.paths()?;
    let bp = bc.paths()?;
    let item = ap.get(target).ok_or("GUI removal source")?;
    // P14 deletes run through the file-operation engine, which journals its
    // operation record under `/state/machine/fileops` (creating that folder on
    // first use) in its own transactions. Those engine objects (the folder,
    // its records, and records allocated and dismissed in between) are the
    // only other permitted change; everything else stays byte-exact.
    // storage-core `persistence::fileops::RECORD_FOLDER`.
    let folder = "/state/machine/fileops";
    let engine_ids: std::collections::BTreeSet<u64> = ap
        .iter()
        .chain(bp.iter())
        .filter(|(p, _)| {
            p.as_str() == folder || p.strip_prefix(folder).is_some_and(|r| r.starts_with('/'))
        })
        .map(|(_, n)| n.id)
        .collect();
    let engine = |id: u64| engine_ids.contains(&id) || id >= ac.next_object;
    if bp.contains_key(target)
        || bp.iter().any(|(p, n)| !ap.contains_key(p) && !engine(n.id))
        || ap
            .iter()
            .any(|(p, n)| p != target && !bp.contains_key(p) && !engine(n.id))
        || bc.revision <= ac.revision
        || bc.revision > ac.revision + 16
        || bc.next_object < ac.next_object
        || bc.next_key != ac.next_key
    {
        return Err("GUI removal exact namespace/counters".into());
    }
    // Node slots (catalog v4/v5 keep per-index access times at 15488/66560;
    // a removal also stamps the parent folder modified time in its record
    // bytes 216..224).
    let slots = |bytes: &[u8]| -> Vec<(usize, usize, u64)> {
        (0..32)
            .map(|i| (i, 128 + i * 224))
            .chain(
                (32..256)
                    .map(|i| (i, 16384 + (i - 32) * 224))
                    .take_while(|(_, start)| start + 224 <= 66560.min(bytes.len())),
            )
            .map(|(index, start)| {
                let id = u64::from_le_bytes(bytes[start..start + 8].try_into().unwrap());
                (start, index, id)
            })
            .collect()
    };
    let (before_slots, after_slots) = (slots(ac.bytes), slots(bc.bytes));
    let record = |id: u64| {
        before_slots
            .iter()
            .rev()
            .find(|s| s.2 == id)
            .map(|&(start, index, _)| (start, index))
    };
    let accessed = |index: usize| {
        if index < 32 {
            15488 + index * 8
        } else {
            66560 + (index - 32) * 8
        }
    };
    let timed = u32::from_le_bytes(ac.bytes[8..12].try_into().unwrap()) >= 4;
    let (offset, index) = record(item.id).ok_or("GUI removed record offset")?;
    let mut allowed = vec![false; ac.bytes.len()];
    let mut allow = |range: std::ops::Range<usize>| {
        for i in range.start.min(allowed.len())..range.end.min(allowed.len()) {
            allowed[i] = true;
        }
    };
    // Header: next object (engine records) and revision/CRC.
    allow(16..24);
    allow(32..44);
    allow(offset..offset + 224);
    if timed {
        allow(accessed(index)..accessed(index) + 8);
    }
    let mut stamped = vec![item.parent];
    for (&(start, index, old), &(_, _, new)) in before_slots.iter().zip(&after_slots) {
        if (old != 0 && engine(old)) || (new != 0 && engine(new)) {
            allow(start..start + 224);
            if timed {
                allow(accessed(index)..accessed(index) + 8);
            }
        }
    }
    for node in ap.values().chain(bp.values()) {
        if engine(node.id) {
            stamped.push(node.parent);
        }
    }
    if timed {
        for parent in stamped {
            if let Some((start, _)) = record(parent) {
                allow(start + 216..start + 224);
            }
        }
    }
    if ac.bytes.len() != bc.bytes.len()
        || bc.bytes[offset..offset + 224].iter().any(|x| *x != 0)
        || timed
            && bc.bytes[accessed(index)..accessed(index) + 8]
                .iter()
                .any(|x| *x != 0)
        || ac
            .bytes
            .iter()
            .zip(bc.bytes)
            .enumerate()
            .any(|(i, (a, b))| a != b && !allowed[i])
    {
        return Err("GUI removal changed unrelated catalog/ACL/settings".into());
    }
    let backing = format!("obj{:016x}", item.id);
    let engine_file = |name: &str| {
        name.strip_prefix("obj")
            .and_then(|hex| u64::from_str_radix(hex, 16).ok())
            .is_some_and(engine)
    };
    for (name, file) in &at.files {
        if name == &backing {
            if bt.files.contains_key(name) {
                return Err("GUI removed backing still reachable".into());
            }
            continue;
        }
        if engine_file(name) {
            continue;
        }
        let next = bt
            .files
            .get(name)
            .ok_or("GUI removal lost unrelated backing")?;
        if file.inode != next.inode
            || file.generation != next.generation
            || file.blocks != next.blocks
            || (name != "door.catalog"
                && name != crate::persistence::catalog::HIVE
                && file.bytes != next.bytes)
        {
            return Err("GUI removal changed unrelated backing identity/bytes/allocation".into());
        }
    }
    if bt
        .files
        .keys()
        .any(|name| !at.files.contains_key(name) && !engine_file(name))
    {
        return Err("GUI removal added an unrelated backing".into());
    }
    let engine_use = |files: &std::collections::BTreeMap<String, tree::File>| {
        files
            .iter()
            .filter(|(name, _)| engine_file(name))
            .fold((0u64, 0u64), |(n, blocks), (_, f)| {
                (n + 1, blocks + f.blocks.len() as u64)
            })
    };
    let (engine_before, engine_blocks_before) = engine_use(&at.files);
    let (engine_after, engine_blocks_after) = engine_use(&bt.files);
    let files = u64::from(item.kind == 1);
    let blocks = at.files.get(&backing).map_or(0, |f| f.blocks.len()) as u64;
    if bt.files.len() as u64 + files + engine_before != at.files.len() as u64 + engine_after
        || bt.free_blocks + engine_blocks_after != at.free_blocks + blocks + engine_blocks_before
        || bt.free_inodes + engine_after != at.free_inodes + files + engine_before
    {
        return Err("GUI removal allocation count mismatch".into());
    }
    if a.len() != b.len()
        || a.iter().zip(&b).enumerate().any(|(i, (a, b))| {
            a != b
                && !(133120 * 512..198656 * 512).contains(&i)
                && !(202752 * 512..206848 * 512).contains(&i)
        })
    {
        return Err("GUI removal modified outside home/journal".into());
    }
    Ok(
        json!({"before":{"path":before,"sha256":file_hash(before)?},"after":{"path":after,"sha256":file_hash(after)?},"target":target,"object":item.id,"inode":item.inode,"generation":item.generation,"revision_before":ac.revision,"revision_after":bc.revision,"journal_before":aj.summary(),"journal_after":bj.summary(),"exact_unrelated_catalog_payload_allocation":true}),
    )
}
fn same_client(a: &scene::Pixels, b: &scene::Pixels, scale: i32) -> Result<(), String> {
    // Explorer's client below its 40 DIP caption strip.
    for y in 64 * scale..648 * scale {
        for x in 140 * scale..1140 * scale {
            if a.pixel(x as usize, y as usize)? != b.pixel(x as usize, y as usize)? {
                return Err("Files cancel/release changed client pixels".into());
            }
        }
    }
    Ok(())
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
        return Err("Files deletion cleanup failed".into());
    }
    Ok(())
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
pub(super) fn verify(run: &mut Run, image: &str, accelerated: bool) -> Result<(), String> {
    let scale = if accelerated { 3 } else { 1 };
    let disk = run.directory.join("desktop-file-deletion.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut first = launch(run, image, &disk, accelerated)?;
    let mut deltas = Vec::new();
    let mut final_snapshot = None;
    let outcome: Result<(), String> = (|| {
        let mut c = connection(&first)?;
        let mut p = (640 * scale, 360 * scale);
        resolution::activate(&run.root, &first, &mut c, 1)?;
        type_line(&mut c, "Permanent removal keeps neighbors")?;
        action(run, &first, &mut c, &["ctrl", "s"], "DOOR_P11 NOTES SAVED")?;
        action(run, &first, &mut c, &["ctrl", "w"], "DOOR_P11 CLIENT_EXIT")?;
        resolution::activate(&run.root, &first, &mut c, 0)?;
        address(run, &first, &mut c, true)?;
        move_to(&mut c, &mut p, 0, 0)?;
        let baseline = scene::capture_settled(run, &mut c, "remove-original-client", &[])?;
        let initial = checkpoint(run, &mut c, &disk, "initial")?;
        action(
            run,
            &first,
            &mut c,
            &["shift", "delete"],
            "FILES REMOVE_PROMPT",
        )?;
        scene::capture_settled(
            run,
            &mut c,
            "remove-light-default-cancel",
            &[probe(CANCEL_EDGE, scale, [32; 3])?],
        )?;
        for keys in [&["f2"][..], &["ctrl", "w"][..], &["shift", "f10"][..]] {
            key(&mut c, keys)?;
        }
        action(
            run,
            &first,
            &mut c,
            &["ret"],
            "FILES REMOVE_CANCEL refresh=false",
        )?;
        let cancel = checkpoint(run, &mut c, &disk, "cancel-enter")?;
        if file_hash(&initial)? != file_hash(&cancel)? {
            return Err("default Cancel wrote disk".into());
        }
        let after = scene::capture_settled(run, &mut c, "remove-after-default-cancel", &[])?;
        same_client(&baseline, &after, scale)?;
        action(
            run,
            &first,
            &mut c,
            &["shift", "delete"],
            "FILES REMOVE_PROMPT",
        )?;
        let (x, y) = at(DELETE_PRESS, scale)?;
        move_to(&mut c, &mut p, x, y)?;
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":true,"button":"left"}}]}),
        )?;
        scene::capture_settled(
            run,
            &mut c,
            "remove-held-delete",
            &[probe(DELETE_FILL, scale, [0, 80, 160])?],
        )?;
        action(run, &first, &mut c, &["esc"], "FILES REMOVE_CANCEL")?;
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":false,"button":"left"}}]}),
        )?;
        move_to(&mut c, &mut p, 0, 0)?;
        let after = scene::capture_settled(run, &mut c, "remove-held-dismiss-release", &[])?;
        same_client(&baseline, &after, scale)?;
        let held = checkpoint(run, &mut c, &disk, "held-cancel")?;
        if file_hash(&held)? != file_hash(&initial)? {
            return Err("held-delete cancel wrote disk".into());
        }
        action(
            run,
            &first,
            &mut c,
            &["shift", "delete"],
            "FILES REMOVE_PROMPT",
        )?;
        scene::drag(
            &mut c,
            &mut p,
            at(RIGHT_GRIP, scale)?,
            at(MIN_RIGHT, scale)?,
        )?;
        scene::drag(
            &mut c,
            &mut p,
            at(BOTTOM_GRIP, scale)?,
            at(MIN_BOTTOM, scale)?,
        )?;
        move_to(&mut c, &mut p, 0, 0)?;
        let minimum = scene::capture_settled(run, &mut c, "remove-minimum-dialog", &[])?;
        // Check the item and warning both retain visible glyph coverage
        // (dialog bands 20..44 and 48..106, x 16..224 of the 240x178 client).
        let (ox, oy) = window_points::origin(window_points::App::Files)?;
        for (top, bottom) in [(20, 44), (48, 106)] {
            let mut ink = 0;
            for y in (oy + top) * scale..(oy + bottom) * scale {
                for x in (ox + 16) * scale..(ox + 224) * scale {
                    if minimum
                        .pixel(x as usize, y as usize)?
                        .iter()
                        .all(|v| *v < 120)
                    {
                        ink += 1;
                    }
                }
            }
            if ink < 30 * scale {
                return Err("minimum delete dialog lost question/item/warning".into());
            }
        }
        action(run, &first, &mut c, &["esc"], "FILES REMOVE_CANCEL")?;
        scene::drag(
            &mut c,
            &mut p,
            at(MIN_RIGHT_GRIP, scale)?,
            at(RESTORED_RIGHT, scale)?,
        )?;
        scene::drag(
            &mut c,
            &mut p,
            at(LOW_BOTTOM_GRIP, scale)?,
            at(RESTORED_BOTTOM, scale)?,
        )?;
        key(&mut c, &["ctrl", "shift", "n"])?;
        key(&mut c, &["home"])?;
        scene::capture_settled(run, &mut c, "remove-new-empty-folder", &[])?;
        let folder_before = checkpoint(run, &mut c, &disk, "folder-before")?;
        action(
            run,
            &first,
            &mut c,
            &["shift", "delete"],
            "FILES REMOVE_PROMPT",
        )?;
        key(&mut c, &["tab"])?;
        action(run, &first, &mut c, &["ret"], "FILES REMOVED")?;
        let folder_after = checkpoint(run, &mut c, &disk, "folder-after")?;
        deltas.push(removed(
            &folder_before,
            &folder_after,
            "/users/1001/notes/New folder 1",
        )?);
        address(run, &first, &mut c, false)?;
        key(&mut c, &["ctrl", "f"])?;
        for ch in "notes".chars() {
            c.execute(
                "send-key",
                json!({"keys":session::keys(ch)?,"hold-time":20}),
            )?;
            thread::sleep(Duration::from_millis(70));
        }
        key(&mut c, &["ret"])?;
        let nonempty_before = checkpoint(run, &mut c, &disk, "nonempty-before")?;
        action(
            run,
            &first,
            &mut c,
            &["shift", "delete"],
            "FILES REMOVE_PROMPT",
        )?;
        let (x, y) = at(DELETE_PRESS, scale)?;
        move_to(&mut c, &mut p, x, y)?;
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":true,"button":"left"}}]}),
        )?;
        scene::capture_settled(
            run,
            &mut c,
            "remove-nonempty-delete-held",
            &[probe(DELETE_FILL, scale, [0, 80, 160])?],
        )?;
        // P14 deletes a non-empty folder recursively, as Windows does (the
        // P12 refusal is gone; p14-files qualifies the delete operation).
        // While Delete is held the dialog still owns the pointer: Esc
        // cancels the dialog and the capture, and the late release is
        // ignored rather than activating Delete.
        let attempted = serial(&run.root, &first)?.len();
        action(run, &first, &mut c, &["esc"], "FILES REMOVE_CANCEL")?;
        fresh(run, &first, attempted, "MODAL_KEY_CAPTURE_CANCELED")?;
        let released = serial(&run.root, &first)?.len();
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":false,"button":"left"}}]}),
        )?;
        fresh(run, &first, released, "MODAL_POINTER_RELEASE_IGNORED")?;
        thread::sleep(Duration::from_millis(500));
        let tail = serial(&run.root, &first)?;
        let tail = tail.get(attempted..).unwrap_or("");
        if tail.contains("FILEOP begin") || tail.contains("FILES REMOVED") {
            return Err("held release deleted the non-empty folder after Esc".into());
        }
        scene::capture_settled(run, &mut c, "remove-nonempty-cancelled", &[])?;
        let nonempty_after = checkpoint(run, &mut c, &disk, "nonempty-after")?;
        if file_hash(&nonempty_before)? != file_hash(&nonempty_after)? {
            return Err("cancelled nonempty removal wrote disk".into());
        }
        resolution::activate(&run.root, &first, &mut c, 2)?;
        let offset = serial(&run.root, &first)?.len();
        settings_mode(&mut c, true)?;
        fresh(run, &first, offset, "SETTINGS SAVED theme=1")?;
        fresh(run, &first, offset, "APPEARANCE applied=1")?;
        resolution::activate(&run.root, &first, &mut c, 0)?;
        address(run, &first, &mut c, true)?;
        let before = checkpoint(run, &mut c, &disk, "file-before")?;
        action(
            run,
            &first,
            &mut c,
            &["shift", "delete"],
            "FILES REMOVE_PROMPT",
        )?;
        move_to(&mut c, &mut p, 0, 0)?;
        scene::capture_settled(
            run,
            &mut c,
            "remove-dark-default-cancel",
            &[
                // Explorer's dark surface (25) and Cancel's light focus border.
                probe(DIALOG_BLANK, scale, [25; 3])?,
                probe(CANCEL_EDGE, scale, [240; 3])?,
            ],
        )?;
        let offset = serial(&run.root, &first)?.len();
        let (x, y) = at(DELETE_PRESS, scale)?;
        click(&mut c, &mut p, x, y)?;
        fresh(run, &first, offset, "FILES REMOVED")?;
        let after = checkpoint(run, &mut c, &disk, "file-after")?;
        let mut delta = removed(&before, &after, "/users/1001/notes/desktop.txt")?;
        for key in ["before", "after"] {
            let raw = delta[key]["path"].as_str().ok_or("delta path")?;
            delta[key]["path"] = json!(relative(&run.root, &std::path::PathBuf::from(raw))?);
        }
        deltas.push(delta);
        final_snapshot = Some(after);
        scene::capture_settled(run, &mut c, "remove-dark-empty-folder", &[])?;
        shutdown_clean(run, &first, &mut c, &mut p, scale)
    })();
    if outcome.is_err()
        && let Ok(mut c) = connection(&first)
    {
        let _ = c.execute("quit", json!({}));
    }
    outcome?;
    seal_serial(&run.root, &mut first)?;
    let first_retirement = shutdown::retirement(&serial(&run.root, &first)?, 3)?;
    let saved = run.directory.join("remove-first-shutdown.raw");
    fs::copy(&disk, &saved).map_err(|e| e.to_string())?;
    if file_hash(&saved)?
        != file_hash(
            final_snapshot
                .as_ref()
                .ok_or("GUI deletion final snapshot")?,
        )?
    {
        return Err("shutdown changed acknowledged removal state".into());
    }
    let mut second = launch(run, image, &disk, accelerated)?;
    let outcome: Result<(), String> = (|| {
        let mut c = connection(&second)?;
        let mut p = (640 * scale, 360 * scale);
        resolution::activate(&run.root, &second, &mut c, 0)?;
        address(run, &second, &mut c, true)?;
        move_to(&mut c, &mut p, 0, 0)?;
        scene::capture_settled(run, &mut c, "remove-empty-after-reboot", &[])?;
        shutdown_clean(run, &second, &mut c, &mut p, scale)
    })();
    if outcome.is_err()
        && let Ok(mut c) = connection(&second)
    {
        let _ = c.execute("quit", json!({}));
    }
    outcome?;
    seal_serial(&run.root, &mut second)?;
    let second_retirement = shutdown::retirement(&serial(&run.root, &second)?, 1)?;
    if file_hash(&saved)? != file_hash(&disk)? {
        return Err("read-only deletion reboot changed disk".into());
    }
    run.check("desktop-files-checked-remove",json!({"scale":scale,"accelerated":accelerated,"first":first,"second":second,"retirement":[first_retirement,second_retirement],"deltas":deltas,"final_disk":{"path":relative(&run.root,&disk)?,"sha256":file_hash(&disk)?},"themes":["light","dark"],"default_cancel_no_write":true,"held_escape_release_no_write":true,"held_nonempty_escape_release_no_write":true,"modal_input_trap":true,"nonempty_recursive_delete":"p14-files","minimum_client":[240,178],"separate_boots":2,"scope":"Permanent one-file/empty-folder deletion through the P14 file-operation engine, ASCII <=32, not trash/restore, native I/O failure injection or complete Explorer fidelity"}));
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::file_mutations::resize_edges;
    const RIGHT: u8 = 2;
    const BOTTOM: u8 = 8;
    /// Each grip is on the edge of the size the previous drag left, and the
    /// drag moves that edge to the intended size (240x178, then 896x524).
    #[test]
    fn minimum_dialog_resize_grips_follow_each_size() {
        assert_eq!(
            window_points::origin(window_points::App::Files),
            Ok((192, 74))
        );
        let mut rect = (192, 74, 896u32, 524u32);
        for (grip, to, edge, size) in [
            (RIGHT_GRIP, MIN_RIGHT, RIGHT, (240, 524)),
            (BOTTOM_GRIP, MIN_BOTTOM, BOTTOM, (240, 178)),
            (MIN_RIGHT_GRIP, RESTORED_RIGHT, RIGHT, (896, 178)),
            (LOW_BOTTOM_GRIP, RESTORED_BOTTOM, BOTTOM, (896, 524)),
        ] {
            let screen = at(grip, 1).unwrap();
            assert_eq!(
                resize_edges(rect, screen),
                Some(edge),
                "{grip:?} on {rect:?}"
            );
            let (dx, dy) = (to.0 - grip.0, to.1 - grip.1);
            rect.2 = (rect.2 as i32 + if edge == RIGHT { dx } else { 0 }) as u32;
            rect.3 = (rect.3 as i32 + if edge == BOTTOM { dy } else { 0 }) as u32;
            assert_eq!((rect.2, rect.3), size, "{grip:?}");
        }
    }
}
