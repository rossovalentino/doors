//! Actual native scanout and client lifecycle at Full HD, 16:10 and 4K.
use super::*;
/// Settings' light background: Mica over the default Bloom.
pub(super) const SETTINGS_BACKGROUND: [u8; 3] = super::mica::settings(0, false);
/// Its dark background after Doors (dark): Mica over dark Bloom.
const SETTINGS_DARK: [u8; 3] = super::mica::settings(1, true);
fn overview_presented(run: &Run, record: &Value, offset: usize, mode: &str) -> Result<(), String> {
    let expected = format!("DOOR_P12 OVERVIEW_PRESENTED mode={mode} frame=");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !serial(&run.root, record)?
        .get(offset..)
        .unwrap_or("")
        .contains(&expected)
    {
        if Instant::now() >= deadline {
            return Err(format!("complete overview scanout missing: {expected}"));
        }
        thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}
pub(super) fn activate(
    root: &std::path::Path,
    record: &Value,
    c: &mut qmp::Client,
    index: usize,
) -> Result<(), String> {
    let before = serial(root, record)?.len();
    key(c, &["meta_l"])?;
    // Start opens with focus in its search pill; the first Right enters the
    // Pinned row and each further Right walks it (Down moves a grid row).
    for _ in 0..=index {
        key(c, &["right"])?;
    }
    key(c, &["ret"])?;
    wait_activation(
        root,
        record,
        *([1usize, 2, 3, 9, 10, 11]
            .get(index)
            .ok_or("unknown desktop app index")?),
        before,
    )
}
pub(super) fn wait_activation(
    root: &std::path::Path,
    record: &Value,
    app: usize,
    before: usize,
) -> Result<(), String> {
    let expected = format!("APP_ACTIVATED app={app}");
    let deadline = Instant::now() + Duration::from_secs(25);
    while !serial(root, record)?
        .get(before..)
        .unwrap_or("")
        .contains(&expected)
    {
        if Instant::now() >= deadline {
            return Err(format!("actual foreground activation missing: {expected}"));
        }
        thread::sleep(Duration::from_millis(100));
    }
    let text = serial(root, record)?;
    let activated_at = before
        + text
            .get(before..)
            .unwrap_or("")
            .find(&expected)
            .ok_or("activation marker disappeared")?
        + expected.len();
    let presented = format!("SCENE_PRESENTED app={app} overlay=0");
    let deadline = Instant::now() + Duration::from_secs(20);
    while !serial(root, record)?
        .get(activated_at..)
        .unwrap_or("")
        .contains(&presented)
    {
        if Instant::now() >= deadline {
            return Err(format!("foreground scanout did not complete: {presented}"));
        }
        thread::sleep(Duration::from_millis(50));
    }
    thread::sleep(Duration::from_millis(300));
    Ok(())
}
pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    verify_modes(run, image, &[(1920, 1080), (2560, 1600)])?;
    density(run, image, 3)?;
    density(run, image, 2)
}
/// Verify actual scanout against pinned native glyph coverage, including shadow
/// and unchanged surrounding wallpaper. Thin LCD strokes need not contain any
/// fully white RGB triples. This is a raster/placement check, not ClearType or
/// Windows appearance qualification.
fn labels(run: &mut Run, frame: &scene::Pixels, scale: u32, mode: &str) -> Result<(), String> {
    use door_graphics_text::{Clip, FontAtlas, Target, TextRun, TextStyle};
    let mut expected = Vec::new();
    let pack = door_window::wallpaper::Pack::parse(include_bytes!(
        "../../../../assets/wallpapers/windows/desktop-assets.blob"
    ))
    .ok_or("label reference wallpaper pack")?;
    door_window::wallpaper::load(
        pack.background(0).ok_or("label reference background")?,
        frame.width as u32,
        frame.height as u32,
        &mut expected,
    )?;
    let fonts =
        FontAtlas::windows_reference_scaled(scale).map_err(|e| format!("label font: {e:?}"))?;
    let mut checked = 0;
    for (i, label) in ["File Explorer", "Notes", "Settings"]
        .into_iter()
        .enumerate()
    {
        // App icons follow the Recycle Bin (`desktop_grid::app_slot`).
        let (center, label_top) =
            door_window::apps::desktop_grid::label(door_window::apps::desktop_grid::app_slot(i));
        let top = label_top as u32 * scale;
        let width = fonts
            .measure(label, TextStyle::Regular12)
            .map_err(|e| format!("label metrics: {e:?}"))?
            .advance_width;
        let left = (center - width as i32 / 2) * scale as i32;
        let mut target = Target::new(
            &mut expected,
            frame.width as u32,
            frame.height as u32,
            frame.width * 4,
        )
        .map_err(|e| format!("label target: {e:?}"))?
        // The desktop draws its icon labels with ClearType (desktop.rs).
        .cleartype();
        for (x, y, color) in [
            (left + scale as i32, top + scale, [0, 0, 0, 128]),
            (left, top, [255, 255, 255, 255]),
        ] {
            fonts
                .draw_physical(
                    &mut target,
                    Clip::new(
                        x,
                        y as i32,
                        frame.width as u32 - x.max(0) as u32,
                        18 * scale,
                    ),
                    TextRun {
                        text: label,
                        x,
                        top: y as i32,
                        style: TextStyle::Regular12,
                        color,
                    },
                )
                .map_err(|e| format!("label raster: {e:?}"))?;
        }
        for y in top as usize..(top + 19 * scale) as usize {
            for x in 0..104 * scale as usize {
                let at = (y * frame.width + x) * 4;
                let rgb = [expected[at + 2], expected[at + 1], expected[at]];
                if frame.pixel(x, y)? != rgb {
                    return Err(format!(
                        "native {mode} label {label:?} differs at {x},{y}: expected {rgb:?}, actual {:?}",
                        frame.pixel(x, y)?
                    ));
                }
                checked += 1;
            }
        }
    }
    run.check(&format!("desktop-label-raster-{mode}"), json!({"outcome":"passed","scale":scale,"labels":3,"compared_rgb_pixels":checked,"oracle":"pinned Segoe LCD glyphs and translucent shadow over pinned native wallpaper; exact placement and surrounding pixels","scope":"native raster integration; no Windows1:1 or ClearType engine claim"}));
    Ok(())
}
pub(super) fn verify_mode(run: &mut Run, image: &str, mode: &str) -> Result<(), String> {
    let dimensions = match mode {
        "2560x1440" => return density(run, image, 2),
        "1920x1080" => (1920, 1080),
        "2560x1600" => (2560, 1600),
        "3840x2160" => return density(run, image, 3),
        _ => {
            return Err(
                "Resolution regression modes:1920x1080,2560x1440,2560x1600,3840x2160".into(),
            );
        }
    };
    verify_modes(run, image, &[dimensions])
}
/// The system accent (RGB) the window server paints Task view and Alt+Tab
/// selections with: the default accent palette's fill (`door_window::paint`).
fn accent_rgb(dark: bool) -> [u8; 3] {
    let c = door_window::paint::Palette::new(dark).accent;
    [c[2], c[1], c[0]]
}
/// A physical point on the Alt+Tab selection ring around item `selected` of
/// `count` maximized client-caption windows (each client fills the logical
/// work area), from the shell's own layout (`door_window::overview`).
fn alt_tab_ring(
    logical: (u32, u32),
    scale: u32,
    count: usize,
    selected: usize,
    dark: bool,
) -> (usize, usize, [u8; 3]) {
    let work = (logical.0, logical.1 - 48);
    let sources = vec![work; count];
    let layout =
        door_window::overview::switch_layout((0, 0), work, &sources, 0).expect("Alt+Tab layout");
    let item = layout.items[selected].expect("visible item");
    let ring = door_window::overview::switch_ring(item).expect("ring");
    (
        ((ring.x() + door_window::overview::switch::RING as i32 / 2) as u32 * scale) as usize,
        ((item.y() + item.height() as i32 / 2) as u32 * scale) as usize,
        accent_rgb(dark),
    )
}
fn verify_modes(run: &mut Run, image: &str, modes: &[(i32, i32)]) -> Result<(), String> {
    for &(width, height) in modes {
        let scale = if width >= 2560 { 2 } else { 1 };
        let mode = format!("{width}x{height}");
        let disk = run.directory.join(format!("desktop-{mode}.raw"));
        fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
        let mut record = session::launch_desktop_resolution(run, image, &disk, false, &mode)?;
        let result: Result<(), String> = (|| {
            let mut c = connection(&record)?;
            let mut p = (width / 2, height / 2);
            thread::sleep(Duration::from_secs(2));
            let expected = if width == 3840 {
                vec![(0, 0, [133, 174, 202]), (3839, 0, [126, 170, 199])]
            } else {
                Vec::new()
            };
            // Desktop icons are off by default; turn them on (user preference).
            show_desktop_icons(&run.root, &record, &mut c, 112)?;
            let frame = scene::capture(run, &mut c, &format!("{mode}-empty"), &expected)?;
            if (frame.width, frame.height) != (width as usize, height as usize) {
                return Err(format!("requested {mode} was not actual scanout"));
            }
            labels(run, &frame, scale as u32, &mode)?;
            selection(run, &mut c, &mut p, &record, &mode)?;
            for index in 0..3 {
                double_click(&mut c, &mut p, desktop_icon(index, scale), true)?;
                wait_serial(
                    &run.root,
                    &record,
                    &format!("APP_LAUNCHED app={}", index + 1),
                    Duration::from_secs(20),
                )
                .map_err(|_| "native desktop double click failed or duplicated app")?;
                thread::sleep(Duration::from_secs(1));
            }
            let text = serial(&run.root, &record)?;
            for app in 1..=3 {
                if text
                    .lines()
                    .filter(|l| l.contains(&format!("APP_LAUNCHED app={app}")))
                    .count()
                    != 1
                {
                    return Err("native desktop double click failed or duplicated app".into());
                }
            }
            // Each app's Maximize button at its initial placement on this
            // logical output (`window_points`, host-tested). All three are
            // client-caption windows, so a maximized client fills the work area.
            let logical = ((width / scale) as u32, (height / scale) as u32);
            for (index, (app, color)) in [
                (window_points::App::Files, [255, 255, 255]),
                (window_points::App::Notes, [255, 255, 255]),
                (window_points::App::Settings, SETTINGS_BACKGROUND),
            ]
            .iter()
            .enumerate()
            {
                activate(&run.root, &record, &mut c, index)?;
                let (x, y) = window_points::maximize_button(*app, logical)?;
                click(&mut c, &mut p, x * scale, y * scale)?;
                wait_serial(
                    &run.root,
                    &record,
                    &format!(
                        "CAPTURED app={} width={width} height={}",
                        index + 1,
                        height - 48 * scale
                    ),
                    Duration::from_secs(20),
                )?;
                let mut expected = vec![(width as usize - 10, height as usize / 2, *color)];
                if index == 2 {
                    // Settings has no client footer: require the newly exposed
                    // lower rows as well. Files/Notes have actual footer widgets.
                    expected.push((width as usize - 10, height as usize - 100, *color));
                }
                scene::capture_settled(
                    run,
                    &mut c,
                    &format!("{mode}-max-{index}"),
                    // Sample the outer content margin. At native200% the Home
                    // cards legitimately occupy the former100px inset sample.
                    &expected,
                )?;
            }
            // All three maximized snapshots coexist before any is restored.
            if width == 3840 {
                let sample = (1000 * scale as usize, 20 * scale as usize);
                let backdrop = frame
                    .pixel(sample.0, sample.1)?
                    .map(|v| (8 + (u32::from(v) * 231 + 127) / 255) as u8);
                let before = serial(&run.root, &record)?.len();
                key(&mut c, &["meta_l", "tab"])?;
                overview_presented(run, &record, before, "task-view")?;
                scene::capture(
                    run,
                    &mut c,
                    &format!("{mode}-task-view-three-max"),
                    &[
                        (sample.0, sample.1, backdrop),
                        (2200, 184, accent_rgb(false)),
                    ],
                )?;
                let before = serial(&run.root, &record)?.len();
                key(&mut c, &["esc"])?;
                overview_presented(run, &record, before, "closed")?;
                scene::capture(
                    run,
                    &mut c,
                    &format!("{mode}-task-view-restored"),
                    &[(
                        width as usize - 10,
                        height as usize - 100,
                        SETTINGS_BACKGROUND,
                    )],
                )?;
                let before = serial(&run.root, &record)?.len();
                c.execute("input-send-event",json!({"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"alt"}}}]}))?;
                key(&mut c, &["tab"])?;
                overview_presented(run, &record, before, "alt-tab")?;
                scene::capture(
                    run,
                    &mut c,
                    &format!("{mode}-alt-tab-native-previews"),
                    &[
                        (sample.0, sample.1, backdrop),
                        alt_tab_ring(logical, scale as u32, 3, 1, false),
                        (1920, 1080, [255, 255, 255]),
                    ],
                )?;
                let before = serial(&run.root, &record)?.len();
                c.execute("input-send-event",json!({"events":[{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"alt"}}}]}))?;
                overview_presented(run, &record, before, "closed")?;
                scene::capture(
                    run,
                    &mut c,
                    &format!("{mode}-alt-tab-selected-notes"),
                    &[(width as usize - 100, height as usize / 2, [255, 255, 255])],
                )?;
                activate(&run.root, &record, &mut c, 2)?;
            }
            // Maximized Settings: the Personalization rail row, then the
            // Doors (dark) theme tile (`resolution_scenario_points_follow_the_settings_layout`
            // in desktop/workspace/tests/settings_reference_painter.rs).
            click(&mut c, &mut p, 140 * scale, 326 * scale)?;
            click(&mut c, &mut p, 660 * scale, 170 * scale)?;
            wait_serial(
                &run.root,
                &record,
                "DOOR_P12 APPEARANCE applied=3",
                Duration::from_secs(20),
            )?;
            scene::capture(
                run,
                &mut c,
                &format!("{mode}-theme-maximized"),
                &[
                    // Stay inside the content margin, clear of the scrollbar
                    // now visible on the longer Personalization page.
                    (
                        width as usize - 20 * scale as usize,
                        height as usize - 100,
                        SETTINGS_DARK,
                    ),
                    // Theme tiles describe their target, independent of the
                    // current dark palette, in both gallery rows; the applied
                    // tile carries the dark-mode accent ring.
                    (692 * scale as usize, 203 * scale as usize, [36, 37, 40]),
                    (933 * scale as usize, 203 * scale as usize, [249, 249, 249]),
                    (933 * scale as usize, 302 * scale as usize, [249, 249, 249]),
                    (692 * scale as usize, 302 * scale as usize, [36, 37, 40]),
                    (608 * scale as usize, 183 * scale as usize, [96, 205, 255]),
                ],
            )?;
            if width == 3840 {
                let before = serial(&run.root, &record)?.len();
                key(&mut c, &["meta_l", "tab"])?;
                overview_presented(run, &record, before, "task-view")?;
                scene::capture(
                    run,
                    &mut c,
                    &format!("{mode}-dark-task-view"),
                    &[(2200, 184, accent_rgb(true)), (2210, 200, [55, 55, 55])],
                )?;
                let before = serial(&run.root, &record)?.len();
                key(&mut c, &["esc"])?;
                overview_presented(run, &record, before, "closed")?;
                scene::capture(
                    run,
                    &mut c,
                    &format!("{mode}-dark-task-view-restored"),
                    &[(width as usize - 10, height as usize - 100, SETTINGS_DARK)],
                )?;
            }
            // Doors (light) (gallery tile 2) restores the default appearance.
            click(&mut c, &mut p, 900 * scale, 170 * scale)?;
            wait_serial(
                &run.root,
                &record,
                "DOOR_P12 SETTINGS SAVED appearance=0",
                Duration::from_secs(20),
            )?;
            for index in 0..3 {
                activate(&run.root, &record, &mut c, index)?;
                click(&mut c, &mut p, width - 69 * scale, 16 * scale)?;
                // Leave the caption buttons: a pointer resting on the next
                // maximized window's Restore opens its Snap layouts flyout,
                // an overlay that would hold back that app's foreground scanout.
                move_to(&mut c, &mut p, width / 2, height / 2)?;
                thread::sleep(Duration::from_secs(1));
                key(&mut c, &["alt", "f4"])?;
                thread::sleep(Duration::from_secs(1));
            }
            if c.execute("query-status", json!({}))?["running"] != true {
                return Err("closing native apps ended VM".into());
            }
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                let text = serial(&run.root, &record)?;
                let exits = text
                    .lines()
                    .filter(|l| l.contains("CLIENT_EXIT") && l.ends_with("code=0"))
                    .count();
                if exits == 3
                    || Instant::now() >= deadline
                    || text.contains("CLOSE TIMEOUT")
                    || text.contains("APP FAILED")
                {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
            let text = serial(&run.root, &record)?;
            if text.contains("APP FAILED")
                || text.contains("SERVER FAILED")
                || text.contains("INIT FAIL")
                || text
                    .lines()
                    .filter(|l| l.contains("CLIENT_EXIT") && l.ends_with("code=0"))
                    .count()
                    != 3
            {
                return Err("native resolution app lifecycle failed".into());
            }
            // Cross several buttons, then saturate emulated motion while moving
            // back to the same position. Input loss must cancel state, not exit.
            for _ in 0..5 {
                move_to(&mut c, &mut p, width / 2 - 24, height - 24)?;
                move_to(&mut c, &mut p, width / 2 + 64, height - 24)?;
            }
            let events: Vec<_> = (0..2000)
                .map(|i| json!({"type":"rel","data":{"axis":"x","value":if i%2==0 {1} else {-1}}}))
                .collect();
            c.execute("input-send-event", json!({"events":events}))?;
            thread::sleep(Duration::from_secs(2));
            if !serial(&run.root, &record)?.contains(&format!(
                "DOOR_P12 BAR FAST transferred_rows={}",
                48 * scale
            )) {
                return Err("native taskbar did not use partial scanout".into());
            }
            activate(&run.root, &record, &mut c, 1)?;
            type_line(&mut c, "High resolution note")?;
            save_and_shutdown(run, &record, &mut c, &mut p)?;
            wait_serial(
                &run.root,
                &record,
                "DOOR_P08 CLEANUP PASS",
                Duration::from_secs(30),
            )?;
            seal_serial(&run.root, &mut record)?;
            let saved = audit(&disk, b"High resolution note\n")?;
            if shutdown::canonical_for(&serial(&run.root, &record)?, 9)?["form"] != "save" {
                return Err(
                    "resolution shutdown must finish after Save without a second request".into(),
                );
            }
            run.check(&format!("desktop-resolution-{mode}"), json!({"outcome":"passed","record":record,"audit":saved,"native_output":[width,height],"ordinary_empty_boot":true,"double_click_singletons":3,"maximized_clients":3,"maximized_capture_protocol":2,"motion_events":2000,"partial_taskbar_rows":48*scale,"unsaved_shutdown_persisted":true,"document_close_protocol":2,"dpi":format!("{} percent native rasterization",100*scale)}));
            Ok(())
        })();
        if result.is_err()
            && let Ok(mut c) = connection(&record)
        {
            let _ = c.execute("quit", json!({}));
        }
        result?;
    }
    Ok(())
}
/// QHD at native 200% keeps the familiar1280x720 logical workspace while
/// providing four physical samples per logical pixel to the actual guest UI.
fn density(run: &mut Run, image: &str, scale: i32) -> Result<(), String> {
    density_backend(run, image, scale, Backend::Tcg, false)
}
pub(super) fn verify_accelerated(run: &mut Run, image: &str) -> Result<(), String> {
    density_backend(run, image, 3, Backend::Whpx, false)
}
pub(super) fn verify_motion(run: &mut Run, image: &str) -> Result<(), String> {
    density_backend(run, image, 3, Backend::InteractiveDefault, true)
}
#[derive(Clone, Copy)]
enum Backend {
    Tcg,
    Whpx,
    InteractiveDefault,
    InteractiveSoftware,
}
pub(super) fn verify_interactive(run: &mut Run, image: &str, software: bool) -> Result<(), String> {
    if !software {
        return super::native_default::verify(run, image);
    }
    density_backend(
        run,
        image,
        if software { 1 } else { 3 },
        if software {
            Backend::InteractiveSoftware
        } else {
            Backend::InteractiveDefault
        },
        false,
    )
}
fn density_backend(
    run: &mut Run,
    image: &str,
    scale: i32,
    backend: Backend,
    require_motion: bool,
) -> Result<(), String> {
    let (width, height) = (1280 * scale, 720 * scale);
    let mode = format!("{width}x{height}");
    let disk = run.directory.join(format!("desktop-density-{mode}.raw"));
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let accelerated = matches!(backend, Backend::Whpx | Backend::InteractiveDefault);
    let mut record = match backend {
        Backend::Whpx => session::launch_desktop_accelerated(run, image, &disk, false, &mode)?,
        Backend::Tcg => session::launch_desktop_resolution(run, image, &disk, false, &mode)?,
        Backend::InteractiveDefault | Backend::InteractiveSoftware => {
            let mut args = vec!["--headless".into(), "--resolution".into(), mode.clone()];
            if matches!(backend, Backend::InteractiveSoftware) {
                args.extend(["--accelerator".into(), "tcg".into()]);
            }
            session::DesktopOptions::parse(&args)?.launch(run, image, &disk)?
        }
    };
    run.check(
        "desktop-display-backend",
        json!({
            "accelerator": if accelerated { "whpx" } else { "tcg" },
            "display_device": if accelerated { "ramfb" } else { "VGA" },
            "profile": if accelerated { "profiles/hardware/qemu-desktop-whpx-x64.json" }
                else { "profiles/hardware/qemu-services-x64.json" },
            "native_output": [width, height],
            "argv": record["argv"],
            "scope": "Explicit runtime identity; no hardware-wide performance qualification"
        }),
    );
    let result: Result<(), String> = (|| {
        let mut c = connection(&record)?;
        let mut p = (width / 2, height / 2);
        wait_serial(
            &run.root,
            &record,
            &format!("DENSITY physical={mode} logical=1280x720 factor={scale}"),
            Duration::from_secs(10),
        )?;
        if serial(&run.root, &record)?.contains("APP_LAUNCHED") {
            return Err("QHD boot autostarted apps".into());
        }
        show_desktop_icons(&run.root, &record, &mut c, 112)?;
        let empty = scene::capture_settled(run, &mut c, &format!("{mode}-native-empty"), &[])?;
        if (empty.width, empty.height) != (width as usize, height as usize) {
            return Err("QHD scanout extent differs".into());
        }
        labels(run, &empty, scale as u32, &mode)?;
        for (index, (name, app)) in [
            ("explorer", door_window::apps::App::Files),
            ("notepad", door_window::apps::App::Notes),
            ("settings", door_window::apps::App::Settings),
        ]
        .iter()
        .enumerate()
        {
            let grid = door_window::apps::desktop_grid::icon(
                door_window::apps::desktop_grid::app_slot(index),
            );
            let side = (door_window::apps::icon_size(
                *app,
                door_window::apps::desktop_grid::ICON,
                door_window::graphics::Scale::from_factor(scale as u32).unwrap(),
            ) * scale as u32) as usize;
            let source = fs::read(
                run.root
                    .join(format!("assets/icons/windows/{name}.{side}.bgra")),
            )
            .map_err(|e| e.to_string())?;
            let at = source
                .chunks_exact(4)
                .position(|p| p[3] == 255)
                .ok_or("icon has no opaque detail")?;
            let expected = [source[at * 4 + 2], source[at * 4 + 1], source[at * 4]];
            let actual = empty.pixel(
                grid.0 as usize * scale as usize + at % side,
                grid.1 as usize * scale as usize + at / side,
            )?;
            if actual != expected {
                return Err(format!(
                    "native {name} desktop icon missing or scaled incorrectly"
                ));
            }
        }
        selection(run, &mut c, &mut p, &record, &mode)?;
        let before_start = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l"])?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while !serial(&run.root, &record)?
            .get(before_start..)
            .unwrap_or("")
            .contains("SCENE_PRESENTED app=0 overlay=1")
        {
            if Instant::now() >= deadline {
                return Err("Start scanout did not complete".into());
            }
            thread::sleep(Duration::from_millis(50));
        }
        let start =
            scene::capture_settled(run, &mut c, &format!("{mode}-native-start-icons"), &[])?;
        let launcher = door_window::taskbar::Layout::new(1280, 720);
        for (index, name) in ["explorer", "notepad", "settings"].iter().enumerate() {
            let side = 32 * scale as usize;
            let source = fs::read(
                run.root
                    .join(format!("assets/icons/windows/{name}.{side}.bgra")),
            )
            .map_err(|e| e.to_string())?;
            let at = source
                .chunks_exact(4)
                .position(|p| p[3] == 255)
                .ok_or("Start icon opacity")?;
            // The painter places each 32 px icon at tile + (28, 8).
            let tile = door_window::start::Geometry::new(launcher)
                .pinned_tile(index, 0)
                .ok_or("Start pinned tile")?;
            if start.pixel(
                (tile.x() as usize + 28) * scale as usize + at % side,
                (tile.y() as usize + 8) * scale as usize + at / side,
            )? != [source[at * 4 + 2], source[at * 4 + 1], source[at * 4]]
            {
                return Err(format!("native pinned Start icon {name} missing"));
            }
        }
        key(&mut c, &["esc"])?;
        for index in 0..3 {
            activate(&run.root, &record, &mut c, index)?;
        }
        activate(&run.root, &record, &mut c, 0)?;
        // Logical points from the real engine placement (`window_points`).
        let m = window_points::motion()?;
        let at = |(x, y): (i32, i32)| (x * scale, y * scale);
        if require_motion {
            // Exercise real caption hover without invoking the close button.
            let (x, y) = at(m.hover_close);
            move_to(&mut c, &mut p, x, y)?;
            thread::sleep(Duration::from_millis(100));
            let (x, y) = at(m.start);
            move_to(&mut c, &mut p, x, y)?;
            thread::sleep(Duration::from_millis(100));
        }
        let before =
            scene::capture_settled(run, &mut c, &format!("{mode}-before-cached-drag"), &[])?;
        let captures = serial(&run.root, &record)?
            .matches("CAPTURED app=1")
            .count();
        let started = Instant::now();
        scene::drag(
            &mut c,
            &mut p,
            // Empty caption strip left of Minimize, below the snap-layouts
            // reveal band, so this stays a plain window move.
            at(m.start),
            at(m.end),
        )?;
        if require_motion {
            // Paced private-QMP gestures exercise cached motion; these are not
            // a frame-cadence or input-latency benchmark. Each pair restores
            // exactly the geometry checked by the existing pixel probes.
            for _ in 0..8 {
                scene::drag(&mut c, &mut p, at(m.end), at(m.alternate))?;
                scene::drag(&mut c, &mut p, at(m.alternate), at(m.end))?;
            }
        }
        let (cx, cy) = at(m.chrome_before);
        let chrome = before.pixel(cx as usize, cy as usize)?;
        let (ax, ay) = at(m.chrome_after);
        let (ex, ey) = at(m.exposed);
        // Settings' light material (Mica, or its fallback while Explorer is
        // the active window).
        let exposed = SETTINGS_BACKGROUND;
        scene::capture_settled(
            run,
            &mut c,
            &format!("{mode}-cached-drag"),
            &[
                (ax as usize, ay as usize, chrome),
                (ex as usize, ey as usize, exposed),
            ],
        )?;
        let elapsed = started.elapsed().as_millis();
        if serial(&run.root, &record)?
            .matches("CAPTURED app=1")
            .count()
            != captures
        {
            return Err("moving cached content caused a new client configure/capture".into());
        }
        run.check(&format!("desktop-cached-drag-{mode}"), json!({"elapsed_including_gesture_and_stillness_ms":elapsed,"client_recaptures":0,"old_and_new_pixels":true,"fps_claim":false}));
        activate(&run.root, &record, &mut c, 2)?;
        // Settings at its initial placement, in front of the moved Explorer.
        let (sx, sy) = window_points::settings_background()?;
        scene::capture_settled(
            run,
            &mut c,
            &format!("{mode}-native-settings"),
            &[(
                (sx * scale) as usize,
                (sy * scale) as usize,
                SETTINGS_BACKGROUND,
            )],
        )?;
        let (maximize, minimize) = window_points::settings_max_min()?;
        click(&mut c, &mut p, maximize.0 * scale, maximize.1 * scale)?;
        wait_serial(
            &run.root,
            &record,
            &format!(
                "CAPTURED app=3 width={width} height={}",
                height - 48 * scale
            ),
            Duration::from_secs(20),
        )?;
        scene::capture_settled(
            run,
            &mut c,
            &format!("{mode}-native-settings-max"),
            &[(
                (width - 20 * scale) as usize,
                (250 * scale) as usize,
                SETTINGS_BACKGROUND,
            )],
        )?;
        // A real minimized Settings must restore through its Start singleton,
        // without recreating another process or changing the output density.
        click(&mut c, &mut p, minimize.0 * scale, minimize.1 * scale)?;
        activate(&run.root, &record, &mut c, 2)?;
        scene::capture_settled(
            run,
            &mut c,
            &format!("{mode}-native-settings-restored"),
            &[(
                (width - 20 * scale) as usize,
                (250 * scale) as usize,
                SETTINGS_BACKGROUND,
            )],
        )?;
        activate(&run.root, &record, &mut c, 1)?;
        type_line(&mut c, "Native density note")?;
        save_and_shutdown(run, &record, &mut c, &mut p)?;
        wait_serial(
            &run.root,
            &record,
            "DOOR_P08 CLEANUP PASS",
            Duration::from_secs(30),
        )?;
        seal_serial(&run.root, &mut record)?;
        let saved = audit(&disk, b"Native density note\n")?;
        let text = serial(&run.root, &record)?;
        if shutdown::canonical_for(&text, 8)?["form"] != "save" {
            return Err("density shutdown must finish after Save without a second request".into());
        }
        let motion = if require_motion || motion::present(&text) {
            Some(motion::validate(&text)?)
        } else {
            None
        };
        if require_motion {
            let summary = motion.as_ref().ok_or("required motion aggregate absent")?;
            if summary["move_reuse"]["pixels"].as_u64().unwrap_or(0) == 0 {
                return Err("contained opaque drag did not exercise translated-pixel reuse".into());
            }
            if summary["groups"][1]["paths"][0].as_u64().unwrap_or(0) < 8
                || summary["groups"][0]["paths"]
                    .as_array()
                    .is_none_or(|p| p.iter().any(|v| v.as_u64().unwrap_or(0) == 0))
            {
                return Err("private motion did not exercise drag and all four paint paths".into());
            }
            run.check("desktop-motion-aggregate", json!({"outcome":"passed","record":record,"summary":summary,
                "extra_round_trips":8,"gesture_pacing":"50 physical pixels/step, 70ms between pointer steps, 100ms button transitions",
                "client_recaptures":0,"fps_claim":false}));
        }
        run.check(&format!("desktop-native-density-{mode}"),json!({"outcome":"passed","record":record,"audit":saved,"native_output":[width,height],"logical_output":[1280,720],"dpi":96 * scale,"scale":scale,"simultaneous_apps":3,"minimize_restore":true,"source_rasterization":true,"host_enlargement":false,"motion":motion}));
        Ok(())
    })();
    if result.is_err()
        && let Ok(mut c) = connection(&record)
    {
        let _ = c.execute("quit", json!({}));
    }
    result
}

/// A dirty document holds the shutdown while its prompt is open (Windows 11).
/// Save in the real Close prompt retires that exact client and the shutdown
/// then finishes by itself. Never replace this path with an earlier Ctrl+S.
fn save_and_shutdown(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    pointer: &mut (i32, i32),
) -> Result<(), String> {
    let _ = pointer;
    let before = serial(&run.root, record)?;
    let pid = before
        .lines()
        .rev()
        .find_map(|line| {
            line.split_once("APP_LAUNCHED app=2 pid=")
                .and_then(|(_, value)| value.trim().parse::<u64>().ok())
        })
        .ok_or("resolution Notes launch identity missing")?;
    start_shut_down(c)?;
    let prompt = format!("DOOR_USER pid={pid} DOOR_P12 NOTES PROMPT intent=Close");
    let deadline = Instant::now() + Duration::from_secs(25);
    while !serial(&run.root, record)?
        .get(before.len()..)
        .unwrap_or("")
        .lines()
        .any(|line| line == prompt)
    {
        if Instant::now() >= deadline {
            return Err("resolution shutdown did not produce a fresh Close prompt".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
    // Enter belongs to the prompt only once Notes is the presented
    // foreground again; until then Start's dismissal still owns the keyboard.
    let prompted = before.len()
        + serial(&run.root, record)?
            .get(before.len()..)
            .unwrap_or("")
            .find(&prompt)
            .ok_or("resolution Close prompt disappeared")?;
    let deadline = Instant::now() + Duration::from_secs(25);
    while !serial(&run.root, record)?
        .get(prompted..)
        .unwrap_or("")
        .contains("SCENE_PRESENTED app=2 overlay=0")
    {
        if Instant::now() >= deadline {
            return Err("resolution Close prompt never reached the foreground".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
    let offset = serial(&run.root, record)?.len();
    key(c, &["ret"])?;
    let deadline = Instant::now() + Duration::from_secs(25);
    while !serial(&run.root, record)?
        .get(offset..)
        .unwrap_or("")
        .lines()
        .any(|line| line == format!("DOOR_USER pid=1 DOOR_P11 CLIENT_EXIT pid={pid} code=0"))
    {
        if Instant::now() >= deadline {
            return Err("resolution Save did not retire its document client".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
    // Save closed the document and the shutdown continues by itself
    // (Windows 11); no second request.
    Ok(())
}
