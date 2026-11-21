//! The widget contract and its per-phase contexts. A widget owns its own
//! state machine; it never mutates other widgets. Side effects (focus,
//! pointer capture, invalidation, notifications to the application) are
//! requests the `Ui` applies after the call returns.
use crate::display::{DisplayList, Icon, Op};
use crate::event::{Event, KeyEvent, Phase};
use crate::id::WidgetId;
use crate::layout::{Measured, Style};
use crate::text::TextSystem;
use crate::theme::{Color, TextRole, Theme};
use crate::units::{Insets, LPoint, LRect, LSize, Lu};
use alloc::{boxed::Box, string::String, vec::Vec};
use core::any::Any;
use door_accessibility::{Action, Node};

pub trait AsAny: Any {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
impl<T: Any> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Text cursor shapes a widget may request over itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Cursor {
    #[default]
    Arrow,
    Text,
    Hand,
}

pub trait Widget: AsAny {
    /// Type name for the inspector and fixtures ("Button").
    fn type_name(&self) -> &'static str;
    /// Style applied when the widget is added (callers may change it).
    fn style(&self) -> Style {
        Style::default()
    }
    /// Intrinsic content size for `Kind::Leaf` nodes (padding excluded).
    fn measure(&self, _cx: &LayoutCx<'_>, _available: LSize) -> Measured {
        Measured::default()
    }
    /// Padding that depends on theme or text (overrides `Style::padding`),
    /// e.g. an expander's header band.
    fn padding(&self, _cx: &LayoutCx<'_>) -> Option<Insets> {
        None
    }
    /// Whether child `index` takes part in layout, paint, hit testing,
    /// focus and semantics (collapsed expanders, unselected tabs).
    fn child_visible(&self, _index: usize) -> bool {
        true
    }
    fn paint(&self, cx: &mut PaintCx<'_>);
    /// After children: focus visuals, scroll bars, adorners.
    fn paint_over(&self, _cx: &mut PaintCx<'_>) {}
    /// Window-space bounds of an open owner popup (drop-down, menu), which
    /// takes pointer input before everything else while open.
    fn overlay(&self) -> Option<LRect> {
        None
    }
    fn paint_overlay(&self, _cx: &mut PaintCx<'_>) {}
    fn event(&mut self, _cx: &mut EventCx<'_>, _event: &Event) {}
    /// Unhandled keys offered to widgets that want accelerators (menu bar
    /// F10/Alt, access keys). Return true when consumed.
    fn accelerator(&mut self, _cx: &mut EventCx<'_>, _key: &KeyEvent) -> bool {
        false
    }
    fn wants_accelerators(&self) -> bool {
        false
    }
    /// Takes keyboard focus (Tab stop) when enabled and visible.
    fn focusable(&self) -> bool {
        false
    }
    /// Describe this widget. Return false for purely structural widgets
    /// (their children attach to the nearest described ancestor).
    fn semantics(&self, _cx: &SemanticsCx<'_>, _node: &mut Node) -> bool {
        false
    }
    /// Virtual children (list rows, menu items, options): key -> node.
    fn virtual_semantics(&self, _cx: &SemanticsCx<'_>, _out: &mut Vec<(u32, Node)>) {}
    /// The virtual item that has keyboard focus inside this widget.
    fn focused_item(&self) -> Option<u32> {
        None
    }
    /// An assistive-technology action on this widget or one of its virtual
    /// items. Return false when unsupported in the current state.
    fn action(&mut self, _cx: &mut EventCx<'_>, _item: Option<u32>, _action: &Action) -> bool {
        false
    }
    /// Result of async work started with `Ui::begin_async` for this widget.
    fn async_result(&mut self, _cx: &mut EventCx<'_>, _payload: Box<dyn Any>) {}
    /// Animation tick (after `EventCx::request_tick`).
    fn tick(&mut self, _cx: &mut EventCx<'_>) {}
    fn cursor(&self, _pos: LPoint) -> Cursor {
        Cursor::Arrow
    }
    /// Continuously animating while shown (indeterminate progress); the
    /// `Ui` ticks it unless reduced motion is on.
    fn animating(&self) -> bool {
        false
    }
}

/// Interaction state the `Ui` tracks for every widget (one authoritative
/// copy: hover, capture and focus are never duplicated in widgets).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WidgetState {
    /// The pointer is over this widget or a descendant.
    pub hovered: bool,
    /// This widget holds pointer capture.
    pub captured: bool,
    /// Captured and the pointer is inside: the "pressed" visual.
    pub pressed: bool,
    pub focused: bool,
    /// Focus arrived by keyboard: draw the focus visual.
    pub focus_visible: bool,
    /// Effective enabled state (all ancestors enabled).
    pub enabled: bool,
    pub window_active: bool,
    /// A forced visual state (`Ui::force_visual_state`): controls with
    /// per-item visuals show it on their focus item.
    pub forced: bool,
}

pub struct LayoutCx<'a> {
    pub theme: &'a Theme,
    pub text: &'a dyn TextSystem,
}
impl LayoutCx<'_> {
    pub fn measure_text(&self, s: &str, role: TextRole) -> LSize {
        self.text.measure(s, role)
    }
}

/// What application code learns from controls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notify {
    /// Button, hyperlink or command invoked.
    Invoked,
    Toggled(bool),
    /// Check box state (None = indeterminate).
    Checked(Option<bool>),
    /// Range value in milli-units (slider, number box, progress).
    ValueChanged(i64),
    /// Selected item key, if any.
    SelectionChanged(Option<u32>),
    TextChanged,
    /// Enter in a single-line field.
    Submitted,
    Expanded(bool),
    /// Menu or command bar command.
    Command(u32),
    /// Dialog closed with a result (see `controls::dialog::RESULT_*`).
    Closed(u32),
    Dismissed,
    /// Right click, Shift+F10 or the Menu key, at a widget-content point.
    ContextRequested(LPoint),
    TabClosed(u32),
    /// Navigation view item activated.
    Navigated(u32),
    /// A key no widget handled (application shortcuts).
    UnhandledKey(KeyEvent),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub source: WidgetId,
    pub kind: Notify,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Request {
    Focus(WidgetId),
    Capture(WidgetId),
    Release(WidgetId),
    Paint(WidgetId),
    Layout(WidgetId),
    Notify(Notification),
    ScrollIntoView(WidgetId, LRect),
    ScrollBy(WidgetId, LPoint),
    Tick(WidgetId),
    CloseLayer(WidgetId),
    EnterMenuMode(WidgetId),
    ExitMenuMode(WidgetId),
    OverlayOpened(WidgetId),
    OverlayClosed(WidgetId),
    Raise(WidgetId, u32),
}

/// Plain-text clipboard as the platform provides it (`door.clipboard`
/// through the native adapter; an in-process store on the host).
pub trait Clipboard {
    fn set_text(&mut self, text: &str) -> bool;
    fn text(&self) -> Option<String>;
}
#[derive(Default)]
pub struct LocalClipboard(Option<String>);
impl Clipboard for LocalClipboard {
    fn set_text(&mut self, text: &str) -> bool {
        self.0 = Some(text.into());
        true
    }
    fn text(&self) -> Option<String> {
        self.0.clone()
    }
}

pub struct EventCx<'a> {
    pub id: WidgetId,
    /// Border box in this widget's content coordinates.
    pub rect: LRect,
    /// Own scroll offset and content extent (scroll containers).
    pub scroll: LPoint,
    pub content: LSize,
    pub state: WidgetState,
    pub phase: Phase,
    pub theme: &'a Theme,
    pub text: &'a dyn TextSystem,
    /// Client area size (for placing popups).
    pub window: LSize,
    /// Milliseconds on the toolkit clock.
    pub now: u64,
    pub(crate) requests: &'a mut Vec<Request>,
    pub(crate) clipboard: &'a mut dyn Clipboard,
    pub(crate) handled: bool,
    pub(crate) stopped: bool,
    /// Offset from content to window coordinates (scrolled ancestors).
    pub(crate) to_window: LPoint,
}
impl EventCx<'_> {
    pub fn set_handled(&mut self) {
        self.handled = true;
    }
    pub fn is_handled(&self) -> bool {
        self.handled
    }
    /// Stop routing to further widgets (also marks handled).
    pub fn stop(&mut self) {
        self.handled = true;
        self.stopped = true;
    }
    pub fn request_focus(&mut self) {
        self.requests.push(Request::Focus(self.id));
    }
    /// Take keyboard focus in menu mode (F10, Alt+letter on a menu bar):
    /// `exit_menu_mode` returns focus to where it was, as Windows does.
    pub fn enter_menu_mode(&mut self) {
        self.requests.push(Request::EnterMenuMode(self.id));
    }
    pub fn exit_menu_mode(&mut self) {
        self.requests.push(Request::ExitMenuMode(self.id));
    }
    pub fn capture_pointer(&mut self) {
        self.requests.push(Request::Capture(self.id));
    }
    pub fn release_pointer(&mut self) {
        self.requests.push(Request::Release(self.id));
    }
    /// Repaint and re-describe this widget.
    pub fn invalidate(&mut self) {
        self.requests.push(Request::Paint(self.id));
    }
    /// Measure and arrange again (implies repaint).
    pub fn relayout(&mut self) {
        self.requests.push(Request::Layout(self.id));
    }
    pub fn notify(&mut self, kind: Notify) {
        self.requests.push(Request::Notify(Notification {
            source: self.id,
            kind,
        }));
    }
    /// Scroll ancestors so `rect` (content coordinates) is visible.
    pub fn scroll_into_view(&mut self, rect: LRect) {
        self.requests.push(Request::ScrollIntoView(self.id, rect));
    }
    /// Scroll this widget's own scroll container content (ScrollViewer).
    pub fn scroll_by(&mut self, delta: LPoint) {
        self.requests.push(Request::ScrollBy(self.id, delta));
    }
    pub fn request_tick(&mut self) {
        self.requests.push(Request::Tick(self.id));
    }
    /// Close the layer this widget is the root of (dialogs, flyouts).
    pub fn close_layer(&mut self) {
        self.requests.push(Request::CloseLayer(self.id));
    }
    /// Tell the `Ui` this widget's owner popup opened/closed (light dismiss,
    /// Escape routing and pointer priority follow).
    pub fn overlay_opened(&mut self) {
        self.requests.push(Request::OverlayOpened(self.id));
    }
    pub fn overlay_closed(&mut self) {
        self.requests.push(Request::OverlayClosed(self.id));
    }
    /// Raise a routed command: ancestors see `Event::Command` (bubble
    /// phase); if none handles it the application gets `Notify::Command`.
    pub fn raise(&mut self, command: u32) {
        self.requests.push(Request::Raise(self.id, command));
    }
    pub fn clipboard(&mut self) -> &mut dyn Clipboard {
        self.clipboard
    }
    /// Window-space form of a content-space rectangle (popup placement).
    pub fn to_window(&self, r: LRect) -> LRect {
        r.translate(self.to_window)
    }
    pub fn from_window(&self, r: LRect) -> LRect {
        r.translate(LPoint::new(-self.to_window.x, -self.to_window.y))
    }
}

pub struct SemanticsCx<'a> {
    pub id: WidgetId,
    /// Border box in window coordinates (scroll applied).
    pub rect: LRect,
    pub state: WidgetState,
    pub theme: &'a Theme,
    pub text: &'a dyn TextSystem,
    /// Accessible name the application set (`Ui::set_name`), if any.
    pub name: Option<&'a str>,
}

pub struct PaintCx<'a> {
    pub id: WidgetId,
    /// Border box in content coordinates.
    pub rect: LRect,
    /// Own scroll offset and content extent (scroll containers).
    pub scroll: LPoint,
    pub content: LSize,
    pub state: WidgetState,
    pub theme: &'a Theme,
    pub text: &'a dyn TextSystem,
    pub window: LSize,
    pub now: u64,
    pub(crate) list: &'a mut DisplayList,
    pub(crate) to_window: LPoint,
}
impl PaintCx<'_> {
    pub fn push(&mut self, op: Op) {
        self.list.push(op);
    }
    pub fn fill(&mut self, rect: LRect, color: Color) {
        if !rect.is_empty() && color[3] != 0 {
            self.list.push(Op::Rect { rect, color });
        }
    }
    pub fn round(&mut self, rect: LRect, radius: i32, color: Color) {
        if !rect.is_empty() && color[3] != 0 {
            self.list.push(Op::RoundRect {
                rect,
                radius: Lu::px(radius),
                color,
            });
        }
    }
    pub fn border(&mut self, rect: LRect, radius: i32, width: i32, color: Color) {
        if !rect.is_empty() && color[3] != 0 {
            self.list.push(Op::Border {
                rect,
                radius: Lu::px(radius),
                width: Lu::px(width),
                color,
            });
        }
    }
    pub fn ellipse(&mut self, rect: LRect, color: Color) {
        if !rect.is_empty() && color[3] != 0 {
            self.list.push(Op::Ellipse { rect, color });
        }
    }
    pub fn line(&mut self, from: LPoint, to: LPoint, width: Lu, color: Color) {
        self.list.push(Op::Line {
            from,
            to,
            width,
            color,
        });
    }
    pub fn icon(&mut self, icon: Icon, rect: LRect, color: Color) {
        self.list.push(Op::Icon { icon, rect, color });
    }
    /// 16-DIP icon centred in `rect`.
    pub fn icon_centered(&mut self, icon: Icon, rect: LRect, size: i32, color: Color) {
        let s = Lu::px(size);
        let r = LRect::new(rect.x + (rect.w - s) / 2, rect.y + (rect.h - s) / 2, s, s);
        self.icon(icon, r, color);
    }
    pub fn shadow(&mut self, rect: LRect, radius: i32, depth: u8) {
        self.list.push(Op::Shadow {
            rect,
            radius: Lu::px(radius),
            depth,
        });
    }
    pub fn push_clip(&mut self, rect: LRect) {
        self.list.push(Op::PushClip(rect));
    }
    pub fn pop_clip(&mut self) {
        self.list.push(Op::PopClip);
    }
    pub fn push_opacity(&mut self, o: u8) {
        self.list.push(Op::PushOpacity(o));
    }
    pub fn pop_opacity(&mut self) {
        self.list.push(Op::PopOpacity);
    }
    /// One line of text with its line box at `origin`.
    pub fn text_at(&mut self, s: &str, origin: LPoint, role: TextRole, color: Color) {
        if s.is_empty() {
            return;
        }
        let size = self.text.measure(s, role);
        let extent = LRect::new(origin.x - Lu::px(2), origin.y, size.w + Lu::px(4), size.h);
        self.list.push_text(s, origin, role, color, extent);
    }
    /// Text whose stable ink band is centred vertically in `rect`, starting
    /// at `rect.x` (the Windows label placement).
    pub fn label(&mut self, s: &str, rect: LRect, role: TextRole, color: Color) {
        let (ink_top, ink_h) = self.text.ink(role);
        let top = rect.y + (rect.h - ink_h) / 2 - ink_top;
        self.text_at(s, LPoint::new(rect.x, top), role, color);
    }
    /// Label centred horizontally and vertically.
    pub fn label_centered(&mut self, s: &str, rect: LRect, role: TextRole, color: Color) {
        let w = self.text.measure(s, role).w;
        let x = rect.x + ((rect.w - w) / 2).max(Lu::ZERO);
        self.label(s, LRect::new(x, rect.y, rect.w, rect.h), role, color);
    }
    /// Label clipped and ellipsized to `rect.w`.
    pub fn label_fit(&mut self, s: &str, rect: LRect, role: TextRole, color: Color) {
        let fitted = crate::text::ellipsize(self.text, s, role, rect.w);
        self.label(&fitted, rect, role, color);
    }
    /// The Windows 11 focus visual: 2-DIP outer stroke and 1-DIP inner
    /// stroke, sitting `focus_margin` outside `rect`.
    pub fn focus_ring(&mut self, rect: LRect, radius: i32) {
        let m = self.theme.metrics;
        let outer = rect.inflate(Lu::px(m.focus_margin));
        let r = radius + m.focus_margin;
        self.border(outer, r, m.focus_outer, self.theme.colors.focus_outer);
        let inner = outer.inflate(Lu::px(-m.focus_outer));
        self.border(
            inner,
            (r - m.focus_outer).max(0),
            m.focus_inner,
            self.theme.colors.focus_inner,
        );
    }
    /// Focus visual when focus arrived by keyboard.
    pub fn focus_if_visible(&mut self, rect: LRect, radius: i32) {
        if self.state.focused && self.state.focus_visible {
            self.focus_ring(rect, radius);
        }
    }
    /// Window-space form of a content rectangle.
    pub fn to_window(&self, r: LRect) -> LRect {
        r.translate(self.to_window)
    }
}
