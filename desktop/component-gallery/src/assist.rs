//! `door-assist`: a Narrator-style caption panel for whatever window has
//! focus. It is an assistive client of the door.accessibility broker: it
//! needs the assistive grant (CAP_ASSISTIVE), follows FOCUS_CHANGED and
//! TREE_CHANGED, reads redacted trees and shows what a screen reader would
//! say. Speech uses the same text once audio exists (P15).
use alloc::{collections::BTreeMap, string::String, vec::Vec};
use door_accessibility::caption::Announcer;
use door_accessibility::mirror::{Applied, Mirror};
use door_ui::controls::{Panel, TextBlock, Tone};
use door_ui::{Insets, TextRole, Ui, WidgetId};

/// Platform-independent caption logic (host-tested).
#[derive(Default)]
pub struct Follower {
    mirror: Mirror,
    announcer: Announcer,
    pub captions: Vec<String>,
    pub paused: bool,
    /// `shape` of the tree last reported per window (`changed_outline`).
    reported: BTreeMap<u64, String>,
    /// Window of the last applied message.
    pub window: Option<u64>,
}
impl Follower {
    /// Apply one tree message read from the broker; returns new captions.
    pub fn apply(&mut self, message: &[u8]) -> Vec<String> {
        let window = match self.mirror.apply_bytes(message) {
            Ok(Applied::Current(_)) => door_accessibility::wire::header(message).map(|h| h.1).ok(),
            _ => None,
        };
        self.window = window;
        let Some(tree) = window.and_then(|w| self.mirror.tree(w)) else {
            return Vec::new();
        };
        let said: Vec<String> = self
            .announcer
            .observe(tree)
            .into_iter()
            .map(|a| a.text)
            .collect();
        self.captions.extend(said.iter().cloned());
        if self.captions.len() > 64 {
            self.captions.drain(..self.captions.len() - 64);
        }
        said
    }
    /// Mirror a tree read from an unfocused window (the taskbar, say)
    /// without announcing anything; returns its outline when its `shape`
    /// differs from the one last reported for that window.
    pub fn apply_quiet(&mut self, message: &[u8]) -> Option<(u64, u32, String)> {
        let Ok(Applied::Current(_)) = self.mirror.apply_bytes(message) else {
            return None;
        };
        let window = door_accessibility::wire::header(message)
            .map(|h| h.1)
            .ok()?;
        let tree = self.mirror.tree(window)?;
        let shape = shape(tree);
        if self.reported.get(&window) == Some(&shape) {
            return None;
        }
        self.reported.insert(window, shape);
        Some((window, tree.version, door_accessibility::outline(tree)))
    }
    /// The mirrored tree of `window`.
    pub fn tree(&self, window: u64) -> Option<&door_accessibility::Tree> {
        self.mirror.tree(window)
    }
    /// The outline (`door_accessibility::outline`) of the last applied
    /// window's tree with its version, when its `shape` differs from the
    /// one last returned for that window: focus moves alone do not change
    /// it, nor do values and states, which are captioned instead.
    pub fn changed_outline(&mut self) -> Option<(u64, u32, String)> {
        let window = self.window?;
        let tree = self.mirror.tree(window)?;
        let shape = shape(tree);
        if self.reported.get(&window) == Some(&shape) {
            return None;
        }
        self.reported.insert(window, shape);
        Some((window, tree.version, door_accessibility::outline(tree)))
    }
    /// The window closed (TREE_CHANGED with version 0): drop its mirror,
    /// report shape and caption state so churn never grows the heap.
    pub fn forget(&mut self, window: u64) {
        self.mirror.forget(window);
        self.announcer.forget(window);
        self.reported.remove(&window);
        if self.window == Some(window) {
            self.window = None;
        }
    }
    /// Windows mirrored now.
    pub fn windows(&self) -> usize {
        self.mirror.windows().count()
    }
    /// The session locked (true) or unlocked: nothing is readable meanwhile.
    pub fn access(&mut self, granted: bool) {
        self.paused = !granted;
        if !granted {
            self.reported.clear();
            self.mirror.clear();
            self.captions.push(String::from(
                "Screen reading is paused while the session is locked",
            ));
        }
    }
}

/// What a tree report is for: the nodes with their roles, names and nesting.
/// Values, states and ranges (a combo box's choice, a switch turned on, the
/// text typed into an edit) leave it unchanged. Those changes reach the
/// caption log already, and re-reporting a 137-line page for every theme
/// picked would spend the client's 128 KiB diagnostic budget mid-session.
fn shape(tree: &door_accessibility::Tree) -> String {
    use core::fmt::Write;
    let mut out = String::new();
    let mut stack = alloc::vec![(tree.root, 0usize)];
    while let Some((id, depth)) = stack.pop() {
        let Some(node) = tree.get(id) else {
            continue;
        };
        let _ = writeln!(out, "{depth} {} {}", node.role as u16, node.name);
        stack.extend(node.children.iter().rev().map(|c| (*c, depth + 1)));
    }
    out
}

/// `text` in pieces of at most `max` bytes, split on character boundaries
/// (an empty line is one empty piece).
pub fn chunks(text: &str, max: usize) -> impl Iterator<Item = &str> {
    let mut rest = Some(text);
    core::iter::from_fn(move || {
        let r = rest?;
        if r.len() <= max {
            rest = None;
            return Some(r);
        }
        let mut cut = max;
        while !r.is_char_boundary(cut) {
            cut -= 1;
        }
        rest = Some(&r[cut..]);
        Some(&r[..cut])
    })
}

#[cfg_attr(not(feature = "accessibility"), allow(dead_code))]
pub struct Assist {
    pub follower: Follower,
    current: Option<WidgetId>,
    history: Option<WidgetId>,
    dirty: bool,
    ready: bool,
    /// Windows whose tree changed while unfocused and that were never
    /// reported (the taskbar, a background app): reported once, quietly.
    unseen: Vec<u64>,
    /// Live windows already read (closed ones are forgotten).
    seen: Vec<u64>,
    /// Serial TREE and CAPTION records (app text): only when Init started
    /// Captions in `ASSIST_DIAGNOSTIC_MODE`, which it does only on the test
    /// image (staged `system/assist-diagnostics.conf`).
    diagnostics: bool,
}
impl Default for Assist {
    fn default() -> Self {
        Self::new(false)
    }
}
/// Windows remembered as read before the list is pruned to live trees.
#[cfg_attr(not(feature = "accessibility"), allow(dead_code))]
const MAX_SEEN: usize = 64;
impl Assist {
    pub fn new(diagnostics: bool) -> Self {
        Self {
            follower: Follower::default(),
            current: None,
            history: None,
            dirty: true,
            ready: false,
            unseen: Vec::new(),
            seen: Vec::new(),
            diagnostics,
        }
    }
    /// Whether app text may reach the serial diagnostics.
    pub fn diagnostics(&self) -> bool {
        self.diagnostics
    }
    /// TREE_CHANGED for `window` (`version` 0: the window closed).
    pub fn tree_changed(&mut self, window: u64, version: u32) {
        self.dirty = true;
        if version == 0 {
            self.follower.forget(window);
            self.seen.retain(|w| *w != window);
            self.unseen.retain(|w| *w != window);
            return;
        }
        if !self.seen.contains(&window) && !self.unseen.contains(&window) && self.unseen.len() < 8 {
            self.unseen.push(window);
        }
    }
    /// `window` was read: remember it, keeping the list to live trees.
    #[cfg_attr(not(feature = "accessibility"), allow(dead_code))]
    fn mark_seen(&mut self, window: u64) {
        if !self.seen.contains(&window) {
            self.seen.push(window);
        }
        if self.seen.len() > MAX_SEEN {
            let follower = &self.follower;
            self.seen.retain(|w| follower.tree(*w).is_some());
        }
        if self.seen.len() > MAX_SEEN {
            self.seen.drain(..self.seen.len() - MAX_SEEN);
        }
    }
    /// Read and pending windows (host tests).
    pub fn tracked(&self) -> (usize, usize) {
        (self.seen.len(), self.unseen.len())
    }
}
/// One `DOOR_ASSIST TREE` block: the outline in diagnostic records of at
/// most 256 bytes (long lines continue on "+" records).
#[cfg(feature = "accessibility")]
fn report(window: u64, version: u32, outline: &str) {
    door_rt::diagnostic(format_args!(
        "DOOR_ASSIST TREE window={window} version={version} lines={}\n",
        outline.lines().count()
    ));
    for line in outline.lines() {
        for (i, part) in chunks(line, 200).enumerate() {
            let mark = if i == 0 { '|' } else { '+' };
            door_rt::diagnostic(format_args!("DOOR_ASSIST {mark} {part}\n"));
        }
    }
    door_rt::diagnostic(format_args!("DOOR_ASSIST END window={window}\n"));
}
#[cfg_attr(not(feature = "accessibility"), allow(dead_code))]
impl Assist {
    fn show(&mut self, ui: &mut Ui) {
        let (Some(current), Some(history)) = (self.current, self.history) else {
            return;
        };
        let latest = self
            .follower
            .captions
            .last()
            .cloned()
            .unwrap_or_else(|| String::from("Waiting for focus..."));
        ui.edit::<TextBlock, _>(current, |t| t.set_text(&latest));
        for c in ui.children(history).to_vec() {
            ui.remove(c);
        }
        for c in self.follower.captions.iter().rev().skip(1).take(8) {
            ui.add(history, TextBlock::new(c).tone(Tone::Secondary));
        }
    }
}
impl door_ui_native::App for Assist {
    fn title(&self) -> &str {
        "Captions"
    }
    /// The assistive grant; the window server gives it to the Captions
    /// manifest identity only, whatever this asks for.
    fn capabilities(&self) -> u32 {
        door_service_protocol::accessibility::CAP_ASSISTIVE
    }
    fn size(&self) -> (u32, u32) {
        (560, 320)
    }
    fn build(&mut self, ui: &mut Ui) {
        let col = ui.add(ui.root(), Panel::column(8).padding(Insets::all(16)));
        self.current = Some(
            ui.add(
                col,
                TextBlock::new("Waiting for focus...")
                    .role(TextRole::Subtitle)
                    .wrapping(),
            ),
        );
        self.history = Some(ui.add(col, Panel::column(4)));
    }
    #[allow(unused_variables)]
    fn platform(
        &mut self,
        ui: &mut Ui,
        client: &mut door_service_client::window::WindowClient,
        event: Option<door_service_protocol::window::Event>,
    ) {
        #[cfg(feature = "accessibility")]
        {
            use door_service_protocol::accessibility as a;
            if let Some(e) = event {
                match e.kind {
                    a::ACCESS_CHANGED => {
                        self.follower.access(e.a != 0);
                        self.show(ui);
                    }
                    a::TREE_CHANGED => self.tree_changed(e.window, e.a),
                    a::FOCUS_CHANGED => self.dirty = true,
                    _ => (),
                }
                return;
            }
            if !self.ready {
                self.ready = true;
                // The shipping image keeps Captions silent on serial.
                if self.diagnostics {
                    door_rt::diagnostic(format_args!(
                        "DOOR_ASSIST READY assistive={}\n",
                        u8::from(client.supports_assistive())
                    ));
                }
            }
            if !self.dirty || !client.supports_assistive() || self.follower.paused {
                return;
            }
            self.dirty = false;
            let Ok(focus) = client.accessibility_focus() else {
                return;
            };
            let mut buffer = alloc::vec![0u8; 128 * 1024];
            // With nothing focused (just launched, or focus in a window that
            // publishes no tree) the unfocused windows below are still read.
            if focus.window != 0 {
                self.read_focused(ui, client, focus.window, &mut buffer);
            }
            // Unfocused windows (the taskbar, background apps): read once.
            for window in core::mem::take(&mut self.unseen) {
                if window == focus.window || self.seen.contains(&window) {
                    continue;
                }
                self.mark_seen(window);
                let whole = a::Snapshot {
                    window,
                    scope: a::SCOPE_TREE,
                    node: 0,
                };
                if let Ok(n) = client.accessibility_read(whole, &mut buffer)
                    && let Some((window, version, outline)) =
                        self.follower.apply_quiet(&buffer[..n])
                    && self.diagnostics
                {
                    report(window, version, &outline);
                }
            }
        }
    }
}
impl Assist {
    /// Reads the focused window's tree: captions for what changed, and the
    /// outline when it differs from the last one reported.
    #[cfg(feature = "accessibility")]
    fn read_focused(
        &mut self,
        ui: &mut Ui,
        client: &mut door_service_client::window::WindowClient,
        window: u64,
        buffer: &mut [u8],
    ) {
        use door_service_protocol::accessibility as a;
        let full = a::Snapshot {
            window,
            scope: a::SCOPE_TREE,
            node: 0,
        };
        let focused = a::Snapshot {
            window,
            scope: a::SCOPE_FOCUS,
            node: 0,
        };
        let n = client
            .accessibility_read(full, buffer)
            .or_else(|_| client.accessibility_read(focused, buffer));
        if let Ok(n) = n {
            self.mark_seen(window);
            let said = self.follower.apply(&buffer[..n]);
            let outline = self.follower.changed_outline();
            // The captions and the semantic tree as read through the broker,
            // for guest checks against the host fixtures: diagnostics image
            // only, never app text on the serial line of a normal session.
            if self.diagnostics {
                for caption in said {
                    door_rt::diagnostic(format_args!("DOOR_ASSIST CAPTION {caption}\n"));
                }
                if let Some((window, version, outline)) = outline {
                    report(window, version, &outline);
                }
            }
            self.show(ui);
        }
    }
}
