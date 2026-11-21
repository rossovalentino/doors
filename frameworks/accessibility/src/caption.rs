//! Screen-reader output: Narrator-style captions for the focused element and
//! for changes worth announcing. Speech (P15 audio) consumes the same text;
//! until then a caption panel shows it, like Narrator's developer captions.
use crate::node::{Node, NodeId, Role, States};
use crate::tree::Tree;
use alloc::{format, string::String, vec::Vec};

fn push(out: &mut String, part: &str) {
    if part.is_empty() {
        return;
    }
    if !out.is_empty() {
        out.push_str(", ");
    }
    out.push_str(part);
}
fn milli(v: i64) -> String {
    let whole = v / 1000;
    let frac = (v % 1000).abs();
    if frac == 0 {
        format!("{whole}")
    } else {
        let mut s = format!(
            "{}{}.{:03}",
            if v < 0 && whole == 0 { "-" } else { "" },
            whole,
            frac
        );
        while s.ends_with('0') {
            s.pop();
        }
        s
    }
}
/// The accessible name, falling back to the labelling node's name.
pub fn name<'a>(tree: &'a Tree, node: &'a Node) -> &'a str {
    if node.name.is_empty()
        && let Some(label) = node.labelled_by.and_then(|l| tree.get(l))
    {
        return &label.name;
    }
    &node.name
}
/// "Name, role, state, value, k of n" — the order Narrator reads a newly
/// focused control in.
pub fn caption(tree: &Tree, node: &Node) -> String {
    let mut out = String::new();
    push(&mut out, name(tree, node));
    let protected = node.is(States::PROTECTED);
    push(
        &mut out,
        if protected && node.role == Role::Edit {
            "password edit"
        } else {
            node.role.spoken()
        },
    );
    if node.level > 0 && matches!(node.role, Role::TreeItem | Role::Heading) {
        push(&mut out, &format!("level {}", node.level));
    }
    let s = node.states;
    if s.contains(States::CHECKABLE) {
        let state = match (
            s.contains(States::CHECKED),
            s.contains(States::MIXED),
            node.role,
        ) {
            (_, true, _) => "partially checked",
            (true, _, Role::ToggleSwitch) => "On",
            (false, _, Role::ToggleSwitch) => "Off",
            (true, _, Role::ToggleButton) => "pressed",
            (false, _, Role::ToggleButton) => "not pressed",
            (true, _, _) => "checked",
            (false, _, _) => "not checked",
        };
        push(&mut out, state);
    }
    if s.contains(States::EXPANDABLE) {
        push(
            &mut out,
            if s.contains(States::EXPANDED) {
                "expanded"
            } else {
                "collapsed"
            },
        );
    }
    if s.contains(States::SELECTABLE) || node.role == Role::RadioButton {
        push(
            &mut out,
            if s.contains(States::SELECTED) {
                "selected"
            } else {
                "non-selected"
            },
        );
    }
    if s.contains(States::HAS_POPUP) && node.role != Role::ComboBox {
        push(&mut out, "has pop-up");
    }
    if s.contains(States::READ_ONLY) {
        push(&mut out, "read only");
    }
    if s.contains(States::REQUIRED) {
        push(&mut out, "required");
    }
    if s.contains(States::INVALID) {
        push(&mut out, "invalid entry");
    }
    if s.contains(States::BUSY) {
        push(&mut out, "busy");
    }
    if s.contains(States::DISABLED) {
        push(&mut out, "dimmed");
    }
    if protected {
        push(&mut out, "protected");
    } else if !node.value.is_empty() {
        let mut v = node.value.clone();
        if v.contains('\n') {
            v = v.lines().next().unwrap_or("").into();
        }
        push(&mut out, &v);
    } else if let Some(r) = node.range {
        push(&mut out, &milli(r.now));
    }
    if let Some((p, n)) = node.position {
        push(&mut out, &format!("{p} of {n}"));
    }
    if !node.accelerator.is_empty() {
        push(&mut out, &node.accelerator);
    }
    if !node.description.is_empty() {
        push(&mut out, &node.description);
    }
    out
}
/// What changed on the same node, in the short form Narrator uses.
fn change(tree: &Tree, old: &Node, new: &Node) -> Option<String> {
    let s = new.states;
    if old.states.contains(States::CHECKED) != s.contains(States::CHECKED)
        || old.states.contains(States::MIXED) != s.contains(States::MIXED)
    {
        let full = caption(tree, new);
        // Name and role were already spoken: announce only the new state.
        return Some(full.split(", ").nth(2).map(String::from).unwrap_or(full));
    }
    if old.states.contains(States::EXPANDED) != s.contains(States::EXPANDED)
        && s.contains(States::EXPANDABLE)
    {
        return Some(String::from(if s.contains(States::EXPANDED) {
            "expanded"
        } else {
            "collapsed"
        }));
    }
    if old.states.contains(States::SELECTED) != s.contains(States::SELECTED)
        && s.contains(States::SELECTED)
    {
        return Some(String::from("selected"));
    }
    if !new.is(States::PROTECTED) && old.value != new.value {
        return Some(if new.value.is_empty() {
            String::from("blank")
        } else {
            new.value.clone()
        });
    }
    if old.range.map(|r| r.now) != new.range.map(|r| r.now) {
        return new.range.map(|r| milli(r.now));
    }
    if old.states.contains(States::DISABLED) != s.contains(States::DISABLED) {
        return Some(String::from(if s.contains(States::DISABLED) {
            "dimmed"
        } else {
            "available"
        }));
    }
    None
}

/// One announcement, attributed to the node it describes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announcement {
    pub window: u64,
    pub node: NodeId,
    pub text: String,
    /// Assertive announcements interrupt (focus changes, alerts).
    pub assertive: bool,
}

/// Follows focus and live regions across tree versions and produces
/// captions. Feed it every new version of the trees it may read.
#[derive(Default)]
pub struct Announcer {
    /// The last focused node and its ancestors in that tree version (the
    /// node may be gone from the next version, e.g. a closed menu).
    last: Option<(u64, Node, Vec<NodeId>)>,
    live: Vec<(u64, Node)>,
    history: Vec<Announcement>,
}
impl Announcer {
    /// Bounded caption history shown in the caption panel.
    pub const HISTORY: usize = 64;
    pub fn history(&self) -> &[Announcement] {
        &self.history
    }
    /// The current focused element's caption (empty before any focus).
    pub fn current(&self) -> Option<&Announcement> {
        self.history.last()
    }
    /// The window closed: drop its focus and live-region state.
    pub fn forget(&mut self, window: u64) {
        self.live.retain(|(w, _)| *w != window);
        if self.last.as_ref().is_some_and(|(w, _, _)| *w == window) {
            self.last = None;
        }
    }
    fn say(&mut self, a: Announcement, out: &mut Vec<Announcement>) {
        if self.history.len() >= Self::HISTORY {
            self.history.remove(0);
        }
        self.history.push(a.clone());
        out.push(a);
    }
    /// Observe a tree version. Returns what a screen reader says now.
    pub fn observe(&mut self, tree: &Tree) -> Vec<Announcement> {
        let mut out = Vec::new();
        let window = tree.window;
        if let Some(focused) = tree.focused() {
            match &self.last {
                Some((w, old, _)) if *w == window && old.id == focused.id => {
                    if let Some(text) = change(tree, old, focused) {
                        self.say(
                            Announcement {
                                window,
                                node: focused.id,
                                text,
                                assertive: false,
                            },
                            &mut out,
                        );
                    }
                }
                _ => {
                    // Entering a dialog or menu announces the container first.
                    let mut text = String::new();
                    for a in tree.ancestors(focused.id).into_iter().rev() {
                        let entered = self
                            .last
                            .as_ref()
                            .is_none_or(|(w, _, chain)| *w != window || !chain.contains(&a.id));
                        if entered
                            && matches!(
                                a.role,
                                Role::Dialog
                                    | Role::Menu
                                    | Role::List
                                    | Role::Tree
                                    | Role::TabList
                                    | Role::MenuBar
                            )
                        {
                            push(&mut text, &a.name);
                            push(&mut text, a.role.spoken());
                        }
                    }
                    push(&mut text, &caption(tree, focused));
                    self.say(
                        Announcement {
                            window,
                            node: focused.id,
                            text,
                            assertive: true,
                        },
                        &mut out,
                    );
                }
            }
            let chain = tree.ancestors(focused.id).iter().map(|n| n.id).collect();
            self.last = Some((window, focused.clone(), chain));
        }
        // Live regions: announce new or changed content.
        let live: Vec<Node> = tree
            .nodes()
            .filter(|n| {
                n.states
                    .intersects(States::LIVE_POLITE | States::LIVE_ASSERTIVE)
            })
            .filter(|n| !n.is(States::OFFSCREEN))
            .cloned()
            .collect();
        for n in &live {
            let previous = self.live.iter().find(|(w, o)| *w == window && o.id == n.id);
            let changed = previous.is_none_or(|(_, o)| {
                o.name != n.name || o.description != n.description || o.value != n.value
            });
            if changed {
                let mut text = String::new();
                push(&mut text, &n.name);
                push(&mut text, &n.description);
                if !n.is(States::PROTECTED) {
                    push(&mut text, &n.value);
                }
                if !text.is_empty() {
                    self.say(
                        Announcement {
                            window,
                            node: n.id,
                            text,
                            assertive: n.is(States::LIVE_ASSERTIVE),
                        },
                        &mut out,
                    );
                }
            }
        }
        self.live.retain(|(w, _)| *w != window);
        self.live.extend(live.into_iter().map(|n| (window, n)));
        out
    }
}
