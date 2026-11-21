//! Window-server side of door.shell (`door_service_protocol::shell`): the
//! restartable desktop shell process's role, its bounded event queue, the
//! taskbar groups / search catalog / notification view it publishes, and the
//! per-window taskbar state (attention, progress) applications set.
//!
//! GUI-H: nothing here calls the shell. The shell polls; publications are
//! staged and swapped in whole; window changes are queued (coalesced, and a
//! full queue becomes one RESYNC so the shell re-enumerates). GUI-F: when the
//! shell is gone the server keeps every window and derives fallback taskbar
//! buttons from its own window list (`extras` with no published model).
extern crate alloc;
use alloc::{collections::VecDeque, string::String, vec::Vec};
use door_graphics_core::{Owner, WindowId};
use door_service_protocol::{Code, shell as s};

/// Queued shell events before the queue collapses into RESYNC.
pub const QUEUE: usize = 64;
/// Key space of server fallback groups (one per owning process).
pub const FALLBACK_KEY: u32 = 0x8000_0000;

/// Taskbar state an application set for one of its windows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowState {
    pub attention: bool,
    pub progress_state: u8,
    pub progress: u16,
}
impl WindowState {
    pub fn progress(self) -> Option<(u8, u16)> {
        (self.progress_state != s::progress::NONE).then_some((self.progress_state, self.progress))
    }
}
/// A published taskbar group or catalog entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub key: u32,
    pub icon: u16,
    pub flags: u16,
    pub progress_state: u8,
    pub progress: u16,
    pub windows: u8,
    pub ids: Vec<u64>,
    pub label: String,
}
impl Group {
    fn from_entry(e: &s::Entry<'_>) -> Self {
        Self {
            key: e.key,
            icon: e.icon,
            flags: e.flags,
            progress_state: e.progress_state,
            progress: e.progress,
            windows: e.windows,
            ids: e.ids.iter().copied().filter(|id| *id != 0).collect(),
            label: e.label.into(),
        }
    }
}
/// One notification as presented (toast and/or centre row).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub id: u32,
    pub icon: u16,
    pub toast: bool,
    pub source: String,
    pub time: String,
    pub title: String,
    pub body: String,
    pub actions: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Notices {
    pub list: Vec<Notice>,
    pub dnd: bool,
    pub badge: u8,
}
impl Notices {
    pub fn toasts(&self) -> impl Iterator<Item = &Notice> {
        self.list.iter().filter(|n| n.toast)
    }
}
#[derive(Default)]
struct Staged<T> {
    generation: u16,
    items: Vec<T>,
}
#[derive(Default)]
pub struct Link {
    holder: Option<Owner>,
    events: VecDeque<s::Event>,
    overflow: bool,
    /// Window-list generation reported to the shell.
    generation: u32,
    taskbar: Option<Vec<Group>>,
    catalog: Vec<Group>,
    notices: Notices,
    staged_taskbar: Option<Staged<Group>>,
    staged_catalog: Option<Staged<Group>>,
    staged_notices: Option<(Staged<Notice>, bool, u8)>,
    states: Vec<(u64, WindowState)>,
    /// Incarnations granted so far (diagnostics).
    pub grants: u32,
}
impl Link {
    pub fn holder(&self) -> Option<Owner> {
        self.holder
    }
    pub fn connected(&self) -> bool {
        self.holder.is_some()
    }
    pub fn generation(&self) -> u32 {
        self.generation
    }
    /// Give the shell role to `owner` (a new incarnation replaces the old).
    /// Its first POLL answers RESYNC: it enumerates before trusting events.
    pub fn grant(&mut self, owner: Owner) -> Option<Owner> {
        let previous = self.holder.replace(owner).filter(|p| *p != owner);
        self.events.clear();
        self.overflow = true;
        self.staged_taskbar = None;
        self.staged_catalog = None;
        self.staged_notices = None;
        self.grants = self.grants.saturating_add(1);
        previous
    }
    /// The role holder disconnected or was revoked: published shell state is
    /// withdrawn (the server falls back to its own window list). Returns
    /// whether `owner` held the role.
    pub fn revoke(&mut self, owner: Owner) -> bool {
        if self.holder != Some(owner) {
            return false;
        }
        self.holder = None;
        self.events.clear();
        self.overflow = false;
        self.taskbar = None;
        self.catalog.clear();
        self.notices = Notices::default();
        self.staged_taskbar = None;
        self.staged_catalog = None;
        self.staged_notices = None;
        true
    }
    fn push(&mut self, e: s::Event) {
        if self.holder.is_none() || self.overflow {
            return;
        }
        // CHANGED coalesces with a queued CHANGED/ADDED of the same window.
        if e.kind == s::event::CHANGED
            && self.events.iter().any(|q| {
                q.window == e.window && matches!(q.kind, s::event::CHANGED | s::event::ADDED)
            })
        {
            return;
        }
        if self.events.len() >= QUEUE {
            self.events.clear();
            self.overflow = true;
            return;
        }
        self.events.push_back(e);
    }
    /// A window was added, changed, removed or focused.
    pub fn note(&mut self, kind: u32, window: u64) {
        self.generation = self.generation.wrapping_add(1);
        if kind == s::event::REMOVED {
            self.forget(window);
        }
        self.push(s::Event {
            kind,
            window,
            ..s::Event::default()
        });
    }
    /// The user acted on a shell-published surface. False without a shell.
    pub fn invoke(&mut self, command: u32, window: u64, a: u32, b: u32) -> bool {
        if self.holder.is_none() {
            return false;
        }
        // An invocation must never be lost to coalescing; a full queue still
        // resyncs (the user repeats the click).
        self.push(s::Event {
            kind: s::event::INVOKE,
            command,
            window,
            a,
            b,
        });
        !self.overflow
    }
    pub fn poll(&mut self, owner: Owner) -> Result<Option<s::Event>, Code> {
        if self.holder != Some(owner) {
            return Err(Code::Denied);
        }
        if self.overflow {
            self.overflow = false;
            return Ok(Some(s::Event {
                kind: s::event::RESYNC,
                ..s::Event::default()
            }));
        }
        Ok(self.events.pop_front())
    }
    pub fn pending(&self) -> bool {
        self.overflow || !self.events.is_empty()
    }
    /// Stage one published entry; true when a whole model was swapped in.
    /// `live` says whether a window id is an ordinary session window now.
    pub fn publish(
        &mut self,
        owner: Owner,
        e: &s::Entry<'_>,
        live: impl Fn(u64) -> bool,
    ) -> Result<bool, Code> {
        if self.holder != Some(owner) {
            return Err(Code::Denied);
        }
        if e.ids.iter().any(|id| *id != 0 && !live(*id)) {
            // A window closed between enumeration and publication: the shell
            // receives REMOVED and republishes.
            return Err(Code::NotFound);
        }
        let staged = match e.kind {
            s::kind::TASKBAR => &mut self.staged_taskbar,
            _ => &mut self.staged_catalog,
        };
        if e.count == 0 {
            *staged = None;
            match e.kind {
                s::kind::TASKBAR => self.taskbar = Some(Vec::new()),
                _ => self.catalog.clear(),
            }
            return Ok(true);
        }
        if e.index == 0 {
            *staged = Some(Staged {
                generation: e.generation,
                items: Vec::new(),
            });
        }
        let Some(stage) = staged.as_mut() else {
            return Err(Code::InvalidRequest);
        };
        if stage.generation != e.generation
            || stage.items.len() != e.index as usize
            || stage.items.iter().any(|g| g.key == e.key)
        {
            *staged = None;
            return Err(Code::InvalidRequest);
        }
        stage.items.push(Group::from_entry(e));
        if e.index + 1 < e.count {
            return Ok(false);
        }
        let items = staged.take().unwrap().items;
        match e.kind {
            s::kind::TASKBAR => self.taskbar = Some(items),
            _ => self.catalog = items,
        }
        Ok(true)
    }
    /// Stage one notification view record; true when the view was swapped.
    pub fn publish_notice(&mut self, owner: Owner, v: &s::NoticeView<'_>) -> Result<bool, Code> {
        if self.holder != Some(owner) {
            return Err(Code::Denied);
        }
        if v.count == 0 {
            self.staged_notices = None;
            self.notices = Notices {
                list: Vec::new(),
                dnd: v.dnd,
                badge: 0,
            };
            return Ok(true);
        }
        if v.index == 0 {
            self.staged_notices = Some((
                Staged {
                    generation: v.generation,
                    items: Vec::new(),
                },
                v.dnd,
                v.badge,
            ));
        }
        let Some((stage, dnd, badge)) = self.staged_notices.as_mut() else {
            return Err(Code::InvalidRequest);
        };
        if stage.generation != v.generation
            || stage.items.len() != v.index as usize
            || *dnd != v.dnd
            || *badge != v.badge
            || stage.items.iter().any(|n| n.id == v.id)
        {
            self.staged_notices = None;
            return Err(Code::InvalidRequest);
        }
        stage.items.push(Notice {
            id: v.id,
            icon: v.icon,
            toast: v.toast,
            source: v.source.into(),
            time: v.time.into(),
            title: v.title.into(),
            body: v.body.into(),
            actions: v
                .actions
                .iter()
                .filter(|a| !a.is_empty())
                .map(|a| String::from(*a))
                .collect(),
        });
        if v.index + 1 < v.count {
            return Ok(false);
        }
        let (stage, dnd, badge) = self.staged_notices.take().unwrap();
        self.notices = Notices {
            list: stage.items,
            dnd,
            badge,
        };
        Ok(true)
    }
    /// The published taskbar model (None: no shell model; use fallback).
    pub fn taskbar(&self) -> Option<&[Group]> {
        self.taskbar.as_deref()
    }
    pub fn catalog(&self) -> &[Group] {
        &self.catalog
    }
    pub fn notices(&self) -> &Notices {
        &self.notices
    }
    /// SET_TASKBAR_STATE from the window's owner (already authorized).
    /// Returns whether anything visible changed.
    pub fn set_window_state(&mut self, s: s::TaskbarState, active: bool) -> bool {
        let i = match self.states.iter().position(|(id, _)| *id == s.window) {
            Some(i) => i,
            None => {
                self.states.push((s.window, WindowState::default()));
                self.states.len() - 1
            }
        };
        let old = self.states[i].1;
        let mut new = old;
        match s.attention {
            // Windows ignores a flash request for the active window.
            s::attention::REQUEST => new.attention = !active,
            s::attention::STOP => new.attention = false,
            _ => (),
        }
        new.progress_state = s.progress_state;
        new.progress = if s.progress_state == s::progress::NONE {
            0
        } else {
            s.progress
        };
        self.states[i].1 = new;
        if new == WindowState::default() {
            self.states.remove(i);
        }
        if new != old {
            self.note(s::event::CHANGED, s.window);
        }
        new != old
    }
    pub fn window_state(&self, id: u64) -> WindowState {
        self.states
            .iter()
            .find(|(w, _)| *w == id)
            .map_or_else(WindowState::default, |(_, s)| *s)
    }
    /// The window became active: its attention request is satisfied.
    pub fn activated(&mut self, id: u64) -> bool {
        let Some(i) = self.states.iter().position(|(w, _)| *w == id) else {
            return false;
        };
        if !self.states[i].1.attention {
            return false;
        }
        self.states[i].1.attention = false;
        if self.states[i].1 == WindowState::default() {
            self.states.remove(i);
        }
        self.note(s::event::CHANGED, id);
        true
    }
    pub fn forget(&mut self, id: u64) {
        self.states.retain(|(w, _)| *w != id);
    }
    /// Some window carries attention or progress.
    pub fn has_states(&self) -> bool {
        !self.states.is_empty()
    }
}

/// A non-catalog top-level window the taskbar may show (active desktop,
/// not owned by another window).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub id: WindowId,
    pub owner: Owner,
    pub active: bool,
    pub title: String,
    pub state: WindowState,
}
/// A taskbar button after the catalog apps' own buttons.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extra {
    pub key: u32,
    pub icon: u16,
    pub label: String,
    /// Live windows (taskbar order); empty for a pinned launcher.
    pub ids: Vec<WindowId>,
    pub active: bool,
    pub attention: bool,
    pub pinned: bool,
    pub progress: Option<(u8, u16)>,
}
impl Extra {
    pub fn windows(&self) -> usize {
        self.ids.len()
    }
}
fn fallback(c: &Candidate) -> Extra {
    Extra {
        key: FALLBACK_KEY | (c.owner.pid as u32 & 0x7fff_ffff),
        icon: s::icon::WINDOWS_APP,
        label: c.title.clone(),
        ids: Vec::new(),
        active: false,
        attention: false,
        pinned: false,
        progress: None,
    }
}
/// The extra taskbar buttons: the shell's published groups (their windows
/// re-validated against live state), then any live window no group covers
/// yet grouped per owning process. Without a published model every
/// candidate is grouped per process (GUI-F fallback).
pub fn extras(published: Option<&[Group]>, candidates: &[Candidate]) -> Vec<Extra> {
    let mut out: Vec<Extra> = Vec::new();
    let mut covered: Vec<WindowId> = Vec::new();
    for g in published.unwrap_or(&[]) {
        let live: Vec<&Candidate> = g
            .ids
            .iter()
            .filter_map(|id| candidates.iter().find(|c| c.id.0 == *id))
            .collect();
        let pinned = g.flags & s::entry::PINNED != 0;
        if live.is_empty() && !pinned {
            continue;
        }
        covered.extend(live.iter().map(|c| c.id));
        let progress = (g.progress_state != s::progress::NONE)
            .then_some((g.progress_state, g.progress))
            .or_else(|| live.iter().find_map(|c| c.state.progress()));
        out.push(Extra {
            key: g.key,
            icon: g.icon,
            label: g.label.clone(),
            ids: live.iter().map(|c| c.id).collect(),
            active: live.iter().any(|c| c.active),
            attention: live.iter().any(|c| c.state.attention)
                || (!live.is_empty() && g.flags & s::entry::ATTENTION != 0),
            pinned,
            progress,
        });
    }
    for c in candidates.iter().filter(|c| !covered.contains(&c.id)) {
        let key = fallback(c).key;
        let i = match out.iter().position(|e| e.key == key) {
            Some(i) => i,
            None => {
                out.push(fallback(c));
                out.len() - 1
            }
        };
        let e = &mut out[i];
        e.ids.push(c.id);
        e.active |= c.active;
        e.attention |= c.state.attention;
        if e.progress.is_none() {
            e.progress = c.state.progress();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    const SHELL: Owner = Owner {
        pid: 9,
        session: 1,
        epoch: 4,
    };
    fn candidate(id: u64, pid: u64, title: &str) -> Candidate {
        Candidate {
            id: WindowId(id),
            owner: Owner {
                pid,
                session: 1,
                epoch: 1,
            },
            active: false,
            title: title.into(),
            state: WindowState::default(),
        }
    }
    fn entry(kind: u8, index: u8, count: u8, key: u32, ids: [u64; 4]) -> s::Entry<'static> {
        s::Entry {
            kind,
            generation: 1,
            index,
            count,
            key,
            icon: s::icon::WINDOWS_APP,
            flags: s::entry::RUNNING,
            progress_state: 0,
            progress: 0,
            windows: ids.iter().filter(|i| **i != 0).count() as u8,
            ids,
            label: "Editor",
        }
    }
    #[test]
    fn events_coalesce_and_overflow_into_one_resync() {
        let mut l = Link::default();
        l.note(s::event::ADDED, 1);
        assert!(!l.pending(), "no shell: nothing queued");
        assert_eq!(l.grant(SHELL), None);
        assert_eq!(l.poll(SHELL).unwrap().unwrap().kind, s::event::RESYNC);
        assert_eq!(l.poll(SHELL).unwrap(), None);
        l.note(s::event::ADDED, 1);
        l.note(s::event::CHANGED, 1);
        l.note(s::event::CHANGED, 1);
        assert!(l.invoke(s::command::LAUNCH, 0, 7, 0));
        let kinds: Vec<u32> = core::iter::from_fn(|| l.poll(SHELL).unwrap())
            .map(|e| e.kind)
            .collect();
        assert_eq!(kinds, [s::event::ADDED, s::event::INVOKE]);
        for i in 0..=QUEUE as u64 {
            l.note(s::event::REMOVED, 100 + i);
        }
        assert_eq!(l.poll(SHELL).unwrap().unwrap().kind, s::event::RESYNC);
        assert_eq!(l.poll(SHELL).unwrap(), None);
        let stranger = Owner { pid: 10, ..SHELL };
        assert_eq!(l.poll(stranger), Err(Code::Denied));
    }
    #[test]
    fn publication_is_staged_and_swapped_whole_and_revoked_with_the_shell() {
        let mut l = Link::default();
        l.grant(SHELL);
        let live = |id: u64| matches!(id, 11 | 12);
        assert_eq!(
            l.publish(
                SHELL,
                &entry(s::kind::TASKBAR, 0, 2, 5, [11, 0, 0, 0]),
                live
            ),
            Ok(false)
        );
        assert!(l.taskbar().is_none(), "half a model is never visible");
        // Out-of-order index abandons the staging.
        assert_eq!(
            l.publish(
                SHELL,
                &entry(s::kind::TASKBAR, 0, 2, 5, [11, 0, 0, 0]),
                live
            ),
            Ok(false)
        );
        assert_eq!(
            l.publish(
                SHELL,
                &entry(s::kind::TASKBAR, 1, 2, 6, [12, 0, 0, 0]),
                live
            ),
            Ok(true)
        );
        assert_eq!(l.taskbar().unwrap().len(), 2);
        assert_eq!(
            l.publish(
                SHELL,
                &entry(s::kind::TASKBAR, 0, 1, 7, [99, 0, 0, 0]),
                live
            ),
            Err(Code::NotFound)
        );
        let other = Owner { pid: 1, ..SHELL };
        assert_eq!(
            l.publish(other, &s::Entry::clear(s::kind::TASKBAR, 2), live),
            Err(Code::Denied)
        );
        assert!(!l.revoke(other));
        assert!(l.revoke(SHELL));
        assert!(l.taskbar().is_none() && !l.connected());
    }
    #[test]
    fn attention_clears_on_activation_and_progress_round_trips() {
        let mut l = Link::default();
        let set = |attention, progress_state, progress| s::TaskbarState {
            window: 3,
            attention,
            progress_state,
            progress,
        };
        assert!(l.set_window_state(set(s::attention::REQUEST, 1, 400), false));
        assert_eq!(
            l.window_state(3),
            WindowState {
                attention: true,
                progress_state: 1,
                progress: 400
            }
        );
        assert!(l.activated(3));
        assert!(!l.window_state(3).attention);
        assert!(!l.set_window_state(set(s::attention::REQUEST, 1, 400), true));
        assert!(l.set_window_state(set(s::attention::KEEP, 0, 999), false));
        assert_eq!(l.window_state(3), WindowState::default());
        assert!(l.states.is_empty());
    }
    #[test]
    fn extras_follow_published_groups_and_fall_back_per_process() {
        let mut a = candidate(11, 40, "Editor - one.txt");
        a.active = true;
        let mut b = candidate(12, 40, "Editor - two.txt");
        b.state.attention = true;
        let c = candidate(13, 41, "Self test");
        let fallback = extras(None, &[a.clone(), b.clone(), c.clone()]);
        assert_eq!(fallback.len(), 2);
        assert_eq!(fallback[0].ids, [WindowId(11), WindowId(12)]);
        assert!(fallback[0].active && fallback[0].attention);
        assert_eq!(fallback[1].label, "Self test");
        let groups = [
            Group {
                key: 0x2000_0001,
                icon: s::icon::WINDOWS_APP,
                flags: s::entry::PINNED,
                progress_state: 0,
                progress: 0,
                windows: 0,
                ids: Vec::new(),
                label: "Pinned app".into(),
            },
            Group {
                key: 0x2000_0002,
                icon: s::icon::WINDOWS_APP,
                flags: s::entry::RUNNING,
                progress_state: s::progress::NORMAL,
                progress: 250,
                windows: 3,
                ids: alloc::vec![12, 11, 77],
                label: "Editor".into(),
            },
            Group {
                key: 0x2000_0003,
                icon: s::icon::WINDOWS_APP,
                flags: s::entry::RUNNING,
                progress_state: 0,
                progress: 0,
                windows: 1,
                ids: alloc::vec![78],
                label: "Gone".into(),
            },
        ];
        let shown = extras(Some(&groups), &[a, b, c]);
        let keys: Vec<u32> = shown.iter().map(|e| e.key).collect();
        assert_eq!(keys, [0x2000_0001, 0x2000_0002, FALLBACK_KEY | 41]);
        assert!(shown[0].pinned && shown[0].ids.is_empty());
        assert_eq!(shown[1].ids, [WindowId(12), WindowId(11)]);
        assert_eq!(shown[1].progress, Some((s::progress::NORMAL, 250)));
        assert!(shown[1].attention && shown[1].active);
    }
    #[test]
    fn notice_views_swap_whole_and_reject_inconsistent_headers() {
        let mut l = Link::default();
        l.grant(SHELL);
        let v = |index, id, badge| s::NoticeView {
            generation: 1,
            index,
            count: 2,
            dnd: false,
            badge,
            id,
            icon: 0,
            toast: index == 0,
            source: "Settings",
            time: "Now",
            title: "Title",
            body: "",
            actions: ["Open", ""],
            toast_only: false,
            focus: s::FocusView::default(),
        };
        assert_eq!(l.publish_notice(SHELL, &v(0, 4, 1)), Ok(false));
        assert!(l.notices().list.is_empty());
        assert_eq!(
            l.publish_notice(SHELL, &v(1, 5, 2)),
            Err(Code::InvalidRequest)
        );
        assert_eq!(l.publish_notice(SHELL, &v(0, 4, 1)), Ok(false));
        assert_eq!(l.publish_notice(SHELL, &v(1, 5, 1)), Ok(true));
        assert_eq!(l.notices().toasts().count(), 1);
        assert_eq!(l.notices().list[0].actions, ["Open"]);
        assert_eq!(
            l.publish_notice(SHELL, &s::NoticeView::empty(2, true)),
            Ok(true)
        );
        assert!(l.notices().dnd && l.notices().list.is_empty());
    }
}
