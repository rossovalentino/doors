//! P14 DESKTOP-A apps (`cargo xtask test desktop --suite desktop-a-apps`,
//! spec sections 62 and 63, DESKTOP-A): the typed settings registry and the
//! Terminal, on a private image with QMP keyboard and pointer input, across
//! two guest boots, audited from raw disk bytes.
//!
//! Boot 1:
//! - Terminal (restored at its initial placement): a mouse drag selects a
//!   word in the scrollback, a right click copies it, Ctrl+V pastes it into
//!   `write pasted.txt ` and Return writes the file (raw-disk bytes). The
//!   right window edge is dragged 450 px left: the Terminal reports new
//!   columns while the button is still held (live), the grid matches the
//!   window geometry, and a drag over exactly the reflowed rows of a
//!   150-character line selects all 150 characters (it rewrapped). Win+Up
//!   maximizes (grid of the work area), Win+Down restores.
//! - Settings switches the app mode to Dark. Settings commits the key; a
//!   different, already running application process reports the same key
//!   and value live (kernel-prefixed writer pids, no relaunch), and the
//!   Terminal's pixels change. Paused-guest disk copies before and after the
//!   commit locate the per-user REG_DWORD that persists the key.
//!
//! Between boots the host writes an out-of-range value (0xFFFFFFFF) into that
//! registry value of the shut-down image (catalog CRC recomputed, the
//! independent decoder re-validates the image).
//! Boot 2: the registry reports `reset key=<key> reason=corrupt`, a
//! notification is posted (Init's record), and after shutdown the raw disk
//! holds the default again (the pre-change value), never the corrupt one.
use super::*;
use crate::persistence::{catalog, journal, tree};
use door_window::graphics as g;
use std::collections::BTreeMap;

const OUTPUT: (u32, u32) = (1920, 1080);
const HOME: &str = "/users/1001";
/// `WindowOptions` of `desktop/workspace/src/terminal_main.rs`.
const TERMINAL_REQUEST: (u32, u32) = (1128, 632);
/// The live drag of the right edge.
const DRAG: i32 = 450;
const SELECT_WORD: &str = "p14selectproof";
const CORRUPT: u32 = u32::MAX;
/// Byte offset of the ext2 home partition (`persistence::HOME_FIRST` sectors).
const HOME_BYTE: usize = 133_120 * 512;
/// Catalog registry areas (`fs/storage-core/src/persistence/codec.rs`).
const KEY_START: usize = 7296;
const VALUE_START: usize = 10368;
const SLOTS: usize = 24;

fn reflow_line() -> String {
    (0..150)
        .map(|i| char::from(b'0' + (i % 10) as u8))
        .collect()
}

// ---- serial records ------------------------------------------------------

fn fields<'a>(line: &'a str, marker: &str) -> BTreeMap<&'a str, &'a str> {
    line.split_once(marker)
        .map_or("", |(_, rest)| rest)
        .split_whitespace()
        .filter_map(|word| word.split_once('='))
        .collect()
}
/// The kernel's writer prefix of a user-process line.
fn writer(line: &str) -> Option<u64> {
    line.split_once("DOOR_USER pid=")?
        .1
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}
fn number(line: &str, marker: &str, key: &str) -> Result<u64, String> {
    fields(line, marker)
        .get(key)
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| format!("{key} missing: {line}"))
}
/// Complete serial lines after `offset` containing `needle`, with the
/// offset just past each.
fn lines_after(text: &str, offset: usize, needle: &str) -> Vec<(String, usize)> {
    let tail = text.get(offset..).unwrap_or("");
    let mut out = Vec::new();
    let mut end = offset;
    for line in tail.split_inclusive('\n') {
        end += line.len();
        if line.ends_with('\n') && line.contains(needle) {
            out.push((line.trim_end().to_owned(), end));
        }
    }
    out
}
fn wait_line(
    run: &Run,
    record: &Value,
    offset: usize,
    needle: &str,
    seconds: u64,
) -> Result<(String, usize), String> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let text = serial(&run.root, record)?;
        if let Some(found) = lines_after(&text, offset, needle).into_iter().next() {
            return Ok(found);
        }
        if text.contains("KERNEL_PANIC") || text.contains("APP FAILED") {
            return Err(format!("guest failed while waiting for {needle}"));
        }
        if Instant::now() >= deadline {
            return Err(format!("serial record missing: {needle}"));
        }
        thread::sleep(Duration::from_millis(80));
    }
}
fn length(run: &Run, record: &Value) -> Result<usize, String> {
    Ok(serial(&run.root, record)?.len())
}
/// Pids of `APP_LAUNCHED app=<app>` records in `text` before `limit`.
fn launched(text: &str, limit: usize, app: Option<u32>) -> Vec<(u32, u64)> {
    lines_after(
        text.get(..limit).unwrap_or(text),
        0,
        "DOOR_P12 APP_LAUNCHED app=",
    )
    .into_iter()
    .filter_map(|(line, _)| {
        let f = fields(&line, "APP_LAUNCHED");
        let a = f.get("app")?.parse().ok()?;
        let pid = f.get("pid")?.parse().ok()?;
        app.is_none_or(|want| want == a).then_some((a, pid))
    })
    .collect()
}

// ---- real input ------------------------------------------------------------

fn type_text(c: &mut qmp::Client, text: &str) -> Result<(), String> {
    for ch in text.chars() {
        c.execute(
            "send-key",
            json!({"keys":session::keys(ch)?,"hold-time":20}),
        )?;
        thread::sleep(Duration::from_millis(60));
    }
    Ok(())
}
/// Start search; the best match must launch app `app`; returns its pid.
fn start_app(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    query: &str,
    app: u32,
) -> Result<u64, String> {
    let offset = length(run, record)?;
    key(c, &["meta_l"])?;
    thread::sleep(Duration::from_millis(400));
    type_line(c, query)?;
    resolution::wait_activation(&run.root, record, app as usize, offset)?;
    let (line, _) = wait_line(
        run,
        record,
        offset,
        &format!("DOOR_P12 APP_LAUNCHED app={app} pid="),
        10,
    )?;
    number(&line, "APP_LAUNCHED", "pid")
}
/// A Terminal builtin that must succeed.
fn command(run: &Run, record: &Value, c: &mut qmp::Client, line: &str) -> Result<(), String> {
    let offset = length(run, record)?;
    type_line(c, line)?;
    let name = line.split_whitespace().next().ok_or("empty command")?;
    let (result, _) = wait_line(
        run,
        record,
        offset,
        &format!("TERMINAL COMMAND name={name} ok="),
        20,
    )?;
    if !result.contains(&format!("name={name} ok=1")) {
        return Err(format!("Terminal command failed: {line}: {result}"));
    }
    Ok(())
}
/// `clear` reports nothing; give the Terminal a frame to empty the tab.
fn clear(c: &mut qmp::Client) -> Result<(), String> {
    type_line(c, "clear")?;
    thread::sleep(Duration::from_millis(800));
    Ok(())
}
fn press(c: &mut qmp::Client, which: &str, down: bool) -> Result<(), String> {
    c.execute(
        "input-send-event",
        json!({"events":[{"type":"btn","data":{"down":down,"button":which}}]}),
    )?;
    thread::sleep(Duration::from_millis(150));
    Ok(())
}
/// A left-button drag from `from` to `to` (motion while held).
fn drag_select(
    c: &mut qmp::Client,
    p: &mut (i32, i32),
    from: (i32, i32),
    to: (i32, i32),
) -> Result<(), String> {
    move_to(c, p, from.0, from.1)?;
    press(c, "left", true)?;
    move_to(c, p, to.0, to.1)?;
    thread::sleep(Duration::from_millis(200));
    press(c, "left", false)
}

// ---- Terminal geometry ---------------------------------------------------------

/// The Terminal's initial (restored) placement: `graphics::initial_window`.
/// Native apps draw their own captions, so client = outer rectangle.
fn terminal_rect() -> Result<g::Rect, String> {
    let (x, y, w, h) = g::initial_window(OUTPUT, TERMINAL_REQUEST, 0, None);
    g::Rect::new(x, y, w, h).map_err(|e| format!("{e:?}"))
}
fn work_area() -> g::Rect {
    door_window::taskbar::Layout::new(OUTPUT.0, OUTPUT.1).work_area()
}
/// Console grid origin, cell and size of a `width` x `height` client, as
/// `Terminal::grid` (desktop/workspace/src/terminal.rs) lays it out at 100 %
/// text size with default preferences and no Find bar.
struct Grid {
    x: i32,
    top: i32,
    cell: (u32, u32),
    columns: usize,
    rows: usize,
}
fn grid(width: u32, height: u32) -> Grid {
    let cell = door_workspace::terminal_model::Preferences::default().cell();
    let rail = door_workspace::terminal_model::Layout {
        width,
        height,
        rail_hidden: false,
    }
    .rail();
    Grid {
        x: rail as i32 + 18,
        top: 78,
        cell,
        columns: (width.saturating_sub(rail + 42) / cell.0).max(1) as usize,
        rows: (height.saturating_sub(78 + 55) / cell.1) as usize,
    }
}
/// Screen point at the centre of console cell (`row`, `column`).
fn cell(origin: (i32, i32), g: &Grid, row: usize, column: usize) -> (i32, i32) {
    (
        origin.0 + g.x + column as i32 * g.cell.0 as i32 + g.cell.0 as i32 / 2,
        origin.1 + g.top + row as i32 * g.cell.1 as i32 + g.cell.1 as i32 / 2,
    )
}
/// A point on the right resize band of `rect` at mid-height, found by hit
/// testing the real window authority with the Terminal's caption.
fn right_grip(rect: g::Rect) -> Result<(i32, i32), String> {
    let mut engine = g::Engine::new(OUTPUT.0, OUTPUT.1, g::Theme::Light, g::Limits::default())
        .map_err(|e| format!("window authority: {e:?}"))?;
    engine
        .set_work_area(work_area())
        .map_err(|e| format!("work area: {e:?}"))?;
    let owner = g::Owner {
        pid: 7,
        session: 1,
        epoch: 1,
    };
    let id = engine
        .create_client_caption(
            owner,
            rect,
            g::CaptionLayout {
                tabbed: true,
                height: door_workspace::caption_tabs::HEIGHT,
                max_width: 2048,
                side_margin: door_workspace::caption_tabs::LEFT as u32,
            },
        )
        .map_err(|e| format!("Terminal window: {e:?}"))?;
    let y = rect.y() + rect.height() as i32 / 2;
    (rect.right() - 8..rect.right() + 8)
        .rev()
        .map(|x| (x, y))
        .find(|&(x, y)| engine.hit_test(x, y) == Some((id, g::HitRegion::Resize)))
        .ok_or_else(|| "no right resize band on the Terminal window".into())
}
/// The last `TERMINAL RESIZE` after `offset`: (columns, rows).
fn last_resize(text: &str, offset: usize) -> Option<(u64, u64)> {
    lines_after(text, offset, "DOOR_P14 TERMINAL RESIZE columns=")
        .last()
        .and_then(|(line, _)| {
            Some((
                number(line, "TERMINAL RESIZE", "columns").ok()?,
                number(line, "TERMINAL RESIZE", "rows").ok()?,
            ))
        })
}
fn near(observed: (u64, u64), expected: (usize, usize)) -> bool {
    observed.0.abs_diff(expected.0 as u64) <= 1 && observed.1.abs_diff(expected.1 as u64) <= 1
}

// ---- registry on the raw disk ----------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct Dword {
    slot: usize,
    key: u64,
    path: String,
    name: String,
    data: u32,
}
/// Every live REG_DWORD of the catalog and of the registry hive that holds
/// the slots past the catalog's (`door.registry`, the catalog's record
/// layout; independent decoder), and whether the journal is clean.
fn dwords(raw: &[u8]) -> Result<(Vec<Dword>, bool), String> {
    let journal = journal::inspect(raw)?;
    let home = journal.replayed_home(raw)?;
    let tree = tree::inspect(&home)?;
    let catalog = catalog::decode(&tree)?.ok_or("catalog absent")?;
    let b = catalog.bytes;
    let u64_at =
        |b: &[u8], at: usize| u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or([0; 8]));
    // (record bytes, first slot): the catalog's key and value records, then
    // the hive's.
    let hive = tree.files.get(catalog::HIVE).map(|f| f.bytes.as_slice());
    let count = |h: &[u8], at: usize| {
        u32::from_le_bytes(h[at..at + 4].try_into().unwrap_or([0; 4])) as usize
    };
    let mut key_records: Vec<&[u8]> = (0..SLOTS)
        .map(|i| &b[KEY_START + i * 128..KEY_START + (i + 1) * 128])
        .collect();
    let mut value_records: Vec<&[u8]> = (0..SLOTS)
        .map(|i| &b[VALUE_START + i * 192..VALUE_START + (i + 1) * 192])
        .collect();
    if let Some(h) = hive.filter(|h| h.len() >= 64 && &h[..8] == b"DOORREG1") {
        let (keys, values) = (count(h, 12), count(h, 16));
        if h.len() < 64 + keys * 128 + values * 192 {
            return Err("registry hive shorter than its records".into());
        }
        key_records.extend((0..keys).map(|i| &h[64 + i * 128..64 + (i + 1) * 128]));
        let start = 64 + keys * 128;
        value_records.extend((0..values).map(|i| &h[start + i * 192..start + (i + 1) * 192]));
    }
    let mut keys = BTreeMap::new();
    for r in &key_records {
        let id = u64_at(r, 0);
        if id != 0 {
            let name = catalog::name(r, 32, r[23] as usize)?;
            keys.insert(id, (u64_at(r, 8), r[20], name));
        }
    }
    let path = |mut id: u64| -> String {
        let mut parts = Vec::new();
        let mut hive = 0;
        while let Some((parent, root, name)) = keys.get(&id) {
            if !name.is_empty() {
                parts.push(name.clone());
            }
            hive = *root;
            if parts.len() > 32 {
                break;
            }
            id = *parent;
        }
        parts.push(if hive == 2 {
            "HKCU".into()
        } else {
            format!("hive{hive}")
        });
        parts.reverse();
        parts.join("\\")
    };
    let mut out = Vec::new();
    for (slot, r) in value_records.iter().enumerate() {
        let key = u64_at(r, 0);
        if key == 0 || r[9] != 4 || u16::from_le_bytes([r[10], r[11]]) != 4 {
            continue;
        }
        out.push(Dword {
            slot,
            key,
            path: path(key),
            name: catalog::name(r, 16, r[8] as usize)?,
            data: u32::from_le_bytes(r[112..116].try_into().unwrap_or([0; 4])),
        });
    }
    Ok((out, !journal.selected().committed))
}
/// Registry DWORDs of a paused-guest copy of the private image.
fn checkpoint(c: &mut qmp::Client, disk: &std::path::Path) -> Result<Vec<Dword>, String> {
    c.execute("stop", json!({}))?;
    let raw = fs::read(disk).map_err(|e| e.to_string());
    let resumed = c.execute("cont", json!({}));
    let found = raw.and_then(|raw| dwords(&raw).map(|(d, _)| d));
    resumed?;
    found
}
/// The DWORD the commit wrote: the only one new or changed across the
/// commit (holding `value` when the value is numeric).
fn committed(before: &[Dword], after: &[Dword], value: &str) -> Result<Dword, String> {
    let wanted = value.parse::<u32>().ok();
    let changed: Vec<&Dword> = after
        .iter()
        .filter(|d| {
            !before
                .iter()
                .any(|b| b.key == d.key && b.name.eq_ignore_ascii_case(&d.name) && b.data == d.data)
        })
        .filter(|d| wanted.is_none_or(|w| d.data == w))
        .collect();
    match changed.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(format!(
            "the committed value {value} is not a per-user REG_DWORD on disk (the typed registry must persist each key as a DWORD)"
        )),
        many => Err(format!("ambiguous persisted setting: {many:?}")),
    }
}
/// Write `value` into registry DWORD `target` of a shut-down image: the
/// catalog's value bytes and CRC, through its ext2 blocks in the home
/// partition. The image is decoded again before returning.
fn corrupt(disk: &std::path::Path, target: &Dword, value: u32) -> Result<Value, String> {
    let mut raw = fs::read(disk).map_err(|e| e.to_string())?;
    let journal = journal::inspect(&raw)?;
    if journal.selected().committed {
        return Err("cannot corrupt an image with a committed journal".into());
    }
    let home = journal.replayed_home(&raw)?;
    if raw.get(HOME_BYTE..HOME_BYTE + home.len()) != Some(home.as_slice()) {
        return Err("home partition offset differs from the journal geometry".into());
    }
    let tree = tree::inspect(&home)?;
    // Slots past the catalog's live in the registry hive (its own CRC).
    let in_hive = target.slot >= SLOTS;
    let file = if in_hive {
        tree.files
            .get(catalog::HIVE)
            .ok_or("registry hive absent")?
    } else {
        tree.files.get("door.catalog").ok_or("catalog absent")?
    };
    let mut bytes = file.bytes.clone();
    let at = if in_hive {
        let keys = u32::from_le_bytes(bytes[12..16].try_into().unwrap_or([0; 4])) as usize;
        64 + keys * 128 + (target.slot - SLOTS) * 192
    } else {
        VALUE_START + target.slot * 192
    };
    if bytes.len() < at + 192
        || u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap_or([0; 8])) != target.key
        || bytes[at + 9] != 4
    {
        return Err("registry value moved before the corruption".into());
    }
    bytes[at + 112..at + 116].copy_from_slice(&value.to_le_bytes());
    let crc = if in_hive {
        let values = u32::from_le_bytes(bytes[16..20].try_into().unwrap_or([0; 4])) as usize;
        let keys = u32::from_le_bytes(bytes[12..16].try_into().unwrap_or([0; 4])) as usize;
        let crc = crate::storage::audit::crc32(&bytes[64..64 + keys * 128 + values * 192]);
        bytes[20..24].copy_from_slice(&crc.to_le_bytes());
        crc
    } else {
        bytes[40..44].fill(0);
        let crc = crate::storage::audit::crc32(&bytes);
        bytes[40..44].copy_from_slice(&crc.to_le_bytes());
        crc
    };
    for (index, block) in file.blocks.iter().enumerate() {
        let chunk = &bytes[index * 4096..((index + 1) * 4096).min(bytes.len())];
        let start = HOME_BYTE + block * 4096;
        raw[start..start + chunk.len()].copy_from_slice(chunk);
    }
    fs::write(disk, &raw).map_err(|e| e.to_string())?;
    let (now, clean) = dwords(&fs::read(disk).map_err(|e| e.to_string())?)?;
    let patched = now
        .iter()
        .find(|d| d.slot == target.slot && d.key == target.key)
        .ok_or("corrupted value vanished")?;
    if patched.data != value || !clean {
        return Err("corrupted image did not decode with the out-of-range value".into());
    }
    Ok(
        json!({"slot":target.slot,"path":target.path,"name":target.name,"from":target.data,
        "to":value,"catalog_crc":format!("{crc:08x}"),"disk_sha256":file_hash(disk)?}),
    )
}
/// The bytes of `path` in a shut-down image.
fn file_bytes(raw: &[u8], path: &str) -> Result<Vec<u8>, String> {
    let journal = journal::inspect(raw)?;
    if journal.selected().committed {
        return Err("journal not clean after shutdown".into());
    }
    let tree = tree::inspect(&journal.replayed_home(raw)?)?;
    let catalog = catalog::decode(&tree)?.ok_or("catalog absent")?;
    let paths = catalog.paths()?;
    let node = paths.get(path).ok_or_else(|| format!("{path} missing"))?;
    Ok(tree
        .files
        .get(&format!("obj{:016x}", node.id))
        .ok_or_else(|| format!("{path} backing missing"))?
        .bytes
        .clone())
}

// ---- the suite ------------------------------------------------------------------

fn shut_down(run: &Run, record: &mut Value, c: &mut qmp::Client) -> Result<(), String> {
    key(c, &["esc"])?;
    start_shut_down(c)?;
    wait_serial(
        &run.root,
        record,
        "DOOR_P08 CLEANUP PASS",
        Duration::from_secs(60),
    )?;
    let text = serial(&run.root, record)?;
    if !text.contains("leaked=0") || text.contains("APP FAILED") || text.contains("SERVER FAILED") {
        return Err("desktop-a-apps session did not shut down cleanly".into());
    }
    seal_serial(&run.root, record)
}
fn shot(run: &mut Run, c: &mut qmp::Client, name: &str) -> Result<(scene::Pixels, Value), String> {
    let p = win32::screen(run, c, &format!("p14-apps-{name}"))?;
    let kept = win32::keep(run, &p, &format!("p14-apps-{name}"))?;
    Ok((p, kept))
}
/// The persisted setting found in boot 1.
struct Setting {
    key: String,
    value: String,
    dword: Dword,
    /// Its value before the change (the default), when it was on disk.
    initial: Option<u32>,
}
pub(super) fn suite(run: &mut Run, image: &str, _test_image: &str) -> Result<Value, String> {
    let disk = win32::fresh_disk(run, "p14-apps")?;
    let mut first =
        session::launch_desktop_accelerated(run, image, &disk, false, win32::RESOLUTION)?;
    thread::sleep(Duration::from_secs(2));
    let result = boot_one(run, &mut first, &disk);
    let (terminal, setting, live) =
        win32::quit_on_error(run, &first, result, "p14-apps-first-failure")?;
    let raw = fs::read(&disk).map_err(|e| e.to_string())?;
    if file_bytes(&raw, &format!("{HOME}/pasted.txt"))? != format!("{SELECT_WORD}\n").as_bytes() {
        return Err("pasted.txt does not hold the pasted selection".into());
    }
    let (saved, clean) = dwords(&raw)?;
    let saved = saved
        .into_iter()
        .find(|d| d.slot == setting.dword.slot && d.key == setting.dword.key)
        .ok_or("the committed setting is not on the shut-down disk")?;
    if !clean || saved.data != setting.dword.data {
        return Err(format!("persisted setting changed by shutdown: {saved:?}"));
    }
    if serial(&run.root, &first)?.contains("DOOR_P14 SETTINGS reset ") {
        return Err("a valid configuration was reset".into());
    }
    let corrupted = corrupt(&disk, &setting.dword, CORRUPT)?;
    let mut second =
        session::launch_desktop_accelerated(run, image, &disk, false, win32::RESOLUTION)?;
    thread::sleep(Duration::from_secs(2));
    let result = boot_two(run, &mut second, &setting.key);
    let recovered = win32::quit_on_error(run, &second, result, "p14-apps-second-failure")?;
    let (after, clean) = dwords(&fs::read(&disk).map_err(|e| e.to_string())?)?;
    let rewritten = after
        .iter()
        .find(|d| d.key == setting.dword.key && d.name.eq_ignore_ascii_case(&setting.dword.name))
        .ok_or("the reset setting was not rewritten on disk")?;
    if !clean
        || rewritten.data == CORRUPT
        || rewritten.data == setting.dword.data
        || setting.initial.is_some_and(|d| d != rewritten.data)
    {
        return Err(format!(
            "reset did not rewrite the default (was {:?}, committed {}): {rewritten:?}",
            setting.initial, setting.dword.data
        ));
    }
    Ok(json!({
        "terminal": terminal,
        "settings": {
            "key": setting.key, "value": setting.value, "live": live,
            "persisted": {"slot":setting.dword.slot,"path":setting.dword.path,"name":setting.dword.name,
                "data":setting.dword.data,"initial":setting.initial},
            "corrupted": corrupted,
            "recovered": recovered,
            "rewritten": rewritten.data,
        },
        "disk": relative(&run.root, &disk)?,
        "disk_sha256": file_hash(&disk)?,
        "sessions": [first, second],
        "separate_boots": 2,
        "reference_comparison": {"state":"not_applicable",
            "scope":"Settings propagation/recovery and Terminal selection, clipboard and resize behavior with raw-disk audits; visual fidelity is VIS-01"},
    }))
}
fn boot_one(
    run: &mut Run,
    record: &mut Value,
    disk: &std::path::Path,
) -> Result<(Value, Setting, Value), String> {
    let mut c = connection(record)?;
    let mut p = (960, 540);
    super::p14_shell::require_unit_scale(run, record)?;
    let terminal_pid = start_app(run, record, &mut c, "terminal", 9)?;
    let terminal = terminal_checks(run, record, &mut c, &mut p)?;
    let (light, light_png) = shot(run, &mut c, "terminal-light")?;
    // Settings changes the app mode; another running app observes it.
    let before = checkpoint(&mut c, disk)?;
    let settings_pid = start_app(run, record, &mut c, "settings", 3)?;
    let offset = length(run, record)?;
    settings_mode(&mut c, true)?;
    let (commit, past) = wait_line(run, record, offset, "DOOR_P14 SETTINGS commit key=", 20)?;
    let f = fields(&commit, "SETTINGS commit");
    let key_name = f.get("key").ok_or("commit key")?.to_string();
    let value = f.get("value").ok_or("commit value")?.to_string();
    let committer = writer(&commit).ok_or("commit writer pid")?;
    let (observed, _) = wait_line(
        run,
        record,
        past,
        &format!("DOOR_P14 SETTINGS observed key={key_name} value={value}"),
        20,
    )?;
    let observer = writer(&observed).ok_or("observed writer pid")?;
    let text = serial(&run.root, record)?;
    let running = launched(&text, offset, None);
    if observer == committer || !running.iter().any(|(_, pid)| *pid == observer) {
        return Err(format!(
            "{key_name} was not observed by another running application (commit pid {committer}, observer {observer})"
        ));
    }
    if launched(&text, text.len(), Some(9)).len() != 1 {
        return Err("the Terminal was relaunched instead of observing the change".into());
    }
    thread::sleep(Duration::from_millis(500));
    let after = checkpoint(&mut c, disk)?;
    let dword = committed(&before, &after, &value)?;
    let initial = before
        .iter()
        .find(|d| d.key == dword.key && d.name.eq_ignore_ascii_case(&dword.name))
        .map(|d| d.data);
    // The Terminal repainted with the new mode (Alt+Tab, most recent first).
    key(&mut c, &["alt", "tab"])?;
    thread::sleep(Duration::from_millis(1200));
    let (dark, dark_png) = shot(run, &mut c, "terminal-dark")?;
    let rect = terminal_rect()?;
    let area = [
        rect.x(),
        rect.y(),
        rect.width() as i32 - DRAG,
        rect.height() as i32,
    ];
    let changed = win32::changed(&light, &dark, area)?;
    if changed < (area[2] * area[3]) as usize / 10 {
        return Err(format!(
            "the running Terminal did not repaint for {key_name}={value} ({changed} px)"
        ));
    }
    shut_down(run, record, &mut c)?;
    let observer_app = running
        .iter()
        .find(|(_, pid)| *pid == observer)
        .map(|(app, _)| *app);
    let live = json!({"commit":commit,"observed":observed,"committer_pid":committer,
        "settings_pid":settings_pid,"observer_pid":observer,"observer_app":observer_app,
        "terminal_pid":terminal_pid,"terminal_changed_pixels":changed,
        "screens":{"light":light_png,"dark":dark_png}});
    Ok((
        terminal,
        Setting {
            key: key_name,
            value,
            dword,
            initial,
        },
        live,
    ))
}
/// Mouse selection, clipboard copy and paste, live resize and reflow.
fn terminal_checks(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    p: &mut (i32, i32),
) -> Result<Value, String> {
    let rect = terminal_rect()?;
    let origin = (rect.x(), rect.y());
    let restored = grid(rect.width(), rect.height());
    // 1. Drag-select the echoed word (row 1, after the prompt row).
    clear(c)?;
    command(run, record, c, &format!("echo {SELECT_WORD}"))?;
    let offset = length(run, record)?;
    drag_select(
        c,
        p,
        cell(origin, &restored, 1, 0),
        cell(origin, &restored, 1, 30),
    )?;
    let (selected, _) = wait_line(run, record, offset, "DOOR_P14 TERMINAL SELECT chars=", 10)?;
    if number(&selected, "TERMINAL SELECT", "chars")? != SELECT_WORD.len() as u64 {
        return Err(format!("drag selected the wrong text: {selected}"));
    }
    let (_, selection_png) = shot(run, c, "terminal-selection")?;
    // 2. A right click on the selection copies it.
    let offset = length(run, record)?;
    let (x, y) = cell(origin, &restored, 1, 3);
    move_to(c, p, x, y)?;
    press(c, "right", true)?;
    press(c, "right", false)?;
    let (copied, _) = wait_line(run, record, offset, "DOOR_P14 TERMINAL COPY bytes=", 10)?;
    if number(&copied, "TERMINAL COPY", "bytes")? != SELECT_WORD.len() as u64 {
        return Err(format!(
            "copy put the wrong bytes on the clipboard: {copied}"
        ));
    }
    // 3. Ctrl+V pastes it into a command line that writes a file.
    type_text(c, "write pasted.txt ")?;
    let offset = length(run, record)?;
    key(c, &["ctrl", "v"])?;
    let (pasted, _) = wait_line(run, record, offset, "DOOR_P14 TERMINAL PASTE bytes=", 10)?;
    if number(&pasted, "TERMINAL PASTE", "bytes")? != SELECT_WORD.len() as u64 {
        return Err(format!("paste inserted the wrong bytes: {pasted}"));
    }
    let offset = length(run, record)?;
    key(c, &["ret"])?;
    let (written, _) = wait_line(run, record, offset, "TERMINAL COMMAND name=write ok=", 20)?;
    if !written.contains("ok=1") {
        return Err(format!("pasted command failed: {written}"));
    }
    // 4. Live resize: drag the right edge 450 px left in steps.
    clear(c)?;
    let line = reflow_line();
    command(run, record, c, &format!("echo {line}"))?;
    let grip = right_grip(rect)?;
    let narrow = grid(rect.width() - DRAG as u32, rect.height());
    let offset = length(run, record)?;
    move_to(c, p, grip.0, grip.1)?;
    press(c, "left", true)?;
    move_to(c, p, grip.0 - DRAG / 3, grip.1)?;
    let held = wait_line(run, record, offset, "DOOR_P14 TERMINAL RESIZE columns=", 15);
    move_to(c, p, grip.0 - DRAG, grip.1)?;
    thread::sleep(Duration::from_millis(400));
    press(c, "left", false)?;
    let (live, _) = held?;
    if number(&live, "TERMINAL RESIZE", "columns")? >= restored.columns as u64 {
        return Err(format!("no narrower grid while the edge was held: {live}"));
    }
    thread::sleep(Duration::from_secs(1));
    let text = serial(&run.root, record)?;
    let resized = last_resize(&text, offset).ok_or("no RESIZE after the drag")?;
    if !near(resized, (narrow.columns, narrow.rows)) {
        return Err(format!(
            "grid {resized:?} does not match the {} px window ({}x{})",
            rect.width() as i32 - DRAG,
            narrow.columns,
            narrow.rows
        ));
    }
    // 5. Reflow: select exactly the rows the 150-character output now
    //    occupies at the new width; all of it (and nothing else) is selected.
    let columns = resized.0 as usize;
    let prompt = format!("{HOME}> echo {line}").chars().count();
    let base = prompt.div_ceil(columns);
    let rows = line.len().div_ceil(columns);
    if base + rows > narrow.rows {
        return Err("reflowed line does not fit the visible rows".into());
    }
    let offset = length(run, record)?;
    drag_select(
        c,
        p,
        cell(origin, &narrow, base, 0),
        cell(origin, &narrow, base + rows - 1, columns - 1),
    )?;
    let (reflowed, _) = wait_line(run, record, offset, "DOOR_P14 TERMINAL SELECT chars=", 10)?;
    if number(&reflowed, "TERMINAL SELECT", "chars")? != line.len() as u64 {
        return Err(format!(
            "the line did not rewrap to {columns} columns: {reflowed}"
        ));
    }
    let (_, reflow_png) = shot(run, c, "terminal-reflowed")?;
    key(c, &["esc"])?;
    // 6. Maximize and restore.
    let work = work_area();
    let maximized = grid(work.width(), work.height());
    let offset = length(run, record)?;
    key(c, &["meta_l", "up"])?;
    wait_line(run, record, offset, "DOOR_P14 TERMINAL RESIZE columns=", 15)?;
    thread::sleep(Duration::from_secs(1));
    let text = serial(&run.root, record)?;
    let max = last_resize(&text, offset).ok_or("no RESIZE after maximize")?;
    if !near(max, (maximized.columns, maximized.rows)) {
        return Err(format!(
            "maximized grid {max:?}, expected {}x{}",
            maximized.columns, maximized.rows
        ));
    }
    let offset = length(run, record)?;
    key(c, &["meta_l", "down"])?;
    wait_line(run, record, offset, "DOOR_P14 TERMINAL RESIZE columns=", 15)?;
    thread::sleep(Duration::from_secs(1));
    let text = serial(&run.root, record)?;
    let back = last_resize(&text, offset).ok_or("no RESIZE after restore")?;
    if back != resized {
        return Err(format!(
            "restore did not return to the dragged grid: {back:?}"
        ));
    }
    Ok(json!({
        "selection": selected, "copy": copied, "paste": pasted, "write": written,
        "live_resize": live, "resized": [resized.0, resized.1],
        "expected_resized": [narrow.columns, narrow.rows],
        "reflow": {"columns": columns, "prompt_rows": base, "line_rows": rows, "select": reflowed},
        "maximized": [max.0, max.1], "expected_maximized": [maximized.columns, maximized.rows],
        "restored": [back.0, back.1],
        "screens": {"selection": selection_png, "reflow": reflow_png},
    }))
}
fn boot_two(run: &mut Run, record: &mut Value, key_name: &str) -> Result<Value, String> {
    let mut c = connection(record)?;
    super::p14_shell::require_unit_scale(run, record)?;
    start_app(run, record, &mut c, "settings", 3)?;
    let (reset, past) = wait_line(run, record, 0, "DOOR_P14 SETTINGS reset key=", 60)?;
    let f = fields(&reset, "SETTINGS reset");
    if f.get("key") != Some(&key_name) || f.get("reason") != Some(&"corrupt") {
        return Err(format!(
            "expected a corrupt-value reset of {key_name}: {reset}"
        ));
    }
    // The reset's toast: a posted notification (Init's NOTIFY BIND at
    // sign-in is the service's configuration, not a notification).
    let posted = |text: &str| {
        text.lines()
            .find(|l| {
                l.contains("DOOR_P14 NOTIFY POST ") || l.contains("DOOR_P14 NOTIFY SYSTEM id=")
            })
            .map(str::to_owned)
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    let notice = loop {
        let text = serial(&run.root, record)?;
        if let Some(line) = posted(&text) {
            break line;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "no notification for the reset (after serial offset {past})"
            ));
        }
        thread::sleep(Duration::from_millis(200));
    };
    thread::sleep(Duration::from_secs(1));
    let (_, png) = shot(run, &mut c, "reset-notice")?;
    let resets = lines_after(
        &serial(&run.root, record)?,
        0,
        "DOOR_P14 SETTINGS reset key=",
    )
    .len();
    shut_down(run, record, &mut c)?;
    Ok(json!({"reset":reset,"resets":resets,"notice":notice,"screen":png}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_grid_follows_the_window_geometry() {
        let rect = terminal_rect().unwrap();
        assert_eq!((rect.width(), rect.height()), TERMINAL_REQUEST);
        let restored = grid(rect.width(), rect.height());
        assert_eq!((restored.columns, restored.rows), (97, 23));
        let narrow = grid(rect.width() - DRAG as u32, rect.height());
        assert_eq!(narrow.columns, 47);
        // The narrow window keeps the full rail (no compact layout).
        assert_eq!(narrow.x, restored.x);
        let prompt = format!("{HOME}> echo {}", reflow_line()).chars().count();
        assert!(prompt.div_ceil(narrow.columns) + 150usize.div_ceil(narrow.columns) <= narrow.rows);
        // The last selected cell stays inside the console.
        let (x, _) = cell((0, 0), &narrow, 0, narrow.columns - 1);
        assert!(x < rect.width() as i32 - DRAG - 18);
        let work = work_area();
        let max = grid(work.width(), work.height());
        assert!(max.columns > restored.columns && max.rows > restored.rows);
    }
    #[test]
    fn the_resize_grip_is_on_the_terminal_edge() {
        let rect = terminal_rect().unwrap();
        let (x, y) = right_grip(rect).unwrap();
        assert!((rect.right() - 8..rect.right() + 8).contains(&x));
        assert_eq!(y, rect.y() + rect.height() as i32 / 2);
    }
    #[test]
    fn serial_parsing() {
        let line = "DOOR_USER pid=33 DOOR_P14 SETTINGS commit key=theme value=1";
        assert_eq!(writer(line), Some(33));
        assert_eq!(fields(line, "SETTINGS commit").get("key"), Some(&"theme"));
        let text = "DOOR_USER pid=1 DOOR_P12 APP_LAUNCHED app=9 pid=40\nDOOR_USER pid=40 DOOR_P14 TERMINAL RESIZE columns=47 rows=23\npartial";
        assert_eq!(launched(text, text.len(), Some(9)), vec![(9, 40)]);
        assert_eq!(last_resize(text, 0), Some((47, 23)));
        assert!(lines_after(text, 0, "partial").is_empty());
    }
    #[test]
    fn committed_setting_is_the_single_changed_dword() {
        let d = |slot, data| Dword {
            slot,
            key: 5,
            path: "HKCU\\Software".into(),
            name: format!("v{slot}"),
            data,
        };
        let before = vec![d(0, 3)];
        assert_eq!(
            committed(&before, &[d(0, 3), d(1, 1)], "1").unwrap().slot,
            1
        );
        assert!(committed(&before, &[d(0, 3)], "1").is_err());
        assert!(committed(&before, &[d(1, 1), d(2, 1)], "1").is_err());
        assert_eq!(committed(&before, &[d(0, 4)], "dark").unwrap().data, 4);
    }
    #[test]
    fn select_word_and_reflow_line_are_typeable() {
        for ch in format!("write pasted.txt echo {SELECT_WORD} {}", reflow_line()).chars() {
            session::keys(ch).unwrap();
        }
    }
}
