//! Pages that replace legacy Windows administration windows (Device Manager,
//! Disk Management, Services, Registry Editor, Event Viewer, ...), written in
//! the same Windows 11 card vocabulary as every other page. Values come from
//! real providers (`Fact`), lists from the app (`List`), and every control
//! either works or says why it is unavailable.
use super::*;

/// Registry Editor (regedit): guarded by an InfoBar; the address and the
/// commands come first so their places never depend on the key's contents.
pub(super) static REGISTRY: &[Block] = &[
    Block::Info(
        "Changing the registry can stop apps or Doors from working. Change only values you understand.",
    ),
    Block::Card(row!(
        "Current key",
        "",
        "code",
        Control::Value(Fact::RegistryPath)
    )),
    Block::Card(row!(
        "Go to a key",
        "Open a key by its path, or create it",
        "search",
        Control::Command("Go to", Command::RegistryGoTo)
    )),
    Block::Card(row!(
        "Up one level",
        "Open the key that contains this one",
        "refresh",
        Control::Command("Up", Command::RegistryUp)
    )),
    Block::Card(row!(
        "Add a value",
        "Create a text, number, binary or list value in this key",
        "edit",
        Control::Command("New value", Command::RegistryNewValue)
    )),
    Block::Header("Values"),
    Block::List(List::RegistryValues),
    Block::Header("Subkeys"),
    Block::List(List::RegistryKeys),
    Block::Header("View"),
    Block::Card(row!(
        "Root key",
        "Where paths start",
        "code",
        Control::Combo(Choice::RegistryRoot)
    )),
    Block::Card(row!(
        "Registry view",
        "64-bit apps and 32-bit apps see separate copies of some keys",
        "code",
        Control::Combo(Choice::RegistryView)
    )),
    Block::Header("Manage"),
    Block::Card(row!(
        "Delete this key",
        "Removes the key and its values (a key with subkeys stays)",
        "code",
        Control::Command("Delete", Command::RegistryDeleteKey)
    )),
    Block::Card(row!(
        "Export this key",
        "Save the key and its subkeys to Documents as a .reg file",
        "code",
        Control::Command("Export", Command::RegistryExport)
    )),
    Block::Note("To rename or delete a value, choose its Edit button."),
];
/// Bluetooth & devices > Devices (Device Manager): one expander per device
/// with its connection, driver and resources, from the kernel's boot
/// inventory, the disk owner and the window server.
pub(super) static DEVICES: &[Block] = &[
    Block::Header("Input"),
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Keyboard",
            "PS/2 keyboard",
            "keyboard",
            Control::Value(Fact::Keyboard)
        ),
        hero: None,
        rows: rows![
            row!(
                "Connection",
                "",
                "",
                Control::Value(Fact::Static("PS/2 controller"))
            ),
            row!(
                "Driver",
                "",
                "",
                Control::Value(Fact::Static("Doors kernel input driver"))
            ),
            row!(
                "Keyboard layouts",
                "Installed keyboards and input methods",
                "",
                link(Page::Typing)
            ),
        ],
    }),
    Block::Expander(Expander {
        id: 2,
        header: row!("Mouse", "PS/2 pointing device", "mouse", Control::None),
        hero: None,
        rows: rows![
            row!(
                "Connection",
                "",
                "",
                Control::Value(Fact::Static("PS/2 controller"))
            ),
            row!(
                "Driver",
                "",
                "",
                Control::Value(Fact::Static("Doors kernel input driver"))
            ),
        ],
    }),
    Block::Header("Display"),
    Block::Expander(Expander {
        id: 3,
        header: row!(
            "Display adapter",
            "UEFI graphics output",
            "display",
            Control::Value(Fact::Resolution)
        ),
        hero: None,
        rows: rows![
            row!("Resolution", "", "", Control::Value(Fact::Resolution)),
            row!("Scale", "", "", Control::Value(Fact::Scale)),
            row!(
                "Driver",
                "",
                "",
                Control::Value(Fact::Static("Doors window server, software composition"))
            ),
            row!(
                "Display settings",
                "Scale, resolution and orientation",
                "",
                link(Page::Display)
            ),
        ],
    }),
    Block::Header("Storage"),
    Block::Expander(Expander {
        id: 4,
        header: row!(
            "Disk 0",
            "VirtIO block device",
            "storage",
            Control::Value(Fact::DiskCapacity)
        ),
        hero: None,
        rows: rows![
            row!("Status", "", "", Control::Value(Fact::DiskStatus)),
            row!("Write access", "", "", Control::Value(Fact::DiskAccess)),
            row!("Write cache flush", "", "", Control::Value(Fact::DiskFlush)),
            row!(
                "Disks & volumes",
                "Volumes and the storage journal on this disk",
                "",
                link(Page::DiskManagement)
            ),
        ],
    }),
    Block::Header("Processors"),
    Block::Expander(Expander {
        id: 5,
        header: row!(
            "Processors",
            "x86-64",
            "devices",
            Control::Value(Fact::ProcessorCount)
        ),
        hero: None,
        rows: rows![
            row!(
                "Architecture",
                "",
                "",
                Control::Value(Fact::Static("x86-64"))
            ),
            row!("Scheduler timer", "", "", Control::Value(Fact::TimerRate)),
        ],
    }),
    Block::Header("System"),
    Block::Expander(Expander {
        id: 6,
        header: row!(
            "Computer",
            "UEFI firmware with ACPI",
            "info",
            Control::Value(Fact::Chassis)
        ),
        hero: None,
        rows: rows![row!(
            "Firmware power profile",
            "",
            "",
            Control::Value(Fact::PowerProfile)
        )],
    }),
    Block::Header("Not available yet"),
    Block::Card(row!(
        "Audio",
        "Mono audio, audio notifications",
        "volume",
        NA
    )),
    Block::Card(row!("Network adapter", "", "network", NA)),
    Block::Card(row!("Game controllers", "", "game", NA)),
    Block::Card(row!("Update or change drivers", "", "refresh", NA)),
];
/// System > Storage > Disks & volumes (Disk Management).
pub(super) static DISKS: &[Block] = &[
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Disk 0",
            "VirtIO block device, GUID partition table",
            "storage",
            Control::Value(Fact::DiskCapacity)
        ),
        hero: None,
        rows: rows![
            row!("Status", "", "", Control::Value(Fact::DiskStatus)),
            row!(
                "Sectors",
                "512 bytes each",
                "",
                Control::Value(Fact::DiskSectors)
            ),
            row!("Write access", "", "", Control::Value(Fact::DiskAccess)),
            row!("Write cache flush", "", "", Control::Value(Fact::DiskFlush)),
        ],
    }),
    Block::Header("Volumes"),
    Block::Expander(Expander {
        id: 2,
        header: row!(
            "Doors",
            "Your files, settings and registry",
            "folder",
            Control::Value(Fact::VolumeSize)
        ),
        hero: None,
        rows: rows![
            row!("Used space", "", "", Control::Value(Fact::VolumeUsed)),
            row!("Free space", "", "", Control::Value(Fact::VolumeFree)),
            row!(
                "Location on disk",
                "",
                "",
                Control::Value(Fact::VolumeExtent)
            ),
        ],
    }),
    Block::Expander(Expander {
        id: 3,
        header: row!(
            "Storage journal",
            "Finishes or undoes interrupted writes when Doors starts",
            "restore",
            Control::Value(Fact::JournalSize)
        ),
        hero: None,
        rows: rows![row!(
            "Location on disk",
            "",
            "",
            Control::Value(Fact::JournalExtent)
        )],
    }),
    Block::Header("More options"),
    Block::Card(row!(
        "Create a virtual hard disk (VHD)",
        "Create and mount a VHD or VHDX to your system",
        "storage",
        NA
    )),
    Block::Card(row!("Format, resize or create volumes", "", "storage", NA)),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Storage",
        "Storage space and cleanup",
        "storage",
        link(Page::Storage)
    )),
    Block::Card(row!(
        "Devices",
        "Disks, display, processors and input devices",
        "devices",
        link(Page::DevicesDrivers)
    )),
];
/// Network & internet > Advanced network settings (Network Connections,
/// Network and Sharing Center, Internet Options, Firewall).
pub(super) static NETWORK_ADVANCED: &[Block] = &[
    Block::Header("Network adapters"),
    Block::Expander(Expander {
        id: 2,
        header: row!(
            "Ethernet",
            "Red Hat VirtIO Ethernet Adapter",
            "ethernet",
            Control::Toggle(Toggle::Adapter)
        ),
        hero: None,
        rows: rows![
            row!("Status", "", "", Control::Value(Fact::Net(NetFact::State))),
            row!(
                "Physical address (MAC)",
                "",
                "",
                Control::Value(Fact::Net(NetFact::Mac))
            )
        ],
    }),
    Block::Header("More settings"),
    Block::Card(row!(
        "Advanced sharing settings",
        "Change network discovery and sharing settings",
        "share",
        NA
    )),
    Block::Card(row!(
        "Data usage",
        "Traffic on the Ethernet adapter since startup",
        "diagnostic",
        Control::Value(Fact::Net(NetFact::Traffic))
    )),
    Block::Expander(Expander {
        id: 3,
        header: row!(
            "Hardware and connection properties",
            "Addresses, DNS servers and the adapter",
            "info",
            Control::Value(Fact::Net(NetFact::State))
        ),
        hero: None,
        rows: rows![
            row!(
                "Description",
                "",
                "",
                Control::Value(Fact::Net(NetFact::Adapter))
            ),
            row!(
                "Physical address (MAC)",
                "",
                "",
                Control::Value(Fact::Net(NetFact::Mac))
            ),
            row!(
                "IPv6 address",
                "",
                "",
                Control::Value(Fact::Net(NetFact::Ipv6Address))
            ),
            row!(
                "Link-local IPv6 address",
                "",
                "",
                Control::Value(Fact::Net(NetFact::LinkLocal6))
            ),
            row!(
                "IPv6 DNS servers",
                "",
                "",
                Control::Value(Fact::Net(NetFact::Dns6))
            ),
            row!(
                "IPv4 address",
                "",
                "",
                Control::Value(Fact::Net(NetFact::Address))
            ),
            row!(
                "IPv4 default gateway",
                "",
                "",
                Control::Value(Fact::Net(NetFact::Gateway))
            ),
            row!(
                "IPv4 DNS servers",
                "",
                "",
                Control::Value(Fact::Net(NetFact::Dns))
            ),
            row!(
                "IP assignment",
                "",
                "",
                Control::Value(Fact::Net(NetFact::Assignment))
            )
        ],
    }),
    Block::Card(row!(
        "Network reset",
        "Turn the adapter back on with automatic (DHCP) addresses and DNS",
        "refresh",
        Control::Command("Reset now", Command::NetworkReset)
    )),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Firewall",
        "Doors Defender Firewall: network profiles and incoming connections",
        "shield",
        link(Page::Firewall)
    )),
    Block::Card(row!("Internet options", "", "globe", NA)),
];
/// System > Advanced > Services (services.msc): the services Doors runs,
/// as Init reports them, and Task Manager for processes.
pub(super) static SERVICES: &[Block] = &[
    Block::Card(row!(
        "Task Manager",
        "See the apps and background processes running now",
        "diagnostic",
        Control::Command("Open Task Manager", Command::OpenTaskManager)
    )),
    Block::Header("Installed services"),
    Block::List(List::InstalledServices),
    Block::Header("Doors services"),
    Block::Card(row!(
        "Storage",
        "Files, settings and the registry",
        "storage",
        Control::Value(Fact::Service(0))
    )),
    Block::Card(row!(
        "Session",
        "Sign-in and permission to start apps",
        "people",
        Control::Value(Fact::Service(1))
    )),
    Block::Card(row!(
        "Console",
        "Text input and output for console apps",
        "code",
        Control::Value(Fact::Service(2))
    )),
    Block::Card(row!(
        "Window",
        "Windows, the desktop and the taskbar",
        "display",
        Control::Value(Fact::Service(3))
    )),
    Block::Card(row!(
        "Service status",
        "Checked when you open this page",
        "refresh",
        Control::Command("Refresh", Command::Refresh)
    )),
    Block::Note("Starting, stopping and configuring services isn't available."),
];
/// System > Advanced > Environment variables (sysdm.cpl Environment
/// Variables): user variables in HKEY_CURRENT_USER\Environment.
pub(super) static ENVIRONMENT: &[Block] = &[
    Block::Info(
        "Variables you save here are stored for your account. Apps still start with the variables Doors gives them.",
    ),
    Block::Header("User variables"),
    Block::Card(row!(
        "Add a variable",
        "Save a name and a value for your account",
        "edit",
        Control::Command("New", Command::NewVariable)
    )),
    Block::List(List::UserVariables),
    Block::Header("Variables Settings started with"),
    Block::List(List::LaunchVariables),
    Block::Note("System variables and deleting variables aren't available."),
];
/// System > Advanced > System configuration (msconfig): each tab's home.
pub(super) static SYSTEM_CONFIGURATION: &[Block] = &[
    Block::Card(row!(
        "Startup selection",
        "Doors always starts normally; diagnostic startup isn't available",
        "power",
        Control::Value(Fact::Static("Normal startup"))
    )),
    Block::Header("Startup and services"),
    Block::Card(row!(
        "Startup apps",
        "Apps that open automatically when you sign in",
        "apps",
        link(Page::Apps)
    )),
    Block::Card(row!(
        "Services",
        "Background services Doors runs",
        "settings",
        link(Page::Services)
    )),
    Block::Header("Boot"),
    Block::Card(row!(
        "Recovery",
        "Advanced startup and how your files are protected",
        "restore",
        link(Page::BackupRestore)
    )),
    Block::Card(row!("Boot options", "", "settings", NA)),
    Block::Header("Tools"),
    Block::Card(row!(
        "Advanced",
        "Administrative tools and developer options",
        "code",
        link(Page::SystemAdvanced)
    )),
];
/// System > Advanced > Policies (gpedit.msc, secpol.msc, rsop.msc).
pub(super) static POLICIES: &[Block] = &[
    Block::Card(row!(
        "Applied policies",
        "Doors doesn't use Group Policy: settings you change here apply directly",
        "shield",
        Control::Value(Fact::Static("None"))
    )),
    Block::Card(row!("Local security policy", "", "shield", NA)),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Accounts",
        "Your info and sign-in options",
        "people",
        link(Page::Accounts)
    )),
    Block::Card(row!(
        "Privacy & security",
        "Permissions and security",
        "shield",
        link(Page::Privacy)
    )),
];
/// System > Advanced > Event logs (Event Viewer).
pub(super) static EVENT_LOGS: &[Block] = &[
    Block::Intro(
        "The newest events Windows logs keep: apps write to the Application log, services and the system to the System log. wevtutil and eventcreate in Terminal work with the same logs.",
    ),
    Block::Header("Application"),
    Block::List(List::ApplicationEvents),
    Block::Header("System"),
    Block::List(List::SystemEvents),
    Block::Header("Settings activity"),
    Block::List(List::Events),
];
/// System > Advanced > Scheduled tasks (Task Scheduler).
pub(super) static SCHEDULED_TASKS: &[Block] = &[
    Block::Header("Scheduled tasks"),
    Block::List(List::ScheduledTasks),
    Block::Card(row!(
        "Create a task",
        "Use schtasks /create in Terminal to run a program at a set time",
        "edit",
        Control::Command("Open", Command::OpenTerminal)
    )),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Startup apps",
        "Apps that open automatically when you sign in",
        "apps",
        link(Page::Apps)
    )),
    Block::Card(row!(
        "Services",
        "Background services Doors runs",
        "settings",
        link(Page::Services)
    )),
];
/// Accounts > Your info (User Accounts, Local Users and Groups).
pub(super) static YOUR_INFO: &[Block] = &[
    Block::Hero(Hero::Account),
    Block::Header("Account details"),
    Block::Card(row!(
        "Account type",
        "Local account",
        "contact",
        Control::Value(Fact::AccountType)
    )),
    Block::Card(row!(
        "User ID",
        "Your account's number on this device",
        "contact",
        Control::Value(Fact::Uid)
    )),
    Block::Card(row!(
        "Groups",
        "Groups your account belongs to",
        "people",
        Control::Value(Fact::Groups)
    )),
    Block::Card(row!(
        "Session",
        "This sign-in",
        "history",
        Control::Value(Fact::Session)
    )),
    Block::Card(row!(
        "Integrity level",
        "How much Doors trusts the apps you start",
        "shield",
        Control::Value(Fact::Integrity)
    )),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Other users",
        "Add or remove accounts on this PC",
        "people",
        link(Page::OtherUsers)
    )),
    Block::Card(row!(
        "Accounts",
        "Sign-in options and other account settings",
        "people",
        link(Page::Accounts)
    )),
];
/// Privacy & security > Microphone, as Windows 11 26H2: the access switches
/// write the MICROPHONE preference (bits set mean off), which the shell
/// pushes to door.audio; the lists come from door.audio's capture records.
pub(super) static MICROPHONE: &[Block] = &[
    Block::Intro(
        "If an app is using your microphone, you'll see a microphone icon on your Doors taskbar. Some desktop apps might not appear on this page or be affected by these settings.",
    ),
    Block::Card(row!(
        "Microphone access",
        "Anyone using this device can choose if their apps have microphone access when this is on",
        "microphone",
        Control::Toggle(Toggle::PrefOff {
            id: p::MICROPHONE,
            bit: door_service_protocol::audio::capture::ACCESS_OFF
        })
    )),
    Block::Card(row!(
        "Let apps access your microphone",
        "Choose which apps can access your microphone",
        "list",
        Control::Toggle(Toggle::PrefOff {
            id: p::MICROPHONE,
            bit: door_service_protocol::audio::capture::APPS_OFF
        })
    )),
    Block::Card(row!(
        "Let desktop apps access your microphone",
        "Desktop apps that have previously accessed your microphone are listed here",
        "",
        Control::Toggle(Toggle::PrefOff {
            id: p::MICROPHONE,
            bit: door_service_protocol::audio::capture::DESKTOP_OFF
        })
    )),
    Block::List(List::MicrophoneApps),
    Block::Card(row!(
        "Recent activity",
        "Review microphone access from the last 7 days. The most recent access data is also captured in other sections in this page",
        "history",
        Control::Value(Fact::Audio(AudioFact::Requests))
    )),
    Block::List(List::MicrophoneActivity),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Sound",
        "Volume levels, output, input, sound devices",
        "volume",
        link(Page::Sound)
    )),
];
/// Privacy & security > File system (file permissions and sharing).
pub(super) static FILE_SYSTEM: &[Block] = &[
    Block::Card(row!(
        "File system access",
        "Apps you start can open the files your account can open",
        "folder",
        ON
    )),
    Block::Card(row!(
        "Check access to a file or folder",
        "See what Doors reports for a path you choose",
        "shield",
        Control::Command("Check", Command::CheckAccess)
    )),
    Block::Header("Last check"),
    Block::Card(row!(
        "Path",
        "",
        "folder",
        Control::Value(Fact::CheckedPath)
    )),
    Block::Card(row!("Type", "", "info", Control::Value(Fact::CheckedKind))),
    Block::Card(row!(
        "Size",
        "",
        "storage",
        Control::Value(Fact::CheckedSize)
    )),
    Block::Note("Changing permissions and sharing folders isn't available."),
];
/// System > Troubleshoot (troubleshooters; Performance and Resource
/// Monitor become Task Manager, as in Windows).
pub(super) static TROUBLESHOOT: &[Block] = &[
    Block::Card(row!(
        "System check",
        "Check the native interface, the clock and storage",
        "repair",
        Control::Command("Run", Command::RunChecks)
    )),
    Block::Header("Results"),
    Block::Card(row!(
        "Native interface",
        "",
        "code",
        Control::Value(Fact::Check(0))
    )),
    Block::Card(row!("Clock", "", "history", Control::Value(Fact::Check(1)))),
    Block::Card(row!(
        "Storage",
        "",
        "storage",
        Control::Value(Fact::Check(2))
    )),
    Block::Header("Other tools"),
    Block::Card(row!(
        "Performance",
        "CPU, memory and disk activity in Task Manager",
        "diagnostic",
        Control::Command("Open Task Manager", Command::OpenTaskManager)
    )),
    Block::Card(row!("Other troubleshooters", "", "repair", NA)),
];
/// System > Power > Power options (Power Options control panel).
pub(super) static POWER_OPTIONS: &[Block] = &[
    Block::Card(row!("Power plan", "", "power", NA)),
    Block::Card(row!("Choose what the power buttons do", "", "power", NA)),
    Block::Card(row!("Turn off the display", "", "display", NA)),
    Block::Card(row!("Sleep", "", "quiet", NA)),
    Block::Card(row!("Hibernate", "", "quiet", NA)),
    Block::Note("Shut down and restart from Start > Power."),
];
/// System > Recovery (Recovery, System Restore, Backup and Restore,
/// File History).
pub(super) static RECOVERY: &[Block] = &[
    Block::Card(row!(
        "Fix problems without resetting your PC",
        "Run the system check first",
        "repair",
        link(Page::Performance)
    )),
    Block::Header("Recovery options"),
    Block::Card(row!(
        "Reset this PC",
        "",
        "restore",
        Control::Button("Reset PC")
    )),
    Block::Card(row!(
        "Advanced startup",
        "Restart your device to change startup settings, including starting from a disc or USB drive",
        "power",
        Control::Button("Restart now")
    )),
    Block::Card(row!(
        "Point-in-time restore",
        "Turn on and manage restore points to recover your PC to an earlier point in time",
        "history",
        NA
    )),
    Block::Card(row!("Recovery drive", "", "usb", NA)),
    Block::Header("Your files"),
    Block::Card(row!(
        "Storage journal",
        "Finishes or undoes interrupted writes when Doors starts",
        "storage",
        Control::Value(Fact::JournalSize)
    )),
    Block::Card(row!("File backup", "", "cloud", NA)),
];
/// System > Advanced > Developer tools.
pub(super) static DEVELOPER_TOOLS: &[Block] = &[
    Block::Header("About this build"),
    Block::Card(row!(
        "Native interface",
        "The version apps are built for",
        "code",
        Control::Value(Fact::NativeAbi)
    )),
    Block::Card(row!(
        "Scheduler timer",
        "",
        "history",
        Control::Value(Fact::TimerRate)
    )),
    Block::Header("Tools"),
    Block::Card(row!(
        "Terminal",
        "Run commands and native apps",
        "code",
        Control::Command("Open", Command::OpenTerminal)
    )),
    Block::Card(row!(
        "Registry Editor",
        "Browse and change registry keys and values",
        "code",
        link(Page::Registry)
    )),
    Block::Card(row!(
        "Environment variables",
        "Variables saved for your account",
        "code",
        link(Page::Environment)
    )),
    Block::Card(row!(
        "Troubleshoot",
        "Run the system check",
        "repair",
        link(Page::Performance)
    )),
];
/// Personalization > Fonts (Fonts control panel, shell:Fonts).
pub(super) static FONTS: &[Block] = &[
    Block::Card(row!(
        "Drag and drop to install",
        "Drag font files from File Explorer or your desktop and drop them here to install fonts on your device.",
        "font",
        Control::Button("Browse and install fonts")
    )),
    Block::Header("Related settings"),
    Block::Card(row!("Adjust ClearType text", "", "font", NA)),
    Block::Card(row!("Find a character", "", "font", NA)),
    Block::Card(row!("Download fonts for all languages", "", "g-e118", NA)),
    Block::Header("Available fonts"),
    Block::Card(row!(
        "Interface text",
        "Segoe UI Variable, or Selawik where it isn't included",
        "font",
        Control::None
    )),
    Block::Card(row!(
        "Symbols and icons",
        "Segoe Fluent Icons",
        "font",
        Control::None
    )),
    Block::Card(row!(
        "Terminal text",
        "Cascadia Mono",
        "font",
        Control::None
    )),
    Block::Header("Loaded when an app needs them"),
    Block::Card(row!("Emoji", "Segoe UI Emoji", "font", Control::None)),
    Block::Card(row!(
        "Chinese, Japanese and Korean",
        "Microsoft YaHei, Microsoft JhengHei, Yu Gothic, Malgun Gothic",
        "font",
        Control::None
    )),
    Block::Card(row!(
        "Indic and historic scripts",
        "Nirmala UI, Segoe UI Historic",
        "font",
        Control::None
    )),
    Block::Note("Language fonts are part of builds that include them."),
];

/// System > Clipboard (ms-settings:clipboard).
pub(super) static CLIPBOARD: &[Block] = &[
    Block::Card(row!(
        "Clipboard history",
        "Save multiple items to the clipboard to use later. Press the Doors logo key + V to view your clipboard history and paste from it.",
        "clipboard",
        Control::Toggle(Toggle::Pref {
            id: p::CLIPBOARD,
            bit: p::CLIPBOARD_HISTORY
        })
    )),
    Block::Card(row!("Suggested actions", "", "clipboard", OFF)),
    Block::Card(row!(
        "Clipboard history across your devices",
        "Paste text from your PC to supported devices This feature is only available if your Macrohard account is syncing with Doors",
        "cloud",
        OFF
    )),
    Block::Card(row!(
        "Clear clipboard data",
        "Clear everything except pinned items",
        "clipboard",
        Control::Command("Clear", Command::ClearClipboard)
    )),
];
/// Accounts > Sign-in options (ms-settings:signinoptions).
pub(super) static SIGN_IN_OPTIONS: &[Block] = &[
    Block::Header("Ways to sign in"),
    Block::Card(row!(
        "PIN",
        "Sign in with a PIN",
        "key",
        Control::Command("Change PIN", Command::ChangePin)
    )),
    Block::Card(row!(
        "Password",
        "Sign in with your account's password",
        "key",
        Control::Command("Change", Command::ChangePassword)
    )),
    Block::Card(row!("Facial recognition", "", "camera", NA)),
    Block::Card(row!("Fingerprint recognition", "", "key", NA)),
    Block::Card(row!(
        "Security key",
        "Sign in with a physical security key",
        "usb",
        NA
    )),
    Block::Header("Additional settings"),
    Block::Card(row!(
        "If you've been away, when should Doors require you to sign in again?",
        "",
        "people",
        Control::Combo(Choice::SignInAway)
    )),
    Block::Card(row!(
        "Dynamic lock",
        "Automatically lock your device when you’re away",
        "phone",
        NA
    )),
    Block::Card(row!(
        "Automatically save my restartable apps and restart them when I sign back in",
        "",
        "restore",
        OFF
    )),
];
/// Personalization > Lock screen (ms-settings:lockscreen).
pub(super) static LOCK_SCREEN: &[Block] = &[
    Block::Hero(Hero::LockPreview),
    Block::Card(row!(
        "Personalize your lock screen",
        "",
        "g-ee3f",
        Control::Combo(Choice::LockScreen)
    )),
    // Doors always shows the lock picture behind sign-in.
    Block::Card(row!(
        "Show the lock screen background picture on the sign-in screen",
        "",
        "",
        ON
    )),
    Block::Header("Related settings"),
    Block::Card(row!("Screen timeout", "", "display", link(Page::Power))),
    Block::Card(row!("Screen saver", "", "display", NA)),
    // Doors extra: where the lock and sign-in rules live.
    Block::Card(row!(
        "Sign-in options",
        "PIN, password, when to sign in again",
        "key",
        link(Page::SignInOptions)
    )),
];
/// Apps > Default apps (ms-settings:defaultapps, Default Programs).
pub(super) static DEFAULT_APPS: &[Block] = &[
    Block::Info(
        "Choose the app that opens each type of file. Until you do, Doors uses its own apps.",
    ),
    Block::Header("Choose defaults by file type"),
    Block::List(List::FileTypes),
    Block::Header("Related settings"),
    Block::Card(row!(
        "Installed apps",
        "Uninstall and manage apps on your PC",
        "apps",
        link(Page::InstalledApps)
    )),
];
/// System > Multitasking (ms-settings:multitasking).
pub(super) static MULTITASKING: &[Block] = &[
    Block::Expander(Expander {
        id: 1,
        header: row!(
            "Snap windows",
            "Arrange windows side by side with the keyboard, the mouse or snap layouts",
            "multitask",
            Control::Toggle(Toggle::PrefOff {
                id: p::MULTITASK,
                bit: p::multitask::NO_SNAP
            })
        ),
        hero: None,
        rows: rows![
            row!(
                "Show snap layouts when I hover over a window's maximize button",
                "",
                "",
                Control::Toggle(Toggle::PrefOff {
                    id: p::MULTITASK,
                    bit: p::multitask::NO_HOVER_LAYOUTS
                })
            ),
            row!(
                "Show snap layouts when I drag a window to the top of my screen",
                "",
                "",
                Control::Toggle(Toggle::PrefOff {
                    id: p::MULTITASK,
                    bit: p::multitask::NO_DRAG_TOP
                })
            ),
            row!(
                "Show what I can snap next to it when I snap a window",
                "",
                "",
                Control::Toggle(Toggle::PrefOff {
                    id: p::MULTITASK,
                    bit: p::multitask::NO_SNAP_ASSIST
                })
            ),
        ],
    }),
    Block::Card(row!(
        "Desktops",
        "Task view (Doors logo key + Tab) adds and switches desktops",
        "taskview",
        Control::None
    )),
    Block::Card(row!(
        "Title bar window shake",
        "When I grab a window's title bar and shake it, minimize all other windows",
        "multitask",
        Control::Toggle(Toggle::Pref {
            id: p::MULTITASK,
            bit: p::multitask::SHAKE
        })
    )),
];
/// Bluetooth & devices > Mouse (ms-settings:mousetouchpad, main.cpl).
pub(super) static MOUSE: &[Block] = &[
    Block::Card(row!(
        "Primary mouse button",
        "",
        "mouse",
        Control::Combo(Choice::PrimaryButton)
    )),
    Block::Card(row!(
        "Mouse pointer speed",
        "1 is slowest, 20 is fastest",
        "pointer",
        Control::Combo(Choice::PointerSpeed)
    )),
    Block::Card(row!(
        "Enhance pointer precision",
        "Pointer acceleration: faster movements go further",
        "pointer",
        Control::Toggle(Toggle::PrefOff {
            id: p::MOUSE,
            bit: p::mouse::NO_PRECISION
        })
    )),
    Block::Header("Scrolling"),
    Block::Card(row!(
        "Lines to scroll at a time",
        "Each notch of the wheel",
        "list",
        Control::Combo(Choice::ScrollLines)
    )),
    Block::Card(row!(
        "Scroll inactive windows when hovering over them",
        "",
        "list",
        Control::Toggle(Toggle::PrefOff {
            id: p::MOUSE,
            bit: p::mouse::NO_INACTIVE_SCROLL
        })
    )),
];
/// Accessibility > Color filters (ms-settings:easeofaccess-colorfilter).
pub(super) static COLOR_FILTERS: &[Block] = &[
    Block::Card(row!(
        "Color filters",
        "Change the colors on your screen to make photos and colors easier to see",
        "color",
        Control::Toggle(Toggle::Pref {
            id: p::VISION,
            bit: p::vision::FILTER
        })
    )),
    Block::Card(row!(
        "Filter",
        "",
        "color",
        Control::Combo(Choice::FilterKind)
    )),
];
