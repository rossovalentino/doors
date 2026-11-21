//! door.shell: the session-scoped shell integration protocol (spec §60,
//! GUI-F/GUI-H). The desktop shell (`door-desktop-shell`, manifest
//! `DESKTOP_SHELL`) is an ordinary restartable session process. It never
//! inspects application internals: it enumerates the window server's
//! authorized top-level windows, follows their changes by polling, publishes
//! the taskbar groups and search catalog it derives from its registration
//! database, and asks the window server to act on windows. The window server
//! remains authoritative: it validates every request against its own window
//! list, never calls into the shell (the shell polls), and rebuilds an
//! ordinary taskbar from its own window list while no shell is connected.
//!
//! All records are bounded little-endian; reserved bytes are zero.
use crate::{BODY_BYTES, Code};

pub const VERSION: u32 = 1;
/// Window-endpoint opcode range of door.shell.
pub const FIRST: u32 = 0x5300;
/// HELLO(version u32, 0 u32) from the session's shell host only (kernel
/// manifest `DESKTOP_SHELL` and the process Init reports as the shell).
/// Reply: `Hello`. A new incarnation replaces the previous role holder.
pub const HELLO: u32 = 0x5300;
/// WINDOWS(after u64): the first authorized top-level window whose id is
/// greater than `after`, as a `WindowPage`.
pub const WINDOWS: u32 = 0x5301;
/// POLL(): one `Event` (EVENT_BYTES) or an empty reply.
pub const POLL: u32 = 0x5302;
/// PUBLISH(`Entry` record): staged taskbar group or catalog entry; the last
/// index of a generation replaces the published model atomically.
pub const PUBLISH: u32 = 0x5303;
/// ACTION(window u64, action u32, 0 u32): ask the server to act on an
/// ordinary top-level window of the session.
pub const ACTION: u32 = 0x5304;
/// NOTIFY_VIEW(`NoticeView` record): the notification centre and toast
/// presentation the shell derived from the notification service.
pub const NOTIFY_VIEW: u32 = 0x5305;
/// Any window owner: SET_TASKBAR_STATE(window u64, attention u32,
/// progress_state u16, progress u16) for its own top-level window
/// (FlashWindowEx / ITaskbarList3::SetProgressValue equivalents).
pub const SET_TASKBAR_STATE: u32 = 0x5310;
pub const LAST: u32 = 0x531F;

pub const HELLO_BYTES: usize = 16;
pub const EVENT_BYTES: usize = 32;
pub const TITLE_BYTES: usize = 96;
pub const LABEL_BYTES: usize = 64;
/// Taskbar groups and catalog entries per published model.
pub const MAX_ENTRIES: usize = 16;
/// Window ids carried by one taskbar group (more windows still count).
pub const GROUP_WINDOWS: usize = 4;
pub const MAX_PROGRESS: u16 = 1000;

/// Window record flags.
pub mod flag {
    pub const ACTIVE: u32 = 1;
    pub const MINIMIZED: u32 = 2;
    pub const MAXIMIZED: u32 = 4;
    /// The window asked for attention (flashes until activated).
    pub const ATTENTION: u32 = 8;
    pub const HUNG: u32 = 16;
    /// Owned by a Windows-personality (CAP_WIN32) client.
    pub const WIN32: u32 = 32;
    /// Owned/tool windows: no taskbar button of their own.
    pub const SKIP_TASKBAR: u32 = 64;
    /// On another virtual desktop.
    pub const OTHER_DESKTOP: u32 = 128;
    pub const ALL: u32 = 255;
}
/// Taskbar progress states (ITaskbarList3 TBPF_* equivalents).
pub mod progress {
    pub const NONE: u8 = 0;
    pub const NORMAL: u8 = 1;
    pub const PAUSED: u8 = 2;
    pub const ERROR: u8 = 3;
    pub const INDETERMINATE: u8 = 4;
}
/// SET_TASKBAR_STATE attention values.
pub mod attention {
    pub const KEEP: u32 = 0;
    /// Flash until the window is activated (FLASHW_TIMERNOFG).
    pub const REQUEST: u32 = 1;
    pub const STOP: u32 = 2;
}
/// ACTION values.
pub mod action {
    pub const ACTIVATE: u32 = 1;
    pub const MINIMIZE: u32 = 2;
    pub const RESTORE: u32 = 3;
    /// A close request the application may decline (unsaved work).
    pub const CLOSE: u32 = 4;
    /// Taskbar click: minimize the active window, otherwise activate it.
    pub const TOGGLE: u32 = 5;
}
/// POLL event kinds.
pub mod event {
    pub const ADDED: u32 = 1;
    pub const CHANGED: u32 = 2;
    pub const REMOVED: u32 = 3;
    pub const FOCUS: u32 = 4;
    /// The bounded event queue overflowed: enumerate again.
    pub const RESYNC: u32 = 5;
    /// The user acted on a shell-published surface (`command`).
    pub const INVOKE: u32 = 6;
    pub const LAST: u32 = INVOKE;
}
/// INVOKE commands. `a` is the entry key (taskbar/catalog) or notification
/// id; `b` an action index; `window` a target window when one applies.
pub mod command {
    /// Primary click on a published taskbar group with no window, or a
    /// catalog entry chosen in Start/search: launch it.
    pub const LAUNCH: u32 = 1;
    /// Shift+click / middle click / jump list "new window".
    pub const LAUNCH_NEW: u32 = 2;
    pub const PIN_TASKBAR: u32 = 3;
    pub const UNPIN_TASKBAR: u32 = 4;
    /// Jump list "Close window"/"Close all windows" of a group.
    pub const CLOSE_GROUP: u32 = 5;
    /// Notification centre: dismiss one (`a`), all (`a` = 0).
    pub const NOTICE_CLEAR: u32 = 6;
    /// Do not disturb toggled (`b` = 1 on).
    pub const NOTICE_DND: u32 = 7;
    /// Notification body (`b` = 0) or action button (`b` = 1 + index).
    pub const NOTICE_ACTIVATE: u32 = 8;
    /// The notification centre was opened (unread badge clears).
    pub const NOTICE_SEEN: u32 = 9;
    /// A toast was dismissed with its close button.
    pub const NOTICE_DISMISS: u32 = 10;
    pub const PIN_START: u32 = 11;
    pub const UNPIN_START: u32 = 12;
    /// Notification center Focus footer: start a session of `a` minutes
    /// (one of `preferences::Focus::LENGTHS`).
    pub const FOCUS_START: u32 = 13;
    /// "Stop focus": end the running session early.
    pub const FOCUS_STOP: u32 = 14;
    /// The footer's minutes stepper chose `a` minutes (the FOCUS length).
    pub const FOCUS_LENGTH: u32 = 15;
    pub const LAST: u32 = FOCUS_LENGTH;
}
/// PUBLISH entry kinds.
pub mod kind {
    pub const TASKBAR: u8 = 1;
    pub const CATALOG: u8 = 2;
}
/// Taskbar group / catalog entry flags.
pub mod entry {
    pub const PINNED: u16 = 1;
    pub const RUNNING: u16 = 2;
    pub const ACTIVE: u16 = 4;
    pub const ATTENTION: u16 = 8;
    /// Catalog: listed in Start's All list (otherwise search only).
    pub const LISTED: u16 = 16;
    /// Catalog: pinned to Start.
    pub const START_PINNED: u16 = 32;
    pub const ALL: u16 = 63;
}
/// Icon references (bundled artwork; never arbitrary pixels).
pub mod icon {
    pub const GENERIC_APP: u16 = 0;
    pub const FILES: u16 = 1;
    pub const NOTES: u16 = 2;
    pub const SETTINGS: u16 = 3;
    pub const TERMINAL: u16 = 4;
    pub const PREVIEW: u16 = 5;
    pub const TASK_MANAGER: u16 = 6;
    pub const GALLERY: u16 = 7;
    pub const CAPTIONS: u16 = 8;
    /// A Windows GUI executable without bundled artwork.
    pub const WINDOWS_APP: u16 = 9;
    pub const SYSTEM: u16 = 10;
    /// Inbox apps: their installed Windows icons.
    pub const CALCULATOR: u16 = 11;
    pub const CLOCK: u16 = 12;
    pub const PAINT: u16 = 13;
    pub const SNIPPING_TOOL: u16 = 14;
    pub const WEATHER: u16 = 15;
    pub const MAGNIFIER: u16 = 16;
    pub const NARRATOR: u16 = 17;
    pub const ON_SCREEN_KEYBOARD: u16 = 18;
    pub const VOICE_ACCESS: u16 = 19;
    pub const LAST: u16 = VOICE_ACCESS;
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
/// The Windows spelling of a native path, as a Windows program sees its
/// own: `/compat/X` is `C:\X`, `/users/X` is `C:\Users\X`, anything else
/// `C:` followed by it. Writes into `out` and returns the length, or None
/// when it does not fit.
pub fn windows_spelling(native: &str, out: &mut [u8]) -> Option<usize> {
    // Already a Windows path: as it is.
    if native.len() >= 2 && native.as_bytes()[1] == b':' {
        let length = native.len();
        out.get_mut(..length)?.copy_from_slice(native.as_bytes());
        return Some(length);
    }
    // A prefix counts only on a component boundary.
    let under = |root: &str| {
        native
            .strip_prefix(root)
            .filter(|rest| rest.is_empty() || rest.starts_with('/'))
    };
    let (head, rest): (&[u8], &str) = if let Some(rest) = under("/compat") {
        (b"C:", if rest.is_empty() { "/" } else { rest })
    } else if let Some(rest) = under("/users") {
        (b"C:\\Users", rest)
    } else {
        (b"C:", native)
    };
    let length = head.len() + rest.len();
    if length > out.len() || !(rest.is_empty() || rest.starts_with('/')) {
        return None;
    }
    out[..head.len()].copy_from_slice(head);
    for (slot, byte) in out[head.len()..length].iter_mut().zip(rest.bytes()) {
        *slot = if byte == b'/' { b'\\' } else { byte };
    }
    Some(length)
}
/// Longest prefix of `s` that fits `max` bytes on a character boundary.
pub fn clip(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hello {
    pub version: u32,
    /// Window-list generation (changes with every add/remove/change).
    pub generation: u32,
    pub windows: u32,
    /// 1 while the secure lock/sign-in surface owns the display.
    pub locked: bool,
}
impl Hello {
    pub fn encode(&self) -> [u8; HELLO_BYTES] {
        let mut o = [0; HELLO_BYTES];
        o[..4].copy_from_slice(&self.version.to_le_bytes());
        o[4..8].copy_from_slice(&self.generation.to_le_bytes());
        o[8..12].copy_from_slice(&self.windows.to_le_bytes());
        o[12] = u8::from(self.locked);
        o
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() != HELLO_BYTES || u32_at(b, 0) != VERSION || b[12] > 1 || b[13..] != [0; 3] {
            return Err(Code::InvalidRequest);
        }
        Ok(Self {
            version: VERSION,
            generation: u32_at(b, 4),
            windows: u32_at(b, 8),
            locked: b[12] == 1,
        })
    }
}

/// One authorized top-level window as the shell may see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window<'a> {
    pub id: u64,
    pub pid: u64,
    pub instance: u64,
    /// Init desktop command of a catalog app (1,2,3,9,10,11) or accessory
    /// (12,13); 0 when the owner is not a catalog app.
    pub app: u32,
    pub flags: u32,
    pub progress_state: u8,
    pub progress: u16,
    /// Kernel-authenticated manifest of the owner.
    pub manifest: u32,
    pub title: &'a str,
}
/// WINDOWS reply: `more` (another window follows this one), the window-list
/// generation, then optionally one window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowPage<'a> {
    pub generation: u32,
    pub window: Option<Window<'a>>,
}
const PAGE_HEADER: usize = 8;
const WINDOW_FIXED: usize = 40;
impl WindowPage<'_> {
    pub const MAX_BYTES: usize = PAGE_HEADER + WINDOW_FIXED + TITLE_BYTES;
    pub fn encode(&self, out: &mut [u8]) -> Result<usize, Code> {
        out[..PAGE_HEADER].fill(0);
        out[..4].copy_from_slice(&u32::from(self.window.is_some()).to_le_bytes());
        out[4..8].copy_from_slice(&self.generation.to_le_bytes());
        let Some(w) = &self.window else {
            return Ok(PAGE_HEADER);
        };
        let title = clip(w.title, TITLE_BYTES);
        if w.id == 0
            || w.pid == 0
            || w.flags & !flag::ALL != 0
            || w.progress_state > progress::INDETERMINATE
            || w.progress > MAX_PROGRESS
            || out.len() < PAGE_HEADER + WINDOW_FIXED + title.len()
        {
            return Err(Code::InvalidRequest);
        }
        let r = &mut out[PAGE_HEADER..];
        r[..WINDOW_FIXED].fill(0);
        r[..8].copy_from_slice(&w.id.to_le_bytes());
        r[8..16].copy_from_slice(&w.pid.to_le_bytes());
        r[16..24].copy_from_slice(&w.instance.to_le_bytes());
        r[24..28].copy_from_slice(&w.app.to_le_bytes());
        r[28..32].copy_from_slice(&w.flags.to_le_bytes());
        r[32..34].copy_from_slice(&w.progress.to_le_bytes());
        r[34] = w.progress_state;
        r[35] = title.len() as u8;
        r[36..40].copy_from_slice(&w.manifest.to_le_bytes());
        r[WINDOW_FIXED..WINDOW_FIXED + title.len()].copy_from_slice(title.as_bytes());
        Ok(PAGE_HEADER + WINDOW_FIXED + title.len())
    }
    pub fn decode(b: &[u8]) -> Result<WindowPage<'_>, Code> {
        if b.len() < PAGE_HEADER || u32_at(b, 0) > 1 {
            return Err(Code::InvalidRequest);
        }
        let generation = u32_at(b, 4);
        if u32_at(b, 0) == 0 {
            if b.len() != PAGE_HEADER {
                return Err(Code::InvalidRequest);
            }
            return Ok(WindowPage {
                generation,
                window: None,
            });
        }
        let r = &b[PAGE_HEADER..];
        if r.len() < WINDOW_FIXED || r.len() != WINDOW_FIXED + r[35] as usize {
            return Err(Code::InvalidRequest);
        }
        let w = Window {
            id: u64_at(r, 0),
            pid: u64_at(r, 8),
            instance: u64_at(r, 16),
            app: u32_at(r, 24),
            flags: u32_at(r, 28),
            progress: u16_at(r, 32),
            progress_state: r[34],
            manifest: u32_at(r, 36),
            title: text(&r[WINDOW_FIXED..])?,
        };
        if w.id == 0
            || w.pid == 0
            || w.flags & !flag::ALL != 0
            || w.progress_state > progress::INDETERMINATE
            || w.progress > MAX_PROGRESS
            || w.title.len() > TITLE_BYTES
        {
            return Err(Code::InvalidRequest);
        }
        Ok(WindowPage {
            generation,
            window: Some(w),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Event {
    pub kind: u32,
    pub command: u32,
    pub window: u64,
    pub a: u32,
    pub b: u32,
}
impl Event {
    pub fn encode(self) -> [u8; EVENT_BYTES] {
        let mut o = [0; EVENT_BYTES];
        o[..4].copy_from_slice(&self.kind.to_le_bytes());
        o[4..8].copy_from_slice(&self.command.to_le_bytes());
        o[8..16].copy_from_slice(&self.window.to_le_bytes());
        o[16..20].copy_from_slice(&self.a.to_le_bytes());
        o[20..24].copy_from_slice(&self.b.to_le_bytes());
        o
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() != EVENT_BYTES || b[24..] != [0; 8] {
            return Err(Code::InvalidRequest);
        }
        let e = Self {
            kind: u32_at(b, 0),
            command: u32_at(b, 4),
            window: u64_at(b, 8),
            a: u32_at(b, 16),
            b: u32_at(b, 20),
        };
        let valid = match e.kind {
            event::INVOKE => (1..=command::LAST).contains(&e.command),
            1..=event::RESYNC => e.command == 0,
            _ => false,
        };
        if !valid {
            return Err(Code::InvalidRequest);
        }
        Ok(e)
    }
}

/// A published taskbar group or catalog entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry<'a> {
    pub kind: u8,
    pub generation: u16,
    pub index: u8,
    pub count: u8,
    /// Registration key, unique within one model.
    pub key: u32,
    pub icon: u16,
    pub flags: u16,
    pub progress_state: u8,
    pub progress: u16,
    /// Open windows of the group (all of them count toward `windows`).
    pub windows: u8,
    pub ids: [u64; GROUP_WINDOWS],
    pub label: &'a str,
}
const ENTRY_FIXED: usize = 20 + 8 * GROUP_WINDOWS;
impl Entry<'_> {
    pub const MAX_BYTES: usize = ENTRY_FIXED + LABEL_BYTES;
    /// An empty model of `kind` (clears it).
    pub fn clear(kind: u8, generation: u16) -> Entry<'static> {
        Entry {
            kind,
            generation,
            index: 0,
            count: 0,
            key: 0,
            icon: 0,
            flags: 0,
            progress_state: 0,
            progress: 0,
            windows: 0,
            ids: [0; GROUP_WINDOWS],
            label: "",
        }
    }
    fn check(&self) -> Result<(), Code> {
        let listed = self.ids.iter().filter(|id| **id != 0).count();
        let empty = self.count == 0;
        if !matches!(self.kind, kind::TASKBAR | kind::CATALOG)
            || self.count as usize > MAX_ENTRIES
            || (!empty && self.index >= self.count)
            || (empty && (self.index != 0 || self.key != 0 || !self.label.is_empty()))
            || (!empty && (self.key == 0 || self.label.is_empty()))
            || self.icon > icon::LAST
            || self.flags & !entry::ALL != 0
            || self.progress_state > progress::INDETERMINATE
            || self.progress > MAX_PROGRESS
            || self.label.len() > LABEL_BYTES
            || self.label.chars().any(char::is_control)
            || (self.windows as usize) < listed
            || self.ids[..listed].contains(&0)
            || (1..listed).any(|i| self.ids[..i].contains(&self.ids[i]))
            || (self.kind == kind::CATALOG && (self.windows != 0 || self.progress_state != 0))
        {
            return Err(Code::InvalidRequest);
        }
        Ok(())
    }
    pub fn encode(&self, out: &mut [u8]) -> Result<usize, Code> {
        self.check()?;
        let len = ENTRY_FIXED + self.label.len();
        if out.len() < len || len > BODY_BYTES {
            return Err(Code::InvalidRequest);
        }
        out[..ENTRY_FIXED].fill(0);
        out[0] = self.kind;
        out[1] = self.index;
        out[2] = self.count;
        out[3] = self.label.len() as u8;
        out[4..6].copy_from_slice(&self.generation.to_le_bytes());
        out[6..8].copy_from_slice(&self.icon.to_le_bytes());
        out[8..12].copy_from_slice(&self.key.to_le_bytes());
        out[12..14].copy_from_slice(&self.flags.to_le_bytes());
        out[14..16].copy_from_slice(&self.progress.to_le_bytes());
        out[16] = self.progress_state;
        out[17] = self.windows;
        for (i, id) in self.ids.iter().enumerate() {
            out[20 + i * 8..28 + i * 8].copy_from_slice(&id.to_le_bytes());
        }
        out[ENTRY_FIXED..len].copy_from_slice(self.label.as_bytes());
        Ok(len)
    }
    pub fn decode(b: &[u8]) -> Result<Entry<'_>, Code> {
        if b.len() < ENTRY_FIXED || b.len() != ENTRY_FIXED + b[3] as usize || b[18..20] != [0; 2] {
            return Err(Code::InvalidRequest);
        }
        let mut ids = [0; GROUP_WINDOWS];
        for (i, id) in ids.iter_mut().enumerate() {
            *id = u64_at(b, 20 + i * 8);
        }
        let e = Entry {
            kind: b[0],
            index: b[1],
            count: b[2],
            generation: u16_at(b, 4),
            icon: u16_at(b, 6),
            key: u32_at(b, 8),
            flags: u16_at(b, 12),
            progress: u16_at(b, 14),
            progress_state: b[16],
            windows: b[17],
            ids,
            label: text(&b[ENTRY_FIXED..])?,
        };
        e.check()?;
        Ok(e)
    }
}

/// One notification as the shell asks the window server to present it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoticeView<'a> {
    pub generation: u16,
    /// 0..count; `count` 0 publishes an empty centre (header only).
    pub index: u8,
    pub count: u8,
    pub dnd: bool,
    /// Unread notifications (the bell badge), saturating.
    pub badge: u8,
    pub id: u32,
    pub icon: u16,
    /// Currently shown as a toast popup (stack order = index order).
    pub toast: bool,
    pub source: &'a str,
    pub time: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub actions: [&'a str; 2],
    /// A banner whose sender turned "Show notifications in notification
    /// center" off: presented as a toast only, never as a centre card.
    pub toast_only: bool,
    /// Focus as the centre's footer shows it (same in every record of a
    /// generation).
    pub focus: FocusView,
}
/// Focus session state carried by every `NoticeView` record (bytes 19..24:
/// flags, length minutes, remaining seconds u16, zero).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FocusView {
    /// A session is running.
    pub active: bool,
    /// Focus options while a session runs: no badges, no flashing.
    pub hide_badges: bool,
    pub hide_flashing: bool,
    /// The user's session length (minutes, one of
    /// `preferences::Focus::LENGTHS`; 0 = not known yet).
    pub length: u8,
    /// Seconds left in the running session when published (0 when idle).
    pub remaining: u16,
}
impl FocusView {
    fn encode(self, out: &mut [u8]) {
        out[0] = u8::from(self.active)
            | u8::from(self.hide_badges) << 1
            | u8::from(self.hide_flashing) << 2;
        out[1] = self.length;
        out[2..4].copy_from_slice(&self.remaining.to_le_bytes());
        out[4] = 0;
    }
    fn decode(b: &[u8]) -> Self {
        Self {
            active: b[0] & 1 != 0,
            hide_badges: b[0] & 2 != 0,
            hide_flashing: b[0] & 4 != 0,
            length: b[1],
            remaining: u16_at(b, 2),
        }
    }
    fn valid(self) -> bool {
        let length = u32::from(self.length);
        (length == 0 || crate::preferences::Focus::LENGTHS.contains(&length))
            && (self.active || self.remaining == 0)
            && u32::from(self.remaining) <= 240 * 60
    }
}
pub mod notice {
    pub const SOURCE_BYTES: usize = 24;
    pub const TIME_BYTES: usize = 12;
    pub const TITLE_BYTES: usize = 48;
    pub const BODY_BYTES: usize = 96;
    pub const ACTION_BYTES: usize = 16;
    /// Notifications presented at once (toasts plus centre).
    pub const MAX: usize = 12;
}
const NOTICE_FIXED: usize = 24;
impl NoticeView<'_> {
    pub fn empty(generation: u16, dnd: bool) -> NoticeView<'static> {
        NoticeView {
            generation,
            index: 0,
            count: 0,
            dnd,
            badge: 0,
            id: 0,
            icon: 0,
            toast: false,
            source: "",
            time: "",
            title: "",
            body: "",
            actions: ["", ""],
            toast_only: false,
            focus: FocusView::default(),
        }
    }
    /// The same record carrying `focus`.
    pub fn with_focus(self, focus: FocusView) -> Self {
        Self { focus, ..self }
    }
    fn check(&self) -> Result<(), Code> {
        let empty = self.count == 0;
        let fields = [
            (self.source, notice::SOURCE_BYTES),
            (self.time, notice::TIME_BYTES),
            (self.title, notice::TITLE_BYTES),
            (self.body, notice::BODY_BYTES),
            (self.actions[0], notice::ACTION_BYTES),
            (self.actions[1], notice::ACTION_BYTES),
        ];
        if self.count as usize > notice::MAX
            || (!empty && self.index >= self.count)
            || (empty
                && (self.index != 0
                    || self.id != 0
                    || self.toast
                    || self.toast_only
                    || self.badge != 0
                    || fields.iter().any(|(s, _)| !s.is_empty())))
            || (self.toast_only && !self.toast)
            || !self.focus.valid()
            || (!empty && (self.id == 0 || self.title.is_empty() || self.source.is_empty()))
            || self.icon > icon::LAST
            || fields.iter().any(|(s, max)| s.len() > *max)
            || fields.iter().any(|(s, _)| s.chars().any(char::is_control))
            || (self.actions[0].is_empty() && !self.actions[1].is_empty())
        {
            return Err(Code::InvalidRequest);
        }
        Ok(())
    }
    pub fn encode(&self, out: &mut [u8]) -> Result<usize, Code> {
        self.check()?;
        let parts = [
            self.source,
            self.time,
            self.title,
            self.body,
            self.actions[0],
            self.actions[1],
        ];
        let len = NOTICE_FIXED + parts.iter().map(|p| p.len()).sum::<usize>();
        if out.len() < len || len > BODY_BYTES {
            return Err(Code::InvalidRequest);
        }
        out[..NOTICE_FIXED].fill(0);
        out[0] = self.index;
        out[1] = self.count;
        out[2] = u8::from(self.dnd);
        out[3] = self.badge;
        out[4..6].copy_from_slice(&self.generation.to_le_bytes());
        out[6..8].copy_from_slice(&self.icon.to_le_bytes());
        out[8..12].copy_from_slice(&self.id.to_le_bytes());
        out[12] = u8::from(self.toast) | u8::from(self.toast_only) << 1;
        for (i, p) in parts.iter().enumerate() {
            out[13 + i] = p.len() as u8;
        }
        self.focus.encode(&mut out[19..NOTICE_FIXED]);
        let mut at = NOTICE_FIXED;
        for p in parts {
            out[at..at + p.len()].copy_from_slice(p.as_bytes());
            at += p.len();
        }
        Ok(len)
    }
    pub fn decode(b: &[u8]) -> Result<NoticeView<'_>, Code> {
        if b.len() < NOTICE_FIXED
            || b[2] > 1
            || b[12] > 3
            || b[19] & !7 != 0
            || b[NOTICE_FIXED - 1] != 0
        {
            return Err(Code::InvalidRequest);
        }
        let lens: [usize; 6] = core::array::from_fn(|i| b[13 + i] as usize);
        if b.len() != NOTICE_FIXED + lens.iter().sum::<usize>() {
            return Err(Code::InvalidRequest);
        }
        let mut at = NOTICE_FIXED;
        let mut parts = [""; 6];
        for (i, n) in lens.iter().enumerate() {
            parts[i] = text(&b[at..at + n])?;
            at += n;
        }
        let v = NoticeView {
            index: b[0],
            count: b[1],
            dnd: b[2] == 1,
            badge: b[3],
            generation: u16_at(b, 4),
            icon: u16_at(b, 6),
            id: u32_at(b, 8),
            toast: b[12] & 1 != 0,
            source: parts[0],
            time: parts[1],
            title: parts[2],
            body: parts[3],
            actions: [parts[4], parts[5]],
            toast_only: b[12] & 2 != 0,
            focus: FocusView::decode(&b[19..NOTICE_FIXED]),
        };
        v.check()?;
        Ok(v)
    }
}

/// SET_TASKBAR_STATE body.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskbarState {
    pub window: u64,
    pub attention: u32,
    pub progress_state: u8,
    pub progress: u16,
}
impl TaskbarState {
    pub const BYTES: usize = 16;
    pub fn encode(&self) -> [u8; Self::BYTES] {
        let mut o = [0; Self::BYTES];
        o[..8].copy_from_slice(&self.window.to_le_bytes());
        o[8..12].copy_from_slice(&self.attention.to_le_bytes());
        o[12..14].copy_from_slice(&u16::from(self.progress_state).to_le_bytes());
        o[14..16].copy_from_slice(&self.progress.to_le_bytes());
        o
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() != Self::BYTES {
            return Err(Code::InvalidRequest);
        }
        let state = u16_at(b, 12);
        let s = Self {
            window: u64_at(b, 0),
            attention: u32_at(b, 8),
            progress_state: state as u8,
            progress: u16_at(b, 14),
        };
        if s.window == 0
            || s.attention > attention::STOP
            || state > u16::from(progress::INDETERMINATE)
            || s.progress > MAX_PROGRESS
        {
            return Err(Code::InvalidRequest);
        }
        Ok(s)
    }
}

/// ACTION body.
pub fn encode_action(window: u64, value: u32) -> [u8; 16] {
    let mut o = [0; 16];
    o[..8].copy_from_slice(&window.to_le_bytes());
    o[8..12].copy_from_slice(&value.to_le_bytes());
    o
}
pub fn decode_action(b: &[u8]) -> Result<(u64, u32), Code> {
    if b.len() != 16 || b[12..] != [0; 4] {
        return Err(Code::InvalidRequest);
    }
    let (window, value) = (u64_at(b, 0), u32_at(b, 8));
    if window == 0 || !(action::ACTIVATE..=action::TOGGLE).contains(&value) {
        return Err(Code::InvalidRequest);
    }
    Ok((window, value))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_paths_take_their_windows_spelling() {
        let spelled = |native: &str, windows: &str| {
            let mut out = [0u8; 96];
            windows_spelling(native, &mut out).map(|n| &out[..n] == windows.as_bytes())
                == Some(true)
        };
        assert!(spelled(
            "/compat/Program Files/Doors Setup Sample/uninstall.exe",
            "C:\\Program Files\\Doors Setup Sample\\uninstall.exe"
        ));
        assert!(spelled(
            "/users/1001/p16install/setupprobe.exe",
            "C:\\Users\\1001\\p16install\\setupprobe.exe"
        ));
        assert!(spelled("/compat", "C:\\"));
        assert!(spelled(
            "/system/bin/tinybasic.exe",
            "C:\\system\\bin\\tinybasic.exe"
        ));
        // Not a component boundary.
        assert!(spelled("/compatible/x", "C:\\compatible\\x"));
        assert!(spelled(
            "C:\\Windows\\System32\\MsiExec.exe",
            "C:\\Windows\\System32\\MsiExec.exe"
        ));
        let mut tiny = [0u8; 4];
        assert_eq!(windows_spelling("/compat/Windows", &mut tiny), None);
    }
    #[test]
    fn hello_and_events_round_trip_and_reject_reserved_bytes() {
        let h = Hello {
            version: VERSION,
            generation: 7,
            windows: 3,
            locked: true,
        };
        assert_eq!(Hello::decode(&h.encode()), Ok(h));
        let mut bad = h.encode();
        bad[15] = 1;
        assert!(Hello::decode(&bad).is_err());
        let e = Event {
            kind: event::INVOKE,
            command: command::PIN_TASKBAR,
            window: 9,
            a: 0x1000_0001,
            b: 0,
        };
        assert_eq!(Event::decode(&e.encode()), Ok(e));
        for (kind, command) in [(0, 0), (event::ADDED, 1), (event::INVOKE, 0), (99, 0)] {
            let e = Event {
                kind,
                command,
                ..Event::default()
            };
            assert!(Event::decode(&e.encode()).is_err(), "{kind} {command}");
        }
        let mut b = e.encode();
        b[31] = 1;
        assert!(Event::decode(&b).is_err());
    }
    #[test]
    fn window_pages_bound_titles_and_validate_flags() {
        let long = "w".repeat(200);
        let page = WindowPage {
            generation: 4,
            window: Some(Window {
                id: 0x1_0000_0002,
                pid: 12,
                instance: 77,
                app: 2,
                flags: flag::ACTIVE | flag::WIN32,
                progress_state: progress::NORMAL,
                progress: 420,
                manifest: 15,
                title: &long,
            }),
        };
        let mut out = [0; WindowPage::MAX_BYTES];
        let n = page.encode(&mut out).unwrap();
        assert_eq!(n, WindowPage::MAX_BYTES);
        let back = WindowPage::decode(&out[..n]).unwrap();
        let w = back.window.unwrap();
        assert_eq!(w.title.len(), TITLE_BYTES);
        assert_eq!(
            (w.id, w.pid, w.app, w.progress),
            (0x1_0000_0002, 12, 2, 420)
        );
        let none = WindowPage {
            generation: 5,
            window: None,
        };
        let n = none.encode(&mut out).unwrap();
        assert_eq!(WindowPage::decode(&out[..n]).unwrap(), none);
        let n = page.encode(&mut out).unwrap();
        for len in 0..n {
            if len != PAGE_HEADER {
                assert!(WindowPage::decode(&out[..len]).is_err(), "{len}");
            }
        }
        let mut bad = out;
        bad[PAGE_HEADER + 28] = 0;
        bad[PAGE_HEADER + 29] = 1; // undefined flag bit 8
        assert!(WindowPage::decode(&bad[..n]).is_err());
        assert_eq!(clip("ab\u{e9}c", 3), "ab");
    }
    #[test]
    fn entries_are_canonical_and_reject_duplicate_or_missing_windows() {
        let e = Entry {
            kind: kind::TASKBAR,
            generation: 3,
            index: 1,
            count: 2,
            key: 0x2000_0001,
            icon: icon::WINDOWS_APP,
            flags: entry::RUNNING | entry::ATTENTION,
            progress_state: progress::PAUSED,
            progress: 500,
            windows: 5,
            ids: [11, 12, 0, 0],
            label: "Door P13 editor",
        };
        let mut out = [0; BODY_BYTES];
        let n = e.encode(&mut out).unwrap();
        assert_eq!(Entry::decode(&out[..n]), Ok(e.clone()));
        let clear = Entry::clear(kind::CATALOG, 9);
        let n = clear.encode(&mut out).unwrap();
        assert_eq!(Entry::decode(&out[..n]), Ok(clear));
        for bad in [
            Entry {
                ids: [11, 11, 0, 0],
                ..e.clone()
            },
            Entry {
                ids: [0, 12, 0, 0],
                ..e.clone()
            },
            Entry {
                windows: 1,
                ..e.clone()
            },
            Entry {
                index: 2,
                ..e.clone()
            },
            Entry {
                key: 0,
                ..e.clone()
            },
            Entry {
                label: "",
                ..e.clone()
            },
            Entry {
                icon: icon::LAST + 1,
                ..e.clone()
            },
            Entry {
                kind: kind::CATALOG,
                ..e.clone()
            },
            Entry {
                progress: MAX_PROGRESS + 1,
                ..e.clone()
            },
        ] {
            assert!(bad.encode(&mut out).is_err(), "{bad:?}");
        }
        let n = e.encode(&mut out).unwrap();
        let mut ctl = out;
        ctl[ENTRY_FIXED] = b'\n';
        assert!(Entry::decode(&ctl[..n]).is_err());
    }
    #[test]
    fn notice_views_fit_one_body_and_validate_fields() {
        let v = NoticeView {
            generation: 2,
            index: 0,
            count: 1,
            dnd: false,
            badge: 1,
            id: 5,
            icon: icon::FILES,
            toast: true,
            source: "File Explorer",
            time: "Now",
            title: &"t".repeat(notice::TITLE_BYTES),
            body: &"b".repeat(notice::BODY_BYTES),
            actions: [&"a".repeat(notice::ACTION_BYTES), "Dismiss"],
            toast_only: true,
            focus: FocusView {
                active: true,
                hide_badges: true,
                hide_flashing: false,
                length: 45,
                remaining: 44 * 60 + 59,
            },
        };
        let mut out = [0; BODY_BYTES];
        let n = v.encode(&mut out).unwrap();
        assert_eq!(NoticeView::decode(&out[..n]), Ok(v.clone()));
        let idle = FocusView {
            length: 30,
            ..FocusView::default()
        };
        let e = NoticeView::empty(3, true).with_focus(idle);
        let n = e.encode(&mut out).unwrap();
        assert_eq!(NoticeView::decode(&out[..n]), Ok(e));
        // Reserved bits and the last reserved byte stay zero.
        let mut bad = out;
        bad[19] = 8;
        assert!(NoticeView::decode(&bad[..n]).is_err());
        let mut bad = out;
        bad[23] = 1;
        assert!(NoticeView::decode(&bad[..n]).is_err());
        for bad in [
            NoticeView {
                toast: false,
                ..v.clone()
            },
            NoticeView {
                focus: FocusView {
                    length: 25,
                    ..v.focus
                },
                ..v.clone()
            },
            NoticeView {
                focus: FocusView {
                    active: false,
                    ..v.focus
                },
                ..v.clone()
            },
            NoticeView {
                title: "",
                ..v.clone()
            },
            NoticeView {
                actions: ["", "x"],
                ..v.clone()
            },
            NoticeView {
                index: 1,
                ..v.clone()
            },
            NoticeView { id: 0, ..v.clone() },
        ] {
            assert!(bad.encode(&mut out).is_err());
        }
    }
    #[test]
    fn taskbar_state_and_actions_reject_out_of_range_values() {
        let s = TaskbarState {
            window: 3,
            attention: attention::REQUEST,
            progress_state: progress::ERROR,
            progress: 1000,
        };
        assert_eq!(TaskbarState::decode(&s.encode()), Ok(s));
        let mut b = s.encode();
        b[14..16].copy_from_slice(&1001u16.to_le_bytes());
        assert!(TaskbarState::decode(&b).is_err());
        let mut b = s.encode();
        b[12] = 5;
        assert!(TaskbarState::decode(&b).is_err());
        assert_eq!(decode_action(&encode_action(8, action::CLOSE)), Ok((8, 4)));
        assert!(decode_action(&encode_action(8, 0)).is_err());
        assert!(decode_action(&encode_action(0, 1)).is_err());
    }
}
