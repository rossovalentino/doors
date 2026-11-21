//! ComboBox: a selection field with a drop-down list. Closed, arrow keys
//! change the selection directly (WinUI); Alt+Down, F4, Enter or Space open
//! it; open, arrows move the highlight, Enter/Space commit, Escape cancels.
use super::basic::control_height;
use super::menu::bounds;
use super::{Press, Visual, button_colors, describe, surface};
use crate::display::Icon;
use crate::event::{Button as MouseButton, Event, Key, Phase};
use crate::layout::{Measured, Style};
use crate::theme::TextRole;
use crate::units::{LPoint, LRect, LSize, Lu};
use crate::widget::{EventCx, LayoutCx, Notify, PaintCx, SemanticsCx, Widget};
use alloc::{string::String, vec::Vec};
use door_accessibility::{Action, Actions, Node, NodeId, Role, States};

const ITEM: i32 = 36;
const MAX_VISIBLE: usize = 12;
const LIST_KEY: u32 = 0xffff_0000;

pub struct ComboBox {
    pub header: String,
    pub placeholder: String,
    pub items: Vec<String>,
    pub selected: Option<usize>,
    open: Option<(LRect, usize)>,
    highlight: usize,
    press: Press,
}
impl ComboBox {
    pub fn new(header: &str, items: &[&str]) -> Self {
        Self {
            header: header.into(),
            placeholder: String::new(),
            items: items.iter().map(|s| String::from(*s)).collect(),
            selected: None,
            open: None,
            highlight: 0,
            press: Press::default(),
        }
    }
    pub fn selected(mut self, i: usize) -> Self {
        self.selected = (i < self.items.len()).then_some(i);
        self
    }
    pub fn placeholder(mut self, p: &str) -> Self {
        self.placeholder = p.into();
        self
    }
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }
    fn header_h(&self, t: &dyn crate::text::TextSystem) -> Lu {
        if self.header.is_empty() {
            Lu::ZERO
        } else {
            t.line_height(TextRole::Body) + Lu::px(8)
        }
    }
    fn field(&self, r: LRect, t: &dyn crate::text::TextSystem) -> LRect {
        let h = self.header_h(t);
        LRect::new(r.x, r.y + h, r.w, r.h - h)
    }
    fn set(&mut self, cx: &mut EventCx<'_>, i: usize) {
        if i < self.items.len() && self.selected != Some(i) {
            self.selected = Some(i);
            cx.notify(Notify::SelectionChanged(Some(i as u32)));
        }
        cx.invalidate();
    }
    /// Open the list with the current item over the field (WinUI placement),
    /// clamped to the window.
    fn open_list(&mut self, cx: &mut EventCx<'_>) {
        if self.items.is_empty() {
            return;
        }
        let field = cx.to_window(self.field(cx.rect, cx.text));
        let rows = self.items.len().min(MAX_VISIBLE);
        let h = Lu::px(ITEM) * rows as i32 + Lu::px(8);
        let w = self
            .items
            .iter()
            .map(|s| cx.text.measure(s, TextRole::Body).w + Lu::px(44))
            .fold(field.w, Lu::max);
        self.highlight = self.selected.unwrap_or(0);
        let top = self
            .highlight
            .saturating_sub(rows / 2)
            .min(self.items.len() - rows);
        let y = field.y - Lu::px(4) - Lu::px(ITEM) * (self.highlight - top) as i32
            + (field.h - Lu::px(ITEM)) / 2;
        let y = y.clamp(Lu::ZERO, (cx.window.h - h).max(Lu::ZERO));
        let x = field.x.clamp(Lu::ZERO, (cx.window.w - w).max(Lu::ZERO));
        self.open = Some((LRect::new(x, y, w, h), top));
        cx.overlay_opened();
        cx.invalidate();
    }
    fn close(&mut self, cx: &mut EventCx<'_>) {
        if self.open.take().is_some() {
            cx.overlay_closed();
        }
        cx.invalidate();
    }
    fn row(&self, i: usize) -> Option<LRect> {
        let (r, top) = self.open?;
        let rows = self.items.len().min(MAX_VISIBLE);
        (i >= top && i < top + rows).then(|| {
            LRect::new(
                r.x + Lu::px(4),
                r.y + Lu::px(4) + Lu::px(ITEM) * (i - top) as i32,
                r.w - Lu::px(8),
                Lu::px(ITEM),
            )
        })
    }
    fn reveal(&mut self) {
        if let Some((r, top)) = self.open {
            let rows = self.items.len().min(MAX_VISIBLE);
            let top = if self.highlight < top {
                self.highlight
            } else if self.highlight >= top + rows {
                self.highlight + 1 - rows
            } else {
                top
            };
            self.open = Some((r, top));
        }
    }
}
impl Widget for ComboBox {
    fn type_name(&self) -> &'static str {
        "ComboBox"
    }
    fn style(&self) -> Style {
        Style::default().align(crate::layout::Align::Start, crate::layout::Align::Baseline)
    }
    fn measure(&self, cx: &LayoutCx<'_>, _a: LSize) -> Measured {
        let w = self
            .items
            .iter()
            .chain(core::iter::once(&self.placeholder))
            .map(|s| cx.text.measure(s, TextRole::Body).w)
            .fold(Lu::px(64), Lu::max)
            + Lu::px(48);
        let w = w.max(cx.text.measure(&self.header, TextRole::Body).w);
        let h = control_height(cx);
        let hh = self.header_h(cx.text);
        let (top, ink) = cx.text.ink(TextRole::Body);
        Measured::with_baseline(
            LSize::new(w, hh + h),
            hh + (h - ink) / 2 - top + cx.text.baseline(TextRole::Body),
        )
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let t = *cx.theme;
        let c = t.colors;
        let r = cx.rect;
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
        let f = self.field(r, cx.text);
        let v = Visual::of(
            &cx.state,
            self.press.pressed(&cx.state) || self.open.is_some(),
        );
        let (fill, fg) = button_colors(&t, v, false);
        surface(cx, f, fill, false);
        let (text, color) = match self.selected {
            Some(i) => (self.items[i].as_str(), fg),
            None => (
                self.placeholder.as_str(),
                if v == Visual::Disabled {
                    c.text_disabled
                } else {
                    c.text_secondary
                },
            ),
        };
        cx.push_clip(f);
        cx.label_fit(
            text,
            LRect::new(f.x + Lu::px(12), f.y, f.w - Lu::px(44), f.h),
            TextRole::Body,
            color,
        );
        cx.pop_clip();
        cx.icon(
            Icon::ChevronDown,
            LRect::new(
                f.right() - Lu::px(30),
                f.y + (f.h - Lu::px(12)) / 2,
                Lu::px(12),
                Lu::px(12),
            ),
            if v == Visual::Disabled {
                c.text_disabled
            } else {
                c.text_secondary
            },
        );
    }
    fn paint_over(&self, cx: &mut PaintCx<'_>) {
        let f = self.field(cx.rect, cx.text);
        cx.focus_if_visible(f, cx.theme.metrics.control_radius);
    }
    fn overlay(&self) -> Option<LRect> {
        self.open.map(|(r, _)| r)
    }
    fn paint_overlay(&self, cx: &mut PaintCx<'_>) {
        let Some((r, _)) = self.open else { return };
        let c = cx.theme.colors;
        cx.shadow(r, 8, 4);
        cx.round(r, 8, c.flyout_stroke);
        cx.round(r.inflate(-Lu::ONE), 7, c.flyout);
        for (i, label) in self.items.iter().enumerate() {
            let Some(row) = self.row(i) else { continue };
            let hot = i == self.highlight;
            if hot {
                cx.round(
                    LRect::new(row.x, row.y + Lu::px(2), row.w, row.h - Lu::px(4)),
                    4,
                    c.subtle_hover,
                );
            }
            if self.selected == Some(i) {
                cx.round(
                    LRect::new(row.x, row.y + Lu::px(10), Lu::px(3), Lu::px(16)),
                    2,
                    c.accent,
                );
            }
            let fg = if hot && cx.theme.is_high_contrast() {
                c.control_text_hover
            } else {
                c.text
            };
            cx.label_fit(
                label,
                LRect::new(row.x + Lu::px(12), row.y, row.w - Lu::px(20), row.h),
                TextRole::Body,
                fg,
            );
        }
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        if cx.phase != Phase::Target {
            return;
        }
        if let Some((list, _)) = self.open {
            let wp = |p: LPoint| {
                cx.to_window(LRect::new(p.x, p.y, Lu::ZERO, Lu::ZERO))
                    .origin()
            };
            match e {
                Event::Dismiss => {
                    self.open = None;
                    cx.invalidate();
                }
                Event::PointerMove(p) => {
                    let q = wp(p.pos);
                    if let Some(i) =
                        (0..self.items.len()).find(|i| self.row(*i).is_some_and(|r| r.contains(q)))
                    {
                        self.highlight = i;
                        cx.invalidate();
                    }
                }
                Event::PointerUp(p) if p.button == MouseButton::Primary => {
                    let q = wp(p.pos);
                    if let Some(i) =
                        (0..self.items.len()).find(|i| self.row(*i).is_some_and(|r| r.contains(q)))
                    {
                        self.set(cx, i);
                        self.close(cx);
                    }
                    cx.set_handled();
                }
                Event::PointerDown(_) => cx.set_handled(),
                Event::Wheel { delta, .. } => {
                    let n = self.items.len();
                    self.highlight =
                        (self.highlight as i32 - delta).clamp(0, n as i32 - 1) as usize;
                    self.reveal();
                    cx.invalidate();
                    cx.set_handled();
                }
                Event::KeyDown(k) => {
                    cx.set_handled();
                    let n = self.items.len();
                    match k.key {
                        Key::Down if !k.modifiers.alt => {
                            self.highlight = (self.highlight + 1).min(n - 1)
                        }
                        Key::Up if !k.modifiers.alt => {
                            self.highlight = self.highlight.saturating_sub(1)
                        }
                        Key::Home => self.highlight = 0,
                        Key::End => self.highlight = n - 1,
                        Key::PageDown => self.highlight = (self.highlight + MAX_VISIBLE).min(n - 1),
                        Key::PageUp => self.highlight = self.highlight.saturating_sub(MAX_VISIBLE),
                        Key::Enter | Key::Space | Key::Tab => {
                            let h = self.highlight;
                            self.set(cx, h);
                            self.close(cx);
                            if k.key == Key::Tab {
                                cx.handled = false;
                            }
                            return;
                        }
                        Key::Escape | Key::Up | Key::Down | Key::F(4) => {
                            self.close(cx);
                            return;
                        }
                        _ => return,
                    }
                    self.reveal();
                    cx.invalidate();
                }
                _ => (),
            }
            let _ = list;
            return;
        }
        if self.press.event(cx, e) {
            self.open_list(cx);
            return;
        }
        if let Event::KeyDown(k) = e {
            let n = self.items.len();
            if n == 0 {
                return;
            }
            let cur = self.selected.unwrap_or(0);
            let next = match k.key {
                Key::Down if k.modifiers.alt => {
                    self.open_list(cx);
                    None
                }
                Key::F(4) => {
                    self.open_list(cx);
                    None
                }
                Key::Down | Key::Right if k.modifiers.none() => {
                    Some((cur + usize::from(self.selected.is_some())).min(n - 1))
                }
                Key::Up | Key::Left if k.modifiers.none() => Some(cur.saturating_sub(1)),
                Key::Home => Some(0),
                Key::End => Some(n - 1),
                Key::Char(ch) if k.modifiers.none() => {
                    // Type-ahead by first letter, cycling from the current item.
                    (1..=n).map(|d| (cur + d) % n).find(|i| {
                        self.items[*i]
                            .chars()
                            .next()
                            .is_some_and(|f| f.to_ascii_lowercase() == ch)
                    })
                }
                _ => return,
            };
            if let Some(i) = next {
                self.set(cx, i);
            }
            cx.set_handled();
        }
    }
    fn focusable(&self) -> bool {
        true
    }
    fn focused_item(&self) -> Option<u32> {
        self.open.map(|_| self.highlight as u32)
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(node, Role::ComboBox, &self.header);
        node.value = self
            .selected
            .map_or(String::new(), |i| self.items[i].clone());
        node.description = self.placeholder.clone();
        node.states |= States::EXPANDABLE | States::HAS_POPUP;
        node.states.set(States::EXPANDED, self.open.is_some());
        node.actions |= if self.open.is_some() {
            Actions::COLLAPSE
        } else {
            Actions::EXPAND
        };
        true
    }
    fn virtual_semantics(&self, _cx: &SemanticsCx<'_>, out: &mut Vec<(u32, Node)>) {
        // A collapsed list publishes nothing.
        let Some((r, _)) = self.open else { return };
        let mut list = Node::new(NodeId(1), Role::List).named(&self.header);
        list.bounds = bounds(r);
        out.push((LIST_KEY, list));
        for (i, label) in self.items.iter().enumerate() {
            let Some(row) = self.row(i) else { continue };
            let mut n = Node::new(NodeId(1), Role::ListItem).named(label);
            n.parent = Some(NodeId(u64::from(LIST_KEY)));
            n.bounds = bounds(row);
            n.states |= States::SELECTABLE | States::FOCUSABLE;
            n.states.set(States::SELECTED, self.selected == Some(i));
            n.actions = Actions::SELECT;
            n.position = Some((i as u32 + 1, self.items.len() as u32));
            out.push((i as u32, n));
        }
    }
    fn action(&mut self, cx: &mut EventCx<'_>, item: Option<u32>, a: &Action) -> bool {
        match (item, a) {
            (None, Action::Expand) if self.open.is_none() => self.open_list(cx),
            (None, Action::Collapse) if self.open.is_some() => self.close(cx),
            (Some(i), Action::Select) if (i as usize) < self.items.len() => {
                self.set(cx, i as usize);
                self.close(cx);
            }
            (None, Action::SetValue(v)) => match self.items.iter().position(|s| s == v) {
                Some(i) => self.set(cx, i),
                None => return false,
            },
            _ => return false,
        }
        true
    }
}
