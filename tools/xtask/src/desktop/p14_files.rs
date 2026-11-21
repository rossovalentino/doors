//! P14 DESKTOP-A files (`cargo xtask test desktop --suite desktop-a-files`,
//! spec section 61 and DESKTOP-A): the real File Explorer drives the
//! `door-file-ops` engine with QMP keyboard and pointer input on a private
//! image, three separate guest boots, and every result is audited from the
//! raw disk bytes independently of any application report.
//!
//! The image variant adds one immutable 4 MiB seed, `C:\ProgramData\
//! p14-large.bin` (deterministic bytes the host regenerates). The Terminal
//! builds the test tree under `/users/1001/p14` with its builtins (`mkdir`,
//! `write`, `cp`), then Explorer, with Windows keyboard conventions:
//!
//! 1. Ctrl+A, Ctrl+C, Ctrl+V copies two files and a folder tree; Ctrl+X,
//!    Ctrl+V moves the copies (files and the tree) to another folder.
//! 2. Conflicts: "Let me decide for each file", then per item Replace
//!    (Alt+R), Skip (Alt+S) and Keep both (Alt+K, "name (2).ext"); then
//!    "Do this for the next N conflicts" (Alt+D) with Keep both.
//! 3. A copy of two small files and the large one is cancelled (Esc in its
//!    progress dialog) while the large file is mid-copy: the guest is paused
//!    by QMP between engine steps until the durable `DOORFOP1` record on disk
//!    shows the large item in progress, so the cancel lands mid-file.
//! 4. Delete recycles a file; the Recycle Bin (`shell:RecycleBinFolder`)
//!    restores it (Shift+F10, first command Restore); a second file stays
//!    recycled. Shift+Delete asks first (Esc keeps the file; the safe default
//!    is No, so Tab, Enter answers Yes) and then purges.
//! 5. Ctrl+N opens a second Explorer window on the same folder (both
//!    snapped); creating a folder in the first repaints the second.
//! 6. Clean shutdown, then the raw-disk audit: copied and moved bytes, the
//!    conflict outcomes, the cancelled copy (finished items present, partial
//!    item absent), the `$R`/`$I` pair of the recycled item (original path,
//!    deletion time against the catalog's own clock) in the hidden
//!    per-user bin, the restored item, the purged item, no `~$fop*.tmp`
//!    temporary and no `/state/machine/fileops/op*` record.
//! 7. Second boot on the same image: Explorer lists the restored item, the
//!    moved files and the bin entry; a copy of the large file is interrupted
//!    by a hard power-off while the record shows it mid-copy (audited).
//! 8. Third boot: Explorer brings the interrupted operation back Paused;
//!    Resume (Enter) completes it; clean shutdown and the full audit again.
use super::*;
use crate::persistence::{catalog, journal, tree};
use std::collections::BTreeMap;

const UID: u64 = 1001;
const ROOT: &str = "/users/1001/p14";
const BIN: &str = "/users/1001/$Recycle.Bin";
const RECORDS: &str = "/state/machine/fileops";
/// The seed installed by the image variant (`install.manifest`).
const SEED: &str = "/compat/ProgramData/p14-large.bin";
/// The seed as the Terminal's Doorsh names it (Doorsh addresses Windows
/// paths only; C:\ProgramData is machine data every user reads).
const SEED_WINDOWS: &str = "C:/ProgramData/p14-large.bin";
const SEED_INITFS: &str = "seed/p14-large.bin";
/// Below the ext2 single-indirect file limit ((12 + 1024) x 4 KiB) and the
/// engine's 64 KiB step: 64 durable steps.
const LARGE: usize = 4 * 1024 * 1024;
const _: () = assert!(LARGE <= (12 + 1024) * 4096);
/// Test folders below `ROOT` ("" is `ROOT`), parents first.
const FOLDERS: [&str; 16] = [
    "",
    "src",
    "src/tree",
    "src/tree/sub",
    "dst",
    "moved",
    "csrc",
    "cdst",
    "asrc",
    "adst",
    "big",
    "cancel",
    "interrupted",
    "recycle",
    "purge",
    "watch",
];
/// Small files below `ROOT`: Terminal `write PATH TEXT` stores `TEXT\n`.
const SMALL: [(&str, &str); 22] = [
    ("src/a.txt", "alpha copy proof"),
    ("src/b.txt", "beta copy proof"),
    ("src/tree/one.txt", "tree level one"),
    ("src/tree/sub/two.txt", "tree level two"),
    ("csrc/x1.txt", "new x1"),
    ("csrc/x2.txt", "new x2"),
    ("csrc/x3.txt", "new x3"),
    ("cdst/x1.txt", "old x1"),
    ("cdst/x2.txt", "old x2"),
    ("cdst/x3.txt", "old x3"),
    ("asrc/y1.txt", "new y1"),
    ("asrc/y2.txt", "new y2"),
    ("asrc/y3.txt", "new y3"),
    ("adst/y1.txt", "old y1"),
    ("adst/y2.txt", "old y2"),
    ("adst/y3.txt", "old y3"),
    ("big/a-small.txt", "small a before the large file"),
    ("big/b-small.txt", "small b before the large file"),
    ("recycle/r1.txt", "recycled then restored"),
    ("recycle/r2.txt", "recycled and kept in the bin"),
    ("purge/p.txt", "permanently deleted"),
    ("watch/w.txt", "watched folder"),
];
/// `big/` in Explorer's name order: the large file is processed last.
const BIG: [&str; 3] = ["a-small.txt", "b-small.txt", "z-large.bin"];
/// Moved items below `moved/` (copied from `src/` first).
const MOVED: [(&str, &str); 4] = [
    ("a.txt", "src/a.txt"),
    ("b.txt", "src/b.txt"),
    ("tree/one.txt", "src/tree/one.txt"),
    ("tree/sub/two.txt", "src/tree/sub/two.txt"),
];
/// Per-item conflict decisions of the first conflict operation.
const DECISIONS: [(&str, &str, &str); 3] = [
    ("x1.txt", "r", "replace"),
    ("x2.txt", "s", "skip"),
    ("x3.txt", "k", "keepboth"),
];
/// One FILETIME second.
const SECOND: u64 = 10_000_000;

/// Deterministic seed bytes (xorshift64), regenerated for the audits.
fn large_seed() -> Vec<u8> {
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut out = Vec::with_capacity(LARGE);
    while out.len() < LARGE {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}
fn small(path: &str) -> Result<Vec<u8>, String> {
    SMALL
        .iter()
        .find(|(p, _)| *p == path)
        .map(|(_, text)| format!("{text}\n").into_bytes())
        .ok_or_else(|| format!("no small fixture {path}"))
}
fn big(name: &str) -> Result<Vec<u8>, String> {
    if name == BIG[2] {
        Ok(large_seed())
    } else {
        small(&format!("big/{name}"))
    }
}
fn under(rel: &str) -> String {
    if rel.is_empty() {
        ROOT.to_owned()
    } else {
        format!("{ROOT}/{rel}")
    }
}

// ---- serial records ------------------------------------------------------

/// `key=value` words of `line` after `marker`.
fn fields<'a>(line: &'a str, marker: &str) -> BTreeMap<&'a str, &'a str> {
    line.split_once(marker)
        .map_or("", |(_, rest)| rest)
        .split_whitespace()
        .filter_map(|word| word.split_once('='))
        .collect()
}
fn ratio(value: Option<&&str>) -> Option<(u64, u64)> {
    let (a, b) = value?.split_once('/')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}
/// An operation or recycle ID as the serial spells it: decimal, else hex.
fn ids(token: &str) -> Vec<u64> {
    [token.parse().ok(), u64::from_str_radix(token, 16).ok()]
        .into_iter()
        .flatten()
        .collect()
}
/// First complete serial line after `offset` containing any of `needles`:
/// (which needle, the line, the offset just past it).
fn wait_any(
    run: &Run,
    record: &Value,
    offset: usize,
    needles: &[&str],
    seconds: u64,
) -> Result<(usize, String, usize), String> {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let text = serial(&run.root, record)?;
        let tail = text.get(offset..).unwrap_or("");
        let mut start = 0;
        for line in tail.split_inclusive('\n') {
            let end = start + line.len();
            if !line.ends_with('\n') {
                break;
            }
            if let Some(which) = needles.iter().position(|n| line.contains(n)) {
                return Ok((which, line.trim_end().to_owned(), offset + end));
            }
            start = end;
        }
        if text.contains("KERNEL_PANIC") || text.contains("APP FAILED") {
            return Err(format!("guest failed while waiting for {needles:?}"));
        }
        if Instant::now() >= deadline {
            return Err(format!("serial record missing: {needles:?}"));
        }
        thread::sleep(Duration::from_millis(80));
    }
}
fn wait_line(
    run: &Run,
    record: &Value,
    offset: usize,
    needle: &str,
    seconds: u64,
) -> Result<(String, usize), String> {
    wait_any(run, record, offset, &[needle], seconds).map(|(_, line, end)| (line, end))
}
fn length(run: &Run, record: &Value) -> Result<usize, String> {
    Ok(serial(&run.root, record)?.len())
}
/// `DOOR_P14 FILEOP begin`: (op token, line, offset past it).
fn wait_begin(
    run: &Run,
    record: &Value,
    offset: usize,
    mode: &str,
    sources: usize,
) -> Result<(String, String, usize), String> {
    let (line, end) = wait_line(run, record, offset, "DOOR_P14 FILEOP begin op=", 30)?;
    let f = fields(&line, "FILEOP begin");
    if f.get("mode") != Some(&mode) || f.get("sources") != Some(&sources.to_string().as_str()) {
        return Err(format!(
            "expected a {mode} of {sources} sources, Explorer began: {line}"
        ));
    }
    let op = f.get("op").ok_or("FILEOP begin without op")?.to_string();
    Ok((op, line, end))
}
#[derive(Debug, PartialEq)]
struct End {
    state: String,
    items: (u64, u64),
    bytes: (u64, u64),
    skipped: u64,
    failed: u64,
    line: String,
}
fn parse_end(line: &str) -> Result<End, String> {
    let f = fields(line, "FILEOP end");
    let number = |key: &str| -> Result<u64, String> {
        f.get(key)
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| format!("FILEOP end without {key}: {line}"))
    };
    Ok(End {
        state: f
            .get("state")
            .ok_or("FILEOP end without state")?
            .to_string(),
        items: ratio(f.get("items")).ok_or("FILEOP end items")?,
        bytes: ratio(f.get("bytes")).ok_or("FILEOP end bytes")?,
        skipped: number("skipped")?,
        failed: number("failed")?,
        line: line.to_owned(),
    })
}
fn wait_end(
    run: &Run,
    record: &Value,
    offset: usize,
    op: &str,
    seconds: u64,
) -> Result<(End, usize), String> {
    let (line, end) = wait_line(
        run,
        record,
        offset,
        &format!("DOOR_P14 FILEOP end op={op} "),
        seconds,
    )?;
    Ok((parse_end(&line)?, end))
}
/// A completed operation: every item and byte, nothing skipped or failed.
fn complete(end: &End, bytes: u64) -> Result<(), String> {
    if end.state != "done"
        || end.items.0 != end.items.1
        || end.bytes != (bytes, bytes)
        || end.skipped != 0
        || end.failed != 0
    {
        return Err(format!(
            "operation did not complete exactly ({bytes} bytes): {}",
            end.line
        ));
    }
    Ok(())
}
fn end_json(end: &End) -> Value {
    json!({"state":end.state,"items":[end.items.0,end.items.1],"bytes":[end.bytes.0,end.bytes.1],
        "skipped":end.skipped,"failed":end.failed,"line":end.line})
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
/// Start search for `query`; Enter opens the best match, app `app` must
/// become the foreground application.
fn start_app(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    query: &str,
    app: usize,
) -> Result<(), String> {
    let offset = length(run, record)?;
    key(c, &["meta_l"])?;
    thread::sleep(Duration::from_millis(400));
    type_line(c, query)?;
    resolution::wait_activation(&run.root, record, app, offset)
}
/// A Terminal builtin that must succeed within `seconds`.
fn command(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    line: &str,
    seconds: u64,
) -> Result<(), String> {
    let offset = length(run, record)?;
    type_line(c, line)?;
    let name = line.split_whitespace().next().ok_or("empty command")?;
    let (result, _) = wait_line(
        run,
        record,
        offset,
        &format!("TERMINAL COMMAND name={name} ok="),
        seconds,
    )?;
    if !result.contains(&format!("name={name} ok=1")) {
        return Err(format!("Terminal command failed: {line}: {result}"));
    }
    Ok(())
}
/// The Terminal builds the test tree from its builtins and the seed.
fn build_tree(run: &Run, record: &Value, c: &mut qmp::Client) -> Result<(), String> {
    start_app(run, record, c, "terminal", 9)?;
    for rel in FOLDERS {
        let path = if rel.is_empty() {
            "p14".to_owned()
        } else {
            format!("p14/{rel}")
        };
        command(run, record, c, &format!("mkdir {path}"), 20)?;
    }
    for (path, text) in SMALL {
        command(run, record, c, &format!("write p14/{path} {text}"), 20)?;
    }
    // 4 MiB through 192-byte storage reads.
    command(
        run,
        record,
        c,
        &format!("cp {SEED_WINDOWS} p14/big/{}", BIG[2]),
        300,
    )?;
    key(c, &["alt", "f4"])?;
    thread::sleep(Duration::from_secs(1));
    Ok(())
}
/// Explorer's address bar (Ctrl+L): `typed`, Return, resolved to `expected`.
fn address(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    typed: &str,
    expected: &str,
) -> Result<(), String> {
    key(c, &["ctrl", "l"])?;
    type_text(c, typed)?;
    let offset = length(run, record)?;
    key(c, &["ret"])?;
    wait_line(
        run,
        record,
        offset,
        &format!("FILES ADDRESS path={expected} history="),
        20,
    )?;
    thread::sleep(Duration::from_millis(400));
    Ok(())
}
fn folder(run: &Run, record: &Value, c: &mut qmp::Client, rel: &str) -> Result<(), String> {
    let mut typed = String::from(r"C:\Users\1001\p14");
    if !rel.is_empty() {
        typed.push('\\');
        typed.push_str(&rel.replace('/', "\\"));
    }
    address(run, record, c, &typed, &under(rel))
}
/// The Recycle Bin view: Explorer reports its virtual location
/// (`file_operations::RECYCLE_BIN`); the physical per-user folder `BIN` stays
/// hidden and is what the disk audit reads.
fn recycle_bin(run: &Run, record: &Value, c: &mut qmp::Client) -> Result<(), String> {
    address(
        run,
        record,
        c,
        "shell:RecycleBinFolder",
        door_workspace::file_operations::RECYCLE_BIN,
    )
}
/// Ctrl+A must select exactly `count` listed items.
fn select_all(run: &Run, record: &Value, c: &mut qmp::Client, count: usize) -> Result<(), String> {
    let offset = length(run, record)?;
    key(c, &["ctrl", "a"])?;
    let (line, _) = wait_line(run, record, offset, "FILES SELECT count=", 10)?;
    let n = fields(&line, "FILES SELECT")
        .get("count")
        .and_then(|v| v.parse::<usize>().ok())
        .ok_or("FILES SELECT count")?;
    if n != count {
        return Err(format!(
            "Ctrl+A selected {n} items, expected {count}: {line}"
        ));
    }
    Ok(())
}
/// Clipboard copy (or cut) of the selection, then paste into `rel`.
fn transfer(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    cut: bool,
    rel: &str,
) -> Result<usize, String> {
    key(c, &["ctrl", if cut { "x" } else { "c" }])?;
    thread::sleep(Duration::from_millis(300));
    folder(run, record, c, rel)?;
    let offset = length(run, record)?;
    key(c, &["ctrl", "v"])?;
    Ok(offset)
}

// ---- raw disk ----------------------------------------------------------------

#[derive(Clone, Debug)]
struct Entry {
    id: u64,
    kind: u8,
    attributes: u64,
    owner: u64,
    pending: bool,
    created: u64,
}
/// The namespace of a raw image (journal replay, ext2, catalog): every
/// path, and the bytes of the files `keep` selects.
struct Snapshot {
    clean: bool,
    entries: BTreeMap<String, Entry>,
    files: BTreeMap<String, Vec<u8>>,
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or([0; 8]))
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap_or([0; 4]))
}
fn snapshot(raw: &[u8], strict: bool, keep: impl Fn(&str) -> bool) -> Result<Snapshot, String> {
    let journal = journal::inspect(raw)?;
    let home = journal.replayed_home(raw)?;
    let tree = tree::inspect(&home)?;
    let catalog = if strict {
        catalog::decode(&tree)?
    } else {
        catalog::decode_pending(&tree)?
    }
    .ok_or("catalog absent")?;
    let mut entries = BTreeMap::new();
    let mut files = BTreeMap::new();
    for (path, node) in catalog.paths()? {
        let created = catalog::record_offset(catalog.bytes, node.id)
            .map_or(0, |(at, _)| u64_at(catalog.bytes, at + 208));
        if node.kind == 1
            && keep(&path)
            && let Some(file) = tree.files.get(&format!("obj{:016x}", node.id))
        {
            files.insert(path.clone(), file.bytes.clone());
        }
        entries.insert(
            path,
            Entry {
                id: node.id,
                kind: node.kind,
                attributes: node.attributes,
                owner: node.owner,
                pending: node.pending,
                created,
            },
        );
    }
    Ok(Snapshot {
        clean: !journal.selected().committed,
        entries,
        files,
    })
}
impl Snapshot {
    fn file(&self, path: &str) -> Result<&[u8], String> {
        self.files
            .get(path)
            .map(Vec::as_slice)
            .ok_or_else(|| format!("{path} missing from the raw disk"))
    }
    fn require(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        if self.file(path)? != bytes {
            return Err(format!("{path} bytes differ on the raw disk"));
        }
        Ok(())
    }
    fn absent(&self, path: &str) -> Result<(), String> {
        if self.entries.contains_key(path) {
            return Err(format!("{path} is still on the raw disk"));
        }
        Ok(())
    }
    fn children(&self, folder: &str) -> Vec<&str> {
        let prefix = format!("{folder}/");
        self.entries
            .keys()
            .filter_map(|p| p.strip_prefix(&prefix))
            .filter(|rest| !rest.contains('/'))
            .collect()
    }
    /// `~$fop<op>.tmp` temporaries anywhere.
    fn temporaries(&self) -> Vec<&str> {
        self.entries
            .keys()
            .filter(|p| {
                p.rsplit('/')
                    .next()
                    .is_some_and(|n| n.starts_with("~$fop") && n.ends_with(".tmp"))
            })
            .map(String::as_str)
            .collect()
    }
    /// Operation records (`/state/machine/fileops/op<16 hex>`).
    fn records(&self) -> Result<Vec<OpRecord>, String> {
        let prefix = format!("{RECORDS}/op");
        self.entries
            .keys()
            .filter(|p| p.starts_with(&prefix))
            .map(|p| {
                let record = OpRecord::decode(self.file(p)?)?;
                if *p != format!("{RECORDS}/op{:016x}", record.op) {
                    return Err(format!("{p} holds operation {}", record.op));
                }
                Ok(record)
            })
            .collect()
    }
}
fn keep_all(_: &str) -> bool {
    true
}
fn keep_records(path: &str) -> bool {
    path.starts_with(RECORDS)
}

/// The engine's durable operation record (`DOORFOP1`, layout table in
/// `fs/file-ops/src/plan.rs`), decoded independently.
#[derive(Clone, Debug, PartialEq)]
struct OpRecord {
    op: u64,
    owner: u32,
    mode: u8,
    state: u8,
    items: (u32, u32),
    skipped: u32,
    failures: u32,
    bytes: (u64, u64),
    flags: u8,
    file_offset: u64,
}
impl OpRecord {
    const RUNNING: u8 = 1;
    const PAUSED: u8 = 2;
    fn decode(b: &[u8]) -> Result<Self, String> {
        if b.len() < 236 || &b[..8] != b"DOORFOP1" || u32_at(b, 8) != 1 {
            return Err("operation record magic/version".into());
        }
        let len = u32_at(b, 12) as usize;
        if !(236..=8192).contains(&len) || len > b.len() || b[len..].iter().any(|v| *v != 0) {
            return Err("operation record length".into());
        }
        if crate::storage::audit::crc32(&b[..len - 4]) != u32_at(b, len - 4) {
            return Err("operation record CRC".into());
        }
        let record = Self {
            op: u64_at(b, 16),
            owner: u32_at(b, 24),
            mode: b[28],
            state: b[30],
            items: (u32_at(b, 36), u32_at(b, 32)),
            skipped: u32_at(b, 40),
            failures: u32_at(b, 44),
            bytes: (u64_at(b, 56), u64_at(b, 48)),
            flags: b[66],
            file_offset: u64_at(b, 80),
        };
        if record.op == 0 || record.mode > 2 || record.state > 6 || b[70..72] != [0, 0] {
            return Err("operation record fields".into());
        }
        Ok(record)
    }
    /// A copy running with a finished item behind it and the current file
    /// part-copied, at most `share`/4 of the bytes done.
    fn mid_copy(&self, share: u64) -> bool {
        self.mode == 0
            && self.state == Self::RUNNING
            && self.items.0 >= 1
            && self.items.0 < self.items.1
            && self.bytes.0 > 0
            && self.bytes.0 * 4 <= self.bytes.1 * share
            && (self.file_offset > 0 || self.flags & 1 != 0)
    }
    fn json(&self) -> Value {
        json!({"op":self.op,"owner":self.owner,"mode":self.mode,"state":self.state,
            "items":[self.items.0,self.items.1],"skipped":self.skipped,"failures":self.failures,
            "bytes":[self.bytes.0,self.bytes.1],"flags":self.flags,"file_offset":self.file_offset})
    }
}
/// A Recycle Bin info record (`DOORRCY1`, `fs/file-ops/src/recycle.rs`).
#[derive(Debug, PartialEq)]
struct Info {
    id: u64,
    deleted: u64,
    bytes: u64,
    items: u32,
    folder: bool,
    original: String,
}
impl Info {
    fn decode(b: &[u8]) -> Result<Self, String> {
        if b.len() < 52 || &b[..8] != b"DOORRCY1" || u32_at(b, 8) != 1 {
            return Err("recycle info magic/version".into());
        }
        let len = u32_at(b, 12) as usize;
        let path = u16::from_le_bytes([b[46], b[47]]) as usize;
        if len != 48 + path + 4 || len > b.len() || b[len..].iter().any(|v| *v != 0) {
            return Err("recycle info length".into());
        }
        if crate::storage::audit::crc32(&b[..len - 4]) != u32_at(b, len - 4) {
            return Err("recycle info CRC".into());
        }
        if b[44] & !1 != 0 || b[45] != 0 {
            return Err("recycle info flags".into());
        }
        Ok(Self {
            id: u64_at(b, 16),
            deleted: u64_at(b, 24),
            bytes: u64_at(b, 32),
            items: u32_at(b, 40),
            folder: b[44] & 1 != 0,
            original: String::from_utf8(b[48..48 + path].to_vec())
                .map_err(|_| "recycle info path encoding")?,
        })
    }
}
fn read_disk(disk: &std::path::Path) -> Result<Vec<u8>, String> {
    fs::read(disk).map_err(|e| format!("read {}: {e}", disk.display()))
}
/// Pause the guest between engine steps until a durable record satisfies
/// `ready`. Returns with the guest STOPPED; the caller resumes or powers it
/// off. Fails if the operation ends first.
fn strobe(
    run: &Run,
    record: &Value,
    c: &mut qmp::Client,
    disk: &std::path::Path,
    offset: usize,
    ready: impl Fn(&OpRecord) -> bool,
) -> Result<(OpRecord, usize), String> {
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut polls = 0;
    loop {
        thread::sleep(Duration::from_millis(30));
        c.execute("stop", json!({}))?;
        polls += 1;
        let found = read_disk(disk)
            .and_then(|raw| snapshot(&raw, false, keep_records))
            .and_then(|s| s.records());
        let found = match found {
            Ok(records) => records.into_iter().find(|r| ready(r)),
            Err(e) => {
                let _ = c.execute("cont", json!({}));
                return Err(format!("raw-disk checkpoint during the operation: {e}"));
            }
        };
        if let Some(found) = found {
            return Ok((found, polls));
        }
        let ended = serial(&run.root, record)?
            .get(offset..)
            .unwrap_or("")
            .contains("DOOR_P14 FILEOP end op=");
        c.execute("cont", json!({}))?;
        if ended {
            return Err(
                "the operation ended before a durable mid-copy checkpoint was observed".into(),
            );
        }
        if Instant::now() >= deadline {
            return Err("no durable mid-copy record within 120 s".into());
        }
    }
}
fn shot(run: &mut Run, c: &mut qmp::Client, name: &str) -> Result<(scene::Pixels, Value), String> {
    let p = win32::screen(run, c, &format!("p14-files-{name}"))?;
    let kept = win32::keep(run, &p, &format!("p14-files-{name}"))?;
    Ok((p, kept))
}
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
        return Err("desktop-a-files session did not shut down cleanly".into());
    }
    seal_serial(&run.root, record)
}
/// QEMU released the private image (its QMP port closed).
fn wait_gone(record: &Value) -> Result<(), String> {
    let port =
        u16::try_from(record["qmp_port"].as_u64().ok_or("QMP port")?).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
        if Instant::now() >= deadline {
            return Err("QEMU kept running after the session ended".into());
        }
        thread::sleep(Duration::from_millis(200));
    }
    thread::sleep(Duration::from_secs(1));
    Ok(())
}

// ---- image variant -------------------------------------------------------------

/// The shipping desktop image plus the large seed, staged once per run.
fn stage(run: &mut Run, image: &str) -> Result<String, String> {
    let cache = run.directory.join("p14-files-stage.json");
    if cache.is_file() {
        let record = crate::image::read_json(&cache)?;
        return Ok(record["image"]
            .as_str()
            .ok_or("files stage image")?
            .to_owned());
    }
    let (base, _) = crate::image::validate_staged(&run.root, image)?;
    let production = fs::read(base.join("esp/Door/initfs.tar")).map_err(|e| e.to_string())?;
    let archive =
        door_initfs::Archive::parse(&production).map_err(|e| format!("desktop initfs {e:?}"))?;
    let mut manifest = String::from_utf8(
        archive
            .find("system/install.manifest")
            .ok_or("desktop initfs lacks system/install.manifest")?
            .data
            .to_vec(),
    )
    .map_err(|e| e.to_string())?;
    if !manifest.ends_with('\n') {
        manifest.push('\n');
    }
    manifest.push_str(&format!("{SEED} {SEED_INITFS}\n"));
    let seed = large_seed();
    let edits: [(&str, &[u8]); 2] = [
        (SEED_INITFS, &seed),
        ("system/install.manifest", manifest.as_bytes()),
    ];
    let target = run.generated_directory(&format!("out/builds/{}/p14-files", run.id))?;
    let out = target.join("initfs-desktop-p14-files.tar");
    fs::write(
        &out,
        crate::initfs::with_files(&production, &edits, &["system/install.manifest"])?,
    )
    .map_err(|e| e.to_string())?;
    let staged = session::stage_desktop_variant(
        run,
        &base.join("esp/EFI/BOOT/BOOTX64.EFI"),
        &base.join("esp/Door/kernel.elf"),
        &out,
        "desktop-p14-files",
    )?;
    let record = json!({"image":staged,"base_image":image,"initfs":relative(&run.root,&out)?,
        "initfs_sha256":file_hash(&out)?,"seed":SEED,"seed_bytes":LARGE});
    write_json(&cache, &record)?;
    run.check("p14-desktop-files-stage", record);
    Ok(staged)
}

// ---- the suite ------------------------------------------------------------------

/// Results of the first boot the audits check.
#[derive(Default)]
struct Observed {
    /// Ops whose records must be gone.
    ops: Vec<String>,
    /// Cancelled copy: items finished before the cancel.
    cancelled_done: usize,
    /// Recycle IDs (restored item, kept item).
    restored: String,
    kept: String,
}
pub(super) fn suite(run: &mut Run, image: &str, _test_image: &str) -> Result<Value, String> {
    let staged = stage(run, image)?;
    let disk = win32::fresh_disk(run, "p14-files")?;
    // Boot 1: every interactive operation.
    let mut first =
        session::launch_desktop_accelerated(run, &staged, &disk, false, win32::RESOLUTION)?;
    thread::sleep(Duration::from_secs(2));
    let mut observed = Observed::default();
    let result = boot_one(run, &mut first, &disk, &mut observed);
    let interactive = win32::quit_on_error(run, &first, result, "p14-files-first-failure")?;
    wait_gone(&first)?;
    let first_audit = audit(&read_disk(&disk)?, &observed, false)?;
    // Boot 2: reopen, then a hard power-off mid-copy.
    let mut second =
        session::launch_desktop_accelerated(run, &staged, &disk, false, win32::RESOLUTION)?;
    thread::sleep(Duration::from_secs(2));
    let result = boot_two(run, &mut second, &disk);
    let (interrupted_op, reopened) =
        win32::quit_on_error(run, &second, result, "p14-files-second-failure")?;
    wait_gone(&second)?;
    crate::evidence::seal_session_serial(&run.root, &mut second)?;
    let raw = read_disk(&disk)?;
    let interrupted_disk = file_hash(&disk)?;
    let after_kill = snapshot(&raw, false, keep_records)?;
    let records = after_kill.records()?;
    let pending = records
        .iter()
        .find(|r| ids(&interrupted_op).contains(&r.op))
        .ok_or("the interrupted operation has no durable record after the power-off")?;
    if records.len() != 1
        || !matches!(pending.state, OpRecord::RUNNING | OpRecord::PAUSED)
        || pending.bytes.0 >= pending.bytes.1
        || pending.owner as u64 != UID
    {
        return Err(format!(
            "power-off record is not an unfinished copy: {:?}",
            records
        ));
    }
    let killed = json!({"op":interrupted_op,"record":pending.json(),"journal_clean":after_kill.clean,
        "temporaries":after_kill.temporaries(),"disk_sha256":interrupted_disk});
    // Boot 3: the interrupted operation comes back Paused and resumes.
    let mut third =
        session::launch_desktop_accelerated(run, &staged, &disk, false, win32::RESOLUTION)?;
    thread::sleep(Duration::from_secs(2));
    let result = boot_three(run, &mut third, &interrupted_op);
    let resumed = win32::quit_on_error(run, &third, result, "p14-files-third-failure")?;
    wait_gone(&third)?;
    let raw = read_disk(&disk)?;
    let final_audit = audit(&raw, &observed, true)?;
    Ok(json!({
        "image": staged,
        "disk": relative(&run.root, &disk)?,
        "disk_sha256": file_hash(&disk)?,
        "interactive": interactive,
        "first_audit": first_audit,
        "reopened": reopened,
        "power_off": killed,
        "resumed": resumed,
        "final_audit": final_audit,
        "sessions": [first, second, third],
        "separate_boots": 3,
        "reference_comparison": {"state":"not_applicable",
            "scope":"File operations, recycle and change notification behavior with raw-disk audits; visual fidelity is VIS-01"},
    }))
}
fn boot_one(
    run: &mut Run,
    record: &mut Value,
    disk: &std::path::Path,
    observed: &mut Observed,
) -> Result<Value, String> {
    let mut c = connection(record)?;
    let mut p = (960, 540);
    super::p14_shell::require_unit_scale(run, record)?;
    build_tree(run, record, &mut c)?;
    start_app(run, record, &mut c, "file explorer", 1)?;
    // 1. Copy two files and a folder tree, then move the copies.
    let tree_bytes: u64 = MOVED
        .iter()
        .map(|(_, source)| small(source).map(|b| b.len() as u64))
        .sum::<Result<u64, String>>()?;
    folder(run, record, &mut c, "src")?;
    select_all(run, record, &mut c, 3)?;
    let offset = transfer(run, record, &mut c, false, "dst")?;
    let (copy_op, _, at) = wait_begin(run, record, offset, "copy", 3)?;
    let (copied, _) = wait_end(run, record, at, &copy_op, 60)?;
    complete(&copied, tree_bytes)?;
    select_all(run, record, &mut c, 3)?;
    let offset = transfer(run, record, &mut c, true, "moved")?;
    let (move_op, _, at) = wait_begin(run, record, offset, "move", 3)?;
    let (moved, _) = wait_end(run, record, at, &move_op, 60)?;
    if moved.state != "done" || moved.items.0 != moved.items.1 || moved.failed != 0 {
        return Err(format!("move did not complete: {}", moved.line));
    }
    let (moved_shot, moved_png) = shot(run, &mut c, "moved")?;
    let _ = moved_shot;
    // 2. Conflicts, one decision per item.
    folder(run, record, &mut c, "csrc")?;
    select_all(run, record, &mut c, 3)?;
    let offset = transfer(run, record, &mut c, false, "cdst")?;
    let (each_op, _, mut at) = wait_begin(run, record, offset, "copy", 3)?;
    let conflict = format!("DOOR_P14 FILEOP conflict op={each_op} ");
    let ending = format!("DOOR_P14 FILEOP end op={each_op} ");
    let mut decided = Vec::new();
    // The first conflict opens the overview ("Replace the files in the
    // destination", "Skip these files", "Let me decide for each file");
    // Tab, Tab, Enter picks "Let me decide", and every later conflict of
    // the operation opens the per-file stage, where Alt+R, Alt+S and Alt+K
    // decide only the item asked about.
    let mut per_file = false;
    let each_end = loop {
        let (which, line, next) = wait_any(run, record, at, &[&conflict, &ending], 60)?;
        if which == 1 {
            break parse_end(&line)?;
        }
        let name = fields(&line, "FILEOP conflict")
            .get("name")
            .map(|n| n.to_string())
            .ok_or("conflict without name")?;
        let (_, accel, decision) = DECISIONS
            .iter()
            .find(|(n, _, _)| *n == name)
            .ok_or_else(|| format!("unexpected conflict: {line}"))?;
        if decided.iter().any(|(n, _): &(String, Value)| *n == name) {
            return Err(format!("conflict asked twice: {line}"));
        }
        if name == "x1.txt" {
            let (dialog, png) = shot(run, &mut c, "conflict-dialog")?;
            let _ = dialog;
            decided.push(("dialog".into(), png));
        }
        if !per_file {
            key(&mut c, &["tab"])?;
            key(&mut c, &["tab"])?;
            key(&mut c, &["ret"])?;
            per_file = true;
        }
        key(&mut c, &["alt", *accel])?;
        let (resolved, past) = wait_line(
            run,
            record,
            next,
            &format!("DOOR_P14 FILEOP resolve op={each_op} "),
            20,
        )?;
        let f = fields(&resolved, "FILEOP resolve");
        if f.get("decision") != Some(decision) || f.get("all") != Some(&"0") {
            return Err(format!(
                "{name}: expected {decision} for this item only: {resolved}"
            ));
        }
        decided.push((name, json!(resolved)));
        at = past;
    };
    if decided.iter().filter(|(n, _)| n != "dialog").count() != DECISIONS.len()
        || each_end.state != "done"
        || each_end.skipped != 1
        || each_end.failed != 0
    {
        return Err(format!(
            "per-item conflicts incomplete ({} decided): {}",
            decided.len(),
            each_end.line
        ));
    }
    // ... and one decision for all.
    folder(run, record, &mut c, "asrc")?;
    select_all(run, record, &mut c, 3)?;
    let offset = transfer(run, record, &mut c, false, "adst")?;
    let (all_op, _, at) = wait_begin(run, record, offset, "copy", 3)?;
    let (first_conflict, next) = wait_line(
        run,
        record,
        at,
        &format!("DOOR_P14 FILEOP conflict op={all_op} "),
        30,
    )?;
    // Let me decide, check "Do this for the next 2 conflicts", Keep both.
    key(&mut c, &["tab"])?;
    key(&mut c, &["tab"])?;
    key(&mut c, &["ret"])?;
    key(&mut c, &["alt", "d"])?;
    key(&mut c, &["alt", "k"])?;
    let (resolved, past) = wait_line(
        run,
        record,
        next,
        &format!("DOOR_P14 FILEOP resolve op={all_op} "),
        20,
    )?;
    let f = fields(&resolved, "FILEOP resolve");
    if f.get("decision") != Some(&"keepboth") || f.get("all") != Some(&"1") {
        return Err(format!("expected Keep both for all conflicts: {resolved}"));
    }
    let (all_end, _) = wait_end(run, record, past, &all_op, 60)?;
    let between = serial(&run.root, record)?;
    let between = between.get(past..).unwrap_or("");
    let between = between
        .split(&format!("FILEOP end op={all_op} "))
        .next()
        .unwrap_or("");
    if between.contains(&format!("FILEOP conflict op={all_op} ")) {
        return Err("\"Do this for all\" still asked about a later conflict".into());
    }
    complete(&all_end, 3 * 7)?;
    // 3. Cancel while the large file is mid-copy.
    folder(run, record, &mut c, "big")?;
    select_all(run, record, &mut c, 3)?;
    key(&mut c, &["ctrl", "c"])?;
    thread::sleep(Duration::from_millis(300));
    folder(run, record, &mut c, "cancel")?;
    move_to(&mut c, &mut p, 1900, 20)?;
    let (before, _) = shot(run, &mut c, "cancel-before")?;
    let offset = length(run, record)?;
    key(&mut c, &["ctrl", "v"])?;
    let (mid, polls) = strobe(run, record, &mut c, disk, offset, |r| r.mid_copy(3))?;
    // The folder's own listing also changes mid-copy (the staged temporary
    // file), so pixels alone cannot tell the dialog is up: Explorer logs
    // `FILEOP shown` in the tick that reveals it, which paints it.
    let mut progress = shot(run, &mut c, "cancel-progress");
    let mut revealed = false;
    for _ in 0..40 {
        let shown = match &progress {
            Ok((p, _)) => win32::changed(&before, p, [0, 0, 1920, 1032])?,
            Err(_) => break,
        };
        if revealed && shown >= 2000 {
            break;
        }
        c.execute("cont", json!({}))?;
        thread::sleep(Duration::from_millis(60));
        c.execute("stop", json!({}))?;
        let tail = serial(&run.root, record)?;
        let after = tail.get(offset..).unwrap_or("");
        if after.contains("DOOR_P14 FILEOP end op=") {
            break;
        }
        revealed = after.contains("DOOR_P14 FILEOP shown op=");
        progress = shot(run, &mut c, "cancel-progress");
    }
    // QEMU discards input sent while the guest is stopped, so Esc follows
    // `cont`; the dialog is already up (its marker), so it cancels mid-copy.
    c.execute("cont", json!({}))?;
    key(&mut c, &["esc"])?;
    let _ = shot(run, &mut c, "cancel-after-esc");
    let (progress, progress_png) = progress?;
    let shown = win32::changed(&before, &progress, [0, 0, 1920, 1032])?;
    if shown < 2000 {
        return Err(format!(
            "no progress dialog on screen during the copy ({shown} px)"
        ));
    }
    let (cancel_op, _, at) = wait_begin(run, record, offset, "copy", 3)?;
    if !ids(&cancel_op).contains(&mid.op) {
        return Err(format!(
            "FILEOP begin op={cancel_op} is not record {}",
            mid.op
        ));
    }
    let (cancelled, _) = wait_end(run, record, at, &cancel_op, 60)?;
    if cancelled.state != "cancelled"
        || cancelled.items.0 == 0
        || cancelled.items.0 >= cancelled.items.1
        || cancelled.bytes.0 >= cancelled.bytes.1
    {
        return Err(format!("cancel did not land mid-copy: {}", cancelled.line));
    }
    observed.cancelled_done = usize::try_from(cancelled.items.0).map_err(|e| e.to_string())?;
    if observed.cancelled_done >= BIG.len() {
        return Err(format!(
            "cancelled copy finished every file: {}",
            cancelled.line
        ));
    }
    // 4. Recycle and restore, recycle and keep, permanent delete.
    folder(run, record, &mut c, "recycle")?;
    key(&mut c, &["home"])?;
    thread::sleep(Duration::from_millis(400));
    let offset = length(run, record)?;
    key(&mut c, &["delete"])?;
    let (recycled, _) = wait_line(run, record, offset, "DOOR_P14 RECYCLE id=", 20)?;
    let f = fields(&recycled, "DOOR_P14 RECYCLE");
    let restored = f.get("id").ok_or("RECYCLE id")?.to_string();
    if f.get("path") != Some(&format!("{ROOT}/recycle/r1.txt").as_str()) {
        return Err(format!("Delete recycled the wrong item: {recycled}"));
    }
    recycle_bin(run, record, &mut c)?;
    let (bin, bin_png) = shot(run, &mut c, "recycle-bin")?;
    let _ = bin;
    key(&mut c, &["home"])?;
    thread::sleep(Duration::from_millis(400));
    let offset = length(run, record)?;
    key(&mut c, &["shift", "f10"])?;
    wait_line(run, record, offset, "FILES MENU subject=", 10)?;
    key(&mut c, &["ret"])?;
    let (restore, _) = wait_line(
        run,
        record,
        offset,
        &format!("DOOR_P14 RESTORE id={restored} "),
        20,
    )
    .or_else(|_| wait_line(run, record, offset, "DOOR_P14 RESTORE id=", 1))?;
    let f = fields(&restore, "DOOR_P14 RESTORE");
    if f.get("id") != Some(&restored.as_str()) || f.get("ok") != Some(&"1") {
        return Err(format!("restore of {restored} failed: {restore}"));
    }
    folder(run, record, &mut c, "recycle")?;
    key(&mut c, &["end"])?;
    thread::sleep(Duration::from_millis(400));
    let offset = length(run, record)?;
    key(&mut c, &["delete"])?;
    let (recycled_kept, _) = wait_line(run, record, offset, "DOOR_P14 RECYCLE id=", 20)?;
    let f = fields(&recycled_kept, "DOOR_P14 RECYCLE");
    let kept = f.get("id").ok_or("RECYCLE id")?.to_string();
    if f.get("path") != Some(&format!("{ROOT}/recycle/r2.txt").as_str()) || kept == restored {
        return Err(format!("Delete recycled the wrong item: {recycled_kept}"));
    }
    folder(run, record, &mut c, "purge")?;
    key(&mut c, &["home"])?;
    thread::sleep(Duration::from_millis(400));
    let offset = length(run, record)?;
    key(&mut c, &["shift", "delete"])?;
    wait_line(run, record, offset, "FILES REMOVE_PROMPT", 15)?;
    let (confirm, confirm_png) = shot(run, &mut c, "purge-confirm")?;
    let _ = confirm;
    key(&mut c, &["esc"])?;
    wait_line(run, record, offset, "FILES REMOVE_CANCEL", 15)?;
    thread::sleep(Duration::from_secs(1));
    let declined = serial(&run.root, record)?;
    let declined = declined.get(offset..).unwrap_or("");
    if declined.contains("DOOR_P14 PURGE") || declined.contains("FILEOP begin op=") {
        return Err("declining the permanent-delete prompt still deleted".into());
    }
    let offset = length(run, record)?;
    key(&mut c, &["shift", "delete"])?;
    wait_line(run, record, offset, "FILES REMOVE_PROMPT", 15)?;
    key(&mut c, &["tab"])?;
    key(&mut c, &["ret"])?;
    let (purge_op, _, at) = wait_begin(run, record, offset, "delete", 1)?;
    wait_line(
        run,
        record,
        offset,
        &format!("DOOR_P14 PURGE op={purge_op}"),
        30,
    )?;
    let (purged, _) = wait_end(run, record, at, &purge_op, 30)?;
    if purged.state != "done" || purged.failed != 0 {
        return Err(format!("permanent delete failed: {}", purged.line));
    }
    // 5. Change notification across two Explorer windows.
    let watch = watch(run, record, &mut c, &mut p)?;
    observed.ops = vec![copy_op, move_op, each_op, all_op, cancel_op, purge_op];
    observed.restored = restored;
    observed.kept = kept;
    shut_down(run, record, &mut c)?;
    Ok(json!({
        "copy": end_json(&copied),
        "move": end_json(&moved),
        "conflicts_each": {"decisions": decided.into_iter().map(|(n, v)| json!({"name":n,"resolve":v})).collect::<Vec<_>>(),
            "end": end_json(&each_end)},
        "conflicts_all": {"conflict": first_conflict, "resolve": resolved, "end": end_json(&all_end)},
        "cancel": {"mid_record": mid.json(), "strobe_polls": polls, "progress_pixels": shown,
            "end": end_json(&cancelled)},
        "recycle": {"restored": recycled, "restore": restore, "kept": recycled_kept},
        "purge": end_json(&purged),
        "watch": watch,
        "screens": {"moved":moved_png,"progress":progress_png,"bin":bin_png,"purge_confirm":confirm_png},
    }))
}
/// Two Explorer windows on `watch/`, snapped side by side; a folder created
/// in the left one must appear in the right one (its pixels), and Explorer
/// reports the watcher's refresh.
fn watch(
    run: &mut Run,
    record: &Value,
    c: &mut qmp::Client,
    p: &mut (i32, i32),
) -> Result<Value, String> {
    let path = under("watch");
    folder(run, record, c, "watch")?;
    key(c, &["ctrl", "n"])?;
    thread::sleep(Duration::from_millis(2500));
    folder(run, record, c, "watch")?;
    key(c, &["meta_l", "right"])?;
    thread::sleep(Duration::from_millis(800));
    key(c, &["alt", "tab"])?;
    thread::sleep(Duration::from_millis(800));
    key(c, &["meta_l", "left"])?;
    thread::sleep(Duration::from_millis(800));
    move_to(c, p, 20, 1000)?;
    let (before, before_png) = shot(run, c, "watch-before")?;
    let offset = length(run, record)?;
    key(c, &["ctrl", "shift", "n"])?;
    let (refresh, _) = wait_line(
        run,
        record,
        offset,
        &format!("DOOR_P14 WATCH refresh path={path}"),
        20,
    )?;
    thread::sleep(Duration::from_secs(1));
    key(c, &["esc"])?;
    thread::sleep(Duration::from_millis(600));
    let (after, after_png) = shot(run, c, "watch-after")?;
    // The right (inactive) window's listing, below its caption and command
    // bar and right of its navigation pane.
    let right = [960 + 260, 140, 960 - 280, 1032 - 180];
    let changed = win32::changed(&before, &after, right)?;
    if changed < 400 {
        return Err(format!(
            "the second Explorer window did not show the change ({changed} px)"
        ));
    }
    let refreshes = serial(&run.root, record)?
        .get(offset..)
        .unwrap_or("")
        .matches(&format!("DOOR_P14 WATCH refresh path={path}"))
        .count();
    Ok(
        json!({"refresh":refresh,"refreshes":refreshes,"second_window_changed_pixels":changed,
        "screens":{"before":before_png,"after":after_png}}),
    )
}
fn boot_two(
    run: &mut Run,
    record: &mut Value,
    disk: &std::path::Path,
) -> Result<(String, Value), String> {
    let mut c = connection(record)?;
    super::p14_shell::require_unit_scale(run, record)?;
    start_app(run, record, &mut c, "file explorer", 1)?;
    // The reopened image as Explorer lists it.
    folder(run, record, &mut c, "recycle")?;
    select_all(run, record, &mut c, 1)?;
    folder(run, record, &mut c, "moved")?;
    select_all(run, record, &mut c, 3)?;
    recycle_bin(run, record, &mut c)?;
    select_all(run, record, &mut c, 1)?;
    let (_, bin_png) = shot(run, &mut c, "reopened-bin")?;
    // A copy powered off while the large file is mid-copy.
    folder(run, record, &mut c, "big")?;
    select_all(run, record, &mut c, 3)?;
    key(&mut c, &["ctrl", "c"])?;
    thread::sleep(Duration::from_millis(300));
    folder(run, record, &mut c, "interrupted")?;
    let offset = length(run, record)?;
    key(&mut c, &["ctrl", "v"])?;
    let (mid, polls) = strobe(run, record, &mut c, disk, offset, |r| r.mid_copy(4))?;
    let progress = shot(run, &mut c, "interrupted-progress");
    let text = serial(&run.root, record)?;
    let begin = text
        .get(offset..)
        .unwrap_or("")
        .lines()
        .find(|l| l.contains("DOOR_P14 FILEOP begin op="))
        .map(str::to_owned);
    // Hard power-off with the guest paused at the durable mid-copy state.
    let _ = c.execute("quit", json!({}));
    drop(c);
    let begin = begin.ok_or("FILEOP begin missing before the power-off")?;
    let op = fields(&begin, "FILEOP begin")
        .get("op")
        .ok_or("FILEOP begin op")?
        .to_string();
    if !ids(&op).contains(&mid.op) {
        return Err(format!("FILEOP begin op={op} is not record {}", mid.op));
    }
    if text.contains(&format!("FILEOP end op={op} ")) {
        return Err("the interrupted copy reported its end".into());
    }
    let (_, progress_png) = progress?;
    Ok((
        op,
        json!({"listed":{"recycle":1,"moved":3,"bin":1},"mid_record":mid.json(),"strobe_polls":polls,
            "begin":begin,"screens":{"bin":bin_png,"progress":progress_png}}),
    ))
}
fn boot_three(run: &mut Run, record: &mut Value, op: &str) -> Result<Value, String> {
    let mut c = connection(record)?;
    super::p14_shell::require_unit_scale(run, record)?;
    start_app(run, record, &mut c, "file explorer", 1)?;
    let (paused, past) = wait_line(
        run,
        record,
        0,
        &format!("DOOR_P14 FILEOP pause op={op}"),
        60,
    )?;
    if fields(&paused, "FILEOP pause").get("op") != Some(&op) {
        return Err(format!("recovered operation differs: {paused}"));
    }
    if serial(&run.root, record)?
        .get(..past)
        .unwrap_or("")
        .contains(&format!("FILEOP end op={op} "))
    {
        return Err("the interrupted operation ended before it was offered Paused".into());
    }
    thread::sleep(Duration::from_secs(1));
    let (_, paused_png) = shot(run, &mut c, "recovered-paused")?;
    key(&mut c, &["ret"])?;
    let (resume, at) = wait_line(
        run,
        record,
        past,
        &format!("DOOR_P14 FILEOP resume op={op}"),
        20,
    )?;
    let (end, _) = wait_end(run, record, at, op, 180)?;
    let total = BIG
        .iter()
        .map(|n| big(n).map(|b| b.len() as u64))
        .sum::<Result<u64, String>>()?;
    complete(&end, total)?;
    shut_down(run, record, &mut c)?;
    Ok(
        json!({"paused":paused,"resume":resume,"end":end_json(&end),"screens":{"paused":paused_png}}),
    )
}

// ---- independent audit -----------------------------------------------------------

fn audit(raw: &[u8], observed: &Observed, resumed: bool) -> Result<Value, String> {
    let s = snapshot(raw, true, keep_all)?;
    if !s.clean {
        return Err("files journal not clean after shutdown".into());
    }
    if let Some((path, _)) = s.entries.iter().find(|(_, e)| e.pending) {
        return Err(format!("{path} is delete-pending after shutdown"));
    }
    // Copy and move: originals kept, copies moved with their bytes.
    for (rel, source) in MOVED {
        s.require(&under(source), &small(source)?)?;
        s.require(&under(&format!("moved/{rel}")), &small(source)?)?;
    }
    if !s.children(&under("dst")).is_empty() {
        return Err(format!(
            "move left items in dst: {:?}",
            s.children(&under("dst"))
        ));
    }
    // Conflicts.
    for (name, text) in [
        ("x1.txt", "new x1"),
        ("x2.txt", "old x2"),
        ("x3.txt", "old x3"),
        ("x3 (2).txt", "new x3"),
    ] {
        s.require(
            &under(&format!("cdst/{name}")),
            format!("{text}\n").as_bytes(),
        )?;
    }
    for name in ["x1 (2).txt", "x2 (2).txt"] {
        s.absent(&under(&format!("cdst/{name}")))?;
    }
    for n in 1..=3 {
        s.require(
            &under(&format!("adst/y{n}.txt")),
            format!("old y{n}\n").as_bytes(),
        )?;
        s.require(
            &under(&format!("adst/y{n} (2).txt")),
            format!("new y{n}\n").as_bytes(),
        )?;
    }
    if s.children(&under("cdst")).len() != 4 || s.children(&under("adst")).len() != 6 {
        return Err("conflict destinations hold unexpected items".into());
    }
    // Cancelled copy: finished items present, the partial one absent.
    for (index, name) in BIG.iter().enumerate() {
        let path = under(&format!("cancel/{name}"));
        if index < observed.cancelled_done {
            s.require(&path, &big(name)?)?;
        } else {
            s.absent(&path)?;
        }
    }
    if s.children(&under("cancel")).len() != observed.cancelled_done {
        return Err("cancelled copy left extra items".into());
    }
    for name in BIG {
        s.require(&under(&format!("big/{name}")), &big(name)?)?;
    }
    s.require(SEED, &large_seed())?;
    // Recycle Bin: the restored item is back, the kept one is a $R/$I pair.
    s.require(&under("recycle/r1.txt"), &small("recycle/r1.txt")?)?;
    s.absent(&under("recycle/r2.txt"))?;
    let bin = s.entries.get(BIN).ok_or("per-user Recycle Bin missing")?;
    if bin.kind != 2 || bin.owner != UID || bin.attributes & 2 == 0 {
        return Err(format!("Recycle Bin identity/hidden attribute: {bin:?}"));
    }
    let entries = s.children(BIN);
    let infos: Vec<(&str, Info)> = entries
        .iter()
        .filter(|n| n.starts_with("$I"))
        .map(|n| Ok((*n, Info::decode(s.file(&format!("{BIN}/{n}"))?)?)))
        .collect::<Result<_, String>>()?;
    let [(info_name, info)] = infos.as_slice() else {
        return Err(format!("Recycle Bin should hold one item: {entries:?}"));
    };
    let kept = small("recycle/r2.txt")?;
    let item_name = format!("$R{:012X}.txt", info.id);
    if !ids(&observed.kept).contains(&info.id)
        || ids(&observed.restored).contains(&info.id)
        || *info_name != format!("$I{:012X}", info.id)
        || info.original != under("recycle/r2.txt")
        || info.bytes != kept.len() as u64
        || info.items != 1
        || info.folder
        || entries.len() != 2
    {
        return Err(format!("Recycle Bin entries differ: {entries:?} {info:?}"));
    }
    s.require(&format!("{BIN}/{item_name}"), &kept)?;
    // Deletion time against the catalog's own clock for the info file.
    let stamped = s
        .entries
        .get(&format!("{BIN}/{info_name}"))
        .map_or(0, |e| e.created);
    if info.deleted == 0 || stamped == 0 || info.deleted.abs_diff(stamped) > 10 * SECOND {
        return Err(format!(
            "deletion time {} does not match the record's creation {stamped}",
            info.deleted
        ));
    }
    // Permanent delete.
    s.absent(&under("purge/p.txt"))?;
    // Change notification left a new folder in watch/.
    s.require(&under("watch/w.txt"), &small("watch/w.txt")?)?;
    let created: Vec<&str> = s
        .children(&under("watch"))
        .into_iter()
        .filter(|n| {
            s.entries
                .get(&format!("{}/{n}", under("watch")))
                .is_some_and(|e| e.kind == 2)
        })
        .collect();
    if created.len() != 1 {
        return Err(format!(
            "watch/ should hold one created folder: {created:?}"
        ));
    }
    // No temporaries, no records of dismissed operations.
    if !s.temporaries().is_empty() {
        return Err(format!("temporary files left: {:?}", s.temporaries()));
    }
    let records = s.records()?;
    if !records.is_empty() {
        return Err(format!(
            "operation records left after dismissal: {:?}",
            records.iter().map(|r| r.op).collect::<Vec<_>>()
        ));
    }
    if resumed {
        for name in BIG {
            s.require(&under(&format!("interrupted/{name}")), &big(name)?)?;
        }
        if s.children(&under("interrupted")).len() != BIG.len() {
            return Err("resumed copy left extra items".into());
        }
    }
    let identity = |p: &str| s.entries.get(p).map(|e| e.id);
    Ok(json!({
        "journal_clean": true,
        "copied_and_moved": MOVED.len(),
        "dst_empty": true,
        "conflicts": {"replace":"x1.txt","skip":"x2.txt","keep_both":"x3 (2).txt","all_keep_both":3},
        "cancelled": {"finished": observed.cancelled_done, "partial_absent": BIG.get(observed.cancelled_done)},
        "restored": {"path": under("recycle/r1.txt"), "object": identity(&under("recycle/r1.txt"))},
        "bin": {"path": BIN, "hidden": true, "info": info_name, "item": item_name, "id": info.id,
            "original": info.original, "deleted": info.deleted, "info_created": stamped, "bytes": info.bytes},
        "purged_absent": under("purge/p.txt"),
        "watch_created": created,
        "temporaries": 0,
        "operation_records": 0,
        "resumed_copy": resumed,
        "operations": observed.ops,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn op_record(op: u64, state: u8, items: (u32, u32), bytes: (u64, u64), offset: u64) -> Vec<u8> {
        let mut b = vec![0u8; 8192];
        b[..8].copy_from_slice(b"DOORFOP1");
        b[8..12].copy_from_slice(&1u32.to_le_bytes());
        b[16..24].copy_from_slice(&op.to_le_bytes());
        b[24..28].copy_from_slice(&1001u32.to_le_bytes());
        b[30] = state;
        b[32..36].copy_from_slice(&items.1.to_le_bytes());
        b[36..40].copy_from_slice(&items.0.to_le_bytes());
        b[48..56].copy_from_slice(&bytes.1.to_le_bytes());
        b[56..64].copy_from_slice(&bytes.0.to_le_bytes());
        b[66] = 1;
        b[80..88].copy_from_slice(&offset.to_le_bytes());
        let mut at = 232;
        for s in [
            "/users/1001/p14/big",
            "/users/1001/p14/cancel",
            "",
            "",
            "z-large.bin",
        ] {
            b[at..at + 2].copy_from_slice(&(s.len() as u16).to_le_bytes());
            b[at + 2..at + 2 + s.len()].copy_from_slice(s.as_bytes());
            at += 2 + s.len();
        }
        let len = at + 4;
        b[12..16].copy_from_slice(&(len as u32).to_le_bytes());
        let crc = crate::storage::audit::crc32(&b[..at]);
        b[at..at + 4].copy_from_slice(&crc.to_le_bytes());
        b.truncate(len);
        b
    }
    #[test]
    fn operation_records_decode_and_detect_mid_copy() {
        let total = 2 * 30 + LARGE as u64;
        let b = op_record(7, 1, (2, 3), (60 + 65536, total), 65536);
        let r = OpRecord::decode(&b).unwrap();
        assert_eq!(
            (r.op, r.owner, r.items, r.file_offset),
            (7, 1001, (2, 3), 65536)
        );
        assert!(r.mid_copy(3));
        assert!(
            !OpRecord::decode(&op_record(7, 1, (3, 3), (total, total), 0))
                .unwrap()
                .mid_copy(4)
        );
        assert!(
            !OpRecord::decode(&op_record(7, 4, (2, 3), (60, total), 0))
                .unwrap()
                .mid_copy(4)
        );
        let mut bad = b.clone();
        bad[100] ^= 1;
        assert!(OpRecord::decode(&bad).is_err());
    }
    #[test]
    fn recycle_info_decodes() {
        let path = "/users/1001/p14/recycle/r2.txt";
        let len = 48 + path.len() + 4;
        let mut b = vec![0u8; len];
        b[..8].copy_from_slice(b"DOORRCY1");
        b[8..12].copy_from_slice(&1u32.to_le_bytes());
        b[12..16].copy_from_slice(&(len as u32).to_le_bytes());
        b[16..24].copy_from_slice(&0x2au64.to_le_bytes());
        b[24..32].copy_from_slice(&133_000_000_000_000_000u64.to_le_bytes());
        b[32..40].copy_from_slice(&29u64.to_le_bytes());
        b[40..44].copy_from_slice(&1u32.to_le_bytes());
        b[46..48].copy_from_slice(&(path.len() as u16).to_le_bytes());
        b[48..48 + path.len()].copy_from_slice(path.as_bytes());
        let crc = crate::storage::audit::crc32(&b[..len - 4]);
        b[len - 4..].copy_from_slice(&crc.to_le_bytes());
        let info = Info::decode(&b).unwrap();
        assert_eq!(
            (info.id, info.bytes, info.items, info.folder),
            (42, 29, 1, false)
        );
        assert_eq!(info.original, path);
        assert_eq!(format!("$I{:012X}", info.id), "$I00000000002A");
    }
    #[test]
    fn fixtures_are_consistent() {
        assert_eq!(large_seed(), large_seed());
        assert_eq!(large_seed().len(), LARGE);
        for (path, text) in SMALL {
            let parent = path.rsplit_once('/').map_or("", |(p, _)| p);
            assert!(FOLDERS.contains(&parent), "{path}");
            assert!(text.split(' ').all(|w| !w.is_empty()), "{path}");
            for ch in format!("write p14/{path} {text}").chars() {
                session::keys(ch).unwrap();
            }
        }
        // Folder parents precede children.
        for (i, f) in FOLDERS.iter().enumerate() {
            if let Some((parent, _)) = f.rsplit_once('/') {
                assert!(FOLDERS[..i].contains(&parent), "{f}");
            }
        }
        let mut sorted = BIG;
        sorted.sort_unstable();
        assert_eq!(sorted, BIG);
        assert_eq!(small("adst/y1.txt").unwrap().len(), 7);
        for typed in [r"C:\Users\1001\p14\src\tree", "shell:RecycleBinFolder"] {
            for ch in typed.chars() {
                session::keys(ch).unwrap();
            }
        }
    }
    #[test]
    fn serial_fields_and_ids_parse() {
        let line = "DOOR_USER pid=41 DOOR_P14 FILEOP end op=12 state=cancelled items=2/3 bytes=70000/4194364 skipped=0 failed=0";
        let end = parse_end(line).unwrap();
        assert_eq!(end.state, "cancelled");
        assert_eq!((end.items, end.bytes), ((2, 3), (70000, 4_194_364)));
        assert_eq!(ids("12"), vec![12, 0x12]);
        assert_eq!(ids("2A"), vec![42]);
        assert_eq!(
            fields(
                "DOOR_USER pid=9 DOOR_P14 RECYCLE id=5 path=/users/1001/p14/recycle/r1.txt",
                "RECYCLE"
            )
            .get("path"),
            Some(&"/users/1001/p14/recycle/r1.txt")
        );
    }
}
