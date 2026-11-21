//! MenuFlyout (also the context menu), MenuBar and CommandBar. Open menus
//! are owner popups: they take pointer and keyboard input first, dismiss on
//! a click outside, Escape or window deactivation, and leave focus where it
//! was. Invoking an item raises its routed command (`Notify::Command(id)`
//! when no ancestor handles it).
use super::describe;
use crate::display::Icon;
use crate::event::{Button as MouseButton, Event, Key, KeyEvent, Phase};
use crate::layout::{Measured, Style};
use crate::text::TextSystem;
use crate::theme::{TextRole, Theme};
use crate::units::{LPoint, LRect, LSize, Lu};
use crate::widget::{EventCx, LayoutCx, Notify, PaintCx, SemanticsCx, Widget};
use alloc::{string::String, vec::Vec};
use door_accessibility::{Action, Actions, Bounds, Node, NodeId, Role, States};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub id: u32,
    pub label: String,
    pub accelerator: String,
    pub icon: Option<Icon>,
    pub enabled: bool,
    /// Toggle item state (ToggleMenuFlyoutItem).
    pub check: Option<bool>,
    pub separator: bool,
}
impl MenuItem {
    pub fn new(id: u32, label: &str) -> Self {
        Self {
            id,
            label: label.into(),
            accelerator: String::new(),
            icon: None,
            enabled: true,
            check: None,
            separator: false,
        }
    }
    pub fn separator() -> Self {
        Self {
            separator: true,
            enabled: false,
            ..Self::new(0, "")
        }
    }
    pub fn accelerator(mut self, a: &str) -> Self {
        self.accelerator = a.into();
        self
    }
    pub fn icon(mut self, i: Icon) -> Self {
        self.icon = Some(i);
        self
    }
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
    pub fn checked(mut self, on: bool) -> Self {
        self.check = Some(on);
        self
    }
}

const ROW: i32 = 32;
const SEPARATOR: i32 = 9;
/// Key base of a popup's menu node among a widget's virtual items.
const MENU_KEY: u32 = 0xffff_0000;

/// One open menu (window coordinates).
#[derive(Clone, Debug)]
pub struct Popup {
    pub items: Vec<MenuItem>,
    pub highlight: Option<usize>,
    pub rect: LRect,
    pressed: Option<usize>,
}
impl Popup {
    fn row_h(item: &MenuItem) -> Lu {
        Lu::px(if item.separator { SEPARATOR } else { ROW })
    }
    fn gutter(items: &[MenuItem]) -> Lu {
        Lu::px(
            if items.iter().any(|i| i.icon.is_some() || i.check.is_some()) {
                40
            } else {
                12
            },
        )
    }
    pub fn size(items: &[MenuItem], t: &dyn TextSystem) -> LSize {
        let label = items
            .iter()
            .map(|i| t.measure(&i.label, TextRole::Body).w)
            .fold(Lu::ZERO, Lu::max);
        let accel = items
            .iter()
            .map(|i| t.measure(&i.accelerator, TextRole::Caption).w)
            .fold(Lu::ZERO, Lu::max);
        let w = Self::gutter(items)
            + label
            + if accel > Lu::ZERO {
                Lu::px(24) + accel
            } else {
                Lu::ZERO
            }
            + Lu::px(24);
        let h = items.iter().map(Self::row_h).fold(Lu::ZERO, |a, b| a + b) + Lu::px(8);
        LSize::new(w.max(Lu::px(120)), h)
    }
    /// Open below `anchor` (left-aligned), flipping above and clamping to
    /// the window.
    pub fn open(
        items: Vec<MenuItem>,
        anchor: LRect,
        window: LSize,
        t: &dyn TextSystem,
        keyboard: bool,
    ) -> Self {
        let s = Self::size(&items, t);
        let mut y = anchor.bottom() + Lu::px(4);
        if y + s.h > window.h && anchor.y - s.h - Lu::px(4) >= Lu::ZERO {
            y = anchor.y - s.h - Lu::px(4);
        }
        let x = anchor.x.clamp(Lu::ZERO, (window.w - s.w).max(Lu::ZERO));
        let y = y.clamp(Lu::ZERO, (window.h - s.h).max(Lu::ZERO));
        let first = items.iter().position(|i| i.enabled && !i.separator);
        Self {
            items,
            highlight: if keyboard { first } else { None },
            rect: LRect::new(x, y, s.w, s.h),
            pressed: None,
        }
    }
    pub fn row(&self, i: usize) -> LRect {
        let mut y = self.rect.y + Lu::px(4);
        for item in &self.items[..i] {
            y += Self::row_h(item);
        }
        LRect::new(
            self.rect.x + Lu::px(4),
            y,
            self.rect.w - Lu::px(8),
            Self::row_h(&self.items[i]),
        )
    }
    pub fn index_at(&self, p: LPoint) -> Option<usize> {
        (0..self.items.len())
            .find(|i| self.row(*i).contains(p))
            .filter(|i| self.items[*i].enabled && !self.items[*i].separator)
    }
    fn step(&mut self, forward: bool) {
        let n = self.items.len();
        if n == 0 {
            return;
        }
        let mut i = self.highlight.unwrap_or(if forward { n - 1 } else { 0 });
        for _ in 0..n {
            i = if forward {
                (i + 1) % n
            } else {
                (i + n - 1) % n
            };
            if self.items[i].enabled && !self.items[i].separator {
                self.highlight = Some(i);
                return;
            }
        }
    }
    /// Keyboard inside the menu. Returns Some(item) to invoke, or
    /// `Err(())` to close.
    #[allow(clippy::result_unit_err)] // Err only means "close the popup".
    pub fn key(&mut self, k: &KeyEvent) -> Result<Option<usize>, ()> {
        match k.key {
            Key::Down => self.step(true),
            Key::Up => self.step(false),
            Key::Home => {
                self.highlight = None;
                self.step(true);
            }
            Key::End => {
                self.highlight = None;
                self.step(false);
            }
            Key::Enter | Key::Space => return Ok(self.highlight),
            Key::Escape => return Err(()),
            Key::Char(c) => {
                // Access by first letter.
                if let Some(i) = self.items.iter().position(|i| {
                    i.enabled
                        && !i.separator
                        && i.label
                            .chars()
                            .next()
                            .is_some_and(|f| f.to_ascii_lowercase() == c)
                }) {
                    return Ok(Some(i));
                }
            }
            _ => (),
        }
        Ok(None)
    }
    pub fn paint(&self, cx: &mut PaintCx<'_>) {
        let t: Theme = *cx.theme;
        let c = t.colors;
        let r = self.rect;
        cx.shadow(r, 8, 4);
        cx.round(r, 8, c.flyout_stroke);
        cx.round(r.inflate(-Lu::ONE), 7, c.flyout);
        let gutter = Self::gutter(&self.items);
        for (i, item) in self.items.iter().enumerate() {
            let row = self.row(i);
            if item.separator {
                cx.fill(LRect::new(r.x, row.y + Lu::px(4), r.w, Lu::ONE), c.divider);
                continue;
            }
            let hot = self.highlight == Some(i) && item.enabled;
            if hot {
                cx.round(
                    row,
                    4,
                    if self.pressed == Some(i) {
                        c.subtle_pressed
                    } else {
                        c.subtle_hover
                    },
                );
            }
            let fg = if !item.enabled {
                c.text_disabled
            } else if hot && t.is_high_contrast() {
                c.control_text_hover
            } else {
                c.text
            };
            if let Some(icon) = item.icon {
                cx.icon(
                    icon,
                    LRect::new(
                        row.x + Lu::px(8),
                        row.y + (row.h - Lu::px(16)) / 2,
                        Lu::px(16),
                        Lu::px(16),
                    ),
                    fg,
                );
            }
            if item.check == Some(true) {
                cx.icon(
                    Icon::Check,
                    LRect::new(
                        row.x + Lu::px(10),
                        row.y + (row.h - Lu::px(12)) / 2,
                        Lu::px(12),
                        Lu::px(12),
                    ),
                    fg,
                );
            }
            cx.label(
                &item.label,
                LRect::new(row.x + gutter - Lu::px(4), row.y, row.w - gutter, row.h),
                TextRole::Body,
                fg,
            );
            if !item.accelerator.is_empty() {
                let w = cx.text.measure(&item.accelerator, TextRole::Caption).w;
                cx.label(
                    &item.accelerator,
                    LRect::new(row.right() - Lu::px(12) - w, row.y, w, row.h),
                    TextRole::Caption,
                    if item.enabled {
                        c.text_secondary
                    } else {
                        c.text_disabled
                    },
                );
            }
        }
    }
    /// Menu node and its items as virtual semantics (window bounds).
    pub fn semantics(&self, name: &str, out: &mut Vec<(u32, Node)>, parent: Option<u32>) {
        let mut menu = Node::new(NodeId(1), Role::Menu).named(name);
        menu.parent = parent.map(|p| NodeId(u64::from(p)));
        menu.bounds = bounds(self.rect);
        out.push((MENU_KEY, menu));
        let count = self.items.iter().filter(|i| !i.separator).count() as u32;
        let mut pos = 0;
        for (i, item) in self.items.iter().enumerate() {
            if item.separator {
                continue;
            }
            pos += 1;
            let mut n = Node::new(NodeId(1), Role::MenuItem).named(&item.label);
            n.parent = Some(NodeId(u64::from(MENU_KEY)));
            n.accelerator = item.accelerator.clone();
            n.bounds = bounds(self.row(i));
            n.position = Some((pos, count));
            n.states.set(States::DISABLED, !item.enabled);
            n.states |= States::FOCUSABLE;
            n.actions = Actions::INVOKE;
            if let Some(on) = item.check {
                n.states |= States::CHECKABLE;
                n.states.set(States::CHECKED, on);
            }
            out.push((MENU_KEY + 1 + i as u32, n));
        }
    }
    pub fn item_of_key(&self, key: u32) -> Option<usize> {
        key.checked_sub(MENU_KEY + 1)
            .map(|i| i as usize)
            .filter(|i| *i < self.items.len())
    }
    /// Pointer input in window coordinates. Returns an item to invoke.
    pub fn pointer(&mut self, e: &Event, to_window: impl Fn(LPoint) -> LPoint) -> Option<usize> {
        match e {
            Event::PointerMove(p) => {
                let h = self.index_at(to_window(p.pos));
                if h.is_some() {
                    self.highlight = h;
                }
                None
            }
            Event::PointerDown(p) if p.button == MouseButton::Primary => {
                self.pressed = self.index_at(to_window(p.pos));
                None
            }
            Event::PointerUp(p) => {
                let at = self.index_at(to_window(p.pos));
                let pressed = self.pressed.take();
                (at.is_some() && (pressed.is_none() || pressed == at))
                    .then_some(at)
                    .flatten()
            }
            _ => None,
        }
    }
}
pub(crate) fn bounds(r: LRect) -> Bounds {
    Bounds {
        x: r.x.0,
        y: r.y.0,
        width: r.w.0,
        height: r.h.0,
    }
}
fn window_point(cx: &EventCx<'_>, p: LPoint) -> LPoint {
    cx.to_window(LRect::new(p.x, p.y, Lu::ZERO, Lu::ZERO))
        .origin()
}

/// MenuFlyout: an attachable menu opened at a point (context menu) or
/// below an element. Place it anywhere in the tree; it takes no space.
pub struct MenuFlyout {
    pub items: Vec<MenuItem>,
    pub name: String,
    popup: Option<Popup>,
}
impl MenuFlyout {
    pub fn new(items: Vec<MenuItem>) -> Self {
        Self {
            items,
            name: "Context".into(),
            popup: None,
        }
    }
    pub fn is_open(&self) -> bool {
        self.popup.is_some()
    }
    /// Open at a window point (keyboard: highlight the first item).
    pub fn open_at(&mut self, cx: &mut EventCx<'_>, at: LPoint, keyboard: bool) {
        self.popup = Some(Popup::open(
            self.items.clone(),
            LRect::new(at.x, at.y - Lu::px(4), Lu::ZERO, Lu::ZERO),
            cx.window,
            cx.text,
            keyboard,
        ));
        cx.overlay_opened();
        cx.invalidate();
    }
    pub fn close(&mut self, cx: &mut EventCx<'_>) {
        if self.popup.take().is_some() {
            cx.overlay_closed();
            cx.invalidate();
        }
    }
    fn invoke(&mut self, cx: &mut EventCx<'_>, i: usize) {
        let item = self.items[i].clone();
        self.close(cx);
        if let Some(on) = item.check {
            self.items[i].check = Some(!on);
        }
        cx.raise(item.id);
    }
}
impl Widget for MenuFlyout {
    fn type_name(&self) -> &'static str {
        "MenuFlyout"
    }
    fn style(&self) -> Style {
        Style::default().fixed(0, 0)
    }
    fn paint(&self, _cx: &mut PaintCx<'_>) {}
    fn overlay(&self) -> Option<LRect> {
        self.popup.as_ref().map(|p| p.rect)
    }
    fn paint_overlay(&self, cx: &mut PaintCx<'_>) {
        if let Some(p) = &self.popup {
            p.paint(cx);
        }
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        if cx.phase != Phase::Target {
            return;
        }
        match e {
            Event::Dismiss => {
                self.popup = None;
                cx.invalidate();
                cx.notify(Notify::Dismissed);
            }
            Event::KeyDown(k) => {
                let Some(p) = self.popup.as_mut() else { return };
                cx.set_handled();
                match p.key(k) {
                    Err(()) => {
                        self.close(cx);
                        cx.notify(Notify::Dismissed);
                    }
                    Ok(Some(i)) => self.invoke(cx, i),
                    Ok(None) => cx.invalidate(),
                }
            }
            Event::PointerMove(_) | Event::PointerDown(_) | Event::PointerUp(_) => {
                let Some(p) = self.popup.as_mut() else { return };
                let at = |q: LPoint| window_point(cx, q);
                let hit = p.pointer(e, at);
                cx.set_handled();
                cx.invalidate();
                if let Some(i) = hit {
                    self.invoke(cx, i);
                }
            }
            _ => (),
        }
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        if self.popup.is_none() {
            return false;
        }
        describe(node, Role::Group, &self.name);
        true
    }
    fn virtual_semantics(&self, _cx: &SemanticsCx<'_>, out: &mut Vec<(u32, Node)>) {
        if let Some(p) = &self.popup {
            p.semantics(&self.name, out, None);
        }
    }
    fn focused_item(&self) -> Option<u32> {
        self.popup
            .as_ref()
            .and_then(|p| p.highlight)
            .map(|i| MENU_KEY + 1 + i as u32)
    }
    fn action(&mut self, cx: &mut EventCx<'_>, item: Option<u32>, a: &Action) -> bool {
        let Some(i) = item.and_then(|k| self.popup.as_ref()?.item_of_key(k)) else {
            return false;
        };
        if *a == Action::Invoke && self.items[i].enabled {
            self.invoke(cx, i);
            return true;
        }
        false
    }
}

/// MenuBar: File/Edit/View-style menus. F10 or Alt focuses it; Alt+letter
/// opens a menu; arrows move between titles and items.
pub struct MenuBar {
    pub menus: Vec<(String, Vec<MenuItem>)>,
    focus: usize,
    open: Option<usize>,
    popup: Option<Popup>,
    hover: Option<usize>,
}
impl MenuBar {
    pub fn new(menus: Vec<(&str, Vec<MenuItem>)>) -> Self {
        Self {
            menus: menus
                .into_iter()
                .map(|(t, i)| (String::from(t), i))
                .collect(),
            focus: 0,
            open: None,
            popup: None,
            hover: None,
        }
    }
    fn title(&self, t: &dyn TextSystem, r: LRect, i: usize) -> LRect {
        let mut x = r.x;
        for (title, _) in &self.menus[..i] {
            x += t.measure(title, TextRole::Body).w + Lu::px(24);
        }
        let w = t.measure(&self.menus[i].0, TextRole::Body).w + Lu::px(24);
        LRect::new(x, r.y + Lu::px(4), w, r.h - Lu::px(8))
    }
    fn index_at(&self, t: &dyn TextSystem, r: LRect, p: LPoint) -> Option<usize> {
        (0..self.menus.len()).find(|i| self.title(t, r, *i).contains(p))
    }
    fn open_menu(&mut self, cx: &mut EventCx<'_>, i: usize, keyboard: bool) {
        let anchor = cx.to_window(self.title(cx.text, cx.rect, i));
        self.focus = i;
        self.open = Some(i);
        self.popup = Some(Popup::open(
            self.menus[i].1.clone(),
            LRect::new(anchor.x, anchor.y, anchor.w, anchor.h - Lu::px(4)),
            cx.window,
            cx.text,
            keyboard,
        ));
        cx.overlay_opened();
        cx.invalidate();
    }
    fn close(&mut self, cx: &mut EventCx<'_>) {
        if self.popup.take().is_some() {
            cx.overlay_closed();
        }
        self.open = None;
        cx.invalidate();
    }
    pub fn is_open(&self) -> bool {
        self.popup.is_some()
    }
}
impl Widget for MenuBar {
    fn type_name(&self) -> &'static str {
        "MenuBar"
    }
    fn measure(&self, cx: &LayoutCx<'_>, a: LSize) -> Measured {
        let w = self
            .menus
            .iter()
            .map(|(t, _)| cx.text.measure(t, TextRole::Body).w + Lu::px(24))
            .fold(Lu::ZERO, |x, y| x + y);
        Measured::new(LSize::new(
            if a.w.is_infinite() { w } else { a.w.max(w) },
            Lu::px(40),
        ))
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let c = cx.theme.colors;
        for (i, (title, _)) in self.menus.iter().enumerate() {
            let r = self.title(cx.text, cx.rect, i);
            let active = self.open == Some(i);
            if active || self.hover == Some(i) && cx.state.hovered {
                cx.round(
                    r,
                    4,
                    if active {
                        c.subtle_pressed
                    } else {
                        c.subtle_hover
                    },
                );
            }
            cx.label_centered(
                title,
                r,
                TextRole::Body,
                if cx.state.enabled {
                    c.text
                } else {
                    c.text_disabled
                },
            );
        }
    }
    fn paint_over(&self, cx: &mut PaintCx<'_>) {
        if self.popup.is_none() && !self.menus.is_empty() {
            let r = self.title(cx.text, cx.rect, self.focus);
            cx.focus_if_visible(r, 4);
        }
    }
    fn overlay(&self) -> Option<LRect> {
        self.popup.as_ref().map(|p| p.rect)
    }
    fn paint_overlay(&self, cx: &mut PaintCx<'_>) {
        if let Some(p) = &self.popup {
            p.paint(cx);
        }
    }
    fn wants_accelerators(&self) -> bool {
        true
    }
    fn accelerator(&mut self, cx: &mut EventCx<'_>, k: &KeyEvent) -> bool {
        match k.key {
            Key::F(10) if k.modifiers.none() => {
                cx.enter_menu_mode();
                cx.invalidate();
                true
            }
            Key::Char(ch) if k.modifiers.alt && !k.modifiers.ctrl => {
                let Some(i) = self.menus.iter().position(|(t, _)| {
                    t.chars()
                        .next()
                        .is_some_and(|f| f.to_ascii_lowercase() == ch)
                }) else {
                    return false;
                };
                cx.enter_menu_mode();
                self.open_menu(cx, i, true);
                true
            }
            _ => false,
        }
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        if cx.phase != Phase::Target || self.menus.is_empty() {
            return;
        }
        let n = self.menus.len();
        match e {
            Event::Dismiss => {
                self.popup = None;
                self.open = None;
                cx.invalidate();
            }
            Event::KeyDown(k) => {
                if let Some(p) = self.popup.as_mut() {
                    cx.set_handled();
                    match k.key {
                        Key::Left | Key::Right => {
                            let i = self.open.unwrap_or(0);
                            let next = if k.key == Key::Right {
                                (i + 1) % n
                            } else {
                                (i + n - 1) % n
                            };
                            self.close(cx);
                            self.open_menu(cx, next, true);
                        }
                        _ => match p.key(k) {
                            Err(()) => self.close(cx),
                            Ok(Some(i)) => {
                                let item = p.items[i].clone();
                                self.close(cx);
                                cx.raise(item.id);
                                cx.exit_menu_mode();
                            }
                            Ok(None) => cx.invalidate(),
                        },
                    }
                    return;
                }
                match k.key {
                    Key::Left => self.focus = (self.focus + n - 1) % n,
                    Key::Right => self.focus = (self.focus + 1) % n,
                    Key::Enter | Key::Space | Key::Down => {
                        let f = self.focus;
                        self.open_menu(cx, f, true);
                    }
                    // Leaving menu mode returns focus where it was.
                    Key::Escape => cx.exit_menu_mode(),
                    _ => return,
                }
                cx.invalidate();
                cx.set_handled();
            }
            Event::PointerMove(p) => {
                let wp = window_point(cx, p.pos);
                if let Some(popup) = self.popup.as_mut()
                    && popup.rect.contains(wp)
                {
                    popup.pointer(e, |q| window_point(cx, q));
                    cx.invalidate();
                    return;
                }
                let h = self.index_at(cx.text, cx.rect, p.pos);
                if h != self.hover {
                    self.hover = h;
                    cx.invalidate();
                }
                if let (Some(i), Some(open)) = (h, self.open)
                    && i != open
                {
                    self.close(cx);
                    self.open_menu(cx, i, false);
                }
            }
            Event::PointerLeave => {
                self.hover = None;
                cx.invalidate();
            }
            Event::PointerDown(p) | Event::PointerUp(p) if p.button == MouseButton::Primary => {
                cx.set_handled();
                let wp = window_point(cx, p.pos);
                if let Some(popup) = self.popup.as_mut()
                    && popup.rect.contains(wp)
                {
                    if let Some(i) = popup.pointer(e, |q| window_point(cx, q)) {
                        let item = popup.items[i].clone();
                        self.close(cx);
                        cx.raise(item.id);
                        cx.exit_menu_mode();
                    }
                    cx.invalidate();
                    return;
                }
                if matches!(e, Event::PointerDown(_))
                    && let Some(i) = self.index_at(cx.text, cx.rect, p.pos)
                {
                    if self.open == Some(i) {
                        self.close(cx);
                    } else {
                        self.close(cx);
                        self.open_menu(cx, i, false);
                    }
                }
            }
            _ => (),
        }
    }
    fn focusable(&self) -> bool {
        !self.menus.is_empty()
    }
    fn focused_item(&self) -> Option<u32> {
        match (&self.popup, self.open) {
            (Some(p), Some(_)) => p
                .highlight
                .map(|i| MENU_KEY + 1 + i as u32)
                .or(Some(self.focus as u32)),
            _ => Some(self.focus as u32),
        }
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(node, Role::MenuBar, "");
        true
    }
    fn virtual_semantics(&self, cx: &SemanticsCx<'_>, out: &mut Vec<(u32, Node)>) {
        for (i, (title, _)) in self.menus.iter().enumerate() {
            let mut n = Node::new(NodeId(1), Role::MenuItem).named(title);
            n.bounds = bounds(self.title(cx.text, cx.rect, i));
            n.states |= States::FOCUSABLE | States::HAS_POPUP | States::EXPANDABLE;
            n.states.set(States::EXPANDED, self.open == Some(i));
            n.actions = Actions::EXPAND | Actions::COLLAPSE;
            n.position = Some((i as u32 + 1, self.menus.len() as u32));
            n.accelerator = alloc::format!(
                "Alt+{}",
                title.chars().next().unwrap_or(' ').to_ascii_uppercase()
            );
            out.push((i as u32, n));
        }
        if let (Some(p), Some(i)) = (&self.popup, self.open) {
            p.semantics(&self.menus[i].0, out, Some(i as u32));
        }
    }
    fn action(&mut self, cx: &mut EventCx<'_>, item: Option<u32>, a: &Action) -> bool {
        let Some(k) = item else { return false };
        if let Some(i) = self.popup.as_ref().and_then(|p| p.item_of_key(k)) {
            if *a == Action::Invoke {
                let item = self.popup.as_ref().unwrap().items[i].clone();
                if !item.enabled {
                    return false;
                }
                self.close(cx);
                cx.raise(item.id);
                return true;
            }
            return false;
        }
        let i = k as usize;
        if i >= self.menus.len() {
            return false;
        }
        match a {
            Action::Expand => self.open_menu(cx, i, true),
            Action::Collapse => self.close(cx),
            Action::Focus => {
                self.focus = i;
                cx.request_focus();
            }
            _ => return false,
        }
        true
    }
}

/// A CommandBar command (label shown to the right of the icon).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub id: u32,
    pub label: String,
    pub icon: Option<Icon>,
    pub enabled: bool,
}
impl Command {
    pub fn new(id: u32, label: &str, icon: Icon) -> Self {
        Self {
            id,
            label: label.into(),
            icon: Some(icon),
            enabled: true,
        }
    }
}
/// CommandBar: primary commands plus a "See more" overflow menu. One tab
/// stop; Left/Right move between commands.
pub struct CommandBar {
    pub commands: Vec<Command>,
    pub secondary: Vec<MenuItem>,
    focus: usize,
    hover: Option<usize>,
    pressed: Option<usize>,
    popup: Option<Popup>,
}
impl CommandBar {
    pub fn new(commands: Vec<Command>, secondary: Vec<MenuItem>) -> Self {
        Self {
            commands,
            secondary,
            focus: 0,
            hover: None,
            pressed: None,
            popup: None,
        }
    }
    fn count(&self) -> usize {
        self.commands.len() + usize::from(!self.secondary.is_empty())
    }
    fn button(&self, t: &dyn TextSystem, r: LRect, i: usize) -> LRect {
        let width = |c: &Command| {
            Lu::px(24)
                + Lu::px(16)
                + if c.label.is_empty() {
                    Lu::ZERO
                } else {
                    Lu::px(8) + t.measure(&c.label, TextRole::Body).w
                }
        };
        let mut x = r.x + Lu::px(4);
        for c in &self.commands[..i.min(self.commands.len())] {
            x += width(c) + Lu::px(4);
        }
        let w = if i < self.commands.len() {
            width(&self.commands[i])
        } else {
            Lu::px(40)
        };
        LRect::new(x, r.y + Lu::px(4), w, r.h - Lu::px(8))
    }
    fn index_at(&self, t: &dyn TextSystem, r: LRect, p: LPoint) -> Option<usize> {
        (0..self.count()).find(|i| self.button(t, r, *i).contains(p))
    }
    fn invoke(&mut self, cx: &mut EventCx<'_>, i: usize) {
        if i < self.commands.len() {
            if self.commands[i].enabled {
                cx.raise(self.commands[i].id);
            }
        } else if self.popup.is_some() {
            self.popup = None;
            cx.overlay_closed();
        } else {
            let b = cx.to_window(self.button(cx.text, cx.rect, i));
            let s = Popup::size(&self.secondary, cx.text);
            let anchor = LRect::new(b.right() - s.w, b.y, s.w, b.h);
            self.popup = Some(Popup::open(
                self.secondary.clone(),
                anchor,
                cx.window,
                cx.text,
                true,
            ));
            cx.overlay_opened();
        }
        cx.invalidate();
    }
}
impl Widget for CommandBar {
    fn type_name(&self) -> &'static str {
        "CommandBar"
    }
    fn measure(&self, cx: &LayoutCx<'_>, a: LSize) -> Measured {
        let r = LRect::new(Lu::ZERO, Lu::ZERO, Lu::px(10_000), Lu::px(48));
        let w = if self.count() == 0 {
            Lu::ZERO
        } else {
            self.button(cx.text, r, self.count() - 1).right() + Lu::px(4)
        };
        Measured::new(LSize::new(
            if a.w.is_infinite() { w } else { a.w.max(w) },
            Lu::px(48),
        ))
    }
    fn paint(&self, cx: &mut PaintCx<'_>) {
        let c = cx.theme.colors;
        for i in 0..self.count() {
            let b = self.button(cx.text, cx.rect, i);
            let enabled = cx.state.enabled && self.commands.get(i).is_none_or(|c| c.enabled);
            let pressed = self.pressed == Some(i) && cx.state.captured
                || i == self.commands.len() && self.popup.is_some();
            if enabled && (pressed || self.hover == Some(i) && cx.state.hovered) {
                cx.round(
                    b,
                    4,
                    if pressed {
                        c.subtle_pressed
                    } else {
                        c.subtle_hover
                    },
                );
            }
            let fg = if enabled { c.text } else { c.text_disabled };
            match self.commands.get(i) {
                Some(cmd) => {
                    if let Some(icon) = cmd.icon {
                        cx.icon(
                            icon,
                            LRect::new(
                                b.x + Lu::px(12),
                                b.y + (b.h - Lu::px(16)) / 2,
                                Lu::px(16),
                                Lu::px(16),
                            ),
                            fg,
                        );
                    }
                    cx.label(
                        &cmd.label,
                        LRect::new(b.x + Lu::px(36), b.y, b.w - Lu::px(36), b.h),
                        TextRole::Body,
                        fg,
                    );
                }
                None => cx.icon_centered(Icon::More, b, 16, fg),
            }
        }
    }
    fn paint_over(&self, cx: &mut PaintCx<'_>) {
        if self.count() > 0 {
            let b = self.button(cx.text, cx.rect, self.focus.min(self.count() - 1));
            cx.focus_if_visible(b, 4);
        }
    }
    fn overlay(&self) -> Option<LRect> {
        self.popup.as_ref().map(|p| p.rect)
    }
    fn paint_overlay(&self, cx: &mut PaintCx<'_>) {
        if let Some(p) = &self.popup {
            p.paint(cx);
        }
    }
    fn event(&mut self, cx: &mut EventCx<'_>, e: &Event) {
        if cx.phase != Phase::Target || self.count() == 0 {
            return;
        }
        let n = self.count();
        match e {
            Event::Dismiss => {
                self.popup = None;
                cx.invalidate();
            }
            Event::KeyDown(k) => {
                if let Some(p) = self.popup.as_mut() {
                    cx.set_handled();
                    match p.key(k) {
                        Err(()) => {
                            self.popup = None;
                            cx.overlay_closed();
                        }
                        Ok(Some(i)) => {
                            let id = p.items[i].id;
                            self.popup = None;
                            cx.overlay_closed();
                            cx.raise(id);
                        }
                        Ok(None) => (),
                    }
                    cx.invalidate();
                    return;
                }
                match k.key {
                    Key::Left => self.focus = (self.focus + n - 1) % n,
                    Key::Right => self.focus = (self.focus + 1) % n,
                    Key::Home => self.focus = 0,
                    Key::End => self.focus = n - 1,
                    Key::Enter | Key::Space => {
                        let f = self.focus;
                        self.invoke(cx, f);
                    }
                    _ => return,
                }
                cx.invalidate();
                cx.set_handled();
            }
            Event::PointerMove(p) => {
                let wp = window_point(cx, p.pos);
                if let Some(popup) = self.popup.as_mut()
                    && popup.rect.contains(wp)
                {
                    popup.pointer(e, |q| window_point(cx, q));
                    cx.invalidate();
                    return;
                }
                let h = self.index_at(cx.text, cx.rect, p.pos);
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
                cx.set_handled();
                let wp = window_point(cx, p.pos);
                if let Some(popup) = self.popup.as_mut()
                    && popup.rect.contains(wp)
                {
                    popup.pointer(e, |q| window_point(cx, q));
                    return;
                }
                self.pressed = self.index_at(cx.text, cx.rect, p.pos);
                if let Some(i) = self.pressed {
                    self.focus = i;
                    cx.capture_pointer();
                }
                cx.invalidate();
            }
            Event::PointerUp(p) if p.button == MouseButton::Primary => {
                cx.set_handled();
                let wp = window_point(cx, p.pos);
                if let Some(popup) = self.popup.as_mut()
                    && popup.rect.contains(wp)
                {
                    if let Some(i) = popup.pointer(e, |q| window_point(cx, q)) {
                        let id = popup.items[i].id;
                        self.popup = None;
                        cx.overlay_closed();
                        cx.raise(id);
                    }
                    cx.invalidate();
                    return;
                }
                if let Some(i) = self.pressed.take()
                    && self.index_at(cx.text, cx.rect, p.pos) == Some(i)
                {
                    self.invoke(cx, i);
                }
                cx.invalidate();
            }
            Event::CaptureLost => {
                self.pressed = None;
                cx.invalidate();
            }
            _ => (),
        }
    }
    fn focusable(&self) -> bool {
        self.count() > 0
    }
    fn focused_item(&self) -> Option<u32> {
        match &self.popup {
            Some(p) => p.highlight.map(|i| MENU_KEY + 1 + i as u32),
            None => Some(self.focus as u32),
        }
    }
    fn semantics(&self, _cx: &SemanticsCx<'_>, node: &mut Node) -> bool {
        describe(node, Role::ToolBar, "Command bar");
        true
    }
    fn virtual_semantics(&self, cx: &SemanticsCx<'_>, out: &mut Vec<(u32, Node)>) {
        for i in 0..self.count() {
            let (name, enabled) = match self.commands.get(i) {
                Some(c) => (c.label.as_str(), c.enabled),
                None => ("See more", true),
            };
            let mut n = Node::new(NodeId(1), Role::Button).named(name);
            n.bounds = bounds(self.button(cx.text, cx.rect, i));
            n.states |= States::FOCUSABLE;
            n.states.set(States::DISABLED, !enabled);
            n.actions = Actions::INVOKE;
            if i == self.commands.len() {
                n.states |= States::HAS_POPUP | States::EXPANDABLE;
                n.states.set(States::EXPANDED, self.popup.is_some());
            }
            out.push((i as u32, n));
        }
        if let Some(p) = &self.popup {
            p.semantics("More options", out, Some(self.commands.len() as u32));
        }
    }
    fn action(&mut self, cx: &mut EventCx<'_>, item: Option<u32>, a: &Action) -> bool {
        let Some(k) = item else { return false };
        if let Some(i) = self.popup.as_ref().and_then(|p| p.item_of_key(k)) {
            if *a == Action::Invoke {
                let id = self.popup.as_ref().unwrap().items[i].id;
                self.popup = None;
                cx.overlay_closed();
                cx.raise(id);
                return true;
            }
            return false;
        }
        let i = k as usize;
        if i < self.count() && *a == Action::Invoke {
            self.invoke(cx, i);
            return true;
        }
        if i < self.count() && *a == Action::Focus {
            self.focus = i;
            cx.request_focus();
            return true;
        }
        false
    }
}
