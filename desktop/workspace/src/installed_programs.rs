//! Settings > Apps > Installed apps: the desktop programs installers
//! registered under the Uninstall keys (per machine, then per user), as
//! Windows lists them: name, version, publisher, install date and size,
//! with Modify and Uninstall from their ModifyPath and UninstallString
//! (Windows Installer products through msiexec.exe).
use crate::registry_browser::{Backend, Location, load, same_name};
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use door_service_client::RegistryView;

pub const UNINSTALL: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Program {
    /// The key's name (a product code for Windows Installer products).
    pub key: String,
    pub machine: bool,
    pub name: String,
    pub publisher: String,
    pub version: String,
    /// `YYYYMMDD`.
    pub install_date: String,
    /// EstimatedSize, KB.
    pub size_kb: u32,
    pub uninstall: String,
    pub quiet_uninstall: String,
    pub modify: String,
    pub windows_installer: bool,
    pub no_modify: bool,
    pub no_remove: bool,
    pub no_repair: bool,
}
impl Program {
    /// The second line of the app's card: version, publisher and date as
    /// Windows shows them (`2.0.0  |  Macrohard Test  |  11/8/2026`).
    pub fn details(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.version.is_empty() {
            parts.push(self.version.clone());
        }
        if !self.publisher.is_empty() {
            parts.push(self.publisher.clone());
        }
        if let Some(d) = date(&self.install_date) {
            parts.push(d);
        }
        parts.join("  |  ")
    }
    /// The size as Windows shows it (three significant digits).
    pub fn size(&self) -> String {
        size(self.size_kb)
    }
    pub fn can_modify(&self) -> bool {
        !self.no_modify && (!self.modify.is_empty() || self.windows_installer)
    }
    pub fn can_uninstall(&self) -> bool {
        !self.no_remove && (!self.uninstall.is_empty() || self.windows_installer)
    }
    /// The program and its arguments for Uninstall (`modify` false) or
    /// Modify: `MsiExec.exe /X{...}` names msiexec.exe in System32.
    pub fn command(&self, modify: bool) -> Option<(String, Vec<String>)> {
        let line = if modify {
            if !self.modify.is_empty() {
                self.modify.clone()
            } else {
                format!("MsiExec.exe /I{}", self.key)
            }
        } else if !self.uninstall.is_empty() {
            self.uninstall.clone()
        } else {
            format!("MsiExec.exe /X{}", self.key)
        };
        let (program, rest) = split_program(line.trim())?;
        let program = expand(&program);
        let program = if !program.contains('\\') {
            let mut p = format!(r"C:\Windows\System32\{program}");
            if !p.to_ascii_lowercase().ends_with(".exe") {
                p.push_str(".exe");
            }
            p
        } else {
            program
        };
        Some((program, arguments(rest)))
    }
}

/// `%SystemRoot%` and `%ProgramFiles%` in a command line.
fn expand(text: &str) -> String {
    let mut out = String::from(text);
    for (name, value) in [
        ("%SystemRoot%", r"C:\Windows"),
        ("%windir%", r"C:\Windows"),
        ("%ProgramFiles%", r"C:\Program Files"),
        ("%ProgramFiles(x86)%", r"C:\Program Files (x86)"),
    ] {
        loop {
            let lower = out.to_ascii_lowercase();
            let Some(at) = lower.find(&name.to_ascii_lowercase()) else {
                break;
            };
            out.replace_range(at..at + name.len(), value);
        }
    }
    out
}
/// The program of a command line (quoted, or up to the first space) and
/// the rest.
fn split_program(line: &str) -> Option<(String, &str)> {
    if let Some(rest) = line.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some((rest[..end].to_string(), &rest[end + 1..]));
    }
    match line.find(' ') {
        Some(i) => Some((line[..i].to_string(), &line[i..])),
        None => (!line.is_empty()).then(|| (line.to_string(), "")),
    }
}
/// Arguments as CommandLineToArgvW splits them.
fn arguments(rest: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in rest.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            ' ' | '\t' if !quoted => {
                if any || !current.is_empty() {
                    out.push(core::mem::take(&mut current));
                    any = false;
                }
            }
            c => current.push(c),
        }
    }
    if any || !current.is_empty() {
        out.push(current);
    }
    out
}
/// `YYYYMMDD` as `M/D/YYYY`.
pub fn date(text: &str) -> Option<String> {
    if text.len() != 8 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let y = &text[..4];
    let m: u32 = text[4..6].parse().ok()?;
    let d: u32 = text[6..8].parse().ok()?;
    (1..=12).contains(&m).then(|| format!("{m}/{d}/{y}"))
}
/// KB as Windows' size text: `868 KB`, `5.71 MB`, `25.8 MB`, `254 MB`,
/// `1.20 GB`.
pub fn size(kb: u32) -> String {
    if kb == 0 {
        return String::new();
    }
    if kb < 1000 {
        return format!("{kb} KB");
    }
    let (unit_kb, unit) = if kb < 1000 * 1024 {
        (1024u64, "MB")
    } else {
        (1024 * 1024, "GB")
    };
    let kb = u64::from(kb);
    let hundredths = kb * 100 / unit_kb;
    let text = if hundredths < 1000 {
        format!("{}.{:02}", hundredths / 100, hundredths % 100)
    } else if hundredths < 10_000 {
        let tenths = kb * 10 / unit_kb;
        format!("{}.{}", tenths / 10, tenths % 10)
    } else {
        format!("{}", kb / unit_kb)
    };
    format!("{text} {unit}")
}

fn text(data: &[u8]) -> String {
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|u| *u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// Every registered program (per machine in both registry views, as
/// Windows lists 32-bit products from WOW6432Node beside 64-bit ones, then
/// per user), by name.
pub fn list<B: Backend>(backend: &mut B) -> Vec<Program> {
    let mut out: Vec<Program> = Vec::new();
    for (root, machine, view) in [
        ("HKLM", true, RegistryView::Bits64),
        ("HKLM", true, RegistryView::Bits32),
        ("HKCU", false, RegistryView::Bits64),
    ] {
        let Ok(location) = Location::from_ascii(&format!("{root}\\{UNINSTALL}"), view) else {
            continue;
        };
        let Ok(snapshot) = load(backend, &location) else {
            continue;
        };
        for child in &snapshot.children {
            let key = String::from_utf16_lossy(child.units());
            // A key both views show (a shared one) is listed once.
            if out
                .iter()
                .any(|p| p.machine == machine && p.key.eq_ignore_ascii_case(&key))
            {
                continue;
            }
            let Ok(location) = Location::from_ascii(&format!("{root}\\{UNINSTALL}\\{key}"), view)
            else {
                continue;
            };
            let Ok(values) = load(backend, &location) else {
                continue;
            };
            let get = |name: &str| {
                let n: Vec<u16> = name.encode_utf16().collect();
                values
                    .values
                    .iter()
                    .find(|v| same_name(v.name.units(), &n))
                    .map(|v| v.value)
            };
            let string = |name: &str| {
                get(name)
                    .filter(|v| matches!(v.kind, 1 | 2))
                    .map(|v| text(v.data()))
                    .unwrap_or_default()
            };
            let dword = |name: &str| {
                get(name)
                    .filter(|v| v.kind == 4 && v.data().len() >= 4)
                    .map(|v| u32::from_le_bytes(v.data()[..4].try_into().unwrap()))
            };
            let name = string("DisplayName");
            // Updates and system components are not listed.
            if name.is_empty()
                || dword("SystemComponent") == Some(1)
                || !string("ParentKeyName").is_empty()
            {
                continue;
            }
            out.push(Program {
                key,
                machine,
                name,
                publisher: string("Publisher"),
                version: string("DisplayVersion"),
                install_date: string("InstallDate"),
                size_kb: dword("EstimatedSize").unwrap_or(0),
                uninstall: string("UninstallString"),
                quiet_uninstall: string("QuietUninstallString"),
                modify: string("ModifyPath"),
                windows_installer: dword("WindowsInstaller") == Some(1),
                no_modify: dword("NoModify") == Some(1),
                no_remove: dword("NoRemove") == Some(1),
                no_repair: dword("NoRepair") == Some(1),
            });
        }
    }
    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn programs_show_and_run_as_windows_shows_and_runs_them() {
        let p = Program {
            key: "{3D1C9A57-0002-4F60-9C14-7A5B2E0D6F11}".into(),
            name: "Doors Tools".into(),
            publisher: "Macrohard Test".into(),
            version: "2.0.0".into(),
            install_date: "20261108".into(),
            size_kb: 5850,
            uninstall: "MsiExec.exe /X{3D1C9A57-0002-4F60-9C14-7A5B2E0D6F11}".into(),
            modify: "MsiExec.exe /I{3D1C9A57-0002-4F60-9C14-7A5B2E0D6F11}".into(),
            windows_installer: true,
            ..Program::default()
        };
        assert_eq!(p.details(), "2.0.0  |  Macrohard Test  |  11/8/2026");
        assert_eq!(p.size(), "5.71 MB");
        assert_eq!(
            p.command(false),
            Some((
                r"C:\Windows\System32\MsiExec.exe".to_string(),
                alloc::vec!["/X{3D1C9A57-0002-4F60-9C14-7A5B2E0D6F11}".to_string()]
            ))
        );
        assert!(p.can_modify() && p.can_uninstall());
        let n = Program {
            uninstall: r#""C:\Program Files\Doors Setup Sample\uninstall.exe" /S"#.into(),
            no_modify: true,
            ..Program::default()
        };
        assert_eq!(
            n.command(false),
            Some((
                r"C:\Program Files\Doors Setup Sample\uninstall.exe".to_string(),
                alloc::vec!["/S".to_string()]
            ))
        );
        assert!(!n.can_modify());
        assert_eq!(size(868), "868 KB");
        assert_eq!(size(26_419), "25.7 MB");
        assert_eq!(size(260_096), "254 MB");
        assert_eq!(size(1_258_291), "1.19 GB");
        assert_eq!(date("2026111"), None);
        assert_eq!(
            Program {
                uninstall: r#""%SystemRoot%\x.exe" /u"#.into(),
                ..Program::default()
            }
            .command(false)
            .unwrap()
            .0,
            r"C:\Windows\x.exe"
        );
    }
}
