//! One reference-based geometry for every taskbar edge, paint and input path.
//! Metrics follow Windows 11 25H2 at 100 %: a 48 px bar, 40×40 buttons on a
//! 44 px pitch with 24 px icons, and a right-aligned system tray.
extern crate alloc;
use crate::apps::App;
use crate::paint::{self, Palette};
use door_abi::clock::WallClock;
use door_graphics_core::{Canvas, Rect, Scale};
use door_graphics_text::{FontAtlas, TextStyle};
use door_service_protocol::desktop::{Controls, Edge};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    width: u32,
    height: u32,
    controls: Controls,
    /// The `preferences::TASKBAR` word, plus [`REVEALED`] while an
    /// auto-hidden taskbar is shown.
    taskbar: u32,
}
/// Run-time bit in `Layout::with_taskbar`: an auto-hidden taskbar is shown.
pub const REVEALED: u32 = 1 << 31;
/// Run-time bit: the focused window is full screen, so the taskbar is off
/// screen (without auto-hide's reveal edge or its larger work area).
pub const UNDER_FULLSCREEN: u32 = 1 << 30;
/// Labelled app buttons ("Combine taskbar buttons and hide labels": Never).
pub const LABELLED_BUTTON: u32 = 160;
/// What an auto-hidden taskbar leaves on screen (Windows: 2 px).
pub const HIDDEN_SLIVER: u32 = 2;
/// A taskbar button position. App slots index the visible app buttons
/// (pinned apps first, then running unpinned apps).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Start,
    Search,
    TaskView,
    App(usize),
}
/// System tray targets (right end of a horizontal bar).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayTarget {
    Language,
    QuickSettings,
    Clock,
    ShowDesktop,
}
/// Width reserved for the tray on a horizontal bar (at 100% text size).
pub const TRAY_WIDTH: u32 = 216;
/// Accessibility > Text size growth of the tray's text targets:
/// (Regular12 line, clock text column, input indicator column), all 0 at
/// 100%. The taskbar keeps its height; its text targets widen instead.
pub fn tray_text_grow() -> (i32, i32, i32) {
    (
        paint::text_grow(0, TextStyle::Regular12) as i32,
        paint::text_len(72) as i32 - 72,
        paint::text_len(28) as i32 - 28,
    )
}
/// Clock text line height in a tray target of height `slot`: the grown
/// Regular12 line box, compressed so two lines stay inside the bar. 26H2
/// sets the time and date 16 px apart (VIS-01 `taskbar.clock_text`:
/// reference ink rows 17..55 of `taskbar-strip.png` at 150 %, the date's
/// digits 24 px below the time's).
fn clock_line(slot: u32) -> u32 {
    paint::text_grow(16, TextStyle::Regular12)
        .min(slot / 2)
        .max(1)
}
/// Search box width (Windows 11 "Search box" taskbar mode).
pub const SEARCH_BOX: u32 = 216;
impl Layout {
    pub fn new(width: u32, height: u32) -> Self {
        Self::with_controls(width, height, Controls::DEFAULT)
    }
    pub fn with_controls(width: u32, height: u32, controls: Controls) -> Self {
        Self {
            width,
            height,
            controls,
            taskbar: 0,
        }
    }
    /// The same layout under the taskbar behaviours word (`preferences::
    /// TASKBAR`, optionally with [`REVEALED`]).
    pub fn with_taskbar(self, taskbar: u32) -> Self {
        Self { taskbar, ..self }
    }
    pub fn taskbar(self) -> u32 {
        self.taskbar
    }
    fn task_view(self) -> bool {
        self.taskbar & door_service_protocol::preferences::taskbar::HIDE_TASK_VIEW == 0
    }
    /// "Automatically hide the taskbar" is on.
    pub fn auto_hide(self) -> bool {
        self.taskbar & door_service_protocol::preferences::taskbar::AUTO_HIDE != 0
    }
    /// The taskbar is off screen (auto-hidden and not revealed).
    pub fn hidden(self) -> bool {
        self.taskbar & UNDER_FULLSCREEN != 0 || self.auto_hide() && self.taskbar & REVEALED == 0
    }
    /// App buttons carry labels: "Never" combine always, "When taskbar is
    /// full" while `apps` labelled buttons fit.
    pub fn labelled(self, apps: usize) -> bool {
        use door_service_protocol::preferences::taskbar as t;
        if self.vertical() {
            return false;
        }
        match t::combine(self.taskbar) {
            2 => true,
            1 => {
                let fixed = self.offset_of(Slot::App(0), false);
                let room = self.width.saturating_sub(TRAY_WIDTH + 160 + fixed);
                apps as u32 * (LABELLED_BUTTON + 4) <= room
            }
            _ => false,
        }
    }
    /// The edge sliver that reveals an auto-hidden taskbar.
    pub fn reveal_edge(self) -> Rect {
        let s = HIDDEN_SLIVER;
        match self.controls.edge() {
            Edge::Bottom => Rect::new(0, (self.height - s) as i32, self.width, s),
            Edge::Top => Rect::new(0, 0, self.width, s),
            Edge::Left => Rect::new(0, 0, s, self.height),
            Edge::Right => Rect::new((self.width - s) as i32, 0, s, self.height),
        }
        .unwrap()
    }
    /// Layout for raw desktop-controls bits (host tools without the protocol
    /// crate); None for bits Settings could never store.
    pub fn from_bits(width: u32, height: u32, bits: u32) -> Option<Self> {
        Controls::from_bits(bits).map(|controls| Self::with_controls(width, height, controls))
    }
    pub fn controls(self) -> Controls {
        self.controls
    }
    pub fn size(self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub fn output(self) -> Rect {
        Rect::new(0, 0, self.width, self.height).unwrap()
    }
    pub fn vertical(self) -> bool {
        matches!(self.controls.edge(), Edge::Left | Edge::Right)
    }
    pub fn thickness(self) -> u32 {
        if self.large() {
            56
        } else if self.small() {
            40
        } else {
            48
        }
    }
    pub fn button_size(self) -> u32 {
        self.thickness() - 8
    }
    pub fn icon_size(self) -> u32 {
        if self.large() {
            32
        } else if self.small() {
            16
        } else {
            24
        }
    }
    /// Taskbar size: Large (`preferences::taskbar::LARGE`, 56 px) wins over
    /// Small (the desktop-controls bit, 40 px); otherwise Default (48 px).
    pub fn large(self) -> bool {
        self.taskbar & door_service_protocol::preferences::taskbar::LARGE != 0
    }
    pub fn small(self) -> bool {
        self.controls.small() && !self.large()
    }
    pub fn bar(self) -> Rect {
        let t = self.thickness();
        // An auto-hidden taskbar sits just past its edge: everything placed
        // on it (buttons, tray) is off screen and can't be hit.
        let away = if self.hidden() { t as i32 } else { 0 };
        match self.controls.edge() {
            Edge::Bottom => Rect::new(0, (self.height - t) as i32 + away, self.width, t),
            Edge::Top => Rect::new(0, -away, self.width, t),
            Edge::Left => Rect::new(-away, 0, t, self.height),
            Edge::Right => Rect::new((self.width - t) as i32 + away, 0, t, self.height),
        }
        .unwrap()
    }
    pub fn work_area(self) -> Rect {
        // An auto-hidden taskbar leaves the whole screen to windows.
        let t = if self.auto_hide() {
            0
        } else {
            self.thickness()
        };
        match self.controls.edge() {
            Edge::Bottom => Rect::new(0, 0, self.width, self.height - t),
            Edge::Top => Rect::new(0, t as i32, self.width, self.height - t),
            Edge::Left => Rect::new(t as i32, 0, self.width - t, self.height),
            Edge::Right => Rect::new(0, 0, self.width - t, self.height),
        }
        .unwrap()
    }
    /// The search box is shown only on a full-size horizontal bar with room;
    /// otherwise (or when the user chose "Search icon only") a 40 px button.
    pub fn search_box(self) -> bool {
        // 26H2: alternate positions (top, left, right) show the search icon.
        !self.controls.search_icon_only()
            && self.controls.edge() == Edge::Bottom
            && !self.small()
            && self.width >= 1000
    }
    /// The 26H2 search field: 36 px tall (4 px taller than 25H2), centered
    /// in its 40 px slot.
    pub fn search_field(self, apps: usize) -> Rect {
        let r = self.slot(Slot::Search, apps);
        Rect::new(r.x(), r.y() + 2, r.width(), r.height() - 4).unwrap()
    }
    fn search(self) -> u32 {
        if self.search_box() {
            SEARCH_BOX
        } else {
            self.button_size()
        }
    }
    fn length(self, slot: Slot, labelled: bool) -> u32 {
        match slot {
            Slot::Search => self.search(),
            Slot::App(_) if labelled => LABELLED_BUTTON,
            _ => self.button_size(),
        }
    }
    fn offset_of(self, slot: Slot, labelled: bool) -> u32 {
        let step = self.button_size() + 4;
        let app = if labelled { LABELLED_BUTTON + 4 } else { step };
        let view = if self.task_view() { step } else { 0 };
        match slot {
            Slot::Start => 0,
            Slot::Search => step,
            Slot::TaskView => step + self.search() + 4,
            Slot::App(i) => step + self.search() + 4 + view + i as u32 * app,
        }
    }
    /// Group length for `apps` app buttons.
    fn total(self, apps: usize) -> u32 {
        self.offset_of(Slot::App(apps), self.labelled(apps)) - 4
    }
    fn start(self, apps: usize) -> u32 {
        let available = if self.vertical() {
            self.height
        } else {
            self.width
        };
        let tray = if self.vertical() {
            150 + 4 * tray_text_grow().0 as u32
        } else {
            TRAY_WIDTH + 12 + (tray_text_grow().1 + tray_text_grow().2) as u32
        };
        let end = available.saturating_sub(tray);
        if self.controls.start_aligned() {
            4
        } else {
            available.saturating_sub(self.total(apps)) / 2
        }
        .min(end.saturating_sub(self.total(apps)))
        .max(4)
    }
    /// Button plate for `slot` with `apps` visible app buttons.
    pub fn slot(self, slot: Slot, apps: usize) -> Rect {
        let size = self.button_size();
        let labelled = self.labelled(apps);
        let start = self.start(apps) + self.offset_of(slot, labelled);
        let length = self.length(slot, labelled);
        let b = self.bar();
        if self.vertical() {
            Rect::new(b.x() + 4, start as i32, size, length)
        } else {
            Rect::new(start as i32, b.y() + 4, length, size)
        }
        .unwrap()
    }
    pub fn slots(self, apps: usize) -> impl Iterator<Item = (Slot, Rect)> {
        [Slot::Start, Slot::Search, Slot::TaskView]
            .into_iter()
            .filter(move |s| *s != Slot::TaskView || self.task_view())
            .chain((0..apps).map(Slot::App))
            .map(move |s| (s, self.slot(s, apps)))
    }
    pub fn hit_slot(self, x: i32, y: i32, apps: usize) -> Option<Slot> {
        self.slots(apps)
            .find(|(_, r)| r.contains(x, y))
            .map(|v| v.0)
    }
    /// Legacy index form with every bundled app pinned: 0 Start, 1 Search,
    /// 2 Task View, 3.. apps in `App::ALL` order.
    pub fn button(self, index: usize) -> Rect {
        let slot = match index {
            0 => Slot::Start,
            1 => Slot::Search,
            2 => Slot::TaskView,
            i => Slot::App(i - 3),
        };
        self.slot(slot, crate::apps::COUNT)
    }
    pub fn hit(self, x: i32, y: i32) -> Option<usize> {
        (0..3 + crate::apps::COUNT).find(|&i| self.button(i).contains(x, y))
    }
    /// Tray target rectangles. Horizontal bars use Windows 11's right-hand
    /// order (input language, quick settings, clock + notifications, show
    /// desktop); vertical bars stack the same targets at the bottom.
    pub fn tray(self, target: TrayTarget) -> Rect {
        let b = self.bar();
        let plate = self.button_size();
        if self.vertical() {
            let h = self.height as i32;
            // Stacked two-line text targets grow in height with text size.
            let g = 2 * tray_text_grow().0;
            let (y, len) = match target {
                TrayTarget::ShowDesktop => (h - 6, 6),
                TrayTarget::Clock => (h - 6 - 4 - 52 - g, 52 + g),
                TrayTarget::QuickSettings => (h - 62 - g - 4 - 40, 40),
                TrayTarget::Language => (h - 106 - g - 4 - 32 - g, 32 + g),
            };
            let (x, w) = if target == TrayTarget::ShowDesktop {
                (b.x(), b.width())
            } else {
                (b.x() + 4, plate)
            };
            return Rect::new(x, y, w, len as u32).unwrap();
        }
        let r = self.width as i32;
        // The clock and input indicator widen to their (grown) text; the
        // bar height, glyph targets and show-desktop sliver keep their size.
        let (_, cg, lg) = tray_text_grow();
        let (x, w) = match target {
            TrayTarget::ShowDesktop => (r - 6, 6),
            TrayTarget::Clock => (r - 112 - cg, 104 + cg),
            TrayTarget::QuickSettings => (r - 176 - cg, 60),
            TrayTarget::Language => (r - 216 - cg - lg, 36 + lg),
        };
        if target == TrayTarget::ShowDesktop {
            Rect::new(x, b.y(), w as u32, b.height()).unwrap()
        } else {
            Rect::new(x, b.y() + 4, w as u32, plate).unwrap()
        }
    }
    /// Where the "microphone in use" glyph sits while an app records: the
    /// notification-area slot left of (on a vertical bar, above) the input
    /// indicator, as Windows shows it.
    pub fn microphone(self) -> Rect {
        let language = self.tray(TrayTarget::Language);
        if self.vertical() {
            Rect::new(language.x(), language.y() - 36, language.width(), 32).unwrap()
        } else {
            Rect::new(language.x() - 32, language.y(), 32, language.height()).unwrap()
        }
    }
    pub fn hit_tray(self, x: i32, y: i32) -> Option<TrayTarget> {
        // "Select the far corner of the taskbar to show the desktop" off.
        let corner =
            self.taskbar & door_service_protocol::preferences::taskbar::NO_DESKTOP_CORNER == 0;
        [
            TrayTarget::ShowDesktop,
            TrayTarget::Clock,
            TrayTarget::QuickSettings,
            TrayTarget::Language,
        ]
        .into_iter()
        .filter(|t| corner || *t != TrayTarget::ShowDesktop)
        .find(|t| self.tray(*t).contains(x, y))
    }
    /// Start panel bounds (see `start::Geometry`).
    pub fn launcher(self) -> Rect {
        crate::start::Geometry::new(self).panel
    }
    /// Right-anchored tray flyout of `width`×`height` above (or beside) the bar.
    pub fn flyout_sized(self, width: u32, height: u32) -> Rect {
        let a = self.work_area();
        let w = width.min(a.width().saturating_sub(24)).max(1);
        let h = height.min(a.height().saturating_sub(24)).max(1);
        let x = if self.controls.edge() == Edge::Left {
            a.x() + 12
        } else {
            a.right() - 12 - w as i32
        };
        let y = if self.controls.edge() == Edge::Top {
            a.y() + 12
        } else {
            a.bottom() - 12 - h as i32
        };
        Rect::new(x, y, w, h).unwrap()
    }
    pub fn flyout(self, height: u32) -> Rect {
        self.flyout_sized(360, height)
    }
    /// A `w` x `h` surface that belongs to the taskbar button `anchor`
    /// (hover thumbnails, tooltips, jump lists), `gap` px off the bar on
    /// its work-area side: above a bottom bar, below a top one and beside a
    /// left or right one (26H2 positional taskbar), centred on the button
    /// along the bar and kept on screen.
    pub fn beside_bar(self, anchor: Rect, w: u32, h: u32, gap: i32) -> Rect {
        let work = self.work_area();
        let out = self.output();
        let along_x = (anchor.x() + anchor.width() as i32 / 2 - w as i32 / 2)
            .clamp(out.x() + 8, (out.right() - 8 - w as i32).max(out.x()));
        let along_y = (anchor.y() + anchor.height() as i32 / 2 - h as i32 / 2)
            .clamp(work.y() + 8, (work.bottom() - 8 - h as i32).max(work.y()));
        let (x, y) = match self.controls.edge() {
            Edge::Bottom => (along_x, work.bottom() - gap - h as i32),
            Edge::Top => (along_x, work.y() + gap),
            Edge::Left => (work.x() + gap, along_y),
            Edge::Right => (work.right() - gap - w as i32, along_y),
        };
        Rect::new(x, y.max(out.y()), w.max(1), h.max(1)).unwrap()
    }
    /// Retained scene underlay: the whole output, so Start, flyouts, menus
    /// and the snap bar can repaint and animate over a stable backdrop.
    pub fn cache(self) -> Rect {
        self.output()
    }
    pub fn overlay(self, open: bool) -> Rect {
        if open { self.cache() } else { self.shown_bar() }
    }
    /// The on-screen part of the bar: the bar itself, or only its reveal
    /// sliver while auto-hidden or under a full-screen window (the bar
    /// itself is then past the output edge, which no transfer may name).
    pub fn shown_bar(self) -> Rect {
        if self.hidden() {
            self.reveal_edge()
        } else {
            self.bar()
        }
    }
}
/// Physical rectangle for a logical one (edges snapped per `Scale::rect`).
pub fn physical(r: Rect, scale: Scale) -> Rect {
    scale.rect(r).unwrap()
}
/// One taskbar app button's live state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Button {
    pub app: App,
    /// Open top-level windows of the app on the active desktop.
    pub windows: u8,
    /// One of its windows has focus.
    pub active: bool,
}
/// Taskbar buttons as Windows orders them: pinned apps in pin order, then
/// running unpinned apps. `running` lists (app, focused) per open window.
pub fn buttons(pins: &[App], running: &[(App, bool)]) -> alloc::vec::Vec<Button> {
    let mut apps: alloc::vec::Vec<App> = pins.to_vec();
    for app in App::ALL {
        if !apps.contains(&app) && running.iter().any(|r| r.0 == app) {
            apps.push(app);
        }
    }
    apps.into_iter()
        .map(|app| Button {
            app,
            windows: running.iter().filter(|r| r.0 == app).count().min(255) as u8,
            active: running.iter().any(|r| r.0 == app && r.1),
        })
        .collect()
}
/// Everything the taskbar painter needs for one frame.
pub struct View<'a> {
    pub buttons: &'a [Button],
    pub pointer: (i32, i32),
    /// Slot under a held primary button (pressed visual).
    pub pressed: Option<Slot>,
    /// Animated pointer feedback (`Feedback`): the held or releasing
    /// button with its press progress, and the fading hover plates. Empty
    /// values fall back to the static hover/press look.
    pub press: Option<(Slot, u32)>,
    pub hover: [(Option<Slot>, u32); 2],
    pub start_open: bool,
    pub search_open: bool,
    pub task_view_open: bool,
    pub quick_open: bool,
    pub calendar_open: bool,
    /// Windows programs have notification icons: the "Show hidden icons"
    /// chevron shows (true while its flyout is open).
    pub hidden_icons: Option<bool>,
    /// Local civil time (the session's time zone).
    pub time: Option<WallClock>,
    /// Regional format id (`door_locale::region`; unknown = en-US).
    pub region: u16,
    /// Regional format overrides (`preferences::REGIONAL_OVERRIDES`).
    pub overrides: u32,
    /// The Quick settings button's network glyph (door.network).
    pub network: Net,
    /// A VPN connection is up: the network glyph carries its badge.
    pub vpn: bool,
    /// door.audio state for the speaker glyph (None: no sound service).
    pub audio: Option<door_service_protocol::audio::Status>,
    pub dark: bool,
    /// Blurred, tinted taskbar backdrop (bar-sized BGRA), when transparency
    /// effects are on.
    pub material: Option<&'a [u8]>,
}
/// Taskbar clock text: the region's short time ("8:20 PM", "20:20") and
/// short date ("10/22/2026", "22.10.2026"), as Windows 11 shows them.
pub fn clock_text(time: WallClock, region: u16) -> (alloc::string::String, alloc::string::String) {
    clock_text_in(
        time,
        door_locale::region::region_or_default(u32::from(region)),
    )
}
/// `clock_text` in a customized regional format (`tray::State::region_format`).
pub fn clock_text_in(
    time: WallClock,
    r: &door_locale::region::Region,
) -> (alloc::string::String, alloc::string::String) {
    let (mut line1, mut line2) = (alloc::string::String::new(), alloc::string::String::new());
    let _ = r.short_time(&time, &mut line1);
    let _ = r.short_date(&time, &mut line2);
    (line1, line2)
}
fn plate(c: &mut Canvas<'_>, r: Rect, palette: &Palette, pressed: bool, raised: bool) {
    plate_level(c, r, palette, pressed, raised, 1000);
}
/// `plate` with the hover fill at `level` (0..=1000) of its opacity, as
/// it fades in and out; pressed and active plates are always full.
fn plate_level(
    c: &mut Canvas<'_>,
    r: Rect,
    palette: &Palette,
    pressed: bool,
    raised: bool,
    level: u32,
) {
    let level = if pressed || raised {
        1000
    } else {
        level.min(1000)
    };
    if level == 0 {
        return;
    }
    if palette.contrast {
        // Contrast themes: a WindowText outline marks hover and the active
        // app on the solid bar.
        paint::outline(c, r, palette.text, if pressed { 2 } else { 1 });
        return;
    }
    let mut fill = if pressed {
        palette.subtle_pressed
    } else if palette.dark {
        paint::white(14)
    } else {
        paint::white(170)
    };
    fill[3] = (u32::from(fill[3]) * level / 1000) as u8;
    paint::rounded(c, r, 4, fill);
    if raised && !pressed {
        // ControlElevationBorder: a lighter top edge (dark) / darker bottom
        // edge (light) on the active button.
        if palette.dark {
            paint::rect(c, r.x() + 3, r.y(), r.width() - 6, 1, paint::white(18));
        } else {
            paint::rect(
                c,
                r.x() + 3,
                r.bottom() - 1,
                r.width() - 6,
                1,
                paint::black(14),
            );
        }
    }
}
macro_rules! mask_pick {
    ($small:expr, $name24:literal, $name16:literal) => {
        if $small {
            crate::shell_mask!($name16, 16)
        } else {
            crate::shell_mask!($name24, 24)
        }
    };
}
/// Glyph scale while a button is held (Windows' press feedback).
fn press_scale(pressed: bool) -> u32 {
    if pressed {
        crate::search_glyph::PRESSED_PERMILLE
    } else {
        1000
    }
}
/// The network glyph in the Quick settings button, from door.network's
/// status: Ethernet when the adapter has an address, the disconnected
/// globe otherwise (no adapter, cable unplugged, disabled, no lease yet).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Net {
    #[default]
    Disconnected,
    Connected,
}
impl Net {
    pub fn of(status: Option<&door_service_protocol::net::Status>) -> Self {
        match status {
            Some(s) if s.present && s.enabled && s.link && s.address != [0; 4] => Self::Connected,
            _ => Self::Disconnected,
        }
    }
    pub const fn name(self) -> &'static str {
        match self {
            Self::Disconnected => "Disconnected",
            Self::Connected => "Connected",
        }
    }
}
/// App and Start icons while held: Windows 11 26H2 shrinks them to about
/// 80 % (captured: a 36 px icon at 150 % draws 29 px while the button is
/// down) and grows them back on release.
pub const ICON_PRESSED_PERMILLE: u32 = 810;
/// Pointer feedback on taskbar buttons: the held button's icon shrinks
/// (Faster, decelerating) and grows back on release (Fast); hover plates
/// fade in and the previous one fades out (Faster). Times are shell µs.
#[derive(Clone, Copy, Debug, Default)]
pub struct Feedback {
    press_slot: Option<Slot>,
    press: crate::motion::Tween,
    hover: [(Option<Slot>, crate::motion::Tween); 2],
}
impl Feedback {
    /// The pointer went down on `slot` (None: released or left the bar).
    pub fn press(&mut self, slot: Option<Slot>, now: u64, reduce: bool) -> bool {
        use crate::motion::{Curve, FAST_US, FASTER_US};
        match slot {
            Some(slot) => {
                let moved = self.press_slot != Some(slot);
                if moved {
                    self.press = crate::motion::Tween::default();
                }
                self.press_slot = Some(slot);
                let full = if reduce { 0 } else { FASTER_US };
                self.press.set(1000, now, full, Curve::Decelerate) | moved
            }
            None => self
                .press
                .set(0, now, if reduce { 0 } else { FAST_US }, Curve::Decelerate),
        }
    }
    /// The pointer is over `slot` now (None: over no button).
    pub fn hover(&mut self, slot: Option<Slot>, now: u64, reduce: bool) -> bool {
        use crate::motion::{Curve, FASTER_US};
        if self.hover[0].0 == slot {
            return false;
        }
        let full = if reduce { 0 } else { FASTER_US };
        // The old plate fades out from wherever it was; the new one fades in
        // (or keeps fading in, if the pointer came straight back).
        let mut old = self.hover[0];
        old.1.set(0, now, full, Curve::Linear);
        let mut new = if self.hover[1].0 == slot && slot.is_some() {
            self.hover[1]
        } else {
            (slot, crate::motion::Tween::default())
        };
        if slot.is_some() {
            new.1.set(1000, now, full, Curve::Linear);
        }
        self.hover = [new, old];
        true
    }
    pub fn animating(&self, now: u64) -> bool {
        self.press.animating(now) || self.hover.iter().any(|h| h.1.animating(now))
    }
    /// The held (or releasing) button and its press progress (0..=1000).
    pub fn pressed(&self, now: u64) -> Option<(Slot, u32)> {
        let value = self.press.value(now);
        self.press_slot
            .filter(|_| value > 0)
            .map(|slot| (slot, value))
    }
    /// Hover plate levels (0..=1000) of the hovered and the previous button.
    pub fn hovered(&self, now: u64) -> [(Option<Slot>, u32); 2] {
        self.hover.map(|(slot, t)| (slot, t.value(now)))
    }
}
/// Icon scale (per mille) for a press progress.
fn icon_scale(progress: u32) -> u32 {
    1000 - (1000 - ICON_PRESSED_PERMILLE) * progress.min(1000) / 1000
}
/// door.shell additions to one taskbar frame: extra buttons after the
/// catalog apps (registered and Windows apps), per-app attention/progress
/// marks of the catalog buttons, and the notification bell's state.
#[derive(Clone, Copy)]
pub struct Extras<'a> {
    pub buttons: &'a [crate::shell_link::Extra],
    pub marks: &'a [(App, crate::shell_link::WindowState)],
    /// Unread notifications (0: the plain bell).
    pub badge: u8,
    pub dnd: bool,
}
impl Extras<'_> {
    pub const NONE: Extras<'static> = Extras {
        buttons: &[],
        marks: &[],
        badge: 0,
        dnd: false,
    };
    fn mark(&self, app: App) -> crate::shell_link::WindowState {
        self.marks.iter().filter(|(a, _)| *a == app).fold(
            crate::shell_link::WindowState::default(),
            |m, (_, s)| crate::shell_link::WindowState {
                attention: m.attention || s.attention,
                progress_state: if m.progress_state != 0 {
                    m.progress_state
                } else {
                    s.progress_state
                },
                progress: if m.progress_state != 0 {
                    m.progress
                } else {
                    s.progress
                },
            },
        )
    }
}
/// Running dot / active pill (attention: orange pill), or the progress bar.
#[allow(clippy::too_many_arguments)]
fn indicator(
    c: &mut Canvas<'_>,
    r: Rect,
    edge: Edge,
    palette: &Palette,
    dark: bool,
    windows: usize,
    active: bool,
    attention: bool,
    progress: Option<(u8, u16)>,
) {
    if let Some((state, value)) = progress {
        crate::shell_view::progress_bar(c, r, edge, state, value, palette);
        return;
    }
    if windows == 0 {
        return;
    }
    // Running dot (6×3, neutral) versus the active pill (16×3, accent),
    // inside the bottom of the plate.
    let length: u32 = if active || attention { 16 } else { 6 };
    let color = if attention && !palette.contrast {
        crate::shell_view::ATTENTION_PILL
    } else if active {
        palette.accent
    } else if palette.contrast {
        palette.text
    } else if dark {
        paint::rgb(152, 154, 160)
    } else {
        paint::rgb(110, 110, 110)
    };
    let indicator = match edge {
        Edge::Left => Rect::new(
            r.x() + 1,
            r.y() + (r.height() - length) as i32 / 2,
            3,
            length,
        ),
        Edge::Right => Rect::new(
            r.right() - 4,
            r.y() + (r.height() - length) as i32 / 2,
            3,
            length,
        ),
        Edge::Top => Rect::new(
            r.x() + (r.width() - length) as i32 / 2,
            r.y() + 1,
            length,
            3,
        ),
        Edge::Bottom => Rect::new(
            r.x() + (r.width() - length) as i32 / 2,
            r.bottom() - 4,
            length,
            3,
        ),
    };
    if let Ok(indicator) = indicator {
        paint::rounded(c, indicator, 1, color);
    }
}
/// Paint the bar band: backdrop, app buttons and tray.
pub fn draw(c: &mut Canvas<'_>, fonts: &FontAtlas, layout: Layout, view: &View<'_>) {
    draw_with(c, fonts, layout, view, &Extras::NONE);
}
/// `draw` with the desktop shell's extra buttons, marks and bell state.
pub fn draw_with(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    layout: Layout,
    view: &View<'_>,
    extras: &Extras<'_>,
) {
    // An auto-hidden taskbar paints nothing until it is revealed.
    if layout.hidden() {
        return;
    }
    let palette = Palette::new(view.dark);
    let bar = layout.bar();
    match view.material {
        Some(pixels) => {
            let scale = c.scale();
            let p = physical(bar, scale);
            let stride = c.physical_dimensions().2;
            let row = p.width() as usize * 4;
            if pixels.len() >= row * p.height() as usize {
                for y in 0..p.height() as usize {
                    let at = (p.y() as usize + y) * stride + p.x() as usize * 4;
                    c.pixels_mut()[at..at + row].copy_from_slice(&pixels[y * row..(y + 1) * row]);
                }
            }
        }
        None => paint::fill(
            c,
            bar,
            if palette.contrast {
                palette.surface
            } else if view.dark {
                paint::start_tint(true).unwrap_or(paint::rgb(28, 28, 28))
            } else {
                paint::rgb(238, 238, 238)
            },
        ),
    }
    // Hairline separating the bar from the desktop.
    let line = if palette.contrast {
        palette.surface_border
    } else if view.dark {
        paint::white(30)
    } else {
        paint::black(22)
    };
    match layout.controls.edge() {
        Edge::Bottom => paint::rect(c, bar.x(), bar.y(), bar.width(), 1, line),
        Edge::Top => paint::rect(c, bar.x(), bar.bottom() - 1, bar.width(), 1, line),
        Edge::Left => paint::rect(c, bar.right() - 1, bar.y(), 1, bar.height(), line),
        Edge::Right => paint::rect(c, bar.x(), bar.y(), 1, bar.height(), line),
    }
    let apps = view.buttons.len() + extras.buttons.len();
    let small = layout.small();
    let icon = layout.icon_size() as i32;
    for (slot, r) in layout.slots(apps) {
        let hovered = r.contains(view.pointer.0, view.pointer.1);
        let pressed = view.pressed == Some(slot) && hovered;
        // Animated feedback when the shell drives it, else the static look.
        let animated = view.hover.iter().any(|h| h.0.is_some());
        let level = if animated {
            view.hover
                .iter()
                .find(|h| h.0 == Some(slot))
                .map_or(0, |h| h.1)
        } else if hovered {
            1000
        } else {
            0
        };
        let scale = view
            .press
            .filter(|p| p.0 == slot)
            .map_or(if pressed { ICON_PRESSED_PERMILLE } else { 1000 }, |p| {
                icon_scale(p.1)
            });
        let button = match slot {
            Slot::App(i) => view.buttons.get(i).copied(),
            _ => None,
        };
        let extra = match slot {
            Slot::App(i) => i
                .checked_sub(view.buttons.len())
                .and_then(|j| extras.buttons.get(j)),
            _ => None,
        };
        let mark = button.map(|b| extras.mark(b.app)).unwrap_or_default();
        // "Show flashing on taskbar apps" off keeps attention unpainted.
        let flashing =
            layout.taskbar() & door_service_protocol::preferences::taskbar::NO_FLASHING == 0;
        let attention = flashing && (mark.attention || extra.is_some_and(|e| e.attention));
        let active = match slot {
            Slot::Start => view.start_open,
            Slot::Search => view.search_open,
            Slot::TaskView => view.task_view_open,
            Slot::App(_) => button.is_some_and(|b| b.active) || extra.is_some_and(|e| e.active),
        };
        let search_box = slot == Slot::Search && layout.search_box();
        if search_box {
            // Rounded search box: subtle fill, hairline border, magnifier and
            // placeholder, lighter on hover.
            let field = layout.search_field(apps);
            let (fill, border) = if palette.contrast {
                (palette.field, palette.field_border)
            } else if view.dark {
                (
                    paint::white(if hovered || active { 30 } else { 20 }),
                    paint::white(26),
                )
            } else {
                (
                    paint::white(if hovered || active { 255 } else { 200 }),
                    paint::black(20),
                )
            };
            paint::rounded(c, field, field.height() / 2, border);
            if let Some(inner) = paint::inset(field, 1, 1) {
                paint::rounded(c, inner, inner.height() / 2, fill);
            }
            // 26H2: the coloured search glyph (monochrome in contrast
            // themes), shrinking while pressed.
            crate::search_glyph::draw(
                c,
                field.x() + 12,
                field.y() + (field.height() as i32 - 16) / 2,
                16,
                &palette,
                press_scale(pressed),
            );
            paint::text_row(
                c,
                fonts,
                field.x() + 40,
                field.y(),
                field.height(),
                "Search",
                TextStyle::Regular14,
                palette.text_secondary,
                field.width() - 48,
            );
            continue;
        }
        if attention && !hovered && !pressed {
            crate::shell_view::attention_plate(c, r, &palette);
        } else if level > 0 || pressed || active {
            plate_level(
                c,
                r,
                &palette,
                pressed,
                active && slot != Slot::Search,
                level,
            );
        }
        // Labelled buttons put the icon at the left and the name after it.
        let labelled = matches!(slot, Slot::App(_)) && r.width() > layout.button_size();
        let inset_x = if labelled {
            r.x() + 8
        } else {
            r.x() + (r.width() as i32 - icon) / 2
        };
        let inset_y = r.y() + (r.height() as i32 - icon) / 2;
        if labelled {
            let label = match (button, extra) {
                (Some(b), _) => alloc::string::String::from(b.app.label()),
                (None, Some(e)) => e.label.clone(),
                _ => alloc::string::String::new(),
            };
            let room = r.width().saturating_sub(icon as u32 + 24);
            let text = paint::ellipsize(fonts, &label, TextStyle::Regular12, room);
            paint::text_row(
                c,
                fonts,
                inset_x + icon + 8,
                r.y(),
                r.height(),
                &text,
                TextStyle::Regular12,
                palette.text,
                room,
            );
        }
        match slot {
            Slot::Start => crate::apps::draw_logo_scaled(c, inset_x, inset_y, icon as u32, scale),
            Slot::Search => crate::search_glyph::draw(
                c,
                inset_x,
                inset_y,
                icon as u32,
                &palette,
                press_scale(pressed),
            ),
            Slot::TaskView => paint::glyph(
                c,
                inset_x,
                inset_y,
                icon as u32,
                mask_pick!(small, "shell-taskview", "shell-taskview-small"),
                palette.text,
            ),
            Slot::App(_) => {
                if let Some(button) = button {
                    crate::apps::draw_icon_scaled(
                        c,
                        inset_x,
                        inset_y,
                        button.app,
                        icon as u32,
                        scale,
                    );
                    indicator(
                        c,
                        r,
                        layout.controls.edge(),
                        &palette,
                        view.dark,
                        usize::from(button.windows),
                        button.active,
                        mark.attention,
                        mark.progress(),
                    );
                } else if let Some(e) = extra {
                    crate::shell_view::draw_icon(c, inset_x, inset_y, icon as u32, e.icon);
                    indicator(
                        c,
                        r,
                        layout.controls.edge(),
                        &palette,
                        view.dark,
                        e.windows(),
                        e.active,
                        e.attention,
                        e.progress,
                    );
                }
            }
        }
    }
    draw_tray(c, fonts, layout, view, &palette, extras);
}
/// What the taskbar input indicator shows (Windows 11): the language over
/// the keyboard ("ENG" / "US"), or the IME conversion mode over the method
/// ("中" / "拼", "あ" / "JP").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputIndicator {
    pub top: &'static str,
    pub bottom: &'static str,
}
impl InputIndicator {
    pub fn new(
        indicator: door_keyboard_layouts::Indicator,
        method: &door_keyboard_layouts::Method,
    ) -> Self {
        match indicator {
            door_keyboard_layouts::Indicator::Keyboard { language, keyboard } => Self {
                top: language,
                bottom: keyboard,
            },
            door_keyboard_layouts::Indicator::Ime { glyph, .. } => Self {
                top: glyph,
                bottom: method.short,
            },
        }
    }
}
/// Paint the input indicator in its tray slot (hover plate, or the open
/// plate while its switcher shows). Nothing when no indicator is shown.
#[allow(clippy::too_many_arguments)]
pub fn draw_input(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    layout: Layout,
    indicator: Option<InputIndicator>,
    pointer: (i32, i32),
    open: bool,
    dark: bool,
) {
    let Some(i) = indicator else { return };
    let palette = Palette::new(dark);
    let r = layout.tray(TrayTarget::Language);
    if open || r.contains(pointer.0, pointer.1) {
        plate(c, r, &palette, false, false);
    }
    let line = paint::text_grow(16, TextStyle::Regular12)
        .min(r.height() / 2)
        .max(1);
    let top = r.y() + (r.height() as i32 - 2 * line as i32) / 2;
    paint::text_span(
        c,
        fonts,
        r.x(),
        r.width(),
        top,
        line,
        i.top,
        TextStyle::Regular12,
        palette.text,
    );
    paint::text_span(
        c,
        fonts,
        r.x(),
        r.width(),
        top + line as i32,
        line,
        i.bottom,
        TextStyle::Regular12,
        palette.text,
    );
}
fn draw_tray(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    layout: Layout,
    view: &View<'_>,
    palette: &Palette,
    extras: &Extras<'_>,
) {
    let fg = palette.text;
    for target in [TrayTarget::QuickSettings, TrayTarget::Clock] {
        let r = layout.tray(target);
        let open = match target {
            TrayTarget::QuickSettings => view.quick_open,
            TrayTarget::Clock => view.calendar_open,
            _ => false,
        };
        if open || r.contains(view.pointer.0, view.pointer.1) {
            plate(c, r, palette, false, false);
        }
    }
    let vertical = layout.vertical();
    // The input indicator is painted by `draw_input` from the session's
    // installed methods (none with a single keyboard, as in Windows).
    // Quick settings: the network glyph from door.network, and Windows'
    // muted glyph (no audio device on this profile). No battery.
    let qs = layout.tray(TrayTarget::QuickSettings);
    let gy = qs.y() + (qs.height() as i32 - 16) / 2;
    let gx = qs.x() + (qs.width() as i32 - 44) / 2;
    paint::glyph(
        c,
        gx,
        gy,
        16,
        match view.network {
            Net::Connected => crate::shell_mask!("shell-ethernet", 16),
            Net::Disconnected => crate::shell_mask!("network-disconnected", 16),
        },
        fg,
    );
    // A VPN is up: Windows 11 badges the network glyph with a shield at its
    // lower right, cut out of the glyph by a ring of the bar's colour.
    if view.vpn {
        let (bx, by) = (gx + 7, gy + 6);
        let ring = if view.dark {
            paint::rgb(32, 32, 32)
        } else {
            paint::rgb(238, 238, 238)
        };
        for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
            paint::glyph(
                c,
                bx + dx,
                by + dy,
                12,
                crate::shell_mask!("shell-shield", 12),
                ring,
            );
        }
        paint::glyph(c, bx, by, 12, crate::shell_mask!("shell-shield", 12), fg);
    }
    paint::glyph(
        c,
        gx + 28,
        gy,
        16,
        crate::tray::volume_glyph(view.audio.as_ref()),
        fg,
    );
    // An app is recording: the microphone glyph in the notification area.
    if view.audio.as_ref().is_some_and(|a| a.capturing > 0) {
        let mic = layout.microphone();
        paint::glyph(
            c,
            mic.x() + (mic.width() as i32 - 16) / 2,
            mic.y() + (mic.height() as i32 - 16) / 2,
            16,
            crate::shell_mask!("shell-microphone", 16),
            fg,
        );
    }
    // Windows programs' notification icons: the "Show hidden icons"
    // chevron (pointing down while its flyout is open).
    if let Some(open) = view.hidden_icons {
        let recording = view.audio.as_ref().is_some_and(|a| a.capturing > 0);
        let r = crate::notify_icons::chevron(layout, recording);
        if open || r.contains(view.pointer.0, view.pointer.1) {
            plate(c, r, palette, false, false);
        }
        paint::glyph(
            c,
            r.x() + (r.width() as i32 - 16) / 2,
            r.y() + (r.height() as i32 - 16) / 2,
            16,
            if open {
                crate::shell_mask!("shell-chevron-down", 16)
            } else {
                crate::shell_mask!("chevron-up", 16)
            },
            fg,
        );
    }
    let clock = layout.tray(TrayTarget::Clock);
    match view.time {
        Some(time) => {
            let r = door_locale::region::region_or_default(u32::from(view.region))
                .customized(door_locale::region::Overrides::from_word(view.overrides));
            let (mut line1, line2) = clock_text_in(time, &r);
            // "Show seconds in system tray clock": the long time format.
            if layout.taskbar() & door_service_protocol::preferences::taskbar::SECONDS != 0 {
                line1.clear();
                let _ = r.long_time(&time, &mut line1);
            }
            if vertical {
                let mut short = alloc::string::String::new();
                let _ = r.format(r.numeric_month_day, &time, &mut short);
                let (h, pitch) = (
                    paint::text_grow(18, TextStyle::Regular12),
                    paint::text_grow_i(20, TextStyle::Regular12),
                );
                paint::text_span(
                    c,
                    fonts,
                    clock.x(),
                    clock.width(),
                    clock.y() + 6,
                    h,
                    &line1,
                    TextStyle::Regular12,
                    fg,
                );
                paint::text_span(
                    c,
                    fonts,
                    clock.x(),
                    clock.width(),
                    clock.y() + 6 + pitch,
                    h,
                    &short,
                    TextStyle::Regular12,
                    fg,
                );
            } else {
                let right = clock.right() - 32;
                let line = clock_line(clock.height());
                let top = clock.y() + clock.height() as i32 / 2 - line as i32;
                paint::text_right(c, fonts, right, top, line, &line1, TextStyle::Regular12, fg);
                paint::text_right(
                    c,
                    fonts,
                    right,
                    top + line as i32,
                    line,
                    &line2,
                    TextStyle::Regular12,
                    fg,
                );
            }
        }
        None => {
            if !vertical {
                paint::text_right(
                    c,
                    fonts,
                    clock.right() - 32,
                    clock.y(),
                    clock.height(),
                    "--:--",
                    TextStyle::Regular12,
                    palette.text_secondary,
                );
            }
        }
    }
    // Notification bell (part of the clock button, as in Windows 11). A
    // vertical bar has no room beside the clock; the clock opens it.
    // Unread notifications replace the bell with a count badge; Do not
    // disturb shows its crescent glyph (Windows 11).
    if !vertical {
        let (bx, by) = (
            clock.right() - 20,
            clock.y() + (clock.height() as i32 - 16) / 2,
        );
        if extras.badge > 0 && !extras.dnd {
            crate::shell_view::draw_badge(c, fonts, bx + 8, by + 8, extras.badge, palette);
        } else if extras.dnd {
            paint::glyph(
                c,
                bx - 2,
                by - 2,
                20,
                crate::shell_mask!("setting-quiet", 20),
                fg,
            );
        } else {
            paint::glyph(c, bx, by, 16, crate::shell_mask!("notification", 16), fg);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn group_is_centered_with_windows_metrics() {
        for (w, h) in [(1280, 720), (1920, 1080), (1707, 1019)] {
            let l = Layout::new(w, h);
            let start = l.slot(Slot::Start, 6);
            let last = l.slot(Slot::App(5), 6);
            assert_eq!((start.width(), start.height()), (40, 40));
            assert_eq!(start.y() as u32, h - 44);
            assert!((start.x() + last.right() - w as i32).abs() <= 1);
            // 44 px pitch between neighbouring app buttons.
            assert_eq!(
                l.slot(Slot::App(1), 6).x() - l.slot(Slot::App(0), 6).x(),
                44
            );
            assert_eq!(l.slot(Slot::Search, 6).width(), SEARCH_BOX);
        }
        let icon_only = Layout::with_controls(
            1920,
            1080,
            Controls::DEFAULT.with(Controls::SEARCH_ICON_ONLY, true),
        );
        assert_eq!(icon_only.slot(Slot::Search, 6).width(), 40);
        assert_eq!(
            icon_only.slot(Slot::Search, 6).x() - icon_only.slot(Slot::Start, 6).x(),
            44
        );
        assert!(!Layout::new(800, 600).search_box());
        let field = Layout::new(1920, 1080).search_field(6);
        assert_eq!(field.height(), 36, "26H2 search box is 36 px tall");
        for edge in [Edge::Top as u32, Edge::Left as u32, Edge::Right as u32] {
            let alternate = Layout::from_bits(1920, 1080, 48 | edge).unwrap();
            assert!(!alternate.search_box());
            assert_eq!(alternate.slot(Slot::Search, 6).width(), 40);
        }
    }
    #[test]
    fn slots_and_tray_are_disjoint_inside_the_bar_for_every_control() {
        for (width, height) in [(800, 600), (1280, 720), (1920, 1080)] {
            for bits in 0..16 {
                for search in [false, true] {
                    let controls = Controls::from_bits(bits | 48)
                        .unwrap()
                        .with(Controls::SEARCH_ICON_ONLY, search);
                    let l = Layout::with_controls(width, height, controls);
                    assert!(l.work_area().intersection(l.bar()).is_none());
                    for apps in [0, 3, 6] {
                        let rects: alloc::vec::Vec<_> = l.slots(apps).collect();
                        for (i, (slot, r)) in rects.iter().enumerate() {
                            assert_eq!(l.bar().intersection(*r), Some(*r), "{bits} {slot:?}");
                            assert_eq!(l.hit_slot(r.x(), r.y(), apps), Some(*slot));
                            assert_eq!(
                                l.hit_slot(r.right() - 1, r.bottom() - 1, apps),
                                Some(*slot)
                            );
                            for (_, other) in &rects[..i] {
                                assert!(r.intersection(*other).is_none());
                            }
                            for t in [
                                TrayTarget::Language,
                                TrayTarget::QuickSettings,
                                TrayTarget::Clock,
                                TrayTarget::ShowDesktop,
                            ] {
                                assert!(
                                    l.tray(t).intersection(*r).is_none(),
                                    "{bits} {apps} {t:?}"
                                );
                            }
                        }
                    }
                    for t in [
                        TrayTarget::Language,
                        TrayTarget::QuickSettings,
                        TrayTarget::Clock,
                        TrayTarget::ShowDesktop,
                    ] {
                        let r = l.tray(t);
                        assert_eq!(l.bar().intersection(r), Some(r));
                        assert_eq!(l.hit_tray(r.x(), r.y()), Some(t));
                    }
                    for h in [380, 500] {
                        let r = l.flyout(h);
                        assert_eq!(l.work_area().intersection(r), Some(r));
                    }
                }
            }
        }
    }
    #[test]
    fn legacy_button_indices_map_to_all_pinned_slots() {
        let l = Layout::new(1280, 720);
        assert_eq!(l.button(0), l.slot(Slot::Start, 6));
        assert_eq!(l.button(3), l.slot(Slot::App(0), 6));
        for i in 0..9 {
            let r = l.button(i);
            assert_eq!(l.hit(r.x() + 1, r.y() + 1), Some(i));
        }
    }
    #[test]
    fn pinned_apps_lead_and_running_unpinned_apps_follow() {
        let pins = [App::Settings, App::Files];
        let running = [
            (App::Terminal, false),
            (App::Files, true),
            (App::Terminal, false),
        ];
        let b = buttons(&pins, &running);
        let order: alloc::vec::Vec<_> = b.iter().map(|b| b.app).collect();
        assert_eq!(order, [App::Settings, App::Files, App::Terminal]);
        assert_eq!((b[0].windows, b[0].active), (0, false));
        assert_eq!((b[1].windows, b[1].active), (1, true));
        assert_eq!((b[2].windows, b[2].active), (2, false));
        // Unpinning a closed app removes its button entirely.
        assert_eq!(buttons(&[], &[]).len(), 0);
    }
    fn render(
        scale: u32,
        dark: bool,
        material: Option<[u8; 4]>,
        pointer: (i32, i32),
        buttons: &[Button],
    ) -> (alloc::vec::Vec<u8>, Layout) {
        let layout = Layout::new(1280, 720);
        let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
        let (pw, ph) = (1280 * scale as usize, 720 * scale as usize);
        let mut bytes = alloc::vec![255u8; pw * ph * 4];
        let bar = physical(layout.bar(), Scale::from_factor(scale).unwrap());
        let backdrop: alloc::vec::Vec<u8> = material
            .map(|tint| tint.repeat(bar.width() as usize * bar.height() as usize))
            .unwrap_or_default();
        let mut c = Canvas::new(&mut bytes, pw as u32, ph as u32, pw * 4).unwrap();
        c.set_scale(Scale::from_factor(scale).unwrap()).unwrap();
        draw(
            &mut c,
            &fonts,
            layout,
            &View {
                buttons,
                pointer,
                pressed: None,
                press: None,
                hover: [(None, 0); 2],
                start_open: false,
                search_open: false,
                task_view_open: false,
                quick_open: false,
                calendar_open: false,
                hidden_icons: None,
                time: None,
                region: 0,
                overrides: 0,
                network: Net::Connected,
                vpn: false,
                audio: None,
                dark,
                material: material.map(|_| &backdrop[..]),
            },
        );
        (bytes, layout)
    }
    /// The 26H2 search pill: "Search" is grayscale (acrylic backdrop, no
    /// ClearType fringes) and its cap height shares the vertical centre of
    /// the 16 px magnifier, which is the 36 px field's centre, at 1x and 2x.
    #[test]
    fn search_pill_text_is_grayscale_and_centred_with_its_glyph() {
        use crate::paint::tests::{channel_spread, ink};
        for scale in [1u32, 2] {
            let s = scale as i32;
            for dark in [false, true] {
                // Opaque tinted acrylic, as `material::Taskbar` produces.
                let tint = if dark {
                    paint::rgb(32, 40, 58)
                } else {
                    paint::rgb(206, 216, 236)
                };
                let (bytes, layout) = render(scale, dark, Some(tint), (0, 0), &[]);
                let pw = 1280 * scale as usize;
                let field = layout.search_field(0);
                assert_eq!((field.height(), field.width()), (36, SEARCH_BOX));
                let palette = Palette::new(dark);
                // Field fill over the tint, sampled between glyph and text.
                let at =
                    (((field.y() + 4) * s) as usize * pw + ((field.x() + 34) * s) as usize) * 4;
                let bg: [u8; 4] = bytes[at..at + 4].try_into().unwrap();
                let text = Rect::new(
                    (field.x() + 36) * s,
                    (field.y() + 2) * s,
                    80 * scale,
                    32 * scale,
                )
                .unwrap();
                let (spread, edges) = channel_spread(&bytes, pw, text, bg, palette.text_secondary);
                assert!(edges > 20, "{scale}x placeholder drawn");
                assert!(
                    spread <= 4,
                    "{scale}x dark={dark}: placeholder has colour fringes ({spread}/255)"
                );
                // Cap centre of "Search" (its "S"; the "h" ascender rises
                // above cap height) versus the magnifier's ink centre: one
                // shared vertical centre.
                let cap = Rect::new(text.x(), text.y(), 11 * scale, text.height()).unwrap();
                let (_, _, t, b) = ink(&bytes, pw, cap, bg, palette.text_secondary).unwrap();
                let glyph = Rect::new(
                    (field.x() + 8) * s,
                    (field.y() + 2) * s,
                    24 * scale,
                    32 * scale,
                )
                .unwrap();
                let (_, _, gt, gb) = ink(&bytes, pw, glyph, bg, palette.text).unwrap();
                let centre = (2 * field.y() + field.height() as i32) * s;
                assert!(
                    (gt + gb - centre).abs() <= s,
                    "{scale}x glyph {gt}..{gb} vs {centre}/2"
                );
                assert!(
                    (t + b - centre).abs() <= s,
                    "{scale}x text {t}..{b} vs {centre}/2"
                );
            }
        }
    }
    /// Taskbar app icons and the Start mark are their native frames blitted
    /// 1:1 at rest (1x, 2x and 3x, idle and hovered): every physical pixel is
    /// exactly the source frame over the button's backplate, with no
    /// resampling, half-pixel offset or premultiplication error.
    #[test]
    fn taskbar_icons_are_native_frames_blitted_exactly_at_rest() {
        let buttons = [
            Button {
                app: App::Files,
                windows: 1,
                active: true,
            },
            Button {
                app: App::Notes,
                windows: 1,
                active: false,
            },
            Button {
                app: App::Settings,
                windows: 0,
                active: false,
            },
            Button {
                app: App::Terminal,
                windows: 2,
                active: false,
            },
            Button {
                app: App::Preview,
                windows: 0,
                active: false,
            },
            Button {
                app: App::Monitor,
                windows: 0,
                active: false,
            },
        ];
        let over = |d: [u8; 4], s: &[u8]| -> [u8; 4] {
            let inverse = 255 - u32::from(s[3]);
            core::array::from_fn(|i| {
                (u32::from(s[i]) + (u32::from(d[i]) * inverse + 127) / 255).min(255) as u8
            })
        };
        for scale in [1u32, 2, 3] {
            let s = scale as i32;
            let pw = 1280 * scale as usize;
            for dark in [false, true] {
                let layout = Layout::new(1280, 720);
                let icon = layout.icon_size();
                let mut slots: alloc::vec::Vec<(Rect, &[u8])> = alloc::vec![(
                    layout.slot(Slot::Start, buttons.len()),
                    crate::apps::logo_pixels(icon * scale).unwrap(),
                )];
                for (i, b) in buttons.iter().enumerate() {
                    assert_eq!(
                        crate::apps::icon_size(b.app, icon, Scale::from_factor(scale).unwrap()),
                        icon,
                        "{:?} has a native {}px frame",
                        b.app,
                        icon * scale
                    );
                    slots.push((
                        layout.slot(Slot::App(i), buttons.len()),
                        crate::apps::icon_pixels(b.app, icon * scale).unwrap(),
                    ));
                }
                for (r, source) in slots {
                    for pointer in [(0, 0), (r.x() + 2, r.y() + 2)] {
                        let (bytes, _) = render(scale, dark, None, pointer, &buttons);
                        let (x0, y0) = (
                            (r.x() + (r.width() - icon) as i32 / 2) * s,
                            (r.y() + (r.height() - icon) as i32 / 2) * s,
                        );
                        // The backplate under the icon is uniform: sample it
                        // just left of the icon on its first row.
                        let at = (y0 as usize * pw + x0 as usize - 1) * 4;
                        let bg: [u8; 4] = bytes[at..at + 4].try_into().unwrap();
                        let n = (icon * scale) as usize;
                        assert_eq!(source.len(), n * n * 4);
                        for y in 0..n {
                            for x in 0..n {
                                let at = ((y0 as usize + y) * pw + x0 as usize + x) * 4;
                                let want = over(bg, &source[(y * n + x) * 4..][..4]);
                                assert_eq!(
                                    bytes[at..at + 4],
                                    want,
                                    "{scale}x {r:?} pointer {pointer:?} pixel {x},{y}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn press_shrinks_the_icon_and_hover_plates_cross_fade() {
        use crate::motion::{FAST_US, FASTER_US};
        let mut f = Feedback::default();
        assert_eq!(f.pressed(0), None);
        let app = Slot::App(2);
        assert!(f.press(Some(app), 1_000, false));
        let (slot, held) = f.pressed(1_000 + FASTER_US).unwrap();
        assert_eq!((slot, held), (app, 1000));
        assert_eq!(icon_scale(held), ICON_PRESSED_PERMILLE);
        // Release: grows back over Fast, then rests at full size.
        let up = 1_000 + FASTER_US + 5;
        assert!(f.press(None, up, false));
        let back = f.pressed(up + FAST_US / 3).unwrap().1;
        assert!(back > 0 && back < 1000);
        assert_eq!(f.pressed(up + FAST_US), None);
        assert!(f.animating(up + FAST_US), "owes a final frame");
        // Hover A then B: A fades out while B fades in.
        let (a, b) = (Slot::App(0), Slot::App(1));
        f.hover(Some(a), 0, false);
        f.hover(Some(b), FASTER_US, false);
        let [(s0, v0), (s1, v1)] = f.hovered(FASTER_US + FASTER_US / 2);
        assert_eq!((s0, s1), (Some(b), Some(a)));
        assert!(v0 > 0 && v0 < 1000 && v1 > 0 && v1 < 1000);
        assert_eq!(f.hovered(10 * FASTER_US)[0], (Some(b), 1000));
        // Reduced motion jumps.
        let mut r = Feedback::default();
        r.press(Some(app), 0, true);
        assert_eq!(r.pressed(0), Some((app, 1000)));
    }
    #[test]
    fn clock_uses_windows_us_format() {
        let t = |hour, minute| WallClock {
            year: 2026,
            month: 10,
            day: 22,
            hour,
            minute,
            second: 0,
        };
        assert_eq!(clock_text(t(20, 5), 0).0, "8:05 PM");
        assert_eq!(clock_text(t(0, 0), 1).0, "12:00 AM");
        assert_eq!(clock_text(t(12, 30), 1).0, "12:30 PM");
        assert_eq!(clock_text(t(9, 7), 1).1, "10/22/2026");
        // Regional formats: en-GB, de-DE, zh-CN, ko-KR.
        assert_eq!(
            clock_text(t(20, 5), 2),
            ("20:05".into(), "22/10/2026".into())
        );
        assert_eq!(
            clock_text(t(20, 5), 3),
            ("20:05".into(), "22.10.2026".into())
        );
        assert_eq!(clock_text(t(9, 7), 5), ("9:07".into(), "2026/10/22".into()));
        assert_eq!(
            clock_text(t(20, 5), 7),
            ("오후 8:05".into(), "2026-10-22".into())
        );
    }
    #[test]
    fn input_indicator_shows_language_and_keyboard_or_the_ime_mode() {
        use door_keyboard_layouts::{methods, methods::method};
        let us = method(1).unwrap();
        let pinyin = method(7).unwrap();
        assert_eq!(
            InputIndicator::new(methods::indicator(us, 0), us),
            InputIndicator {
                top: "ENG",
                bottom: "US"
            }
        );
        assert_eq!(
            InputIndicator::new(methods::indicator(pinyin, 1), pinyin),
            InputIndicator {
                top: "中",
                bottom: "拼"
            }
        );
        assert_eq!(
            InputIndicator::new(methods::indicator(pinyin, 0), pinyin).top,
            "英"
        );
        let layout = Layout::new(1280, 720);
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let mut bytes = alloc::vec![0u8; 1280 * 720 * 4];
        let mut c = Canvas::new(&mut bytes, 1280, 720, 1280 * 4).unwrap();
        draw_input(&mut c, &fonts, layout, None, (0, 0), false, false);
        assert!(
            bytes.iter().all(|b| *b == 0),
            "no indicator with one keyboard"
        );
        let mut c = Canvas::new(&mut bytes, 1280, 720, 1280 * 4).unwrap();
        let slot = layout.tray(TrayTarget::Language);
        draw_input(
            &mut c,
            &fonts,
            layout,
            Some(InputIndicator {
                top: "ENG",
                bottom: "US",
            }),
            (slot.x() + 2, slot.y() + 2),
            false,
            true,
        );
        for (i, px) in bytes.chunks_exact(4).enumerate() {
            if px != [0, 0, 0, 0] {
                assert!(
                    slot.contains((i % 1280) as i32, (i / 1280) as i32),
                    "painted outside the slot"
                );
            }
        }
        assert!(bytes.iter().any(|b| *b != 0));
    }
}
#[cfg(test)]
mod fullscreen_tests {
    use super::*;
    #[test]
    fn a_full_screen_window_hides_the_bar_without_moving_the_work_area() {
        let normal = Layout::new(1280, 720);
        let under = normal.with_taskbar(UNDER_FULLSCREEN);
        assert!(under.hidden() && !normal.hidden());
        assert_eq!(under.work_area(), normal.work_area());
        // What is transferred stays on the output: the bar or its sliver.
        let output = under.output();
        for l in [
            normal,
            under,
            normal.with_taskbar(door_service_protocol::preferences::taskbar::AUTO_HIDE),
        ] {
            assert_eq!(l.shown_bar().intersection(output), Some(l.shown_bar()));
            assert_eq!(
                l.overlay(false).intersection(output),
                Some(l.overlay(false))
            );
        }
    }
}
