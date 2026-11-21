//! File operations in File Explorer (P14), as Windows 11 26H2 does them:
//! Cut / Copy / Paste through the session clipboard broker (FILES with the
//! "Preferred DropEffect"; an in-process copy when no broker is offered),
//! the copy progress dialog driving `Storage::fileop_step` from the tick
//! (percent, speed graph, time and items remaining, pause/resume/cancel,
//! minimised pill, several operations in one dialog, interrupted operations
//! offered after a restart), the "Replace or Skip Files" conflict dialog,
//! Recycle Bin deletion, confirmations and view, and live refresh from the
//! storage folder watch. Decisions live in `door_workspace::file_operations`.
use super::*;
use core::sync::atomic::{AtomicBool, Ordering};
use door_service_client::window::WindowClient;
use door_service_client::{FileInfo, FileTimes};
use door_service_protocol::clipboard as cb;
use door_service_protocol::files::{
    ConflictPolicy, FileOpMode, OpProgress, OpState, RecycleOutcome,
};
use door_workspace::dialog::{Dialog, Layout as PromptLayout};
use door_workspace::file_operations::{
    self as fo, ConflictChoice, ConflictDialog, ConflictLayout, ConflictStage, FileClipboard, Plan,
    ProgressHit, ProgressLayout, RecycleRow, Resolution, SpeedGraph, Watch, WatchAction,
};
use modern::paint::{HOVER, MUTED, SELECTED, TEXT, label};

/// Interrupted operations are offered once per process (its first window).
static RECOVERED: AtomicBool = AtomicBool::new(false);
/// Engine steps (64 KiB or 16 namespace changes each) one tick may spend.
const STEPS_PER_TICK: usize = 16;
/// Operations still running after a second show the dialog; quicker ones
/// finish without it, as in Windows.
const SHOW_AFTER: u64 = 100;
/// A new operation first runs inline for up to this long (ticks) and this
/// many steps, so copying, moving or deleting a few items completes at once
/// with no dialog, as in Windows; longer work continues from the tick.
const QUICK_TICKS: u64 = 10;
const QUICK_STEPS: usize = 512;
/// The dialog's open (fade and grow) and close (fade and shrink) animations,
/// in ticks; an approximation of the DWM window transitions.
const OPEN_TICKS: u64 = 20;
const CLOSE_TICKS: u64 = 15;
/// The bin's item count is re-read this often for the empty/full icon.
const BIN_TICKS: u64 = 500;
/// Hit-test ids: prompt buttons, conflict choices, progress dialog parts.
pub(super) const PROMPT: u32 = 3010;
pub(super) const CONFLICT: u32 = 3020;
pub(super) const CONFLICT_CHECK: u32 = 3029;
pub(super) const PROGRESS: u32 = 3100;
const CHOICES: [ConflictChoice; 7] = [
    ConflictChoice::ReplaceAll,
    ConflictChoice::SkipAll,
    ConflictChoice::Decide,
    ConflictChoice::KeepBoth,
    ConflictChoice::Replace,
    ConflictChoice::Skip,
    ConflictChoice::Cancel,
];
/// Copy dialog colours (BGRA), from Windows 11 26H2 at 150%.
const GREEN: [u8; 4] = [15, 123, 15, 255];
const PAUSED: [u8; 4] = [0, 176, 236, 255];
const FAILED: [u8; 4] = [28, 43, 196, 255];
const BAR_BORDER: [u8; 4] = [188, 188, 188, 255];
const BAR_TRACK: [u8; 4] = [230, 230, 230, 255];
const LINK: [u8; 4] = [204, 102, 0, 255];
const SEPARATOR: [u8; 4] = [223, 223, 223, 255];
const CLOSE_HOVER: [u8; 4] = [28, 43, 196, 255];

pub(super) struct Job {
    pub(super) progress: OpProgress,
    pub(super) summary: String,
    pub(super) graph: SpeedGraph,
    started: Option<u64>,
    /// In the dialog (after `SHOW_AFTER`, a conflict, a pause or a restart);
    /// shorter work never shows it.
    pub(super) shown: bool,
    conflict_logged: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PromptKind {
    /// "Too big to recycle": Yes deletes `paths` permanently.
    TooBig,
    /// "Empty Recycle Bin": Yes empties it.
    EmptyBin,
    /// Delete bin entries permanently: Yes purges `ids`.
    Purge,
    /// One button (restore failures, properties).
    Notice,
}
pub(super) struct Prompt {
    pub(super) dialog: Dialog,
    pub(super) kind: PromptKind,
    pub(super) title: String,
    pub(super) body: String,
    paths: Vec<String>,
    ids: Vec<u64>,
}
impl Prompt {
    pub(super) fn labels(&self) -> &'static [&'static str] {
        if self.kind == PromptKind::Notice {
            &["OK"]
        } else {
            &["Yes", "No"]
        }
    }
}
/// Clipboard work owed to the broker (`window_update` has the client).
pub(super) enum Clip {
    Set,
    Text(String),
    Paste(String),
}
#[derive(Default)]
pub(super) struct Ops {
    pub(super) jobs: Vec<Job>,
    pub(super) minimized: bool,
    /// The last tick's time, for the dialog animations.
    now: u64,
    /// When the dialog appeared (its open animation runs from there).
    opened_at: Option<u64>,
    /// The last operation's dialog, closing (fading out) since the tick.
    fading: Option<(Job, u64)>,
    pub(super) details: bool,
    /// What this window cut or copied (also the clipboard without a broker).
    pub(super) clipboard: FileClipboard,
    /// The broker holds files (None: no broker, or not asked yet).
    broker_files: Option<bool>,
    /// Sequence of the broker clipboard this window set.
    sequence: Option<u32>,
    queue: Vec<Clip>,
    pub(super) prompt: Option<Prompt>,
    pub(super) conflict: Option<ConflictDialog>,
    /// Operations whose conflicts are decided per file.
    per_file: Vec<u64>,
    pub(super) watch: Watch,
    /// Permanent delete waiting for its confirmation (`Files::removal`).
    pub(super) pending_delete: Vec<String>,
    pending_folder: bool,
    /// Bin entries purged one operation at a time.
    purge_queue: Vec<u64>,
    /// Rows of the Recycle Bin view (by id, as `entries`).
    pub(super) bin_rows: Vec<RecycleRow>,
    /// Items in the bin (None: not read yet); picks the empty/full icon.
    pub(super) bin_items: Option<u32>,
    bin_read_at: u64,
    /// The full/empty state last reported to the desktop (SET_RECYCLE_BIN).
    bin_reported: Option<bool>,
}

impl Files {
    /// The bin's full/empty state once it changed since the last report
    /// (the desktop Recycle Bin icon follows it).
    pub(crate) fn take_bin_report(&mut self) -> Option<bool> {
        let full = if self.recycle_view() {
            !self.entries.is_empty()
        } else {
            self.ops.bin_items? > 0
        };
        (self.ops.bin_reported != Some(full)).then(|| {
            self.ops.bin_reported = Some(full);
            full
        })
    }
    /// The Recycle Bin view is shown (`::RecycleBin`).
    pub(super) fn recycle_view(&self) -> bool {
        self.cwd == RECYCLE_BIN
    }
    /// The bin glyph: full while it holds items.
    pub(super) fn bin_icon(&self) -> Icon {
        let full = if self.recycle_view() {
            !self.entries.is_empty()
        } else {
            self.ops.bin_items.unwrap_or(0) > 0
        };
        if full {
            Icon::RecycleBinFull
        } else {
            Icon::RecycleBin
        }
    }
    /// A system object, the profile or a known folder: never cut or deleted.
    fn protected(&self, path: &str, entry: &DirEntry) -> bool {
        entry.info.object <= 16
            || path == self.home
            || FOLDERS.iter().any(|(name, _)| {
                path.strip_prefix(self.home.as_str())
                    .and_then(|rest| rest.strip_prefix('/'))
                    == Some(*name)
            })
    }
    /// Paths of the selected listed items (marked ones, else the focused one).
    pub(super) fn selection_paths(&self) -> Vec<String> {
        if (self.overview && self.search.is_empty()) || self.recycle_view() {
            return Vec::new();
        }
        self.shown()
            .filter(|(i, _)| self.is_selected(*i))
            .filter_map(|(_, e)| self.path_for(e))
            .collect()
    }
    /// The selection that may be cut or deleted: (path, is a folder).
    fn removable(&self) -> Vec<(String, bool)> {
        if (self.overview && self.search.is_empty()) || self.recycle_view() {
            return Vec::new();
        }
        self.shown()
            .filter(|(i, _)| self.is_selected(*i))
            .filter_map(|(_, e)| self.path_for(e).map(|p| (p, e)))
            .filter(|(p, e)| !self.protected(p, e))
            .map(|(p, e)| (p, e.info.directory))
            .collect()
    }
    /// Why Cut or Delete is unavailable for the selection, if it is.
    pub(super) fn delete_block(&self) -> Option<&'static str> {
        if !self.removable().is_empty() {
            return None;
        }
        if self.selection_count() == 0 {
            return Some(context_menu::SELECT_FIRST);
        }
        self.rename_block()
            .or(Some("This item is protected by the system."))
    }
    pub(super) fn copy_block(&self) -> Option<&'static str> {
        self.selection_paths()
            .is_empty()
            .then_some(context_menu::SELECT_FIRST)
    }
    /// Where Paste puts items: the folder shown.
    fn paste_target(&self) -> Option<String> {
        (!self.home_view && !self.overview && !self.recycle_view() && self.result_paths.is_empty())
            .then(|| self.cwd.clone())
    }
    pub(super) fn paste_block(&self) -> Option<&'static str> {
        if self.paste_target().is_none() {
            return Some("Open a folder to paste into it.");
        }
        let files = self
            .ops
            .broker_files
            .unwrap_or(!self.ops.clipboard.is_empty());
        (!files).then_some("There's nothing to paste.")
    }

    // ---- Clipboard ----

    /// Ctrl+C / Ctrl+X: the selection goes on the clipboard; cut items fade.
    pub(super) fn copy_selection(&mut self, cut: bool) {
        let paths: Vec<String> = if cut {
            self.removable().into_iter().map(|(p, _)| p).collect()
        } else {
            self.selection_paths()
        };
        if paths.is_empty() {
            return;
        }
        self.ops.clipboard = FileClipboard::new(paths, cut);
        self.ops.queue.push(Clip::Set);
    }
    /// "Copy as path": quoted Windows paths, one per line, as text.
    pub(super) fn copy_as_path(&mut self) {
        let paths = self.selection_paths();
        if paths.is_empty() {
            return;
        }
        let text: Vec<String> = paths
            .iter()
            .map(|p| format!("\"{}\"", explorer_path::display(p)))
            .collect();
        self.ops.queue.push(Clip::Text(text.join("\r\n")));
    }
    /// Ctrl+V: paste into the folder shown.
    pub(super) fn paste(&mut self) {
        if let Some(dest) = self.paste_target() {
            self.ops.queue.push(Clip::Paste(dest));
        }
    }
    /// The broker changed (CHANGED event): another owner's data replaces
    /// what this window cut, which stops fading.
    pub(crate) fn clipboard_changed(&mut self, sequence: u32, formats: u32) {
        self.ops.broker_files = Some(formats & cb::mask(cb::FILES) != 0);
        if self.ops.sequence != Some(sequence) {
            self.ops.sequence = None;
            self.ops.clipboard = FileClipboard::default();
        }
    }
    /// Clipboard requests owed to the broker; pastes start operations.
    pub(crate) fn clipboard_update(
        &mut self,
        storage: &mut Storage,
        client: &mut WindowClient,
        window: u64,
    ) -> bool {
        let broker = client.supports_clipboard();
        if broker && self.ops.broker_files.is_none() && self.ops.sequence.is_none() {
            self.ops.broker_files = Some(
                client
                    .clipboard_info()
                    .is_ok_and(|i| i.formats & cb::mask(cb::FILES) != 0),
            );
        }
        let mut dirty = false;
        for clip in core::mem::take(&mut self.ops.queue) {
            match clip {
                Clip::Set if broker => match self.ops.clipboard.encode() {
                    Ok((files, effect)) => {
                        match client.clipboard_set(
                            window,
                            &[(cb::FILES, &files), (cb::DROP_EFFECT, &effect)],
                        ) {
                            Ok(sequence) => {
                                self.ops.sequence = Some(sequence);
                                self.ops.broker_files = Some(true);
                            }
                            // The in-process clipboard still pastes here.
                            Err(_) => self.ops.broker_files = None,
                        }
                    }
                    Err(_) => {
                        self.message = "These items can't be put on the clipboard.".into();
                        self.ops.clipboard = FileClipboard::default();
                    }
                },
                Clip::Text(text) => {
                    if broker
                        && client
                            .clipboard_set(window, &[(cb::TEXT, text.as_bytes())])
                            .is_ok()
                    {
                        self.ops.clipboard = FileClipboard::default();
                        self.ops.sequence = None;
                        self.ops.broker_files = Some(false);
                    } else {
                        self.message = context_menu::NO_CLIPBOARD.into();
                    }
                }
                Clip::Paste(dest) => {
                    let clip = if broker && self.ops.broker_files.is_some() {
                        Self::read_broker(client).unwrap_or_default()
                    } else {
                        self.ops.clipboard.clone()
                    };
                    let plans = fo::paste_plans(&clip, &dest);
                    if self.start_plans(storage, plans) > 0 && clip.cut {
                        // A pasted cut empties the clipboard (the items moved).
                        self.ops.clipboard.pasted();
                        if broker && self.ops.sequence.is_some() {
                            let _ = client.clipboard_set(window, &[]);
                            self.ops.sequence = None;
                            self.ops.broker_files = Some(false);
                        }
                    }
                    dirty = true;
                }
                Clip::Set => (),
            }
        }
        dirty
    }
    fn read_broker(client: &mut WindowClient) -> Option<FileClipboard> {
        let info = client.clipboard_info().ok()?;
        if info.formats & cb::mask(cb::FILES) == 0 {
            return None;
        }
        let mut files = alloc::vec![0u8; info.size(cb::FILES) as usize];
        let n = client
            .clipboard_read_into(&info, cb::FILES, &mut files)
            .ok()?;
        let mut effect = [0u8; 4];
        let effect = (info.formats & cb::mask(cb::DROP_EFFECT) != 0)
            .then(|| client.clipboard_read_into(&info, cb::DROP_EFFECT, &mut effect))
            .and_then(Result::ok)
            .map(|n| &effect[..n]);
        FileClipboard::decode(&files[..n], effect)
    }

    // ---- Operations ----

    /// Begins each plan as an engine operation; returns how many started.
    pub(super) fn start_plans(&mut self, storage: &mut Storage, plans: Vec<Plan>) -> usize {
        let started = self.begin_plans(storage, plans).len();
        self.run_quick(storage);
        started
    }
    /// Begins each plan as an engine operation; returns their ids.
    fn begin_plans(&mut self, storage: &mut Storage, plans: Vec<Plan>) -> Vec<u64> {
        // Release the preview pane's read first, as Explorer releases its
        // preview handler: an item deleted or moved while the pane still
        // holds it open would stay delete-pending (or in use) until the read
        // finished. The selection's preview is read again afterwards.
        self.cancel_preview(storage);
        let mut ops = Vec::new();
        for plan in plans {
            match self.start_plan(storage, &plan) {
                Ok(()) => ops.extend(self.ops.jobs.last().map(|j| j.progress.op)),
                Err(e) => self.message = Self::operation_error(plan.mode, &e),
            }
        }
        ops
    }
    /// New operations run here first, before the next paint: a few items
    /// finish with no dialog.
    fn run_quick(&mut self, storage: &mut Storage) {
        if self.ops.jobs.iter().any(|j| j.started.is_none()) {
            let now = door_sys::time().unwrap_or(self.ops.now);
            self.step_jobs(storage, now, now + QUICK_TICKS, QUICK_STEPS);
        }
    }
    fn start_plan(&mut self, storage: &mut Storage, plan: &Plan) -> Result<(), Error> {
        let op = storage.fileop_begin(plan.mode, &plan.destination, ConflictPolicy::Ask)?;
        for source in &plan.sources {
            if let Err(e) = storage.fileop_add(op, source) {
                let _ = storage.fileop_dismiss(op);
                return Err(e);
            }
        }
        let (from, to) = (
            fo::folder_label(plan.folder()),
            fo::folder_label(&plan.destination),
        );
        let summary = fo::summary(plan.mode, plan.sources.len() as u32, &from, &to);
        self.push_job(op, plan.mode, summary, plan.sources.len());
        Ok(())
    }
    fn push_job(&mut self, op: u64, mode: FileOpMode, summary: String, sources: usize) {
        door_rt::diagnostic(format_args!("{}", fo::diag_begin(op, mode, sources)));
        self.ops.jobs.push(Job {
            progress: OpProgress::new(
                op,
                mode,
                OpState::Scanning,
                (0, 0),
                0,
                0,
                (0, 0),
                None,
                None,
                "",
            ),
            summary,
            graph: SpeedGraph::default(),
            started: None,
            shown: false,
            conflict_logged: false,
        });
    }
    pub(super) fn operation_error(mode: FileOpMode, error: &Error) -> String {
        let verb = match mode {
            FileOpMode::Copy => "copy",
            FileOpMode::Move => "move",
            FileOpMode::Delete => "delete",
        };
        let why = match error {
            Error::Service(Code::InvalidRequest) => {
                "The destination folder is a subfolder of the source folder."
            }
            Error::Service(Code::Denied) => "You need permission to perform this action.",
            Error::Service(Code::Resources) => "There isn't enough space on Local Disk (C:).",
            Error::Service(Code::NotFound) => "Couldn't find this item.",
            Error::Service(Code::SharingViolation) => "The item is open in another app.",
            _ => "The storage service didn't complete the request.",
        };
        format!("Couldn't {verb} the items. {why}")
    }
    /// Per tick: offer interrupted operations once, step running ones,
    /// poll the folder watch and the bin's item count.
    pub(crate) fn operations_tick(&mut self, storage: &mut Storage, now: u64) -> bool {
        let mut dirty = false;
        if !RECOVERED.swap(true, Ordering::Relaxed) {
            dirty |= self.recover(storage);
        }
        self.ops.now = now;
        dirty |= self.step_jobs(storage, now, now + 1, STEPS_PER_TICK);
        dirty |= self.animate(now);
        dirty |= self.watch_step(storage, now);
        if self.ops.bin_items.is_none() || now >= self.ops.bin_read_at {
            self.ops.bin_read_at = now + BIN_TICKS;
            let items = storage.recycle_usage().ok().map(|u| u.items);
            dirty |= items.is_some() && items != self.ops.bin_items;
            self.ops.bin_items = items.or(Some(0));
        }
        dirty
    }
    /// After a restart the engine lists interrupted operations as Paused:
    /// the dialog offers Resume (the pause button) and Cancel.
    fn recover(&mut self, storage: &mut Storage) -> bool {
        let mut after = 0;
        let mut found = false;
        for _ in 0..16 {
            let Ok(Some(p)) = storage.fileop_list(after) else {
                break;
            };
            after = p.op;
            if p.state != OpState::Paused || self.ops.jobs.iter().any(|j| j.progress.op == p.op) {
                continue;
            }
            let summary = format!(
                "{} (interrupted by a restart)",
                fo::summary(p.mode, p.items_total, "", "")
                    .split(" from ")
                    .next()
                    .unwrap_or("")
            );
            // Offered Paused: the dialog's Resume continues it, Cancel ends it.
            door_rt::diagnostic(format_args!("{}", fo::diag_control(p.op, "pause")));
            self.ops.jobs.push(Job {
                progress: p,
                summary,
                graph: SpeedGraph::default(),
                started: None,
                shown: true,
                conflict_logged: false,
            });
            self.ops.minimized = false;
            found = true;
        }
        found
    }
    /// Steps running operations until `deadline` (ticks) or `steps` engine
    /// steps; hidden ones repaint nothing until they show or finish.
    fn step_jobs(&mut self, storage: &mut Storage, now: u64, deadline: u64, steps: usize) -> bool {
        let mut dirty = self.next_purge(storage);
        let mut budget = steps;
        let mut opened = false;
        let was_open = self.ops.jobs.iter().any(|j| j.shown);
        for job in &mut self.ops.jobs {
            let started = *job.started.get_or_insert(now);
            if !job.shown
                && (now.saturating_sub(started) >= SHOW_AFTER
                    || matches!(job.progress.state, OpState::Conflict | OpState::Paused))
            {
                job.shown = true;
                opened = true;
                dirty = true;
                // The progress dialog appears (the guest scenarios wait for
                // it before driving its keys).
                door_rt::diagnostic(format_args!(
                    "DOOR_P14 FILEOP shown op={}\n",
                    job.progress.op
                ));
            }
            while budget > 0 && matches!(job.progress.state, OpState::Scanning | OpState::Running) {
                budget -= 1;
                match storage.fileop_step(job.progress.op) {
                    Ok(p) => {
                        job.progress = p;
                        job.graph.record(now, p.bytes_done, fo::percent(&p));
                    }
                    Err(e) => {
                        job.progress.state = OpState::Failed;
                        job.progress.conflict = None;
                        job.progress.last_error = match e {
                            Error::Service(code) => Some(code),
                            _ => Some(Code::Io),
                        };
                    }
                }
                dirty |= job.shown;
                if door_sys::time().is_ok_and(|t| t > deadline) {
                    budget = 0;
                }
            }
            if job.progress.state == OpState::Conflict {
                if !job.conflict_logged {
                    job.conflict_logged = true;
                    opened |= !job.shown;
                    job.shown = true;
                    dirty = true;
                    door_rt::diagnostic(format_args!(
                        "{}",
                        fo::diag_conflict(job.progress.op, job.progress.name())
                    ));
                }
            } else {
                job.conflict_logged = false;
            }
        }
        if self.ops.conflict.is_none()
            && let Some(op) = self
                .ops
                .jobs
                .iter()
                .find(|j| j.progress.state == OpState::Conflict)
                .map(|j| j.progress.op)
        {
            self.ops.conflict = Some(ConflictDialog::new(op, self.ops.per_file.contains(&op)));
            self.ops.minimized = false;
            dirty = true;
        }
        if opened && !was_open {
            // The dialog appears with the window open animation.
            self.ops.opened_at = Some(now);
            self.ops.fading = None;
        }
        while let Some(i) = self
            .ops
            .jobs
            .iter()
            .position(|j| j.progress.state.finished())
        {
            let job = self.ops.jobs.remove(i);
            self.finish_job(storage, &job);
            if job.shown && !self.ops.minimized && !self.ops.jobs.iter().any(|j| j.shown) {
                // The last operation's dialog closes with the close animation.
                self.ops.fading = Some((job, now));
                self.ops.opened_at = None;
            }
            dirty = true;
        }
        dirty
    }
    /// Advances the dialog animations; true while one needs a repaint.
    fn animate(&mut self, now: u64) -> bool {
        let mut dirty = false;
        if let Some(at) = self.ops.opened_at {
            dirty = true;
            if now.saturating_sub(at) > OPEN_TICKS {
                self.ops.opened_at = None;
            }
        }
        if let Some((_, at)) = &self.ops.fading {
            dirty = true;
            if now.saturating_sub(*at) > CLOSE_TICKS {
                self.ops.fading = None;
            }
        }
        dirty
    }
    /// (alpha, scale per mille) of the dialog while it opens or closes.
    fn dialog_motion(&self) -> Option<(u8, u32)> {
        let ease = |t: u64, len: u64| t.min(len) as f32 / len as f32;
        if self.shown_jobs().next().is_some() {
            let t = ease(self.ops.now.saturating_sub(self.ops.opened_at?), OPEN_TICKS);
            if t >= 1.0 {
                return None;
            }
            // Decelerating: fades in while growing from 95%.
            let e = 1.0 - (1.0 - t) * (1.0 - t) * (1.0 - t);
            return Some(((e * 255.0) as u8, 950 + (e * 50.0) as u32));
        }
        let (_, at) = self.ops.fading.as_ref()?;
        let t = ease(self.ops.now.saturating_sub(*at), CLOSE_TICKS);
        // Accelerating: fades out while shrinking to 96%.
        let e = t * t;
        Some((((1.0 - e) * 255.0) as u8, 1000 - (e * 40.0) as u32))
    }
    /// The operations the dialog shows: the running ones, else the last one
    /// while its dialog closes.
    fn dialog_jobs(&self) -> Vec<&Job> {
        let shown: Vec<&Job> = self.shown_jobs().collect();
        if !shown.is_empty() {
            return shown;
        }
        match &self.ops.fading {
            Some((job, at)) if self.ops.now.saturating_sub(*at) <= CLOSE_TICKS => alloc::vec![job],
            _ => Vec::new(),
        }
    }
    fn finish_job(&mut self, storage: &mut Storage, job: &Job) {
        let p = job.progress;
        door_rt::diagnostic(format_args!("{}", fo::diag_end(&p)));
        let _ = storage.fileop_dismiss(p.op);
        self.ops.per_file.retain(|o| *o != p.op);
        if self.ops.conflict.is_some_and(|c| c.op == p.op) {
            self.ops.conflict = None;
        }
        let verb = match p.mode {
            FileOpMode::Copy => "copied",
            FileOpMode::Move => "moved",
            FileOpMode::Delete => "deleted",
        };
        match p.state {
            OpState::Failed => {
                self.message =
                    Self::operation_error(p.mode, &Error::Service(p.last_error.unwrap_or(Code::Io)))
            }
            OpState::Done if p.failures > 0 => {
                self.message = format!("{} item(s) couldn't be {verb}.", p.failures)
            }
            // The P12 removal marker the deletion scenarios and the shutdown
            // retirement audit read: every item of a permanent delete is gone.
            OpState::Done if p.mode == FileOpMode::Delete => door_rt::diagnostic(format_args!(
                "DOOR_P12 FILES REMOVED op={} items={}\n",
                p.op, p.items_done
            )),
            _ => (),
        }
        if self.ops.minimized && job.shown {
            self.toast(job);
        }
        if self.ops.jobs.iter().all(|j| !j.shown) {
            self.ops.minimized = false;
        }
        self.ops.bin_items = None;
        // The finished operation's items show at once, as in Windows; the
        // folder watch's later notification only confirms them.
        let _ = self.relist(storage);
    }
    /// "Operation finished" toast, only when the dialog was minimised.
    fn toast(&self, job: &Job) {
        if self.registry == 0 {
            return;
        }
        let title = match (job.progress.mode, job.progress.state) {
            (FileOpMode::Copy, OpState::Done) => "Copy complete",
            (FileOpMode::Move, OpState::Done) => "Move complete",
            (FileOpMode::Delete, OpState::Done) => "Delete complete",
            (_, OpState::Cancelled) => "Operation cancelled",
            _ => "Operation couldn't finish",
        };
        let mut end = job.summary.len().min(120);
        while !job.summary.is_char_boundary(end) {
            end -= 1;
        }
        if let Ok(mut n) = door_service_client::Notifications::connect(self.registry) {
            let _ = n.post(&door_service_client::notifications::Toast {
                icon: door_service_protocol::shell::icon::FILES,
                flags: 0,
                title,
                body: &job.summary[..end],
                actions: ["", ""],
            });
        }
    }
    /// Pause/resume (`pause`) or cancel one operation.
    fn control_job(&mut self, op: u64, pause: bool, storage: &mut Storage) {
        let Some(job) = self.ops.jobs.iter().find(|j| j.progress.op == op) else {
            return;
        };
        let state = job.progress.state;
        if state.finished() {
            return;
        }
        let (result, action) = if !pause {
            (storage.fileop_cancel(op), "cancel")
        } else if state == OpState::Paused {
            (storage.fileop_resume(op), "resume")
        } else {
            (storage.fileop_pause(op), "pause")
        };
        match result {
            Ok(p) => {
                door_rt::diagnostic(format_args!("{}", fo::diag_control(op, action)));
                if let Some(job) = self.ops.jobs.iter_mut().find(|j| j.progress.op == op) {
                    job.progress = p;
                    job.graph.interrupt();
                }
                if action == "cancel" && self.ops.conflict.is_some_and(|c| c.op == op) {
                    self.ops.conflict = None;
                }
            }
            Err(e) => {
                door_rt::diagnostic(format_args!(
                    "DOOR_P14 FILEOP control_failed op={op} action={action} error={e:?}\n"
                ));
                self.message = format!("Couldn't {action} the operation: {e:?}");
            }
        }
    }
    /// A conflict dialog choice.
    pub(super) fn conflict_choice(&mut self, choice: ConflictChoice, storage: &mut Storage) {
        let Some(mut d) = self.ops.conflict else {
            return;
        };
        match d.choose(choice) {
            Resolution::PerFile => {
                if !self.ops.per_file.contains(&d.op) {
                    self.ops.per_file.push(d.op);
                }
                self.ops.conflict = Some(d);
            }
            Resolution::Resolve(decision, all) => {
                self.ops.conflict = None;
                match storage.fileop_resolve(d.op, decision, all) {
                    Ok(()) => {
                        door_rt::diagnostic(format_args!(
                            "{}",
                            fo::diag_resolve(d.op, decision, all)
                        ));
                        if all {
                            self.ops.per_file.retain(|o| *o != d.op);
                        }
                        if let Some(job) = self.ops.jobs.iter_mut().find(|j| j.progress.op == d.op)
                        {
                            job.progress.state = OpState::Running;
                            job.progress.conflict = None;
                            job.conflict_logged = false;
                        }
                    }
                    Err(e) => {
                        self.message = format!("Couldn't apply the choice: {e:?}");
                        self.control_job(d.op, false, storage);
                    }
                }
            }
            Resolution::Cancel => {
                self.ops.conflict = None;
                self.control_job(d.op, false, storage);
            }
        }
    }

    // ---- Deletion ----

    /// Delete key (`permanent` false): the selection goes to the Recycle Bin
    /// at once, as Windows 11 does by default; Shift+Delete asks first and
    /// deletes permanently. In the Recycle Bin both delete permanently.
    pub(super) fn delete_selection(&mut self, permanent: bool, storage: &mut Storage) {
        if self.recycle_view() {
            self.ask_purge();
            return;
        }
        let items = self.removable();
        if items.is_empty() {
            if let Some(reason) = self.delete_block() {
                self.message = reason.into();
            }
            return;
        }
        if permanent {
            self.ask_permanent(items);
            return;
        }
        let (mut recycled, mut too_big, mut error) = (0, Vec::new(), None);
        for (path, directory) in &items {
            match storage.recycle(path) {
                Ok(RecycleOutcome::Recycled(id)) => {
                    recycled += 1;
                    door_rt::diagnostic(format_args!("DOOR_P14 RECYCLE id={id} path={path}\n"));
                }
                Ok(RecycleOutcome::TooBig) => too_big.push((path.clone(), *directory)),
                Err(e) => error = Some((String::from(fo::file_name(path)), e)),
            }
        }
        if recycled > 0 {
            self.ops.bin_items = None;
            let _ = self.relist(storage);
        }
        if let Some((name, e)) = error {
            self.message = match e {
                Error::Service(Code::SharingViolation) => {
                    format!("{name} is open in another app. Close it and try again.")
                }
                Error::Service(Code::Denied) => {
                    format!("You need permission to delete {name}.")
                }
                _ => format!("Couldn't delete {name}: {e:?}"),
            };
        }
        if let Some((_, directory)) = too_big.first().cloned() {
            let (title, question) = fo::too_big_question(directory);
            let body = if too_big.len() == 1 {
                format!("{}\n{question}", fo::file_name(&too_big[0].0))
            } else {
                format!(
                    "{} items are too big to recycle. Do you want to permanently delete them?",
                    too_big.len()
                )
            };
            self.prompt(
                PromptKind::TooBig,
                title,
                body,
                too_big.into_iter().map(|(p, _)| p).collect(),
                Vec::new(),
            );
        }
    }
    /// Shift+Delete: Windows' permanent-delete confirmation.
    fn ask_permanent(&mut self, items: Vec<(String, bool)>) {
        // The confirmation reuses the removal dialog; the engine re-checks
        // every item it deletes against the objects it actually opens.
        let first = Observed {
            parent: 1,
            object: 1,
            version: 1,
        };
        if !self.removal.begin(first) {
            self.message = "Refresh the folder with F5 before deleting.".into();
            return;
        }
        self.ops.pending_folder = items.len() == 1 && items[0].1;
        self.removal_name = if items.len() == 1 {
            fo::file_name(&items[0].0).into()
        } else {
            String::new()
        };
        self.removal_error.clear();
        self.ops.pending_delete = items.into_iter().map(|(p, _)| p).collect();
        self.dismiss_menu();
        self.rename = None;
        self.editor = 0;
        door_rt::diagnostic(format_args!(
            "DOOR_P12 FILES REMOVE_PROMPT count={}\n",
            self.ops.pending_delete.len()
        ));
    }
    /// Title, body and buttons of the permanent-delete confirmation.
    pub(super) fn removal_texts(&self) -> (String, String, &'static [&'static str]) {
        if !self.removal_error.is_empty() {
            return (
                "Couldn't delete".into(),
                format!("{}\n{}", self.removal_name, self.removal_error),
                &["Close"],
            );
        }
        let count = self.ops.pending_delete.len().max(1);
        let (title, question) = fo::delete_question(count, self.ops.pending_folder);
        let body = if count == 1 {
            format!("{question}\n{}", self.removal_name)
        } else {
            question
        };
        (title.into(), body, &["Yes", "No"])
    }
    /// The permanent-delete confirmation was answered.
    pub(super) fn confirm_permanent(&mut self, choice: usize, storage: &mut Storage) {
        let yes = self.removal.choose(choice).is_some();
        self.removal.cancel();
        self.removal_error.clear();
        let paths = core::mem::take(&mut self.ops.pending_delete);
        if yes {
            self.delete_permanently(storage, &paths);
        } else {
            door_rt::diagnostic(format_args!(
                "DOOR_P12 FILES REMOVE_CANCEL refresh={}\n",
                self.removal.refresh_required()
            ));
        }
    }
    /// Shift+Delete (or "too big to recycle"): delete operations that purge
    /// the items, logged like the Recycle Bin's own purges.
    fn delete_permanently(&mut self, storage: &mut Storage, paths: &[String]) {
        for op in self.begin_plans(storage, fo::plans(paths, "", FileOpMode::Delete)) {
            door_rt::diagnostic(format_args!("DOOR_P14 PURGE op={op}\n"));
        }
        self.run_quick(storage);
    }
    fn prompt(
        &mut self,
        kind: PromptKind,
        title: &str,
        body: String,
        paths: Vec<String>,
        ids: Vec<u64>,
    ) {
        let mut dialog = Dialog::default();
        if kind == PromptKind::Notice {
            dialog.show(1);
        } else {
            // Destructive questions start on No.
            dialog.show_focused(2, 1);
        }
        self.dismiss_menu();
        self.ops.prompt = Some(Prompt {
            dialog,
            kind,
            title: title.into(),
            body,
            paths,
            ids,
        });
    }
    pub(super) fn prompt_layout(&self) -> PromptLayout {
        PromptLayout::new(
            self.layout.width,
            self.layout.height,
            self.ops.prompt.as_ref().map_or(1, |p| p.dialog.count()),
        )
    }
    /// A prompt button: 0 is Yes (or OK).
    pub(super) fn prompt_choose(&mut self, choice: usize, storage: &mut Storage) {
        let Some(prompt) = self.ops.prompt.take() else {
            return;
        };
        if choice != 0 {
            return;
        }
        match prompt.kind {
            PromptKind::TooBig => self.delete_permanently(storage, &prompt.paths),
            PromptKind::EmptyBin => match storage.recycle_empty() {
                Ok(0) => (),
                Ok(op) => {
                    door_rt::diagnostic(format_args!("DOOR_P14 PURGE op={op}\n"));
                    let n = self.ops.bin_rows.len().max(1) as u32;
                    let summary = fo::summary(FileOpMode::Delete, n, "Recycle Bin", "");
                    self.push_job(op, FileOpMode::Delete, summary, n as usize);
                }
                Err(e) => self.message = Self::operation_error(FileOpMode::Delete, &e),
            },
            PromptKind::Purge => {
                self.ops.purge_queue = prompt.ids;
                self.ops.purge_queue.reverse();
                self.next_purge(storage);
            }
            PromptKind::Notice => (),
        }
    }
    /// Purges queued bin entries one operation at a time (the engine keeps
    /// at most 16 operations per user).
    fn next_purge(&mut self, storage: &mut Storage) -> bool {
        if self.ops.purge_queue.is_empty()
            || self
                .ops
                .jobs
                .iter()
                .any(|j| j.summary.ends_with("from Recycle Bin"))
        {
            return false;
        }
        let Some(id) = self.ops.purge_queue.pop() else {
            return false;
        };
        match storage.recycle_purge(id) {
            Ok(op) => {
                door_rt::diagnostic(format_args!("DOOR_P14 PURGE op={op}\n"));
                let summary = fo::summary(FileOpMode::Delete, 1, "Recycle Bin", "");
                self.push_job(op, FileOpMode::Delete, summary, 1);
            }
            Err(e) => self.message = Self::operation_error(FileOpMode::Delete, &e),
        }
        true
    }

    // ---- Recycle Bin view ----

    /// The bin's entries as list rows (identities kept apart from files).
    pub(super) fn recycle_entries(
        &mut self,
        storage: &mut Storage,
    ) -> Result<Vec<DirEntry>, Error> {
        let mut rows = Vec::new();
        let mut after = 0;
        while rows.len() < 1024 {
            let Some(entry) = storage.recycle_list(after)? else {
                break;
            };
            after = entry.id;
            rows.push(RecycleRow::new(&entry));
        }
        let entries = rows
            .iter()
            .map(|row| {
                let mut name = [0u16; door_service_protocol::files::NAME_UNITS];
                let mut name_len = 0;
                for (slot, unit) in name.iter_mut().zip(row.name.encode_utf16()) {
                    *slot = unit;
                    name_len += 1;
                }
                DirEntry {
                    cookie: row.id,
                    name,
                    name_len,
                    info: FileInfo {
                        attributes: 0,
                        bytes: row.bytes,
                        object: fo::recycle_object(row.id),
                        version: 1,
                        directory: row.directory,
                    },
                    times: FileTimes {
                        created: 0,
                        modified: row.deleted,
                        accessed: 0,
                    },
                }
            })
            .collect();
        self.ops.bin_items = Some(rows.len() as u32);
        self.ops.bin_rows = rows;
        Ok(entries)
    }
    /// The listing of `path`, including the Recycle Bin.
    pub(super) fn list_location(
        &mut self,
        storage: &mut Storage,
        path: &str,
    ) -> Result<Vec<DirEntry>, Error> {
        if path == RECYCLE_BIN {
            self.recycle_entries(storage)
        } else {
            let mut entries = Self::read_entries(storage, path)?;
            entries.retain(Self::visible);
            Ok(entries)
        }
    }
    /// Opens the Recycle Bin (Details, as Windows shows it).
    pub(super) fn go_recycle_bin(&mut self, storage: &mut Storage) -> Result<(), Error> {
        let entries = self.recycle_entries(storage)?;
        self.navigate_entries(RECYCLE_BIN.into(), entries);
        if self.prefs.get(RECYCLE_BIN).is_none() {
            self.view = ViewMode::Details;
        }
        Ok(())
    }
    fn bin_row(&self, entry: &DirEntry) -> Option<&RecycleRow> {
        let id = fo::recycle_id(entry.info.object)?;
        self.ops.bin_rows.iter().find(|r| r.id == id)
    }
    fn selected_bin_ids(&self) -> Vec<u64> {
        self.shown()
            .filter(|(i, _)| self.is_selected(*i))
            .filter_map(|(_, e)| fo::recycle_id(e.info.object))
            .collect()
    }
    /// "Restore the selected items" / "Restore all items".
    pub(super) fn restore(&mut self, all: bool, storage: &mut Storage) {
        let ids: Vec<u64> = if all {
            self.ops.bin_rows.iter().map(|r| r.id).collect()
        } else {
            self.selected_bin_ids()
        };
        let mut failed = None;
        for id in ids {
            let ok = storage.recycle_restore(id);
            door_rt::diagnostic(format_args!(
                "DOOR_P14 RESTORE id={id} ok={}\n",
                u8::from(ok.is_ok())
            ));
            if let Err(e) = ok
                && failed.is_none()
            {
                failed = Some((id, e));
            }
        }
        self.ops.bin_items = None;
        let _ = self.relist(storage);
        if let Some((id, e)) = failed {
            let row = self.ops.bin_rows.iter().find(|r| r.id == id);
            let (name, location) = row.map_or((String::new(), String::new()), |r| {
                (r.name.clone(), r.location.clone())
            });
            let why = match e {
                Error::Service(Code::Exists) => {
                    format!("An item named \"{name}\" already exists in {location}.")
                }
                Error::Service(Code::Denied) => "You need permission to restore it there.".into(),
                _ => format!("The item couldn't be restored: {e:?}"),
            };
            self.prompt(
                PromptKind::Notice,
                "Couldn't restore",
                why,
                Vec::new(),
                Vec::new(),
            );
        }
    }
    /// Delete in the Recycle Bin: permanent, confirmed.
    fn ask_purge(&mut self) {
        let ids = self.selected_bin_ids();
        if ids.is_empty() {
            return;
        }
        let folder = ids.len() == 1
            && self
                .ops
                .bin_rows
                .iter()
                .any(|r| r.id == ids[0] && r.directory);
        let (title, question) = fo::delete_question(ids.len(), folder);
        let body = match self
            .ops
            .bin_rows
            .iter()
            .find(|r| ids.len() == 1 && r.id == ids[0])
        {
            Some(row) => format!("{question}\n{}", row.name),
            None => question,
        };
        self.prompt(PromptKind::Purge, title, body, Vec::new(), ids);
    }
    /// "Empty Recycle Bin": confirmed with the item count.
    pub(super) fn ask_empty_bin(&mut self, storage: &mut Storage) {
        if !self.recycle_view() && self.recycle_entries(storage).is_err() {
            return;
        }
        let n = self.ops.bin_rows.len();
        if n == 0 {
            self.message = context_menu::BIN_EMPTY.into();
            return;
        }
        let (title, question) = fo::empty_question(n);
        self.prompt(
            PromptKind::EmptyBin,
            title,
            question,
            Vec::new(),
            Vec::new(),
        );
    }
    /// Properties of a bin entry (or of the bin): origin, size, deletion date.
    pub(super) fn recycle_properties(&mut self, item: bool, storage: &mut Storage) {
        if !self.recycle_view() && self.recycle_entries(storage).is_err() {
            return;
        }
        let row = self
            .selected
            .filter(|_| item)
            .and_then(|i| self.entries.get(i))
            .and_then(|e| self.bin_row(e))
            .cloned();
        let (title, body) = match row {
            Some(r) => (
                r.name.clone(),
                format!(
                    "Origin: {}\nType: {}\nSize: {}\nDeleted: {}",
                    r.location,
                    r.kind,
                    previewing::size(r.bytes),
                    self.date_text(r.deleted)
                ),
            ),
            None => {
                let bytes: u64 = self.ops.bin_rows.iter().map(|r| r.bytes).sum();
                (
                    "Recycle Bin Properties".into(),
                    format!(
                        "Items: {}\nSize: {}",
                        self.ops.bin_rows.len(),
                        previewing::size(bytes)
                    ),
                )
            }
        };
        self.prompt(PromptKind::Notice, &title, body, Vec::new(), Vec::new());
    }

    // ---- Live refresh ----

    /// Re-lists the folder shown, keeping the selection by object identity.
    pub(super) fn relist(&mut self, storage: &mut Storage) -> Result<(), Error> {
        if self.home_view || self.overview || !self.result_paths.is_empty() {
            return Ok(());
        }
        let cwd = self.cwd.clone();
        let mut entries = self.list_location(storage, &cwd)?;
        self.hide_tag_metadata(&cwd, &mut entries);
        let focus = self
            .selected
            .and_then(|i| self.entries.get(i))
            .map(|e| e.info.object);
        let marked = core::mem::take(&mut self.marked);
        let objects: Vec<u64> = entries.iter().map(|e| e.info.object).collect();
        let (selected, marked) = fo::keep_selection(&objects, focus, &marked);
        self.entries = entries;
        self.selected = selected;
        self.marked = marked;
        self.sort();
        if focus.is_none() {
            self.selected = None;
        }
        self.offset = self.offset.min(self.max_offset());
        self.thumbnail_scanned = false;
        self.folders_scanned = false;
        self.prune_folders();
        Ok(())
    }
    /// Follows the folder shown with one watch; re-lists on change and
    /// leaves a folder that was removed.
    fn watch_step(&mut self, storage: &mut Storage, now: u64) -> bool {
        let want = if self.recycle_view() {
            Some(fo::bin_folder(&self.home))
        } else if self.home_view || self.overview || !self.result_paths.is_empty() {
            None
        } else {
            Some(self.cwd.clone())
        };
        match self.ops.watch.action(want.as_deref()) {
            WatchAction::Keep => (),
            WatchAction::Unsubscribe(token) => {
                let _ = storage.unwatch(token);
                self.ops.watch.token = None;
            }
            action => {
                if let WatchAction::Replace(token) = action {
                    let _ = storage.unwatch(token);
                    self.ops.watch.token = None;
                }
                let folder = want.clone().unwrap_or_default();
                match storage.watch(&folder) {
                    Ok(token) => {
                        self.ops.watch.token = Some(token);
                        self.ops.watch.failed = None;
                    }
                    Err(_) => self.ops.watch.failed = Some(folder.clone()),
                }
                self.ops.watch.folder = folder;
            }
        }
        let Some(token) = self.ops.watch.token else {
            return false;
        };
        if !self.ops.watch.due(now) {
            return false;
        }
        match storage.watch_poll(token) {
            Ok(state) if state.gone && !self.recycle_view() => {
                let _ = storage.unwatch(token);
                self.ops.watch.token = None;
                self.leave_removed_folder(storage);
                true
            }
            Ok(state) if state.changed || state.overflow || state.gone => {
                let folder = self.ops.watch.folder.clone();
                if self.relist(storage).is_ok() {
                    door_rt::diagnostic(format_args!("DOOR_P14 WATCH refresh path={folder}\n"));
                }
                true
            }
            Ok(_) => false,
            Err(_) => {
                self.ops.watch.token = None;
                self.ops.watch.failed = Some(self.ops.watch.folder.clone());
                false
            }
        }
    }
    /// The folder shown was removed: open the nearest existing parent.
    fn leave_removed_folder(&mut self, storage: &mut Storage) {
        let windows = explorer_path::display(&self.cwd).starts_with("C:");
        let mut path = self.cwd.clone();
        for _ in 0..16 {
            let parent = explorer_path::parent(&path, windows);
            if parent == path {
                break;
            }
            path = parent;
            if self.navigate(storage, path.clone()).is_ok() {
                self.message = "The folder was removed.".into();
                return;
            }
        }
        let _ = self.go_home(storage);
    }
    /// The storage connection was replaced: watch tokens died with it.
    pub(super) fn operations_disconnected(&mut self) {
        self.ops.watch.connection_lost();
    }
    pub(crate) fn release_watch(&mut self, storage: &mut Storage) {
        if let Some(token) = self.ops.watch.token.take() {
            let _ = storage.unwatch(token);
        }
    }

    // ---- Input ----

    /// A dialog of this module traps input.
    pub(super) fn operations_modal(&self) -> bool {
        self.ops.prompt.is_some() || self.ops.conflict.is_some()
    }
    fn conflict_layout(&self) -> Option<(ConflictDialog, ConflictLayout)> {
        let d = self.ops.conflict?;
        Some((
            d,
            ConflictLayout::new(self.layout.width, self.layout.height, d.stage),
        ))
    }
    /// Hit-test ids of the open prompt or conflict dialog.
    pub(super) fn operations_modal_target(&self, x: i32, y: i32) -> Option<u32> {
        if let Some(p) = &self.ops.prompt {
            return p
                .dialog
                .target(&self.prompt_layout(), x, y)
                .map(|i| PROMPT + i as u32);
        }
        let (d, l) = self.conflict_layout()?;
        match l.hit(d.stage, x, y)? {
            None => Some(CONFLICT_CHECK),
            Some(c) => CHOICES
                .iter()
                .position(|k| *k == c)
                .map(|i| CONFLICT + i as u32),
        }
    }
    pub(super) fn operations_pointer(&mut self, x: i32, y: i32) {
        let layout = self.prompt_layout();
        if let Some(p) = self.ops.prompt.as_mut() {
            let target = p.dialog.target(&layout, x, y);
            p.dialog.pointer(target);
        }
    }
    pub(super) fn operations_click(&mut self, x: i32, y: i32, storage: &mut Storage) {
        match self.operations_modal_target(x, y) {
            Some(id) if (PROMPT..PROMPT + 3).contains(&id) => {
                self.prompt_choose((id - PROMPT) as usize, storage)
            }
            Some(CONFLICT_CHECK) => {
                if let Some(d) = self.ops.conflict.as_mut() {
                    d.apply_rest = !d.apply_rest;
                }
            }
            Some(id) if (CONFLICT..CONFLICT + 7).contains(&id) => {
                self.conflict_choice(CHOICES[(id - CONFLICT) as usize], storage)
            }
            _ => (),
        }
    }
    pub(super) fn operations_key(&mut self, code: u32, storage: &mut Storage) {
        let scan = code & 511;
        let shift = (code >> 9) & door_abi::graphics::SHIFT != 0;
        let alt = (code >> 9) & door_abi::graphics::ALT != 0;
        if let Some(p) = self.ops.prompt.as_mut() {
            if let Some(choice) = p.dialog.key(scan, shift) {
                self.prompt_choose(choice, storage);
            }
            return;
        }
        let Some(d) = self.ops.conflict.as_mut() else {
            return;
        };
        // Access keys: Alt+R replace, Alt+S skip, Alt+K keep both, Alt+D
        // "Do this for all" (the overview applies Replace/Skip to all).
        let choice = match (alt, scan) {
            (true, 32) => {
                d.apply_rest = !d.apply_rest;
                return;
            }
            (true, 19) if d.stage == ConflictStage::Overview => Some(ConflictChoice::ReplaceAll),
            (true, 31) if d.stage == ConflictStage::Overview => Some(ConflictChoice::SkipAll),
            (true, 19) => Some(ConflictChoice::Replace),
            (true, 31) => Some(ConflictChoice::Skip),
            (true, 37) => {
                if d.stage == ConflictStage::Overview {
                    d.stage = ConflictStage::PerFile;
                }
                Some(ConflictChoice::KeepBoth)
            }
            (true, _) => None,
            (false, _) => d.key(scan, shift),
        };
        if let Some(choice) = choice {
            self.conflict_choice(choice, storage);
        }
    }
    /// Keys for the visible progress dialog: Esc cancels the active
    /// operation, Enter resumes a paused one (also one interrupted by a
    /// restart). Returns whether the key was used.
    pub(super) fn progress_key(&mut self, scan: u32, storage: &mut Storage) -> bool {
        if self.ops.minimized || self.operations_modal() {
            return false;
        }
        let shown: Vec<(u64, OpState)> = self
            .shown_jobs()
            .map(|j| (j.progress.op, j.progress.state))
            .collect();
        match scan {
            1 => match shown.iter().find(|(_, s)| !s.finished()) {
                Some((op, _)) => {
                    self.control_job(*op, false, storage);
                    true
                }
                None => false,
            },
            28 | 0x11c => match shown.iter().find(|(_, s)| *s == OpState::Paused) {
                Some((op, _)) => {
                    self.control_job(*op, true, storage);
                    true
                }
                None => false,
            },
            _ => false,
        }
    }
    fn shown_jobs(&self) -> impl Iterator<Item = &Job> {
        self.ops.jobs.iter().filter(|j| j.shown)
    }
    fn progress_layout(&self) -> ProgressLayout {
        ProgressLayout::new(
            self.layout.width,
            self.layout.height,
            self.dialog_jobs().len(),
            self.ops.details,
        )
    }
    /// Hit-test ids of the progress dialog or its minimised pill.
    pub(super) fn progress_target(&self, x: i32, y: i32) -> Option<u32> {
        if self.shown_jobs().next().is_none() || self.operations_modal() {
            return None;
        }
        if self.ops.minimized {
            return fo::pill(self.layout.width, self.layout.height)
                .contains(x, y)
                .then_some(PROGRESS + 3);
        }
        Some(match self.progress_layout().hit(x, y)? {
            ProgressHit::Close => PROGRESS,
            ProgressHit::Toggle => PROGRESS + 1,
            ProgressHit::Panel => PROGRESS + 2,
            ProgressHit::Pause(i) => PROGRESS + 10 + i as u32,
            ProgressHit::Cancel(i) => PROGRESS + 20 + i as u32,
        })
    }
    pub(super) fn progress_click(&mut self, id: u32, storage: &mut Storage) {
        let op = |s: &Self, i: u32| s.shown_jobs().nth(i as usize).map(|j| j.progress.op);
        match id - PROGRESS {
            0 => {
                // Closing the dialog cancels its operations.
                let ops: Vec<u64> = self
                    .shown_jobs()
                    .filter(|j| !j.progress.state.finished())
                    .map(|j| j.progress.op)
                    .collect();
                for op in ops {
                    self.control_job(op, false, storage);
                }
            }
            1 => self.ops.details = !self.ops.details,
            3 => self.ops.minimized = false,
            i @ 10..=19 => {
                if let Some(op) = op(self, i - 10) {
                    self.control_job(op, true, storage);
                }
            }
            i @ 20..=29 => {
                if let Some(op) = op(self, i - 20) {
                    self.control_job(op, false, storage);
                }
            }
            _ => (),
        }
    }

    // ---- Painting ----

    /// Cut items stay drawn faded until they are pasted.
    pub(super) fn draw_cut_items(&self, v: &mut ui::View<'_>) {
        if !self.ops.clipboard.cut
            || self.home_view
            || self.recycle_view()
            || (self.overview && self.search.is_empty())
        {
            return;
        }
        let layout = self.content_layout();
        let mode = self.view_mode();
        let wash = if v.palette.dark {
            [15, 15, 15, 150]
        } else {
            [150, 150, 150, 150]
        };
        for (slot, (_, entry)) in self
            .shown()
            .skip(self.offset)
            .take(layout.capacity(mode))
            .enumerate()
        {
            let Some(cell) = self.item_cell(slot) else {
                break;
            };
            if self
                .path_for(entry)
                .is_some_and(|p| self.ops.clipboard.is_cut(&p))
            {
                let r = layout.icon_bounds(cell, mode);
                v.rounded_asset(r.x, r.y, r.width, r.height, 0, wash);
            }
        }
    }
    /// The Recycle Bin's Details columns: Name, Original Location, Date
    /// Deleted, Size, Item type.
    pub(super) fn draw_recycle(&self, v: &mut ui::View<'_>) {
        if !self.recycle_view() || self.view_mode() != ViewMode::Details {
            return;
        }
        let layout = self.content_layout();
        let columns = self.detail_columns(layout);
        let header = columns[0].1;
        let left = header.x;
        let right = columns[3].1.x + columns[3].1.width as i32;
        let bottom = self.layout.height as i32 - xl::status_height() as i32;
        let clear_right = (layout.width as i32 - 8).max(left);
        v.rect(
            self.layout.sidebar,
            header.y,
            (clear_right - self.layout.sidebar).max(0) as u32,
            (bottom - header.y).max(0) as u32,
            [255; 4],
        );
        let cols = fo::recycle_columns(left, (right - left).max(0) as u32);
        for (i, (x, w)) in cols.iter().enumerate() {
            let text_x = if i == 0 { x + 30 } else { x + 8 };
            label(
                v,
                text_x,
                header.y,
                w.saturating_sub(if i == 0 { 38 } else { 16 }),
                header.height,
                fo::RECYCLE_COLUMNS[i],
                MUTED,
            );
            v.rect(
                x + *w as i32 - 1,
                header.y + 6,
                1,
                header.height.saturating_sub(12),
                [229, 229, 229, 255],
            );
        }
        let mode = ViewMode::Details;
        let mut any = false;
        for (slot, (index, entry)) in self
            .shown()
            .skip(self.offset)
            .take(layout.capacity(mode))
            .enumerate()
        {
            let Some(cell) = self.item_cell(slot) else {
                break;
            };
            any = true;
            self.item_fill(v, cell, self.is_selected(index));
            let icon_box = layout.icon_bounds(cell, mode);
            let name = Self::name(entry);
            let icon = if entry.info.directory {
                Icon::Folder
            } else {
                Icon::for_file_name(&name)
            };
            v.sized_icon(icon_box.x, icon_box.y, icon_box.width, icon);
            let Some(row) = self.bin_row(entry) else {
                continue;
            };
            let text_x = layout.label_bounds(cell, mode).x;
            let cells = [
                (
                    text_x,
                    cols[0].0 + cols[0].1 as i32 - 8,
                    row.name.clone(),
                    TEXT,
                ),
                (
                    cols[1].0 + 8,
                    cols[1].0 + cols[1].1 as i32 - 8,
                    row.location.clone(),
                    MUTED,
                ),
                (
                    cols[2].0 + 8,
                    cols[2].0 + cols[2].1 as i32 - 8,
                    self.date_text(row.deleted),
                    MUTED,
                ),
                (0, 0, row.size(), MUTED),
                (
                    cols[4].0 + 8,
                    cols[4].0 + cols[4].1 as i32 - 8,
                    row.kind.into(),
                    MUTED,
                ),
            ];
            for (i, (x, end, text, color)) in cells.into_iter().enumerate() {
                let text = ui::display_ascii(&text);
                if i == 3 {
                    let w = v.measure(&text).min(cols[3].1.saturating_sub(16));
                    let x = cols[3].0 + cols[3].1 as i32 - 12 - w as i32;
                    label(v, x, cell.y, w, cell.height, &text, color);
                } else {
                    label(
                        v,
                        x,
                        cell.y,
                        (end - x).max(0) as u32,
                        cell.height,
                        &text,
                        color,
                    );
                }
            }
        }
        if !any {
            label(
                v,
                self.layout.sidebar + 28,
                layout.view_top(mode) + 24,
                layout.width.saturating_sub(self.layout.sidebar as u32 + 56),
                (32 + xl::grow()) as u32,
                "This folder is empty.",
                MUTED,
            );
        }
    }
    fn panel(v: &mut ui::View<'_>, r: Bounds) {
        v.elevation(r.x, r.y + 2, r.width, r.height, 60);
        v.rounded(r.x, r.y, r.width, r.height, 8, [200, 200, 200, 255]);
        v.rounded(
            r.x + 1,
            r.y + 1,
            r.width.saturating_sub(2),
            r.height.saturating_sub(2),
            7,
            [255; 4],
        );
    }
    fn hover_plate(v: &mut ui::View<'_>, r: Bounds) {
        if v.hover.is_some_and(|(x, y)| r.contains(x, y)) {
            let down = v.pressed.is_some_and(|(x, y)| r.contains(x, y));
            v.rounded(
                r.x,
                r.y,
                r.width,
                r.height,
                4,
                if down { SELECTED } else { HOVER },
            );
        }
    }
    /// The progress dialog (or its pill), above the content.
    pub(super) fn draw_operations(&self, v: &mut ui::View<'_>) {
        let jobs = self.dialog_jobs();
        if jobs.is_empty() {
            return;
        }
        let progress: Vec<OpProgress> = jobs.iter().map(|j| j.progress).collect();
        if self.ops.minimized {
            let r = fo::pill(self.layout.width, self.layout.height);
            Self::panel(v, r);
            Self::hover_plate(
                v,
                Bounds {
                    x: r.x + 2,
                    y: r.y + 2,
                    width: r.width.saturating_sub(4),
                    height: r.height.saturating_sub(4),
                },
            );
            v.compact_icon(r.x + 10, r.y + 10, Icon::Copy, TEXT);
            label(
                v,
                r.x + 34,
                r.y,
                r.width.saturating_sub(44),
                r.height - 4,
                &fo::caption(&progress),
                TEXT,
            );
            let pct = fo::percent(&progress[0]);
            let w = r.width.saturating_sub(20);
            v.rect(r.x + 10, r.y + r.height as i32 - 7, w, 3, BAR_TRACK);
            v.rect(r.x + 10, r.y + r.height as i32 - 7, w * pct / 100, 3, GREEN);
            return;
        }
        let l = self.progress_layout();
        let p = l.panel;
        let Some((alpha, scale)) = self.dialog_motion() else {
            self.paint_dialog(v, &l, &jobs);
            return;
        };
        // Opening or closing: paint the window, lift it off, then replay it
        // faded and scaled about its centre over what was under it.
        const SHADOW: i32 = 24;
        let Some(under) = v.snapshot(
            p.x - SHADOW,
            p.y - SHADOW,
            p.width + 2 * SHADOW as u32,
            p.height + 2 * SHADOW as u32,
        ) else {
            return;
        };
        self.paint_dialog(v, &l, &jobs);
        let window = v.snapshot(p.x, p.y, p.width, p.height);
        v.restore(&under);
        v.elevation(
            p.x,
            p.y + 2,
            p.width,
            p.height,
            (60 * u32::from(alpha) / 255) as u8,
        );
        if let Some(window) = window {
            v.composite(&window, alpha, scale, 8);
        }
    }
    /// Windows 11's copy dialog: a window with a title bar ("45% complete",
    /// a disabled maximize and close), one section per operation (summary
    /// with its folder links, headline, pause and cancel, the green bar or
    /// the speed graph) and the More details footer.
    fn paint_dialog(&self, v: &mut ui::View<'_>, l: &ProgressLayout, jobs: &[&Job]) {
        let progress: Vec<OpProgress> = jobs.iter().map(|j| j.progress).collect();
        let p = l.panel;
        let dark = v.palette.dark;
        let title = if dark { [32, 32, 32, 255] } else { [255; 4] };
        let border = if dark {
            [59, 59, 59, 255]
        } else {
            [200, 200, 200, 255]
        };
        v.elevation(p.x, p.y + 2, p.width, p.height, 60);
        v.rounded(p.x, p.y, p.width, p.height, 8, border);
        v.rounded(p.x + 1, p.y + 1, p.width - 2, p.height - 2, 7, [255; 4]);
        v.rounded(p.x + 1, p.y + 1, p.width - 2, fo::CAPTION + 7, 7, title);
        v.rect(p.x + 1, p.y + fo::CAPTION as i32, p.width - 2, 8, [255; 4]);
        // Title bar: the clock icon, the caption, maximize (disabled), close.
        Self::clock_icon(v, p.x + 8, p.y + 8);
        v.text_clip_centered(
            l.caption.x,
            (l.caption.y, l.caption.height),
            l.caption.width,
            &fo::caption(&progress),
            TextStyle::Regular12,
            TEXT,
        );
        let m = l.maximize;
        let (mx, my) = (m.x + m.width as i32 / 2 - 5, m.y + m.height as i32 / 2 - 5);
        let disabled = if dark {
            [89, 77, 72, 255]
        } else {
            [160, 160, 160, 255]
        };
        for (x, y, w, h) in [(0, 0, 10, 1), (0, 9, 10, 1), (0, 0, 1, 10), (9, 0, 1, 10)] {
            v.rect(mx + x, my + y, w, h, disabled);
        }
        let c = l.close;
        let hot = v.hover.is_some_and(|(x, y)| c.contains(x, y));
        if hot {
            // Square but for the window's rounded top-right corner.
            let (w, h) = (c.width - 1, c.height - 1);
            v.rounded(c.x, c.y + 1, w, h, 7, CLOSE_HOVER);
            v.rect(c.x, c.y + 1, 8, h, CLOSE_HOVER);
            v.rect(c.x, c.y + h as i32 - 7, w, 8, CLOSE_HOVER);
        }
        let (cx, cy) = (
            c.x as f32 + c.width as f32 / 2.0,
            c.y as f32 + c.height as f32 / 2.0,
        );
        let glyph = if hot { [255; 4] } else { TEXT };
        v.stroke((cx - 5.0, cy - 5.0), (cx + 5.0, cy + 5.0), 1.0, glyph);
        v.stroke((cx - 5.0, cy + 5.0), (cx + 5.0, cy - 5.0), 1.0, glyph);
        for (job, s) in jobs.iter().zip(&l.sections) {
            let state = job.progress;
            Self::summary_links(v, s.summary, &job.summary);
            v.text_clip_centered(
                s.headline.x,
                (s.headline.y, s.headline.height),
                s.headline.width,
                &fo::headline(&state),
                TextStyle::Regular16,
                TEXT,
            );
            for b in [s.pause, s.cancel] {
                Self::hover_plate(v, b);
            }
            let (px, py) = (
                s.pause.x + s.pause.width as i32 / 2,
                s.pause.y + s.pause.height as i32 / 2,
            );
            if state.state == OpState::Paused {
                // Resume: a play triangle.
                for i in 0..5 {
                    v.rect(px - 2 + i, py - 5 + i, 1, (10 - 2 * i) as u32, TEXT);
                }
            } else {
                v.rect(px - 4, py - 4, 3, 8, TEXT);
                v.rect(px + 1, py - 4, 3, 8, TEXT);
            }
            let (kx, ky) = (
                s.cancel.x as f32 + s.cancel.width as f32 / 2.0,
                s.cancel.y as f32 + s.cancel.height as f32 / 2.0,
            );
            v.stroke((kx - 3.5, ky - 3.5), (kx + 3.5, ky + 3.5), 1.7, TEXT);
            v.stroke((kx - 3.5, ky + 3.5), (kx + 3.5, ky - 3.5), 1.7, TEXT);
            let pct = fo::percent(&state);
            let fill = match state.state {
                OpState::Paused => PAUSED,
                OpState::Failed => FAILED,
                _ => GREEN,
            };
            let m = s.meter;
            if l.details {
                v.rect(m.x, m.y, m.width, m.height, [210, 230, 210, 255]);
                v.rect(
                    m.x + 1,
                    m.y + 1,
                    m.width.saturating_sub(2),
                    m.height.saturating_sub(2),
                    [255; 4],
                );
                let (gw, gh) = (m.width.saturating_sub(2), m.height.saturating_sub(2));
                for (c, h) in job.graph.columns(gw, gh).into_iter().enumerate() {
                    if h > 0 {
                        v.rect(
                            m.x + 1 + c as i32,
                            m.y + 1 + (gh - h) as i32,
                            1,
                            h,
                            [184, 226, 184, 255],
                        );
                    }
                }
                let line = job.graph.line(gh);
                let ly = m.y + 1 + (gh - line.min(gh)) as i32;
                v.rect(m.x + 1, ly.min(m.y + m.height as i32 - 2), gw, 1, fill);
                let speed = fo::speed_text(job.graph.speed);
                let w = v.measure_small(&speed).min(gw);
                v.text_clip_centered(
                    m.x + m.width as i32 - 6 - w as i32,
                    ((ly - 18).max(m.y + 2), 16),
                    w,
                    &speed,
                    TextStyle::Regular12,
                    TEXT,
                );
                let lines = [
                    format!("Name: {}", ui::display_ascii(state.name())),
                    format!(
                        "Time remaining: {}",
                        fo::time_remaining(
                            state.bytes_total.saturating_sub(state.bytes_done),
                            job.graph.speed
                        )
                    ),
                    format!("Items remaining: {}", fo::items_remaining(&state)),
                ];
                for (i, text) in lines.iter().enumerate() {
                    v.text_clip_centered(
                        s.info.x,
                        (s.info.y + i as i32 * fo::INFO_LINE, fo::INFO_LINE as u32),
                        s.info.width,
                        text,
                        TextStyle::Regular12,
                        TEXT,
                    );
                }
            } else {
                v.rect(m.x, m.y, m.width, m.height, BAR_BORDER);
                let (iw, ih) = (m.width.saturating_sub(2), m.height.saturating_sub(2));
                v.rect(m.x + 1, m.y + 1, iw, ih, BAR_TRACK);
                v.rect(m.x + 1, m.y + 1, iw * pct / 100, ih, fill);
            }
        }
        // Footer: a separator, then the chevron and "More details".
        v.rect(
            p.x + 1,
            p.y + p.height as i32 - fo::FOOTER as i32,
            p.width - 2,
            1,
            SEPARATOR,
        );
        Self::hover_plate(v, l.toggle);
        if l.details {
            v.tiny_icon_flipped(l.toggle.x + 11, l.toggle.y + 8, Icon::Chevron, TEXT);
        } else {
            v.tiny_icon(l.toggle.x + 11, l.toggle.y + 8, Icon::Chevron, TEXT);
        }
        v.text_clip_centered(
            l.toggle.x + 33,
            (l.toggle.y, l.toggle.height),
            l.toggle.width.saturating_sub(35),
            if l.details {
                "Fewer details"
            } else {
                "More details"
            },
            TextStyle::Regular12,
            TEXT,
        );
    }
    /// "Copying 2 items from src to dst" with the folder names as links.
    fn summary_links(v: &mut ui::View<'_>, r: Bounds, summary: &str) {
        let text = ui::display_ascii(summary);
        let mut parts: Vec<(&str, [u8; 4])> = Vec::new();
        match text.find(" from ") {
            Some(i) => {
                let (head, rest) = text.split_at(i + 6);
                parts.push((head, TEXT));
                match rest.rfind(" to ") {
                    Some(j) => {
                        parts.push((&rest[..j], LINK));
                        parts.push((" to ", TEXT));
                        parts.push((&rest[j + 4..], LINK));
                    }
                    None => parts.push((rest, LINK)),
                }
            }
            None => parts.push((&text, TEXT)),
        }
        let mut x = r.x;
        let right = r.x + r.width as i32;
        for (part, color) in parts {
            let w = v.measure_small(part);
            if x >= right {
                break;
            }
            let room = (right - x) as u32;
            v.text_clip_centered(
                x,
                (r.y, r.height),
                w.min(room),
                part,
                TextStyle::Regular12,
                color,
            );
            x += w as i32;
        }
    }
    /// The dialog's title bar icon: a clock face (16 px).
    fn clock_icon(v: &mut ui::View<'_>, x: i32, y: i32) {
        // Drawn in its own colours in either theme.
        let palette = v.palette;
        v.palette = door_workspace::palette::Palette::default();
        v.rounded(x, y, 16, 16, 8, [150, 150, 150, 255]);
        v.rounded(x + 1, y + 1, 14, 14, 7, [242, 242, 242, 255]);
        let (cx, cy) = (x as f32 + 8.0, y as f32 + 8.0);
        v.stroke((cx, cy - 4.3), (cx, cy + 0.4), 1.3, [96, 96, 96, 255]);
        v.stroke((cx - 0.4, cy), (cx + 3.4, cy), 1.3, [96, 96, 96, 255]);
        v.palette = palette;
    }
    /// The conflict dialog (modal), then any prompt over it.
    pub(super) fn draw_operation_dialogs(&self, v: &mut ui::View<'_>) {
        if let Some((d, l)) = self.conflict_layout()
            && let Some(job) = self.ops.jobs.iter().find(|j| j.progress.op == d.op)
        {
            v.rounded_asset(0, 0, v.width, v.height, 0, [0, 0, 0, 80]);
            let p = job.progress;
            let info = p.conflict.unwrap_or_default();
            Self::panel(v, l.panel);
            Self::hover_plate(v, l.close);
            v.tiny_icon(l.close.x + 14, l.close.y + 8, Icon::Close, TEXT);
            let title = match d.stage {
                ConflictStage::Overview => "Replace or Skip Files",
                ConflictStage::PerFile => "1 File Conflict",
            };
            label(
                v,
                l.panel.x + 16,
                l.panel.y,
                l.panel.width.saturating_sub(64),
                36,
                title,
                TEXT,
            );
            let name = ui::display_ascii(p.name());
            let lines = [
                ui::display_ascii(&job.summary),
                fo::conflict_question(&name, info.directory),
            ];
            for (i, text) in lines.iter().enumerate() {
                label(
                    v,
                    l.text.x,
                    l.text.y + i as i32 * 24,
                    l.text.width,
                    22,
                    text,
                    if i == 0 { MUTED } else { TEXT },
                );
            }
            let focus = |i: usize| d.focus == i;
            match d.stage {
                ConflictStage::Overview => {
                    for (i, (b, text)) in l.options.iter().zip(fo::overview_options(&p)).enumerate()
                    {
                        v.rounded(b.x, b.y, b.width, b.height, 4, [246, 246, 246, 255]);
                        Self::hover_plate(v, *b);
                        if focus(i) {
                            Self::focus_ring(v, *b);
                        }
                        let icon = match i {
                            0 => Icon::Check,
                            1 => Icon::Forward,
                            _ => Icon::Info,
                        };
                        v.compact_icon(b.x + 12, b.y + (b.height as i32 - 16) / 2, icon, TEXT);
                        label(
                            v,
                            b.x + 40,
                            b.y,
                            b.width.saturating_sub(48),
                            b.height,
                            text,
                            TEXT,
                        );
                    }
                }
                ConflictStage::PerFile => {
                    let facts = [
                        (
                            "Item to copy",
                            info.source_bytes,
                            self.date_text(info.source_modified),
                        ),
                        (
                            "Item in the destination",
                            info.dest_bytes,
                            self.date_text(info.dest_modified),
                        ),
                    ];
                    for (c, (heading, bytes, date)) in l.cards.iter().zip(facts) {
                        v.rounded(c.x, c.y, c.width, c.height, 4, [246, 246, 246, 255]);
                        let rows = [
                            String::from(heading),
                            name.clone(),
                            format!("Size: {}", previewing::size(bytes)),
                            format!("Date modified: {date}"),
                        ];
                        for (i, text) in rows.iter().enumerate() {
                            label(
                                v,
                                c.x + 10,
                                c.y + 4 + i as i32 * 21,
                                c.width.saturating_sub(20),
                                21,
                                text,
                                if i == 0 { TEXT } else { MUTED },
                            );
                        }
                    }
                    if let Some(rest) = fo::rest_label(&p) {
                        let b = l.checkbox;
                        let (bx, by) = (b.x, b.y + (b.height as i32 - 16) / 2);
                        v.rounded(bx, by, 16, 16, 3, [120, 120, 120, 255]);
                        v.rounded(bx + 1, by + 1, 14, 14, 2, [255; 4]);
                        if d.apply_rest {
                            v.rounded(bx + 1, by + 1, 14, 14, 2, [0, 95, 184, 255]);
                            v.compact_icon(bx, by, Icon::Check, [255; 4]);
                        }
                        label(
                            v,
                            bx + 26,
                            b.y,
                            b.width.saturating_sub(30),
                            b.height,
                            &rest,
                            TEXT,
                        );
                    }
                    for (i, (b, text)) in l.buttons.iter().zip(fo::PER_FILE_BUTTONS).enumerate() {
                        v.rounded(b.x, b.y, b.width, b.height, 4, [210, 210, 210, 255]);
                        v.rounded(
                            b.x + 1,
                            b.y + 1,
                            b.width.saturating_sub(2),
                            b.height.saturating_sub(2),
                            3,
                            [251, 251, 251, 255],
                        );
                        Self::hover_plate(v, *b);
                        if focus(i) {
                            Self::focus_ring(v, *b);
                        }
                        let w = v.measure(text).min(b.width.saturating_sub(8));
                        label(
                            v,
                            b.x + (b.width as i32 - w as i32) / 2,
                            b.y,
                            w,
                            b.height,
                            text,
                            TEXT,
                        );
                    }
                }
            }
        }
        if let Some(p) = &self.ops.prompt {
            v.dialog(&p.dialog, &p.title, &p.body, p.labels());
        }
    }
    fn focus_ring(v: &mut ui::View<'_>, r: Bounds) {
        let c = if v.palette.dark {
            [255; 4]
        } else {
            [0, 0, 0, 255]
        };
        v.rect(r.x - 2, r.y - 2, r.width + 4, 2, c);
        v.rect(r.x - 2, r.y + r.height as i32, r.width + 4, 2, c);
        v.rect(r.x - 2, r.y, 2, r.height, c);
        v.rect(r.x + r.width as i32, r.y, 2, r.height, c);
    }
    /// door.accessibility: the progress, conflict and prompt dialogs.
    pub(super) fn operations_semantics(&self, s: &mut door_workspace::semantics::Semantics) {
        use door_workspace::semantics::{Role, States};
        let b = |r: Bounds| (r.x, r.y, r.width, r.height);
        if let Some(p) = &self.ops.prompt {
            door_workspace::semantics::dialog(
                s,
                &self.prompt_layout(),
                &p.dialog,
                &p.title,
                &p.body,
                p.labels(),
                PROMPT,
            );
            return;
        }
        if let Some((d, l)) = self.conflict_layout() {
            s.begin(Role::Dialog, "Replace or Skip Files", b(l.panel))
                .states(States::MODAL);
            let progress = self
                .ops
                .jobs
                .iter()
                .find(|j| j.progress.op == d.op)
                .map(|j| j.progress);
            let name = progress.as_ref().map_or("", |p| p.name());
            s.text(Role::Text, &fo::conflict_question(name, false), b(l.text));
            let options = progress
                .as_ref()
                .map_or(fo::OVERVIEW_OPTIONS, fo::overview_options);
            let controls: Vec<(Bounds, &str, u32)> = match d.stage {
                ConflictStage::Overview => l
                    .options
                    .iter()
                    .zip(options)
                    .enumerate()
                    .map(|(i, (r, t))| (*r, t, CONFLICT + i as u32))
                    .collect(),
                ConflictStage::PerFile => l
                    .buttons
                    .iter()
                    .zip(fo::PER_FILE_BUTTONS)
                    .enumerate()
                    .map(|(i, (r, t))| (*r, t, CONFLICT + 3 + i as u32))
                    .collect(),
            };
            for (i, (r, text, id)) in controls.into_iter().enumerate() {
                s.control(Role::Button, text, b(r), Some(id))
                    .focused(i == d.focus);
            }
            s.control(Role::Button, "Close", b(l.close), Some(CONFLICT + 6));
            s.end();
            return;
        }
        if self.shown_jobs().next().is_none() {
            return;
        }
        let progress: Vec<OpProgress> = self.shown_jobs().map(|j| j.progress).collect();
        if self.ops.minimized {
            s.control(
                Role::Button,
                &fo::caption(&progress),
                b(fo::pill(self.layout.width, self.layout.height)),
                Some(PROGRESS + 3),
            );
            return;
        }
        let l = self.progress_layout();
        s.begin(Role::Dialog, &fo::caption(&progress), b(l.panel));
        for (i, (job, sec)) in self.shown_jobs().zip(&l.sections).enumerate() {
            s.text(Role::Text, &job.summary, b(sec.summary));
            s.text(Role::Text, &fo::headline(&job.progress), b(sec.headline));
            let pause = if job.progress.state == OpState::Paused {
                "Resume"
            } else {
                "Pause"
            };
            s.control(
                Role::Button,
                pause,
                b(sec.pause),
                Some(PROGRESS + 10 + i as u32),
            );
            s.control(
                Role::Button,
                "Cancel",
                b(sec.cancel),
                Some(PROGRESS + 20 + i as u32),
            );
        }
        s.control(
            Role::Button,
            if l.details {
                "Fewer details"
            } else {
                "More details"
            },
            b(l.toggle),
            Some(PROGRESS + 1),
        );
        s.control(Role::Button, "Close", b(l.close), Some(PROGRESS));
        s.end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(name: &str, object: u64, directory: bool) -> DirEntry {
        let mut units = [0u16; door_service_protocol::files::NAME_UNITS];
        let mut len = 0;
        for (slot, unit) in units.iter_mut().zip(name.encode_utf16()) {
            *slot = unit;
            len += 1;
        }
        DirEntry {
            cookie: object,
            name: units,
            name_len: len,
            info: FileInfo {
                attributes: 0,
                bytes: 10,
                object,
                version: 1,
                directory,
            },
            times: FileTimes::NONE,
        }
    }
    fn folder() -> Files {
        let mut f = Files::empty("/users/1001".into(), Controls::default());
        f.navigate_entries(
            "/users/1001/Documents".into(),
            alloc::vec![entry("a.txt", 40, false), entry("b.txt", 41, false)],
        );
        f
    }
    #[test]
    fn cut_copy_and_paste_follow_the_selection_and_cut_items_fade() {
        let mut f = folder();
        assert_eq!(f.copy_block(), Some(context_menu::SELECT_FIRST));
        f.selected = Some(0);
        f.copy_selection(true);
        assert!(f.ops.clipboard.cut);
        assert!(f.ops.clipboard.is_cut("/users/1001/Documents/a.txt"));
        assert!(matches!(f.ops.queue.last(), Some(Clip::Set)));
        // Without a broker the in-process clipboard pastes.
        assert_eq!(f.paste_block(), None);
        f.paste();
        assert!(matches!(f.ops.queue.last(), Some(Clip::Paste(d)) if d == "/users/1001/Documents"));
        // Another owner's clipboard ends the cut.
        f.clipboard_changed(9, cb::mask(cb::TEXT));
        assert!(f.ops.clipboard.is_empty());
        assert_eq!(f.paste_block(), Some("There's nothing to paste."));
    }
    #[test]
    fn known_folders_are_never_cut_or_deleted() {
        let mut f = Files::empty("/users/1001".into(), Controls::default());
        f.navigate_entries(
            "/users/1001".into(),
            alloc::vec![entry("Documents", 30, true), entry("notes", 31, true)],
        );
        f.selected = f.entries.iter().position(|e| Files::name(e) == "Documents");
        assert!(f.delete_block().is_some());
        f.copy_selection(true);
        assert!(f.ops.clipboard.is_empty());
        f.selected = f.entries.iter().position(|e| Files::name(e) == "notes");
        assert_eq!(f.delete_block(), None);
    }
    #[test]
    fn the_recycle_bin_view_has_its_own_title_commands_and_no_paths() {
        let mut f = folder();
        f.ops.bin_rows = alloc::vec![RecycleRow::new(
            &door_service_protocol::files::RecycleEntry::new(
                5,
                0,
                10,
                1,
                false,
                "/users/1001/Documents/old.txt"
            )
        )];
        let mut e = entry("old.txt", fo::recycle_object(5), false);
        e.times.modified = 0;
        f.navigate_entries(RECYCLE_BIN.into(), alloc::vec![e]);
        assert!(f.recycle_view());
        assert_eq!(f.title(), "Recycle Bin");
        assert_eq!(Files::history_label(RECYCLE_BIN), "Recycle Bin");
        assert!(matches!(f.bin_icon(), Icon::RecycleBinFull));
        f.selected = Some(0);
        assert!(f.path_for(&f.entries[0]).is_none());
        assert!(f.rename_candidate().is_none());
        assert_eq!(f.selected_bin_ids(), [5]);
        let ids: Vec<u32> = f.command_controls().iter().map(|c| c.1).collect();
        assert!(ids.contains(&49) && ids.contains(&50), "{ids:?}");
        assert_eq!(f.command_reason(48), None, "Delete purges (confirmed)");
        assert!(f.paste_block().is_some());
        f.ask_purge();
        let p = f.ops.prompt.as_ref().unwrap();
        assert_eq!(p.kind, PromptKind::Purge);
        assert!(
            p.body
                .starts_with("Are you sure you want to permanently delete this file?")
        );
        assert!(f.modal());
    }
    #[test]
    fn progress_dialog_and_pill_hit_tests_and_permanent_delete_texts() {
        let mut f = folder();
        f.push_job(7, FileOpMode::Copy, "Copying 1 item from A to B".into(), 1);
        assert_eq!(
            f.progress_target(500, 300),
            None,
            "hidden until it runs a while"
        );
        f.ops.jobs[0].shown = true;
        let l = f.progress_layout();
        let s = l.sections[0];
        assert_eq!(
            f.progress_target(s.cancel.x + 2, s.cancel.y + 2),
            Some(PROGRESS + 20)
        );
        assert_eq!(
            f.progress_target(l.close.x + 2, l.close.y + 2),
            Some(PROGRESS)
        );
        f.ops.minimized = true;
        let pill = fo::pill(f.layout.width, f.layout.height);
        assert_eq!(
            f.progress_target(pill.x + 4, pill.y + 4),
            Some(PROGRESS + 3)
        );
        f.selected = Some(0);
        f.ask_permanent(alloc::vec![("/users/1001/Documents/a.txt".into(), false)]);
        let (title, body, labels) = f.removal_texts();
        assert_eq!(title, "Delete File");
        assert_eq!(
            body,
            "Are you sure you want to permanently delete this file?\na.txt"
        );
        assert_eq!(labels, ["Yes", "No"]);
    }
    #[test]
    fn a_shown_copy_paints_its_progress_dialog() {
        let paint = |f: &Files| {
            let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(1).unwrap();
            let (w, h) = (f.layout.width, f.layout.height);
            let mut px = alloc::vec![0u8; (w * h * 4) as usize];
            let images = ui::ImageCache::default();
            let mut v = ui::View::new_scaled(&mut px, w, h, &fonts, 1).with_images(&images);
            f.draw(&mut v);
            px
        };
        let mut f = folder();
        f.configure(1000, 640);
        f.push_job(
            7,
            FileOpMode::Copy,
            "Copying 3 items from big to cancel".into(),
            3,
        );
        let hidden = paint(&f);
        f.ops.jobs[0].shown = true;
        let shown = paint(&f);
        let l = f.progress_layout();
        let changed = hidden
            .chunks_exact(4)
            .zip(shown.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            changed > 2000,
            "progress dialog painted {changed} px at {:?}",
            l.panel
        );
        // Mid open animation: faded and smaller than the settled window.
        f.ops.opened_at = Some(0);
        f.ops.now = OPEN_TICKS / 4;
        let opening = paint(&f);
        let differs = |a: &[u8], b: &[u8]| {
            a.chunks_exact(4)
                .zip(b.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count()
        };
        assert!(differs(&opening, &hidden) > 1000 && differs(&opening, &shown) > 1000);
        // Finished: the window closes from the last state, then is gone.
        f.ops.opened_at = None;
        let job = f.ops.jobs.remove(0);
        f.ops.fading = Some((job, 100));
        f.ops.now = 101;
        assert!(differs(&paint(&f), &hidden) > 1000, "closing");
        assert_eq!(
            f.progress_target(500, 300),
            None,
            "a closing dialog takes no input"
        );
        f.ops.now = 100 + CLOSE_TICKS + 1;
        assert_eq!(differs(&paint(&f), &hidden), 0, "closed");
    }
    /// Writes the copy dialog to DUMP_DIR (run with --ignored).
    #[test]
    #[ignore]
    fn render_copy_dialog() {
        extern crate std;
        let dir = std::env::var("DUMP_DIR").unwrap();
        let mut f = folder();
        f.configure(1000, 640);
        f.push_job(
            7,
            FileOpMode::Copy,
            "Copying 1 item from src to dst".into(),
            1,
        );
        f.ops.jobs[0].shown = true;
        let p = &mut f.ops.jobs[0].progress;
        p.state = OpState::Running;
        (p.bytes_done, p.bytes_total) = (85, 100);
        let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(1).unwrap();
        let (w, h) = (f.layout.width, f.layout.height);
        let mut px = alloc::vec![0u8; (w * h * 4) as usize];
        let images = ui::ImageCache::default();
        let mut v = ui::View::new_scaled(&mut px, w, h, &fonts, 1).with_images(&images);
        f.draw(&mut v);
        let mut out = std::vec::Vec::new();
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(54 + w * h * 4).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(w as i32).to_le_bytes());
        out.extend_from_slice(&(-(h as i32)).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&[0; 24]);
        out.extend_from_slice(&px);
        std::fs::write(std::format!("{dir}/copy-dialog.bmp"), out).unwrap();
    }
}
