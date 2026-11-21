//! door.accessibility broker. The window server owns focus, the lock state
//! and the authenticated peer identities, so it brokers semantic trees:
//!
//! * providers stage and publish trees for windows they own; every message
//!   is fully validated (`door_accessibility::wire`) before it is visible;
//! * only clients authorized as assistive technology (decided by the
//!   integration from the kernel identity, never from request bytes) may
//!   read trees, follow focus or invoke actions;
//! * secure surfaces (sign-in, lock, elevation prompts) read as a single
//!   placeholder node, and protected (password) content is always stripped;
//! * a locked session denies every read and action, drops frozen snapshots
//!   and pending actions, and tells assistive clients access was revoked.
//!
//! The broker is a pure state machine: the integration supplies ownership,
//! secure-window and focus facts plus section snapshots through `Context`,
//! and delivers the queued events from `take_event`.
extern crate alloc;
use crate::shell_semantics::{self, Caption};
use alloc::{collections::BTreeMap, vec::Vec};
use door_accessibility::{NodeId, Tree, wire};
use door_service_protocol::accessibility as a;
use door_service_protocol::window::Event;
use door_service_protocol::{BODY_BYTES, Code};

/// Kernel-authenticated client identity (pid, session, instance).
pub type ClientKey = (u64, u64, u64);
/// Owner of the trees the window server publishes itself (shell surfaces).
/// No client identity can equal it: kernel pids and instances are nonzero.
pub const SERVER: ClientKey = (0, 0, 0);
/// Trees one provider may hold at once (one per window it owns).
pub const MAX_TREES_PER_CLIENT: usize = 16;
/// Nodes across all of one provider's trees (each tree is at most
/// `wire::MAX_NODES`); a commit past either bound answers `Resources`.
pub const MAX_NODES_PER_CLIENT: usize = 16_384;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Peer {
    pub key: ClientKey,
    /// The identity holds the assistive-technology grant.
    pub assistive: bool,
}

/// Facts the broker needs from the window authority.
pub trait Context {
    fn owns(&self, client: ClientKey, window: u64) -> bool;
    /// Trusted surfaces whose content is never exposed.
    fn secure(&self, window: u64) -> bool;
    fn focused_window(&self) -> Option<u64>;
    /// Copy `out.len()` bytes from offset 0.. of a transferred section.
    fn snapshot(&mut self, section: u64, offset: u64, out: &mut [u8]) -> Result<(), Code>;
    /// The title bar the server draws for `window`, merged into what readers
    /// see (providers never publish it).
    fn caption(&self, _window: u64) -> Option<Caption> {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reply {
    Empty,
    Data(Vec<u8>),
    /// INVOKE of a server-owned node (shell surface or caption button): the
    /// integration performs `action` on `node` of `window`, answers the
    /// reader with `[serial u32]` and reports the outcome with
    /// `server_action_done`.
    Server {
        window: u64,
        node: u64,
        action: u32,
        serial: u32,
    },
}

/// A published tree, retained as its validated, redacted full encoding: one
/// allocation however many nodes it has. A decoded `Tree` costs several
/// allocations per node (strings, child lists, map nodes), and every tree of
/// every client lives as long as its window, so the broker decodes only
/// transiently (deltas, reads, invokes) and keeps the facts it consults on
/// every focus change beside the bytes.
struct Published {
    owner: ClientKey,
    version: u32,
    root: NodeId,
    focus: Option<NodeId>,
    nodes: usize,
    bytes: Vec<u8>,
}
impl Published {
    /// A tree whose full encoding exceeds the wire bound answers `Resources`
    /// (no reader could snapshot it either).
    fn new(owner: ClientKey, tree: &Tree) -> Result<Self, Code> {
        let mut bytes = wire::encode_full(tree).map_err(|_| Code::Resources)?;
        bytes.shrink_to_fit();
        Ok(Self {
            owner,
            version: tree.version,
            root: tree.root,
            focus: tree.focus,
            nodes: tree.len(),
            bytes,
        })
    }
    fn tree(&self) -> Result<Tree, Code> {
        match wire::decode(&self.bytes) {
            Ok(wire::Message::Full(tree)) => Ok(tree),
            _ => Err(Code::Corrupt),
        }
    }
}
struct Staging {
    owner: ClientKey,
    publish: a::Publish,
    data: Vec<u8>,
}
struct Snapshot {
    reader: ClientKey,
    serial: u32,
    bytes: Vec<u8>,
}
struct Pending {
    serial: u32,
    reader: ClientKey,
    provider: ClientKey,
    request: Vec<u8>,
}

/// Events queued per client (bounded; state events coalesce).
const MAX_EVENTS: usize = 32;

#[derive(Default)]
pub struct Broker {
    trees: BTreeMap<u64, Published>,
    staging: Vec<Staging>,
    snapshots: Vec<Snapshot>,
    pending: Vec<Pending>,
    assistive: Vec<ClientKey>,
    events: Vec<(ClientKey, Event)>,
    locked: bool,
    serial: u32,
    focus: Option<(u64, u64)>,
}

impl Broker {
    /// HELLO: the accessibility capabilities granted to `peer` out of
    /// `requested`. CAP_ASSISTIVE needs the identity grant.
    pub fn hello(&mut self, peer: &Peer, requested: u32) -> u32 {
        let mut granted = requested & a::CAP_ACCESSIBILITY;
        if requested & a::CAP_ASSISTIVE != 0 && peer.assistive {
            granted |= a::CAP_ASSISTIVE;
            if !self.assistive.contains(&peer.key) {
                self.assistive.push(peer.key);
            }
        }
        granted
    }
    pub fn locked(&self) -> bool {
        self.locked
    }
    /// An authorized assistive client is connected and may read now (the
    /// server builds its shell trees only then).
    pub fn has_readers(&self) -> bool {
        !self.assistive.is_empty() && !self.locked
    }
    fn next_serial(&mut self) -> u32 {
        self.serial = self.serial.wrapping_add(1).max(1);
        self.serial
    }
    fn push(&mut self, to: ClientKey, e: Event) {
        // Coalesce state events: the newest TREE_CHANGED/FOCUS_CHANGED per
        // window and the newest ACCESS_CHANGED replace older ones.
        if matches!(
            e.kind,
            a::TREE_CHANGED | a::FOCUS_CHANGED | a::ACCESS_CHANGED
        ) {
            self.events.retain(|(k, old)| {
                !(*k == to
                    && old.kind == e.kind
                    && (e.kind == a::ACCESS_CHANGED || old.window == e.window))
            });
        }
        // A full queue drops the client's oldest re-readable state event
        // (TREE_CHANGED/FOCUS_CHANGED: a reader resyncs from SNAPSHOT), then
        // its oldest data event. ACCESS_CHANGED (one per client) is never
        // dropped: a lost unlock would leave the reader paused for good.
        if e.kind != a::ACCESS_CHANGED
            && self
                .events
                .iter()
                .filter(|(k, old)| *k == to && old.kind != a::ACCESS_CHANGED)
                .count()
                >= MAX_EVENTS
            && let Some(i) = self
                .events
                .iter()
                .position(|(k, old)| {
                    *k == to && matches!(old.kind, a::TREE_CHANGED | a::FOCUS_CHANGED)
                })
                .or_else(|| {
                    self.events
                        .iter()
                        .position(|(k, old)| *k == to && old.kind != a::ACCESS_CHANGED)
                })
        {
            self.events.remove(i);
        }
        self.events.push((to, e));
    }
    /// Next event for `client`, in order.
    pub fn take_event(&mut self, client: ClientKey) -> Option<Event> {
        let i = self.events.iter().position(|(k, _)| *k == client)?;
        Some(self.events.remove(i).1)
    }
    pub fn pending_events(&self, client: ClientKey) -> bool {
        self.events.iter().any(|(k, _)| *k == client)
    }
    fn broadcast(&mut self, e: Event) {
        if self.locked {
            return;
        }
        for k in self.assistive.clone() {
            self.push(k, e);
        }
    }
    fn authorize(&self, peer: &Peer) -> Result<(), Code> {
        if !peer.assistive || !self.assistive.contains(&peer.key) || self.locked {
            return Err(Code::Denied);
        }
        Ok(())
    }
    /// Re-evaluate focus after a window activation change or a publish.
    pub fn focus_changed(&mut self, cx: &dyn Context) {
        let now = cx.focused_window().map(|w| {
            let node = if cx.secure(w) {
                0
            } else {
                self.trees.get(&w).and_then(|p| p.focus).map_or(0, |n| n.0)
            };
            (w, node)
        });
        if now != self.focus {
            self.focus = now;
            if let Some((window, node)) = now {
                let version = self.trees.get(&window).map_or(0, |p| p.version);
                self.broadcast(Event {
                    kind: a::FOCUS_CHANGED,
                    window,
                    value: node,
                    a: version,
                    ..Event::default()
                });
            }
        }
    }
    /// Lock or unlock the session.
    pub fn set_locked(&mut self, locked: bool, cx: &dyn Context) {
        if locked == self.locked {
            return;
        }
        if locked {
            self.snapshots.clear();
            for p in core::mem::take(&mut self.pending) {
                self.push(
                    p.reader,
                    Event {
                        kind: a::ACTION_RESULT,
                        value: u64::from(p.serial),
                        a: a::RESULT_GONE,
                        ..Event::default()
                    },
                );
            }
            for k in self.assistive.clone() {
                self.events.retain(|(c, e)| {
                    !(*c == k && matches!(e.kind, a::TREE_CHANGED | a::FOCUS_CHANGED))
                });
                self.push(
                    k,
                    Event {
                        kind: a::ACCESS_CHANGED,
                        a: 0,
                        ..Event::default()
                    },
                );
            }
            self.locked = true;
            self.focus = None;
        } else {
            self.locked = false;
            for k in self.assistive.clone() {
                self.push(
                    k,
                    Event {
                        kind: a::ACCESS_CHANGED,
                        a: 1,
                        ..Event::default()
                    },
                );
            }
            let windows: Vec<(u64, u32)> =
                self.trees.iter().map(|(w, p)| (*w, p.version)).collect();
            for (window, version) in windows {
                self.broadcast(Event {
                    kind: a::TREE_CHANGED,
                    window,
                    a: version,
                    ..Event::default()
                });
            }
            // Readers learn the current focus afresh.
            self.focus = None;
            self.focus_changed(cx);
        }
    }
    pub fn window_closed(&mut self, window: u64, cx: &dyn Context) {
        if self.trees.remove(&window).is_some() {
            self.broadcast(Event {
                kind: a::TREE_CHANGED,
                window,
                a: 0,
                ..Event::default()
            });
        }
        self.staging.retain(|s| s.publish.window != window);
        let gone: Vec<Pending> = self
            .pending
            .iter()
            .filter(|p| a::ActionRequest::decode(&p.request).is_ok_and(|r| r.window == window))
            .map(|p| Pending {
                serial: p.serial,
                reader: p.reader,
                provider: p.provider,
                request: Vec::new(),
            })
            .collect();
        for p in gone {
            self.pending.retain(|q| q.serial != p.serial);
            self.push(
                p.reader,
                Event {
                    kind: a::ACTION_RESULT,
                    value: u64::from(p.serial),
                    a: a::RESULT_GONE,
                    ..Event::default()
                },
            );
        }
        self.focus_changed(cx);
    }
    /// A client disconnected or was revoked.
    pub fn client_gone(&mut self, client: ClientKey, cx: &dyn Context) {
        let windows: Vec<u64> = self
            .trees
            .iter()
            .filter(|(_, p)| p.owner == client)
            .map(|(w, _)| *w)
            .collect();
        for w in windows {
            self.window_closed(w, cx);
        }
        self.staging.retain(|s| s.owner != client);
        self.snapshots.retain(|s| s.reader != client);
        self.assistive.retain(|k| *k != client);
        self.events.retain(|(k, _)| *k != client);
        for p in self.pending.iter().filter(|p| p.provider == client) {
            self.events.push((
                p.reader,
                Event {
                    kind: a::ACTION_RESULT,
                    value: u64::from(p.serial),
                    a: a::RESULT_GONE,
                    ..Event::default()
                },
            ));
        }
        self.pending
            .retain(|p| p.provider != client && p.reader != client);
    }
    /// The published tree of a window, decoded (integration and tests).
    pub fn tree(&self, window: u64) -> Option<Tree> {
        self.trees.get(&window).and_then(|p| p.tree().ok())
    }
    /// Publish a tree the server builds itself (a shell surface). Its
    /// version is assigned here: one more than the published one, and an
    /// unchanged tree publishes nothing. Returns whether it changed.
    pub fn publish_server(&mut self, mut tree: Tree, cx: &dyn Context) -> bool {
        let window = tree.window;
        if tree.validate().is_err() {
            return false;
        }
        let version = match self.trees.get(&window) {
            Some(p) if p.owner != SERVER => return false,
            Some(p) => {
                tree.version = p.version;
                // Encoding is canonical (every field, reading order), so an
                // unchanged tree encodes to the retained bytes.
                if wire::encode_full(&tree).is_ok_and(|b| b == p.bytes) {
                    return false;
                }
                p.version.wrapping_add(1).max(1)
            }
            None => 1,
        };
        tree.version = version;
        let Ok(published) = Published::new(SERVER, &tree) else {
            return false;
        };
        self.trees.insert(window, published);
        self.broadcast(Event {
            kind: a::TREE_CHANGED,
            window,
            a: version,
            ..Event::default()
        });
        self.focus_changed(cx);
        true
    }
    /// Withdraw a server tree (the surface closed).
    pub fn withdraw_server(&mut self, window: u64, cx: &dyn Context) {
        if self.trees.get(&window).is_some_and(|p| p.owner == SERVER) {
            self.window_closed(window, cx);
        }
    }
    /// Report a server-performed action (`Reply::Server`) to its reader.
    pub fn server_action_done(&mut self, reader: ClientKey, serial: u32, result: u32) {
        self.push(
            reader,
            Event {
                kind: a::ACTION_RESULT,
                value: u64::from(serial),
                a: result,
                ..Event::default()
            },
        );
    }
    /// Trees and nodes `owner` holds, not counting `except`'s tree.
    fn usage(&self, owner: ClientKey, except: u64) -> (usize, usize) {
        self.trees
            .iter()
            .filter(|(w, p)| p.owner == owner && **w != except)
            .fold((0, 0), |(t, n), (_, p)| (t + 1, n + p.nodes))
    }
    fn within_bounds(&self, owner: ClientKey, window: u64, nodes: usize) -> Result<(), Code> {
        let (trees, held) = self.usage(owner, window);
        if trees >= MAX_TREES_PER_CLIENT || held + nodes > MAX_NODES_PER_CLIENT {
            return Err(Code::Resources);
        }
        Ok(())
    }

    fn commit(&mut self, peer: &Peer, cx: &dyn Context) -> Result<Reply, Code> {
        let i = self
            .staging
            .iter()
            .position(|s| s.owner == peer.key)
            .ok_or(Code::InvalidRequest)?;
        let staged = self.staging.remove(i);
        if staged.data.len() != staged.publish.bytes as usize {
            return Err(Code::InvalidRequest);
        }
        let p = staged.publish;
        if !cx.owns(peer.key, p.window) {
            return Err(Code::Denied);
        }
        let message = wire::decode(&staged.data).map_err(|_| Code::InvalidRequest)?;
        if message.window() != p.window || message.version() != p.version {
            return Err(Code::InvalidRequest);
        }
        match message {
            wire::Message::Full(mut tree) => {
                if p.kind != a::KIND_FULL {
                    return Err(Code::InvalidRequest);
                }
                self.within_bounds(peer.key, p.window, tree.len())?;
                tree.redact_protected();
                let published = Published::new(peer.key, &tree)?;
                self.trees.insert(p.window, published);
            }
            wire::Message::Delta(delta) => {
                if p.kind != a::KIND_DELTA {
                    return Err(Code::InvalidRequest);
                }
                let published = self
                    .trees
                    .get(&p.window)
                    .filter(|t| t.owner == peer.key)
                    .ok_or(Code::Changed)?;
                if published.version != delta.base_version {
                    return Err(Code::Changed);
                }
                // A decoded copy: a delta that fails, or passes the client's
                // node budget, leaves the retained encoding untouched.
                let mut next = published.tree()?;
                next.apply(&delta).map_err(|_| Code::InvalidRequest)?;
                self.within_bounds(peer.key, p.window, next.len())?;
                next.redact_protected();
                let published = Published::new(peer.key, &next)?;
                self.trees.insert(p.window, published);
            }
        }
        self.broadcast(Event {
            kind: a::TREE_CHANGED,
            window: p.window,
            a: p.version,
            ..Event::default()
        });
        self.focus_changed(cx);
        Ok(Reply::Data(p.version.to_le_bytes().to_vec()))
    }
    fn readable(&self, window: u64, cx: &dyn Context) -> Result<Tree, Code> {
        let published = self.trees.get(&window).ok_or(Code::NotFound)?;
        if cx.secure(window) {
            return Ok(wire::placeholder(window, published.version, published.root));
        }
        let mut t = published.tree()?;
        t.redact_protected();
        if let Some(caption) = cx.caption(window) {
            shell_semantics::merge_caption(&mut t, &caption);
        }
        Ok(t)
    }

    /// Handle one request in `a::FIRST..=a::LAST`.
    pub fn request(
        &mut self,
        peer: &Peer,
        op: u32,
        b: &[u8],
        sections: &[u64],
        cx: &mut dyn Context,
    ) -> Result<Reply, Code> {
        if op != a::PUBLISH_SECTION && !sections.is_empty() {
            return Err(Code::InvalidRequest);
        }
        let word = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let long = |at: usize| u64::from_le_bytes(b[at..at + 8].try_into().unwrap());
        match op {
            a::PUBLISH_BEGIN | a::PUBLISH_SECTION => {
                let p = a::Publish::decode(b)?;
                if !cx.owns(peer.key, p.window) {
                    return Err(Code::Denied);
                }
                self.staging.retain(|s| s.owner != peer.key);
                let mut data = Vec::new();
                data.try_reserve_exact(p.bytes as usize)
                    .map_err(|_| Code::Resources)?;
                if op == a::PUBLISH_SECTION {
                    let [section] = sections else {
                        return Err(Code::InvalidRequest);
                    };
                    data.resize(p.bytes as usize, 0);
                    cx.snapshot(*section, 0, &mut data)?;
                }
                self.staging.push(Staging {
                    owner: peer.key,
                    publish: p,
                    data,
                });
                Ok(Reply::Empty)
            }
            a::PUBLISH_CHUNK => {
                if b.len() < 4 || b.len() > 4 + a::CHUNK_BYTES {
                    return Err(Code::InvalidRequest);
                }
                let s = self
                    .staging
                    .iter_mut()
                    .find(|s| s.owner == peer.key)
                    .ok_or(Code::InvalidRequest)?;
                let offset = word(0) as usize;
                let chunk = &b[4..];
                if offset != s.data.len()
                    || offset + chunk.len() > s.publish.bytes as usize
                    || chunk.is_empty()
                {
                    return Err(Code::InvalidRequest);
                }
                s.data.extend_from_slice(chunk);
                Ok(Reply::Empty)
            }
            a::PUBLISH_COMMIT => {
                if !b.is_empty() {
                    return Err(Code::InvalidRequest);
                }
                self.commit(peer, cx)
            }
            a::WITHDRAW => {
                if b.len() != 8 {
                    return Err(Code::InvalidRequest);
                }
                let window = long(0);
                if !cx.owns(peer.key, window) {
                    return Err(Code::Denied);
                }
                self.window_closed(window, cx);
                Ok(Reply::Empty)
            }
            a::ACTION_GET => {
                if b.len() != 4 {
                    return Err(Code::InvalidRequest);
                }
                let serial = word(0);
                let p = self
                    .pending
                    .iter()
                    .find(|p| p.serial == serial && p.provider == peer.key)
                    .ok_or(Code::InvalidRequest)?;
                Ok(Reply::Data(p.request.clone()))
            }
            a::ACTION_DONE => {
                if b.len() != 8 || word(4) > a::RESULT_GONE {
                    return Err(Code::InvalidRequest);
                }
                let serial = word(0);
                let i = self
                    .pending
                    .iter()
                    .position(|p| p.serial == serial && p.provider == peer.key)
                    .ok_or(Code::InvalidRequest)?;
                let p = self.pending.remove(i);
                self.push(
                    p.reader,
                    Event {
                        kind: a::ACTION_RESULT,
                        value: u64::from(serial),
                        a: word(4),
                        ..Event::default()
                    },
                );
                Ok(Reply::Empty)
            }
            a::FOCUS_GET => {
                self.authorize(peer)?;
                if !b.is_empty() {
                    return Err(Code::InvalidRequest);
                }
                let f = match cx.focused_window() {
                    Some(w) => {
                        let secure = cx.secure(w);
                        let t = self.trees.get(&w);
                        a::Focus {
                            window: w,
                            node: if secure {
                                0
                            } else {
                                t.and_then(|t| t.focus).map_or(0, |n| n.0)
                            },
                            version: t.map_or(0, |t| t.version),
                            flags: a::WINDOW_FOCUSED | if secure { a::WINDOW_SECURE } else { 0 },
                        }
                    }
                    None => a::Focus::default(),
                };
                Ok(Reply::Data(f.encode().to_vec()))
            }
            a::SNAPSHOT => {
                self.authorize(peer)?;
                let s = a::Snapshot::decode(b)?;
                let tree = self.readable(s.window, cx)?;
                let tree = match s.scope {
                    a::SCOPE_TREE => tree,
                    a::SCOPE_FOCUS => {
                        let f = tree.focus.ok_or(Code::NotFound)?;
                        wire::single(&tree, f).ok_or(Code::NotFound)?
                    }
                    _ => wire::single(&tree, NodeId(s.node)).ok_or(Code::NotFound)?,
                };
                let bytes = wire::encode_full(&tree).map_err(|_| Code::Corrupt)?;
                let serial = self.next_serial();
                let mine = self
                    .snapshots
                    .iter()
                    .filter(|x| x.reader == peer.key)
                    .count();
                if mine >= a::MAX_SNAPSHOTS
                    && let Some(i) = self.snapshots.iter().position(|x| x.reader == peer.key)
                {
                    self.snapshots.remove(i);
                }
                let len = bytes.len() as u32;
                self.snapshots.push(Snapshot {
                    reader: peer.key,
                    serial,
                    bytes,
                });
                let mut out = serial.to_le_bytes().to_vec();
                out.extend_from_slice(&len.to_le_bytes());
                Ok(Reply::Data(out))
            }
            a::READ => {
                self.authorize(peer)?;
                if b.len() != 8 {
                    return Err(Code::InvalidRequest);
                }
                let (serial, offset) = (word(0), word(4) as usize);
                let s = self
                    .snapshots
                    .iter()
                    .find(|s| s.serial == serial && s.reader == peer.key)
                    .ok_or(Code::Changed)?;
                if offset > s.bytes.len() {
                    return Err(Code::InvalidRequest);
                }
                let end = (offset + a::READ_BYTES).min(s.bytes.len());
                Ok(Reply::Data(s.bytes[offset..end].to_vec()))
            }
            a::INVOKE => {
                self.authorize(peer)?;
                let r = a::ActionRequest::decode(b)?;
                if cx.secure(r.window) {
                    return Err(Code::Denied);
                }
                let published = self.trees.get(&r.window).ok_or(Code::NotFound)?;
                // The server-drawn caption buttons answer Invoke only.
                if shell_semantics::is_caption_node(r.node) && cx.caption(r.window).is_some() {
                    if r.action != a::ACTION_INVOKE || r.node == shell_semantics::CAPTION_BAR {
                        return Err(Code::Denied);
                    }
                    let serial = self.next_serial();
                    return Ok(Reply::Server {
                        window: r.window,
                        node: r.node,
                        action: r.action,
                        serial,
                    });
                }
                let provider = published.owner;
                let tree = published.tree()?;
                let node = tree.get(NodeId(r.node)).ok_or(Code::NotFound)?;
                let flag = door_accessibility::Actions(1 << r.action);
                if !node.actions.contains(flag) || node.is(door_accessibility::States::DISABLED) {
                    return Err(Code::Denied);
                }
                drop(tree);
                if provider == SERVER {
                    let serial = self.next_serial();
                    return Ok(Reply::Server {
                        window: r.window,
                        node: r.node,
                        action: r.action,
                        serial,
                    });
                }
                if self.pending.len() >= a::MAX_PENDING_ACTIONS {
                    return Err(Code::WouldBlock);
                }
                let serial = self.next_serial();
                self.pending.push(Pending {
                    serial,
                    reader: peer.key,
                    provider,
                    request: b.to_vec(),
                });
                self.push(
                    provider,
                    Event {
                        kind: a::ACTION,
                        window: r.window,
                        value: r.node,
                        a: r.action,
                        b: serial,
                        ..Event::default()
                    },
                );
                Ok(Reply::Data(serial.to_le_bytes().to_vec()))
            }
            a::WINDOWS => {
                self.authorize(peer)?;
                if !b.is_empty() {
                    return Err(Code::InvalidRequest);
                }
                let focused = cx.focused_window();
                let max = (BODY_BYTES - 4) / 16;
                let list: Vec<(u64, u32, u32)> = self
                    .trees
                    .iter()
                    .take(max)
                    .map(|(w, p)| {
                        let flags = if Some(*w) == focused {
                            a::WINDOW_FOCUSED
                        } else {
                            0
                        } | if cx.secure(*w) { a::WINDOW_SECURE } else { 0 };
                        (*w, p.version, flags)
                    })
                    .collect();
                let mut out = (list.len() as u32).to_le_bytes().to_vec();
                for (w, v, f) in list {
                    out.extend_from_slice(&w.to_le_bytes());
                    out.extend_from_slice(&v.to_le_bytes());
                    out.extend_from_slice(&f.to_le_bytes());
                }
                Ok(Reply::Data(out))
            }
            _ => Err(Code::Unsupported),
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use door_accessibility::{Actions, Node, Role, States, TreeBuilder};
    use std::vec;

    struct World {
        owners: Vec<(ClientKey, u64)>,
        secure: Vec<u64>,
        focused: Option<u64>,
        section: Vec<u8>,
    }
    impl Context for World {
        fn owns(&self, c: ClientKey, w: u64) -> bool {
            self.owners.contains(&(c, w))
        }
        fn secure(&self, w: u64) -> bool {
            self.secure.contains(&w)
        }
        fn focused_window(&self) -> Option<u64> {
            self.focused
        }
        fn snapshot(&mut self, _s: u64, offset: u64, out: &mut [u8]) -> Result<(), Code> {
            let at = offset as usize;
            let src = self
                .section
                .get(at..at + out.len())
                .ok_or(Code::InvalidRequest)?;
            out.copy_from_slice(src);
            Ok(())
        }
    }
    const APP: Peer = Peer {
        key: (10, 1, 1),
        assistive: false,
    };
    const OTHER: Peer = Peer {
        key: (11, 1, 1),
        assistive: false,
    };
    const READER: Peer = Peer {
        key: (12, 1, 1),
        assistive: true,
    };
    fn tree(window: u64, version: u32, secret: bool) -> Tree {
        let mut b = TreeBuilder::new(
            window,
            version,
            Node::new(NodeId(1), Role::Window).named("Notes"),
        );
        let mut save = Node::new(NodeId(2), Role::Button).named("Save");
        save.actions = Actions::INVOKE;
        save.states = States::FOCUSABLE;
        b.push(save).unwrap();
        let mut pw = Node::new(NodeId(3), Role::Edit).named("Password");
        pw.states = States::PROTECTED | States::FOCUSABLE;
        if secret {
            pw.states = States::FOCUSABLE;
            pw.value = "hunter2".into();
        }
        b.push(pw).unwrap();
        b.finish(Some(NodeId(2)))
    }
    fn publish(
        br: &mut Broker,
        w: &mut dyn Context,
        peer: &Peer,
        bytes: &[u8],
    ) -> Result<Reply, Code> {
        let p =
            a::Publish::of_message(u64::from_le_bytes(bytes[8..16].try_into().unwrap()), bytes)?;
        br.request(peer, a::PUBLISH_BEGIN, &p.encode(), &[], w)?;
        for (off, len) in a::chunks(bytes.len()) {
            let mut body = (off as u32).to_le_bytes().to_vec();
            body.extend_from_slice(&bytes[off..off + len]);
            br.request(peer, a::PUBLISH_CHUNK, &body, &[], w)?;
        }
        br.request(peer, a::PUBLISH_COMMIT, &[], &[], w)
    }
    fn read_tree(
        br: &mut Broker,
        w: &mut dyn Context,
        peer: &Peer,
        window: u64,
        scope: u32,
        node: u64,
    ) -> Result<Tree, Code> {
        let s = a::Snapshot {
            window,
            scope,
            node,
        };
        let Reply::Data(r) = br.request(peer, a::SNAPSHOT, &s.encode(), &[], w)? else {
            panic!()
        };
        let serial = u32::from_le_bytes(r[..4].try_into().unwrap());
        let len = u32::from_le_bytes(r[4..8].try_into().unwrap()) as usize;
        let mut out = vec![];
        while out.len() < len {
            let mut q = serial.to_le_bytes().to_vec();
            q.extend_from_slice(&(out.len() as u32).to_le_bytes());
            let Reply::Data(chunk) = br.request(peer, a::READ, &q, &[], w)? else {
                panic!()
            };
            out.extend_from_slice(&chunk);
        }
        match wire::decode(&out).unwrap() {
            wire::Message::Full(t) => Ok(t),
            _ => panic!(),
        }
    }

    #[test]
    fn publishing_requires_ownership_and_valid_messages() {
        let mut br = Broker::default();
        let mut w = World {
            owners: vec![(APP.key, 5)],
            secure: vec![],
            focused: Some(5),
            section: vec![],
        };
        let bytes = wire::encode_full(&tree(5, 1, false)).unwrap();
        assert_eq!(
            publish(&mut br, &mut w, &OTHER, &bytes),
            Err(Code::Denied),
            "foreign window"
        );
        assert_eq!(
            publish(&mut br, &mut w, &APP, &bytes),
            Ok(Reply::Data(1u32.to_le_bytes().to_vec()))
        );
        // Out-of-order chunk and corrupted bytes are refused.
        let p = a::Publish::of_message(5, &bytes).unwrap();
        br.request(&APP, a::PUBLISH_BEGIN, &p.encode(), &[], &mut w)
            .unwrap();
        let mut body = 7u32.to_le_bytes().to_vec();
        body.extend_from_slice(&bytes[..10]);
        assert_eq!(
            br.request(&APP, a::PUBLISH_CHUNK, &body, &[], &mut w),
            Err(Code::InvalidRequest)
        );
        let mut bad = bytes.clone();
        bad[56] = 0xee;
        assert!(publish(&mut br, &mut w, &APP, &bad).is_err());
        // Section publishing snapshots the provider's bytes.
        let next = wire::encode_full(&tree(5, 2, false)).unwrap();
        w.section = next.clone();
        let p = a::Publish::of_message(5, &next).unwrap();
        br.request(&APP, a::PUBLISH_SECTION, &p.encode(), &[42], &mut w)
            .unwrap();
        assert!(
            br.request(&APP, a::PUBLISH_COMMIT, &[], &[], &mut w)
                .is_ok()
        );
        assert_eq!(br.tree(5).unwrap().version, 2);
        // A delta must match the published version.
        let mut t3 = tree(5, 3, false);
        t3.focus = Some(NodeId(3));
        let delta = t3.diff(&tree(5, 1, false));
        assert_eq!(
            publish(&mut br, &mut w, &APP, &wire::encode_delta(&delta).unwrap()),
            Err(Code::Changed)
        );
        let delta = t3.diff(&tree(5, 2, false));
        assert!(publish(&mut br, &mut w, &APP, &wire::encode_delta(&delta).unwrap()).is_ok());
        assert_eq!(br.tree(5).unwrap().focus, Some(NodeId(3)));
    }

    #[test]
    fn only_authorized_assistive_clients_read_and_secure_surfaces_are_redacted() {
        let mut br = Broker::default();
        let mut w = World {
            owners: vec![(APP.key, 5), (APP.key, 6)],
            secure: vec![6],
            focused: Some(5),
            section: vec![],
        };
        assert_eq!(
            br.hello(&APP, a::CAP_ACCESSIBILITY | a::CAP_ASSISTIVE),
            a::CAP_ACCESSIBILITY,
            "no grant, no assistive capability"
        );
        assert_eq!(br.hello(&READER, a::CAP_ASSISTIVE), a::CAP_ASSISTIVE);
        publish(
            &mut br,
            &mut w,
            &APP,
            &wire::encode_full(&tree(5, 1, false)).unwrap(),
        )
        .unwrap();
        publish(
            &mut br,
            &mut w,
            &APP,
            &wire::encode_full(&tree(6, 1, false)).unwrap(),
        )
        .unwrap();
        // Unauthorized readers are refused everywhere.
        for op in [a::FOCUS_GET, a::WINDOWS] {
            assert_eq!(br.request(&APP, op, &[], &[], &mut w), Err(Code::Denied));
        }
        assert_eq!(
            read_tree(&mut br, &mut w, &APP, 5, a::SCOPE_TREE, 0),
            Err(Code::Denied)
        );
        let t = read_tree(&mut br, &mut w, &READER, 5, a::SCOPE_TREE, 0).unwrap();
        assert_eq!(t.len(), 3);
        let focus = read_tree(&mut br, &mut w, &READER, 5, a::SCOPE_FOCUS, 0).unwrap();
        assert_eq!(focus.root().name, "Save");
        let secure = read_tree(&mut br, &mut w, &READER, 6, a::SCOPE_TREE, 0).unwrap();
        assert_eq!(
            (secure.len(), secure.root().name.as_str()),
            (1, "Secure surface")
        );
        let Reply::Data(f) = br.request(&READER, a::FOCUS_GET, &[], &[], &mut w).unwrap() else {
            panic!()
        };
        assert_eq!(
            a::Focus::decode(&f).unwrap(),
            a::Focus {
                window: 5,
                node: 2,
                version: 1,
                flags: a::WINDOW_FOCUSED
            }
        );
        // Focus events reach the assistive client only.
        assert!(br.take_event(APP.key).is_none());
        let mut kinds = vec![];
        while let Some(e) = br.take_event(READER.key) {
            kinds.push(e.kind);
        }
        assert!(kinds.contains(&a::TREE_CHANGED) && kinds.contains(&a::FOCUS_CHANGED));
        // Invoking a secure window's node is denied.
        let mut buf = [0u8; 64];
        let n = a::ActionRequest {
            window: 6,
            node: 2,
            action: a::ACTION_INVOKE,
            arg: &[],
        }
        .encode(&mut buf)
        .unwrap();
        assert_eq!(
            br.request(&READER, a::INVOKE, &buf[..n], &[], &mut w),
            Err(Code::Denied)
        );
    }

    #[test]
    fn protected_values_never_reach_readers() {
        let mut br = Broker::default();
        let mut w = World {
            owners: vec![(APP.key, 5)],
            secure: vec![],
            focused: Some(5),
            section: vec![],
        };
        br.hello(&READER, a::CAP_ASSISTIVE);
        // The wire format already refuses a provider publishing a protected
        // value; a provider that drops the PROTECTED flag is its own data,
        // but a flagged node is stripped even if a buggy encoder slipped one.
        let mut t = tree(5, 1, false);
        let pw = t.get_mut(NodeId(3)).unwrap();
        pw.value = "hunter2".into();
        assert!(wire::encode_full(&t).is_err());
        publish(
            &mut br,
            &mut w,
            &APP,
            &wire::encode_full(&tree(5, 1, false)).unwrap(),
        )
        .unwrap();
        let read = read_tree(&mut br, &mut w, &READER, 5, a::SCOPE_NODE, 3).unwrap();
        assert!(read.root().value.is_empty() && read.root().is(States::PROTECTED));
    }

    #[test]
    fn actions_are_brokered_and_locking_revokes_everything() {
        let mut br = Broker::default();
        let mut w = World {
            owners: vec![(APP.key, 5)],
            secure: vec![],
            focused: Some(5),
            section: vec![],
        };
        br.hello(&READER, a::CAP_ASSISTIVE);
        publish(
            &mut br,
            &mut w,
            &APP,
            &wire::encode_full(&tree(5, 1, false)).unwrap(),
        )
        .unwrap();
        let mut buf = [0u8; 64];
        // Unadvertised action.
        let n = a::ActionRequest {
            window: 5,
            node: 2,
            action: a::ACTION_TOGGLE,
            arg: &[],
        }
        .encode(&mut buf)
        .unwrap();
        assert_eq!(
            br.request(&READER, a::INVOKE, &buf[..n], &[], &mut w),
            Err(Code::Denied)
        );
        let n = a::ActionRequest {
            window: 5,
            node: 2,
            action: a::ACTION_INVOKE,
            arg: &[],
        }
        .encode(&mut buf)
        .unwrap();
        let Reply::Data(s) = br
            .request(&READER, a::INVOKE, &buf[..n], &[], &mut w)
            .unwrap()
        else {
            panic!()
        };
        let serial = u32::from_le_bytes(s[..4].try_into().unwrap());
        let e = br.take_event(APP.key).unwrap();
        assert_eq!(
            (e.kind, e.value, e.a, e.b),
            (a::ACTION, 2, a::ACTION_INVOKE, serial)
        );
        // Only the provider fetches and completes it.
        assert_eq!(
            br.request(&OTHER, a::ACTION_GET, &serial.to_le_bytes(), &[], &mut w),
            Err(Code::InvalidRequest)
        );
        let Reply::Data(req) = br
            .request(&APP, a::ACTION_GET, &serial.to_le_bytes(), &[], &mut w)
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(a::ActionRequest::decode(&req).unwrap().node, 2);
        let mut done = serial.to_le_bytes().to_vec();
        done.extend_from_slice(&a::RESULT_DONE.to_le_bytes());
        br.request(&APP, a::ACTION_DONE, &done, &[], &mut w)
            .unwrap();
        while let Some(e) = br.take_event(READER.key) {
            if e.kind == a::ACTION_RESULT {
                assert_eq!((e.value, e.a), (u64::from(serial), a::RESULT_DONE));
            }
        }
        // A pending action is cancelled by locking; every read is denied.
        let Reply::Data(s) = br
            .request(&READER, a::INVOKE, &buf[..n], &[], &mut w)
            .unwrap()
        else {
            panic!()
        };
        let pending = u32::from_le_bytes(s[..4].try_into().unwrap());
        let snap = a::Snapshot {
            window: 5,
            scope: a::SCOPE_TREE,
            node: 0,
        };
        let Reply::Data(r) = br
            .request(&READER, a::SNAPSHOT, &snap.encode(), &[], &mut w)
            .unwrap()
        else {
            panic!()
        };
        br.set_locked(true, &w);
        let mut q = r[..4].to_vec();
        q.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(
            br.request(&READER, a::READ, &q, &[], &mut w),
            Err(Code::Denied)
        );
        assert_eq!(
            br.request(&READER, a::FOCUS_GET, &[], &[], &mut w),
            Err(Code::Denied)
        );
        assert_eq!(
            br.request(&READER, a::SNAPSHOT, &snap.encode(), &[], &mut w),
            Err(Code::Denied)
        );
        let mut saw = (false, false);
        while let Some(e) = br.take_event(READER.key) {
            saw.0 |= e.kind == a::ACTION_RESULT
                && e.value == u64::from(pending)
                && e.a == a::RESULT_GONE;
            saw.1 |= e.kind == a::ACCESS_CHANGED && e.a == 0;
            assert!(
                e.kind != a::FOCUS_CHANGED && e.kind != a::TREE_CHANGED,
                "nothing leaks while locked"
            );
        }
        assert!(saw.0 && saw.1);
        // Publishing while locked stays possible, but readers learn nothing.
        let mut t2 = tree(5, 2, false);
        t2.focus = Some(NodeId(3));
        publish(&mut br, &mut w, &APP, &wire::encode_full(&t2).unwrap()).unwrap();
        assert!(br.take_event(READER.key).is_none());
        br.set_locked(false, &w);
        let mut kinds = vec![];
        while let Some(e) = br.take_event(READER.key) {
            kinds.push(e.kind);
        }
        assert!(kinds.contains(&a::ACCESS_CHANGED) && kinds.contains(&a::TREE_CHANGED));
        // Snapshots taken before the lock are gone for good.
        assert_eq!(
            br.request(&READER, a::READ, &q, &[], &mut w),
            Err(Code::Changed)
        );
        // A departed provider fails its pending actions and withdraws trees.
        let Reply::Data(_) = br
            .request(&READER, a::INVOKE, &buf[..n], &[], &mut w)
            .unwrap()
        else {
            panic!()
        };
        br.client_gone(APP.key, &w);
        assert!(br.tree(5).is_none());
    }

    /// A world whose windows carry server-drawn captions.
    struct Captioned(World);
    impl Context for Captioned {
        fn owns(&self, c: ClientKey, w: u64) -> bool {
            self.0.owns(c, w)
        }
        fn secure(&self, w: u64) -> bool {
            self.0.secure(w)
        }
        fn focused_window(&self) -> Option<u64> {
            self.0.focused_window()
        }
        fn snapshot(&mut self, s: u64, o: u64, out: &mut [u8]) -> Result<(), Code> {
            self.0.snapshot(s, o, out)
        }
        fn caption(&self, w: u64) -> Option<Caption> {
            let r = door_graphics_core::Rect::new(0, 0, 400, 300).unwrap();
            self.0
                .owners
                .iter()
                .any(|(_, x)| *x == w)
                .then(|| shell_semantics::caption("Notes", r, r, 32, false))
        }
    }

    #[test]
    fn storage_per_client_is_bounded() {
        let mut br = Broker::default();
        let owners = (1..=MAX_TREES_PER_CLIENT as u64 + 1)
            .map(|w| (APP.key, w))
            .collect();
        let mut w = World {
            owners,
            secure: vec![],
            focused: None,
            section: vec![],
        };
        for window in 1..=MAX_TREES_PER_CLIENT as u64 {
            publish(
                &mut br,
                &mut w,
                &APP,
                &wire::encode_full(&tree(window, 1, false)).unwrap(),
            )
            .unwrap();
        }
        let extra = MAX_TREES_PER_CLIENT as u64 + 1;
        assert_eq!(
            publish(
                &mut br,
                &mut w,
                &APP,
                &wire::encode_full(&tree(extra, 1, false)).unwrap()
            ),
            Err(Code::Resources)
        );
        // Republishing an owned window stays within the bound.
        publish(
            &mut br,
            &mut w,
            &APP,
            &wire::encode_full(&tree(1, 2, false)).unwrap(),
        )
        .unwrap();
        // Another client has its own budget; a withdrawn tree frees a slot.
        br.request(&APP, a::WITHDRAW, &1u64.to_le_bytes(), &[], &mut w)
            .unwrap();
        publish(
            &mut br,
            &mut w,
            &APP,
            &wire::encode_full(&tree(extra, 1, false)).unwrap(),
        )
        .unwrap();
        assert_eq!(
            br.usage(APP.key, 0),
            (MAX_TREES_PER_CLIENT, 3 * MAX_TREES_PER_CLIENT)
        );
    }

    #[test]
    fn server_trees_and_captions_are_read_and_invoked_through_the_server() {
        let mut br = Broker::default();
        let mut w = Captioned(World {
            owners: vec![(APP.key, 5)],
            secure: vec![],
            focused: Some(shell_semantics::START),
            section: vec![],
        });
        br.hello(&READER, a::CAP_ASSISTIVE);
        let mut b = TreeBuilder::new(
            shell_semantics::START,
            7,
            Node::new(NodeId(1), Role::Window).named("Start"),
        );
        let mut search = Node::new(NodeId(2), Role::Edit).named("Search box");
        search.states = States::FOCUSABLE;
        search.actions = Actions::INVOKE;
        b.push(search).unwrap();
        let t = b.finish(Some(NodeId(2)));
        assert!(br.publish_server(t.clone(), &w));
        assert!(
            !br.publish_server(t.clone(), &w),
            "unchanged trees publish nothing"
        );
        assert_eq!(br.tree(shell_semantics::START).map(|t| t.version), Some(1));
        let mut events = vec![];
        while let Some(e) = br.take_event(READER.key) {
            events.push((e.kind, e.window, e.value));
        }
        assert!(
            events.contains(&(a::FOCUS_CHANGED, shell_semantics::START, 2)),
            "{events:?}"
        );
        // Clients cannot publish into server windows.
        assert_eq!(
            publish(
                &mut br,
                &mut w.0,
                &APP,
                &wire::encode_full(&tree(shell_semantics::START, 9, false)).unwrap()
            ),
            Err(Code::Denied)
        );
        let mut buf = [0u8; 64];
        let n = a::ActionRequest {
            window: shell_semantics::START,
            node: 2,
            action: a::ACTION_INVOKE,
            arg: &[],
        }
        .encode(&mut buf)
        .unwrap();
        let Ok(Reply::Server {
            window,
            node,
            serial,
            ..
        }) = br.request(&READER, a::INVOKE, &buf[..n], &[], &mut w)
        else {
            panic!("server node routes to the server")
        };
        assert_eq!((window, node), (shell_semantics::START, 2));
        br.server_action_done(READER.key, serial, a::RESULT_DONE);
        assert_eq!(
            br.take_event(READER.key).map(|e| (e.kind, e.value)),
            Some((a::ACTION_RESULT, u64::from(serial)))
        );
        // A provider's tree reads back with the server's title bar first.
        publish(
            &mut br,
            &mut w.0,
            &APP,
            &wire::encode_full(&tree(5, 1, false)).unwrap(),
        )
        .unwrap();
        let read = read_tree(&mut br, &mut w.0, &READER, 5, a::SCOPE_TREE, 0).unwrap();
        assert!(
            read.get(NodeId(shell_semantics::CAPTION_CLOSE)).is_none(),
            "plain world: no caption"
        );
        let read = read_tree(&mut br, &mut w, &READER, 5, a::SCOPE_TREE, 0).unwrap();
        let bar = read.root().children[0];
        assert_eq!(bar, NodeId(shell_semantics::CAPTION_BAR));
        assert_eq!(read.get(bar).unwrap().children.len(), 3);
        assert_eq!(
            read.get(NodeId(shell_semantics::CAPTION_CLOSE))
                .unwrap()
                .name,
            "Close"
        );
        assert_eq!(
            br.tree(5).unwrap().len(),
            3,
            "the stored tree stays the provider's"
        );
        let n = a::ActionRequest {
            window: 5,
            node: shell_semantics::CAPTION_CLOSE,
            action: a::ACTION_INVOKE,
            arg: &[],
        }
        .encode(&mut buf)
        .unwrap();
        assert!(matches!(
            br.request(&READER, a::INVOKE, &buf[..n], &[], &mut w),
            Ok(Reply::Server {
                node: shell_semantics::CAPTION_CLOSE,
                ..
            })
        ));
        let n = a::ActionRequest {
            window: 5,
            node: shell_semantics::CAPTION_CLOSE,
            action: a::ACTION_TOGGLE,
            arg: &[],
        }
        .encode(&mut buf)
        .unwrap();
        assert_eq!(
            br.request(&READER, a::INVOKE, &buf[..n], &[], &mut w),
            Err(Code::Denied)
        );
        br.withdraw_server(shell_semantics::START, &w);
        assert!(br.tree(shell_semantics::START).is_none());
    }
    /// M5: with 40 trees, the unlock's TREE_CHANGED flood overflows the
    /// 32-event queue; ACCESS_CHANGED survives it (and comes first), so the
    /// reader never stays paused.
    #[test]
    fn unlock_with_40_trees_never_loses_access_changed() {
        let mut br = Broker::default();
        let w = World {
            owners: vec![],
            secure: vec![],
            focused: None,
            section: vec![],
        };
        br.hello(&READER, a::CAP_ASSISTIVE);
        for window in 1000..1040 {
            assert!(br.publish_server(tree(window, 1, false), &w));
        }
        br.set_locked(true, &w);
        br.set_locked(false, &w);
        let mut events = vec![];
        while let Some(e) = br.take_event(READER.key) {
            events.push(e);
        }
        assert_eq!((events[0].kind, events[0].a), (a::ACCESS_CHANGED, 1));
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind == a::ACCESS_CHANGED)
                .count(),
            1
        );
        assert!(events.len() <= MAX_EVENTS + 1);
        // The newest trees' changes are the ones kept.
        assert!(
            events
                .iter()
                .any(|e| e.kind == a::TREE_CHANGED && e.window == 1039)
        );
        // A full queue of state events still takes a new ACCESS_CHANGED.
        for window in 1000..1040 {
            br.window_closed(window, &w);
        }
        br.set_locked(true, &w);
        let last = core::iter::from_fn(|| br.take_event(READER.key)).last();
        assert_eq!(last.map(|e| (e.kind, e.a)), Some((a::ACCESS_CHANGED, 0)));
    }
}
