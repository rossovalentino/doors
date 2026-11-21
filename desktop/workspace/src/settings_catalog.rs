//! Windows 11 (25H2) Settings page model. Each page is a list of blocks
//! (hero areas, section headers, setting cards, grouped cards, expanders).
//! One layout pass places them; paint, hit testing, keyboard focus and the
//! scroll extent all read the same placement. Controls without a provider
//! are drawn disabled and never become targets; live preferences are bound
//! to `Controls` bits or the appearance word, never to stand-in state.
use crate::settings_layout::{GALLERY, Layout, Page, THEMES};
use alloc::vec::Vec;
use door_design_tokens as tokens;
use door_keyboard_layouts::methods::{Ime, METHODS, Method, Preferences};
use door_service_protocol::audio::capture;
use door_service_protocol::desktop::{ContrastTheme, Controls};
use door_service_protocol::preferences as p;
/// Typed views of the preference words a page edits.
fn theme_schedule(state: State) -> p::ThemeSchedule {
    p::ThemeSchedule::from_bits(state.pref(p::THEME_SCHEDULE)).unwrap_or(p::ThemeSchedule::DEFAULT)
}
fn dnd_schedule(state: State) -> p::DndSchedule {
    p::DndSchedule::from_bits(state.pref(p::DND_SCHEDULE)).unwrap_or(p::DndSchedule::DEFAULT)
}
fn focus(state: State) -> p::Focus {
    p::Focus::from_bits(state.pref(p::FOCUS)).unwrap_or(p::Focus::DEFAULT)
}
fn background(state: State) -> p::Background {
    p::Background::from_bits(state.pref(p::BACKGROUND)).unwrap_or(p::Background::DEFAULT)
}
fn overrides(state: State) -> p::RegionalOverrides {
    p::RegionalOverrides::from_bits(state.pref(p::REGIONAL_OVERRIDES))
        .unwrap_or(p::RegionalOverrides::NONE)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}
impl Rect {
    pub fn center_x(self) -> i32 {
        self.x + self.w as i32 / 2
    }
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }
    pub fn right(self) -> i32 {
        self.x + self.w as i32
    }
    pub fn bottom(self) -> i32 {
        self.y + self.h as i32
    }
    pub fn contains(self, x: i32, y: i32) -> bool {
        (self.x..self.right()).contains(&x) && (self.y..self.bottom()).contains(&y)
    }
    pub fn center_y(self) -> i32 {
        self.y + self.h as i32 / 2
    }
}
/// Live preference state read by bound controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub controls: Controls,
    pub appearance: u32,
    /// Stored System > Display > Scale (`ScalePercent`): 0 is automatic.
    pub scale: u32,
    /// What automatic resolves to on this display (the "(Recommended)" one).
    pub recommended: u32,
    /// Scales the display offers (bit i = `SCALES[i]`).
    pub offered: u32,
    /// Contrast themes list selection, applied with Apply: 0 None, else
    /// `ContrastTheme::ALL[pick - 1]`.
    pub contrast_pick: u8,
    /// Text size slider value (percent), applied with Apply.
    pub text_pick: u32,
    /// Installed keyboards and input methods with the current one
    /// (`Setting::InputMethods`, `door_keyboard_layouts::Preferences`).
    pub input: u32,
    /// The window server's text-input state flags (`text_input::STATE_IME`,
    /// methods without candidates); 0 until reported or without an IME.
    pub ime: u32,
    /// `Setting::TimeZone` (`door_locale::tz::Zone::id`).
    pub zone: u32,
    /// `Setting::RegionalFormat` (`door_locale::region::Region::id`).
    pub region: u32,
    /// Registry Editor: root key (0 HKEY_CURRENT_USER, 1 HKEY_LOCAL_MACHINE,
    /// 2 HKEY_CLASSES_ROOT) and view (0 64-bit, 1 32-bit).
    pub registry_root: u8,
    pub registry_view: u8,
    /// P14 preference words, ids `PREF_FIRST..` (`door_service_protocol::preferences`).
    pub prefs: [u32; PREFS],
    /// `Setting::AccentColor` (`desktop::Accent` bits).
    pub accent: u32,
    /// The network service's status (None: not reached yet or not running).
    pub net: Option<door_service_protocol::net::Status>,
    /// Its IPv6 side.
    pub net6: Option<door_service_protocol::net::v6::Status6>,
    /// The sound service's status (None: not reached yet or not running).
    pub audio: Option<door_service_protocol::audio::Status>,
    /// Proxy > Automatically detect settings.
    pub proxy_auto_detect: bool,
    /// The firewall's profile bytes (domain, private, public) door.network
    /// enforces.
    pub firewall: [u8; 3],
}
/// First preference id and how many there are.
pub const PREF_FIRST: u32 = 10;
pub const PREFS: usize = door_service_protocol::preferences::ALL.len();
/// Every preference at its clean-Windows default.
pub const fn default_prefs() -> [u32; PREFS] {
    let mut out = [0; PREFS];
    let mut i = 0;
    while i < PREFS {
        out[i] = door_service_protocol::preferences::ALL[i].default;
        i += 1;
    }
    out
}
impl State {
    pub const fn new(controls: Controls, appearance: u32) -> Self {
        Self {
            controls,
            appearance,
            scale: 0,
            recommended: 100,
            offered: ALL_SCALES,
            contrast_pick: contrast_pick(controls),
            text_pick: controls.text_percent(),
            input: Preferences::DEFAULT.bits(),
            ime: 0,
            zone: door_locale::tz::DEFAULT_ID as u32,
            region: door_locale::region::DEFAULT_ID as u32,
            registry_root: 0,
            registry_view: 0,
            prefs: default_prefs(),
            accent: 0,
            net: None,
            net6: None,
            audio: None,
            proxy_auto_detect: true,
            firewall: [door_service_protocol::net::firewall::profile::DEFAULT; 3],
        }
    }
    /// Preference word `id` (its default for an unknown id).
    pub fn pref(self, id: u32) -> u32 {
        id.checked_sub(PREF_FIRST)
            .and_then(|i| self.prefs.get(i as usize).copied())
            .unwrap_or_else(|| door_service_protocol::preferences::default(id))
    }
    /// The installed input methods (the US keyboard if the word is invalid).
    pub fn input_methods(self) -> Preferences {
        Preferences::from_bits(self.input).unwrap_or_default()
    }
}
/// "Language - Keyboard" as Windows 11 lists input methods (Override for
/// default input method, Add a keyboard), by method id.
const INPUT_LABELS: [(u8, &str); 9] = [
    (1, "English (United States) - US"),
    (2, "English (United States) - United States-International"),
    (3, "English (United Kingdom) - United Kingdom"),
    (4, "Italiano (Italia) - Italian"),
    (5, "Deutsch (Deutschland) - German"),
    (6, "Français (France) - French"),
    (7, "中文(中华人民共和国) - Macrohard Pinyin"),
    (8, "日本語 - Macrohard IME"),
    (9, "한국어 - Macrohard IME"),
];
pub fn input_label(id: u8) -> &'static str {
    INPUT_LABELS
        .iter()
        .find(|(i, _)| *i == id)
        .map_or("", |(_, label)| label)
}
/// Installed keyboard "..." menu (Windows: Move up, Move down, Remove).
pub const KEYBOARD_MENU: [&str; 3] = ["Move up", "Move down", "Remove"];
/// Methods "Add a keyboard" offers: the catalog minus the installed ones.
pub fn addable(state: State) -> impl Iterator<Item = &'static Method> {
    let installed = state.input_methods();
    METHODS
        .iter()
        .filter(move |m| !installed.ids().any(|id| id == m.id))
}
/// Enabled "..." menu entries of installed method `index`: one keyboard
/// always stays installed.
pub fn keyboard_menu_mask(state: State, index: usize) -> u64 {
    let n = state.input_methods().count();
    if index >= n {
        return 0;
    }
    u64::from(index > 0) | u64::from(index + 1 < n) << 1 | u64::from(n > 1) << 2
}
/// Rebuild the list keeping the current method current (removing it
/// selects the first, as Windows does).
fn reordered(state: State, ids: &[u8]) -> Option<Change> {
    let p = state.input_methods();
    let current = p.current().id;
    let index = ids.iter().position(|id| *id == current).unwrap_or(0);
    Preferences::new(ids, index, p.per_window()).map(|p| Change::InputMethods(p.bits()))
}
/// "..." menu entry `option` of installed method `index`.
pub fn keyboard_menu_apply(state: State, index: usize, option: usize) -> Option<Change> {
    if option >= KEYBOARD_MENU.len() || keyboard_menu_mask(state, index) & (1 << option) == 0 {
        return None;
    }
    let mut ids: Vec<u8> = state.input_methods().ids().collect();
    match option {
        0 => ids.swap(index - 1, index),
        1 => ids.swap(index, index + 1),
        _ => {
            ids.remove(index);
        }
    }
    reordered(state, &ids)
}
/// "Add a keyboard" entry `option` (an index into `addable`).
pub fn keyboard_add_apply(state: State, option: usize) -> Option<Change> {
    let method = addable(state).nth(option)?;
    let mut ids: Vec<u8> = state.input_methods().ids().collect();
    if ids.len() >= door_keyboard_layouts::methods::MAX_METHODS {
        return None;
    }
    ids.push(method.id);
    reordered(state, &ids)
}
/// Row description of an installed method: its language, and for an input
/// method editor whether it runs and offers candidates.
pub fn keyboard_status(state: State, method: &Method) -> &'static str {
    let ime = door_service_protocol::text_input::State {
        preferences: state.input,
        method: u32::from(method.id),
        mode: 0,
        flags: state.ime,
    };
    match method.ime {
        Ime::None => "Keyboard layout",
        _ if state.ime & door_service_protocol::text_input::STATE_IME == 0 => {
            "Unavailable: the input method editor is not running"
        }
        _ if !ime.candidates_available(u32::from(method.id)) => {
            "Input method editor; candidates unavailable: dictionary not installed"
        }
        _ => "Input method editor",
    }
}
/// The Contrast themes list entry for `controls`: None, or the active theme.
pub const fn contrast_pick(controls: Controls) -> u8 {
    match controls.contrast() {
        None => 0,
        Some(ContrastTheme::Aquatic) => 1,
        Some(ContrastTheme::Desert) => 2,
        Some(ContrastTheme::Dusk) => 3,
        Some(ContrastTheme::NightSky) => 4,
    }
}
/// The theme of a Contrast themes list entry (0 is None).
pub fn picked_theme(pick: u8) -> Option<ContrastTheme> {
    ContrastTheme::ALL
        .get(usize::from(pick).checked_sub(1)?)
        .copied()
}
/// System > Display > Scale percentages, as Windows lists them (no 275%),
/// in the order of the window server's offered-scale bits.
pub const SCALES: [u32; 8] = door_service_protocol::window::DISPLAY_SCALES;
/// Every scale offered (before the window server reports its list).
pub const ALL_SCALES: u32 = (1 << SCALES.len()) - 1;
const SCALE_LABELS: [&str; 8] = [
    "100%", "125%", "150%", "175%", "200%", "225%", "250%", "300%",
];
const SCALE_RECOMMENDED: [&str; 8] = [
    "100% (Recommended)",
    "125% (Recommended)",
    "150% (Recommended)",
    "175% (Recommended)",
    "200% (Recommended)",
    "225% (Recommended)",
    "250% (Recommended)",
    "300% (Recommended)",
];
/// The scales listed for `state`: only those the display offers, as
/// Windows lists them (every scale until the window server reports).
fn offered(state: State) -> impl Iterator<Item = usize> {
    let mask = if state.offered & ALL_SCALES == 0 {
        ALL_SCALES
    } else {
        state.offered
    };
    (0..SCALES.len()).filter(move |i| mask & (1 << i) != 0)
}
/// Index into `SCALES` of list entry `index`.
fn scale_entry(state: State, index: usize) -> Option<usize> {
    offered(state).nth(index)
}
/// Accessibility > Text size slider range (percent).
pub const TEXT_MIN: u32 = 100;
pub const TEXT_MAX: u32 = 225;
/// A persisted change produced by a control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Controls(Controls),
    /// Reconfigure the network adapter (door.network CONFIGURE).
    Network(door_service_protocol::net::Config),
    Appearance(u32),
    /// The Contrast themes list moved (nothing applies until Apply).
    ContrastPick(u8),
    /// Store `ScalePercent` (0: automatic).
    Scale(u32),
    /// Store and apply `InputMethods` (Preferences bits).
    InputMethods(u32),
    /// Store `TimeZone` and apply it to the taskbar clock.
    TimeZone(u32),
    /// Store `RegionalFormat` and apply it to the taskbar clock.
    RegionalFormat(u32),
    /// Registry Editor: open the root key of this view (root, view).
    RegistryLocation(u8, u8),
    /// Store preference word (id, value) and apply it live (P14).
    Preference(u32, u32),
    /// Store `AccentColor` (`desktop::Accent` bits) and apply it live.
    Accent(u32),
    /// Taskbar size: the desktop controls (Small) and the TASKBAR
    /// preference word (Large), stored together.
    TaskbarSize(Controls, u32),
    /// Proxy > Automatically detect settings (Internet Settings).
    ProxyAutoDetect(bool),
    /// Firewall & network protection: profile (0..3) gets this byte
    /// (`net::firewall::profile`), once an administrator approves.
    Firewall(u8, u8),
}
/// Profile byte bits the firewall page shows.
pub const FIREWALL_ON: u8 = door_service_protocol::net::firewall::profile::ON;
pub const FIREWALL_SHIELDS: u8 = door_service_protocol::net::firewall::profile::SHIELDS_UP;
/// Commands of card buttons and page heroes. Each one does real work; a
/// command that cannot apply in the current state is painted disabled and
/// ignored (`Settings::command_enabled`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// Contrast themes > Apply: the listed theme, or None.
    ApplyContrast,
    /// Text size > Apply: the slider's size.
    ApplyTextSize,
    /// Read the page's providers again (F5 does the same).
    Refresh,
    /// Open Task Manager (processes, performance, services).
    OpenTaskManager,
    /// Open Terminal.
    OpenTerminal,
    /// Troubleshoot: run the native interface, clock and storage checks.
    RunChecks,
    /// Registry Editor: open or create a key by its path (dialog).
    RegistryGoTo,
    /// Registry Editor: open the parent key.
    RegistryUp,
    /// Registry Editor: add a value to the current key (dialog).
    RegistryNewValue,
    /// Registry Editor: delete the current key (confirmation dialog).
    RegistryDeleteKey,
    /// Registry Editor: export the current key to Documents as a .reg file.
    RegistryExport,
    /// Personalization > Context menu: open the customizer window.
    CustomizeContextMenu,
    /// Environment variables: add a user variable (dialog).
    NewVariable,
    /// File system: check access to a path (dialog).
    CheckAccess,
    /// System > Clipboard: clear the history (pinned items stay).
    ClearClipboard,
    /// Sign-in options: change the PIN or the password (dialog).
    ChangePin,
    ChangePassword,
    /// Other users: add a local account (dialog).
    AddAccount,
    /// An account's page: Administrator or Standard User (dialog).
    ChangeAccountType,
    /// An account's page: delete the account (confirmation).
    RemoveAccount,
    /// About: rename this PC (dialog).
    RenamePc,
    /// Ethernet > IP assignment / DNS server assignment > Edit (dialogs).
    EditIp,
    EditDns,
    /// Proxy > Manual proxy setup > Set up (dialog).
    EditProxy,
    /// Proxy > Automatic proxy setup > Use setup script > Set up (dialog).
    EditSetupScript,
    /// Advanced network settings > Network reset: the adapter back on,
    /// with DHCP addressing and DHCP DNS servers.
    NetworkReset,
    /// System > Sound > Speakers > Test: a one-second tone.
    TestSound,
    /// Network & internet > VPN > Add VPN (dialog).
    AddVpn,
    /// Firewall & network protection > Restore defaults (administrator
    /// approval first).
    RestoreFirewall,
}
impl Command {
    /// Accessible name when the button's own label is not shown.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ApplyContrast | Self::ApplyTextSize => "Apply",
            Self::Refresh => "Refresh",
            Self::OpenTaskManager => "Open Task Manager",
            Self::OpenTerminal => "Open",
            Self::RunChecks => "Run",
            Self::RegistryGoTo => "Go to",
            Self::RegistryUp => "Up",
            Self::RegistryNewValue => "New value",
            Self::RegistryDeleteKey => "Delete",
            Self::RegistryExport => "Export",
            Self::NewVariable => "New",
            Self::CheckAccess => "Check",
            Self::ClearClipboard => "Clear",
            Self::ChangePin => "Change PIN",
            Self::ChangePassword => "Change",
            Self::AddAccount => "Add account",
            Self::ChangeAccountType => "Change account type",
            Self::RemoveAccount => "Remove",
            Self::RenamePc => "Rename this PC",
            Self::CustomizeContextMenu => "Customize",
            Self::EditIp | Self::EditDns => "Edit",
            Self::EditProxy | Self::EditSetupScript => "Set up",
            Self::NetworkReset => "Reset now",
            Self::TestSound => "Test",
            Self::AddVpn => "Add VPN",
            Self::RestoreFirewall => "Restore defaults",
        }
    }
    /// The page's primary action, a WinUI AccentButton.
    pub const fn accent(self) -> bool {
        matches!(self, Self::AddVpn)
    }
    /// Button width (WinUI buttons size to their label; 120 px minimum here).
    pub const fn width(self) -> u32 {
        match self {
            Self::OpenTaskManager | Self::ChangeAccountType => 168,
            Self::RestoreFirewall => 144,
            Self::AddVpn => 90,
            _ => 120,
        }
    }
}
/// Entries the application supplies at run time, placed as one card each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum List {
    /// Registry Editor: subkeys of the current key (open).
    RegistryKeys,
    /// Registry Editor: values of the current key (Edit).
    RegistryValues,
    /// Environment variables saved for your account (Edit).
    UserVariables,
    /// Environment variables Settings started with (read-only).
    LaunchVariables,
    /// Event logs: Settings changes in this session (read-only).
    Events,
    /// Default apps: file types and the app that opens each (Choose).
    FileTypes,
    /// Microphone: desktop apps that have used the microphone.
    MicrophoneApps,
    /// Microphone > Recent activity: every recent use.
    MicrophoneActivity,
    /// VPN: the user's connections (Connect or Disconnect, and Remove).
    VpnConnections,
    /// Other users: the local accounts besides yours (open one).
    OtherUsers,
    /// Event logs: the Application and System logs, newest first.
    ApplicationEvents,
    SystemEvents,
    /// Scheduled tasks: the Task Scheduler's tasks you can see.
    ScheduledTasks,
    /// Services: what the Service Control Manager runs and knows.
    InstalledServices,
}
impl List {
    pub const COUNT: usize = 14;
    pub const ALL: [Self; Self::COUNT] = [
        Self::RegistryKeys,
        Self::RegistryValues,
        Self::UserVariables,
        Self::LaunchVariables,
        Self::Events,
        Self::FileTypes,
        Self::MicrophoneApps,
        Self::MicrophoneActivity,
        Self::VpnConnections,
        Self::OtherUsers,
        Self::ApplicationEvents,
        Self::SystemEvents,
        Self::ScheduledTasks,
        Self::InstalledServices,
    ];
    /// What activating an entry does: open it (chevron) or edit it (button).
    pub const fn entry_action(self) -> Option<bool> {
        match self {
            Self::RegistryKeys | Self::OtherUsers => Some(true),
            Self::RegistryValues | Self::UserVariables | Self::FileTypes | Self::VpnConnections => {
                Some(false)
            }
            Self::LaunchVariables
            | Self::Events
            | Self::MicrophoneApps
            | Self::MicrophoneActivity
            | Self::ApplicationEvents
            | Self::SystemEvents
            | Self::ScheduledTasks
            | Self::InstalledServices => None,
        }
    }
    /// Text shown when the list has no entries.
    pub const fn empty(self) -> &'static str {
        match self {
            Self::RegistryKeys => "This key has no subkeys.",
            Self::RegistryValues => "This key has no values.",
            Self::UserVariables => "You haven't saved any variables.",
            Self::LaunchVariables => "Settings started without environment variables.",
            Self::Events => "Nothing has changed in this session yet.",
            Self::FileTypes => "No file types are registered.",
            Self::MicrophoneApps => "No desktop apps have used your microphone.",
            Self::MicrophoneActivity => "No apps have used your microphone in the last 7 days.",
            Self::VpnConnections => "You haven't added a VPN connection.",
            Self::OtherUsers => "No other users have accounts on this PC.",
            Self::ApplicationEvents => "No events in the Application log.",
            Self::SystemEvents => "No events in the System log.",
            Self::ScheduledTasks => "There are no scheduled tasks.",
            Self::InstalledServices => "The Service Control Manager isn't available.",
        }
    }
}
/// The entry `index` of a list block placed at `r`.
pub fn list_entry(r: Rect, index: usize) -> Rect {
    Rect::new(
        r.x,
        r.y + index as i32 * (card() as i32 + CARD_GAP),
        r.w,
        card(),
    )
}
/// The Edit button of a list entry.
pub fn list_edit_button(entry: Rect) -> Rect {
    let h = field_height();
    Rect::new(
        entry.right() - CARD_RIGHT - 120,
        entry.center_y() - h as i32 / 2,
        120,
        h,
    )
}
fn list_height(count: usize) -> u32 {
    if count == 0 {
        crate::palette::text_grow(28, door_graphics_text::TextStyle::Regular14)
    } else {
        count as u32 * (card() + CARD_GAP as u32) - CARD_GAP as u32
    }
}
/// Lines an InfoBar message takes at card width `width` (one or two; the
/// estimate errs towards two so the painter's wrap always fits).
pub fn info_lines(text: &str, width: u32) -> u32 {
    let room = width.saturating_sub(76) as usize;
    let text_width = text.len() * 37 / 5
        * crate::palette::text_grow(100, door_graphics_text::TextStyle::Regular14) as usize
        / 100;
    if text_width <= room { 1 } else { 2 }
}
/// Lines an introduction wraps to at `width` (an estimate of Segoe UI 14
/// px; the painter wraps by measuring), at most three.
pub fn intro_lines(text: &str, width: u32) -> u32 {
    let room = width.saturating_sub(4).max(1) as usize;
    let text_width = text.len() * 37 / 5
        * crate::palette::text_grow(100, door_graphics_text::TextStyle::Regular14) as usize
        / 100;
    text_width.div_ceil(room).clamp(1, 3) as u32
}
/// Introduction height: its lines and the gap to the first card.
pub fn intro_height(text: &str, width: u32) -> u32 {
    intro_lines(text, width)
        * crate::palette::text_grow(20, door_graphics_text::TextStyle::Regular14)
        + 16
}
/// InfoBar height: its message lines and 14 px padding, grown with text.
pub fn info_height(text: &str, width: u32) -> u32 {
    use door_graphics_text::TextStyle;
    let lines = info_lines(text, width);
    28 + lines * crate::palette::text_grow(20, TextStyle::Regular14)
}
/// The visible title a hero paints (deep links can name it).
pub const fn hero_title(hero: Hero) -> Option<&'static str> {
    match hero {
        Hero::DeviceSpecs => Some("Device specifications"),
        Hero::DoorsSpecs => Some("Doors specifications"),
        _ => None,
    }
}
/// The expander (by id) that holds a row titled `title` on `page`, so a
/// deep link can open it before scrolling there; `Some(id)` also when the
/// title is the expander's own header.
pub fn section_expander(page: Page, title: &str) -> Option<u8> {
    blocks_for(page).iter().find_map(|b| match b {
        Block::Expander(e)
            if e.header.title == title || e.rows.iter().any(|r| r.title == title) =>
        {
            Some(e.id)
        }
        _ => None,
    })
}
/// Where a deep-link section titled `title` lies in a placed page: the
/// first header, card, group, expander, hero or row with that title.
pub fn section_rect(layout: &PageLayout, title: &str) -> Option<Rect> {
    for placed in &layout.blocks {
        match placed.block {
            Block::Header(text) if text == title => return Some(placed.rect),
            Block::Hero(hero) if hero_title(hero) == Some(title) => return Some(placed.rect),
            Block::Card(row) if row.title == title => return Some(placed.rect),
            Block::Group(g) if g.title == title => return Some(placed.rect),
            Block::Expander(e) if e.header.title == title => return Some(placed.rect),
            _ => (),
        }
        if let Some((_, r)) = placed.rows.iter().find(|(row, _)| row.title == title) {
            return Some(*r);
        }
    }
    None
}
/// Settings' ContentDialogs (WinUI: 24 px padding, a 20 px semibold title,
/// labelled 32 px text boxes, and a footer of equal-width buttons).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogKind {
    /// Registry Editor > Go to a key: Key path; Open, Create, Cancel.
    RegistryGoTo,
    /// Registry Editor > New value: Name, Type, Data; Save, Cancel.
    RegistryValue,
    /// Registry Editor > Edit: Name (rename), Type, Data; Save, Delete, Cancel.
    RegistryEdit,
    /// Registry Editor > Delete key: confirmation; Delete, Cancel.
    RegistryDeleteKey,
    /// Environment variables > New: Name, Value; Save, Cancel.
    Variable,
    /// Environment variables > Edit: Name, Value; Save, Delete, Cancel.
    VariableEdit,
    /// File system > Check access: Path; Check, Cancel.
    CheckAccess,
    /// Sign-in options: current PIN and new PIN; OK, Cancel.
    Pin,
    /// Sign-in options (no PIN yet): password and new PIN; OK, Cancel.
    AddPin,
    /// Sign-in options: current password and new password; OK, Cancel.
    Password,
    /// About > Rename this PC: the new name; Save, Cancel.
    RenamePc,
    /// Ethernet > Edit IP settings: address/prefix and gateway (empty
    /// address: DHCP); Save, Cancel.
    IpSettings,
    /// Ethernet > Edit DNS settings: preferred and alternate servers (both
    /// empty: from DHCP); Save, Cancel.
    DnsSettings,
    /// Proxy > Edit proxy server: address and port (empty address: no
    /// proxy); Save, Cancel.
    Proxy,
    /// Proxy > Edit setup script: the script address (empty: no script);
    /// Save, Cancel.
    SetupScript,
    /// VPN > Add a VPN connection (provider WireGuard): the connection name
    /// and the provider's configuration file; Save, Cancel.
    AddVpn,
    /// Other users > Add account ("Create a user for this PC"): user name
    /// and password; Next, Cancel.
    AddAccount,
    /// An account's page > Change account type: Administrator, Standard
    /// User, Cancel.
    AccountType,
    /// An account's page > Remove: Delete account, Cancel.
    RemoveAccount,
}
impl DialogKind {
    /// Text box labels, in order (the registry Type combo follows Name).
    pub const fn fields(self) -> &'static [&'static str] {
        match self {
            Self::RegistryGoTo => &["Key path"],
            Self::RegistryValue | Self::RegistryEdit => &["Name", "Data"],
            Self::RegistryDeleteKey => &[],
            Self::Variable | Self::VariableEdit => &["Name", "Value"],
            Self::CheckAccess => &["File or folder path"],
            Self::Pin => &["Current PIN", "New PIN"],
            Self::AddPin => &["Password", "New PIN"],
            Self::Password => &["Current password", "New password"],
            Self::RenamePc => &["New name"],
            Self::IpSettings => &["IP address / subnet prefix length", "Gateway"],
            Self::DnsSettings => &["Preferred DNS", "Alternate DNS"],
            Self::Proxy => &["Proxy IP address", "Port"],
            Self::SetupScript => &["Script address"],
            Self::AddVpn => &["Connection name", "Configuration file (WireGuard)"],
            Self::AddAccount => &["User name", "Password"],
            Self::AccountType | Self::RemoveAccount => &[],
        }
    }
    /// Button labels: primary (accent), then secondary, then close.
    pub const fn buttons(self) -> &'static [&'static str] {
        match self {
            Self::RegistryGoTo => &["Open", "Create", "Cancel"],
            Self::RegistryValue | Self::Variable => &["Save", "Cancel"],
            Self::RegistryEdit | Self::VariableEdit => &["Save", "Delete", "Cancel"],
            Self::RegistryDeleteKey => &["Delete", "Cancel"],
            Self::CheckAccess => &["Check", "Cancel"],
            Self::Pin | Self::AddPin | Self::Password => &["OK", "Cancel"],
            Self::RenamePc
            | Self::IpSettings
            | Self::DnsSettings
            | Self::Proxy
            | Self::SetupScript
            | Self::AddVpn => &["Save", "Cancel"],
            Self::AddAccount => &["Next", "Cancel"],
            Self::AccountType => &["Administrator", "Standard User", "Cancel"],
            Self::RemoveAccount => &["Delete account", "Cancel"],
        }
    }
    pub const fn has_type(self) -> bool {
        matches!(self, Self::RegistryValue | Self::RegistryEdit)
    }
    /// Help lines under the fields.
    pub const fn help(self) -> &'static [&'static str] {
        match self {
            Self::RegistryGoTo => &[
                "For example HKEY_CURRENT_USER\\Software\\Door.",
                "Create makes the key if it doesn't exist yet.",
            ],
            Self::RegistryDeleteKey => &[
                "The key and its values are deleted. Keys with subkeys can't be deleted.",
                "",
            ],
            Self::RegistryValue | Self::RegistryEdit => &[
                "Numbers: decimal or 0x hex. Binary: pairs of hex digits.",
                "Text: \\n, \\0, \\\\, \\uXXXX. Lists: separate items with \\0.",
            ],
            Self::Variable | Self::VariableEdit => {
                &["Names and values use letters, digits and symbols.", ""]
            }
            Self::CheckAccess => &["For example /users/1001/Documents.", ""],
            Self::Pin | Self::AddPin => &["A PIN has 4 to 16 digits.", ""],
            Self::Password => &["A password has at least 4 characters.", ""],
            Self::RenamePc => &[
                "Use up to 15 letters, numbers and hyphens.",
                "The name can't be only numbers.",
            ],
            Self::IpSettings => &[
                "For example 192.168.1.20/24 and 192.168.1.1.",
                "Leave the address empty to get one automatically (DHCP).",
            ],
            Self::DnsSettings => &[
                "For example 1.1.1.1 and 9.9.9.9.",
                "Leave both empty to use the servers DHCP provides.",
            ],
            Self::Proxy => &[
                "Local (intranet) addresses don't use the proxy server.",
                "Leave the address empty to stop using a proxy server.",
            ],
            Self::SetupScript => &[
                "For example http://wpad.example.com/proxy.pac.",
                "Leave the address empty to stop using a setup script.",
            ],
            Self::AddVpn => &[
                "VPN provider: WireGuard. Use the .conf file from your VPN provider,",
                "for example C:\\Users\\you\\Downloads\\office.conf.",
            ],
            Self::AddAccount => &[
                "Use up to 20 characters. New accounts are standard users.",
                "A password has at least 4 characters.",
            ],
            Self::AccountType => &[
                "Administrators can install apps and change settings",
                "that affect everyone who uses this PC.",
            ],
            Self::RemoveAccount => &[
                "The account can no longer sign in. Its files stay in its",
                "folder under C:\\Users.",
            ],
        }
    }
    /// Text boxes show bullets instead of their text.
    pub const fn secret(self) -> bool {
        matches!(self, Self::Pin | Self::AddPin | Self::Password)
    }
    /// Text box `index` shows bullets (Add account: the password only).
    pub const fn secret_field(self, index: usize) -> bool {
        self.secret() || (matches!(self, Self::AddAccount) && index == 1)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DialogGeometry {
    pub rect: Rect,
    /// Text boxes, in `fields()` order (unused entries are empty).
    pub fields: [Rect; 2],
    /// The registry Type combo (empty for other dialogs).
    pub kind: Rect,
    /// Top of the first help line and of the error line.
    pub help: i32,
    pub error: i32,
    /// Footer band and its buttons in `buttons()` order.
    pub footer: Rect,
    pub buttons: [Rect; 3],
}
const DIALOG_PAD: i32 = 24;
/// Placement of `kind` centred in a `width` x `height` client.
pub fn dialog_geometry(kind: DialogKind, width: u32, height: u32) -> DialogGeometry {
    use door_graphics_text::TextStyle;
    let grow = |base: u32, style| crate::palette::text_grow(base, style) as i32;
    let w = width.saturating_sub(48).clamp(320, 548).min(width);
    let label = grow(20, TextStyle::Regular14);
    let field = field_height() as i32;
    let row = label + 4 + field + 12;
    let mut y = DIALOG_PAD + grow(28, TextStyle::Semibold20) + 16;
    let inner = (w as i32 - 2 * DIALOG_PAD).max(0) as u32;
    let mut fields = [Rect::default(); 2];
    let mut kind_rect = Rect::default();
    for (i, _) in kind.fields().iter().enumerate() {
        if i == 1 && kind.has_type() {
            kind_rect = Rect::new(DIALOG_PAD, y + label + 4, inner.min(240), field as u32);
            y += row;
        }
        fields[i] = Rect::new(DIALOG_PAD, y + label + 4, inner, field as u32);
        y += row;
    }
    let help = y;
    y += 2 * grow(18, TextStyle::Regular12);
    let error = y;
    y += grow(20, TextStyle::Regular14) + 12;
    let footer_h = 80;
    let h = (y + footer_h) as u32;
    let x0 = (width as i32 - w as i32) / 2;
    let y0 = ((height as i32 - h as i32) / 2).max(0);
    let shift = |r: Rect| {
        if r == Rect::default() {
            r
        } else {
            Rect::new(r.x + x0, r.y + y0, r.w, r.h)
        }
    };
    let footer = Rect::new(x0, y0 + y, w, footer_h as u32);
    let n = kind.buttons().len() as i32;
    let bw = ((inner as i32 - (n - 1) * 8) / n).max(0) as u32;
    let mut buttons = [Rect::default(); 3];
    for (i, b) in buttons.iter_mut().enumerate().take(n as usize) {
        *b = Rect::new(
            x0 + DIALOG_PAD + i as i32 * (bw as i32 + 8),
            footer.y + DIALOG_PAD,
            bw,
            32,
        );
    }
    DialogGeometry {
        rect: Rect::new(x0, y0, w, h),
        fields: [shift(fields[0]), shift(fields[1])],
        kind: shift(kind_rect),
        help: y0 + help,
        error: y0 + error,
        footer,
        buttons,
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slider {
    /// Accessibility > Text size, 100..=225%.
    TextSize,
    /// System > Sound: master volume 0..=100 and left/right balance
    /// -100..=100 (the SOUND preference word).
    Volume,
    Balance,
    /// System > Sound > Input: the microphone level 0..=100 (kept in the
    /// MICROPHONE word beside the privacy bits).
    InputVolume,
    /// System > Display: brightness 0..=100 (the VISION word; applied in
    /// software at scanout).
    Brightness,
}
impl Slider {
    pub const fn range(self) -> (i32, i32) {
        match self {
            Self::TextSize => (TEXT_MIN as i32, TEXT_MAX as i32),
            Self::Volume | Self::InputVolume => (0, 100),
            Self::Balance => (-100, 100),
            Self::Brightness => (0, 100),
        }
    }
    fn output(state: State) -> door_service_protocol::audio::Output {
        door_service_protocol::audio::Output::from_word(state.pref(p::SOUND)).unwrap_or_default()
    }
    /// The value shown (card sliders; the text size slider shows its pick).
    pub fn value(self, state: State) -> i32 {
        match self {
            Self::TextSize => state.text_pick as i32,
            Self::Volume => i32::from(Self::output(state).volume),
            Self::Balance => i32::from(Self::output(state).balance),
            Self::InputVolume => i32::from(capture::input_of(state.pref(p::MICROPHONE)).volume),
            Self::Brightness => p::vision::brightness(state.pref(p::VISION)) as i32,
        }
    }
    /// The value a pointer at `x` picks on `track`.
    pub fn value_at(self, track: Rect, x: i32) -> i32 {
        let (lo, hi) = self.range();
        let along = (x - track.x).clamp(0, track.w as i32);
        lo + (along * (hi - lo) + track.w as i32 / 2) / (track.w as i32).max(1)
    }
    /// Centre x of the thumb at `value`.
    pub fn thumb(self, track: Rect, value: i32) -> i32 {
        let (lo, hi) = self.range();
        track.x + ((value.clamp(lo, hi) - lo) * track.w as i32 + (hi - lo) / 2) / (hi - lo)
    }
    /// Setting `value` on a card slider.
    pub fn apply(self, state: State, value: i32) -> Option<Change> {
        if self == Self::Brightness {
            let word = state.pref(p::VISION);
            let next = p::vision::with_brightness(word, value.clamp(0, 100) as u32);
            return (next != word).then_some(Change::Preference(p::VISION, next));
        }
        if self == Self::InputVolume {
            let word = state.pref(p::MICROPHONE);
            let level = door_service_protocol::audio::Input {
                volume: value.clamp(0, 100) as u8,
                muted: false,
            };
            let next = capture::with_input(word, level);
            return (next != word).then_some(Change::Preference(p::MICROPHONE, next));
        }
        let mut o = Self::output(state);
        match self {
            Self::TextSize => return None,
            Self::Volume => {
                o.volume = value.clamp(0, 100) as u8;
                o.muted = false;
            }
            Self::Balance => o.balance = value.clamp(-100, 100) as i8,
            Self::Brightness | Self::InputVolume => return None,
        }
        (o != Self::output(state)).then(|| Change::Preference(p::SOUND, o.word()))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    /// A `Controls` flag. `inverted`: the switch is On while the bit is clear
    /// (Transparency effects On = REDUCE_TRANSPARENCY clear).
    Control {
        flag: u32,
        inverted: bool,
    },
    /// No provider. Shown disabled in this fixed state.
    Unavailable(bool),
    /// Adjust for daylight saving time automatically: on (and fixed, since
    /// Doors always applies the zone's rules) when the zone observes DST.
    Daylight,
    /// Bit `bit` of preference word `id` (`door_service_protocol::preferences`).
    Pref {
        id: u32,
        bit: u32,
    },
    /// As `Pref`, on while the bit is clear (a word that stores "off").
    PrefOff {
        id: u32,
        bit: u32,
    },
    /// Advanced network settings > Network adapters: the Ethernet adapter
    /// is enabled (off stops its traffic; Airplane mode only stops radios).
    Adapter,
    /// Proxy > Automatically detect settings (Internet Settings'
    /// DefaultConnectionSettings, as Windows keeps it).
    ProxyAutoDetect,
    /// Firewall & network protection: profile `0..3` (domain, private,
    /// public) is on, or blocks every incoming connection.
    Firewall(u8),
    FirewallShields(u8),
}
impl Toggle {
    pub const fn control(flag: u32) -> Self {
        Self::Control {
            flag,
            inverted: false,
        }
    }
    pub const fn inverted(flag: u32) -> Self {
        Self::Control {
            flag,
            inverted: true,
        }
    }
    pub fn enabled(self) -> bool {
        matches!(
            self,
            Self::Control { .. }
                | Self::Pref { .. }
                | Self::PrefOff { .. }
                | Self::Adapter
                | Self::ProxyAutoDetect
                | Self::Firewall(_)
                | Self::FirewallShields(_)
        )
    }
    /// Whether the switch can change now: Privacy & security > Microphone's
    /// app switches need "Microphone access" on (Windows greys them and
    /// shows them Off otherwise).
    pub fn live(self, state: State) -> bool {
        match self {
            Self::PrefOff {
                id: p::MICROPHONE,
                bit,
            } if bit != door_service_protocol::audio::capture::ACCESS_OFF => {
                state.pref(p::MICROPHONE) & door_service_protocol::audio::capture::ACCESS_OFF == 0
            }
            _ => self.enabled(),
        }
    }
    pub fn value(self, state: State) -> bool {
        match self {
            Self::PrefOff {
                id: p::MICROPHONE,
                bit,
            } => {
                state.pref(p::MICROPHONE)
                    & (bit | door_service_protocol::audio::capture::ACCESS_OFF)
                    == 0
            }
            Self::Control { flag, inverted } => (state.controls.bits() & flag != 0) != inverted,
            Self::Unavailable(on) => on,
            Self::Daylight => door_locale::tz::zone_or_utc(state.zone).observes_dst(),
            Self::Pref { id, bit } => state.pref(id) & bit != 0,
            Self::PrefOff { id, bit } => state.pref(id) & bit == 0,
            Self::Adapter => state.net.is_some_and(|n| n.present && n.config.enabled),
            Self::ProxyAutoDetect => state.proxy_auto_detect,
            Self::Firewall(i) => state.firewall[usize::from(i) % 3] & FIREWALL_ON != 0,
            Self::FirewallShields(i) => state.firewall[usize::from(i) % 3] & FIREWALL_SHIELDS != 0,
        }
    }
    pub fn flip(self, state: State) -> Option<Change> {
        if !self.live(state) {
            return None;
        }
        match self {
            Self::Control { flag, .. } => Some(Change::Controls(
                state.controls.with(flag, state.controls.bits() & flag == 0),
            )),
            Self::Pref { id, bit } | Self::PrefOff { id, bit } => {
                Some(Change::Preference(id, state.pref(id) ^ bit))
            }
            Self::Adapter => state.net.filter(|n| n.present).map(|n| {
                Change::Network(door_service_protocol::net::Config {
                    enabled: !n.config.enabled,
                    ..n.config
                })
            }),
            Self::ProxyAutoDetect => Some(Change::ProxyAutoDetect(!state.proxy_auto_detect)),
            Self::Firewall(i) | Self::FirewallShields(i) => {
                let bit = if matches!(self, Self::Firewall(_)) {
                    FIREWALL_ON
                } else {
                    FIREWALL_SHIELDS
                };
                let i = i % 3;
                Some(Change::Firewall(i, state.firewall[usize::from(i)] ^ bit))
            }
            Self::Unavailable(_) | Self::Daylight => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    /// Choose your mode: Light / Dark / Custom (no separate app mode).
    Mode,
    /// Taskbar alignment: Left / Center (Top / Center on vertical edges).
    Alignment,
    /// Taskbar position (26H2): Bottom / Top / Left / Right.
    Position,
    /// Taskbar size (26H2): Small / Default (`Controls::small`).
    TaskbarSize,
    /// Start size (26H2): Automatic / Small / Large.
    StartSize,
    /// Taskbar search: Hide / Search icon only / Search icon and label /
    /// Search box. Top and side taskbars force the icon (26H2).
    Search,
    /// Context menu Properties placement: Top / Bottom.
    PropertiesPlacement,
    /// Personalize your background: Picture / Solid color / Slideshow / Spotlight.
    BackgroundKind,
    /// Accessibility > Contrast themes: None / Aquatic / Desert / Dusk /
    /// Night sky. Selecting only lists it; Apply applies.
    Contrast,
    /// System > Display > Scale: 100%..300%; the recommended one stores
    /// automatic.
    Scale,
    /// Time & language > Typing: Override for default input method (the
    /// installed keyboards and input methods).
    InputMethod,
    /// Time & language > Date & time > Time zone (`door_locale::tz::ZONES`).
    TimeZone,
    /// Language & region > Regional format (`door_locale::region::REGIONS`).
    RegionalFormat,
    /// Registry Editor > Root key.
    RegistryRoot,
    /// Registry Editor > Registry view: 64-bit / 32-bit.
    RegistryView,
    /// Personalization > Colors > Accent color: Manual / Automatic.
    AccentMode,
    /// Accounts > Sign-in options > "If you've been away" (minutes).
    SignInAway,
    /// Colors > More color options: light / dark mode from (hours).
    ScheduleLight,
    ScheduleDark,
    /// Notifications > Do not disturb automatically: from / to (hours), repeats.
    DndFrom,
    DndTo,
    DndRepeat,
    /// Notifications > Focus: session length.
    FocusLength,
    /// Personalization > Lock screen: what the lock screen shows.
    LockScreen,
    /// Language & region > Regional format: first day of week / clock.
    FirstDay,
    ClockFormat,
    /// Taskbar behaviors: combine buttons and hide labels; the material.
    Combine,
    TaskbarLook,
    /// Background: kind, fit, slideshow interval.
    BackgroundFit,
    SlideshowInterval,
    /// Window appearance (Doors extras): corners, animation speed.
    Corners,
    AnimationSpeed,
    /// Mouse: pointer speed 1..=20, primary button, lines per notch.
    PointerSpeed,
    PrimaryButton,
    ScrollLines,
    /// Night light strength; colour filter kind.
    NightStrength,
    FilterKind,
    /// No provider: options shown, current index fixed, disabled.
    Unavailable(&'static [&'static str], u8),
}
/// Hourly times for schedule choices.
const HOURS: [&str; 24] = [
    "12:00 AM", "1:00 AM", "2:00 AM", "3:00 AM", "4:00 AM", "5:00 AM", "6:00 AM", "7:00 AM",
    "8:00 AM", "9:00 AM", "10:00 AM", "11:00 AM", "12:00 PM", "1:00 PM", "2:00 PM", "3:00 PM",
    "4:00 PM", "5:00 PM", "6:00 PM", "7:00 PM", "8:00 PM", "9:00 PM", "10:00 PM", "11:00 PM",
];
/// Background > Solid color choices (0 stores Windows blue, the default).
const NUMBERS_1_20: [&str; 20] = [
    "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "17",
    "18", "19", "20",
];
const STRENGTHS: [u32; 7] = [0, 20, 40, 50, 60, 80, 100];
const STRENGTH_LABELS: [&str; 7] = ["0%", "20%", "40%", "50%", "60%", "80%", "100%"];
/// Background > Solid color swatches: the Desktop Background control
/// panel's palette (themecpl.dll), black first. The word's colour 0 means
/// the default (0x0063B1), so black is stored as 0x010101.
pub const SOLID_PALETTE: [u32; 33] = [
    0x010101, 0xFFFFFF, 0xDCDCDC, 0x787878, 0x464646, 0x0A3B76, 0x4395D1, 0x99D9EA, 0x0076A3,
    0x0D686B, 0x00A99D, 0x7ACCC8, 0x82CA9C, 0x74A402, 0xC4DF9B, 0x84871C, 0xD9D56F, 0xFFF468,
    0xFFF799, 0xFFC20E, 0xEB6119, 0xFBAF5D, 0xE57300, 0xC14000, 0x9E6B52, 0x8C6239, 0xC7B299,
    0xB82832, 0xD85171, 0xFEDFEC, 0x563F7F, 0xA186BE, 0xD9CFE5,
];
const FOCUS_LABELS: [&str; 12] = [
    "5 mins",
    "10 mins",
    "15 mins",
    "20 mins",
    "30 mins",
    "45 mins",
    "1 hour",
    "1.5 hours",
    "2 hours",
    "2.5 hours",
    "3 hours",
    "4 hours",
];
const AWAY_LABELS: [&str; 6] = [
    "Never",
    "1 minute",
    "3 minutes",
    "5 minutes",
    "15 minutes",
    "30 minutes",
];
const LOCK_LABELS: [&str; 5] = [
    "Same as desktop background",
    crate::settings_layout::BACKGROUNDS[0],
    crate::settings_layout::BACKGROUNDS[1],
    crate::settings_layout::BACKGROUNDS[2],
    crate::settings_layout::BACKGROUNDS[3],
];
const WEEKDAYS: [&str; 8] = [
    "Use regional format",
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
impl Choice {
    pub fn options(self) -> &'static [&'static str] {
        match self {
            Self::Mode => &["Light", "Dark", "Custom"],
            Self::Alignment => &["Left", "Center"],
            Self::Position => &["Bottom", "Top", "Left", "Right"],
            Self::TaskbarSize => &["Small", "Default", "Large"],
            Self::StartSize => &["Automatic", "Small", "Large"],
            Self::Search => &[
                "Hide",
                "Search icon only",
                "Search icon and label",
                "Search box",
            ],
            Self::PropertiesPlacement => &["Top of the menu", "Bottom of the menu"],
            Self::BackgroundKind => &["Picture", "Solid color", "Slideshow", "Doors spotlight"],
            Self::Contrast => &["None", "Aquatic", "Desert", "Dusk", "Night sky"],
            Self::Scale => &SCALE_LABELS,
            Self::RegistryRoot => &[
                "HKEY_CURRENT_USER",
                "HKEY_LOCAL_MACHINE",
                "HKEY_CLASSES_ROOT",
            ],
            Self::RegistryView => &["64-bit", "32-bit"],
            Self::InputMethod | Self::TimeZone | Self::RegionalFormat => &[],
            Self::AccentMode => &["Manual", "Automatic"],
            Self::SignInAway => &AWAY_LABELS,
            Self::ScheduleLight | Self::ScheduleDark | Self::DndFrom | Self::DndTo => &HOURS,
            Self::DndRepeat => &door_service_protocol::preferences::DndSchedule::REPEATS,
            Self::FocusLength => &FOCUS_LABELS,
            Self::LockScreen => &LOCK_LABELS,
            Self::FirstDay => &WEEKDAYS,
            Self::ClockFormat => &["Use regional format", "12-hour", "24-hour"],
            Self::Combine => &["Always", "When taskbar is full", "Never"],
            Self::TaskbarLook => &["Default", "Translucent", "Opaque", "Clear"],
            Self::BackgroundFit => &p::Background::FITS,
            Self::SlideshowInterval => &p::Background::INTERVALS,
            Self::Corners => &["Rounded", "Square"],
            Self::AnimationSpeed => &["Normal", "Fast", "Slow"],
            Self::PointerSpeed | Self::ScrollLines => &NUMBERS_1_20,
            Self::PrimaryButton => &["Left", "Right"],
            Self::NightStrength => &STRENGTH_LABELS,
            Self::FilterKind => &p::vision::KINDS,
            Self::Unavailable(options, _) => options,
        }
    }
    /// Label shown for option `index` in the current state (vertical
    /// taskbars align to the top, as the Doors shell does).
    pub fn label(self, index: usize, state: State) -> &'static str {
        if self == Self::Alignment && index == 0 && Self::vertical(state) {
            return "Top";
        }
        if self == Self::InputMethod {
            return state
                .input_methods()
                .ids()
                .nth(index)
                .map_or("", input_label);
        }
        if self == Self::TimeZone {
            return door_locale::tz::ZONES.get(index).map_or("", |z| z.display);
        }
        if self == Self::RegionalFormat {
            return door_locale::region::REGIONS
                .get(index)
                .map_or("", |r| r.name);
        }
        if self == Self::Scale {
            return match scale_entry(state, index) {
                Some(i) if SCALES[i] == state.recommended => SCALE_RECOMMENDED[i],
                Some(i) => SCALE_LABELS[i],
                None => "",
            };
        }
        self.options().get(index).copied().unwrap_or("")
    }
    /// Entries in the list for `state` (Scale lists the offered scales).
    pub fn count(self, state: State) -> usize {
        match self {
            Self::Scale => offered(state).count(),
            Self::InputMethod => state.input_methods().count(),
            Self::TimeZone => door_locale::tz::ZONES.len(),
            Self::RegionalFormat => door_locale::region::REGIONS.len(),
            _ => self.options().len(),
        }
    }
    fn vertical(state: State) -> bool {
        matches!(
            state.controls.edge(),
            door_service_protocol::desktop::Edge::Left
                | door_service_protocol::desktop::Edge::Right
        )
    }
    /// The taskbar is not at the bottom edge (26H2 shows the search icon only).
    pub fn alternate_position(state: State) -> bool {
        state.controls.edge() != door_service_protocol::desktop::Edge::Bottom
    }
    /// Options with a provider in any state; a choice with none is disabled.
    pub fn enabled_mask(self) -> u64 {
        match self {
            Self::Mode => 0b011,
            Self::Alignment => 0b11,
            Self::Position => 0b1111,
            Self::TaskbarSize => 0b111,
            Self::StartSize => 0b111,
            Self::Search => 0b1010,
            Self::PropertiesPlacement => 0b11,
            Self::BackgroundKind => 0b111,
            Self::Contrast => 0b1_1111,
            Self::Scale => ALL_SCALES as u64,
            Self::InputMethod => (1 << door_keyboard_layouts::methods::MAX_METHODS) - 1,
            Self::TimeZone => (1u64 << door_locale::tz::ZONES.len()) - 1,
            Self::RegionalFormat => (1u64 << door_locale::region::REGIONS.len()) - 1,
            Self::RegistryRoot => 0b111,
            Self::RegistryView => 0b11,
            Self::AccentMode
            | Self::SignInAway
            | Self::ScheduleLight
            | Self::ScheduleDark
            | Self::DndFrom
            | Self::DndTo
            | Self::DndRepeat
            | Self::FocusLength
            | Self::LockScreen
            | Self::FirstDay
            | Self::ClockFormat
            | Self::Combine
            | Self::TaskbarLook
            | Self::BackgroundFit
            | Self::SlideshowInterval
            | Self::Corners
            | Self::AnimationSpeed
            | Self::PointerSpeed
            | Self::PrimaryButton
            | Self::ScrollLines
            | Self::NightStrength
            | Self::FilterKind => (1u64 << self.options().len()) - 1,
            Self::Unavailable(..) => 0,
        }
    }
    /// Options selectable in `state`.
    pub fn mask(self, state: State) -> u64 {
        match self {
            Self::Search if Self::alternate_position(state) => 0b0010,
            // Only offered scales are listed, and all of them apply.
            Self::Scale | Self::InputMethod => (1u64 << self.count(state)) - 1,
            _ => self.enabled_mask(),
        }
    }
    pub fn enabled(self) -> bool {
        self.enabled_mask() != 0
    }
    pub fn current(self, state: State) -> usize {
        let c = state.controls;
        match self {
            Self::Mode => usize::from(state.appearance & 1 != 0),
            Self::Alignment => usize::from(!c.start_aligned()),
            Self::Position => c.edge() as usize,
            Self::TaskbarSize => {
                if state.pref(p::TASKBAR) & p::taskbar::LARGE != 0 {
                    2
                } else {
                    usize::from(!c.small())
                }
            }
            Self::StartSize => match c.start_size() {
                1 => 1,
                2 => 2,
                _ => 0,
            },
            Self::Search => {
                if c.search_icon_only() || Self::alternate_position(state) {
                    1
                } else {
                    3
                }
            }
            Self::PropertiesPlacement => usize::from(c.properties_bottom()),
            Self::BackgroundKind => background(state).kind as usize,
            Self::Contrast => usize::from(state.contrast_pick),
            Self::InputMethod => state.input_methods().current_index(),
            Self::TimeZone => door_locale::tz::index_of(state.zone)
                .or_else(|| door_locale::tz::index_of(u32::from(door_locale::tz::DEFAULT_ID)))
                .unwrap_or(0),
            Self::RegionalFormat => door_locale::region::index_of(state.region).unwrap_or(0),
            Self::RegistryRoot => usize::from(state.registry_root.min(2)),
            Self::RegistryView => usize::from(state.registry_view.min(1)),
            Self::Scale => {
                let percent = if state.scale == 0 {
                    state.recommended
                } else {
                    state.scale
                };
                offered(state)
                    .position(|i| SCALES[i] == percent)
                    .unwrap_or(0)
            }
            Self::AccentMode => usize::from(
                door_service_protocol::desktop::Accent::from_bits(state.accent).is_some_and(|a| {
                    a.mode() == door_service_protocol::desktop::AccentMode::Automatic
                }),
            ),
            Self::SignInAway => {
                let minutes = state.pref(p::SIGN_IN_AWAY);
                p::SIGN_IN_AWAY_MINUTES
                    .iter()
                    .position(|m| *m == minutes)
                    .unwrap_or(0)
            }
            Self::ScheduleLight => (theme_schedule(state).light_from / 60) as usize,
            Self::ScheduleDark => (theme_schedule(state).dark_from / 60) as usize,
            Self::DndFrom => (dnd_schedule(state).from / 60) as usize,
            Self::DndTo => (dnd_schedule(state).to / 60) as usize,
            Self::DndRepeat => dnd_schedule(state).repeat as usize,
            Self::FocusLength => p::Focus::LENGTHS
                .iter()
                .position(|m| *m == focus(state).minutes)
                .unwrap_or(0),
            Self::LockScreen => {
                let kind =
                    p::LockScreen::from_bits(state.pref(p::LOCK_SCREEN)).map_or(0, |l| l.kind());
                if kind <= 4 { kind as usize } else { 0 }
            }
            Self::FirstDay => overrides(state).first_day as usize,
            Self::ClockFormat => overrides(state).clock as usize,
            Self::Combine => p::taskbar::combine(state.pref(p::TASKBAR)) as usize,
            Self::TaskbarLook => p::taskbar::look(state.pref(p::TASKBAR)) as usize,
            Self::BackgroundFit => background(state).fit as usize,
            Self::SlideshowInterval => background(state).interval as usize,
            Self::Corners => usize::from(state.pref(p::WINDOWS) & p::windows::SQUARE_CORNERS != 0),
            Self::AnimationSpeed => p::windows::speed(state.pref(p::WINDOWS)) as usize,
            Self::PointerSpeed => p::mouse::speed(state.pref(p::MOUSE)).saturating_sub(1) as usize,
            Self::ScrollLines => p::mouse::lines(state.pref(p::MOUSE)).saturating_sub(1) as usize,
            Self::PrimaryButton => usize::from(state.pref(p::MOUSE) & p::mouse::SWAP_BUTTONS != 0),
            Self::NightStrength => STRENGTHS
                .iter()
                .position(|s| *s == p::vision::strength(state.pref(p::VISION)))
                .unwrap_or(3),
            Self::FilterKind => p::vision::kind(state.pref(p::VISION)) as usize,
            Self::Unavailable(_, index) => index as usize,
        }
    }
    /// Selecting `index`. Disabled options and providerless choices never apply.
    pub fn apply(self, index: usize, state: State) -> Option<Change> {
        if index >= 64 || self.mask(state) & (1 << index) == 0 {
            return None;
        }
        let c = state.controls;
        Some(match self {
            Self::Mode => Change::Appearance((state.appearance & !1) | index as u32),
            Self::Alignment => Change::Controls(c.with(4, index == 0)),
            Self::Position => {
                Change::Controls(Controls::from_bits((c.bits() & !3) | index as u32)?)
            }
            Self::TaskbarSize => Change::TaskbarSize(
                c.with(8, index == 0),
                state.pref(p::TASKBAR) & !p::taskbar::LARGE
                    | if index == 2 { p::taskbar::LARGE } else { 0 },
            ),
            Self::StartSize => Change::Controls(Controls::from_bits(
                (c.bits() & !(3 << 13)) | ((index as u32) << 13),
            )?),
            Self::Search => Change::Controls(c.with(Controls::SEARCH_ICON_ONLY, index == 1)),
            Self::PropertiesPlacement => Change::Controls(c.with(32, index == 1)),
            Self::BackgroundKind => {
                let mut b = background(state);
                b.kind = index as u32;
                Change::Preference(p::BACKGROUND, b.bits())
            }
            Self::Contrast => Change::ContrastPick(index as u8),
            Self::InputMethod => Change::InputMethods(state.input_methods().select(index)?.bits()),
            Self::TimeZone => Change::TimeZone(u32::from(door_locale::tz::ZONES.get(index)?.id)),
            Self::RegionalFormat => {
                Change::RegionalFormat(u32::from(door_locale::region::REGIONS.get(index)?.id))
            }
            Self::RegistryRoot => Change::RegistryLocation(index as u8, state.registry_view),
            Self::RegistryView => Change::RegistryLocation(state.registry_root, index as u8),
            Self::Scale => {
                let percent = SCALES[scale_entry(state, index)?];
                Change::Scale(if percent == state.recommended {
                    0
                } else {
                    percent
                })
            }
            Self::AccentMode => {
                use door_service_protocol::desktop::{Accent, AccentMode};
                let now = Accent::from_bits(state.accent).unwrap_or_default();
                Change::Accent(match (index, now.mode()) {
                    (1, _) => Accent::AUTOMATIC.bits(),
                    // Manual keeps the colour picked last (the default blue
                    // after Automatic).
                    (_, AccentMode::Automatic) => Accent::DEFAULT.bits(),
                    _ => now.bits(),
                })
            }
            Self::SignInAway => {
                Change::Preference(p::SIGN_IN_AWAY, *p::SIGN_IN_AWAY_MINUTES.get(index)?)
            }
            Self::ScheduleLight | Self::ScheduleDark => {
                let mut s = theme_schedule(state);
                if self == Self::ScheduleLight {
                    s.light_from = index as u32 * 60;
                } else {
                    s.dark_from = index as u32 * 60;
                }
                Change::Preference(p::THEME_SCHEDULE, s.bits())
            }
            Self::DndFrom | Self::DndTo | Self::DndRepeat => {
                let mut s = dnd_schedule(state);
                match self {
                    Self::DndFrom => s.from = index as u32 * 60,
                    Self::DndTo => s.to = index as u32 * 60,
                    _ => s.repeat = index as u32,
                }
                Change::Preference(p::DND_SCHEDULE, s.bits())
            }
            Self::FocusLength => {
                let mut f = focus(state);
                f.minutes = *p::Focus::LENGTHS.get(index)?;
                Change::Preference(p::FOCUS, f.bits())
            }
            Self::LockScreen => Change::Preference(
                p::LOCK_SCREEN,
                if index == 0 {
                    p::LockScreen::DESKTOP
                } else {
                    p::LockScreen::picture(index as u32 - 1)?
                }
                .bits(),
            ),
            Self::FirstDay | Self::ClockFormat => {
                let mut o = overrides(state);
                if self == Self::FirstDay {
                    o.first_day = index as u32;
                } else {
                    o.clock = index as u32;
                }
                Change::Preference(p::REGIONAL_OVERRIDES, o.bits())
            }
            Self::Combine => Change::Preference(
                p::TASKBAR,
                (state.pref(p::TASKBAR) & !p::taskbar::COMBINE_MASK) | index as u32,
            ),
            Self::TaskbarLook => Change::Preference(
                p::TASKBAR,
                (state.pref(p::TASKBAR) & !p::taskbar::LOOK_MASK)
                    | (index as u32) << p::taskbar::LOOK_SHIFT,
            ),
            Self::BackgroundFit | Self::SlideshowInterval => {
                let mut b = background(state);
                if self == Self::BackgroundFit {
                    b.fit = index as u32;
                } else {
                    b.interval = index as u32;
                }
                Change::Preference(p::BACKGROUND, b.bits())
            }
            Self::Corners => Change::Preference(
                p::WINDOWS,
                (state.pref(p::WINDOWS) & !p::windows::SQUARE_CORNERS) | index as u32,
            ),
            Self::AnimationSpeed => Change::Preference(
                p::WINDOWS,
                (state.pref(p::WINDOWS) & !p::windows::SPEED_MASK)
                    | (index as u32) << p::windows::SPEED_SHIFT,
            ),
            Self::PointerSpeed => Change::Preference(
                p::MOUSE,
                (state.pref(p::MOUSE) & !p::mouse::SPEED_MASK) | (index as u32 + 1),
            ),
            Self::ScrollLines => Change::Preference(
                p::MOUSE,
                (state.pref(p::MOUSE) & !p::mouse::LINES_MASK)
                    | (index as u32 + 1) << p::mouse::LINES_SHIFT,
            ),
            Self::PrimaryButton => Change::Preference(
                p::MOUSE,
                (state.pref(p::MOUSE) & !p::mouse::SWAP_BUTTONS)
                    | if index == 1 {
                        p::mouse::SWAP_BUTTONS
                    } else {
                        0
                    },
            ),
            Self::NightStrength => Change::Preference(
                p::VISION,
                (state.pref(p::VISION) & !p::vision::STRENGTH_MASK)
                    | *STRENGTHS.get(index)? << p::vision::STRENGTH_SHIFT,
            ),
            Self::FilterKind => Change::Preference(
                p::VISION,
                (state.pref(p::VISION) & !p::vision::KIND_MASK)
                    | (index as u32) << p::vision::KIND_SHIFT,
            ),
            Self::Unavailable(..) => return None,
        })
    }
    /// Closed-combo arrow keys move to the adjacent enabled option (WinUI).
    pub fn step(self, state: State, down: bool) -> Option<usize> {
        let count = self.count(state);
        let mut index = self.current(state);
        loop {
            index = if down {
                index.checked_add(1).filter(|i| *i < count)?
            } else {
                index.checked_sub(1)?
            };
            if self.mask(state) & (1 << index) != 0 {
                return Some(index);
            }
        }
    }
    pub fn width(self) -> u32 {
        match self {
            Self::Mode | Self::TaskbarSize | Self::StartSize => 160,
            Self::Alignment | Self::Position => 140,
            Self::Search | Self::PropertiesPlacement => 220,
            Self::BackgroundKind => 200,
            Self::Contrast => 200,
            Self::Scale => 200,
            Self::InputMethod => 280,
            Self::TimeZone => 420,
            Self::RegionalFormat => 240,
            Self::RegistryRoot => 240,
            Self::RegistryView => 140,
            Self::AccentMode => 160,
            Self::SignInAway | Self::FirstDay | Self::ClockFormat => 220,
            Self::ScheduleLight | Self::ScheduleDark | Self::DndFrom | Self::DndTo => 140,
            Self::DndRepeat | Self::FocusLength => 160,
            Self::LockScreen => 260,
            Self::Combine => 220,
            Self::TaskbarLook | Self::BackgroundFit | Self::SlideshowInterval => 160,
            Self::Corners | Self::AnimationSpeed => 140,
            Self::PointerSpeed | Self::ScrollLines | Self::PrimaryButton => 120,
            Self::NightStrength => 120,
            Self::FilterKind => 320,
            Self::Unavailable(..) => 200,
        }
    }
}
/// Live read-only values supplied by the application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fact {
    Resolution,
    Scale,
    Orientation,
    VolumeUsed,
    VolumeFree,
    Processors,
    SystemType,
    Edition,
    Version,
    NativeAbi,
    /// Local date and time now (wall clock in the chosen zone and format).
    CurrentTime,
    /// Regional format example: 0 first day of week, 1 short date, 2 long
    /// date, 3 short time, 4 long time.
    Region(u8),
    DisplayLanguage,
    Keyboard,
    Account,
    /// The signed-in account's type: Administrator or Standard user.
    AccountType,
    Static(&'static str),
    /// Personalization > Themes: the applied theme's name, or Custom.
    CurrentTheme,
    /// Network & internet facts from the network service.
    Net(NetFact),
    /// Settings > VPN > Advanced options: the open connection.
    Vpn(VpnFact),
    /// System > Sound facts from door.audio.
    Audio(AudioFact),
    /// Logical processors ("8 logical processors").
    ProcessorCount,
    /// Chassis from the firmware power profile (Desktop, Laptop, Tablet).
    Chassis,
    /// Firmware power profile number.
    PowerProfile,
    /// Scheduler timer rate ("100 Hz").
    TimerRate,
    /// Disk 0: capacity, write access, cache flush, sector count, status.
    DiskCapacity,
    DiskAccess,
    DiskFlush,
    DiskSectors,
    DiskStatus,
    /// The user volume: size and sector extent.
    VolumeSize,
    VolumeExtent,
    /// The transaction journal: size and sector extent.
    JournalSize,
    JournalExtent,
    /// Doors service `n` (Storage, Session, Console, Window) as discovered.
    Service(u8),
    /// The signed-in account: user number, groups, session, integrity.
    Uid,
    Groups,
    Session,
    Integrity,
    /// Troubleshoot check `n` result (native interface, clock, storage).
    Check(u8),
    /// File system: the last checked path and what Storage reported.
    CheckedPath,
    CheckedKind,
    CheckedSize,
    /// Registry Editor: the current key's path.
    RegistryPath,
}
/// What a network fact shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetFact {
    /// "Connected" / "Not connected" / "Disabled".
    State,
    /// IPv4 address and prefix.
    Address,
    Gateway,
    Dns,
    /// Physical (MAC) address.
    Mac,
    /// Automatic (DHCP) or Manual.
    Assignment,
    /// Frames and bytes sent and received.
    Traffic,
    /// The adapter: "VirtIO network adapter" or none.
    Adapter,
    /// IPv6: the autoconfigured address, the link-local one (with its
    /// `%index`) and the router-advertised DNS servers.
    Ipv6Address,
    LinkLocal6,
    Dns6,
}
/// What a VPN connection fact shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VpnFact {
    Name,
    Server,
    /// The VPN type ("WireGuard").
    Kind,
    Status,
    /// The tunnel's IPv4 address and prefix.
    Address,
    /// This connection's public key (base64).
    PublicKey,
    /// Seconds since the latest handshake.
    Handshake,
    /// Bytes received and sent through the tunnel.
    Traffic,
}
/// What a sound fact shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioFact {
    /// The output device: "Speakers (VirtIO Sound)" or none found.
    Device,
    /// "2 channels, 16 bit, 48000 Hz (Studio Quality)".
    Format,
    /// Volume as a percentage, or Muted.
    Level,
    /// Streams playing now (the volume mixer).
    Streams,
    /// The input device: "Microphone (VirtIO Sound)" or none found.
    Input,
    /// Microphone > Recent activity: "1 request", "3 requests".
    Requests,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    /// Informational card: no action.
    None,
    /// Whole card navigates (chevron).
    Link(Page),
    Toggle(Toggle),
    /// Toggle plus a chevron to the detail page (Home Wi-Fi style).
    ToggleLink(Toggle, Page),
    Combo(Choice),
    /// A command whose provider is missing: always disabled.
    Button(&'static str),
    /// A working button (WinUI Button) with its label.
    Command(&'static str, Command),
    Value(Fact),
    /// A WinUI slider (System > Sound volume and balance).
    Slider(Slider),
    /// Disabled card: the provider is missing on this device.
    Unavailable,
}
impl Control {
    pub fn enabled(self) -> bool {
        match self {
            Self::Link(_) | Self::ToggleLink(..) | Self::Command(..) => true,
            Self::Toggle(t) => t.enabled(),
            Self::Combo(c) => c.enabled(),
            Self::Slider(_) => true,
            Self::None | Self::Button(_) | Self::Value(_) | Self::Unavailable => false,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub title: &'static str,
    pub description: &'static str,
    pub glyph: &'static str,
    pub control: Control,
}
macro_rules! row {
    ($title:literal, $description:literal, $glyph:literal, $control:expr) => {
        Row {
            title: $title,
            description: $description,
            glyph: $glyph,
            control: $control,
        }
    };
}
const fn link(page: Page) -> Control {
    Control::Link(page)
}
const NA: Control = Control::Unavailable;
const OFF: Control = Control::Toggle(Toggle::Unavailable(false));
const ON: Control = Control::Toggle(Toggle::Unavailable(true));
/// Custom-painted areas. Their geometry is still decided here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hero {
    /// Home: device thumbnail and name, network and update status.
    Home,
    /// System: device thumbnail, name, Rename, update status.
    Device,
    Network,
    Account,
    Update,
    /// Personalization: live preview and theme gallery.
    Personalization,
    /// Background preview (desktop with a sample window).
    Wallpaper,
    /// Recent images (four wallpapers).
    Backgrounds,
    /// Theme tiles (six themes).
    Themes,
    /// Background > Solid color: "Choose your background color" swatches.
    SolidColors,
    /// Lock screen: the lock picture with the time and date.
    LockPreview,
    /// Themes: the desktop preview with the theme's Background, Color,
    /// Sounds and Mouse cursor, and Save.
    ThemePreview,
    /// Home's compact theme strip inside "Personalize your device".
    HomeThemes,
    /// Display arrangement monitor.
    Monitor,
    /// Volume usage bar.
    Storage,
    /// About (26H2): the row of hardware tiles (Storage, Graphics card,
    /// Installed RAM, Processor).
    AboutTiles,
    /// Date & time (26H2): the large clock with time zone and region.
    Clock,
    /// About: device name card.
    AboutDevice,
    /// About: Device specifications.
    DeviceSpecs,
    /// About: Doors specifications (logo, edition, version).
    DoorsSpecs,
    /// Installed apps list.
    Apps,
    /// Bluetooth adapter state.
    Bluetooth,
    /// Accessibility > Contrast themes: preview, theme list, Apply, Edit.
    Contrast,
    /// Accessibility > Text size: preview text and the size slider.
    TextSize,
    /// Time & language > Typing: installed keyboards and input methods,
    /// Add a keyboard, and each one's "..." menu.
    Keyboards,
    /// Colors > Accent color: the 48 Windows colors.
    Accents,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Group {
    pub title: &'static str,
    pub subtitle: &'static str,
    pub rows: &'static [Row],
    /// Custom content painted between the header and the rows.
    pub hero: Option<Hero>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Expander {
    pub id: u8,
    pub header: Row,
    pub rows: &'static [Row],
    pub hero: Option<Hero>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    Hero(Hero),
    Header(&'static str),
    Card(Row),
    Group(Group),
    Expander(Expander),
    Note(&'static str),
    /// Run-time entries, one card each (`List`).
    List(List),
    /// Windows InfoBar (informational): a card with an info symbol and a
    /// message of up to two lines.
    Info(&'static str),
    /// The page's introduction above its first card (up to three lines).
    Intro(&'static str),
}
/// VPN connections are expanders: open bits from `VPN_OPEN` (24 at most).
pub const VPN_OPEN: u8 = 32;
/// Connection `index` of the list placed at `r`: its header card and, when
/// expanded, its body (Advanced options and Remove).
pub fn vpn_entry(r: Rect, open: u64, index: usize) -> (Rect, Option<Rect>) {
    let mut y = r.y;
    let mut i = 0;
    loop {
        let header = Rect::new(r.x, y, r.w, card());
        let expanded = i < 24 && open & (1u64 << (VPN_OPEN as usize + i)) != 0;
        let body = expanded.then(|| Rect::new(r.x, header.bottom() + 1, r.w, CHILD_ROW));
        if i == index {
            return (header, body);
        }
        y = body.unwrap_or(header).bottom() + CARD_GAP;
        i += 1;
    }
}
/// A connection's Connect/Disconnect button, left of its chevron.
/// Measured on Settings > VPN (150 %): the button ends 44 px from the card
/// edge (the chevron's column) and sizes to its label as WinUI's does:
/// "Connect" 90 px, "Disconnect" 104.
pub const VPN_BUTTON_RIGHT: i32 = 44;
pub fn vpn_connect_button(header: Rect, up: bool) -> Rect {
    let (h, w) = (field_height(), if up { 104 } else { 90 });
    Rect::new(
        header.right() - VPN_BUTTON_RIGHT - w as i32,
        header.center_y() - h as i32 / 2,
        w,
        h,
    )
}
/// The expanded body's buttons, aligned with the header's: Advanced
/// options, then Remove.
pub fn vpn_remove_button(body: Rect) -> Rect {
    let h = field_height();
    Rect::new(
        body.right() - VPN_BUTTON_RIGHT - 90,
        body.center_y() - h as i32 / 2,
        90,
        h,
    )
}
pub fn vpn_options_button(body: Rect) -> Rect {
    let remove = vpn_remove_button(body);
    Rect::new(remove.x - 8 - 140, remove.y, 140, remove.h)
}
/// Activation of an interactive item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Navigate(Page),
    Toggle(Toggle),
    Combo(Choice),
    Theme(usize),
    Background(u32),
    /// Accent color swatch `index` (`tokens::accent::WINDOWS_COLORS`).
    Accent(usize),
    /// Colors > Recent colors slot.
    RecentAccent(usize),
    /// Background > Solid color swatch (`SOLID_PALETTE` index).
    SolidColor(usize),
    Expand(u8),
    AppMenu(usize),
    Command(Command),
    Slider(Slider),
    /// Installed keyboard `index`: its "..." menu.
    KeyboardMenu(usize),
    /// "Add a keyboard".
    KeyboardAdd,
    /// Open list entry `index` (a registry subkey).
    ListOpen(List, usize),
    /// Edit list entry `index` (a registry value or a variable).
    ListEdit(List, usize),
    /// Remove list entry `index` (a VPN connection).
    ListRemove(List, usize),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    pub rect: Rect,
    pub action: Action,
}
/// A placed block and, for cards, each row's rectangle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub block: Block,
    pub rect: Rect,
    /// Rows (card/group/expander) with their rectangles.
    pub rows: Vec<(Row, Rect)>,
    /// Header row rectangle (groups/expanders) and custom hero rectangle.
    pub header: Rect,
    pub hero: Option<(Hero, Rect)>,
    pub open: bool,
}
pub struct PageLayout {
    pub blocks: Vec<Placed>,
    pub items: Vec<Item>,
    pub bottom: u32,
}
#[derive(Clone, Copy, Debug)]
pub struct Context {
    /// Card width (Layout::content_width).
    pub width: u32,
    /// Expander open bits (bit = expander id).
    pub open: u64,
    pub apps: usize,
    /// Installed keyboards and input methods (Typing page rows).
    pub keyboards: usize,
    /// Entries of each run-time list (`List::ALL` order).
    pub lists: [usize; List::COUNT],
    /// VPN connections that are up or coming up (bit = entry): their
    /// button says Disconnect, which is wider.
    pub vpn_up: u32,
    /// Personalization > Background: the background kind (0 Picture,
    /// 1 Solid color, 2 Slideshow), which picks the page's rows.
    pub background: u32,
    /// Personalization > Colors: how many Recent colors there are.
    pub recent_accents: usize,
}
pub const X: i32 = Layout::MAIN;
/// First block of a page (26H2 at 150 %: 166 px = 111 epx, the same on
/// card, header and hero pages; owner captures 2026-11-12).
pub const TOP: i32 = 111;
/// 26H2 setting card (docs/design/fidelity-26h2-diff.md G5, measured on
/// System > Display at 150 %: 104 px = 69.3 epx tall at a 110 px = 73.3 epx
/// pitch, so the gap is `space.xs`).
pub const CARD: u32 = 69;
pub const CARD_GAP: i32 = tokens::spacing(tokens::SpacingToken::Xs) as i32;
/// Card content insets (WinUI SettingsCard: 16 px padding, 1 px stroke):
/// the symbol at 19 px, the text column at 59 px (measured 59.3), controls
/// ending 17 px from the right edge, and the chevron button's centre 25 px
/// from it (measured 25.3).
pub const CARD_ICON: i32 = 19;
pub const CARD_TEXT: i32 = 59;
pub const CARD_RIGHT: i32 = 17;
pub const CARD_CHEVRON: i32 = 25;
/// Section rhythm (G6): a header's 14 px semibold line box sits 27 px above
/// its first card (measured 26.7), and 31 px below the cards before it.
pub const HEADER_TO_CARD: u32 = 27;
pub const CARD_TO_HEADER: i32 = 31;
/// Rows inside a group card (Home > Recommended settings: 104 px at 150 %,
/// owner capture 2026-10-27) are card height.
pub const GROUP_ROW: u32 = CARD;
pub const CHILD_ROW: u32 = 60;
/// Narrow cards move combos/buttons below the text (Windows SettingsCard
/// wrap). The default window at 1280x720 (896 wide: 564 px cards) wraps, so
/// the longest descriptions keep the full card width.
pub const WRAP_WIDTH: u32 = 600;
/// A title-only card keeps its control beside the title down to WinUI's
/// SettingsCard wrap threshold (Settings > VPN's "VPN connections").
pub const WRAP_TITLE_ONLY: u32 = 476;
/// Text of a card without a symbol starts at the card's padding.
pub const CARD_PADDING: i32 = 16;
pub fn wrapped(row: &Row, width: u32) -> bool {
    width
        < if row.description.is_empty() {
            WRAP_TITLE_ONLY
        } else {
            WRAP_WIDTH
        }
        && matches!(
            row.control,
            Control::Combo(_) | Control::Button(_) | Control::Command(..)
        )
}
/// Text size growth of a card holding a 14 px title over a 12 px
/// description: both line boxes grow, the padding stays (Windows
/// SettingsCard). Zero at 100%.
pub fn card_growth() -> u32 {
    use door_graphics_text::TextStyle;
    crate::palette::text_grow(0, TextStyle::Regular14)
        + crate::palette::text_grow(0, TextStyle::Regular12)
}
/// A standard card's height at the applied text size.
pub fn card() -> u32 {
    CARD + card_growth()
}
/// Combo boxes and buttons inside cards: 32 px grown with their label.
pub fn field_height() -> u32 {
    crate::palette::text_grow(32, door_graphics_text::TextStyle::Regular14)
}
/// Vertical centre of a wrapped row's title block (its control sits below).
pub fn wrapped_center(r: Rect) -> i32 {
    r.y + card() as i32 / 2
}
pub fn row_height(row: &Row, width: u32, base: u32) -> u32 {
    let base = base + card_growth();
    if wrapped(row, width) {
        base + 12 + field_height()
    } else {
        base
    }
}
/// Geometry of a row's control inside its card rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct RowGeometry {
    /// Toggle switch (40 x 20).
    pub toggle: Option<Rect>,
    /// Combo or button.
    pub field: Option<Rect>,
    /// Chevron (12 x 12) centre.
    pub chevron: Option<(i32, i32)>,
    /// Right edge available to the title/description text.
    pub text_right: i32,
}
pub fn row_geometry(row: &Row, r: Rect, width: u32) -> RowGeometry {
    let right = r.right();
    let cy = if wrapped(row, width) {
        wrapped_center(r)
    } else {
        r.center_y()
    };
    let mut g = RowGeometry {
        text_right: right - CARD_RIGHT - 8,
        ..Default::default()
    };
    match row.control {
        Control::Link(_) => {
            g.chevron = Some((right - CARD_CHEVRON, cy));
            g.text_right = right - 52;
        }
        Control::ToggleLink(..) => {
            g.chevron = Some((right - CARD_CHEVRON, cy));
            g.toggle = Some(Rect::new(right - 85, cy - 10, 40, 20));
            g.text_right = right - 143;
        }
        Control::Toggle(_) => {
            g.toggle = Some(Rect::new(right - CARD_RIGHT - 40, cy - 10, 40, 20));
            g.text_right = right - CARD_RIGHT - 98;
        }
        Control::Combo(choice) => {
            g.field = Some(field(row, r, width, choice.width()));
        }
        Control::Button(_) => {
            g.field = Some(field(row, r, width, 160));
        }
        Control::Command(_, command) => {
            g.field = Some(field(row, r, width, command.width()));
        }
        // WinUI slider: a 200 px track, 20 px tall with its thumb.
        Control::Slider(_) => {
            g.field = Some(Rect::new(right - CARD_RIGHT - 210, cy - 10, 200, 20));
        }
        Control::Value(_) => g.text_right = right - CARD_RIGHT,
        Control::None | Control::Unavailable => (),
    }
    if let Some(f) = g.field
        && !wrapped(row, width)
    {
        g.text_right = f.x - 16;
    }
    g
}
fn field(row: &Row, r: Rect, width: u32, w: u32) -> Rect {
    let h = field_height();
    if wrapped(row, width) {
        let w = w.min(r.w.saturating_sub((CARD_TEXT + CARD_RIGHT) as u32));
        Rect::new(r.x + CARD_TEXT, r.y + 60 + card_growth() as i32, w, h)
    } else {
        Rect::new(
            r.right() - CARD_RIGHT - w as i32,
            r.center_y() - h as i32 / 2,
            w,
            h,
        )
    }
}
/// Items a row contributes, in focus order.
fn row_items(row: &Row, r: Rect, width: u32, items: &mut Vec<Item>) {
    let g = row_geometry(row, r, width);
    match row.control {
        Control::Link(page) => items.push(Item {
            rect: r,
            action: Action::Navigate(page),
        }),
        Control::ToggleLink(toggle, page) => {
            items.push(Item {
                rect: r,
                action: Action::Navigate(page),
            });
            if toggle.enabled() {
                let t = g.toggle.unwrap();
                items.push(Item {
                    rect: Rect::new(t.x - 4, t.y - 6, t.w + 8, t.h + 12),
                    action: Action::Toggle(toggle),
                });
            }
        }
        Control::Toggle(toggle) if toggle.enabled() => {
            let t = g.toggle.unwrap();
            // The switch and its On/Off label, as WinUI's ToggleSwitch.
            items.push(Item {
                rect: Rect::new(t.x - 48, t.y - 6, t.w + 52, t.h + 12),
                action: Action::Toggle(toggle),
            })
        }
        Control::Combo(choice) if choice.enabled() => items.push(Item {
            rect: g.field.unwrap(),
            action: Action::Combo(choice),
        }),
        Control::Command(_, command) => items.push(Item {
            rect: g.field.unwrap(),
            action: Action::Command(command),
        }),
        // The track band: its x span maps to the value range.
        Control::Slider(slider) => items.push(Item {
            rect: g.field.unwrap(),
            action: Action::Slider(slider),
        }),
        _ => (),
    }
}
/// Tile geometry shared by galleries: (columns, tile w, tile h, pitch x, pitch y).
pub fn gallery(hero: Hero, width: u32) -> (usize, u32, u32, i32, i32) {
    match hero {
        // Themes > Current theme: one row of tiles, 60 px in, 8 px apart.
        Hero::Themes => (
            ((width.saturating_sub(60 + 24) + 10) / 120).clamp(1, 6) as usize,
            110,
            86,
            120,
            98,
        ),
        Hero::Personalization => (
            if personalization_split(width) {
                3
            } else {
                ((width + 10) / 120).clamp(1, 3) as usize
            },
            110,
            86,
            120,
            98,
        ),
        Hero::HomeThemes => {
            let columns = if hero == Hero::HomeThemes && width >= 760 {
                6
            } else {
                ((width.saturating_sub(48) + 12) / 122).clamp(1, 3) as usize
            };
            (columns, 110, 86, 122, 98)
        }
        // Windows: 100 px square recent images, 4 px apart (150 % capture).
        Hero::Backgrounds => (
            ((width.saturating_sub(58 + 24) + 4) / 104).clamp(1, RECENT_IMAGES as u32) as usize,
            100,
            100,
            104,
            104,
        ),
        Hero::SolidColors => (
            ((width.saturating_sub(58 + 24) + 4) / 44).clamp(1, 8) as usize,
            40,
            40,
            44,
            44,
        ),
        // Windows colors (26H2 at 150 %): 75 px swatches at an 80 px pitch
        // (50 and 53 epx), nine to a row, 58 px in.
        Hero::Accents => (
            ((width.saturating_sub(58 + 24) + 3) / 53).clamp(1, 9) as usize,
            50,
            50,
            53,
            53,
        ),
        _ => (1, 0, 0, 0, 0),
    }
}
/// Personalization > Colors: Recent colors slots, the rows' tops (label
/// "Recent colors" above the first, "Windows colors" above the second).
pub const RECENT_ACCENTS: usize = 5;
pub const ACCENT_RECENT_Y: i32 = 35;
pub const ACCENT_WINDOWS_Y: i32 = ACCENT_RECENT_Y + 50 + 57;
/// Personalization > Background > "Recent images" tiles (most recent first,
/// `preferences::RecentBackgrounds`).
pub const RECENT_IMAGES: usize = 5;
pub fn hero_count(hero: Hero) -> usize {
    match hero {
        Hero::Themes => THEMES.len(),
        Hero::Personalization | Hero::HomeThemes => GALLERY.len(),
        Hero::Backgrounds => RECENT_IMAGES,
        Hero::Accents => RECENT_ACCENTS + tokens::accent::WINDOWS_COLORS.len(),
        Hero::SolidColors => SOLID_PALETTE.len(),
        _ => 0,
    }
}
/// Preview + gallery side by side when wide enough.
pub fn personalization_split(width: u32) -> bool {
    width >= PERSONALIZATION_GALLERY_X as u32 + 3 * 120 - 10
}
/// Personalization: the theme grid's left edge and its label's height
/// above the tiles (the preview sits 2 px lower than the tiles).
pub const PERSONALIZATION_GALLERY_X: i32 = 300;
pub const PERSONALIZATION_LABEL: i32 = 32;
/// Tile `index` of a gallery hero placed at `r`.
pub fn tile(hero: Hero, r: Rect, index: usize, width: u32) -> Option<Rect> {
    if index >= hero_count(hero) {
        return None;
    }
    let (columns, w, h, px, py) = gallery(hero, width);
    let (x0, y0) = match hero {
        Hero::Personalization if personalization_split(width) => {
            (r.x + PERSONALIZATION_GALLERY_X, r.y + PERSONALIZATION_LABEL)
        }
        Hero::Personalization => (
            r.x,
            r.y + THEME_PREVIEW.1 as i32 + 24 + PERSONALIZATION_LABEL,
        ),
        Hero::Themes => (r.x + 60, r.y + 24),
        Hero::HomeThemes => (r.x + 24, r.y),
        Hero::Backgrounds => (r.x + 58, r.y + 35),
        Hero::SolidColors => (r.x + 58, r.y + 40),
        // Recent colors first (slots 0..5), then the Windows colors.
        Hero::Accents if index < RECENT_ACCENTS => {
            return Some(Rect::new(
                r.x + 58 + index as i32 * px,
                r.y + ACCENT_RECENT_Y,
                w,
                h,
            ));
        }
        Hero::Accents => {
            let index = index - RECENT_ACCENTS;
            let (column, row) = (index % columns, index / columns);
            return Some(Rect::new(
                r.x + 58 + column as i32 * px,
                r.y + ACCENT_WINDOWS_Y + row as i32 * py,
                w,
                h,
            ));
        }
        _ => return None,
    };
    let (column, row) = (index % columns, index / columns);
    Some(Rect::new(
        x0 + column as i32 * px,
        y0 + row as i32 * py,
        w,
        h,
    ))
}
pub fn hero_height(hero: Hero, ctx: &Context) -> u32 {
    let width = ctx.width;
    let rows = |hero: Hero| {
        let (columns, _, h, _, py) = gallery(hero, width);
        let n = hero_count(hero).div_ceil(columns);
        (n as i32 - 1) * py + h as i32
    };
    match hero {
        Hero::Home | Hero::Device if status_wrapped(hero, width) => {
            HEADER_H + STATUS_ROW_GAP as u32 + STATUS_TILE_H
        }
        Hero::Home | Hero::Device => HEADER_H,
        Hero::Contrast => contrast_geometry(Rect::new(0, 0, width, 0)).height,
        Hero::TextSize => text_size_geometry(Rect::new(0, 0, width, 0)).height,
        Hero::Network | Hero::Update => 72,
        Hero::Account => 104,
        Hero::Bluetooth => 68,
        Hero::Personalization => {
            let gallery = PERSONALIZATION_LABEL as u32 + rows(Hero::Personalization) as u32;
            if personalization_split(width) {
                gallery.max(PERSONALIZATION_LABEL as u32 + 2 + THEME_PREVIEW.1)
            } else {
                THEME_PREVIEW.1 + 24 + gallery
            }
        }
        Hero::Wallpaper | Hero::LockPreview => THEME_PREVIEW.1,
        Hero::ThemePreview => {
            if theme_preview_split(ctx.width) {
                THEME_PREVIEW.1
            } else {
                THEME_PREVIEW.1 + 24 + 2 * THEME_ENTRY_H as u32 + 48
            }
        }
        Hero::Backgrounds => (35 + rows(Hero::Backgrounds) + 10) as u32,
        Hero::SolidColors => (40 + rows(Hero::SolidColors) + 16) as u32,
        Hero::Accents => {
            let (columns, _, h, _, py) = gallery(Hero::Accents, width);
            let n = tokens::accent::WINDOWS_COLORS.len().div_ceil(columns);
            (ACCENT_WINDOWS_Y + (n as i32 - 1) * py + h as i32 + 23) as u32
        }
        Hero::Themes => (24 + rows(Hero::Themes) + 26) as u32,
        Hero::HomeThemes => (rows(Hero::HomeThemes) + 16) as u32,
        Hero::Monitor => 184,
        Hero::Storage => 124,
        Hero::AboutTiles => {
            let n = about_tiles(Rect::new(0, 0, width, 0)).len().max(1) as u32;
            let rows = n.div_ceil(about_tile_columns(width) as u32);
            rows * ABOUT_TILE.1 + (rows - 1) * ABOUT_TILE_GAP as u32
        }
        Hero::Clock => CLOCK_HEIGHT,
        Hero::AboutDevice => card(),
        Hero::DeviceSpecs => specs_height(5) + ABOUT_LINKS_HEIGHT,
        Hero::DoorsSpecs => specs_height(4),
        Hero::Apps => 52 + ctx.apps as u32 * (card() + CARD_GAP as u32),
        Hero::Keyboards => KEYBOARD_HEADER + ctx.keyboards as u32 * (card() + CARD_GAP as u32),
    }
}
/// About (26H2, settings-about.uia.tsv at 150 %): hardware tiles 300 x 213
/// px (200 x 142) at a 318 px (212) pitch; 81 px (54) down to the name
/// card; specification rows at a 37 px (24.7) pitch with values 261 px
/// (174) into the card; a 86 px (57) "Related links" strip.
pub const ABOUT_TILE: (u32, u32) = (200, 142);
pub const ABOUT_TILE_GAP: i32 = 12;
pub const ABOUT_TILES_GAP: i32 = 54;
pub const SPECS_ROW: i32 = 25;
pub const SPECS_VALUE: i32 = 174;
pub const ABOUT_LINKS_HEIGHT: u32 = 57;
/// Specification card: header row, rows, bottom padding.
pub fn specs_height(rows: u32) -> u32 {
    card() + 15 + rows * SPECS_ROW as u32 + 12
}
/// Tiles per row at content width `width` (wrapping when narrow).
pub fn about_tile_columns(width: u32) -> usize {
    ((width as i32 + ABOUT_TILE_GAP) / (ABOUT_TILE.0 as i32 + ABOUT_TILE_GAP)).clamp(1, 4) as usize
}
/// The four hardware tiles of an AboutTiles hero placed at `r`.
pub fn about_tiles(r: Rect) -> Vec<Rect> {
    let columns = about_tile_columns(r.w);
    (0..4)
        .map(|i| {
            let (c, row) = ((i % columns) as i32, (i / columns) as i32);
            Rect::new(
                r.x + c * (ABOUT_TILE.0 as i32 + ABOUT_TILE_GAP),
                r.y + row * (ABOUT_TILE.1 as i32 + ABOUT_TILE_GAP),
                ABOUT_TILE.0,
                ABOUT_TILE.1,
            )
        })
        .collect()
}
/// Device specifications' "Related links" strip: the label and its links
/// (text width estimated at 7.4 px per character, as the painter fits it).
/// A link without a destination has no provider and no target.
pub const ABOUT_LINKS: [(&str, Option<Page>); 3] = [
    ("Domain or workgroup", None),
    ("System protection", Some(Page::BackupRestore)),
    ("Advanced system settings", Some(Page::SystemAdvanced)),
];
pub fn about_links(r: Rect) -> [(Rect, Option<Page>); 3] {
    let y = r.bottom() - ABOUT_LINKS_HEIGHT as i32 + 16;
    let mut x = r.x + 119;
    ABOUT_LINKS.map(|(label, page)| {
        let w = label.len() as u32 * 37 / 5;
        if x + w as i32 > r.right() - CARD_RIGHT {
            // Too narrow for this link: it is not shown (and not a target).
            return (Rect::new(x, y, 0, 22), None);
        }
        let link = Rect::new(x, y, w, 22);
        x += w as i32 + 20;
        (link, page)
    })
}
/// Date & time (26H2): the clock header's height.
pub const CLOCK_HEIGHT: u32 = 64;
/// Typing: "Installed keyboards" title row with Add a keyboard.
const KEYBOARD_HEADER: u32 = 48;
/// The "Add a keyboard" button of the Keyboards hero placed at `r`.
pub fn keyboard_add_button(r: Rect) -> Rect {
    let w = r.w.min(150);
    Rect::new(r.right() - w as i32, r.y + 4, w, 32)
}
/// Installed keyboard card `index` inside the Keyboards hero.
pub fn keyboard_row(r: Rect, index: usize) -> Rect {
    Rect::new(
        r.x,
        r.y + KEYBOARD_HEADER as i32 + index as i32 * (card() as i32 + CARD_GAP),
        r.w,
        card(),
    )
}
/// Home/System header: the device card (thumbnail, name, Rename) is 82 px
/// tall; Windows 11's status tiles sit beside it or wrap under it.
const HEADER_H: u32 = 82;
/// Device card part of the header the tiles never cover.
pub const DEVICE_PART: u32 = 300;
/// Status tile size and spacing. Fixed sizes keep hit testing independent of
/// fonts; the painter ellipsizes text that does not fit.
pub const STATUS_TILE_W: u32 = 220;
pub const STATUS_TILE_H: u32 = 60;
pub const STATUS_GAP: i32 = 8;
const STATUS_ROW_GAP: i32 = 12;
/// Pages the header's pressable status tiles open (Windows 11 Home: the
/// network and Windows Update buttons; System: Windows Update).
pub fn status_pages(hero: Hero) -> &'static [Page] {
    match hero {
        Hero::Home => &[Page::Network, Page::Update],
        Hero::Device => &[Page::Update],
        _ => &[],
    }
}
/// Too narrow for the tiles beside the device card: they wrap under it.
pub fn status_wrapped(hero: Hero, width: u32) -> bool {
    let n = status_pages(hero).len() as u32;
    n > 0 && width < DEVICE_PART + n * (STATUS_TILE_W + STATUS_GAP as u32)
}
/// Status tiles of a Home/System header placed at `r`, in focus order: right
/// aligned beside the device card, or a row under it (narrower tiles share
/// the width).
pub fn status_tiles(hero: Hero, r: Rect, width: u32) -> Vec<(Page, Rect)> {
    let pages = status_pages(hero);
    let n = pages.len() as i32;
    let wrapped = status_wrapped(hero, width);
    let w = if wrapped {
        STATUS_TILE_W
            .min(width.saturating_sub(((n - 1).max(0) * STATUS_GAP) as u32) / n.max(1) as u32)
    } else {
        STATUS_TILE_W
    };
    let pitch = w as i32 + STATUS_GAP;
    pages
        .iter()
        .enumerate()
        .map(|(i, page)| {
            let i = i as i32;
            let rect = if wrapped {
                Rect::new(
                    r.x + i * pitch,
                    r.y + HEADER_H as i32 + STATUS_ROW_GAP,
                    w,
                    STATUS_TILE_H,
                )
            } else {
                Rect::new(
                    r.right() - (n - i) * pitch + STATUS_GAP,
                    r.y + (HEADER_H - STATUS_TILE_H) as i32 / 2,
                    w,
                    STATUS_TILE_H,
                )
            };
            (*page, rect)
        })
        .collect()
}
/// Accessibility > Contrast themes: the theme preview, the list with Apply
/// and Edit beside it (under it when narrow), then the shortcut line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContrastGeometry {
    pub preview: Rect,
    pub list: Rect,
    pub apply: Rect,
    pub edit: Rect,
    /// Top of "Press left Alt + left Shift + Print screen ..." text.
    pub shortcut: i32,
    pub height: u32,
}
pub fn contrast_geometry(r: Rect) -> ContrastGeometry {
    let preview = Rect::new(r.x, r.y, r.w.min(360), 200);
    let row = preview.bottom() + 16;
    let list = Rect::new(r.x, row, r.w.min(200), 32);
    let (apply, edit) = if r.w < 420 {
        (
            Rect::new(r.x, row + 40, 96, 32),
            Rect::new(r.x + 104, row + 40, 96, 32),
        )
    } else {
        (
            Rect::new(r.x + 212, row, 96, 32),
            Rect::new(r.x + 316, row, 96, 32),
        )
    };
    let shortcut = apply.bottom() + 16;
    ContrastGeometry {
        preview,
        list,
        apply,
        edit,
        shortcut,
        height: (shortcut + 20 - r.y) as u32,
    }
}
/// Accessibility > Text size: the preview card, then a card with the
/// slider, its value and Apply (the slider under the title when narrow).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextSizeGeometry {
    pub preview: Rect,
    pub card: Rect,
    pub slider: Rect,
    /// Left edge of the "150%" value beside the slider.
    pub value_x: i32,
    pub apply: Rect,
    pub height: u32,
}
pub fn text_size_geometry(r: Rect) -> TextSizeGeometry {
    let preview = Rect::new(r.x, r.y, r.w, 140);
    let narrow = r.w < 560;
    let card = Rect::new(
        r.x,
        preview.bottom() + 12,
        r.w,
        if narrow { 112 + card_growth() } else { card() },
    );
    let (apply, slider) = if narrow {
        let apply = Rect::new(card.right() - 24 - 96, card.y + 60, 96, 32);
        let w = (apply.x - 12 - 52 - (card.x + 24)).max(40) as u32;
        (apply, Rect::new(card.x + 24, card.y + 64, w, 24))
    } else {
        let apply = Rect::new(card.right() - 24 - 96, card.center_y() - 16, 96, 32);
        (
            apply,
            Rect::new(apply.x - 16 - 52 - 240, card.center_y() - 12, 240, 24),
        )
    };
    TextSizeGeometry {
        preview,
        card,
        slider,
        value_x: slider.right() + 12,
        apply,
        height: (card.bottom() - r.y) as u32,
    }
}
/// The text size a pointer at `x` picks on `slider`.
pub fn slider_value(slider: Rect, x: i32) -> u32 {
    let span = TEXT_MAX - TEXT_MIN;
    let along = (x - slider.x).clamp(0, slider.w as i32) as u32;
    TEXT_MIN + (along * span + slider.w / 2) / slider.w.max(1)
}
/// Centre x of the slider thumb at `value`.
pub fn slider_thumb(slider: Rect, value: u32) -> i32 {
    let value = value.clamp(TEXT_MIN, TEXT_MAX) - TEXT_MIN;
    slider.x + ((value * slider.w + (TEXT_MAX - TEXT_MIN) / 2) / (TEXT_MAX - TEXT_MIN)) as i32
}
/// Themes' preview entries side by side with the preview when wide enough.
pub fn theme_preview_split(width: u32) -> bool {
    width >= THEME_ENTRY_X as u32 + 2 * THEME_ENTRY_W
}
/// Themes' desktop preview (bezel included), as Windows draws it.
pub const THEME_PREVIEW: (u32, u32) = (250, 156);
/// Entry buttons: first column's left edge, column pitch and row pitch.
pub const THEME_ENTRY_X: i32 = 274;
pub const THEME_ENTRY_W: u32 = 208;
pub const THEME_ENTRY_H: i32 = 52;
/// Themes' preview entries (Background, Color, Sounds, Mouse cursor) and
/// the Save button, placed for a hero at `r`.
pub fn theme_preview_geometry(r: Rect, width: u32) -> ([Rect; 4], Rect) {
    let (x, y) = if theme_preview_split(width) {
        (r.x + THEME_ENTRY_X, r.y + 4)
    } else {
        (r.x, r.y + THEME_PREVIEW.1 as i32 + 24)
    };
    // Narrow panes halve their width between the two columns.
    let column = THEME_ENTRY_W.min(width / 2);
    let entry = |i: i32| {
        Rect::new(
            x + (i % 2) * column as i32,
            y + (i / 2) * THEME_ENTRY_H,
            column - 8,
            THEME_ENTRY_H as u32 - 4,
        )
    };
    let save = Rect::new(x + 8, y + 2 * THEME_ENTRY_H + 1, 130, 32);
    ([entry(0), entry(1), entry(2), entry(3)], save)
}
fn hero_items(hero: Hero, r: Rect, ctx: &Context, items: &mut Vec<Item>) {
    match hero {
        Hero::ThemePreview => {
            let (entries, _) = theme_preview_geometry(r, ctx.width);
            for (rect, page) in [
                (entries[0], Page::Background),
                (entries[1], Page::Colors),
                (entries[3], Page::MousePointerAndTouch),
            ] {
                items.push(Item {
                    rect,
                    action: Action::Navigate(page),
                });
            }
        }
        Hero::Contrast => {
            let g = contrast_geometry(r);
            items.push(Item {
                rect: g.list,
                action: Action::Combo(Choice::Contrast),
            });
            items.push(Item {
                rect: g.apply,
                action: Action::Command(Command::ApplyContrast),
            });
        }
        Hero::TextSize => {
            let g = text_size_geometry(r);
            items.push(Item {
                rect: g.slider,
                action: Action::Slider(Slider::TextSize),
            });
            items.push(Item {
                rect: g.apply,
                action: Action::Command(Command::ApplyTextSize),
            });
        }
        Hero::DeviceSpecs => {
            for (link, page) in about_links(r) {
                if let Some(page) = page {
                    items.push(Item {
                        rect: link,
                        action: Action::Navigate(page),
                    });
                }
            }
        }
        _ => (),
    }
    for (page, rect) in status_tiles(hero, r, ctx.width) {
        items.push(Item {
            rect,
            action: Action::Navigate(page),
        });
    }
    for index in 0..hero_count(hero) {
        if hero == Hero::Accents && index < RECENT_ACCENTS && index >= ctx.recent_accents {
            continue;
        }
        let t = tile(hero, r, index, ctx.width).unwrap();
        items.push(Item {
            rect: t,
            action: match hero {
                Hero::Backgrounds => Action::Background(index as u32),
                Hero::Accents if index < RECENT_ACCENTS => Action::RecentAccent(index),
                Hero::Accents => Action::Accent(index - RECENT_ACCENTS),
                Hero::SolidColors => Action::SolidColor(index),
                Hero::Themes => Action::Theme(index),
                _ => Action::Theme(GALLERY.get(index).copied().unwrap_or(index)),
            },
        });
    }
    if hero == Hero::AboutDevice && ctx.width >= 360 {
        items.push(Item {
            rect: about_rename_button(r),
            action: Action::Command(Command::RenamePc),
        });
    }
    if hero == Hero::Keyboards {
        items.push(Item {
            rect: keyboard_add_button(r),
            action: Action::KeyboardAdd,
        });
        for index in 0..ctx.keyboards {
            items.push(Item {
                rect: app_menu_button(keyboard_row(r, index)),
                action: Action::KeyboardMenu(index),
            });
        }
    }
    if hero == Hero::Apps {
        for index in 0..ctx.apps {
            let row = app_row(r, index);
            items.push(Item {
                rect: app_menu_button(row),
                action: Action::AppMenu(index),
            });
        }
    }
}
/// Expander header area available to its own control (chevron excluded).
pub fn expander_controls(header: Rect) -> Rect {
    Rect {
        w: header.w.saturating_sub(40),
        ..header
    }
}
/// Installed app card `index` inside the Apps hero.
pub fn app_row(r: Rect, index: usize) -> Rect {
    Rect::new(
        r.x,
        r.y + 52 + index as i32 * (card() as i32 + CARD_GAP),
        r.w,
        card(),
    )
}
/// The "..." (More) button of an installed app card.
pub fn app_menu_button(row: Rect) -> Rect {
    Rect::new(row.right() - 56, row.center_y() - 16, 32, 32)
}
fn group_header_height(g: &Group) -> u32 {
    use door_graphics_text::TextStyle;
    let title = crate::palette::text_grow(0, TextStyle::Semibold20);
    match (g.title.is_empty(), g.subtitle.is_empty()) {
        (true, _) => 0,
        (false, true) => 72 + title,
        (false, false) => 104 + title + crate::palette::text_grow(0, TextStyle::Regular14),
    }
}
pub fn layout(page: Page, ctx: &Context) -> PageLayout {
    let width = ctx.width;
    let mut y = TOP + Layout::heading_growth();
    let mut blocks = Vec::new();
    let mut items = Vec::new();
    let mut previous: Option<Block> = None;
    for block in page_blocks(page, ctx) {
        // Windows spacing: 4 px between sibling cards, 12 between card
        // groups, a section header sits 20 px below the cards it follows.
        y += match (previous, block) {
            (None, _) => 0,
            (Some(Block::Card(_)), Block::Card(_)) => CARD_GAP,
            (Some(Block::Expander(_)), Block::Card(_) | Block::Expander(_))
            | (Some(Block::Card(_)), Block::Expander(_)) => CARD_GAP,
            (Some(Block::Card(_) | Block::Expander(_) | Block::List(_)), Block::List(_))
            | (Some(Block::List(_)), Block::Card(_) | Block::Expander(_)) => CARD_GAP,
            (_, Block::Header(_)) => CARD_TO_HEADER,
            (Some(Block::Header(_)), _) => 0,
            // About (26H2): 54 px from the tiles to the name card, then
            // the name and specification cards 4 px apart.
            (Some(Block::Hero(Hero::AboutTiles)), Block::Hero(_)) => ABOUT_TILES_GAP,
            (Some(Block::Hero(Hero::AboutDevice | Hero::DeviceSpecs)), Block::Hero(_)) => CARD_GAP,
            // Personalization previews (26H2 at 150 %): 22 px below the
            // Background/Colors preview, 38 px below the Themes one.
            (Some(Block::Hero(Hero::Wallpaper | Hero::LockPreview)), _) => 15,
            (Some(Block::Hero(Hero::ThemePreview)), _) => 25,
            (Some(Block::Hero(_)), _) => 24,
            _ => 12,
        };
        let mut placed = Placed {
            block: *block,
            rect: Rect::new(X, y, width, 0),
            rows: Vec::new(),
            header: Rect::default(),
            hero: None,
            open: false,
        };
        match *block {
            Block::Hero(hero) => {
                placed.rect.h = hero_height(hero, ctx);
                placed.hero = Some((hero, placed.rect));
                hero_items(hero, placed.rect, ctx, &mut items);
            }
            Block::Header(_) => {
                placed.rect.h = crate::palette::text_grow(
                    HEADER_TO_CARD,
                    door_graphics_text::TextStyle::Semibold14,
                )
            }
            Block::Note(_) => {
                placed.rect.h =
                    crate::palette::text_grow(28, door_graphics_text::TextStyle::Regular14)
            }
            Block::Info(text) => placed.rect.h = info_height(text, width),
            Block::Intro(text) => placed.rect.h = intro_height(text, width),
            Block::List(List::VpnConnections) => {
                // Each connection is an expander: Connect (or Disconnect)
                // beside the chevron; open, Advanced options and Remove.
                let list = List::VpnConnections;
                let count = ctx.lists[list as usize];
                // Windows shows nothing for no connections.
                placed.rect.h = 0;
                for index in 0..count {
                    let (header, body) = vpn_entry(placed.rect, ctx.open, index);
                    items.push(Item {
                        rect: header,
                        action: Action::Expand(VPN_OPEN + index as u8),
                    });
                    let up = index < 32 && ctx.vpn_up & (1 << index) != 0;
                    items.push(Item {
                        rect: vpn_connect_button(header, up),
                        action: Action::ListEdit(list, index),
                    });
                    if let Some(b) = body {
                        items.push(Item {
                            rect: vpn_options_button(b),
                            action: Action::ListOpen(list, index),
                        });
                        items.push(Item {
                            rect: vpn_remove_button(b),
                            action: Action::ListRemove(list, index),
                        });
                    }
                    placed.rect.h = (body.unwrap_or(header).bottom() - placed.rect.y) as u32;
                }
            }
            Block::List(list) => {
                let count = ctx.lists[list as usize];
                placed.rect.h = list_height(count);
                for index in 0..count {
                    let entry = list_entry(placed.rect, index);
                    match list.entry_action() {
                        Some(true) => items.push(Item {
                            rect: entry,
                            action: Action::ListOpen(list, index),
                        }),
                        Some(false) => items.push(Item {
                            rect: list_edit_button(entry),
                            action: Action::ListEdit(list, index),
                        }),
                        None => (),
                    }
                }
            }
            Block::Card(row) => {
                let r = Rect::new(X, y, width, row_height(&row, width, CARD));
                placed.rect = r;
                placed.rows.push((row, r));
                row_items(&row, r, width, &mut items);
            }
            Block::Group(g) => {
                let mut cursor = y + group_header_height(&g) as i32;
                placed.header = Rect::new(X, y, width, group_header_height(&g));
                if let Some(hero) = g.hero {
                    let h = hero_height(hero, ctx);
                    let r = Rect::new(X, cursor, width, h);
                    placed.hero = Some((hero, r));
                    hero_items(hero, r, ctx, &mut items);
                    cursor += h as i32;
                }
                for row in g.rows {
                    let r = Rect::new(X, cursor, width, row_height(row, width, GROUP_ROW));
                    placed.rows.push((*row, r));
                    row_items(row, r, width, &mut items);
                    cursor += r.h as i32;
                }
                placed.rect.h = (cursor - y) as u32;
            }
            Block::Expander(e) => {
                let open = ctx.open & (1 << e.id) != 0;
                placed.open = open;
                let header = Rect::new(X, y, width, row_height(&e.header, width, CARD));
                placed.header = header;
                items.push(Item {
                    rect: header,
                    action: Action::Expand(e.id),
                });
                // Header controls (e.g. a combo) stay actionable when closed and
                // sit left of the disclosure chevron.
                if !matches!(e.header.control, Control::Link(_)) {
                    row_items(&e.header, expander_controls(header), width, &mut items);
                }
                let mut cursor = header.bottom();
                if open {
                    cursor += 1;
                    if let Some(hero) = e.hero {
                        let h = hero_height(hero, ctx);
                        let r = Rect::new(X, cursor, width, h);
                        placed.hero = Some((hero, r));
                        hero_items(hero, r, ctx, &mut items);
                        cursor += h as i32;
                    }
                    for row in e.rows {
                        let r = Rect::new(X, cursor, width, row_height(row, width, CHILD_ROW));
                        placed.rows.push((*row, r));
                        row_items(row, r, width, &mut items);
                        cursor += r.h as i32;
                    }
                }
                placed.rect.h = (cursor - y) as u32;
            }
        }
        y = placed.rect.bottom();
        previous = Some(*block);
        blocks.push(placed);
    }
    PageLayout {
        blocks,
        items,
        bottom: (y + 32).max(TOP + Layout::heading_growth()) as u32,
    }
}
/// Expanders open when a page is first shown (Windows: Taskbar items).
pub fn default_open(page: Page) -> u64 {
    match page {
        Page::Taskbar => 1 << 1,
        // The adapter card opens with its properties, as Windows shows it.
        Page::Ethernet => 1 << 1,
        // The theme grid shows, as Windows opens Themes.
        Page::Themes => 1 << 2,
        _ => 0,
    }
}
macro_rules! rows {
    ($($row:expr),* $(,)?) => { &[$($row),*] };
}
mod generated;
/// The blocks laid out for `page` now: Background's rows follow its kind.
fn page_blocks(page: Page, ctx: &Context) -> &'static [Block] {
    match (page, ctx.background) {
        (Page::Background, 1) => &BACKGROUND_SOLID,
        (Page::Background, 2) => &BACKGROUND_SLIDESHOW,
        _ => blocks_for(page),
    }
}
pub fn blocks_for(page: Page) -> &'static [Block] {
    match page {
        Page::Home => &HOME,
        Page::System => &SYSTEM,
        Page::Bluetooth => &BLUETOOTH,
        Page::Network => &NETWORK,
        Page::Personalization => &PERSONALIZATION,
        Page::Apps => &APPS,
        Page::Accounts => &ACCOUNTS,
        Page::Time => &TIME,
        Page::Gaming => &GAMING,
        Page::Accessibility => &ACCESSIBILITY,
        Page::Privacy => &PRIVACY,
        Page::Update => &UPDATE,
        Page::About => &ABOUT,
        Page::Display => &DISPLAY,
        Page::InstalledApps => &INSTALLED,
        Page::Taskbar => &TASKBAR,
        Page::ContextMenu => &CONTEXT,
        Page::Sound => &SOUND,
        Page::Notifications => &NOTIFICATIONS,
        Page::Power => &POWER,
        Page::Storage => &STORAGE,
        Page::Background => &BACKGROUND,
        Page::Colors => &COLORS,
        Page::Themes => &THEMES_PAGE,
        Page::Start => &START,
        Page::SystemAdvanced => &SYSTEM_ADVANCED,
        Page::Search => &SEARCH_PAGE,
        Page::ContrastThemes => &CONTRAST_PAGE,
        Page::TextSize => &TEXT_SIZE_PAGE,
        Page::DateTime => &DATE_TIME,
        Page::LanguageRegion => &LANGUAGE,
        Page::Typing => &TYPING,
        Page::VisualEffects => &VISUAL,
        Page::Registry => tools::REGISTRY,
        Page::DevicesDrivers => tools::DEVICES,
        Page::DiskManagement => tools::DISKS,
        Page::NetworkAdministration => tools::NETWORK_ADVANCED,
        Page::Services => tools::SERVICES,
        Page::Environment => tools::ENVIRONMENT,
        Page::SystemConfiguration => tools::SYSTEM_CONFIGURATION,
        Page::GroupPolicy => tools::POLICIES,
        Page::EventViewer => tools::EVENT_LOGS,
        Page::TaskScheduler => tools::SCHEDULED_TASKS,
        Page::UsersGroups => tools::YOUR_INFO,
        Page::PermissionsSharing => tools::FILE_SYSTEM,
        Page::Performance => tools::TROUBLESHOOT,
        Page::PowerManagement => tools::POWER_OPTIONS,
        Page::BackupRestore => tools::RECOVERY,
        Page::DeveloperTools => tools::DEVELOPER_TOOLS,
        Page::Fonts => tools::FONTS,
        Page::Clipboard => tools::CLIPBOARD,
        Page::SignInOptions => tools::SIGN_IN_OPTIONS,
        Page::LockScreen => tools::LOCK_SCREEN,
        Page::DefaultApps => tools::DEFAULT_APPS,
        Page::Multitasking => tools::MULTITASKING,
        Page::Mouse => tools::MOUSE,
        Page::ColorFilters => tools::COLOR_FILTERS,
        Page::Microphone => tools::MICROPHONE,
        Page::Vpn => &VPN,
        Page::Proxy => &PROXY,
        Page::Firewall => &FIREWALL,
        Page::VpnConnection => &VPN_CONNECTION,
        Page::OtherUsers => &OTHER_USERS,
        Page::OtherUser => &OTHER_USER,
        Page::Audio => &generated::AUDIO,
        Page::Captions => &generated::CAPTIONS,
        Page::EyeControl => &generated::EYE_CONTROL,
        Page::HearingDevices => &generated::HEARING_DEVICES,
        Page::AccessibilityKeyboard => &generated::ACCESSIBILITY_KEYBOARD,
        Page::Magnifier => &generated::MAGNIFIER,
        Page::AccessibilityMouse => &generated::ACCESSIBILITY_MOUSE,
        Page::MousePointerAndTouch => &generated::MOUSE_POINTER_AND_TOUCH,
        Page::Narrator => &generated::NARRATOR,
        Page::ConfigureBrailleDisplays => &generated::CONFIGURE_BRAILLE_DISPLAYS,
        Page::MathReading => &generated::MATH_READING,
        Page::ScreenTint => &generated::SCREEN_TINT,
        Page::AccessibilitySpeech => &generated::ACCESSIBILITY_SPEECH,
        Page::TextCursor => &generated::TEXT_CURSOR,
        Page::AccessWorkOrSchool => &generated::ACCESS_WORK_OR_SCHOOL,
        Page::Kiosk => &generated::KIOSK,
        Page::AccountsPasskeys => &generated::ACCOUNTS_PASSKEYS,
        Page::PasskeysAdvancedOptions => &generated::PASSKEYS_ADVANCED_OPTIONS,
        Page::YourAccounts => &generated::YOUR_ACCOUNTS,
        Page::AdvancedAppSettings => &generated::ADVANCED_APP_SETTINGS,
        Page::AppsForWebsites => &generated::APPS_FOR_WEBSITES,
        Page::OfflineMaps => &generated::OFFLINE_MAPS,
        Page::Resume => &generated::RESUME,
        Page::Startup => &generated::STARTUP,
        Page::VideoPlayback => &generated::VIDEO_PLAYBACK,
        Page::AutoPlay => &generated::AUTO_PLAY,
        Page::Cameras => &generated::CAMERAS,
        Page::DevicesKeyboard => &generated::DEVICES_KEYBOARD,
        Page::PenAndWindowsInk => &generated::PEN_AND_WINDOWS_INK,
        Page::PrintersAndScanners => &generated::PRINTERS_AND_SCANNERS,
        Page::Touchpad => &generated::TOUCHPAD,
        Page::USB => &generated::USB,
        Page::GameMode => &generated::GAME_MODE,
        Page::AdvancedSharingSettings => &generated::ADVANCED_SHARING_SETTINGS,
        Page::AirplaneMode => &generated::AIRPLANE_MODE,
        Page::DialUp => &generated::DIAL_UP,
        Page::Ethernet => &generated::ETHERNET,
        Page::MobileHotspot => &generated::MOBILE_HOTSPOT,
        Page::WiFi => &generated::WI_FI,
        Page::ManageKnownNetworks => &generated::MANAGE_KNOWN_NETWORKS,
        Page::DynamicLighting => &generated::DYNAMIC_LIGHTING,
        Page::Folders => &generated::FOLDERS,
        Page::TextInput => &generated::TEXT_INPUT,
        Page::AccountInfo => &generated::ACCOUNT_INFO,
        Page::AppDiagnostics => &generated::APP_DIAGNOSTICS,
        Page::AutomaticFileDownloads => &generated::AUTOMATIC_FILE_DOWNLOADS,
        Page::Calendar => &generated::CALENDAR,
        Page::CallHistory => &generated::CALL_HISTORY,
        Page::Calls => &generated::CALLS,
        Page::Camera => &generated::CAMERA,
        Page::Contacts => &generated::CONTACTS,
        Page::CustomDictionary => &generated::CUSTOM_DICTIONARY,
        Page::Documents => &generated::DOCUMENTS,
        Page::Downloads => &generated::DOWNLOADS,
        Page::Email => &generated::EMAIL,
        Page::ForegroundTextAccess => &generated::FOREGROUND_TEXT_ACCESS,
        Page::Location => &generated::LOCATION,
        Page::Messaging => &generated::MESSAGING,
        Page::Music => &generated::MUSIC,
        Page::PrivacyNotifications => &generated::PRIVACY_NOTIFICATIONS,
        Page::OtherDevices => &generated::OTHER_DEVICES,
        Page::PrivacyPasskeys => &generated::PRIVACY_PASSKEYS,
        Page::Pictures => &generated::PICTURES,
        Page::Radios => &generated::RADIOS,
        Page::ScreenshotBorders => &generated::SCREENSHOT_BORDERS,
        Page::ScreenshotsAndScreenRecording => &generated::SCREENSHOTS_AND_SCREEN_RECORDING,
        Page::PrivacySpeech => &generated::PRIVACY_SPEECH,
        Page::Tasks => &generated::TASKS,
        Page::Videos => &generated::VIDEOS,
        Page::VoiceActivation => &generated::VOICE_ACTIVATION,
        Page::WindowsSecurity => &generated::WINDOWS_SECURITY,
        Page::Activation => &generated::ACTIVATION,
        Page::AdvancedDisplay => &generated::ADVANCED_DISPLAY,
        Page::Graphics => &generated::GRAPHICS,
        Page::HDR => &generated::HDR,
        Page::NightLight => &generated::NIGHT_LIGHT,
        Page::Focus => &generated::FOCUS,
        Page::OptionalFeatures => &generated::OPTIONAL_FEATURES,
        Page::EnergyRecommendations => &generated::ENERGY_RECOMMENDATIONS,
        Page::ProjectingToThisPC => &generated::PROJECTING_TO_THIS_PC,
        Page::RemoteDesktop => &generated::REMOTE_DESKTOP,
        Page::Share => &generated::SHARE,
        Page::AllSoundDevices => &generated::ALL_SOUND_DEVICES,
        Page::Properties => &generated::PROPERTIES,
        Page::VolumeMixer => &generated::VOLUME_MIXER,
        Page::StorageSense => &generated::STORAGE_SENSE,
        Page::WhereNewContentIsSaved => &generated::WHERE_NEW_CONTENT_IS_SAVED,
        Page::SystemComponents => &generated::SYSTEM_COMPONENTS,
        Page::DomainLexicon => &generated::DOMAIN_LEXICON,
        Page::Keys => &generated::KEYS,
        Page::MicrosoftIME => &generated::MICROSOFT_IME,
        Page::MicrosoftPinyin => &generated::MICROSOFT_PINYIN,
        Page::UserDefinedPhrases => &generated::USER_DEFINED_PHRASES,
        Page::TimeSpeech => &generated::TIME_SPEECH,
        Page::WindowsUpdateAdvancedOptions => &generated::WINDOWS_UPDATE_ADVANCED_OPTIONS,
        Page::OptionalUpdates => &generated::OPTIONAL_UPDATES,
        Page::UninstallUpdates => &generated::UNINSTALL_UPDATES,
        Page::UpdateHistory => &generated::UPDATE_HISTORY,
    }
}
/// Privacy & security > Firewall & network protection, as Windows
/// Security's page has it: the three network profiles (the active one
/// first) with Doors Defender Firewall's switch and "Incoming
/// connections", then Restore defaults. Every change asks an administrator
/// (User Account Control) before Init stores it.
static FIREWALL: [Block; 9] = [
    Block::Header("Public network (active)"),
    Block::Card(row!(
        "Doors Defender Firewall",
        "Helps protect your device while on a public network",
        "shield",
        Control::Toggle(Toggle::Firewall(2))
    )),
    Block::Card(row!(
        "Incoming connections",
        "Blocks all incoming connections, including those in the list of allowed apps",
        "",
        Control::Toggle(Toggle::FirewallShields(2))
    )),
    Block::Header("Private network"),
    Block::Card(row!(
        "Doors Defender Firewall",
        "Helps protect your device while on a private network",
        "shield",
        Control::Toggle(Toggle::Firewall(1))
    )),
    Block::Header("Domain network"),
    Block::Card(row!(
        "Doors Defender Firewall",
        "Helps protect your device while on a domain network",
        "shield",
        Control::Toggle(Toggle::Firewall(0))
    )),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Restore firewalls to default",
        "Every network's settings back to the defaults; rules you or apps added are removed",
        "refresh",
        Control::Command("Restore defaults", Command::RestoreFirewall)
    )),
];
/// Network & internet > Proxy, as 26H2 lays it out.
static PROXY: [Block; 5] = [
    Block::Header("Automatic proxy setup"),
    Block::Card(row!(
        "Automatically detect settings",
        "",
        "",
        Control::Toggle(Toggle::ProxyAutoDetect)
    )),
    Block::Card(row!(
        "Use setup script",
        "",
        "",
        Control::Command("Set up", Command::EditSetupScript)
    )),
    Block::Header("Manual proxy setup"),
    Block::Card(row!(
        "Use a proxy server",
        "",
        "",
        Control::Command("Set up", Command::EditProxy)
    )),
];
/// Network & internet > VPN, as Windows 11 26H2 lays it out: Add VPN, the
/// user's connections (expanders: Connect or Disconnect; open, Advanced
/// options and Remove) and the two settings for all connections. Windows'
/// web help links (Related support, Get help, Give feedback) are left out
/// on purpose. WireGuard is the provider; the built-in IKEv2, SSTP, L2TP
/// and PPTP types are not offered yet.
static VPN: [Block; 5] = [
    Block::Card(row!(
        "VPN connections",
        "",
        "",
        Control::Command("Add VPN", Command::AddVpn)
    )),
    Block::List(List::VpnConnections),
    Block::Header("Advanced settings for all VPN connections"),
    Block::Card(row!(
        "Allow VPN over metered networks",
        "",
        "",
        Control::Toggle(Toggle::PrefOff {
            id: p::VPN,
            bit: p::vpn::NO_METERED
        })
    )),
    Block::Card(row!(
        "Allow VPN while roaming",
        "",
        "",
        Control::Toggle(Toggle::PrefOff {
            id: p::VPN,
            bit: p::vpn::NO_ROAMING
        })
    )),
];
/// A connection's Advanced options (its heading is the connection's
/// name): what Windows lists under Connection properties, for WireGuard.
/// Accounts > Other users (ms-settings:otherusers).
static OTHER_USERS: [Block; 3] = [
    Block::Header("Other users"),
    Block::Card(row!(
        "Add other user",
        "",
        "people",
        Control::Command("Add account", Command::AddAccount)
    )),
    Block::List(List::OtherUsers),
];
/// An account of Other users (headed by its name).
static OTHER_USER: [Block; 2] = [
    Block::Card(row!(
        "Account options",
        "",
        "people",
        Control::Command("Change account type", Command::ChangeAccountType)
    )),
    Block::Card(row!(
        "Account and data",
        "",
        "delete",
        Control::Command("Remove", Command::RemoveAccount)
    )),
];
static VPN_CONNECTION: [Block; 1] = [Block::Group(Group {
    title: "Connection properties",
    subtitle: "",
    rows: rows![
        row!(
            "Connection name",
            "",
            "",
            Control::Value(Fact::Vpn(VpnFact::Name))
        ),
        row!(
            "Server name or address",
            "",
            "",
            Control::Value(Fact::Vpn(VpnFact::Server))
        ),
        row!("VPN type", "", "", Control::Value(Fact::Vpn(VpnFact::Kind))),
        row!("Status", "", "", Control::Value(Fact::Vpn(VpnFact::Status))),
        row!(
            "IPv4 address",
            "",
            "",
            Control::Value(Fact::Vpn(VpnFact::Address))
        ),
        row!(
            "Public key",
            "",
            "",
            Control::Value(Fact::Vpn(VpnFact::PublicKey))
        ),
        row!(
            "Latest handshake",
            "",
            "",
            Control::Value(Fact::Vpn(VpnFact::Handshake))
        ),
        row!(
            "Data received / sent",
            "",
            "",
            Control::Value(Fact::Vpn(VpnFact::Traffic))
        )
    ],
    hero: None,
})];
/// Pages that replace legacy administration windows (P14 unification).
#[path = "settings_catalog/tools.rs"]
mod tools;
/// Window points guest scenarios press, derived from the same placement the
/// app paints and hit-tests (tools/xtask/src/desktop/administration.rs,
/// settings_reference.rs, p14_settings.rs). `width` x `height` is the
/// Settings client, the page is scrolled to the top and text is at 100%.
pub mod points {
    use super::*;
    fn center(r: Rect) -> (i32, i32) {
        (r.x + r.w as i32 / 2, r.center_y())
    }
    /// The layout context Settings uses for a client of this size.
    pub fn context(width: u32, height: u32, lists: [usize; List::COUNT]) -> Context {
        let layout = Layout { width, height };
        Context {
            width: layout.content_width(),
            open: 0,
            apps: crate::settings_layout::INCLUDED_APPS.len(),
            keyboards: 1,
            lists,
            vpn_up: 0,
            background: 0,
            recent_accents: 1,
        }
    }
    /// Centre of the first item of `page` that `wanted` accepts.
    pub fn item(
        page: Page,
        width: u32,
        height: u32,
        lists: [usize; List::COUNT],
        wanted: impl Fn(&Item) -> bool,
    ) -> Option<(i32, i32)> {
        layout(page, &context(width, height, lists))
            .items
            .iter()
            .find(|i| wanted(i))
            .map(|i| center(i.rect))
    }
    /// A card button of `page` (Registry Editor > Go to, New value, ...).
    pub fn command(page: Page, command: Command, width: u32, height: u32) -> Option<(i32, i32)> {
        item(page, width, height, [0; List::COUNT], |i| {
            i.action == Action::Command(command)
        })
    }
    /// The Edit button of list entry `index` (`lists` gives every list's
    /// length, which places the lists after it).
    pub fn list_edit(
        page: Page,
        list: List,
        index: usize,
        width: u32,
        height: u32,
        lists: [usize; List::COUNT],
    ) -> Option<(i32, i32)> {
        item(page, width, height, lists, |i| {
            i.action == Action::ListEdit(list, index)
        })
    }
    pub fn dialog_field(kind: DialogKind, index: usize, width: u32, height: u32) -> (i32, i32) {
        center(dialog_geometry(kind, width, height).fields[index])
    }
    pub fn dialog_type(kind: DialogKind, width: u32, height: u32) -> (i32, i32) {
        center(dialog_geometry(kind, width, height).kind)
    }
    pub fn dialog_button(kind: DialogKind, index: usize, width: u32, height: u32) -> (i32, i32) {
        center(dialog_geometry(kind, width, height).buttons[index])
    }
    /// A pixel of the page background left of the content (the rail side
    /// of the main column, below the heading): the Mica colour.
    pub fn background(height: u32) -> (i32, i32) {
        (Layout::MAIN - 4, height as i32 - 8)
    }
    /// A background pixel of the page heading row, right of the heading text
    /// and just below the title bar: the Mica colour.
    pub fn heading_background(width: u32) -> (i32, i32) {
        let layout = Layout { width, height: 0 };
        (
            Layout::MAIN + layout.content_width() as i32 - 8,
            Layout::TITLE + 4,
        )
    }
    /// Centre of rail row `row` (the rail scrolled to the top, 100 % text).
    /// The rail starts below the account block at every window size.
    pub fn rail_row(row: usize) -> (i32, i32) {
        (
            Layout::NAV_X + Layout::NAV_W as i32 / 2,
            Layout::RAIL_TOP + row as i32 * Layout::NAV_PITCH + Layout::NAV_ROW / 2,
        )
    }
    /// A pixel inside the accent selection pill of rail row `row`: x inside
    /// the 3 px pill, y its centre.
    pub fn rail_pill(row: usize) -> (i32, i32) {
        (Layout::NAV_X + 1, rail_row(row).1)
    }
    /// Centre of the title-bar search pill, if the window has one.
    pub fn search(width: u32, height: u32) -> Option<(i32, i32)> {
        Layout { width, height }
            .search()
            .map(|s| (s.x + s.w as i32 / 2, s.y + s.h as i32 / 2))
    }
    /// The card of `page` that opens `target` (page scrolled to the top).
    pub fn link(page: Page, target: Page, width: u32, height: u32) -> Option<(i32, i32)> {
        item(page, width, height, [0; List::COUNT], |i| {
            i.action == Action::Navigate(target)
        })
    }
    /// Theme tile `index` (`settings_layout::THEMES`) on Personalization.
    pub fn theme_tile(index: usize, width: u32, height: u32) -> Option<(i32, i32)> {
        item(
            Page::Personalization,
            width,
            height,
            [0; List::COUNT],
            |i| i.action == Action::Theme(index),
        )
    }
    /// How far End scrolls `page` (its maximum scroll offset). Resizing
    /// keeps the offset, clamped to the new maximum.
    pub fn end_scroll(page: Page, width: u32, height: u32) -> u32 {
        layout(page, &context(width, height, [0; List::COUNT]))
            .bottom
            .saturating_sub(height)
    }
    /// The read-only value of `fact` with `page` scrolled by `scroll`: the
    /// right edge of its text and its centre line.
    pub fn value(
        page: Page,
        fact: Fact,
        width: u32,
        height: u32,
        scroll: u32,
    ) -> Option<(i32, i32)> {
        let ctx = context(width, height, [0; List::COUNT]);
        let scroll = scroll as i32;
        layout(page, &ctx)
            .blocks
            .iter()
            .flat_map(|b| b.rows.iter())
            .find(|(row, _)| row.control == Control::Value(fact))
            .map(|(row, r)| {
                let g = row_geometry(row, *r, ctx.width);
                let cy = if wrapped(row, ctx.width) {
                    wrapped_center(*r)
                } else {
                    r.center_y()
                };
                (g.text_right, cy - scroll)
            })
    }
    /// Installed apps: app `index`'s "..." button and its menu's Open entry.
    pub fn installed_app_open(index: usize, width: u32, height: u32) -> ((i32, i32), (i32, i32)) {
        let layout = Layout { width, height };
        let hero = Rect::new(X, TOP, layout.content_width(), 0);
        let button = app_menu_button(app_row(hero, index));
        let menu = Popup::menu(button, 200, 3, 0b001, Layout::TITLE, height as i32);
        let open = menu.option(0);
        (center(button), (open.x + 80, open.center_y()))
    }
    /// A point in the bottom padding of app `index`'s open More menu (the
    /// popup fill, never an option), for a guest probe that the menu shows.
    pub fn installed_app_menu_padding(index: usize, width: u32, height: u32) -> (i32, i32) {
        let layout = Layout { width, height };
        let hero = Rect::new(X, TOP, layout.content_width(), 0);
        let button = app_menu_button(app_row(hero, index));
        let menu = Popup::menu(button, 200, 3, 0b001, Layout::TITLE, height as i32);
        (menu.rect.x + menu.rect.w as i32 / 2, menu.rect.bottom() - 2)
    }
}
static HOME: [Block; 4] = [
    Block::Hero(Hero::Home),
    Block::Group(Group {
        title: "Recommended settings",
        subtitle: "Recent and commonly used settings",
        hero: None,
        // 26H2 shows recently used pages here; Doors keeps no usage history,
        // so these are the commonly used ones (owner capture: Storage,
        // Installed apps, Lock screen; Doors has no lock screen settings).
        rows: rows![
            row!("Storage", "", "storage", link(Page::Storage)),
            row!("Installed apps", "", "list", link(Page::InstalledApps)),
            row!("Taskbar", "", "taskbar", link(Page::Taskbar)),
        ],
    }),
    Block::Group(Group {
        title: "Bluetooth devices",
        subtitle: "Manage, add, and remove devices",
        hero: Some(Hero::Bluetooth),
        rows: rows![row!(
            "View all devices",
            "",
            "devices",
            link(Page::Bluetooth)
        )],
    }),
    Block::Group(Group {
        title: "Personalize your device",
        subtitle: "Select a theme to apply",
        hero: Some(Hero::HomeThemes),
        rows: rows![
            row!("Color mode", "", "color", Control::Combo(Choice::Mode)),
            row!(
                "Browse more backgrounds, colors, and themes",
                "",
                "personalize",
                link(Page::Background)
            ),
        ],
    }),
];
static SYSTEM: [Block; 19] = [
    Block::Hero(Hero::Device),
    Block::Card(row!(
        "Display",
        "Monitors, brightness, night light, display profile",
        "display",
        link(Page::Display)
    )),
    Block::Card(row!(
        "Sound",
        "Volume levels, output, input, sound devices",
        "volume",
        link(Page::Sound)
    )),
    Block::Card(row!(
        "Notifications",
        "Alerts from apps and system, do not disturb",
        "ringer",
        link(Page::Notifications)
    )),
    Block::Card(row!(
        "Focus",
        "Reduce distractions",
        "quiet",
        link(Page::Focus)
    )),
    Block::Card(row!(
        "Power & battery",
        "Screen and sleep, battery usage, power mode, energy saver",
        "power",
        link(Page::Power)
    )),
    Block::Card(row!(
        "Storage",
        "Storage space, drives, configuration rules",
        "storage",
        link(Page::Storage)
    )),
    Block::Card(row!(
        "Share",
        "Share files, photos, links and more",
        "share",
        link(Page::Share)
    )),
    Block::Card(row!(
        "Multitasking",
        "Snap windows, desktops, task switching",
        "multitask",
        link(Page::Multitasking)
    )),
    Block::Card(row!(
        "Advanced",
        "Performance, optimization, and developer features",
        "code",
        link(Page::SystemAdvanced)
    )),
    Block::Card(row!(
        "Activation",
        "Activation state, subscriptions, product key",
        "key",
        link(Page::Activation)
    )),
    Block::Card(row!(
        "Troubleshoot",
        "Recommended troubleshooters, preferences, history",
        "repair",
        link(Page::Performance)
    )),
    Block::Card(row!(
        "Recovery",
        "Reset, advanced startup, go back",
        "restore",
        link(Page::BackupRestore)
    )),
    Block::Card(row!(
        "Projecting to this PC",
        "Permissions, pairing PIN, discoverability",
        "display",
        link(Page::ProjectingToThisPC)
    )),
    Block::Card(row!(
        "Remote Desktop",
        "Remote Desktop users, connection permissions",
        "remote",
        link(Page::RemoteDesktop)
    )),
    Block::Card(row!(
        "Clipboard",
        "Cut and copy history, sync, clear",
        "clipboard",
        link(Page::Clipboard)
    )),
    Block::Card(row!(
        "System components",
        "Manage system components that are preinstalled on Doors",
        "settings",
        link(Page::SystemComponents)
    )),
    Block::Card(row!(
        "Optional features",
        "Extra functionality for your device",
        "settings",
        link(Page::OptionalFeatures)
    )),
    Block::Card(row!(
        "About",
        "Device specifications, rename PC, Doors specifications",
        "info",
        link(Page::About)
    )),
];
static BLUETOOTH: [Block; 12] = [
    Block::Hero(Hero::Bluetooth),
    Block::Card(row!(
        "Devices",
        "Keyboard, mouse, display, disks and other devices",
        "devices",
        link(Page::DevicesDrivers)
    )),
    Block::Card(row!(
        "Printers & scanners",
        "Preferences",
        "print",
        link(Page::PrintersAndScanners)
    )),
    Block::Card(row!("Mobile devices", "", "phone", NA)),
    Block::Card(row!(
        "Cameras",
        "Connected cameras, default image settings",
        "camera",
        link(Page::Cameras)
    )),
    Block::Card(row!(
        "Mouse",
        "Buttons, mouse pointer speed, scrolling",
        "mouse",
        link(Page::Mouse)
    )),
    Block::Card(row!(
        "Keyboard",
        "PS/2 keyboard; character repeat and hotkeys can't be changed",
        "keyboard",
        link(Page::DevicesKeyboard)
    )),
    Block::Card(row!(
        "Touchpad",
        "Gestures, sensitivity",
        "touch",
        link(Page::Touchpad)
    )),
    Block::Card(row!(
        "Pen & Doors Ink",
        "",
        "pen",
        link(Page::PenAndWindowsInk)
    )),
    Block::Card(row!(
        "AutoPlay",
        "Defaults for removable drives and memory cards",
        "play",
        link(Page::AutoPlay)
    )),
    Block::Card(row!(
        "USB",
        "Notifications, USB battery saver",
        "usb",
        link(Page::USB)
    )),
    Block::Note(
        "Devices are detected at startup. Pairing new devices is not supported on this device.",
    ),
];
static NETWORK: [Block; 9] = [
    Block::Hero(Hero::Network),
    Block::Card(row!(
        "Wi-Fi",
        "Connect, manage known networks, metered network",
        "wifi",
        Control::ToggleLink(Toggle::Unavailable(false), Page::WiFi)
    )),
    Block::Card(row!(
        "Ethernet",
        "Authentication, IP and DNS settings, metered network",
        "ethernet",
        link(Page::Ethernet)
    )),
    Block::Card(row!("VPN", "Add, connect, manage", "vpn", link(Page::Vpn))),
    Block::Card(row!(
        "Mobile hotspot",
        "Share your internet connection",
        "wifi",
        Control::ToggleLink(Toggle::Unavailable(false), Page::MobileHotspot)
    )),
    Block::Card(row!(
        "Airplane mode",
        "Stop all wireless communication",
        "airplane",
        Control::ToggleLink(Toggle::Unavailable(false), Page::AirplaneMode)
    )),
    Block::Card(row!(
        "Proxy",
        "For Wi-Fi and Ethernet connections",
        "globe",
        link(Page::Proxy)
    )),
    Block::Card(row!(
        "Dial-up",
        "Set up a dial-up internet connection",
        "phone",
        link(Page::DialUp)
    )),
    Block::Card(row!(
        "Advanced network settings",
        "Adapters, addressing and connection administration",
        "network",
        link(Page::NetworkAdministration)
    )),
];
static PERSONALIZATION: [Block; 11] = [
    Block::Hero(Hero::Personalization),
    Block::Card(row!(
        "Background",
        "Background image, color, slideshow",
        "g-e91b",
        link(Page::Background)
    )),
    Block::Card(row!(
        "Colors",
        "Accent color, transparency effects, color theme",
        "g-e2b1",
        link(Page::Colors)
    )),
    Block::Card(row!(
        "Themes",
        "Install, create, manage",
        "personalize",
        link(Page::Themes)
    )),
    Block::Card(row!(
        "Dynamic Lighting",
        "Connected devices, effects, app settings",
        "effects",
        link(Page::DynamicLighting)
    )),
    Block::Card(row!(
        "Lock screen",
        "Lock screen images, apps, animations",
        "g-ee3f",
        link(Page::LockScreen)
    )),
    Block::Card(row!(
        "Text input",
        "Touch keyboard, voice typing, emoji and more, input method editor",
        // Windows draws f741; its 40 px mask blurs when area-resampled to
        // 30 px at 150 %, so the snapped keyboard glyph stays until native
        // 150 % masks exist.
        "keyboard",
        link(Page::TextInput)
    )),
    Block::Card(row!(
        "Start",
        "Recent apps and items, folders",
        "g-e1e4",
        link(Page::Start)
    )),
    Block::Card(row!(
        "Taskbar",
        "Taskbar behaviors, system pins",
        "g-e147",
        link(Page::Taskbar)
    )),
    Block::Card(row!(
        "Context menu",
        "File actions, Show more options and Properties placement",
        "list",
        link(Page::ContextMenu)
    )),
    Block::Card(row!(
        "Fonts",
        "Install, manage",
        "g-e185",
        link(Page::Fonts)
    )),
];
static APPS: [Block; 8] = [
    Block::Card(row!(
        "Installed apps",
        "Uninstall and manage apps on your PC",
        "list",
        link(Page::InstalledApps)
    )),
    Block::Card(row!(
        "Advanced app settings",
        "Choose where to get apps, archive apps, uninstall updates",
        "apps",
        link(Page::AdvancedAppSettings)
    )),
    Block::Card(row!(
        "Default apps",
        "Defaults for file and link types",
        "apps",
        link(Page::DefaultApps)
    )),
    Block::Card(row!(
        "Offline maps",
        "Downloads, storage location, map updates",
        "location",
        link(Page::OfflineMaps)
    )),
    Block::Card(row!(
        "Optional features",
        "Extra functionality for your device",
        "apps",
        link(Page::OptionalFeatures)
    )),
    Block::Card(row!(
        "Apps for websites",
        "Websites that can open in an app instead of a browser",
        "link",
        link(Page::AppsForWebsites)
    )),
    Block::Card(row!(
        "Video playback",
        "Video adjustments, HDR streaming, battery options",
        "video",
        link(Page::VideoPlayback)
    )),
    Block::Card(row!(
        "Startup",
        "Apps that open automatically when you sign in",
        "power",
        link(Page::Startup)
    )),
];
static ACCOUNTS: [Block; 9] = [
    Block::Hero(Hero::Account),
    Block::Header("Account settings"),
    Block::Card(row!(
        "Your info",
        "Profile photo",
        "contact",
        link(Page::UsersGroups)
    )),
    Block::Card(row!(
        "Sign-in options",
        "PIN, security key, password, dynamic lock",
        "key",
        link(Page::SignInOptions)
    )),
    Block::Card(row!(
        "Your accounts",
        "Add or manage accounts used in Doors",
        "mail",
        link(Page::YourAccounts)
    )),
    Block::Card(row!(
        "Other users",
        "Device access, work or school users, kiosk assigned access",
        "people",
        link(Page::OtherUsers)
    )),
    Block::Card(row!(
        "Access work or school",
        "Organization resources like email, apps, and network",
        "work",
        link(Page::AccessWorkOrSchool)
    )),
    Block::Card(row!(
        "Passkeys",
        "Use your face, fingerprint, or PIN to sign in to apps and websites",
        "g-ee7e",
        link(Page::AccountsPasskeys)
    )),
    Block::Card(row!(
        "Saved credentials",
        "Web and Doors credentials saved for your account",
        "key",
        NA
    )),
];
static TIME: [Block; 4] = [
    Block::Card(row!(
        "Date & time",
        "Time zone, automatic clock settings, calendar display",
        "calendar",
        link(Page::DateTime)
    )),
    Block::Card(row!(
        "Language & region",
        "Doors and some apps format dates and time based on your region",
        "world",
        link(Page::LanguageRegion)
    )),
    Block::Card(row!(
        "Typing",
        "Keyboards, input method editors and input switching",
        "keyboard",
        link(Page::Typing)
    )),
    Block::Card(row!(
        "Speech",
        "Voice access, voice typing",
        "speech",
        link(Page::TimeSpeech)
    )),
];
static GAMING: [Block; 4] = [
    Block::Card(row!("Game Bar", "", "game", NA)),
    Block::Card(row!(
        "Captures",
        "Save location, recording preferences",
        "record",
        NA
    )),
    Block::Card(row!(
        "Game Mode",
        "Optimize your PC for play",
        "game",
        Control::ToggleLink(Toggle::Unavailable(false), Page::GameMode)
    )),
    Block::Note("Games run as ordinary apps on this device."),
];
static CONTRAST_PAGE: [Block; 1] = [Block::Hero(Hero::Contrast)];
static TEXT_SIZE_PAGE: [Block; 3] = [
    Block::Hero(Hero::TextSize),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Display",
        "Scale, resolution and orientation",
        "display",
        link(Page::Display)
    )),
];
static ACCESSIBILITY: [Block; 19] = [
    Block::Header("Vision"),
    Block::Card(row!(
        "Text size",
        "Text size that appears throughout Doors and your apps",
        "font",
        link(Page::TextSize)
    )),
    Block::Card(row!(
        "Visual effects",
        "Scroll bars, transparency, animations, notification timeout",
        "effects",
        link(Page::VisualEffects)
    )),
    Block::Card(row!(
        "Mouse pointer and touch",
        "Mouse pointer color, size",
        "pointer",
        link(Page::MousePointerAndTouch)
    )),
    Block::Card(row!(
        "Text cursor",
        "Appearance and thickness, text cursor indicator",
        "text-cursor",
        link(Page::TextCursor)
    )),
    Block::Card(row!(
        "Magnifier",
        "Magnifier reading, zoom increment",
        "search",
        link(Page::Magnifier)
    )),
    Block::Card(row!(
        "Color filters",
        "Colorblindness filters, grayscale, inverted",
        "color",
        link(Page::ColorFilters)
    )),
    Block::Card(row!(
        "Screen tint",
        "Add a subtle color overlay to make your screen more comfortable to view",
        "g-e81e",
        link(Page::ScreenTint)
    )),
    Block::Card(row!(
        "Contrast themes",
        "Color themes for low vision, light sensitivity",
        "contrast",
        link(Page::ContrastThemes)
    )),
    Block::Card(row!(
        "Narrator",
        "Voice, verbosity, keyboard, braille",
        "narrator",
        link(Page::Narrator)
    )),
    Block::Header("Hearing"),
    Block::Card(row!(
        "Audio",
        "Mono audio, audio notifications",
        "volume",
        link(Page::Audio)
    )),
    Block::Card(row!(
        "Hearing devices",
        "Presets, environment sound control, audio routing",
        "g-e9ce",
        link(Page::HearingDevices)
    )),
    Block::Card(row!(
        "Captions",
        "Styles, live captions",
        "captions",
        link(Page::Captions)
    )),
    Block::Header("Interaction"),
    Block::Card(row!(
        "Speech",
        "Voice access, voice typing",
        "speech",
        link(Page::AccessibilitySpeech)
    )),
    Block::Card(row!(
        "Keyboard",
        "Sticky, Filter, and Toggle keys, on-screen keyboard",
        "keyboard",
        link(Page::AccessibilityKeyboard)
    )),
    Block::Card(row!(
        "Mouse",
        "Mouse keys, speed, acceleration",
        "mouse",
        link(Page::AccessibilityMouse)
    )),
    Block::Card(row!(
        "Eye control",
        "Eye tracker, text-to-speech",
        "eye",
        link(Page::EyeControl)
    )),
];
static PRIVACY: [Block; 38] = [
    Block::Header("Core privacy settings"),
    Block::Card(row!("Location", "", "g-e37a", link(Page::Location))),
    Block::Card(row!("Camera", "", "g-e722", link(Page::Camera))),
    Block::Card(row!("Microphone", "", "microphone", link(Page::Microphone))),
    Block::Header("Security"),
    Block::Card(row!(
        "Doors Security",
        "Antivirus, browser, firewall, and network protection for your device",
        "g-ea18",
        link(Page::WindowsSecurity)
    )),
    Block::Card(row!(
        "Firewall & network protection",
        "Who and what can access your networks",
        "shield",
        link(Page::Firewall)
    )),
    Block::Card(row!(
        "Passkeys",
        "Passkey services and sync settings",
        "g-ee7e",
        link(Page::PrivacyPasskeys)
    )),
    Block::Header("Your info"),
    Block::Card(row!(
        "Custom dictionary",
        "",
        "g-f67b",
        link(Page::CustomDictionary)
    )),
    Block::Card(row!("Account info", "", "g-e779", link(Page::AccountInfo))),
    Block::Card(row!("Contacts", "", "g-e125", link(Page::Contacts))),
    Block::Card(row!("Calendar", "", "g-e787", link(Page::Calendar))),
    Block::Card(row!("Tasks", "", "g-f7ec", link(Page::Tasks))),
    Block::Card(row!(
        "Search",
        "Search results, history and indexing",
        "search",
        link(Page::Search)
    )),
    Block::Card(row!(
        "Screenshot borders",
        "",
        "g-e91b",
        link(Page::ScreenshotBorders)
    )),
    Block::Card(row!(
        "Screenshots and screen recording",
        "",
        "g-e722",
        link(Page::ScreenshotsAndScreenRecording)
    )),
    Block::Header("Communication"),
    Block::Card(row!(
        "Voice activation",
        "",
        "g-f12e",
        link(Page::VoiceActivation)
    )),
    Block::Card(row!("Email", "", "g-e119", link(Page::Email))),
    Block::Card(row!("Messaging", "", "g-e15f", link(Page::Messaging))),
    Block::Card(row!("Calls", "", "g-e717", link(Page::Calls))),
    Block::Card(row!("Call history", "", "g-e81c", link(Page::CallHistory))),
    Block::Card(row!("Speech", "", "g-efa9", link(Page::PrivacySpeech))),
    Block::Header("File access"),
    Block::Card(row!(
        "File system",
        "File system access, and checking access to your files and folders",
        "g-e8b7",
        link(Page::PermissionsSharing)
    )),
    Block::Card(row!("Downloads", "", "g-e118", link(Page::Downloads))),
    Block::Card(row!("Documents", "", "g-e130", link(Page::Documents))),
    Block::Card(row!("Pictures", "", "g-e91b", link(Page::Pictures))),
    Block::Card(row!("Music", "", "g-e8d6", link(Page::Music))),
    Block::Card(row!("Videos", "", "g-e714", link(Page::Videos))),
    Block::Header("System"),
    Block::Card(row!(
        "App diagnostics",
        "",
        "g-e9d9",
        link(Page::AppDiagnostics)
    )),
    Block::Card(row!(
        "Notifications",
        "",
        "g-ea8f",
        link(Page::PrivacyNotifications)
    )),
    Block::Card(row!("Radios", "", "g-ec05", link(Page::Radios))),
    Block::Card(row!(
        "Other devices",
        "",
        "g-e772",
        link(Page::OtherDevices)
    )),
    Block::Card(row!(
        "Automatic file downloads",
        "",
        "g-e753",
        link(Page::AutomaticFileDownloads)
    )),
    Block::Card(row!(
        "Foreground text access",
        "",
        "g-f741",
        link(Page::ForegroundTextAccess)
    )),
];
static UPDATE: [Block; 6] = [
    Block::Hero(Hero::Update),
    Block::Header("More options"),
    Block::Card(row!(
        "Pause updates",
        "",
        "pause",
        Control::Button("Pause for 1 week")
    )),
    Block::Card(row!(
        "Update history",
        "",
        "history",
        link(Page::UpdateHistory)
    )),
    Block::Card(row!(
        "Advanced options",
        "Manage where your passkeys are saved",
        "settings",
        link(Page::WindowsUpdateAdvancedOptions)
    )),
    Block::Card(row!("Doors Insider Program", "", "people", NA)),
];
/// System > About (26H2): hardware tiles, the name card with Rename this
/// PC, Device specifications with its Related links, Doors specifications.
static ABOUT: [Block; 4] = [
    Block::Hero(Hero::AboutTiles),
    Block::Hero(Hero::AboutDevice),
    Block::Hero(Hero::DeviceSpecs),
    Block::Hero(Hero::DoorsSpecs),
];
static DISPLAY: [Block; 16] = [
    Block::Hero(Hero::Monitor),
    Block::Header("Brightness & color"),
    Block::Card(row!(
        "Brightness",
        "Adjust the brightness of the built-in display",
        "brightness",
        Control::Slider(Slider::Brightness)
    )),
    Block::Card(row!(
        "Night light",
        "Use warmer colors to help block blue light",
        "night",
        Control::ToggleLink(
            Toggle::Pref {
                id: p::VISION,
                bit: p::vision::NIGHT_LIGHT
            },
            Page::NightLight
        )
    )),
    Block::Card(row!(
        "Color profile",
        "Determine how colors appear on your screen",
        "color",
        NA
    )),
    Block::Card(row!(
        "HDR",
        "Display capabilities, use HDR, Auto HDR",
        "display",
        link(Page::HDR)
    )),
    Block::Header("Scale & layout"),
    Block::Card(row!(
        "Scale",
        "Change the size of text, apps, and other items",
        "zoom",
        Control::Combo(Choice::Scale)
    )),
    Block::Card(row!(
        "Display resolution",
        "Adjust the resolution to fit your connected display",
        "resolution",
        Control::Value(Fact::Resolution)
    )),
    Block::Card(row!(
        "Display orientation",
        "",
        "rotate",
        Control::Value(Fact::Orientation)
    )),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Multiple displays",
            "Choose the presentation mode for your displays",
            "multitask",
            Control::None
        ),
        hero: None,
        rows: rows![
            row!(
                "Remember window locations based on monitor connection",
                "",
                "",
                ON
            ),
            row!(
                "Minimize windows when a monitor is disconnected",
                "",
                "",
                ON
            ),
            row!("Detect other display", "", "", Control::Button("Detect")),
            row!(
                "Connect to a wireless display",
                "",
                "",
                Control::Button("Connect")
            ),
        ],
    }),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Advanced display",
        "Display information, refresh rate",
        "display",
        link(Page::AdvancedDisplay)
    )),
    Block::Card(row!("Graphics", "", "g-f211", link(Page::Graphics))),
    Block::Card(row!(
        "Screen tint",
        "Add a subtle color overlay to make your screen more comfortable to view",
        "g-e81e",
        link(Page::ScreenTint)
    )),
    Block::Card(row!(
        "Position of on-screen indicators",
        "Volume, brightness and other controls will appear in this location",
        "g-e75a",
        NA
    )),
];
static INSTALLED: [Block; 1] = [Block::Hero(Hero::Apps)];
static TASKBAR: [Block; 8] = [
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Taskbar items",
            "Show or hide buttons that appear on the taskbar",
            "taskbar",
            Control::None
        ),
        hero: None,
        rows: rows![
            row!(
                "Search",
                "Top and side taskbar positions show the search icon only",
                "search",
                Control::Combo(Choice::Search)
            ),
            row!(
                "Task view",
                "",
                "taskview",
                Control::Toggle(Toggle::PrefOff {
                    id: p::TASKBAR,
                    bit: p::taskbar::HIDE_TASK_VIEW
                })
            ),
        ],
    }),
    Block::Expander(Expander {
        id: 2,
        header: row!(
            "System tray icons",
            "Show or hide additional system tray icons",
            "devices",
            Control::None
        ),
        hero: None,
        rows: rows![
            row!("Pen menu", "", "pen", OFF),
            row!("Touch keyboard", "", "keyboard", OFF),
            row!("Virtual touchpad", "", "touch", OFF),
        ],
    }),
    Block::Card(row!(
        "Other system tray icons",
        "Show or hide additional system tray icons",
        "apps",
        NA
    )),
    Block::Expander(Expander {
        id: 4,
        header: row!(
            "Taskbar behaviors",
            "Taskbar alignment, position, size, badging, automatically hide, and multiple displays",
            "settings",
            Control::None
        ),
        hero: None,
        rows: rows![
            row!(
                "Taskbar alignment",
                "",
                "",
                Control::Combo(Choice::Alignment)
            ),
            row!(
                "Taskbar position",
                "Move the taskbar to an edge of your screen",
                "",
                Control::Combo(Choice::Position)
            ),
            row!(
                "Taskbar size",
                "Small uses compact buttons and a shorter taskbar",
                "",
                Control::Combo(Choice::TaskbarSize)
            ),
            row!(
                "Automatically hide the taskbar",
                "",
                "",
                Control::Toggle(Toggle::Pref {
                    id: p::TASKBAR,
                    bit: p::taskbar::AUTO_HIDE
                })
            ),
            row!(
                "Show badges on taskbar apps",
                "",
                "",
                Control::Toggle(Toggle::PrefOff {
                    id: p::TASKBAR,
                    bit: p::taskbar::NO_BADGES
                })
            ),
            row!(
                "Show flashing on taskbar apps",
                "",
                "",
                Control::Toggle(Toggle::PrefOff {
                    id: p::TASKBAR,
                    bit: p::taskbar::NO_FLASHING
                })
            ),
            row!(
                "Show my taskbar on all displays",
                "Only one display is supported",
                "",
                OFF
            ),
            row!("Share any window from my taskbar", "", "", OFF),
            row!(
                "Select the far corner of the taskbar to show the desktop",
                "",
                "",
                Control::Toggle(Toggle::PrefOff {
                    id: p::TASKBAR,
                    bit: p::taskbar::NO_DESKTOP_CORNER
                })
            ),
            row!(
                "Combine taskbar buttons and hide labels",
                "",
                "",
                Control::Combo(Choice::Combine)
            ),
            row!(
                "Show seconds in system tray clock",
                "Uses more power",
                "",
                Control::Toggle(Toggle::Pref {
                    id: p::TASKBAR,
                    bit: p::taskbar::SECONDS
                })
            ),
        ],
    }),
    // Doors extras (TranslucentTB, Windhawk): off until the user acts.
    Block::Expander(Expander {
        id: 5,
        header: row!(
            "More taskbar options",
            "Look, middle-click to close, last active window",
            "taskbar",
            Control::None
        ),
        hero: None,
        rows: rows![
            row!(
                "Taskbar look",
                "Translucent, opaque or clear background",
                "",
                Control::Combo(Choice::TaskbarLook)
            ),
            row!(
                "Middle-click a taskbar button to close its windows",
                "",
                "",
                Control::Toggle(Toggle::Pref {
                    id: p::TASKBAR,
                    bit: p::taskbar::MIDDLE_CLOSE
                })
            ),
            row!(
                "Click a grouped app to switch to its last active window",
                "",
                "",
                Control::Toggle(Toggle::Pref {
                    id: p::TASKBAR,
                    bit: p::taskbar::LAST_ACTIVE
                })
            ),
        ],
    }),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Date & time",
        "Time zones, automatic clock settings, calendar display",
        "g-ec92",
        link(Page::DateTime)
    )),
    Block::Card(row!(
        "Notifications",
        "Alerts from apps and system, do not disturb",
        "g-ea8f",
        link(Page::Notifications)
    )),
];
static CONTEXT: [Block; 12] = [
    Block::Card(row!(
        "File actions",
        "Show Send to, Open with and other file actions",
        "share",
        Control::Toggle(Toggle::inverted(Controls::CONTEXT_HIDE_FILE_ACTIONS))
    )),
    Block::Card(row!("App extensions", "", "apps", NA)),
    Block::Card(row!(
        "Show more options",
        "Show the entry that opens the full classic menu",
        "more",
        Control::Toggle(Toggle::inverted(Controls::CONTEXT_HIDE_MORE_OPTIONS))
    )),
    Block::Card(row!(
        "Properties placement",
        "Where Properties appears in the File Explorer context menu",
        "repair",
        Control::Combo(Choice::PropertiesPlacement)
    )),
    Block::Header("Additional options"),
    Block::Card(row!(
        "Arrange primary actions inline",
        "Show cut, copy, paste, rename, share and delete as icons at the top",
        "settings",
        Control::Toggle(Toggle::control(16))
    )),
    Block::Card(row!(
        "Show the classic context menu",
        "Open the full menu directly instead of the compact Doors 11 menu",
        "list",
        Control::Toggle(Toggle::control(Controls::CLASSIC_CONTEXT_MENU))
    )),
    Block::Card(row!(
        "Keep cloud storage actions in their own section",
        "",
        "cloud",
        OFF
    )),
    Block::Card(row!(
        "Customize the context menu",
        "Choose, order and group what each right-click shows, with a live preview",
        "list",
        Control::Command("Customize", Command::CustomizeContextMenu)
    )),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Taskbar",
        "Taskbar behaviors, system pins",
        "taskbar",
        link(Page::Taskbar)
    )),
    Block::Card(row!(
        "Start",
        "Size, sections, recent files and folders",
        "start",
        link(Page::Start)
    )),
];
static SOUND: [Block; 14] = [
    Block::Header("Output"),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Choose where to play sound",
            "Apps might have their own settings",
            "volume",
            Control::Value(Fact::Audio(AudioFact::Device))
        ),
        hero: None,
        rows: rows![
            row!(
                "Speakers",
                "VirtIO Sound",
                "",
                Control::Value(Fact::Audio(AudioFact::Level))
            ),
            row!("Pair a new output device", "", "", NA)
        ],
    }),
    Block::Card(row!(
        "Volume",
        "",
        "volume",
        Control::Slider(Slider::Volume)
    )),
    Block::Expander(Expander {
        id: 2,
        header: row!(
            "Speakers",
            "Properties: format, balance, test",
            "volume",
            Control::Command("Test", Command::TestSound)
        ),
        hero: None,
        rows: rows![
            row!(
                "Format",
                "",
                "",
                Control::Value(Fact::Audio(AudioFact::Format))
            ),
            row!(
                "Left / right balance",
                "Left at the start, right at the end",
                "",
                Control::Slider(Slider::Balance)
            ),
            row!(
                "Spatial sound",
                "Off: Doors Sonic isn't available",
                "",
                Control::Value(Fact::Static("Off"))
            ),
            row!("Enhance audio", "", "", OFF)
        ],
    }),
    Block::Card(row!(
        "Mono audio",
        "Combine left and right audio channels into one",
        "volume",
        Control::Toggle(Toggle::Pref {
            id: p::SOUND,
            bit: 1 << 9
        })
    )),
    Block::Header("Input"),
    Block::Card(row!(
        "Choose a device for speaking or recording",
        "Apps might have their own settings",
        "microphone",
        Control::Value(Fact::Audio(AudioFact::Input))
    )),
    Block::Card(row!(
        "Volume",
        "",
        "microphone",
        Control::Slider(Slider::InputVolume)
    )),
    Block::Header("Advanced"),
    Block::Card(row!(
        "Troubleshoot common sound problems",
        "Output devices",
        "repair",
        Control::Command("Run", Command::RunChecks)
    )),
    Block::Card(row!(
        "All sound devices",
        "Turn devices on and off, troubleshoot, other options",
        "settings",
        link(Page::AllSoundDevices)
    )),
    Block::Card(row!(
        "Volume mixer",
        "App volume mix, app input and output devices",
        "settings",
        link(Page::VolumeMixer)
    )),
    Block::Card(row!(
        "More sound settings",
        "The classic Sound control panel opens this page in Doors",
        "window",
        NA
    )),
    Block::Note("Volume changes apply to every app; the taskbar's speaker shows the level."),
];
static NOTIFICATIONS: [Block; 9] = [
    Block::Expander(Expander {
        id: 4,
        header: row!(
            "Notifications",
            "Get notifications from apps and other senders",
            "ringer",
            Control::Toggle(Toggle::Pref {
                id: p::NOTIFICATIONS,
                bit: p::notify::ON
            })
        ),
        hero: None,
        rows: rows![
            row!(
                "Allow notifications to play sounds",
                "",
                "",
                Control::Toggle(Toggle::Pref {
                    id: p::NOTIFICATIONS,
                    bit: p::notify::SOUNDS
                })
            ),
            row!("Show notifications on the lock screen", "", "", OFF),
            row!(
                "Show reminders and incoming VoIP calls on the lock screen",
                "",
                "",
                OFF
            ),
            row!("Show notification bell icon", "", "", OFF),
        ],
    }),
    Block::Card(row!(
        "Do not disturb",
        "Notifications will be sent directly to notification center",
        "quiet",
        Control::Toggle(Toggle::Pref {
            id: p::NOTIFICATIONS,
            bit: p::notify::DND
        })
    )),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Turn on do not disturb automatically",
            "",
            "history",
            Control::None
        ),
        hero: None,
        rows: rows![
            row!(
                "During these times",
                "",
                "",
                Control::Toggle(Toggle::Pref {
                    id: p::NOTIFICATIONS,
                    bit: p::notify::DND_SCHEDULED
                })
            ),
            row!("From", "", "", Control::Combo(Choice::DndFrom)),
            row!("To", "", "", Control::Combo(Choice::DndTo)),
            row!("Repeats", "", "", Control::Combo(Choice::DndRepeat)),
            row!(
                "When duplicating your display (priority notification banners are also hidden)",
                "",
                "",
                OFF
            ),
            row!("When playing a game", "", "", OFF),
            row!(
                "When using an app in full-screen mode (priority notification banners are also hidden)",
                "",
                "",
                OFF
            ),
            row!(
                "For the first hour after a Doors feature update",
                "",
                "",
                OFF
            ),
        ],
    }),
    Block::Card(row!("Set priority notifications", "", "g-ea8f", NA)),
    Block::Card(row!(
        "Focus",
        "Session duration, hide badges on apps",
        "eye",
        link(Page::Focus)
    )),
    Block::Header("Notifications from apps and other senders"),
    Block::Note("Apps and other senders appear here after they notify you."),
    Block::Header("Indicators from keyboard actions"),
    Block::Card(row!(
        "Position of on-screen indicators",
        "",
        "g-e75a",
        Control::Combo(Choice::Unavailable(
            &[
                "Top left",
                "Top center",
                "Top right",
                "Bottom left",
                "Bottom center",
                "Bottom right"
            ],
            4
        ))
    )),
];
/// Power timeouts (26H2 lists, Plugged in).
const SCREEN_OFF: [&str; 14] = [
    "1 minute",
    "2 minutes",
    "3 minutes",
    "5 minutes",
    "10 minutes",
    "15 minutes",
    "20 minutes",
    "25 minutes",
    "30 minutes",
    "45 minutes",
    "1 hour",
    "2 hours",
    "5 hours",
    "Never",
];
const POWER_BUTTON: [&str; 5] = [
    "Do nothing",
    "Sleep",
    "Hibernate",
    "Shut down",
    "Turn off the display",
];
static POWER: [Block; 8] = [
    Block::Card(row!(
        "Energy recommendations",
        "Lower your carbon footprint by applying these recommendations",
        "g-e8be",
        link(Page::EnergyRecommendations)
    )),
    Block::Card(row!(
        "Power Mode",
        "Optimize your device based on power use and performance",
        "g-ec48",
        Control::Combo(Choice::Unavailable(
            &["Best Power Efficiency", "Balanced", "Best Performance"],
            1
        ))
    )),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Screen, sleep, & hibernate timeouts",
            "Choose what happens when your device is idle for a specified time",
            "g-ee3f",
            Control::None
        ),
        hero: None,
        rows: rows![
            row!(
                "Turn my screen off after",
                "",
                "",
                Control::Combo(Choice::Unavailable(&SCREEN_OFF, 13))
            ),
            row!(
                "Make my device sleep after",
                "",
                "",
                Control::Combo(Choice::Unavailable(&SCREEN_OFF, 13))
            ),
            row!(
                "Make my device hibernate after",
                "",
                "",
                Control::Combo(Choice::Unavailable(&SCREEN_OFF, 13))
            ),
        ],
    }),
    Block::Expander(Expander {
        id: 2,
        header: row!(
            "Energy saver",
            "Reduce power consumption and increase battery life by limiting some background activities",
            "g-f432",
            Control::None
        ),
        hero: None,
        rows: rows![row!("Always use energy saver", "", "", OFF)],
    }),
    Block::Expander(Expander {
        id: 3,
        header: row!(
            "Lid, power & sleep button controls",
            "Choose what happens when you interact with your device's physical controls",
            "g-e179",
            Control::None
        ),
        hero: None,
        rows: rows![row!(
            "Pressing the power button will make my PC",
            "",
            "",
            Control::Combo(Choice::Unavailable(&POWER_BUTTON, 3))
        )],
    }),
    Block::Header("Related settings"),
    // The Power Options control panel, unified here.
    Block::Card(row!(
        "Power options",
        "Power plans, power buttons, sleep and hibernate",
        "power",
        link(Page::PowerManagement)
    )),
    Block::Note("Battery settings appear on devices with a battery."),
];
static STORAGE: [Block; 8] = [
    Block::Hero(Hero::Storage),
    Block::Header("Storage management"),
    Block::Card(row!(
        "Storage Sense",
        "Automatically free up space, delete temporary files, and manage locally available cloud content",
        "storage",
        Control::ToggleLink(Toggle::Unavailable(false), Page::StorageSense)
    )),
    Block::Card(row!("Cleanup recommendations", "", "effects", NA)),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Advanced storage settings",
            "Backup options, Storage Spaces, other disks and volumes",
            "settings",
            Control::None
        ),
        hero: None,
        rows: rows![
            row!(
                "Storage used on other drives",
                "",
                "",
                link(Page::DiskManagement)
            ),
            row!("Where new content is saved", "", "", NA),
            row!("Storage Spaces", "", "", NA),
            row!(
                "Disks & volumes",
                "Your disk, its volumes and the storage journal",
                "",
                link(Page::DiskManagement)
            ),
            row!("Backup options", "", "", NA),
            row!("Drive optimization", "", "", NA),
        ],
    }),
    Block::Header("Related"),
    Block::Card(row!(
        "Devices",
        "Disks, display, processors and input devices",
        "devices",
        link(Page::DevicesDrivers)
    )),
    Block::Note("Usage by category is unavailable: the volume reports only free and total space."),
];
// Personalization > Background: Windows shows the rows of the chosen kind.
static BACKGROUND: [Block; 4] = [
    Block::Hero(Hero::Wallpaper),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Personalize your background",
            "A picture background applies to your current desktop. Solid color or slideshow backgrounds apply to all your desktops.",
            "g-e91b",
            Control::Combo(Choice::BackgroundKind)
        ),
        hero: Some(Hero::Backgrounds),
        rows: rows![
            row!("Choose a photo", "", "", Control::Button("Browse photos")),
            row!(
                "Choose a fit for your desktop image",
                "",
                "",
                Control::Combo(Choice::BackgroundFit)
            ),
        ],
    }),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Contrast themes",
        "Color themes for low vision, light sensitivity",
        "contrast",
        link(Page::ContrastThemes)
    )),
];
static BACKGROUND_SOLID: [Block; 4] = [
    Block::Hero(Hero::Wallpaper),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Personalize your background",
            "A picture background applies to your current desktop. Solid color or slideshow backgrounds apply to all your desktops.",
            "g-e91b",
            Control::Combo(Choice::BackgroundKind)
        ),
        hero: Some(Hero::SolidColors),
        rows: rows![row!(
            "Custom colors",
            "",
            "",
            Control::Button("View colors")
        ),],
    }),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Contrast themes",
        "Color themes for low vision, light sensitivity",
        "contrast",
        link(Page::ContrastThemes)
    )),
];
static BACKGROUND_SLIDESHOW: [Block; 4] = [
    Block::Hero(Hero::Wallpaper),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Personalize your background",
            "A picture background applies to your current desktop. Solid color or slideshow backgrounds apply to all your desktops.",
            "g-e91b",
            Control::Combo(Choice::BackgroundKind)
        ),
        hero: None,
        rows: rows![
            row!(
                "Choose a picture album for a slideshow",
                "The pictures of the current background's theme",
                "",
                Control::Button("Browse")
            ),
            row!(
                "Change picture every",
                "",
                "",
                Control::Combo(Choice::SlideshowInterval)
            ),
            row!(
                "Shuffle the picture order",
                "",
                "",
                Control::Toggle(Toggle::Pref {
                    id: p::BACKGROUND,
                    bit: 1 << 7
                })
            ),
            row!("Let slideshow run even if I'm on battery power", "", "", ON),
            row!(
                "Choose a fit for your desktop image",
                "",
                "",
                Control::Combo(Choice::BackgroundFit)
            ),
        ],
    }),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Contrast themes",
        "Color themes for low vision, light sensitivity",
        "contrast",
        link(Page::ContrastThemes)
    )),
];
static COLORS: [Block; 9] = [
    Block::Hero(Hero::Wallpaper),
    Block::Card(row!(
        "Choose your mode",
        "Change the colors that appear in Doors and your apps",
        "color",
        Control::Combo(Choice::Mode)
    )),
    Block::Card(row!(
        "Transparency effects",
        "Windows and surfaces appear translucent",
        "effects",
        Control::Toggle(Toggle::inverted(Controls::REDUCE_TRANSPARENCY))
    )),
    Block::Expander(Expander {
        id: 2,
        header: row!(
            "Accent color",
            "",
            "color",
            Control::Combo(Choice::AccentMode)
        ),
        hero: Some(Hero::Accents),
        rows: rows![
            row!("Custom colors", "", "", Control::Button("View colors")),
            row!(
                "Show accent color on Start and taskbar",
                "",
                "",
                Control::Toggle(Toggle::Pref {
                    id: p::ACCENT_SURFACES,
                    bit: p::ACCENT_START_TASKBAR
                })
            ),
            row!(
                "Show accent color on title bars and window borders",
                "",
                "",
                Control::Toggle(Toggle::Pref {
                    id: p::ACCENT_SURFACES,
                    bit: p::ACCENT_TITLE_BARS
                })
            ),
        ],
    }),
    // Doors extra (Auto Dark Mode): switch modes at fixed times.
    Block::Expander(Expander {
        id: 3,
        header: row!(
            "More color options",
            "Switch between light and dark mode on a schedule",
            "history",
            Control::None
        ),
        hero: None,
        rows: rows![
            row!(
                "Switch mode automatically",
                "Light mode by day, dark mode at night",
                "history",
                Control::Toggle(Toggle::Pref {
                    id: p::THEME_SCHEDULE,
                    bit: 1
                })
            ),
            row!(
                "Light mode from",
                "",
                "brightness",
                Control::Combo(Choice::ScheduleLight)
            ),
            row!(
                "Dark mode from",
                "",
                "night",
                Control::Combo(Choice::ScheduleDark)
            ),
        ],
    }),
    // Doors extras (square corners, MicaForEveryone).
    Block::Expander(Expander {
        id: 4,
        header: row!(
            "Window style",
            "Corner shape and Mica in every app",
            "personalize",
            Control::None
        ),
        hero: None,
        rows: rows![
            row!("Window corners", "", "", Control::Combo(Choice::Corners)),
            row!(
                "Use Mica in every app",
                "Apps that don't choose a material get Mica behind their title bar",
                "",
                Control::Toggle(Toggle::Pref {
                    id: p::WINDOWS,
                    bit: p::windows::MICA_EVERYWHERE
                })
            ),
        ],
    }),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Contrast themes",
        "Color themes for low vision, light sensitivity",
        "contrast",
        link(Page::ContrastThemes)
    )),
    Block::Card(row!(
        "Visual effects",
        "Animation effects and transparency",
        "effects",
        link(Page::VisualEffects)
    )),
];
static THEMES_PAGE: [Block; 5] = [
    Block::Hero(Hero::ThemePreview),
    Block::Expander(Expander {
        id: 2,
        header: row!(
            "Current theme",
            "Choose a combination of wallpapers, sounds, and colors together to give your desktop more personality",
            "",
            Control::Value(Fact::CurrentTheme)
        ),
        hero: Some(Hero::Themes),
        rows: rows![],
    }),
    Block::Header("Related settings"),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Desktop icon settings",
            "Show or hide icons on the desktop",
            "desktop",
            Control::None
        ),
        hero: None,
        rows: rows![row!(
            "Show desktop icons",
            "This PC, user files and Network on the desktop",
            "",
            Control::Toggle(Toggle::control(Controls::SHOW_DESKTOP_ICONS))
        )],
    }),
    Block::Card(row!(
        "Contrast themes",
        "Color themes for low vision, light sensitivity",
        "contrast",
        link(Page::ContrastThemes)
    )),
];
static START: [Block; 14] = [
    // Doors' Start layout (its size and sections; not on the owner's 26H2).
    Block::Header("Layout"),
    Block::Card(row!(
        "Start size",
        "Automatic chooses the size for your display",
        "start",
        Control::Combo(Choice::StartSize)
    )),
    Block::Card(row!(
        "Show Pinned",
        "Apps you pin to Start",
        "start",
        Control::Toggle(Toggle::inverted(Controls::START_HIDE_PINNED))
    )),
    Block::Card(row!(
        "Show Recent",
        "Recently installed apps and recent files",
        "history",
        Control::Toggle(Toggle::inverted(Controls::START_HIDE_RECENT))
    )),
    Block::Card(row!(
        "Show All",
        "Every installed app",
        "apps",
        Control::Toggle(Toggle::inverted(Controls::START_HIDE_ALL))
    )),
    Block::Card(row!(
        "Show account name and picture",
        "At the bottom of Start, next to the power button",
        "contact",
        Control::Toggle(Toggle::inverted(Controls::START_HIDE_ACCOUNT))
    )),
    Block::Header("Recommended"),
    // Recently installed apps always appear in Doors' Recent section.
    Block::Card(row!("Show recently added apps", "", "g-e118", ON)),
    Block::Card(row!(
        "Show recommended files in Start, recent files in File Explorer, and items in Jump Lists",
        "",
        "list",
        Control::Toggle(Toggle::inverted(Controls::START_HIDE_RECENT_FILES))
    )),
    Block::Header("All"),
    Block::Card(row!("Show most used apps", "", "g-e734", OFF)),
    Block::Header("Other"),
    Block::Card(row!(
        "Show account-related notifications",
        "When off, required notifications are still shown",
        "contact",
        OFF
    )),
    Block::Card(row!(
        "Folders",
        "These folders appear on Start next to the Power button",
        "g-e8b7",
        link(Page::Folders)
    )),
];
const fn folder_toggle(index: u32) -> Control {
    Control::Toggle(Toggle::Pref {
        id: p::START_FOLDERS,
        bit: 1 << index,
    })
}
/// System > Advanced (26H2 order: Taskbar, File Explorer, Virtual
/// Workspace, Terminal, For developers), plus the collapsed Administrative
/// tools that replace Windows Tools and the MMC snap-ins.
static SYSTEM_ADVANCED: [Block; 17] = [
    Block::Header("Taskbar"),
    Block::Card(row!(
        "End task",
        "Enable end task in taskbar by right-click",
        "taskbar",
        Control::Toggle(Toggle::Pref {
            id: p::END_TASK,
            bit: p::END_TASK_ON
        })
    )),
    Block::Header("File Explorer"),
    Block::Card(row!("Show file extensions", "", "folder", NA)),
    Block::Card(row!(
        "Run dialog",
        "Doors ships only the modern Run dialog",
        "code",
        ON
    )),
    Block::Card(row!(
        "NoneDrive",
        "Your own cloud folder, synced with a private server you run; shown in File Explorer",
        "cloud",
        Control::Toggle(Toggle::Pref {
            id: p::FEATURES,
            bit: door_service_protocol::preferences::features::NONEDRIVE,
        })
    )),
    Block::Header("Virtual Workspace"),
    Block::Card(row!(
        "Linux",
        "The Linux environment; its files appear in File Explorer",
        "code",
        Control::Toggle(Toggle::Pref {
            id: p::FEATURES,
            bit: door_service_protocol::preferences::features::LINUX,
        })
    )),
    Block::Card(row!(
        "Remote Desktop",
        "Remote Desktop users, connection permissions",
        "remote",
        link(Page::RemoteDesktop)
    )),
    Block::Header("Terminal"),
    Block::Card(row!(
        "Terminal",
        "The app that hosts command-line apps",
        "code",
        Control::Value(Fact::Static("Doors Terminal"))
    )),
    Block::Header("For developers"),
    Block::Card(row!(
        "Developer Mode",
        "Install apps from any source, including loose files",
        "code",
        OFF
    )),
    Block::Card(row!(
        "Developer tools",
        "Native interface version, Terminal and diagnostics",
        "settings",
        link(Page::DeveloperTools)
    )),
    Block::Header("Tools"),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Administrative tools",
            "Registry Editor, services, event logs and other tools for experienced users",
            "settings",
            Control::None
        ),
        hero: None,
        rows: rows![
            row!(
                "Registry Editor",
                "Browse and change registry keys and values",
                "",
                link(Page::Registry)
            ),
            row!(
                "Services",
                "Background services Doors runs",
                "",
                link(Page::Services)
            ),
            row!(
                "Environment variables",
                "Variables saved for your account",
                "",
                link(Page::Environment)
            ),
            row!(
                "Event logs",
                "Changes Settings made in this session",
                "",
                link(Page::EventViewer)
            ),
            row!(
                "Scheduled tasks",
                "Tasks that run at a set time",
                "",
                link(Page::TaskScheduler)
            ),
            row!(
                "Policies",
                "Group Policy and local security policy",
                "",
                link(Page::GroupPolicy)
            ),
            row!(
                "System configuration",
                "Startup selection, services and boot options",
                "",
                link(Page::SystemConfiguration)
            ),
        ],
    }),
    Block::Note(
        "These tools are for experienced users. Settings on other pages are safer to change.",
    ),
];
static SEARCH_PAGE: [Block; 6] = [
    Block::Card(row!(
        "Show suggested search results",
        "Doors search never queries the web",
        "globe",
        OFF
    )),
    Block::Header("History"),
    Block::Card(row!(
        "Search history on this device",
        "Improve search results by using your search history on this device",
        "history",
        Control::Toggle(Toggle::Pref {
            id: p::SEARCH_HISTORY,
            bit: p::SEARCH_HISTORY_ON
        })
    )),
    Block::Header("Searching Doors"),
    Block::Card(row!("Find files", "", "search", NA)),
    Block::Note("Start search finds installed apps and Settings pages on this device only."),
];
/// Time & language > Date & time (26H2): the clock header, then the rows.
static DATE_TIME: [Block; 9] = [
    Block::Hero(Hero::Clock),
    Block::Card(row!("Set time automatically", "", "history", OFF)),
    Block::Card(row!("Set time zone automatically", "", "world", OFF)),
    Block::Card(row!(
        "Time zone",
        "",
        "world",
        Control::Combo(Choice::TimeZone)
    )),
    Block::Card(row!(
        "Adjust for daylight saving time automatically",
        "Follows the time zone's daylight saving rules",
        "history",
        Control::Toggle(Toggle::Daylight)
    )),
    Block::Card(row!(
        "Show time and date in the System tray",
        "Always shown on this taskbar",
        "calendar",
        ON
    )),
    Block::Header("Additional settings"),
    Block::Card(row!("Sync now", "", "refresh", Control::Button("Sync now"))),
    Block::Card(row!(
        "Show additional calendars in the taskbar",
        "",
        "calendar",
        Control::Combo(Choice::Unavailable(&["Don't show additional calendars"], 0))
    )),
];
static LANGUAGE: [Block; 13] = [
    Block::Header("Language"),
    Block::Card(row!(
        "Doors display language",
        "Doors features like Settings and File Explorer appear in this language",
        "world",
        Control::Value(Fact::DisplayLanguage)
    )),
    Block::Card(row!(
        "Preferred languages",
        "Apps will appear in the first supported language in this list",
        "world",
        Control::Button("Add a language")
    )),
    Block::Header("Region"),
    Block::Card(row!(
        "Country or region",
        "Doors and apps might use your country or region to give you local content",
        "location",
        NA
    )),
    Block::Card(row!(
        "Regional format",
        "Doors and some apps format dates and times based on your regional format",
        "calendar",
        Control::Combo(Choice::RegionalFormat)
    )),
    Block::Card(row!(
        "First day of week",
        "",
        "calendar",
        Control::Combo(Choice::FirstDay)
    )),
    Block::Card(row!(
        "Time format",
        "12-hour or 24-hour clock",
        "history",
        Control::Combo(Choice::ClockFormat)
    )),
    Block::Card(row!(
        "Short date",
        "",
        "calendar",
        Control::Value(Fact::Region(1))
    )),
    Block::Card(row!(
        "Long date",
        "",
        "calendar",
        Control::Value(Fact::Region(2))
    )),
    Block::Card(row!(
        "Short time",
        "",
        "calendar",
        Control::Value(Fact::Region(3))
    )),
    Block::Card(row!(
        "Long time",
        "",
        "calendar",
        Control::Value(Fact::Region(4))
    )),
    Block::Card(row!(
        "Keyboard",
        "Installed keyboards and input methods",
        "keyboard",
        link(Page::Typing)
    )),
];
/// Time & language > Typing (Windows 11 26H2 Language & region keyboard
/// options): installed keyboards with add and remove, the default input
/// method and the switching hotkeys. No touch keyboard or suggestions.
static TYPING: [Block; 7] = [
    Block::Hero(Hero::Keyboards),
    Block::Header("Input method"),
    Block::Card(row!(
        "Override for default input method",
        "The keyboard or input method you type with after signing in",
        "keyboard",
        Control::Combo(Choice::InputMethod)
    )),
    Block::Card(row!(
        "Switch input methods",
        "Doors logo key + Space, left Alt + Shift, or Ctrl + Shift",
        "keyboard",
        Control::None
    )),
    Block::Header("Related settings"),
    Block::Card(row!("Touch keyboard", "", "touch", NA)),
    Block::Card(row!("Text suggestions", "", "edit", NA)),
];
static VISUAL: [Block; 5] = [
    Block::Card(row!(
        "Always show scrollbars",
        "Scroll bars are shown while content is scrollable",
        "list",
        OFF
    )),
    Block::Card(row!(
        "Transparency effects",
        "Windows and surfaces appear translucent",
        "effects",
        Control::Toggle(Toggle::inverted(Controls::REDUCE_TRANSPARENCY))
    )),
    Block::Card(row!(
        "Animation effects",
        "Windows, Start and menus animate when they open and close",
        "motion",
        Control::Toggle(Toggle::inverted(Controls::REDUCE_MOTION))
    )),
    Block::Card(row!(
        "Animation speed",
        "How fast windows, menus and Start animate",
        "motion",
        Control::Combo(Choice::AnimationSpeed)
    )),
    Block::Card(row!(
        "Dismiss notifications after this amount of time",
        "",
        "ringer",
        Control::Combo(Choice::Unavailable(&["5 seconds"], 0))
    )),
];
/// An open combo list or "More" menu. Like WinUI, a combo list opens over its
/// field with the current option aligned to the field; menus open below.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Popup {
    pub rect: Rect,
    pub count: usize,
    pub selected: usize,
    pub enabled: u64,
    /// First listed option: long lists (Time zone) scroll within the
    /// window, as WinUI combo lists do.
    pub first: usize,
}
/// Combo popup option row: 36 px grown with its 14 px label.
pub fn popup_row() -> i32 {
    crate::palette::text_grow(36, door_graphics_text::TextStyle::Regular14) as i32
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupKey {
    Moved,
    Chosen(usize),
    Closed,
    Ignored,
}
impl Popup {
    fn clamp(rect: Rect, top: i32, bottom: i32) -> Rect {
        let y = rect.y.min(bottom - rect.h as i32).max(top);
        Rect { y, ..rect }
    }
    /// Rows that fit between `top` and `bottom`.
    fn rows(count: usize, top: i32, bottom: i32) -> usize {
        count.min(((bottom - top - 8) / popup_row()).max(1) as usize)
    }
    /// Combo list over `field` (window coordinates), kept within [top, bottom).
    pub fn combo(
        field: Rect,
        count: usize,
        current: usize,
        enabled: u64,
        top: i32,
        bottom: i32,
    ) -> Self {
        let rows = Self::rows(count, top, bottom);
        let current = current.min(count.saturating_sub(1));
        // A list longer than the window opens scrolled to the current option.
        let first = current.saturating_sub(rows / 2).min(count - rows);
        let h = rows as u32 * popup_row() as u32 + 8;
        let y = field.y + (field.h as i32 - popup_row()) / 2
            - 4
            - (current - first) as i32 * popup_row();
        Self {
            rect: Self::clamp(Rect::new(field.x, y, field.w.max(120), h), top, bottom),
            count,
            selected: current,
            enabled,
            first,
        }
    }
    /// Options shown at once.
    pub fn visible(self) -> usize {
        ((self.rect.h.saturating_sub(8)) / popup_row() as u32) as usize
    }
    /// Scroll by `rows` (wheel: 3 per notch), within the list.
    pub fn scroll(&mut self, rows: i32) -> bool {
        let max = self.count.saturating_sub(self.visible());
        let next = (self.first as i64 + i64::from(rows)).clamp(0, max as i64) as usize;
        let moved = next != self.first;
        self.first = next;
        moved
    }
    fn reveal(&mut self, index: usize) {
        let visible = self.visible().max(1);
        if index < self.first {
            self.first = index;
        } else if index >= self.first + visible {
            self.first = index + 1 - visible;
        }
    }
    /// Menu below `button`, right aligned, `width` wide.
    pub fn menu(
        button: Rect,
        width: u32,
        count: usize,
        enabled: u64,
        top: i32,
        bottom: i32,
    ) -> Self {
        let h = Self::rows(count, top, bottom) as u32 * popup_row() as u32 + 8;
        let mut y = button.bottom() + 4;
        if y + h as i32 > bottom {
            y = button.y - 4 - h as i32;
        }
        let selected = (0..count).find(|i| enabled & (1 << i) != 0).unwrap_or(0);
        Self {
            rect: Self::clamp(
                Rect::new(button.right() - width as i32, y, width, h),
                top,
                bottom,
            ),
            count,
            selected,
            enabled,
            first: 0,
        }
    }
    pub fn enabled(self, index: usize) -> bool {
        index < 64 && self.enabled & (1 << index) != 0
    }
    pub fn option(self, index: usize) -> Rect {
        Rect::new(
            self.rect.x + 4,
            self.rect.y + 4 + (index as i32 - self.first as i32) * popup_row(),
            self.rect.w.saturating_sub(8),
            popup_row() as u32,
        )
    }
    /// Option under the pointer (enabled or not); None outside the list.
    pub fn hit(self, x: i32, y: i32) -> Option<usize> {
        (self.first..self.count.min(self.first + self.visible()))
            .find(|i| self.option(*i).contains(x, y))
    }
    pub fn key(&mut self, scan: u32) -> PopupKey {
        match scan {
            0x148 | 0x150 => {
                let mut index = self.selected;
                loop {
                    let next = if scan == 0x148 {
                        index.checked_sub(1)
                    } else {
                        Some(index + 1).filter(|i| *i < self.count)
                    };
                    let Some(next) = next else {
                        return PopupKey::Ignored;
                    };
                    index = next;
                    if self.enabled(index) {
                        self.selected = index;
                        self.reveal(index);
                        return PopupKey::Moved;
                    }
                }
            }
            28 | 57 | 0x11c if self.enabled(self.selected) => PopupKey::Chosen(self.selected),
            1 | 15 => PopupKey::Closed,
            _ => PopupKey::Ignored,
        }
    }
}
/// About: the "Rename this PC" button inside the device name card.
pub fn about_rename_button(r: Rect) -> Rect {
    let w = 130u32.min(r.w / 3);
    Rect::new(
        r.x + r.w as i32 - CARD_RIGHT - w as i32,
        r.y + r.h as i32 / 2 - 16,
        w,
        32,
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    /// Every row control on every page, expanders included.
    fn all_controls() -> Vec<Control> {
        let mut out = Vec::new();
        for page in Page::ALL {
            for block in blocks_for(page) {
                match block {
                    Block::Card(row) => out.push(row.control),
                    Block::Group(g) => out.extend(g.rows.iter().map(|r| r.control)),
                    Block::Expander(e) => {
                        out.push(e.header.control);
                        out.extend(e.rows.iter().map(|r| r.control));
                    }
                    _ => (),
                }
            }
        }
        out
    }
    #[test]
    fn preference_controls_store_only_valid_words_and_round_trip() {
        assert!(
            p::ALL
                .iter()
                .enumerate()
                .all(|(i, w)| w.id == PREF_FIRST + i as u32)
        );
        let state = State::new(Controls::DEFAULT, 0);
        let mut toggles = 0;
        let mut choices = 0;
        for control in all_controls() {
            match control {
                Control::Toggle(t @ Toggle::Pref { id, .. }) => {
                    toggles += 1;
                    let Some(Change::Preference(cid, value)) = t.flip(state) else {
                        panic!("{t:?} must change its word");
                    };
                    assert_eq!(cid, id);
                    assert!(p::valid(id, value), "{t:?} -> {value:#x}");
                    let mut after = state;
                    after.prefs[(id - PREF_FIRST) as usize] = value;
                    assert_ne!(t.value(after), t.value(state), "{t:?} flips");
                }
                Control::Combo(c) if c.enabled() => {
                    for index in 0..c.count(state) {
                        match c.apply(index, state) {
                            Some(Change::Preference(id, value)) => {
                                choices += 1;
                                assert!(p::valid(id, value), "{c:?}[{index}] -> {value:#x}");
                                let mut after = state;
                                after.prefs[(id - PREF_FIRST) as usize] = value;
                                assert_eq!(c.current(after), index, "{c:?} round trip");
                            }
                            Some(Change::Accent(bits)) => {
                                let mut after = state;
                                after.accent = bits;
                                assert_eq!(c.current(after), index, "{c:?} round trip");
                            }
                            _ => (),
                        }
                    }
                }
                _ => (),
            }
        }
        assert!(
            toggles >= 15 && choices >= 40,
            "{toggles} toggles, {choices} options"
        );
        // Five Recent colors slots, then the 48 Windows colors.
        assert_eq!(hero_count(Hero::Accents), RECENT_ACCENTS + 48);
        let r = Rect::new(0, 0, 640, 0);
        assert_eq!(gallery(Hero::Accents, 640).0, 9, "nine swatches to a row");
        let last = tile(Hero::Accents, r, RECENT_ACCENTS + 47, 640).unwrap();
        assert!(
            last.y + last.h as i32
                <= hero_height(
                    Hero::Accents,
                    &Context {
                        width: 640,
                        open: 0,
                        apps: 0,
                        keyboards: 0,
                        lists: [0; List::COUNT],
                        vpn_up: 0,
                        background: 0,
                        recent_accents: 1,
                    },
                ) as i32
        );
    }
    #[test]
    fn combo_list_aligns_the_current_option_and_skips_disabled_options() {
        let field = Rect::new(800, 400, 160, 32);
        let mut p = Popup::combo(field, 3, 1, 0b011, 48, 787);
        let current = p.option(1);
        assert_eq!(current.center_y(), field.center_y());
        assert_eq!(p.hit(current.x + 4, current.y + 4), Some(1));
        assert_eq!(p.key(0x150), PopupKey::Ignored, "Custom is disabled");
        assert_eq!(p.key(0x148), PopupKey::Moved);
        assert_eq!(p.key(28), PopupKey::Chosen(0));
        assert_eq!(p.key(1), PopupKey::Closed);
        // Clamped inside the window, never above the title bar.
        let p = Popup::combo(Rect::new(800, 60, 160, 32), 4, 3, 0b1111, 48, 787);
        assert!(p.rect.y >= 48);
        let p = Popup::combo(Rect::new(800, 770, 160, 32), 4, 0, 0b1111, 48, 787);
        assert!(p.rect.bottom() <= 787);
        let m = Popup::menu(Rect::new(1000, 300, 32, 32), 200, 3, 0b001, 48, 787);
        assert_eq!(m.rect.right(), 1032);
        assert!(m.rect.y >= 336);
        assert_eq!(m.selected, 0);
        assert!(!m.enabled(2));
    }
    fn state(bits: u32, appearance: u32) -> State {
        State::new(Controls::from_bits(bits).unwrap(), appearance)
    }
    fn ctx(width: u32) -> Context {
        Context {
            width,
            open: u64::MAX,
            apps: 6,
            keyboards: 6,
            lists: [3; List::COUNT],
            vpn_up: 0,
            background: 0,
            recent_accents: 1,
        }
    }
    #[test]
    fn vpn_connections_are_expanders_with_windows_buttons() {
        let mut lists = [0; List::COUNT];
        lists[List::VpnConnections as usize] = 2;
        // Connection 1 is open and up; Related support starts expanded.
        let ctx = Context {
            width: 900,
            open: 1 << (VPN_OPEN + 1),
            apps: 0,
            keyboards: 0,
            lists,
            vpn_up: 0b10,
            background: 0,
            recent_accents: 1,
        };
        let l = layout(Page::Vpn, &ctx);
        let rect = |a: Action| l.items.iter().find(|i| i.action == a).map(|i| i.rect);
        let list = List::VpnConnections;
        let (c0, c1) = (
            rect(Action::ListEdit(list, 0)).unwrap(),
            rect(Action::ListEdit(list, 1)).unwrap(),
        );
        // Connect 90 px, Disconnect 104, both ending at the chevron column.
        assert_eq!((c0.w, c1.w), (90, 104));
        assert_eq!(c0.right(), c1.right());
        let header = rect(Action::Expand(VPN_OPEN)).unwrap();
        assert_eq!(header.right() - c0.right(), VPN_BUTTON_RIGHT);
        // Only the open connection shows Advanced options and Remove, below
        // its header.
        assert!(
            rect(Action::ListOpen(list, 0)).is_none()
                && rect(Action::ListRemove(list, 0)).is_none()
        );
        let (options, remove) = (
            rect(Action::ListOpen(list, 1)).unwrap(),
            rect(Action::ListRemove(list, 1)).unwrap(),
        );
        assert!(
            options.right() < remove.x && remove.right() == c1.right() && options.y > c1.bottom()
        );
        // Add VPN is the accent primary action; the two settings are toggles.
        assert!(Command::AddVpn.accent());
        assert_eq!(
            l.items
                .iter()
                .filter(|i| matches!(i.action, Action::Toggle(Toggle::PrefOff { id: p::VPN, .. })))
                .count(),
            2
        );
        // No web help links (left out on purpose): the toggles end the page.
        let last = l.items.iter().map(|i| i.rect.bottom()).max().unwrap();
        assert!(
            l.items
                .iter()
                .filter(|i| i.rect.bottom() == last)
                .all(|i| matches!(i.action, Action::Toggle(_)))
        );
        // No connections: nothing between the header card and the next section.
        let empty = layout(
            Page::Vpn,
            &Context {
                lists: [0; List::COUNT],
                open: 0,
                ..ctx
            },
        );
        assert!(
            !empty
                .items
                .iter()
                .any(|i| matches!(i.action, Action::ListEdit(..)))
        );
    }
    #[test]
    fn microphone_app_switches_follow_microphone_access() {
        use door_service_protocol::audio::capture::{ACCESS_OFF, DESKTOP_OFF};
        let mut s = State::new(Controls::from_bits(48).unwrap(), 0);
        let access = Toggle::PrefOff {
            id: p::MICROPHONE,
            bit: ACCESS_OFF,
        };
        let desktop = Toggle::PrefOff {
            id: p::MICROPHONE,
            bit: DESKTOP_OFF,
        };
        assert!(access.value(s) && desktop.value(s) && desktop.live(s));
        assert_eq!(
            desktop.flip(s),
            Some(Change::Preference(p::MICROPHONE, DESKTOP_OFF))
        );
        s.prefs[(p::MICROPHONE - PREF_FIRST) as usize] = ACCESS_OFF;
        // Access off: the app switches show Off, greyed, and do nothing.
        assert!(!access.value(s) && access.live(s));
        assert!(!desktop.value(s) && !desktop.live(s) && desktop.flip(s).is_none());
    }
    #[test]
    fn typing_keyboards_add_move_remove_and_keep_one() {
        let base = State::new(Controls::from_bits(48).unwrap(), 0);
        assert_eq!(Choice::InputMethod.count(base), 1);
        assert_eq!(
            Choice::InputMethod.label(0, base),
            "English (United States) - US"
        );
        assert_eq!(keyboard_menu_mask(base, 0), 0, "the last keyboard stays");
        assert_eq!(addable(base).count(), 8);
        // Add Pinyin (the sixth addable method) and Korean.
        let Some(Change::InputMethods(bits)) = keyboard_add_apply(base, 5) else {
            panic!()
        };
        let s = State {
            input: bits,
            ..base
        };
        assert_eq!(s.input_methods().ids().collect::<Vec<_>>(), [1, 7]);
        let Some(Change::InputMethods(bits)) = keyboard_add_apply(s, 6) else {
            panic!()
        };
        let s = State { input: bits, ..s };
        assert_eq!(s.input_methods().ids().collect::<Vec<_>>(), [1, 7, 9]);
        // Default input method: Pinyin.
        let Some(Change::InputMethods(bits)) = Choice::InputMethod.apply(1, s) else {
            panic!()
        };
        let s = State { input: bits, ..s };
        assert_eq!(s.input_methods().current().id, 7);
        assert_eq!(keyboard_menu_mask(s, 0), 0b110);
        assert_eq!(keyboard_menu_mask(s, 2), 0b101);
        // Moving keeps Pinyin current; removing it selects the first.
        let Some(Change::InputMethods(bits)) = keyboard_menu_apply(s, 1, 0) else {
            panic!()
        };
        let moved = State { input: bits, ..s };
        assert_eq!(moved.input_methods().ids().collect::<Vec<_>>(), [7, 1, 9]);
        assert_eq!(moved.input_methods().current().id, 7);
        let Some(Change::InputMethods(bits)) = keyboard_menu_apply(moved, 0, 2) else {
            panic!()
        };
        let removed = State {
            input: bits,
            ..moved
        };
        assert_eq!(removed.input_methods().ids().collect::<Vec<_>>(), [1, 9]);
        assert_eq!(removed.input_methods().current().id, 1);
        assert_eq!(
            keyboard_menu_apply(removed, 0, 0),
            None,
            "first cannot move up"
        );
        // Labels match the method catalog; statuses are honest.
        for m in METHODS {
            assert_eq!(
                input_label(m.id),
                alloc::format!("{} - {}", m.language, m.keyboard)
            );
        }
        let pinyin = &METHODS[6];
        assert!(keyboard_status(s, pinyin).starts_with("Unavailable"));
        let missing = door_service_protocol::text_input::STATE_IME | 1 << (16 + 7);
        assert!(
            keyboard_status(State { ime: missing, ..s }, pinyin)
                .contains("dictionary not installed")
        );
        assert_eq!(
            keyboard_status(State { ime: 1, ..s }, pinyin),
            "Input method editor"
        );
        assert_eq!(keyboard_status(s, &METHODS[0]), "Keyboard layout");
    }
    #[test]
    fn placement_never_overlaps_and_every_item_lies_in_its_block() {
        for page in Page::ALL {
            for width in [300, 480, 560, 797, 1000] {
                let l = layout(page, &ctx(width));
                let mut prior = TOP;
                for b in &l.blocks {
                    assert!(b.rect.y >= prior, "{page:?} {width} overlap");
                    assert!(b.rect.right() <= X + width as i32);
                    for (_, r) in &b.rows {
                        assert!(r.y >= b.rect.y && r.bottom() <= b.rect.bottom());
                    }
                    prior = b.rect.bottom();
                }
                assert!(l.bottom as i32 >= prior);
                for item in &l.items {
                    assert!(item.rect.x >= X - 52 && item.rect.right() <= X + width as i32 + 4);
                    assert!(
                        l.blocks.iter().any(|b| b
                            .rect
                            .contains(item.rect.x + item.rect.w as i32 / 2, item.rect.center_y())),
                        "{page:?} {width} {item:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn unavailable_providers_never_become_targets() {
        for page in Page::ALL {
            let l = layout(page, &ctx(797));
            for b in &l.blocks {
                for (row, r) in &b.rows {
                    let inside = l.items.iter().any(|i| {
                        r.contains(i.rect.x + i.rect.w as i32 / 2, i.rect.center_y())
                            && !matches!(i.action, Action::Expand(_))
                    });
                    if row.description.starts_with("Unavailable") {
                        assert!(!row.control.enabled(), "{page:?} {}", row.title);
                        assert!(!inside, "{page:?} {}", row.title);
                    }
                    assert_eq!(inside, row.control.enabled(), "{page:?} {}", row.title);
                }
            }
        }
    }
    #[test]
    fn bound_controls_round_trip_their_exact_preference_bits() {
        let base = state(48, 0);
        // Transparency effects On means the reduce bit is clear.
        let t = Toggle::inverted(Controls::REDUCE_TRANSPARENCY);
        assert!(t.value(base));
        let Some(Change::Controls(c)) = t.flip(base) else {
            panic!()
        };
        assert_eq!(c.bits(), 48 | Controls::REDUCE_TRANSPARENCY);
        assert!(!t.value(State {
            controls: c,
            ..base
        }));
        let m = Toggle::inverted(Controls::REDUCE_MOTION);
        assert_eq!(
            m.flip(base),
            Some(Change::Controls(
                base.controls.with(Controls::REDUCE_MOTION, true)
            ))
        );
        let icons = Toggle::control(Controls::SHOW_DESKTOP_ICONS);
        assert!(!icons.value(base));
        assert_eq!(
            icons.flip(base),
            Some(Change::Controls(Controls::from_bits(48 | 64).unwrap()))
        );
        let classic = Toggle::control(Controls::CLASSIC_CONTEXT_MENU);
        assert_eq!(
            classic.flip(base),
            Some(Change::Controls(Controls::from_bits(48 | 1024).unwrap()))
        );
        assert_eq!(Toggle::Unavailable(true).flip(base), None);
        // 26H2 Start size uses bits 13..14 and never produces the invalid 3.
        for index in 0..3 {
            let Some(Change::Controls(c)) = Choice::StartSize.apply(index, base) else {
                panic!()
            };
            assert_eq!(c.start_size(), index as u32);
            assert_eq!(
                Choice::StartSize.current(State {
                    controls: c,
                    ..base
                }),
                index
            );
        }
        // Start sections: On means the hide bit is clear (Windows default 0).
        for flag in [
            Controls::START_HIDE_PINNED,
            Controls::START_HIDE_RECENT,
            Controls::START_HIDE_ALL,
            Controls::START_HIDE_ACCOUNT,
            Controls::START_HIDE_RECENT_FILES,
            Controls::CONTEXT_HIDE_FILE_ACTIONS,
            Controls::CONTEXT_HIDE_MORE_OPTIONS,
        ] {
            let t = Toggle::inverted(flag);
            assert!(t.value(base));
            assert_eq!(
                t.flip(base),
                Some(Change::Controls(base.controls.with(flag, true)))
            );
        }
        // Taskbar position, size and Properties placement.
        for edge in 0..4 {
            let Some(Change::Controls(c)) = Choice::Position.apply(edge, base) else {
                panic!()
            };
            assert_eq!(c.edge() as usize, edge);
            assert_eq!(c.bits() & !3, 48);
        }
        assert_eq!(Choice::TaskbarSize.current(base), 1, "Default");
        let Some(Change::TaskbarSize(c, word)) = Choice::TaskbarSize.apply(0, base) else {
            panic!()
        };
        assert!(c.small() && word & p::taskbar::LARGE == 0);
        // Large clears Small and sets the preference bit; Default clears both.
        let Some(Change::TaskbarSize(c, word)) = Choice::TaskbarSize.apply(2, base) else {
            panic!()
        };
        assert!(!c.small() && word & p::taskbar::LARGE != 0);
        let Some(Change::TaskbarSize(c, word)) = Choice::TaskbarSize.apply(1, base) else {
            panic!()
        };
        assert!(!c.small() && word & p::taskbar::LARGE == 0);
        assert_eq!(
            Choice::PropertiesPlacement.current(base),
            1,
            "Bottom by default"
        );
        let Some(Change::Controls(c)) = Choice::PropertiesPlacement.apply(0, base) else {
            panic!()
        };
        assert!(!c.properties_bottom());
        // Alternate taskbar positions force the search icon (26H2).
        let top = state(48 | 1, 0);
        assert_eq!(Choice::Search.current(top), 1);
        assert_eq!(Choice::Search.apply(3, top), None);
        assert_eq!(Choice::Search.step(top, true), None);
        assert_eq!(Choice::Search.mask(top), 0b0010);
        // Search: only "icon only" and "box" have a provider.
        assert_eq!(Choice::Search.current(base), 3);
        assert_eq!(Choice::Search.apply(0, base), None);
        assert_eq!(Choice::Search.apply(2, base), None);
        let Some(Change::Controls(c)) = Choice::Search.apply(1, base) else {
            panic!()
        };
        assert!(c.search_icon_only());
        assert_eq!(
            Choice::Search.step(
                State {
                    controls: c,
                    ..base
                },
                true
            ),
            Some(3)
        );
        assert_eq!(Choice::Search.step(base, false), Some(1));
        assert_eq!(Choice::Search.step(base, true), None);
        // Mode keeps the background bits; Custom is listed but disabled.
        assert_eq!(
            Choice::Mode.apply(1, state(48, 6)),
            Some(Change::Appearance(7))
        );
        assert_eq!(
            Choice::Mode.apply(0, state(48, 7)),
            Some(Change::Appearance(6))
        );
        assert_eq!(Choice::Mode.apply(2, base), None);
        assert_eq!(Choice::Mode.step(base, true), Some(1));
        assert_eq!(Choice::Mode.step(state(48, 1), true), None);
        // Alignment: Left sets the start-aligned bit; vertical edges say Top.
        let Some(Change::Controls(c)) = Choice::Alignment.apply(0, base) else {
            panic!()
        };
        assert!(c.start_aligned());
        assert_eq!(Choice::Alignment.current(base), 1);
        assert_eq!(Choice::Alignment.label(0, state(48 | 2, 0)), "Top");
        assert_eq!(Choice::Alignment.label(0, base), "Left");
        assert_eq!(Choice::Unavailable(&["A"], 0).apply(0, base), None);
    }
    #[test]
    fn galleries_and_expanders_share_paint_and_hit_geometry() {
        let l = layout(
            Page::Taskbar,
            &Context {
                width: 797,
                open: 0,
                apps: 0,
                keyboards: 0,
                lists: [0; List::COUNT],
                vpn_up: 0,
                background: 0,
                recent_accents: 1,
            },
        );
        // Closed expanders expose only their headers.
        assert!(
            l.items
                .iter()
                .all(|i| matches!(i.action, Action::Expand(_) | Action::Navigate(_)))
        );
        assert_eq!(
            l.items
                .iter()
                .filter(|i| matches!(i.action, Action::Expand(_)))
                .count(),
            4
        );
        // Taskbar behaviors (26H2): alignment, position and size combos,
        // combine; then the Doors extras' taskbar look.
        let l = layout(Page::Taskbar, &ctx(797));
        let combos: Vec<_> = l
            .items
            .iter()
            .filter_map(|i| match i.action {
                Action::Combo(c) => Some(c),
                _ => None,
            })
            .collect();
        assert_eq!(
            combos,
            [
                Choice::Search,
                Choice::Alignment,
                Choice::Position,
                Choice::TaskbarSize,
                Choice::Combine,
                Choice::TaskbarLook
            ]
        );
        for width in [300, 560, 797, 1000] {
            let l = layout(Page::Personalization, &ctx(width));
            let tiles: Vec<_> = l
                .items
                .iter()
                .filter(|i| matches!(i.action, Action::Theme(_)))
                .collect();
            assert_eq!(tiles.len(), THEMES.len());
            let hero = l.blocks[0].rect;
            for t in &tiles {
                assert!(t.rect.y >= hero.y && t.rect.bottom() <= hero.bottom());
                assert!(t.rect.right() <= hero.right());
            }
        }
        let apps = layout(Page::InstalledApps, &ctx(797));
        let menus: Vec<_> = apps
            .items
            .iter()
            .filter(|i| matches!(i.action, Action::AppMenu(_)))
            .collect();
        assert_eq!(menus.len(), 6);
        assert_eq!(default_open(Page::Taskbar), 1 << 1);
    }
    #[test]
    fn narrow_cards_wrap_fields_below_their_text() {
        let row = Row {
            title: "Choose your mode",
            description: "",
            glyph: "color",
            control: Control::Combo(Choice::Mode),
        };
        let wide = row_geometry(&row, Rect::new(X, 200, 797, CARD), 797);
        let f = wide.field.unwrap();
        assert_eq!(f.right(), X + 797 - CARD_RIGHT);
        assert!(wide.text_right <= f.x);
        let narrow_h = row_height(&row, 400, CARD);
        let narrow = row_geometry(&row, Rect::new(X, 200, 400, narrow_h), 400);
        let f = narrow.field.unwrap();
        assert!(f.y >= 200 + 56 && f.bottom() <= 200 + narrow_h as i32);
        assert!(f.right() <= X + 400);
    }
    #[test]
    fn time_zone_and_regional_format_choices_store_stable_ids() {
        let base = State::new(Controls::DEFAULT, 0);
        // Defaults: UTC and English (United States); DST shown off for UTC.
        assert_eq!(
            Choice::TimeZone.label(Choice::TimeZone.current(base), base),
            "(UTC) Coordinated Universal Time"
        );
        assert_eq!(
            Choice::RegionalFormat.label(Choice::RegionalFormat.current(base), base),
            "English (United States)"
        );
        assert!(!Toggle::Daylight.value(base) && !Toggle::Daylight.enabled());
        assert_eq!(Toggle::Daylight.flip(base), None);
        assert_eq!(Choice::TimeZone.count(base), door_locale::tz::ZONES.len());
        let pacific = (0..Choice::TimeZone.count(base))
            .find(|i| Choice::TimeZone.label(*i, base) == "(UTC-08:00) Pacific Time (US & Canada)")
            .unwrap();
        assert_eq!(
            Choice::TimeZone.apply(pacific, base),
            Some(Change::TimeZone(3))
        );
        let la = State {
            zone: 3,
            region: 3,
            ..base
        };
        assert_eq!(Choice::TimeZone.current(la), pacific);
        assert!(Toggle::Daylight.value(la));
        assert_eq!(
            Choice::RegionalFormat.label(Choice::RegionalFormat.current(la), la),
            "German (Germany)"
        );
        assert_eq!(
            Choice::RegionalFormat.apply(6, la),
            Some(Change::RegionalFormat(7))
        );
        assert_eq!(Choice::RegionalFormat.apply(7, la), None);
        // An unknown stored zone shows UTC selected.
        let unknown = State { zone: 999, ..base };
        assert_eq!(
            Choice::TimeZone.current(unknown),
            Choice::TimeZone.current(base)
        );
    }
    #[test]
    fn cards_and_sections_follow_the_measured_26h2_rhythm() {
        // docs/design/fidelity-26h2-diff.md G5/G6, System > Display at 150 %:
        // cards 104 px (69.3 epx) at a 110 px (73.3) pitch, text at 89 px
        // (59.3), a header 40 px (26.7) above its first card and 47 px (31.3)
        // below the previous card.
        assert_eq!(CARD, 69);
        assert_eq!(CARD as i32 + CARD_GAP, 73);
        assert_eq!(
            (CARD_ICON, CARD_TEXT, CARD_RIGHT, CARD_CHEVRON),
            (19, 59, 17, 25)
        );
        let l = layout(Page::Display, &ctx(797));
        let headers: Vec<_> = l
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| matches!(b.block, Block::Header(_)))
            .map(|(i, _)| i)
            .collect();
        for &i in &headers {
            let header = l.blocks[i].rect;
            let next = l.blocks[i + 1].rect;
            assert_eq!(next.y - header.y, HEADER_TO_CARD as i32, "header to card");
            if i > 0 && matches!(l.blocks[i - 1].block, Block::Card(_)) {
                assert_eq!(header.y - l.blocks[i - 1].rect.bottom(), CARD_TO_HEADER);
            }
        }
        // Consecutive cards: 69 tall at a 73 pitch.
        let cards: Vec<_> = l
            .blocks
            .iter()
            .filter(|b| matches!(b.block, Block::Card(_)))
            .map(|b| b.rect)
            .collect();
        assert!(cards.iter().all(|r| r.h == CARD));
        assert!(
            cards
                .windows(2)
                .any(|w| w[1].y - w[0].y == CARD as i32 + CARD_GAP)
        );
        // Controls end 17 px from the card's right edge; the chevron button's
        // centre is 25 px from it.
        let toggle = Row {
            title: "t",
            description: "",
            glyph: "",
            control: Control::Toggle(Toggle::control(1)),
        };
        let r = Rect::new(X, 200, 797, CARD);
        assert_eq!(
            row_geometry(&toggle, r, 797).toggle.unwrap().right(),
            r.right() - 17
        );
        let link = Row {
            control: Control::Link(Page::Display),
            ..toggle
        };
        assert_eq!(
            row_geometry(&link, r, 797).chevron.unwrap().0,
            r.right() - 25
        );
    }
    /// docs/design/fidelity-26h2-diff.md G15/G12: About's hardware tiles,
    /// name card, specifications with Related links; Date & time's clock.
    #[test]
    fn about_and_date_time_follow_the_26h2_layout() {
        let l = layout(Page::About, &ctx(1000));
        let tiles = about_tiles(l.blocks[0].hero.unwrap().1);
        assert_eq!(tiles.len(), 4);
        assert_eq!((tiles[0].w, tiles[0].h), (200, 142));
        assert_eq!(tiles[1].x - tiles[0].x, 212);
        assert!(
            tiles.iter().all(|t| t.y == tiles[0].y),
            "one row at 1000 px"
        );
        assert_eq!(l.blocks[1].rect.y - l.blocks[0].rect.bottom(), 54);
        assert_eq!(l.blocks[1].rect.h, card());
        assert_eq!(l.blocks[2].rect.y - l.blocks[1].rect.bottom(), CARD_GAP);
        assert_eq!(l.blocks[3].rect.y - l.blocks[2].rect.bottom(), CARD_GAP);
        // Related links: System protection and Advanced system settings act;
        // Domain or workgroup has no provider and no target.
        let links: Vec<_> = l
            .items
            .iter()
            .filter_map(|i| match i.action {
                Action::Navigate(p) => Some(p),
                _ => None,
            })
            .collect();
        assert_eq!(links, [Page::BackupRestore, Page::SystemAdvanced]);
        // Narrow: the tiles wrap two by two.
        let narrow = layout(Page::About, &ctx(480));
        let tiles = about_tiles(narrow.blocks[0].hero.unwrap().1);
        assert_eq!((tiles[2].x, tiles[2].y - tiles[0].y), (tiles[0].x, 154));
        assert_eq!(narrow.blocks[0].rect.h, 2 * 142 + 12);
        // Date & time opens with the clock header.
        assert_eq!(blocks_for(Page::DateTime)[0], Block::Hero(Hero::Clock));
        assert_eq!(hero_height(Hero::Clock, &ctx(797)), CLOCK_HEIGHT);
    }
    #[test]
    fn long_combo_lists_scroll_within_the_window() {
        let field = Rect::new(300, 400, 420, 32);
        let mut p = Popup::combo(field, 49, 40, (1u64 << 49) - 1, 48, 787);
        let rows = p.visible();
        assert!(rows < 49 && p.rect.bottom() <= 787 && p.rect.y >= 48);
        assert!(p.first <= 40 && 40 < p.first + rows);
        // The current option is under the pointer row and hit-testable.
        let o = p.option(40);
        assert_eq!(p.hit(o.x + 4, o.y + 4), Some(40));
        // Rows scrolled out are not hit; the wheel scrolls within bounds.
        assert!(p.scroll(-100));
        assert_eq!(p.first, 0);
        assert!(!p.scroll(-3));
        assert!(p.scroll(1000));
        assert_eq!(p.first, 49 - rows);
        // Keyboard selection keeps the selected option visible.
        p.first = 0;
        p.selected = 0;
        for _ in 0..rows + 2 {
            assert_eq!(p.key(0x150), PopupKey::Moved);
        }
        assert!(p.first > 0 && p.selected < p.first + rows);
        // Short lists are unchanged.
        let short = Popup::combo(field, 3, 1, 0b111, 48, 787);
        assert_eq!((short.first, short.visible()), (0, 3));
    }
}
