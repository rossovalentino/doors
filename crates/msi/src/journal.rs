//! The rollback script: before the engine changes anything it records how
//! to undo it, in memory and in a journal file under [`FOLDER`]
//! (flushed before the change is made), named by the Installer's
//! `InProgress` registry key. A failed or cancelled installation replays
//! the entries backwards; an installation interrupted by a crash or power
//! loss is replayed the same way by the next Windows Installer session.
//! Backups of overwritten and removed files live beside the journal until
//! the installation commits. Custom actions that ran outside the script
//! are recorded too: rollback cannot undo them unless the package paired
//! them with a rollback custom action, and the log says so.
use crate::execute::Target;
use crate::install::{RegValue, Root};
use alloc::{format, string::String, vec::Vec};

/// Where the Installer keeps rollback scripts and backups. Windows uses
/// C:\Config.Msi; on Doors the drive's root is not writable even for
/// Administrators, so they live beside the package cache.
pub const FOLDER: &str = "C:\\Windows\\Installer\\Config.Msi";
/// The Installer's record of an installation in progress (under HKLM).
pub const IN_PROGRESS: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Installer\\InProgress";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Undo {
    /// A file was created: delete it.
    Created(String),
    /// A file was overwritten or removed; its old bytes are at `backup`.
    Replaced { path: String, backup: String },
    /// A folder was created: remove it if it is empty again.
    Folder(String),
    /// A folder was removed: create it again.
    FolderRemoved(String),
    /// A registry value changed: its previous value (None: it did not
    /// exist).
    Value {
        root: Root,
        key: String,
        name: String,
        previous: Option<RegValue>,
    },
    /// A key was created: delete it (with what the installation put in it).
    Key { root: Root, key: String },
    /// A service was installed: delete it.
    Service(String),
    /// A service was deleted or stopped; start (1) or reinstall (the
    /// command line) is beyond rollback, which records it.
    ServiceChanged(String),
    /// A rollback custom action: run it while rolling back (type, source,
    /// target, CustomActionData).
    Action {
        name: String,
        kind: i32,
        source: String,
        target: String,
        data: String,
    },
    /// A custom action that changed the system outside the script: the
    /// record of what rollback cannot undo.
    Nontransactional(String),
    /// A type library or self-registering module was registered
    /// (`registered`) or unregistered: rollback makes the opposite call.
    Registration {
        kind: Registration,
        path: String,
        /// A type library's help folder.
        help: String,
        registered: bool,
        /// A type library registered for the user.
        per_user: bool,
        /// In the registry's 32-bit view (a 32-bit component's).
        view32: bool,
    },
}
/// What an `Undo::Registration` registered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Registration {
    /// RegisterTypeLib / UnRegisterTypeLib (oleaut32).
    TypeLib,
    /// DllRegisterServer / DllUnregisterServer.
    Module,
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\0' => out.push_str("\\0"),
            c => out.push(c),
        }
    }
    out
}
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some(o) => out.push(o),
            None => (),
        }
    }
    out
}
/// A root's code in the journal: 0-3 (HKCR, HKCU, HKLM, HKU) in the 64-bit
/// view, 10-13 in the 32-bit view.
pub fn root_code(root: Root) -> &'static str {
    match root {
        Root::ClassesRoot => "0",
        Root::CurrentUser => "1",
        Root::LocalMachine => "2",
        Root::Users => "3",
        Root::ClassesRoot32 => "10",
        Root::CurrentUser32 => "11",
        Root::LocalMachine32 => "12",
        Root::Users32 => "13",
    }
}
pub fn root_from(code: &str) -> Option<Root> {
    Some(match code {
        "0" => Root::ClassesRoot,
        "1" => Root::CurrentUser,
        "2" => Root::LocalMachine,
        "3" => Root::Users,
        "10" => Root::ClassesRoot32,
        "11" => Root::CurrentUser32,
        "12" => Root::LocalMachine32,
        "13" => Root::Users32,
        _ => return None,
    })
}
/// A registry value as text: `S` string, `E` expandable, `D` DWORD, `B`
/// hex bytes, `M` NUL-separated strings.
pub fn value_text(v: &RegValue) -> String {
    match v {
        RegValue::String(s) => format!("S{s}"),
        RegValue::Expand(s) => format!("E{s}"),
        RegValue::Dword(d) => format!("D{d}"),
        RegValue::Binary(b) => {
            let mut s = String::from("B");
            for x in b {
                s.push_str(&format!("{x:02x}"));
            }
            s
        }
        RegValue::Multi(m) => {
            let mut s = String::from("M");
            for (i, x) in m.iter().enumerate() {
                if i > 0 {
                    s.push('\0');
                }
                s.push_str(x);
            }
            s
        }
    }
}
pub fn value_from(text: &str) -> Option<RegValue> {
    let (kind, rest) = text.split_at_checked(1)?;
    Some(match kind {
        "S" => RegValue::String(String::from(rest)),
        "E" => RegValue::Expand(String::from(rest)),
        "D" => RegValue::Dword(rest.parse().ok()?),
        "B" => RegValue::Binary(
            rest.as_bytes()
                .chunks(2)
                .map(|p| u8::from_str_radix(core::str::from_utf8(p).ok()?, 16).ok())
                .collect::<Option<Vec<u8>>>()?,
        ),
        "M" => RegValue::Multi(rest.split('\0').map(String::from).collect()),
        _ => return None,
    })
}

impl Undo {
    pub fn line(&self) -> String {
        let f = |parts: &[&str]| -> String {
            let mut s = String::new();
            for (i, p) in parts.iter().enumerate() {
                if i > 0 {
                    s.push('\t');
                }
                s.push_str(&escape(p));
            }
            s.push('\n');
            s
        };
        match self {
            Undo::Created(p) => f(&["F+", p]),
            Undo::Replaced { path, backup } => f(&["F~", path, backup]),
            Undo::Folder(p) => f(&["D+", p]),
            Undo::FolderRemoved(p) => f(&["D-", p]),
            Undo::Value {
                root,
                key,
                name,
                previous,
            } => {
                let v = previous.as_ref().map(value_text);
                f(&[
                    "R=",
                    root_code(*root),
                    key,
                    name,
                    v.as_deref().unwrap_or("-"),
                ])
            }
            Undo::Key { root, key } => f(&["K+", root_code(*root), key]),
            Undo::Service(n) => f(&["S+", n]),
            Undo::ServiceChanged(n) => f(&["S~", n]),
            Undo::Action {
                name,
                kind,
                source,
                target,
                data,
            } => f(&["CA", name, &format!("{kind}"), source, target, data]),
            Undo::Nontransactional(n) => f(&["NT", n]),
            Undo::Registration {
                kind,
                path,
                help,
                registered,
                per_user,
                view32,
            } => f(&[
                "RG",
                match kind {
                    Registration::TypeLib => "T",
                    Registration::Module => "M",
                },
                path,
                help,
                if *registered { "1" } else { "0" },
                if *per_user { "1" } else { "0" },
                if *view32 { "1" } else { "0" },
            ]),
        }
    }
    pub fn parse(line: &str) -> Option<Undo> {
        let parts: Vec<String> = line.split('\t').map(unescape).collect();
        let p = |i: usize| parts.get(i).cloned();
        Some(match parts.first()?.as_str() {
            "F+" => Undo::Created(p(1)?),
            "F~" => Undo::Replaced {
                path: p(1)?,
                backup: p(2)?,
            },
            "D+" => Undo::Folder(p(1)?),
            "D-" => Undo::FolderRemoved(p(1)?),
            "R=" => Undo::Value {
                root: root_from(&p(1)?)?,
                key: p(2)?,
                name: p(3)?,
                previous: match p(4)?.as_str() {
                    "-" => None,
                    v => Some(value_from(v)?),
                },
            },
            "K+" => Undo::Key {
                root: root_from(&p(1)?)?,
                key: p(2)?,
            },
            "S+" => Undo::Service(p(1)?),
            "S~" => Undo::ServiceChanged(p(1)?),
            "CA" => Undo::Action {
                name: p(1)?,
                kind: p(2)?.parse().ok()?,
                source: p(3)?,
                target: p(4)?,
                data: p(5).unwrap_or_default(),
            },
            "NT" => Undo::Nontransactional(p(1)?),
            "RG" => Undo::Registration {
                kind: match p(1)?.as_str() {
                    "T" => Registration::TypeLib,
                    "M" => Registration::Module,
                    _ => return None,
                },
                path: p(2)?,
                help: p(3)?,
                registered: p(4)? == "1",
                per_user: p(5)? == "1",
                view32: p(6).as_deref() == Some("1"),
            },
            _ => return None,
        })
    }
}

/// The installation's rollback script.
#[derive(Clone, Debug, Default)]
pub struct Journal {
    /// The journal file (`<FOLDER>\<n>.rbs`); empty before
    /// InstallInitialize.
    pub path: String,
    pub entries: Vec<Undo>,
    /// Backups made so far (named after the journal).
    backups: u32,
}
impl Journal {
    /// Start the script for `product` (the engine has already written the
    /// InProgress record that names it).
    pub fn begin(target: &mut dyn Target, product: &str, serial: u64) -> Result<Self, u32> {
        if !target.exists(FOLDER) {
            let parent = &FOLDER[..FOLDER.rfind('\\').unwrap_or(0)];
            if !target.exists(parent) {
                target.create_folder(parent)?;
            }
            target.create_folder(FOLDER)?;
        }
        let path = format!("{FOLDER}\\{serial:x}.rbs");
        let header = format!("MSIRBS1\t{}\n", escape(product));
        target.write_file(&path, header.as_bytes())?;
        target.flush(&path);
        Ok(Self {
            path,
            entries: Vec::new(),
            backups: 0,
        })
    }
    pub fn active(&self) -> bool {
        !self.path.is_empty()
    }
    /// Record `undo` durably before the change it undoes.
    pub fn record(&mut self, target: &mut dyn Target, undo: Undo) -> Result<(), u32> {
        if self.active() {
            target.append_file(&self.path, undo.line().as_bytes())?;
            target.flush(&self.path);
        }
        self.entries.push(undo);
        Ok(())
    }
    /// A backup name for a file about to be overwritten or removed.
    pub fn backup_name(&mut self) -> String {
        self.backups += 1;
        let stem = self.path.trim_end_matches(".rbs");
        format!("{stem}-{}.rbf", self.backups)
    }
    /// Read a journal file back (an interrupted installation's).
    pub fn load(target: &mut dyn Target, path: &str) -> Option<Self> {
        let bytes = target.read_file(path).ok()?;
        let text = core::str::from_utf8(&bytes).ok()?;
        let mut lines = text.lines();
        if !lines.next()?.starts_with("MSIRBS1") {
            return None;
        }
        let entries: Vec<Undo> = lines.filter_map(Undo::parse).collect();
        Some(Self {
            path: String::from(path),
            backups: entries
                .iter()
                .filter(|e| matches!(e, Undo::Replaced { .. }))
                .count() as u32,
            entries,
        })
    }
    /// The backups to delete once the installation commits.
    pub fn backups(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|e| match e {
                Undo::Replaced { backup, .. } => Some(backup.clone()),
                _ => None,
            })
            .collect()
    }
    /// Commit: the backups and the script go.
    pub fn commit(&mut self, target: &mut dyn Target) {
        for b in self.backups() {
            if target.exists(&b) {
                let _ = target.remove_file(&b);
            }
        }
        if self.active() && target.exists(&self.path) {
            let _ = target.remove_file(&self.path);
        }
        self.entries.clear();
        self.path.clear();
        // The folder goes once nothing else is in it.
        let _ = target.remove_folder(FOLDER);
    }
}
