//! P13 Windows GUI acceptance suites on the Door desktop
//! (`cargo xtask test compat --suite win32-gui-f1|win32-gui-f2|win32-gui-f3|
//! scenario-141-5|scenario-141-6`, contract `specs/contracts/windows-gui-v1.md`).
//!
//! Each suite rebuilds the fixtures twice and requires the bytes recorded by
//! the Windows reference (`tests/fixtures/windows-gui/reference.json`) and the
//! published import inventory, builds the original GUI DLLs, and boots a
//! private image variant whose initfs adds the fixtures (installed at
//! `/system/bin/wingui-*.exe`) and `windows/{user32,gdi32,comdlg32,comctl32}.dll`.
//! Input is real QMP keyboard and pointer input only. A process is identified
//! by the kernel's `DOOR_P09 MODULE` record carrying the fixture's SHA-256 and
//! retired by its `DOOR_P08 PROCESS ... uid=1001 ... exit=` record; fixture
//! results files and saved documents are read back from paused-guest copies and
//! the final disk image (journal replay, ext2 tree, catalog), never from the
//! application's own view; screenshots are QEMU screendumps used only for
//! visual claims.
use super::*;
use crate::windows::{dlls, gui};
use std::collections::BTreeMap;

pub(super) const RESOLUTION: &str = "1920x1080";
pub(super) const HOME: &str = "/users/1001";
const USER: u64 = 1001;
/// F1/F2/F3 window rectangles requested by the fixtures (x, y, w, h).
const F1_RECT: [i32; 4] = [48, 48, 480, 320];
pub(super) const F2_RECT: [i32; 4] = [64, 64, 640, 440];
const F3_RECT: [i32; 4] = [200, 160, 420, 260];
/// F2 custom chrome: requested rectangle and the real-input drags.
const CHROME: [i32; 4] = [760, 120, 320, 200];
const CHROME_MOVE: (i32, i32) = (120, 80);
const CHROME_GROW: (i32, i32) = (40, 30);
/// Text typed into F2 with the US layout and what the edit control saves.
const F2_TYPED: [&str; 2] = ["Door P13 editor", "second line"];
const F2_APPENDED: &str = " reopened";
const F2_DOCUMENT: &str = "editor-guest.txt";
/// Inside F2's edit control (x, y, w, h): its client area left of the
/// vertical scroll bar, where the typed lines are drawn.
const F2_EDIT: [i32; 4] = [76, 120, 596, 340];
/// Typing must change at least `F2_TEXT_CHANGED` edit pixels and add
/// `F2_TEXT_INK` near-black ones (the two typed lines measured 741 changed
/// and 15 -> 220 ink pixels in run host-1792978236704166300-6824; before the
/// fix the area stayed blank).
const F2_TEXT_CHANGED: usize = 300;
const F2_TEXT_INK: usize = 100;
/// 141.5 editor main window: window rectangle F2_RECT; the server-drawn
/// native window lies inside the 8 px invisible Win32 frame on the left,
/// right and bottom (`door_windows_gui::metrics::frame`) and carries the
/// 32 px server caption, then the 20 px menu bar. It is moved by its caption,
/// then sized from the native bottom-right corner (4 px resize edge, inside
/// the 8 px rounded corner).
const MAIN_NATIVE: [i32; 4] = [F2_RECT[0] + 8, F2_RECT[1], F2_RECT[2] - 16, F2_RECT[3] - 8];
const MAIN_MOVE: (i32, i32) = (160, 90);
const MAIN_GROW: (i32, i32) = (80, 60);
/// Client height lost to the caption and menu bar.
const MAIN_NONCLIENT_TOP: i32 = 32 + 20;
/// 141.5 chrome: a completed grip resize the fixture reports, then a held
/// grip resize whose window is destroyed (Alt+F4) before the button is up.
const CHROME_141_GROW: (i32, i32) = (30, 20);
const CHROME_HELD: (i32, i32) = (40, 30);
/// The fixture's chrome layout (`tests/fixtures/windows-gui/editor.c`):
/// caption height, close box width at the client's right, size grip.
const CHROME_CAPTION: i32 = 32;
const CHROME_CLOSE: i32 = 40;
const CHROME_GRIP: i32 = 16;
/// Text copied from Notes into the Win32 editor in 141.5.
const NOTES_TO_WIN32: &str = "copied from Notes";
/// Window-server line reporting what an owner still holds once its
/// connection is retired; every count must be zero. Requested from the
/// window-server owner on 2026-10-26 (`DOOR_P13 OWNER_RELEASED pid=P
/// windows=N surfaces=N buffers=N attachments=N bytes=N`); until the window
/// server emits it the 141.5 and F3 accounting checks fail closed.
const OWNER_RELEASED: &str = "DOOR_P13 OWNER_RELEASED pid=";
/// The Windows hung semantics as the window server reports them (records of
/// `graphics/window-server/src/authority.rs`, `not_responding.rs`, being
/// finished by the window-server owner on 2026-10-26): a client whose events
/// wait five seconds without a POLL is hung and its windows become ghosts
/// with " (Not Responding)" after the title; close on a hung window opens the
/// End task / Wait prompt (focus on Wait: Esc waits, Tab then Enter ends the
/// task); a responsive Win32 window gets its CLOSE at once, no deadline.
const HUNG_GHOST: &str = "DOOR_P13 WIN32 NOT_RESPONDING pid=";
const HUNG_PROMPT: &str = "DOOR_P13 WIN32 NOT_RESPONDING_PROMPT pid=";
const HUNG_WAITED: &str = "DOOR_P13 WIN32 WAIT pid=";
const HUNG_ENDED: &str = "DOOR_P13 WIN32 END_TASK pid=";
const HUNG_WAIT_KEYS: &[&[&str]] = &[&["esc"]];
const HUNG_END_TASK_KEYS: &[&[&str]] = &[&["tab"], &["ret"]];
/// "(Not Responding)" appended to the ghost's title changes at least this
/// many caption pixels of the hung window.
const GHOST_TITLE_PIXELS: usize = 100;

/// The staged image variant and the identities the suites compare against.
pub(super) struct Staged {
    pub(super) image: String,
    pub(super) fixtures: BTreeMap<String, String>,
    dlls: BTreeMap<String, String>,
    reference: Value,
    pub(super) record: Value,
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Builds and stages once per run; later suites of the same run reuse it.
/// Where Doorsh copies the staged GUI fixtures from (`stage`).
const FIXTURES: &str = "C:/Windows/System32";
pub(super) fn stage(run: &mut Run, image: &str) -> Result<Staged, String> {
    let cache = run.directory.join("win32-gui-stage.json");
    let record = if cache.is_file() {
        crate::image::read_json(&cache)?
    } else {
        let target = run.generated_directory(&format!("out/builds/{}/win32-gui", run.id))?;
        let (fixtures, reproducibility) = gui::double_build(run, &target)?;
        let published = gui::check_published(&run.root, &fixtures)?;
        let dll_artifacts = dlls::build_gui(run, &target.join("dlls"))?;
        let (base, _) = crate::image::validate_staged(&run.root, image)?;
        let production = fs::read(base.join("esp/Door/initfs.tar")).map_err(|e| e.to_string())?;
        let archive = door_initfs::Archive::parse(&production)
            .map_err(|e| format!("desktop initfs {e:?}"))?;
        let manifest = archive
            .find("system/install.manifest")
            .ok_or("desktop initfs lacks system/install.manifest")?
            .data
            .to_vec();
        let mut manifest = String::from_utf8(manifest).map_err(|e| e.to_string())?;
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        for artifact in &fixtures {
            let name = artifact["name"].as_str().ok_or("fixture name")?;
            let path =
                paths::existing(&run.root, artifact["path"].as_str().ok_or("fixture path")?)?;
            files.push((
                format!("seed/{name}.exe"),
                fs::read(&path).map_err(|e| e.to_string())?,
            ));
            if !manifest.ends_with('\n') {
                manifest.push('\n');
            }
            manifest.push_str(&format!("/system/bin/{name}.exe seed/{name}.exe\n"));
            // Also where a Windows path reaches it: Doorsh (the Terminal's
            // default shell) maps `/system/bin` to C:\system\bin, so the
            // scenarios copy the fixtures from System32.
            manifest.push_str(&format!(
                "/compat/Windows/System32/{name}.exe seed/{name}.exe\n"
            ));
        }
        let mut dll_hashes = BTreeMap::new();
        for artifact in &dll_artifacts {
            let name = artifact["name"].as_str().ok_or("DLL name")?;
            if !dlls::GUI_NAMES.contains(&name) {
                continue;
            }
            let path = paths::existing(&run.root, artifact["path"].as_str().ok_or("DLL path")?)?;
            dll_hashes.insert(format!("{name}.dll"), file_hash(&path)?);
            let bytes = fs::read(&path).map_err(|e| e.to_string())?;
            let destination = format!("windows/{name}.dll");
            // The desktop image ships the GUI DLLs; they must be the bytes
            // of this independent rebuild. An older image gets them staged.
            match archive.find(&destination) {
                Some(entry) if entry.data == bytes.as_slice() => (),
                Some(_) => {
                    return Err(format!(
                        "desktop image {destination} differs from the reproducible rebuild"
                    ));
                }
                None => files.push((destination, bytes)),
            }
        }
        files.push(("system/install.manifest".into(), manifest.into_bytes()));
        let edits: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(n, b)| (n.as_str(), b.as_slice()))
            .collect();
        let initfs = target.join("initfs-desktop-win32.tar");
        fs::write(
            &initfs,
            crate::initfs::with_files(&production, &edits, &["system/install.manifest"])?,
        )
        .map_err(|e| e.to_string())?;
        let staged = session::stage_desktop_variant(
            run,
            &base.join("esp/EFI/BOOT/BOOTX64.EFI"),
            &base.join("esp/Door/kernel.elf"),
            &initfs,
            "desktop-win32",
        )?;
        let record = json!({"image":staged,"base_image":image,"initfs":relative(&run.root,&initfs)?,
            "initfs_sha256":file_hash(&initfs)?,"fixtures":fixtures.iter().map(|a| (a["name"].as_str().unwrap_or("").to_owned(), a["artifact"]["sha256"].clone())).collect::<BTreeMap<_,_>>(),
            "dlls":dll_hashes,"reproducibility":reproducibility,"published":published,
            "dll_artifacts":dll_artifacts,"fixture_artifacts":fixtures});
        crate::evidence::write_json(&cache, &record)?;
        run.check("p13-win32-gui-stage", record.clone());
        record
    };
    let strings = |key: &str| -> Result<BTreeMap<String, String>, String> {
        record[key]
            .as_object()
            .ok_or(format!("stage record {key}"))?
            .iter()
            .map(|(k, v)| Ok((k.clone(), v.as_str().ok_or("stage hash")?.to_owned())))
            .collect()
    };
    Ok(Staged {
        image: record["image"].as_str().ok_or("staged image")?.to_owned(),
        fixtures: strings("fixtures")?,
        dlls: strings("dlls")?,
        reference: crate::image::read_json(&paths::existing(&run.root, gui::REFERENCE)?)?,
        record,
    })
}

/// The reference results text of `directory/results` (e.g. `f2/wingui-editor.results`).
fn reference_text(staged: &Staged, key: &str) -> Result<String, String> {
    staged.reference["results"][key]["text"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("reference results {key} missing"))
}
/// The reference text up to and including the first line starting `marker`.
fn reference_prefix(staged: &Staged, key: &str, marker: &str) -> Result<String, String> {
    let text = reference_text(staged, key)?;
    let start = text
        .find(marker)
        .ok_or(format!("reference {key} lacks {marker}"))?;
    let end = text[start..].find("\r\n").ok_or("reference line end")? + start + 2;
    Ok(text[..end].to_owned())
}

/// Kernel records of one Windows process.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Process {
    pub(super) modules: Vec<String>,
    pub(super) exit: Option<i64>,
    uid: Option<u64>,
    module_offset: usize,
    exit_offset: usize,
}
fn fields(text: &str) -> BTreeMap<&str, &str> {
    text.split_whitespace()
        .filter_map(|w| w.split_once('='))
        .collect()
}
/// `DOOR_P09 MODULE pid= index= ... sha256=` and `DOOR_P08 PROCESS ... pid=
/// ... uid= ... exit=` records after `offset`, keyed by pid.
pub(super) fn processes(serial: &str, offset: usize) -> BTreeMap<u64, Process> {
    let mut found: BTreeMap<u64, Process> = BTreeMap::new();
    let mut at = offset.min(serial.len());
    for line in serial.get(at..).unwrap_or("").split_inclusive('\n') {
        let position = at;
        at += line.len();
        if let Some((_, rest)) = line.split_once("DOOR_P09 MODULE ") {
            let f = fields(rest);
            if let (Some(pid), Some(index), Some(hash)) = (
                f.get("pid").and_then(|p| p.parse().ok()),
                f.get("index").and_then(|i| i.parse::<usize>().ok()),
                f.get("sha256"),
            ) {
                let process = found.entry(pid).or_default();
                if index == 0 {
                    process.module_offset = position;
                }
                if process.modules.len() == index {
                    process.modules.push((*hash).to_owned());
                }
            }
        } else if let Some((_, rest)) = line.split_once("DOOR_P08 PROCESS ") {
            let f = fields(rest);
            if let Some(pid) = f.get("pid").and_then(|p| p.parse().ok())
                && let Some(process) = found.get_mut(&pid)
            {
                process.exit = f.get("exit").and_then(|e| e.parse().ok());
                process.uid = f.get("uid").and_then(|u| u.parse().ok());
                process.exit_offset = position;
            }
        }
    }
    found
}
/// The first process after `offset` whose executable module has `hash`.
fn launched(serial: &str, offset: usize, hash: &str) -> Option<(u64, Process)> {
    processes(serial, offset)
        .into_iter()
        .find(|(_, p)| p.modules.first().is_some_and(|m| m == hash))
}

pub(super) fn wait_launch(
    run: &Run,
    record: &Value,
    offset: usize,
    hash: &str,
    what: &str,
) -> Result<u64, String> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some((pid, _)) = launched(&serial(&run.root, record)?, offset, hash) {
            return Ok(pid);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{what}: no DOOR_P09 MODULE record for the fixture bytes {hash}"
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// Screenshot evidence collected while waiting.
struct Watch<'a> {
    baseline: &'a scene::Pixels,
    rect: [i32; 4],
    name: &'a str,
}
pub(super) fn screen(run: &Run, c: &mut qmp::Client, name: &str) -> Result<scene::Pixels, String> {
    let ppm = run.directory.join(format!("win32-{name}.ppm"));
    c.execute(
        "screendump",
        json!({"filename":ppm.to_string_lossy().trim_start_matches("\\\\?\\").replace('\\',"/"),"format":"ppm"}),
    )?;
    scene::Pixels::read(&ppm)
}
pub(super) fn keep(run: &Run, p: &scene::Pixels, name: &str) -> Result<Value, String> {
    let png = run.directory.join(format!("win32-{name}.png"));
    p.save_png(&png)?;
    Ok(json!({"png":relative(&run.root,&png)?,"sha256":file_hash(&png)?}))
}
/// Pixels inside `r` that differ (sum of channel deltas above 48).
pub(super) fn changed(a: &scene::Pixels, b: &scene::Pixels, r: [i32; 4]) -> Result<usize, String> {
    let x0 = r[0].clamp(0, a.width as i32) as usize;
    let y0 = r[1].clamp(0, a.height as i32) as usize;
    let x1 = (r[0] + r[2]).clamp(0, a.width as i32) as usize;
    let y1 = (r[1] + r[3]).clamp(0, a.height as i32) as usize;
    let mut n = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let (p, q) = (a.pixel(x, y)?, b.pixel(x, y)?);
            n += usize::from((0..3).map(|i| u32::from(p[i].abs_diff(q[i]))).sum::<u32>() > 48);
        }
    }
    Ok(n)
}
/// Dark (text-ink) pixels inside `r`: every channel below 110.
fn ink(p: &scene::Pixels, r: [i32; 4]) -> Result<usize, String> {
    let mut n = 0;
    for y in r[1].max(0) as usize..(r[1] + r[3]).clamp(0, p.height as i32) as usize {
        for x in r[0].max(0) as usize..(r[0] + r[2]).clamp(0, p.width as i32) as usize {
            n += usize::from(p.pixel(x, y)?.iter().all(|&c| c < 110));
        }
    }
    Ok(n)
}
/// A window "appears" when at least a tenth of its requested rectangle changed.
fn appeared(rect: [i32; 4], pixels: usize) -> bool {
    pixels * 10 >= (rect[2] * rect[3]) as usize
}
fn similar(p: [u8; 3], q: [u8; 3]) -> bool {
    (0..3).map(|i| u32::from(p[i].abs_diff(q[i]))).sum::<u32>() <= 48
}
/// Geometry from two screendumps. Informative pixels of `before` inside `r`
/// are edges (unlike their left, or upper, neighbour) that changed in
/// `after` at the same place; one matches when the edge pair reappears in
/// `after` displaced by `shift`. Content that moved by exactly `shift`
/// matches; a window left in place, or moved by anything else (even one
/// pixel), does not. Returns (informative, matched) for vertical edges
/// (left neighbour) and horizontal edges (upper neighbour).
fn displaced(
    before: &scene::Pixels,
    after: &scene::Pixels,
    r: [i32; 4],
    shift: (i32, i32),
) -> Result<[(usize, usize); 2], String> {
    let (w, h) = (before.width as i32, before.height as i32);
    let mut counts = [(0, 0); 2];
    let at = |p: &scene::Pixels, x: i32, y: i32| p.pixel(x as usize, y as usize);
    for y in r[1].max(1)..(r[1] + r[3]).min(h) {
        for x in r[0].max(1)..(r[0] + r[2]).min(w) {
            let (tx, ty) = (x + shift.0, y + shift.1);
            if tx < 1 || ty < 1 || tx >= w || ty >= h {
                continue;
            }
            let p = at(before, x, y)?;
            if similar(p, at(after, x, y)?) {
                continue;
            }
            for (slot, (nx, ny)) in [(x - 1, y), (x, y - 1)].into_iter().enumerate() {
                let neighbour = at(before, nx, ny)?;
                if similar(p, neighbour) {
                    continue;
                }
                counts[slot].0 += 1;
                let (mx, my) = (nx + shift.0, ny + shift.1);
                counts[slot].1 += usize::from(
                    similar(p, at(after, tx, ty)?) && similar(neighbour, at(after, mx, my)?),
                );
            }
        }
    }
    Ok(counts)
}
/// At least 40 informative edge pixels, and 80 % of them (overall and in
/// each orientation with 20 or more) displaced by `shift`.
fn require_displaced(
    what: &str,
    before: &scene::Pixels,
    after: &scene::Pixels,
    r: [i32; 4],
    shift: (i32, i32),
) -> Result<Value, String> {
    let counts = displaced(before, after, r, shift)?;
    let informative = counts[0].0 + counts[1].0;
    let matched = counts[0].1 + counts[1].1;
    let short = |(i, m): (usize, usize)| m * 5 < i * 4;
    if informative < 40
        || short((informative, matched))
        || counts.iter().any(|&(i, m)| i >= 20 && short((i, m)))
    {
        return Err(format!(
            "{what}: edges of {r:?} not displaced by {shift:?} (vertical {}/{}, horizontal {}/{})",
            counts[0].1, counts[0].0, counts[1].1, counts[1].0
        ));
    }
    Ok(
        json!({"rect":r,"shift":[shift.0,shift.1],"vertical_edges":[counts[0].0,counts[0].1],
        "horizontal_edges":[counts[1].0,counts[1].1]}),
    )
}
/// A live bottom-right resize of the custom-chrome window at `rect` (x, y,
/// w, h) by `grow`, from two screendumps: the server's 1 px right edge
/// displaced by exactly the horizontal pointer delta and its bottom edge by
/// exactly the vertical one (regions inside the window, clear of the caption
/// and the grip, so the soft shadow over whatever lies outside does not
/// count), the close box the fixture paints at its
/// client's right edge (`paint_chrome`) repainted that much further right,
/// and the caption with its title left in place (a resize, not a move).
/// Counting changed pixels in the newly covered strip instead depends on
/// what the window grows over: in run host-1792984917188557000-10836 it was
/// Explorer's near-white page, within 48 of the #f3f3f3 body, and the window
/// that had grown by exactly (40, 30) changed 583 of 11700 strip pixels.
fn require_resized(
    what: &str,
    before: &scene::Pixels,
    after: &scene::Pixels,
    rect: [i32; 4],
    grow: (i32, i32),
) -> Result<Value, String> {
    let [x, y, w, h] = rect;
    let right = [
        x + w - 4,
        y + CHROME_CAPTION,
        4,
        h - CHROME_CAPTION - 2 * CHROME_GRIP,
    ];
    let bottom = [x + CHROME_GRIP, y + h - 4, w - 3 * CHROME_GRIP, 4];
    let close = [
        x + w - CHROME_CLOSE - 3,
        y,
        CHROME_CLOSE + 6,
        CHROME_CAPTION,
    ];
    let title = [x, y, w - CHROME_CLOSE - 3, CHROME_CAPTION];
    let right = require_displaced(
        &format!("{what} (right edge)"),
        before,
        after,
        right,
        (grow.0, 0),
    )?;
    let bottom = require_displaced(
        &format!("{what} (bottom edge)"),
        before,
        after,
        bottom,
        (0, grow.1),
    )?;
    let close = require_displaced(
        &format!("{what} (close box)"),
        before,
        after,
        close,
        (grow.0, 0),
    )?;
    let moved = changed(before, after, title)?;
    if appeared(title, moved) {
        return Err(format!(
            "{what}: its caption {title:?} changed ({moved} pixels), a move rather than a resize"
        ));
    }
    Ok(
        json!({"rect":rect,"grow":[grow.0,grow.1],"right_edge":right,"bottom_edge":bottom,
        "close_box":close,"caption_changed":moved}),
    )
}
/// The window server's retirement record of an owner: every count zero.
fn owner_released(run: &Run, record: &Value, offset: usize, pid: u64) -> Result<Value, String> {
    let needle = format!("{OWNER_RELEASED}{pid} ");
    wait_after(run, record, offset, &needle, 15)?;
    let text = serial(&run.root, record)?;
    let line = text
        .get(offset..)
        .unwrap_or("")
        .lines()
        .find(|l| l.contains(&needle))
        .ok_or("owner release record")?;
    let counts = fields(line.split_once(&needle).map_or("", |(_, rest)| rest));
    for key in ["windows", "surfaces", "buffers", "attachments", "bytes"] {
        if counts.get(key) != Some(&"0") {
            return Err(format!(
                "window server retained {key} of owner {pid} after cleanup: {line}"
            ));
        }
    }
    if let Some((key, value)) = counts.iter().find(|(_, v)| **v != "0") {
        return Err(format!(
            "window server retained {key}={value} of owner {pid}"
        ));
    }
    Ok(json!(counts))
}

/// Waits for the kernel's retirement record of `pid`, optionally sampling
/// screendumps for `watch`; returns (exit, uid, visual evidence).
fn wait_exit(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    pid: u64,
    offset: usize,
    seconds: u64,
    watch: Option<Watch>,
) -> Result<(i64, u64, Value), String> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let mut best: Option<(usize, scene::Pixels)> = None;
    let mut samples = 0;
    loop {
        let text = serial(&run.root, record)?;
        if let Some(process) = processes(&text, offset).remove(&pid)
            && let (Some(exit), Some(uid)) = (process.exit, process.uid)
        {
            let visual = match (watch, best) {
                (Some(w), Some((pixels, frame))) => {
                    json!({"rect":w.rect,"changed_pixels":pixels,"samples":samples,"appeared":appeared(w.rect, pixels),"screenshot":keep(run,&frame,w.name)?})
                }
                (Some(w), None) => json!({"rect":w.rect,"samples":samples,"appeared":false}),
                (None, _) => Value::Null,
            };
            return Ok((exit, uid, visual));
        }
        if Instant::now() >= deadline {
            return Err(format!("process {pid} did not retire within {seconds} s"));
        }
        if let Some(w) = &watch {
            let frame = screen(run, c, &format!("{}-sample", w.name))?;
            samples += 1;
            let pixels = changed(w.baseline, &frame, w.rect)?;
            if best.as_ref().is_none_or(|(b, _)| pixels > *b) {
                best = Some((pixels, frame));
            }
            // Back-to-back screendumps (each takes ~100 ms): a 300 ms gap
            // missed the short-lived Open dialog of F1 --reopen (run
            // host-1792961346122828100-15760); the criterion is unchanged.
            thread::sleep(Duration::from_millis(20));
        } else {
            thread::sleep(Duration::from_millis(100));
        }
    }
}

fn type_chars(c: &mut qmp::Client, text: &str) -> Result<(), String> {
    for ch in text.chars() {
        c.execute(
            "send-key",
            json!({"keys":session::keys(ch)?,"hold-time":20}),
        )?;
        thread::sleep(Duration::from_millis(80));
    }
    Ok(())
}
fn drag_to(
    c: &mut qmp::Client,
    p: &mut (i32, i32),
    from: (i32, i32),
    to: (i32, i32),
) -> Result<(), String> {
    scene::drag(c, p, from, to)?;
    thread::sleep(Duration::from_millis(400));
    Ok(())
}

/// Start search opens Terminal (app 9).
pub(super) fn open_terminal(run: &Run, record: &Value, c: &mut qmp::Client) -> Result<(), String> {
    let before = serial(&run.root, record)?.len();
    key(c, &["meta_l"])?;
    type_line(c, "terminal")?;
    resolution::wait_activation(&run.root, record, 9, before)
}
/// A Terminal builtin that must complete successfully.
pub(super) fn terminal(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    line: &str,
) -> Result<(), String> {
    let offset = serial(&run.root, record)?.len();
    type_line(c, line)?;
    let name = line
        .split_whitespace()
        .next()
        .ok_or("empty Terminal command")?;
    wait_after(
        run,
        record,
        offset,
        &format!("TERMINAL COMMAND name={name} ok=1"),
        15,
    )
}
pub(super) fn wait_after(
    run: &Run,
    record: &Value,
    offset: usize,
    needle: &str,
    seconds: u64,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !serial(&run.root, record)?
        .get(offset..)
        .unwrap_or("")
        .contains(needle)
    {
        if Instant::now() >= deadline {
            return Err(format!("serial record missing: {needle}"));
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

/// One fixture run launched from the Terminal (bare name resolves to
/// `/system/bin`, working directory = the tab's directory).
#[allow(clippy::too_many_arguments)]
fn terminal_run(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    staged: &Staged,
    fixture: &str,
    argument: &str,
    exit: i64,
    rect: Option<[i32; 4]>,
) -> Result<Value, String> {
    let hash = staged
        .fixtures
        .get(fixture)
        .ok_or("staged fixture")?
        .clone();
    let baseline = screen(run, c, &format!("{fixture}{argument}-baseline"))?;
    let offset = serial(&run.root, record)?.len();
    let line = if argument.is_empty() {
        format!("{fixture}.exe")
    } else {
        format!("{fixture}.exe {argument}")
    };
    type_line(c, &line)?;
    let pid = wait_launch(run, record, offset, &hash, &line)?;
    let name = format!("{fixture}{argument}-window");
    let watch = rect.map(|rect| Watch {
        baseline: &baseline,
        rect,
        name: &name,
    });
    let (observed, uid, visual) = wait_exit(run, record, c, pid, offset, 120, watch)?;
    if observed != exit || uid != USER {
        return Err(format!(
            "{line}: exit {observed} uid {uid}; expected exit {exit} uid {USER}"
        ));
    }
    if rect.is_some() && visual["appeared"] != true {
        return Err(format!("{line}: no window appeared in the screendumps"));
    }
    wait_after(
        run,
        record,
        offset,
        &format!("TERMINAL COMMAND name={fixture}.exe"),
        15,
    )?;
    let modules = modules_of(run, record, offset, pid)?;
    require_gui_modules(staged, fixture, &modules)?;
    Ok(
        json!({"command":line,"pid":pid,"exit":observed,"uid":uid,"modules":modules,"visual":visual}),
    )
}
fn modules_of(run: &Run, record: &Value, offset: usize, pid: u64) -> Result<Vec<String>, String> {
    processes(&serial(&run.root, record)?, offset)
        .remove(&pid)
        .map(|p| p.modules)
        .ok_or_else(|| format!("process {pid} module records missing"))
}
/// Every GUI DLL the fixture imports was mapped from the staged bytes.
fn require_gui_modules(staged: &Staged, fixture: &str, modules: &[String]) -> Result<(), String> {
    let inventory = staged.record["fixture_artifacts"]
        .as_array()
        .ok_or("stage fixtures")?
        .iter()
        .find(|a| a["name"] == fixture)
        .ok_or("stage fixture")?["imports"]
        .clone();
    for module in inventory.as_object().ok_or("fixture imports")?.keys() {
        if let Some(hash) = staged.dlls.get(module)
            && !modules.contains(hash)
        {
            return Err(format!(
                "{fixture}: {module} was not mapped from the staged DLL bytes"
            ));
        }
    }
    Ok(())
}

/// Explorer launch: the address bar resolves the executable's Windows path
/// and Init EXECUTE starts it with its containing folder as working directory.
fn explorer_run(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    staged: &Staged,
    folder: &str,
    fixture: &str,
) -> Result<(u64, usize), String> {
    let hash = staged
        .fixtures
        .get(fixture)
        .ok_or("staged fixture")?
        .clone();
    resolution::activate(&run.root, record, c, 0)?;
    let offset = serial(&run.root, record)?.len();
    key(c, &["ctrl", "l"])?;
    type_line(c, &format!(r"C:\Users\1001\{folder}\{fixture}.exe"))?;
    let pid = wait_launch(
        run,
        record,
        offset,
        &hash,
        &format!("Explorer {fixture}.exe"),
    )?;
    Ok((pid, offset))
}

/// Pauses the guest, copies its disk and returns every file of the copy.
fn checkpoint(
    run: &Run,
    c: &mut qmp::Client,
    disk: &std::path::Path,
    name: &str,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    c.execute("stop", json!({}))?;
    let copy = run.directory.join(format!("win32-{name}.raw"));
    let copied = fs::copy(disk, &copy).map_err(|e| e.to_string());
    let resumed = c.execute("cont", json!({}));
    copied?;
    resumed?;
    let files = disk_files(&copy, false);
    let _ = fs::remove_file(&copy);
    files
}
/// Files of a raw disk through journal replay, the ext2 tree and the catalog.
fn disk_files(disk: &std::path::Path, clean: bool) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let raw = fs::read(disk).map_err(|e| e.to_string())?;
    let journal = crate::persistence::journal::inspect(&raw)?;
    if clean && journal.selected().committed {
        return Err("Windows GUI suite left a committed journal".into());
    }
    let tree = crate::persistence::tree::inspect(&journal.replayed_home(&raw)?)?;
    let catalog = crate::persistence::catalog::decode(&tree)?.ok_or("catalog absent")?;
    let mut files = BTreeMap::new();
    for (path, node) in catalog.paths()? {
        if let Some(file) = tree.files.get(&format!("obj{:016x}", node.id)) {
            files.insert(path.to_string(), file.bytes.clone());
        }
    }
    Ok(files)
}
/// Polls paused-guest copies until `path` satisfies `ok`.
fn wait_file(
    run: &Run,
    c: &mut qmp::Client,
    disk: &std::path::Path,
    path: &str,
    seconds: u64,
    ok: impl Fn(&[u8]) -> bool,
) -> Result<Vec<u8>, String> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let files = checkpoint(run, c, disk, "poll")?;
        if let Some(bytes) = files.get(path)
            && ok(bytes)
        {
            return Ok(bytes.clone());
        }
        if Instant::now() >= deadline {
            return Err(format!("{path} did not reach the expected bytes on disk"));
        }
        thread::sleep(Duration::from_secs(2));
    }
}
fn text_of<'a>(files: &'a BTreeMap<String, Vec<u8>>, path: &str) -> Result<&'a str, String> {
    core::str::from_utf8(
        files
            .get(path)
            .ok_or(format!("{path} absent from the disk"))?,
    )
    .map_err(|_| format!("{path} is not UTF-8"))
}
fn require_bytes(
    files: &BTreeMap<String, Vec<u8>>,
    path: &str,
    sha: &str,
) -> Result<Value, String> {
    let bytes = files
        .get(path)
        .ok_or(format!("{path} absent from the disk"))?;
    if sha256(bytes) != sha {
        return Err(format!(
            "{path} bytes differ from the expected identity {sha}"
        ));
    }
    Ok(json!({"path":path,"bytes":bytes.len(),"sha256":sha}))
}
/// The installed fixture executables are the reference bytes.
fn require_installed(files: &BTreeMap<String, Vec<u8>>, staged: &Staged) -> Result<Value, String> {
    let mut installed = Vec::new();
    for (name, sha) in &staged.fixtures {
        installed.push(require_bytes(
            files,
            &format!("/system/bin/{name}.exe"),
            sha,
        )?);
    }
    Ok(json!(installed))
}

pub(super) fn boot(
    run: &mut Run,
    staged: &Staged,
    disk: &std::path::Path,
) -> Result<Value, String> {
    let record = session::launch_desktop_accelerated(run, &staged.image, disk, false, RESOLUTION)?;
    thread::sleep(Duration::from_secs(2));
    Ok(record)
}
/// The kernel's once-per-boot rollback sweep of the P13 application path
/// (every construction checkpoint failed once, nothing leaked), run before
/// the first ordinary application of the boot.
pub(super) fn require_app_rollback(serial: &str) -> Result<(), String> {
    if serial.contains("DOOR_P13 APP_ROLLBACK FAIL") {
        return Err("kernel application-path rollback sweep failed".into());
    }
    let line = serial
        .lines()
        .find(|l| l.contains("DOOR_P13 APP_ROLLBACK PASS "))
        .ok_or("kernel application-path rollback sweep missing")?;
    let f = fields(line.split_once("APP_ROLLBACK PASS ").map_or("", |(_, r)| r));
    let number = |key: &str| f.get(key).and_then(|v| v.parse::<u64>().ok());
    if number("checkpoints").is_none_or(|n| n == 0)
        || number("modules").is_none_or(|n| n < 2)
        || f.get("leaks") != Some(&"0")
    {
        return Err(format!(
            "incomplete application-path rollback record: {line}"
        ));
    }
    Ok(())
}
pub(super) fn finish(run: &mut Run, record: &mut Value, c: &mut qmp::Client) -> Result<(), String> {
    let shut_down = require_app_rollback(&serial(&run.root, record)?)
        .and_then(|()| start_shut_down(c))
        .and_then(|()| {
            wait_serial(
                &run.root,
                record,
                "DOOR_P08 CLEANUP PASS",
                Duration::from_secs(40),
            )
        });
    if let Err(error) = shut_down {
        // A guest that panicked or hung while shutting down halts without
        // exiting: the emulator goes, with the serial kept for inspection.
        let _ = c.execute("quit", json!({}));
        return Err(error);
    }
    seal_serial(&run.root, record)
}
/// Where each vCPU is, `count` samples 300 ms apart (QEMU's `info
/// registers -a`): `cpuN rip=.. cpl=.. hlt=.. rsp=.. cr3=..` per CPU. A
/// stalled guest program shows its user RIPs (match them against the
/// `DOOR_P09 MODULE` / `DOOR_LDR` bases) or every CPU halted (it waits).
pub(super) fn vcpu_samples(c: &mut qmp::Client, count: usize) -> Vec<String> {
    let mut out = Vec::new();
    for sample in 0..count {
        let Ok(value) = c.execute(
            "human-monitor-command",
            json!({"command-line":"info registers -a"}),
        ) else {
            out.push(format!("sample {sample}: monitor unavailable"));
            break;
        };
        let text = value.as_str().unwrap_or("");
        let mut line = String::new();
        let field = |l: &str, key: &str| -> Option<String> {
            l.split_whitespace()
                .find_map(|w| w.strip_prefix(key).map(str::to_owned))
        };
        for l in text.lines() {
            if let Some(cpu) = l.strip_prefix("CPU#") {
                if !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                }
                line = format!("sample {sample} cpu{}", cpu.trim());
            } else if l.starts_with("RIP=") {
                for key in ["RIP=", "CPL=", "HLT="] {
                    if let Some(v) = field(l, key) {
                        line.push_str(&format!(" {}{v}", key.to_lowercase()));
                    }
                }
            } else if l.contains("RSP=")
                && let Some(v) = field(l, "RSP=")
            {
                line.push_str(&format!(" rsp={v}"));
            } else if l.starts_with("CR0=")
                && let Some(v) = field(l, "CR3=")
            {
                line.push_str(&format!(" cr3={v}"));
            }
        }
        if !line.is_empty() {
            out.push(line);
        }
        thread::sleep(Duration::from_millis(300));
    }
    out
}
pub(super) fn quit_on_error<T>(
    run: &mut Run,
    record: &Value,
    result: Result<T, String>,
    name: &str,
) -> Result<T, String> {
    if result.is_err()
        && let Ok(mut c) = connection(record)
    {
        let _ = scene::capture_settled(run, &mut c, name, &[]);
        let _ = c.execute("quit", json!({}));
    }
    result
}
pub(super) fn fresh_disk(run: &Run, name: &str) -> Result<std::path::PathBuf, String> {
    let disk = run.directory.join(format!("{name}-user.raw"));
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    Ok(disk)
}
/// The decoded console stream (`DOOR_CONSOLE` records) contains `text`.
fn require_console(run: &Run, record: &Value, text: &str) -> Result<(), String> {
    if !session::transcript(&run.root, record)?.contains(text) {
        return Err(format!(
            "standard output of the Terminal-launched fixture is missing from the console stream: {:?}",
            text.lines().next().unwrap_or("")
        ));
    }
    Ok(())
}

/// WIN-GUI F1: the self-test and its reopen relaunch from the Terminal.
pub(super) fn f1(run: &mut Run, image: &str, _test_image: &str) -> Result<Value, String> {
    let staged = stage(run, image)?;
    let disk = fresh_disk(run, "win32-f1")?;
    let mut record = boot(run, &staged, &disk)?;
    let result = (|| -> Result<Value, String> {
        let mut c = connection(&record)?;
        open_terminal(run, &record, &mut c)?;
        terminal(run, &record, &mut c, "mkdir wingui-f1")?;
        terminal(run, &record, &mut c, "cd wingui-f1")?;
        let first = terminal_run(
            run,
            &record,
            &mut c,
            &staged,
            "wingui-selftest",
            "",
            75,
            Some(F1_RECT),
        )?;
        let reopen = terminal_run(
            run,
            &record,
            &mut c,
            &staged,
            "wingui-selftest",
            "--reopen",
            76,
            Some(F1_RECT),
        )?;
        finish(run, &mut record, &mut c)?;
        Ok(json!({"self_test":first,"reopen":reopen}))
    })();
    let mut evidence = quit_on_error(run, &record, result, "win32-f1-failure")?;
    let files = disk_files(&disk, true)?;
    let folder = format!("{HOME}/wingui-f1");
    let self_test = reference_text(&staged, "f1/wingui-selftest.results")?;
    let reopen = reference_text(&staged, "f1/wingui-selftest-reopen.results")?;
    for (name, expected) in [
        ("wingui-selftest.results", &self_test),
        ("wingui-selftest-reopen.results", &reopen),
    ] {
        if text_of(&files, &format!("{folder}/{name}"))? != expected.as_str() {
            return Err(format!("F1 {name} differs from the Windows reference"));
        }
        require_console(run, &record, expected)?;
    }
    let saved = staged.reference["saved"]["f1/wingui-selftest-\u{3bb}.txt"]["sha256"]
        .as_str()
        .ok_or("reference F1 saved document")?;
    evidence["saved"] = require_bytes(
        &files,
        &format!("{folder}/wingui-selftest-\u{3bb}.txt"),
        saved,
    )?;
    evidence["installed"] = require_installed(&files, &staged)?;
    evidence["results"] =
        json!({"self_test":self_test,"reopen":reopen,"equal_to_reference":true,"console":true});
    evidence["reference_comparison"] = json!({"state":"passed","reference":gui::REFERENCE,
        "compared":["f1/wingui-selftest.results","f1/wingui-selftest-reopen.results","f1/wingui-selftest-{3bb}.txt"]});
    evidence["session"] = record;
    evidence["stage"] = json!({"image":staged.image,"fixtures":staged.fixtures,"dlls":staged.dlls});
    Ok(evidence)
}

/// The interactive F2 results lines a correct session produces.
fn f2_interactive_expected() -> String {
    let document = format!("{}\r\n{}", F2_TYPED[0], F2_TYPED[1]);
    let reopened = format!("{document}{F2_APPENDED}");
    let crc = |s: &str| crate::storage::audit::crc32(s.as_bytes());
    let (x, y) = (CHROME[0] + CHROME_MOVE.0, CHROME[1] + CHROME_MOVE.1);
    let (w, h) = (CHROME[2] + CHROME_GROW.0, CHROME[3] + CHROME_GROW.1);
    [
        "EDITOR_START variant=code mode=interactive".to_owned(),
        format!("EDITOR_DIALOG kind=save result=1 name={F2_DOCUMENT} offset_ok=1"),
        format!(
            "EDITOR_SAVE result=1 name={F2_DOCUMENT} chars={0} bytes={0} crc={1:08x}",
            document.len(),
            crc(&document)
        ),
        format!(
            "EDITOR_CHROME event=open x={} y={} w={} h={}",
            CHROME[0], CHROME[1], CHROME[2], CHROME[3]
        ),
        format!(
            "EDITOR_CHROME event=moved x={x} y={y} w={} h={}",
            CHROME[2], CHROME[3]
        ),
        format!("EDITOR_CHROME event=moved x={x} y={y} w={w} h={h}"),
        format!("EDITOR_CHROME event=close x={x} y={y} w={w} h={h}"),
        "EDITOR_EXIT variant=code code=0".to_owned(),
        "EDITOR_START variant=code mode=interactive".to_owned(),
        format!("EDITOR_DIALOG kind=open result=1 name={F2_DOCUMENT} offset_ok=1"),
        format!(
            "EDITOR_OPEN result=1 name={F2_DOCUMENT} bytes={0} chars={0} crc={1:08x}",
            document.len(),
            crc(&document)
        ),
        format!(
            "EDITOR_SAVE result=1 name={F2_DOCUMENT} chars={0} bytes={0} crc={1:08x}",
            reopened.len(),
            crc(&reopened)
        ),
        "EDITOR_EXIT variant=code code=0".to_owned(),
    ]
    .iter()
    .map(|l| format!("{l}\r\n"))
    .collect()
}

/// F2 session 1: type, Save As, custom chrome by mouse, File > Exit by menu.
fn f2_first(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    staged: &Staged,
    disk: &std::path::Path,
) -> Result<Value, String> {
    let baseline = screen(run, c, "f2-baseline")?;
    let (pid, offset) = explorer_run(run, record, c, staged, "wingui-f2", "wingui-editor")?;
    thread::sleep(Duration::from_secs(2));
    let shown = screen(run, c, "f2-shown")?;
    let window_pixels = changed(&baseline, &shown, F2_RECT)?;
    if !appeared(F2_RECT, window_pixels) {
        return Err("F2 window did not appear".into());
    }
    type_chars(c, F2_TYPED[0])?;
    key(c, &["ret"])?;
    type_chars(c, F2_TYPED[1])?;
    // The edit control must repaint what was typed: compared with the
    // pre-typing capture its area changes and gains text ink.
    thread::sleep(Duration::from_millis(500));
    let typed = screen(run, c, "f2-typed")?;
    let text_pixels = changed(&shown, &typed, F2_EDIT)?;
    let (ink_before, ink_after) = (ink(&shown, F2_EDIT)?, ink(&typed, F2_EDIT)?);
    if text_pixels < F2_TEXT_CHANGED || ink_after < ink_before + F2_TEXT_INK {
        keep(run, &typed, "f2-typed")?;
        return Err(format!(
            "F2 edit control did not repaint the typed text (changed {text_pixels}, ink {ink_before} -> {ink_after})"
        ));
    }
    key(c, &["ctrl", "s"])?;
    thread::sleep(Duration::from_millis(1500));
    let dialog = screen(run, c, "f2-save-dialog")?;
    type_line(c, F2_DOCUMENT)?;
    let document = format!("{}\r\n{}", F2_TYPED[0], F2_TYPED[1]);
    let path = format!("{HOME}/wingui-f2/{F2_DOCUMENT}");
    wait_file(run, c, disk, &path, 30, |b| b == document.as_bytes())?;
    // Custom chrome: caption drag moves, grip drag sizes, the drawn close box closes.
    key(c, &["ctrl", "k"])?;
    thread::sleep(Duration::from_millis(800));
    let chrome = screen(run, c, "f2-chrome")?;
    if !appeared(CHROME, changed(&shown, &chrome, CHROME)?) {
        return Err("F2 custom chrome window did not appear".into());
    }
    let mut p = (960, 540);
    let caption = (CHROME[0] + 100, CHROME[1] + 16);
    drag_to(
        c,
        &mut p,
        caption,
        (caption.0 + CHROME_MOVE.0, caption.1 + CHROME_MOVE.1),
    )?;
    let (x, y) = (CHROME[0] + CHROME_MOVE.0, CHROME[1] + CHROME_MOVE.1);
    let grip = (x + CHROME[2] - 6, y + CHROME[3] - 6);
    drag_to(
        c,
        &mut p,
        grip,
        (grip.0 + CHROME_GROW.0, grip.1 + CHROME_GROW.1),
    )?;
    let moved = screen(run, c, "f2-chrome-moved")?;
    click(c, &mut p, x + CHROME[2] + CHROME_GROW.0 - 20, y + 16)?;
    // File > Exit through the keyboard menu.
    key(c, &["alt", "f"])?;
    key(c, &["x"])?;
    let (exit, uid, _) = wait_exit(run, record, c, pid, offset, 30, None)?;
    if exit != 0 || uid != USER {
        return Err(format!("F2 session 1 exit {exit} uid {uid}"));
    }
    Ok(json!({"pid":pid,"exit":exit,"window_pixels":window_pixels,
        "typed_text":{"rect":F2_EDIT,"changed_pixels":text_pixels,"ink_before":ink_before,"ink_after":ink_after},
        "screenshots":{"shown":keep(run,&shown,"f2-shown")?,"typed":keep(run,&typed,"f2-typed")?,"save_dialog":keep(run,&dialog,"f2-save-dialog")?,
            "chrome":keep(run,&chrome,"f2-chrome")?,"chrome_moved":keep(run,&moved,"f2-chrome-moved")?}}))
}

/// F2 session 2: relaunch, Open dialog, append, save without a dialog.
fn f2_second(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    staged: &Staged,
    disk: &std::path::Path,
) -> Result<Value, String> {
    let (pid, offset) = explorer_run(run, record, c, staged, "wingui-f2", "wingui-editor")?;
    thread::sleep(Duration::from_secs(2));
    key(c, &["ctrl", "o"])?;
    thread::sleep(Duration::from_millis(1500));
    type_line(c, F2_DOCUMENT)?;
    thread::sleep(Duration::from_millis(800));
    key(c, &["ctrl", "end"])?;
    type_chars(c, F2_APPENDED)?;
    key(c, &["ctrl", "s"])?;
    let expected = format!("{}\r\n{}{F2_APPENDED}", F2_TYPED[0], F2_TYPED[1]);
    let path = format!("{HOME}/wingui-f2/{F2_DOCUMENT}");
    wait_file(run, c, disk, &path, 30, |b| b == expected.as_bytes())?;
    let reopened = screen(run, c, "f2-reopened")?;
    key(c, &["alt", "f4"])?;
    let (exit, uid, _) = wait_exit(run, record, c, pid, offset, 30, None)?;
    if exit != 0 || uid != USER {
        return Err(format!("F2 session 2 exit {exit} uid {uid}"));
    }
    Ok(json!({"pid":pid,"exit":exit,"screenshot":keep(run,&reopened,"f2-reopened")?}))
}

/// The resource variant: Help > About shows the dialog template from resources.
fn f2_resources(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    staged: &Staged,
) -> Result<Value, String> {
    let (pid, offset) = explorer_run(run, record, c, staged, "wingui-f2", "wingui-editor-rc")?;
    thread::sleep(Duration::from_secs(2));
    let shown = screen(run, c, "f2rc-shown")?;
    key(c, &["alt", "h"])?;
    key(c, &["a"])?;
    thread::sleep(Duration::from_millis(1200));
    let about = screen(run, c, "f2rc-about")?;
    // The 220x72-unit About dialog covers far more than 1000 pixels.
    if changed(&shown, &about, [0, 0, 1920, 1080])? < 1000 {
        return Err("F2-RC About dialog did not appear".into());
    }
    key(c, &["ret"])?;
    thread::sleep(Duration::from_millis(500));
    key(c, &["alt", "f4"])?;
    let (exit, uid, _) = wait_exit(run, record, c, pid, offset, 30, None)?;
    if exit != 0 || uid != USER {
        return Err(format!("F2-RC exit {exit} uid {uid}"));
    }
    Ok(
        json!({"pid":pid,"exit":exit,"screenshots":{"shown":keep(run,&shown,"f2rc-shown")?,"about":keep(run,&about,"f2rc-about")?}}),
    )
}

/// WIN-GUI F2: both variants' self-tests from the Terminal (compared with the
/// reference), then the interactive editor from Explorer with real input:
/// type, Ctrl+S Save As, custom chrome by mouse, File > Exit, relaunch,
/// Ctrl+O reopen, append, save; the resource variant's About dialog.
pub(super) fn f2(run: &mut Run, image: &str, _test_image: &str) -> Result<Value, String> {
    let staged = stage(run, image)?;
    let disk = fresh_disk(run, "win32-f2")?;
    let mut record = boot(run, &staged, &disk)?;
    let result = (|| -> Result<Value, String> {
        let mut c = connection(&record)?;
        open_terminal(run, &record, &mut c)?;
        let mut selftests = Vec::new();
        for (folder, fixture) in [
            ("wingui-f2-self", "wingui-editor"),
            ("wingui-f2-rc-self", "wingui-editor-rc"),
        ] {
            terminal(run, &record, &mut c, &format!("mkdir {HOME}/{folder}"))?;
            terminal(run, &record, &mut c, &format!("cd {HOME}/{folder}"))?;
            selftests.push(terminal_run(
                run,
                &record,
                &mut c,
                &staged,
                fixture,
                "--selftest",
                80,
                Some(F2_RECT),
            )?);
            selftests.push(terminal_run(
                run, &record, &mut c, &staged, fixture, "--reopen", 81, None,
            )?);
        }
        terminal(run, &record, &mut c, &format!("mkdir {HOME}/wingui-f2"))?;
        for fixture in ["wingui-editor", "wingui-editor-rc"] {
            terminal(
                run,
                &record,
                &mut c,
                &format!("cp {FIXTURES}/{fixture}.exe {HOME}/wingui-f2/{fixture}.exe"),
            )?;
        }
        let first = f2_first(run, &record, &mut c, &staged, &disk)?;
        let second = f2_second(run, &record, &mut c, &staged, &disk)?;
        let resources = f2_resources(run, &record, &mut c, &staged)?;
        finish(run, &mut record, &mut c)?;
        Ok(
            json!({"self_tests":selftests,"interactive":{"first":first,"second":second,"resources":resources}}),
        )
    })();
    let mut evidence = quit_on_error(run, &record, result, "win32-f2-failure")?;
    let files = disk_files(&disk, true)?;
    for (folder, key, results) in [
        ("wingui-f2-self", "f2", "wingui-editor.results"),
        ("wingui-f2-rc-self", "f2-rc", "wingui-editor-rc.results"),
    ] {
        let expected = reference_text(&staged, &format!("{key}/{results}"))?;
        if text_of(&files, &format!("{HOME}/{folder}/{results}"))? != expected {
            return Err(format!(
                "F2 {results} self-test results differ from the Windows reference"
            ));
        }
        let saved = staged.reference["saved"][format!("{key}/doors-editor-\u{3bb}.txt")]["sha256"]
            .as_str()
            .ok_or("reference F2 saved document")?;
        require_bytes(
            &files,
            &format!("{HOME}/{folder}/doors-editor-\u{3bb}.txt"),
            saved,
        )?;
    }
    let interactive = f2_interactive_expected();
    if text_of(&files, &format!("{HOME}/wingui-f2/wingui-editor.results"))? != interactive {
        return Err("F2 interactive results differ from the expected callbacks, chrome geometry and saved bytes".into());
    }
    let resources = "EDITOR_START variant=rc mode=interactive\r\nEDITOR_ABOUT result=1\r\nEDITOR_EXIT variant=rc code=0\r\n";
    if text_of(
        &files,
        &format!("{HOME}/wingui-f2/wingui-editor-rc.results"),
    )? != resources
    {
        return Err("F2-RC interactive results differ (About dialog from resources)".into());
    }
    let document = format!("{}\r\n{}{F2_APPENDED}", F2_TYPED[0], F2_TYPED[1]);
    evidence["document"] = require_bytes(
        &files,
        &format!("{HOME}/wingui-f2/{F2_DOCUMENT}"),
        &sha256(document.as_bytes()),
    )?;
    for fixture in ["wingui-editor", "wingui-editor-rc"] {
        require_bytes(
            &files,
            &format!("{HOME}/wingui-f2/{fixture}.exe"),
            &staged.fixtures[fixture],
        )?;
    }
    evidence["installed"] = require_installed(&files, &staged)?;
    evidence["results"] =
        json!({"interactive":interactive,"resources":resources,"self_tests_equal_reference":true});
    evidence["reference_comparison"] = json!({"state":"passed","reference":gui::REFERENCE,
        "compared":["f2/wingui-editor.results","f2-rc/wingui-editor-rc.results","f2/doors-editor-{3bb}.txt","f2-rc/doors-editor-{3bb}.txt"]});
    evidence["session"] = record;
    Ok(evidence)
}

/// F3 interactive under the Windows hung semantics: launch from Explorer; an
/// idle window that keeps retrieving messages is never hung; after `h` the
/// thread retrieves nothing, and past five seconds the server shows the
/// "(Not Responding)" ghost. Alt+F4 on it offers End task or Wait: Wait keeps
/// the process, End task terminates it (never a close deadline), and the
/// window server then holds nothing for the owner.
fn f3_contain(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    staged: &Staged,
    disk: &std::path::Path,
    folder: &str,
) -> Result<Value, String> {
    let baseline = screen(run, c, "f3-baseline")?;
    let (pid, offset) = explorer_run(run, record, c, staged, folder, "wingui-hang")?;
    thread::sleep(Duration::from_secs(2));
    let shown = screen(run, c, "f3-shown")?;
    if !appeared(F3_RECT, changed(&baseline, &shown, F3_RECT)?) {
        return Err("F3 window did not appear".into());
    }
    // The 32 px server caption of the native window (inside the 8 px frame).
    let caption = [F3_RECT[0] + 8, F3_RECT[1], F3_RECT[2] - 16, 32];
    // Idle in GetMessage for longer than the threshold: not hung.
    let idle_at = serial(&run.root, record)?.len();
    thread::sleep(Duration::from_secs(6));
    let idle = screen(run, c, "f3-idle")?;
    let ghost = format!("{HUNG_GHOST}{pid} ");
    if serial(&run.root, record)?
        .get(idle_at..)
        .is_some_and(|t| t.contains(&ghost))
        || changed(&shown, &idle, caption)? >= GHOST_TITLE_PIXELS
    {
        return Err("an idle F3 window that keeps retrieving messages was treated as hung".into());
    }
    key(c, &["h"])?;
    let results = format!("{HOME}/{folder}/wingui-hang.results");
    wait_file(run, c, disk, &results, 20, |b| {
        core::str::from_utf8(b).is_ok_and(|t| t.ends_with("HANG_STOP pumping=0\r\n"))
    })?;
    let stopped = serial(&run.root, record)?.len();
    // Input now waits for a thread that retrieves nothing; five seconds
    // later the ghost replaces the window.
    key(c, &["shift"])?;
    wait_after(run, record, stopped, &ghost, 12)?;
    thread::sleep(Duration::from_millis(500));
    let hung = screen(run, c, "f3-hung")?;
    let ghost_pixels = changed(&shown, &hung, caption)?;
    if ghost_pixels < GHOST_TITLE_PIXELS {
        return Err(format!(
            "F3 ghost window not drawn: {ghost_pixels} caption pixels changed"
        ));
    }
    // Alt+F4 on the ghost: the End task prompt. Wait keeps the process.
    let close = serial(&run.root, record)?.len();
    key(c, &["alt", "f4"])?;
    let prompt = format!("{HUNG_PROMPT}{pid} ");
    wait_after(run, record, close, &prompt, 10)?;
    thread::sleep(Duration::from_millis(500));
    let prompted = screen(run, c, "f3-end-task-prompt")?;
    for chord in HUNG_WAIT_KEYS {
        key(c, chord)?;
    }
    wait_after(run, record, close, &format!("{HUNG_WAITED}{pid} "), 10)?;
    thread::sleep(Duration::from_secs(1));
    if processes(&serial(&run.root, record)?, offset)
        .get(&pid)
        .is_some_and(|p| p.exit.is_some())
    {
        return Err("Wait on the hung F3 prompt terminated the process".into());
    }
    // Close again and choose End task.
    let end = serial(&run.root, record)?.len();
    key(c, &["alt", "f4"])?;
    wait_after(run, record, end, &prompt, 10)?;
    for chord in HUNG_END_TASK_KEYS {
        key(c, chord)?;
    }
    wait_after(run, record, end, &format!("{HUNG_ENDED}{pid} "), 10)?;
    let (exit, uid, _) = wait_exit(run, record, c, pid, offset, 20, None)?;
    if exit == 0 || uid != USER {
        return Err(format!(
            "hung F3 was not ended by End task (exit {exit} uid {uid})"
        ));
    }
    if serial(&run.root, record)?
        .get(close..)
        .is_some_and(|t| t.contains("DOOR_P11 CLOSE TIMEOUT revoked"))
    {
        return Err("hung F3 was killed by a close deadline instead of End task".into());
    }
    let released = owner_released(run, record, offset, pid)?;
    Ok(
        json!({"pid":pid,"exit":exit,"uid":uid,"idle_not_hung":true,"ghost_caption_pixels":ghost_pixels,
        "wait_kept_process":true,"end_task":true,"owner_released":released,
        "screenshots":{"shown":keep(run,&shown,"f3-shown")?,"idle":keep(run,&idle,"f3-idle")?,
            "hung":keep(run,&hung,"f3-hung")?,"prompt":keep(run,&prompted,"f3-end-task-prompt")?}}),
    )
}

/// Notes stays usable: activate, type, save, and the save is acknowledged.
fn notes_responsive(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    line: &str,
) -> Result<usize, String> {
    resolution::activate(&run.root, record, c, 1)?;
    key(c, &["ctrl", "end"])?;
    type_line(c, line)?;
    let offset = serial(&run.root, record)?.len();
    key(c, &["ctrl", "s"])?;
    wait_after(run, record, offset, "DOOR_P11 NOTES SAVED", 30)?;
    Ok(serial(&run.root, record)?.len())
}

/// WIN-GUI F3: self-test and rejected initialization from the Terminal, then
/// hung-window containment and an unrelated app that stays responsive.
pub(super) fn f3(run: &mut Run, image: &str, _test_image: &str) -> Result<Value, String> {
    let staged = stage(run, image)?;
    let disk = fresh_disk(run, "win32-f3")?;
    let mut record = boot(run, &staged, &disk)?;
    let result = (|| -> Result<Value, String> {
        let mut c = connection(&record)?;
        open_terminal(run, &record, &mut c)?;
        terminal(run, &record, &mut c, "mkdir wingui-f3")?;
        terminal(run, &record, &mut c, "cd wingui-f3")?;
        let selftest = terminal_run(
            run,
            &record,
            &mut c,
            &staged,
            "wingui-hang",
            "--selftest",
            82,
            Some(F3_RECT),
        )?;
        let reject = terminal_run(
            run,
            &record,
            &mut c,
            &staged,
            "wingui-hang",
            "--reject",
            83,
            None,
        )?;
        terminal(
            run,
            &record,
            &mut c,
            &format!("cp {FIXTURES}/wingui-hang.exe wingui-hang.exe"),
        )?;
        let contained = f3_contain(run, &record, &mut c, &staged, &disk, "wingui-f3")?;
        notes_responsive(
            run,
            &record,
            &mut c,
            "desktop responsive after a hung window",
        )?;
        finish(run, &mut record, &mut c)?;
        Ok(json!({"self_test":selftest,"reject":reject,"containment":contained}))
    })();
    let mut evidence = quit_on_error(run, &record, result, "win32-f3-failure")?;
    let files = disk_files(&disk, true)?;
    let expected = format!(
        "{}HANG_START mode=interactive\r\nHANG_READY window=1\r\nHANG_STOP pumping=0\r\n",
        reference_text(&staged, "f3/wingui-hang.results")?
    );
    if text_of(&files, &format!("{HOME}/wingui-f3/wingui-hang.results"))? != expected {
        return Err(
            "F3 results differ from the Windows reference plus the interactive hang".into(),
        );
    }
    if !text_of(&files, &format!("{HOME}/notes/desktop.txt"))?
        .contains("desktop responsive after a hung window")
    {
        return Err("Notes did not save while the hung window was contained".into());
    }
    evidence["installed"] = require_installed(&files, &staged)?;
    evidence["results"] = json!({"text":expected,"equal_to_reference":true});
    evidence["reference_comparison"] = json!({"state":"passed","reference":gui::REFERENCE,
        "compared":["f3/wingui-hang.results"],"scope":"self-test and rejection records; the interactive hang lines are Door-only"});
    evidence["session"] = record;
    Ok(evidence)
}

/// Scenario 141.6: synchronous messaging and callback reentrancy. The F3
/// self-test keeps a hung top-level window for over five seconds while Notes
/// must save before F3 retires; then F2 and F1 self-tests. Sequence-numbered
/// callback records (EDITOR_ORDER, HANG_PROBE, F1's internal ordered traces)
/// must equal the reviewed Windows reference.
pub(super) fn scenario_141_6(
    run: &mut Run,
    image: &str,
    _test_image: &str,
) -> Result<Value, String> {
    let staged = stage(run, image)?;
    let disk = fresh_disk(run, "win32-141-6")?;
    let mut record = boot(run, &staged, &disk)?;
    let result = (|| -> Result<Value, String> {
        let mut c = connection(&record)?;
        open_terminal(run, &record, &mut c)?;
        terminal(run, &record, &mut c, "mkdir wingui-141-6")?;
        terminal(run, &record, &mut c, "cd wingui-141-6")?;
        let hash = staged.fixtures["wingui-hang"].clone();
        let offset = serial(&run.root, &record)?.len();
        type_line(&mut c, "wingui-hang.exe --selftest")?;
        let pid = wait_launch(run, &record, offset, &hash, "wingui-hang.exe --selftest")?;
        thread::sleep(Duration::from_millis(500));
        let saved_at =
            notes_responsive(run, &record, &mut c, "responsive during a hung recipient")?;
        let (exit, uid, _) = wait_exit(run, &record, &mut c, pid, offset, 60, None)?;
        let retired_at = processes(&serial(&run.root, &record)?, offset)
            .get(&pid)
            .map(|p| p.exit_offset)
            .ok_or("141.6: F3 retirement record missing")?;
        if exit != 82 || uid != USER || saved_at > retired_at {
            return Err(format!(
                "141.6: F3 exit {exit} uid {uid}; Notes saved at {saved_at}, F3 retired at {retired_at}"
            ));
        }
        open_terminal(run, &record, &mut c)?;
        let editor = terminal_run(
            run,
            &record,
            &mut c,
            &staged,
            "wingui-editor",
            "--selftest",
            80,
            None,
        )?;
        let selftest = terminal_run(
            run,
            &record,
            &mut c,
            &staged,
            "wingui-selftest",
            "",
            75,
            None,
        )?;
        finish(run, &mut record, &mut c)?;
        Ok(
            json!({"hung_recipient":{"pid":pid,"exit":exit,"notes_saved_before_retirement":true},"editor":editor,"self_test":selftest}),
        )
    })();
    let mut evidence = quit_on_error(run, &record, result, "win32-141-6-failure")?;
    let files = disk_files(&disk, true)?;
    let folder = format!("{HOME}/wingui-141-6");
    let checks = [
        (
            "wingui-hang.results",
            reference_prefix(&staged, "f3/wingui-hang.results", "HANG_EXIT code=82")?,
        ),
        (
            "wingui-editor.results",
            reference_prefix(
                &staged,
                "f2/wingui-editor.results",
                "EDITOR_EXIT variant=code code=80",
            )?,
        ),
        (
            "wingui-selftest.results",
            reference_text(&staged, "f1/wingui-selftest.results")?,
        ),
    ];
    let mut records = BTreeMap::new();
    for (name, expected) in &checks {
        if text_of(&files, &format!("{folder}/{name}"))? != expected.as_str() {
            return Err(format!(
                "141.6 {name} differs from the reviewed Windows ordering"
            ));
        }
        records.insert(
            *name,
            expected
                .lines()
                .filter(|l| l.contains("_ORDER ") || l.contains("_PROBE "))
                .collect::<Vec<_>>(),
        );
    }
    evidence["callback_records"] = json!(records);
    evidence["reference_comparison"] = json!({"state":"passed","reference":gui::REFERENCE,
        "compared":["f3/wingui-hang.results (through HANG_EXIT code=82)","f2/wingui-editor.results (through EDITOR_EXIT code=80)","f1/wingui-selftest.results"]});
    evidence["session"] = record;
    Ok(evidence)
}

/// Scenario 141.5: the Windows editor beside two native windows (Notes and
/// Explorer): focus switching, maximize/restore/minimize, typing, clipboard
/// both ways (Win32 to Notes, Notes to Win32), a reported grip resize, a
/// window destroyed during a held server-side resize after which a new
/// trusted drag moves and sizes the main window (geometry from screendumps:
/// the content displaced by exactly the pointer delta, the WM_SIZE layout of
/// status and scroll bars), save/relaunch/reopen with the window server
/// releasing every owner resource, and a hung application under the Windows
/// hung semantics while the others work.
pub(super) fn scenario_141_5(
    run: &mut Run,
    image: &str,
    _test_image: &str,
) -> Result<Value, String> {
    let staged = stage(run, image)?;
    let disk = fresh_disk(run, "win32-141-5")?;
    let mut record = boot(run, &staged, &disk)?;
    let document = "Door 141.5 editor\r\nline two";
    let tail = "\r\nafter restore";
    let reopened = " reopened";
    let result = (|| -> Result<Value, String> {
        let mut c = connection(&record)?;
        let mut p = (960, 540);
        notes_responsive(run, &record, &mut c, "native window one")?;
        open_terminal(run, &record, &mut c)?;
        terminal(run, &record, &mut c, "mkdir wingui-141-5")?;
        for fixture in ["wingui-editor", "wingui-hang"] {
            terminal(
                run,
                &record,
                &mut c,
                &format!("cp {FIXTURES}/{fixture}.exe wingui-141-5/{fixture}.exe"),
            )?;
        }
        let (editor, editor_offset) = explorer_run(
            run,
            &record,
            &mut c,
            &staged,
            "wingui-141-5",
            "wingui-editor",
        )?;
        thread::sleep(Duration::from_secs(2));
        type_chars(&mut c, "Door 141.5 editor")?;
        key(&mut c, &["ret"])?;
        type_chars(&mut c, "line two")?;
        key(&mut c, &["ctrl", "s"])?;
        thread::sleep(Duration::from_millis(1500));
        type_line(&mut c, "scenario.txt")?;
        let path = format!("{HOME}/wingui-141-5/scenario.txt");
        wait_file(run, &mut c, &disk, &path, 30, |b| b == document.as_bytes())?;
        // Window management with real shortcuts; screendumps record each state.
        let normal = screen(run, &mut c, "1415-normal")?;
        key(&mut c, &["meta_l", "up"])?;
        thread::sleep(Duration::from_millis(800));
        let maximized = screen(run, &mut c, "1415-maximized")?;
        key(&mut c, &["meta_l", "down"])?;
        thread::sleep(Duration::from_millis(800));
        key(&mut c, &["meta_l", "down"])?;
        thread::sleep(Duration::from_millis(800));
        let minimized = screen(run, &mut c, "1415-minimized")?;
        key(&mut c, &["alt", "tab"])?;
        thread::sleep(Duration::from_millis(800));
        let restored = screen(run, &mut c, "1415-restored")?;
        let states = json!({"maximized_changed":changed(&normal,&maximized,[0,0,1920,1080])?,
            "minimized_changed":changed(&normal,&minimized,F2_RECT)?,"restored_changed":changed(&minimized,&restored,F2_RECT)?});
        let state = |key: &str| states[key].as_u64().unwrap_or(0) as usize;
        if !appeared([0, 0, 1920, 1080], state("maximized_changed"))
            || !appeared(F2_RECT, state("minimized_changed"))
            || !appeared(F2_RECT, state("restored_changed"))
        {
            return Err(format!(
                "141.5 maximize/minimize/restore did not change the screen: {states}"
            ));
        }
        // Focus returned to the editor: typed text reaches its document.
        key(&mut c, &["ctrl", "end"])?;
        key(&mut c, &["ret"])?;
        type_chars(&mut c, "after restore")?;
        key(&mut c, &["ctrl", "s"])?;
        let saved = format!("{document}{tail}");
        wait_file(run, &mut c, &disk, &path, 30, |b| b == saved.as_bytes())?;
        // Clipboard from the Win32 edit control into native Notes...
        key(&mut c, &["ctrl", "a"])?;
        key(&mut c, &["ctrl", "c"])?;
        resolution::activate(&run.root, &record, &mut c, 1)?;
        key(&mut c, &["ctrl", "end"])?;
        key(&mut c, &["ctrl", "v"])?;
        // ...and from Notes into the Win32 editor: a new Notes line selected
        // with Shift+Home and copied.
        key(&mut c, &["ret"])?;
        type_chars(&mut c, NOTES_TO_WIN32)?;
        key(&mut c, &["shift", "home"])?;
        key(&mut c, &["ctrl", "c"])?;
        let notes_offset = serial(&run.root, &record)?.len();
        key(&mut c, &["ctrl", "s"])?;
        wait_after(run, &record, notes_offset, "DOOR_P11 NOTES SAVED", 30)?;
        key(&mut c, &["meta_l", "down"])?;
        click(
            &mut c,
            &mut p,
            F2_RECT[0] + 40,
            F2_RECT[1] + F2_RECT[3] - 40,
        )?;
        key(&mut c, &["ctrl", "end"])?;
        key(&mut c, &["ret"])?;
        key(&mut c, &["ctrl", "v"])?;
        key(&mut c, &["ctrl", "s"])?;
        let pasted = format!("{saved}\r\n{NOTES_TO_WIN32}");
        wait_file(run, &mut c, &disk, &path, 30, |b| b == pasted.as_bytes())?;
        // Custom chrome: a completed grip resize the fixture reports
        // (WM_EXITSIZEMOVE geometry; its client area is the whole window)...
        key(&mut c, &["ctrl", "k"])?;
        thread::sleep(Duration::from_millis(800));
        let grip = (CHROME[0] + CHROME[2] - 6, CHROME[1] + CHROME[3] - 6);
        drag_to(
            &mut c,
            &mut p,
            grip,
            (grip.0 + CHROME_141_GROW.0, grip.1 + CHROME_141_GROW.1),
        )?;
        thread::sleep(Duration::from_millis(500));
        let grown = screen(run, &mut c, "1415-chrome-grown")?;
        // ...then a held grip resize: the server resizes live under the
        // button, and Alt+F4 destroys the window before the button is up.
        let (gw, gh) = (CHROME[2] + CHROME_141_GROW.0, CHROME[3] + CHROME_141_GROW.1);
        let held = (CHROME[0] + gw - 6, CHROME[1] + gh - 6);
        move_to(&mut c, &mut p, held.0, held.1)?;
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":true,"button":"left"}}]}),
        )?;
        thread::sleep(Duration::from_millis(400));
        move_to(
            &mut c,
            &mut p,
            held.0 + CHROME_HELD.0,
            held.1 + CHROME_HELD.1,
        )?;
        thread::sleep(Duration::from_millis(600));
        let dragging = screen(run, &mut c, "1415-chrome-dragging")?;
        let live = require_resized(
            "141.5 held grip did not resize the chrome window live",
            &grown,
            &dragging,
            [CHROME[0], CHROME[1], gw, gh],
            CHROME_HELD,
        )?;
        let close = serial(&run.root, &record)?.len();
        key(&mut c, &["alt", "f4"])?;
        wait_after(
            run,
            &record,
            close,
            &format!("DOOR_P12 CLOSE REQUEST pid={editor} "),
            10,
        )?;
        thread::sleep(Duration::from_millis(800));
        let destroyed = screen(run, &mut c, "1415-chrome-destroyed")?;
        let chrome_area = [CHROME[0], CHROME[1], gw + CHROME_HELD.0, gh + CHROME_HELD.1];
        let gone = changed(&dragging, &destroyed, chrome_area)?;
        if !appeared(chrome_area, gone) {
            return Err("141.5 chrome window survived Alt+F4 during its held resize".into());
        }
        // The button is still down and the pointer keeps moving: with the
        // window gone nothing is dragged, then the button comes up.
        move_to(
            &mut c,
            &mut p,
            held.0 + CHROME_HELD.0 + 30,
            held.1 + CHROME_HELD.1 + 30,
        )?;
        c.execute(
            "input-send-event",
            json!({"events":[{"type":"btn","data":{"down":false,"button":"left"}}]}),
        )?;
        thread::sleep(Duration::from_millis(500));
        let released = screen(run, &mut c, "1415-drag-released")?;
        if appeared(MAIN_NATIVE, changed(&destroyed, &released, MAIN_NATIVE)?) {
            return Err("141.5 the destroyed window's drag went on moving the editor".into());
        }
        if processes(&serial(&run.root, &record)?, editor_offset)
            .get(&editor)
            .is_some_and(|p| p.exit.is_some())
        {
            return Err("141.5 Alt+F4 on the chrome window ended the editor".into());
        }
        // The server released that drag: a new trusted drag (admission is
        // refused while one is held) moves the Win32 main window by its
        // caption, by exactly the pointer delta.
        let caption = (MAIN_NATIVE[0] + 200, MAIN_NATIVE[1] + 16);
        drag_to(
            &mut c,
            &mut p,
            caption,
            (caption.0 + MAIN_MOVE.0, caption.1 + MAIN_MOVE.1),
        )?;
        thread::sleep(Duration::from_millis(600));
        let moved = screen(run, &mut c, "1415-main-moved")?;
        let top = [
            MAIN_NATIVE[0],
            MAIN_NATIVE[1],
            MAIN_NATIVE[2],
            MAIN_NONCLIENT_TOP + 60,
        ];
        let move_check =
            require_displaced("141.5 main window move", &released, &moved, top, MAIN_MOVE)?;
        // Sized from the native bottom-right corner: the fixture's WM_SIZE
        // layout puts the status bar on the new client bottom and the edit
        // control's scroll bar on the new client right edge.
        let at = [
            MAIN_NATIVE[0] + MAIN_MOVE.0,
            MAIN_NATIVE[1] + MAIN_MOVE.1,
            MAIN_NATIVE[2],
            MAIN_NATIVE[3],
        ];
        // Pressed in the 4 px resize band of both edges and inside the 8 px
        // rounded corner, which the window server hit-tests as it paints it
        // (`rounded_contains`): 2 px in from each edge lies outside the
        // curve, so that press went to the Explorer window beneath (run
        // host-1792986422058966200-20148).
        let corner = (at[0] + at[2] - 4, at[1] + at[3] - 4);
        drag_to(
            &mut c,
            &mut p,
            corner,
            (corner.0 + MAIN_GROW.0, corner.1 + MAIN_GROW.1),
        )?;
        thread::sleep(Duration::from_millis(800));
        let sized = screen(run, &mut c, "1415-main-sized")?;
        let status_bar = require_displaced(
            "141.5 client height (status bar on WM_SIZE)",
            &moved,
            &sized,
            [at[0] + 2, at[1] + at[3] - 30, at[2] - 40, 28],
            (0, MAIN_GROW.1),
        )?;
        let scroll_bar = require_displaced(
            "141.5 client width (edit scroll bar on WM_SIZE)",
            &moved,
            &sized,
            [
                at[0] + at[2] - 30,
                at[1] + MAIN_NONCLIENT_TOP + 4,
                28,
                at[3] - MAIN_NONCLIENT_TOP - 44,
            ],
            (MAIN_GROW.0, 0),
        )?;
        let geometry = json!({"moved":move_check,"status_bar":status_bar,"scroll_bar":scroll_bar,
            "native":[at[0],at[1],at[2]+MAIN_GROW.0,at[3]+MAIN_GROW.1],
            "client":[at[2]+MAIN_GROW.0,at[3]+MAIN_GROW.1-MAIN_NONCLIENT_TOP]});
        // Focus is on the editor: close it (its owner fully released) and relaunch.
        key(&mut c, &["alt", "f4"])?;
        let (exit, uid, _) = wait_exit(run, &record, &mut c, editor, editor_offset, 30, None)?;
        if exit != 0 || uid != USER {
            return Err(format!("141.5 editor exit {exit} uid {uid}"));
        }
        let first_released = owner_released(run, &record, editor_offset, editor)?;
        let (again, again_offset) = explorer_run(
            run,
            &record,
            &mut c,
            &staged,
            "wingui-141-5",
            "wingui-editor",
        )?;
        thread::sleep(Duration::from_secs(2));
        key(&mut c, &["ctrl", "o"])?;
        thread::sleep(Duration::from_millis(1500));
        type_line(&mut c, "scenario.txt")?;
        thread::sleep(Duration::from_millis(800));
        key(&mut c, &["ctrl", "end"])?;
        type_chars(&mut c, reopened)?;
        key(&mut c, &["ctrl", "s"])?;
        let last = format!("{pasted}{reopened}");
        wait_file(run, &mut c, &disk, &path, 30, |b| b == last.as_bytes())?;
        key(&mut c, &["alt", "f4"])?;
        let (exit, uid, _) = wait_exit(run, &record, &mut c, again, again_offset, 30, None)?;
        if exit != 0 || uid != USER {
            return Err(format!("141.5 relaunched editor exit {exit} uid {uid}"));
        }
        let second_released = owner_released(run, &record, again_offset, again)?;
        // Hang one application, contain it, and keep using the others.
        let hang = f3_contain(run, &record, &mut c, &staged, &disk, "wingui-141-5")?;
        notes_responsive(run, &record, &mut c, "native window during containment")?;
        finish(run, &mut record, &mut c)?;
        Ok(
            json!({"editor_pids":[editor,again],"window_states":states,"chrome_live_resize":live,"geometry":geometry,"hang":hang,
            "owner_released":{"editor":first_released,"relaunched":second_released},
            "screenshots":{"normal":keep(run,&normal,"1415-normal")?,"maximized":keep(run,&maximized,"1415-maximized")?,
                "minimized":keep(run,&minimized,"1415-minimized")?,"restored":keep(run,&restored,"1415-restored")?,
                "chrome_grown":keep(run,&grown,"1415-chrome-grown")?,"chrome_dragging":keep(run,&dragging,"1415-chrome-dragging")?,
                "chrome_destroyed":keep(run,&destroyed,"1415-chrome-destroyed")?,"drag_released":keep(run,&released,"1415-drag-released")?,
                "main_moved":keep(run,&moved,"1415-main-moved")?,"main_sized":keep(run,&sized,"1415-main-sized")?}}),
        )
    })();
    let mut evidence = quit_on_error(run, &record, result, "win32-141-5-failure")?;
    let files = disk_files(&disk, true)?;
    let final_document = format!("{document}{tail}\r\n{NOTES_TO_WIN32}{reopened}");
    evidence["document"] = require_bytes(
        &files,
        &format!("{HOME}/wingui-141-5/scenario.txt"),
        &sha256(final_document.as_bytes()),
    )?;
    let notes = text_of(&files, &format!("{HOME}/notes/desktop.txt"))?;
    for line in [
        "native window one",
        "Door 141.5 editor",
        "line two",
        NOTES_TO_WIN32,
        "native window during containment",
    ] {
        if !notes.contains(line) {
            return Err(format!(
                "141.5 Notes document lacks {line:?} (clipboard or responsiveness)"
            ));
        }
    }
    let results = text_of(
        &files,
        &format!("{HOME}/wingui-141-5/wingui-editor.results"),
    )?;
    if results.contains("EDITOR_FAIL")
        || !chrome_destroyed_mid_resize(results)
        || results
            .matches("EDITOR_EXIT variant=code code=0\r\n")
            .count()
            != 2
    {
        return Err(
            "141.5 editor results: a failure, chrome records other than open, the reported grip resize and a destruction during the held resize, or an unclean exit"
                .into(),
        );
    }
    evidence["editor_results"] = json!(results);
    evidence["session"] = record;
    Ok(evidence)
}

/// The 141.5 chrome records: opened at CHROME, the completed grip resize
/// reported with its new size, then destroyed by Alt+F4 during the held
/// resize — never by Esc or its close box (at most the aborted loop's own
/// WM_EXITSIZEMOVE record follows).
fn chrome_destroyed_mid_resize(results: &str) -> bool {
    let chrome: Vec<&str> = results
        .lines()
        .filter(|l| l.starts_with("EDITOR_CHROME "))
        .collect();
    let open = format!(
        "EDITOR_CHROME event=open x={} y={} w={} h={}",
        CHROME[0], CHROME[1], CHROME[2], CHROME[3]
    );
    let grown = format!(
        "EDITOR_CHROME event=moved x={} y={} w={} h={}",
        CHROME[0],
        CHROME[1],
        CHROME[2] + CHROME_141_GROW.0,
        CHROME[3] + CHROME_141_GROW.1
    );
    (2..=3).contains(&chrome.len())
        && chrome[0] == open
        && chrome[1] == grown
        && chrome
            .get(2)
            .is_none_or(|l| l.starts_with("EDITOR_CHROME event=moved "))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kernel_records_identify_fixture_processes_and_exits() {
        let serial = "noise\nDOOR_P09 MODULE pid=7 index=0 base=0x140000000 preferred=0x140000000 bytes=10 sha256=aa\n\
            DOOR_P09 MODULE pid=7 index=1 base=0x180000000 preferred=0x180000000 bytes=5 sha256=bb\n\
            DOOR_P09 MODULE pid=8 index=0 base=0x140000000 preferred=0x140000000 bytes=10 sha256=cc\n\
            DOOR_P08 PROCESS manifest=13 pid=7 instance=2 uid=1001 session=1 epoch=1 exit=75\n";
        let (pid, process) = launched(serial, 0, "aa").unwrap();
        assert_eq!(pid, 7);
        assert_eq!(process.modules, ["aa", "bb"]);
        assert_eq!((process.exit, process.uid), (Some(75), Some(1001)));
        assert!(process.exit_offset > process.module_offset);
        assert!(launched(serial, 0, "zz").is_none());
        let after = serial.find("DOOR_P09 MODULE pid=8").unwrap();
        assert!(launched(serial, after, "aa").is_none());
        assert_eq!(processes(serial, 0)[&8].exit, None);
    }
    #[test]
    fn interactive_editor_expectations_follow_the_real_input_script() {
        let text = f2_interactive_expected();
        assert!(text.contains("EDITOR_SAVE result=1 name=editor-guest.txt chars=28 bytes=28 crc="));
        assert!(text.contains("EDITOR_CHROME event=close x=880 y=200 w=360 h=230\r\n"));
        assert!(text.contains("chars=37 bytes=37"));
        assert_eq!(
            text.matches("EDITOR_EXIT variant=code code=0\r\n").count(),
            2
        );
    }
    #[test]
    fn appearance_needs_a_tenth_of_the_window() {
        assert!(appeared(F2_RECT, 640 * 44));
        assert!(!appeared(F2_RECT, 640 * 43));
    }
    /// A 1024x768 screendump: grey desktop, one white window at `at` with a
    /// dark caption band and text-like ink.
    fn frame(at: (i32, i32)) -> scene::Pixels {
        let (w, h) = (1024usize, 768usize);
        let mut bytes = format!("P6\n{w} {h}\n255\n").into_bytes();
        let start = bytes.len();
        bytes.resize(start + w * h * 3, 0x80);
        for y in 0..120 {
            for x in 0..300 {
                let ink = y < 20 || (y % 9 < 2 && x % 7 < 4);
                let i = start + ((at.1 + y) as usize * w + (at.0 + x) as usize) * 3;
                bytes[i..i + 3].fill(if ink { 0x10 } else { 0xff });
            }
        }
        let path = std::env::temp_dir().join(format!(
            "door-win32-frame-{}-{}-{}.ppm",
            std::process::id(),
            at.0,
            at.1
        ));
        fs::write(&path, bytes).unwrap();
        let pixels = scene::Pixels::read(&path).unwrap();
        fs::remove_file(path).unwrap();
        pixels
    }
    #[test]
    fn displacement_proves_exact_window_geometry() {
        let before = frame((100, 100));
        let after = frame((160, 130));
        let window = [100, 100, 300, 120];
        assert!(require_displaced("move", &before, &after, window, (60, 30)).is_ok());
        for wrong in [(0, 0), (61, 30), (60, 0), (-60, -30)] {
            assert!(
                require_displaced("move", &before, &after, window, wrong).is_err(),
                "{wrong:?}"
            );
        }
        // A window that never moved has nothing informative.
        assert_eq!(
            displaced(&before, &before, window, (60, 30)).unwrap(),
            [(0, 0); 2]
        );
    }
    /// A screendump of the fixture's chrome window at `rect` (#f3f3f3 body,
    /// blue caption with title ink, red close box with a white cross, the
    /// server's grey 1 px edge) over a near-white page like Explorer's.
    fn chrome_frame(rect: [i32; 4]) -> scene::Pixels {
        let (w, h) = (1280usize, 720usize);
        let mut bytes = format!("P6\n{w} {h}\n255\n").into_bytes();
        let start = bytes.len();
        bytes.resize(start + w * h * 3, 0);
        let [rx, ry, rw, rh] = rect;
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let (cx, cy) = (x - rx, y - ry);
                let colour = if cx < 0 || cy < 0 || cx >= rw || cy >= rh {
                    [0xf6, 0xf8, 0xf9]
                } else if cx == rw - 1 || cy == rh - 1 {
                    [0xb4, 0xb4, 0xb4]
                } else if cy < CHROME_CAPTION && cx >= rw - CHROME_CLOSE {
                    let (bx, by) = (cx - (rw - CHROME_CLOSE), cy);
                    if (bx - by - 4).abs() < 2 || (bx + by - 36).abs() < 2 {
                        [0xff, 0xff, 0xff]
                    } else {
                        [0xc4, 0x2b, 0x1c]
                    }
                } else if cy < CHROME_CAPTION {
                    if (8..20).contains(&cy) && (10..150).contains(&cx) && cx % 5 < 2 {
                        [0xff, 0xff, 0xff]
                    } else {
                        [0x20, 0x50, 0x90]
                    }
                } else {
                    [0xf3, 0xf3, 0xf3]
                };
                let i = start + (y as usize * w + x as usize) * 3;
                bytes[i..i + 3].copy_from_slice(&colour);
            }
        }
        let path = std::env::temp_dir().join(format!(
            "door-win32-chrome-{}-{}-{}-{}-{}.ppm",
            std::process::id(),
            rx,
            ry,
            rw,
            rh
        ));
        fs::write(&path, bytes).unwrap();
        let pixels = scene::Pixels::read(&path).unwrap();
        fs::remove_file(path).unwrap();
        pixels
    }
    #[test]
    fn live_resize_is_proved_by_edges_not_by_what_the_window_covers() {
        let rect = [760, 120, 350, 220];
        let before = chrome_frame(rect);
        let resized = chrome_frame([760, 120, 390, 250]);
        // The strip the grown window covers barely changes over a light page...
        let strip = [760, 340, 390, 30];
        assert!(!appeared(strip, changed(&before, &resized, strip).unwrap()));
        // ...but its edges and repainted close box prove the resize.
        assert!(require_resized("resize", &before, &resized, rect, (40, 30)).is_ok());
        for (wrong, grow) in [
            (chrome_frame(rect), (40, 30)),
            (chrome_frame([800, 150, 350, 220]), (40, 30)),
            (chrome_frame([760, 120, 391, 250]), (40, 30)),
            (chrome_frame([760, 120, 390, 249]), (40, 30)),
            (chrome_frame([760, 120, 390, 220]), (40, 30)),
            (chrome_frame([760, 120, 390, 250]), (40, 0)),
        ] {
            assert!(
                require_resized("resize", &before, &wrong, rect, grow).is_err(),
                "{grow:?}"
            );
        }
    }
    #[test]
    fn application_rollback_record_is_required_and_complete() {
        let pass = "DOOR_P13 APP_ROLLBACK PASS checkpoints=141 modules=7 runs=31 tls=0 leaks=0\n";
        assert!(require_app_rollback(&format!("noise\n{pass}")).is_ok());
        assert!(require_app_rollback("noise\n").is_err());
        assert!(
            require_app_rollback(&format!("{pass}DOOR_P13 APP_ROLLBACK FAIL modules=7\n")).is_err()
        );
        for bad in [
            "DOOR_P13 APP_ROLLBACK PASS checkpoints=0 modules=7 runs=31 tls=0 leaks=0\n",
            "DOOR_P13 APP_ROLLBACK PASS checkpoints=141 modules=1 runs=31 tls=0 leaks=0\n",
            "DOOR_P13 APP_ROLLBACK PASS checkpoints=141 modules=7 runs=31 tls=0\n",
        ] {
            assert!(require_app_rollback(bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn chrome_records_require_destruction_during_the_held_resize() {
        let open = "EDITOR_CHROME event=open x=760 y=120 w=320 h=200\r\n";
        let grown = "EDITOR_CHROME event=moved x=760 y=120 w=350 h=220\r\n";
        let start = "EDITOR_START variant=code mode=interactive\r\n";
        assert!(chrome_destroyed_mid_resize(&format!(
            "{start}{open}{grown}"
        )));
        assert!(chrome_destroyed_mid_resize(&format!(
            "{start}{open}{grown}EDITOR_CHROME event=moved x=760 y=120 w=390 h=250\r\n"
        )));
        for bad in [
            format!("{start}{open}"),
            format!("{start}{open}EDITOR_CHROME event=moved x=760 y=120 w=320 h=200\r\n"),
            format!("{start}{open}{grown}EDITOR_CHROME event=escape x=760 y=120 w=350 h=220\r\n"),
            format!("{start}{open}{grown}EDITOR_CHROME event=close x=760 y=120 w=350 h=220\r\n"),
        ] {
            assert!(!chrome_destroyed_mid_resize(&bad), "{bad}");
        }
    }
}
