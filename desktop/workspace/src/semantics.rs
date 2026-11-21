//! Shared semantic-tree adapter for the immediate-mode desktop applications
//! (spec sections 58 and 113). An application describes its controls from
//! the same layout its hit testing uses (`Application::semantics`): role,
//! name, bounds in client logical pixels, states, and the hit-test target
//! id. This module turns that description into a validated
//! `door_accessibility` tree, and `Publisher` sends it through
//! door.accessibility (a full tree, then deltas) for the runtime, which also
//! routes assistive actions back to the target's own click path.
//!
//! Node ids derive from target ids, so an element keeps its identity across
//! frames and a focus move is a two-node delta; elements without a target
//! get ids from their order within the description.
use alloc::{string::String, vec::Vec};
pub use door_accessibility::{Actions, Role, States};
use door_accessibility::{Bounds, Node, NodeId, RangeValue, Tree, TreeBuilder, wire};

/// Ids of target-bearing nodes: `TARGET + target`.
const TARGET: u64 = 1 << 32;
/// Ids of described nodes without a target: `AUTO + order`.
const AUTO: u64 = 2 << 32;
const ROOT: u64 = 1;

struct Entry {
    depth: usize,
    node: Node,
    target: Option<u32>,
}

/// One window's control description, built in reading order.
pub struct Semantics {
    width: u32,
    height: u32,
    entries: Vec<Entry>,
    depth: usize,
    focus: Option<usize>,
}
/// Builder handle for the control just described.
pub struct Control<'a> {
    entry: &'a mut Entry,
    focus: &'a mut Option<usize>,
    index: usize,
}
impl Control<'_> {
    pub fn state(self, state: States, on: bool) -> Self {
        self.entry.node.states.set(state, on);
        self
    }
    pub fn states(self, states: States) -> Self {
        self.entry.node.states |= states;
        self
    }
    pub fn actions(self, actions: Actions) -> Self {
        self.entry.node.actions = actions;
        self
    }
    pub fn value(self, value: &str) -> Self {
        self.entry.node.value = clean(value, wire::MAX_VALUE_BYTES, true);
        if value.len() > wire::MAX_VALUE_BYTES {
            self.entry.node.states |= States::VALUE_TRUNCATED;
        }
        self
    }
    pub fn description(self, text: &str) -> Self {
        self.entry.node.description = clean(text, wire::MAX_DESCRIPTION_BYTES, true);
        self
    }
    pub fn accelerator(self, text: &str) -> Self {
        self.entry.node.accelerator = clean(text, wire::MAX_ACCELERATOR_BYTES, false);
        self
    }
    /// 1-based position in a set of `count` (ignored when out of range).
    pub fn position(self, index: usize, count: usize) -> Self {
        if index < count {
            self.entry.node.position =
                Some((index as u32 + 1, count.min(u32::MAX as usize) as u32));
        }
        self
    }
    pub fn level(self, level: u16) -> Self {
        self.entry.node.level = level;
        self
    }
    /// RangeValue (whole units); ignored when inconsistent.
    pub fn range(self, min: i64, max: i64, now: i64) -> Self {
        let r = RangeValue::integral(min, max, now, 1);
        if r.valid() {
            self.entry.node.range = Some(r);
            self.entry.node.states |= States::HAS_RANGE;
        }
        self
    }
    /// Text selection (anchor, caret) as UTF-8 offsets into the value set
    /// before; ignored unless both fall on its character boundaries.
    pub fn selection(self, anchor: usize, caret: usize) -> Self {
        let v = &self.entry.node.value;
        if anchor <= v.len()
            && caret <= v.len()
            && v.is_char_boundary(anchor)
            && v.is_char_boundary(caret)
        {
            self.entry.node.selection = Some((anchor as u32, caret as u32));
            self.entry.node.states |= States::HAS_SELECTION;
        }
        self
    }
    /// This control has keyboard focus when `on`.
    pub fn focused(self, on: bool) -> Self {
        if on {
            *self.focus = Some(self.index);
        }
        self
    }
}
/// Names and values never carry control characters on the wire.
fn clean(text: &str, max: usize, breaks: bool) -> String {
    let mut s: String = text
        .chars()
        .map(|c| {
            if c == '\0' || (c.is_control() && !(breaks && matches!(c, '\n' | '\t'))) {
                ' '
            } else {
                c
            }
        })
        .collect();
    wire::clamp(&mut s, max);
    s
}
fn bounds(r: (i32, i32, u32, u32)) -> Bounds {
    let w = r.2.min(i32::MAX as u32 / 64) as i32;
    let h = r.3.min(i32::MAX as u32 / 64) as i32;
    let b = Bounds::from_logical(
        r.0.clamp(-(1 << 24), 1 << 24),
        r.1.clamp(-(1 << 24), 1 << 24),
        w,
        h,
    );
    if b.valid() { b } else { Bounds::default() }
}
impl Semantics {
    /// A description for a client area of `width`×`height` logical pixels
    /// (controls outside it are marked OFFSCREEN).
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            entries: Vec::new(),
            depth: 0,
            focus: None,
        }
    }
    /// Describe a control at `rect` (x, y, width, height). `target` is the id
    /// the application's `target()` returns there: assistive Invoke clicks it.
    /// Interactive roles get FOCUSABLE and INVOKE; adjust with the builder.
    pub fn control(
        &mut self,
        role: Role,
        name: &str,
        rect: (i32, i32, u32, u32),
        target: Option<u32>,
    ) -> Control<'_> {
        let mut node = Node::new(NodeId(0), role);
        node.name = clean(name, wire::MAX_NAME_BYTES, false);
        node.bounds = bounds(rect);
        let interactive = target.is_some()
            && !matches!(
                role,
                Role::Text
                    | Role::Heading
                    | Role::Image
                    | Role::Separator
                    | Role::Pane
                    | Role::Group
                    | Role::Status
                    | Role::Document
            );
        if interactive {
            node.states = States::FOCUSABLE;
            node.actions = Actions::INVOKE;
        }
        let (x, y, w, h) = rect;
        let visible = x < self.width as i32
            && y < self.height as i32
            && x.saturating_add(w as i32) > 0
            && y.saturating_add(h as i32) > 0;
        if !visible {
            node.states |= States::OFFSCREEN;
        }
        self.entries.push(Entry {
            depth: self.depth,
            node,
            target,
        });
        let index = self.entries.len() - 1;
        Control {
            entry: &mut self.entries[index],
            focus: &mut self.focus,
            index,
        }
    }
    /// Static text (labels, headings, status).
    pub fn text(&mut self, role: Role, name: &str, rect: (i32, i32, u32, u32)) {
        if !name.trim().is_empty() {
            self.control(role, name, rect, None);
        }
    }
    /// Open a container (list, tree, tool bar, menu, dialog, group); the
    /// controls described until `end` are its children.
    pub fn begin(&mut self, role: Role, name: &str, rect: (i32, i32, u32, u32)) -> Control<'_> {
        let depth = self.depth;
        self.depth += 1;
        let index = self.entries.len();
        self.entries.push(Entry {
            depth,
            node: {
                let mut n = Node::new(NodeId(0), role);
                n.name = clean(name, wire::MAX_NAME_BYTES, false);
                n.bounds = bounds(rect);
                n
            },
            target: None,
        });
        Control {
            entry: &mut self.entries[index],
            focus: &mut self.focus,
            index,
        }
    }
    pub fn end(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// The validated tree: a Window root named `title`, the described
    /// controls under it (bounded by the wire limits; extra rows are
    /// dropped), and focus on the focused control or else the window.
    pub fn finish(self, window: u64, version: u32, title: &str) -> Built {
        let mut root = Node::new(NodeId(ROOT), Role::Window);
        root.name = clean(title, wire::MAX_NAME_BYTES, false);
        root.bounds = bounds((0, 0, self.width, self.height));
        let mut b = TreeBuilder::new(window, version.max(1), root);
        let mut stack: Vec<(usize, NodeId)> = Vec::new();
        let mut targets = Vec::new();
        let mut used = alloc::collections::BTreeSet::new();
        let mut focus = None;
        for (i, mut e) in self.entries.into_iter().enumerate() {
            if i + 2 > wire::MAX_NODES {
                break;
            }
            while stack.last().is_some_and(|(d, _)| *d >= e.depth) {
                stack.pop();
                b.leave();
            }
            if stack.len() >= wire::MAX_DEPTH - 2 {
                continue;
            }
            let wanted = e.target.map_or(AUTO + i as u64, |t| TARGET + u64::from(t));
            let id = if used.insert(wanted) {
                wanted
            } else {
                let auto = AUTO + i as u64;
                used.insert(auto);
                auto
            };
            e.node.id = NodeId(id);
            let parent = b.current();
            let parent_children = b.tree_mut().get(parent).map_or(0, |n| n.children.len());
            if parent_children >= wire::MAX_CHILDREN {
                continue;
            }
            if b.push(e.node).is_err() {
                continue;
            }
            if let Some(t) = e.target {
                targets.push((NodeId(id), t));
            }
            if self.focus == Some(i) {
                focus = Some(NodeId(id));
            }
            // A container's children follow at a greater depth.
            stack.push((e.depth, NodeId(id)));
            b.enter(NodeId(id));
        }
        let tree = b.finish(focus.or(Some(NodeId(ROOT))));
        Built { tree, targets }
    }
}
fn bounded(b: crate::explorer_layout::Bounds) -> (i32, i32, u32, u32) {
    (b.x, b.y, b.width, b.height)
}
fn center(b: crate::explorer_layout::Bounds) -> (i32, i32) {
    (b.x + b.width as i32 / 2, b.y + b.height as i32 / 2)
}
/// Describe an open Windows 11 context menu (`crate::context_menu`): its
/// icon row, commands and open submenu, each with the target `target`
/// hit-tests at the command's center. Focus follows the menu's own
/// keyboard focus (the highlighted command), as Narrator reads it.
pub fn context_menu(
    s: &mut Semantics,
    m: &crate::context_menu::MenuState,
    target: &dyn Fn(i32, i32) -> Option<u32>,
) {
    use crate::context_menu::Focus;
    let root = m.root();
    let in_sub = m.sub.is_some_and(|(_, child, _)| child.is_some());
    s.begin(Role::Menu, "Context", bounded(root.bounds));
    let icons = m.menu.icons.len();
    for (i, (e, b)) in m.menu.icons.iter().zip(&root.icons).enumerate() {
        s.control(
            Role::MenuItem,
            &e.label,
            bounded(*b),
            target(center(*b).0, center(*b).1),
        )
        .state(States::DISABLED, !e.enabled)
        .description(e.reason)
        .position(i, icons)
        .focused(!in_sub && m.focus == Focus::Icon(i));
    }
    let commands = m.menu.entries.iter().filter(|e| !e.separator).count();
    let mut k = 0;
    for (i, e) in m.menu.entries.iter().enumerate() {
        if e.separator {
            continue;
        }
        let at = k;
        k += 1;
        let Some(b) = root.rows.get(i).copied().flatten() else {
            continue;
        };
        let (cx, cy) = center(b);
        let c = s
            .control(Role::MenuItem, &e.label, bounded(b), target(cx, cy))
            .state(States::DISABLED, !e.enabled)
            .accelerator(e.shortcut)
            .description(e.reason)
            .position(at, commands)
            .focused(!in_sub && m.focus == Focus::Row(i));
        let c = if e.checked {
            c.states(States::CHECKABLE | States::CHECKED)
        } else {
            c
        };
        if !e.children.is_empty() {
            c.states(States::HAS_POPUP | States::EXPANDABLE)
                .state(States::EXPANDED, m.sub.is_some_and(|(p, _, _)| p == i));
        }
    }
    s.end();
    if let (Some((parent, child, _)), Some(p)) = (m.sub, m.submenu())
        && let Some(entry) = m.menu.entries.get(parent)
    {
        s.begin(Role::Menu, &entry.label, bounded(p.bounds));
        let n = entry.children.iter().filter(|e| !e.separator).count();
        let mut k = 0;
        for (j, e) in entry.children.iter().enumerate() {
            if e.separator {
                continue;
            }
            let at = k;
            k += 1;
            let Some(b) = p.rows.get(j).copied().flatten() else {
                continue;
            };
            let (cx, cy) = center(b);
            s.control(Role::MenuItem, &e.label, bounded(b), target(cx, cy))
                .state(States::DISABLED, !e.enabled)
                .accelerator(e.shortcut)
                .position(at, n)
                .focused(child == Some(j));
        }
        s.end();
    }
}
/// Describe an open classic drop-down menu (`crate::menu::Menu`, painted by
/// `View::popup`): its visible rows (targets `1000 + index`), with the
/// keyboard-selected row focused. A label's accelerator follows two spaces.
pub fn popup_menu(s: &mut Semantics, menu: &crate::menu::Menu, labels: &[&str], name: &str) {
    let Some((left, top)) = menu.origin() else {
        return;
    };
    let (offset, rows) = menu.viewport();
    let header = menu.header() as i32;
    let row = crate::menu::Menu::row_height();
    s.begin(
        Role::Menu,
        name,
        (left, top, 224, (rows as u32) * row + 8 + header as u32),
    );
    for (i, label) in labels.iter().enumerate().skip(offset).take(rows) {
        let (text, keys) = match label.find("  ") {
            Some(at) => (label[..at].trim(), label[at..].trim()),
            None => (label.trim(), ""),
        };
        let y = top + 4 + header + (i - offset) as i32 * row as i32;
        s.control(
            Role::MenuItem,
            text,
            (left + 4, y, 216, row),
            Some(1000 + i as u32),
        )
        .state(States::DISABLED, !menu.enabled(i))
        .accelerator(keys)
        .position(i, labels.len())
        .focused(i == menu.selected());
    }
    s.end();
}
/// Describe a modal message dialog (`crate::dialog`): title, body text and
/// its buttons (targets `first_target + index`); focus on the focused
/// button, as the dialog's own keyboard model has it.
pub fn dialog(
    s: &mut Semantics,
    layout: &crate::dialog::Layout,
    model: &crate::dialog::Dialog,
    title: &str,
    body: &str,
    buttons: &[&str],
    first_target: u32,
) {
    s.begin(Role::Dialog, title, bounded(layout.panel))
        .states(States::MODAL);
    s.text(Role::Text, body, bounded(layout.text));
    for (i, label) in buttons.iter().enumerate().take(model.count()) {
        s.control(
            Role::Button,
            label,
            bounded(layout.buttons[i]),
            Some(first_target + i as u32),
        )
        .state(States::DEFAULT, i == 0)
        .focused(model.focus() == Some(i) || (model.focus().is_none() && i == 0));
    }
    s.end();
}
/// A finished tree and the hit-test target behind each node.
pub struct Built {
    pub tree: Tree,
    pub targets: Vec<(NodeId, u32)>,
}
impl Built {
    pub fn target(&self, node: u64) -> Option<u32> {
        self.targets
            .iter()
            .find(|(n, _)| n.0 == node)
            .map(|(_, t)| *t)
    }
    /// The center of `node` in client logical pixels (assistive clicks).
    pub fn center(&self, node: u64) -> Option<(i32, i32)> {
        let n = self.tree.get(NodeId(node))?;
        let (x, y, w, h) = n.bounds.logical();
        Some((x + w / 2, y + h / 2))
    }
}

/// What to send for the next tree: a full message, or a delta from the
/// version the broker holds.
#[derive(Debug, PartialEq, Eq)]
pub enum Message {
    Full(Vec<u8>),
    Delta(Vec<u8>),
}
/// Versioned publication state of one window's tree.
#[derive(Default)]
pub struct Publisher {
    /// The tree the broker holds (after a successful commit).
    published: Option<Built>,
    /// A tree changed since the last publication and waits for its slot.
    pub stale: bool,
    /// Earliest tick for the next publication (bounds publication rate
    /// during animations; focus and state changes are not delayed past it).
    pub next_at: u64,
    /// Last version sent; versions keep increasing across reconnects.
    version: u32,
}
impl Publisher {
    /// Forget the broker's state (reconnect): the next tree is sent full.
    pub fn reset(&mut self) {
        self.published = None;
        self.stale = true;
    }
    pub fn published(&self) -> Option<&Built> {
        self.published.as_ref()
    }
    /// The message turning the broker's tree into `next` (renumbered to the
    /// following version), or None when nothing changed.
    pub fn prepare(&self, next: &mut Built) -> Option<Message> {
        next.tree.version = self.version.wrapping_add(1).max(1);
        match &self.published {
            Some(old) if old.tree.window == next.tree.window => {
                let delta = next.tree.diff(&old.tree);
                if delta.is_empty() && delta.focus == old.tree.focus {
                    return None;
                }
                wire::encode_delta(&delta).ok().map(Message::Delta)
            }
            _ => wire::encode_full(&next.tree).ok().map(Message::Full),
        }
    }
    /// The full message for `next` at its current version (after `Changed`).
    pub fn full(next: &Built) -> Option<Vec<u8>> {
        wire::encode_full(&next.tree).ok()
    }
    pub fn committed(&mut self, next: Built) {
        self.version = next.tree.version;
        self.published = Some(next);
        self.stale = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use door_accessibility::outline;
    fn describe(focus: usize) -> Semantics {
        let mut s = Semantics::new(800, 600);
        s.control(Role::Button, "Back", (0, 0, 40, 32), Some(7));
        s.begin(Role::List, "Items", (0, 40, 800, 400));
        for (i, name) in ["Documents", "Music", "Pictures"].iter().enumerate() {
            s.control(
                Role::ListItem,
                name,
                (0, 40 + i as i32 * 30, 800, 30),
                Some(100 + i as u32),
            )
            .states(States::SELECTABLE)
            .state(States::SELECTED, i == focus)
            .position(i, 3)
            .focused(i == focus);
        }
        s.end();
        s.text(Role::Status, "3 items", (0, 580, 800, 20));
        s.control(Role::Button, "Hidden", (0, 900, 40, 20), Some(9));
        s
    }
    #[test]
    fn descriptions_become_valid_trees_with_target_identity() {
        let built = describe(1).finish(42, 1, "File Explorer");
        built.tree.validate().unwrap();
        let o = outline(&built.tree);
        assert_eq!(
            o,
            "window \"File Explorer\"\n  button \"Back\" [FOCUSABLE] {INVOKE}\n  list \"Items\"\n    list item \"Documents\" [FOCUSABLE SELECTABLE] {INVOKE} pos=1/3\n    list item \"Music\" [FOCUSABLE SELECTED SELECTABLE] {INVOKE} pos=2/3\n    list item \"Pictures\" [FOCUSABLE SELECTABLE] {INVOKE} pos=3/3\n  status \"3 items\"\n  button \"Hidden\" [FOCUSABLE] {INVOKE}\n"
        );
        let focused = built.tree.focused().unwrap();
        assert_eq!(focused.name, "Music");
        assert_eq!(built.target(focused.id.0), Some(101));
        assert_eq!(built.center(focused.id.0), Some((400, 85)));
        assert!(
            built
                .tree
                .get(NodeId(TARGET + 9))
                .unwrap()
                .is(States::OFFSCREEN)
        );
        // Nothing focused: the window itself.
        let mut s = Semantics::new(10, 10);
        s.text(Role::Text, "\u{7}bell", (0, 0, 5, 5));
        let b = s.finish(1, 1, "T");
        assert_eq!(b.tree.focused().map(|n| n.role), Some(Role::Window));
        assert_eq!(b.tree.get(NodeId(AUTO)).unwrap().name, " bell");
    }
    #[test]
    fn publisher_sends_full_then_small_deltas_and_skips_unchanged_trees() {
        let mut p = Publisher::default();
        let mut first = describe(0).finish(42, 1, "Explorer");
        assert!(matches!(p.prepare(&mut first), Some(Message::Full(_))));
        p.committed(first);
        let mut same = describe(0).finish(42, 1, "Explorer");
        assert_eq!(p.prepare(&mut same), None);
        let mut moved = describe(2).finish(42, 1, "Explorer");
        let Some(Message::Delta(bytes)) = p.prepare(&mut moved) else {
            panic!("focus move is a delta")
        };
        let wire::Message::Delta(d) = wire::decode(&bytes).unwrap() else {
            panic!()
        };
        assert_eq!((d.base_version, d.version), (1, 2));
        assert_eq!(
            d.upserts.len(),
            2,
            "only the two items whose selection changed"
        );
        let mut mirror = p.published().unwrap().tree.clone();
        mirror.apply(&d).unwrap();
        assert_eq!(mirror.focused().map(|n| n.name.as_str()), Some("Pictures"));
        p.committed(moved);
        p.reset();
        let mut again = describe(2).finish(42, 1, "Explorer");
        assert!(matches!(p.prepare(&mut again), Some(Message::Full(_))));
    }
}
