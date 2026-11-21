//! The retained widget tree and its pipeline:
//! `event -> model update -> invalidation -> layout if needed -> paint if
//! needed (exact damage) -> semantics if needed (versioned tree + delta)`.
//! The UI thread owns every widget; background work reports through
//! generation-tagged tokens (`begin_async` / `deliver`).
use crate::display::{DisplayList, Images, Op};
use crate::event::{Button, Composition, Event, Key, KeyEvent, Modifiers, Phase, Pointer};
use crate::id::{Arena, WidgetId};
use crate::layout::{self, Host, Kind, Measured, Style};
use crate::render::{self, Fonts, Frame, RenderStats};
use crate::text::{TextSystem, wrap};
use crate::theme::{TextRole, Theme};
use crate::units::{Insets, LPoint, LRect, LSize, Lu};
use crate::widget::{
    Clipboard, Cursor, EventCx, LayoutCx, LocalClipboard, Notification, Notify, PaintCx, Request,
    SemanticsCx, Widget, WidgetState,
};
use alloc::{boxed::Box, collections::BTreeMap, string::String, vec::Vec};
use core::any::Any;
use door_accessibility::{Action, Bounds, Node, NodeId, Role, States, Tree, TreeBuilder, wire};
use door_graphics_core::Scale;

/// Why a node was invalidated since the last frame (inspector).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reasons(pub u8);
impl Reasons {
    pub const CREATED: u8 = 1;
    pub const LAYOUT: u8 = 2;
    pub const PAINT: u8 = 4;
    pub const THEME: u8 = 8;
    pub const HOVER: u8 = 16;
    pub const FOCUS: u8 = 32;
    pub const SCROLL: u8 = 64;
    pub const RESIZE: u8 = 128;
    pub fn names(self) -> Vec<&'static str> {
        [
            (Self::CREATED, "created"),
            (Self::LAYOUT, "layout"),
            (Self::PAINT, "paint"),
            (Self::THEME, "theme"),
            (Self::HOVER, "hover"),
            (Self::FOCUS, "focus"),
            (Self::SCROLL, "scroll"),
            (Self::RESIZE, "resize"),
        ]
        .into_iter()
        .filter(|(b, _)| self.0 & b != 0)
        .map(|(_, n)| n)
        .collect()
    }
}

struct NodeData {
    widget: Box<dyn Widget>,
    parent: Option<WidgetId>,
    children: Vec<WidgetId>,
    style: Style,
    enabled: bool,
    rect: LRect,
    baseline: Option<Lu>,
    content: LSize,
    scroll: LPoint,
    cache: Vec<(LSize, Measured)>,
    name: Option<String>,
    tooltip: Option<String>,
    tab_index: Option<i32>,
    forced: Option<VisualState>,
    tab_stop: bool,
    async_op: u64,
    measures: u32,
    reasons: u8,
    last_reasons: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Centred in the window (dialogs).
    Center,
    /// Below a window-space anchor, flipped above when there is no room.
    Below(LRect),
    /// At a window-space point (context flyouts).
    At(LPoint),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LayerOptions {
    /// Blocks input to everything below and traps focus.
    pub modal: bool,
    /// Closes on a click outside or Escape.
    pub light_dismiss: bool,
    pub placement: Placement,
}
impl LayerOptions {
    pub const fn modal() -> Self {
        Self {
            modal: true,
            light_dismiss: false,
            placement: Placement::Center,
        }
    }
    pub const fn flyout(placement: Placement) -> Self {
        Self {
            modal: false,
            light_dismiss: true,
            placement,
        }
    }
}
struct Layer {
    root: WidgetId,
    options: LayerOptions,
    restore: Option<WidgetId>,
}

/// A visual state forced for paint only (`Ui::force_visual_state`, as
/// WinUI's `VisualStateManager.GoToState` in state galleries). Behaviour,
/// hit testing and semantics keep the real state; disabled is not forced
/// (use `set_enabled`, which also blocks input).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisualState {
    Rest,
    /// Pointer over.
    Hover,
    /// Pointer captured and inside.
    Pressed,
    /// Keyboard focus with the focus visual.
    Focus,
}

/// Generation-tagged handle for asynchronous work aimed at one widget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AsyncToken {
    widget: WidgetId,
    op: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionError {
    UnknownNode,
    /// Not advertised by the node, or the node is disabled.
    NotAccepted,
    /// The control declined in its current state.
    Rejected,
    /// Outside the active modal layer: a dialog above it owns input, for
    /// assistive technology as for the keyboard and pointer.
    OutsideModal,
}

/// Counters for invalidation accounting (deterministic per input).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    pub frames: u64,
    pub layout_passes: u64,
    pub measures_last: u32,
    pub paint_passes: u64,
    pub ops_last: usize,
    pub damaged_widgets_last: usize,
    /// Damage area of the last paint pass in whole square DIPs.
    pub damage_area_last: u64,
    pub semantic_versions: u64,
    pub events_routed: u64,
    pub async_delivered: u64,
    pub async_discarded: u64,
}

/// What changed in the semantic tree since the last `take_semantics`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticUpdate {
    pub version: u32,
    /// The complete tree (wire format).
    pub full: Vec<u8>,
    /// Version the delta applies to: the tree the previous `take_semantics`
    /// returned. Versions in between are folded into this one update.
    pub base: Option<u32>,
    /// Delta from `base` (wire format), when there is a base.
    pub delta: Option<Vec<u8>>,
}

#[derive(Default)]
struct Tooltip {
    owner: Option<WidgetId>,
    since: u64,
    shown: bool,
    at: LPoint,
}

pub struct Ui {
    nodes: Arena<NodeData>,
    root: WidgetId,
    layers: Vec<Layer>,
    theme: Theme,
    text: Box<dyn TextSystem>,
    clipboard: Box<dyn Clipboard>,
    size: LSize,
    title: String,
    window: u64,
    now: u64,
    focus: Option<WidgetId>,
    focus_visible: bool,
    keyboard_mode: bool,
    active: bool,
    remembered: Option<WidgetId>,
    hover: Vec<WidgetId>,
    capture: Option<WidgetId>,
    pointer: Option<LPoint>,
    overlays: Vec<WidgetId>,
    notifications: Vec<Notification>,
    layout_dirty: bool,
    paint_dirty: bool,
    semantics_dirty: bool,
    list: DisplayList,
    painted: BTreeMap<WidgetId, (u64, LRect)>,
    damage: Option<LRect>,
    semantics: Option<Tree>,
    semantic_map: BTreeMap<NodeId, (WidgetId, Option<u32>)>,
    /// The tree the consumer last took (the delta base).
    published: Option<Tree>,
    semantics_pending: bool,
    ticking: Vec<WidgetId>,
    accelerators: Vec<WidgetId>,
    stats: FrameStats,
    images: Images,
    tooltip: Tooltip,
    focus_requested: bool,
    /// Menu mode (F10, Alt+letter): the owner and the focus to return to.
    menu_mode: Option<(WidgetId, Option<WidgetId>)>,
}

/// The root container: a vertical stack filling the window.
struct Root;
impl Widget for Root {
    fn type_name(&self) -> &'static str {
        "Window"
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        cx.fill(cx.rect, cx.theme.colors.background);
    }
}

/// Semantic id of a widget.
pub fn widget_node(id: WidgetId) -> NodeId {
    NodeId(id.to_bits() & !(1 << 63))
}
/// Semantic id of a widget's virtual item. Unique among live widgets;
/// stable for the widget's lifetime.
pub fn item_node(id: WidgetId, key: u32) -> NodeId {
    NodeId(
        (1 << 63)
            | ((u64::from(id.index()) & 0x7_ffff) << 44)
            | ((u64::from(id.generation()) & 0xfff) << 32)
            | u64::from(key),
    )
}

impl Ui {
    pub fn new(size: LSize, theme: Theme, text: Box<dyn TextSystem>) -> Self {
        let mut nodes = Arena::default();
        let root = nodes.insert(NodeData::new(
            Box::new(Root),
            Style::stack(layout::Axis::Vertical, 0),
        ));
        Self {
            nodes,
            root,
            layers: Vec::new(),
            theme,
            text,
            clipboard: Box::new(LocalClipboard::default()),
            size,
            title: String::new(),
            window: 1,
            now: 0,
            focus: None,
            focus_visible: false,
            keyboard_mode: false,
            active: true,
            remembered: None,
            hover: Vec::new(),
            capture: None,
            pointer: None,
            overlays: Vec::new(),
            notifications: Vec::new(),
            layout_dirty: true,
            paint_dirty: true,
            semantics_dirty: true,
            list: DisplayList::default(),
            painted: BTreeMap::new(),
            damage: Some(LRect::new(Lu::ZERO, Lu::ZERO, size.w, size.h)),
            semantics: None,
            semantic_map: BTreeMap::new(),
            published: None,
            semantics_pending: false,
            ticking: Vec::new(),
            accelerators: Vec::new(),
            stats: FrameStats::default(),
            images: Images::default(),
            tooltip: Tooltip::default(),
            focus_requested: false,
            menu_mode: None,
        }
    }

    // ----- construction -------------------------------------------------
    pub fn root(&self) -> WidgetId {
        self.root
    }
    pub fn add<W: Widget>(&mut self, parent: WidgetId, widget: W) -> WidgetId {
        self.add_boxed(parent, Box::new(widget))
    }
    pub fn add_boxed(&mut self, parent: WidgetId, widget: Box<dyn Widget>) -> WidgetId {
        let index = self.nodes.get(parent).map_or(0, |p| p.children.len());
        self.insert_boxed(parent, index, widget)
    }
    pub fn insert<W: Widget>(&mut self, parent: WidgetId, index: usize, widget: W) -> WidgetId {
        self.insert_boxed(parent, index, Box::new(widget))
    }
    fn insert_boxed(
        &mut self,
        parent: WidgetId,
        index: usize,
        widget: Box<dyn Widget>,
    ) -> WidgetId {
        let accel = widget.wants_accelerators();
        let style = widget.style();
        let mut data = NodeData::new(widget, style);
        let parent = if self.nodes.contains(parent) {
            Some(parent)
        } else {
            None
        };
        data.parent = parent;
        let id = self.nodes.insert(data);
        if let Some(p) = parent.and_then(|p| self.nodes.get_mut(p)) {
            let at = index.min(p.children.len());
            p.children.insert(at, id);
        }
        if accel {
            self.accelerators.push(id);
        }
        self.invalidate_layout(id, Reasons::CREATED);
        id
    }
    /// A detached subtree root (dialog, flyout content) for `open_layer`.
    pub fn create<W: Widget>(&mut self, widget: W) -> WidgetId {
        let accel = widget.wants_accelerators();
        let style = widget.style();
        let id = self.nodes.insert(NodeData::new(Box::new(widget), style));
        if accel {
            self.accelerators.push(id);
        }
        id
    }
    /// Remove a widget and its subtree. Focus moves to the next tab stop.
    pub fn remove(&mut self, id: WidgetId) {
        if id == self.root || !self.nodes.contains(id) {
            return;
        }
        let order = self.tab_order();
        let focus_inside = self.focus.is_some_and(|f| self.is_ancestor_or_self(id, f));
        let position = self.focus.and_then(|f| order.iter().position(|o| *o == f));
        if let Some(p) = self.nodes.get(id).and_then(|n| n.parent) {
            if let Some(pn) = self.nodes.get_mut(p) {
                pn.children.retain(|c| *c != id);
            }
            self.invalidate_layout(p, Reasons::LAYOUT);
        }
        if let Some(i) = self.layers.iter().position(|l| l.root == id) {
            self.layers.remove(i);
        }
        let mut stack = alloc::vec![id];
        while let Some(n) = stack.pop() {
            if let Some(data) = self.nodes.remove(n) {
                stack.extend(data.children);
            }
            self.hover.retain(|h| *h != n);
            self.overlays.retain(|o| *o != n);
            self.ticking.retain(|t| *t != n);
            self.accelerators.retain(|a| *a != n);
            if self.capture == Some(n) {
                self.capture = None;
            }
            if self.remembered == Some(n) {
                self.remembered = None;
            }
            if self
                .menu_mode
                .is_some_and(|(owner, back)| owner == n || back == Some(n))
            {
                self.menu_mode = None;
            }
            if self.tooltip.owner == Some(n) {
                self.tooltip = Tooltip::default();
            }
        }
        if focus_inside {
            self.focus = None;
            // The next tab stop after the removed one, wrapping to the first.
            let next = position.and_then(|p| {
                order[p..]
                    .iter()
                    .chain(order[..p].iter())
                    .find(|o| self.focusable(**o))
                    .copied()
            });
            let next = next.or_else(|| self.tab_order().first().copied());
            if let Some(n) = next {
                self.set_focus(Some(n), self.keyboard_mode);
            }
        }
        self.paint_dirty = true;
        self.semantics_dirty = true;
    }
    pub fn children(&self, id: WidgetId) -> &[WidgetId] {
        self.nodes.get(id).map_or(&[], |n| &n.children)
    }
    pub fn parent(&self, id: WidgetId) -> Option<WidgetId> {
        self.nodes.get(id).and_then(|n| n.parent)
    }
    pub fn contains(&self, id: WidgetId) -> bool {
        self.nodes.contains(id)
    }
    pub fn len(&self) -> usize {
        self.nodes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.nodes.len() == 0
    }
    pub fn style(&self, id: WidgetId) -> Option<&Style> {
        self.nodes.get(id).map(|n| &n.style)
    }
    pub fn set_style(&mut self, id: WidgetId, style: Style) {
        if let Some(n) = self.nodes.get_mut(id) {
            n.style = style;
            self.invalidate_layout(id, Reasons::LAYOUT);
        }
    }
    pub fn style_mut(&mut self, id: WidgetId, f: impl FnOnce(&mut Style)) {
        if let Some(n) = self.nodes.get_mut(id) {
            f(&mut n.style);
            self.invalidate_layout(id, Reasons::LAYOUT);
        }
    }
    pub fn widget<W: Widget>(&self, id: WidgetId) -> Option<&W> {
        self.nodes
            .get(id)
            .and_then(|n| n.widget.as_any().downcast_ref::<W>())
    }
    /// Mutate a widget's model from application code. Layout, paint and
    /// semantics of the widget are invalidated.
    pub fn edit<W: Widget, R>(&mut self, id: WidgetId, f: impl FnOnce(&mut W) -> R) -> Option<R> {
        let n = self.nodes.get_mut(id)?;
        let w = n.widget.as_any_mut().downcast_mut::<W>()?;
        let r = f(w);
        self.invalidate_layout(id, Reasons::LAYOUT);
        Some(r)
    }
    /// Drive a widget from application code with a full event context, for
    /// state changes that need requests (opening a menu at a point,
    /// focusing an item, notifying). Requests apply after `f` returns.
    pub fn command<W: Widget, R>(
        &mut self,
        id: WidgetId,
        f: impl FnOnce(&mut W, &mut EventCx<'_>) -> R,
    ) -> Option<R> {
        let to_window = self.to_window(id);
        let state = self.state_of(id);
        let mut requests = Vec::new();
        let r = {
            let n = self.nodes.get_mut(id)?;
            let (rect, scroll, content) = (n.rect, n.scroll, n.content);
            let w = n.widget.as_any_mut().downcast_mut::<W>()?;
            let mut cx = EventCx {
                id,
                rect,
                scroll,
                content,
                state,
                phase: Phase::Target,
                theme: &self.theme,
                text: &*self.text,
                window: self.size,
                now: self.now,
                requests: &mut requests,
                clipboard: &mut *self.clipboard,
                handled: false,
                stopped: false,
                to_window,
            };
            f(w, &mut cx)
        };
        self.apply(requests);
        self.mark(id, Reasons::PAINT);
        Some(r)
    }
    pub fn type_name(&self, id: WidgetId) -> Option<&'static str> {
        self.nodes.get(id).map(|n| n.widget.type_name())
    }
    pub fn set_enabled(&mut self, id: WidgetId, enabled: bool) {
        if let Some(n) = self.nodes.get_mut(id)
            && n.enabled != enabled
        {
            n.enabled = enabled;
            self.invalidate_subtree_paint(id);
            if !enabled && self.focus.is_some_and(|f| self.is_ancestor_or_self(id, f)) {
                let order = self.tab_order();
                self.set_focus(order.first().copied(), self.keyboard_mode);
            }
            if !enabled
                && self
                    .capture
                    .is_some_and(|c| self.is_ancestor_or_self(id, c))
            {
                self.lose_capture();
            }
        }
    }
    pub fn is_enabled(&self, id: WidgetId) -> bool {
        let mut at = Some(id);
        while let Some(i) = at {
            match self.nodes.get(i) {
                Some(n) if n.enabled => at = n.parent,
                _ => return false,
            }
        }
        true
    }
    pub fn set_visible(&mut self, id: WidgetId, visible: bool) {
        self.style_mut(id, |s| s.visible = visible);
        if !visible {
            if self.focus.is_some_and(|f| self.is_ancestor_or_self(id, f)) {
                let order = self.tab_order();
                self.set_focus(order.first().copied(), self.keyboard_mode);
            }
            if self
                .capture
                .is_some_and(|c| self.is_ancestor_or_self(id, c))
            {
                self.lose_capture();
            }
        }
    }
    /// Accessible name override (AutomationProperties.Name).
    pub fn set_name(&mut self, id: WidgetId, name: &str) {
        if let Some(n) = self.nodes.get_mut(id) {
            n.name = Some(name.into());
            self.semantics_dirty = true;
        }
    }
    pub fn set_tooltip(&mut self, id: WidgetId, text: &str) {
        if let Some(n) = self.nodes.get_mut(id) {
            n.tooltip = (!text.is_empty()).then(|| text.into());
            self.semantics_dirty = true;
        }
    }
    /// Tab order override; lower first, unset elements follow in tree order.
    pub fn set_tab_index(&mut self, id: WidgetId, index: Option<i32>) {
        if let Some(n) = self.nodes.get_mut(id) {
            n.tab_index = index;
        }
    }
    /// Whether Tab stops at `id` (WinUI `IsTabStop`); it stays focusable by
    /// pointer, programmatic focus and automation.
    pub fn set_tab_stop(&mut self, id: WidgetId, stop: bool) {
        if let Some(n) = self.nodes.get_mut(id) {
            n.tab_stop = stop;
        }
    }
    pub fn set_title(&mut self, title: &str) {
        self.title = title.into();
        self.semantics_dirty = true;
    }
    pub fn title(&self) -> &str {
        &self.title
    }
    /// Window id used in published semantic trees.
    pub fn set_window_id(&mut self, window: u64) {
        if self.window != window {
            self.window = window;
            self.semantics = None;
            self.semantics_dirty = true;
        }
    }
    pub fn set_clipboard(&mut self, clipboard: Box<dyn Clipboard>) {
        self.clipboard = clipboard;
    }
    pub fn clipboard(&mut self) -> &mut dyn Clipboard {
        &mut *self.clipboard
    }
    pub fn images(&mut self) -> &mut Images {
        &mut self.images
    }
    pub fn theme(&self) -> &Theme {
        &self.theme
    }
    pub fn text_system(&self) -> &dyn TextSystem {
        &*self.text
    }
    /// Install a new theme snapshot. Widget state is kept; paint (and
    /// layout, when text size changed) is invalidated everywhere.
    pub fn set_theme(&mut self, theme: Theme) {
        if theme == self.theme {
            return;
        }
        let relayout = theme.text_scale != self.theme.text_scale;
        self.theme = theme;
        self.text.set_text_scale(theme.text_scale);
        for id in self.nodes.ids().collect::<Vec<_>>() {
            if let Some(n) = self.nodes.get_mut(id) {
                n.reasons |= Reasons::THEME;
                if relayout {
                    n.cache.clear();
                }
            }
        }
        self.layout_dirty |= relayout;
        self.paint_dirty = true;
        self.semantics_dirty = true;
        self.damage = Some(self.window_rect());
    }
    pub fn set_text_system(&mut self, text: Box<dyn TextSystem>) {
        self.text = text;
        self.text.set_text_scale(self.theme.text_scale);
        for id in self.nodes.ids().collect::<Vec<_>>() {
            if let Some(n) = self.nodes.get_mut(id) {
                n.cache.clear();
            }
        }
        self.layout_dirty = true;
    }
    pub fn size(&self) -> LSize {
        self.size
    }
    pub fn window_rect(&self) -> LRect {
        LRect::new(Lu::ZERO, Lu::ZERO, self.size.w, self.size.h)
    }
    /// The client area changed size (logical units). Open owner popups
    /// close, as Windows closes popups when their window resizes.
    pub fn resize(&mut self, size: LSize) {
        if size == self.size {
            return;
        }
        self.size = size;
        for o in core::mem::take(&mut self.overlays) {
            self.call(o, Phase::Target, &Event::Dismiss);
        }
        if let Some(n) = self.nodes.get_mut(self.root) {
            n.reasons |= Reasons::RESIZE;
        }
        self.layout_dirty = true;
        self.paint_dirty = true;
        self.damage = Some(self.window_rect());
    }
    /// Toolkit clock in milliseconds (deterministic tests set it directly).
    pub fn set_time(&mut self, now: u64) {
        self.now = now;
    }
    pub fn now(&self) -> u64 {
        self.now
    }

    // ----- geometry -----------------------------------------------------
    /// Offset from `id`'s content coordinates to window coordinates.
    fn to_window(&self, id: WidgetId) -> LPoint {
        let mut d = LPoint::default();
        let mut at = self.nodes.get(id).and_then(|n| n.parent);
        while let Some(p) = at {
            let Some(n) = self.nodes.get(p) else { break };
            if matches!(n.style.kind, Kind::Scroll { .. }) {
                d = LPoint::new(d.x - n.scroll.x, d.y - n.scroll.y);
            }
            at = n.parent;
        }
        d
    }
    /// Window-space border box of a widget.
    pub fn rect(&self, id: WidgetId) -> Option<LRect> {
        self.nodes
            .get(id)
            .map(|n| n.rect.translate(self.to_window(id)))
    }
    /// Window position of a point in `id`'s content coordinates (as carried
    /// by `Notify::ContextRequested`).
    pub fn to_window_point(&self, id: WidgetId, p: LPoint) -> LPoint {
        p.offset(self.to_window(id))
    }
    pub fn scroll_offset(&self, id: WidgetId) -> LPoint {
        self.nodes.get(id).map_or(LPoint::default(), |n| n.scroll)
    }
    /// Scrollable content extent of a scroll container.
    pub fn scroll_extent(&self, id: WidgetId) -> (LSize, LSize) {
        self.nodes.get(id).map_or((LSize::ZERO, LSize::ZERO), |n| {
            (n.content, n.rect.inset(n.style.padding).size())
        })
    }
    fn is_ancestor_or_self(&self, ancestor: WidgetId, id: WidgetId) -> bool {
        let mut at = Some(id);
        while let Some(i) = at {
            if i == ancestor {
                return true;
            }
            at = self.nodes.get(i).and_then(|n| n.parent);
        }
        false
    }
    /// Whether `id` is laid out and painted (it and every ancestor visible,
    /// and attached to the root or an open layer).
    pub fn is_shown(&self, id: WidgetId) -> bool {
        let mut at = id;
        loop {
            let Some(n) = self.nodes.get(at) else {
                return false;
            };
            if !n.style.visible {
                return false;
            }
            match n.parent {
                Some(p) => {
                    let Some(pn) = self.nodes.get(p) else {
                        return false;
                    };
                    let index = pn.children.iter().position(|c| *c == at).unwrap_or(0);
                    if !pn.widget.child_visible(index) {
                        return false;
                    }
                    at = p;
                }
                None => return at == self.root || self.layers.iter().any(|l| l.root == at),
            }
        }
    }
    fn shown_children(&self, id: WidgetId) -> Vec<WidgetId> {
        let Some(n) = self.nodes.get(id) else {
            return Vec::new();
        };
        n.children
            .iter()
            .enumerate()
            .filter(|(i, c)| {
                n.widget.child_visible(*i) && self.nodes.get(**c).is_some_and(|c| c.style.visible)
            })
            .map(|(_, c)| *c)
            .collect()
    }
    fn hit_node(&self, id: WidgetId, p: LPoint) -> Option<WidgetId> {
        let n = self.nodes.get(id)?;
        if !n.style.visible || !n.rect.contains(p) {
            return None;
        }
        let inner = if matches!(n.style.kind, Kind::Scroll { .. }) {
            p.offset(n.scroll)
        } else {
            p
        };
        for c in self.shown_children(id).into_iter().rev() {
            if let Some(h) = self.hit_node(c, inner) {
                return Some(h);
            }
        }
        Some(id)
    }
    /// Topmost widget at a window point (owner popups, then layers, then
    /// the window content; a modal layer blocks what lies below it).
    pub fn hit_test(&self, p: LPoint) -> Option<WidgetId> {
        for o in self.overlays.iter().rev() {
            if self
                .nodes
                .get(*o)
                .and_then(|n| n.widget.overlay())
                .is_some_and(|r| r.contains(p))
            {
                return Some(*o);
            }
        }
        for layer in self.layers.iter().rev() {
            if let Some(h) = self.hit_node(layer.root, p) {
                return Some(h);
            }
            if layer.options.modal {
                return None;
            }
        }
        self.hit_node(self.root, p)
    }
    fn path(&self, id: WidgetId) -> Vec<WidgetId> {
        let mut out = Vec::new();
        let mut at = Some(id);
        while let Some(i) = at {
            out.push(i);
            at = self.nodes.get(i).and_then(|n| n.parent);
            if out.len() > layout::MAX_DEPTH {
                break;
            }
        }
        out.reverse();
        out
    }

    // ----- state and invalidation ----------------------------------------
    fn state_of(&self, id: WidgetId) -> WidgetState {
        let hovered = self.hover.contains(&id);
        let captured = self.capture == Some(id);
        let focused = self.focus == Some(id);
        WidgetState {
            hovered,
            captured,
            pressed: captured && hovered,
            focused,
            focus_visible: focused && self.focus_visible && self.active,
            enabled: self.is_enabled(id),
            window_active: self.active,
            forced: false,
        }
    }
    /// The state painted: the real one unless a visual state is forced.
    fn paint_state_of(&self, id: WidgetId) -> WidgetState {
        let real = self.state_of(id);
        let Some(forced) = self.nodes.get(id).and_then(|n| n.forced) else {
            return real;
        };
        let hovered = matches!(forced, VisualState::Hover | VisualState::Pressed);
        let pressed = forced == VisualState::Pressed;
        let focused = forced == VisualState::Focus;
        WidgetState {
            hovered,
            captured: pressed,
            pressed,
            focused,
            focus_visible: focused,
            enabled: real.enabled,
            window_active: true,
            forced: true,
        }
    }
    /// Paint `id` in a fixed visual state (state galleries, screenshot
    /// fixtures), or return it to its real state with `None`.
    pub fn force_visual_state(&mut self, id: WidgetId, state: Option<VisualState>) {
        if let Some(n) = self.nodes.get_mut(id)
            && n.forced != state
        {
            n.forced = state;
            self.mark(id, Reasons::PAINT);
        }
    }
    pub fn forced_visual_state(&self, id: WidgetId) -> Option<VisualState> {
        self.nodes.get(id).and_then(|n| n.forced)
    }
    fn mark(&mut self, id: WidgetId, reason: u8) {
        if let Some(n) = self.nodes.get_mut(id) {
            n.reasons |= reason;
        }
        self.paint_dirty = true;
        self.semantics_dirty = true;
    }
    fn invalidate_layout(&mut self, id: WidgetId, reason: u8) {
        let mut at = Some(id);
        while let Some(i) = at {
            let Some(n) = self.nodes.get_mut(i) else {
                break;
            };
            n.cache.clear();
            if i == id {
                n.reasons |= reason;
            }
            at = n.parent;
        }
        self.layout_dirty = true;
        self.paint_dirty = true;
        self.semantics_dirty = true;
    }
    fn invalidate_subtree_paint(&mut self, id: WidgetId) {
        let mut stack = alloc::vec![id];
        while let Some(i) = stack.pop() {
            if let Some(n) = self.nodes.get_mut(i) {
                n.reasons |= Reasons::PAINT;
                stack.extend(n.children.iter().copied());
            }
        }
        self.paint_dirty = true;
        self.semantics_dirty = true;
    }

    // ----- dispatch ------------------------------------------------------
    fn translate_event(event: &Event, d: LPoint) -> Event {
        let mut e = event.clone();
        match &mut e {
            Event::PointerDown(p) | Event::PointerUp(p) | Event::PointerMove(p) => {
                p.pos = p.pos.offset(d)
            }
            Event::Wheel { pos, .. } => *pos = pos.offset(d),
            _ => (),
        }
        e
    }
    /// Deliver one event to one widget; returns (handled, stopped). Window
    /// positions in `event` are converted to the widget's content space.
    fn call(&mut self, id: WidgetId, phase: Phase, event: &Event) -> (bool, bool) {
        self.call_with(id, phase, event, false)
    }
    fn call_with(
        &mut self,
        id: WidgetId,
        phase: Phase,
        event: &Event,
        already: bool,
    ) -> (bool, bool) {
        if !self.nodes.contains(id) {
            return (false, false);
        }
        let to_window = self.to_window(id);
        let local = Self::translate_event(event, LPoint::new(-to_window.x, -to_window.y));
        let state = self.state_of(id);
        let mut requests = Vec::new();
        let (handled, stopped) = {
            let Some(n) = self.nodes.get_mut(id) else {
                return (false, false);
            };
            let (rect, scroll, content) = (n.rect, n.scroll, n.content);
            let mut cx = EventCx {
                id,
                rect,
                scroll,
                content,
                state,
                phase,
                theme: &self.theme,
                text: &*self.text,
                window: self.size,
                now: self.now,
                requests: &mut requests,
                clipboard: &mut *self.clipboard,
                handled: already,
                stopped: false,
                to_window,
            };
            n.widget.event(&mut cx, &local);
            (cx.handled, cx.stopped)
        };
        self.stats.events_routed += 1;
        self.apply(requests);
        (handled, stopped)
    }
    /// Capture (root..parent), target, bubble (parent..root). Disabled
    /// widgets receive nothing.
    fn route(&mut self, target: WidgetId, event: &Event) -> bool {
        let path = self.path(target);
        let mut handled = false;
        for id in &path[..path.len().saturating_sub(1)] {
            if !self.is_enabled(*id) {
                continue;
            }
            let (h, s) = self.call_with(*id, Phase::Capture, event, handled);
            handled |= h;
            if s {
                return true;
            }
        }
        if self.is_enabled(target) {
            let (h, s) = self.call_with(target, Phase::Target, event, handled);
            handled |= h;
            if s {
                return true;
            }
        }
        for id in path[..path.len().saturating_sub(1)].iter().rev() {
            if !self.is_enabled(*id) || !self.nodes.contains(*id) {
                continue;
            }
            let (h, s) = self.call_with(*id, Phase::Bubble, event, handled);
            handled |= h;
            if s {
                return true;
            }
        }
        handled
    }
    fn apply(&mut self, requests: Vec<Request>) {
        for r in requests {
            match r {
                Request::Focus(id) => {
                    self.focus_requested = true;
                    let v = self.keyboard_mode;
                    self.set_focus(Some(id), v);
                }
                Request::Capture(id) => {
                    if self.nodes.contains(id) {
                        self.capture = Some(id);
                        self.mark(id, Reasons::PAINT);
                    }
                }
                Request::Release(id) => {
                    if self.capture == Some(id) {
                        self.capture = None;
                        self.mark(id, Reasons::PAINT);
                    }
                }
                Request::Paint(id) => self.mark(id, Reasons::PAINT),
                Request::Layout(id) => self.invalidate_layout(id, Reasons::LAYOUT),
                Request::Notify(n) => self.notifications.push(n),
                Request::ScrollIntoView(id, rect) => self.scroll_into_view(id, rect),
                Request::ScrollBy(id, d) => self.scroll_by(id, d),
                Request::Tick(id) => {
                    if !self.ticking.contains(&id) {
                        self.ticking.push(id);
                    }
                }
                Request::EnterMenuMode(id) => {
                    if self.nodes.contains(id) {
                        let back = match self.menu_mode {
                            Some((owner, back)) if owner == id => back,
                            _ => self.focus.filter(|f| *f != id),
                        };
                        self.set_focus(Some(id), true);
                        self.menu_mode = Some((id, back));
                    }
                }
                Request::ExitMenuMode(id) => {
                    if let Some((owner, back)) = self.menu_mode.take()
                        && owner == id
                        && self.focus == Some(id)
                    {
                        let v = self.keyboard_mode;
                        let back = back.filter(|b| self.focusable(*b));
                        self.set_focus(back.or(Some(id)), v);
                    }
                }
                Request::CloseLayer(id) => self.close_layer(id),
                Request::OverlayOpened(id) => {
                    if !self.overlays.contains(&id) {
                        self.overlays.push(id);
                    }
                    self.mark(id, Reasons::PAINT);
                }
                Request::OverlayClosed(id) => {
                    self.overlays.retain(|o| *o != id);
                    self.mark(id, Reasons::PAINT);
                }
                Request::Raise(id, command) => {
                    let mut handled = false;
                    let path = self.path(id);
                    for a in path[..path.len().saturating_sub(1)].iter().rev() {
                        if !self.is_enabled(*a) {
                            continue;
                        }
                        let (h, s) = self.call(*a, Phase::Bubble, &Event::Command(command));
                        handled |= h;
                        if h || s {
                            break;
                        }
                    }
                    if !handled {
                        self.notifications.push(Notification {
                            source: id,
                            kind: Notify::Command(command),
                        });
                    }
                }
            }
        }
    }
    fn scroll_into_view(&mut self, id: WidgetId, rect: LRect) {
        // `rect` is in `id`'s content space; walk scroll ancestors.
        let mut r = rect;
        let mut at = self.nodes.get(id).and_then(|n| n.parent);
        while let Some(p) = at {
            let Some(n) = self.nodes.get_mut(p) else {
                break;
            };
            if matches!(n.style.kind, Kind::Scroll { .. }) {
                let view = n.rect.inset(n.style.padding);
                let max = LPoint::new(
                    (n.content.w - view.w).max(Lu::ZERO),
                    (n.content.h - view.h).max(Lu::ZERO),
                );
                let mut s = n.scroll;
                // r is in p's content space (scrolled); visible window = view + s.
                if r.y < view.y + s.y {
                    s.y = r.y - view.y;
                } else if r.bottom() > view.bottom() + s.y {
                    s.y = r.bottom() - view.bottom();
                }
                if r.x < view.x + s.x {
                    s.x = r.x - view.x;
                } else if r.right() > view.right() + s.x {
                    s.x = r.right() - view.right();
                }
                s = LPoint::new(s.x.clamp(Lu::ZERO, max.x), s.y.clamp(Lu::ZERO, max.y));
                if s != n.scroll {
                    n.scroll = s;
                    n.reasons |= Reasons::SCROLL;
                    self.paint_dirty = true;
                    self.semantics_dirty = true;
                }
                r = r.translate(LPoint::new(-s.x, -s.y));
            }
            at = self.nodes.get(p).and_then(|n| n.parent);
        }
    }
    fn scroll_by(&mut self, id: WidgetId, d: LPoint) {
        if let Some(n) = self.nodes.get_mut(id) {
            let view = n.rect.inset(n.style.padding);
            let max = LPoint::new(
                (n.content.w - view.w).max(Lu::ZERO),
                (n.content.h - view.h).max(Lu::ZERO),
            );
            let s = LPoint::new(
                (n.scroll.x + d.x).clamp(Lu::ZERO, max.x),
                (n.scroll.y + d.y).clamp(Lu::ZERO, max.y),
            );
            if s != n.scroll {
                n.scroll = s;
                n.reasons |= Reasons::SCROLL;
                self.paint_dirty = true;
                self.semantics_dirty = true;
                if let Some(p) = self.pointer {
                    self.update_hover(p);
                }
            }
        }
    }
    /// Set a scroll container's offset directly (clamped).
    pub fn scroll_to(&mut self, id: WidgetId, offset: LPoint) {
        let current = self.scroll_offset(id);
        self.scroll_by(id, LPoint::new(offset.x - current.x, offset.y - current.y));
    }

    // ----- focus ---------------------------------------------------------
    pub fn focused(&self) -> Option<WidgetId> {
        self.focus
    }
    pub fn focus_visible(&self) -> bool {
        self.focus_visible
    }
    /// Programmatic focus (the visual follows the last input modality).
    pub fn focus(&mut self, id: WidgetId) {
        if self.focusable(id) {
            let v = self.keyboard_mode;
            self.set_focus(Some(id), v);
        }
    }
    fn focusable(&self, id: WidgetId) -> bool {
        self.nodes.get(id).is_some_and(|n| n.widget.focusable())
            && self.is_enabled(id)
            && self.is_shown(id)
    }
    fn set_focus(&mut self, id: Option<WidgetId>, visible: bool) {
        let id = id.filter(|i| self.nodes.contains(*i));
        if self.focus == id {
            if self.focus_visible != visible {
                self.focus_visible = visible;
                if let Some(f) = id {
                    self.mark(f, Reasons::FOCUS);
                }
            }
            return;
        }
        let old = self.focus;
        self.focus = id;
        self.focus_visible = visible;
        // Focus moving anywhere else ends menu mode.
        if self.menu_mode.is_some_and(|(owner, _)| Some(owner) != id) {
            self.menu_mode = None;
        }
        if let Some(o) = old {
            self.mark(o, Reasons::FOCUS);
            self.call(o, Phase::Target, &Event::FocusOut);
        }
        if let Some(n) = id {
            self.mark(n, Reasons::FOCUS);
            self.call(n, Phase::Target, &Event::FocusIn);
            if visible && let Some(r) = self.nodes.get(n).map(|d| d.rect) {
                self.scroll_into_view(n, r);
            }
        }
        self.semantics_dirty = true;
    }
    fn scope_root(&self) -> WidgetId {
        self.layers.last().map_or(self.root, |l| l.root)
    }
    /// Tab stops of the active focus scope (the topmost layer, else the
    /// window), in tab order.
    pub fn tab_order(&self) -> Vec<WidgetId> {
        let mut out = Vec::new();
        let mut stack = alloc::vec![self.scope_root()];
        while let Some(id) = stack.pop() {
            if self.focusable(id) && self.nodes.get(id).is_some_and(|n| n.tab_stop) {
                out.push(id);
            }
            for c in self.shown_children(id).into_iter().rev() {
                stack.push(c);
            }
        }
        // Stable: explicit indices first (ascending), then tree order.
        out.sort_by_key(|id| {
            self.nodes
                .get(*id)
                .and_then(|n| n.tab_index)
                .unwrap_or(i32::MAX)
        });
        out
    }
    fn move_focus(&mut self, forward: bool) {
        let order = self.tab_order();
        if order.is_empty() {
            return;
        }
        let at = self.focus.and_then(|f| order.iter().position(|o| *o == f));
        let next = match (at, forward) {
            (None, true) => 0,
            (None, false) => order.len() - 1,
            (Some(i), true) => (i + 1) % order.len(),
            (Some(i), false) => (i + order.len() - 1) % order.len(),
        };
        self.set_focus(Some(order[next]), true);
    }

    // ----- layers and popups ---------------------------------------------
    pub fn open_layer(&mut self, root: WidgetId, options: LayerOptions) {
        if !self.nodes.contains(root) || self.layers.iter().any(|l| l.root == root) {
            return;
        }
        if self.nodes.get(root).is_some_and(|n| n.parent.is_some()) {
            return;
        }
        for o in core::mem::take(&mut self.overlays) {
            self.call(o, Phase::Target, &Event::Dismiss);
        }
        if options.modal {
            self.lose_capture();
        }
        self.layers.push(Layer {
            root,
            options,
            restore: self.focus,
        });
        self.invalidate_layout(root, Reasons::CREATED);
        let first = self.tab_order().first().copied();
        let v = self.keyboard_mode;
        self.set_focus(first, v);
    }
    pub fn close_layer(&mut self, root: WidgetId) {
        let Some(i) = self.layers.iter().position(|l| l.root == root) else {
            return;
        };
        let layer = self.layers.remove(i);
        if self
            .capture
            .is_some_and(|c| self.is_ancestor_or_self(root, c))
        {
            self.lose_capture();
        }
        let hover: Vec<WidgetId> = self
            .hover
            .iter()
            .copied()
            .filter(|h| !self.is_ancestor_or_self(root, *h))
            .collect();
        self.hover = hover;
        let restore = layer.restore.filter(|r| self.focusable(*r));
        let target = restore.or_else(|| self.tab_order().first().copied());
        let v = self.keyboard_mode;
        self.set_focus(target, v);
        self.layout_dirty = true;
        self.paint_dirty = true;
        self.semantics_dirty = true;
    }
    pub fn layer_open(&self, root: WidgetId) -> bool {
        self.layers.iter().any(|l| l.root == root)
    }
    pub fn overlays(&self) -> &[WidgetId] {
        &self.overlays
    }
    fn lose_capture(&mut self) {
        if let Some(c) = self.capture.take() {
            self.mark(c, Reasons::PAINT);
            self.call(c, Phase::Target, &Event::CaptureLost);
        }
    }

    // ----- input -----------------------------------------------------------
    fn update_hover(&mut self, p: LPoint) {
        let path = self.hit_test(p).map(|t| self.path(t)).unwrap_or_default();
        if path == self.hover {
            return;
        }
        let old = core::mem::replace(&mut self.hover, path.clone());
        for o in old.iter().filter(|o| !path.contains(o)) {
            self.mark(*o, Reasons::HOVER);
            self.call(*o, Phase::Target, &Event::PointerLeave);
        }
        for n in path.iter().filter(|n| !old.contains(n)) {
            self.mark(*n, Reasons::HOVER);
            self.call(*n, Phase::Target, &Event::PointerEnter);
        }
        let owner = path
            .iter()
            .rev()
            .find(|i| self.nodes.get(**i).is_some_and(|n| n.tooltip.is_some()))
            .copied();
        if owner != self.tooltip.owner {
            if self.tooltip.shown {
                self.paint_dirty = true;
                self.semantics_dirty = true;
            }
            self.tooltip = Tooltip {
                owner,
                since: self.now,
                shown: false,
                at: p,
            };
        }
    }
    pub fn pointer_move(&mut self, p: LPoint, modifiers: Modifiers) {
        self.pointer = Some(p);
        self.update_hover(p);
        let target = self.capture.or_else(|| self.hit_test(p));
        if let Some(t) = target {
            let e = Event::PointerMove(Pointer {
                pos: p,
                button: Button::Primary,
                modifiers,
            });
            if self.capture.is_some() {
                self.call(t, Phase::Target, &e);
            } else {
                self.route(t, &e);
            }
        }
    }
    pub fn pointer_down(&mut self, p: LPoint, button: Button, modifiers: Modifiers) {
        self.keyboard_mode = false;
        self.pointer = Some(p);
        self.tooltip.shown = false;
        if let Some(&o) = self.overlays.last() {
            let inside = self
                .nodes
                .get(o)
                .and_then(|n| n.widget.overlay())
                .is_some_and(|r| r.contains(p));
            if !inside {
                self.overlays.pop();
                self.mark(o, Reasons::PAINT);
                self.call(o, Phase::Target, &Event::Dismiss);
                self.update_hover(p);
                return;
            }
        }
        if let Some(layer) = self.layers.last()
            && layer.options.light_dismiss
        {
            let root = layer.root;
            if self.nodes.get(root).is_some_and(|n| !n.rect.contains(p)) {
                self.notifications.push(Notification {
                    source: root,
                    kind: Notify::Dismissed,
                });
                self.close_layer(root);
                self.update_hover(p);
                return;
            }
        }
        self.update_hover(p);
        let Some(target) = self.hit_test(p) else {
            return;
        };
        if self.focus_visible {
            self.focus_visible = false;
            if let Some(f) = self.focus {
                self.mark(f, Reasons::FOCUS);
            }
        }
        // Focus moves on press, before the control sees the press.
        if button == Button::Primary {
            let path = self.path(target);
            if let Some(f) = path.iter().rev().find(|i| self.focusable(**i)) {
                self.set_focus(Some(*f), false);
            }
        }
        self.focus_requested = false;
        self.route(
            target,
            &Event::PointerDown(Pointer {
                pos: p,
                button,
                modifiers,
            }),
        );
    }
    pub fn pointer_up(&mut self, p: LPoint, button: Button, modifiers: Modifiers) {
        self.pointer = Some(p);
        self.update_hover(p);
        let captured = self.capture;
        let target = captured.or_else(|| self.hit_test(p));
        let Some(t) = target else { return };
        let e = Event::PointerUp(Pointer {
            pos: p,
            button,
            modifiers,
        });
        let handled = if captured.is_some() {
            self.call(t, Phase::Target, &e).0
        } else {
            self.route(t, &e)
        };
        if captured.is_some() && self.capture == captured {
            // Implicit release on button up (no CaptureLost).
            self.capture = None;
            self.mark(t, Reasons::PAINT);
        }
        if button == Button::Secondary
            && !handled
            && let Some(src) = self.path(t).into_iter().rev().find(|i| self.is_enabled(*i))
        {
            let local = p.offset(LPoint::new(-self.to_window(src).x, -self.to_window(src).y));
            self.notifications.push(Notification {
                source: src,
                kind: Notify::ContextRequested(local),
            });
        }
    }
    /// The pointer left the window.
    pub fn pointer_leave(&mut self) {
        self.pointer = None;
        for o in core::mem::take(&mut self.hover) {
            self.mark(o, Reasons::HOVER);
            self.call(o, Phase::Target, &Event::PointerLeave);
        }
        if self.tooltip.shown {
            self.paint_dirty = true;
            self.semantics_dirty = true;
        }
        self.tooltip = Tooltip::default();
    }
    pub fn wheel(&mut self, p: LPoint, delta: i32, horizontal: bool, modifiers: Modifiers) {
        self.pointer = Some(p);
        let Some(t) = self.hit_test(p) else { return };
        self.route(
            t,
            &Event::Wheel {
                pos: p,
                delta: delta.clamp(-127, 127),
                horizontal,
                modifiers,
            },
        );
        self.update_hover(p);
    }
    /// Keyboard target: an open popup (drop-down, menu) takes the keyboard
    /// while focus stays where it was, so closing it needs no restoration.
    fn key_target(&self) -> WidgetId {
        match self.overlays.last() {
            Some(o) => *o,
            None => self
                .focus
                .filter(|f| self.is_shown(*f))
                .unwrap_or_else(|| self.scope_root()),
        }
    }
    pub fn key_down(&mut self, key: KeyEvent) {
        self.keyboard_mode = true;
        self.tooltip.shown = false;
        let target = self.key_target();
        let handled = self.route(target, &Event::KeyDown(key.clone()));
        if handled {
            return;
        }
        let m = key.modifiers;
        match key.key {
            Key::Tab if !m.ctrl && !m.alt => {
                self.move_focus(!m.shift);
                return;
            }
            Key::Escape => {
                if let Some(o) = self.overlays.pop() {
                    self.mark(o, Reasons::PAINT);
                    self.call(o, Phase::Target, &Event::Dismiss);
                    return;
                }
                if let Some(l) = self.layers.last().filter(|l| l.options.light_dismiss) {
                    let root = l.root;
                    self.notifications.push(Notification {
                        source: root,
                        kind: Notify::Dismissed,
                    });
                    self.close_layer(root);
                    return;
                }
            }
            Key::Menu | Key::F(10) if key.key == Key::Menu || m.shift => {
                if let Some(f) = self.focus {
                    let c = self
                        .nodes
                        .get(f)
                        .map_or(LPoint::default(), |n| n.rect.center());
                    self.notifications.push(Notification {
                        source: f,
                        kind: Notify::ContextRequested(c),
                    });
                    return;
                }
            }
            _ => (),
        }
        for a in self.accelerators.clone() {
            if !self.is_shown(a) || !self.is_enabled(a) {
                continue;
            }
            if self.accelerate(a, &key) {
                return;
            }
        }
        let source = self.focus.unwrap_or(self.root);
        self.notifications.push(Notification {
            source,
            kind: Notify::UnhandledKey(key),
        });
    }
    fn accelerate(&mut self, id: WidgetId, key: &KeyEvent) -> bool {
        let to_window = self.to_window(id);
        let state = self.state_of(id);
        let mut requests = Vec::new();
        let consumed = {
            let Some(n) = self.nodes.get_mut(id) else {
                return false;
            };
            let (rect, scroll, content) = (n.rect, n.scroll, n.content);
            let mut cx = EventCx {
                id,
                rect,
                scroll,
                content,
                state,
                phase: Phase::Target,
                theme: &self.theme,
                text: &*self.text,
                window: self.size,
                now: self.now,
                requests: &mut requests,
                clipboard: &mut *self.clipboard,
                handled: false,
                stopped: false,
                to_window,
            };
            n.widget.accelerator(&mut cx, key)
        };
        self.apply(requests);
        consumed
    }
    pub fn key_up(&mut self, key: KeyEvent) {
        let target = self.key_target();
        self.route(target, &Event::KeyUp(key));
    }
    /// Convenience: press and release one key.
    pub fn key(&mut self, key: Key, modifiers: Modifiers) {
        let e = KeyEvent::new(key, modifiers);
        self.key_down(e.clone());
        self.key_up(e);
    }
    /// Type text into the focused widget, one character per key event.
    pub fn type_text(&mut self, s: &str) {
        for c in s.chars() {
            let e = KeyEvent {
                key: Key::Other(0),
                modifiers: Modifiers::NONE,
                text: Some(c),
            };
            self.key_down(e);
        }
    }
    pub fn composition(&mut self, c: Composition) {
        if let Some(f) = self.focus {
            self.route(f, &Event::Composition(c));
        }
    }
    /// Window activation. Deactivation closes popups and cancels capture;
    /// reactivation restores the remembered focus.
    pub fn window_focus(&mut self, active: bool) {
        if active == self.active {
            return;
        }
        self.active = active;
        if !active {
            for o in core::mem::take(&mut self.overlays) {
                self.call(o, Phase::Target, &Event::Dismiss);
            }
            self.lose_capture();
            self.remembered = self.focus;
            self.tooltip = Tooltip::default();
        } else if self.focus.is_none()
            && let Some(r) = self.remembered.take().filter(|r| self.focusable(*r))
        {
            self.set_focus(Some(r), false);
        }
        if let Some(f) = self.focus {
            self.mark(f, Reasons::FOCUS);
        }
        self.paint_dirty = true;
    }
    /// The platform cancelled pointer capture.
    pub fn capture_lost(&mut self) {
        self.lose_capture();
    }
    pub fn cursor(&self) -> Cursor {
        let Some(&t) = self.hover.last() else {
            return Cursor::Arrow;
        };
        let (Some(p), Some(n)) = (self.pointer, self.nodes.get(t)) else {
            return Cursor::Arrow;
        };
        let d = self.to_window(t);
        n.widget.cursor(p.offset(LPoint::new(-d.x, -d.y)))
    }

    // ----- application notifications, async, animation ---------------------
    pub fn take_notifications(&mut self) -> Vec<Notification> {
        core::mem::take(&mut self.notifications)
    }
    /// Start (or restart) async work for `id`; earlier tokens go stale.
    pub fn begin_async(&mut self, id: WidgetId) -> Option<AsyncToken> {
        let n = self.nodes.get_mut(id)?;
        n.async_op += 1;
        Some(AsyncToken {
            widget: id,
            op: n.async_op,
        })
    }
    /// Deliver a result. Returns false (and drops the payload) when the
    /// widget was removed, its slot reused, or newer work was started.
    pub fn deliver(&mut self, token: AsyncToken, payload: Box<dyn Any>) -> bool {
        let current = self.nodes.get(token.widget).map(|n| n.async_op);
        if current != Some(token.op) {
            self.stats.async_discarded += 1;
            return false;
        }
        let id = token.widget;
        let to_window = self.to_window(id);
        let state = self.state_of(id);
        let mut requests = Vec::new();
        if let Some(n) = self.nodes.get_mut(id) {
            let (rect, scroll, content) = (n.rect, n.scroll, n.content);
            let mut cx = EventCx {
                id,
                rect,
                scroll,
                content,
                state,
                phase: Phase::Target,
                theme: &self.theme,
                text: &*self.text,
                window: self.size,
                now: self.now,
                requests: &mut requests,
                clipboard: &mut *self.clipboard,
                handled: false,
                stopped: false,
                to_window,
            };
            n.widget.async_result(&mut cx, payload);
        }
        self.apply(requests);
        self.stats.async_delivered += 1;
        true
    }
    /// Whether animations or tooltips need `tick` calls.
    pub fn wants_tick(&self) -> bool {
        !self.ticking.is_empty() || self.tooltip.owner.is_some() && !self.tooltip.shown
    }
    pub fn tick(&mut self, now: u64) {
        self.now = now;
        for id in core::mem::take(&mut self.ticking) {
            let to_window = self.to_window(id);
            let state = self.state_of(id);
            let mut requests = Vec::new();
            if let Some(n) = self.nodes.get_mut(id) {
                let (rect, scroll, content) = (n.rect, n.scroll, n.content);
                let mut cx = EventCx {
                    id,
                    rect,
                    scroll,
                    content,
                    state,
                    phase: Phase::Target,
                    theme: &self.theme,
                    text: &*self.text,
                    window: self.size,
                    now,
                    requests: &mut requests,
                    clipboard: &mut *self.clipboard,
                    handled: false,
                    stopped: false,
                    to_window,
                };
                n.widget.tick(&mut cx);
            }
            self.apply(requests);
            self.mark(id, Reasons::PAINT);
        }
        if self.tooltip.owner.is_some()
            && !self.tooltip.shown
            && now.saturating_sub(self.tooltip.since) >= u64::from(self.theme.motion.tooltip_delay)
        {
            self.tooltip.shown = true;
            self.paint_dirty = true;
            self.semantics_dirty = true;
        }
    }
    /// Show `id`'s tool tip now (keyboard focus, deterministic tests).
    pub fn show_tooltip(&mut self, id: WidgetId) {
        if self.nodes.get(id).is_some_and(|n| n.tooltip.is_some()) {
            let at = self
                .rect(id)
                .map_or(LPoint::default(), |r| LPoint::new(r.x, r.bottom()));
            self.tooltip = Tooltip {
                owner: Some(id),
                since: self.now,
                shown: true,
                at,
            };
            self.paint_dirty = true;
            self.semantics_dirty = true;
        }
    }
    pub fn tooltip_shown(&self) -> Option<WidgetId> {
        self.tooltip.owner.filter(|_| self.tooltip.shown)
    }

    // ----- frame pipeline ----------------------------------------------------
    /// Run layout, paint and semantics as needed. Cheap when nothing changed.
    pub fn update(&mut self) {
        let any = self.layout_dirty || self.paint_dirty || self.semantics_dirty;
        if self.layout_dirty {
            self.layout_pass();
        }
        if self.paint_dirty {
            self.paint_pass();
            if !self.theme.reduced_motion {
                let animating: Vec<WidgetId> = self
                    .nodes
                    .ids()
                    .filter(|id| {
                        self.nodes.get(*id).is_some_and(|n| n.widget.animating())
                            && self.is_shown(*id)
                    })
                    .collect();
                for id in animating {
                    if !self.ticking.contains(&id) {
                        self.ticking.push(id);
                    }
                }
            }
        }
        if self.semantics_dirty {
            self.semantics_pass();
        }
        if any {
            self.stats.frames += 1;
            for id in self.nodes.ids().collect::<Vec<_>>() {
                if let Some(n) = self.nodes.get_mut(id)
                    && n.reasons != 0
                {
                    n.last_reasons = n.reasons;
                    n.reasons = 0;
                }
            }
        }
    }
    /// Pending damage in window coordinates (after `update`).
    pub fn damage(&self) -> Option<LRect> {
        self.damage
    }
    pub fn needs_frame(&self) -> bool {
        self.layout_dirty || self.paint_dirty || self.semantics_dirty || self.damage.is_some()
    }
    /// Render pending damage (or everything with `full`) and clear it.
    pub fn render(
        &mut self,
        frame: &mut Frame<'_>,
        scale: Scale,
        fonts: &mut Fonts,
        full: bool,
    ) -> RenderStats {
        self.update();
        let damage = if full {
            None
        } else {
            match self.damage.and_then(|d| d.physical(scale)) {
                Some(d) => Some(d),
                None => {
                    self.damage = None;
                    return RenderStats::default();
                }
            }
        };
        self.damage = None;
        render::render(
            &self.list,
            frame,
            scale,
            self.theme.text_scale,
            fonts,
            &self.images,
            damage,
        )
    }
    pub fn display_list(&self) -> &DisplayList {
        &self.list
    }
    pub fn stats(&self) -> FrameStats {
        self.stats
    }

    fn layout_pass(&mut self) {
        self.layout_dirty = false;
        self.stats.layout_passes += 1;
        self.stats.measures_last = 0;
        for id in self.nodes.ids().collect::<Vec<_>>() {
            if let Some(n) = self.nodes.get_mut(id) {
                n.measures = 0;
            }
        }
        let root = self.root;
        let window = self.window_rect();
        layout::arrange(self, root, window, 0);
        for i in 0..self.layers.len() {
            let (root, placement) = (self.layers[i].root, self.layers[i].options.placement);
            let avail = LSize::new(self.size.w - Lu::px(24), self.size.h - Lu::px(24));
            let m = layout::measure(self, root, avail, 0);
            let s = LSize::new(m.size.w.min(self.size.w), m.size.h.min(self.size.h));
            let origin = match placement {
                Placement::Center => LPoint::new((self.size.w - s.w) / 2, (self.size.h - s.h) / 2),
                Placement::At(p) => p,
                Placement::Below(a) => {
                    let below = a.bottom() + Lu::px(4);
                    let y = if below + s.h > self.size.h && a.y - s.h - Lu::px(4) >= Lu::ZERO {
                        a.y - s.h - Lu::px(4)
                    } else {
                        below
                    };
                    LPoint::new(a.x, y)
                }
            };
            let origin = LPoint::new(
                origin.x.clamp(Lu::ZERO, (self.size.w - s.w).max(Lu::ZERO)),
                origin.y.clamp(Lu::ZERO, (self.size.h - s.h).max(Lu::ZERO)),
            );
            layout::arrange(self, root, LRect::from_size(origin, s), 0);
        }
        if let Some(p) = self.pointer {
            self.update_hover(p);
        }
    }

    fn paint_node(&self, list: &mut DisplayList, id: WidgetId, to_window: LPoint) {
        let Some(n) = self.nodes.get(id) else { return };
        if !n.style.visible {
            return;
        }
        let scroll = matches!(n.style.kind, Kind::Scroll { .. });
        let clip = n.style.clip || scroll;
        list.begin(id);
        {
            let mut cx = PaintCx {
                id,
                rect: n.rect,
                scroll: n.scroll,
                content: n.content,
                state: self.paint_state_of(id),
                theme: &self.theme,
                text: &*self.text,
                window: self.size,
                now: self.now,
                list,
                to_window,
            };
            n.widget.paint(&mut cx);
        }
        if clip {
            list.push(Op::PushClip(n.rect.inset(if scroll {
                n.style.padding
            } else {
                Insets::ZERO
            })));
        }
        if scroll {
            list.push(Op::PushTranslate(LPoint::new(-n.scroll.x, -n.scroll.y)));
        }
        list.end();
        let child_offset = if scroll {
            LPoint::new(to_window.x - n.scroll.x, to_window.y - n.scroll.y)
        } else {
            to_window
        };
        for c in self.shown_children(id) {
            self.paint_node(list, c, child_offset);
        }
        list.begin(id);
        if scroll {
            list.push(Op::PopTranslate);
        }
        if clip {
            list.push(Op::PopClip);
        }
        let mut cx = PaintCx {
            id,
            rect: n.rect,
            scroll: n.scroll,
            content: n.content,
            state: self.paint_state_of(id),
            theme: &self.theme,
            text: &*self.text,
            window: self.size,
            now: self.now,
            list,
            to_window,
        };
        n.widget.paint_over(&mut cx);
        list.end();
    }
    fn paint_tooltip(&self, list: &mut DisplayList) {
        let Some(owner) = self.tooltip_shown() else {
            return;
        };
        let Some(text) = self.nodes.get(owner).and_then(|n| n.tooltip.clone()) else {
            return;
        };
        let c = self.theme.colors;
        let role = TextRole::Caption;
        let max = Lu::px(320);
        let lines = wrap(&*self.text, &text, role, max - Lu::px(16));
        let lh = self.text.line_height(role);
        let w = lines
            .iter()
            .map(|r| self.text.measure(&text[r.clone()], role).w)
            .fold(Lu::ZERO, Lu::max)
            + Lu::px(18);
        let h = lh * lines.len() as i32 + Lu::px(12);
        let anchor = self.rect(owner).unwrap_or_default();
        let mut x = self.tooltip.at.x;
        let mut y = anchor.bottom() + Lu::px(8);
        if y + h > self.size.h {
            y = anchor.y - h - Lu::px(8);
        }
        x = x.clamp(Lu::ZERO, (self.size.w - w).max(Lu::ZERO));
        y = y.max(Lu::ZERO);
        let r = LRect::new(x, y, w, h);
        list.begin(owner);
        list.push(Op::Shadow {
            rect: r,
            radius: Lu::px(4),
            depth: 2,
        });
        list.push(Op::RoundRect {
            rect: r,
            radius: Lu::px(4),
            color: c.tooltip_stroke,
        });
        list.push(Op::RoundRect {
            rect: r.inflate(-Lu::ONE),
            radius: Lu::px(3),
            color: c.tooltip,
        });
        for (i, line) in lines.iter().enumerate() {
            let origin = LPoint::new(x + Lu::px(9), y + Lu::px(6) + lh * i as i32);
            let s = &text[line.clone()];
            let size = self.text.measure(s, role);
            list.push_text(s, origin, role, c.text, LRect::from_size(origin, size));
        }
        list.end();
    }
    fn paint_pass(&mut self) {
        self.paint_dirty = false;
        self.stats.paint_passes += 1;
        let mut list = core::mem::take(&mut self.list);
        list.clear();
        self.paint_node(&mut list, self.root, LPoint::default());
        for layer in &self.layers {
            if layer.options.modal {
                list.begin(layer.root);
                list.push(Op::Rect {
                    rect: self.window_rect(),
                    color: self.theme.colors.smoke,
                });
                list.end();
            }
            self.paint_node(&mut list, layer.root, LPoint::default());
        }
        for o in &self.overlays {
            let Some(n) = self.nodes.get(*o) else {
                continue;
            };
            list.begin(*o);
            let mut cx = PaintCx {
                id: *o,
                rect: n.rect.translate(self.to_window(*o)),
                scroll: n.scroll,
                content: n.content,
                state: self.paint_state_of(*o),
                theme: &self.theme,
                text: &*self.text,
                window: self.size,
                now: self.now,
                list: &mut list,
                to_window: LPoint::default(),
            };
            n.widget.paint_overlay(&mut cx);
            list.end();
        }
        self.paint_tooltip(&mut list);
        let summary = list.summary();
        let mut damage = self.damage;
        let mut damaged = 0;
        for (id, (h, b)) in &summary {
            match self.painted.get(id) {
                Some((oh, ob)) if oh == h && ob == b => (),
                Some((_, ob)) => {
                    damaged += 1;
                    damage = Some(damage.map_or(ob.union(*b), |d| d.union(*ob).union(*b)));
                }
                None => {
                    damaged += 1;
                    damage = Some(damage.map_or(*b, |d| d.union(*b)));
                }
            }
        }
        for (id, (_, ob)) in &self.painted {
            if !summary.contains_key(id) {
                damaged += 1;
                damage = Some(damage.map_or(*ob, |d| d.union(*ob)));
            }
        }
        self.damage = damage.and_then(|d| d.intersect(self.window_rect()));
        self.stats.damaged_widgets_last = damaged;
        self.stats.damage_area_last = self
            .damage
            .map_or(0, |d| d.w.ceil().max(0) as u64 * d.h.ceil().max(0) as u64);
        self.stats.ops_last = list.len();
        self.painted = summary;
        self.list = list;
    }

    fn describe(
        &self,
        b: &mut TreeBuilder,
        map: &mut BTreeMap<NodeId, (WidgetId, Option<u32>)>,
        id: WidgetId,
        to_window: LPoint,
        clip: LRect,
        modal: bool,
    ) {
        let Some(n) = self.nodes.get(id) else { return };
        if !n.style.visible {
            return;
        }
        let wrect = n.rect.translate(to_window);
        let state = self.state_of(id);
        let cx = SemanticsCx {
            id,
            rect: wrect,
            state,
            theme: &self.theme,
            text: &*self.text,
            name: n.name.as_deref(),
        };
        let nid = widget_node(id);
        let mut node = Node::new(nid, Role::Group);
        let published = n.widget.semantics(&cx, &mut node);
        if published {
            node.id = nid;
            if let Some(name) = &n.name {
                node.name = name.clone();
            }
            if node.description.is_empty()
                && let Some(t) = &n.tooltip
            {
                node.description = t.clone();
            }
            let (x, y) = (wrect.x.0, wrect.y.0);
            node.bounds = Bounds {
                x,
                y,
                width: wrect.w.0,
                height: wrect.h.0,
            };
            if self.focusable(id) {
                node.states |= States::FOCUSABLE;
                node.actions |= door_accessibility::Actions::FOCUS;
            }
            if state.focused && n.widget.focused_item().is_none() {
                node.states |= States::FOCUSED;
            }
            if !state.enabled {
                node.states |= States::DISABLED;
            }
            if modal {
                node.states |= States::MODAL;
            }
            if !wrect.intersects(clip) {
                node.states |= States::OFFSCREEN;
            }
            sanitize(&mut node);
            if b.push(node).is_err() {
                return;
            }
            map.insert(nid, (id, None));
            b.enter(nid);
            let mut items = Vec::new();
            n.widget.virtual_semantics(&cx, &mut items);
            for (key, mut item) in items {
                let iid = item_node(id, key);
                let parent = item.parent.map_or(nid, |p| item_node(id, p.0 as u32));
                item.id = iid;
                if item.labelled_by.is_some_and(|l| l.0 <= u64::from(u32::MAX)) {
                    item.labelled_by = item.labelled_by.map(|l| item_node(id, l.0 as u32));
                }
                if state.focused && n.widget.focused_item() == Some(key) {
                    item.states |= States::FOCUSED;
                }
                if !state.enabled {
                    item.states |= States::DISABLED;
                }
                sanitize(&mut item);
                if b.tree_mut().push(parent, item).is_ok() {
                    map.insert(iid, (id, Some(key)));
                }
            }
        }
        let scroll = matches!(n.style.kind, Kind::Scroll { .. });
        let child_offset = if scroll {
            LPoint::new(to_window.x - n.scroll.x, to_window.y - n.scroll.y)
        } else {
            to_window
        };
        let child_clip = if scroll || n.style.clip {
            clip.intersect(wrect).unwrap_or(LRect::ZERO)
        } else {
            clip
        };
        for c in self.shown_children(id) {
            self.describe(b, map, c, child_offset, child_clip, false);
        }
        if published {
            b.leave();
        }
    }
    fn semantics_pass(&mut self) {
        self.semantics_dirty = false;
        let window = self.window_rect();
        let mut root = Node::new(widget_node(self.root), Role::Window).named(&self.title);
        root.bounds = Bounds {
            x: 0,
            y: 0,
            width: window.w.0,
            height: window.h.0,
        };
        wire::clamp(&mut root.name, wire::MAX_NAME_BYTES);
        let version = self
            .semantics
            .as_ref()
            .map_or(1, |t| t.version.wrapping_add(1).max(1));
        let mut b = TreeBuilder::new(self.window, version, root);
        let mut map = BTreeMap::new();
        map.insert(widget_node(self.root), (self.root, None));
        for c in self.shown_children(self.root) {
            self.describe(&mut b, &mut map, c, LPoint::default(), window, false);
        }
        for layer in &self.layers {
            self.describe(
                &mut b,
                &mut map,
                layer.root,
                LPoint::default(),
                window,
                layer.options.modal,
            );
        }
        if let Some(owner) = self.tooltip_shown()
            && let Some(text) = self.nodes.get(owner).and_then(|n| n.tooltip.clone())
        {
            let mut tip = Node::new(item_node(owner, u32::MAX), Role::ToolTip).named(&text);
            sanitize(&mut tip);
            let _ = b.push(tip);
        }
        let focus = self.focus.and_then(|f| {
            let n = self.nodes.get(f)?;
            match n.widget.focused_item() {
                Some(k) => Some(item_node(f, k)),
                None => Some(widget_node(f)),
            }
        });
        // An owner popup that takes the keyboard (a context menu) reports
        // its highlighted item as the semantic focus while it is open, as
        // WinUI moves focus into a MenuFlyout.
        let popup = self
            .overlays
            .last()
            .filter(|o| Some(**o) != self.focus)
            .and_then(|o| {
                let k = self.nodes.get(*o)?.widget.focused_item()?;
                Some(item_node(*o, k))
            });
        let mut tree = b.finish(popup.or(focus));
        if popup.is_some() && tree.focus.is_none() {
            tree.focus = focus.filter(|f| tree.get(*f).is_some());
        }
        // Focus falls back to the nearest published ancestor.
        if tree.focus.is_none()
            && let Some(f) = self.focus
        {
            tree.focus = self
                .path(f)
                .into_iter()
                .rev()
                .map(widget_node)
                .find(|n| tree.get(*n).is_some());
        }
        let changed = match &self.semantics {
            Some(old) => {
                let mut same = old.clone();
                same.version = tree.version;
                same != tree
            }
            None => true,
        };
        self.semantic_map = map;
        if !changed {
            return;
        }
        self.stats.semantic_versions += 1;
        self.semantics_pending = true;
        self.semantics = Some(tree);
    }
    /// The current semantic tree (after `update`).
    pub fn semantics(&mut self) -> Option<&Tree> {
        self.update();
        self.semantics.as_ref()
    }
    /// The changes since the last call, for publishing: the full tree and a
    /// delta from the tree the previous call returned (versions published
    /// in between fold into one update, so the base is what the consumer
    /// last saw).
    pub fn take_semantics(&mut self) -> Option<SemanticUpdate> {
        self.update();
        if !self.semantics_pending {
            return None;
        }
        self.semantics_pending = false;
        let tree = self.semantics.as_ref()?;
        let full = wire::encode_full(tree).ok()?;
        let delta = match &self.published {
            Some(old) if old.window == tree.window => wire::encode_delta(&tree.diff(old)).ok(),
            _ => None,
        };
        let base = self
            .published
            .as_ref()
            .map(|t| t.version)
            .filter(|_| delta.is_some());
        let update = SemanticUpdate {
            version: tree.version,
            full,
            base,
            delta,
        };
        self.published = Some(tree.clone());
        Some(update)
    }
    /// Map a semantic node to its widget (and virtual item).
    pub fn node_target(&self, node: NodeId) -> Option<(WidgetId, Option<u32>)> {
        self.semantic_map.get(&node).copied()
    }
    /// Under a modal layer only that layer and what opened above it take
    /// input (`hit_test` and the tab order stop at it the same way).
    fn in_modal_scope(&self, id: WidgetId) -> bool {
        let Some(top) = self.layers.iter().rposition(|l| l.options.modal) else {
            return true;
        };
        let root = self.path(id).first().copied();
        self.layers[top..].iter().any(|l| Some(l.root) == root)
    }
    /// Perform an assistive-technology action (already authorized by the
    /// broker). Disabled nodes, unadvertised actions and nodes outside the
    /// active modal layer are refused.
    pub fn accessibility_action(
        &mut self,
        node: NodeId,
        action: Action,
    ) -> Result<(), ActionError> {
        self.update();
        let tree = self.semantics.as_ref().ok_or(ActionError::UnknownNode)?;
        let n = tree.get(node).ok_or(ActionError::UnknownNode)?;
        if !n.accepts(&action) {
            return Err(ActionError::NotAccepted);
        }
        let (id, item) = self.node_target(node).ok_or(ActionError::UnknownNode)?;
        if !self.in_modal_scope(id) {
            return Err(ActionError::OutsideModal);
        }
        self.keyboard_mode = true;
        match (&action, item) {
            (Action::Focus, None) => {
                self.set_focus(Some(id), true);
                return Ok(());
            }
            (Action::ScrollIntoView, None) => {
                if let Some(r) = self.nodes.get(id).map(|n| n.rect) {
                    self.scroll_into_view(id, r);
                }
                return Ok(());
            }
            _ => (),
        }
        let to_window = self.to_window(id);
        let state = self.state_of(id);
        let mut requests = Vec::new();
        let ok = {
            let Some(n) = self.nodes.get_mut(id) else {
                return Err(ActionError::UnknownNode);
            };
            let (rect, scroll, content) = (n.rect, n.scroll, n.content);
            let mut cx = EventCx {
                id,
                rect,
                scroll,
                content,
                state,
                phase: Phase::Target,
                theme: &self.theme,
                text: &*self.text,
                window: self.size,
                now: self.now,
                requests: &mut requests,
                clipboard: &mut *self.clipboard,
                handled: false,
                stopped: false,
                to_window,
            };
            n.widget.action(&mut cx, item, &action)
        };
        self.apply(requests);
        self.mark(id, Reasons::PAINT);
        if ok {
            Ok(())
        } else {
            Err(ActionError::Rejected)
        }
    }

    // ----- inspector -----------------------------------------------------------
    /// Development inspector data for this window only (no cross-process
    /// inspection exists): identity, bounds, invalidation reasons, measure
    /// and paint cost, and semantic role per widget.
    pub fn inspect(&self) -> Inspection {
        let mut nodes = Vec::new();
        let mut roots = alloc::vec![self.root];
        roots.extend(self.layers.iter().map(|l| l.root));
        for r in roots {
            let mut stack = alloc::vec![(r, 0usize)];
            while let Some((id, depth)) = stack.pop() {
                let Some(n) = self.nodes.get(id) else {
                    continue;
                };
                let semantic = self
                    .semantics
                    .as_ref()
                    .and_then(|t| t.get(widget_node(id)))
                    .map(|s| (s.role, s.name.clone()));
                nodes.push(InspectNode {
                    id,
                    parent: n.parent,
                    depth,
                    type_name: n.widget.type_name(),
                    rect: self.rect(id).unwrap_or_default(),
                    layout: match n.style.kind {
                        Kind::Leaf => "leaf",
                        Kind::Stack {
                            axis: layout::Axis::Vertical,
                            ..
                        } => "column",
                        Kind::Stack { .. } => "row",
                        Kind::Grid { .. } => "grid",
                        Kind::Overlay => "overlay",
                        Kind::Scroll { .. } => "scroll",
                    },
                    shown: self.is_shown(id),
                    enabled: self.is_enabled(id),
                    focused: self.focus == Some(id),
                    measures: n.measures,
                    paint_ops: self.list.ops_of(id),
                    reasons: Reasons(n.last_reasons | n.reasons),
                    semantic,
                });
                for c in n.children.iter().rev() {
                    stack.push((*c, depth + 1));
                }
            }
        }
        Inspection {
            nodes,
            stats: self.stats,
            widgets: self.nodes.len(),
            images: self.images.count(),
            image_bytes: self.images.bytes(),
            display_ops: self.list.len(),
            theme: self.theme.name(),
            semantic_nodes: self.semantics.as_ref().map_or(0, Tree::len),
        }
    }
}

fn sanitize(node: &mut Node) {
    wire::clamp(&mut node.name, wire::MAX_NAME_BYTES);
    wire::clamp(&mut node.description, wire::MAX_DESCRIPTION_BYTES);
    wire::clamp(&mut node.accelerator, wire::MAX_ACCELERATOR_BYTES);
    let strip = |s: &mut String, breaks: bool| {
        if s.chars()
            .any(|c| c.is_control() && !(breaks && matches!(c, '\t' | '\n')))
        {
            *s = s
                .chars()
                .map(|c| {
                    if c.is_control() && !(breaks && matches!(c, '\t' | '\n')) {
                        ' '
                    } else {
                        c
                    }
                })
                .collect();
        }
    };
    strip(&mut node.name, false);
    strip(&mut node.accelerator, false);
    strip(&mut node.description, true);
    strip(&mut node.value, true);
    if node.states.contains(States::PROTECTED) {
        node.value.clear();
        node.selection = None;
        node.states.set(States::HAS_SELECTION, false);
    }
    if wire::clamp(&mut node.value, wire::MAX_VALUE_BYTES) {
        node.states |= States::VALUE_TRUNCATED;
        if let Some((a, c)) = node.selection {
            let len = node.value.len() as u32;
            node.selection = Some((a.min(len), c.min(len)));
        }
    }
    if let Some((a, c)) = node.selection {
        let ok = |i: u32| node.value.is_char_boundary(i as usize);
        if !ok(a) || !ok(c) {
            node.selection = None;
        }
    }
    node.states
        .set(States::HAS_SELECTION, node.selection.is_some());
    node.states.set(States::HAS_RANGE, node.range.is_some());
    if let Some(r) = &mut node.range {
        r.max = r.max.max(r.min);
        r.now = r.now.clamp(r.min, r.max);
        r.step = r.step.max(0);
    }
    if node
        .position
        .is_some_and(|(p, s)| p == 0 || s == 0 || p > s)
    {
        node.position = None;
    }
}

#[derive(Clone, Debug)]
pub struct InspectNode {
    pub id: WidgetId,
    pub parent: Option<WidgetId>,
    pub depth: usize,
    pub type_name: &'static str,
    pub rect: LRect,
    pub layout: &'static str,
    pub shown: bool,
    pub enabled: bool,
    pub focused: bool,
    /// Measurements during the last layout pass (cache misses).
    pub measures: u32,
    /// Display-list ops this widget recorded in the last paint.
    pub paint_ops: usize,
    /// Invalidation reasons since the previous frame.
    pub reasons: Reasons,
    pub semantic: Option<(Role, String)>,
}
#[derive(Clone, Debug)]
pub struct Inspection {
    pub nodes: Vec<InspectNode>,
    pub stats: FrameStats,
    pub widgets: usize,
    pub images: usize,
    pub image_bytes: usize,
    pub display_ops: usize,
    pub theme: &'static str,
    pub semantic_nodes: usize,
}
impl Inspection {
    /// Stable text form (for traces and the gallery inspector pane).
    pub fn text(&self) -> String {
        use core::fmt::Write;
        let mut out = String::new();
        for n in &self.nodes {
            let (x, y, w, h) = (
                n.rect.x.round(),
                n.rect.y.round(),
                n.rect.w.round(),
                n.rect.h.round(),
            );
            let _ = write!(
                out,
                "{:indent$}{} #{}.{} {} @{x},{y} {w}x{h}",
                "",
                n.type_name,
                n.id.index(),
                n.id.generation(),
                n.layout,
                indent = n.depth * 2
            );
            if !n.shown {
                out.push_str(" hidden");
            }
            if !n.enabled {
                out.push_str(" disabled");
            }
            if n.focused {
                out.push_str(" focused");
            }
            if let Some((role, name)) = &n.semantic {
                let _ = write!(out, " [{} {:?}]", role.spoken(), name);
            }
            let _ = writeln!(out, " ops={}", n.paint_ops);
        }
        let _ = writeln!(
            out,
            "widgets={} display_ops={} semantic_nodes={} images={} theme={}",
            self.widgets, self.display_ops, self.semantic_nodes, self.images, self.theme
        );
        out
    }
}

impl NodeData {
    fn new(widget: Box<dyn Widget>, style: Style) -> Self {
        Self {
            widget,
            parent: None,
            children: Vec::new(),
            style,
            enabled: true,
            rect: LRect::ZERO,
            baseline: None,
            content: LSize::ZERO,
            scroll: LPoint::default(),
            cache: Vec::new(),
            name: None,
            tooltip: None,
            tab_index: None,
            forced: None,
            tab_stop: true,
            async_op: 0,
            measures: 0,
            reasons: Reasons::CREATED,
            last_reasons: 0,
        }
    }
}

impl Host for Ui {
    fn style(&self, id: WidgetId) -> &Style {
        static DEFAULT: Style = Style {
            kind: Kind::Leaf,
            width: layout::Dim::Auto,
            height: layout::Dim::Auto,
            min: LSize::ZERO,
            max: LSize::INFINITE,
            margin: Insets::ZERO,
            padding: Insets::ZERO,
            grow: 0,
            h_align: layout::Align::Stretch,
            v_align: layout::Align::Stretch,
            cell: layout::Cell {
                row: 0,
                column: 0,
                row_span: 1,
                column_span: 1,
            },
            visible: false,
            clip: false,
        };
        self.nodes.get(id).map_or(&DEFAULT, |n| &n.style)
    }
    fn padding(&self, id: WidgetId) -> Insets {
        let Some(n) = self.nodes.get(id) else {
            return Insets::ZERO;
        };
        let cx = LayoutCx {
            theme: &self.theme,
            text: &*self.text,
        };
        n.widget.padding(&cx).unwrap_or(n.style.padding)
    }
    fn layout_children(&self, id: WidgetId) -> Vec<WidgetId> {
        self.shown_children(id)
    }
    fn measure_leaf(&mut self, id: WidgetId, content: LSize) -> Measured {
        let Some(n) = self.nodes.get(id) else {
            return Measured::default();
        };
        let cx = LayoutCx {
            theme: &self.theme,
            text: &*self.text,
        };
        n.widget.measure(&cx, content)
    }
    fn cached(&self, id: WidgetId, available: LSize) -> Option<Measured> {
        self.nodes
            .get(id)?
            .cache
            .iter()
            .find(|(a, _)| *a == available)
            .map(|(_, m)| *m)
    }
    fn cache(&mut self, id: WidgetId, available: LSize, m: Measured) {
        if let Some(n) = self.nodes.get_mut(id) {
            if n.cache.len() >= 6 {
                n.cache.remove(0);
            }
            n.cache.push((available, m));
        }
    }
    fn scroll(&self, id: WidgetId) -> LPoint {
        self.nodes.get(id).map_or(LPoint::default(), |n| n.scroll)
    }
    fn store(
        &mut self,
        id: WidgetId,
        rect: LRect,
        baseline: Option<Lu>,
        content: LSize,
        scroll: LPoint,
    ) {
        if let Some(n) = self.nodes.get_mut(id) {
            if n.rect != rect {
                n.reasons |= Reasons::LAYOUT;
            }
            n.rect = rect;
            n.baseline = baseline;
            n.content = content;
            n.scroll = scroll;
        }
    }
    fn note_measure(&mut self, id: WidgetId) {
        self.stats.measures_last += 1;
        if let Some(n) = self.nodes.get_mut(id) {
            n.measures += 1;
        }
    }
}
