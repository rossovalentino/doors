//! Settings > Personalization > Context menu > "Customize": its own window,
//! kept simple. Choose the menu style (Windows 11's compact menu or the
//! classic Windows 10 menu) and, for the compact menu, what the three menus
//! people actually change list: Files, Folders and Empty space. Every other
//! menu (drives, the navigation pane, several items, Recycle Bin items) stays
//! as Windows builds it. Rows switch commands on or off, drag to reorder (or
//! into the icon row), and the preview beside them is the real menu, painted
//! by File Explorer's own painter. Changes are saved as they are made
//! (`.door-context-menu`; Explorer re-reads it).
use super::pages::{button, card, toggle};
use super::theme::{self, Theme};
use super::*;
use door_workspace::context_menu::{self as cm, Entry, Gates, MenuState};
use door_workspace::context_menu_profile::{
    self as cmp, Context, FIXED, Layout, MAX_ROW, Profile, Token, row_capable,
};

type R = door_workspace::settings_catalog::Rect;

/// Window size (logical px), also the Settings host's `WindowOptions`.
pub const WINDOW: (u32, u32) = (980, 760);
/// The menus the customizer edits; the others keep Windows' layout.
pub const EDITABLE: [Context; 3] = [Context::File, Context::Folder, Context::Background];
const CAPTION: i32 = 48;
const PAD: i32 = 24;
const COMMAND_ROW: i32 = 40;
const BREAK_ROW: i32 = 22;
const HEADER_ROW: i32 = 34;
const FOOTER: i32 = 64;
const PREVIEW_W: u32 = 360;

// Targets.
const T_TAB: u32 = 100;
const T_TOGGLE: u32 = 200;
const T_PLACE: u32 = 300;
const T_REMOVE: u32 = 400;
const T_ROW: u32 = 500;
const T_STYLE: u32 = 600;
const T_LABELS: u32 = 610;
const T_RESET: u32 = 700;
const T_DONE: u32 = 702;
const T_BREAK: u32 = 703;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    /// None for a divider.
    key: Option<String>,
    label: String,
    action: cm::Action,
    in_row: bool,
    hidden: bool,
}
impl Row {
    fn locked(&self) -> bool {
        self.key.as_deref() == Some(FIXED)
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Elem {
    Header(&'static str),
    Row(usize),
    AddBreak,
    /// The empty icon row's drop hint.
    Placeholder,
}
#[derive(Clone, Copy)]
struct Drag {
    index: usize,
    grab: i32,
    y: i32,
    moved: bool,
}

pub struct Customizer {
    uid: u32,
    controls: Controls,
    profile: Profile,
    context: Context,
    rows: Vec<Row>,
    width: u32,
    height: u32,
    hover: Option<(i32, i32)>,
    drag: Option<Drag>,
    scroll: i32,
    status: String,
    pub close: bool,
    /// A drop changed the layout; saved on the next tick.
    unsaved: bool,
    /// The menu style chosen here until the window server confirms it.
    pending_controls: Option<Controls>,
    /// The same request, taken once by the host to send.
    pub controls_request: Option<Controls>,
}

/// Gates that offer every situational command, so the editor and the
/// preview show all a menu can list (each real menu shows only what applies
/// at that moment).
fn sample_gates(context: Context) -> Gates {
    Gates {
        cut: None,
        copy: None,
        rename: None,
        delete: None,
        terminal: None,
        compress: None,
        new_window: None,
        paste: None,
        pin_reason: None,
        pinned: (context == Context::Folder).then_some(false),
        favorite: (context == Context::File).then_some(false),
        located: context == Context::File,
        empty_bin: None,
        ..Gates::default()
    }
}
fn tab_label(context: Context) -> &'static str {
    match context {
        Context::File => "Files",
        Context::Folder => "Folders",
        _ => "Empty space",
    }
}
/// Six-dot drag gripper centred on (x, y).
fn grip(v: &mut ui::View<'_>, color: [u8; 4], x: i32, y: i32) {
    for row in 0..3 {
        for col in 0..2 {
            v.rounded(x + col * 4, y - 5 + row * 4, 2, 2, 1, color);
        }
    }
}

impl Customizer {
    pub fn new(storage: &mut Storage, uid: u32, controls: Controls) -> Self {
        let mut c = Self::blank(uid, controls);
        c.profile = c.read(storage).unwrap_or_default();
        c.rebuild();
        c
    }
    fn blank(uid: u32, controls: Controls) -> Self {
        Self {
            uid,
            controls,
            profile: Profile::default(),
            context: Context::File,
            rows: Vec::new(),
            width: WINDOW.0,
            height: WINDOW.1,
            hover: None,
            drag: None,
            scroll: 0,
            status: String::new(),
            close: false,
            unsaved: false,
            pending_controls: None,
            controls_request: None,
        }
    }
    fn path(&self) -> String {
        format!("/users/{}/{}", self.uid, cmp::FILE)
    }
    fn read(&self, storage: &mut Storage) -> Option<Profile> {
        use door_service_client::{Disposition, READ, SHARE_ALL};
        let file = storage
            .open(&self.path(), Disposition::OpenExisting, READ, SHARE_ALL)
            .ok()?;
        let mut bytes = Vec::new();
        let mut chunk = [0; door_service_protocol::CHUNK_BYTES];
        let read = loop {
            match storage.read(&file, bytes.len() as u64, &mut chunk) {
                Ok(0) => break true,
                Ok(n) if bytes.len() + n <= cmp::MAX_BYTES => bytes.extend_from_slice(&chunk[..n]),
                _ => break false,
            }
        };
        let _ = storage.close(file);
        read.then(|| Profile::decode(&bytes).ok()).flatten()
    }
    fn save(&mut self, storage: &mut Storage) {
        self.status = match super::write_whole_file(storage, &self.path(), &self.profile.encode()) {
            Ok(()) => String::new(),
            Err(_) => "Couldn't save your changes. Try again.".into(),
        };
    }
    pub fn title(&self) -> &str {
        "Customize context menu"
    }
    pub fn configure(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        self.scroll = self.scroll.min(self.max_scroll());
    }
    /// The desktop controls changed (the menu style, from here or Settings).
    pub fn controls_changed(&mut self, controls: Controls) {
        self.controls = controls;
        if self.pending_controls == Some(controls) {
            self.pending_controls = None;
        }
        self.rebuild();
    }
    fn classic(&self) -> bool {
        self.pending_controls
            .unwrap_or(self.controls)
            .classic_context_menu()
    }
    /// The compact menu Explorer builds for the tab (Open with included).
    fn menu(&self, context: Context) -> cm::Menu {
        let mut menu = cm::modern(context.subject(), sample_gates(context), self.controls);
        if context == Context::File {
            cm::insert_open_with(&mut menu, cm::open_with(&[]));
        }
        menu
    }
    /// This menu's commands in the user's order, hidden ones included.
    fn rebuild(&mut self) {
        let subject = self.context.subject();
        let mut full = self.profile.clone();
        full.layout_mut(self.context).hidden.clear();
        let mut menu = self.menu(self.context);
        full.apply(subject, &mut menu);
        let layout = self.profile.layout(self.context);
        let row = |e: &Entry, in_row: bool| {
            let key = if e.separator {
                None
            } else {
                cmp::key(e).map(String::from)
            };
            Row {
                hidden: key.as_deref().is_some_and(|k| layout.is_hidden(k)),
                key,
                label: e.label.clone(),
                action: e.action,
                in_row,
            }
        };
        let mut rows: Vec<Row> = menu.icons.iter().map(|e| row(e, true)).collect();
        rows.extend(
            menu.entries
                .iter()
                .filter(|e| e.separator || cmp::key(e).is_some())
                .map(|e| row(e, false)),
        );
        self.rows = rows;
    }
    fn commit(&mut self, storage: &mut Storage) {
        self.commit_layout();
        self.save(storage);
    }
    /// Saves a layout a drop changed (`pointer_up` has no storage).
    pub fn tick(&mut self, storage: &mut Storage) -> bool {
        if core::mem::take(&mut self.unsaved) {
            self.save(storage);
            return true;
        }
        false
    }
    /// The layout the editor rows describe; Windows' default when equal.
    fn commit_layout(&mut self) {
        let layout = Layout {
            order: self
                .rows
                .iter()
                .filter(|r| !r.in_row)
                .map(|r| match &r.key {
                    Some(k) => Token::Item(k.clone()),
                    None => Token::Separator,
                })
                .collect(),
            hidden: self
                .rows
                .iter()
                .filter(|r| r.hidden && !r.locked())
                .filter_map(|r| r.key.clone())
                .collect(),
            row: Some(
                self.rows
                    .iter()
                    .filter(|r| r.in_row)
                    .filter_map(|r| r.key.clone())
                    .collect(),
            ),
        };
        let edited = self.rows.clone();
        *self.profile.layout_mut(self.context) = Layout::default();
        self.rebuild();
        let default = core::mem::take(&mut self.rows);
        if edited != default {
            *self.profile.layout_mut(self.context) = layout;
        }
        self.rows = edited;
    }

    // ---- Geometry ----

    fn inner_w(&self) -> u32 {
        self.width.saturating_sub(2 * PAD as u32)
    }
    fn style_card(&self) -> R {
        R::new(PAD, CAPTION + 8, self.inner_w(), 68)
    }
    fn labels_card(&self) -> R {
        let s = self.style_card();
        R::new(PAD, s.bottom() + 4, self.inner_w(), 52)
    }
    fn style_segments(&self) -> [R; 2] {
        let s = self.style_card();
        let w = 116;
        let x = s.right() - 16 - 2 * w as i32;
        [
            R::new(x, s.center_y() - 16, w, 32),
            R::new(x + w as i32, s.center_y() - 16, w, 32),
        ]
    }
    fn labels_switch(&self) -> R {
        let r = self.labels_card();
        R::new(r.right() - 60, r.center_y() - 10, 40, 20)
    }
    fn tabs_top(&self) -> i32 {
        self.labels_card().bottom() + 22
    }
    /// SelectorBar items (fixed widths keep hit tests independent of fonts).
    fn tab(&self, i: usize) -> R {
        let widths = [64, 82, 112];
        let x = PAD + widths[..i].iter().sum::<i32>() + i as i32 * 8;
        R::new(x, self.tabs_top(), widths[i] as u32, 36)
    }
    fn preview_x(&self) -> i32 {
        self.width as i32 - PAD - PREVIEW_W as i32
    }
    fn middle(&self) -> R {
        let top = self.tabs_top() + 48;
        R::new(
            PAD,
            top,
            (self.preview_x() - 24 - PAD).max(200) as u32,
            (self.height as i32 - FOOTER - 12 - top).max(40) as u32,
        )
    }
    fn preview_area(&self) -> R {
        let m = self.middle();
        R::new(self.preview_x(), m.y, PREVIEW_W, m.h)
    }
    /// The command list's elements and their content-space rectangles.
    fn elems(&self) -> Vec<(Elem, R)> {
        let m = self.middle();
        let mut out = Vec::new();
        let mut y = m.y;
        let mut push = |e: Elem, h: i32, out: &mut Vec<(Elem, R)>| {
            out.push((e, R::new(m.x, y, m.w, h as u32)));
            y += h;
        };
        push(Elem::Header("Icon row"), HEADER_ROW, &mut out);
        if !self.rows.iter().any(|r| r.in_row) {
            push(Elem::Placeholder, COMMAND_ROW, &mut out);
        }
        let mut menu_header = false;
        for (i, r) in self.rows.iter().enumerate() {
            if !r.in_row && !menu_header {
                menu_header = true;
                push(Elem::Header("Menu"), HEADER_ROW + 12, &mut out);
            }
            push(
                Elem::Row(i),
                if r.key.is_none() {
                    BREAK_ROW
                } else {
                    COMMAND_ROW
                },
                &mut out,
            );
        }
        if !menu_header {
            push(Elem::Header("Menu"), HEADER_ROW + 12, &mut out);
        }
        push(Elem::AddBreak, COMMAND_ROW + 8, &mut out);
        out
    }
    fn max_scroll(&self) -> i32 {
        let m = self.middle();
        let end = self.elems().last().map_or(m.y, |(_, r)| r.bottom());
        (end - m.bottom()).max(0)
    }
    fn footer_buttons(&self) -> [(u32, R); 2] {
        let y = self.height as i32 - FOOTER + 16;
        [
            (T_RESET, R::new(PAD, y, 150, 32)),
            (T_DONE, R::new(self.width as i32 - PAD - 120, y, 120, 32)),
        ]
    }
    /// A command row's switch, placement button and remove button.
    fn row_controls(r: R) -> (R, R, R) {
        (
            R::new(r.right() - 56, r.center_y() - 10, 40, 20),
            R::new(r.right() - 100, r.center_y() - 14, 32, 28),
            R::new(r.right() - 36, r.center_y() - 11, 22, 22),
        )
    }

    // ---- Input ----

    pub fn target(&self, x: i32, y: i32) -> Option<u32> {
        for (i, s) in self.style_segments().into_iter().enumerate() {
            if s.contains(x, y) {
                return Some(T_STYLE + i as u32);
            }
        }
        for (id, r) in self.footer_buttons() {
            if r.contains(x, y) {
                return Some(id);
            }
        }
        if self.classic() {
            return None;
        }
        if self.labels_card().contains(x, y) {
            return Some(T_LABELS);
        }
        for i in 0..EDITABLE.len() {
            if self.tab(i).contains(x, y) {
                return Some(T_TAB + i as u32);
            }
        }
        let m = self.middle();
        if !m.contains(x, y) {
            return None;
        }
        let cy = y + self.scroll;
        for (e, r) in self.elems() {
            if !r.contains(x, cy) {
                continue;
            }
            return match e {
                Elem::AddBreak => R::new(r.x, r.y + 8, 180, 30)
                    .contains(x, cy)
                    .then_some(T_BREAK),
                Elem::Row(i) => {
                    let (switch, place, remove) = Self::row_controls(r);
                    let row = &self.rows[i];
                    if row.key.is_none() {
                        if remove.contains(x, cy) {
                            return Some(T_REMOVE + i as u32);
                        }
                    } else if !row.locked() {
                        if switch.contains(x, cy) {
                            return Some(T_TOGGLE + i as u32);
                        }
                        if row.key.as_deref().is_some_and(row_capable) && place.contains(x, cy) {
                            return Some(T_PLACE + i as u32);
                        }
                    }
                    (!row.locked()).then_some(T_ROW + i as u32)
                }
                Elem::Header(_) | Elem::Placeholder => None,
            };
        }
        None
    }
    pub fn click(&mut self, x: i32, y: i32, storage: &mut Storage) -> Result<bool, Error> {
        let Some(id) = self.target(x, y) else {
            return Ok(false);
        };
        match id {
            T_DONE => self.close = true,
            T_RESET => {
                self.profile = Profile::default();
                self.set_classic(false);
                self.rebuild();
                self.save(storage);
            }
            T_STYLE => self.set_classic(false),
            id if id == T_STYLE + 1 => self.set_classic(true),
            T_LABELS => {
                self.profile.look.labels = !self.profile.look.labels;
                self.save(storage);
            }
            T_BREAK => {
                let at = self
                    .rows
                    .iter()
                    .position(Row::locked)
                    .unwrap_or(self.rows.len());
                self.rows.insert(
                    at,
                    Row {
                        key: None,
                        label: String::new(),
                        action: cm::Action::Submenu,
                        in_row: false,
                        hidden: false,
                    },
                );
                self.commit(storage);
                self.scroll = self.max_scroll();
            }
            id if (T_TAB..T_TAB + 3).contains(&id) => {
                self.context = EDITABLE[(id - T_TAB) as usize];
                self.scroll = 0;
                self.rebuild();
            }
            id if (T_TOGGLE..T_TOGGLE + 100).contains(&id) => {
                let i = (id - T_TOGGLE) as usize;
                self.rows[i].hidden = !self.rows[i].hidden;
                self.commit(storage);
            }
            id if (T_PLACE..T_PLACE + 100).contains(&id) => {
                let i = (id - T_PLACE) as usize;
                let to_row = !self.rows[i].in_row;
                if to_row && self.rows.iter().filter(|r| r.in_row).count() >= MAX_ROW {
                    self.status = format!("The icon row holds up to {MAX_ROW} buttons.");
                } else {
                    let mut r = self.rows.remove(i);
                    r.in_row = to_row;
                    // Joining the row: its last button; leaving it: the
                    // menu's first command.
                    let at = self.rows.iter().filter(|r| r.in_row).count();
                    self.rows.insert(at, r);
                    self.commit(storage);
                }
            }
            id if (T_REMOVE..T_REMOVE + 100).contains(&id) => {
                self.rows.remove((id - T_REMOVE) as usize);
                self.commit(storage);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn set_classic(&mut self, on: bool) {
        let mut bits = self.controls.bits();
        if on {
            bits |= Controls::CLASSIC_CONTEXT_MENU;
        } else {
            bits &= !Controls::CLASSIC_CONTEXT_MENU;
        }
        self.pending_controls = Controls::from_bits(bits).filter(|c| *c != self.controls);
        self.controls_request = self.pending_controls;
    }
    pub fn key(&mut self, code: u32, _storage: &mut Storage) -> Result<bool, Error> {
        match code & 511 {
            // Esc and Enter close the window, as Done does.
            1 | 28 => {
                self.close = true;
                Ok(true)
            }
            // Left/Right switch between Files, Folders and Empty space.
            0x14b | 0x14d if !self.classic() => {
                let i = EDITABLE
                    .iter()
                    .position(|c| *c == self.context)
                    .unwrap_or(0) as i32
                    + if code & 511 == 0x14b { -1 } else { 1 };
                self.context = EDITABLE[i.rem_euclid(EDITABLE.len() as i32) as usize];
                self.scroll = 0;
                self.rebuild();
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    pub fn pointer_down(&mut self, x: i32, y: i32) -> bool {
        let Some(id) = self.target(x, y) else {
            return false;
        };
        if !(T_ROW..T_ROW + 100).contains(&id) {
            return false;
        }
        let i = (id - T_ROW) as usize;
        let row_y = self
            .elems()
            .iter()
            .find_map(|(e, r)| (*e == Elem::Row(i)).then_some(r.y))
            .unwrap_or(y);
        self.drag = Some(Drag {
            index: i,
            grab: y + self.scroll - row_y,
            y,
            moved: false,
        });
        true
    }
    pub fn pointer_motion(&mut self, x: i32, y: i32) -> bool {
        let before = (self.hover_target(), self.hover_row());
        self.hover = Some((x, y));
        if let Some(d) = self.drag.as_mut() {
            if (y - d.y).abs() > 3 {
                d.moved = true;
            }
            d.y = y;
            return true;
        }
        before != (self.hover_target(), self.hover_row())
    }
    fn hover_target(&self) -> Option<u32> {
        self.hover.and_then(|(x, y)| self.target(x, y))
    }
    /// The list row under the pointer (its hover-only controls show).
    fn hover_row(&self) -> Option<usize> {
        let (x, y) = self.hover?;
        if self.classic() || !self.middle().contains(x, y) {
            return None;
        }
        let cy = y + self.scroll;
        self.elems().into_iter().find_map(|(e, r)| match e {
            Elem::Row(i) if r.contains(x, cy) => Some(i),
            _ => None,
        })
    }
    /// Where a dragged row would land: its index in `rows` after removal,
    /// and whether it joins the icon row.
    fn drop_slot(&self, d: Drag) -> (usize, bool) {
        let cy = d.y + self.scroll;
        let mut slot = 0;
        let mut in_row = true;
        for (e, r) in self.elems() {
            match e {
                Elem::Header("Menu") if cy >= r.y => in_row = false,
                Elem::Row(j) if j != d.index && cy > r.center_y() => slot += 1,
                _ => (),
            }
        }
        (slot, in_row)
    }
    pub fn release_pending(&mut self) {
        let Some(d) = self.drag.take() else {
            return;
        };
        if !d.moved {
            return;
        }
        let (slot, in_row) = self.drop_slot(d);
        let mut row = self.rows.remove(d.index);
        let capable = row.key.as_deref().is_some_and(row_capable);
        row.in_row = in_row && capable;
        if row.in_row && self.rows.iter().filter(|r| r.in_row).count() >= MAX_ROW {
            row.in_row = false;
        }
        let rows_in = self.rows.iter().filter(|r| r.in_row).count();
        let slot = if row.in_row {
            slot.min(rows_in)
        } else {
            slot.max(rows_in)
        };
        // Nothing goes below the fixed tail.
        let limit = self
            .rows
            .iter()
            .position(Row::locked)
            .unwrap_or(self.rows.len());
        self.rows.insert(slot.min(limit), row);
        self.commit_layout();
        self.unsaved = true;
    }
    pub fn wheel(&mut self, x: i32, y: i32, notches: i32) -> bool {
        if !self.middle().contains(x, y) {
            return false;
        }
        let next = (self.scroll - notches * 48).clamp(0, self.max_scroll());
        let changed = next != self.scroll;
        self.scroll = next;
        changed
    }

    // ---- Paint ----

    pub fn draw(&self, v: &mut ui::View<'_>) {
        let t = theme::theme(v.palette.dark);
        let (w, h) = (self.width, self.height);
        v.rect(0, 0, w, h, t.bg);
        let hot = self.hover_target();
        let pressed = v.pressed.and_then(|(x, y)| self.target(x, y));
        // Caption.
        v.sized_icon(16, (CAPTION - 16) / 2, 16, Icon::Settings);
        v.text_clip_centered(
            44,
            (0, CAPTION as u32),
            w.saturating_sub(200),
            self.title(),
            TextStyle::Regular12,
            t.text,
        );
        self.draw_style(v, &t, hot, pressed);
        if self.classic() {
            let top = self.tabs_top();
            let m = self.middle();
            v.text(PAD, top + 4, "Classic menu", TextStyle::Semibold14, t.text);
            v.text_clip(
                PAD,
                top + 28,
                m.w,
                "Every command in one list, as in Doors 10.",
                TextStyle::Regular12,
                t.secondary,
            );
            v.text_clip(
                PAD,
                top + 46,
                m.w,
                "Switch to Doors 11 to choose what the compact menu shows.",
                TextStyle::Regular12,
                t.secondary,
            );
        } else {
            self.draw_tabs(v, &t, hot);
            self.draw_commands(v, &t, hot, pressed);
        }
        self.draw_preview(v);
        // Footer.
        let fy = h as i32 - FOOTER;
        v.rect(0, fy, w, 1, t.divider);
        for (id, r) in self.footer_buttons() {
            if id == T_DONE {
                v.rounded(
                    r.x,
                    r.y,
                    r.w,
                    r.h,
                    4,
                    if hot == Some(id) {
                        t.accent_hover
                    } else {
                        t.accent
                    },
                );
                v.text_in_box(
                    (r.x, r.y, r.w, r.h),
                    "Done",
                    TextStyle::Regular14,
                    t.on_accent,
                );
            } else {
                button(
                    v,
                    &t,
                    r,
                    "Restore defaults",
                    hot == Some(id),
                    pressed == Some(id),
                );
            }
        }
        if !self.status.is_empty() {
            v.text_clip_centered(
                PAD + 166,
                (fy, FOOTER as u32),
                w.saturating_sub(PAD as u32 * 2 + 300),
                &self.status,
                TextStyle::Regular12,
                t.secondary,
            );
        }
    }
    fn draw_style(&self, v: &mut ui::View<'_>, t: &Theme, hot: Option<u32>, pressed: Option<u32>) {
        let s = self.style_card();
        card(v, t, s, 6, t.card);
        if let Some(px) =
            super::menu_paint::command_pixels("show-more", t.dark, v.scale.clamp(1, 3))
        {
            v.color_pixels(s.x + 20, s.center_y() - 8, 16, px);
        }
        v.text(
            s.x + 56,
            s.y + 14,
            "Menu style",
            TextStyle::Regular14,
            t.text,
        );
        v.text(
            s.x + 56,
            s.y + 36,
            "The compact Doors 11 menu or the classic Doors 10 menu",
            TextStyle::Regular12,
            t.secondary,
        );
        // Segmented control.
        let seg = self.style_segments();
        let all = R::new(seg[0].x, seg[0].y, seg[0].w + seg[1].w, seg[0].h);
        v.rounded(all.x, all.y, all.w, all.h, 5, t.control_border);
        v.rounded(all.x + 1, all.y + 1, all.w - 2, all.h - 2, 4, t.control);
        let classic = self.classic();
        for (i, r) in seg.iter().enumerate() {
            let on = (i == 1) == classic;
            let id = T_STYLE + i as u32;
            if on {
                v.rounded(r.x + 3, r.y + 3, r.w - 6, r.h - 6, 3, t.accent);
            } else if hot == Some(id) {
                v.rounded(
                    r.x + 3,
                    r.y + 3,
                    r.w - 6,
                    r.h - 6,
                    3,
                    if pressed == Some(id) {
                        t.control_pressed
                    } else {
                        t.control_hover
                    },
                );
            }
            v.text_in_box(
                (r.x, r.y, r.w, r.h),
                if i == 0 { "Doors 11" } else { "Classic" },
                TextStyle::Regular14,
                if on { t.on_accent } else { t.text },
            );
        }
        // Names under the icon row (compact menu only).
        let l = self.labels_card();
        card(v, t, l, 6, t.card);
        let enabled = !classic;
        if enabled && hot == Some(T_LABELS) {
            v.rounded(l.x + 1, l.y + 1, l.w - 2, l.h - 2, 5, t.card_hover);
        }
        match super::menu_paint::command_pixels("rename", t.dark, v.scale.clamp(1, 3)) {
            Some(px) if enabled => v.color_pixels(l.x + 20, l.center_y() - 8, 16, px),
            _ => v.mask_pixels(
                l.x + 20,
                l.center_y() - 8,
                16,
                Icon::Rename.mask_scaled(16, v.scale),
                t.disabled,
                None,
            ),
        }
        v.text_clip_centered(
            l.x + 56,
            (l.y, l.h),
            l.w.saturating_sub(140),
            "Show names under the icons at the top",
            TextStyle::Regular14,
            if enabled { t.text } else { t.disabled },
        );
        toggle(
            v,
            t,
            self.labels_switch(),
            self.profile.look.labels,
            enabled,
            hot == Some(T_LABELS),
            pressed == Some(T_LABELS),
        );
    }
    /// WinUI SelectorBar: text tabs, the chosen one underlined in accent.
    fn draw_tabs(&self, v: &mut ui::View<'_>, t: &Theme, hot: Option<u32>) {
        for (i, context) in EDITABLE.iter().enumerate() {
            let r = self.tab(i);
            let id = T_TAB + i as u32;
            let on = *context == self.context;
            if hot == Some(id) && !on {
                v.rounded(r.x, r.y, r.w, r.h, 4, t.rail_hover);
            }
            v.text_in_box(
                (r.x, r.y, r.w, r.h - 4),
                tab_label(*context),
                if on {
                    TextStyle::Semibold14
                } else {
                    TextStyle::Regular14
                },
                if on { t.text } else { t.secondary },
            );
            if on {
                let lw = 16;
                v.rounded(
                    r.x + (r.w as i32 - lw) / 2,
                    r.bottom() - 4,
                    lw as u32,
                    3,
                    1,
                    t.accent,
                );
            }
        }
    }
    fn draw_commands(
        &self,
        v: &mut ui::View<'_>,
        t: &Theme,
        hot: Option<u32>,
        pressed: Option<u32>,
    ) {
        let m = self.middle();
        if let Ok(clip) = door_graphics_core::Rect::new(m.x - 4, m.y, m.w + 8, m.h) {
            v.viewport(clip, self.scroll);
        }
        let dragging = self.drag.filter(|d| d.moved);
        let slot = dragging.map(|d| self.drop_slot(d));
        let elems = self.elems();
        // One list card per section (Settings' grouped rows).
        let mut section: Option<R> = None;
        let mut cards = Vec::new();
        for (e, r) in &elems {
            match e {
                Elem::Row(i) if dragging.is_none_or(|d| d.index != *i) => {
                    section = Some(match section {
                        Some(s) => R::new(s.x, s.y, s.w, (r.bottom() - s.y) as u32),
                        None => *r,
                    });
                }
                Elem::Row(_) => (),
                _ => {
                    if let Some(s) = section.take() {
                        cards.push(s);
                    }
                }
            }
        }
        if let Some(s) = section {
            cards.push(s);
        }
        for c in &cards {
            card(v, t, *c, 6, t.card);
        }
        let hover_row = self.hover_row().filter(|_| dragging.is_none());
        let mut seen = 0usize;
        let mut marker: Option<i32> = None;
        let mut last_bottom = m.y;
        for (e, r) in &elems {
            let r = *r;
            match *e {
                Elem::Header(title) => {
                    v.text(
                        r.x + 2,
                        r.bottom() - 26,
                        title,
                        TextStyle::Semibold14,
                        t.text,
                    );
                }
                Elem::Placeholder => {
                    let b = R::new(r.x, r.y + 2, r.w, r.h - 4);
                    v.rounded(b.x, b.y, b.w, b.h, 6, t.card_border);
                    v.rounded(b.x + 1, b.y + 1, b.w - 2, b.h - 2, 5, t.bg);
                    v.text_in_box(
                        (b.x, b.y, b.w, b.h),
                        "Drag a command here",
                        TextStyle::Regular12,
                        t.secondary,
                    );
                }
                Elem::AddBreak => {
                    let b = R::new(r.x, r.y + 8, 180, 30);
                    if hot == Some(T_BREAK) {
                        v.rounded(b.x, b.y, b.w, b.h, 4, t.rail_hover);
                    }
                    v.mask_pixels(
                        b.x + 8,
                        b.y + 7,
                        16,
                        Icon::Add.mask_scaled(16, v.scale),
                        t.accent,
                        None,
                    );
                    v.text_clip_centered(
                        b.x + 32,
                        (b.y, b.h),
                        b.w - 36,
                        "Add a divider",
                        TextStyle::Regular14,
                        t.text,
                    );
                }
                Elem::Row(i) => {
                    if let (Some(d), Some((at, _))) = (dragging, slot) {
                        if i == d.index {
                            continue;
                        }
                        if seen == at && marker.is_none() {
                            marker = Some(r.y);
                        }
                        seen += 1;
                        last_bottom = r.bottom();
                    }
                    let first = cards.iter().any(|c| c.y == r.y);
                    self.draw_row(v, t, i, r, hot, pressed, hover_row == Some(i), first);
                }
            }
        }
        if let (Some(_), Some((at, _))) = (dragging, slot) {
            let y = marker.unwrap_or(if at >= seen { last_bottom } else { m.y });
            v.rounded(m.x + 6, y - 1, m.w - 12, 3, 1, t.accent);
        }
        v.reset_viewport();
        // The dragged row floats under the pointer, lifted.
        if let Some(d) = dragging {
            let h = if self.rows[d.index].key.is_none() {
                BREAK_ROW
            } else {
                COMMAND_ROW
            };
            let r = R::new(m.x, d.y - d.grab, m.w, h as u32);
            v.elevation(r.x, r.y + 4, r.w, r.h, if t.dark { 120 } else { 70 });
            card(v, t, r, 6, t.card_hover);
            self.draw_row(v, t, d.index, r, None, None, true, true);
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn draw_row(
        &self,
        v: &mut ui::View<'_>,
        t: &Theme,
        i: usize,
        r: R,
        hot: Option<u32>,
        pressed: Option<u32>,
        hovered: bool,
        first: bool,
    ) {
        let row = &self.rows[i];
        let (switch, place, remove) = Self::row_controls(r);
        if !first {
            v.rect(r.x + 1, r.y, r.w - 2, 1, t.divider);
        }
        if hovered && hot == Some(T_ROW + i as u32) {
            v.rect(r.x + 1, r.y + 1, r.w - 2, r.h - 2, t.card_hover);
        }
        let grip_ink = if hovered {
            t.secondary
        } else {
            t.strong_disabled
        };
        if row.key.is_none() {
            // A divider: a quiet rule; its remove button shows on hover.
            grip(v, grip_ink, r.x + 12, r.center_y());
            let right = r.right() - if hovered { 44 } else { 16 };
            v.rect(
                r.x + 30,
                r.center_y(),
                (right - r.x - 30).max(0) as u32,
                1,
                t.card_border,
            );
            if hovered {
                let id = T_REMOVE + i as u32;
                if hot == Some(id) {
                    v.rounded(remove.x, remove.y, remove.w, remove.h, 4, t.control_hover);
                }
                v.mask_pixels(
                    remove.x + 5,
                    remove.y + 5,
                    12,
                    Icon::Close.mask_scaled(12, v.scale),
                    t.secondary,
                    None,
                );
            }
            return;
        }
        let ink = if row.hidden { t.disabled } else { t.text };
        if !row.locked() {
            grip(v, grip_ink, r.x + 12, r.center_y());
        }
        // The command's own 26H2 icon (a switched-off command greys out).
        let colour = (!row.hidden)
            .then(|| super::menu_paint::command_key(row.action, &row.label))
            .flatten()
            .and_then(|k| super::menu_paint::command_pixels(k, t.dark, v.scale.clamp(1, 3)));
        match colour {
            Some(px) => v.color_pixels(r.x + 30, r.center_y() - 8, 16, px),
            None => {
                if let Some(icon) = super::menu_paint::action_icon(row.action, &row.label) {
                    v.mask_pixels(
                        r.x + 30,
                        r.center_y() - 8,
                        16,
                        icon.mask_scaled(16, v.scale),
                        ink,
                        None,
                    );
                }
            }
        }
        let label_w = r.w.saturating_sub(if row.locked() { 70 } else { 170 });
        v.text_clip_centered(
            r.x + 58,
            (r.y, r.h),
            label_w,
            &super::pages::fitted(v, &row.label, label_w, TextStyle::Regular14),
            TextStyle::Regular14,
            ink,
        );
        if row.locked() {
            v.text_clip_centered(
                r.right() - 120,
                (r.y, r.h),
                106,
                "Always shown",
                TextStyle::Regular12,
                t.secondary,
            );
            return;
        }
        // Move between the icon row and the menu (shown on hover).
        if hovered && row.key.as_deref().is_some_and(row_capable) {
            let id = T_PLACE + i as u32;
            if hot == Some(id) || pressed == Some(id) {
                v.rounded(
                    place.x,
                    place.y,
                    place.w,
                    place.h,
                    4,
                    if pressed == Some(id) {
                        t.control_pressed
                    } else {
                        t.control_hover
                    },
                );
            }
            let glyph = if row.in_row { Icon::Chevron } else { Icon::Up };
            v.mask_pixels(
                place.x + 8,
                place.y + 6,
                16,
                glyph.mask_scaled(16, v.scale),
                t.secondary,
                None,
            );
        }
        let id = T_TOGGLE + i as u32;
        toggle(
            v,
            t,
            switch,
            !row.hidden,
            true,
            hot == Some(id),
            pressed == Some(id),
        );
    }
    /// The menu the preview shows: the real compact (or classic) menu for the
    /// tab, as Explorer builds it for a text file, a folder or empty space.
    fn preview_menu(&self) -> cm::Menu {
        let subject = self.context.subject();
        if self.classic() {
            let mut menu = cm::classic(subject, sample_gates(self.context));
            menu.look.win10 = true;
            return menu;
        }
        let mut menu = cm::modern(subject, sample_gates(self.context), self.controls);
        if self.context == Context::File {
            use door_service_protocol::shell::icon;
            let apps = [
                cm::OpenWithApp {
                    name: "Preview".into(),
                    icon: icon::PREVIEW,
                    unavailable: None,
                },
                cm::OpenWithApp {
                    name: "Notes".into(),
                    icon: icon::NOTES,
                    unavailable: None,
                },
            ];
            cm::insert_open_with(&mut menu, cm::open_with(&apps));
            if let Some(open) = menu
                .entries
                .iter_mut()
                .find(|e| e.action == cm::Action::Open)
            {
                open.glyph = cm::Glyph::App(icon::NOTES);
            }
        }
        self.profile.apply(subject, &mut menu);
        // Commands look as they do when available.
        for e in menu.icons.iter_mut().chain(menu.entries.iter_mut()) {
            e.enabled = true;
            e.reason = "";
        }
        menu
    }
    /// The preview is the menu itself, centred in its column, nothing else.
    fn draw_preview(&self, v: &mut ui::View<'_>) {
        let area = self.preview_area();
        let menu = self.preview_menu();
        let width = (cm::grown_width(menu.width) as i32).min(area.w as i32);
        let state = MenuState::open(
            menu,
            (area.x + (area.w as i32 - width) / 2, 0),
            (area.right() as u32, area.h),
            false,
        );
        let root = state.root();
        if let Ok(clip) = door_graphics_core::Rect::new(area.x - 8, area.y, area.w + 16, area.h) {
            v.viewport(clip, -area.y);
        }
        let entries = state.menu.entries.clone();
        let apps = |v: &mut ui::View<'_>, glyph: cm::Glyph, (x, y): (i32, i32), _: [u8; 4]| {
            use door_service_protocol::shell::icon;
            match glyph {
                cm::Glyph::App(icon::NOTES) => v.sized_icon(x, y, 16, Icon::Notepad),
                cm::Glyph::App(icon::PREVIEW) => {
                    let pixels: &[u8] = match v.scale {
                        1 => door_workspace::asset!("../../../../assets/branding/preview.16.bgra"),
                        2 => door_workspace::asset!("../../../../assets/branding/preview.32.bgra"),
                        _ => door_workspace::asset!("../../../../assets/branding/preview.48.bgra"),
                    };
                    v.color_pixels(x, y, 16, pixels);
                }
                _ => return false,
            }
            true
        };
        super::menu_paint::draw_panel(v, &state, &root, &entries, None, None, None, &apps);
        v.reset_viewport();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn customizer() -> Customizer {
        let mut c = Customizer::blank(1001, Controls::DEFAULT);
        c.rebuild();
        c
    }
    #[test]
    fn each_tab_lists_its_own_commands_and_the_fixed_tail() {
        let mut c = customizer();
        for context in EDITABLE {
            c.context = context;
            c.rebuild();
            assert!(!c.rows.is_empty(), "{context:?}");
            assert!(
                c.rows.last().unwrap().locked(),
                "{context:?} ends with Customize menu"
            );
        }
        c.context = Context::Background;
        c.rebuild();
        let keys: Vec<_> = c.rows.iter().filter_map(|r| r.key.as_deref()).collect();
        assert!(keys.contains(&"view") && keys.contains(&"refresh"));
        assert!(
            !keys.contains(&"rename"),
            "nothing to rename on empty space"
        );
        c.context = Context::File;
        c.rebuild();
        let keys: Vec<_> = c.rows.iter().filter_map(|r| r.key.as_deref()).collect();
        assert!(keys.contains(&"open-location") && keys.contains(&"open-with"));
    }
    #[test]
    fn targets_hit_rows_tabs_style_and_buttons() {
        let c = customizer();
        let (e, r) = c
            .elems()
            .into_iter()
            .find(|(e, _)| matches!(e, Elem::Row(_)))
            .unwrap();
        let Elem::Row(i) = e else { unreachable!() };
        let (switch, _, _) = Customizer::row_controls(r);
        assert_eq!(
            c.target(switch.center_x(), switch.center_y()),
            Some(T_TOGGLE + i as u32)
        );
        let done = c.footer_buttons()[1].1;
        assert_eq!(c.target(done.center_x(), done.center_y()), Some(T_DONE));
        let tab = c.tab(2);
        assert_eq!(c.target(tab.center_x(), tab.center_y()), Some(T_TAB + 2));
        let classic = c.style_segments()[1];
        assert_eq!(
            c.target(classic.center_x(), classic.center_y()),
            Some(T_STYLE + 1)
        );
    }
    #[test]
    fn the_classic_style_writes_the_desktop_control_and_hides_the_editor() {
        let mut c = customizer();
        c.set_classic(true);
        assert!(c.pending_controls.unwrap().classic_context_menu());
        assert!(c.classic());
        let (_, r) = c
            .elems()
            .into_iter()
            .find(|(e, _)| matches!(e, Elem::Row(_)))
            .unwrap();
        assert_eq!(c.target(r.center_x(), r.center_y()), None);
        c.set_classic(false);
        assert_eq!(c.pending_controls, None, "back to the controls in effect");
    }
    #[test]
    fn every_preview_fits_its_column_without_scrolling() {
        let mut c = customizer();
        for classic in [false, true] {
            c.set_classic(classic);
            for context in EDITABLE {
                c.context = context;
                let a = c.preview_area();
                let state =
                    MenuState::open(c.preview_menu(), (a.x, 0), (a.right() as u32, a.h), false);
                let root = state.root();
                assert!(!root.more_below, "{context:?} classic={classic} scrolls");
            }
        }
    }
    /// Review renders for the owner (`DUMP_DIR=<dir> cargo test -p door-workspace
    /// render_customizer -- --ignored`): BMP files of the window.
    #[test]
    #[ignore]
    fn render_customizer() {
        use door_graphics_text::FontAtlas;
        fn bmp(path: &str, w: u32, h: u32, bgra: &[u8]) {
            let mut out = std::vec::Vec::new();
            out.extend_from_slice(b"BM");
            out.extend_from_slice(&(54 + w * h * 4).to_le_bytes());
            out.extend_from_slice(&[0; 4]);
            out.extend_from_slice(&54u32.to_le_bytes());
            out.extend_from_slice(&40u32.to_le_bytes());
            out.extend_from_slice(&(w as i32).to_le_bytes());
            out.extend_from_slice(&(-(h as i32)).to_le_bytes());
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&32u16.to_le_bytes());
            out.extend_from_slice(&[0; 24]);
            out.extend_from_slice(bgra);
            std::fs::write(path, out).unwrap();
        }
        let dir = std::env::var("DUMP_DIR").unwrap();
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let paint = |c: &Customizer, dark: bool, name: &str| {
            let (w, h) = (c.width, c.height);
            let mut px = alloc::vec![0; (w * h * 4) as usize];
            let images = ui::ImageCache::default();
            let mut v = ui::View::new_scaled(&mut px, w, h, &fonts, 1).with_images(&images);
            v.palette.dark = dark;
            v.hover = c.hover;
            c.draw(&mut v);
            bmp(&format!("{dir}/{name}.bmp"), w, h, &px);
        };
        let mut c = customizer();
        let (_, r) = c
            .elems()
            .into_iter()
            .filter(|(e, _)| matches!(e, Elem::Row(_)))
            .nth(6)
            .unwrap();
        c.hover = Some((r.x + 120, r.center_y()));
        paint(&c, false, "customizer-file");
        c.hover = None;
        c.context = Context::Background;
        c.rebuild();
        paint(&c, true, "customizer-background-dark");
        c.context = Context::Folder;
        c.rebuild();
        c.set_classic(true);
        paint(&c, false, "customizer-classic");
    }
}
