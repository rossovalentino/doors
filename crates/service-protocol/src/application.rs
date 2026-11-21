//! File-based application launch requests to the authenticated init broker.
use crate::{BODY_BYTES, Code};
pub const EXECUTE: u32 = 0x1110;
pub const POWER: u32 = 0x1111;
/// End the programs the caller started (no body); from the session's
/// Terminal, a console byte: every program it (or a program it started)
/// runs in that tab's console.
pub const CANCEL_CHILDREN: u32 = 0x1112;
/// EXECUTE for a launch without a console (Explorer): Init adds
/// `DETACHED_ENVIRONMENT` to the child's environment, so a Windows GUI image
/// runs with NULL standard handles instead of joining the session console.
pub const EXECUTE_DETACHED: u32 = 0x1117;
pub const DETACHED_ENVIRONMENT: &str = "DOOR_CONSOLE=none";
/// Init started the image host for this program (an executable too large
/// to capture whole): ntdll's loader lays the program out from its file
/// (the process's image path) and runs it as the process's executable.
pub const IMAGE_ENVIRONMENT: &str = "DOOR_IMAGE=1";
/// The image host Init starts for such a program, by machine (boot files).
pub const IMAGE_HOST: &str = "seed/imagehost.exe";
pub const IMAGE_HOST32: &str = "seed/imagehost32.exe";
/// The authenticated current window server only, after User Account
/// Control approved Run as administrator: EXECUTE_DETACHED as the session
/// user, with `[image SHA-256, 32 bytes]` before the `Execute` body. Init
/// starts the image only if the bytes it loads hash to exactly that value,
/// so the program that runs is the one whose signature the prompt showed.
pub const EXECUTE_ELEVATED: u32 = 0x1118;
/// Authenticated current display service only. Commands 1..3 launch catalog apps;
/// 9 launches Terminal; 4 shuts down and 5 reboots. No arbitrary path, UID or mode is accepted.
pub const DESKTOP: u32 = 0x1113;
/// Versioned extensible catalog. Legacy DESKTOP query0 retains its104B layout.
pub const DESKTOP_STATE_V2: u32 = 0x1114;
/// Authenticated current display service only: persist the session user's
/// `desktop::ShellLayout` (8 bytes: flags, order). Init stores it with the
/// user's settings and replays it to the display service at the next login.
pub const SHELL_LAYOUT: u32 = 0x1115;
/// Authenticated current display service only: persist and apply the session
/// user's `desktop::Controls` (4 bytes), exactly as Settings does: Init writes
/// storage, then sends SET_DESKTOP_CONTROLS so every client gets
/// CONTROLS_CHANGED.
pub const DESKTOP_CONTROLS_PERSIST: u32 = 0x1116;
pub const DESKTOP_APPS: usize = 6;
/// DESKTOP_STATE_V2 slot (after the app slots) of the session's windowless
/// search host (`door_abi::platform::mode::SEARCH_HOST`), when running.
pub const SEARCH_HOST_SLOT: usize = DESKTOP_APPS;
pub const DESKTOP_COMMANDS: [u8; DESKTOP_APPS] = [1, 2, 3, 9, 10, 11];
/// The elevated Terminal (Terminal with BUILTIN\Administrators enabled, in
/// a window of its own as Windows Terminal opens one): the window server
/// asks for it with this DESKTOP command only after User Account Control
/// approved an administrator console. DESKTOP_STATE_V2 reports it in
/// `ELEVATED_TERMINAL_SLOT`.
pub const ELEVATED_TERMINAL_COMMAND: u8 = 23;
/// The window server's sign-in of the second local account (uid 1002;
/// command 8 signs in the first).
pub const SIGN_IN_SECOND_COMMAND: u8 = 24;
/// Sign out (Start > account > Sign out): sent like power, once the
/// desktop has frozen and every app has closed; Init logs the session off
/// and restarts the window server at the sign-in screen.
pub const SIGN_OUT_COMMAND: u8 = 25;
pub const ELEVATED_TERMINAL_SLOT: usize = SEARCH_HOST_SLOT + 1;
pub fn desktop_index(command: u8) -> Option<usize> {
    DESKTOP_COMMANDS.iter().position(|c| *c == command)
}
/// Accessory tools listed in Start search only (no pin, no taskbar slot):
/// 12 the Door UI component gallery (`door-gallery`, manifest GALLERY) and
/// 13 the Captions assistive client (`door-assist`, manifest ASSIST) and the
/// inbox apps (`door-inbox`, manifests `door_abi::platform::manifest::
/// INBOX` in order): 14 Calculator, 15 Clock, 16 Paint, 17 Snipping Tool,
/// 18 Weather, 19 Magnifier, 20 Narrator, 21 On-Screen Keyboard, 22 Voice
/// access. Init launches each at most once per session.
pub const ACCESSORIES: usize = 11;
pub const ACCESSORY_COMMANDS: [u8; ACCESSORIES] = [12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22];
pub fn accessory_index(command: u8) -> Option<usize> {
    ACCESSORY_COMMANDS.iter().position(|c| *c == command)
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DesktopProcess {
    pub pid: u64,
    pub instance: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DesktopState {
    pub count: usize,
    pub power: u32,
    pub frozen: bool,
    pub processes: [Option<DesktopProcess>; 8],
}
impl DesktopState {
    pub const MAX_BYTES: usize = 16 + 8 * 24;
    pub fn encode(&self, out: &mut [u8]) -> Result<usize, Code> {
        if self.count == 0
            || self.count > 8
            || self.power > 2
            || out.len() < 16 + self.count * 24
            || self.processes[self.count..].iter().any(Option::is_some)
        {
            return Err(Code::InvalidRequest);
        }
        let len = 16 + self.count * 24;
        out[..len].fill(0);
        out[0] = 2;
        out[2] = self.count as u8;
        out[4..8].copy_from_slice(&self.power.to_le_bytes());
        out[8] = self.frozen as u8;
        for (i, p) in self.processes[..self.count].iter().enumerate() {
            if let Some(p) = p {
                if p.pid == 0
                    || p.instance == 0
                    || self.processes[..i].iter().flatten().any(|v| v.pid == p.pid)
                {
                    return Err(Code::InvalidRequest);
                }
                let r = &mut out[16 + i * 24..16 + (i + 1) * 24];
                r[..4].copy_from_slice(&(i as u32 + 1).to_le_bytes());
                r[8..16].copy_from_slice(&p.pid.to_le_bytes());
                r[16..24].copy_from_slice(&p.instance.to_le_bytes());
            }
        }
        Ok(len)
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() < 16 || b[0..2] != [2, 0] || b[3] != 0 || b[8] > 1 || b[9..16] != [0; 7] {
            return Err(Code::InvalidRequest);
        }
        let count = b[2] as usize;
        if count == 0 || count > 8 || b.len() != 16 + count * 24 {
            return Err(Code::InvalidRequest);
        }
        let mut s = Self {
            count,
            power: u32::from_le_bytes(b[4..8].try_into().unwrap()),
            frozen: b[8] != 0,
            processes: [None; 8],
        };
        if s.power > 2 {
            return Err(Code::InvalidRequest);
        }
        for (i, r) in b[16..].chunks_exact(24).enumerate() {
            if r == [0; 24] {
                continue;
            }
            if r[..4] != (i as u32 + 1).to_le_bytes() || r[4..8] != [0; 4] {
                return Err(Code::InvalidRequest);
            }
            let p = DesktopProcess {
                pid: u64::from_le_bytes(r[8..16].try_into().unwrap()),
                instance: u64::from_le_bytes(r[16..24].try_into().unwrap()),
            };
            if p.pid == 0
                || p.instance == 0
                || s.processes[..i].iter().flatten().any(|v| v.pid == p.pid)
            {
                return Err(Code::InvalidRequest);
            }
            s.processes[i] = Some(p);
        }
        Ok(s)
    }
}
/// Queue and admission state behind Init's authenticated desktop boundary.
#[derive(Default)]
pub struct DesktopAdmission {
    command: Option<u8>,
    frozen: bool,
}
impl DesktopAdmission {
    pub fn frozen(&self) -> bool {
        self.frozen
    }
    pub fn request(&mut self, command: u8) -> Result<(), Code> {
        if command == 6 || command == 7 {
            self.frozen = command == 6;
            if self.frozen
                && self.command.is_some_and(|v| {
                    desktop_index(v).is_some()
                        || accessory_index(v).is_some()
                        || v == ELEVATED_TERMINAL_COMMAND
                })
            {
                self.command = None;
            }
            return Ok(());
        }
        let app = desktop_index(command).is_some()
            || accessory_index(command).is_some()
            || command == ELEVATED_TERMINAL_COMMAND;
        if !matches!(command, 4 | 5 | SIGN_OUT_COMMAND) && !app {
            return Err(Code::InvalidRequest);
        }
        if app && self.frozen || !app && !self.frozen {
            return Err(Code::Denied);
        }
        if self.command.is_some() {
            return Err(Code::WouldBlock);
        }
        self.command = Some(command);
        Ok(())
    }
    pub fn take(&mut self) -> Option<u8> {
        self.command.take()
    }
    /// Service loss aborts incomplete shutdown, preserving reconnecting apps.
    pub fn restart(&mut self) {
        self.command = None;
        self.frozen = false;
    }
}

/// A launch path: native (`/...`) or a Windows path on C: (`C:...`),
/// which storage resolves the Windows way.
pub fn rooted(path: &str) -> bool {
    let b = path.as_bytes();
    b.first() == Some(&b'/')
        || (b.len() >= 3
            && b[0].eq_ignore_ascii_case(&b'c')
            && b[1] == b':'
            && matches!(b[2], b'\\' | b'/'))
}
/// Stage arguments longer than an EXECUTE body (`ARGUMENTS_STAGE`), for the
/// caller's next EXECUTE(_DETACHED) whose argument length has
/// `STAGED_ARGUMENTS` set: `[offset u16][whole length u16][bytes]`.
pub const ARGUMENTS_STAGE: u32 = 0x1119;
/// A child started with staged arguments asks for them (it finds
/// `LONG_ARGUMENTS_ENVIRONMENT` in its environment): `[offset u16]`, reply
/// `[whole length u16][0; 2][bytes]`. Each child gets only its own.
pub const ARGUMENTS_FETCH: u32 = 0x111b;
/// From the session's desktop shell at sign-in: its working-folder
/// authority (one transfer, no body). Init launches scheduled tasks and
/// services with it.
pub const SESSION_DIRECTORY: u32 = 0x111c;
/// The argument-length flag of an EXECUTE whose arguments were staged.
pub const STAGED_ARGUMENTS: u16 = 0x8000;
/// Arguments a launch may stage (UTF-8, NUL-separated, NUL-terminated).
/// A staged block may also carry the child's environment (`ExecuteOptions`):
/// a Windows environment block is several KiB.
pub const LONG_ARGUMENT_BYTES: usize = 4096;
/// The command line part of a staged block.
pub const COMMAND_LINE_BYTES: usize = 2048;
pub const LONG_ARGUMENTS_ENVIRONMENT: &str = "DOOR_LONG_ARGUMENTS=1";
/// `DOOR_STAGED_ENVIRONMENT=<n>`: the staged block's first n bytes are the
/// arguments, the rest the environment the launcher gave (CreateProcess's
/// lpEnvironment), which replaces the session's.
pub const STAGED_ENVIRONMENT_PREFIX: &str = "DOOR_STAGED_ENVIRONMENT=";
/// CreateProcess options a staged EXECUTE may carry.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExecuteOptions {
    /// CREATE_SUSPENDED: the first thread waits for ResumeThread.
    pub suspended: bool,
    /// Nonzero: the staged block holds this many argument bytes, then the
    /// environment (NUL-separated `NAME=value`, UTF-8).
    pub environment: u16,
    /// STARTF_USESTDHANDLES: bit 0 input, 1 output, 2 error: that standard
    /// handle is a pipe section, sent after the working folder in this
    /// order.
    pub standard: u8,
    /// Nonzero: the Terminal tab's console (`console::Command::Open`) the
    /// child uses; 0: the caller's own (Init gives each child its
    /// parent's, as Windows children inherit their parent's console).
    pub console: u8,
}
impl ExecuteOptions {
    pub const NONE: Self = Self {
        suspended: false,
        environment: 0,
        standard: 0,
        console: 0,
    };
}
pub struct Execute<'a> {
    pub path: &'a str,
    pub cwd: &'a str,
    pub arguments: &'a [u8],
    /// The arguments were staged (`ARGUMENTS_STAGE`); `arguments` is empty.
    pub staged: bool,
    /// Only with `staged`.
    pub options: ExecuteOptions,
}
impl<'a> Execute<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, Code> {
        if bytes.len() < 6 {
            return Err(Code::InvalidRequest);
        }
        let path = usize::from(u16::from_le_bytes([bytes[0], bytes[1]]));
        let cwd = usize::from(u16::from_le_bytes([bytes[2], bytes[3]]));
        let raw = u16::from_le_bytes([bytes[4], bytes[5]]);
        if raw == STAGED_ARGUMENTS {
            let names = 6 + path + cwd;
            let options = match bytes.len().checked_sub(names) {
                Some(0) => ExecuteOptions::NONE,
                Some(n @ (3 | 4)) => {
                    let flags = bytes[names];
                    let environment = u16::from_le_bytes([bytes[names + 1], bytes[names + 2]]);
                    let console = if n == 4 { bytes[names + 3] } else { 0 };
                    if flags & !0x3f != 0
                        || (flags & 0x20 != 0) != (n == 4)
                        || (n == 4
                            && (console == 0 || usize::from(console) >= crate::console::CONSOLES))
                        || (flags & 2 != 0) != (environment != 0)
                        || usize::from(environment) > COMMAND_LINE_BYTES
                        || flags == 0
                    {
                        return Err(Code::InvalidRequest);
                    }
                    ExecuteOptions {
                        suspended: flags & 1 != 0,
                        environment,
                        standard: (flags >> 2) & 7,
                        console,
                    }
                }
                _ => return Err(Code::InvalidRequest),
            };
            if path == 0 || cwd == 0 {
                return Err(Code::InvalidRequest);
            }
            let path =
                core::str::from_utf8(&bytes[6..6 + path]).map_err(|_| Code::InvalidRequest)?;
            let cwd = core::str::from_utf8(&bytes[6 + path.len()..names])
                .map_err(|_| Code::InvalidRequest)?;
            if !rooted(path) || !rooted(cwd) || path.contains('\0') || cwd.contains('\0') {
                return Err(Code::InvalidRequest);
            }
            return Ok(Self {
                path,
                cwd,
                arguments: &[],
                staged: true,
                options,
            });
        }
        let args = usize::from(raw);
        if path == 0
            || cwd == 0
            || args == 0
            || args > door_abi::platform::ARGUMENT_BYTES
            || bytes.len() != 6 + path + cwd + args
        {
            return Err(Code::InvalidRequest);
        }
        let path = core::str::from_utf8(&bytes[6..6 + path]).map_err(|_| Code::InvalidRequest)?;
        let cwd = core::str::from_utf8(&bytes[6 + path.len()..6 + path.len() + cwd])
            .map_err(|_| Code::InvalidRequest)?;
        let arguments = &bytes[6 + path.len() + cwd.len()..];
        if !rooted(path)
            || !rooted(cwd)
            || path.contains('\0')
            || cwd.contains('\0')
            || arguments.last() != Some(&0)
            || core::str::from_utf8(arguments).is_err()
        {
            return Err(Code::InvalidRequest);
        }
        Ok(Self {
            path,
            cwd,
            arguments,
            staged: false,
            options: ExecuteOptions::NONE,
        })
    }
    pub fn encode(&self, out: &mut [u8; BODY_BYTES]) -> Result<usize, Code> {
        let extra = if self.options == ExecuteOptions::NONE {
            0
        } else {
            3 + usize::from(self.options.console != 0)
        };
        let len = 6 + self.path.len() + self.cwd.len() + self.arguments.len() + extra;
        if len > out.len()
            || (self.staged && !self.arguments.is_empty())
            || (extra != 0 && !self.staged)
        {
            return Err(Code::InvalidRequest);
        }
        out[..2].copy_from_slice(&(self.path.len() as u16).to_le_bytes());
        out[2..4].copy_from_slice(&(self.cwd.len() as u16).to_le_bytes());
        let args = if self.staged {
            STAGED_ARGUMENTS
        } else {
            self.arguments.len() as u16
        };
        out[4..6].copy_from_slice(&args.to_le_bytes());
        out[6..6 + self.path.len()].copy_from_slice(self.path.as_bytes());
        out[6 + self.path.len()..6 + self.path.len() + self.cwd.len()]
            .copy_from_slice(self.cwd.as_bytes());
        out[6 + self.path.len() + self.cwd.len()..len - extra].copy_from_slice(self.arguments);
        if extra != 0 {
            if self.options.standard > 7 {
                return Err(Code::InvalidRequest);
            }
            let at = len - extra;
            out[at] = u8::from(self.options.suspended)
                | if self.options.environment != 0 { 2 } else { 0 }
                | self.options.standard << 2
                | if self.options.console != 0 { 0x20 } else { 0 };
            out[at + 1..at + 3].copy_from_slice(&self.options.environment.to_le_bytes());
            if self.options.console != 0 {
                out[at + 3] = self.options.console;
            }
        }
        Execute::decode(&out[..len])?;
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extensible_catalog_is_versioned_canonical_and_rejects_identity_aliases() {
        let mut state = DesktopState {
            count: 5,
            power: 0,
            frozen: false,
            processes: [None; 8],
        };
        state.processes[4] = Some(DesktopProcess {
            pid: 9,
            instance: 11,
        });
        let mut b = [0; DesktopState::MAX_BYTES];
        let n = state.encode(&mut b).unwrap();
        assert_eq!(n, 136);
        assert_eq!(DesktopState::decode(&b[..n]), Ok(state));
        for len in 0..n {
            assert!(DesktopState::decode(&b[..len]).is_err());
        }
        for (offset, value) in [
            (0, 1),
            (1, 1),
            (2, 9),
            (3, 1),
            (4, 3),
            (8, 2),
            (12, 1),
            (112, 4),
            (116, 1),
            (120, 0),
            (128, 0),
        ] {
            let mut bad = b;
            bad[offset] = value;
            assert!(DesktopState::decode(&bad[..n]).is_err(), "offset={offset}");
        }
        state.processes[0] = state.processes[4];
        assert!(state.encode(&mut b).is_err());
        let mut gate = DesktopAdmission::default();
        gate.request(10).unwrap();
        assert_eq!(gate.take(), Some(10));
        gate.request(10).unwrap();
        gate.request(6).unwrap();
        assert_eq!(gate.take(), None);
        assert_eq!(gate.request(10), Err(Code::Denied));
        gate.request(7).unwrap();
        assert_eq!(desktop_index(4), None);
        assert_eq!(desktop_index(10), Some(4));
        assert_eq!(desktop_index(11), Some(5));
        // 23: the elevated Terminal (window server only, after consent).
        gate.request(ELEVATED_TERMINAL_COMMAND).unwrap();
        assert_eq!(gate.take(), Some(ELEVATED_TERMINAL_COMMAND));
        assert_eq!(gate.request(24), Err(Code::InvalidRequest));
        // Accessories (gallery 12, Captions 13, Calculator 14) are app commands:
        // admitted while unfrozen, cancelled by a freeze, never app slots.
        assert_eq!((desktop_index(12), desktop_index(13)), (None, None));
        assert_eq!(
            (
                accessory_index(12),
                accessory_index(13),
                accessory_index(14),
                accessory_index(15),
                accessory_index(22),
                accessory_index(11)
            ),
            (Some(0), Some(1), Some(2), Some(3), Some(10), None)
        );
        gate.request(13).unwrap();
        assert_eq!(gate.take(), Some(13));
        gate.request(12).unwrap();
        gate.request(6).unwrap();
        assert_eq!(gate.take(), None);
        assert_eq!(gate.request(12), Err(Code::Denied));
        gate.request(7).unwrap();
    }
    #[test]
    fn terminal_admission_obeys_freeze_and_never_becomes_a_power_command() {
        let mut gate = DesktopAdmission::default();
        gate.request(9).unwrap();
        assert_eq!(gate.take(), Some(9));
        gate.request(9).unwrap();
        gate.request(6).unwrap();
        assert_eq!(gate.take(), None);
        assert_eq!(gate.request(9), Err(Code::Denied));
        gate.request(5).unwrap();
        assert_eq!(gate.take(), Some(5));
        gate.request(7).unwrap();
        gate.request(9).unwrap();
        assert_eq!(gate.request(2), Err(Code::WouldBlock));
        assert_eq!(gate.take(), Some(9));
        // Sign out is admitted like power: only once the desktop froze.
        assert_eq!(gate.request(SIGN_OUT_COMMAND), Err(Code::Denied));
        gate.request(6).unwrap();
        gate.request(SIGN_OUT_COMMAND).unwrap();
        assert_eq!(gate.take(), Some(SIGN_OUT_COMMAND));
    }
    #[test]
    fn freeze_cancels_queued_admission_and_restart_or_cancel_reopens_it() {
        let mut gate = DesktopAdmission::default();
        assert_eq!(gate.request(4), Err(Code::Denied));
        gate.request(2).unwrap();
        assert_eq!(gate.request(3), Err(Code::WouldBlock));
        gate.request(6).unwrap();
        assert_eq!(gate.take(), None);
        assert_eq!(gate.request(2), Err(Code::Denied));
        gate.request(4).unwrap();
        assert_eq!(gate.take(), Some(4));
        gate.request(7).unwrap();
        gate.request(3).unwrap();
        gate.request(6).unwrap();
        gate.restart();
        assert!(!gate.frozen());
        assert_eq!(gate.take(), None);
        gate.request(2).unwrap();
    }
    #[test]
    fn capture_preserves_argument_boundaries_and_rejects_short_messages() {
        let request = Execute {
            path: "/system/bin/tools",
            cwd: "/users/1001",
            arguments: b"tools\0echo\0two words\0\0",
            staged: false,
            options: ExecuteOptions::NONE,
        };
        let mut bytes = [0; BODY_BYTES];
        let len = request.encode(&mut bytes).unwrap();
        let decoded = Execute::decode(&bytes[..len]).unwrap();
        assert_eq!(decoded.arguments, request.arguments);
        assert!(!decoded.staged);
        for n in 0..len {
            assert!(Execute::decode(&bytes[..n]).is_err());
        }
        bytes[len - 1] = 1;
        assert!(Execute::decode(&bytes[..len]).is_err());
        // Staged arguments (fetched from Init) travel without bytes.
        let staged = Execute {
            path: "/system/bin/tools",
            cwd: "/users/1001",
            arguments: &[],
            staged: true,
            options: ExecuteOptions::NONE,
        };
        let len = staged.encode(&mut bytes).unwrap();
        assert!(Execute::decode(&bytes[..len]).unwrap().staged);
        // CREATE_SUSPENDED with an environment block: three option bytes.
        let options = ExecuteOptions {
            suspended: true,
            environment: 12,
            standard: 0b110,
            console: 0,
        };
        let suspended = Execute { options, ..staged };
        let len = suspended.encode(&mut bytes).unwrap();
        assert_eq!(Execute::decode(&bytes[..len]).unwrap().options, options);
        bytes[len - 3] = 0x40;
        assert!(Execute::decode(&bytes[..len]).is_err());
        // A Terminal tab's console: a fourth option byte.
        let tab = ExecuteOptions {
            console: 3,
            ..ExecuteOptions::NONE
        };
        let len = Execute {
            options: tab,
            ..staged
        }
        .encode(&mut bytes)
        .unwrap();
        assert_eq!(Execute::decode(&bytes[..len]).unwrap().options, tab);
        bytes[len - 1] = 0;
        assert!(Execute::decode(&bytes[..len]).is_err());
        bytes[len - 1] = 3;
        bytes[len - 4] = 0;
        assert!(Execute::decode(&bytes[..len]).is_err());
        // Options need staged arguments.
        let plain = Execute {
            path: "/system/bin/tools",
            cwd: "/users/1001",
            arguments: b"tools\0",
            staged: false,
            options,
        };
        assert!(plain.encode(&mut bytes).is_err());
    }
}
