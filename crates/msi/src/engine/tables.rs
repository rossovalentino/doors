//! The standard actions of the tables `install::tables` plans: fonts
//! (RegisterFonts), type libraries (RegisterTypeLibraries through
//! oleaut32), self-registering modules (SelfRegModules), environment
//! variables (WriteEnvironmentStrings, RemoveEnvironmentStrings), INI
//! files (WriteIniValues, RemoveIniValues), duplicated files
//! (DuplicateFiles, RemoveDuplicateFiles) and moved files (MoveFiles).
//! Every change is journaled; a registration a module or type library
//! makes itself is undone by the opposite call.
use super::*;
use crate::install::{EnvironmentPlan, IniPlan};
use crate::journal::Registration;

/// Where Windows keeps the fonts.
const FONTS: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Fonts";
const SYSTEM_ENVIRONMENT: &str = "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment";

impl Session<'_, '_> {
    /// Windows Installer registers fonts per machine, whatever the
    /// installation's scope.
    fn fonts_root(&self) -> Root {
        Root::LocalMachine
    }
    /// A font's registry value: its title (the Font table's, else the one
    /// the font file gives) and the file (its name when it is in the Fonts
    /// folder, else its path).
    fn font_value(&mut self, path: &str, title: Option<&str>) -> Option<(String, String)> {
        let title = match title {
            Some(t) if !t.is_empty() => String::from(t),
            _ => font_title(&self.target.read_file(path).ok()?)?,
        };
        let fonts = String::from(self.property("FontsFolder"));
        let data = match path.rsplit_once('\\') {
            Some((dir, name))
                if !fonts.is_empty() && dir.eq_ignore_ascii_case(fonts.trim_end_matches('\\')) =>
            {
                String::from(name)
            }
            _ => String::from(path),
        };
        Some((title, data))
    }
    pub(super) fn register_fonts(&mut self) -> Result<(), u32> {
        self.ensure_costed()?;
        let root = self.fonts_root();
        let fonts: Vec<_> = self
            .state
            .plan
            .fonts
            .iter()
            .filter(|f| self.state.install_set.contains(&f.component))
            .cloned()
            .collect();
        for f in fonts {
            match self.font_value(&f.path, f.title.as_deref()) {
                Some((title, data)) => {
                    self.log(&format!("RegisterFonts: {title} = {data}."));
                    self.j_set_value(root, FONTS, Some(&title), &RegValue::String(data))?;
                }
                None => self.log(&format!("RegisterFonts: {} has no font title.", f.path)),
            }
        }
        Ok(())
    }
    pub(super) fn unregister_fonts(&mut self) -> Result<(), u32> {
        self.ensure_costed()?;
        let root = self.fonts_root();
        let removing = self.removing();
        let fonts: Vec<_> = self
            .installed()
            .fonts
            .iter()
            .filter(|f| removing.contains(&f.component))
            .cloned()
            .collect();
        for f in fonts {
            if let Some((title, _)) = self.font_value(&f.path, f.title.as_deref()) {
                self.j_delete_value(root, FONTS, Some(&title))?;
            }
        }
        Ok(())
    }

    /// RegisterTypeLibraries (`register`) or UnregisterTypeLibraries: each
    /// type library of the installed (removed) components, through
    /// oleaut32 as Windows Installer does; rollback makes the opposite
    /// call. A failure is reported and logged, and the installation goes
    /// on.
    pub(super) fn register_typelibs(&mut self, ui: &mut dyn Ui, register: bool) -> Result<(), u32> {
        self.ensure_costed()?;
        let per_user = !self.state.all_users();
        let libs: Vec<_> = if register {
            self.state
                .plan
                .typelibs
                .iter()
                .filter(|t| self.state.install_set.contains(&t.component))
                .cloned()
                .collect()
        } else {
            let removing = self.removing();
            self.installed()
                .typelibs
                .iter()
                .filter(|t| removing.contains(&t.component))
                .cloned()
                .collect()
        };
        for lib in libs {
            // Journaled once it happened (rollback must not undo another
            // product's registration of the same library).
            match self
                .target
                .register_typelib(&lib.path, &lib.help, per_user, lib.view32, register)
            {
                Ok(()) => self.record(Undo::Registration {
                    kind: Registration::TypeLib,
                    path: lib.path.clone(),
                    help: lib.help.clone(),
                    registered: register,
                    per_user,
                    view32: lib.view32,
                })?,
                Err(e) => {
                    self.log(&format!(
                        "{} type library {} failed: {e:#x}.",
                        if register {
                            "Registering"
                        } else {
                            "Unregistering"
                        },
                        lib.path
                    ));
                    if register {
                        let path = lib.path.clone();
                        self.warning(ui, 1911, &[&path]);
                    }
                }
            }
        }
        Ok(())
    }

    /// SelfRegModules (`register`: DllRegisterServer) or SelfUnregModules
    /// (DllUnregisterServer) of the SelfReg table's modules; rollback makes
    /// the opposite call. A module that fails is reported and logged
    /// (error 1904 or 1905), and the installation goes on, as Windows
    /// Installer's quiet installations do.
    pub(super) fn self_register(&mut self, ui: &mut dyn Ui, register: bool) -> Result<(), u32> {
        self.ensure_costed()?;
        let modules: Vec<_> = if register {
            self.state
                .plan
                .self_reg
                .iter()
                .filter(|m| self.state.install_set.contains(&m.component))
                .cloned()
                .collect()
        } else {
            let removing = self.removing();
            self.installed()
                .self_reg
                .iter()
                .filter(|m| removing.contains(&m.component))
                .cloned()
                .collect()
        };
        for m in modules {
            self.log(&format!(
                "{} {}.",
                if register { "SelfReg" } else { "SelfUnreg" },
                m.path
            ));
            let result = self.target.self_register(&m.path, register);
            if result.is_ok() {
                self.record(Undo::Registration {
                    kind: Registration::Module,
                    path: m.path.clone(),
                    help: String::new(),
                    registered: register,
                    per_user: false,
                    view32: false,
                })?;
            }
            if let Err(e) = result {
                self.log(&format!(
                    "Module {} failed to {}. HRESULT {e:#x}.",
                    m.path,
                    if register { "register" } else { "unregister" }
                ));
                let path = m.path.clone();
                let hr = format!("{e:#x}");
                self.warning(ui, if register { 1904 } else { 1905 }, &[&path, &hr]);
            }
        }
        Ok(())
    }

    // ---- environment ------------------------------------------------------

    fn environment_key(e: &EnvironmentPlan) -> (Root, &'static str) {
        if e.system {
            (Root::LocalMachine, SYSTEM_ENVIRONMENT)
        } else {
            (Root::CurrentUser, "Environment")
        }
    }
    fn environment_value(&mut self, e: &EnvironmentPlan) -> Option<RegValue> {
        let (root, key) = Self::environment_key(e);
        self.target.value(root, key, &e.name)
    }
    /// Set a variable, keeping an expandable value expandable (and making a
    /// new one with `%` expandable).
    fn set_environment(
        &mut self,
        e: &EnvironmentPlan,
        old: Option<&RegValue>,
        text: String,
    ) -> Result<(), u32> {
        let (root, key) = Self::environment_key(e);
        let value = if matches!(old, Some(RegValue::Expand(_))) || text.contains('%') {
            RegValue::Expand(text)
        } else {
            RegValue::String(text)
        };
        self.j_set_value(root, key, Some(&e.name), &value)
    }
    /// WriteEnvironmentStrings: `=` sets (appending or prepending to the
    /// list with `[~]`, once), `+` sets a variable that is absent.
    pub(super) fn write_environment(&mut self) -> Result<(), u32> {
        self.ensure_costed()?;
        let rows: Vec<_> = self
            .state
            .plan
            .environment
            .iter()
            .filter(|e| {
                matches!(e.install, Some('=' | '+'))
                    && self.state.install_set.contains(&e.component)
            })
            .cloned()
            .collect();
        let mut changed = false;
        for e in rows {
            let old = self.environment_value(&e);
            let current = old.as_ref().and_then(reg_text);
            if e.install == Some('+') && current.is_some() {
                continue;
            }
            let part = e.value.clone().unwrap_or_default();
            // `=` with no value removes the variable.
            if e.install == Some('=') && part.is_empty() && !(e.append || e.prepend) {
                if current.is_some() {
                    let (root, key) = Self::environment_key(&e);
                    self.j_delete_value(root, key, Some(&e.name))?;
                    changed = true;
                }
                continue;
            }
            let new = match (&current, e.append || e.prepend) {
                (Some(cur), true) if !cur.is_empty() => {
                    if list_contains(cur, &part, &e.separator) {
                        continue;
                    }
                    if e.append {
                        format!("{cur}{}{part}", e.separator)
                    } else {
                        format!("{part}{}{cur}", e.separator)
                    }
                }
                _ => part,
            };
            if current.as_deref() == Some(new.as_str()) {
                continue;
            }
            self.log(&format!("WriteEnvironmentStrings: {}={new}.", e.name));
            self.set_environment(&e, old.as_ref(), new)?;
            changed = true;
        }
        if changed {
            self.target.environment_changed();
        }
        Ok(())
    }
    /// RemoveEnvironmentStrings: a removed component's variables with `-`
    /// (its part of a `[~]` list, or the variable when it still holds the
    /// component's value), and an installed component's `!` variables
    /// (when they match, or always without a value).
    pub(super) fn remove_environment(&mut self) -> Result<(), u32> {
        self.ensure_costed()?;
        let removing = self.removing();
        let mut rows: Vec<EnvironmentPlan> = self
            .installed()
            .environment
            .iter()
            .filter(|e| e.remove && removing.contains(&e.component))
            .cloned()
            .collect();
        rows.extend(
            self.state
                .plan
                .environment
                .iter()
                .filter(|e| e.install == Some('!') && self.state.install_set.contains(&e.component))
                .cloned(),
        );
        let mut changed = false;
        for e in rows {
            let old = self.environment_value(&e);
            let Some(current) = old.as_ref().and_then(reg_text) else {
                continue;
            };
            let (root, key) = Self::environment_key(&e);
            let part = e.value.clone().unwrap_or_default();
            if e.append || e.prepend {
                let rest = list_without(&current, &part, &e.separator);
                if rest == current {
                    continue;
                }
                if rest.is_empty() {
                    self.j_delete_value(root, key, Some(&e.name))?;
                } else {
                    self.set_environment(&e, old.as_ref(), rest)?;
                }
            } else if part.is_empty() || current.eq_ignore_ascii_case(&part) {
                self.j_delete_value(root, key, Some(&e.name))?;
            } else {
                continue;
            }
            self.log(&format!("RemoveEnvironmentStrings: {}.", e.name));
            changed = true;
        }
        if changed {
            self.target.environment_changed();
        }
        Ok(())
    }

    // ---- INI files --------------------------------------------------------

    /// Apply `rows` to their INI files (each file read once, written once,
    /// journaled).
    fn edit_ini(&mut self, rows: Vec<(IniPlan, IniEdit)>) -> Result<(), u32> {
        let mut files: Vec<String> = Vec::new();
        for (r, _) in &rows {
            if !files.iter().any(|f| f.eq_ignore_ascii_case(&r.path)) {
                files.push(r.path.clone());
            }
        }
        for path in files {
            let existing = self.target.read_file(&path).ok();
            let (mut text, wide) = match &existing {
                Some(bytes) => decode_ini(bytes),
                None => (String::new(), false),
            };
            let original = text.clone();
            for (r, edit) in rows
                .iter()
                .filter(|(r, _)| r.path.eq_ignore_ascii_case(&path))
            {
                text = ini_apply(&text, &r.section, &r.key, &r.value, *edit);
            }
            if text == original || (existing.is_none() && text.is_empty()) {
                continue;
            }
            self.log(&format!("INI file {path} changed."));
            self.j_write_file(&path, &encode_ini(&text, wide))?;
        }
        Ok(())
    }
    /// WriteIniValues: an installed component's IniFile lines (action 0,
    /// 1 and 3).
    pub(super) fn write_ini(&mut self) -> Result<(), u32> {
        self.ensure_costed()?;
        let rows: Vec<(IniPlan, IniEdit)> = self
            .state
            .plan
            .ini
            .iter()
            .filter(|r| self.state.install_set.contains(&r.component))
            .filter_map(|r| {
                let edit = match r.action {
                    0 => IniEdit::Set,
                    1 => IniEdit::SetAbsent,
                    3 => IniEdit::AddTag,
                    _ => return None,
                };
                Some((r.clone(), edit))
            })
            .collect();
        self.edit_ini(rows)
    }
    /// RemoveIniValues: an installed component's RemoveIniFile lines
    /// (action 2 and 4), and a removed component's IniFile lines.
    pub(super) fn remove_ini(&mut self) -> Result<(), u32> {
        self.ensure_costed()?;
        let removing = self.removing();
        let mut rows: Vec<(IniPlan, IniEdit)> = self
            .state
            .plan
            .ini
            .iter()
            .filter(|r| self.state.install_set.contains(&r.component))
            .filter_map(|r| match r.action {
                2 => Some((r.clone(), IniEdit::Remove)),
                4 => Some((r.clone(), IniEdit::RemoveTag)),
                _ => None,
            })
            .collect();
        rows.extend(
            self.installed()
                .ini
                .iter()
                .filter(|r| removing.contains(&r.component))
                .filter_map(|r| match r.action {
                    0 | 1 => Some((r.clone(), IniEdit::Remove)),
                    3 => Some((r.clone(), IniEdit::RemoveTag)),
                    _ => None,
                }),
        );
        self.edit_ini(rows)
    }

    // ---- duplicated and moved files ---------------------------------------

    pub(super) fn duplicate_files(&mut self, ui: &mut dyn Ui) -> Result<(), u32> {
        self.ensure_costed()?;
        let copies: Vec<_> = self
            .state
            .plan
            .duplicates
            .iter()
            .filter(|d| self.state.install_set.contains(&d.component))
            .cloned()
            .collect();
        for d in copies {
            let data = match self.target.read_file(&d.source) {
                Ok(b) => b,
                Err(_) => {
                    self.log(&format!("DuplicateFiles: {} is not installed.", d.source));
                    continue;
                }
            };
            self.log(&format!("Duplicate: {} -> {}", d.source, d.target));
            if let Err(e) = self.j_write_file(&d.target, &data) {
                self.log(&format!("Writing {} failed: system error {e}.", d.target));
                let target = d.target.clone();
                self.error(ui, 1310, &[&target], msg::MB_OK);
                return Err(code::INSTALL_FAILURE);
            }
        }
        Ok(())
    }
    pub(super) fn remove_duplicate_files(&mut self) -> Result<(), u32> {
        self.ensure_costed()?;
        let removing = self.removing();
        let paths: Vec<String> = self
            .installed()
            .duplicates
            .iter()
            .filter(|d| removing.contains(&d.component))
            .map(|d| d.target.clone())
            .collect();
        for p in paths {
            self.j_remove_file(&p)?;
        }
        Ok(())
    }
    /// MoveFiles: files already on the machine copied (or moved) into the
    /// destination folder; they stay when the product is removed.
    pub(super) fn move_files(&mut self, ui: &mut dyn Ui) -> Result<(), u32> {
        self.ensure_costed()?;
        let moves: Vec<_> = self
            .state
            .plan
            .moves
            .iter()
            .filter(|m| self.state.install_set.contains(&m.component))
            .cloned()
            .collect();
        for m in moves {
            let source = String::from(self.property(&m.source_folder));
            let dest = String::from(self.property(&m.dest_folder));
            if source.is_empty() || dest.is_empty() {
                continue;
            }
            let dest = String::from(dest.trim_end_matches('\\'));
            // The files: the pattern in the source folder, or the file the
            // source property names.
            let files: Vec<String> = match &m.source_name {
                Some(pattern) => {
                    let dir = String::from(source.trim_end_matches('\\'));
                    self.target
                        .list(&dir)
                        .into_iter()
                        .filter(|n| wildcard(pattern, n))
                        .map(|n| format!("{dir}\\{n}"))
                        .collect()
                }
                None => alloc::vec![source.clone()],
            };
            let single = files.len() == 1;
            for from in files {
                let name = match (&m.dest_name, single) {
                    (Some(n), true) => n.clone(),
                    _ => String::from(from.rsplit('\\').next().unwrap_or(&from)),
                };
                let to = format!("{dest}\\{name}");
                let Ok(data) = self.target.read_file(&from) else {
                    continue;
                };
                self.log(&format!(
                    "{}: {from} -> {to}",
                    if m.moving { "Move" } else { "Copy" }
                ));
                if let Err(e) = self.j_write_file(&to, &data) {
                    self.log(&format!("Writing {to} failed: system error {e}."));
                    self.error(ui, 1310, &[&to], msg::MB_OK);
                    return Err(code::INSTALL_FAILURE);
                }
                if m.moving {
                    self.j_remove_file(&from)?;
                }
            }
        }
        Ok(())
    }
}

/// A registry value's text (a string or expandable string).
fn reg_text(v: &RegValue) -> Option<String> {
    match v {
        RegValue::String(s) | RegValue::Expand(s) => Some(s.clone()),
        _ => None,
    }
}
fn list_contains(list: &str, part: &str, separator: &str) -> bool {
    if separator.is_empty() {
        return list
            .to_ascii_lowercase()
            .contains(&part.to_ascii_lowercase());
    }
    list.split(separator).any(|p| p.eq_ignore_ascii_case(part))
}
/// `list` without the entries equal to `part`.
fn list_without(list: &str, part: &str, separator: &str) -> String {
    if separator.is_empty() {
        return list.replace(part, "");
    }
    let kept: Vec<&str> = list
        .split(separator)
        .filter(|p| !p.eq_ignore_ascii_case(part))
        .collect();
    kept.join(separator)
}
/// A file name against a pattern with `*` and `?` (case-insensitive).
fn wildcard(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.to_ascii_lowercase().chars().collect();
    let n: Vec<char> = name.to_ascii_lowercase().chars().collect();
    let (mut pi, mut ni, mut star, mut mark) = (0, 0, None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

// ---- font titles --------------------------------------------------------

fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
/// A font file's title as Windows registers it: the full name (the name
/// table's ID 4, the Windows English one first) with " (TrueType)" (or "
/// (OpenType)" for CFF outlines); a collection's names joined with " & ".
pub fn font_title(bytes: &[u8]) -> Option<String> {
    let mut faces: Vec<usize> = Vec::new();
    if bytes.get(..4)? == b"ttcf" {
        let count = be32(bytes, 8)? as usize;
        for i in 0..count.min(64) {
            faces.push(be32(bytes, 12 + i * 4)? as usize);
        }
    } else {
        faces.push(0);
    }
    let mut names: Vec<String> = Vec::new();
    let mut cff = false;
    for at in faces {
        // Offsets past the file are not faces (and keep the sums below
        // from overflowing a 32-bit usize).
        if at >= bytes.len() {
            continue;
        }
        let version = be32(bytes, at)?;
        cff |= version == u32::from_be_bytes(*b"OTTO");
        if let Some(name) = face_name(bytes, at)
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    if names.is_empty() {
        return None;
    }
    Some(format!(
        "{} ({})",
        names.join(" & "),
        if cff { "OpenType" } else { "TrueType" }
    ))
}
fn face_name(bytes: &[u8], at: usize) -> Option<String> {
    let tables = be16(bytes, at + 4)? as usize;
    let name = (0..tables).find_map(|i| {
        let record = at + 12 + i * 16;
        (bytes.get(record..record + 4)? == b"name").then(|| be32(bytes, record + 8))?
    })? as usize;
    if name >= bytes.len() {
        return None;
    }
    let count = be16(bytes, name + 2)? as usize;
    let strings = name + be16(bytes, name + 4)? as usize;
    // (platform, encoding, language, id, length, offset), best first.
    let mut best: Option<(u32, String)> = None;
    for i in 0..count.min(1024) {
        let r = name + 6 + i * 12;
        let (platform, encoding, language, id) = (
            be16(bytes, r)?,
            be16(bytes, r + 2)?,
            be16(bytes, r + 4)?,
            be16(bytes, r + 6)?,
        );
        if id != 4 {
            continue;
        }
        let (length, offset) = (be16(bytes, r + 8)? as usize, be16(bytes, r + 10)? as usize);
        let raw = bytes.get(strings + offset..strings + offset + length)?;
        let (rank, text) = match (platform, encoding) {
            // Unicode BMP and full repertoire, and symbol fonts' (3, 0).
            (3, 0 | 1 | 10) => (
                if language == 0x409 { 0 } else { 1 },
                String::from_utf16_lossy(
                    &raw.chunks_exact(2)
                        .map(|c| u16::from_be_bytes([c[0], c[1]]))
                        .collect::<Vec<_>>(),
                ),
            ),
            (1, 0) => (2, raw.iter().map(|&b| b as char).collect()),
            _ => continue,
        };
        if best.as_ref().is_none_or(|(r, _)| rank < *r) {
            best = Some((rank, text));
        }
    }
    best.map(|(_, t)| t).filter(|t| !t.is_empty())
}

// ---- INI files ------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IniEdit {
    /// Add or replace the line.
    Set,
    /// Add the line only when the key is absent.
    SetAbsent,
    /// Remove the line.
    Remove,
    /// Add the value to the line's comma-separated list.
    AddTag,
    /// Remove the value from the list.
    RemoveTag,
}
/// The value of `key` in `section` of an INI text (the profile functions'
/// case-insensitive matching, spaces around `=` trimmed).
pub fn ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut inside = false;
    for line in text.split('\n') {
        let l = line.trim();
        if let Some(name) = l.strip_prefix('[').and_then(|r| r.split_once(']')) {
            inside = name.0.trim().eq_ignore_ascii_case(section);
            continue;
        }
        if inside
            && let Some((k, v)) = l.split_once('=')
            && k.trim().eq_ignore_ascii_case(key)
        {
            return Some(String::from(v.trim()));
        }
    }
    None
}
/// An INI file's text: UTF-16 with its BOM, else the bytes as Latin-1
/// (each byte one character, so an ANSI file round-trips unchanged).
pub fn decode_ini(bytes: &[u8]) -> (String, bool) {
    if bytes.starts_with(&[0xff, 0xfe]) {
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        (String::from_utf16_lossy(&units), true)
    } else {
        (bytes.iter().map(|&b| b as char).collect(), false)
    }
}
fn encode_ini(text: &str, wide: bool) -> Vec<u8> {
    if wide {
        let mut out = alloc::vec![0xff, 0xfe];
        out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        out
    } else {
        text.chars()
            .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
            .collect()
    }
}
/// `text` with one edit of `key` in `section` (case-insensitive, as the
/// profile functions match them); other lines are kept as they are.
pub fn ini_apply(text: &str, section: &str, key: &str, value: &str, edit: IniEdit) -> String {
    let mut lines: Vec<String> = text
        .split('\n')
        .map(|l| String::from(l.strip_suffix('\r').unwrap_or(l)))
        .collect();
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let header = |l: &str| -> Option<String> {
        let t = l.trim();
        t.strip_prefix('[')
            .and_then(|r| r.split_once(']'))
            .map(|(name, _)| String::from(name.trim()))
    };
    let start = lines
        .iter()
        .position(|l| header(l).is_some_and(|h| h.eq_ignore_ascii_case(section)));
    let end = start.map(|s| {
        lines[s + 1..]
            .iter()
            .position(|l| header(l).is_some())
            .map_or(lines.len(), |i| s + 1 + i)
    });
    let line_of = |lines: &[String]| -> Option<usize> {
        let (s, e) = (start?, end?);
        (s + 1..e).find(|&i| {
            lines[i]
                .split_once('=')
                .is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case(key))
        })
    };
    let found = line_of(&lines);
    let current = found.map(|i| String::from(lines[i].split_once('=').unwrap().1.trim()));
    let tags = |v: &str| -> Vec<String> {
        v.split(',')
            .map(|t| String::from(t.trim()))
            .filter(|t| !t.is_empty())
            .collect()
    };
    let new_value: Option<Option<String>> = match edit {
        IniEdit::Set => Some(Some(String::from(value))),
        IniEdit::SetAbsent if found.is_none() => Some(Some(String::from(value))),
        IniEdit::SetAbsent => None,
        IniEdit::Remove => found.map(|_| None),
        IniEdit::AddTag => {
            let mut list = tags(current.as_deref().unwrap_or(""));
            if list.iter().any(|t| t.eq_ignore_ascii_case(value)) {
                None
            } else {
                list.push(String::from(value));
                Some(Some(list.join(",")))
            }
        }
        IniEdit::RemoveTag => {
            let list = tags(current.as_deref().unwrap_or(""));
            let kept: Vec<String> = list
                .iter()
                .filter(|t| !t.eq_ignore_ascii_case(value))
                .cloned()
                .collect();
            if found.is_none() || kept.len() == list.len() {
                None
            } else if kept.is_empty() {
                Some(None)
            } else {
                Some(Some(kept.join(",")))
            }
        }
    };
    let Some(new_value) = new_value else {
        return text_of(&lines, text);
    };
    match (found, new_value) {
        (Some(i), Some(v)) => lines[i] = format!("{key}={v}"),
        (Some(i), None) => {
            lines.remove(i);
        }
        (None, Some(v)) => match end {
            Some(e) => lines.insert(e, format!("{key}={v}")),
            None => {
                lines.push(format!("[{section}]"));
                lines.push(format!("{key}={v}"));
            }
        },
        (None, None) => (),
    }
    text_of(&lines, text)
}
fn text_of(lines: &[String], original: &str) -> String {
    let newline = if original.contains("\r\n") || original.is_empty() {
        "\r\n"
    } else {
        "\n"
    };
    let mut out = String::new();
    for l in lines {
        out.push_str(l);
        out.push_str(newline);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ini_edits_follow_the_profile_functions() {
        let t = ini_apply("", "App", "Path", "C:\\x", IniEdit::Set);
        assert_eq!(t, "[App]\r\nPath=C:\\x\r\n");
        let t = ini_apply(&t, "app", "path", "C:\\y", IniEdit::SetAbsent);
        assert_eq!(t, "[App]\r\nPath=C:\\x\r\n");
        let t = ini_apply(&t, "App", "Path", "C:\\y", IniEdit::Set);
        assert_eq!(t, "[App]\r\nPath=C:\\y\r\n");
        let t = ini_apply(&t, "App", "Plugins", "a", IniEdit::AddTag);
        let t = ini_apply(&t, "App", "Plugins", "b", IniEdit::AddTag);
        let t = ini_apply(&t, "App", "Plugins", "A", IniEdit::AddTag);
        assert_eq!(t, "[App]\r\nPath=C:\\y\r\nPlugins=a,b\r\n");
        let t = ini_apply(&t, "App", "Plugins", "a", IniEdit::RemoveTag);
        assert_eq!(t, "[App]\r\nPath=C:\\y\r\nPlugins=b\r\n");
        let t = ini_apply(&t, "App", "Plugins", "b", IniEdit::RemoveTag);
        let t = ini_apply(&t, "App", "Path", "", IniEdit::Remove);
        assert_eq!(t, "[App]\r\n");
        // Other sections, comments and line endings stay.
        let t = ini_apply("; c\n[A]\nk=1\n[B]\n", "A", "j", "2", IniEdit::Set);
        assert_eq!(t, "; c\n[A]\nk=1\nj=2\n[B]\n");
        let (text, wide) = decode_ini(&[b'a', 0xe9]);
        assert_eq!((text.as_str(), wide), ("a\u{e9}", false));
        assert_eq!(encode_ini(&text, false), [b'a', 0xe9]);
    }
    #[test]
    fn lists_and_wildcards() {
        assert!(list_contains("C:\\a;C:\\b", "c:\\B", ";"));
        assert_eq!(
            list_without("C:\\a;C:\\b;C:\\c", "C:\\B", ";"),
            "C:\\a;C:\\c"
        );
        assert!(wildcard("*.txt", "Read Me.TXT"));
        assert!(wildcard("a?c*", "abcdef"));
        assert!(!wildcard("*.txt", "a.txt.bak"));
    }
    #[test]
    fn font_titles_come_from_the_name_table() {
        // A minimal sfnt: one table, `name`, with a Windows English full
        // name.
        let full: Vec<u8> = "Doors Sans"
            .encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect();
        let mut name = Vec::new();
        name.extend(0u16.to_be_bytes());
        name.extend(1u16.to_be_bytes());
        name.extend(18u16.to_be_bytes());
        for v in [3u16, 1, 0x409, 4, full.len() as u16, 0] {
            name.extend(v.to_be_bytes());
        }
        name.extend(&full);
        let mut font = Vec::new();
        font.extend(0x0001_0000u32.to_be_bytes());
        font.extend(1u16.to_be_bytes());
        font.extend([0u8; 6]);
        font.extend(b"name");
        font.extend(0u32.to_be_bytes());
        font.extend(28u32.to_be_bytes());
        font.extend((name.len() as u32).to_be_bytes());
        font.extend(&name);
        assert_eq!(font_title(&font).as_deref(), Some("Doors Sans (TrueType)"));
        assert_eq!(font_title(b"junk"), None);
    }
}
