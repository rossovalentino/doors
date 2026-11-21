//! ScrollViewer, Expander, InfoBar and the ToolTip visual.
use super::describe;
use crate::display::Icon;
use crate::event::{Button as MouseButton, Event, Key, Phase};
use crate::layout::{Axis, Kind, Measured, Style};
use crate::text::wrap;
use crate::theme::TextRole;
use crate::units::{Insets, LPoint, LRect, LSize, Lu};
use crate::widget::{EventCx, LayoutCx, Notify, PaintCx, SemanticsCx, Widget};
use alloc::string::String;
use door_accessibility::{Action, Actions, Node, Role, States};

/// One wheel detent (three 16-DIP lines, as Windows scrolls by default).
pub const WHEEL_STEP: i32 = 48;

/// ScrollViewer: one scrolled child; wheel, keyboard (when the content
/// leaves the key unhandled) and an overlay scroll bar that widens on hover.
pub struct ScrollViewer {
    pub horizontal: bool,
    drag: Option<(Lu, Lu)>,
    bar_hover: bool,
}
impl ScrollViewer {
    pub fn vertical() -> Self {
        Self {
            horizontal: false,
            drag: None,
            bar_hover: false,
        }
    }
    fn bar(&self, r: LRect, content: LSize, scroll: LPoint) -> Option<(LRect, LRect)> {
        if content.h <= r.h {
            return None;
        }
        let w = Lu::px(if self.bar_hover || self.drag.is_some() {
            6
        } else {
            2
        });
        let track = LRect::new(
            r.right() - w - Lu::px(3),
            r.y + Lu::px(4),
            w,
            r.h - Lu::px(8),
        );
        let len = track
            .h
            .mul_div(r.h.0, content.h.0.max(1))
            .max(Lu::px(24))
            .min(track.h);
        let max = (content.h - r.h).max(Lu::ONE);
        let y = track.y + (track.h - len).mul_div(scroll.y.0, max.0);
        Some((track, LRect::new(track.x, y, w, len)))
    }
}
impl Widget for ScrollViewer {
    fn type_name(&self) -> &'static str {
        "ScrollViewer"
    }
    fn style(&self) -> Style {
        Style {
            kind: Kind::Scroll {
                horizontal: self.horizontal,
                vertical: true,
            },
            clip: true,
            ..Style::default()
        }
        .grow(1)
    }
    fn paint(&self, _cx: &mut PaintCx<'_>) {}
    fn paint_over(&self, cx: &mut PaintCx<'_>) {
        if let Some((track, thumb)) = self.bar(cx.rect, cx.content, cx.scroll) {
            if self.bar_hover || self.drag.is_some() {
                cx.round(track, 3, crate::theme::fade(cx.theme.colors.card, 200));
            }
            cx.round(thumb, 3, cx.theme.colors.scrollbar);
        }
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        let view = cx.rect.h;
        match e {
            Event::Wheel {
                delta,
                horizontal: false,
                ..
            } if !cx.is_handled() && cx.content.h > view => {
                cx.scroll_by(LPoint::new(Lu::ZERO, Lu::px(-delta * WHEEL_STEP)));
                cx.set_handled();
            }
            Event::PointerMove(p) => {
                let near = self.bar(cx.rect, cx.content, cx.scroll).is_some()
                    && p.pos.x >= cx.rect.right() - Lu::px(14);
                if near != self.bar_hover {
                    self.bar_hover = near;
                    cx.invalidate();
                }
                if let (Some((grab, start)), true) = (self.drag, cx.state.captured)
                    && let Some((track, thumb)) = self.bar(cx.rect, cx.content, cx.scroll)
                {
                    let travel = (track.h - thumb.h).max(Lu::ONE);
                    let max = cx.content.h - view;
                    let target = start + (p.pos.y - grab).mul_div(max.0, travel.0);
                    cx.scroll_by(LPoint::new(Lu::ZERO, target - cx.scroll.y));
                }
            }
            Event::PointerLeave => {
                self.bar_hover = false;
                cx.invalidate();
            }
            Event::PointerDown(p) if cx.phase == Phase::Capture || cx.phase == Phase::Target => {
                if p.button == MouseButton::Primary
                    && p.pos.x >= cx.rect.right() - Lu::px(14)
                    && let Some((_, thumb)) = self.bar(cx.rect, cx.content, cx.scroll)
                {
                    if thumb.inflate(Lu::px(4)).contains(p.pos) {
                        self.drag = Some((p.pos.y, cx.scroll.y));
                    } else {
                        let page = if p.pos.y < thumb.y { -view } else { view };
                        cx.scroll_by(LPoint::new(Lu::ZERO, page));
                    }
                    cx.capture_pointer();
                    cx.stop();
                }
            }
            Event::PointerUp(_) | Event::CaptureLost => {
                if self.drag.take().is_some() {
                    cx.invalidate();
                }
            }
            Event::KeyDown(k)
                if cx.phase == Phase::Bubble && !cx.is_handled() && cx.content.h > view =>
            {
                let d = match k.key {
                    Key::Up if k.modifiers.none() => Some(-Lu::px(WHEEL_STEP)),
                    Key::Down if k.modifiers.none() => Some(Lu::px(WHEEL_STEP)),
                    Key::PageUp => Some(-view),
                    Key::PageDown => Some(view),
                    Key::Home if k.modifiers.ctrl => Some(-cx.content.h),
                    Key::End if k.modifiers.ctrl => Some(cx.content.h),
                    _ => None,
                };
                if let Some(d) = d {
                    cx.scroll_by(LPoint::new(Lu::ZERO, d));
                    cx.set_handled();
                }
            }
            _ => (),
        }
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(node, Role::ScrollViewer, "");
        node.states |= States::VERTICAL;
        true
    }
}

/// Expander: a header card that shows or hides its content (first child).
pub struct Expander {
    pub header: String,
    pub description: String,
    pub expanded: bool,
    pressed: bool,
}
impl Expander {
    pub fn new(header: &str) -> Self {
        Self {
            header: header.into(),
            description: String::new(),
            expanded: false,
            pressed: false,
        }
    }
    pub fn description(mut self, d: &str) -> Self {
        self.description = d.into();
        self
    }
    pub fn expanded(mut self, on: bool) -> Self {
        self.expanded = on;
        self
    }
    fn header_h(&self) -> Lu {
        Lu::px(if self.description.is_empty() { 48 } else { 64 })
    }
    fn toggle(&mut self, cx: &mut EventCx<'_>) {
        self.expanded = !self.expanded;
        cx.relayout();
        cx.notify(Notify::Expanded(self.expanded));
    }
}
impl Widget for Expander {
    fn type_name(&self) -> &'static str {
        "Expander"
    }
    fn style(&self) -> Style {
        Style::stack(Axis::Vertical, 0)
    }
    fn padding(&self, _cx: &LayoutCx<'_>) -> Option<Insets> {
        let h = self.header_h();
        Some(if self.expanded {
            Insets {
                left: Lu::px(16),
                top: h + Lu::px(16),
                right: Lu::px(16),
                bottom: Lu::px(16),
            }
        } else {
            Insets {
                left: Lu::ZERO,
                top: h,
                right: Lu::ZERO,
                bottom: Lu::ZERO,
            }
        })
    }
    fn child_visible(&self, _index: usize) -> bool {
        self.expanded
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let c = cx.theme.colors;
        let r = cx.rect;
        let h = self.header_h();
        let header = LRect::new(r.x, r.y, r.w, h);
        if self.expanded {
            cx.round(r, 4, c.card_stroke);
            cx.round(r.inflate(-Lu::ONE), 3, c.background);
        }
        let fill = if self.pressed && cx.state.captured {
            c.card_pressed
        } else if cx.state.hovered && cx.state.enabled {
            c.card_hover
        } else {
            c.card
        };
        cx.round(header, 4, c.card_stroke);
        cx.round(header.inflate(-Lu::ONE), 3, fill);
        let fg = if cx.state.enabled {
            c.text
        } else {
            c.text_disabled
        };
        if self.description.is_empty() {
            cx.label(
                &self.header,
                LRect::new(r.x + Lu::px(16), r.y, r.w - Lu::px(64), h),
                TextRole::Body,
                fg,
            );
        } else {
            cx.text_at(
                &self.header,
                LPoint::new(r.x + Lu::px(16), r.y + Lu::px(12)),
                TextRole::Body,
                fg,
            );
            cx.text_at(
                &self.description,
                LPoint::new(r.x + Lu::px(16), r.y + Lu::px(32)),
                TextRole::Caption,
                c.text_secondary,
            );
        }
        let chevron = LRect::new(
            r.right() - Lu::px(48),
            r.y + (h - Lu::px(32)) / 2,
            Lu::px(32),
            Lu::px(32),
        );
        if cx.state.hovered && cx.state.enabled {
            cx.round(chevron, 4, c.subtle_hover);
        }
        cx.icon_centered(
            if self.expanded {
                Icon::ChevronUp
            } else {
                Icon::ChevronDown
            },
            chevron,
            12,
            fg,
        );
    }
    fn paint_over(&self, cx: &mut PaintCx<'_>) {
        let r = cx.rect;
        let h = self.header_h();
        cx.focus_if_visible(LRect::new(r.x, r.y, r.w, h), 4);
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        if cx.phase != Phase::Target {
            return;
        }
        let header = LRect::new(cx.rect.x, cx.rect.y, cx.rect.w, self.header_h());
        match e {
            Event::PointerDown(p) if p.button == MouseButton::Primary && header.contains(p.pos) => {
                self.pressed = true;
                cx.capture_pointer();
                cx.invalidate();
                cx.set_handled();
            }
            Event::PointerUp(p) if self.pressed => {
                self.pressed = false;
                if cx.state.captured && header.contains(p.pos) {
                    self.toggle(cx);
                }
                cx.invalidate();
            }
            Event::CaptureLost => {
                self.pressed = false;
                cx.invalidate();
            }
            Event::PointerEnter | Event::PointerLeave => cx.invalidate(),
            Event::KeyDown(k) if matches!(k.key, Key::Space | Key::Enter) && k.modifiers.none() => {
                self.toggle(cx);
                cx.set_handled();
            }
            _ => (),
        }
    }
    fn focusable(&self) -> bool {
        true
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(node, Role::Expander, &self.header);
        node.description = self.description.clone();
        node.states |= States::EXPANDABLE;
        node.states.set(States::EXPANDED, self.expanded);
        node.actions |= if self.expanded {
            Actions::COLLAPSE
        } else {
            Actions::EXPAND
        };
        true
    }
    fn action(&mut self, cx: &mut EventCx<'_>, _i: Option<u32>, a: &Action) -> bool {
        match (a, self.expanded) {
            (Action::Expand, false) | (Action::Collapse, true) => {
                self.toggle(cx);
                true
            }
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Severity {
    #[default]
    Informational,
    Success,
    Warning,
    Error,
}
/// InfoBar: an inline status message with severity, optional close button,
/// announced as a live region.
pub struct InfoBar {
    pub severity: Severity,
    pub title: String,
    pub message: String,
    pub closable: bool,
    pub open: bool,
    close_pressed: bool,
}
impl InfoBar {
    pub fn new(severity: Severity, title: &str, message: &str) -> Self {
        Self {
            severity,
            title: title.into(),
            message: message.into(),
            closable: true,
            open: true,
            close_pressed: false,
        }
    }
    pub fn closable(mut self, on: bool) -> Self {
        self.closable = on;
        self
    }
    fn close_rect(&self, r: LRect) -> LRect {
        LRect::new(
            r.right() - Lu::px(44),
            r.y + Lu::px(5),
            Lu::px(38),
            Lu::px(38),
        )
    }
    fn lines(
        &self,
        cx_text: &dyn crate::text::TextSystem,
        width: Lu,
    ) -> (bool, alloc::vec::Vec<core::ops::Range<usize>>) {
        let title_w = cx_text.measure(&self.title, TextRole::BodyStrong).w + Lu::px(12);
        let room = width
            - Lu::px(48)
            - if self.closable {
                Lu::px(48)
            } else {
                Lu::px(16)
            };
        let inline = title_w + cx_text.measure(&self.message, TextRole::Body).w <= room;
        let lines = if inline {
            alloc::vec![0..self.message.len()]
        } else {
            wrap(cx_text, &self.message, TextRole::Body, room)
        };
        (inline, lines)
    }
    fn close(&mut self, cx: &mut EventCx<'_>) {
        self.open = false;
        cx.relayout();
        cx.notify(Notify::Dismissed);
    }
}
impl Widget for InfoBar {
    fn type_name(&self) -> &'static str {
        "InfoBar"
    }
    fn measure(&self, cx: &LayoutCx<'_>, a: LSize) -> Measured {
        if !self.open {
            return Measured::default();
        }
        let width = if a.w.is_infinite() { Lu::px(480) } else { a.w };
        let (inline, lines) = self.lines(cx.text, width);
        let lh = cx.text.line_height(TextRole::Body);
        let h = if inline {
            Lu::px(48)
        } else {
            Lu::px(16) + lh * (lines.len() as i32 + 1) + Lu::px(14)
        };
        Measured::new(LSize::new(width, h.max(Lu::px(48))))
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        if !self.open {
            return;
        }
        let c = cx.theme.colors;
        let r = cx.rect;
        let (badge, back, icon) = match self.severity {
            Severity::Informational => (c.attention, c.attention_background, Icon::Info),
            Severity::Success => (c.success, c.success_background, Icon::Success),
            Severity::Warning => (c.caution, c.caution_background, Icon::Warning),
            Severity::Error => (c.critical, c.critical_background, Icon::Error),
        };
        cx.round(r, 4, c.card_stroke);
        cx.round(r.inflate(-Lu::ONE), 3, back);
        let b = LRect::new(r.x + Lu::px(16), r.y + Lu::px(16), Lu::px(16), Lu::px(16));
        cx.ellipse(b, badge);
        cx.icon(
            icon,
            b,
            if cx.theme.is_high_contrast() {
                c.background
            } else {
                c.text_on_accent
            },
        );
        let (inline, lines) = self.lines(cx.text, r.w);
        let x = r.x + Lu::px(48);
        let lh = cx.text.line_height(TextRole::Body);
        let first = LRect::new(x, r.y + Lu::px(14), r.w, lh);
        cx.label(&self.title, first, TextRole::BodyStrong, c.text);
        let tw = cx.text.measure(&self.title, TextRole::BodyStrong).w + Lu::px(12);
        for (i, line) in lines.iter().enumerate() {
            let at = if inline {
                LRect::new(x + tw, first.y, r.w, lh)
            } else {
                LRect::new(x, first.y + lh * (i as i32 + 1), r.w, lh)
            };
            cx.label(&self.message[line.clone()], at, TextRole::Body, c.text);
        }
        if self.closable {
            let cr = self.close_rect(r);
            if cx.state.hovered {
                cx.round(
                    cr,
                    4,
                    if self.close_pressed && cx.state.captured {
                        c.subtle_pressed
                    } else {
                        c.subtle_hover
                    },
                );
            }
            cx.icon_centered(Icon::Close, cr, 12, c.text);
        }
    }
    fn paint_over(&self, cx: &mut PaintCx<'_>) {
        if self.open && self.closable {
            let cr = self.close_rect(cx.rect);
            cx.focus_if_visible(cr, 4);
        }
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        if cx.phase != Phase::Target || !self.open || !self.closable {
            return;
        }
        let cr = self.close_rect(cx.rect);
        match e {
            Event::PointerDown(p) if p.button == MouseButton::Primary && cr.contains(p.pos) => {
                self.close_pressed = true;
                cx.capture_pointer();
                cx.invalidate();
                cx.set_handled();
            }
            Event::PointerUp(p) if self.close_pressed => {
                self.close_pressed = false;
                if cx.state.captured && cr.contains(p.pos) {
                    self.close(cx);
                }
                cx.invalidate();
            }
            Event::CaptureLost | Event::PointerEnter | Event::PointerLeave => {
                self.close_pressed = false;
                cx.invalidate();
            }
            Event::KeyDown(k) if matches!(k.key, Key::Space | Key::Enter) => {
                self.close(cx);
                cx.set_handled();
            }
            _ => (),
        }
    }
    fn focusable(&self) -> bool {
        self.open && self.closable
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        if !self.open {
            return false;
        }
        describe(node, Role::InfoBar, &self.title);
        node.description = self.message.clone();
        node.states |= if matches!(self.severity, Severity::Error | Severity::Warning) {
            States::LIVE_ASSERTIVE
        } else {
            States::LIVE_POLITE
        };
        if self.closable {
            node.actions |= Actions::DISMISS;
        }
        true
    }
    fn action(&mut self, cx: &mut EventCx<'_>, _i: Option<u32>, a: &Action) -> bool {
        if *a == Action::Dismiss && self.closable && self.open {
            self.close(cx);
            return true;
        }
        false
    }
}

/// The ToolTip visual as a standalone element (gallery state display; real
/// tool tips come from `Ui::set_tooltip`).
pub struct ToolTip {
    pub text: String,
}
impl ToolTip {
    pub fn new(text: &str) -> Self {
        Self { text: text.into() }
    }
}
impl Widget for ToolTip {
    fn type_name(&self) -> &'static str {
        "ToolTip"
    }
    fn style(&self) -> Style {
        Style::default().align(crate::layout::Align::Start, crate::layout::Align::Start)
    }
    fn measure(&self, cx: &LayoutCx<'_>, _a: LSize) -> Measured {
        let s = cx.text.measure(&self.text, TextRole::Caption);
        Measured::new(LSize::new(s.w + Lu::px(18), s.h + Lu::px(12)))
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let c = cx.theme.colors;
        let r = cx.rect;
        cx.shadow(r, 4, 2);
        cx.round(r, 4, c.tooltip_stroke);
        cx.round(r.inflate(-Lu::ONE), 3, c.tooltip);
        cx.text_at(
            &self.text,
            LPoint::new(r.x + Lu::px(9), r.y + Lu::px(6)),
            TextRole::Caption,
            c.text,
        );
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(node, Role::ToolTip, &self.text);
        true
    }
}
