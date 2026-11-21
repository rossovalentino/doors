//! The application registration database (spec §60): what the shell can
//! launch, how it is named and drawn, which file types it opens and where
//! the user pinned it. Catalog apps and the accessories are built in;
//! installed Windows executables register by discovery under
//! `/system/bin/*.exe`. Taskbar/Start pins of registrations persist per
//! user in `PROFILE_FILE`; catalog-app pins stay in the window server's
//! shell layout (one source of truth each, never two).
extern crate alloc;
use alloc::{format, string::String, vec::Vec};
use door_service_protocol::{associations, shell::icon};

/// Per-user database file, relative to the profile folder.
pub const PROFILE_FILE: &str = "AppData/Local/Door/shell-registrations.db";
pub const PROFILE_DIRS: [&str; 3] = ["AppData", "AppData/Local", "AppData/Local/Door"];
const MAGIC: &str = "door-shell-registrations 1";
/// Bound on the persisted file and on discovered executables.
pub const MAX_BYTES: usize = 8192;
pub const MAX_EXECUTABLES: usize = 32;
/// Key spaces: catalog apps/accessories by Init desktop command; Windows
/// executables by a hash of their admitted path.
pub const NATIVE_KEY: u32 = 0x0100_0000;
pub const EXECUTABLE_KEY: u32 = 0x2000_0000;
/// A running Windows program Init admitted from a path nobody registered.
pub const ADHOC_KEY: u32 = 0x3000_0000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Init desktop command (catalog app 1,2,3,9,10,11 or accessory 12,13).
    Native(u8),
    /// A Windows executable, started through Init EXECUTE (no console) with
    /// its folder as working directory and its path as argv[0] only.
    Executable(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    pub key: u32,
    pub name: String,
    pub icon: u16,
    pub target: Target,
    /// Lower-case extensions the app opens.
    pub associations: Vec<&'static str>,
    pub keywords: Vec<&'static str>,
    /// Listed in Start's All list (otherwise found by search only).
    pub listed: bool,
    /// Its windows get taskbar buttons (Captions-like tools do not).
    pub taskbar: bool,
}
impl Registration {
    pub fn native(&self) -> Option<u8> {
        match self.target {
            Target::Native(c) => Some(c),
            Target::Executable(_) => None,
        }
    }
    pub fn path(&self) -> Option<&str> {
        match &self.target {
            Target::Executable(p) => Some(p),
            Target::Native(_) => None,
        }
    }
    /// Stable registration key (`associations::AppKey`): what the pin file
    /// and the per-user file associations name.
    pub fn identity(&self) -> String {
        format!("{}", self.app_key())
    }
    pub fn app_key(&self) -> associations::AppKey<'_> {
        match &self.target {
            Target::Native(c) => associations::AppKey::Native(*c),
            Target::Executable(p) => associations::AppKey::Executable(p),
        }
    }
}
fn fnv(text: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in text.bytes() {
        h ^= u32::from(b.to_ascii_lowercase());
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}
pub fn executable_key(path: &str) -> u32 {
    EXECUTABLE_KEY | (fnv(path) & 0x0fff_ffff)
}
/// Display name of an executable without version resources: its file name
/// without the extension (as Windows shows an unnamed program).
pub fn executable_name(path: &str) -> String {
    String::from(associations::program_name(path))
}
/// A catalog app or accessory. The file types it opens come from the shared
/// association table (`door_service_protocol::associations`), the one source
/// of truth File Explorer and Settings > Default apps also use.
fn native(
    command: u8,
    name: &str,
    icon: u16,
    keywords: &[&'static str],
    listed: bool,
) -> Registration {
    Registration {
        key: NATIVE_KEY | u32::from(command),
        name: name.into(),
        icon,
        target: Target::Native(command),
        associations: associations::declared(command).to_vec(),
        keywords: keywords.to_vec(),
        listed,
        taskbar: listed,
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Database {
    pub entries: Vec<Registration>,
    /// Registration keys pinned to the taskbar / Start, in pin order.
    pub taskbar_pins: Vec<u32>,
    pub start_pins: Vec<u32>,
}
impl Database {
    /// The catalog apps and accessories this image bundles.
    pub fn builtin() -> Self {
        let entries = alloc::vec![
            native(
                1,
                "File Explorer",
                icon::FILES,
                &["explorer", "files", "folders"],
                true
            ),
            native(
                2,
                "Notes",
                icon::NOTES,
                &["notepad", "text", "editor"],
                true
            ),
            native(
                3,
                "Settings",
                icon::SETTINGS,
                &["control panel", "preferences"],
                true
            ),
            native(
                9,
                "Terminal",
                icon::TERMINAL,
                &["cmd", "shell", "console", "powershell"],
                true
            ),
            native(
                10,
                "Preview",
                icon::PREVIEW,
                &["photos", "images", "viewer"],
                true
            ),
            // The Monitor app is Task Manager everywhere (owner request).
            native(
                11,
                "Task Manager",
                icon::TASK_MANAGER,
                &["taskmgr", "processes", "performance"],
                true
            ),
            native(
                12,
                "Doors UI Gallery",
                icon::GALLERY,
                &["controls", "gallery"],
                false
            ),
            native(
                13,
                "Live captions",
                icon::CAPTIONS,
                &["captions", "subtitles"],
                false
            ),
            native(
                14,
                "Calculator",
                icon::CALCULATOR,
                &["calc", "math", "arithmetic", "numbers"],
                true
            ),
            native(
                15,
                "Clock",
                icon::CLOCK,
                &["alarms", "timer", "stopwatch", "world clock", "focus"],
                true
            ),
            native(
                16,
                "Paint",
                icon::PAINT,
                &["mspaint", "paint 3d", "draw", "3d"],
                true
            ),
            native(
                17,
                "Snipping Tool",
                icon::SNIPPING_TOOL,
                &["screenshot", "snip", "screen capture"],
                true
            ),
            native(
                18,
                "Weather",
                icon::WEATHER,
                &["forecast", "temperature"],
                true
            ),
            native(19, "Magnifier", icon::MAGNIFIER, &["magnify", "zoom"], true),
            native(20, "Narrator", icon::NARRATOR, &["screen reader"], true),
            native(
                21,
                "On-Screen Keyboard",
                icon::ON_SCREEN_KEYBOARD,
                &["osk", "keyboard"],
                true
            ),
            native(
                22,
                "Voice access",
                icon::VOICE_ACCESS,
                &["voice control", "dictation"],
                true
            ),
        ];
        Self {
            entries,
            taskbar_pins: Vec::new(),
            start_pins: Vec::new(),
        }
    }
    /// Register installed Windows executables (file names in `folder`).
    /// Returns how many were added.
    pub fn discover<'a>(&mut self, folder: &str, names: impl Iterator<Item = &'a str>) -> usize {
        let mut added = 0;
        for name in names {
            let exe = associations::is_program(name);
            if !exe || name.contains('/') || name.chars().any(char::is_control) {
                continue;
            }
            if self.executables().count() >= MAX_EXECUTABLES {
                break;
            }
            let path = format!("{}/{name}", folder.trim_end_matches('/'));
            let key = executable_key(&path);
            if self.get(key).is_some() {
                continue;
            }
            self.entries.push(Registration {
                key,
                name: executable_name(&path),
                icon: icon::WINDOWS_APP,
                target: Target::Executable(path),
                associations: Vec::new(),
                keywords: Vec::new(),
                listed: true,
                taskbar: true,
            });
            added += 1;
        }
        added
    }
    pub fn executables(&self) -> impl Iterator<Item = &Registration> {
        self.entries.iter().filter(|r| r.path().is_some())
    }
    pub fn get(&self, key: u32) -> Option<&Registration> {
        self.entries.iter().find(|r| r.key == key)
    }
    pub fn by_command(&self, command: u8) -> Option<&Registration> {
        self.entries.iter().find(|r| r.native() == Some(command))
    }
    pub fn by_path(&self, path: &str) -> Option<&Registration> {
        self.entries
            .iter()
            .find(|r| r.path().is_some_and(|p| p.eq_ignore_ascii_case(path)))
    }
    /// The registration that opens files with extension `ext`.
    /// That is the shared table's built-in default; per-user choices live in
    /// the registry (`associations::KEY_PATH`, Settings > Default apps).
    pub fn for_extension(&self, ext: &str) -> Option<&Registration> {
        associations::builtin_default(ext).and_then(|h| self.by_command(h.command))
    }
    /// Registrations that declare `ext` (Open with lists them).
    pub fn declaring(&self, ext: &str) -> impl Iterator<Item = &Registration> {
        let ext = String::from(ext.strip_prefix('.').unwrap_or(ext));
        self.entries
            .iter()
            .filter(move |r| r.associations.iter().any(|a| a.eq_ignore_ascii_case(&ext)))
    }
    /// Registrations the shell pins itself (not catalog apps or tools).
    fn pinnable(&self, key: u32) -> bool {
        self.get(key)
            .is_some_and(|r| r.native().is_none() && r.taskbar)
    }
    /// Pin or unpin `key` on the taskbar; true when the pins changed.
    /// Catalog apps are pinned through the window server's shell layout.
    pub fn set_taskbar_pin(&mut self, key: u32, pinned: bool) -> bool {
        let eligible = self.pinnable(key);
        set_pin(&mut self.taskbar_pins, eligible, key, pinned)
    }
    pub fn set_start_pin(&mut self, key: u32, pinned: bool) -> bool {
        let eligible = self.pinnable(key);
        set_pin(&mut self.start_pins, eligible, key, pinned)
    }
    /// The per-user file: pins by stable identity (never by hash alone).
    pub fn encode_profile(&self) -> String {
        let mut out = String::from(MAGIC);
        out.push('\n');
        for (kind, pins) in [
            ("pin-taskbar", &self.taskbar_pins),
            ("pin-start", &self.start_pins),
        ] {
            for key in pins {
                if let Some(r) = self.get(*key) {
                    out.push_str(&format!("{kind} {}\n", r.identity()));
                }
            }
        }
        out
    }
    /// Apply a per-user file. Pins naming registrations this image lacks
    /// are dropped (an uninstalled program stays unpinned).
    pub fn apply_profile(&mut self, bytes: &[u8]) -> Result<(), &'static str> {
        if bytes.is_empty() {
            return Ok(());
        }
        if bytes.len() > MAX_BYTES {
            return Err("registration profile exceeds its limit");
        }
        let text = core::str::from_utf8(bytes).map_err(|_| "registration profile is not text")?;
        let mut lines = text.lines();
        if lines.next() != Some(MAGIC) {
            return Err("unknown registration profile format");
        }
        let (mut taskbar, mut start) = (Vec::new(), Vec::new());
        for line in lines.filter(|l| !l.is_empty()) {
            let (kind, identity) = line.split_once(' ').ok_or("malformed registration pin")?;
            let key = self
                .entries
                .iter()
                .find(|r| r.identity() == identity)
                .map(|r| r.key);
            let pins = match kind {
                "pin-taskbar" => &mut taskbar,
                "pin-start" => &mut start,
                _ => return Err("malformed registration pin"),
            };
            if let Some(key) = key
                && self.get(key).is_some_and(|r| r.native().is_none())
                && !pins.contains(&key)
            {
                pins.push(key);
            }
        }
        self.taskbar_pins = taskbar;
        self.start_pins = start;
        Ok(())
    }
}
fn set_pin(pins: &mut Vec<u32>, eligible: bool, key: u32, pinned: bool) -> bool {
    if !eligible {
        return false;
    }
    let present = pins.contains(&key);
    if pinned && !present {
        pins.push(key);
        true
    } else if !pinned && present {
        pins.retain(|k| *k != key);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtin_catalog_names_task_manager_and_keeps_tools_off_the_taskbar() {
        let db = Database::builtin();
        assert_eq!(db.by_command(11).unwrap().name, "Task Manager");
        assert_eq!(db.by_command(11).unwrap().icon, icon::TASK_MANAGER);
        assert!(!db.by_command(12).unwrap().taskbar);
        assert!(!db.by_command(13).unwrap().listed);
        assert_eq!(db.for_extension("TXT").unwrap().native(), Some(2));
        assert_eq!(db.for_extension("qoi").unwrap().native(), Some(10));
        assert!(db.for_extension("exe").is_none());
    }
    #[test]
    fn associations_come_from_the_shared_table_with_matching_names_and_icons() {
        let db = Database::builtin();
        for h in associations::HANDLERS {
            let r = db.by_command(h.command).unwrap();
            assert_eq!((r.name.as_str(), r.icon), (h.name, h.icon));
            assert_eq!(r.associations, h.extensions);
            assert_eq!(r.identity(), format!("native:{}", h.command));
            assert_eq!(
                associations::AppKey::parse(&r.identity()),
                Some(r.app_key())
            );
        }
        // Apps that open no files declare nothing.
        for command in [1, 3, 11, 12, 13] {
            assert!(db.by_command(command).unwrap().associations.is_empty());
        }
        // Batch files and PowerShell scripts run in Terminal (Explorer's
        // Open; .ps1 runs in pwsh); Notes edits a .ps1 and Preview shows it
        // read-only, so Open with lists all three.
        assert_eq!(db.for_extension(".bat").unwrap().native(), Some(9));
        let all: Vec<u8> = db.declaring(".PS1").filter_map(|r| r.native()).collect();
        assert_eq!(all, [2, 9, 10]);
        let mut db = db;
        db.discover("/system/bin", ["editor.exe"].into_iter());
        let editor = db.by_path("/system/bin/editor.exe").unwrap();
        assert_eq!(editor.identity(), "exe:/system/bin/editor.exe");
        assert!(editor.associations.is_empty());
    }
    #[test]
    fn executables_register_by_discovery_with_stable_keys() {
        let mut db = Database::builtin();
        let names = [
            "wingui-editor.exe",
            "wingui-selftest.EXE",
            "readme.txt",
            "a/b.exe",
        ];
        assert_eq!(db.discover("/system/bin", names.into_iter()), 2);
        assert_eq!(
            db.discover("/system/bin/", names.into_iter()),
            0,
            "idempotent"
        );
        let editor = db.by_path("/system/bin/wingui-editor.exe").unwrap();
        assert_eq!(editor.name, "wingui-editor");
        assert_eq!(editor.key, executable_key("/system/bin/wingui-editor.exe"));
        assert_eq!(editor.key & 0xf000_0000, EXECUTABLE_KEY);
        assert_eq!(
            db.by_path("/SYSTEM/bin/WINGUI-EDITOR.exe").unwrap().key,
            editor.key
        );
        assert_eq!(executable_name("/x/tool"), "tool");
    }
    #[test]
    fn pins_persist_by_identity_and_reject_catalog_apps_and_unknown_programs() {
        let mut db = Database::builtin();
        db.discover(
            "/system/bin",
            ["wingui-editor.exe", "wingui-f3.exe"].into_iter(),
        );
        let editor = executable_key("/system/bin/wingui-editor.exe");
        let f3 = executable_key("/system/bin/wingui-f3.exe");
        assert!(db.set_taskbar_pin(f3, true));
        assert!(db.set_taskbar_pin(editor, true));
        assert!(!db.set_taskbar_pin(editor, true));
        assert!(
            !db.set_taskbar_pin(NATIVE_KEY | 2, true),
            "catalog pins live in the server"
        );
        assert!(db.set_start_pin(editor, true));
        let text = db.encode_profile();
        assert!(text.contains("pin-taskbar exe:/system/bin/wingui-f3.exe\n"));
        let mut other = Database::builtin();
        other.discover("/system/bin", ["wingui-editor.exe"].into_iter());
        other.apply_profile(text.as_bytes()).unwrap();
        assert_eq!(
            other.taskbar_pins,
            [editor],
            "uninstalled f3 stays unpinned"
        );
        assert_eq!(other.start_pins, [editor]);
        assert!(other.apply_profile(b"something else\n").is_err());
        assert!(
            other
                .apply_profile(b"door-shell-registrations 1\nbogus\n")
                .is_err()
        );
        assert!(other.set_taskbar_pin(editor, false));
        assert!(other.taskbar_pins.is_empty());
    }
}
