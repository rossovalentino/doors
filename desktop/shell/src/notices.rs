//! Notification presentation policy (Windows 11): a new notification pops
//! up as a toast for `TOAST_TICKS` (5 s, the "Dismiss notifications after"
//! default) unless Do not disturb is on or it was posted silently; at most
//! `MAX_TOASTS` show at once; everything stays in the notification centre
//! until cleared. A restarted shell does not toast what it already showed:
//! only unread notifications younger than the toast time pop up again.
//!
//! Each notification carries its sender's settings (`policy`, the
//! `notifications::sender` flags the shell read from the registry): a sender
//! turned off is neither toasted nor listed; with banners off it is listed
//! only; with "Show notifications in notification center" off it is toasted
//! only; its priority orders the centre (top, high, normal; newest first
//! within a level) and keeps a top or high toast over normal ones when the
//! toasts overflow.
extern crate alloc;
use crate::registry::Database;
use alloc::{format, string::String, vec::Vec};
use door_service_protocol::{
    notifications::{flag, sender, source},
    shell::{self as s, icon, notice},
};

/// 5 s of 10 ms monotonic ticks.
pub const TOAST_TICKS: u64 = 500;
pub const MAX_TOASTS: usize = 3;

/// A retained notification (an owned `notifications::Record`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub id: u32,
    pub source: u32,
    pub icon: u16,
    pub flags: u16,
    pub posted: u64,
    pub read: bool,
    pub name: String,
    pub title: String,
    pub body: String,
    pub actions: Vec<String>,
    /// `sender::path_hash` of a Windows app's executable (0 otherwise).
    pub sender: u32,
    /// The sender's settings (`notifications::sender` flags).
    pub policy: u32,
}
impl Notice {
    pub fn from_record(r: &door_service_protocol::notifications::Record<'_>) -> Self {
        Self {
            id: r.id,
            source: r.source,
            icon: r.icon,
            flags: r.flags,
            posted: r.posted,
            read: r.read,
            name: r.name.into(),
            title: r.title.into(),
            body: r.body.into(),
            actions: r
                .actions
                .iter()
                .filter(|a| !a.is_empty())
                .map(|a| String::from(*a))
                .collect(),
            sender: r.sender,
            policy: sender::DEFAULT,
        }
    }
    /// The sender's registry key (`notifications::sender`).
    pub fn sender_key(&self) -> String {
        let mut out = [0; sender::KEY_BYTES];
        sender::key(self.source, &self.name, self.sender, &mut out).into()
    }
    /// The sender may notify at all.
    pub fn allowed(&self) -> bool {
        self.policy & sender::ON != 0
    }
    /// It may pop up as a banner.
    pub fn banners(&self) -> bool {
        self.allowed() && self.policy & sender::BANNERS != 0
    }
    /// It is listed in the notification centre.
    pub fn listed(&self) -> bool {
        self.allowed() && self.policy & sender::CENTER != 0
    }
    pub fn priority(&self) -> u32 {
        sender::priority(self.policy)
    }
}
/// One owned notification view record, ready for NOTIFY_VIEW.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub id: u32,
    pub icon: u16,
    pub toast: bool,
    /// Toasted only: its sender keeps it out of the centre.
    pub toast_only: bool,
    pub source: String,
    pub time: String,
    pub title: String,
    pub body: String,
    pub actions: Vec<String>,
}
impl View {
    pub fn record(
        &self,
        generation: u16,
        index: u8,
        count: u8,
        dnd: bool,
        badge: u8,
        focus: s::FocusView,
    ) -> s::NoticeView<'_> {
        let action = |i: usize| {
            self.actions
                .get(i)
                .map_or("", |a| s::clip(a, notice::ACTION_BYTES))
        };
        s::NoticeView {
            generation,
            index,
            count,
            dnd,
            badge,
            id: self.id,
            icon: self.icon,
            toast: self.toast,
            source: s::clip(&self.source, notice::SOURCE_BYTES),
            time: s::clip(&self.time, notice::TIME_BYTES),
            title: s::clip(&self.title, notice::TITLE_BYTES),
            body: s::clip(&self.body, notice::BODY_BYTES),
            actions: [action(0), action(1)],
            toast_only: self.toast_only && self.toast,
            focus,
        }
    }
}
/// Relative time the centre shows ("Now", "5 min ago", "2 h ago").
pub fn time_label(posted: u64, now: u64) -> String {
    let seconds = now.saturating_sub(posted) / 100;
    match seconds {
        0..60 => String::from("Now"),
        60..3600 => format!("{} min ago", seconds / 60),
        _ => format!("{} h ago", seconds / 3600),
    }
}
/// Source name and artwork, from the registration database.
pub fn source_of(db: &Database, n: &Notice) -> (String, u16) {
    let (name, art) = match n.source {
        source::SYSTEM => (String::from("Doors"), icon::SYSTEM),
        source::SHELL => (String::from("Desktop"), icon::SYSTEM),
        code if code & 0xf000 == source::JOB => {
            let name = if n.name.is_empty() {
                String::from("Windows app")
            } else {
                crate::registry::executable_name(&n.name)
            };
            (name, icon::WINDOWS_APP)
        }
        code => match u8::try_from(code).ok().and_then(|c| db.by_command(c)) {
            Some(r) => (r.name.clone(), r.icon),
            None => (String::from("App"), icon::GENERIC_APP),
        },
    };
    let art = if n.icon == icon::GENERIC_APP {
        art
    } else {
        n.icon
    };
    (name, art)
}
/// The display name stored with a sender's registry entry (what Settings
/// lists under "Notifications from apps and other senders").
pub fn sender_name(db: &Database, n: &Notice) -> String {
    match n.source {
        source::SYSTEM => String::from("System"),
        source::SESSION => String::from("Other apps"),
        _ => source_of(db, n).0,
    }
}
#[derive(Default)]
pub struct Presenter {
    /// Toasts showing: (id, hide tick; `u64::MAX` persistent).
    toasts: Vec<(u32, u64)>,
    /// Ids already considered for a toast.
    seen: Vec<u32>,
    started: Option<u64>,
}
impl Presenter {
    /// Fold the service's current notifications in; true when the
    /// presentation changed (republish).
    pub fn update(&mut self, notices: &[Notice], dnd: bool, now: u64) -> bool {
        let started = *self.started.get_or_insert(now);
        let before = self.toasts.clone();
        self.toasts.retain(|(id, until)| {
            *until > now && notices.iter().any(|n| n.id == *id && n.banners())
        });
        if dnd {
            self.toasts.clear();
        }
        let mut fresh: Vec<&Notice> = notices
            .iter()
            .filter(|n| !self.seen.contains(&n.id))
            .collect();
        fresh.sort_by_key(|n| n.id);
        let priority = |id: u32| {
            notices
                .iter()
                .find(|n| n.id == id)
                .map_or(0, Notice::priority)
        };
        for n in fresh {
            self.seen.push(n.id);
            // Already shown before a shell restart (read, or older than a
            // toast lasts at the time this shell started).
            let stale = n.read || n.posted.saturating_add(TOAST_TICKS) < started.min(now);
            if dnd || stale || n.flags & flag::SILENT != 0 || !n.banners() {
                continue;
            }
            let until = if n.flags & flag::PERSISTENT != 0 {
                u64::MAX
            } else {
                now.saturating_add(TOAST_TICKS)
            };
            self.toasts.push((n.id, until));
            if self.toasts.len() > MAX_TOASTS {
                // The oldest of the lowest priority gives way: a top or high
                // toast outlasts normal ones.
                if let Some(victim) = self
                    .toasts
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, (id, _))| (priority(*id), *id))
                    .map(|(i, _)| i)
                {
                    self.toasts.remove(victim);
                }
            }
        }
        self.seen.retain(|id| notices.iter().any(|n| n.id == *id));
        self.toasts != before
    }
    pub fn dismiss(&mut self, id: u32) -> bool {
        let before = self.toasts.len();
        self.toasts.retain(|(t, _)| *t != id);
        self.toasts.len() != before
    }
    pub fn toasting(&self, id: u32) -> bool {
        self.toasts.iter().any(|(t, _)| *t == id)
    }
    /// The next tick at which a toast hides.
    pub fn next_deadline(&self) -> Option<u64> {
        self.toasts
            .iter()
            .map(|(_, until)| *until)
            .filter(|u| *u != u64::MAX)
            .min()
    }
    /// Views in the order the centre lists and toasts stack: top priority
    /// first, then high, then normal, newest first within a level. Senders
    /// turned off are left out; toast-only ones appear while toasting. Every
    /// showing toast fits the presented maximum.
    pub fn views(&self, db: &Database, notices: &[Notice], now: u64) -> Vec<View> {
        let mut ordered: Vec<&Notice> = notices
            .iter()
            .filter(|n| n.listed() || self.toasting(n.id))
            .collect();
        ordered.sort_by(|a, b| b.priority().cmp(&a.priority()).then(b.id.cmp(&a.id)));
        while ordered.len() > notice::MAX {
            match ordered.iter().rposition(|n| !self.toasting(n.id)) {
                Some(i) => {
                    ordered.remove(i);
                }
                None => ordered.truncate(notice::MAX),
            }
        }
        ordered
            .into_iter()
            .map(|n| {
                let (source, icon) = source_of(db, n);
                View {
                    id: n.id,
                    icon,
                    toast: self.toasting(n.id),
                    toast_only: !n.listed(),
                    source,
                    time: time_label(n.posted, now),
                    title: n.title.clone(),
                    body: n.body.clone(),
                    actions: n.actions.clone(),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn notice(id: u32, posted: u64, flags: u16) -> Notice {
        Notice {
            id,
            source: 1,
            icon: 0,
            flags,
            posted,
            read: false,
            name: String::new(),
            title: format!("t{id}"),
            body: String::new(),
            actions: Vec::new(),
            sender: 0,
            policy: sender::DEFAULT,
        }
    }
    fn with(mut n: Notice, policy: u32) -> Notice {
        n.policy = policy;
        n
    }
    const HIGH: u32 = sender::DEFAULT | sender::HIGH << sender::PRIORITY_SHIFT;
    const TOP: u32 = sender::DEFAULT | sender::TOP << sender::PRIORITY_SHIFT;
    #[test]
    fn new_notifications_toast_for_five_seconds_at_most_three() {
        let mut p = Presenter::default();
        assert!(!p.update(&[], false, 1000));
        let list: Vec<Notice> = (1..=4).map(|i| notice(i, 1000, 0)).collect();
        assert!(p.update(&list, false, 1000));
        assert!(!p.toasting(1) && p.toasting(2) && p.toasting(4));
        assert_eq!(p.next_deadline(), Some(1000 + TOAST_TICKS));
        assert!(
            !p.update(&list, false, 1200),
            "nothing new, nothing expired"
        );
        assert!(p.update(&list, false, 1000 + TOAST_TICKS));
        assert!(!p.toasting(4), "hidden after five seconds, still listed");
        let views = p.views(&Database::builtin(), &list, 1000 + TOAST_TICKS);
        assert_eq!(views.len(), 4);
        assert_eq!(views[0].id, 4, "newest first");
        assert_eq!(views[0].source, "File Explorer");
        assert_eq!(views[0].icon, icon::FILES);
    }
    #[test]
    fn dnd_silent_and_restart_suppress_popups() {
        let mut p = Presenter::default();
        let silent = notice(1, 50, flag::SILENT);
        assert!(!p.update(core::slice::from_ref(&silent), false, 50));
        let quiet = notice(2, 60, 0);
        assert!(!p.update(&[silent.clone(), quiet.clone()], true, 60));
        assert!(!p.toasting(2));
        // A restarted shell (new presenter) sees an old and a fresh one.
        let mut restarted = Presenter::default();
        let fresh = notice(3, 5_000, 0);
        restarted.update(&[quiet.clone(), fresh.clone()], false, 5_100);
        assert!(!restarted.toasting(2) && restarted.toasting(3));
        assert!(restarted.dismiss(3));
        assert!(!restarted.toasting(3));
        let persistent = notice(4, 5_200, flag::PERSISTENT);
        restarted.update(&[persistent], false, 5_200);
        assert!(restarted.toasting(4));
        assert_eq!(restarted.next_deadline(), None);
    }
    #[test]
    fn sender_settings_gate_banners_and_the_centre() {
        let db = Database::builtin();
        let mut p = Presenter::default();
        let list = [
            with(notice(1, 0, 0), 0),
            with(notice(2, 0, 0), sender::DEFAULT & !sender::BANNERS),
            with(notice(3, 0, 0), sender::DEFAULT & !sender::CENTER),
            with(notice(4, 0, 0), sender::DEFAULT & !sender::ON),
        ];
        p.update(&list, false, 0);
        // Off: neither. Banners off: listed only. Centre off: toast only.
        assert!(!p.toasting(1) && !p.toasting(2) && p.toasting(3) && !p.toasting(4));
        let views = p.views(&db, &list, 0);
        let ids: Vec<(u32, bool, bool)> = views
            .iter()
            .map(|v| (v.id, v.toast, v.toast_only))
            .collect();
        assert_eq!(ids, [(3, true, true), (2, false, false)]);
        // Once the toast hides, a centre-off notification is gone.
        p.update(&list, false, TOAST_TICKS);
        assert_eq!(p.views(&db, &list, TOAST_TICKS).len(), 1);
        // Settings turned banners off while a toast showed: it hides.
        let mut p = Presenter::default();
        let mut one = [notice(5, 0, 0)];
        p.update(&one, false, 0);
        assert!(p.toasting(5));
        one[0].policy = sender::DEFAULT & !sender::BANNERS;
        assert!(p.update(&one, false, 1));
        assert!(!p.toasting(5));
    }
    #[test]
    fn priority_orders_the_centre_and_survives_the_toast_cap() {
        let db = Database::builtin();
        let mut p = Presenter::default();
        let list = [
            with(notice(1, 0, 0), TOP),
            with(notice(2, 0, 0), HIGH),
            notice(3, 0, 0),
            notice(4, 0, 0),
            with(notice(5, 0, 0), HIGH),
        ];
        p.update(&list, false, 0);
        // Four toasts competed for three places: the normal ones gave way.
        assert!(p.toasting(1) && p.toasting(2) && p.toasting(5));
        assert!(!p.toasting(3) && !p.toasting(4));
        let order: Vec<u32> = p.views(&db, &list, 0).iter().map(|v| v.id).collect();
        assert_eq!(order, [1, 5, 2, 4, 3], "top, high, normal; newest first");
        // A full centre still presents every showing toast.
        let many: Vec<Notice> = (1..=14)
            .map(|i| {
                let n = notice(i, 0, 0);
                if i == 1 {
                    with(n, sender::DEFAULT & !sender::CENTER)
                } else {
                    n
                }
            })
            .collect();
        let mut p = Presenter::default();
        p.update(&many, false, 0);
        let mut q = Presenter::default();
        q.update(&many[..1], false, 0);
        let views = q.views(&db, &many, 0);
        assert_eq!(views.len(), notice::MAX);
        assert!(views.iter().any(|v| v.id == 1 && v.toast_only));
        assert!(p.views(&db, &many, 0).len() <= notice::MAX);
    }
    #[test]
    fn labels_and_sources_come_from_the_registration_database() {
        assert_eq!(time_label(0, 5_900), "Now");
        assert_eq!(time_label(0, 6_000), "1 min ago");
        assert_eq!(time_label(0, 720_000), "2 h ago");
        let db = Database::builtin();
        let mut n = notice(1, 0, 0);
        n.source = source::JOB | 3;
        n.name = String::from("wingui-editor.exe");
        n.sender = 0x0012_abcd;
        assert_eq!(
            source_of(&db, &n),
            (String::from("wingui-editor"), icon::WINDOWS_APP)
        );
        assert_eq!(n.sender_key(), "Exe.wingui-editor.exe.0012abcd");
        assert_eq!(sender_name(&db, &n), "wingui-editor");
        n.source = 11;
        assert_eq!(source_of(&db, &n).0, "Task Manager");
        assert_eq!(
            (n.sender_key().as_str(), sender_name(&db, &n).as_str()),
            ("App.11", "Task Manager")
        );
        n.source = source::SYSTEM;
        n.icon = icon::SETTINGS;
        assert_eq!(source_of(&db, &n), (String::from("Doors"), icon::SETTINGS));
        assert_eq!(
            (n.sender_key().as_str(), sender_name(&db, &n).as_str()),
            ("System", "System")
        );
        let v = View {
            id: 1,
            icon: 0,
            toast: true,
            toast_only: true,
            source: "x".repeat(40),
            time: "Now".into(),
            title: "t".repeat(80),
            body: "b".repeat(200),
            actions: alloc::vec!["Open".into()],
        };
        let mut out = [0; door_service_protocol::BODY_BYTES];
        let focus = s::FocusView {
            active: true,
            length: 30,
            remaining: 100,
            ..s::FocusView::default()
        };
        let n = v.record(1, 0, 1, false, 1, focus).encode(&mut out).unwrap();
        let back = s::NoticeView::decode(&out[..n]).unwrap();
        assert!(back.toast_only && back.focus == focus);
    }
}
