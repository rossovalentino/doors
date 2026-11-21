//! Explorer details pane: the real metadata Storage reports for the selection
//! (or for the open folder / This PC when nothing is selected), tag chips with
//! inline add/remove, and quick actions that either work or say why not.
//! Paint, hit testing and scrolling share one measured layout.
use super::modern::paint::{
    ACCENT, HOVER, MUTED, SELECTED, TEXT, control, fill, label, line, plate, tag_color, wrap,
};
use super::*;
use door_service_client::{File, FileInfo};
use door_workspace::explorer_size::State;
use door_workspace::explorer_tags::TagGrid;
use door_workspace::file_metadata::{self as metadata, Outcome, Report, Sparse, Stream};

/// Height of the "Select a single file" card.
pub(super) const INFO_CARD: i32 = 78;
pub(super) const QA_TERMINAL: u32 = 4000;
pub(super) const QA_ARCHIVE: u32 = 4001;
pub(super) const QA_EMAIL: u32 = 4002;
pub(super) const QA_CONVERT: u32 = 4003;
/// Remove ("x") of the chip for tag i.
pub(super) const CHIP_REMOVE: u32 = 4010;
/// Body of the chip for tag i (shows items with the tag).
pub(super) const CHIP: u32 = 4020;
pub(super) const CHIP_ADD: u32 = 4030;
pub(super) const CHIP_EDITOR: u32 = 4031;
/// Body top inside the pane (below the Details/Contents/Tags tabs) at 100%.
const BODY: i32 = 224;
const LABEL_W: u32 = 128;
const ROW_H: i32 = 24;
/// Body top at the applied text size: the header's title, subtitle, Open
/// button and tab row each hold one line of text and grow with it.
pub(super) fn body() -> i32 {
    BODY + 4 * xl::grow()
}
fn row_h() -> i32 {
    ROW_H + xl::grow()
}
/// Label column: widens with the text, up to half the pane's content.
fn label_w(r: Bounds) -> u32 {
    LABEL_W.max(door_workspace::palette::text_len(LABEL_W).min(r.width.saturating_sub(40) / 2))
}

/// What a file's bytes say about it (`door_workspace::file_metadata`).
#[derive(Clone, Debug, Default)]
pub(super) struct Meta {
    pub(super) report: Report,
    pub(super) failed: bool,
}
/// Reads a file's metadata in the background, a few RPCs per tick: header,
/// chunk tables and tail sections on demand (bounded by
/// `file_metadata::MAX_FETCH` / `MAX_ROUNDS`), or the whole of a text-like
/// file up to `file_metadata::MAX_STREAM`.
pub(super) struct MetaJob {
    file: File,
    info: FileInfo,
    generation: u64,
    name: String,
    mode: MetaMode,
}
// One per open details pane, so the variant size difference is immaterial.
#[allow(clippy::large_enum_variant)]
enum MetaMode {
    Ranges {
        data: Sparse,
        fetch: Option<(u64, u64)>,
        rounds: u32,
    },
    Stream {
        stream: Stream,
        read: u64,
        limit: u64,
    },
    /// A program's whole file for its signature, after the header pass.
    Signature { report: Report, bytes: Vec<u8> },
}
const META_CALLS: usize = 48;
/// The wall clock as Unix seconds (certificate validity), if set.
fn unix_now() -> Option<i64> {
    let filetime = door_sys::wall_time().ok()?;
    i64::try_from(filetime / 10_000_000)
        .ok()?
        .checked_sub(11_644_473_600)
}
/// Smallest range fetched per round (headers are usually contiguous).
const FETCH_MIN: u64 = 4096;

/// A label/value row; an empty value makes it a section header ("Image",
/// "File"), as Windows groups Properties > Details.
pub(super) struct Row {
    pub(super) label: &'static str,
    pub(super) value: String,
    pub(super) muted: bool,
}
impl Row {
    fn section(title: &'static str) -> Self {
        Self {
            label: title,
            value: String::new(),
            muted: false,
        }
    }
    pub(super) fn is_section(&self) -> bool {
        self.value.is_empty()
    }
}
pub(super) struct QuickAction {
    pub(super) id: u32,
    pub(super) label: String,
    pub(super) icon: Icon,
    pub(super) reason: &'static str,
}
impl QuickAction {
    fn new(id: u32, label: &str, icon: Icon) -> Self {
        Self {
            id,
            label: label.into(),
            icon,
            reason: "",
        }
    }
    fn unavailable(mut self, reason: &'static str) -> Self {
        self.reason = reason;
        self
    }
    pub(super) fn enabled(&self) -> bool {
        self.reason.is_empty()
    }
}
/// Pane-space layout (y before scrolling) of the Details tab body.
pub(super) struct PaneLayout {
    pub(super) rows: Vec<(Row, i32, usize)>,
    /// Windows' "Select a single file" card (nothing selected), its top.
    pub(super) info: Option<i32>,
    pub(super) bar: Option<i32>,
    pub(super) tags_y: Option<i32>,
    pub(super) chips: Vec<(Bounds, usize)>,
    pub(super) add: Option<Bounds>,
    pub(super) actions_y: i32,
    pub(super) actions: Vec<(QuickAction, Bounds)>,
    pub(super) status_y: i32,
    pub(super) height: i32,
}

pub(super) fn grouped(n: u64) -> String {
    let digits = format!("{n}");
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
/// "1.2 KB (1,234 bytes)", as Windows' Properties shows sizes.
pub(super) fn size_text(bytes: u64) -> String {
    if bytes < 1024 {
        format!(
            "{} {}",
            grouped(bytes),
            if bytes == 1 { "byte" } else { "bytes" }
        )
    } else {
        format!("{} ({} bytes)", previewing::size(bytes), grouped(bytes))
    }
}

impl Files {
    fn parent_display(path: &str) -> String {
        if path == "/users" {
            return "C:\\".into();
        }
        explorer_path::display(path.rsplit_once('/').map_or("/", |(parent, _)| {
            if parent.is_empty() { "/" } else { parent }
        }))
    }
    pub(super) fn meta_for(&self, info: FileInfo) -> Option<&Meta> {
        self.meta_cache
            .iter()
            .find(|(i, _)| *i == info)
            .map(|(_, m)| m)
    }
    pub(super) fn detail_rows(&self, d: &Detail) -> Vec<Row> {
        let row = |label, value: String| Row {
            label,
            value,
            muted: false,
        };
        let quiet = |label, value: &str| Row {
            label,
            value: value.into(),
            muted: true,
        };
        let mut rows = Vec::new();
        let drive = self.drives().iter().find(|drive| drive.path == d.path);
        if let Some(drive) = drive {
            rows.push(row("Type", "Local Disk".into()));
            rows.push(row("Name", drive.label.into()));
            match self.volume {
                VolumeState::Known(u) => {
                    rows.push(row("Capacity", size_text(u.total_bytes)));
                    rows.push(row("Used space", size_text(u.used_bytes())));
                    rows.push(row("Free space", size_text(u.free_bytes)));
                }
                VolumeState::Unavailable => rows.push(quiet("Capacity", "Unavailable")),
                VolumeState::Unknown => rows.push(quiet("Capacity", "Reading...")),
            }
            return rows;
        }
        if d.info.directory {
            rows.push(row("Type", "File folder".into()));
            rows.push(row("Location", Self::parent_display(&d.path)));
            match self.folder_size() {
                Some(t) => {
                    let prefix = match t.state {
                        State::Calculating => "Calculating... ",
                        State::Partial => "At least ",
                        State::Complete => "",
                    };
                    rows.push(Row {
                        label: "Size",
                        value: format!("{prefix}{}", size_text(t.bytes)),
                        muted: t.state != State::Complete,
                    });
                    rows.push(Row {
                        label: "Contains",
                        value: format!(
                            "{} {}, {} {}{}",
                            grouped(t.files),
                            if t.files == 1 { "file" } else { "files" },
                            grouped(t.folders),
                            if t.folders == 1 { "folder" } else { "folders" },
                            if t.unreadable > 0 {
                                format!(", {} unreadable", t.unreadable)
                            } else {
                                String::new()
                            }
                        ),
                        muted: t.state != State::Complete,
                    });
                }
                None => rows.push(quiet("Size", "Calculating...")),
            }
            rows.push(row(
                "Items",
                format!(
                    "{}{} directly inside",
                    d.children.len(),
                    if d.children_done && !d.truncated {
                        ""
                    } else {
                        "+"
                    }
                ),
            ));
        } else {
            // Per-type sections first, then "File", as Windows orders them.
            let meta = self.meta_for(d.info);
            match meta {
                None => rows.push(quiet("Details", "Reading...")),
                Some(m) if m.failed => rows.push(quiet("Details", "Could not be read")),
                Some(m) => {
                    for (section, fields) in m.report.sections() {
                        rows.push(Row::section(section.title()));
                        for f in fields {
                            rows.push(row(f.label, f.value.clone()));
                        }
                    }
                    if let Some(note) = m.report.note {
                        rows.push(quiet("Note", note));
                    }
                }
            }
            rows.push(Row::section("File"));
            let ext = d
                .name
                .rsplit_once('.')
                .map(|(_, e)| e)
                .filter(|e| !e.is_empty());
            let kind = meta.and_then(|m| m.report.format).unwrap_or(d.kind.label());
            rows.push(row(
                "Type",
                match ext {
                    Some(e) => format!("{kind} (.{})", e.to_ascii_lowercase()),
                    None => kind.into(),
                },
            ));
            rows.push(row("Size", size_text(d.info.bytes)));
            rows.push(row("Location", Self::parent_display(&d.path)));
        }
        // Windows omits properties it cannot read: a time the volume did not
        // record (0) shows no row. Object IDs and versions are internal.
        for (label, time) in [
            ("Date modified", d.times.modified),
            ("Date created", d.times.created),
            ("Date accessed", d.times.accessed),
        ] {
            let text = self.date_text(time);
            if !text.is_empty() {
                rows.push(row(label, text));
            }
        }
        rows
    }
    pub(super) fn quick_actions(&self) -> Vec<QuickAction> {
        let mut actions = Vec::new();
        let Some(d) = self.detail.as_ref() else {
            if self.selection_count() > 1 {
                actions.push(self.archive_action());
                actions.push(
                    QuickAction::new(QA_EMAIL, "Compress and email", Icon::Mail)
                        .unavailable("No mail app is installed."),
                );
            }
            return actions;
        };
        let terminal = || {
            let action = QuickAction::new(QA_TERMINAL, "Open in Terminal", Icon::Terminal);
            if d.path == self.home {
                action
            } else {
                action.unavailable(context_menu::TERMINAL_HOME)
            }
        };
        if self.drives().iter().any(|drive| drive.path == d.path) {
            actions.push(QuickAction::new(915, "Open", Icon::Open));
            actions.push(QuickAction::new(916, "Open in new tab", Icon::NewTab));
            actions.push(terminal());
            actions.push(QuickAction::new(929, "Refresh information", Icon::Refresh));
            return actions;
        }
        if d.info.directory {
            actions.push(QuickAction::new(916, "Open in new tab", Icon::NewTab));
            actions.push(terminal());
        } else if d.kind.previewable() {
            actions.push(QuickAction::new(
                913,
                if d.kind == Kind::Image {
                    "Preview image"
                } else {
                    "Read file"
                },
                Icon::Preview,
            ));
        }
        actions.push(self.archive_action());
        actions.push(
            QuickAction::new(QA_EMAIL, "Compress and email", Icon::Mail)
                .unavailable("No mail app is installed."),
        );
        if !d.info.directory {
            match Self::convert_target(&d.name) {
                Some((_, label)) => {
                    let mut action = QuickAction::new(QA_CONVERT, label, Icon::Convert);
                    if let Some(job) = self.convert.as_ref() {
                        action.label = format!("Converting to {}...", job.name);
                        action = action.unavailable("A conversion is in progress.");
                    } else if d.info.bytes > previewing::MAX_VIEW_INPUT as u64 {
                        action = action.unavailable("Images larger than 6 MB cannot be converted.");
                    }
                    actions.push(action);
                }
                None => actions.push(
                    QuickAction::new(QA_CONVERT, "Convert", Icon::Convert)
                        .unavailable("Only PNG, JPEG, BMP, QOI and PPM images can be converted."),
                ),
            }
        }
        if !d.current
            && self
                .rename_candidate()
                .is_some_and(|e| e.info.object == d.info.object)
        {
            actions.push(QuickAction::new(917, "Rename", Icon::Rename));
        }
        actions.push(QuickAction::new(914, "Manage tags", Icon::Tag));
        actions.push(QuickAction::new(929, "Refresh information", Icon::Refresh));
        actions
    }
    fn archive_action(&self) -> QuickAction {
        match self.archive.as_ref() {
            Some(job) => {
                let mut a = QuickAction::new(QA_ARCHIVE, "", Icon::Zip)
                    .unavailable("An archive is being created.");
                a.label = format!("Creating {}... {}%", job.name, job.percent());
                a
            }
            None => QuickAction::new(QA_ARCHIVE, "Create archive", Icon::Zip),
        }
    }
    /// Tags assigned to the inspected item, in catalog order.
    fn detail_tags(&self) -> Vec<usize> {
        let tags = self.detail.as_ref().map_or(0, |d| d.tags);
        (0..self.tags.names.len())
            .filter(|i| tags & (1 << i) != 0)
            .collect()
    }
    pub(super) fn pane_layout(&self, r: Bounds, measure: &dyn Fn(&str) -> u32) -> PaneLayout {
        let x = r.x + 20;
        let right = r.x + r.width as i32 - 20;
        let value_w = (right - x - label_w(r) as i32 - 8).max(0) as u32;
        let g = xl::grow();
        let mut y = r.y + body() + 12;
        let mut rows = Vec::new();
        // Nothing selected (the open folder, Home, This PC): the card first.
        let info = (self.selection_count() == 0 && self.detail.as_ref().is_none_or(|d| d.current))
            .then_some(y);
        if info.is_some() {
            y += INFO_CARD + 12;
        }
        let source: Vec<Row> = match self.detail.as_ref() {
            Some(d) => self.detail_rows(d),
            None => self.summary_rows(),
        };
        for (i, row) in source.into_iter().enumerate() {
            if row.is_section() {
                y += if i == 0 { 0 } else { 10 };
                rows.push((row, y, 1));
                y += row_h() + 2;
                continue;
            }
            let lines =
                door_workspace::dialog::lines(&ui::display_ascii(&row.value), value_w, measure)
                    .len()
                    .clamp(1, 3);
            rows.push((row, y, lines));
            y += row_h() * lines as i32 + 4;
        }
        let bar = self
            .detail
            .as_ref()
            .filter(|d| self.drives().iter().any(|drive| drive.path == d.path))
            .map(|_| y)
            .or_else(|| (self.detail.is_none() && self.overview).then_some(y));
        if bar.is_some() {
            y += 18;
        }
        let mut chips = Vec::new();
        let mut add = None;
        let mut tags_y = None;
        let taggable = self
            .detail
            .as_ref()
            .is_some_and(|d| !self.drives().iter().any(|drive| drive.path == d.path))
            && self.tags_error.is_empty();
        if taggable {
            y += 8;
            tags_y = Some(y);
            y += 30 + g;
            let (mut cx, mut cy) = (x, y);
            for i in self.detail_tags() {
                let w =
                    (measure(&ui::display_ascii(&self.tags.names[i])) + 58).min((right - x) as u32);
                if cx + w as i32 > right && cx > x {
                    cx = x;
                    cy += 34 + g;
                }
                chips.push((
                    Bounds {
                        x: cx,
                        y: cy,
                        width: w,
                        height: (28 + g) as u32,
                    },
                    i,
                ));
                cx += w as i32 + 8;
            }
            let add_w = if self.editor == 6 {
                (right - x) as u32
            } else {
                measure("Add tag") + 42
            };
            if cx + add_w as i32 > right && cx > x {
                cx = x;
                cy += 34 + g;
            }
            let b = Bounds {
                x: cx,
                y: cy,
                width: add_w,
                height: (28 + g) as u32,
            };
            add = Some(b);
            y = cy + 28 + g + (if self.editor == 6 { 26 + g } else { 0 });
        }
        y += 18;
        let actions_y = y;
        y += 40 + g;
        let mut actions = Vec::new();
        for action in self.quick_actions() {
            let h = if action.enabled() { 36 + g } else { 52 + 2 * g } as u32;
            actions.push((
                action,
                Bounds {
                    x: r.x + 12,
                    y,
                    width: r.width.saturating_sub(24),
                    height: h,
                },
            ));
            y += h as i32 + 2;
        }
        let status_y = y + 8;
        let status = self
            .detail
            .as_ref()
            .map_or(0, |d| if d.status.is_empty() { 0 } else { 80 });
        PaneLayout {
            rows,
            info,
            bar,
            tags_y,
            chips,
            add,
            actions_y,
            actions,
            status_y,
            height: status_y + status + 24 - (r.y + body()),
        }
    }
    /// No selection on This PC, or several items selected.
    fn summary_rows(&self) -> Vec<Row> {
        let row = |label, value: String| Row {
            label,
            value,
            muted: false,
        };
        let mut rows = Vec::new();
        if self.selection_count() > 1 {
            let picked: Vec<&DirEntry> = self
                .entries
                .iter()
                .filter(|e| self.marked.contains(&e.info.object))
                .collect();
            let files: Vec<_> = picked.iter().filter(|e| !e.info.directory).collect();
            let folders = picked.len() - files.len();
            rows.push(row("Files", grouped(files.len() as u64)));
            rows.push(row("Folders", grouped(folders as u64)));
            rows.push(Row {
                label: "Size",
                value: format!(
                    "{}{}",
                    size_text(files.iter().map(|e| e.info.bytes).sum()),
                    if folders > 0 { " in files" } else { "" }
                ),
                muted: false,
            });
            rows.push(row("Location", explorer_path::display(&self.cwd)));
            return rows;
        }
        if self.home_view {
            for (label, value) in self.home_summary() {
                rows.push(row(label, value));
            }
            rows.push(Row {
                label: "Recent list",
                value: String::from(if self.recent.persistent() {
                    "Saved in your profile"
                } else {
                    "This session only (the saved list could not be read)"
                }),
                muted: true,
            });
            return rows;
        }
        if !self.overview {
            // The open folder, until the next tick inspects it in full.
            rows.push(row("Type", String::from("File folder")));
            rows.push(row("Location", Self::parent_display(&self.cwd)));
            let n = self.item_count();
            rows.push(row(
                "Contains",
                format!(
                    "{} {}",
                    grouped(n as u64),
                    if n == 1 { "item" } else { "items" }
                ),
            ));
            return rows;
        }
        // The PC itself (Settings > System > About > Rename this PC).
        rows.push(row("Device name", String::from(self.device_name.as_str())));
        rows.push(row(
            "Devices",
            format!(
                "{} {}",
                self.drives().len(),
                if self.drives().len() == 1 {
                    "drive"
                } else {
                    "drives"
                }
            ),
        ));
        match self.volume {
            VolumeState::Known(u) => {
                rows.push(row("Capacity", size_text(u.total_bytes)));
                rows.push(row("Free space", size_text(u.free_bytes)));
            }
            VolumeState::Unavailable => rows.push(Row {
                label: "Capacity",
                value: "Unavailable".into(),
                muted: true,
            }),
            VolumeState::Unknown => (),
        }
        rows
    }
    /// The header's Open/Preview button (below the title and subtitle).
    pub(super) fn pane_open_button(r: Bounds) -> Bounds {
        let g = xl::grow();
        Bounds {
            x: r.x + 20,
            y: r.y + 124 + 2 * g,
            width: 118,
            height: (34 + g) as u32,
        }
    }
    /// The Details/Contents/Tags tab row, ending above the body.
    pub(super) fn pane_tab_row(r: Bounds) -> Bounds {
        let g = xl::grow();
        Bounds {
            x: r.x + 12,
            y: r.y + 180 + 3 * g,
            width: r.width.saturating_sub(24),
            height: (36 + g) as u32,
        }
    }
    pub(super) fn pane_scroll_rows(&self) -> usize {
        self.detail_height.get()
    }
    /// Pane hit testing below the header: tabs, chips, quick actions.
    pub(super) fn details_target(
        &self,
        r: Bounds,
        x: i32,
        y: i32,
        fonts: &FontAtlas,
    ) -> Option<u32> {
        let measure = |s: &str| {
            fonts
                .measure(s, TextStyle::Regular14)
                .map_or(0, |m| m.advance_width)
        };
        let l = self.pane_layout(r, &measure);
        for (b, tag) in &l.chips {
            if b.contains(x, y) {
                return Some(
                    if x >= b.x + b.width as i32 - 26 {
                        CHIP_REMOVE
                    } else {
                        CHIP
                    } + *tag as u32,
                );
            }
        }
        if let Some(b) = l.add
            && b.contains(x, y)
        {
            return Some(if self.editor == 6 {
                CHIP_EDITOR
            } else {
                CHIP_ADD
            });
        }
        for (action, b) in &l.actions {
            if b.contains(x, y) {
                return action.enabled().then_some(action.id);
            }
        }
        None
    }
    pub(super) fn pane_extra(&mut self, id: u32, storage: &mut Storage) -> Result<(), Error> {
        match id {
            QA_TERMINAL => self.pending_launch = Some(9),
            QA_ARCHIVE => {
                let sources = if self.selection_count() > 1 {
                    self.entries
                        .iter()
                        .filter(|e| self.marked.contains(&e.info.object))
                        .filter_map(|e| self.path_for(e))
                        .collect()
                } else {
                    self.detail
                        .as_ref()
                        .map(|d| alloc::vec![d.path.clone()])
                        .unwrap_or_default()
                };
                self.begin_archive(storage, sources);
            }
            QA_CONVERT => self.begin_convert(storage),
            CHIP_ADD => self.begin_add_tag(),
            CHIP_EDITOR => self.editor = 6,
            _ if (CHIP_REMOVE..CHIP_REMOVE + 8).contains(&id) => {
                self.remove_tag_from_detail((id - CHIP_REMOVE) as usize, storage)?
            }
            _ if (CHIP..CHIP + 8).contains(&id) => self.filter_tag((id - CHIP) as usize),
            _ => (),
        }
        Ok(())
    }
    /// Starts or advances the metadata read of the inspected file.
    pub(super) fn meta_step(&mut self, storage: &mut Storage) -> bool {
        let wanted = self
            .detail
            .as_ref()
            .filter(|d| !d.info.directory && !d.current && self.details_visible)
            .filter(|d| self.meta_for(d.info).is_none())
            .map(|d| (d.path.clone(), d.name.clone(), d.info));
        let current = self.meta_job.as_ref().is_some_and(|j| {
            wanted.as_ref().is_some_and(|w| w.2 == j.info) && j.generation == self.nav_generation
        });
        if !current {
            if let Some(job) = self.meta_job.take() {
                let _ = storage.close(job.file);
            }
            let Some((path, name, info)) = wanted else {
                return false;
            };
            let file = match storage.open(&path, Disposition::OpenExisting, READ, SHARE_ALL) {
                Ok(f) => f,
                Err(_) => {
                    self.remember_meta(
                        info,
                        Meta {
                            failed: true,
                            ..Meta::default()
                        },
                    );
                    return true;
                }
            };
            self.meta_job = Some(MetaJob {
                file,
                info,
                generation: self.nav_generation,
                name,
                mode: MetaMode::Ranges {
                    data: Sparse::new(info.bytes),
                    fetch: None,
                    rounds: 0,
                },
            });
        }
        let job = self.meta_job.as_mut().unwrap();
        let size = job.info.bytes;
        let mut result = None;
        for _ in 0..META_CALLS {
            let mut chunk = [0u8; door_service_protocol::CHUNK_BYTES];
            let mut stream = None;
            let mut signed = None;
            match &mut job.mode {
                MetaMode::Ranges {
                    data,
                    fetch,
                    rounds,
                } => {
                    if fetch.is_none() {
                        match metadata::read(&job.name, data) {
                            Outcome::Done(report)
                                if metadata::signable(&job.name)
                                    && size <= metadata::MAX_SIGNED =>
                            {
                                signed = Some(report);
                            }
                            Outcome::Done(report) => {
                                result = Some(Meta {
                                    report,
                                    failed: false,
                                });
                                break;
                            }
                            Outcome::Stream(kind) => stream = Some(kind),
                            Outcome::Need {
                                offset,
                                len,
                                partial,
                            } => {
                                *rounds += 1;
                                if *rounds > metadata::MAX_ROUNDS
                                    || data.held() >= metadata::MAX_FETCH
                                {
                                    result = Some(Meta {
                                        report: partial,
                                        failed: false,
                                    });
                                    break;
                                }
                                *fetch = Some((
                                    offset,
                                    offset.saturating_add(len.max(FETCH_MIN)).min(size),
                                ));
                            }
                        }
                    }
                    if let Some((at, end)) = fetch.filter(|_| signed.is_none()) {
                        let want = (end - at).min(chunk.len() as u64) as usize;
                        match storage.read(&job.file, at, &mut chunk[..want]) {
                            Ok(n) if n > 0 => {
                                data.insert(at, &chunk[..n]);
                                let next = at + n as u64;
                                *fetch = (next < end).then_some((next, end));
                            }
                            _ => {
                                result = Some(Meta {
                                    failed: true,
                                    ..Meta::default()
                                });
                                break;
                            }
                        }
                    }
                }
                MetaMode::Signature { report, bytes } => {
                    if bytes.len() as u64 >= size {
                        let mut report = core::mem::take(report);
                        if let Some(now) = unix_now() {
                            let store = metadata::root_store(storage);
                            metadata::signature_with(&mut report, bytes, now, &store);
                        }
                        result = Some(Meta {
                            report,
                            failed: false,
                        });
                        break;
                    }
                    let want = (size - bytes.len() as u64).min(chunk.len() as u64) as usize;
                    match storage.read(&job.file, bytes.len() as u64, &mut chunk[..want]) {
                        Ok(n) if n > 0 => bytes.extend_from_slice(&chunk[..n]),
                        _ => {
                            result = Some(Meta {
                                report: core::mem::take(report),
                                failed: false,
                            });
                            break;
                        }
                    }
                }
                MetaMode::Stream {
                    stream,
                    read,
                    limit,
                } => {
                    if *read >= *limit {
                        result = Some(Meta {
                            report: stream.finish(*read >= size),
                            failed: false,
                        });
                        break;
                    }
                    let want = (*limit - *read).min(chunk.len() as u64) as usize;
                    match storage.read(&job.file, *read, &mut chunk[..want]) {
                        Ok(n) if n > 0 => {
                            stream.feed(&chunk[..n]);
                            *read += n as u64;
                        }
                        _ => {
                            result = Some(Meta {
                                failed: true,
                                ..Meta::default()
                            });
                            break;
                        }
                    }
                }
            }
            if let Some(kind) = stream {
                job.mode = MetaMode::Stream {
                    stream: Stream::new(kind),
                    read: 0,
                    limit: size.min(metadata::MAX_STREAM),
                };
            }
            if let Some(report) = signed {
                let mut bytes = Vec::new();
                if bytes.try_reserve_exact(size as usize).is_err() {
                    result = Some(Meta {
                        report,
                        failed: false,
                    });
                    break;
                }
                job.mode = MetaMode::Signature { report, bytes };
            }
        }
        let changed = result.is_some();
        if let Some(meta) = result {
            let job = self.meta_job.take().unwrap();
            let unchanged = storage.info(&job.file).is_ok_and(|i| i == job.info);
            let _ = storage.close(job.file);
            self.remember_meta(
                job.info,
                if unchanged {
                    meta
                } else {
                    Meta {
                        failed: true,
                        ..Meta::default()
                    }
                },
            );
        }
        // Repaint only when a result lands, not on every read.
        changed
    }
    fn remember_meta(&mut self, info: FileInfo, meta: Meta) {
        if self.meta_cache.len() == 16 {
            self.meta_cache.remove(0);
        }
        self.meta_cache.push((info, meta));
    }
}

/// Painting.
impl Files {
    pub(super) fn draw_pane(&self, v: &mut ui::View<'_>, r: Bounds) {
        v.rounded_asset(
            r.x,
            r.y,
            r.width,
            r.height,
            0,
            fill(v, [255; 4], [25, 25, 25, 255]),
        );
        v.rect(r.x, r.y, 1, r.height, [236, 231, 224, 255]);
        self.chip_hits.borrow_mut().clear();
        self.close_glyph(v, r.x + r.width as i32 - 21, r.y + 22, MUTED);
        if self.detail.is_none() && self.detail_tab == 2 && self.selection_count() <= 1 {
            v.text_clip_centered(
                r.x + 22,
                (
                    r.y + 24,
                    door_workspace::palette::text_grow(32, TextStyle::Semibold20),
                ),
                r.width.saturating_sub(64),
                "Tags",
                TextStyle::Semibold20,
                TEXT,
            );
            wrap(
                v,
                r.x + 22,
                r.y + 72,
                r.width.saturating_sub(44),
                "Create tags here, then select a file or folder to assign them.",
                4,
                MUTED,
            );
            self.draw_body(v, r, |s, v, shift| s.draw_user_tags(v, r, shift));
            return;
        }
        let multi = self.selection_count() > 1;
        // Header: icon, name and type.
        let (icon, title, subtitle) = match self.detail.as_ref() {
            Some(d) if self.drives().iter().any(|drive| drive.path == d.path) => (
                Icon::Drive,
                String::from(DRIVES[0].label),
                String::from("Local Disk"),
            ),
            Some(d) => (
                if d.info.directory {
                    Self::folder_icon(&d.name)
                } else {
                    Icon::for_file_name(&d.name)
                },
                if d.current {
                    String::from(self.title())
                } else {
                    d.name.clone()
                },
                if d.current {
                    format!(
                        "{} {}",
                        self.item_count(),
                        if self.item_count() == 1 {
                            "item"
                        } else {
                            "items"
                        }
                    )
                } else {
                    door_workspace::explorer_preview::type_name(&d.name, false)
                },
            ),
            None if multi => (
                Icon::Document,
                format!("{} items selected", self.selection_count()),
                String::from("Multiple types"),
            ),
            None if self.home_view => (
                Icon::Home,
                String::from("Home"),
                format!(
                    "{} {}",
                    self.item_count(),
                    if self.item_count() == 1 {
                        "item"
                    } else {
                        "items"
                    }
                ),
            ),
            None if self.overview => (
                Icon::Computer,
                String::from("This PC"),
                format!(
                    "{} {}",
                    self.drives().len(),
                    if self.drives().len() == 1 {
                        "drive"
                    } else {
                        "drives"
                    }
                ),
            ),
            // The open folder, until the next tick inspects it.
            None => (
                self.current_icon(),
                String::from(self.title()),
                format!(
                    "{} {}",
                    self.item_count(),
                    if self.item_count() == 1 {
                        "item"
                    } else {
                        "items"
                    }
                ),
            ),
        };
        let program = self
            .detail
            .as_ref()
            .filter(|d| !d.current && !d.info.directory)
            .map(|d| (d.name.as_str(), d.info));
        if !self.draw_program_icon(v, r.x + 18, r.y + 30, 80, program) {
            v.hero_icon(r.x + 18, r.y + 30, 80, icon);
        }
        // Header lines grow with the text size; the hero icon keeps its size.
        let g = xl::grow();
        v.text_clip_centered(
            r.x + 112,
            (r.y + 40, (32 + g) as u32),
            r.width.saturating_sub(150),
            &ui::display_ascii(&title),
            TextStyle::Semibold14,
            TEXT,
        );
        label(
            v,
            r.x + 112,
            r.y + 74 + g,
            r.width.saturating_sub(132),
            (22 + g) as u32,
            &subtitle,
            MUTED,
        );
        match self.detail.as_ref() {
            Some(d) if !d.current && (d.info.directory || d.kind.previewable()) => {
                let b = Self::pane_open_button(r);
                v.rounded(b.x, b.y, b.width, b.height, 5, SELECTED);
                control(
                    v,
                    b,
                    if d.info.directory {
                        Icon::Open
                    } else {
                        Icon::Preview
                    },
                    if d.info.directory { "Open" } else { "Preview" },
                    true,
                );
            }
            Some(d) if !d.current && !metadata::signable(&d.name) => label(
                v,
                r.x + 20,
                r.y + 124 + 2 * g,
                r.width.saturating_sub(40),
                (34 + g) as u32,
                "Preview not available",
                MUTED,
            ),
            _ => (),
        }
        let tab_width = r.width.saturating_sub(24) / 3;
        let second = if self.detail.as_ref().is_some_and(|d| !d.info.directory) {
            "Preview"
        } else {
            "Contents"
        };
        for (i, name) in ["Details", second, "Tags"].into_iter().enumerate() {
            let x = r.x + 12 + i as i32 * tab_width as i32;
            let w = v.measure(name);
            let tx = x + (tab_width.saturating_sub(w) / 2) as i32;
            let active = self.detail_tab == i as u8;
            let tab = Bounds {
                x,
                width: tab_width,
                ..Self::pane_tab_row(r)
            };
            if !active && v.hover.is_some_and(|(px, py)| tab.contains(px, py)) {
                v.rounded(
                    x + 4,
                    tab.y + 4,
                    tab_width.saturating_sub(8),
                    tab.height - 8,
                    4,
                    HOVER,
                );
            }
            label(
                v,
                tx,
                tab.y,
                tab_width.saturating_sub((tx - x) as u32),
                tab.height,
                name,
                if active { TEXT } else { MUTED },
            );
            if active {
                v.rounded(
                    x + 10,
                    tab.y + tab.height as i32 - 2,
                    tab_width.saturating_sub(20),
                    2,
                    1,
                    ACCENT,
                );
            }
        }
        line(v, r.x + 12, r.y + body() - 7, r.width.saturating_sub(24));
        self.draw_body(v, r, |s, v, shift| match s.detail_tab {
            0 => s.draw_information(v, r, shift),
            1 if s.detail.is_some() => s.draw_preview(v, r, shift),
            1 => wrap(
                v,
                r.x + 22,
                r.y + body() + 14 - shift,
                r.width.saturating_sub(44),
                "Select a single item to see its contents.",
                3,
                MUTED,
            ),
            _ => s.draw_user_tags(v, r, shift),
        });
    }
    fn draw_body(
        &self,
        v: &mut ui::View<'_>,
        r: Bounds,
        paint: impl FnOnce(&Self, &mut ui::View<'_>, i32),
    ) {
        let body_y = r.y + body();
        let body_h = v
            .height
            .saturating_sub((body_y + xl::status_height() as i32).max(0) as u32);
        if body_h > 0
            && let Ok(clip) =
                door_graphics_core::Rect::new(r.x + 1, body_y, r.width.saturating_sub(2), body_h)
        {
            v.viewport(clip, 0);
            paint(self, v, (self.detail_scroll * 24) as i32);
            v.reset_viewport();
        }
        if let Some((track, thumb)) = self.pane_scrollbar(r) {
            let hovered =
                v.hover.is_some_and(|(x, y)| track.contains(x, y)) || self.pane_drag.is_some();
            v.rounded(
                thumb.x + if hovered { 2 } else { 4 },
                thumb.y,
                if hovered { 6 } else { 3 },
                thumb.height,
                2,
                [172, 172, 172, 255],
            );
        }
    }
    fn draw_information(&self, v: &mut ui::View<'_>, r: Bounds, shift: i32) {
        let measure = |s: &str| v.measure(s);
        let l = self.pane_layout(r, &measure);
        self.detail_height.set(l.height.max(0) as usize);
        let x = r.x + 20;
        if let Some(top) = l.info {
            // Windows 11: an outlined card with an info glyph.
            let card = Bounds {
                x: r.x + 14,
                y: top - shift,
                width: r.width.saturating_sub(28),
                height: INFO_CARD as u32,
            };
            v.rounded_asset(
                card.x,
                card.y,
                card.width,
                card.height,
                6,
                fill(v, [229, 229, 229, 255], [58, 58, 58, 255]),
            );
            v.rounded_asset(
                card.x + 1,
                card.y + 1,
                card.width.saturating_sub(2),
                card.height.saturating_sub(2),
                5,
                fill(v, [251, 251, 251, 255], [39, 39, 39, 255]),
            );
            let cy = card.y + card.height as i32 / 2;
            v.rounded_asset(card.x + 14, cy - 8, 16, 16, 8, MUTED);
            v.rounded_asset(
                card.x + 21,
                cy - 1,
                2,
                5,
                1,
                fill(v, [255; 4], [39, 39, 39, 255]),
            );
            v.rounded_asset(
                card.x + 21,
                cy - 4,
                2,
                2,
                1,
                fill(v, [255; 4], [39, 39, 39, 255]),
            );
            wrap(
                v,
                card.x + 40,
                card.y + 10,
                card.width.saturating_sub(52),
                "Select a single file to get more information and share your cloud content.",
                3,
                TEXT,
            );
        }
        let label_w = label_w(r);
        let row_h = row_h();
        let value_x = x + label_w as i32 + 8;
        let value_w = (r.x + r.width as i32 - 20 - value_x).max(0) as u32;
        for (row, y, lines) in &l.rows {
            if row.is_section() {
                v.text_clip_centered(
                    x,
                    (y - shift, row_h as u32),
                    r.width.saturating_sub(40),
                    row.label,
                    TextStyle::Semibold14,
                    TEXT,
                );
                continue;
            }
            label(v, x, y - shift, label_w, row_h as u32, row.label, MUTED);
            let color = if row.muted { MUTED } else { TEXT };
            if *lines == 1 {
                label(
                    v,
                    value_x,
                    y - shift,
                    value_w,
                    row_h as u32,
                    &ui::display_ascii(&row.value),
                    color,
                );
            } else {
                let text = ui::display_ascii(&row.value);
                for (i, range) in door_workspace::dialog::lines(&text, value_w, |s| v.measure(s))
                    .into_iter()
                    .take(*lines)
                    .enumerate()
                {
                    label(
                        v,
                        value_x,
                        y - shift + i as i32 * row_h,
                        value_w,
                        row_h as u32,
                        &text[range],
                        color,
                    );
                }
            }
        }
        if let Some(y) = l.bar {
            // Windows' drive capacity bar: accent fill, red when nearly full.
            if let VolumeState::Known(u) = self.volume {
                let w = (r.width as i32 - 40).max(0) as u32;
                let track = fill(v, [222, 222, 222, 255], [68, 68, 68, 255]);
                v.rounded_asset(x, y + 2 - shift, w, 6, 3, track);
                let used = (u128::from(u.used_bytes()) * u128::from(w)
                    / u128::from(u.total_bytes.max(1))) as u32;
                let full = u.free_bytes * 10 < u.total_bytes;
                let bar = if full {
                    [28, 43, 196, 255]
                } else {
                    [212, 120, 0, 255]
                };
                v.rounded_asset(
                    x,
                    y + 2 - shift,
                    used.max(6).min(w),
                    6,
                    3,
                    fill(v, bar, bar),
                );
            }
        }
        if let Some(ty) = l.tags_y {
            label(
                v,
                x,
                ty - shift,
                r.width.saturating_sub(40),
                (24 + xl::grow()) as u32,
                "Tags",
                TEXT,
            );
            let chip_fill = fill(v, [243, 243, 243, 255], [45, 45, 45, 255]);
            let chip_border = fill(v, [229, 229, 229, 255], [62, 62, 62, 255]);
            let chip_over = fill(v, [234, 234, 234, 255], [55, 55, 55, 255]);
            for (b, tag) in &l.chips {
                let b = Bounds {
                    y: b.y - shift,
                    ..*b
                };
                let over = v.hover.is_some_and(|(px, py)| b.contains(px, py));
                v.rounded_asset(b.x, b.y, b.width, b.height, 14, chip_border);
                v.rounded_asset(
                    b.x + 1,
                    b.y + 1,
                    b.width - 2,
                    b.height - 2,
                    13,
                    if over { chip_over } else { chip_fill },
                );
                v.rounded_asset(
                    b.x + 12,
                    b.y + (b.height as i32 - 8) / 2,
                    8,
                    8,
                    4,
                    tag_color(v, *tag),
                );
                label(
                    v,
                    b.x + 26,
                    b.y,
                    b.width.saturating_sub(56),
                    b.height,
                    &ui::display_ascii(&self.tags.names[*tag]),
                    TEXT,
                );
                let close = Bounds {
                    x: b.x + b.width as i32 - 26,
                    y: b.y + (b.height as i32 - 20) / 2,
                    width: 20,
                    height: 20,
                };
                if v.hover.is_some_and(|(px, py)| close.contains(px, py)) {
                    plate(v, close, 10, [222, 222, 222, 255], [72, 72, 72, 255]);
                }
                v.tiny_icon(close.x + 4, close.y + 4, Icon::Close, MUTED);
                self.chip_hits
                    .borrow_mut()
                    .push((b, *tag, CHIP + *tag as u32));
            }
            if let Some(b) = l.add {
                let b = Bounds {
                    y: b.y - shift,
                    ..b
                };
                if self.editor == 6 {
                    v.rounded(b.x, b.y, b.width, b.height, 4, [214, 210, 204, 255]);
                    v.rounded(b.x + 1, b.y + 1, b.width - 2, b.height - 2, 3, [255; 4]);
                    self.draw_editor(v, b.x + 10, b.y + 4, b.width.saturating_sub(20));
                    v.rect(
                        b.x + 3,
                        b.y + b.height as i32 - 2,
                        b.width.saturating_sub(6),
                        2,
                        ACCENT,
                    );
                    label(
                        v,
                        b.x,
                        b.y + b.height as i32 + 2,
                        b.width,
                        (20 + xl::grow()) as u32,
                        "Enter adds the tag. Esc cancels.",
                        MUTED,
                    );
                } else {
                    let over = v.hover.is_some_and(|(px, py)| b.contains(px, py));
                    v.rounded_asset(b.x, b.y, b.width, b.height, 14, chip_border);
                    let face = if over {
                        chip_over
                    } else {
                        fill(v, [255; 4], [32, 32, 32, 255])
                    };
                    v.rounded_asset(b.x + 1, b.y + 1, b.width - 2, b.height - 2, 13, face);
                    v.tiny_icon(b.x + 12, b.y + (b.height as i32 - 12) / 2, Icon::Add, TEXT);
                    label(
                        v,
                        b.x + 30,
                        b.y,
                        b.width.saturating_sub(34),
                        b.height,
                        "Add tag",
                        TEXT,
                    );
                }
            }
        }
        if !l.actions.is_empty() {
            line(v, x, l.actions_y - shift, r.width.saturating_sub(40));
            label(
                v,
                x,
                l.actions_y + 8 - shift,
                r.width.saturating_sub(40),
                (26 + xl::grow()) as u32,
                "Quick actions",
                TEXT,
            );
        }
        for (action, b) in &l.actions {
            let b = Bounds {
                y: b.y - shift,
                ..*b
            };
            if action.enabled() {
                control(v, b, action.icon, &action.label, false);
                v.tiny_icon(
                    b.x + b.width as i32 - 18,
                    b.y + (b.height as i32 - 12) / 2,
                    Icon::ChevronRight,
                    MUTED,
                );
            } else {
                let disabled = if v.palette.dark {
                    [110, 110, 110, 255]
                } else {
                    [160, 160, 160, 255]
                };
                v.mask_pixels(
                    b.x + 10,
                    b.y + 10,
                    16,
                    action.icon.mask_scaled(16, v.scale),
                    disabled,
                    None,
                );
                let g = xl::grow();
                label(
                    v,
                    b.x + 34,
                    b.y + 2,
                    b.width.saturating_sub(40),
                    (24 + g) as u32,
                    &action.label,
                    disabled,
                );
                let reason_h = door_workspace::palette::text_grow(20, TextStyle::Regular12);
                v.small_clip(
                    b.x + 34,
                    v.centered_text_top(b.y + 26 + g, reason_h, TextStyle::Regular12),
                    b.width.saturating_sub(44),
                    action.reason,
                    MUTED,
                );
            }
        }
        if let Some(d) = self.detail.as_ref().filter(|d| !d.status.is_empty()) {
            wrap(
                v,
                x,
                l.status_y - shift,
                r.width.saturating_sub(40),
                &d.status,
                4,
                MUTED,
            );
        }
    }
    /// Tags tab: every catalog tag as a toggle for the inspected item, then
    /// "Create tag". Geometry is `TagGrid`, shared with hit testing.
    pub(super) fn draw_user_tags(&self, v: &mut ui::View<'_>, r: Bounds, shift: i32) {
        if !self.tags_error.is_empty() {
            wrap(
                v,
                r.x + 22,
                r.y + 240 - shift,
                r.width.saturating_sub(44),
                &self.tags_error,
                8,
                MUTED,
            );
            return;
        }
        let tags = self.detail.as_ref().map_or(0, |d| d.tags);
        let grid = TagGrid::new(r.width, self.tags.names.len());
        self.detail_height.set(grid.content_height());
        for (i, name) in self.tags.names.iter().enumerate() {
            let Some(cell) = grid.cell(i) else { continue };
            let b = Bounds {
                x: r.x + cell.x,
                y: r.y + body() + cell.y - shift,
                ..cell
            };
            let active = tags & (1 << i) != 0;
            let hover = v.hover.is_some_and(|(px, py)| b.contains(px, py));
            let border = if active {
                fill(v, ACCENT, ACCENT)
            } else {
                fill(v, [229, 229, 229, 255], [62, 62, 62, 255])
            };
            v.rounded_asset(b.x, b.y, b.width, b.height, 6, border);
            let (light, dark_face) = match (active, hover) {
                (true, _) => ([250, 239, 226, 255], [70, 52, 38, 255]),
                (false, true) => ([243, 243, 243, 255], [50, 50, 50, 255]),
                (false, false) => ([251, 251, 251, 255], [38, 38, 38, 255]),
            };
            let face = fill(v, light, dark_face);
            v.rounded_asset(b.x + 1, b.y + 1, b.width - 2, b.height - 2, 5, face);
            v.rounded_asset(
                b.x + 12,
                b.y + (b.height as i32 - 8) / 2,
                8,
                8,
                4,
                tag_color(v, i),
            );
            label(
                v,
                b.x + 30,
                b.y,
                b.width.saturating_sub(58),
                b.height,
                &ui::display_ascii(name),
                TEXT,
            );
            if active {
                // Check mark: assigned to the inspected item.
                v.mask_pixels(
                    b.x + b.width as i32 - 24,
                    b.y + (b.height as i32 - 16) / 2,
                    16,
                    Icon::Check.mask_scaled(16, v.scale),
                    ACCENT,
                    None,
                );
            }
            self.chip_hits.borrow_mut().push((b, i, 920 + i as u32));
        }
        let y = r.y + body() + grid.create().y - shift;
        if self.editor == 4 {
            v.rounded(
                r.x + 20,
                y,
                r.width.saturating_sub(40),
                34,
                5,
                [214, 210, 204, 255],
            );
            v.rounded(r.x + 21, y + 1, r.width.saturating_sub(42), 32, 4, [255; 4]);
            self.draw_editor(v, r.x + 30, y + 7, r.width.saturating_sub(60));
            v.rect(r.x + 24, y + 32, r.width.saturating_sub(48), 2, ACCENT);
            wrap(
                v,
                r.x + 22,
                y + 44,
                r.width.saturating_sub(44),
                "Enter to save the tag. Esc to cancel.",
                3,
                MUTED,
            );
        } else {
            control(
                v,
                Bounds {
                    x: r.x + 20,
                    y,
                    width: r.width.saturating_sub(40),
                    height: 34,
                },
                Icon::Add,
                "Create tag",
                false,
            );
            wrap(
                v,
                r.x + 22,
                y + 48,
                r.width.saturating_sub(44),
                if self.tags.names.is_empty() {
                    "Create a name relevant to your files. No tags are added automatically."
                } else if self.detail.is_some() {
                    "Select a tag to add it to or remove it from this item. Right-click a tag in the navigation pane to rename or delete it."
                } else {
                    "Select a file or folder to assign tags. Right-click a tag in the navigation pane to rename or delete it."
                },
                4,
                MUTED,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sizes_read_like_windows_properties() {
        assert_eq!(grouped(1_234_567), "1,234,567");
        assert_eq!(grouped(12), "12");
        assert_eq!(size_text(1), "1 byte");
        assert_eq!(size_text(1023), "1,023 bytes");
        assert_eq!(size_text(1536), "1.5 KB (1,536 bytes)");
    }
}

#[cfg(test)]
mod device_name_tests {
    use super::*;
    #[test]
    fn this_pc_details_show_the_device_name() {
        let mut f = Files::empty("/users/owner".into(), Controls::default());
        f.configure(1460, 900);
        f.overview = true;
        let rows = f.summary_rows();
        assert_eq!(rows[0].label, "Device name");
        assert_eq!(rows[0].value, "DOORS-PC", "absent means today's name");
        f.device_name = door_service_protocol::device::Name::parse("Bob-Laptop").unwrap();
        assert_eq!(f.summary_rows()[0].value, "Bob-Laptop");
        // Folders other than This PC do not show it.
        f.overview = false;
        assert!(f.summary_rows().iter().all(|r| r.label != "Device name"));
    }
}
