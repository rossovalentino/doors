//! Ordinary widescreen boot, simultaneous app lifecycle and durable Settings.
//! This suite has no adversarial boot flag and never uses the owner's disk.
use super::*;

fn open(c: &mut qmp::Client, index: usize) -> Result<(), String> {
    key(c, &["meta_l"])?;
    // Start focuses its search pill; the first Right enters the Pinned row
    // and each further Right walks it (Down moves a grid row).
    for _ in 0..=index {
        key(c, &["right"])?;
    }
    key(c, &["ret"])?;
    thread::sleep(Duration::from_secs(2));
    Ok(())
}
fn healthy(root: &std::path::Path, record: &Value) -> Result<(), String> {
    let text = serial(root, record)?;
    if [
        "APP FAILED",
        "SERVER FAILED",
        "INIT FAIL",
        "KERNEL_PANIC",
        "LAUNCH_REJECTED",
    ]
    .iter()
    .any(|v| text.contains(v))
    {
        return Err("ordinary desktop lifecycle failed; inspect serial".into());
    }
    Ok(())
}
fn diagnostic_credit(run: &mut Run, record: &Value, boot: &str) -> Result<(), String> {
    let text = serial(&run.root, record)?;
    let records: Vec<_> = text
        .lines()
        .filter_map(|line| {
            let (pid, payload) = line.strip_prefix("DOOR_USER pid=")?.split_once(' ')?;
            Some((pid, payload))
        })
        .collect();
    let pid = records
        .iter()
        .find(|(_, payload)| payload.starts_with("DOOR_P12 DENSITY "))
        .map(|(pid, _)| *pid)
        .ok_or("window diagnostic identity missing")?;
    let payload_bytes: usize = records
        .iter()
        .filter(|(who, _)| *who == pid)
        .map(|(_, payload)| payload.len() + 1)
        .sum();
    let frame_samples = records
        .iter()
        .filter(|(who, payload)| *who == pid && payload.starts_with("DOOR_P12 FRAME "))
        .count();
    let first_frame_included = records
        .iter()
        .any(|(who, payload)| *who == pid && payload.starts_with("DOOR_P12 FRAME serial=1 "));
    if payload_bytes + 512 >= door_abi::PROCESS_DIAGNOSTIC_BUDGET || frame_samples > 16 {
        return Err("window telemetry exhausted reserved diagnostic credit".into());
    }
    run.check(&format!("desktop-window-diagnostic-credit-{boot}"), json!({
        "pid":pid,"payload_bytes":payload_bytes,"budget":door_abi::PROCESS_DIAGNOSTIC_BUDGET,
        "reserved_bytes":512,"frame_samples":frame_samples,
        "first_frame_included":first_frame_included,
        "scope":"Actual native serial payloads after the long workflow; first frame plus slow frames within unchanged authority budget"
    }));
    Ok(())
}
pub(super) fn appearance_on_disk(disk: &std::path::Path) -> Result<u32, String> {
    let bytes = fs::read(disk).map_err(|e| e.to_string())?;
    let journal = crate::persistence::journal::inspect(&bytes)?;
    if journal.selected().committed {
        return Err("Settings journal not clean".into());
    }
    let tree = crate::persistence::tree::inspect(&journal.replayed_home(&bytes)?)?;
    let catalog = crate::persistence::catalog::decode(&tree)?.ok_or("Settings catalog absent")?;
    let records = &catalog.bytes[14976..15488];
    let record = records
        .chunks_exact(64)
        .find(|r| u32::from_le_bytes(r[..4].try_into().unwrap()) == 1001)
        .ok_or("Settings user record absent")?;
    if !matches!(u32::from_le_bytes(record[4..8].try_into().unwrap()), 2..=4)
        || u32::from_le_bytes(record[24..28].try_into().unwrap()) >= 4
        || record[28..].iter().any(|b| *b != 0)
    {
        return Err("Settings record schema".into());
    }
    Ok(
        (u32::from_le_bytes(record[24..28].try_into().unwrap()) << 1)
            | u32::from(u32::from_le_bytes(record[8..12].try_into().unwrap()) != 0),
    )
}
/// Settings search: Ctrl+F, query and Return (first result), Esc, then the
/// page marker, which Settings writes once the 300 ms page entrance has
/// settled (input during the entrance hits the moving page, as on Windows).
fn settings_search(
    root: &std::path::Path,
    record: &Value,
    c: &mut qmp::Client,
    query: &str,
) -> Result<(), String> {
    let before = serial(root, record)?.len();
    key(c, &["ctrl", "f"])?;
    type_line(c, query)?;
    key(c, &["esc"])?;
    wait_fresh(root, record, before, "SETTINGS PAGE id=")
}
// Settings points for the 1280x720 wide profile, derived from the Settings
// layout (`door_workspace::settings_catalog::points`) and asserted by the host
// test `lifecycle_scenario_points_follow_the_settings_layout`
// (desktop/workspace/tests/settings_reference_painter.rs). A maximized Settings
// window is 1280x672 at the origin, so window points are screen points; the
// snaps are 640x672 at x = 0 (left) and x = 640 (right).
use door_workspace::settings_catalog::{self, Fact};
use door_workspace::settings_layout::Page as SettingsPage;
const MAXIMIZED: (u32, u32) = (1280, 672);
const SNAP: (u32, u32) = (640, 672);
/// The light accent of the rail selection pill.
const ACCENT: [u8; 3] = [0, 103, 203];
fn found(point: Option<(i32, i32)>, what: &str) -> Result<(i32, i32), String> {
    point.ok_or_else(|| format!("Settings layout has no {what}"))
}
/// Category rail row centre (the rail always starts below the account block).
fn rail(row: usize) -> (i32, i32) {
    settings_catalog::points::rail_row(row)
}
/// A pixel probe inside rail row `row`'s selection pill.
fn pill(row: usize) -> (usize, usize, [u8; 3]) {
    let (x, y) = settings_catalog::points::rail_pill(row);
    (x as usize, y as usize, ACCENT)
}
/// The maximized card of `page` that opens `target`.
fn card(page: SettingsPage, target: SettingsPage) -> Result<(i32, i32), String> {
    found(
        settings_catalog::points::link(page, target, MAXIMIZED.0, MAXIMIZED.1),
        "card link",
    )
}
/// Theme tile `index` with Settings snapped right (two columns).
fn theme_tile(index: usize) -> Result<(i32, i32), String> {
    let (x, y) = found(
        settings_catalog::points::theme_tile(index, SNAP.0, SNAP.1),
        "theme tile",
    )?;
    Ok((SNAP.0 as i32 + x, y))
}
/// Maximized Installed apps: app `index`'s "..." button and its Open entry.
fn installed_app(index: usize) -> ((i32, i32), (i32, i32)) {
    settings_catalog::points::installed_app_open(index, MAXIMIZED.0, MAXIMIZED.1)
}
/// The maximized window background in the heading row (right of the title).
fn heading_background(rgb: [u8; 3]) -> (usize, usize, [u8; 3]) {
    let (x, y) = settings_catalog::points::heading_background(MAXIMIZED.0);
    (x as usize, y as usize, rgb)
}
fn settings_pages(
    run: &mut Run,
    c: &mut qmp::Client,
    p: &mut (i32, i32),
    record: &Value,
) -> Result<(), String> {
    key(c, &["meta_l", "up"])?;
    for index in 0..12 {
        let (x, y) = rail(index);
        click(c, p, x, y)?;
        scene::capture_settled(run, c, &format!("settings-page-{index}"), &[pill(index)])?;
    }
    settings_search(&run.root, record, c, "about")?;
    wait_serial(
        &run.root,
        record,
        "DOOR_P12 SETTINGS PAGE id=12",
        Duration::from_secs(10),
    )?;
    scene::capture_settled(run, c, "settings-about", &[pill(1)])?;
    let (x, y) = rail(5); // Apps
    let before = serial(&run.root, record)?.len();
    click(c, p, x, y)?;
    // The card is hit after the page entrance settles.
    wait_fresh(&run.root, record, before, "SETTINGS PAGE id=")?;
    let (x, y) = card(SettingsPage::Apps, SettingsPage::InstalledApps)?;
    click(c, p, x, y)?;
    wait_serial(
        &run.root,
        record,
        "DOOR_P12 SETTINGS PAGE id=14",
        Duration::from_secs(10),
    )?;
    scene::capture_settled(run, c, "settings-installed-apps", &[pill(5)])?;
    let (x, y) = rail(1); // System
    let before = serial(&run.root, record)?.len();
    click(c, p, x, y)?;
    // The card is hit after the page entrance settles.
    wait_fresh(&run.root, record, before, "SETTINGS PAGE id=")?;
    let (x, y) = card(SettingsPage::System, SettingsPage::Display)?;
    click(c, p, x, y)?;
    wait_serial(
        &run.root,
        record,
        "DOOR_P12 SETTINGS PAGE id=13",
        Duration::from_secs(10),
    )?;
    // End, then the left snap: the resolution and orientation values stay
    // right aligned in their cards while the Scale combo wraps below its
    // label (as Windows does); the snap keeps the scroll offset, clamped.
    let wide_scroll =
        settings_catalog::points::end_scroll(SettingsPage::Display, MAXIMIZED.0, MAXIMIZED.1);
    let narrow_scroll = wide_scroll.min(settings_catalog::points::end_scroll(
        SettingsPage::Display,
        SNAP.0,
        SNAP.1,
    ));
    let value = |fact, (w, h): (u32, u32), scroll| {
        found(
            settings_catalog::points::value(SettingsPage::Display, fact, w, h, scroll),
            "Display value",
        )
    };
    // The page can still be taking focus when End arrives (gate attempt
    // 16 captured the unscrolled top): wait for the view to move, and
    // press End once more if it has not.
    let top = scene::capture_settled(run, c, "settings-display-top", &[pill(1)])?;
    let (tx, ty) = value(Fact::Orientation, MAXIMIZED, wide_scroll)?;
    let moved = |frame: &scene::Pixels| -> Result<bool, String> {
        for dy in (-40i32..40).step_by(8) {
            for dx in (-120i32..0).step_by(8) {
                let (x, y) = ((tx + dx) as usize, (ty + dy) as usize);
                if frame.pixel(x, y)? != top.pixel(x, y)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    };
    let mut scrolled = false;
    for _ in 0..2 {
        key(c, &["end"])?;
        let frame = scene::capture_settled(run, c, "settings-display-scroll", &[pill(1)])?;
        if moved(&frame)? {
            scrolled = true;
            break;
        }
    }
    if !scrolled {
        return Err("Settings > Display did not scroll to its end".into());
    }
    // The recorded capture, once (the desktop validation counts it).
    let wide = scene::capture_settled(run, c, "settings-display", &[pill(1)])?;
    key(c, &["meta_l", "left"])?;
    let narrow = scene::capture_settled(run, c, "settings-display-narrow", &[pill(1)])?;
    // The orientation value: Display's Related settings push the resolution
    // card above the end-scrolled view at the maximized size.
    for fact in [Fact::Orientation] {
        let (wx, wy) = value(fact, MAXIMIZED, wide_scroll)?;
        let (nx, ny) = value(fact, SNAP, narrow_scroll)?;
        // Settings draws over Mica, whose colour follows the wallpaper under
        // each screen point, so the two captures are compared as ink: each
        // pixel's luminance against the card just right of its value.
        let luma = |p: [u8; 3]| i32::from(p[0]) * 54 + i32::from(p[1]) * 183 + i32::from(p[2]) * 19;
        let (ref_a, ref_b) = (
            luma(wide.pixel((wx + 6) as usize, wy as usize)?),
            luma(narrow.pixel((nx + 6) as usize, ny as usize)?),
        );
        let mut ink = 0;
        for dy in -7..5 {
            for dx in 1..=28 {
                let a = luma(wide.pixel((wx - dx) as usize, (wy + dy) as usize)?) - ref_a;
                let b = luma(narrow.pixel((nx - dx) as usize, (ny + dy) as usize)?) - ref_b;
                if (a - b).abs() > 12 * 256 {
                    return Err("narrow Display lost or changed an actual value".into());
                }
                ink += usize::from(a.abs() > 40 * 256);
            }
        }
        if ink < 5 {
            return Err("Display value glyphs were not rendered".into());
        }
    }
    run.check(
        "desktop-settings-narrow-values",
        json!({"resolution_orientation":true,"same_native_value_glyphs":true}),
    );
    key(c, &["meta_l", "up"])?;
    scene::capture_settled(run, c, "settings-display-restored", &[pill(1)])?;
    key(c, &["ctrl", "f"])?;
    type_line(c, "installed")?;
    scene::capture_settled(run, c, "settings-detail-search", &[pill(5)])?;
    key(c, &["esc"])?;
    // Search lives in client title-bar content; click it through the real
    // non-client hit policy, type, navigate, then restore the full category list.
    let (sx, sy) = found(
        settings_catalog::points::search(MAXIMIZED.0, MAXIMIZED.1),
        "title-bar search",
    )?;
    click(c, p, sx, sy)?;
    type_line(c, "network")?;
    scene::capture_settled(run, c, "settings-search", &[pill(3)])?;
    key(c, &["esc"])?;
    let (x, y) = rail(0);
    click(c, p, x, y)?;
    key(c, &["pgdn"])?;
    scene::capture_settled(
        run,
        c,
        "settings-scroll",
        // The title search box: its control veil over Mica (default Bloom).
        &[
            (
                sx as usize,
                sy as usize,
                super::mica::settings_control(0, false),
            ),
            pill(0),
        ],
    )?;
    key(c, &["pgup"])?;
    let launches = |text: &str| {
        text.lines()
            .filter(|l| l.contains("APP_LAUNCHED app=1"))
            .count()
    };
    // Installed apps > File Explorer > More > Open.
    let (more, open_files) = installed_app(0);
    settings_search(&run.root, record, c, "installed apps")?;
    let before = launches(&serial(&run.root, record)?);
    click(c, p, more.0, more.1)?;
    click(c, p, open_files.0, open_files.1)?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while launches(&serial(&run.root, record)?) == before && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
    }
    if launches(&serial(&run.root, record)?) != before + 1 {
        return Err("Settings did not launch real File Explorer".into());
    }
    open(c, 2)?;
    click(c, p, more.0, more.1)?;
    click(c, p, open_files.0, open_files.1)?;
    thread::sleep(Duration::from_secs(2));
    if launches(&serial(&run.root, record)?) != before + 1 {
        return Err("Settings launch duplicated File Explorer".into());
    }
    scene::capture_settled(
        run,
        c,
        "settings-launched-files",
        &[(770, 560, [255, 255, 255])],
    )?;
    open(c, 2)?;
    run.check("desktop-settings-layouts",json!({"pages":15,"actual_native_titlebar_search":true,"scroll_preserves_header_and_navigation":true,"files_launch_and_singleton_activation":true,"scope":"native subset; unsupported device/service controls remain unavailable; Windows pixel fidelity not qualified"}));
    Ok(())
}
fn themes(
    run: &mut Run,
    c: &mut qmp::Client,
    p: &mut (i32, i32),
    record: &Value,
) -> Result<(), String> {
    // Snap Settings right so the wallpaper stays visible at (0, 0).
    key(c, &["meta_l", "right"])?;
    settings_search(&run.root, record, c, "personalization")?;
    // `settings_layout::THEMES`: Doors (light), Doors (dark), Glow,
    // Captured Motion, Sunrise, Flow (their first pictures at (0, 0)).
    for (index, (value, color)) in [
        (0, [132, 173, 201]),
        (3, [4, 15, 29]),
        (5, [7, 1, 27]),
        (15, [7, 7, 7]),
        (6, [132, 179, 195]),
        (28, [143, 159, 159]),
    ]
    .into_iter()
    .enumerate()
    {
        let (x, y) = theme_tile(index)?;
        click(c, p, x, y)?;
        if let Err(e) = wait_serial(
            &run.root,
            record,
            &format!("SETTINGS SAVED appearance={value}"),
            Duration::from_secs(15),
        ) {
            // Keep the frame that shows where the theme change stopped.
            let _ = scene::capture(run, c, &format!("settings-theme-{index}-timeout"), &[]);
            let _ = super::move_to(c, p, x + 40, y + 40);
            let _ = scene::capture(run, c, &format!("settings-theme-{index}-pointer"), &[]);
            return Err(e);
        }
        scene::capture(run, c, &format!("settings-theme-{index}"), &[(0, 0, color)])?;
    }
    let (x, y) = theme_tile(0)?;
    click(c, p, x, y)?;
    scene::capture(run, c, "settings-theme-reset", &[(0, 0, [132, 173, 201])])?;
    run.check("desktop-settings-themes",json!({"backgrounds":door_initfs::desktop_assets::COUNT,"themes":6,"native_offline_assets":true,"all_selected_in_guest":true}));
    Ok(())
}
/// `needle` in the serial log after `offset` (a marker an earlier step may
/// already have printed).
fn wait_fresh(
    root: &std::path::Path,
    record: &Value,
    offset: usize,
    needle: &str,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if serial(root, record)?
            .get(offset..)
            .unwrap_or("")
            .contains(needle)
        {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(format!("desktop did not report {needle}"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
fn power(c: &mut qmp::Client, p: &mut (i32, i32)) -> Result<(), String> {
    let _ = p;
    start_shut_down(c)
}
fn close_after_click(
    root: &std::path::Path,
    c: &mut qmp::Client,
    p: &mut (i32, i32),
    record: &Value,
    point: (i32, i32),
    expected: &str,
) -> Result<(), String> {
    move_to(c, p, point.0, point.1)?;
    let before = serial(root, record)?;
    let settings_pid = before
        .lines()
        .rev()
        .find_map(|line| {
            line.split_once("APP_LAUNCHED app=3 pid=")
                .and_then(|(_, pid)| pid.trim().parse::<u64>().ok())
        })
        .ok_or("Settings process identity missing")?;
    // Separate PS/2 sample boundaries; one input-sync batch coalesces down/up.
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"btn","data":{"down":true,"button":"left"}}]}),
    )?;
    thread::sleep(Duration::from_millis(100));
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"btn","data":{"down":false,"button":"left"}}]}),
    )?;
    key(c, &["alt", "f4"])?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let now = serial(root, record)?;
        let tail = &now[before.len()..];
        if tail.contains(&format!("CLIENT_EXIT pid={settings_pid} code=0"))
            && tail.contains(expected)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("queued click/close did not complete {expected}"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}
pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let disk = run.directory.join("desktop-wide.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut first = session::launch_desktop_wide(run, image, &disk, false)?;
    let result: Result<(), String> = (|| {
        let mut c = connection(&first)?;
        let mut pointer = (640, 360);
        thread::sleep(Duration::from_secs(2));
        if serial(&run.root, &first)?.contains("APP_LAUNCHED") {
            return Err("ordinary boot autostarted an app".into());
        }
        let screen = scene::capture(run, &mut c, "wide-empty", &[])?;
        if (screen.width, screen.height) != (1280, 720) {
            return Err("widescreen GOP dimensions".into());
        }
        show_desktop_icons(&run.root, &first, &mut c, 112)?;
        selection(run, &mut c, &mut pointer, &first, "wide")?;
        // Exercise the exact owner-reported desktop gesture, not just Start's
        // keyboard path. Repeated activation must keep one authenticated app.
        for index in 0..3 {
            // Icon centers come from the shell's desktop grid. The second
            // release returns at once: sample cursor movement while Init
            // starts the application.
            double_click(&mut c, &mut pointer, desktop_icon(index, 1), false)?;
            cursor_step(
                run,
                &mut c,
                &mut pointer,
                -12,
                &format!("launch-pointer-{index}"),
            )?;
            // TCG launch time varies with load: wait for the launch, then
            // give a duplicate time to appear.
            wait_serial(
                &run.root,
                &first,
                &format!("APP_LAUNCHED app={}", index + 1),
                Duration::from_secs(20),
            )
            .map_err(|_| "desktop double click failed or duplicated client")?;
            thread::sleep(Duration::from_secs(1));
            healthy(&run.root, &first)?;
            let text = serial(&run.root, &first)?;
            if text
                .lines()
                .filter(|l| l.contains(&format!("APP_LAUNCHED app={}", index + 1)))
                .count()
                != 1
            {
                return Err("desktop double click failed or duplicated client".into());
            }
        }
        // All three real clients coexist; each crosses the 2 MiB section boundary
        // on maximize, then returns to its independently painted normal surface.
        for index in 0..3 {
            open(&mut c, index)?;
        }
        open(&mut c, 0)?;
        // Explorer has stayed at its initial placement and opened to Home.
        // Maximized, its client is 1280x672 at the origin, so these points
        // are window points from the real layout; they are asserted by
        // desktop/workspace/src/files/scenario_points.rs. Explorer is restored
        // after `explorer::exercise` for the window-management cycles.
        key(&mut c, &["meta_l", "up"])?;
        thread::sleep(Duration::from_millis(500));
        click(&mut c, &mut pointer, 240, 160)?; // Home "Quick access" header
        scene::capture_settled(
            run,
            &mut c,
            "explorer-home-collapsed",
            &[
                // The first Quick access tile's icon is gone; Home stays
                // selected in the navigation pane.
                (205, 220, [255, 255, 255]),
                (130, 159, [213, 233, 250]),
            ],
        )?;
        click(&mut c, &mut pointer, 240, 160)?; // expand it again (remembered)
        click(&mut c, &mut pointer, 100, 447)?; // navigation pane: This PC
        scene::capture_settled(
            run,
            &mut c,
            "explorer-collapsed-navigation",
            &[
                (130, 447, [213, 233, 250]),
                (130, 159, [233, 240, 246]),
                (212, 194, [225, 227, 230]),
            ],
        )?;
        click(&mut c, &mut pointer, 262, 152)?; // collapse "Devices and drives"
        scene::capture_settled(
            run,
            &mut c,
            "explorer-collapsed-folders",
            &[(212, 194, [255, 255, 255])],
        )?;
        click(&mut c, &mut pointer, 262, 152)?;
        key(&mut c, &["ctrl", "l"])?;
        type_line(&mut c, "/users/1001/Documents")?;
        key(&mut c, &["ctrl", "l"])?;
        type_line(&mut c, "/users/1001/Downloads")?;
        // Right-clicking Back lists the recent locations, newest first:
        // Downloads, Documents, This PC, Home.
        move_to(&mut c, &mut pointer, 32, 64)?;
        for down in [true, false] {
            c.execute(
                "input-send-event",
                json!({"events":[{"type":"btn","data":{"down":down,"button":"right"}}]}),
            )?;
            thread::sleep(Duration::from_millis(100));
        }
        scene::capture_settled(
            run,
            &mut c,
            "explorer-history-menu",
            &[(272, 203, [250, 250, 250])],
        )?;
        click(&mut c, &mut pointer, 148, 203)?; // its last row: Home
        wait_serial(
            &run.root,
            &first,
            "FILES HISTORY index=0 count=4 path=/users/1001\n",
            Duration::from_secs(10),
        )?;
        key(&mut c, &["alt", "right"])?;
        wait_serial(
            &run.root,
            &first,
            "FILES HISTORY index=1 count=4 path=/users/1001\n",
            Duration::from_secs(10),
        )?;
        key(&mut c, &["alt", "right"])?;
        wait_serial(
            &run.root,
            &first,
            "FILES HISTORY index=2 count=4 path=/users/1001/Documents",
            Duration::from_secs(10),
        )?;
        key(&mut c, &["alt", "right"])?;
        wait_serial(
            &run.root,
            &first,
            "FILES HISTORY index=3 count=4 path=/users/1001/Downloads",
            Duration::from_secs(10),
        )?;
        run.check("desktop-explorer-history-position",json!({"select_home_preserves_forward":true,"this_pc_documents_then_downloads_reachable":true,"count":4,"read_before_publish":true}));
        super::explorer::exercise(run, &mut c, &mut pointer, &first)?;
        key(&mut c, &["meta_l", "down"])?;
        // Points come from the real authority and placement
        // (`window_points`, host-tested). Cycle 0 maximizes with Win+Up, which
        // also normalizes any earlier state; Restore returns the initial
        // placement, whose Maximize button cycle 1 clicks. Captures poll
        // through the open/close transitions.
        for (index, (app, color)) in [
            (window_points::App::Files, [255, 255, 255]),
            (window_points::App::Notes, [255, 255, 255]),
            // Settings' light background (maximized Home page): Mica over
            // the default Bloom.
            (
                window_points::App::Settings,
                super::mica::settings(0, false),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let (maximize, restore) = window_points::wide_max(app)?;
            open(&mut c, index)?;
            for cycle in 0..2 {
                if cycle == 0 {
                    key(&mut c, &["meta_l", "up"])?;
                } else {
                    click(&mut c, &mut pointer, maximize.0, maximize.1)?;
                }
                scene::capture_settled(
                    run,
                    &mut c,
                    &format!("wide-max-{index}-{cycle}"),
                    &[(1270, 630, color)],
                )?;
                click(&mut c, &mut pointer, restore.0, restore.1)?;
                thread::sleep(Duration::from_millis(500));
                healthy(&run.root, &first)?;
            }
        }
        // Explorer's caption drag point (right of its tabs, below the
        // snap-layouts reveal band) and its left border column.
        let (grab, left) = window_points::explorer_drag()?;
        let border = window_points::border();
        open(&mut c, 0)?;
        move_to(&mut c, &mut pointer, grab.0, grab.1)?;
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":true,"button":"left"}}]}),
        )?;
        for index in 0..3 {
            cursor_step(run, &mut c, &mut pointer, 12, &format!("drag-out-{index}"))?;
        }
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":false,"button":"left"}}]}),
        )?;
        scene::capture_settled(
            run,
            &mut c,
            "wide-drag-out",
            &[((left + 36) as usize, 500, border)],
        )?;
        move_to(&mut c, &mut pointer, grab.0, grab.1)?;
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":true,"button":"left"}}]}),
        )?;
        for index in 0..3 {
            cursor_step(
                run,
                &mut c,
                &mut pointer,
                -12,
                &format!("drag-back-{index}"),
            )?;
        }
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":false,"button":"left"}}]}),
        )?;
        scene::capture_settled(
            run,
            &mut c,
            "wide-drag-restored",
            &[(left as usize, 500, border)],
        )?;
        run.check("desktop-three-client-drag",json!({"coexisting_clients":3,"physical_held_drags":2,"cursor_samples":6,"restored_original_geometry":true}));
        for index in (0..3).rev() {
            open(&mut c, index)?;
            key(&mut c, &["alt", "f4"])?;
            thread::sleep(Duration::from_secs(1));
        }
        if c.execute("query-status", json!({}))?["running"] != true {
            return Err("closing all clients ended desktop".into());
        }
        let text = serial(&run.root, &first)?;
        if text
            .lines()
            .filter(|l| l.contains("CLIENT_EXIT") && l.ends_with("code=0"))
            .count()
            != 3
        {
            return Err("clients did not all close cleanly".into());
        }
        scene::capture(run, &mut c, "wide-all-closed", &[])?;
        shell::exercise(run, &mut c, &mut pointer, &first)?;
        open(&mut c, 2)?;
        settings_pages(run, &mut c, &mut pointer, &first)?;
        themes(run, &mut c, &mut pointer, &first)?;
        // Accepted down/up must still mutate the model when the immediately
        // following close makes an intervening paint/capture RPC return Closing.
        // Settings is still snapped right on Personalization: Sunrise light
        // (tile 4, appearance 6) leaves the following dark switch a change.
        let (x, y) = theme_tile(4)?;
        close_after_click(
            &run.root,
            &mut c,
            &mut pointer,
            &first,
            (x, y),
            "SETTINGS SAVED appearance=6",
        )?;
        open(&mut c, 2)?;
        run.check(
            "desktop-queued-click-close",
            json!({"persisted_appearance":6,"ordered_close":true}),
        );
        // Installed apps > Notes > More > Open.
        key(&mut c, &["meta_l", "up"])?;
        settings_search(&run.root, &first, &mut c, "installed apps")?;
        let (more, open_notes) = installed_app(1);
        click(&mut c, &mut pointer, more.0, more.1)?;
        // Evidence of the open More menu (its solid fill, light theme)
        // before the queued Open click.
        let (mx, my) =
            settings_catalog::points::installed_app_menu_padding(1, MAXIMIZED.0, MAXIMIZED.1);
        scene::capture(
            run,
            &mut c,
            "installed-apps-more",
            &[(mx as usize, my as usize, [252, 252, 252])],
        )?;
        close_after_click(
            &run.root,
            &mut c,
            &mut pointer,
            &first,
            open_notes,
            "APP_LAUNCHED app=2",
        )?;
        open(&mut c, 2)?;
        key(&mut c, &["meta_l", "up"])?;
        run.check(
            "desktop-queued-launch-close",
            json!({"launched_app":2,"ordered_close":true}),
        );
        settings_mode(&mut c, true)?;
        wait_serial(
            &run.root,
            &first,
            "DOOR_P12 SETTINGS SAVED theme=1",
            Duration::from_secs(10),
        )?;
        // Maximized dark Settings background (heading row, right of the title).
        scene::capture_settled(
            run,
            &mut c,
            "wide-dark-settings",
            // Sunrise's first picture (the theme applied above), dark Mica.
            &[heading_background(super::mica::settings(3, true))],
        )?;
        key(&mut c, &["meta_l", "right"])?;
        settings_search(&run.root, &first, &mut c, "personalization")?;
        // Sunrise (light), then Dark again: dark Sunrise persists.
        let before = serial(&run.root, &first)?.len();
        let (x, y) = theme_tile(4)?;
        click(&mut c, &mut pointer, x, y)?;
        wait_fresh(&run.root, &first, before, "SETTINGS SAVED appearance=6")?;
        let before = serial(&run.root, &first)?.len();
        settings_mode(&mut c, true)?;
        wait_fresh(&run.root, &first, before, "SETTINGS SAVED theme=1")?;
        scene::capture(
            run,
            &mut c,
            "settings-sunrise-saved",
            &[(0, 0, [132, 179, 195])],
        )?;
        // Many moves within one taskbar button must stay on the cursor path.
        let begin = Instant::now();
        for _ in 0..10 {
            move_to(&mut c, &mut pointer, 610, 700)?;
            move_to(&mut c, &mut pointer, 620, 700)?;
        }
        cursor_step(run, &mut c, &mut pointer, -12, "taskbar-pointer")?;
        let motion_ms = begin.elapsed().as_millis();
        if !serial(&run.root, &first)?.contains("DOOR_P12 BAR FAST transferred_rows=48") {
            return Err("taskbar motion did not use partial transfer".into());
        }
        healthy(&run.root, &first)?;
        open(&mut c, 1)?;
        type_line(&mut c, "Saved during shutdown")?;
        let before_power = serial(&run.root, &first)?;
        let notes_pid = before_power
            .lines()
            .rev()
            .find_map(|line| {
                line.split_once("APP_LAUNCHED app=2 pid=")
                    .and_then(|(_, value)| value.trim().parse::<u64>().ok())
            })
            .ok_or("shutdown Notes launch identity missing")?;
        // Windows 11 shutdown with unsaved work. First request: Notes asks,
        // the user chooses Cancel, so the shutdown is cancelled and the
        // session stays usable with Notes still open.
        let root = run.root.clone();
        let fresh = |offset: usize, needle: &str| -> Result<(), String> {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !serial(&root, &first)?
                .get(offset..)
                .unwrap_or("")
                .contains(needle)
            {
                if Instant::now() >= deadline {
                    return Err(format!("wide shutdown did not report fresh {needle}"));
                }
                thread::sleep(Duration::from_millis(50));
            }
            Ok(())
        };
        let close_offset = before_power.len();
        power(&mut c, &mut pointer)?;
        fresh(
            close_offset,
            &format!("DOOR_USER pid={notes_pid} DOOR_P12 NOTES PROMPT intent=Close"),
        )?;
        let (sx, sy) = window_points::notes_save_focus()?;
        scene::capture_settled(
            run,
            &mut c,
            "wide-shutdown-cancel-dialog",
            &[(sx as usize, sy as usize, [0, 103, 196])],
        )?;
        let cancel_offset = serial(&run.root, &first)?.len();
        key(&mut c, &["esc"])?;
        fresh(
            cancel_offset,
            &format!("DOOR_USER pid={notes_pid} DOOR_P12 NOTES PROMPT_CANCEL intent=Close"),
        )?;
        fresh(
            cancel_offset,
            &format!("DOOR_P12 SHUTDOWN CANCELLED pid={notes_pid}"),
        )?;
        // The session is alive with Notes still open (its window, without
        // the prompt): nothing retired the document and nothing shut down.
        // The second request below is the fresh interaction.
        scene::capture_settled(run, &mut c, "wide-shutdown-cancelled", &[])?;
        let after_cancel = serial(&run.root, &first)?;
        if after_cancel.contains(&format!("CLIENT_EXIT pid={notes_pid} "))
            || after_cancel.contains("DOOR_P08 CLEANUP")
        {
            return Err("a cancelled shutdown closed the document or the session".into());
        }
        // Second request: Save in the prompt closes Notes and the shutdown
        // continues by itself, without another request.
        let save_offset = after_cancel.len();
        power(&mut c, &mut pointer)?;
        fresh(
            save_offset,
            &format!("DOOR_USER pid={notes_pid} DOOR_P12 NOTES PROMPT intent=Close"),
        )?;
        scene::capture_settled(
            run,
            &mut c,
            "wide-shutdown-save-dialog",
            &[(sx as usize, sy as usize, [0, 103, 196])],
        )?;
        let saved_offset = serial(&run.root, &first)?.len();
        key(&mut c, &["ret"])?;
        fresh(
            saved_offset,
            "DOOR_P12 NOTES PROMPT_DONE intent=Close choice=0",
        )?;
        fresh(
            saved_offset,
            &format!("DOOR_P11 CLIENT_EXIT pid={notes_pid} code=0"),
        )
        .map_err(|_| "shutdown's explicit document Save did not close its native client")?;
        wait_serial(
            &run.root,
            &first,
            "DOOR_P08 CLEANUP PASS",
            Duration::from_secs(30),
        )
        .map_err(|_| "the shutdown did not continue after the document was saved")?;
        seal_serial(&run.root, &mut first)?;
        let canonical = shutdown::canonical(&serial(&run.root, &first)?)?;
        if canonical["form"] != "cancel-then-save" {
            return Err("wide shutdown must cancel once, then save and finish".into());
        }
        let snapshot = run.directory.join("wide-shutdown-save.raw");
        fs::copy(&disk, &snapshot).map_err(|e| e.to_string())?;
        let shutdown_saved = audit(&snapshot, b"Saved during shutdown\n")?;
        run.check("desktop-shutdown-save-dialog",json!({"notes_pid":notes_pid,"decision":"Save","power_requests":2,"first_request_canceled":true,"cancel_kept_session":true,"continued_after_save":true,"fresh_close_prompt":true,"fresh_client_exit":true,"saved":shutdown_saved,"snapshot":relative(&run.root,&snapshot)?,"sha256":file_hash(&snapshot)?}));
        diagnostic_credit(run, &first, "first")?;
        audit(&disk, b"Saved during shutdown\n")?;
        if appearance_on_disk(&disk)? != 7 {
            return Err("dark Settings did not persist in catalog".into());
        }
        run.check("desktop-wide-motion",json!({"moves":20,"observed_ms":motion_ms,"scope":"QMP movement completion; no frame-rate claim"}));
        Ok(())
    })();
    if result.is_err()
        && let Ok(mut c) = connection(&first)
    {
        let _ = c.execute("quit", json!({}));
    }
    result?;
    let mut second = session::launch_desktop_wide(run, image, &disk, false)?;
    let result: Result<(), String> = (|| {
        let mut c = connection(&second)?;
        let mut pointer = (640, 360);
        // The desktop menu's "Show desktop icons" persisted for the user;
        // restore the default (hidden) before the Settings controls checks.
        wait_serial(
            &run.root,
            &second,
            "DOOR_P12 CONTROLS bits=112",
            Duration::from_secs(15),
        )?;
        show_desktop_icons(&run.root, &second, &mut c, 48)?;
        open(&mut c, 2)?;
        // Default placement leaves the wallpaper visible at (0, 0).
        scene::capture_settled(
            run,
            &mut c,
            "settings-sunrise-reopened",
            &[(0, 0, [132, 179, 195])],
        )?;
        key(&mut c, &["meta_l", "up"])?;
        scene::capture_settled(
            run,
            &mut c,
            "wide-dark-reopened",
            &[heading_background(super::mica::settings(3, true))],
        )?;
        settings_mode(&mut c, false)?;
        wait_serial(
            &run.root,
            &second,
            "DOOR_P12 SETTINGS SAVED theme=0",
            Duration::from_secs(10),
        )?;
        scene::capture_settled(
            run,
            &mut c,
            "wide-light-settings",
            &[heading_background(super::mica::settings(3, false))],
        )?;
        key(&mut c, &["meta_l", "right"])?;
        settings_search(&run.root, &second, &mut c, "personalization")?;
        let (x, y) = theme_tile(0)?; // Doors (light).
        click(&mut c, &mut pointer, x, y)?;
        wait_serial(
            &run.root,
            &second,
            "SETTINGS SAVED appearance=0",
            Duration::from_secs(15),
        )?;
        scene::capture(
            run,
            &mut c,
            "settings-default-restored",
            &[(0, 0, [132, 173, 201])],
        )?;
        super::controls::exercise(run, &second, &mut c, &mut pointer)?;
        super::administration::exercise(run, &second, &mut c, &mut pointer)?;
        healthy(&run.root, &second)?;
        power(&mut c, &mut pointer)?;
        wait_serial(
            &run.root,
            &second,
            "DOOR_P08 CLEANUP PASS",
            Duration::from_secs(20),
        )?;
        seal_serial(&run.root, &mut second)?;
        diagnostic_credit(run, &second, "reboot")?;
        if appearance_on_disk(&disk)? != 0 {
            return Err("light Settings did not persist".into());
        }
        run.check(
            "desktop-advanced-settings-durable",
            super::administration::audit(&disk)?,
        );
        Ok(())
    })();
    if result.is_err()
        && let Ok(mut c) = connection(&second)
    {
        let _ = c.execute("quit", json!({}));
    }
    result?;
    run.check("desktop-wide-lifecycle",json!({"outcome":"passed","first":first,"second":second,"image":image,"disk":relative(&run.root,&disk)?,"disk_sha256":file_hash(&disk)?,"ordinary_empty_boot":true,"launch_relaunch":true,"simultaneous_apps":3,"maximize_restore_cycles":6,"settings_separate_boot_persistence":true,"desktop_icons_preference_persisted":true,"document_close_protocol":2}));
    Ok(())
}
