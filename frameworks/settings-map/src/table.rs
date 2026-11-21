//! Page sections (deep-link anchors) and the legacy redirect table. Every
//! entry names a Settings page, optionally a section of it; the inventory
//! in docs/design/settings-unification.md is the source of these rows.
use crate::Page;
use Page::*;

/// A named part of a page: a deep link scrolls to it, opens it if it is a
/// collapsed section and highlights it, as Windows does for a search hit.
/// `title` is the visible title of the card, header or section it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Section {
    pub anchor: &'static str,
    pub page: Page,
    pub title: &'static str,
}
macro_rules! sections {
    ($(($anchor:literal, $page:expr, $title:literal)),* $(,)?) => {
        &[$(Section { anchor: $anchor, page: $page, title: $title }),*]
    };
}
pub static SECTIONS: &[Section] = sections![
    ("device-specifications", About, "Device specifications"),
    ("doors-specifications", About, "Doors specifications"),
    (
        "administrative-tools",
        SystemAdvanced,
        "Administrative tools"
    ),
    ("end-task", SystemAdvanced, "End task"),
    ("file-explorer", SystemAdvanced, "File Explorer"),
    ("remote-desktop", SystemAdvanced, "Remote Desktop"),
    ("for-developers", SystemAdvanced, "For developers"),
    ("terminal", SystemAdvanced, "Terminal"),
    ("advanced-storage", Storage, "Advanced storage settings"),
    ("power-mode", Power, "Power Mode"),
    ("screen-sleep", Power, "Screen, sleep, & hibernate timeouts"),
    (
        "power-button",
        PowerManagement,
        "Choose what the power buttons do"
    ),
    ("reset-pc", BackupRestore, "Reset this PC"),
    ("advanced-startup", BackupRestore, "Advanced startup"),
    (
        "point-in-time-restore",
        BackupRestore,
        "Point-in-time restore"
    ),
    ("file-backup", BackupRestore, "File backup"),
    ("recovery-drive", BackupRestore, "Recovery drive"),
    ("user-variables", Environment, "User variables"),
    ("other-users", Accounts, "Other users"),
    ("sign-in-options", Accounts, "Sign-in options"),
    ("saved-credentials", Accounts, "Saved credentials"),
    ("printers", Bluetooth, "Printers & scanners"),
    ("mouse", Bluetooth, "Mouse"),
    ("keyboard", Bluetooth, "Keyboard"),
    ("autoplay", Bluetooth, "AutoPlay"),
    ("usb", Bluetooth, "USB"),
    ("pen", Bluetooth, "Pen & Doors Ink"),
    ("game-controllers", DevicesDrivers, "Game controllers"),
    ("proxy", Network, "Proxy"),
    (
        "network-adapters",
        NetworkAdministration,
        "Network adapters"
    ),
    (
        "sharing",
        NetworkAdministration,
        "Advanced sharing settings"
    ),
    (
        "internet-options",
        NetworkAdministration,
        "Internet options"
    ),
    ("firewall", NetworkAdministration, "Firewall"),
    ("doors-security", Privacy, "Doors Security"),
    ("location", Privacy, "Location"),
    ("find-files", Search, "Find files"),
    ("desktop-icons", Themes, "Desktop icon settings"),
    ("color-profile", Display, "Color profile"),
    ("night-light", Display, "Night light"),
    ("scale", Display, "Scale"),
    ("advanced-display", Display, "Advanced display"),
    ("optional-features", Apps, "Optional features"),
    ("default-apps", Apps, "Default apps"),
    ("startup", Apps, "Startup"),
    ("regional-format", LanguageRegion, "Regional format"),
    ("time-zone", DateTime, "Time zone"),
    ("speech", Time, "Speech"),
    ("system-check", Performance, "System check"),
    ("performance", Performance, "Performance"),
    ("activation", System, "Activation"),
    ("projecting", System, "Projecting to this PC"),
    ("clipboard", System, "Clipboard"),
    ("multitasking", System, "Multitasking"),
    ("sound-devices", Sound, "All sound devices"),
    ("game-mode", Gaming, "Game Mode"),
    ("transparency", Colors, "Transparency effects"),
    ("visual-performance", VisualEffects, "Animation effects"),
    ("mouse-pointer", Accessibility, "Mouse pointer and touch"),
    ("magnifier", Accessibility, "Magnifier"),
    ("narrator", Accessibility, "Narrator"),
    ("captions", Accessibility, "Captions"),
    ("keyboard-accessibility", Accessibility, "Keyboard"),
    ("install-fonts", Fonts, "Drag and drop to install"),
    (
        "local-security-policy",
        GroupPolicy,
        "Local security policy"
    ),
    (
        "startup-selection",
        SystemConfiguration,
        "Startup selection"
    ),
    ("update-history", Update, "Update history"),
];
/// How a legacy entry point is typed (documentation and search only).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A Run command or executable name (`regedit`, `msconfig`).
    Command,
    /// A control panel applet file (`sysdm.cpl`), optionally with a tab.
    Applet,
    /// A canonical control panel name (`control /name Microsoft.System`).
    CanonicalName,
    /// A `control <keyword>` shortcut (`control printers`).
    ControlKeyword,
    /// An MMC snap-in (`devmgmt.msc`).
    SnapIn,
    /// A Settings URI (`ms-settings:display`).
    Uri,
    /// A shell folder (`shell:::{CLSID}`, `shell:Fonts`).
    ShellFolder,
}
/// One legacy entry point: `key` is its normalized spelling (see
/// `normalize`), `name` the window or tool a user knows it by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Redirect {
    pub key: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    pub page: Page,
    pub section: Option<&'static str>,
}
macro_rules! r {
    ($key:literal, $name:literal, $kind:ident, $page:expr) => {
        Redirect {
            key: $key,
            name: $name,
            kind: Kind::$kind,
            page: $page,
            section: None,
        }
    };
    ($key:literal, $name:literal, $kind:ident, $page:expr, $section:literal) => {
        Redirect {
            key: $key,
            name: $name,
            kind: Kind::$kind,
            page: $page,
            section: Some($section),
        }
    };
}
pub static REDIRECTS: &[Redirect] = &[
    // Control Panel itself: its category view becomes Settings Home.
    r!("control", "Control Panel", Command, Home),
    // Windows programs replaced by a Settings page (owner, 2026-11-02:
    // every Windows program a user can start opens its Doors replacement).
    r!("charmap", "Character Map", Command, Fonts),
    r!("eudcedit", "Private Character Editor", Command, Fonts),
    r!("cttune", "ClearType Text Tuner", Command, Fonts),
    r!(
        "msra",
        "Doors Remote Assistance",
        Command,
        SystemAdvanced,
        "remote-desktop"
    ),
    r!(
        "quickassist",
        "Quick Assist",
        Command,
        SystemAdvanced,
        "remote-desktop"
    ),
    r!(
        "mmc",
        "Macrohard Management Console",
        Command,
        SystemAdvanced,
        "administrative-tools"
    ),
    r!(
        "odbcad32",
        "ODBC Data Sources",
        Command,
        SystemAdvanced,
        "administrative-tools"
    ),
    r!("sndvol", "Volume Mixer", Command, Sound),
    r!("mblctr", "Doors Mobility Center", Command, Power),
    r!("displayswitch", "Project", Command, Display),
    r!(
        "presentationsettings",
        "Presentation Settings",
        Command,
        Notifications
    ),
    r!(
        "useraccountcontrolsettings",
        "User Account Control Settings",
        Command,
        Privacy,
        "doors-security"
    ),
    r!(
        "sigverif",
        "File Signature Verification",
        Command,
        Privacy,
        "doors-security"
    ),
    r!(
        "mrt",
        "Malicious Software Removal Tool",
        Command,
        Privacy,
        "doors-security"
    ),
    r!(
        "rekeywiz",
        "Encrypting File System",
        Command,
        Privacy,
        "doors-security"
    ),
    r!(
        "bitlockerwizard",
        "BitLocker Drive Encryption",
        Command,
        Privacy,
        "doors-security"
    ),
    r!(
        "tpminit",
        "TPM Initialization",
        Command,
        Privacy,
        "doors-security"
    ),
    r!(
        "verifier",
        "Driver Verifier Manager",
        Command,
        DevicesDrivers
    ),
    r!("devicepairingwizard", "Add a device", Command, Bluetooth),
    r!(
        "tabcal",
        "Digitizer Calibration Tool",
        Command,
        Bluetooth,
        "pen"
    ),
    r!("wfs", "Doors Fax and Scan", Command, Bluetooth, "printers"),
    r!(
        "fxscover",
        "Fax Cover Page Editor",
        Command,
        Bluetooth,
        "printers"
    ),
    r!("dialer", "Phone Dialer", Command, Network),
    r!(
        "shrpubw",
        "Create A Shared Folder Wizard",
        Command,
        NetworkAdministration,
        "sharing"
    ),
    r!(
        "wf",
        "Doors Defender Firewall with Advanced Security",
        Command,
        NetworkAdministration,
        "firewall"
    ),
    r!("windowsupdate", "Doors Update", Command, Update),
    r!(
        "changepk",
        "Change product key",
        Command,
        System,
        "activation"
    ),
    r!(
        "computerdefaults",
        "Set Program Access and Computer Defaults",
        Command,
        Apps,
        "default-apps"
    ),
    r!(
        "fodhelper",
        "Manage optional features",
        Command,
        Apps,
        "optional-features"
    ),
    r!("msedge", "Macrohard Edge", Command, Apps, "default-apps"),
    r!(
        "iexplore",
        "Internet Explorer",
        Command,
        NetworkAdministration,
        "internet-options"
    ),
    r!(
        "shell:::{26ee0668-a00a-44d7-9371-beb064c98683}",
        "Control Panel",
        ShellFolder,
        Home
    ),
    r!(
        "shell:::{21ec2020-3aea-1069-a2dd-08002b30309d}",
        "All Control Panel Items",
        ShellFolder,
        Home
    ),
    r!(
        "shell:::{ed7ba470-8e54-465e-825c-99712043e01c}",
        "All Tasks",
        ShellFolder,
        Home
    ),
    r!(
        "shell:controlpanelfolder",
        "Control Panel",
        ShellFolder,
        Home
    ),
    // System and System Properties (sysdm.cpl and its tabs).
    r!("microsoft.system", "System", CanonicalName, About),
    r!("control system", "System", ControlKeyword, About),
    r!(
        "shell:::{bb06c0e4-d293-4f75-8a90-cb05b6477eee}",
        "System",
        ShellFolder,
        About
    ),
    r!("sysdm.cpl", "System Properties", Applet, About),
    r!(
        "sysdm.cpl,0",
        "System Properties: Computer Name",
        Applet,
        About
    ),
    r!(
        "systempropertiescomputername",
        "System Properties: Computer Name",
        Command,
        About
    ),
    r!(
        "sysdm.cpl,1",
        "System Properties: Hardware",
        Applet,
        DevicesDrivers
    ),
    r!(
        "systempropertieshardware",
        "System Properties: Hardware",
        Command,
        DevicesDrivers
    ),
    r!(
        "sysdm.cpl,3",
        "System Properties: Advanced",
        Applet,
        SystemAdvanced,
        "administrative-tools"
    ),
    r!(
        "systempropertiesadvanced",
        "Advanced system settings",
        Command,
        SystemAdvanced,
        "administrative-tools"
    ),
    r!(
        "systempropertiesperformance",
        "Performance Options",
        Command,
        VisualEffects
    ),
    r!(
        "systempropertiesdataexecutionprevention",
        "Data Execution Prevention",
        Command,
        Privacy,
        "doors-security"
    ),
    r!(
        "sysdm.cpl,4",
        "System Properties: System Protection",
        Applet,
        BackupRestore,
        "point-in-time-restore"
    ),
    r!(
        "systempropertiesprotection",
        "System Protection",
        Command,
        BackupRestore,
        "point-in-time-restore"
    ),
    r!(
        "sysdm.cpl,5",
        "System Properties: Remote",
        Applet,
        SystemAdvanced,
        "remote-desktop"
    ),
    r!(
        "systempropertiesremote",
        "System Properties: Remote",
        Command,
        SystemAdvanced,
        "remote-desktop"
    ),
    r!(
        "sysdm.cpl,editenvironmentvariables",
        "Environment Variables",
        Command,
        Environment
    ),
    r!(
        "rundll32 sysdm.cpl,editenvironmentvariables",
        "Environment Variables",
        Command,
        Environment
    ),
    r!(
        "winver",
        "About Doors",
        Command,
        About,
        "doors-specifications"
    ),
    r!(
        "msinfo32",
        "System Information",
        Command,
        About,
        "device-specifications"
    ),
    r!(
        "dxdiag",
        "DirectX Diagnostic Tool",
        Command,
        About,
        "device-specifications"
    ),
    r!("slui", "Windows Activation", Command, System, "activation"),
    // Display, personalization and fonts.
    r!("desk.cpl", "Display", Applet, Display),
    r!(
        "control desktop",
        "Personalization",
        ControlKeyword,
        Personalization
    ),
    r!(
        "control color",
        "Color and Appearance",
        ControlKeyword,
        Colors
    ),
    r!("microsoft.display", "Display", CanonicalName, Display),
    r!(
        "microsoft.personalization",
        "Personalization",
        CanonicalName,
        Personalization
    ),
    r!(
        "microsoft.taskbar",
        "Taskbar and Navigation",
        CanonicalName,
        Taskbar
    ),
    r!(
        "desk.cpl,0",
        "Desktop Icon Settings",
        Applet,
        Themes,
        "desktop-icons"
    ),
    r!("dpiscaling", "Display scaling", Command, Display, "scale"),
    r!(
        "colorcpl",
        "Color Management",
        Command,
        Display,
        "color-profile"
    ),
    r!(
        "microsoft.colormanagement",
        "Color Management",
        CanonicalName,
        Display,
        "color-profile"
    ),
    r!("control fonts", "Fonts", ControlKeyword, Fonts),
    r!("microsoft.fonts", "Fonts", CanonicalName, Fonts),
    r!("shell:fonts", "Fonts folder", ShellFolder, Fonts),
    r!("fontview", "Font Viewer", Command, Fonts, "install-fonts"),
    // Sound and power.
    r!("mmsys.cpl", "Sound", Applet, Sound),
    r!("microsoft.sound", "Sound", CanonicalName, Sound),
    r!("powercfg.cpl", "Power Options", Applet, PowerManagement),
    r!(
        "microsoft.poweroptions",
        "Power Options",
        CanonicalName,
        PowerManagement
    ),
    r!(
        "microsoft.windowsmobilitycenter",
        "Windows Mobility Center",
        CanonicalName,
        Power
    ),
    // Storage, disks and recovery.
    r!("diskmgmt.msc", "Disk Management", SnapIn, DiskManagement),
    r!(
        "microsoft.storagespaces",
        "Storage Spaces",
        CanonicalName,
        Storage,
        "advanced-storage"
    ),
    r!("cleanmgr", "Disk Cleanup", Command, Storage),
    r!(
        "dfrgui",
        "Optimize Drives",
        Command,
        Storage,
        "advanced-storage"
    ),
    r!(
        "microsoft.recovery",
        "Recovery",
        CanonicalName,
        BackupRestore
    ),
    r!(
        "rstrui",
        "System Restore",
        Command,
        BackupRestore,
        "point-in-time-restore"
    ),
    r!(
        "recoverydrive",
        "Recovery Media Creator",
        Command,
        BackupRestore,
        "recovery-drive"
    ),
    r!(
        "microsoft.backupandrestorecenter",
        "Backup and Restore (Doors 7)",
        CanonicalName,
        BackupRestore,
        "file-backup"
    ),
    r!(
        "sdclt",
        "Backup and Restore (Doors 7)",
        Command,
        BackupRestore,
        "file-backup"
    ),
    r!(
        "microsoft.filehistory",
        "File History",
        CanonicalName,
        BackupRestore,
        "file-backup"
    ),
    r!(
        "microsoft.bitlockerdriveencryption",
        "BitLocker Drive Encryption",
        CanonicalName,
        DiskManagement
    ),
    r!(
        "microsoft.iscsiinitiator",
        "iSCSI Initiator",
        CanonicalName,
        DiskManagement
    ),
    r!("iscsicpl", "iSCSI Initiator", Command, DiskManagement),
    // Devices and hardware.
    r!("devmgmt.msc", "Device Manager", SnapIn, DevicesDrivers),
    r!("hdwwiz.cpl", "Device Manager", Applet, DevicesDrivers),
    r!("hdwwiz", "Add Hardware", Command, DevicesDrivers),
    r!(
        "microsoft.devicemanager",
        "Device Manager",
        CanonicalName,
        DevicesDrivers
    ),
    r!(
        "control printers",
        "Devices and Printers",
        ControlKeyword,
        Bluetooth,
        "printers"
    ),
    r!(
        "microsoft.devicesandprinters",
        "Devices and Printers",
        CanonicalName,
        Bluetooth,
        "printers"
    ),
    r!(
        "shell:::{a8a91a66-3a7d-4424-8d24-04e180695c7a}",
        "Devices and Printers",
        ShellFolder,
        Bluetooth,
        "printers"
    ),
    r!(
        "printui",
        "Printer settings",
        Command,
        Bluetooth,
        "printers"
    ),
    r!(
        "printmanagement.msc",
        "Print Management",
        SnapIn,
        Bluetooth,
        "printers"
    ),
    r!("main.cpl", "Mouse Properties", Applet, Mouse),
    r!("control mouse", "Mouse Properties", ControlKeyword, Mouse),
    r!(
        "microsoft.mouse",
        "Mouse Properties",
        CanonicalName,
        Bluetooth,
        "mouse"
    ),
    r!(
        "control keyboard",
        "Keyboard Properties",
        ControlKeyword,
        Bluetooth,
        "keyboard"
    ),
    r!(
        "microsoft.keyboard",
        "Keyboard Properties",
        CanonicalName,
        Bluetooth,
        "keyboard"
    ),
    r!("bthprops.cpl", "Bluetooth Devices", Applet, Bluetooth),
    r!(
        "microsoft.bluetoothdevices",
        "Bluetooth Devices",
        CanonicalName,
        Bluetooth
    ),
    r!("fsquirt", "Bluetooth File Transfer", Command, Bluetooth),
    r!(
        "joy.cpl",
        "Game Controllers",
        Applet,
        DevicesDrivers,
        "game-controllers"
    ),
    r!(
        "microsoft.gamecontrollers",
        "Game Controllers",
        CanonicalName,
        DevicesDrivers,
        "game-controllers"
    ),
    r!("tabletpc.cpl", "Pen and Touch", Applet, Bluetooth, "pen"),
    r!(
        "microsoft.penandtouch",
        "Pen and Touch",
        CanonicalName,
        Bluetooth,
        "pen"
    ),
    r!(
        "microsoft.autoplay",
        "AutoPlay",
        CanonicalName,
        Bluetooth,
        "autoplay"
    ),
    r!(
        "telephon.cpl",
        "Phone and Modem",
        Applet,
        Privacy,
        "location"
    ),
    r!(
        "microsoft.phoneandmodem",
        "Phone and Modem",
        CanonicalName,
        Privacy,
        "location"
    ),
    r!(
        "microsoft.locationsettings",
        "Location Settings",
        CanonicalName,
        Privacy,
        "location"
    ),
    // Network and internet.
    r!(
        "ncpa.cpl",
        "Network Connections",
        Applet,
        NetworkAdministration,
        "network-adapters"
    ),
    r!(
        "control netconnections",
        "Network Connections",
        ControlKeyword,
        NetworkAdministration,
        "network-adapters"
    ),
    r!(
        "shell:::{7007acc7-3202-11d1-aad2-00805fc1270e}",
        "Network Connections",
        ShellFolder,
        NetworkAdministration,
        "network-adapters"
    ),
    r!(
        "microsoft.networkandsharingcenter",
        "Network and Sharing Center",
        CanonicalName,
        NetworkAdministration
    ),
    r!(
        "inetcpl.cpl",
        "Internet Properties",
        Applet,
        NetworkAdministration,
        "internet-options"
    ),
    r!(
        "microsoft.internetoptions",
        "Internet Options",
        CanonicalName,
        NetworkAdministration,
        "internet-options"
    ),
    r!("firewall.cpl", "Doors Defender Firewall", Applet, Firewall),
    r!(
        "microsoft.windowsfirewall",
        "Doors Defender Firewall",
        CanonicalName,
        Firewall
    ),
    r!(
        "wf.msc",
        "Doors Defender Firewall with Advanced Security",
        SnapIn,
        NetworkAdministration,
        "firewall"
    ),
    r!(
        "microsoft.remoteappanddesktopconnections",
        "RemoteApp and Desktop Connections",
        CanonicalName,
        SystemAdvanced,
        "remote-desktop"
    ),
    r!(
        "mstsc",
        "Remote Desktop Connection",
        Command,
        SystemAdvanced,
        "remote-desktop"
    ),
    r!(
        "fsmgmt.msc",
        "Shared Folders",
        SnapIn,
        NetworkAdministration,
        "sharing"
    ),
    r!(
        "microsoft.synccenter",
        "Sync Center",
        CanonicalName,
        NetworkAdministration,
        "sharing"
    ),
    r!(
        "microsoft.workfolders",
        "Work Folders",
        CanonicalName,
        Accounts
    ),
    // Apps.
    r!("appwiz.cpl", "Programs and Features", Applet, InstalledApps),
    r!(
        "microsoft.programsandfeatures",
        "Programs and Features",
        CanonicalName,
        InstalledApps
    ),
    r!(
        "shell:::{7b81be6a-ce2b-4676-a29e-eb907a5126c5}",
        "Programs and Features",
        ShellFolder,
        InstalledApps
    ),
    r!(
        "shell:appsfolder",
        "Applications",
        ShellFolder,
        InstalledApps
    ),
    r!(
        "microsoft.getprograms",
        "Get Programs",
        CanonicalName,
        InstalledApps
    ),
    r!(
        "optionalfeatures",
        "Windows Features",
        Command,
        Apps,
        "optional-features"
    ),
    r!(
        "microsoft.defaultprograms",
        "Default Programs",
        CanonicalName,
        DefaultApps
    ),
    // Accounts and security.
    r!(
        "control userpasswords",
        "User Accounts",
        ControlKeyword,
        UsersGroups
    ),
    r!(
        "microsoft.useraccounts",
        "User Accounts",
        CanonicalName,
        UsersGroups
    ),
    r!(
        "control userpasswords2",
        "Advanced User Accounts",
        ControlKeyword,
        Accounts,
        "other-users"
    ),
    r!(
        "netplwiz",
        "Advanced User Accounts",
        Command,
        Accounts,
        "other-users"
    ),
    r!("lusrmgr.msc", "Local Users and Groups", SnapIn, UsersGroups),
    r!(
        "microsoft.credentialmanager",
        "Credential Manager",
        CanonicalName,
        Accounts,
        "saved-credentials"
    ),
    r!(
        "control keymgr.dll",
        "Credential Manager",
        ControlKeyword,
        Accounts,
        "saved-credentials"
    ),
    r!(
        "credwiz",
        "Stored User Names and Passwords",
        Command,
        Accounts,
        "saved-credentials"
    ),
    r!(
        "certmgr.msc",
        "Certificates",
        SnapIn,
        Privacy,
        "doors-security"
    ),
    r!(
        "certlm.msc",
        "Certificates - Local Computer",
        SnapIn,
        Privacy,
        "doors-security"
    ),
    r!(
        "tpm.msc",
        "Trusted Platform Module Management",
        SnapIn,
        Privacy,
        "doors-security"
    ),
    r!(
        "azman.msc",
        "Authorization Manager",
        SnapIn,
        SystemAdvanced,
        "administrative-tools"
    ),
    r!(
        "comexp.msc",
        "Component Services",
        SnapIn,
        SystemAdvanced,
        "administrative-tools"
    ),
    r!(
        "wmimgmt.msc",
        "WMI Control",
        SnapIn,
        SystemAdvanced,
        "administrative-tools"
    ),
    r!(
        "devmoderunasuserconfig.msc",
        "Run As User Configuration",
        SnapIn,
        SystemAdvanced,
        "for-developers"
    ),
    r!("irprops.cpl", "Infrared", Applet, Bluetooth),
    r!(
        "wscui.cpl",
        "Security and Maintenance",
        Applet,
        Privacy,
        "doors-security"
    ),
    r!(
        "microsoft.actioncenter",
        "Security and Maintenance",
        CanonicalName,
        Privacy,
        "doors-security"
    ),
    r!(
        "microsoft.windowsdefender",
        "Windows Defender",
        CanonicalName,
        Privacy,
        "doors-security"
    ),
    r!(
        "gpedit.msc",
        "Local Group Policy Editor",
        SnapIn,
        GroupPolicy
    ),
    r!(
        "secpol.msc",
        "Local Security Policy",
        SnapIn,
        GroupPolicy,
        "local-security-policy"
    ),
    r!("rsop.msc", "Resultant Set of Policy", SnapIn, GroupPolicy),
    r!("gpresult", "Group Policy Results", Command, GroupPolicy),
    // Time, language and region.
    r!(
        "intl.cpl",
        "Region",
        Applet,
        LanguageRegion,
        "regional-format"
    ),
    r!(
        "control international",
        "Region",
        ControlKeyword,
        LanguageRegion,
        "regional-format"
    ),
    r!(
        "microsoft.regionandlanguage",
        "Region",
        CanonicalName,
        LanguageRegion
    ),
    r!("timedate.cpl", "Date and Time", Applet, DateTime),
    r!(
        "control date/time",
        "Date and Time",
        ControlKeyword,
        DateTime
    ),
    r!(
        "microsoft.dateandtime",
        "Date and Time",
        CanonicalName,
        DateTime
    ),
    r!("sapi.cpl", "Speech Properties", Applet, Time, "speech"),
    r!(
        "microsoft.speechrecognition",
        "Speech Recognition",
        CanonicalName,
        Time,
        "speech"
    ),
    r!(
        "microsoft.texttospeech",
        "Text to Speech",
        CanonicalName,
        Time,
        "speech"
    ),
    r!(
        "lpksetup",
        "Install or uninstall display languages",
        Command,
        LanguageRegion
    ),
    // Accessibility.
    r!("access.cpl", "Ease of Access Center", Applet, Accessibility),
    r!(
        "microsoft.easeofaccesscenter",
        "Ease of Access Center",
        CanonicalName,
        Accessibility
    ),
    // File Explorer and search.
    r!(
        "control folders",
        "File Explorer Options",
        ControlKeyword,
        SystemAdvanced,
        "file-explorer"
    ),
    r!(
        "microsoft.folderoptions",
        "File Explorer Options",
        CanonicalName,
        SystemAdvanced,
        "file-explorer"
    ),
    r!(
        "microsoft.indexingoptions",
        "Indexing Options",
        CanonicalName,
        Search,
        "find-files"
    ),
    r!(
        "control srchadmin.dll",
        "Indexing Options",
        ControlKeyword,
        Search,
        "find-files"
    ),
    // Administrative tools and MMC snap-ins.
    r!(
        "control admintools",
        "Doors Tools",
        ControlKeyword,
        SystemAdvanced,
        "administrative-tools"
    ),
    r!(
        "microsoft.administrativetools",
        "Doors Tools",
        CanonicalName,
        SystemAdvanced,
        "administrative-tools"
    ),
    r!(
        "shell:::{d20ea4e1-3957-11d2-a40b-0c5020524153}",
        "Doors Tools",
        ShellFolder,
        SystemAdvanced,
        "administrative-tools"
    ),
    r!(
        "shell:common administrative tools",
        "Doors Tools",
        ShellFolder,
        SystemAdvanced,
        "administrative-tools"
    ),
    r!(
        "compmgmt.msc",
        "Computer Management",
        SnapIn,
        SystemAdvanced,
        "administrative-tools"
    ),
    r!("services.msc", "Services", SnapIn, Services),
    r!("eventvwr", "Event Viewer", Command, EventViewer),
    // The Remote Access Phonebook (dial-up and VPN entries): Settings >
    // Network & internet > VPN holds the same phonebook (rasphone.pbk).
    r!("rasphone", "Remote Access Phonebook", Command, Vpn),
    r!("eventvwr.msc", "Event Viewer", SnapIn, EventViewer),
    r!("taskschd.msc", "Task Scheduler", SnapIn, TaskScheduler),
    r!(
        "control schedtasks",
        "Task Scheduler",
        ControlKeyword,
        TaskScheduler
    ),
    r!(
        "perfmon",
        "Performance Monitor",
        Command,
        Performance,
        "performance"
    ),
    r!(
        "perfmon.msc",
        "Performance Monitor",
        SnapIn,
        Performance,
        "performance"
    ),
    r!(
        "resmon",
        "Resource Monitor",
        Command,
        Performance,
        "performance"
    ),
    r!(
        "perfmon /res",
        "Resource Monitor",
        Command,
        Performance,
        "performance"
    ),
    r!(
        "mdsched",
        "Windows Memory Diagnostic",
        Command,
        Performance,
        "system-check"
    ),
    r!(
        "msdt",
        "Troubleshooting",
        Command,
        Performance,
        "system-check"
    ),
    r!(
        "microsoft.troubleshooting",
        "Troubleshooting",
        CanonicalName,
        Performance
    ),
    r!(
        "msconfig",
        "System Configuration",
        Command,
        SystemConfiguration
    ),
    r!("regedit", "Registry Editor", Command, Registry),
    r!("regedt32", "Registry Editor", Command, Registry),
    // Settings URIs (ms-settings:) for every page and section Doors has.
    r!("ms-settings:", "Settings", Uri, Home),
    r!(
        "ms-settings:agenttools",
        "Advanced display",
        Uri,
        AdvancedDisplay
    ),
    r!(
        "ms-settings:aicomponents",
        "Advanced display",
        Uri,
        AdvancedDisplay
    ),
    r!("ms-settings:customizelockscreenwidget", "Share", Uri, Share),
    r!(
        "ms-settings:devices-touch",
        "Keyboard",
        Uri,
        DevicesKeyboard
    ),
    r!(
        "ms-settings:display-advancedgraphics",
        "Graphics",
        Uri,
        Graphics
    ),
    r!(
        "ms-settings:easeofaccess-narrator-isautostartenabled",
        "Narrator",
        Uri,
        Narrator
    ),
    r!(
        "ms-settings:extras",
        "Energy recommendations",
        Uri,
        EnergyRecommendations
    ),
    r!("ms-settings:maps", "Offline maps", Uri, OfflineMaps),
    r!(
        "ms-settings:network-cellular",
        "Airplane mode",
        Uri,
        AirplaneMode
    ),
    r!("ms-settings:network-directaccess", "Dial-up", Uri, DialUp),
    r!(
        "ms-settings:phone-defaultapps",
        "Text input",
        Uri,
        TextInput
    ),
    r!(
        "ms-settings:privacy-activityhistory",
        "Account info",
        Uri,
        AccountInfo
    ),
    r!(
        "ms-settings:privacy-agenttools",
        "Account info",
        Uri,
        AccountInfo
    ),
    r!(
        "ms-settings:privacy-backgroundaitasks",
        "Automatic file downloads",
        Uri,
        AutomaticFileDownloads
    ),
    r!(
        "ms-settings:privacy-backgroundapps",
        "Automatic file downloads",
        Uri,
        AutomaticFileDownloads
    ),
    r!(
        "ms-settings:privacy-cloudpc",
        "Call history",
        Uri,
        CallHistory
    ),
    r!("ms-settings:privacy-eyetracker", "Email", Uri, Email),
    r!(
        "ms-settings:privacy-presencesensing",
        "Pictures",
        Uri,
        Pictures
    ),
    r!("ms-settings:privacy-webcam", "Camera", Uri, Camera),
    r!("ms-settings:proximity", "Airplane mode", Uri, AirplaneMode),
    r!(
        "ms-settings:sound-defaultoutputproperties",
        "Properties",
        Uri,
        Properties
    ),
    r!(
        "ms-settings:websearch",
        "Video playback",
        Uri,
        VideoPlayback
    ),
    r!("ms-settings:wheel", "Video playback", Uri, VideoPlayback),
    r!(
        "ms-settings:windowsupdate-options",
        "Advanced options",
        Uri,
        WindowsUpdateAdvancedOptions
    ),
    r!("ms-settings:easeofaccess-audio", "Audio", Uri, Audio),
    r!(
        "ms-settings:easeofaccess-eyecontrol",
        "Eye control",
        Uri,
        EyeControl
    ),
    r!(
        "ms-settings:easeofaccess-hearingaids",
        "Hearing devices",
        Uri,
        HearingDevices
    ),
    r!(
        "ms-settings:easeofaccess-mouse",
        "Mouse",
        Uri,
        AccessibilityMouse
    ),
    r!(
        "ms-settings:easeofaccess-narrator-extensions",
        "Narrator",
        Uri,
        Narrator
    ),
    r!(
        "ms-settings:easeofaccess-narrator-usebrailledisplay",
        "Configure braille displays",
        Uri,
        ConfigureBrailleDisplays
    ),
    r!(
        "ms-settings:easeofaccess-narrator-mathreading",
        "Math reading",
        Uri,
        MathReading
    ),
    r!(
        "ms-settings:easeofaccess-screentint",
        "Screen tint",
        Uri,
        ScreenTint
    ),
    r!(
        "ms-settings:easeofaccess-speechrecognition",
        "Speech",
        Uri,
        AccessibilitySpeech
    ),
    r!(
        "ms-settings:easeofaccess-cursor",
        "Text cursor",
        Uri,
        TextCursor
    ),
    r!("ms-settings:assignedaccess", "Kiosk", Uri, Kiosk),
    r!(
        "ms-settings:savedpasskeys",
        "Passkeys",
        Uri,
        AccountsPasskeys
    ),
    r!(
        "ms-settings:passkeys-advancedoptions",
        "Advanced options",
        Uri,
        PasskeysAdvancedOptions
    ),
    r!(
        "ms-settings:advanced-apps",
        "Advanced app settings",
        Uri,
        AdvancedAppSettings
    ),
    r!(
        "ms-settings:maps-downloadmaps",
        "Offline maps",
        Uri,
        OfflineMaps
    ),
    r!("ms-settings:system-resume", "Resume", Uri, Resume),
    r!("ms-settings:camera", "Cameras", Uri, Cameras),
    r!(
        "ms-settings:devices-keyboard",
        "Keyboard",
        Uri,
        DevicesKeyboard
    ),
    r!(
        "ms-settings:pen-button",
        "Pen & Windows Ink",
        Uri,
        PenAndWindowsInk
    ),
    r!(
        "ms-settings:printers-autodiscovery",
        "Printers & scanners",
        Uri,
        PrintersAndScanners
    ),
    r!(
        "ms-settings:network-advancedsharing",
        "Advanced sharing settings",
        Uri,
        AdvancedSharingSettings
    ),
    r!(
        "ms-settings:network-wifisettings",
        "Manage known networks",
        Uri,
        ManageKnownNetworks
    ),
    r!(
        "ms-settings:personalization-lighting",
        "Dynamic Lighting",
        Uri,
        DynamicLighting
    ),
    r!(
        "ms-settings:personalization-start-places",
        "Folders",
        Uri,
        Folders
    ),
    r!(
        "ms-settings:personalization-textinput",
        "Text input",
        Uri,
        TextInput
    ),
    r!(
        "ms-settings:privacy-accountinfo",
        "Account info",
        Uri,
        AccountInfo
    ),
    r!(
        "ms-settings:privacy-appdiagnostics",
        "App diagnostics",
        Uri,
        AppDiagnostics
    ),
    r!(
        "ms-settings:privacy-automaticfiledownloads",
        "Automatic file downloads",
        Uri,
        AutomaticFileDownloads
    ),
    r!("ms-settings:privacy-calendar", "Calendar", Uri, Calendar),
    r!(
        "ms-settings:privacy-callhistory",
        "Call history",
        Uri,
        CallHistory
    ),
    r!("ms-settings:privacy-phonecalls", "Calls", Uri, Calls),
    r!(
        "ms-settings:privacy-webcam-spotlightactivity",
        "Camera",
        Uri,
        Camera
    ),
    r!("ms-settings:privacy-contacts", "Contacts", Uri, Contacts),
    r!(
        "ms-settings:privacy-speechtyping",
        "Custom dictionary",
        Uri,
        CustomDictionary
    ),
    r!("ms-settings:privacy-documents", "Documents", Uri, Documents),
    r!(
        "ms-settings:privacy-downloadsfolder",
        "Downloads",
        Uri,
        Downloads
    ),
    r!("ms-settings:privacy-email", "Email", Uri, Email),
    r!(
        "ms-settings:privacy-foregroundtextaccess",
        "Foreground text access",
        Uri,
        ForegroundTextAccess
    ),
    r!(
        "ms-settings:privacy-location-spotlightactivity",
        "Location",
        Uri,
        Location
    ),
    r!("ms-settings:privacy-messaging", "Messaging", Uri, Messaging),
    r!("ms-settings:privacy-musiclibrary", "Music", Uri, Music),
    r!(
        "ms-settings:privacy-notifications",
        "Notifications",
        Uri,
        PrivacyNotifications
    ),
    r!(
        "ms-settings:privacy-customdevices",
        "Other devices",
        Uri,
        OtherDevices
    ),
    r!(
        "ms-settings:privacy-passkeys",
        "Passkeys",
        Uri,
        PrivacyPasskeys
    ),
    r!("ms-settings:privacy-pictures", "Pictures", Uri, Pictures),
    r!("ms-settings:privacy-radios", "Radios", Uri, Radios),
    r!(
        "ms-settings:privacy-graphicscapturewithoutborder",
        "Screenshot borders",
        Uri,
        ScreenshotBorders
    ),
    r!(
        "ms-settings:privacy-graphicscaptureprogrammatic",
        "Screenshots and screen recording",
        Uri,
        ScreenshotsAndScreenRecording
    ),
    r!("ms-settings:privacy-speech", "Speech", Uri, PrivacySpeech),
    r!("ms-settings:privacy-tasks", "Tasks", Uri, Tasks),
    r!("ms-settings:privacy-videos", "Videos", Uri, Videos),
    r!(
        "ms-settings:privacy-voiceactivation",
        "Voice activation",
        Uri,
        VoiceActivation
    ),
    r!(
        "ms-settings:advanceddisplay",
        "Advanced display",
        Uri,
        AdvancedDisplay
    ),
    r!(
        "ms-settings:display-advancedgraphics-default",
        "Graphics",
        Uri,
        Graphics
    ),
    r!("ms-settings:display-hdr", "HDR", Uri, HDR),
    r!(
        "ms-settings:easeofaccess-colorfilter-bluelightlink",
        "Night light",
        Uri,
        NightLight
    ),
    r!(
        "ms-settings:energyrecommendations",
        "Energy recommendations",
        Uri,
        EnergyRecommendations
    ),
    r!("ms-settings:crossdevice", "Share", Uri, Share),
    r!(
        "ms-settings:sound-defaultinputproperties",
        "Properties",
        Uri,
        Properties
    ),
    r!("ms-settings:apps-volume", "Volume mixer", Uri, VolumeMixer),
    r!(
        "ms-settings:storagepolicies",
        "Storage Sense",
        Uri,
        StorageSense
    ),
    r!(
        "ms-settings:systemcomponents",
        "System components",
        Uri,
        SystemComponents
    ),
    r!(
        "ms-settings:regionlanguage-chsime-pinyin-domainlexicon",
        "Domain lexicon",
        Uri,
        DomainLexicon
    ),
    r!(
        "ms-settings:regionlanguage-chsime-pinyin-keyconfig",
        "Keys",
        Uri,
        Keys
    ),
    r!(
        "ms-settings:regionlanguage-jpnime",
        "Microsoft IME",
        Uri,
        MicrosoftIME
    ),
    r!(
        "ms-settings:regionlanguage-chsime-pinyin",
        "Microsoft Pinyin",
        Uri,
        MicrosoftPinyin
    ),
    r!(
        "ms-settings:regionlanguage-chsime-pinyin-udp",
        "User defined phrases",
        Uri,
        UserDefinedPhrases
    ),
    r!(
        "ms-settings:windowsupdate-activehours",
        "Advanced options",
        Uri,
        WindowsUpdateAdvancedOptions
    ),
    r!(
        "ms-settings:windowsupdate-optionalupdates",
        "Optional updates",
        Uri,
        OptionalUpdates
    ),
    r!(
        "ms-settings:windowsupdate-uninstallupdates",
        "Uninstall updates",
        Uri,
        UninstallUpdates
    ),
    r!("ms-settings:system", "System", Uri, System),
    r!("ms-settings:display", "Display", Uri, Display),
    r!(
        "ms-settings:display-advanced",
        "Advanced display",
        Uri,
        Display,
        "advanced-display"
    ),
    r!("ms-settings:nightlight", "Night light", Uri, NightLight),
    r!("ms-settings:sound", "Sound", Uri, Sound),
    r!(
        "ms-settings:sound-devices",
        "All sound devices",
        Uri,
        AllSoundDevices
    ),
    r!(
        "ms-settings:notifications",
        "Notifications",
        Uri,
        Notifications
    ),
    r!("ms-settings:quiethours", "Focus", Uri, Focus),
    r!("ms-settings:focus", "Focus", Uri, Notifications),
    r!("ms-settings:powersleep", "Power", Uri, Power),
    r!("ms-settings:batterysaver", "Energy saver", Uri, Power),
    r!("ms-settings:storagesense", "Storage", Uri, Storage),
    r!(
        "ms-settings:disksandvolumes",
        "Disks & volumes",
        Uri,
        DiskManagement
    ),
    r!(
        "ms-settings:savelocations",
        "Where new content is saved",
        Uri,
        WhereNewContentIsSaved
    ),
    r!(
        "ms-settings:multitasking",
        "Multitasking",
        Uri,
        Multitasking
    ),
    r!(
        "ms-settings:project",
        "Projecting to this PC",
        Uri,
        ProjectingToThisPC
    ),
    r!("ms-settings:clipboard", "Clipboard", Uri, Clipboard),
    r!(
        "ms-settings:remotedesktop",
        "Remote Desktop",
        Uri,
        RemoteDesktop
    ),
    r!("ms-settings:activation", "Activation", Uri, Activation),
    r!("ms-settings:troubleshoot", "Troubleshoot", Uri, Performance),
    r!("ms-settings:recovery", "Recovery", Uri, BackupRestore),
    r!("ms-settings:about", "About", Uri, About),
    r!("ms-settings:advanced", "Advanced", Uri, SystemAdvanced),
    r!(
        "ms-settings:developers",
        "For developers",
        Uri,
        SystemAdvanced,
        "for-developers"
    ),
    r!(
        "ms-settings:bluetooth",
        "Bluetooth & devices",
        Uri,
        Bluetooth
    ),
    r!("ms-settings:devices", "Bluetooth & devices", Uri, Bluetooth),
    r!(
        "ms-settings:connecteddevices",
        "Devices",
        Uri,
        DevicesDrivers
    ),
    r!(
        "ms-settings:printers",
        "Printers & scanners",
        Uri,
        PrintersAndScanners
    ),
    r!("ms-settings:mousetouchpad", "Mouse", Uri, Mouse),
    r!(
        "ms-settings:easeofaccess-colorfilter",
        "Color filters",
        Uri,
        ColorFilters
    ),
    r!("ms-settings:devices-touchpad", "Touchpad", Uri, Touchpad),
    r!(
        "ms-settings:pen",
        "Pen & Windows Ink",
        Uri,
        PenAndWindowsInk
    ),
    r!("ms-settings:autoplay", "AutoPlay", Uri, AutoPlay),
    r!("ms-settings:usb", "USB", Uri, USB),
    r!("ms-settings:network", "Network & internet", Uri, Network),
    r!(
        "ms-settings:network-status",
        "Network & internet",
        Uri,
        Network
    ),
    r!("ms-settings:network-ethernet", "Ethernet", Uri, Ethernet),
    r!("ms-settings:network-wifi", "Wi-Fi", Uri, WiFi),
    r!("ms-settings:network-vpn", "VPN", Uri, Vpn),
    r!("ms-settings:network-proxy", "Proxy", Uri, Network, "proxy"),
    r!(
        "ms-settings:network-airplanemode",
        "Airplane mode",
        Uri,
        AirplaneMode
    ),
    r!(
        "ms-settings:network-mobilehotspot",
        "Mobile hotspot",
        Uri,
        MobileHotspot
    ),
    r!("ms-settings:network-dialup", "Dial-up", Uri, DialUp),
    r!(
        "ms-settings:network-advancedsettings",
        "Advanced network settings",
        Uri,
        NetworkAdministration
    ),
    r!(
        "ms-settings:personalization",
        "Personalization",
        Uri,
        Personalization
    ),
    r!(
        "ms-settings:personalization-background",
        "Background",
        Uri,
        Background
    ),
    r!("ms-settings:personalization-colors", "Colors", Uri, Colors),
    r!("ms-settings:colors", "Colors", Uri, Colors),
    r!("ms-settings:themes", "Themes", Uri, Themes),
    r!("ms-settings:personalization-start", "Start", Uri, Start),
    r!("ms-settings:taskbar", "Taskbar", Uri, Taskbar),
    r!("ms-settings:fonts", "Fonts", Uri, Fonts),
    r!("ms-settings:lockscreen", "Lock screen", Uri, LockScreen),
    r!(
        "ms-settings:appsfeatures",
        "Installed apps",
        Uri,
        InstalledApps
    ),
    r!(
        "ms-settings:installed-apps",
        "Installed apps",
        Uri,
        InstalledApps
    ),
    r!(
        "ms-settings:optionalfeatures",
        "Optional features",
        Uri,
        OptionalFeatures
    ),
    r!("ms-settings:defaultapps", "Default apps", Uri, DefaultApps),
    r!("ms-settings:startupapps", "Startup", Uri, Startup),
    r!(
        "ms-settings:appsforwebsites",
        "Apps for websites",
        Uri,
        AppsForWebsites
    ),
    r!(
        "ms-settings:videoplayback",
        "Video playback",
        Uri,
        VideoPlayback
    ),
    r!("ms-settings:yourinfo", "Your info", Uri, UsersGroups),
    r!(
        "ms-settings:signinoptions",
        "Sign-in options",
        Uri,
        SignInOptions
    ),
    r!("ms-settings:otherusers", "Other users", Uri, OtherUsers),
    r!(
        "ms-settings:emailandaccounts",
        "Your accounts",
        Uri,
        YourAccounts
    ),
    r!(
        "ms-settings:workplace",
        "Access work or school",
        Uri,
        AccessWorkOrSchool
    ),
    r!(
        "ms-settings:backup",
        "Doors backup",
        Uri,
        BackupRestore,
        "file-backup"
    ),
    r!("ms-settings:dateandtime", "Date & time", Uri, DateTime),
    r!(
        "ms-settings:regionformatting",
        "Language & region",
        Uri,
        LanguageRegion
    ),
    r!(
        "ms-settings:regionlanguage",
        "Language & region",
        Uri,
        LanguageRegion
    ),
    r!("ms-settings:keyboard", "Typing", Uri, Typing),
    r!("ms-settings:typing", "Typing", Uri, Typing),
    r!("ms-settings:speech", "Speech", Uri, TimeSpeech),
    r!("ms-settings:gaming-gamebar", "Game Bar", Uri, Gaming),
    r!("ms-settings:gaming-gamemode", "Game Mode", Uri, GameMode),
    r!(
        "ms-settings:easeofaccess",
        "Accessibility",
        Uri,
        Accessibility
    ),
    r!(
        "ms-settings:easeofaccess-display",
        "Text size",
        Uri,
        TextSize
    ),
    r!(
        "ms-settings:easeofaccess-visualeffects",
        "Visual effects",
        Uri,
        VisualEffects
    ),
    r!(
        "ms-settings:easeofaccess-highcontrast",
        "Contrast themes",
        Uri,
        ContrastThemes
    ),
    r!(
        "ms-settings:easeofaccess-mousepointer",
        "Mouse pointer and touch",
        Uri,
        MousePointerAndTouch
    ),
    r!(
        "ms-settings:easeofaccess-magnifier",
        "Magnifier",
        Uri,
        Magnifier
    ),
    r!(
        "ms-settings:easeofaccess-narrator",
        "Narrator",
        Uri,
        Narrator
    ),
    r!(
        "ms-settings:easeofaccess-closedcaptioning",
        "Captions",
        Uri,
        Captions
    ),
    r!(
        "ms-settings:easeofaccess-keyboard",
        "Keyboard",
        Uri,
        AccessibilityKeyboard
    ),
    r!("ms-settings:privacy", "Privacy & security", Uri, Privacy),
    r!("ms-settings:privacy-location", "Location", Uri, Location),
    r!(
        "ms-settings:privacy-microphone",
        "Microphone",
        Uri,
        Microphone
    ),
    r!(
        "ms-settings:privacy-broadfilesystemaccess",
        "File system",
        Uri,
        PermissionsSharing
    ),
    r!(
        "ms-settings:windowsdefender",
        "Windows Security",
        Uri,
        WindowsSecurity
    ),
    r!(
        "ms-settings:search-permissions",
        "Search permissions",
        Uri,
        Search
    ),
    r!(
        "ms-settings:cortana-windowssearch",
        "Searching Doors",
        Uri,
        Search,
        "find-files"
    ),
    r!("ms-settings:windowsupdate", "Doors Update", Uri, Update),
    r!(
        "ms-settings:windowsupdate-history",
        "Update history",
        Uri,
        UpdateHistory
    ),
];
