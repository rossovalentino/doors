//! One window's semantic tree, its structural invariants and incremental deltas.
use crate::node::{Node, NodeId, States};
use crate::wire::{self, WireError};
use alloc::{collections::BTreeMap, vec::Vec};

/// A complete semantic tree for one window. Versions increase by one for every
/// published change; a delta applies only to its exact base version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tree {
    pub window: u64,
    pub version: u32,
    pub root: NodeId,
    pub focus: Option<NodeId>,
    nodes: BTreeMap<NodeId, Node>,
}

/// Changes from `base_version` to `version`: removed subtrees' node ids and
/// every added or changed node (complete records, children included).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Delta {
    pub window: u64,
    pub base_version: u32,
    pub version: u32,
    pub focus: Option<NodeId>,
    pub removed: Vec<NodeId>,
    pub upserts: Vec<Node>,
}
impl Delta {
    pub fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.upserts.is_empty()
    }
}

impl Tree {
    /// A tree whose root is `root` (its parent is cleared).
    pub fn new(window: u64, version: u32, mut root: Node) -> Self {
        root.parent = None;
        root.children.clear();
        let id = root.id;
        let mut nodes = BTreeMap::new();
        nodes.insert(id, root);
        Self {
            window,
            version,
            root: id,
            focus: None,
            nodes,
        }
    }
    pub(crate) fn from_parts(
        window: u64,
        version: u32,
        root: NodeId,
        focus: Option<NodeId>,
        nodes: BTreeMap<NodeId, Node>,
    ) -> Self {
        Self {
            window,
            version,
            root,
            focus,
            nodes,
        }
    }
    /// Append `node` as the last child of `parent`. Fails for unknown parents
    /// and duplicate ids; the tree is unchanged on failure.
    pub fn push(&mut self, parent: NodeId, mut node: Node) -> Result<(), WireError> {
        if self.nodes.contains_key(&node.id) {
            return Err(WireError::DuplicateNode);
        }
        if self.nodes.len() >= wire::MAX_NODES {
            return Err(WireError::TooManyNodes);
        }
        let p = self
            .nodes
            .get_mut(&parent)
            .ok_or(WireError::UnknownParent)?;
        if p.children.len() >= wire::MAX_CHILDREN {
            return Err(WireError::TooManyChildren);
        }
        p.children.push(node.id);
        node.parent = Some(parent);
        node.children.clear();
        self.nodes.insert(node.id, node);
        Ok(())
    }
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(&id)
    }
    pub fn root(&self) -> &Node {
        &self.nodes[&self.root]
    }
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
    /// Nodes in id order (not reading order).
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }
    /// Nodes in depth-first reading order.
    pub fn walk(&self) -> Vec<(&Node, usize)> {
        let mut out = Vec::with_capacity(self.nodes.len());
        let mut stack = alloc::vec![(self.root, 0usize)];
        while let Some((id, depth)) = stack.pop() {
            let Some(node) = self.nodes.get(&id) else {
                continue;
            };
            out.push((node, depth));
            for child in node.children.iter().rev() {
                stack.push((*child, depth + 1));
            }
        }
        out
    }
    pub fn focused(&self) -> Option<&Node> {
        self.focus.and_then(|f| self.nodes.get(&f))
    }
    /// Ancestors of `id`, nearest first (excluding `id`).
    pub fn ancestors(&self, id: NodeId) -> Vec<&Node> {
        let mut out = Vec::new();
        let mut at = self.nodes.get(&id).and_then(|n| n.parent);
        while let Some(p) = at {
            let Some(n) = self.nodes.get(&p) else { break };
            out.push(n);
            at = n.parent;
            if out.len() > wire::MAX_DEPTH {
                break;
            }
        }
        out
    }
    /// Full structural validation: one root, consistent parent/children
    /// links, every node reachable exactly once, bounded depth, valid
    /// geometry/range/selection and string limits, focus on a live node.
    pub fn validate(&self) -> Result<(), WireError> {
        if self.nodes.len() > wire::MAX_NODES {
            return Err(WireError::TooManyNodes);
        }
        let root = self.nodes.get(&self.root).ok_or(WireError::MissingRoot)?;
        if root.parent.is_some() {
            return Err(WireError::MissingRoot);
        }
        for node in self.nodes.values() {
            wire::check_node(node)?;
            match node.parent {
                None if node.id != self.root => return Err(WireError::MissingRoot),
                Some(p) => {
                    let parent = self.nodes.get(&p).ok_or(WireError::UnknownParent)?;
                    if !parent.children.contains(&node.id) {
                        return Err(WireError::InconsistentChildren);
                    }
                }
                None => (),
            }
            for child in &node.children {
                let c = self.nodes.get(child).ok_or(WireError::UnknownChild)?;
                if c.parent != Some(node.id) {
                    return Err(WireError::InconsistentChildren);
                }
            }
        }
        // Every node reached exactly once from the root: a node listed twice
        // (by one parent or two) or reached again (a cycle) is refused, and a
        // node never reached (a detached subtree or self-cycle) too. Counting
        // visits is not enough: root [A, A] plus a detached C -> C balances.
        let mut visited = alloc::collections::BTreeSet::new();
        let mut stack = alloc::vec![(self.root, 0usize)];
        while let Some((id, depth)) = stack.pop() {
            if depth > wire::MAX_DEPTH {
                return Err(WireError::TooDeep);
            }
            if !visited.insert(id) {
                return Err(WireError::InconsistentChildren);
            }
            for child in &self.nodes[&id].children {
                stack.push((*child, depth + 1));
            }
        }
        if visited.len() != self.nodes.len() {
            return Err(WireError::Unreachable);
        }
        if self.focus.is_some_and(|f| !self.nodes.contains_key(&f)) {
            return Err(WireError::UnknownFocus);
        }
        Ok(())
    }
    /// The changes that turn `old` into `self` (same window). Unchanged nodes
    /// are omitted, so a focus move or a single value change is a small delta.
    pub fn diff(&self, old: &Tree) -> Delta {
        let removed = old
            .nodes
            .keys()
            .filter(|id| !self.nodes.contains_key(id))
            .copied()
            .collect();
        let upserts = self
            .nodes
            .values()
            .filter(|n| old.nodes.get(&n.id) != Some(*n))
            .cloned()
            .collect();
        Delta {
            window: self.window,
            base_version: old.version,
            version: self.version,
            focus: self.focus,
            removed,
            upserts,
        }
    }
    /// Apply a delta for this exact version; the result is validated and the
    /// tree is left unchanged when the delta does not apply cleanly.
    pub fn apply(&mut self, delta: &Delta) -> Result<(), WireError> {
        if delta.window != self.window {
            return Err(WireError::WrongWindow);
        }
        if delta.base_version != self.version || delta.version == self.version {
            return Err(WireError::StaleVersion);
        }
        let mut next = self.clone();
        for id in &delta.removed {
            if *id == next.root {
                return Err(WireError::MissingRoot);
            }
            next.nodes.remove(id);
        }
        for node in &delta.upserts {
            if node.parent.is_none() {
                next.root = node.id;
            }
            next.nodes.insert(node.id, node.clone());
        }
        next.version = delta.version;
        next.focus = delta.focus;
        next.validate()?;
        *self = next;
        Ok(())
    }
    /// Strip what an assistive reader must never receive: values, selections
    /// and descriptions of PROTECTED (password) nodes.
    pub fn redact_protected(&mut self) {
        for node in self.nodes.values_mut() {
            if node.is(States::PROTECTED) {
                node.value.clear();
                node.selection = None;
                node.states.set(States::HAS_SELECTION, false);
                node.states.set(States::VALUE_TRUNCATED, false);
            }
        }
    }
}

/// Builds a tree in reading order from a toolkit traversal.
pub struct TreeBuilder {
    tree: Tree,
    stack: Vec<NodeId>,
}
impl TreeBuilder {
    pub fn new(window: u64, version: u32, root: Node) -> Self {
        let id = root.id;
        Self {
            tree: Tree::new(window, version, root),
            stack: alloc::vec![id],
        }
    }
    /// Current parent for `push`.
    pub fn current(&self) -> NodeId {
        *self.stack.last().unwrap_or(&self.tree.root)
    }
    /// Add a leaf (or a node whose children follow after `enter`).
    pub fn push(&mut self, node: Node) -> Result<NodeId, WireError> {
        let id = node.id;
        self.tree.push(self.current(), node)?;
        Ok(id)
    }
    /// Subsequent pushes become children of `id` until `leave`.
    pub fn enter(&mut self, id: NodeId) {
        self.stack.push(id);
    }
    pub fn leave(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }
    pub fn tree_mut(&mut self) -> &mut Tree {
        &mut self.tree
    }
    pub fn finish(self, focus: Option<NodeId>) -> Tree {
        let mut tree = self.tree;
        tree.focus = focus.filter(|f| tree.nodes.contains_key(f));
        tree
    }
}
