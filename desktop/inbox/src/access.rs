//! The accessibility apps Start files under Accessibility:
//! - Magnifier: its toolbar (zoom out, the zoom level, zoom in, Views,
//!   reading controls, settings); the shell magnifies the screen for it
//!   (`window::MAGNIFY`: full screen following the pointer, lens, docked)
//!   and undoes it when Magnifier closes.
//! - On-Screen Keyboard: the classic osk.exe layout (main block and the
//!   navigation block), sticky modifiers; keys go to the focused window
//!   through the shell (`window::INJECT_KEY`) and the keyboard never takes
//!   focus.
//! - Narrator: Narrator Home. Speech needs a voice (speech synthesis) Doors OS does not
//!   have yet, which it says.
//! - Voice access: its bar. It needs a microphone, which it says.
use crate::paint::view::{Ink, Rect, button, glyph, ink};
use alloc::{format, string::String, vec::Vec};
use door_app_ui::{Application, View};
use door_graphics_text::{FontAtlas, TextStyle};
use door_service_client::{Error, Storage, window::WindowClient};

pub const TITLE: i32 = 32;
const fn rgb(r: u8, g: u8, b: u8) -> [u8; 4] {
    [b, g, r, 255]
}
fn theme(v: &mut View<'_>) -> Ink {
    let dark = v.palette.dark;
    ink(dark)
}
fn backdrop(v: &mut View<'_>, k: &Ink, w: u32, h: u32) {
    if v.palette.system().is_some() {
        v.rect(0, 0, w, h, k.mica);
    } else {
        v.material_rect(0, 0, w, h, k.mica, 230);
    }
}
fn title_bar(v: &mut View<'_>, k: &Ink, app: crate::App, title: &str) {
    if let Some(px) = crate::icon_frames(app, v.scale) {
        v.color_pixels(12, 8, 16, px);
    }
    v.text_clip_centered(
        40,
        (0, TITLE as u32),
        300,
        title,
        TextStyle::Regular12,
        k.text,
    );
}
/// Open a Settings page by its ms-settings name (Accessibility pages).
fn open_settings(c: &mut WindowClient, name: &str) {
    if let Some((page, section)) = door_settings_map::resolve(name) {
        let _ = c.open_settings_page(door_settings_map::navigate_arg(page, section));
    }
}

// ---- Magnifier ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MagView {
    FullScreen,
    Lens,
    Docked,
}
impl MagView {
    const ALL: [Self; 3] = [Self::Docked, Self::FullScreen, Self::Lens];
    const fn label(self) -> &'static str {
        match self {
            Self::FullScreen => "Full screen",
            Self::Lens => "Lens",
            Self::Docked => "Docked",
        }
    }
    const fn key(self) -> &'static str {
        match self {
            Self::FullScreen => "Ctrl+Alt+F",
            Self::Lens => "Ctrl+Alt+L",
            Self::Docked => "Ctrl+Alt+D",
        }
    }
    const fn word(self) -> u32 {
        match self {
            Self::FullScreen => 0,
            Self::Lens => 1,
            Self::Docked => 2,
        }
    }
}
pub struct Magnifier {
    pub width: u32,
    pub height: u32,
    ready: bool,
    pub percent: u32,
    pub step: u32,
    pub view: MagView,
    pub menu: bool,
    /// The shell's magnification is out of date.
    dirty: bool,
    settings: bool,
    pub applied: Option<(u32, MagView)>,
}
impl Default for Magnifier {
    fn default() -> Self {
        Self::new()
    }
}
impl Magnifier {
    pub fn new() -> Self {
        Self {
            width: 420,
            height: 82,
            ready: false,
            percent: 200,
            step: 100,
            view: MagView::FullScreen,
            menu: false,
            dirty: true,
            settings: false,
            applied: None,
        }
    }
    fn buttons(&self) -> [(Rect, u32); 7] {
        let y = TITLE + 6;
        [
            (Rect::new(8, y, 40, 36), 1),   // zoom out
            (Rect::new(52, y, 64, 36), 2),  // level (Views shortcut)
            (Rect::new(120, y, 40, 36), 3), // zoom in
            (Rect::new(172, y, 88, 36), 4), // Views
            (Rect::new(268, y, 40, 36), 5), // read from here
            (Rect::new(312, y, 40, 36), 6), // settings
            (Rect::new(0, 0, 0, 0), 0),
        ]
    }
    fn menu_rows(&self) -> [Rect; 3] {
        let b = self.buttons()[3].0;
        [0, 1, 2].map(|i| Rect::new(b.x, b.y + b.h as i32 + 4 + i * 36, 220, 36))
    }
    fn zoom(&mut self, up: bool) {
        self.percent = if up {
            (self.percent + self.step).min(1600)
        } else {
            self.percent.saturating_sub(self.step).max(100)
        };
        self.dirty = true;
    }
    pub fn paint(&self, v: &mut View<'_>) {
        let k = theme(v);
        backdrop(v, &k, self.width, self.height);
        title_bar(v, &k, crate::App::Magnifier, "Magnifier");
        let hover = v.hover;
        let hovered = |r: Rect| hover.is_some_and(|(x, y)| r.contains(x, y));
        for (r, t) in self.buttons() {
            if t == 0 {
                continue;
            }
            let enabled = t != 5;
            if enabled && (hovered(r) || (t == 4 && self.menu)) {
                v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.hover);
            }
            let color = if enabled { k.text } else { k.disabled };
            match t {
                1 => glyph(v, "access-zoom-out", r.x + 12, r.y + 10, 16, color),
                2 => {
                    let text = format!("{}%", self.percent);
                    let tw = v.measure_style(&text, TextStyle::Regular14) as i32;
                    v.text_clip_centered(
                        r.x + (r.w as i32 - tw) / 2,
                        (r.y, r.h),
                        r.w,
                        &text,
                        TextStyle::Regular14,
                        k.text,
                    );
                }
                3 => glyph(v, "access-zoom-in", r.x + 12, r.y + 10, 16, color),
                4 => {
                    v.text_clip_centered(
                        r.x + 12,
                        (r.y, r.h),
                        50,
                        "Views",
                        TextStyle::Regular14,
                        k.text,
                    );
                    glyph(v, "paint-chevron-down", r.x + 66, r.y + 12, 12, k.text);
                }
                5 => glyph(v, "access-play", r.x + 12, r.y + 10, 16, color),
                _ => glyph(v, "paint-settings", r.x + 12, r.y + 10, 16, color),
            }
        }
        v.rect(166, TITLE + 14, 1, 20, k.divider);
        v.rect(264, TITLE + 14, 1, 20, k.divider);
        if self.menu {
            let rows = self.menu_rows();
            let f = Rect::new(rows[0].x - 4, rows[0].y - 4, 228, 3 * 36 + 8);
            v.elevation(f.x, f.y, f.w, f.h, 80);
            v.rounded_asset(f.x, f.y, f.w, f.h, 8, k.flyout_edge);
            v.rounded_asset(f.x + 1, f.y + 1, f.w - 2, f.h - 2, 7, k.flyout);
            for (r, view) in rows.iter().zip(MagView::ALL) {
                if view == self.view {
                    v.rounded_asset(r.x, r.y + 2, r.w, r.h - 4, 4, k.pressed);
                    v.rounded_asset(r.x, r.y + 11, 3, 14, 1, k.accent);
                } else if hovered(*r) {
                    v.rounded_asset(r.x, r.y + 2, r.w, r.h - 4, 4, k.hover);
                }
                v.text_clip_centered(
                    r.x + 14,
                    (r.y, r.h),
                    120,
                    view.label(),
                    TextStyle::Regular14,
                    k.text,
                );
                let kw = v.measure_style(view.key(), TextStyle::Regular12) as i32;
                v.text_clip_centered(
                    r.x + r.w as i32 - 12 - kw,
                    (r.y, r.h),
                    kw as u32 + 2,
                    view.key(),
                    TextStyle::Regular12,
                    k.secondary,
                );
            }
        }
    }
}
impl Application for Magnifier {
    fn title(&self) -> &str {
        "Magnifier"
    }
    fn options(&self) -> door_workspace::runtime::WindowOptions {
        door_workspace::runtime::WindowOptions {
            width: 360,
            height: 82,
            caption_height: TITLE as u32,
            caption_left: 120,
            centered: true,
        }
    }
    fn caption(&self) -> (u32, u32) {
        (120, 1)
    }
    fn configure(&mut self, w: u32, h: u32) {
        self.width = w.max(200);
        self.height = if self.menu { h.max(82) } else { h.max(60) };
        if !self.ready {
            self.ready = true;
            door_rt::diagnostic(format_args!("DOOR_INBOX READY app=magnifier\n"));
        }
    }
    fn draw(&self, v: &mut View<'_>) {
        self.paint(v);
    }
    fn target(&self, x: i32, y: i32) -> Option<u32> {
        if self.menu {
            if let Some(i) = self.menu_rows().iter().position(|r| r.contains(x, y)) {
                return Some(10 + i as u32);
            }
            return Some(99);
        }
        self.buttons()
            .into_iter()
            .find(|(r, t)| *t != 0 && r.contains(x, y))
            .map(|(_, t)| t)
    }
    fn click(&mut self, id: u32, _s: &mut Storage) -> Result<bool, Error> {
        match id {
            1 => self.zoom(false),
            3 => self.zoom(true),
            2 | 4 => self.menu = !self.menu,
            6 => self.settings = true,
            10..=12 => {
                self.view = MagView::ALL[(id - 10) as usize];
                self.menu = false;
                self.dirty = true;
            }
            99 => self.menu = false,
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn key(&mut self, key: u32, _s: &mut Storage) -> Result<bool, Error> {
        let scan = key & 511;
        let m = key >> 9;
        let ctrl_alt = m & door_abi::graphics::CONTROL != 0 && m & door_abi::graphics::ALT != 0;
        match (scan, ctrl_alt) {
            (13 | 78, _) => self.zoom(true),
            (12 | 74, _) => self.zoom(false),
            (33, true) => {
                self.view = MagView::FullScreen;
                self.dirty = true;
            }
            (38, true) => {
                self.view = MagView::Lens;
                self.dirty = true;
            }
            (32, true) => {
                self.view = MagView::Docked;
                self.dirty = true;
            }
            (1, _) if self.menu => self.menu = false,
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn update(&mut self, _s: &mut Storage, c: &mut WindowClient, _w: u64) -> Result<bool, Error> {
        if core::mem::take(&mut self.settings) {
            open_settings(c, "ms-settings:easeofaccess-magnifier");
        }
        if core::mem::take(&mut self.dirty) {
            let ok = c.magnify(self.percent, self.view.word(), 0).is_ok();
            self.applied = ok.then_some((self.percent, self.view));
            door_rt::diagnostic(format_args!(
                "DOOR_INBOX MAGNIFIER percent={} view={} ok={}\n",
                self.percent,
                self.view.word(),
                u8::from(ok)
            ));
            return Ok(true);
        }
        Ok(false)
    }
    fn closing(&mut self, _s: &mut Storage) -> Result<bool, Error> {
        Ok(true)
    }
}

// ---- On-Screen Keyboard ----

/// One key: label, shifted label, scan code, width in key units.
#[derive(Clone, Copy)]
struct Key {
    label: &'static str,
    shift: &'static str,
    scan: u32,
    units: u8,
}
const fn k(label: &'static str, shift: &'static str, scan: u32, units: u8) -> Key {
    Key {
        label,
        shift,
        scan,
        units,
    }
}
/// Rows in quarter units (4 = one key).
const ROWS: [&[Key]; 5] = [
    &[
        k("Esc", "", 1, 4),
        k("`", "~", 41, 4),
        k("1", "!", 2, 4),
        k("2", "@", 3, 4),
        k("3", "#", 4, 4),
        k("4", "$", 5, 4),
        k("5", "%", 6, 4),
        k("6", "^", 7, 4),
        k("7", "&", 8, 4),
        k("8", "*", 9, 4),
        k("9", "(", 10, 4),
        k("0", ")", 11, 4),
        k("-", "_", 12, 4),
        k("=", "+", 13, 4),
        k("\u{232b}", "", 14, 6),
    ],
    &[
        k("Tab", "", 15, 6),
        k("q", "", 16, 4),
        k("w", "", 17, 4),
        k("e", "", 18, 4),
        k("r", "", 19, 4),
        k("t", "", 20, 4),
        k("y", "", 21, 4),
        k("u", "", 22, 4),
        k("i", "", 23, 4),
        k("o", "", 24, 4),
        k("p", "", 25, 4),
        k("[", "{", 26, 4),
        k("]", "}", 27, 4),
        k("\\", "|", 43, 4),
        k("Del", "", 0x153, 4),
    ],
    &[
        k("Caps", "", 58, 8),
        k("a", "", 30, 4),
        k("s", "", 31, 4),
        k("d", "", 32, 4),
        k("f", "", 33, 4),
        k("g", "", 34, 4),
        k("h", "", 35, 4),
        k("j", "", 36, 4),
        k("k", "", 37, 4),
        k("l", "", 38, 4),
        k(";", ":", 39, 4),
        k("'", "\"", 40, 4),
        k("Enter", "", 28, 10),
    ],
    &[
        k("Shift", "", 42, 10),
        k("z", "", 44, 4),
        k("x", "", 45, 4),
        k("c", "", 46, 4),
        k("v", "", 47, 4),
        k("b", "", 48, 4),
        k("n", "", 49, 4),
        k("m", "", 50, 4),
        k(",", "<", 51, 4),
        k(".", ">", 52, 4),
        k("/", "?", 53, 4),
        k("\u{2191}", "", 0x148, 4),
        k("Shift", "", 54, 8),
    ],
    &[
        k("Fn", "", 0, 4),
        k("Ctrl", "", 29, 4),
        k("\u{229e}", "", 0x15b, 4),
        k("Alt", "", 56, 4),
        k("", "", 57, 22),
        k("Alt", "", 0x138, 4),
        k("Ctrl", "", 0x11d, 4),
        k("\u{2190}", "", 0x14b, 4),
        k("\u{2193}", "", 0x150, 4),
        k("\u{2192}", "", 0x14d, 4),
        k("\u{2630}", "", 0x15d, 4),
    ],
];
const NAV: [[(&str, u32); 3]; 5] = [
    [("Home", 0x147), ("PgUp", 0x149), ("Nav", 0)],
    [("End", 0x14f), ("PgDn", 0x151), ("Mv Up", 0)],
    [("Insert", 0x152), ("Pause", 0x145), ("Mv Dn", 0)],
    [("PrtScn", 0x137), ("ScrLk", 70), ("Dock", 0)],
    [("Options", 0), ("Help", 0), ("Fade", 0)],
];
pub struct Keyboard {
    pub width: u32,
    pub height: u32,
    ready: bool,
    pub shift: bool,
    pub caps: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub win: bool,
    queue: Vec<(u32, u32)>,
    pub sent: u32,
    pressed: Option<(usize, usize)>,
}
impl Default for Keyboard {
    fn default() -> Self {
        Self::new()
    }
}
impl Keyboard {
    pub fn new() -> Self {
        Self {
            width: 1000,
            height: 330,
            ready: false,
            shift: false,
            caps: false,
            ctrl: false,
            alt: false,
            win: false,
            queue: Vec::new(),
            sent: 0,
            pressed: None,
        }
    }
    /// Key rectangles: main rows then the navigation block (row 5..).
    fn keys(&self) -> Vec<(Rect, usize, usize)> {
        let top = TITLE + 64;
        let gap = 3;
        let main_w = (self.width as i32 - 16) * 1712 / 1972;
        let unit = main_w as f32 / 60.0; // 15 keys of 4 quarter units
        let rows_h = self.height as i32 - top - 8;
        let rh = rows_h / 5;
        let mut v = Vec::new();
        for (r, row) in ROWS.iter().enumerate() {
            let mut x = 8.0f32;
            for (i, key) in row.iter().enumerate() {
                let w = key.units as f32 * unit;
                v.push((
                    Rect::new(
                        x as i32,
                        top + r as i32 * rh,
                        (w as i32 - gap).max(1) as u32,
                        (rh - gap) as u32,
                    ),
                    r,
                    i,
                ));
                x += w;
            }
        }
        let nx = 8 + main_w + 12;
        let nw = (self.width as i32 - 8 - nx) / 3;
        for (r, row) in NAV.iter().enumerate() {
            for i in 0..row.len() {
                v.push((
                    Rect::new(
                        nx + i as i32 * nw,
                        top + r as i32 * rh,
                        (nw - gap) as u32,
                        (rh - gap) as u32,
                    ),
                    5 + r,
                    i,
                ));
            }
        }
        v
    }
    fn press(&mut self, row: usize, i: usize) {
        if row >= 5 {
            let (_, scan) = NAV[row - 5][i];
            if scan != 0 {
                self.queue.push((scan, self.flags()));
            }
            return;
        }
        let key = ROWS[row][i];
        match key.scan {
            42 | 54 => self.shift = !self.shift,
            29 | 0x11d => self.ctrl = !self.ctrl,
            56 | 0x138 => self.alt = !self.alt,
            0x15b => self.win = !self.win,
            58 => self.caps = !self.caps,
            0 => {}
            scan => {
                self.queue.push((scan, self.flags()));
                // Modifiers are one-shot, as osk.exe's are.
                self.shift = false;
                self.ctrl = false;
                self.alt = false;
                self.win = false;
            }
        }
    }
    fn flags(&self) -> u32 {
        u32::from(self.shift ^ self.caps)
            | u32::from(self.ctrl) << 1
            | u32::from(self.alt) << 2
            | u32::from(self.win) << 3
    }
    fn latched(&self, scan: u32) -> bool {
        match scan {
            42 | 54 => self.shift,
            29 | 0x11d => self.ctrl,
            56 | 0x138 => self.alt,
            0x15b => self.win,
            58 => self.caps,
            _ => false,
        }
    }
    pub fn paint(&self, v: &mut View<'_>) {
        // osk.exe keeps its own dark key field in either theme.
        let kk = theme(v);
        v.rect(0, 0, self.width, self.height, rgb(28, 28, 28));
        v.rect(
            0,
            0,
            self.width,
            TITLE as u32,
            if kk.dark {
                rgb(32, 32, 32)
            } else {
                rgb(255, 255, 255)
            },
        );
        title_bar(v, &kk, crate::App::OnScreenKeyboard, "On-Screen Keyboard");
        let hover = v.hover;
        let upper = self.shift ^ self.caps;
        for (r, row, i) in self.keys() {
            let (label, shift, scan) = if row >= 5 {
                let (l, s) = NAV[row - 5][i];
                (l, "", s)
            } else {
                let key = ROWS[row][i];
                (key.label, key.shift, key.scan)
            };
            let on = self.latched(scan);
            let pressed = self.pressed == Some((row, i));
            let fill = if on || pressed {
                rgb(0, 120, 212)
            } else if hover.is_some_and(|(x, y)| r.contains(x, y)) {
                rgb(74, 74, 74)
            } else {
                rgb(55, 55, 55)
            };
            v.rect(r.x, r.y, r.w, r.h, fill);
            let white = rgb(255, 255, 255);
            let grey = rgb(150, 150, 150);
            let single =
                label.chars().count() == 1 && label.chars().all(|c| c.is_ascii_lowercase());
            if single {
                let shown = if upper {
                    label.to_ascii_uppercase()
                } else {
                    String::from(label)
                };
                v.text_clip(
                    r.x + 6,
                    r.y + 4,
                    r.w - 8,
                    &shown,
                    TextStyle::Regular16,
                    white,
                );
            } else if !shift.is_empty() {
                v.text_clip(
                    r.x + 6,
                    r.y + 2,
                    r.w - 8,
                    shift,
                    TextStyle::Regular14,
                    if self.shift { white } else { grey },
                );
                let tw = v.measure_style(label, TextStyle::Regular16) as i32;
                v.text_clip(
                    r.x + (r.w as i32 - tw) / 2,
                    r.y + r.h as i32 - 26,
                    r.w,
                    label,
                    TextStyle::Regular16,
                    if self.shift { grey } else { white },
                );
            } else if label.chars().count() == 1 {
                let tw = v.measure_style(label, TextStyle::Regular16) as i32;
                v.text_clip(
                    r.x + (r.w as i32 - tw) / 2,
                    r.y + (r.h as i32 - 20) / 2,
                    r.w,
                    label,
                    TextStyle::Regular16,
                    white,
                );
            } else {
                let style = if v.measure_style(label, TextStyle::Regular14) + 10 > r.w {
                    TextStyle::Regular12
                } else {
                    TextStyle::Regular14
                };
                v.text_clip(r.x + 5, r.y + 4, r.w - 6, label, style, white);
            }
        }
    }
}
impl Application for Keyboard {
    fn title(&self) -> &str {
        "On-Screen Keyboard"
    }
    fn options(&self) -> door_workspace::runtime::WindowOptions {
        door_workspace::runtime::WindowOptions {
            width: 1000,
            height: 330,
            caption_height: TITLE as u32,
            caption_left: 200,
            centered: true,
        }
    }
    fn caption(&self) -> (u32, u32) {
        (200, 1)
    }
    fn configure(&mut self, w: u32, h: u32) {
        self.width = w.max(600);
        self.height = h.max(240);
        if !self.ready {
            self.ready = true;
            door_rt::diagnostic(format_args!("DOOR_INBOX READY app=osk\n"));
        }
    }
    fn draw(&self, v: &mut View<'_>) {
        self.paint(v);
    }
    fn target(&self, x: i32, y: i32) -> Option<u32> {
        self.keys()
            .into_iter()
            .find(|(r, _, _)| r.contains(x, y))
            .map(|(_, row, i)| (row * 32 + i) as u32 + 1)
    }
    fn pointer_down(&mut self, x: i32, y: i32, _f: &FontAtlas) -> bool {
        self.pressed = self
            .keys()
            .into_iter()
            .find(|(r, _, _)| r.contains(x, y))
            .map(|(_, row, i)| (row, i));
        self.pressed.is_some()
    }
    fn pointer_up(&mut self) {
        self.pressed = None;
    }
    fn click(&mut self, id: u32, _s: &mut Storage) -> Result<bool, Error> {
        let id = id - 1;
        self.press(id as usize / 32, id as usize % 32);
        Ok(true)
    }
    fn key(&mut self, _key: u32, _s: &mut Storage) -> Result<bool, Error> {
        Ok(false)
    }
    fn update(&mut self, _s: &mut Storage, c: &mut WindowClient, _w: u64) -> Result<bool, Error> {
        let mut any = false;
        for (scan, flags) in core::mem::take(&mut self.queue) {
            let ok = c.inject_key(scan, flags).is_ok();
            self.sent += u32::from(ok);
            door_rt::diagnostic(format_args!(
                "DOOR_INBOX OSK key scan={scan} flags={flags} ok={}\n",
                u8::from(ok)
            ));
            any = true;
        }
        Ok(any)
    }
}

// ---- Narrator ----

pub struct Narrator {
    pub width: u32,
    pub height: u32,
    ready: bool,
    pub page: usize,
    settings: bool,
}
const NARRATOR_NAV: [(&str, &str); 5] = [
    ("Quick start", "access-quick"),
    ("Narrator guide", "access-guide"),
    ("What's new", "access-new"),
    ("Settings", "paint-settings"),
    ("Feedback", "access-feedback"),
];
impl Default for Narrator {
    fn default() -> Self {
        Self::new()
    }
}
impl Narrator {
    pub fn new() -> Self {
        Self {
            width: 760,
            height: 560,
            ready: false,
            page: 0,
            settings: false,
        }
    }
    fn nav(&self, i: usize) -> Rect {
        Rect::new(4, TITLE + 60 + i as i32 * 40, 232, 36)
    }
    pub fn paint(&self, v: &mut View<'_>) {
        let k = theme(v);
        backdrop(v, &k, self.width, self.height);
        title_bar(v, &k, crate::App::Narrator, "Narrator Home");
        let hover = v.hover;
        v.text_clip(
            16,
            TITLE + 16,
            220,
            "Narrator Home",
            TextStyle::Semibold20,
            k.text,
        );
        for (i, (label, g)) in NARRATOR_NAV.iter().enumerate() {
            let r = self.nav(i);
            if i == self.page {
                v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.pressed);
                v.rounded_asset(r.x + 1, r.y + 10, 3, 16, 1, k.accent);
            } else if hover.is_some_and(|(x, y)| r.contains(x, y)) {
                v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.hover);
            }
            glyph(v, g, r.x + 12, r.y + 10, 16, k.text);
            v.text_clip_centered(
                r.x + 44,
                (r.y, r.h),
                r.w - 52,
                label,
                TextStyle::Regular14,
                k.text,
            );
        }
        let c = Rect::new(244, TITLE, self.width - 244, self.height - TITLE as u32);
        v.rounded_asset(c.x, c.y, c.w + 8, c.h + 8, 8, k.stroke);
        v.rounded_asset(c.x + 1, c.y + 1, c.w + 8, c.h + 8, 8, k.card);
        let x = c.x + 32;
        let lines: &[&str] = match self.page {
            0 => &[
                "Narrator is a screen reader that describes what's on your screen,",
                "so you can use your PC without seeing it.",
                "",
                "Turn Narrator on or off: Doors logo key + Ctrl + Enter.",
            ],
            1 => &[
                "The Narrator guide teaches the keys Narrator uses: the Narrator key",
                "(Caps Lock or Insert) with the arrow keys moves between items,",
                "and Narrator key + Space activates the current one.",
            ],
            2 => &[
                "What's new in Narrator: natural voices, scan mode in apps",
                "and better reading of tables and links.",
            ],
            3 => &[
                "Narrator's settings (voice, speed, verbosity) are in",
                "Settings > Accessibility > Narrator.",
            ],
            _ => &["Feedback Hub isn't available on this device."],
        };
        v.text_clip(
            x,
            c.y + 28,
            c.w - 64,
            if self.page == 0 {
                "Welcome to Narrator"
            } else {
                NARRATOR_NAV[self.page].0
            },
            TextStyle::Semibold28,
            k.text,
        );
        for (i, l) in lines.iter().enumerate() {
            v.text_clip(
                x,
                c.y + 84 + i as i32 * 22,
                c.w - 64,
                l,
                TextStyle::Regular14,
                k.text,
            );
        }
        // Honest state: no speech engine (voices) is installed yet.
        let b = Rect::new(x, c.y + 200, c.w - 64, 72);
        v.rounded_asset(
            b.x,
            b.y,
            b.w,
            b.h,
            6,
            if k.dark {
                rgb(67, 53, 25)
            } else {
                rgb(255, 244, 206)
            },
        );
        glyph(
            v,
            "access-warning",
            b.x + 16,
            b.y + 16,
            16,
            if k.dark {
                rgb(252, 225, 0)
            } else {
                rgb(157, 93, 0)
            },
        );
        v.text_clip(
            b.x + 44,
            b.y + 14,
            b.w - 60,
            "Narrator can't speak on this device yet",
            TextStyle::Semibold14,
            k.text,
        );
        v.text_clip(
            b.x + 44,
            b.y + 38,
            b.w - 60,
            "No speech voices are installed on Doors yet.",
            TextStyle::Regular14,
            k.text,
        );
        if self.page == 3 {
            let r = Rect::new(x, c.y + 296, 220, 32);
            button(
                v,
                &k,
                r,
                "Open Narrator settings",
                true,
                hover.is_some_and(|(hx, hy)| r.contains(hx, hy)),
            );
        }
    }
}
impl Application for Narrator {
    fn title(&self) -> &str {
        "Narrator Home"
    }
    fn options(&self) -> door_workspace::runtime::WindowOptions {
        door_workspace::runtime::WindowOptions {
            width: 760,
            height: 560,
            caption_height: TITLE as u32,
            caption_left: 244,
            centered: true,
        }
    }
    fn caption(&self) -> (u32, u32) {
        (244, 1)
    }
    fn configure(&mut self, w: u32, h: u32) {
        self.width = w.max(560);
        self.height = h.max(400);
        if !self.ready {
            self.ready = true;
            door_rt::diagnostic(format_args!("DOOR_INBOX READY app=narrator\n"));
        }
    }
    fn draw(&self, v: &mut View<'_>) {
        self.paint(v);
    }
    fn target(&self, x: i32, y: i32) -> Option<u32> {
        if let Some(i) = (0..NARRATOR_NAV.len()).find(|i| self.nav(*i).contains(x, y)) {
            return Some(1 + i as u32);
        }
        (self.page == 3 && Rect::new(276, TITLE + 296, 220, 32).contains(x, y)).then_some(20)
    }
    fn click(&mut self, id: u32, _s: &mut Storage) -> Result<bool, Error> {
        match id {
            1..=5 => self.page = id as usize - 1,
            20 => self.settings = true,
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn key(&mut self, _key: u32, _s: &mut Storage) -> Result<bool, Error> {
        Ok(false)
    }
    fn update(&mut self, _s: &mut Storage, c: &mut WindowClient, _w: u64) -> Result<bool, Error> {
        if core::mem::take(&mut self.settings) {
            open_settings(c, "ms-settings:easeofaccess-narrator");
        }
        Ok(false)
    }
}

// ---- Voice access ----

pub struct VoiceAccess {
    pub width: u32,
    pub height: u32,
    ready: bool,
    pub help: bool,
    settings: bool,
}
impl Default for VoiceAccess {
    fn default() -> Self {
        Self::new()
    }
}
impl VoiceAccess {
    pub fn new() -> Self {
        Self {
            width: 760,
            height: 88,
            ready: false,
            help: false,
            settings: false,
        }
    }
    fn buttons(&self) -> [(Rect, u32); 3] {
        let w = self.width as i32;
        [
            (Rect::new(12, TITLE + 8, 40, 40), 1),
            (Rect::new(w - 96, TITLE + 10, 40, 36), 2),
            (Rect::new(w - 52, TITLE + 10, 40, 36), 3),
        ]
    }
    pub fn paint(&self, v: &mut View<'_>) {
        let k = theme(v);
        backdrop(v, &k, self.width, self.height);
        title_bar(v, &k, crate::App::VoiceAccess, "Voice access");
        let hover = v.hover;
        let hovered = |r: Rect| hover.is_some_and(|(x, y)| r.contains(x, y));
        for (r, t) in self.buttons() {
            match t {
                1 => {
                    v.rounded_asset(
                        r.x,
                        r.y,
                        r.w,
                        r.h,
                        20,
                        if k.dark {
                            rgb(60, 60, 60)
                        } else {
                            rgb(229, 229, 229)
                        },
                    );
                    glyph(v, "access-mic-off", r.x + 10, r.y + 10, 20, k.disabled);
                }
                2 => {
                    if hovered(r) || self.help {
                        v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.hover);
                    }
                    glyph(v, "access-help", r.x + 12, r.y + 10, 16, k.text);
                }
                _ => {
                    if hovered(r) {
                        v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.hover);
                    }
                    glyph(v, "paint-settings", r.x + 12, r.y + 10, 16, k.text);
                }
            }
        }
        v.text_clip_centered(
            64,
            (TITLE + 4, 24),
            self.width - 180,
            "Voice access can't find a microphone",
            TextStyle::Semibold14,
            k.text,
        );
        v.text_clip_centered(
            64,
            (TITLE + 26, 24),
            self.width - 180,
            "Connect a microphone to control your PC with your voice.",
            TextStyle::Regular12,
            k.secondary,
        );
        if self.help {
            let f = Rect::new(self.width as i32 - 340, TITLE + 50, 320, 120);
            v.elevation(f.x, f.y, f.w, f.h, 80);
            v.rounded_asset(f.x, f.y, f.w, f.h, 8, k.flyout_edge);
            v.rounded_asset(f.x + 1, f.y + 1, f.w - 2, f.h - 2, 7, k.flyout);
            for (i, l) in [
                "Say \"Voice access wake up\" to start listening,",
                "\"Click Start\" to click a button, or \"Show",
                "numbers\" to label everything you can click.",
                "Voice access needs a microphone first.",
            ]
            .iter()
            .enumerate()
            {
                v.text_clip(
                    f.x + 16,
                    f.y + 14 + i as i32 * 22,
                    f.w - 32,
                    l,
                    TextStyle::Regular14,
                    k.text,
                );
            }
        }
    }
}
impl Application for VoiceAccess {
    fn title(&self) -> &str {
        "Voice access"
    }
    fn options(&self) -> door_workspace::runtime::WindowOptions {
        door_workspace::runtime::WindowOptions {
            width: 760,
            height: 88,
            caption_height: TITLE as u32,
            caption_left: 160,
            centered: true,
        }
    }
    fn caption(&self) -> (u32, u32) {
        (160, 1)
    }
    fn configure(&mut self, w: u32, h: u32) {
        self.width = w.max(480);
        self.height = h.max(80);
        if !self.ready {
            self.ready = true;
            door_rt::diagnostic(format_args!("DOOR_INBOX READY app=voice\n"));
        }
    }
    fn draw(&self, v: &mut View<'_>) {
        self.paint(v);
    }
    fn target(&self, x: i32, y: i32) -> Option<u32> {
        self.buttons()
            .into_iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, t)| t)
            .or(self.help.then_some(9))
    }
    fn click(&mut self, id: u32, _s: &mut Storage) -> Result<bool, Error> {
        match id {
            2 => self.help = !self.help,
            3 => self.settings = true,
            9 => self.help = false,
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn key(&mut self, _key: u32, _s: &mut Storage) -> Result<bool, Error> {
        Ok(false)
    }
    fn update(&mut self, _s: &mut Storage, c: &mut WindowClient, _w: u64) -> Result<bool, Error> {
        if core::mem::take(&mut self.settings) {
            open_settings(c, "ms-settings:easeofaccess-speechrecognition");
        }
        Ok(false)
    }
}
