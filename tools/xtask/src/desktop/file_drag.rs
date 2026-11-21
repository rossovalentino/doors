//! `file-drag`: File Explorer as a door.dnd source and drop target. Notes
//! saves desktop.txt into /users/1001/notes, Explorer opens that folder and
//! creates "New folder 1", then the mouse drags desktop.txt onto the folder
//! row (a same-volume drop moves, as in Windows 11). The disk proves the
//! result: the same object now lives at "New folder 1/desktop.txt" with its
//! bytes unchanged and the old path gone.
//!
//! Window points come from `desktop/workspace/src/files/scenario_points.rs`
//! (`file_drag_points_hit_the_file_and_the_folder_rows`): Explorer stays at
//! its initial placement, so a screen point is a window point plus
//! `window_points::origin`, at density 3 on the accelerated 3840x2160 profile.
use super::*;

const SCALE: i32 = 3;
/// desktop.txt's row and "New folder 1"'s row (folders list first).
const DRAG_FILE: (i32, i32) = (217, 211);
const DRAG_FOLDER: (i32, i32) = (217, 182);
const NOTES: &str = "/users/1001/notes/desktop.txt";
const MOVED: &str = "/users/1001/notes/New folder 1/desktop.txt";
const FOLDER: &str = "/users/1001/notes/New folder 1";
/// The file-operation engine's record folder (storage-core
/// `persistence::fileops::RECORD_FOLDER`).
const ENGINE_RECORDS: &str = "/state/machine/fileops";

fn at((x, y): (i32, i32)) -> Result<(i32, i32), String> {
    let (ox, oy) = window_points::origin(window_points::App::Files)?;
    Ok(((ox + x) * SCALE, (oy + y) * SCALE))
}
fn fresh(run: &Run, record: &Value, offset: usize, marker: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        let text = serial(&run.root, record)?;
        if text.get(offset..).unwrap_or("").contains(marker) {
            return Ok(());
        }
        if text.contains("KERNEL_PANIC") || text.contains("APP FAILED") {
            return Err(format!("guest failed while waiting for {marker}"));
        }
        if Instant::now() >= deadline {
            return Err(format!("Explorer drag marker missing: {marker}"));
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
fn button(c: &mut qmp::Client, down: bool) -> Result<(), String> {
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"btn","data":{"down":down,"button":"left"}}]}),
    )?;
    thread::sleep(Duration::from_millis(120));
    Ok(())
}
/// A paused copy of the private disk.
fn checkpoint(
    run: &Run,
    c: &mut qmp::Client,
    disk: &std::path::Path,
    name: &str,
) -> Result<std::path::PathBuf, String> {
    c.execute("stop", json!({}))?;
    let path = run.directory.join(format!("drag-{name}.raw"));
    let copied = fs::copy(disk, &path).map_err(|e| e.to_string());
    let resumed = c.execute("cont", json!({}));
    copied?;
    resumed?;
    Ok(path)
}
/// The namespace of a checkpoint: path -> (object, parent, kind) and the
/// object's backing bytes.
type Namespace = (
    std::collections::BTreeMap<String, (u64, u64, u8)>,
    std::collections::BTreeMap<String, Value>,
);
fn namespace(path: &std::path::Path) -> Result<Namespace, String> {
    use crate::persistence::{catalog, journal, tree};
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let home = journal::inspect(&bytes)?.replayed_home(&bytes)?;
    let t = tree::inspect(&home)?;
    let c = catalog::decode(&t)?.ok_or("drag checkpoint has no catalog")?;
    let paths = c
        .paths()?
        .into_iter()
        .map(|(p, n)| (p, (n.id, n.parent, n.kind)))
        .collect();
    let files = t
        .files
        .iter()
        .map(|(name, f)| {
            (
                name.clone(),
                json!({"bytes":f.bytes,"blocks":f.blocks.len()}),
            )
        })
        .collect();
    Ok((paths, files))
}
/// The drop moved the file into the folder: same object, same bytes, the
/// old path gone and nothing else added or removed.
fn moved(before: &std::path::Path, after: &std::path::Path) -> Result<Value, String> {
    let (bp, bf) = namespace(before)?;
    let (ap, af) = namespace(after)?;
    let &(object, _, kind) = bp.get(NOTES).ok_or("desktop.txt missing before the drag")?;
    let &(folder, _, folder_kind) = bp
        .get(FOLDER)
        .ok_or("New folder 1 missing before the drag")?;
    if kind != 1 || folder_kind == 1 || bp.contains_key(MOVED) {
        return Err("drag start state is not a file and an empty folder".into());
    }
    let &(id, parent, _) = ap
        .get(MOVED)
        .ok_or("desktop.txt did not land in New folder 1")?;
    if ap.contains_key(NOTES) {
        return Err("desktop.txt is still in notes (copied, not moved)".into());
    }
    // The drop runs through the file-operation engine, which keeps its
    // records under `/state/machine/fileops` (created on first use), as for
    // GUI deletion (`file_deletion.rs`); everything else must be unchanged.
    let engine = |p: &String| {
        p.as_str() == ENGINE_RECORDS
            || p.strip_prefix(ENGINE_RECORDS)
                .is_some_and(|r| r.starts_with('/'))
    };
    let (user_before, user_after) = (
        bp.keys().filter(|p| !engine(p)).count(),
        ap.keys().filter(|p| !engine(p)).count(),
    );
    if id != object || parent != folder || user_after != user_before {
        return Err(format!(
            "move changed identity or namespace: object {object}->{id}, parent {parent}, paths {user_before}->{user_after}"
        ));
    }
    let backing = format!("obj{object:016x}");
    if bf.get(&backing) != af.get(&backing) {
        return Err("moved file bytes changed".into());
    }
    Ok(json!({
        "before":{"path":before,"sha256":file_hash(before)?},
        "after":{"path":after,"sha256":file_hash(after)?},
        "from":NOTES,"to":MOVED,"object":object,"folder":folder,
        "same_object_same_bytes_old_path_gone":true
    }))
}
pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let disk = run.directory.join("desktop-file-drag.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut record = session::launch_desktop_accelerated(run, image, &disk, false, "3840x2160")?;
    let outcome = drive(run, &record, &disk);
    if outcome.is_err()
        && let Ok(mut c) = connection(&record)
    {
        let _ = scene::capture_settled(run, &mut c, "drag-failure", &[]);
        let _ = c.execute("quit", json!({}));
    }
    let evidence = outcome?;
    seal_serial(&run.root, &mut record)?;
    run.check(
        "p12-file-drag",
        json!({"scenario":"file-drag","session":record,"disk":evidence,
            "scope":"Explorer drag source and drop target on the accelerated desktop; a same-volume drop onto a folder row moves the file"}),
    );
    Ok(())
}
fn drive(run: &mut Run, record: &Value, disk: &std::path::Path) -> Result<Value, String> {
    let mut c = connection(record)?;
    let mut p = (640 * SCALE, 360 * SCALE);
    // Notes saves desktop.txt into the notes folder.
    resolution::activate(&run.root, record, &mut c, 1)?;
    type_line(&mut c, "Dragged between folders")?;
    action(run, record, &mut c, &["ctrl", "s"], "DOOR_P11 NOTES SAVED")?;
    action(run, record, &mut c, &["ctrl", "w"], "DOOR_P11 CLIENT_EXIT")?;
    // Explorer opens that folder and creates "New folder 1" next to it.
    resolution::activate(&run.root, record, &mut c, 0)?;
    key(&mut c, &["ctrl", "l"])?;
    for ch in r"C:\Users\1001\notes".chars() {
        c.execute(
            "send-key",
            json!({"keys":session::keys(ch)?,"hold-time":20}),
        )?;
        thread::sleep(Duration::from_millis(70));
    }
    action(
        run,
        record,
        &mut c,
        &["ret"],
        "FILES ADDRESS path=/users/1001/notes history=",
    )?;
    key(&mut c, &["ctrl", "shift", "n"])?;
    move_to(&mut c, &mut p, 0, 0)?;
    scene::capture_settled(run, &mut c, "drag-start-folder", &[])?;
    let before = checkpoint(run, &mut c, disk, "before")?;
    // Press on the file, leave the drag rectangle, wait for the drag.
    let offset = serial(&run.root, record)?.len();
    let (fx, fy) = at(DRAG_FILE)?;
    move_to(&mut c, &mut p, fx, fy)?;
    button(&mut c, true)?;
    move_to(&mut c, &mut p, fx + 4 * SCALE, fy - 6 * SCALE)?;
    fresh(run, record, offset, "DOOR_P12 FILES DRAG count=1")?;
    // Over the folder row the server labels the drag "Move to New folder 1".
    let (dx, dy) = at(DRAG_FOLDER)?;
    move_to(&mut c, &mut p, dx, dy)?;
    thread::sleep(Duration::from_millis(400));
    scene::capture_settled(run, &mut c, "drag-move-to-new-folder", &[])?;
    button(&mut c, false)?;
    fresh(
        run,
        record,
        offset,
        "DOOR_P12 FILES DROP effect=move done=1 skipped=0 folder=/users/1001/notes/New folder 1",
    )?;
    move_to(&mut c, &mut p, 0, 0)?;
    scene::capture_settled(run, &mut c, "drag-dropped", &[])?;
    let after = checkpoint(run, &mut c, disk, "after")?;
    let evidence = moved(&before, &after)?;
    key(&mut c, &["esc"])?;
    start_shut_down(&mut c)?;
    wait_serial(
        &run.root,
        record,
        "DOOR_P08 CLEANUP PASS",
        Duration::from_secs(25),
    )?;
    let text = serial(&run.root, record)?;
    if !text.contains("leaked=0") || text.contains("APP FAILED") || text.contains("SERVER FAILED") {
        return Err("Explorer drag session did not shut down cleanly".into());
    }
    Ok(evidence)
}
