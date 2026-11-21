//! Real input and independent persistent-byte checks for the unified
//! administration pages (P14: legacy tools as Windows 11 Settings pages).
use super::*;
use door_workspace::settings_catalog::{Command, DialogKind, List, points};
use door_workspace::settings_layout::Page;
/// Settings' client when maximized on the 1280x720 desktop: the work area
/// (the taskbar excluded), at the screen origin, so window points are
/// screen points.
pub(super) fn maximized_client(output: (u32, u32)) -> (u32, u32) {
    let work = door_window::taskbar::Layout::new(output.0, output.1).work_area();
    (work.width(), work.height())
}
/// Registry Editor and dialog points for a `client`, derived from the
/// Settings layout (`door_workspace::settings_catalog::points`) and pinned by
/// `administration_scenario_registry_points_follow_the_page_layout`
/// (desktop/workspace/tests/settings_reference_painter.rs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RegistryPoints {
    /// Card buttons: Go to a key, Add a value.
    pub go_to: (i32, i32),
    pub new_value: (i32, i32),
    /// The first value's Edit (a key with no subkeys and one value).
    pub first_value: (i32, i32),
    /// Go to a key dialog: its path box and Create.
    pub path: (i32, i32),
    pub create: (i32, i32),
    /// Value dialog: Type combo and Data box.
    pub value_type: (i32, i32),
    pub value_data: (i32, i32),
}
pub(super) fn registry_points((w, h): (u32, u32)) -> Result<RegistryPoints, String> {
    let mut lists = [0; List::COUNT];
    lists[List::RegistryValues as usize] = 1;
    Ok(RegistryPoints {
        go_to: points::command(Page::Registry, Command::RegistryGoTo, w, h)
            .ok_or("Go to a key button")?,
        new_value: points::command(Page::Registry, Command::RegistryNewValue, w, h)
            .ok_or("Add a value button")?,
        first_value: points::list_edit(Page::Registry, List::RegistryValues, 0, w, h, lists)
            .ok_or("first value Edit")?,
        path: points::dialog_field(DialogKind::RegistryGoTo, 0, w, h),
        create: points::dialog_button(DialogKind::RegistryGoTo, 1, w, h),
        value_type: points::dialog_type(DialogKind::RegistryValue, w, h),
        value_data: points::dialog_field(DialogKind::RegistryValue, 1, w, h),
    })
}
fn click_at(c: &mut qmp::Client, p: &mut (i32, i32), (x, y): (i32, i32)) -> Result<(), String> {
    click(c, p, x, y)
}
fn page(c: &mut qmp::Client, query: &str) -> Result<(), String> {
    key(c, &["esc"])?;
    key(c, &["ctrl", "f"])?;
    type_line(c, query)?;
    key(c, &["esc"])
}
/// The sixteen unified pages by the queries a user types (the first
/// suggestion; `administration_scenario_pages_and_history_follow_search`).
pub(super) const PAGES: [(&str, u32); 16] = [
    ("drivers", 18),
    ("disk management", 19),
    ("advanced network settings", 20),
    ("services", 21),
    ("environment", 22),
    ("system configuration", 23),
    ("registry", 17),
    ("group policy", 24),
    ("event viewer", 25),
    ("task scheduler", 26),
    ("your info", 27),
    ("file system", 28),
    ("troubleshoot", 29),
    ("power options", 30),
    ("recovery", 31),
    ("developer tools", 32),
];
/// Open the Go to a key dialog, type `path` and press Open (Return), then
/// Create when `create`.
fn go_to(
    c: &mut qmp::Client,
    p: &mut (i32, i32),
    r: &RegistryPoints,
    path: &str,
    create: bool,
) -> Result<(), String> {
    click_at(c, p, r.go_to)?;
    // The dialog opens with the current path selected: typing replaces it.
    type_line(c, path)?;
    if create {
        click_at(c, p, r.create)?;
    }
    Ok(())
}
pub(super) fn exercise(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    p: &mut (i32, i32),
) -> Result<(), String> {
    key(c, &["meta_l", "up"])?;
    let r = registry_points(maximized_client(window_points::WIDE))?;
    for (query, id) in PAGES {
        page(c, query)?;
        wait_serial(
            &run.root,
            record,
            &format!("SETTINGS PAGE id={id}"),
            Duration::from_secs(15),
        )?;
        scene::capture_settled(run, c, &format!("admin-page-{id}"), &[])?;
    }
    page(c, "registry")?;
    // Opening a missing key fails safely (the dialog stays with its reason);
    // Create makes it explicitly.
    go_to(c, p, &r, "HKCU\\Software\\Door\\AdminSettings", true)?;
    click_at(c, p, r.new_value)?;
    // Name has focus.
    for ch in "Probe".chars() {
        type_character(c, ch)?;
    }
    click_at(c, p, r.value_type)?;
    for _ in 0..3 {
        key(c, &["down"])?;
    }
    key(c, &["ret"])?; // REG_DWORD in the type list.
    click_at(c, p, r.value_data)?;
    let before = serial(&run.root, record)?.len();
    type_line(c, "0x1234ABCD")?; // Return saves.
    let deadline = Instant::now() + Duration::from_secs(15);
    while !serial(&run.root, record)?[before..].contains("ADMIN REGISTRY SAVED") {
        if Instant::now() >= deadline {
            return Err("native registry save acknowledgement missing".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
    scene::capture_settled(run, c, "admin-registry-saved", &[])?;
    // Invalid numeric data cannot mutate the durable checkpoint.
    click_at(c, p, r.first_value)?;
    click_at(c, p, r.value_data)?;
    type_line(c, "4294967296")?;
    scene::capture_settled(run, c, "admin-registry-invalid-draft", &[])?;
    let before = serial(&run.root, record)?.len();
    key(c, &["esc"])?; // Explicit cancel.
    wait_fresh(run, record, before, "ADMIN REGISTRY CANCELED")?;
    key(c, &["meta_l", "down"])?; // Restore the normal short window.
    // Focus returned to the value's Edit: Return opens it again.
    key(c, &["ret"])?;
    scene::capture_settled(run, c, "admin-registry-restored-dialog", &[])?;
    let restored = window_points::placement(window_points::App::Settings, window_points::WIDE)?;
    let cancel = points::dialog_button(
        DialogKind::RegistryEdit,
        2,
        restored.width(),
        restored.height(),
    );
    let before = serial(&run.root, record)?.len();
    let cancel = window_points::screen(window_points::App::Settings, cancel)?;
    click_at(c, p, cancel)?; // Cancel in the restored window's dialog.
    wait_fresh(run, record, before, "ADMIN REGISTRY CANCELED")
        .map_err(|_| String::from("pointer Cancel did not reach the restored dialog"))?;
    scene::capture_settled(run, c, "admin-pointer-cancel-restored", &[])?;
    key(c, &["meta_l", "up"])?;
    key(c, &["ctrl", "home"])?;
    scene::capture_settled(run, c, "admin-registry-top", &[])?;
    // A path typed but not opened never redirects a new value elsewhere:
    // GuardB exists, AdminSettings is open, GuardB is typed then canceled.
    go_to(c, p, &r, "HKCU\\Software\\Door\\GuardB", true)?;
    go_to(c, p, &r, "HKCU\\Software\\Door\\AdminSettings", false)?;
    click_at(c, p, r.go_to)?;
    for ch in "HKCU\\Software\\Door\\GuardB".chars() {
        type_character(c, ch)?;
    }
    key(c, &["esc"])?;
    click_at(c, p, r.new_value)?;
    for ch in "TargetGuard".chars() {
        type_character(c, ch)?;
    }
    click_at(c, p, r.go_to)?; // The page behind the dialog doesn't react.
    key(c, &["ctrl", "f"])?; // Nor does global Find.
    click_at(c, p, r.value_data)?;
    confirmed_value(run, record, c, "bound to A")?;
    for (name, kind, text) in [
        ("Text", 0, "Door\\u4E2D\\uD800"),
        ("Expanded", 1, "%PATH%"),
        ("Binary", 2, "00 A5 FF"),
        ("Multi", 4, "one\\0two"),
        ("Qword", 5, "0xFFFFFFFFFFFFFFFF"),
        ("None", 6, "DE AD"),
    ] {
        click_at(c, p, r.new_value)?;
        for ch in name.chars() {
            type_character(c, ch)?;
        }
        click_at(c, p, r.value_type)?;
        for _ in 0..kind {
            key(c, &["down"])?;
        }
        key(c, &["ret"])?;
        click_at(c, p, r.value_data)?;
        confirmed_value(run, record, c, text)?;
    }
    key(c, &["pgdn"])?;
    scene::capture_settled(run, c, "admin-registry-typed-values", &[])?;
    page(c, "services")?;
    let before = serial(&run.root, record)?.len();
    key(c, &["alt", "left"])?;
    wait_fresh_page(run, record, before, 17)?;
    let before = serial(&run.root, record)?.len();
    key(c, &["alt", "right"])?;
    wait_fresh_page(run, record, before, 21)?;
    page(c, "personalization")?; // Restore the existing suite's category geometry.
    run.check("desktop-advanced-settings",json!({"pages":16,"registry_value":"Probe","registry_kind":4,"registry_expected":0x1234ABCDu32,"native_types":7,"draft_target_guard":true,"pointer_leaves_search":true,"invalid_numeric_edit_canceled":true,"back_forward":true,"provider_scope":"unified Windows 11 Settings pages: kernel boot inventory, disk owner, Init discovery, identity, inherited and saved environment, object metadata, checks, registry dialogs; other provider operations unavailable"}));
    Ok(())
}
fn wait_fresh(run: &Run, record: &Value, before: usize, marker: &str) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !serial(&run.root, record)?[before..].contains(marker) {
        if Instant::now() >= deadline {
            return Err(format!("{marker} missing"));
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
fn confirmed_value(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    text: &str,
) -> Result<(), String> {
    let before = serial(&run.root, record)?.len();
    type_line(c, text)?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while !serial(&run.root, record)?[before..].contains("ADMIN REGISTRY SAVED") {
        if Instant::now() >= deadline {
            return Err("fresh native typed registry save missing".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
fn wait_fresh_page(run: &Run, record: &Value, before: usize, id: u32) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !serial(&run.root, record)?[before..].contains(&format!("SETTINGS PAGE id={id}")) {
        if Instant::now() >= deadline {
            return Err(format!("fresh Back/Forward page {id} missing"));
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
pub(super) fn type_character(c: &mut qmp::Client, ch: char) -> Result<(), String> {
    // Reuse the production character mapper without its trailing Return.
    c.execute(
        "send-key",
        json!({"keys":session::keys(ch)?,"hold-time":20}),
    )?;
    thread::sleep(Duration::from_millis(70));
    Ok(())
}
pub(super) fn audit(disk: &std::path::Path) -> Result<Value, String> {
    let bytes = fs::read(disk).map_err(|e| e.to_string())?;
    let journal = crate::persistence::journal::inspect(&bytes)?;
    if journal.selected().committed {
        return Err("administration journal not clean".into());
    }
    let tree = crate::persistence::tree::inspect(&journal.replayed_home(&bytes)?)?;
    let catalog =
        crate::persistence::catalog::decode(&tree)?.ok_or("administration catalog absent")?;
    let u64at = |r: &[u8], at| u64::from_le_bytes(r[at..at + 8].try_into().unwrap());
    let mut parent = 0;
    for part in ["", "Software", "Door", "AdminSettings"] {
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
            return Err("administration key identity missing or ambiguous".into());
        }
        parent = u64at(found[0], 0);
    }
    let found: Vec<_> = catalog
        .value_records()
        .filter(|r| {
            u64at(r, 0) == parent
                && crate::persistence::catalog::name(r, 16, r[8] as usize)
                    .is_ok_and(|name| name == "Probe")
        })
        .collect();
    if found.len() != 1
        || found[0][9] != 4
        || found[0][10..12] != 4u16.to_le_bytes()
        || found[0][112..116] != 0x1234ABCDu32.to_le_bytes()
    {
        return Err("registry typed value differs from the native confirmed save".into());
    }
    fn text(units: &[u16], multi: bool) -> Vec<u8> {
        let mut out: Vec<_> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
        out.extend_from_slice(if multi { &[0; 4] } else { &[0; 2] });
        out
    }
    for (name, kind, data) in [
        (
            "TargetGuard",
            1,
            text(&"bound to A".encode_utf16().collect::<Vec<_>>(), false),
        ),
        ("Text", 1, text(&[68, 111, 111, 114, 0x4e2d, 0xd800], false)),
        ("Expanded", 2, text(&[37, 80, 65, 84, 72, 37], false)),
        ("Binary", 3, vec![0, 0xa5, 0xff]),
        ("Multi", 7, text(&[111, 110, 101, 0, 116, 119, 111], true)),
        ("Qword", 11, u64::MAX.to_le_bytes().to_vec()),
        ("None", 0, vec![0xde, 0xad]),
    ] {
        let found: Vec<_> = catalog
            .value_records()
            .filter(|r| {
                u64at(r, 0) == parent
                    && crate::persistence::catalog::name(r, 16, r[8] as usize)
                        .is_ok_and(|n| n == name)
            })
            .collect();
        if found.len() != 1
            || found[0][9] != kind
            || u16::from_le_bytes(found[0][10..12].try_into().unwrap()) as usize != data.len()
            || found[0][112..112 + data.len()] != data
        {
            return Err(format!("native typed registry value {name} differs"));
        }
    }
    Ok(
        json!({"path":"HKCU\\Software\\Door\\AdminSettings","uid":1001,"view":64,"name":"Probe","kind":4,"value":0x1234ABCDu32,"native_types":7,"draft_target_guard":true,"journal_clean":true,"independent_catalog_decode":true}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    /// The derived points: on screen in the maximized 1280x720 client, and
    /// pressing what the scenario means (the host painter test asserts the
    /// same points hit the same targets in the real Settings).
    #[test]
    fn registry_points_are_derived_from_the_settings_layout() {
        let client = maximized_client(window_points::WIDE);
        assert_eq!(client, (1280, 672));
        let r = registry_points(client).unwrap();
        for (name, (x, y)) in [
            ("go_to", r.go_to),
            ("new_value", r.new_value),
            ("first_value", r.first_value),
            ("path", r.path),
            ("create", r.create),
            ("value_type", r.value_type),
            ("value_data", r.value_data),
        ] {
            assert!(
                (0..1280).contains(&x) && (120..672).contains(&y),
                "{name} ({x}, {y}) is off the client"
            );
        }
        // Card buttons sit in the content column's right edge; dialog
        // controls inside the centred dialog.
        assert!(r.go_to.0 > 900 && r.new_value.0 == r.go_to.0);
        assert!(r.new_value.1 > r.go_to.1 && r.first_value.1 > r.new_value.1);
        assert!(r.create.0 > r.path.0 - 200 && r.value_data.1 > r.value_type.1);
        // Pinned values: a layout change shows up here and in the host test.
        assert_eq!(r, registry_points((1280, 672)).unwrap());
        // The restored window's dialog Cancel lies inside that window.
        let restored =
            window_points::placement(window_points::App::Settings, window_points::WIDE).unwrap();
        let cancel = points::dialog_button(
            DialogKind::RegistryEdit,
            2,
            restored.width(),
            restored.height(),
        );
        assert!(cancel.0 > 0 && (cancel.0 as u32) < restored.width());
        assert!(cancel.1 > 0 && (cancel.1 as u32) < restored.height());
        // Each scenario query lands on its page first.
        for (query, id) in PAGES {
            let hit = door_settings_map::search(query).next().unwrap();
            assert_eq!(
                hit.target(),
                (Page::from_index(id as usize).unwrap(), None),
                "{query}"
            );
        }
    }
}
