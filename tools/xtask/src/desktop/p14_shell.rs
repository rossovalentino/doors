//! P14 DESKTOP-A shell suite (`cargo xtask test desktop --suite
//! desktop-a-shell`, spec §60 and DESKTOP-A): every shell control the suite
//! touches does something real, observed independently of the shell.
//!
//! On the Windows-fixture image variant plus a deliberately invalid
//! `/system/bin/broken.exe`, with real QMP keyboard and pointer input:
//! the registration database registers the installed Windows programs;
//! Start search launches `wingui-editor` through its registration (the
//! kernel's `DOOR_P09 MODULE` record carries the fixture bytes); the Win32
//! F2 window gets its own taskbar button (pixels in the button slot); the
//! button minimizes and restores the real window (pixels in its rectangle);
//! the hover preview opens; the jump list pins the program (the per-user
//! registration file on the final disk names it); a closed program keeps
//! its pinned launcher, which relaunches it; launching `broken` produces a
//! real notification (toast pixels, dismiss, unread badge, centre, Do not
//! disturb, Clear all). Screens are QEMU screendumps; no shell self-report
//! alone decides a check.
use super::*;
use crate::desktop::win32;
use door_window::taskbar::{Layout, Slot, TrayTarget};

/// Install-manifest additions of the shell suite image variants.
pub(super) const DIAGNOSTICS_CONF: &[u8] = b"shell-diagnostics-v1\n";
/// Not a Windows executable: Init admits the bytes ("MZ"), the kernel's PE
/// validation refuses them, and the shell must say so.
fn broken_exe() -> Vec<u8> {
    let mut bytes = b"MZ".to_vec();
    bytes.extend(std::iter::repeat_n(0x5a, 510));
    bytes
}
/// The Windows-fixture image plus `broken.exe` and, for RECOVER-01, the
/// shell diagnostics flag (the fault hook). Staged once per run and name.
pub(super) fn stage(
    run: &mut Run,
    image: &str,
    diagnostics: bool,
) -> Result<(win32::Staged, String), String> {
    let staged = win32::stage(run, image)?;
    let variant = if diagnostics {
        "desktop-shell-diagnostics"
    } else {
        "desktop-shell"
    };
    let cache = run.directory.join(format!("{variant}-stage.json"));
    if cache.is_file() {
        let record = crate::image::read_json(&cache)?;
        return Ok((
            staged,
            record["image"]
                .as_str()
                .ok_or("shell stage image")?
                .to_owned(),
        ));
    }
    let initfs = paths::existing(
        &run.root,
        staged.record["initfs"]
            .as_str()
            .ok_or("win32 stage initfs")?,
    )?;
    let production = fs::read(&initfs).map_err(|e| e.to_string())?;
    let archive =
        door_initfs::Archive::parse(&production).map_err(|e| format!("win32 initfs {e:?}"))?;
    let mut manifest = String::from_utf8(
        archive
            .find("system/install.manifest")
            .ok_or("win32 initfs lacks system/install.manifest")?
            .data
            .to_vec(),
    )
    .map_err(|e| e.to_string())?;
    if !manifest.ends_with('\n') {
        manifest.push('\n');
    }
    let broken = broken_exe();
    let mut files: Vec<(String, Vec<u8>)> = vec![("seed/broken.exe".into(), broken)];
    manifest.push_str("/system/bin/broken.exe seed/broken.exe\n");
    if diagnostics {
        files.push((
            "seed/shell-diagnostics.conf".into(),
            DIAGNOSTICS_CONF.to_vec(),
        ));
        manifest.push_str("/system/share/shell-diagnostics.conf seed/shell-diagnostics.conf\n");
    }
    files.push(("system/install.manifest".into(), manifest.into_bytes()));
    let edits: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    let target = run.generated_directory(&format!("out/builds/{}/{variant}", run.id))?;
    let out = target.join(format!("initfs-{variant}.tar"));
    fs::write(
        &out,
        crate::initfs::with_files(&production, &edits, &["system/install.manifest"])?,
    )
    .map_err(|e| e.to_string())?;
    let (base, _) = crate::image::validate_staged(&run.root, image)?;
    let staged_image = session::stage_desktop_variant(
        run,
        &base.join("esp/EFI/BOOT/BOOTX64.EFI"),
        &base.join("esp/Door/kernel.elf"),
        &out,
        variant,
    )?;
    let record = json!({"image":staged_image,"base_image":image,"initfs":relative(&run.root,&out)?,
        "initfs_sha256":file_hash(&out)?,"diagnostics":diagnostics,"broken_exe_bytes":512});
    write_json(&cache, &record)?;
    run.check(&format!("p14-{variant}-stage"), record);
    Ok((staged, staged_image))
}
/// Wait for `needle` in serial text after `offset`; returns the line.
pub(super) fn wait_line(
    run: &Run,
    record: &Value,
    offset: usize,
    needle: &str,
    seconds: u64,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let text = serial(&run.root, record)?;
        if let Some(line) = text
            .get(offset..)
            .unwrap_or("")
            .lines()
            .find(|l| l.contains(needle))
        {
            return Ok(line.to_owned());
        }
        if Instant::now() >= deadline {
            return Err(format!("serial record missing: {needle}"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}
/// The window server's display density must be 100% (coordinates below are
/// logical = physical).
pub(super) fn require_unit_scale(run: &Run, record: &Value) -> Result<(), String> {
    let line = wait_line(run, record, 0, "DOOR_P12 DENSITY ", 30)?;
    if !line.contains("percent=100") {
        return Err(format!("shell suites need 100% density: {line}"));
    }
    Ok(())
}
pub(super) fn layout() -> Layout {
    Layout::new(1920, 1080)
}
fn center(r: door_window::graphics::Rect) -> (i32, i32) {
    (r.x() + r.width() as i32 / 2, r.y() + r.height() as i32 / 2)
}
/// Pixels of the generic Windows-app icon's accent caption band inside a
/// taskbar slot (BGR in the screendump: blue dominant).
pub(super) fn icon_ink(p: &scene::Pixels, r: door_window::graphics::Rect) -> Result<usize, String> {
    let mut n = 0;
    for y in r.y().max(0)..r.bottom().min(p.height as i32) {
        for x in r.x().max(0)..r.right().min(p.width as i32) {
            let [red, green, blue] = p.pixel(x as usize, y as usize)?;
            n += usize::from(blue > 140 && red < 60 && green < 140);
        }
    }
    Ok(n)
}
/// The extra (registered) taskbar button after the six pinned catalog apps.
pub(super) fn extra_slot(extras: usize) -> door_window::graphics::Rect {
    layout().slot(Slot::App(6), 6 + extras)
}
/// Launch a registered program from Start search; returns its pid.
pub(super) fn search_launch(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    staged: &win32::Staged,
    name: &str,
) -> Result<u64, String> {
    let hash = staged.fixtures.get(name).ok_or("staged fixture")?.clone();
    let offset = serial(&run.root, record)?.len();
    key(c, &["meta_l"])?;
    thread::sleep(Duration::from_millis(400));
    type_line(c, name)?;
    wait_line(
        run,
        record,
        offset,
        &format!("DOOR_P14 SHELL LAUNCH exe:/system/bin/{name}.exe ok=1"),
        20,
    )?;
    win32::wait_launch(run, record, offset, &hash, name)
}
/// A whole-screen dump kept as evidence.
pub(super) fn shot(
    run: &mut Run,
    c: &mut qmp::Client,
    name: &str,
) -> Result<(scene::Pixels, Value), String> {
    let p = win32::screen(run, c, &format!("p14-{name}"))?;
    let kept = win32::keep(run, &p, &format!("p14-{name}"))?;
    Ok((p, kept))
}
/// Client area of the F2 editor window (inside the invisible frame, below
/// the caption and menu bar).
pub(super) const F2_CLIENT: [i32; 4] = [
    win32::F2_RECT[0] + 16,
    win32::F2_RECT[1] + 64,
    win32::F2_RECT[2] - 48,
    win32::F2_RECT[3] - 96,
];
pub(super) fn suite(run: &mut Run, image: &str, _test_image: &str) -> Result<Value, String> {
    let (staged, staged_image) = stage(run, image, false)?;
    let disk = win32::fresh_disk(run, "p14-shell")?;
    let mut record =
        session::launch_desktop_accelerated(run, &staged_image, &disk, false, win32::RESOLUTION)?;
    thread::sleep(Duration::from_secs(2));
    let result = (|| -> Result<Value, String> {
        let mut c = connection(&record)?;
        let mut p = (960, 540);
        require_unit_scale(run, &record)?;
        let registrations = wait_line(run, &record, 0, "DOOR_P14 SHELL REGISTRATIONS ", 60)?;
        wait_line(run, &record, 0, "DOOR_P14 SHELL READY ", 60)?;
        let executables = registrations
            .split_whitespace()
            .find_map(|f| f.strip_prefix("executables="))
            .and_then(|v| v.parse::<usize>().ok())
            .ok_or("registration count")?;
        if executables < staged.fixtures.len() + 1 {
            return Err(format!(
                "registration database lacks installed programs: {registrations}"
            ));
        }
        // 1. Start search launches the registered Win32 F2 editor.
        let (before, _) = shot(run, &mut c, "shell-before")?;
        let offset = serial(&run.root, &record)?.len();
        let editor = search_launch(run, &record, &mut c, &staged, "wingui-editor")?;
        wait_line(
            run,
            &record,
            offset,
            "DOOR_P14 SHELL TASKBAR groups=1 windows=1",
            30,
        )?;
        thread::sleep(Duration::from_secs(2));
        let (running, running_png) = shot(run, &mut c, "shell-f2-running")?;
        let slot = extra_slot(1);
        let ink = icon_ink(&running, slot)?;
        // Before the launch the centred bar held six buttons: the place a
        // seventh would take is one pitch right of the last one (the new
        // button's slot in the seven-button layout overlaps Monitor's old
        // place by half a button, so it cannot be the "before" probe).
        let last = layout().slot(Slot::App(5), 6);
        let pitch = last.x() - layout().slot(Slot::App(4), 6).x();
        let vacant = door_window::graphics::Rect::new(
            last.x() + pitch,
            last.y(),
            last.width(),
            last.height(),
        )
        .map_err(|_| "vacant slot")?;
        if ink < 20 || icon_ink(&before, vacant)? > 5 {
            return Err(format!(
                "Win32 F2 has no taskbar button of its own ({ink} icon pixels)"
            ));
        }
        // 2. The button minimizes and restores the real window.
        let (x, y) = center(slot);
        click(&mut c, &mut p, x, y)?;
        thread::sleep(Duration::from_millis(1500));
        let (minimized, minimized_png) = shot(run, &mut c, "shell-f2-minimized")?;
        let gone = win32::changed(&running, &minimized, F2_CLIENT)?;
        click(&mut c, &mut p, x, y)?;
        thread::sleep(Duration::from_millis(1500));
        let (restored, restored_png) = shot(run, &mut c, "shell-f2-restored")?;
        let back = win32::changed(&running, &restored, F2_CLIENT)?;
        let area = (F2_CLIENT[2] * F2_CLIENT[3]) as usize;
        if gone < area / 4 || back > area / 20 {
            return Err(format!(
                "taskbar button did not minimize/restore F2 (changed {gone}, then {back} of {area})"
            ));
        }
        // 3. Hover preview with the live thumbnail.
        let offset = serial(&run.root, &record)?.len();
        move_to(&mut c, &mut p, x, y - 2)?;
        wait_line(
            run,
            &record,
            offset,
            "DOOR_P14 SHELL PREVIEW slot=6 windows=1",
            10,
        )?;
        let (preview, preview_png) = shot(run, &mut c, "shell-f2-preview")?;
        let preview_rect = door_window::shell_view::preview_rect(layout(), slot, 1);
        let shown = win32::changed(
            &restored,
            &preview,
            [
                preview_rect.x(),
                preview_rect.y(),
                preview_rect.width() as i32,
                preview_rect.height() as i32,
            ],
        )?;
        if shown < 2000 {
            return Err("taskbar hover preview did not appear".into());
        }
        // 4. Jump list: Pin to taskbar (label, Pin, Close window).
        let offset = serial(&run.root, &record)?.len();
        move_to(&mut c, &mut p, x, y)?;
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":true,"button":"right"}}]}),
        )?;
        thread::sleep(Duration::from_millis(150));
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":false,"button":"right"}}]}),
        )?;
        thread::sleep(Duration::from_millis(600));
        let (jump, jump_png) = shot(run, &mut c, "shell-f2-jump-list")?;
        key(&mut c, &["down"])?;
        key(&mut c, &["down"])?;
        key(&mut c, &["ret"])?;
        wait_line(
            run,
            &record,
            offset,
            "DOOR_P14 SHELL PINS taskbar=1 start=0 saved=1",
            15,
        )?;
        let _ = jump;
        // 5. Closing the program keeps its pinned launcher...
        let offset = serial(&run.root, &record)?.len();
        let (fx, fy) = (F2_CLIENT[0] + 40, F2_CLIENT[1] + 40);
        click(&mut c, &mut p, fx, fy)?;
        key(&mut c, &["alt", "f4"])?;
        wait_exit(run, &record, editor, offset)?;
        wait_line(
            run,
            &record,
            offset,
            "DOOR_P14 SHELL TASKBAR groups=1 windows=0",
            15,
        )?;
        thread::sleep(Duration::from_secs(1));
        let (pinned, pinned_png) = shot(run, &mut c, "shell-f2-pinned")?;
        if icon_ink(&pinned, slot)? < 20 {
            return Err("pinned launcher button is not drawn after the program exits".into());
        }
        // ...which relaunches it.
        let hash = staged.fixtures["wingui-editor"].clone();
        let offset = serial(&run.root, &record)?.len();
        click(&mut c, &mut p, x, y)?;
        let again = win32::wait_launch(run, &record, offset, &hash, "pinned relaunch")?;
        wait_line(
            run,
            &record,
            offset,
            "DOOR_P14 SHELL TASKBAR groups=1 windows=1",
            30,
        )?;
        thread::sleep(Duration::from_secs(2));
        let offset = serial(&run.root, &record)?.len();
        click(&mut c, &mut p, fx, fy)?;
        key(&mut c, &["alt", "f4"])?;
        wait_exit(run, &record, again, offset)?;
        // 6. A refused launch is a real notification.
        let notices = notification_flow(run, &record, &mut c, &mut p)?;
        win32::finish(run, &mut record, &mut c)?;
        Ok(json!({
            "registrations": registrations,
            "editor_pid": editor,
            "relaunched_pid": again,
            "taskbar_button_icon_pixels": ink,
            "minimize_changed": gone,
            "restore_residual": back,
            "preview_changed": shown,
            "screens": {"running":running_png,"minimized":minimized_png,"restored":restored_png,
                "preview":preview_png,"jump_list":jump_png,"pinned":pinned_png},
            "notifications": notices,
        }))
    })();
    let mut evidence = win32::quit_on_error(run, &record, result, "p14-shell-failure")?;
    // The per-user registration file on the final disk names the pin.
    let files = disk_files(&disk)?;
    let pins = files
        .get(&format!(
            "{}/AppData/Local/Door/shell-registrations.db",
            win32::HOME
        ))
        .ok_or("per-user registration file absent")?;
    let text = String::from_utf8_lossy(pins);
    if !text.contains("pin-taskbar exe:/system/bin/wingui-editor.exe") {
        return Err(format!("persisted pins differ: {text:?}"));
    }
    evidence["persisted_pins"] = json!(text);
    evidence["session"] = record;
    evidence["reference_comparison"] = json!({"state":"not_applicable",
        "scope":"Interaction and real-state checks; visual fidelity is VIS-01"});
    Ok(evidence)
}
/// Every file of the final disk through journal replay and the catalog.
pub(super) fn disk_files(
    disk: &std::path::Path,
) -> Result<std::collections::BTreeMap<String, Vec<u8>>, String> {
    let raw = fs::read(disk).map_err(|e| e.to_string())?;
    let journal = crate::persistence::journal::inspect(&raw)?;
    let tree = crate::persistence::tree::inspect(&journal.replayed_home(&raw)?)?;
    let catalog = crate::persistence::catalog::decode(&tree)?.ok_or("catalog absent")?;
    let mut out = std::collections::BTreeMap::new();
    for (path, node) in catalog.paths()? {
        if let Some(file) = tree.files.get(&format!("obj{:016x}", node.id)) {
            out.insert(path.clone(), file.bytes.clone());
        }
    }
    Ok(out)
}
/// The kernel's retirement record of `pid` after `offset`.
pub(super) fn wait_exit(run: &Run, record: &Value, pid: u64, offset: usize) -> Result<i64, String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    // The kernel's retirement line names the pid itself; the process was
    // launched (its module lines printed) before `offset`.
    let needle = format!(" pid={pid} ");
    loop {
        let text = serial(&run.root, record)?;
        if let Some(exit) = text
            .get(offset.min(text.len())..)
            .unwrap_or("")
            .lines()
            .filter_map(|l| l.split_once("DOOR_P08 PROCESS ").map(|(_, rest)| rest))
            .find(|rest| format!(" {rest}").contains(&needle))
            .and_then(|rest| {
                rest.split_whitespace()
                    .find_map(|f| f.strip_prefix("exit="))
            })
            .and_then(|e| e.parse().ok())
        {
            return Ok(exit);
        }
        if Instant::now() >= deadline {
            return Err(format!("process {pid} did not exit"));
        }
        thread::sleep(Duration::from_millis(100));
    }
}
fn notice_state(line: &str) -> String {
    line.split_once("DOOR_P14 SHELL NOTICES ")
        .map_or(line, |(_, r)| r)
        .to_owned()
}
fn notification_flow(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    p: &mut (i32, i32),
) -> Result<Value, String> {
    use door_window::shell_link::{Notice, Notices};
    let l = layout();
    let one = Notices {
        list: vec![Notice {
            id: 1,
            icon: 0,
            toast: true,
            source: String::new(),
            time: String::new(),
            title: String::new(),
            body: String::new(),
            actions: Vec::new(),
        }],
        dnd: false,
        badge: 1,
    };
    let (quiet, _) = shot(run, c, "notify-before")?;
    let offset = serial(&run.root, record)?.len();
    key(c, &["meta_l"])?;
    thread::sleep(Duration::from_millis(400));
    type_line(c, "broken")?;
    wait_line(
        run,
        record,
        offset,
        "DOOR_P14 SHELL LAUNCH exe:/system/bin/broken.exe ok=0",
        20,
    )?;
    let posted = wait_line(
        run,
        record,
        offset,
        "DOOR_P14 SHELL NOTICES count=1 toasts=1 unread=1 dnd=0",
        10,
    )?;
    thread::sleep(Duration::from_millis(700));
    let (toast, toast_png) = shot(run, c, "notify-toast")?;
    let (index, toast_rect) = door_window::shell_view::toasts(l, &one)
        .first()
        .copied()
        .ok_or("toast geometry")?;
    let _ = index;
    let rect = [
        toast_rect.x(),
        toast_rect.y(),
        toast_rect.width() as i32,
        toast_rect.height() as i32,
    ];
    let drawn = win32::changed(&quiet, &toast, rect)?;
    if drawn < (rect[2] * rect[3]) as usize / 3 {
        return Err(format!("toast not drawn bottom-right ({drawn} pixels)"));
    }
    // Dismiss with its close button: gone from the screen, kept unread.
    let card = door_window::shell_view::card(toast_rect, &one.list[0]);
    let offset = serial(&run.root, record)?.len();
    let (x, y) = center(card.close);
    click(c, p, x, y)?;
    let dismissed = wait_line(
        run,
        record,
        offset,
        "DOOR_P14 SHELL NOTICES count=1 toasts=0 unread=1",
        10,
    )?;
    thread::sleep(Duration::from_millis(600));
    let (badged, badge_png) = shot(run, c, "notify-badge")?;
    let clock = l.tray(TrayTarget::Clock);
    let bell = door_window::graphics::Rect::new(clock.right() - 22, clock.y(), 22, clock.height())
        .map_err(|_| "bell rect")?;
    let badge = win32::changed(
        &quiet,
        &badged,
        [
            bell.x(),
            bell.y(),
            bell.width() as i32,
            bell.height() as i32,
        ],
    )?;
    if badge < 20 {
        return Err("unread badge not drawn on the bell".into());
    }
    // Open the centre: the badge clears; Do not disturb; Clear all.
    let offset = serial(&run.root, record)?.len();
    let (x, y) = center(clock);
    click(c, p, x, y)?;
    let seen = wait_line(
        run,
        record,
        offset,
        "DOOR_P14 SHELL NOTICES count=1 toasts=0 unread=0 dnd=0",
        10,
    )?;
    thread::sleep(Duration::from_millis(700));
    let (centre_shot, centre_png) = shot(run, c, "notify-centre")?;
    let read = Notices {
        badge: 0,
        list: vec![Notice {
            toast: false,
            ..one.list[0].clone()
        }],
        ..one.clone()
    };
    let centre = door_window::shell_view::centre(l, &read).ok_or("centre geometry")?;
    let _ = centre_shot;
    let offset = serial(&run.root, record)?.len();
    let (x, y) = center(centre.dnd);
    click(c, p, x, y)?;
    let dnd = wait_line(run, record, offset, "dnd=1", 10)?;
    let (x, y) = center(centre.clear_all.ok_or("Clear all")?);
    let offset = serial(&run.root, record)?.len();
    click(c, p, x, y)?;
    let cleared = wait_line(run, record, offset, "DOOR_P14 SHELL NOTICES count=0", 10)?;
    // A second refusal under Do not disturb: listed, never a popup.
    key(c, &["esc"])?;
    let offset = serial(&run.root, record)?.len();
    key(c, &["meta_l"])?;
    thread::sleep(Duration::from_millis(400));
    type_line(c, "broken")?;
    let quiet_post = wait_line(
        run,
        record,
        offset,
        "DOOR_P14 SHELL NOTICES count=1 toasts=0 unread=1 dnd=1",
        15,
    )?;
    Ok(json!({
        "posted": notice_state(&posted),
        "dismissed": notice_state(&dismissed),
        "seen": notice_state(&seen),
        "dnd": notice_state(&dnd),
        "cleared": notice_state(&cleared),
        "dnd_post": notice_state(&quiet_post),
        "toast_pixels": drawn,
        "badge_pixels": badge,
        "screens": {"toast":toast_png,"badge":badge_png,"centre":centre_png},
    }))
}
