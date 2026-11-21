//! Persistent filesystem browser. Entries and preview bytes come from Storage.
use super::*;
use crate::icons::Icon;
use door_workspace::explorer_preview::{self as previewing, Kind};
use door_workspace::explorer_sort::{self, SortKey};
#[path = "files/address.rs"]
mod address;
#[path = "files/archive.rs"]
mod archive;
#[path = "files/details.rs"]
mod details;
#[path = "files/extract.rs"]
mod extract;
#[path = "files/home.rs"]
mod home;
#[path = "files/menus.rs"]
mod menus;
// The compact menu painter shared with the Settings customizer preview.
#[path = "context_menu_paint.rs"]
mod menu_paint;
#[path = "files/modern.rs"]
mod modern;
#[path = "files/prefs.rs"]
mod prefs;
/// Programs' own icons (read in the background, drawn in every view).
#[path = "files/program_icons.rs"]
mod program_icons;
#[path = "files/search.rs"]
mod search;
#[path = "files/tags.rs"]
mod tags;
// door.accessibility tree and pane focus (shared adapter `door_workspace::semantics`).
#[cfg(test)]
#[path = "files/fit_review.rs"]
mod fit_review;
#[cfg(test)]
#[path = "files/scenario_points.rs"]
mod scenario_points;
#[path = "files/semantics.rs"]
mod semantics;
// File Explorer as a drag source and drop target (door.dnd).
#[path = "files/dnd.rs"]
mod dnd;
// Clipboard, file-operation dialogs, Recycle Bin and live refresh (P14).
#[path = "files/operations.rs"]
mod operations;
// Open with the default app, "Open with" and the app picker (P14).
#[path = "files/open_with.rs"]
mod open_with;
// "Open File - Security Warning" for programs from the Internet (P16).
#[path = "files/security.rs"]
mod security;
use alloc::vec::Vec;
use door_graphics_text::TextStyle;
use door_service_client::{DirEntry, Disposition, READ, SHARE_ALL};
use door_service_protocol::desktop::Controls;
use door_workspace::context_menu;
use door_workspace::dialog::Layout as DialogLayout;
pub(crate) use door_workspace::file_operations::RECYCLE_BIN;
use door_workspace::file_removal::{Observed, Removal};
use door_workspace::{
    explorer_layout::{self as xl, Bounds, ExplorerLayout, ViewMode, content_top},
    explorer_path::{self, Address},
    line_edit::LineEdit,
};
use home::HOME_PAGE;
use menus::MenuFor;
use modern::{CachedFolder, CachedPreview, Detail, PreviewJob, Tab};
const FOLDERS: &[(&str, Icon)] = &[
    ("Desktop", Icon::Desktop),
    ("Documents", Icon::Documents),
    ("Downloads", Icon::Downloads),
    ("Music", Icon::Music),
    ("Pictures", Icon::Pictures),
    ("Videos", Icon::Videos),
];
const PINNED_ORDER: [usize; 6] = [0, 2, 1, 4, 3, 5];
/// Details column widths: Name, Date modified, Type, Size.
const DEFAULT_COLUMNS: [u32; 4] = [300, 160, 160, 96];
/// A mounted volume listed under This PC.
pub(crate) struct Drive {
    pub label: &'static str,
    /// Door namespace root presented as the drive.
    pub path: &'static str,
}
/// The storage service mounts one persistent home volume; its compatibility
/// root is presented as C:. Further volumes need a mount enumeration API.
const DRIVES: [Drive; 1] = [Drive {
    label: "Local Disk (C:)",
    path: "/compat",
}];
/// Capacity query state for This PC; read once per visit or refresh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VolumeState {
    Unknown,
    Known(door_service_protocol::administration::VolumeUsage),
    Unavailable,
}
#[cfg(test)]
#[path = "../tests/support/files_raster.rs"]
mod raster_tests;
/// History token for the This PC overview. The overview shares the home
/// directory listing, so it needs its own entry for Back/Forward to work.
pub(crate) const THIS_PC: &str = "::ThisPC";
/// A Windows executable File Explorer opens by running it.
fn is_program(name: &str) -> bool {
    name.len() > 4
        && name.is_char_boundary(name.len() - 4)
        && name[name.len() - 4..].eq_ignore_ascii_case(".exe")
}
/// A Windows Installer package (msiexec.exe installs it).
fn is_package(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".msi")
}
/// msiexec.exe, Windows Installer's program.
const MSIEXEC: &str = "/compat/Windows/System32/msiexec.exe";
/// A batch file Terminal runs (Run as administrator elevates it too).
fn is_script(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".bat") || lower.ends_with(".cmd")
}
pub struct Files {
    /// Inherited registry and directory handles for program launch (0 in
    /// host tests).
    registry: u64,
    directory: u64,
    tags: door_workspace::explorer_tags::Catalog,
    tag_filter: Option<usize>,
    tags_error: String,
    /// Catalog tag being renamed in place in the navigation pane (editor 5).
    tag_rename: Option<usize>,
    rail_offset: i32,
    rail_drag: Option<i32>,
    /// Details column being resized: (column, press x, width at press).
    column_drag: Option<(usize, i32, u32)>,
    /// Navigation pane splitter drag: pointer x minus the pane width.
    sidebar_drag: Option<i32>,
    /// Drag source and drop target state (`files/dnd.rs`).
    dnd: dnd::State,
    /// Navigation tree expansion and lazily listed subfolders (per window).
    tree: door_workspace::explorer_tree::Tree,
    /// Tree row the user last opened, so an aliased location highlights
    /// the row that was clicked rather than its first occurrence.
    rail_key: Option<String>,
    pub pending_close: bool,
    /// App to launch through the window server (9 = Terminal in home).
    pub pending_launch: Option<u32>,
    /// Location for "Open in new window"; taken by the window factory.
    window_request: core::cell::Cell<Option<door_workspace::explorer_search::Open>>,
    window_flag: bool,
    tabs: Vec<Tab>,
    active_tab: usize,
    tab_scroll: usize,
    details_visible: bool,
    detail_tab: u8,
    detail_scroll: usize,
    detail: Option<Detail>,
    /// (location generation, subject) whose inspection last failed, so the
    /// pane follows the selection without retrying a failure each tick.
    auto_inspect: Option<(u64, String)>,
    /// Properties showed something other than the selection: the selection
    /// (location generation, focus, marked count) it holds for.
    pane_hold: Option<(u64, Option<usize>, usize)>,
    preview_job: Option<PreviewJob>,
    /// Recursive size of the inspected folder, advanced a few RPCs per tick.
    size_job: Option<modern::SizeJob>,
    /// Finished folder totals keyed by the folder's (object, version).
    size_cache: Vec<(
        door_service_client::FileInfo,
        door_workspace::explorer_size::Totals,
    )>,
    /// Header/line scan of a large inspected file (dimensions, line count).
    meta_job: Option<details::MetaJob>,
    icon_job: Option<program_icons::IconJob>,
    program_icons: Vec<(
        door_service_client::FileInfo,
        Option<door_workspace::program_icon::ProgramIcon>,
    )>,
    meta_cache: Vec<(door_service_client::FileInfo, details::Meta)>,
    /// Background ZIP creation ("Compress to ZIP file", "Create archive").
    archive: Option<archive::ArchiveJob>,
    /// Background "Extract All".
    extract: Option<extract::ExtractJob>,
    /// Background image conversion ("Convert").
    convert: Option<archive::ConvertJob>,
    /// Tag chips as last painted: (bounds, tag, target id). Chip widths are
    /// measured text, so right-click hit testing reuses the painted layout.
    chip_hits: core::cell::RefCell<Vec<(Bounds, usize, u32)>>,
    /// Details content height as last painted (scroll range).
    detail_height: core::cell::Cell<usize>,
    /// 26H2 per-folder view/sort and remembered sections.
    prefs: prefs::Prefs,
    prefs_dirty: bool,
    /// Settings > Personalization > Context menu > Customize (the profile
    /// file the customizer saves), re-read when it changes.
    menu_profile: door_workspace::context_menu_profile::Profile,
    menu_profile_info: Option<Option<door_service_client::FileInfo>>,
    menu_profile_poll: u32,
    prefs_writable: bool,
    /// This PC search: an index of every name on the mounted volumes.
    index: Option<door_workspace::explorer_search::Index>,
    /// Index size when the shown results were computed.
    index_applied: usize,
    /// Full paths of `entries` while they are volume search results or Home.
    result_paths: Vec<String>,
    /// File Explorer Home is shown (cwd is the profile folder).
    home_view: bool,
    /// Home pixel scroll.
    home_scroll: i32,
    /// Home section of each entry (aligned with `entries`).
    home_sections: Vec<u8>,
    /// Recent items (Home > Recent), shared with Notes and Preview.
    recent: door_workspace::recent::Recent,
    /// Rasterized chevron masks: (scale, pointing down, coverage).
    chevron_cache: core::cell::RefCell<Vec<(u32, bool, Vec<u8>)>>,
    preview_cache: Vec<CachedPreview>,
    folder_cache: Vec<CachedFolder>,
    folders_scanned: bool,
    last_click: Option<(u32, u64)>,
    last_click_bounds: Option<Bounds>,
    last_click_point: Option<(i32, i32)>,
    thumbnail_scanned: bool,
    pane_drag: Option<i32>,
    pane_focused: bool,
    pub pending_settings: Option<u32>,
    /// Run as administrator for the window server: the path, whether it is
    /// a batch file, and a program's bytes; sent one frame later, once the
    /// closed context menu is off screen (the secure desktop freezes it).
    pending_elevation: Option<(String, bool, Vec<u8>)>,
    elevation_frame: bool,
    /// Open Windows 11 menu or flyout and what it acts on.
    xmenu: Option<context_menu::MenuState>,
    menu_for: MenuFor,
    /// Shift+F10 with nothing selected opens the background menu.
    background_menu: bool,
    /// Address-bar plate whose flyout is open (its chevron points down).
    crumb_flyout: Option<u32>,
    cwd: String,
    home: String,
    entries: Vec<DirEntry>,
    selected: Option<usize>,
    /// Additional selection (object ids) from Ctrl+A / Shift+arrows.
    marked: Vec<u64>,
    /// Object where a Shift+arrow range started.
    anchor: Option<u64>,
    offset: usize,
    preview: String,
    message: String,
    history: Vec<String>,
    history_index: usize,
    /// Bumped on every location change so late results from an earlier
    /// location (preview/size jobs, sticky double-click targets) are dropped.
    nav_generation: u64,
    volume: VolumeState,
    layout: ExplorerLayout,
    overview: bool,
    descending: bool,
    sort_key: SortKey,
    /// Details column widths (Name, Date modified, Type, Size); per tab,
    /// inherited by new tabs.
    columns: [u32; 4],
    /// Time zone and regional format ids (`door_locale`) for dates.
    locale: (u32, u32),
    editor: u8,
    edit: LineEdit,
    search: String,
    overview_selected: usize,
    view: ViewMode,
    overview_view: ViewMode,
    folders_expanded: bool,
    drives_expanded: bool,
    controls: Controls,
    rename: Option<ObservedRename>,
    removal: Removal,
    removal_name: String,
    removal_error: String,
    /// Navigation-pane keyboard focus (tree row), `files/semantics.rs`.
    nav_focus: Option<usize>,
    /// Clipboard, file operations, Recycle Bin and live refresh (P14).
    ops: operations::Ops,
    /// Default apps, Open with and the app picker (`files/open_with.rs`).
    apps: open_with::State,
    /// Regional format overrides word (`preferences::REGIONAL_OVERRIDES`),
    /// re-read with the time zone and region.
    region_overrides: u32,
    /// The device name (This PC details), read on each This PC visit.
    device_name: door_service_protocol::device::Name,
    /// The open "Open File - Security Warning".
    security: Option<door_workspace::security_warning::Warning>,
}
#[derive(Clone, Copy)]
struct ObservedRename {
    parent: u64,
    object: u64,
    version: u64,
}
impl Files {
    pub fn storage_retired(&mut self) {
        self.operations_disconnected();
        self.removal.connection_lost();
        if self.modal() {
            self.removal_error =
                "Storage disconnected. Refresh this folder before deleting.".into();
        }
        self.preview_job = None;
        self.preview_cache.clear();
        self.folder_cache.clear();
        self.folders_scanned = false;
        self.detail = None;
        self.rename = None;
        if self.editor == 3 {
            self.editor = 0;
        }
        self.dismiss_menu();
        self.meta_job = None;
        // Handles of the old connection are gone; partial output is reported.
        if self.archive.take().is_some() || self.convert.take().is_some() {
            self.message =
                "Storage disconnected; the archive or conversion was not completed.".into();
            return;
        }
        self.message = "Storage disconnected. Refresh this folder after reconnecting.".into();
    }
    pub fn modal(&self) -> bool {
        self.removal.dialog.open()
            || self.operations_modal()
            || self.picker_open()
            || self.security_open()
    }
    fn dialog_layout(&self) -> DialogLayout {
        DialogLayout::new(
            self.layout.width,
            self.layout.height,
            self.removal.dialog.count(),
        )
    }
    pub fn pointer_down(&mut self, x: i32, y: i32) -> bool {
        if !self.modal() {
            if let Some(id @ 965..=968) = self.column_target(x, y) {
                let column = (id - 965) as usize;
                // Drags start from the width shown (fitted to the list).
                let shown = self.detail_columns(self.content_layout())[column].1.width;
                self.column_drag = Some((column, x, shown));
                return true;
            }
            if let Some((_, thumb)) = self.rail_scrollbar()
                && thumb.contains(x, y)
            {
                self.rail_drag = Some(y - thumb.y);
                self.pane_focused = false;
                return true;
            }
            // The splitter between the navigation pane and the item view.
            if self.layout.on_splitter(x, y) {
                self.sidebar_drag = Some(x - self.layout.sidebar);
                return true;
            }
            if let Some((_, thumb)) = self.pane().and_then(|r| self.pane_scrollbar(r))
                && thumb.contains(x, y)
            {
                self.pane_drag = Some(y - thumb.y);
                self.pane_focused = true;
                return true;
            }
            return false;
        }
        if !self.removal.dialog.open() {
            self.operations_pointer(x, y);
            return true;
        }
        let target = self.removal.dialog.target(&self.dialog_layout(), x, y);
        self.removal.dialog.pointer(target);
        true
    }
    pub fn configure(&mut self, width: u32, height: u32) {
        if (width, height) != (self.layout.width, self.layout.height) {
            self.dismiss_menu();
        }
        self.layout = ExplorerLayout {
            width,
            height,
            sidebar: self.layout.sidebar,
        };
        self.reveal_tab();
        self.pane_drag = None;
        self.rail_drag = None;
        self.rail_offset = self.rail_offset.min(self.rail_scroll_max() as i32);
        self.last_click = None;
        self.last_click_bounds = None;
        self.thumbnail_scanned = false;
        self.folders_scanned = false;
        if self.editor == 4 {
            let draft: String = self.edit.text().into();
            self.editor = 0;
            self.begin_tag();
            if self.editor == 4 {
                self.edit.set(draft, false);
            } else {
                self.message = "Tag creation cancelled because the pane is too small.".into();
            }
        }
        if let Some(observed) = self.rename
            && !self.reveal_for_rename(observed.object)
        {
            self.editor = 0;
            self.rename = None;
            self.edit.set(String::new(), false);
            self.message =
                "Cannot rename in this small view. Enlarge the window and try again.".into();
            let _ = door_sys::write(b"DOOR_P12 FILES RENAME CANCEL layout\n");
        }
    }
    fn view_mode(&self) -> ViewMode {
        if self.overview && self.search.is_empty() {
            self.overview_view
        } else {
            self.view
        }
    }
    /// Items the current view lists: drives on This PC, else the filtered folder.
    fn item_count(&self) -> usize {
        if self.overview && self.search.is_empty() {
            self.drives().len()
        } else {
            self.shown().count()
        }
    }
    /// Position (in view order) of the selected item, if it is listed.
    fn selected_position(&self) -> Option<usize> {
        if self.overview && self.search.is_empty() {
            // A drive is selected while its details are shown (single click).
            (self.detail.is_some() && self.overview_selected < self.drives().len())
                .then_some(self.overview_selected)
        } else {
            self.shown()
                .position(|(index, _)| Some(index) == self.selected)
        }
    }
    /// Scrolls so `position` is visible, keeping the offset step-aligned.
    fn reveal_position(&mut self, position: usize) {
        if self.home_view {
            self.reveal_home(position);
            return;
        }
        if let Some(offset) =
            self.content_layout()
                .reveal_in(position, self.offset, self.view_mode())
        {
            self.offset = offset;
        }
    }
    /// Largest step-aligned offset that still fills the view.
    fn max_offset(&self) -> usize {
        let layout = self.content_layout();
        let (capacity, step) = (
            layout.capacity(self.view_mode()),
            layout.step(self.view_mode()).max(1),
        );
        self.item_count().saturating_sub(capacity).div_ceil(step) * step
    }
    /// Changes the item view; location, selection and details stay, and the
    /// selection stays in view with the new geometry.
    fn set_view(&mut self, mode: ViewMode) {
        let position = self.selected_position();
        if self.overview && self.search.is_empty() {
            self.overview_view = mode;
        } else {
            self.view = mode;
        }
        let step = self.content_layout().step(mode).max(1);
        self.offset = (self.offset / step * step).min(self.max_offset());
        if let Some(position) = position {
            self.reveal_position(position);
        }
        self.thumbnail_scanned = false;
        self.folders_scanned = false;
        self.remember_view();
        let _ = door_sys::write(
            format!("DOOR_P12 FILES VIEW mode={:?} path={}\n", mode, self.cwd).as_bytes(),
        );
    }
    fn command(&mut self, id: u32, storage: &mut Storage) -> Result<(), Error> {
        match id {
            1 => self.new_folder(storage)?,
            2 => self.user_refresh(storage)?,
            3 => self.up(storage)?,
            5..=8 => {
                if self.xmenu.is_some() && matches!(self.menu_for, MenuFor::Command) {
                    self.dismiss_menu();
                } else {
                    self.open_command_menu(id);
                }
            }
            _ => (),
        }
        Ok(())
    }
    pub fn target(&self, x: i32, y: i32, fonts: &FontAtlas) -> Option<u32> {
        self.modern_target(x, y, fonts)
    }
    pub fn new(
        home: String,
        registry: u64,
        directory: u64,
        storage: &mut Storage,
    ) -> Result<Self, Error> {
        let controls = Controls::from_bits(
            storage.get_setting(door_service_client::Setting::DesktopControls)?,
        )
        .ok_or(Error::Malformed)?;
        let mut s = Self::empty(home, controls);
        s.registry = registry;
        s.directory = directory;
        let mut created = false;
        for (folder, _) in FOLDERS {
            match storage.mkdir(&format!("{}/{folder}", s.home)) {
                Ok(()) => created = true,
                Err(Error::Service(Code::Exists)) => (),
                Err(error) => return Err(error),
            }
        }
        if created {
            storage.flush()?;
        }
        s.reload_tags(storage);
        s.load_prefs(storage);
        s.load_menu_profile(storage);
        s.recall_view(THIS_PC);
        s.load_associations(storage);
        s.refresh(storage)?;
        // New windows open to Home, as File Explorer does by default.
        s.history.push(HOME_PAGE.into());
        s.show_home(storage)?;
        Ok(s)
    }
    pub(crate) fn empty(home: String, controls: Controls) -> Self {
        Self {
            registry: 0,
            directory: 0,
            tags: Default::default(),
            tag_filter: None,
            tags_error: String::new(),
            tag_rename: None,
            rail_offset: 0,
            rail_drag: None,
            column_drag: None,
            sidebar_drag: None,
            dnd: Default::default(),
            tree: door_workspace::explorer_tree::Tree::new(),
            rail_key: None,
            pending_close: false,
            pending_launch: None,
            window_request: core::cell::Cell::new(None),
            window_flag: false,
            tabs: Vec::new(),
            active_tab: 0,
            tab_scroll: 0,
            details_visible: false,
            detail_tab: 0,
            detail_scroll: 0,
            detail: None,
            auto_inspect: None,
            pane_hold: None,
            preview_job: None,
            size_job: None,
            size_cache: Vec::new(),
            meta_job: None,
            icon_job: None,
            program_icons: Vec::new(),
            meta_cache: Vec::new(),
            archive: None,
            extract: None,
            convert: None,
            chip_hits: Default::default(),
            detail_height: core::cell::Cell::new(0),
            chevron_cache: Default::default(),
            prefs: Default::default(),
            prefs_dirty: false,
            menu_profile: Default::default(),
            menu_profile_info: None,
            menu_profile_poll: 0,
            prefs_writable: false,
            index: None,
            index_applied: 0,
            result_paths: Vec::new(),
            home_view: false,
            home_scroll: 0,
            home_sections: Vec::new(),
            recent: Default::default(),
            preview_cache: Vec::new(),
            folder_cache: Vec::new(),
            folders_scanned: false,
            last_click: None,
            last_click_bounds: None,
            last_click_point: None,
            thumbnail_scanned: false,
            pane_drag: None,
            pane_focused: false,
            pending_settings: None,
            pending_elevation: None,
            elevation_frame: false,
            xmenu: None,
            menu_for: MenuFor::None,
            background_menu: false,
            crumb_flyout: None,
            cwd: home.clone(),
            home,
            entries: Vec::new(),
            selected: None,
            marked: Vec::new(),
            anchor: None,
            offset: 0,
            preview: String::new(),
            message: String::new(),
            history: Vec::new(),
            history_index: 0,
            nav_generation: 0,
            volume: VolumeState::Unknown,
            layout: ExplorerLayout::new(1000, 668),
            overview: true,
            descending: false,
            sort_key: SortKey::Name,
            columns: DEFAULT_COLUMNS,
            locale: (
                door_locale::tz::DEFAULT_ID as u32,
                door_locale::region::DEFAULT_ID as u32,
            ),
            editor: 0,
            edit: LineEdit::default(),
            search: String::new(),
            overview_selected: 2,
            view: ViewMode::Details,
            overview_view: ViewMode::Medium,
            folders_expanded: true,
            drives_expanded: true,
            controls,
            rename: None,
            removal: Removal::default(),
            removal_name: String::new(),
            removal_error: String::new(),
            nav_focus: None,
            ops: Default::default(),
            apps: Default::default(),
            security: None,
            region_overrides: 0,
            device_name: Default::default(),
        }
    }
    /// The user's time zone and regional format, re-read on every refresh
    /// so a Settings change shows at the next F5 or navigation. Unknown or
    /// unreadable values fall back to UTC / English (United States).
    fn load_locale(&mut self, storage: &mut Storage) {
        let read = |storage: &mut Storage, setting, valid: fn(u32) -> bool, default: u16| {
            storage
                .get_setting(setting)
                .ok()
                .filter(|id| valid(*id))
                .unwrap_or(u32::from(default))
        };
        self.locale = (
            read(
                storage,
                door_service_client::Setting::TimeZone,
                door_locale::tz::valid,
                door_locale::tz::DEFAULT_ID,
            ),
            read(
                storage,
                door_service_client::Setting::RegionalFormat,
                door_locale::region::valid,
                door_locale::region::DEFAULT_ID,
            ),
        );
        // Regional format overrides (24-hour time, custom short and long
        // dates): 0 keeps every format of the region.
        self.region_overrides = storage
            .get_setting(door_service_client::Setting::RegionalOverrides)
            .ok()
            .filter(|w| {
                door_service_protocol::preferences::RegionalOverrides::from_bits(*w).is_some()
            })
            .unwrap_or(0);
        self.load_features(storage);
    }
    /// Optional features (Settings > System > Advanced): NoneDrive adds its
    /// folder under Home (created on first use) and Linux its root after
    /// Network, as Windows shows OneDrive and Linux.
    fn load_features(&mut self, storage: &mut Storage) {
        use door_service_protocol::preferences::{FEATURES, features, valid};
        let bits = storage
            .get_setting(door_service_client::Setting::Features)
            .ok()
            .filter(|w| valid(FEATURES, *w))
            .unwrap_or(0);
        self.tree.nonedrive = (bits & features::NONEDRIVE != 0).then(|| {
            let path = format!("{}/NoneDrive", self.home);
            if storage.stat(&path).is_err() {
                let _ = storage.mkdir(&path);
            }
            path
        });
        self.tree.linux = bits & features::LINUX != 0;
    }
    #[cfg(test)]
    pub(crate) fn show_features_for_test(&mut self, nonedrive: Option<String>, linux: bool) {
        self.tree.nonedrive = nonedrive;
        self.tree.linux = linux;
    }
    /// A volume timestamp in the region's short date and time (with the
    /// user's Regional format overrides), in the user's time zone; empty
    /// when not recorded (0), as Windows leaves missing values blank.
    pub(crate) fn date_text(&self, filetime: u64) -> String {
        if filetime == 0 {
            return String::new();
        }
        let zone = door_locale::tz::zone_or_utc(self.locale.0);
        let region = door_locale::region::region_or_default(self.locale.1).customized(
            door_locale::region::Overrides::from_word(self.region_overrides),
        );
        let mut text = String::new();
        if let Some(local) = zone.to_local(filetime) {
            let _ = region.date_time(&local.time, &mut text);
        }
        text
    }
    fn refresh(&mut self, storage: &mut Storage) -> Result<(), Error> {
        self.load_locale(storage);
        // Default-app choices (Settings may have changed them), like the
        // time zone and region.
        self.load_choices(storage);
        if self.volume_search() {
            // F5 on This PC search results rebuilds the index.
            self.index = None;
            self.apply_results();
            return Ok(());
        }
        if self.home_view {
            // F5 on Home re-reads pins, favorites and the recent list.
            return self.show_home(storage);
        }
        let object = self
            .selected
            .and_then(|i| self.entries.get(i))
            .map(|e| e.info.object);
        let cwd = self.cwd.clone();
        let mut entries = self.list_location(storage, &cwd)?;
        // F5 recounts folder sizes and re-lists expanded tree folders.
        self.size_cache.clear();
        self.size_job = None;
        self.relist_tree(storage);
        self.reload_tags(storage);
        self.hide_tag_metadata(&self.cwd, &mut entries);
        self.entries = entries;
        self.selected = object.and_then(|id| self.entries.iter().position(|e| e.info.object == id));
        self.thumbnail_scanned = false;
        self.folders_scanned = false;
        self.preview_cache.clear();
        self.folder_cache.clear();
        self.folders_scanned = false;
        self.sort();
        self.offset = self.offset.min(self.max_offset());
        self.preview.clear();
        self.detail = None;
        // The status bar derives the count from the active listing.
        self.message.clear();
        if self.overview {
            self.load_volume(storage);
        }
        self.removal.refreshed();
        Ok(())
    }
    fn sort(&mut self) {
        if self.home_view {
            // Home keeps its sections' own order (Recent is newest first).
            return;
        }
        let object = self
            .selected
            .and_then(|index| self.entries.get(index))
            .map(|e| e.info.object);
        let (key, descending) = (self.sort_key, self.descending);
        let kinds: Vec<String> = self
            .entries
            .iter()
            .map(|e| door_workspace::explorer_preview::type_name(&Self::name(e), e.info.directory))
            .collect();
        let item = |i: usize| {
            let e = &self.entries[i];
            explorer_sort::Item {
                name: &e.name[..e.name_len],
                directory: e.info.directory,
                bytes: e.info.bytes,
                kind: &kinds[i],
                modified: e.times.modified,
            }
        };
        let mut order: Vec<usize> = (0..self.entries.len()).collect();
        order.sort_by(|a, b| explorer_sort::compare(key, descending, item(*a), item(*b)));
        if self.result_paths.len() == self.entries.len() && !self.result_paths.is_empty() {
            // Volume search results carry their full paths alongside.
            let paths = core::mem::take(&mut self.result_paths);
            self.result_paths = order.iter().map(|i| paths[*i].clone()).collect();
        }
        let mut slots: Vec<Option<DirEntry>> = core::mem::take(&mut self.entries)
            .into_iter()
            .map(Some)
            .collect();
        self.entries = order.into_iter().filter_map(|i| slots[i].take()).collect();
        self.selected = object.and_then(|id| self.entries.iter().position(|e| e.info.object == id));
        if !self
            .selected
            .is_some_and(|index| self.shown().any(|(i, _)| i == index))
        {
            let first = self.shown().next().map(|(index, _)| index);
            self.selected = first;
        }
    }
    fn shown(&self) -> impl Iterator<Item = (usize, &DirEntry)> {
        self.entries.iter().enumerate().filter(|(index, entry)| {
            // Collapsed Home sections hide their items.
            !self
                .home_section(*index)
                .is_some_and(|section| self.home_collapsed(section))
                && self
                    .tag_filter
                    .is_none_or(|tag| self.tags.tags(entry.info.object) & (1 << tag) != 0)
                && (self.search.is_empty()
                    || entry.name[..entry.name_len]
                        .windows(self.search.len())
                        .any(|units| {
                            units.iter().zip(self.search.bytes()).all(|(unit, ch)| {
                                *unit <= 127 && (*unit as u8).eq_ignore_ascii_case(&ch)
                            })
                        }))
        })
    }
    /// Whether a listed item shows in folder views: Windows Explorer's
    /// default ("Hidden items" off) leaves out FILE_ATTRIBUTE_HIDDEN items
    /// such as the profile's AppData. Typed paths still reach them.
    fn visible(entry: &DirEntry) -> bool {
        entry.info.attributes & 0x2 == 0
    }
    fn read_entries(storage: &mut Storage, path: &str) -> Result<Vec<DirEntry>, Error> {
        let mut entries = Vec::new();
        let mut cookie = 0;
        while entries.len() < 128 {
            let Some(entry) = storage.list(path, cookie)? else {
                break;
            };
            cookie = entry.cookie;
            // The per-user `$Recycle.Bin` (hidden) is never an ordinary
            // folder; the Recycle Bin view shows its contents.
            if door_workspace::file_operations::hidden_name(&Self::name(&entry)) {
                continue;
            }
            entries.push(entry);
        }
        if path == "/compat" {
            // Mirror the existing storage Windows resolver's Users junction.
            // The metadata is the real authenticated /users directory object.
            let (info, times) = storage.stat("/users")?;
            let mut name = [0; door_service_protocol::files::NAME_UNITS];
            name[..5].copy_from_slice(&[85, 115, 101, 114, 115]);
            entries.push(DirEntry {
                cookie: info.object,
                name,
                name_len: 5,
                info,
                times,
            });
        }
        Ok(entries)
    }
    fn address(&mut self, storage: &mut Storage) -> Result<(), Error> {
        if self.edit.text().trim().eq_ignore_ascii_case("Home") {
            return self.go_home(storage);
        }
        let typed = self.edit.text().trim();
        if typed.eq_ignore_ascii_case("Recycle Bin")
            || typed.eq_ignore_ascii_case("shell:RecycleBinFolder")
            || typed == RECYCLE_BIN
        {
            self.go_recycle_bin(storage)?;
            let _ = door_sys::write(
                format!(
                    "DOOR_P12 FILES ADDRESS path={} history={}/{}\n",
                    self.cwd,
                    self.history_index,
                    self.history.len()
                )
                .as_bytes(),
            );
            return Ok(());
        }
        // A virtual location (the Recycle Bin's `::RecycleBin`) has no
        // folder to resolve relative input against: use the profile folder,
        // so an absolute path typed there opens as anywhere else.
        let base = if self.cwd.starts_with('/') {
            self.cwd.clone()
        } else {
            self.home.clone()
        };
        let plan = explorer_path::parse(self.edit.text(), &base, &self.home).map_err(|error| {
            Error::Service(match error {
                explorer_path::AddressError::Invalid => Code::InvalidRequest,
                explorer_path::AddressError::Unsupported => Code::Unsupported,
            })
        })?;
        let Address::Directory {
            mut start,
            parts,
            windows,
        } = plan
        else {
            return self.go_this_pc(storage);
        };
        let mut entries = Self::read_entries(storage, &start)?;
        let count = parts.len();
        for (index, part) in parts.into_iter().enumerate() {
            if part == "." {
                continue;
            }
            // A typed file path (the last component, an existing file)
            // opens it as a double-click does: a program runs with its
            // folder as working directory, a package goes to Windows
            // Installer, a document to its default app.
            if index + 1 == count
                && let Some(entry) = entries.iter().find(|entry| {
                    !entry.info.directory
                        && String::from_utf16(&entry.name[..entry.name_len]).is_ok_and(|name| {
                            if windows {
                                name.eq_ignore_ascii_case(&part)
                            } else {
                                name == part
                            }
                        })
                })
            {
                let name = Self::name(entry);
                let path = if start == "/" {
                    format!("/{name}")
                } else {
                    format!("{start}/{name}")
                };
                if is_program(&name) || is_package(&name) {
                    return self.run_checked(storage, &path);
                }
                self.open_file(storage, &path);
                return Ok(());
            }
            let next = if part == ".." {
                explorer_path::parent(&start, windows)
            } else if windows && start == "/compat" && part.eq_ignore_ascii_case("Users") {
                "/users".into()
            } else {
                let entry = entries
                    .iter()
                    .find(|entry| {
                        entry.info.directory
                            && String::from_utf16(&entry.name[..entry.name_len]).is_ok_and(|name| {
                                if windows {
                                    name.eq_ignore_ascii_case(&part)
                                } else {
                                    name == part
                                }
                            })
                    })
                    .ok_or(Error::Service(Code::NotFound))?;
                let name = String::from_utf16(&entry.name[..entry.name_len])
                    .map_err(|_| Error::Service(Code::Unsupported))?;
                format!(
                    "{}{name}",
                    if start == "/" {
                        "/".into()
                    } else {
                        format!("{start}/")
                    }
                )
            };
            // No lexical '..' cancellation: every predecessor is enumerated
            // under the authenticated Storage identity before advancing.
            entries = Self::read_entries(storage, &next)?;
            start = next;
        }
        self.navigate_entries(start, entries);
        let _ = door_sys::write(
            format!(
                "DOOR_P12 FILES ADDRESS path={} history={}/{}\n",
                self.cwd,
                self.history_index,
                self.history.len()
            )
            .as_bytes(),
        );
        Ok(())
    }
    fn navigate(&mut self, storage: &mut Storage, path: String) -> Result<(), Error> {
        if path == RECYCLE_BIN {
            return self.go_recycle_bin(storage);
        }
        let entries = Self::read_entries(storage, &path)?;
        self.navigate_entries(path, entries);
        Ok(())
    }
    /// The single entry point for a user-visible location change: one history
    /// record (deduplicated against the current one) followed by `install`.
    fn navigate_entries(&mut self, path: String, entries: Vec<DirEntry>) {
        self.push_history(&path);
        self.install(path, entries);
    }
    fn push_history(&mut self, path: &str) {
        if self
            .history
            .get(self.history_index)
            .is_some_and(|current| current == path)
        {
            return;
        }
        self.history.truncate(self.history_index + 1);
        if self.history.len() == 32 {
            self.history.remove(0);
        }
        self.history.push(path.into());
        self.history_index = self.history.len() - 1;
    }
    fn go_this_pc(&mut self, storage: &mut Storage) -> Result<(), Error> {
        let entries = Self::read_entries(storage, &self.home)?;
        self.show_this_pc(entries);
        self.load_volume(storage);
        Ok(())
    }
    /// This PC lists the home directory but is a distinct location.
    fn show_this_pc(&mut self, entries: Vec<DirEntry>) {
        self.push_history(THIS_PC);
        self.install(self.home.clone(), entries);
        self.overview = true;
        self.recall_view(THIS_PC);
    }
    pub(crate) fn history_label(path: &str) -> String {
        if path == THIS_PC {
            "This PC".into()
        } else if path == HOME_PAGE {
            "Home".into()
        } else if path == RECYCLE_BIN {
            "Recycle Bin".into()
        } else {
            explorer_path::display(path)
        }
    }
    pub(crate) fn location_error(error: &Error) -> &'static str {
        match error {
            Error::Service(Code::NotFound | Code::DeletePending | Code::StaleInstance) => {
                "This location no longer exists"
            }
            Error::Service(Code::Denied) => "Access denied",
            Error::Service(Code::InvalidRequest) => "This is not a valid location",
            Error::Service(Code::Unsupported) => "This location is not supported",
            _ => "Cannot open this location",
        }
    }
    fn install(&mut self, path: String, mut entries: Vec<DirEntry>) {
        if path != RECYCLE_BIN {
            entries.retain(Self::visible);
        }
        self.home_view = false;
        self.home_sections.clear();
        self.home_scroll = 0;
        // A pending preview job belongs to the previous location: the bumped
        // generation makes the next `preview_step` close it instead of
        // caching its result.
        self.nav_generation = self.nav_generation.wrapping_add(1);
        self.preview_cache.clear();
        // Folder contents stay cached per folder (`CachedFolder`).
        self.hide_tag_metadata(&path, &mut entries);
        self.last_click = None;
        self.last_click_bounds = None;
        self.last_click_point = None;
        self.detail_scroll = 0;
        self.thumbnail_scanned = false;
        self.folders_scanned = false;
        self.tag_filter = None;
        self.marked.clear();
        self.anchor = None;
        self.cwd = path;
        self.entries = entries;
        self.result_paths.clear();
        self.overview = false;
        self.search.clear();
        let key = self.cwd.clone();
        self.recall_view(&key);
        self.sort();
        self.editor = 0;
        self.rename = None;
        self.selected = None;
        self.offset = 0;
        self.thumbnail_scanned = false;
        self.folders_scanned = false;
        self.preview.clear();
        self.detail = None;
        // The status bar derives the count from the active listing.
        self.message.clear();
        self.removal.refreshed();
    }
    fn history(&mut self, storage: &mut Storage, forward: bool) -> Result<(), Error> {
        let index = if forward {
            (self.history_index + 1).min(self.history.len().saturating_sub(1))
        } else {
            self.history_index.saturating_sub(1)
        };
        self.jump_history(storage, index)
    }
    fn jump_history(&mut self, storage: &mut Storage, index: usize) -> Result<(), Error> {
        if let Some(path) = self
            .history
            .get(index)
            .filter(|_| index != self.history_index)
            .cloned()
        {
            if path == HOME_PAGE {
                self.history_index = index;
                self.show_home(storage)?;
                door_rt::diagnostic(format_args!(
                    "DOOR_P12 FILES HISTORY index={index} count={} path={}
",
                    self.history.len(),
                    self.cwd
                ));
                return Ok(());
            }
            let this_pc = path == THIS_PC;
            let listed = if this_pc { self.home.clone() } else { path };
            let entries = match self.list_location(storage, &listed) {
                Ok(entries) => entries,
                Err(error) => {
                    // Keep the current location intact; the history entry
                    // stays so the user can retry once it is reachable.
                    self.message = Self::location_error(&error).into();
                    let _ = door_sys::write(
                        format!("DOOR_P12 FILES HISTORY REJECT index={index} error={error:?}\n")
                            .as_bytes(),
                    );
                    return Ok(());
                }
            };
            self.history_index = index;
            self.install(listed, entries);
            self.overview = this_pc;
            let _ = door_sys::write(
                format!(
                    "DOOR_P12 FILES HISTORY index={index} count={} path={}\n",
                    self.history.len(),
                    self.cwd
                )
                .as_bytes(),
            );
        }
        Ok(())
    }
    fn name(entry: &DirEntry) -> String {
        String::from_utf16_lossy(&entry.name[..entry.name_len])
    }
    /// Opens a Windows executable as File Explorer does: Init EXECUTE starts
    /// it as the signed-in user, in the session, without a console, with its
    /// folder as working directory. Init can terminate it (hung windows).
    fn run_program(&mut self, path: &str) -> Result<(), Error> {
        if is_package(path) {
            return self.run_installer(path, "/i");
        }
        if self.registry == 0 {
            return Err(Error::Service(Code::Unsupported));
        }
        let folder = match path.rsplit_once('/') {
            Some(("", _)) => "/",
            Some((folder, _)) => folder,
            None => return Err(Error::Service(Code::InvalidRequest)),
        };
        // The program's command line names it by its Windows path, as
        // Windows' Explorer starts it (GetModuleFileName returns C:\...).
        let mut arguments = Vec::from(explorer_path::display(path).as_bytes());
        arguments.push(0);
        let mut applications = door_service_client::Applications::connect(self.registry)?;
        let child = applications.execute_detached(path, folder, &arguments, self.directory)?;
        let _ = door_sys::close(child);
        self.record_open(path, false);
        self.editor = 0;
        let _ = door_sys::write(format!("DOOR_P12 FILES LAUNCH path={path}\n").as_bytes());
        Ok(())
    }
    /// A package's verb as its file type registers it: msiexec.exe with
    /// `/i`, `/f` or `/x` and the package's Windows path, in the package's
    /// folder.
    pub(super) fn run_installer(&mut self, path: &str, verb: &str) -> Result<(), Error> {
        if self.registry == 0 {
            return Err(Error::Service(Code::Unsupported));
        }
        let folder = match path.rsplit_once('/') {
            Some(("", _)) => "/",
            Some((folder, _)) => folder,
            None => return Err(Error::Service(Code::InvalidRequest)),
        };
        let package = explorer_path::display(path);
        let mut arguments = Vec::new();
        for word in [MSIEXEC, verb, package.as_str()] {
            arguments.extend_from_slice(word.as_bytes());
            arguments.push(0);
        }
        let mut applications = door_service_client::Applications::connect(self.registry)?;
        let child = applications.execute_detached(MSIEXEC, folder, &arguments, self.directory)?;
        let _ = door_sys::close(child);
        self.record_open(path, false);
        self.editor = 0;
        let _ = door_sys::write(
            format!("DOOR_P16 FILES INSTALLER verb={verb} path={path}\n").as_bytes(),
        );
        Ok(())
    }
    /// Run as administrator: the window server, not File Explorer, verifies
    /// the program's signature, asks with User Account Control and starts
    /// it, so File Explorer only hands over the path and the bytes.
    fn run_as_administrator(&mut self, path: &str, storage: &mut Storage) {
        if is_script(path) {
            self.pending_elevation = Some((String::from(path), true, Vec::new()));
            return;
        }
        let read = (|| -> Result<Vec<u8>, Error> {
            let file = storage.open(path, Disposition::OpenExisting, READ, SHARE_ALL)?;
            let result = (|| {
                let info = storage.info(&file)?;
                if info.bytes == 0 || info.bytes > door_abi::platform::EXECUTABLE_MAX_BYTES as u64 {
                    return Err(Error::Service(Code::Resources));
                }
                let mut data = alloc::vec![0; info.bytes as usize];
                let mut at = 0;
                while at < data.len() {
                    let end = (at + door_service_protocol::CHUNK_BYTES).min(data.len());
                    let n = storage.read(&file, at as u64, &mut data[at..end])?;
                    if n == 0 {
                        return Err(Error::Service(Code::Io));
                    }
                    at += n;
                }
                Ok(data)
            })();
            let closed = storage.close(file);
            let data = result?;
            closed?;
            Ok(data)
        })();
        match read {
            Ok(image) => self.pending_elevation = Some((String::from(path), false, image)),
            Err(Error::Service(Code::Resources)) => {
                self.message = String::from("This app is too large to run on this PC.");
            }
            Err(error) => {
                self.message = format!(
                    "Couldn't run as administrator: {}",
                    Self::location_error(&error)
                )
            }
        }
    }
    /// Hand a pending Run as administrator to the window server.
    pub(crate) fn send_elevation(
        &mut self,
        c: &mut door_service_client::window::WindowClient,
    ) -> Result<bool, Error> {
        if self.pending_elevation.is_none() {
            return Ok(false);
        }
        if !core::mem::replace(&mut self.elevation_frame, true) {
            // Repaint without the menu first.
            return Ok(true);
        }
        self.elevation_frame = false;
        let Some((path, script, image)) = self.pending_elevation.take() else {
            return Ok(false);
        };
        let flags = if script {
            door_service_protocol::window::elevation::SCRIPT
        } else {
            0
        };
        let section = if script {
            None
        } else {
            Some(door_workspace::section_from(&image).map_err(Error::from)?)
        };
        let sent = c.request_elevation(
            &path,
            flags,
            section.map(|s| (s, image.len() as u32)),
            self.directory,
        );
        if let Some(section) = section {
            let _ = door_sys::close(section);
        }
        match sent {
            Ok(()) => {
                let _ = door_sys::write(
                    format!(
                        "DOOR_P16 FILES RUNAS path={path}
"
                    )
                    .as_bytes(),
                );
                Ok(true)
            }
            Err(Error::Service(Code::WouldBlock)) => Ok(false),
            Err(error) => Err(error),
        }
    }
    fn activate(&mut self, storage: &mut Storage) -> Result<(), Error> {
        let Some(index) = self
            .selected
            .filter(|index| self.shown().any(|(i, _)| i == *index))
        else {
            return Ok(());
        };
        let Some(entry) = self.entries.get(index) else {
            return Ok(());
        };
        if self.recycle_view() {
            // Opening a deleted item shows its properties, as in Windows.
            self.recycle_properties(true, storage);
            return Ok(());
        }
        let path = self
            .path_for(entry)
            .ok_or(Error::Service(Code::Unsupported))?;
        if entry.info.directory {
            self.navigate(storage, path)
        } else if is_program(&Self::name(entry)) || is_package(&Self::name(entry)) {
            self.run_checked(storage, &path)
        } else {
            // Opening a file hands it to its default app (or asks which app
            // to use); a started app adds it to Recent.
            self.open_file(storage, &path);
            Ok(())
        }
    }

    fn up(&mut self, storage: &mut Storage) -> Result<(), Error> {
        // This PC and Home are roots of the navigation namespace.
        if self.cwd != "/" && !self.overview && !self.home_view && !self.recycle_view() {
            let windows = explorer_path::display(&self.cwd).starts_with("C:");
            self.navigate(storage, explorer_path::parent(&self.cwd, windows))?;
        }
        Ok(())
    }
    fn rename_candidate(&self) -> Option<&DirEntry> {
        // This PC lists drives; its search results live in other folders.
        if self.overview || self.recycle_view() {
            return None;
        }
        let index = self.selected?;
        let entry = self.entries.get(index)?;
        if entry.info.object <= 16 || !self.shown().any(|(i, _)| i == index) {
            return None;
        }
        // Known-folder relocation needs a provider that updates the persistent
        // associations. Do not silently strand pinned standard locations.
        if self.cwd == self.home
            && FOLDERS.iter().any(|(name, _)| {
                entry.name[..entry.name_len]
                    .iter()
                    .copied()
                    .eq(name.encode_utf16())
            })
        {
            return None;
        }
        Some(entry)
    }
    fn begin_rename(&mut self, storage: &mut Storage) {
        self.dismiss_menu();
        let Some(entry) = self.rename_candidate().copied() else {
            return;
        };
        if !self.reveal_for_rename(entry.info.object) {
            self.message =
                "Cannot rename in this small view. Enlarge the window and try again.".into();
            let _ = door_sys::write(b"DOOR_P12 FILES RENAME REJECT not_visible\n");
            return;
        }
        let name = match door_workspace::file_name::editable_name(&entry.name[..entry.name_len]) {
            Ok(name) => name,
            Err(error) => {
                self.message = format!("Cannot rename: {}", error.message());
                return;
            }
        };
        let parent = match storage.metadata(&self.cwd) {
            Ok(info) if info.directory => info.object,
            Ok(_) => {
                self.message =
                    "Cannot rename: the folder has changed. Refresh and try again.".into();
                return;
            }
            Err(error) => {
                self.message = format!("Cannot rename: {error:?}");
                return;
            }
        };
        self.start_rename(entry, name, parent);
    }
    /// The inline editor over a revealed, renamable entry of folder `parent`.
    fn start_rename(&mut self, entry: DirEntry, name: String, parent: u64) {
        self.rename = Some(ObservedRename {
            parent,
            object: entry.info.object,
            version: entry.info.version,
        });
        self.editor = 3;
        let selected = door_workspace::file_name::initial_selection(&name, entry.info.directory);
        self.edit.set(name, false);
        self.edit.place(selected.start, false);
        self.edit.place(selected.end, true);
        // The details pane keeps describing the item being renamed.
        self.preview.clear();
        self.message = "Type a new name. Enter to save; Esc to cancel.".into();
        let _ = door_sys::write(
            format!(
                "DOOR_P12 FILES RENAME BEGIN object={} version={}\n",
                entry.info.object, entry.info.version
            )
            .as_bytes(),
        );
    }
    /// Shift+Delete on the selection: the permanent-delete confirmation.
    fn begin_remove(&mut self, storage: &mut Storage) {
        self.delete_selection(true, storage);
    }
    fn choose_remove(&mut self, choice: usize, storage: &mut Storage) {
        self.confirm_permanent(choice, storage);
    }
    fn finish_rename(&mut self, storage: &mut Storage) {
        let Some(observed) = self.rename else {
            return;
        };
        let name = match door_workspace::file_name::component(self.edit.text()) {
            Ok(name) => name,
            Err(error) => {
                self.message = format!("Cannot rename: {}", error.message());
                let _ = door_sys::write(b"DOOR_P12 FILES RENAME REJECT invalid_name\n");
                return;
            }
        };
        if let Err(error) =
            storage.rename_child_checked(observed.parent, observed.object, observed.version, &name)
        {
            self.message = match error {
                Error::Service(Code::Changed | Code::NotFound) => {
                    "Cannot rename: the item changed. Press Esc, refresh, and try again.".into()
                }
                Error::Service(Code::Exists) => {
                    "Cannot rename: an item with this name already exists.".into()
                }
                Error::Service(Code::Denied) => "Cannot rename: access is denied.".into(),
                Error::Service(Code::SharingViolation) => {
                    "Cannot rename: another app is using this item.".into()
                }
                _ => "Cannot confirm rename. Reopen this folder before retrying.".into(),
            };
            let _ = door_sys::write(
                format!(
                    "DOOR_P12 FILES RENAME REJECT object={} error={error:?}\n",
                    observed.object
                )
                .as_bytes(),
            );
            return;
        }
        self.editor = 0;
        self.rename = None;
        if self.refresh(storage).is_ok() {
            let renamed = self
                .entries
                .iter()
                .position(|entry| entry.info.object == observed.object);
            self.selected =
                renamed.filter(|wanted| self.shown().any(|(index, _)| index == *wanted));
            if self.selected.is_none() {
                let first = self.shown().next().map(|(index, _)| index);
                self.selected = first;
            }
            let position = self
                .shown()
                .position(|(index, _)| Some(index) == self.selected);
            if let Some(position) = position {
                self.reveal_position(position);
            }
        } else {
            self.entries.clear();
            self.selected = None;
            self.message =
                "Renamed. This folder could not be refreshed; reopen its current location.".into();
        }
        if self.details_visible && self.selected.is_some() {
            let _ = self.inspect_selection(storage);
        }
        let _ = door_sys::write(
            format!(
                "DOOR_P12 FILES RENAME COMMITTED object={} parent={}\n",
                observed.object, observed.parent
            )
            .as_bytes(),
        );
    }
    fn reveal_object(&mut self, object: u64) -> bool {
        let position = self
            .shown()
            .position(|(_, entry)| entry.info.object == object);
        let Some(offset) = position.and_then(|position| {
            self.content_layout()
                .reveal_in(position, self.offset, self.view)
        }) else {
            return false;
        };
        self.offset = offset;
        true
    }
    /// Reveals `object` for inline renaming: false when it cannot be shown
    /// or its editor would not fit (a view too short or too narrow).
    fn reveal_for_rename(&mut self, object: u64) -> bool {
        if !self.reveal_object(object) {
            return false;
        }
        let layout = self.content_layout();
        self.shown()
            .skip(self.offset)
            .take(layout.capacity(self.view))
            .position(|(_, entry)| entry.info.object == object)
            .and_then(|row| layout.rename_in(row, self.view))
            .is_some()
    }
    fn rename_bounds(&self) -> Option<Bounds> {
        let observed = self.rename?;
        let row = self
            .shown()
            .skip(self.offset)
            .take(self.content_layout().capacity(self.view))
            .position(|(_, entry)| entry.info.object == observed.object)?;
        self.content_layout().rename_in(row, self.view)
    }
    /// Why the selected item cannot be renamed or deleted here, if it cannot.
    pub(super) fn rename_block(&self) -> Option<&'static str> {
        if self.rename_candidate().is_some() {
            return None;
        }
        let known = self.cwd == self.home
            && self
                .selected
                .and_then(|i| self.entries.get(i))
                .is_some_and(|entry| {
                    FOLDERS.iter().any(|(name, _)| {
                        entry.name[..entry.name_len]
                            .iter()
                            .copied()
                            .eq(name.encode_utf16())
                    })
                });
        Some(if self.recycle_view() {
            "Restore the item to rename it."
        } else if self.volume_search() {
            "Open the item's folder to rename it."
        } else if known {
            "Known folders keep their names and locations."
        } else {
            "This item is protected by the system."
        })
    }
    /// New > Text Document: an empty "New Text Document.txt" (unique name),
    /// selected and opened for renaming, as Windows does.
    fn new_text_document(&mut self, storage: &mut Storage) -> Result<(), Error> {
        if self.overview || self.home_view || self.recycle_view() {
            return Ok(());
        }
        let taken: Vec<String> = self.entries.iter().map(Self::name).collect();
        let name = archive::unique_name("New Text Document", ".txt", &taken);
        let path = if self.cwd == "/" {
            format!("/{name}")
        } else {
            format!("{}/{name}", self.cwd)
        };
        let file = storage.open(
            &path,
            Disposition::CreateNew,
            READ | door_service_client::WRITE,
            SHARE_ALL,
        )?;
        storage.close(file)?;
        storage.flush()?;
        self.refresh(storage)?;
        if let Some(i) = self.entries.iter().position(|e| Self::name(e) == name) {
            self.selected = Some(i);
            self.marked.clear();
            self.begin_rename(storage);
        }
        door_rt::diagnostic(format_args!("DOOR_P12 FILES NEW_TEXT name={name}\n"));
        Ok(())
    }
    fn new_folder(&mut self, storage: &mut Storage) -> Result<(), Error> {
        if self.recycle_view() {
            return Ok(());
        }
        for n in 1..=128 {
            let path = format!("{}/New folder {n}", self.cwd);
            match storage.mkdir(&path) {
                Ok(()) => {
                    storage.flush()?;
                    // From This PC or Home the folder lands in the profile
                    // folder, which is then shown.
                    self.overview = false;
                    if self.home_view {
                        self.home_view = false;
                        self.home_sections.clear();
                        self.result_paths.clear();
                    }
                    return self.refresh(storage);
                }
                Err(Error::Service(Code::Exists)) => (),
                Err(e) => return Err(e),
            }
        }
        Err(Error::Service(Code::Resources))
    }
    pub fn key(&mut self, code: u32, storage: &mut Storage) -> Result<bool, Error> {
        let scan = code & 511;
        if matches!(self.editor, 4 | 6) {
            return self.tag_key(code, storage);
        }
        if self.editor == 5 {
            return self.tag_rename_key(code, storage);
        }
        if self.editor == 0
            && self.pane_focused
            && matches!(scan, 0x149 | 0x151)
            && self.details_visible
        {
            self.scroll_pane(scan == 0x151);
            return Ok(true);
        }
        if self.modal() {
            let shift = (code >> 9) & door_abi::graphics::SHIFT != 0;
            if self.security_open() {
                self.security_key(storage, code);
            } else if self.picker_open() {
                self.picker_key(storage, code);
            } else if !self.removal.dialog.open() {
                self.operations_key(code, storage);
            } else if let Some(choice) = self.removal.dialog.key(scan, shift) {
                self.choose_remove(choice, storage);
            }
            return Ok(true);
        }
        if self.xmenu.is_some() {
            self.menu_key(scan, storage)?;
            return Ok(true);
        }
        let modifiers =
            door_abi::graphics::CONTROL | door_abi::graphics::SHIFT | door_abi::graphics::ALT;
        if self.editor == 0 && (code >> 9) & modifiers == 0 && self.progress_key(scan, storage) {
            return Ok(true);
        }
        if let Some(done) = self.navigation_key(code, storage)? {
            return Ok(done);
        }
        let control = (code >> 9) & door_abi::graphics::CONTROL != 0;
        let shift = (code >> 9) & door_abi::graphics::SHIFT != 0;
        let alt = (code >> 9) & door_abi::graphics::ALT != 0;
        if self.editor != 3 && control && !alt && scan == 20 {
            // New tabs open to Home, as in File Explorer.
            self.new_tab(storage, HOME_PAGE.into())?;
            return Ok(true);
        }
        if self.editor != 3 && control && !alt && scan == 15 {
            self.switch_tab(
                (self.active_tab + if shift { self.tab_count() - 1 } else { 1 }) % self.tab_count(),
                storage,
            )?;
            return Ok(true);
        }
        if self.editor != 3 && control && !alt && scan == 17 {
            self.close_tab(self.active_tab, storage)?;
            return Ok(true);
        }
        if self.editor != 3 && alt && !control && scan == 28 {
            self.inspect_selection(storage)?;
            self.details_visible = true;
            self.detail_tab = 0;
            return Ok(true);
        }
        if self.editor != 3 && alt && !control && scan == 25 {
            let mode = if shift { 0 } else { 1 };
            self.details_visible = !(self.details_visible && self.detail_tab == mode);
            self.detail_tab = mode;
            if self.details_visible {
                self.inspect_selection(storage)?;
            }
            return Ok(true);
        }
        if self.editor == 0 && scan == 1 && self.extract.is_some() && self.search.is_empty() {
            return Ok(self.cancel_extract(storage));
        }
        if self.editor == 0 && scan == 1 && self.details_visible && self.search.is_empty() {
            self.details_visible = false;
            return Ok(true);
        }
        if self.editor == 0 && scan == 0x153 && shift && !control && !alt {
            self.begin_remove(storage);
            return Ok(true);
        }
        // Delete (and Ctrl+D) recycles; Ctrl+X/C/V cut, copy and paste;
        // Ctrl+Shift+C copies the paths.
        if self.editor == 0
            && !alt
            && !shift
            && (scan == 0x153 && !control || control && scan == 32)
        {
            self.delete_selection(false, storage);
            return Ok(true);
        }
        if self.editor == 0 && control && !alt && matches!(scan, 45..=47) {
            match (scan, shift) {
                (45, false) => self.copy_selection(true),
                (46, false) => self.copy_selection(false),
                (46, true) => self.copy_as_path(),
                (47, false) => self.paste(),
                _ => return Ok(false),
            }
            return Ok(true);
        }
        if self.editor == 3 && matches!(scan, 15 | 28 | 0x11c) {
            self.finish_rename(storage);
            return Ok(true);
        }
        if self.editor != 3 && control && shift && !alt && (2..=9).contains(&scan) {
            self.editor = 0;
            self.set_view(ViewMode::from_index(scan as usize - 2).unwrap_or_default());
            return Ok(true);
        }
        if self.editor != 3 && alt && !control && matches!(scan, 0x148 | 0x14b | 0x14d) {
            return self.global_key(scan, control, shift, alt, storage);
        }
        if scan == 1 && self.editor == 0 && !self.search.is_empty() {
            let volume = self.volume_search();
            self.search.clear();
            if volume {
                self.end_volume_search(storage);
            }
            self.thumbnail_scanned = false;
            self.folders_scanned = false;
            self.sort();
            self.offset = 0;
            self.inspect_selection(storage)?;
            return Ok(true);
        }
        if self.editor != 3 && (control && scan == 38 || alt && scan == 32 || scan == 0x3e && !alt)
        {
            self.edit_address();
            return Ok(true);
        }
        if self.editor == 0 && control && !alt && !shift && scan == 30 {
            self.select_all();
            return Ok(true);
        }
        if self.editor != 3 && (control && matches!(scan, 33 | 18) || scan == 0x3d) {
            self.editor = 2;
            self.edit.set(self.search.clone(), true);
            return Ok(true);
        }
        if self.editor != 0 {
            if control && scan == 30 {
                self.edit.select_all();
                return Ok(true);
            }
            match scan {
                1 => {
                    self.editor = 0;
                    self.rename = None;
                    // The status bar derives the count from the active listing.
                    self.message.clear();
                    self.edit.set(String::new(), false);
                }
                14 => {
                    if control && self.edit.selection().is_empty() {
                        self.edit.horizontal(false, true, true);
                    }
                    self.edit.backspace();
                }
                0x153 => {
                    if control && self.edit.selection().is_empty() {
                        self.edit.horizontal(true, true, true);
                    }
                    self.edit.delete();
                }
                0x14b => self.edit.horizontal(false, shift, control),
                0x14d => self.edit.horizontal(true, shift, control),
                0x147 => self.edit.place(0, shift),
                0x14f => self.edit.place(self.edit.text().len(), shift),
                15 => {
                    self.editor = if self.editor == 1 { 2 } else { 1 };
                    let text = if self.editor == 1 {
                        explorer_path::display(&self.cwd)
                    } else {
                        self.search.clone()
                    };
                    self.edit.set(text, true);
                }
                28 | 0x11c => {
                    if self.editor == 1 {
                        if let Err(error) = self.address(storage) {
                            // The current location stays valid; only the
                            // typed text was rejected, so keep its listing.
                            self.message = Self::location_error(&error).into();
                            let _ = door_sys::write(
                                format!(
                                    "DOOR_P12 FILES ADDRESS REJECT path={} history={}/{} error={error:?}\n",
                                    self.cwd, self.history_index, self.history.len()
                                )
                                .as_bytes(),
                            );
                            return Ok(true);
                        }
                    } else {
                        let was_volume = self.volume_search();
                        self.search = self.edit.text().into();
                        // The status bar derives the count from the active listing.
                        self.message.clear();
                        self.offset = 0;
                        if self.volume_search() {
                            // This PC searches every mounted volume through the index.
                            self.apply_results();
                        } else if was_volume {
                            self.end_volume_search(storage);
                        }
                        let first = self.shown().next().map(|(index, _)| index);
                        self.selected = first;
                        self.thumbnail_scanned = false;
                        self.folders_scanned = false;
                        self.inspect_selection(storage)?;
                    }
                    self.editor = 0;
                }
                _ => {
                    let ch = (code >> 16) as u8;
                    if !control && !alt && (32..=126).contains(&ch) {
                        self.edit.insert(ch as char);
                    }
                }
            }
            return Ok(true);
        }
        if self.overview && self.search.is_empty() && !control && !alt {
            let Some(last) = self.drives().len().checked_sub(1) else {
                return self.global_key(scan, control, shift, alt, storage);
            };
            let layout = self.content_layout();
            let (across, down) = layout.arrow_steps(self.overview_view);
            let page = layout.capacity(self.overview_view).max(1);
            let at = self.overview_selected.min(last);
            self.overview_selected = match scan {
                0x14b => at.saturating_sub(across as usize),
                0x14d => (at + across as usize).min(last),
                0x148 => at.saturating_sub(down as usize),
                0x150 => (at + down as usize).min(last),
                0x147 => 0,
                0x14f => last,
                0x149 => at.saturating_sub(page),
                0x151 => (at + page).min(last),
                28 | 0x11c => {
                    let path = self.drives()[at].path.into();
                    self.navigate(storage, path)?;
                    return Ok(true);
                }
                _ => return self.global_key(scan, control, shift, alt, storage),
            };
            if matches!(
                scan,
                0x14b | 0x14d | 0x148 | 0x150 | 0x147 | 0x14f | 0x149 | 0x151
            ) {
                self.reveal_overview();
                self.inspect_selection(storage)?;
            }
            return Ok(true);
        }
        self.global_key(scan, control, shift, alt, storage)
    }
    fn global_key(
        &mut self,
        scan: u32,
        control: bool,
        shift: bool,
        alt: bool,
        storage: &mut Storage,
    ) -> Result<bool, Error> {
        let navigating = !control
            && !alt
            && matches!(
                scan,
                0x148 | 0x150 | 0x14b | 0x14d | 0x147 | 0x14f | 0x149 | 0x151
            );
        if navigating {
            if shift {
                if self.anchor.is_none() {
                    self.anchor = self
                        .selected
                        .and_then(|i| self.entries.get(i))
                        .map(|e| e.info.object);
                }
            } else {
                self.anchor = None;
                self.marked.clear();
            }
        }
        match scan {
            0x148 if alt => self.up(storage)?,
            0x14b if alt => self.history(storage, false)?,
            0x14d if alt => self.history(storage, true)?,
            // Arrows follow the view's flow: rows for grids, columns for List;
            // Home walks its sections in order.
            0x150 | 0x148 | 0x14d | 0x14b
                if matches!(scan, 0x150 | 0x148)
                    || self.home_view
                    || self.content_layout().arrow_steps(self.view).0 != 0 =>
            {
                let (across, down) = if self.home_view {
                    (1, 1)
                } else {
                    self.content_layout().arrow_steps(self.view)
                };
                let delta = match scan {
                    0x150 => down,
                    0x148 => -down,
                    0x14d => across,
                    _ => -across,
                };
                self.select(delta as i32, false)
            }
            28 | 0x11c => self.activate(storage)?,
            14 if !control && !alt => self.history(storage, false)?,
            63 => self.user_refresh(storage)?,
            60 if !control && !alt && !shift => self.begin_rename(storage),
            19 if control && !alt => self.user_refresh(storage)?,
            49 if control && shift && !alt => self.new_folder(storage)?,
            0x147 => self.select(-1, true),
            0x14f => self.select(1, true),
            0x149 => self.select(
                -(self.content_layout().capacity(self.view).max(1) as i32),
                false,
            ),
            0x151 => self.select(
                self.content_layout().capacity(self.view).max(1) as i32,
                false,
            ),
            _ => return Ok(false),
        }
        if navigating {
            if shift {
                self.extend_marks();
            }
            self.inspect_selection(storage)?;
        }
        Ok(true)
    }
    /// Ctrl+A: every listed item (marks), keeping the focused one.
    fn select_all(&mut self) {
        if self.overview && self.search.is_empty() {
            return;
        }
        self.marked = self.shown().map(|(_, e)| e.info.object).collect();
        if self
            .selected
            .is_none_or(|i| !self.shown().any(|(j, _)| j == i))
        {
            let first = self.shown().next().map(|(i, _)| i);
            self.selected = first;
        }
        self.anchor = self
            .selected
            .and_then(|i| self.entries.get(i))
            .map(|e| e.info.object);
        if self.marked.len() < 2 {
            self.marked.clear();
        }
        self.detail = None;
        door_rt::diagnostic(format_args!(
            "DOOR_P12 FILES SELECT count={}\n",
            self.marked.len().max(usize::from(self.selected.is_some()))
        ));
    }
    /// "Invert selection": every listed item that was not selected.
    fn invert_selection(&mut self) {
        let picked: Vec<u64> = self
            .shown()
            .filter(|(i, _)| !self.is_selected(*i))
            .map(|(_, e)| e.info.object)
            .collect();
        let first = self
            .shown()
            .find(|(_, e)| picked.first() == Some(&e.info.object))
            .map(|(i, _)| i);
        self.selected = first;
        self.marked = if picked.len() > 1 { picked } else { Vec::new() };
        self.anchor = None;
        self.detail = None;
    }
    /// Shift+arrows: marks the listed range between the anchor and the focus.
    fn extend_marks(&mut self) {
        let order: Vec<u64> = self.shown().map(|(_, e)| e.info.object).collect();
        let focus = self
            .selected
            .and_then(|i| self.entries.get(i))
            .map(|e| e.info.object);
        let (Some(a), Some(b)) = (
            self.anchor.and_then(|o| order.iter().position(|x| *x == o)),
            focus.and_then(|o| order.iter().position(|x| *x == o)),
        ) else {
            self.marked.clear();
            return;
        };
        let (from, to) = (a.min(b), a.max(b));
        self.marked = order[from..=to].to_vec();
        if self.marked.len() < 2 {
            self.marked.clear();
        }
    }
    /// Whether a listed entry is part of the selection.
    pub(super) fn is_selected(&self, index: usize) -> bool {
        self.selected == Some(index)
            || (!self.marked.is_empty()
                && self
                    .entries
                    .get(index)
                    .is_some_and(|e| self.marked.contains(&e.info.object)))
    }
    /// Number of selected listed items.
    pub(super) fn selection_count(&self) -> usize {
        if !self.marked.is_empty() {
            self.marked.len()
        } else {
            usize::from(self.selected_position().is_some())
        }
    }
    fn select(&mut self, delta: i32, end: bool) {
        self.thumbnail_scanned = false;
        self.folders_scanned = false;
        self.last_click = None;
        self.last_click_bounds = None;
        let indices: Vec<_> = self.shown().map(|(index, _)| index).collect();
        if indices.is_empty() {
            return;
        }
        let current = indices
            .iter()
            .position(|i| Some(*i) == self.selected)
            .map(|i| i as i32)
            .unwrap_or(if delta > 0 { -1 } else { indices.len() as i32 });
        let next = if end {
            if delta < 0 { 0 } else { indices.len() - 1 }
        } else {
            (current + delta).clamp(0, indices.len() as i32 - 1) as usize
        };
        self.selected = Some(indices[next]);
        self.reveal_position(next);
    }
    pub fn title(&self) -> &str {
        if self.overview {
            "This PC"
        } else if self.home_view {
            "Home"
        } else if self.cwd == "/compat" {
            DRIVES[0].label
        } else if self.cwd == "/users" {
            "Users"
        } else if self.recycle_view() {
            "Recycle Bin"
        } else {
            self.cwd
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("/")
        }
    }
    fn current_icon(&self) -> Icon {
        if self.overview {
            Icon::Computer
        } else if self.home_view {
            Icon::Home
        } else if self.cwd == "/compat" {
            Icon::Drive
        } else if self.cwd == self.home {
            Icon::Home
        } else if self.recycle_view() {
            self.bin_icon()
        } else {
            let name = self.cwd.rsplit('/').next().unwrap_or("");
            Self::folder_icon(name)
        }
    }
    fn folder_icon(name: &str) -> Icon {
        FOLDERS
            .iter()
            .find(|(label, _)| *label == name)
            .map_or(Icon::Folder, |(_, icon)| *icon)
    }
    pub(crate) fn drives(&self) -> &'static [Drive] {
        &DRIVES
    }
    /// One capacity RPC per This PC visit or refresh, never per frame.
    fn load_volume(&mut self, storage: &mut Storage) {
        self.volume = match storage.volume_usage() {
            Ok(usage) => VolumeState::Known(usage),
            Err(_) => VolumeState::Unavailable,
        };
        // Rename this PC shows in This PC's details at the next visit.
        self.device_name = door_service_client::device_name(storage);
    }
    fn draw_editor(&self, v: &mut ui::View<'_>, x: i32, y: i32, width: u32) {
        v.line_editor(x, y, width, &self.edit, true);
    }
    pub fn click(
        &mut self,
        x: i32,
        y: i32,
        storage: &mut Storage,
        fonts: &FontAtlas,
    ) -> Result<bool, Error> {
        if self.modal() {
            if self.security_open() {
                self.security_click(storage, x, y);
            } else if self.picker_open() {
                self.picker_click(storage, x, y);
            } else if !self.removal.dialog.open() {
                self.operations_click(x, y, storage);
            } else if let Some(choice) = self.removal.dialog.target(&self.dialog_layout(), x, y) {
                self.choose_remove(choice, storage);
            }
            return Ok(true);
        }
        if self.xmenu.is_some() {
            let flyout = self.crumb_flyout;
            let outside = self.menu_target(x, y) == Some(menus::MENU_OUTSIDE);
            self.menu_click(x, y, storage)?;
            // A click on the chevron that owns the open flyout only closes it.
            let target = self.target(x, y, fonts);
            if outside
                && flyout.is_some()
                && target != flyout
                && target.is_some_and(|t| (700..770).contains(&t))
            {
                self.address_click(target.unwrap(), storage, fonts)?;
            }
            return Ok(true);
        }
        let target = self.target(x, y, fonts);
        if self.editor == 3 {
            if target == Some(34) {
                return Ok(true);
            }
            self.finish_rename(storage);
            if self.editor == 3 {
                return Ok(true);
            }
        }
        let Some(target) = target else {
            self.editor = 0;
            return Ok(false);
        };
        if self.editor == 5 {
            self.finish_tag_rename(storage);
        }
        if !(target == 30 && self.editor == 1) {
            self.editor = 0;
        }
        self.pane_focused = (910..=939).contains(&target) || (4000..4100).contains(&target);
        self.nav_focus = None;
        match target {
            1..=8 => self.command(target, storage)?,
            44 if self.command_reason(44).is_none() => self.copy_selection(true),
            45 if self.command_reason(45).is_none() => self.copy_selection(false),
            46 if self.command_reason(46).is_none() => self.paste(),
            48 => self.delete_selection(false, storage),
            49 if !self.entries.is_empty() => self.ask_empty_bin(storage),
            50 if !self.entries.is_empty() => self.restore(self.selection_count() == 0, storage),
            operations::PROGRESS..=3199 => self.progress_click(target, storage),
            20 => self.history(storage, false)?,
            21 => self.history(storage, true)?,
            22 => self.up(storage)?,
            23 => {
                if let Some(anchor) = ExplorerLayout::navigation(1) {
                    self.open_history_menu(anchor);
                }
            }
            30 if self.editor == 1 => (),
            30 | 700..=769 => self.address_click(target, storage, fonts)?,
            4000..=4099 => self.pane_extra(target, storage)?,
            31 => {
                self.editor = 2;
                self.edit.set(self.search.clone(), true);
            }
            33 => self.begin_rename(storage),
            40 => self.set_view(ViewMode::Details),
            41 => self.set_view(ViewMode::Large),
            42 => self.scroll_items(false),
            43 => self.scroll_items(true),
            32 => {
                self.navigate(storage, self.home.clone())?;
                self.overview = false;
            }
            10..=15 => {
                let path = format!(
                    "{}/{}",
                    self.home,
                    FOLDERS[PINNED_ORDER[(target - 10) as usize]].0
                );
                self.navigate(storage, path)?;
            }
            19 => self.go_this_pc(storage)?,
            1100..=1299 => self.open_rail_row(storage, (target - 1100) as usize)?,
            1400..=1599 => self.toggle_rail_row(storage, (target - 1400) as usize),
            960..=963 => self.sort_by_column((target - 960) as usize),
            // Resize handles act on drag only.
            965..=968 => return Ok(false),
            // This PC drive tiles: a click selects and inspects, a double click opens.
            500..=500 if self.overview => {
                let slot = (target - 500) as usize;
                let Some(path) = self.drives().get(slot).map(|d| String::from(d.path)) else {
                    return Ok(true);
                };
                self.overview_selected = slot;
                if self.double_click(target, x, y) {
                    self.navigate(storage, path)?;
                } else {
                    self.show_details(storage, path)?;
                }
            }
            // Navigation-pane drive rows are destinations: one click opens.
            501 => self.navigate(storage, "/compat".into())?,
            600 => self.folders_expanded = !self.folders_expanded,
            601 => self.drives_expanded = !self.drives_expanded,
            502 => {
                self.drives_expanded = !self.drives_expanded;
                self.remember_sections();
            }
            510..=512 if self.home_view => self.toggle_home_section((target - 510) as u8),
            100..=227 => {
                self.selected = Some((target - 100) as usize);
                self.marked.clear();
                if self.double_click(target, x, y) {
                    self.activate(storage)?;
                } else {
                    self.inspect_selection(storage)?;
                }
            }
            800..=899 => {
                self.tab_action(target, storage)?;
            }
            940..=947 => self.filter_tag((target - 940) as usize),
            910..=939 => {
                self.detail_action(target, storage)?;
            }
            9998 => {
                self.selected = None;
                self.marked.clear();
                self.detail = None;
                self.last_click = None;
                self.last_click_bounds = None;
                self.thumbnail_scanned = false;
                self.folders_scanned = false;
                if self.details_visible {
                    self.inspect_selection(storage)?;
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    pub fn draw(&self, v: &mut ui::View<'_>) {
        self.draw_modern(v);
        self.draw_recycle(v);
        self.draw_cut_items(v);
        self.draw_navigation_focus(v);
        self.draw_operations(v);
        self.draw_command_tooltip(v);
        self.draw_menus(v);
        self.draw_operation_dialogs(v);
        self.draw_picker(v);
        self.draw_security(v);
        if self.removal.dialog.open() {
            let (title, body, labels) = self.removal_texts();
            v.dialog(&self.removal.dialog, &title, &body, labels);
        }
    }
    pub fn controls_changed(&mut self, controls: Controls) {
        if self.controls != controls {
            self.controls = controls;
            self.dismiss_menu();
        }
    }
    /// "Open in new window": the location the window factory should open.
    pub fn take_window_request(&self) -> Option<door_workspace::explorer_search::Open> {
        self.window_request.take()
    }
    /// Asks the window factory for a new Explorer window showing `open` (a
    /// location with an item selected, or a This PC search).
    pub fn request_window(&mut self, open: door_workspace::explorer_search::Open) {
        self.window_request.set(Some(open));
        self.window_flag = true;
    }
    /// Consumed once per request; the factory then takes the location.
    pub fn window_requested(&mut self) -> bool {
        core::mem::take(&mut self.window_flag)
    }
    /// Opens `path` (or This PC) as the first location of a new window.
    pub fn open_location(&mut self, storage: &mut Storage, path: String) -> Result<(), Error> {
        if path == HOME_PAGE {
            return Ok(());
        }
        if path == THIS_PC {
            self.go_this_pc(storage)?;
            self.history = alloc::vec![String::from(THIS_PC)];
            self.history_index = 0;
            return Ok(());
        }
        {
            self.navigate(storage, path)?;
            let current = self.history[self.history_index].clone();
            self.history = alloc::vec![current];
            self.history_index = 0;
        }
        Ok(())
    }
}
#[cfg(test)]
mod program_tests {
    use super::*;
    #[test]
    fn windows_executables_open_by_running_and_other_files_do_not() {
        for name in ["wingui-editor.exe", "SETUP.EXE", "a.Exe"] {
            assert!(is_program(name), "{name}");
        }
        for name in [".exe", "exe", "notes.txt", "archive.exe.zip", "caf\u{e9}"] {
            assert!(!is_program(name), "{name}");
        }
        // Host windows have no launch handles: nothing starts.
        let mut f = Files::empty("/users/1001".into(), Controls::default());
        assert_eq!(
            f.run_program("/users/1001/wingui-editor.exe"),
            Err(Error::Service(Code::Unsupported))
        );
    }
}
