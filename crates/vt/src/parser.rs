//! The output parser: UTF-8 text with C0 controls and the xterm/VT escape
//! sequences Windows Terminal reads, applied to a `Sink`. The parser moves
//! the sink's `Cursor` itself; the sink keeps whatever else it needs
//! (cells, scrollback, a title, replies).
use crate::cursor::{Color, Cursor, Pen, attr};

/// What a parsed program output changes besides the cursor. Every method
/// but `cursor` has a default that ignores it, so a cursor tracker only
/// keeps the cursor (and the replies it answers).
pub trait Sink {
    fn cursor(&mut self) -> &mut Cursor;
    /// Write `ch` in the pen at the cursor (always a cell on screen); in
    /// insert mode the rest of the row shifts right first.
    fn put(&mut self, _ch: char) {}
    /// Rows `top..=bottom` move up `count` rows, blank rows entering at the
    /// bottom. `keep`: a line feed pushed them out (a screen keeps the
    /// main screen's top rows as scrollback).
    fn scroll_up(&mut self, _top: usize, _bottom: usize, _count: usize, _keep: bool) {}
    fn scroll_down(&mut self, _top: usize, _bottom: usize, _count: usize) {}
    /// Blank the cells `from..to` (linear indices, clamped to the screen).
    fn erase(&mut self, _from: usize, _to: usize) {}
    /// ICH (`insert`) or DCH: `count` cells at the cursor.
    fn shift(&mut self, _insert: bool, _count: usize) {}
    /// The alternate screen is shown (blank) or the main one returns.
    fn alternate(&mut self, _on: bool) {}
    /// RIS: blank everything (the cursor is already reset).
    fn reset(&mut self) {}
    /// OSC 0 or 2: the window title.
    fn title(&mut self, _title: &str) {}
    /// A report the program asked for (DSR, DA), for its input.
    fn reply(&mut self, _bytes: &[u8]) {}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Ground,
    Escape,
    /// ESC followed by an intermediate ('(', ')', '#', ...): one more byte.
    EscapeIntermediate,
    Csi,
    Osc,
    /// ESC inside an OSC: the string terminator's second byte.
    OscEscape,
    /// DCS, SOS, PM, APC strings: ignored up to their terminator.
    Ignore,
    IgnoreEscape,
}

const MAX_PARAMS: usize = 16;
const OSC_BYTES: usize = 512;

pub struct Parser {
    state: State,
    params: [u32; MAX_PARAMS],
    count: usize,
    private: u8,
    intermediate: u8,
    osc: [u8; OSC_BYTES],
    osc_len: usize,
    utf8: [u8; 4],
    utf8_len: usize,
    utf8_need: usize,
}
impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

/// LF, IND and wraps: down a row, scrolling the region at its bottom.
fn line_feed<S: Sink>(s: &mut S) {
    let c = s.cursor();
    c.wrap_pending = false;
    if c.y == c.bottom {
        let (top, bottom) = (c.top, c.bottom);
        s.scroll_up(top, bottom, 1, true);
    } else if c.y + 1 < c.rows() {
        c.y += 1;
    }
}
fn reverse_index<S: Sink>(s: &mut S) {
    let c = s.cursor();
    c.wrap_pending = false;
    if c.y == c.top {
        let (top, bottom) = (c.top, c.bottom);
        s.scroll_down(top, bottom, 1);
    } else {
        c.y = c.y.saturating_sub(1);
    }
}
fn print<S: Sink>(s: &mut S, ch: char) {
    let c = s.cursor();
    if c.wrap_pending {
        c.wrap_pending = false;
        if c.autowrap {
            c.x = 0;
            line_feed(s);
        }
    }
    s.put(ch);
    let c = s.cursor();
    if c.x + 1 >= c.columns() {
        c.wrap_pending = true;
    } else {
        c.x += 1;
    }
}
/// Decimal digits of `n` into `out` at `at`; returns the new end.
fn digits(out: &mut [u8], mut at: usize, n: usize) -> usize {
    let mut buffer = [0u8; 20];
    let mut len = 0;
    let mut n = n;
    loop {
        buffer[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    while len > 0 {
        len -= 1;
        out[at] = buffer[len];
        at += 1;
    }
    at
}

impl Parser {
    pub const fn new() -> Self {
        Self {
            state: State::Ground,
            params: [0; MAX_PARAMS],
            count: 0,
            private: 0,
            intermediate: 0,
            osc: [0; OSC_BYTES],
            osc_len: 0,
            utf8: [0; 4],
            utf8_len: 0,
            utf8_need: 0,
        }
    }
    /// Apply the program's output.
    pub fn feed<S: Sink>(&mut self, sink: &mut S, bytes: &[u8]) {
        for &b in bytes {
            self.byte(sink, b);
        }
    }
    fn byte<S: Sink>(&mut self, s: &mut S, b: u8) {
        // A UTF-8 sequence in text.
        if self.utf8_need > 0 {
            if b & 0xC0 == 0x80 {
                self.utf8[self.utf8_len] = b;
                self.utf8_len += 1;
                if self.utf8_len == self.utf8_need {
                    let ch = core::str::from_utf8(&self.utf8[..self.utf8_len])
                        .ok()
                        .and_then(|t| t.chars().next())
                        .unwrap_or('\u{fffd}');
                    self.utf8_need = 0;
                    self.utf8_len = 0;
                    print(s, ch);
                }
                return;
            }
            self.utf8_need = 0;
            self.utf8_len = 0;
            print(s, '\u{fffd}');
        }
        match self.state {
            State::Ground => self.ground(s, b),
            State::Escape => self.escape(s, b),
            State::EscapeIntermediate => self.state = State::Ground,
            State::Csi => self.csi(s, b),
            State::Osc => match b {
                0x07 => self.osc_end(s),
                0x1b => self.state = State::OscEscape,
                0x00..=0x1f => {}
                _ => {
                    if self.osc_len < OSC_BYTES {
                        self.osc[self.osc_len] = b;
                        self.osc_len += 1;
                    }
                }
            },
            State::OscEscape => {
                if b == b'\\' {
                    self.osc_end(s);
                } else {
                    self.state = State::Escape;
                    self.escape(s, b);
                }
            }
            State::Ignore => match b {
                0x07 => self.state = State::Ground,
                0x1b => self.state = State::IgnoreEscape,
                _ => {}
            },
            State::IgnoreEscape => {
                self.state = if b == b'\\' {
                    State::Ground
                } else {
                    State::Ignore
                };
            }
        }
    }
    /// A UTF-8 lead byte (or ASCII) of text.
    fn lead<S: Sink>(&mut self, s: &mut S, b: u8) {
        let need = match b {
            0x00..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => {
                print(s, '\u{fffd}');
                return;
            }
        };
        if need == 1 {
            print(s, b as char);
        } else {
            self.utf8[0] = b;
            self.utf8_len = 1;
            self.utf8_need = need;
        }
    }
    fn ground<S: Sink>(&mut self, s: &mut S, b: u8) {
        match b {
            0x1b => {
                self.state = State::Escape;
                self.intermediate = 0;
            }
            0x08 => {
                let c = s.cursor();
                c.wrap_pending = false;
                c.x = c.x.min(c.columns() - 1).saturating_sub(1);
            }
            0x09 => {
                let c = s.cursor();
                c.wrap_pending = false;
                c.x = ((c.x / 8 + 1) * 8).min(c.columns() - 1);
            }
            0x0a..=0x0c => {
                let c = s.cursor();
                if c.newline {
                    c.x = 0;
                }
                line_feed(s);
            }
            0x0d => {
                let c = s.cursor();
                c.wrap_pending = false;
                c.x = 0;
            }
            0x00..=0x1f | 0x7f => {}
            _ => self.lead(s, b),
        }
    }
    fn escape<S: Sink>(&mut self, s: &mut S, b: u8) {
        self.state = State::Ground;
        match b {
            b'[' => {
                self.state = State::Csi;
                self.params = [0; MAX_PARAMS];
                self.count = 0;
                self.private = 0;
                self.intermediate = 0;
            }
            b']' => {
                self.state = State::Osc;
                self.osc_len = 0;
            }
            b'P' | b'X' | b'^' | b'_' => self.state = State::Ignore,
            b'(' | b')' | b'*' | b'+' | b'#' | b'%' => self.state = State::EscapeIntermediate,
            b'7' => s.cursor().save(),
            b'8' => s.cursor().restore(),
            b'D' => line_feed(s),
            b'E' => {
                s.cursor().x = 0;
                line_feed(s);
            }
            b'M' => reverse_index(s),
            b'c' => {
                s.cursor().reset();
                s.reset();
            }
            _ => {}
        }
    }
    fn param(&self, index: usize, default: u32) -> u32 {
        if index < self.count && self.params[index] != 0 {
            self.params[index]
        } else {
            default
        }
    }
    fn csi<S: Sink>(&mut self, s: &mut S, b: u8) {
        match b {
            b'0'..=b'9' => {
                if self.count == 0 {
                    self.count = 1;
                }
                let at = self.count - 1;
                self.params[at] = self.params[at]
                    .saturating_mul(10)
                    .saturating_add(u32::from(b - b'0'))
                    .min(65535);
            }
            b';' | b':' => {
                if self.count == 0 {
                    self.count = 1;
                }
                if self.count < MAX_PARAMS {
                    self.count += 1;
                }
            }
            b'?' | b'>' | b'<' | b'=' => self.private = b,
            0x20..=0x2f => self.intermediate = b,
            0x40..=0x7e => {
                self.state = State::Ground;
                self.dispatch(s, b);
            }
            0x1b => self.state = State::Escape,
            // C0 controls act inside a sequence too.
            0x08 | 0x0a..=0x0d => self.ground(s, b),
            _ => {}
        }
    }
    fn private_mode<S: Sink>(&mut self, s: &mut S, on: bool) {
        for index in 0..self.count.max(1) {
            let c = s.cursor();
            match self.params[index] {
                1 => c.application_cursor = on,
                6 => {
                    c.origin = on;
                    c.move_to(0, 0);
                }
                7 => c.autowrap = on,
                25 => c.visible = on,
                47 | 1047 | 1049 => {
                    if on != c.alternate {
                        if on {
                            c.save();
                        }
                        c.alternate = on;
                        s.alternate(on);
                        if !on {
                            s.cursor().restore();
                        }
                    }
                }
                1048 => {
                    if on {
                        c.save()
                    } else {
                        c.restore()
                    }
                }
                2004 => c.bracketed_paste = on,
                _ => {}
            }
        }
    }
    fn dispatch<S: Sink>(&mut self, s: &mut S, b: u8) {
        let p = |i: usize, d: u32| self.param(i, d) as usize;
        if self.private == b'?' {
            if matches!(b, b'h' | b'l') {
                self.private_mode(s, b == b'h');
            }
            return;
        }
        if self.private != 0 || self.intermediate != 0 {
            // DA2 (ESC [ > c) gets Windows Terminal's answer; DECSTR
            // (ESC [ ! p) a soft reset; others are ignored.
            if self.private == b'>' && b == b'c' {
                s.reply(b"\x1b[>0;10;1c");
            } else if self.intermediate == b'!' && b == b'p' {
                let c = s.cursor();
                let rows = c.rows();
                c.pen = Pen::default();
                c.visible = true;
                c.origin = false;
                c.autowrap = true;
                c.insert = false;
                c.application_cursor = false;
                c.set_margins(0, rows - 1);
            }
            return;
        }
        let c = s.cursor();
        let (columns, rows) = (c.columns(), c.rows());
        match b {
            b'A' => {
                let low = if c.y >= c.top { c.top } else { 0 };
                c.y = c.y.saturating_sub(p(0, 1)).max(low);
                c.wrap_pending = false;
            }
            b'B' | b'e' => {
                let high = if c.y <= c.bottom { c.bottom } else { rows - 1 };
                c.y = (c.y + p(0, 1)).min(high);
                c.wrap_pending = false;
            }
            b'C' | b'a' => {
                c.x = (c.x + p(0, 1)).min(columns - 1);
                c.wrap_pending = false;
            }
            b'D' => {
                c.x = c.x.min(columns - 1).saturating_sub(p(0, 1));
                c.wrap_pending = false;
            }
            b'E' => {
                c.y = (c.y + p(0, 1)).min(c.bottom.max(c.y));
                c.x = 0;
                c.wrap_pending = false;
            }
            b'F' => {
                c.y = c.y.saturating_sub(p(0, 1)).max(c.top.min(c.y));
                c.x = 0;
                c.wrap_pending = false;
            }
            b'G' | b'`' => {
                c.x = (p(0, 1) - 1).min(columns - 1);
                c.wrap_pending = false;
            }
            b'd' => {
                let x = c.x;
                c.move_to(x, p(0, 1) - 1);
            }
            b'H' | b'f' => c.move_to(p(1, 1) - 1, p(0, 1) - 1),
            b'J' => {
                let at = c.at();
                let total = columns * rows;
                match self.param(0, 0) {
                    0 => s.erase(at, total),
                    1 => s.erase(0, at + 1),
                    2 | 3 => s.erase(0, total),
                    _ => {}
                }
            }
            b'K' => {
                let start = c.y * columns;
                let at = c.at();
                match self.param(0, 0) {
                    0 => s.erase(at, start + columns),
                    1 => s.erase(start, at + 1),
                    2 => s.erase(start, start + columns),
                    _ => {}
                }
            }
            b'X' => {
                let at = c.at();
                let end = c.y * columns + columns;
                s.erase(at, (at + p(0, 1)).min(end));
            }
            b'@' => s.shift(true, p(0, 1)),
            b'P' => s.shift(false, p(0, 1)),
            b'L' | b'M' => {
                if c.y >= c.top && c.y <= c.bottom {
                    let (y, bottom) = (c.y, c.bottom);
                    c.x = 0;
                    c.wrap_pending = false;
                    if b == b'L' {
                        s.scroll_down(y, bottom, p(0, 1));
                    } else {
                        s.scroll_up(y, bottom, p(0, 1), false);
                    }
                }
            }
            b'S' => {
                let (top, bottom) = (c.top, c.bottom);
                s.scroll_up(top, bottom, p(0, 1), false);
            }
            b'T' => {
                let (top, bottom) = (c.top, c.bottom);
                s.scroll_down(top, bottom, p(0, 1));
            }
            b'r' => c.set_margins(p(0, 1) - 1, p(1, rows as u32) - 1),
            b's' => c.save(),
            b'u' => c.restore(),
            b'h' | b'l' => match self.param(0, 0) {
                4 => c.insert = b == b'h',
                20 => c.newline = b == b'h',
                _ => {}
            },
            b'm' => self.sgr(&mut c.pen),
            b'n' => match self.param(0, 0) {
                5 => s.reply(b"\x1b[0n"),
                6 => {
                    let (x, y) = c.position();
                    let y = if c.origin { y - c.top } else { y };
                    let mut reply = [0u8; 48];
                    reply[..2].copy_from_slice(b"\x1b[");
                    let mut at = digits(&mut reply, 2, y + 1);
                    reply[at] = b';';
                    at = digits(&mut reply, at + 1, x + 1);
                    reply[at] = b'R';
                    s.reply(&reply[..at + 1]);
                }
                _ => {}
            },
            // DA1: a VT100 with advanced video, as Windows Terminal answers.
            b'c' => s.reply(b"\x1b[?1;0c"),
            _ => {}
        }
    }
    fn sgr(&self, pen: &mut Pen) {
        let count = self.count.max(1);
        let mut i = 0;
        while i < count {
            let v = self.params[i];
            match v {
                0 => *pen = Pen::default(),
                1 => pen.attrs |= attr::BOLD,
                2 => pen.attrs |= attr::DIM,
                3 => pen.attrs |= attr::ITALIC,
                4 => pen.attrs |= attr::UNDERLINE,
                5 | 6 => pen.attrs |= attr::BLINK,
                7 => pen.attrs |= attr::INVERSE,
                8 => pen.attrs |= attr::HIDDEN,
                9 => pen.attrs |= attr::STRIKE,
                21 => pen.attrs |= attr::DOUBLE_UNDERLINE,
                22 => pen.attrs &= !(attr::BOLD | attr::DIM),
                23 => pen.attrs &= !attr::ITALIC,
                24 => pen.attrs &= !(attr::UNDERLINE | attr::DOUBLE_UNDERLINE),
                25 => pen.attrs &= !attr::BLINK,
                27 => pen.attrs &= !attr::INVERSE,
                28 => pen.attrs &= !attr::HIDDEN,
                29 => pen.attrs &= !attr::STRIKE,
                30..=37 => pen.fg = Color::Indexed((v - 30) as u8),
                39 => pen.fg = Color::Default,
                40..=47 => pen.bg = Color::Indexed((v - 40) as u8),
                49 => pen.bg = Color::Default,
                90..=97 => pen.fg = Color::Indexed((v - 90 + 8) as u8),
                100..=107 => pen.bg = Color::Indexed((v - 100 + 8) as u8),
                38 | 48 => {
                    let color = match self.params.get(i + 1) {
                        Some(5) if i + 2 < count => {
                            i += 2;
                            Some(Color::Indexed(self.params[i].min(255) as u8))
                        }
                        Some(2) if i + 4 < count => {
                            let c = Color::Rgb(
                                self.params[i + 2].min(255) as u8,
                                self.params[i + 3].min(255) as u8,
                                self.params[i + 4].min(255) as u8,
                            );
                            i += 4;
                            Some(c)
                        }
                        _ => None,
                    };
                    if let Some(c) = color {
                        if v == 38 {
                            pen.fg = c;
                        } else {
                            pen.bg = c;
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    fn osc_end<S: Sink>(&mut self, s: &mut S) {
        self.state = State::Ground;
        let text = core::str::from_utf8(&self.osc[..self.osc_len]).unwrap_or("");
        if let Some((kind, value)) = text.split_once(';')
            && matches!(kind, "0" | "2")
        {
            s.title(value);
        }
        self.osc_len = 0;
    }
}
