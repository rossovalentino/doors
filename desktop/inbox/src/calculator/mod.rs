//! Calculator's window, measured from Windows 11 26H2 Calculator at 150%:
//! a 32 px title bar, the 40 px header (navigation button, "Standard"),
//! the right-aligned expression and result, the memory row and a 4 x 6
//! keypad of 4 px-radius keys 2 px apart over Mica; from 640 px wide the
//! History/Memory pane docks on the right (320 px), narrower windows open it
//! as a flyout over the keypad.
mod engine;
mod number;
use alloc::{string::String, vec::Vec};
use door_app_ui::{Application, View};
use door_graphics_text::{FontAtlas, TextStyle};
use door_service_client::{Error, Storage};
pub use engine::{Calculator as Engine, Command, Op};
pub use number::Number;

pub const TITLE: i32 = 32;
const HEADER: i32 = 44;
const MEMORY_ROW: i32 = 40;
/// Docked History/Memory pane width and the window width that docks it.
pub const PANE: u32 = 320;
pub const DOCK: u32 = 640;
const MARGIN: i32 = 4;

/// Target ids.
mod id {
    pub const KEY: u32 = 1; // 1..=24, row-major
    pub const MEMORY: u32 = 30; // MC MR M+ M- MS M▾
    pub const NAV: u32 = 40;
    pub const HISTORY_BUTTON: u32 = 41;
    pub const TAB: u32 = 50; // History, Memory
    pub const CLEAR: u32 = 52;
    pub const BACKDROP: u32 = 53;
    pub const NAV_STANDARD: u32 = 54;
    pub const HISTORY_ITEM: u32 = 100;
    pub const MEMORY_ITEM: u32 = 300;
    /// Hover buttons of memory item i: 600 + 3i + (MC, M+, M-).
    pub const MEMORY_ITEM_BUTTON: u32 = 600;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Face {
    /// Digits, the point and negate: the lighter number keys.
    Number,
    /// Operators and functions.
    Operator,
    /// "=": the accent key.
    Accent,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Label {
    Text(&'static str),
    Glyph(&'static str),
    Reciprocal,
    Square,
    Root,
    Negate,
}
const KEYS: [(Label, Face, Command); 24] = {
    use Face::{Accent, Number as Num, Operator as Ops};
    use Label::{Glyph, Text};
    [
        (Glyph("calc-percent"), Ops, Command::Percent),
        (Text("CE"), Ops, Command::ClearEntry),
        (Text("C"), Ops, Command::Clear),
        (Glyph("calc-backspace"), Ops, Command::Backspace),
        (Label::Reciprocal, Ops, Command::Reciprocal),
        (Label::Square, Ops, Command::Square),
        (Label::Root, Ops, Command::SquareRoot),
        (Glyph("calc-divide"), Ops, Command::Op(Op::Div)),
        (Text("7"), Num, Command::Digit(7)),
        (Text("8"), Num, Command::Digit(8)),
        (Text("9"), Num, Command::Digit(9)),
        (Glyph("calc-multiply"), Ops, Command::Op(Op::Mul)),
        (Text("4"), Num, Command::Digit(4)),
        (Text("5"), Num, Command::Digit(5)),
        (Text("6"), Num, Command::Digit(6)),
        (Glyph("calc-subtract"), Ops, Command::Op(Op::Sub)),
        (Text("1"), Num, Command::Digit(1)),
        (Text("2"), Num, Command::Digit(2)),
        (Text("3"), Num, Command::Digit(3)),
        (Glyph("calc-add"), Ops, Command::Op(Op::Add)),
        (Label::Negate, Num, Command::Negate),
        (Text("0"), Num, Command::Digit(0)),
        (Text("."), Num, Command::Point),
        (Glyph("calc-equals"), Accent, Command::Equals),
    ]
};
const MEMORY_LABELS: [(&str, Command); 5] = [
    ("MC", Command::MemoryClear),
    ("MR", Command::MemoryRecall),
    ("M+", Command::MemoryAdd),
    ("M\u{2212}", Command::MemorySubtract),
    ("MS", Command::MemoryStore),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    History,
    Memory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}
impl Rect {
    const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }
    pub fn contains(self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w as i32 && py < self.y + self.h as i32
    }
}

/// Theme colours: premultiplied BGRA fills over Mica and text.
struct Ink {
    mica: [u8; 4],
    number: [u8; 4],
    number_hover: [u8; 4],
    number_press: [u8; 4],
    operator: [u8; 4],
    operator_hover: [u8; 4],
    operator_press: [u8; 4],
    accent: [u8; 4],
    accent_hover: [u8; 4],
    accent_press: [u8; 4],
    on_accent: [u8; 4],
    edge: [u8; 4],
    text: [u8; 4],
    secondary: [u8; 4],
    disabled: [u8; 4],
    subtle_hover: [u8; 4],
}
const fn white(a: u8) -> [u8; 4] {
    [a, a, a, a]
}
const fn black(a: u8) -> [u8; 4] {
    [0, 0, 0, a]
}
/// Opaque grey/colour helpers (BGRA).
const fn rgb(r: u8, g: u8, b: u8) -> [u8; 4] {
    [b, g, r, 255]
}
fn ink(dark: bool) -> Ink {
    if dark {
        // Sampled at 150%: number keys lift Mica by 12% white, operator
        // keys by 8%; "=" is the dark-theme accent #4CC2FF with black text.
        Ink {
            mica: rgb(32, 32, 32),
            number: white(0x1f),
            number_hover: white(0x28),
            number_press: white(0x12),
            operator: white(0x14),
            operator_hover: white(0x1d),
            operator_press: white(0x0a),
            accent: rgb(0x4c, 0xc2, 0xff),
            accent_hover: rgb(0x47, 0xb1, 0xe8),
            accent_press: rgb(0x42, 0xa1, 0xd2),
            on_accent: rgb(0, 0, 0),
            edge: white(0x0c),
            text: rgb(255, 255, 255),
            secondary: rgb(206, 206, 206),
            disabled: rgb(93, 93, 93),
            subtle_hover: white(0x0f),
        }
    } else {
        Ink {
            mica: rgb(243, 243, 243),
            number: white(0xb3),
            number_hover: white(0x80),
            number_press: white(0x4d),
            operator: [0x7c, 0x7c, 0x7c, 0x80],
            operator_hover: [0x7a, 0x7a, 0x7a, 0x99],
            operator_press: [0x7c, 0x7c, 0x7c, 0x4d],
            accent: rgb(0x00, 0x5f, 0xb8),
            accent_hover: rgb(0x19, 0x6e, 0xbf),
            accent_press: rgb(0x31, 0x7e, 0xc6),
            on_accent: rgb(255, 255, 255),
            edge: black(0x0f),
            text: rgb(26, 26, 26),
            secondary: rgb(96, 96, 96),
            disabled: rgb(160, 160, 160),
            subtle_hover: black(0x09),
        }
    }
}

pub struct Calculator {
    pub engine: Engine,
    pub width: u32,
    pub height: u32,
    pub tab: Tab,
    /// Narrow windows: the History/Memory flyout over the keypad.
    pub flyout: Option<Tab>,
    pub nav: bool,
    ready: bool,
}

impl Default for Calculator {
    fn default() -> Self {
        Self::new()
    }
}

impl Calculator {
    pub fn new() -> Self {
        Self {
            engine: Engine::new(),
            width: 322,
            height: 500,
            tab: Tab::History,
            flyout: None,
            nav: false,
            ready: false,
        }
    }
    pub fn docked(&self) -> bool {
        self.width >= DOCK
    }
    /// Width of the calculator column (the docked pane takes the rest).
    fn column(&self) -> u32 {
        if self.docked() {
            self.width - PANE
        } else {
            self.width
        }
    }
    /// The keypad rows fill what the title, header, display and memory row
    /// leave; the display takes a quarter of that (as measured at 150%)
    /// and at least 108 px.
    fn bands(&self) -> (i32, i32, i32, i32) {
        let h = self.height as i32;
        let free = h - TITLE - HEADER - MEMORY_ROW - MARGIN;
        let display = (free / 4).max(108).min(free - 6 * 40);
        let display_top = TITLE + HEADER;
        let memory_top = display_top + display;
        let keys_top = memory_top + MEMORY_ROW;
        (display_top, memory_top, keys_top, h - MARGIN)
    }
    pub fn key_rect(&self, index: usize) -> Rect {
        let (_, _, top, bottom) = self.bands();
        let col = (index % 4) as i32;
        let row = (index / 4) as i32;
        let w = self.column() as i32 - 2 * MARGIN;
        let x0 = MARGIN + w * col / 4;
        let x1 = MARGIN + w * (col + 1) / 4;
        let y0 = top + (bottom - top) * row / 6;
        let y1 = top + (bottom - top) * (row + 1) / 6;
        // 1 px margin around every key.
        Rect::new(
            x0 + 1,
            y0 + 1,
            (x1 - x0 - 2).max(1) as u32,
            (y1 - y0 - 2).max(1) as u32,
        )
    }
    /// Memory row buttons; narrow windows add "M▾" (the memory flyout).
    pub fn memory_rect(&self, index: usize) -> Rect {
        let (_, top, _, _) = self.bands();
        let n = if self.docked() { 5 } else { 6 };
        let w = self.column() as i32 - 2 * MARGIN;
        let pitch = if self.docked() { 80.min(w / n) } else { w / n };
        Rect::new(
            MARGIN + pitch * index as i32,
            top + 4,
            pitch as u32,
            (MEMORY_ROW - 8) as u32,
        )
    }
    fn memory_count(&self) -> usize {
        if self.docked() { 5 } else { 6 }
    }
    fn nav_rect() -> Rect {
        Rect::new(4, TITLE + 2, 40, 40)
    }
    fn history_button(&self) -> Rect {
        Rect::new(self.width as i32 - 44, TITLE + 2, 40, 40)
    }
    /// The pane's area: docked on the right, or the flyout over the keypad.
    fn pane_rect(&self) -> Option<Rect> {
        if self.docked() {
            return Some(Rect::new(
                self.column() as i32,
                TITLE,
                PANE,
                self.height - TITLE as u32,
            ));
        }
        self.flyout?;
        let (_, memory_top, _, _) = self.bands();
        Some(Rect::new(
            0,
            memory_top,
            self.width,
            self.height - memory_top as u32,
        ))
    }
    fn shown_tab(&self) -> Tab {
        if self.docked() {
            self.tab
        } else {
            self.flyout.unwrap_or(self.tab)
        }
    }
    fn tab_rect(&self, i: usize) -> Rect {
        let p = self.pane_rect().unwrap_or(Rect::new(0, 0, 0, 0));
        Rect::new(p.x + 6 + i as i32 * 74, p.y + 4, 70, 40)
    }
    fn clear_rect(&self) -> Rect {
        let p = self.pane_rect().unwrap_or(Rect::new(0, 0, 0, 0));
        Rect::new(p.x + p.w as i32 - 48, p.y + p.h as i32 - 48, 40, 40)
    }
    fn list_item(&self, i: usize) -> Rect {
        let p = self.pane_rect().unwrap_or(Rect::new(0, 0, 0, 0));
        let pitch = 72;
        Rect::new(
            p.x + 4,
            p.y + 52 + i as i32 * pitch,
            p.w - 8,
            pitch as u32 - 4,
        )
    }
    fn list_capacity(&self) -> usize {
        let p = self.pane_rect().unwrap_or(Rect::new(0, 0, 0, 0));
        ((p.h as i32 - 52 - 56).max(0) / 72) as usize
    }
    fn memory_item_button(&self, i: usize, k: usize) -> Rect {
        let r = self.list_item(i);
        Rect::new(
            r.x + r.w as i32 - 12 - (3 - k as i32) * 44,
            r.y + r.h as i32 - 34,
            40,
            28,
        )
    }
    fn nav_panel() -> Rect {
        Rect::new(0, TITLE, 256, 400)
    }

    fn press(&mut self, c: Command) {
        self.engine.press(c);
        // The guest scenarios read each result (`p14-inbox`).
        if c == Command::Equals {
            door_rt::diagnostic(format_args!(
                "DOOR_INBOX CALCULATOR display={}\n",
                self.engine.display()
            ));
        }
    }
    fn memory_enabled(&self, index: usize) -> bool {
        match index {
            5 => !self.engine.memory.is_empty() && !self.engine.error(),
            i => self.engine.enabled(MEMORY_LABELS[i].1),
        }
    }

    pub fn hits(&self) -> Vec<(Rect, u32)> {
        let mut out = Vec::new();
        if self.nav {
            out.push((Self::nav_rect(), id::NAV));
            out.push((Rect::new(4, TITLE + 48, 248, 40), id::NAV_STANDARD));
            out.push((Rect::new(0, 0, self.width, self.height), id::BACKDROP));
            return out;
        }
        out.push((Self::nav_rect(), id::NAV));
        if !self.docked() {
            out.push((self.history_button(), id::HISTORY_BUTTON));
        }
        if let Some(p) = self.pane_rect() {
            let tab = self.shown_tab();
            for i in 0..2 {
                out.push((self.tab_rect(i), id::TAB + i as u32));
            }
            let count = match tab {
                Tab::History => self.engine.history.len(),
                Tab::Memory => self.engine.memory.len(),
            };
            if count > 0 {
                out.push((self.clear_rect(), id::CLEAR));
            }
            for i in 0..count.min(self.list_capacity()) {
                if tab == Tab::Memory {
                    for k in 0..3 {
                        out.push((
                            self.memory_item_button(i, k),
                            id::MEMORY_ITEM_BUTTON + 3 * i as u32 + k as u32,
                        ));
                    }
                    out.push((self.list_item(i), id::MEMORY_ITEM + i as u32));
                } else {
                    out.push((self.list_item(i), id::HISTORY_ITEM + i as u32));
                }
            }
            if !self.docked() {
                // The flyout covers the keypad; elsewhere dismisses it.
                out.push((p, u32::MAX));
                out.push((Rect::new(0, 0, self.width, self.height), id::BACKDROP));
                return out;
            }
        }
        for i in 0..self.memory_count() {
            out.push((self.memory_rect(i), id::MEMORY + i as u32));
        }
        for i in 0..KEYS.len() {
            out.push((self.key_rect(i), id::KEY + i as u32));
        }
        out
    }

    pub(crate) fn activate(&mut self, target: u32) -> bool {
        match target {
            t if (id::KEY..id::KEY + 24).contains(&t) => {
                self.press(KEYS[(t - id::KEY) as usize].2);
            }
            t if (id::MEMORY..id::MEMORY + 5).contains(&t) => {
                self.press(MEMORY_LABELS[(t - id::MEMORY) as usize].1);
            }
            t if t == id::MEMORY + 5 => {
                if !self.engine.memory.is_empty() {
                    self.flyout = Some(Tab::Memory);
                }
            }
            id::NAV => self.nav = !self.nav,
            id::NAV_STANDARD => self.nav = false,
            id::HISTORY_BUTTON => {
                self.flyout = match self.flyout {
                    Some(_) => None,
                    None => Some(Tab::History),
                };
            }
            t if t == id::TAB || t == id::TAB + 1 => {
                let tab = if t == id::TAB {
                    Tab::History
                } else {
                    Tab::Memory
                };
                if self.docked() {
                    self.tab = tab;
                } else {
                    self.flyout = Some(tab);
                }
            }
            id::CLEAR => match self.shown_tab() {
                Tab::History => self.engine.clear_history(),
                Tab::Memory => self.engine.memory.clear(),
            },
            id::BACKDROP => {
                self.nav = false;
                self.flyout = None;
            }
            t if (id::MEMORY_ITEM_BUTTON..id::MEMORY_ITEM_BUTTON + 300).contains(&t) => {
                let n = t - id::MEMORY_ITEM_BUTTON;
                let action = [
                    Command::MemoryClear,
                    Command::MemoryAdd,
                    Command::MemorySubtract,
                ][(n % 3) as usize];
                self.engine.memory_item((n / 3) as usize, action);
            }
            t if (id::MEMORY_ITEM..id::MEMORY_ITEM + 300).contains(&t) => {
                self.engine
                    .memory_item((t - id::MEMORY_ITEM) as usize, Command::MemoryRecall);
                self.flyout = None;
            }
            t if (id::HISTORY_ITEM..id::HISTORY_ITEM + 200).contains(&t) => {
                self.engine.recall((t - id::HISTORY_ITEM) as usize);
                self.flyout = None;
            }
            _ => return false,
        }
        true
    }

    /// Windows Calculator's keyboard: digits and operators on the main and
    /// numeric keypads, Enter/=, Backspace, Esc (C), Delete (CE), % F9 R @ Q,
    /// Ctrl+M/P/Q/R/L for memory and Ctrl+H for history (scan code set 1,
    /// US layout).
    fn key_command(key: u32) -> Option<Command> {
        let scan = key & 511;
        let m = key >> 9;
        let shift = m & door_abi::graphics::SHIFT != 0;
        let control = m & door_abi::graphics::CONTROL != 0;
        use Command::*;
        Some(match (scan, shift, control) {
            (50, _, true) => MemoryStore,
            (25, _, true) => MemoryAdd,
            (16, _, true) => MemorySubtract,
            (19, _, true) => MemoryRecall,
            (38, _, true) => MemoryClear,
            (_, _, true) => return None,
            (9, true, _) | (55, _, _) => Command::Op(self::Op::Mul),
            (13, true, _) | (78, _, _) => Command::Op(self::Op::Add),
            (6, true, _) => Percent,
            (3, true, _) => SquareRoot,
            (2..=11, false, _) => Digit(((scan - 1) % 10) as u8),
            (71..=73, _, _) => Digit((scan - 64) as u8),
            (75..=77, _, _) => Digit((scan - 71) as u8),
            (79..=81, _, _) => Digit((scan - 78) as u8),
            (82, _, _) => Digit(0),
            (12, false, _) | (74, _, _) => Command::Op(self::Op::Sub),
            (53, false, _) | (0x135, _, _) => Command::Op(self::Op::Div),
            (13, false, _) | (28, _, _) | (0x11c, _, _) => Equals,
            (52, false, _) | (83, _, _) => Point,
            (14, _, _) => Backspace,
            (1, _, _) => Clear,
            (0x153, _, _) => ClearEntry,
            (67, _, _) => Command::Negate,
            (19, false, _) => Reciprocal,
            (16, false, _) => Square,
            _ => return None,
        })
    }

    pub fn paint(&self, v: &mut View<'_>) {
        let k = ink(v.palette.dark);
        let system = v.palette.system().is_some();
        if system {
            v.rect(0, 0, v.width, v.height, k.mica);
        } else {
            v.material_rect(0, 0, v.width, v.height, k.mica, 220);
        }
        self.paint_title(v, &k);
        self.paint_header(v, &k);
        self.paint_display(v, &k);
        if !self.docked() && self.flyout.is_some() {
            self.paint_pane(v, &k);
        } else {
            self.paint_memory(v, &k);
            self.paint_keys(v, &k);
            if self.docked() {
                self.paint_pane(v, &k);
            }
        }
        if self.nav {
            self.paint_nav(v, &k);
        }
    }
    fn paint_title(&self, v: &mut View<'_>, k: &Ink) {
        if let Some(px) = super::icon_frames(super::App::Calculator, v.scale) {
            v.color_pixels(16, 8, 16, px);
        }
        v.text_clip_centered(
            48,
            (0, TITLE as u32),
            200,
            "Calculator",
            TextStyle::Regular12,
            k.text,
        );
    }
    fn paint_header(&self, v: &mut View<'_>, k: &Ink) {
        let n = Self::nav_rect();
        if v.hover.is_some_and(|(x, y)| n.contains(x, y)) || self.nav {
            v.rounded_asset(n.x, n.y, n.w, n.h, 4, k.subtle_hover);
        }
        super::glyph(v, "calc-nav", n.x + 12, n.y + 12, 16, k.text);
        v.text_clip_centered(
            52,
            (TITLE + 2, 40),
            200,
            "Standard",
            TextStyle::Semibold20,
            k.text,
        );
        if !self.docked() {
            let h = self.history_button();
            if v.hover.is_some_and(|(x, y)| h.contains(x, y)) || self.flyout == Some(Tab::History) {
                v.rounded_asset(h.x, h.y, h.w, h.h, 4, k.subtle_hover);
            }
            super::glyph(v, "calc-history", h.x + 12, h.y + 12, 16, k.text);
        }
    }
    fn paint_display(&self, v: &mut View<'_>, k: &Ink) {
        let (top, bottom, _, _) = self.bands();
        let right = self.column() as i32 - 12;
        let width = (self.column() as i32 - 24).max(1) as u32;
        // Expression line, right aligned, secondary.
        let expr = self.engine.expression();
        if !expr.is_empty() {
            let w = v.measure_style(expr, TextStyle::Regular14).min(width);
            v.text_clip_centered(
                right - w as i32,
                (top + 4, 20),
                w + 2,
                expr,
                TextStyle::Regular14,
                k.secondary,
            );
        }
        // Result: Semibold, as large as the display allows (46 px at the
        // default size, 72 px in a large window), shrinking to fit.
        let text = self.engine.display();
        let error = self.engine.error();
        let room = (bottom - top - 28).max(24);
        let mut size = if error {
            26.0
        } else {
            (room as f32 * 0.7).clamp(28.0, 72.0)
        };
        let drawn = super::big_text(v, &text, &mut size, width, right, bottom - 8, k.text);
        if !drawn {
            let w = v.measure_style(&text, TextStyle::Semibold28).min(width);
            v.text_clip_centered(
                right - w as i32,
                (bottom - 48, 40),
                w + 2,
                &text,
                TextStyle::Semibold28,
                k.text,
            );
        }
    }
    fn paint_memory(&self, v: &mut View<'_>, k: &Ink) {
        for i in 0..self.memory_count() {
            let r = self.memory_rect(i);
            let enabled = self.memory_enabled(i);
            if enabled && v.hover.is_some_and(|(x, y)| r.contains(x, y)) {
                v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.subtle_hover);
            }
            let label = MEMORY_LABELS.get(i).map_or("M\u{25be}", |m| m.0);
            let w = v.measure_style(label, TextStyle::Regular12);
            v.text_clip_centered(
                r.x + (r.w as i32 - w as i32) / 2,
                (r.y, r.h),
                w + 2,
                label,
                TextStyle::Regular12,
                if enabled { k.text } else { k.disabled },
            );
        }
    }
    fn paint_keys(&self, v: &mut View<'_>, k: &Ink) {
        let large = self.key_rect(0).h >= 70;
        for (i, (label, face, command)) in KEYS.iter().enumerate() {
            let r = self.key_rect(i);
            let enabled = self.engine.enabled(*command);
            let hover = enabled && v.hover.is_some_and(|(x, y)| r.contains(x, y));
            let press = enabled && v.pressed.is_some_and(|(x, y)| r.contains(x, y));
            let fill = match (face, press, hover) {
                (Face::Number, true, _) => k.number_press,
                (Face::Number, _, true) => k.number_hover,
                (Face::Number, _, _) => k.number,
                (Face::Operator, true, _) => k.operator_press,
                (Face::Operator, _, true) => k.operator_hover,
                (Face::Operator, _, _) => k.operator,
                (Face::Accent, true, _) => k.accent_press,
                (Face::Accent, _, true) => k.accent_hover,
                (Face::Accent, _, _) => k.accent,
            };
            v.rounded_asset(r.x, r.y, r.w, r.h, 4, fill);
            // The elevation edge: a lit top line in dark, a shaded bottom
            // line in light.
            if v.palette.dark {
                v.rounded_asset(r.x + 2, r.y, r.w.saturating_sub(4), 1, 0, k.edge);
            } else {
                v.rounded_asset(
                    r.x + 2,
                    r.y + r.h as i32 - 1,
                    r.w.saturating_sub(4),
                    1,
                    0,
                    k.edge,
                );
            }
            let color = match (face, enabled) {
                (Face::Accent, _) => k.on_accent,
                (_, false) => k.disabled,
                _ if press => k.secondary,
                _ => k.text,
            };
            let (cx, cy) = (r.x + r.w as i32 / 2, r.y + r.h as i32 / 2);
            let glyph = if large { 20 } else { 16 };
            match label {
                Label::Text(t) => {
                    let style = if t.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
                        TextStyle::Regular20
                    } else {
                        TextStyle::Regular16
                    };
                    let w = v.measure_style(t, style);
                    v.text_clip_centered(cx - w as i32 / 2, (r.y, r.h), w + 2, t, style, color);
                }
                Label::Glyph(name) => {
                    super::glyph(v, name, cx - glyph / 2, cy - glyph / 2, glyph as u32, color)
                }
                Label::Reciprocal => super::fraction(v, cx, cy, "1", "x", color),
                Label::Square => super::power(v, cx, cy, "x", "2", color),
                Label::Root => super::root(v, cx, cy, color),
                Label::Negate => super::fraction(v, cx, cy, "+", "\u{2212}", color),
            }
        }
    }
    fn paint_pane(&self, v: &mut View<'_>, k: &Ink) {
        let Some(p) = self.pane_rect() else {
            return;
        };
        if !self.docked() {
            // The flyout: an acrylic sheet over the keypad.
            let sheet = if v.palette.dark {
                rgb(44, 44, 44)
            } else {
                rgb(249, 249, 249)
            };
            v.elevation(p.x, p.y, p.w, p.h, 60);
            v.rounded_asset(p.x, p.y, p.w, p.h + 8, 8, sheet);
        }
        let tab = self.shown_tab();
        for (i, label) in ["History", "Memory"].into_iter().enumerate() {
            let r = self.tab_rect(i);
            let selected = (i == 0) == (tab == Tab::History);
            let w = v.measure_style(label, TextStyle::Regular14);
            let x = r.x + 8;
            v.text_clip_centered(
                x,
                (r.y, r.h),
                w + 2,
                label,
                TextStyle::Regular14,
                if selected { k.text } else { k.secondary },
            );
            if selected {
                let accent = if v.palette.dark {
                    rgb(0x4c, 0xc2, 0xff)
                } else {
                    rgb(0x00, 0x5f, 0xb8)
                };
                v.rounded_asset(x + w as i32 / 2 - 8, r.y + r.h as i32 - 6, 16, 3, 1, accent);
            }
        }
        let count = match tab {
            Tab::History => self.engine.history.len(),
            Tab::Memory => self.engine.memory.len(),
        };
        if count == 0 {
            let text = match tab {
                Tab::History => "There\u{2019}s no history yet.",
                Tab::Memory => "There\u{2019}s nothing saved in memory.",
            };
            v.text_clip_centered(
                p.x + 14,
                (p.y + 64, 24),
                p.w - 28,
                text,
                TextStyle::Regular14,
                k.text,
            );
            return;
        }
        for i in 0..count.min(self.list_capacity()) {
            let r = self.list_item(i);
            if v.hover.is_some_and(|(x, y)| r.contains(x, y)) {
                v.rounded_asset(r.x, r.y, r.w, r.h, 4, k.subtle_hover);
            }
            let right = r.x + r.w as i32 - 12;
            match tab {
                Tab::History => {
                    let item = &self.engine.history[i];
                    let w = v
                        .measure_style(&item.expression, TextStyle::Regular14)
                        .min(r.w - 24);
                    v.text_clip_centered(
                        right - w as i32,
                        (r.y + 6, 20),
                        w + 2,
                        &item.expression,
                        TextStyle::Regular14,
                        k.secondary,
                    );
                    let result = item.result.display(true);
                    let w = v
                        .measure_style(&result, TextStyle::Semibold20)
                        .min(r.w - 24);
                    v.text_clip_centered(
                        right - w as i32,
                        (r.y + 28, 32),
                        w + 2,
                        &result,
                        TextStyle::Semibold20,
                        k.text,
                    );
                }
                Tab::Memory => {
                    let value = self.engine.memory[i].display(true);
                    let w = v.measure_style(&value, TextStyle::Semibold20).min(r.w - 24);
                    v.text_clip_centered(
                        right - w as i32,
                        (r.y + 4, 32),
                        w + 2,
                        &value,
                        TextStyle::Semibold20,
                        k.text,
                    );
                    if v.hover.is_some_and(|(x, y)| r.contains(x, y)) {
                        for (j, label) in ["MC", "M+", "M\u{2212}"].into_iter().enumerate() {
                            let b = self.memory_item_button(i, j);
                            v.rounded_asset(b.x, b.y, b.w, b.h, 4, k.operator);
                            let w = v.measure_style(label, TextStyle::Regular12);
                            v.text_clip_centered(
                                b.x + (b.w as i32 - w as i32) / 2,
                                (b.y, b.h),
                                w + 2,
                                label,
                                TextStyle::Regular12,
                                k.text,
                            );
                        }
                    }
                }
            }
        }
        let c = self.clear_rect();
        if v.hover.is_some_and(|(x, y)| c.contains(x, y)) {
            v.rounded_asset(c.x, c.y, c.w, c.h, 4, k.subtle_hover);
        }
        super::glyph(v, "calc-delete", c.x + 12, c.y + 12, 16, k.text);
    }
    fn paint_nav(&self, v: &mut View<'_>, k: &Ink) {
        let p = Self::nav_panel();
        let sheet = if v.palette.dark {
            rgb(44, 44, 44)
        } else {
            rgb(249, 249, 249)
        };
        v.elevation(p.x, p.y, p.w, p.h, 80);
        v.rounded_asset(p.x, p.y, p.w, p.h, 8, sheet);
        let n = Self::nav_rect();
        super::glyph(v, "calc-nav", n.x + 12, n.y + 12, 16, k.text);
        v.text_clip_centered(
            16,
            (TITLE + 48 - 18, 18),
            200,
            "Calculator",
            TextStyle::Semibold14,
            k.text,
        );
        let row = Rect::new(4, TITLE + 48, 248, 40);
        v.rounded_asset(row.x, row.y, row.w, row.h, 4, k.subtle_hover);
        let accent = if v.palette.dark {
            rgb(0x4c, 0xc2, 0xff)
        } else {
            rgb(0x00, 0x5f, 0xb8)
        };
        v.rounded_asset(row.x, row.y + 12, 3, 16, 1, accent);
        super::glyph(v, "calc-standard", row.x + 12, row.y + 12, 16, k.text);
        v.text_clip_centered(
            row.x + 44,
            (row.y, row.h),
            180,
            "Standard",
            TextStyle::Regular14,
            k.text,
        );
    }
}

impl Application for Calculator {
    fn title(&self) -> &str {
        "Calculator"
    }
    fn options(&self) -> door_workspace::runtime::WindowOptions {
        door_workspace::runtime::WindowOptions {
            width: 322,
            height: 500,
            caption_height: TITLE as u32,
            caption_left: 160,
            centered: true,
        }
    }
    fn caption(&self) -> (u32, u32) {
        (160, 1)
    }
    fn configure(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        if !self.ready {
            self.ready = true;
            door_rt::diagnostic(format_args!("DOOR_INBOX READY app=calculator\n"));
        }
        if self.docked() {
            self.flyout = None;
        }
    }
    fn draw(&self, v: &mut View<'_>) {
        self.paint(v);
    }
    fn target(&self, x: i32, y: i32) -> Option<u32> {
        self.hits()
            .into_iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, t)| t)
            .filter(|t| *t != u32::MAX)
    }
    fn click(&mut self, id: u32, _s: &mut Storage) -> Result<bool, Error> {
        Ok(self.activate(id))
    }
    fn key(&mut self, key: u32, _s: &mut Storage) -> Result<bool, Error> {
        let scan = key & 511;
        let control = (key >> 9) & door_abi::graphics::CONTROL != 0;
        if control && scan == 35 {
            if !self.docked() {
                self.flyout = if self.flyout.is_some() {
                    None
                } else {
                    Some(Tab::History)
                };
            }
            return Ok(true);
        }
        if scan == 1 && (self.nav || self.flyout.is_some()) {
            self.nav = false;
            self.flyout = None;
            return Ok(true);
        }
        match Self::key_command(key) {
            Some(c) => {
                self.press(c);
                Ok(true)
            }
            None => Ok(false),
        }
    }
    fn menu_open(&self) -> bool {
        self.nav || self.flyout.is_some()
    }
    fn dismiss_menu(&mut self) {
        self.nav = false;
        self.flyout = None;
    }
    fn semantics(&self, s: &mut door_workspace::semantics::Semantics, _f: &FontAtlas) {
        super::describe_calculator(self, s);
    }
}

/// For semantics and tests: (label, enabled, bounds) of every key.
pub fn key_labels(c: &Calculator) -> Vec<(String, bool, Rect)> {
    KEYS.iter()
        .enumerate()
        .map(|(i, (label, _, command))| {
            let name = match label {
                Label::Text(t) => String::from(*t),
                Label::Glyph(g) => String::from(match *g {
                    "calc-percent" => "Percent",
                    "calc-backspace" => "Backspace",
                    "calc-divide" => "Divide by",
                    "calc-multiply" => "Multiply by",
                    "calc-subtract" => "Minus",
                    "calc-add" => "Plus",
                    "calc-equals" => "Equals",
                    other => other,
                }),
                Label::Reciprocal => "Reciprocal".into(),
                Label::Square => "Square".into(),
                Label::Root => "Square root".into(),
                Label::Negate => "Positive negative".into(),
            };
            (name, c.engine.enabled(*command), c.key_rect(i))
        })
        .collect()
}
