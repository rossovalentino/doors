//! Text, panels, buttons and toggles.
use super::{Press, Visual, button_colors, describe, surface};
use crate::display::Icon;
use crate::event::{Button as MouseButton, Event, Key, Phase};
use crate::layout::{Axis, Kind, Measured, Style, Track};
use crate::text::wrap;
use crate::theme::{TextRole, fade};
use crate::units::{Insets, LPoint, LRect, LSize, Lu};
use crate::widget::{EventCx, LayoutCx, Notify, PaintCx, SemanticsCx, Widget};
use alloc::{string::String, vec::Vec};
use door_accessibility::{Action, Actions, Node, RangeValue, Role, States};

fn max_lu(a: Lu, b: Lu) -> Lu {
    a.max(b)
}

// ---------------------------------------------------------------- TextBlock
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Primary,
    Secondary,
    Disabled,
    Accent,
}
/// Static text (TextBlock). Wraps at the available width when `wrap`.
pub struct TextBlock {
    pub text: String,
    pub role: TextRole,
    pub tone: Tone,
    pub wrap: bool,
    /// Published as a heading at this level (0: plain text).
    pub heading: u16,
}
impl TextBlock {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.into(),
            role: TextRole::Body,
            tone: Tone::Primary,
            wrap: false,
            heading: 0,
        }
    }
    pub fn role(mut self, role: TextRole) -> Self {
        self.role = role;
        self
    }
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }
    pub fn wrapping(mut self) -> Self {
        self.wrap = true;
        self
    }
    pub fn heading(mut self, level: u16) -> Self {
        self.heading = level;
        self
    }
    pub fn set_text(&mut self, text: &str) {
        self.text = text.into();
    }
}
impl Widget for TextBlock {
    fn type_name(&self) -> &'static str {
        "TextBlock"
    }
    fn style(&self) -> Style {
        Style::default().align(
            crate::layout::Align::Stretch,
            crate::layout::Align::Baseline,
        )
    }
    fn measure(&self, cx: &LayoutCx<'_>, available: LSize) -> Measured {
        let lh = cx.text.line_height(self.role);
        let base = cx.text.baseline(self.role);
        if self.wrap && !available.w.is_infinite() {
            let lines = wrap(cx.text, &self.text, self.role, available.w);
            let w = lines
                .iter()
                .map(|r| cx.text.measure(&self.text[r.clone()], self.role).w)
                .fold(Lu::ZERO, max_lu);
            return Measured::with_baseline(LSize::new(w, lh * lines.len().max(1) as i32), base);
        }
        let s = cx.text.measure(&self.text, self.role);
        Measured::with_baseline(LSize::new(s.w, s.h.max(lh)), base)
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let c = &cx.theme.colors;
        let color = match (self.tone, cx.state.enabled) {
            (_, false) | (Tone::Disabled, _) => c.text_disabled,
            (Tone::Secondary, _) => c.text_secondary,
            (Tone::Accent, _) => c.accent_text,
            (Tone::Primary, _) => c.text,
        };
        let lh = cx.text.line_height(self.role);
        let lines = if self.wrap {
            wrap(cx.text, &self.text, self.role, cx.rect.w)
        } else {
            alloc::vec![0..self.text.len()]
        };
        for (i, r) in lines.into_iter().enumerate() {
            let s = &self.text[r];
            let origin = LPoint::new(cx.rect.x, cx.rect.y + lh * i as i32);
            if self.wrap {
                cx.text_at(s, origin, self.role, color);
            } else {
                cx.push_clip(cx.rect);
                cx.text_at(s, origin, self.role, color);
                cx.pop_clip();
            }
        }
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(
            node,
            if self.heading > 0 {
                Role::Heading
            } else {
                Role::Text
            },
            &self.text,
        );
        node.level = self.heading;
        true
    }
}

// ---------------------------------------------------------------- Panel
/// Layout container (StackPanel / Grid), optionally a card surface and
/// optionally a named group for assistive technology.
pub struct Panel {
    pub card: bool,
    pub name: Option<String>,
    style: Style,
}
impl Panel {
    pub fn column(gap: i32) -> Self {
        Self {
            card: false,
            name: None,
            style: Style::stack(Axis::Vertical, gap),
        }
    }
    pub fn row(gap: i32) -> Self {
        Self {
            card: false,
            name: None,
            style: Style::stack(Axis::Horizontal, gap),
        }
    }
    pub fn grid(columns: Vec<Track>, rows: Vec<Track>, gap: i32) -> Self {
        Self {
            card: false,
            name: None,
            style: Style {
                kind: Kind::Grid {
                    columns,
                    rows,
                    column_gap: Lu::px(gap),
                    row_gap: Lu::px(gap),
                },
                ..Style::default()
            },
        }
    }
    pub fn overlay() -> Self {
        Self {
            card: false,
            name: None,
            style: Style {
                kind: Kind::Overlay,
                ..Style::default()
            },
        }
    }
    /// Settings-style card: card fill, 1-DIP stroke, 4-DIP radius, padding.
    pub fn card(mut self) -> Self {
        self.card = true;
        if self.style.padding == Insets::ZERO {
            self.style.padding = Insets::xy(16, 12);
        }
        self
    }
    pub fn named(mut self, name: &str) -> Self {
        self.name = Some(name.into());
        self
    }
    pub fn padding(mut self, p: Insets) -> Self {
        self.style.padding = p;
        self
    }
    pub fn with_style(mut self, f: impl FnOnce(&mut Style)) -> Self {
        f(&mut self.style);
        self
    }
}
impl Widget for Panel {
    fn type_name(&self) -> &'static str {
        if self.card { "Card" } else { "Panel" }
    }
    fn style(&self) -> Style {
        self.style.clone()
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        if self.card {
            let c = cx.theme.colors;
            cx.round(cx.rect, cx.theme.metrics.control_radius, c.card_stroke);
            cx.round(
                cx.rect.inflate(-Lu::ONE),
                cx.theme.metrics.control_radius - 1,
                c.card,
            );
        }
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        match &self.name {
            Some(n) => {
                describe(node, Role::Group, n);
                true
            }
            None => false,
        }
    }
}

// ---------------------------------------------------------------- Button
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonKind {
    #[default]
    Standard,
    /// AccentButtonStyle (the default/primary action).
    Accent,
    /// Text-only HyperlinkButton.
    Hyperlink,
    /// Borderless icon or command button (SubtleButton).
    Subtle,
}
/// Button (and HyperlinkButton). Invokes per `Press`; notifies `Invoked`.
pub struct Button {
    pub label: String,
    pub icon: Option<Icon>,
    pub kind: ButtonKind,
    pub accelerator: String,
    /// Raise this routed command instead of notifying `Invoked`.
    pub command: Option<u32>,
    press: Press,
}
impl Button {
    pub fn new(label: &str) -> Self {
        Self {
            label: label.into(),
            icon: None,
            kind: ButtonKind::Standard,
            accelerator: String::new(),
            command: None,
            press: Press::default(),
        }
    }
    pub fn command(mut self, command: u32) -> Self {
        self.command = Some(command);
        self
    }
    fn invoke(&self, cx: &mut EventCx<'_>) {
        match self.command {
            Some(c) => cx.raise(c),
            None => cx.notify(Notify::Invoked),
        }
    }
    pub fn accent(mut self) -> Self {
        self.kind = ButtonKind::Accent;
        self
    }
    pub fn hyperlink(mut self) -> Self {
        self.kind = ButtonKind::Hyperlink;
        self
    }
    pub fn subtle(mut self) -> Self {
        self.kind = ButtonKind::Subtle;
        self
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn accelerator(mut self, a: &str) -> Self {
        self.accelerator = a.into();
        self
    }
    pub fn pressed_visual(&self, s: &crate::widget::WidgetState) -> bool {
        self.press.pressed(s)
    }
}
pub(crate) fn content_width(cx: &LayoutCx<'_>, label: &str, icon: bool) -> Lu {
    let t = if label.is_empty() {
        Lu::ZERO
    } else {
        cx.text.measure(label, TextRole::Body).w
    };
    let i = if icon {
        Lu::px(16)
            + if label.is_empty() {
                Lu::ZERO
            } else {
                Lu::px(8)
            }
    } else {
        Lu::ZERO
    };
    t + i
}
pub(crate) fn control_height(cx: &LayoutCx<'_>) -> Lu {
    Lu::px(cx.theme.metrics.control_height).max(cx.text.line_height(TextRole::Body) + Lu::px(12))
}
impl Widget for Button {
    fn type_name(&self) -> &'static str {
        match self.kind {
            ButtonKind::Hyperlink => "HyperlinkButton",
            _ => "Button",
        }
    }
    fn style(&self) -> Style {
        Style::default().align(crate::layout::Align::Start, crate::layout::Align::Baseline)
    }
    fn measure(&self, cx: &LayoutCx<'_>, _a: LSize) -> Measured {
        let h = control_height(cx);
        let pad = if self.label.is_empty() {
            Lu::px(8)
        } else {
            Lu::px(11)
        };
        let w = (content_width(cx, &self.label, self.icon.is_some()) + pad * 2)
            .max(if self.label.is_empty() { h } else { Lu::px(32) });
        let (top, ink) = cx.text.ink(TextRole::Body);
        let text_top = (h - ink) / 2 - top;
        Measured::with_baseline(
            LSize::new(w, h),
            text_top + cx.text.baseline(TextRole::Body),
        )
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let r = cx.rect;
        let v = Visual::of(&cx.state, self.press.pressed(&cx.state));
        let t = *cx.theme;
        let (fill, mut fg) = button_colors(&t, v, self.kind == ButtonKind::Accent);
        match self.kind {
            ButtonKind::Standard | ButtonKind::Accent => {
                surface(cx, r, fill, self.kind == ButtonKind::Accent)
            }
            ButtonKind::Subtle => {
                let plate = match v {
                    Visual::Hover => t.colors.subtle_hover,
                    Visual::Pressed => t.colors.subtle_pressed,
                    _ => [0; 4],
                };
                cx.round(r, t.metrics.control_radius, plate);
                if t.is_high_contrast() && matches!(v, Visual::Hover | Visual::Pressed) {
                    fg = t.colors.text_on_accent;
                }
            }
            ButtonKind::Hyperlink => {
                let plate = match v {
                    Visual::Hover => t.colors.subtle_hover,
                    Visual::Pressed => t.colors.subtle_pressed,
                    _ => [0; 4],
                };
                cx.round(r, t.metrics.control_radius, plate);
                fg = if v == Visual::Disabled {
                    t.colors.text_disabled
                } else {
                    t.colors.link
                };
            }
        }
        let content = super::basic::content_width(
            &LayoutCx {
                theme: cx.theme,
                text: cx.text,
            },
            &self.label,
            self.icon.is_some(),
        );
        let mut x = r.x + ((r.w - content) / 2).max(Lu::px(4));
        if let Some(icon) = self.icon {
            cx.icon(
                icon,
                LRect::new(x, r.y + (r.h - Lu::px(16)) / 2, Lu::px(16), Lu::px(16)),
                fg,
            );
            x += Lu::px(24);
        }
        if !self.label.is_empty() {
            cx.push_clip(r);
            cx.label(
                &self.label,
                LRect::new(x, r.y, r.right() - x, r.h),
                TextRole::Body,
                fg,
            );
            cx.pop_clip();
        }
    }
    fn paint_over(&self, cx: &mut PaintCx<'_>) {
        let r = cx.rect;
        cx.focus_if_visible(r, cx.theme.metrics.control_radius);
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        if self.press.event(cx, e) {
            self.invoke(cx);
        }
    }
    fn focusable(&self) -> bool {
        true
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(
            node,
            if self.kind == ButtonKind::Hyperlink {
                Role::Hyperlink
            } else {
                Role::Button
            },
            &self.label,
        );
        node.actions |= Actions::INVOKE;
        node.accelerator = self.accelerator.clone();
        if self.kind == ButtonKind::Accent {
            node.states |= States::DEFAULT;
        }
        true
    }
    fn action(&mut self, cx: &mut EventCx<'_>, _item: Option<u32>, a: &Action) -> bool {
        if *a == Action::Invoke {
            self.invoke(cx);
            return true;
        }
        false
    }
    fn cursor(&self, _p: LPoint) -> crate::widget::Cursor {
        if self.kind == ButtonKind::Hyperlink {
            crate::widget::Cursor::Hand
        } else {
            crate::widget::Cursor::Arrow
        }
    }
}

// ---------------------------------------------------------------- ToggleButton
pub struct ToggleButton {
    pub label: String,
    pub icon: Option<Icon>,
    pub checked: bool,
    press: Press,
}
impl ToggleButton {
    pub fn new(label: &str) -> Self {
        Self {
            label: label.into(),
            icon: None,
            checked: false,
            press: Press::default(),
        }
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.checked = on;
        self
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    fn toggle(&mut self, cx: &mut EventCx<'_>) {
        self.checked = !self.checked;
        cx.invalidate();
        cx.notify(Notify::Toggled(self.checked));
    }
}
impl Widget for ToggleButton {
    fn type_name(&self) -> &'static str {
        "ToggleButton"
    }
    fn style(&self) -> Style {
        Style::default().align(crate::layout::Align::Start, crate::layout::Align::Baseline)
    }
    fn measure(&self, cx: &LayoutCx<'_>, a: LSize) -> Measured {
        let b = Button {
            label: self.label.clone(),
            icon: self.icon,
            kind: ButtonKind::Standard,
            accelerator: String::new(),
            command: None,
            press: Press::default(),
        };
        b.measure(cx, a)
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let b = Button {
            label: self.label.clone(),
            icon: self.icon,
            kind: if self.checked {
                ButtonKind::Accent
            } else {
                ButtonKind::Standard
            },
            accelerator: String::new(),
            command: None,
            press: self.press,
        };
        b.paint(cx);
    }
    fn paint_over(&self, cx: &mut PaintCx<'_>) {
        let r = cx.rect;
        cx.focus_if_visible(r, cx.theme.metrics.control_radius);
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        if self.press.event(cx, e) {
            self.toggle(cx);
        }
    }
    fn focusable(&self) -> bool {
        true
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(node, Role::ToggleButton, &self.label);
        node.states |= States::CHECKABLE;
        node.states.set(States::CHECKED, self.checked);
        node.actions |= Actions::TOGGLE;
        true
    }
    fn action(&mut self, cx: &mut EventCx<'_>, _i: Option<u32>, a: &Action) -> bool {
        if *a == Action::Toggle {
            self.toggle(cx);
            return true;
        }
        false
    }
}

// ---------------------------------------------------------------- CheckBox
/// Two- or three-state check box. `state`: Some(checked) or None
/// (indeterminate). Space toggles on key up; Enter does not (WinUI).
pub struct CheckBox {
    pub label: String,
    pub state: Option<bool>,
    pub three_state: bool,
    press: Press,
}
impl CheckBox {
    pub fn new(label: &str) -> Self {
        Self {
            label: label.into(),
            state: Some(false),
            three_state: false,
            press: Press::default(),
        }
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.state = Some(on);
        self
    }
    pub fn indeterminate(mut self) -> Self {
        self.three_state = true;
        self.state = None;
        self
    }
    pub fn three_state(mut self) -> Self {
        self.three_state = true;
        self
    }
    /// Unchecked -> checked -> (indeterminate) -> unchecked.
    pub fn cycle(&mut self) {
        self.state = match self.state {
            Some(false) => Some(true),
            Some(true) if self.three_state => None,
            Some(true) => Some(false),
            None => Some(false),
        };
    }
}
impl Widget for CheckBox {
    fn type_name(&self) -> &'static str {
        "CheckBox"
    }
    fn style(&self) -> Style {
        Style::default().align(crate::layout::Align::Start, crate::layout::Align::Baseline)
    }
    fn measure(&self, cx: &LayoutCx<'_>, _a: LSize) -> Measured {
        let h = control_height(cx);
        let w = Lu::px(28)
            + if self.label.is_empty() {
                -Lu::px(8)
            } else {
                cx.text.measure(&self.label, TextRole::Body).w
            };
        let (top, ink) = cx.text.ink(TextRole::Body);
        Measured::with_baseline(
            LSize::new(w.max(Lu::px(20)), h),
            (h - ink) / 2 - top + cx.text.baseline(TextRole::Body),
        )
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let t = *cx.theme;
        let c = t.colors;
        let r = cx.rect;
        let v = Visual::of(&cx.state, self.press.pressed(&cx.state));
        let b = LRect::new(r.x, r.y + (r.h - Lu::px(20)) / 2, Lu::px(20), Lu::px(20));
        let on = self.state != Some(false);
        if t.is_high_contrast() {
            // WinUI high contrast: Highlight box when checked (inverted while
            // pointer over), Highlight stroke while pointer over or pressed,
            // GrayText when disabled.
            let (fill, stroke, glyph) = match (v, on) {
                (Visual::Disabled, _) => (c.control_disabled, c.strong_disabled, c.text_disabled),
                (Visual::Rest, true) => (c.accent, c.accent, c.text_on_accent),
                (Visual::Hover, true) => (c.text_on_accent, c.accent, c.accent),
                (Visual::Pressed, _) => (c.accent, c.accent, c.text_on_accent),
                (Visual::Rest, false) => (c.control_alt, c.strong, c.text),
                (Visual::Hover, false) => (c.control_alt, c.accent, c.text),
            };
            cx.round(b, 4, stroke);
            cx.round(b.inflate(-Lu::ONE), 3, fill);
            if on {
                let g = LRect::new(b.x + Lu::px(4), b.y + Lu::px(4), Lu::px(12), Lu::px(12));
                cx.icon(
                    if self.state.is_none() {
                        Icon::Dash
                    } else {
                        Icon::Check
                    },
                    g,
                    glyph,
                );
            }
        } else if on {
            let fill = match v {
                Visual::Rest => c.accent,
                Visual::Hover => c.accent_hover,
                Visual::Pressed => c.accent_pressed,
                Visual::Disabled => c.accent_disabled,
            };
            cx.round(b, 4, fill);
            let glyph = if v == Visual::Disabled {
                c.text_on_accent_disabled
            } else {
                c.text_on_accent
            };
            let g = LRect::new(b.x + Lu::px(4), b.y + Lu::px(4), Lu::px(12), Lu::px(12));
            cx.icon(
                if self.state.is_none() {
                    Icon::Dash
                } else {
                    Icon::Check
                },
                g,
                glyph,
            );
        } else {
            let (fill, stroke) = match v {
                Visual::Rest => (c.control_alt, c.strong),
                Visual::Hover => (c.control_alt_hover, c.strong),
                Visual::Pressed => (c.control_pressed, c.strong_disabled),
                Visual::Disabled => (c.control_disabled, c.strong_disabled),
            };
            cx.round(b, 4, stroke);
            cx.round(b.inflate(-Lu::ONE), 3, fill);
        }
        if !self.label.is_empty() {
            let fg = if v == Visual::Disabled {
                c.text_disabled
            } else {
                c.text
            };
            cx.label(
                &self.label,
                LRect::new(r.x + Lu::px(28), r.y, r.w - Lu::px(28), r.h),
                TextRole::Body,
                fg,
            );
        }
    }
    fn paint_over(&self, cx: &mut PaintCx<'_>) {
        let r = cx.rect;
        cx.focus_if_visible(r, cx.theme.metrics.control_radius);
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        if matches!(e, Event::KeyDown(k) if k.key == Key::Enter) {
            return;
        }
        if self.press.event(cx, e) {
            self.cycle();
            cx.invalidate();
            cx.notify(Notify::Checked(self.state));
        }
    }
    fn focusable(&self) -> bool {
        true
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(node, Role::CheckBox, &self.label);
        node.states |= States::CHECKABLE;
        node.states.set(States::CHECKED, self.state == Some(true));
        node.states.set(States::MIXED, self.state.is_none());
        node.actions |= Actions::TOGGLE;
        true
    }
    fn action(&mut self, cx: &mut EventCx<'_>, _i: Option<u32>, a: &Action) -> bool {
        if *a == Action::Toggle {
            self.cycle();
            cx.notify(Notify::Checked(self.state));
            return true;
        }
        false
    }
}

// ---------------------------------------------------------------- RadioButtons
/// RadioButtons group: one tab stop; arrow keys move focus and selection
/// together; each option is a virtual RadioButton for assistive technology.
pub struct RadioButtons {
    pub header: String,
    pub items: Vec<String>,
    pub selected: Option<usize>,
    pub disabled: Vec<usize>,
    focus: usize,
    pressed: Option<usize>,
    hover: Option<usize>,
}
impl RadioButtons {
    pub fn new(header: &str, items: &[&str]) -> Self {
        Self {
            header: header.into(),
            items: items.iter().map(|s| String::from(*s)).collect(),
            selected: None,
            disabled: Vec::new(),
            focus: 0,
            pressed: None,
            hover: None,
        }
    }
    pub fn selected(mut self, i: usize) -> Self {
        if i < self.items.len() {
            self.selected = Some(i);
            self.focus = i;
        }
        self
    }
    fn header_h(&self, lh: Lu) -> Lu {
        if self.header.is_empty() {
            Lu::ZERO
        } else {
            lh + Lu::px(8)
        }
    }
    fn row(&self, cx_rect: LRect, lh: Lu, i: usize) -> LRect {
        LRect::new(
            cx_rect.x,
            cx_rect.y + self.header_h(lh) + Lu::px(32) * i as i32,
            cx_rect.w,
            Lu::px(32),
        )
    }
    fn index_at(&self, r: LRect, lh: Lu, p: LPoint) -> Option<usize> {
        (0..self.items.len()).find(|i| self.row(r, lh, *i).contains(p))
    }
    fn select(&mut self, cx: &mut EventCx<'_>, i: usize) {
        if i < self.items.len() && !self.disabled.contains(&i) {
            self.focus = i;
            if self.selected != Some(i) {
                self.selected = Some(i);
                cx.notify(Notify::SelectionChanged(Some(i as u32)));
            }
            cx.invalidate();
        }
    }
    fn step(&self, from: usize, forward: bool) -> usize {
        let n = self.items.len();
        let mut i = from;
        for _ in 0..n {
            i = if forward {
                (i + 1) % n
            } else {
                (i + n - 1) % n
            };
            if !self.disabled.contains(&i) {
                return i;
            }
        }
        from
    }
}
impl Widget for RadioButtons {
    fn type_name(&self) -> &'static str {
        "RadioButtons"
    }
    fn style(&self) -> Style {
        Style::default().align(crate::layout::Align::Start, crate::layout::Align::Start)
    }
    fn measure(&self, cx: &LayoutCx<'_>, _a: LSize) -> Measured {
        let lh = cx.text.line_height(TextRole::Body);
        let w = self
            .items
            .iter()
            .map(|s| cx.text.measure(s, TextRole::Body).w + Lu::px(28))
            .fold(cx.text.measure(&self.header, TextRole::Body).w, max_lu);
        Measured::new(LSize::new(
            w,
            self.header_h(lh) + Lu::px(32) * self.items.len() as i32,
        ))
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let c = cx.theme.colors;
        let lh = cx.text.line_height(TextRole::Body);
        let r = cx.rect;
        if !self.header.is_empty() {
            let fg = if cx.state.enabled {
                c.text
            } else {
                c.text_disabled
            };
            cx.text_at(&self.header, r.origin(), TextRole::Body, fg);
        }
        for (i, label) in self.items.iter().enumerate() {
            let row = self.row(r, lh, i);
            let enabled = cx.state.enabled && !self.disabled.contains(&i);
            // A forced visual state (state galleries) shows on the focus item.
            let item = |at: Option<usize>| at == Some(i) || cx.state.forced && i == self.focus;
            let pressed = cx.state.captured && item(self.pressed);
            let hover = enabled && cx.state.hovered && item(self.hover);
            let dot = LRect::new(row.x, row.y + Lu::px(6), Lu::px(20), Lu::px(20));
            if self.selected == Some(i) {
                cx.ellipse(
                    dot,
                    if enabled {
                        if hover { c.accent_hover } else { c.accent }
                    } else {
                        c.accent_disabled
                    },
                );
                let inner = if pressed {
                    6
                } else if hover {
                    10
                } else {
                    8
                };
                let d = Lu::px(inner);
                cx.ellipse(
                    LRect::new(dot.x + (dot.w - d) / 2, dot.y + (dot.h - d) / 2, d, d),
                    if enabled {
                        c.text_on_accent
                    } else {
                        c.text_on_accent_disabled
                    },
                );
            } else {
                cx.ellipse(dot, if enabled { c.strong } else { c.strong_disabled });
                let fill = if pressed {
                    c.control_pressed
                } else if hover {
                    c.control_alt_hover
                } else {
                    c.control_alt
                };
                cx.ellipse(dot.inflate(-Lu::ONE), fill);
                if pressed {
                    let d = Lu::px(10);
                    cx.ellipse(
                        LRect::new(dot.x + (dot.w - d) / 2, dot.y + (dot.h - d) / 2, d, d),
                        c.strong,
                    );
                }
            }
            let fg = if enabled { c.text } else { c.text_disabled };
            cx.label(
                label,
                LRect::new(row.x + Lu::px(28), row.y, row.w - Lu::px(28), row.h),
                TextRole::Body,
                fg,
            );
        }
    }
    fn paint_over(&self, cx: &mut PaintCx<'_>) {
        let lh = cx.text.line_height(TextRole::Body);
        let row = self.row(cx.rect, lh, self.focus);
        let w = cx
            .text
            .measure(self.items.get(self.focus).map_or("", |s| s), TextRole::Body)
            .w
            + Lu::px(28);
        cx.focus_if_visible(
            LRect::new(row.x, row.y, w.min(row.w), row.h),
            cx.theme.metrics.control_radius,
        );
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        if cx.phase != Phase::Target || self.items.is_empty() {
            return;
        }
        let lh = cx.text.line_height(TextRole::Body);
        match e {
            Event::PointerMove(p) => {
                let h = self.index_at(cx.rect, lh, p.pos);
                if h != self.hover {
                    self.hover = h;
                    cx.invalidate();
                }
            }
            Event::PointerLeave => {
                self.hover = None;
                cx.invalidate();
            }
            Event::PointerDown(p) if p.button == MouseButton::Primary => {
                self.pressed = self
                    .index_at(cx.rect, lh, p.pos)
                    .filter(|i| !self.disabled.contains(i));
                if self.pressed.is_some() {
                    cx.capture_pointer();
                }
                cx.invalidate();
                cx.set_handled();
            }
            Event::PointerUp(p) if p.button == MouseButton::Primary => {
                if let Some(i) = self.pressed.take()
                    && self.index_at(cx.rect, lh, p.pos) == Some(i)
                {
                    self.select(cx, i);
                }
                cx.invalidate();
                cx.set_handled();
            }
            Event::CaptureLost => {
                self.pressed = None;
                cx.invalidate();
            }
            Event::KeyDown(k) if k.modifiers.none() => {
                let next = match k.key {
                    Key::Down | Key::Right => Some(self.step(self.focus, true)),
                    Key::Up | Key::Left => Some(self.step(self.focus, false)),
                    Key::Space => Some(self.focus),
                    _ => None,
                };
                if let Some(i) = next {
                    self.select(cx, i);
                    let row = self.row(cx.rect, lh, i);
                    cx.scroll_into_view(row);
                    cx.set_handled();
                }
            }
            _ => (),
        }
    }
    fn focusable(&self) -> bool {
        !self.items.is_empty()
    }
    fn focused_item(&self) -> Option<u32> {
        Some(self.focus as u32)
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(node, Role::Group, &self.header);
        true
    }
    fn virtual_semantics(&self, cx: &SemanticsCx<'_>, out: &mut Vec<(u32, Node)>) {
        let lh = cx.text.line_height(TextRole::Body);
        for (i, label) in self.items.iter().enumerate() {
            let mut n = Node::new(door_accessibility::NodeId(1), Role::RadioButton).named(label);
            let r = self.row(cx.rect, lh, i);
            n.bounds = door_accessibility::Bounds {
                x: r.x.0,
                y: r.y.0,
                width: r.w.0,
                height: r.h.0,
            };
            n.states.set(States::SELECTED, self.selected == Some(i));
            n.states.set(States::DISABLED, self.disabled.contains(&i));
            n.states |= States::FOCUSABLE;
            n.actions = Actions::SELECT | Actions::FOCUS;
            n.position = Some((i as u32 + 1, self.items.len() as u32));
            out.push((i as u32, n));
        }
    }
    fn action(&mut self, cx: &mut EventCx<'_>, item: Option<u32>, a: &Action) -> bool {
        let Some(i) = item
            .map(|i| i as usize)
            .filter(|i| *i < self.items.len() && !self.disabled.contains(i))
        else {
            return false;
        };
        match a {
            Action::Select => {
                self.select(cx, i);
                true
            }
            Action::Focus => {
                self.focus = i;
                cx.request_focus();
                cx.invalidate();
                true
            }
            _ => false,
        }
    }
}

// ---------------------------------------------------------------- ToggleSwitch
/// ToggleSwitch: the whole control toggles on tap; Space toggles; the knob
/// slides over `motion.normal` (instant under reduced motion).
pub struct ToggleSwitch {
    pub header: String,
    pub on: bool,
    pub on_text: String,
    pub off_text: String,
    press: Press,
    changed_at: Option<u64>,
}
impl ToggleSwitch {
    pub fn new(header: &str) -> Self {
        Self {
            header: header.into(),
            on: false,
            on_text: "On".into(),
            off_text: "Off".into(),
            press: Press::default(),
            changed_at: None,
        }
    }
    pub fn on(mut self, on: bool) -> Self {
        self.on = on;
        self
    }
    fn header_h(&self, cx_text: &dyn crate::text::TextSystem) -> Lu {
        if self.header.is_empty() {
            Lu::ZERO
        } else {
            cx_text.line_height(TextRole::Body) + Lu::px(4)
        }
    }
    fn toggle(&mut self, cx: &mut EventCx<'_>) {
        self.on = !self.on;
        self.changed_at = Some(cx.now);
        if cx.theme.motion.normal > 0 {
            cx.request_tick();
        }
        cx.invalidate();
        cx.notify(Notify::Toggled(self.on));
    }
}
impl Widget for ToggleSwitch {
    fn type_name(&self) -> &'static str {
        "ToggleSwitch"
    }
    fn style(&self) -> Style {
        Style::default().align(crate::layout::Align::Start, crate::layout::Align::Start)
    }
    fn measure(&self, cx: &LayoutCx<'_>, _a: LSize) -> Measured {
        let state = cx
            .text
            .measure(&self.on_text, TextRole::Body)
            .w
            .max(cx.text.measure(&self.off_text, TextRole::Body).w);
        let w = (Lu::px(52) + state).max(cx.text.measure(&self.header, TextRole::Body).w);
        Measured::new(LSize::new(w, self.header_h(cx.text) + control_height(cx)))
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let c = cx.theme.colors;
        let r = cx.rect;
        let hh = self.header_h(cx.text);
        if !self.header.is_empty() {
            cx.text_at(
                &self.header,
                r.origin(),
                TextRole::Body,
                if cx.state.enabled {
                    c.text
                } else {
                    c.text_disabled
                },
            );
        }
        let row = LRect::new(r.x, r.y + hh, r.w, r.h - hh);
        let track = LRect::new(
            row.x,
            row.y + (row.h - Lu::px(20)) / 2,
            Lu::px(40),
            Lu::px(20),
        );
        let v = Visual::of(&cx.state, self.press.pressed(&cx.state));
        // WinUI knob: 12 at rest, 14 pointer over, 17x14 pressed (it
        // stretches toward the track's centre).
        let knob = Lu::px(match v {
            Visual::Hover | Visual::Pressed => 14,
            _ => 12,
        });
        let stretch = if v == Visual::Pressed {
            Lu::px(3)
        } else {
            Lu::ZERO
        };
        // Knob position with animation progress.
        let duration = u64::from(cx.theme.motion.normal);
        let p = match self.changed_at {
            Some(t) if duration > 0 => {
                (cx.now.saturating_sub(t).min(duration) * 64 / duration) as i32
            }
            _ => 64,
        };
        let travel = Lu::px(20);
        let from = if self.on { Lu::ZERO } else { travel };
        let to = if self.on { travel } else { Lu::ZERO };
        let offset = from + (to - from).mul_div(p, 64);
        let kx = track.x + Lu::px(4) + offset
            - (knob - Lu::px(12)) / 2
            - if self.on { stretch } else { Lu::ZERO };
        let ky = track.y + (track.h - knob) / 2;
        let knob_rect = LRect::new(kx, ky, knob + stretch, knob);
        let radius = knob.round() / 2;
        if self.on {
            let fill = match v {
                Visual::Disabled => c.accent_disabled,
                Visual::Hover => c.accent_hover,
                Visual::Pressed => c.accent_pressed,
                Visual::Rest => c.accent,
            };
            cx.round(track, 10, fill);
            cx.round(
                knob_rect,
                radius,
                if v == Visual::Disabled {
                    c.text_on_accent_disabled
                } else {
                    c.text_on_accent
                },
            );
        } else {
            let stroke = if v == Visual::Disabled {
                c.strong_disabled
            } else {
                c.strong
            };
            cx.round(track, 10, stroke);
            cx.round(
                track.inflate(-Lu::ONE),
                9,
                if v == Visual::Hover {
                    c.control_alt_hover
                } else {
                    c.control_alt
                },
            );
            cx.round(knob_rect, radius, stroke);
        }
        let state = if self.on {
            &self.on_text
        } else {
            &self.off_text
        };
        let fg = if v == Visual::Disabled {
            c.text_disabled
        } else {
            c.text
        };
        cx.label(
            state,
            LRect::new(track.right() + Lu::px(12), row.y, row.w - Lu::px(52), row.h),
            TextRole::Body,
            fg,
        );
    }
    fn paint_over(&self, cx: &mut PaintCx<'_>) {
        let hh = self.header_h(cx.text);
        let r = cx.rect;
        let track = LRect::new(
            r.x,
            r.y + hh + (r.h - hh - Lu::px(20)) / 2,
            Lu::px(40),
            Lu::px(20),
        );
        cx.focus_if_visible(track, 10);
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        if matches!(e, Event::KeyDown(k) if k.key == Key::Enter) {
            return;
        }
        if self.press.event(cx, e) {
            self.toggle(cx);
        }
    }
    fn tick(&mut self, cx: &mut EventCx<'_>) {
        if let Some(t) = self.changed_at {
            if cx.now.saturating_sub(t) < u64::from(cx.theme.motion.normal) {
                cx.request_tick();
            } else {
                self.changed_at = None;
            }
            cx.invalidate();
        }
    }
    fn focusable(&self) -> bool {
        true
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(node, Role::ToggleSwitch, &self.header);
        node.states |= States::CHECKABLE;
        node.states.set(States::CHECKED, self.on);
        node.actions |= Actions::TOGGLE;
        true
    }
    fn action(&mut self, cx: &mut EventCx<'_>, _i: Option<u32>, a: &Action) -> bool {
        if *a == Action::Toggle {
            self.toggle(cx);
            return true;
        }
        false
    }
}

/// Fill a range-value node from integral values.
pub(crate) fn range_node(node: &mut Node, min: i64, max: i64, now: i64, step: i64) {
    node.range = Some(RangeValue::integral(min, max, now, step));
    node.states |= States::HAS_RANGE;
}
/// Muted plate colour helper for list-like rows.
pub(crate) fn row_plate(
    t: &crate::theme::Theme,
    selected: bool,
    hover: bool,
    pressed: bool,
) -> crate::theme::Color {
    let c = &t.colors;
    if pressed {
        c.subtle_pressed
    } else if selected && hover {
        c.subtle_hover
    } else if selected {
        c.subtle_selected
    } else if hover {
        c.subtle_hover
    } else {
        fade(c.subtle_hover, 0)
    }
}
