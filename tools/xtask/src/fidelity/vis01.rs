//! VIS-01 (`cargo xtask test desktop --suite vis-01`, acceptance :4277): the
//! frozen shell surface set compared with the Windows 11 26H2 reference
//! profile through the five-dimension harness.
//!
//! Controlled inputs first, through the real Settings UI only (no hidden
//! switches): dark mode (Personalization > Colors) and the reference 150 %
//! display scale (System > Display) on a 2560x1600 output, the reference
//! extent. Then settled screendumps of Settings > Colors, the desktop and
//! taskbar, Start (with a timestamped opening sequence for motion), File
//! Explorer and Notes maximized, and finally the lock screen. The sign-in and
//! lock captures are kept as unmatched evidence: the reference cannot capture
//! the secure desktop (profile surface `lock-logon`, excluded).
//!
//! The suite fails unless every dimension passes; a dimension with an
//! unresolved probe (such as Start motion before a reference sequence exists)
//! fails closed. The comparison is always recorded before the outcome.
use super::{Capture, Image, Sequence};
use crate::{
    evidence::{Run, file_hash, relative, write_json},
    paths, qmp, session,
};
use serde_json::{Value, json};
use std::{
    fs, thread,
    time::{Duration, Instant},
};

const RESOLUTION: &str = "2560x1600";
const OUTPUT: (usize, usize) = (2560, 1600);
/// The reference display scale (`profiles/windows-reference`, displays[0]).
const REFERENCE_SCALE: u32 = 150;
/// Settings > System > Display (`desktop/display_scale.rs` DISPLAY_PAGE).
const DISPLAY_PAGE: usize = 13;
/// Start's pinned row order: File Explorer, Notes, Settings.
const EXPLORER: (usize, usize) = (0, 1);
const NOTES: (usize, usize) = (1, 2);
const SETTINGS: (usize, usize) = (2, 3);
const WAIT: Duration = Duration::from_secs(25);
/// Motion sampling window after the Start key.
const MOTION_WINDOW: Duration = Duration::from_millis(1500);
const MOTION_MAX_FRAMES: usize = 24;

pub fn vis_01_suite(run: &mut Run, image: &str, _test_image: &str) -> Result<Value, String> {
    let profile_path = paths::existing(&run.root, super::PROFILE)?;
    let profile = crate::image::read_json(&profile_path)?;
    let profile_sha256 = file_hash(&profile_path)?;
    // A malformed or tampered profile fails before any guest boot.
    super::compare(&run.root, &profile, &[], &[])?;
    let disk = run.directory.join("vis01-private.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut record = session::launch_desktop_accelerated(run, image, &disk, false, RESOLUTION)?;
    let driven = drive(run, &record);
    if let Ok(mut c) = connection(&record) {
        if driven.is_err() {
            let failure = run.directory.join("vis01-failure.png");
            let _ = c.execute(
                "screendump",
                json!({"filename":qmp_path(&failure),"format":"png"}),
            );
        }
        let _ = c.execute("quit", json!({}));
    }
    let (captures, sequences, guest) = driven?;
    crate::evidence::seal_session_serial(&run.root, &mut record)?;
    let report = super::compare(&run.root, &profile, &captures, &sequences)?;
    let path = run.directory.join("vis01-comparison.json");
    write_json(&path, &report)?;
    let summary = super::summary(&report, &profile_sha256);
    run.check(
        "vis01-reference-comparison",
        json!({"report":relative(&run.root,&path)?,"sha256":file_hash(&path)?,"summary":summary}),
    );
    if report["outcome"] != "passed" {
        let failed: Vec<String> = super::Dimension::ALL
            .iter()
            .filter(|d| report["dimensions"][d.name()]["outcome"] != "passed")
            .map(|d| format!("{} {}", d.name(), report["dimensions"][d.name()]["outcome"]))
            .collect();
        return Err(format!(
            "VIS-01 failed against {} ({}); deviations {}; see {}",
            super::PROFILE,
            failed.join(", "),
            report["deviations"],
            relative(&run.root, &path)?
        ));
    }
    Ok(
        json!({"scenario":"vis-01","profile":super::PROFILE,"profile_sha256":profile_sha256,
        "session":record,"guest":guest,"comparison":relative(&run.root,&path)?,
        "reference_comparison":summary}),
    )
}

type Driven = (Vec<Capture>, Vec<Sequence>, Value);

fn drive(run: &mut Run, record: &Value) -> Result<Driven, String> {
    let root = run.root.clone();
    let mut c = connection(record)?;
    thread::sleep(Duration::from_secs(2));
    let density = serial(&root, record)?
        .lines()
        .find(|l| l.contains("DOOR_P12 DENSITY "))
        .and_then(|l| field(l, "percent="))
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or("VIS-01: DOOR_P12 DENSITY record missing")?;
    // Controlled inputs through Settings: dark mode, then the reference scale.
    activate(&root, record, &mut c, SETTINGS)?;
    key(&mut c, &["meta_l", "up"])?;
    let offset = serial(&root, record)?.len();
    page(&root, record, &mut c, "colors", None)?;
    key(&mut c, &["tab"])?;
    key(&mut c, &["down"])?;
    after(&root, record, offset, "SETTINGS SAVED theme=1")?;
    let mut scale = density;
    if density != REFERENCE_SCALE {
        page(&root, record, &mut c, "display", Some(DISPLAY_PAGE))?;
        for _ in 0..crate::desktop::SCALE_TABS {
            key(&mut c, &["tab"])?;
        }
        // The offered list runs 100..=200 in 25 % steps; Up moves one step.
        while scale > REFERENCE_SCALE {
            let offset = serial(&root, record)?.len();
            key(&mut c, &["up"])?;
            let next = scale - 25;
            after(
                &root,
                record,
                offset,
                &format!("DISPLAY_SCALE preference={next} effective={next}"),
            )?;
            scale = next;
        }
        if scale != REFERENCE_SCALE {
            return Err(format!(
                "VIS-01: could not reach the reference {REFERENCE_SCALE}% scale from {density}%"
            ));
        }
        thread::sleep(Duration::from_millis(1200));
    }
    page(&root, record, &mut c, "colors", None)?;
    let mut captures = vec![capture(run, &mut c, "settings", scale)?];
    // Desktop and taskbar with nothing open.
    close_foreground(&root, record, &mut c)?;
    captures.push(capture(run, &mut c, "desktop", scale)?);
    // Start: timestamped opening sequence, then the settled surface.
    let sequence = motion(run, &mut c, "start-open", scale)?;
    captures.push(capture(run, &mut c, "start", scale)?);
    key(&mut c, &["esc"])?;
    thread::sleep(Duration::from_millis(600));
    for (surface, app) in [("explorer", EXPLORER), ("notes", NOTES)] {
        activate(&root, record, &mut c, app)?;
        key(&mut c, &["meta_l", "up"])?;
        thread::sleep(Duration::from_millis(800));
        captures.push(capture(run, &mut c, surface, scale)?);
        close_foreground(&root, record, &mut c)?;
    }
    // Lock screen last (unmatched evidence: the reference cannot capture it).
    let offset = serial(&root, record)?.len();
    key(&mut c, &["meta_l", "l"])?;
    after(&root, record, offset, "DOOR_P12 LOCKED")?;
    thread::sleep(Duration::from_millis(800));
    let lock = capture(run, &mut c, "lock", scale)?;
    let mut unmatched = vec![
        json!({"surface":"lock","capture":lock.path,"sha256":lock.sha256,"scale_percent":scale}),
    ];
    if let Some(signin) = record["signin_screenshot"].as_str() {
        let path = paths::existing(&root, signin)?;
        unmatched.push(
            json!({"surface":"logon","capture":signin,"sha256":file_hash(&path)?,
            "scale_percent":density,"note":"trusted sign-in screen before the scale change"}),
        );
    }
    let guest = json!({"density_at_sign_in":density,"scale_percent":scale,
        "controlled_inputs":{"theme":"dark (Settings > Personalization > Colors)","scale":format!("{scale}% (Settings > System > Display)"),"output":RESOLUTION,
            "accent":"guest default (no accent control): accent-coloured pixels are excluded from probes"},
        "unmatched_captures":unmatched,
        "captures":captures.iter().map(|c| json!({"surface":c.surface,"path":c.path,"sha256":c.sha256})).collect::<Vec<_>>(),
        "motion":[{"surface":sequence.surface,"frames":sequence.frames.len(),"sample_interval_ms":sequence.sample_interval_ms}]});
    Ok((captures, vec![sequence], guest))
}

fn qmp_path(path: &std::path::Path) -> String {
    path.to_string_lossy()
        .trim_start_matches("\\\\?\\")
        .replace('\\', "/")
}
fn connection(record: &Value) -> Result<qmp::Client, String> {
    let port = u16::try_from(record["qmp_port"].as_u64().ok_or("desktop QMP port")?)
        .map_err(|e| e.to_string())?;
    let mut c = qmp::Client::connect(port)?;
    if c.execute("query-uuid", json!({}))?["UUID"] != record["uuid"] {
        return Err("desktop QMP identity mismatch".into());
    }
    Ok(c)
}
fn serial(root: &std::path::Path, record: &Value) -> Result<String, String> {
    fs::read_to_string(paths::existing(
        root,
        record["serial"].as_str().ok_or("desktop serial")?,
    )?)
    .map_err(|e| e.to_string())
}
fn field<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    line.split_whitespace().find_map(|w| w.strip_prefix(name))
}
fn key(c: &mut qmp::Client, keys: &[&str]) -> Result<(), String> {
    c.execute("send-key",json!({"keys":keys.iter().map(|k|json!({"type":"qcode","data":k})).collect::<Vec<_>>(),"hold-time":25}))?;
    thread::sleep(Duration::from_millis(200));
    Ok(())
}
fn type_line(c: &mut qmp::Client, text: &str) -> Result<(), String> {
    for ch in text.chars().chain(core::iter::once('\n')) {
        c.execute(
            "send-key",
            json!({"keys":session::keys(ch)?,"hold-time":20}),
        )?;
        thread::sleep(Duration::from_millis(70));
    }
    Ok(())
}
/// Waits for `marker` in the serial after byte `offset`.
fn after(
    root: &std::path::Path,
    record: &Value,
    offset: usize,
    marker: &str,
) -> Result<(), String> {
    let deadline = Instant::now() + WAIT;
    loop {
        let text = serial(root, record)?;
        if text.get(offset..).unwrap_or("").contains(marker) {
            return Ok(());
        }
        if text.contains("KERNEL_PANIC")
            || text.contains("INIT FAIL")
            || text.contains("APP FAILED")
        {
            return Err(format!(
                "VIS-01: guest failed while waiting for {marker}; inspect serial"
            ));
        }
        if Instant::now() >= deadline {
            return Err(format!("VIS-01: {marker} missing"));
        }
        thread::sleep(Duration::from_millis(80));
    }
}
/// Start pinned row: Right enters the row, each further Right walks it.
fn activate(
    root: &std::path::Path,
    record: &Value,
    c: &mut qmp::Client,
    (index, app): (usize, usize),
) -> Result<(), String> {
    let offset = serial(root, record)?.len();
    key(c, &["meta_l"])?;
    for _ in 0..=index {
        key(c, &["right"])?;
    }
    key(c, &["ret"])?;
    after(root, record, offset, &format!("APP_ACTIVATED app={app}"))
}
/// Settings search to a page; `id` waits for that page's record.
fn page(
    root: &std::path::Path,
    record: &Value,
    c: &mut qmp::Client,
    query: &str,
    id: Option<usize>,
) -> Result<(), String> {
    let offset = serial(root, record)?.len();
    key(c, &["esc"])?;
    key(c, &["ctrl", "f"])?;
    type_line(c, query)?;
    key(c, &["esc"])?;
    match id {
        Some(id) => after(root, record, offset, &format!("SETTINGS PAGE id={id}")),
        None => after(root, record, offset, "SETTINGS PAGE id="),
    }
}
/// Alt+F4 on the foreground window; waits for the client to leave.
fn close_foreground(
    root: &std::path::Path,
    record: &Value,
    c: &mut qmp::Client,
) -> Result<(), String> {
    let offset = serial(root, record)?.len();
    key(c, &["alt", "f4"])?;
    after(root, record, offset, "CLIENT_EXIT")?;
    thread::sleep(Duration::from_millis(600));
    Ok(())
}
fn screendump(c: &mut qmp::Client, path: &std::path::Path) -> Result<Image, String> {
    c.execute(
        "screendump",
        json!({"filename":qmp_path(path),"format":"ppm"}),
    )?;
    let image = Image::read(path)?;
    if (image.width, image.height) != OUTPUT {
        return Err(format!(
            "VIS-01: scanout {}x{}, expected {RESOLUTION}",
            image.width, image.height
        ));
    }
    Ok(image)
}
fn save_png(image: &Image, path: &std::path::Path) -> Result<(), String> {
    let mut png = resvg::tiny_skia::Pixmap::new(image.width as u32, image.height as u32)
        .ok_or("capture PNG allocation")?;
    for (rgb, rgba) in image
        .rgb
        .chunks_exact(3)
        .zip(png.data_mut().chunks_exact_mut(4))
    {
        rgba[..3].copy_from_slice(rgb);
        rgba[3] = 255;
    }
    png.save_png(path).map_err(|e| e.to_string())
}
/// Four identical whole-frame samples (150 ms apart, 10 s deadline), as the
/// desktop scenes require; the PPM is the compared evidence, the PNG a view.
fn capture(
    run: &mut Run,
    c: &mut qmp::Client,
    surface: &str,
    scale: u32,
) -> Result<Capture, String> {
    let begin = Instant::now();
    let ppm = run.directory.join(format!("vis01-{surface}.ppm"));
    let mut previous: Option<Image> = None;
    let mut identical = 0;
    let image = loop {
        let image = screendump(c, &ppm)?;
        identical = if previous.as_ref() == Some(&image) {
            identical + 1
        } else {
            0
        };
        if identical >= 3 {
            break image;
        }
        if begin.elapsed() > Duration::from_secs(10) {
            return Err(format!("VIS-01 {surface}: whole-frame stability deadline"));
        }
        previous = Some(image);
        thread::sleep(Duration::from_millis(150));
    };
    let png = ppm.with_extension("png");
    save_png(&image, &png)?;
    let path = relative(&run.root, &ppm)?;
    let sha256 = file_hash(&ppm)?;
    run.check(
        &format!("vis01-capture-{surface}"),
        json!({"ppm":path,"ppm_sha256":sha256,"screenshot":relative(&run.root,&png)?,"sha256":file_hash(&png)?,
            "scale_percent":scale,"identical_whole_frame_samples":identical + 1,"settle_ms":begin.elapsed().as_millis()}),
    );
    Ok(Capture {
        surface: surface.into(),
        image,
        scale_percent: scale,
        path,
        sha256,
    })
}
/// Timestamped screendumps after a Start key press (ms since the press). QMP
/// screendump latency bounds the resolution; it is recorded, never hidden.
fn motion(
    run: &mut Run,
    c: &mut qmp::Client,
    surface: &str,
    scale: u32,
) -> Result<Sequence, String> {
    let ppm = run.directory.join(format!("vis01-{surface}.ppm"));
    c.execute(
        "send-key",
        json!({"keys":[{"type":"qcode","data":"meta_l"}],"hold-time":25}),
    )?;
    let trigger = Instant::now();
    let mut frames = Vec::new();
    while trigger.elapsed() < MOTION_WINDOW && frames.len() < MOTION_MAX_FRAMES {
        let image = screendump(c, &ppm)?;
        frames.push((trigger.elapsed().as_secs_f64() * 1000.0, image));
    }
    let interval = frames.windows(2).map(|w| w[1].0 - w[0].0).sum::<f64>()
        / frames.len().saturating_sub(1).max(1) as f64;
    run.check(
        &format!("vis01-motion-{surface}"),
        json!({"frames":frames.len(),"timestamps_ms":frames.iter().map(|f| (f.0 * 10.0).round() / 10.0).collect::<Vec<_>>(),
            "sample_interval_ms":(interval * 10.0).round() / 10.0,"scale_percent":scale,
            "method":"host-timed QMP screendumps after the QMP key event; not input-to-photon latency"}),
    );
    Ok(Sequence {
        surface: surface.into(),
        frames,
        scale_percent: scale,
        sample_interval_ms: interval,
    })
}
