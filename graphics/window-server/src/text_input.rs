//! Session text-input broker (door.text-input v1, `text-input-v1.md`, §53).
//!
//! The window server translates every physical key through the session's
//! current keyboard layout (`door_keyboard_layouts`), applies the Windows
//! input-method hotkeys, and routes the key:
//!
//! * a window without an enabled context gets the legacy INPUT key event
//!   (layout-translated ASCII, `KeyRoute::Legacy`);
//! * a window with an enabled context gets an ordered `KEY` record stream;
//! * when the current input method is an IME, the context is eligible
//!   (not secure, not URL/e-mail/number) and the IME is responsive, the key
//!   goes to the IME first, which finishes it in order: forwarded to the
//!   application or consumed into preedit/commit records.
//!
//! Only the registered IME owner receives `IME_KEY`, activation and
//! surrounding text; only it may send preedit/commit. A locked session
//! cancels composition and blocks every IME request.
extern crate alloc;
pub mod candidate;
use alloc::{collections::VecDeque, vec::Vec};
pub use candidate::CandidateWindow;
use door_abi::graphics as d;
use door_graphics_core::Owner;
use door_keyboard_layouts::{self as kl, Hotkeys, Ime, Preferences, Translator};
use door_service_protocol::{
    BODY_BYTES, Code,
    text_input::{self as ti, Key, Record},
    window::Rect,
};
/// Request environment, from authenticated server state.
pub trait Env {
    /// The focused window and its owner.
    fn focused(&self) -> Option<(Owner, u64)>;
    fn owns(&self, owner: Owner, window: u64) -> bool;
    /// Content-area origin of `window` in output physical pixels.
    fn content_origin(&self, window: u64) -> Option<(i32, i32)>;
    /// Output scale in percent (100, 125, 150 ...).
    fn scale_percent(&self) -> u32;
    fn locked(&self) -> bool;
    /// Monotonic ticks (10 ms), the unit of device timestamps.
    fn now(&self) -> u64;
}
/// The authenticated role of a requester (from its manifest and session).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Peer {
    App,
    /// Workspace apps: Settings and the taskbar input-indicator flyout.
    Settings,
    /// Init in session 0.
    Init,
    /// The session IME process (`ti::IME_MANIFEST`).
    Ime,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyRoute {
    /// Route through the window engine as a legacy key code.
    Legacy(u32),
    /// Delivered (or queued at the IME) through text input.
    Consumed,
}
pub enum Reply {
    Empty,
    Scalar(u64),
    Bytes(Vec<u8>),
}
struct Context {
    owner: Owner,
    window: u64,
    serial: u32,
    content: u32,
    hints: u32,
    caret: Rect,
    surrounding: Option<(Vec<u8>, u16, u16)>,
}
struct Queue {
    owner: Owner,
    records: VecDeque<Vec<u8>>,
    /// Overflowed and not polled since: no context may be enabled.
    overflowed: bool,
}
/// A CAP_TEXT_INPUT/CAP_IME client and whether its PENDING event is due.
struct Listener {
    owner: Owner,
    due: bool,
}
struct Pending {
    key_serial: u32,
    /// Context serial the key was sent to the IME for.
    serial: u32,
    owner: Owner,
    window: u64,
    key: Key,
    deadline: u64,
}
struct ImeClient {
    owner: Owner,
    methods: u32,
    /// Methods composing without a candidate dictionary (bit = method id).
    no_candidates: u32,
    records: VecDeque<Vec<u8>>,
    /// A key timed out: keys bypass the IME until it answers again.
    unresponsive: bool,
}
pub struct Broker {
    prefs: Preferences,
    translator: Translator,
    hotkeys: Hotkeys,
    contexts: Vec<Context>,
    queues: Vec<Queue>,
    ime: Option<ImeClient>,
    /// Context serial activated at the IME.
    active: Option<u32>,
    /// Recently deactivated contexts: a commit-on-blur still reaches them.
    retired: VecDeque<(u32, Owner, u64)>,
    pending: VecDeque<Pending>,
    next_serial: u32,
    next_key: u32,
    mode: u32,
    candidates: Option<CandidateWindow>,
    focus: Option<(Owner, u64)>,
    locked: bool,
    listeners: Vec<Listener>,
    /// Indicator or candidate window changed (repaint).
    dirty: bool,
    /// Keys forwarded without the IME (timeout, method switch, overflow)
    /// while it still holds them: the last such key serial. Until the IME
    /// finishes that key (or polls an empty queue) its output for the
    /// `tainted` contexts is dropped, so no text arrives twice.
    stale: Option<u32>,
    tainted: VecDeque<u32>,
    /// The current method changed (taskbar, serial marker); at most
    /// `MAX_ANNOUNCEMENTS` are reported.
    method_changed: bool,
    announcements: u32,
    /// Candidate windows opened, and the last one reported.
    opened: u32,
    announced: core::cell::Cell<(u32, u32)>,
}
/// Serial markers per session for method changes and candidate windows
/// (the window server's diagnostic budget is shared).
const MAX_ANNOUNCEMENTS: u32 = 24;
/// How a context leaves (`Broker::retire`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Retire {
    /// DISABLE: the IME commits its composition to the field, as on a
    /// focus change (late commits reach recently retired contexts).
    Commit,
    /// Replaced by a new ENABLE, or its window closed: the IME cancels and
    /// the application's preedit is cleared.
    Cancel,
    /// The application left or its queue overflowed: the IME cancels and
    /// nothing more is queued for the context.
    Silent,
}
fn clear_preedit() -> ti::PreeditBuf<'static> {
    ti::PreeditBuf {
        text: "",
        cursor: 0,
        count: 0,
        attrs: [ti::Attr::default(); ti::MAX_ATTRS],
    }
}
/// Control records are never dropped on overflow: commits, deletions,
/// disables and preedit clears. KEY records and preedit text are data.
fn control(r: &Record<'_>) -> bool {
    match r {
        Record::Key { .. } => false,
        Record::Preedit { preedit, .. } => preedit.text.is_empty(),
        _ => true,
    }
}
impl Default for Broker {
    fn default() -> Self {
        Self::new()
    }
}
fn w32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn w64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
fn encode(r: &Record<'_>) -> Result<Vec<u8>, Code> {
    let mut b = [0u8; ti::MAX_RECORD];
    let n = r.encode(&mut b)?;
    Ok(b[..n].to_vec())
}
/// Legacy window key code: physical code, packed flags, layout ASCII.
/// AltGr text drops Ctrl/Alt so legacy shortcuts never fire on it.
pub fn legacy_code(code: u32, flags: u32, t: &kl::Translation) -> u32 {
    let mut f = flags & d::PACKED_FLAGS;
    if t.status & kl::layout::ALTGR != 0 {
        f = (f & !(d::CONTROL | d::ALT)) | d::RIGHT_ALT;
    }
    code | (f << 9) | (u32::from(t.legacy) << 16)
}
impl Broker {
    pub fn new() -> Self {
        Self {
            prefs: Preferences::DEFAULT,
            translator: Translator::new(),
            hotkeys: Hotkeys::default(),
            contexts: Vec::new(),
            queues: Vec::new(),
            ime: None,
            active: None,
            retired: VecDeque::new(),
            pending: VecDeque::new(),
            next_serial: 1,
            next_key: 1,
            mode: ti::MODE_DIRECT,
            candidates: None,
            focus: None,
            locked: false,
            listeners: Vec::new(),
            dirty: false,
            stale: None,
            tainted: VecDeque::new(),
            method_changed: false,
            announcements: 0,
            opened: 0,
            announced: core::cell::Cell::new((0, 0)),
        }
    }
    pub fn preferences(&self) -> Preferences {
        self.prefs
    }
    pub fn method(&self) -> &'static kl::Method {
        self.prefs.current()
    }
    /// A registered IME serves the current method. Otherwise keys type the
    /// method's keyboard layout directly, and the indicator says so.
    fn ime_serves(&self) -> bool {
        let m = self.method();
        m.ime != Ime::None
            && self
                .ime
                .as_ref()
                .is_some_and(|i| i.methods & (1 << m.id) != 0)
    }
    pub fn state(&self) -> ti::State {
        ti::State {
            preferences: self.prefs.bits(),
            method: u32::from(self.method().id),
            mode: if self.ime_serves() {
                self.mode
            } else {
                ti::MODE_DIRECT
            },
            flags: self.ime.as_ref().map_or(0, |i| {
                ti::STATE_IME | i.no_candidates << ti::NO_CANDIDATES_SHIFT
            }),
        }
    }
    /// The current method once after it changed (serial marker), at most
    /// `MAX_ANNOUNCEMENTS` times.
    pub fn take_method_change(&mut self) -> Option<&'static kl::Method> {
        if !core::mem::take(&mut self.method_changed) || self.announcements >= MAX_ANNOUNCEMENTS {
            return None;
        }
        self.announcements += 1;
        Some(self.method())
    }
    /// True once per opened candidate window (serial marker of the painted
    /// frame), at most `MAX_ANNOUNCEMENTS` times.
    pub fn announce_candidates(&self) -> bool {
        let (last, count) = self.announced.get();
        if self.candidates.is_none() || last == self.opened || count >= MAX_ANNOUNCEMENTS {
            return false;
        }
        self.announced.set((self.opened, count + 1));
        true
    }
    /// Taskbar input indicator; None when one keyboard is installed. The IME
    /// conversion-mode glyph (中, あ, 가) shows only while a registered IME
    /// serves the method; without one it reads as direct input.
    pub fn indicator(&self) -> Option<kl::Indicator> {
        let m = self.method();
        (self.prefs.count() > 1 || m.ime != Ime::None)
            .then(|| kl::methods::indicator(m, self.state().mode))
    }
    /// The candidate window to draw, with its caret anchor.
    pub fn candidates(&self) -> Option<&CandidateWindow> {
        self.candidates.as_ref()
    }
    /// The focused window's text caret in output physical pixels, when it
    /// has an enabled context (Win+V opens at the caret, as in Windows).
    pub fn caret(&self, env: &dyn Env) -> Option<Rect> {
        let (owner, window) = env.focused()?;
        let c = self
            .contexts
            .iter()
            .find(|c| c.owner == owner && c.window == window)?;
        Some(self.screen_caret(env, c))
    }
    /// Indicator or candidate window changed since the last call.
    pub fn take_dirty(&mut self) -> bool {
        core::mem::take(&mut self.dirty)
    }
    /// HELLO granted CAP_TEXT_INPUT or CAP_IME: PENDING events start.
    pub fn listen(&mut self, owner: Owner) {
        if !self.listeners.iter().any(|l| l.owner == owner) {
            self.listeners.push(Listener { owner, due: false });
        }
        self.notify(owner);
    }
    fn queued(&self, owner: Owner) -> usize {
        if let Some(ime) = self.ime.as_ref().filter(|i| i.owner == owner) {
            return ime.records.len();
        }
        self.queues
            .iter()
            .find(|q| q.owner == owner)
            .map_or(0, |q| q.records.len())
    }
    fn notify(&mut self, owner: Owner) {
        let queued = self.queued(owner);
        if let Some(l) = self.listeners.iter_mut().find(|l| l.owner == owner) {
            l.due = queued > 0;
        }
    }
    /// The PENDING window event for `owner`, once per batch of records.
    pub fn take_event(&mut self, owner: Owner) -> Option<door_service_protocol::window::Event> {
        let l = self
            .listeners
            .iter_mut()
            .find(|l| l.owner == owner && l.due)?;
        l.due = false;
        Some(door_service_protocol::window::Event {
            kind: ti::PENDING,
            a: self.queued(owner) as u32,
            ..Default::default()
        })
    }
    fn queue(&mut self, owner: Owner) -> &mut Queue {
        if let Some(i) = self.queues.iter().position(|q| q.owner == owner) {
            return &mut self.queues[i];
        }
        self.queues.push(Queue {
            owner,
            records: VecDeque::new(),
            overflowed: false,
        });
        self.queues.last_mut().unwrap()
    }
    /// Queue a record for an application. Data (KEY, preedit text) fits
    /// `MAX_QUEUED_RECORDS`; control records are never dropped. When the
    /// queue is full the queued data is dropped, the application's contexts
    /// are disabled (`R_DISABLED`, kept in order after the control records)
    /// and false is returned for a data record. Until the application polls,
    /// no context can be enabled, so control records stay within
    /// `MAX_QUEUED_RECORDS + CONTROL_RESERVE`.
    fn push_app(&mut self, owner: Owner, r: Record<'_>) -> bool {
        let Ok(bytes) = encode(&r) else {
            return false;
        };
        let control = control(&r);
        if self.queue(owner).records.len() >= ti::MAX_QUEUED_RECORDS {
            self.overflow(owner);
            if !control {
                self.notify(owner);
                return false;
            }
        }
        let q = self.queue(owner);
        debug_assert!(q.records.len() < ti::MAX_QUEUED_RECORDS + ti::CONTROL_RESERVE);
        if q.records.len() >= ti::MAX_QUEUED_RECORDS + ti::CONTROL_RESERVE {
            return false;
        }
        q.records.push_back(bytes);
        self.notify(owner);
        true
    }
    /// The application stopped polling: drop its queued data, disable its
    /// contexts (the IME cancels them) and forget its keys at the IME.
    fn overflow(&mut self, owner: Owner) {
        let q = self.queue(owner);
        q.overflowed = true;
        q.records.retain(|b| match ti::decode_record(b) {
            Ok((r, _)) => control(&r),
            Err(_) => false,
        });
        let dropped: Vec<(u64, u32)> = self
            .contexts
            .iter()
            .filter(|c| c.owner == owner)
            .map(|c| (c.window, c.serial))
            .collect();
        self.pending.retain(|p| p.owner != owner);
        self.retired.retain(|r| r.1 != owner);
        for (window, serial) in dropped {
            self.retire(serial, Retire::Silent);
            if let Ok(rec) = encode(&Record::Disabled {
                window,
                serial,
                reason: ti::DISABLED_OVERFLOW,
            }) {
                self.queue(owner).records.push_back(rec);
            }
        }
    }
    fn push_ime(&mut self, r: Record<'_>) {
        let Ok(bytes) = encode(&r) else {
            return;
        };
        if let Some(ime) = self.ime.as_mut() {
            let owner = ime.owner;
            if ime.records.len() < ti::MAX_QUEUED_RECORDS {
                ime.records.push_back(bytes);
            } else {
                ime.unresponsive = true;
            }
            self.notify(owner);
        }
    }
    fn context(&self, serial: u32) -> Option<&Context> {
        self.contexts.iter().find(|c| c.serial == serial)
    }
    /// Retire a context (DISABLE, re-ENABLE, window closed, owner gone or
    /// overflowed). An active one is deactivated at the IME first, while it
    /// still exists, so the commit-on-blur or the preedit clear reaches the
    /// application exactly as on a focus change.
    fn retire(&mut self, serial: u32, how: Retire) {
        if self.active == Some(serial) {
            match how {
                Retire::Commit => self.deactivate(false, true),
                Retire::Cancel => self.deactivate(true, true),
                Retire::Silent => self.deactivate(true, false),
            }
        }
        if how == Retire::Silent {
            self.retired.retain(|r| r.0 != serial);
        }
        self.contexts.retain(|c| c.serial != serial);
    }
    fn eligible(&self, c: &Context) -> bool {
        let m = self.method();
        m.ime != Ime::None
            && !ti::secure(c.content)
            && !ti::latin(c.content)
            && self
                .ime
                .as_ref()
                .is_some_and(|i| i.methods & (1 << m.id) != 0)
    }
    fn screen_caret(&self, env: &dyn Env, c: &Context) -> Rect {
        let (x, y) = env.content_origin(c.window).unwrap_or((0, 0));
        Rect {
            x: x.saturating_add(c.caret.x),
            y: y.saturating_add(c.caret.y),
            ..c.caret
        }
    }
    /// Deactivate the IME context. `cancel`: the IME drops its composition
    /// (otherwise it commits it to the retired context); `notify`: the
    /// application's preedit is cleared when cancelled.
    fn deactivate(&mut self, cancel: bool, notify: bool) {
        let Some(serial) = self.active.take() else {
            return;
        };
        if cancel {
            self.push_ime(Record::Reset {
                serial,
                mode: ti::RESET_CANCEL,
            });
        }
        self.push_ime(Record::Deactivate { serial });
        if let Some(c) = self.context(serial) {
            let (owner, window) = (c.owner, c.window);
            if self.retired.len() == 4 {
                self.retired.pop_front();
            }
            self.retired.push_back((serial, owner, window));
            if cancel && notify {
                self.push_app(
                    owner,
                    Record::Preedit {
                        window,
                        serial,
                        preedit: clear_preedit(),
                    },
                );
            }
        }
        if self.candidates.take().is_some() {
            self.dirty = true;
        }
    }
    /// Follow focus, window lifetime and the lock state: (de)activate the
    /// IME context. Called before every key and request. A context whose
    /// window closed is retired with its composition cancelled.
    pub fn sync(&mut self, env: &dyn Env) {
        let stale: Vec<u32> = self
            .contexts
            .iter()
            .filter(|c| !env.owns(c.owner, c.window))
            .map(|c| c.serial)
            .collect();
        for serial in stale {
            self.retire(serial, Retire::Cancel);
        }
        let locked = env.locked();
        let focus = if locked { None } else { env.focused() };
        if focus != self.focus {
            self.translator.reset();
            self.focus = focus;
        }
        let target = focus
            .and_then(|(o, w)| self.contexts.iter().find(|c| c.owner == o && c.window == w))
            .filter(|c| self.eligible(c))
            .map(|c| c.serial);
        if target != self.active {
            // Locking cancels composition; a focus change commits it.
            self.deactivate(locked && !self.locked, true);
            if let Some(serial) = target {
                let c = self.context(serial).unwrap();
                let (content, hints) = (c.content, c.hints);
                let caret = self.screen_caret(env, c);
                let surrounding = c.surrounding.clone();
                let method = u32::from(self.method().id);
                self.push_ime(Record::Activate {
                    serial,
                    content,
                    hints,
                    method,
                    scale: env.scale_percent(),
                    caret,
                });
                if let Some((text, cursor, anchor)) = surrounding {
                    let text = core::str::from_utf8(&text).unwrap_or("");
                    self.push_ime(Record::Surrounding {
                        serial,
                        surrounding: ti::Surrounding {
                            text,
                            cursor,
                            anchor,
                        },
                    });
                }
                self.active = Some(serial);
            }
        }
        self.locked = locked;
    }
    fn set_preferences(&mut self, env: &dyn Env, p: Preferences) {
        let changed = p.current().id != self.prefs.current().id;
        self.prefs = p;
        self.dirty = true;
        if changed {
            self.translator.reset();
            self.flush_pending();
            self.deactivate(false, true);
            self.method_changed = true;
            self.push_ime(Record::Method {
                method: u32::from(p.current().id),
            });
            self.mode = ti::MODE_NATIVE;
        }
        self.sync(env);
    }
    /// Windows input hotkeys, before the shell's own Win chords. True when
    /// the event is consumed (Win+Space and its release).
    pub fn hotkey(&mut self, env: &dyn Env, event: &d::Input) -> bool {
        let (switch, consumed) = self.hotkeys.event(event.code, event.flags);
        if let Some(s) = switch {
            let next = s.apply(self.prefs);
            if next != self.prefs {
                self.set_preferences(env, next);
            }
        }
        consumed
    }
    /// Forward every pending IME key to its application, in order. The IME
    /// still holds these keys, so they get an epoch (`stale`): its output for
    /// their contexts is dropped until it has finished the last of them, the
    /// composition built from them is cancelled at the IME (or, for a
    /// context no longer active, its late commit refused) and the
    /// application's preedit is cleared. No text arrives twice.
    fn flush_pending(&mut self) {
        let Some(last) = self.pending.back().map(|p| p.key_serial) else {
            return;
        };
        let mut contexts: Vec<(u32, Owner, u64)> = Vec::new();
        while let Some(p) = self.pending.pop_front() {
            if !contexts.iter().any(|c| c.0 == p.serial) {
                contexts.push((p.serial, p.owner, p.window));
            }
            let key = Key {
                status: p.key.status | ti::KEY_FORWARDED,
                ..p.key
            };
            self.push_app(
                p.owner,
                Record::Key {
                    window: p.window,
                    key,
                },
            );
        }
        if self.ime.is_none() {
            return;
        }
        self.stale = Some(last);
        for (serial, owner, window) in contexts {
            if !self.tainted.contains(&serial) {
                if self.tainted.len() == ti::MAX_PENDING_KEYS {
                    self.tainted.pop_front();
                }
                self.tainted.push_back(serial);
            }
            if self.active == Some(serial) {
                self.push_ime(Record::Reset {
                    serial,
                    mode: ti::RESET_CANCEL,
                });
            } else {
                self.retired.retain(|r| r.0 != serial);
            }
            self.push_app(
                owner,
                Record::Preedit {
                    window,
                    serial,
                    preedit: clear_preedit(),
                },
            );
        }
    }
    /// The IME finished every key forwarded without it: its output counts
    /// again and it is responsive.
    fn caught_up(&mut self) {
        self.stale = None;
        self.tainted.clear();
        if let Some(ime) = self.ime.as_mut() {
            ime.unresponsive = false;
        }
    }
    /// IME output for `serial` produced from keys already forwarded raw.
    fn late(&self, serial: u32) -> bool {
        self.stale.is_some() && self.tainted.contains(&serial)
    }
    fn expire(&mut self, now: u64) {
        if self.pending.front().is_some_and(|p| now >= p.deadline) {
            if let Some(ime) = self.ime.as_mut() {
                ime.unresponsive = true;
            }
            self.flush_pending();
        }
    }
    /// Translate and route one KEY event that reached application routing.
    pub fn key(&mut self, env: &dyn Env, event: &d::Input) -> KeyRoute {
        self.sync(env);
        self.expire(env.now());
        let t = self
            .translator
            .translate(self.method().layout, event.code, event.flags);
        let legacy = legacy_code(event.code, event.flags, &t);
        let mut text = [0u8; 16];
        let n = t.text.encode(&mut text);
        let key = Key {
            code: event.code,
            flags: event.flags,
            legacy,
            vk: u16::from(t.vk),
            usage: t.usage,
            status: t.status,
            timestamp: event.timestamp,
            ..Key::default()
        }
        .with_text(core::str::from_utf8(&text[..n]).unwrap_or(""));
        let Some((owner, window)) = self.focus else {
            return KeyRoute::Legacy(legacy);
        };
        let Some(serial) = self
            .contexts
            .iter()
            .find(|c| c.owner == owner && c.window == window)
            .map(|c| c.serial)
        else {
            return KeyRoute::Legacy(legacy);
        };
        let to_ime = self.active == Some(serial)
            && self.ime.as_ref().is_some_and(|i| !i.unresponsive)
            && self.pending.len() < ti::MAX_PENDING_KEYS;
        if to_ime {
            let key_serial = self.next_key;
            self.next_key = self.next_key.wrapping_add(1).max(1);
            self.pending.push_back(Pending {
                key_serial,
                serial,
                owner,
                window,
                key,
                deadline: env.now() + ti::IME_KEY_TIMEOUT_TICKS,
            });
            self.push_ime(Record::ImeKey {
                key_serial,
                serial,
                key,
            });
            return KeyRoute::Consumed;
        }
        // Keys never overtake keys still at the IME.
        self.flush_pending();
        if self.push_app(owner, Record::Key { window, key }) {
            KeyRoute::Consumed
        } else {
            KeyRoute::Legacy(legacy)
        }
    }
    /// Device input epoch lost: drop dead keys, chords and pending keys and
    /// cancel composition.
    pub fn reset(&mut self) {
        self.translator.reset();
        self.hotkeys.reset();
        self.pending.clear();
        if let Some(serial) = self.active {
            self.push_ime(Record::Reset {
                serial,
                mode: ti::RESET_CANCEL,
            });
        }
    }
    /// An authenticated client left (or was evicted).
    pub fn revoke(&mut self, owner: Owner) {
        if self.ime.as_ref().is_some_and(|i| i.owner == owner) {
            // The IME crashed or left: every composition it held is gone.
            // The active and recently retired contexts get a preedit clear,
            // its pending keys reach their applications, and no state of it
            // survives into the next registration.
            let mut cleared: Vec<(u32, Owner, u64)> = self.retired.drain(..).collect();
            if let Some(c) = self.active.and_then(|s| self.context(s)) {
                cleared.push((c.serial, c.owner, c.window));
            }
            self.ime = None;
            self.active = None;
            self.flush_pending();
            self.stale = None;
            self.tainted.clear();
            for (serial, o, window) in cleared {
                self.push_app(
                    o,
                    Record::Preedit {
                        window,
                        serial,
                        preedit: clear_preedit(),
                    },
                );
            }
            self.candidates = None;
            self.mode = ti::MODE_DIRECT;
            self.dirty = true;
        }
        let serials: Vec<u32> = self
            .contexts
            .iter()
            .filter(|c| c.owner == owner)
            .map(|c| c.serial)
            .collect();
        for s in serials {
            self.retire(s, Retire::Silent);
        }
        self.pending.retain(|p| p.owner != owner);
        self.retired.retain(|r| r.1 != owner);
        self.queues.retain(|q| q.owner != owner);
        self.listeners.retain(|l| l.owner != owner);
    }
    fn own_context(&self, owner: Owner, window: u64, serial: Option<u32>) -> Result<u32, Code> {
        let c = self
            .contexts
            .iter()
            .find(|c| c.owner == owner && c.window == window)
            .ok_or(Code::NotFound)?;
        match serial {
            Some(s) if s != c.serial => Err(Code::Changed),
            _ => Ok(c.serial),
        }
    }
    /// The active (or, for a commit, recently deactivated) context target.
    fn target(&self, serial: u32, allow_retired: bool) -> Result<(Owner, u64), Code> {
        if self.active == Some(serial) {
            let c = self.context(serial).ok_or(Code::Changed)?;
            return Ok((c.owner, c.window));
        }
        if allow_retired && let Some(r) = self.retired.iter().find(|r| r.0 == serial) {
            return Ok((r.1, r.2));
        }
        Err(Code::Changed)
    }
    fn poll(&mut self, peer: Peer, owner: Owner) -> Result<Reply, Code> {
        let records = if peer == Peer::Ime {
            &mut self
                .ime
                .as_mut()
                .filter(|i| i.owner == owner)
                .ok_or(Code::Denied)?
                .records
        } else {
            match self.queues.iter_mut().find(|q| q.owner == owner) {
                Some(q) => {
                    q.overflowed = false;
                    &mut q.records
                }
                None => return Err(Code::WouldBlock),
            }
        };
        let mut out = Vec::new();
        while let Some(r) = records.front() {
            if out.len() + r.len() > BODY_BYTES {
                break;
            }
            out.extend_from_slice(&records.pop_front().unwrap());
        }
        self.notify(owner);
        if out.is_empty() {
            // An IME polling an empty queue has finished every record it
            // was given (it answers in order before polling again).
            if peer == Peer::Ime {
                self.caught_up();
            }
            Err(Code::WouldBlock)
        } else {
            Ok(Reply::Bytes(out))
        }
    }
    /// One door.text-input request from an authenticated peer.
    pub fn request(
        &mut self,
        env: &dyn Env,
        peer: Peer,
        owner: Owner,
        op: u32,
        b: &[u8],
    ) -> Result<Reply, Code> {
        self.sync(env);
        self.expire(env.now());
        if ti::ime_op(op) {
            if peer != Peer::Ime {
                return Err(Code::Denied);
            }
            if env.locked() {
                return Err(Code::Denied);
            }
            if op != ti::IME_REGISTER {
                let stale = self.stale.is_some();
                let ime = self
                    .ime
                    .as_mut()
                    .filter(|i| i.owner == owner)
                    .ok_or(Code::Denied)?;
                // Still working through keys forwarded without it: it
                // stays bypassed until it catches up.
                if !stale {
                    ime.unresponsive = false;
                }
            }
        }
        match op {
            ti::ENABLE => {
                let e = ti::Enable::decode(b)?;
                if !env.owns(owner, e.window) {
                    return Err(Code::Denied);
                }
                // An overflowed queue must be polled before new contexts.
                if self.queued(owner) >= ti::MAX_QUEUED_RECORDS
                    || self.queues.iter().any(|q| q.owner == owner && q.overflowed)
                {
                    return Err(Code::Resources);
                }
                if let Some(old) = self
                    .contexts
                    .iter()
                    .find(|c| c.owner == owner && c.window == e.window)
                    .map(|c| c.serial)
                {
                    // The new context replaces it: preedit discarded.
                    self.retire(old, Retire::Cancel);
                } else if self.contexts.iter().filter(|c| c.owner == owner).count()
                    >= ti::MAX_CONTEXTS
                    || self.contexts.len() >= ti::MAX_TOTAL_CONTEXTS
                {
                    return Err(Code::Resources);
                }
                let serial = self.next_serial;
                self.next_serial = self.next_serial.wrapping_add(1).max(1);
                self.contexts.push(Context {
                    owner,
                    window: e.window,
                    serial,
                    content: e.content,
                    hints: e.hints,
                    caret: e.caret,
                    surrounding: None,
                });
                self.queue(owner);
                self.sync(env);
                Ok(Reply::Scalar(u64::from(serial)))
            }
            ti::DISABLE if b.len() == 8 => {
                let serial = self.own_context(owner, w64(b, 0), None)?;
                // Lost focus: the IME commits first (Windows behaviour).
                self.retire(serial, Retire::Commit);
                self.sync(env);
                Ok(Reply::Empty)
            }
            ti::CURSOR => {
                let (window, serial, caret) = ti::decode_cursor(b)?;
                let serial = self.own_context(owner, window, Some(serial))?;
                let c = self
                    .contexts
                    .iter_mut()
                    .find(|c| c.serial == serial)
                    .unwrap();
                c.caret = caret;
                if self.active == Some(serial) {
                    let caret = self.screen_caret(env, self.context(serial).unwrap());
                    self.push_ime(Record::Cursor {
                        serial,
                        scale: env.scale_percent(),
                        caret,
                    });
                    if let Some(c) = self.candidates.as_mut() {
                        c.anchor = caret;
                        self.dirty = true;
                    }
                }
                Ok(Reply::Empty)
            }
            ti::SURROUNDING => {
                let (window, serial, s) = ti::decode_surrounding(b)?;
                let serial = self.own_context(owner, window, Some(serial))?;
                let c = self
                    .contexts
                    .iter_mut()
                    .find(|c| c.serial == serial)
                    .unwrap();
                if ti::secure(c.content) {
                    return Err(Code::Unsupported);
                }
                c.surrounding = Some((s.text.as_bytes().to_vec(), s.cursor, s.anchor));
                if self.active == Some(serial) {
                    self.push_ime(Record::Surrounding {
                        serial,
                        surrounding: s,
                    });
                }
                Ok(Reply::Empty)
            }
            ti::RESET if b.len() == 16 => {
                let serial = self.own_context(owner, w64(b, 0), Some(w32(b, 8)))?;
                let mode = w32(b, 12);
                if mode > ti::RESET_COMPLETE {
                    return Err(Code::InvalidRequest);
                }
                if self.active == Some(serial) {
                    self.push_ime(Record::Reset { serial, mode });
                }
                Ok(Reply::Empty)
            }
            ti::POLL if b.is_empty() => self.poll(peer, owner),
            ti::STATE if b.is_empty() => Ok(Reply::Bytes(self.state().encode().to_vec())),
            ti::SET_METHODS | ti::SELECT | ti::SET_PREFERENCES if b.len() == 4 => {
                let allowed = match op {
                    ti::SET_PREFERENCES => peer == Peer::Init,
                    _ => peer == Peer::Settings,
                };
                if !allowed {
                    return Err(Code::Denied);
                }
                let p = if op == ti::SELECT {
                    self.prefs.select(w32(b, 0) as usize)
                } else {
                    Preferences::from_bits(w32(b, 0))
                }
                .ok_or(Code::InvalidRequest)?;
                self.set_preferences(env, p);
                Ok(Reply::Empty)
            }
            ti::IME_REGISTER if b.len() == 8 => {
                if w32(b, 0) != ti::VERSION {
                    return Err(Code::Unsupported);
                }
                if self.ime.as_ref().is_some_and(|i| i.owner != owner) {
                    return Err(Code::Exists);
                }
                let methods = w32(b, 4) & kl::methods::IME_METHODS;
                let no_candidates = (w32(b, 4) >> ti::NO_CANDIDATES_SHIFT) & methods;
                self.ime = Some(ImeClient {
                    owner,
                    methods,
                    no_candidates,
                    records: VecDeque::new(),
                    unresponsive: false,
                });
                self.active = None;
                self.stale = None;
                self.tainted.clear();
                self.mode = ti::MODE_NATIVE;
                self.dirty = true;
                self.sync(env);
                Ok(Reply::Scalar(u64::from(self.method().id)))
            }
            ti::IME_PREEDIT => {
                let (serial, p) = ti::decode_ime_preedit(b)?;
                // A preedit clear still reaches a retired context (its
                // composition was committed or cancelled on retirement).
                let (o, window) = self.target(serial, p.text.is_empty())?;
                if !self.late(serial) {
                    self.push_app(
                        o,
                        Record::Preedit {
                            window,
                            serial,
                            preedit: p,
                        },
                    );
                }
                Ok(Reply::Empty)
            }
            ti::IME_COMMIT => {
                // At most MAX_COMMIT bytes: exactly one COMMIT record.
                let (serial, text) = ti::decode_ime_commit(b)?;
                let (o, window) = self.target(serial, true)?;
                if !self.late(serial) {
                    self.push_app(
                        o,
                        Record::Commit {
                            window,
                            serial,
                            text,
                        },
                    );
                }
                Ok(Reply::Empty)
            }
            ti::IME_DELETE if b.len() == 12 => {
                let serial = w32(b, 0);
                let (o, window) = self.target(serial, false)?;
                let (before, after) = (w32(b, 4), w32(b, 8));
                if before > ti::MAX_SURROUNDING as u32 || after > ti::MAX_SURROUNDING as u32 {
                    return Err(Code::InvalidRequest);
                }
                if !self.late(serial) {
                    self.push_app(
                        o,
                        Record::Delete {
                            window,
                            serial,
                            before,
                            after,
                        },
                    );
                }
                Ok(Reply::Empty)
            }
            ti::IME_KEY_DONE if b.len() == 8 => {
                let (key_serial, verdict) = (w32(b, 0), w32(b, 4));
                if verdict > ti::FORWARD {
                    return Err(Code::InvalidRequest);
                }
                if let Some(last) = self.stale
                    && key_serial.wrapping_sub(last) as i32 >= 0
                {
                    // The IME finished the last key forwarded without it.
                    self.caught_up();
                }
                let Some(at) = self.pending.iter().position(|p| p.key_serial == key_serial) else {
                    // Already forwarded after a timeout.
                    return Ok(Reply::Empty);
                };
                for _ in 0..at {
                    let p = self.pending.pop_front().unwrap();
                    let key = Key {
                        status: p.key.status | ti::KEY_FORWARDED,
                        ..p.key
                    };
                    self.push_app(
                        p.owner,
                        Record::Key {
                            window: p.window,
                            key,
                        },
                    );
                }
                let p = self.pending.pop_front().unwrap();
                if verdict == ti::FORWARD {
                    let key = Key {
                        status: p.key.status | ti::KEY_FORWARDED,
                        ..p.key
                    };
                    self.push_app(
                        p.owner,
                        Record::Key {
                            window: p.window,
                            key,
                        },
                    );
                }
                Ok(Reply::Empty)
            }
            ti::IME_CANDIDATES => {
                let mut items = [""; ti::MAX_CANDIDATES];
                let n = b.get(5).copied().unwrap_or(0) as usize;
                let c = ti::decode_candidates(b, &mut items)?;
                self.target(c.serial, false)?;
                if self.late(c.serial) {
                    return Ok(Reply::Empty);
                }
                let caret = self.screen_caret(env, self.context(c.serial).unwrap());
                if n > 0 && self.candidates.is_none() {
                    self.opened = self.opened.wrapping_add(1);
                }
                self.candidates = (n > 0)
                    .then(|| CandidateWindow::new(c, &items[..n], caret, env.scale_percent()));
                self.dirty = true;
                Ok(Reply::Empty)
            }
            ti::IME_MODE if b.len() == 4 => {
                let mode = w32(b, 0);
                if mode > ti::MODE_FULLWIDTH {
                    return Err(Code::InvalidRequest);
                }
                self.dirty |= mode != self.mode;
                self.mode = mode;
                Ok(Reply::Empty)
            }
            _ => Err(Code::InvalidRequest),
        }
    }
}
#[cfg(test)]
mod tests;
