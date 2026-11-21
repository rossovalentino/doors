//! Windows 11 tray flyouts: Quick settings, and the Notification center with
//! the calendar. Real RTC time in the session's time zone and regional format
//! (UTC / en-US until set); devices this profile lacks (network, radios,
//! audio, display brightness, battery) are shown as unavailable, never faked.
extern crate alloc;
use crate::acrylic::Acrylic;
use crate::paint::{self, Palette};
use crate::taskbar::{Layout, TrayTarget};
use alloc::format;
use door_abi::clock::{WallClock, days_in_month};
use door_graphics_core::{Canvas, Rect};
use door_graphics_text::{FontAtlas, TextStyle};
use door_service_protocol::desktop::Controls;
#[path = "tray_pages.rs"]
pub mod pages;
pub use pages::{AccessRow, Battery, VpnEntry};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flyout {
    Calendar,
    Devices,
    /// The input switcher listing this many installed methods.
    Input(u8),
    /// "Show hidden icons": this many Windows programs' notification icons.
    Hidden(u8),
}
#[derive(Default)]
pub struct State {
    /// Local civil time in `zone` (`localize`).
    pub time: Option<WallClock>,
    /// Time zone and regional format ids (`door_locale`); 0 = default
    /// (UTC, English (United States)).
    pub zone: u16,
    pub region: u16,
    /// Regional format overrides (`preferences::REGIONAL_OVERRIDES`, 0 =
    /// none): 24-hour time, first day of week and the four formats.
    pub overrides: u32,
    /// "Show seconds in system tray clock": repaint every second.
    pub seconds: bool,
    pub open: Option<Flyout>,
    pub month: Option<(u16, u8)>,
    pub poll_at: u64,
    pub backdrop: Acrylic,
    /// Installed input methods while the taskbar shows an input indicator
    /// (several methods or an IME); None hides the indicator and its
    /// switcher, as Windows does with one keyboard.
    pub input: Option<door_keyboard_layouts::Preferences>,
    /// Notifications the desktop shell presents in the centre (door.shell).
    pub notices: crate::shell_link::Notices,
    /// Windows programs' notification icons (the hidden-icons flyout).
    pub notify: crate::notify_icons::Icons,
    /// A desktop shell is connected: Clear all and Do not disturb act.
    pub notices_live: bool,
    /// The calendar's Focus session length (minutes; 0 = the default 30).
    pub focus_minutes: u16,
    /// The Quick settings network glyph, polled from door.network
    /// (`network_at`). The service is resolved without blocking
    /// (`network_resolve`), since Init calls this server too; a failed call
    /// drops the connection and it is resolved again later.
    pub network: crate::taskbar::Net,
    pub network_at: u64,
    pub network_link: Option<door_service_client::Network>,
    pub network_resolve: Option<door_service_client::Init>,
    /// door.audio status (None: no service or not yet connected), its
    /// connection and non-blocking resolve, like the network link.
    pub audio: Option<door_service_protocol::audio::Status>,
    pub audio_at: u64,
    pub audio_link: Option<door_service_client::audio::Audio>,
    pub audio_resolve: Option<door_service_client::Init>,
    /// The output pushed to door.audio from the SOUND preference.
    pub audio_pushed: Option<door_service_protocol::audio::Output>,
    /// The microphone policy pushed from the MICROPHONE preference.
    pub capture_pushed: Option<u32>,
    /// The microphone level pushed from the MICROPHONE preference.
    pub input_pushed: Option<door_service_protocol::audio::Input>,
    /// Apps recording now (door.audio's capture records), for the
    /// microphone glyph's tooltip, and when the pointer began resting on it.
    pub capture_names: alloc::vec::Vec<alloc::string::String>,
    pub mic_hover: Option<u64>,
    pub quick_page: QuickPage,
    /// The volume slider is being dragged.
    pub volume_drag: bool,
    /// The tile page shown (0 or 1) and its scroll between pages.
    pub tile_page: u8,
    pub tile_scroll: crate::motion::Tween,
    /// The VISION preference word (night light, colour filters,
    /// brightness) as last applied.
    pub vision: u32,
    /// Magnifier is running.
    pub magnifier: bool,
    /// The brightness slider is being dragged.
    pub brightness_drag: bool,
    /// The Accessibility page's list scroll (logical px).
    pub list_scroll: i32,
    /// Project mode (0 = PC screen only).
    pub projection: u8,
    /// Battery status; None on desktops (no battery is shown).
    pub battery: Option<Battery>,
    /// VPN profiles from door.network.
    pub vpns: alloc::vec::Vec<VpnEntry>,
    /// When the window server next reads them (monotonic ticks).
    pub vpn_at: u64,
    /// The shell clock (µs) at the last sync, for page animations.
    pub now: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// The "microphone in use" glyph: Privacy & security > Microphone.
    Microphone,
    /// The "Show hidden icons" chevron.
    Chevron,
    /// A program's notification icon in the hidden-icons flyout.
    NotifyIcon(usize),
    Clock,
    Devices,
    Language,
    Previous,
    Next,
    Today,
    Settings,
    Accessibility,
    Desktop,
    Dismiss,
    KeepOpen,
    /// Input switcher: make installed method n current.
    InputMethod(usize),
    /// Calendar Focus footer: shorter / longer session.
    FocusShorter,
    FocusLonger,
    /// Input switcher footer: Settings > Time & language > Typing.
    KeyboardSettings,
    /// A notification centre control (carried out by the shell).
    Notice(crate::shell_view::NoticeHit),
    /// Quick settings volume row: the speaker button (mute), the slider
    /// (press and drag; the shell reads the pointer), and the Sound output
    /// page button.
    Mute,
    Volume,
    SoundOutput,
    /// Sound output page: back, the output device, Spatial sound Off and
    /// "More volume settings" (Settings > System > Sound).
    Back,
    OutputDevice,
    SpatialOff,
    MoreVolume,
    /// Tile pages: the page indicator's arrows or the wheel.
    TilesUp,
    TilesDown,
    /// Second-page tiles.
    NightLight,
    Cast,
    Project,
    /// The VPN tile (Settings > VPN without profiles).
    Vpn,
    /// The brightness slider (press and drag).
    Brightness,
    /// Sub-page controls.
    AccessToggle(AccessRow),
    ProjectMode(u8),
    CastHelp,
    VpnConnect(u8),
    VpnSelect(u8),
    /// A sub-page's "More ... settings" footer.
    MoreSettings,
}
/// Which page Quick settings shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QuickPage {
    #[default]
    Main,
    /// 26H2 "Sound output": output devices and spatial sound.
    SoundOutput,
    /// Accessibility, VPN, Cast (Win+K) and Project (Win+P) pages.
    Accessibility,
    Vpn,
    Cast,
    Project,
}
pub const QUICK_HEIGHT: u32 = 386;
/// Scroll between the two tile pages (µs).
pub const TILE_SCROLL_US: u64 = 300_000;
/// Windows 11 26H2 calendar panel (`reports/artifacts/reference-26h2/shell-
/// notifications.uia.tsv`, px / 1.5): 336 x 457 with the Focus footer.
pub const CALENDAR_HEIGHT: u32 = 457;
/// Calendar and notification centre width (504 px at 150 %).
pub const CENTRE_WIDTH: u32 = 336;
/// Focus session lengths (minutes) the footer steps through.
pub const FOCUS_MINUTES: [u16; 10] = [5, 10, 15, 20, 25, 30, 45, 60, 90, 120];
pub const NOTIFICATIONS_HEIGHT: u32 = 116;
/// Quick-settings tiles in Windows order; `None` reason = available.
pub const TILES: [(&str, &str, Option<&str>); 6] = [
    ("Wi-Fi", "No Wi-Fi adapter", Some("No Wi-Fi adapter")),
    ("Bluetooth", "No Bluetooth", Some("No Bluetooth adapter")),
    ("Airplane mode", "Airplane mode", Some("No radios")),
    ("Accessibility", "Accessibility", None),
    ("VPN", "VPN", None),
    ("Energy saver", "Energy saver", Some("No power provider")),
];
/// Growth of one text row of `style` at the applied Accessibility > Text
/// size (0 at 100%). Flyout text bands grow by it; tiles, glyph buttons,
/// slider tracks and the day grid keep their size, as in Windows.
fn grow(style: TextStyle) -> i32 {
    paint::text_grow(0, style) as i32
}
/// Quick settings height: two tile-label bands and two slider captions
/// grow with the text size.
pub fn quick_height() -> u32 {
    QUICK_HEIGHT + 4 * grow(TextStyle::Regular12) as u32
}
/// Calendar height: the date header, month heading and weekday row grow.
pub fn calendar_height() -> u32 {
    CALENDAR_HEIGHT + 2 * grow(TextStyle::Semibold14) as u32 + grow(TextStyle::Regular12) as u32
}
/// Notification center height: its heading and empty-state line grow.
pub fn notifications_height() -> u32 {
    NOTIFICATIONS_HEIGHT + (grow(TextStyle::Semibold14) + grow(TextStyle::Regular14)) as u32
}
pub fn quick_rect(layout: Layout) -> Rect {
    layout.flyout_sized(360, quick_height())
}
pub fn calendar_rect(layout: Layout) -> Rect {
    layout.flyout_sized(CENTRE_WIDTH, calendar_height())
}
pub fn notifications_rect(layout: Layout) -> Option<Rect> {
    let c = calendar_rect(layout);
    let top = layout.work_area().y() + 12;
    let h = notifications_height();
    let y = c.y() - 12 - h as i32;
    (y >= top).then(|| Rect::new(c.x(), y, c.width(), h).unwrap())
}
/// Notification center text rows: (heading, empty-state line).
pub fn notifications_rows(n: Rect) -> (Rect, Rect) {
    let heading = Rect::new(
        n.x() + 17,
        n.y() + 12,
        200,
        paint::text_grow(24, TextStyle::Semibold14),
    )
    .unwrap();
    let line = Rect::new(
        n.x(),
        n.y() + 56 + grow(TextStyle::Semibold14),
        n.width(),
        paint::text_grow(24, TextStyle::Regular14),
    )
    .unwrap();
    (heading, line)
}
/// Input switcher (Windows 11): one row per installed method, then the
/// "More keyboard settings" footer.
pub const INPUT_WIDTH: u32 = 320;
pub const INPUT_ROW: u32 = 56;
pub const INPUT_FOOTER: u32 = 48;
/// A switcher row (language over keyboard) at the applied text size.
pub fn input_row_height() -> u32 {
    INPUT_ROW + (grow(TextStyle::Regular14) + grow(TextStyle::Regular12)) as u32
}
/// Switcher width: its text columns (abbreviation, 36 px; names, 244 px
/// at 100%) widen with the text size, as the Windows flyout sizes to content.
pub fn input_width() -> u32 {
    INPUT_WIDTH + (paint::text_len(244) - 244) + (paint::text_len(36) - 36)
}
pub fn input_footer_height() -> u32 {
    paint::text_grow(INPUT_FOOTER, TextStyle::Regular14)
}
pub fn input_rect(layout: Layout, methods: u8) -> Rect {
    layout.flyout_sized(
        input_width(),
        8 + u32::from(methods) * input_row_height() + 8 + input_footer_height(),
    )
}
pub fn input_row(layout: Layout, methods: u8, index: usize) -> Rect {
    let r = input_rect(layout, methods);
    let h = input_row_height();
    Rect::new(
        r.x() + 4,
        r.y() + 8 + (index as u32 * h) as i32,
        r.width() - 8,
        h,
    )
    .unwrap()
}
pub fn input_footer(layout: Layout, methods: u8) -> Rect {
    let r = input_rect(layout, methods);
    let h = input_footer_height();
    Rect::new(r.x(), r.bottom() - h as i32, r.width(), h).unwrap()
}
/// Text boxes of a switcher row: (abbreviation, language line, keyboard
/// line). The abbreviation column widens to its label at large text sizes.
pub fn input_row_text(fonts: &FontAtlas, row: Rect, abbreviation: &str) -> (Rect, Rect, Rect) {
    let abbr_w = 44.max(paint::measure(fonts, abbreviation, TextStyle::Semibold14) + 8);
    let abbr = Rect::new(row.x() + 8, row.y(), abbr_w, row.height()).unwrap();
    let x = row.x() + 16 + abbr_w as i32;
    let w = row.width().saturating_sub(abbr_w + 24).max(1);
    let g14 = grow(TextStyle::Regular14);
    let language = Rect::new(
        x,
        row.y() + 8,
        w,
        paint::text_grow(22, TextStyle::Regular14),
    )
    .unwrap();
    let keyboard = Rect::new(
        x,
        row.y() + 30 + g14,
        w,
        paint::text_grow(18, TextStyle::Regular12),
    )
    .unwrap();
    (abbr, language, keyboard)
}
/// Second line of a switcher row: "US keyboard", or the IME's name.
pub fn keyboard_label(m: &door_keyboard_layouts::Method) -> alloc::string::String {
    if m.ime == door_keyboard_layouts::Ime::None {
        format!("{} keyboard", m.keyboard)
    } else {
        alloc::string::String::from(m.keyboard)
    }
}
pub fn tile(layout: Layout, index: usize) -> Rect {
    let r = quick_rect(layout);
    Rect::new(
        r.x() + 24 + (index % 3) as i32 * 108,
        r.y() + 22 + (index / 3) as i32 * (96 + grow(TextStyle::Regular12)),
        96,
        48,
    )
    .unwrap()
}
/// The caption band under tile `index` (its label, grown with text size).
pub fn tile_label(layout: Layout, index: usize) -> Rect {
    let t = tile(layout, index);
    Rect::new(
        t.x() - 6,
        t.bottom() + 8,
        t.width() + 12,
        paint::text_grow(18, TextStyle::Regular12),
    )
    .unwrap()
}
/// Vertical centre of slider `row`'s track (brightness, volume).
fn slider_center(r: Rect, row: usize) -> i32 {
    let g = grow(TextStyle::Regular12);
    r.y() + 251 + 2 * g + row as i32 * (52 + g)
}
/// Slider `row`'s control band (its track, as UI Automation exposes it).
pub fn slider_row(layout: Layout, row: usize) -> Rect {
    let r = quick_rect(layout);
    Rect::new(r.x() + 57, slider_center(r, row) - 6, 236, 16).unwrap()
}
/// Slider `row`'s caption band under the track.
pub fn slider_caption(layout: Layout, row: usize) -> Rect {
    let r = quick_rect(layout);
    Rect::new(
        r.x() + 57,
        slider_center(r, row) + 6,
        280,
        paint::text_grow(18, TextStyle::Regular12),
    )
    .unwrap()
}
/// The volume row's speaker button (mute) and Sound output button.
pub fn mute_button(layout: Layout) -> Rect {
    let r = quick_rect(layout);
    let cy = slider_center(r, 1);
    Rect::new(r.x() + 13, cy - 20, 40, 40).unwrap()
}
pub fn output_button(layout: Layout) -> Rect {
    let r = quick_rect(layout);
    let cy = slider_center(r, 1);
    Rect::new(r.x() + 299, cy - 16, 48, 32).unwrap()
}
/// The volume slider's track (logical px): full width = 100.
pub fn volume_track(layout: Layout) -> Rect {
    slider_row(layout, 1)
}
/// The level (0..=100) a pointer at `x` selects on slider `row` (0
/// brightness, 1 volume).
pub fn level_at(layout: Layout, row: usize, x: i32) -> u8 {
    let t = slider_row(layout, row);
    ((x - t.x()).clamp(0, t.width() as i32) * 100 / t.width() as i32) as u8
}
/// The volume a pointer at `x` selects on the track (0..=100).
pub fn volume_at(layout: Layout, x: i32) -> u8 {
    let t = volume_track(layout);
    let v = (x - t.x()).clamp(0, t.width() as i32) * 100 / t.width() as i32;
    v as u8
}
/// The taskbar/quick-settings speaker glyph for an output state: the mute
/// glyph without a device or while muted, then Volume0..3 by level, as
/// Windows 11 draws it.
/// The microphone glyph's tooltip: who is recording.
pub fn microphone_tooltip(names: &[alloc::string::String]) -> alloc::string::String {
    match names {
        [] => alloc::string::String::from("An app is using your microphone"),
        [one] => alloc::format!("{one} is using your microphone"),
        many => alloc::format!("{} apps are using your microphone", many.len()),
    }
}
pub fn volume_glyph(audio: Option<&door_service_protocol::audio::Status>) -> paint::Mask {
    match audio.filter(|a| a.present) {
        None => crate::shell_mask!("mute", 16),
        Some(a) if a.output.muted => crate::shell_mask!("mute", 16),
        Some(a) => match a.output.volume {
            0 => crate::shell_mask!("shell-volume-zero", 16),
            1..=32 => crate::shell_mask!("shell-volume-low", 16),
            33..=65 => crate::shell_mask!("shell-volume-mid", 16),
            _ => crate::shell_mask!("shell-volume-high", 16),
        },
    }
}
/// Sound output page rows (26H2): back button, title, "Output device"
/// heading and device row, "Spatial sound" heading and its Off row, and the
/// "More volume settings" footer link.
pub struct SoundPage {
    pub back: Rect,
    pub title: Rect,
    pub device_heading: Rect,
    pub device: Rect,
    pub spatial_heading: Rect,
    pub spatial: Rect,
    pub more: Rect,
}
pub fn sound_page(layout: Layout) -> SoundPage {
    let r = quick_rect(layout);
    let g = grow(TextStyle::Regular14);
    let row = |y: i32| Rect::new(r.x() + 8, y, r.width() - 16, 40 + g as u32).unwrap();
    let back = Rect::new(r.x() + 16, r.y() + 16, 32, 32).unwrap();
    let title = Rect::new(r.x() + 56, r.y() + 16, r.width() - 72, 32).unwrap();
    let device_heading = Rect::new(r.x() + 24, r.y() + 64, 280, 28 + g as u32).unwrap();
    let device = row(device_heading.bottom() + 4);
    let spatial_heading = Rect::new(r.x() + 24, device.bottom() + 24, 280, 28 + g as u32).unwrap();
    let spatial = row(spatial_heading.bottom() + 4);
    let more = Rect::new(r.x() + 1, r.bottom() - 49, r.width() - 2, 48).unwrap();
    SoundPage {
        back,
        title,
        device_heading,
        device,
        spatial_heading,
        spatial,
        more,
    }
}
/// A point on Quick settings' own background (left padding beside the
/// first tile column, mid-height): no tile, slider or button.
pub fn quick_background_point(layout: Layout) -> (i32, i32) {
    let r = quick_rect(layout);
    (r.x() + 12, r.y() + r.height() as i32 / 2)
}
/// Calendar controls: (date header, month heading row, previous, next).
pub fn calendar_controls(layout: Layout) -> (Rect, Rect, Rect, Rect) {
    calendar_layout(calendar_rect(layout))
}
/// Quick settings tile fills: (enabled, border, disabled), opaque BGRA.
pub fn tile_colors(dark: bool) -> ([u8; 4], [u8; 4], [u8; 4]) {
    if let Some(theme) = paint::contrast() {
        // Contrast themes: Window tiles with WindowText borders.
        let p = Palette::contrast_theme(theme);
        return (p.tile, p.tile_border, p.tile);
    }
    if dark {
        (
            paint::rgb(55, 55, 55),
            paint::rgb(67, 67, 67),
            paint::rgb(46, 46, 46),
        )
    } else {
        (
            paint::rgb(251, 251, 251),
            paint::rgb(222, 222, 222),
            paint::rgb(245, 245, 245),
        )
    }
}
/// An opaque point inside tile `index` (fill only: clear of its border,
/// rounded corners, glyphs and the split divider).
pub fn tile_fill_point(layout: Layout, index: usize) -> (i32, i32) {
    let t = tile(layout, index);
    (t.x() + 8, t.y() + 8)
}
pub fn settings_button(layout: Layout) -> Rect {
    let r = quick_rect(layout);
    Rect::new(r.right() - 49, r.bottom() - 40, 32, 32).unwrap()
}
fn calendar_layout(r: Rect) -> (Rect, Rect, Rect, Rect) {
    // (date header band, month row, previous, next), 26H2 metrics: a 51 px
    // darker date band, the month row from (9, 62), 36 tall (its heading
    // button is 238 wide), Previous/Next 35/34 x 36 at x 253/293 inside it. Text bands grow
    // with the text size; the buttons stay centred.
    let g = grow(TextStyle::Semibold14);
    let header = Rect::new(r.x(), r.y(), r.width(), (51 + g) as u32).unwrap();
    let month = Rect::new(r.x() + 9, r.y() + 62 + g, r.width() - 18, (36 + g) as u32).unwrap();
    let button_y = month.y() + (month.height() as i32 - 36) / 2;
    let prev = Rect::new(r.x() + 253, button_y, 35, 36).unwrap();
    let next = Rect::new(r.x() + 293, button_y, 34, 36).unwrap();
    (header, month, prev, next)
}
/// The weekday-name row above the day grid (text box at y 122).
fn weekday_row(r: Rect) -> (i32, u32) {
    (
        r.y() + 116 + 2 * grow(TextStyle::Semibold14),
        paint::text_grow(28, TextStyle::Regular12),
    )
}
/// Day `index` of the 6x7 grid: 41 px cells at a 47 x 42 pitch from (7,
/// 153). Cells keep their size (the digits fit by width at every text
/// size); the grid moves down with the grown text bands above it.
fn day_cell(r: Rect, index: i32) -> Rect {
    let (top, h) = weekday_row(r);
    Rect::new(
        r.x() + 7 + (index % 7) * 47,
        top + h as i32 + 9 + (index / 7) * 42,
        41,
        41,
    )
    .unwrap()
}
/// The Focus footer below the grid: (decrease, duration label, increase,
/// Focus button), 24 px tall, 37 px above the panel bottom; its 1 px rule
/// lies 51 px above the bottom.
pub fn focus_controls(r: Rect) -> (Rect, Rect, Rect, Rect) {
    let y = r.bottom() - 37;
    (
        Rect::new(r.x() + 17, y, 24, 24).unwrap(),
        Rect::new(r.x() + 49, y, 66, 24).unwrap(),
        Rect::new(r.x() + 123, y, 24, 24).unwrap(),
        Rect::new(r.x() + 247, y, 72, 24).unwrap(),
    )
}
impl State {
    /// An app records: the microphone glyph takes its notification slot.
    pub fn recording(&self) -> bool {
        self.audio.is_some_and(|a| a.capturing > 0)
    }
    /// The Focus session length shown in the footer.
    pub fn focus_minutes(&self) -> u16 {
        if self.focus_minutes == 0 {
            30
        } else {
            self.focus_minutes
        }
    }
}
impl State {
    /// The regional format the clock and calendar show: the session's
    /// region with the user's overrides applied.
    pub fn region_format(&self) -> door_locale::region::Region {
        door_locale::region::region_or_default(u32::from(self.region))
            .customized(door_locale::region::Overrides::from_word(self.overrides))
    }
    /// REGIONAL_OVERRIDES changed: the clock re-reads at once.
    pub fn set_overrides(&mut self, overrides: u32) -> bool {
        let changed = self.overrides != overrides;
        self.overrides = overrides;
        if changed {
            self.time = None;
            self.poll_at = 0;
        }
        changed
    }
}
impl State {
    /// UTC RTC time to local civil time in the session's zone.
    pub fn localize(&self, utc: WallClock) -> Option<WallClock> {
        let zone = door_locale::tz::zone_or_utc(u32::from(self.zone));
        Some(zone.to_local(utc.to_filetime()?)?.time)
    }
    /// SET_TIME_LOCALE: validated ids; the clock re-reads at once.
    pub fn set_locale(&mut self, zone: u32, region: u32) -> bool {
        if !door_locale::tz::valid(zone) || !door_locale::region::valid(region) {
            return false;
        }
        self.zone = zone as u16;
        self.region = region as u16;
        self.time = None;
        self.poll_at = 0;
        true
    }
    pub fn update(&mut self, time: WallClock) -> bool {
        if !time.valid() {
            return false;
        }
        let seconds = self.seconds;
        let changed = self.time.is_none_or(|old| {
            (old.year, old.month, old.day, old.hour, old.minute)
                != (time.year, time.month, time.day, time.hour, time.minute)
                || (seconds && old.second != time.second)
        });
        self.time = Some(time);
        changed
    }
    pub fn bounds(&self, width: u32, height: u32) -> Option<Rect> {
        self.bounds_with_controls(width, height, Controls::DEFAULT)
    }
    /// Union of the open flyout surfaces.
    pub fn bounds_with_controls(
        &self,
        width: u32,
        height: u32,
        controls: Controls,
    ) -> Option<Rect> {
        let layout = Layout::with_controls(width, height, controls);
        match self.open? {
            Flyout::Devices => Some(quick_rect(layout)),
            Flyout::Input(n) => Some(input_rect(layout, n)),
            Flyout::Hidden(n) => Some(crate::notify_icons::flyout(
                layout,
                self.recording(),
                usize::from(n),
            )),
            Flyout::Calendar => {
                let c = calendar_rect(layout);
                Some(
                    crate::shell_view::centre(layout, &self.notices)
                        .map_or(c, |n| paint::union(n.rect, c)),
                )
            }
        }
    }
    pub fn hit(&self, width: u32, height: u32, x: i32, y: i32) -> Option<Action> {
        self.hit_with_controls(width, height, x, y, Controls::DEFAULT)
    }
    pub fn hit_with_controls(
        &self,
        width: u32,
        height: u32,
        x: i32,
        y: i32,
        controls: Controls,
    ) -> Option<Action> {
        let layout = Layout::with_controls(width, height, controls);
        if !self.notify.is_empty()
            && crate::notify_icons::chevron(layout, self.recording()).contains(x, y)
        {
            return Some(Action::Chevron);
        }
        if self.recording() && layout.microphone().contains(x, y) {
            return Some(Action::Microphone);
        }
        // The indicator slot is empty (and inert) without an input indicator.
        if let Some(target) = layout
            .hit_tray(x, y)
            .filter(|t| *t != TrayTarget::Language || self.input.is_some())
        {
            return Some(match target {
                TrayTarget::ShowDesktop => Action::Desktop,
                TrayTarget::Clock => Action::Clock,
                TrayTarget::QuickSettings => Action::Devices,
                TrayTarget::Language => Action::Language,
            });
        }
        let bounds = self.bounds_with_controls(width, height, controls)?;
        if !bounds.contains(x, y) {
            return Some(Action::Dismiss);
        }
        match self.open? {
            Flyout::Calendar => {
                let c = calendar_rect(layout);
                if !c.contains(x, y) {
                    return Some(
                        match crate::shell_view::hit_centre(
                            layout,
                            &self.notices,
                            self.notices_live,
                            x,
                            y,
                        ) {
                            Some(crate::shell_view::NoticeHit::Inside) => Action::KeepOpen,
                            Some(hit) => Action::Notice(hit),
                            None => Action::Dismiss,
                        },
                    );
                }
                let (header, _, prev, next) = calendar_layout(c);
                let (shorter, _, longer, _) = focus_controls(c);
                if shorter.contains(x, y) {
                    return Some(Action::FocusShorter);
                }
                if longer.contains(x, y) {
                    return Some(Action::FocusLonger);
                }
                if prev.contains(x, y) {
                    return Some(Action::Previous);
                }
                if next.contains(x, y) {
                    return Some(Action::Next);
                }
                if header.contains(x, y) {
                    return Some(Action::Today);
                }
            }
            Flyout::Hidden(n) => {
                let f = crate::notify_icons::flyout(layout, self.recording(), usize::from(n));
                if let Some(i) = crate::notify_icons::hit(f, usize::from(n), x, y) {
                    return Some(Action::NotifyIcon(i));
                }
            }
            Flyout::Input(n) => {
                if let Some(i) =
                    (0..usize::from(n)).find(|i| input_row(layout, n, *i).contains(x, y))
                {
                    return Some(Action::InputMethod(i));
                }
                if input_footer(layout, n).contains(x, y) {
                    return Some(Action::KeyboardSettings);
                }
            }
            Flyout::Devices if self.quick_page == QuickPage::SoundOutput => {
                let p = sound_page(layout);
                for (rect, action) in [
                    (p.back, Action::Back),
                    (p.device, Action::OutputDevice),
                    (p.spatial, Action::SpatialOff),
                    (p.more, Action::MoreVolume),
                ] {
                    if rect.contains(x, y) {
                        return Some(action);
                    }
                }
            }
            Flyout::Devices if self.quick_page != QuickPage::Main => {
                if let Some(action) = pages::hit(self, layout, x, y) {
                    return Some(action);
                }
            }
            Flyout::Devices => {
                if settings_button(layout).contains(x, y) {
                    return Some(Action::Settings);
                }
                let scrolling = self.tile_scroll.animating(self.now);
                if self.tile_page == 1 && pages::page_up(layout).contains(x, y) {
                    return Some(Action::TilesUp);
                }
                if self.tile_page == 0 && pages::page_down(layout).contains(x, y) {
                    return Some(Action::TilesDown);
                }
                if !scrolling && pages::tiles_band(layout).contains(x, y) {
                    let index = (0..6).find(|i| tile(layout, *i).contains(x, y));
                    let action = match (self.tile_page, index) {
                        (0, Some(3)) => Some(Action::Accessibility),
                        (0, Some(4)) => Some(Action::Vpn),
                        (1, Some(1)) => Some(Action::NightLight),
                        (1, Some(4)) => Some(Action::Cast),
                        (1, Some(5)) => Some(Action::Project),
                        _ => None,
                    };
                    if action.is_some() {
                        return action;
                    }
                }
                let bright = slider_row(layout, 0);
                let band =
                    Rect::new(bright.x() - 8, bright.y() - 8, bright.width() + 16, 32).unwrap();
                if band.contains(x, y) {
                    return Some(Action::Brightness);
                }
                if self.audio.is_some_and(|a| a.present) {
                    let track = volume_track(layout);
                    let band =
                        Rect::new(track.x() - 8, track.y() - 8, track.width() + 16, 32).unwrap();
                    if mute_button(layout).contains(x, y) {
                        return Some(Action::Mute);
                    }
                    if band.contains(x, y) {
                        return Some(Action::Volume);
                    }
                    if output_button(layout).contains(x, y) {
                        return Some(Action::SoundOutput);
                    }
                }
            }
        }
        Some(Action::KeepOpen)
    }
    pub fn activate(&mut self, action: Action) {
        match action {
            Action::KeepOpen | Action::Notice(_) | Action::Microphone => (),
            Action::Chevron => {
                let hidden = Flyout::Hidden(self.notify.len().min(u8::MAX as usize) as u8);
                self.open = if matches!(self.open, Some(Flyout::Hidden(_))) {
                    None
                } else {
                    Some(hidden)
                };
            }
            // The desktop sends the click to the icon's owner and closes
            // the flyout.
            Action::NotifyIcon(_) => self.open = None,
            Action::Language => {
                let input = self.input.map(|p| Flyout::Input(p.count() as u8));
                self.open = if self.open.is_some() && self.open == input {
                    None
                } else {
                    input
                };
            }
            Action::Clock => {
                self.open = if self.open == Some(Flyout::Calendar) {
                    None
                } else {
                    Some(Flyout::Calendar)
                };
                self.month = None;
            }
            Action::Devices => {
                self.open = if self.open == Some(Flyout::Devices) {
                    None
                } else {
                    Some(Flyout::Devices)
                };
                self.quick_page = QuickPage::Main;
            }
            // The shell carries out the volume and device actions; these
            // only move between Quick settings pages.
            Action::SoundOutput => self.quick_page = QuickPage::SoundOutput,
            Action::Accessibility => {
                self.quick_page = QuickPage::Accessibility;
                self.list_scroll = 0;
            }
            Action::Cast => self.quick_page = QuickPage::Cast,
            Action::Project => self.quick_page = QuickPage::Project,
            Action::Back => self.quick_page = QuickPage::Main,
            Action::TilesUp | Action::TilesDown => {
                let page = u8::from(action == Action::TilesDown);
                if page != self.tile_page {
                    self.tile_page = page;
                    self.tile_scroll.set(
                        u32::from(page) * 1000,
                        self.now,
                        TILE_SCROLL_US,
                        crate::motion::Curve::Decelerate,
                    );
                }
            }
            Action::Mute
            | Action::Volume
            | Action::OutputDevice
            | Action::SpatialOff
            | Action::NightLight
            | Action::Vpn
            | Action::Brightness
            | Action::AccessToggle(_)
            | Action::CastHelp
            | Action::VpnConnect(_)
            | Action::VpnSelect(_) => (),
            Action::ProjectMode(mode) => {
                // One display: every mode shows the PC screen, so Windows
                // keeps (and reopens with) "PC screen only".
                let _ = mode;
                self.projection = 0;
                self.open = None;
            }
            Action::Today => self.month = None,
            Action::FocusShorter | Action::FocusLonger => {
                let now = self.focus_minutes();
                let i = FOCUS_MINUTES.iter().position(|m| *m == now).unwrap_or(5);
                let i = if action == Action::FocusShorter {
                    i.saturating_sub(1)
                } else {
                    (i + 1).min(FOCUS_MINUTES.len() - 1)
                };
                self.focus_minutes = FOCUS_MINUTES[i];
            }
            Action::Previous | Action::Next => {
                if let Some((year, month)) =
                    self.month.or(self.time.map(|time| (time.year, time.month)))
                {
                    let index = i32::from(year) * 12 + i32::from(month) - 1
                        + if action == Action::Previous { -1 } else { 1 };
                    if (1601 * 12..10000 * 12).contains(&index) {
                        self.month = Some(((index / 12) as u16, (index % 12 + 1) as u8));
                    }
                }
            }
            _ => self.open = None,
        }
    }
}
/// Legacy glyph helper kept for other painters (login, Task View).
pub fn glyph(canvas: &mut Canvas<'_>, x: i32, y: i32, size: usize, mask: &[u8], color: [u8; 4]) {
    // `mask` is the native mask of the scale's asset density; fractional
    // scales area-filter it to the physical size.
    let scale = canvas.scale();
    let source = size * scale.asset_factor() as usize;
    if mask.len() != source * source {
        return;
    }
    let physical = scale.len(size as u32) as usize;
    let ox = scale.px(x);
    let oy = scale.px(y);
    for i in 0..physical * physical {
        let a = door_graphics_core::area_sample(mask, source, physical, i % physical, i / physical);
        if a != 0 {
            let _ = canvas.pixel(
                ox + (i % physical) as i32,
                oy + (i / physical) as i32,
                paint::alpha(color, a),
            );
        }
    }
}
/// Frame inputs shared by both flyouts.
pub struct Frame {
    pub dark: bool,
    pub pointer: (i32, i32),
    pub slide: i32,
    pub transparency: bool,
}
fn surface(c: &mut Canvas<'_>, r: Rect, palette: &Palette, backdrop: &Acrylic, frame: &Frame) {
    paint::shadow(c, r, 8, palette.dark);
    paint::rounded(c, r, 8, palette.surface_border);
    let inner = paint::inset(r, 1, 1).unwrap_or(r);
    // "Show accent colour on Start and taskbar" also tints the flyouts.
    let accent = paint::start_tint(palette.dark);
    let tint = if palette.dark {
        paint::rgb(44, 44, 44)
    } else {
        paint::rgb(243, 243, 243)
    };
    if frame.transparency && backdrop.region().is_some() {
        backdrop.paint(c, inner, 7, frame.slide, accent.unwrap_or(tint), 232);
    } else {
        paint::rounded(c, inner, 7, accent.unwrap_or(palette.surface));
    }
}
fn shifted(r: Rect, dy: i32) -> Rect {
    Rect::new(r.x(), r.y() + dy, r.width(), r.height()).unwrap()
}
/// The darker top band of a flyout surface `r` (the calendar's date band:
/// #222222 over the #292929 panel in the 26H2 reference), inside its border
/// and rounded corners.
fn top_band(c: &mut Canvas<'_>, r: Rect, height: u32, color: [u8; 4]) {
    let Ok(band) = Rect::new(r.x(), r.y(), r.width(), height) else {
        return;
    };
    let saved = c.clip_rect();
    if let Some(clip) = saved.and_then(|s| s.intersection(paint::physical(band, c.scale()))) {
        c.set_clip(clip);
        paint::rounded(c, paint::inset(r, 1, 1).unwrap_or(r), 7, color);
    }
    if let Some(saved) = saved {
        c.set_clip(saved);
    }
}
/// Paint `kind` (which may be closing while `state.open` is already None).
pub fn draw_flyout(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    state: &State,
    kind: Flyout,
    layout: Layout,
    frame: &Frame,
) {
    let palette = Palette::new(frame.dark);
    let pointer = (frame.pointer.0, frame.pointer.1 - frame.slide);
    match kind {
        Flyout::Devices => draw_quick(c, fonts, state, layout, &palette, pointer, frame),
        Flyout::Calendar => draw_calendar(c, fonts, state, layout, &palette, pointer, frame),
        Flyout::Input(n) => draw_input(c, fonts, state, layout, &palette, pointer, frame, n),
        Flyout::Hidden(n) => draw_hidden(c, state, layout, &palette, pointer, frame, n),
    }
}
/// Surfaces of a flyout kind (calendar includes the notification center).
pub fn rects(kind: Flyout, layout: Layout) -> alloc::vec::Vec<Rect> {
    match kind {
        Flyout::Devices => alloc::vec![quick_rect(layout)],
        Flyout::Input(n) => alloc::vec![input_rect(layout, n)],
        Flyout::Hidden(n) => {
            alloc::vec![crate::notify_icons::flyout(layout, false, usize::from(n))]
        }
        Flyout::Calendar => notifications_rect(layout)
            .into_iter()
            .chain(core::iter::once(calendar_rect(layout)))
            .collect(),
    }
}
impl State {
    /// `rects` with the notification centre sized for its notifications.
    pub fn surfaces(&self, kind: Flyout, layout: Layout) -> alloc::vec::Vec<Rect> {
        match kind {
            Flyout::Calendar => crate::shell_view::centre(layout, &self.notices)
                .map(|k| k.rect)
                .into_iter()
                .chain(core::iter::once(calendar_rect(layout)))
                .collect(),
            Flyout::Hidden(n) => alloc::vec![crate::notify_icons::flyout(
                layout,
                self.recording(),
                usize::from(n),
            )],
            other => rects(other, layout),
        }
    }
}
/// "Show hidden icons": the programs' icons in a grid of 40 px cells,
/// the one under the pointer on a subtle plate.
fn draw_hidden(
    c: &mut Canvas<'_>,
    state: &State,
    layout: Layout,
    palette: &Palette,
    pointer: (i32, i32),
    frame: &Frame,
    n: u8,
) {
    use crate::notify_icons as ni;
    let count = usize::from(n).min(state.notify.len());
    let hit = ni::flyout(layout, state.recording(), count);
    let r = shifted(hit, frame.slide);
    surface(c, r, palette, &state.backdrop, frame);
    for (i, icon) in state.notify.icons.iter().take(count).enumerate() {
        let cell = shifted(ni::cell(hit, i), frame.slide);
        if ni::cell(hit, i).contains(pointer.0, pointer.1) {
            paint::rounded(
                c,
                paint::inset(cell, 2, 2).unwrap_or(cell),
                4,
                palette.subtle_hover,
            );
        }
        if icon.bgra.len() != door_service_protocol::window::notify::ICON_BYTES {
            continue;
        }
        let at = Rect::new(
            cell.x() + (cell.width() - ni::ICON) as i32 / 2,
            cell.y() + (cell.height() - ni::ICON) as i32 / 2,
            ni::ICON,
            ni::ICON,
        )
        .unwrap();
        let physical = c.scale().len(ni::ICON) as usize;
        let edge = door_service_protocol::window::notify::EDGE as usize;
        let pixels = if physical == edge {
            icon.bgra.clone()
        } else {
            door_graphics_core::area_resample(&icon.bgra, 4, (edge, edge), (physical, physical))
        };
        let _ = c.blit_bgra(at, &pixels);
    }
}
/// The input switcher: installed methods (current one selected, as the
/// Windows 11 list pill shows it) and "More keyboard settings".
#[allow(clippy::too_many_arguments)]
fn draw_input(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    state: &State,
    layout: Layout,
    palette: &Palette,
    pointer: (i32, i32),
    frame: &Frame,
    n: u8,
) {
    let dy = frame.slide;
    let r = shifted(input_rect(layout, n), dy);
    surface(c, r, palette, &state.backdrop, frame);
    let Some(prefs) = state.input else { return };
    let current = prefs.current_index();
    for (i, m) in prefs.methods().enumerate().take(usize::from(n)) {
        let hit = input_row(layout, n, i);
        let row = shifted(hit, dy);
        let selected = i == current;
        let hot = hit.contains(pointer.0, pointer.1);
        // Contrast themes: Highlight/HighlightText for the selection.
        let (fill, fg, fg2) = if selected && palette.contrast {
            (
                Some(palette.accent),
                palette.accent_text,
                palette.accent_text,
            )
        } else if selected || hot {
            (
                Some(if selected && hot {
                    palette.subtle_pressed
                } else {
                    palette.subtle_hover
                }),
                palette.text,
                palette.text_secondary,
            )
        } else {
            (None, palette.text, palette.text_secondary)
        };
        if let Some(fill) = fill {
            paint::rounded(c, paint::inset(row, 0, 2).unwrap_or(row), 4, fill);
        }
        if selected {
            let pill = Rect::new(row.x(), row.y() + (row.height() as i32 - 16) / 2, 3, 16).unwrap();
            paint::rounded(
                c,
                pill,
                1,
                if palette.contrast {
                    palette.accent_text
                } else {
                    palette.accent
                },
            );
        }
        let (abbreviation, language, keyboard) = input_row_text(fonts, row, m.abbreviation);
        paint::text_in(
            c,
            fonts,
            abbreviation,
            m.abbreviation,
            TextStyle::Semibold14,
            fg,
        );
        // Names fit the widened flyout; where the work area clamps it they
        // end in an ellipsis rather than being cut.
        let name = paint::ellipsize(fonts, m.language, TextStyle::Regular14, language.width());
        let board = paint::ellipsize(
            fonts,
            &keyboard_label(m),
            TextStyle::Regular12,
            keyboard.width(),
        );
        paint::text_row(
            c,
            fonts,
            language.x(),
            language.y(),
            language.height(),
            &name,
            TextStyle::Regular14,
            fg,
            language.width(),
        );
        paint::text_row(
            c,
            fonts,
            keyboard.x(),
            keyboard.y(),
            keyboard.height(),
            &board,
            TextStyle::Regular12,
            fg2,
            keyboard.width(),
        );
    }
    let footer_hit = input_footer(layout, n);
    let footer = shifted(footer_hit, dy);
    paint::rect(
        c,
        footer.x() + 1,
        footer.y(),
        footer.width() - 2,
        1,
        palette.surface_divider,
    );
    if footer_hit.contains(pointer.0, pointer.1) {
        paint::rounded(
            c,
            paint::inset(footer, 4, 6).unwrap_or(footer),
            4,
            palette.subtle_hover,
        );
    }
    paint::text_row(
        c,
        fonts,
        footer.x() + 16,
        footer.y(),
        footer.height(),
        "More keyboard settings",
        TextStyle::Regular14,
        palette.accent,
        footer.width() - 32,
    );
}
/// WinUI slider metrics: a 4 px track, accent fill up to the value, and a
/// 20 px thumb (outer ring in the surface colour, accent centre that grows
/// from 12 to 14 px under the pointer and shrinks to 10 px while dragged).
fn draw_volume_row(
    c: &mut Canvas<'_>,
    state: &State,
    layout: Layout,
    palette: &Palette,
    pointer: (i32, i32),
    dy: i32,
    a: door_service_protocol::audio::Status,
) {
    let track0 = volume_track(layout);
    let track = shifted(track0, dy);
    let cy = track.y() + track.height() as i32 / 2;
    let mute0 = mute_button(layout);
    if mute0.contains(pointer.0, pointer.1) {
        paint::rounded(c, shifted(mute0, dy), 4, palette.subtle_hover);
    }
    paint::glyph_in(
        c,
        shifted(mute0, dy),
        16,
        volume_glyph(Some(&a)),
        palette.text,
    );
    let rail = if palette.dark {
        paint::rgb(158, 158, 158)
    } else {
        paint::rgb(134, 134, 134)
    };
    let fill = if a.output.muted {
        palette.text_disabled
    } else {
        palette.accent
    };
    let w = track.width() as i32 * i32::from(a.output.volume) / 100;
    paint::rounded(
        c,
        Rect::new(track.x(), cy - 2, track.width(), 4).unwrap(),
        2,
        rail,
    );
    if w > 0 {
        paint::rounded(
            c,
            Rect::new(track.x(), cy - 2, w as u32, 4).unwrap(),
            2,
            fill,
        );
    }
    let thumb = Rect::new(track.x() + w - 10, cy - 10, 20, 20).unwrap();
    let hovered = shifted(thumb, -dy).contains(pointer.0, pointer.1);
    let ring = if palette.dark {
        paint::rgb(69, 69, 69)
    } else {
        paint::rgb(255, 255, 255)
    };
    paint::framed(c, thumb, 10, palette.surface_border, ring);
    let inner = if state.volume_drag {
        10
    } else if hovered {
        14
    } else {
        12
    };
    let o = (20 - inner) / 2;
    paint::rounded(
        c,
        Rect::new(thumb.x() + o, thumb.y() + o, inner as u32, inner as u32).unwrap(),
        inner as u32 / 2,
        fill,
    );
    let out0 = output_button(layout);
    if out0.contains(pointer.0, pointer.1) {
        paint::rounded(c, shifted(out0, dy), 4, palette.subtle_hover);
    }
    let out = shifted(out0, dy);
    paint::glyph(
        c,
        out.x() + 6,
        out.y() + 8,
        16,
        crate::shell_mask!("shell-sound-output", 16),
        palette.text,
    );
    paint::glyph(
        c,
        out.x() + 26,
        out.y() + 8,
        16,
        crate::shell_mask!("next", 16),
        palette.text,
    );
}
/// The 26H2 "Sound output" page of Quick settings.
fn draw_sound_page(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    state: &State,
    layout: Layout,
    palette: &Palette,
    pointer: (i32, i32),
    dy: i32,
) {
    let p = sound_page(layout);
    let hover = |r: Rect| r.contains(pointer.0, pointer.1);
    if hover(p.back) {
        paint::rounded(c, shifted(p.back, dy), 4, palette.subtle_hover);
    }
    paint::glyph_in(
        c,
        shifted(p.back, dy),
        16,
        crate::shell_mask!("shell-back", 16),
        palette.text,
    );
    let title = shifted(p.title, dy);
    paint::text_row(
        c,
        fonts,
        title.x(),
        title.y(),
        title.height(),
        "Sound output",
        TextStyle::Regular14,
        palette.text,
        title.width(),
    );
    for (heading, row, glyph, label, enabled) in [
        (
            p.device_heading,
            p.device,
            Some(crate::shell_mask!("shell-speakers", 16)),
            "Speakers (VirtIO Sound)",
            state.audio.is_some_and(|a| a.present),
        ),
        (p.spatial_heading, p.spatial, None, "Off", true),
    ] {
        let h = shifted(heading, dy);
        paint::text_row(
            c,
            fonts,
            h.x(),
            h.y(),
            h.height(),
            if heading == p.device_heading {
                "Output device"
            } else {
                "Spatial sound"
            },
            TextStyle::Semibold14,
            palette.text,
            h.width(),
        );
        let r = shifted(row, dy);
        // The selected row: a subtle plate and the accent pill at its left.
        paint::rounded(
            c,
            r,
            4,
            if hover(row) {
                palette.subtle_hover
            } else {
                palette.subtle_pressed
            },
        );
        paint::rounded(
            c,
            Rect::new(r.x(), r.y() + r.height() as i32 / 2 - 8, 3, 16).unwrap(),
            2,
            palette.accent,
        );
        let mut x = r.x() + 16;
        if let Some(mask) = glyph {
            paint::glyph(
                c,
                x,
                r.y() + (r.height() as i32 - 16) / 2,
                16,
                mask,
                palette.text,
            );
            x += 28;
        }
        paint::text_row(
            c,
            fonts,
            x,
            r.y(),
            r.height(),
            label,
            TextStyle::Regular14,
            if enabled {
                palette.text
            } else {
                palette.text_disabled
            },
            (r.right() - x - 8).max(0) as u32,
        );
    }
    let more = shifted(p.more, dy);
    paint::rect(
        c,
        more.x(),
        more.y(),
        more.width(),
        1,
        palette.surface_divider,
    );
    if hover(p.more) {
        paint::rect(
            c,
            more.x(),
            more.y() + 1,
            more.width(),
            more.height() - 1,
            palette.subtle_hover,
        );
    }
    paint::text_row(
        c,
        fonts,
        more.x() + 23,
        more.y(),
        more.height(),
        "More volume settings",
        TextStyle::Regular14,
        palette.text,
        more.width() - 46,
    );
}
fn draw_quick(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    state: &State,
    layout: Layout,
    palette: &Palette,
    pointer: (i32, i32),
    frame: &Frame,
) {
    let dy = frame.slide;
    let r0 = quick_rect(layout);
    let r = shifted(r0, dy);
    surface(c, r, palette, &state.backdrop, frame);
    match state.quick_page {
        QuickPage::Main => (),
        QuickPage::SoundOutput => {
            draw_sound_page(c, fonts, state, layout, palette, pointer, dy);
            return;
        }
        _ => {
            pages::draw(c, fonts, state, layout, palette, pointer, dy);
            return;
        }
    }
    // Page indicator at the right edge (26H2): two dots, the current one
    // brighter, with the up arrow above them on the second page and the
    // down arrow below them on the first.
    let px = r.right() - 13;
    let (first, second) = if state.tile_page == 0 {
        (palette.text, palette.text_secondary)
    } else {
        (palette.text_secondary, palette.text)
    };
    paint::circle(c, px, r.y() + 100, 2, first);
    paint::circle(c, px, r.y() + 111, 2, second);
    let arrow = |c: &mut Canvas<'_>, rect: Rect, up: bool| {
        let hovered = rect.contains(pointer.0, pointer.1);
        let color = if hovered {
            palette.text
        } else {
            palette.text_secondary
        };
        let a = shifted(rect, dy);
        for row in 0..4 {
            let y = if up { a.y() + 4 + row } else { a.y() + 5 + row };
            let w = if up { 2 * row + 2 } else { 8 - 2 * row };
            paint::rect(c, px - w / 2, y, w as u32, 1, color);
        }
    };
    if state.tile_page == 0 {
        arrow(c, pages::page_down(layout), false);
    } else {
        arrow(c, pages::page_up(layout), true);
    }
    // Tiles: both pages while scrolling between them, clipped to the band.
    let band = shifted(pages::tiles_band(layout), dy);
    let saved = c.clip_rect();
    let scale = c.scale();
    if let Some(clip) = saved.and_then(|cl| cl.intersection(paint::physical(band, scale))) {
        c.set_clip(clip);
    }
    let travel = pages::tile_page_height();
    let position = state.tile_scroll.value(state.now) as i32 * travel / 1000;
    for page in 0..2 {
        let offset = page * travel - position;
        if offset.abs() < travel {
            draw_tiles(
                c,
                fonts,
                state,
                layout,
                palette,
                pointer,
                dy + offset,
                page as u8,
            );
        }
    }
    if let Some(saved) = saved {
        c.set_clip(saved);
    }
    // Divider, then the brightness and volume rows.
    paint::rect(
        c,
        r.x() + 1,
        r.y() + 213 + 2 * grow(TextStyle::Regular12),
        r.width() - 2,
        1,
        palette.surface_border,
    );
    let track = if palette.contrast {
        palette.text_disabled
    } else if palette.dark {
        paint::rgb(80, 80, 80)
    } else {
        paint::rgb(200, 200, 200)
    };
    // Brightness: software brightness at scanout (no backlight here).
    let bright0 = slider_row(layout, 0);
    let bright = shifted(bright0, dy);
    let cy = bright.y() + bright.height() as i32 / 2;
    paint::glyph(
        c,
        r.x() + 25,
        cy - 8,
        16,
        crate::shell_mask!("shell-brightness", 16),
        palette.text,
    );
    let level = door_service_protocol::preferences::vision::brightness(state.vision) as u8;
    let thumb_x = bright0.x() + bright0.width() as i32 * i32::from(level) / 100;
    let hovered = (pointer.0 - thumb_x).abs() <= 10 && (pointer.1 - (cy - dy)).abs() <= 10;
    pages::draw_slider(
        c,
        palette,
        bright,
        level,
        palette.accent,
        hovered,
        state.brightness_drag,
    );
    let audio = state.audio.filter(|a| a.present);
    if let Some(a) = audio {
        draw_volume_row(c, state, layout, palette, pointer, dy, a);
    } else {
        let cy = slider_center(r, 1);
        paint::glyph(
            c,
            r.x() + 25,
            cy - 8,
            16,
            crate::shell_mask!("mute", 16),
            palette.text_disabled,
        );
        paint::rounded(c, Rect::new(r.x() + 57, cy - 2, 236, 4).unwrap(), 2, track);
        let caption = shifted(slider_caption(layout, 1), dy);
        let reason = paint::ellipsize(
            fonts,
            "No audio output device",
            TextStyle::Regular12,
            caption.width(),
        );
        paint::text_row(
            c,
            fonts,
            caption.x(),
            caption.y(),
            caption.height(),
            &reason,
            TextStyle::Regular12,
            palette.text_secondary,
            caption.width(),
        );
    }
    // Footer band: the battery (laptops) and the Settings gear.
    let footer = Rect::new(r.x() + 1, r.bottom() - 49, r.width() - 2, 48).unwrap();
    let saved = c.clip_rect();
    let scale = c.scale();
    if let Some(clip) = saved.and_then(|cl| cl.intersection(paint::physical(footer, scale))) {
        c.set_clip(clip);
        let band = if palette.contrast {
            paint::black(0)
        } else if palette.dark {
            paint::black(46)
        } else {
            paint::black(9)
        };
        paint::rounded(c, paint::inset(r, 1, 1).unwrap_or(r), 7, band);
    }
    if let Some(saved) = saved {
        c.set_clip(saved);
    }
    paint::rect(
        c,
        r.x() + 1,
        footer.y(),
        r.width() - 2,
        1,
        palette.surface_divider,
    );
    let gear0 = settings_button(layout);
    let gear = shifted(gear0, dy);
    if gear0.contains(pointer.0, pointer.1) {
        paint::rounded(c, gear, 4, palette.subtle_hover);
    }
    paint::glyph_in(
        c,
        gear,
        16,
        crate::shell_mask!("shell-settings", 16),
        palette.text,
    );
    if let Some(b) = state.battery {
        let y = footer.y() + 16;
        pages::draw_battery(c, r.x() + 20, y, b, palette);
        paint::text_row(
            c,
            fonts,
            r.x() + 42,
            footer.y(),
            48,
            &format!("{}%", b.percent),
            TextStyle::Regular14,
            palette.text,
            80,
        );
    }
}
/// One tile page (0: Wi-Fi .. Energy saver; 1: Live captions .. Project)
/// drawn `dy` below its resting place.
#[allow(clippy::too_many_arguments)]
fn draw_tiles(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    state: &State,
    layout: Layout,
    palette: &Palette,
    pointer: (i32, i32),
    dy: i32,
    page: u8,
) {
    let first = [
        crate::shell_mask!("shell-wifi", 16),
        crate::shell_mask!("shell-bluetooth", 16),
        crate::shell_mask!("shell-airplane", 16),
        crate::shell_mask!("shell-accessibility", 16),
        crate::shell_mask!("shell-vpn", 16),
        crate::shell_mask!("shell-energy", 16),
    ];
    let second = pages::second_masks();
    let resting = !state.tile_scroll.animating(state.now) && page == state.tile_page;
    let (tile_off, tile_border, disabled_fill) = tile_colors(palette.dark);
    for i in 0..6 {
        let (label, reason, mask) = if page == 0 {
            (TILES[i].1, TILES[i].2, first[i])
        } else {
            (
                pages::TILES_SECOND[i].0,
                pages::TILES_SECOND[i].1,
                second[i],
            )
        };
        let on = page == 1
            && i == 1
            && state.vision & door_service_protocol::preferences::vision::NIGHT_LIGHT != 0;
        let t0 = tile(layout, i);
        let t = shifted(t0, dy);
        let enabled = reason.is_none();
        let hovered = enabled && resting && t0.contains(pointer.0, pointer.1);
        let (fill, border, fg) = if on {
            (palette.accent, palette.accent, palette.accent_text)
        } else if enabled {
            (tile_off, tile_border, palette.text)
        } else {
            (disabled_fill, tile_border, palette.text_disabled)
        };
        paint::framed(c, t, 4, border, fill);
        if hovered && let Some(inner) = paint::inset(t, 1, 1) {
            paint::rounded(c, inner, 3, palette.subtle_hover);
        }
        // Wi-Fi and Bluetooth are split buttons (toggle | details);
        // Accessibility, VPN, Cast and Project open a page (chevron).
        let split = page == 0 && i < 2;
        let chevron = if page == 0 {
            i == 3 || i == 4
        } else {
            i == 4 || i == 5
        };
        if split {
            paint::rect(c, t.x() + 48, t.y() + 1, 1, t.height() - 2, border);
            paint::glyph(c, t.x() + 16, t.y() + 16, 16, mask, fg);
            paint::glyph(
                c,
                t.x() + 48 + 16,
                t.y() + 16,
                16,
                crate::shell_mask!("next", 16),
                fg,
            );
        } else if chevron {
            paint::glyph(c, t.x() + 30, t.y() + 16, 16, mask, fg);
            paint::glyph(
                c,
                t.x() + 52,
                t.y() + 16,
                16,
                crate::shell_mask!("next", 16),
                fg,
            );
        } else {
            paint::glyph_in(c, t, 16, mask, fg);
        }
        // A caption wider than its band (large text sizes) ends in an
        // ellipsis rather than being cut, as Windows trims tile labels.
        let band = shifted(tile_label(layout, i), dy);
        let label = paint::ellipsize(fonts, label, TextStyle::Regular12, band.width());
        paint::text_span(
            c,
            fonts,
            band.x(),
            band.width(),
            band.y(),
            band.height(),
            &label,
            TextStyle::Regular12,
            if enabled {
                palette.text
            } else {
                palette.text_secondary
            },
        );
    }
}
fn draw_calendar(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    state: &State,
    layout: Layout,
    palette: &Palette,
    pointer: (i32, i32),
    frame: &Frame,
) {
    let dy = frame.slide;
    if let Some(k) = crate::shell_view::centre(layout, &state.notices)
        && !state.notices.list.is_empty()
    {
        let n = shifted(k.rect, dy);
        surface(c, n, palette, &state.backdrop, frame);
        let heading = shifted(k.heading, dy);
        paint::text_row(
            c,
            fonts,
            heading.x(),
            heading.y(),
            heading.height(),
            "Notifications",
            TextStyle::Semibold14,
            palette.text,
            heading.width(),
        );
        crate::shell_view::draw_centre_content(
            c,
            fonts,
            &k,
            &state.notices,
            state.notices_live,
            palette,
            pointer,
            dy,
        );
    } else if let Some(n0) = notifications_rect(layout) {
        let n = shifted(n0, dy);
        surface(c, n, palette, &state.backdrop, frame);
        if state.notices_live
            && let Some(k) = crate::shell_view::centre(layout, &state.notices)
        {
            crate::shell_view::draw_centre_content(
                c,
                fonts,
                &k,
                &state.notices,
                true,
                palette,
                pointer,
                dy,
            );
        }
        let (heading, line) = notifications_rows(n);
        paint::text_row(
            c,
            fonts,
            heading.x(),
            heading.y(),
            heading.height(),
            "Notifications",
            TextStyle::Semibold14,
            palette.text,
            heading.width(),
        );
        paint::text_span(
            c,
            fonts,
            line.x(),
            line.width(),
            line.y(),
            line.height(),
            "No new notifications",
            TextStyle::Regular14,
            palette.text_secondary,
        );
    }
    let r0 = calendar_rect(layout);
    let r = shifted(r0, dy);
    surface(c, r, palette, &state.backdrop, frame);
    let Some(time) = state.time else {
        paint::text_row(
            c,
            fonts,
            r.x() + 16,
            r.y() + 12,
            paint::text_grow(24, TextStyle::Semibold14),
            "Clock unavailable",
            TextStyle::Semibold14,
            palette.text,
            r.width() - 32,
        );
        return;
    };
    let (header0, month_row0, prev0, next0) = calendar_layout(r0);
    let header = shifted(header0, dy);
    if !palette.contrast {
        top_band(c, r, header.height(), palette.surface_header);
    }
    let region = state.region_format();
    let mut title = alloc::string::String::new();
    let _ = region.format(region.month_day, &time, &mut title);
    if header0.contains(pointer.0, pointer.1) && state.month.is_some() {
        paint::rounded(
            c,
            paint::inset(header, 4, 4).unwrap_or(header),
            4,
            palette.subtle_hover,
        );
    }
    // The default zone is the RTC's own UTC: say so, since no zone was chosen.
    let utc = door_locale::tz::zone_or_utc(u32::from(state.zone)).iana == "Etc/UTC";
    let utc_w = if utc {
        paint::measure(fonts, "UTC", TextStyle::Regular12) + 8
    } else {
        0
    };
    let title_w = header.width().saturating_sub(32 + utc_w).max(1);
    paint::text_row(
        c,
        fonts,
        header.x() + 17,
        header.y(),
        header.height(),
        &title,
        TextStyle::Semibold14,
        palette.text,
        title_w,
    );
    if utc {
        paint::text_right(
            c,
            fonts,
            header.right() - 16,
            header.y(),
            header.height(),
            "UTC",
            TextStyle::Regular12,
            palette.text_secondary,
        );
    }
    paint::rect(
        c,
        r.x() + 1,
        header.bottom(),
        r.width() - 2,
        1,
        palette.surface_divider,
    );
    let (year, month) = state.month.unwrap_or((time.year, time.month));
    let month_row = shifted(month_row0, dy);
    let mut heading = alloc::string::String::new();
    let _ = region.format(
        region.year_month,
        &WallClock {
            year,
            month,
            day: 1,
            ..time
        },
        &mut heading,
    );
    let heading_w = (prev0.x() - month_row0.x() - 8).max(1) as u32;
    paint::text_row(
        c,
        fonts,
        month_row.x() + 4,
        month_row.y(),
        month_row.height(),
        &heading,
        TextStyle::Semibold14,
        palette.text,
        heading_w,
    );
    for (b0, mask) in [
        (prev0, crate::shell_mask!("chevron-up", 16)),
        (next0, crate::shell_mask!("shell-chevron-down", 16)),
    ] {
        let b = shifted(b0, dy);
        if b0.contains(pointer.0, pointer.1) {
            paint::rounded(c, b, 4, palette.subtle_hover);
        }
        paint::glyph_in(c, b, 16, mask, palette.text);
    }
    // Columns start on the region's first day of the week.
    let first = usize::from(region.first_day % 7);
    let (weekday_top, weekday_h) = weekday_row(r);
    for i in 0..7 {
        let day = region.short_days[(first + i) % 7];
        let cell = day_cell(r, i as i32);
        paint::text_span(
            c,
            fonts,
            cell.x(),
            cell.width(),
            weekday_top,
            weekday_h,
            day,
            TextStyle::Regular12,
            palette.text,
        );
    }
    let offset = (WallClock {
        year,
        month,
        day: 1,
        ..time
    }
    .weekday()
    .unwrap_or(0) as i32
        + 7
        - first as i32)
        % 7;
    // Leading/trailing days of neighbouring months in secondary text, as in
    // Windows, so the grid always has six full rows.
    let previous_days = if month == 1 {
        days_in_month(year.saturating_sub(1), 12)
    } else {
        days_in_month(year, month - 1)
    } as i32;
    let days = days_in_month(year, month) as i32;
    for index in 0..42 {
        let cell = day_cell(r, index);
        let (day, current_month) = if index < offset {
            (previous_days - offset + index + 1, false)
        } else if index - offset < days {
            (index - offset + 1, true)
        } else {
            (index - offset - days + 1, false)
        };
        let today = current_month && (year, month, day as u8) == (time.year, time.month, time.day);
        if today {
            paint::circle(c, cell.x() + 20, cell.y() + 20, 18, palette.accent);
        }
        let color = if today {
            palette.accent_text
        } else if current_month {
            palette.text
        } else {
            palette.text_disabled
        };
        paint::text_in(
            c,
            fonts,
            cell,
            &format!("{day}"),
            TextStyle::Regular14,
            color,
        );
    }
    draw_focus_footer(c, fonts, state, r, palette, pointer, dy);
}
/// The 26H2 Focus footer: a rule, "−", the session length, "+" and the
/// "▶ Focus" button.
fn draw_focus_footer(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    state: &State,
    r: Rect,
    palette: &Palette,
    pointer: (i32, i32),
    dy: i32,
) {
    paint::rect(
        c,
        r.x() + 1,
        r.bottom() - 51,
        r.width() - 2,
        1,
        palette.surface_divider,
    );
    let r0 = shifted(r, -dy);
    let (shorter, label, longer, focus) = focus_controls(r);
    let (shorter0, _, longer0, focus0) = focus_controls(r0);
    for (b, b0) in [(shorter, shorter0), (longer, longer0), (focus, focus0)] {
        let fill = if b0.contains(pointer.0, pointer.1) {
            palette.subtle_hover
        } else {
            palette.card
        };
        paint::framed(c, b, 4, palette.field_border, fill);
    }
    // Minus and plus strokes (Segoe Fluent ChromeMinimize/Add proportions).
    for b in [shorter, longer] {
        paint::rect(c, b.x() + 7, b.y() + 11, 10, 1, palette.text);
    }
    paint::rect(c, longer.x() + 11, longer.y() + 7, 1, 10, palette.text);
    paint::text_in(
        c,
        fonts,
        label,
        &format!("{} mins", state.focus_minutes()),
        TextStyle::Semibold14,
        palette.text,
    );
    // Play triangle then "Focus", centred as one group.
    let text_w = paint::measure(fonts, "Focus", TextStyle::Regular14) as i32;
    let left = focus.x() + (focus.width() as i32 - (10 + 6 + text_w)) / 2;
    for row in 0i32..10 {
        let w = (5 - (row - 5).abs()).max(0) * 2;
        if w > 0 {
            paint::rect(c, left, focus.y() + 7 + row, w as u32, 1, palette.text);
        }
    }
    paint::text_row(
        c,
        fonts,
        left + 16,
        focus.y(),
        focus.height(),
        "Focus",
        TextStyle::Regular14,
        palette.text,
        text_w as u32 + 2,
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clock_follows_the_validated_time_zone() {
        let utc = WallClock {
            year: 2026,
            month: 10,
            day: 24,
            hour: 19,
            minute: 5,
            second: 0,
        };
        let mut state = State::default();
        // Unset: the RTC's UTC.
        assert_eq!(state.localize(utc), Some(utc));
        assert!(!state.set_locale(0, 1) && !state.set_locale(3, 99));
        state.time = Some(utc);
        assert!(state.set_locale(3, 3)); // Pacific Time, German (Germany)
        assert_eq!(
            (state.zone, state.region, state.time, state.poll_at),
            (3, 3, None, 0)
        );
        let local = state.localize(utc).unwrap();
        assert_eq!((local.day, local.hour, local.minute), (24, 12, 5)); // PDT
        assert!(state.set_locale(39, 5)); // Beijing: next day
        assert_eq!(state.localize(utc).map(|t| (t.day, t.hour)), Some((25, 3)));
    }
    #[test]
    fn calendar_boundary_and_tray_hit_targets() {
        let mut state = State::default();
        assert!(state.update(WallClock {
            year: 2026,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0
        }));
        assert_eq!(state.hit(1280, 720, 1200, 700), Some(Action::Clock));
        state.activate(Action::Clock);
        let layout = Layout::new(1280, 720);
        let cal = calendar_rect(layout);
        assert_eq!(
            state.hit(1280, 720, cal.x() + 100, cal.y() + 250),
            Some(Action::KeepOpen)
        );
        state.activate(Action::KeepOpen);
        assert_eq!(state.open, Some(Flyout::Calendar));
        let (_, _, prev, next) = calendar_layout(cal);
        assert_eq!(
            state.hit(1280, 720, prev.x() + 4, prev.y() + 4),
            Some(Action::Previous)
        );
        state.activate(Action::Previous);
        assert_eq!(state.month, Some((2025, 12)));
        assert_eq!(
            state.hit(1280, 720, next.x() + 4, next.y() + 4),
            Some(Action::Next)
        );
        state.activate(Action::Next);
        assert_eq!(state.month, Some((2026, 1)));
        state.activate(Action::Previous);
        state.activate(Action::Today);
        assert_eq!(state.month, None);
        assert_eq!(state.hit(1280, 720, 0, 0), Some(Action::Dismiss));
        state.activate(Action::Dismiss);
        assert_eq!(state.open, None);
    }
    /// The guest scenario's tray points (shell.rs) from this geometry: the
    /// taskbar targets, calendar arrows and gear act, and the tile fill
    /// probes are opaque tile colors even over an acrylic surface.
    #[test]
    fn scenario_points_hit_their_targets_and_tile_probes_are_opaque() {
        let center = |r: Rect| (r.x() + r.width() as i32 / 2, r.y() + r.height() as i32 / 2);
        let layout = Layout::new(1280, 720);
        assert_eq!(
            layout.hit_tray(
                center(layout.tray(TrayTarget::Clock)).0,
                center(layout.tray(TrayTarget::Clock)).1
            ),
            Some(TrayTarget::Clock)
        );
        let mut state = State::default();
        state.update(WallClock {
            year: 2026,
            month: 10,
            day: 22,
            hour: 22,
            minute: 9,
            second: 0,
        });
        state.activate(Action::Clock);
        let (_, month, prev, next) = calendar_controls(layout);
        assert!(calendar_rect(layout).contains(month.x(), month.y()));
        assert_eq!(
            state.hit(1280, 720, center(prev).0, center(prev).1),
            Some(Action::Previous)
        );
        assert_eq!(
            state.hit(1280, 720, center(next).0, center(next).1),
            Some(Action::Next)
        );
        let mut quick = State::default();
        quick.activate(Action::Devices);
        let (gx, gy) = center(settings_button(layout));
        assert_eq!(quick.hit(1280, 720, gx, gy), Some(Action::Settings));
        let (qx, qy) = center(layout.tray(TrayTarget::QuickSettings));
        assert_eq!(layout.hit_tray(qx, qy), Some(TrayTarget::QuickSettings));
        // Paint over a mid-gray scene with transparency on (no backdrop yet
        // means the opaque fallback) and with an acrylic backdrop built.
        let fonts = FontAtlas::windows_reference().unwrap();
        for dark in [false, true] {
            let mut bytes = alloc::vec![0u8; 1280 * 720 * 4];
            for p in bytes.chunks_exact_mut(4) {
                p.copy_from_slice(&[120, 90, 60, 255]);
            }
            let scene = bytes.clone();
            quick
                .backdrop
                .build(&scene, 1280 * 4, quick_rect(layout), 8);
            let mut c = Canvas::new(&mut bytes, 1280, 720, 5120).unwrap();
            draw_flyout(
                &mut c,
                &fonts,
                &quick,
                Flyout::Devices,
                layout,
                &Frame {
                    dark,
                    pointer: (0, 0),
                    slide: 0,
                    transparency: true,
                },
            );
            let (enabled, _, disabled) = tile_colors(dark);
            for (i, expected) in [(3, enabled), (2, disabled)] {
                let (x, y) = tile_fill_point(layout, i);
                let at = (y as usize * 1280 + x as usize) * 4;
                assert_eq!(&bytes[at..at + 4], &expected, "tile {i} dark {dark}");
            }
        }
    }
    #[test]
    fn quick_settings_targets_are_inside_and_only_working_tiles_act() {
        for (w, h) in [(1280, 720), (1920, 1080), (800, 600)] {
            let layout = Layout::new(w, h);
            let mut state = State::default();
            state.activate(Action::Devices);
            let r = quick_rect(layout);
            assert_eq!(layout.work_area().intersection(r), Some(r));
            for i in 0..6 {
                let t = tile(layout, i);
                assert_eq!(r.intersection(t), Some(t));
                let expected = match i {
                    3 => Action::Accessibility,
                    4 => Action::Vpn,
                    _ => Action::KeepOpen,
                };
                assert_eq!(state.hit(w, h, t.x() + 4, t.y() + 4), Some(expected));
            }
            // The second page: Night light toggles, Cast and Project open
            // their pages; the radio-dependent tiles stay inert.
            let down = pages::page_down(layout);
            assert_eq!(
                state.hit(w, h, down.x() + 8, down.y() + 7),
                Some(Action::TilesDown)
            );
            state.activate(Action::TilesDown);
            state.now = u64::MAX / 2;
            for i in 0..6 {
                let t = tile(layout, i);
                let expected = match i {
                    1 => Action::NightLight,
                    4 => Action::Cast,
                    5 => Action::Project,
                    _ => Action::KeepOpen,
                };
                assert_eq!(
                    state.hit(w, h, t.x() + 4, t.y() + 4),
                    Some(expected),
                    "page 2 tile {i}"
                );
            }
            for (action, page) in [
                (Action::Cast, QuickPage::Cast),
                (Action::Project, QuickPage::Project),
                (Action::Accessibility, QuickPage::Accessibility),
            ] {
                state.activate(action);
                assert_eq!(state.quick_page, page);
                let f = pages::page_frame(layout);
                assert_eq!(
                    state.hit(w, h, f.more.x() + 30, f.more.y() + 20),
                    Some(Action::MoreSettings)
                );
                assert_eq!(
                    state.hit(w, h, f.back.x() + 8, f.back.y() + 8),
                    Some(Action::Back)
                );
                state.activate(Action::Back);
            }
            state.activate(Action::Project);
            let row = pages::project_row(layout, 2);
            assert_eq!(
                state.hit(w, h, row.x() + 40, row.y() + 10),
                Some(Action::ProjectMode(2))
            );
            state.activate(Action::Back);
            state.activate(Action::Accessibility);
            let rows = pages::access_rows(layout, 0);
            let (_, filters) = rows[2];
            assert_eq!(
                state.hit(w, h, filters.x() + 40, filters.y() + 10),
                Some(Action::AccessToggle(AccessRow::ColorFilters))
            );
            // Narrator needs a speech engine: inert.
            let (_, narrator) = rows[1];
            assert_eq!(
                state.hit(w, h, narrator.x() + 40, narrator.y() + 10),
                Some(Action::KeepOpen)
            );
            state.activate(Action::Back);
            state.activate(Action::TilesUp);
            let gear = settings_button(layout);
            assert_eq!(
                state.hit(w, h, gear.x() + 4, gear.y() + 4),
                Some(Action::Settings)
            );
            assert!(TILES.iter().filter(|t| t.2.is_none()).count() == 2);
            // Its background keeps it open; outside light-dismisses.
            let (bx, by) = quick_background_point(layout);
            assert!(r.contains(bx, by) && (0..6).all(|i| !tile(layout, i).contains(bx, by)));
            assert_eq!(state.hit(w, h, bx, by), Some(Action::KeepOpen));
            assert_eq!(
                state.hit(w, h, r.x() - 108, r.y() + 126),
                Some(Action::Dismiss)
            );
            for i in 0..6 {
                let (px, py) = tile_fill_point(layout, i);
                assert!(tile(layout, i).contains(px, py));
            }
        }
    }
    #[test]
    fn input_switcher_lists_installed_methods_and_is_hidden_with_one_keyboard() {
        let layout = Layout::new(1280, 720);
        let lang = layout.tray(TrayTarget::Language);
        let (lx, ly) = (lang.x() + 4, lang.y() + 4);
        let mut state = State::default();
        // One keyboard: no indicator, so its slot is inert.
        assert_eq!(state.hit(1280, 720, lx, ly), None);
        state.activate(Action::Language);
        assert_eq!(state.open, None);
        // English (US) and Microsoft Pinyin, Pinyin current.
        state.input = door_keyboard_layouts::Preferences::new(&[1, 7], 1, false);
        assert_eq!(state.hit(1280, 720, lx, ly), Some(Action::Language));
        state.activate(Action::Language);
        assert_eq!(state.open, Some(Flyout::Input(2)));
        let r = input_rect(layout, 2);
        assert_eq!(layout.work_area().intersection(r), Some(r));
        for i in 0..2 {
            let row = input_row(layout, 2, i);
            assert!(r.contains(row.x(), row.y()) && r.contains(row.right() - 1, row.bottom() - 1));
            assert_eq!(
                state.hit(1280, 720, row.x() + 8, row.y() + 8),
                Some(Action::InputMethod(i))
            );
        }
        let footer = input_footer(layout, 2);
        assert_eq!(
            state.hit(1280, 720, footer.x() + 20, footer.y() + 20),
            Some(Action::KeyboardSettings)
        );
        assert_eq!(state.hit(1280, 720, 100, 100), Some(Action::Dismiss));
        let methods: alloc::vec::Vec<_> =
            state.input.unwrap().methods().map(keyboard_label).collect();
        assert_eq!(methods, ["US keyboard", "Macrohard Pinyin"]);
        // The indicator toggles the switcher closed; choosing closes it.
        state.activate(Action::Language);
        assert_eq!(state.open, None);
        state.activate(Action::Language);
        state.activate(Action::InputMethod(0));
        assert_eq!(state.open, None);
        // Painting stays inside the flyout (with its shadow) in both themes.
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        for dark in [false, true] {
            let mut bytes = alloc::vec![0u8; 1280 * 720 * 4];
            let before = bytes.clone();
            let mut c = Canvas::new(&mut bytes, 1280, 720, 1280 * 4).unwrap();
            draw_flyout(
                &mut c,
                &fonts,
                &state,
                Flyout::Input(2),
                layout,
                &Frame {
                    dark,
                    pointer: (0, 0),
                    slide: 0,
                    transparency: false,
                },
            );
            let grown = paint::grow(r, 26, layout.output());
            let changed = bytes
                .chunks_exact(4)
                .zip(before.chunks_exact(4))
                .enumerate()
                .filter(|(_, (a, b))| a != b);
            for (i, _) in changed {
                assert!(grown.contains((i % 1280) as i32, (i / 1280) as i32));
            }
        }
    }
}
