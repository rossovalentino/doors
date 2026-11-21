//! Native desktop workflows on isolated disks. Owner data is never a test fixture.
use crate::{
    evidence::{Run, file_hash, relative, write_json},
    paths, qmp, session,
};
use serde_json::{Value, json};
use std::{
    fs, thread,
    time::{Duration, Instant},
};
mod administration;
mod mica;
mod p14_inbox;
mod p14_settings;
mod p15_connect;
mod p15_desktop;
mod p15_ftp;
mod p15_http;
mod p15_names;
mod p15_sound;
mod p15_tls;
mod p15_vpn;
mod p16_accounts;
mod p16_cmd;
mod p16_com;
mod p16_comoop;
mod p16_crypt;
mod p16_install;
mod p16_oleaut;
mod p16_paths;
mod p16_scm;
mod p16_security;
mod p16_shell;
mod p16_trust;
mod p16_tsched;
mod p17_x86;
mod p18_crt;
mod p18_dotnet;
mod p18_electron;
mod p18_mem;
mod p18_pwsh;
mod p18_pwsh_interactive;
mod reference_routes;
mod settings_reference;
pub fn p14_inbox_test(run: &mut Run, image: &str) -> Result<(), String> {
    p14_inbox::verify(run, image)
}
pub fn p14_settings_test(run: &mut Run, image: &str) -> Result<(), String> {
    p14_settings::verify(run, image)
}
mod terminal;
pub fn reference_apps_preview(run: &mut Run) -> Result<(), String> {
    for (app, pages) in [
        ("monitor", (0..10).chain([20, 21]).collect::<Vec<_>>()),
        ("preview", (0..17).collect()),
    ] {
        for page in pages {
            for theme in ["light", "dark"] {
                let source = run
                    .root
                    .join(format!("target/{app}-review/page-{page}-{theme}.ppm"));
                let png = run.directory.join(format!("{app}-{page}-{theme}-host.png"));
                scene::Pixels::read_host_preview(&source)?.save_png(&png)?;
                run.check("reference-app-host-painter",json!({"app":app,"page":page,"theme":theme,"source":relative(&run.root,&source)?,"source_sha256":file_hash(&source)?,"png":relative(&run.root,&png)?,"sha256":file_hash(&png)?,"scope":"Actual application painter at1472x952 with explicit host fixture data; not guest execution, measured reference equality or backend qualification"}));
            }
        }
    }
    Ok(())
}
pub fn settings_preview(run: &mut Run) -> Result<(), String> {
    for page in [18, 19, 21, 17, 24, 26] {
        for theme in ["light", "dark"] {
            let source = run
                .root
                .join(format!("target/settings-review/page-{page}-{theme}.ppm"));
            let png = run
                .directory
                .join(format!("settings-{page}-{theme}-host-preview.png"));
            scene::Pixels::read_host_preview(&source)?.save_png(&png)?;
            run.check("settings-host-painter-review",json!({"page":page,"theme":theme,"source_sha256":file_hash(&source)?,"png":relative(&run.root,&png)?,"sha256":file_hash(&png)?,"scope":"Actual painter with explicit host fixtures; no guest or functionality qualification"}));
        }
    }
    Ok(())
}
pub fn terminal_test(run: &mut Run, image: &str) -> Result<(), String> {
    terminal::verify(run, image)
}
pub fn explorer_preview(run: &mut Run) -> Result<(), String> {
    for name in [
        "light",
        "dark",
        "narrow",
        "contents",
        "tags",
        "reference-dark-2048",
    ] {
        let ppm = run.root.join(format!("target/explorer-review/{name}.ppm"));
        let png = run
            .directory
            .join(format!("explorer-{name}-host-preview.png"));
        scene::Pixels::read_host_preview(&ppm)?.save_png(&png)?;
        run.check(&format!("explorer-{name}-host-preview"),json!({"source":relative(&run.root,&ppm)?,"source_sha256":file_hash(&ppm)?,"png":relative(&run.root,&png)?,"sha256":file_hash(&png)?,"scope":"Actual Explorer native painter with explicit fixture data; host raster only, not guest execution or exact-Windows qualification"}));
    }
    Ok(())
}
pub fn terminal_preview(run: &mut Run) -> Result<(), String> {
    for name in [
        "home",
        "settings",
        "about",
        "profiles",
        "home-dark",
        "settings-dark",
        "about-dark",
        "profiles-dark",
    ] {
        let ppm = run.root.join(format!("target/terminal-review/{name}.ppm"));
        let png = run
            .directory
            .join(format!("terminal-{name}-host-preview.png"));
        scene::Pixels::read_host_preview(&ppm)?.save_png(&png)?;
        run.check(&format!("terminal-{name}-host-preview"),json!({"source":relative(&run.root,&ppm)?,"source_sha256":file_hash(&ppm)?,"png":relative(&run.root,&png)?,"sha256":file_hash(&png)?,"scope":"Actual painter host raster only; not guest execution"}));
    }
    Ok(())
}
pub fn settings_reference_test(run: &mut Run, image: &str) -> Result<(), String> {
    settings_reference::verify(run, image)
}
mod controls;
mod explorer;
mod explorer_modern;
pub fn explorer_modern_test(run: &mut Run, image: &str) -> Result<(), String> {
    explorer_modern::verify(run, image)
}
mod file_deletion;
mod file_drag;
mod file_mutations;
mod lifecycle;
mod motion;
mod multitask;
mod native_default;
mod notes;
mod recovery;
mod resolution;
mod scene;
mod shell;
mod shutdown;
mod validate;
mod window_points;
pub use validate::{revalidate, validate};
pub fn file_deletion_test(run: &mut Run, image: &str, accelerated: bool) -> Result<(), String> {
    file_deletion::verify(run, image, accelerated)
}
/// `cargo xtask test file-drag --image <image.json>`: Explorer drags a file
/// onto a folder and the disk shows it moved.
pub fn file_drag_test(run: &mut Run, image: &str) -> Result<(), String> {
    file_drag::verify(run, image)
}
pub fn notes_test(run: &mut Run, image: &str, accelerated: bool) -> Result<(), String> {
    notes::verify(run, image, accelerated)
}
pub fn recovery_test(run: &mut Run, image: &str) -> Result<(), String> {
    recovery::verify(run, image)
}
pub fn file_mutation_test(run: &mut Run, image: &str) -> Result<(), String> {
    file_mutations::verify(run, image)
}
pub fn file_mutation_accelerated_test(run: &mut Run, image: &str) -> Result<(), String> {
    file_mutations::verify_accelerated(run, image)
}
pub fn native_default_test(run: &mut Run, image: &str) -> Result<(), String> {
    native_default::verify(run, image)
}
pub fn wide_test(run: &mut Run, image: &str) -> Result<(), String> {
    lifecycle::verify(run, image)
}
pub fn shell_test(run: &mut Run, image: &str) -> Result<(), String> {
    shell::verify(run, image)
}
pub fn resolution_test(run: &mut Run, image: &str) -> Result<(), String> {
    resolution::verify(run, image)
}
pub fn resolution_test_mode(run: &mut Run, image: &str, mode: &str) -> Result<(), String> {
    resolution::verify_mode(run, image, mode)
}
pub fn accelerated_test(run: &mut Run, image: &str) -> Result<(), String> {
    resolution::verify_accelerated(run, image)
}
pub fn interactive_test(run: &mut Run, image: &str, software: bool) -> Result<(), String> {
    resolution::verify_interactive(run, image, software)
}
pub fn motion_test(run: &mut Run, image: &str) -> Result<(), String> {
    resolution::verify_motion(run, image)
}
/// One step of the P11 desktop gate (`verify_all`), recording into the
/// gate's run. `exclusive` steps measure timing, select the host backend or
/// race a deliberately crashed system process against its restart, so the
/// guest pool (`parallel`) runs them with no other guest in flight.
pub struct GateStep {
    pub name: &'static str,
    pub exclusive: bool,
    pub run: fn(&mut Run, &str, &str) -> Result<(), String>,
}
/// The P11 desktop gate steps, in order.
pub const GATE_STEPS: &[GateStep] = &[
    GateStep {
        name: "desktop-suite",
        exclusive: false,
        run: |run, image, _| verify(run, image),
    },
    // Crashes the window server and requires Init to restart it in time:
    // under four concurrent guests Init once failed (stage 40, I/O failure)
    // where the same step passes alone.
    GateStep {
        name: "desktop-recovery",
        exclusive: true,
        run: |run, _, test_image| recovery::verify(run, test_image),
    },
    // Cursor-response latency samples (at most 1000 ms under TCG).
    GateStep {
        name: "desktop-wide",
        exclusive: true,
        run: |run, image, _| lifecycle::verify(run, image),
    },
    // 4K/QHD TCG density runs record cached-drag elapsed times.
    GateStep {
        name: "desktop-resolutions",
        exclusive: true,
        run: |run, image, _| resolution::verify(run, image),
    },
    // Automatic host display mode and WHPX backend selection.
    GateStep {
        name: "desktop-native-default",
        exclusive: true,
        run: |run, image, _| native_default_test(run, image),
    },
];
pub fn verify_all(run: &mut Run, image: &str, test_image: &str) -> Result<(), String> {
    for step in GATE_STEPS {
        crate::evidence::timed(run, "desktop-gate-step", step.name, |run| {
            (step.run)(run, image, test_image)
        })?;
    }
    Ok(())
}
/// Acceptance suites whose runners boot the shared Windows GUI fixture image
/// (`win32::stage`, directly or through the P14 shell stage).
pub fn uses_win32_stage(suite: &str) -> bool {
    matches!(
        suite,
        "win32-gui-f1"
            | "win32-gui-f2"
            | "win32-gui-f3"
            | "scenario-141-5"
            | "scenario-141-6"
            | "desktop-a-shell"
            | "recover-01"
    )
}
/// Builds and stages the Windows GUI fixture image once for `run` (the
/// suites' own `win32::stage` calls then reuse it).
pub fn stage_win32(run: &mut Run, image: &str) -> Result<(), String> {
    win32::stage(run, image).map(|_| ())
}
pub fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let disk = run.directory.join("desktop-user.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut first = session::launch_desktop(run, image, &disk, false)?;
    let result = workflows(run, &first);
    if result.is_err()
        && let Ok(mut c) = connection(&first)
    {
        let screen = run.directory.join("desktop-failure.png");
        let _=c.execute("screendump",json!({"filename":screen.to_string_lossy().trim_start_matches("\\\\?\\").replace('\\',"/"),"format":"png"}));
        let _ = c.execute("quit", json!({}));
    }
    result?;
    seal_serial(&run.root, &mut first)?;
    let saved = audit(&disk, b"Persistent desktop note\n")?;
    require_folders(&saved)?;
    let mut second = session::launch_desktop(run, image, &disk, false)?;
    let outcome: Result<(), String> = (|| {
        thread::sleep(Duration::from_secs(2));
        let mut c = connection(&second)?;
        let mut pointer = (512, 384);
        start_apps(&run.root, &second, &mut c)?;
        click(&mut c, &mut pointer, 850, 390)?;
        type_line(&mut c, " reopened")?;
        key(&mut c, &["ctrl", "s"])?;
        wait_serial(
            &run.root,
            &second,
            "DOOR_P11 NOTES SAVED",
            Duration::from_secs(10),
        )?;
        key(&mut c, &["ctrl", "w"])?;
        wait_serial(
            &run.root,
            &second,
            "DOOR_P11 CLIENT_EXIT",
            Duration::from_secs(10),
        )?;
        shutdown(&run.root, &second, &mut c, &mut pointer)?;
        Ok(())
    })();
    if outcome.is_err()
        && let Ok(mut c) = connection(&second)
    {
        let _ = c.execute("quit", json!({}));
    }
    outcome?;
    seal_serial(&run.root, &mut second)?;
    let reopened = audit(&disk, b"Persistent desktop note\n reopened\n")?;
    require_folders(&reopened)?;
    let evidence = json!({"schema_version":1,"record_kind":"native_desktop_suite","outcome":"passed","source_snapshot":run.build_inputs(),"image":image,"disk":relative(&run.root,&disk)?,"first":first,"second":second,"saved":saved,"reopened":reopened,"visual_fidelity":"not_qualified","boot_test_token":false});
    write_json(&run.directory.join("desktop-suite.json"), &evidence)?;
    run.check("native-desktop-suite", evidence);
    Ok(())
}
fn start_apps(
    root: &std::path::Path,
    record: &Value,
    client: &mut qmp::Client,
) -> Result<(), String> {
    // Start focuses its search pill; Right enters the Pinned row.
    key(client, &["meta_l"])?;
    key(client, &["right"])?;
    key(client, &["ret"])?;
    wait_serial(root, record, "CAPTURED app=1", Duration::from_secs(20))?;
    key(client, &["meta_l"])?;
    key(client, &["right"])?;
    key(client, &["right"])?;
    key(client, &["ret"])?;
    // A restored document performs real storage reads before its first surface.
    // Clicking a presumed rectangle earlier can legitimately focus the desktop
    // and suppress a late client's foreground request.
    wait_serial(root, record, "CAPTURED app=2", Duration::from_secs(20))?;
    Ok(())
}
fn workflows(run: &mut Run, record: &Value) -> Result<(), String> {
    let mut client = connection(record)?;
    start_apps(&run.root, record, &mut client)?;
    thread::sleep(Duration::from_secs(3));
    let screen = run.directory.join("desktop-initial.png");
    client.execute("screendump",json!({"filename":screen.to_string_lossy().trim_start_matches("\\\\?\\").replace('\\',"/"),"format":"png"}))?;
    run.check(
        "desktop-first-render",
        json!({"screenshot":relative(&run.root,&screen)?,"sha256":file_hash(&screen)?}),
    );
    let mut pointer = (512, 384);
    click(&mut client, &mut pointer, 850, 390)?;
    type_line(&mut client, "Persistent desktop note")?;
    key(&mut client, &["ctrl", "s"])?;
    wait_serial(
        &run.root,
        record,
        "DOOR_P11 NOTES SAVED",
        Duration::from_secs(10),
    )?;
    let screen = run.directory.join("desktop-edited.png");
    client.execute("screendump",json!({"filename":screen.to_string_lossy().trim_start_matches("\\\\?\\").replace('\\',"/"),"format":"png"}))?;
    scene::exercise(run, record, &mut client, &mut pointer)?;
    shutdown(&run.root, record, &mut client, &mut pointer)?;
    run.check("desktop-notes-workflow",json!({"typed":"Persistent desktop note\n","save":"Ctrl+S","screenshot":relative(&run.root,&screen)?,"sha256":file_hash(&screen)?}));
    Ok(())
}
/// Start > Power > Shut down through the keyboard. Start opens with focus in
/// its search pill; Shift+Tab reaches the footer power button, Enter opens
/// the power menu and Down skips the unsupported Sleep item.
fn start_shut_down(client: &mut qmp::Client) -> Result<(), String> {
    key(client, &["meta_l"])?;
    thread::sleep(Duration::from_millis(300));
    key(client, &["shift", "tab"])?;
    key(client, &["ret"])?;
    key(client, &["down"])?;
    key(client, &["ret"])
}
/// Desktop icons are off by default, as on a clean Windows 11 desktop. Toggle
/// them from the desktop context menu (Shift+F10, View > Show desktop icons)
/// while no window has focus. The choice is the user's DesktopControls
/// preference: Init stores it, then applies it to every client, so wait for
/// the saved controls `bits` (112 shows icons, 48 hides them).
fn show_desktop_icons(
    root: &std::path::Path,
    record: &Value,
    client: &mut qmp::Client,
    bits: u32,
) -> Result<(), String> {
    key(client, &["shift", "f10"])?;
    key(client, &["right"])?;
    key(client, &["down"])?;
    key(client, &["ret"])?;
    wait_serial(
        root,
        record,
        &format!("DOOR_P12 CONTROLS saved bits={bits}"),
        Duration::from_secs(15),
    )?;
    thread::sleep(Duration::from_millis(300));
    Ok(())
}
/// An opaque pixel of the open Start search pill (light theme field color)
/// for a logical output and desktop-controls bits, from the shell geometry.
fn start_probe(width: u32, height: u32, bits: u32) -> Result<(usize, usize, [u8; 3]), String> {
    start_probe_recent(width, height, bits, 0)
}
/// `start_probe` while Start lists `recent` Recent items (each grows the
/// panel, which stays anchored above the taskbar, so the pill moves up).
fn start_probe_recent(
    width: u32,
    height: u32,
    bits: u32,
    recent: usize,
) -> Result<(usize, usize, [u8; 3]), String> {
    let layout = door_window::taskbar::Layout::from_bits(width, height, bits)
        .ok_or("desktop controls bits")?;
    let pill = door_window::start::Geometry::with_recent(
        layout,
        door_window::apps::COUNT,
        door_window::start::View::Category,
        false,
        recent,
    )
    .search_box();
    Ok((
        (pill.right() - 24) as usize,
        (pill.y() + 16) as usize,
        [251, 251, 251],
    ))
}
/// The same opaque search-box pixel in the 26H2 Search panel (taskbar
/// Search, Win+S, or typing in Start), which has its own height.
fn search_probe(width: u32, height: u32, bits: u32) -> Result<(usize, usize, [u8; 3]), String> {
    let layout = door_window::taskbar::Layout::from_bits(width, height, bits)
        .ok_or("desktop controls bits")?;
    let pill = door_window::start::Geometry::search_panel(layout).search_box();
    Ok((
        (pill.right() - 24) as usize,
        (pill.y() + 16) as usize,
        [251, 251, 251],
    ))
}
fn key(client: &mut qmp::Client, keys: &[&str]) -> Result<(), String> {
    client.execute("send-key",json!({"keys":keys.iter().map(|key|json!({"type":"qcode","data":key})).collect::<Vec<_>>(),"hold-time":25}))?;
    thread::sleep(Duration::from_millis(200));
    Ok(())
}
fn type_line(client: &mut qmp::Client, text: &str) -> Result<(), String> {
    for ch in text.chars().chain(core::iter::once('\n')) {
        client.execute(
            "send-key",
            json!({"keys":session::keys(ch)?,"hold-time":20}),
        )?;
        thread::sleep(Duration::from_millis(70));
    }
    Ok(())
}
/// Opens Personalization > Colors through Settings search, then moves the
/// closed "Choose your mode" combo one step (Down = dark, Up = light). The
/// caller has already activated Settings; see `settings_reference.rs`.
fn settings_mode(client: &mut qmp::Client, dark: bool) -> Result<(), String> {
    key(client, &["ctrl", "f"])?;
    type_line(client, "colors")?;
    key(client, &["esc"])?;
    key(client, &["tab"])?;
    key(client, &[if dark { "down" } else { "up" }])
}
fn move_to(
    client: &mut qmp::Client,
    position: &mut (i32, i32),
    x: i32,
    y: i32,
) -> Result<(), String> {
    while *position != (x, y) {
        let dx = (x - position.0).clamp(-50, 50);
        let dy = (y - position.1).clamp(-50, 50);
        client.execute("input-send-event",json!({"events":[{"type":"rel","data":{"axis":"x","value":dx}},{"type":"rel","data":{"axis":"y","value":dy}}]}))?;
        position.0 += dx;
        position.1 += dy;
        thread::sleep(Duration::from_millis(70));
    }
    Ok(())
}
fn click(
    client: &mut qmp::Client,
    position: &mut (i32, i32),
    x: i32,
    y: i32,
) -> Result<(), String> {
    move_to(client, position, x, y)?;
    for down in [true, false] {
        client.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":down,"button":"left"}}]}),
        )?;
        thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}
/// Physical pointer point at the center of the selection plate of the app
/// at `index` of the desktop order (`door_window::apps::desktop_grid`: the
/// Recycle Bin holds the first slot), for the default bottom taskbar (work
/// area at the output origin).
fn desktop_icon(index: usize, scale: i32) -> (i32, i32) {
    use door_window::apps::desktop_grid;
    let (x, y, w, h) = desktop_grid::plate(desktop_grid::app_slot(index));
    ((x + w as i32 / 2) * scale, (y + h as i32 / 2) * scale)
}
/// A real double click (two presses well inside the 500 ms double-click
/// time even when QMP is slow under TCG). With `settle` false the second
/// release returns at once so the caller can sample the cursor while the
/// application starts.
fn double_click(
    client: &mut qmp::Client,
    position: &mut (i32, i32),
    (x, y): (i32, i32),
    settle: bool,
) -> Result<(), String> {
    move_to(client, position, x, y)?;
    for (i, down) in [true, false, true, false].into_iter().enumerate() {
        client.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":down,"button":"left"}}]}),
        )?;
        if i < 3 {
            thread::sleep(Duration::from_millis(50));
        }
    }
    if settle {
        thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}
/// Check actual pixels and absence of a process launch after one desktop click.
/// A delayed second single click must still select; only a bounded pair opens.
fn selection(
    run: &mut Run,
    client: &mut qmp::Client,
    pointer: &mut (i32, i32),
    record: &Value,
    name: &str,
) -> Result<(), String> {
    let baseline = scene::capture(run, client, &format!("{name}-selection-baseline"), &[])?;
    let scale = if baseline.width >= 3840 {
        3
    } else if baseline.width >= 2560 {
        2
    } else {
        1
    };
    // A plate pixel of the first app's icon, left of its art.
    let (px, py, _, _) =
        door_window::apps::desktop_grid::plate(door_window::apps::desktop_grid::app_slot(0));
    let probe = ((px + 8) as usize * scale, (py + 12) as usize * scale);
    let base = baseline.pixel(probe.0, probe.1)?;
    let icon = desktop_icon(0, scale as i32);
    click(client, pointer, icon.0, icon.1)?;
    move_to(client, pointer, 120 * scale as i32, 32 * scale as i32)?;
    let over = |destination: [u8; 3], source: [u32; 4]| -> [u8; 3] {
        core::array::from_fn(|i| {
            (source[i] + (u32::from(destination[i]) * (255 - source[3]) + 127) / 255).min(255) as u8
        })
    };
    let selected = over(over(base, [100, 145, 180, 180]), [40, 74, 100, 104]);
    scene::capture(
        run,
        client,
        &format!("{name}-single-selected"),
        &[(probe.0, probe.1, selected)],
    )?;
    thread::sleep(Duration::from_millis(600));
    click(client, pointer, icon.0, icon.1)?;
    if serial(&run.root, record)?.contains("APP_LAUNCHED") {
        return Err("a desktop single click opened an application".into());
    }
    click(client, pointer, 120 * scale as i32, 300 * scale as i32)?;
    scene::capture(
        run,
        client,
        &format!("{name}-selection-cleared"),
        &[(probe.0, probe.1, base)],
    )?;
    Ok(())
}
fn cursor_step(
    run: &mut Run,
    client: &mut qmp::Client,
    pointer: &mut (i32, i32),
    dx: i32,
    name: &str,
) -> Result<(), String> {
    let begin = Instant::now();
    client.execute(
        "input-send-event",
        json!({"events":[{"type":"rel","data":{"axis":"x","value":dx}}]}),
    )?;
    pointer.0 += dx;
    // Pinned arrow hotspot0,0 has opaque white ink at1,2. The previous
    // pointer cannot cover this pixel for these twelve-pixel horizontal steps.
    scene::capture(
        run,
        client,
        name,
        &[((pointer.0 + 1) as usize, (pointer.1 + 2) as usize, [255; 3])],
    )?;
    let elapsed = begin.elapsed().as_millis();
    if elapsed > 1000 {
        return Err(format!(
            "{name}: cursor response took {elapsed}ms (limit1000ms under TCG)"
        ));
    }
    run.check(&format!("desktop-{name}-latency"), json!({"observed_ms":elapsed,"maximum_ms":1000,"proof":"new cursor ink in actual QMP framebuffer","scope":"bounded TCG sample including screendump overhead; no FPS or worst-case guarantee"}));
    Ok(())
}
fn serial(root: &std::path::Path, record: &Value) -> Result<String, String> {
    fs::read_to_string(paths::existing(
        root,
        record["serial"].as_str().ok_or("desktop serial")?,
    )?)
    .map_err(|e| e.to_string())
}
fn wait_serial(
    root: &std::path::Path,
    record: &Value,
    needle: &str,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let text = serial(root, record)?;
        if text.contains(needle) {
            return Ok(());
        }
        if text.contains("APP FAILED")
            || text.contains("KERNEL_PANIC")
            || text.contains("INIT FAIL")
        {
            return Err("desktop guest failed; inspect serial".into());
        }
        if Instant::now() > deadline {
            return Err(format!("desktop did not report {needle}"));
        }
        thread::sleep(Duration::from_millis(50));
    }
}
fn shutdown(
    root: &std::path::Path,
    record: &Value,
    client: &mut qmp::Client,
    pointer: &mut (i32, i32),
) -> Result<(), String> {
    // Target the actual trusted Start footer. The former Shift+F10/Down
    // sequence now correctly opens Files Properties in the refined context menu.
    let _ = pointer;
    key(client, &["esc"])?;
    start_shut_down(client)?;
    wait_serial(
        root,
        record,
        "DOOR_P08 CLEANUP PASS",
        Duration::from_secs(15),
    )?;
    wait_serial(root, record, "reclaimed=8", Duration::from_secs(10))?;
    let text = serial(root, record)?;
    if !text.contains("leaked=0") || !text.contains("reclaimed=8") {
        return Err("desktop resource cleanup absent".into());
    }
    Ok(())
}
fn seal_serial(root: &std::path::Path, record: &mut Value) -> Result<(), String> {
    let path = paths::existing(root, record["serial"].as_str().ok_or("desktop serial")?)?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !shutdown::device_complete(&fs::read_to_string(&path).map_err(|e| e.to_string())?) {
        if Instant::now() >= deadline {
            return Err("complete final kernel DEVICE reclamation record missing".into());
        }
        thread::sleep(Duration::from_millis(50));
    }
    // The sealed log is retained in the owning run (out/ is cleaned) as
    // `serial_retained`; `serial_sha256` is the hash of that copy.
    crate::evidence::seal_session_serial(root, record)?;
    Ok(())
}
fn audit(disk: &std::path::Path, expected: &[u8]) -> Result<Value, String> {
    let bytes = fs::read(disk).map_err(|e| e.to_string())?;
    let journal = crate::persistence::journal::inspect(&bytes)?;
    if journal.selected().committed {
        return Err("desktop shutdown left committed journal".into());
    }
    let tree = crate::persistence::tree::inspect(&journal.replayed_home(&bytes)?)?;
    let catalog = crate::persistence::catalog::decode(&tree)?.ok_or("desktop catalog missing")?;
    let paths = catalog.paths()?;
    let node = paths
        .get("/users/1001/notes/desktop.txt")
        .ok_or("desktop note missing from actual disk")?;
    let file = &tree.files[&format!("obj{:016x}", node.id)];
    if file.bytes != expected {
        return Err(format!(
            "desktop persisted bytes differ: expected {:?}, got {:?}",
            String::from_utf8_lossy(expected),
            String::from_utf8_lossy(&file.bytes)
        ));
    }
    Ok(
        json!({"path":"/users/1001/notes/desktop.txt","bytes":file.bytes.len(),"disk_sha256":file_hash(disk)?,"journal_clean":true,"created_folders":paths.iter().filter(|(p,n)|(p.starts_with("/users/1001/New folder ") || p.starts_with("/users/1001/notes/New folder ")) && n.kind==2 && n.owner==1001).map(|(p,_)|p.to_string()).collect::<Vec<_>>()}),
    )
}
fn require_folders(audit: &Value) -> Result<(), String> {
    let folders = audit["created_folders"]
        .as_array()
        .ok_or("desktop created directories")?;
    for name in [
        "/users/1001/New folder 1",
        "/users/1001/New folder 2",
        "/users/1001/New folder 3",
        "/users/1001/notes/New folder 1",
        "/users/1001/notes/New folder 2",
    ] {
        if !folders.iter().any(|f| f == name) {
            return Err(format!("Files action did not persist {name}"));
        }
    }
    if folders.len() != 5 {
        return Err("unexpected menu/directory action".into());
    }
    Ok(())
}
fn reconnected_both(text: &str) -> bool {
    let Some(line) = text.lines().find(|l| l.contains("DOOR_P11 CLIENTS")) else {
        return false;
    };
    ["files=", "notes="].iter().all(|prefix| {
        line.split_whitespace()
            .find_map(|word| word.strip_prefix(prefix))
            .and_then(|s| s.parse::<u64>().ok())
            .is_some_and(|pid| {
                text.contains(&format!("DOOR_USER pid={pid} DOOR_P11 APP RECONNECT"))
            })
    })
}
fn connection(record: &Value) -> Result<qmp::Client, String> {
    let mut c = qmp::Client::connect(
        u16::try_from(record["qmp_port"].as_u64().ok_or("desktop QMP port")?)
            .map_err(|e| e.to_string())?,
    )?;
    if c.execute("query-uuid", json!({}))?["UUID"] != record["uuid"] {
        return Err("desktop QMP identity mismatch".into());
    }
    Ok(c)
}

mod preview;
pub fn preview_test(run: &mut Run, image: &str) -> Result<(), String> {
    preview::verify(run, image)
}

mod monitor;
pub fn monitor_test(run: &mut Run, image: &str) -> Result<(), String> {
    monitor::verify(run, image)
}

mod multiwindow;
pub fn multiwindow_test(run: &mut Run, image: &str) -> Result<(), String> {
    multiwindow::verify(run, image)
}
pub fn multiwindow_pressure_test(run: &mut Run, image: &str) -> Result<(), String> {
    multiwindow::pressure(run, image)
}

mod gallery;
/// `cargo xtask test ui --suite component-gallery` (registered in
/// `scenarios::RUNNERS`).
pub fn component_gallery_suite(
    run: &mut Run,
    image: &str,
    test_image: &str,
) -> Result<Value, String> {
    gallery::suite(run, image, test_image)
}
mod accessibility;
/// `cargo xtask test ui --suite a11y-01` (registered in `scenarios::RUNNERS`).
pub fn a11y_01_suite(run: &mut Run, image: &str, test_image: &str) -> Result<Value, String> {
    accessibility::suite(run, image, test_image)
}
mod display_scale;
pub(crate) use display_scale::SCALE_TABS;
mod motion_timing;
mod text_editing;
/// `cargo xtask test ui --suite text-01` (registered in `scenarios::RUNNERS`).
pub fn text_01_suite(run: &mut Run, image: &str, test_image: &str) -> Result<Value, String> {
    text_editing::suite(run, image, test_image)
}
/// `cargo xtask test ui --suite display-01` (registered in `scenarios::RUNNERS`).
pub fn display_01_suite(run: &mut Run, image: &str, test_image: &str) -> Result<Value, String> {
    display_scale::suite(run, image, test_image)
}
/// `cargo xtask test ui --suite motion-01` (registered in `scenarios::RUNNERS`).
pub fn motion_01_suite(run: &mut Run, image: &str, test_image: &str) -> Result<Value, String> {
    motion_timing::suite(run, image, test_image)
}
mod win32;
/// `cargo xtask test compat --suite win32-gui-f1` (registered in `scenarios::RUNNERS`).
pub fn win32_gui_f1_suite(run: &mut Run, image: &str, test_image: &str) -> Result<Value, String> {
    win32::f1(run, image, test_image)
}
/// `cargo xtask test compat --suite win32-gui-f2` (registered in `scenarios::RUNNERS`).
pub fn win32_gui_f2_suite(run: &mut Run, image: &str, test_image: &str) -> Result<Value, String> {
    win32::f2(run, image, test_image)
}
/// `cargo xtask test compat --suite win32-gui-f3` (registered in `scenarios::RUNNERS`).
pub fn win32_gui_f3_suite(run: &mut Run, image: &str, test_image: &str) -> Result<Value, String> {
    win32::f3(run, image, test_image)
}
/// `cargo xtask test compat --suite scenario-141-5` (registered in `scenarios::RUNNERS`).
pub fn scenario_141_5_suite(run: &mut Run, image: &str, test_image: &str) -> Result<Value, String> {
    win32::scenario_141_5(run, image, test_image)
}
/// `cargo xtask test compat --suite scenario-141-6` (registered in `scenarios::RUNNERS`).
pub fn scenario_141_6_suite(run: &mut Run, image: &str, test_image: &str) -> Result<Value, String> {
    win32::scenario_141_6(run, image, test_image)
}
mod p14_security;
pub(crate) use p14_security::sign_in_keys;
/// `cargo xtask test desktop --suite sec-02-trusted` (registered in `scenarios::RUNNERS`).
pub fn sec_02_trusted_suite(run: &mut Run, image: &str, test_image: &str) -> Result<Value, String> {
    p14_security::sec_02_trusted_suite(run, image, test_image)
}
mod p14_recovery;
mod p14_shell;
/// `cargo xtask test desktop --suite desktop-a-shell` (registered in `scenarios::RUNNERS`).
pub fn desktop_a_shell_suite(
    run: &mut Run,
    image: &str,
    test_image: &str,
) -> Result<Value, String> {
    p14_shell::suite(run, image, test_image)
}
/// `cargo xtask test desktop --suite recover-01` (registered in `scenarios::RUNNERS`).
pub fn recover_01_suite(run: &mut Run, image: &str, test_image: &str) -> Result<Value, String> {
    p14_recovery::suite(run, image, test_image)
}
mod p14_apps;
mod p14_files;
/// `cargo xtask test desktop --suite desktop-a-files` (registered in `scenarios::RUNNERS`).
pub fn desktop_a_files_suite(
    run: &mut Run,
    image: &str,
    test_image: &str,
) -> Result<Value, String> {
    p14_files::suite(run, image, test_image)
}
/// `cargo xtask test desktop --suite desktop-a-apps` (registered in `scenarios::RUNNERS`).
pub fn desktop_a_apps_suite(run: &mut Run, image: &str, test_image: &str) -> Result<Value, String> {
    p14_apps::suite(run, image, test_image)
}
/// P15: network and sound shell surfaces, Settings pages and the captured
/// Test tone (`cargo xtask test p15-desktop --image <image.json>`).
pub fn p15_desktop_test(run: &mut Run, image: &str) -> Result<(), String> {
    p15_desktop::verify(run, image)
}
/// P15 CONNECT-A: a Winsock console program against host endpoints
/// (`cargo xtask test p15-connect --image <image.json>`).
pub fn p15_connect_test(run: &mut Run, image: &str) -> Result<(), String> {
    p15_connect::verify(run, image)
}
/// SOUND-A: soundprobe.exe through winmm.dll, its tones found in the wav
/// capture (`cargo xtask test p15-sound --image <image.json>`).
pub fn p15_sound_test(run: &mut Run, image: &str) -> Result<(), String> {
    p15_sound::verify(run, image)
}
/// MIC-A: soundprobe.exe recording through waveIn from QEMU's silent input,
/// the taskbar and Settings while it records, and the desktop-apps switch
/// refusing the next open (`cargo xtask test p15-capture --image <image.json>`).
/// DSOUND-A: dsprobe.exe through dsound.dll, its tones found in the wav
/// capture (`cargo xtask test p15-dsound --image <image.json>`).
pub fn p15_dsound_test(run: &mut Run, image: &str) -> Result<(), String> {
    p15_sound::verify_dsound(run, image)
}
/// MIDI-A: midiprobe.exe through winmm.dll's MIDI synthesizer, MIDI
/// streams, MCI and the mixer, its tones found in the wav capture
/// (`cargo xtask test p15-midi --image <image.json>`).
pub fn p15_midi_test(run: &mut Run, image: &str) -> Result<(), String> {
    p15_sound::verify_midi(run, image)
}
pub fn p15_capture_test(run: &mut Run, image: &str) -> Result<(), String> {
    p15_sound::verify_capture(run, image)
}
/// WASAPI-A: wasapiprobe.exe through MMDevApi.dll (Core Audio over COM)
/// (`cargo xtask test p15-wasapi --image <image.json>`).
pub fn p15_wasapi_test(run: &mut Run, image: &str) -> Result<(), String> {
    p15_sound::verify_wasapi(run, image)
}
/// VPN-A: WireGuard to a boringtun peer on this host, through Terminal
/// (`cargo xtask test p15-vpn --image <image.json>`).
pub fn p15_vpn_test(run: &mut Run, image: &str) -> Result<(), String> {
    p15_vpn::verify(run, image)
}
/// CONNECT-A's host endpoints alone, for a Windows reference run of the
/// probes on this PC (`cargo xtask serve p15-connect`).
pub fn p15_connect_serve(run: &mut Run) -> Result<(), String> {
    p15_connect::serve_reference(run)
}
/// P16: cmd.exe in Terminal (`cargo xtask test p16-cmd --image <image.json>`).
pub fn p16_cmd_test(run: &mut Run, image: &str) -> Result<(), String> {
    p16_cmd::verify(run, image)
}
/// P16 INSTALL-A: msiexec installs and removes a Windows Installer package
/// (`cargo xtask test p16-install --image <image.json>`).
pub fn p16_install_test(run: &mut Run, image: &str) -> Result<(), String> {
    p16_install::verify(run, image)
}
/// P16 ACCOUNTS-A: a second local account, its sign-in and a standard
/// user's credential prompt (`cargo xtask test p16-accounts --image
/// <image.json>`).
pub fn p16_accounts_test(run: &mut Run, image: &str) -> Result<(), String> {
    p16_accounts::verify(run, image)
}
/// P16 SCM-A: a Windows service through Init's Service Control Manager
/// (`cargo xtask test p16-scm --image <image.json>`).
pub fn p16_scm_test(run: &mut Run, image: &str) -> Result<(), String> {
    p16_scm::verify(run, image)
}
/// P16 SECURITY-A: advapi32's token and security functions with the
/// filtered and the full token (`cargo xtask test p16-security --image
/// <image.json>`).
pub fn p16_security_test(run: &mut Run, image: &str) -> Result<(), String> {
    p16_security::verify(run, image)
}
/// P16 COM-A: comprobe.exe through ole32.dll (`cargo xtask test p16-com
/// --image <image.json>`).
pub fn p16_com_test(run: &mut Run, image: &str) -> Result<(), String> {
    p16_com::verify(run, image)
}
/// P16 OLEAUT-A: oleautprobe.exe through oleaut32.dll (`cargo xtask test
/// p16-oleaut --image <image.json>`).
pub fn p16_oleaut_test(run: &mut Run, image: &str) -> Result<(), String> {
    p16_oleaut::verify(run, image)
}
/// TSCHED-A: tschedprobe.exe through taskschd.dll (the Task Scheduler 2.0
/// API) against Windows' output, then a task registered, run and deleted
/// (`cargo xtask test p16-tsched --image <image.json>`).
pub fn p16_tsched_test(run: &mut Run, image: &str) -> Result<(), String> {
    p16_tsched::verify(run, image)
}
/// P16 CRYPT-A: cryptprobe.exe through advapi32's CryptoAPI keys against
/// Windows' output, then a key container (`cargo xtask test p16-crypt
/// --image <image.json>`).
pub fn p16_crypt_test(run: &mut Run, image: &str) -> Result<(), String> {
    p16_crypt::verify(run, image)
}
/// P16 TRUST-A: trustprobe.exe through wintrust.dll (WinVerifyTrust and the
/// catalog store) against Windows' output, then a trusted root, CRL
/// revocation and the catalog store, elevated (`cargo xtask test p16-trust
/// --image <image.json>`).
pub fn p16_trust_test(run: &mut Run, image: &str) -> Result<(), String> {
    p16_trust::verify(run, image)
}
/// P16 PATHS-A: pathprobe.exe and paths up to MAX_PATH (`cargo xtask test
/// p16-paths --image <image.json>`).
pub fn p16_paths_test(run: &mut Run, image: &str) -> Result<(), String> {
    p16_paths::verify(run, image)
}
/// P18 MEM-A: memprobe.exe, sections, placeholders and copy on write
/// (`cargo xtask test p18-mem --image <image.json>`).
pub fn p18_mem_test(run: &mut Run, image: &str) -> Result<(), String> {
    p18_mem::verify(run, image)
}
/// P18 CRT-A: crtprobe.exe on Doors' Universal CRT (`cargo xtask test p18-crt
/// --image <image.json>`).
pub fn p18_crt_test(run: &mut Run, image: &str) -> Result<(), String> {
    p18_crt::verify(run, image)
}
/// P18 DOTNET-A: the .NET 10 runtime and an SDK-built program on a large
/// volume (`cargo xtask test p18-dotnet --image <image.json>`).
pub fn p18_dotnet_test(run: &mut Run, image: &str) -> Result<(), String> {
    p18_dotnet::verify(run, image)
}
/// P18 PWSH-A: PowerShell 7 running a script on a large volume
/// (`cargo xtask test p18-pwsh --image <image.json>`).
pub fn p18_pwsh_test(run: &mut Run, image: &str) -> Result<(), String> {
    p18_pwsh::verify(run, image)
}
/// P18 ELECTRON-A: an Electron app from a Terminal tab
/// (`cargo xtask test p18-electron --image <image.json>`).
pub fn p18_electron_test(run: &mut Run, image: &str) -> Result<(), String> {
    p18_electron::verify(run, image)
}
/// P18 PWSH-B: interactive PowerShell 7 in a Terminal tab opened from
/// Start search (`cargo xtask test p18-pwsh-b --image <image.json>`).
pub fn p18_pwsh_b_test(run: &mut Run, image: &str) -> Result<(), String> {
    p18_pwsh_interactive::verify(run, image)
}
/// P17 X86-A: the 32-bit x86probe.exe under WOW64 against Windows' output
/// (cargo xtask test p17-x86 --image <image.json>).
pub fn p17_x86_test(run: &mut Run, image: &str) -> Result<(), String> {
    p17_x86::verify(run, image)
}
/// P17 X86-C: WOW64's environment, folders, registry and file-system views
/// and a 32-bit installer's writes (cargo xtask test p17-wow64 --image
/// <image.json>).
pub fn p17_wow64_test(run: &mut Run, image: &str) -> Result<(), String> {
    p17_x86::verify_wow(run, image)
}
/// P16 COMOOP-A: out-of-process COM between comclient.exe and the
/// LocalServer32 comserver.exe (`cargo xtask test p16-comoop --image
/// <image.json>`).
pub fn p16_comoop_test(run: &mut Run, image: &str) -> Result<(), String> {
    p16_comoop::verify(run, image)
}
/// P16 SHELL-A: shellprobe.exe through shell32.dll and CreateProcess
/// (`cargo xtask test p16-shell --image <image.json>`).
pub fn p16_shell_test(run: &mut Run, image: &str) -> Result<(), String> {
    p16_shell::verify(run, image)
}
