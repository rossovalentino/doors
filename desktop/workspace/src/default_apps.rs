//! Default apps (Settings > Apps > Default apps) and File Explorer's Open,
//! Open with and "Choose another app", on one model.
//!
//! * Apps that open files: the built-in handlers of the shared association
//!   table (`door_service_protocol::associations`: Notes, Terminal,
//!   Preview) and the Windows programs installed in `/system/bin`, each
//!   with its stable registration key (`native:2`,
//!   `exe:/system/bin/editor.exe`).
//! * A per-user choice per file type: `HKCU\Software\Doors\FileAssociations\
//!   <.ext>`, REG_SZ `App` = a registration key. Absent or empty means the
//!   built-in default, as on a clean Windows install. Resetting a type
//!   writes the empty value until the registry gains value deletion.
//! * What opening a file with an app does ([`launch`]): Notes and Preview
//!   get the document through the window server (OPEN_DOCUMENT); a Windows
//!   program is started through Init with the file as its argument.
//!
//! Registry access goes through [`Backend`] (Storage in the apps, a fake in
//! the host tests). Extensions are passed with or without their dot, in any
//! case; results use the registry form (`.txt`).
extern crate alloc;
use alloc::{format, string::String, vec::Vec};
use door_service_client::{Error, Key, RegistryRoot, RegistryValue, RegistryView, Storage};
use door_service_protocol::{Code, associations as assoc, shell::icon, window as w};

/// Installed programs listed per load (as the registration database).
pub const MAX_PROGRAMS: usize = 32;
/// Stored per-user choices read for the file-type list.
pub const MAX_CHOICES: u32 = 256;

/// What an app is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A built-in app by Init desktop command; `documents`: it can be
    /// handed a file (Notes, Preview; not Terminal yet).
    Native { command: u8, documents: bool },
    /// A Windows program by its Door path.
    Program(String),
}
/// An app that can be set as the default for file types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct App {
    /// Stable registration key (`associations::AppKey` text form).
    pub key: String,
    pub name: String,
    /// `shell::icon` id.
    pub icon: u16,
    /// Lower-case extensions (without the dot) the app declares.
    pub extensions: Vec<&'static str>,
    pub target: Target,
}
impl App {
    pub fn command(&self) -> Option<u8> {
        match self.target {
            Target::Native { command, .. } => Some(command),
            Target::Program(_) => None,
        }
    }
    pub fn program(&self) -> Option<&str> {
        match &self.target {
            Target::Program(path) => Some(path),
            Target::Native { .. } => None,
        }
    }
    /// Why this app cannot open a file (its control stays disabled with
    /// this text), if so: Terminal declares script types but cannot be
    /// handed a file yet.
    pub fn unavailable(&self) -> Option<&'static str> {
        match self.target {
            Target::Native {
                documents: false, ..
            } => Some(assoc::TERMINAL_UNAVAILABLE),
            _ => None,
        }
    }
    /// The app declares `ext` (with or without the dot, any case).
    pub fn declares(&self, ext: &str) -> bool {
        let ext = ext.strip_prefix('.').unwrap_or(ext);
        self.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext))
    }
}

/// The apps that can open files, in Windows' list order: the built-in
/// apps, then installed programs in discovery order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Apps {
    pub list: Vec<App>,
}
impl Apps {
    /// The built-in apps of the shared association table.
    pub fn builtin() -> Self {
        let list = assoc::HANDLERS
            .iter()
            .map(|h| App {
                key: format!("{}", assoc::AppKey::Native(h.command)),
                name: h.name.into(),
                icon: h.icon,
                extensions: h.extensions.to_vec(),
                target: Target::Native {
                    command: h.command,
                    documents: h.documents,
                },
            })
            .collect();
        Self { list }
    }
    /// Add the Windows programs among `names` (file names in
    /// `associations::PROGRAM_FOLDER`), as the registration database does.
    /// Returns how many were added.
    pub fn discover<'a>(&mut self, names: impl Iterator<Item = &'a str>) -> usize {
        let mut added = 0;
        for name in names {
            if !assoc::is_program(name) || name.contains('/') || name.chars().any(char::is_control)
            {
                continue;
            }
            if self.list.iter().filter(|a| a.program().is_some()).count() >= MAX_PROGRAMS {
                break;
            }
            let path = format!("{}/{name}", assoc::PROGRAM_FOLDER);
            let key = format!("{}", assoc::AppKey::Executable(&path));
            if self.get(&key).is_some() {
                continue;
            }
            self.list.push(App {
                key,
                name: assoc::program_name(&path).into(),
                icon: icon::WINDOWS_APP,
                extensions: Vec::new(),
                target: Target::Program(path),
            });
            added += 1;
        }
        added
    }
    /// The built-in apps and the programs installed in `/system/bin`.
    pub fn load(storage: &mut Storage) -> Self {
        let mut apps = Self::builtin();
        let mut names = Vec::new();
        let mut cookie = 0;
        while names.len() < MAX_PROGRAMS * 4 {
            match storage.list(assoc::PROGRAM_FOLDER, cookie) {
                Ok(Some(entry)) => {
                    cookie = entry.cookie;
                    if !entry.info.directory {
                        names.push(String::from_utf16_lossy(&entry.name[..entry.name_len]));
                    }
                }
                _ => break,
            }
        }
        apps.discover(names.iter().map(String::as_str));
        apps
    }
    /// The app with registration key `key` (program paths compare without
    /// case, as Door paths do).
    pub fn get(&self, key: &str) -> Option<&App> {
        self.list
            .iter()
            .find(|a| a.key == key || a.program().is_some() && a.key.eq_ignore_ascii_case(key))
    }
    /// "Open with": the apps that declare `ext`, the current default first.
    pub fn open_with(&self, ext: &str, default: Option<&str>) -> Vec<&App> {
        let mut list: Vec<&App> = self.list.iter().filter(|a| a.declares(ext)).collect();
        if let Some(app) = default.and_then(|k| self.get(k)) {
            list.retain(|a| a.key != app.key);
            list.insert(0, app);
        }
        list
    }
    /// The context menu's "Open with": Doors' own viewer (Preview) leads
    /// for every type it can show, then the default and the other apps
    /// registered for the type. ("Choose another app" keeps the default
    /// first, as Windows does.)
    pub fn open_with_menu(&self, ext: &str, default: Option<&str>) -> Vec<&App> {
        let mut list = self.open_with(ext, default);
        let preview = format!("{}", assoc::AppKey::Native(10));
        if let Some(i) = list.iter().position(|a| a.key == preview) {
            let app = list.remove(i);
            list.insert(0, app);
        }
        list
    }
    /// "Choose another app": the current default first, then the apps that
    /// declare the type, then every other app that can open a file.
    pub fn choices(&self, ext: &str, default: Option<&str>) -> Vec<&App> {
        let mut list = self.open_with(ext, default);
        for app in &self.list {
            // Terminal runs scripts only: offered just for its own types.
            let scripts_only = app.key == "native:9";
            if app.unavailable().is_none()
                && !scripts_only
                && !list.iter().any(|a| a.key == app.key)
            {
                list.push(app);
            }
        }
        list
    }
}

/// `.ext` in registry form (lower case, with its dot), or `None` when it is
/// not a valid type name (`associations::valid_extension`).
pub fn normalize(ext: &str) -> Option<String> {
    let ext = ext.strip_prefix('.').unwrap_or(ext);
    assoc::valid_extension(ext).then(|| format!(".{}", ext.to_ascii_lowercase()))
}
/// The file type (`.ext`) of a file name or path, if it has one.
pub fn extension_for(name: &str) -> Option<String> {
    assoc::extension_of(name).and_then(normalize)
}
/// The built-in default's registration key for `ext`.
pub fn builtin_default(ext: &str) -> Option<String> {
    assoc::builtin_default(ext).map(|h| format!("{}", assoc::AppKey::Native(h.command)))
}

/// The registry operations the model uses (per-user hive, 64-bit view).
pub trait Backend {
    type Key;
    fn open(&mut self, path: &str, write: bool, create: bool) -> Result<Self::Key, Error>;
    fn query(&mut self, key: &Self::Key, name: &str) -> Result<RegistryValue, Error>;
    fn set(&mut self, key: &Self::Key, name: &str, value: RegistryValue) -> Result<(), Error>;
    /// Subkey names of `key` (bounded).
    fn children(&mut self, key: &Self::Key, max: u32) -> Result<Vec<String>, Error>;
    fn flush(&mut self) -> Result<(), Error>;
    fn close(&mut self, key: Self::Key) -> Result<(), Error>;
}
fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}
impl Backend for Storage {
    type Key = Key;
    fn open(&mut self, path: &str, write: bool, create: bool) -> Result<Key, Error> {
        self.registry_open(
            RegistryRoot::CurrentUser,
            RegistryView::Bits64,
            &units(path),
            if write { 3 } else { 1 },
            create,
        )
    }
    fn query(&mut self, key: &Key, name: &str) -> Result<RegistryValue, Error> {
        self.registry_query(key, &units(name))
    }
    fn set(&mut self, key: &Key, name: &str, value: RegistryValue) -> Result<(), Error> {
        self.registry_set(key, &units(name), value)
    }
    fn children(&mut self, key: &Key, max: u32) -> Result<Vec<String>, Error> {
        let version = self.registry_version(key)?;
        let mut out = Vec::new();
        for i in 0..max {
            match self.registry_enumerate_key(key, i, version) {
                Ok(name) => out.push(String::from_utf16_lossy(name.units())),
                Err(Error::Service(Code::NotFound)) => break,
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }
    fn flush(&mut self) -> Result<(), Error> {
        Storage::flush(self)
    }
    fn close(&mut self, key: Key) -> Result<(), Error> {
        self.registry_close(key)
    }
}
fn type_key(ext: &str) -> Result<String, Error> {
    let ext = normalize(ext).ok_or(Error::Service(Code::InvalidRequest))?;
    Ok(format!("{}\\{ext}", assoc::KEY_PATH))
}
/// REG_SZ data: UTF-16LE with the terminating NUL.
pub fn string_value(text: &str) -> Result<RegistryValue, Error> {
    let mut data = Vec::with_capacity(text.len() * 2 + 2);
    for unit in text.encode_utf16() {
        data.extend_from_slice(&unit.to_le_bytes());
    }
    data.extend_from_slice(&[0, 0]);
    RegistryValue::new(1, &data)
}
/// The text of a REG_SZ / REG_EXPAND_SZ value (up to its first NUL).
pub fn string_of(value: &RegistryValue) -> Option<String> {
    if !matches!(value.kind, 1 | 2) || !value.data().len().is_multiple_of(2) {
        return None;
    }
    let units: Vec<u16> = value
        .data()
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|u| *u != 0)
        .collect();
    String::from_utf16(&units).ok()
}
/// The registration key the user chose for `ext`, if any (absent, empty or
/// malformed values are no choice).
pub fn user_choice<B: Backend>(b: &mut B, ext: &str) -> Result<Option<String>, Error> {
    let key = match b.open(&type_key(ext)?, false, false) {
        Ok(key) => key,
        Err(Error::Service(Code::NotFound)) => return Ok(None),
        Err(e) => return Err(e),
    };
    let value = b.query(&key, assoc::APP_VALUE);
    let closed = b.close(key);
    let choice = match value {
        Ok(v) => string_of(&v).filter(|k| assoc::AppKey::parse(k).is_some()),
        Err(Error::Service(Code::NotFound)) => None,
        Err(e) => return Err(e),
    };
    closed?;
    Ok(choice)
}
/// Where a type's current default comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The user's choice (Open with > Always, Settings > Default apps).
    User,
    /// The built-in default of the association table.
    Builtin,
}
/// The current default of `ext`: the user's choice when it names an app in
/// `apps` that can open files (an uninstalled program falls back), else
/// the built-in default; `None` for a type nothing opens.
pub fn default_for<B: Backend>(
    b: &mut B,
    apps: &Apps,
    ext: &str,
) -> Result<Option<(String, Source)>, Error> {
    let choice = user_choice(b, ext)?;
    Ok(resolve(apps, ext, choice.as_deref()))
}
/// The current default of `ext` given the user's stored choice `user` (as
/// [`default_for`] reads it; apps that keep [`user_choices`] use this).
pub fn resolve(apps: &Apps, ext: &str, user: Option<&str>) -> Option<(String, Source)> {
    if let Some(app) = user
        .and_then(|k| apps.get(k))
        .filter(|a| a.unavailable().is_none())
    {
        return Some((app.key.clone(), Source::User));
    }
    builtin_default(ext).map(|k| (k, Source::Builtin))
}
/// Every stored choice as (`.ext`, registration key), for apps that keep
/// them between registry reads (File Explorer re-reads them on refresh).
pub fn user_choices<B: Backend>(b: &mut B) -> Result<Vec<(String, String)>, Error> {
    let names = match b.open(assoc::KEY_PATH, false, false) {
        Ok(key) => {
            let children = b.children(&key, MAX_CHOICES);
            let closed = b.close(key);
            let children = children?;
            closed?;
            children
        }
        Err(Error::Service(Code::NotFound)) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for name in names {
        let Some(ext) = name.strip_prefix('.').and_then(normalize) else {
            continue;
        };
        if let Some(key) = user_choice(b, &ext)? {
            out.push((ext, key));
        }
    }
    Ok(out)
}
/// Make the app `key` the default for `ext` ("Always", Settings > Default
/// apps). Refuses apps that are not in `apps` or cannot open files.
pub fn set_default<B: Backend>(b: &mut B, apps: &Apps, ext: &str, key: &str) -> Result<(), Error> {
    let app = apps
        .get(key)
        .filter(|a| a.unavailable().is_none())
        .ok_or(Error::Service(Code::InvalidRequest))?;
    write_choice(b, ext, &app.key.clone())
}
/// Return `ext` to its built-in default.
pub fn reset_default<B: Backend>(b: &mut B, ext: &str) -> Result<(), Error> {
    // No value deletion yet: an empty value reads as "no choice".
    match b.open(&type_key(ext)?, true, false) {
        Ok(key) => {
            let written = string_value("")
                .and_then(|v| b.set(&key, assoc::APP_VALUE, v))
                .and_then(|()| b.flush());
            let closed = b.close(key);
            written.and(closed)
        }
        Err(Error::Service(Code::NotFound)) => Ok(()),
        Err(e) => Err(e),
    }
}
fn write_choice<B: Backend>(b: &mut B, ext: &str, key: &str) -> Result<(), Error> {
    let value = string_value(key)?;
    let opened = b.open(&type_key(ext)?, true, true)?;
    let written = b
        .set(&opened, assoc::APP_VALUE, value)
        .and_then(|()| b.flush());
    let closed = b.close(opened);
    written.and(closed)
}
/// One row of Settings' "Choose defaults by file type".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileType {
    /// `.ext`, lower case.
    pub ext: String,
    /// Registration key of the current default (`None`: "Choose a
    /// default").
    pub default: Option<String>,
    pub source: Option<Source>,
}
/// Every type a built-in app declares plus every type the user chose an app
/// for, sorted as Windows lists them, each with its current default.
pub fn file_types<B: Backend>(b: &mut B, apps: &Apps) -> Result<Vec<FileType>, Error> {
    let mut exts: Vec<String> = assoc::known_extensions().filter_map(normalize).collect();
    match b.open(assoc::KEY_PATH, false, false) {
        Ok(key) => {
            let children = b.children(&key, MAX_CHOICES);
            let closed = b.close(key);
            for name in children? {
                if let Some(ext) = name.strip_prefix('.').and_then(normalize)
                    && !exts.contains(&ext)
                {
                    exts.push(ext);
                }
            }
            closed?;
        }
        Err(Error::Service(Code::NotFound)) => (),
        Err(e) => return Err(e),
    }
    exts.sort();
    exts.into_iter()
        .map(|ext| file_type(b, apps, ext))
        .collect()
}
fn file_type<B: Backend>(b: &mut B, apps: &Apps, ext: String) -> Result<FileType, Error> {
    let current = default_for(b, apps, &ext)?;
    Ok(FileType {
        ext,
        source: current.as_ref().map(|c| c.1),
        default: current.map(|c| c.0),
    })
}
/// Settings' per-app page ("Set defaults for applications"): the types the
/// app `key` declares, each with its current default.
pub fn app_types<B: Backend>(b: &mut B, apps: &Apps, key: &str) -> Result<Vec<FileType>, Error> {
    let Some(app) = apps.get(key) else {
        return Ok(Vec::new());
    };
    let mut exts: Vec<String> = app.extensions.iter().filter_map(|e| normalize(e)).collect();
    exts.sort();
    exts.into_iter()
        .map(|ext| file_type(b, apps, ext))
        .collect()
}

/// What opening a file with an app asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Launch {
    /// OPEN_DOCUMENT to the window server: app command and Door path.
    Document { app: u32, payload: Vec<u8> },
    /// Init EXECUTE_DETACHED: the program, started in the file's folder
    /// with argv = [program, file as a Windows path].
    Program {
        path: String,
        cwd: String,
        arguments: Vec<u8>,
    },
}
/// The Windows form of a Door path as the compatibility layer resolves it
/// (`C:` is the Door root): `/users/1/a.txt` is `C:\users\1\a.txt`.
pub fn windows_path(path: &str) -> String {
    format!("C:{}", path.replace('/', "\\"))
}
/// How to open the file at `path` (a Door path) with `app`; the error is
/// the text to show.
pub fn launch(app: &App, path: &str) -> Result<Launch, &'static str> {
    if let Some(reason) = app.unavailable() {
        return Err(reason);
    }
    if !path.starts_with('/') || path.contains('\0') || path.chars().any(char::is_control) {
        return Err("This file can't be opened from here.");
    }
    match &app.target {
        Target::Native { command, .. } => {
            if path.len() > w::MAX_DOCUMENT {
                return Err("The file's path is too long to open it in this app.");
            }
            Ok(Launch::Document {
                app: u32::from(*command),
                payload: path.as_bytes().to_vec(),
            })
        }
        Target::Program(program) => {
            let cwd = match path.rsplit_once('/') {
                Some(("", _)) => String::from("/"),
                Some((folder, _)) => String::from(folder),
                None => return Err("This file can't be opened from here."),
            };
            let mut arguments = Vec::from(program.as_bytes());
            arguments.push(0);
            arguments.extend_from_slice(windows_path(path).as_bytes());
            arguments.push(0);
            if arguments.len() > door_abi::platform::ARGUMENT_BYTES {
                return Err("The file's path is too long to open it with this program.");
            }
            Ok(Launch::Program {
                path: program.clone(),
                cwd,
                arguments,
            })
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use alloc::collections::BTreeMap;
    /// An in-memory per-user hive: key path (lower case) -> values.
    #[derive(Default)]
    pub(crate) struct Hive {
        pub keys: BTreeMap<String, BTreeMap<String, RegistryValue>>,
        pub flushes: usize,
        pub open: usize,
        /// Values longer than this fail like today's 64-byte store.
        pub capacity: usize,
    }
    impl Hive {
        pub(crate) fn new() -> Self {
            Self {
                capacity: 2048,
                ..Self::default()
            }
        }
    }
    impl Backend for Hive {
        type Key = String;
        fn open(&mut self, path: &str, _write: bool, create: bool) -> Result<String, Error> {
            let path = path.to_ascii_lowercase();
            if !self.keys.contains_key(&path) {
                if !create {
                    return Err(Error::Service(Code::NotFound));
                }
                // Parents are created with the key.
                let mut at = 0;
                while let Some(i) = path[at..].find('\\') {
                    self.keys.entry(path[..at + i].into()).or_default();
                    at += i + 1;
                }
                self.keys.insert(path.clone(), BTreeMap::new());
            }
            self.open += 1;
            Ok(path)
        }
        fn query(&mut self, key: &String, name: &str) -> Result<RegistryValue, Error> {
            self.keys[key]
                .get(&name.to_ascii_lowercase())
                .copied()
                .ok_or(Error::Service(Code::NotFound))
        }
        fn set(&mut self, key: &String, name: &str, value: RegistryValue) -> Result<(), Error> {
            if value.data().len() > self.capacity {
                return Err(Error::Service(Code::InvalidRequest));
            }
            self.keys
                .get_mut(key)
                .unwrap()
                .insert(name.to_ascii_lowercase(), value);
            Ok(())
        }
        fn children(&mut self, key: &String, max: u32) -> Result<Vec<String>, Error> {
            let prefix = format!("{key}\\");
            Ok(self
                .keys
                .keys()
                .filter_map(|k| k.strip_prefix(&prefix))
                .filter(|rest| !rest.contains('\\'))
                .take(max as usize)
                .map(String::from)
                .collect())
        }
        fn flush(&mut self) -> Result<(), Error> {
            self.flushes += 1;
            Ok(())
        }
        fn close(&mut self, _key: String) -> Result<(), Error> {
            self.open -= 1;
            Ok(())
        }
    }
    pub(crate) fn apps() -> Apps {
        let mut apps = Apps::builtin();
        apps.discover(["wingui-editor.exe", "readme.txt", "SETUP.EXE"].into_iter());
        apps
    }
    #[test]
    fn apps_come_from_the_shared_table_and_installed_programs() {
        let apps = apps();
        let keys: Vec<&str> = apps.list.iter().map(|a| a.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "native:2",
                "native:9",
                "native:10",
                "exe:/system/bin/wingui-editor.exe",
                "exe:/system/bin/SETUP.EXE"
            ]
        );
        let notes = apps.get("native:2").unwrap();
        assert_eq!((notes.name.as_str(), notes.icon), ("Notes", icon::NOTES));
        assert!(notes.declares(".TXT") && notes.declares("ps1"));
        let terminal = apps.get("native:9").unwrap();
        assert_eq!(terminal.unavailable(), None, "Terminal runs batch files");
        assert!(terminal.declares("bat") && terminal.declares(".DSH") && terminal.declares("ps1"));
        let editor = apps.get("EXE:/SYSTEM/bin/wingui-editor.exe").unwrap();
        assert_eq!(editor.name, "wingui-editor");
        assert_eq!(editor.icon, icon::WINDOWS_APP);
        assert!(editor.extensions.is_empty());
        let mut again = apps.clone();
        assert_eq!(
            again.discover(["wingui-editor.exe", "a/b.exe", ".exe"].into_iter()),
            0
        );
        assert!(
            apps.get("native:1").is_none(),
            "File Explorer opens no files"
        );
    }
    #[test]
    fn open_with_lists_declaring_apps_default_first_and_choices_add_the_rest() {
        let apps = apps();
        let names = |list: Vec<&App>| list.iter().map(|a| a.name.clone()).collect::<Vec<_>>();
        assert_eq!(
            names(apps.open_with(".bat", Some("native:2"))),
            ["Notes", "Terminal", "Preview"]
        );
        assert_eq!(
            names(apps.open_with("bat", Some("exe:/system/bin/wingui-editor.exe"))),
            ["wingui-editor", "Notes", "Terminal", "Preview"]
        );
        // The context menu puts Preview first wherever it can show the type.
        assert_eq!(
            names(apps.open_with_menu(".bat", Some("native:2"))),
            ["Preview", "Notes", "Terminal"]
        );
        assert_eq!(names(apps.open_with_menu("png", None)), ["Preview"]);
        assert_eq!(names(apps.open_with("png", Some("native:10"))), ["Preview"]);
        assert!(apps.open_with("xyz", None).is_empty());
        // Choose another app: everything that can open a file; Terminal only
        // where it declares the type (shown disabled).
        assert_eq!(
            names(apps.choices("xyz", None)),
            ["Notes", "Preview", "wingui-editor", "SETUP"]
        );
        assert_eq!(
            names(apps.choices("bat", Some("native:2"))),
            ["Notes", "Terminal", "Preview", "wingui-editor", "SETUP"]
        );
    }
    #[test]
    fn extensions_normalize_to_the_registry_form() {
        assert_eq!(normalize("TXT").as_deref(), Some(".txt"));
        assert_eq!(normalize(".Md").as_deref(), Some(".md"));
        for bad in ["", ".", "a.b", "a b", "..txt", "abcdefghijklmnopq"] {
            assert_eq!(normalize(bad), None, "{bad}");
        }
        assert_eq!(extension_for("/u/Report.PDF").as_deref(), Some(".pdf"));
        assert_eq!(extension_for("Makefile"), None);
        assert_eq!(builtin_default(".JPG").as_deref(), Some("native:10"));
        assert_eq!(builtin_default("bat").as_deref(), Some("native:9"));
        assert_eq!(builtin_default("ps1").as_deref(), Some("native:9"));
        assert_eq!(builtin_default("psm1").as_deref(), Some("native:2"));
        assert_eq!(builtin_default("exe"), None);
    }
    #[test]
    fn defaults_persist_per_type_reset_and_fall_back_to_the_table() {
        let apps = apps();
        let mut hive = Hive::new();
        assert_eq!(
            default_for(&mut hive, &apps, ".txt").unwrap(),
            Some((String::from("native:2"), Source::Builtin))
        );
        assert_eq!(default_for(&mut hive, &apps, ".xyz").unwrap(), None);
        // Always use this app: stored per type under FileAssociations.
        set_default(&mut hive, &apps, "TXT", "exe:/system/bin/SETUP.EXE").unwrap();
        let stored = &hive.keys["software\\doors\\fileassociations\\.txt"]["app"];
        assert_eq!(
            string_of(stored).as_deref(),
            Some("exe:/system/bin/SETUP.EXE")
        );
        assert!(hive.flushes >= 1 && hive.open == 0, "flushed and closed");
        assert_eq!(
            default_for(&mut hive, &apps, ".txt").unwrap(),
            Some((String::from("exe:/system/bin/SETUP.EXE"), Source::User))
        );
        // Unknown types can get a default too.
        set_default(&mut hive, &apps, ".xyz", "native:2").unwrap();
        assert_eq!(
            user_choice(&mut hive, "xyz").unwrap().as_deref(),
            Some("native:2")
        );
        // Refused: unknown apps and bad types.
        for (ext, key) in [("txt", "native:3"), ("a b", "native:2"), ("txt", "Notes")] {
            assert_eq!(
                set_default(&mut hive, &apps, ext, key),
                Err(Error::Service(Code::InvalidRequest)),
                "{ext} {key}"
            );
        }
        // An uninstalled program falls back to the built-in default.
        let mut fewer = Apps::builtin();
        fewer.discover(core::iter::empty());
        assert_eq!(
            default_for(&mut hive, &fewer, "txt").unwrap(),
            Some((String::from("native:2"), Source::Builtin))
        );
        // Reset: an empty value is no choice; resetting an unset type is a no-op.
        reset_default(&mut hive, ".TXT").unwrap();
        assert_eq!(user_choice(&mut hive, "txt").unwrap(), None);
        assert_eq!(
            default_for(&mut hive, &apps, "txt").unwrap(),
            Some((String::from("native:2"), Source::Builtin))
        );
        reset_default(&mut hive, "png").unwrap();
        assert!(
            !hive
                .keys
                .contains_key("software\\doors\\fileassociations\\.png")
        );
        // A hand-edited value that is not a registration key is ignored.
        let key = hive
            .open("Software\\Doors\\FileAssociations\\.md", true, true)
            .unwrap();
        hive.set(&key, "App", string_value("Notepad").unwrap())
            .unwrap();
        hive.set(&key, "Other", RegistryValue::dword(1)).unwrap();
        hive.close(key).unwrap();
        assert_eq!(user_choice(&mut hive, "md").unwrap(), None);
    }
    #[test]
    fn file_type_and_app_pages_list_current_defaults() {
        let apps = apps();
        let mut hive = Hive::new();
        set_default(&mut hive, &apps, "png", "exe:/system/bin/SETUP.EXE").unwrap();
        set_default(&mut hive, &apps, "zzz", "native:10").unwrap();
        let types = file_types(&mut hive, &apps).unwrap();
        let exts: Vec<&str> = types.iter().map(|t| t.ext.as_str()).collect();
        assert_eq!(exts.first(), Some(&".bat"));
        assert_eq!(exts.last(), Some(&".zzz"));
        let mut sorted = exts.clone();
        sorted.sort();
        assert_eq!(exts, sorted);
        assert_eq!(exts.len(), assoc::known_extensions().count() + 1);
        let png = types.iter().find(|t| t.ext == ".png").unwrap();
        assert_eq!(png.default.as_deref(), Some("exe:/system/bin/SETUP.EXE"));
        assert_eq!(png.source, Some(Source::User));
        let txt = types.iter().find(|t| t.ext == ".txt").unwrap();
        assert_eq!(
            (txt.default.as_deref(), txt.source),
            (Some("native:2"), Some(Source::Builtin))
        );
        let preview = app_types(&mut hive, &apps, "native:10").unwrap();
        // Pictures, PDF and the text types Preview shows read-only.
        assert_eq!(preview.len(), 24);
        assert_eq!(preview[0].ext, ".bat");
        assert!(app_types(&mut hive, &apps, "native:1").unwrap().is_empty());
        let empty = Hive::new();
        let mut empty = empty;
        assert_eq!(
            file_types(&mut empty, &apps).unwrap().len(),
            assoc::known_extensions().count()
        );
    }
    #[test]
    fn stored_choices_list_and_resolve_like_default_for() {
        let apps = apps();
        let mut hive = Hive::new();
        assert!(user_choices(&mut hive).unwrap().is_empty());
        set_default(&mut hive, &apps, "png", "native:2").unwrap();
        set_default(&mut hive, &apps, "md", "exe:/system/bin/SETUP.EXE").unwrap();
        reset_default(&mut hive, "md").unwrap();
        let choices = user_choices(&mut hive).unwrap();
        assert_eq!(
            choices,
            [(String::from(".png"), String::from("native:2"))],
            "a reset type is no choice"
        );
        assert_eq!(
            resolve(&apps, ".png", Some("native:2")),
            Some((String::from("native:2"), Source::User))
        );
        assert_eq!(
            resolve(&apps, "bat", Some("native:9")),
            Some((String::from("native:9"), Source::User)),
            "Terminal runs batch files"
        );
        assert_eq!(resolve(&apps, "xyz", None), None);
    }
    #[test]
    fn values_beyond_the_store_capacity_fail_without_partial_writes() {
        let apps = apps();
        let mut hive = Hive::new();
        hive.capacity = 64;
        // "exe:/system/bin/wingui-editor.exe" is 33 UTF-16 units + NUL.
        assert!(set_default(&mut hive, &apps, "txt", "exe:/system/bin/wingui-editor.exe").is_err());
        assert_eq!(hive.open, 0, "the key is closed after a failed write");
        assert_eq!(user_choice(&mut hive, "txt").unwrap(), None);
        set_default(&mut hive, &apps, "txt", "exe:/system/bin/SETUP.EXE").unwrap();
    }
    #[test]
    fn launching_hands_documents_to_apps_and_files_to_programs() {
        let apps = apps();
        assert_eq!(
            launch(apps.get("native:2").unwrap(), "/users/1001/Documents/a.txt"),
            Ok(Launch::Document {
                app: 2,
                payload: b"/users/1001/Documents/a.txt".to_vec()
            })
        );
        assert_eq!(
            launch(
                apps.get("exe:/system/bin/wingui-editor.exe").unwrap(),
                "/users/1001/a b.txt"
            ),
            Ok(Launch::Program {
                path: "/system/bin/wingui-editor.exe".into(),
                cwd: "/users/1001".into(),
                arguments: b"/system/bin/wingui-editor.exe\0C:\\users\\1001\\a b.txt\0".to_vec(),
            })
        );
        assert_eq!(
            launch(apps.get("native:9").unwrap(), "/users/1001/run.bat"),
            Ok(Launch::Document {
                app: 9,
                payload: b"/users/1001/run.bat".to_vec()
            })
        );
        let long = format!("/users/1001/{}.txt", "a".repeat(240));
        assert!(launch(apps.get("native:2").unwrap(), &long).is_err());
        let longer = format!("/users/1001/{}.txt", "a".repeat(100));
        assert!(launch(apps.get("exe:/system/bin/SETUP.EXE").unwrap(), &longer).is_err());
        assert!(launch(apps.get("native:2").unwrap(), "relative.txt").is_err());
        assert_eq!(windows_path("/a.txt"), "C:\\a.txt");
    }
}
