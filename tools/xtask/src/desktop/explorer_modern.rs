//! Current Explorer behavior on an isolated persistent guest volume, across two boots.
use super::*;
fn fresh(run: &Run, record: &Value, offset: usize, marker: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        let serial = serial(&run.root, record)?;
        if serial.get(offset..).unwrap_or("").contains(marker) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Explorer acknowledgement missing: {marker}; tail={}",
                serial.get(offset..).unwrap_or("")
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}
fn gesture(
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
fn address(run: &Run, record: &Value, c: &mut qmp::Client) -> Result<(), String> {
    let offset = serial(&run.root, record)?.len();
    key(c, &["ctrl", "l"])?;
    type_line(c, r"C:\Users\1001\notes")?;
    fresh(run, record, offset, "FILES ADDRESS path=/users/1001/notes")
}
pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let disk = run.directory.join("explorer-modern-private.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut sessions = Vec::new();
    let mut retirements = Vec::new();
    let mut original_object = 0;
    for boot in 0..2 {
        let mut record =
            session::launch_desktop_accelerated(run, image, &disk, false, "1920x1080")?;
        let result: Result<(), String> = (|| {
            let mut c = connection(&record)?;
            let mut p = (960, 540);
            if boot == 0 {
                resolution::activate(&run.root, &record, &mut c, 1)?;
                key(&mut c, &["ctrl", "a"])?;
                type_line(&mut c, "Explorer preview proof")?;
                gesture(run, &record, &mut c, &["ctrl", "s"], "DOOR_P11 NOTES SAVED")?;
                key(&mut c, &["ctrl", "w"])?;
            }
            resolution::activate(&run.root, &record, &mut c, 0)?;
            key(&mut c, &["meta_l", "up"])?;
            address(run, &record, &mut c)?;
            let preview_before = serial(&run.root, &record)?.len();
            if boot == 0 {
                key(&mut c, &["home"])?;
            }
            if boot == 1 {
                key(&mut c, &["ctrl", "f"])?;
                type_line(&mut c, "preview-proof")?;
            }
            gesture(run, &record, &mut c, &["alt", "p"], "FILES INSPECT object=")?;
            let text = serial(&run.root, &record)?;
            let inspect = text
                .lines()
                .rev()
                .find(|line| line.contains("FILES INSPECT object="))
                .ok_or("Explorer inspector missing")?;
            let object = inspect
                .split_whitespace()
                .find_map(|word| word.strip_prefix("object="))
                .ok_or("Explorer object ID missing")?
                .parse::<u64>()
                .map_err(|e| e.to_string())?;
            fresh(
                run,
                &record,
                preview_before,
                &format!("FILES PREVIEW object={object}"),
            )?;
            if boot == 0 {
                original_object = object;
            } else if object != original_object || !inspect.contains("tags=1") {
                return Err("Explorer tag/selection identity did not survive reboot".into());
            }
            scene::capture_settled(
                run,
                &mut c,
                &format!("explorer-modern-preview-boot{boot}"),
                &[],
            )?;
            if boot == 0 {
                // Maximized 1920x1080 is 1x; tabs are within the40-DIP caption.
                // Inspector Tags tab and Create tag field (pinned by
                // files::scenario_points::modern_inspector_tag_points_follow_the_layout).
                click(&mut c, &mut p, 1857, 333)?; // Tags tab.
                click(&mut c, &mut p, 1739, 392)?; // Create tag field.
                let offset = serial(&run.root, &record)?.len();
                type_line(&mut c, "Build review")?;
                fresh(run, &record, offset, "FILES TAGS SAVED names=1 bindings=1")?;
                scene::capture_settled(run, &mut c, "explorer-modern-user-tag", &[])?;
                gesture(run, &record, &mut c, &["f2"], "FILES RENAME BEGIN")?;
                let offset = serial(&run.root, &record)?.len();
                type_line(&mut c, "preview-proof")?;
                fresh(run, &record, offset, "FILES RENAME COMMITTED")?;
                gesture(
                    run,
                    &record,
                    &mut c,
                    &["ctrl", "t"],
                    "FILES TAB new=1 count=2",
                )?;
                key(&mut c, &["ctrl", "l"])?;
                type_line(&mut c, r"C:\Users\1001\Documents")?;
                gesture(
                    run,
                    &record,
                    &mut c,
                    &["ctrl", "shift", "tab"],
                    "FILES TAB active=0 count=2 path=/users/1001/notes",
                )?;
                scene::capture_settled(run, &mut c, "explorer-modern-tabs-and-rename", &[])?;
                key(&mut c, &["ctrl", "w"])?; // Close one tab, preserving the other window/tab.
            }
            key(&mut c, &["ctrl", "w"])?;
            start_shut_down(&mut c)?;
            wait_serial(
                &run.root,
                &record,
                "DOOR_P08 CLEANUP PASS",
                Duration::from_secs(25),
            )?;
            let text = serial(&run.root, &record)?;
            if text.contains("APP FAILED")
                || text.contains("SERVER FAILED")
                || !text.contains("leaked=0")
            {
                return Err("Explorer guest lifecycle/cleanup failed".into());
            }
            Ok(())
        })();
        if result.is_err()
            && let Ok(mut c) = connection(&record)
        {
            let _ = scene::capture_settled(run, &mut c, "explorer-modern-failure", &[]);
            let _ = c.execute("quit", json!({}));
        }
        result?;
        seal_serial(&run.root, &mut record)?;
        let expected: &[i64] = if boot == 0 { &[1, 2] } else { &[1] };
        retirements.push(shutdown::retirement_commands(
            &serial(&run.root, &record)?,
            expected,
        )?);
        sessions.push(record);
    }
    let raw = fs::read(&disk).map_err(|e| e.to_string())?;
    let journal = crate::persistence::journal::inspect(&raw)?;
    if journal.selected().committed {
        return Err("Explorer journal was not clean".into());
    }
    let tree = crate::persistence::tree::inspect(&journal.replayed_home(&raw)?)?;
    let catalog = crate::persistence::catalog::decode(&tree)?.ok_or("Explorer catalog absent")?;
    let paths = catalog.paths()?;
    let file = paths
        .get("/users/1001/notes/preview-proof.txt")
        .ok_or("Explorer rename not durable")?;
    if file.id != original_object
        || file.owner != 1001
        || tree
            .files
            .get(&format!("obj{:016x}", file.id))
            .is_none_or(|f| f.bytes != b"Explorer preview proof\n")
    {
        return Err("Explorer changed selected identity or payload".into());
    }
    let tags = paths
        .get("/users/1001/.door-explorer-tags")
        .ok_or("Explorer tag catalog not durable")?;
    let bytes = &tree
        .files
        .get(&format!("obj{:016x}", tags.id))
        .ok_or("Explorer tag backing absent")?
        .bytes;
    // Independent wire audit: one user-created label and one exact stable-ID binding.
    let mut expected = b"DOORTAG1\x01\x01\x00\x0cBuild review".to_vec();
    expected.extend_from_slice(&original_object.to_le_bytes());
    expected.push(1);
    if bytes != &expected {
        return Err("Explorer user tag bytes differ in raw-disk audit".into());
    }
    run.check("explorer-modern-native",json!({"sessions":sessions,"retirements":retirements,"boots":2,"scope":"real text preview, user tag create/assign, checked rename, tab creation/switch/close, two-boot identity and independent raw-disk audit","original_object":original_object,"disk_sha256":file_hash(&disk)?,"owner_volume_used":false,"pixel_exact_claim":false,"phase_closed":false}));
    Ok(())
}
