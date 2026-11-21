//! Native Terminal consumer. File operations use the authenticated storage SDK.
//!
//! Windows Terminal mouse behaviour (`door_workspace::terminal_selection`):
//! drag, double- and triple-click, Shift+click and Alt+drag select; Ctrl+C
//! copies a selection (else interrupts), Ctrl+Shift+C and Ctrl+Insert copy,
//! Ctrl+V, Ctrl+Shift+V and Shift+Insert paste, and a right click copies a
//! selection or else pastes. Clipboard calls run in `window_update`.
use crate::ui;
use alloc::{collections::VecDeque, format, string::String, vec::Vec};
use door_graphics_text::TextStyle;
use door_service_client::{
    Disposition, Error, FileInfo, READ, SHARE_ALL, Storage, WRITE, window::WindowClient,
};
use door_service_protocol::{Code, clipboard as cb};
use door_workspace::terminal_selection::{Clicks, Point, sanitize_paste};
use door_workspace::{line_edit::LineEdit, palette::Palette, terminal_model::*};
#[path = "terminal/commands.rs"]
mod commands;
#[path = "terminal/engines.rs"]
mod engines;
#[path = "terminal/host.rs"]
mod host;
use door_service_client::Applications;
// door.accessibility tree (shared adapter `door_workspace::semantics`).
#[path = "terminal/semantics.rs"]
mod semantics;
const FG: [u8; 4] = [32, 32, 32, 255];
const MUTED: [u8; 4] = [110, 110, 110, 255];
const ACCENT: [u8; 4] = [223, 139, 0, 255];
/// Accessibility > Text size growth of a line box in `style` (0 at 100%).
/// The rail rows, the find box, the actions menu, page headings and the
/// settings, about and profile cards hold text and grow by it; the console
/// grid deliberately keeps its own cell size (as Windows Terminal follows
/// its font size), and icons, toggles and the caption keep theirs.
fn grow_by(style: TextStyle) -> i32 {
    door_workspace::palette::text_grow_i(0, style)
}
fn grow() -> i32 {
    grow_by(TextStyle::Regular14)
}
/// Target ids of the caption's tabs and their close buttons, and of Home's
/// "Open sessions" cards and theirs: a range each, for any number of tabs.
pub(crate) const TAB: u32 = 0x10_0000;
pub(crate) const TAB_CLOSE: u32 = 0x20_0000;
pub(crate) const SESSION: u32 = 0x30_0000;
pub(crate) const SESSION_CLOSE: u32 = 0x40_0000;
/// The last id of each range.
pub(crate) const TAB_LAST: u32 = TAB_CLOSE - 1;
pub(crate) const TAB_CLOSE_LAST: u32 = SESSION - 1;
pub(crate) const SESSION_LAST: u32 = SESSION_CLOSE - 1;
pub(crate) const SESSION_CLOSE_LAST: u32 = SESSION_CLOSE + (TAB_CLOSE - TAB) - 1;
/// Rail row pitch; a row's plate is 2 px shorter (40 and 38 px at 100%).
fn rail_pitch() -> i32 {
    40 + grow()
}
/// Rail rows (id, top): Home, the four profiles (Doorsh, Command Prompt,
/// PowerShell, DSL) under the "Profiles" heading, Add profile, then
/// Settings and About under the rule. Paint, hit testing and semantics
/// share these.
fn rail_rows() -> [(u32, i32); 8] {
    let (g, p) = (grow(), rail_pitch());
    let profiles = 144 + 2 * g;
    let settings = 364 + 7 * g;
    [
        (1, 64),
        (10, profiles),
        (11, profiles + p),
        (12, profiles + 2 * p),
        (13, profiles + 3 * p),
        (20, profiles + 4 * p),
        (2, settings),
        (3, settings + p),
    ]
}
/// Top of the rail's "Profiles" heading and y of the rule above Settings.
fn rail_heading() -> i32 {
    118 + grow()
}
fn rail_rule() -> i32 {
    348 + 7 * grow()
}
fn rail_label(id: u32) -> &'static str {
    match id {
        1 => "Home",
        2 => "Settings",
        3 => "About",
        20 => "Add profile",
        10..=13 => Profile::ALL[(id - 10) as usize].name(),
        _ => "",
    }
}
/// Home's section header tops (New session, Open sessions, Recent
/// folders) and its targets (id, bounds); see `Terminal::home_layout`.
pub(crate) struct HomeLayout {
    /// (x, y) of each header shown.
    pub headers: [Option<(i32, i32)>; 3],
    pub targets: Vec<(u32, door_workspace::explorer_layout::Bounds)>,
}
/// Recent folders Home lists.
const MAX_RECENT: usize = 6;
/// Settings' folder symbol (Home's recent folders).
fn folder_glyph(scale: u32) -> &'static [u8] {
    match scale {
        3 => door_workspace::asset!("../../../assets/icons/windows-shell/app-folder.60.mask"),
        2 => door_workspace::asset!("../../../assets/icons/windows-shell/app-folder.40.mask"),
        _ => door_workspace::asset!("../../../assets/icons/windows-shell/app-folder.20.mask"),
    }
}
/// Page heading band: a 28 px title and a 12 px subtitle; content starts
/// `heading_growth()` lower than at 100%.
fn heading_growth() -> i32 {
    grow_by(TextStyle::Semibold28) + grow_by(TextStyle::Regular12)
}
/// Card height: a 14 px title over a 12 px description (70 px at 100%).
fn card_growth() -> i32 {
    grow() + grow_by(TextStyle::Regular12)
}
/// Settings cards (id, top, height): Color scheme, Text size, Command
/// history, Background opacity.
fn settings_cards() -> [(u32, i32, i32); 4] {
    let (top, h) = (132 + heading_growth(), 70 + card_growth());
    let pitch = h + 16;
    [
        (40, top, h),
        (41, top + pitch, h),
        (42, top + 2 * pitch, h),
        (44, top + 3 * pitch, h),
    ]
}
/// Settings footer top and its Save button.
fn settings_footer(height: u32) -> (i32, door_workspace::explorer_layout::Bounds) {
    let footer = height.saturating_sub(64 + grow() as u32) as i32;
    (
        footer,
        door_workspace::explorer_layout::Bounds {
            x: 0,
            y: footer + 16,
            width: 120,
            height: (32 + grow()) as u32,
        },
    )
}
pub struct Terminal {
    pub state: TerminalState,
    layout: Layout,
    query: LineEdit,
    searching: bool,
    actions_open: bool,
    search_focus: bool,
    pub message: String,
    preferences_path: String,
    preferences_info: Option<FileInfo>,
    preferences_writable: bool,
    environment: Vec<String>,
    uid: u32,
    /// The account is an administrator (Administrators for deny only when
    /// unelevated); a standard user's token has no Administrators group.
    administrator: bool,
    /// Inherited registry and directory handles: programs start through Init
    /// EXECUTE as the session user (0 in host tests: no program launch).
    registry: u64,
    directory: u64,
    /// The program a tab started (`terminal/commands.rs`).
    /// The programs the tabs run (at most one per tab).
    running: Vec<commands::Running>,
    /// PowerShell 7 has been looked for (`refresh_powershell`).
    powershell_checked: bool,
    /// The last tab's own program ended: the window closes (`take_close`),
    /// as Windows Terminal closes with its last tab.
    pending_close: bool,
    /// The window gave its last tab to another window (it closes).
    emptied: bool,
    /// Settings > Personalization > Colors > Transparency effects is off.
    reduce_transparency: bool,
    /// A contrast theme is on: Windows Terminal then paints opaque, with
    /// the system colours.
    contrast: bool,
    clicks: Clicks,
    /// A left-button selection drag is in progress; the last pointer point.
    drag: Option<(i32, i32)>,
    /// When a drag past the console's top or bottom edge next scrolls.
    autoscroll_at: u64,
    /// Shift and Alt are held (Shift+click extends, Alt+drag is a block).
    shift: bool,
    alt: bool,
    /// Clipboard work for `window_update` (the window client), oldest first.
    clipboard: VecDeque<Clip>,
    /// Copied text when the session has no clipboard broker.
    local_clipboard: String,
    /// Console grid (columns, rows) last reported by `DOOR_P14 TERMINAL RESIZE`.
    reported_grid: core::cell::Cell<(usize, usize)>,
    /// SETTINGS_NAVIGATE argument a legacy command resolved to (taken by
    /// `window_update`).
    pub pending_settings: Option<u32>,
    /// CMD tabs' interpreters (`terminal/engines.rs`).
    sessions: Vec<engines::Session>,
    launches: VecDeque<(u64, door_shells::Launch)>,
    pending_batches: VecDeque<String>,
    /// The signed-in user's name (C:\Users\<name>) and the PC's name.
    user: String,
    computer: String,
    /// Minutes east of UTC for local times.
    utc_offset: i64,
    /// Full screen, and a change for `window_update` to ask for.
    fullscreen: bool,
    pending_fullscreen: Option<bool>,
    /// Home's recent folders (native paths, newest first), their file, and
    /// the tab folders last seen (changes add to the list).
    pub(crate) recent: Vec<String>,
    recent_path: String,
    recent_dirty: bool,
    seen_cwd: Vec<(u64, String)>,
}
/// A clipboard command waiting for `window_update`.
enum Clip {
    Copy(String),
    Paste,
}
/// Queued clipboard commands at most (Ctrl+C then Ctrl+V keeps both).
const MAX_CLIPBOARD: usize = 8;
/// Clipboard text read for a paste at most; only its first line is used.
const PASTE_BYTES: usize = 64 * 1024;
/// `door_sys::time` units between autoscroll steps while dragging.
const AUTOSCROLL_TICKS: u64 = 5;
/// The console grid exactly as `draw_home` paints it; hit testing and
/// selection use the same numbers.
#[derive(Clone, Copy)]
struct Grid {
    x: i32,
    top: i32,
    rows: usize,
    columns: usize,
    cw: u32,
    rh: u32,
}
impl Terminal {
    /// Mouse wheel over the console scrolls the active tab's scrollback by
    /// three rows per detent (see `Application::wheel`).
    pub fn wheel(&mut self, x: i32, y: i32, notches: i32) -> bool {
        if notches == 0
            || self.state.page != Page::Console
            || self.actions_open
            || x < self.layout.rail() as i32
            || y < CONTENT_TOP
            || !self.state.tabs[self.state.active].profile.available()
        {
            return false;
        }
        let columns = self.columns();
        let tab = self.state.tab();
        let before = tab.scroll;
        tab.scroll_rows(
            notches > 0,
            WHEEL_ROWS * notches.unsigned_abs() as usize,
            columns,
        );
        tab.scroll != before
    }
    /// Live acrylic (see `Application::acrylic`): the whole client background
    /// is translucent so the compositor blurs the windows behind it, unless
    /// Settings has transparency effects off. Text is painted opaque over it.
    pub fn acrylic(&self) -> bool {
        !self.reduce_transparency && !self.contrast
    }
    /// Desktop controls from Settings: transparency and the contrast theme.
    pub fn controls_changed(&mut self, controls: door_service_protocol::desktop::Controls) {
        self.reduce_transparency = controls.reduce_transparency();
        self.contrast = controls.contrast().is_some();
    }
    /// Premultiplied acrylic tint (or the opaque fallback): BGRA 32,32,32 at
    /// 78% in dark, 243,243,243 at 80% in light; the tab row slightly darker.
    fn material(&self, dark: bool, caption: bool, contrast: bool) -> [u8; 4] {
        let (gray, alpha) = match (dark, caption) {
            (true, false) => (32u16, 199u16),
            (true, true) => (20, 214),
            (false, false) => (243, 204),
            (false, true) => (232, 214),
        };
        // A contrast theme is opaque; the light layers map to Window.
        if self.contrast || contrast {
            return [255; 4];
        }
        if self.reduce_transparency {
            let g = gray as u8;
            return [g, g, g, 255];
        }
        let c = ((gray * alpha + 127) / 255) as u8;
        [c, c, c, alpha as u8]
    }
    fn tab_colors(&self, palette: Palette) -> door_workspace::caption_tabs::TabColors {
        let (dark, contrast) = (palette.dark, palette.contrast.is_some());
        // Hover, pressed and divider are overlays blended onto the material.
        let overlay = |white: bool, percent: u16| {
            let a = (percent * 255 / 100) as u8;
            if white { [a, a, a, a] } else { [0, 0, 0, a] }
        };
        door_workspace::caption_tabs::TabColors {
            caption: self.material(dark, true, contrast),
            selected: self.material(dark, false, contrast),
            hover: overlay(dark, 6),
            pressed: overlay(dark, 10),
            divider: overlay(dark, 14),
            foreground: if dark {
                [240, 240, 240, 255]
            } else {
                [24, 24, 24, 255]
            },
        }
    }
    /// Windows Terminal's new-tab split button after the tabs: "+" and a
    /// "v" profile drop-down, 64 x 24 together at y 8, when it fits before
    /// the window controls; otherwise Explorer's plain "+" button alone.
    pub(crate) fn new_tab_buttons(
        &self,
    ) -> Option<(
        door_workspace::explorer_layout::Bounds,
        Option<door_workspace::explorer_layout::Bounds>,
    )> {
        let p = self.layout.plus(self.state.tabs.len(), self.state.active)?;
        let half = |x| door_workspace::explorer_layout::Bounds {
            x,
            y: NEW_TAB_Y,
            width: NEW_TAB_HALF,
            height: NEW_TAB_H,
        };
        Some(
            if p.x + 2 * NEW_TAB_HALF as i32 <= self.layout.width as i32 - 144 {
                (half(p.x), Some(half(p.x + NEW_TAB_HALF as i32)))
            } else {
                (p, None)
            },
        )
    }
    /// Client-owned caption region: Explorer's tab strip from the left edge
    /// through the new-tab buttons; the rest of the caption stays draggable.
    pub fn caption_region(&self) -> (u32, u32) {
        let strip = self.layout.strip(self.state.tabs.len(), self.state.active);
        let (left, width) = door_workspace::caption_tabs::region(strip);
        match self.new_tab_buttons() {
            Some((_, Some(d))) => (left, (d.x + d.width as i32 - left as i32) as u32),
            _ => (left, width),
        }
    }
    pub fn new(
        home: String,
        startup: &door_abi::platform::StartupV2,
        storage: &mut Storage,
    ) -> Self {
        let mut this = Self::for_home(home);
        this.uid = startup.identity.uid;
        this.administrator = startup.identity.groups
            & (door_abi::platform::group::ADMINISTRATOR_ACCOUNT
                | door_abi::platform::group::ADMINISTRATORS)
            != 0;
        // Run as administrator: Init started this window with the full
        // token (mode TERMINAL_ELEVATED); every tab is elevated.
        if startup.identity.groups & door_abi::platform::group::ADMINISTRATORS != 0 {
            this.state.elevate();
        }
        this.environment = door_userland::environment(startup);
        this.registry = startup.base.inherited[door_abi::platform::inherited::REGISTRY].handle;
        this.directory = startup.base.inherited[door_abi::platform::inherited::DIRECTORY].handle;
        if let Some(name) = this
            .environment
            .iter()
            .find_map(|p| p.strip_prefix("USERNAME="))
            .filter(|n| !n.is_empty())
        {
            this.user = String::from(name);
        }
        this.computer = String::from(door_service_client::device_name(storage).as_str());
        if let Ok(id) = storage.get_setting(door_service_client::Setting::TimeZone) {
            let zone = door_locale::tz::zone_or_utc(id);
            let now = door_sys::wall_time().unwrap_or(0);
            this.utc_offset = i64::from(zone.offset_at(now).0);
        }
        // Later changes arrive as CONTROLS_CHANGED; an unreadable setting keeps
        // the default (transparency on), which the server can still flatten.
        if let Some(c) = storage
            .get_setting(door_service_client::Setting::DesktopControls)
            .ok()
            .and_then(door_service_protocol::desktop::Controls::from_bits)
        {
            this.controls_changed(c);
        }
        if let Err(e) = this.load_preferences(storage) {
            this.preferences_writable = false;
            this.message = format!("Preferences could not be loaded: {e:?}. Defaults are in use.");
        }
        this.load_recent(storage);
        this.sync_columns();
        this
    }
    pub fn for_home(home: String) -> Self {
        let mut this = Self {
            preferences_path: format!("{home}/terminal.conf"),
            state: TerminalState::new(home.clone()),
            layout: Layout {
                width: 1050,
                height: 650,
                rail_hidden: false,
            },
            query: LineEdit::default(),
            searching: false,
            actions_open: false,
            search_focus: false,
            message: String::new(),
            preferences_info: None,
            preferences_writable: true,
            environment: Vec::new(),
            uid: 0,
            administrator: true,
            registry: 0,
            directory: 0,
            running: Vec::new(),
            powershell_checked: false,
            pending_close: false,
            emptied: false,
            pending_settings: None,
            reduce_transparency: false,
            contrast: false,
            clicks: Clicks::default(),
            drag: None,
            autoscroll_at: 0,
            shift: false,
            alt: false,
            clipboard: VecDeque::new(),
            local_clipboard: String::new(),
            reported_grid: core::cell::Cell::new((0, 0)),
            sessions: Vec::new(),
            launches: VecDeque::new(),
            pending_batches: VecDeque::new(),
            user: String::from("User"),
            computer: String::from("DOORS-PC"),
            utc_offset: 0,
            fullscreen: false,
            pending_fullscreen: None,
            recent: Vec::new(),
            recent_path: format!("{home}/terminal-recent.conf"),
            recent_dirty: false,
            seen_cwd: Vec::new(),
        };
        this.sync_columns();
        this
    }
    /// One `DOOR_P14 TERMINAL RESIZE` line whenever the console grid the
    /// painter uses changes size (window, find box, font or text size).
    fn report_grid(&self) {
        let grid = self.grid();
        let size = (grid.columns, grid.rows);
        if self.reported_grid.replace(size) != size {
            door_rt::diagnostic(format_args!(
                "DOOR_P14 TERMINAL RESIZE columns={} rows={}\n",
                size.0, size.1
            ));
        }
    }
    /// Every tab wraps at the console's current width (`Tab::reflow`).
    fn sync_columns(&mut self) {
        let columns = self.columns();
        self.state.set_columns(columns);
    }
    /// Integration-test hook: show the "..." actions menu.
    #[cfg(test)]
    pub(crate) fn open_actions_for_review(&mut self) {
        self.actions_open = true;
    }
    /// Top of the unavailable-profile "View runtime requirements" button,
    /// under the grown title, status and explanation lines (300 at 100%).
    fn requirements_top() -> i32 {
        300 + grow_by(TextStyle::Semibold28) + grow_by(TextStyle::Semibold14) + 4 * grow()
    }
    fn load_preferences(&mut self, storage: &mut Storage) -> Result<(), Error> {
        let file = match storage.open(
            &self.preferences_path,
            Disposition::OpenExisting,
            READ,
            SHARE_ALL,
        ) {
            Ok(f) => f,
            Err(Error::Service(Code::NotFound)) => return Ok(()),
            Err(e) => return Err(e),
        };
        let result: Result<(Preferences, FileInfo), Error> = (|| {
            let info = storage.info(&file)?;
            if info.bytes == 0 {
                return Ok((Preferences::default(), info));
            }
            if info.bytes != 8 {
                return Err(Error::Malformed);
            }
            let mut data = [0; 8];
            if storage.read(&file, 0, &mut data)? != 8 {
                return Err(Error::Malformed);
            }
            if storage.info(&file)? != info {
                return Err(Error::Service(Code::Changed));
            }
            let preferences = Preferences::decode(&data).ok_or(Error::Malformed)?;
            Ok((preferences, info))
        })();
        let closed = storage.close(file);
        let (preferences, info) = result?;
        closed?;
        self.state.preferences = preferences;
        self.preferences_info = Some(info);
        Ok(())
    }
    fn save_preferences(&mut self, storage: &mut Storage) -> Result<(), Error> {
        if !self.preferences_writable {
            return Err(Error::Service(Code::Changed));
        }
        let file = storage.open(
            &self.preferences_path,
            if self.preferences_info.is_some() {
                Disposition::OpenExisting
            } else {
                Disposition::CreateNew
            },
            READ | WRITE,
            SHARE_ALL,
        )?;
        let result: Result<(), Error> = (|| {
            let expected = match self.preferences_info {
                Some(info) => info,
                None => {
                    let info = storage.info(&file)?;
                    self.preferences_info = Some(info);
                    info
                }
            };
            let staged =
                storage.prepare_replace(&file, expected, &self.state.preferences.encode())?;
            // An uncertain commit is never replayed. Reopening Terminal reloads
            // its actual durable checkpoint before permitting another save.
            self.preferences_writable = false;
            let info = storage.commit_replace(staged)?;
            self.preferences_info = Some(info);
            Ok(())
        })();
        let closed = storage.close(file);
        result?;
        closed?;
        self.preferences_writable = true;
        Ok(())
    }
    /// A window resize reflows every tab's scrollback to the new width.
    pub fn configure(&mut self, width: u32, height: u32) {
        self.layout = Layout {
            width,
            height,
            rail_hidden: self.layout.rail_hidden,
        };
        self.sync_columns();
        self.report_grid();
    }
    fn columns(&self) -> usize {
        (self.layout.width.saturating_sub(self.layout.rail() + 42)
            / self.state.preferences.cell().0)
            .max(1) as usize
    }
    fn grid(&self) -> Grid {
        let top = if self.searching {
            136 + grow() as u32
        } else {
            78
        };
        let (cw, rh) = self.state.preferences.cell();
        // The status line under the console grows with its 12 px text.
        let status = grow_by(TextStyle::Regular12) as u32;
        Grid {
            x: self.layout.rail() as i32 + 18,
            top: top as i32,
            rows: (self.layout.height.saturating_sub(top + 55 + status) / rh) as usize,
            columns: self.columns(),
            cw,
            rh,
        }
    }
    /// The console toolbar band (Find, the disabled pane button, Clear,
    /// "..."), which overlaps the first console rows.
    fn toolbar(&self, x: i32, y: i32) -> bool {
        (64..100).contains(&y) && x >= self.layout.width as i32 - 164
    }
    /// The console's text cells, where a left press starts a selection.
    fn console_hit(&self, x: i32, y: i32) -> bool {
        let grid = self.grid();
        let tab = &self.state.tabs[self.state.active];
        self.state.page == Page::Console
            && !self.actions_open
            && tab.profile.available()
            // The program grid's cells are not selectable; its scrollback is.
            && (tab.console.is_none() || tab.scroll > 0)
            && x >= self.layout.rail() as i32 + 16
            && x < self.layout.width as i32 - 18
            && y >= grid.top
            && y < self.layout.height as i32 - 30 - grow_by(TextStyle::Regular12)
            && self.target(x, y).is_none()
            && !self.toolbar(x, y)
    }
    /// The scrollback cell under (x, y), clamped to the rows shown: above
    /// or below them selects in the first or last row.
    fn point_at(&self, x: i32, y: i32) -> Option<Point> {
        let grid = self.grid();
        let tab = &self.state.tabs[self.state.active];
        let view = tab.viewport(grid.columns, grid.rows);
        let last = view.len().checked_sub(1)?;
        let row = (y.saturating_sub(grid.top).max(0) / grid.rh as i32) as usize;
        let column = (x.saturating_sub(grid.x).max(0) / grid.cw as i32) as usize;
        tab.point(view.start + row.min(last), column, grid.columns)
    }
    /// Left press: a click in the console starts a selection (2 clicks a
    /// word, 3 a line; Shift extends, Alt makes a block).
    pub fn pointer_down(&mut self, x: i32, y: i32) -> bool {
        self.pointer_down_at(x, y, door_sys::time().unwrap_or(0))
    }
    pub(crate) fn pointer_down_at(&mut self, x: i32, y: i32, now: u64) -> bool {
        if !self.console_hit(x, y) {
            return false;
        }
        let clicks = self.clicks.press(now, x, y);
        let Some(point) = self.point_at(x, y) else {
            return false;
        };
        let (shift, alt) = (self.shift, self.alt);
        self.search_focus = false;
        self.state.tab().press(point, clicks, shift, alt);
        self.drag = Some((x, y));
        self.autoscroll_at = now;
        true
    }
    pub fn pointer_motion(&mut self, x: i32, y: i32) -> bool {
        if self.drag.is_none() {
            return false;
        }
        self.drag = Some((x, y));
        match self.point_at(x, y) {
            Some(point) => self.state.tab().drag(point),
            None => false,
        }
    }
    pub fn pointer_up(&mut self) {
        if self.drag.take().is_some() {
            self.state.tab().release();
            let columns = self.columns();
            if let Some(text) = self.state.tabs[self.state.active].selected_text(columns) {
                door_rt::diagnostic(format_args!(
                    "DOOR_P14 TERMINAL SELECT chars={}\n",
                    text.chars().count()
                ));
            }
        }
    }
    /// Dragging past the console's top or bottom edge scrolls the
    /// scrollback (faster further out) and extends the selection with it.
    pub fn autoscroll(&mut self, now: u64) -> bool {
        let Some((x, y)) = self.drag else {
            return false;
        };
        if now < self.autoscroll_at {
            return false;
        }
        let grid = self.grid();
        let bottom = grid.top + (grid.rows as u32 * grid.rh) as i32;
        let (previous, distance) = if y < grid.top {
            (true, grid.top - y)
        } else if y >= bottom {
            (false, y - bottom + 1)
        } else {
            return false;
        };
        let step = (1 + (distance / grid.rh as i32) as usize).min(grid.rows.max(1));
        self.autoscroll_at = now + AUTOSCROLL_TICKS;
        let tab = self.state.tab();
        let before = tab.scroll;
        tab.scroll_rows(previous, step, grid.columns);
        let moved = tab.scroll != before;
        let extended = match self.point_at(x, y) {
            Some(point) => self.state.tab().drag(point),
            None => false,
        };
        moved || extended
    }
    /// Right click over the console: copy the selection, or else paste
    /// (Windows Terminal's default).
    pub fn context(&mut self, x: i32, y: i32) -> bool {
        if self.state.page != Page::Console
            || self.actions_open
            || x < self.layout.rail() as i32
            || y < CONTENT_TOP
            || !self.state.tabs[self.state.active].profile.available()
        {
            return false;
        }
        if self.state.tabs[self.state.active]
            .selection_bounds()
            .is_some()
        {
            self.copy();
        } else {
            self.queue(Clip::Paste);
        }
        true
    }
    /// Shift and Alt presses and releases by scan code; true for a modifier
    /// key (it neither edits nor scrolls).
    pub fn modifier(&mut self, scan: u32, down: bool) -> bool {
        match scan {
            42 | 54 => self.shift = down,
            56 | 0x138 => self.alt = down,
            29 | 0x11d | 0x15b | 0x15c => (),
            _ => return false,
        }
        true
    }
    /// Focus left the window: no key is known to be held, no drag goes on.
    pub fn focus(&mut self, focused: bool) {
        if !focused {
            self.shift = false;
            self.alt = false;
            self.pointer_up();
        }
    }
    /// Windows Terminal's clipboard keys: Ctrl+Shift+C, Ctrl+Insert and
    /// (over a selection) Ctrl+C copy; Ctrl+V, Ctrl+Shift+V and Shift+Insert
    /// paste. False for other keys: Ctrl+C without a selection still
    /// interrupts (`key`).
    fn clipboard_key(&mut self, scan: u32, ctrl: bool, shift: bool) -> bool {
        let selected = self.state.page == Page::Console
            && self.state.tabs[self.state.active]
                .selection_bounds()
                .is_some();
        if ctrl && ((scan == 46 && (shift || selected)) || scan == 0x152) {
            self.copy();
            return true;
        }
        if (ctrl && scan == 47) || (shift && scan == 0x152) {
            self.queue(Clip::Paste);
            return true;
        }
        false
    }
    /// Copy the active tab's selection and clear it; false without text.
    fn copy(&mut self) -> bool {
        let columns = self.columns();
        let tab = self.state.tab();
        let text = tab.selected_text(columns);
        tab.selection = None;
        match text {
            Some(text) => {
                self.queue(Clip::Copy(text));
                true
            }
            None => false,
        }
    }
    fn queue(&mut self, clip: Clip) {
        if self.clipboard.len() >= MAX_CLIPBOARD {
            self.message = "The clipboard is busy. Try again.".into();
            return;
        }
        self.clipboard.push_back(clip);
    }
    /// Insert clipboard text: into the find box when it has focus, else the
    /// console input, one sanitized line only (`sanitize_paste`); a
    /// multi-line paste never submits a command.
    pub fn paste_text(&mut self, text: &str) -> bool {
        // A program in the active tab's grid gets the whole paste as input.
        if !self.search_focus && self.state.page == Page::Console && self.program_paste(text) {
            return true;
        }
        let (limit, available) = if self.search_focus {
            (192, true)
        } else {
            (
                MAX_LINE,
                self.state.page == Page::Console
                    && self.state.tabs[self.state.active].profile.available(),
            )
        };
        if !available {
            return false;
        }
        let (line, more) = sanitize_paste(text, limit);
        let target = if self.search_focus {
            &mut self.query
        } else {
            let tab = self.state.tab();
            tab.selection = None;
            tab.scroll = 0;
            &mut tab.input
        };
        for c in line.chars() {
            target.insert_bounded(c, limit);
        }
        door_rt::diagnostic(format_args!(
            "DOOR_P14 TERMINAL PASTE bytes={}\n",
            line.len()
        ));
        if more > 0 {
            self.message = format!(
                "Pasted the first line only; {more} more line{} not pasted or run.",
                if more == 1 { " was" } else { "s were" }
            );
        }
        true
    }
    /// Run queued clipboard commands with the window client.
    pub fn window_update(&mut self, client: &mut WindowClient, window: u64) -> Result<bool, Error> {
        let mut dirty = false;
        if let Some(page) = self.pending_settings.take() {
            client.open_settings_page(page)?;
            dirty = true;
        }
        if let Some(on) = self.pending_fullscreen.take() {
            // Refused (the window lost focus first): keep the state honest.
            if client.set_fullscreen(window, on).is_err() {
                self.fullscreen = !on;
            }
            dirty = true;
        }
        dirty |= self.run_launches(client)?;
        while let Some(clip) = self.clipboard.pop_front() {
            dirty |= match clip {
                Clip::Copy(text) => self.store_clipboard(text, client, window)?,
                Clip::Paste => match self.read_clipboard(client)? {
                    Some(text) => self.paste_text(&text),
                    None => false,
                },
            };
        }
        Ok(dirty)
    }
    fn store_clipboard(
        &mut self,
        text: String,
        client: &mut WindowClient,
        window: u64,
    ) -> Result<bool, Error> {
        if client.supports_clipboard() {
            match client.clipboard_set(window, &[(cb::TEXT, text.as_bytes())]) {
                Ok(_) => (),
                Err(Error::Service(_)) => {
                    self.message = "The text couldn't be copied to the clipboard.".into();
                    return Ok(true);
                }
                Err(e) => return Err(e),
            }
        }
        door_rt::diagnostic(format_args!(
            "DOOR_P14 TERMINAL COPY bytes={}\n",
            text.len()
        ));
        self.local_clipboard = text;
        Ok(false)
    }
    /// Clipboard text, its first `PASTE_BYTES` at most.
    fn read_clipboard(&mut self, client: &mut WindowClient) -> Result<Option<String>, Error> {
        if !client.supports_clipboard() {
            return Ok(Some(self.local_clipboard.clone()));
        }
        let info = match client.clipboard_info() {
            Ok(info) => info,
            Err(Error::Service(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        let size = (info.size(cb::TEXT) as usize).min(PASTE_BYTES);
        let mut bytes = Vec::with_capacity(size);
        let mut chunk = [0; cb::READ_BYTES];
        while bytes.len() < size {
            match client.clipboard_read(info.sequence, cb::TEXT, bytes.len() as u32, &mut chunk) {
                Ok(0) => break,
                Ok(n) => bytes.extend_from_slice(&chunk[..n.min(size - bytes.len())]),
                Err(Error::Service(_)) => return Ok(None),
                Err(e) => return Err(e),
            }
        }
        Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
    }
    pub fn text_hit(&self, x: i32, y: i32) -> bool {
        self.state.page == Page::Console
            && !self.actions_open
            && x >= self.layout.rail() as i32 + 16
            && x < self.layout.width as i32 - 18
            // The find box (from y 90) or the console rows (from their top).
            && y >= self.grid().top.min(90)
            && y < self.layout.height as i32 - 30 - grow_by(TextStyle::Regular12)
            && matches!(self.target(x, y), None | Some(34))
            && !self.toolbar(x, y)
            && self.state.tabs[self.state.active].profile.available()
    }
    pub fn menu_open(&self) -> bool {
        self.actions_open
    }
    fn row_at(&self, y: i32) -> Option<u32> {
        let pitch = rail_pitch();
        rail_rows()
            .into_iter()
            .find(|(_, top)| (*top..top + pitch).contains(&y))
            .map(|(id, _)| id)
    }
    /// The "..." actions menu: three 14 px rows; it widens and heightens
    /// with the text (184 x 108 px at 100%).
    fn actions_menu(&self) -> door_workspace::explorer_layout::Bounds {
        let width = 28 + door_workspace::palette::text_len(156);
        door_workspace::explorer_layout::Bounds {
            x: self.layout.width as i32 - 12 - width as i32,
            y: 104,
            width,
            height: (108 + 3 * grow()) as u32,
        }
    }
    /// Find box over the console (36 px tall at 100%).
    fn find_box(&self) -> door_workspace::explorer_layout::Bounds {
        let x = self.layout.rail() as i32 + 16;
        door_workspace::explorer_layout::Bounds {
            x,
            y: 90,
            width: self.layout.width.saturating_sub(self.layout.rail() + 32),
            height: (36 + grow()) as u32,
        }
    }
    pub fn target(&self, x: i32, y: i32) -> Option<u32> {
        if x < 0 || y < 0 || x >= self.layout.width as i32 || y >= self.layout.height as i32 {
            return None;
        }
        let (count, active) = (self.state.tabs.len(), self.state.active);
        if y < CAPTION as i32 {
            // Explorer caption strip: close buttons win over their tab.
            if let Some(i) = self.layout.close_at(x, y, count, active) {
                return Some(TAB_CLOSE + i as u32);
            }
            if let Some(i) = self.layout.tab_at(x, y, count, active) {
                return Some(TAB + i as u32);
            }
            let (plus, profiles) = self.new_tab_buttons()?;
            if plus.contains(x, y) {
                return Some(30);
            }
            // The drop-down lists the profiles (the Profiles page).
            return profiles.filter(|r| r.contains(x, y)).map(|_| 20);
        }
        if x < self.layout.rail() as i32 {
            return self.row_at(y);
        }
        if y < CONTENT_TOP {
            return None;
        }
        if self.state.page == Page::Console {
            let menu = self.actions_menu();
            if self.actions_open && menu.contains(x, y) {
                let row = ((y - menu.y) / (36 + grow())).min(2) as usize;
                return Some([31, 32, 33][row]);
            }
            if (64..100).contains(&y) {
                let right = self.layout.width as i32;
                if x >= right - 44 {
                    return Some(37);
                }
                if x >= right - 84 {
                    return Some(32);
                }
                if x >= right - 124 {
                    return Some(36);
                }
                if x >= right - 164 {
                    return Some(31);
                }
                if x >= right - 204 {
                    return Some(38);
                }
            }
            if self.searching && (90..130 + grow()).contains(&y) {
                return Some(34);
            }
            if x >= self.layout.width as i32 - 18 && y >= 90 {
                return Some(35);
            }
            let button = Self::requirements_top();
            if !self.state.tabs[self.state.active].profile.available()
                && (button..button + 40 + grow()).contains(&y)
            {
                return Some(20);
            }
        }
        if self.state.page == Page::Home {
            // Close buttons sit inside their cards and win over them.
            return self
                .home_layout()
                .targets
                .iter()
                .rev()
                .find(|(_, r)| r.contains(x, y))
                .map(|(id, _)| *id);
        }
        if self.state.page == Page::Settings {
            let (_, save) = settings_footer(self.layout.height);
            if x >= self.layout.width as i32 - 148
                && x < self.layout.width as i32 - 28
                && (save.y..save.y + save.height as i32).contains(&y)
            {
                return Some(43);
            }
            if x < self.layout.rail() as i32 + 28 || x >= self.layout.width as i32 - 28 {
                return None;
            }
            for (id, top, height) in settings_cards() {
                if (top..top + height).contains(&y) {
                    return Some(id);
                }
            }
        }
        None
    }
    pub fn click(&mut self, x: i32, y: i32, storage: &mut Storage) -> Result<bool, Error> {
        let Some(id) = self.target(x, y) else {
            self.actions_open = false;
            self.search_focus = false;
            return Ok(true);
        };
        self.search_focus = id == 34;
        if id != 37 {
            self.actions_open = false;
        }
        match id {
            1 => self.state.page = Page::Home,
            2 => self.state.page = Page::Settings,
            3 => self.state.page = Page::About,
            10..=13 => {
                let p = Profile::ALL[(id - 10) as usize];
                if let Some(i) = self.state.tabs.iter().position(|t| t.profile == p) {
                    self.state.active = i;
                    self.state.page = Page::Console;
                } else {
                    self.open(p);
                }
            }
            20 => self.state.page = Page::Profiles,
            // Home: a profile tile always starts a new session; a session
            // row switches to that tab.
            50..=53 => {
                self.open(Profile::ALL[(id - 50) as usize]);
                self.state.page = Page::Console;
            }
            SESSION..=SESSION_LAST if ((id - SESSION) as usize) < self.state.tabs.len() => {
                self.state.active = (id - SESSION) as usize;
                self.state.page = Page::Console;
            }
            SESSION_CLOSE..=SESSION_CLOSE_LAST
                if ((id - SESSION_CLOSE) as usize) < self.state.tabs.len() =>
            {
                self.state.close((id - SESSION_CLOSE) as usize)
            }
            80..=87 if ((id - 80) as usize) < self.recent.len() => {
                let path = self.recent[(id - 80) as usize].clone();
                if self.state.new_tab(Profile::Native) {
                    self.state.tab().cwd = path;
                    self.state.page = Page::Console;
                }
            }
            30 => self.open(Profile::Native),
            31 => {
                self.searching = !self.searching;
                self.search_focus = self.searching;
            }
            32 => {
                self.state.tab().clear();
            }
            37 => {
                self.actions_open = !self.actions_open;
            }
            33 => {
                self.state.tab().scroll = 0;
            }
            34 => (),
            36 => self.toggle_rail(),
            38 => self.toggle_fullscreen(),
            35 => {
                let columns = self.columns();
                self.state
                    .tab()
                    .scroll_rows(y < self.layout.height as i32 / 2, 8, columns);
            }
            40 => self.state.preferences.light_console = !self.state.preferences.light_console,
            41 => {
                self.state.preferences.font = (self.state.preferences.font + 1) % 3;
                self.sync_columns();
            }
            42 => {
                self.state.preferences.history = !self.state.preferences.history;
                if !self.state.preferences.history {
                    for tab in &mut self.state.tabs {
                        tab.forget();
                    }
                }
            }
            44 => {
                let opacity = self.state.preferences.opacity;
                self.state.preferences.opacity = if opacity >= 100 { 50 } else { opacity + 5 };
            }
            43 => {
                self.message = match self.save_preferences(storage) {
                    Ok(()) => "Preferences saved".into(),
                    Err(e) => format!(
                        "Preferences not confirmed saved: {e:?}. Reopen Terminal to reload before retrying."
                    ),
                };
            }
            TAB..=TAB_LAST if ((id - TAB) as usize) < self.state.tabs.len() => {
                self.state.active = (id - TAB) as usize;
                self.state.page = Page::Console;
            }
            TAB_CLOSE..=TAB_CLOSE_LAST => self.state.close((id - TAB_CLOSE) as usize),
            _ => (),
        }
        door_rt::diagnostic(format_args!(
            "DOOR_P12 TERMINAL ACTION id={id} page={:?} tabs={}\n",
            self.state.page,
            self.state.tabs.len()
        ));
        Ok(true)
    }
    /// The console's pane button: hide the navigation pane so the console
    /// takes the whole window, or show it again.
    fn toggle_rail(&mut self) {
        self.layout.rail_hidden = !self.layout.rail_hidden;
        self.sync_columns();
        door_rt::diagnostic(format_args!(
            "DOOR_P16 TERMINAL RAIL hidden={}\n",
            u8::from(self.layout.rail_hidden)
        ));
    }
    /// F11, Alt+Enter or the Full screen button (Windows Terminal's
    /// toggleFullscreen): asked of the window server by `window_update`.
    fn toggle_fullscreen(&mut self) {
        self.fullscreen = !self.fullscreen;
        self.pending_fullscreen = Some(self.fullscreen);
    }
    /// The window should close (its last tab's program ended); taken once.
    pub fn take_close(&mut self) -> bool {
        core::mem::take(&mut self.pending_close)
    }
    fn open(&mut self, p: Profile) {
        self.state.new_tab(p);
    }
    pub fn key(&mut self, code: u32, storage: &mut Storage) -> Result<bool, Error> {
        let scan = code & 511;
        if self.modifier(scan, true) {
            return Ok(false);
        }
        self.actions_open = false;
        let text = (code >> 16) as u8;
        let ctrl = (code >> 9) & door_abi::graphics::CONTROL != 0;
        let shift = (code >> 9) & door_abi::graphics::SHIFT != 0;
        self.shift = shift;
        self.alt = (code >> 9) & door_abi::graphics::ALT != 0;
        if self.clipboard_key(scan, ctrl, shift) {
            return Ok(true);
        }
        if scan == 87 || (self.alt && !ctrl && scan == 28) {
            self.toggle_fullscreen();
            return Ok(true);
        }
        // A program in the active tab's console grid takes the keys typed
        // (Terminal keeps its own shortcuts).
        if self.state.page == Page::Console
            && !self.search_focus
            && self.program_key(scan, text, ctrl, shift)
        {
            return Ok(true);
        }
        if ctrl {
            match scan {
                20 => self.open(Profile::Native),
                17 => {
                    let active = self.state.active;
                    self.state.close(active);
                }
                33 => {
                    self.searching = true;
                    self.search_focus = true;
                }
                31 if self.state.page == Page::Settings => {
                    self.message = match self.save_preferences(storage) {
                        Ok(()) => "Preferences saved".into(),
                        Err(e) => {
                            format!("Save not confirmed: {e:?}. Reopen Terminal before retrying.")
                        }
                    };
                }
                38 => self.state.tab().clear(),
                46 if self.state.page == Page::Console
                    && !self.search_focus
                    && matches!(self.state.tab().profile, Profile::Cmd | Profile::Native) =>
                {
                    self.cmd_interrupt(storage);
                }
                46 if self.state.page == Page::Console && !self.search_focus => {
                    let tab = self.state.tab();
                    tab.input.set(String::new(), false);
                    tab.scroll = 0;
                    tab.output("^C");
                    self.cancel_program();
                }
                51 => self.state.page = Page::Settings,
                15 => {
                    self.state.active = if shift {
                        (self.state.active + self.state.tabs.len() - 1) % self.state.tabs.len()
                    } else {
                        (self.state.active + 1) % self.state.tabs.len()
                    };
                    self.state.page = Page::Console;
                }
                30 => {
                    if self.search_focus {
                        self.query.select_all();
                    } else {
                        self.state.tab().input.select_all();
                    }
                }
                _ => return Ok(false),
            }
            return Ok(true);
        }
        if scan == 1 {
            // Escape first dismisses a selection, as in Windows Terminal.
            if self.state.page == Page::Console && self.state.tab().selection.take().is_some() {
                return Ok(true);
            }
            self.actions_open = false;
            self.search_focus = false;
            self.searching = false;
            self.state.page = Page::Console;
            return Ok(true);
        }
        if self.state.page != Page::Console {
            return Ok(false);
        }
        if self.search_focus {
            if scan == 28 {
                let query = String::from(self.query.text());
                let columns = self.columns();
                self.message = if self.state.tab().search_columns(&query, columns) {
                    "Match found".into()
                } else {
                    "No match".into()
                };
                self.search_focus = false;
            } else {
                edit(&mut self.query, scan, text, shift, 192);
            }
            return Ok(true);
        }
        if matches!(scan, 0x149 | 0x151) {
            let columns = self.columns();
            let tab = self.state.tab();
            tab.scroll_rows(scan == 0x149, 12, columns);
            return Ok(true);
        }
        // Foreign profiles have no input path until their genuine engine exists.
        if !self.state.tab().profile.available() {
            return Ok(false);
        }
        // Input to the shell ends a selection (and snaps to the bottom).
        self.state.tab().selection = None;
        if matches!(self.state.tab().profile, Profile::Cmd | Profile::Native) {
            if self.cmd_key(scan, text, storage) {
                self.state.tab().scroll = 0;
                return Ok(true);
            }
            if self.state.tab().hide_input {
                return Ok(true);
            }
        }
        match scan {
            0x148 => self.state.tab().history(true),
            0x150 => self.state.tab().history(false),
            28 => {
                let remember = self.state.preferences.history;
                let line = self.state.tab().submit(remember);
                self.cmd_submit(&line, storage);
            }
            _ => edit(&mut self.state.tab().input, scan, text, shift, MAX_LINE),
        }
        self.state.tab().scroll = 0;
        Ok(true)
    }
    pub fn draw(&self, v: &mut ui::View<'_>) {
        use door_workspace::caption_tabs::{TabIcon, paint_plus_colored, paint_tab_colored};
        let old = v.palette;
        let rail = self.layout.rail();
        let compact = rail < 100;
        // Acrylic: every background pixel is stored as one premultiplied tint
        // (`rect` stores, it does not blend), so the compositor's blurred
        // backdrop shows evenly; tab row, pane and console are separate tints.
        let colors = self.tab_colors(old);
        let system = old.system();
        v.rect(0, 0, v.width, v.height, colors.selected);
        v.rect(0, 0, v.width, CAPTION, colors.caption);
        let (count, active) = (self.state.tabs.len(), self.state.active);
        let strip = self.layout.strip(count, active);
        for i in strip.range() {
            if let Some(r) = strip.tab(i) {
                let profile = self.state.tabs[i].profile;
                paint_tab_colored(
                    v,
                    r,
                    strip.close(i),
                    TabIcon::Color(profile_icon16(v.scale, profile)),
                    &self.state.tabs[i].title,
                    i == active,
                    colors,
                );
            }
        }
        if let Some((plus, profiles)) = self.new_tab_buttons() {
            paint_plus_colored(v, plus, colors);
            if let Some(d) = profiles {
                if v.hover.is_some_and(|(x, y)| d.contains(x, y)) {
                    let pressed = v.pressed.is_some_and(|(x, y)| d.contains(x, y));
                    let fill = if pressed {
                        colors.pressed
                    } else {
                        colors.hover
                    };
                    v.rounded(d.x, d.y, d.width, d.height, 5, fill);
                }
                v.tiny_icon(
                    d.x + (d.width as i32 - 12) / 2,
                    d.y + (d.height as i32 - 12) / 2,
                    door_workspace::icons::Icon::Chevron,
                    colors.foreground,
                );
            }
        }
        // Pane fills are overlays blended onto the material; a contrast
        // theme lights rows with Highlight (the accent role) instead.
        let (selected_fill, hover_fill, rule) = if system.is_some() {
            (ACCENT, ACCENT, [160, 160, 160, 255])
        } else if old.dark {
            ([20, 20, 20, 20], [13, 13, 13, 13], [36, 36, 36, 36])
        } else {
            ([0, 0, 0, 15], [0, 0, 0, 9], [0, 0, 0, 31])
        };
        // Rows hold one 14 px label: plates grow with it, icons stay 20 px
        // and keep their offset from the plate's centre.
        let (row_h, lift) = ((rail_pitch() - 2) as u32, grow() / 2);
        for (id, top) in rail_rows().into_iter().filter(|_| rail > 0) {
            let selected = matches!(
                (id, self.state.page),
                (1, Page::Home) | (2, Page::Settings) | (3, Page::About) | (20, Page::Profiles)
            ) || (self.state.page == Page::Console
                && (10..=13).contains(&id)
                && self.state.tabs[self.state.active].profile == Profile::ALL[(id - 10) as usize]);
            let hovered = v.hover.is_some_and(|(x, y)| self.target(x, y) == Some(id));
            if selected || hovered {
                v.rounded(
                    10,
                    top,
                    rail - 22,
                    row_h,
                    5,
                    if selected { selected_fill } else { hover_fill },
                );
            }
            let ink = if system.is_some() && (selected || hovered) {
                [255; 4]
            } else {
                FG
            };
            if selected {
                match system {
                    Some(s) => v.rounded_asset(12, top + 6 + lift, 3, 26, 1, sys(s.highlight_text)),
                    None => v.rounded(12, top + 6 + lift, 3, 26, 1, ACCENT),
                }
            }
            match id {
                10..=13 => profile_icon(v, 28, top + 9 + lift, Profile::ALL[(id - 10) as usize]),
                _ => fluent(
                    v,
                    28,
                    top + 9 + lift,
                    match id {
                        1 => 0,
                        2 => 1,
                        3 => 2,
                        _ => 3,
                    },
                    ink,
                ),
            }
            if !compact {
                // Long labels end in "..." (larger text sizes), never a cut glyph.
                let label =
                    door_workspace::explorer_path::ellipsize(rail_label(id), rail - 68, |s| {
                        v.measure(s)
                    });
                v.text_clip_centered(
                    60,
                    (top, row_h),
                    rail - 68,
                    &label,
                    TextStyle::Regular14,
                    ink,
                );
            }
        }
        if !compact && rail > 0 {
            v.text_clip(
                28,
                rail_heading(),
                rail - 36,
                "Profiles",
                TextStyle::Regular14,
                FG,
            );
        }
        if rail > 0 {
            v.rounded(20, rail_rule(), rail - 40, 1, 0, rule);
        }
        if self.state.page != Page::Console {
            // Windows 11 Settings' page background over Mica.
            v.rect(
                rail as i32,
                CONTENT_TOP,
                v.width.saturating_sub(rail),
                v.height.saturating_sub(CONTENT_TOP as u32),
                [250, 247, 244, 255],
            );
        }
        match self.state.page {
            Page::Console => self.draw_home(v),
            Page::Home => self.draw_dashboard(v),
            Page::Settings => self.draw_settings(v),
            Page::About => self.draw_about(v),
            Page::Profiles => self.draw_profiles(v),
        }
        v.palette = old;
    }
    fn draw_home(&self, v: &mut ui::View<'_>) {
        let x = self.layout.rail() as i32;
        let width = self.layout.width - self.layout.rail();
        let tab = &self.state.tabs[self.state.active];
        let light = self.state.preferences.light_console;
        let old = v.palette;
        // The console paints its scheme literally; a contrast theme replaces
        // the scheme with the system colours, as Windows Terminal does.
        let system = old.system();
        v.palette = Palette::light();
        // With transparency effects off the console is drawn fully opaque.
        let bg = if self.reduce_transparency {
            Preferences {
                opacity: 100,
                ..self.state.preferences
            }
            .background()
        } else {
            self.state.preferences.background()
        };
        let ink = if light {
            [45, 35, 25, 255]
        } else {
            [230, 222, 211, 255]
        };
        let secondary = if light {
            [120, 105, 90, 255]
        } else {
            [185, 169, 145, 255]
        };
        // Contrast: Window background, WindowText text, GrayText disabled,
        // Highlight/HighlightText for lit controls.
        let (bg, ink, secondary) = match system {
            Some(s) => (sys(s.window), sys(s.window_text), sys(s.window_text)),
            None => (bg, ink, secondary),
        };
        // `rect` stores the premultiplied tint as-is (no blend with the
        // material beneath), so the console keeps exactly its own opacity.
        v.rect(
            x,
            CONTENT_TOP,
            width,
            self.layout.height.saturating_sub(CONTENT_TOP as u32),
            bg,
        );
        let screen = if self.fullscreen { 12 } else { 11 };
        for (offset, icon, id) in [
            (204, screen, 38),
            (164, 8, 31),
            (124, 9, 36),
            (84, 6, 32),
            (44, 10, 37),
        ] {
            let bx = self.layout.width as i32 - offset;
            if v.hover
                .is_some_and(|(px, py)| self.target(px, py) == Some(id))
            {
                v.rounded_asset(
                    bx,
                    68,
                    36,
                    30,
                    4,
                    match system {
                        Some(s) => sys(s.highlight),
                        None if light => [230, 225, 215, 255],
                        None => [62, 49, 35, 255],
                    },
                );
            }
            let hovered = v
                .hover
                .is_some_and(|(px, py)| self.target(px, py) == Some(id));
            fluent(
                v,
                bx + 8,
                73,
                icon,
                match system {
                    Some(s) if hovered => sys(s.highlight_text),
                    _ => secondary,
                },
            );
        }
        if !tab.profile.available() {
            v.text_clip(
                x + 28,
                127,
                width.saturating_sub(56),
                tab.profile.name(),
                TextStyle::Semibold28,
                ink,
            );
            let title = grow_by(TextStyle::Semibold28);
            v.text_clip(
                x + 28,
                175 + title,
                width.saturating_sub(56),
                tab.profile.status(),
                TextStyle::Semibold14,
                system.map_or([245, 173, 92, 255], |s| sys(s.hotlight)),
            );
            wrapped(
                v,
                x + 28,
                210 + title + grow_by(TextStyle::Semibold14),
                width.saturating_sub(56),
                tab.profile.explanation(),
                ink,
                TextStyle::Regular14,
            );
            let button = Self::requirements_top();
            let button_h = (40 + grow()) as u32;
            v.rounded_asset(
                x + 28,
                button,
                width.saturating_sub(56).min(240),
                button_h,
                5,
                system.map_or([112, 62, 21, 255], |s| sys(s.highlight)),
            );
            v.text_in_box(
                (
                    x + 36,
                    button,
                    width.saturating_sub(56).min(240).saturating_sub(16),
                    button_h,
                ),
                "View runtime requirements",
                TextStyle::Regular14,
                system.map_or([255; 4], |s| sys(s.highlight_text)),
            );
        } else {
            let top = if self.searching {
                136 + grow() as u32
            } else {
                78
            };
            if self.searching {
                let find = self.find_box();
                if let Some(s) = system {
                    // A bordered Window field, as contrast text boxes are.
                    v.rounded_asset(
                        find.x,
                        find.y,
                        find.width,
                        find.height,
                        5,
                        sys(s.window_text),
                    );
                }
                let inset = i32::from(system.is_some());
                v.rounded_asset(
                    find.x + inset,
                    find.y + inset,
                    find.width.saturating_sub(2 * inset as u32),
                    find.height.saturating_sub(2 * inset as u32),
                    5,
                    match system {
                        Some(s) => sys(s.window),
                        None if light => [235, 230, 220, 255],
                        None => [62, 49, 35, 255],
                    },
                );
                v.text_clip(
                    x + 26,
                    99,
                    width.saturating_sub(52),
                    &format!(
                        "Find: {}{}",
                        self.query.text(),
                        if self.search_focus { "|" } else { "" }
                    ),
                    TextStyle::Regular14,
                    ink,
                );
            }
            let grid = self.grid();
            self.report_grid();
            let (cw, rh) = (grid.cw, grid.rh);
            // The status line under the console grows with its 12 px text.
            let status = grow_by(TextStyle::Regular12) as u32;
            let columns = grid.columns as u32;
            // Only the rows shown are wrapped (no full re-wrap per frame).
            let view = tab.viewport(grid.columns, grid.rows);
            let style = self.console_style();
            // Selection: the scheme's text over a plate mixed from its text
            // and background; a contrast theme uses Highlight/HighlightText.
            let highlight = tab.highlight(grid.columns);
            let (select_fill, select_ink) = match system {
                Some(s) => (sys(s.highlight), sys(s.highlight_text)),
                None if light => (mix([252, 250, 247, 255], ink, 25), ink),
                None => (mix(DARK_CONSOLE, ink, 35), ink),
            };
            if let Some(console) = tab.console.as_ref().filter(|_| tab.scroll == 0) {
                // A program runs in the tab: its console grid fills the rows.
                draw_console_grid(
                    v,
                    (grid.x, top as i32),
                    (cw, rh),
                    grid.rows,
                    &console.screen,
                    style,
                    (ink, self.state.preferences.scheme()),
                );
            } else {
                for (row, (line, wrap)) in tab
                    .rows_in(view.clone(), grid.columns)
                    .into_iter()
                    .enumerate()
                {
                    let y = top as i32 + row as i32 * rh as i32;
                    let selected = highlight
                        .and_then(|h| h.columns(view.start + row, tab.serial(line), wrap))
                        .map(|(from, to)| (from, to.min(grid.columns - 1), select_ink));
                    if let Some((from, to, _)) = selected {
                        v.rect(
                            grid.x + (from as u32 * cw) as i32,
                            y,
                            (to - from + 1) as u32 * cw,
                            rh,
                            select_fill,
                        );
                    }
                    mono(
                        v,
                        grid.x,
                        y,
                        tab.row_text(line, wrap, grid.columns),
                        cw,
                        columns,
                        style,
                        ink,
                        selected,
                    );
                }
                let prompt_y = if tab.scroll == 0 {
                    top as i32 + view.len() as i32 * rh as i32
                } else {
                    self.layout.height as i32 - 52 - status as i32
                };
                // Columns are characters (pasted text need not be ASCII).
                // The engine's prompt (cmd.exe's PROMPT), else Doorsh's "dir> ";
                // nothing while a command runs or waits for a key.
                let lead = match (&tab.prompt, tab.hide_input) {
                    (_, true) => String::new(),
                    (Some(p), false) => p.clone(),
                    (None, false) => format!("{}> ", tab.cwd),
                };
                let typed = if tab.hide_input { "" } else { tab.input.text() };
                let prompt = ui::display_ascii(&format!("{lead}{typed}"));
                let input = typed;
                let cursor = lead.chars().count()
                    + input[..tab.input.cursor().min(input.len())].chars().count();
                let offset = cursor.saturating_sub(columns.saturating_sub(1) as usize);
                let shown = prompt
                    .char_indices()
                    .nth(offset)
                    .map_or("", |(at, _)| &prompt[at..]);
                mono(
                    v,
                    x + 18,
                    prompt_y,
                    shown,
                    cw,
                    columns,
                    style,
                    if light || system.is_some() {
                        ink
                    } else {
                        [231, 190, 111, 255]
                    },
                    None,
                );
                if !self.search_focus {
                    v.rect(
                        x + 18 + cursor.saturating_sub(offset) as i32 * cw as i32,
                        prompt_y + 2,
                        2,
                        rh - 4,
                        ink,
                    );
                }
            }
            // Windows Terminal's scrollbar is hidden at rest: a 16 px track
            // with its thumb appears while the pointer is over the right edge.
            let track = self.layout.height.saturating_sub(top + 64 + status);
            let over = v
                .hover
                .is_some_and(|(px, py)| self.target(px, py) == Some(35));
            if over && track > 0 {
                let total = tab.total_rows(grid.columns).max(view.len()).max(1);
                let thumb = ((track as usize * view.len().max(1) / total) as u32)
                    .clamp(track.min(24), track);
                let range = total.saturating_sub(view.len()).max(1);
                let thumb_y =
                    top as i32 + ((track - thumb) as usize * view.start.min(range) / range) as i32;
                let bar_x = self.layout.width as i32 - SCROLLBAR as i32;
                let (plate, thumb_ink) = match system {
                    Some(s) => (sys(s.window), sys(s.window_text)),
                    None => (
                        mix(self.state.preferences.scheme(), secondary, 12),
                        secondary,
                    ),
                };
                v.rounded_asset(bar_x, top as i32, SCROLLBAR, track, 4, plate);
                v.rounded_asset(bar_x + 5, thumb_y, 6, thumb, 3, thumb_ink);
            }
        }
        v.small_clip(
            x + 18,
            self.layout.height as i32 - 23 - grow_by(TextStyle::Regular12),
            width.saturating_sub(36),
            &self.message,
            secondary,
        );
        if self.actions_open {
            // Menus follow the application theme, independently of the console scheme.
            v.palette = old;
            let m = self.actions_menu();
            let pitch = 36 + grow();
            v.elevation(m.x, m.y, m.width, m.height, 28);
            if system.is_some() {
                // Contrast menus are outlined in WindowText.
                v.rounded(
                    m.x - 1,
                    m.y - 1,
                    m.width + 2,
                    m.height + 2,
                    7,
                    [160, 160, 160, 255],
                );
            }
            v.rounded(m.x, m.y, m.width, m.height, 6, [249, 249, 249, 255]);
            for (i, label) in ["Find in output", "Clear output", "Scroll to bottom"]
                .iter()
                .enumerate()
            {
                let top = m.y + i as i32 * pitch;
                let hovered = v
                    .hover
                    .is_some_and(|(x, y)| self.target(x, y) == Some([31, 32, 33][i]));
                if hovered {
                    let plate = if system.is_some() {
                        ACCENT
                    } else {
                        [235, 235, 235, 255]
                    };
                    v.rounded(m.x + 4, top + 2, m.width - 8, pitch as u32 - 4, 4, plate);
                }
                let ink = if hovered && system.is_some() {
                    [255; 4]
                } else {
                    FG
                };
                v.text_clip_centered(
                    m.x + 14,
                    (top, pitch as u32),
                    m.width - 28,
                    label,
                    TextStyle::Regular14,
                    ink,
                );
            }
        }
        v.palette = old;
    }
    fn console_style(&self) -> TextStyle {
        match self.state.preferences.font {
            0 => TextStyle::Regular12,
            2 => TextStyle::Regular20,
            _ => TextStyle::Regular14,
        }
    }
    fn heading(&self, v: &mut ui::View<'_>, title: &str, subtitle: &str) {
        let x = self.layout.rail() as i32 + 28;
        let width = self.layout.width.saturating_sub(self.layout.rail() + 56);
        v.text_clip(x, 68, width, title, TextStyle::Semibold28, FG);
        v.small_clip(
            x,
            109 + grow_by(TextStyle::Semibold28),
            width,
            subtitle,
            MUTED,
        );
    }
    fn card(&self, v: &mut ui::View<'_>, top: i32, title: &str, description: &str) {
        let x = self.layout.rail() as i32 + 28;
        let width = self.layout.width.saturating_sub(self.layout.rail() + 56);
        let h = (70 + card_growth()) as u32;
        v.rounded(x, top + 1, width, h, 6, [224, 224, 224, 255]);
        v.rounded(x, top, width, h, 6, [255; 4]);
        v.text_clip(
            x + 18,
            top + 12,
            width.saturating_sub(36),
            title,
            TextStyle::Semibold14,
            FG,
        );
        v.small_clip(
            x + 18,
            top + 39 + grow(),
            width.saturating_sub(36),
            description,
            MUTED,
        );
    }
    fn draw_settings(&self, v: &mut ui::View<'_>) {
        self.heading(v, "Settings", "Appearance and interaction");
        let x = self.layout.rail() as i32 + 28;
        let width = self.layout.width.saturating_sub(self.layout.rail() + 56);
        let opacity = format!("{}%", self.state.preferences.opacity);
        let cards = settings_cards();
        for ((id, top, card_h), (title, description, value)) in cards.into_iter().zip([
            (
                "Color scheme",
                "Choose the colors used by your terminal sessions.",
                if self.state.preferences.light_console {
                    "Light"
                } else {
                    "Midnight"
                },
            ),
            (
                "Text size",
                "Fixed cell text at the native display scale.",
                ["12 px", "14 px", "20 px"][self.state.preferences.font as usize],
            ),
            (
                "Command history",
                "Remember commands for this session.",
                if self.state.preferences.history {
                    "On"
                } else {
                    "Off"
                },
            ),
            (
                "Background opacity",
                "Acrylic tint over the cached desktop backdrop.",
                opacity.as_str(),
            ),
        ]) {
            // Cards hold a 14 px title over a 12 px description and grow
            // with both; the value box and its text widen and heighten too.
            let (h, extra) = (card_h as u32, card_h - 70);
            v.rounded(x, top + 1, width, h + 2, 6, [224, 224, 224, 255]);
            let hovered = v
                .hover
                .is_some_and(|(px, py)| self.target(px, py) == Some(id));
            v.rounded(
                x,
                top,
                width,
                h,
                6,
                if hovered {
                    if v.palette.dark {
                        [235, 235, 235, 255]
                    } else {
                        [245, 245, 245, 255]
                    }
                } else {
                    [255; 4]
                },
            );
            let value_text = door_workspace::palette::text_len(90);
            let control_width = if id == 42 {
                76 + door_workspace::palette::text_len(32) as i32 - 32
            } else {
                128 + value_text as i32 - 90
            };
            let right = x + width as i32 - control_width - 16;
            v.text_clip(
                x + 16,
                top + 12,
                width.saturating_sub(control_width as u32 + 48),
                title,
                TextStyle::Semibold14,
                FG,
            );
            v.small_clip(
                x + 16,
                top + 39 + grow(),
                width.saturating_sub(control_width as u32 + 48),
                description,
                MUTED,
            );
            if id == 42 {
                let on = self.state.preferences.history;
                let middle = extra / 2;
                let value_w = control_width - 44;
                let small = door_workspace::palette::text_grow_i(0, TextStyle::Regular12);
                v.text_centered(
                    right - 12,
                    top + 19 + middle - small / 2,
                    value_w as u32,
                    value,
                    TextStyle::Regular12,
                    MUTED,
                );
                // The toggle keeps its size, centred in the grown card.
                let track = right + control_width - 56;
                v.rounded(
                    track,
                    top + 23 + middle,
                    44,
                    22,
                    11,
                    if on { ACCENT } else { [160, 160, 160, 255] },
                );
                let knob = match v.palette.system() {
                    Some(s) => sys(if on { s.highlight_text } else { s.window }),
                    None => [255; 4],
                };
                v.rounded_asset(
                    track + if on { 24 } else { 4 },
                    top + 27 + middle,
                    14,
                    14,
                    7,
                    knob,
                );
            } else {
                let box_h = (32 + grow()) as u32;
                let box_y = top + (card_h - box_h as i32) / 2;
                v.rounded(
                    right,
                    box_y,
                    control_width as u32,
                    box_h,
                    4,
                    [242, 242, 242, 255],
                );
                v.text_clip_centered(
                    right + 12,
                    (box_y, box_h),
                    value_text,
                    value,
                    TextStyle::Regular14,
                    FG,
                );
                v.small(
                    right + 16 + value_text as i32,
                    top + 26 + extra / 2,
                    ">",
                    MUTED,
                );
            }
        }
        // Windows-style fixed action area stays reachable at the supported size.
        let (footer, save) = settings_footer(self.layout.height);
        v.rect(x, footer, width, 1, [218, 218, 218, 255]);
        let button_x = (x + width as i32 - 120).max(x);
        v.rounded(button_x, save.y, 120.min(width), save.height, 4, ACCENT);
        v.text_in_box(
            (button_x, save.y, 120.min(width), save.height),
            "Save",
            TextStyle::Semibold14,
            [255; 4],
        );
        v.small_clip(
            x,
            footer + 24 + grow() / 2,
            width.saturating_sub(140),
            "Ctrl+S to save",
            MUTED,
        );
        let message = cards[3].1 + cards[3].2 + 20;
        if footer > message + 40 {
            wrapped(
                v,
                x,
                message,
                width,
                &self.message,
                MUTED,
                TextStyle::Regular14,
            );
        }
    }
    fn draw_about(&self, v: &mut ui::View<'_>) {
        self.heading(
            v,
            "Terminal",
            "One workspace for your command-line sessions.",
        );
        let (first, pitch) = (150 + heading_growth(), 88 + card_growth());
        self.card(
            v,
            first,
            "Version 0.1.0 - Native preview",
            "Built for Doors using the native window and storage services.",
        );
        self.card(
            v,
            first + pitch,
            "Native shell",
            "Real file operations, independent tabs, command history and search.",
        );
        self.card(
            v,
            first + 2 * pitch,
            "CMD and PowerShell compatibility",
            "Pending genuine engines and runtime APIs. No command emulation claim.",
        );
        self.card(v, first + 3 * pitch, "WSL (Ubuntu)", "Work in progress");
        wrapped(
            v,
            self.layout.rail() as i32 + 28,
            first + 3 * pitch + 70 + card_growth() + 20,
            self.layout.width.saturating_sub(self.layout.rail() + 56),
            "Windows application support is currently bounded. DLLs are libraries loaded by compatible programs; they are not shell scripts. Full batch, PowerShell, pseudoconsole and split-pane support remain open.",
            MUTED,
            TextStyle::Regular14,
        );
    }
    /// The window's title: the active tab's.
    pub fn window_title(&self) -> &str {
        self.state
            .tabs
            .get(self.state.active)
            .map_or("Terminal", |t| t.title.as_str())
    }
    /// Home's layout (Windows 11 Settings rhythm: a 14 px semibold header
    /// 27 px above its cards, cards 4 px apart, 31 px from the next
    /// header): "New session" tiles (50..=53, 2 x 2 in narrow windows),
    /// "Open sessions" cards (`SESSION`..) with their close buttons (`SESSION_CLOSE`..),
    /// and "Recent folders" cards (80..=87). Paint, hits and semantics share
    /// it; cards that would leave the window are dropped.
    pub(crate) fn home_layout(&self) -> HomeLayout {
        use door_workspace::explorer_layout::Bounds;
        let x = self.layout.rail() as i32 + 28;
        let width = self.layout.width.saturating_sub(self.layout.rail() + 56);
        let bottom = self.layout.height as i32 - 12;
        let header = |top: i32| (top, top + 27 + grow_by(TextStyle::Semibold14));
        let card_h = (69 + card_growth()) as u32;
        let mut out = HomeLayout {
            headers: [None; 3],
            targets: Vec::new(),
        };
        let (h, mut y) = header(116 + grow_by(TextStyle::Semibold28));
        out.headers[0] = Some((x, h));
        let columns: u32 = if width >= 4 * 150 + 24 { 4 } else { 2 };
        let tile_w = width.saturating_sub(8 * (columns - 1)) / columns;
        let tile_h = (60 + grow()) as u32;
        for i in 0..4u32 {
            let (col, row) = (i % columns, i / columns);
            out.targets.push((
                50 + i,
                Bounds {
                    x: x + (col * (tile_w + 8)) as i32,
                    y: y + (row * (tile_h + 8)) as i32,
                    width: tile_w,
                    height: tile_h,
                },
            ));
        }
        y += ((4 / columns) * (tile_h + 8)) as i32 - 8;
        // Wide windows set the lists side by side, as Settings' Home does.
        let wide = width >= 720 && !self.recent.is_empty();
        let column = if wide { (width - 16) / 2 } else { width };
        let list = |section: usize, first: u32, count: usize, out: &mut HomeLayout, y: &mut i32| {
            if count == 0 || *y + 31 + 27 + card_h as i32 > bottom {
                return;
            }
            let x = if wide && section == 2 {
                x + (column + 16) as i32
            } else {
                x
            };
            let (h, top) = header(*y + 31);
            out.headers[section] = Some((x, h));
            *y = top;
            // Open sessions: every tab, until the window ends; recent
            // folders: the eight most recent.
            for i in 0..if section == 1 { count } else { count.min(8) } {
                if *y + card_h as i32 > bottom {
                    break;
                }
                let card = Bounds {
                    x,
                    y: *y,
                    width: column,
                    height: card_h,
                };
                out.targets.push((first + i as u32, card));
                if section == 1 && self.state.tabs.len() > 1 {
                    let close = Bounds {
                        x: card.x + card.width as i32 - 17 - 32,
                        y: card.y + (card_h as i32 - 32) / 2,
                        width: 32,
                        height: 32,
                    };
                    out.targets.push((SESSION_CLOSE + i as u32, close));
                }
                *y += card_h as i32 + 4;
            }
            *y -= 4;
        };
        let lists = y;
        list(1, SESSION, self.state.tabs.len(), &mut out, &mut y);
        if wide {
            y = lists;
        }
        list(2, 80, self.recent.len(), &mut out, &mut y);
        out
    }
    fn draw_dashboard(&self, v: &mut ui::View<'_>) {
        // Windows 11 Settings card colours (WinUI resources over Mica).
        const CARD: [u8; 4] = [253, 252, 251, 255];
        const CARD_HOVER: [u8; 4] = [249, 248, 246, 255];
        const CARD_PRESSED: [u8; 4] = [246, 245, 243, 255];
        const BORDER: [u8; 4] = [233, 230, 228, 255];
        const TEXT: [u8; 4] = [26, 26, 26, 255];
        const SECONDARY: [u8; 4] = [96, 96, 96, 255];
        let x = self.layout.rail() as i32 + 28;
        let width = self.layout.width.saturating_sub(self.layout.rail() + 56);
        v.text_clip(x, 68, width, "Home", TextStyle::Semibold28, TEXT);
        let layout = self.home_layout();
        for (header, label) in
            layout
                .headers
                .iter()
                .zip(["New session", "Open sessions", "Recent folders"])
        {
            if let Some((hx, y)) = header {
                v.text_clip(
                    *hx,
                    *y,
                    width.saturating_sub((*hx - x) as u32),
                    label,
                    TextStyle::Semibold14,
                    TEXT,
                );
            }
        }
        let hovered = v.hover.and_then(|(px, py)| self.target(px, py));
        let pressed = v.pressed.and_then(|(px, py)| self.target(px, py));
        let fill = |id: u32| {
            if pressed == Some(id) {
                CARD_PRESSED
            } else if hovered == Some(id) {
                CARD_HOVER
            } else {
                CARD
            }
        };
        let card =
            |v: &mut ui::View<'_>, r: door_workspace::explorer_layout::Bounds, color: [u8; 4]| {
                v.rounded(r.x, r.y, r.width, r.height, 4, BORDER);
                v.rounded(
                    r.x + 1,
                    r.y + 1,
                    r.width.saturating_sub(2),
                    r.height.saturating_sub(2),
                    3,
                    color,
                );
            };
        let lift = grow() / 2;
        for (id, r) in &layout.targets {
            let (id, r) = (*id, *r);
            match id {
                50..=53 => {
                    let p = Profile::ALL[(id - 50) as usize];
                    card(v, r, fill(id));
                    profile_icon(v, r.x + 19, r.y + (r.height as i32 - 20) / 2, p);
                    let ink = if p.available() { TEXT } else { SECONDARY };
                    v.text_clip_centered(
                        r.x + 51,
                        (r.y, r.height),
                        r.width.saturating_sub(63),
                        p.name(),
                        TextStyle::Regular14,
                        ink,
                    );
                }
                SESSION..=SESSION_LAST => {
                    let i = (id - SESSION) as usize;
                    let tab = &self.state.tabs[i];
                    // The close button's own hover leaves the card plain.
                    let over_close = matches!(hovered, Some(c) if c == SESSION_CLOSE + i as u32);
                    card(v, r, if over_close { CARD } else { fill(id) });
                    profile_icon(v, r.x + 19, r.y + (r.height as i32 - 20) / 2, tab.profile);
                    let right = if self.state.tabs.len() > 1 {
                        17 + 32 + 12
                    } else {
                        17
                    };
                    let active = i == self.state.active;
                    let tag = if active { 64 } else { 0 };
                    let room = r.width.saturating_sub(59 + right + tag);
                    let place = if tab.profile.available() {
                        self.shown_path(&tab.cwd)
                    } else {
                        String::from(tab.profile.status())
                    };
                    self.two_lines(v, r, room, &tab.title, &place, TEXT, SECONDARY);
                    if active {
                        v.small_clip(
                            r.x + r.width as i32 - right as i32 - tag as i32 + 8,
                            r.y + (r.height as i32 - 16) / 2 + lift,
                            tag,
                            "Active",
                            SECONDARY,
                        );
                    }
                }
                SESSION_CLOSE..=SESSION_CLOSE_LAST => {
                    if hovered == Some(id) {
                        v.rounded(
                            r.x,
                            r.y,
                            r.width,
                            r.height,
                            4,
                            if pressed == Some(id) {
                                [237, 235, 233, 255]
                            } else {
                                [242, 240, 238, 255]
                            },
                        );
                    }
                    // Windows 11's 16 px close glyph in a 32 px button.
                    let mask = match v.scale {
                        3 => door_workspace::asset!(
                            "../../../assets/icons/windows-shell/explorer-close.48.mask"
                        ),
                        2 => door_workspace::asset!(
                            "../../../assets/icons/windows-shell/explorer-close.32.mask"
                        ),
                        _ => door_workspace::asset!(
                            "../../../assets/icons/windows-shell/explorer-close.16.mask"
                        ),
                    };
                    v.mask_pixels(r.x + 8, r.y + 8, 16, mask, TEXT, None);
                }
                _ => {
                    let path = &self.recent[(id - 80) as usize];
                    card(v, r, fill(id));
                    v.mask_pixels(
                        r.x + 19,
                        r.y + (r.height as i32 - 20) / 2,
                        20,
                        folder_glyph(v.scale),
                        TEXT,
                        None,
                    );
                    let shown = self.shown_path(path);
                    let name = String::from(
                        shown
                            .trim_end_matches('\\')
                            .rsplit('\\')
                            .next()
                            .filter(|n| !n.is_empty())
                            .unwrap_or(&shown),
                    );
                    self.two_lines(
                        v,
                        r,
                        r.width.saturating_sub(59 + 17),
                        &name,
                        &shown,
                        TEXT,
                        SECONDARY,
                    );
                }
            }
        }
    }
    /// A folder as File Explorer names it, the profile as C:\Users\<name>.
    pub(crate) fn shown_path(&self, native: &str) -> String {
        let home = &self.state.home;
        if native == home || native.starts_with(&format!("{home}/")) {
            return format!(
                "C:\\Users\\{}{}",
                self.user,
                native[home.len()..].replace('/', "\\")
            );
        }
        door_workspace::explorer_path::display(native)
    }
    /// A card's title over its description, centred as a pair at the text
    /// column (59 px).
    #[allow(clippy::too_many_arguments)]
    fn two_lines(
        &self,
        v: &mut ui::View<'_>,
        r: door_workspace::explorer_layout::Bounds,
        room: u32,
        title: &str,
        description: &str,
        ink: [u8; 4],
        secondary: [u8; 4],
    ) {
        let line = 20 + grow();
        let small = 16 + grow_by(TextStyle::Regular12);
        let top = r.y + (r.height as i32 - line - small) / 2;
        let title = door_workspace::explorer_path::ellipsize(title, room, |s| v.measure(s));
        v.text_clip(r.x + 59, top, room, &title, TextStyle::Regular14, ink);
        v.small_clip(r.x + 59, top + line, room, description, secondary);
    }
    /// The last folders Doorsh sessions worked in (most recent first),
    /// kept in `terminal-recent.conf` beside the preferences.
    fn note_recent(&mut self, path: &str) {
        if path == self.state.home || path.is_empty() || path.chars().any(char::is_control) {
            return;
        }
        self.recent.retain(|p| p != path);
        self.recent.insert(0, String::from(path));
        self.recent.truncate(MAX_RECENT);
        self.recent_dirty = true;
    }
    /// Note folder changes of Doorsh tabs and save the list when it changed.
    pub fn track_recent(&mut self, storage: &mut Storage) {
        let changes: Vec<String> = self
            .state
            .tabs
            .iter()
            .filter(|t| t.profile == Profile::Native)
            .filter(|t| {
                !self
                    .seen_cwd
                    .iter()
                    .any(|(id, cwd)| *id == t.id && *cwd == t.cwd)
            })
            .map(|t| t.cwd.clone())
            .collect();
        self.seen_cwd = self
            .state
            .tabs
            .iter()
            .map(|t| (t.id, t.cwd.clone()))
            .collect();
        for path in changes {
            self.note_recent(&path);
        }
        if self.recent_dirty {
            self.recent_dirty = false;
            let mut text = self.recent.join("\n");
            text.push('\n');
            let _ = Self::replace_file(storage, &self.recent_path, text.as_bytes());
        }
    }
    fn load_recent(&mut self, storage: &mut Storage) {
        let Ok(file) = storage.open(
            &self.recent_path,
            Disposition::OpenExisting,
            READ,
            SHARE_ALL,
        ) else {
            return;
        };
        let mut data = alloc::vec![0; 4096];
        let n = storage.read(&file, 0, &mut data).unwrap_or(0);
        let _ = storage.close(file);
        let text = String::from_utf8_lossy(&data[..n]);
        self.recent = text
            .lines()
            .filter(|l| l.starts_with('/') && !l.chars().any(char::is_control))
            .take(MAX_RECENT)
            .map(String::from)
            .collect();
    }
    /// Replace a small file's contents atomically (created when missing).
    fn replace_file(storage: &mut Storage, path: &str, bytes: &[u8]) -> Result<(), Error> {
        let file = storage.open(path, Disposition::OpenAlways, READ | WRITE, SHARE_ALL)?;
        let result = (|| {
            let info = storage.info(&file)?;
            let staged = storage.prepare_replace(&file, info, bytes)?;
            storage.commit_replace(staged).map(|_| ())
        })();
        let closed = storage.close(file);
        result.and(closed)
    }
    fn draw_profiles(&self, v: &mut ui::View<'_>) {
        self.heading(
            v,
            "Profiles",
            "Select a profile in the sidebar to open its session.",
        );
        let x = self.layout.rail() as i32 + 28;
        let width = self.layout.width.saturating_sub(self.layout.rail() + 56);
        // A 14 px title over up to three 12 px lines of explanation.
        let extra = grow() + 3 * grow_by(TextStyle::Regular12);
        for (index, p) in Profile::ALL.iter().enumerate() {
            let top = 148 + heading_growth() + index as i32 * (104 + extra);
            v.rounded(x, top, width, (92 + extra) as u32, 6, [255; 4]);
            profile_icon(v, x + 16, top + 16, *p);
            v.text_clip(
                x + 48,
                top + 12,
                width.saturating_sub(64),
                &format!("{} - {}", p.name(), p.status()),
                TextStyle::Semibold14,
                FG,
            );
            wrapped(
                v,
                x + 16,
                top + 42 + grow(),
                width.saturating_sub(32),
                p.explanation(),
                MUTED,
                TextStyle::Regular12,
            );
        }
    }
}
fn edit(line: &mut LineEdit, scan: u32, text: u8, shift: bool, limit: usize) {
    match scan {
        14 => line.backspace(),
        0x153 => line.delete(),
        0x14b => line.horizontal(false, shift, false),
        0x14d => line.horizontal(true, shift, false),
        0x147 => line.place(0, shift),
        0x14f => line.place(line.text().len(), shift),
        _ if (32..=126).contains(&text) => line.insert_bounded(text as char, limit),
        _ => (),
    }
}
/// One console row, a character per cell; `selected` cells (first, last,
/// ink) draw in the selection's ink.
#[allow(clippy::too_many_arguments)]
fn mono(
    v: &mut ui::View<'_>,
    x: i32,
    y: i32,
    text: &str,
    cw: u32,
    columns: u32,
    style: TextStyle,
    color: [u8; 4],
    selected: Option<(usize, usize, [u8; 4])>,
) {
    for (index, c) in text.chars().take(columns as usize).enumerate() {
        let mut bytes = [0; 4];
        let color = match selected {
            Some((from, to, ink)) if (from..=to).contains(&index) => ink,
            _ => color,
        };
        v.monospace_text(
            x + index as i32 * cw as i32,
            y,
            cw,
            c.encode_utf8(&mut bytes),
            style,
            color,
        );
    }
}
/// Opaque `a` blended toward `b` by `percent`.
/// Windows Terminal's Campbell scheme: the 16 console colours (RGB).
const CAMPBELL: [[u8; 3]; 16] = [
    [12, 12, 12],
    [197, 15, 31],
    [19, 161, 14],
    [193, 156, 0],
    [0, 55, 218],
    [136, 23, 152],
    [58, 150, 221],
    [204, 204, 204],
    [118, 118, 118],
    [231, 72, 86],
    [22, 198, 12],
    [249, 241, 165],
    [59, 120, 255],
    [180, 0, 158],
    [97, 214, 214],
    [242, 242, 242],
];
/// A cell colour as a BGRA pixel: the 16 scheme colours (bold makes the
/// first eight bright, as Windows Terminal draws bold), the xterm 6x6x6
/// cube and grey ramp, or 24-bit; None for the default (the scheme's text
/// or background).
fn cell_pixel(color: door_vt::Color, bright: bool) -> Option<[u8; 4]> {
    let rgb = match color {
        door_vt::Color::Default => return None,
        door_vt::Color::Indexed(n) if n < 16 => {
            let n = if bright && n < 8 { n + 8 } else { n };
            CAMPBELL[n as usize]
        }
        door_vt::Color::Indexed(n) if n < 232 => {
            let i = n - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            [level(i / 36), level(i / 6 % 6), level(i % 6)]
        }
        door_vt::Color::Indexed(n) => {
            let v = 8 + (n - 232) * 10;
            [v, v, v]
        }
        door_vt::Color::Rgb(r, g, b) => [r, g, b],
    };
    Some([rgb[2], rgb[1], rgb[0], 255])
}
/// The running program's console grid: each cell's background, character,
/// underline and strike-through in its colours (bold as the bright colour,
/// dim mixed toward the background, inverse swapped), and the cursor as
/// Windows Terminal's default bar. `colors` are the scheme's text and
/// background.
fn draw_console_grid(
    v: &mut ui::View<'_>,
    (x, top): (i32, i32),
    (cw, rh): (u32, u32),
    rows: usize,
    screen: &door_vt::Screen,
    style: TextStyle,
    (ink, background): ([u8; 4], [u8; 4]),
) {
    use door_vt::attr;
    for y in 0..rows.min(screen.rows()) {
        let py = top + y as i32 * rh as i32;
        for (column, cell) in screen.row(y).iter().enumerate() {
            let px = x + column as i32 * cw as i32;
            let bold = cell.attrs & attr::BOLD != 0;
            let mut fg = cell_pixel(cell.fg, bold).unwrap_or(ink);
            let mut bg = cell_pixel(cell.bg, false);
            if cell.attrs & attr::INVERSE != 0 {
                let old = fg;
                fg = bg.unwrap_or(background);
                bg = Some(old);
            }
            if cell.attrs & attr::DIM != 0 {
                fg = mix(fg, bg.unwrap_or(background), 40);
            }
            if let Some(bg) = bg {
                v.rect(px, py, cw, rh, bg);
            }
            if cell.attrs & attr::HIDDEN != 0 {
                continue;
            }
            if cell.ch != ' ' {
                let mut bytes = [0; 4];
                v.monospace_text(px, py, cw, cell.ch.encode_utf8(&mut bytes), style, fg);
            }
            if cell.attrs & (attr::UNDERLINE | attr::DOUBLE_UNDERLINE) != 0 {
                v.rect(px, py + rh as i32 - 2, cw, 1, fg);
            }
            if cell.attrs & attr::STRIKE != 0 {
                v.rect(px, py + rh as i32 / 2, cw, 1, fg);
            }
        }
    }
    if screen.state().visible {
        let (cx, cy) = screen.cursor();
        if cy < rows {
            v.rect(
                x + cx as i32 * cw as i32,
                top + cy as i32 * rh as i32,
                2,
                rh,
                ink,
            );
        }
    }
}
fn mix(a: [u8; 4], b: [u8; 4], percent: u16) -> [u8; 4] {
    let blend =
        |a: u8, b: u8| ((u16::from(a) * (100 - percent) + u16::from(b) * percent + 50) / 100) as u8;
    [blend(a[0], b[0]), blend(a[1], b[1]), blend(a[2], b[2]), 255]
}
fn wrapped(
    v: &mut ui::View<'_>,
    x: i32,
    mut y: i32,
    width: u32,
    text: &str,
    color: [u8; 4],
    style: TextStyle,
) {
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && v.measure(&format!("{line} {word}")) > width {
            v.text_clip(x, y, width, &line, style, color);
            y += door_workspace::palette::text_len(style.line_height()) as i32;
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    v.text_clip(x, y, width, &line, style, color);
}
/// A system colour (RGB) as an opaque BGRA pixel.
fn sys(rgb: [u8; 3]) -> [u8; 4] {
    [rgb[2], rgb[1], rgb[0], 255]
}
/// 20 px profile icon (the rail and the Profiles page).
fn profile_icon(v: &mut ui::View<'_>, x: i32, y: i32, profile: Profile) {
    macro_rules! pixels {
        ($dir:literal, $name:literal, $one:literal) => {
            match v.scale {
                3 => &door_workspace::asset!(concat!("../../../assets/", $dir, $name, ".60.bgra"))[..],
                2 => &door_workspace::asset!(concat!("../../../assets/", $dir, $name, ".40.bgra"))[..],
                _ => &door_workspace::asset!(concat!("../../../assets/", $dir, $name, $one))[..],
            }
        };
    }
    let pixels = match profile {
        Profile::PowerShell => pixels!("icons/windows/", "terminal-powershell", ".bgra"),
        Profile::Cmd => pixels!("icons/windows/", "terminal-cmd", ".bgra"),
        Profile::Dsl => pixels!("icons/windows/", "terminal-wsl", ".bgra"),
        Profile::Native => pixels!("branding/", "doorsh", ".20.bgra"),
    };
    v.color_pixels(x, y, 20, pixels);
}
fn fluent(v: &mut ui::View<'_>, x: i32, y: i32, name: u8, color: [u8; 4]) {
    macro_rules! glyph {
        ($name:literal) => {
            match v.scale {
                3 => &door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-shell/",
                    $name,
                    ".60.mask"
                ))[..],
                2 => &door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-shell/",
                    $name,
                    ".40.mask"
                ))[..],
                _ => &door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-shell/",
                    $name,
                    ".20.mask"
                ))[..],
            }
        };
    }
    let mask = match name {
        0 => glyph!("terminal-home"),
        1 => glyph!("settings"),
        2 => glyph!("setting-info"),
        3 => glyph!("explorer-add"),
        4 => glyph!("explorer-close"),
        5 => glyph!("explorer-search"),
        6 => glyph!("explorer-delete"),
        8 => glyph!("terminal-book"),
        9 => glyph!("explorer-pane"),
        10 => glyph!("explorer-more"),
        11 => glyph!("terminal-full-screen"),
        12 => glyph!("terminal-back-to-window"),
        _ => glyph!("terminal-end"),
    };
    v.mask_pixels(x, y, 20, mask, color, None);
}

/// 16 px caption-tab icon for a profile at native density: each shell's
/// own mark (Doorsh is project artwork; DSL shows the Windows 11 WSL logo).
fn profile_icon16(scale: u32, profile: Profile) -> &'static [u8] {
    macro_rules! icon {
        ($dir:literal, $name:literal) => {
            match scale {
                3 => &door_workspace::asset!(concat!("../../../assets/", $dir, $name, ".48.bgra"))[..],
                2 => &door_workspace::asset!(concat!("../../../assets/", $dir, $name, ".32.bgra"))[..],
                _ => &door_workspace::asset!(concat!("../../../assets/", $dir, $name, ".16.bgra"))[..],
            }
        };
    }
    match profile {
        Profile::PowerShell => icon!("icons/windows/", "terminal-powershell"),
        Profile::Cmd => icon!("icons/windows/", "terminal-cmd"),
        Profile::Dsl => icon!("icons/windows/", "terminal-wsl"),
        Profile::Native => icon!("branding/", "doorsh"),
    }
}

#[cfg(test)]
mod pointer_tests {
    use super::*;
    #[test]
    fn text_cursor_excludes_buttons_scrollbar_menu_and_nontext_profiles() {
        let mut terminal = Terminal::for_home("/".into());
        terminal.configure(1050, 650);
        assert!(terminal.text_hit(400, 200));
        for (x, y) in [
            (-1, 200),
            (400, -1),
            (1050, 200),
            (1038, 200),
            (1010, 95),
            (400, 640),
        ] {
            assert!(
                !terminal.text_hit(x, y),
                "Control or out-of-bounds point {x},{y}"
            );
        }
        terminal.searching = true;
        assert!(terminal.text_hit(400, 110));
        terminal.actions_open = true;
        assert!(terminal.menu_open());
        assert!(!terminal.text_hit(900, 150));
        assert!(!terminal.text_hit(400, 200));
        terminal.actions_open = false;
        terminal.state.new_tab(Profile::PowerShell);
        assert!(!terminal.text_hit(400, 200));
    }
    /// A 1050x650 console: grid at x 226, top 78, 9x21 px cells.
    fn console(text: &str) -> Terminal {
        let mut t = Terminal::for_home("/".into());
        t.configure(1050, 650);
        t.state.tab().clear();
        t.state.tab().output(text);
        t
    }
    fn cell(column: i32, row: i32) -> (i32, i32) {
        (226 + column * 9 + 4, 78 + row * 21 + 10)
    }
    fn copied(t: &Terminal) -> Option<&str> {
        match t.clipboard.back() {
            Some(Clip::Copy(text)) => Some(text),
            _ => None,
        }
    }
    #[test]
    fn pane_button_hides_the_rail_and_full_screen_is_queued_for_the_server() {
        let mut t = console("");
        let columns = t.grid().columns;
        assert_eq!(t.target(100, 166), Some(10), "Doorsh row");
        t.toggle_rail();
        assert_eq!(t.layout.rail(), 0);
        assert!(t.grid().columns > columns, "the console takes the width");
        assert_eq!(t.target(100, 166), None, "no rail rows");
        assert_eq!(
            t.target(t.layout.width as i32 - 106, 80),
            Some(36),
            "the button stays"
        );
        t.configure(900, 600);
        assert_eq!(t.layout.rail(), 0, "hidden across resizes");
        t.toggle_rail();
        assert!(t.layout.rail() > 0);
        assert!(!t.fullscreen);
        t.toggle_fullscreen();
        assert_eq!((t.fullscreen, t.pending_fullscreen), (true, Some(true)));
        t.toggle_fullscreen();
        assert_eq!((t.fullscreen, t.pending_fullscreen), (false, Some(false)));
    }
    #[test]
    fn mouse_drag_selects_cells_and_ctrl_c_copies_only_a_selection() {
        let mut t = console("alpha beta  \ngamma delta");
        assert_eq!(t.grid().columns, 88);
        let (x, y) = cell(6, 0);
        assert!(t.pointer_down_at(x, y, 100));
        let (x, y) = cell(2, 1);
        assert!(t.pointer_motion(x, y));
        t.pointer_up();
        assert!(t.clipboard_key(46, true, false), "Ctrl+C over a selection");
        assert_eq!(copied(&t), Some("beta\r\ngam"));
        assert!(t.state.tabs[0].selection.is_none(), "copying clears it");
        assert!(!t.clipboard_key(46, true, false), "else Ctrl+C interrupts");
        // A click alone selects nothing.
        let (x, y) = cell(1, 1);
        t.pointer_down_at(x, y, 1000);
        t.pointer_up();
        assert!(t.state.tabs[0].selection.is_none());
        // Double click: a word; triple click: the line.
        let (x, y) = cell(8, 1);
        t.pointer_down_at(x, y, 2000);
        t.pointer_up();
        t.pointer_down_at(x, y, 2010);
        t.pointer_up();
        assert!(t.clipboard_key(46, true, true), "Ctrl+Shift+C");
        assert_eq!(copied(&t), Some("delta"));
        for now in [3000, 3010, 3020] {
            t.pointer_down_at(x, y, now);
            t.pointer_up();
        }
        assert!(t.clipboard_key(0x152, true, false), "Ctrl+Insert");
        assert_eq!(copied(&t), Some("gamma delta"));
        // Presses outside the console (rail, toolbar) never select.
        assert!(!t.pointer_down_at(100, 200, 4000));
        assert!(!t.pointer_down_at(1010, 80, 4100));
    }
    #[test]
    fn right_click_copies_a_selection_or_pastes_and_paste_keys_queue() {
        let mut t = console("one two");
        let (x, y) = cell(0, 0);
        t.pointer_down_at(x, y, 100);
        let (x, y) = cell(2, 0);
        t.pointer_motion(x, y);
        t.pointer_up();
        assert!(t.context(x, y));
        assert_eq!(copied(&t), Some("one"));
        assert!(t.context(x, y));
        assert!(matches!(t.clipboard.back(), Some(Clip::Paste)));
        for (scan, ctrl, shift) in [(47, true, false), (47, true, true), (0x152, false, true)] {
            t.clipboard.clear();
            assert!(t.clipboard_key(scan, ctrl, shift));
            assert!(matches!(t.clipboard.back(), Some(Clip::Paste)));
        }
        // Bounded: a stuck clipboard never queues without limit.
        for _ in 0..3 * MAX_CLIPBOARD {
            t.clipboard_key(47, true, false);
        }
        assert_eq!(t.clipboard.len(), MAX_CLIPBOARD);
        t.state.page = Page::Settings;
        assert!(!t.context(x, y));
    }
    #[test]
    fn paste_inserts_the_first_line_only_and_never_submits() {
        let mut t = console("");
        t.state.tab().input.set("ls ".into(), false);
        assert!(t.paste_text("/users\x1b[2J\r\nrm -r /users\r\n"));
        assert_eq!(t.state.tabs[0].input.text(), "ls /users");
        assert!(t.message.contains("1 more line was not pasted"));
        assert!(
            t.state.tabs[0]
                .lines
                .iter()
                .all(|l| !l.text().contains("rm"))
        );
        // The find box takes the paste while it has focus.
        t.searching = true;
        t.search_focus = true;
        assert!(t.paste_text("needle\tx"));
        assert_eq!(t.query.text(), "needle x");
        // Profiles that aren't available yet have no input to paste into.
        t.search_focus = false;
        t.state.new_tab(Profile::Dsl);
        assert!(!t.paste_text("dir"));
    }
    #[test]
    fn dragging_past_the_edge_autoscrolls_and_extends_the_selection() {
        let rows: String = (0..200).map(|i| format!("row {i}\n")).collect();
        let mut t = console(&rows);
        let (x, y) = cell(0, 10);
        t.pointer_down_at(x, y, 100);
        t.pointer_motion(x, 40);
        assert!(t.autoscroll(100));
        let scroll = t.state.tabs[0].scroll;
        assert!(scroll >= 2);
        assert!(!t.autoscroll(101), "throttled between steps");
        assert!(t.autoscroll(100 + AUTOSCROLL_TICKS));
        assert!(t.state.tabs[0].scroll > scroll);
        t.pointer_up();
        let text = t.state.tabs[0].selected_text(88).unwrap();
        assert!(text.lines().count() > 10, "{text}");
        assert!(!t.autoscroll(1000), "no drag, no scroll");
    }
    #[test]
    fn resize_reflows_every_tab_and_the_selection_paints_in_the_scheme() {
        let mut t = console(&"0123456789".repeat(20));
        t.state.new_tab(Profile::Native);
        t.state.active = 0;
        t.configure(520, 650);
        let columns = t.columns();
        assert!(t.state.tabs.iter().all(|tab| tab.columns() == columns));
        t.configure(1050, 650);
        let (x, y) = cell(2, 0);
        t.pointer_down_at(x, y, 100);
        let (x, y) = cell(8, 0);
        t.pointer_motion(x, y);
        t.pointer_up();
        let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(1).unwrap();
        let mono = door_graphics_text::FontAtlas::monospace_reference_scaled(1).unwrap();
        let mut pixels = alloc::vec![0; 1050 * 650 * 4];
        let mut v =
            ui::View::new_scaled(&mut pixels, 1050, 650, &fonts, 1).with_monospace(Some(&mono));
        t.draw(&mut v);
        let at = |x: i32, y: i32| {
            let i = ((y as usize) * 1050 + x as usize) * 4;
            [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
        };
        let fill = mix(DARK_CONSOLE, [230, 222, 211, 255], 35);
        // Cell edges hold no glyph ink: the plate under the selected cells,
        // the console tint outside them.
        assert_eq!(at(226 + 2 * 9, 78 + 1), fill);
        assert_eq!(at(226 + 8 * 9 + 8, 78 + 1), fill);
        assert_eq!(
            at(226 + 9 * 9 + 1, 78 + 1),
            t.state.preferences.background()
        );
    }
    /// Windows Terminal reference (docs/design/fidelity-26h2-diff.md): a 64 x 24
    /// "+" / drop-down split button at y 8, Campbell #0C0C0C text area and a
    /// 16 px scrollbar hidden at rest.
    #[test]
    fn split_button_campbell_background_and_hidden_scrollbar_match_the_reference() {
        let mut t = console(&"line\n".repeat(200));
        let (plus, profiles) = t.new_tab_buttons().unwrap();
        let d = profiles.unwrap();
        assert_eq!((plus.y, plus.width, plus.height), (8, 32, 24));
        assert_eq!((d.x - plus.x, d.y, d.width, d.height), (32, 8, 32, 24));
        let (left, w) = t.caption_region();
        assert_eq!((left + w) as i32, d.x + 32);
        assert!(left + w <= 1050 - 138);
        assert_eq!(t.target(plus.x + 16, 20), Some(30));
        assert_eq!(t.target(d.x + 16, 20), Some(20));
        assert_eq!(DARK_CONSOLE, [12, 12, 12, 255]);
        t.state.preferences.opacity = 100;
        let fonts = door_graphics_text::FontAtlas::windows_reference_scaled(1).unwrap();
        let mono = door_graphics_text::FontAtlas::monospace_reference_scaled(1).unwrap();
        let bar = |hover: Option<(i32, i32)>| {
            let mut pixels = alloc::vec![0; 1050 * 650 * 4];
            let mut v =
                ui::View::new_scaled(&mut pixels, 1050, 650, &fonts, 1).with_monospace(Some(&mono));
            v.hover = hover;
            t.draw(&mut v);
            (78..560)
                .filter(|y| {
                    let i = ((*y as usize) * 1050 + 1050 - 8) * 4;
                    pixels[i..i + 4] != t.state.preferences.background()
                })
                .count()
        };
        assert_eq!(bar(None), 0, "scrollbar shown at rest");
        assert!(
            bar(Some((1050 - 8, 300))) > 24,
            "no 16 px scrollbar on hover"
        );
    }
}
