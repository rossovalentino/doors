//! ListView and TreeView: stable item identity (keys), virtualized
//! realization (only rows in the viewport are painted and published),
//! selection that survives sorting, keyboard navigation and wheel scrolling.
use super::basic::row_plate;
use super::describe;
use super::menu::bounds;
use crate::display::Icon;
use crate::event::{Button as MouseButton, Event, Key, Phase};
use crate::layout::{Measured, Style};
use crate::theme::TextRole;
use crate::units::{LPoint, LRect, LSize, Lu};
use crate::widget::{EventCx, LayoutCx, Notify, PaintCx, SemanticsCx, Widget};
use alloc::{string::String, vec::Vec};
use door_accessibility::{Action, Actions, Node, NodeId, Role, States};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Selection {
    None,
    #[default]
    Single,
    /// Ctrl toggles, Shift extends; check boxes show the state.
    Multiple,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    pub key: u32,
    pub text: String,
    pub detail: String,
    pub icon: Option<Icon>,
}
impl ListItem {
    pub fn new(key: u32, text: &str) -> Self {
        Self {
            key,
            text: text.into(),
            detail: String::new(),
            icon: None,
        }
    }
    pub fn detail(mut self, d: &str) -> Self {
        self.detail = d.into();
        self
    }
    pub fn icon(mut self, i: Icon) -> Self {
        self.icon = Some(i);
        self
    }
}

/// Scroll position and viewport shared by both lists.
#[derive(Clone, Copy, Debug, Default)]
struct Scroll {
    offset: Lu,
}

pub struct ListView {
    pub name: String,
    pub items: Vec<ListItem>,
    pub mode: Selection,
    pub selected: Vec<u32>,
    pub row_height: i32,
    focus: Option<u32>,
    anchor: Option<u32>,
    scroll: Scroll,
    hover: Option<u32>,
    pressed: Option<u32>,
}
impl ListView {
    pub fn new(name: &str, items: Vec<ListItem>) -> Self {
        let focus = items.first().map(|i| i.key);
        Self {
            name: name.into(),
            items,
            mode: Selection::Single,
            selected: Vec::new(),
            row_height: 40,
            focus,
            anchor: focus,
            scroll: Scroll::default(),
            hover: None,
            pressed: None,
        }
    }
    pub fn mode(mut self, m: Selection) -> Self {
        self.mode = m;
        self
    }
    pub fn select(mut self, key: u32) -> Self {
        self.selected = alloc::vec![key];
        self.focus = Some(key);
        self
    }
    pub fn index_of(&self, key: u32) -> Option<usize> {
        self.items.iter().position(|i| i.key == key)
    }
    /// Sort by `f`; selection and focus follow item identity.
    pub fn sort_by(&mut self, f: impl FnMut(&ListItem, &ListItem) -> core::cmp::Ordering) {
        self.items.sort_by(f);
    }
    /// Replace items; selections of removed keys are dropped.
    pub fn set_items(&mut self, items: Vec<ListItem>) {
        self.items = items;
        self.selected
            .retain(|k| self.items.iter().any(|i| i.key == *k));
        if self.focus.is_none_or(|f| self.index_of(f).is_none()) {
            self.focus = self.items.first().map(|i| i.key);
        }
    }
    fn rh(&self) -> Lu {
        Lu::px(self.row_height)
    }
    fn row(&self, r: LRect, index: usize) -> LRect {
        LRect::new(
            r.x,
            r.y + self.rh() * index as i32 - self.scroll.offset,
            r.w,
            self.rh(),
        )
    }
    /// Indices of rows intersecting the viewport (the realized range).
    pub fn realized(&self, r: LRect) -> core::ops::Range<usize> {
        let rh = self.rh().0.max(1);
        let first = (self.scroll.offset.0 / rh) as usize;
        let last = ((self.scroll.offset + r.h).0 + rh - 1) / rh;
        first.min(self.items.len())..(last.max(0) as usize).min(self.items.len())
    }
    fn max_scroll(&self, r: LRect) -> Lu {
        (self.rh() * self.items.len() as i32 - r.h).max(Lu::ZERO)
    }
    fn index_at(&self, r: LRect, p: LPoint) -> Option<usize> {
        if !r.contains(p) {
            return None;
        }
        let i = ((p.y - r.y + self.scroll.offset).0 / self.rh().0.max(1)) as usize;
        (i < self.items.len()).then_some(i)
    }
    fn reveal(&mut self, r: LRect, index: usize) {
        let top = self.rh() * index as i32;
        if top < self.scroll.offset {
            self.scroll.offset = top;
        } else if top + self.rh() > self.scroll.offset + r.h {
            self.scroll.offset = top + self.rh() - r.h;
        }
        self.scroll.offset = self.scroll.offset.clamp(Lu::ZERO, self.max_scroll(r));
    }
    fn notify(&self, cx: &mut EventCx<'_>) {
        cx.notify(Notify::SelectionChanged(self.selected.first().copied()));
    }
    fn choose(&mut self, cx: &mut EventCx<'_>, index: usize, ctrl: bool, shift: bool) {
        let key = self.items[index].key;
        self.focus = Some(key);
        match self.mode {
            Selection::None => (),
            Selection::Single => {
                if self.selected != [key] {
                    self.selected = alloc::vec![key];
                    self.notify(cx);
                }
                self.anchor = Some(key);
            }
            Selection::Multiple if shift => {
                let a = self.anchor.and_then(|a| self.index_of(a)).unwrap_or(index);
                let (lo, hi) = (a.min(index), a.max(index));
                self.selected = self.items[lo..=hi].iter().map(|i| i.key).collect();
                self.notify(cx);
            }
            Selection::Multiple if ctrl => {
                self.anchor = Some(key);
            }
            Selection::Multiple => {
                if let Some(p) = self.selected.iter().position(|k| *k == key) {
                    self.selected.remove(p);
                } else {
                    self.selected.push(key);
                }
                self.anchor = Some(key);
                self.notify(cx);
            }
        }
        self.reveal(cx.rect, index);
        cx.invalidate();
    }
}
impl Widget for ListView {
    fn type_name(&self) -> &'static str {
        "ListView"
    }
    fn style(&self) -> Style {
        Style {
            clip: true,
            ..Style::default()
        }
    }
    fn measure(&self, _cx: &LayoutCx<'_>, a: LSize) -> Measured {
        let h = (self.rh() * self.items.len() as i32).min(self.rh() * 8);
        Measured::new(LSize::new(
            if a.w.is_infinite() { Lu::px(280) } else { a.w },
            h,
        ))
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let t = *cx.theme;
        let c = t.colors;
        let r = cx.rect;
        cx.push_clip(r);
        for i in self.realized(r) {
            let item = &self.items[i];
            let row = self.row(r, i).inflate(-Lu::px(2));
            let selected = self.selected.contains(&item.key);
            let hover = self.hover == Some(item.key) && cx.state.hovered;
            let pressed = self.pressed == Some(item.key) && cx.state.captured;
            cx.round(
                LRect::new(row.x + Lu::px(2), row.y, row.w - Lu::px(4), row.h),
                4,
                row_plate(&t, selected, hover, pressed),
            );
            let mut x = row.x + Lu::px(14);
            let enabled = cx.state.enabled;
            let fg = if !enabled {
                c.text_disabled
            } else if (selected || hover) && t.is_high_contrast() {
                c.control_text_hover
            } else {
                c.text
            };
            match self.mode {
                Selection::Single if selected => cx.round(
                    LRect::new(
                        row.x + Lu::px(2),
                        row.y + (row.h - Lu::px(16)) / 2,
                        Lu::px(3),
                        Lu::px(16),
                    ),
                    2,
                    if enabled { c.accent } else { c.accent_disabled },
                ),
                Selection::Multiple => {
                    let b = LRect::new(x, row.y + (row.h - Lu::px(20)) / 2, Lu::px(20), Lu::px(20));
                    if selected {
                        cx.round(b, 4, c.accent);
                        cx.icon(Icon::Check, b.inflate(-Lu::px(4)), c.text_on_accent);
                    } else {
                        cx.round(b, 4, c.strong);
                        cx.round(b.inflate(-Lu::ONE), 3, c.control_alt);
                    }
                    x += Lu::px(32);
                }
                _ => (),
            }
            if let Some(icon) = item.icon {
                cx.icon(
                    icon,
                    LRect::new(x, row.y + (row.h - Lu::px(16)) / 2, Lu::px(16), Lu::px(16)),
                    fg,
                );
                x += Lu::px(28);
            }
            if item.detail.is_empty() {
                cx.label_fit(
                    &item.text,
                    LRect::new(x, row.y, row.right() - x - Lu::px(8), row.h),
                    TextRole::Body,
                    fg,
                );
            } else {
                let w = cx.text.measure(&item.detail, TextRole::Caption).w;
                cx.label_fit(
                    &item.text,
                    LRect::new(x, row.y, row.right() - x - w - Lu::px(24), row.h),
                    TextRole::Body,
                    fg,
                );
                cx.label(
                    &item.detail,
                    LRect::new(row.right() - w - Lu::px(12), row.y, w, row.h),
                    TextRole::Caption,
                    c.text_secondary,
                );
            }
        }
        cx.pop_clip();
    }
    fn paint_over(&self, cx: &mut PaintCx<'_>) {
        let r = cx.rect;
        let total = self.rh() * self.items.len() as i32;
        if total > r.h {
            let track = r.h - Lu::px(8);
            let len = track.mul_div(r.h.0, total.0).max(Lu::px(24));
            let y = r.y
                + Lu::px(4)
                + (track - len).mul_div(self.scroll.offset.0, self.max_scroll(r).0.max(1));
            cx.round(
                LRect::new(r.right() - Lu::px(5), y, Lu::px(2), len),
                1,
                cx.theme.colors.scrollbar,
            );
        }
        if let Some(i) = self.focus.and_then(|f| self.index_of(f))
            && self.realized(r).contains(&i)
        {
            let row = self.row(r, i).inflate(-Lu::px(4));
            cx.push_clip(r);
            cx.focus_if_visible(row, 4);
            cx.pop_clip();
        }
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        if cx.phase != Phase::Target || self.items.is_empty() {
            return;
        }
        let r = cx.rect;
        match e {
            Event::PointerMove(p) => {
                let h = self.index_at(r, p.pos).map(|i| self.items[i].key);
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
                self.pressed = self.index_at(r, p.pos).map(|i| self.items[i].key);
                cx.capture_pointer();
                cx.invalidate();
                cx.set_handled();
            }
            Event::PointerUp(p) if p.button == MouseButton::Primary => {
                if let Some(k) = self.pressed.take()
                    && let Some(i) = self.index_at(r, p.pos).filter(|i| self.items[*i].key == k)
                {
                    self.choose(cx, i, p.modifiers.ctrl, p.modifiers.shift);
                }
                cx.invalidate();
            }
            Event::CaptureLost => {
                self.pressed = None;
                cx.invalidate();
            }
            Event::Wheel { delta, .. } => {
                let next = (self.scroll.offset - Lu::px(super::WHEEL_STEP * delta))
                    .clamp(Lu::ZERO, self.max_scroll(r));
                if next != self.scroll.offset {
                    self.scroll.offset = next;
                    cx.invalidate();
                    cx.set_handled();
                }
            }
            Event::KeyDown(k) => {
                let n = self.items.len();
                let cur = self.focus.and_then(|f| self.index_of(f)).unwrap_or(0);
                let page = (r.h.0 / self.rh().0.max(1)).max(1) as usize;
                let next = match k.key {
                    Key::Down => Some((cur + 1).min(n - 1)),
                    Key::Up => Some(cur.saturating_sub(1)),
                    Key::Home => Some(0),
                    Key::End => Some(n - 1),
                    Key::PageDown => Some((cur + page).min(n - 1)),
                    Key::PageUp => Some(cur.saturating_sub(page)),
                    Key::Space => {
                        self.choose(cx, cur, false, false);
                        cx.set_handled();
                        return;
                    }
                    Key::Enter => {
                        cx.notify(Notify::Invoked);
                        cx.set_handled();
                        return;
                    }
                    Key::Char('a') if k.modifiers.ctrl && self.mode == Selection::Multiple => {
                        self.selected = self.items.iter().map(|i| i.key).collect();
                        self.notify(cx);
                        cx.invalidate();
                        cx.set_handled();
                        return;
                    }
                    _ => None,
                };
                if let Some(i) = next {
                    if self.mode == Selection::Single && !k.modifiers.ctrl {
                        self.choose(cx, i, false, false);
                    } else if self.mode == Selection::Multiple && k.modifiers.shift {
                        self.choose(cx, i, false, true);
                    } else {
                        self.focus = Some(self.items[i].key);
                        self.reveal(r, i);
                        cx.invalidate();
                    }
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
        self.focus
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(node, Role::List, &self.name);
        if self.mode == Selection::Multiple {
            node.states |= States::MULTISELECTABLE;
        }
        node.states |= States::VERTICAL;
        true
    }
    fn virtual_semantics(&self, cx: &SemanticsCx<'_>, out: &mut Vec<(u32, Node)>) {
        // Only realized rows: a virtualized list never publishes its whole
        // data set, but every row carries its position in it.
        for i in self.realized(cx.rect) {
            let item = &self.items[i];
            let mut n = Node::new(NodeId(1), Role::ListItem).named(&item.text);
            n.description = item.detail.clone();
            n.bounds = bounds(self.row(cx.rect, i));
            n.position = Some((i as u32 + 1, self.items.len() as u32));
            n.states |= States::FOCUSABLE;
            if self.mode != Selection::None {
                n.states |= States::SELECTABLE;
                n.states
                    .set(States::SELECTED, self.selected.contains(&item.key));
                n.actions |= Actions::SELECT;
            }
            n.actions |= Actions::FOCUS | Actions::SCROLL_INTO_VIEW;
            out.push((item.key, n));
        }
    }
    fn action(&mut self, cx: &mut EventCx<'_>, item: Option<u32>, a: &Action) -> bool {
        let Some(i) = item.and_then(|k| self.index_of(k)) else {
            return false;
        };
        match a {
            Action::Select if self.mode != Selection::None => self.choose(cx, i, false, false),
            Action::Focus => {
                self.focus = Some(self.items[i].key);
                self.reveal(cx.rect, i);
                cx.request_focus();
                cx.invalidate();
            }
            Action::ScrollIntoView => {
                self.reveal(cx.rect, i);
                cx.invalidate();
            }
            _ => return false,
        }
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeItem {
    pub key: u32,
    pub parent: Option<u32>,
    pub text: String,
    pub expanded: bool,
}
impl TreeItem {
    pub fn new(key: u32, parent: Option<u32>, text: &str) -> Self {
        Self {
            key,
            parent,
            text: text.into(),
            expanded: false,
        }
    }
    pub fn expanded(mut self) -> Self {
        self.expanded = true;
        self
    }
}
/// TreeView: Right expands or enters, Left collapses or goes to the parent.
/// A collapsed branch publishes none of its descendants.
pub struct TreeView {
    pub name: String,
    pub items: Vec<TreeItem>,
    pub selected: Option<u32>,
    focus: Option<u32>,
    scroll: Scroll,
    hover: Option<u32>,
}
impl TreeView {
    pub fn new(name: &str, items: Vec<TreeItem>) -> Self {
        let focus = items.iter().find(|i| i.parent.is_none()).map(|i| i.key);
        Self {
            name: name.into(),
            items,
            selected: None,
            focus,
            scroll: Scroll::default(),
            hover: None,
        }
    }
    fn item(&self, key: u32) -> Option<&TreeItem> {
        self.items.iter().find(|i| i.key == key)
    }
    fn has_children(&self, key: u32) -> bool {
        self.items.iter().any(|i| i.parent == Some(key))
    }
    /// Visible rows in order: (key, level from 1).
    pub fn visible(&self) -> Vec<(u32, u16)> {
        let mut out = Vec::new();
        fn walk(t: &TreeView, parent: Option<u32>, level: u16, out: &mut Vec<(u32, u16)>) {
            if level > 32 {
                return;
            }
            for item in t.items.iter().filter(|i| i.parent == parent) {
                out.push((item.key, level));
                if item.expanded {
                    walk(t, Some(item.key), level + 1, out);
                }
            }
        }
        walk(self, None, 1, &mut out);
        out
    }
    fn set_expanded(&mut self, cx: &mut EventCx<'_>, key: u32, on: bool) -> bool {
        if !self.has_children(key) {
            return false;
        }
        let Some(item) = self.items.iter_mut().find(|i| i.key == key) else {
            return false;
        };
        if item.expanded == on {
            return false;
        }
        item.expanded = on;
        cx.notify(Notify::Expanded(on));
        cx.invalidate();
        true
    }
    fn select(&mut self, cx: &mut EventCx<'_>, key: u32) {
        self.focus = Some(key);
        if self.selected != Some(key) {
            self.selected = Some(key);
            cx.notify(Notify::SelectionChanged(Some(key)));
        }
        let rows = self.visible();
        if let Some(i) = rows.iter().position(|(k, _)| *k == key) {
            let top = Lu::px(32) * i as i32;
            if top < self.scroll.offset {
                self.scroll.offset = top;
            } else if top + Lu::px(32) > self.scroll.offset + cx.rect.h {
                self.scroll.offset = top + Lu::px(32) - cx.rect.h;
            }
        }
        cx.invalidate();
    }
    fn row(&self, r: LRect, i: usize) -> LRect {
        LRect::new(
            r.x,
            r.y + Lu::px(32) * i as i32 - self.scroll.offset,
            r.w,
            Lu::px(32),
        )
    }
}
impl Widget for TreeView {
    fn type_name(&self) -> &'static str {
        "TreeView"
    }
    fn style(&self) -> Style {
        Style {
            clip: true,
            ..Style::default()
        }
    }
    fn measure(&self, _cx: &LayoutCx<'_>, a: LSize) -> Measured {
        let n = self.visible().len().clamp(1, 10) as i32;
        Measured::new(LSize::new(
            if a.w.is_infinite() { Lu::px(280) } else { a.w },
            Lu::px(32) * n,
        ))
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let t = *cx.theme;
        let c = t.colors;
        let r = cx.rect;
        cx.push_clip(r);
        for (i, (key, level)) in self.visible().into_iter().enumerate() {
            let row = self.row(r, i);
            if row.bottom() < r.y || row.y > r.bottom() {
                continue;
            }
            let item = self.item(key).unwrap();
            let selected = self.selected == Some(key);
            let hover = self.hover == Some(key) && cx.state.hovered;
            let plate = LRect::new(
                row.x + Lu::px(2),
                row.y + Lu::px(2),
                row.w - Lu::px(4),
                row.h - Lu::px(4),
            );
            cx.round(plate, 4, row_plate(&t, selected, hover, false));
            if selected {
                cx.round(
                    LRect::new(
                        plate.x,
                        plate.y + (plate.h - Lu::px(16)) / 2,
                        Lu::px(3),
                        Lu::px(16),
                    ),
                    2,
                    c.accent,
                );
            }
            let indent = Lu::px(16) * (i32::from(level) - 1);
            let fg = if !cx.state.enabled {
                c.text_disabled
            } else if (selected || hover) && t.is_high_contrast() {
                c.control_text_hover
            } else {
                c.text
            };
            if self.has_children(key) {
                let g = LRect::new(
                    row.x + Lu::px(12) + indent,
                    row.y + Lu::px(10),
                    Lu::px(12),
                    Lu::px(12),
                );
                cx.icon(
                    if item.expanded {
                        Icon::ChevronDown
                    } else {
                        Icon::ChevronRight
                    },
                    g,
                    c.text_secondary,
                );
            }
            cx.label_fit(
                &item.text,
                LRect::new(
                    row.x + Lu::px(32) + indent,
                    row.y,
                    row.w - Lu::px(40) - indent,
                    row.h,
                ),
                TextRole::Body,
                fg,
            );
        }
        cx.pop_clip();
    }
    fn paint_over(&self, cx: &mut PaintCx<'_>) {
        let rows = self.visible();
        if let Some(i) = self
            .focus
            .and_then(|f| rows.iter().position(|(k, _)| *k == f))
        {
            let row = self.row(cx.rect, i).inflate(-Lu::px(4));
            let clip = cx.rect;
            cx.push_clip(clip);
            cx.focus_if_visible(row, 4);
            cx.pop_clip();
        }
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        if cx.phase != Phase::Target {
            return;
        }
        let rows = self.visible();
        if rows.is_empty() {
            return;
        }
        let r = cx.rect;
        let at = |p: LPoint, s: &TreeView| -> Option<usize> {
            r.contains(p)
                .then(|| ((p.y - r.y + s.scroll.offset).0 / Lu::px(32).0) as usize)
                .filter(|i| *i < rows.len())
        };
        match e {
            Event::PointerMove(p) => {
                let h = at(p.pos, self).map(|i| rows[i].0);
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
                if let Some(i) = at(p.pos, self) {
                    let (key, level) = rows[i];
                    let glyph = r.x + Lu::px(8) + Lu::px(16) * (i32::from(level) - 1);
                    if p.pos.x >= glyph && p.pos.x < glyph + Lu::px(24) && self.has_children(key) {
                        let on = !self.item(key).unwrap().expanded;
                        self.set_expanded(cx, key, on);
                        self.focus = Some(key);
                    } else {
                        self.select(cx, key);
                    }
                }
                cx.set_handled();
            }
            Event::Wheel { delta, .. } => {
                let max = (Lu::px(32) * rows.len() as i32 - r.h).max(Lu::ZERO);
                let next =
                    (self.scroll.offset - Lu::px(super::WHEEL_STEP * delta)).clamp(Lu::ZERO, max);
                if next != self.scroll.offset {
                    self.scroll.offset = next;
                    cx.invalidate();
                    cx.set_handled();
                }
            }
            Event::KeyDown(k) => {
                let cur = self
                    .focus
                    .and_then(|f| rows.iter().position(|(key, _)| *key == f))
                    .unwrap_or(0);
                let key = rows[cur].0;
                match k.key {
                    Key::Down => self.select(cx, rows[(cur + 1).min(rows.len() - 1)].0),
                    Key::Up => self.select(cx, rows[cur.saturating_sub(1)].0),
                    Key::Home => self.select(cx, rows[0].0),
                    Key::End => self.select(cx, rows[rows.len() - 1].0),
                    Key::Right => {
                        if !self.set_expanded(cx, key, true)
                            && let Some((child, _)) = rows.get(cur + 1).filter(|(c, _)| {
                                self.item(*c).is_some_and(|i| i.parent == Some(key))
                            })
                        {
                            self.select(cx, *child);
                        }
                    }
                    Key::Left => {
                        if !self.set_expanded(cx, key, false)
                            && let Some(p) = self.item(key).and_then(|i| i.parent)
                        {
                            self.select(cx, p);
                        }
                    }
                    Key::Enter | Key::Space => {
                        let on = self.item(key).is_some_and(|i| !i.expanded);
                        if !self.set_expanded(cx, key, on) {
                            self.select(cx, key);
                        }
                    }
                    _ => return,
                }
                cx.set_handled();
            }
            _ => (),
        }
    }
    fn focusable(&self) -> bool {
        !self.items.is_empty()
    }
    fn focused_item(&self) -> Option<u32> {
        self.focus
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(node, Role::Tree, &self.name);
        true
    }
    fn virtual_semantics(&self, cx: &SemanticsCx<'_>, out: &mut Vec<(u32, Node)>) {
        for (i, (key, level)) in self.visible().into_iter().enumerate() {
            let item = self.item(key).unwrap();
            let siblings: Vec<u32> = self
                .items
                .iter()
                .filter(|s| s.parent == item.parent)
                .map(|s| s.key)
                .collect();
            let mut n = Node::new(NodeId(1), Role::TreeItem).named(&item.text);
            n.parent = item.parent.map(|p| NodeId(u64::from(p)));
            let row = self.row(cx.rect, i);
            n.bounds = bounds(row);
            if !row.intersects(cx.rect) {
                n.states |= States::OFFSCREEN;
            }
            n.level = level;
            n.position = siblings
                .iter()
                .position(|k| *k == key)
                .map(|p| (p as u32 + 1, siblings.len() as u32));
            n.states |= States::FOCUSABLE | States::SELECTABLE;
            n.states.set(States::SELECTED, self.selected == Some(key));
            n.actions = Actions::SELECT | Actions::FOCUS;
            if self.has_children(key) {
                n.states |= States::EXPANDABLE;
                n.states.set(States::EXPANDED, item.expanded);
                n.actions |= if item.expanded {
                    Actions::COLLAPSE
                } else {
                    Actions::EXPAND
                };
            }
            out.push((key, n));
        }
    }
    fn action(&mut self, cx: &mut EventCx<'_>, item: Option<u32>, a: &Action) -> bool {
        let Some(key) = item.filter(|k| self.item(*k).is_some()) else {
            return false;
        };
        match a {
            Action::Expand => self.set_expanded(cx, key, true),
            Action::Collapse => self.set_expanded(cx, key, false),
            Action::Select => {
                self.select(cx, key);
                true
            }
            Action::Focus => {
                self.focus = Some(key);
                cx.request_focus();
                cx.invalidate();
                true
            }
            _ => false,
        }
    }
}
