#![no_std]
//! `door-desktop-shell` (P14, spec §60): the restartable desktop shell.
//!
//! It owns no kernel objects and no window authority. Through door.shell it
//! enumerates the window server's authorized windows and follows their
//! changes by polling, publishes taskbar groups and a search catalog derived
//! from its registration database, carries out what the user chose on those
//! surfaces (launch, pin, close, notification actions) and presents the
//! notification service's toasts and centre. It also hosts Start file
//! search (the former windowless search host). If it dies, Init restarts it
//! with backoff; the window server keeps every app and falls back to its own
//! window list meanwhile, and a new incarnation rebuilds everything from
//! enumeration and the notification service: no application restarts.
extern crate alloc;
pub mod model;
pub mod notices;
pub mod registry;
pub mod senders;

use alloc::vec::Vec;
use door_service_protocol::{
    notifications::{Status, sender, status},
    shell::{self as s, command, event},
};
use model::Window;
use notices::{Notice, Presenter};
use registry::{Database, Target};

/// The toast for a launch Init refused (spec §60: unsupported or failed
/// launches say why, with a diagnostic identifier, never silently).
pub fn launch_failure(
    name: &str,
    code: Option<door_service_protocol::Code>,
) -> (alloc::string::String, alloc::string::String) {
    use door_service_protocol::Code;
    let (reason, id) = match code {
        Some(Code::Unsupported) => ("This isn't a program Doors can run.", "UNSUPPORTED"),
        Some(Code::Resources) => (
            "No room for another app. Close an app and try again.",
            "RESOURCES",
        ),
        Some(Code::NotFound) => ("The program file is missing.", "NOTFOUND"),
        Some(Code::Denied) => ("You don't have permission to run it.", "DENIED"),
        Some(Code::InvalidRequest) => ("The program isn't a valid application.", "INVALID"),
        _ => ("The program failed to start.", "FAILED"),
    };
    (
        alloc::format!("Couldn't open {}", s::clip(name, 32)),
        alloc::format!("{reason} (DOOR-LAUNCH-{id})"),
    )
}
/// What the native loop must do after an event (the logic stays testable).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Re-enumerate every window (RESYNC or start).
    Enumerate,
    /// Refresh one window's record (ADDED/CHANGED).
    Fetch(u64),
    Launch(Target),
    /// Write the per-user registration file.
    Persist,
    /// Notification service calls.
    NoticeClear(u32),
    NoticeDnd(bool),
    NoticeActivate(u32, u32),
    NoticeSeen,
    /// Focus (Notification center footer): start a session of n minutes,
    /// stop it, or store a new session length.
    FocusStart(u32),
    FocusStop,
    FocusLength(u32),
    /// Bring the source app of an activated notification forward.
    OpenApp(u8),
    /// Ask the window server to close a window.
    Close(u64),
}
#[derive(Default)]
pub struct Shell {
    pub db: Database,
    pub windows: Vec<Window>,
    pub notices: Vec<Notice>,
    pub dnd: bool,
    /// Unread notifications the centre lists (the bell badge).
    pub unread: u8,
    pub presenter: Presenter,
    /// Per-app sender settings read from the registry.
    pub senders: senders::Senders,
    /// Init's Do not disturb and Focus status, and when it was read.
    pub status: Option<(Status, u64)>,
    /// Publications are stale (republish taskbar / catalog / notices).
    pub taskbar_dirty: bool,
    pub catalog_dirty: bool,
    pub notices_dirty: bool,
    pub generation: u16,
}
impl Shell {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            taskbar_dirty: true,
            catalog_dirty: true,
            notices_dirty: true,
            ..Self::default()
        }
    }
    pub fn next_generation(&mut self) -> u16 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }
    /// Replace the window list after enumeration.
    pub fn set_windows(&mut self, windows: Vec<Window>) {
        if windows != self.windows {
            self.windows = windows;
            self.taskbar_dirty = true;
        }
    }
    pub fn upsert(&mut self, w: Window) {
        match self.windows.iter_mut().find(|v| v.id == w.id) {
            Some(v) if *v == w => return,
            Some(v) => *v = w,
            None => {
                self.windows.push(w);
                self.windows.sort_by_key(|w| w.id);
            }
        }
        self.taskbar_dirty = true;
    }
    pub fn remove(&mut self, id: u64) {
        let before = self.windows.len();
        self.windows.retain(|w| w.id != id);
        self.taskbar_dirty |= self.windows.len() != before;
    }
    /// One door.shell event.
    pub fn on_event(&mut self, e: s::Event) -> Vec<Effect> {
        let mut out = Vec::new();
        match e.kind {
            event::RESYNC => out.push(Effect::Enumerate),
            event::ADDED | event::CHANGED => out.push(Effect::Fetch(e.window)),
            event::REMOVED => self.remove(e.window),
            event::FOCUS => {
                for w in &mut self.windows {
                    let active = w.id == e.window;
                    if (w.flags & s::flag::ACTIVE != 0) != active {
                        w.flags ^= s::flag::ACTIVE;
                        self.taskbar_dirty = true;
                    }
                }
            }
            event::INVOKE => self.invoke(e, &mut out),
            _ => (),
        }
        out
    }
    fn invoke(&mut self, e: s::Event, out: &mut Vec<Effect>) {
        match e.command {
            command::LAUNCH | command::LAUNCH_NEW => {
                if let Some(r) = self.db.get(e.a) {
                    out.push(Effect::Launch(r.target.clone()));
                }
            }
            command::PIN_TASKBAR | command::UNPIN_TASKBAR => {
                if self
                    .db
                    .set_taskbar_pin(e.a, e.command == command::PIN_TASKBAR)
                {
                    self.taskbar_dirty = true;
                    self.catalog_dirty = true;
                    out.push(Effect::Persist);
                }
            }
            command::PIN_START | command::UNPIN_START => {
                if self.db.set_start_pin(e.a, e.command == command::PIN_START) {
                    self.catalog_dirty = true;
                    out.push(Effect::Persist);
                }
            }
            command::CLOSE_GROUP if self.windows.iter().any(|w| w.id == e.window) => {
                out.push(Effect::Close(e.window));
            }
            command::NOTICE_CLEAR => out.push(Effect::NoticeClear(e.a)),
            command::NOTICE_DND => out.push(Effect::NoticeDnd(e.b == 1)),
            command::NOTICE_DISMISS => {
                if self.presenter.dismiss(e.a) {
                    self.notices_dirty = true;
                }
            }
            command::NOTICE_ACTIVATE => {
                out.push(Effect::NoticeActivate(e.a, e.b));
                // Windows brings the source app forward on a body click.
                if e.b == 0
                    && let Some(n) = self.notices.iter().find(|n| n.id == e.a)
                    && let Ok(code) = u8::try_from(n.source)
                    && self.db.by_command(code).is_some()
                {
                    out.push(Effect::OpenApp(code));
                }
            }
            command::NOTICE_SEEN => out.push(Effect::NoticeSeen),
            command::FOCUS_START => out.push(Effect::FocusStart(e.a)),
            command::FOCUS_STOP => out.push(Effect::FocusStop),
            command::FOCUS_LENGTH => out.push(Effect::FocusLength(e.a)),
            _ => (),
        }
    }
    /// The notification service's state and records were read. Each record
    /// takes its sender's stored flags; a sender without an entry yet keeps
    /// the default and is queued for creation (`senders.take_pending`).
    pub fn set_notices(&mut self, mut notices: Vec<Notice>, dnd: bool, unread: u8, now: u64) {
        for n in &mut notices {
            let key = n.sender_key();
            n.policy = match self.senders.flags(&key) {
                Some(flags) => flags,
                None => {
                    self.senders
                        .request(&key, &notices::sender_name(&self.db, n));
                    sender::DEFAULT
                }
            };
        }
        let changed = notices != self.notices || dnd != self.dnd || unread != self.unread;
        self.notices = notices;
        self.dnd = dnd;
        self.unread = unread;
        if self.presenter.update(&self.notices, dnd, now) || changed {
            self.notices_dirty = true;
        }
    }
    /// A sender's flags changed in the registry (Settings): re-apply the
    /// policies to the current records.
    pub fn sender_changed(&mut self, now: u64) {
        let notices = core::mem::take(&mut self.notices);
        let (dnd, unread) = (self.dnd, self.unread);
        self.set_notices(notices, dnd, unread, now);
        self.notices_dirty = true;
    }
    /// Init's Do not disturb / Focus status was read at `now`.
    pub fn set_status(&mut self, s: Status, now: u64) {
        let before = self.status.map(|(old, _)| old);
        // A running session's countdown is derived from `now`; republish
        // only when what the footer shows changes.
        if before != Some(s) {
            self.notices_dirty = true;
        }
        self.status = Some((s, now));
    }
    /// What the Focus footer shows at `now` (ticks are 100 per second).
    pub fn focus_view(&self, now: u64) -> s::FocusView {
        let Some((st, read)) = self.status else {
            return s::FocusView::default();
        };
        let active = st.has(status::FOCUS);
        let elapsed = (now.saturating_sub(read) / 100) as u32;
        s::FocusView {
            active,
            hide_badges: active && st.has(status::HIDE_BADGES),
            hide_flashing: active && st.has(status::HIDE_FLASHING),
            length: u8::try_from(st.focus_length).unwrap_or(0),
            remaining: if active {
                u16::try_from(st.focus_remaining.saturating_sub(elapsed)).unwrap_or(u16::MAX)
            } else {
                0
            },
        }
    }
    /// Toast timers: true when a toast hid (republish).
    pub fn tick(&mut self, now: u64) -> bool {
        if self.presenter.next_deadline().is_some_and(|d| d <= now)
            && self.presenter.update(&self.notices, self.dnd, now)
        {
            self.notices_dirty = true;
        }
        self.notices_dirty
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use door_service_protocol::notifications::source;
    fn invoke(command: u32, a: u32, b: u32) -> s::Event {
        s::Event {
            kind: event::INVOKE,
            command,
            window: 0,
            a,
            b,
        }
    }
    #[test]
    fn events_drive_enumeration_launches_pins_and_notification_effects() {
        let mut db = Database::builtin();
        db.discover("/system/bin", ["wingui-editor.exe"].into_iter());
        let key = registry::executable_key("/system/bin/wingui-editor.exe");
        let mut shell = Shell::new(db);
        assert_eq!(
            shell.on_event(s::Event {
                kind: event::RESYNC,
                ..s::Event::default()
            }),
            [Effect::Enumerate]
        );
        assert_eq!(
            shell.on_event(invoke(command::LAUNCH, key, 0)),
            [Effect::Launch(Target::Executable(
                "/system/bin/wingui-editor.exe".into()
            ))]
        );
        assert_eq!(
            shell.on_event(invoke(command::LAUNCH, registry::NATIVE_KEY | 12, 0)),
            [Effect::Launch(Target::Native(12))]
        );
        assert!(shell.on_event(invoke(command::LAUNCH, 999, 0)).is_empty());
        shell.taskbar_dirty = false;
        assert_eq!(
            shell.on_event(invoke(command::PIN_TASKBAR, key, 0)),
            [Effect::Persist]
        );
        assert!(shell.taskbar_dirty);
        assert!(
            shell
                .on_event(invoke(command::PIN_TASKBAR, key, 0))
                .is_empty()
        );
        shell.set_notices(
            alloc::vec![Notice {
                id: 7,
                source: 2,
                icon: 0,
                flags: 0,
                posted: 10,
                read: false,
                name: Default::default(),
                title: "Saved".into(),
                body: Default::default(),
                actions: Vec::new(),
                sender: 0,
                policy: sender::DEFAULT,
            }],
            false,
            1,
            10,
        );
        assert!(shell.presenter.toasting(7));
        assert_eq!(
            shell.on_event(invoke(command::NOTICE_ACTIVATE, 7, 0)),
            [Effect::NoticeActivate(7, 0), Effect::OpenApp(2)]
        );
        shell.notices_dirty = false;
        assert!(
            shell
                .on_event(invoke(command::NOTICE_DISMISS, 7, 0))
                .is_empty()
        );
        assert!(shell.notices_dirty && !shell.presenter.toasting(7));
        assert_eq!(
            shell.on_event(invoke(command::NOTICE_DND, 0, 1)),
            [Effect::NoticeDnd(true)]
        );
        assert_eq!(
            shell.on_event(invoke(command::NOTICE_CLEAR, 0, 0)),
            [Effect::NoticeClear(0)]
        );
        let _ = source::SYSTEM;
    }
    #[test]
    fn sender_flags_and_focus_status_shape_the_presentation() {
        let mut shell = Shell::new(Database::builtin());
        let notice = |id| Notice {
            id,
            source: 11,
            icon: 0,
            flags: 0,
            posted: 10,
            read: false,
            name: Default::default(),
            title: "Hi".into(),
            body: Default::default(),
            actions: Vec::new(),
            sender: 0,
            policy: 0,
        };
        // Unknown sender: default policy (toast) and one creation request.
        shell.set_notices(alloc::vec![notice(1)], false, 1, 10);
        assert!(shell.presenter.toasting(1));
        assert_eq!(shell.notices[0].policy, sender::DEFAULT);
        let pending = shell.senders.take_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, "App.11");
        // Settings turned banners off: later notices only go to the centre.
        assert!(
            shell
                .senders
                .learn("App.11", sender::ON | sender::CENTER, 2)
        );
        shell.sender_changed(11);
        shell.set_notices(alloc::vec![notice(1), notice(2)], false, 2, 12);
        assert!(!shell.presenter.toasting(2));
        assert!(shell.notices.iter().all(|n| n.listed() && !n.banners()));
        // Focus: the countdown runs from the status read.
        assert_eq!(shell.focus_view(0), s::FocusView::default());
        shell.notices_dirty = false;
        shell.set_status(
            Status {
                generation: 1,
                flags: status::FOCUS | status::HIDE_BADGES | status::ENABLED,
                focus_length: 30,
                focus_minutes: 30,
                focus_remaining: 1800,
            },
            1000,
        );
        assert!(shell.notices_dirty);
        let f = shell.focus_view(1000 + 6000);
        assert!(f.active && f.hide_badges && !f.hide_flashing);
        assert_eq!((f.length, f.remaining), (30, 1740));
    }
    #[test]
    fn window_changes_mark_the_taskbar_stale() {
        let mut shell = Shell::new(Database::builtin());
        shell.taskbar_dirty = false;
        let w = Window {
            id: 5,
            pid: 9,
            app: 0,
            flags: s::flag::WIN32,
            progress_state: 0,
            progress: 0,
            title: "Editor".into(),
        };
        shell.upsert(w.clone());
        assert!(shell.taskbar_dirty);
        shell.taskbar_dirty = false;
        shell.upsert(w.clone());
        assert!(!shell.taskbar_dirty, "identical record");
        shell.on_event(s::Event {
            kind: event::FOCUS,
            window: 5,
            ..s::Event::default()
        });
        assert!(shell.taskbar_dirty && shell.windows[0].flags & s::flag::ACTIVE != 0);
        shell.on_event(s::Event {
            kind: event::REMOVED,
            window: 5,
            ..s::Event::default()
        });
        assert!(shell.windows.is_empty());
    }
}
#[cfg(test)]
mod launch_tests {
    use super::*;
    #[test]
    fn refused_launches_explain_themselves_with_a_diagnostic_identifier() {
        let (title, body) =
            launch_failure("broken", Some(door_service_protocol::Code::Unsupported));
        assert_eq!(title, "Couldn't open broken");
        assert!(body.ends_with("(DOOR-LAUNCH-UNSUPPORTED)"));
        let post = door_service_protocol::notifications::Post {
            icon: door_service_protocol::shell::icon::WINDOWS_APP,
            flags: 0,
            title: &title,
            body: &body,
            actions: ["", ""],
        };
        let mut out = [0; door_service_protocol::BODY_BYTES];
        assert!(post.encode(&mut out).is_ok(), "fits a POST");
        let (_, other) = launch_failure(&"x".repeat(80), None);
        assert!(other.contains("DOOR-LAUNCH-FAILED"));
    }
}
