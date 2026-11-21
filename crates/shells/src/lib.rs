//! Terminal's command engines (P16): `cmd` reproduces cmd.exe (batch
//! files, every internal command and the System32 console tools it is used
//! with); `doorsh` is the native Doors shell. Both run over [`host::Host`]
//! (files, clock, system facts) and never block: a command that needs a
//! program, a key or time yields a [`Step`] and the caller resumes the engine
//! with the outcome, so Terminal keeps painting while a script runs and the
//! engines are tested on the host with an in-memory file system.
#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod cmd;
pub mod doorsh;
pub mod format;
pub mod host;
pub mod legacy;
#[cfg(test)]
pub mod memhost;
pub mod wild;
pub mod wpath;

use alloc::{string::String, vec::Vec};

/// The product names users see (owner's branding: Macrohard for Microsoft,
/// Doors for Windows); API identifiers software depends on keep Windows'.
pub const PRODUCT: &str = "Macrohard Doors";
pub const COMPANY: &str = "Macrohard Corporation";
/// The Windows version Doors reports to Windows software (`ver`,
/// `GetVersionEx`-style queries): Windows 11 26H2.
pub const WINDOWS_VERSION: (u32, u32, u32, u32) = (10, 0, 26300, 1);

/// What an engine needs from its caller to continue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing more to run: show the prompt and wait for a line.
    Idle,
    /// Run a program and resume with [`Resume::Exited`].
    Spawn(Spawn),
    /// Open something in a Doors app (legacy command redirection, `start`,
    /// associations) and resume with [`Resume::Done`].
    Launch(Launch),
    /// Wait for input: resume with [`Resume::Key`] or [`Resume::Line`].
    Input(InputKind),
    /// Wait this long (or until a key, when `keys` is true), then resume
    /// with [`Resume::Tick`] (or [`Resume::Key`]).
    Sleep { ms: u64, keys: bool },
    /// The engine asked its window to close (`exit`).
    Exit(i32),
}
/// How the caller finished what a [`Step`] asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resume {
    /// A program exited; `output` is what it printed when it was captured.
    Exited {
        code: i32,
        output: Vec<u8>,
    },
    /// The program could not be started.
    Failed(host::FsError),
    Key(char),
    Line(String),
    Tick,
    Done(bool),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputKind {
    /// One key (PAUSE, CHOICE, Y/N prompts, MORE).
    Key,
    /// A line of text (SET /P, DATE, TIME, COPY overwrite prompts).
    Line,
}
/// A program to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spawn {
    /// Native path of the executable.
    pub program: String,
    /// The Windows command line (program name first) and its words.
    pub command_line: String,
    pub argv: Vec<String>,
    /// Native working directory.
    pub cwd: String,
    /// `NAME=value` pairs.
    pub environment: Vec<String>,
    /// Bytes for the program's standard input (pipes, `<file`).
    pub input: Option<Vec<u8>>,
    /// Return the output in [`Resume::Exited`] instead of showing it.
    pub capture: bool,
    /// `start`: do not wait for the program.
    pub detached: bool,
}
/// Something a Doors app opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Launch {
    /// A Doors app or inbox tool by its launch name (`legacy::App`).
    App {
        app: legacy::App,
        argument: Option<String>,
    },
    /// The Settings page a legacy entry point resolves to
    /// (`door_settings_map::navigate_arg`).
    Settings(u32),
    /// A file or folder with its associated app (native path).
    Open(String),
    /// A web address.
    Url(String),
    /// Shut down, restart or sign out (shutdown.exe).
    Power(Power),
    /// A new Terminal tab with this engine (`start cmd`, `.bat` from
    /// Explorer) at a native directory, optionally running a batch file.
    Terminal {
        profile: Profile,
        cwd: String,
        script: Option<String>,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Power {
    ShutDown,
    Restart,
    SignOut,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Cmd,
    Doorsh,
}

/// Windows command-line splitting (`CommandLineToArgvW` rules): quotes
/// group, `\"` is a literal quote, 2n backslashes before a quote are n.
pub fn split_command_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let mut word = String::new();
        let mut quoted = false;
        while i < chars.len() {
            let c = chars[i];
            if c == '\\' {
                let mut n = 0;
                while i < chars.len() && chars[i] == '\\' {
                    n += 1;
                    i += 1;
                }
                if chars.get(i) == Some(&'"') {
                    word.extend(core::iter::repeat_n('\\', n / 2));
                    if n % 2 == 1 {
                        word.push('"');
                        i += 1;
                    }
                } else {
                    word.extend(core::iter::repeat_n('\\', n));
                }
                continue;
            }
            if c == '"' {
                if quoted && chars.get(i + 1) == Some(&'"') {
                    word.push('"');
                    i += 2;
                    continue;
                }
                quoted = !quoted;
                i += 1;
                continue;
            }
            if !quoted && (c == ' ' || c == '\t') {
                break;
            }
            word.push(c);
            i += 1;
        }
        out.push(word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn command_line_splitting_follows_windows_rules() {
        assert_eq!(split_command_line(r#"a "b c" d"#), ["a", "b c", "d"]);
        assert_eq!(split_command_line(r#"a\\\"b c"#), [r#"a\"b"#, "c"]);
        assert_eq!(
            split_command_line(r#""C:\Program Files\x.exe" /q"#),
            [r"C:\Program Files\x.exe", "/q"]
        );
        assert_eq!(split_command_line(r#"a\\"b c" d"#), [r"a\b c", "d"]);
    }
}
