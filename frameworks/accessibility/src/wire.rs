//! door.accessibility tree wire format, version 1. Little endian, bounded,
//! self-delimiting; every field is validated before a tree is published.
//!
//! ```text
//! Header (40 bytes)
//!   0  magic        "DA11"
//!   4  format u16   FORMAT_VERSION (1)
//!   6  kind u16     KIND_FULL (1) | KIND_DELTA (2)
//!   8  window u64   provider window id (client-chosen; the broker rewrites
//!                   nothing but checks ownership)
//!  16  version u32  tree version after this message (nonzero)
//!  20  base u32     delta: version it applies to; full: 0
//!  24  focus u64    focused node id, 0 = none
//!  32  nodes u16    node records that follow
//!  34  removed u16  delta: removed ids that follow the header; full: 0
//!  36  reserved u32 zero
//! removed ids       u64 each
//! node records:
//!   id u64, parent u64 (0 = root), role u16, level u16, states u32,
//!   actions u32, bounds x,y,w,h i32 (26.6 logical),
//!   [states & HAS_RANGE]     min,max,now,step i64 (milli-units)
//!   pos u32, size u32 (0,0 = none),
//!   [states & HAS_SELECTION] anchor u32, caret u32 (UTF-8 bytes into value)
//!   labelled_by u64 (0 = none),
//!   children u16, child ids u64 each,
//!   name, value, description, accelerator: u16 length + UTF-8 bytes
//! ```
//! Full messages must form one valid tree (`Tree::validate`). Strings contain
//! no NUL or other C0 controls except TAB/LF in values.
use crate::node::{Actions, Bounds, Node, NodeId, RangeValue, Role, States};
use crate::tree::{Delta, Tree};
use alloc::{collections::BTreeMap, string::String, vec::Vec};

pub const MAGIC: [u8; 4] = *b"DA11";
pub const FORMAT_VERSION: u16 = 1;
pub const KIND_FULL: u16 = 1;
pub const KIND_DELTA: u16 = 2;
pub const HEADER_BYTES: usize = 40;
/// Bounds for one window's tree. A toolkit virtualizes lists and trees so
/// only realized rows are published.
pub const MAX_NODES: usize = 4096;
pub const MAX_CHILDREN: usize = 1024;
pub const MAX_DEPTH: usize = 64;
pub const MAX_NAME_BYTES: usize = 512;
pub const MAX_VALUE_BYTES: usize = 2048;
pub const MAX_DESCRIPTION_BYTES: usize = 512;
pub const MAX_ACCELERATOR_BYTES: usize = 32;
/// Largest encoded message (full tree or delta).
pub const MAX_MESSAGE_BYTES: usize = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    Truncated,
    BadMagic,
    UnsupportedFormat,
    BadKind,
    TrailingBytes,
    TooLarge,
    TooManyNodes,
    TooManyChildren,
    TooDeep,
    ZeroId,
    DuplicateNode,
    UnknownRole,
    UnknownFlags,
    BadBounds,
    BadRange,
    BadSelection,
    BadString,
    StringTooLong,
    MissingRoot,
    UnknownParent,
    UnknownChild,
    InconsistentChildren,
    Unreachable,
    UnknownFocus,
    StaleVersion,
    WrongWindow,
    Reserved,
}

/// A decoded, structurally checked message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Full(Tree),
    Delta(Delta),
}
impl Message {
    pub fn window(&self) -> u64 {
        match self {
            Self::Full(t) => t.window,
            Self::Delta(d) => d.window,
        }
    }
    pub fn version(&self) -> u32 {
        match self {
            Self::Full(t) => t.version,
            Self::Delta(d) => d.version,
        }
    }
}

/// Peek at the header: (kind, window, version, base).
pub fn header(b: &[u8]) -> Result<(u16, u64, u32, u32), WireError> {
    let mut r = Reader { b, at: 0 };
    let (kind, window, version, base, _, _, _) = read_header(&mut r)?;
    Ok((kind, window, version, base))
}

fn valid_text(s: &str, allow_breaks: bool) -> bool {
    !s.chars()
        .any(|c| (c.is_control() && !(allow_breaks && matches!(c, '\t' | '\n'))) || c == '\u{0}')
}
/// Per-node field checks shared by decoding and `Tree::validate`.
pub fn check_node(n: &Node) -> Result<(), WireError> {
    if n.id.0 == 0 || n.parent.is_some_and(|p| p.0 == 0) || n.labelled_by.is_some_and(|l| l.0 == 0)
    {
        return Err(WireError::ZeroId);
    }
    if n.children.len() > MAX_CHILDREN {
        return Err(WireError::TooManyChildren);
    }
    if n.states.0 & !States::KNOWN != 0 || n.actions.0 & !Actions::KNOWN != 0 {
        return Err(WireError::UnknownFlags);
    }
    if !n.bounds.valid() {
        return Err(WireError::BadBounds);
    }
    if n.states.contains(States::HAS_RANGE) != n.range.is_some()
        || n.range.is_some_and(|r| !r.valid())
    {
        return Err(WireError::BadRange);
    }
    if n.states.contains(States::HAS_SELECTION) != n.selection.is_some() {
        return Err(WireError::BadSelection);
    }
    if let Some((a, c)) = n.selection {
        let len = n.value.len() as u32;
        if a > len
            || c > len
            || !n.value.is_char_boundary(a as usize)
            || !n.value.is_char_boundary(c as usize)
        {
            return Err(WireError::BadSelection);
        }
    }
    if n.position.is_some_and(|(p, s)| p == 0 || s == 0 || p > s) {
        return Err(WireError::BadRange);
    }
    if n.name.len() > MAX_NAME_BYTES
        || n.value.len() > MAX_VALUE_BYTES
        || n.description.len() > MAX_DESCRIPTION_BYTES
        || n.accelerator.len() > MAX_ACCELERATOR_BYTES
    {
        return Err(WireError::StringTooLong);
    }
    if !valid_text(&n.name, false)
        || !valid_text(&n.value, true)
        || !valid_text(&n.description, true)
        || !valid_text(&n.accelerator, false)
    {
        return Err(WireError::BadString);
    }
    if n.states.contains(States::PROTECTED) && (!n.value.is_empty() || n.selection.is_some()) {
        // A provider must never publish password content.
        return Err(WireError::BadString);
    }
    Ok(())
}
/// Cut `text` to at most `max` bytes on a character boundary. Returns whether
/// anything was removed (toolkits then set VALUE_TRUNCATED).
pub fn clamp(text: &mut String, max: usize) -> bool {
    if text.len() <= max {
        return false;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    true
}

struct Writer<'a> {
    out: &'a mut Vec<u8>,
}
impl Writer<'_> {
    fn u16(&mut self, v: u16) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.u16(s.len() as u16);
        self.out.extend_from_slice(s.as_bytes());
    }
}
fn write_node(w: &mut Writer<'_>, n: &Node) {
    w.u64(n.id.0);
    w.u64(n.parent.map_or(0, |p| p.0));
    w.u16(n.role.wire());
    w.u16(n.level);
    w.u32(n.states.0);
    w.u32(n.actions.0);
    for v in [n.bounds.x, n.bounds.y, n.bounds.width, n.bounds.height] {
        w.i32(v);
    }
    if let Some(r) = n.range {
        for v in [r.min, r.max, r.now, r.step] {
            w.i64(v);
        }
    }
    let (p, s) = n.position.unwrap_or((0, 0));
    w.u32(p);
    w.u32(s);
    if let Some((a, c)) = n.selection {
        w.u32(a);
        w.u32(c);
    }
    w.u64(n.labelled_by.map_or(0, |l| l.0));
    w.u16(n.children.len() as u16);
    for c in &n.children {
        w.u64(c.0);
    }
    w.str(&n.name);
    w.str(&n.value);
    w.str(&n.description);
    w.str(&n.accelerator);
}
#[allow(clippy::too_many_arguments)]
fn write_header(
    w: &mut Writer<'_>,
    kind: u16,
    window: u64,
    version: u32,
    base: u32,
    focus: Option<NodeId>,
    nodes: usize,
    removed: usize,
) {
    w.out.extend_from_slice(&MAGIC);
    w.u16(FORMAT_VERSION);
    w.u16(kind);
    w.u64(window);
    w.u32(version);
    w.u32(base);
    w.u64(focus.map_or(0, |f| f.0));
    w.u16(nodes as u16);
    w.u16(removed as u16);
    w.u32(0);
}
/// Encode a complete tree (validated first).
pub fn encode_full(tree: &Tree) -> Result<Vec<u8>, WireError> {
    tree.validate()?;
    if tree.version == 0 {
        return Err(WireError::StaleVersion);
    }
    let mut out = Vec::new();
    let mut w = Writer { out: &mut out };
    write_header(
        &mut w,
        KIND_FULL,
        tree.window,
        tree.version,
        0,
        tree.focus,
        tree.len(),
        0,
    );
    // Reading order keeps decoders streaming-friendly (parents first).
    for (node, _) in tree.walk() {
        write_node(&mut w, node);
    }
    if out.len() > MAX_MESSAGE_BYTES {
        return Err(WireError::TooLarge);
    }
    Ok(out)
}
/// Encode a delta. Node fields are checked; structure is checked by the
/// receiver against its base tree (`Tree::apply`).
pub fn encode_delta(delta: &Delta) -> Result<Vec<u8>, WireError> {
    if delta.upserts.len() > MAX_NODES || delta.removed.len() > MAX_NODES {
        return Err(WireError::TooManyNodes);
    }
    if delta.version == 0 || delta.version == delta.base_version {
        return Err(WireError::StaleVersion);
    }
    for n in &delta.upserts {
        check_node(n)?;
    }
    if delta.removed.iter().any(|r| r.0 == 0) {
        return Err(WireError::ZeroId);
    }
    let mut out = Vec::new();
    let mut w = Writer { out: &mut out };
    write_header(
        &mut w,
        KIND_DELTA,
        delta.window,
        delta.version,
        delta.base_version,
        delta.focus,
        delta.upserts.len(),
        delta.removed.len(),
    );
    for r in &delta.removed {
        w.u64(r.0);
    }
    for n in &delta.upserts {
        write_node(&mut w, n);
    }
    if out.len() > MAX_MESSAGE_BYTES {
        return Err(WireError::TooLarge);
    }
    Ok(out)
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self.at.checked_add(n).ok_or(WireError::Truncated)?;
        let s = self.b.get(self.at..end).ok_or(WireError::Truncated)?;
        self.at = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> Result<i32, WireError> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, WireError> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn id(&mut self) -> Result<Option<NodeId>, WireError> {
        Ok(NodeId::new(self.u64()?))
    }
    fn str(&mut self, max: usize) -> Result<String, WireError> {
        let len = self.u16()? as usize;
        if len > max {
            return Err(WireError::StringTooLong);
        }
        let bytes = self.take(len)?;
        core::str::from_utf8(bytes)
            .map(String::from)
            .map_err(|_| WireError::BadString)
    }
}
type RawHeader = (u16, u64, u32, u32, Option<NodeId>, usize, usize);
fn read_header(r: &mut Reader<'_>) -> Result<RawHeader, WireError> {
    if r.b.len() > MAX_MESSAGE_BYTES {
        return Err(WireError::TooLarge);
    }
    if r.take(4)? != MAGIC {
        return Err(WireError::BadMagic);
    }
    if r.u16()? != FORMAT_VERSION {
        return Err(WireError::UnsupportedFormat);
    }
    let kind = r.u16()?;
    if kind != KIND_FULL && kind != KIND_DELTA {
        return Err(WireError::BadKind);
    }
    let window = r.u64()?;
    let version = r.u32()?;
    let base = r.u32()?;
    let focus = r.id()?;
    let nodes = r.u16()? as usize;
    let removed = r.u16()? as usize;
    if r.u32()? != 0 {
        return Err(WireError::Reserved);
    }
    if version == 0 || nodes > MAX_NODES || removed > MAX_NODES {
        return Err(if version == 0 {
            WireError::StaleVersion
        } else {
            WireError::TooManyNodes
        });
    }
    if kind == KIND_FULL && (base != 0 || removed != 0) {
        return Err(WireError::BadKind);
    }
    if kind == KIND_DELTA && base == version {
        return Err(WireError::StaleVersion);
    }
    Ok((kind, window, version, base, focus, nodes, removed))
}
fn read_node(r: &mut Reader<'_>) -> Result<Node, WireError> {
    let id = r.id()?.ok_or(WireError::ZeroId)?;
    let parent = r.id()?;
    let role = Role::from_wire(r.u16()?).ok_or(WireError::UnknownRole)?;
    let level = r.u16()?;
    let states = States::from_bits(r.u32()?).ok_or(WireError::UnknownFlags)?;
    let actions = Actions::from_bits(r.u32()?).ok_or(WireError::UnknownFlags)?;
    let bounds = Bounds {
        x: r.i32()?,
        y: r.i32()?,
        width: r.i32()?,
        height: r.i32()?,
    };
    let range = if states.contains(States::HAS_RANGE) {
        Some(RangeValue {
            min: r.i64()?,
            max: r.i64()?,
            now: r.i64()?,
            step: r.i64()?,
        })
    } else {
        None
    };
    let (p, s) = (r.u32()?, r.u32()?);
    let position = if (p, s) == (0, 0) { None } else { Some((p, s)) };
    let selection = if states.contains(States::HAS_SELECTION) {
        Some((r.u32()?, r.u32()?))
    } else {
        None
    };
    let labelled_by = r.id()?;
    let count = r.u16()? as usize;
    if count > MAX_CHILDREN {
        return Err(WireError::TooManyChildren);
    }
    let mut children = Vec::with_capacity(count);
    for _ in 0..count {
        children.push(r.id()?.ok_or(WireError::ZeroId)?);
    }
    let node = Node {
        id,
        parent,
        children,
        role,
        level,
        states,
        actions,
        bounds,
        range,
        position,
        selection,
        labelled_by,
        name: r.str(MAX_NAME_BYTES)?,
        value: r.str(MAX_VALUE_BYTES)?,
        description: r.str(MAX_DESCRIPTION_BYTES)?,
        accelerator: r.str(MAX_ACCELERATOR_BYTES)?,
    };
    check_node(&node)?;
    Ok(node)
}
/// Decode and validate one message. Full trees are structurally validated;
/// deltas are validated per node (structure is checked when applied).
pub fn decode(b: &[u8]) -> Result<Message, WireError> {
    let mut r = Reader { b, at: 0 };
    let (kind, window, version, base, focus, count, removed_count) = read_header(&mut r)?;
    let mut removed = Vec::with_capacity(removed_count);
    for _ in 0..removed_count {
        removed.push(r.id()?.ok_or(WireError::ZeroId)?);
    }
    let mut nodes: BTreeMap<NodeId, Node> = BTreeMap::new();
    let mut order = Vec::with_capacity(count);
    for _ in 0..count {
        let node = read_node(&mut r)?;
        order.push(node.id);
        if nodes.insert(node.id, node).is_some() {
            return Err(WireError::DuplicateNode);
        }
    }
    if r.at != b.len() {
        return Err(WireError::TrailingBytes);
    }
    if kind == KIND_FULL {
        let root = nodes
            .values()
            .find(|n| n.parent.is_none())
            .map(|n| n.id)
            .ok_or(WireError::MissingRoot)?;
        let tree = Tree::from_parts(window, version, root, focus, nodes);
        tree.validate()?;
        Ok(Message::Full(tree))
    } else {
        let upserts = order
            .into_iter()
            .map(|id| nodes.remove(&id).unwrap())
            .collect();
        Ok(Message::Delta(Delta {
            window,
            base_version: base,
            version,
            focus,
            removed,
            upserts,
        }))
    }
}
/// The broker's copy for an assistive reader: protected content stripped,
/// re-encoded as a full tree.
pub fn redact(tree: &Tree) -> Result<Vec<u8>, WireError> {
    let mut copy = tree.clone();
    copy.redact_protected();
    encode_full(&copy)
}
/// What an assistive reader receives for a secure surface (credential
/// prompts, the lock screen): one window node and nothing else.
pub fn placeholder(window: u64, version: u32, root: NodeId) -> Tree {
    let mut node = Node::new(root, Role::Window).named("Secure surface");
    node.states = States::PROTECTED;
    Tree::new(window, version.max(1), node)
}
/// A one-node tree containing just `id` (re-rooted, children dropped), for
/// cheap focus-following reads.
pub fn single(tree: &Tree, id: NodeId) -> Option<Tree> {
    let node = tree.get(id)?.clone();
    let mut t = Tree::new(tree.window, tree.version, node);
    t.focus = (tree.focus == Some(id)).then_some(id);
    Some(t)
}
