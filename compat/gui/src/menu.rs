//! Menus: the `HMENU` model behind CreateMenu/AppendMenuW/InsertMenuW/
//! CheckMenuItem/EnableMenuItem/GetMenuState/GetMenuStringW/..., menu-bar
//! layout, mnemonics, and the keyboard state machine of menu tracking.
//! Submenus are separate handles; DestroyMenu releases them recursively.
use crate::handles::{Handle, Table, kind};
use crate::msg::{mf, mfs, mft};
use crate::resource::MenuNode;
use crate::{Error, Rect, Result, try_push};
use alloc::vec::Vec;

pub const MAX_MENUS: usize = 4096;
pub const MAX_ITEMS: usize = 512;
pub const NOT_FOUND: u32 = 0xffff_ffff;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    /// MFT_* type bits (separator, owner draw, radio check, breaks).
    pub kind: u32,
    /// MFS_* state bits (grayed/disabled, checked, highlighted, default).
    pub state: u32,
    pub id: u32,
    pub text: Vec<u16>,
    pub submenu: Option<Handle>,
    pub data: u64,
}
impl Item {
    pub fn separator(&self) -> bool {
        self.kind & mft::SEPARATOR != 0
    }
    pub fn enabled(&self) -> bool {
        self.state & (mf::GRAYED | mf::DISABLED) == 0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Menu {
    pub items: Vec<Item>,
    pub popup: bool,
    pub data: u64,
}

pub struct Menus {
    table: Table<Menu>,
}

impl Default for Menus {
    fn default() -> Self {
        Self::new()
    }
}

impl Menus {
    pub const fn new() -> Self {
        Self {
            table: Table::new(kind::MENU, MAX_MENUS),
        }
    }
    pub fn len(&self) -> usize {
        self.table.len()
    }
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }
    pub fn create(&mut self, popup: bool) -> Result<Handle> {
        self.table.insert(Menu {
            popup,
            ..Menu::default()
        })
    }
    pub fn resolve(&self, value: u64) -> Option<Handle> {
        self.table.resolve(value)
    }
    pub fn get(&self, h: Handle) -> Option<&Menu> {
        self.table.get(h)
    }
    pub fn get_mut(&mut self, h: Handle) -> Option<&mut Menu> {
        self.table.get_mut(h)
    }
    /// DestroyMenu: the menu and, recursively, every submenu.
    pub fn destroy(&mut self, h: Handle) -> Result<()> {
        let menu = self.table.remove(h).ok_or(Error::InvalidHandle)?;
        for item in menu.items {
            if let Some(sub) = item.submenu {
                let _ = self.destroy(sub);
            }
        }
        Ok(())
    }
    /// Build a menu bar from a parsed `MENU`/`MENUEX` template.
    pub fn from_template(&mut self, nodes: &[MenuNode], popup: bool) -> Result<Handle> {
        let h = self.create(popup)?;
        for n in nodes {
            let sub = match &n.children {
                Some(kids) => match self.from_template(kids, true) {
                    Ok(s) => Some(s),
                    Err(e) => {
                        let _ = self.destroy(h);
                        return Err(e);
                    }
                },
                None => None,
            };
            let separator = n.text.is_empty() && sub.is_none() && n.id == 0;
            let item = Item {
                kind: n.flags | if separator { mft::SEPARATOR } else { 0 },
                state: n.state,
                id: n.id,
                text: n.text.clone(),
                submenu: sub,
                data: 0,
            };
            let menu = self.table.get_mut(h).unwrap();
            if let Err(e) = try_push(&mut menu.items, item, MAX_ITEMS) {
                let _ = self.destroy(h);
                return Err(e);
            }
        }
        Ok(h)
    }
    /// InsertMenuW/AppendMenuW. `position` None appends. With MF_POPUP the
    /// `id_or_menu` value must name an existing menu.
    pub fn insert(
        &mut self,
        h: Handle,
        position: Option<(u32, bool)>,
        flags: u32,
        id_or_menu: u64,
        text: Vec<u16>,
    ) -> Result<()> {
        let submenu = if flags & mf::POPUP != 0 {
            Some(self.resolve(id_or_menu).ok_or(Error::InvalidHandle)?)
        } else {
            None
        };
        if submenu == Some(h) {
            return Err(Error::InvalidParameter);
        }
        let item = Item {
            kind: flags
                & (mft::SEPARATOR
                    | mft::OWNERDRAW
                    | mft::MENUBARBREAK
                    | mft::MENUBREAK
                    | mft::RIGHTJUSTIFY
                    | mf::BITMAP),
            state: flags & (mf::GRAYED | mf::DISABLED | mf::CHECKED | mf::HILITE | mf::DEFAULT),
            id: if submenu.is_some() {
                id_or_menu as u32
            } else {
                id_or_menu as u32 & 0xffff
            },
            text: if flags & mf::SEPARATOR != 0 {
                Vec::new()
            } else {
                text
            },
            submenu,
            data: 0,
        };
        let (target, index) = match position {
            None => (h, None),
            Some((p, by_position)) => match self.find(h, p, by_position) {
                Some((m, i)) => (m, Some(i)),
                None if by_position => (h, None),
                None => return Err(Error::InvalidParameter),
            },
        };
        let menu = self.table.get_mut(target).ok_or(Error::InvalidHandle)?;
        if menu.items.len() >= MAX_ITEMS {
            return Err(Error::Quota);
        }
        menu.items.try_reserve(1).map_err(|_| Error::Quota)?;
        match index {
            Some(i) => menu.items.insert(i, item),
            None => menu.items.push(item),
        }
        Ok(())
    }
    /// Locate an item by position or (recursively) by command.
    pub fn find(&self, h: Handle, item: u32, by_position: bool) -> Option<(Handle, usize)> {
        let menu = self.table.get(h)?;
        if by_position {
            return ((item as usize) < menu.items.len()).then_some((h, item as usize));
        }
        for (i, it) in menu.items.iter().enumerate() {
            if it.submenu.is_none() && it.id == item {
                return Some((h, i));
            }
        }
        for it in &menu.items {
            if let Some(sub) = it.submenu {
                if let Some(found) = self.find(sub, item, false) {
                    return Some(found);
                }
                if it.id == item {
                    return self.table.get(h).and_then(|m| {
                        m.items
                            .iter()
                            .position(|x| x.submenu == Some(sub))
                            .map(|i| (h, i))
                    });
                }
            }
        }
        None
    }
    pub fn item(&self, h: Handle, item: u32, by_position: bool) -> Option<&Item> {
        let (m, i) = self.find(h, item, by_position)?;
        self.table.get(m)?.items.get(i)
    }
    fn item_mut(&mut self, h: Handle, item: u32, by_position: bool) -> Option<&mut Item> {
        let (m, i) = self.find(h, item, by_position)?;
        self.table.get_mut(m)?.items.get_mut(i)
    }
    pub fn count(&self, h: Handle) -> Option<usize> {
        self.table.get(h).map(|m| m.items.len())
    }
    /// GetMenuItemID: the command, -1 for a popup or a missing position.
    pub fn item_id(&self, h: Handle, position: u32) -> u32 {
        match self.item(h, position, true) {
            Some(it) if it.submenu.is_none() => it.id,
            _ => NOT_FOUND,
        }
    }
    pub fn submenu(&self, h: Handle, position: u32) -> Option<Handle> {
        self.item(h, position, true).and_then(|it| it.submenu)
    }
    /// GetMenuState: the MF_* flags; a popup adds its item count << 8.
    pub fn state(&self, h: Handle, item: u32, by_position: bool) -> u32 {
        let Some(it) = self.item(h, item, by_position) else {
            return NOT_FOUND;
        };
        let mut flags = it.kind | it.state;
        if let Some(sub) = it.submenu {
            flags |= mf::POPUP | (self.count(sub).unwrap_or(0) as u32 & 0xff) << 8;
        }
        flags
    }
    /// CheckMenuItem: returns the previous MF_CHECKED state or -1.
    pub fn check(&mut self, h: Handle, item: u32, flags: u32) -> u32 {
        let Some(it) = self.item_mut(h, item, flags & mf::BYPOSITION != 0) else {
            return NOT_FOUND;
        };
        let old = it.state & mf::CHECKED;
        it.state = (it.state & !mf::CHECKED) | (flags & mf::CHECKED);
        old
    }
    /// EnableMenuItem: returns the previous enabled state or -1.
    pub fn enable(&mut self, h: Handle, item: u32, flags: u32) -> u32 {
        let Some(it) = self.item_mut(h, item, flags & mf::BYPOSITION != 0) else {
            return NOT_FOUND;
        };
        let old = it.state & (mf::GRAYED | mf::DISABLED);
        it.state =
            (it.state & !(mf::GRAYED | mf::DISABLED)) | (flags & (mf::GRAYED | mf::DISABLED));
        old
    }
    pub fn set_state(&mut self, h: Handle, item: u32, by_position: bool, state: u32) -> bool {
        match self.item_mut(h, item, by_position) {
            Some(it) => {
                it.state = state & (mfs::GRAYED | mfs::CHECKED | mfs::HILITE | mfs::DEFAULT);
                true
            }
            None => false,
        }
    }
    pub fn set_text(&mut self, h: Handle, item: u32, by_position: bool, text: Vec<u16>) -> bool {
        match self.item_mut(h, item, by_position) {
            Some(it) => {
                it.text = text;
                true
            }
            None => false,
        }
    }
    /// RemoveMenu (keeps a submenu alive) / DeleteMenu (destroys it).
    pub fn remove(&mut self, h: Handle, item: u32, by_position: bool, destroy: bool) -> Result<()> {
        let (m, i) = self
            .find(h, item, by_position)
            .ok_or(Error::InvalidParameter)?;
        let removed = self.table.get_mut(m).unwrap().items.remove(i);
        if destroy && let Some(sub) = removed.submenu {
            let _ = self.destroy(sub);
        }
        Ok(())
    }
    /// The menu that contains `sub` as a popup, for WM_INITMENUPOPUP.
    pub fn parent_of(&self, sub: Handle) -> Option<(Handle, usize)> {
        self.table.iter().find_map(|(h, m)| {
            m.items
                .iter()
                .position(|it| it.submenu == Some(sub))
                .map(|i| (h, i))
        })
    }
}

/// The mnemonic of a menu or control label (`&x`), lower-cased ASCII.
pub fn mnemonic(text: &[u16]) -> Option<u16> {
    let mut i = 0;
    while i + 1 < text.len() {
        if text[i] == u16::from(b'&') {
            if text[i + 1] == u16::from(b'&') {
                i += 2;
                continue;
            }
            return Some(crate::geom::wide::fold(text[i + 1]));
        }
        i += 1;
    }
    None
}
/// Display text: `&x` marks removed, `&&` kept as `&`, the part after a
/// tab returned separately (accelerator column).
pub fn display(text: &[u16]) -> (Vec<u16>, Vec<u16>, Option<usize>) {
    let (label, accel) = match text.iter().position(|&u| u == 9) {
        Some(t) => (&text[..t], &text[t + 1..]),
        None => (text, &[][..]),
    };
    let mut out = Vec::new();
    let mut underline = None;
    let mut i = 0;
    while i < label.len() {
        if label[i] == u16::from(b'&') && i + 1 < label.len() {
            if label[i + 1] != u16::from(b'&') && underline.is_none() {
                underline = Some(out.len());
            }
            out.push(label[i + 1]);
            i += 2;
        } else if label[i] == u16::from(b'&') {
            i += 1;
        } else {
            out.push(label[i]);
            i += 1;
        }
    }
    (out, accel.to_vec(), underline)
}

/// Horizontal menu-bar layout: item rectangles for a bar of `width`
/// pixels; widths come from the caller's text measurement. Items wrap to
/// further rows when they do not fit.
pub fn layout_bar(widths: &[i32], width: i32, row_height: i32) -> Vec<Rect> {
    let mut out = Vec::new();
    let (mut x, mut y) = (0, 0);
    for &w in widths {
        let w = w + 18;
        if x > 0 && x + w > width {
            x = 0;
            y += row_height;
        }
        out.push(Rect::new(x, y, x + w, y + row_height));
        x += w;
    }
    out
}

/// Keyboard menu tracking over a menu bar and its popups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tracker {
    /// Selected top-level (bar) index.
    pub bar: usize,
    /// Open popup stack: (menu, highlighted item).
    pub popups: Vec<(Handle, Option<usize>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Redraw,
    Command(u32),
    Close,
    /// A popup was opened (WM_INITMENUPOPUP is due).
    Opened(Handle),
}

impl Tracker {
    pub fn new(bar: usize) -> Self {
        Self {
            bar,
            popups: Vec::new(),
        }
    }
    fn selectable(menus: &Menus, m: Handle, from: Option<usize>, forward: bool) -> Option<usize> {
        let menu = menus.get(m)?;
        let n = menu.items.len();
        if n == 0 {
            return None;
        }
        let mut i = from.unwrap_or(if forward { n - 1 } else { 0 });
        for _ in 0..n {
            i = if forward {
                (i + 1) % n
            } else {
                (i + n - 1) % n
            };
            if !menu.items[i].separator() {
                return Some(i);
            }
        }
        None
    }
    /// Open the popup of bar item `self.bar`.
    pub fn open(&mut self, menus: &Menus, bar: Handle) -> Action {
        self.popups.clear();
        match menus.submenu(bar, self.bar as u32) {
            Some(sub) => {
                let first = Self::selectable(menus, sub, None, true);
                self.popups.push((sub, first));
                Action::Opened(sub)
            }
            None => Action::Redraw,
        }
    }
    fn activate(&mut self, menus: &Menus, m: Handle, i: usize) -> Action {
        let Some(it) = menus.get(m).and_then(|x| x.items.get(i)) else {
            return Action::None;
        };
        if !it.enabled() {
            return Action::None;
        }
        if let Some(sub) = it.submenu {
            let first = Self::selectable(menus, sub, None, true);
            self.popups.push((sub, first));
            return Action::Opened(sub);
        }
        Action::Command(it.id)
    }
    /// One virtual key (or character, for mnemonics) while tracking.
    pub fn key(&mut self, menus: &Menus, bar: Handle, key: u8, ch: Option<u16>) -> Action {
        use crate::keyboard::vk;
        let count = menus.count(bar).unwrap_or(0).max(1);
        match key {
            vk::ESCAPE => {
                if self.popups.len() > 1 {
                    self.popups.pop();
                    Action::Redraw
                } else if !self.popups.is_empty() {
                    self.popups.clear();
                    Action::Redraw
                } else {
                    Action::Close
                }
            }
            vk::LEFT | vk::RIGHT if self.popups.len() <= 1 => {
                self.bar = if key == vk::RIGHT {
                    (self.bar + 1) % count
                } else {
                    (self.bar + count - 1) % count
                };
                if self.popups.is_empty() {
                    Action::Redraw
                } else {
                    self.open(menus, bar)
                }
            }
            vk::LEFT => {
                self.popups.pop();
                Action::Redraw
            }
            vk::UP | vk::DOWN => {
                if self.popups.is_empty() {
                    return self.open(menus, bar);
                }
                let (m, cur) = *self.popups.last().unwrap();
                let next = Self::selectable(menus, m, cur, key == vk::DOWN);
                self.popups.last_mut().unwrap().1 = next;
                Action::Redraw
            }
            vk::RIGHT | vk::RETURN => {
                if self.popups.is_empty() {
                    return self.open(menus, bar);
                }
                let (m, cur) = *self.popups.last().unwrap();
                match cur {
                    Some(i) => {
                        let has_sub = menus
                            .get(m)
                            .and_then(|x| x.items.get(i))
                            .is_some_and(|it| it.submenu.is_some());
                        if key == vk::RIGHT && !has_sub {
                            return Action::None;
                        }
                        self.activate(menus, m, i)
                    }
                    None => Action::None,
                }
            }
            _ => {
                let Some(c) = ch.map(crate::geom::wide::fold) else {
                    return Action::None;
                };
                let (m, from_bar) = match self.popups.last() {
                    Some(&(m, _)) => (m, false),
                    None => (bar, true),
                };
                let Some(menu) = menus.get(m) else {
                    return Action::None;
                };
                let hit = menu
                    .items
                    .iter()
                    .position(|it| mnemonic(&it.text) == Some(c));
                match hit {
                    Some(i) if from_bar => {
                        self.bar = i;
                        self.open(menus, bar)
                    }
                    Some(i) => {
                        self.popups.last_mut().unwrap().1 = Some(i);
                        self.activate(menus, m, i)
                    }
                    None => Action::None,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    /// The F1 fixture's menus.
    fn fixture(m: &mut Menus) -> (Handle, Handle, Handle) {
        let bar = m.create(false).unwrap();
        let file = m.create(true).unwrap();
        let help = m.create(true).unwrap();
        m.insert(file, None, mf::STRING, 101, w("&Run\tF5"))
            .unwrap();
        m.insert(file, None, mf::SEPARATOR, 0, Vec::new()).unwrap();
        m.insert(file, None, mf::STRING, 102, w("E&xit")).unwrap();
        m.insert(help, None, mf::STRING, 103, w("&About")).unwrap();
        m.insert(bar, None, mf::POPUP, file.value(), w("&File"))
            .unwrap();
        m.insert(bar, None, mf::POPUP, help.value(), w("&Help"))
            .unwrap();
        (bar, file, help)
    }
    #[test]
    fn fixture_menu_queries() {
        let mut m = Menus::new();
        let (bar, file, help) = fixture(&mut m);
        assert_eq!(m.count(bar), Some(2));
        assert_eq!(m.item(bar, 0, true).unwrap().text, w("&File"));
        assert_eq!(m.submenu(bar, 0), Some(file));
        assert_eq!(m.count(file), Some(3));
        assert_eq!(m.item_id(file, 0), 101);
        assert_eq!(m.item_id(file, 1), 0, "separator");
        assert_eq!(m.item_id(bar, 0), NOT_FOUND, "popup");
        assert_eq!(m.item_id(file, 9), NOT_FOUND);
        assert_eq!(
            m.item(bar, 101, false).unwrap().text,
            w("&Run\tF5"),
            "by command, recursive"
        );
        assert_eq!(
            m.check(file, 101, mf::BYCOMMAND | mf::CHECKED),
            mf::UNCHECKED
        );
        assert!(m.state(file, 101, false) & mf::CHECKED != 0);
        assert_eq!(m.check(file, 101, mf::UNCHECKED), mf::CHECKED);
        assert_eq!(m.enable(file, 102, mf::GRAYED), mf::ENABLED);
        assert!(m.state(file, 102, false) & mf::GRAYED != 0);
        assert_eq!(m.enable(file, 999, mf::GRAYED), NOT_FOUND);
        assert_eq!(m.state(bar, 0, true) & 0xff10, mf::POPUP | 3 << 8);
        assert_eq!(m.parent_of(help), Some((bar, 1)));
        assert!(m.insert(bar, None, mf::POPUP, 0x1234, w("x")).is_err());
        assert!(m.insert(bar, None, mf::POPUP, bar.value(), w("x")).is_err());
        m.insert(file, Some((0, true)), mf::STRING, 100, w("First"))
            .unwrap();
        assert_eq!(m.item_id(file, 0), 100);
        m.remove(file, 100, false, true).unwrap();
        assert_eq!(m.len(), 3);
        m.destroy(bar).unwrap();
        assert!(m.is_empty(), "submenus destroyed recursively");
        assert!(m.get(file).is_none());
    }
    #[test]
    fn mnemonics_and_display_text() {
        assert_eq!(mnemonic(&w("E&xit")), Some(u16::from(b'x')));
        assert_eq!(mnemonic(&w("A&&B &Copy")), Some(u16::from(b'c')));
        assert_eq!(mnemonic(&w("Plain")), None);
        let (label, accel, under) = display(&w("&Run\tF5"));
        assert_eq!((label, accel, under), (w("Run"), w("F5"), Some(0)));
        let (label, _, under) = display(&w("A&&B"));
        assert_eq!((label, under), (w("A&B"), None));
        let rects = layout_bar(&[20, 30, 400], 200, 20);
        assert_eq!(rects[0], Rect::new(0, 0, 38, 20));
        assert_eq!(rects[1], Rect::new(38, 0, 86, 20));
        assert_eq!(rects[2].top, 20, "wraps");
    }
    #[test]
    fn keyboard_tracking() {
        use crate::keyboard::vk;
        let mut m = Menus::new();
        let (bar, file, help) = fixture(&mut m);
        let mut t = Tracker::new(0);
        assert_eq!(t.open(&m, bar), Action::Opened(file));
        assert_eq!(t.popups, [(file, Some(0))]);
        assert_eq!(t.key(&m, bar, vk::DOWN, None), Action::Redraw);
        assert_eq!(t.popups[0].1, Some(2), "skips the separator");
        assert_eq!(t.key(&m, bar, vk::RETURN, None), Action::Command(102));
        let mut t = Tracker::new(0);
        assert_eq!(t.key(&m, bar, vk::RIGHT, None), Action::Redraw);
        assert_eq!(t.bar, 1);
        assert_eq!(
            t.key(&m, bar, 0, Some(u16::from(b'f'))),
            Action::Opened(file)
        );
        assert_eq!(t.key(&m, bar, vk::RIGHT, None), Action::Opened(help));
        assert_eq!(t.key(&m, bar, vk::LEFT, None), Action::Opened(file));
        assert_eq!(
            t.key(&m, bar, vk::LEFT, None),
            Action::Opened(help),
            "wraps"
        );
        assert_eq!(
            t.key(&m, bar, 0, Some(u16::from(b'A'))),
            Action::Command(103)
        );
        assert_eq!(t.key(&m, bar, vk::ESCAPE, None), Action::Redraw);
        assert_eq!(t.key(&m, bar, vk::ESCAPE, None), Action::Close);
        m.enable(file, 101, mf::GRAYED);
        let mut t = Tracker::new(0);
        t.open(&m, bar);
        assert_eq!(t.key(&m, bar, vk::RETURN, None), Action::None, "grayed");
    }
    #[test]
    fn templates_build_menus() {
        use crate::resource::MenuNode;
        let node = |id: u32, text: &str, children: Option<Vec<MenuNode>>| MenuNode {
            flags: 0,
            state: 0,
            id,
            text: w(text),
            help_id: 0,
            children,
        };
        let tree = [node(
            0,
            "&File",
            Some(alloc::vec![node(1, "&New", None), node(0, "", None)]),
        )];
        let mut m = Menus::new();
        let bar = m.from_template(&tree, false).unwrap();
        let file = m.submenu(bar, 0).unwrap();
        assert!(m.get(file).unwrap().popup);
        assert!(m.item(file, 1, true).unwrap().separator());
        assert_eq!(m.len(), 2);
    }
}
