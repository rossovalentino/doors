//! The Settings route table. A page's number is its identity: it is reported
//! to guest scenarios (`SETTINGS PAGE id=N`), carried by SETTINGS_NAVIGATE and
//! stored by callers, so existing numbers never move and new pages append.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u16)]
pub enum Page {
    Home,
    System,
    Bluetooth,
    Network,
    Personalization,
    Apps,
    Accounts,
    Time,
    Gaming,
    Accessibility,
    Privacy,
    Update,
    About,
    Display,
    InstalledApps,
    Taskbar,
    ContextMenu,
    /// System > Advanced > Registry Editor (regedit).
    Registry,
    /// Bluetooth & devices > Devices (Device Manager).
    DevicesDrivers,
    /// System > Storage > Disks & volumes (Disk Management).
    DiskManagement,
    /// Network & internet > Advanced network settings.
    NetworkAdministration,
    /// System > Advanced > Services (services.msc).
    Services,
    /// System > Advanced > Environment variables.
    Environment,
    /// System > Advanced > System configuration (msconfig).
    SystemConfiguration,
    /// System > Advanced > Policies (Group Policy, Local Security Policy).
    GroupPolicy,
    /// System > Advanced > Event logs (Event Viewer).
    EventViewer,
    /// System > Advanced > Scheduled tasks (Task Scheduler).
    TaskScheduler,
    /// Accounts > Your info (Local Users and Groups).
    UsersGroups,
    /// Privacy & security > File system (permissions and sharing).
    PermissionsSharing,
    /// System > Troubleshoot (checks; Performance Monitor -> Task Manager).
    Performance,
    /// System > Power > Power options (Power Options control panel).
    PowerManagement,
    /// System > Recovery (Recovery, System Restore, Backup and Restore).
    BackupRestore,
    /// System > Advanced > Developer tools.
    DeveloperTools,
    // Appended: the numeric identities above are reported to guest scenarios.
    Sound,
    Notifications,
    Power,
    Storage,
    Background,
    Colors,
    Themes,
    Start,
    DateTime,
    LanguageRegion,
    VisualEffects,
    SystemAdvanced,
    Search,
    ContrastThemes,
    TextSize,
    Typing,
    /// Personalization > Fonts (P14).
    Fonts,
    /// System > Clipboard (P14).
    Clipboard,
    /// Accounts > Sign-in options (P14).
    SignInOptions,
    /// Personalization > Lock screen (P14).
    LockScreen,
    /// Apps > Default apps (P14).
    DefaultApps,
    /// System > Multitasking (P14).
    Multitasking,
    /// Bluetooth & devices > Mouse (P14).
    Mouse,
    /// Accessibility > Color filters (P14).
    ColorFilters,
    /// Privacy & security > Microphone (P15).
    Microphone,
    /// Network & internet > VPN (P15).
    Vpn,
    /// Network & internet > VPN > a connection's Advanced options (its
    /// heading is the connection's name).
    VpnConnection,
    /// Accounts > Other users (P16): the local accounts besides yours.
    OtherUsers,
    /// Accounts > Other users > an account: its type and removal (the
    /// heading is the account's name).
    OtherUser,
    /// Network & internet > Proxy: automatic detection, the setup script
    /// and the manual proxy server.
    Proxy,
    /// Privacy & security > Firewall & network protection (Windows
    /// Security's page): each network profile's firewall switch and
    /// incoming-connections block, and Restore defaults.
    Firewall,
    /// Accessibility > Audio.
    Audio,
    /// Accessibility > Captions.
    Captions,
    /// Accessibility > Eye control.
    EyeControl,
    /// Accessibility > Hearing devices.
    HearingDevices,
    /// Accessibility > Keyboard.
    AccessibilityKeyboard,
    /// Accessibility > Magnifier.
    Magnifier,
    /// Accessibility > Mouse.
    AccessibilityMouse,
    /// Accessibility > Mouse pointer and touch.
    MousePointerAndTouch,
    /// Accessibility > Narrator.
    Narrator,
    /// Accessibility > Narrator > Configure braille displays.
    ConfigureBrailleDisplays,
    /// Accessibility > Narrator > Math reading.
    MathReading,
    /// Accessibility > Screen tint.
    ScreenTint,
    /// Accessibility > Speech.
    AccessibilitySpeech,
    /// Accessibility > Text cursor.
    TextCursor,
    /// Accounts > Access work or school.
    AccessWorkOrSchool,
    /// Accounts > Other Users > Kiosk.
    Kiosk,
    /// Accounts > Passkeys.
    AccountsPasskeys,
    /// Accounts > Passkeys > Advanced options.
    PasskeysAdvancedOptions,
    /// Accounts > Your accounts.
    YourAccounts,
    /// Apps > Advanced app settings.
    AdvancedAppSettings,
    /// Apps > Apps for websites.
    AppsForWebsites,
    /// Apps > Offline maps.
    OfflineMaps,
    /// Apps > Resume.
    Resume,
    /// Apps > Startup.
    Startup,
    /// Apps > Video playback.
    VideoPlayback,
    /// Bluetooth & devices > AutoPlay.
    AutoPlay,
    /// Bluetooth & devices > Cameras.
    Cameras,
    /// Bluetooth & devices > Keyboard.
    DevicesKeyboard,
    /// Bluetooth & devices > Pen & Windows Ink.
    PenAndWindowsInk,
    /// Bluetooth & devices > Printers & scanners.
    PrintersAndScanners,
    /// Bluetooth & devices > Touchpad.
    Touchpad,
    /// Bluetooth & devices > USB.
    USB,
    /// Gaming > Game Mode.
    GameMode,
    /// Network & internet > Advanced network settings > Advanced sharing settings.
    AdvancedSharingSettings,
    /// Network & internet > Airplane mode.
    AirplaneMode,
    /// Network & internet > Dial-up.
    DialUp,
    /// Network & internet > Ethernet.
    Ethernet,
    /// Network & internet > Mobile hotspot.
    MobileHotspot,
    /// Network & internet > Wi-Fi.
    WiFi,
    /// Network & internet > Wi-Fi > Manage known networks.
    ManageKnownNetworks,
    /// Personalization > Dynamic Lighting.
    DynamicLighting,
    /// Personalization > Start > Folders.
    Folders,
    /// Personalization > Text input.
    TextInput,
    /// Privacy & security > Account info.
    AccountInfo,
    /// Privacy & security > App diagnostics.
    AppDiagnostics,
    /// Privacy & security > Automatic file downloads.
    AutomaticFileDownloads,
    /// Privacy & security > Calendar.
    Calendar,
    /// Privacy & security > Call history.
    CallHistory,
    /// Privacy & security > Calls.
    Calls,
    /// Privacy & security > Camera.
    Camera,
    /// Privacy & security > Contacts.
    Contacts,
    /// Privacy & security > Custom dictionary.
    CustomDictionary,
    /// Privacy & security > Documents.
    Documents,
    /// Privacy & security > Downloads.
    Downloads,
    /// Privacy & security > Email.
    Email,
    /// Privacy & security > Foreground text access.
    ForegroundTextAccess,
    /// Privacy & security > Location.
    Location,
    /// Privacy & security > Messaging.
    Messaging,
    /// Privacy & security > Music.
    Music,
    /// Privacy & security > Notifications.
    PrivacyNotifications,
    /// Privacy & security > Other devices.
    OtherDevices,
    /// Privacy & security > Passkeys.
    PrivacyPasskeys,
    /// Privacy & security > Pictures.
    Pictures,
    /// Privacy & security > Radios.
    Radios,
    /// Privacy & security > Screenshot borders.
    ScreenshotBorders,
    /// Privacy & security > Screenshots and screen recording.
    ScreenshotsAndScreenRecording,
    /// Privacy & security > Speech.
    PrivacySpeech,
    /// Privacy & security > Tasks.
    Tasks,
    /// Privacy & security > Videos.
    Videos,
    /// Privacy & security > Voice activation.
    VoiceActivation,
    /// Privacy & security > Windows Security.
    WindowsSecurity,
    /// System > Activation.
    Activation,
    /// System > Display > Advanced display.
    AdvancedDisplay,
    /// System > Display > Graphics.
    Graphics,
    /// System > Display > HDR.
    HDR,
    /// System > Display > Night light.
    NightLight,
    /// System > Focus.
    Focus,
    /// System > Optional features.
    OptionalFeatures,
    /// System > Power & battery > Energy recommendations.
    EnergyRecommendations,
    /// System > Projecting to this PC.
    ProjectingToThisPC,
    /// System > Remote Desktop.
    RemoteDesktop,
    /// System > Share.
    Share,
    /// System > Sound > All sound devices.
    AllSoundDevices,
    /// System > Sound > Properties.
    Properties,
    /// System > Sound > Volume mixer.
    VolumeMixer,
    /// System > Storage > Storage Sense.
    StorageSense,
    /// System > Storage > Where new content is saved.
    WhereNewContentIsSaved,
    /// System > System components.
    SystemComponents,
    /// Time & language > Language & region > Domain lexicon.
    DomainLexicon,
    /// Time & language > Language & region > Keys.
    Keys,
    /// Time & language > Language & region > Microsoft IME.
    MicrosoftIME,
    /// Time & language > Language & region > Microsoft Pinyin.
    MicrosoftPinyin,
    /// Time & language > Language & region > User defined phrases.
    UserDefinedPhrases,
    /// Time & language > Speech.
    TimeSpeech,
    /// Windows Update > Advanced options.
    WindowsUpdateAdvancedOptions,
    /// Windows Update > Optional updates.
    OptionalUpdates,
    /// Windows Update > Uninstall updates.
    UninstallUpdates,
    /// Windows Update > Update history.
    UpdateHistory,
}
impl Page {
    pub const COUNT: usize = 162;
    pub const ALL: [Self; Self::COUNT] = [
        Self::Home,
        Self::System,
        Self::Bluetooth,
        Self::Network,
        Self::Personalization,
        Self::Apps,
        Self::Accounts,
        Self::Time,
        Self::Gaming,
        Self::Accessibility,
        Self::Privacy,
        Self::Update,
        Self::About,
        Self::Display,
        Self::InstalledApps,
        Self::Taskbar,
        Self::ContextMenu,
        Self::Registry,
        Self::DevicesDrivers,
        Self::DiskManagement,
        Self::NetworkAdministration,
        Self::Services,
        Self::Environment,
        Self::SystemConfiguration,
        Self::GroupPolicy,
        Self::EventViewer,
        Self::TaskScheduler,
        Self::UsersGroups,
        Self::PermissionsSharing,
        Self::Performance,
        Self::PowerManagement,
        Self::BackupRestore,
        Self::DeveloperTools,
        Self::Sound,
        Self::Notifications,
        Self::Power,
        Self::Storage,
        Self::Background,
        Self::Colors,
        Self::Themes,
        Self::Start,
        Self::DateTime,
        Self::LanguageRegion,
        Self::VisualEffects,
        Self::SystemAdvanced,
        Self::Search,
        Self::ContrastThemes,
        Self::TextSize,
        Self::Typing,
        Self::Fonts,
        Self::Clipboard,
        Self::SignInOptions,
        Self::LockScreen,
        Self::DefaultApps,
        Self::Multitasking,
        Self::Mouse,
        Self::ColorFilters,
        Self::Microphone,
        Self::Vpn,
        Self::VpnConnection,
        Self::OtherUsers,
        Self::OtherUser,
        Self::Proxy,
        Self::Firewall,
        Self::Audio,
        Self::Captions,
        Self::EyeControl,
        Self::HearingDevices,
        Self::AccessibilityKeyboard,
        Self::Magnifier,
        Self::AccessibilityMouse,
        Self::MousePointerAndTouch,
        Self::Narrator,
        Self::ConfigureBrailleDisplays,
        Self::MathReading,
        Self::ScreenTint,
        Self::AccessibilitySpeech,
        Self::TextCursor,
        Self::AccessWorkOrSchool,
        Self::Kiosk,
        Self::AccountsPasskeys,
        Self::PasskeysAdvancedOptions,
        Self::YourAccounts,
        Self::AdvancedAppSettings,
        Self::AppsForWebsites,
        Self::OfflineMaps,
        Self::Resume,
        Self::Startup,
        Self::VideoPlayback,
        Self::AutoPlay,
        Self::Cameras,
        Self::DevicesKeyboard,
        Self::PenAndWindowsInk,
        Self::PrintersAndScanners,
        Self::Touchpad,
        Self::USB,
        Self::GameMode,
        Self::AdvancedSharingSettings,
        Self::AirplaneMode,
        Self::DialUp,
        Self::Ethernet,
        Self::MobileHotspot,
        Self::WiFi,
        Self::ManageKnownNetworks,
        Self::DynamicLighting,
        Self::Folders,
        Self::TextInput,
        Self::AccountInfo,
        Self::AppDiagnostics,
        Self::AutomaticFileDownloads,
        Self::Calendar,
        Self::CallHistory,
        Self::Calls,
        Self::Camera,
        Self::Contacts,
        Self::CustomDictionary,
        Self::Documents,
        Self::Downloads,
        Self::Email,
        Self::ForegroundTextAccess,
        Self::Location,
        Self::Messaging,
        Self::Music,
        Self::PrivacyNotifications,
        Self::OtherDevices,
        Self::PrivacyPasskeys,
        Self::Pictures,
        Self::Radios,
        Self::ScreenshotBorders,
        Self::ScreenshotsAndScreenRecording,
        Self::PrivacySpeech,
        Self::Tasks,
        Self::Videos,
        Self::VoiceActivation,
        Self::WindowsSecurity,
        Self::Activation,
        Self::AdvancedDisplay,
        Self::Graphics,
        Self::HDR,
        Self::NightLight,
        Self::Focus,
        Self::OptionalFeatures,
        Self::EnergyRecommendations,
        Self::ProjectingToThisPC,
        Self::RemoteDesktop,
        Self::Share,
        Self::AllSoundDevices,
        Self::Properties,
        Self::VolumeMixer,
        Self::StorageSense,
        Self::WhereNewContentIsSaved,
        Self::SystemComponents,
        Self::DomainLexicon,
        Self::Keys,
        Self::MicrosoftIME,
        Self::MicrosoftPinyin,
        Self::UserDefinedPhrases,
        Self::TimeSpeech,
        Self::WindowsUpdateAdvancedOptions,
        Self::OptionalUpdates,
        Self::UninstallUpdates,
        Self::UpdateHistory,
    ];
    /// The navigation rail: Windows 11's twelve categories, nothing else.
    pub const NAV: [Self; 12] = [
        Self::Home,
        Self::System,
        Self::Bluetooth,
        Self::Network,
        Self::Personalization,
        Self::Apps,
        Self::Accounts,
        Self::Time,
        Self::Gaming,
        Self::Accessibility,
        Self::Privacy,
        Self::Update,
    ];
    /// Expert tools reached from System > Advanced > Administrative tools.
    pub const TOOLS: [Self; 8] = [
        Self::Registry,
        Self::Services,
        Self::Environment,
        Self::EventViewer,
        Self::TaskScheduler,
        Self::GroupPolicy,
        Self::SystemConfiguration,
        Self::DeveloperTools,
    ];
    pub const fn index(self) -> u32 {
        self as u32
    }
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }
    /// The page one level up (the page itself for a category).
    pub const fn parent(self) -> Self {
        match self {
            Self::About
            | Self::Display
            | Self::Sound
            | Self::Notifications
            | Self::Power
            | Self::Storage
            | Self::SystemAdvanced
            | Self::Performance
            | Self::BackupRestore
            | Self::Clipboard
            | Self::Multitasking => Self::System,
            Self::Registry
            | Self::Services
            | Self::Environment
            | Self::SystemConfiguration
            | Self::GroupPolicy
            | Self::EventViewer
            | Self::TaskScheduler
            | Self::DeveloperTools => Self::SystemAdvanced,
            Self::DiskManagement => Self::Storage,
            Self::PowerManagement => Self::Power,
            Self::DevicesDrivers | Self::Mouse => Self::Bluetooth,
            Self::NetworkAdministration | Self::Vpn | Self::Proxy => Self::Network,
            Self::VpnConnection => Self::Vpn,
            Self::OtherUsers => Self::Accounts,
            Self::OtherUser => Self::OtherUsers,
            Self::InstalledApps | Self::DefaultApps => Self::Apps,
            Self::UsersGroups | Self::SignInOptions => Self::Accounts,
            Self::PermissionsSharing | Self::Search | Self::Microphone | Self::Firewall => {
                Self::Privacy
            }
            Self::Taskbar
            | Self::ContextMenu
            | Self::Background
            | Self::Colors
            | Self::Themes
            | Self::Start
            | Self::Fonts
            | Self::LockScreen => Self::Personalization,
            Self::DateTime | Self::LanguageRegion | Self::Typing => Self::Time,
            Self::VisualEffects | Self::ContrastThemes | Self::TextSize | Self::ColorFilters => {
                Self::Accessibility
            }
            Self::Audio => Self::Accessibility,
            Self::Captions => Self::Accessibility,
            Self::EyeControl => Self::Accessibility,
            Self::HearingDevices => Self::Accessibility,
            Self::AccessibilityKeyboard => Self::Accessibility,
            Self::Magnifier => Self::Accessibility,
            Self::AccessibilityMouse => Self::Accessibility,
            Self::MousePointerAndTouch => Self::Accessibility,
            Self::Narrator => Self::Accessibility,
            Self::ConfigureBrailleDisplays => Self::Narrator,
            Self::MathReading => Self::Narrator,
            Self::ScreenTint => Self::Accessibility,
            Self::AccessibilitySpeech => Self::Accessibility,
            Self::TextCursor => Self::Accessibility,
            Self::AccessWorkOrSchool => Self::Accounts,
            Self::Kiosk => Self::OtherUsers,
            Self::AccountsPasskeys => Self::Accounts,
            Self::PasskeysAdvancedOptions => Self::AccountsPasskeys,
            Self::YourAccounts => Self::Accounts,
            Self::AdvancedAppSettings => Self::Apps,
            Self::AppsForWebsites => Self::Apps,
            Self::OfflineMaps => Self::Apps,
            Self::Resume => Self::Apps,
            Self::Startup => Self::Apps,
            Self::VideoPlayback => Self::Apps,
            Self::AutoPlay => Self::Bluetooth,
            Self::Cameras => Self::Bluetooth,
            Self::DevicesKeyboard => Self::Bluetooth,
            Self::PenAndWindowsInk => Self::Bluetooth,
            Self::PrintersAndScanners => Self::Bluetooth,
            Self::Touchpad => Self::Bluetooth,
            Self::USB => Self::Bluetooth,
            Self::GameMode => Self::Gaming,
            Self::AdvancedSharingSettings => Self::NetworkAdministration,
            Self::AirplaneMode => Self::Network,
            Self::DialUp => Self::Network,
            Self::Ethernet => Self::Network,
            Self::MobileHotspot => Self::Network,
            Self::WiFi => Self::Network,
            Self::ManageKnownNetworks => Self::WiFi,
            Self::DynamicLighting => Self::Personalization,
            Self::Folders => Self::Start,
            Self::TextInput => Self::Personalization,
            Self::AccountInfo => Self::Privacy,
            Self::AppDiagnostics => Self::Privacy,
            Self::AutomaticFileDownloads => Self::Privacy,
            Self::Calendar => Self::Privacy,
            Self::CallHistory => Self::Privacy,
            Self::Calls => Self::Privacy,
            Self::Camera => Self::Privacy,
            Self::Contacts => Self::Privacy,
            Self::CustomDictionary => Self::Privacy,
            Self::Documents => Self::Privacy,
            Self::Downloads => Self::Privacy,
            Self::Email => Self::Privacy,
            Self::ForegroundTextAccess => Self::Privacy,
            Self::Location => Self::Privacy,
            Self::Messaging => Self::Privacy,
            Self::Music => Self::Privacy,
            Self::PrivacyNotifications => Self::Privacy,
            Self::OtherDevices => Self::Privacy,
            Self::PrivacyPasskeys => Self::Privacy,
            Self::Pictures => Self::Privacy,
            Self::Radios => Self::Privacy,
            Self::ScreenshotBorders => Self::Privacy,
            Self::ScreenshotsAndScreenRecording => Self::Privacy,
            Self::PrivacySpeech => Self::Privacy,
            Self::Tasks => Self::Privacy,
            Self::Videos => Self::Privacy,
            Self::VoiceActivation => Self::Privacy,
            Self::WindowsSecurity => Self::Privacy,
            Self::Activation => Self::System,
            Self::AdvancedDisplay => Self::Display,
            Self::Graphics => Self::Display,
            Self::HDR => Self::Display,
            Self::NightLight => Self::Display,
            Self::Focus => Self::System,
            Self::OptionalFeatures => Self::System,
            Self::EnergyRecommendations => Self::Power,
            Self::ProjectingToThisPC => Self::System,
            Self::RemoteDesktop => Self::System,
            Self::Share => Self::System,
            Self::AllSoundDevices => Self::Sound,
            Self::Properties => Self::Sound,
            Self::VolumeMixer => Self::Sound,
            Self::StorageSense => Self::Storage,
            Self::WhereNewContentIsSaved => Self::Storage,
            Self::SystemComponents => Self::System,
            Self::DomainLexicon => Self::LanguageRegion,
            Self::Keys => Self::LanguageRegion,
            Self::MicrosoftIME => Self::LanguageRegion,
            Self::MicrosoftPinyin => Self::LanguageRegion,
            Self::UserDefinedPhrases => Self::LanguageRegion,
            Self::TimeSpeech => Self::Time,
            Self::WindowsUpdateAdvancedOptions => Self::Update,
            Self::OptionalUpdates => Self::Update,
            Self::UninstallUpdates => Self::Update,
            Self::UpdateHistory => Self::Update,
            page => page,
        }
    }
    /// The rail category a page belongs to (never deeper than three levels).
    pub const fn category(self) -> Self {
        self.parent().parent()
    }
    /// Levels below the category: 0 for a category, 1 for its pages, 2 for
    /// their subpages.
    pub const fn depth(self) -> usize {
        if self.parent() as u16 == self as u16 {
            0
        } else if self.parent().parent() as u16 == self.parent() as u16 {
            1
        } else {
            2
        }
    }
    /// The page's own name, as the rail, the heading and search show it.
    pub const fn nav_title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::System => "System",
            Self::Bluetooth => "Bluetooth & devices",
            Self::Network => "Network & internet",
            Self::Personalization => "Personalization",
            Self::Apps => "Apps",
            Self::Accounts => "Accounts",
            Self::Time => "Time & language",
            Self::Gaming => "Gaming",
            Self::Accessibility => "Accessibility",
            Self::Privacy => "Privacy & security",
            Self::Update => "Doors Update",
            Self::About => "About",
            Self::Display => "Display",
            Self::InstalledApps => "Installed apps",
            Self::Taskbar => "Taskbar",
            Self::ContextMenu => "Context menu",
            Self::Registry => "Registry Editor",
            Self::DevicesDrivers => "Devices",
            Self::DiskManagement => "Disks & volumes",
            Self::NetworkAdministration => "Advanced network settings",
            Self::Services => "Services",
            Self::Environment => "Environment variables",
            Self::SystemConfiguration => "System configuration",
            Self::GroupPolicy => "Policies",
            Self::EventViewer => "Event logs",
            Self::TaskScheduler => "Scheduled tasks",
            Self::UsersGroups => "Your info",
            Self::PermissionsSharing => "File system",
            Self::Performance => "Troubleshoot",
            Self::PowerManagement => "Power options",
            Self::BackupRestore => "Recovery",
            Self::DeveloperTools => "Developer tools",
            Self::Sound => "Sound",
            Self::Notifications => "Notifications",
            Self::Power => "Power & battery",
            Self::Storage => "Storage",
            Self::Background => "Background",
            Self::Colors => "Colors",
            Self::Themes => "Themes",
            Self::Start => "Start",
            Self::DateTime => "Date & time",
            Self::LanguageRegion => "Language & region",
            Self::VisualEffects => "Visual effects",
            Self::SystemAdvanced => "Advanced",
            Self::Search => "Search",
            Self::ContrastThemes => "Contrast themes",
            Self::TextSize => "Text size",
            Self::Typing => "Typing",
            Self::Fonts => "Fonts",
            Self::Clipboard => "Clipboard",
            Self::SignInOptions => "Sign-in options",
            Self::LockScreen => "Lock screen",
            Self::DefaultApps => "Default apps",
            Self::Multitasking => "Multitasking",
            Self::Mouse => "Mouse",
            Self::ColorFilters => "Color filters",
            Self::Microphone => "Microphone",
            Self::Vpn => "VPN",
            Self::VpnConnection => "Advanced options",
            Self::OtherUsers => "Other users",
            Self::OtherUser => "Account",
            Self::Proxy => "Proxy",
            Self::Firewall => "Firewall & network protection",
            Self::Audio => "Audio",
            Self::Captions => "Captions",
            Self::EyeControl => "Eye control",
            Self::HearingDevices => "Hearing devices",
            Self::AccessibilityKeyboard => "Keyboard",
            Self::Magnifier => "Magnifier",
            Self::AccessibilityMouse => "Mouse",
            Self::MousePointerAndTouch => "Mouse pointer and touch",
            Self::Narrator => "Narrator",
            Self::ConfigureBrailleDisplays => "Configure braille displays",
            Self::MathReading => "Math reading",
            Self::ScreenTint => "Screen tint",
            Self::AccessibilitySpeech => "Speech",
            Self::TextCursor => "Text cursor",
            Self::AccessWorkOrSchool => "Access work or school",
            Self::Kiosk => "Kiosk",
            Self::AccountsPasskeys => "Passkeys",
            Self::PasskeysAdvancedOptions => "Advanced options",
            Self::YourAccounts => "Your accounts",
            Self::AdvancedAppSettings => "Advanced app settings",
            Self::AppsForWebsites => "Apps for websites",
            Self::OfflineMaps => "Offline maps",
            Self::Resume => "Resume",
            Self::Startup => "Startup",
            Self::VideoPlayback => "Video playback",
            Self::AutoPlay => "AutoPlay",
            Self::Cameras => "Cameras",
            Self::DevicesKeyboard => "Keyboard",
            Self::PenAndWindowsInk => "Pen & Doors Ink",
            Self::PrintersAndScanners => "Printers & scanners",
            Self::Touchpad => "Touchpad",
            Self::USB => "USB",
            Self::GameMode => "Game Mode",
            Self::AdvancedSharingSettings => "Advanced sharing settings",
            Self::AirplaneMode => "Airplane mode",
            Self::DialUp => "Dial-up",
            Self::Ethernet => "Ethernet",
            Self::MobileHotspot => "Mobile hotspot",
            Self::WiFi => "Wi-Fi",
            Self::ManageKnownNetworks => "Manage known networks",
            Self::DynamicLighting => "Dynamic Lighting",
            Self::Folders => "Folders",
            Self::TextInput => "Text input",
            Self::AccountInfo => "Account info",
            Self::AppDiagnostics => "App diagnostics",
            Self::AutomaticFileDownloads => "Automatic file downloads",
            Self::Calendar => "Calendar",
            Self::CallHistory => "Call history",
            Self::Calls => "Calls",
            Self::Camera => "Camera",
            Self::Contacts => "Contacts",
            Self::CustomDictionary => "Custom dictionary",
            Self::Documents => "Documents",
            Self::Downloads => "Downloads",
            Self::Email => "Email",
            Self::ForegroundTextAccess => "Foreground text access",
            Self::Location => "Location",
            Self::Messaging => "Messaging",
            Self::Music => "Music",
            Self::PrivacyNotifications => "Notifications",
            Self::OtherDevices => "Other devices",
            Self::PrivacyPasskeys => "Passkeys",
            Self::Pictures => "Pictures",
            Self::Radios => "Radios",
            Self::ScreenshotBorders => "Screenshot borders",
            Self::ScreenshotsAndScreenRecording => "Screenshots and screen recording",
            Self::PrivacySpeech => "Speech",
            Self::Tasks => "Tasks",
            Self::Videos => "Videos",
            Self::VoiceActivation => "Voice activation",
            Self::WindowsSecurity => "Doors Security",
            Self::Activation => "Activation",
            Self::AdvancedDisplay => "Advanced display",
            Self::Graphics => "Graphics",
            Self::HDR => "HDR",
            Self::NightLight => "Night light",
            Self::Focus => "Focus",
            Self::OptionalFeatures => "Optional features",
            Self::EnergyRecommendations => "Energy recommendations",
            Self::ProjectingToThisPC => "Projecting to this PC",
            Self::RemoteDesktop => "Remote Desktop",
            Self::Share => "Share",
            Self::AllSoundDevices => "All sound devices",
            Self::Properties => "Properties",
            Self::VolumeMixer => "Volume mixer",
            Self::StorageSense => "Storage Sense",
            Self::WhereNewContentIsSaved => "Where new content is saved",
            Self::SystemComponents => "System components",
            Self::DomainLexicon => "Domain lexicon",
            Self::Keys => "Keys",
            Self::MicrosoftIME => "Macrohard IME",
            Self::MicrosoftPinyin => "Macrohard Pinyin",
            Self::UserDefinedPhrases => "User defined phrases",
            Self::TimeSpeech => "Speech",
            Self::WindowsUpdateAdvancedOptions => "Advanced options",
            Self::OptionalUpdates => "Optional updates",
            Self::UninstallUpdates => "Uninstall updates",
            Self::UpdateHistory => "Update history",
        }
    }
    /// The breadcrumb from the category down to this page.
    pub fn crumbs(self) -> impl Iterator<Item = Self> {
        let chain = [self.category(), self.parent(), self];
        let skip = 2 - self.depth();
        chain.into_iter().skip(skip)
    }
    /// Plain-language search terms (Windows Settings also finds "dark mode"
    /// or "wallpaper"). Legacy tool names live in the redirect table.
    pub const fn keywords(self) -> &'static str {
        match self {
            Self::Display => "resolution scale orientation monitor screen brightness",
            Self::Sound => "volume audio speakers microphone",
            Self::Notifications => "do not disturb alerts focus banners senders tips suggestions",
            Self::Power => "sleep battery energy shut down",
            Self::Storage => "disk space drive capacity free space",
            Self::About => "version build processor specifications rename pc device name",
            Self::Background => "wallpaper picture desktop background",
            Self::Colors => {
                "dark mode light mode accent transparency auto dark mode schedule title bar color"
            }
            Self::Themes => "desktop icons theme",
            Self::Start => "start menu size pinned recent all sections account",
            Self::Taskbar => "search box alignment position small buttons",
            Self::InstalledApps => "programs uninstall applications",
            Self::DateTime => "clock time zone date",
            Self::LanguageRegion => "language keyboard region format",
            Self::Typing => {
                "keyboard layout input method ime pinyin japanese korean add a keyboard"
            }
            Self::VisualEffects => "animation motion transparency effects",
            Self::SystemAdvanced => {
                "run dialog end task developer file explorer terminal file extensions hidden files"
            }
            Self::Search => "web results suggestions store search history",
            Self::ContrastThemes => "high contrast aquatic desert dusk night sky low vision",
            Self::TextSize => "font size larger text bigger text",
            Self::ContextMenu => "classic context menu right-click show more options file actions",
            Self::Network => "wi-fi ethernet vpn airplane proxy",
            Self::Bluetooth => "mouse printers usb keyboard",
            Self::Accounts => "user sign-in",
            Self::Update => "windows update updates",
            Self::Registry => "registry keys values hkey",
            Self::DevicesDrivers => {
                "drivers hardware processor keyboard mouse disk display adapter"
            }
            Self::DiskManagement => "disk management partitions volumes drive format",
            Self::NetworkAdministration => "network adapters sharing firewall internet options",
            Self::Services => "background services running",
            Self::Environment => "environment variables path",
            Self::SystemConfiguration => "startup boot configuration",
            Self::GroupPolicy => "policy group policy security policy",
            Self::EventViewer => "events log history activity",
            Self::TaskScheduler => "scheduled tasks schedule",
            Self::UsersGroups => "user account groups identity",
            Self::PermissionsSharing => "permissions sharing access files folders",
            Self::Performance => "troubleshooter diagnostics checks problems",
            Self::PowerManagement => "power plans power options sleep hibernate",
            Self::BackupRestore => "reset restore backup recovery",
            Self::DeveloperTools => "developer mode abi tools",
            Self::Fonts => "fonts typefaces install fonts",
            Self::Clipboard => "clipboard history win+v paste copy sync clear",
            Self::SignInOptions => {
                "pin password sign in options lock away windows hello change pin change password"
            }
            Self::LockScreen => "lock screen background picture spotlight sign-in screen",
            Self::DefaultApps => "default apps file types open with associations default programs",
            Self::Multitasking => {
                "snap windows snap layouts snap assist title bar shake aero shake alt+tab"
            }
            Self::Mouse => {
                "pointer speed mouse acceleration enhance pointer precision primary button scroll wheel lines"
            }
            Self::ColorFilters => {
                "grayscale inverted color blind deuteranopia protanopia tritanopia filters"
            }
            Self::Microphone => {
                "microphone mic recording record voice capture app permissions privacy"
            }
            Self::Vpn => "vpn wireguard tunnel virtual private network connect add remove",
            Self::Proxy => "proxy server setup script pac wpad automatically detect settings",
            Self::Firewall => {
                "firewall defender security network protection public private domain incoming connections block allow app restore defaults"
            }
            Self::OtherUsers => {
                "other users add account local account standard user administrator change account type remove account"
            }
            _ => "",
        }
    }
    /// Whether the page's own name contains `query` (ASCII, any case).
    pub fn name_matches(self, query: &str) -> bool {
        contains(self.nav_title(), query)
    }
    pub fn matches_keywords(self, query: &str) -> bool {
        !query.is_empty() && contains(self.keywords(), query)
    }
}
/// ASCII case-insensitive substring test (an empty query matches).
pub fn contains(text: &str, query: &str) -> bool {
    let text = text.as_bytes();
    let query = query.as_bytes();
    query.is_empty()
        || text
            .windows(query.len())
            .any(|part| part.eq_ignore_ascii_case(query))
}
