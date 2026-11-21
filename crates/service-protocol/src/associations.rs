//! File associations (Settings > Apps > Default apps; File Explorer's Open
//! and Open with): the file types each built-in app declares, the built-in
//! default for each type, the stable registration keys a per-user choice
//! names, and the naming rule for installed Windows programs. This is the
//! one source of truth for the desktop shell's registration database and the
//! workspace apps (Explorer, Settings).
//!
//! A per-user choice lives in `HKCU\Software\Doors\FileAssociations\<.ext>`
//! (one key per type, named with its dot, lower case), REG_SZ value `App` =
//! a registration key ([`AppKey`]). An absent or empty value means the
//! built-in default ([`builtin_default`]), as a clean Windows install.
//! Allocation-free; the workspace `default_apps` module holds the
//! registry-backed model on top of it.
use crate::shell::{LABEL_BYTES, clip, icon};
use core::fmt;

/// Per-user choices: one subkey per extension under this HKCU path.
pub const KEY_PATH: &str = "Software\\Doors\\FileAssociations";
/// REG_SZ value of an extension key naming the chosen registration.
pub const APP_VALUE: &str = "App";
/// Installed Windows programs register from this folder (`*.exe`).
pub const PROGRAM_FOLDER: &str = "/system/bin";
/// Longest extension (without its dot) a choice can be stored for.
pub const MAX_EXTENSION: usize = 16;

/// A built-in app that can be associated with file types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handler {
    /// Init desktop command (2 Notes, 9 Terminal, 10 Preview).
    pub command: u8,
    pub name: &'static str,
    /// `shell::icon` id.
    pub icon: u16,
    /// Lower-case extensions (without the dot) the app declares.
    pub extensions: &'static [&'static str],
    /// The app receives a document (the window server's OPEN_DOCUMENT
    /// accepts File Explorer, Notes, Terminal and Preview).
    pub documents: bool,
    /// Types this app is the built-in default for ahead of table order.
    pub defaults: &'static [&'static str],
}
/// Why an app that declares a type cannot open a file (shown with the
/// control, which stays disabled).
pub const TERMINAL_UNAVAILABLE: &str =
    "Unavailable: Terminal can't open a file yet. It starts in your home folder.";

/// Built-in apps in preference order: the first app (with `documents`) that
/// declares a type is its built-in default (Terminal runs batch files, and
/// PowerShell scripts in its PowerShell profile; Notes edits them).
pub const HANDLERS: [Handler; 3] = [
    Handler {
        command: 2,
        name: "Notes",
        icon: icon::NOTES,
        extensions: &[
            "txt", "md", "log", "ini", "json", "csv", "xml", "rs", "toml", "ps1", "bat", "cmd",
            "sh", "dsh", "psm1", "psd1", "ps1xml",
        ],
        documents: true,
        defaults: &[],
    },
    // Batch files and Doorsh scripts run in Terminal (cmd.exe, Doorsh), as
    // Windows runs .bat files on Open, and PowerShell scripts in its
    // PowerShell profile (pwsh -File, as PowerShell 7's "Run with
    // PowerShell 7"); Notes edits them (Open with).
    Handler {
        command: 9,
        name: "Terminal",
        icon: icon::TERMINAL,
        extensions: &["bat", "cmd", "dsh", "ps1"],
        documents: true,
        defaults: &["bat", "cmd", "dsh", "ps1"],
    },
    Handler {
        command: 10,
        name: "Preview",
        icon: icon::PREVIEW,
        // Pictures and PDF (its own default), plus the text types Notes opens:
        // Preview shows them read-only and leads "Open with" for all of them.
        extensions: &[
            "png", "jpg", "jpeg", "bmp", "gif", "qoi", "webp", "pdf", "txt", "md", "log", "ini",
            "json", "csv", "xml", "rs", "toml", "ps1", "bat", "cmd", "sh", "psm1", "psd1",
            "ps1xml",
        ],
        documents: true,
        defaults: &[],
    },
];

/// The built-in app with desktop command `command`, if it opens files.
pub fn handler(command: u8) -> Option<&'static Handler> {
    HANDLERS.iter().find(|h| h.command == command)
}
/// Extensions the built-in app `command` declares (empty when none).
pub fn declared(command: u8) -> &'static [&'static str] {
    handler(command).map_or(&[], |h| h.extensions)
}
/// The built-in default for `ext` (without the dot, any case): the first
/// handler that declares it and can be handed a document.
pub fn builtin_default(ext: &str) -> Option<&'static Handler> {
    let ext = ext.strip_prefix('.').unwrap_or(ext);
    if let Some(h) = HANDLERS
        .iter()
        .find(|h| h.defaults.iter().any(|e| e.eq_ignore_ascii_case(ext)))
    {
        return Some(h);
    }
    HANDLERS
        .iter()
        .filter(|h| h.documents)
        .find(|h| h.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)))
}
/// Every extension some built-in app declares, each once, in table order.
pub fn known_extensions() -> impl Iterator<Item = &'static str> {
    HANDLERS
        .iter()
        .enumerate()
        .flat_map(|(i, h)| h.extensions.iter().map(move |e| (i, *e)))
        .filter(|(i, e)| !HANDLERS[..*i].iter().any(|h| h.extensions.contains(e)))
        .map(|(_, e)| e)
}

/// A stable registration key: what a per-user choice and the shell's pin
/// file name. Catalog apps and accessories by Init desktop command
/// (`native:2`), installed Windows programs by their admitted Door path
/// (`exe:/system/bin/editor.exe`), never by a hash or display name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppKey<'a> {
    Native(u8),
    Executable(&'a str),
}
impl<'a> AppKey<'a> {
    /// Parse the canonical text form (`native:N` without leading zeros, or
    /// `exe:` + an absolute Door path to a `.exe`); anything else is `None`.
    pub fn parse(text: &'a str) -> Option<Self> {
        if let Some(n) = text.strip_prefix("native:") {
            let canonical = !n.is_empty()
                && n.bytes().all(|b| b.is_ascii_digit())
                && (n == "0" || !n.starts_with('0'));
            return n.parse().ok().filter(|_| canonical).map(Self::Native);
        }
        let path = text.strip_prefix("exe:")?;
        let valid = path.starts_with('/')
            && is_program(path)
            && !path.contains('\\')
            && !path.contains("//")
            && !path.chars().any(char::is_control);
        valid.then_some(Self::Executable(path))
    }
}
impl fmt::Display for AppKey<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Native(command) => write!(f, "native:{command}"),
            Self::Executable(path) => write!(f, "exe:{path}"),
        }
    }
}

/// A Windows executable (`*.exe`, any case) by file name or path.
pub fn is_program(name: &str) -> bool {
    name.len() > 4
        && name.is_char_boundary(name.len() - 4)
        && name[name.len() - 4..].eq_ignore_ascii_case(".exe")
        && !name.ends_with("/.exe")
}
/// Display name of a program without version resources: its file name
/// without the extension, as Windows shows an unnamed program.
pub fn program_name(path: &str) -> &str {
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = match file.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("exe") => stem,
        _ => file,
    };
    clip(stem, LABEL_BYTES)
}
/// A type name a choice can be stored for: 1..=`MAX_EXTENSION` ASCII
/// letters, digits, `-`, `_` or `+` (no dot, space or path separator).
pub fn valid_extension(ext: &str) -> bool {
    (1..=MAX_EXTENSION).contains(&ext.len())
        && ext
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'+'))
}
/// The extension (without the dot, as written) of a file name: the text
/// after the last dot when the name has a stem before it and the text is a
/// valid type name. `archive.tar.gz` has `gz`; `.profile` and `README` none.
pub fn extension_of(name: &str) -> Option<&str> {
    let file = name.rsplit('/').next().unwrap_or(name);
    let (stem, ext) = file.rsplit_once('.')?;
    (!stem.is_empty() && valid_extension(ext)).then_some(ext)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtin_defaults_follow_the_registration_table_and_windows() {
        assert_eq!(builtin_default("TXT").unwrap().name, "Notes");
        assert_eq!(builtin_default(".png").unwrap().command, 10);
        // Batch files, Doorsh and PowerShell scripts run in Terminal (Windows
        // runs .bat on Open); .sh and PowerShell modules open in Notes.
        for ext in ["bat", "cmd", "dsh", "ps1"] {
            assert_eq!(builtin_default(ext).unwrap().command, 9, "{ext}");
            assert!(declared(2).contains(&ext), "Notes edits {ext}");
        }
        for ext in ["sh", "psm1", "psd1", "ps1xml"] {
            assert_eq!(builtin_default(ext).unwrap().command, 2, "{ext}");
        }
        assert!(handler(9).unwrap().documents);
        assert!(builtin_default("exe").is_none());
        assert!(builtin_default("xyz").is_none());
        assert!(declared(1).is_empty() && declared(3).is_empty());
        let known: alloc_free::List = known_extensions().collect();
        assert_eq!(known.0[..3], ["txt", "md", "log"]);
        assert_eq!(known.0.iter().filter(|e| **e == "ps1").count(), 1);
        assert!(known.0.contains(&"pdf"));
        assert_eq!(known.1, 3 + 14 + 8);
    }
    /// A bounded collector (the crate has no allocator).
    mod alloc_free {
        pub struct List(pub [&'static str; 32], pub usize);
        impl FromIterator<&'static str> for List {
            fn from_iter<I: IntoIterator<Item = &'static str>>(iter: I) -> Self {
                let mut list = List([""; 32], 0);
                for e in iter {
                    list.0[list.1] = e;
                    list.1 += 1;
                }
                list
            }
        }
    }
    #[test]
    fn registration_keys_round_trip_in_canonical_form_only() {
        let mut out = [0u8; 64];
        let mut text = Writer(&mut out, 0);
        use core::fmt::Write;
        write!(text, "{}", AppKey::Native(10)).unwrap();
        assert_eq!(text.as_str(), "native:10");
        assert_eq!(AppKey::parse("native:10"), Some(AppKey::Native(10)));
        assert_eq!(
            AppKey::parse("exe:/system/bin/editor.EXE"),
            Some(AppKey::Executable("/system/bin/editor.EXE"))
        );
        for bad in [
            "native:",
            "native:010",
            "native:+1",
            "native:256",
            "exe:editor.exe",
            "exe:/system/bin/readme.txt",
            "exe:/a\\b.exe",
            "exe:/a//b.exe",
            "exe:/bin/.exe",
            "Notes",
            "",
        ] {
            assert_eq!(AppKey::parse(bad), None, "{bad}");
        }
        assert_eq!(AppKey::parse("native:0"), Some(AppKey::Native(0)));
    }
    struct Writer<'a>(&'a mut [u8], usize);
    impl Writer<'_> {
        fn as_str(&self) -> &str {
            core::str::from_utf8(&self.0[..self.1]).unwrap()
        }
    }
    impl core::fmt::Write for Writer<'_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            self.0[self.1..self.1 + s.len()].copy_from_slice(s.as_bytes());
            self.1 += s.len();
            Ok(())
        }
    }
    #[test]
    fn extensions_and_program_names_follow_explorer() {
        assert_eq!(extension_of("notes.TXT"), Some("TXT"));
        assert_eq!(extension_of("/users/1/a.tar.gz"), Some("gz"));
        assert_eq!(extension_of(".profile"), None);
        assert_eq!(extension_of("README"), None);
        assert_eq!(extension_of("a.b c"), None);
        assert_eq!(extension_of("a."), None);
        assert!(valid_extension("c++") && !valid_extension("a.b") && !valid_extension(""));
        assert!(!valid_extension("abcdefghijklmnopq"));
        assert!(is_program("SETUP.EXE") && !is_program(".exe") && !is_program("a.exe.zip"));
        assert_eq!(
            program_name("/system/bin/wingui-editor.exe"),
            "wingui-editor"
        );
        assert_eq!(program_name("/x/tool"), "tool");
    }
}
