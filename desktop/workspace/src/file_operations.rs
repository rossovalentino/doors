//! File Explorer file operations (Windows 11 26H2), the decisions only:
//! the file clipboard (Cut / Copy / Paste over the broker's FILES and
//! "Preferred DropEffect" formats), how a paste or drop becomes storage
//! engine operations, the copy progress dialog (percent, speed graph, time
//! and items remaining, minimised pill, several operations in one dialog),
//! the "Replace or Skip Files" conflict dialog, the Recycle Bin view's rows
//! and confirmations, live-refresh selection by identity and the P14 serial
//! diagnostics. `files/operations.rs` drives `Storage::fileop_step` with
//! these models; everything here is host-tested.
extern crate alloc;
use crate::explorer_layout::Bounds;
use alloc::{format, string::String, vec::Vec};
use door_service_protocol::files::{Decision, FileOpMode, OpProgress, OpState, RecycleEntry};
use door_service_protocol::{Code, clipboard as cb, dnd};

/// Event-loop ticks per second (`door_sys::time` runs at 100 Hz).
pub const TICKS_PER_SECOND: u64 = 100;
/// Live refresh polls the folder watch this often (250 ms).
pub const WATCH_TICKS: u64 = 25;
/// Sources one engine operation takes (all in one folder).
pub const MAX_SOURCES: usize = 32;
/// History token of the Recycle Bin view (as `::ThisPC`).
pub const RECYCLE_BIN: &str = "::RecycleBin";
/// The per-user bin folder the storage service keeps (hidden, 0x02).
pub const BIN_FOLDER: &str = "$Recycle.Bin";
/// Listings never show the bin folder as an ordinary folder.
pub fn hidden_name(name: &str) -> bool {
    name.eq_ignore_ascii_case(BIN_FOLDER)
}
/// The per-user bin folder of `home` ("/users/1001/$Recycle.Bin").
pub fn bin_folder(home: &str) -> String {
    format!("{home}/{BIN_FOLDER}")
}

// ---- Paths ----

pub fn parent(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) => "/",
        Some(i) => &path[..i],
        None => "",
    }
}
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
fn within(path: &str, folder: &str) -> bool {
    path == folder
        || path
            .strip_prefix(folder)
            .is_some_and(|rest| rest.starts_with('/') || folder.ends_with('/'))
}
/// What a folder is called in progress text ("Documents", "Local Disk (C:)").
pub fn folder_label(folder: &str) -> String {
    match folder {
        RECYCLE_BIN => "Recycle Bin".into(),
        "/compat" => "Local Disk (C:)".into(),
        "/" | "" => "Door".into(),
        _ => file_name(folder).into(),
    }
}

// ---- File clipboard ----

/// Cut or copied items. File Explorer's Cut is FILES with the MOVE drop
/// effect; Copy is FILES with COPY. Cut items stay drawn faded until pasted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileClipboard {
    pub paths: Vec<String>,
    pub cut: bool,
}
impl FileClipboard {
    pub fn new(paths: Vec<String>, cut: bool) -> Self {
        Self { paths, cut }
    }
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
    /// `path` is waiting to be moved by a paste (drawn faded).
    pub fn is_cut(&self, path: &str) -> bool {
        self.cut && self.paths.iter().any(|p| p == path)
    }
    pub fn effect(&self) -> u32 {
        if self.cut { dnd::MOVE } else { dnd::COPY }
    }
    /// The FILES payload and the "Preferred DropEffect" word.
    pub fn encode(&self) -> Result<(Vec<u8>, [u8; 4]), Code> {
        let refs: Vec<&str> = self.paths.iter().map(String::as_str).collect();
        let mut out = alloc::vec![0u8; cb::MAX_FILES_BYTES];
        let n = cb::encode_files(&refs, &mut out)?;
        out.truncate(n);
        Ok((out, self.effect().to_le_bytes()))
    }
    /// Clipboard contents read back: FILES, and DROP_EFFECT when present
    /// (absent: a copy, as Windows treats files without the effect).
    pub fn decode(files: &[u8], effect: Option<&[u8]>) -> Option<Self> {
        cb::validate(cb::FILES, files).ok()?;
        let paths = cb::files(files)
            .map(|p| p.map(String::from))
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        let cut = effect
            .and_then(|b| b.try_into().ok())
            .map(u32::from_le_bytes)
            == Some(dnd::MOVE);
        Some(Self { paths, cut })
    }
    /// A pasted cut empties the clipboard (the items moved); a copy stays.
    pub fn pasted(&mut self) {
        if self.cut {
            self.paths.clear();
            self.cut = false;
        }
    }
}

// ---- Planning operations ----

/// One storage engine operation: sources in one folder, at most 32.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub mode: FileOpMode,
    /// Destination folder ("" for a permanent delete).
    pub destination: String,
    pub sources: Vec<String>,
}
impl Plan {
    pub fn folder(&self) -> &str {
        self.sources.first().map_or("", |s| parent(s))
    }
}
/// Groups `sources` by folder (first-seen order, duplicates dropped) into
/// operations of at most `MAX_SOURCES`. Moves into the folder they are in
/// do nothing (Windows), and nothing goes into itself or its own subtree.
pub fn plans(sources: &[String], destination: &str, mode: FileOpMode) -> Vec<Plan> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for s in sources {
        if s.is_empty() || !s.starts_with('/') {
            continue;
        }
        if mode != FileOpMode::Delete && within(destination, s) {
            continue;
        }
        let folder = parent(s);
        if mode == FileOpMode::Move && folder == destination {
            continue;
        }
        match groups.iter_mut().find(|(f, _)| f == folder) {
            Some((_, list)) if !list.contains(s) => list.push(s.clone()),
            Some(_) => (),
            None => groups.push((folder.into(), alloc::vec![s.clone()])),
        }
    }
    let destination = if mode == FileOpMode::Delete {
        String::new()
    } else {
        destination.into()
    };
    groups
        .into_iter()
        .flat_map(|(_, list)| {
            list.chunks(MAX_SOURCES)
                .map(|chunk| Plan {
                    mode,
                    destination: destination.clone(),
                    sources: chunk.to_vec(),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}
/// Paste into `destination`: a cut moves, a copy copies.
pub fn paste_plans(clip: &FileClipboard, destination: &str) -> Vec<Plan> {
    let mode = if clip.cut {
        FileOpMode::Move
    } else {
        FileOpMode::Copy
    };
    plans(&clip.paths, destination, mode)
}
/// The drop effect for `sources` dropped into `dest` with `keys`: within a
/// volume a drop moves, across volumes it copies; Ctrl copies, Shift moves.
pub fn drop_effect(sources: &[String], dest: &str, keys: u32) -> u32 {
    let (effects, default) = crate::explorer_dnd::status(sources, dest);
    dnd::resolve(dnd::COPY | dnd::MOVE, effects, default, keys) & (dnd::COPY | dnd::MOVE)
}

// ---- Progress ----

pub fn mode_name(mode: FileOpMode) -> &'static str {
    match mode {
        FileOpMode::Copy => "copy",
        FileOpMode::Move => "move",
        FileOpMode::Delete => "delete",
    }
}
pub fn state_name(state: OpState) -> &'static str {
    match state {
        OpState::Scanning => "scanning",
        OpState::Running => "running",
        OpState::Paused => "paused",
        OpState::Conflict => "conflict",
        OpState::Done => "done",
        OpState::Cancelled => "cancelled",
        OpState::Failed => "failed",
    }
}
pub fn decision_name(decision: Decision) -> &'static str {
    match decision {
        Decision::Replace => "replace",
        Decision::Skip => "skip",
        Decision::KeepBoth => "keepboth",
    }
}
/// Percent complete: by bytes when the operation moves data, else by items.
pub fn percent(p: &OpProgress) -> u32 {
    if p.state == OpState::Done {
        return 100;
    }
    let (done, total) = if p.bytes_total > 0 {
        (p.bytes_done, p.bytes_total)
    } else {
        (u64::from(p.items_done), u64::from(p.items_total))
    };
    if total == 0 {
        return 0;
    }
    (done.min(total) * 100 / total) as u32
}
fn items(n: u32) -> String {
    if n == 1 {
        "1 item".into()
    } else {
        format!("{n} items")
    }
}
/// "Copying 3 items from Documents to Music" / "Deleting 1 item from Recycle Bin".
pub fn summary(mode: FileOpMode, count: u32, from: &str, to: &str) -> String {
    match mode {
        FileOpMode::Copy => format!("Copying {} from {from} to {to}", items(count)),
        FileOpMode::Move => format!("Moving {} from {from} to {to}", items(count)),
        FileOpMode::Delete => format!("Deleting {} from {from}", items(count)),
    }
}
/// The large line: "45% complete", "Paused - 45% complete", "Calculating...".
pub fn headline(p: &OpProgress) -> String {
    match p.state {
        OpState::Scanning => "Calculating...".into(),
        OpState::Paused => format!("Paused - {}% complete", percent(p)),
        OpState::Cancelled => "Cancelled".into(),
        OpState::Failed => "Couldn't finish".into(),
        _ => format!("{}% complete", percent(p)),
    }
}
/// Windows' "Time remaining" wording.
pub fn time_remaining(bytes_left: u64, speed: u64) -> String {
    if speed == 0 {
        return "Calculating...".into();
    }
    let secs = bytes_left.div_ceil(speed).max(1);
    let plural = |n: u64, unit: &str| {
        if n == 1 {
            format!("1 {unit}")
        } else {
            format!("{n} {unit}s")
        }
    };
    if secs < 60 {
        // Windows rounds short estimates up to 5 seconds.
        let rounded = if secs <= 5 {
            secs
        } else {
            secs.div_ceil(5) * 5
        };
        return format!("About {}", plural(rounded, "second"));
    }
    if secs < 3600 {
        let (m, s) = (secs / 60, secs % 60);
        return if s == 0 {
            format!("About {}", plural(m, "minute"))
        } else {
            format!("About {} and {}", plural(m, "minute"), plural(s, "second"))
        };
    }
    let (h, m) = (secs / 3600, secs % 3600 / 60);
    if m == 0 {
        format!("About {}", plural(h, "hour"))
    } else {
        format!("About {} and {}", plural(h, "hour"), plural(m, "minute"))
    }
}
/// "Items remaining: 3 (1.2 MB)".
pub fn items_remaining(p: &OpProgress) -> String {
    let left = p.items_total.saturating_sub(p.items_done);
    let bytes = p.bytes_total.saturating_sub(p.bytes_done);
    format!("{left} ({})", crate::explorer_preview::size(bytes))
}
pub fn speed_text(speed: u64) -> String {
    format!("Speed: {}/s", crate::explorer_preview::size(speed))
}

/// Transfer speed over time for the "More details" graph. Windows plots
/// speed against completion: one sample per percent reached, a filled area
/// up to the current percent and a line at the current speed.
#[derive(Clone, Debug, Default)]
pub struct SpeedGraph {
    /// (tick, bytes done) over the last two seconds.
    window: Vec<(u64, u64)>,
    /// Bytes per second at each percent 0..=current.
    pub samples: Vec<u64>,
    pub peak: u64,
    /// Current speed (bytes per second); 0 until measured.
    pub speed: u64,
}
impl SpeedGraph {
    pub fn record(&mut self, now: u64, bytes_done: u64, percent: u32) {
        if self
            .window
            .last()
            .is_some_and(|(t, b)| *t > now || *b > bytes_done)
        {
            // Time or progress went backwards (resumed after a reboot).
            self.window.clear();
        }
        self.window.push((now, bytes_done));
        while self.window.len() > 2 && now.saturating_sub(self.window[1].0) >= 2 * TICKS_PER_SECOND
        {
            self.window.remove(0);
        }
        let (t0, b0) = self.window[0];
        if now.saturating_sub(t0) >= TICKS_PER_SECOND / 4 {
            self.speed = (bytes_done - b0) * TICKS_PER_SECOND / (now - t0);
        }
        let percent = percent.min(100) as usize;
        while self.samples.len() <= percent {
            self.samples.push(self.speed);
        }
        if let Some(last) = self.samples.last_mut() {
            *last = self.speed;
        }
        self.peak = self.peak.max(self.speed);
    }
    /// Paused or interrupted: the next measurement starts afresh.
    pub fn interrupt(&mut self) {
        self.window.clear();
        self.speed = 0;
    }
    /// Filled-area column heights for a `width` x `height` plot: column c
    /// shows the sample at percent c * 100 / width; columns past the
    /// current percent are empty.
    pub fn columns(&self, width: u32, height: u32) -> Vec<u32> {
        let peak = self.peak.max(1);
        (0..width)
            .map(|c| {
                let at = (c as usize * 100) / width.max(1) as usize;
                self.samples
                    .get(at)
                    .map_or(0, |s| (s * u64::from(height) / peak) as u32)
            })
            .collect()
    }
    /// Height of the current-speed line in a plot of `height`.
    pub fn line(&self, height: u32) -> u32 {
        (self.speed * u64::from(height) / self.peak.max(1)) as u32
    }
}

/// Where the progress dialog's parts are. One section per operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Section {
    pub summary: Bounds,
    pub headline: Bounds,
    pub pause: Bounds,
    pub cancel: Bounds,
    /// Thin bar (Fewer details) or the speed graph (More details).
    pub meter: Bounds,
    /// Name / Time remaining / Items remaining (More details).
    pub info: Bounds,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressLayout {
    pub panel: Bounds,
    /// The title bar's "45% complete".
    pub caption: Bounds,
    /// The title bar's maximize (disabled) and close buttons; close cancels
    /// the operations, as closing Windows' dialog does.
    pub maximize: Bounds,
    pub close: Bounds,
    pub sections: Vec<Section>,
    pub toggle: Bounds,
    /// More details shown (it falls back to fewer when they cannot fit).
    pub details: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressHit {
    Close,
    Toggle,
    Pause(usize),
    Cancel(usize),
    Panel,
}
/// Windows 11 26H2's copy dialog (measured at 150%): 447 wide, a 32 px
/// title bar, 26 px margins, one 89 px section per operation and a 44 px
/// "More details" footer under a separator.
const WIDTH: u32 = 447;
const PAD: i32 = 26;
pub const CAPTION: u32 = 32;
pub const FOOTER: u32 = 44;
const COMPACT: u32 = 89;
const DETAILED: u32 = 208;
pub const INFO_LINE: i32 = 20;
impl ProgressLayout {
    /// The dialog for `jobs` operations in a `width` x `height` window,
    /// centred; sections beyond the room left are not shown.
    pub fn new(width: u32, height: u32, jobs: usize, details: bool) -> Self {
        let w = width.saturating_sub(16).min(WIDTH);
        let room = height.saturating_sub(16 + CAPTION + FOOTER);
        let details = details && room >= DETAILED;
        let pitch = if details { DETAILED } else { COMPACT };
        let shown = jobs.min((room / pitch).max(1) as usize).max(1);
        let h = (CAPTION + FOOTER + pitch * shown as u32).min(height.saturating_sub(16));
        let panel = Bounds {
            x: (width.saturating_sub(w) / 2) as i32,
            y: (height.saturating_sub(h) / 2) as i32,
            width: w,
            height: h,
        };
        let inner = w.saturating_sub(2 * PAD as u32);
        let title_button = |right: i32| Bounds {
            x: panel.x + w as i32 - right,
            y: panel.y,
            width: 46,
            height: CAPTION,
        };
        let sections = (0..shown)
            .map(|i| {
                let top = panel.y + CAPTION as i32 + (i as u32 * pitch) as i32;
                let x = panel.x + PAD;
                // Pause and cancel centre 81 and 34 px in from the right.
                let button = |centre: i32| Bounds {
                    x: panel.x + w as i32 - centre - 14,
                    y: top + 28,
                    width: 28,
                    height: 28,
                };
                Section {
                    summary: Bounds {
                        x,
                        y: top + 11,
                        width: inner,
                        height: 20,
                    },
                    headline: Bounds {
                        x,
                        y: top + 30,
                        width: inner.saturating_sub(100),
                        height: 24,
                    },
                    pause: button(81),
                    cancel: button(34),
                    meter: Bounds {
                        x,
                        y: top + 59,
                        width: inner,
                        height: if details { 72 } else { 15 },
                    },
                    info: Bounds {
                        x,
                        y: top + 142,
                        width: if details { inner } else { 0 },
                        height: if details { 3 * INFO_LINE as u32 } else { 0 },
                    },
                }
            })
            .collect();
        Self {
            panel,
            caption: Bounds {
                x: panel.x + 31,
                y: panel.y,
                width: w.saturating_sub(31 + 100),
                height: CAPTION,
            },
            maximize: title_button(92),
            close: title_button(46),
            sections,
            toggle: Bounds {
                x: panel.x + 18,
                y: panel.y + h as i32 - FOOTER as i32 + 8,
                width: 150.min(inner),
                height: 28,
            },
            details,
        }
    }
    pub fn hit(&self, x: i32, y: i32) -> Option<ProgressHit> {
        if !self.panel.contains(x, y) {
            return None;
        }
        if self.close.contains(x, y) {
            return Some(ProgressHit::Close);
        }
        if self.toggle.contains(x, y) {
            return Some(ProgressHit::Toggle);
        }
        for (i, s) in self.sections.iter().enumerate() {
            if s.pause.contains(x, y) {
                return Some(ProgressHit::Pause(i));
            }
            if s.cancel.contains(x, y) {
                return Some(ProgressHit::Cancel(i));
            }
        }
        Some(ProgressHit::Panel)
    }
}
/// The minimised dialog: a pill at the window's bottom right, above the
/// status bar; clicking it reopens the dialog.
pub fn pill(width: u32, height: u32) -> Bounds {
    let w = width.saturating_sub(16).min(280);
    Bounds {
        x: width.saturating_sub(w + 12) as i32,
        y: height as i32 - crate::explorer_layout::status_height() as i32 - 44,
        width: w,
        height: 36,
    }
}
/// Caption of the dialog: "45% complete" for one operation, "2 operations
/// running" for several (as Windows titles its combined dialog).
pub fn caption(progress: &[OpProgress]) -> String {
    match progress {
        [one] => headline(one),
        many => format!("{} operations running", many.len()),
    }
}

// ---- Conflicts ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictStage {
    /// "Replace or Skip Files": replace all, skip all, or decide per file.
    Overview,
    /// "1 File Conflict": keep both / replace / skip this file.
    PerFile,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictChoice {
    ReplaceAll,
    SkipAll,
    Decide,
    Replace,
    Skip,
    KeepBoth,
    /// The dialog's close button: the operation is cancelled.
    Cancel,
}
/// The conflict dialog of one operation. "Let me decide for each file"
/// keeps later conflicts of the operation in the per-file stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConflictDialog {
    pub op: u64,
    pub stage: ConflictStage,
    /// "Do this for the next N conflicts" is checked.
    pub apply_rest: bool,
    pub focus: usize,
}
/// What a choice does: answer the engine, switch stage, or cancel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Resolve(Decision, bool),
    PerFile,
    Cancel,
}
impl ConflictDialog {
    pub fn new(op: u64, per_file: bool) -> Self {
        Self {
            op,
            stage: if per_file {
                ConflictStage::PerFile
            } else {
                ConflictStage::Overview
            },
            apply_rest: false,
            focus: 0,
        }
    }
    pub fn choose(&mut self, choice: ConflictChoice) -> Resolution {
        use ConflictChoice as C;
        match (self.stage, choice) {
            (_, C::Cancel) => Resolution::Cancel,
            (ConflictStage::Overview, C::ReplaceAll) => {
                Resolution::Resolve(Decision::Replace, true)
            }
            (ConflictStage::Overview, C::SkipAll) => Resolution::Resolve(Decision::Skip, true),
            (ConflictStage::Overview, _) => {
                self.stage = ConflictStage::PerFile;
                self.focus = 0;
                Resolution::PerFile
            }
            (ConflictStage::PerFile, C::Replace | C::ReplaceAll) => {
                Resolution::Resolve(Decision::Replace, self.apply_rest)
            }
            (ConflictStage::PerFile, C::Skip | C::SkipAll) => {
                Resolution::Resolve(Decision::Skip, self.apply_rest)
            }
            (ConflictStage::PerFile, C::KeepBoth | C::Decide) => {
                Resolution::Resolve(Decision::KeepBoth, self.apply_rest)
            }
        }
    }
    /// The choices in focus order for this stage.
    pub fn choices(&self) -> &'static [ConflictChoice] {
        match self.stage {
            ConflictStage::Overview => &[
                ConflictChoice::ReplaceAll,
                ConflictChoice::SkipAll,
                ConflictChoice::Decide,
            ],
            ConflictStage::PerFile => &[
                ConflictChoice::KeepBoth,
                ConflictChoice::Replace,
                ConflictChoice::Skip,
                ConflictChoice::Cancel,
            ],
        }
    }
    /// Tab / Shift+Tab / arrows move the focus; Enter or Space chooses;
    /// Esc cancels. Returns the choice to perform.
    pub fn key(&mut self, scan: u32, shift: bool) -> Option<ConflictChoice> {
        let n = self.choices().len();
        match scan {
            1 => Some(ConflictChoice::Cancel),
            15 | 0x148 | 0x150 | 0x14b | 0x14d => {
                let back = (scan == 15 && shift) || matches!(scan, 0x148 | 0x14b);
                self.focus = (self.focus + if back { n - 1 } else { 1 }) % n;
                None
            }
            28 | 0x11c | 57 => self.choices().get(self.focus).copied(),
            _ => None,
        }
    }
}
/// "Do this for the next N conflicts": N counts the items still to come
/// (an upper bound; the engine reports conflicts one at a time). None when
/// this is the last item.
pub fn rest_label(p: &OpProgress) -> Option<String> {
    let rest = p.items_total.saturating_sub(p.items_done).saturating_sub(1);
    (rest > 0).then(|| {
        if rest == 1 {
            "Do this for the next conflict".into()
        } else {
            format!("Do this for the next {rest} conflicts")
        }
    })
}
/// The conflict dialog's headline: `The destination has a file named "x"`.
pub fn conflict_question(name: &str, directory: bool) -> String {
    if directory {
        format!("The destination already has a folder named \"{name}\"")
    } else {
        format!("The destination already has a file named \"{name}\"")
    }
}
pub const OVERVIEW_OPTIONS: [&str; 3] = [
    "Replace the files in the destination",
    "Skip these files",
    "Let me decide for each file",
];
/// Windows' wording when the conflicting item is the operation's last one.
pub const OVERVIEW_OPTIONS_ONE: [&str; 3] = [
    "Replace the file in the destination",
    "Skip this file",
    "Compare info for both files",
];
/// The overview's choices for this conflict: plural while later items may
/// still conflict, singular for the last item.
pub fn overview_options(p: &OpProgress) -> [&'static str; 3] {
    if rest_label(p).is_some() {
        OVERVIEW_OPTIONS
    } else {
        OVERVIEW_OPTIONS_ONE
    }
}
pub const PER_FILE_BUTTONS: [&str; 4] = ["Keep both", "Replace", "Skip", "Cancel"];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConflictLayout {
    pub panel: Bounds,
    pub close: Bounds,
    /// Summary and question lines.
    pub text: Bounds,
    /// Overview: three option rows.
    pub options: [Bounds; 3],
    /// Per file: source and destination facts.
    pub cards: [Bounds; 2],
    pub checkbox: Bounds,
    /// Per file: Keep both, Replace, Skip, Cancel.
    pub buttons: [Bounds; 4],
}
impl ConflictLayout {
    pub fn new(width: u32, height: u32, stage: ConflictStage) -> Self {
        let w = width.saturating_sub(16).min(480);
        let want = match stage {
            ConflictStage::Overview => 300,
            ConflictStage::PerFile => 312,
        };
        let h = height.saturating_sub(16).min(want);
        let panel = Bounds {
            x: (width.saturating_sub(w) / 2) as i32,
            y: (height.saturating_sub(h) / 2) as i32,
            width: w,
            height: h,
        };
        let inner = w.saturating_sub(2 * PAD as u32);
        let x = panel.x + PAD;
        let zero = Bounds {
            x,
            y: panel.y,
            width: 0,
            height: 0,
        };
        let text = Bounds {
            x,
            y: panel.y + 40,
            width: inner,
            height: 64,
        };
        let mut options = [zero; 3];
        let mut cards = [zero; 2];
        let mut buttons = [zero; 4];
        let mut checkbox = zero;
        match stage {
            ConflictStage::Overview => {
                for (i, o) in options.iter_mut().enumerate() {
                    *o = Bounds {
                        x,
                        y: panel.y + 112 + i as i32 * 56,
                        width: inner,
                        height: 48,
                    };
                }
            }
            ConflictStage::PerFile => {
                let half = inner.saturating_sub(12) / 2;
                for (i, c) in cards.iter_mut().enumerate() {
                    *c = Bounds {
                        x: x + i as i32 * (half as i32 + 12),
                        y: panel.y + 108,
                        width: half,
                        height: 92,
                    };
                }
                checkbox = Bounds {
                    x,
                    y: panel.y + 210,
                    width: inner,
                    height: 28,
                };
                let gap = 8;
                let bw = inner.saturating_sub(3 * gap) / 4;
                for (i, b) in buttons.iter_mut().enumerate() {
                    *b = Bounds {
                        x: x + i as i32 * (bw + gap) as i32,
                        y: panel.y + h as i32 - 48,
                        width: bw,
                        height: 32,
                    };
                }
            }
        }
        Self {
            panel,
            close: Bounds {
                x: panel.x + w as i32 - 44,
                y: panel.y + 4,
                width: 40,
                height: 28,
            },
            text,
            options,
            cards,
            checkbox,
            buttons,
        }
    }
    /// What a click at (`x`, `y`) chooses; `Some(None)` toggles the
    /// checkbox, `None` misses every control.
    pub fn hit(&self, stage: ConflictStage, x: i32, y: i32) -> Option<Option<ConflictChoice>> {
        if self.close.contains(x, y) {
            return Some(Some(ConflictChoice::Cancel));
        }
        match stage {
            ConflictStage::Overview => {
                let all = [
                    ConflictChoice::ReplaceAll,
                    ConflictChoice::SkipAll,
                    ConflictChoice::Decide,
                ];
                self.options
                    .iter()
                    .zip(all)
                    .find(|(b, _)| b.contains(x, y))
                    .map(|(_, c)| Some(c))
            }
            ConflictStage::PerFile => {
                if self.checkbox.contains(x, y) {
                    return Some(None);
                }
                let all = [
                    ConflictChoice::KeepBoth,
                    ConflictChoice::Replace,
                    ConflictChoice::Skip,
                    ConflictChoice::Cancel,
                ];
                self.buttons
                    .iter()
                    .zip(all)
                    .find(|(b, _)| b.contains(x, y))
                    .map(|(_, c)| Some(c))
            }
        }
    }
}

// ---- Deletion and the Recycle Bin ----

/// Windows' permanent-delete confirmation (Shift+Delete): (title, question).
pub fn delete_question(count: usize, directory: bool) -> (&'static str, String) {
    match (count, directory) {
        (1, false) => (
            "Delete File",
            "Are you sure you want to permanently delete this file?".into(),
        ),
        (1, true) => (
            "Delete Folder",
            "Are you sure you want to permanently delete this folder?".into(),
        ),
        (n, _) => (
            "Delete Multiple Items",
            format!("Are you sure you want to permanently delete these {n} items?"),
        ),
    }
}
/// A recycle refused because the item is larger than the bin (`TooBig`).
pub fn too_big_question(directory: bool) -> (&'static str, &'static str) {
    if directory {
        (
            "Delete Folder",
            "This folder is too big to recycle. Do you want to permanently delete it?",
        )
    } else {
        (
            "Delete File",
            "This file is too big to recycle. Do you want to permanently delete it?",
        )
    }
}
/// "Empty Recycle Bin" confirmation for `count` entries.
pub fn empty_question(count: usize) -> (&'static str, String) {
    if count == 1 {
        (
            "Delete File",
            "Are you sure you want to permanently delete this item?".into(),
        )
    } else {
        (
            "Delete Multiple Items",
            format!("Are you sure you want to permanently delete these {count} items?"),
        )
    }
}
/// Details columns of the Recycle Bin view.
pub const RECYCLE_COLUMNS: [&str; 5] = [
    "Name",
    "Original Location",
    "Date Deleted",
    "Size",
    "Item type",
];
/// Column (x, width) pairs across `width` starting at `x`: Name and
/// Original Location share the slack, the rest keep their Windows widths.
pub fn recycle_columns(x: i32, width: u32) -> [(i32, u32); 5] {
    let fixed = [150u32, 80, 120];
    let rest = width.saturating_sub(fixed.iter().sum::<u32>());
    let name = (rest * 45 / 100).max(120);
    let location = rest.saturating_sub(name).max(100);
    let widths = [name, location, fixed[0], fixed[1], fixed[2]];
    let mut out = [(x, 0); 5];
    let mut at = x;
    for (o, w) in out.iter_mut().zip(widths) {
        *o = (at, w);
        at += w as i32;
    }
    out
}
/// One Recycle Bin row as listed (dates are formatted by the caller's
/// locale).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecycleRow {
    pub id: u64,
    pub name: String,
    /// "C:\Users\owner\Documents" (the original folder).
    pub location: String,
    pub deleted: u64,
    pub bytes: u64,
    pub directory: bool,
    pub kind: &'static str,
}
impl RecycleRow {
    pub fn new(entry: &RecycleEntry) -> Self {
        let name = String::from(entry.name());
        let folder = entry.folder();
        let mut location = crate::explorer_path::display(folder);
        if entry.truncated {
            location.insert_str(0, "...");
        }
        Self {
            id: entry.id,
            kind: crate::explorer_preview::Kind::of(&name, entry.directory).label(),
            name,
            location,
            deleted: entry.deleted,
            bytes: entry.bytes,
            directory: entry.directory,
        }
    }
    /// Size column: blank for folders, as Windows shows them.
    pub fn size(&self) -> String {
        if self.directory {
            String::new()
        } else {
            crate::explorer_preview::size(self.bytes)
        }
    }
}
/// Object identities of synthesized Recycle Bin list entries: kept apart
/// from filesystem objects so tags and pins never match them.
pub const RECYCLE_OBJECT: u64 = 1 << 62;
pub fn recycle_object(id: u64) -> u64 {
    RECYCLE_OBJECT | id
}
pub fn recycle_id(object: u64) -> Option<u64> {
    (object & RECYCLE_OBJECT != 0).then_some(object & !RECYCLE_OBJECT)
}

// ---- Live refresh ----

/// After a re-list, the item that had focus and the marked items, found by
/// object identity in `after` (never by row position). Vanished items drop
/// out of the selection.
pub fn keep_selection(
    after: &[u64],
    focus: Option<u64>,
    marked: &[u64],
) -> (Option<usize>, Vec<u64>) {
    let selected = focus.and_then(|o| after.iter().position(|a| *a == o));
    let marked: Vec<u64> = marked
        .iter()
        .copied()
        .filter(|o| after.contains(o))
        .collect();
    let marked = if marked.len() < 2 { Vec::new() } else { marked };
    (selected, marked)
}
/// What the folder watch must do for the folder now shown (`want`, None for
/// Home, This PC and search results).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchAction {
    Keep,
    Subscribe,
    Unsubscribe(u64),
    /// Unsubscribe the token, then subscribe to the new folder.
    Replace(u64),
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Watch {
    pub token: Option<u64>,
    pub folder: String,
    /// The folder whose subscription failed; not retried until it changes.
    pub failed: Option<String>,
    pub next_poll: u64,
}
impl Watch {
    pub fn action(&self, want: Option<&str>) -> WatchAction {
        match (self.token, want) {
            (Some(t), None) => WatchAction::Unsubscribe(t),
            (Some(t), Some(w)) if w != self.folder => WatchAction::Replace(t),
            (Some(_), Some(_)) => WatchAction::Keep,
            (None, Some(w)) if self.failed.as_deref() != Some(w) => WatchAction::Subscribe,
            (None, _) => WatchAction::Keep,
        }
    }
    /// Tokens die with the storage connection: subscribe again later.
    pub fn connection_lost(&mut self) {
        self.token = None;
        self.failed = None;
    }
    /// The poll is due at `now` (every `WATCH_TICKS`).
    pub fn due(&mut self, now: u64) -> bool {
        if now < self.next_poll {
            return false;
        }
        self.next_poll = now + WATCH_TICKS;
        true
    }
}

// ---- Serial diagnostics (guest runner) ----

pub fn diag_begin(op: u64, mode: FileOpMode, sources: usize) -> String {
    format!(
        "DOOR_P14 FILEOP begin op={op} mode={} sources={sources}\n",
        mode_name(mode)
    )
}
pub fn diag_conflict(op: u64, name: &str) -> String {
    format!("DOOR_P14 FILEOP conflict op={op} name={name}\n")
}
pub fn diag_resolve(op: u64, decision: Decision, all: bool) -> String {
    format!(
        "DOOR_P14 FILEOP resolve op={op} decision={} all={}\n",
        decision_name(decision),
        u8::from(all)
    )
}
pub fn diag_control(op: u64, action: &str) -> String {
    format!("DOOR_P14 FILEOP {action} op={op}\n")
}
pub fn diag_end(p: &OpProgress) -> String {
    format!(
        "DOOR_P14 FILEOP end op={} state={} items={}/{} bytes={}/{} skipped={} failed={}\n",
        p.op,
        state_name(p.state),
        p.items_done,
        p.items_total,
        p.bytes_done,
        p.bytes_total,
        p.skipped,
        p.failures
    )
}

// ---- Recycle Bin glyph ----

/// Coverage of the bin glyph at design point (x, y) in 1/64 units of a
/// 16-unit box: an open bin (rim, tapered body outline, two ribs) and, when
/// full, paper above the rim and a filled interior.
const fn bin_coverage(full: bool, x: i32, y: i32) -> u8 {
    const U: i32 = 64;
    // Rim.
    if y >= 3 * U && y < 4 * U && x >= 2 * U && x < 14 * U {
        return 255;
    }
    // Body: outer trapezoid 5..15, 3..13 at the top to 4..12 at the bottom.
    if y >= 5 * U && y < 15 * U {
        let shift = (y - 5 * U) / 10;
        let (left, right) = (3 * U + shift, 13 * U - shift);
        if x >= left && x < right {
            let inner = y >= 6 * U && y < 14 * U && x >= left + U && x < right - U;
            if !inner {
                return 255;
            }
            let rib = (x >= 6 * U && x < 7 * U) || (x >= 9 * U && x < 10 * U);
            if rib && y >= 7 * U && y < 13 * U {
                return 255;
            }
            return if full { 150 } else { 0 };
        }
    }
    if full {
        // Two sheets of paper standing above the rim.
        if (y >= U / 2 && y < 3 * U && x >= 4 * U && x < 8 * U)
            || (y >= 3 * U / 2 && y < 3 * U && x >= 9 * U && x < 12 * U)
        {
            return 255;
        }
    }
    0
}
const fn bin<const N: usize>(full: bool) -> [u8; N] {
    let side = isqrt(N);
    let mut out = [0u8; N];
    let mut py = 0;
    while py < side {
        let mut px = 0;
        while px < side {
            let mut sum = 0u32;
            let mut sy = 0;
            while sy < 4 {
                let mut sx = 0;
                while sx < 4 {
                    let x = (((px * 4 + sx) * 2 + 1) * 128 / side) as i32;
                    let y = (((py * 4 + sy) * 2 + 1) * 128 / side) as i32;
                    sum += bin_coverage(full, x, y) as u32;
                    sx += 1;
                }
                sy += 1;
            }
            out[py * side + px] = (sum / 16) as u8;
            px += 1;
        }
        py += 1;
    }
    out
}
const fn isqrt(n: usize) -> usize {
    let mut s = 0;
    while (s + 1) * (s + 1) <= n {
        s += 1;
    }
    s
}
macro_rules! bins {
    ($($px:literal),*) => {
        /// The Recycle Bin glyph (empty or full) as `px` x `px` coverage, for
        /// the native mask frames (12, 16, 20 logical at 1x, 2x, 3x).
        pub fn bin_mask(full: bool, px: usize) -> &'static [u8] {
            $(
                {
                    static EMPTY: [u8; $px * $px] = bin::<{ $px * $px }>(false);
                    static FULL: [u8; $px * $px] = bin::<{ $px * $px }>(true);
                    if px == $px {
                        return if full { &FULL } else { &EMPTY };
                    }
                }
            )*
            bin_mask(full, 20)
        }
    };
}
bins!(12, 16, 20, 24, 32, 36, 40, 48, 60);

#[cfg(test)]
mod tests {
    use super::*;
    use door_service_protocol::files::ConflictInfo;
    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|p| String::from(*p)).collect()
    }
    fn progress(state: OpState, items: (u32, u32), bytes: (u64, u64)) -> OpProgress {
        OpProgress::new(
            7,
            FileOpMode::Copy,
            state,
            items,
            0,
            0,
            bytes,
            None,
            (state == OpState::Conflict).then(ConflictInfo::default),
            "a.txt",
        )
    }
    #[test]
    fn clipboard_round_trips_cut_and_copy_and_a_pasted_cut_empties() {
        let paths = s(&["/users/1/Documents/a.txt", "/users/1/Documents/b"]);
        let mut cut = FileClipboard::new(paths.clone(), true);
        let (files, effect) = cut.encode().unwrap();
        assert_eq!(u32::from_le_bytes(effect), dnd::MOVE);
        let back = FileClipboard::decode(&files, Some(&effect)).unwrap();
        assert_eq!(back, cut);
        assert!(back.is_cut("/users/1/Documents/a.txt"));
        assert!(!back.is_cut("/users/1/Documents/c.txt"));
        // No effect format: Windows treats the files as copied.
        let copy = FileClipboard::decode(&files, None).unwrap();
        assert!(!copy.cut && !copy.is_cut("/users/1/Documents/a.txt"));
        cut.pasted();
        assert!(cut.is_empty());
        let mut copied = FileClipboard::new(paths, false);
        copied.pasted();
        assert_eq!(copied.paths.len(), 2, "a copy can be pasted again");
        assert!(FileClipboard::decode(b"relative", None).is_none());
    }
    #[test]
    fn pastes_group_by_folder_chunk_at_32_and_skip_impossible_targets() {
        let mut sources: Vec<String> = (0..40).map(|i| format!("/a/f{i}")).collect();
        sources.push("/b/x".into());
        sources.push("/a/f0".into());
        let p = plans(&sources, "/d", FileOpMode::Copy);
        assert_eq!(p.len(), 3);
        assert_eq!(
            (p[0].sources.len(), p[1].sources.len(), p[2].sources.len()),
            (32, 8, 1)
        );
        assert!(p.iter().all(|p| p.destination == "/d"));
        assert_eq!(p[2].folder(), "/b");
        // Cut-paste into the same folder does nothing; a folder never goes
        // into itself or its own subtree.
        let clip = FileClipboard::new(s(&["/d/a.txt", "/x/y"]), true);
        let p = paste_plans(&clip, "/d");
        assert_eq!(p.len(), 1);
        assert_eq!(
            (p[0].mode, p[0].sources[0].as_str()),
            (FileOpMode::Move, "/x/y")
        );
        assert!(plans(&s(&["/x/y"]), "/x/y/z", FileOpMode::Copy).is_empty());
        // Copy into the same folder is allowed ("name - Copy").
        assert_eq!(plans(&s(&["/d/a.txt"]), "/d", FileOpMode::Copy).len(), 1);
        let del = plans(&s(&["/d/a.txt"]), "/ignored", FileOpMode::Delete);
        assert_eq!(del[0].destination, "");
    }
    #[test]
    fn drops_move_within_a_volume_ctrl_copies_and_shift_moves() {
        let a = s(&["/users/1/Documents/a.txt"]);
        assert_eq!(drop_effect(&a, "/users/1/Music", 0), dnd::MOVE);
        assert_eq!(
            drop_effect(&a, "/users/1/Music", dnd::KEY_CONTROL),
            dnd::COPY
        );
        assert_eq!(drop_effect(&a, "/users/1/Music", dnd::KEY_SHIFT), dnd::MOVE);
        // Ctrl+Shift links in Windows; files cannot be linked here.
        assert_eq!(
            drop_effect(&a, "/users/1/Music", dnd::KEY_CONTROL | dnd::KEY_SHIFT),
            dnd::NONE
        );
        assert_eq!(drop_effect(&a, "/users/1/Documents", 0), dnd::NONE);
    }
    #[test]
    fn percent_and_texts_follow_bytes_then_items() {
        let p = progress(OpState::Running, (4, 1), (1000, 450));
        assert_eq!(percent(&p), 45);
        assert_eq!(headline(&p), "45% complete");
        assert_eq!(percent(&progress(OpState::Running, (4, 1), (0, 0))), 25);
        assert_eq!(percent(&progress(OpState::Done, (0, 0), (0, 0))), 100);
        assert_eq!(percent(&progress(OpState::Running, (0, 0), (0, 0))), 0);
        assert_eq!(
            headline(&progress(OpState::Paused, (4, 1), (1000, 450))),
            "Paused - 45% complete"
        );
        assert_eq!(
            headline(&progress(OpState::Scanning, (0, 0), (0, 0))),
            "Calculating..."
        );
        assert_eq!(items_remaining(&p), "3 (550 bytes)");
        assert_eq!(
            summary(FileOpMode::Copy, 3, "Documents", "Music"),
            "Copying 3 items from Documents to Music"
        );
        assert_eq!(
            summary(FileOpMode::Delete, 1, "Recycle Bin", ""),
            "Deleting 1 item from Recycle Bin"
        );
        assert_eq!(caption(&[p, p]), "2 operations running");
        assert_eq!(folder_label(RECYCLE_BIN), "Recycle Bin");
        assert_eq!(folder_label("/compat"), "Local Disk (C:)");
    }
    #[test]
    fn time_remaining_reads_like_windows() {
        assert_eq!(time_remaining(100, 0), "Calculating...");
        assert_eq!(time_remaining(1, 10), "About 1 second");
        assert_eq!(time_remaining(30, 10), "About 3 seconds");
        assert_eq!(time_remaining(110, 10), "About 15 seconds");
        assert_eq!(time_remaining(600, 10), "About 1 minute");
        assert_eq!(time_remaining(800, 10), "About 1 minute and 20 seconds");
        assert_eq!(time_remaining(36_000, 10), "About 1 hour");
        assert_eq!(time_remaining(39_000, 10), "About 1 hour and 5 minutes");
        assert_eq!(speed_text(3 << 20), "Speed: 3.0 MB/s");
    }
    #[test]
    fn speed_samples_fill_by_percent_and_restart_after_a_pause() {
        let mut g = SpeedGraph::default();
        g.record(0, 0, 0);
        assert_eq!(g.speed, 0, "no rate before a quarter second");
        g.record(50, 1000, 10);
        assert_eq!(g.speed, 2000);
        assert_eq!(g.samples.len(), 11);
        g.record(100, 4000, 40);
        assert_eq!(g.speed, 4000, "rate over the window since the first sample");
        assert_eq!(g.peak, 4000);
        assert_eq!(g.samples.len(), 41);
        // Percent 11..=40 carry the speed measured when they were reached.
        assert_eq!(g.samples[40], 4000);
        let cols = g.columns(100, 50);
        assert_eq!(cols[40], 50);
        assert_eq!(cols[41], 0, "nothing past the current percent");
        assert_eq!(g.line(50), 50);
        g.interrupt();
        g.record(1000, 4000, 40);
        assert_eq!(g.speed, 0, "the paused time is not averaged in");
        g.record(1050, 5000, 50);
        assert_eq!(g.speed, 2000);
        // Progress going backwards (recovered after reboot) restarts.
        g.record(10, 0, 0);
        assert_eq!(g.window.len(), 1);
    }
    #[test]
    fn progress_layout_keeps_controls_inside_and_falls_back_when_small() {
        for (w, h) in [(1000, 640), (500, 360), (300, 240), (2400, 1400)] {
            for jobs in 1..=4 {
                for details in [false, true] {
                    let l = ProgressLayout::new(w, h, jobs, details);
                    assert!(!l.sections.is_empty());
                    assert!(l.panel.x >= 0 && l.panel.y >= 0);
                    assert!(l.panel.x as u32 + l.panel.width <= w);
                    assert!(l.panel.y as u32 + l.panel.height <= h);
                    for s in &l.sections {
                        for b in [s.pause, s.cancel] {
                            assert!(l.panel.contains(b.x, b.y));
                            assert_eq!(
                                l.hit(b.x + 1, b.y + 1).map(|h| matches!(
                                    h,
                                    ProgressHit::Pause(_) | ProgressHit::Cancel(_)
                                )),
                                Some(true)
                            );
                        }
                        assert!(s.pause.x + s.pause.width as i32 <= s.cancel.x);
                    }
                    assert_eq!(
                        l.hit(l.close.x + 1, l.close.y + 1),
                        Some(ProgressHit::Close)
                    );
                    assert_eq!(
                        l.hit(l.toggle.x + 1, l.toggle.y + 1),
                        Some(ProgressHit::Toggle)
                    );
                    assert_eq!(l.hit(-1, -1), None);
                }
            }
        }
        let big = ProgressLayout::new(1000, 800, 2, true);
        assert!(big.details && big.sections.len() == 2);
        assert!(big.sections[0].meter.height > 6, "the speed graph");
        let small = ProgressLayout::new(500, 240, 3, true);
        assert!(!small.details, "too small for More details");
        let p = pill(1000, 640);
        assert!(p.x as u32 + p.width <= 1000 && p.y > 0);
    }
    #[test]
    fn conflicts_resolve_all_or_per_file_with_apply_to_all() {
        let mut d = ConflictDialog::new(9, false);
        assert_eq!(
            d.clone().choose(ConflictChoice::ReplaceAll),
            Resolution::Resolve(Decision::Replace, true)
        );
        assert_eq!(
            d.clone().choose(ConflictChoice::SkipAll),
            Resolution::Resolve(Decision::Skip, true)
        );
        assert_eq!(d.choose(ConflictChoice::Decide), Resolution::PerFile);
        assert_eq!(d.stage, ConflictStage::PerFile);
        assert_eq!(
            d.choose(ConflictChoice::KeepBoth),
            Resolution::Resolve(Decision::KeepBoth, false)
        );
        d.apply_rest = true;
        assert_eq!(
            d.choose(ConflictChoice::Skip),
            Resolution::Resolve(Decision::Skip, true)
        );
        assert_eq!(d.choose(ConflictChoice::Cancel), Resolution::Cancel);
        // Keyboard: Tab cycles, Enter chooses, Esc cancels.
        let mut k = ConflictDialog::new(9, false);
        assert_eq!(k.key(15, false), None);
        assert_eq!(k.key(28, false), Some(ConflictChoice::SkipAll));
        assert_eq!(k.key(15, true), None);
        assert_eq!(k.key(57, false), Some(ConflictChoice::ReplaceAll));
        assert_eq!(k.key(1, false), Some(ConflictChoice::Cancel));
        let p = progress(OpState::Conflict, (5, 2), (10, 5));
        assert_eq!(
            rest_label(&p).as_deref(),
            Some("Do this for the next 2 conflicts")
        );
        assert_eq!(
            rest_label(&progress(OpState::Conflict, (3, 2), (1, 0))),
            None
        );
        assert_eq!(
            conflict_question("a.txt", false),
            "The destination already has a file named \"a.txt\""
        );
    }
    #[test]
    fn conflict_layout_hits_every_control_once() {
        for (w, h) in [(1000, 640), (480, 360), (2000, 1200)] {
            let o = ConflictLayout::new(w, h, ConflictStage::Overview);
            for (b, c) in o.options.iter().zip([
                ConflictChoice::ReplaceAll,
                ConflictChoice::SkipAll,
                ConflictChoice::Decide,
            ]) {
                assert!(o.panel.contains(b.x, b.y + b.height as i32 - 1));
                assert_eq!(
                    o.hit(ConflictStage::Overview, b.x + 2, b.y + 2),
                    Some(Some(c))
                );
            }
            let f = ConflictLayout::new(w, h, ConflictStage::PerFile);
            assert_eq!(
                f.hit(ConflictStage::PerFile, f.checkbox.x + 2, f.checkbox.y + 2),
                Some(None)
            );
            for b in f.buttons {
                assert!(f.panel.contains(b.x, b.y + b.height as i32 - 1));
                assert!(f.hit(ConflictStage::PerFile, b.x + 1, b.y + 1).is_some());
            }
            assert_eq!(
                f.hit(ConflictStage::PerFile, f.close.x + 1, f.close.y + 1),
                Some(Some(ConflictChoice::Cancel))
            );
        }
    }
    #[test]
    fn deletion_questions_and_recycle_rows_match_windows() {
        assert_eq!(
            delete_question(1, false).1,
            "Are you sure you want to permanently delete this file?"
        );
        assert_eq!(delete_question(1, true).0, "Delete Folder");
        assert_eq!(
            delete_question(3, false),
            (
                "Delete Multiple Items",
                "Are you sure you want to permanently delete these 3 items?".into()
            )
        );
        assert_eq!(
            too_big_question(false).1,
            "This file is too big to recycle. Do you want to permanently delete it?"
        );
        assert_eq!(
            empty_question(4).1,
            "Are you sure you want to permanently delete these 4 items?"
        );
        let e = RecycleEntry::new(
            5,
            133_000_000_000_000_000,
            2048,
            1,
            false,
            "/users/1001/Documents/report.txt",
        );
        let row = RecycleRow::new(&e);
        assert_eq!(row.name, "report.txt");
        assert_eq!(row.location, "C:\\Users\\1001\\Documents");
        assert_eq!(row.kind, "Text document");
        assert_eq!(row.size(), "2.0 KB");
        let folder = RecycleRow::new(&RecycleEntry::new(6, 0, 9, 3, true, "/compat/Old"));
        assert_eq!((folder.size().as_str(), folder.kind), ("", "File folder"));
        assert_eq!(recycle_id(recycle_object(42)), Some(42));
        assert_eq!(recycle_id(42), None);
        let cols = recycle_columns(200, 800);
        assert_eq!(cols[0].0, 200);
        for w in cols.windows(2) {
            assert_eq!(w[0].0 + w[0].1 as i32, w[1].0, "columns abut");
        }
        assert!(hidden_name("$RECYCLE.BIN") && !hidden_name("Recycle Bin"));
        assert_eq!(bin_folder("/users/7"), "/users/7/$Recycle.Bin");
    }
    #[test]
    fn refresh_keeps_selection_by_identity_and_watch_follows_the_folder() {
        let (sel, marked) = keep_selection(&[30, 10, 20], Some(10), &[10, 20, 99]);
        assert_eq!(sel, Some(1), "found by object, not row");
        assert_eq!(marked, [10, 20]);
        let (sel, marked) = keep_selection(&[30], Some(10), &[10, 30]);
        assert_eq!(
            (sel, marked.len()),
            (None, 0),
            "gone items leave the selection"
        );
        let mut w = Watch::default();
        assert_eq!(w.action(Some("/a")), WatchAction::Subscribe);
        w.token = Some(3);
        w.folder = "/a".into();
        assert_eq!(w.action(Some("/a")), WatchAction::Keep);
        assert_eq!(w.action(Some("/b")), WatchAction::Replace(3));
        assert_eq!(w.action(None), WatchAction::Unsubscribe(3));
        w.connection_lost();
        assert_eq!(w.action(Some("/a")), WatchAction::Subscribe);
        w.failed = Some("/a".into());
        assert_eq!(w.action(Some("/a")), WatchAction::Keep, "no retry storm");
        assert_eq!(w.action(Some("/b")), WatchAction::Subscribe);
        assert!(w.due(0) && !w.due(10) && w.due(25));
    }
    #[test]
    fn diagnostics_are_exact_single_lines() {
        assert_eq!(
            diag_begin(4, FileOpMode::Move, 2),
            "DOOR_P14 FILEOP begin op=4 mode=move sources=2\n"
        );
        assert_eq!(
            diag_resolve(4, Decision::KeepBoth, true),
            "DOOR_P14 FILEOP resolve op=4 decision=keepboth all=1\n"
        );
        assert_eq!(diag_control(4, "pause"), "DOOR_P14 FILEOP pause op=4\n");
        assert_eq!(
            diag_conflict(4, "a.txt"),
            "DOOR_P14 FILEOP conflict op=4 name=a.txt\n"
        );
        let mut p = progress(OpState::Done, (3, 3), (10, 10));
        p.skipped = 1;
        assert_eq!(
            diag_end(&p),
            "DOOR_P14 FILEOP end op=7 state=done items=3/3 bytes=10/10 skipped=1 failed=0\n"
        );
    }
    #[test]
    fn empty_and_full_bin_glyphs_differ_at_every_frame() {
        for px in [12, 16, 20, 24, 32, 36, 40, 48, 60] {
            let (empty, full) = (bin_mask(false, px), bin_mask(true, px));
            assert_eq!(empty.len(), px * px);
            assert_eq!(full.len(), px * px);
            let ink = |m: &[u8]| m.iter().map(|v| u32::from(*v)).sum::<u32>();
            assert!(ink(empty) > 0 && ink(full) > ink(empty) * 5 / 4, "{px}");
        }
        assert_eq!(bin_mask(false, 7).len(), 400, "other sizes use 20 px");
    }
}

#[cfg(test)]
mod location_tests {
    use super::*;
    use crate::explorer_search::Open;
    #[test]
    fn the_desktop_can_open_explorer_at_the_recycle_bin() {
        let open = Open::Location {
            folder: RECYCLE_BIN.into(),
            select: None,
        };
        assert_eq!(Open::decode(&open.encode()), Some(open));
        assert!(Open::decode(b"L::Elsewhere").is_none());
    }
}
