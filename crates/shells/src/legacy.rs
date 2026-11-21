//! Windows program names and URI schemes typed at a prompt, mapped to the
//! Doors app or Settings page that replaces them (`notepad` opens Notes,
//! `control printers` and `ms-settings:display` open Settings). Settings
//! entry points come from the shared route table (`door_settings_map`), as
//! in Run; this table holds the programs.
use alloc::{format, string::String};

/// Doors apps and inbox tools by their desktop launch command
/// (`door_service_protocol::application`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum App {
    Files,
    Notes,
    Settings,
    Terminal,
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
    LiveCaptions,
}
impl App {
    /// The `LAUNCH_APP` command.
    pub fn command(self) -> u32 {
        match self {
            Self::Files => 1,
            Self::Notes => 2,
            Self::Settings => 3,
            Self::Terminal => 9,
            Self::Preview => 10,
            Self::TaskManager => 11,
            Self::LiveCaptions => 13,
            Self::Calculator => 14,
            Self::Clock => 15,
            Self::Paint => 16,
            Self::SnippingTool => 17,
            Self::Weather => 18,
            Self::Magnifier => 19,
            Self::Narrator => 20,
            Self::OnScreenKeyboard => 21,
            Self::VoiceAccess => 22,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Files => "File Explorer",
            Self::Notes => "Notes",
            Self::Settings => "Settings",
            Self::Terminal => "Terminal",
            Self::Preview => "Preview",
            Self::TaskManager => "Task Manager",
            Self::Calculator => "Calculator",
            Self::Clock => "Clock",
            Self::Paint => "Paint",
            Self::SnippingTool => "Snipping Tool",
            Self::Weather => "Weather",
            Self::Magnifier => "Magnifier",
            Self::Narrator => "Narrator",
            Self::OnScreenKeyboard => "On-Screen Keyboard",
            Self::VoiceAccess => "Voice access",
            Self::LiveCaptions => "Live captions",
        }
    }
    /// Apps that open a document passed to them.
    pub fn opens_files(self) -> bool {
        matches!(
            self,
            Self::Files | Self::Notes | Self::Preview | Self::Paint
        )
    }
}
/// Program names (without `.exe`) and URI schemes.
const PROGRAMS: &[(&str, App)] = &[
    ("explorer", App::Files),
    ("notepad", App::Notes),
    ("write", App::Notes),
    ("wordpad", App::Notes),
    ("systemsettings", App::Settings),
    ("wt", App::Terminal),
    ("windowsterminal", App::Terminal),
    ("conhost", App::Terminal),
    ("powershell", App::Terminal),
    ("pwsh", App::Terminal),
    ("powershell_ise", App::Terminal),
    ("photos", App::Preview),
    ("photoviewer", App::Preview),
    ("mspaint", App::Paint),
    ("pbrush", App::Paint),
    ("taskmgr", App::TaskManager),
    ("resmon", App::TaskManager),
    ("perfmon", App::TaskManager),
    ("calc", App::Calculator),
    ("calculator", App::Calculator),
    ("snippingtool", App::SnippingTool),
    ("magnify", App::Magnifier),
    ("narrator", App::Narrator),
    ("osk", App::OnScreenKeyboard),
    ("voiceaccess", App::VoiceAccess),
    ("livecaptions", App::LiveCaptions),
    ("ms-photos:", App::Preview),
    ("ms-paint:", App::Paint),
    ("ms-calculator:", App::Calculator),
    ("calculator:", App::Calculator),
    ("ms-clock:", App::Clock),
    ("ms-screensketch:", App::SnippingTool),
    ("ms-screenclip:", App::SnippingTool),
    ("msnweather:", App::Weather),
    ("bingweather:", App::Weather),
    ("ms-taskmgr:", App::TaskManager),
];
/// What a typed name opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    App(App),
    /// `door_settings_map::navigate_arg`.
    Settings(u32),
}
/// Resolve the command word (and, for Settings entry points such as
/// `control printers`, the whole line).
pub fn resolve(word: &str, line: &str) -> Option<Target> {
    let name = word.trim_matches('"');
    let base = name.rsplit(['\\', '/']).next().unwrap_or(name);
    let lower = base.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
    // A URI's scheme: "ms-settings:display" -> "ms-settings:".
    let scheme = lower.find(':').filter(|&i| i > 1).map(|i| &lower[..=i]);
    if let Some((_, app)) = PROGRAMS
        .iter()
        .find(|(n, _)| *n == stem || Some(*n) == scheme)
    {
        return Some(Target::App(*app));
    }
    let resolved =
        door_settings_map::resolve(line.trim()).or_else(|| door_settings_map::resolve(name))?;
    Some(Target::Settings(door_settings_map::navigate_arg(
        resolved.0, resolved.1,
    )))
}
/// The one-line note a prompt prints before opening a Settings page.
pub fn settings_note(navigate: u32) -> String {
    let Some((page, section)) = door_settings_map::decode_navigate(navigate) else {
        return String::from("Opening Settings");
    };
    let mut note = String::from("Opening Settings > ");
    for (i, crumb) in page.crumbs().enumerate() {
        if i > 0 {
            note.push_str(" > ");
        }
        note.push_str(crumb.nav_title());
    }
    if let Some(s) = section {
        note = format!("{note} > {}", s.title);
    }
    note
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_names_open_doors_apps_and_settings() {
        assert_eq!(
            resolve("notepad", "notepad a.txt"),
            Some(Target::App(App::Notes))
        );
        assert_eq!(
            resolve("C:\\Windows\\System32\\CALC.EXE", ""),
            Some(Target::App(App::Calculator))
        );
        assert_eq!(resolve("ms-clock:alarm", ""), Some(Target::App(App::Clock)));
        assert!(matches!(
            resolve("control", "control printers"),
            Some(Target::Settings(_))
        ));
        assert!(matches!(
            resolve("ms-settings:display", "ms-settings:display"),
            Some(Target::Settings(_))
        ));
        assert!(matches!(
            resolve("desk.cpl", "desk.cpl"),
            Some(Target::Settings(_))
        ));
        assert_eq!(resolve("dir", "dir"), None);
    }
}
