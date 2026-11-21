//! TabView and NavigationView. Child `i` of a TabView is tab `i`'s
//! content; a NavigationView's first child is its content frame.
use super::basic::row_plate;
use super::describe;
use super::menu::bounds;
use crate::display::Icon;
use crate::event::{Button as MouseButton, Event, Key, Phase};
use crate::layout::{Axis, Style};
use crate::theme::TextRole;
use crate::units::{Insets, LPoint, LRect, Lu};
use crate::widget::{EventCx, LayoutCx, Notify, PaintCx, SemanticsCx, Widget};
use alloc::{string::String, vec::Vec};
use door_accessibility::{Action, Actions, Node, NodeId, Role, States};

const STRIP: i32 = 40;
/// `Notify::Command` raised by a TabView's add button.
pub const TAB_ADD: u32 = 0xfffe_0001;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub key: u32,
    pub title: String,
    pub icon: Option<Icon>,
    pub closable: bool,
}
impl Tab {
    pub fn new(key: u32, title: &str) -> Self {
        Self {
            key,
            title: title.into(),
            icon: Some(Icon::Document),
            closable: true,
        }
    }
}
pub struct TabView {
    pub tabs: Vec<Tab>,
    pub selected: usize,
    pub add_button: bool,
    hover: Option<usize>,
    pressed: Option<(usize, bool)>,
}
impl TabView {
    pub fn new(tabs: Vec<Tab>) -> Self {
        Self {
            tabs,
            selected: 0,
            add_button: true,
            hover: None,
            pressed: None,
        }
    }
    /// Remove a tab's header; the caller removes the matching content child.
    pub fn remove_tab(&mut self, key: u32) -> Option<usize> {
        let i = self.tabs.iter().position(|t| t.key == key)?;
        self.tabs.remove(i);
        if (self.selected >= self.tabs.len() || i < self.selected) && self.selected > 0 {
            self.selected -= 1;
        }
        Some(i)
    }
    fn tab(&self, cx_text: &dyn crate::text::TextSystem, r: LRect, i: usize) -> LRect {
        let width = |t: &Tab| {
            (cx_text.measure(&t.title, TextRole::Body).w + Lu::px(if t.closable { 76 } else { 48 }))
                .clamp(Lu::px(100), Lu::px(240))
        };
        let mut x = r.x + Lu::px(8);
        for t in &self.tabs[..i.min(self.tabs.len())] {
            x += width(t);
        }
        let w = if i < self.tabs.len() {
            width(&self.tabs[i])
        } else {
            Lu::px(40)
        };
        LRect::new(x, r.y + Lu::px(8), w, Lu::px(STRIP - 8))
    }
    fn close_rect(tab: LRect) -> LRect {
        LRect::new(
            tab.right() - Lu::px(32),
            tab.y + Lu::px(4),
            Lu::px(24),
            Lu::px(24),
        )
    }
    fn select(&mut self, cx: &mut EventCx<'_>, i: usize) {
        if i < self.tabs.len() && i != self.selected {
            self.selected = i;
            cx.relayout();
            cx.notify(Notify::SelectionChanged(Some(self.tabs[i].key)));
        }
        cx.invalidate();
    }
    fn hit(&self, cx: &EventCx<'_>, p: LPoint) -> Option<(usize, bool)> {
        let n = self.tabs.len() + usize::from(self.add_button);
        (0..n)
            .find(|i| self.tab(cx.text, cx.rect, *i).contains(p))
            .map(|i| {
                let r = self.tab(cx.text, cx.rect, i);
                (
                    i,
                    i < self.tabs.len() && self.tabs[i].closable && Self::close_rect(r).contains(p),
                )
            })
    }
}
impl Widget for TabView {
    fn type_name(&self) -> &'static str {
        "TabView"
    }
    fn style(&self) -> Style {
        Style::stack(Axis::Vertical, 0).grow(1)
    }
    fn padding(&self, _cx: &LayoutCx<'_>) -> Option<Insets> {
        Some(Insets::ltrb(16, STRIP + 16, 16, 16))
    }
    fn child_visible(&self, index: usize) -> bool {
        index == self.selected
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let t = *cx.theme;
        let c = t.colors;
        let r = cx.rect;
        cx.fill(LRect::new(r.x, r.y, r.w, Lu::px(STRIP)), c.background);
        let body = LRect::new(r.x, r.y + Lu::px(STRIP), r.w, r.h - Lu::px(STRIP));
        cx.fill(body, c.card);
        cx.fill(LRect::new(r.x, body.y, r.w, Lu::ONE), c.card_stroke);
        for i in 0..self.tabs.len() + usize::from(self.add_button) {
            let tr = self.tab(cx.text, r, i);
            let Some(tab) = self.tabs.get(i) else {
                if self.hover == Some(i) && cx.state.hovered {
                    cx.round(tr.inflate(-Lu::px(4)), 4, c.subtle_hover);
                }
                cx.icon_centered(Icon::Add, tr, 12, c.text);
                continue;
            };
            let selected = i == self.selected;
            if selected {
                cx.round(
                    LRect::new(tr.x, tr.y, tr.w, tr.h + Lu::px(8)),
                    8,
                    c.card_stroke,
                );
                cx.round(
                    LRect::new(
                        tr.x + Lu::ONE,
                        tr.y + Lu::ONE,
                        tr.w - Lu::px(2),
                        tr.h + Lu::px(8),
                    ),
                    7,
                    c.card,
                );
            } else if self.hover == Some(i) && cx.state.hovered {
                cx.round(
                    LRect::new(
                        tr.x + Lu::px(2),
                        tr.y + Lu::px(2),
                        tr.w - Lu::px(4),
                        tr.h - Lu::px(6),
                    ),
                    6,
                    c.subtle_hover,
                );
            }
            let fg = if !cx.state.enabled {
                c.text_disabled
            } else if selected {
                c.text
            } else {
                c.text_secondary
            };
            let mut x = tr.x + Lu::px(12);
            if let Some(icon) = tab.icon {
                cx.icon(
                    icon,
                    LRect::new(x, tr.y + (tr.h - Lu::px(16)) / 2, Lu::px(16), Lu::px(16)),
                    fg,
                );
                x += Lu::px(24);
            }
            let room = tr.right() - x - if tab.closable { Lu::px(36) } else { Lu::px(8) };
            cx.label_fit(
                &tab.title,
                LRect::new(x, tr.y, room, tr.h),
                TextRole::Body,
                fg,
            );
            if tab.closable {
                let cr = Self::close_rect(tr);
                if self.pressed == Some((i, true)) && cx.state.captured {
                    cx.round(cr, 4, c.subtle_pressed);
                }
                cx.icon_centered(Icon::Close, cr, 10, fg);
            }
        }
        cx.fill(
            LRect::new(r.x, r.y + Lu::px(STRIP) - Lu::ONE, r.w, Lu::ONE),
            c.card_stroke,
        );
    }
    fn paint_over(&self, cx: &mut PaintCx<'_>) {
        if !self.tabs.is_empty() {
            let tr = self.tab(cx.text, cx.rect, self.selected);
            cx.focus_if_visible(tr, 8);
        }
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        // Ctrl+Tab / Ctrl+Shift+Tab / Ctrl+W work from inside the content.
        if let Event::KeyDown(k) = e
            && (cx.phase == Phase::Bubble || cx.phase == Phase::Target)
        {
            let n = self.tabs.len();
            if n > 0 && k.modifiers.ctrl && k.key == Key::Tab {
                let next = if k.modifiers.shift {
                    (self.selected + n - 1) % n
                } else {
                    (self.selected + 1) % n
                };
                self.select(cx, next);
                cx.set_handled();
                return;
            }
            if n > 0
                && k.modifiers.ctrl
                && (k.key == Key::Char('w') || k.key == Key::F(4))
                && self.tabs[self.selected].closable
            {
                cx.notify(Notify::TabClosed(self.tabs[self.selected].key));
                cx.set_handled();
                return;
            }
        }
        if cx.phase != Phase::Target {
            return;
        }
        let n = self.tabs.len();
        match e {
            Event::PointerMove(p) => {
                let h = self.hit(cx, p.pos).map(|(i, _)| i);
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
                self.pressed = self.hit(cx, p.pos);
                if let Some((i, false)) = self.pressed
                    && i < n
                {
                    self.select(cx, i);
                }
                if self.pressed.is_some() {
                    cx.capture_pointer();
                    cx.set_handled();
                }
                cx.invalidate();
            }
            Event::PointerUp(p) => {
                let pressed = self.pressed.take();
                if pressed.is_some() && pressed == self.hit(cx, p.pos) {
                    match pressed {
                        Some((i, true)) => cx.notify(Notify::TabClosed(self.tabs[i].key)),
                        Some((i, false)) if i == n => cx.raise(TAB_ADD),
                        _ => (),
                    }
                }
                cx.invalidate();
            }
            Event::CaptureLost => {
                self.pressed = None;
                cx.invalidate();
            }
            Event::KeyDown(k) if n > 0 => {
                match k.key {
                    Key::Left => self.select(cx, (self.selected + n - 1) % n),
                    Key::Right => self.select(cx, (self.selected + 1) % n),
                    Key::Home => self.select(cx, 0),
                    Key::End => self.select(cx, n - 1),
                    Key::Delete if self.tabs[self.selected].closable => {
                        cx.notify(Notify::TabClosed(self.tabs[self.selected].key))
                    }
                    _ => return,
                }
                cx.set_handled();
            }
            _ => (),
        }
    }
    fn focusable(&self) -> bool {
        !self.tabs.is_empty()
    }
    fn focused_item(&self) -> Option<u32> {
        self.tabs.get(self.selected).map(|t| t.key)
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(node, Role::TabList, "Tabs");
        true
    }
    fn virtual_semantics(&self, cx: &SemanticsCx<'_>, out: &mut Vec<(u32, Node)>) {
        for (i, tab) in self.tabs.iter().enumerate() {
            let mut n = Node::new(NodeId(1), Role::Tab).named(&tab.title);
            n.bounds = bounds(self.tab(cx.text, cx.rect, i));
            n.states |= States::SELECTABLE | States::FOCUSABLE;
            n.states.set(States::SELECTED, i == self.selected);
            n.actions = Actions::SELECT
                | Actions::FOCUS
                | if tab.closable {
                    Actions::DISMISS
                } else {
                    Actions::NONE
                };
            n.position = Some((i as u32 + 1, self.tabs.len() as u32));
            out.push((tab.key, n));
        }
    }
    fn action(&mut self, cx: &mut EventCx<'_>, item: Option<u32>, a: &Action) -> bool {
        let Some(i) = item.and_then(|k| self.tabs.iter().position(|t| t.key == k)) else {
            return false;
        };
        match a {
            Action::Select | Action::Focus => {
                self.select(cx, i);
                if *a == Action::Focus {
                    cx.request_focus();
                }
            }
            Action::Dismiss if self.tabs[i].closable => {
                cx.notify(Notify::TabClosed(self.tabs[i].key))
            }
            _ => return false,
        }
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavItem {
    pub key: u32,
    pub label: String,
    pub icon: Icon,
}
impl NavItem {
    pub fn new(key: u32, label: &str, icon: Icon) -> Self {
        Self {
            key,
            label: label.into(),
            icon,
        }
    }
}
/// NavigationView (Left mode): a pane of items with a toggle button and a
/// pinned footer; one tab stop for the item list.
pub struct NavigationView {
    pub items: Vec<NavItem>,
    pub footer: Vec<NavItem>,
    pub selected: Option<u32>,
    pub pane_open: bool,
    focus: usize,
    hover: Option<usize>,
    pressed: Option<usize>,
}
const TOGGLE: usize = usize::MAX;
const TOGGLE_KEY: u32 = 0xffff_fff0;
impl NavigationView {
    pub fn new(items: Vec<NavItem>, footer: Vec<NavItem>) -> Self {
        let selected = items.first().map(|i| i.key);
        Self {
            items,
            footer,
            selected,
            pane_open: true,
            focus: 0,
            hover: None,
            pressed: None,
        }
    }
    fn pane_w(&self) -> Lu {
        Lu::px(if self.pane_open { 280 } else { 48 })
    }
    fn all(&self) -> impl Iterator<Item = &NavItem> {
        self.items.iter().chain(self.footer.iter())
    }
    fn count(&self) -> usize {
        self.items.len() + self.footer.len()
    }
    fn row(&self, r: LRect, i: usize) -> LRect {
        if i == TOGGLE {
            return LRect::new(r.x + Lu::px(4), r.y + Lu::px(4), Lu::px(40), Lu::px(36));
        }
        let y = if i < self.items.len() {
            r.y + Lu::px(48) + Lu::px(40) * i as i32
        } else {
            r.bottom() - Lu::px(44) - Lu::px(40) * (self.count() - 1 - i) as i32
        };
        LRect::new(r.x + Lu::px(4), y, self.pane_w() - Lu::px(8), Lu::px(36))
    }
    fn index_at(&self, r: LRect, p: LPoint) -> Option<usize> {
        if self.row(r, TOGGLE).contains(p) {
            return Some(TOGGLE);
        }
        (0..self.count()).find(|i| self.row(r, *i).contains(p))
    }
    fn activate(&mut self, cx: &mut EventCx<'_>, i: usize) {
        if i == TOGGLE {
            self.pane_open = !self.pane_open;
            cx.relayout();
            return;
        }
        let key = self.all().nth(i).map(|n| n.key);
        self.focus = i;
        if let Some(k) = key.filter(|k| Some(*k) != self.selected) {
            self.selected = Some(k);
            cx.notify(Notify::Navigated(k));
        }
        cx.invalidate();
    }
}
impl Widget for NavigationView {
    fn type_name(&self) -> &'static str {
        "NavigationView"
    }
    fn style(&self) -> Style {
        Style::stack(Axis::Vertical, 0).grow(1)
    }
    fn padding(&self, _cx: &LayoutCx<'_>) -> Option<Insets> {
        Some(Insets {
            left: self.pane_w() + Lu::px(24),
            top: Lu::px(24),
            right: Lu::px(24),
            bottom: Lu::px(24),
        })
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let t = *cx.theme;
        let c = t.colors;
        let r = cx.rect;
        let pane = LRect::new(r.x, r.y, self.pane_w(), r.h);
        cx.fill(r, c.background);
        let content = LRect::new(pane.right(), r.y, r.w - pane.w, r.h);
        cx.round(content, 8, c.card_stroke);
        cx.round(content.inflate(-Lu::ONE), 7, c.card);
        let toggle = self.row(r, TOGGLE);
        if self.hover == Some(TOGGLE) && cx.state.hovered {
            cx.round(toggle, 4, c.subtle_hover);
        }
        cx.icon_centered(Icon::Menu, toggle, 16, c.text);
        for (i, item) in self.all().enumerate() {
            let row = self.row(r, i);
            let selected = self.selected == Some(item.key);
            let hover = self.hover == Some(i) && cx.state.hovered;
            let pressed = self.pressed == Some(i) && cx.state.captured;
            cx.round(row, 4, row_plate(&t, selected, hover, pressed));
            if selected {
                cx.round(
                    LRect::new(row.x, row.y + Lu::px(10), Lu::px(3), Lu::px(16)),
                    2,
                    c.accent,
                );
            }
            let fg = if !cx.state.enabled {
                c.text_disabled
            } else if (selected || hover) && t.is_high_contrast() {
                c.control_text_hover
            } else {
                c.text
            };
            cx.icon(
                item.icon,
                LRect::new(
                    row.x + Lu::px(12),
                    row.y + Lu::px(10),
                    Lu::px(16),
                    Lu::px(16),
                ),
                fg,
            );
            if self.pane_open {
                cx.label_fit(
                    &item.label,
                    LRect::new(row.x + Lu::px(44), row.y, row.w - Lu::px(52), row.h),
                    TextRole::Body,
                    fg,
                );
            }
        }
    }
    fn paint_over(&self, cx: &mut PaintCx<'_>) {
        if self.count() > 0 {
            let row = self.row(cx.rect, self.focus.min(self.count() - 1));
            cx.focus_if_visible(row, 4);
        }
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        if cx.phase != Phase::Target {
            return;
        }
        let r = cx.rect;
        let n = self.count();
        match e {
            Event::PointerMove(p) => {
                let h = self.index_at(r, p.pos);
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
                self.pressed = self.index_at(r, p.pos);
                if self.pressed.is_some() {
                    cx.capture_pointer();
                    cx.set_handled();
                }
                cx.invalidate();
            }
            Event::PointerUp(p) => {
                if let Some(i) = self.pressed.take()
                    && self.index_at(r, p.pos) == Some(i)
                {
                    self.activate(cx, i);
                }
                cx.invalidate();
            }
            Event::CaptureLost => {
                self.pressed = None;
                cx.invalidate();
            }
            Event::KeyDown(k) if n > 0 => {
                match k.key {
                    Key::Down => self.focus = (self.focus + 1).min(n - 1),
                    Key::Up => self.focus = self.focus.saturating_sub(1),
                    Key::Home => self.focus = 0,
                    Key::End => self.focus = n - 1,
                    Key::Enter | Key::Space => {
                        let f = self.focus;
                        self.activate(cx, f);
                    }
                    _ => return,
                }
                cx.invalidate();
                cx.set_handled();
            }
            _ => (),
        }
    }
    fn focusable(&self) -> bool {
        self.count() > 0
    }
    fn focused_item(&self) -> Option<u32> {
        self.all().nth(self.focus).map(|i| i.key)
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(node, Role::Navigation, "Navigation");
        true
    }
    fn virtual_semantics(&self, cx: &SemanticsCx<'_>, out: &mut Vec<(u32, Node)>) {
        let mut toggle = Node::new(NodeId(1), Role::Button).named(if self.pane_open {
            "Close Navigation"
        } else {
            "Open Navigation"
        });
        toggle.bounds = bounds(self.row(cx.rect, TOGGLE));
        toggle.actions = Actions::INVOKE;
        out.push((TOGGLE_KEY, toggle));
        let n = self.count() as u32;
        for (i, item) in self.all().enumerate() {
            let mut node = Node::new(NodeId(1), Role::ListItem).named(&item.label);
            node.bounds = bounds(self.row(cx.rect, i));
            node.states |= States::SELECTABLE | States::FOCUSABLE;
            node.states
                .set(States::SELECTED, self.selected == Some(item.key));
            node.actions = Actions::SELECT | Actions::FOCUS;
            node.position = Some((i as u32 + 1, n));
            out.push((item.key, node));
        }
    }
    fn action(&mut self, cx: &mut EventCx<'_>, item: Option<u32>, a: &Action) -> bool {
        match (item, a) {
            (Some(TOGGLE_KEY), Action::Invoke) => {
                self.activate(cx, TOGGLE);
                true
            }
            (Some(k), Action::Select | Action::Focus) => {
                let Some(i) = self.all().position(|n| n.key == k) else {
                    return false;
                };
                if *a == Action::Select {
                    self.activate(cx, i);
                } else {
                    self.focus = i;
                    cx.request_focus();
                    cx.invalidate();
                }
                true
            }
            _ => false,
        }
    }
}
