//! Bounded six-page reference workflow on a private disk; not a full desktop gate.
use super::*;
pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let disk = run.directory.join("settings-reference-user.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut record = session::launch_desktop_accelerated(run, image, &disk, false, "1920x1080")?;
    let result: Result<(), String> = (|| {
        let mut c = connection(&record)?;
        let mut p = (960, 540);
        thread::sleep(Duration::from_secs(2));
        resolution::activate(&run.root, &record, &mut c, 2)?;
        key(&mut c, &["meta_l", "up"])?;
        // Maximized: the client is the work area, at the screen origin.
        let client = super::administration::maximized_client((1920, 1080));
        for (query, id) in [
            ("drivers", 18),
            ("disk management", 19),
            ("services", 21),
            ("registry", 17),
            ("group policy", 24),
            ("task scheduler", 26),
        ] {
            key(&mut c, &["ctrl", "f"])?;
            type_line(&mut c, query)?;
            key(&mut c, &["esc"])?;
            wait_serial(
                &run.root,
                &record,
                &format!("SETTINGS PAGE id={id}"),
                Duration::from_secs(15),
            )?;
            let pixels =
                scene::capture_settled(run, &mut c, &format!("settings-reference-{id}"), &[])?;
            if pixels.width != 1920 || pixels.height != 1080 {
                return Err("reference scanout extent".into());
            }
        }
        // Exercise the actual appearance event, not a hosted palette mock:
        // Personalization > Colors, Tab to "Choose your mode", then Down/Up
        // changes the closed combo like WinUI (host test
        // `guest_scenario_keyboard_paths_stay_valid`).
        for (dark, expected_theme) in [(true, 1), (false, 0)] {
            key(&mut c, &["ctrl", "f"])?;
            type_line(&mut c, "colors")?;
            key(&mut c, &["esc"])?;
            key(&mut c, &["tab"])?;
            key(&mut c, &[if dark { "down" } else { "up" }])?;
            wait_serial(
                &run.root,
                &record,
                &format!("SETTINGS SAVED theme={expected_theme}"),
                Duration::from_secs(15),
            )?;
            key(&mut c, &["ctrl", "f"])?;
            type_line(&mut c, "drivers")?;
            key(&mut c, &["esc"])?;
            // Devices is an ordinary Windows page: the Mica background (from
            // the Settings theme) follows the appearance event.
            let theme = if dark { "dark" } else { "light" };
            let expected = super::mica::settings(0, dark);
            let (x, y) = door_workspace::settings_catalog::points::background(client.1);
            scene::capture_settled(
                run,
                &mut c,
                &format!("settings-devices-{theme}"),
                &[(x as usize, y as usize, expected)],
            )?;
        }
        // Actual registry create, text edit and durable save through the
        // Registry Editor's dialogs (points from the Settings layout).
        let r = super::administration::registry_points(client)?;
        key(&mut c, &["ctrl", "f"])?;
        type_line(&mut c, "registry")?;
        key(&mut c, &["esc"])?;
        click(&mut c, &mut p, r.go_to.0, r.go_to.1)?;
        type_line(&mut c, "HKCU\\Software\\Door\\ReferenceSettings")?; // Open fails.
        click(&mut c, &mut p, r.create.0, r.create.1)?; // Create the key.
        click(&mut c, &mut p, r.new_value.0, r.new_value.1)?;
        for ch in "ReferenceProbe".chars() {
            super::administration::type_character(&mut c, ch)?;
        }
        click(&mut c, &mut p, r.value_data.0, r.value_data.1)?;
        let before = serial(&run.root, &record)?.len();
        type_line(&mut c, "native reference inspector")?; // Return saves.
        let deadline = Instant::now() + Duration::from_secs(15);
        while !serial(&run.root, &record)?
            .get(before..)
            .unwrap_or("")
            .contains("ADMIN REGISTRY SAVED")
        {
            if Instant::now() >= deadline {
                return Err("reference inspector save acknowledgement missing".into());
            }
            thread::sleep(Duration::from_millis(100));
        }
        scene::capture_settled(run, &mut c, "settings-reference-registry-saved", &[])?;
        key(&mut c, &["ctrl", "f"])?;
        type_line(&mut c, "installed apps")?;
        key(&mut c, &["esc"])?;
        scene::capture_settled(run, &mut c, "settings-installed-apps", &[])?;
        // Terminal row (apps are listed by name; Terminal sorts last): its
        // More button opens the app menu; Open launches it. Points from the
        // Settings layout, asserted by the host test
        // `installed_apps_list_real_apps_and_open_launches_through_the_menu`.
        let (more, open) =
            door_workspace::settings_catalog::points::installed_app_open(5, client.0, client.1);
        let before = serial(&run.root, &record)?.len();
        click(&mut c, &mut p, more.0, more.1)?;
        click(&mut c, &mut p, open.0, open.1)?;
        resolution::wait_activation(&run.root, &record, 9, before)?;
        scene::capture_settled(run, &mut c, "settings-installed-terminal-launched", &[])?;
        key(&mut c, &["alt", "f4"])?;
        key(&mut c, &["ctrl", "w"])?;
        start_shut_down(&mut c)?;
        wait_serial(
            &run.root,
            &record,
            "DOOR_P08 CLEANUP PASS",
            Duration::from_secs(20),
        )?;
        Ok(())
    })();
    if result.is_err()
        && let Ok(mut c) = connection(&record)
    {
        let _ = scene::capture_settled(run, &mut c, "settings-reference-failure", &[]);
        let _ = c.execute("quit", json!({}));
    }
    result?;
    seal_serial(&run.root, &mut record)?;
    let retirement = shutdown::retirement_commands(&serial(&run.root, &record)?, &[3, 9])?;
    let raw = fs::read(&disk).map_err(|e| e.to_string())?;
    let journal = crate::persistence::journal::inspect(&raw)?;
    if journal.selected().committed {
        return Err("reference journal not clean".into());
    }
    let tree = crate::persistence::tree::inspect(&journal.replayed_home(&raw)?)?;
    let catalog = crate::persistence::catalog::decode(&tree)?.ok_or("reference catalog absent")?;
    let u64at = |r: &[u8], at| u64::from_le_bytes(r[at..at + 8].try_into().unwrap());
    let mut parent = 0;
    for part in ["", "Software", "Door", "ReferenceSettings"] {
        let found: Vec<_> = catalog
            .key_records()
            .filter(|r| {
                u64at(r, 0) != 0
                    && u64at(r, 8) == parent
                    && r[16..20] == 1001u32.to_le_bytes()
                    && r[20..23] == [2, 64, 0]
                    && crate::persistence::catalog::name(r, 32, r[23] as usize)
                        .is_ok_and(|name| name == part)
            })
            .collect();
        if found.len() != 1 {
            return Err("reference key identity missing or ambiguous".into());
        }
        parent = u64at(found[0], 0);
    }
    let found: Vec<_> = catalog
        .value_records()
        .filter(|r| {
            u64at(r, 0) == parent
                && crate::persistence::catalog::name(r, 16, r[8] as usize)
                    .is_ok_and(|name| name == "ReferenceProbe")
        })
        .collect();
    let expected: Vec<_> = "native reference inspector"
        .encode_utf16()
        .chain(Some(0))
        .flat_map(|v| v.to_le_bytes())
        .collect();
    if found.len() != 1
        || found[0][9] != 1
        || found[0][10..12] != (expected.len() as u16).to_le_bytes()
        || found[0][112..112 + expected.len()] != expected
    {
        return Err("reference value differs in independent raw-disk audit".into());
    }
    run.check("settings-reference-native",json!({"session":record,"pages":[18,19,21,17,24,26],"scope":"six unified Windows 11 Settings pages, appearance events and Mica background pixels, registry create/edit/save through dialogs, Installed apps Terminal launch; no full Windows functionality or frame-rate qualification","theme_switch_pixels":["dark","light"],"installed_terminal_launch":true,"retirement":retirement,"raw_disk_audit":{"journal_clean":true,"registry_identity_and_value_verified":true,"disk_sha256":file_hash(&disk)?},"owner_volume_used":false}));
    Ok(())
}
