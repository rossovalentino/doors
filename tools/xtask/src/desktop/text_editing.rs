//! TEXT-01 (`cargo xtask test ui --suite text-01`): multilingual editing in
//! the real Notes editor. ASCII through the US layout; Microsoft Pinyin,
//! Microsoft IME (Japanese) and Microsoft IME (Korean) installed in
//! Settings > Time & language > Typing and reached with Win+Space: "nihao" +
//! Space commits 你好; "nihongo", Space, Space opens the candidate list
//! (serial marker plus a screenshot region with ink), 1 picks 日本語 and Enter
//! commits it; "gksrnrdj" composes 한국어. Grapheme navigation over a ZWJ
//! emoji sequence (Japanese IME emoji candidate): Left/Right step over the
//! whole cluster and Backspace deletes one cluster. A screenshot region of
//! the edited Notes line must change. Ctrl+A/C/V growth then takes the
//! document past the storage format's multi-transaction staging and
//! single-indirect thresholds, followed by Ctrl+Z/Y, Ctrl+S, Ctrl+O reopen
//! and a second boot. Every content claim is an independent raw-disk audit
//! of `/users/1001/notes/desktop.txt` (journal replay and ext2 tree walk in
//! `crate::persistence`), never the app's view. Steps synchronise on serial
//! markers (method switches, candidate windows and IME commits are bounded
//! per process), not sleeps. Key paths are pinned by the host test
//! `acceptance_suite_settings_key_paths` (desktop/workspace/tests); the IME key
//! scripts by `text01_scripts_with_the_shipped_dictionaries` (desktop/ime).
use super::*;

/// Settings page ids (`settings_layout::Page as usize`, host-asserted).
const TYPING_PAGE: usize = 48;
/// `door_keyboard_layouts::Preferences` as each keyboard is added (US
/// current): [US, Pinyin], + Japanese, + Korean.
const US_AND_PINYIN: u32 = 0x71;
const WITH_JAPANESE: u32 = 0x871;
const WITH_KOREAN: u32 = 0x9871;
/// Input method ids (`door_keyboard_layouts::methods`) in Win+Space order.
const US: u8 = 1;
const PINYIN: u8 = 7;
const JAPANESE: u8 = 8;
const KOREAN: u8 = 9;
const CYCLE: [u8; 4] = [US, PINYIN, JAPANESE, KOREAN];
/// Every Win+Space switch the suite makes, in order (the window server
/// reports at most 24 method changes per session).
const SWITCHES: [(u8, u8); 8] = [
    (US, PINYIN),
    (PINYIN, US),
    (US, JAPANESE),
    (JAPANESE, US),
    (US, JAPANESE),
    (JAPANESE, US),
    (US, KOREAN),
    (KOREAN, US),
];
const ASCII_LINE: &str = "Door TEXT-01 ASCII line";
/// Typed after the Pinyin commit, back on the US layout.
const AFTER_IME: &str = " ok";
const JAPANESE_LINE: &str = "日本語 ok";
const KOREAN_LINE: &str = "한국어 ok";
/// Man, woman and girl joined by ZWJ: one grapheme cluster of 18 bytes.
const FAMILY: &str = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
/// Ctrl+A, Ctrl+C, Ctrl+End, Ctrl+V doubles the document each round.
const ROUNDS: u32 = 11;
/// Storage format thresholds (`persistent-namespace-v1.md`, staged
/// whole-file replacement; ext2): the owner commits once 24 pages are
/// staged, 12 direct blocks precede the single-indirect block, and staged
/// candidates end at the direct + single-indirect cap.
const PAGE: usize = 4096;
const STAGED_PAGES_PER_TRANSACTION: usize = 24;
const DIRECT_BLOCKS: usize = 12;
const LARGE_REPLACE_BYTES: usize = 4_243_456;

/// The first two lines: ASCII, then the committed Pinyin candidate.
fn seed() -> String {
    format!("{ASCII_LINE}\n你好{AFTER_IME}\n")
}
/// `[<`, the family, `->]`: what the grapheme navigation leaves.
fn grapheme_line() -> String {
    format!("[<{FAMILY}->]")
}
/// The whole multilingual document before the clipboard rounds.
fn document() -> String {
    format!(
        "{}{JAPANESE_LINE}\n{}\n{KOREAN_LINE}\n",
        seed(),
        grapheme_line()
    )
}
/// The document after `rounds` doublings.
fn grown(rounds: u32) -> Vec<u8> {
    document().repeat(1 << rounds).into_bytes()
}
fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn type_text(c: &mut qmp::Client, text: &str) -> Result<(), String> {
    for ch in text.chars() {
        c.execute(
            "send-key",
            json!({"keys":session::keys(ch)?,"hold-time":20}),
        )?;
        thread::sleep(Duration::from_millis(90));
    }
    Ok(())
}
/// Waits for `marker` after serial byte `offset`.
fn after(
    run: &Run,
    record: &Value,
    offset: usize,
    marker: &str,
    seconds: u64,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        if serial(&run.root, record)?
            .get(offset..)
            .unwrap_or("")
            .contains(marker)
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("TEXT-01: {marker} missing"));
        }
        thread::sleep(Duration::from_millis(80));
    }
}
/// IME commits reported so far (`DOOR_P12 IME COMMIT`, one per commit).
fn commits(run: &Run, record: &Value) -> Result<usize, String> {
    Ok(serial(&run.root, record)?
        .matches("DOOR_P12 IME COMMIT ")
        .count())
}
/// Waits until at least `count` IME commits were reported.
fn await_commits(run: &Run, record: &Value, count: usize) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while commits(run, record)? < count {
        if Instant::now() >= deadline {
            return Err(format!("TEXT-01: IME commit {count} missing"));
        }
        thread::sleep(Duration::from_millis(80));
    }
    Ok(())
}
/// Win+Space presses from `from` to `to` with all four methods installed.
fn presses(from: u8, to: u8) -> usize {
    let at = |id| CYCLE.iter().position(|m| *m == id).unwrap_or(0);
    (at(to) + CYCLE.len() - at(from)) % CYCLE.len()
}
/// Win+Space to `to`; the window server reports the method change.
fn switch(run: &Run, record: &Value, c: &mut qmp::Client, from: u8, to: u8) -> Result<(), String> {
    let offset = serial(&run.root, record)?.len();
    for _ in 0..presses(from, to) {
        key(c, &["meta_l", "spc"])?;
    }
    after(
        run,
        record,
        offset,
        &format!("DOOR_P12 TEXT_INPUT METHOD id={to} "),
        10,
    )
}
/// Ctrl+S and its completion marker.
fn save(run: &Run, record: &Value, c: &mut qmp::Client) -> Result<(), String> {
    let offset = serial(&run.root, record)?.len();
    key(c, &["ctrl", "s"])?;
    after(run, record, offset, "DOOR_P11 NOTES SAVED", 60)
}
/// Pauses the guest, copies the disk and audits the copy: the saved bytes
/// as a separate reader finds them, while the session keeps running.
fn checkpoint(
    run: &Run,
    c: &mut qmp::Client,
    disk: &std::path::Path,
    name: &str,
    expected: &[u8],
) -> Result<Value, String> {
    c.execute("stop", json!({}))?;
    let copy = run.directory.join(format!("text01-{name}.raw"));
    let copied = fs::copy(disk, &copy).map_err(|e| e.to_string());
    let resumed = c.execute("cont", json!({}));
    copied?;
    resumed?;
    let mut result = audit(&copy, expected).map_err(|e| format!("TEXT-01 {name}: {e}"))?;
    result["checkpoint"] = json!(name);
    result["content_sha256"] = json!(sha256(expected));
    result["bytes"] = json!(expected.len());
    Ok(result)
}
/// Settings > Time & language > Typing > Add a keyboard: the `option`-th
/// method not yet installed (the page reloads its default focus each time).
fn install(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    option: usize,
    expected: u32,
) -> Result<(), String> {
    resolution::activate(&run.root, record, c, 2)?;
    let offset = serial(&run.root, record)?.len();
    key(c, &["esc"])?;
    key(c, &["ctrl", "f"])?;
    type_line(c, "typing")?;
    key(c, &["esc"])?;
    after(
        run,
        record,
        offset,
        &format!("SETTINGS PAGE id={TYPING_PAGE}"),
        15,
    )?;
    // First focusable: Add a keyboard; its list offers the methods not
    // installed in id order.
    key(c, &["tab"])?;
    key(c, &["ret"])?;
    for _ in 0..option {
        key(c, &["down"])?;
    }
    key(c, &["ret"])?;
    after(
        run,
        record,
        offset,
        &format!("SETTINGS SAVED input_methods={expected}"),
        15,
    )?;
    after(
        run,
        record,
        offset,
        &format!("SETTINGS APPLIED input_methods={expected}"),
        15,
    )
}
/// The candidate window the window server painted: frame and caret anchor
/// (output physical pixels) from the last `DOOR_P12 CANDIDATES SHOWN`
/// after `offset`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shown {
    frame: [i32; 4],
    caret: [i32; 3],
}
fn parse_shown(line: &str) -> Option<Shown> {
    let field = |name: &str| {
        line.split_whitespace()
            .find_map(|w| w.strip_prefix(name)?.strip_prefix('='))
    };
    let number = |name: &str| field(name)?.parse::<i32>().ok();
    let caret: Vec<i32> = field("caret")?
        .split(',')
        .filter_map(|v| v.parse().ok())
        .collect();
    Some(Shown {
        frame: [
            number("x")?,
            number("y")?,
            number("width")?,
            number("height")?,
        ],
        caret: caret.try_into().ok()?,
    })
}
fn candidates_shown(run: &Run, record: &Value, offset: usize) -> Result<Shown, String> {
    after(run, record, offset, "DOOR_P12 CANDIDATES SHOWN ", 10)?;
    serial(&run.root, record)?
        .get(offset..)
        .unwrap_or("")
        .lines()
        .rev()
        .find_map(|l| {
            l.split_once("DOOR_P12 CANDIDATES SHOWN ")
                .and_then(|(_, v)| parse_shown(v))
        })
        .ok_or_else(|| "TEXT-01: unreadable candidate window marker".into())
}
fn screen(run: &Run, c: &mut qmp::Client, name: &str) -> Result<scene::Pixels, String> {
    let ppm = run.directory.join(format!("text01-{name}.ppm"));
    c.execute(
        "screendump",
        json!({"filename":ppm.to_string_lossy().trim_start_matches("\\\\?\\").replace('\\',"/"),"format":"ppm"}),
    )?;
    scene::Pixels::read(&ppm)
}
/// Evidence for a screenshot: PNG path and hash.
fn keep(run: &Run, p: &scene::Pixels, name: &str) -> Result<Value, String> {
    let png = run.directory.join(format!("text01-{name}.png"));
    p.save_png(&png)?;
    Ok(json!({"png":relative(&run.root,&png)?,"sha256":file_hash(&png)?}))
}
/// `r` = [x, y, width, height] clipped to the frame, as pixel coordinates.
fn region(p: &scene::Pixels, r: [i32; 4]) -> Vec<(usize, usize)> {
    let x0 = r[0].clamp(0, p.width as i32) as usize;
    let y0 = r[1].clamp(0, p.height as i32) as usize;
    let x1 = (r[0] + r[2]).clamp(0, p.width as i32) as usize;
    let y1 = (r[1] + r[3]).clamp(0, p.height as i32) as usize;
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .collect()
}
fn distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    (0..3).map(|i| a[i].abs_diff(b[i]) as u32).sum()
}
/// Pixels in `r` that differ from the region's most common colour (its
/// background) by more than 48: glyph and chrome ink.
fn ink(p: &scene::Pixels, r: [i32; 4]) -> Result<usize, String> {
    let points = region(p, r);
    let mut counts = std::collections::HashMap::new();
    for &(x, y) in &points {
        *counts.entry(p.pixel(x, y)?).or_insert(0usize) += 1;
    }
    let background = counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(c, _)| c)
        .ok_or("TEXT-01: empty screenshot region")?;
    let mut n = 0;
    for &(x, y) in &points {
        n += usize::from(distance(p.pixel(x, y)?, background) > 48);
    }
    Ok(n)
}
/// Pixels in `r` that differ between two screenshots.
fn changed(a: &scene::Pixels, b: &scene::Pixels, r: [i32; 4]) -> Result<usize, String> {
    let mut n = 0;
    for (x, y) in region(a, r) {
        n += usize::from(distance(a.pixel(x, y)?, b.pixel(x, y)?) > 48);
    }
    Ok(n)
}
/// Screenshot once `ok` holds for it (presentation trails the paint
/// marker by a frame or two).
fn screen_until(
    run: &Run,
    c: &mut qmp::Client,
    name: &str,
    ok: impl Fn(&scene::Pixels) -> Result<bool, String>,
) -> Result<scene::Pixels, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let p = screen(run, c, name)?;
        if ok(&p)? || Instant::now() >= deadline {
            return Ok(p);
        }
        thread::sleep(Duration::from_millis(100));
    }
}
/// The caret's text line in a screenshot: the composition and what follows
/// it on the same row, clear of the candidate window below.
fn caret_line(s: Shown) -> [i32; 4] {
    let [x, y, h] = s.caret;
    [x - 240, y + 1, 360, (h - 2).max(4)]
}
/// Japanese: nihongo, Space converts, Space opens the list, 1 picks 日本語,
/// Enter commits; the list and the edited line are checked on screen.
fn japanese(run: &Run, record: &Value, c: &mut qmp::Client) -> Result<Value, String> {
    switch(run, record, c, US, JAPANESE)?;
    let committed = commits(run, record)?;
    type_text(c, "nihongo")?;
    key(c, &["spc"])?;
    let offset = serial(&run.root, record)?.len();
    key(c, &["spc"])?;
    let shown = candidates_shown(run, record, offset)?;
    let frame = shown.frame;
    let open = screen_until(run, c, "japanese-candidates", |p| Ok(ink(p, frame)? >= 80))?;
    let list_ink = ink(&open, frame)?;
    if list_ink < 80 {
        return Err(format!(
            "TEXT-01: candidate window {frame:?} shows no text ({list_ink} ink pixels)"
        ));
    }
    let line = caret_line(shown);
    let composing_ink = ink(&open, line)?;
    let candidates = keep(run, &open, "japanese-candidates")?;
    key(c, &["1"])?;
    key(c, &["ret"])?;
    await_commits(run, record, committed + 1)?;
    switch(run, record, c, JAPANESE, US)?;
    type_line(c, AFTER_IME)?;
    // The edited line on screen: the preedit became 日本語 followed by " ok".
    let edited = screen_until(run, c, "japanese-edited", |p| {
        Ok(changed(&open, p, line)? >= 40 && ink(p, line)? >= 40)
    })?;
    let (difference, edited_ink) = (changed(&open, &edited, line)?, ink(&edited, line)?);
    if difference < 40 || edited_ink < 40 {
        return Err(format!(
            "TEXT-01: Notes line {line:?} did not change on screen ({difference} changed, {edited_ink} ink)"
        ));
    }
    Ok(json!({
        "candidate_window":{"frame":frame,"caret":shown.caret,"ink_pixels":list_ink,"screenshot":candidates},
        "visible_edit":{"region":line,"composing_ink":composing_ink,"edited_ink":edited_ink,
            "changed_pixels":difference,"screenshot":keep(run,&edited,"japanese-edited")?},
    }))
}
/// `[`, two ZWJ families from the Japanese IME (Up wraps to the last
/// candidate), `]`, then Left x3, `<`, Right, `-`, Right, Backspace, `>`:
/// only whole-cluster caret steps and deletion leave `[<family->]`. The
/// caret then sits before `]`, so End moves past it before the line break.
fn graphemes(run: &Run, record: &Value, c: &mut qmp::Client) -> Result<(), String> {
    type_text(c, "[")?;
    switch(run, record, c, US, JAPANESE)?;
    for _ in 0..2 {
        let committed = commits(run, record)?;
        type_text(c, "kazoku")?;
        key(c, &["spc"])?;
        key(c, &["up"])?;
        key(c, &["ret"])?;
        await_commits(run, record, committed + 1)?;
    }
    switch(run, record, c, JAPANESE, US)?;
    type_text(c, "]")?;
    for _ in 0..3 {
        key(c, &["left"])?;
    }
    type_text(c, "<")?;
    key(c, &["right"])?;
    type_text(c, "-")?;
    key(c, &["right"])?;
    key(c, &["backspace"])?;
    type_text(c, ">")?;
    key(c, &["end"])?;
    key(c, &["ret"])
}
/// Korean 2-set: gksrnrdj composes 한국어 (three commits); Space commits
/// the last syllable and types a space.
fn korean(run: &Run, record: &Value, c: &mut qmp::Client) -> Result<(), String> {
    switch(run, record, c, US, KOREAN)?;
    let committed = commits(run, record)?;
    type_text(c, "gksrnrdj")?;
    key(c, &["spc"])?;
    await_commits(run, record, committed + 3)?;
    switch(run, record, c, KOREAN, US)?;
    type_line(c, "ok")
}
fn edit(run: &mut Run, record: &Value, disk: &std::path::Path) -> Result<Value, String> {
    let mut c = connection(record)?;
    thread::sleep(Duration::from_secs(2));
    after(run, record, 0, "DOOR_P12 IME READY", 60)?;
    let text = serial(&run.root, record)?;
    let dictionaries = text
        .lines()
        .find_map(|l| {
            l.split_once("DOOR_P12 IME DICTIONARIES ")
                .map(|(_, v)| v.to_string())
        })
        .ok_or("TEXT-01: IME dictionary report missing")?;
    if !dictionaries.contains("pinyin=ready") || !dictionaries.contains("japanese=ready") {
        return Err(format!(
            "TEXT-01: IME dictionaries unavailable: {dictionaries}"
        ));
    }
    // Pinyin is the sixth method offered, then Japanese, then Korean.
    install(run, record, &mut c, 5, US_AND_PINYIN)?;
    install(run, record, &mut c, 5, WITH_JAPANESE)?;
    install(run, record, &mut c, 5, WITH_KOREAN)?;
    // Notes on its default document; the US keyboard is current.
    resolution::activate(&run.root, record, &mut c, 1)?;
    type_line(&mut c, ASCII_LINE)?;
    save(run, record, &mut c)?;
    let ascii = checkpoint(
        run,
        &mut c,
        disk,
        "ascii",
        format!("{ASCII_LINE}\n").as_bytes(),
    )?;
    // Microsoft Pinyin: "nihao" composes, Space commits the first candidate.
    switch(run, record, &mut c, US, PINYIN)?;
    let committed = commits(run, record)?;
    type_text(&mut c, "nihao")?;
    key(&mut c, &["spc"])?;
    await_commits(run, record, committed + 1)?;
    switch(run, record, &mut c, PINYIN, US)?;
    type_line(&mut c, AFTER_IME)?;
    save(run, record, &mut c)?;
    let composed = checkpoint(run, &mut c, disk, "pinyin-commit", seed().as_bytes())?;
    let japanese = japanese(run, record, &mut c)?;
    graphemes(run, record, &mut c)?;
    korean(run, record, &mut c)?;
    save(run, record, &mut c)?;
    let multilingual = checkpoint(run, &mut c, disk, "multilingual", document().as_bytes())?;
    // Clipboard growth through the session clipboard broker. Ctrl+C and
    // Ctrl+V queue in order in Notes (no wait between them).
    for _ in 0..ROUNDS {
        key(&mut c, &["ctrl", "a"])?;
        key(&mut c, &["ctrl", "c"])?;
        key(&mut c, &["ctrl", "end"])?;
        key(&mut c, &["ctrl", "v"])?;
        thread::sleep(Duration::from_millis(700));
    }
    save(run, record, &mut c)?;
    let large = grown(ROUNDS);
    let pasted = checkpoint(run, &mut c, disk, "clipboard-growth", &large)?;
    // Undo the last paste, save; redo it, save.
    key(&mut c, &["ctrl", "z"])?;
    thread::sleep(Duration::from_millis(400));
    save(run, record, &mut c)?;
    let undone = checkpoint(run, &mut c, disk, "undo", &grown(ROUNDS - 1))?;
    key(&mut c, &["ctrl", "y"])?;
    thread::sleep(Duration::from_millis(400));
    save(run, record, &mut c)?;
    let redone = checkpoint(run, &mut c, disk, "redo", &large)?;
    // Reopen from disk, then edit: the save rewrites the whole document,
    // so the audit proves the reopened text was complete.
    let offset = serial(&run.root, record)?.len();
    key(&mut c, &["ctrl", "o"])?;
    after(run, record, offset, "DOOR_P12 NOTES REOPENED", 30)?;
    key(&mut c, &["ctrl", "end"])?;
    type_line(&mut c, "reopened")?;
    save(run, record, &mut c)?;
    let mut expected = large.clone();
    expected.extend_from_slice(b"reopened\n");
    let reopened = checkpoint(run, &mut c, disk, "reopened", &expected)?;
    start_shut_down(&mut c)?;
    wait_serial(
        &run.root,
        record,
        "DOOR_P08 CLEANUP PASS",
        Duration::from_secs(30),
    )?;
    Ok(json!({
        "ascii":ascii,"pinyin_commit":composed,"japanese":japanese,
        "multilingual":multilingual,"clipboard_growth":pasted,"undo":undone,
        "redo":redone,"reopened":reopened,"dictionaries":dictionaries,
        "input_methods":[US_AND_PINYIN,WITH_JAPANESE,WITH_KOREAN],"rounds":ROUNDS,
        "large_bytes":large.len(),
        "storage_thresholds":{"staged_pages":large.len().div_ceil(PAGE),
            "pages_per_staging_transaction":STAGED_PAGES_PER_TRANSACTION,
            "direct_bytes":DIRECT_BLOCKS*PAGE,"undo_bytes":grown(ROUNDS-1).len(),
            "large_replace_cap":LARGE_REPLACE_BYTES},
        "method_switches":SWITCHES.iter().map(|&(a, b)| json!({"from":a,"to":b,"win_space":presses(a, b)})).collect::<Vec<_>>(),
        "graphemes":{"line":grapheme_line(),"cluster_bytes":FAMILY.len(),
            "keys":"[ family family ] Left Left Left < Right - Right Backspace > End"},
    }))
}
/// Second boot: Notes restores the saved document; one more line is saved.
fn reboot(run: &mut Run, record: &Value, previous: &[u8]) -> Result<Value, String> {
    let mut c = connection(record)?;
    thread::sleep(Duration::from_secs(2));
    resolution::activate(&run.root, record, &mut c, 1)?;
    key(&mut c, &["ctrl", "end"])?;
    type_line(&mut c, "second boot")?;
    save(run, record, &mut c)?;
    start_shut_down(&mut c)?;
    wait_serial(
        &run.root,
        record,
        "DOOR_P08 CLEANUP PASS",
        Duration::from_secs(30),
    )?;
    let mut expected = previous.to_vec();
    expected.extend_from_slice(b"second boot\n");
    Ok(json!({"expected_bytes":expected.len(),"expected_sha256":sha256(&expected)}))
}
fn quit_on_error<T>(
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
pub(super) fn suite(run: &mut Run, image: &str, _test_image: &str) -> Result<Value, String> {
    let disk = run.directory.join("text01-user.raw");
    fs::write(&disk, crate::persistence::format::image(false)).map_err(|e| e.to_string())?;
    let mut first = session::launch_desktop_accelerated(run, image, &disk, false, "1920x1080")?;
    let result = edit(run, &first, &disk);
    let mut evidence = quit_on_error(run, &first, result, "text01-failure")?;
    seal_serial(&run.root, &mut first)?;
    let mut saved = grown(ROUNDS);
    saved.extend_from_slice(b"reopened\n");
    let shutdown_audit = audit(&disk, &saved)?;
    let mut second = session::launch_desktop_accelerated(run, image, &disk, false, "1920x1080")?;
    let result = reboot(run, &second, &saved);
    let second_boot = quit_on_error(run, &second, result, "text01-reboot-failure")?;
    seal_serial(&run.root, &mut second)?;
    saved.extend_from_slice(b"second boot\n");
    let final_audit = audit(&disk, &saved)?;
    let text = String::from_utf8(saved).map_err(|_| "TEXT-01 saved bytes are not UTF-8")?;
    let lines = [
        format!("\n你好{AFTER_IME}\n"),
        format!("\n{JAPANESE_LINE}\n"),
        format!("\n{}\n", grapheme_line()),
        format!("\n{KOREAN_LINE}\n"),
    ];
    if lines.iter().any(|l| !text.contains(l.as_str())) || text.starts_with('\u{feff}') {
        return Err("TEXT-01 saved encoding differs (UTF-8 without BOM, LF)".into());
    }
    evidence["shutdown_audit"] = shutdown_audit;
    evidence["second_boot"] = second_boot;
    evidence["final_audit"] = final_audit;
    evidence["encoding"] = json!({"utf8":true,"bom":false,"newline":"LF","nihao_bytes":"e4bda0e5a5bd",
        "family_bytes":sha256(FAMILY.as_bytes())});
    evidence["sessions"] = json!([first, second]);
    evidence["method"] = json!(
        "QMP keyboard input only; every content check reads the raw disk (paused-guest copies and the final image) through journal replay and the ext2 tree, not the editor; on-screen checks are QEMU screendumps of the candidate window frame and the edited Notes line located by the window server's candidate marker"
    );
    evidence["limits"] = json!([
        "On-screen checks count ink and changed pixels in regions; glyph shapes are not compared with a reference",
        "Crash atomicity of the staged multi-transaction save is covered by host tests, not interrupted here",
        "AltGr/dead keys and bidirectional text are not exercised by this suite"
    ]);
    Ok(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text01_documents_cross_the_storage_thresholds_in_utf8() {
        assert_eq!(seed().len(), 34);
        assert!(
            seed()
                .as_bytes()
                .windows(6)
                .any(|w| w == [0xe4, 0xbd, 0xa0, 0xe5, 0xa5, 0xbd])
        );
        assert_eq!(FAMILY.len(), 18);
        assert_eq!(FAMILY.chars().count(), 5, "one cluster of five scalars");
        assert!(document().starts_with(&seed()));
        assert_eq!(document().len(), 84);
        let large = grown(ROUNDS).len();
        assert!(
            large > (STAGED_PAGES_PER_TRANSACTION + 2) * PAGE,
            "the save needs more than one staging transaction"
        );
        assert!(
            grown(ROUNDS - 1).len() > DIRECT_BLOCKS * PAGE,
            "the undo target already needs the indirect block"
        );
        assert!(large <= LARGE_REPLACE_BYTES);
        assert_eq!(large, 2 * grown(ROUNDS - 1).len());
    }
    #[test]
    fn method_switches_follow_win_space_and_fit_the_marker_budget() {
        assert_eq!(presses(US, PINYIN), 1);
        assert_eq!(presses(PINYIN, US), 3);
        assert_eq!(presses(US, JAPANESE), 2);
        assert_eq!(presses(JAPANESE, US), 2);
        assert_eq!(presses(KOREAN, US), 1);
        let total: usize = SWITCHES.iter().map(|&(a, b)| presses(a, b)).sum();
        assert!(total <= 24, "{total} method-change markers");
        // Each switch starts where the previous one ended, from US.
        assert_eq!(SWITCHES[0].0, US);
        assert!(SWITCHES.windows(2).all(|w| w[0].1 == w[1].0));
        assert_eq!(US_AND_PINYIN & 0xfff, 0x71);
        assert_eq!(WITH_KOREAN >> 12 & 0xf, u32::from(KOREAN));
    }
    #[test]
    fn candidate_markers_parse() {
        let s = parse_shown("x=310 y=420 width=168 height=150 items=3 vertical=1 caret=342,392,24")
            .unwrap();
        assert_eq!(s.frame, [310, 420, 168, 150]);
        assert_eq!(s.caret, [342, 392, 24]);
        assert_eq!(caret_line(s), [102, 393, 360, 22]);
        assert!(parse_shown("x=1 y=2 width=3 height=4 caret=1,2").is_none());
    }
}
