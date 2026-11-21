//! The registry: a typed, cached view of one caller's settings with
//! separate read, propose/validate, commit and observe steps.
use crate::backend::{Backend, BackendError};
use crate::event::{Event, EventKind, Notice, ResetReason};
use crate::schema::{
    self, Apply, Caller, Dependency, MAX_WORDS, Property, PropertyId, PropertySet, Schema,
    SchemaError, Visibility, Word,
};
use door_settings_map::Page;

/// In-process subscriptions per registry (fixed capacity, no allocation).
pub const MAX_SUBSCRIBERS: usize = 8;
/// Diagnostic events kept until drained (the oldest are dropped).
pub const EVENT_CAPACITY: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No property has this key.
    UnknownKey,
    Invalid(Invalid),
    ReadOnly(ReadOnly),
    /// The property changed since the proposal; read it and propose again.
    Stale,
    /// Every subscription slot is in use.
    Capacity,
    Backend(BackendError),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalid {
    /// Not a value of the property's type (or wider than its field).
    Type,
    /// A valid value the storage word refuses with the other fields'
    /// current values.
    Combination,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOnly {
    /// The caller lacks the authority (machine settings for a user).
    Permission,
    /// Storage reported damage; defaults are shown, nothing is written.
    StorageCorrupt,
    /// The caller cannot read it.
    Unreadable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Ok,
    /// A persisted value failed validation and could not be rewritten.
    Damaged,
    StorageCorrupt,
    Unreadable,
}

/// A property as read now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PropertyState {
    /// The stored value.
    pub value: u32,
    /// The value in effect (a `ForcedUnless` dependency may override it).
    pub effective: u32,
    pub read_only: Option<ReadOnly>,
    /// An `ActiveWhen` dependency that currently disables it.
    pub inactive_by: Option<PropertyId>,
}

/// A validated, not yet committed change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Proposal {
    pub id: PropertyId,
    pub value: u32,
    word: u8,
    base: u32,
    next: u32,
    /// When the change (and its effects) takes effect: the latest timing.
    pub apply: Apply,
    /// Other properties whose stored value, effective value or
    /// availability the change alters (shared storage, dependencies).
    pub effects: PropertySet,
    /// After the change, the property is disabled by this one.
    pub inactive_by: Option<PropertyId>,
}
impl Proposal {
    /// Committing changes nothing.
    pub const fn unchanged(&self) -> bool {
        self.next == self.base
    }
}

/// The outcome of a commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Committed {
    /// Every property whose stored or effective value changed.
    pub changed: PropertySet,
    /// `SignOut` / `Restart`: the user must do this before it applies.
    pub apply: Apply,
}

/// A search result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub id: PropertyId,
    pub key: &'static str,
    pub title: &'static str,
    pub page: Page,
    pub anchor: Option<&'static str>,
    /// Shown, but the caller cannot change it.
    pub read_only: bool,
}

/// An in-process subscription handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Subscription {
    slot: u8,
    generation: u32,
}
#[derive(Clone, Copy)]
struct Subscriber {
    live: bool,
    generation: u32,
    filter: PropertySet,
    pending: PropertySet,
}

const NO_EVENT: Event = Event {
    kind: EventKind::Commit,
    key: "",
    value: 0,
};

/// Typed settings for one caller. Every call that touches persistence
/// takes the backend by reference; the registry keeps only a cache, the
/// cross-process watch token and in-process subscriptions.
pub struct Registry {
    schema: &'static Schema,
    caller: Caller,
    words: [u32; MAX_WORDS],
    status: [Status; MAX_WORDS],
    watch: Option<u64>,
    sequence: u64,
    subscribers: [Subscriber; MAX_SUBSCRIBERS],
    generation: u32,
    events: [Event; EVENT_CAPACITY],
    head: usize,
    queued: usize,
    lost: u32,
    notice: Notice,
}

impl Registry {
    /// The built-in schema (checked by the crate's tests).
    pub fn new(caller: Caller) -> Self {
        Self::build(&crate::table::BUILTIN, caller)
    }
    /// A custom schema, checked first.
    pub fn with_schema(schema: &'static Schema, caller: Caller) -> Result<Self, SchemaError> {
        schema::validate(schema)?;
        Ok(Self::build(schema, caller))
    }
    fn build(schema: &'static Schema, caller: Caller) -> Self {
        let mut words = [0; MAX_WORDS];
        for (slot, word) in words.iter_mut().zip(schema.words) {
            *slot = word.default;
        }
        Self {
            schema,
            caller,
            words,
            status: [Status::Ok; MAX_WORDS],
            watch: None,
            sequence: 0,
            subscribers: [Subscriber {
                live: false,
                generation: 0,
                filter: PropertySet::EMPTY,
                pending: PropertySet::EMPTY,
            }; MAX_SUBSCRIBERS],
            generation: 0,
            events: [NO_EVENT; EVENT_CAPACITY],
            head: 0,
            queued: 0,
            lost: 0,
            notice: Notice::default(),
        }
    }
    pub fn schema(&self) -> &'static Schema {
        self.schema
    }
    pub fn caller(&self) -> Caller {
        self.caller
    }

    // ---- Reading ----------------------------------------------------------

    pub fn find(&self, key: &str) -> Option<PropertyId> {
        schema::find(self.schema.properties, key).map(|i| PropertyId(i as u8))
    }
    /// The declaration of `id` (an id of this registry's schema).
    pub fn property(&self, id: PropertyId) -> &'static Property {
        &self.schema.properties[id.index()]
    }
    /// The storage word holding `id`: persistence location, scope and write
    /// authority.
    pub fn word(&self, id: PropertyId) -> &'static Word {
        &self.schema.words[usize::from(self.property(id).word)]
    }
    pub fn properties(&self) -> impl Iterator<Item = PropertyId> + use<> {
        (0..self.schema.properties.len() as u8).map(PropertyId)
    }
    /// Properties shown on `page`, in schema order.
    pub fn on_page(&self, page: Page) -> impl Iterator<Item = PropertyId> + '_ {
        self.properties()
            .filter(move |id| self.property(*id).page == page)
    }
    /// The stored value (the default until loaded, or when damaged).
    pub fn get(&self, id: PropertyId) -> u32 {
        Self::stored(self.schema, &self.words, id)
    }
    pub fn get_bool(&self, id: PropertyId) -> bool {
        self.get(id) != 0
    }
    pub fn get_key(&self, key: &str) -> Result<u32, Error> {
        self.find(key)
            .map(|id| self.get(id))
            .ok_or(Error::UnknownKey)
    }
    /// The value in effect after dependencies.
    pub fn effective(&self, id: PropertyId) -> u32 {
        self.effective_in(&self.words, id)
    }
    pub fn state(&self, id: PropertyId) -> PropertyState {
        PropertyState {
            value: self.get(id),
            effective: self.effective(id),
            read_only: self.read_only(id),
            inactive_by: self.inactive_in(&self.words, id),
        }
    }
    /// Why the caller cannot change `id`, if it cannot.
    pub fn read_only(&self, id: PropertyId) -> Option<ReadOnly> {
        let index = usize::from(self.property(id).word);
        match self.status[index] {
            Status::Unreadable => Some(ReadOnly::Unreadable),
            Status::StorageCorrupt => Some(ReadOnly::StorageCorrupt),
            _ if !self.caller.may(self.schema.words[index].write) => Some(ReadOnly::Permission),
            _ => None,
        }
    }
    /// Display name of `value` for `id` (choices, catalogs, special values).
    pub fn label(&self, id: PropertyId, value: u32) -> Option<&'static str> {
        self.property(id).kind.label(value)
    }
    fn stored(schema: &Schema, words: &[u32; MAX_WORDS], id: PropertyId) -> u32 {
        let p = &schema.properties[id.index()];
        p.extract(words[usize::from(p.word)])
    }
    fn controller(&self, key: &str) -> Option<PropertyId> {
        self.find(key)
    }
    fn effective_in(&self, words: &[u32; MAX_WORDS], id: PropertyId) -> u32 {
        let mut value = Self::stored(self.schema, words, id);
        for dependency in self.property(id).depends {
            if let Dependency::ForcedUnless {
                key,
                value: on,
                forced,
            } = *dependency
                && let Some(other) = self.controller(key)
                && Self::stored(self.schema, words, other) != on
            {
                value = forced;
            }
        }
        value
    }
    fn inactive_in(&self, words: &[u32; MAX_WORDS], id: PropertyId) -> Option<PropertyId> {
        self.property(id).depends.iter().find_map(|d| match *d {
            Dependency::ActiveWhen { key, value } => self
                .controller(key)
                .filter(|other| Self::stored(self.schema, words, *other) != value),
            Dependency::ForcedUnless { .. } => None,
        })
    }
    /// Properties whose stored value, effective value or availability
    /// differ between two word arrays.
    fn diff(&self, before: &[u32; MAX_WORDS], after: &[u32; MAX_WORDS]) -> PropertySet {
        let mut set = PropertySet::EMPTY;
        for id in self.properties() {
            if Self::stored(self.schema, before, id) != Self::stored(self.schema, after, id)
                || self.effective_in(before, id) != self.effective_in(after, id)
                || self.inactive_in(before, id) != self.inactive_in(after, id)
            {
                set.insert(id);
            }
        }
        set
    }
    fn latest(&self, set: PropertySet) -> Apply {
        set.iter()
            .map(|id| self.property(id).apply)
            .max()
            .unwrap_or_default()
    }
    fn word_ok(&self, index: usize, value: u32) -> bool {
        (self.schema.words[index].valid)(value)
            && self
                .schema
                .properties
                .iter()
                .filter(|p| usize::from(p.word) == index)
                .all(|p| p.kind.accepts(p.extract(value)))
    }
    fn word_properties(&self, index: usize) -> impl Iterator<Item = PropertyId> + '_ {
        self.properties()
            .filter(move |id| usize::from(self.property(*id).word) == index)
    }

    // ---- Proposing and validating -----------------------------------------

    /// Check `value` for `id` against its type and the storage word, without
    /// regard to permission.
    pub fn validate(&self, id: PropertyId, value: u32) -> Result<(), Error> {
        self.compose(id, value, self.words[usize::from(self.property(id).word)])
            .map(|_| ())
    }
    fn compose(&self, id: PropertyId, value: u32, base: u32) -> Result<u32, Error> {
        let p = self.property(id);
        if !p.kind.accepts(value) {
            return Err(Error::Invalid(Invalid::Type));
        }
        let next = p.insert(base, value).ok_or(Error::Invalid(Invalid::Type))?;
        if !(self.schema.words[usize::from(p.word)].valid)(next) {
            return Err(Error::Invalid(Invalid::Combination));
        }
        Ok(next)
    }
    fn predict(&self, words: &mut [u32; MAX_WORDS], index: usize, value: u32) {
        words[index] = value;
        if let Some(couple) = self.schema.words[index].couple {
            let other = usize::from(couple.word);
            words[other] = (couple.map)(value, words[other]);
        }
    }
    /// Validate a change and describe its consequences; nothing is written.
    pub fn propose(&self, id: PropertyId, value: u32) -> Result<Proposal, Error> {
        if let Some(reason) = self.read_only(id) {
            return Err(Error::ReadOnly(reason));
        }
        let word = self.property(id).word;
        let base = self.words[usize::from(word)];
        let next = self.compose(id, value, base)?;
        let mut after = self.words;
        self.predict(&mut after, usize::from(word), next);
        let changed = self.diff(&self.words, &after);
        let mut effects = PropertySet::EMPTY;
        for other in changed.iter().filter(|other| *other != id) {
            effects.insert(other);
        }
        Ok(Proposal {
            id,
            value,
            word,
            base,
            next,
            apply: self.latest(changed.union(PropertySet::of(id))),
            effects,
            inactive_by: self.inactive_in(&after, id),
        })
    }
    pub fn propose_key(&self, key: &str, value: u32) -> Result<Proposal, Error> {
        self.propose(self.find(key).ok_or(Error::UnknownKey)?, value)
    }

    // ---- Committing --------------------------------------------------------

    /// Persist a proposal. The word is read back first: a change elsewhere
    /// to another field is kept (the proposal is rebased), a change to this
    /// property returns `Stale`. A backend error keeps the old value.
    pub fn commit<B: Backend + ?Sized>(
        &mut self,
        backend: &mut B,
        proposal: Proposal,
    ) -> Result<Committed, Error> {
        let index = usize::from(proposal.word);
        if proposal.id.index() >= self.schema.properties.len()
            || self.property(proposal.id).word != proposal.word
        {
            return Err(Error::UnknownKey);
        }
        if let Some(reason) = self.read_only(proposal.id) {
            return Err(Error::ReadOnly(reason));
        }
        self.read_word(backend, index, true)?;
        if let Some(reason) = self.read_only(proposal.id) {
            return Err(Error::ReadOnly(reason));
        }
        let current = self.words[index];
        let p = self.property(proposal.id);
        let next = if current == proposal.base {
            proposal.next
        } else if p.extract(current) == p.extract(proposal.base) {
            self.compose(proposal.id, proposal.value, current)?
        } else {
            return Err(Error::Stale);
        };
        if next == current {
            return Ok(Committed {
                changed: PropertySet::EMPTY,
                apply: Apply::Live,
            });
        }
        let store = self.schema.words[index].store;
        if let Err(error) = backend.write(store, next) {
            // Interrupted: keep the old value unless storage now reports
            // another one.
            if error != BackendError::Disconnected {
                let _ = self.read_word(backend, index, true);
            }
            return Err(Error::Backend(error));
        }
        let before = self.words;
        let mut after = self.words;
        self.predict(&mut after, index, next);
        self.words = after;
        self.status[index] = Status::Ok;
        let changed = self.diff(&before, &after);
        // The committed property first, then those it changed through a
        // coupling (dark mode moves the console theme's dark bit).
        let own = proposal.id;
        let order = core::iter::once(own)
            .filter(|id| changed.contains(*id))
            .chain(changed.iter().filter(|id| *id != own));
        for id in order {
            let value = Self::stored(self.schema, &after, id);
            if value != Self::stored(self.schema, &before, id) {
                self.push(EventKind::Commit, id, value);
            }
        }
        self.notify(changed);
        Ok(Committed {
            changed,
            apply: self.latest(changed),
        })
    }

    // ---- Loading and observing --------------------------------------------

    /// Read every word (repairing damaged ones) and subscribe to changes.
    /// Returns the properties whose values differ from before.
    pub fn load<B: Backend + ?Sized>(&mut self, backend: &mut B) -> Result<PropertySet, Error> {
        let changed = self.read_all(backend, false)?;
        if self.watch.is_none()
            && let Ok(token) = backend.watch()
        {
            self.watch = Some(token);
        }
        Ok(changed)
    }
    /// Read every word again and report what changed (`observed` events).
    pub fn refresh<B: Backend + ?Sized>(&mut self, backend: &mut B) -> Result<PropertySet, Error> {
        self.read_all(backend, true)
    }
    /// Cross-process observation: poll the storage subscription and re-read
    /// on a new change sequence. A lost subscription (service restart,
    /// reconnected backend) is renewed and followed by a full re-read.
    pub fn poll<B: Backend + ?Sized>(&mut self, backend: &mut B) -> Result<PropertySet, Error> {
        if let Some(token) = self.watch {
            match backend.poll(token) {
                Ok(state) if !state.gone => {
                    if state.changed || state.overflow || state.sequence != self.sequence {
                        self.sequence = state.sequence;
                        return self.refresh(backend);
                    }
                    return Ok(PropertySet::EMPTY);
                }
                _ => self.watch = None,
            }
        }
        let token = backend.watch().map_err(Error::Backend)?;
        self.watch = Some(token);
        self.sequence = backend.poll(token).map_or(0, |s| s.sequence);
        self.refresh(backend)
    }
    /// Forget the subscription (the backend connection was replaced).
    pub fn disconnected(&mut self) {
        self.watch = None;
    }
    fn read_all<B: Backend + ?Sized>(
        &mut self,
        backend: &mut B,
        observe: bool,
    ) -> Result<PropertySet, Error> {
        let mut changed = PropertySet::EMPTY;
        for index in 0..self.schema.words.len() {
            changed = changed.union(self.read_word(backend, index, observe)?);
        }
        Ok(changed)
    }
    fn read_word<B: Backend + ?Sized>(
        &mut self,
        backend: &mut B,
        index: usize,
        observe: bool,
    ) -> Result<PropertySet, Error> {
        let word = self.schema.words[index];
        let previous = self.status[index];
        let (value, status) = match backend.read(word.store) {
            Ok(value) if self.word_ok(index, value) => (value, Status::Ok),
            Ok(_) | Err(BackendError::Invalid) => {
                let rewritten =
                    self.caller.may(word.write) && backend.write(word.store, word.default).is_ok();
                if previous != Status::Damaged {
                    self.reset(index, ResetReason::Corrupt, rewritten);
                }
                (
                    word.default,
                    if rewritten {
                        Status::Ok
                    } else {
                        Status::Damaged
                    },
                )
            }
            Err(BackendError::NotFound) => (word.default, Status::Ok),
            Err(BackendError::Corrupt) => {
                if previous != Status::StorageCorrupt {
                    self.reset(index, ResetReason::StorageCorrupt, false);
                }
                (word.default, Status::StorageCorrupt)
            }
            Err(BackendError::Denied) => (word.default, Status::Unreadable),
            Err(error) => return Err(Error::Backend(error)),
        };
        self.status[index] = status;
        let before = self.words;
        self.words[index] = value;
        let after = self.words;
        let changed = self.diff(&before, &after);
        if observe {
            for id in changed.iter() {
                let value = Self::stored(self.schema, &after, id);
                if value != Self::stored(self.schema, &before, id) {
                    self.push(EventKind::Observed, id, value);
                }
            }
        }
        self.notify(changed);
        Ok(changed)
    }
    fn reset(&mut self, index: usize, reason: ResetReason, rewritten: bool) {
        let mut set = PropertySet::EMPTY;
        for id in self.word_properties(index) {
            set.insert(id);
        }
        for id in set.iter() {
            self.push(EventKind::Reset(reason), id, self.property(id).default);
        }
        self.notice.reset = self.notice.reset.union(set);
        if rewritten {
            self.notice.rewritten = self.notice.rewritten.union(set);
        }
        if reason == ResetReason::StorageCorrupt {
            self.notice.storage_corrupt = true;
        }
    }

    /// The pending "settings were reset" notice, if any.
    pub fn notice(&self) -> Option<Notice> {
        (!self.notice.reset.is_empty()).then_some(self.notice)
    }
    pub fn dismiss_notice(&mut self) {
        self.notice = Notice::default();
    }

    // ---- In-process subscribers and diagnostics ----------------------------

    /// Observe `filter` (use `PropertySet::ALL` for everything).
    pub fn subscribe(&mut self, filter: PropertySet) -> Result<Subscription, Error> {
        let slot = self
            .subscribers
            .iter()
            .position(|s| !s.live)
            .ok_or(Error::Capacity)?;
        self.generation = self.generation.wrapping_add(1);
        self.subscribers[slot] = Subscriber {
            live: true,
            generation: self.generation,
            filter,
            pending: PropertySet::EMPTY,
        };
        Ok(Subscription {
            slot: slot as u8,
            generation: self.generation,
        })
    }
    /// Changes since the last call (empty for an ended subscription).
    pub fn take_changes(&mut self, subscription: Subscription) -> PropertySet {
        match self.subscriber(subscription) {
            Some(s) => core::mem::take(&mut s.pending),
            None => PropertySet::EMPTY,
        }
    }
    pub fn unsubscribe(&mut self, subscription: Subscription) {
        if let Some(s) = self.subscriber(subscription) {
            s.live = false;
        }
    }
    fn subscriber(&mut self, subscription: Subscription) -> Option<&mut Subscriber> {
        self.subscribers
            .get_mut(usize::from(subscription.slot))
            .filter(|s| s.live && s.generation == subscription.generation)
    }
    fn notify(&mut self, changed: PropertySet) {
        for s in self.subscribers.iter_mut().filter(|s| s.live) {
            s.pending = s.pending.union(changed.intersection(s.filter));
        }
    }
    fn push(&mut self, kind: EventKind, id: PropertyId, value: u32) {
        let event = Event {
            kind,
            key: self.property(id).key,
            value,
        };
        if self.queued == EVENT_CAPACITY {
            self.head = (self.head + 1) % EVENT_CAPACITY;
            self.queued -= 1;
            self.lost = self.lost.saturating_add(1);
        }
        self.events[(self.head + self.queued) % EVENT_CAPACITY] = event;
        self.queued += 1;
    }
    /// Hand queued diagnostics to `sink` in order (print each with
    /// `door_rt::diagnostic`).
    pub fn drain_events(&mut self, mut sink: impl FnMut(Event)) {
        while self.queued > 0 {
            let event = self.events[self.head];
            self.head = (self.head + 1) % EVENT_CAPACITY;
            self.queued -= 1;
            sink(event);
        }
    }
    /// Events dropped because the queue was full.
    pub fn events_lost(&self) -> u32 {
        self.lost
    }

    // ---- Search -------------------------------------------------------------

    /// Properties whose title, keywords or key contain `query` (ASCII, any
    /// case), as `caller` may see them: machine settings it cannot change
    /// are read-only, private ones are omitted, unreadable ones never
    /// appear. An empty query finds nothing.
    pub fn search<'a>(&'a self, query: &'a str, caller: Caller) -> impl Iterator<Item = Hit> + 'a {
        let query = query.trim();
        self.properties().filter_map(move |id| {
            let p = self.property(id);
            let index = usize::from(p.word);
            let may = caller.may(self.schema.words[index].write);
            let listed = match p.visibility {
                Visibility::Public => true,
                Visibility::Private => may,
                Visibility::Internal => false,
            };
            let matched = !query.is_empty()
                && (door_settings_map::contains(p.title, query)
                    || door_settings_map::contains(p.keywords, query)
                    || door_settings_map::contains(p.key, query));
            (listed && matched && self.status[index] != Status::Unreadable).then_some(Hit {
                id,
                key: p.key,
                title: p.title,
                page: p.page,
                anchor: p.anchor,
                read_only: !may || self.status[index] == Status::StorageCorrupt,
            })
        })
    }
}
