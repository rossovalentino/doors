//! P14 WS-D: legacy Windows entry points and the unified Settings pages on a
//! private disk. Legacy commands typed into Run open their Settings page and
//! section; Settings search finds tools by their legacy names; keyboard and
//! pointer navigate the new pages; a variable saved through a dialog is
//! persisted, read back after reopening, and audited in raw disk bytes.
//! Points come from the Settings layout (`door_workspace::settings_catalog::
//! points`), never from remembered coordinates.
use super::*;
use door_workspace::settings_catalog::{self as catalog, Action, List, points};
use door_workspace::settings_layout::{Layout, Page};

/// Run commands and the page (and section) each must open.
pub(super) const RUN_COMMANDS: [(&str, u32, Option<&str>); 10] = [
    ("control", 0, None),
    ("sysdm.cpl", 12, None),
    ("devmgmt.msc", 18, None),
    ("ncpa.cpl", 20, Some("network-adapters")),
    ("control sysdm.cpl,,3", 44, Some("administrative-tools")),
    ("regedit", 17, None),
    ("ms-settings:startupapps", Page::Startup as u32, None),
    (
        "control /name Microsoft.CredentialManager",
        6,
        Some("saved-credentials"),
    ),
    ("powercfg.cpl", 30, None),
    ("gpedit.msc", 24, None),
];
/// Run commands whose page asks for administrator consent (User Account
/// Control) before it opens, and the program the prompt names (Windows
/// prompts for regedit and for the Group Policy console, mmc.exe).
const ELEVATED_COMMANDS: [(&str, &str); 2] = [
    ("regedit", "Registry Editor"),
    ("gpedit.msc", "Macrohard Management Console"),
];
/// Legacy names typed into Settings search and the page found first.
pub(super) const SEARCHES: [(&str, u32); 4] = [
    ("Device Manager", 18),
    ("msconfig", 23),
    ("Event Viewer", 25),
    ("Control Panel", 0),
];
/// The variable the scenario saves (HKEY_CURRENT_USER\Environment).
const NAME: &str = "DOOR_P14";
const VALUE: &str = "persisted";
/// Pointer points in the maximized Settings client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct NavigationPoints {
    /// Rail: System.
    pub system: (i32, i32),
    /// System > Storage card.
    pub storage: (i32, i32),
    /// Storage > Advanced storage settings (expander header).
    pub advanced_storage: (i32, i32),
    /// Its Disks & volumes row once open.
    pub disks: (i32, i32),
}
pub(super) fn navigation_points((w, h): (u32, u32)) -> Result<NavigationPoints, String> {
    let rail = door_workspace::settings_navigation::Rail::new(
        Layout {
            width: w,
            height: h,
        },
        0,
    );
    let index = door_workspace::settings_navigation::items()
        .position(|p| p == Page::System)
        .ok_or("System rail row")?;
    let lists = [0; List::COUNT];
    let mut open = points::context(w, h, lists);
    open.open = 1 << 1;
    let disks = catalog::layout(Page::Storage, &open)
        .items
        .iter()
        .find(|i| i.action == Action::Navigate(Page::DiskManagement))
        .map(|i| (i.rect.x + i.rect.w as i32 / 2, i.rect.center_y()))
        .ok_or("Disks & volumes row")?;
    Ok(NavigationPoints {
        system: (Layout::NAV_X + 100, rail.row(index) + Layout::nav_row() / 2),
        storage: points::item(Page::System, w, h, lists, |i| {
            i.action == Action::Navigate(Page::Storage)
        })
        .ok_or("Storage card")?,
        advanced_storage: points::item(Page::Storage, w, h, lists, |i| {
            i.action == Action::Expand(1)
        })
        .ok_or("Advanced storage settings")?,
        disks,
    })
}
/// Wait for a serial line written after `before` that ends with `marker`
/// (whole-line match, so `id=1` never matches `id=12`).
fn fresh(run: &Run, record: &Value, before: usize, marker: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let text = serial(&run.root, record)?;
        if text
            .get(before..)
            .unwrap_or("")
            .lines()
            .any(|line| line.trim_end().ends_with(marker))
        {
            return Ok(());
        }
        if text.contains("APP FAILED") || text.contains("KERNEL_PANIC") {
            return Err("desktop guest failed; inspect serial".into());
        }
        if Instant::now() >= deadline {
            return Err(format!("{marker} missing"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}
/// Win+R, the command, Return; Settings must report the page (and section).
fn run_command(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    command: &str,
    page: u32,
    section: Option<&str>,
) -> Result<(), String> {
    let before = serial(&run.root, record)?.len();
    key(c, &["meta_l", "r"])?;
    thread::sleep(Duration::from_millis(300));
    type_line(c, command)?;
    // Administrative tools ask for consent first (P16), as on Windows; the
    // scenario consents and expects the page.
    if let Some((_, program)) = ELEVATED_COMMANDS.iter().find(|(c, _)| *c == command) {
        // The prompt line goes on with the publisher and button rectangles.
        let prompt = format!("DOOR_P16 UAC PROMPT program={program} publisher=");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !serial(&run.root, record)?
            .get(before..)
            .unwrap_or("")
            .contains(&prompt)
        {
            if Instant::now() >= deadline {
                return Err(format!("{program} consent prompt missing"));
            }
            thread::sleep(Duration::from_millis(100));
        }
        thread::sleep(Duration::from_millis(800));
        key(c, &["alt", "y"])?;
    }
    fresh(run, record, before, &format!("SETTINGS PAGE id={page}"))?;
    if let Some(anchor) = section {
        fresh(
            run,
            record,
            before,
            &format!("SETTINGS SECTION page={page} anchor={anchor}"),
        )?;
    }
    Ok(())
}
fn search(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    query: &str,
    page: u32,
) -> Result<(), String> {
    let before = serial(&run.root, record)?.len();
    key(c, &["esc"])?;
    key(c, &["ctrl", "f"])?;
    type_line(c, query)?;
    key(c, &["esc"])?;
    fresh(run, record, before, &format!("SETTINGS PAGE id={page}"))
}
pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let disk = run.directory.join("p14-settings-user.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut record = session::launch_desktop_accelerated(run, image, &disk, false, "1280x720")?;
    let client = super::administration::maximized_client(window_points::WIDE);
    let nav = navigation_points(client)?;
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        let mut p = (640, 360);
        thread::sleep(Duration::from_secs(2));
        // 1. Legacy commands typed into Run open their Settings pages.
        for (command, page, section) in RUN_COMMANDS {
            run_command(run, &record, &mut c, command, page, section)?;
            scene::capture_settled(run, &mut c, &format!("p14-run-{page}"), &[])?;
        }
        key(&mut c, &["meta_l", "up"])?;
        // 2. Settings search finds tools by their legacy names.
        for (query, page) in SEARCHES {
            search(run, &record, &mut c, query, page)?;
        }
        // 3. Pointer: rail System, the Storage card, the collapsed Advanced
        //    storage settings, then Disks & volumes.
        let before = serial(&run.root, &record)?.len();
        click(&mut c, &mut p, nav.system.0, nav.system.1)?;
        fresh(run, &record, before, "SETTINGS PAGE id=1")?;
        click(&mut c, &mut p, nav.storage.0, nav.storage.1)?;
        fresh(run, &record, before, "SETTINGS PAGE id=36")?;
        click(
            &mut c,
            &mut p,
            nav.advanced_storage.0,
            nav.advanced_storage.1,
        )?;
        scene::capture_settled(run, &mut c, "p14-storage-advanced-open", &[])?;
        click(&mut c, &mut p, nav.disks.0, nav.disks.1)?;
        fresh(run, &record, before, "SETTINGS PAGE id=19")?;
        fresh(run, &record, before, "SETTINGS REFRESH page=19")?;
        scene::capture_settled(run, &mut c, "p14-disks-volumes", &[])?;
        // Breadcrumb back with the keyboard (Alt+Left twice: Storage, System).
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["alt", "left"])?;
        fresh(run, &record, before, "SETTINGS PAGE id=36")?;
        // 4. Keyboard: Environment variables, Tab to Add a variable, Return
        //    opens the dialog, the name, Tab, the value, Return saves.
        search(run, &record, &mut c, "environment", 22)?;
        key(&mut c, &["tab"])?;
        key(&mut c, &["ret"])?;
        for ch in NAME.chars() {
            super::administration::type_character(&mut c, ch)?;
        }
        key(&mut c, &["tab"])?;
        let before = serial(&run.root, &record)?.len();
        type_line(&mut c, VALUE)?;
        fresh(
            run,
            &record,
            before,
            &format!("SETTINGS EVENT Environment variable saved: {NAME}"),
        )?;
        scene::capture_settled(run, &mut c, "p14-environment-saved", &[])?;
        // 5. Close Settings; reopen it with the legacy Environment Variables
        //    command: the variable is read back from the registry.
        key(&mut c, &["alt", "f4"])?;
        thread::sleep(Duration::from_secs(1));
        let before = serial(&run.root, &record)?.len();
        run_command(
            run,
            &record,
            &mut c,
            "rundll32.exe sysdm.cpl,EditEnvironmentVariables",
            22,
            None,
        )?;
        fresh(
            run,
            &record,
            before,
            &format!("SETTINGS VARIABLES names={NAME}"),
        )?;
        scene::capture_settled(run, &mut c, "p14-environment-reopened", &[])?;
        key(&mut c, &["alt", "f4"])?;
        start_shut_down(&mut c)?;
        wait_serial(
            &run.root,
            &record,
            "DOOR_P08 CLEANUP PASS",
            Duration::from_secs(20),
        )?;
        Ok(json!({
            "run_commands": RUN_COMMANDS.len(),
            "legacy_searches": SEARCHES.len(),
            "pointer_pages": [1, 36, 19],
            "keyboard_dialog_save": true,
            "reopened_variable": NAME,
        }))
    })();
    if result.is_err()
        && let Ok(mut c) = connection(&record)
    {
        let _ = scene::capture_settled(run, &mut c, "p14-settings-failure", &[]);
        let _ = c.execute("quit", json!({}));
    }
    let observed = result?;
    seal_serial(&run.root, &mut record)?;
    let audit = audit(&disk)?;
    run.check(
        "p14-settings-unified",
        json!({"session":record,"observed":observed,"raw_disk_audit":audit,"scope":"legacy Run commands to Settings pages and sections, legacy-name search, keyboard and pointer navigation of unified pages, dialog save persisted across reopening; owner volume not used"}),
    );
    Ok(())
}
/// Independent raw-disk check: HKEY_CURRENT_USER\Environment holds the
/// variable as REG_SZ.
fn audit(disk: &std::path::Path) -> Result<Value, String> {
    let bytes = fs::read(disk).map_err(|e| e.to_string())?;
    let journal = crate::persistence::journal::inspect(&bytes)?;
    if journal.selected().committed {
        return Err("p14 settings journal not clean".into());
    }
    let tree = crate::persistence::tree::inspect(&journal.replayed_home(&bytes)?)?;
    let catalog = crate::persistence::catalog::decode(&tree)?.ok_or("p14 catalog absent")?;
    let u64at = |r: &[u8], at| u64::from_le_bytes(r[at..at + 8].try_into().unwrap());
    let mut parent = 0;
    for part in ["", "Environment"] {
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
            return Err("Environment key identity missing or ambiguous".into());
        }
        parent = u64at(found[0], 0);
    }
    let expected: Vec<u8> = VALUE
        .encode_utf16()
        .chain(Some(0))
        .flat_map(|u| u.to_le_bytes())
        .collect();
    let found: Vec<_> = catalog
        .value_records()
        .filter(|r| {
            u64at(r, 0) == parent
                && crate::persistence::catalog::name(r, 16, r[8] as usize)
                    .is_ok_and(|name| name == NAME)
        })
        .collect();
    if found.len() != 1
        || found[0][9] != 1
        || found[0][10..12] != (expected.len() as u16).to_le_bytes()
        || found[0][112..112 + expected.len()] != expected
    {
        return Err("saved variable differs in the independent raw-disk audit".into());
    }
    Ok(
        json!({"key":"HKEY_CURRENT_USER\\Environment","name":NAME,"kind":1,"value":VALUE,"journal_clean":true}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn run_commands_and_searches_resolve_as_the_scenario_expects() {
        for (command, page, section) in RUN_COMMANDS {
            let (p, s) = door_settings_map::resolve(command).unwrap();
            assert_eq!((p as u32, s), (page, section), "{command}");
        }
        assert_eq!(
            door_settings_map::resolve("rundll32.exe sysdm.cpl,EditEnvironmentVariables"),
            Some((Page::Environment, None))
        );
        for (query, page) in SEARCHES {
            let hit = door_settings_map::search(query).next().unwrap();
            assert_eq!(hit.page() as u32, page, "{query}");
        }
    }
    #[test]
    fn navigation_points_are_derived_from_the_settings_layout() {
        let client = super::super::administration::maximized_client(window_points::WIDE);
        let n = navigation_points(client).unwrap();
        for (name, (x, y)) in [
            ("system", n.system),
            ("storage", n.storage),
            ("advanced_storage", n.advanced_storage),
            ("disks", n.disks),
        ] {
            assert!(
                (0..client.0 as i32).contains(&x) && (120..client.1 as i32 - 20).contains(&y),
                "{name} ({x}, {y}) is off the visible client"
            );
        }
        assert!(n.system.0 < Layout::MAIN && n.storage.0 > Layout::MAIN);
        assert!(n.disks.1 > n.advanced_storage.1);
    }
}
