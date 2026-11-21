//! Drag and drop broker (door.dnd v1, `native-ui-v1.md`): the window
//! server's half of OLE DoDragDrop/IDropTarget. One drag at a time, owned
//! by the trusted pointer: targets learn only which offered formats they
//! accept while the pointer is over them, and only the target that received
//! the drop may read, and only those formats. Offers are copied at START
//! (`clipboard::Broker::take_drag`), so a source that exits mid-drop cannot
//! stall the target.
extern crate alloc;
use crate::clipboard::{self, Content, Reply};
use alloc::vec::Vec;
use door_graphics_core::Owner;
use door_service_protocol::{
    Code, clipboard as cb,
    dnd::{self as d, resolve},
    window::{Event, Rect},
};
/// Ticks a drag rests on a taskbar button before that app's window comes
/// forward (the Windows mouse hover time, 400 ms).
pub const TASKBAR_HOVER_TICKS: u64 = 400 / door_abi::platform::MONOTONIC_TICK_MILLIS;
/// A dropped offer stays readable this long without FINISH.
pub const DROP_TICKS: u64 = 30_000 / door_abi::platform::MONOTONIC_TICK_MILLIS;
const MAX_TARGETS: usize = 64;
const MAX_EVENTS: usize = 16;
const MAX_DROPPED: usize = 4;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Target {
    owner: Owner,
    window: u64,
    formats: u32,
    effects: u32,
    default: u32,
}
/// What the pointer is over, as the server hit-tests it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hover {
    /// Shell UI, the desktop, or nothing that can take a drop.
    Nothing,
    /// A window's content, at content-local physical coordinates.
    Window {
        owner: Owner,
        window: u64,
        x: i32,
        y: i32,
    },
    /// A taskbar app button (its slot index): never a drop target, but
    /// resting there brings the app's window forward.
    Taskbar(u32),
}
/// A drop description insert ("Documents" in "Copy to Documents").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Insert {
    bytes: [u8; d::MAX_DESCRIPTION],
    len: u8,
}
impl Insert {
    pub fn new(s: &str) -> Option<Self> {
        if s.is_empty() || !d::valid_description(s) {
            return None;
        }
        let mut bytes = [0u8; d::MAX_DESCRIPTION];
        bytes[..s.len()].copy_from_slice(s.as_bytes());
        Some(Self {
            bytes,
            len: s.len() as u8,
        })
    }
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("")
    }
}
#[derive(Clone, Copy, Debug)]
struct Over {
    target: Target,
    /// The target's latest drop description insert.
    insert: Option<Insert>,
    /// Offered formats the target accepts.
    formats: u32,
    /// The target's latest STATUS (or its registration).
    effects: u32,
    default: u32,
    x: i32,
    y: i32,
}
struct Drag {
    serial: u32,
    source: Owner,
    source_window: u64,
    allowed: u32,
    offer: Content,
    over: Option<Over>,
    keys: u32,
    effect: u32,
    /// Taskbar slot, since when, and whether it already activated.
    taskbar: Option<(u32, u64, bool)>,
}
struct Dropped {
    serial: u32,
    source: Owner,
    source_window: u64,
    target: Owner,
    formats: u32,
    effect: u32,
    offer: Content,
    deadline: u64,
}
/// Cursor feedback for the shell to paint at the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Feedback {
    /// Resolved effect (NONE: "can't drop here").
    pub effect: u32,
    /// Resting on a taskbar button.
    pub taskbar: bool,
    /// What the target says is under the pointer ("Copy to %1").
    pub insert: Option<Insert>,
}
/// How a drag ended at release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Released {
    Dropped { serial: u32, effect: u32 },
    Refused,
    Idle,
}
#[derive(Default)]
pub struct Broker {
    /// Clients that negotiated CAP_DRAG_DROP (only they get drag events).
    clients: Vec<Owner>,
    targets: Vec<Target>,
    drag: Option<Drag>,
    dropped: Vec<Dropped>,
    serial: u32,
    events: Vec<(Owner, Event)>,
}
/// Server checks for START, from trusted input state.
pub trait StartEnv {
    fn locked(&self) -> bool;
    fn owns(&self, owner: Owner, window: u64) -> bool;
    /// The primary button is held with the pointer over (or captured by)
    /// the source window: a drag starts from the user's own gesture.
    fn gesture(&self, owner: Owner, window: u64) -> bool;
}
fn word(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn long(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
fn effect_valid(effect: u32) -> bool {
    effect == d::NONE || effect & !d::EFFECTS == 0 && effect.is_power_of_two()
}
impl Broker {
    pub fn new() -> Self {
        Self::default()
    }
    /// A client negotiated CAP_DRAG_DROP.
    pub fn grant(&mut self, owner: Owner) {
        if !self.clients.contains(&owner) {
            self.clients.push(owner);
        }
    }
    pub fn active(&self) -> bool {
        self.drag.is_some()
    }
    /// Bytes of offers held (counted in the clipboard broker's budget).
    pub fn held_bytes(&self) -> usize {
        self.drag.iter().map(|d| d.offer.bytes()).sum::<usize>()
            + self.dropped.iter().map(|d| d.offer.bytes()).sum::<usize>()
    }
    pub fn feedback(&self) -> Option<Feedback> {
        self.drag.as_ref().map(|d| Feedback {
            effect: d.effect,
            taskbar: d.taskbar.is_some(),
            insert: d
                .over
                .and_then(|o| o.insert)
                .filter(|_| d.effect != d::NONE),
        })
    }
    fn push(&mut self, owner: Owner, event: Event) {
        // Motion coalesces: one pending OVER per target and serial.
        if event.kind == d::OVER
            && let Some(slot) = self.events.iter_mut().find(|(o, e)| {
                *o == owner
                    && e.kind == d::OVER
                    && e.value == event.value
                    && e.window == event.window
            })
        {
            slot.1 = event;
            return;
        }
        if self.events.iter().filter(|(o, _)| *o == owner).count() >= MAX_EVENTS {
            // A consumer that stopped polling loses its oldest motion.
            if let Some(i) = self
                .events
                .iter()
                .position(|(o, e)| *o == owner && e.kind == d::OVER)
            {
                self.events.remove(i);
            } else {
                return;
            }
        }
        self.events.push((owner, event));
    }
    pub fn take_event(&mut self, owner: Owner) -> Option<Event> {
        let i = self.events.iter().position(|(o, _)| *o == owner)?;
        Some(self.events.remove(i).1)
    }
    fn target_event(kind: u32, drag: &Drag, over: &Over) -> Event {
        Event {
            kind,
            detail: drag.keys & (d::KEY_CONTROL | d::KEY_SHIFT | d::KEY_ALT),
            window: over.target.window,
            value: u64::from(drag.serial),
            rect: Rect {
                x: over.x,
                y: over.y,
                width: 0,
                height: 0,
            },
            a: over.formats,
            b: if kind == d::DROP {
                drag.effect
            } else {
                drag.allowed
            },
            timestamp: 0,
        }
    }
    fn finished(&mut self, source: Owner, window: u64, serial: u32, effect: u32, outcome: u32) {
        self.push(
            source,
            Event {
                kind: d::FINISHED,
                window,
                value: u64::from(serial),
                a: effect,
                b: outcome,
                ..Event::default()
            },
        );
    }
    fn recompute(drag: &mut Drag) {
        drag.effect = match &drag.over {
            Some(o) => resolve(drag.allowed, o.effects, o.default, drag.keys),
            None => d::NONE,
        };
    }
    /// Dispatch one drag-and-drop opcode.
    pub fn request(
        &mut self,
        env: &dyn StartEnv,
        clip: &mut clipboard::Broker,
        clip_env: &dyn clipboard::Env,
        owner: Owner,
        op: u32,
        b: &[u8],
    ) -> Result<Reply, Code> {
        if env.locked() {
            return Err(Code::Denied);
        }
        if !self.clients.contains(&owner) {
            return Err(Code::Unsupported);
        }
        match op {
            d::REGISTER if b.len() == 20 => {
                let (window, formats, effects, default) =
                    (long(b, 0), word(b, 8), word(b, 12), word(b, 16));
                if !env.owns(owner, window) {
                    return Err(Code::Denied);
                }
                if formats == 0
                    || formats & !cb::ALL_FORMATS != 0
                    || effects == 0
                    || effects & !d::EFFECTS != 0
                {
                    return Err(Code::InvalidRequest);
                }
                if !effect_valid(default) || default & !effects != 0 {
                    return Err(Code::InvalidRequest);
                }
                let target = Target {
                    owner,
                    window,
                    formats,
                    effects,
                    default,
                };
                match self.targets.iter().position(|t| t.window == window) {
                    Some(i) => self.targets[i] = target,
                    None if self.targets.len() < MAX_TARGETS => self.targets.push(target),
                    None => return Err(Code::Resources),
                }
                Ok(Reply::Empty)
            }
            d::REVOKE if b.len() == 8 => {
                let window = long(b, 0);
                if !env.owns(owner, window) {
                    return Err(Code::Denied);
                }
                self.targets.retain(|t| t.window != window);
                self.leave_if(|o| o.target.window == window);
                Ok(Reply::Empty)
            }
            d::BEGIN if b.len() == 12 => {
                let effects = word(b, 8);
                if effects == 0 || effects & !d::EFFECTS != 0 {
                    return Err(Code::InvalidRequest);
                }
                clip.begin(
                    clip_env,
                    owner,
                    long(b, 0),
                    clipboard::Purpose::Drag { effects },
                )?;
                Ok(Reply::Empty)
            }
            d::START if b.is_empty() => {
                if self.drag.is_some() {
                    return Err(Code::WouldBlock);
                }
                let offer = clip.take_drag(owner)?;
                if !env.owns(owner, offer.window) || !env.gesture(owner, offer.window) {
                    return Err(Code::Denied);
                }
                self.serial = self.serial.wrapping_add(1).max(1);
                self.drag = Some(Drag {
                    serial: self.serial,
                    source: owner,
                    source_window: offer.window,
                    allowed: offer.effects,
                    offer: offer.content,
                    over: None,
                    keys: 0,
                    effect: d::NONE,
                    taskbar: None,
                });
                Ok(Reply::Bytes(self.serial.to_le_bytes().to_vec()))
            }
            d::STATUS if (20..=20 + d::MAX_DESCRIPTION).contains(&b.len()) => {
                let (serial, window, effects, default) =
                    (word(b, 0), long(b, 4), word(b, 12), word(b, 16));
                if effects & !d::EFFECTS != 0 || !effect_valid(default) || default & !effects != 0 {
                    return Err(Code::InvalidRequest);
                }
                let insert = match core::str::from_utf8(&b[20..]) {
                    Ok(s) if d::valid_description(s) => Insert::new(s),
                    _ => return Err(Code::InvalidRequest),
                };
                let drag = self
                    .drag
                    .as_mut()
                    .filter(|d| d.serial == serial)
                    .ok_or(Code::Changed)?;
                let over = drag
                    .over
                    .as_mut()
                    .filter(|o| o.target.owner == owner && o.target.window == window)
                    .ok_or(Code::Changed)?;
                // A target can narrow, never widen, what it registered.
                over.effects = effects & over.target.effects;
                over.default = default & over.effects;
                over.insert = insert;
                Self::recompute(drag);
                Ok(Reply::Empty)
            }
            d::READ if b.len() == 12 => {
                let (serial, format, offset) = (word(b, 0), word(b, 4), word(b, 8) as usize);
                let dropped = self
                    .dropped
                    .iter()
                    .find(|x| x.serial == serial && x.target == owner)
                    .ok_or(Code::NotFound)?;
                // Scoped offer: only formats this target registered for.
                if dropped.formats & cb::mask(format) == 0 {
                    return Err(Code::Denied);
                }
                clipboard::read_chunk(dropped.offer.get(format).ok_or(Code::NotFound)?, offset)
            }
            d::FINISH if b.len() == 8 => {
                let (serial, effect) = (word(b, 0), word(b, 4));
                let i = self
                    .dropped
                    .iter()
                    .position(|x| x.serial == serial && x.target == owner)
                    .ok_or(Code::NotFound)?;
                if !effect_valid(effect) || effect & !self.dropped[i].effect != 0 {
                    return Err(Code::InvalidRequest);
                }
                let x = self.dropped.remove(i);
                self.finished(
                    x.source,
                    x.source_window,
                    x.serial,
                    effect,
                    d::OUTCOME_DROPPED,
                );
                Ok(Reply::Empty)
            }
            d::CANCEL if b.len() == 4 => {
                if !self
                    .drag
                    .as_ref()
                    .is_some_and(|x| x.serial == word(b, 0) && x.source == owner)
                {
                    return Err(Code::NotFound);
                }
                self.cancel();
                Ok(Reply::Empty)
            }
            d::FIRST..=d::LAST => Err(Code::InvalidRequest),
            _ => Err(Code::Unsupported),
        }
    }
    fn leave_if(&mut self, gone: impl Fn(&Over) -> bool) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        if let Some(over) = drag.over.filter(|o| gone(o)) {
            drag.over = None;
            Self::recompute(drag);
            let e = Self::target_event(d::LEAVE, drag, &over);
            self.push(over.target.owner, e);
        }
    }
    /// The pointer moved (or keys changed) during a drag.
    pub fn motion(&mut self, hover: Hover, keys: u32, now: u64) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        let keys = keys & (d::KEY_CONTROL | d::KEY_SHIFT | d::KEY_ALT);
        let keys_changed = keys != drag.keys;
        drag.keys = keys;
        drag.taskbar = match (hover, drag.taskbar) {
            (Hover::Taskbar(slot), Some(t)) if t.0 == slot => Some(t),
            (Hover::Taskbar(slot), _) => Some((slot, now, false)),
            _ => None,
        };
        let target = match hover {
            Hover::Window {
                owner,
                window,
                x,
                y,
            } => self
                .targets
                .iter()
                .find(|t| {
                    t.owner == owner && t.window == window && t.formats & drag.offer.mask() != 0
                })
                .map(|t| (*t, x, y)),
            _ => None,
        };
        let mut out: Vec<(Owner, Event)> = Vec::new();
        match (drag.over, target) {
            (Some(mut over), Some((t, x, y)))
                if over.target.window == t.window && over.target.owner == t.owner =>
            {
                if (over.x, over.y) != (x, y) || keys_changed {
                    over.x = x;
                    over.y = y;
                    drag.over = Some(over);
                    out.push((t.owner, Self::target_event(d::OVER, drag, &over)));
                }
            }
            (old, new) => {
                if let Some(old) = old {
                    out.push((old.target.owner, Self::target_event(d::LEAVE, drag, &old)));
                }
                drag.over = new.map(|(t, x, y)| Over {
                    target: t,
                    insert: None,
                    formats: t.formats & drag.offer.mask(),
                    effects: t.effects,
                    default: t.default,
                    x,
                    y,
                });
                if let Some(over) = drag.over {
                    out.push((over.target.owner, Self::target_event(d::ENTER, drag, &over)));
                }
            }
        }
        Self::recompute(drag);
        for (owner, e) in out {
            self.push(owner, e);
        }
    }
    /// Time passes: a drag resting on a taskbar button activates that app
    /// once (returns its slot); unfinished drops expire.
    pub fn tick(&mut self, now: u64) -> Option<u32> {
        while let Some(i) = self.dropped.iter().position(|x| now >= x.deadline) {
            let x = self.dropped.remove(i);
            self.finished(
                x.source,
                x.source_window,
                x.serial,
                d::NONE,
                d::OUTCOME_LOST,
            );
        }
        let drag = self.drag.as_mut()?;
        let (slot, since, fired) = drag.taskbar?;
        if fired || now < since + TASKBAR_HOVER_TICKS {
            return None;
        }
        drag.taskbar = Some((slot, since, true));
        Some(slot)
    }
    /// The primary button was released.
    pub fn release(&mut self, now: u64) -> Released {
        let Some(drag) = self.drag.take() else {
            return Released::Idle;
        };
        match drag.over {
            Some(over) if drag.effect != d::NONE => {
                let e = Self::target_event(d::DROP, &drag, &over);
                self.push(over.target.owner, e);
                if self.dropped.len() >= MAX_DROPPED {
                    let x = self.dropped.remove(0);
                    self.finished(
                        x.source,
                        x.source_window,
                        x.serial,
                        d::NONE,
                        d::OUTCOME_LOST,
                    );
                }
                self.dropped.push(Dropped {
                    serial: drag.serial,
                    source: drag.source,
                    source_window: drag.source_window,
                    target: over.target.owner,
                    formats: over.formats,
                    effect: drag.effect,
                    offer: drag.offer,
                    deadline: now + DROP_TICKS,
                });
                Released::Dropped {
                    serial: drag.serial,
                    effect: drag.effect,
                }
            }
            over => {
                if let Some(over) = over {
                    let e = Self::target_event(d::LEAVE, &drag, &over);
                    self.push(over.target.owner, e);
                }
                self.finished(
                    drag.source,
                    drag.source_window,
                    drag.serial,
                    d::NONE,
                    d::OUTCOME_REFUSED,
                );
                Released::Refused
            }
        }
    }
    /// Esc, the source's CANCEL, a lock or an input reset.
    pub fn cancel(&mut self) {
        let Some(drag) = self.drag.take() else { return };
        if let Some(over) = drag.over {
            let e = Self::target_event(d::LEAVE, &drag, &over);
            self.push(over.target.owner, e);
        }
        self.finished(
            drag.source,
            drag.source_window,
            drag.serial,
            d::NONE,
            d::OUTCOME_CANCELLED,
        );
    }
    /// A window went away: it no longer takes drops.
    pub fn window_gone(&mut self, window: u64) {
        self.targets.retain(|t| t.window != window);
        self.leave_if(|o| o.target.window == window);
        if self
            .drag
            .as_ref()
            .is_some_and(|x| x.source_window == window)
        {
            self.cancel();
        }
    }
    pub fn owner_exited(&mut self, owner: Owner) {
        self.clients.retain(|o| *o != owner);
        self.targets.retain(|t| t.owner != owner);
        self.events.retain(|(o, _)| *o != owner);
        if self.drag.as_ref().is_some_and(|x| x.source == owner) {
            self.cancel();
            self.events.retain(|(o, _)| *o != owner);
        } else if let Some(drag) = self.drag.as_mut()
            && drag.over.is_some_and(|o| o.target.owner == owner)
        {
            drag.over = None;
            Self::recompute(drag);
        }
        // A target that exits before FINISH loses the drop; a source that
        // exits leaves its copied offer readable until FINISH or expiry.
        while let Some(i) = self.dropped.iter().position(|x| x.target == owner) {
            let x = self.dropped.remove(i);
            self.finished(
                x.source,
                x.source_window,
                x.serial,
                d::NONE,
                d::OUTCOME_LOST,
            );
        }
        self.events.retain(|(o, _)| *o != owner);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::tests::{A, B, SETTINGS as C, TestEnv, body, stage, window};
    struct Start {
        locked: bool,
        gesture: bool,
    }
    impl StartEnv for Start {
        fn locked(&self) -> bool {
            self.locked
        }
        fn owns(&self, owner: Owner, window: u64) -> bool {
            <TestEnv as clipboard::Env>::owns(&TestEnv::default(), owner, window)
        }
        fn gesture(&self, _: Owner, _: u64) -> bool {
            self.gesture
        }
    }
    struct World {
        dnd: Broker,
        clip: clipboard::Broker,
        env: TestEnv,
        start: Start,
    }
    impl World {
        fn new() -> Self {
            let mut dnd = Broker::new();
            for o in [A, B, C] {
                dnd.grant(o);
            }
            Self {
                dnd,
                clip: clipboard::tests::broker(),
                env: TestEnv::default(),
                start: Start {
                    locked: false,
                    gesture: true,
                },
            }
        }
        fn call(&mut self, owner: Owner, op: u32, b: &[u8]) -> Result<Reply, Code> {
            self.dnd
                .request(&self.start, &mut self.clip, &self.env, owner, op, b)
        }
        fn register(
            &mut self,
            owner: Owner,
            window: u64,
            formats: u32,
            effects: u32,
            default: u32,
        ) {
            let mut b = window_bytes(window);
            b.extend(body(&[formats as u64, effects as u64, default as u64]));
            self.call(owner, d::REGISTER, &b).unwrap();
        }
        /// A (window 1) drags text and HTML allowing copy and move.
        fn start(&mut self, formats: &[(u32, &[u8])], effects: u32) -> Result<u32, Code> {
            let mut b = window(1);
            b.extend(body(&[effects as u64]));
            self.call(A, d::BEGIN, &b)?;
            stage(&mut self.clip, &mut self.env, A, formats)?;
            match self.call(A, d::START, &[])? {
                Reply::Bytes(b) => Ok(word(&b, 0)),
                Reply::Empty => Err(Code::Io),
            }
        }
        fn events(&mut self, owner: Owner) -> Vec<Event> {
            core::iter::from_fn(|| self.dnd.take_event(owner)).collect()
        }
        fn read(&mut self, owner: Owner, serial: u32, format: u32) -> Result<Vec<u8>, Code> {
            let mut out = Vec::new();
            loop {
                match self.call(
                    owner,
                    d::READ,
                    &body(&[serial as u64, format as u64, out.len() as u64]),
                )? {
                    Reply::Bytes(b) if b.is_empty() => return Ok(out),
                    Reply::Bytes(b) => out.extend_from_slice(&b),
                    Reply::Empty => return Err(Code::Io),
                }
            }
        }
    }
    fn window_bytes(w: u64) -> Vec<u8> {
        w.to_le_bytes().to_vec()
    }
    fn over(owner: Owner, window: u64, x: i32, y: i32) -> Hover {
        Hover::Window {
            owner,
            window,
            x,
            y,
        }
    }
    #[test]
    fn formats_are_negotiated_and_a_target_only_reads_what_it_accepts() {
        let mut w = World::new();
        // B's window 101 takes text (copy/move, move by default); C's
        // window 201 takes only files.
        w.register(
            B,
            101,
            cb::mask(cb::TEXT) | cb::mask(cb::FILES),
            d::COPY | d::MOVE,
            d::MOVE,
        );
        w.register(C, 201, cb::mask(cb::FILES), d::COPY, d::COPY);
        let serial = w
            .start(
                &[(cb::TEXT, b"dragged words"), (cb::HTML, b"<i>dragged</i>")],
                d::COPY | d::MOVE,
            )
            .unwrap();
        // Over C: nothing it accepts is offered, so it is not even told.
        w.dnd.motion(over(C, 201, 5, 5), 0, 0);
        assert!(w.events(C).is_empty());
        assert_eq!(w.dnd.feedback().unwrap().effect, d::NONE);
        // Over B: ENTER names only the accepted subset, never content.
        w.dnd.motion(over(B, 101, 10, 20), 0, 1);
        let enter = w.events(B);
        assert_eq!(enter.len(), 1);
        assert_eq!(
            (enter[0].kind, enter[0].value, enter[0].a, enter[0].b),
            (
                d::ENTER,
                serial as u64,
                cb::mask(cb::TEXT),
                d::COPY | d::MOVE
            )
        );
        assert_eq!((enter[0].rect.x, enter[0].rect.y), (10, 20));
        assert_eq!(
            w.dnd.feedback().unwrap().effect,
            d::MOVE,
            "the target's default"
        );
        // Nobody reads before the drop.
        assert_eq!(w.read(B, serial, cb::TEXT).err(), Some(Code::NotFound));
        // Ctrl copies; the target's STATUS narrows to copy anyway.
        w.dnd.motion(over(B, 101, 12, 20), d::KEY_CONTROL, 2);
        assert_eq!(w.dnd.feedback().unwrap().effect, d::COPY);
        let mut status = body(&[serial as u64]);
        status.extend(window_bytes(101));
        status.extend(body(&[d::COPY as u64, d::COPY as u64]));
        w.call(B, d::STATUS, &status).unwrap();
        w.dnd.motion(over(B, 101, 13, 20), 0, 3);
        assert_eq!(
            w.dnd.feedback().unwrap().effect,
            d::COPY,
            "STATUS narrowed to copy"
        );
        let overs = w.events(B);
        assert!(overs.iter().all(|e| e.kind == d::OVER));
        assert_eq!(overs.len(), 1, "motion coalesces into one pending OVER");
        assert_eq!(
            w.call(C, d::STATUS, &status).err(),
            Some(Code::Changed),
            "only the current target answers"
        );
        assert_eq!(
            w.dnd.release(10),
            Released::Dropped {
                serial,
                effect: d::COPY
            }
        );
        let drop = w.events(B);
        assert_eq!(
            (drop[0].kind, drop[0].a, drop[0].b),
            (d::DROP, cb::mask(cb::TEXT), d::COPY)
        );
        // Scoped: the HTML was offered but B did not register for it.
        assert_eq!(w.read(B, serial, cb::TEXT).unwrap(), b"dragged words");
        assert_eq!(w.read(B, serial, cb::HTML).err(), Some(Code::Denied));
        assert_eq!(
            w.read(C, serial, cb::TEXT).err(),
            Some(Code::NotFound),
            "not the drop target"
        );
        assert_eq!(
            w.read(A, serial, cb::TEXT).err(),
            Some(Code::NotFound),
            "not even the source"
        );
        // A target cannot claim more than the drop allowed.
        assert_eq!(
            w.call(B, d::FINISH, &body(&[serial as u64, d::MOVE as u64]))
                .err(),
            Some(Code::InvalidRequest)
        );
        w.call(B, d::FINISH, &body(&[serial as u64, d::COPY as u64]))
            .unwrap();
        let finished = w.events(A);
        assert_eq!(
            (
                finished[0].kind,
                finished[0].a,
                finished[0].b,
                finished[0].window
            ),
            (d::FINISHED, d::COPY, d::OUTCOME_DROPPED, 1)
        );
        assert_eq!(
            w.read(B, serial, cb::TEXT).err(),
            Some(Code::NotFound),
            "freed after FINISH"
        );
        assert_eq!(w.dnd.held_bytes(), 0);
    }
    #[test]
    fn drop_descriptions_label_the_target_and_reset_on_leave() {
        let mut w = World::new();
        w.register(B, 101, cb::mask(cb::FILES), d::COPY | d::MOVE, d::MOVE);
        w.register(C, 201, cb::mask(cb::FILES), d::COPY, d::COPY);
        let serial = w
            .start(&[(cb::FILES, b"/users/1001/a.txt")], d::COPY | d::MOVE)
            .unwrap();
        w.dnd.motion(over(B, 101, 4, 4), 0, 0);
        assert_eq!(
            w.dnd.feedback().unwrap().insert,
            None,
            "no description before STATUS"
        );
        let status = |effects: u32, default: u32, insert: &[u8]| {
            let mut b = body(&[serial as u64]);
            b.extend(window_bytes(101));
            b.extend(body(&[effects as u64, default as u64]));
            b.extend_from_slice(insert);
            b
        };
        w.call(
            B,
            d::STATUS,
            &status(d::COPY | d::MOVE, d::MOVE, "Documents".as_bytes()),
        )
        .unwrap();
        let f = w.dnd.feedback().unwrap();
        assert_eq!(
            (f.effect, f.insert.unwrap().as_str()),
            (d::MOVE, "Documents")
        );
        // Malformed inserts are refused and leave the last answer in place.
        for bad in [
            &b"a\nb"[..],
            &[0xff, 0xfe][..],
            &[b'x'; d::MAX_DESCRIPTION + 1][..],
        ] {
            assert_eq!(
                w.call(B, d::STATUS, &status(d::COPY, d::COPY, bad)).err(),
                Some(Code::InvalidRequest)
            );
        }
        assert_eq!(
            w.dnd.feedback().unwrap().insert.unwrap().as_str(),
            "Documents"
        );
        // Refusing here hides the label; a plain STATUS clears the insert.
        w.call(
            B,
            d::STATUS,
            &status(d::NONE, d::NONE, "Documents".as_bytes()),
        )
        .unwrap();
        assert_eq!(w.dnd.feedback().unwrap().insert, None);
        w.call(B, d::STATUS, &status(d::COPY, d::COPY, &[]))
            .unwrap();
        assert_eq!(
            w.dnd.feedback().unwrap(),
            Feedback {
                effect: d::COPY,
                taskbar: false,
                insert: None
            }
        );
        // A new target starts without the old one's description.
        w.call(B, d::STATUS, &status(d::COPY, d::COPY, "文档".as_bytes()))
            .unwrap();
        w.dnd.motion(over(C, 201, 4, 4), 0, 1);
        assert_eq!(w.dnd.feedback().unwrap().insert, None);
        w.dnd.cancel();
        assert!(w.dnd.feedback().is_none());
    }
    #[test]
    fn clients_without_the_capability_are_refused() {
        let mut w = World::new();
        let stranger = Owner {
            pid: 99,
            session: 1,
            epoch: 1,
        };
        assert_eq!(
            w.call(stranger, d::START, &[]).err(),
            Some(Code::Unsupported)
        );
        w.dnd.owner_exited(B);
        let mut reg = window_bytes(101);
        reg.extend(body(&[
            cb::mask(cb::TEXT) as u64,
            d::COPY as u64,
            d::COPY as u64,
        ]));
        assert_eq!(
            w.call(B, d::REGISTER, &reg).err(),
            Some(Code::Unsupported),
            "an exited client is forgotten"
        );
    }
    #[test]
    fn a_cancelled_drag_leaves_the_target_and_tells_the_source() {
        let mut w = World::new();
        w.register(B, 101, cb::mask(cb::TEXT), d::COPY, d::COPY);
        let serial = w.start(&[(cb::TEXT, b"x")], d::COPY).unwrap();
        w.dnd.motion(over(B, 101, 1, 1), 0, 0);
        w.events(B);
        // Esc.
        w.dnd.cancel();
        assert!(!w.dnd.active());
        let leave = w.events(B);
        assert_eq!((leave[0].kind, leave[0].value), (d::LEAVE, serial as u64));
        let done = w.events(A);
        assert_eq!(
            (done[0].kind, done[0].a, done[0].b),
            (d::FINISHED, d::NONE, d::OUTCOME_CANCELLED)
        );
        assert_eq!(
            w.read(B, serial, cb::TEXT).err(),
            Some(Code::NotFound),
            "a cancelled offer is gone"
        );
        assert_eq!(w.dnd.held_bytes(), 0);
        // Releasing where nothing accepts is refused, not dropped.
        let serial = w.start(&[(cb::TEXT, b"y")], d::COPY).unwrap();
        w.dnd.motion(Hover::Nothing, 0, 0);
        assert_eq!(w.dnd.release(5), Released::Refused);
        let done = w.events(A);
        assert_eq!(
            (done[0].value, done[0].b),
            (serial as u64, d::OUTCOME_REFUSED)
        );
        // The source may cancel its own drag, nobody else's.
        let serial = w.start(&[(cb::TEXT, b"z")], d::COPY).unwrap();
        assert_eq!(
            w.call(B, d::CANCEL, &body(&[serial as u64])).err(),
            Some(Code::NotFound)
        );
        w.call(A, d::CANCEL, &body(&[serial as u64])).unwrap();
        assert!(!w.dnd.active());
    }
    #[test]
    fn drags_need_the_users_gesture_an_unlocked_session_and_an_owned_window() {
        let mut w = World::new();
        w.start.gesture = false;
        assert_eq!(
            w.start(&[(cb::TEXT, b"x")], d::COPY).err(),
            Some(Code::Denied)
        );
        w.start.gesture = true;
        let mut begin = window(101);
        begin.extend(body(&[d::COPY as u64]));
        assert_eq!(
            w.call(A, d::BEGIN, &begin).err(),
            Some(Code::Denied),
            "B's window"
        );
        // An empty offer is not a drag.
        let mut begin = window(1);
        begin.extend(body(&[d::COPY as u64]));
        w.call(A, d::BEGIN, &begin).unwrap();
        assert_eq!(w.call(A, d::START, &[]).err(), Some(Code::InvalidRequest));
        // A clipboard staging cannot be started as a drag, nor a drag committed.
        w.clip
            .request(&mut w.env, A, cb::BEGIN, &window(1), None)
            .unwrap();
        stage(&mut w.clip, &mut w.env, A, &[(cb::TEXT, b"x")]).unwrap();
        assert_eq!(w.call(A, d::START, &[]).err(), Some(Code::InvalidRequest));
        w.call(A, d::BEGIN, &begin).unwrap();
        stage(&mut w.clip, &mut w.env, A, &[(cb::TEXT, b"x")]).unwrap();
        assert_eq!(
            w.clip.request(&mut w.env, A, cb::COMMIT, &[], None).err(),
            Some(Code::InvalidRequest)
        );
        // Locked: nothing starts, and every request is denied.
        w.start.locked = true;
        assert_eq!(w.call(A, d::START, &[]).err(), Some(Code::Denied));
        let mut reg = window_bytes(101);
        reg.extend(body(&[
            cb::mask(cb::TEXT) as u64,
            d::COPY as u64,
            d::COPY as u64,
        ]));
        assert_eq!(w.call(B, d::REGISTER, &reg).err(), Some(Code::Denied));
        // Bad registrations.
        w.start.locked = false;
        let mut reg = window_bytes(101);
        reg.extend(body(&[
            cb::mask(cb::TEXT) as u64,
            d::COPY as u64,
            d::MOVE as u64,
        ]));
        assert_eq!(
            w.call(B, d::REGISTER, &reg).err(),
            Some(Code::InvalidRequest),
            "default outside effects"
        );
        let mut reg = window_bytes(1);
        reg.extend(body(&[
            cb::mask(cb::TEXT) as u64,
            d::COPY as u64,
            d::COPY as u64,
        ]));
        assert_eq!(
            w.call(B, d::REGISTER, &reg).err(),
            Some(Code::Denied),
            "A's window"
        );
    }
    #[test]
    fn owner_death_ends_drags_and_drops_without_stranding_anyone() {
        let mut w = World::new();
        w.register(B, 101, cb::mask(cb::TEXT), d::COPY | d::MOVE, d::MOVE);
        // The target exits mid-drag: the drag goes on with nothing under it.
        w.start(&[(cb::TEXT, b"x")], d::COPY | d::MOVE).unwrap();
        w.dnd.motion(over(B, 101, 1, 1), 0, 0);
        w.dnd.owner_exited(B);
        assert!(w.events(B).is_empty());
        assert_eq!(w.dnd.feedback().unwrap().effect, d::NONE);
        w.dnd.motion(over(B, 101, 2, 2), 0, 1);
        assert_eq!(
            w.dnd.release(2),
            Released::Refused,
            "B's registration died with it"
        );
        w.events(A);
        // The source exits mid-drag: cancelled; the target sees LEAVE.
        // (Test processes reconnect under the same identity.)
        w.dnd.grant(B);
        w.register(B, 101, cb::mask(cb::TEXT), d::COPY, d::COPY);
        w.start(&[(cb::TEXT, b"y")], d::COPY).unwrap();
        w.dnd.motion(over(B, 101, 1, 1), 0, 0);
        w.events(B);
        w.dnd.owner_exited(A);
        assert!(!w.dnd.active());
        assert_eq!(w.events(B)[0].kind, d::LEAVE);
        assert!(w.events(A).is_empty(), "no events queue for a dead client");
        // The source exits after the drop: the copied offer stays readable.
        w.dnd.grant(A);
        let serial = w.start(&[(cb::TEXT, b"kept")], d::COPY).unwrap();
        w.dnd.motion(over(B, 101, 1, 1), 0, 0);
        w.dnd.release(0);
        w.dnd.owner_exited(A);
        assert_eq!(w.read(B, serial, cb::TEXT).unwrap(), b"kept");
        w.dnd.grant(A);
        // The target exits before FINISH: the source learns the drop was lost.
        let serial = w.start(&[(cb::TEXT, b"lost")], d::COPY).unwrap();
        w.dnd.motion(over(B, 101, 1, 1), 0, 0);
        w.dnd.release(0);
        w.dnd.owner_exited(B);
        let done = w.events(A);
        assert_eq!(
            (done.last().unwrap().value, done.last().unwrap().b),
            (serial as u64, d::OUTCOME_LOST)
        );
        // Unfinished drops expire.
        w.dnd.grant(B);
        w.register(B, 101, cb::mask(cb::TEXT), d::COPY, d::COPY);
        let serial = w.start(&[(cb::TEXT, b"slow")], d::COPY).unwrap();
        w.dnd.motion(over(B, 101, 1, 1), 0, 0);
        w.dnd.release(100);
        w.events(A);
        assert_eq!(w.dnd.tick(100 + DROP_TICKS), None);
        assert_eq!(w.read(B, serial, cb::TEXT).err(), Some(Code::NotFound));
        assert_eq!(w.events(A)[0].b, d::OUTCOME_LOST);
    }
    #[test]
    fn resting_on_a_taskbar_button_activates_its_app_once() {
        let mut w = World::new();
        w.start(&[(cb::TEXT, b"x")], d::COPY).unwrap();
        w.dnd.motion(Hover::Taskbar(2), 0, 1000);
        assert!(w.dnd.feedback().unwrap().taskbar);
        assert_eq!(
            w.dnd.feedback().unwrap().effect,
            d::NONE,
            "the taskbar never takes the drop"
        );
        assert_eq!(w.dnd.tick(1000 + TASKBAR_HOVER_TICKS - 1), None);
        w.dnd
            .motion(Hover::Taskbar(2), 0, 1000 + TASKBAR_HOVER_TICKS - 1);
        assert_eq!(w.dnd.tick(1000 + TASKBAR_HOVER_TICKS), Some(2));
        assert_eq!(w.dnd.tick(5000), None, "once per rest");
        // Moving to another button restarts the wait.
        w.dnd.motion(Hover::Taskbar(3), 0, 6000);
        assert_eq!(w.dnd.tick(6000 + TASKBAR_HOVER_TICKS), Some(3));
    }
}
