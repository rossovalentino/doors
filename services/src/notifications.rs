//! door.notifications service state (spec §60/§62): a bounded,
//! allocation-free notification centre for one signed-in session, hosted by
//! Init behind its authenticated endpoint. Init decides who is calling
//! (`Caller`) from kernel peer identity; this module never trusts a source
//! named in a request. The desktop shell presents what is retained here, so
//! notifications survive a shell restart.
//!
//! It also owns System > Notifications' session state (P14), with Windows 11
//! semantics:
//! * "Notifications" off: posts are accepted (the poster gets an id) but
//!   nothing is kept, toasted or listed;
//! * Do not disturb: the manual choice is kept in the NOTIFICATIONS word's
//!   `notify::DND` bit, which Init stores for the user (`take_pending`), so it
//!   survives sign-out and restart;
//! * "Turn on do not disturb automatically" > "During these times": at the
//!   schedule's start boundary DND turns on, at its end it turns off; a
//!   manual choice in between holds until the next boundary;
//! * Focus: a timed session (one of `Focus::LENGTHS` minutes) that holds DND
//!   on when `Focus::dnd`, ends with Windows' completion toast, and can be
//!   stopped early (no toast).
use crate::server::Response;
use core::fmt::Write;
use door_service_protocol::{
    BODY_BYTES, Code,
    notifications::{
        self as n, ACTION_BYTES, BODY_TEXT_BYTES, CAPACITY, MAX_ACTIONS, Post, Record,
        SOURCE_BYTES, STATE_IDS, State, Status, TITLE_BYTES, status,
    },
    preferences::{self as prefs, DndSchedule, Focus, notify},
};

/// Notifications one source may keep (a flood cannot evict every other app).
pub const PER_SOURCE: usize = 8;
/// Pending activations waiting for their posters.
pub const ACTIVATIONS: usize = 8;
/// Monotonic ticks per second (10 ms ticks).
pub const TICKS_PER_SECOND: u64 = 100;
/// How often the schedule is evaluated, and the time zone re-read.
pub const SCHEDULE_TICKS: u64 = TICKS_PER_SECOND;
pub const ZONE_TICKS: u64 = 60 * TICKS_PER_SECOND;
/// Windows' Focus completion toast.
pub const FOCUS_DONE_TITLE: &str = "Focus session complete";

#[derive(Clone, Copy)]
struct Text<const N: usize> {
    bytes: [u8; N],
    len: u8,
}
impl<const N: usize> Text<N> {
    const EMPTY: Self = Self {
        bytes: [0; N],
        len: 0,
    };
    fn new(s: &str) -> Self {
        let s = door_service_protocol::shell::clip(s, N);
        let mut t = Self::EMPTY;
        t.bytes[..s.len()].copy_from_slice(s.as_bytes());
        t.len = s.len() as u8;
        t
    }
    fn get(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}
impl<const N: usize> Write for Text<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let at = self.len as usize;
        if at + s.len() > N {
            return Err(core::fmt::Error);
        }
        self.bytes[at..at + s.len()].copy_from_slice(s.as_bytes());
        self.len += s.len() as u8;
        Ok(())
    }
}
#[derive(Clone, Copy)]
struct Notice {
    id: u32,
    source: u32,
    sender: u32,
    pid: u64,
    instance: u64,
    icon: u16,
    flags: u16,
    posted: u64,
    read: bool,
    name: Text<SOURCE_BYTES>,
    title: Text<TITLE_BYTES>,
    body: Text<BODY_TEXT_BYTES>,
    actions: [Text<ACTION_BYTES>; MAX_ACTIONS],
}
/// Who is calling, as Init established it from the kernel identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caller<'a> {
    pub pid: u64,
    pub instance: u64,
    /// `notifications::source` code of the process.
    pub source: u32,
    /// Display name of the source (an executable's file name), if any.
    pub name: &'a str,
    /// `sender::path_hash` of the admitted executable (EXECUTE jobs), else 0.
    pub sender: u32,
    /// The session's current desktop shell host.
    pub shell: bool,
    /// One of the session's workspace apps (Settings among them): it may
    /// read STATUS, toggle Do not disturb, run Focus and apply words.
    pub workspace: bool,
}
impl Caller<'static> {
    /// Init itself (`source::SYSTEM`).
    pub const SYSTEM: Self = Self {
        pid: 1,
        instance: 0,
        source: n::source::SYSTEM,
        name: "",
        sender: 0,
        shell: false,
        workspace: false,
    };
}
#[derive(Clone, Copy)]
struct Activation {
    pid: u64,
    instance: u64,
    id: u32,
    action: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Session {
    ends: u64,
    minutes: u32,
    dnd: bool,
}
/// Preference words changed here (not by Settings) that Init must store for
/// the session user.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pending {
    pub notifications: Option<u32>,
    pub focus: Option<u32>,
}
pub struct Center {
    items: [Option<Notice>; CAPACITY],
    activations: [Option<Activation>; ACTIVATIONS],
    next_id: u32,
    generation: u32,
    /// Do not disturb from the user or the schedule (Focus excluded).
    dnd: bool,
    /// `dnd` was turned on by the schedule.
    by_rule: bool,
    /// Schedule window at the last evaluation (None: take a fresh look).
    inside: Option<bool>,
    evaluate_at: u64,
    notify: u32,
    schedule: DndSchedule,
    focus: Focus,
    session: Option<Session>,
    pending: Pending,
    /// The session user whose words these are (None before binding).
    uid: Option<u32>,
    /// Time zone (`door_locale::tz`) for the schedule, and when Init read it.
    zone: u32,
    zone_at: Option<u64>,
}
impl Default for Center {
    fn default() -> Self {
        Self::new()
    }
}
impl Center {
    pub const fn new() -> Self {
        Self {
            items: [None; CAPACITY],
            activations: [None; ACTIVATIONS],
            next_id: 1,
            generation: 1,
            dnd: false,
            by_rule: false,
            inside: None,
            evaluate_at: 0,
            notify: notify::DEFAULT,
            schedule: DndSchedule::DEFAULT,
            focus: Focus::DEFAULT,
            session: None,
            pending: Pending {
                notifications: None,
                focus: None,
            },
            uid: None,
            zone: 16,
            zone_at: None,
        }
    }
    fn changed(&mut self) {
        self.generation = self.generation.wrapping_add(1).max(1);
    }
    pub fn generation(&self) -> u32 {
        self.generation
    }
    /// Do not disturb in effect: the user's or the schedule's, or a running
    /// Focus session's.
    pub fn dnd(&self) -> bool {
        self.dnd || self.session.is_some_and(|s| s.dnd)
    }
    /// "Notifications" (get notifications from apps and other senders).
    pub fn enabled(&self) -> bool {
        self.notify & notify::ON != 0
    }
    /// The current NOTIFICATIONS word (with the manual DND bit).
    pub fn notifications_word(&self) -> u32 {
        self.notify
    }
    pub fn focus_running(&self) -> bool {
        self.session.is_some()
    }
    pub fn uid(&self) -> Option<u32> {
        self.uid
    }
    pub fn len(&self) -> usize {
        self.items.iter().flatten().count()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The session ended: nothing carries over to the next sign-in (which
    /// binds the user's stored words again).
    pub fn reset(&mut self) {
        let next = self.next_id;
        let generation = self.generation;
        *self = Self::new();
        self.next_id = next;
        self.generation = generation;
        self.changed();
    }
    /// Session binding: the user's stored words (an invalid word reads as
    /// its default). A second binding in the same session (the display
    /// service restarted) keeps the live state.
    pub fn bind(&mut self, uid: u32, notifications: u32, schedule: u32, focus: u32) {
        if self.uid == Some(uid) {
            return;
        }
        self.uid = Some(uid);
        self.notify = if prefs::valid(prefs::NOTIFICATIONS, notifications) {
            notifications
        } else {
            notify::DEFAULT
        };
        self.schedule = DndSchedule::from_bits(schedule).unwrap_or(DndSchedule::DEFAULT);
        self.focus = Focus::from_bits(focus).unwrap_or(Focus::DEFAULT);
        self.dnd = self.notify & notify::DND != 0;
        self.by_rule = false;
        self.inside = None;
        self.session = None;
        self.pending = Pending::default();
        self.changed();
    }
    /// The "During these times" rule is on (local time is needed).
    pub fn scheduled(&self) -> bool {
        self.notify & notify::DND_SCHEDULED != 0
    }
    /// The rule wants a look at the local time now.
    pub fn evaluation_due(&self, now: u64) -> bool {
        self.scheduled() && (self.inside.is_none() || now >= self.evaluate_at)
    }
    /// The time zone needs reading (first use or every `ZONE_TICKS`).
    pub fn zone_due(&self, now: u64) -> bool {
        self.zone_at
            .is_none_or(|at| now >= at.saturating_add(ZONE_TICKS))
    }
    pub fn zone(&self) -> u32 {
        self.zone
    }
    pub fn set_zone(&mut self, zone: u32, now: u64) {
        self.zone = zone;
        self.zone_at = Some(now);
    }
    /// The schedule at local `minute` (0..1440) on `weekday` (0 = Sunday).
    /// Its start boundary turns DND on; its end boundary turns it off (a
    /// manual "on" included, whose stored bit is cleared); a manual choice
    /// in between holds until the next boundary. A fresh look (after
    /// binding or a rule change) inside the window turns DND on.
    pub fn evaluate(&mut self, now: u64, minute: u32, weekday: u32) {
        self.evaluate_at = now.saturating_add(SCHEDULE_TICKS);
        if !self.scheduled() {
            self.inside = None;
            return;
        }
        let inside = self.schedule.active(minute, weekday);
        if self.inside == Some(inside) {
            return;
        }
        if inside {
            if !self.dnd {
                self.dnd = true;
                self.by_rule = true;
                self.changed();
            }
        } else if self.inside.is_some() {
            let was = self.dnd;
            self.dnd = false;
            self.by_rule = false;
            self.store_manual(false);
            if was {
                self.changed();
            }
        }
        self.inside = Some(inside);
    }
    fn store_manual(&mut self, on: bool) {
        let word = if on {
            self.notify | notify::DND
        } else {
            self.notify & !notify::DND
        };
        if word != self.notify {
            self.notify = word;
            self.pending.notifications = Some(word);
        }
    }
    /// SET_DND: the user's choice, stored for the next sign-in. It holds
    /// until the schedule's next boundary; turning it off also releases a
    /// running Focus session's DND.
    pub fn set_dnd(&mut self, on: bool) {
        self.dnd = on;
        self.by_rule = false;
        if !on && let Some(s) = self.session.as_mut() {
            s.dnd = false;
        }
        self.store_manual(on);
        self.changed();
    }
    /// APPLY_PREFERENCE: a word Settings already stored takes effect.
    pub fn apply(&mut self, id: u32, value: u32) -> Result<(), Code> {
        if !prefs::valid(id, value) {
            return Err(Code::InvalidRequest);
        }
        match id {
            prefs::NOTIFICATIONS => {
                let old = core::mem::replace(&mut self.notify, value);
                if (old ^ value) & notify::DND != 0 {
                    self.dnd = value & notify::DND != 0;
                    self.by_rule = false;
                    if !self.dnd
                        && let Some(s) = self.session.as_mut()
                    {
                        s.dnd = false;
                    }
                }
                if (old ^ value) & notify::DND_SCHEDULED != 0 {
                    // The rule switched: a DND it turned on goes; turned on,
                    // it takes a fresh look at the local time.
                    if self.by_rule {
                        self.dnd = false;
                        self.by_rule = false;
                    }
                    self.inside = None;
                }
                // A word written while the store was pending supersedes it.
                self.pending.notifications = None;
            }
            prefs::DND_SCHEDULE => {
                self.schedule = DndSchedule::from_bits(value).ok_or(Code::InvalidRequest)?;
                if self.by_rule {
                    self.dnd = false;
                    self.by_rule = false;
                }
                self.inside = None;
            }
            prefs::FOCUS => {
                self.focus = Focus::from_bits(value).ok_or(Code::InvalidRequest)?;
                if let Some(s) = self.session.as_mut() {
                    s.dnd = self.focus.dnd;
                }
                self.pending.focus = None;
            }
            _ => return Err(Code::InvalidRequest),
        }
        self.changed();
        Ok(())
    }
    /// Start a Focus session (`minutes` 0: the user's length). A running
    /// session restarts with the new length.
    pub fn focus_start(&mut self, minutes: u32, now: u64) -> Result<(), Code> {
        let minutes = if minutes == 0 {
            self.focus.minutes
        } else {
            minutes
        };
        if !Focus::LENGTHS.contains(&minutes) {
            return Err(Code::InvalidRequest);
        }
        self.session = Some(Session {
            ends: now.saturating_add(u64::from(minutes) * 60 * TICKS_PER_SECOND),
            minutes,
            dnd: self.focus.dnd,
        });
        self.changed();
        Ok(())
    }
    /// "Stop focus": true when a session was running.
    pub fn focus_stop(&mut self) -> bool {
        let running = self.session.take().is_some();
        if running {
            self.changed();
        }
        running
    }
    /// The Focus footer's stepper: a new FOCUS length, stored for the user.
    pub fn focus_length(&mut self, minutes: u32) -> Result<(), Code> {
        if !Focus::LENGTHS.contains(&minutes) {
            return Err(Code::InvalidRequest);
        }
        if minutes != self.focus.minutes {
            self.focus.minutes = minutes;
            self.pending.focus = Some(self.focus.bits());
            self.changed();
        }
        Ok(())
    }
    /// Focus timer: a session that reached its end completes (DND it held
    /// is released) with Windows' completion toast. True when one did.
    pub fn tick(&mut self, now: u64) -> bool {
        let Some(s) = self.session else {
            return false;
        };
        if now < s.ends {
            return false;
        }
        self.session = None;
        self.changed();
        let mut body = Text::<BODY_TEXT_BYTES>::EMPTY;
        let _ = write!(
            body,
            "You stayed focused for {} minutes. Time for a break.",
            s.minutes
        );
        let post = Post {
            icon: door_service_protocol::shell::icon::SYSTEM,
            flags: 0,
            title: FOCUS_DONE_TITLE,
            body: body.get(),
            actions: ["", ""],
        };
        self.post(Caller::SYSTEM, &post, now);
        true
    }
    /// STATUS: what Settings and the Focus footer show.
    pub fn status(&self, now: u64) -> Status {
        let mut flags = 0;
        for (on, flag) in [
            (self.dnd(), status::DND),
            (self.dnd && self.by_rule, status::SCHEDULED),
            (self.session.is_some(), status::FOCUS),
            (self.session.is_some_and(|s| s.dnd), status::FOCUS_DND),
            (self.enabled(), status::ENABLED),
            (self.focus.hide_badges, status::HIDE_BADGES),
            (self.focus.hide_flashing, status::HIDE_FLASHING),
        ] {
            if on {
                flags |= flag;
            }
        }
        let (minutes, remaining) = self.session.map_or((0, 0), |s| {
            let left = s.ends.saturating_sub(now).div_ceil(TICKS_PER_SECOND);
            (s.minutes, left.min(u64::from(s.minutes) * 60) as u32)
        });
        Status {
            generation: self.generation,
            flags,
            focus_length: self.focus.minutes as u16,
            focus_minutes: minutes as u16,
            focus_remaining: remaining,
        }
    }
    /// Words Init must store now (cleared; `restore_pending` on failure).
    pub fn take_pending(&mut self) -> Pending {
        core::mem::take(&mut self.pending)
    }
    /// The store failed: keep the words for the next attempt (newer ones
    /// taken meanwhile win).
    pub fn restore_pending(&mut self, p: Pending) {
        self.pending.notifications = self.pending.notifications.or(p.notifications);
        self.pending.focus = self.pending.focus.or(p.focus);
    }
    fn oldest(&self, source: Option<u32>) -> Option<usize> {
        self.items
            .iter()
            .enumerate()
            .filter_map(|(i, n)| n.map(|n| (i, n)))
            .filter(|(_, n)| source.is_none_or(|s| n.source == s))
            .min_by_key(|(_, n)| n.id)
            .map(|(i, _)| i)
    }
    /// Record `post` from `caller`; returns its id. The source's oldest is
    /// dropped beyond `PER_SOURCE`, the centre's oldest beyond `CAPACITY`.
    /// With "Notifications" off the id is issued but nothing is kept.
    pub fn post(&mut self, caller: Caller<'_>, post: &Post<'_>, now: u64) -> u32 {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        if !self.enabled() {
            return id;
        }
        let same = self
            .items
            .iter()
            .flatten()
            .filter(|n| n.source == caller.source)
            .count();
        let slot = if same >= PER_SOURCE {
            self.oldest(Some(caller.source))
        } else {
            self.items.iter().position(Option::is_none)
        }
        .or_else(|| self.oldest(None))
        .unwrap_or(0);
        self.items[slot] = Some(Notice {
            id,
            source: caller.source,
            sender: caller.sender,
            pid: caller.pid,
            instance: caller.instance,
            icon: post.icon,
            flags: post.flags,
            posted: now,
            read: false,
            name: Text::new(caller.name),
            title: Text::new(post.title),
            body: Text::new(post.body),
            actions: [Text::new(post.actions[0]), Text::new(post.actions[1])],
        });
        self.changed();
        id
    }
    fn find(&self, id: u32) -> Option<usize> {
        self.items
            .iter()
            .position(|n| n.is_some_and(|n| n.id == id && id != 0))
    }
    /// The poster (same process incarnation) removes its own notification.
    pub fn withdraw(&mut self, caller: Caller<'_>, id: u32) -> Result<(), Code> {
        let i = self.find(id).ok_or(Code::NotFound)?;
        let n = self.items[i].unwrap();
        if (n.pid, n.instance) != (caller.pid, caller.instance) {
            return Err(Code::Denied);
        }
        self.items[i] = None;
        self.changed();
        Ok(())
    }
    pub fn state(&self) -> State {
        let mut ordered: [Option<Notice>; CAPACITY] = self.items;
        ordered.sort_unstable_by(|a, b| b.map(|n| n.id).cmp(&a.map(|n| n.id)));
        let mut s = State {
            generation: self.generation,
            dnd: self.dnd(),
            ..State::default()
        };
        for n in ordered.iter().flatten().take(STATE_IDS) {
            s.ids[s.count as usize] = n.id;
            s.count += 1;
            if !n.read {
                s.unread += 1;
            }
        }
        s
    }
    pub fn record(&self, id: u32) -> Option<Record<'_>> {
        let n = self.items[self.find(id)?].as_ref()?;
        Some(Record {
            id: n.id,
            source: n.source,
            icon: n.icon,
            flags: n.flags,
            posted: n.posted,
            read: n.read,
            name: n.name.get(),
            sender: n.sender,
            title: n.title.get(),
            body: n.body.get(),
            actions: [n.actions[0].get(), n.actions[1].get()],
        })
    }
    /// Dismiss one notification, or every one (`id` 0).
    pub fn clear(&mut self, id: u32) -> Result<(), Code> {
        if id == 0 {
            self.items = [None; CAPACITY];
        } else {
            let i = self.find(id).ok_or(Code::NotFound)?;
            self.items[i] = None;
        }
        self.changed();
        Ok(())
    }
    pub fn mark_read(&mut self) {
        let mut any = false;
        for n in self.items.iter_mut().flatten() {
            any |= !n.read;
            n.read = true;
        }
        if any {
            self.changed();
        }
    }
    /// The user chose the body (`action` 0) or a button (1..=actions). The
    /// activation waits for the poster; the notification leaves the centre.
    pub fn activate(&mut self, id: u32, action: u32) -> Result<(), Code> {
        let i = self.find(id).ok_or(Code::NotFound)?;
        let n = self.items[i].unwrap();
        let buttons = n.actions.iter().filter(|a| a.len != 0).count() as u32;
        if action > buttons {
            return Err(Code::InvalidRequest);
        }
        let slot = self
            .activations
            .iter()
            .position(Option::is_none)
            .unwrap_or(0);
        self.activations[slot] = Some(Activation {
            pid: n.pid,
            instance: n.instance,
            id,
            action,
        });
        self.items[i] = None;
        self.changed();
        Ok(())
    }
    pub fn take_activation(&mut self, caller: Caller<'_>) -> Option<(u32, u32)> {
        let i = self.activations.iter().position(|a| {
            a.is_some_and(|a| (a.pid, a.instance) == (caller.pid, caller.instance))
        })?;
        let a = self.activations[i].take()?;
        Some((a.id, a.action))
    }
    /// Decode, authorize and answer one door.notifications request. A Focus
    /// session that reached its end completes first.
    pub fn request(&mut self, caller: Caller<'_>, op: u32, body: &[u8], now: u64) -> Response {
        self.tick(now);
        match self.answer(caller, op, body, now) {
            Ok(r) => r,
            Err(code) => Response::error(code),
        }
    }
    fn answer(
        &mut self,
        caller: Caller<'_>,
        op: u32,
        body: &[u8],
        now: u64,
    ) -> Result<Response, Code> {
        let word = |at: usize| -> Result<u32, Code> {
            body.get(at..at + 4)
                .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                .ok_or(Code::InvalidRequest)
        };
        let shell_only = matches!(
            op,
            n::STATE | n::GET | n::CLEAR | n::ACTIVATE | n::MARK_READ
        );
        let managing = matches!(
            op,
            n::SET_DND
                | n::STATUS
                | n::FOCUS_START
                | n::FOCUS_STOP
                | n::FOCUS_LENGTH
                | n::APPLY_PREFERENCE
        );
        if (shell_only && !caller.shell) || (managing && !caller.shell && !caller.workspace) {
            return Err(Code::Denied);
        }
        let mut out = [0; BODY_BYTES];
        match op {
            n::POST => {
                let post = Post::decode(body)?;
                let id = self.post(caller, &post, now);
                Ok(Response::bytes(&id.to_le_bytes()))
            }
            n::WITHDRAW if body.len() == 4 => {
                self.withdraw(caller, word(0)?)?;
                Ok(Response::empty())
            }
            n::STATE if body.is_empty() => {
                let len = self.state().encode(&mut out)?;
                Ok(Response::bytes(&out[..len]))
            }
            n::GET if body.len() == 4 => {
                let record = self.record(word(0)?).ok_or(Code::NotFound)?;
                let len = record.encode(&mut out)?;
                Ok(Response::bytes(&out[..len]))
            }
            n::CLEAR if body.len() == 4 => {
                self.clear(word(0)?)?;
                Ok(Response::empty())
            }
            n::SET_DND if body.len() == 4 && word(0)? <= 1 => {
                self.set_dnd(word(0)? == 1);
                Ok(Response::empty())
            }
            n::ACTIVATE if body.len() == 8 => {
                self.activate(word(0)?, word(4)?)?;
                Ok(Response::empty())
            }
            n::TAKE_ACTIVATION if body.is_empty() => Ok(match self.take_activation(caller) {
                Some((id, action)) => {
                    out[..4].copy_from_slice(&id.to_le_bytes());
                    out[4..8].copy_from_slice(&action.to_le_bytes());
                    Response::bytes(&out[..8])
                }
                None => Response::empty(),
            }),
            n::MARK_READ if body.is_empty() => {
                self.mark_read();
                Ok(Response::empty())
            }
            n::STATUS if body.is_empty() => Ok(Response::bytes(&self.status(now).encode())),
            n::FOCUS_START if body.len() == 4 => {
                self.focus_start(word(0)?, now)?;
                Ok(Response::empty())
            }
            n::FOCUS_STOP if body.is_empty() => {
                self.focus_stop();
                Ok(Response::empty())
            }
            n::FOCUS_LENGTH if body.len() == 4 => {
                self.focus_length(word(0)?)?;
                Ok(Response::empty())
            }
            n::APPLY_PREFERENCE if body.len() == 8 => {
                self.apply(word(0)?, word(4)?)?;
                Ok(Response::empty())
            }
            _ => Err(Code::InvalidRequest),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const APP: Caller = Caller {
        pid: 20,
        instance: 7,
        source: 1,
        name: "",
        sender: 0,
        shell: false,
        workspace: false,
    };
    const SHELL: Caller = Caller {
        pid: 9,
        instance: 3,
        source: n::source::SHELL,
        name: "",
        sender: 0,
        shell: true,
        workspace: false,
    };
    const SETTINGS: Caller = Caller {
        pid: 30,
        instance: 4,
        source: 3,
        name: "",
        sender: 0,
        shell: false,
        workspace: true,
    };
    fn post(title: &str) -> Post<'_> {
        Post {
            icon: 0,
            flags: 0,
            title,
            body: "body",
            actions: ["Open", ""],
        }
    }
    fn status(c: &mut Center, who: Caller<'_>, now: u64) -> Status {
        let r = c.request(who, n::STATUS, &[], now);
        assert_eq!(r.code, Code::Success);
        Status::decode(&r.body[..r.len]).unwrap()
    }
    #[test]
    fn posts_are_attributed_bounded_and_listed_newest_first() {
        let mut c = Center::new();
        let a = c.post(APP, &post("one"), 10);
        let b = c.post(
            Caller {
                pid: 21,
                source: n::source::JOB | 2,
                name: "wingui-editor",
                sender: 0xabcd,
                ..APP
            },
            &post("two"),
            11,
        );
        let s = c.state();
        assert_eq!((s.count, s.unread, &s.ids[..2]), (2, 2, &[b, a][..]));
        let r = c.record(b).unwrap();
        assert_eq!(
            (r.source, r.name, r.title, r.sender),
            (0x1002, "wingui-editor", "two", 0xabcd)
        );
        // One source cannot evict everyone: it recycles its own oldest.
        for i in 0..20 {
            c.post(APP, &post("flood"), 20 + i);
        }
        assert!(c.record(b).is_some());
        assert!(c.record(a).is_none());
        assert_eq!(
            c.items
                .iter()
                .flatten()
                .filter(|n| n.source == APP.source)
                .count(),
            PER_SOURCE
        );
        let g = c.generation();
        c.mark_read();
        assert_eq!(c.state().unread, 0);
        assert_ne!(c.generation(), g);
    }
    #[test]
    fn only_the_poster_withdraws_and_only_the_shell_manages() {
        let mut c = Center::new();
        let id = c.post(APP, &post("one"), 1);
        let other = Caller { instance: 8, ..APP };
        assert_eq!(c.withdraw(other, id), Err(Code::Denied));
        for op in [n::STATE, n::MARK_READ, n::STATUS, n::FOCUS_STOP] {
            assert_eq!(c.request(APP, op, &[], 0).code, Code::Denied);
        }
        assert_eq!(
            c.request(APP, n::CLEAR, &id.to_le_bytes(), 0).code,
            Code::Denied
        );
        assert_eq!(
            c.request(APP, n::SET_DND, &1u32.to_le_bytes(), 0).code,
            Code::Denied
        );
        // Settings (a workspace app) manages DND and Focus but never lists,
        // clears or activates other apps' notifications.
        for op in [n::STATE, n::MARK_READ] {
            assert_eq!(c.request(SETTINGS, op, &[], 0).code, Code::Denied);
        }
        assert_eq!(
            c.request(SETTINGS, n::GET, &id.to_le_bytes(), 0).code,
            Code::Denied
        );
        let r = c.request(SHELL, n::STATE, &[], 0);
        assert_eq!(r.code, Code::Success);
        assert_eq!(State::decode(&r.body[..r.len]).unwrap().ids[0], id);
        assert_eq!(c.withdraw(APP, id), Ok(()));
        assert_eq!(c.withdraw(APP, id), Err(Code::NotFound));
    }
    #[test]
    fn activations_reach_only_the_posting_incarnation() {
        let mut c = Center::new();
        let id = c.post(APP, &post("one"), 1);
        let mut body = [0; 8];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4..].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(
            c.request(SHELL, n::ACTIVATE, &body, 0).code,
            Code::InvalidRequest
        );
        body[4..].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(c.request(SHELL, n::ACTIVATE, &body, 0).code, Code::Success);
        assert!(c.record(id).is_none());
        let restarted = Caller {
            instance: 99,
            ..APP
        };
        assert_eq!(c.take_activation(restarted), None);
        let r = c.request(APP, n::TAKE_ACTIVATION, &[], 0);
        assert_eq!(&r.body[..r.len], &body);
        assert_eq!(c.take_activation(APP), None);
    }
    #[test]
    fn clear_all_dnd_and_reset_change_the_generation() {
        let mut c = Center::new();
        c.post(APP, &post("one"), 1);
        c.post(APP, &post("two"), 2);
        let g = c.generation();
        assert_eq!(
            c.request(SHELL, n::SET_DND, &1u32.to_le_bytes(), 0).code,
            Code::Success
        );
        assert!(c.dnd() && c.generation() != g);
        assert_eq!(
            c.request(SHELL, n::SET_DND, &2u32.to_le_bytes(), 0).code,
            Code::InvalidRequest
        );
        c.clear(0).unwrap();
        assert!(c.is_empty());
        let next = c.post(APP, &post("three"), 3);
        c.reset();
        assert!(c.is_empty() && !c.dnd());
        assert!(c.post(APP, &post("four"), 4) > next);
    }
    #[test]
    fn notifications_off_accepts_posts_but_keeps_nothing() {
        let mut c = Center::new();
        assert!(c.enabled(), "default on, as Windows");
        c.bind(1000, notify::SOUNDS, DndSchedule::DEFAULT.bits(), 0);
        assert!(!c.enabled());
        let mut out = [0; BODY_BYTES];
        let len = post("hidden").encode(&mut out).unwrap();
        let r = c.request(APP, n::POST, &out[..len], 1);
        assert_eq!(r.code, Code::Success, "accepted");
        let id = u32::from_le_bytes(r.body[..4].try_into().unwrap());
        assert!(id != 0 && c.record(id).is_none() && c.is_empty());
        // Init's own notices obey the switch too.
        c.post(Caller::SYSTEM, &post("system"), 2);
        assert!(c.is_empty());
        // Settings turns it back on: posts are kept again.
        let mut word = [0; 8];
        word[..4].copy_from_slice(&prefs::NOTIFICATIONS.to_le_bytes());
        word[4..].copy_from_slice(&notify::DEFAULT.to_le_bytes());
        assert_eq!(
            c.request(SETTINGS, n::APPLY_PREFERENCE, &word, 3).code,
            Code::Success
        );
        assert!(c.enabled() && status(&mut c, SETTINGS, 3).has(status::ENABLED));
        let id = c.post(APP, &post("shown"), 4);
        assert!(c.record(id).is_some());
        // Invalid words and ids are refused.
        word[4..].copy_from_slice(&0x80u32.to_le_bytes());
        assert_eq!(
            c.request(SETTINGS, n::APPLY_PREFERENCE, &word, 5).code,
            Code::InvalidRequest
        );
        word[..4].copy_from_slice(&prefs::CLIPBOARD.to_le_bytes());
        word[4..].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            c.request(SETTINGS, n::APPLY_PREFERENCE, &word, 5).code,
            Code::InvalidRequest
        );
    }
    #[test]
    fn manual_dnd_is_stored_and_restored_at_the_next_binding() {
        let mut c = Center::new();
        c.bind(
            1000,
            notify::DEFAULT,
            DndSchedule::DEFAULT.bits(),
            Focus::DEFAULT.bits(),
        );
        assert!(!c.dnd() && c.take_pending() == Pending::default());
        // Settings toggles it on: Init must store the word.
        assert_eq!(
            c.request(SETTINGS, n::SET_DND, &1u32.to_le_bytes(), 1).code,
            Code::Success
        );
        assert!(c.dnd());
        let stored = c.take_pending().notifications.unwrap();
        assert_eq!(stored, notify::DEFAULT | notify::DND);
        assert!(c.take_pending().notifications.is_none(), "taken once");
        // A failed store is retried; a newer word wins.
        c.restore_pending(Pending {
            notifications: Some(stored),
            focus: None,
        });
        assert_eq!(c.take_pending().notifications, Some(stored));
        // Sign-out and sign-in again: the stored word brings DND back.
        c.reset();
        assert!(!c.dnd() && c.uid().is_none());
        c.bind(
            1000,
            stored,
            DndSchedule::DEFAULT.bits(),
            Focus::DEFAULT.bits(),
        );
        assert!(c.dnd() && c.state().dnd);
        // A display-service restart re-binds the same user: state is kept.
        c.set_dnd(false);
        c.bind(1000, stored, 0, 0);
        assert!(!c.dnd());
        // Invalid stored words read as the defaults.
        c.reset();
        c.bind(1000, u32::MAX, u32::MAX, u32::MAX);
        let s = c.status(0);
        assert!(s.has(status::ENABLED) && !s.has(status::DND));
        assert_eq!(s.focus_length, 30);
    }
    #[test]
    fn schedule_turns_dnd_on_and_off_at_its_boundaries_and_manual_wins_between() {
        let mut c = Center::new();
        let rule = notify::DEFAULT | notify::DND_SCHEDULED;
        c.bind(1000, rule, DndSchedule::DEFAULT.bits(), 0);
        let wed = 3;
        assert!(c.evaluation_due(0));
        c.evaluate(0, 12 * 60, wed);
        assert!(!c.dnd() && !c.evaluation_due(1) && c.evaluation_due(SCHEDULE_TICKS));
        // 22:00: the window starts.
        c.evaluate(100, 22 * 60, wed);
        let s = c.status(100);
        assert!(s.has(status::DND) && s.has(status::SCHEDULED));
        assert_eq!(c.take_pending(), Pending::default(), "not stored");
        // The user turns it off at 23:00: it stays off inside the window.
        c.set_dnd(false);
        c.evaluate(200, 23 * 60, wed);
        c.evaluate(300, 60, wed + 1);
        assert!(!c.dnd());
        // 07:00 ends the window (still off); 22:00 next day turns it on.
        c.evaluate(400, 7 * 60, wed + 1);
        assert!(!c.dnd());
        c.evaluate(500, 22 * 60, wed + 1);
        assert!(c.dnd());
        // A manual "on" outside the window holds until the next end.
        c.evaluate(600, 7 * 60 + 1, wed + 2);
        assert!(!c.dnd());
        let _ = c.take_pending();
        c.set_dnd(true);
        assert_eq!(c.take_pending().notifications, Some(rule | notify::DND));
        c.evaluate(700, 21 * 60, wed + 2);
        c.evaluate(800, 23 * 60, wed + 2);
        assert!(c.dnd() && !c.status(800).has(status::SCHEDULED));
        c.evaluate(900, 7 * 60, wed + 3);
        assert!(!c.dnd());
        assert_eq!(c.take_pending().notifications, Some(rule), "bit cleared");
        // Binding inside the window turns it on (a fresh look).
        c.reset();
        c.bind(1000, rule, DndSchedule::DEFAULT.bits(), 0);
        c.evaluate(0, 23 * 60, wed);
        assert!(c.dnd());
        // Turning the rule off releases the DND it set.
        let mut word = [0; 8];
        word[..4].copy_from_slice(&prefs::NOTIFICATIONS.to_le_bytes());
        word[4..].copy_from_slice(&notify::DEFAULT.to_le_bytes());
        assert_eq!(
            c.request(SETTINGS, n::APPLY_PREFERENCE, &word, 1).code,
            Code::Success
        );
        assert!(!c.dnd() && !c.evaluation_due(2));
        // New times: a fresh look against them (weekends only, Wednesday).
        word[4..].copy_from_slice(&rule.to_le_bytes());
        c.request(SETTINGS, n::APPLY_PREFERENCE, &word, 2);
        let weekends = DndSchedule {
            repeat: 2,
            ..DndSchedule::DEFAULT
        };
        word[..4].copy_from_slice(&prefs::DND_SCHEDULE.to_le_bytes());
        word[4..].copy_from_slice(&weekends.bits().to_le_bytes());
        assert_eq!(
            c.request(SETTINGS, n::APPLY_PREFERENCE, &word, 3).code,
            Code::Success
        );
        assert!(c.evaluation_due(3));
        c.evaluate(3, 23 * 60, wed);
        assert!(!c.dnd());
        c.evaluate(103, 23 * 60, 6);
        assert!(c.dnd());
    }
    #[test]
    fn focus_runs_holds_dnd_completes_with_a_toast_and_stops_early() {
        let mut c = Center::new();
        c.bind(
            1000,
            notify::DEFAULT,
            DndSchedule::DEFAULT.bits(),
            Focus::DEFAULT.bits(),
        );
        // Bad lengths are refused; 0 means the user's 30 minutes.
        assert_eq!(
            c.request(SHELL, n::FOCUS_START, &25u32.to_le_bytes(), 0)
                .code,
            Code::InvalidRequest
        );
        assert_eq!(
            c.request(SHELL, n::FOCUS_START, &0u32.to_le_bytes(), 0)
                .code,
            Code::Success
        );
        let s = status(&mut c, SETTINGS, 0);
        assert!(s.has(status::FOCUS) && s.has(status::FOCUS_DND) && s.has(status::DND));
        assert!(!s.has(status::SCHEDULED));
        assert_eq!((s.focus_minutes, s.focus_remaining), (30, 1800));
        assert!(c.state().dnd, "the shell sees DND during the session");
        let s = status(&mut c, SETTINGS, 61 * TICKS_PER_SECOND);
        assert_eq!(s.focus_remaining, 1800 - 61);
        // The session ends: DND is released and the toast is posted.
        let end = 1800 * TICKS_PER_SECOND;
        let before = c.len();
        let s = status(&mut c, SHELL, end);
        assert!(!s.has(status::FOCUS) && !s.has(status::DND));
        assert_eq!((s.focus_minutes, s.focus_remaining), (0, 0));
        assert_eq!(c.len(), before + 1);
        let id = c.state().ids[0];
        let r = c.record(id).unwrap();
        assert_eq!((r.source, r.title), (n::source::SYSTEM, FOCUS_DONE_TITLE));
        assert!(r.body.contains("30 minutes"));
        // Stop early: no toast.
        c.focus_start(5, end).unwrap();
        assert_eq!(
            c.request(SETTINGS, n::FOCUS_STOP, &[], end + 1).code,
            Code::Success
        );
        assert!(!c.focus_running() && !c.dnd());
        assert!(!c.tick(end + 5 * 60 * TICKS_PER_SECOND));
        assert_eq!(c.len(), before + 1);
        // Turning DND off during a session releases the session's DND.
        c.focus_start(10, end).unwrap();
        c.set_dnd(false);
        let s = c.status(end + 1);
        assert!(s.has(status::FOCUS) && !s.has(status::DND));
        // Focus without DND (Settings unticked it) leaves DND alone.
        let quiet = Focus {
            dnd: false,
            ..Focus::DEFAULT
        };
        c.apply(prefs::FOCUS, quiet.bits()).unwrap();
        c.focus_start(0, end).unwrap();
        assert!(!c.dnd() && c.focus_running());
    }
    #[test]
    fn the_footer_stepper_stores_the_focus_length() {
        let mut c = Center::new();
        c.bind(1000, notify::DEFAULT, 0, Focus::DEFAULT.bits());
        assert_eq!(
            c.request(SHELL, n::FOCUS_LENGTH, &45u32.to_le_bytes(), 0)
                .code,
            Code::Success
        );
        let word = c.take_pending().focus.unwrap();
        assert_eq!(Focus::from_bits(word).unwrap().minutes, 45);
        assert_eq!(c.status(0).focus_length, 45);
        // Unchanged: nothing to store; invalid: refused.
        c.focus_length(45).unwrap();
        assert_eq!(c.take_pending(), Pending::default());
        assert_eq!(c.focus_length(7), Err(Code::InvalidRequest));
        c.focus_start(0, 0).unwrap();
        assert_eq!(c.status(0).focus_minutes, 45);
    }
}
