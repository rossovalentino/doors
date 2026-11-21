//! door.notifications: toast notifications through Init's authenticated
//! endpoint (spec §60/§62). Any session process may POST; Init records the
//! poster's kernel identity as the source (an app cannot speak for another
//! app) and keeps a bounded centre per session. Only the session's desktop
//! shell lists, clears and activates; the shell and the session's workspace
//! apps (Settings) toggle Do not disturb, run Focus and apply the
//! notification preference words; a poster withdraws its own notifications
//! and takes its own activations.
//!
//! With "Notifications" off (`preferences::notify::ON` clear) a POST is
//! accepted and answered with an id, but nothing is kept, toasted or listed,
//! as Windows. Per-app settings live in the registry (`sender`) and are
//! applied by the desktop shell.
//!
//! Records are bounded little-endian; strings are UTF-8 without controls.
use crate::{BODY_BYTES, Code};

/// POST(`Post`) -> id u32.
pub const POST: u32 = 0x1120;
/// WITHDRAW(id u32): the poster removes its own notification.
pub const WITHDRAW: u32 = 0x1121;
/// STATE() -> `State` (shell only).
pub const STATE: u32 = 0x1122;
/// GET(id u32) -> `Record` (shell only).
pub const GET: u32 = 0x1123;
/// CLEAR(id u32; 0 = all) (shell only).
pub const CLEAR: u32 = 0x1124;
/// SET_DND(on u32) (shell, workspace): the manual Do not disturb choice. Init
/// stores it in the NOTIFICATIONS word (`notify::DND`) so it survives
/// sign-out and restart; turning it off also ends a running Focus
/// session's Do not disturb.
pub const SET_DND: u32 = 0x1125;
/// ACTIVATE(id u32, action u32: 0 body, 1.. button) (shell only): queued for
/// the poster, which takes it with TAKE_ACTIVATION.
pub const ACTIVATE: u32 = 0x1126;
/// TAKE_ACTIVATION() -> (id u32, action u32) or empty (poster).
pub const TAKE_ACTIVATION: u32 = 0x1127;
/// MARK_READ() (shell only): the centre was opened.
pub const MARK_READ: u32 = 0x1128;
/// APP_IDENTITY(pid u64) -> `Identity` (shell only): what Init knows about
/// a session process, so the shell groups windows by registration without
/// trusting application self-description.
pub const APP_IDENTITY: u32 = 0x1129;
/// STATUS() -> `Status` (desktop shell and the session's workspace apps,
/// Settings among them): Do not disturb and Focus as Settings shows them.
pub const STATUS: u32 = 0x112A;
/// FOCUS_START(minutes u32; 0 = the user's FOCUS length, otherwise one of
/// `preferences::Focus::LENGTHS`) (shell, workspace): start a Focus session.
pub const FOCUS_START: u32 = 0x112B;
/// FOCUS_STOP() (shell, workspace): end the running session early (no toast).
pub const FOCUS_STOP: u32 = 0x112C;
/// FOCUS_LENGTH(minutes u32) (shell, workspace): the Focus footer's stepper;
/// Init stores the FOCUS preference word for the session user.
pub const FOCUS_LENGTH: u32 = 0x112D;
/// APPLY_PREFERENCE(id u32, value u32) (shell, workspace): a preference word
/// already stored with Storage SET_SETTING (`preferences::NOTIFICATIONS`,
/// `DND_SCHEDULE` or `FOCUS`) takes effect now. Init stores nothing.
pub const APPLY_PREFERENCE: u32 = 0x112E;
/// Last opcode Init routes to door.notifications.
pub const LAST: u32 = APPLY_PREFERENCE;

pub const TITLE_BYTES: usize = 48;
pub const BODY_TEXT_BYTES: usize = 112;
pub const ACTION_BYTES: usize = 12;
pub const SOURCE_BYTES: usize = 16;
pub const MAX_ACTIONS: usize = 2;
/// Notifications retained per session (oldest dropped first).
pub const CAPACITY: usize = 16;
/// Ids per STATE reply.
pub const STATE_IDS: usize = CAPACITY;
pub const PATH_BYTES: usize = 160;

/// Post flags.
pub mod flag {
    /// Shown in the centre only, never as a popup.
    pub const SILENT: u16 = 1;
    /// Kept as a toast until acted on (reminders).
    pub const PERSISTENT: u16 = 2;
    pub const ALL: u16 = 3;
}
/// Source codes Init assigns (never supplied by the poster).
pub mod source {
    /// Init itself (session, shell recovery, launch diagnostics).
    pub const SYSTEM: u32 = 0xFFFF;
    /// `JOB | slot`: a program started through EXECUTE (Windows apps).
    pub const JOB: u32 = 0x1000;
    /// The desktop shell.
    pub const SHELL: u32 = 0x2000;
    /// Another session process (input method, test client, ...).
    pub const SESSION: u32 = 0x3000;
    // Otherwise the Init desktop command of the catalog app (1, 2, 3, 9, 10,
    // 11) or accessory (12, 13).
}
fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
fn text(b: &[u8]) -> Result<&str, Code> {
    let s = core::str::from_utf8(b).map_err(|_| Code::InvalidRequest)?;
    if s.chars().any(char::is_control) {
        return Err(Code::InvalidRequest);
    }
    Ok(s)
}
fn put(out: &mut [u8], at: &mut usize, s: &str) {
    out[*at..*at + s.len()].copy_from_slice(s.as_bytes());
    *at += s.len();
}
fn take<'a>(b: &'a [u8], at: &mut usize, n: usize) -> Result<&'a str, Code> {
    let s = text(&b[*at..*at + n])?;
    *at += n;
    Ok(s)
}

/// POST body: what an app asks to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Post<'a> {
    /// `shell::icon` artwork; the shell substitutes the source's own icon
    /// for `GENERIC_APP`.
    pub icon: u16,
    pub flags: u16,
    pub title: &'a str,
    pub body: &'a str,
    pub actions: [&'a str; MAX_ACTIONS],
}
const POST_FIXED: usize = 8;
impl Post<'_> {
    fn check(&self) -> Result<(), Code> {
        let controls = [self.title, self.body, self.actions[0], self.actions[1]]
            .iter()
            .any(|s| s.chars().any(char::is_control));
        if controls
            || self.title.trim().is_empty()
            || self.title.len() > TITLE_BYTES
            || self.body.len() > BODY_TEXT_BYTES
            || self.flags & !flag::ALL != 0
            || self.icon > crate::shell::icon::LAST
            || self.actions.iter().any(|a| a.len() > ACTION_BYTES)
            || (self.actions[0].is_empty() && !self.actions[1].is_empty())
        {
            return Err(Code::InvalidRequest);
        }
        Ok(())
    }
    pub fn encode(&self, out: &mut [u8]) -> Result<usize, Code> {
        self.check()?;
        let len = POST_FIXED
            + self.title.len()
            + self.body.len()
            + self.actions.iter().map(|a| a.len()).sum::<usize>();
        if out.len() < len || len > BODY_BYTES {
            return Err(Code::InvalidRequest);
        }
        out[..POST_FIXED].fill(0);
        out[..2].copy_from_slice(&self.icon.to_le_bytes());
        out[2..4].copy_from_slice(&self.flags.to_le_bytes());
        out[4] = self.title.len() as u8;
        out[5] = self.body.len() as u8;
        out[6] = self.actions[0].len() as u8;
        out[7] = self.actions[1].len() as u8;
        let mut at = POST_FIXED;
        for s in [self.title, self.body, self.actions[0], self.actions[1]] {
            put(out, &mut at, s);
        }
        Ok(len)
    }
    pub fn decode(b: &[u8]) -> Result<Post<'_>, Code> {
        if b.len() < POST_FIXED {
            return Err(Code::InvalidRequest);
        }
        let lens = [b[4], b[5], b[6], b[7]].map(usize::from);
        if b.len() != POST_FIXED + lens.iter().sum::<usize>() {
            return Err(Code::InvalidRequest);
        }
        let mut at = POST_FIXED;
        let title = take(b, &mut at, lens[0])?;
        let body = take(b, &mut at, lens[1])?;
        let a0 = take(b, &mut at, lens[2])?;
        let a1 = take(b, &mut at, lens[3])?;
        let p = Post {
            icon: u16_at(b, 0),
            flags: u16_at(b, 2),
            title,
            body,
            actions: [a0, a1],
        };
        p.check()?;
        Ok(p)
    }
}

/// STATE reply: change generation, Do not disturb, unread count and the
/// retained ids, newest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub generation: u32,
    pub dnd: bool,
    pub unread: u8,
    pub count: u8,
    pub ids: [u32; STATE_IDS],
}
impl State {
    pub const MAX_BYTES: usize = 8 + 4 * STATE_IDS;
    pub fn encode(&self, out: &mut [u8]) -> Result<usize, Code> {
        let n = self.count as usize;
        if n > STATE_IDS
            || self.unread > self.count
            || self.ids[..n].contains(&0)
            || self.ids[n..].iter().any(|id| *id != 0)
            || out.len() < 8 + 4 * n
        {
            return Err(Code::InvalidRequest);
        }
        out[..8].fill(0);
        out[..4].copy_from_slice(&self.generation.to_le_bytes());
        out[4] = u8::from(self.dnd);
        out[5] = self.unread;
        out[6] = self.count;
        for (i, id) in self.ids[..n].iter().enumerate() {
            out[8 + i * 4..12 + i * 4].copy_from_slice(&id.to_le_bytes());
        }
        Ok(8 + 4 * n)
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() < 8 || b[4] > 1 || b[7] != 0 {
            return Err(Code::InvalidRequest);
        }
        let n = b[6] as usize;
        if n > STATE_IDS || b.len() != 8 + 4 * n || b[5] > b[6] {
            return Err(Code::InvalidRequest);
        }
        let mut s = Self {
            generation: u32_at(b, 0),
            dnd: b[4] == 1,
            unread: b[5],
            count: b[6],
            ids: [0; STATE_IDS],
        };
        for i in 0..n {
            s.ids[i] = u32_at(b, 8 + i * 4);
            if s.ids[i] == 0 || s.ids[..i].contains(&s.ids[i]) {
                return Err(Code::InvalidRequest);
            }
        }
        Ok(s)
    }
}

/// GET reply: one retained notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record<'a> {
    pub id: u32,
    /// `source` code Init derived from the poster's kernel identity.
    pub source: u32,
    pub icon: u16,
    pub flags: u16,
    /// Monotonic tick of the post.
    pub posted: u64,
    pub read: bool,
    /// Init's display name for the source (an executable's file name).
    pub name: &'a str,
    /// `sender::path_hash` of the executable Init admitted (EXECUTE jobs),
    /// else 0: the stable part of a Windows app's sender key.
    pub sender: u32,
    pub title: &'a str,
    pub body: &'a str,
    pub actions: [&'a str; MAX_ACTIONS],
}
const RECORD_FIXED: usize = 32;
impl Record<'_> {
    pub const MAX_BYTES: usize =
        RECORD_FIXED + SOURCE_BYTES + TITLE_BYTES + BODY_TEXT_BYTES + MAX_ACTIONS * ACTION_BYTES;
    pub fn encode(&self, out: &mut [u8]) -> Result<usize, Code> {
        Post {
            icon: self.icon,
            flags: self.flags,
            title: self.title,
            body: self.body,
            actions: self.actions,
        }
        .check()?;
        if self.id == 0 || self.name.len() > SOURCE_BYTES {
            return Err(Code::InvalidRequest);
        }
        let parts = [
            self.name,
            self.title,
            self.body,
            self.actions[0],
            self.actions[1],
        ];
        let len = RECORD_FIXED + parts.iter().map(|p| p.len()).sum::<usize>();
        if out.len() < len || len > BODY_BYTES {
            return Err(Code::InvalidRequest);
        }
        out[..RECORD_FIXED].fill(0);
        out[..4].copy_from_slice(&self.id.to_le_bytes());
        out[4..8].copy_from_slice(&self.source.to_le_bytes());
        out[8..10].copy_from_slice(&self.icon.to_le_bytes());
        out[10..12].copy_from_slice(&self.flags.to_le_bytes());
        out[12..20].copy_from_slice(&self.posted.to_le_bytes());
        out[20] = u8::from(self.read);
        out[28..32].copy_from_slice(&self.sender.to_le_bytes());
        for (i, p) in parts.iter().enumerate() {
            out[21 + i] = p.len() as u8;
        }
        let mut at = RECORD_FIXED;
        for p in parts {
            put(out, &mut at, p);
        }
        Ok(len)
    }
    pub fn decode(b: &[u8]) -> Result<Record<'_>, Code> {
        if b.len() < RECORD_FIXED || b[20] > 1 || b[26..28] != [0; 2] {
            return Err(Code::InvalidRequest);
        }
        let lens: [usize; 5] = core::array::from_fn(|i| b[21 + i] as usize);
        if b.len() != RECORD_FIXED + lens.iter().sum::<usize>() {
            return Err(Code::InvalidRequest);
        }
        let mut at = RECORD_FIXED;
        let mut parts = [""; 5];
        for (i, n) in lens.iter().enumerate() {
            parts[i] = take(b, &mut at, *n)?;
        }
        let r = Record {
            id: u32_at(b, 0),
            source: u32_at(b, 4),
            icon: u16_at(b, 8),
            flags: u16_at(b, 10),
            posted: u64_at(b, 12),
            read: b[20] == 1,
            name: parts[0],
            sender: u32_at(b, 28),
            title: parts[1],
            body: parts[2],
            actions: [parts[3], parts[4]],
        };
        let mut check = [0; BODY_BYTES];
        r.encode(&mut check)?;
        Ok(r)
    }
}

/// APP_IDENTITY reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity<'a> {
    /// `source` code of the process (catalog command, `JOB | slot`, ...).
    pub source: u32,
    /// Executable path Init admitted (EXECUTE jobs), else empty.
    pub path: &'a str,
}
impl Identity<'_> {
    pub fn encode(&self, out: &mut [u8]) -> Result<usize, Code> {
        if self.path.len() > PATH_BYTES || out.len() < 8 + self.path.len() {
            return Err(Code::InvalidRequest);
        }
        out[..8].fill(0);
        out[..4].copy_from_slice(&self.source.to_le_bytes());
        out[4] = self.path.len() as u8;
        out[8..8 + self.path.len()].copy_from_slice(self.path.as_bytes());
        Ok(8 + self.path.len())
    }
    pub fn decode(b: &[u8]) -> Result<Identity<'_>, Code> {
        if b.len() < 8 || b[5..8] != [0; 3] || b.len() != 8 + b[4] as usize {
            return Err(Code::InvalidRequest);
        }
        Ok(Identity {
            source: u32_at(b, 0),
            path: text(&b[8..])?,
        })
    }
}

/// STATUS reply flags.
pub mod status {
    /// Do not disturb is in effect (manual, scheduled or Focus).
    pub const DND: u32 = 1;
    /// The schedule ("During these times") turned it on.
    pub const SCHEDULED: u32 = 2;
    /// A Focus session is running.
    pub const FOCUS: u32 = 4;
    /// The running session holds Do not disturb on.
    pub const FOCUS_DND: u32 = 8;
    /// "Notifications" (get notifications from apps and other senders).
    pub const ENABLED: u32 = 16;
    /// The FOCUS options "Hide badges" / "Hide flashing on taskbar apps".
    pub const HIDE_BADGES: u32 = 32;
    pub const HIDE_FLASHING: u32 = 64;
    pub const ALL: u32 = 127;
}
/// STATUS reply (16 bytes): what Settings > System > Notifications and the
/// Focus footer show. `generation` is the centre's change generation (the
/// same as `State::generation`), so a poller re-reads only on change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub generation: u32,
    /// `status` flags.
    pub flags: u32,
    /// The user's FOCUS session length (minutes).
    pub focus_length: u16,
    /// Length of the running session (minutes; 0 idle).
    pub focus_minutes: u16,
    /// Seconds left in the running session (0 idle).
    pub focus_remaining: u32,
}
impl Status {
    pub const BYTES: usize = 16;
    pub fn has(&self, flag: u32) -> bool {
        self.flags & flag != 0
    }
    pub fn encode(&self) -> [u8; Self::BYTES] {
        let mut o = [0; Self::BYTES];
        o[..4].copy_from_slice(&self.generation.to_le_bytes());
        o[4..8].copy_from_slice(&self.flags.to_le_bytes());
        o[8..10].copy_from_slice(&self.focus_length.to_le_bytes());
        o[10..12].copy_from_slice(&self.focus_minutes.to_le_bytes());
        o[12..16].copy_from_slice(&self.focus_remaining.to_le_bytes());
        o
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() != Self::BYTES {
            return Err(Code::InvalidRequest);
        }
        let s = Self {
            generation: u32_at(b, 0),
            flags: u32_at(b, 4),
            focus_length: u16_at(b, 8),
            focus_minutes: u16_at(b, 10),
            focus_remaining: u32_at(b, 12),
        };
        let lengths = crate::preferences::Focus::LENGTHS;
        let running = s.has(status::FOCUS);
        if s.flags & !status::ALL != 0
            || !lengths.contains(&u32::from(s.focus_length))
            || running != (s.focus_minutes != 0)
            || (running && !lengths.contains(&u32::from(s.focus_minutes)))
            || s.focus_remaining > u32::from(s.focus_minutes) * 60
            || (s.has(status::FOCUS_DND) && !running)
            || (s.has(status::SCHEDULED) && !s.has(status::DND))
        {
            return Err(Code::InvalidRequest);
        }
        Ok(s)
    }
}

/// Per-app notification settings (Settings > System > Notifications >
/// "Notifications from apps and other senders"). Each sender that ever
/// posted has a key `HKCU\<PATH>\<sender key>` holding REG_SZ [`NAME`] (its
/// display name, at most [`NAME_UNITS`] UTF-16 units plus the terminator,
/// 64 bytes) and REG_DWORD [`FLAGS`]. The desktop shell creates the key the
/// first time the sender posts (flags [`DEFAULT`]), re-reads the flags when
/// the registry changes and applies them: [`ON`] clear hides the sender
/// entirely; [`BANNERS`] clear lists without a toast; [`CENTER`] clear
/// toasts without listing; the priority orders the centre (top, high,
/// normal; newest first within a level) and a top or high toast is kept
/// over normal ones when toasts overflow. [`SOUND`] is kept for when Doors
/// has an audio device (nothing plays today).
///
/// Sender keys (stable, at most [`KEY_BYTES`] bytes):
/// * `System`: Init's own notices (launch failures, desktop restarts, Focus);
/// * `Desktop`: the desktop shell's own notices (programs it could not open);
/// * `App.<command>`: a Doors app by its Init desktop command (`App.1` File
///   Explorer, `App.2` Notepad, `App.3` Settings, ...);
/// * `Exe.<file name>.<hash>`: a Windows program, by its executable's file
///   name (registry-unsafe characters replaced by `_`, at most 34 bytes) and
///   [`path_hash`] of its full admitted path as 8 lowercase hex digits;
/// * `Session`: any other session process.
///
/// [`NAME`]: sender::NAME
/// [`NAME_UNITS`]: sender::NAME_UNITS
/// [`FLAGS`]: sender::FLAGS
/// [`DEFAULT`]: sender::DEFAULT
/// [`ON`]: sender::ON
/// [`BANNERS`]: sender::BANNERS
/// [`CENTER`]: sender::CENTER
/// [`SOUND`]: sender::SOUND
/// [`KEY_BYTES`]: sender::KEY_BYTES
/// [`path_hash`]: sender::path_hash
pub mod sender {
    pub const PATH: &str = "Software\\Doors\\Notifications\\Senders";
    pub const NAME: &str = "Name";
    pub const FLAGS: &str = "Flags";
    /// UTF-16 units of `Name` before its terminator (64 bytes in all).
    pub const NAME_UNITS: usize = 31;
    /// "Notifications" for this sender.
    pub const ON: u32 = 1;
    /// "Show notification banners".
    pub const BANNERS: u32 = 2;
    /// "Play a sound when a notification arrives".
    pub const SOUND: u32 = 4;
    /// "Show notifications in notification center".
    pub const CENTER: u32 = 8;
    /// Bits 4..6: "Priority of notifications in notification center".
    pub const PRIORITY_SHIFT: u32 = 4;
    pub const PRIORITY_MASK: u32 = 3 << PRIORITY_SHIFT;
    pub const NORMAL: u32 = 0;
    pub const HIGH: u32 = 1;
    pub const TOP: u32 = 2;
    pub const ALL: u32 = 0x3f;
    pub const DEFAULT: u32 = ON | BANNERS | SOUND | CENTER;
    pub const SYSTEM: &str = "System";
    pub const DESKTOP: &str = "Desktop";
    pub const SESSION: &str = "Session";
    /// Longest sender key.
    pub const KEY_BYTES: usize = 47;
    const FILE_BYTES: usize = 34;
    pub const fn priority(flags: u32) -> u32 {
        (flags & PRIORITY_MASK) >> PRIORITY_SHIFT
    }
    pub const fn valid(flags: u32) -> bool {
        flags & !ALL == 0 && priority(flags) <= TOP
    }
    /// FNV-1a over the ASCII-lowercased bytes of an admitted executable path.
    pub fn path_hash(path: &str) -> u32 {
        let mut h: u32 = 0x811c_9dc5;
        for b in path.bytes() {
            h ^= u32::from(b.to_ascii_lowercase());
            h = h.wrapping_mul(0x0100_0193);
        }
        h
    }
    /// The registry key name of `source` (a `notifications::source` code);
    /// `name` (the executable's file name) and `hash` matter for jobs only.
    pub fn key<'a>(source: u32, name: &str, hash: u32, out: &'a mut [u8; KEY_BYTES]) -> &'a str {
        let mut at = 0;
        let mut put = |s: &[u8], out: &mut [u8; KEY_BYTES]| {
            out[at..at + s.len()].copy_from_slice(s);
            at += s.len();
        };
        match source {
            super::source::SYSTEM => put(SYSTEM.as_bytes(), out),
            super::source::SHELL => put(DESKTOP.as_bytes(), out),
            code if code & 0xf000 == super::source::JOB => {
                put(b"Exe.", out);
                for c in crate::shell::clip(name, FILE_BYTES).chars() {
                    let mut utf8 = [0; 4];
                    let safe = !c.is_control() && !"/\\:\"*<>?|".contains(c);
                    put(
                        if safe {
                            c.encode_utf8(&mut utf8).as_bytes()
                        } else {
                            b"_"
                        },
                        out,
                    );
                }
                put(b".", out);
                for shift in (0..8).rev() {
                    put(
                        &[b"0123456789abcdef"[(hash >> (shift * 4)) as usize & 15]],
                        out,
                    );
                }
            }
            code @ 1..=255 => {
                put(b"App.", out);
                let c = code as u8;
                if c >= 100 {
                    put(&[b'0' + c / 100], out);
                }
                if c >= 10 {
                    put(&[b'0' + c / 10 % 10], out);
                }
                put(&[b'0' + c % 10], out);
            }
            _ => put(SESSION.as_bytes(), out),
        }
        let len = at;
        core::str::from_utf8(&out[..len]).unwrap_or(SESSION)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn posts_are_bounded_and_reject_controls_and_gaps() {
        let p = Post {
            icon: crate::shell::icon::FILES,
            flags: flag::SILENT,
            title: "Copy complete",
            body: "3 items copied to Documents",
            actions: ["Open folder", ""],
        };
        let mut out = [0; BODY_BYTES];
        let n = p.encode(&mut out).unwrap();
        assert_eq!(Post::decode(&out[..n]), Ok(p.clone()));
        for len in 0..n {
            assert!(Post::decode(&out[..len]).is_err());
        }
        let max = Post {
            title: &"t".repeat(TITLE_BYTES),
            body: &"b".repeat(BODY_TEXT_BYTES),
            actions: [&"a".repeat(ACTION_BYTES), &"c".repeat(ACTION_BYTES)],
            ..p.clone()
        };
        assert!(max.encode(&mut out).is_ok());
        for bad in [
            Post {
                title: " ",
                ..p.clone()
            },
            Post {
                title: "a\nb",
                ..p.clone()
            },
            Post {
                actions: ["", "x"],
                ..p.clone()
            },
            Post {
                flags: 4,
                ..p.clone()
            },
            Post {
                body: &"b".repeat(BODY_TEXT_BYTES + 1),
                ..p.clone()
            },
        ] {
            assert!(bad.encode(&mut out).is_err());
        }
    }
    #[test]
    fn state_and_records_round_trip_canonically() {
        let mut s = State {
            generation: 9,
            dnd: true,
            unread: 1,
            count: 2,
            ..State::default()
        };
        s.ids[..2].copy_from_slice(&[4, 3]);
        let mut out = [0; BODY_BYTES];
        let n = s.encode(&mut out).unwrap();
        assert_eq!(State::decode(&out[..n]), Ok(s.clone()));
        let mut dup = out;
        dup[12..16].copy_from_slice(&4u32.to_le_bytes());
        assert!(State::decode(&dup[..n]).is_err());
        let r = Record {
            id: 4,
            source: source::JOB | 2,
            icon: 0,
            flags: 0,
            posted: 1234,
            read: false,
            name: "wingui-editor",
            sender: sender::path_hash("/system/bin/wingui-editor.exe"),
            title: &"t".repeat(TITLE_BYTES),
            body: &"b".repeat(BODY_TEXT_BYTES),
            actions: [&"a".repeat(ACTION_BYTES), &"c".repeat(ACTION_BYTES)],
        };
        const { assert!(Record::MAX_BYTES <= BODY_BYTES) };
        let n = r.encode(&mut out).unwrap();
        assert_eq!(Record::decode(&out[..n]), Ok(r));
        let i = Identity {
            source: source::JOB | 1,
            path: "/system/bin/wingui-editor.exe",
        };
        let n = i.encode(&mut out).unwrap();
        assert_eq!(Identity::decode(&out[..n]), Ok(i));
    }
    #[test]
    fn status_round_trips_and_rejects_inconsistent_focus() {
        let s = Status {
            generation: 7,
            flags: status::DND | status::FOCUS | status::FOCUS_DND | status::ENABLED,
            focus_length: 30,
            focus_minutes: 45,
            focus_remaining: 45 * 60,
        };
        assert_eq!(Status::decode(&s.encode()), Ok(s));
        let idle = Status {
            flags: status::ENABLED,
            focus_minutes: 0,
            focus_remaining: 0,
            ..s
        };
        assert_eq!(Status::decode(&idle.encode()), Ok(idle));
        for bad in [
            Status { flags: 128, ..s },
            Status {
                focus_length: 25,
                ..s
            },
            Status {
                focus_remaining: 45 * 60 + 1,
                ..s
            },
            Status {
                focus_minutes: 0,
                ..s
            },
            Status {
                flags: status::FOCUS_DND,
                ..idle
            },
            Status {
                flags: status::SCHEDULED,
                ..idle
            },
        ] {
            assert!(Status::decode(&bad.encode()).is_err(), "{bad:?}");
        }
        assert!(Status::decode(&s.encode()[..15]).is_err());
    }
    #[test]
    fn sender_keys_are_stable_registry_names() {
        let mut out = [0; sender::KEY_BYTES];
        assert_eq!(sender::key(source::SYSTEM, "", 0, &mut out), "System");
        assert_eq!(sender::key(source::SHELL, "", 0, &mut out), "Desktop");
        assert_eq!(sender::key(source::SESSION, "", 0, &mut out), "Session");
        assert_eq!(sender::key(1, "", 0, &mut out), "App.1");
        assert_eq!(sender::key(13, "", 0, &mut out), "App.13");
        let h = sender::path_hash("/system/bin/wingui-editor.exe");
        assert_eq!(h, sender::path_hash("/SYSTEM/bin/WinGUI-Editor.exe"));
        assert_ne!(h, sender::path_hash("/users/1000/wingui-editor.exe"));
        let key = sender::key(source::JOB | 3, "wingui-editor.exe", 0x0012_abcd, &mut out);
        assert_eq!(key, "Exe.wingui-editor.exe.0012abcd");
        let long = "a:b".repeat(30);
        let key = sender::key(source::JOB, &long, u32::MAX, &mut out);
        assert_eq!(key.len(), sender::KEY_BYTES);
        assert!(key.ends_with(".ffffffff") && !key.contains(':'));
        assert!(sender::valid(sender::DEFAULT) && sender::DEFAULT == 15);
        assert!(sender::valid(
            sender::DEFAULT | sender::TOP << sender::PRIORITY_SHIFT
        ));
        assert!(!sender::valid(3 << sender::PRIORITY_SHIFT) && !sender::valid(64));
        assert_eq!(
            sender::priority(sender::HIGH << sender::PRIORITY_SHIFT),
            sender::HIGH
        );
    }
}
