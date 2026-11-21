//! Windows 11 Task Manager composition: Mica title bar with a centered search
//! box, hamburger navigation rail, a page header with commands, dense sortable
//! tables and time-based performance graphs. Paint and hit testing read the
//! same geometry, so every painted control is exactly what answers input.
use super::*;
use door_app_ui::fluent::Rect as R;
use door_app_ui::icons::Icon;
use door_graphics_text::TextStyle;
// door.accessibility tree (shared adapter `door_workspace::semantics`).
#[path = "semantics.rs"]
mod semantics;

// Geometry measured on Windows 11 26H2 Task Manager (Processes, compact
// navigation, 768 epx window; reports/artifacts/reference-26h2/taskmgr-screen-initial.png
// at 150 %, docs/design/fidelity-26h2-diff.md T1-T5), in logical px.
/// Title bar height (T1: 78 px = 52); the window service draws the trusted
/// caption buttons.
pub(crate) const CAPTION: i32 = 52;
/// Icon, title and search box share the band above the caption's lower 2 px
/// (Windows centres them at y 25, not at the caption's middle).
const CAPTION_BAND: u32 = 50;
/// App icon (16 px at x 17), title text (x 56) and search box (260 x 32 at
/// y 8, centred) in the caption.
const CAPTION_ICON: (i32, i32) = (17, 17);
const CAPTION_TITLE_X: i32 = 56;
const SEARCH_WIDTH: i32 = 260;
const SEARCH_Y: i32 = 8;
/// Hamburger (centre y 75) and the first destination (top 97) at a 40 px pitch (T2).
const HAMBURGER: R = R::new(4, 57, 40, 36);
const NAV_TOP: i32 = 97;
const PITCH: i32 = 40;
const ITEM: u32 = 36;
/// Page header band under the caption (T3: 52..101) and its 1 px separator.
const HEADER_Y: i32 = CAPTION;
const HEADER_H: i32 = 49;
/// Column header top (T4: 104) and the two-line resource header (51).
const TABLE_Y: i32 = 104;
const TWO_LINE_HEADER: i32 = 51;
/// Table row pitch (T5: 42 px = 28) and the taller "Apps (n)" group rows
/// (58 px = 39) whose label is 20 px text.
const ROW: i32 = 28;
const GROUP_ROW: i32 = 39;
const WHEEL_STEP: i32 = 48;
/// Trusted caption buttons occupy this many pixels at the top right.
const SYSTEM_BUTTONS: i32 = 138;
/// Accessibility > Text size: a band that holds text grows by the growth of
/// its style's line box (0 at 100%); icons, gaps and the caption keep their
/// size. Paint, hit testing and semantics all read these.
fn grow(style: TextStyle) -> i32 {
    door_workspace::palette::text_grow_i(0, style)
}
/// Table row pitch (28 at 100%).
fn row() -> i32 {
    ROW + grow(TextStyle::Regular14)
}
/// Group header row pitch (39 at 100%): a Regular20 label.
fn group_row() -> i32 {
    GROUP_ROW + grow(TextStyle::Regular20)
}
/// Height of one table row of either kind.
fn row_height(r: &Row) -> i32 {
    if matches!(r, Row::Group(..)) {
        group_row()
    } else {
        row()
    }
}
/// (top relative to the first row, height) of every row, in order.
fn row_extents(rows: &[Row]) -> impl Iterator<Item = (i32, i32)> + '_ {
    rows.iter().scan(0, |top, r| {
        let h = row_height(r);
        let at = *top;
        *top += h;
        Some((at, h))
    })
}
/// Navigation destination pitch and item height (40 and 36 at 100%).
fn pitch() -> i32 {
    PITCH + grow(TextStyle::Regular14)
}
fn item() -> u32 {
    ITEM + grow(TextStyle::Regular14) as u32
}
/// Page header band growth: its Semibold20 title is the tallest text.
fn header_grow() -> i32 {
    grow(TextStyle::Semibold20)
}
/// Page header band height (49 at 100%; its separator sits at the bottom).
fn header_band() -> i32 {
    HEADER_H + header_grow()
}
/// Top of the page body under the header band (104 at 100%).
fn table_y() -> i32 {
    TABLE_Y + header_grow()
}
/// Offset that centres a `size` px glyph in a `height` px band.
fn centre(height: u32, size: u32) -> i32 {
    (height as i32 - size as i32) / 2
}
/// A logical text-holding width at the applied text size.
fn text_w(width: u32) -> u32 {
    door_workspace::palette::text_len(width)
}
/// Performance tile pitch and height (76 and 68 at 100%): a Regular16 name
/// above a Regular12 value.
fn tile_grow() -> i32 {
    grow(TextStyle::Regular16) + grow(TextStyle::Regular12)
}
/// Settings page: (heading, control, check, note) growth.
fn settings_grow() -> (i32, i32) {
    (grow(TextStyle::Semibold14), grow(TextStyle::Regular12))
}

pub(crate) struct Service {
    pub manifest: u64,
    pub name: &'static str,
    pub description: &'static str,
    pub group: &'static str,
}
/// The protected service graph that Init supervises (see service-protocol).
pub(crate) const SERVICE_CATALOG: [Service; 5] = [
    Service {
        manifest: door_abi::platform::manifest::INIT,
        name: "Init",
        description: "Service supervisor and application launcher",
        group: "Core",
    },
    Service {
        manifest: door_abi::platform::manifest::STORAGE,
        name: "Storage",
        description: "File system, registry and settings storage",
        group: "Core",
    },
    Service {
        manifest: door_abi::platform::manifest::SESSION,
        name: "Session",
        description: "Sign-in and session authority",
        group: "Core",
    },
    Service {
        manifest: door_abi::platform::manifest::CONSOLE,
        name: "Console",
        description: "Console input and output",
        group: "Core",
    },
    Service {
        manifest: door_abi::platform::manifest::WINDOW,
        name: "Window",
        description: "Desktop window manager and compositor",
        group: "Desktop",
    },
];

// Target ids. Hover identity is target identity, so every control that
// changes on hover (including disabled commands with a tooltip) has one.
const DISMISS: u32 = 0;
const ID_HAMBURGER: u32 = 1;
const ID_SEARCH: u32 = 2;
const ID_SEARCH_CLEAR: u32 = 3;
const ID_NAV: u32 = 10;
const ID_COMMAND: u32 = 20;
const ID_TILE: u32 = 40;
const ID_COLUMN: u32 = 50;
const ID_ROW: u32 = 100;
const ROW_LIMIT: usize = 300;
const ID_MENU: u32 = 500;
const ID_OK: u32 = 600;
const ID_CANCEL: u32 = 601;
const ID_FIELD: u32 = 602;
const ID_MODAL: u32 = 603;
const ID_START_PAGE: u32 = 700;
const ID_SPEED: u32 = 701;
const ID_ON_TOP: u32 = 702;
const ID_GROUPING: u32 = 703;

const CMD_RUN: u32 = ID_COMMAND;
const CMD_END: u32 = ID_COMMAND + 1;
const CMD_EFFICIENCY: u32 = ID_COMMAND + 2;
const CMD_VIEW: u32 = ID_COMMAND + 3;
const CMD_DELETE_HISTORY: u32 = ID_COMMAND + 4;
const CMD_ENABLE: u32 = ID_COMMAND + 5;
const CMD_DISABLE: u32 = ID_COMMAND + 6;
const CMD_DISCONNECT: u32 = ID_COMMAND + 7;
const CMD_START: u32 = ID_COMMAND + 8;
const CMD_STOP: u32 = ID_COMMAND + 9;
const CMD_RESTART: u32 = ID_COMMAND + 10;

const END_TASK_REASON: &str = "End task is unavailable: ending processes is reserved for the window manager, and Task Manager has no process-control permission.";
const EFFICIENCY_REASON: &str =
    "Efficiency mode needs scheduler priority control, which this system does not provide.";
const STARTUP_REASON: &str = "No startup app registration provider is available.";
const SERVICE_REASON: &str = "Select a service the Service Control Manager runs to start or stop it. Doors' core services can't be stopped.";
const SERVICE_STATE_REASON: &str = "Not available while the service is in this state.";
const HISTORY_REASON: &str =
    "App history is computed from running apps and is not stored, so there is nothing to delete.";
const DISCONNECT_REASON: &str = "Only the local console session exists; it cannot be disconnected.";
const ON_TOP_REASON: &str = "Not supported by the window manager";

#[derive(Clone, Copy)]
struct Theme {
    dark: bool,
    /// The contrast theme's system colours, when one is on. The light roles
    /// below then reach the painters as Window, WindowText and Highlight
    /// through the palette; lit (Highlight) fills carry HighlightText, the
    /// heat map is off and the graphs draw in Highlight on Window, as Task
    /// Manager does under a contrast theme.
    system: Option<door_service_protocol::desktop::SystemColors>,
}
const fn rgb(r: u8, g: u8, b: u8) -> [u8; 4] {
    [b, g, r, 255]
}
/// The system accent's Dark1 step (the light theme's accent fill).
const ACCENT_DARK1: [u8; 4] = rgb(0, 95, 184);
impl Theme {
    fn pick(self, light: [u8; 4], dark: [u8; 4]) -> [u8; 4] {
        if self.dark { dark } else { light }
    }
    /// A hover/pressed/selected fill: Highlight under a contrast theme.
    fn lit(self, color: [u8; 4]) -> [u8; 4] {
        if self.system.is_some() {
            rgb(0, 95, 184)
        } else {
            color
        }
    }
    /// Text on a fill: HighlightText on a lit contrast fill, else `color`.
    fn on(self, lit: bool, color: [u8; 4]) -> [u8; 4] {
        if lit && self.system.is_some() {
            [255; 4]
        } else {
            color
        }
    }
    fn mica(self) -> [u8; 4] {
        self.pick(rgb(232, 239, 246), rgb(30, 33, 40))
    }
    fn layer(self) -> [u8; 4] {
        self.pick(rgb(250, 250, 251), rgb(39, 41, 46))
    }
    fn stroke(self) -> [u8; 4] {
        self.pick(rgb(229, 229, 229), rgb(54, 57, 63))
    }
    fn text(self) -> [u8; 4] {
        self.pick(rgb(26, 26, 26), rgb(255, 255, 255))
    }
    /// Foreground for shapes painted with `rect`/`rounded`: the dark palette
    /// remaps near-white neutral surfaces, so dark ink carries a slight tint.
    fn ink(self) -> [u8; 4] {
        self.pick(rgb(26, 26, 26), rgb(236, 240, 248))
    }
    fn secondary(self) -> [u8; 4] {
        self.pick(rgb(96, 96, 96), rgb(207, 207, 207))
    }
    fn disabled(self) -> [u8; 4] {
        // Tinted in dark so the palette's gray remapping leaves it alone.
        self.pick(rgb(161, 161, 161), rgb(118, 121, 126))
    }
    fn hover(self) -> [u8; 4] {
        self.lit(self.pick(rgb(221, 229, 237), rgb(43, 47, 56)))
    }
    fn pressed(self) -> [u8; 4] {
        self.lit(self.pick(rgb(213, 222, 231), rgb(38, 41, 49)))
    }
    fn nav_selected(self) -> [u8; 4] {
        self.lit(self.pick(rgb(217, 226, 236), rgb(47, 51, 61)))
    }
    fn row_hover(self) -> [u8; 4] {
        self.pick(rgb(240, 241, 243), rgb(48, 50, 56))
    }
    fn row_selected(self) -> [u8; 4] {
        self.lit(self.pick(rgb(204, 228, 247), rgb(37, 61, 87)))
    }
    fn accent(self) -> [u8; 4] {
        self.pick(ACCENT_DARK1, rgb(96, 205, 255))
    }
    fn on_accent(self) -> [u8; 4] {
        self.pick(rgb(255, 255, 255), rgb(12, 10, 8))
    }
    fn control(self) -> [u8; 4] {
        self.pick(rgb(255, 255, 255), rgb(49, 51, 57))
    }
    fn control_hover(self) -> [u8; 4] {
        self.pick(rgb(249, 249, 249), rgb(55, 57, 63))
    }
    fn control_stroke(self) -> [u8; 4] {
        self.pick(rgb(226, 226, 226), rgb(62, 65, 71))
    }
    fn control_bottom(self) -> [u8; 4] {
        self.pick(rgb(138, 138, 138), rgb(150, 152, 156))
    }
    fn card(self) -> [u8; 4] {
        self.pick(rgb(249, 249, 249), rgb(44, 46, 52))
    }
    fn subtle(self) -> [u8; 4] {
        self.pick(rgb(243, 243, 243), rgb(52, 54, 60))
    }
    /// The accent's deep shade the heat map is mixed from: the light
    /// theme's accent (Dark1) itself, and a darker step of it in dark
    /// (Windows mixes AccentDark2 there).
    fn heat_accent(self) -> [u8; 3] {
        let [b, g, r, _] = ACCENT_DARK1;
        let deep = |c: u8| {
            if self.dark {
                (c as u32 * 7 / 10) as u8
            } else {
                c
            }
        };
        [deep(r), deep(g), deep(b)]
    }
    /// Task Manager heat map (T5): the accent ramp over the table layer as
    /// usage rises. Windows 26H2 dark cells measure 30 % (`#311D3A`) to 70 %
    /// (`#512362`) of AccentDark2 over the content; light cells use paler
    /// tints so dark text stays legible.
    fn heat(self, t: u32) -> [u8; 4] {
        let t = t.min(1000);
        let (lo, hi) = if self.dark { (77, 179) } else { (31, 115) };
        let [b, g, r, _] = self.layer();
        let c = mix3([r, g, b], self.heat_accent(), lo + (hi - lo) * t / 1000);
        rgb(c[0], c[1], c[2])
    }
    /// (line, grid, background) for the CPU, Memory and Disk graphs.
    fn graph(self, resource: u32) -> ([u8; 3], [u8; 3], [u8; 3]) {
        if let Some(s) = self.system {
            return (s.highlight, s.gray_text, s.window);
        }
        let (light, dark) = match resource {
            0 => (
                ([17, 125, 187], [212, 231, 244]),
                ([64, 164, 228], [33, 51, 66]),
            ),
            1 => (
                ([139, 18, 174], [237, 223, 243]),
                ([182, 98, 220], [52, 38, 62]),
            ),
            _ => (
                ([77, 166, 12], [222, 238, 209]),
                ([112, 192, 52], [38, 56, 33]),
            ),
        };
        if self.dark {
            (dark.0, dark.1, [28, 30, 35])
        } else {
            (light.0, light.1, [255, 255, 255])
        }
    }
}
fn bgra(c: [u8; 3]) -> [u8; 4] {
    [c[2], c[1], c[0], 255]
}
/// A graph-colour fill: under a contrast theme the colour already is a
/// system colour, so it bypasses the palette's role mapping.
fn solid(v: &mut View<'_>, t: Theme, x: i32, y: i32, w: u32, h: u32, color: [u8; 4]) {
    if t.system.is_some() {
        v.rounded_asset(x, y, w, h, 0, color);
    } else {
        v.rect(x, y, w, h, color);
    }
}
fn mix3(a: [u8; 3], b: [u8; 3], alpha: u32) -> [u8; 3] {
    let m = |x: u8, y: u8| ((x as u32 * (255 - alpha) + y as u32 * alpha + 127) / 255) as u8;
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2])]
}
macro_rules! mask {
    ($scale:expr, $name:literal) => {
        match $scale {
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
macro_rules! color16 {
    ($scale:expr, $path:literal) => {
        match $scale {
            3 => &door_workspace::asset!(concat!("../../../assets/", $path, ".48.bgra"))[..],
            2 => &door_workspace::asset!(concat!("../../../assets/", $path, ".32.bgra"))[..],
            _ => &door_workspace::asset!(concat!("../../../assets/", $path, ".16.bgra"))[..],
        }
    };
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Glyph {
    Processes,
    Performance,
    History,
    Startup,
    Users,
    Details,
    Services,
    Settings,
    Add,
    End,
    Efficiency,
    Play,
    Stop,
    Refresh,
    Delete,
    Disconnect,
    Check,
}
fn glyph(v: &mut View<'_>, x: i32, y: i32, g: Glyph, color: [u8; 4]) {
    let s = v.scale;
    let m: &[u8] = match g {
        Glyph::Processes => mask!(s, "view"),
        Glyph::Performance => mask!(s, "app-performance"),
        Glyph::History => mask!(s, "app-history"),
        Glyph::Startup => mask!(s, "app-power"),
        Glyph::Users => mask!(s, "app-people"),
        Glyph::Details => mask!(s, "app-processes"),
        Glyph::Services => mask!(s, "setting-repair"),
        Glyph::Settings => mask!(s, "settings"),
        Glyph::Add => mask!(s, "app-add"),
        Glyph::End => mask!(s, "app-close"),
        Glyph::Efficiency => mask!(s, "setting-quiet"),
        Glyph::Play => mask!(s, "setting-play"),
        Glyph::Stop => mask!(s, "app-record"),
        Glyph::Refresh => mask!(s, "app-refresh"),
        Glyph::Delete => mask!(s, "explorer-delete"),
        Glyph::Disconnect => mask!(s, "app-power"),
        Glyph::Check => mask!(s, "app-check"),
    };
    v.mask_pixels(x, y, 20, m, color, None);
}
const NAV_GLYPHS: [Glyph; 8] = [
    Glyph::Processes,
    Glyph::Performance,
    Glyph::History,
    Glyph::Startup,
    Glyph::Users,
    Glyph::Details,
    Glyph::Services,
    Glyph::Settings,
];
fn process_icon(v: &mut View<'_>, x: i32, y: i32, icon: model::ProcessIcon) {
    use model::ProcessIcon as P;
    let s = v.scale;
    let pixels: &[u8] = match icon {
        P::Explorer => color16!(s, "icons/windows/explorer"),
        P::Settings => color16!(s, "icons/windows/settings"),
        P::Terminal => color16!(s, "icons/windows/terminal-current"),
        P::Notes => color16!(s, "icons/windows/notepad"),
        P::Preview => color16!(s, "branding/preview"),
        P::TaskManager => color16!(s, "icons/windows/taskmgr"),
        P::Calculator => color16!(s, "icons/windows/calculator"),
        P::Clock => color16!(s, "icons/windows/clock"),
        P::Paint => color16!(s, "icons/windows/paint"),
        P::SnippingTool => color16!(s, "icons/windows/snipping-tool"),
        P::Weather => color16!(s, "icons/windows/weather"),
        P::Magnifier => color16!(s, "icons/windows/magnifier"),
        P::Narrator => color16!(s, "icons/windows/narrator"),
        P::OnScreenKeyboard => color16!(s, "icons/windows/on-screen-keyboard"),
        P::VoiceAccess => color16!(s, "icons/windows/voice-access"),
        P::Service => color16!(s, "icons/windows/terminal-settings"),
        P::Desktop => color16!(s, "icons/windows/desktop"),
        P::Application => color16!(s, "icons/windows/desktop-current"),
        P::Console => color16!(s, "icons/windows/terminal-cmd"),
    };
    v.color_pixels(x, y, 16, pixels);
}
/// Centered title-bar search box, or a narrower box beside the title when the
/// window is small; `None` when there is no room at all. The window service
/// accepts a caption region starting at most 2048 physical pixels from the
/// left, so on very wide windows the box stops short of exact centering.
pub(crate) fn search_box(width: u32, scale: u32) -> Option<R> {
    let w = width as i32;
    let centered = (w - 2 * 176).min(SEARCH_WIDTH);
    if centered >= 240 {
        let x = ((w - centered) / 2).min(2048 / scale.clamp(1, 3) as i32);
        return Some(R::new(x, SEARCH_Y, centered as u32, 32));
    }
    let available = w - SYSTEM_BUTTONS - 8 - 152;
    (available >= 120).then(|| R::new(152, SEARCH_Y, available as u32, 32))
}
/// Client-owned part of the caption for the window service: exactly the
/// search box. The rest of the title bar stays draggable.
pub(crate) fn caption_region(width: u32, scale: u32) -> (u32, u32) {
    match search_box(width, scale) {
        Some(r) => (r.x as u32, r.w),
        None => (width.saturating_sub(SYSTEM_BUTTONS as u32 + 1), 1),
    }
}
#[derive(Clone, Copy)]
struct Command {
    id: u32,
    label: &'static str,
    glyph: Option<Glyph>,
    enabled: bool,
    reason: &'static str,
}
#[derive(Clone, Copy)]
pub(crate) struct Column {
    pub(crate) key: Key,
    label: &'static str,
    pub(crate) x: i32,
    w: u32,
    right: bool,
}
#[derive(Clone, Copy)]
enum MenuEntry {
    Item(&'static str, Option<&'static str>),
    Heading(&'static str),
    Separator,
}
impl Monitor {
    pub(super) fn measure(&mut self, fonts: &FontAtlas) {
        self.widths.clear();
        let mut add = |s: &'static str, style: TextStyle| {
            let w = fonts.measure(s, style).map_or(0, |m| m.advance_width);
            self.widths.push((s, style == TextStyle::Semibold20, w));
        };
        for label in [
            "Run new task",
            "End task",
            "Efficiency mode",
            "View",
            "Delete usage history",
            "Enable",
            "Disable",
            "Disconnect",
            "Start",
            "Stop",
            "Restart",
        ] {
            add(label, TextStyle::Regular14);
        }
        for title in PAGE_NAMES {
            add(title, TextStyle::Semibold20);
            // Navigation labels: the open pane widens when larger text needs it.
            add(title, TextStyle::Regular14);
        }
    }
    /// Measured (or, before fonts are known, conservatively estimated) width.
    fn text_width(&self, s: &str, title: bool) -> u32 {
        self.widths
            .iter()
            .find(|(k, t, _)| *k == s && *t == title)
            .map_or(s.len() as u32 * if title { 11 } else { 8 }, |(_, _, w)| *w)
    }
    pub(super) fn search(&self) -> Option<R> {
        search_box(self.width, self.scale.get())
    }
    fn nav_open(&self) -> bool {
        self.nav_expanded.unwrap_or(self.width >= 1000)
    }
    pub(super) fn rail(&self) -> u32 {
        if !self.nav_open() {
            return 48;
        }
        // Labels start 44 px into the 8 px inset row and keep 8 px clear.
        let widest = PAGE_NAMES
            .iter()
            .map(|n| self.text_width(n, false))
            .max()
            .unwrap_or(0);
        220.max(widest + 60)
    }
    fn nav_bottom(&self) -> i32 {
        self.height as i32 - 12 - item() as i32
    }
    fn nav_max_scroll(&self) -> u32 {
        (7 * pitch() - (pitch() - item() as i32) - (self.nav_bottom() - NAV_TOP)).max(0) as u32
    }
    pub(super) fn clamp_nav(&mut self) {
        self.nav_scroll = self.nav_scroll.min(self.nav_max_scroll());
    }
    fn nav_item(&self, i: usize) -> R {
        if i == 7 {
            return R::new(
                4,
                self.height as i32 - 8 - item() as i32,
                self.rail() - 8,
                item(),
            );
        }
        R::new(
            4,
            NAV_TOP + i as i32 * pitch() - self.nav_scroll as i32,
            self.rail() - 8,
            item(),
        )
    }
    /// The part of a scrolling destination row that is inside the pane.
    fn nav_visible(&self, i: usize) -> Option<R> {
        let r = self.nav_item(i);
        if i == 7 {
            return (r.y >= NAV_TOP).then_some(r);
        }
        let top = r.y.max(NAV_TOP);
        let bottom = (r.y + r.h as i32).min(self.nav_bottom());
        (bottom > top).then(|| R::new(r.x, top, r.w, (bottom - top) as u32))
    }
    fn content_x(&self) -> i32 {
        self.rail() as i32
    }
    fn commands(&self) -> Vec<Command> {
        let run = Command {
            id: CMD_RUN,
            label: "Run new task",
            glyph: Some(Glyph::Add),
            enabled: true,
            reason: "",
        };
        let off = |id, label, glyph, reason| Command {
            id,
            label,
            glyph: Some(glyph),
            enabled: false,
            reason,
        };
        let mut list = alloc::vec![run];
        match self.page {
            PROCESSES => {
                list.push(off(CMD_END, "End task", Glyph::End, END_TASK_REASON));
                list.push(off(
                    CMD_EFFICIENCY,
                    "Efficiency mode",
                    Glyph::Efficiency,
                    EFFICIENCY_REASON,
                ));
                list.push(Command {
                    id: CMD_VIEW,
                    label: "View",
                    glyph: None,
                    enabled: true,
                    reason: "",
                });
            }
            APP_HISTORY => list.push(off(
                CMD_DELETE_HISTORY,
                "Delete usage history",
                Glyph::Delete,
                HISTORY_REASON,
            )),
            STARTUP => {
                list.push(off(CMD_ENABLE, "Enable", Glyph::Play, STARTUP_REASON));
                list.push(off(CMD_DISABLE, "Disable", Glyph::Stop, STARTUP_REASON));
            }
            USERS => list.push(off(
                CMD_DISCONNECT,
                "Disconnect",
                Glyph::Disconnect,
                DISCONNECT_REASON,
            )),
            DETAILS => list.push(off(CMD_END, "End task", Glyph::End, END_TASK_REASON)),
            SERVICES => {
                // Doors' own services (DoorsStorage...) are never stopped.
                let scm = self.selected_scm().filter(|s| {
                    !door_service_protocol::scm::BUILTIN
                        .iter()
                        .any(|b| b.0.eq_ignore_ascii_case(&s.name))
                });
                let (start, stop) = match scm.map(|s| s.state) {
                    Some(door_service_protocol::scm::state::STOPPED) => (true, false),
                    Some(
                        door_service_protocol::scm::state::RUNNING
                        | door_service_protocol::scm::state::PAUSED,
                    ) => (false, true),
                    _ => (false, false),
                };
                let reason = if scm.is_some() {
                    SERVICE_STATE_REASON
                } else {
                    SERVICE_REASON
                };
                let command = |id, label, glyph, enabled| Command {
                    id,
                    label,
                    glyph: Some(glyph),
                    enabled,
                    reason,
                };
                list.push(command(CMD_START, "Start", Glyph::Play, start));
                list.push(command(CMD_STOP, "Stop", Glyph::Stop, stop));
                list.push(command(CMD_RESTART, "Restart", Glyph::Refresh, stop));
            }
            PERFORMANCE => (),
            _ => return Vec::new(),
        }
        list
    }
    /// Start, Stop or Restart the selected Service Control Manager service.
    fn control_service(&mut self, id: u32) {
        let Some(name) = self.selected_scm().map(|s| s.name.clone()) else {
            return;
        };
        let Ok(mut scm) = door_service_client::Services::connect(self.registry) else {
            self.service_notice = crate::service_failure("change", &name, 1063);
            return;
        };
        let (verb, result) = match id {
            CMD_START => ("start", scm.start(&name, 0).map(|_| ())),
            CMD_STOP => ("stop", scm.control(&name, 1).map(|_| ())),
            _ => {
                let result = scm.control(&name, 1).map(|_| ());
                if result.is_ok() {
                    self.restart = Some(name.clone());
                }
                ("restart", result)
            }
        };
        door_rt::diagnostic(format_args!(
            "DOOR_MONITOR SERVICE {verb} name={name} result={}\n",
            result.err().unwrap_or(0)
        ));
        self.service_notice = match result {
            Ok(()) => String::new(),
            Err(code) => crate::service_failure(verb, &name, code),
        };
        self.refresh_requested = true;
    }
    /// Header commands right-aligned after the page title; labels collapse to
    /// icons when the row is too narrow, and commands that still do not fit
    /// are omitted rather than overlapping the title.
    fn command_rects(&self) -> Vec<(Command, R, bool)> {
        let commands = self.commands();
        let title_end = self.content_x()
            + 24
            + self.text_width(PAGE_NAMES[self.page as usize], true) as i32
            + 24;
        let right = self.width as i32 - 16;
        let full = |c: &Command| -> u32 {
            if c.glyph.is_none() {
                12 + self.text_width(c.label, false) + 8 + 12 + 12
            } else {
                12 + 20 + 8 + self.text_width(c.label, false) + 12
            }
        };
        let total: i32 = commands.iter().map(|c| full(c) as i32 + 4).sum();
        let compact = right - total < title_end;
        let mut x = right;
        let mut out = Vec::new();
        for c in commands.iter().rev() {
            let w = if compact && c.glyph.is_some() {
                40
            } else {
                full(c)
            };
            x -= w as i32;
            if x < title_end {
                break;
            }
            let g = grow(TextStyle::Regular14);
            // Centred in the header band (60..92 at 100%).
            let y = HEADER_Y + (header_band() - 32 - g).max(0) / 2;
            out.push((
                *c,
                R::new(x, y, w, (32 + g) as u32),
                compact && c.glyph.is_some(),
            ));
            x -= 4;
        }
        out.reverse();
        out
    }
    fn table(&self) -> R {
        let x = self.content_x() + 8;
        // App history states its measurement scope above the table.
        let top = if self.page == APP_HISTORY {
            table_y() + 30 + grow(TextStyle::Regular12)
        } else {
            table_y()
        };
        R::new(
            x,
            top,
            (self.width as i32 - x - 12).max(0) as u32,
            (self.height as i32 - top - 4).max(0) as u32,
        )
    }
    fn header_height(&self) -> u32 {
        let label = grow(TextStyle::Regular12);
        if matches!(self.page, PROCESSES | USERS) {
            (TWO_LINE_HEADER + grow(TextStyle::Regular16) + label) as u32
        } else {
            (36 + label) as u32
        }
    }
    pub(super) fn list_viewport(&self) -> R {
        let t = self.table();
        let top = t.y + self.header_height() as i32;
        R::new(t.x, top, t.w, (t.y + t.h as i32 - top).max(0) as u32)
    }
    /// Height of every row together (group rows are taller).
    fn content_height(&self) -> u32 {
        row_extents(&self.rows()).map(|(_, h)| h as u32).sum()
    }
    pub(super) fn max_scroll(&self) -> u32 {
        self.content_height().saturating_sub(self.list_viewport().h)
    }
    pub(super) fn reveal_row(&mut self, index: usize) {
        let view = self.list_viewport();
        let Some((top, pitch)) = row_extents(&self.rows()).nth(index) else {
            return;
        };
        let (top, pitch) = (top as u32, pitch as u32);
        if top < self.scroll {
            self.scroll = top;
        } else if top + pitch > self.scroll + view.h {
            self.scroll = (top + pitch).saturating_sub(view.h);
        }
        self.clamp_scroll();
    }
    pub(super) fn columns(&self) -> Vec<Column> {
        let t = self.table();
        let right = t.x + t.w as i32;
        // (key, label, width or 0 for the flexible column, right aligned).
        // NPU columns (Windows 26H2) are not offered: this device has no NPU.
        let spec: &[(Key, &str, u32, bool)] = match self.page {
            PROCESSES if self.show_isolation => &[
                (Key::Name, "Name", 0, false),
                (Key::Status, "Status", 128, false),
                (Key::Isolation, "Isolation", 150, false),
                (Key::Cpu, "CPU", 76, true),
                (Key::Memory, "Memory", 104, true),
            ],
            PROCESSES => &[
                (Key::Name, "Name", 0, false),
                (Key::Status, "Status", 128, false),
                (Key::Cpu, "CPU", 76, true),
                (Key::Memory, "Memory", 104, true),
            ],
            USERS => &[
                (Key::Name, "User", 0, false),
                (Key::Status, "Status", 128, false),
                (Key::Cpu, "CPU", 76, true),
                (Key::Memory, "Memory", 104, true),
            ],
            APP_HISTORY => &[
                (Key::Name, "Name", 0, false),
                (Key::CpuTime, "CPU time", 120, true),
            ],
            DETAILS => &[
                (Key::Name, "Name", 190, false),
                (Key::Pid, "PID", 72, true),
                (Key::Status, "Status", 96, false),
                (Key::User, "User name", 124, false),
                (Key::Cpu, "CPU", 64, true),
                (Key::Memory, "Memory (private)", 136, true),
                (Key::Threads, "Threads", 76, true),
                (Key::Isolation, "Isolation", 150, false),
                (Key::Description, "Description", 0, false),
            ],
            SERVICES => &[
                (Key::Name, "Name", 150, false),
                (Key::Pid, "PID", 72, true),
                (Key::Description, "Description", 0, false),
                (Key::Status, "Status", 96, false),
                (Key::User, "Group", 120, false),
            ],
            STARTUP => &[
                (Key::Name, "Name", 0, false),
                (Key::Description, "Publisher", 180, false),
                (Key::Status, "Status", 110, false),
                (Key::User, "Startup impact", 140, false),
            ],
            _ => &[],
        };
        // Narrow tables drop trailing optional columns before squeezing names.
        // Fixed columns hold text, so they widen with the text size.
        let mut chosen: Vec<_> = spec
            .iter()
            .map(|&(k, l, w, r)| (k, l, text_w(w), r))
            .collect();
        loop {
            let fixed: u32 = chosen.iter().map(|c| c.2).sum();
            let flex_min = chosen.iter().filter(|c| c.2 == 0).count() as u32 * text_w(120);
            if fixed + flex_min <= t.w || chosen.len() <= 2 {
                break;
            }
            let drop = chosen
                .iter()
                .rposition(|c| c.0 != Key::Name && c.2 != 0)
                .unwrap_or(chosen.len() - 1);
            chosen.remove(drop);
        }
        let fixed: u32 = chosen.iter().map(|c| c.2).sum();
        let flexible = chosen.iter().filter(|c| c.2 == 0).count().max(1) as u32;
        let flex = t.w.saturating_sub(fixed) / flexible;
        let mut x = t.x;
        let mut out = Vec::new();
        for (key, label, w, right_aligned) in chosen {
            let width = if w == 0 { flex } else { w };
            out.push(Column {
                key,
                label,
                x,
                w: width.min((right - x).max(0) as u32),
                right: right_aligned,
            });
            x += width as i32;
        }
        out
    }
    fn tiles_width(&self) -> u32 {
        if self.width.saturating_sub(self.rail()) >= 820 {
            244
        } else {
            168
        }
    }
    fn tile(&self, i: u32) -> R {
        R::new(
            self.content_x() + 8,
            table_y() + i as i32 * (76 + tile_grow()),
            self.tiles_width(),
            (68 + tile_grow()) as u32,
        )
    }
    pub(super) fn settings_controls(&self) -> [(u32, R); 4] {
        let x = self.content_x() + 24;
        let [_, start, _, speed, _, on_top, _, grouping, _] = self.settings_rows();
        let (g, _) = settings_grow();
        let combo = (32 + g) as u32;
        let check = (28 + g) as u32;
        let check_w = 300 + text_w(270) - 270;
        [
            (ID_START_PAGE, R::new(x, start, 260, combo)),
            (ID_SPEED, R::new(x, speed, 260, combo)),
            (ID_ON_TOP, R::new(x, on_top, check_w, check)),
            (ID_GROUPING, R::new(x, grouping, check_w, check)),
        ]
    }
    /// Settings page row tops: heading, start-page combo, heading, speed
    /// combo, heading, on-top check, heading, grouping check, footer note.
    fn settings_rows(&self) -> [i32; 9] {
        let (g, small) = settings_grow();
        let t = table_y();
        [
            t,
            t + 30 + g,
            t + 84 + 2 * g,
            t + 114 + 3 * g,
            t + 168 + 4 * g,
            t + 198 + 5 * g,
            t + 260 + 6 * g + small,
            t + 290 + 7 * g + small,
            t + 340 + 8 * g + small,
        ]
    }
    fn menu_entries(&self) -> Vec<MenuEntry> {
        match self.menu {
            Some(Menu::View) => alloc::vec![
                MenuEntry::Item("Refresh now", Some("F5")),
                MenuEntry::Separator,
                MenuEntry::Heading("Real time update speed"),
                MenuEntry::Item("High", None),
                MenuEntry::Item("Normal", None),
                MenuEntry::Item("Low", None),
                MenuEntry::Item("Paused", None),
                MenuEntry::Separator,
                MenuEntry::Item("Group by type", None),
                MenuEntry::Item("Isolation column", None),
            ],
            Some(Menu::StartPage) => PAGE_NAMES[..7]
                .iter()
                .map(|n| MenuEntry::Item(n, None))
                .collect(),
            Some(Menu::Speed) => Speed::ALL
                .iter()
                .map(|s| MenuEntry::Item(s.label(), None))
                .collect(),
            None => Vec::new(),
        }
    }
    /// Menu card and its entries' rectangles (entries 1:1 with `menu_entries`).
    pub(super) fn menu_layout(&self) -> Option<(R, Vec<R>)> {
        let anchor = match self.menu? {
            Menu::View => self
                .command_rects()
                .iter()
                .find(|(c, _, _)| c.id == CMD_VIEW)
                .map(|(_, r, _)| *r)?,
            Menu::StartPage => self.settings_controls()[0].1,
            Menu::Speed => self.settings_controls()[1].1,
        };
        let entries = self.menu_entries();
        let width = if self.menu == Some(Menu::View) {
            (240 + text_w(196) - 196).min(self.width)
        } else {
            anchor.w
        };
        let mut y = 4;
        let mut rects = Vec::new();
        for e in &entries {
            let h = match e {
                MenuEntry::Separator => 9,
                MenuEntry::Heading(_) => (28 + grow(TextStyle::Regular12)) as u32,
                MenuEntry::Item(..) => (36 + grow(TextStyle::Regular14)) as u32,
            };
            rects.push(R::new(4, y, width - 8, h));
            y += h as i32;
        }
        let height = y as u32 + 4;
        let mut x = if self.menu == Some(Menu::View) {
            anchor.x + anchor.w as i32 - width as i32
        } else {
            anchor.x
        };
        x = x.clamp(0, (self.width as i32 - width as i32).max(0));
        let mut top = anchor.y + anchor.h as i32 + 4;
        if top + height as i32 > self.height as i32 {
            top = (anchor.y - 4 - height as i32).max(CAPTION);
        }
        for r in &mut rects {
            r.x += x;
            r.y += top;
        }
        Some((R::new(x, top, width, height), rects))
    }
    fn dialog_rect(&self) -> R {
        let w = self.width.saturating_sub(32).min(448 + text_w(400) - 400);
        let h = self
            .height
            .saturating_sub(64)
            .min(252 + Self::dialog_grow() as u32);
        R::new(
            (self.width as i32 - w as i32) / 2,
            CAPTION + (self.height as i32 - CAPTION - h as i32) / 2,
            w,
            h,
        )
    }
    /// Run dialog growth: title, description, field, two note lines, buttons.
    fn dialog_grow() -> i32 {
        grow(TextStyle::Semibold20)
            + 3 * grow(TextStyle::Regular14)
            + 2 * grow(TextStyle::Regular12)
    }
    pub(super) fn dialog_controls(&self) -> [R; 3] {
        let d = self.dialog_rect();
        let g = grow(TextStyle::Regular14);
        let control = (32 + g) as u32;
        let button = (d.w.saturating_sub(48 + 8)) / 2;
        let by = d.y + d.h as i32 - 56 - g;
        let field_x = 24 + text_w(56) as i32;
        let field_y = d.y + 108 + grow(TextStyle::Semibold20) + g;
        [
            R::new(
                d.x + field_x,
                field_y,
                d.w.saturating_sub(field_x as u32 + 24),
                control,
            ),
            R::new(d.x + 24, by, button, control),
            R::new(d.x + 24 + button as i32 + 8, by, button, control),
        ]
    }
    pub(super) fn text_field_at(&self, x: i32, y: i32) -> bool {
        if self.dialog.is_some() {
            return self.dialog_controls()[0].hit(x, y);
        }
        self.menu.is_none() && self.search().is_some_and(|r| r.hit(x, y))
    }
    pub(super) fn hits(&self) -> Vec<(u32, R)> {
        let mut a = Vec::new();
        if self.dialog.is_some() {
            a.push((ID_MODAL, R::new(0, 0, self.width, self.height)));
            let [field, ok, cancel] = self.dialog_controls();
            a.push((ID_FIELD, field));
            a.push((ID_OK, ok));
            a.push((ID_CANCEL, cancel));
            return Self::clip(a, self.width, self.height, self.search());
        }
        if let Some(r) = self.search() {
            a.push((ID_SEARCH, r));
            if !self.search.text().is_empty() {
                a.push((
                    ID_SEARCH_CLEAR,
                    R::new(r.x + r.w as i32 - 34, r.y + 4, 28, 24),
                ));
            }
        }
        a.push((ID_HAMBURGER, HAMBURGER));
        for i in 0..8 {
            if let Some(r) = self.nav_visible(i) {
                a.push((ID_NAV + i as u32, r));
            }
        }
        for (c, r, _) in self.command_rects() {
            a.push((c.id, r));
        }
        match self.page {
            PERFORMANCE => {
                for i in 0..3 {
                    a.push((ID_TILE + i, self.tile(i)));
                }
            }
            SETTINGS => {
                for (id, r) in self.settings_controls() {
                    a.push((id, r));
                }
            }
            STARTUP => (),
            _ => {
                let t = self.table();
                for (i, c) in self.columns().iter().enumerate() {
                    a.push((
                        ID_COLUMN + i as u32,
                        R::new(c.x, t.y, c.w, self.header_height()),
                    ));
                }
                let view = self.list_viewport();
                let rows = self.rows();
                for (i, (offset, pitch)) in row_extents(&rows).enumerate().take(ROW_LIMIT) {
                    let y = view.y + offset - self.scroll as i32;
                    let top = y.max(view.y);
                    let bottom = (y + pitch).min(view.y + view.h as i32);
                    if bottom > top {
                        a.push((
                            ID_ROW + i as u32,
                            R::new(view.x, top, view.w, (bottom - top) as u32),
                        ));
                    }
                }
            }
        }
        if let Some((card, entries)) = self.menu_layout() {
            a.push((DISMISS, R::new(0, 0, self.width, self.height)));
            a.push((ID_MODAL, card));
            for (i, (e, r)) in self.menu_entries().iter().zip(entries).enumerate() {
                if matches!(e, MenuEntry::Item(..)) {
                    a.push((ID_MENU + i as u32, r));
                }
            }
        }
        Self::clip(a, self.width, self.height, self.search())
    }
    fn clip(mut a: Vec<(u32, R)>, width: u32, height: u32, region: Option<R>) -> Vec<(u32, R)> {
        // The caption outside the search box belongs to the window service.

        a.retain(|(id, r)| {
            r.w > 0
                && r.h > 0
                && r.x >= 0
                && r.y >= 0
                && r.x + r.w as i32 <= width as i32
                && r.y + r.h as i32 <= height as i32
                && (r.y >= CAPTION
                    || matches!(*id, ID_SEARCH | ID_SEARCH_CLEAR)
                    || matches!(*id, DISMISS | ID_MODAL)
                    || region.is_some_and(|s| r.x >= s.x && r.x + r.w as i32 <= s.x + s.w as i32))
        });
        a
    }
    /// Applies a target id; returns whether anything visible changed.
    pub(super) fn activate(&mut self, id: u32) -> bool {
        if self.dialog.is_some() {
            match id {
                ID_OK => self.submit_run(),
                ID_CANCEL => self.dialog = None,
                _ => return false,
            }
            return true;
        }
        if let Some(menu) = self.menu {
            if (ID_MENU..ID_MENU + 16).contains(&id) {
                self.menu_choice(menu, (id - ID_MENU) as usize);
            }
            self.menu = None;
            return true;
        }
        if id != ID_SEARCH && id != ID_SEARCH_CLEAR {
            self.searching = false;
        }
        match id {
            ID_HAMBURGER => {
                self.nav_expanded = Some(!self.nav_open());
                self.clamp_scroll();
            }
            ID_SEARCH => self.searching = true,
            ID_SEARCH_CLEAR => {
                self.search.set(String::new(), false);
                self.searching = true;
                self.clamp_scroll();
            }
            n if (ID_NAV..ID_NAV + 8).contains(&n) => self.navigate(n - ID_NAV),
            CMD_RUN => self.open_run_dialog(),
            CMD_VIEW => self.menu = Some(Menu::View),
            CMD_START | CMD_STOP | CMD_RESTART if self.page == SERVICES => {
                if !self.commands().iter().any(|c| c.id == id && c.enabled) {
                    return false;
                }
                self.control_service(id);
            }
            n if (ID_COMMAND..ID_COMMAND + 16).contains(&n) => return false,
            n if (ID_TILE..ID_TILE + 3).contains(&n) => self.select_resource(n - ID_TILE),
            n if (ID_COLUMN..ID_COLUMN + 16).contains(&n) => {
                if let Some(c) = self.columns().get((n - ID_COLUMN) as usize) {
                    let page = self.page as usize;
                    self.sorts[page] = self.sorts[page].toggled(c.key);
                }
            }
            n if (ID_ROW..ID_ROW + ROW_LIMIT as u32).contains(&n) => {
                let rows = self.rows();
                match rows.get((n - ID_ROW) as usize) {
                    Some(Row::User(..)) => {
                        self.users_expanded = !self.users_expanded;
                        self.selected = rows[(n - ID_ROW) as usize].key();
                        self.clamp_scroll();
                    }
                    Some(row) => {
                        if let Some(k) = row.key() {
                            self.selected = Some(k);
                        }
                    }
                    None => return false,
                }
            }
            ID_START_PAGE => self.menu = Some(Menu::StartPage),
            ID_SPEED => self.menu = Some(Menu::Speed),
            ID_GROUPING => self.group_by_type = !self.group_by_type,
            _ => return false,
        }
        true
    }
    fn menu_choice(&mut self, menu: Menu, index: usize) {
        match menu {
            Menu::View => match index {
                0 => {
                    self.refresh_requested = true;
                    self.next = 0;
                }
                3..=6 => self.set_speed(Speed::ALL[index - 3]),
                8 => self.group_by_type = !self.group_by_type,
                9 => self.show_isolation = !self.show_isolation,
                _ => (),
            },
            Menu::StartPage => self.default_page = (index as u32).min(6),
            Menu::Speed => {
                if let Some(s) = Speed::ALL.get(index) {
                    self.set_speed(*s)
                }
            }
        }
        self.clamp_scroll();
    }
    pub(super) fn scroll_at(&mut self, x: i32, y: i32, notches: i32) -> bool {
        if self.dialog.is_some() || self.menu.is_some() || notches == 0 {
            return false;
        }
        let delta = -notches * WHEEL_STEP;
        if x < self.rail() as i32 && y >= NAV_TOP && y < self.nav_bottom() {
            let before = self.nav_scroll;
            self.nav_scroll = (self.nav_scroll as i32 + delta).max(0) as u32;
            self.clamp_nav();
            return before != self.nav_scroll;
        }
        if self.table().hit(x, y) {
            let before = self.scroll;
            self.scroll = (self.scroll as i32 + delta).max(0) as u32;
            self.clamp_scroll();
            return before != self.scroll;
        }
        false
    }
    fn hovered(&self, v: &View<'_>, r: R) -> bool {
        v.hover.is_some_and(|(x, y)| r.hit(x, y))
            && self.dialog.is_none()
            && (self.menu.is_none()
                || self
                    .menu_layout()
                    .is_some_and(|(card, _)| card.hit(r.x, r.y)))
    }
    fn pressed(&self, v: &View<'_>, r: R) -> bool {
        self.hovered(v, r) && v.pressed.is_some_and(|(x, y)| r.hit(x, y))
    }
    pub(super) fn paint(&self, v: &mut View<'_>) {
        let t = Theme {
            dark: v.palette.dark,
            system: v.palette.system(),
        };
        let rail = self.rail();
        // Mica title bar and navigation pane, then the content layer with its
        // rounded top-left corner and hairline edge.
        // One Mica surface under everything, so the layer's rounded corner
        // reveals the title bar material rather than unpainted pixels.
        if t.system.is_some() {
            // Contrast themes are opaque: a translucent Mica tint would map to
            // WindowText instead of the Window background.
            v.rect(0, 0, v.width, v.height, t.mica());
        } else {
            v.material_rect(0, 0, v.width, v.height, t.mica(), 220);
        }
        let cx = rail as i32;
        v.rounded(
            cx,
            CAPTION,
            v.width + 8 - rail,
            v.height + 8 - CAPTION as u32,
            8,
            t.stroke(),
        );
        v.rounded(
            cx + 1,
            CAPTION + 1,
            v.width + 8 - rail,
            v.height + 8 - CAPTION as u32,
            7,
            t.layer(),
        );
        self.paint_caption(v, t);
        self.paint_nav(v, t);
        // Page header band (T3) closed by a 1 px separator across the layer.
        v.rect(
            cx + 1,
            HEADER_Y + header_band(),
            v.width.saturating_sub(rail + 1),
            1,
            t.stroke(),
        );
        v.text_clip_centered(
            cx + 24,
            (HEADER_Y, header_band() as u32),
            self.text_width(PAGE_NAMES[self.page as usize], true) + 8,
            PAGE_NAMES[self.page as usize],
            TextStyle::Semibold20,
            t.text(),
        );
        let commands = self.command_rects();
        for (c, r, compact) in &commands {
            self.paint_command(v, t, c, *r, *compact);
        }
        match self.page {
            PERFORMANCE => self.paint_performance(v, t),
            SETTINGS => self.paint_settings(v, t),
            STARTUP => self.paint_startup(v, t),
            _ => self.paint_table(v, t),
        }
        let notice = if self.error.is_empty() && self.page == SERVICES {
            &self.service_notice
        } else {
            &self.error
        };
        if !notice.is_empty() {
            let g = grow(TextStyle::Regular12);
            let r = R::new(
                cx + 24,
                v.height as i32 - 28 - g,
                v.width.saturating_sub(rail + 48),
                (24 + g) as u32,
            );
            v.text_clip_centered(
                r.x,
                (r.y, r.h),
                r.w,
                notice,
                TextStyle::Regular12,
                t.secondary(),
            );
        }
        // Tooltip for a hovered command, including why a disabled one is off.
        if self.menu.is_none() && self.dialog.is_none() {
            for (c, r, compact) in &commands {
                if self.hovered(v, *r) && (!c.enabled || *compact) {
                    let text = if c.enabled { c.label } else { c.reason };
                    self.paint_tooltip(v, t, *r, text);
                }
            }
        }
        if self.menu.is_some() {
            self.paint_menu(v, t);
        }
        if self.dialog.is_some() {
            self.paint_dialog(v, t);
        }
    }
    fn paint_caption(&self, v: &mut View<'_>, t: Theme) {
        let s = v.scale;
        let (x, y) = CAPTION_ICON;
        v.color_pixels(x, y, 16, color16!(s, "icons/windows/taskmgr"));
        // Clipped before the narrow-window search box at x 152.
        v.text_clip_centered(
            CAPTION_TITLE_X,
            (0, CAPTION_BAND),
            92,
            "Task Manager",
            TextStyle::Regular12,
            t.text(),
        );
        let Some(r) = self.search() else {
            return;
        };
        let active = self.searching && self.dialog.is_none();
        let fill = if active {
            t.control()
        } else if self.hovered(v, r) {
            t.control_hover()
        } else {
            t.pick(rgb(246, 248, 251), rgb(44, 47, 54))
        };
        v.rounded(r.x, r.y, r.w, r.h, 5, t.control_stroke());
        v.rounded(r.x + 1, r.y + 1, r.w - 2, r.h - 2, 4, fill);
        if active {
            v.rect(r.x + 4, r.y + r.h as i32 - 2, r.w - 8, 2, t.accent());
        }
        v.compact_icon(r.x + 12, r.y + 8, Icon::Search, t.secondary());
        let text_x = r.x + 40;
        let text_w =
            r.w.saturating_sub(40 + if self.search.text().is_empty() { 8 } else { 40 });
        if self.search.text().is_empty() && !active {
            v.text_clip_centered(
                text_x,
                (r.y, r.h),
                text_w,
                "Type a name, publisher, or PID to search",
                TextStyle::Regular14,
                t.secondary(),
            );
        } else {
            v.line_editor(text_x, r.y + 6, text_w, &self.search, active && self.caret);
        }
        if !self.search.text().is_empty() {
            let c = R::new(r.x + r.w as i32 - 34, r.y + 4, 28, 24);
            if self.hovered(v, c) {
                v.rounded(c.x, c.y, c.w, c.h, 4, t.subtle());
            }
            v.tiny_icon(c.x + 8, c.y + 6, Icon::Close, t.secondary());
        }
    }
    fn paint_nav(&self, v: &mut View<'_>, t: Theme) {
        let h = HAMBURGER;
        let lit = self.hovered(v, h);
        if lit {
            v.rounded(
                h.x,
                h.y,
                h.w,
                h.h,
                5,
                if self.pressed(v, h) {
                    t.pressed()
                } else {
                    t.hover()
                },
            );
        }
        for i in 0..3 {
            match t.system {
                Some(s) if lit => {
                    v.rounded_asset(h.x + 12, h.y + 12 + i * 5, 16, 1, 0, bgra(s.highlight_text))
                }
                _ => v.rect(h.x + 12, h.y + 12 + i * 5, 16, 1, t.ink()),
            }
        }
        let open = self.nav_open();
        for i in 0..8 {
            let Some(visible) = self.nav_visible(i) else {
                continue;
            };
            let r = self.nav_item(i);
            if i < 7 {
                clip(
                    v,
                    R::new(
                        0,
                        NAV_TOP,
                        self.rail(),
                        (self.nav_bottom() - NAV_TOP).max(1) as u32,
                    ),
                );
            }
            let selected = self.page == i as u32;
            let fill = if self.pressed(v, visible) {
                Some(t.pressed())
            } else if self.hovered(v, visible) {
                Some(t.hover())
            } else if selected {
                Some(t.nav_selected())
            } else {
                None
            };
            let lit = fill.is_some();
            if let Some(fill) = fill {
                v.rounded(r.x, r.y, r.w, r.h, 5, fill);
            }
            if selected && t.system.is_none() {
                v.rounded(r.x, r.y + centre(r.h, 16), 3, 16, 2, t.accent());
            }
            let ink = t.on(lit, t.text());
            glyph(v, r.x + 10, r.y + centre(r.h, 20), NAV_GLYPHS[i], ink);
            if open {
                v.text_clip_centered(
                    r.x + 44,
                    (r.y, r.h),
                    r.w.saturating_sub(52),
                    PAGE_NAMES[i],
                    TextStyle::Regular14,
                    ink,
                );
            }
            v.reset_viewport();
        }
    }
    fn paint_command(&self, v: &mut View<'_>, t: Theme, c: &Command, r: R, compact: bool) {
        let open = c.id == CMD_VIEW && self.menu == Some(Menu::View);
        let pressed = c.enabled && (self.hovered(v, r) || open) && self.pressed(v, r);
        if c.enabled && (self.hovered(v, r) || open) {
            v.rounded(
                r.x,
                r.y,
                r.w,
                r.h,
                4,
                if pressed { t.pressed() } else { t.row_hover() },
            );
        }
        let ink = t.on(pressed, if c.enabled { t.text() } else { t.disabled() });
        match c.glyph {
            Some(g) => {
                glyph(
                    v,
                    r.x + if compact { 10 } else { 12 },
                    r.y + centre(r.h, 20),
                    g,
                    ink,
                );
                if !compact {
                    v.text_clip_centered(
                        r.x + 40,
                        (r.y, r.h),
                        r.w.saturating_sub(48),
                        c.label,
                        TextStyle::Regular14,
                        ink,
                    );
                }
            }
            None => {
                v.text_clip_centered(
                    r.x + 12,
                    (r.y, r.h),
                    r.w.saturating_sub(36),
                    c.label,
                    TextStyle::Regular14,
                    ink,
                );
                v.tiny_icon(
                    r.x + r.w as i32 - 24,
                    r.y + centre(r.h, 12),
                    Icon::Chevron,
                    ink,
                );
            }
        }
    }
    fn paint_tooltip(&self, v: &mut View<'_>, t: Theme, anchor: R, text: &str) {
        let width = (v.measure(text) + 20).min(text_w(320));
        let lines = text_lines(v, text, width - 20);
        let pitch = (20 + grow(TextStyle::Regular12)) as u32;
        let height = lines.len() as u32 * pitch + 12;
        let x = (anchor.x + anchor.w as i32 / 2 - width as i32 / 2)
            .clamp(4, (v.width as i32 - width as i32 - 4).max(4));
        let y = anchor.y + anchor.h as i32 + 6;
        v.elevation(x, y, width, height, 24);
        v.rounded(x, y, width, height, 4, t.control_stroke());
        v.rounded(x + 1, y + 1, width - 2, height - 2, 3, t.card());
        for (i, line) in lines.iter().enumerate() {
            v.text_clip(
                x + 10,
                y + 6 + i as i32 * pitch as i32,
                width - 20,
                line,
                TextStyle::Regular12,
                t.text(),
            );
        }
    }
    fn paint_table(&self, v: &mut View<'_>, t: Theme) {
        let table = self.table();
        let columns = self.columns();
        let header = self.header_height();
        let sort = self.sorts[self.page as usize];
        let snapshot = self.current;
        let cpu_total = self.history.back().and_then(|s| s.cpu);
        let memory_total = self.history.back().map(|s| s.memory);
        if self.page == APP_HISTORY {
            v.text_clip_centered(
                table.x + 10,
                (table_y(), (24 + grow(TextStyle::Regular12)) as u32),
                table.w.saturating_sub(20),
                "Resource usage since each app started, for the current user account.",
                TextStyle::Regular12,
                t.secondary(),
            );
        }
        for (i, c) in columns.iter().enumerate() {
            let r = R::new(c.x, table.y, c.w, header);
            if self.hovered(v, r) {
                v.rect(r.x, r.y, r.w, r.h, t.row_hover());
            }
            if i > 0 {
                v.rect(c.x, table.y, 1, header, t.stroke());
            }
            let label_w = v.measure_small(c.label);
            let label_x = if c.right {
                c.x + c.w as i32 - 10 - label_w as i32
            } else {
                c.x + 10
            };
            let small = grow(TextStyle::Regular12);
            let bottom = (table.y + header as i32 - 24 - small, (20 + small) as u32);
            if header > 40 {
                // Two-line resource header: current total above the column name.
                let total = match c.key {
                    Key::Cpu => cpu_total.map(|p| format!("{}%", (p + 5) / 10)),
                    Key::Memory => memory_total.map(|p| format!("{}%", (p + 5) / 10)),
                    _ => None,
                };
                if let Some(total) = total {
                    let w = v.measure_style(&total, TextStyle::Regular16);
                    v.text_clip_centered(
                        c.x + c.w as i32 - 10 - w as i32,
                        (table.y + 4, (24 + grow(TextStyle::Regular16)) as u32),
                        w + 2,
                        &total,
                        TextStyle::Regular16,
                        t.text(),
                    );
                }
            }
            v.text_clip_centered(
                label_x,
                bottom,
                c.w.saturating_sub(20),
                c.label,
                TextStyle::Regular12,
                t.secondary(),
            );
            if c.key == sort.key {
                let x = c.x + c.w as i32 / 2 - 6;
                if sort.descending {
                    v.tiny_icon(x, table.y + 1, Icon::Chevron, t.secondary());
                } else {
                    v.tiny_icon_flipped(x, table.y + 1, Icon::Chevron, t.secondary());
                }
            }
        }
        v.rect(table.x, table.y + header as i32 - 1, table.w, 1, t.stroke());
        let view = self.list_viewport();
        let rows = self.rows();
        if !clip(v, view) {
            return;
        }
        let total_pages = snapshot.map_or(1, |s| s.managed_pages.max(1));
        for (row, (offset, pitch)) in rows.iter().zip(row_extents(&rows)).take(ROW_LIMIT) {
            let y = view.y + offset - self.scroll as i32;
            if y + pitch <= view.y || y >= view.y + view.h as i32 {
                continue;
            }
            let rr = R::new(view.x, y, view.w, pitch as u32);
            let visible = R::new(
                view.x,
                y.max(view.y),
                view.w,
                ((y + pitch).min(view.y + view.h as i32) - y.max(view.y)).max(0) as u32,
            );
            let selected = row.key().is_some() && row.key() == self.selected;
            // Selected rows carry HighlightText under a contrast theme.
            let text = t.on(selected, t.text());
            let secondary = t.on(selected, t.secondary());
            if selected {
                v.rect(rr.x, rr.y, rr.w, rr.h, t.row_selected());
            } else if self.hovered(v, visible) && row.key().is_some() {
                v.rect(rr.x, rr.y, rr.w, rr.h, t.row_hover());
            }
            match row {
                Row::Group(group, count) => {
                    v.text_clip_centered(
                        rr.x + 10,
                        (rr.y, rr.h),
                        rr.w.saturating_sub(20),
                        &format!("{} ({count})", group.label()),
                        TextStyle::Regular20,
                        text,
                    );
                }
                Row::User(uid, count) => {
                    let name_col = columns[0];
                    let chevron_x = name_col.x + 10;
                    let chevron_y = rr.y + centre(rr.h, 12);
                    if self.users_expanded {
                        v.tiny_icon(chevron_x, chevron_y, Icon::Chevron, secondary);
                    } else {
                        v.tiny_icon(chevron_x, chevron_y, Icon::ChevronRight, secondary);
                    }
                    glyph(
                        v,
                        name_col.x + 30,
                        rr.y + centre(rr.h, 20),
                        Glyph::Users,
                        text,
                    );
                    v.text_clip_centered(
                        name_col.x + 58,
                        (rr.y, rr.h),
                        name_col.w.saturating_sub(64),
                        &format!("{} ({count})", model::user_name(*uid)),
                        TextStyle::Regular14,
                        text,
                    );
                    let members: Vec<Process> = self
                        .processes()
                        .into_iter()
                        .filter(|p| p.uid == *uid)
                        .collect();
                    let cpu: Option<u32> = members.iter().map(|p| self.cpu(p)).sum();
                    let memory: u64 = members.iter().map(|p| p.private_bytes).sum();
                    for c in columns.iter().skip(1) {
                        let value = match c.key {
                            Key::Status => "Active".into(),
                            Key::Cpu => cpu.map_or("--".into(), model::percent),
                            Key::Memory => model::megabytes(memory),
                            _ => String::new(),
                        };
                        if !selected
                            && t.system.is_none()
                            && let Some(heat) = self.heat_for(c.key, cpu, memory, total_pages)
                        {
                            v.rect(c.x + 1, rr.y, c.w.saturating_sub(1), rr.h, t.heat(heat));
                        }
                        cell(v, t, c, rr, &value, text);
                    }
                }
                Row::Process(p) => {
                    let id = model::identity(p);
                    let indent = if self.page == USERS { 30 } else { 0 };
                    let name = columns[0];
                    process_icon(v, name.x + 12 + indent, rr.y + centre(rr.h, 16), id.icon);
                    let label = if self.page == DETAILS {
                        id.image
                    } else {
                        id.name
                    };
                    v.text_clip_centered(
                        name.x + 40 + indent,
                        (rr.y, rr.h),
                        name.w.saturating_sub(48 + indent as u32),
                        label,
                        TextStyle::Regular14,
                        text,
                    );
                    let cpu = self.cpu(p);
                    for c in columns.iter().skip(1) {
                        let value = match c.key {
                            Key::Status if self.page == DETAILS => model::details_status(p).into(),
                            Key::Status => model::status(p).into(),
                            Key::Cpu => cpu.map_or("--".into(), |n| {
                                if self.page == DETAILS {
                                    format!("{:02}", (n + 5) / 10)
                                } else {
                                    model::percent(n)
                                }
                            }),
                            Key::Memory if self.page == DETAILS => {
                                model::kilobytes(p.private_bytes)
                            }
                            Key::Memory => model::megabytes(p.private_bytes),
                            Key::Pid => p.pid.to_string(),
                            Key::User => model::user_name(p.uid).into(),
                            Key::Threads => p.threads.to_string(),
                            Key::Description => id.name.into(),
                            Key::CpuTime => model::duration(p.samples, false),
                            Key::Isolation => model::isolation(p).into(),
                            Key::Name => String::new(),
                        };
                        if !selected
                            && t.system.is_none()
                            && matches!(self.page, PROCESSES | USERS)
                            && let Some(heat) =
                                self.heat_for(c.key, cpu, p.private_bytes, total_pages)
                        {
                            v.rect(c.x + 1, rr.y, c.w.saturating_sub(1), rr.h, t.heat(heat));
                        }
                        cell(v, t, c, rr, &value, text);
                    }
                }
                Row::Service(_) | Row::Scm(_) => {
                    let Some(s) = self.service_fields(row) else {
                        continue;
                    };
                    process_icon(
                        v,
                        columns[0].x + 12,
                        rr.y + centre(rr.h, 16),
                        model::ProcessIcon::Service,
                    );
                    v.text_clip_centered(
                        columns[0].x + 40,
                        (rr.y, rr.h),
                        columns[0].w.saturating_sub(48),
                        &s.name,
                        TextStyle::Regular14,
                        text,
                    );
                    for c in columns.iter().skip(1) {
                        let value = match c.key {
                            Key::Pid => s.pid.map_or(String::new(), |p| p.to_string()),
                            Key::Description => s.description.clone(),
                            Key::Status => s.status.into(),
                            Key::User => s.group.into(),
                            _ => String::new(),
                        };
                        cell(v, t, c, rr, &value, text);
                    }
                }
            }
        }
        v.reset_viewport();
        if rows.is_empty() {
            let message = if self.current.is_none() {
                "Gathering information..."
            } else if self.search.text().is_empty() {
                "There are no items to display"
            } else {
                "No results found"
            };
            v.text_clip_centered(
                view.x + 24,
                (view.y + 24, (24 + grow(TextStyle::Regular14)) as u32),
                view.w.saturating_sub(48),
                message,
                TextStyle::Regular14,
                t.secondary(),
            );
        }
        // Overlay scroll indicator.
        let content = self.content_height();
        if content > view.h && view.h > 24 {
            let thumb = (view.h * view.h / content).max(24);
            let travel = view.h - thumb;
            let top = view.y
                + (travel as u64 * self.scroll as u64 / self.max_scroll().max(1) as u64) as i32;
            v.rounded(
                table.x + table.w as i32 + 4,
                top,
                3,
                thumb,
                2,
                t.control_bottom(),
            );
        }
    }
    fn heat_for(&self, key: Key, cpu: Option<u32>, bytes: u64, total_pages: u64) -> Option<u32> {
        match key {
            // Full intensity at 30% of the machine, as busy processes stand out.
            Key::Cpu => Some(cpu.unwrap_or(0) * 1000 / 300),
            Key::Memory => {
                Some(((bytes / 4096) as u128 * 8000 / total_pages.max(1) as u128).min(1000) as u32)
            }
            _ => None,
        }
    }
    fn paint_startup(&self, v: &mut View<'_>, t: Theme) {
        let table = self.table();
        let header = self.header_height();
        for (i, c) in self.columns().iter().enumerate() {
            if i > 0 {
                v.rect(c.x, table.y, 1, header, t.stroke());
            }
            let small = grow(TextStyle::Regular12);
            v.text_clip_centered(
                c.x + 10,
                (table.y + header as i32 - 24 - small, (20 + small) as u32),
                c.w.saturating_sub(20),
                c.label,
                TextStyle::Regular12,
                t.secondary(),
            );
        }
        v.rect(table.x, table.y + header as i32 - 1, table.w, 1, t.stroke());
        let mid = table.y + header as i32 + (table.h.saturating_sub(header) as i32) / 3;
        let title = "There are no startup items to display";
        let title_h = 24 + grow(TextStyle::Regular14);
        v.text_in_box(
            (table.x, mid, table.w, title_h as u32),
            title,
            TextStyle::Regular14,
            t.text(),
        );
        let detail = STARTUP_REASON;
        let detail_h = (20 + grow(TextStyle::Regular12)) as u32;
        v.text_in_box(
            (table.x, mid + 2 + title_h, table.w, detail_h),
            detail,
            TextStyle::Regular12,
            t.secondary(),
        );
    }
    fn paint_settings(&self, v: &mut View<'_>, t: Theme) {
        let [start, speed, on_top, grouping] = self.settings_controls();
        let x = start.1.x;
        let rows = self.settings_rows();
        let (g, small) = settings_grow();
        for (y, heading) in [
            (rows[0], "Default start page"),
            (rows[2], "Real time update speed"),
            (rows[4], "Window management"),
            (rows[6], "Other options"),
        ] {
            v.text_clip_centered(
                x,
                (y, (24 + g) as u32),
                text_w(400),
                heading,
                TextStyle::Semibold14,
                t.text(),
            );
        }
        self.paint_combo(
            v,
            t,
            start.1,
            PAGE_NAMES[self.default_page as usize],
            self.menu == Some(Menu::StartPage),
        );
        self.paint_combo(
            v,
            t,
            speed.1,
            self.speed.label(),
            self.menu == Some(Menu::Speed),
        );
        self.paint_check(v, t, on_top.1, "Always on top", false, false);
        v.text_clip_centered(
            on_top.1.x + 30,
            (on_top.1.y + on_top.1.h as i32, (20 + small) as u32),
            text_w(400),
            ON_TOP_REASON,
            TextStyle::Regular12,
            t.secondary(),
        );
        self.paint_check(
            v,
            t,
            grouping.1,
            "Group processes by type",
            self.group_by_type,
            true,
        );
        v.text_clip_centered(
            x,
            (rows[8], (20 + small) as u32),
            v.width.saturating_sub(x as u32 + 24),
            "Preferences apply to Task Manager windows opened in this session.",
            TextStyle::Regular12,
            t.secondary(),
        );
    }
    fn paint_combo(&self, v: &mut View<'_>, t: Theme, r: R, value: &str, open: bool) {
        let fill = if self.pressed(v, r) || open {
            t.subtle()
        } else if self.hovered(v, r) {
            t.control_hover()
        } else {
            t.control()
        };
        v.rounded(r.x, r.y, r.w, r.h, 5, t.control_stroke());
        v.rounded(r.x + 1, r.y + 1, r.w - 2, r.h - 2, 4, fill);
        v.rect(
            r.x + 4,
            r.y + r.h as i32 - 1,
            r.w - 8,
            1,
            t.control_bottom(),
        );
        v.text_clip_centered(
            r.x + 12,
            (r.y, r.h),
            r.w.saturating_sub(44),
            value,
            TextStyle::Regular14,
            t.text(),
        );
        v.tiny_icon(
            r.x + r.w as i32 - 26,
            r.y + centre(r.h, 12),
            Icon::Chevron,
            t.secondary(),
        );
    }
    fn paint_check(
        &self,
        v: &mut View<'_>,
        t: Theme,
        r: R,
        label: &str,
        checked: bool,
        enabled: bool,
    ) {
        let b = R::new(r.x, r.y + centre(r.h, 20), 20, 20);
        if checked {
            v.rounded(b.x, b.y, b.w, b.h, 4, t.accent());
            glyph(v, b.x, b.y, Glyph::Check, t.on_accent());
        } else {
            v.rounded(
                b.x,
                b.y,
                b.w,
                b.h,
                4,
                if enabled {
                    t.control_bottom()
                } else {
                    t.disabled()
                },
            );
            v.rounded(
                b.x + 1,
                b.y + 1,
                18,
                18,
                3,
                if enabled && self.hovered(v, r) {
                    t.control_hover()
                } else {
                    t.layer()
                },
            );
        }
        v.text_clip_centered(
            r.x + 30,
            (r.y, r.h),
            r.w.saturating_sub(30),
            label,
            TextStyle::Regular14,
            if enabled { t.text() } else { t.disabled() },
        );
    }
    fn paint_menu(&self, v: &mut View<'_>, t: Theme) {
        let Some((card, rects)) = self.menu_layout() else {
            return;
        };
        v.elevation(card.x, card.y, card.w, card.h, 40);
        v.rounded(card.x, card.y, card.w, card.h, 8, t.control_stroke());
        v.rounded(card.x + 1, card.y + 1, card.w - 2, card.h - 2, 7, t.card());
        let entries = self.menu_entries();
        for (i, (e, r)) in entries.iter().zip(rects).enumerate() {
            match e {
                MenuEntry::Separator => v.rect(r.x - 4, r.y + 4, r.w + 8, 1, t.stroke()),
                MenuEntry::Heading(label) => {
                    v.text_clip_centered(
                        r.x + 12,
                        (r.y, r.h),
                        r.w.saturating_sub(24),
                        label,
                        TextStyle::Regular12,
                        t.secondary(),
                    );
                }
                MenuEntry::Item(label, accelerator) => {
                    let pressed = self.hovered(v, r) && self.pressed(v, r);
                    if self.hovered(v, r) {
                        v.rounded(
                            r.x,
                            r.y + 2,
                            r.w,
                            r.h - 4,
                            4,
                            if pressed { t.pressed() } else { t.row_hover() },
                        );
                    }
                    let (text, secondary) = (t.on(pressed, t.text()), t.on(pressed, t.secondary()));
                    let marked = match self.menu {
                        Some(Menu::View) => match i {
                            3..=6 => Some(Speed::ALL[i - 3] == self.speed),
                            8 => Some(self.group_by_type),
                            9 => Some(self.show_isolation),
                            _ => None,
                        },
                        Some(Menu::StartPage) => Some(i as u32 == self.default_page),
                        Some(Menu::Speed) => Some(Speed::ALL[i] == self.speed),
                        None => None,
                    };
                    if marked == Some(true) {
                        if self.menu == Some(Menu::View) && i >= 8 {
                            glyph(v, r.x + 8, r.y + centre(r.h, 20), Glyph::Check, text);
                        } else {
                            v.rounded(r.x + 15, r.y + r.h as i32 / 2 - 3, 6, 6, 3, t.ink());
                        }
                    }
                    v.text_clip_centered(
                        r.x + 36,
                        (r.y, r.h),
                        r.w.saturating_sub(44),
                        label,
                        TextStyle::Regular14,
                        text,
                    );
                    if let Some(key) = accelerator {
                        let w = v.measure_small(key);
                        v.text_clip_centered(
                            r.x + r.w as i32 - 12 - w as i32,
                            (r.y, r.h),
                            w + 2,
                            key,
                            TextStyle::Regular12,
                            secondary,
                        );
                    }
                }
            }
        }
    }
    fn paint_dialog(&self, v: &mut View<'_>, t: Theme) {
        let Some(dialog) = self.dialog.as_ref() else {
            return;
        };
        let d = self.dialog_rect();
        let [field, ok, cancel] = self.dialog_controls();
        v.elevation(d.x, d.y, d.w, d.h, 64);
        v.rounded(d.x, d.y, d.w, d.h, 8, t.control_stroke());
        v.rounded(d.x + 1, d.y + 1, d.w - 2, d.h - 2, 7, t.layer());
        let g = grow(TextStyle::Regular14);
        let title_g = grow(TextStyle::Semibold20);
        let footer = d.y + d.h as i32 - 80 - g;
        v.rect(d.x + 1, footer, d.w - 2, (72 + g) as u32, t.card());
        v.rounded(d.x + 1, footer + 64 + g, d.w - 2, 15, 7, t.card());
        v.rect(d.x + 1, footer, d.w - 2, 1, t.stroke());
        v.text_clip_centered(
            d.x + 24,
            (d.y + 18, (32 + title_g) as u32),
            d.w.saturating_sub(48),
            "Create new task",
            TextStyle::Semibold20,
            t.text(),
        );
        v.text_clip_centered(
            d.x + 24,
            (d.y + 58 + title_g, (40 + g) as u32),
            d.w.saturating_sub(48),
            "Type the name of an app, and Doors will open it for you.",
            TextStyle::Regular14,
            t.text(),
        );
        v.text_clip_centered(
            d.x + 24,
            (field.y, field.h),
            (field.x - d.x - 24).max(0) as u32,
            "Open:",
            TextStyle::Regular14,
            t.text(),
        );
        v.rounded(field.x, field.y, field.w, field.h, 5, t.control_stroke());
        v.rounded(
            field.x + 1,
            field.y + 1,
            field.w - 2,
            field.h - 2,
            4,
            t.control(),
        );
        v.rect(
            field.x + 4,
            field.y + field.h as i32 - 2,
            field.w - 8,
            2,
            t.accent(),
        );
        v.line_editor(
            field.x + 10,
            field.y + centre(field.h, 20),
            field.w.saturating_sub(20),
            &dialog.input,
            !dialog.launching && self.caret,
        );
        let note: &str = if !dialog.error.is_empty() {
            &dialog.error
        } else if dialog.launching {
            "Starting..."
        } else {
            "Apps: explorer, notepad, settings, terminal, preview, taskmgr"
        };
        let color = if dialog.error.is_empty() {
            t.secondary()
        } else {
            t.pick(rgb(196, 43, 28), rgb(255, 153, 164))
        };
        for (i, line) in text_lines(v, note, d.w.saturating_sub(48))
            .iter()
            .take(2)
            .enumerate()
        {
            let pitch = 18 + grow(TextStyle::Regular12);
            v.text_clip(
                d.x + 24,
                field.y + field.h as i32 + 6 + i as i32 * pitch,
                d.w.saturating_sub(48),
                line,
                TextStyle::Regular12,
                color,
            );
        }
        for (r, label, accent) in [(ok, "OK", true), (cancel, "Cancel", false)] {
            let enabled = !(accent && dialog.launching);
            if accent && !enabled && t.system.is_some() {
                // Disabled under a contrast theme: GrayText on Window.
                v.rounded(r.x, r.y, r.w, r.h, 5, t.control_stroke());
                v.rounded(r.x + 1, r.y + 1, r.w - 2, r.h - 2, 4, t.control());
            } else if accent {
                let fill = if !enabled {
                    t.disabled()
                } else if self.pressed_dialog(v, r) {
                    t.pick(rgb(0, 84, 163), rgb(76, 180, 230))
                } else if v.hover.is_some_and(|(x, y)| r.hit(x, y)) {
                    t.pick(rgb(0, 103, 192), rgb(84, 193, 243))
                } else {
                    t.accent()
                };
                v.rounded(r.x, r.y, r.w, r.h, 5, fill);
            } else {
                v.rounded(r.x, r.y, r.w, r.h, 5, t.control_stroke());
                let fill = if v.hover.is_some_and(|(x, y)| r.hit(x, y)) {
                    t.control_hover()
                } else {
                    t.control()
                };
                v.rounded(r.x + 1, r.y + 1, r.w - 2, r.h - 2, 4, fill);
            }
            let ink = if accent && !enabled && t.system.is_some() {
                t.disabled()
            } else if accent {
                t.on_accent()
            } else {
                t.text()
            };
            v.text_in_box((r.x, r.y, r.w, r.h), label, TextStyle::Regular14, ink);
        }
    }
    fn pressed_dialog(&self, v: &View<'_>, r: R) -> bool {
        v.hover.is_some_and(|(x, y)| r.hit(x, y)) && v.pressed.is_some_and(|(x, y)| r.hit(x, y))
    }
    fn series(&self, resource: u32) -> Vec<(u64, Option<u32>)> {
        self.history
            .iter()
            .map(|s| {
                (
                    s.ticks,
                    match resource {
                        0 => s.cpu,
                        1 => Some(s.memory),
                        _ => s.io,
                    },
                )
            })
            .collect()
    }
    fn io_scale(&self) -> u32 {
        let peak = self.history.iter().filter_map(|s| s.io).max().unwrap_or(0);
        // Round the axis up to 1-2-5 steps so the label stays readable.
        let mut step = 10;
        loop {
            for m in [1, 2, 5] {
                if peak <= step * m {
                    return step * m;
                }
            }
            step *= 10;
        }
    }
    fn plot(&self, v: &mut View<'_>, t: Theme, r: R, resource: u32, grid: bool, border: bool) {
        if r.w < 4 || r.h < 4 {
            return;
        }
        let (line, grid_color, background) = t.graph(resource);
        let points = self.series(resource);
        let now = self.history.back().map_or(0, |s| s.ticks);
        let max = match resource {
            0 | 1 => 1000,
            _ => self.io_scale(),
        };
        let (inner_x, inner_y, inner_w, inner_h) = if border {
            solid(v, t, r.x, r.y, r.w, r.h, bgra(line));
            (r.x + 1, r.y + 1, r.w - 2, r.h - 2)
        } else {
            (r.x, r.y, r.w, r.h)
        };
        // Rendered at the physical size of its cell (snapped grid lines at
        // fractional scales too).
        let (pw, ph) = (
            v.physical_span(inner_x, inner_w),
            v.physical_span(inner_y, inner_h),
        );
        let image = graph::render_physical(
            pw as usize,
            ph as usize,
            v.zoom.percent() as f32 / 100.0,
            &graph::Plot {
                points: &points,
                max,
                now,
                span: 6000,
            },
            &graph::Style {
                background,
                grid: grid.then_some(grid_color),
                line,
                fill: if grid { 36 } else { 48 },
            },
        );
        v.color_image_physical(inner_x, inner_y, pw, ph, &image);
    }
    fn resource_summary(&self, resource: u32) -> (&'static str, String) {
        let latest = self.history.back();
        match resource {
            0 => (
                "CPU",
                latest
                    .and_then(|s| s.cpu)
                    .map_or("--".into(), |p| format!("{}%", (p + 5) / 10)),
            ),
            1 => (
                "Memory",
                self.current.map_or("--".into(), |s| {
                    format!(
                        "{}/{} ({}%)",
                        model::gigabytes(s.allocated_pages * 4096).trim_end_matches(" GB"),
                        model::gigabytes(s.managed_pages * 4096),
                        (s.allocated_pages * 100 + s.managed_pages / 2) / s.managed_pages.max(1)
                    )
                }),
            ),
            _ => (
                "Disk 0",
                latest
                    .and_then(|s| s.io)
                    .map_or("--".into(), |n| format!("{n} requests/s")),
            ),
        }
    }
    fn paint_performance(&self, v: &mut View<'_>, t: Theme) {
        let thumbnails = self.tiles_width() > 200;
        for i in 0..3 {
            let r = self.tile(i);
            let selected = self.resource == i;
            let lit = self.pressed(v, r) || selected;
            let fill = if self.pressed(v, r) {
                Some(t.pressed())
            } else if selected {
                Some(t.row_selected())
            } else if self.hovered(v, r) {
                Some(t.row_hover())
            } else {
                None
            };
            if let Some(fill) = fill {
                v.rounded(r.x, r.y, r.w, r.h, 6, fill);
            }
            let text_x = if thumbnails {
                self.plot(
                    v,
                    t,
                    R::new(r.x + 10, r.y + centre(r.h, 44), 72, 44),
                    i,
                    false,
                    true,
                );
                r.x + 96
            } else {
                r.x + 14
            };
            let (name, value) = self.resource_summary(i);
            let text_w = (r.x + r.w as i32 - text_x - 8).max(0) as u32;
            let name_g = grow(TextStyle::Regular16);
            v.text_clip_centered(
                text_x,
                (r.y + 10, (24 + name_g) as u32),
                text_w,
                name,
                TextStyle::Regular16,
                t.on(lit, t.text()),
            );
            v.text_clip_centered(
                text_x,
                (r.y + 36 + name_g, (20 + grow(TextStyle::Regular12)) as u32),
                text_w,
                &value,
                TextStyle::Regular12,
                t.on(lit, t.secondary()),
            );
        }
        let note_y = self.tile(2).y + self.tile(2).h as i32 + 16;
        v.text_clip(
            self.tile(0).x + 8,
            note_y,
            self.tiles_width().saturating_sub(16),
            "GPU and network adapters",
            TextStyle::Regular12,
            t.secondary(),
        );
        v.text_clip(
            self.tile(0).x + 8,
            note_y + 18 + grow(TextStyle::Regular12),
            self.tiles_width().saturating_sub(16),
            "are not reported by this device.",
            TextStyle::Regular12,
            t.secondary(),
        );
        self.paint_resource(v, t);
    }
    fn paint_resource(&self, v: &mut View<'_>, t: Theme) {
        let x = self.tile(0).x + self.tiles_width() as i32 + 24;
        let right = v.width as i32 - 24;
        let w = (right - x).max(0) as u32;
        if w < 160 {
            return;
        }
        let r = self.resource;
        let s = self.current;
        let (title, subtitle, axis_label, axis_max) = match r {
            0 => (
                "CPU",
                s.map_or(String::new(), |s| {
                    format!(
                        "{} logical processor{}",
                        s.cpus,
                        if s.cpus == 1 { "" } else { "s" }
                    )
                }),
                "% Utilization over 60 seconds",
                "100%".into(),
            ),
            1 => (
                "Memory",
                s.map_or(String::new(), |s| model::gigabytes(s.managed_pages * 4096)),
                "Memory usage",
                s.map_or(String::new(), |s| model::gigabytes(s.managed_pages * 4096)),
            ),
            _ => (
                "Disk 0",
                "Storage device requests".into(),
                "Requests per second",
                format!("{}/s", self.io_scale()),
            ),
        };
        let top = table_y();
        let (g12, g16, g20, g28) = (
            grow(TextStyle::Regular12),
            grow(TextStyle::Regular16),
            grow(TextStyle::Regular20),
            grow(TextStyle::Semibold28),
        );
        let small = (20 + g12) as u32;
        v.text_clip_centered(
            x,
            (top - 4, (40 + g28) as u32),
            w / 2,
            title,
            TextStyle::Semibold28,
            t.text(),
        );
        let sw = v.measure_style(&subtitle, TextStyle::Regular16).min(w / 2);
        v.text_clip_centered(
            right - sw as i32,
            (top + 4, (28 + g16) as u32),
            sw + 2,
            &subtitle,
            TextStyle::Regular16,
            t.text(),
        );
        let label_y = top + 44 + g28;
        v.text_clip_centered(
            x,
            (label_y, small),
            w * 2 / 3,
            axis_label,
            TextStyle::Regular12,
            t.secondary(),
        );
        let mw = v.measure_small(&axis_max);
        v.text_clip_centered(
            right - mw as i32,
            (label_y, small),
            mw + 2,
            &axis_max,
            TextStyle::Regular12,
            t.secondary(),
        );
        let stats_rows = if r == 1 { 3 } else { 2 };
        let stat_pitch = 56 + g12 + g20;
        let reserved = 56 + g12 + stats_rows * stat_pitch + if r == 1 { 64 + g12 } else { 0 };
        let graph_top = label_y + 22 + g12;
        let graph_h = (v.height as i32 - graph_top - 24 - reserved).clamp(80, 420) as u32;
        let g = R::new(x, graph_top, w, graph_h);
        self.plot(v, t, g, r, true, true);
        let below = g.y + g.h as i32 + 2;
        v.text_clip_centered(
            x,
            (below, small),
            text_w(120),
            "60 seconds",
            TextStyle::Regular12,
            t.secondary(),
        );
        let zero = text_w(10);
        v.text_clip_centered(
            right - zero as i32 + 2,
            (below, small),
            zero,
            "0",
            TextStyle::Regular12,
            t.secondary(),
        );
        let mut y = below + 30 + g12;
        if r == 1 {
            v.text_clip_centered(
                x,
                (y, small),
                w,
                "Memory composition",
                TextStyle::Regular12,
                t.secondary(),
            );
            let bar = R::new(x, y + 22 + g12, w, 28);
            let (line, _, background) = t.graph(1);
            solid(v, t, bar.x, bar.y, bar.w, bar.h, bgra(line));
            solid(
                v,
                t,
                bar.x + 1,
                bar.y + 1,
                bar.w - 2,
                bar.h - 2,
                bgra(background),
            );
            if let Some(s) = s {
                let used = ((bar.w - 2) as u64 * s.allocated_pages / s.managed_pages.max(1)) as u32;
                solid(
                    v,
                    t,
                    bar.x + 1,
                    bar.y + 1,
                    used,
                    bar.h - 2,
                    bgra(mix3(background, line, 72)),
                );
                solid(
                    v,
                    t,
                    bar.x + 1 + used as i32,
                    bar.y + 1,
                    1,
                    bar.h - 2,
                    bgra(line),
                );
            }
            y += 64 + g12;
        }
        let stats: Vec<(&str, String)> = match (r, s) {
            (_, None) => alloc::vec![("Status", "Gathering information...".into())],
            (0, Some(s)) => alloc::vec![
                ("Utilization", self.resource_summary(0).1),
                ("Processes", s.count.to_string()),
                (
                    "Threads",
                    s.processes[..s.count]
                        .iter()
                        .map(|p| p.threads as u64)
                        .sum::<u64>()
                        .to_string()
                ),
                ("Up time", model::duration(s.ticks, true)),
            ],
            (1, Some(s)) => alloc::vec![
                ("In use", model::memory_short(s.allocated_pages * 4096)),
                (
                    "Available",
                    model::memory_short((s.managed_pages - s.allocated_pages) * 4096)
                ),
                (
                    "Processes (private)",
                    model::memory_short(
                        s.processes[..s.count].iter().map(|p| p.private_bytes).sum()
                    )
                ),
            ],
            (_, Some(s)) => alloc::vec![
                (
                    "Requests",
                    self.history
                        .back()
                        .and_then(|h| h.io)
                        .map_or("--".into(), |n| format!("{n}/s"))
                ),
                ("Reads", s.reads.to_string()),
                ("Writes", s.writes.to_string()),
                ("Flushes", s.flushes.to_string()),
            ],
        };
        let block = if w >= 520 { w / 2 } else { w };
        let cell_w = block / 2;
        for (i, (label, value)) in stats.iter().enumerate() {
            let cx = x + (i as i32 % 2) * cell_w as i32;
            let cy = y + (i as i32 / 2) * stat_pitch;
            v.text_clip_centered(
                cx,
                (cy, small),
                cell_w.saturating_sub(12),
                label,
                TextStyle::Regular12,
                t.secondary(),
            );
            v.text_clip_centered(
                cx,
                (cy + 20 + g12, (28 + g20) as u32),
                cell_w.saturating_sub(12),
                value,
                TextStyle::Regular20,
                t.text(),
            );
        }
        if w >= 520 {
            let facts: Vec<(&str, String)> = match (r, s) {
                (0, Some(s)) => alloc::vec![
                    ("Logical processors:", s.cpus.to_string()),
                    ("Sampling:", "100 Hz scheduler timer".into()),
                    ("Update speed:", self.speed.label().into()),
                ],
                (1, Some(s)) => alloc::vec![
                    ("Total:", model::memory_short(s.managed_pages * 4096)),
                    ("Page size:", "4 KB".into()),
                    ("Update speed:", self.speed.label().into()),
                ],
                (_, Some(_)) => alloc::vec![
                    ("Source:", "Kernel device counters".into()),
                    ("Update speed:", self.speed.label().into()),
                ],
                _ => Vec::new(),
            };
            let fx = x + (w / 2) as i32 + 16;
            let key_w = text_w(140);
            for (i, (k, val)) in facts.iter().enumerate() {
                let fy = y + 4 + i as i32 * (22 + g12);
                v.text_clip_centered(
                    fx,
                    (fy, small),
                    key_w,
                    k,
                    TextStyle::Regular12,
                    t.secondary(),
                );
                v.text_clip_centered(
                    fx + key_w as i32,
                    (fy, small),
                    (right - fx - key_w as i32).max(0) as u32,
                    val,
                    TextStyle::Regular12,
                    t.text(),
                );
            }
        }
    }
}
/// Restrict painting to `r` (reset with `View::reset_viewport`); the shared
/// navigation-pane clip is the workspace's public viewport helper.
fn clip(v: &mut View<'_>, r: R) -> bool {
    door_workspace::nav_pane::Pane {
        x: r.x,
        width: r.w,
        top: r.y,
        bottom: r.y + r.h as i32,
        scroll: 0,
        compact: false,
    }
    .clip(v)
}
fn cell(v: &mut View<'_>, t: Theme, c: &Column, row: R, value: &str, color: [u8; 4]) {
    let _ = t;
    if value.is_empty() {
        return;
    }
    if c.right {
        let w = v.measure(value).min(c.w.saturating_sub(16));
        v.text_clip_centered(
            c.x + c.w as i32 - 10 - w as i32,
            (row.y, row.h),
            w + 2,
            value,
            TextStyle::Regular14,
            color,
        );
    } else {
        v.text_clip_centered(
            c.x + 10,
            (row.y, row.h),
            c.w.saturating_sub(16),
            value,
            TextStyle::Regular14,
            color,
        );
    }
}
/// Greedy word wrap with the same 12 px face used to paint the lines.
fn text_lines(v: &View<'_>, text: &str, width: u32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() {
            String::from(word)
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && v.measure_small(&candidate) > width {
            lines.push(core::mem::take(&mut line));
            line = word.into();
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}
