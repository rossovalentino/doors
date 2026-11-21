//! Start search over real, locally available targets only: installed apps and
//! Settings pages that the Settings app can open directly. No web results.
extern crate alloc;
use crate::apps::{App, Tool};
use alloc::vec::Vec;
/// A Settings page the Settings app accepts through SETTINGS_NAVIGATE. The
/// index is the page's position in Settings' `Page::ALL` route table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingsPage {
    pub page: u32,
    pub title: &'static str,
    pub keywords: &'static [&'static str],
}
/// Settings route indices (desktop/workspace/src/settings_layout.rs `Page::ALL`).
pub mod page {
    pub const HOME: u32 = 0;
    pub const SYSTEM: u32 = 1;
    pub const BLUETOOTH: u32 = 2;
    pub const NETWORK: u32 = 3;
    pub const PERSONALIZATION: u32 = 4;
    pub const APPS: u32 = 5;
    pub const ACCOUNTS: u32 = 6;
    pub const TIME: u32 = 7;
    pub const GAMING: u32 = 8;
    pub const ACCESSIBILITY: u32 = 9;
    pub const PRIVACY: u32 = 10;
    pub const UPDATE: u32 = 11;
    pub const ABOUT: u32 = 12;
    pub const DISPLAY: u32 = 13;
    pub const INSTALLED_APPS: u32 = 14;
    pub const TASKBAR: u32 = 15;
    pub const CONTEXT_MENU: u32 = 16;
    pub const REGISTRY: u32 = 17;
    pub const DEVICES_DRIVERS: u32 = 18;
    pub const DISK_MANAGEMENT: u32 = 19;
    pub const NETWORK_ADMINISTRATION: u32 = 20;
    pub const SERVICES: u32 = 21;
    pub const ENVIRONMENT: u32 = 22;
    pub const SYSTEM_CONFIGURATION: u32 = 23;
    pub const GROUP_POLICY: u32 = 24;
    pub const EVENT_VIEWER: u32 = 25;
    pub const TASK_SCHEDULER: u32 = 26;
    pub const USERS_GROUPS: u32 = 27;
    pub const PERMISSIONS_SHARING: u32 = 28;
    pub const PERFORMANCE: u32 = 29;
    pub const POWER_MANAGEMENT: u32 = 30;
    pub const BACKUP_RESTORE: u32 = 31;
    pub const DEVELOPER_TOOLS: u32 = 32;
    pub const SOUND: u32 = 33;
    pub const NOTIFICATIONS: u32 = 34;
    pub const POWER: u32 = 35;
    pub const STORAGE: u32 = 36;
    pub const BACKGROUND: u32 = 37;
    pub const COLORS: u32 = 38;
    pub const THEMES: u32 = 39;
    pub const START: u32 = 40;
    pub const DATE_TIME: u32 = 41;
    pub const LANGUAGE_REGION: u32 = 42;
    pub const VISUAL_EFFECTS: u32 = 43;
    pub const SYSTEM_ADVANCED: u32 = 44;
    pub const SEARCH_PERMISSIONS: u32 = 45;
    /// Number of routes Settings accepts (exclusive upper bound).
    pub const COUNT: u32 = 46;
}
macro_rules! pages {
    ($(($page:expr, $title:literal, [$($k:literal),*])),* $(,)?) => {
        &[$(SettingsPage { page: $page, title: $title, keywords: &[$($k),*] }),*]
    };
}
pub const SETTINGS: &[SettingsPage] = pages![
    (page::SYSTEM, "System", ["system"]),
    (
        page::SYSTEM_ADVANCED,
        "Advanced system settings",
        ["advanced", "advanced system", "for developers", "end task"]
    ),
    (
        page::SOUND,
        "Sound settings",
        ["sound", "volume", "speakers", "audio", "microphone"]
    ),
    (
        page::NOTIFICATIONS,
        "Notifications",
        ["notifications", "do not disturb", "focus"]
    ),
    (
        page::POWER,
        "Power & battery",
        [
            "power",
            "battery",
            "sleep",
            "energy saver",
            "power mode",
            "screen timeout"
        ]
    ),
    (
        page::STORAGE,
        "Storage",
        ["storage", "disk space", "free space", "cleanup"]
    ),
    (
        page::DISPLAY,
        "Display settings",
        [
            "display",
            "resolution",
            "scale",
            "screen",
            "monitor",
            "brightness",
            "night light"
        ]
    ),
    (
        page::ABOUT,
        "About your PC",
        [
            "about",
            "device specifications",
            "pc name",
            "version",
            "system information"
        ]
    ),
    (
        page::BLUETOOTH,
        "Bluetooth & devices",
        [
            "bluetooth",
            "devices",
            "mouse",
            "keyboard",
            "printers",
            "touchpad"
        ]
    ),
    (
        page::NETWORK,
        "Network & internet",
        [
            "network",
            "wi-fi",
            "wifi",
            "ethernet",
            "internet",
            "vpn",
            "proxy",
            "airplane mode"
        ]
    ),
    (
        page::PERSONALIZATION,
        "Personalization",
        ["personalize", "customize"]
    ),
    (
        page::BACKGROUND,
        "Background",
        ["background", "wallpaper", "desktop background", "picture"]
    ),
    (
        page::COLORS,
        "Colors",
        [
            "colors",
            "dark mode",
            "light mode",
            "accent color",
            "transparency effects",
            "theme color"
        ]
    ),
    (
        page::THEMES,
        "Themes",
        ["themes", "desktop icons", "desktop icon settings", "cursor"]
    ),
    (
        page::START,
        "Start settings",
        [
            "start",
            "start menu",
            "start layout",
            "recommended",
            "pinned"
        ]
    ),
    (
        page::TASKBAR,
        "Taskbar settings",
        [
            "taskbar",
            "taskbar alignment",
            "search box",
            "start menu",
            "start layout",
            "task view",
            "small taskbar"
        ]
    ),
    (
        page::CONTEXT_MENU,
        "Context menu settings",
        [
            "context menu",
            "right click",
            "show more options",
            "classic menu"
        ]
    ),
    (page::APPS, "Apps", ["apps", "default apps", "startup apps"]),
    (
        page::INSTALLED_APPS,
        "Installed apps",
        [
            "installed apps",
            "uninstall",
            "add or remove programs",
            "programs"
        ]
    ),
    (
        page::ACCOUNTS,
        "Accounts",
        ["accounts", "your info", "sign-in options", "user"]
    ),
    (
        page::USERS_GROUPS,
        "Users & groups",
        ["users", "groups", "local users", "other users"]
    ),
    (
        page::TIME,
        "Time & language",
        ["time and language", "typing"]
    ),
    (
        page::DATE_TIME,
        "Date & time",
        ["date", "time", "clock", "time zone", "calendar"]
    ),
    (
        page::LANGUAGE_REGION,
        "Language & region",
        [
            "language",
            "region",
            "keyboard layout",
            "input",
            "regional format"
        ]
    ),
    (page::GAMING, "Gaming", ["gaming", "game mode"]),
    (
        page::ACCESSIBILITY,
        "Accessibility",
        [
            "accessibility",
            "ease of access",
            "text size",
            "contrast",
            "magnifier",
            "narrator"
        ]
    ),
    (
        page::VISUAL_EFFECTS,
        "Visual effects",
        [
            "visual effects",
            "animation effects",
            "reduce motion",
            "transparency",
            "scrollbars"
        ]
    ),
    (
        page::PRIVACY,
        "Privacy & security",
        [
            "privacy",
            "security",
            "permissions",
            "location",
            "camera",
            "microphone"
        ]
    ),
    (
        page::SEARCH_PERMISSIONS,
        "Search permissions",
        [
            "search permissions",
            "search history",
            "searching windows",
            "search settings"
        ]
    ),
    (
        page::PERMISSIONS_SHARING,
        "Permissions & sharing",
        ["sharing", "file sharing", "access control"]
    ),
    (
        page::UPDATE,
        "Update & recovery",
        ["update", "windows update", "recovery", "reset"]
    ),
    (
        page::BACKUP_RESTORE,
        "Backup & restore",
        ["backup", "restore", "file history"]
    ),
    (
        page::POWER_MANAGEMENT,
        "Power management",
        ["power plan", "power options", "power policies", "shutdown"]
    ),
    (
        page::DISK_MANAGEMENT,
        "Disk management",
        ["disk", "drive", "partition", "volumes"]
    ),
    (
        page::DEVICES_DRIVERS,
        "Devices & drivers",
        ["device manager", "drivers", "hardware"]
    ),
    (
        page::NETWORK_ADMINISTRATION,
        "Network administration",
        ["network connections", "adapters", "ip address", "dns"]
    ),
    (
        page::SERVICES,
        "Services",
        ["services", "background services"]
    ),
    (
        page::ENVIRONMENT,
        "Environment variables",
        ["environment variables", "path", "variables"]
    ),
    (
        page::SYSTEM_CONFIGURATION,
        "System configuration",
        ["system configuration", "msconfig", "boot", "startup"]
    ),
    (page::REGISTRY, "Registry", ["registry", "regedit"]),
    (
        page::GROUP_POLICY,
        "Group policy",
        ["group policy", "gpedit", "policies"]
    ),
    (
        page::EVENT_VIEWER,
        "Event viewer",
        ["event viewer", "logs", "events"]
    ),
    (
        page::TASK_SCHEDULER,
        "Task scheduler",
        ["task scheduler", "scheduled tasks"]
    ),
    (
        page::PERFORMANCE,
        "Performance & diagnostics",
        ["performance", "diagnostics", "troubleshoot"]
    ),
    (
        page::DEVELOPER_TOOLS,
        "Developer & tools",
        ["developer mode", "for developers", "developer"]
    ),
];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    App(App),
    /// A Terminal profile (Doorsh, Command Prompt, PowerShell, DSL).
    Shell(crate::apps::Shell),
    /// A bundled tool listed only in search (gallery, Captions).
    Tool(Tool),
    Settings(SettingsPage),
    /// An app from the desktop shell's registration database (index into
    /// Start's catalog): installed Windows programs and other registrations.
    Registered(u8),
    /// A file or folder from the search provider (index into Start's files).
    File(u8),
    /// "Search in File Explorer" for the current query.
    Explore,
    /// No file-search provider is connected (honest placeholder row).
    FilesUnavailable,
}
impl Hit {
    /// Static title (provider results are titled by Start from their data).
    pub fn title(self) -> &'static str {
        match self {
            Self::App(app) => app.label(),
            Self::Shell(shell) => shell.label(),
            Self::Tool(tool) => tool.label(),
            Self::Settings(page) => page.title,
            Self::File(_) | Self::Registered(_) => "",
            Self::Explore => "Search in File Explorer",
            Self::FilesUnavailable => "File search isn't available right now",
        }
    }
    pub fn kind(self) -> &'static str {
        match self {
            Self::App(_) | Self::Shell(_) | Self::Tool(_) | Self::Registered(_) => "App",
            Self::Settings(_) => "System settings",
            Self::File(_) => "File",
            Self::Explore => "File Explorer",
            Self::FilesUnavailable => "Open File Explorer to search files and folders",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    Apps,
    /// Files (not folders) from the search provider.
    Documents,
    /// Web results: no web backend, so the pill is shown disabled.
    Web,
    Settings,
    /// Folders from the search provider.
    Folders,
    /// Photos: no photo index, so the pill is shown disabled.
    Photos,
}
impl Filter {
    /// Windows 11 26H2 Search pill order.
    pub const ALL: [Self; 7] = [
        Self::All,
        Self::Apps,
        Self::Documents,
        Self::Web,
        Self::Settings,
        Self::Folders,
        Self::Photos,
    ];
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Apps => "Apps",
            Self::Documents => "Documents",
            Self::Web => "Web",
            Self::Settings => "Settings",
            Self::Folders => "Folders",
            Self::Photos => "Photos",
        }
    }
    /// A backend serves this filter (Web and Photos have none).
    pub const fn available(self) -> bool {
        !matches!(self, Self::Web | Self::Photos)
    }
    /// Why an unavailable filter is disabled (accessibility description).
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Web => "Web search isn't available: Doors has no web search service",
            Self::Photos => "Photo search isn't available: Doors has no photo index",
            _ => "",
        }
    }
    /// Stable index (hover keys and accessibility node ids): the original
    /// four keep 0..=3.
    pub const fn id(self) -> u8 {
        match self {
            Self::All => 0,
            Self::Apps => 1,
            Self::Documents => 2,
            Self::Settings => 3,
            Self::Web => 4,
            Self::Folders => 5,
            Self::Photos => 6,
        }
    }
    /// Provider files of this kind (`folder`) pass the filter.
    pub const fn shows_file(self, folder: bool) -> bool {
        match self {
            Self::All => true,
            Self::Documents => !folder,
            Self::Folders => folder,
            _ => false,
        }
    }
}
fn lower(b: u8) -> u8 {
    b.to_ascii_lowercase()
}
fn starts_with(text: &str, query: &str) -> bool {
    text.len() >= query.len()
        && text
            .bytes()
            .zip(query.bytes())
            .all(|(a, b)| lower(a) == lower(b))
}
fn contains(text: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    (0..=text.len().saturating_sub(query.len()))
        .any(|i| text.is_char_boundary(i) && starts_with(&text[i..], query))
}
/// Any word of `text` starts with `query`.
fn word_prefix(text: &str, query: &str) -> bool {
    let bytes = text.as_bytes();
    (0..text.len()).any(|i| {
        (i == 0 || matches!(bytes[i - 1], b' ' | b'-' | b'&' | b'(' | b'/'))
            && text.is_char_boundary(i)
            && starts_with(&text[i..], query)
    })
}
/// Relevance: whole-title prefix > word prefix > substring > keyword.
fn score(title: &str, keywords: &[&str], query: &str) -> Option<u32> {
    let query = query.trim();
    if query.is_empty() {
        return None;
    }
    if starts_with(title, query) {
        return Some(if title.len() == query.len() {
            1000
        } else {
            900
        });
    }
    if word_prefix(title, query) {
        return Some(800);
    }
    if contains(title, query) {
        return Some(600);
    }
    if keywords.iter().any(|k| starts_with(k, query)) {
        return Some(500);
    }
    if keywords.iter().any(|k| word_prefix(k, query)) {
        return Some(400);
    }
    if query.len() >= 3 && keywords.iter().any(|k| contains(k, query)) {
        return Some(300);
    }
    None
}
/// Ranked results. Apps win ties against Settings (Windows' best match bias).
pub fn search(query: &str, filter: Filter) -> Vec<Hit> {
    search_with(query, filter, &[])
}
/// `search` over the catalog apps, tools, Settings and the registered apps
/// `registered` ((label, keywords) in Start catalog order).
pub fn search_with(query: &str, filter: Filter, registered: &[(&str, &[&str])]) -> Vec<Hit> {
    let mut hits: Vec<(u32, u32, Hit)> = Vec::new();
    if matches!(filter, Filter::All | Filter::Apps) {
        for (i, (label, keywords)) in registered.iter().enumerate().take(usize::from(u8::MAX)) {
            if let Some(s) = score(label, keywords, query) {
                hits.push((s + 50, 60 + i as u32, Hit::Registered(i as u8)));
            }
        }
    }
    if matches!(filter, Filter::All | Filter::Apps) {
        for app in App::ALL {
            if let Some(s) = score(app.label(), app.keywords(), query) {
                hits.push((s + 50, app.index() as u32, Hit::App(app)));
            }
        }
        for tool in Tool::ALL {
            if let Some(s) = score(tool.label(), tool.keywords(), query) {
                hits.push((s + 50, 50 + tool.index() as u32, Hit::Tool(tool)));
            }
        }
        for shell in crate::apps::Shell::ALL {
            if let Some(s) = score(shell.label(), shell.keywords(), query) {
                hits.push((s + 50, 40 + shell.index() as u32, Hit::Shell(shell)));
            }
        }
        // A Windows program typed by name ("notepad", "taskmgr.exe",
        // "pwsh", "mspaint") is a best match for the Doors app replacing it.
        let legacy = match crate::run::resolve(query) {
            Some(crate::run::Action::App(app)) => Some(Hit::App(app)),
            Some(crate::run::Action::Tool(tool)) => Some(Hit::Tool(tool)),
            Some(crate::run::Action::Profile(shell)) => Some(Hit::Shell(shell)),
            _ => None,
        };
        if let Some(hit) = legacy {
            match hits.iter_mut().find(|h| h.2 == hit) {
                Some(h) => h.0 = h.0.max(1060),
                None => hits.push((1060, 0, hit)),
            }
        }
    }
    if matches!(filter, Filter::All | Filter::Settings) {
        for (i, page) in SETTINGS.iter().enumerate() {
            if let Some(s) = score(page.title, page.keywords, query) {
                hits.push((s, 100 + i as u32, Hit::Settings(*page)));
            }
        }
        // The shared Settings route table (door_settings_map): legacy tools
        // ("Control Panel", "Device Manager", "System Properties"), page
        // sections and pages this table has no entry for. A typed legacy
        // command ("sysdm.cpl", "ms-settings:display") is a strong match.
        let exact = door_settings_map::resolve(query);
        for (k, hit) in door_settings_map::search(query).take(32).enumerate() {
            use door_settings_map::Hit as Map;
            let page = hit.page().index();
            if hits
                .iter()
                .any(|h| matches!(h.2, Hit::Settings(p) if p.page & 0xffff == page))
            {
                continue;
            }
            let title = match hit {
                Map::Section(s) => s.title,
                Map::Legacy(r) => r.name,
                Map::Page(p) | Map::Keyword(p) => p.nav_title(),
            };
            let Some(s) =
                score(title, &[], query).or_else(|| (exact == Some(hit.target())).then_some(950))
            else {
                continue;
            };
            hits.push((
                s,
                200 + k as u32,
                Hit::Settings(SettingsPage {
                    page: door_settings_map::navigate_arg(hit.page(), hit.anchor()),
                    title,
                    keywords: &[],
                }),
            ));
        }
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.into_iter().map(|h| h.2).collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn apps_and_settings_rank_by_relevance_without_web_results() {
        let r = search("set", Filter::All);
        assert_eq!(r[0], Hit::App(App::Settings));
        assert!(r.iter().any(|h| matches!(h, Hit::Settings(_))));
        let r = search("taskbar", Filter::All);
        assert!(matches!(r[0], Hit::Settings(p) if p.page == page::TASKBAR));
        let r = search("task manager", Filter::All);
        assert_eq!(r[0], Hit::App(App::Monitor));
        assert_eq!(search("taskmgr", Filter::Apps), [Hit::App(App::Monitor)]);
        assert_eq!(search("monitor", Filter::Apps), [Hit::App(App::Monitor)]);
        let r = search("dark mode", Filter::All);
        assert!(matches!(r[0], Hit::Settings(p) if p.page == page::COLORS));
        let r = search("wallpaper", Filter::All);
        assert!(matches!(r[0], Hit::Settings(p) if p.page == page::BACKGROUND));
        // Shells open Terminal on their own tab; Terminal itself follows.
        use crate::apps::Shell;
        let r = search("cmd", Filter::All);
        assert_eq!(r[0], Hit::Shell(Shell::CommandPrompt));
        assert!(r.contains(&Hit::App(App::Terminal)));
        assert_eq!(search("doorsh", Filter::All)[0], Hit::Shell(Shell::Doorsh));
        assert_eq!(
            search("PowerShell", Filter::All)[0],
            Hit::Shell(Shell::PowerShell)
        );
        assert_eq!(
            search("pwsh", Filter::All)[0],
            Hit::Shell(Shell::PowerShell)
        );
        assert_eq!(search("wsl", Filter::All)[0], Hit::Shell(Shell::Dsl));
        assert_eq!(search("dsl", Filter::All)[0], Hit::Shell(Shell::Dsl));
        // Windows program names find the Doors app replacing them.
        assert_eq!(search("notepad", Filter::All)[0], Hit::App(App::Notes));
        assert_eq!(search("wordpad", Filter::All)[0], Hit::App(App::Notes));
        assert!(search("zzzz", Filter::All).is_empty());
        assert!(search("   ", Filter::All).is_empty());
        assert!(search("display", Filter::Apps).is_empty());
        // Settings results only ("File Explorer Options" is a legacy tool).
        assert!(
            search("explorer", Filter::Settings)
                .iter()
                .all(|h| matches!(h, Hit::Settings(_)))
        );
        assert_eq!(search("EXPL", Filter::Apps), [Hit::App(App::Files)]);
        assert!(
            search("settings", Filter::Documents).is_empty(),
            "files come from the provider"
        );
    }
    #[test]
    fn every_settings_route_is_in_range_and_unique() {
        let mut seen = Vec::new();
        for page in SETTINGS {
            // Routes are validated by the shared table, not a local count.
            assert!(
                door_settings_map::decode_navigate(page.page).is_some() && page.page != page::HOME
            );
            assert!(!seen.contains(&page.page), "{}", page.title);
            seen.push(page.page);
        }
        // Every route Settings added for 26H2 (33..=45) is reachable.
        for id in 33..page::COUNT {
            assert!(seen.contains(&id), "Settings page {id} has no search entry");
        }
        assert!(
            matches!(search("search permissions", Filter::Settings)[0], Hit::Settings(p) if p.page == page::SEARCH_PERMISSIONS)
        );
        assert!(
            matches!(search("advanced system", Filter::Settings)[0], Hit::Settings(p) if p.page == page::SYSTEM_ADVANCED)
        );
        // Legacy names reach Settings through the shared route table.
        let r = search("control panel", Filter::All);
        assert!(
            matches!(r[0], Hit::Settings(p) if p.title == "Control Panel" && p.page & 0xffff == page::HOME),
            "{:?}",
            r.first()
        );
        assert!(matches!(
            search("sysdm.cpl", Filter::Settings).first(),
            Some(Hit::Settings(_))
        ));
        let fonts = door_settings_map::Page::Fonts.index();
        assert!(
            search("fonts", Filter::Settings)
                .iter()
                .any(|h| matches!(h, Hit::Settings(p) if p.page & 0xffff == fonts)),
            "a route the local table lacks is still found"
        );
    }
}
