//! Modern Explorer interaction, per-tab state and incremental read-only inspection.
use super::*;
use door_service_client::{File, FileInfo};
use door_workspace::explorer_layout::{SIDEBAR, content_top};
// `SIDEBAR` is the default pane width; the live width is `layout.sidebar`.
use door_workspace::explorer_tree::{self, Node as TreeNode, Row as TreeRow, Tree};
#[path = "modern/paint.rs"]
pub(super) mod paint;

#[derive(Clone)]
pub(super) struct Tab {
    id: u64,
    cwd: String,
    entries: Vec<DirEntry>,
    selected: Option<usize>,
    offset: usize,
    history: Vec<String>,
    history_index: usize,
    search: String,
    overview: bool,
    /// The tab shows Home; `paths`/`sections` describe its items.
    home: bool,
    paths: Vec<String>,
    sections: Vec<u8>,
    descending: bool,
    sort_key: SortKey,
    /// Details column widths (Name, Date modified, Type, Size); per tab,
    /// inherited by new tabs.
    columns: [u32; 4],
    view: ViewMode,
    overview_view: ViewMode,
    overview_selected: usize,
    tag_filter: Option<usize>,
    details_visible: bool,
    detail_tab: u8,
}
pub(super) struct Detail {
    pub(super) path: String,
    pub(super) name: String,
    pub(super) info: FileInfo,
    /// Volume timestamps (0 = not recorded).
    pub(super) times: door_service_client::FileTimes,
    pub(super) kind: Kind,
    pub(super) tags: u32,
    pub(super) children: Vec<DirEntry>,
    pub(super) cookie: u64,
    pub(super) children_done: bool,
    pub(super) truncated: bool,
    pub(super) status: String,
    /// The open folder itself, shown while nothing is selected.
    pub(super) current: bool,
}
pub(super) struct PreviewJob {
    file: File,
    /// `Files::nav_generation` when the job was opened; any location or tab
    /// change makes the job stale even when the same path is visible again.
    generation: u64,
    path: String,
    info: FileInfo,
    kind: Kind,
    bytes: Vec<u8>,
    limit: usize,
    /// Audio, video and archives: ranged reads instead of a prefix, and
    /// the range being fetched.
    media: Option<door_workspace::media_preview::Reader>,
    fetch: Option<(u64, u64)>,
}
/// Incremental recursive size of one inspected folder. Listing is stateless
/// RPC, so dropping the job cancels it without leaking a handle.
pub(super) struct SizeJob {
    scan: door_workspace::explorer_size::Scan,
    root: FileInfo,
    generation: u64,
    /// Directory being listed: (path, depth, last cookie, children so far).
    current: Option<(
        String,
        usize,
        u64,
        Vec<door_workspace::explorer_size::Child>,
    )>,
}
/// Listing calls (one entry each) a size job may spend per event-loop tick.
const SIZE_RPCS_PER_TICK: usize = 48;
/// Remembered folder totals (the inspected folder and, with Folder sizes,
/// the visible Details rows).
const SIZE_CACHE: usize = 64;
/// Entries read from one directory before its listing is cut short.
const SIZE_DIRECTORY_LIMIT: usize = 4096;
pub(super) struct CachedPreview {
    pub(super) info: FileInfo,
    pub(super) image: Option<previewing::Image>,
    pub(super) text: String,
    pub(super) status: String,
    /// Audio, video and archive previews (`media_preview`), with the
    /// metadata facts shown under the picture.
    pub(super) media: Option<alloc::boxed::Box<MediaPreview>>,
}
pub(super) struct MediaPreview {
    pub(super) preview: door_workspace::media_preview::Preview,
    pub(super) report: door_workspace::file_metadata::Report,
}
/// What one folder holds, for its content-aware icon (Windows 11): empty
/// folders keep the plain folder, folders with files show paper sheets
/// (`Icon::FolderFilled`), and in Medium and Large icons a folder holding an
/// image shows that image's thumbnail between `FolderBack` and
/// `FolderFront`. Cached per folder across navigation, keyed by the
/// folder's identity and last-write time; adding or removing an item
/// re-dates the folder and notifies its parent's storage watch, whose
/// re-listing drops the stale entry (`prune_folders`).
pub(super) struct CachedFolder {
    pub(super) info: FileInfo,
    pub(super) modified: u64,
    pub(super) path: String,
    pub(super) cookie: u64,
    /// Items listed so far (up to `FOLDER_SCAN`).
    pub(super) entries: usize,
    /// The first image listed: its path and identity.
    pub(super) image: Option<(String, FileInfo)>,
    pub(super) done: bool,
}
impl CachedFolder {
    pub(super) fn of(&self, entry: &DirEntry) -> bool {
        self.info == entry.info && self.modified == entry.times.modified
    }
}
/// How a folder's icon shows its contents (`Files::folder_look`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FolderLook {
    Plain,
    /// Paper sheets peeking out (`Icon::FolderFilled`).
    Sheets,
    /// The cached preview of this image, framed between the plates.
    Thumbnail(FileInfo),
}
/// Folders whose contents are remembered (the oldest is dropped first).
const FOLDER_CACHE: usize = 64;
/// Items a folder scan reads looking for an image before settling on sheets.
const FOLDER_SCAN: usize = 64;
/// Listing calls one folder scan step may spend.
const FOLDER_RPCS_PER_STEP: usize = 16;
/// Rail tags: the "Your tags" heading band and the tag row pitch hold one
/// 14 px label each (42 and 28 px at 100%) and grow with the text size.
pub(super) fn tag_head() -> i32 {
    42 + xl::grow()
}
pub(super) fn tag_pitch() -> i32 {
    28 + xl::grow()
}
impl Files {
    pub fn tab_identity(&self, index: usize) -> Option<u64> {
        self.tabs.get(index).map(|t| t.id).filter(|id| *id != 0)
    }
    pub fn tab_index(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id && id != 0)
    }
    pub fn dragged_tab(&self, x: i32, y: i32) -> Option<usize> {
        if self.modal() || self.editor != 0 || self.xmenu.is_some() {
            return None;
        }
        self.visible_tabs().find(|&i| {
            let r = self.tab_bounds(i);
            r.contains(x, y) && x < r.x + r.width as i32 - 32
        })
    }
    pub fn window_empty(&self) -> bool {
        self.tabs.is_empty()
    }
    fn restore_tab_state(&mut self, index: usize) {
        if let Some(t) = self.tabs.get(index).cloned() {
            self.cwd = t.cwd;
            self.entries = t.entries;
            self.selected = t.selected;
            self.offset = t.offset;
            self.history = t.history;
            self.history_index = t.history_index;
            self.search = t.search;
            self.overview = t.overview;
            self.home_view = t.home;
            self.home_sections = t.sections;
            self.result_paths = t.paths;
            self.home_scroll = 0;
            self.descending = t.descending;
            self.sort_key = t.sort_key;
            self.columns = t.columns;
            self.view = t.view;
            self.overview_view = t.overview_view;
            self.overview_selected = t.overview_selected;
            self.tag_filter = t.tag_filter;
            self.details_visible = t.details_visible;
            self.detail_tab = t.detail_tab;
            self.active_tab = index;
            self.reveal_tab();
            self.editor = 0;
            self.detail = None;
            self.preview_cache.clear();
            self.folders_scanned = false;
            self.thumbnail_scanned = false;
            self.last_click = None;
            self.last_click_bounds = None;
            self.last_click_point = None;
            self.detail_scroll = 0;
            self.rename = None;
            if !self.home_view {
                self.result_paths.clear();
                if self.volume_search() {
                    self.apply_results();
                }
            }
            // Results computed for the previous tab must not land here; the
            // next `preview_step` closes the stale job's handle.
            self.nav_generation = self.nav_generation.wrapping_add(1);
            self.dismiss_menu();
        }
    }
    pub fn transfer_to(
        &mut self,
        target: &mut Self,
        index: usize,
        replace_blank: bool,
        storage: &mut Storage,
    ) -> Result<bool, Error> {
        if index >= self.tab_count()
            || (!replace_blank && target.tab_count() >= 8)
            || self.modal()
            || target.modal()
            || self.editor != 0
            || target.editor != 0
        {
            return Ok(false);
        }
        target
            .tabs
            .try_reserve(1)
            .map_err(|_| Error::Service(Code::Resources))?;
        self.save_tab();
        target.save_tab();
        self.cancel_preview(storage);
        target.cancel_preview(storage);
        let tab = self.tabs.remove(index);
        if replace_blank {
            target.tabs.clear();
        }
        target.tabs.push(tab);
        let active = target.tabs.len() - 1;
        target.restore_tab_state(active);
        if !self.tabs.is_empty() {
            self.restore_tab_state(
                self.active_tab
                    .saturating_sub(usize::from(index < self.active_tab))
                    .min(self.tabs.len() - 1),
            );
        }
        if target.details_visible
            && let Err(e) = target.inspect_selection(storage)
        {
            target.message = format!("Tab moved; details could not refresh: {e:?}");
        }
        Ok(true)
    }
    pub fn reorder(&mut self, from: usize, to: usize) -> bool {
        if from >= self.tab_count()
            || to >= self.tab_count()
            || from == to
            || self.editor != 0
            || self.modal()
        {
            return false;
        }
        self.save_tab();
        let t = self.tabs.remove(from);
        self.tabs.insert(to, t);
        self.restore_tab_state(to);
        true
    }

    /// Pinned folder paths in rail order (labels come from `FOLDERS`).
    /// Quick access in rail order: known folders, then the user's pins.
    pub(super) fn rail_pins(&self) -> (Vec<String>, Vec<String>) {
        self.quick_access()
            .into_iter()
            .map(|(path, label, _)| (path, label))
            .unzip()
    }
    pub(super) fn rail_drives(&self) -> Vec<(&'static str, &'static str)> {
        self.drives().iter().map(|d| (d.label, d.path)).collect()
    }
    /// Drops every tree listing and re-lists the expanded rows, level by
    /// level, so a refresh shows current subfolders without duplicates.
    pub(super) fn relist_tree(&mut self, storage: &mut Storage) {
        self.tree.invalidate();
        let (pins, _) = self.rail_pins();
        let drives = self.rail_drives();
        for _ in 0..8 {
            let missing = self.tree.missing(&self.rail_rows(), &pins, &drives);
            if missing.is_empty() {
                break;
            }
            for path in missing {
                self.list_rail_children(storage, &path);
            }
        }
    }
    pub(super) fn rail_rows(&self) -> Vec<TreeRow> {
        let (pins, labels) = self.rail_pins();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        self.tree.rows(&pins, &labels, &self.rail_drives())
    }
    /// Content-space top of the first tree row (scrolled by `rail_offset`).
    pub(super) fn rail_top() -> i32 {
        content_top() + 8
    }
    /// Content-space bounds of each tree row.
    pub(super) fn rail_row_bounds(&self, rows: &[TreeRow]) -> Vec<Bounds> {
        Tree::offsets(rows)
            .into_iter()
            .zip(rows)
            .map(|(top, row)| Bounds {
                x: 2,
                y: Self::rail_top() + top,
                width: self.layout.sidebar as u32 - 6,
                height: if row.node == TreeNode::Separator {
                    row.pitch() as u32
                } else {
                    explorer_tree::row_height()
                },
            })
            .collect()
    }
    /// Disclosure chevron box of a row (12 px glyph, padded hit area).
    pub(super) fn rail_chevron(row: &TreeRow, r: Bounds) -> Bounds {
        // The chevron's centre sits 17 px (25 px at 150 %) from the pane's
        // edge, one indent further per level.
        Bounds {
            x: 6 + i32::from(row.depth) * explorer_tree::INDENT,
            y: r.y,
            width: 22,
            height: r.height,
        }
    }
    /// Tree hit test in content space: 1100+row opens, 1400+row toggles.
    pub(super) fn rail_target(&self, x: i32, y: i32) -> Option<u32> {
        let rows = self.rail_rows();
        let bounds = self.rail_row_bounds(&rows);
        let (i, (row, r)) = rows
            .iter()
            .zip(&bounds)
            .enumerate()
            .find(|(_, (_, r))| r.contains(x, y))?;
        if row.node == TreeNode::Separator {
            return None;
        }
        if row.expandable && Self::rail_chevron(row, *r).contains(x, y) {
            return Some(1400 + i as u32);
        }
        Some(1100 + i as u32)
    }
    /// The highlighted tree row: the clicked row while it still describes the
    /// location, else the first row that does.
    pub(super) fn rail_selected(&self, rows: &[TreeRow]) -> Option<usize> {
        let (pins, _) = self.rail_pins();
        let drives = self.rail_drives();
        let here = |row: &TreeRow| match row.node {
            TreeNode::Home => self.home_view,
            TreeNode::ThisPc => self.overview,
            TreeNode::Network | TreeNode::Separator => false,
            TreeNode::RecycleBin => self.recycle_view(),
            _ => {
                !self.overview
                    && !self.home_view
                    && row.path(&pins, &drives) == Some(self.cwd.as_str())
            }
        };
        self.rail_key
            .as_ref()
            .and_then(|key| rows.iter().position(|r| &r.key == key && here(r)))
            .or_else(|| rows.iter().position(here))
    }
    pub(super) fn open_rail_row(
        &mut self,
        storage: &mut Storage,
        index: usize,
    ) -> Result<(), Error> {
        let rows = self.rail_rows();
        let Some(row) = rows.get(index) else {
            return Ok(());
        };
        let (pins, _) = self.rail_pins();
        let drives = self.rail_drives();
        match row.node {
            TreeNode::Home => self.go_home(storage)?,
            TreeNode::ThisPc => self.go_this_pc(storage)?,
            TreeNode::RecycleBin => self.go_recycle_bin(storage)?,
            TreeNode::Network => {
                self.message = "No network locations: this device has no network provider.".into();
                return Ok(());
            }
            TreeNode::Linux => {
                self.message = "No Linux distributions are installed.".into();
                return Ok(());
            }
            TreeNode::Separator => return Ok(()),
            _ => {
                let Some(path) = row.path(&pins, &drives).map(String::from) else {
                    return Ok(());
                };
                self.navigate(storage, path)?;
            }
        }
        self.rail_key = Some(row.key.clone());
        let _ = door_sys::write(
            format!(
                "DOOR_P12 FILES RAIL open key={} path={}\n",
                row.key,
                if self.overview { THIS_PC } else { &self.cwd }
            )
            .as_bytes(),
        );
        Ok(())
    }
    /// Expands or collapses one row; never navigates. Opening lists the
    /// row's subfolders (replacing any earlier listing).
    pub(super) fn toggle_rail_row(&mut self, storage: &mut Storage, index: usize) {
        let rows = self.rail_rows();
        let Some(row) = rows.get(index) else {
            return;
        };
        let (pins, _) = self.rail_pins();
        let drives = self.rail_drives();
        let key = row.key.clone();
        if let Some(path) = self.tree.toggle(row, &pins, &drives) {
            self.list_rail_children(storage, &path);
        }
        self.rail_offset = self.rail_offset.min(self.rail_scroll_max() as i32);
        self.remember_sections();
        let rows = self.rail_rows();
        let children = rows
            .iter()
            .filter(|r| r.key.starts_with(&format!("{key}/")))
            .count();
        let _ = door_sys::write(
            format!(
                "DOOR_P12 FILES RAIL toggle key={key} open={} children={children}\n",
                u8::from(self.tree.is_expanded(&key))
            )
            .as_bytes(),
        );
    }
    /// Lists subfolders for the tree; errors leave an honest empty state.
    pub(super) fn list_rail_children(&mut self, storage: &mut Storage, path: &str) {
        let listed = Self::read_entries(storage, path)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| e.info.directory && Self::visible(e))
                    .filter_map(|e| {
                        let name = String::from_utf16(&e.name[..e.name_len]).ok()?;
                        let child = if path == "/compat" && name == "Users" {
                            "/users".into()
                        } else if path == "/" {
                            format!("/{name}")
                        } else {
                            format!("{path}/{name}")
                        };
                        Some((name, child))
                    })
                    .collect()
            })
            .map_err(|_| ());
        self.tree.set_children(path, listed);
    }
    pub(super) fn rail_tag_top(&self) -> i32 {
        Self::rail_top() + Tree::height(&self.rail_rows()) + 10
    }
    pub(super) fn rail_scroll_max(&self) -> usize {
        let bottom = self.rail_tag_top()
            + tag_head()
            + self.tags.names.len() as i32 * tag_pitch()
            + 34
            + xl::grow();
        (bottom - (self.layout.height as i32 - xl::status_height() as i32)).max(0) as usize
    }
    pub(super) fn rail_scrollbar(&self) -> Option<(Bounds, Bounds)> {
        let max = self.rail_scroll_max();
        if max == 0 {
            return None;
        }
        let track = Bounds {
            x: self.layout.sidebar - 12,
            y: content_top() + 8,
            width: 10,
            height: self.layout.height.saturating_sub(content_top() as u32 + 40),
        };
        if track.height < 24 {
            return None;
        }
        let size = (track.height as usize * track.height as usize / (track.height as usize + max))
            .max(24)
            .min(track.height as usize) as u32;
        let thumb = Bounds {
            x: self.layout.sidebar - 12,
            y: track.y
                + ((track.height - size) as usize * self.rail_offset.max(0) as usize / max) as i32,
            width: 10,
            height: size,
        };
        Some((track, thumb))
    }
    pub(super) fn pane_scroll_max(&self, r: Bounds) -> usize {
        let visible = self
            .layout
            .height
            .saturating_sub(r.y as u32 + details::body() as u32 + xl::status_height())
            as usize;
        if self.detail_tab == 2 {
            return door_workspace::explorer_tags::TagGrid::new(r.width, self.tags.names.len())
                .content_height()
                .saturating_sub(visible)
                .div_ceil(24);
        }
        if self.detail_tab == 0 {
            // The measured Details layout height, recorded by its painter.
            return self.pane_scroll_rows().saturating_sub(visible).div_ceil(24);
        }
        let Some(d) = self.detail.as_ref() else {
            return 0;
        };
        let content = match self.detail_tab {
            _ if d.info.directory => d.children.len() * 34 + 30,
            _ => self.cached(d.info).map_or(120, |p| {
                if p.image.is_some() {
                    270
                } else {
                    let columns = (r.width.saturating_sub(56) / 14).max(1) as usize;
                    // A conservative bound keeps every source byte reachable, even
                    // before the first measured wrapping pass at this width.
                    (p.text.len().div_ceil(columns) + p.text.lines().count() + 1) * 19 + 60
                }
            }),
        };
        content.saturating_sub(visible).div_ceil(24)
    }
    pub(super) fn pane_scrollbar(&self, r: Bounds) -> Option<(Bounds, Bounds)> {
        let maximum = self.pane_scroll_max(r);
        if maximum == 0 {
            return None;
        }
        let track = Bounds {
            x: r.x + r.width as i32 - 10,
            y: r.y + details::body(),
            width: 10,
            height: self
                .layout
                .height
                .saturating_sub(r.y as u32 + details::body() as u32 + xl::status_height() + 4),
        };
        if track.height < 20 {
            return None;
        }
        let thumb_height = (track.height as usize * track.height as usize
            / (track.height as usize + maximum * 24))
            .max(24)
            .min(track.height as usize) as u32;
        let thumb = Bounds {
            x: track.x,
            y: track.y
                + ((track.height - thumb_height) as usize * self.detail_scroll.min(maximum)
                    / maximum) as i32,
            width: 10,
            height: thumb_height,
        };
        Some((track, thumb))
    }
    pub(super) fn scroll_pane(&mut self, forward: bool) {
        let max = self.pane().map_or(0, |r| self.pane_scroll_max(r));
        let page = self.pane().map_or(5, |r| {
            (r.height.saturating_sub(details::body() as u32) / 24).max(1) as usize
        });
        self.detail_scroll = if forward {
            (self.detail_scroll + page).min(max)
        } else {
            self.detail_scroll.saturating_sub(page)
        };
    }
    pub fn pointer_motion(&mut self, x: i32, y: i32) -> bool {
        if let Some(grab) = self.sidebar_drag {
            // Live reflow: every band and label follows the new width.
            let next = self.layout.clamp_sidebar(x - grab);
            let changed = next != self.layout.sidebar;
            self.layout.sidebar = next;
            if changed {
                self.thumbnail_scanned = false;
                self.folders_scanned = false;
                self.dismiss_menu();
            }
            return changed;
        }
        if let Some((column, start, width)) = self.column_drag {
            let next = (width as i32 + x - start).clamp(48, 640) as u32;
            let changed = self.columns[column] != next;
            self.columns[column] = next;
            return changed;
        }
        if let Some(grab) = self.rail_drag
            && let Some((track, thumb)) = self.rail_scrollbar()
        {
            let space = track.height.saturating_sub(thumb.height).max(1) as usize;
            let at = (y - track.y - grab).max(0) as usize;
            let next = (at.min(space) * self.rail_scroll_max() / space) as i32;
            let changed = self.rail_offset != next;
            self.rail_offset = next;
            return changed;
        }
        let Some(grab) = self.pane_drag else {
            return false;
        };
        let Some(r) = self.pane() else {
            return false;
        };
        let Some((track, thumb)) = self.pane_scrollbar(r) else {
            return false;
        };
        let space = track.height.saturating_sub(thumb.height).max(1) as usize;
        let at = (y - track.y - grab).max(0) as usize;
        let next = (at.min(space) * self.pane_scroll_max(r) / space).min(self.pane_scroll_max(r));
        let changed = next != self.detail_scroll;
        self.detail_scroll = next;
        changed
    }
    /// Mouse wheel (`notches > 0` scrolls up): a flyout under the pointer,
    /// the details pane, the navigation pane or the item view. Items move by
    /// whole rows (about three lines of text per notch, as Windows does), List
    /// view by columns; the panes by 48 px per notch.
    pub fn wheel(&mut self, x: i32, y: i32, notches: i32) -> bool {
        if notches != 0 && self.picker_open() {
            return self.picker_wheel(notches);
        }
        if notches == 0 || self.modal() {
            return false;
        }
        if let Some(m) = self.xmenu.as_mut() {
            return m.root().bounds.contains(x, y) && m.wheel(notches);
        }
        if let Some(r) = self.pane()
            && r.contains(x, y)
        {
            let max = self.pane_scroll_max(r);
            let before = self.detail_scroll;
            let step = notches.unsigned_abs() as usize * 2;
            self.detail_scroll = if notches > 0 {
                self.detail_scroll.saturating_sub(step)
            } else {
                (self.detail_scroll + step).min(max)
            };
            return before != self.detail_scroll;
        }
        if y < content_top() || y >= self.layout.height as i32 - xl::status_height() as i32 {
            return false;
        }
        if x < self.layout.sidebar {
            let before = self.rail_offset;
            self.rail_offset =
                (self.rail_offset - notches * 48).clamp(0, self.rail_scroll_max() as i32);
            return before != self.rail_offset;
        }
        if self.home_view {
            let before = self.home_scroll;
            self.home_scroll = (self.home_scroll - notches * 48).clamp(0, self.home_scroll_max());
            return before != self.home_scroll;
        }
        let layout = self.content_layout();
        let mode = self.view_mode();
        let step = layout.step(mode).max(1);
        let rows = match mode.flow() {
            // A notch is three text lines: 3 Details rows, one icon row.
            door_workspace::explorer_layout::Flow::Rows => {
                (96 / layout.cell_bounds(0, mode).height.max(1) as usize).max(1)
            }
            _ => 1,
        };
        let before = self.offset;
        let delta = rows * step * notches.unsigned_abs() as usize;
        self.offset = if notches > 0 {
            self.offset.saturating_sub(delta)
        } else {
            (self.offset + delta).min(self.max_offset())
        };
        if before != self.offset {
            self.thumbnail_scanned = false;
            self.folders_scanned = false;
            self.last_click = None;
            self.last_click_bounds = None;
            return true;
        }
        false
    }
    /// The east-west resize pointer: over the navigation pane splitter, and
    /// throughout a drag of it.
    pub fn splitter_cursor(&self, x: i32, y: i32) -> bool {
        self.sidebar_drag.is_some()
            || (!self.modal() && self.xmenu.is_none() && self.layout.on_splitter(x, y))
    }
    pub fn pointer_up(&mut self) {
        self.pane_drag = None;
        self.rail_drag = None;
        self.column_drag = None;
        if self.sidebar_drag.take().is_some()
            && self.prefs.sidebar.unwrap_or(SIDEBAR) != self.layout.sidebar
        {
            self.prefs.sidebar = Some(self.layout.sidebar);
            self.prefs_dirty = true;
        }
    }
    /// Column widths that fit `available` px: the user's widths while they
    /// fit; otherwise Name gives way first (down to its minimum), then Date
    /// modified, Type and Size shrink in proportion to what each can give,
    /// and a list narrower than all minimums scales the minimums.
    pub(super) fn fitted_columns(&self, available: u32) -> [u32; 4] {
        // The four minimums still leave every column at least 48 px in the
        // narrowest default list (717 px window with the details pane).
        const MIN: [u32; 4] = [80, 80, 72, 64];
        let want = self.columns;
        let total: u32 = want.iter().sum();
        if total <= available {
            return want;
        }
        let floor = |i: usize| MIN[i].min(want[i]);
        let mut w = want;
        let mut excess = total - available;
        let give = excess.min(w[0] - floor(0));
        w[0] -= give;
        excess -= give;
        let spare = [w[1] - floor(1), w[2] - floor(2), w[3] - floor(3)];
        let pool: u32 = spare.iter().sum();
        if excess > 0 && pool > 0 {
            let cut = excess.min(pool);
            let mut taken = 0;
            for (i, give) in spare.iter().enumerate() {
                let share = if i == 2 {
                    cut - taken
                } else {
                    (u64::from(cut) * u64::from(*give) / u64::from(pool)) as u32
                };
                w[i + 1] -= share;
                taken += share;
            }
            excess -= cut;
        }
        if excess > 0 {
            let sum: u32 = w.iter().sum();
            let mut used = 0;
            for (i, width) in w.iter_mut().enumerate() {
                *width = if i == 3 {
                    available - used
                } else {
                    (u64::from(*width) * u64::from(available) / u64::from(sum.max(1))) as u32
                };
                used += *width;
            }
        }
        w
    }
    /// Details column headers (Name, Date modified, Type, Size), fitted to
    /// the list width.
    pub(super) fn detail_columns(&self, layout: ExplorerLayout) -> [(SortKey, Bounds); 4] {
        let right = layout.width as i32 - 12;
        let mut x = self.layout.sidebar + 12;
        let widths = self.fitted_columns((right - x).max(0) as u32);
        SortKey::ALL.map(|key| {
            let width = widths[key as usize] as i32;
            let r = Bounds {
                x,
                y: content_top(),
                width: width.min((right - x).max(0)) as u32,
                height: xl::details_header(),
            };
            x += width;
            (key, r)
        })
    }
    /// Header hit test: 960+column sorts, 965+column grabs its resize edge.
    pub(super) fn column_target(&self, x: i32, y: i32) -> Option<u32> {
        if self.view_mode() != ViewMode::Details
            || !(content_top()..content_top() + xl::details_header() as i32).contains(&y)
        {
            return None;
        }
        let columns = self.detail_columns(self.content_layout());
        for (i, (_, r)) in columns.iter().enumerate() {
            let edge = r.x + r.width as i32;
            if r.width > 0 && (edge - 4..edge + 4).contains(&x) {
                return Some(965 + i as u32);
            }
        }
        columns
            .iter()
            .position(|(_, r)| r.contains(x, y))
            .map(|i| 960 + i as u32)
    }
    /// Clicking a header sorts by it; clicking the active one reverses it.
    pub(super) fn sort_by_column(&mut self, column: usize) {
        let Some(key) = SortKey::ALL.get(column).copied() else {
            return;
        };
        if self.sort_key == key {
            self.descending = !self.descending;
        } else {
            self.sort_key = key;
            self.descending = false;
        }
        if !self.overview {
            self.sort();
        }
        self.remember_view();
        if let Some(position) = self.selected_position() {
            self.reveal_position(position);
        }
    }
    /// Visible item cell for a slot; This PC drops its cells below the
    /// "Devices and drives" group header. Paint and hit testing share it.
    /// This PC "Devices and drives" group header; a click collapses the group
    /// (remembered with the other sections).
    pub(super) fn drives_header(&self) -> Bounds {
        Bounds {
            x: self.layout.sidebar + 12,
            y: content_top() + 4,
            width: self
                .content_layout()
                .width
                .saturating_sub(self.layout.sidebar as u32 + 24),
            height: (24 + xl::grow()) as u32,
        }
    }
    pub(super) fn item_cell(&self, slot: usize) -> Option<Bounds> {
        if self.overview && self.search.is_empty() && !self.drives_expanded {
            return None;
        }
        let layout = self.content_layout();
        let mode = self.view_mode();
        let mut r = layout.cell_bounds(slot, mode);
        if self.overview && self.search.is_empty() {
            r.y += (content_top() + 34 + xl::grow() - layout.view_top(mode)).max(0);
        }
        (slot < layout.capacity(mode)
            && r.y + r.height as i32 <= layout.height as i32 - xl::status_height() as i32)
            .then_some(r)
    }
    pub fn tab_count(&self) -> usize {
        self.tabs.len().max(1)
    }
    pub(super) fn content_layout(&self) -> ExplorerLayout {
        ExplorerLayout {
            width: self.pane().map_or(self.layout.width, |r| r.x as u32),
            height: self.layout.height,
            sidebar: self.layout.sidebar,
        }
    }
    pub(super) fn pane(&self) -> Option<Bounds> {
        if !self.details_visible || self.layout.width < 560 || self.layout.height < 240 {
            return None;
        }
        let width = (self.layout.width / 4).clamp(280, 360).min(
            self.layout
                .width
                .saturating_sub(self.layout.sidebar as u32 + 88),
        );
        Some(Bounds {
            x: self.layout.width.saturating_sub(width) as i32,
            y: content_top(),
            width,
            height: self
                .layout
                .height
                .saturating_sub(content_top() as u32 + xl::status_height()),
        })
    }
    fn tab_capacity(&self) -> usize {
        let room = self.layout.width.saturating_sub(186);
        let basic = (room / 120).max(1) as usize;
        if self.tab_count() > basic {
            (room.saturating_sub(56) / 120).max(1) as usize
        } else {
            basic.min(self.tab_count())
        }
    }
    pub(super) fn visible_tabs(&self) -> core::ops::Range<usize> {
        if self.layout.width < 360 {
            return 0..0;
        }
        let capacity = self.tab_capacity();
        let start = self
            .tab_scroll
            .min(self.tab_count().saturating_sub(capacity));
        start..(start + capacity).min(self.tab_count())
    }
    pub(super) fn reveal_tab(&mut self) {
        let capacity = self.tab_capacity();
        self.tab_scroll = self.tab_scroll.min(self.active_tab);
        if self.active_tab >= self.tab_scroll + capacity {
            self.tab_scroll = self.active_tab + 1 - capacity;
        }
        self.tab_scroll = self
            .tab_scroll
            .min(self.tab_count().saturating_sub(capacity));
    }
    pub(super) fn tab_bounds(&self, index: usize) -> Bounds {
        let range = self.visible_tabs();
        let extra = if range.len() < self.tab_count() {
            56
        } else {
            0
        };
        let width =
            (self.layout.width.saturating_sub(186 + extra) / range.len().max(1) as u32).min(294);
        Bounds {
            x: 8 + (index as i32 - range.start as i32) * width as i32,
            y: 3,
            width: width.saturating_sub(4),
            height: 38,
        }
    }
    pub(super) fn caption_controls(&self) -> Vec<(Bounds, u32)> {
        let range = self.visible_tabs();
        if range.is_empty() {
            return Vec::new();
        }
        let last = self.tab_bounds(range.end - 1);
        let mut x = last.x + last.width as i32 + 6;
        let mut out = Vec::new();
        if range.len() < self.tab_count() {
            for id in [897, 898] {
                out.push((
                    Bounds {
                        x,
                        y: 3,
                        width: 24,
                        height: 38,
                    },
                    id,
                ));
                x += 28;
            }
        }
        out.push((
            Bounds {
                x,
                y: 3,
                width: 32,
                height: 38,
            },
            899,
        ));
        out
    }
    pub fn caption_width(&self) -> u32 {
        self.caption_controls().pop().map_or(1, |(last, _)| {
            (last.x + last.width as i32 - 8).max(1) as u32
        })
    }
    pub(super) fn tab_icon(&self, index: usize) -> Icon {
        if index == self.active_tab {
            return self.current_icon();
        }
        self.tabs.get(index).map_or(Icon::Home, |tab| {
            if tab.overview {
                Icon::Computer
            } else if tab.home {
                Icon::Home
            } else if tab.cwd == "/compat" {
                Icon::Drive
            } else if tab.cwd == self.home {
                Icon::Home
            } else if tab.cwd == RECYCLE_BIN {
                self.bin_icon()
            } else {
                Self::folder_icon(tab.cwd.rsplit('/').next().unwrap_or(""))
            }
        })
    }
    pub(super) fn tab_label(&self, index: usize) -> &str {
        if index == self.active_tab {
            self.title()
        } else {
            self.tabs.get(index).map_or("Home", |t| {
                if t.overview {
                    "This PC"
                } else if t.home {
                    "Home"
                } else if t.cwd == RECYCLE_BIN {
                    "Recycle Bin"
                } else {
                    t.cwd.rsplit('/').next().unwrap_or("Home")
                }
            })
        }
    }
    fn save_tab(&mut self) {
        let snapshot = Tab {
            id: self
                .tabs
                .get(self.active_tab)
                .map_or_else(door_workspace::document_tabs::next_id, |t| t.id),
            cwd: self.cwd.clone(),
            entries: self.entries.clone(),
            selected: self.selected,
            offset: self.offset,
            history: self.history.clone(),
            history_index: self.history_index,
            search: self.search.clone(),
            overview: self.overview,
            home: self.home_view,
            paths: self.result_paths.clone(),
            sections: self.home_sections.clone(),
            descending: self.descending,
            sort_key: self.sort_key,
            columns: self.columns,
            view: self.view,
            overview_view: self.overview_view,
            overview_selected: self.overview_selected,
            tag_filter: self.tag_filter,
            details_visible: self.details_visible,
            detail_tab: self.detail_tab,
        };
        if self.tabs.is_empty() {
            self.tabs.push(snapshot);
        } else {
            self.tabs[self.active_tab] = snapshot;
        }
    }
    pub(super) fn switch_tab(&mut self, index: usize, storage: &mut Storage) -> Result<(), Error> {
        if index == self.active_tab || index >= self.tab_count() {
            return Ok(());
        }
        if self.editor == 3 {
            self.finish_rename(storage);
            if self.editor == 3 {
                return Ok(());
            }
        }
        self.cancel_preview(storage);
        self.save_tab();
        let t = &self.tabs[index];
        self.cwd = t.cwd.clone();
        self.entries = t.entries.clone();
        self.selected = t.selected;
        self.offset = t.offset;
        self.history = t.history.clone();
        self.history_index = t.history_index;
        self.search = t.search.clone();
        self.overview = t.overview;
        self.home_view = t.home;
        self.home_sections = t.sections.clone();
        self.result_paths = t.paths.clone();
        self.home_scroll = 0;
        self.descending = t.descending;
        self.sort_key = t.sort_key;
        self.columns = t.columns;
        self.view = t.view;
        self.overview_view = t.overview_view;
        self.overview_selected = t.overview_selected;
        self.tag_filter = t.tag_filter;
        self.details_visible = t.details_visible;
        self.detail_tab = t.detail_tab;
        self.active_tab = index;
        self.reveal_tab();
        self.normalize_tag_view();
        self.preview_cache.clear();
        self.folders_scanned = false;
        self.thumbnail_scanned = false;
        self.editor = 0;
        self.rename = None;
        self.detail = None;
        self.last_click = None;
        self.last_click_bounds = None;
        self.last_click_point = None;
        self.detail_scroll = 0;
        // Results computed for the previous tab must not land here; the next
        // `preview_step` closes the stale job's handle.
        self.nav_generation = self.nav_generation.wrapping_add(1);
        self.dismiss_menu();
        // Re-enumerate on return, preserving selection by object rather than position.
        let object = self
            .selected
            .and_then(|i| self.entries.get(i))
            .map(|e| e.info.object);
        let listed = if self.home_view {
            // Home is re-read (pins, favorites, recent may have changed).
            self.show_home(storage).map(|()| None)
        } else {
            let cwd = self.cwd.clone();
            self.list_location(storage, &cwd).map(Some)
        };
        match listed {
            Ok(None) => {
                self.selected =
                    object.and_then(|id| self.entries.iter().position(|e| e.info.object == id));
            }
            Ok(Some(mut entries)) => {
                self.hide_tag_metadata(&self.cwd, &mut entries);
                self.entries = entries;
                self.result_paths.clear();
                if self.volume_search() {
                    self.apply_results();
                }
                self.sort();
                self.selected =
                    object.and_then(|id| self.entries.iter().position(|e| e.info.object == id));
                // The status bar derives the count from the active listing.
                self.message.clear();
            }
            Err(e) => {
                self.entries.clear();
                self.selected = None;
                self.message = format!("Cannot refresh this tab: {e:?}");
            }
        }
        self.offset = self.offset.min(self.max_offset());
        if self.overview {
            self.reveal_overview();
        } else if let Some(object) = self
            .selected
            .and_then(|i| self.entries.get(i))
            .map(|e| e.info.object)
        {
            self.reveal_object(object);
        }
        if self.details_visible {
            self.inspect_selection(storage)?;
        }
        let _ = door_sys::write(
            format!(
                "DOOR_P12 FILES TAB active={} count={} path={}\n",
                self.active_tab,
                self.tab_count(),
                self.cwd
            )
            .as_bytes(),
        );
        Ok(())
    }
    pub(super) fn new_tab(&mut self, storage: &mut Storage, path: String) -> Result<(), Error> {
        if self.tab_count() >= 8 {
            self.message = "Cannot open more than 8 tabs.".into();
            return Ok(());
        }
        if self.editor == 3 {
            self.finish_rename(storage);
            if self.editor == 3 {
                return Ok(());
            }
        }
        let entries = if path == HOME_PAGE {
            None
        } else {
            Some(self.list_location(storage, &path)?)
        };
        self.cancel_preview(storage);
        self.save_tab();
        self.active_tab = self.tabs.len();
        match entries {
            Some(entries) => self.install(path.clone(), entries),
            None => self.show_home(storage)?,
        }
        self.history = alloc::vec![path];
        self.history_index = 0;
        self.save_new_tab();
        self.reveal_tab();
        self.details_visible = false;
        self.last_click = None;
        let _ = door_sys::write(
            format!(
                "DOOR_P12 FILES TAB new={} count={} path={}\n",
                self.active_tab,
                self.tab_count(),
                self.cwd
            )
            .as_bytes(),
        );
        Ok(())
    }
    fn save_new_tab(&mut self) {
        self.tabs.push(Tab {
            id: door_workspace::document_tabs::next_id(),
            cwd: self.cwd.clone(),
            entries: self.entries.clone(),
            selected: self.selected,
            offset: 0,
            history: self.history.clone(),
            history_index: 0,
            search: String::new(),
            overview: false,
            home: self.home_view,
            paths: self.result_paths.clone(),
            sections: self.home_sections.clone(),
            descending: self.descending,
            sort_key: self.sort_key,
            columns: self.columns,
            view: self.view,
            overview_view: ViewMode::Medium,
            overview_selected: 0,
            tag_filter: None,
            details_visible: false,
            detail_tab: 0,
        });
    }
    pub(super) fn close_tab(&mut self, index: usize, storage: &mut Storage) -> Result<(), Error> {
        if self.tab_count() == 1 {
            self.pending_close = true;
            return Ok(());
        }
        if index >= self.tabs.len() {
            return Ok(());
        }
        if self.editor == 3 {
            self.finish_rename(storage);
            if self.editor == 3 {
                return Ok(());
            }
        }
        self.save_tab();
        if index == self.active_tab {
            let other = if index > 0 { index - 1 } else { 1 };
            self.switch_tab(other, storage)?;
        }
        self.tabs.remove(index);
        if index < self.active_tab {
            self.active_tab -= 1;
        }
        self.reveal_tab();
        Ok(())
    }
    pub(super) fn tab_action(&mut self, target: u32, storage: &mut Storage) -> Result<(), Error> {
        if target == 897 || target == 898 {
            let capacity = self.tab_capacity();
            self.tab_scroll = if target == 897 {
                self.tab_scroll.saturating_sub(capacity)
            } else {
                (self.tab_scroll + capacity).min(self.tab_count().saturating_sub(capacity))
            };
            return Ok(());
        }

        match target {
            899 => self.new_tab(storage, HOME_PAGE.into()),
            850..=857 => self.close_tab((target - 850) as usize, storage),
            800..=807 => self.switch_tab((target - 800) as usize, storage),
            _ => Ok(()),
        }
    }
    pub(super) fn double_click(&mut self, target: u32, x: i32, y: i32) -> bool {
        let now = door_sys::time().unwrap_or(0);
        let double = self
            .last_click
            .is_some_and(|(id, at)| id == target && now.saturating_sub(at) <= 50)
            && self
                .last_click_point
                .is_some_and(|(px, py)| (px - x).abs() <= 4 && (py - y).abs() <= 4);
        if double {
            self.last_click = None;
            self.last_click_bounds = None;
            self.last_click_point = None;
        } else {
            self.last_click = Some((target, now));
            self.last_click_point = Some((x, y));
        }
        double
    }
    pub(super) fn reveal_overview(&mut self) {
        self.reveal_position(self.overview_selected);
    }
    pub(super) fn scroll_items(&mut self, forward: bool) {
        let capacity = self.content_layout().capacity(self.view_mode()).max(1);
        self.offset = if forward {
            (self.offset + capacity).min(self.max_offset())
        } else {
            self.offset.saturating_sub(capacity)
        };
        self.thumbnail_scanned = false;
        self.folders_scanned = false;
        self.preview_cache.clear();
        self.last_click = None;
        self.last_click_bounds = None;
    }
    pub(super) fn path_for(&self, entry: &DirEntry) -> Option<String> {
        if self.recycle_view() {
            // Deleted items have no path until they are restored.
            return None;
        }
        if !self.result_paths.is_empty() {
            // Volume search results live in many folders.
            let i = self
                .entries
                .iter()
                .position(|e| e.info.object == entry.info.object)?;
            return self.result_paths.get(i).cloned();
        }
        let name = String::from_utf16(&entry.name[..entry.name_len]).ok()?;
        if self.cwd == "/compat" && name == "Users" {
            Some("/users".into())
        } else if self.cwd == "/" {
            Some(format!("/{name}"))
        } else {
            Some(format!("{}/{name}", self.cwd))
        }
    }
    /// Shows the selection in the details pane; with nothing selected, the
    /// open folder itself (This PC shows its summary), like Windows.
    pub(super) fn inspect_selection(&mut self, storage: &mut Storage) -> Result<(), Error> {
        if self.selection_count() > 1 {
            self.cancel_preview(storage);
            self.detail = None;
            return Ok(());
        }
        let path = if self.overview && self.search.is_empty() {
            self.drives()
                .get(self.overview_selected)
                .map(|d| String::from(d.path))
        } else {
            self.selected
                .and_then(|i| self.entries.get(i))
                .filter(|_| {
                    self.selected
                        .is_some_and(|i| self.shown().any(|(j, _)| i == j))
                })
                .and_then(|e| self.path_for(e))
        };
        if let Some(path) = path {
            self.show_details(storage, path)?;
        } else if !self.overview && !self.home_view && !self.recycle_view() && self.details_visible
        {
            self.show_details(storage, self.cwd.clone())?;
        } else {
            self.cancel_preview(storage);
            self.detail = None;
        }
        Ok(())
    }
    pub(super) fn show_details(
        &mut self,
        storage: &mut Storage,
        path: String,
    ) -> Result<(), Error> {
        self.details_visible = true;
        self.cancel_preview(storage);
        self.detail = None;
        self.thumbnail_scanned = false;
        self.folders_scanned = false;
        let (info, times) = storage.stat(&path)?;
        if !self.overview
            && self
                .selected
                .and_then(|i| self.entries.get(i))
                .is_some_and(|entry| {
                    self.path_for(entry).as_ref() == Some(&path) && entry.info.object != info.object
                })
        {
            return Err(Error::Service(Code::Changed));
        }
        let current = !self.overview && path == self.cwd;
        let name: String = if path == "/compat" {
            DRIVES[0].label.into()
        } else {
            path.rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("/")
                .into()
        };
        let kind = Kind::of(&name, info.directory);
        let tags = u32::from(self.tags.tags(info.object));
        self.detail = Some(Detail {
            path,
            name,
            info,
            times,
            kind,
            tags,
            children: Vec::new(),
            cookie: 0,
            children_done: !info.directory,
            truncated: false,
            status: String::new(),
            current,
        });
        self.detail_scroll = 0;
        // Keep the clicked item visible after the pane reflows the grid.
        if self.overview {
            self.reveal_overview();
        }
        if !self.overview
            && let Some(object) = self
                .selected
                .and_then(|i| self.entries.get(i))
                .map(|e| e.info.object)
        {
            self.reveal_object(object);
        }
        let _ = door_sys::write(
            format!(
                "DOOR_P12 FILES INSPECT object={} version={} kind={kind:?} tags={tags}\n",
                info.object, info.version
            )
            .as_bytes(),
        );
        Ok(())
    }
    pub(super) fn detail_action(&mut self, id: u32, storage: &mut Storage) -> Result<(), Error> {
        match id {
            910 => {
                self.details_visible = !self.details_visible;
                if self.details_visible {
                    self.inspect_selection(storage)?;
                }
            }
            911 => self.details_visible = false,
            912..=914 => {
                self.detail_tab = (id - 912) as u8;
                self.detail_scroll = 0;
            }
            915 => {
                if let Some(d) = self.detail.as_ref() {
                    if d.info.directory {
                        let path = d.path.clone();
                        self.navigate(storage, path)?;
                    } else {
                        self.detail_tab = 1;
                    }
                }
            }
            916 => {
                if let Some(d) = self.detail.as_ref()
                    && d.info.directory
                {
                    self.new_tab(storage, d.path.clone())?;
                }
            }
            917 => self.begin_rename(storage),
            918 => {
                self.scroll_pane(false);
            }
            919 => {
                self.scroll_pane(true);
            }
            920..=927 => self.toggle_user_tag((id - 920) as usize, storage)?,
            930 => self.begin_tag(),
            931 => self.editor = 4,
            932 => (),
            933 => {
                self.rail_offset = self.rail_offset.saturating_sub(120).max(0);
            }
            934 => {
                self.rail_offset = (self.rail_offset + 120).min(self.rail_scroll_max() as i32);
            }
            935 => (),
            938 => {
                if self.detail.is_none() {
                    self.show_details(storage, self.cwd.clone())?;
                }
                self.begin_tag();
            }
            929 => {
                self.cancel_preview(storage);
                self.preview_cache.clear();
                // Recount: deep changes do not change the folder's own version.
                self.size_cache.clear();
                self.size_job = None;
                self.inspect_selection(storage)?;
            }
            _ => (),
        }
        Ok(())
    }
    pub(super) fn cancel_preview(&mut self, storage: &mut Storage) {
        if let Some(job) = self.preview_job.take() {
            let _ = storage.close(job.file);
        }
    }
    pub fn release_previews(&mut self, storage: &mut Storage) {
        self.cancel_preview(storage);
    }
    pub(super) fn cached(&self, info: FileInfo) -> Option<&CachedPreview> {
        self.preview_cache.iter().find(|p| p.info == info)
    }
    /// What the details pane should describe: the single selected item, else
    /// the open folder. None: a summary (several items, Home, This PC).
    pub(super) fn pane_subject(&self) -> Option<String> {
        if self.selection_count() > 1 {
            return None;
        }
        let selected = self
            .selected
            .filter(|i| self.shown().any(|(j, _)| j == *i))
            .and_then(|i| self.entries.get(i))
            .and_then(|e| self.path_for(e));
        selected.or_else(|| {
            (!self.home_view && !self.overview && !self.recycle_view()).then(|| self.cwd.clone())
        })
    }
    /// Keeps the details pane on the current selection whatever changed it
    /// (keyboard, rename, refresh, tab switch): the selected item, else the
    /// open folder, else the summary. A subject that fails to inspect is
    /// not retried until the location or the subject changes.
    pub(super) fn follow_selection(&mut self, storage: &mut Storage) -> bool {
        if !self.details_visible || self.modal() || (self.overview && self.search.is_empty()) {
            return false;
        }
        let selection = (self.nav_generation, self.selected, self.marked.len());
        if self.pane_hold == Some(selection) {
            return false;
        }
        self.pane_hold = None;
        let wanted = self.pane_subject();
        if wanted.as_deref() == self.detail.as_ref().map(|d| d.path.as_str()) {
            return false;
        }
        match wanted {
            Some(path) => {
                let key = (self.nav_generation, path.clone());
                if self.auto_inspect.as_ref() == Some(&key) {
                    return false;
                }
                let shown = self.show_details(storage, path).is_ok();
                self.auto_inspect = (!shown).then_some(key);
                true
            }
            None => {
                self.cancel_preview(storage);
                self.detail = None;
                true
            }
        }
    }
    /// One enumeration/read step per event-loop iteration. Selection changes close
    /// the prior handle; results must still match the opened object and version.
    pub fn poll_preview(&mut self, storage: &mut Storage) -> bool {
        let inspected = self.follow_selection(storage);
        // The customizer may have saved a new context-menu profile.
        self.menu_profile_poll = self.menu_profile_poll.wrapping_add(1);
        if self.menu_profile_poll.is_multiple_of(64) && self.xmenu.is_none() {
            self.load_menu_profile(storage);
        }
        let sized = self.size_step(storage)
            | inspected
            | self.archive_step(storage)
            | self.extract_step(storage)
            | self.convert_step(storage)
            | self.meta_step(storage)
            | self.icon_step(storage)
            | self.save_prefs(storage)
            | self.save_recent(storage)
            | self.index_step(storage);
        let job_info = self.preview_job.as_ref().map(|j| j.info);
        sized
            | match self.preview_step(storage) {
                Ok(changed) => changed,
                Err(error) => {
                    self.cancel_preview(storage);
                    if let Some(info) = job_info {
                        self.cache_preview(CachedPreview {
                            info,
                            media: None,
                            image: None,
                            text: String::new(),
                            status: format!("Preview unavailable: {error:?}"),
                        });
                    }
                    if let Some(d) = self
                        .detail
                        .as_mut()
                        .filter(|d| job_info.is_none_or(|i| i == d.info))
                    {
                        d.status = format!("Preview unavailable: {error:?}");
                        d.children_done = true;
                    }
                    true
                }
            }
    }
    /// Totals for the inspected folder: cached, in progress, or none.
    pub(super) fn folder_size(&self) -> Option<door_workspace::explorer_size::Totals> {
        let d = self.detail.as_ref().filter(|d| d.info.directory)?;
        self.size_cache
            .iter()
            .find(|(info, _)| *info == d.info)
            .map(|(_, totals)| *totals)
            .or_else(|| {
                self.size_job
                    .as_ref()
                    .filter(|job| job.root == d.info)
                    .map(|job| job.scan.totals())
            })
    }
    /// The first visible Details folder row without remembered totals, when
    /// View > Show > Folder sizes is on (a Doors extra: Windows leaves the
    /// Size column blank for folders).
    pub(super) fn unsized_folder(&self) -> Option<(String, FileInfo)> {
        if !self.prefs.folder_sizes() || self.view != ViewMode::Details || self.overview {
            return None;
        }
        self.shown()
            .skip(self.offset)
            .take(self.content_layout().capacity(self.view))
            .filter(|(_, e)| e.info.directory)
            .find(|(_, e)| !self.size_cache.iter().any(|(i, _)| *i == e.info))
            .and_then(|(_, e)| self.path_for(e).map(|path| (path, e.info)))
    }
    /// Details' Size column text of folder `entry`: its remembered totals
    /// with Folder sizes on ("At least" while a listing was capped).
    pub(super) fn folder_size_text(&self, entry: &DirEntry) -> Option<String> {
        use door_workspace::explorer_size::State;
        if !self.prefs.folder_sizes() {
            return None;
        }
        let (_, t) = self.size_cache.iter().find(|(i, _)| *i == entry.info)?;
        let size = previewing::size(t.bytes);
        Some(match t.state {
            State::Partial => format!("> {size}"),
            _ => size,
        })
    }
    /// Advances the folder-size job by a bounded number of listing calls. The
    /// job follows the inspected folder: a different folder, location or tab
    /// generation drops it; a finished result is cached.
    fn size_step(&mut self, storage: &mut Storage) -> bool {
        use door_workspace::explorer_size::{Child, Scan, State};
        // The inspected folder first; then, with View > Show > Folder sizes,
        // the visible Details rows' folders, one job at a time.
        let wanted = self
            .detail
            .as_ref()
            .filter(|d| d.info.directory)
            .map(|d| (d.path.clone(), d.info))
            .filter(|(_, info)| !self.size_cache.iter().any(|(i, _)| i == info))
            .or_else(|| self.unsized_folder());
        let current = self.size_job.as_ref().is_some_and(|j| {
            Some(j.root) == wanted.as_ref().map(|w| w.1) && j.generation == self.nav_generation
        });
        if !current {
            self.size_job = None;
            let Some((path, info)) = wanted else {
                return false;
            };
            if self.size_cache.iter().any(|(i, _)| *i == info) {
                return false;
            }
            self.size_job = Some(SizeJob {
                scan: Scan::new(path, info.object),
                root: info,
                generation: self.nav_generation,
                current: None,
            });
        }
        let Some(job) = self.size_job.as_mut() else {
            return false;
        };
        for _ in 0..SIZE_RPCS_PER_TICK {
            if job.current.is_none() {
                match job.scan.next() {
                    Some((path, depth)) => job.current = Some((path, depth, 0, Vec::new())),
                    None => break,
                }
            }
            let Some((path, depth, cookie, children)) = job.current.as_mut() else {
                break;
            };
            match storage.list(path, *cookie) {
                Ok(Some(entry)) => {
                    *cookie = entry.cookie;
                    if let Ok(name) = String::from_utf16(&entry.name[..entry.name_len]) {
                        let child = if path.as_str() == "/compat" && name == "Users" {
                            "/users".into()
                        } else if path.as_str() == "/" {
                            format!("/{name}")
                        } else {
                            format!("{path}/{name}")
                        };
                        children.push(Child {
                            path: child,
                            object: entry.info.object,
                            directory: entry.info.directory,
                            bytes: entry.info.bytes,
                        });
                    }
                    if children.len() >= SIZE_DIRECTORY_LIMIT {
                        let (_, depth, _, children) = job.current.take().unwrap();
                        job.scan.record(depth, Ok(children));
                        job.scan.mark_partial();
                    }
                }
                Ok(None) => {
                    let (_, depth, _, children) = job.current.take().unwrap();
                    job.scan.record(depth, Ok(children));
                }
                Err(_) => {
                    let depth = *depth;
                    job.current = None;
                    job.scan.record(depth, Err(()));
                }
            }
        }
        let totals = job.scan.totals();
        if totals.state != State::Calculating {
            let root = job.root;
            self.size_job = None;
            if self.size_cache.len() == SIZE_CACHE {
                self.size_cache.remove(0);
            }
            self.size_cache.push((root, totals));
        }
        true
    }
    fn preview_step(&mut self, storage: &mut Storage) -> Result<bool, Error> {
        if self.modal() {
            self.cancel_preview(storage);
            return Ok(false);
        }
        if self.preview_job.as_ref().is_some_and(|j| {
            j.generation != self.nav_generation
                || (!j.path.starts_with(&format!("{}/", self.cwd))
                    && self.detail.as_ref().is_none_or(|d| d.path != j.path))
        }) {
            self.cancel_preview(storage);
        }
        if self.details_visible
            && self.pane().is_some()
            && let Some(d) = self.detail.as_mut()
            && d.status.is_empty()
            && d.info.directory
            && !d.children_done
        {
            match storage.list(&d.path, d.cookie)? {
                Some(e) => {
                    d.cookie = e.cookie;
                    let hidden = d.path == self.home
                        && (Self::name(&e) == super::prefs::FILE
                            || self.tags_error.is_empty()
                                && Self::name(&e) == ".door-explorer-tags");
                    if !hidden {
                        d.children.push(e);
                    }
                    if d.children.len() == 128 {
                        d.children_done = true;
                        d.truncated = true;
                        if storage.metadata(&d.path)? != d.info {
                            d.children.clear();
                            d.status = "Folder changed. Refresh to inspect it again.".into();
                        }
                    }
                }
                None => {
                    d.children_done = true;
                    if storage.metadata(&d.path)? != d.info {
                        d.children.clear();
                        d.status = "Folder changed. Refresh to inspect it again.".into();
                    }
                }
            }
            return Ok(d.children_done || d.children.len().is_multiple_of(16));
        }
        if self.preview_job.is_none() {
            let selected = if self.details_visible {
                self.detail
                    .as_ref()
                    .filter(|d| {
                        d.status.is_empty() && d.kind.previewable() && self.cached(d.info).is_none()
                    })
                    .map(|d| (d.path.clone(), d.info, d.kind))
            } else {
                None
            };
            // Thumbnails are requested only for visible image tiles and, in
            // Medium and Large icons, the first image of visible folders.
            // Cache misses beyond the fixed 8-item window never force an
            // eviction/reload loop.
            let thumbnail = if !self.thumbnail_scanned
                && selected.is_none()
                && self.preview_cache.len() < 8
                && self.view.thumbnails()
                && !self.overview
            {
                let folders = self.view.folder_thumbnails();
                self.shown()
                    .skip(self.offset)
                    .take(self.content_layout().capacity(self.view))
                    .find_map(|(_, e)| {
                        if Kind::of(&Self::name(e), e.info.directory) == Kind::Image {
                            return self
                                .cached(e.info)
                                .is_none()
                                .then(|| self.path_for(e))
                                .flatten()
                                .map(|path| (path, e.info, Kind::Image));
                        }
                        self.folder_image(e)
                            .filter(|(_, info)| folders && self.cached(*info).is_none())
                            .map(|(path, info)| (path.clone(), *info, Kind::Image))
                    })
            } else {
                None
            };
            let Some((path, info, kind)) = selected.or(thumbnail) else {
                self.thumbnail_scanned = true;
                return self.folder_step(storage);
            };
            let limit = if kind == Kind::Image {
                previewing::MAX_INPUT
            } else {
                previewing::MAX_TEXT
            };
            if info.bytes > limit as u64 && kind == Kind::Image {
                self.cache_preview(CachedPreview {
                    info,
                    media: None,
                    image: None,
                    text: String::new(),
                    status: "File exceeds the 1 MB image preview limit.".into(),
                });
                return Ok(true);
            }
            let file = match storage.open(&path, Disposition::OpenExisting, READ, SHARE_ALL) {
                Ok(file) => file,
                Err(error) => {
                    self.cache_preview(CachedPreview {
                        info,
                        media: None,
                        image: None,
                        text: String::new(),
                        status: format!("Preview unavailable: {error:?}"),
                    });
                    return Ok(true);
                }
            };
            let actual = storage.info(&file);
            if actual.as_ref().is_err() || actual.as_ref().is_ok_and(|i| *i != info) {
                let _ = storage.close(file);
                self.cache_preview(CachedPreview {
                    info,
                    media: None,
                    image: None,
                    text: String::new(),
                    status: "File changed. Refresh before previewing.".into(),
                });
                return Ok(true);
            }
            let media = matches!(kind, Kind::Audio | Kind::Video | Kind::Archive);
            let limit = if media {
                0
            } else {
                limit.min(info.bytes as usize)
            };
            let mut bytes = Vec::new();
            let mut margin = Vec::<u8>::new();
            if margin.try_reserve_exact(8 * 1024 * 1024).is_err()
                || bytes.try_reserve_exact(limit).is_err()
            {
                let _ = storage.close(file);
                self.cache_preview(CachedPreview {
                    info,
                    media: None,
                    image: None,
                    text: String::new(),
                    status:
                        "Preview needs more shared app memory. Close another window and refresh."
                            .into(),
                });
                return Ok(true);
            }
            drop(margin);
            self.preview_job = Some(PreviewJob {
                file,
                generation: self.nav_generation,
                path,
                info,
                kind,
                bytes,
                limit,
                media: media.then(|| door_workspace::media_preview::Reader::new(info.bytes)),
                fetch: None,
            });
            return Ok(false);
        }
        if self.preview_job.as_ref().is_some_and(|j| j.media.is_some()) {
            return self.media_step(storage);
        }
        let job = self.preview_job.as_mut().unwrap();
        let mut chunk = [0; door_service_protocol::CHUNK_BYTES];
        let count = (job.limit - job.bytes.len()).min(chunk.len());
        let n = if count > 0 {
            storage.read(&job.file, job.bytes.len() as u64, &mut chunk[..count])?
        } else {
            0
        };
        job.bytes.extend_from_slice(&chunk[..n]);
        if n > 0 && job.bytes.len() < job.limit {
            return Ok(false);
        }
        let job = self.preview_job.take().unwrap();
        let actual = storage.info(&job.file);
        let closed = storage.close(job.file);
        let actual = actual?;
        closed?;
        if actual != job.info || storage.metadata(&job.path)? != job.info {
            return Err(Error::Service(Code::StaleInstance));
        }
        if job.bytes.len() != job.limit {
            return Err(Error::Malformed);
        }
        let mut cached = CachedPreview {
            info: job.info,
            media: None,
            image: None,
            text: String::new(),
            status: String::new(),
        };
        if !door_workspace::runtime::working_memory(32 * 1024 * 1024) {
            cached.status =
                "Preview needs more shared app memory. Existing folders and previews are retained."
                    .into();
            self.cache_preview(cached);
            return Ok(true);
        }
        if job.kind == Kind::Image {
            match previewing::decode(&job.bytes) {
                Ok(i) => cached.image = Some(i),
                Err(e) => cached.status = e.into(),
            }
        } else {
            match previewing::text(&job.bytes) {
                Ok(t) => {
                    cached.text = t;
                    if job.info.bytes > job.limit as u64 {
                        cached.status = "Showing the first 16 KB.".into();
                    }
                }
                Err(e) => cached.status = e.into(),
            }
        }
        self.cache_preview(cached);
        let _ = door_sys::write(
            format!(
                "DOOR_P12 FILES PREVIEW object={} version={} bytes={}\n",
                job.info.object,
                job.info.version,
                job.bytes.len()
            )
            .as_bytes(),
        );
        Ok(true)
    }
    /// One slice of a ranged media preview (`media_preview::Reader`): at
    /// most `MEDIA_CALLS` storage reads, then, once the reader is done, the
    /// cover or poster decoded like an image and the result cached.
    fn media_step(&mut self, storage: &mut Storage) -> Result<bool, Error> {
        use door_workspace::media_preview::Step;
        const MEDIA_CALLS: usize = 256;
        let job = self.preview_job.as_mut().unwrap();
        let name = String::from(job.path.rsplit('/').next().unwrap_or(&job.path));
        let size = job.info.bytes;
        let mut chunk = [0; door_service_protocol::CHUNK_BYTES];
        let mut calls = 0;
        let done = loop {
            let reader = job.media.as_mut().unwrap();
            if let Some((at, end)) = job.fetch {
                if calls == MEDIA_CALLS {
                    return Ok(false);
                }
                calls += 1;
                let want = ((end - at) as usize).min(chunk.len());
                let n = storage.read(&job.file, at, &mut chunk[..want])?;
                if n == 0 {
                    return Err(Error::Malformed);
                }
                reader.insert(at, &chunk[..n]);
                let next = at + n as u64;
                job.fetch = (next < end).then_some((next, end));
                continue;
            }
            match reader.step(&name) {
                Step::Done(preview, report) => break (preview, report),
                // At least 4 KiB at a time: neighbouring headers usually
                // follow in the same block.
                Step::Fetch(at, len) => {
                    job.fetch = Some((at, at.saturating_add(len.max(4096)).min(size)))
                        .filter(|(a, e)| a < e);
                    if job.fetch.is_none() {
                        return Err(Error::Malformed);
                    }
                }
            }
        };
        let job = self.preview_job.take().unwrap();
        let actual = storage.info(&job.file);
        let closed = storage.close(job.file);
        let actual = actual?;
        closed?;
        if actual != job.info || storage.metadata(&job.path)? != job.info {
            return Err(Error::Service(Code::StaleInstance));
        }
        let (mut preview, report) = done;
        let picture = preview.picture.take();
        let image = picture
            .filter(|_| door_workspace::runtime::working_memory(32 * 1024 * 1024))
            .and_then(|bytes| previewing::decode(&bytes).ok());
        let _ = door_sys::write(
            format!(
                "DOOR_P15 FILES MEDIA_PREVIEW object={} format={} entries={} picture={} peaks={}\n",
                job.info.object,
                if preview.format.is_empty() {
                    "media"
                } else {
                    preview.format
                },
                preview.count,
                u8::from(image.is_some()),
                preview.peaks.len()
            )
            .as_bytes(),
        );
        self.cache_preview(CachedPreview {
            info: job.info,
            image,
            text: String::new(),
            status: String::new(),
            media: Some(alloc::boxed::Box::new(MediaPreview { preview, report })),
        });
        Ok(true)
    }
    fn folder_step(&mut self, storage: &mut Storage) -> Result<bool, Error> {
        if self.folders_scanned || !self.view.thumbnails() || self.overview {
            return Ok(false);
        }
        let next = self
            .shown()
            .skip(self.offset)
            .take(self.content_layout().capacity(self.view))
            .filter(|(_, e)| {
                e.info.directory && matches!(Self::folder_icon(&Self::name(e)), Icon::Folder)
            })
            .find(|(_, e)| !self.folder_cache.iter().any(|f| f.of(e) && f.done))
            .and_then(|(_, e)| {
                self.path_for(e)
                    .map(|path| (path, e.info, e.times.modified))
            });
        let Some((path, info, modified)) = next else {
            self.folders_scanned = true;
            return Ok(false);
        };
        let index = if let Some(i) = self
            .folder_cache
            .iter()
            .position(|f| f.info == info && f.modified == modified && f.path == path)
        {
            i
        } else {
            if self.folder_cache.len() >= FOLDER_CACHE {
                self.folder_cache.remove(0);
            }
            self.folder_cache.push(CachedFolder {
                info,
                modified,
                path,
                cookie: 0,
                entries: 0,
                image: None,
                done: false,
            });
            self.folder_cache.len() - 1
        };
        let f = &mut self.folder_cache[index];
        for _ in 0..FOLDER_RPCS_PER_STEP {
            match storage.list(&f.path, f.cookie) {
                Ok(Some(e)) => {
                    f.cookie = e.cookie;
                    f.entries += 1;
                    let name = Self::name(&e);
                    if f.image.is_none() && Kind::of(&name, e.info.directory) == Kind::Image {
                        f.image = Some((format!("{}/{name}", f.path), e.info));
                    }
                    f.done = f.image.is_some() || f.entries >= FOLDER_SCAN;
                }
                Ok(None) => f.done = true,
                Err(_) => {
                    f.done = true;
                    f.entries = 0;
                    f.image = None;
                }
            }
            if f.done {
                break;
            }
        }
        if f.done
            && !storage
                .metadata(&f.path)
                .is_ok_and(|current| current == f.info)
        {
            // Changed while listed: the plain folder until the parent's
            // watch re-lists it with its new identity.
            f.entries = 0;
            f.image = None;
        }
        if f.done && f.image.is_some() {
            // The thumbnail is loaded by the image preview pass.
            self.thumbnail_scanned = false;
        }
        Ok(f.done)
    }
    /// Drops remembered contents of the shown folder's children that no
    /// longer match the listing (`relist` after a storage watch change).
    pub(super) fn prune_folders(&mut self) {
        let prefix = format!("{}/", self.cwd.trim_end_matches('/'));
        let entries = &self.entries;
        self.folder_cache.retain(|f| {
            !f.path.starts_with(&prefix)
                || f.path[prefix.len()..].contains('/')
                || entries.iter().any(|e| f.of(e))
        });
    }
    /// A folder's content-aware icon in `mode` (`CachedFolder`). Known
    /// folders keep their own art; folders not listed yet are plain.
    pub(super) fn folder_look(&self, entry: &DirEntry, mode: ViewMode) -> FolderLook {
        if !mode.thumbnails()
            || !entry.info.directory
            || !matches!(Self::folder_icon(&Self::name(entry)), Icon::Folder)
        {
            return FolderLook::Plain;
        }
        let Some(folder) = self
            .folder_cache
            .iter()
            .find(|f| f.of(entry) && f.done && f.entries > 0)
        else {
            return FolderLook::Plain;
        };
        folder
            .image
            .as_ref()
            .filter(|(_, info)| {
                mode.folder_thumbnails() && self.cached(*info).is_some_and(|p| p.image.is_some())
            })
            .map_or(FolderLook::Sheets, |(_, info)| FolderLook::Thumbnail(*info))
    }
    /// The image a folder's icon shows as its thumbnail, once scanned.
    pub(super) fn folder_image(&self, entry: &DirEntry) -> Option<&(String, FileInfo)> {
        self.folder_cache
            .iter()
            .find(|f| f.of(entry) && f.done)
            .and_then(|f| f.image.as_ref())
    }
    fn cache_preview(&mut self, preview: CachedPreview) {
        if self.preview_cache.len() == 8 {
            self.preview_cache.remove(0);
        }
        self.preview_cache.push(preview);
    }
    /// Extra width a labelled command needs at the applied text size (0 at
    /// 100%): its label's advance grows with the text, and 10 px per
    /// character bounds Segoe UI's 14 px advance at 100% ("New" is 28).
    fn command_growth(text: &str) -> i32 {
        let w = 10 * text.len() as u32;
        (door_workspace::palette::text_len(w) - w) as i32
    }
    /// x of the two command bar dividers (after New, after Delete).
    pub(super) fn command_separators(&self) -> [i32; 2] {
        let shift = Self::command_growth("New");
        [112 + shift, 392 + shift]
    }
    pub(super) fn command_controls(&self) -> Vec<(Bounds, u32, Icon, &'static str, bool)> {
        let mut commands = Vec::new();
        // Buttons fill the bar (E1/E9) and hold one 14 px label: they grow
        // with its line box and labelled ones widen with its advance,
        // pushing later ones right.
        let (top, height) = xl::command_button();
        let details_width = 100 + Self::command_growth("Details") as u32;
        let details_x = self.layout.width as i32 - 8 - details_width as i32;
        let mut shift = 0;
        let recycle = self.recycle_view();
        let mut buttons = alloc::vec![
            (12, 92, 8, Icon::AddCircle, "New", !recycle),
            (
                122,
                40,
                44,
                Icon::Cut,
                "",
                self.command_reason(44).is_none()
            ),
            (
                166,
                40,
                45,
                Icon::Copy,
                "",
                self.command_reason(45).is_none()
            ),
            (
                210,
                40,
                46,
                Icon::Paste,
                "",
                self.command_reason(46).is_none()
            ),
            (
                254,
                40,
                33,
                Icon::Rename,
                "",
                self.rename_candidate().is_some(),
            ),
            (298, 40, 47, Icon::Share, "", false),
            (
                342,
                40,
                48,
                Icon::Delete,
                "",
                self.command_reason(48).is_none()
            ),
            (402, 90, 5, Icon::Sort, "Sort", !self.home_view),
            (500, 90, 6, Icon::PC, "View", true),
        ];
        let mut more = 596;
        if recycle {
            // Recycle Bin tools, as the 26H2 command bar shows them.
            let any = !self.entries.is_empty();
            let restore = if self.selection_count() > 0 {
                "Restore the selected items"
            } else {
                "Restore all items"
            };
            // Labelled when they fit before Details, else icon-only (their
            // names stay in the semantic tree and the context menus).
            let labelled = 596 + 176 + 4 + 214 + 4 + 40 + 12 < details_x + 8;
            if labelled {
                buttons.push((596, 176, 49, self.bin_icon(), "Empty Recycle Bin", any));
                buttons.push((776, 214, 50, Icon::Back, restore, any));
                more = 994;
            } else {
                buttons.push((596, 40, 49, self.bin_icon(), "", any));
                buttons.push((640, 40, 50, Icon::Back, "", any));
                more = 684;
            }
        }
        buttons.push((more, 40, 7, Icon::More, "", true));
        for (x, width, id, icon, text, enabled) in buttons {
            let grown = Self::command_growth(text);
            let (x, width) = (x + shift, width + grown as u32);
            shift += grown;
            if x + width as i32 + 12 < details_x + 8 {
                commands.push((
                    Bounds {
                        x,
                        y: top,
                        width,
                        height,
                    },
                    id,
                    icon,
                    text,
                    enabled,
                ));
            }
        }
        commands.push((
            Bounds {
                x: details_x,
                y: top,
                width: details_width,
                height,
            },
            910,
            Icon::DetailsPane,
            "Details",
            true,
        ));
        commands
    }
    pub(super) fn modern_target(&self, x: i32, y: i32, fonts: &FontAtlas) -> Option<u32> {
        if x < 0 || y < 0 || x >= self.layout.width as i32 || y >= self.layout.height as i32 {
            return None;
        }
        if self.modal() {
            if self.security_open() {
                return self.security_target(x, y);
            }
            if self.picker_open() {
                return self.picker_target(x, y);
            }
            if !self.removal.dialog.open() {
                return self.operations_modal_target(x, y);
            }
            return self
                .removal
                .dialog
                .target(&self.dialog_layout(), x, y)
                .map(|i| 3000 + i as u32);
        }
        if self.xmenu.is_some() {
            return self.menu_target(x, y);
        }
        if let Some(id) = self.progress_target(x, y) {
            return Some(id);
        }
        // The navigation pane splitter is a drag handle, never a click.
        if self.layout.on_splitter(x, y) {
            return None;
        }
        if self.editor == 3 && self.rename_bounds().is_some_and(|r| r.contains(x, y)) {
            return Some(34);
        }
        for i in self.visible_tabs() {
            let r = self.tab_bounds(i);
            if r.contains(x, y) {
                return Some(if x >= r.x + r.width as i32 - 26 {
                    850 + i as u32
                } else {
                    800 + i as u32
                });
            }
        }
        for (r, id) in self.caption_controls() {
            if r.contains(x, y) {
                return Some(id);
            }
        }
        if y >= self.layout.height as i32 - xl::status_height() as i32 {
            for (icons, id) in [(false, 40), (true, 41)] {
                if self.layout.status_view(icons).contains(x, y) {
                    return Some(id);
                }
            }
            return None;
        }
        for (index, id) in [20, 21, 22, 2].into_iter().enumerate() {
            if ExplorerLayout::navigation(index).unwrap().contains(x, y) {
                return Some(id);
            }
        }
        if let Some(id) = self.address_target(x, y, fonts) {
            return Some(id);
        }
        if self.layout.search().contains(x, y) {
            return Some(31);
        }
        // The double-click re-target only applies to inspect-then-open items in
        // the content area; rail rows navigate on the first click.
        if y >= content_top()
            && x >= self.layout.sidebar
            && self
                .last_click_point
                .is_some_and(|(px, py)| (px - x).abs() <= 4 && (py - y).abs() <= 4)
            && let Some((id, at)) = self.last_click
            && door_sys::time().is_ok_and(|now| now.saturating_sub(at) <= 50)
        {
            return Some(id);
        }
        if let Some(r) = self.pane()
            && r.contains(x, y)
        {
            return self.pane_target(r, x, y, fonts);
        }
        if (xl::command_top()..content_top()).contains(&y) {
            // Disabled buttons still answer so hovering shows why they are off;
            // their clicks do nothing.
            for (r, id, _, _, _) in self.command_controls() {
                if r.contains(x, y) {
                    return Some(id);
                }
            }
            return None;
        }
        if x < self.layout.sidebar {
            if y < content_top() {
                return None;
            }
            if let Some((track, thumb)) = self.rail_scrollbar()
                && track.contains(x, y)
            {
                return Some(if thumb.contains(x, y) {
                    935
                } else if y < thumb.y {
                    933
                } else {
                    934
                });
            }
            let y = y + self.rail_offset;
            if let Some(id) = self.rail_target(x, y) {
                return Some(id);
            }
            let tag_top = self.rail_tag_top();
            let create_y = tag_top + tag_head() + self.tags.names.len() as i32 * tag_pitch();
            if (create_y..create_y + 32 + xl::grow()).contains(&y) && self.tags_error.is_empty() {
                return Some(938);
            }
            for i in 0..self.tags.names.len() {
                let top = tag_top + tag_head() + i as i32 * tag_pitch();
                if (top..top + tag_pitch()).contains(&y) {
                    return Some(940 + i as u32);
                }
            }
            return None;
        }
        if y < content_top() {
            return None;
        }
        if self.home_view && x >= self.layout.sidebar {
            return self.home_target(x, y);
        }
        let l = self.content_layout();
        if x >= l.width as i32 - 10 && self.item_count() > l.capacity(self.view_mode()) {
            return Some(if y < self.layout.height as i32 / 2 {
                42
            } else {
                43
            });
        }
        // The Recycle Bin paints its own columns (no sort or resize there).
        if let Some(id) = self.column_target(x, y).filter(|_| !self.recycle_view()) {
            return Some(id);
        }
        if self.overview && self.search.is_empty() {
            if self.drives_header().contains(x, y) {
                return Some(502);
            }
            for (row, slot) in (self.offset..self.drives().len()).enumerate() {
                if self.item_cell(row).is_some_and(|r| r.contains(x, y)) {
                    return Some(500 + slot as u32);
                }
            }
        } else {
            for (row, (index, _)) in self
                .shown()
                .skip(self.offset)
                .take(l.capacity(self.view))
                .enumerate()
            {
                if self.item_cell(row).is_some_and(|r| r.contains(x, y)) {
                    return Some(100 + index as u32);
                }
            }
        }
        (x >= self.layout.sidebar).then_some(9998)
    }
    fn pane_target(&self, r: Bounds, x: i32, y: i32, fonts: &FontAtlas) -> Option<u32> {
        let top = r.y;
        if (r.x + r.width as i32 - 36..r.x + r.width as i32 - 8).contains(&x)
            && (top + 8..top + 36).contains(&y)
        {
            return Some(911);
        }
        let catalog = self.detail.is_none() && self.detail_tab == 2 && self.selection_count() <= 1;
        if !catalog {
            let open = Self::pane_open_button(r);
            if (open.y..open.y + open.height as i32).contains(&y) && x < r.x + 140 {
                return self
                    .detail
                    .as_ref()
                    .filter(|d| !d.current && (d.info.directory || d.kind.previewable()))
                    .map(|_| 915);
            }
            let tabs = Self::pane_tab_row(r);
            if (tabs.y..tabs.y + tabs.height as i32).contains(&y) {
                let i =
                    ((x - r.x - 12).max(0) as u32 / (r.width.saturating_sub(24) / 3).max(1)).min(2);
                return Some(912 + i);
            }
        }
        if let Some((track, thumb)) = self.pane_scrollbar(r)
            && track.contains(x, y)
        {
            return Some(if thumb.contains(x, y) {
                932
            } else if y < thumb.y {
                918
            } else {
                919
            });
        }
        if y < top + details::body() {
            return None;
        }
        let y = y + (self.detail_scroll * 24) as i32;
        if self.detail_tab == 2 {
            if !self.tags_error.is_empty() {
                return None;
            }
            let grid = door_workspace::explorer_tags::TagGrid::new(r.width, self.tags.names.len());
            let (gx, gy) = (x - r.x, y - top - details::body());
            if grid.create().contains(gx, gy) {
                return Some(if self.editor == 4 { 931 } else { 930 });
            }
            for i in 0..self.tags.names.len() {
                if grid.cell(i).is_some_and(|b| b.contains(gx, gy)) {
                    return Some(920 + i as u32);
                }
            }
            return None;
        }
        if self.detail_tab == 0 {
            return self.details_target(r, x, y, fonts);
        }
        None
    }
}

/// Integration-test hooks (text-size and contrast reviews in
/// `tests/text_size_apps.rs`): the unit tests' populated folder and Home.
#[cfg(test)]
impl Files {
    pub(crate) fn review_fixture(view: ViewMode, width: u32, height: u32) -> Self {
        let mut f = tests::fixture();
        f.configure(width, height);
        f.view = view;
        f
    }
    pub(crate) fn review_home(width: u32, height: u32) -> Self {
        let mut f = home_review::home(true, true);
        f.configure(width, height);
        f
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn entry(name: &str, object: u64, directory: bool) -> DirEntry {
        let mut units = [0; door_service_protocol::files::NAME_UNITS];
        let name_len = name.encode_utf16().count();
        for (out, u) in units.iter_mut().zip(name.encode_utf16()) {
            *out = u;
        }
        DirEntry {
            times: Default::default(),
            cookie: object,
            name: units,
            name_len,
            info: FileInfo {
                attributes: 0,
                object,
                version: 1,
                bytes: if directory { 0 } else { 3840 },
                directory,
            },
        }
    }
    pub(super) fn fixture() -> Files {
        let mut f = Files::empty("/users/owner".into(), Controls::default());
        f.configure(1460, 900);
        f.cwd = "/users/owner/Projects".into();
        f.overview = false;
        f.view = ViewMode::Large;
        f.entries = [
            ("Door OS", true),
            ("Research", true),
            ("Design", true),
            ("Assets", true),
            ("photo.jpg", false),
            ("project_proposal.docx", false),
            ("demo.mp4", false),
            ("model.glb", false),
            ("data.xlsx", false),
            ("diagram.svg", false),
            ("paper.pdf", false),
            ("config.json", false),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (name, dir))| entry(name, i as u64 + 10, dir))
        .collect();
        f.selected = Some(0);
        f.details_visible = true;
        f.detail = Some(Detail {
            path: "/users/owner/Projects/Door OS".into(),
            name: "Door OS".into(),
            info: f.entries[0].info,
            times: f.entries[0].times,
            kind: Kind::Folder,
            tags: 0,
            children: f.entries.clone(),
            cookie: 0,
            children_done: true,
            truncated: false,
            status: String::new(),
            current: false,
        });
        f.message = "12 items".into();
        f.history = alloc::vec![f.cwd.clone()];
        f.save_tab();
        f.tabs.push(Tab {
            id: door_workspace::document_tabs::next_id(),
            cwd: "/users/owner/Pictures".into(),
            entries: Vec::new(),
            selected: None,
            offset: 0,
            history: Vec::new(),
            history_index: 0,
            search: String::new(),
            overview: false,
            home: false,
            paths: Vec::new(),
            sections: Vec::new(),
            descending: false,
            sort_key: SortKey::Name,
            columns: DEFAULT_COLUMNS,
            view: ViewMode::Details,
            overview_view: ViewMode::Tiles,
            overview_selected: 0,
            tag_filter: None,
            details_visible: false,
            detail_tab: 0,
        });
        f.tabs.push(Tab {
            id: door_workspace::document_tabs::next_id(),
            cwd: "/users/owner/Documents".into(),
            entries: Vec::new(),
            selected: None,
            offset: 0,
            history: Vec::new(),
            history_index: 0,
            search: String::new(),
            overview: false,
            home: false,
            paths: Vec::new(),
            sections: Vec::new(),
            descending: false,
            sort_key: SortKey::Name,
            columns: DEFAULT_COLUMNS,
            view: ViewMode::Details,
            overview_view: ViewMode::Tiles,
            overview_selected: 0,
            tag_filter: None,
            details_visible: false,
            detail_tab: 0,
        });
        f
    }
    /// Windows 11 content-aware folders: empty ones stay plain, folders with
    /// items show sheets, and Medium/Large icons show the first image once
    /// its preview is decoded. Entries outlive navigation and are pruned
    /// only when a re-listing (storage watch) re-dates their folder.
    #[test]
    fn folder_icons_follow_their_contents_and_watch_relists() {
        let mut f = fixture();
        let cwd = f.cwd.clone();
        let (empty, papers, photos) = (
            entry("Empty", 70, true),
            entry("Papers", 71, true),
            entry("Photos", 72, true),
        );
        let image = FileInfo {
            attributes: 0,
            object: 99,
            version: 1,
            bytes: 64,
            directory: false,
        };
        for (e, entries, first) in [
            (&empty, 0, None),
            (&papers, 3, None),
            (&photos, 2, Some(image)),
        ] {
            f.folder_cache.push(CachedFolder {
                info: e.info,
                modified: e.times.modified,
                path: format!("{cwd}/{}", Files::name(e)),
                cookie: 0,
                entries,
                image: first.map(|i| (format!("{cwd}/Photos/a.png"), i)),
                done: true,
            });
        }
        assert_eq!(f.folder_look(&empty, ViewMode::Large), FolderLook::Plain);
        assert_eq!(f.folder_look(&papers, ViewMode::Large), FolderLook::Sheets);
        // Until the image is decoded, sheets.
        assert_eq!(f.folder_look(&photos, ViewMode::Medium), FolderLook::Sheets);
        assert_eq!(
            f.folder_image(&photos).map(|i| i.1),
            Some(image),
            "the preview pass loads the first image"
        );
        f.preview_cache.push(CachedPreview {
            info: image,
            media: None,
            image: Some(previewing::Image {
                width: 2,
                height: 2,
                source_width: 2,
                source_height: 2,
                bgra: alloc::vec![255; 16],
            }),
            text: String::new(),
            status: String::new(),
        });
        for mode in [ViewMode::Medium, ViewMode::Large, ViewMode::ExtraLarge] {
            assert_eq!(f.folder_look(&photos, mode), FolderLook::Thumbnail(image));
        }
        // Tiles show sheets; small views the plain folder.
        assert_eq!(f.folder_look(&photos, ViewMode::Tiles), FolderLook::Sheets);
        for mode in [ViewMode::Details, ViewMode::List, ViewMode::Small] {
            assert_eq!(f.folder_look(&photos, mode), FolderLook::Plain);
        }
        // Known folders keep their own art.
        let documents = entry("Documents", 73, true);
        assert_eq!(
            f.folder_look(&documents, ViewMode::Large),
            FolderLook::Plain
        );
        // A watch re-listing that re-dates "Papers" drops only its entry.
        let mut redated = papers;
        redated.times.modified = 7;
        f.entries = alloc::vec![empty, redated, photos];
        f.prune_folders();
        assert_eq!(f.folder_cache.len(), 2);
        assert_eq!(f.folder_look(&redated, ViewMode::Large), FolderLook::Plain);
        assert_eq!(f.folder_look(&empty, ViewMode::Large), FolderLook::Plain);
        assert_eq!(
            f.folder_look(&photos, ViewMode::Large),
            FolderLook::Thumbnail(image)
        );
    }
    #[test]
    fn grid_pane_and_tab_targets_match_visible_controls_at_all_sizes() {
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        for (width, height) in [(400, 400), (560, 498), (742, 498), (1000, 668), (1460, 900)] {
            let mut f = fixture();
            f.configure(width, height);
            let l = f.content_layout();
            if let Some(p) = f.pane() {
                assert!(p.x >= SIDEBAR);
                assert_eq!(p.x as u32 + p.width, width);
                assert_eq!(
                    f.modern_target(p.x + p.width as i32 - 22, p.y + 22, &fonts),
                    Some(911)
                );
            }
            for i in 0..l.capacity(f.view).min(12) {
                let r = f.item_cell(i).unwrap();
                assert!(r.x + r.width as i32 <= l.width as i32);
                assert_eq!(
                    f.modern_target(r.x + 5, r.y + 5, &fonts),
                    Some(100 + i as u32)
                );
            }
            for i in f.visible_tabs() {
                let r = f.tab_bounds(i);
                assert_eq!(
                    f.modern_target(r.x + 12, r.y + 12, &fonts),
                    Some(800 + i as u32)
                );
                assert_eq!(
                    f.modern_target(r.x + r.width as i32 - 12, r.y + 12, &fonts),
                    Some(850 + i as u32)
                );
            }
        }
    }
    #[test]
    fn format_actions_are_relevant_and_painting_does_not_change_preview_state() {
        let mut f = fixture();
        assert!(f.quick_actions().iter().any(|a| a.id == 916));
        let d = f.detail.as_mut().unwrap();
        d.kind = Kind::Pdf;
        d.info.directory = false;
        assert!(
            !f.quick_actions()
                .iter()
                .any(|a| matches!(a.id, 915 | 916 | 913))
        );
        f.detail.as_mut().unwrap().kind = Kind::Image;
        assert!(f.quick_actions().iter().any(|a| a.id == 913));
        assert!(f.preview_job.is_none());
        assert!(f.preview_cache.is_empty());
    }
    #[test]
    fn all_user_tags_and_drive_are_reachable_after_scrolling() {
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let mut f = fixture();
        f.configure(1000, 498);
        for i in 0..8 {
            f.tags.create(&format!("User tag {i}")).unwrap();
        }
        assert!(f.rail_scroll_max() > 0);
        f.rail_offset = f.rail_scroll_max() as i32;
        let y = f.rail_tag_top() + 42 + 7 * 28 - f.rail_offset + 10;
        assert_eq!(f.modern_target(80, y, &fonts), Some(947));
        let create = f.rail_tag_top() + 42 + 8 * 28 - f.rail_offset + 10;
        assert_eq!(f.modern_target(80, create, &fonts), Some(938));
        let (_, thumb) = f.rail_scrollbar().unwrap();
        assert_eq!(f.modern_target(thumb.x + 2, thumb.y + 2, &fonts), Some(935));
        f.rail_offset = 0;
        f.details_visible = false;
        f.overview = true;
        f.offset = 0;
        let r = f.item_cell(0).unwrap();
        assert_eq!(f.modern_target(r.x + 5, r.y + 5, &fonts), Some(500));
    }
    /// Center of a tree row in window space (rail scrolled to the top).
    fn rail_point(f: &Files, key: &str) -> (i32, i32) {
        let rows = f.rail_rows();
        let i = rows.iter().position(|r| r.key == key).unwrap();
        let r = f.rail_row_bounds(&rows)[i];
        (100, r.y + r.height as i32 / 2 - f.rail_offset)
    }
    #[test]
    fn tree_rows_open_locations_and_chevrons_only_toggle() {
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let f = fixture();
        let rows = f.rail_rows();
        let pc = rows.iter().position(|r| r.key == "pc").unwrap();
        let (x, y) = rail_point(&f, "pc");
        assert_eq!(f.modern_target(x, y, &fonts), Some(1100 + pc as u32));
        // The chevron of This PC is a separate target that never navigates.
        assert_eq!(f.modern_target(12, y, &fonts), Some(1400 + pc as u32));
        // C: is listed once, under This PC (no root duplicate, as Windows).
        let under_pc = rows.iter().position(|r| r.key == "pc/d:0").unwrap();
        assert!(rows.iter().all(|r| r.key != "d:0"));
        let (_, y) = rail_point(&f, "pc/d:0");
        assert_eq!(
            f.modern_target(100, y, &fonts),
            Some(1100 + under_pc as u32)
        );
        // Network has no provider: no chevron target.
        let (_, y) = rail_point(&f, "net");
        assert_ne!(
            f.modern_target(12, y, &fonts).map(|id| id >= 1400),
            Some(true)
        );
        // Separators never answer.
        let rows = f.rail_rows();
        let sep = rows.iter().position(|r| r.key == "sep:0").unwrap();
        let r = f.rail_row_bounds(&rows)[sep];
        assert_eq!(f.modern_target(100, r.y + 8, &fonts), None);
    }
    #[test]
    fn clicked_drive_row_highlights_and_listings_never_multiply() {
        let mut f = fixture();
        f.cwd = "/compat".into();
        f.overview = false;
        let rows = f.rail_rows();
        let first = f.rail_selected(&rows).unwrap();
        assert_eq!(rows[first].key, "pc/d:0");
        f.rail_key = Some("pc/d:0".into());
        assert_eq!(rows[f.rail_selected(&rows).unwrap()].key, "pc/d:0");
        // A stale rail key (location changed elsewhere) falls back.
        f.cwd = "/users/owner/Desktop".into();
        assert_eq!(rows[f.rail_selected(&rows).unwrap()].key, "pin:0");
        for _ in 0..5 {
            let rows = f.rail_rows();
            let drive = rows.iter().find(|r| r.key == "pc/d:0").unwrap().clone();
            let (pins, _) = f.rail_pins();
            let drives = f.rail_drives();
            if let Some(path) = f.tree.toggle(&drive, &pins, &drives) {
                f.tree
                    .set_children(&path, Ok(alloc::vec![("Users".into(), "/users".into())]));
            }
            let rows = f.rail_rows();
            assert!(rows.iter().filter(|r| r.key == "pc/d:0/Users").count() <= 1);
        }
    }
    #[test]
    fn double_click_retarget_is_limited_to_the_content_area() {
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let mut f = fixture();
        let (x, y) = rail_point(&f, "pc");
        f.last_click_point = Some((x, y));
        f.last_click = Some((105, door_sys::time().unwrap_or(0)));
        let pc = f.rail_rows().iter().position(|r| r.key == "pc").unwrap();
        assert_eq!(f.modern_target(x, y, &fonts), Some(1100 + pc as u32));
    }
    #[test]
    fn navigation_replaces_location_state_and_invalidates_pending_work() {
        let mut f = fixture();
        f.detail_scroll = 40;
        f.offset = 3;
        f.last_click_point = Some((300, 300));
        f.last_click = Some((100, 0));
        f.search = "door".into();
        let generation = f.nav_generation;
        f.navigate_entries(
            "/users/owner/B".into(),
            alloc::vec![entry("x.txt", 70, false)],
        );
        assert_eq!(f.cwd, "/users/owner/B");
        assert!(f.detail.is_none());
        assert_eq!(f.selected, None);
        assert_eq!(f.offset, 0);
        assert_eq!(f.detail_scroll, 0);
        assert_eq!(f.last_click_point, None);
        assert!(f.last_click.is_none());
        assert!(f.search.is_empty());
        assert_ne!(f.nav_generation, generation);
        assert_eq!(f.entries.len(), 1);
        assert_eq!(f.history[f.history_index], "/users/owner/B");
        assert_eq!(f.history[f.history_index - 1], "/users/owner/Projects");
    }
    #[test]
    fn this_pc_is_its_own_history_entry() {
        let mut f = fixture();
        f.show_this_pc(Vec::new());
        assert!(f.overview);
        assert_eq!(f.history[f.history_index], THIS_PC);
        assert_eq!(Files::history_label(THIS_PC), "This PC");
        f.navigate_entries("/users/owner/C".into(), Vec::new());
        assert!(!f.overview);
        assert_eq!(f.history[f.history_index - 1], THIS_PC);
        assert_eq!(f.cwd, "/users/owner/C");
    }
    #[test]
    fn history_dedupes_truncates_forward_entries_and_is_bounded() {
        let mut f = fixture();
        f.push_history("/users/owner/Projects");
        assert_eq!(f.history.len(), 1);
        f.push_history("/a");
        f.push_history("/b");
        f.history_index = 1;
        f.push_history("/c");
        assert_eq!(
            f.history,
            alloc::vec![
                String::from("/users/owner/Projects"),
                String::from("/a"),
                String::from("/c")
            ]
        );
        for i in 0..40 {
            f.push_history(&format!("/p{i}"));
        }
        assert_eq!(f.history.len(), 32);
        assert_eq!(f.history_index, 31);
        assert_eq!(f.history[31], "/p39");
    }
    #[test]
    fn tab_switch_resets_transient_state_and_invalidates_pending_work() {
        let mut f = fixture();
        f.detail_scroll = 12;
        f.last_click_point = Some((10, 10));
        let generation = f.nav_generation;
        f.restore_tab_state(1);
        assert_eq!(f.cwd, "/users/owner/Pictures");
        assert_eq!(f.detail_scroll, 0);
        assert_eq!(f.last_click_point, None);
        assert!(f.detail.is_none());
        assert_ne!(f.nav_generation, generation);
    }
    #[test]
    fn eight_tabs_scroll_within_the_app_caption_at_the_native_minimum() {
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let mut f = fixture();
        while f.tabs.len() < 8 {
            f.tabs.push(Tab {
                id: door_workspace::document_tabs::next_id(),
                cwd: "/users/owner/Documents".into(),
                entries: Vec::new(),
                selected: None,
                offset: 0,
                history: Vec::new(),
                history_index: 0,
                search: String::new(),
                overview: false,
                home: false,
                paths: Vec::new(),
                sections: Vec::new(),
                descending: false,
                sort_key: SortKey::Name,
                columns: DEFAULT_COLUMNS,
                view: ViewMode::Details,
                overview_view: ViewMode::Tiles,
                overview_selected: 0,
                tag_filter: None,
                details_visible: false,
                detail_tab: 0,
            });
        }
        for width in [160, 200, 240, 320, 359, 360, 400, 560, 742, 1000] {
            f.configure(width, 498);
            for active in 0..8 {
                f.active_tab = active;
                f.reveal_tab();
                if width < 360 {
                    assert!(f.visible_tabs().is_empty());
                    assert!(f.caption_controls().is_empty());
                    assert!(f.caption_width() + 8 <= width - 138);
                    continue;
                }
                assert!(f.visible_tabs().contains(&active));
                for i in f.visible_tabs() {
                    let r = f.tab_bounds(i);
                    assert!(r.x + r.width as i32 <= width as i32 - 138);
                    assert_eq!(
                        f.modern_target(r.x + 12, r.y + 12, &fonts),
                        Some(800 + i as u32)
                    );
                }
                for (r, id) in f.caption_controls() {
                    assert!(r.x + r.width as i32 <= width as i32 - 138);
                    assert_eq!(f.modern_target(r.x + 12, r.y + 12, &fonts), Some(id));
                }
            }
        }
    }
    #[test]
    fn catalog_reset_clears_obsolete_filter_and_clamps_scrolled_rail() {
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let mut f = fixture();
        f.configure(1000, 498);
        f.tag_filter = Some(7);
        f.rail_offset = 9999;
        f.normalize_tag_view();
        assert_eq!(f.tag_filter, None);
        assert_eq!(f.rail_offset, f.rail_scroll_max() as i32);
        // A scrolled rail never answers above its top (the rule under the
        // command bar, whose buttons fill the bar above it).
        assert_eq!(f.modern_target(80, content_top() - 1, &fonts), None);
        f.rail_offset = 80;
        f.rail_offset = f.rail_offset.saturating_sub(120).max(0);
        assert_eq!(f.rail_offset, 0);
    }
    #[test]
    fn tabs_keep_the_actual_special_folder_icons() {
        let f = fixture();
        assert!(matches!(f.tab_icon(0), Icon::Folder));
        assert!(matches!(f.tab_icon(1), Icon::Pictures));
        assert!(matches!(f.tab_icon(2), Icon::Documents));
    }
    #[test]
    fn render_installed_explorer_reference_geometry_at_2048() {
        let mut f = fixture();
        f.configure(2048, 1224);
        f.view = ViewMode::ExtraLarge;
        f.cwd = "/users/owner/Desktop".into();
        f.tabs.truncate(1);
        f.selected = None;
        f.detail = None;
        f.message = "6 items".into();
        f.entries.truncate(6);
        f.entries[4] = entry("Archive", 90, true);
        f.entries[5] = entry("preview.png", 91, false);
        for (i, e) in f.entries.iter().take(5).enumerate() {
            // Folders 0 and 1 are empty, the others hold items (sheets);
            // folder 3 holds an image (its thumbnail).
            let image = FileInfo {
                attributes: 0,
                object: 500,
                version: 1,
                bytes: 64,
                directory: false,
            };
            f.folder_cache.push(CachedFolder {
                info: e.info,
                modified: e.times.modified,
                path: String::new(),
                cookie: 0,
                entries: usize::from(i >= 2),
                image: (i == 3).then(|| (String::new(), image)),
                done: true,
            });
            if i == 3 {
                // A 64 x 40 sky-to-sand gradient.
                let bgra = (0..40)
                    .flat_map(|y: u32| {
                        (0..64u32).flat_map(move |x| {
                            let t = (y * 255 / 39) as u8;
                            [255 - t / 2, 160 + (x as u8 / 4), 40 + t / 2, 255]
                        })
                    })
                    .collect();
                f.preview_cache.push(CachedPreview {
                    info: image,
                    media: None,
                    image: Some(previewing::Image {
                        width: 64,
                        height: 40,
                        source_width: 64,
                        source_height: 40,
                        bgra,
                    }),
                    text: String::new(),
                    status: String::new(),
                });
            }
        }
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let images = ui::ImageCache::default();
        let mut pixels = alloc::vec![0;2048*1224*4];
        let mut view =
            ui::View::new_scaled(&mut pixels, 2048, 1224, &fonts, 1).with_images(&images);
        view.palette.dark = true;
        f.draw(&mut view);
        let mut ppm = alloc::vec::Vec::from(&b"P6\n2048 1224\n255\n"[..]);
        for p in pixels.chunks_exact(4) {
            ppm.extend_from_slice(&[p[2], p[1], p[0]]);
        }
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/explorer-review");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("reference-dark-2048.ppm"), ppm).unwrap();
        assert_eq!(f.pane().unwrap().x, 1688);
        assert_eq!(f.content_layout().view_columns(f.view), 6);
    }
    #[test]
    fn render_modern_explorer_review_artifacts_from_the_actual_native_painter() {
        let mut f = fixture();
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/explorer-review");
        std::fs::create_dir_all(&root).unwrap();
        for (name, width, height, scale, dark, tab) in [
            ("light", 1460, 900, 1, false, 0),
            ("dark", 1460, 900, 1, true, 0),
            ("narrow", 742, 498, 2, false, 0),
            ("contents", 1460, 900, 1, false, 1),
            ("tags", 1460, 900, 1, false, 2),
        ] {
            f.configure(width, height);
            f.detail_tab = tab;
            let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
            let mut pixels = alloc::vec![0;(width*height*scale*scale*4)as usize];
            let images = ui::ImageCache::default();
            let mut view =
                ui::View::new_scaled(&mut pixels, width * scale, height * scale, &fonts, scale)
                    .with_images(&images);
            view.palette.dark = dark;
            f.draw(&mut view);
            let mut ppm = format!("P6\n{} {}\n255\n", width * scale, height * scale).into_bytes();
            for p in pixels.chunks_exact(4) {
                ppm.extend_from_slice(&[p[2], p[1], p[0]]);
            }
            std::fs::write(root.join(format!("{name}.ppm")), ppm).unwrap();
            assert!(f.preview_job.is_none());
            assert!(f.preview_cache.is_empty());
        }
    }
}
#[cfg(test)]
mod view_review {
    extern crate std;
    use super::tests::fixture;
    use super::*;
    fn render(f: &Files, width: u32, height: u32, dark: bool) -> alloc::vec::Vec<u8> {
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let mut pixels = alloc::vec![0; (width * height * 4) as usize];
        let images = ui::ImageCache::default();
        let mut view =
            ui::View::new_scaled(&mut pixels, width, height, &fonts, 1).with_images(&images);
        view.palette.dark = dark;
        f.draw(&mut view);
        let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
        for p in pixels.chunks_exact(4) {
            ppm.extend_from_slice(&[p[2], p[1], p[0]]);
        }
        ppm
    }
    /// Every view mode and This PC through the native painter, plus the
    /// selection/hit-test agreement for each mode.
    #[test]
    fn every_view_mode_paints_and_hit_tests_its_items() {
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/explorer-review");
        std::fs::create_dir_all(&root).unwrap();
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let mut f = fixture();
        f.configure(1200, 760);
        f.details_visible = false;
        for mode in ViewMode::ALL {
            f.set_view(mode);
            assert_eq!(f.view, mode);
            for slot in 0..f.content_layout().capacity(mode).min(f.shown().count()) {
                let r = f.item_cell(slot).unwrap();
                let index = f.shown().nth(f.offset + slot).unwrap().0;
                assert_eq!(
                    f.modern_target(r.x + 3, r.y + 3, &fonts),
                    Some(100 + index as u32),
                    "{mode:?}"
                );
            }
            std::fs::write(
                root.join(format!("view-{:?}.ppm", mode)),
                render(&f, 1200, 760, false),
            )
            .unwrap();
        }
        // Details header: sort by Size, then reverse it; Date modified is
        // the second column.
        f.set_view(ViewMode::Details);
        let columns = f.detail_columns(f.content_layout());
        assert_eq!(columns.map(|c| c.0), SortKey::ALL);
        let size = columns[3].1;
        assert_eq!(f.modern_target(size.x + 10, size.y + 10, &fonts), Some(963));
        f.sort_by_column(3);
        assert_eq!(f.sort_key, SortKey::Size);
        f.sort_by_column(3);
        assert!(f.descending);
        f.sort_by_column(1);
        assert_eq!((f.sort_key, f.descending), (SortKey::Date, false));
        f.sort_by_column(3);
        f.sort_by_column(3);
        let edge = columns[0].1.x + columns[0].1.width as i32;
        assert_eq!(
            f.modern_target(edge, columns[0].1.y + 10, &fonts),
            Some(965)
        );
        assert!(f.pointer_down(edge, columns[0].1.y + 10));
        assert!(f.pointer_motion(edge + 40, columns[0].1.y + 10));
        f.pointer_up();
        assert_eq!(f.columns[0], DEFAULT_COLUMNS[0] + 40);
        std::fs::write(
            root.join("view-details-sorted.ppm"),
            render(&f, 1200, 760, false),
        )
        .unwrap();
        // This PC: drives only, with capacity from the volume model.
        f.overview = true;
        f.volume = VolumeState::Known(door_service_protocol::administration::VolumeUsage {
            total_bytes: 128 << 20,
            free_bytes: 40 << 20,
        });
        let r = f.item_cell(0).unwrap();
        assert_eq!(f.modern_target(r.x + 5, r.y + 5, &fonts), Some(500));
        assert_eq!(f.item_count(), 1);
        std::fs::write(root.join("this-pc-tiles.ppm"), render(&f, 1200, 760, false)).unwrap();
        f.volume = VolumeState::Unavailable;
        std::fs::write(
            root.join("this-pc-unavailable-dark.ppm"),
            render(&f, 1200, 760, true),
        )
        .unwrap();
        // Expanded navigation tree: This PC > C: with listed folders.
        let (pins, _) = f.rail_pins();
        let drives = f.rail_drives();
        let drive = f
            .rail_rows()
            .into_iter()
            .find(|r| r.key == "pc/d:0")
            .unwrap();
        let path = f.tree.toggle(&drive, &pins, &drives).unwrap();
        f.tree.set_children(
            &path,
            Ok(alloc::vec![
                ("Users".into(), "/users".into()),
                ("Windows".into(), "/compat/Windows".into()),
            ]),
        );
        f.overview = false;
        f.cwd = "/compat".into();
        f.rail_key = Some("pc/d:0".into());
        std::fs::write(root.join("tree-expanded.ppm"), render(&f, 1200, 760, false)).unwrap();
    }
}
#[cfg(test)]
mod paint_cost {
    extern crate std;
    use super::tests::fixture;
    use super::*;
    /// Host-side cost split of one app frame (release build): full repaint vs.
    /// the copy into the shared section vs. the server's validate+diff passes.
    /// Run with `cargo test --release -p door-workspace paint_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn frame_stage_costs_on_the_host() {
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let images = ui::ImageCache::default();
        for (w, h) in [(1460u32, 900u32), (1920, 1040)] {
            let mut f = fixture();
            f.configure(w, h);
            let mut pixels = alloc::vec![0u8; (w * h * 4) as usize];
            let mut section = alloc::vec![0u32; (w * h) as usize];
            let old = pixels.clone();
            let runs: u32 = 30;
            let t = std::time::Instant::now();
            for i in 0..runs {
                pixels.fill(0);
                let mut view =
                    ui::View::new_scaled(&mut pixels, w, h, &fonts, 1).with_images(&images);
                view.hover = Some((400, 200 + (i as i32 % 5) * 30));
                f.draw(&mut view);
            }
            let paint = t.elapsed() / runs;
            let t = std::time::Instant::now();
            for _ in 0..runs {
                for (i, p) in pixels.chunks_exact(4).enumerate() {
                    unsafe {
                        section
                            .as_mut_ptr()
                            .add(i)
                            .write_volatile(u32::from_le_bytes(p.try_into().unwrap()))
                    }
                }
            }
            let copy = t.elapsed() / runs;
            let t = std::time::Instant::now();
            let mut opaque = true;
            let mut changed = 0usize;
            for _ in 0..runs {
                for p in pixels.chunks_exact(4) {
                    assert!(!(p[0] > p[3] || p[1] > p[3] || p[2] > p[3]));
                    opaque &= p[3] == 255;
                }
                for (a, b) in old
                    .chunks_exact(w as usize * 4)
                    .zip(pixels.chunks_exact(w as usize * 4))
                {
                    if a != b {
                        changed += 1;
                    }
                }
            }
            let server = t.elapsed() / runs;
            std::println!(
                "{w}x{h}: paint {paint:?} copy {copy:?} validate+diff {server:?} (opaque={opaque}, rows={})",
                changed / runs as usize
            );
        }
    }
}
#[cfg(test)]
mod shell_review {
    //! Address bar, menus and details pane through the native painter:
    //! review captures in target/explorer-review plus geometry/hit agreement.
    extern crate std;
    use super::super::address::{CRUMB_CHEVRON, CRUMB_ICON, CRUMB_ROOT, CRUMB_TEXT};
    use super::super::menus::{MENU_ICON, MENU_ROW};
    use super::tests::fixture;
    use super::*;
    use door_workspace::context_menu::{self as cm, Hit};
    fn out(
        name: &str,
        f: &Files,
        size: (u32, u32),
        scale: u32,
        dark: bool,
        hover: Option<(i32, i32)>,
    ) -> Vec<u8> {
        let (width, height) = size;
        let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
        let mut pixels = alloc::vec![0; (width * height * scale * scale * 4) as usize];
        let images = ui::ImageCache::default();
        let mut view =
            ui::View::new_scaled(&mut pixels, width * scale, height * scale, &fonts, scale)
                .with_images(&images);
        view.palette.dark = dark;
        view.hover = hover;
        f.draw(&mut view);
        let mut ppm = format!("P6\n{} {}\n255\n", width * scale, height * scale).into_bytes();
        for p in pixels.chunks_exact(4) {
            ppm.extend_from_slice(&[p[2], p[1], p[0]]);
        }
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/explorer-review");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(format!("{name}.ppm")), &ppm).unwrap();
        pixels
    }
    fn reference_location() -> Files {
        let mut f = fixture();
        f.cwd = "/users/owner/Desktop/Doors OS".into();
        f.tabs.truncate(1);
        f
    }
    #[test]
    fn address_bar_follows_the_reference_spacing_and_hit_tests_every_plate() {
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let mut f = reference_location();
        f.configure(1460, 900);
        let (glyph, chain) = f.crumb_chain();
        assert!(
            matches!(glyph, Icon::PC),
            "Desktop roots its chain with the monitor glyph"
        );
        assert_eq!(
            chain.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
            ["Desktop", "Doors OS"]
        );
        let measure = |s: &str| {
            fonts
                .measure(s, TextStyle::Regular14)
                .map_or(0, |m| m.advance_width)
        };
        let (_, l) = f.crumb_geometry(&measure);
        assert_eq!(
            f.modern_target(l.icon.x + 4, l.icon.y + 4, &fonts),
            Some(CRUMB_ICON)
        );
        assert_eq!(
            f.modern_target(l.root.x + 4, l.root.y + 4, &fonts),
            Some(CRUMB_ROOT)
        );
        for (i, item) in l.items.iter().enumerate() {
            assert_eq!(
                f.modern_target(item.text.x + 2, item.text.y + 4, &fonts),
                Some(CRUMB_TEXT + i as u32)
            );
            assert_eq!(
                f.modern_target(item.chevron.x + 2, item.chevron.y + 4, &fonts),
                Some(CRUMB_CHEVRON + i as u32)
            );
        }
        let end = l.items.last().unwrap().chevron;
        assert_eq!(
            f.modern_target(end.x + end.width as i32 + 20, end.y + 4, &fonts),
            Some(30),
            "empty area edits"
        );
        // Ink measurement at 100%: icon at box + 10, chevron centers 22 px
        // from neighbouring ink (+-1.5 px for glyph side bearings).
        for dark in [true, false] {
            let pixels = out(
                if dark {
                    "address-dark"
                } else {
                    "address-light"
                },
                &f,
                (1460, 900),
                1,
                dark,
                None,
            );
            let a = f.layout.address();
            let bg = pixels[((a.y + a.height as i32 / 2) as usize * 1460 + a.x as usize + 3) * 4];
            let mut spans: Vec<(i32, i32)> = Vec::new();
            for x in a.x + 2..a.x + 400 {
                let ink = (a.y + 4..a.y + a.height as i32 - 4).any(|y| {
                    let p = pixels[(y as usize * 1460 + x as usize) * 4];
                    p.abs_diff(bg) > 60
                });
                match spans.last_mut() {
                    Some(s) if ink && s.1 + 3 >= x => s.1 = x,
                    _ if ink => spans.push((x, x)),
                    _ => (),
                }
            }
            std::println!("address spans dark={dark}: {spans:?}");
            assert_eq!(spans[0].0, a.x + 10, "{spans:?}");
            let chevrons: Vec<_> = spans.iter().filter(|s| s.1 - s.0 <= 6).collect();
            assert_eq!(chevrons.len(), 3, "{spans:?}");
            let center = |s: &(i32, i32)| (s.0 + s.1) as f32 / 2.0;
            assert!(
                (center(chevrons[0]) - (spans[0].1 as f32 + 1.0) - 22.0).abs() <= 1.5,
                "{spans:?}"
            );
        }
        // Narrow windows move leading segments into the overflow list.
        f.cwd = "/users/owner/Desktop/Doors OS/apps/workspace/src/files/modern".into();
        f.configure(742, 498);
        let (_, l) = f.crumb_geometry(&measure);
        assert!(l.overflow.is_some() && l.hidden > 0);
        assert_eq!(l.items.last().unwrap().index, f.crumb_chain().1.len() - 1);
        out("address-overflow", &f, (742, 498), 2, true, None);
        f.edit_address();
        out("address-editing", &f, (742, 498), 1, true, None);
        assert_eq!(
            f.edit.text(),
            "C:\\Users\\owner\\Desktop\\Doors OS\\apps\\workspace\\src\\files\\modern"
        );
    }
    #[test]
    fn context_menus_match_their_subject_and_paint_hit_test_alike() {
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let mut f = reference_location();
        f.configure(1460, 900);
        // Folder item.
        let cell = f.item_cell(0).unwrap();
        f.context(cell.x + 20, cell.y + 20, 1460, 900);
        let m = f.xmenu.as_ref().unwrap();
        assert_eq!(m.menu.icons.len(), 5);
        assert!(
            m.menu
                .entries
                .iter()
                .any(|e| e.action == cm::Action::OpenNewTab)
        );
        let p = m.root();
        for (i, r) in p.rows.iter().enumerate() {
            if let Some(r) = r {
                assert_eq!(
                    f.modern_target(r.x + 6, r.y + 6, &fonts),
                    Some(MENU_ROW + i as u32)
                );
            }
        }
        assert_eq!(
            f.modern_target(p.icons[2].x + 4, p.icons[2].y + 4, &fonts),
            Some(MENU_ICON + 2)
        );
        let cut = p.icons[0];
        out(
            "menu-folder-dark",
            &f,
            (1460, 900),
            1,
            true,
            Some((cut.x + 8, cut.y + 8)),
        );
        let open_tab = p.rows[0].unwrap();
        out(
            "menu-folder-light",
            &f,
            (1460, 900),
            1,
            false,
            Some((open_tab.x + 20, open_tab.y + 8)),
        );
        out("menu-folder-2x", &f, (1460, 900), 2, true, None);
        // File item.
        let cell = f.item_cell(4).unwrap();
        f.context(cell.x + 20, cell.y + 20, 1460, 900);
        assert!(
            f.xmenu
                .as_ref()
                .unwrap()
                .menu
                .entries
                .iter()
                .any(|e| e.action == cm::Action::Open)
        );
        out("menu-file-dark", &f, (1460, 900), 1, true, None);
        // Background with the View submenu open (keyboard).
        f.dismiss_menu();
        f.open_context(900, 700, 1460, 900, true);
        let m = f.xmenu.as_mut().unwrap();
        assert_eq!(m.key(0x14d), cm::Outcome::Stay);
        assert!(m.sub.is_some());
        let sub = m.submenu().unwrap();
        let first = sub.rows[0].unwrap();
        assert_eq!(m.hit(first.x + 4, first.y + 4), Hit::SubRow(0));
        out("menu-background-view", &f, (1460, 900), 1, true, None);
        // Opened near the bottom edge: upward.
        f.dismiss_menu();
        f.context(600, 860, 1460, 900);
        let m = f.xmenu.as_ref().unwrap();
        assert!(m.root().upward);
        out("menu-background-up-light", &f, (1460, 900), 1, false, None);
        // Classic menu (Settings > Context menu, or "Show more options").
        f.dismiss_menu();
        f.controls = f.controls.with(Controls::CLASSIC_CONTEXT_MENU, true);
        let cell = f.item_cell(0).unwrap();
        f.context(cell.x + 20, cell.y + 20, 1460, 900);
        assert_eq!(f.xmenu.as_ref().unwrap().menu.style, cm::Style::Classic);
        out("menu-classic-dark", &f, (1460, 900), 1, true, None);
        f.controls = Controls::default();
        // This PC drive and multiple selection.
        f.dismiss_menu();
        f.overview = true;
        let cell = f.item_cell(0).unwrap();
        f.context(cell.x + 20, cell.y + 20, 1460, 900);
        assert!(f.xmenu.as_ref().unwrap().menu.icons.is_empty());
        out("menu-drive-dark", &f, (1460, 900), 1, true, None);
        f.dismiss_menu();
        f.overview = false;
        f.select_all();
        assert_eq!(f.selection_count(), f.entries.len());
        let cell = f.item_cell(1).unwrap();
        f.context(cell.x + 20, cell.y + 20, 1460, 900);
        let m = f.xmenu.as_ref().unwrap();
        // 26H2: Compress to ZIP file lives in the "Manage file" flyout.
        assert!(
            m.menu
                .entries
                .iter()
                .any(|e| e.children.iter().any(|c| c.action == cm::Action::Compress))
        );
        assert_eq!(m.menu.entries.last().unwrap().action, cm::Action::Customize);
        assert!(!m.menu.icons[2].enabled, "rename needs a single item");
        out("menu-multiple-dark", &f, (1460, 900), 1, true, None);
        // Navigation tree row.
        f.dismiss_menu();
        f.marked.clear();
        let rows = f.rail_rows();
        let pc = rows.iter().position(|r| r.key == "pc").unwrap();
        let r = f.rail_row_bounds(&rows)[pc];
        f.context(100, r.y + 10 - f.rail_offset, 1460, 900);
        assert!(f.xmenu.is_some());
        out("menu-tree-this-pc", &f, (1460, 900), 1, true, None);
    }
    fn file_detail(f: &Files, index: usize, name: &str, kind: Kind, tags: u32) -> Detail {
        Detail {
            path: format!("{}/{name}", f.cwd),
            name: name.into(),
            info: f.entries[index].info,
            times: f.entries[index].times,
            kind,
            tags,
            children: Vec::new(),
            cookie: 0,
            children_done: true,
            truncated: false,
            status: String::new(),
            current: false,
        }
    }
    #[test]
    fn details_pane_follows_the_selection_through_rename() {
        // Owner report: Home then F2 on a file showed the folder ("notes,
        // 1 item") in the pane. The pane describes the selection.
        let mut f = reference_location();
        f.configure(1460, 900);
        f.details_visible = true;
        let i = f
            .shown()
            .map(|(i, _)| i)
            .find(|i| !f.entries[*i].info.directory)
            .unwrap();
        let entry = f.entries[i];
        let name = Files::name(&entry);
        f.selected = Some(i);
        f.detail = Some(file_detail(&f, i, &name, Kind::of(&name, false), 0));
        let path = f.path_for(&entry).unwrap();
        assert_eq!(f.pane_subject().as_deref(), Some(path.as_str()));
        assert!(f.reveal_for_rename(entry.info.object));
        let editable =
            door_workspace::file_name::editable_name(&entry.name[..entry.name_len]).unwrap();
        f.start_rename(entry, editable, 7);
        assert_eq!(f.editor, 3);
        assert_eq!(
            f.detail.as_ref().map(|d| d.path.clone()),
            Some(path.clone()),
            "rename keeps the item"
        );
        assert_eq!(
            f.pane_subject(),
            Some(path),
            "nothing to re-inspect while renaming"
        );
        out("details-rename-dark", &f, (1460, 900), 1, true, None);
        // Nothing selected: the open folder. Several: the summary.
        f.editor = 0;
        f.rename = None;
        f.selected = None;
        assert_eq!(f.pane_subject().as_deref(), Some(f.cwd.as_str()));
        f.select_all();
        assert_eq!(f.pane_subject(), None);
    }
    #[test]
    fn folders_reopen_with_their_own_view_and_sort_26h2() {
        let mut f = fixture();
        f.configure(1460, 900);
        f.navigate_entries("/users/owner/Pictures".into(), Vec::new());
        f.set_view(ViewMode::Large);
        f.sort_by_column(3);
        f.navigate_entries("/users/owner/Documents".into(), Vec::new());
        assert_eq!(
            f.view,
            ViewMode::Large,
            "a folder never changed keeps the tab's view"
        );
        f.set_view(ViewMode::Details);
        f.navigate_entries("/users/owner/Pictures".into(), Vec::new());
        assert_eq!((f.view, f.sort_key), (ViewMode::Large, SortKey::Size));
        f.navigate_entries("/users/owner/Documents".into(), Vec::new());
        assert_eq!(f.view, ViewMode::Details);
        assert!(f.prefs_dirty, "saved on the next tick");
        // This PC keeps its own view; its group collapse is remembered.
        f.show_this_pc(Vec::new());
        f.set_view(ViewMode::Content);
        f.navigate_entries("/users/owner/Pictures".into(), Vec::new());
        f.show_this_pc(Vec::new());
        assert_eq!(f.overview_view, ViewMode::Content);
        f.drives_expanded = false;
        f.remember_sections();
        let saved = super::super::prefs::Prefs::decode(&f.prefs.encode()).unwrap();
        assert!(saved.drives_collapsed);
        assert_eq!(saved.get(THIS_PC).unwrap().view, ViewMode::Content);
        assert!(f.item_cell(0).is_none(), "collapsed group hides its drives");
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let h = f.drives_header();
        assert_eq!(f.modern_target(h.x + 4, h.y + 4, &fonts), Some(502));
    }
    #[test]
    fn shift_f10_on_this_pc_matches_the_controls_scenario() {
        // tools/xtask controls: Shift+F10 on This PC, Up = Customize menu,
        // Up x3 = Properties.
        let mut f = Files::empty("/users/1001".into(), Controls::default());
        f.configure(1920, 1032);
        f.keyboard_context(1920, 1032);
        let m = f.xmenu.as_mut().unwrap();
        assert!(matches!(
            f.menu_for,
            MenuFor::Context {
                subject: cm::Subject::Background,
                ..
            }
        ));
        let action = |m: &cm::MenuState| match m.focus {
            cm::Focus::Row(i) => m.menu.entries[i].action,
            _ => panic!("no focus"),
        };
        m.key(0x148);
        assert_eq!(action(m), cm::Action::Customize);
        m.key(0x148);
        m.key(0x148);
        assert_eq!(action(m), cm::Action::Properties);
    }
    #[test]
    fn tab_close_glyph_stands_out_in_both_themes() {
        let mut f = fixture();
        f.configure(1460, 900);
        for dark in [true, false] {
            for scale in [1, 2] {
                let pixels = out(
                    if dark { "tabs-dark" } else { "tabs-light" },
                    &f,
                    (1460, 900),
                    scale,
                    dark,
                    None,
                );
                let width = (1460 * scale) as usize;
                for i in f.visible_tabs() {
                    let r = f.tab_bounds(i);
                    let (cx, cy) = (r.x + r.width as i32 - 22, r.y + r.height as i32 / 2);
                    let luma = |x: i32, y: i32| {
                        let at =
                            ((y * scale as i32) as usize * width + (x * scale as i32) as usize) * 4;
                        pixels[at..at + 3]
                            .iter()
                            .map(|c| u32::from(*c))
                            .sum::<u32>()
                            / 3
                    };
                    let back = luma(cx - 9, cy - 9);
                    let ink = (-4..=4)
                        .flat_map(|d| [luma(cx + d, cy + d), luma(cx + d, cy - d)])
                        .map(|l| l.abs_diff(back))
                        .max()
                        .unwrap();
                    assert!(
                        ink > 120,
                        "tab {i} close glyph contrast {ink} (dark {dark}, scale {scale})"
                    );
                }
            }
        }
    }
    #[test]
    fn details_pane_reports_real_metadata_for_each_kind() {
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let mut f = reference_location();
        f.configure(1460, 900);
        f.tags.create("Review").unwrap();
        f.tags.create("Build").unwrap();
        // Text file: while its bytes are read the pane says so; then the
        // Content section (from `file_metadata::Stream`) precedes File.
        f.selected = Some(11);
        f.detail = Some(file_detail(&f, 11, "config.json", Kind::Code, 0b11));
        let rows = f.detail_rows(f.detail.as_ref().unwrap());
        assert!(
            rows.iter()
                .any(|r| r.label == "Details" && r.value == "Reading...")
        );
        let mut stream = door_workspace::file_metadata::Stream::new(
            door_workspace::file_metadata::StreamKind::Text("JSON"),
        );
        stream.feed(b"{\n  \"a\": 1\n}\n");
        f.meta_cache.push((
            f.entries[11].info,
            details::Meta {
                report: stream.finish(true),
                failed: false,
            },
        ));
        let rows = f.detail_rows(f.detail.as_ref().unwrap());
        let value = |label: &str| {
            rows.iter()
                .find(|r| r.label == label)
                .map(|r| r.value.clone())
        };
        assert_eq!(value("Lines").as_deref(), Some("3"));
        assert_eq!(value("Encoding").as_deref(), Some("ASCII"));
        let sections: Vec<&str> = rows
            .iter()
            .filter(|r| r.is_section())
            .map(|r| r.label)
            .collect();
        assert_eq!(sections, ["Content", "File"]);
        assert_eq!(value("Size").as_deref(), Some("3.7 KB (3,840 bytes)"));
        assert_eq!(value("Type").as_deref(), Some("Source file (.json)"));
        assert_eq!(
            value("Location").as_deref(),
            Some("C:\\Users\\owner\\Desktop\\Doors OS")
        );
        out("details-file-dark", &f, (1460, 900), 1, true, None);
        out("details-file-light", &f, (1460, 900), 1, false, None);
        // Tag chips: remove targets.
        let r = f.pane().unwrap();
        let measure = |s: &str| {
            fonts
                .measure(s, TextStyle::Regular14)
                .map_or(0, |m| m.advance_width)
        };
        let l = f.pane_layout(r, &measure);
        assert_eq!(l.chips.len(), 2);
        let (chip, tag) = l.chips[1];
        // The Content section puts the tags below the fold: scroll them in.
        f.detail_scroll = (chip.y - r.y - 400).max(0) as usize / 24;
        let cy = chip.y - (f.detail_scroll * 24) as i32;
        assert_eq!(
            f.modern_target(chip.x + chip.width as i32 - 10, cy + 10, &fonts),
            Some(details::CHIP_REMOVE + tag as u32)
        );
        assert_eq!(
            f.modern_target(chip.x + 10, cy + 10, &fonts),
            Some(details::CHIP + tag as u32)
        );
        out("details-file-scrolled-dark", &f, (1460, 900), 1, true, None);
        f.detail_scroll = 0;
        f.begin_add_tag();
        f.edit.set("Release".into(), false);
        out("details-tag-editor-dark", &f, (1460, 900), 1, true, None);
        f.editor = 0;
        // Image: sections from the parsed header and Exif.
        f.detail = Some(file_detail(&f, 4, "photo.jpg", Kind::Image, 0));
        let mut report = door_workspace::file_metadata::Report {
            format: Some("JPEG image"),
            ..Default::default()
        };
        report.add(
            door_workspace::file_metadata::Section::Image,
            "Dimensions",
            "4032 x 3024",
        );
        report.add(
            door_workspace::file_metadata::Section::Camera,
            "Camera model",
            "EOS R6",
        );
        report.add(
            door_workspace::file_metadata::Section::Origin,
            "Date taken",
            "2024-05-01 10:20:30",
        );
        f.meta_cache.push((
            f.entries[4].info,
            details::Meta {
                report,
                failed: false,
            },
        ));
        let rows = f.detail_rows(f.detail.as_ref().unwrap());
        assert!(
            rows.iter()
                .any(|r| r.label == "Dimensions" && r.value == "4032 x 3024")
        );
        assert!(
            rows.iter()
                .any(|r| r.label == "Type" && r.value == "JPEG image (.jpg)")
        );
        let sections: Vec<&str> = rows
            .iter()
            .filter(|r| r.is_section())
            .map(|r| r.label)
            .collect();
        assert_eq!(sections, ["Origin", "Image", "Camera", "File"]);
        assert!(
            f.quick_actions()
                .iter()
                .any(|a| a.id == details::QA_CONVERT && a.enabled())
        );
        assert!(
            f.quick_actions()
                .iter()
                .any(|a| a.id == details::QA_EMAIL && !a.enabled())
        );
        out("details-image-dark", &f, (1460, 900), 1, true, None);
        // Drive and This PC summary.
        f.volume = VolumeState::Known(door_service_protocol::administration::VolumeUsage {
            total_bytes: 64 << 30,
            free_bytes: 20 << 30,
        });
        let mut drive = file_detail(&f, 0, "x", Kind::Folder, 0);
        drive.path = "/compat".into();
        drive.name = DRIVES[0].label.into();
        f.detail = Some(drive);
        let rows = f.detail_rows(f.detail.as_ref().unwrap());
        assert!(
            rows.iter()
                .any(|r| r.label == "Free space" && r.value.starts_with("20.0 GB"))
        );
        out("details-drive-dark", &f, (1460, 900), 1, true, None);
        f.overview = true;
        f.detail = None;
        out("details-this-pc-dark", &f, (1460, 900), 1, true, None);
        // Several items selected.
        f.overview = false;
        f.select_all();
        out("details-multiple-dark", &f, (1460, 900), 1, true, None);
        // Rail: tag rows and rename in place.
        f.marked.clear();
        f.begin_tag_rename(1);
        out("rail-tag-rename-light", &f, (1460, 900), 1, false, None);
    }
}
#[cfg(test)]
mod home_review {
    //! File Explorer Home through the native painter, with geometry, menu
    //! and keyboard agreement.
    extern crate std;
    use super::super::home::{EMPTY_FAVORITES, EMPTY_RECENT, FAVORITES, HEADER, QUICK, RECENT};
    use super::*;
    use door_workspace::context_menu::{self as cm};
    fn item(name: &str, object: u64, directory: bool) -> DirEntry {
        let mut units = [0u16; door_service_protocol::files::NAME_UNITS];
        for (o, u) in units.iter_mut().zip(name.encode_utf16()) {
            *o = u;
        }
        DirEntry {
            times: Default::default(),
            cookie: object,
            name: units,
            name_len: name.encode_utf16().count(),
            info: FileInfo {
                attributes: 0,
                object,
                version: 1,
                bytes: if directory { 0 } else { 2048 },
                directory,
            },
        }
    }
    /// Home as `show_home` builds it, without Storage.
    pub(super) fn home(favorites: bool, recent: bool) -> Files {
        let mut f = Files::empty("/users/1001".into(), Controls::default());
        f.configure(1460, 900);
        f.overview = false;
        f.home_view = true;
        f.prefs.pins = alloc::vec!["/users/1001/Projects".into()];
        let add = |f: &mut Files, path: &str, dir: bool, section: u8| {
            let name = path.rsplit('/').next().unwrap();
            f.entries
                .push(item(name, 100 + f.entries.len() as u64, dir));
            f.result_paths.push(path.into());
            f.home_sections.push(section);
        };
        for (path, _, _) in f.quick_access() {
            add(&mut f, &path, true, QUICK);
        }
        if favorites {
            f.prefs.favorites = alloc::vec!["/users/1001/Documents/plan.txt".into()];
            add(&mut f, "/users/1001/Documents/plan.txt", false, FAVORITES);
        }
        if recent {
            for path in [
                "/users/1001/notes/desktop.txt",
                "/users/1001/Pictures/photo.jpg",
                "/users/1001/Documents/plan.txt",
            ] {
                f.recent.record(path, door_workspace::recent::Kind::File);
            }
            for i in f
                .recent
                .items()
                .iter()
                .map(|i| i.path.clone())
                .collect::<Vec<_>>()
            {
                add(&mut f, &i, false, RECENT);
            }
        }
        f.history = alloc::vec![HOME_PAGE.into()];
        f
    }
    fn render(name: &str, f: &Files, dark: bool, hover: Option<(i32, i32)>) {
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let mut pixels = alloc::vec![0; 1460 * 900 * 4];
        let images = ui::ImageCache::default();
        let mut view = ui::View::new_scaled(&mut pixels, 1460, 900, &fonts, 1).with_images(&images);
        view.palette.dark = dark;
        view.hover = hover;
        f.draw(&mut view);
        let mut ppm = alloc::vec::Vec::from(&b"P6\n1460 900\n255\n"[..]);
        for p in pixels.chunks_exact(4) {
            ppm.extend_from_slice(&[p[2], p[1], p[0]]);
        }
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/explorer-review");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(format!("{name}.ppm")), ppm).unwrap();
    }
    #[test]
    fn home_lists_quick_access_favorites_and_recent_with_shared_hit_tests() {
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let mut f = home(true, true);
        assert_eq!(f.title(), "Home");
        let (glyph, chain) = f.crumb_chain();
        assert!(matches!(glyph, Icon::Home));
        assert_eq!(chain, [(String::from("Home"), String::from(HOME_PAGE))]);
        let rows = f.rail_rows();
        assert_eq!(rows[0].node, TreeNode::Home);
        assert_eq!(rows[f.rail_selected(&rows).unwrap()].node, TreeNode::Home);
        assert!(
            rows.iter().any(|r| r.label == "Projects"),
            "user pin in the navigation pane"
        );
        let l = f.home_layout();
        for (s, h) in l.headers.iter().enumerate() {
            assert_eq!(
                f.modern_target(h.x + 30, h.y + 10, &fonts),
                Some(HEADER + s as u32)
            );
        }
        for (i, cell) in l.cells.iter().enumerate() {
            let c = cell.unwrap();
            assert_eq!(
                f.modern_target(c.x + 5, c.y + 5, &fonts),
                Some(100 + i as u32),
                "item {i}"
            );
            assert_eq!(f.path_for(&f.entries[i]).unwrap(), f.result_paths[i]);
        }
        assert_eq!(
            l.columns.len(),
            2,
            "Favorites and Recent use Name / Location columns"
        );
        render("home-dark", &f, true, None);
        render(
            "home-light",
            &f,
            false,
            Some((l.headers[0].x + 30, l.headers[0].y + 10)),
        );
        // Large icons: Favorites and Recent become tiles; Quick access stays.
        f.set_view(ViewMode::Large);
        assert!(f.home_layout().columns.is_empty());
        assert_eq!(
            f.prefs.get(HOME_PAGE).unwrap().view,
            ViewMode::Large,
            "Home keeps its own view"
        );
        render("home-tiles-dark", &f, true, None);
        f.set_view(ViewMode::Details);
        // Collapsing a section hides its items and is remembered.
        let before = f.shown().count();
        f.toggle_home_section(QUICK);
        assert!(f.home_collapsed(QUICK) && f.prefs_dirty);
        assert_eq!(f.shown().count(), before - 7);
        assert!(f.home_layout().cells[0].is_none());
        let saved = super::super::prefs::Prefs::decode(&f.prefs.encode()).unwrap();
        assert_eq!(saved.home_collapsed & 1, 1);
        assert_eq!(saved.pins, ["/users/1001/Projects"]);
        assert_eq!(saved.favorites, ["/users/1001/Documents/plan.txt"]);
        render("home-collapsed-dark", &f, true, None);
        f.toggle_home_section(QUICK);
    }
    #[test]
    fn home_menus_pin_favorite_and_recent_commands_match_their_item() {
        let mut f = home(true, true);
        let l = f.home_layout();
        let at = |f: &Files, section: u8, nth: usize| {
            let i = (0..f.entries.len())
                .filter(|i| f.home_sections[*i] == section)
                .nth(nth)
                .unwrap();
            let c = f.home_layout().cells[i].unwrap();
            (c.x + 8, c.y + 8)
        };
        let labels = |f: &Files| {
            f.xmenu
                .as_ref()
                .unwrap()
                .menu
                .entries
                .iter()
                .filter(|e| !e.separator)
                .map(|e| (e.label.clone(), e.enabled))
                .collect::<Vec<_>>()
        };
        // Known folder: pinned for good.
        let (x, y) = at(&f, QUICK, 0);
        f.context(x, y, 1460, 900);
        assert!(labels(&f).contains(&(String::from("Unpin from Quick access"), false)));
        // User pin: unpinning works.
        let (x, y) = at(&f, QUICK, 6);
        f.context(x, y, 1460, 900);
        assert!(labels(&f).contains(&(String::from("Unpin from Quick access"), true)));
        // Favorite file.
        let (x, y) = at(&f, FAVORITES, 0);
        f.context(x, y, 1460, 900);
        let l1 = labels(&f);
        assert!(l1.contains(&(String::from("Remove from Favorites"), true)));
        assert!(l1.contains(&(String::from("Open file location"), true)));
        // Recent file.
        let (x, y) = at(&f, RECENT, 1);
        f.context(x, y, 1460, 900);
        let l2 = labels(&f);
        assert!(l2.contains(&(String::from("Remove from list"), true)));
        assert!(l2.contains(&(String::from("Open file location"), true)));
        assert!(l2.contains(&(String::from("Add to Favorites"), true)));
        let menu = f.xmenu.as_ref().unwrap();
        let remove = menu
            .menu
            .entries
            .iter()
            .position(|e| e.action == cm::Action::RemoveRecent)
            .unwrap();
        let r = menu.root().rows[remove].unwrap();
        render("home-menu-recent-dark", &f, true, Some((r.x + 20, r.y + 8)));
        // The recent list drops the item (Storage-free part of the action).
        let path = f.result_paths[(0..f.entries.len())
            .filter(|i| f.home_sections[*i] == RECENT)
            .nth(1)
            .unwrap()]
        .clone();
        assert!(f.recent.remove(&path));
        assert!(!f.recent.items().iter().any(|i| i.path == path));
        // Background: the controls scenario tail (Properties, Show more options,
        // Customize menu) also holds on Home.
        f.dismiss_menu();
        // Right-clicking the items above selected the last one; with nothing
        // selected Shift+F10 opens the background menu.
        f.selected = None;
        f.keyboard_context(1460, 900);
        let m = f.xmenu.as_mut().unwrap();
        assert!(
            !m.menu
                .entries
                .iter()
                .any(|e| e.label == "New" || e.label == "Paste")
        );
        let focused = |m: &cm::MenuState| match m.focus {
            cm::Focus::Row(i) => m.menu.entries[i].action,
            _ => panic!(),
        };
        m.key(0x148);
        assert_eq!(focused(m), cm::Action::Customize);
        m.key(0x148);
        m.key(0x148);
        assert_eq!(focused(m), cm::Action::Properties);
        let _ = l;
    }
    #[test]
    fn home_keyboard_wheel_and_empty_states() {
        let mut f = home(false, false);
        f.configure(900, 420);
        // Empty sections say what they will show, in Windows' words.
        let l = f.home_layout();
        assert_eq!(l.empty.len(), 2);
        render("home-empty-light", &f, false, None);
        let _ = (EMPTY_FAVORITES, EMPTY_RECENT);
        let mut f = home(true, true);
        f.configure(900, 420);
        assert!(f.home_scroll_max() > 0);
        assert!(f.wheel(400, 300, -2));
        assert_eq!(f.home_scroll, 96.min(f.home_scroll_max()));
        assert!(f.wheel(400, 300, 5));
        assert_eq!(f.home_scroll, 0);
        // Arrows walk every item across sections and keep it visible.
        f.selected = None;
        for _ in 0..f.entries.len() {
            f.select(1, false);
            let i = f.selected.unwrap();
            let c = f.home_layout().cells[i].unwrap();
            assert!(
                c.y >= content_top() && c.y + c.height as i32 <= 420 - 24,
                "item {i} at {c:?}"
            );
        }
        assert_eq!(f.selected, Some(f.entries.len() - 1));
        f.select(-1, true);
        assert_eq!(f.selected, Some(0));
        assert_eq!(f.home_scroll, 0);
    }
    #[test]
    fn date_modified_column_and_details_follow_the_region_and_zone() {
        use door_service_client::FileTimes;
        // 2026-10-24 19:05 UTC and a day earlier.
        let at = |day: u8| {
            door_abi::clock::WallClock {
                year: 2026,
                month: 10,
                day,
                hour: 19,
                minute: 5,
                second: 0,
            }
            .to_filetime()
            .unwrap()
        };
        let mut f = super::tests::fixture();
        f.set_view(ViewMode::Details);
        // Defaults: UTC, English (United States). Not recorded is blank.
        assert_eq!(f.date_text(at(24)), "10/24/2026 7:05 PM");
        assert_eq!(f.date_text(0), "");
        f.locale = (3, 3); // Pacific Time, German (Germany)
        assert_eq!(f.date_text(at(24)), "24.10.2026 12:05");
        f.locale = (39, 5); // Beijing, Chinese (Simplified, China)
        assert_eq!(f.date_text(at(24)), "2026/10/25 3:05");
        // Regional format overrides (24-hour time, yyyy-MM-dd) apply to
        // every date Explorer shows; 0 keeps the region's formats.
        f.locale = (16, 1); // UTC, English (United States)
        f.region_overrides = door_locale::region::Overrides {
            clock: 2,
            short_date: 6,
            ..door_locale::region::Overrides::NONE
        }
        .word();
        assert_eq!(f.date_text(at(24)), "2026-10-24 19:05");
        f.region_overrides = door_locale::region::Overrides {
            short_time: 2,
            ..door_locale::region::Overrides::NONE
        }
        .word();
        assert_eq!(f.date_text(at(24)), "10/24/2026 07:05 PM");
        f.region_overrides = 0;
        assert_eq!(f.date_text(at(24)), "10/24/2026 7:05 PM");
        f.locale = (16, 1);
        // Sort by Date modified: folders first, oldest first, blank oldest.
        let stamps = [at(23), 0, at(24), at(22)];
        for (entry, modified) in f.entries.iter_mut().skip(4).zip(stamps) {
            entry.times = FileTimes {
                created: at(20),
                modified,
                accessed: modified,
            };
        }
        f.sort_by_column(1);
        let files: Vec<u64> = f
            .entries
            .iter()
            .filter(|e| !e.info.directory)
            .map(|e| e.times.modified)
            .collect();
        let mut sorted = files.clone();
        sorted.sort();
        assert_eq!(files, sorted);
        assert!(f.entries.iter().take(4).all(|e| e.info.directory));
        // Details pane: Date modified and Date created, omitted when 0.
        let index = f
            .entries
            .iter()
            .position(|e| e.times.modified == at(24))
            .unwrap();
        let name = ui::display_ascii(&Files::name(&f.entries[index]));
        let mut d = Detail {
            path: format!("{}/{name}", f.cwd),
            name: name.clone(),
            info: f.entries[index].info,
            times: f.entries[index].times,
            kind: Kind::of(&name, false),
            tags: 0,
            children: Vec::new(),
            cookie: 0,
            children_done: true,
            truncated: false,
            status: String::new(),
            current: false,
        };
        let rows = f.detail_rows(&d);
        let value = |rows: &[super::super::details::Row], label: &str| {
            rows.iter()
                .find(|r| r.label == label)
                .map(|r| r.value.clone())
        };
        assert_eq!(
            value(&rows, "Date modified").as_deref(),
            Some("10/24/2026 7:05 PM")
        );
        assert_eq!(
            value(&rows, "Date created").as_deref(),
            Some("10/20/2026 7:05 PM")
        );
        d.times = FileTimes::NONE;
        let rows = f.detail_rows(&d);
        assert!(value(&rows, "Date modified").is_none() && value(&rows, "Date created").is_none());
    }
}
