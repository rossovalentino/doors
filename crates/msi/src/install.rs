//! What installing a database does, as Windows Installer's standard actions
//! decide it: the properties (the package's, the system folders and the
//! command line's), the selected features and their components, the
//! resolved directory tree, the files with the cabinet each comes from,
//! the registry values, the shortcuts and the Add/Remove Programs entry.
//! Conditions use the Windows Installer condition syntax (`condition`).
use crate::{Database, Error, Table};

mod tables;
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
pub use tables::{
    DuplicatePlan, EnvironmentPlan, FontPlan, IniPlan, MovePlan, RegistryGroup, SelfRegPlan,
    TypeLibPlan, base85_guid,
};

/// The machine's folders and facts the installer's properties report.
#[derive(Clone, Debug)]
pub struct Machine {
    /// `(property, value)`: ProgramFiles64Folder, WindowsFolder, ... with
    /// trailing backslashes, plus VersionNT, ComputerName, ...
    pub properties: Vec<(String, String)>,
}
impl Machine {
    /// Doors' layout for a user profile `profile` (`C:\Users\name\`),
    /// per-machine when `all_users`.
    pub fn doors(profile: &str, user: &str, computer: &str, all_users: bool) -> Self {
        let p = |s: &str| String::from(s);
        let roaming = format!("{profile}AppData\\Roaming\\");
        let start = if all_users {
            p("C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\")
        } else {
            format!("{roaming}Microsoft\\Windows\\Start Menu\\")
        };
        let desktop = if all_users {
            p("C:\\Users\\Public\\Desktop\\")
        } else {
            format!("{profile}Desktop\\")
        };
        let list = [
            ("ROOTDRIVE", p("C:\\")),
            ("WindowsFolder", p("C:\\Windows\\")),
            ("WindowsVolume", p("C:\\")),
            ("SystemFolder", p("C:\\Windows\\SysWOW64\\")),
            ("System64Folder", p("C:\\Windows\\System32\\")),
            ("System16Folder", p("C:\\Windows\\system\\")),
            ("ProgramFilesFolder", p("C:\\Program Files (x86)\\")),
            ("ProgramFiles64Folder", p("C:\\Program Files\\")),
            (
                "CommonFilesFolder",
                p("C:\\Program Files (x86)\\Common Files\\"),
            ),
            (
                "CommonFiles64Folder",
                p("C:\\Program Files\\Common Files\\"),
            ),
            ("CommonAppDataFolder", p("C:\\ProgramData\\")),
            ("FontsFolder", p("C:\\Windows\\Fonts\\")),
            ("TempFolder", format!("{profile}AppData\\Local\\Temp\\")),
            ("AppDataFolder", roaming.clone()),
            ("LocalAppDataFolder", format!("{profile}AppData\\Local\\")),
            ("PersonalFolder", format!("{profile}Documents\\")),
            ("MyPicturesFolder", format!("{profile}Pictures\\")),
            ("FavoritesFolder", format!("{profile}Favorites\\")),
            (
                "SendToFolder",
                format!("{roaming}Microsoft\\Windows\\SendTo\\"),
            ),
            (
                "TemplateFolder",
                format!("{roaming}Microsoft\\Windows\\Templates\\"),
            ),
            ("StartMenuFolder", start.clone()),
            ("ProgramMenuFolder", format!("{start}Programs\\")),
            ("StartupFolder", format!("{start}Programs\\Startup\\")),
            ("DesktopFolder", desktop),
            (
                "AdminToolsFolder",
                format!("{start}Programs\\Administrative Tools\\"),
            ),
            // Windows Installer reports Windows 8.1 to unmanifested
            // installers on Windows 10 and 11.
            ("VersionNT", p("603")),
            ("VersionNT64", p("603")),
            ("WindowsBuild", p("9600")),
            ("ServicePackLevel", p("0")),
            ("MsiNTProductType", p("1")),
            ("VersionMsi", p("5.00")),
            ("Msix64", p("6")),
            ("ComputerName", p(computer)),
            ("LogonUser", p(user)),
            ("UserLanguageID", p("1033")),
            ("SystemLanguageID", p("1033")),
            ("Privileged", p("1")),
            ("AdminUser", p("1")),
            ("INSTALLLEVEL", p("1")),
        ];
        Self {
            properties: list
                .into_iter()
                .map(|(k, v)| (String::from(k), v))
                .collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cabinet {
    /// A stream inside the package (`#name` in the Media table).
    Embedded(String),
    /// A file beside the package.
    External(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// The file named `name` (the File key) inside a cabinet.
    Cabinet { cabinet: Cabinet, name: String },
    /// An uncompressed file beside the package (its source path, relative).
    Loose(String),
    /// An empty file (an isolated application's `.local`).
    Empty,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileCopy {
    pub key: String,
    pub source: Source,
    pub target: String,
    pub size: u32,
    pub component: String,
    /// File.Version: a version (`1.2.3.4`), a companion file key, or empty.
    pub version: String,
    pub attributes: i32,
}
/// A component as the Component table describes it, its directory
/// resolved.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComponentInfo {
    pub key: String,
    /// ComponentId (empty: an unregistered component).
    pub guid: String,
    pub directory: String,
    pub attributes: i32,
    /// The key path: a File key, a Registry key (attribute 0x4), or empty
    /// (the directory).
    pub keypath: String,
    /// Selected by the features and its condition.
    pub selected: bool,
}
pub mod component_attributes {
    pub const REGISTRY_KEY_PATH: i32 = 0x4;
    pub const SHARED_DLL_REF_COUNT: i32 = 0x8;
    pub const PERMANENT: i32 = 0x10;
    pub const NEVER_OVERWRITE: i32 = 0x80;
    pub const SIXTY_FOUR_BIT: i32 = 0x100;
    pub const UNINSTALL_ON_SUPERSEDENCE: i32 = 0x800;
}
/// ServiceInstall: a service the component installs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServicePlan {
    pub component: String,
    pub name: String,
    pub display_name: String,
    pub service_type: u32,
    pub start_type: u32,
    pub error_control: u32,
    pub dependencies: Vec<String>,
    pub account: String,
    /// The executable (the component's key path) and its arguments.
    pub command_line: String,
    pub description: String,
}
/// ServiceControl: what happens to a service when its component is
/// installed or removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceControlPlan {
    pub component: String,
    pub name: String,
    /// msidbServiceControlEvent* bits.
    pub event: u32,
    pub wait: bool,
}
pub mod service_event {
    pub const INSTALL_START: u32 = 0x1;
    pub const INSTALL_STOP: u32 = 0x2;
    pub const INSTALL_DELETE: u32 = 0x8;
    pub const UNINSTALL_START: u32 = 0x10;
    pub const UNINSTALL_STOP: u32 = 0x20;
    pub const UNINSTALL_DELETE: u32 = 0x80;
}
/// RemoveFile: files (wildcards allowed) a component removes on install
/// (mode 1), removal (2) or both (3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoveFilePlan {
    pub component: String,
    pub folder: String,
    /// None: the folder itself (removed when empty).
    pub pattern: Option<String>,
    pub mode: i32,
}
/// A registry root in one of the registry's views: the 64-bit (native)
/// view, or the 32-bit view WOW64 gives 32-bit programs (KEY_WOW64_32KEY:
/// HKLM\SOFTWARE\WOW6432Node and the redirected COM keys). Windows
/// Installer writes a 32-bit component's registry in the 32-bit view, a
/// 64-bit component's (msidbComponentAttributes64bit) and its own records
/// in the 64-bit one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Root {
    ClassesRoot,
    CurrentUser,
    #[default]
    LocalMachine,
    Users,
    ClassesRoot32,
    CurrentUser32,
    LocalMachine32,
    Users32,
}
impl Root {
    pub fn name(self) -> &'static str {
        match self.native() {
            Root::ClassesRoot => "HKEY_CLASSES_ROOT",
            Root::CurrentUser => "HKEY_CURRENT_USER",
            Root::Users => "HKEY_USERS",
            _ => "HKEY_LOCAL_MACHINE",
        }
    }
    /// In the 32-bit view.
    pub fn view32(self) -> bool {
        matches!(
            self,
            Root::ClassesRoot32 | Root::CurrentUser32 | Root::LocalMachine32 | Root::Users32
        )
    }
    /// The same root in the 64-bit view.
    pub fn native(self) -> Root {
        match self {
            Root::ClassesRoot32 => Root::ClassesRoot,
            Root::CurrentUser32 => Root::CurrentUser,
            Root::LocalMachine32 => Root::LocalMachine,
            Root::Users32 => Root::Users,
            r => r,
        }
    }
    /// The same root in the 32-bit view (`view32`) or the 64-bit one.
    pub fn in_view(self, view32: bool) -> Root {
        match (self.native(), view32) {
            (r, false) => r,
            (Root::ClassesRoot, true) => Root::ClassesRoot32,
            (Root::CurrentUser, true) => Root::CurrentUser32,
            (Root::Users, true) => Root::Users32,
            (_, true) => Root::LocalMachine32,
        }
    }
}
/// A 32-bit package: the summary's platform is Intel (or absent, as in old
/// packages). Its components are all 32-bit.
pub fn package_is_32bit(db: &Database) -> bool {
    db.summary()
        .ok()
        .and_then(|s| s.platform().map(String::from))
        .is_none_or(|p| p.is_empty() || p.eq_ignore_ascii_case("Intel"))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegValue {
    String(String),
    Expand(String),
    Dword(u32),
    Binary(Vec<u8>),
    Multi(Vec<String>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryWrite {
    pub root: Root,
    pub key: String,
    /// None: the key's default value.
    pub name: Option<String>,
    /// None: only create the key.
    pub value: Option<RegValue>,
    pub component: String,
    /// The key itself goes with the component (`-` or `*` name, or a
    /// file association's keys).
    pub remove_key: bool,
    /// The standard action that writes and removes it.
    pub group: RegistryGroup,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutPlan {
    /// The .lnk file.
    pub path: String,
    pub target: String,
    pub arguments: String,
    pub description: String,
    pub working_directory: String,
    pub show: i32,
    pub component: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub product_code: String,
    pub product_name: String,
    pub version: String,
    pub manufacturer: String,
    pub upgrade_code: String,
    pub all_users: bool,
    pub properties: Vec<(String, String)>,
    /// Directory key and resolved path (with a trailing backslash).
    pub directories: Vec<(String, String)>,
    /// Folders to create (every installed file's, plus CreateFolder).
    pub folders: Vec<String>,
    pub files: Vec<FileCopy>,
    pub registry: Vec<RegistryWrite>,
    pub shortcuts: Vec<ShortcutPlan>,
    /// The Add/Remove Programs key (under HKLM or HKCU) and its values; its
    /// root in the package's view (a 32-bit package's entry is in the
    /// 32-bit view, as Windows Installer writes it).
    pub uninstall_key: String,
    pub uninstall_root: Root,
    pub uninstall: Vec<(String, RegValue)>,
    pub components: Vec<String>,
    pub features: Vec<String>,
    /// Every component's directory key ([$component]).
    pub component_dirs: Vec<(String, String)>,
    /// Every component (selected or not).
    pub component_info: Vec<ComponentInfo>,
    /// Feature -> its components.
    pub feature_components: Vec<(String, String)>,
    pub services: Vec<ServicePlan>,
    pub service_controls: Vec<ServiceControlPlan>,
    pub remove_files: Vec<RemoveFilePlan>,
    /// A 32-bit (Intel) package.
    pub x86: bool,
    pub fonts: Vec<FontPlan>,
    pub typelibs: Vec<TypeLibPlan>,
    pub self_reg: Vec<SelfRegPlan>,
    pub environment: Vec<EnvironmentPlan>,
    pub ini: Vec<IniPlan>,
    pub duplicates: Vec<DuplicatePlan>,
    pub moves: Vec<MovePlan>,
}
impl Plan {
    /// Whether `component` is a 32-bit one: every component of a 32-bit
    /// package, and those of a 64-bit package without
    /// msidbComponentAttributes64bit.
    pub fn component_is_32bit(&self, component: &str) -> bool {
        self.x86
            || self
                .component_info
                .iter()
                .find(|c| c.key == component)
                .is_none_or(|c| c.attributes & component_attributes::SIXTY_FOUR_BIT == 0)
    }
    pub fn property(&self, name: &str) -> Option<&str> {
        self.properties
            .iter()
            .find(|p| p.0 == name)
            .map(|p| p.1.as_str())
    }
    pub fn directory(&self, key: &str) -> Option<&str> {
        self.directories
            .iter()
            .find(|d| d.0 == key)
            .map(|d| d.1.as_str())
    }
    /// A formatted string (`[Property]`, `[#file]`, `[$component]`,
    /// `[!file]`, `[%ENV]`, `[\x]`, `[~]`) as MsiFormatRecord expands it.
    pub fn format(&self, text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(open) = rest.find('[') {
            out.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            let Some(close) = after.find(']') else {
                out.push_str(&rest[open..]);
                return out;
            };
            let inner = &after[..close];
            rest = &after[close + 1..];
            if let Some(escaped) = inner.strip_prefix('\\') {
                out.push_str(escaped);
            } else if inner == "~" {
                out.push('\0');
            } else if let Some(file) = inner.strip_prefix('#').or_else(|| inner.strip_prefix('!')) {
                if let Some(f) = self.files.iter().find(|f| f.key == file) {
                    out.push_str(&f.target);
                }
            } else if let Some(component) = inner.strip_prefix('$') {
                if let Some(d) = self.component_directory(component) {
                    out.push_str(d);
                }
            } else if inner.starts_with('%') {
                // Environment variables are the executor's: left as is.
                out.push('[');
                out.push_str(inner);
                out.push(']');
            } else if let Some(v) = self.property(inner).or_else(|| self.directory(inner)) {
                out.push_str(v);
            }
        }
        out.push_str(rest);
        out
    }
    fn component_directory(&self, component: &str) -> Option<&str> {
        self.component_dirs
            .iter()
            .find(|c| c.0 == component)
            .and_then(|c| self.directory(&c.1))
    }
}

/// `short|long`: the long name.
fn long_name(name: &str) -> &str {
    name.rsplit('|').next().unwrap_or(name)
}
/// The target part of a DefaultDir (`target:source`).
fn target_dir(default_dir: &str) -> &str {
    long_name(default_dir.split(':').next().unwrap_or(default_dir))
}

fn opt_table(db: &Database, name: &str) -> Result<Option<Table>, Error> {
    match db.table(name) {
        Ok(t) => Ok(Some(t)),
        Err(Error::NotFound) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Plan installing `db` on `machine` with the command line's `overrides`
/// (`PROPERTY=value`).
pub fn plan(
    db: &Database,
    machine: &Machine,
    overrides: &[(String, String)],
) -> Result<Plan, Error> {
    let mut plan = Plan {
        x86: package_is_32bit(db),
        ..Plan::default()
    };
    // Properties: machine, then the package, then the command line.
    let set = |plan: &mut Plan, k: &str, v: &str| {
        if let Some(p) = plan.properties.iter_mut().find(|p| p.0 == k) {
            p.1 = String::from(v);
        } else {
            plan.properties.push((String::from(k), String::from(v)));
        }
    };
    for (k, v) in &machine.properties {
        set(&mut plan, k, v);
    }
    if let Some(t) = opt_table(db, "Property")? {
        for r in 0..t.rows.len() {
            if let (Some(k), Some(v)) = (t.get(r, "Property").str(), t.get(r, "Value").str()) {
                set(&mut plan, k, v);
            }
        }
    }
    for (k, v) in overrides {
        set(&mut plan, k, v);
    }
    let get = |plan: &Plan, k: &str| plan.property(k).unwrap_or("").to_string();
    plan.product_code = get(&plan, "ProductCode");
    plan.product_name = get(&plan, "ProductName");
    plan.version = get(&plan, "ProductVersion");
    plan.manufacturer = get(&plan, "Manufacturer");
    plan.upgrade_code = get(&plan, "UpgradeCode");
    plan.all_users = matches!(plan.property("ALLUSERS"), Some("1" | "2"));
    if plan.product_code.is_empty() {
        return Err(Error::Malformed);
    }

    // Directories: TARGETDIR is the root drive; a property of a
    // directory's name (set on the command line, or a system folder)
    // replaces its resolved path.
    if let Some(t) = opt_table(db, "Directory")? {
        let mut pending: Vec<usize> = (0..t.rows.len()).collect();
        let mut guard = 0;
        while !pending.is_empty() {
            guard += 1;
            if guard > t.rows.len() + 1 {
                return Err(Error::Malformed);
            }
            pending.retain(|&r| {
                let key = t.get(r, "Directory").str().unwrap_or("").to_string();
                let parent = t.get(r, "Directory_Parent").str().unwrap_or("").to_string();
                let default = t.get(r, "DefaultDir").str().unwrap_or("").to_string();
                let base = if parent.is_empty() || parent == key {
                    Some(get(&plan, "ROOTDRIVE"))
                } else {
                    plan.directory(&parent).map(String::from)
                };
                let Some(base) = base else {
                    return true;
                };
                let resolved = match plan.property(&key) {
                    Some(v) if !v.is_empty() => {
                        let mut v = String::from(v);
                        if !v.ends_with('\\') {
                            v.push('\\');
                        }
                        v
                    }
                    _ => {
                        let name = target_dir(&default);
                        if parent.is_empty()
                            || name == "."
                            || name == "SourceDir"
                            || name == "SOURCEDIR"
                        {
                            base
                        } else {
                            format!("{base}{name}\\")
                        }
                    }
                };
                plan.directories.push((key, resolved));
                false
            });
        }
    }
    // Directory paths are properties too ([INSTALLFOLDER]).
    let dirs = plan.directories.clone();
    for (k, v) in &dirs {
        set(&mut plan, k, v);
    }

    // Features at or below INSTALLLEVEL (level 0 is never installed),
    // after the Condition table's adjustments.
    let level: i32 = get(&plan, "INSTALLLEVEL").parse().unwrap_or(1);
    let mut feature_levels: Vec<(String, i32)> = Vec::new();
    if let Some(t) = opt_table(db, "Feature")? {
        for r in 0..t.rows.len() {
            let key = t.get(r, "Feature").str().unwrap_or("").to_string();
            feature_levels.push((key, t.get(r, "Level").int().unwrap_or(0)));
        }
    }
    if let Some(t) = opt_table(db, "Condition")? {
        for r in 0..t.rows.len() {
            let feature = t.get(r, "Feature_").str().unwrap_or("");
            let condition = t.get(r, "Condition").str().unwrap_or("");
            if condition::evaluate(condition, &plan.properties)
                && let Some(f) = feature_levels.iter_mut().find(|f| f.0 == feature)
            {
                f.1 = t.get(r, "Level").int().unwrap_or(f.1);
            }
        }
    }
    let mut selected: Vec<String> = match plan.property("ADDLOCAL") {
        Some(all) if all.eq_ignore_ascii_case("ALL") => {
            feature_levels.iter().map(|f| f.0.clone()).collect()
        }
        Some(list) if !list.is_empty() => list
            .split(',')
            .map(|f| String::from(f.trim()))
            .filter(|f| feature_levels.iter().any(|l| l.0 == *f))
            .collect(),
        _ => feature_levels
            .iter()
            .filter(|f| f.1 > 0 && f.1 <= level)
            .map(|f| f.0.clone())
            .collect(),
    };
    // REMOVE=Feature,...: those features go (REMOVE=ALL is the engine's).
    if let Some(remove) = plan.property("REMOVE")
        && !remove.eq_ignore_ascii_case("ALL")
    {
        let gone: Vec<String> = remove.split(',').map(|f| String::from(f.trim())).collect();
        selected.retain(|f| !gone.contains(f));
    }
    plan.features = selected.clone();
    let mut components: Vec<String> = Vec::new();
    if let Some(t) = opt_table(db, "FeatureComponents")? {
        for r in 0..t.rows.len() {
            let feature = t.get(r, "Feature_").str().unwrap_or("");
            let component = t.get(r, "Component_").str().unwrap_or("");
            plan.feature_components
                .push((String::from(feature), String::from(component)));
            if selected.iter().any(|f| f == feature) && !components.iter().any(|c| c == component) {
                components.push(String::from(component));
            }
        }
    }
    // Components whose condition holds.
    if let Some(t) = opt_table(db, "Component")? {
        for r in 0..t.rows.len() {
            let key = t.get(r, "Component").str().unwrap_or("").to_string();
            let directory = t.get(r, "Directory_").str().unwrap_or("").to_string();
            let condition = t.get(r, "Condition").str().unwrap_or("");
            if !condition.is_empty() && !condition::evaluate(condition, &plan.properties) {
                components.retain(|c| *c != key);
            }
            plan.component_info.push(ComponentInfo {
                key: key.clone(),
                guid: t.get(r, "ComponentId").str().unwrap_or("").to_string(),
                directory: plan.directory(&directory).unwrap_or("").to_string(),
                attributes: t.get(r, "Attributes").int().unwrap_or(0),
                keypath: t.get(r, "KeyPath").str().unwrap_or("").to_string(),
                selected: components.contains(&key),
            });
            plan.component_dirs.push((key, directory));
        }
    }
    plan.components = components.clone();

    // Files and their sources.
    let compressed = db.summary().map(|s| s.compressed()).unwrap_or(true);
    let media: Vec<(i32, String)> = match opt_table(db, "Media")? {
        Some(t) => {
            let mut m: Vec<(i32, String)> = (0..t.rows.len())
                .map(|r| {
                    (
                        t.get(r, "LastSequence").int().unwrap_or(0),
                        t.get(r, "Cabinet").str().unwrap_or("").to_string(),
                    )
                })
                .collect();
            m.sort_by_key(|x| x.0);
            m
        }
        None => Vec::new(),
    };
    if let Some(t) = opt_table(db, "File")? {
        for r in 0..t.rows.len() {
            let key = t.get(r, "File").str().unwrap_or("").to_string();
            let component = t.get(r, "Component_").str().unwrap_or("");
            if !components.iter().any(|c| c == component) {
                continue;
            }
            let Some(dir) = plan.component_directory(component).map(String::from) else {
                return Err(Error::Malformed);
            };
            let name = long_name(t.get(r, "FileName").str().unwrap_or(""));
            let sequence = t.get(r, "Sequence").int().unwrap_or(0);
            let attributes = t.get(r, "Attributes").int().unwrap_or(0);
            // msidbFileAttributesNoncompressed / Compressed override the
            // package default.
            let in_cabinet = if attributes & 0x2000 != 0 {
                false
            } else if attributes & 0x4000 != 0 {
                true
            } else {
                compressed
            };
            let cabinet = media.iter().find(|m| m.0 >= sequence).map(|m| m.1.clone());
            let source = match cabinet {
                Some(c) if in_cabinet && !c.is_empty() => Source::Cabinet {
                    cabinet: match c.strip_prefix('#') {
                        Some(stream) => Cabinet::Embedded(String::from(stream)),
                        None => Cabinet::External(c),
                    },
                    name: key.clone(),
                },
                _ => Source::Loose(String::from(name)),
            };
            if !plan.folders.contains(&dir) {
                plan.folders.push(dir.clone());
            }
            plan.files.push(FileCopy {
                key,
                source,
                target: format!("{dir}{name}"),
                size: t.get(r, "FileSize").int().unwrap_or(0) as u32,
                component: String::from(component),
                version: t.get(r, "Version").str().unwrap_or("").to_string(),
                attributes,
            });
        }
    }
    if let Some(t) = opt_table(db, "CreateFolder")? {
        for r in 0..t.rows.len() {
            let component = t.get(r, "Component_").str().unwrap_or("");
            if components.iter().any(|c| c == component)
                && let Some(d) = plan.directory(t.get(r, "Directory_").str().unwrap_or(""))
            {
                let d = String::from(d);
                if !plan.folders.contains(&d) {
                    plan.folders.push(d);
                }
            }
        }
    }

    // Registry values.
    if let Some(t) = opt_table(db, "Registry")? {
        for r in 0..t.rows.len() {
            let component = t.get(r, "Component_").str().unwrap_or("");
            if !components.iter().any(|c| c == component) {
                continue;
            }
            let root = match t.get(r, "Root").int().unwrap_or(-1) {
                0 => Root::ClassesRoot,
                1 => Root::CurrentUser,
                2 => Root::LocalMachine,
                3 => Root::Users,
                // -1: per-machine or per-user with the installation.
                _ if plan.all_users => Root::LocalMachine,
                _ => Root::CurrentUser,
            };
            let key = plan.format(t.get(r, "Key").str().unwrap_or(""));
            let name = t.get(r, "Name").str().map(|n| plan.format(n));
            let value = t.get(r, "Value").str().map(|v| registry_value(&plan, v));
            // "+", "-" and "*" with no value act on the key itself: "-"
            // and "*" remove it with the component.
            let remove_key = matches!((name.as_deref(), &value), (Some("-" | "*"), None));
            let name = match (name.as_deref(), &value) {
                (Some("+" | "-" | "*"), None) => None,
                _ => name,
            };
            plan.registry.push(RegistryWrite {
                root: root.in_view(plan.component_is_32bit(component)),
                key,
                name,
                value,
                component: String::from(component),
                remove_key,
                group: RegistryGroup::Registry,
            });
        }
    }
    // File associations (RegisterExtensionInfo, RegisterProgIdInfo): under
    // the machine's or the user's Software\Classes.
    let classes_root = if plan.all_users {
        Root::LocalMachine
    } else {
        Root::CurrentUser
    };
    if let Some(ext) = opt_table(db, "Extension")? {
        let progids = opt_table(db, "ProgId")?;
        let verbs = opt_table(db, "Verb")?;
        let component_rows = opt_table(db, "Component")?;
        for r in 0..ext.rows.len() {
            let component = ext.get(r, "Component_").str().unwrap_or("").to_string();
            if !components.contains(&component) {
                continue;
            }
            let extension = ext.get(r, "Extension").str().unwrap_or("").to_string();
            let progid = ext.get(r, "ProgId_").str().unwrap_or("").to_string();
            // The verb runs the component's key path file.
            let keyfile = component_rows
                .as_ref()
                .and_then(|c| {
                    let row = c.find(&component)?;
                    c.get(row, "KeyPath").str().map(String::from)
                })
                .and_then(|k| {
                    plan.files
                        .iter()
                        .find(|f| f.key == k)
                        .map(|f| f.target.clone())
                })
                .unwrap_or_default();
            let s = |v: &str| Some(RegValue::String(String::from(v)));
            let push = |plan: &mut Plan,
                        key: String,
                        name: Option<String>,
                        value: Option<RegValue>,
                        remove_key: bool| {
                plan.registry.push(RegistryWrite {
                    root: classes_root.in_view(plan.component_is_32bit(&component)),
                    key,
                    name,
                    value,
                    component: component.clone(),
                    remove_key,
                    group: RegistryGroup::Extension,
                });
            };
            let class_key = |k: &str| format!("Software\\Classes\\{k}");
            let handler = if progid.is_empty() {
                format!("{}_auto_file", extension)
            } else {
                progid.clone()
            };
            push(
                &mut plan,
                class_key(&format!(".{extension}")),
                None,
                s(&handler),
                true,
            );
            if let Some(mime) = ext.get(r, "MIME_").str() {
                push(
                    &mut plan,
                    class_key(&format!(".{extension}")),
                    Some(String::from("Content Type")),
                    s(mime),
                    false,
                );
            }
            if let Some(p) = &progids
                && let Some(row) = p.find(&progid)
            {
                if let Some(d) = p.get(row, "Description").str() {
                    let d = plan.format(d);
                    push(&mut plan, class_key(&handler), None, s(&d), true);
                }
                if p.get(row, "Icon_").str().is_some() || !keyfile.is_empty() {
                    let index = p.get(row, "IconIndex").int().unwrap_or(0);
                    push(
                        &mut plan,
                        class_key(&format!("{handler}\\DefaultIcon")),
                        None,
                        s(&format!("{keyfile},{index}")),
                        false,
                    );
                }
            }
            if let Some(v) = &verbs {
                let mut list: Vec<(i32, String, String, String)> = (0..v.rows.len())
                    .filter(|&i| v.get(i, "Extension_").str() == Some(extension.as_str()))
                    .map(|i| {
                        (
                            v.get(i, "Sequence").int().unwrap_or(0),
                            v.get(i, "Verb").str().unwrap_or("").to_string(),
                            v.get(i, "Command").str().unwrap_or("").to_string(),
                            v.get(i, "Argument").str().unwrap_or("").to_string(),
                        )
                    })
                    .collect();
                list.sort_by_key(|x| x.0);
                if let Some(first) = list.first() {
                    let first = first.1.clone();
                    push(
                        &mut plan,
                        class_key(&format!("{handler}\\shell")),
                        None,
                        s(&first),
                        false,
                    );
                }
                for (_, verb, command, argument) in list {
                    let shell = class_key(&format!("{handler}\\shell\\{verb}"));
                    if !command.is_empty() {
                        let c = plan.format(&command);
                        push(&mut plan, shell.clone(), None, s(&c), false);
                    }
                    let line = if argument.is_empty() {
                        format!("\"{keyfile}\"")
                    } else {
                        format!("\"{keyfile}\" {}", plan.format(&argument))
                    };
                    push(
                        &mut plan,
                        format!("{shell}\\command"),
                        None,
                        s(&line),
                        false,
                    );
                }
            }
        }
    }
    // COM classes, MIME types, fonts, type libraries, self-registration,
    // environment variables, INI files, duplicated, isolated and moved
    // files, qualified components.
    tables::plan(db, &mut plan, classes_root)?;
    // Services (InstallServices, StartServices, StopServices,
    // DeleteServices).
    if let Some(t) = opt_table(db, "ServiceInstall")? {
        let component_rows = opt_table(db, "Component")?;
        for r in 0..t.rows.len() {
            let component = t.get(r, "Component_").str().unwrap_or("").to_string();
            if !components.contains(&component) {
                continue;
            }
            let keyfile = component_rows
                .as_ref()
                .and_then(|c| {
                    let row = c.find(&component)?;
                    c.get(row, "KeyPath").str().map(String::from)
                })
                .and_then(|k| {
                    plan.files
                        .iter()
                        .find(|f| f.key == k)
                        .map(|f| f.target.clone())
                })
                .unwrap_or_default();
            let arguments = plan.format(t.get(r, "Arguments").str().unwrap_or(""));
            let command_line = if arguments.is_empty() {
                format!("\"{keyfile}\"")
            } else {
                format!("\"{keyfile}\" {arguments}")
            };
            plan.services.push(ServicePlan {
                component,
                name: plan.format(t.get(r, "Name").str().unwrap_or("")),
                display_name: plan.format(t.get(r, "DisplayName").str().unwrap_or("")),
                service_type: t.get(r, "ServiceType").int().unwrap_or(0x10) as u32,
                start_type: t.get(r, "StartType").int().unwrap_or(3) as u32,
                error_control: t.get(r, "ErrorControl").int().unwrap_or(1) as u32,
                dependencies: t
                    .get(r, "Dependencies")
                    .str()
                    .map(|d| {
                        plan.format(d)
                            .split('\0')
                            .filter(|s| !s.is_empty())
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default(),
                account: plan.format(t.get(r, "StartName").str().unwrap_or("")),
                command_line,
                description: plan.format(t.get(r, "Description").str().unwrap_or("")),
            });
        }
    }
    if let Some(t) = opt_table(db, "ServiceControl")? {
        for r in 0..t.rows.len() {
            plan.service_controls.push(ServiceControlPlan {
                component: t.get(r, "Component_").str().unwrap_or("").to_string(),
                name: plan.format(t.get(r, "Name").str().unwrap_or("")),
                event: t.get(r, "Event").int().unwrap_or(0) as u32,
                wait: t.get(r, "Wait").int().unwrap_or(1) != 0,
            });
        }
    }
    if let Some(t) = opt_table(db, "RemoveFile")? {
        for r in 0..t.rows.len() {
            let property = t.get(r, "DirProperty").str().unwrap_or("");
            let Some(folder) = plan
                .property(property)
                .or_else(|| plan.directory(property))
                .map(String::from)
            else {
                continue;
            };
            plan.remove_files.push(RemoveFilePlan {
                component: t.get(r, "Component_").str().unwrap_or("").to_string(),
                folder,
                pattern: t
                    .get(r, "FileName")
                    .str()
                    .map(|n| String::from(long_name(n))),
                mode: t.get(r, "InstallMode").int().unwrap_or(2),
            });
        }
    }

    // Shortcuts.
    if let Some(t) = opt_table(db, "Shortcut")? {
        for r in 0..t.rows.len() {
            let component = t.get(r, "Component_").str().unwrap_or("");
            if !components.iter().any(|c| c == component) {
                continue;
            }
            let Some(dir) = plan.directory(t.get(r, "Directory_").str().unwrap_or("")) else {
                continue;
            };
            let name = long_name(t.get(r, "Name").str().unwrap_or(""));
            let raw_target = t.get(r, "Target").str().unwrap_or("");
            // An advertised shortcut names a feature: its target is the
            // shortcut component's key path.
            let target = if raw_target.contains('[') {
                plan.format(raw_target)
            } else {
                let keypath = opt_table(db, "Component")?.and_then(|c| {
                    let row = c.find(component)?;
                    c.get(row, "KeyPath").str().map(String::from)
                });
                keypath
                    .and_then(|k| {
                        plan.files
                            .iter()
                            .find(|f| f.key == k)
                            .map(|f| f.target.clone())
                    })
                    .unwrap_or_default()
            };
            let working = t
                .get(r, "WkDir")
                .str()
                .and_then(|w| plan.directory(w))
                .unwrap_or("")
                .to_string();
            plan.shortcuts.push(ShortcutPlan {
                path: format!("{dir}{name}.lnk"),
                target,
                arguments: plan.format(t.get(r, "Arguments").str().unwrap_or("")),
                description: plan.format(t.get(r, "Description").str().unwrap_or("")),
                working_directory: working,
                show: t.get(r, "ShowCmd").int().unwrap_or(1),
                component: String::from(component),
            });
        }
    }
    // Shortcut folders are created too.
    for s in &plan.shortcuts {
        if let Some(i) = s.path.rfind('\\') {
            let d = String::from(&s.path[..=i]);
            if !plan.folders.contains(&d) {
                plan.folders.push(d);
            }
        }
    }

    // Add/Remove Programs (RegisterProduct, PublishProduct).
    let base = if plan.all_users {
        "HKEY_LOCAL_MACHINE"
    } else {
        "HKEY_CURRENT_USER"
    };
    plan.uninstall_key = format!(
        "{base}\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{}",
        plan.product_code
    );
    plan.uninstall_root = if plan.all_users {
        Root::LocalMachine
    } else {
        Root::CurrentUser
    }
    .in_view(plan.x86);
    let size_kb = plan
        .files
        .iter()
        .map(|f| u64::from(f.size))
        .sum::<u64>()
        .div_ceil(1024) as u32;
    let mut version = plan
        .version
        .split('.')
        .map(|p| p.parse::<u32>().unwrap_or(0));
    let (major, minor, build) = (
        version.next().unwrap_or(0),
        version.next().unwrap_or(0),
        version.next().unwrap_or(0),
    );
    let s = |v: &str| RegValue::String(String::from(v));
    let mut arp = Vec::new();
    arp.push((String::from("DisplayName"), s(&plan.product_name)));
    arp.push((String::from("DisplayVersion"), s(&plan.version)));
    arp.push((String::from("Publisher"), s(&plan.manufacturer)));
    let location = plan
        .property("ARPINSTALLLOCATION")
        .or_else(|| plan.property("INSTALLFOLDER"))
        .or_else(|| plan.property("INSTALLDIR"))
        .or_else(|| plan.property("APPLICATIONFOLDER"))
        .unwrap_or("")
        .to_string();
    arp.push((String::from("InstallLocation"), s(&location)));
    arp.push((
        String::from("UninstallString"),
        RegValue::Expand(format!("MsiExec.exe /X{}", plan.product_code)),
    ));
    arp.push((
        String::from("ModifyPath"),
        RegValue::Expand(format!("MsiExec.exe /I{}", plan.product_code)),
    ));
    arp.push((String::from("EstimatedSize"), RegValue::Dword(size_kb)));
    arp.push((
        String::from("Version"),
        RegValue::Dword(major << 24 | (minor & 0xff) << 16 | (build & 0xffff)),
    ));
    arp.push((String::from("VersionMajor"), RegValue::Dword(major)));
    arp.push((String::from("VersionMinor"), RegValue::Dword(minor)));
    arp.push((String::from("WindowsInstaller"), RegValue::Dword(1)));
    if let Ok(language) = get(&plan, "ProductLanguage").parse::<u32>() {
        arp.push((String::from("Language"), RegValue::Dword(language)));
    }
    for (property, value) in [
        ("ARPCONTACT", "Contact"),
        ("ARPCOMMENTS", "Comments"),
        ("ARPHELPLINK", "HelpLink"),
        ("ARPURLINFOABOUT", "URLInfoAbout"),
        ("ARPURLUPDATEINFO", "URLUpdateInfo"),
        ("ARPPRODUCTICON", "DisplayIcon"),
    ] {
        if let Some(v) = plan.property(property) {
            let v = plan.format(v);
            arp.push((String::from(value), s(&v)));
        }
    }
    for (property, value) in [
        ("ARPNOMODIFY", "NoModify"),
        ("ARPNOREMOVE", "NoRemove"),
        ("ARPNOREPAIR", "NoRepair"),
        ("ARPSYSTEMCOMPONENT", "SystemComponent"),
    ] {
        if plan.property(property).is_some_and(|v| !v.is_empty()) {
            arp.push((String::from(value), RegValue::Dword(1)));
        }
    }
    plan.uninstall = arp;
    Ok(plan)
}

/// A Registry table Value: `#123` a DWORD, `#x0a0b` binary, `#%text` an
/// expandable string, `##text` a string starting with `#`, `[~]`
/// separating a multi-string, else a string; all formatted first.
fn registry_value(plan: &Plan, raw: &str) -> RegValue {
    if let Some(rest) = raw.strip_prefix("##") {
        return RegValue::String(format!("#{}", plan.format(rest)));
    }
    if let Some(hex) = raw.strip_prefix("#x").or_else(|| raw.strip_prefix("#X")) {
        let hex = plan.format(hex);
        let digits: Vec<u8> = hex
            .bytes()
            .filter_map(|c| (c as char).to_digit(16).map(|d| d as u8))
            .collect();
        let mut bytes = Vec::new();
        // An odd count gets a leading zero.
        let start = digits.len() % 2;
        if start == 1 {
            bytes.push(digits[0]);
        }
        for pair in digits[start..].chunks_exact(2) {
            bytes.push(pair[0] << 4 | pair[1]);
        }
        return RegValue::Binary(bytes);
    }
    if let Some(text) = raw.strip_prefix("#%") {
        return RegValue::Expand(plan.format(text));
    }
    if let Some(number) = raw.strip_prefix('#') {
        let n = plan.format(number);
        let value = n.trim().parse::<i64>().map(|v| v as u32).unwrap_or(0);
        return RegValue::Dword(value);
    }
    if raw.contains("[~]") {
        let formatted = plan.format(raw);
        return RegValue::Multi(
            formatted
                .split('\0')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect(),
        );
    }
    RegValue::String(plan.format(raw))
}

pub mod condition {
    //! Windows Installer conditional statements: properties, string and
    //! integer literals, `=` `<>` `<` `>` `<=` `>=`, `~=` (case-insensitive
    //! equality), `NOT`, `AND`, `OR`, `XOR`, `EQV`, `IMP` and parentheses.
    //! A property alone is true when set to a non-empty value.
    use alloc::{string::String, vec::Vec};

    #[derive(Clone, Debug, PartialEq)]
    enum Token {
        Name(String),
        Text(String),
        Number(i64),
        Op(&'static str),
        Open,
        Close,
    }
    fn tokens(s: &str) -> Option<Vec<Token>> {
        let c: Vec<char> = s.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < c.len() {
            let ch = c[i];
            if ch.is_whitespace() {
                i += 1;
            } else if ch == '(' {
                out.push(Token::Open);
                i += 1;
            } else if ch == ')' {
                out.push(Token::Close);
                i += 1;
            } else if ch == '"' {
                let end = c[i + 1..].iter().position(|x| *x == '"')? + i + 1;
                out.push(Token::Text(c[i + 1..end].iter().collect()));
                i = end + 1;
            } else if ch.is_ascii_digit()
                || (ch == '-' && c.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
            {
                let start = i;
                i += 1;
                while i < c.len() && c[i].is_ascii_digit() {
                    i += 1;
                }
                let text: String = c[start..i].iter().collect();
                out.push(Token::Number(text.parse().ok()?));
            } else if "<>=~".contains(ch) {
                let two: String = c[i..(i + 2).min(c.len())].iter().collect();
                let op = ["<>", "<=", ">=", "~="].into_iter().find(|o| two == *o);
                match op {
                    Some(o) => {
                        out.push(Token::Op(o));
                        i += 2;
                    }
                    None => {
                        out.push(Token::Op(match ch {
                            '<' => "<",
                            '>' => ">",
                            '=' => "=",
                            _ => return None,
                        }));
                        i += 1;
                    }
                }
            } else if ch.is_alphanumeric()
                || ch == '_'
                || ch == '.'
                || ch == '!'
                || ch == '$'
                || ch == '&'
                || ch == '?'
                || ch == '%'
                || ch == '#'
            {
                let start = i;
                i += 1;
                while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '.') {
                    i += 1;
                }
                let word: String = c[start..i].iter().collect();
                let upper = word.to_ascii_uppercase();
                out.push(match upper.as_str() {
                    "NOT" => Token::Op("NOT"),
                    "AND" => Token::Op("AND"),
                    "OR" => Token::Op("OR"),
                    "XOR" => Token::Op("XOR"),
                    "EQV" => Token::Op("EQV"),
                    "IMP" => Token::Op("IMP"),
                    _ => Token::Name(word),
                });
            } else {
                return None;
            }
        }
        Some(out)
    }
    #[derive(Clone, Debug)]
    enum V {
        Text(String),
        Number(i64),
    }
    struct Parser<'a> {
        t: Vec<Token>,
        i: usize,
        properties: &'a [(String, String)],
    }
    impl Parser<'_> {
        fn peek(&self) -> Option<&Token> {
            self.t.get(self.i)
        }
        fn value(&self, name: &str) -> V {
            let v = self
                .properties
                .iter()
                .find(|p| p.0 == name)
                .map(|p| p.1.clone())
                .unwrap_or_default();
            match v.parse::<i64>() {
                Ok(n) => V::Number(n),
                Err(_) => V::Text(v),
            }
        }
        fn operand(&mut self) -> Option<V> {
            let t = self.peek()?.clone();
            self.i += 1;
            Some(match t {
                Token::Name(n) => self.value(&n),
                Token::Text(s) => V::Text(s),
                Token::Number(n) => V::Number(n),
                _ => return None,
            })
        }
        fn truth(v: &V) -> bool {
            match v {
                V::Text(s) => !s.is_empty(),
                V::Number(n) => *n != 0,
            }
        }
        fn term(&mut self) -> Option<bool> {
            match self.peek()? {
                Token::Op("NOT") => {
                    self.i += 1;
                    Some(!self.term()?)
                }
                Token::Open => {
                    self.i += 1;
                    let v = self.expression()?;
                    if self.peek() != Some(&Token::Close) {
                        return None;
                    }
                    self.i += 1;
                    Some(v)
                }
                _ => {
                    let left = self.operand()?;
                    let op = match self.peek() {
                        Some(Token::Op(o))
                            if ["=", "<>", "<", ">", "<=", ">=", "~="].contains(o) =>
                        {
                            *o
                        }
                        _ => return Some(Self::truth(&left)),
                    };
                    self.i += 1;
                    let right = self.operand()?;
                    Some(compare(&left, op, &right))
                }
            }
        }
        fn conjunction(&mut self) -> Option<bool> {
            let mut v = self.term()?;
            while self.peek() == Some(&Token::Op("AND")) {
                self.i += 1;
                let r = self.term()?;
                v = v && r;
            }
            Some(v)
        }
        fn expression(&mut self) -> Option<bool> {
            let mut v = self.conjunction()?;
            loop {
                let op = match self.peek() {
                    Some(Token::Op(o @ ("OR" | "XOR" | "EQV" | "IMP"))) => *o,
                    _ => return Some(v),
                };
                self.i += 1;
                let r = self.conjunction()?;
                v = match op {
                    "OR" => v || r,
                    "XOR" => v != r,
                    "EQV" => v == r,
                    _ => !v || r,
                };
            }
        }
    }
    fn compare(a: &V, op: &str, b: &V) -> bool {
        match (a, b) {
            (V::Number(x), V::Number(y)) => match op {
                "=" | "~=" => x == y,
                "<>" => x != y,
                "<" => x < y,
                ">" => x > y,
                "<=" => x <= y,
                _ => x >= y,
            },
            _ => {
                let text = |v: &V| match v {
                    V::Text(s) => s.clone(),
                    V::Number(n) => alloc::format!("{n}"),
                };
                let (x, y) = (text(a), text(b));
                match op {
                    "=" => x == y,
                    "~=" => x.eq_ignore_ascii_case(&y),
                    "<>" => x != y,
                    "<" => x < y,
                    ">" => x > y,
                    "<=" => x <= y,
                    _ => x >= y,
                }
            }
        }
    }
    /// Evaluate `condition` (an empty one is true; a malformed one false).
    pub fn evaluate(condition: &str, properties: &[(String, String)]) -> bool {
        if condition.trim().is_empty() {
            return true;
        }
        let Some(t) = tokens(condition) else {
            return false;
        };
        let mut p = Parser {
            t,
            i: 0,
            properties,
        };
        match p.expression() {
            Some(v) if p.i == p.t.len() => v,
            _ => false,
        }
    }
}
