//! Real QMP gestures and independent output-pixel checks at fixed profile geometry.
//! These assertions establish composition behavior, not Windows reference fidelity.
use super::*;
pub(super) struct Pixels {
    bytes: Vec<u8>,
    start: usize,
    pub(super) width: usize,
    pub(super) height: usize,
}
/// Expected probes per scene step, shared by `exercise` and `validate`.
type Probes = Vec<(usize, usize, [u8; 3])>;
fn scene_probes() -> Result<Vec<(&'static str, Probes)>, String> {
    use window_points::{MENU_FOCUS, NOTES_BAR, SCENE_WALLPAPER};
    let s = window_points::scene()?;
    let (rx, ry) = window_points::scene_resize_probe_pixels()?;
    let white = window_points::content();
    let at = |(x, y): (usize, usize), color| (x, y, color);
    let bar = at(s.bar, NOTES_BAR);
    let body = at(s.outside, white);
    Ok(vec![
        ("moved", vec![bar, at(s.exposed, s.exposed_color)]),
        ("resized", vec![bar, body]),
        (
            "maximized",
            vec![at(s.bar_max, NOTES_BAR), at(s.body_max, white)],
        ),
        ("restored", vec![bar, body]),
        ("minimized", vec![at(s.outside, SCENE_WALLPAPER)]),
        ("keyboard-restore", vec![bar]),
        ("closed", vec![at(s.outside, SCENE_WALLPAPER)]),
        ("text-cursor", vec![(740, 390, [0, 0, 0])]),
        ("files-context", vec![at(s.menu_new, MENU_FOCUS)]),
        ("context-dismissed", vec![at(s.menu_last, white)]),
        // Start lists one Recent item: the desktop.txt Notes saved.
        ("launcher", vec![start_probe_recent(1024, 768, 48, 1)?]),
        ("launcher-dismissed", vec![(670, 530, [255, 255, 255])]),
        ("resize-cursor", vec![(rx, ry - 1, [67, 67, 78])]),
        (
            "cursor-clipped",
            vec![(740, 390, [255, 255, 255]), (rx, ry, [255, 255, 255])],
        ),
        ("left-edge", vec![at(s.bar_left, NOTES_BAR), body]),
        ("left-edge-restored", vec![bar]),
        ("top-edge", vec![at(s.bar_top, NOTES_BAR), body]),
        ("top-edge-restored", vec![bar]),
        ("launcher-activate", vec![bar]),
    ])
}
/// Whether a captured pixel satisfies a probe. Notes' command bar is
/// composited over the wallpaper-derived Mica backdrop (26H2 Notepad), whose
/// tint depends on the window position, so `NOTES_BAR` matches within 8 per
/// channel of its opaque colour; white content differs by 12 and still fails.
/// Every other probe is exact.
/// `probe_matches` at screen point `point` of a frame of size `output`:
/// Mica probes (`mica::settings`) compare with the compositor's Mica there.
pub(super) fn probe_matches_at(
    expected: [u8; 3],
    got: [u8; 3],
    point: (usize, usize),
    output: (usize, usize),
) -> bool {
    super::mica::matches(expected, got, point, output)
        .unwrap_or_else(|| probe_matches(expected, got))
}
pub(super) fn probe_matches(expected: [u8; 3], got: [u8; 3]) -> bool {
    if expected == window_points::NOTES_BAR {
        expected.iter().zip(got).all(|(e, g)| e.abs_diff(g) <= 8)
    } else {
        expected == got
    }
}
fn probes(all: &[(&str, Probes)], name: &str) -> Result<Probes, String> {
    all.iter()
        .find(|(n, _)| *n == name)
        .map(|(_, p)| p.clone())
        .ok_or_else(|| format!("no scene probes for {name}"))
}
pub(super) fn validate(root: &std::path::Path, checks: &[Value]) -> Result<(), String> {
    for (name, expected) in scene_probes()? {
        let c = checks
            .iter()
            .find(|v| v["id"] == format!("desktop-{name}") && v["outcome"] == "passed")
            .ok_or("desktop gesture evidence")?;
        let detail = &c["details"];
        let ppm = paths::existing(root, detail["ppm"].as_str().ok_or("desktop ppm")?)?;
        if file_hash(&ppm)? != detail["ppm_sha256"]
            || file_hash(&paths::existing(
                root,
                detail["screenshot"].as_str().ok_or("desktop screenshot")?,
            )?)? != detail["sha256"]
        {
            return Err("desktop capture changed".into());
        }
        let pixels = Pixels::read(&ppm)?;
        for (x, y, color) in expected {
            let output = (pixels.width, pixels.height);
            if !probe_matches_at(color, pixels.pixel(x, y)?, (x, y), output) {
                return Err(format!("retained desktop {name} pixels differ"));
            }
        }
    }
    Ok(())
}
impl Pixels {
    pub(super) fn read(path: &std::path::Path) -> Result<Self, String> {
        Self::read_kind(path, false)
    }
    pub(super) fn read_host_preview(path: &std::path::Path) -> Result<Self, String> {
        Self::read_kind(path, true)
    }
    fn read_kind(path: &std::path::Path, host: bool) -> Result<Self, String> {
        let bytes = fs::read(path).map_err(|e| e.to_string())?;
        let mut offset = 0;
        let mut tokens = Vec::new();
        for _ in 0..4 {
            while bytes.get(offset).is_some_and(u8::is_ascii_whitespace) {
                offset += 1
            }
            let begin = offset;
            while bytes.get(offset).is_some_and(|b| !b.is_ascii_whitespace()) {
                offset += 1
            }
            tokens.push(core::str::from_utf8(&bytes[begin..offset]).map_err(|e| e.to_string())?);
        }
        if tokens[0] != "P6" || tokens[3] != "255" || bytes.get(offset) != Some(&b'\n') {
            return Err("unsupported QEMU PPM header".into());
        }
        let width = tokens[1].parse::<usize>().map_err(|e| e.to_string())?;
        let height = tokens[2].parse::<usize>().map_err(|e| e.to_string())?;
        offset += 1;
        let native_extent = matches!(
            (width, height),
            (1024, 768)
                | (1280, 720)
                | (1920, 1080)
                | (1920, 1200)
                | (2560, 1440)
                | (2560, 1600)
                | (3840, 2160)
        );
        if !(native_extent
            || host
                && matches!(
                    (width, height),
                    (1050, 650)
                        | (1472, 952)
                        | (1280, 800)
                        | (1460, 900)
                        | (1484, 996)
                        | (2048, 1224)
                ))
            || bytes.len() != offset + width * height * 3
        {
            return Err("wrong QEMU framebuffer extent".into());
        }
        Ok(Self {
            bytes,
            start: offset,
            width,
            height,
        })
    }
    pub(super) fn pixel(&self, x: usize, y: usize) -> Result<[u8; 3], String> {
        if x >= self.width || y >= self.height {
            return Err("pixel outside frame".into());
        }
        let i = self.start + (y * self.width + x) * 3;
        Ok(self.bytes[i..i + 3].try_into().unwrap())
    }
    pub(super) fn save_png(&self, path: &std::path::Path) -> Result<(), String> {
        // Encode the exact asserted screendump. A second QMP request can capture
        // another frame, so it cannot be paired with this PPM as visual evidence.
        let mut png = resvg::tiny_skia::Pixmap::new(self.width as u32, self.height as u32)
            .ok_or("screendump PNG allocation")?;
        for (rgb, rgba) in self.bytes[self.start..]
            .chunks_exact(3)
            .zip(png.data_mut().chunks_exact_mut(4))
        {
            rgba[..3].copy_from_slice(rgb);
            rgba[3] = 255;
        }
        png.save_png(path).map_err(|e| e.to_string())
    }
}
pub(super) fn capture(
    run: &mut Run,
    c: &mut qmp::Client,
    name: &str,
    expected: &[(usize, usize, [u8; 3])],
) -> Result<Pixels, String> {
    capture_impl(run, c, name, expected, false)
}
/// Visual study requires four identical whole-frame samples after the expected
/// pixels match. Sparse assertions can pass before lower GOP rows arrive.
pub(super) fn capture_settled(
    run: &mut Run,
    c: &mut qmp::Client,
    name: &str,
    expected: &[(usize, usize, [u8; 3])],
) -> Result<Pixels, String> {
    capture_impl(run, c, name, expected, true)
}
/// A frame that animates by design (WinUI's indeterminate ProgressBar and
/// ProgressRing keep moving while Animation effects are on): four
/// consecutive samples identical outside the bounding box of what changed
/// among them, that box no larger than `max_permille` of the frame. The box
/// is recorded with the capture.
pub(super) fn capture_settled_moving(
    run: &mut Run,
    c: &mut qmp::Client,
    name: &str,
    max_permille: usize,
) -> Result<Pixels, String> {
    let begin = Instant::now();
    let ppm = run.directory.join(format!("desktop-{name}.ppm"));
    let png = run.directory.join(format!("desktop-{name}.png"));
    let mut last: Option<Pixels> = None;
    // Change boxes of the latest consecutive sample pairs (at most three).
    let mut boxes: Vec<Option<[usize; 4]>> = Vec::new();
    let (pixels, moving) = loop {
        c.execute("screendump",json!({"filename":ppm.to_string_lossy().trim_start_matches("\\\\?\\").replace('\\',"/"),"format":"ppm"}))?;
        let pixels = Pixels::read(&ppm)?;
        if let Some(previous) = &last {
            if boxes.len() == 3 {
                boxes.remove(0);
            }
            boxes.push(change_box(previous, &pixels));
        }
        let moving = boxes
            .iter()
            .flatten()
            .fold(None, |acc: Option<[usize; 4]>, b| {
                Some(acc.map_or(*b, |a| {
                    [
                        a[0].min(b[0]),
                        a[1].min(b[1]),
                        a[2].max(b[2]),
                        a[3].max(b[3]),
                    ]
                }))
            });
        let area = moving.map_or(0, |[x0, y0, x1, y1]| (x1 - x0) * (y1 - y0));
        if boxes.len() == 3 && area * 1000 <= pixels.width * pixels.height * max_permille {
            break (pixels, moving);
        }
        if begin.elapsed() > Duration::from_secs(10) {
            return Err(format!(
                "{name}: stability deadline outside the moving region (last change box {moving:?})"
            ));
        }
        last = Some(pixels);
        thread::sleep(Duration::from_millis(150));
    };
    pixels.save_png(&png)?;
    run.check(&format!("desktop-{name}"),json!({"ppm":relative(&run.root,&ppm)?,"ppm_sha256":file_hash(&ppm)?,"screenshot":relative(&run.root,&png)?,"sha256":file_hash(&png)?,"identical_outside_moving_samples":4,"moving_box":moving,"moving_box_limit_permille":max_permille,"settle_ms":begin.elapsed().as_millis(),"timeout_ms":10000,"scope":"native composition; animated by design inside moving_box (x0,y0,x1,y1 exclusive); host-observed timing"}));
    Ok(pixels)
}
/// Bounding box `[x0, y0, x1, y1)` of the pixels that differ.
fn change_box(a: &Pixels, b: &Pixels) -> Option<[usize; 4]> {
    if (a.width, a.height) != (b.width, b.height) {
        return Some([0, 0, b.width, b.height]);
    }
    let rows = a.bytes[a.start..]
        .chunks_exact(a.width * 3)
        .zip(b.bytes[b.start..].chunks_exact(b.width * 3));
    let mut found: Option<[usize; 4]> = None;
    for (y, (ra, rb)) in rows.enumerate() {
        if ra == rb {
            continue;
        }
        let differs = |x: &usize| ra[x * 3..x * 3 + 3] != rb[x * 3..x * 3 + 3];
        let first = (0..a.width).find(differs).unwrap_or(0);
        let end = (0..a.width).rev().find(differs).map_or(a.width, |x| x + 1);
        found = Some(found.map_or([first, y, end, y + 1], |f| {
            [f[0].min(first), f[1], f[2].max(end), y + 1]
        }));
    }
    found
}
/// Whole-frame stillness a settled capture requires: 450 ms at 1920x1080,
/// scaled by output area (2560x1600 about 890 ms, 4K 1.8 s; at most 2 s).
fn settle_window(width: usize, height: usize) -> Duration {
    let area = (width * height) as u64;
    Duration::from_millis((450 * area / (1920 * 1080)).clamp(450, 2000))
}
fn capture_impl(
    run: &mut Run,
    c: &mut qmp::Client,
    name: &str,
    expected: &[(usize, usize, [u8; 3])],
    settled: bool,
) -> Result<Pixels, String> {
    let begin = Instant::now();
    let ppm = run.directory.join(format!("desktop-{name}.ppm"));
    let png = run.directory.join(format!("desktop-{name}.png"));
    let mut previous = Vec::new();
    let mut identical = 0;
    let mut still_since = begin;
    let mut still;
    let mut deadline = Duration::from_secs(10);
    let pixels = loop {
        c.execute("screendump",json!({"filename":ppm.to_string_lossy().trim_start_matches("\\\\?\\").replace('\\',"/"),"format":"ppm"}))?;
        let pixels = Pixels::read(&ppm)?;
        let output = (pixels.width, pixels.height);
        let matches = expected.iter().all(|&(x, y, color)| {
            pixels
                .pixel(x, y)
                .is_ok_and(|got| probe_matches_at(color, got, (x, y), output))
        });
        identical = if matches && previous == pixels.bytes {
            identical + 1
        } else {
            still_since = Instant::now();
            0
        };
        // One CPU-composed animation frame at 4K can outlast the base 450 ms
        // stillness window, so the window grows with the output area.
        still = settle_window(pixels.width, pixels.height);
        deadline = deadline.max(still * 6);
        let at_rest = identical >= 3 && still_since.elapsed() >= still;
        if (matches && (!settled || at_rest)) || begin.elapsed() > deadline {
            break pixels;
        }
        previous.clone_from(&pixels.bytes);
        thread::sleep(Duration::from_millis(if settled { 150 } else { 100 }));
    };
    pixels.save_png(&png)?;
    for &(x, y, color) in expected {
        let output = (pixels.width, pixels.height);
        if !probe_matches_at(color, pixels.pixel(x, y)?, (x, y), output) {
            return Err(format!(
                "{name}: pixel ({x},{y}) expected {color:?}, got {:?}",
                pixels.pixel(x, y)?
            ));
        }
    }
    if settled && (identical < 3 || still_since.elapsed() < still) {
        return Err(format!("{name}: whole-frame stability deadline"));
    }
    run.check(&format!("desktop-{name}"),json!({"ppm":relative(&run.root,&ppm)?,"ppm_sha256":file_hash(&ppm)?,"screenshot":relative(&run.root,&png)?,"sha256":file_hash(&png)?,"expected_pixels":expected,"identical_whole_frame_samples":if settled {identical+1} else {0},"settle_ms":begin.elapsed().as_millis(),"stillness_ms":still.as_millis(),"timeout_ms":deadline.as_millis(),"scope":"native composition; provisional appearance; host-observed timing; backend recorded in argv"}));
    Ok(pixels)
}
/// Reopen exactly paired RGB pixels and every declared native probe.
pub(super) fn validate_capture(root: &std::path::Path, detail: &Value) -> Result<Pixels, String> {
    let ppm = paths::existing(root, detail["ppm"].as_str().ok_or("native PPM path")?)?;
    let png = paths::existing(
        root,
        detail["screenshot"].as_str().ok_or("native PNG path")?,
    )?;
    if file_hash(&ppm)? != detail["ppm_sha256"]
        || file_hash(&png)? != detail["sha256"]
        || detail["identical_whole_frame_samples"]
            .as_u64()
            .is_none_or(|n| n < 4)
    {
        return Err("native capture identity or stillness".into());
    }
    let pixels = Pixels::read(&ppm)?;
    let image = resvg::tiny_skia::Pixmap::decode_png(&fs::read(png).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if (image.width() as usize, image.height() as usize) != (pixels.width, pixels.height)
        || image
            .data()
            .chunks_exact(4)
            .zip(pixels.bytes[pixels.start..].chunks_exact(3))
            .any(|(a, b)| a[..3] != *b || a[3] != 255)
    {
        return Err("native PNG/PPM pixels differ".into());
    }
    for probe in detail["expected_pixels"]
        .as_array()
        .ok_or("native probes")?
    {
        let (x, y) = (
            probe[0].as_u64().ok_or("probe x")? as usize,
            probe[1].as_u64().ok_or("probe y")? as usize,
        );
        let expected: [u8; 3] =
            serde_json::from_value(probe[2].clone()).map_err(|_| "probe colour")?;
        let output = (pixels.width, pixels.height);
        if !probe_matches_at(expected, pixels.pixel(x, y)?, (x, y), output) {
            return Err("retained native probe differs".into());
        }
    }
    Ok(pixels)
}
fn button(c: &mut qmp::Client, down: bool) -> Result<(), String> {
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"btn","data":{"down":down,"button":"left"}}]}),
    )?;
    thread::sleep(Duration::from_millis(100));
    Ok(())
}
pub(super) fn drag(
    c: &mut qmp::Client,
    p: &mut (i32, i32),
    from: (i32, i32),
    to: (i32, i32),
) -> Result<(), String> {
    move_to(c, p, from.0, from.1)?;
    button(c, true)?;
    move_to(c, p, to.0, to.1)?;
    button(c, false)
}
pub(super) fn exercise(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    p: &mut (i32, i32),
) -> Result<(), String> {
    // Holding a client press across Alt+Tab must revoke its capture, and a late
    // capture RPC must not kill the app. A real Files operation proves routing.
    let s = window_points::scene()?;
    let all = scene_probes()?;
    let step = |name| probes(&all, name);
    move_to(c, p, 740, 390)?;
    capture(run, c, "text-cursor", &step("text-cursor")?)?;
    let (px, py) = window_points::scene_resize_probe()?;
    move_to(c, p, px, py)?;
    capture(run, c, "resize-cursor", &step("resize-cursor")?)?;
    move_to(c, p, 0, 767)?;
    capture(run, c, "cursor-clipped", &step("cursor-clipped")?)?;
    move_to(c, p, 740, 390)?;
    button(c, true)?;
    key(c, &["alt", "tab"])?;
    button(c, false)?;
    // History shortcuts must reach distinct real directories, not just return
    // success. The raw audit below requires one new folder in each location.
    key(c, &["ctrl", "l"])?;
    type_line(c, "/users/1001/notes")?;
    key(c, &["alt", "left"])?;
    key(c, &["ctrl", "shift", "n"])?;
    key(c, &["alt", "right"])?;
    key(c, &["ctrl", "shift", "n"])?;
    key(c, &["backspace"])?;
    key(c, &["ctrl", "r"])?;
    key(c, &["ctrl", "f"])?;
    type_line(c, "zz-no-matching-file")?;
    key(c, &["ret"])?;
    key(c, &["esc"])?;
    // Later exact folder paths prove that Enter on zero results did not open
    // an invisible first entry or redirect the subsequent durable operations.
    // Files covers Notes exactly (one shared initial placement), so Notes'
    // taskbar button brings it back to the front.
    click(c, p, s.activate.0, s.activate.1)?;
    drag(c, p, s.moves[0], s.moves[1])?;
    capture(run, c, "moved", &step("moved")?)?;
    drag(c, p, s.resize[0], s.resize[1])?;
    capture(run, c, "resized", &step("resized")?)?;
    click(c, p, s.maximize.0, s.maximize.1)?;
    capture(run, c, "maximized", &step("maximized")?)?;
    click(c, p, s.restore.0, s.restore.1)?;
    capture(run, c, "restored", &step("restored")?)?;
    drag(c, p, s.left[0], s.left[1])?;
    capture(run, c, "left-edge", &step("left-edge")?)?;
    drag(c, p, s.left[1], s.left[0])?;
    capture(run, c, "left-edge-restored", &step("left-edge-restored")?)?;
    drag(c, p, s.top[0], s.top[1])?;
    capture(run, c, "top-edge", &step("top-edge")?)?;
    drag(c, p, s.top[1], s.top[0])?;
    capture(run, c, "top-edge-restored", &step("top-edge-restored")?)?;
    click(c, p, s.minimize.0, s.minimize.1)?;
    let hidden = capture(run, c, "minimized", &step("minimized")?)?;
    let visible = |frame: &Pixels| -> Result<bool, String> {
        Ok(probe_matches(
            window_points::NOTES_BAR,
            frame.pixel(s.bar.0, s.bar.1)?,
        ))
    };
    if visible(&hidden)? {
        return Err("minimized window still visible".into());
    }
    key(c, &["alt", "tab"])?;
    let restored = capture(run, c, "keyboard-switch", &[])?;
    if !visible(&restored)? {
        key(c, &["alt", "tab"])?;
    }
    capture(run, c, "keyboard-restore", &step("keyboard-restore")?)?;
    key(c, &["meta_l"])?;
    // An opaque pixel of the open Start search pill (acrylic body colors
    // depend on the wallpaper behind it).
    capture_settled(run, c, "launcher", &step("launcher")?)?;
    move_to(c, p, 650, 600)?;
    for down in [true, false] {
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":down,"button":"right"}}]}),
        )?;
        thread::sleep(Duration::from_millis(100));
    }
    key(c, &["esc"])?;
    capture(run, c, "launcher-dismissed", &step("launcher-dismissed")?)?;
    // Start focuses its search pill: Right enters Pinned, a second Right
    // reaches Notes.
    key(c, &["meta_l"])?;
    key(c, &["right"])?;
    key(c, &["right"])?;
    key(c, &["ret"])?;
    capture(run, c, "launcher-activate", &step("launcher-activate")?)?;
    click(c, p, 800, 500)?;
    key(c, &["alt", "f4"])?;
    wait_serial(
        &run.root,
        record,
        "DOOR_P11 CLIENT_EXIT",
        Duration::from_secs(10),
    )?;
    let text = serial(&run.root, record)?;
    if !text
        .lines()
        .any(|s| s.contains("DOOR_P11 CLIENT_EXIT") && s.ends_with("code=0"))
    {
        return Err("native close did not exit cleanly".into());
    }
    capture(run, c, "closed", &step("closed")?)?;
    click(c, p, s.files_menu.0, s.files_menu.1)?;
    key(c, &["ctrl", "shift", "n"])?;
    thread::sleep(Duration::from_millis(300));
    // Bare desktop clicks must dismiss and clear keyboard focus, so Enter
    // cannot activate a stale menu. The exact folder count detects that bug.
    for down in [true, false] {
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":down,"button":"right"}}]}),
        )?;
        thread::sleep(Duration::from_millis(200));
    }
    click(c, p, 950, 710)?;
    key(c, &["ret"])?;
    click(c, p, s.files_menu.0, s.files_menu.1)?;
    // Actual right-button menu activation creates a third durable directory.
    for down in [true, false] {
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":down,"button":"right"}}]}),
        )?;
        thread::sleep(Duration::from_millis(200));
    }
    // The menu opens unfocused, above the pointer (no room below it). Down
    // walks View, Sort by and Refresh to New (Paste is disabled with an empty
    // clipboard), Right opens New with Folder focused. Probe the visibly
    // painted focus fill of the New row.
    for k in ["down", "down", "down", "down", "right"] {
        key(c, &[k])?;
    }
    capture(run, c, "files-context", &step("files-context")?)?;
    key(c, &["ret"])?;
    key(c, &["shift", "f10"])?;
    key(c, &["esc"])?;
    capture(run, c, "context-dismissed", &step("context-dismissed")?)?;
    run.check("desktop-context-menu",json!({"action":"visibly painted right-click menu; Down to New, Right, then Enter on Folder creates New folder 3; Shift+F10/Escape dismisses"}));
    key(c, &["ctrl", "l"])?;
    type_line(c, "/users/1001/notes")?;
    key(c, &["ctrl", "shift", "n"])?;
    key(c, &["ctrl", "l"])?;
    type_line(c, "/users/1001")?;
    run.check("desktop-address-search",json!({"actions":"zero-result Enter, Escape clear, Ctrl+L replaces address, navigate to notes and create New folder 2; raw audit requires exact paths"}));
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launcher_probe_follows_start_with_its_recent_item() {
        // One Recent row grows Start upward, moving the search pill.
        let (x, y, color) = start_probe_recent(1024, 768, 48, 1).unwrap();
        let layout = door_window::taskbar::Layout::new(1024, 768);
        let pill = |recent| {
            door_window::start::Geometry::with_recent(
                layout,
                door_window::apps::COUNT,
                door_window::start::View::Category,
                false,
                recent,
            )
            .search_box()
        };
        assert!(pill(1).contains(x as i32, y as i32));
        assert!(!pill(0).contains(x as i32, y as i32));
        assert_eq!(start_probe(1024, 768, 48).unwrap(), (776, 90, color));
        assert_eq!((x, y, color), (776, 68, [251, 251, 251]));
    }
    #[test]
    fn ppm_bounds_and_header_fail_closed() {
        let d = crate::test_support::Workspace::new();
        let path = d.root.join("bad.ppm");
        fs::write(&path, b"P6\n1024 768\n255\nshort").unwrap();
        assert!(Pixels::read(&path).is_err());
    }
    #[test]
    fn screenshot_png_preserves_the_exact_asserted_ppm_pixels() {
        let d = crate::test_support::Workspace::new();
        let ppm = d.root.join("frame.ppm");
        let png = d.root.join("frame.png");
        let mut bytes = b"P6\n1024 768\n255\n".to_vec();
        for i in 0..1024 * 768 {
            bytes.extend_from_slice(&[(i % 251) as u8, (i % 241) as u8, (i % 239) as u8]);
        }
        fs::write(&ppm, bytes).unwrap();
        let pixels = Pixels::read(&ppm).unwrap();
        pixels.save_png(&png).unwrap();
        let encoded = resvg::tiny_skia::Pixmap::load_png(png).unwrap();
        for (rgb, rgba) in pixels.bytes[pixels.start..]
            .chunks_exact(3)
            .zip(encoded.data().chunks_exact(4))
        {
            assert_eq!(&rgba[..3], rgb);
            assert_eq!(rgba[3], 255);
        }
    }
}
#[cfg(test)]
mod probe_tests {
    use super::*;
    #[test]
    fn the_notes_bar_probe_accepts_mica_tint_but_not_content_or_wallpaper() {
        let bar = window_points::NOTES_BAR;
        // desktop-suite "moved": the bar over the light-blue Bloom sky.
        assert!(probe_matches(bar, [236, 243, 247]));
        assert!(!probe_matches(bar, window_points::content()));
        assert!(!probe_matches(bar, window_points::SCENE_WALLPAPER));
        assert!(!probe_matches(window_points::MENU_FOCUS, [240, 239, 239]));
    }
}
