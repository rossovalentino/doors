//! Trusted shell interactions through physical QMP input on the private wide disk.
use super::*;

/// Settings' light background: Mica over the default Bloom (or its solid
/// fallback while another window is active).
const SETTINGS_BACKGROUND: [u8; 3] = super::mica::settings(0, false);

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let disk = run.directory.join("desktop-shell.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut record = session::launch_desktop_wide(run, image, &disk, false)?;
    let result = (|| {
        let mut c = connection(&record)?;
        let mut pointer = (640, 360);
        exercise(run, &mut c, &mut pointer, &record)?;
        start_shut_down(&mut c)?;
        wait_serial(
            &run.root,
            &record,
            "DOOR_P08 CLEANUP PASS",
            Duration::from_secs(20),
        )?;
        seal_serial(&run.root, &mut record)?;
        run.check(
            "desktop-shell-suite",
            json!({"record":record,"image":image,"private_disk":relative(&run.root,&disk)?}),
        );
        Ok(())
    })();
    if result.is_err()
        && let Ok(mut c) = connection(&record)
    {
        let _ = c.execute("quit", json!({}));
    }
    result
}

fn button(c: &mut qmp::Client, down: bool) -> Result<(), String> {
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"btn","data":{"down":down,"button":"left"}}]}),
    )?;
    thread::sleep(Duration::from_millis(100));
    Ok(())
}
fn launches(text: &str) -> usize {
    text.lines()
        .filter(|line| line.contains("APP_LAUNCHED"))
        .count()
}
fn focus_roundtrips(
    run: &mut Run,
    c: &mut qmp::Client,
    p: &mut (i32, i32),
    record: &Value,
) -> Result<(), String> {
    resolution::activate(&run.root, record, c, 1)?;
    // Notes opens at its initial placement (`window_points`). Probe the actual
    // editor above its status bar, then require a complete stable frame after
    // fresh activation.
    let (bx, by) = super::window_points::screen(
        super::window_points::App::Notes,
        super::window_points::NOTES_BODY_POINT,
    )?;
    scene::capture_settled(
        run,
        c,
        "focus-notes-ready",
        &[(bx as usize, by as usize, [255; 3])],
    )?;
    key(c, &["meta_l", "left"])?;
    scene::capture(run, c, "focus-notes-snapped", &[(600, 600, [255; 3])])?;
    let disk = paths::existing(
        &run.root,
        record["disk"].as_str().ok_or("focus fixture disk")?,
    )?;
    for gesture in 0..13 {
        match gesture {
            0 | 1 => {
                key(c, &["meta_l", if gesture == 0 { "a" } else { "n" }])?;
                key(c, &["esc"])?;
            }
            2 | 3 => {
                let chord = if gesture == 2 { "a" } else { "n" };
                key(c, &["meta_l", chord])?;
                key(c, &["meta_l", chord])?;
            }
            4 | 5 => {
                let x = if gesture == 4 { 1140 } else { 1220 };
                click(c, p, x, 696)?;
                click(c, p, x, 696)?;
            }
            6 | 7 => {
                key(c, &["meta_l"])?;
                key(c, if gesture == 6 { &["esc"] } else { &["meta_l"] })?;
            }
            8 => {
                // A click on Quick settings' own background (no tile or
                // button) keeps it open, as in Windows; Esc then closes it
                // and returns keyboard focus to Notes. A click outside would
                // light-dismiss it and give focus to whatever is under the
                // pointer (the desktop), so Notes would rightly lose focus.
                let layout = door_window::taskbar::Layout::from_bits(1280, 720, 48)
                    .ok_or("default taskbar layout")?;
                let (x, y) = door_window::tray::quick_background_point(layout);
                key(c, &["meta_l", "a"])?;
                click(c, p, x, y)?;
                key(c, &["esc"])?;
            }
            9 => {
                key(c, &["meta_l", "a"])?;
                click(c, p, 200, 350)?;
            }
            10 => {
                key(c, &["meta_l"])?;
                click(c, p, 200, 350)?;
            }
            _ => {
                if gesture == 11 {
                    key(c, &["meta_l"])?;
                } else {
                    key(c, &["meta_l", "a"])?;
                }
                move_to(c, p, 200, 350)?;
                for down in [true, false] {
                    c.execute(
                        "input-send-event",
                        json!({"events":[{"type":"btn","data":{"down":down,"button":"right"}}]}),
                    )?;
                    thread::sleep(Duration::from_millis(200));
                }
                scene::capture(
                    run,
                    c,
                    &format!("focus-right-context-{gesture}"),
                    &[(410, 360, [237; 3])],
                )?;
                key(c, &["esc"])?;
            }
        }
        // A real edit, flush and independent disk parse prove input focus
        // returned; a decorative active title or successful key command cannot.
        for (keys, expected) in [(&["a"][..], &b"a"[..]), (&["backspace"][..], &b""[..])] {
            let offset = serial(&run.root, record)?.len();
            key(c, keys)?;
            key(c, &["ctrl", "s"])?;
            let deadline = Instant::now() + Duration::from_secs(15);
            while !serial(&run.root, record)?
                .get(offset..)
                .unwrap_or("")
                .contains("DOOR_P11 NOTES SAVED")
            {
                if Instant::now() >= deadline {
                    return Err(format!("shell gesture {gesture} lost Notes keyboard focus"));
                }
                thread::sleep(Duration::from_millis(50));
            }
            audit(&disk, expected)?;
        }
    }
    let before = serial(&run.root, record)?;
    let pid = before
        .lines()
        .rev()
        .find_map(|l| {
            l.split_once("APP_LAUNCHED app=2 pid=")
                .and_then(|(_, p)| p.trim().parse::<u64>().ok())
        })
        .ok_or("focus Notes pid")?;
    key(c, &["alt", "f4"])?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while !serial(&run.root, record)?
        .get(before.len()..)
        .unwrap_or("")
        .contains(&format!("CLIENT_EXIT pid={pid} code=0"))
    {
        if Instant::now() >= deadline {
            return Err("focus Notes did not close cleanly".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
    scene::capture(run, c, "focus-notes-closed", &[])?;
    run.check("desktop-shell-focus-return",json!({"gestures":13,"actual_edits_and_flushes":26,"independent_note_byte_audits":26,"empty_note_restored":true}));
    Ok(())
}
pub(super) fn exercise(
    run: &mut Run,
    c: &mut qmp::Client,
    p: &mut (i32, i32),
    record: &Value,
) -> Result<(), String> {
    focus_roundtrips(run, c, p, record)?;
    let initial = serial(&run.root, record)?;
    let clock = initial
        .lines()
        .find_map(|line| {
            line.split_once("DOOR_P12 CLOCK UTC ")
                .map(|(_, clock)| clock)
        })
        .ok_or("RTC did not supply tray time")?;
    // The taskbar shows that time converted to the session's zone and
    // regional format; a fresh profile has none set (UTC, en-US).
    let local = initial
        .lines()
        .find_map(|line| {
            line.split_once("DOOR_P12 CLOCK LOCAL ")
                .map(|(_, local)| local)
        })
        .ok_or("tray clock did not report its local time")?;
    if !local.starts_with("zone=Etc/UTC region=en-US ") {
        return Err(format!("unexpected default clock locale: {local}"));
    }
    // A guest RTC diagnostic, not a host time substituted into the product.
    run.check(
        "desktop-tray-clock",
        json!({"guest_utc":clock,"local":local,"source":"validated kernel CMOS RTC syscall"}),
    );
    // Every point below comes from the shell's tray/taskbar geometry (host
    // tested in door_window::tray). The flyouts are acrylic, so presence is
    // proved two ways: opaque tile fills inside Quick settings, and most of
    // each flyout surface differing from the bare desktop beneath it.
    let layout =
        door_window::taskbar::Layout::from_bits(1280, 720, 48).ok_or("default taskbar layout")?;
    let center = |r: door_window::graphics::Rect| {
        (r.x() + r.width() as i32 / 2, r.y() + r.height() as i32 / 2)
    };
    let quick = door_window::tray::quick_rect(layout);
    let calendar = door_window::tray::calendar_rect(layout);
    let notifications =
        door_window::tray::notifications_rect(layout).ok_or("notification center fits")?;
    let (_, month_row, previous_month, next_month) = door_window::tray::calendar_controls(layout);
    let (enabled, _, disabled) = door_window::tray::tile_colors(false);
    let gray = |bgra: [u8; 4]| [bgra[2], bgra[1], bgra[0]];
    let tile_probes = [3usize, 2].map(|i| {
        let (x, y) = door_window::tray::tile_fill_point(layout, i);
        (
            x as usize,
            y as usize,
            gray(if i == 3 { enabled } else { disabled }),
        )
    });
    let bare = scene::capture(run, c, "tray-bare-desktop", &[])?;
    let covered =
        |frame: &scene::Pixels, r: door_window::graphics::Rect, name: &str| -> Result<(), String> {
            let (mut differ, mut total) = (0usize, 0usize);
            for y in r.y()..r.bottom() {
                for x in r.x()..r.right() {
                    total += 1;
                    if frame.pixel(x as usize, y as usize)? != bare.pixel(x as usize, y as usize)? {
                        differ += 1;
                    }
                }
            }
            if differ * 10 < total * 7 {
                return Err(format!(
                    "{name} surface not shown ({differ}/{total} pixels differ)"
                ));
            }
            Ok(())
        };
    key(c, &["meta_l", "a"])?;
    let shown = scene::capture_settled(run, c, "quick-settings-keyboard", &tile_probes)?;
    covered(&shown, quick, "Quick settings")?;
    key(c, &["esc"])?;
    key(c, &["meta_l", "n"])?;
    let shown = scene::capture_settled(run, c, "notifications-keyboard", &[])?;
    covered(&shown, notifications, "Notification center")?;
    covered(&shown, calendar, "Calendar")?;
    key(c, &["esc"])?;
    let (x, y) = center(layout.tray(door_window::taskbar::TrayTarget::Clock));
    click(c, p, x, y)?;
    wait_serial(
        &run.root,
        record,
        "TRAY open=Some(Calendar) month=None",
        Duration::from_secs(5),
    )?;
    // The flyout fades in (motion): measure it once it has settled.
    let before = scene::capture_settled(run, c, "calendar-current", &[])?;
    covered(&before, calendar, "Calendar")?;
    let (x, y) = center(previous_month);
    click(c, p, x, y)?;
    wait_serial(
        &run.root,
        record,
        "TRAY open=Some(Calendar) month=Some(",
        Duration::from_secs(5),
    )?;
    // The marker precedes presentation: compare against the settled frame.
    let previous = scene::capture_settled(run, c, "calendar-previous", &[])?;
    // The month heading text (left of the arrows) must repaint.
    let month_changes = (month_row.x()..previous_month.x())
        .flat_map(|x| (month_row.y()..month_row.bottom()).map(move |y| (x as usize, y as usize)))
        .filter(|&(x, y)| before.pixel(x, y) != previous.pixel(x, y))
        .count();
    if month_changes == 0 {
        return Err("calendar navigation did not repaint month heading".into());
    }
    let (x, y) = center(next_month);
    click(c, p, x, y)?;
    scene::capture(run, c, "calendar-next", &[])?;
    key(c, &["esc"])?;
    let (x, y) = center(layout.tray(door_window::taskbar::TrayTarget::QuickSettings));
    click(c, p, x, y)?;
    wait_serial(
        &run.root,
        record,
        "TRAY open=Some(Devices)",
        Duration::from_secs(5),
    )?;
    scene::capture_settled(run, c, "quick-settings", &tile_probes)?;
    let admitted = launches(&serial(&run.root, record)?);
    let (x, y) = center(door_window::tray::settings_button(layout));
    click(c, p, x, y)?;
    wait_serial(
        &run.root,
        record,
        "APP_ACTIVATED app=3",
        Duration::from_secs(10),
    )?;
    // Settings opens at its initial placement (`window_points`).
    let (sx, sy) = super::window_points::settings_background()?;
    let (sx, sy) = (sx as usize, sy as usize);
    scene::capture(
        run,
        c,
        "quick-settings-open-settings",
        &[(sx, sy, SETTINGS_BACKGROUND)],
    )?;
    if launches(&serial(&run.root, record)?) != admitted + 1 {
        return Err("quick settings did not admit one real Settings process".into());
    }
    // Native caption visuals and cancellation must agree with command timing.
    // Button centres at Settings' initial placement; probes sit 14 px left
    // of centre, clear of the glyph.
    let [minimize, maximize, close] = super::window_points::settings_caption()?;
    let probe = |(x, y): (i32, i32)| ((x - 14) as usize, y as usize);
    for ((x, y), name) in [(minimize, "minimize"), (maximize, "maximize")] {
        move_to(c, p, x, y)?;
        let (px, py) = probe((x, y));
        scene::capture(
            run,
            c,
            &format!("caption-{name}-hover"),
            &[(px, py, [225, 225, 225])],
        )?;
    }
    let (cx, cy) = probe(close);
    move_to(c, p, close.0, close.1)?;
    scene::capture(run, c, "caption-close-hover", &[(cx, cy, [232, 35, 35])])?;
    button(c, true)?;
    scene::capture(run, c, "caption-close-pressed", &[(cx, cy, [197, 45, 45])])?;
    // Off the caption buttons, onto Settings' client (host-tested in
    // `window_points`).
    move_to(c, p, 1000, 110)?;
    button(c, false)?;
    // Back to the client-drawn caption under the button.
    scene::capture(
        run,
        c,
        "caption-close-cancelled",
        &[(cx, cy, SETTINGS_BACKGROUND)],
    )?;
    let now = serial(&run.root, record)?;
    let last = now
        .rsplit_once("APP_LAUNCHED app=3 pid=")
        .ok_or("Settings identity")?
        .1;
    let pid = last.lines().next().ok_or("Settings pid")?.trim();
    if now.contains(&format!("CLIENT_EXIT pid={pid}")) {
        return Err("dragging off Close still closed Settings".into());
    }
    cursor_step(run, c, p, 12, "settings-caption-cancel-pointer")?;
    key(c, &["alt", "f4"])?;
    thread::sleep(Duration::from_secs(1));
    let count = launches(&serial(&run.root, record)?);
    key(c, &["meta_l"])?;
    type_line(c, "zzzz")?;
    // Typing in Start switches to the 26H2 Search panel (no footer, its own
    // height); its body is acrylic, so probe the opaque search box.
    scene::capture(run, c, "start-no-results", &[search_probe(1280, 720, 48)?])?;
    if launches(&serial(&run.root, record)?) != count {
        return Err("empty Start search launched an app".into());
    }
    // The first Esc clears the query (back to Start), the second closes it.
    key(c, &["esc"])?;
    key(c, &["esc"])?;
    key(c, &["meta_l"])?;
    type_line(c, "settings")?;
    scene::capture(
        run,
        c,
        "start-search-settings",
        &[(sx, sy, SETTINGS_BACKGROUND)],
    )?;
    if launches(&serial(&run.root, record)?) != count + 1 {
        return Err("Start search did not launch Settings exactly once".into());
    }
    key(c, &["alt", "f4"])?;
    thread::sleep(Duration::from_secs(1));
    // 26H2: the taskbar Search box opens the dedicated Search panel, not
    // Start (the serial names the panel; the capture keeps the taskbar's
    // coloured glyph and the open panel). Esc dismisses it; Win+S opens it
    // again and an outside click light-dismisses it, as Start does.
    let layout =
        door_window::taskbar::Layout::from_bits(1280, 720, 48).ok_or("default taskbar layout")?;
    let slot = layout.slot(door_window::taskbar::Slot::Search, door_window::apps::COUNT);
    let opened = |run: &Run| -> Result<usize, String> {
        Ok(serial(&run.root, record)?
            .matches("SHELL_PANEL open=search")
            .count())
    };
    let before = opened(run)?;
    click(c, p, slot.x() + 20, slot.y() + slot.height() as i32 / 2)?;
    scene::capture_settled(run, c, "search-panel-open", &[search_probe(1280, 720, 48)?])?;
    if opened(run)? != before + 1 || launches(&serial(&run.root, record)?) != count + 1 {
        return Err("taskbar Search did not open the Search panel (alone)".into());
    }
    key(c, &["esc"])?;
    key(c, &["meta_l", "s"])?;
    scene::capture(
        run,
        c,
        "search-panel-win-s",
        &[search_probe(1280, 720, 48)?],
    )?;
    if opened(run)? != before + 2 {
        return Err("Win+S did not open the Search panel".into());
    }
    click(c, p, 200, 350)?;
    thread::sleep(Duration::from_millis(500));
    run.check("desktop-trusted-shell-interactions",json!({"real_calendar_month_heading_changes":month_changes,"quick_settings_launch":true,"caption_hover_pressed_and_drag_cancel":true,"start_empty_and_filtered_activation":true,"search_panel_from_taskbar_and_win_s":true,"scope":"bounded guest interactions; Windows pixel fidelity and worst-case fluidity unqualified"}));
    multitask::exercise(run, c, p, record)?;
    Ok(())
}
