//! Window classes and the window tree: class registration with per-instance
//! and system scopes, generational `HWND`s, parent/child/owner relations,
//! sibling order, coordinate mapping, visibility and per-window update
//! regions (invalidation propagates into children of a parent without
//! WS_CLIPCHILDREN, as in Windows).
use crate::handles::{Handle, Table, kind};
use crate::msg::ws;
use crate::region::{Op, Region};
use crate::win32_error as e;
use crate::{Point, Rect, try_push};
use alloc::vec::Vec;

pub const MAX_WINDOWS: usize = 4096;
pub const MAX_CLASSES: usize = 512;
pub const MAX_EXTRA: u32 = 40_000;
pub const MAX_TEXT: usize = 0x7fff;

pub type Win32Result<T> = core::result::Result<T, u32>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Class {
    pub atom: u16,
    pub name: Vec<u16>,
    /// Registering module; 0 for system classes.
    pub instance: u64,
    pub global: bool,
    pub system: bool,
    pub style: u32,
    pub procedure: u64,
    pub class_extra: Vec<u8>,
    pub window_extra: u32,
    pub icon: u64,
    pub cursor: u64,
    pub background: u64,
    pub menu_name: u64,
    pub small_icon: u64,
    pub windows: u32,
}

/// A class reference from an application: an atom or a name.
#[derive(Clone, Copy, Debug)]
pub enum ClassRef<'a> {
    Atom(u16),
    Name(&'a [u16]),
}

#[derive(Default)]
pub struct Classes {
    list: Vec<Class>,
    next_atom: u16,
}

fn same_name(a: &[u16], b: &[u16]) -> bool {
    crate::geom::wide::eq_ignore_case(a, b)
}

impl Classes {
    pub fn new() -> Self {
        Self {
            list: Vec::new(),
            next_atom: 0xc000,
        }
    }
    pub fn register(&mut self, mut class: Class) -> Win32Result<u16> {
        if class.name.is_empty() || class.name.len() > 255 || class.window_extra > MAX_EXTRA {
            return Err(e::INVALID_PARAMETER);
        }
        let clash = self.list.iter().any(|c| {
            same_name(&c.name, &class.name)
                && !c.system
                && (c.instance == class.instance || c.global || class.global)
        });
        if clash {
            return Err(e::CLASS_ALREADY_EXISTS);
        }
        if self.list.len() >= MAX_CLASSES {
            return Err(e::NOT_ENOUGH_MEMORY);
        }
        class.atom = self.next_atom;
        self.next_atom = self.next_atom.wrapping_add(1).max(0xc000);
        class.windows = 0;
        self.list.push(class);
        Ok(self.list.last().unwrap().atom)
    }
    /// Lookup order: the instance's local classes, global application
    /// classes, then system classes.
    pub fn find(&self, r: ClassRef, instance: u64) -> Option<usize> {
        let hit = |c: &Class| match r {
            ClassRef::Atom(a) => c.atom == a,
            ClassRef::Name(n) => same_name(&c.name, n),
        };
        self.list
            .iter()
            .position(|c| hit(c) && !c.system && !c.global && c.instance == instance)
            .or_else(|| {
                self.list
                    .iter()
                    .position(|c| hit(c) && c.global && !c.system)
            })
            .or_else(|| self.list.iter().position(|c| hit(c) && c.system))
            .or_else(|| match r {
                // An atom names exactly one class whatever its scope.
                ClassRef::Atom(a) => self.list.iter().position(|c| c.atom == a),
                ClassRef::Name(_) => None,
            })
    }
    pub fn get(&self, i: usize) -> Option<&Class> {
        self.list.get(i)
    }
    pub fn get_mut(&mut self, i: usize) -> Option<&mut Class> {
        self.list.get_mut(i)
    }
    pub fn by_atom(&self, atom: u16) -> Option<usize> {
        self.list.iter().position(|c| c.atom == atom)
    }
    pub fn unregister(&mut self, r: ClassRef, instance: u64) -> Win32Result<()> {
        let i = self
            .find(r, instance)
            .filter(|&i| !self.list[i].system)
            .ok_or(e::CLASS_DOES_NOT_EXIST)?;
        if self.list[i].windows != 0 {
            return Err(e::CLASS_HAS_WINDOWS);
        }
        self.list.remove(i);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Life {
    Creating,
    Live,
    Destroying,
}

#[derive(Clone, Debug)]
pub struct Window {
    pub class: u16,
    pub style: u32,
    pub ex_style: u32,
    pub parent: Option<Handle>,
    pub owner: Option<Handle>,
    pub message_only: bool,
    pub children: Vec<Handle>,
    /// Window rectangle: parent-client coordinates for children, screen
    /// coordinates for top-level windows.
    pub rect: Rect,
    /// Client rectangle relative to the window rectangle's top-left.
    pub client: Rect,
    pub text: Vec<u16>,
    /// Child id, or the menu handle value of a top-level window.
    pub id: u64,
    pub instance: u64,
    pub procedure: u64,
    pub user_data: u64,
    pub extra: Vec<u8>,
    pub thread: u32,
    /// Update region in client coordinates.
    pub update: Region,
    pub erase: bool,
    pub frame_dirty: bool,
    pub life: Life,
    /// First WM_SIZE/WM_MOVE still owed (sent at the first show).
    pub need_size: bool,
    pub font: u64,
    pub native: Option<u64>,
}
impl Window {
    pub fn new(class: u16, style: u32, ex_style: u32, thread: u32) -> Self {
        Self {
            class,
            style,
            ex_style,
            parent: None,
            owner: None,
            message_only: false,
            children: Vec::new(),
            rect: Rect::EMPTY,
            client: Rect::EMPTY,
            text: Vec::new(),
            id: 0,
            instance: 0,
            procedure: 0,
            user_data: 0,
            extra: Vec::new(),
            thread,
            update: Region::new(),
            erase: false,
            frame_dirty: false,
            life: Life::Creating,
            need_size: false,
            font: 0,
            native: None,
        }
    }
    pub fn is_child(&self) -> bool {
        self.style & ws::CHILD != 0
    }
    pub fn visible(&self) -> bool {
        self.style & ws::VISIBLE != 0
    }
    pub fn enabled(&self) -> bool {
        self.style & ws::DISABLED == 0
    }
    pub fn client_size(&self) -> (i32, i32) {
        (self.client.width().max(0), self.client.height().max(0))
    }
}

pub struct Windows {
    table: Table<Window>,
    /// Top-level windows, topmost first.
    pub top: Vec<Handle>,
}

impl Default for Windows {
    fn default() -> Self {
        Self::new()
    }
}

impl Windows {
    pub fn new() -> Self {
        Self {
            table: Table::new(kind::WINDOW, MAX_WINDOWS),
            top: Vec::new(),
        }
    }
    pub fn len(&self) -> usize {
        self.table.len()
    }
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }
    pub fn resolve(&self, value: u64) -> Option<Handle> {
        self.table.resolve(value)
    }
    pub fn get(&self, h: Handle) -> Option<&Window> {
        self.table.get(h)
    }
    pub fn get_mut(&mut self, h: Handle) -> Option<&mut Window> {
        self.table.get_mut(h)
    }
    pub fn iter(&self) -> impl Iterator<Item = (Handle, &Window)> + '_ {
        self.table.iter()
    }
    /// Insert and link: children at the bottom of their siblings (creation
    /// order is z-order from the top), top-level windows on top.
    pub fn insert(&mut self, w: Window) -> Win32Result<Handle> {
        let parent = w.parent;
        let message_only = w.message_only;
        let h = self.table.insert(w).map_err(|_| e::NOT_ENOUGH_MEMORY)?;
        let linked = match parent {
            Some(p) => match self.table.get_mut(p) {
                Some(pw) => try_push(&mut pw.children, h, MAX_WINDOWS),
                None => Err(crate::Error::InvalidHandle),
            },
            None if message_only => Ok(()),
            None => self
                .top
                .try_reserve(1)
                .map(|_| self.top.insert(0, h))
                .map_err(|_| crate::Error::Quota),
        };
        if linked.is_err() {
            self.table.remove(h);
            return Err(e::NOT_ENOUGH_MEMORY);
        }
        Ok(h)
    }
    /// Unlink and free one window (its children must already be gone).
    pub fn remove(&mut self, h: Handle) -> Option<Window> {
        let w = self.table.remove(h)?;
        if let Some(p) = w.parent
            && let Some(pw) = self.table.get_mut(p)
        {
            pw.children.retain(|&c| c != h);
        }
        self.top.retain(|&t| t != h);
        Some(w)
    }
    pub fn parent(&self, h: Handle) -> Option<Handle> {
        self.get(h).and_then(|w| w.parent)
    }
    /// GA_ROOT: the top-level ancestor.
    pub fn root(&self, mut h: Handle) -> Handle {
        while let Some(p) = self.parent(h) {
            h = p;
        }
        h
    }
    pub fn is_descendant(&self, ancestor: Handle, mut h: Handle) -> bool {
        while let Some(p) = self.parent(h) {
            if p == ancestor {
                return true;
            }
            h = p;
        }
        false
    }
    /// Window and descendants, pre-order.
    pub fn subtree(&self, h: Handle) -> Vec<Handle> {
        let mut out = Vec::new();
        let mut stack = alloc::vec![h];
        while let Some(x) = stack.pop() {
            out.push(x);
            if let Some(w) = self.get(x) {
                stack.extend(w.children.iter().rev());
            }
        }
        out
    }
    /// GetDlgItem: a direct child with this id.
    pub fn child_by_id(&self, parent: Handle, id: u64) -> Option<Handle> {
        self.get(parent)?
            .children
            .iter()
            .copied()
            .find(|&c| self.get(c).is_some_and(|w| w.id == id))
    }
    /// Screen position of `h`'s client origin.
    pub fn client_origin(&self, h: Handle) -> Point {
        let mut p = Point::new(0, 0);
        let mut cur = Some(h);
        while let Some(x) = cur {
            let Some(w) = self.get(x) else { break };
            p = p.offset(w.rect.left + w.client.left, w.rect.top + w.client.top);
            cur = w.parent;
        }
        p
    }
    /// Window rectangle in screen coordinates.
    pub fn screen_rect(&self, h: Handle) -> Rect {
        let Some(w) = self.get(h) else {
            return Rect::EMPTY;
        };
        let base = match w.parent {
            Some(p) => self.client_origin(p),
            None => Point::new(0, 0),
        };
        w.rect.offset(base.x, base.y)
    }
    /// Where `h`'s client area (or whole window) lies on its top-level
    /// surface, whose top-left is `surface` in screen coordinates: the DC
    /// origin and the visible region (clipped to every ancestor's client
    /// area and to `clip`, given in the DC's own coordinates), both in
    /// surface pixels. Child windows share their top-level's surface, so
    /// this is how their paint lands at the right place. None while `h` or
    /// an ancestor is hidden.
    pub fn surface_target(
        &self,
        h: Handle,
        whole: bool,
        surface: Point,
        clip: Option<&Region>,
    ) -> Option<(Point, Region)> {
        if !self.really_visible(h) {
            return None;
        }
        let win = self.get(h)?;
        let (origin, area) = if whole {
            let screen = self.screen_rect(h);
            (screen.top_left(), screen)
        } else {
            let o = self.client_origin(h);
            let (cw, ch) = win.client_size();
            (o, Rect::from_size(o.x, o.y, cw, ch))
        };
        let mut visible = area;
        let mut cur = win.parent;
        while let Some(p) = cur {
            let pw = self.get(p)?;
            let o = self.client_origin(p);
            let (cw, ch) = pw.client_size();
            visible = visible.intersect(Rect::from_size(o.x, o.y, cw, ch));
            cur = pw.parent;
        }
        let origin = Point::new(origin.x - surface.x, origin.y - surface.y);
        let mut region = Region::from_rect(visible.offset(-surface.x, -surface.y));
        if let Some(c) = clip {
            let mut c = c.clone();
            c.offset(origin.x, origin.y);
            region = region.combine(&c, Op::And).ok()?;
        }
        Some((origin, region))
    }
    /// Visible together with every ancestor.
    pub fn really_visible(&self, h: Handle) -> bool {
        let mut cur = Some(h);
        while let Some(x) = cur {
            match self.get(x) {
                Some(w) if w.visible() => cur = w.parent,
                _ => return false,
            }
        }
        true
    }
    /// Top-level windows of one thread (EnumThreadWindows), topmost first.
    pub fn thread_windows(&self, thread: u32) -> Vec<Handle> {
        self.top
            .iter()
            .copied()
            .filter(|&h| self.get(h).is_some_and(|w| w.thread == thread))
            .collect()
    }
    pub fn raise(&mut self, h: Handle) {
        if let Some(p) = self.top.iter().position(|&t| t == h) {
            self.top.remove(p);
            self.top.insert(0, h);
        }
    }
    /// InvalidateRect: `area` in client coordinates (None = whole client).
    pub fn invalidate(&mut self, h: Handle, area: Option<Rect>, erase: bool) -> bool {
        let Some(w) = self.get(h) else {
            return false;
        };
        let (cw, ch) = w.client_size();
        let client = Rect::new(0, 0, cw, ch);
        let r = area.map_or(client, |a| a.normalized().intersect(client));
        let clip_children = w.style & ws::CLIPCHILDREN != 0;
        let children = w.children.clone();
        if !r.is_empty() {
            let w = self.get_mut(h).unwrap();
            if let Ok(u) = w.update.union_rect(r) {
                w.update = u;
            }
            w.erase |= erase;
        }
        if !clip_children {
            for c in children {
                let Some(cw) = self.get(c) else { continue };
                if !cw.visible() {
                    continue;
                }
                let origin = cw.rect.top_left().offset(cw.client.left, cw.client.top);
                let part = r.offset(-origin.x, -origin.y);
                if !part.is_empty() {
                    self.invalidate(c, Some(part), erase);
                }
                if let Some(cw) = self.get_mut(c)
                    && !cw.rect.intersect(r).is_empty()
                {
                    cw.frame_dirty = true;
                }
            }
        }
        true
    }
    /// ValidateRect (None = the whole update region).
    pub fn validate(&mut self, h: Handle, area: Option<Rect>) -> bool {
        let Some(w) = self.get_mut(h) else {
            return false;
        };
        match area {
            None => {
                w.update = Region::new();
                w.erase = false;
            }
            Some(a) => {
                if let Ok(u) = w.update.combine(&Region::from_rect(a), Op::Diff) {
                    w.update = u;
                }
                if w.update.is_empty() {
                    w.erase = false;
                }
            }
        }
        true
    }
    pub fn needs_paint(&self, h: Handle) -> bool {
        self.get(h)
            .is_some_and(|w| !w.update.is_empty() || w.frame_dirty)
    }
    /// The first window of `thread` needing WM_PAINT: parents before
    /// children, topmost windows first.
    pub fn paint_candidate(&self, thread: u32, accept: &dyn Fn(Handle) -> bool) -> Option<Handle> {
        for &t in &self.top {
            let Some(w) = self.get(t) else { continue };
            if w.thread != thread || !w.visible() {
                continue;
            }
            for x in self.subtree(t) {
                if self.really_visible(x) && self.needs_paint(x) && accept(x) {
                    return Some(x);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    fn class(name: &str, instance: u64) -> Class {
        Class {
            atom: 0,
            name: w(name),
            instance,
            global: false,
            system: false,
            style: 0,
            procedure: 0x1000,
            class_extra: Vec::new(),
            window_extra: 0,
            icon: 0,
            cursor: 0,
            background: 0,
            menu_name: 0,
            small_icon: 0,
            windows: 0,
        }
    }
    #[test]
    fn class_registration_scopes_and_errors() {
        let mut c = Classes::new();
        let mut system = class("Button", 0);
        system.system = true;
        c.register(system).unwrap();
        let atom = c.register(class("DoorsGuiSelftest", 0x140000000)).unwrap();
        assert!(atom >= 0xc000);
        assert_eq!(
            c.register(class("doorsguiselftest", 0x140000000)),
            Err(e::CLASS_ALREADY_EXISTS)
        );
        assert!(
            c.register(class("DoorsGuiSelftest", 0x180000000)).is_ok(),
            "other module"
        );
        assert_eq!(
            c.register(class("Button", 0x140000000)).map(|_| ()),
            Ok(()),
            "shadows system"
        );
        let i = c.find(ClassRef::Name(&w("BUTTON")), 0x150000000).unwrap();
        assert!(c.get(i).unwrap().system);
        assert_eq!(
            c.find(ClassRef::Atom(atom), 0),
            Some(c.by_atom(atom).unwrap())
        );
        let local = c
            .find(ClassRef::Name(&w("DoorsGuiSelftest")), 0x140000000)
            .unwrap();
        c.get_mut(local).unwrap().windows = 1;
        assert_eq!(
            c.unregister(ClassRef::Name(&w("DoorsGuiSelftest")), 0x140000000),
            Err(e::CLASS_HAS_WINDOWS)
        );
        assert_eq!(
            c.unregister(ClassRef::Name(&w("Missing")), 0x140000000),
            Err(e::CLASS_DOES_NOT_EXIST)
        );
        c.get_mut(local).unwrap().windows = 0;
        assert_eq!(
            c.unregister(ClassRef::Name(&w("DoorsGuiSelftest")), 0x140000000),
            Ok(())
        );
        assert_eq!(c.register(class("", 1)), Err(e::INVALID_PARAMETER));
    }
    fn tree() -> (Windows, Handle, [Handle; 3]) {
        let mut ws_ = Windows::new();
        let mut main = Window::new(0xc000, ws::OVERLAPPEDWINDOW | ws::VISIBLE, 0, 1);
        main.rect = Rect::new(48, 48, 528, 368);
        main.client = Rect::new(0, 52, 480, 320);
        let main = ws_.insert(main).unwrap();
        let mut kids = [main; 3];
        for (i, k) in kids.iter_mut().enumerate() {
            let mut c = Window::new(0xc001, ws::CHILD | ws::VISIBLE, 0, 1);
            c.parent = Some(main);
            c.id = 201 + i as u64;
            c.rect = Rect::from_size(12 + 100 * i as i32, 12, 50, 20);
            c.client = Rect::new(0, 0, 50, 20);
            *k = ws_.insert(c).unwrap();
        }
        (ws_, main, kids)
    }
    #[test]
    fn tree_relations_and_coordinates() {
        let (mut t, main, kids) = tree();
        assert_eq!(t.get(main).unwrap().children, kids, "creation order");
        assert_eq!(t.child_by_id(main, 202), Some(kids[1]));
        assert_eq!(t.root(kids[2]), main);
        assert!(t.is_descendant(main, kids[0]));
        assert_eq!(t.client_origin(main), Point::new(48, 100));
        assert_eq!(t.client_origin(kids[1]), Point::new(160, 112));
        assert_eq!(t.screen_rect(kids[0]), Rect::new(60, 112, 110, 132));
        assert_eq!(t.subtree(main), [main, kids[0], kids[1], kids[2]]);
        assert_eq!(t.thread_windows(1), [main]);
        assert!(t.really_visible(kids[0]));
        t.get_mut(main).unwrap().style &= !ws::VISIBLE;
        assert!(!t.really_visible(kids[0]));
        let gone = t.remove(kids[1]).unwrap();
        assert_eq!(gone.id, 202);
        assert_eq!(t.get(main).unwrap().children, [kids[0], kids[2]]);
        assert!(t.get(kids[1]).is_none());
        assert_eq!(t.resolve(kids[1].value()), None, "stale HWND");
    }
    #[test]
    fn invalidation_propagates_without_clipchildren() {
        let (mut t, main, kids) = tree();
        assert!(t.invalidate(main, Some(Rect::new(20, 30, 60, 70)), false));
        assert_eq!(
            t.get(main).unwrap().update.bounds(),
            Rect::new(20, 30, 60, 70)
        );
        assert!(t.needs_paint(kids[0]), "overlapping child invalidated too");
        assert!(!t.needs_paint(kids[2]));
        assert_eq!(t.paint_candidate(1, &|_| true), Some(main), "parent first");
        t.validate(main, None);
        assert!(t.get(main).unwrap().update.is_empty());
        assert_eq!(t.paint_candidate(1, &|_| true), Some(kids[0]));
        t.validate(kids[0], None);
        t.get_mut(kids[0]).unwrap().frame_dirty = false;
        assert_eq!(t.paint_candidate(1, &|_| true), None);
        t.get_mut(main).unwrap().style |= ws::CLIPCHILDREN;
        t.invalidate(main, None, true);
        assert!(!t.needs_paint(kids[0]));
        assert!(t.get(main).unwrap().erase);
        t.validate(main, Some(Rect::new(0, 0, 480, 268)));
        assert!(t.get(main).unwrap().update.is_empty());
        assert_eq!(t.paint_candidate(2, &|_| true), None, "other thread");
    }
}
