//! Native Settings: Windows 11 (26H2) window structure with service-backed
//! preferences. Title bar (Back, caption, centred search), account header,
//! category rail and a scrolling page of cards built from `settings_catalog`.
//! Every legacy Control Panel applet, System Properties tab and MMC tool
//! Doors replaces is a page of this app (see `door_settings_map`), reached
//! from its category, from search by its legacy name, or by a deep link.
use super::*;
use crate::icons::Icon;
use alloc::vec::Vec;
use door_graphics_text::TextStyle;
use door_service_client::Setting;
use door_service_protocol::desktop::Controls;
use door_settings_map::Hit;
use door_workspace::settings_catalog::{
    self as catalog, Action, Change, Choice, DialogKind, List, Popup, PopupKey, State,
};
use door_workspace::settings_layout::{Layout, Page};
use door_workspace::settings_navigation::{self as navigation, Rail};
#[path = "settings/admin.rs"]
mod admin;
#[path = "settings/desktop.rs"]
mod desktop;
#[path = "settings/dialog.rs"]
pub mod dialog;
#[path = "settings/menu_custom.rs"]
pub mod menu_custom;
// Explorer's compact menu painter, for the customizer's live preview.
#[path = "context_menu_paint.rs"]
mod menu_paint;
#[path = "settings/pages.rs"]
mod pages;
#[path = "settings/symbols.rs"]
mod symbols;
#[path = "settings/theme.rs"]
mod theme;
// door.accessibility semantic tree (shared adapter `door_workspace::semantics`).
#[path = "settings/semantics.rs"]
mod semantics;
/// Top of the large page heading (Semibold28): 26H2 puts the breadcrumb's
/// ink top 64 px below the window top (VIS-01 `settings.page_title`,
/// `settings-colors.png`: TextBlock at y 82 px of a window at -11 px, 150 %).
const HEADING_Y: i32 = 54;
/// Height of the page heading band (40 px at 100% text size).
fn heading_height() -> i32 {
    40 + Layout::heading_growth()
}
/// Target identities.
const SEARCH: u32 = 200;
const BACK: u32 = 201;
const FORWARD: u32 = 202;
const SUGGESTION: u32 = 150;
const SUGGESTION_LIST: u32 = 149;
/// Breadcrumb ancestors: ANCESTOR + their index in the heading.
const ANCESTOR: u32 = 260;
const MAIN_BAR: u32 = 400;
const RAIL_BAR: u32 = 401;
const ITEM: u32 = 3000;
const POPUP_OPTION: u32 = 5000;
const POPUP_BACKGROUND: u32 = 4999;
/// Wheel distance per detent (WinUI ScrollViewer: three 16 px lines, 48).
const WHEEL_STEP: i32 = 48;
/// How long a deep-link target stays highlighted (100 Hz ticks).
const HIGHLIGHT_TICKS: u64 = 250;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PopupKind {
    Choice(Choice),
    AppMenu(usize),
    /// Typing: installed keyboard `index`'s "..." menu.
    KeyboardMenu(usize),
    /// Typing: Add a keyboard.
    AddKeyboard,
    /// A dialog's registry Type combo.
    DialogType,
    /// Default apps: the apps file type `index` can open with.
    DefaultApp(usize),
    /// Installed apps: the confirmation under an app's Uninstall.
    ConfirmUninstall(usize),
}
/// Read-only facts gathered once from real providers.
#[derive(Clone, Copy, Default)]
struct Facts {
    volume: Option<door_service_protocol::administration::VolumeUsage>,
    processors: Option<u16>,
    illustration: Option<door_abi::hardware::Illustration>,
    tick_hz: Option<u64>,
}
/// Settings page entrance rise (26H2 at 150 %: 210 px = 140 epx).
const ENTRANCE_RISE: i32 = 140;
/// Smoothly animated scroll offset (ease toward `target` on ticks).
#[derive(Clone, Copy, Default)]
struct Scroll {
    value: u32,
    target: u32,
}
impl Scroll {
    fn set(&mut self, value: u32) {
        self.value = value;
        self.target = value;
    }
    fn clamp(&mut self, max: u32) {
        self.value = self.value.min(max);
        self.target = self.target.min(max);
    }
    /// Advance `ticks` 10 ms animation steps. Returns whether the value changed.
    fn advance(&mut self, ticks: u64) -> bool {
        let before = self.value;
        for _ in 0..ticks.min(8) {
            self.step();
        }
        self.value != before
    }
    fn step(&mut self) -> bool {
        if self.value == self.target {
            return false;
        }
        let distance = i64::from(self.target) - i64::from(self.value);
        // Ease out: cover ~40% of the remaining distance per 10 ms tick.
        let delta = if distance.abs() <= 2 {
            distance
        } else {
            (distance * 2 / 5).clamp(-480, 480)
        };
        let delta = if delta == 0 { distance.signum() } else { delta };
        self.value = (i64::from(self.value) + delta).max(0) as u32;
        true
    }
}
pub struct Settings {
    page: Page,
    history: Vec<Page>,
    future: Vec<Page>,
    rail: Scroll,
    rail_grab: Option<i32>,
    /// Providers of the administration pages (devices, disks, services,
    /// registry, variables, checks) and the session event log.
    admin: admin::Admin,
    /// The open ContentDialog (modal).
    dialog: Option<dialog::Dialog>,
    /// A deep-link or search target being highlighted: page, the
    /// section's title and the tick the highlight ends.
    highlight: Option<(Page, &'static str, u64)>,
    /// The title of the setting the current activation changes (event log).
    acting: Option<&'static str>,
    pub pending_theme: Option<u32>,
    pub pending_controls: Option<Controls>,
    controls: Controls,
    appearance: u32,
    pub pending_launch: Option<u8>,
    /// The typed settings registry Settings commits through (None in
    /// host-painted reviews, which have no storage).
    watch: Option<door_workspace::settings_watch::Watch>,
    /// Installed apps: a program's Modify or Uninstall command (Windows
    /// path, arguments) for the app loop to start.
    pub pending_program: Option<(String, Vec<String>)>,
    /// The button the last app menu opened from (the Uninstall flyout
    /// opens there too).
    menu_anchor: Option<catalog::Rect>,
    /// When Installed apps reads the registered programs again.
    programs_next: u64,
    /// The network service's last status (Network & internet pages) and a
    /// configuration waiting to be sent (adapter on/off, addressing, DNS).
    pub net: Option<door_service_protocol::net::Status>,
    /// The adapter's IPv6 side (link-local, autoconfigured address, DNS).
    pub net6: Option<door_service_protocol::net::v6::Status6>,
    /// The sound service's status (System > Sound), a pending Test tone,
    /// and a slider change still to be stored (sliders apply live while
    /// dragged and are stored once released).
    pub audio: Option<door_service_protocol::audio::Status>,
    /// Proxy > Automatically detect settings, read with the page.
    pub proxy_auto_detect: bool,
    /// Firewall & network protection: the profiles door.network enforces
    /// (read once a second on the page), and a change for the window
    /// server to put to User Account Control (`firewall::CHANGE` body).
    pub firewall: [u8; 3],
    pub pending_firewall: Option<Vec<u8>>,
    pub pending_test: bool,
    pub unsaved_pref: Option<(u32, u32)>,
    pub pending_net: Option<door_service_protocol::net::Config>,
    /// VPN requests for door.network, sent by the main loop in order.
    pub pending_vpn: Vec<VpnOp>,
    /// "Customize the context menu" asked for the customizer window.
    pub open_customizer: bool,
    /// Physical display size and scale (percent) reported by the window server.
    pub output: (u32, u32),
    pub output_scale: u32,
    layout: Layout,
    query: String,
    search_focus: bool,
    suggestion: usize,
    /// Focused content item (index into the page layout's items).
    focus: Option<usize>,
    /// Focus visuals appear only for keyboard focus, as in WinUI.
    keyboard_focus: bool,
    /// Open expanders of the current page.
    open: u64,
    popup: Option<(Popup, PopupKind)>,
    uid: u32,
    scroll: Scroll,
    scroll_grab: Option<i32>,
    facts: Facts,
    last_tick: u64,
    /// Painted ancestor crumb (page, x, width): hit testing uses exactly the
    /// geometry the last frame measured with the real font.
    #[allow(clippy::type_complexity)]
    heading_hit: core::cell::Cell<(Option<Page>, [Option<(Page, i32, u32)>; 2])>,
    /// Stored System > Display > Scale (`ScalePercent`, 0 automatic).
    scale_setting: u32,
    /// A chosen scale the window server could not apply live: the row says
    /// "Sign out to apply the new scale".
    scale_note: bool,
    /// A chosen scale (0 automatic) waiting for the window server.
    pub pending_scale: Option<u32>,
    /// The persisted scale (restored when the server refuses a choice).
    scale_stored: u32,
    /// The window server's scale report is still to be read (once).
    scale_query: bool,
    /// Window server report: effective and recommended percents and the
    /// offered scales (bit i = `catalog::SCALES[i]`).
    scale_effective: Option<u32>,
    scale_recommended: Option<u32>,
    scale_offered: u32,
    /// Accessibility pages' pending selections (applied with Apply).
    contrast_pick: u8,
    text_pick: u32,
    /// The text size slider follows the pointer while pressed.
    slider_grab: Option<catalog::Slider>,
    /// Installed keyboards and input methods (`Setting::InputMethods`) and
    /// the window server's text-input flags (IME running, dictionaries).
    input: u32,
    ime: u32,
    /// A changed keyboard list waiting for the window server (applied live).
    pub pending_input: Option<u32>,
    /// The window server's text-input state is to be read (Typing page).
    input_query: bool,
    /// `Setting::TimeZone` / `Setting::RegionalFormat` ids.
    zone: u32,
    region: u32,
    /// A changed time zone or regional format waiting for the window
    /// server's taskbar clock (zone, region).
    pub pending_locale: Option<(u32, u32)>,
    /// P14 preference words and the accent setting, as stored.
    prefs: [u32; catalog::PREFS],
    accent: u32,
    /// Changed preference words and accent waiting for the window server
    /// (and, for notification words, the notification service).
    pub pending_prefs: Vec<(u32, u32)>,
    /// Page entrance start (scheduler tick) while the new page rises.
    entrance: Option<u64>,
    /// Page entrance animation enabled (the review harness turns it off).
    page_motion: bool,
    pub pending_accent: Option<u32>,
    /// Sign-in options: the session's credential store, whether a PIN is
    /// enrolled (None: unknown) and the update in flight.
    credentials: Option<door_service_client::Credentials>,
    pin_enrolled: Option<bool>,
    credential_busy: bool,
    /// About: this PC's name, and the service registry Rename asks Init through.
    pub device_name: String,
    registry: u64,
}
impl Settings {
    pub fn new(
        storage: &mut Storage,
        startup: &door_abi::platform::StartupV2,
    ) -> Result<Self, Error> {
        // The registry first: a damaged stored value is reset to its default
        // (and reported) before Settings reads it.
        let watch = door_workspace::settings_watch::Watch::load(
            storage,
            startup.base.inherited[door_abi::platform::inherited::REGISTRY].handle,
        );
        let appearance = storage.get_setting(Setting::DesktopAppearance)?;
        let controls = Controls::from_bits(storage.get_setting(Setting::DesktopControls)?)
            .ok_or(Error::Malformed)?;
        let mut settings = Self::with_preferences(startup, appearance, controls);
        settings.watch = Some(watch);
        settings.scale_setting = storage.get_setting(Setting::ScalePercent).unwrap_or(0);
        settings.input = storage
            .get_setting(Setting::InputMethods)
            .ok()
            .filter(|bits| door_keyboard_layouts::Preferences::from_bits(*bits).is_some())
            .unwrap_or(door_keyboard_layouts::Preferences::DEFAULT.bits());
        settings.scale_stored = settings.scale_setting;
        // Unknown stored ids fall back to UTC / English (United States).
        settings.zone = storage
            .get_setting(Setting::TimeZone)
            .ok()
            .filter(|id| door_locale::tz::valid(*id))
            .unwrap_or(door_locale::tz::DEFAULT_ID as u32);
        settings.region = storage
            .get_setting(Setting::RegionalFormat)
            .ok()
            .filter(|id| door_locale::region::valid(*id))
            .unwrap_or(door_locale::region::DEFAULT_ID as u32);
        // P14 preference words (storage returns each word's default until
        // the user changes it) and the accent setting.
        for (i, word) in settings.prefs.iter_mut().enumerate() {
            if let Some(setting) = Setting::from_id(catalog::PREF_FIRST + i as u32)
                && let Ok(v) = storage.get_setting(setting)
            {
                *word = v;
            }
        }
        settings.accent = storage.get_setting(Setting::AccentColor).unwrap_or(0);
        // Sign-in options: which secrets the credential store holds (a
        // state query never waits for Argon2).
        let registry = startup.base.inherited[door_abi::platform::inherited::REGISTRY].handle;
        settings.registry = registry;
        settings.device_name = String::from(door_service_client::device_name(storage).as_str());
        if let Ok(mut c) = door_service_client::Credentials::connect(registry) {
            let start = door_sys::time().unwrap_or(0);
            if c.begin_state(start).is_ok() {
                loop {
                    let now = door_sys::time().unwrap_or(0);
                    match c.poll(now) {
                        Ok(Some(door_service_client::CredentialReply::State(s))) => {
                            settings.pin_enrolled = Some(
                                s.enrolled == Some(door_service_protocol::credential::Kind::Pin),
                            );
                            break;
                        }
                        Ok(None) if now.saturating_sub(start) <= 50 => {
                            let _ = door_sys::sleep(now + 1);
                        }
                        _ => break,
                    }
                }
            }
            settings.credentials = Some(c);
        }
        // Real providers only; a failed query leaves the value unavailable.
        settings.facts.volume = storage.volume_usage().ok();
        if let Ok(info) = door_sys::hardware_info() {
            settings.facts.processors = Some(info.processors);
            settings.facts.illustration = Some(info.illustration());
        }
        settings.facts.tick_hz = door_sys::query().ok().map(|info| info.tick_hz);
        Ok(settings)
    }
    fn with_preferences(
        startup: &door_abi::platform::StartupV2,
        appearance: u32,
        controls: Controls,
    ) -> Self {
        Self {
            page: Page::Home,
            history: Vec::new(),
            future: Vec::new(),
            rail: Scroll::default(),
            rail_grab: None,
            admin: admin::Admin::new(startup),
            dialog: None,
            highlight: None,
            acting: None,
            appearance,
            pending_theme: None,
            pending_controls: None,
            controls,
            pending_launch: None,
            watch: None,
            pending_program: None,
            menu_anchor: None,
            programs_next: 0,
            net: None,
            net6: None,
            pending_net: None,
            pending_vpn: Vec::new(),
            audio: None,
            proxy_auto_detect: true,
            firewall: [door_service_protocol::net::firewall::profile::DEFAULT; 3],
            pending_firewall: None,
            pending_test: false,
            unsaved_pref: None,
            open_customizer: false,
            output: (0, 0),
            output_scale: 0,
            layout: Layout {
                width: 1129,
                height: 787,
            },
            query: String::new(),
            search_focus: false,
            suggestion: 0,
            focus: None,
            keyboard_focus: false,
            open: 0,
            popup: None,
            uid: startup.identity.uid,
            scroll: Scroll::default(),
            scroll_grab: None,
            facts: Facts::default(),
            last_tick: 0,
            heading_hit: core::cell::Cell::new((None, [None; 2])),
            scale_setting: 0,
            scale_note: false,
            pending_scale: None,
            scale_stored: 0,
            scale_query: true,
            scale_effective: None,
            scale_recommended: None,
            scale_offered: catalog::ALL_SCALES,
            contrast_pick: catalog::contrast_pick(controls),
            text_pick: controls.text_percent(),
            slider_grab: None,
            input: door_keyboard_layouts::Preferences::DEFAULT.bits(),
            ime: 0,
            pending_input: None,
            input_query: false,
            zone: door_locale::tz::DEFAULT_ID as u32,
            region: door_locale::region::DEFAULT_ID as u32,
            pending_locale: None,
            prefs: catalog::default_prefs(),
            accent: 0,
            pending_prefs: Vec::new(),
            entrance: None,
            page_motion: true,
            pending_accent: None,
            credentials: None,
            pin_enrolled: None,
            credential_busy: false,
            device_name: String::from(door_service_protocol::device::DEFAULT_NAME),
            registry: 0,
        }
    }
    #[cfg(test)]
    pub fn review(startup: &door_abi::platform::StartupV2, page: Page, dark: bool) -> Self {
        let mut settings =
            Self::with_preferences(startup, u32::from(dark), Controls::from_bits(48).unwrap());
        settings.page = page;
        settings.page_motion = false;
        settings.entrance = None;
        settings.open = u64::MAX;
        settings.admin.review();
        settings.output = (1920, 1080);
        settings.output_scale = 100;
        settings.facts.volume = Some(door_service_protocol::administration::VolumeUsage {
            total_bytes: 64 << 30,
            free_bytes: 41 << 30,
        });
        settings.facts.processors = Some(2);
        settings.facts.illustration = Some(door_abi::hardware::Illustration::Monitor);
        settings.facts.tick_hz = Some(100);
        settings
    }
    /// Registry page review state: these subkeys and values at the current
    /// key; `edit`: Some(None) opens New value, Some(Some(i)) edits value `i`.
    #[cfg(test)]
    pub fn review_registry(
        &mut self,
        children: &[&str],
        values: &[(&str, u8, &str)],
        edit: Option<Option<usize>>,
    ) {
        self.admin.review_registry(children, values);
        match edit {
            Some(None) => self.command(catalog::Command::RegistryNewValue, None),
            Some(Some(index)) => self.edit_entry(List::RegistryValues, index, None),
            None => (),
        }
    }
    /// The open dialog: its kind, focus, field texts, type and error.
    #[cfg(test)]
    pub fn dialog_state(&self) -> Option<(DialogKind, dialog::Focus, [String; 2], u8, String)> {
        self.dialog.as_ref().map(|d| {
            (
                d.kind,
                d.focus,
                [
                    String::from(d.fields[0].text()),
                    String::from(d.fields[1].text()),
                ],
                d.value_kind,
                d.error.clone(),
            )
        })
    }
    /// Event log entries (newest first).
    #[cfg(test)]
    pub fn events(&self) -> Vec<String> {
        self.admin
            .events
            .iter()
            .map(|(_, text)| text.clone())
            .collect()
    }
    /// The highlighted section title, if any.
    #[cfg(test)]
    pub fn highlighted(&self) -> Option<&'static str> {
        self.highlight.map(|(_, title, _)| title)
    }
    #[cfg(test)]
    pub fn review_state(&mut self, appearance: u32, controls: u32) {
        self.appearance = appearance;
        self.controls = Controls::from_bits(controls).unwrap();
    }
    /// The page entrance on (the review harness starts with it off).
    #[cfg(test)]
    pub fn enable_page_motion(&mut self) {
        self.page_motion = true;
    }
    #[cfg(test)]
    pub fn entrance_offset_for_test(&self) -> i32 {
        self.entrance_offset()
    }
    #[cfg(test)]
    pub fn current_page(&self) -> Page {
        self.page
    }
    #[cfg(test)]
    pub fn scroll_offset(&self) -> (u32, u32) {
        (self.scroll.value, self.scroll.target)
    }
    #[cfg(test)]
    pub fn controls_bits(&self) -> u32 {
        self.pending_controls.unwrap_or(self.controls).bits()
    }
    pub fn configure(&mut self, width: u32, height: u32) {
        self.popup = None;
        self.layout = Layout { width, height };
        if self.layout.search().is_none() {
            self.search_focus = false;
        }
        let max = self.max_scroll();
        self.scroll.clamp(max);
        let rail_max = self.rail().max_scroll(self.items().count());
        self.rail.clamp(rail_max);
    }
    /// Display mode reported by the window server (physical pixels, scale
    /// in percent).
    pub fn display(&mut self, width: u32, height: u32, percent: u32) {
        self.output = (width, height);
        self.output_scale = percent;
    }
    fn state(&self) -> State {
        State {
            controls: self.controls,
            appearance: self.appearance,
            scale: self.scale_setting,
            recommended: self.recommended_scale(),
            offered: self.scale_offered,
            contrast_pick: self.contrast_pick,
            text_pick: self.text_pick,
            input: self.input,
            ime: self.ime,
            zone: self.zone,
            region: self.region,
            registry_root: self.admin.root_index().0,
            registry_view: self.admin.root_index().1,
            prefs: self.prefs,
            accent: self.accent,
            net: self.net,
            net6: self.net6,
            audio: self.audio,
            proxy_auto_detect: self.proxy_auto_detect,
            firewall: self.firewall,
        }
    }
    /// Whether the open page shows sound state (polled while open).
    pub fn wants_audio(&self) -> bool {
        matches!(self.page, Page::Sound | Page::Microphone)
    }
    /// Whether the open page lists microphone use (door.audio capture
    /// records, polled while open).
    pub fn wants_capture(&self) -> bool {
        self.page == Page::Microphone
    }
    /// New capture records from door.audio: each use as the Microphone
    /// page lists it ("Currently in use" or "Last accessed <date time>").
    /// True when what the page shows changed.
    pub fn set_capture(
        &mut self,
        status: Option<door_service_protocol::audio::CaptureStatus>,
    ) -> bool {
        let region = door_locale::region::region_or_default(self.state().region);
        let zone = door_locale::tz::zone_or_utc(self.state().zone);
        let utc = door_sys::wall_time().ok();
        let mut shown = Vec::new();
        if let Some(status) = status {
            for u in status.recent.iter().take(usize::from(status.used)) {
                let detail = if u.active {
                    String::from("Currently in use")
                } else {
                    // door.audio's clock is monotonic: the wall time of the
                    // last access is now minus the time since.
                    let elapsed = status.now.saturating_sub(u.last).saturating_mul(10_000);
                    match utc.and_then(|t| zone.to_local(t.saturating_sub(elapsed))) {
                        Some(local) => {
                            let mut s = String::from("Last accessed ");
                            let _ = region.short_date(&local.time, &mut s);
                            s.push(' ');
                            let _ = region.long_time(&local.time, &mut s);
                            s
                        }
                        None => String::from("Last accessed recently"),
                    }
                };
                let name = if u.name().is_empty() {
                    String::from("Unknown app")
                } else {
                    String::from(u.name())
                };
                shown.push((name, detail, u.desktop));
            }
        }
        if shown == self.admin.microphone {
            return false;
        }
        self.admin.microphone = shown;
        true
    }
    /// Firewall & network protection reads the policy door.network holds.
    pub fn wants_firewall(&self) -> bool {
        self.page == Page::Firewall
    }
    /// Settings > VPN or a connection's Advanced options is showing (the
    /// connections are polled).
    pub fn wants_vpn(&self) -> bool {
        matches!(self.page, Page::Vpn | Page::VpnConnection)
    }
    /// The connections door.network reports, and their rows (name, status,
    /// up or coming up); true when the page changed.
    pub fn set_vpns(
        &mut self,
        entries: Vec<door_service_protocol::net::vpn::Entry>,
        rows: Vec<(String, String, bool)>,
    ) -> bool {
        if self.admin.vpns == rows && self.admin.vpn_entries == entries {
            return false;
        }
        self.admin.vpns = rows;
        self.admin.vpn_entries = entries;
        true
    }
    /// Whether the open page shows network state (polled while open).
    pub fn wants_network(&self) -> bool {
        matches!(
            self.page,
            Page::Network
                | Page::NetworkAdministration
                | Page::Home
                | Page::Vpn
                | Page::VpnConnection
                | Page::Firewall
        )
    }
    /// Date & time / Language & region: persist the time zone or regional
    /// format and queue both for the taskbar clock (applied live).
    fn save_locale(
        &mut self,
        setting: Setting,
        id: u32,
        storage: Option<&mut Storage>,
    ) -> Result<(), Error> {
        if let Some(storage) = storage {
            storage.set_setting(setting, id)?;
            storage.flush()?;
        }
        if setting == Setting::TimeZone {
            self.zone = id;
        } else {
            self.region = id;
        }
        self.pending_locale = Some((self.zone, self.region));
        let _ = door_sys::write(
            format!(
                "DOOR_P12 SETTINGS SAVED time_zone={} regional_format={}
",
                self.zone, self.region
            )
            .as_bytes(),
        );
        Ok(())
    }
    /// The window server applied (or refused) the queued time locale.
    pub fn locale_applied(&mut self, applied: bool) {
        if let Some((zone, region)) = self.pending_locale.take()
            && applied
        {
            let _ = door_sys::write(
                format!(
                    "DOOR_P12 SETTINGS APPLIED time_zone={zone} regional_format={region}
"
                )
                .as_bytes(),
            );
        }
    }
    #[cfg(test)]
    pub fn locale(&self) -> (u32, u32) {
        (self.zone, self.region)
    }
    /// True once per Typing page visit: read the window server's input
    /// state (installed methods as applied, IME and dictionaries).
    pub fn take_input_query(&mut self) -> bool {
        core::mem::replace(&mut self.input_query, false)
    }
    /// The window server's text-input state. Its list is the live one (it
    /// includes Win+Space switches), unless a change is still queued.
    pub fn input_reported(&mut self, preferences: u32, flags: u32) {
        if self.pending_input.is_none()
            && door_keyboard_layouts::Preferences::from_bits(preferences).is_some()
        {
            self.input = preferences;
        }
        self.ime = flags;
    }
    /// The window server applied the queued keyboard list.
    pub fn input_applied(&mut self) {
        if let Some(bits) = self.pending_input.take() {
            let _ = door_sys::write(
                format!("DOOR_P12 SETTINGS APPLIED input_methods={bits}\n").as_bytes(),
            );
        }
    }
    /// The window server refused the list (it stays saved for sign-in).
    pub fn input_refused(&mut self) {
        self.pending_input = None;
    }
    /// Typing: persist the keyboard list (`InputMethods`) and queue it for
    /// the window server, which applies it live.
    fn save_input(&mut self, bits: u32, storage: Option<&mut Storage>) -> Result<(), Error> {
        if let Some(storage) = storage {
            storage.set_setting(Setting::InputMethods, bits)?;
            storage.flush()?;
        }
        self.input = bits;
        self.pending_input = Some(bits);
        let _ =
            door_sys::write(format!("DOOR_P12 SETTINGS SAVED input_methods={bits}\n").as_bytes());
        Ok(())
    }
    #[cfg(test)]
    pub fn input_bits(&self) -> u32 {
        self.input
    }
    /// What automatic scale resolves to on this display: the window
    /// server's report, else the scale it renders at.
    fn recommended_scale(&self) -> u32 {
        self.scale_recommended
            .unwrap_or_else(|| self.output_scale.clamp(100, 300))
    }
    /// The scale in effect (percent), for Display and About.
    pub fn effective_scale(&self) -> Option<u32> {
        self.scale_effective
            .or((self.output_scale > 0).then_some(self.output_scale))
    }
    /// True once: the binary should read the window server's scale report.
    pub fn take_scale_query(&mut self) -> bool {
        core::mem::replace(&mut self.scale_query, false)
    }
    /// The window server's scale report (`DISPLAY_SCALE_INFO`).
    pub fn scale_info(&mut self, effective: u32, recommended: u32, offered: u32) {
        self.scale_effective = Some(effective);
        self.scale_recommended = Some(recommended);
        if offered & catalog::ALL_SCALES != 0 {
            self.scale_offered = offered & catalog::ALL_SCALES;
        }
    }
    /// The window server applied the pending scale live; persist it
    /// (`ScalePercent`, 0 automatic).
    pub fn scale_applied(
        &mut self,
        effective: u32,
        storage: Option<&mut Storage>,
    ) -> Result<(), Error> {
        let Some(percent) = self.pending_scale.take() else {
            return Ok(());
        };
        if let Some(storage) = storage {
            storage.set_setting(Setting::ScalePercent, percent)?;
            storage.flush()?;
        }
        self.scale_stored = percent;
        self.scale_effective = Some(effective);
        self.scale_note = false;
        let _ = door_sys::write(
            format!("DOOR_P12 SETTINGS SAVED scale={percent} effective={effective}\n").as_bytes(),
        );
        Ok(())
    }
    /// The window server refused the scale (InvalidRequest): nothing is
    /// persisted and the combo shows the stored scale again.
    pub fn scale_refused(&mut self) {
        if self.pending_scale.take().is_some() {
            self.scale_setting = self.scale_stored;
        }
    }
    /// A window server without live scale changes: persist the choice for
    /// the next sign-in, and the row says so.
    pub fn scale_deferred(&mut self, storage: Option<&mut Storage>) -> Result<(), Error> {
        let Some(percent) = self.pending_scale.take() else {
            return Ok(());
        };
        if let Some(storage) = storage {
            storage.set_setting(Setting::ScalePercent, percent)?;
            storage.flush()?;
        }
        self.scale_stored = percent;
        self.scale_note = true;
        Ok(())
    }
    /// Installed apps in display order, by name as Windows sorts them:
    /// Ok(index into INCLUDED_APPS) or Err(index into the programs).
    pub(super) fn app_order(&self) -> Vec<Result<usize, usize>> {
        let mut order: Vec<(String, Result<usize, usize>)> =
            door_workspace::settings_layout::INCLUDED_APPS
                .iter()
                .enumerate()
                .map(|(i, (name, _))| (name.to_lowercase(), Ok(i)))
                .chain(
                    self.admin
                        .programs
                        .iter()
                        .enumerate()
                        .map(|(i, p)| (p.name.to_lowercase(), Err(i))),
                )
                .collect();
        order.sort_by(|a, b| a.0.cmp(&b.0));
        order.into_iter().map(|x| x.1).collect()
    }
    /// The program shown at display index `app`, if it is one.
    fn program_at(&self, app: usize) -> Option<&door_workspace::installed_programs::Program> {
        match self.app_order().get(app) {
            Some(Err(p)) => self.admin.programs.get(*p),
            _ => None,
        }
    }
    /// An installed app's "..." menu: its entries and which are enabled.
    fn app_menu(&self, app: usize) -> (usize, u64) {
        match self.program_at(app) {
            Some(p) => (
                pages::PROGRAM_MENU.len(),
                u64::from(p.can_modify()) | u64::from(p.can_uninstall()) << 1,
            ),
            None => (pages::APP_MENU.len(), 0b001),
        }
    }
    pub(super) fn app_menu_labels(&self, app: usize) -> Vec<&'static str> {
        if self.program_at(app).is_some() {
            pages::PROGRAM_MENU.to_vec()
        } else {
            pages::APP_MENU.to_vec()
        }
    }
    fn context(&self) -> catalog::Context {
        catalog::Context {
            width: self.layout.content_width(),
            open: self.open,
            apps: door_workspace::settings_layout::INCLUDED_APPS.len() + self.admin.programs.len(),
            keyboards: self.state().input_methods().count(),
            lists: self.admin.lists(),
            vpn_up: self
                .admin
                .vpns
                .iter()
                .take(32)
                .enumerate()
                .filter(|(_, v)| v.2)
                .fold(0, |bits, (i, _)| bits | 1 << i),
            background: door_service_protocol::preferences::Background::from_bits(
                self.state()
                    .pref(door_service_protocol::preferences::BACKGROUND),
            )
            .map_or(0, |b| b.kind),
            recent_accents: self.recent_accents().colors().count(),
        }
    }
    fn recent_accents(&self) -> door_service_protocol::preferences::RecentAccents {
        use door_service_protocol::preferences::{self as p, RecentAccents};
        RecentAccents::from_bits(self.state().pref(p::ACCENT_RECENT))
            .unwrap_or(RecentAccents::DEFAULT)
    }
    /// Apply Windows color `index` as the manual accent and put it first in
    /// Recent colors.
    fn pick_accent(&mut self, index: u32, mut storage: Option<&mut Storage>) -> Result<(), Error> {
        use door_service_protocol::preferences as p;
        let Some(rgb) = door_design_tokens::accent::WINDOWS_COLORS.get(index as usize) else {
            return Ok(());
        };
        let bits = door_service_protocol::desktop::Accent::manual(*rgb).bits();
        self.apply(Change::Accent(bits), storage.as_deref_mut())?;
        let recent = self.recent_accents();
        let used = recent.used(index);
        if used != recent {
            self.save_preference(p::ACCENT_RECENT, used.bits(), storage)?;
        }
        let name = door_design_tokens::accent::WINDOWS_COLOR_NAMES[index as usize];
        self.log(format!("Accent color: {name} (#{rgb:06X})"));
        Ok(())
    }
    fn page_layout(&self) -> catalog::PageLayout {
        catalog::layout(self.page, &self.context())
    }
    fn max_scroll(&self) -> u32 {
        self.page_layout().bottom.saturating_sub(self.layout.height)
    }
    fn reduce_motion(&self) -> bool {
        self.controls.reduce_motion()
    }
    /// Mouse wheel (see `Application::wheel`): smooth pixel scrolling of the
    /// rail or the page under the pointer; an open list longer than the
    /// window scrolls by three options per notch.
    pub fn wheel(&mut self, x: i32, y: i32, notches: i32) -> bool {
        if let Some((popup, _)) = &mut self.popup {
            return popup.scroll(-notches.clamp(-20, 20) * 3);
        }
        if notches == 0 {
            return false;
        }
        let delta = -notches.clamp(-20, 20) * WHEEL_STEP;
        let reduce = self.reduce_motion();
        let rail = x < Layout::MAIN - 8 && y >= self.layout.rail_top();
        let (scroll, max) = if rail {
            let max = self.rail().max_scroll(self.items().count());
            (&mut self.rail, max)
        } else {
            let max = self.max_scroll();
            (&mut self.scroll, max)
        };
        let target = (i64::from(scroll.target) + i64::from(delta)).clamp(0, i64::from(max)) as u32;
        if target == scroll.target {
            return false;
        }
        scroll.target = target;
        if reduce {
            scroll.value = target;
        }
        true
    }
    /// Advance scroll animations by elapsed scheduler ticks (100 Hz), so the
    /// motion takes the same time however often the loop polls.
    /// A request the app loop has not yet sent (a setting to persist, an
    /// app or program to start). A close that arrives behind the click that
    /// made it waits for it, as Windows handles the click's message before
    /// WM_CLOSE.
    pub fn has_pending_request(&self) -> bool {
        self.pending_theme.is_some()
            || self.pending_controls.is_some()
            || self.pending_scale.is_some()
            || self.pending_input.is_some()
            || self.pending_locale.is_some()
            || !self.pending_prefs.is_empty()
            || self.pending_accent.is_some()
            || self.pending_launch.is_some()
            || self.pending_program.is_some()
    }
    pub fn tick(&mut self, now: u64) -> bool {
        let answered = self.poll_credentials(now);
        let elapsed = now.saturating_sub(self.last_tick);
        self.last_tick = now;
        if elapsed == 0 {
            return answered;
        }
        let page = self.scroll.advance(elapsed);
        let rail = self.rail.advance(elapsed);
        let entering = match self.entrance {
            Some(start) if self.entrance_offset() == 0 || now.saturating_sub(start) >= 100 => {
                self.entrance = None;
                self.report_page();
                true
            }
            Some(_) => true,
            None => false,
        };
        // A deep-link highlight fades out: repaint while it shows and once
        // when it ends.
        let highlight = match self.highlight {
            Some((_, _, until)) if now >= until => {
                self.highlight = None;
                true
            }
            Some(_) => true,
            None => false,
        };
        page | rail | highlight | answered | entering
    }
    pub fn cancel_drag(&mut self) {
        self.scroll_grab = None;
        self.rail_grab = None;
        self.slider_grab = None;
    }
    /// The slider item under (`x`, `y`), if any.
    fn slider_at(&self, x: i32, y: i32) -> Option<(usize, catalog::Rect, catalog::Slider)> {
        let index = self.target(x, y)?.checked_sub(ITEM)? as usize;
        let item = *self.page_layout().items.get(index)?;
        match item.action {
            Action::Slider(slider) => Some((index, item.rect, slider)),
            _ => None,
        }
    }
    /// WinUI Slider keys: arrows step 1, Page Up/Down 10, Home/End the ends.
    /// Card sliders store at once (no drag to wait for).
    fn slider_key(&mut self, slider: catalog::Slider, scan: u32) -> bool {
        let (lo, hi) = slider.range();
        let now = slider.value(self.state());
        let next = match scan {
            0x14b | 0x150 => now - 1,
            0x14d | 0x148 => now + 1,
            0x151 => now - 10,
            0x149 => now + 10,
            0x147 => lo,
            0x14f => hi,
            _ => return false,
        }
        .clamp(lo, hi);
        if next == now {
            return false;
        }
        if slider == catalog::Slider::TextSize {
            self.text_pick = next as u32;
            return true;
        }
        match slider.apply(self.state(), next) {
            Some(change @ Change::Preference(id, word)) => {
                let _ = self.apply(change, None);
                self.unsaved_pref = Some((id, word));
                true
            }
            _ => false,
        }
    }
    /// Move slider `slider` (track `rect`) to the pointer at `x`: the text
    /// size pick, or a card slider's change applied live (stored once the
    /// pointer is released, `unsaved_pref`).
    fn slide(&mut self, slider: catalog::Slider, rect: catalog::Rect, x: i32) -> bool {
        if slider == catalog::Slider::TextSize {
            let value = catalog::slider_value(rect, x);
            let changed = value != self.text_pick;
            self.text_pick = value;
            return changed;
        }
        let value = slider.value_at(rect, x);
        match slider.apply(self.state(), value) {
            Some(change @ Change::Preference(id, word)) => {
                let _ = self.apply(change, None);
                self.unsaved_pref = Some((id, word));
                true
            }
            _ => false,
        }
    }
    /// Test review: the focused item's action.
    #[cfg(test)]
    pub fn focused_action(&self) -> Option<Action> {
        let layout = self.page_layout();
        self.focus
            .and_then(|i| layout.items.get(i))
            .map(|i| i.action)
    }
    pub fn menu_open(&self) -> bool {
        self.popup.is_some() || self.suggestions_open() || self.dialog.is_some()
    }
    pub fn hover_menu(&mut self, x: i32, y: i32) -> bool {
        if let Some((popup, _)) = &mut self.popup
            && let Some(index) = popup.hit(x, y)
            && popup.enabled(index)
            && popup.selected != index
        {
            popup.selected = index;
            return true;
        }
        if self.suggestions_open()
            && let Some(index) = self.suggestion_at(x, y)
            && index != self.suggestion
        {
            self.suggestion = index;
            return true;
        }
        // A dialog's buttons and fields show their hover state.
        self.dialog.is_some()
    }
    /// Transient dismissal (focus change, capture loss): close lists only;
    /// the search box keeps its keyboard focus, as a WinUI text box does,
    /// and an open dialog stays open.
    pub fn dismiss_menu(&mut self) {
        self.popup = None;
        self.cancel_drag();
    }
    pub fn pointer_down(&mut self, x: i32, y: i32) -> bool {
        if self.dialog.is_none()
            && let Some((index, rect, slider)) = self.slider_at(x, y)
        {
            self.focus = Some(index);
            self.keyboard_focus = false;
            self.slider_grab = Some(slider);
            self.slide(slider, rect, x);
            return true;
        }
        match self.target(x, y) {
            Some(RAIL_BAR) => {
                let (top, thumb, _) = self.rail().thumb(self.items().count());
                self.rail_grab = Some(if (top..top + thumb).contains(&y) {
                    y - top
                } else {
                    thumb / 2
                });
            }
            Some(MAIN_BAR) => {
                self.scroll_grab = self
                    .layout
                    .scrollbar(self.scroll.value, self.max_scroll())
                    .map(|g| {
                        if (g.top as i32..(g.top + g.thumb) as i32).contains(&y) {
                            y - g.top as i32
                        } else {
                            g.thumb as i32 / 2
                        }
                    });
            }
            _ => {
                self.scroll_grab = None;
                return false;
            }
        }
        self.pointer_motion(x, y)
    }
    pub fn pointer_motion(&mut self, x: i32, y: i32) -> bool {
        if let Some(slider) = self.slider_grab {
            let rect = self
                .page_layout()
                .items
                .iter()
                .find(|i| i.action == Action::Slider(slider))
                .map(|i| i.rect);
            return rect.is_some_and(|rect| self.slide(slider, rect, x));
        }
        if let Some(grab) = self.rail_grab {
            let rail = self.rail();
            let (_, _, travel) = rail.thumb(self.items().count());
            let max = rail.max_scroll(self.items().count());
            let next = ((y - rail.top - grab).clamp(0, travel) as u64 * max as u64
                / travel.max(1) as u64) as u32;
            let changed = next != self.rail.value;
            self.rail.set(next);
            return changed;
        }
        let Some(grab) = self.scroll_grab else {
            return false;
        };
        let max = self.max_scroll();
        let Some(geometry) = self.layout.scrollbar(self.scroll.value, max) else {
            return false;
        };
        let next = geometry.position(y, grab, max);
        let changed = self.scroll.value != next;
        self.scroll.set(next);
        changed
    }
    fn items(&self) -> impl Iterator<Item = Page> + use<> {
        navigation::items()
    }
    fn rail(&self) -> Rail {
        Rail::new(self.layout, self.rail.value)
    }
    fn suggestions(&self) -> impl Iterator<Item = Hit> + '_ {
        navigation::suggestions(&self.query)
    }
    fn suggestions_open(&self) -> bool {
        self.search_focus && !self.query.trim().is_empty()
    }
    fn suggestion_at(&self, x: i32, y: i32) -> Option<usize> {
        let (sx, sy, sw, pitch) = navigation::suggestion_box(self.layout)?;
        let count = self.suggestions().count().max(1);
        if !(sx..sx + sw as i32).contains(&x) || y < sy + 4 || y >= sy + 4 + count as i32 * pitch {
            return None;
        }
        Some(((y - sy - 4) / pitch) as usize).filter(|i| *i < self.suggestions().count())
    }
    fn suggestion_list_contains(&self, x: i32, y: i32) -> bool {
        navigation::suggestion_box(self.layout).is_some_and(|(sx, sy, sw, pitch)| {
            let count = self.suggestions().count().max(1) as i32;
            (sx..sx + sw as i32).contains(&x) && (sy..sy + 8 + count * pitch).contains(&y)
        })
    }
    fn reveal_page(&mut self) {
        let selected = self.page.category();
        if let Some(index) = self.items().position(|page| page == selected) {
            let target = self.rail().reveal(index, self.items().count());
            self.rail.set(target);
        }
    }
    fn enter(&mut self) {
        // 26H2 page entrance; the page marker follows the settled page.
        if self.page_motion && !self.controls.reduce_motion() {
            self.entrance = Some(self.last_tick);
        } else {
            self.entrance = None;
        }
        self.scroll.set(0);
        self.scroll_grab = None;
        self.popup = None;
        self.focus = None;
        self.dialog = None;
        self.highlight = None;
        self.open = catalog::default_open(self.page);
        if self.page == Page::Typing {
            self.input_query = true;
        }
        // Administration providers are read again on each visit.
        self.admin.leave();
        if matches!(
            self.page,
            Page::OtherUsers | Page::OtherUser | Page::UsersGroups | Page::Accounts
        ) {
            self.load_users();
        }
        if self.entrance.is_none() {
            self.report_page();
        }
    }
    /// The entrance's remaining offset (epx): the page rises
    /// `ENTRANCE_RISE` over `motion.page_enter`.
    fn entrance_offset(&self) -> i32 {
        let Some(start) = self.entrance else {
            return 0;
        };
        let motion = door_design_tokens::motion(door_design_tokens::MotionToken::PageEnter);
        let ms = self.last_tick.saturating_sub(start) * 10;
        let duration = u64::from(motion.duration_ms.max(1));
        if ms >= duration {
            return 0;
        }
        let eased = motion.easing.sample((ms * 1000 / duration) as u32);
        (ENTRANCE_RISE * (1000 - eased as i32)) / 1000
    }
    /// Other users: the local accounts besides yours, from the session
    /// service's account book.
    fn load_users(&mut self) {
        let accounts =
            door_service_client::Credentials::connect(self.registry).and_then(|mut c| c.accounts());
        match accounts {
            Ok(accounts) => {
                self.admin.own_account = accounts
                    .iter()
                    .find(|a| a.uid == self.uid)
                    .map(|a| (String::from(a.name()), a.administrator));
                self.admin.users = accounts
                    .iter()
                    .filter(|a| a.uid != self.uid)
                    .map(|a| (a.uid, String::from(a.name()), a.administrator))
                    .collect();
                if self
                    .admin
                    .user_selected
                    .is_some_and(|uid| !self.admin.users.iter().any(|u| u.0 == uid))
                {
                    self.admin.user_selected = None;
                }
            }
            Err(_) => {
                self.admin.users.clear();
                self.admin.own_account = None;
            }
        }
    }
    /// The open account page's name.
    fn selected_user_name(&self) -> Option<&str> {
        let uid = self.admin.user_selected?;
        self.admin
            .users
            .iter()
            .find(|u| u.0 == uid)
            .map(|u| u.1.as_str())
    }
    fn go(&mut self, page: Page) {
        if self.page != page {
            if self.history.len() == 32 {
                self.history.remove(0);
            }
            self.history.push(self.page);
            self.future.clear();
            self.page = page;
            self.enter();
        }
        self.search_focus = false;
        self.query.clear();
        self.reveal_page();
    }
    /// Search results and shell deep links always navigate, like Windows:
    /// choosing the current page reloads it (scroll, focus and expanders
    /// return to their defaults and the page is reported again).
    fn navigate(&mut self, page: Page) {
        if page == self.page {
            self.enter();
        }
        self.go(page);
    }
    /// Navigate to `page` and, for a section, open the section that holds
    /// it, scroll it into view and highlight it, as Windows does for a
    /// search result.
    fn navigate_to(&mut self, page: Page, anchor: Option<&str>) {
        self.navigate(page);
        // A deep link lands at its section at once (the highlight shows it).
        if anchor.is_some() && self.entrance.take().is_some() {
            self.report_page();
        }
        let Some(section) = anchor.and_then(|a| door_settings_map::section(page, a)) else {
            return;
        };
        if let Some(id) = catalog::section_expander(page, section.title)
            && id < 64
        {
            self.open |= 1 << id;
        }
        if let Some(r) = catalog::section_rect(&self.page_layout(), section.title) {
            let top = (r.y - Layout::content_top() - 16).max(0) as u32;
            self.scroll.set(top.min(self.max_scroll()));
            self.highlight = Some((page, section.title, self.last_tick + HIGHLIGHT_TICKS));
            let _ = door_sys::write(
                format!(
                    "DOOR_P14 SETTINGS SECTION page={} anchor={}\n",
                    page as usize, section.anchor
                )
                .as_bytes(),
            );
        }
    }
    fn report_page(&self) {
        let _ = door_sys::write(
            format!("DOOR_P12 SETTINGS PAGE id={}\n", self.page as usize).as_bytes(),
        );
    }
    fn back(&mut self) {
        if let Some(page) = self.history.pop() {
            self.future.push(self.page);
            self.page = page;
            self.enter();
        }
        self.search_focus = false;
        self.reveal_page();
    }
    fn forward(&mut self) {
        if let Some(page) = self.future.pop() {
            self.history.push(self.page);
            self.page = page;
            self.enter();
            self.reveal_page();
        }
    }
    /// Shell deep links (SETTINGS_NAVIGATE): a `Page::ALL` route index in
    /// the low 16 bits and optionally a section (`door_settings_map::
    /// navigate_arg`), e.g. Display settings, `sysdm.cpl,,3`, `ncpa.cpl`.
    /// Read the current page's providers if this visit hasn't yet (a deep
    /// link arrives without input). True when something was read.
    /// Installed apps: start a pending Modify or Uninstall; while the page
    /// shows, read the registered programs again every two seconds (an
    /// uninstaller removes its entry when it is done). True when the page
    /// changed.
    pub fn programs_tick(
        &mut self,
        storage: &mut Storage,
        c: &mut door_service_client::window::WindowClient,
        now: u64,
    ) -> bool {
        if let Some((program, arguments)) = self.pending_program.take() {
            if let Err(error) = self.admin.start_program(storage, c, &program, &arguments) {
                self.log(format!(
                    "Installed apps: {program} could not start ({error:?})"
                ));
            }
            self.programs_next = 0;
        }
        if self.page != Page::InstalledApps || now < self.programs_next {
            return false;
        }
        let tick = door_sys::query().map_or(1000, |q| q.tick_hz.max(1));
        self.programs_next = now + 2 * tick;
        let programs = door_workspace::installed_programs::list(storage);
        if programs == self.admin.programs {
            return false;
        }
        self.admin.programs = programs;
        true
    }
    pub fn ensure_providers(&mut self, storage: &mut Storage) -> bool {
        let fresh = self.admin.ensure(self.page, storage);
        // Proxy: the switch shows what Internet Settings hold now.
        if fresh && self.page == Page::Proxy {
            self.proxy_auto_detect = door_workspace::proxy::read_automatic(storage).0;
        }
        fresh
    }
    pub fn navigate_requested(&mut self, argument: u32) -> Result<(), Error> {
        let (page, section) =
            door_settings_map::decode_navigate(argument).ok_or(Error::Malformed)?;
        self.navigate_to(page, section.map(|s| s.anchor));
        Ok(())
    }
    /// Item under (x, y) in content coordinates, last match winning (a
    /// toggle inside a navigating card).
    fn item_at(&self, x: i32, y: i32) -> Option<usize> {
        let layout = self.page_layout();
        layout
            .items
            .iter()
            .enumerate()
            .rev()
            .find(|(_, item)| item.rect.contains(x, y))
            .map(|(index, _)| index)
    }
    pub fn target(&self, x: i32, y: i32) -> Option<u32> {
        if x < 0 || y < 0 || x >= self.layout.width as i32 || y >= self.layout.height as i32 {
            return None;
        }
        if let Some((popup, _)) = &self.popup {
            return Some(match popup.hit(x, y) {
                Some(index) => POPUP_OPTION + index as u32,
                None => POPUP_BACKGROUND,
            });
        }
        // A dialog is modal: only its own fields and buttons are targets.
        if let Some(d) = &self.dialog {
            return Some(d.hit(self.layout.width, self.layout.height, x, y));
        }
        if self.suggestions_open() && self.suggestion_list_contains(x, y) {
            return Some(
                self.suggestion_at(x, y)
                    .map_or(SUGGESTION_LIST, |i| SUGGESTION + i as u32),
            );
        }
        if self.layout.back_contains(x, y) {
            return Some(BACK);
        }
        if let Some(s) = self.layout.search()
            && s.contains(x, y)
        {
            return Some(SEARCH);
        }
        if y < Layout::TITLE {
            return None;
        }
        // Page heading ancestors (route parents, not Back history).
        let (page, crumbs) = self.heading_hit.get();
        if page == Some(self.page) && (HEADING_Y - 4..HEADING_Y + heading_height()).contains(&y) {
            for (i, crumb) in crumbs.iter().enumerate() {
                if let Some((_, hx, hw)) = crumb
                    && (*hx..*hx + *hw as i32).contains(&x)
                {
                    return Some(ANCESTOR + i as u32);
                }
            }
        }
        let rail = self.rail();
        if (Layout::RAIL_BAR - 2..Layout::RAIL_BAR + 6).contains(&x)
            && (rail.top..rail.bottom()).contains(&y)
            && rail.max_scroll(self.items().count()) > 0
        {
            return Some(RAIL_BAR);
        }
        if let Some(row) = rail.hit(x, y, self.items().count()) {
            return self.items().nth(row).map(|p| 100 + p as u32);
        }
        if self
            .layout
            .scrollbar(self.scroll.value, self.max_scroll())
            .is_some()
            && x >= self.layout.width as i32 - 14
            && (Layout::content_top()..self.layout.height as i32 - 20).contains(&y)
        {
            return Some(MAIN_BAR);
        }
        if !self.layout.main_contains(x, y) {
            return None;
        }
        let y = y + self.scroll.value as i32;
        self.item_at(x, y).map(|index| ITEM + index as u32)
    }
    pub fn click(&mut self, x: i32, y: i32, storage: &mut Storage) -> Result<bool, Error> {
        self.click_with(x, y, Some(storage))
    }
    /// Pointer activation. Host reviews pass `None`: persistence is skipped,
    /// every other effect (state, diagnostics, queued shell updates) is real.
    pub fn click_with(
        &mut self,
        x: i32,
        y: i32,
        mut storage: Option<&mut Storage>,
    ) -> Result<bool, Error> {
        if let Some((popup, kind)) = self.popup {
            self.popup = None;
            if let Some(index) = popup.hit(x, y) {
                if popup.enabled(index) {
                    self.choose(kind, index, storage.as_deref_mut())?;
                } else {
                    // Disabled options keep the list open, as in WinUI.
                    self.popup = Some((popup, kind));
                }
            }
            return Ok(true);
        }
        let target = self.target(x, y);
        if self.dialog.is_some() {
            if let Some(id) = target {
                self.dialog_activate(id, x, storage.as_deref_mut())?;
            }
            if let Some(storage) = storage {
                self.admin.ensure(self.page, storage);
            }
            return Ok(true);
        }
        let Some(id) = target else {
            let changed = self.search_focus || self.focus.is_some();
            self.search_focus = false;
            self.focus = None;
            return Ok(changed);
        };
        if id != SEARCH && !(SUGGESTION_LIST..SUGGESTION + 8).contains(&id) {
            self.search_focus = false;
        }
        self.activate(id, storage.as_deref_mut())?;
        if let Some(storage) = storage {
            self.admin.ensure(self.page, storage);
        }
        let max = self.max_scroll();
        self.scroll.clamp(max);
        let rail_max = self.rail().max_scroll(self.items().count());
        self.rail.clamp(rail_max);
        Ok(true)
    }
    fn activate(&mut self, id: u32, storage: Option<&mut Storage>) -> Result<(), Error> {
        match id {
            100..=148 => {
                if let Some(page) = Page::from_index((id - 100) as usize) {
                    self.go(page);
                }
            }
            SEARCH => {
                self.search_focus = true;
                self.focus = None;
            }
            BACK => self.back(),
            FORWARD => self.forward(),
            SUGGESTION..=157 => {
                let hit = self.suggestions().nth((id - SUGGESTION) as usize);
                if let Some(hit) = hit {
                    let (page, anchor) = hit.target();
                    self.navigate_to(page, anchor);
                }
            }
            SUGGESTION_LIST => (),
            ANCESTOR..=261 => {
                let (_, crumbs) = self.heading_hit.get();
                if let Some(Some((page, _, _))) = crumbs.get((id - ANCESTOR) as usize) {
                    self.go(*page);
                }
            }
            id if (ITEM..POPUP_BACKGROUND).contains(&id) => {
                let index = (id - ITEM) as usize;
                self.focus = Some(index);
                self.keyboard_focus = false;
                self.act(index, storage)?;
            }
            _ => (),
        }
        Ok(())
    }
    /// Whether a card button's command can act now (a disabled button shows
    /// why by its state: Up at a root key, New value without a key).
    fn command_enabled(&self, command: catalog::Command) -> bool {
        match command {
            catalog::Command::RegistryUp => self.admin.location.parent().is_some(),
            catalog::Command::RegistryNewValue => self.admin.snapshot.is_some(),
            catalog::Command::EditIp
            | catalog::Command::EditDns
            | catalog::Command::NetworkReset => self.net.is_some_and(|n| n.present),
            catalog::Command::EditProxy | catalog::Command::EditSetupScript => true,
            catalog::Command::TestSound => self.audio.is_some_and(|a| a.present),
            _ => true,
        }
    }
    /// A card button: open Task Manager or Terminal, run checks, refresh
    /// the page's providers, or open the dialog that collects input.
    fn command(&mut self, command: catalog::Command, storage: Option<&mut Storage>) {
        use catalog::Command as C;
        if !self.command_enabled(command) {
            return;
        }
        let opener = self.focus;
        match command {
            C::ApplyContrast | C::ApplyTextSize => (),
            C::Refresh => {
                if let Some(storage) = storage {
                    self.admin.refresh(self.page, storage);
                }
            }
            C::OpenTaskManager => self.pending_launch = Some(11),
            C::OpenTerminal => self.pending_launch = Some(9),
            C::RestoreFirewall => {
                self.pending_firewall = Some(alloc::vec![
                    door_service_protocol::firewall::change::RESTORE
                ]);
            }
            C::CustomizeContextMenu => self.open_customizer = true,
            C::RunChecks => {
                if let Some(storage) = storage {
                    self.admin.run_checks(storage);
                    let passed = self.admin.checks.iter().flatten().filter(|c| c.0).count();
                    self.log(format!("System check: {passed} of 3 passed"));
                }
            }
            C::RegistryUp => self.admin.registry_up(storage),
            C::RegistryGoTo => {
                let mut d = dialog::Dialog::new(
                    DialogKind::RegistryGoTo,
                    "Go to a key",
                    dialog::Target::None,
                    opener,
                );
                d.fields[0].set(self.admin.location.display(), true);
                self.dialog = Some(d);
            }
            C::RegistryDeleteKey => {
                self.dialog = Some(dialog::Dialog::new(
                    DialogKind::RegistryDeleteKey,
                    "Delete this key?",
                    dialog::Target::None,
                    opener,
                ));
            }
            C::RegistryExport => {
                if let Some(storage) = storage {
                    let location = self.admin.location.clone();
                    let event =
                        match door_workspace::registry_browser::export(storage, &location, 4) {
                            Ok(text) => {
                                let leaf = location.display();
                                let leaf = leaf.rsplit('\\').next().unwrap_or("registry");
                                let safe: String = leaf
                                    .chars()
                                    .map(|c| {
                                        if c.is_ascii_alphanumeric() || c == '-' {
                                            c
                                        } else {
                                            '_'
                                        }
                                    })
                                    .take(32)
                                    .collect();
                                let path = format!("/users/{}/Documents/{safe}.reg", self.uid);
                                match write_whole_file(storage, &path, text.as_bytes()) {
                                    Ok(()) => {
                                        let _ = door_sys::write(
                                            format!("DOOR_P14 ADMIN REGISTRY EXPORTED {path}\n")
                                                .as_bytes(),
                                        );
                                        format!("Registry exported: {path}")
                                    }
                                    Err(_) => String::from("Registry export couldn't be saved"),
                                }
                            }
                            Err(e) => format!("Registry export failed: {}", admin::reason(e)),
                        };
                    self.log(event);
                }
            }
            C::RegistryNewValue => {
                if let Some(snapshot) = self.admin.snapshot.clone() {
                    self.dialog = Some(dialog::Dialog::new(
                        DialogKind::RegistryValue,
                        "New value",
                        dialog::Target::Registry(snapshot, None),
                        opener,
                    ));
                }
            }
            C::NewVariable => {
                self.dialog = Some(dialog::Dialog::new(
                    DialogKind::Variable,
                    "New user variable",
                    dialog::Target::Variable(None),
                    opener,
                ));
            }
            C::CheckAccess => {
                let mut d = dialog::Dialog::new(
                    DialogKind::CheckAccess,
                    "Check access",
                    dialog::Target::None,
                    opener,
                );
                let path = self
                    .admin
                    .checked
                    .as_ref()
                    .map_or_else(|| format!("/users/{}", self.uid), |(p, _)| p.clone());
                d.fields[0].set(path, true);
                self.dialog = Some(d);
            }
            C::ClearClipboard => {
                // The window server clears the history when it goes off;
                // it comes straight back on (pinned items are kept).
                let word = self
                    .state()
                    .pref(door_service_protocol::preferences::CLIPBOARD);
                self.pending_prefs
                    .push((door_service_protocol::preferences::CLIPBOARD, word & !1));
                self.pending_prefs
                    .push((door_service_protocol::preferences::CLIPBOARD, word));
                self.log(String::from("Clipboard: cleared"));
            }
            C::ChangePin => {
                let (kind, title) = if self.pin_enrolled == Some(true) {
                    (DialogKind::Pin, "Change your PIN")
                } else {
                    (DialogKind::AddPin, "Set up a PIN")
                };
                self.dialog = Some(dialog::Dialog::new(
                    kind,
                    title,
                    dialog::Target::None,
                    opener,
                ));
            }
            C::RenamePc => {
                let mut d = dialog::Dialog::new(
                    DialogKind::RenamePc,
                    "Rename your PC",
                    dialog::Target::None,
                    opener,
                );
                d.fields[0].set(self.device_name.clone(), true);
                self.dialog = Some(d);
            }
            C::ChangePassword => {
                self.dialog = Some(dialog::Dialog::new(
                    DialogKind::Password,
                    "Change your password",
                    dialog::Target::None,
                    opener,
                ));
            }
            C::AddAccount => {
                self.dialog = Some(dialog::Dialog::new(
                    DialogKind::AddAccount,
                    "Create a user for this PC",
                    dialog::Target::None,
                    opener,
                ));
            }
            C::ChangeAccountType if self.admin.user_selected.is_some() => {
                self.dialog = Some(dialog::Dialog::new(
                    DialogKind::AccountType,
                    "Change account type",
                    dialog::Target::None,
                    opener,
                ));
            }
            C::RemoveAccount if self.admin.user_selected.is_some() => {
                self.dialog = Some(dialog::Dialog::new(
                    DialogKind::RemoveAccount,
                    "Delete account and data?",
                    dialog::Target::None,
                    opener,
                ));
            }
            C::ChangeAccountType | C::RemoveAccount => (),
            C::EditIp | C::EditDns => {
                let Some(net) = self.net.filter(|n| n.present) else {
                    self.log(String::from("No network adapter was found"));
                    return;
                };
                let ip = |a: [u8; 4]| format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3]);
                let (kind, title, values) = if command == C::EditIp {
                    let values = net
                        .config
                        .fixed
                        .map_or([String::new(), String::new()], |f| {
                            [format!("{}/{}", ip(f.0), f.1), ip(f.2)]
                        });
                    (DialogKind::IpSettings, "Edit IP settings", values)
                } else {
                    let d = net.config.dns;
                    let show = |a: [u8; 4]| if a == [0; 4] { String::new() } else { ip(a) };
                    (
                        DialogKind::DnsSettings,
                        "Edit DNS settings",
                        [show(d[0]), show(d[1])],
                    )
                };
                let mut d = dialog::Dialog::new(kind, title, dialog::Target::None, opener);
                let [first, second] = values;
                d.fields[0].set(first, true);
                d.fields[1].set(second, false);
                self.dialog = Some(d);
            }
            C::EditProxy => {
                let current = storage.and_then(door_workspace::proxy::read);
                let mut d = dialog::Dialog::new(
                    DialogKind::Proxy,
                    "Edit proxy server",
                    dialog::Target::None,
                    opener,
                );
                let (host, port) = current.map_or((String::new(), String::new()), |p| {
                    (p.host, format!("{}", p.port))
                });
                d.fields[0].set(host, true);
                d.fields[1].set(port, false);
                self.dialog = Some(d);
            }
            C::EditSetupScript => {
                let (_, script) =
                    storage.map_or((true, None), door_workspace::proxy::read_automatic);
                let mut d = dialog::Dialog::new(
                    DialogKind::SetupScript,
                    "Edit setup script",
                    dialog::Target::None,
                    opener,
                );
                d.fields[0].set(script.unwrap_or_default(), true);
                self.dialog = Some(d);
            }
            C::TestSound => {
                self.pending_test = true;
                self.log(String::from("Speakers: test tone"));
            }
            C::AddVpn => {
                self.dialog = Some(dialog::Dialog::new(
                    DialogKind::AddVpn,
                    "Add a VPN connection",
                    dialog::Target::None,
                    opener,
                ));
            }
            C::NetworkReset => {
                if self.net.is_some_and(|n| n.present) {
                    self.pending_net = Some(door_service_protocol::net::Config::default());
                    self.log(String::from("Network reset: automatic (DHCP) settings"));
                }
            }
        }
    }
    /// List entry `index`: open the subkey, or edit the value or variable.
    fn open_entry(&mut self, list: List, index: usize, storage: Option<&mut Storage>) {
        // Another user's page: Change account type and Remove.
        if list == List::OtherUsers {
            if let Some((uid, _, _)) = self.admin.users.get(index) {
                self.admin.user_selected = Some(*uid);
                self.go(Page::OtherUser);
            }
            return;
        }
        // A VPN connection's Advanced options: its own page.
        if list == List::VpnConnections {
            if let Some((name, _, _)) = self.admin.vpns.get(index) {
                self.admin.vpn_selected = Some(name.clone());
                self.go(Page::VpnConnection);
            }
            return;
        }
        if list == List::RegistryKeys {
            self.admin.open_subkey(index, storage);
        }
    }
    fn edit_entry(&mut self, list: List, index: usize, opener: Option<usize>) {
        if list == List::VpnConnections {
            if let Some((name, _, up)) = self.admin.vpns.get(index) {
                let op = if *up {
                    VpnOp::Disconnect(name.clone())
                } else {
                    VpnOp::Connect(name.clone())
                };
                self.pending_vpn.push(op);
            }
            return;
        }
        if list == List::FileTypes {
            // Default apps: the apps that can open the type, as a menu at
            // the row's button (the current default first).
            let count = self.admin.file_type_choices(index).len();
            let layout = self.page_layout();
            let Some(item) = opener.and_then(|i| layout.items.get(i)).copied() else {
                return;
            };
            if count > 0 {
                let button = self.to_window(item.rect);
                self.popup = Some((
                    Popup::menu(
                        button,
                        280,
                        count,
                        (1u64 << count.min(63)) - 1,
                        Layout::TITLE,
                        self.layout.height as i32,
                    ),
                    PopupKind::DefaultApp(index),
                ));
            }
            return;
        }
        let (snapshot, kind, title) = match list {
            List::RegistryValues => (
                self.admin.snapshot.clone(),
                DialogKind::RegistryEdit,
                "Edit value",
            ),
            List::UserVariables => (
                self.admin.variables.clone(),
                DialogKind::VariableEdit,
                "Edit user variable",
            ),
            _ => return,
        };
        let Some(snapshot) = snapshot else {
            return;
        };
        let Some(entry) = snapshot.values.get(index).copied() else {
            return;
        };
        let name = door_workspace::registry_browser::display_units(entry.name.units());
        let data = door_workspace::registry_value::format(entry.value);
        let target = if list == List::RegistryValues {
            dialog::Target::Registry(snapshot, Some(entry.name.units().to_vec()))
        } else {
            dialog::Target::Variable(Some(name.clone()))
        };
        let mut d = dialog::Dialog::new(kind, title, target, opener).editing(name, data);
        // Registry values can be renamed: the name stays editable.
        d.fixed_name = kind != DialogKind::RegistryEdit;
        d.value_kind = entry.value.kind;
        self.dialog = Some(d);
    }
    /// The current local time for the event log, in the regional format.
    fn event_time(&self) -> String {
        let region = door_locale::region::region_or_default(self.state().region);
        match pages::local_now(self.state().zone) {
            Some(now) => {
                let mut s = String::new();
                let _ = region.short_time(&now, &mut s);
                s
            }
            None => String::from("This session"),
        }
    }
    /// Record a Settings change in the session event log.
    fn log(&mut self, text: String) {
        let time = self.event_time();
        self.admin.log(time, text);
    }
    /// Perform content item `index` of the current page.
    fn act(&mut self, index: usize, mut storage: Option<&mut Storage>) -> Result<(), Error> {
        let layout = self.page_layout();
        let Some(item) = layout.items.get(index).copied() else {
            return Ok(());
        };
        let state = self.state();
        self.acting = semantics::row_title(&layout, &item);
        match item.action {
            Action::Navigate(page) => self.go(page),
            Action::Toggle(toggle) => {
                if let Some(change) = toggle.flip(state) {
                    self.apply(change, storage.as_deref_mut())?;
                    if let Some(title) = self.acting {
                        let on = toggle.value(self.state());
                        self.log(format!("{title}: {}", if on { "On" } else { "Off" }));
                    }
                }
            }
            Action::ListOpen(list, entry) => self.open_entry(list, entry, storage),
            Action::ListRemove(_, entry) => {
                if let Some((name, _, _)) = self.admin.vpns.get(entry) {
                    self.pending_vpn.push(VpnOp::Remove(name.clone()));
                    self.log(format!("VPN: removed {name}"));
                }
            }
            Action::ListEdit(list, entry) => self.edit_entry(list, entry, Some(index)),
            Action::Command(
                command @ (catalog::Command::Refresh
                | catalog::Command::OpenTaskManager
                | catalog::Command::OpenTerminal
                | catalog::Command::RunChecks
                | catalog::Command::RegistryGoTo
                | catalog::Command::RegistryUp
                | catalog::Command::RegistryNewValue
                | catalog::Command::NewVariable
                | catalog::Command::CheckAccess
                | catalog::Command::ClearClipboard
                | catalog::Command::ChangePin
                | catalog::Command::ChangePassword
                | catalog::Command::AddAccount
                | catalog::Command::ChangeAccountType
                | catalog::Command::RemoveAccount
                | catalog::Command::RenamePc
                | catalog::Command::RegistryDeleteKey
                | catalog::Command::RegistryExport
                | catalog::Command::CustomizeContextMenu
                | catalog::Command::EditIp
                | catalog::Command::EditDns
                | catalog::Command::EditProxy
                | catalog::Command::EditSetupScript
                | catalog::Command::NetworkReset
                | catalog::Command::TestSound
                | catalog::Command::AddVpn
                | catalog::Command::RestoreFirewall),
            ) => self.command(command, storage),
            Action::Combo(choice) => {
                let field = self.to_window(item.rect);
                self.popup = Some((
                    Popup::combo(
                        field,
                        choice.count(state),
                        choice.current(state),
                        choice.mask(state),
                        Layout::TITLE,
                        self.layout.height as i32,
                    ),
                    PopupKind::Choice(choice),
                ));
            }
            Action::Theme(theme) => {
                // A theme sets its picture (or its album's slideshow), mode
                // and accent together, as applying a .theme file does.
                use door_service_protocol::preferences::{self as p, Background};
                let def = door_workspace::settings_layout::THEMES[theme];
                self.save_appearance(def.appearance(), false, storage.as_deref_mut())?;
                self.apply(Change::Accent(def.accent_bits()), storage.as_deref_mut())?;
                let mut word = Background::from_bits(self.state().pref(p::BACKGROUND))
                    .unwrap_or(Background::DEFAULT);
                word.kind = if def.slideshow { 2 } else { 0 };
                if def.slideshow {
                    // [Slideshow] Interval=1800000: every 30 minutes, in order.
                    word.interval = 2;
                    word.shuffle = false;
                }
                self.apply(
                    Change::Preference(p::BACKGROUND, word.bits()),
                    storage.as_deref_mut(),
                )?;
                self.remember_background(def.background, storage.as_deref_mut())?;
                self.log(format!("Theme: {}", def.name));
            }
            Action::Background(slot) => {
                use door_service_protocol::preferences::{
                    self as p, Background, RecentBackgrounds,
                };
                let recent = RecentBackgrounds::from_bits(self.state().pref(p::BACKGROUND_RECENT))
                    .unwrap_or(RecentBackgrounds::DEFAULT);
                let Some(&background) = recent.0.get(slot as usize) else {
                    return Ok(());
                };
                let appearance = (self.appearance & 1) | (background << 1);
                self.save_appearance(appearance, false, storage.as_deref_mut())?;
                // A picture from Recent images shows as a Picture background.
                let mut word = Background::from_bits(self.state().pref(p::BACKGROUND))
                    .unwrap_or(Background::DEFAULT);
                if word.kind != 0 {
                    word.kind = 0;
                    self.apply(
                        Change::Preference(p::BACKGROUND, word.bits()),
                        storage.as_deref_mut(),
                    )?;
                }
                self.remember_background(background, storage.as_deref_mut())?;
                if let Some(name) =
                    door_workspace::settings_layout::BACKGROUNDS.get(background as usize)
                {
                    self.log(format!("Background: {name}"));
                }
            }
            Action::SolidColor(index) => {
                use door_service_protocol::preferences::{self as p, Background};
                if let Some(&rgb) = catalog::SOLID_PALETTE.get(index) {
                    let mut word = Background::from_bits(self.state().pref(p::BACKGROUND))
                        .unwrap_or(Background::DEFAULT);
                    word.kind = 1;
                    word.color = rgb;
                    self.apply(
                        Change::Preference(p::BACKGROUND, word.bits()),
                        storage.as_deref_mut(),
                    )?;
                    self.log(format!("Background color: #{rgb:06X}"));
                }
            }
            Action::Accent(index) => self.pick_accent(index as u32, storage.as_deref_mut())?,
            Action::RecentAccent(slot) => {
                if let Some(index) = self.recent_accents().colors().nth(slot) {
                    self.pick_accent(index, storage.as_deref_mut())?;
                }
            }
            Action::Expand(id) => self.open ^= 1 << id,
            Action::Command(catalog::Command::ApplyContrast) => {
                let controls = self
                    .controls
                    .with_contrast(catalog::picked_theme(self.contrast_pick));
                self.save_controls(storage.as_deref_mut(), controls.bits())?;
                let state = self.state();
                let label = Choice::Contrast.label(Choice::Contrast.current(state), state);
                self.log(format!("Contrast theme: {label}"));
            }
            Action::Command(catalog::Command::ApplyTextSize) => {
                let controls = self.controls.with_text_percent(self.text_pick);
                self.save_controls(storage, controls.bits())?;
                self.log(format!("Text size: {}%", self.text_pick));
            }
            Action::Slider(_) => {}
            Action::AppMenu(app) => {
                let button = self.to_window(item.rect);
                let (count, mask) = self.app_menu(app);
                self.popup = Some((
                    Popup::menu(
                        button,
                        200,
                        count,
                        mask,
                        Layout::TITLE,
                        self.layout.height as i32,
                    ),
                    PopupKind::AppMenu(app),
                ));
                self.menu_anchor = Some(button);
            }
            Action::KeyboardMenu(index) => {
                let button = self.to_window(item.rect);
                self.popup = Some((
                    Popup::menu(
                        button,
                        200,
                        catalog::KEYBOARD_MENU.len(),
                        catalog::keyboard_menu_mask(state, index),
                        Layout::TITLE,
                        self.layout.height as i32,
                    ),
                    PopupKind::KeyboardMenu(index),
                ));
            }
            Action::KeyboardAdd => {
                let n = catalog::addable(state).count();
                let full =
                    state.input_methods().count() >= door_keyboard_layouts::methods::MAX_METHODS;
                let button = self.to_window(item.rect);
                self.popup = Some((
                    Popup::menu(
                        button,
                        320,
                        n,
                        if full { 0 } else { (1 << n) - 1 },
                        Layout::TITLE,
                        self.layout.height as i32,
                    ),
                    PopupKind::AddKeyboard,
                ));
            }
        }
        Ok(())
    }
    /// System > Display > Scale list entries as shown.
    #[cfg(test)]
    pub fn scale_labels(&self) -> Vec<&'static str> {
        let (choice, state) = (catalog::Choice::Scale, self.state());
        (0..choice.count(state))
            .map(|i| choice.label(i, state))
            .collect()
    }
    /// Window rects of theme and background pictures and of the Contrast
    /// themes preview: Windows' contrast themes keep their own colours.
    #[cfg(test)]
    pub fn picture_rects(&self) -> Vec<(i32, i32, u32, u32)> {
        let layout = self.page_layout();
        let previews = layout.blocks.iter().filter_map(|b| match b.hero {
            Some((catalog::Hero::Contrast, r)) => Some(catalog::contrast_geometry(r).preview),
            _ => None,
        });
        layout
            .items
            .iter()
            .filter(|i| matches!(i.action, Action::Theme(_) | Action::Background(_)))
            .map(|i| i.rect)
            .chain(previews)
            .map(|r| {
                let r = self.to_window(r);
                (r.x, r.y, r.w, r.h)
            })
            .collect()
    }
    fn to_window(&self, r: catalog::Rect) -> catalog::Rect {
        catalog::Rect {
            y: r.y - self.scroll.value as i32,
            ..r
        }
    }
    fn choose(
        &mut self,
        kind: PopupKind,
        index: usize,
        mut storage: Option<&mut Storage>,
    ) -> Result<(), Error> {
        match kind {
            PopupKind::Choice(choice) => {
                let before = self.state();
                if let Some(change) = choice.apply(index, before) {
                    self.apply(change, storage.as_deref_mut())?;
                    // Selections that persist something are logged; a
                    // listed-only pick (Contrast themes before Apply) is not.
                    if !matches!(
                        change,
                        Change::ContrastPick(_) | Change::RegistryLocation(..)
                    ) && let Some(title) = self.acting
                    {
                        self.log(format!("{title}: {}", choice.label(index, before)));
                    }
                }
            }
            PopupKind::DialogType => {
                if let Some(d) = &mut self.dialog
                    && let Some(kind) = door_workspace::registry_value::KINDS.get(index)
                {
                    // An explicit type change keeps the data for validation;
                    // bytes are never coerced.
                    d.value_kind = *kind;
                    d.focus = dialog::Focus::Type;
                }
            }
            PopupKind::AppMenu(app) => {
                let entry = self.app_order().get(app).copied();
                if let Some(Ok(included)) = entry {
                    if index == 0 {
                        self.pending_launch =
                            door_workspace::settings_layout::installed_app_command(included);
                    }
                } else if let Some(program) = self.program_at(app).cloned() {
                    if index == 0 {
                        self.pending_program = program.command(true);
                        self.log(format!("Installed apps: Modify {}", program.name));
                    } else if let Some(button) = self.menu_anchor {
                        // Uninstall asks first, in a flyout under the button.
                        self.popup = Some((
                            Popup::menu(
                                button,
                                320,
                                pages::CONFIRM_UNINSTALL.len(),
                                0b10,
                                Layout::TITLE,
                                self.layout.height as i32,
                            ),
                            PopupKind::ConfirmUninstall(app),
                        ));
                    }
                }
            }
            PopupKind::ConfirmUninstall(app) => {
                if index == 1
                    && let Some(program) = self.program_at(app).cloned()
                {
                    self.pending_program = program.command(false);
                    let _ = door_sys::write(
                        format!("DOOR_P16 SETTINGS UNINSTALL app={}\n", program.name).as_bytes(),
                    );
                    self.log(format!("Installed apps: Uninstall {}", program.name));
                }
            }
            PopupKind::KeyboardMenu(keyboard) => {
                if let Some(change) = catalog::keyboard_menu_apply(self.state(), keyboard, index) {
                    self.apply(change, storage.as_deref_mut())?;
                    self.log(format!("Keyboards: {}", catalog::KEYBOARD_MENU[index]));
                }
            }
            PopupKind::AddKeyboard => {
                if let Some(change) = catalog::keyboard_add_apply(self.state(), index) {
                    self.apply(change, storage)?;
                    self.log(String::from("Keyboards: Added a keyboard"));
                }
            }
            PopupKind::DefaultApp(row) => {
                let choices = self.admin.file_type_choices(row);
                if let (Some((key, _)), Some(storage)) = (choices.get(index), storage)
                    && let Ok(event) = self.admin.set_file_type_default(row, key, storage)
                {
                    self.log(event);
                }
            }
        }
        Ok(())
    }
    /// Persist a change, update the view at once and queue it for the shell.
    fn apply(&mut self, change: Change, storage: Option<&mut Storage>) -> Result<(), Error> {
        match change {
            Change::Controls(c) => self.save_controls(storage, c.bits()),
            // Only "Choose your mode" produces this change: it reports `theme=`.
            Change::Appearance(appearance) => self.save_appearance(appearance, true, storage),
            Change::ContrastPick(pick) => {
                self.contrast_pick = pick;
                Ok(())
            }
            Change::Scale(percent) => {
                self.choose_scale(percent);
                Ok(())
            }
            Change::InputMethods(bits) => self.save_input(bits, storage),
            Change::TimeZone(id) => self.save_locale(Setting::TimeZone, id, storage),
            Change::RegionalFormat(id) => self.save_locale(Setting::RegionalFormat, id, storage),
            Change::Network(config) => {
                self.pending_net = Some(config);
                Ok(())
            }
            Change::RegistryLocation(root, view) => {
                self.admin.set_root(root, view, storage);
                Ok(())
            }
            Change::Preference(id, value) => self.save_preference(id, value, storage),
            Change::Accent(bits) => self.save_accent(bits, storage),
            Change::TaskbarSize(controls, taskbar) => {
                let mut storage = storage;
                self.save_preference(
                    door_service_protocol::preferences::TASKBAR,
                    taskbar,
                    storage.as_deref_mut(),
                )?;
                self.save_controls(storage, controls.bits())
            }
            Change::ProxyAutoDetect(on) => {
                if let Some(storage) = storage {
                    door_workspace::proxy::write_auto_detect(storage, on)?;
                }
                self.proxy_auto_detect = on;
                door_rt::diagnostic(format_args!(
                    "DOOR_P15 SETTINGS PROXY auto_detect={}\n",
                    u8::from(on)
                ));
                Ok(())
            }
            // The switch moves once an administrator approved and
            // door.network reports the new policy.
            Change::Firewall(profile, byte) => {
                self.pending_firewall = Some(alloc::vec![
                    door_service_protocol::firewall::change::PROFILES,
                    1 << profile,
                    byte
                ]);
                door_rt::diagnostic(format_args!(
                    "DOOR_P15 SETTINGS FIREWALL profile={profile} byte={byte:#x}\n"
                ));
                Ok(())
            }
        }
    }
    /// Persist preference word `id` and queue it for its live consumers.
    fn save_preference(
        &mut self,
        id: u32,
        value: u32,
        storage: Option<&mut Storage>,
    ) -> Result<(), Error> {
        if !door_service_protocol::preferences::valid(id, value) {
            return Err(Error::Service(Code::InvalidRequest));
        }
        if let Some(storage) = storage {
            let setting = Setting::from_id(id).ok_or(Error::Service(Code::InvalidRequest))?;
            storage.set_setting(setting, value)?;
            storage.flush()?;
        }
        if let Some(word) = self.prefs.get_mut((id - catalog::PREF_FIRST) as usize) {
            *word = value;
        }
        self.pending_prefs.push((id, value));
        let _ = door_sys::write(
            format!("DOOR_P14 SETTINGS PREFERENCE id={id} value={value}\n").as_bytes(),
        );
        Ok(())
    }
    /// `def` is the current theme: its picture, mode, accent and background
    /// kind all still apply (any later change makes the theme "Custom").
    pub(super) fn theme_current(&self, def: door_workspace::settings_layout::ThemeDef) -> bool {
        use door_service_protocol::preferences::{self as p, Background};
        let kind = Background::from_bits(self.state().pref(p::BACKGROUND))
            .unwrap_or(Background::DEFAULT)
            .kind;
        self.appearance == def.appearance()
            && self.accent == def.accent_bits()
            && (kind == 2) == def.slideshow
    }
    /// Move `background` to the front of "Recent images".
    fn remember_background(
        &mut self,
        background: u32,
        storage: Option<&mut Storage>,
    ) -> Result<(), Error> {
        use door_service_protocol::preferences::{self as p, RecentBackgrounds};
        let recent = RecentBackgrounds::from_bits(self.state().pref(p::BACKGROUND_RECENT))
            .unwrap_or(RecentBackgrounds::DEFAULT);
        let used = recent.used(background);
        if used == recent {
            return Ok(());
        }
        self.save_preference(p::BACKGROUND_RECENT, used.bits(), storage)
    }
    /// Persist the accent setting and queue it for the window server.
    fn save_accent(&mut self, bits: u32, storage: Option<&mut Storage>) -> Result<(), Error> {
        if door_service_protocol::desktop::Accent::from_bits(bits).is_none() {
            return Err(Error::Service(Code::InvalidRequest));
        }
        if let Some(storage) = storage {
            storage.set_setting(Setting::AccentColor, bits)?;
            storage.flush()?;
        }
        self.accent = bits;
        self.pending_accent = Some(bits);
        let _ = door_sys::write(format!("DOOR_P14 SETTINGS ACCENT bits={bits:#x}\n").as_bytes());
        Ok(())
    }
    /// The accent setting (`desktop::Accent` bits) as stored.
    pub fn accent(&self) -> u32 {
        self.accent
    }
    /// System > Display > Scale: queue the choice (0 automatic) for the
    /// window server, which applies it live. Only a scale it accepts is
    /// persisted (`scale_applied`); a refused one reverts (`scale_refused`).
    fn choose_scale(&mut self, percent: u32) {
        if self.pending_scale.is_none() {
            self.scale_stored = self.scale_setting;
        }
        self.scale_setting = percent;
        self.pending_scale = Some(percent);
    }
    /// Stored scale and whether the "Sign out to apply" note shows.
    #[cfg(test)]
    pub fn scale_state(&self) -> (u32, bool) {
        (self.scale_setting, self.scale_note)
    }
    #[cfg(test)]
    pub fn text_pick(&self) -> u32 {
        self.text_pick
    }
    /// Persist the appearance word. Mode changes report `theme=`; theme and
    /// background gallery choices report `appearance=` (guest scenarios).
    fn save_appearance(
        &mut self,
        appearance: u32,
        mode: bool,
        storage: Option<&mut Storage>,
    ) -> Result<(), Error> {
        if let Some(storage) = storage {
            // Through the typed registry (P14-REGISTRY): the mode or the
            // picture, validated and committed (`SETTINGS commit`).
            use door_settings_registry::key;
            match self.watch.as_mut() {
                Some(watch) if mode => {
                    watch.commit(storage, key::APPEARANCE_MODE, appearance & 1)?
                }
                Some(watch) => {
                    watch.commit(storage, key::APPEARANCE_BACKGROUND, appearance >> 1)?
                }
                None => {
                    storage.set_setting(Setting::DesktopAppearance, appearance)?;
                    storage.flush()?;
                }
            }
        }
        self.appearance = appearance;
        self.pending_theme = Some(appearance);
        let _ = door_sys::write(
            if mode {
                format!("DOOR_P12 SETTINGS SAVED theme={}\n", appearance & 1)
            } else {
                format!("DOOR_P12 SETTINGS SAVED appearance={appearance}\n")
            }
            .as_bytes(),
        );
        Ok(())
    }
    pub fn key(&mut self, code: u32, storage: &mut Storage) -> Result<bool, Error> {
        self.key_with(code, Some(storage))
    }
    pub fn key_with(
        &mut self,
        code: u32,
        mut storage: Option<&mut Storage>,
    ) -> Result<bool, Error> {
        let result = self.key_inner(code, storage.as_deref_mut());
        if let Some(storage) = storage {
            self.admin.ensure(self.page, storage);
        }
        let max = self.max_scroll();
        self.scroll.clamp(max);
        let rail_max = self.rail().max_scroll(self.items().count());
        self.rail.clamp(rail_max);
        result
    }
    /// Scroll so content rows [top, bottom) are visible.
    fn reveal_y(&mut self, top: i32, bottom: i32) {
        let view_top = self.scroll.value as i32 + Layout::content_top();
        let view_bottom = self.scroll.value as i32 + self.layout.height as i32 - 20;
        let next = if top < view_top {
            (top - Layout::content_top() - 8).max(0)
        } else if bottom > view_bottom {
            (bottom + 28 - self.layout.height as i32).max(0)
        } else {
            return;
        };
        let max = self.max_scroll();
        self.scroll.set((next as u32).min(max));
    }
    fn focus_item(&mut self, index: usize) {
        self.focus = Some(index);
        self.keyboard_focus = true;
        if let Some(item) = self.page_layout().items.get(index) {
            self.reveal_y(item.rect.y, item.rect.bottom());
        }
    }
    fn key_inner(&mut self, code: u32, mut storage: Option<&mut Storage>) -> Result<bool, Error> {
        let scan = code & 511;
        let modifiers = code >> 9;
        let shift = modifiers & door_abi::graphics::SHIFT != 0;
        let control = modifiers & door_abi::graphics::CONTROL != 0;
        let alt = modifiers & door_abi::graphics::ALT != 0;
        if let Some((mut popup, kind)) = self.popup {
            match popup.key(scan) {
                PopupKey::Moved => self.popup = Some((popup, kind)),
                PopupKey::Chosen(index) => {
                    self.popup = None;
                    self.choose(kind, index, storage.as_deref_mut())?;
                }
                PopupKey::Closed => self.popup = None,
                PopupKey::Ignored => return Ok(false),
            }
            return Ok(true);
        }
        if self.dialog.is_some() {
            return self.dialog_key(code, storage);
        }
        // F5 reads the page's providers again (never replays a change).
        if scan == 63 && !self.search_focus {
            if let Some(storage) = storage {
                self.admin.refresh(self.page, storage);
            }
            return Ok(true);
        }
        if alt && scan == 0x14b {
            self.back();
            return Ok(true);
        }
        if alt && scan == 0x14d {
            self.forward();
            return Ok(true);
        }
        // Ctrl+F: the title-bar search (none in a window too narrow for it).
        if control && scan == 33 {
            if self.layout.search().is_none() {
                return Ok(false);
            }
            self.search_focus = true;
            self.focus = None;
            self.suggestion = 0;
            return Ok(true);
        }
        if self.search_focus {
            return self.search_key(code);
        }
        if scan == 1 {
            let changed = self.focus.is_some() || !self.query.is_empty();
            self.focus = None;
            self.query.clear();
            return Ok(changed);
        }
        // A focused slider takes Home/End and Page Up/Down (WinUI) before
        // the page scrolls with them.
        if matches!(scan, 0x147 | 0x14f | 0x149 | 0x151)
            && let Some(focus) = self.focus
            && let Some(Action::Slider(slider)) =
                self.page_layout().items.get(focus).map(|i| i.action)
        {
            return Ok(self.slider_key(slider, scan));
        }
        if matches!(scan, 0x147 | 0x14f) {
            let target = if scan == 0x147 { 0 } else { self.max_scroll() };
            self.scroll_to(target);
            return Ok(true);
        }
        if matches!(scan, 0x149 | 0x151) {
            let page = self.layout.height.saturating_sub(160).max(120);
            let target = if scan == 0x149 {
                self.scroll.target.saturating_sub(page)
            } else {
                self.scroll
                    .target
                    .saturating_add(page)
                    .min(self.max_scroll())
            };
            self.scroll_to(target);
            return Ok(true);
        }
        let layout = self.page_layout();
        let count = layout.items.len();
        if scan == 15 {
            if count == 0 {
                return Ok(false);
            }
            let next = match self.focus.filter(|f| *f < count) {
                None if shift => count - 1,
                None => 0,
                Some(f) if shift => (f + count - 1) % count,
                Some(f) => (f + 1) % count,
            };
            self.focus_item(next);
            return Ok(true);
        }
        let Some(focus) = self.focus.filter(|f| *f < count) else {
            return self.rail_key(scan);
        };
        let item = layout.items[focus];
        if let Action::Slider(slider) = item.action
            && self.slider_key(slider, scan)
        {
            return Ok(true);
        }
        match scan {
            28 | 57 | 0x11c => {
                self.act(focus, storage.as_deref_mut())?;
                Ok(true)
            }
            0x148 | 0x150 => {
                let down = scan == 0x150;
                if let Action::Combo(choice) = item.action {
                    let before = self.state();
                    if let Some(index) = choice.step(before, down)
                        && let Some(change) = choice.apply(index, before)
                    {
                        self.apply(change, storage)?;
                        if !matches!(
                            change,
                            Change::ContrastPick(_) | Change::RegistryLocation(..)
                        ) && let Some(title) = semantics::row_title(&layout, &item)
                        {
                            self.log(format!("{title}: {}", choice.label(index, before)));
                        }
                    }
                    return Ok(true);
                }
                let next = if down {
                    (focus + 1).min(count - 1)
                } else {
                    focus.saturating_sub(1)
                };
                self.focus_item(next);
                Ok(true)
            }
            0x14b | 0x14d => {
                let next = if scan == 0x14d {
                    (focus + 1).min(count - 1)
                } else {
                    focus.saturating_sub(1)
                };
                self.focus_item(next);
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    fn scroll_to(&mut self, target: u32) {
        self.scroll.target = target;
        if self.reduce_motion() {
            self.scroll.value = target;
        }
    }
    /// Up/Down with nothing focused move through the rail (selection follows).
    fn rail_key(&mut self, scan: u32) -> Result<bool, Error> {
        if !matches!(scan, 0x148 | 0x150) {
            return Ok(false);
        }
        let pages: Vec<_> = self.items().collect();
        let current = self.page.category();
        let index = pages
            .iter()
            .position(|p| *p == self.page || *p == current)
            .unwrap_or(0);
        let next = if scan == 0x148 {
            (index + pages.len() - 1) % pages.len()
        } else {
            (index + 1) % pages.len()
        };
        self.go(pages[next]);
        Ok(true)
    }
    fn search_key(&mut self, code: u32) -> Result<bool, Error> {
        let scan = code & 511;
        let count = self.suggestions().count();
        match scan {
            1 => {
                self.search_focus = false;
                self.query.clear();
            }
            15 => self.search_focus = false,
            14 => {
                self.query.pop();
                self.suggestion = 0;
            }
            28 | 0x11c => {
                let hit = self
                    .suggestions()
                    .nth(self.suggestion.min(count.saturating_sub(1)));
                if let Some(hit) = hit {
                    let (page, anchor) = hit.target();
                    self.navigate_to(page, anchor);
                }
            }
            0x148 if count > 0 => self.suggestion = (self.suggestion + count - 1) % count,
            0x150 if count > 0 => self.suggestion = (self.suggestion + 1) % count,
            _ => {
                let ch = (code >> 16) as u8;
                if (32..=126).contains(&ch) && self.query.len() < 64 {
                    self.query.push(ch as char);
                    self.suggestion = 0;
                } else {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
    pub fn appearance_changed(&mut self, value: u32) {
        // While a contrast theme is on the shell reports light appearance so
        // every painter hands the palette its light colours; the user's own
        // mode stays what Settings shows and stores.
        let value = if self.controls.contrast().is_some() {
            (value & !1) | (self.appearance & 1)
        } else {
            value
        };
        if self.pending_theme.is_none_or(|pending| pending == value) {
            self.appearance = value;
        }
    }
    /// The desktop controls in effect (the context menu customizer builds its
    /// menus with them).
    pub fn controls(&self) -> Controls {
        self.controls
    }
    pub fn controls_changed(&mut self, value: Controls) {
        if self.pending_controls.is_none_or(|pending| pending == value) {
            self.controls = value;
            self.contrast_pick = catalog::contrast_pick(value);
            self.text_pick = value.text_percent();
        }
        // Every process reads presentation (contrast, text size) from the
        // last delivered controls.
        door_service_client::window::observe_desktop_controls(value);
    }
    fn save_controls(&mut self, storage: Option<&mut Storage>, bits: u32) -> Result<(), Error> {
        let controls = Controls::from_bits(bits).ok_or(Error::Service(Code::InvalidRequest))?;
        if let Some(storage) = storage {
            storage.set_setting(Setting::DesktopControls, bits)?;
            storage.flush()?;
        }
        self.controls = controls;
        self.pending_controls = Some(controls);
        let _ = door_sys::write(format!("DOOR_P12 SETTINGS SAVED controls={bits}\n").as_bytes());
        let _ = door_sys::write(
            format!(
                "DOOR_P12 SETTINGS CONTROLS bits={bits} page={:?}\n",
                self.page
            )
            .as_bytes(),
        );
        Ok(())
    }
    /// Large breadcrumb heading: muted actionable ancestors, chevrons, the
    /// bright current page (never itself a link).
    fn draw_heading(&self, v: &mut ui::View<'_>, t: &theme::Theme) {
        let style = TextStyle::Semibold28;
        let available = self.layout.main_width();
        let h = navigation::heading(self.page, Layout::MAIN, available, |text| {
            v.measure_style(text, style)
        });
        self.heading_hit.set((Some(self.page), h.ancestors));
        for (i, crumb) in h.ancestors.iter().enumerate() {
            let Some((page, x, width)) = *crumb else {
                continue;
            };
            let over = |p: Option<(i32, i32)>| {
                p.is_some_and(|(px, py)| {
                    (x..x + width as i32).contains(&px)
                        && (HEADING_Y - 4..HEADING_Y + heading_height()).contains(&py)
                })
            };
            let color = if over(v.pressed) {
                t.disabled
            } else if over(v.hover) {
                t.text
            } else {
                t.secondary
            };
            v.text_clip(x, HEADING_Y, width, page.nav_title(), style, color);
            v.mask_pixels(
                h.chevrons[i],
                HEADING_Y + 13 + Layout::heading_growth() / 2,
                16,
                Icon::ChevronRight.mask_scaled(16, v.scale),
                t.secondary,
                None,
            );
        }
        // A VPN connection's Advanced options are headed by its name.
        let user = self.selected_user_name();
        let current = match (self.page, self.admin.vpn_selected.as_deref(), user) {
            (Page::VpnConnection, Some(name), _) => name,
            (Page::OtherUser, _, Some(name)) => name,
            _ => self.page.nav_title(),
        };
        let room = h
            .current_width
            .max(v.measure_style(current, style))
            .min(available.saturating_sub((h.current_x - Layout::MAIN).max(0) as u32));
        v.text_clip(
            h.current_x,
            HEADING_Y,
            room,
            &pages::fitted(v, current, room, style),
            style,
            t.text,
        );
    }
    fn draw_title_bar(&self, v: &mut ui::View<'_>, t: &theme::Theme) {
        // Back (disabled without history, as in Windows) and the caption.
        let (bx, by, bw, bh) = self.layout.back();
        let enabled = !self.history.is_empty();
        let over = |p: Option<(i32, i32)>| p.is_some_and(|(x, y)| self.layout.back_contains(x, y));
        if enabled && over(v.hover) {
            v.rounded(
                bx,
                by,
                bw,
                bh,
                4,
                if over(v.pressed) {
                    t.rail_pressed
                } else {
                    t.rail_hover
                },
            );
        }
        v.mask_pixels(
            bx + (bw as i32 - 16) / 2,
            by + (bh as i32 - 16) / 2,
            16,
            Icon::Back.mask_scaled(16, v.scale),
            if enabled { t.text } else { t.disabled },
            None,
        );
        if self.layout.width >= 120 {
            v.text_clip_centered(48, (8, 32), 120, "Settings", TextStyle::Regular12, t.text);
        }
        if let Some(s) = self.layout.search() {
            pages::search_box(v, t, s, &self.query, self.search_focus);
        }
    }
    fn draw_account(&self, v: &mut ui::View<'_>, t: &theme::Theme) {
        if self.layout.height < 140 {
            return;
        }
        // 26H2 account block (owner capture): a 60 px avatar at x 19, the
        // name and the account line at x 91; the rail starts below it.
        pages::avatar(v, t, 19, 56, 60);
        v.text_clip(91, 69, 200, "Local account", TextStyle::Semibold14, t.text);
        v.text_clip(
            91,
            90,
            200,
            &format!("User {}", self.uid),
            TextStyle::Regular12,
            t.secondary,
        );
    }
    fn draw_rail(&self, v: &mut ui::View<'_>, t: &theme::Theme) {
        let rail = self.rail();
        if self.layout.height <= rail.top as u32 + 12 {
            return;
        }
        let selected = self.page.category();
        let count = self.items().count();
        v.viewport(
            door_graphics_core::Rect::new(
                0,
                rail.top,
                (Layout::MAIN as u32 - 6).min(v.width),
                self.layout.height - rail.top as u32 - 12,
            )
            .unwrap(),
            0,
        );
        for (row, page) in self.items().enumerate() {
            let y = rail.row(row);
            if y + Layout::nav_row() <= rail.top || y >= rail.bottom() {
                continue;
            }
            let x = Layout::NAV_X;
            let w = Layout::NAV_W;
            let hit = |p: Option<(i32, i32)>| {
                p.is_some_and(|(px, py)| rail.hit(px, py, count) == Some(row))
            };
            let fill = if selected == page {
                Some(if hit(v.pressed) {
                    t.rail_pressed
                } else {
                    t.rail_selected
                })
            } else if hit(v.pressed) {
                Some(t.rail_pressed)
            } else if hit(v.hover) {
                Some(t.rail_hover)
            } else {
                None
            };
            if let Some(fill) = fill {
                v.rounded(x, y, w, Layout::nav_row() as u32, 4, fill);
            }
            if selected == page {
                let (pw, ph) = Layout::NAV_PILL;
                v.rounded(
                    x,
                    y + (Layout::nav_row() - ph as i32) / 2,
                    pw,
                    ph,
                    door_design_tokens::radius(door_design_tokens::RadiusToken::Bar) as u32,
                    t.accent,
                );
            }
            category(
                v,
                page,
                x + Layout::NAV_ICON,
                y + (Layout::nav_row() - 20) / 2,
                20,
            );
            v.text_clip_centered(
                x + Layout::NAV_LABEL,
                (y, Layout::nav_row() as u32),
                w - Layout::NAV_LABEL as u32 - 8,
                page.nav_title(),
                TextStyle::Regular14,
                t.text,
            );
        }
        v.reset_viewport();
        // The 26H2 navigation scroll indicator is hidden at rest: it shows
        // while the pointer is over the rail or the thumb is dragged.
        let over = v
            .hover
            .is_some_and(|(x, y)| x < Layout::MAIN && (rail.top..rail.bottom()).contains(&y));
        if rail.max_scroll(count) > 0 && (over || self.rail_grab.is_some()) {
            let (top, thumb, _) = rail.thumb(count);
            v.rounded(Layout::RAIL_BAR - 1, top, 4, thumb as u32, 2, t.scrollbar);
        }
    }
    pub fn draw(&self, v: &mut ui::View<'_>) {
        // Over Mica (the window server composes it when the runtime paints
        // with materials) the page clears its background and its fills are
        // translucent; contrast themes keep their solid system colours.
        let solid = theme::theme(v.palette.dark);
        let t = if v.materials && v.palette.system().is_none() {
            theme::mica(solid)
        } else {
            solid
        };
        // A modal dialog keeps the page from reacting to the pointer.
        let (hover, pressed) = (v.hover, v.pressed);
        if self.dialog.is_some() {
            v.hover = None;
            v.pressed = None;
        }
        v.rect(0, 0, v.width, v.height, t.bg);
        self.draw_title_bar(v, &t);
        self.draw_account(v, &t);
        self.draw_rail(v, &t);
        if self.layout.has_main() {
            self.draw_heading(v, &t);
            if v.height > Layout::content_top() as u32 {
                v.viewport(
                    door_graphics_core::Rect::new(
                        Layout::MAIN - 8,
                        Layout::content_top(),
                        self.layout.main_width() + 16,
                        v.height.saturating_sub(Layout::content_top() as u32).max(1),
                    )
                    .expect("Settings viewport"),
                    self.scroll.value as i32 - self.entrance_offset(),
                );
                self.draw_page(v, &t);
                v.reset_viewport();
            }
            if let Some(geometry) = self.layout.scrollbar(self.scroll.value, self.max_scroll()) {
                let over = v
                    .hover
                    .is_some_and(|(x, _)| x >= self.layout.width as i32 - 14)
                    || self.scroll_grab.is_some();
                v.rounded(
                    v.width as i32 - if over { 11 } else { 9 },
                    geometry.top as i32,
                    if over { 4 } else { 2 },
                    geometry.thumb,
                    2,
                    t.scrollbar,
                );
            }
        }
        if self.suggestions_open() {
            pages::suggestions(
                v,
                &solid,
                self.layout,
                &self.suggestions().collect::<Vec<_>>(),
                self.suggestion,
            );
        }
        v.hover = hover;
        v.pressed = pressed;
        if let Some(d) = &self.dialog {
            d.draw(v, &solid, self.keyboard_focus, self.popup.is_some());
        }
        if let Some((popup, kind)) = &self.popup {
            let state = self.state();
            let owned: Vec<String> = match kind {
                PopupKind::DefaultApp(i) => self
                    .admin
                    .file_type_choices(*i)
                    .into_iter()
                    .map(|(_, name)| name)
                    .collect(),
                _ => Vec::new(),
            };
            let labels: Vec<&str> = match kind {
                PopupKind::DefaultApp(_) => owned.iter().map(String::as_str).collect(),
                PopupKind::Choice(choice) => (0..choice.count(state))
                    .map(|i| choice.label(i, state))
                    .collect(),
                PopupKind::AppMenu(app) => self.app_menu_labels(*app),
                PopupKind::ConfirmUninstall(_) => pages::CONFIRM_UNINSTALL.to_vec(),
                PopupKind::KeyboardMenu(_) => catalog::KEYBOARD_MENU.to_vec(),
                PopupKind::AddKeyboard => catalog::addable(state)
                    .map(|m| catalog::input_label(m.id))
                    .collect(),
                PopupKind::DialogType => dialog::Dialog::type_labels(),
            };
            let current = match kind {
                PopupKind::Choice(choice) => Some(choice.current(state)),
                PopupKind::DialogType => self.dialog.as_ref().map(dialog::Dialog::type_index),
                PopupKind::AppMenu(_)
                | PopupKind::ConfirmUninstall(_)
                | PopupKind::KeyboardMenu(_)
                | PopupKind::AddKeyboard
                | PopupKind::DefaultApp(_) => None,
            };
            pages::popup(v, &solid, popup, &labels, current);
        }
    }
    /// Keys while a dialog is open: Tab moves through its controls, Return
    /// presses the focused button (the primary one from a text box), Escape
    /// cancels, Space presses a focused button or opens the type list, and
    /// the arrows change the closed type list. Nothing reaches the page.
    fn dialog_key(&mut self, code: u32, storage: Option<&mut Storage>) -> Result<bool, Error> {
        let scan = code & 511;
        let shift = (code >> 9) & door_abi::graphics::SHIFT != 0;
        let Some(d) = self.dialog.as_mut() else {
            return Ok(false);
        };
        self.keyboard_focus = true;
        match scan {
            15 => d.tab(shift),
            1 => {
                let close = d.kind.buttons().len() - 1;
                self.dialog_press(close, storage);
            }
            28 | 0x11c => {
                let button = match d.focus {
                    dialog::Focus::Button(i) => i,
                    dialog::Focus::Type => {
                        self.open_type_list();
                        return Ok(true);
                    }
                    dialog::Focus::Field(_) => 0,
                };
                self.dialog_press(button, storage);
            }
            57 if matches!(d.focus, dialog::Focus::Button(_) | dialog::Focus::Type) => {
                match d.focus {
                    dialog::Focus::Button(i) => self.dialog_press(i, storage),
                    _ => self.open_type_list(),
                }
            }
            0x148 | 0x150 if d.focus == dialog::Focus::Type => {
                let kinds = door_workspace::registry_value::KINDS;
                let i = d.type_index();
                let next = if scan == 0x150 {
                    (i + 1).min(kinds.len() - 1)
                } else {
                    i.saturating_sub(1)
                };
                d.value_kind = kinds[next];
            }
            _ => return Ok(d.edit(code)),
        }
        Ok(true)
    }
    fn open_type_list(&mut self) {
        let Some(d) = &self.dialog else {
            return;
        };
        let g = d.geometry(self.layout.width, self.layout.height);
        let count = door_workspace::registry_value::KINDS.len();
        self.popup = Some((
            Popup::combo(
                g.kind,
                count,
                d.type_index(),
                (1 << count) - 1,
                0,
                self.layout.height as i32,
            ),
            PopupKind::DialogType,
        ));
    }
    /// Pointer activation inside a dialog.
    fn dialog_activate(
        &mut self,
        id: u32,
        x: i32,
        storage: Option<&mut Storage>,
    ) -> Result<(), Error> {
        let Some(d) = self.dialog.as_mut() else {
            return Ok(());
        };
        self.keyboard_focus = false;
        match id {
            dialog::FIELD..dialog::TYPE => {
                let i = (id - dialog::FIELD) as usize;
                if d.focus != dialog::Focus::Field(i) {
                    d.focus = dialog::Focus::Field(i);
                }
                // The caret goes where the pointer is (end of text past it).
                let g = d.geometry(self.layout.width, self.layout.height);
                let field = &mut d.fields[i];
                let len = field.text().len();
                let _ = (x, g);
                field.place(len, false);
            }
            dialog::TYPE => {
                d.focus = dialog::Focus::Type;
                self.open_type_list();
            }
            dialog::BUTTON..dialog::BACKGROUND => {
                self.dialog_press((id - dialog::BUTTON) as usize, storage);
            }
            _ => (),
        }
        Ok(())
    }
    /// Press dialog button `index` (0 is the primary button; the last one
    /// is Cancel). Failures keep the dialog and its draft, with the reason.
    fn dialog_press(&mut self, index: usize, storage: Option<&mut Storage>) {
        let Some(d) = self.dialog.as_mut() else {
            return;
        };
        let cancel = index + 1 >= d.kind.buttons().len();
        if cancel {
            let d = self.dialog.take().unwrap();
            if matches!(d.kind, DialogKind::RegistryValue | DialogKind::RegistryEdit) {
                let _ = door_sys::write(b"DOOR_P12 ADMIN REGISTRY CANCELED\n");
            }
            self.focus = d.opener;
            self.popup = None;
            return;
        }
        let Some(storage) = storage else {
            // Host reviews have no services: nothing can be confirmed.
            d.error = String::from("Not available in this review");
            return;
        };
        let result: Result<String, String> = match d.kind {
            DialogKind::AddAccount => {
                let name = String::from(d.fields[0].text().trim());
                let password = String::from(d.fields[1].text());
                let reason = |e: door_service_client::Error| -> String {
                    use door_service_protocol::Code;
                    String::from(match e {
                        door_service_client::Error::Service(Code::Denied) => {
                            "Only an administrator can add accounts."
                        }
                        door_service_client::Error::Service(Code::Exists) => {
                            "That user name is already taken."
                        }
                        door_service_client::Error::Service(Code::Resources) => {
                            "This PC already has as many accounts as it supports."
                        }
                        door_service_client::Error::Service(Code::InvalidRequest) => {
                            "Check the user name and the password."
                        }
                        _ => "The account couldn't be created.",
                    })
                };
                if !door_service_protocol::credential::valid_name(&name) {
                    Err(String::from(
                        "A user name has 1 to 20 characters and none of \" / \\ [ ] : ; | = , + * ? < > @",
                    ))
                } else if password.len() < door_service_protocol::credential::PASSWORD_MIN {
                    Err(String::from("A password has at least 4 characters."))
                } else {
                    let created = door_service_client::Credentials::connect(self.registry)
                        .and_then(|mut c| c.create_account(&name, password.as_bytes()));
                    match created {
                        Ok(uid) => {
                            door_rt::diagnostic(format_args!(
                                "DOOR_P16 SETTINGS ACCOUNT created uid={uid}\n"
                            ));
                            self.load_users();
                            Ok(format!("Account added: {name}"))
                        }
                        Err(e) => Err(reason(e)),
                    }
                }
            }
            DialogKind::AccountType => {
                let administrator = index == 0;
                let uid = self.admin.user_selected.unwrap_or(0);
                let changed = door_service_client::Credentials::connect(self.registry)
                    .and_then(|mut c| c.set_account_type(uid, administrator));
                match changed {
                    Ok(()) => {
                        self.load_users();
                        Ok(String::from(if administrator {
                            "Account type: Administrator"
                        } else {
                            "Account type: Standard User"
                        }))
                    }
                    Err(door_service_client::Error::Service(
                        door_service_protocol::Code::Denied,
                    )) => Err(String::from(
                        "Only an administrator can change account types, and this PC needs at least one administrator.",
                    )),
                    Err(_) => Err(String::from("The account type couldn't be changed.")),
                }
            }
            DialogKind::RemoveAccount => {
                let uid = self.admin.user_selected.unwrap_or(0);
                let name = self
                    .selected_user_name()
                    .map(String::from)
                    .unwrap_or_default();
                let removed = door_service_client::Credentials::connect(self.registry)
                    .and_then(|mut c| c.delete_account(uid));
                match removed {
                    Ok(()) => {
                        self.dialog = None;
                        self.admin.user_selected = None;
                        self.go(Page::OtherUsers);
                        self.log(format!("Account removed: {name}"));
                        return;
                    }
                    Err(door_service_client::Error::Service(
                        door_service_protocol::Code::Denied,
                    )) => Err(String::from(
                        "Only an administrator can remove accounts, and not a signed-in one.",
                    )),
                    Err(_) => Err(String::from("The account couldn't be removed.")),
                }
            }
            DialogKind::AddVpn => {
                let name = String::from(d.fields[0].text().trim());
                let typed = String::from(d.fields[1].text().trim());
                let home = format!("/users/{}", self.uid);
                if name.is_empty()
                    || name.len() > door_service_protocol::net::vpn::MAX_NAME
                    || name.chars().any(char::is_control)
                {
                    Err(String::from(
                        "Enter a connection name of up to 64 characters",
                    ))
                } else if self
                    .admin
                    .vpns
                    .iter()
                    .any(|v| v.0.eq_ignore_ascii_case(&name))
                {
                    Err(String::from(
                        "A VPN connection with this name already exists",
                    ))
                } else {
                    match typed_file(&typed, &home) {
                        None => Err(String::from(
                            "Enter the path of the WireGuard configuration file",
                        )),
                        Some(path) => read_small(
                            storage,
                            &path,
                            door_service_protocol::net::vpn::MAX_TEXT,
                        )
                        .and_then(|bytes| {
                            String::from_utf8(bytes)
                                .map_err(|_| String::from("The configuration file isn't text"))
                        })
                        .and_then(|text| {
                            match door_wireguard::Config::parse(&text) {
                                Err(e) => {
                                    Err(format!("This isn't a valid WireGuard configuration. {e}"))
                                }
                                Ok(c) if c.peer.endpoint.is_none() => Err(String::from(
                                    "The configuration has no Endpoint (server name or address)",
                                )),
                                Ok(c) if c.addresses.is_empty() => {
                                    Err(String::from("The configuration has no IPv4 Address"))
                                }
                                Ok(_) => {
                                    self.pending_vpn.push(VpnOp::Save(name.clone(), text));
                                    Ok(format!("VPN: added {name}"))
                                }
                            }
                        }),
                    }
                }
            }
            DialogKind::RegistryGoTo => {
                let text = String::from(d.fields[0].text());
                self.admin
                    .go_to(&text, index == 1, storage)
                    .map(|()| {
                        if index == 1 {
                            format!("Registry key created: {}", self.admin.location.display())
                        } else {
                            String::new()
                        }
                    })
                    .map_err(String::from)
            }
            DialogKind::RegistryValue => self.save_registry_value(storage),
            DialogKind::RegistryEdit if index == 1 => self.delete_registry_value(storage),
            DialogKind::RegistryEdit => self.save_registry_value(storage),
            DialogKind::RegistryDeleteKey => {
                let location = self.admin.location.clone();
                match door_workspace::registry_browser::delete_key(storage, &location) {
                    Ok(()) => {
                        let shown = location.display();
                        self.admin.registry_up(Some(storage));
                        Ok(format!("Registry key deleted: {shown}"))
                    }
                    Err(e) => Err(String::from(admin::reason(e))),
                }
            }
            DialogKind::VariableEdit if index == 1 => {
                let name = String::from(d.fields[0].text().trim());
                self.admin
                    .delete_variable(&name, storage)
                    .map(|()| format!("Environment variable deleted: {name}"))
            }
            DialogKind::Variable | DialogKind::VariableEdit => {
                let name = String::from(d.fields[0].text().trim());
                let data = String::from(d.fields[1].text());
                let new = matches!(d.target, dialog::Target::Variable(None));
                self.admin
                    .save_variable(&name, &data, new, storage)
                    .map(|()| format!("Environment variable saved: {name}"))
            }
            DialogKind::CheckAccess => {
                let path = String::from(d.fields[0].text());
                self.admin.check_access(&path, storage);
                Ok(String::new())
            }
            DialogKind::Pin | DialogKind::AddPin | DialogKind::Password => {
                self.begin_credential_update();
                return;
            }
            DialogKind::SetupScript => {
                let address = String::from(d.fields[0].text().trim());
                if address.is_empty() {
                    door_workspace::proxy::write_script(storage, None)
                        .map(|()| String::from("Setup script turned off"))
                        .map_err(|e| format!("The setup script couldn't be saved: {e:?}"))
                } else if door_workspace::proxy::valid_script(&address) {
                    door_workspace::proxy::write_script(storage, Some(&address))
                        .map(|()| {
                            door_rt::diagnostic(format_args!(
                                "DOOR_P15 SETTINGS PROXY script={address}\n"
                            ));
                            format!("Using setup script {address}")
                        })
                        .map_err(|e| format!("The setup script couldn't be saved: {e:?}"))
                } else {
                    Err(String::from("Enter an http:// or https:// address"))
                }
            }
            DialogKind::Proxy => {
                let host = String::from(d.fields[0].text().trim());
                let port = String::from(d.fields[1].text().trim());
                if host.is_empty() {
                    door_workspace::proxy::write(storage, None)
                        .map(|()| String::from("Proxy server turned off"))
                        .map_err(|e| format!("The proxy setting couldn't be saved: {e:?}"))
                } else {
                    match (
                        door_workspace::proxy::parse_server(&host),
                        port.parse::<u16>().ok().filter(|p| *p > 0),
                    ) {
                        (Some((host, _)), Some(port)) => {
                            let proxy = door_workspace::proxy::Proxy {
                                host,
                                port,
                                bypass: alloc::vec![String::from("<local>")],
                            };
                            door_workspace::proxy::write(storage, Some(&proxy))
                                .map(|()| {
                                    door_rt::diagnostic(format_args!(
                                        "DOOR_P15 SETTINGS PROXY server={}
",
                                        proxy.server()
                                    ));
                                    format!("Using proxy server {}", proxy.server())
                                })
                                .map_err(|e| format!("The proxy setting couldn't be saved: {e:?}"))
                        }
                        (None, _) => Err(String::from("Enter a valid proxy address")),
                        (_, None) => Err(String::from("Enter a port from 1 to 65535")),
                    }
                }
            }
            DialogKind::IpSettings | DialogKind::DnsSettings => {
                use door_service_protocol::net::{Config, parse_ipv4};
                let ip_dialog = d.kind == DialogKind::IpSettings;
                let first = String::from(d.fields[0].text().trim());
                let second = String::from(d.fields[1].text().trim());
                let current = self.net.map(|n| n.config).unwrap_or_default();
                // An empty field means "automatic" (DHCP).
                let optional = |t: &str| {
                    if t.is_empty() {
                        Some([0; 4])
                    } else {
                        parse_ipv4(t).filter(|a| *a != [0; 4])
                    }
                };
                let config = if !ip_dialog {
                    match (optional(&first), optional(&second)) {
                        (Some(a), Some(b)) => Ok(Config {
                            dns: [a, b],
                            ..current
                        }),
                        _ => Err(String::from("Enter valid DNS server addresses")),
                    }
                } else if first.is_empty() {
                    Ok(Config {
                        fixed: None,
                        ..current
                    })
                } else {
                    let (address, prefix) = first.split_once('/').unwrap_or((&first, "24"));
                    match (
                        parse_ipv4(address).filter(|a| *a != [0; 4]),
                        prefix
                            .trim()
                            .parse::<u8>()
                            .ok()
                            .filter(|p| (1..=30).contains(p)),
                        optional(&second),
                    ) {
                        (None, _, _) => Err(String::from("Enter a valid IP address")),
                        (_, None, _) => {
                            Err(String::from("The subnet prefix length must be 1 to 30"))
                        }
                        (_, _, None) => Err(String::from("Enter a valid gateway address")),
                        (Some(a), Some(p), Some(g)) => Ok(Config {
                            fixed: Some((a, p, g)),
                            ..current
                        }),
                    }
                };
                config.map(|config| {
                    self.pending_net = Some(config);
                    String::from(if ip_dialog {
                        "IP settings saved"
                    } else {
                        "DNS settings saved"
                    })
                })
            }
            DialogKind::RenamePc => {
                let name = String::from(d.fields[0].text().trim());
                match door_service_protocol::device::validate(&name) {
                    Err(e) => Err(String::from(e.message())),
                    Ok(()) => match door_service_client::DeviceName::connect(self.registry)
                        .and_then(|mut c| c.set(&name))
                    {
                        Ok(()) => {
                            self.device_name = name.clone();
                            let _ = door_sys::write(
                                format!("DOOR_P14 SETTINGS DEVICE_NAME {name}\n").as_bytes(),
                            );
                            Ok(format!("This PC renamed: {name}"))
                        }
                        Err(_) => Err(String::from("The name couldn't be saved")),
                    },
                }
            }
        };
        match result {
            Ok(event) => {
                let d = self.dialog.take().unwrap();
                self.focus = d.opener;
                if !event.is_empty() {
                    self.log(event);
                }
            }
            Err(error) => {
                if let Some(d) = self.dialog.as_mut() {
                    d.error = error;
                }
            }
        }
    }
    /// Sign-in options dialogs: check both secrets locally, then ask the
    /// session's credential store (the answer arrives in `tick`).
    fn begin_credential_update(&mut self) {
        use door_service_protocol::credential::{self as cr, Kind};
        let Some(d) = self.dialog.as_mut() else {
            return;
        };
        if self.credential_busy {
            return;
        }
        let (current_kind, new_kind) = match d.kind {
            DialogKind::Pin => (Kind::Pin, Kind::Pin),
            DialogKind::AddPin => (Kind::Password, Kind::Pin),
            _ => (Kind::Password, Kind::Password),
        };
        let current = String::from(d.fields[0].text());
        let new = String::from(d.fields[1].text());
        if cr::validate(new_kind, new.as_bytes()).is_err() {
            d.error = String::from(if new_kind == Kind::Pin {
                "Use 4 to 16 digits for the new PIN."
            } else {
                "Use at least 4 characters for the new password."
            });
            return;
        }
        if current.is_empty() {
            d.error = String::from("Enter your current one first.");
            return;
        }
        let Some(c) = self.credentials.as_mut() else {
            d.error = String::from("Unavailable: the credential store didn't answer");
            return;
        };
        let now = door_sys::time().unwrap_or(0);
        let started = if d.kind == DialogKind::AddPin {
            c.begin_add(Kind::Pin, current.as_bytes(), new.as_bytes(), now)
        } else {
            c.begin_change(current_kind, current.as_bytes(), new.as_bytes(), now)
        };
        match started {
            Ok(()) => {
                self.credential_busy = true;
                d.error = String::from("Checking...");
            }
            Err(_) => d.error = String::from("Unavailable: the credential store didn't answer"),
        }
    }
    /// The credential store answered a Sign-in options update.
    fn poll_credentials(&mut self, now: u64) -> bool {
        use door_service_protocol::credential::{Kind, Verdict};
        if !self.credential_busy {
            return false;
        }
        let Some(c) = self.credentials.as_mut() else {
            return false;
        };
        let reply = match c.poll(now) {
            Ok(None) => return false,
            Ok(Some(reply)) => Some(reply),
            Err(_) => None,
        };
        self.credential_busy = false;
        let message = match reply {
            Some(door_service_client::CredentialReply::Updated(outcome)) => {
                self.pin_enrolled = Some(outcome.state.enrolled == Some(Kind::Pin));
                match outcome.verdict {
                    Verdict::Accepted => None,
                    Verdict::Rejected { .. } => Some("That's not correct. Try again."),
                    Verdict::Throttled { .. } => {
                        Some("Too many attempts. Wait a moment and try again.")
                    }
                }
            }
            _ => Some("Unavailable: the credential store didn't answer"),
        };
        match message {
            None => {
                if let Some(d) = self.dialog.take() {
                    self.focus = d.opener;
                    let what = if d.kind == DialogKind::Password {
                        "Password"
                    } else {
                        "PIN"
                    };
                    let _ = door_sys::write(
                        format!("DOOR_P14 SETTINGS CREDENTIAL {what} changed=1\n").as_bytes(),
                    );
                    self.log(format!("Sign-in options: {what} changed"));
                }
            }
            Some(text) => {
                if let Some(d) = self.dialog.as_mut() {
                    d.error = String::from(text);
                    d.fields[0].set(String::new(), false);
                }
            }
        }
        true
    }
    /// Registry Editor > New value / Edit: validate the name and the typed
    /// data, then save against the key version the dialog was opened on.
    /// Registry Editor > Edit > Delete: remove the value being edited.
    fn delete_registry_value(&mut self, storage: &mut Storage) -> Result<String, String> {
        let d = self.dialog.as_ref().ok_or_else(String::new)?;
        let dialog::Target::Registry(snapshot, Some(name)) = &d.target else {
            return Err(String::new());
        };
        door_workspace::registry_browser::delete_value(storage, snapshot, name)
            .map_err(|e| String::from(admin::reason(e)))?;
        let shown = door_workspace::registry_browser::display_units(name);
        let _ = door_sys::write(b"DOOR_P14 ADMIN REGISTRY DELETED\n");
        self.admin.reload_registry(storage);
        Ok(format!("Registry value deleted: {shown}"))
    }
    fn save_registry_value(&mut self, storage: &mut Storage) -> Result<String, String> {
        let d = self.dialog.as_ref().ok_or_else(String::new)?;
        let dialog::Target::Registry(snapshot, original) = &d.target else {
            return Err(String::new());
        };
        // Editing with a changed name renames the value.
        let typed = d.fields[0].text();
        let renamed = original.as_ref().is_some_and(|old| {
            d.kind == DialogKind::RegistryEdit
                && typed != door_workspace::registry_browser::display_units(old)
        });
        let name: Vec<u16> = match original {
            Some(name) if !renamed => name.clone(),
            _ => {
                let text = typed;
                if text.len() > 48 || !text.bytes().all(|b| (32..=126).contains(&b)) {
                    return Err(String::from(
                        "Use a value name of at most 48 printable ASCII characters.",
                    ));
                }
                text.encode_utf16().collect()
            }
        };
        if renamed {
            let old = original.clone().unwrap_or_default();
            let parsed = door_workspace::registry_value::parse(d.value_kind, d.fields[1].text())
                .map_err(|e| format!("That isn't valid data ({e:?}). Your text is kept."))?;
            door_workspace::registry_browser::rename_value(storage, snapshot, &old, &name, parsed)
                .map_err(|e| String::from(admin::reason(e)))?;
            let _ = door_sys::write(b"DOOR_P12 ADMIN REGISTRY SAVED\n");
            self.admin.reload_registry(storage);
            return Ok(format!(
                "Registry value renamed: {}",
                door_workspace::registry_browser::display_units(&name)
            ));
        }
        let parsed = door_workspace::registry_value::parse(d.value_kind, d.fields[1].text())
            .map_err(|e| {
                format!(
                    "That isn't valid {} data ({e:?}). Your text is kept.",
                    door_workspace::registry_value::label(d.value_kind)
                )
            })?;
        door_workspace::registry_browser::save(
            storage,
            snapshot,
            &name,
            parsed,
            original.is_none(),
        )
        .map_err(|e| String::from(admin::reason(e)))?;
        let _ = door_sys::write(b"DOOR_P12 ADMIN REGISTRY SAVED\n");
        self.admin.reload_registry(storage);
        Ok(format!(
            "Registry value saved: {}",
            door_workspace::registry_browser::display_units(&name)
        ))
    }
}
/// Colored Windows category artwork (rail and page heroes). Subpages use
/// their category's artwork.
fn category(v: &mut ui::View<'_>, page: Page, x: i32, y: i32, size: usize) {
    // Contrast themes draw the rail and row categories as monochrome Segoe
    // Fluent glyphs in the theme's text colour, as Windows Settings does.
    let page = page.category();
    if size == 20 && v.palette.contrast.is_some() {
        let glyph = match page {
            Page::System => "display",
            Page::Bluetooth => "devices",
            Page::Network => "network",
            Page::Personalization => "personalize",
            Page::Apps => "apps",
            Page::Accounts => "people",
            Page::Time => "globe",
            Page::Gaming => "game",
            Page::Accessibility => "eye",
            Page::Privacy => "shield",
            Page::Update => "refresh",
            _ => "home",
        };
        v.mask_pixels(
            x,
            y,
            20,
            symbols::mask(glyph, v.scale),
            [26, 26, 26, 255],
            None,
        );
        return;
    }
    if page == Page::System {
        let pixels: &[u8] = match (size, v.scale) {
            (32, 3) => door_workspace::asset!("../../../assets/icons/windows/computer.96.bgra"),
            (32, 2) => door_workspace::asset!("../../../assets/icons/windows/computer.64.bgra"),
            (32, _) => door_workspace::asset!("../../../assets/icons/windows/computer.32.bgra"),
            (_, 3) => door_workspace::asset!("../../../assets/icons/windows/computer.60.bgra"),
            (_, 2) => door_workspace::asset!("../../../assets/icons/windows/computer.40.bgra"),
            _ => door_workspace::asset!("../../../assets/icons/windows/computer.bgra"),
        };
        v.color_pixels(x, y, if size == 32 { 32 } else { 20 }, pixels);
        return;
    }
    macro_rules! png {
        ($name:literal) => {
            match (size, v.scale) {
                (48, 3) => &door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-settings/",
                    $name,
                    ".144.bgra"
                ))[..],
                (48, 2) => &door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-settings/",
                    $name,
                    ".96.bgra"
                ))[..],
                (32, 3) => &door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-settings/",
                    $name,
                    ".96.bgra"
                ))[..],
                (32, 2) => &door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-settings/",
                    $name,
                    ".64.bgra"
                ))[..],
                (_, 3) => &door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-settings/",
                    $name,
                    ".60.bgra"
                ))[..],
                (_, 2) => &door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-settings/",
                    $name,
                    ".40.bgra"
                ))[..],
                (48, _) => &door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-settings/",
                    $name,
                    ".48.bgra"
                ))[..],
                (32, _) => &door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-settings/",
                    $name,
                    ".32.bgra"
                ))[..],
                _ => &door_workspace::asset!(concat!(
                    "../../../assets/icons/windows-settings/",
                    $name,
                    ".20.bgra"
                ))[..],
            }
        };
    }
    let pixels = match page {
        Page::Bluetooth => png!("bluetooth-devices"),
        Page::Network => png!("network-internet"),
        Page::Personalization => png!("personalization"),
        Page::Apps => png!("apps"),
        Page::Accounts => png!("accounts"),
        Page::Time => png!("time-language"),
        Page::Gaming => png!("gaming"),
        Page::Accessibility => png!("accessibility"),
        Page::Privacy => png!("security-privacy"),
        Page::Update => png!("windows-update"),
        _ => png!("home"),
    };
    let size = if matches!(size, 32 | 48) { size } else { 20 };
    v.color_pixels(x, y, size, pixels);
}
/// Create (or replace) `path` with `bytes` (Registry Editor > Export).
fn write_whole_file(storage: &mut Storage, path: &str, bytes: &[u8]) -> Result<(), Error> {
    use door_service_client::{Disposition, SHARE_ALL, WRITE};
    let file = storage.open(path, Disposition::CreateAlways, WRITE, SHARE_ALL)?;
    let written = (|| {
        let mut at = 0;
        for chunk in bytes.chunks(door_service_protocol::CHUNK_BYTES) {
            if storage.write(&file, at as u64, chunk)? != chunk.len() {
                return Err(Error::Malformed);
            }
            at += chunk.len();
        }
        storage.flush()
    })();
    let closed = storage.close(file);
    written.and(closed)
}

/// A VPN request Settings sends to door.network (Network & internet > VPN).
pub enum VpnOp {
    /// A new connection: its name and wg-quick text.
    Save(String, String),
    Connect(String),
    Disconnect(String),
    Remove(String),
}
/// The native path of a file the user typed (`C:\Users\...`, `~\...`,
/// `%USERPROFILE%\...`, `shell:downloads\...` or a native path).
fn typed_file(input: &str, home: &str) -> Option<String> {
    let door_workspace::explorer_path::Address::Directory { start, parts, .. } =
        door_workspace::explorer_path::parse(input, home, home).ok()?
    else {
        return None;
    };
    let mut path = start;
    for part in parts {
        match part.as_str() {
            "." => (),
            ".." => {
                let cut = path.rfind('/').unwrap_or(0);
                path.truncate(cut.max(1));
            }
            _ => {
                if !path.ends_with('/') {
                    path.push('/');
                }
                path.push_str(&part);
            }
        }
    }
    Some(path)
}
/// A small text file's bytes (at most `limit`).
fn read_small(storage: &mut Storage, path: &str, limit: usize) -> Result<Vec<u8>, String> {
    use door_service_client::{Disposition, READ, SHARE_ALL};
    let file = storage
        .open(path, Disposition::OpenExisting, READ, SHARE_ALL)
        .map_err(|_| String::from("The configuration file couldn't be opened"))?;
    let result = (|| {
        let size = storage
            .info(&file)
            .map_err(|_| String::from("The configuration file couldn't be read"))?
            .bytes as usize;
        if size > limit {
            return Err(String::from("The configuration file is too large"));
        }
        let mut out = alloc::vec![0u8; size];
        let mut at = 0;
        while at < size {
            let n = storage
                .read(&file, at as u64, &mut out[at..size.min(at + 192)])
                .map_err(|_| String::from("The configuration file couldn't be read"))?;
            if n == 0 {
                break;
            }
            at += n;
        }
        out.truncate(at);
        Ok(out)
    })();
    let _ = storage.close(file);
    result
}
