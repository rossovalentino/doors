//! Process identity, grouping, sorting and value formatting. Everything here is
//! derived from the sanitized kernel snapshot; nothing is invented.
use alloc::{format, string::String, vec::Vec};
use door_abi::platform::{manifest, mode};
use door_abi::telemetry::{MODE_X86, Process};

/// Task Manager's process groups, in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Apps,
    Background,
    Windows,
    /// Every process in one category ("Group by type" off).
    All,
}
impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Self::Apps => "Apps",
            Self::Background => "Background processes",
            Self::Windows => "Doors processes",
            Self::All => "Processes",
        }
    }
}
/// Row icon source; painted by the presentation layer at native density.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessIcon {
    Explorer,
    Settings,
    Terminal,
    Notes,
    Preview,
    TaskManager,
    Calculator,
    Clock,
    Paint,
    SnippingTool,
    Weather,
    Magnifier,
    Narrator,
    OnScreenKeyboard,
    VoiceAccess,
    Service,
    Desktop,
    Application,
    Console,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    /// Friendly name (Processes, Users, App history).
    pub name: &'static str,
    /// Executable image name (Details).
    pub image: &'static str,
    pub group: Group,
    pub icon: ProcessIcon,
}
pub fn identity(p: &Process) -> Identity {
    let m = p.manifest as u64;
    let (name, image, group, icon) = match m {
        manifest::INIT => ("Init", "door-init", Group::Windows, ProcessIcon::Service),
        manifest::STORAGE => (
            "Storage service",
            "door-storage",
            Group::Windows,
            ProcessIcon::Service,
        ),
        manifest::SESSION => (
            "Session manager",
            "door-session",
            Group::Windows,
            ProcessIcon::Service,
        ),
        manifest::CONSOLE => (
            "Console host",
            "door-console",
            Group::Windows,
            ProcessIcon::Console,
        ),
        manifest::NETWORK => (
            "Network service",
            "door-network",
            Group::Windows,
            ProcessIcon::Service,
        ),
        manifest::AUDIO => (
            "Doors Audio",
            "door-audio",
            Group::Windows,
            ProcessIcon::Service,
        ),
        manifest::WORKLOAD => (
            "Native workload",
            "door-workload",
            Group::Background,
            ProcessIcon::Application,
        ),
        manifest::WINDOWS | manifest::WINDOWS_BASE => (
            "Windows compatibility process",
            "windows-process",
            Group::Background,
            ProcessIcon::Console,
        ),
        manifest::SHELL => (
            "Shell",
            "door-shell",
            Group::Background,
            ProcessIcon::Console,
        ),
        manifest::APPLICATION => (
            "Native application",
            "door-app",
            Group::Background,
            ProcessIcon::Application,
        ),
        // A 32-bit program is named with "(32 bit)", as Windows 11's Task
        // Manager names WOW64 processes (P17).
        manifest::WINDOWS_APPLICATION if p.mode & MODE_X86 != 0 => (
            "Windows application (32 bit)",
            "windows-app",
            Group::Background,
            ProcessIcon::Console,
        ),
        manifest::WINDOWS_APPLICATION => (
            "Windows application",
            "windows-app",
            Group::Background,
            ProcessIcon::Console,
        ),
        // A Windows GUI image (PE subsystem 2) has its own top-level windows.
        manifest::WINDOWS_GUI if p.mode & MODE_X86 != 0 => (
            "Windows application (32 bit)",
            "windows-gui-app",
            Group::Apps,
            ProcessIcon::Application,
        ),
        manifest::WINDOWS_GUI => (
            "Windows application",
            "windows-gui-app",
            Group::Apps,
            ProcessIcon::Application,
        ),
        manifest::WINDOW => (
            "Desktop Window Manager",
            "door-window",
            Group::Windows,
            ProcessIcon::Desktop,
        ),
        manifest::WORKSPACE => match p.mode as u64 {
            mode::SETTINGS => (
                "Settings",
                "door-settings",
                Group::Apps,
                ProcessIcon::Settings,
            ),
            mode::TERMINAL => (
                "Terminal",
                "door-terminal",
                Group::Apps,
                ProcessIcon::Terminal,
            ),
            mode::SESSION => ("Notes", "door-notes", Group::Apps, ProcessIcon::Notes),
            // Start file search: windowless, started by Init per session.
            mode::SEARCH_HOST => (
                "Search indexer",
                "door-search",
                Group::Background,
                ProcessIcon::Service,
            ),
            _ => (
                "File Explorer",
                "door-files",
                Group::Apps,
                ProcessIcon::Explorer,
            ),
        },
        manifest::PREVIEW => ("Preview", "door-preview", Group::Apps, ProcessIcon::Preview),
        manifest::CALCULATOR => (
            "Calculator",
            "door-inbox",
            Group::Apps,
            ProcessIcon::Calculator,
        ),
        manifest::CLOCK => ("Clock", "door-inbox", Group::Apps, ProcessIcon::Clock),
        manifest::PAINT => ("Paint", "door-inbox", Group::Apps, ProcessIcon::Paint),
        manifest::SNIPPING_TOOL => (
            "Snipping Tool",
            "door-inbox",
            Group::Apps,
            ProcessIcon::SnippingTool,
        ),
        manifest::WEATHER => ("Weather", "door-inbox", Group::Apps, ProcessIcon::Weather),
        manifest::MAGNIFIER => (
            "Magnifier",
            "door-inbox",
            Group::Apps,
            ProcessIcon::Magnifier,
        ),
        manifest::NARRATOR => ("Narrator", "door-inbox", Group::Apps, ProcessIcon::Narrator),
        manifest::ON_SCREEN_KEYBOARD => (
            "On-Screen Keyboard",
            "door-inbox",
            Group::Apps,
            ProcessIcon::OnScreenKeyboard,
        ),
        manifest::VOICE_ACCESS => (
            "Voice access",
            "door-inbox",
            Group::Apps,
            ProcessIcon::VoiceAccess,
        ),
        // Windows lists its text input host the same way: a background
        // process of the signed-in session, started at sign-in.
        manifest::IME => (
            "Doors Input Method Editor",
            "door-ime",
            Group::Background,
            ProcessIcon::Service,
        ),
        manifest::MONITOR => (
            "Task Manager",
            "door-monitor",
            Group::Apps,
            ProcessIcon::TaskManager,
        ),
        // The restartable taskbar/Start/desktop process, listed the way
        // Windows lists explorer.exe among its Windows processes.
        manifest::DESKTOP_SHELL => (
            "Desktop shell",
            "door-desktop-shell",
            Group::Windows,
            ProcessIcon::Explorer,
        ),
        _ => (
            "Native process",
            "door-process",
            Group::Background,
            ProcessIcon::Application,
        ),
    };
    Identity {
        name,
        image,
        group,
        icon,
    }
}
/// Kernel task states as reported by the snapshot (see kernel monitor).
pub fn status(p: &Process) -> &'static str {
    match p.state {
        4 => "Suspended",
        5 => "Exiting",
        _ => "",
    }
}
pub fn details_status(p: &Process) -> &'static str {
    match p.state {
        4 => "Suspended",
        5 => "Exiting",
        _ => "Running",
    }
}
/// Isolation column: the execution boundary the kernel actually applies, from
/// the authenticated identity in the snapshot. Windows-personality processes
/// run under the PE compatibility personality; system services are session-0
/// identities; everything else is a native process confined to the signed-in
/// user's session with its manifest's capability policy.
pub fn isolation(p: &Process) -> &'static str {
    match p.manifest as u64 {
        manifest::WINDOWS
        | manifest::WINDOWS_BASE
        | manifest::WINDOWS_APPLICATION
        | manifest::WINDOWS_GUI => "Windows personality",
        _ if system_account(p.uid) => "System service",
        _ => "User session",
    }
}
/// Service processes run under small system identities; signed-in users do not.
pub fn system_account(uid: u32) -> bool {
    uid < 1000
}
pub fn user_name(uid: u32) -> &'static str {
    if system_account(uid) {
        "SYSTEM"
    } else {
        "Local account"
    }
}
/// Columns that can order process tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Name,
    Status,
    Cpu,
    Memory,
    Pid,
    User,
    Threads,
    Description,
    CpuTime,
    Isolation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub key: Key,
    pub descending: bool,
}
impl Sort {
    pub const NAME: Self = Self {
        key: Key::Name,
        descending: false,
    };
    /// Numeric columns open largest-first, text columns A to Z (as Windows).
    pub fn toggled(self, key: Key) -> Self {
        if self.key == key {
            Self {
                key,
                descending: !self.descending,
            }
        } else {
            Self {
                key,
                descending: matches!(key, Key::Cpu | Key::Memory | Key::Threads | Key::CpuTime),
            }
        }
    }
}
/// Decimal with one fractional digit from a permille value: 25 -> "2.5%".
pub fn percent(permille: u32) -> String {
    if permille == 0 {
        "0%".into()
    } else {
        format!("{}.{}%", permille / 10, permille % 10)
    }
}
fn grouped(mut n: u64) -> String {
    let mut parts = Vec::new();
    loop {
        if n < 1000 {
            parts.push(format!("{n}"));
            break;
        }
        parts.push(format!("{:03}", n % 1000));
        n /= 1000;
    }
    parts.reverse();
    parts.join(",")
}
/// Task Manager memory column: megabytes with one decimal and grouping.
pub fn megabytes(bytes: u64) -> String {
    let tenths = (bytes as u128 * 10 / (1024 * 1024)) as u64;
    format!("{}.{} MB", grouped(tenths / 10), tenths % 10)
}
/// Details memory column: "26,624 K".
pub fn kilobytes(bytes: u64) -> String {
    format!("{} K", grouped(bytes / 1024))
}
/// "1.2 GB" style capacity used on the Performance page.
pub fn gigabytes(bytes: u64) -> String {
    let tenths = (bytes as u128 * 10 / (1u128 << 30)) as u64;
    format!("{}.{} GB", tenths / 10, tenths % 10)
}
pub fn memory_short(bytes: u64) -> String {
    if bytes >= 1 << 30 {
        gigabytes(bytes)
    } else {
        format!("{} MB", bytes / (1024 * 1024))
    }
}
/// Up time / CPU time from 100 Hz timer ticks, "d:hh:mm:ss" or "h:mm:ss".
pub fn duration(ticks: u64, days: bool) -> String {
    let seconds = ticks / 100;
    let (d, h, m, s) = (
        seconds / 86400,
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60,
    );
    if days {
        format!("{d}:{h:02}:{m:02}:{s:02}")
    } else {
        format!("{}:{m:02}:{s:02}", d * 24 + h)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn values_format_like_task_manager() {
        assert_eq!(percent(0), "0%");
        assert_eq!(percent(25), "2.5%");
        assert_eq!(percent(1000), "100.0%");
        assert_eq!(megabytes(56 * 1024 * 1024), "56.0 MB");
        assert_eq!(megabytes(1536 * 1024 * 1024 + 512 * 1024), "1,536.5 MB");
        assert_eq!(gigabytes(3 << 29), "1.5 GB");
        assert_eq!(duration(100 * 3725, true), "0:01:02:05");
        assert_eq!(duration(100 * 90061, false), "25:01:01");
        assert_eq!(
            Sort::NAME.toggled(Key::Cpu),
            Sort {
                key: Key::Cpu,
                descending: true
            }
        );
        assert_eq!(
            Sort::NAME.toggled(Key::Name),
            Sort {
                key: Key::Name,
                descending: true
            }
        );
    }
    #[test]
    fn every_catalog_manifest_has_a_named_group_and_image() {
        for m in (1..=14u32).chain([16]) {
            for md in [
                0,
                mode::SETTINGS,
                mode::TERMINAL,
                mode::SESSION,
                mode::DESKTOP,
                mode::SEARCH_HOST,
            ] {
                let p = Process {
                    pid: 1,
                    instance: 1,
                    manifest: m,
                    mode: md as u32,
                    ..Process::default()
                };
                let id = identity(&p);
                assert!(
                    !id.name.is_empty() && id.image.starts_with(|c: char| c.is_ascii_lowercase())
                );
            }
        }
        let host = Process {
            manifest: 12,
            mode: mode::SEARCH_HOST as u32,
            ..Process::default()
        };
        assert_eq!(identity(&host).group, Group::Background);
        let tm = Process {
            manifest: 14,
            ..Process::default()
        };
        assert_eq!(identity(&tm).name, "Task Manager");
        assert_eq!(identity(&tm).group, Group::Apps);
        let shell = Process {
            manifest: manifest::DESKTOP_SHELL as u32,
            ..Process::default()
        };
        assert_eq!(
            (
                identity(&shell).name,
                identity(&shell).image,
                identity(&shell).group
            ),
            ("Desktop shell", "door-desktop-shell", Group::Windows)
        );
        let ime = Process {
            manifest: 16,
            mode: mode::DESKTOP as u32,
            ..Process::default()
        };
        assert_eq!(
            (identity(&ime).name, identity(&ime).image),
            ("Doors Input Method Editor", "door-ime")
        );
        assert_eq!(identity(&ime).group, Group::Background);
        // A Windows GUI application launched from Explorer or Terminal.
        let gui = Process {
            manifest: manifest::WINDOWS_GUI as u32,
            mode: mode::SESSION as u32,
            uid: 1001,
            ..Process::default()
        };
        assert_eq!(
            (
                identity(&gui).name,
                identity(&gui).image,
                identity(&gui).group
            ),
            ("Windows application", "windows-gui-app", Group::Apps)
        );
        assert_eq!(isolation(&gui), "Windows personality");
        // The same program built for x86 (P17's WOW64 flag).
        let wow = Process {
            mode: mode::SESSION as u32 | MODE_X86,
            ..gui
        };
        assert_eq!(identity(&wow).name, "Windows application (32 bit)");
        assert_eq!(identity(&wow).group, Group::Apps);
        let console = Process {
            manifest: manifest::WINDOWS_APPLICATION as u32,
            mode: MODE_X86,
            ..Process::default()
        };
        assert_eq!(identity(&console).name, "Windows application (32 bit)");
    }
}
