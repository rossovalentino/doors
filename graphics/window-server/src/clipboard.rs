//! Session clipboard broker (door.clipboard v1, `native-ui-v1.md`), with
//! Windows 11 clipboard history. Copy-at-set: COMMIT keeps a private,
//! validated copy, so a paste never calls into (or waits on) the owner and
//! the data outlives an owner that exits, as non-delayed data does in
//! Windows. Bytes are shared (`Rc`) between the clipboard, its history entry
//! and drag offers, and everything held is bounded by
//! `clipboard::MAX_BROKER_BYTES`.
extern crate alloc;
use alloc::{rc::Rc, vec::Vec};
use door_graphics_core::Owner;
use door_service_protocol::{
    Code,
    clipboard::{self as cb, Info},
    window::Event,
};
/// One validated format.
#[derive(Clone, Debug)]
pub struct Data {
    pub format: u32,
    pub bytes: Rc<Vec<u8>>,
}
/// A complete clipboard set (or drag offer): each format at most once.
#[derive(Clone, Debug, Default)]
pub struct Content {
    pub formats: Vec<Data>,
}
impl Content {
    pub fn mask(&self) -> u32 {
        self.formats.iter().fold(0, |m, d| m | cb::mask(d.format))
    }
    pub fn get(&self, format: u32) -> Option<&[u8]> {
        self.formats
            .iter()
            .find(|d| d.format == format)
            .map(|d| d.bytes.as_slice())
    }
    pub fn bytes(&self) -> usize {
        self.formats.iter().map(|d| d.bytes.len()).sum()
    }
    pub fn is_empty(&self) -> bool {
        self.formats.is_empty()
    }
    /// The same bytes in the same formats.
    pub fn same(&self, other: &Content) -> bool {
        self.formats.len() == other.formats.len()
            && self
                .formats
                .iter()
                .all(|d| other.get(d.format).is_some_and(|b| b == d.bytes.as_slice()))
    }
    /// Text, HTML and bitmaps only, and only when under the 4 MB item limit
    /// (Windows 11 clipboard history).
    pub fn history_subset(&self) -> Option<Content> {
        let formats: Vec<Data> = self
            .formats
            .iter()
            .filter(|d| matches!(d.format, cb::TEXT | cb::HTML | cb::IMAGE))
            .cloned()
            .collect();
        let subset = Content { formats };
        (!subset.is_empty() && subset.bytes() <= cb::HISTORY_ITEM_BYTES).then_some(subset)
    }
}
/// Request environment, from authenticated server state.
pub trait Env {
    fn locked(&self) -> bool;
    /// `owner` owns the focused window.
    fn focused(&self, owner: Owner) -> bool;
    fn owns(&self, owner: Owner, window: u64) -> bool;
    /// May change Settings > System > Clipboard (the Settings app).
    fn settings_client(&self, owner: Owner) -> bool;
    /// Copy `out.len()` bytes from a transferred section at `offset`.
    fn snapshot(&mut self, section: u64, offset: u64, out: &mut [u8]) -> Result<(), Code>;
    /// Bytes held outside this broker (drag offers).
    fn external_bytes(&self) -> usize {
        0
    }
    /// Monotonic ticks (`platform::MONOTONIC_TICK_MILLIS`); 0 when unknown,
    /// which never expires a staging set.
    fn now(&self) -> u64;
}
/// What a staging set becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Clipboard,
    /// A drag source's offer with its allowed effects.
    Drag {
        effects: u32,
    },
}
struct Staging {
    owner: Owner,
    window: u64,
    purpose: Purpose,
    /// (format, bytes so far, declared size)
    formats: Vec<(u32, Vec<u8>, usize)>,
    /// Tick of the last request that advanced it.
    touched: u64,
}
impl Staging {
    fn declared(&self) -> usize {
        self.formats.iter().map(|f| f.2).sum()
    }
}
#[derive(Clone, Debug)]
pub struct HistoryItem {
    pub id: u32,
    pub content: Content,
    pub pinned: bool,
    /// Win+V preview lines, cut once when the item is added (never the
    /// whole 4 MB text per frame).
    pub preview: Vec<alloc::string::String>,
}
impl HistoryItem {
    pub fn new(id: u32, content: Content) -> Self {
        let preview = crate::clipboard_history::preview_of(&content);
        Self {
            id,
            content,
            pinned: false,
            preview,
        }
    }
}
/// The offer `take_drag` hands to the drag broker.
pub struct Offer {
    pub window: u64,
    pub effects: u32,
    pub content: Content,
}
pub enum Reply {
    Empty,
    Bytes(Vec<u8>),
}
#[derive(Default)]
pub struct Broker {
    content: Content,
    sequence: u32,
    owner: Option<Owner>,
    owner_alive: bool,
    staging: Vec<Staging>,
    /// Newest first.
    history: Vec<HistoryItem>,
    next_id: u32,
    settings: u32,
    /// CAP_CLIPBOARD clients and whether a CHANGED event is pending.
    listeners: Vec<(Owner, bool)>,
}
/// Staging sets kept at once (one per client, like window clients).
const MAX_STAGING: usize = 8;
/// One client's staging set: a full clipboard set and its effect word.
pub const STAGING_PER_CLIENT: usize = cb::MAX_CONTENT + 8;
/// Of `cb::MAX_BROKER_BYTES`, the part staging sets and drag offers may
/// declare. Staging never evicts history: committed data (clipboard and
/// history) keeps the rest, and only COMMIT makes room in it.
pub const STAGING_BYTES: usize = 2 * STAGING_PER_CLIENT;
pub const COMMITTED_BYTES: usize = cb::MAX_BROKER_BYTES - STAGING_BYTES;
/// A staging set no request advanced for 10 s is dropped, so a stalled or
/// hostile client cannot sit on the staging budget.
pub const STAGING_IDLE_TICKS: u64 = 10_000 / door_abi::platform::MONOTONIC_TICK_MILLIS;
fn word(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn long(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
impl Broker {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn content(&self) -> &Content {
        &self.content
    }
    pub fn sequence(&self) -> u32 {
        self.sequence
    }
    pub fn settings(&self) -> u32 {
        self.settings
    }
    pub fn history_enabled(&self) -> bool {
        self.settings & cb::SETTING_HISTORY != 0
    }
    pub fn history(&self) -> &[HistoryItem] {
        &self.history
    }
    pub fn info(&self) -> Info {
        let mut info = Info {
            sequence: self.sequence,
            formats: self.content.mask(),
            owner: match (self.owner, self.owner_alive) {
                (None, _) => cb::OWNER_NONE,
                (Some(_), true) => cb::OWNER_ALIVE,
                (Some(_), false) => cb::OWNER_EXITED,
            },
            sizes: [0; cb::FORMATS.len()],
        };
        for (i, f) in cb::FORMATS.iter().enumerate() {
            info.sizes[i] = self.content.get(*f).map_or(0, |b| b.len() as u32);
        }
        info
    }
    /// Every distinct buffer held (shared bytes counted once) plus declared
    /// staging sizes.
    pub fn held_bytes(&self) -> usize {
        Self::distinct(self.content.formats.iter().chain(self.history_formats()))
            + self.staging_bytes()
    }
    /// Bytes the staging sets declared.
    pub fn staging_bytes(&self) -> usize {
        self.staging.iter().map(Staging::declared).sum()
    }
    fn history_formats(&self) -> impl Iterator<Item = &Data> {
        self.history.iter().flat_map(|h| h.content.formats.iter())
    }
    /// Distinct buffers among `all` (shared bytes counted once).
    fn distinct<'a>(all: impl Iterator<Item = &'a Data>) -> usize {
        let mut seen: Vec<*const Vec<u8>> = Vec::new();
        let mut total = 0;
        for d in all {
            let p = Rc::as_ptr(&d.bytes);
            if !seen.contains(&p) {
                seen.push(p);
                total += d.bytes.len();
            }
        }
        total
    }
    /// A client negotiated CAP_CLIPBOARD.
    pub fn listen(&mut self, owner: Owner) {
        if !self.listeners.iter().any(|l| l.0 == owner) {
            self.listeners.push((owner, false));
        }
    }
    pub fn take_event(&mut self, owner: Owner) -> Option<Event> {
        let pending = self.listeners.iter_mut().find(|l| l.0 == owner && l.1)?;
        pending.1 = false;
        let info = self.info();
        Some(Event {
            kind: cb::CHANGED,
            a: info.sequence,
            b: info.formats,
            detail: info.owner,
            ..Event::default()
        })
    }
    fn changed(&mut self) {
        self.sequence = self.sequence.wrapping_add(1).max(1);
        for l in &mut self.listeners {
            l.1 = true;
        }
    }
    /// The owner exited: its staging goes, its committed data stays.
    pub fn owner_exited(&mut self, owner: Owner) {
        self.staging.retain(|s| s.owner != owner);
        self.listeners.retain(|l| l.0 != owner);
        if self.owner == Some(owner) {
            self.owner_alive = false;
        }
    }
    /// The session locked: nothing staged survives.
    pub fn lock(&mut self) {
        self.staging.clear();
    }
    /// Room for `bytes` more staging (drag offers held elsewhere count):
    /// checked against the staging budget only, never by evicting history.
    fn reserve_staging(&self, bytes: usize, external: usize) -> Result<(), Code> {
        if self.staging_bytes() + external + bytes > STAGING_BYTES {
            return Err(Code::Resources);
        }
        Ok(())
    }
    /// COMMIT: make room for `content` replacing the clipboard by dropping
    /// the oldest unpinned history items; nothing changes when even that
    /// is not enough.
    fn make_room(&mut self, content: &Content) -> Result<(), Code> {
        let fits = |history: &[HistoryItem]| {
            let kept = history.iter().flat_map(|h| h.content.formats.iter());
            Self::distinct(content.formats.iter().chain(kept)) <= COMMITTED_BYTES
        };
        // Planned on a copy (shared bytes, at most 25 items) so a refusal
        // leaves the history as it was.
        let mut kept = self.history.clone();
        while !fits(&kept) {
            let i = kept
                .iter()
                .rposition(|h| !h.pinned)
                .ok_or(Code::Resources)?;
            kept.remove(i);
        }
        self.history = kept;
        Ok(())
    }
    /// Drop the expired staging sets.
    fn expire_staging(&mut self, now: u64) {
        self.staging
            .retain(|s| now.saturating_sub(s.touched) <= STAGING_IDLE_TICKS);
    }
    /// Replace the clipboard (COMMIT, a history paste, Clear).
    fn set(&mut self, content: Content, owner: Option<Owner>) {
        self.content = content;
        self.owner = owner;
        self.owner_alive = owner.is_some();
        self.changed();
    }
    fn remember(&mut self) {
        if !self.history_enabled() {
            return;
        }
        let Some(subset) = self.content.history_subset() else {
            return;
        };
        if let Some(i) = self.history.iter().position(|h| h.content.same(&subset)) {
            // Copying the same thing again moves it to the top.
            let item = self.history.remove(i);
            self.history.insert(0, item);
            return;
        }
        if self.history.len() >= cb::HISTORY_ITEMS {
            match self.history.iter().rposition(|h| !h.pinned) {
                Some(i) => {
                    self.history.remove(i);
                }
                None => return,
            }
        }
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.history
            .insert(0, HistoryItem::new(self.next_id, subset));
    }
    #[cfg(test)]
    pub(crate) fn remember_for_tests(&mut self, content: Content) {
        self.set(content, None);
        self.remember();
    }
    fn staging_mut(&mut self, owner: Owner) -> Result<&mut Staging, Code> {
        self.staging
            .iter_mut()
            .find(|s| s.owner == owner)
            .ok_or(Code::NotFound)
    }
    /// Open a staging set (clipboard BEGIN and `dnd::BEGIN`).
    pub fn begin(
        &mut self,
        env: &dyn Env,
        owner: Owner,
        window: u64,
        purpose: Purpose,
    ) -> Result<(), Code> {
        if env.locked() {
            return Err(Code::Denied);
        }
        if !env.owns(owner, window) {
            return Err(Code::Denied);
        }
        self.expire_staging(env.now());
        self.staging.retain(|s| s.owner != owner);
        if self.staging.len() >= MAX_STAGING {
            return Err(Code::Resources);
        }
        self.staging.push(Staging {
            owner,
            window,
            purpose,
            formats: Vec::new(),
            touched: env.now(),
        });
        Ok(())
    }
    /// The drag offer staged by `owner` (validated), for `dnd::START`.
    pub fn take_drag(&mut self, owner: Owner) -> Result<Offer, Code> {
        let i = self
            .staging
            .iter()
            .position(|s| s.owner == owner)
            .ok_or(Code::NotFound)?;
        let Purpose::Drag { effects } = self.staging[i].purpose else {
            return Err(Code::InvalidRequest);
        };
        let staging = self.staging.remove(i);
        let window = staging.window;
        let content = Self::finish(staging)?;
        if content.is_empty() {
            return Err(Code::InvalidRequest);
        }
        Ok(Offer {
            window,
            effects,
            content,
        })
    }
    fn finish(staging: Staging) -> Result<Content, Code> {
        let mut formats = Vec::with_capacity(staging.formats.len());
        for (format, bytes, declared) in staging.formats {
            if bytes.len() != declared {
                return Err(Code::InvalidRequest);
            }
            cb::validate(format, &bytes)?;
            formats.push(Data {
                format,
                bytes: Rc::new(bytes),
            });
        }
        Ok(Content { formats })
    }
    /// Dispatch one clipboard opcode. `section` is the handle transferred
    /// with WRITE_SECTION (no other opcode accepts one).
    pub fn request(
        &mut self,
        env: &mut dyn Env,
        owner: Owner,
        op: u32,
        b: &[u8],
        section: Option<u64>,
    ) -> Result<Reply, Code> {
        if section.is_some() != (op == cb::WRITE_SECTION) {
            return Err(Code::InvalidRequest);
        }
        // A locked session neither discloses nor accepts clipboard data.
        if env.locked() {
            return Err(Code::Denied);
        }
        let now = env.now();
        self.expire_staging(now);
        // Clients that did not negotiate CAP_CLIPBOARD (staging writes are
        // reachable only through a BEGIN or a drag BEGIN).
        let granted = self.listeners.iter().any(|l| l.0 == owner);
        if !granted
            && matches!(
                op,
                cb::BEGIN
                    | cb::INFO
                    | cb::READ
                    | cb::SETTINGS_GET
                    | cb::SETTINGS_SET
                    | cb::CLEAR_DATA
            )
        {
            return Err(Code::Unsupported);
        }
        match op {
            cb::BEGIN if b.len() == 8 => self
                .begin(env, owner, long(b, 0), Purpose::Clipboard)
                .map(|_| Reply::Empty),
            cb::FORMAT if b.len() == 8 => {
                let (format, bytes) = (word(b, 0), word(b, 4) as usize);
                if cb::mask(format) == 0 {
                    return Err(Code::Unsupported);
                }
                if bytes == 0 {
                    return Err(Code::InvalidRequest);
                }
                let s = self.staging_mut(owner)?;
                if s.formats.iter().any(|f| f.0 == format) {
                    return Err(Code::Exists);
                }
                if bytes > cb::max_bytes(format) || s.declared() + bytes > STAGING_PER_CLIENT {
                    return Err(Code::Resources);
                }
                self.reserve_staging(bytes, env.external_bytes())?;
                let mut data = Vec::new();
                data.try_reserve_exact(bytes).map_err(|_| Code::Resources)?;
                let s = self.staging_mut(owner)?;
                s.formats.push((format, data, bytes));
                s.touched = now;
                Ok(Reply::Empty)
            }
            cb::WRITE if (8..=8 + cb::WRITE_BYTES).contains(&b.len()) => {
                let (format, offset) = (word(b, 0), word(b, 4) as usize);
                let s = self.staging_mut(owner)?;
                s.touched = now;
                let f = s
                    .formats
                    .iter_mut()
                    .find(|f| f.0 == format)
                    .ok_or(Code::NotFound)?;
                if offset != f.1.len() || offset + b.len() - 8 > f.2 {
                    return Err(Code::InvalidRequest);
                }
                f.1.extend_from_slice(&b[8..]);
                Ok(Reply::Empty)
            }
            cb::WRITE_SECTION if b.len() == 12 => {
                let (format, offset, bytes) =
                    (word(b, 0), word(b, 4) as usize, word(b, 8) as usize);
                let handle = section.ok_or(Code::InvalidRequest)?;
                let s = self.staging_mut(owner)?;
                s.touched = now;
                let f = s
                    .formats
                    .iter_mut()
                    .find(|f| f.0 == format)
                    .ok_or(Code::NotFound)?;
                if bytes == 0 || offset != f.1.len() || offset + bytes > f.2 {
                    return Err(Code::InvalidRequest);
                }
                f.1.resize(offset + bytes, 0);
                if let Err(e) = env.snapshot(handle, 0, &mut f.1[offset..]) {
                    f.1.truncate(offset);
                    return Err(e);
                }
                Ok(Reply::Empty)
            }
            cb::COMMIT if b.is_empty() => {
                let i = self
                    .staging
                    .iter()
                    .position(|s| s.owner == owner)
                    .ok_or(Code::NotFound)?;
                if self.staging[i].purpose != Purpose::Clipboard {
                    return Err(Code::InvalidRequest);
                }
                let staging = self.staging.remove(i);
                let content = Self::finish(staging)?;
                self.make_room(&content)?;
                self.set(content, Some(owner));
                self.remember();
                Ok(Reply::Bytes(self.sequence.to_le_bytes().to_vec()))
            }
            cb::ABORT if b.is_empty() => {
                self.staging.retain(|s| s.owner != owner);
                Ok(Reply::Empty)
            }
            cb::INFO if b.is_empty() => Ok(Reply::Bytes(self.info().encode().to_vec())),
            cb::READ if b.len() == 12 => {
                // Only the app the user is working in reads (as Windows does
                // for packaged apps): background apps cannot snoop.
                if !env.focused(owner) {
                    return Err(Code::Denied);
                }
                if word(b, 0) != self.sequence {
                    return Err(Code::Changed);
                }
                let data = self.content.get(word(b, 4)).ok_or(Code::NotFound)?;
                read_chunk(data, word(b, 8) as usize)
            }
            cb::SETTINGS_GET if b.is_empty() => {
                Ok(Reply::Bytes(self.settings.to_le_bytes().to_vec()))
            }
            cb::SETTINGS_SET if b.len() == 4 => {
                if !env.settings_client(owner) {
                    return Err(Code::Denied);
                }
                self.apply_settings(word(b, 0))?;
                Ok(Reply::Empty)
            }
            cb::CLEAR_DATA if b.is_empty() => {
                if !env.settings_client(owner) {
                    return Err(Code::Denied);
                }
                self.clear_data();
                Ok(Reply::Empty)
            }
            cb::FIRST..=cb::LAST => Err(Code::InvalidRequest),
            _ => Err(Code::Unsupported),
        }
    }
    /// Settings > System > Clipboard. Turning history off clears it, pinned
    /// items included (Windows 11).
    pub fn apply_settings(&mut self, settings: u32) -> Result<(), Code> {
        if settings & !cb::SETTINGS_KNOWN != 0 {
            return Err(Code::InvalidRequest);
        }
        if settings & cb::SETTING_HISTORY == 0 {
            self.history.clear();
        }
        self.settings = settings;
        Ok(())
    }
    /// "Clear clipboard data": the clipboard and unpinned history.
    pub fn clear_data(&mut self) {
        self.history.retain(|h| h.pinned);
        self.set(Content::default(), None);
    }
    /// Win+V "Clear all": unpinned history (the current clipboard stays).
    pub fn clear_history(&mut self) {
        self.history.retain(|h| h.pinned);
    }
    pub fn toggle_pin(&mut self, id: u32) -> bool {
        match self.history.iter_mut().find(|h| h.id == id) {
            Some(h) => {
                h.pinned = !h.pinned;
                true
            }
            None => false,
        }
    }
    pub fn delete(&mut self, id: u32) -> bool {
        let before = self.history.len();
        self.history.retain(|h| h.id != id);
        before != self.history.len()
    }
    /// Win+V: the chosen item becomes the clipboard (owned by the shell)
    /// and moves to the top.
    /// The shell's own set (a snip): replaces the clipboard like a COMMIT
    /// with no owning client, and joins the history when it is on.
    pub fn set_system(&mut self, content: Content) -> Result<(), Code> {
        if content.bytes() > cb::MAX_CONTENT {
            return Err(Code::Resources);
        }
        self.make_room(&content)?;
        self.set(content, None);
        self.remember();
        Ok(())
    }
    pub fn paste_history(&mut self, id: u32) -> bool {
        let Some(i) = self.history.iter().position(|h| h.id == id) else {
            return false;
        };
        let item = self.history.remove(i);
        self.set(item.content.clone(), None);
        self.history.insert(0, item);
        true
    }
}
/// One READ reply: up to `READ_BYTES` from `offset` (empty at the end).
pub fn read_chunk(data: &[u8], offset: usize) -> Result<Reply, Code> {
    if offset > data.len() {
        return Err(Code::InvalidRequest);
    }
    let end = (offset + cb::READ_BYTES).min(data.len());
    Ok(Reply::Bytes(data[offset..end].to_vec()))
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use alloc::vec;
    pub(crate) const A: Owner = Owner {
        pid: 10,
        session: 1,
        epoch: 1,
    };
    pub(crate) const B: Owner = Owner {
        pid: 11,
        session: 1,
        epoch: 1,
    };
    pub(crate) const SETTINGS: Owner = Owner {
        pid: 12,
        session: 1,
        epoch: 1,
    };
    /// Windows 1.. belong to A, 101.. to B, 201.. to Settings.
    pub(crate) struct TestEnv {
        pub locked: bool,
        pub focus: Option<Owner>,
        pub sections: Vec<(u64, Vec<u8>)>,
        pub external: usize,
        pub now: u64,
    }
    impl Default for TestEnv {
        fn default() -> Self {
            Self {
                locked: false,
                focus: Some(B),
                sections: Vec::new(),
                external: 0,
                now: 0,
            }
        }
    }
    impl Env for TestEnv {
        fn locked(&self) -> bool {
            self.locked
        }
        fn focused(&self, owner: Owner) -> bool {
            self.focus == Some(owner)
        }
        fn owns(&self, owner: Owner, window: u64) -> bool {
            match window {
                1..=100 => owner == A,
                101..=200 => owner == B,
                201..=300 => owner == SETTINGS,
                _ => false,
            }
        }
        fn settings_client(&self, owner: Owner) -> bool {
            owner == SETTINGS
        }
        fn snapshot(&mut self, section: u64, offset: u64, out: &mut [u8]) -> Result<(), Code> {
            let (_, data) = self
                .sections
                .iter()
                .find(|s| s.0 == section)
                .ok_or(Code::InvalidHandle)?;
            let at = offset as usize;
            let src = data.get(at..at + out.len()).ok_or(Code::Io)?;
            out.copy_from_slice(src);
            Ok(())
        }
        fn external_bytes(&self) -> usize {
            self.external
        }
        fn now(&self) -> u64 {
            self.now
        }
    }
    /// A broker with A, B and Settings connected with CAP_CLIPBOARD.
    pub(crate) fn broker() -> Broker {
        let mut c = Broker::new();
        for o in [A, B, SETTINGS] {
            c.listen(o);
        }
        c
    }
    pub(crate) fn body(words: &[u64]) -> Vec<u8> {
        let mut out = Vec::new();
        for w in words {
            out.extend_from_slice(&(*w as u32).to_le_bytes());
        }
        out
    }
    pub(crate) fn window(w: u64) -> Vec<u8> {
        w.to_le_bytes().to_vec()
    }
    /// BEGIN/FORMAT/WRITE every format, in WRITE_BYTES chunks.
    pub(crate) fn stage(
        broker: &mut Broker,
        env: &mut TestEnv,
        owner: Owner,
        formats: &[(u32, &[u8])],
    ) -> Result<(), Code> {
        for (format, bytes) in formats {
            broker.request(
                env,
                owner,
                cb::FORMAT,
                &body(&[*format as u64, bytes.len() as u64]),
                None,
            )?;
            for (i, chunk) in bytes.chunks(cb::WRITE_BYTES).enumerate() {
                let mut b = body(&[*format as u64, (i * cb::WRITE_BYTES) as u64]);
                b.extend_from_slice(chunk);
                broker.request(env, owner, cb::WRITE, &b, None)?;
            }
        }
        Ok(())
    }
    pub(crate) fn copy(
        broker: &mut Broker,
        env: &mut TestEnv,
        owner: Owner,
        win: u64,
        formats: &[(u32, &[u8])],
    ) -> Result<u32, Code> {
        broker.request(env, owner, cb::BEGIN, &window(win), None)?;
        stage(broker, env, owner, formats)?;
        match broker.request(env, owner, cb::COMMIT, &[], None)? {
            Reply::Bytes(b) => Ok(word(&b, 0)),
            Reply::Empty => Err(Code::Io),
        }
    }
    fn read_all(
        broker: &mut Broker,
        env: &mut TestEnv,
        owner: Owner,
        format: u32,
    ) -> Result<Vec<u8>, Code> {
        let sequence = broker.sequence();
        let mut out = Vec::new();
        loop {
            let Reply::Bytes(chunk) = broker.request(
                env,
                owner,
                cb::READ,
                &body(&[sequence as u64, format as u64, out.len() as u64]),
                None,
            )?
            else {
                return Err(Code::Io);
            };
            if chunk.is_empty() {
                return Ok(out);
            }
            out.extend_from_slice(&chunk);
        }
    }
    #[test]
    fn clients_without_the_capability_are_refused() {
        let (mut c, mut env) = (Broker::new(), TestEnv::default());
        assert_eq!(
            c.request(&mut env, A, cb::BEGIN, &window(1), None).err(),
            Some(Code::Unsupported)
        );
        assert_eq!(
            c.request(&mut env, A, cb::INFO, &[], None).err(),
            Some(Code::Unsupported)
        );
        assert_eq!(
            c.request(&mut env, A, cb::FORMAT, &body(&[1, 1]), None)
                .err(),
            Some(Code::NotFound)
        );
        c.listen(A);
        assert!(c.request(&mut env, A, cb::INFO, &[], None).is_ok());
    }
    #[test]
    fn copy_then_paste_in_another_app_survives_the_owner_exiting() {
        let (mut c, mut env) = (broker(), TestEnv::default());
        c.listen(B);
        let text = "Hello from Notes\r\nsecond line — ünïcode".repeat(20);
        let seq = copy(
            &mut c,
            &mut env,
            A,
            1,
            &[(cb::TEXT, text.as_bytes()), (cb::HTML, b"<p>Hello</p>")],
        )
        .unwrap();
        assert_eq!(seq, 1);
        let changed = c.take_event(B).unwrap();
        assert_eq!(
            (changed.kind, changed.a, changed.b, changed.detail),
            (
                cb::CHANGED,
                1,
                cb::mask(cb::TEXT) | cb::mask(cb::HTML),
                cb::OWNER_ALIVE
            )
        );
        assert!(c.take_event(B).is_none(), "one event per change");
        assert_eq!(
            read_all(&mut c, &mut env, B, cb::TEXT).unwrap(),
            text.as_bytes()
        );
        // The owner dies: data stays (copy-at-set), the owner reads "exited",
        // and nothing changed for listeners.
        c.owner_exited(A);
        let info = c.info();
        assert_eq!(
            (info.sequence, info.owner, info.size(cb::TEXT)),
            (1, cb::OWNER_EXITED, text.len() as u32)
        );
        assert!(c.take_event(B).is_none());
        assert_eq!(
            read_all(&mut c, &mut env, B, cb::HTML).unwrap(),
            b"<p>Hello</p>"
        );
        // A dead client's half-staged set is discarded with it (the test
        // reconnects A under the same identity).
        c.listen(A);
        c.request(&mut env, A, cb::BEGIN, &window(2), None).unwrap();
        stage(&mut c, &mut env, A, &[(cb::TEXT, b"never committed")]).unwrap();
        let held = c.held_bytes();
        c.owner_exited(A);
        assert_eq!(c.held_bytes(), held - 15);
        assert_eq!(
            c.request(&mut env, A, cb::COMMIT, &[], None).err(),
            Some(Code::NotFound)
        );
    }
    #[test]
    fn reads_are_scoped_to_the_focused_app_and_to_the_current_sequence() {
        let (mut c, mut env) = (broker(), TestEnv::default());
        copy(&mut c, &mut env, A, 1, &[(cb::TEXT, b"secret")]).unwrap();
        env.focus = Some(A);
        let read = |c: &mut Broker, env: &mut TestEnv, owner, seq: u32, format: u32| {
            c.request(
                env,
                owner,
                cb::READ,
                &body(&[seq as u64, format as u64, 0]),
                None,
            )
            .map(|_| ())
        };
        assert_eq!(
            read(&mut c, &mut env, B, 1, cb::TEXT),
            Err(Code::Denied),
            "background app"
        );
        env.focus = Some(B);
        assert_eq!(read(&mut c, &mut env, B, 1, cb::TEXT), Ok(()));
        assert_eq!(read(&mut c, &mut env, B, 1, cb::FILES), Err(Code::NotFound));
        copy(&mut c, &mut env, A, 1, &[(cb::TEXT, b"newer")]).unwrap();
        assert_eq!(
            read(&mut c, &mut env, B, 1, cb::TEXT),
            Err(Code::Changed),
            "stale sequence"
        );
        // Staging another app's window is refused.
        assert_eq!(
            c.request(&mut env, B, cb::BEGIN, &window(1), None).err(),
            Some(Code::Denied)
        );
    }
    #[test]
    fn size_limits_are_enforced_before_bytes_are_kept() {
        let (mut c, mut env) = (broker(), TestEnv::default());
        c.request(&mut env, A, cb::BEGIN, &window(1), None).unwrap();
        let declare = |c: &mut Broker, env: &mut TestEnv, f: u32, n: usize| {
            c.request(env, A, cb::FORMAT, &body(&[f as u64, n as u64]), None)
                .map(|_| ())
        };
        assert_eq!(
            declare(&mut c, &mut env, cb::TEXT, cb::MAX_CONTENT + 1),
            Err(Code::Resources)
        );
        assert_eq!(
            declare(&mut c, &mut env, cb::FILES, cb::MAX_FILES_BYTES + 1),
            Err(Code::Resources)
        );
        assert_eq!(
            declare(&mut c, &mut env, cb::DROP_EFFECT, 8),
            Err(Code::Resources)
        );
        assert_eq!(declare(&mut c, &mut env, 77, 8), Err(Code::Unsupported));
        assert_eq!(
            declare(&mut c, &mut env, cb::TEXT, 0),
            Err(Code::InvalidRequest)
        );
        assert_eq!(declare(&mut c, &mut env, cb::TEXT, 6 * 1024 * 1024), Ok(()));
        assert_eq!(declare(&mut c, &mut env, cb::TEXT, 4), Err(Code::Exists));
        // The whole set is bounded too.
        assert_eq!(
            declare(&mut c, &mut env, cb::HTML, 3 * 1024 * 1024),
            Err(Code::Resources)
        );
        // Writes stay inside the declared size and in order.
        let mut w = body(&[cb::TEXT as u64, 4]);
        w.extend_from_slice(b"abc");
        assert_eq!(
            c.request(&mut env, A, cb::WRITE, &w, None).err(),
            Some(Code::InvalidRequest),
            "gap"
        );
        let big = vec![0u8; 8 + cb::WRITE_BYTES + 1];
        assert_eq!(
            c.request(&mut env, A, cb::WRITE, &big, None).err(),
            Some(Code::InvalidRequest),
            "oversized chunk"
        );
        // An incomplete format never commits.
        assert_eq!(
            c.request(&mut env, A, cb::COMMIT, &[], None).err(),
            Some(Code::InvalidRequest)
        );
        assert_eq!(c.sequence(), 0);
        // The staging budget counts drag offers held elsewhere.
        c.request(&mut env, A, cb::BEGIN, &window(1), None).unwrap();
        env.external = STAGING_BYTES - 1024;
        assert_eq!(
            declare(&mut c, &mut env, cb::TEXT, 2048),
            Err(Code::Resources)
        );
        assert_eq!(declare(&mut c, &mut env, cb::TEXT, 512), Ok(()));
        // Invalid bytes are refused at COMMIT and the clipboard is unchanged.
        env.external = 0;
        assert_eq!(
            copy(&mut c, &mut env, A, 1, &[(cb::TEXT, &[0xff, 0x00])]),
            Err(Code::InvalidRequest)
        );
        assert_eq!(c.sequence(), 0);
    }
    #[test]
    fn section_writes_snapshot_large_payloads_once() {
        let (mut c, mut env) = (broker(), TestEnv::default());
        let mut image = vec![0u8; 8 + 64 * 32 * 4];
        image[..4].copy_from_slice(&64u32.to_le_bytes());
        image[4..8].copy_from_slice(&32u32.to_le_bytes());
        for p in image[8..].chunks_exact_mut(4) {
            p.copy_from_slice(&[10, 20, 30, 200]);
        }
        env.sections.push((77, image.clone()));
        c.request(&mut env, A, cb::BEGIN, &window(1), None).unwrap();
        c.request(
            &mut env,
            A,
            cb::FORMAT,
            &body(&[cb::IMAGE as u64, image.len() as u64]),
            None,
        )
        .unwrap();
        let ws = body(&[cb::IMAGE as u64, 0, image.len() as u64]);
        assert_eq!(
            c.request(&mut env, A, cb::WRITE_SECTION, &ws, None).err(),
            Some(Code::InvalidRequest),
            "handle required"
        );
        assert_eq!(
            c.request(&mut env, A, cb::WRITE, &ws, Some(77)).err(),
            Some(Code::InvalidRequest),
            "only WRITE_SECTION takes one"
        );
        c.request(&mut env, A, cb::WRITE_SECTION, &ws, Some(77))
            .unwrap();
        c.request(&mut env, A, cb::COMMIT, &[], None).unwrap();
        assert_eq!(c.content().get(cb::IMAGE), Some(&image[..]));
    }
    #[test]
    fn a_locked_session_denies_every_clipboard_request() {
        let (mut c, mut env) = (broker(), TestEnv::default());
        copy(&mut c, &mut env, A, 1, &[(cb::TEXT, b"before lock")]).unwrap();
        c.request(&mut env, A, cb::BEGIN, &window(1), None).unwrap();
        env.locked = true;
        c.lock();
        for (op, b) in [
            (cb::READ, body(&[1, cb::TEXT as u64, 0])),
            (cb::INFO, Vec::new()),
            (cb::BEGIN, window(1)),
            (cb::COMMIT, Vec::new()),
            (cb::SETTINGS_GET, Vec::new()),
        ] {
            assert_eq!(
                c.request(&mut env, B, op, &b, None).err(),
                Some(Code::Denied),
                "op {op:#x}"
            );
        }
        env.locked = false;
        // Staging did not survive the lock; the clipboard did.
        assert_eq!(
            c.request(&mut env, A, cb::COMMIT, &[], None).err(),
            Some(Code::NotFound)
        );
        assert_eq!(
            read_all(&mut c, &mut env, B, cb::TEXT).unwrap(),
            b"before lock"
        );
    }
    #[test]
    fn history_keeps_25_text_html_and_bitmap_items_under_4_mb_and_pins_survive_clear_all() {
        let (mut c, mut env) = (broker(), TestEnv::default());
        copy(&mut c, &mut env, A, 1, &[(cb::TEXT, b"history off")]).unwrap();
        assert!(c.history().is_empty(), "off by default, as in Windows");
        assert_eq!(
            c.request(&mut env, A, cb::SETTINGS_SET, &body(&[1]), None)
                .err(),
            Some(Code::Denied),
            "Settings only"
        );
        c.request(
            &mut env,
            SETTINGS,
            cb::SETTINGS_SET,
            &body(&[cb::SETTING_HISTORY as u64]),
            None,
        )
        .unwrap();
        for i in 0..30 {
            copy(
                &mut c,
                &mut env,
                A,
                1,
                &[(cb::TEXT, alloc::format!("item {i}").as_bytes())],
            )
            .unwrap();
        }
        assert_eq!(c.history().len(), cb::HISTORY_ITEMS);
        assert_eq!(c.history()[0].content.get(cb::TEXT), Some(&b"item 29"[..]));
        // Files are not history items; the current clipboard still changes.
        copy(&mut c, &mut env, A, 1, &[(cb::FILES, b"/users/1001/a.txt")]).unwrap();
        assert_eq!(c.history().len(), cb::HISTORY_ITEMS);
        assert_eq!(c.history()[0].content.get(cb::TEXT), Some(&b"item 29"[..]));
        // Over 4 MB: on the clipboard, not in history.
        let big = vec![b'x'; cb::HISTORY_ITEM_BYTES + 1];
        copy(&mut c, &mut env, A, 1, &[(cb::TEXT, &big)]).unwrap();
        assert_eq!(c.history()[0].content.get(cb::TEXT), Some(&b"item 29"[..]));
        // Copying an item again moves it to the top instead of duplicating.
        copy(&mut c, &mut env, A, 1, &[(cb::TEXT, b"item 20")]).unwrap();
        assert_eq!(c.history()[0].content.get(cb::TEXT), Some(&b"item 20"[..]));
        assert_eq!(c.history().len(), cb::HISTORY_ITEMS);
        let pinned = c.history()[3].id;
        assert!(c.toggle_pin(pinned));
        // When full, the oldest unpinned item goes.
        let oldest = c.history().last().unwrap().id;
        copy(&mut c, &mut env, A, 1, &[(cb::TEXT, b"fresh")]).unwrap();
        assert!(!c.history().iter().any(|h| h.id == oldest));
        // Win+V Clear all keeps pinned items and the current clipboard.
        c.clear_history();
        assert_eq!(c.history().len(), 1);
        assert_eq!(c.history()[0].id, pinned);
        assert_eq!(c.content().get(cb::TEXT), Some(&b"fresh"[..]));
        // Pasting from history makes it the clipboard (owned by the shell).
        let seq = c.sequence();
        assert!(c.paste_history(pinned));
        assert_eq!((c.sequence(), c.info().owner), (seq + 1, cb::OWNER_NONE));
        // Settings "Clear clipboard data": clipboard and unpinned history.
        copy(&mut c, &mut env, A, 1, &[(cb::TEXT, b"unpinned")]).unwrap();
        c.request(&mut env, SETTINGS, cb::CLEAR_DATA, &[], None)
            .unwrap();
        assert_eq!((c.history().len(), c.content().is_empty()), (1, true));
        // Turning history off clears it, pinned items included.
        c.request(&mut env, SETTINGS, cb::SETTINGS_SET, &body(&[0]), None)
            .unwrap();
        assert!(c.history().is_empty());
        assert_eq!(
            c.request(&mut env, SETTINGS, cb::SETTINGS_SET, &body(&[8]), None)
                .err(),
            Some(Code::InvalidRequest)
        );
    }
    #[test]
    fn history_shares_bytes_with_the_clipboard_and_evicts_under_the_broker_budget() {
        let (mut c, mut env) = (broker(), TestEnv::default());
        c.apply_settings(cb::SETTING_HISTORY).unwrap();
        let three = vec![b'a'; 3 * 1024 * 1024];
        copy(&mut c, &mut env, A, 1, &[(cb::TEXT, &three)]).unwrap();
        assert_eq!(
            c.held_bytes(),
            three.len(),
            "one buffer for clipboard and history"
        );
        for i in 0..12u8 {
            let item = vec![b'b' + i; 3 * 1024 * 1024];
            copy(&mut c, &mut env, A, 1, &[(cb::TEXT, &item)]).unwrap();
            assert!(c.held_bytes() <= cb::MAX_BROKER_BYTES);
        }
        assert!(c.history().len() < 12, "older items were evicted for room");
    }
    /// M3: staging declares against its own budget and never evicts history;
    /// one client cannot hold more than a set, and idle sets expire.
    #[test]
    fn staging_never_evicts_history_and_idle_sets_expire() {
        let (mut c, mut env) = (broker(), TestEnv::default());
        c.apply_settings(cb::SETTING_HISTORY).unwrap();
        let three = 3 * 1024 * 1024;
        for i in 0..4u8 {
            env.sections = vec![(7, vec![b'a' + i; three])];
            c.request(&mut env, A, cb::BEGIN, &window(1), None).unwrap();
            let format = body(&[cb::TEXT as u64, three as u64]);
            c.request(&mut env, A, cb::FORMAT, &format, None).unwrap();
            let write = body(&[cb::TEXT as u64, 0, three as u64]);
            c.request(&mut env, A, cb::WRITE_SECTION, &write, Some(7))
                .unwrap();
            c.request(&mut env, A, cb::COMMIT, &[], None).unwrap();
        }
        let ids = |c: &Broker| c.history().iter().map(|h| h.id).collect::<Vec<_>>();
        let before = ids(&c);
        assert_eq!(before.len(), 4);
        let declare = |c: &mut Broker, env: &mut TestEnv, o: Owner, f: u32, n: usize| {
            c.request(env, o, cb::FORMAT, &body(&[f as u64, n as u64]), None)
                .map(|_| ())
        };
        // BEGIN + 8 MiB FORMAT + ABORT: the history is untouched.
        c.request(&mut env, A, cb::BEGIN, &window(1), None).unwrap();
        assert_eq!(
            declare(&mut c, &mut env, A, cb::TEXT, cb::MAX_CONTENT),
            Ok(())
        );
        assert_eq!(ids(&c), before);
        c.request(&mut env, A, cb::ABORT, &[], None).unwrap();
        assert_eq!(ids(&c), before);
        assert_eq!(c.staging_bytes(), 0);
        // Two full sets fill the staging budget; a third client is refused
        // instead of wiping the history, and one client cannot exceed a set.
        for (o, w) in [(B, 101), (SETTINGS, 201)] {
            c.request(&mut env, o, cb::BEGIN, &window(w), None).unwrap();
            assert_eq!(
                declare(&mut c, &mut env, o, cb::TEXT, cb::MAX_CONTENT),
                Ok(())
            );
        }
        assert_eq!(
            declare(&mut c, &mut env, B, cb::HTML, 1024),
            Err(Code::Resources)
        );
        c.request(&mut env, A, cb::BEGIN, &window(1), None).unwrap();
        assert_eq!(
            declare(&mut c, &mut env, A, cb::TEXT, 1024),
            Err(Code::Resources)
        );
        assert_eq!(ids(&c), before);
        assert!(c.held_bytes() <= cb::MAX_BROKER_BYTES);
        // Ten idle seconds later the stalled sets are gone.
        env.now = STAGING_IDLE_TICKS + 1;
        c.request(&mut env, A, cb::BEGIN, &window(1), None).unwrap();
        assert_eq!(c.staging_bytes(), 0);
        assert_eq!(
            c.request(&mut env, B, cb::COMMIT, &[], None).err(),
            Some(Code::NotFound),
            "an expired set never commits"
        );
        // Only COMMIT makes room: an 8 MiB set drops the oldest items.
        env.sections = vec![(7, vec![b'z'; cb::MAX_CONTENT])];
        assert_eq!(
            declare(&mut c, &mut env, A, cb::TEXT, cb::MAX_CONTENT),
            Ok(())
        );
        let write = body(&[cb::TEXT as u64, 0, cb::MAX_CONTENT as u64]);
        c.request(&mut env, A, cb::WRITE_SECTION, &write, Some(7))
            .unwrap();
        c.request(&mut env, A, cb::COMMIT, &[], None).unwrap();
        assert_eq!(ids(&c), before[..2]);
        assert!(c.held_bytes() <= COMMITTED_BYTES);
    }
}
