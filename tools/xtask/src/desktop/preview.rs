//! Actual standalone viewer capture on private Storage, with real app file writes.
use super::*;
fn wait(run: &Run, record: &Value, offset: usize, marker: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !serial(&run.root, record)?
        .get(offset..)
        .unwrap_or("")
        .contains(marker)
    {
        if Instant::now() >= deadline {
            return Err(format!("Preview marker missing: {marker}"));
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let disk = run.directory.join("preview-private.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut sessions = Vec::new();
    let mut retirements = Vec::new();
    for boot in 0..2 {
        let mut record =
            session::launch_desktop_accelerated(run, image, &disk, false, "1920x1080")?;
        let mut visits = Vec::new();
        let result = (|| -> Result<(), String> {
            let mut c = connection(&record)?;
            let mut p = (960, 540);
            if boot == 0 {
                resolution::activate(&run.root, &record, &mut c, 1)?;
                type_line(&mut c, "P3 2 2 255 255 0 0 0 255 0 0 0 255 255 255 255")?;
                let before = serial(&run.root, &record)?.len();
                key(&mut c, &["ctrl", "s"])?;
                wait(run, &record, before, "DOOR_P11 NOTES SAVED")?;
                key(&mut c, &["ctrl", "w"])?;
                resolution::activate(&run.root, &record, &mut c, 0)?;
                key(&mut c, &["ctrl", "l"])?;
                type_line(&mut c, r"C:\Users\1001\notes")?;
                let before = serial(&run.root, &record)?.len();
                key(&mut c, &["home"])?;
                key(&mut c, &["f2"])?;
                key(&mut c, &["ctrl", "a"])?;
                type_line(&mut c, "preview-colors.ppm")?;
                wait(run, &record, before, "FILES RENAME COMMITTED")?;
                key(&mut c, &["ctrl", "w"])?;
            }
            resolution::activate(&run.root, &record, &mut c, 4)?;
            if boot == 0 {
                scene::capture_settled(run, &mut c, "preview-reference-window", &[])?;
            }
            key(&mut c, &["meta_l", "up"])?;
            key(&mut c, &["ctrl", "o"])?;
            type_line(&mut c, r"C:\Users\1001\notes\preview-colors.ppm")?;
            wait(run, &record, 0, "image=1")?;
            scene::capture_settled(
                run,
                &mut c,
                &format!("preview-image-boot{boot}"),
                &[
                    (650, 250, [255, 0, 0]),
                    (1100, 250, [0, 255, 0]),
                    (650, 700, [0, 0, 255]),
                    (1100, 700, [255, 255, 255]),
                ],
            )?;
            if boot == 0 {
                click(&mut c, &mut p, 546, 90)?;
                scene::capture_settled(
                    run,
                    &mut c,
                    "preview-rotated-view",
                    &[
                        (650, 250, [0, 0, 255]),
                        (1100, 250, [255, 0, 0]),
                        (650, 700, [255, 255, 255]),
                        (1100, 700, [0, 255, 0]),
                    ],
                )?;
                let exported_at = serial(&run.root, &record)?.len();
                key(&mut c, &["ctrl", "shift", "s"])?;
                key(&mut c, &["ctrl", "a"])?;
                type_line(&mut c, r"C:\Users\1001\notes\preview-colors-edited.qoi")?;
                wait(run, &record, exported_at, "DOOR_PREVIEW EXPORT bytes=")?;
                let opened_at = serial(&run.root, &record)?.len();
                key(&mut c, &["ctrl", "o"])?;
                type_line(&mut c, r"C:\Users\1001\notes\preview-colors-edited.qoi")?;
                wait(run, &record, opened_at, "image=1")?;
                scene::capture_settled(
                    run,
                    &mut c,
                    "preview-exported-image",
                    &[
                        (650, 250, [0, 0, 255]),
                        (1100, 250, [255, 0, 0]),
                        (650, 700, [255, 255, 255]),
                        (1100, 700, [0, 255, 0]),
                    ],
                )?;
                // Ctrl+W closes the exported QOI's tab; the PNG export below
                // converts the remaining rotated preview-colors.ppm tab, so
                // its suggested name is preview-colors.png (an export keeps
                // its source tab, unlike a Save As).
                key(&mut c, &["ctrl", "w"])?;
                let png_at = serial(&run.root, &record)?.len();
                key(&mut c, &["ctrl", "shift", "s"])?;
                key(&mut c, &["ret"])?;
                wait(run, &record, png_at, "DOOR_PREVIEW EXPORT bytes=")?;
                let collision_at = serial(&run.root, &record)?.len();
                key(&mut c, &["ctrl", "shift", "s"])?;
                key(&mut c, &["ret"])?;
                wait(
                    run,
                    &record,
                    collision_at,
                    "DOOR_PREVIEW EXPORT_REFUSED code=Service(Exists)",
                )?;
                key(&mut c, &["esc"])?;
                let tabs_at = serial(&run.root, &record)?.len();
                key(&mut c, &["ctrl", "o"])?;
                type_line(&mut c, r"C:\Users\1001\notes\preview-colors.ppm")?;
                wait(run, &record, tabs_at, "tabs=2")?;
                key(&mut c, &["ctrl", "tab"])?;
                key(&mut c, &["ctrl", "w"])?;
                scene::capture_settled(run, &mut c, "preview-tabs", &[])?;
            }
            if boot == 1 {
                let before = serial(&run.root, &record)?.len();
                key(&mut c, &["ctrl", "o"])?;
                type_line(&mut c, r"C:\Users\1001\notes\preview-colors.png")?;
                wait(run, &record, before, "image=1")?;
                scene::capture_settled(
                    run,
                    &mut c,
                    "preview-png-after-reboot",
                    &[
                        (650, 250, [0, 0, 255]),
                        (1100, 250, [255, 0, 0]),
                        (650, 700, [255, 255, 255]),
                        (1100, 700, [0, 255, 0]),
                    ],
                )?;
            }
            if boot == 0 {
                for page in 0..17 {
                    let before = serial(&run.root, &record)?.len();
                    // Navigation pane rows: 40 px pitch from y=64 with the
                    // Library and Tools headers each taking one row; centers
                    // are row top + 18. Settings is pinned at height - 60.
                    let y = match page {
                        0..=2 => 82 + page * 40,
                        3..=9 => 122 + page * 40,
                        10..=15 => 162 + page * 40,
                        _ => 990,
                    };
                    click(&mut c, &mut p, 100, y)?;
                    wait(
                        run,
                        &record,
                        before,
                        &format!("DOOR_PREVIEW SECTION id={page}"),
                    )?;
                    scene::capture_settled(run, &mut c, &format!("preview-section-{page}"), &[])?;
                    visits.push((page as u32, before));
                }
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
            let _ = scene::capture_settled(run, &mut c, "preview-failure", &[]);
            let _ = c.execute("quit", json!({}));
        }
        result?;
        seal_serial(&run.root, &mut record)?;
        let sealed = serial(&run.root, &record)?;
        let retired =
            shutdown::retirement_commands(&sealed, if boot == 0 { &[2, 1, 10] } else { &[10] })?;
        if boot == 0 {
            let routes = reference_routes::validate(
                &sealed,
                &retired,
                10,
                13,
                "DOOR_PREVIEW SECTION",
                &visits,
            )?;
            run.check("preview-all-section-navigation", routes);
        }
        retirements.push(retired);
        sessions.push(record);
    }
    let raw = fs::read(&disk).map_err(|e| e.to_string())?;
    let journal = crate::persistence::journal::inspect(&raw)?;
    if journal.selected().committed {
        return Err("Preview journal not clean".into());
    }
    let tree = crate::persistence::tree::inspect(&journal.replayed_home(&raw)?)?;
    let catalog = crate::persistence::catalog::decode(&tree)?.ok_or("Preview catalog absent")?;
    let paths = catalog.paths()?;
    let source = paths
        .get("/users/1001/notes/preview-colors.ppm")
        .ok_or("Preview source not durable")?;
    if tree
        .files
        .get(&format!("obj{:016x}", source.id))
        .is_none_or(|f| f.bytes != b"P3 2 2 255 255 0 0 0 255 0 0 0 255 255 255 255\n")
    {
        return Err("Preview changed its source bytes".into());
    }
    let exported = paths
        .get("/users/1001/notes/preview-colors-edited.qoi")
        .ok_or("Preview export not durable")?;
    let data = &tree
        .files
        .get(&format!("obj{:016x}", exported.id))
        .ok_or("Preview export payload absent")?
        .bytes;
    let (header, rgba) = qoi::decode_to_vec(data).map_err(|e| e.to_string())?;
    if header.width != 2
        || header.height != 2
        || rgba
            != [
                0, 0, 255, 255, 255, 0, 0, 255, 255, 255, 255, 255, 0, 255, 0, 255,
            ]
    {
        return Err("Preview converted pixels differ".into());
    }
    let png = paths
        .get("/users/1001/notes/preview-colors.png")
        .ok_or("PNG export not durable")?;
    if !tree
        .files
        .get(&format!("obj{:016x}", png.id))
        .is_some_and(|f| f.bytes.starts_with(b"\x89PNG\r\n\x1a\n"))
    {
        return Err("PNG export payload absent".into());
    }
    if paths.keys().any(|p| p.contains(".preview-")) {
        return Err("Completed export left staging binding".into());
    }
    run.check("preview-native-viewer",json!({"sessions":sessions,"retirements":retirements,"boots":2,"source_bytes_unchanged":true,"exported_qoi_pixels_verified":true,"journal_clean":true,"disk_sha256":file_hash(&disk)?,"owner_volume_used":false,"scope":"actual independent viewer, authenticated PPM read, QOI export, view rotation, tabs and source preservation; no full format/edit/Windows fidelity qualification"}));
    density(run, image, &disk)?;
    Ok(())
}

fn density(run: &mut Run, image: &str, source: &std::path::Path) -> Result<(), String> {
    let disk = run.directory.join("preview-density-private.raw");
    fs::copy(source, &disk).map_err(|e| e.to_string())?;
    let mut record = session::launch_desktop_accelerated(run, image, &disk, false, "3840x2160")?;
    let result = (|| -> Result<(), String> {
        let mut c = connection(&record)?;
        let mut p = (1920, 1080);
        resolution::activate(&run.root, &record, &mut c, 4)?;
        key(&mut c, &["meta_l", "up"])?;
        for count in 1..=4 {
            let before = serial(&run.root, &record)?.len();
            key(&mut c, &["ctrl", "o"])?;
            type_line(&mut c, r"C:\Users\1001\notes\preview-colors.ppm")?;
            wait(run, &record, before, &format!("tabs={count} image=1"))?;
        }
        scene::capture_settled(run, &mut c, "preview-3x-four-tabs", &[])?;
        let before = serial(&run.root, &record)?.len();
        // Four Explorer caption tabs (workspace caption_tabs/document_tabs::Strip
        // at 1280 logical): left 8, stride min((1280-8-184)/4, 294) = 272, so the
        // fourth tab is x 824..1092; its close box is x 1060..1084, y 12..32,
        // centred at (1072, 22) logical, here in physical 3x pixels.
        click(&mut c, &mut p, 1072 * 3, 22 * 3)?;
        wait(run, &record, before, "DOOR_PREVIEW TABS count=3")?;
        scene::capture_settled(run, &mut c, "preview-3x-last-caption-close", &[])?;
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
        let _ = scene::capture_settled(run, &mut c, "preview-density-failure", &[]);
        let _ = c.execute("quit", json!({}));
    }
    result?;
    seal_serial(&run.root, &mut record)?;
    let retired = shutdown::retirement_commands(&serial(&run.root, &record)?, &[10])?;
    run.check("preview-native-density",json!({"session":record,"retirement":retired,"scope":"Actual3x raster and fourth-tab trusted caption close; tiny image fixture does not qualify large-image native memory pressure","owner_volume_used":false}));
    Ok(())
}
