//! The tables of the standard actions other than InstallFiles and
//! WriteRegistryValues, planned with the rest of the installation as
//! Windows Installer reads them: COM classes with their ProgIds and AppIds
//! (RegisterClassInfo, RegisterProgIdInfo), MIME types (RegisterMIMEInfo),
//! fonts (RegisterFonts), type libraries (RegisterTypeLibraries),
//! self-registering modules (SelfRegModules), environment variables
//! (WriteEnvironmentStrings), INI files (WriteIniValues, RemoveIniValues),
//! duplicated, isolated and moved files (DuplicateFiles,
//! IsolateComponents, MoveFiles) and qualified components
//! (PublishComponents). Sources: the Windows Installer database table
//! reference on Microsoft Learn (Class, ProgId, AppId, MIME, Font,
//! TypeLib, SelfReg, Environment, IniFile, RemoveIniFile, DuplicateFile,
//! IsolatedComponent, MoveFile and PublishComponent tables).
use super::*;
use crate::execute::packed_guid;

/// Which standard action writes (and removes) a registry row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RegistryGroup {
    /// The Registry table (WriteRegistryValues, RemoveRegistryValues).
    #[default]
    Registry,
    /// Extension and Verb, with their ProgIds (RegisterExtensionInfo,
    /// UnregisterExtensionInfo).
    Extension,
    /// Class and AppId (RegisterClassInfo, UnregisterClassInfo).
    Class,
    /// A class's ProgIds (RegisterProgIdInfo, UnregisterProgIdInfo).
    ProgId,
    /// MIME (RegisterMIMEInfo, UnregisterMIMEInfo).
    Mime,
    /// PublishComponent (PublishComponents, UnpublishComponents).
    Published,
}

/// Font: a font file its component installs, registered under its title
/// (None: the title the font file gives).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontPlan {
    pub component: String,
    pub path: String,
    pub title: Option<String>,
}
/// TypeLib: the type library (the component's key file) and its help
/// folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeLibPlan {
    pub component: String,
    pub path: String,
    pub help: String,
    /// A 32-bit component's library.
    pub view32: bool,
}
/// SelfReg: a module whose DllRegisterServer registers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelfRegPlan {
    pub component: String,
    pub path: String,
}
/// Environment: an environment variable the component sets or removes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvironmentPlan {
    pub component: String,
    pub name: String,
    /// The formatted value, without the `[~]` marker (None: no value).
    pub value: Option<String>,
    /// `*`: a system variable (else the user's).
    pub system: bool,
    /// What installing does: `=` set, `+` set when absent, `!` remove a
    /// matching value; None: nothing.
    pub install: Option<char>,
    /// `-` (or no action at all): removing the component removes it.
    pub remove: bool,
    /// `[~]` first: appended to the existing value after `separator`;
    /// `[~]` last: prepended.
    pub append: bool,
    pub prepend: bool,
    pub separator: String,
}
/// IniFile and RemoveIniFile: a line of an INI file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IniPlan {
    pub component: String,
    pub path: String,
    pub section: String,
    pub key: String,
    pub value: String,
    /// msidbIniFileAction: 0 add or replace the line, 1 add it only when
    /// absent, 2 remove the line, 3 add a tag to a comma-separated list, 4
    /// remove a tag.
    pub action: i32,
}
/// DuplicateFile: a copy of an installed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicatePlan {
    pub component: String,
    pub source: String,
    pub target: String,
}
/// MoveFile: files already on the machine, copied or moved when the
/// component is installed (folders are properties, read when it runs).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MovePlan {
    pub component: String,
    pub source_folder: String,
    /// A file name with wildcards (None: the source property names the
    /// file itself).
    pub source_name: Option<String>,
    pub dest_folder: String,
    pub dest_name: Option<String>,
    /// Options 1: move (else copy).
    pub moving: bool,
}

/// The base-85 GUID form of the Installer's descriptors (as Windows and
/// Wine encode it: each little-endian DWORD of the GUID, five digits, the
/// least significant first).
pub fn base85_guid(guid: &str) -> Option<String> {
    const TABLE: &[u8; 85] =
        b"!$%&'()*+,-.0123456789=?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[]^_`abcdefghijklmnopqrstuvwxyz{}~";
    let bytes = guid_bytes(guid)?;
    let mut out = String::with_capacity(20);
    for chunk in bytes.chunks_exact(4) {
        let mut x = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        for _ in 0..5 {
            out.push(TABLE[(x % 85) as usize] as char);
            x /= 85;
        }
    }
    Some(out)
}
/// A braced GUID's 16 bytes in memory order.
fn guid_bytes(guid: &str) -> Option<[u8; 16]> {
    let hex: Vec<u8> = guid
        .trim_matches(|c| c == '{' || c == '}')
        .bytes()
        .filter(|b| *b != b'-')
        .collect();
    if hex.len() != 32 {
        return None;
    }
    let n = |i: usize| -> Option<u8> {
        u8::from_str_radix(core::str::from_utf8(&hex[i * 2..i * 2 + 2]).ok()?, 16).ok()
    };
    let mut b = [0u8; 16];
    for (i, v) in b.iter_mut().enumerate() {
        *v = n(i)?;
    }
    // Data1, Data2 and Data3 are little-endian in memory.
    b[..4].reverse();
    b[4..6].reverse();
    b[6..8].reverse();
    Some(b)
}

/// The key file of `component` (its installed path), if it has one.
fn key_file(plan: &Plan, component: &str) -> Option<String> {
    let info = plan.component_info.iter().find(|c| c.key == component)?;
    plan.files
        .iter()
        .find(|f| f.key == info.keypath)
        .map(|f| f.target.clone())
}
/// A directory key or property's path (with a trailing backslash).
fn folder(plan: &Plan, name: &str) -> Option<String> {
    let v = plan.directory(name).or_else(|| plan.property(name))?;
    if v.is_empty() {
        return None;
    }
    Some(if v.ends_with('\\') {
        String::from(v)
    } else {
        format!("{v}\\")
    })
}
fn file_name(path: &str) -> &str {
    path.rsplit('\\').next().unwrap_or(path)
}

/// Plan the tables after the components, files, registry and associations.
pub(super) fn plan(db: &Database, plan: &mut Plan, classes_root: Root) -> Result<(), Error> {
    classes(db, plan, classes_root)?;
    mime(db, plan, classes_root)?;
    files(db, plan)?;
    environment(db, plan)?;
    ini(db, plan)?;
    published(db, plan)?;
    Ok(())
}

/// Whose registry rows these are: the action's group, the root (in the
/// component's view) and the component.
#[derive(Clone, Copy)]
struct Owner<'a> {
    group: RegistryGroup,
    root: Root,
    component: &'a str,
}
fn push(
    plan: &mut Plan,
    o: Owner,
    key: String,
    name: Option<&str>,
    value: Option<RegValue>,
    remove_key: bool,
) {
    plan.registry.push(RegistryWrite {
        root: o.root,
        key,
        name: name.map(String::from),
        value,
        component: String::from(o.component),
        remove_key,
        group: o.group,
    });
}
fn s(v: &str) -> Option<RegValue> {
    Some(RegValue::String(String::from(v)))
}

/// Class: HKCR\CLSID\{clsid} with its server under the context's key (the
/// component's key file, its Argument after it), ProgID,
/// VersionIndependentProgID, AppID, InprocHandler and the FileType masks;
/// the class's ProgIds with CLSID and CurVer; AppId's settings.
fn classes(db: &Database, plan: &mut Plan, classes_root: Root) -> Result<(), Error> {
    let Some(t) = opt_table(db, "Class")? else {
        return Ok(());
    };
    let progids = opt_table(db, "ProgId")?;
    let appids = opt_table(db, "AppId")?;
    let mut appids_done: Vec<String> = Vec::new();
    for r in 0..t.rows.len() {
        let component = t.get(r, "Component_").str().unwrap_or("").to_string();
        if !plan.components.contains(&component) {
            continue;
        }
        let clsid = t.get(r, "CLSID").str().unwrap_or("").to_string();
        let context = t.get(r, "Context").str().unwrap_or("").to_string();
        if clsid.is_empty() || context.is_empty() {
            continue;
        }
        let root = classes_root.in_view(plan.component_is_32bit(&component));
        let base = format!("Software\\Classes\\CLSID\\{clsid}");
        let o = Owner {
            group: RegistryGroup::Class,
            root,
            component: &component,
        };
        let description = t.get(r, "Description").str().map(|d| plan.format(d));
        match &description {
            Some(d) => push(plan, o, base.clone(), None, s(d), true),
            None => push(plan, o, base.clone(), None, None, true),
        }
        // msidbClassAttributesRelativePath: the file name only.
        let file = key_file(plan, &component).unwrap_or_default();
        let mut server = if t.get(r, "Attributes").int().unwrap_or(0) & 1 != 0 {
            String::from(file_name(&file))
        } else {
            file
        };
        if let Some(a) = t.get(r, "Argument").str() {
            let a = plan.format(a);
            if !a.is_empty() {
                server.push(' ');
                server.push_str(&a);
            }
        }
        push(
            plan,
            o,
            format!("{base}\\{context}"),
            None,
            s(&server),
            false,
        );
        if let Some(handler) = t.get(r, "DefInprocHandler").str() {
            match handler {
                "1" => push(
                    plan,
                    o,
                    format!("{base}\\InprocHandler"),
                    None,
                    s("ole2.dll"),
                    false,
                ),
                "2" => push(
                    plan,
                    o,
                    format!("{base}\\InprocHandler32"),
                    None,
                    s("ole32.dll"),
                    false,
                ),
                "3" => {
                    push(
                        plan,
                        o,
                        format!("{base}\\InprocHandler"),
                        None,
                        s("ole2.dll"),
                        false,
                    );
                    push(
                        plan,
                        o,
                        format!("{base}\\InprocHandler32"),
                        None,
                        s("ole32.dll"),
                        false,
                    );
                }
                other => {
                    let h = plan.format(other);
                    push(
                        plan,
                        o,
                        format!("{base}\\InprocHandler32"),
                        None,
                        s(&h),
                        false,
                    );
                }
            }
        }
        if let Some(appid) = t.get(r, "AppId_").str().map(String::from) {
            push(plan, o, base.clone(), Some("AppID"), s(&appid), false);
            if let Some(a) = &appids
                && let Some(row) = a.find(&appid)
                && !appids_done.contains(&appid)
            {
                appids_done.push(appid.clone());
                let key = format!("Software\\Classes\\AppID\\{appid}");
                push(plan, o, key.clone(), None, None, true);
                for column in [
                    "RemoteServerName",
                    "LocalService",
                    "ServiceParameters",
                    "DllSurrogate",
                ] {
                    if let Some(v) = a.get(row, column).str() {
                        let v = plan.format(v);
                        push(plan, o, key.clone(), Some(column), s(&v), false);
                    }
                }
                if a.get(row, "ActivateAtStorage").int() == Some(1) {
                    push(
                        plan,
                        o,
                        key.clone(),
                        Some("ActivateAtStorage"),
                        s("Y"),
                        false,
                    );
                }
                if a.get(row, "RunAsInteractiveUser").int() == Some(1) {
                    push(
                        plan,
                        o,
                        key.clone(),
                        Some("RunAs"),
                        s("Interactive User"),
                        false,
                    );
                }
            }
        }
        if let Some(mask) = t.get(r, "FileTypeMask").str() {
            for (i, m) in mask.split(';').filter(|m| !m.is_empty()).enumerate() {
                push(
                    plan,
                    o,
                    format!("Software\\Classes\\FileType\\{clsid}\\{i}"),
                    None,
                    s(m),
                    true,
                );
            }
        }
        // The class's ProgIds: the default one, its version-independent
        // child (ProgId_Parent), and the others that name the class.
        let Some(p) = &progids else { continue };
        let default = t.get(r, "ProgId_Default").str().map(String::from);
        let named: Vec<usize> = (0..p.rows.len())
            .filter(|&i| p.get(i, "Class_").str() == Some(clsid.as_str()))
            .collect();
        if let Some(d) = &default {
            push(plan, o, format!("{base}\\ProgID"), None, s(d), false);
            if let Some(child) =
                (0..p.rows.len()).find(|&i| p.get(i, "ProgId_Parent").str() == Some(d.as_str()))
            {
                let v = p.get(child, "ProgId").str().unwrap_or("").to_string();
                push(
                    plan,
                    o,
                    format!("{base}\\VersionIndependentProgID"),
                    None,
                    s(&v),
                    false,
                );
            }
        }
        let op = Owner {
            group: RegistryGroup::ProgId,
            ..o
        };
        for i in named {
            let progid = p.get(i, "ProgId").str().unwrap_or("").to_string();
            let key = format!("Software\\Classes\\{progid}");
            let d = p.get(i, "Description").str().map(|d| plan.format(d));
            push(plan, op, key.clone(), None, d.as_deref().and_then(s), true);
            push(plan, op, format!("{key}\\CLSID"), None, s(&clsid), false);
            for c in (0..p.rows.len())
                .filter(|&c| p.get(c, "ProgId_Parent").str() == Some(progid.as_str()))
            {
                let child = p.get(c, "ProgId").str().unwrap_or("").to_string();
                let ckey = format!("Software\\Classes\\{child}");
                let d = p.get(c, "Description").str().map(|d| plan.format(d));
                push(plan, op, ckey.clone(), None, d.as_deref().and_then(s), true);
                push(plan, op, format!("{ckey}\\CLSID"), None, s(&clsid), false);
                push(plan, op, format!("{ckey}\\CurVer"), None, s(&progid), false);
            }
        }
    }
    Ok(())
}

/// MIME: HKCR\MIME\Database\Content Type\<type> with the extension (and
/// the class) of an installed Extension row.
fn mime(db: &Database, plan: &mut Plan, classes_root: Root) -> Result<(), Error> {
    let (Some(t), Some(ext)) = (opt_table(db, "MIME")?, opt_table(db, "Extension")?) else {
        return Ok(());
    };
    for r in 0..t.rows.len() {
        let extension = t.get(r, "Extension_").str().unwrap_or("").to_string();
        let Some(component) = (0..ext.rows.len())
            .find(|&i| ext.get(i, "Extension").str() == Some(extension.as_str()))
            .and_then(|i| ext.get(i, "Component_").str().map(String::from))
        else {
            continue;
        };
        if !plan.components.contains(&component) {
            continue;
        }
        let root = classes_root.in_view(plan.component_is_32bit(&component));
        let key = format!(
            "Software\\Classes\\MIME\\Database\\Content Type\\{}",
            t.get(r, "ContentType").str().unwrap_or("")
        );
        let o = Owner {
            group: RegistryGroup::Mime,
            root,
            component: &component,
        };
        push(
            plan,
            o,
            key.clone(),
            Some("Extension"),
            s(&format!(".{extension}")),
            true,
        );
        if let Some(clsid) = t.get(r, "CLSID").str() {
            push(plan, o, key, Some("CLSID"), s(clsid), false);
        }
    }
    Ok(())
}

/// Font, TypeLib, SelfReg, DuplicateFile, IsolatedComponent and MoveFile.
fn files(db: &Database, plan: &mut Plan) -> Result<(), Error> {
    let file_of = |plan: &Plan, key: &str| plan.files.iter().find(|f| f.key == key).cloned();
    if let Some(t) = opt_table(db, "Font")? {
        for r in 0..t.rows.len() {
            if let Some(f) = file_of(plan, t.get(r, "File_").str().unwrap_or("")) {
                let title = t.get(r, "FontTitle").str().map(|v| plan.format(v));
                plan.fonts.push(FontPlan {
                    component: f.component,
                    path: f.target,
                    title,
                });
            }
        }
    }
    if let Some(t) = opt_table(db, "TypeLib")? {
        for r in 0..t.rows.len() {
            let component = t.get(r, "Component_").str().unwrap_or("").to_string();
            if !plan.components.contains(&component) {
                continue;
            }
            let Some(path) = key_file(plan, &component) else {
                continue;
            };
            let help = t
                .get(r, "Directory_")
                .str()
                .and_then(|d| folder(plan, d))
                .map(|d| String::from(d.trim_end_matches('\\')))
                .unwrap_or_default();
            if !plan.typelibs.iter().any(|x| x.path == path) {
                let view32 = plan.component_is_32bit(&component);
                plan.typelibs.push(TypeLibPlan {
                    component,
                    path,
                    help,
                    view32,
                });
            }
        }
    }
    if let Some(t) = opt_table(db, "SelfReg")? {
        for r in 0..t.rows.len() {
            if let Some(f) = file_of(plan, t.get(r, "File_").str().unwrap_or("")) {
                plan.self_reg.push(SelfRegPlan {
                    component: f.component,
                    path: f.target,
                });
            }
        }
    }
    if let Some(t) = opt_table(db, "DuplicateFile")? {
        for r in 0..t.rows.len() {
            let component = t.get(r, "Component_").str().unwrap_or("").to_string();
            if !plan.components.contains(&component) {
                continue;
            }
            let Some(source) = file_of(plan, t.get(r, "File_").str().unwrap_or("")) else {
                continue;
            };
            // No DestFolder: the source file's own folder.
            let dir = match t.get(r, "DestFolder").str() {
                Some(d) => folder(plan, d),
                None => source
                    .target
                    .rfind('\\')
                    .map(|i| String::from(&source.target[..=i])),
            };
            let Some(dir) = dir else { continue };
            let name = match t.get(r, "DestName").str() {
                Some(n) => String::from(long_name(n)),
                None => String::from(file_name(&source.target)),
            };
            let target = format!("{dir}{name}");
            // A file duplicated onto itself is skipped, as Windows does.
            if target.eq_ignore_ascii_case(&source.target) {
                continue;
            }
            plan.duplicates.push(DuplicatePlan {
                component,
                source: source.target,
                target,
            });
        }
    }
    // An isolated component: InstallFiles copies its files again into the
    // application's folder, with the application's `.local` file (DLL
    // redirection); they go with the application's component.
    if let Some(t) = opt_table(db, "IsolatedComponent")? {
        for r in 0..t.rows.len() {
            let shared = t.get(r, "Component_Shared").str().unwrap_or("").to_string();
            let application = t
                .get(r, "Component_Application")
                .str()
                .unwrap_or("")
                .to_string();
            if !plan.components.contains(&shared) || !plan.components.contains(&application) {
                continue;
            }
            let Some(exe) = key_file(plan, &application) else {
                continue;
            };
            let dir = &exe[..exe.rfind('\\').map_or(0, |i| i + 1)];
            let copies: Vec<FileCopy> = plan
                .files
                .iter()
                .filter(|f| f.component == shared)
                .map(|f| FileCopy {
                    key: format!("{}:{application}", f.key),
                    target: format!("{dir}{}", file_name(&f.target)),
                    component: application.clone(),
                    ..f.clone()
                })
                .collect();
            plan.files.extend(copies);
            plan.files.push(FileCopy {
                key: format!("{application}.local"),
                source: Source::Empty,
                target: format!("{exe}.local"),
                size: 0,
                component: application.clone(),
                version: String::new(),
                attributes: 0,
            });
        }
    }
    if let Some(t) = opt_table(db, "MoveFile")? {
        for r in 0..t.rows.len() {
            let component = t.get(r, "Component_").str().unwrap_or("").to_string();
            if !plan.components.contains(&component) {
                continue;
            }
            plan.moves.push(MovePlan {
                component,
                source_folder: t.get(r, "SourceFolder").str().unwrap_or("").to_string(),
                source_name: t
                    .get(r, "SourceName")
                    .str()
                    .map(|n| String::from(long_name(n))),
                dest_folder: t.get(r, "DestFolder").str().unwrap_or("").to_string(),
                dest_name: t
                    .get(r, "DestName")
                    .str()
                    .map(|n| String::from(long_name(n))),
                moving: t.get(r, "Options").int().unwrap_or(0) & 1 != 0,
            });
        }
    }
    Ok(())
}

/// Environment: the Name's prefixes (`=`, `+`, `-`, `!`, `*`) and the
/// Value's `[~]` (append or prepend), as the Environment table defines
/// them.
fn environment(db: &Database, plan: &mut Plan) -> Result<(), Error> {
    let Some(t) = opt_table(db, "Environment")? else {
        return Ok(());
    };
    for r in 0..t.rows.len() {
        let component = t.get(r, "Component_").str().unwrap_or("").to_string();
        if !plan.components.contains(&component) {
            continue;
        }
        let raw = t.get(r, "Name").str().unwrap_or("");
        let name = raw.trim_start_matches(['=', '+', '-', '!', '*']);
        if name.is_empty() {
            continue;
        }
        let flags = &raw[..raw.len() - name.len()];
        let has = |c: char| flags.contains(c);
        // `=`, `+` and `!` exclude each other (an invalid row is skipped).
        if [has('='), has('+'), has('!')]
            .iter()
            .filter(|x| **x)
            .count()
            > 1
        {
            continue;
        }
        // `-` alone only removes the variable with the component; no
        // action prefix at all sets it and removes it with the component.
        let (install, remove) = if has('!') {
            (Some('!'), has('-'))
        } else if has('+') {
            (Some('+'), has('-'))
        } else if has('=') {
            (Some('='), has('-'))
        } else if has('-') {
            (None, true)
        } else {
            (Some('='), true)
        };
        let mut value = t.get(r, "Value").str().map(|v| plan.format(v));
        // `+` cannot add to a list.
        if install == Some('+') && value.as_deref().is_some_and(|v| v.contains('\0')) {
            continue;
        }
        // `[~]` formats to NUL: the existing value's place.
        let (mut append, mut prepend, mut separator) = (false, false, String::new());
        if let Some(v) = &value {
            if let Some(rest) = v.strip_prefix('\0') {
                append = true;
                let sep: String = rest.chars().take(1).collect();
                value = Some(String::from(&rest[sep.len()..]));
                separator = sep;
            } else if let Some(rest) = v.strip_suffix('\0') {
                prepend = true;
                let sep: String = rest.chars().last().map(String::from).unwrap_or_default();
                value = Some(String::from(&rest[..rest.len() - sep.len()]));
                separator = sep;
            } else if v.contains('\0') {
                value = Some(v.replace('\0', ""));
            }
        }
        plan.environment.push(EnvironmentPlan {
            component,
            name: String::from(name),
            value,
            system: flags.contains('*'),
            install,
            remove,
            append,
            prepend,
            separator,
        });
    }
    Ok(())
}

/// IniFile and RemoveIniFile: the file (FileName in DirProperty, else the
/// Windows folder), section, key, value and action.
fn ini(db: &Database, plan: &mut Plan) -> Result<(), Error> {
    for (table, remove) in [("IniFile", false), ("RemoveIniFile", true)] {
        let Some(t) = opt_table(db, table)? else {
            continue;
        };
        for r in 0..t.rows.len() {
            let component = t.get(r, "Component_").str().unwrap_or("").to_string();
            if !plan.components.contains(&component) {
                continue;
            }
            let dir = t
                .get(r, "DirProperty")
                .str()
                .and_then(|d| folder(plan, d))
                .or_else(|| folder(plan, "WindowsFolder"))
                .unwrap_or_else(|| String::from("C:\\Windows\\"));
            let name = long_name(t.get(r, "FileName").str().unwrap_or(""));
            let action = t
                .get(r, "Action")
                .int()
                .unwrap_or(if remove { 2 } else { 0 });
            plan.ini.push(IniPlan {
                component,
                path: format!("{dir}{name}"),
                section: plan.format(t.get(r, "Section").str().unwrap_or("")),
                key: plan.format(t.get(r, "Key").str().unwrap_or("")),
                value: t
                    .get(r, "Value")
                    .str()
                    .map(|v| plan.format(v))
                    .unwrap_or_default(),
                action,
            });
        }
    }
    Ok(())
}

/// PublishComponent: the qualifier under the category's Components key,
/// a multi-string of the component's descriptor (product, feature and
/// component in base 85) and its AppData.
fn published(db: &Database, plan: &mut Plan) -> Result<(), Error> {
    let Some(t) = opt_table(db, "PublishComponent")? else {
        return Ok(());
    };
    let (root, base) = if plan.all_users {
        (
            Root::LocalMachine,
            "Software\\Classes\\Installer\\Components",
        )
    } else {
        (
            Root::CurrentUser,
            "Software\\Microsoft\\Installer\\Components",
        )
    };
    let product = base85_guid(&plan.product_code);
    for r in 0..t.rows.len() {
        let component = t.get(r, "Component_").str().unwrap_or("").to_string();
        if !plan.components.contains(&component) {
            continue;
        }
        let Some(category) = t.get(r, "ComponentId").str().and_then(packed_guid) else {
            continue;
        };
        let Some(guid) = plan
            .component_info
            .iter()
            .find(|c| c.key == component)
            .and_then(|c| base85_guid(&c.guid))
        else {
            continue;
        };
        let Some(product) = &product else { continue };
        let feature = t.get(r, "Feature_").str().unwrap_or("");
        let qualifier = plan.format(t.get(r, "Qualifier").str().unwrap_or(""));
        let app_data = t
            .get(r, "AppData")
            .str()
            .map(|v| plan.format(v))
            .unwrap_or_default();
        let descriptor = format!("{product}{feature}>{guid}{app_data}");
        let o = Owner {
            group: RegistryGroup::Published,
            root,
            component: &component,
        };
        push(
            plan,
            o,
            format!("{base}\\{category}"),
            Some(&qualifier),
            Some(RegValue::Multi(alloc::vec![descriptor])),
            false,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptors_encode_guids_in_base_85() {
        // Five digits per little-endian DWORD, least significant first.
        let zero = base85_guid("{00000000-0000-0000-0000-000000000000}").unwrap();
        assert_eq!(zero, "!!!!!!!!!!!!!!!!!!!!");
        let one = base85_guid("{00000001-0000-0000-0000-000000000000}").unwrap();
        assert_eq!(&one[..5], "$!!!!");
        // 85 = "!$" in two digits.
        let e = base85_guid("{00000055-0000-0000-0000-000000000000}").unwrap();
        assert_eq!(&e[..5], "!$!!!");
        assert_eq!(base85_guid("{bad}"), None);
    }
}
