//! Notes' Windows 11 26H2 Notepad chrome and its lightweight formatting.
//!
//! Geometry is in logical px, measured on the owner's dark Notepad at 150 %
//! (`reports/artifacts/reference-26h2/notepad-window.uia.tsv` and the owner's
//! screenshot; docs/design/fidelity-26h2-diff.md N1-N3): a 41 px tab row
//! that is the title bar, one 32 px menu + formatting band, the editor
//! under a 1 px separator, and a 32 px status bar. Formatting is Markdown
//! source kept as plain text in the file: [`parse_line`] finds the markup a
//! painter shows in Formatted mode, and the editing helpers produce the text
//! the toolbar inserts. Everything here is pure, so host tests pin it.
use crate::explorer_layout::Bounds;
use alloc::{format, string::String, vec::Vec};
use core::ops::Range;

/// Tab row / title bar height (62 px at 150 %).
pub const CAPTION: u32 = 41;
/// The app icon, 16 px, left of the tabs.
pub const ICON: (i32, i32) = (16, 17);
/// First tab: x 44, y 9, at most 246 x 32 (it joins the menu band below).
pub const TAB_LEFT: i32 = 44;
pub const TAB_TOP: i32 = 9;
pub const TAB_WIDTH: u32 = 246;
pub const TAB_HEIGHT: u32 = 32;
/// Space between neighbouring tabs.
pub const TAB_GAP: u32 = 2;
/// Tab label inset from the tab's left edge.
pub const TAB_TEXT: i32 = 13;
/// The close button (an unsaved dot until hovered): 32 x 24 at y 13, its
/// left edge 41 px before the tab's right edge.
pub const CLOSE: (i32, u32, u32) = (13, 32, 24);
pub const CLOSE_FROM_RIGHT: i32 = 41;
/// "+": 32 x 24 at y 15, 3 px after the last tab.
pub const PLUS: (i32, u32, u32) = (15, 32, 24);
/// Right of this many px the title bar holds the trusted window controls.
pub const SYSTEM_BUTTONS: u32 = 144;
/// Menu + formatting band (41..73), its separator and the editor top.
pub const BAND_TOP: i32 = 41;
pub const BAND_HEIGHT: u32 = 32;
pub const SEPARATOR: i32 = 73;
pub const BODY_TOP: i32 = 74;
/// Menu bar items: from x 4, label + 10 px each side, 8 px apart.
pub const MENU_LEFT: i32 = 4;
pub const MENU_PADDING: u32 = 10;
pub const MENU_GAP: i32 = 8;
/// The document text: Windows' 16 px padding at both sides, and the first
/// line's top, where Door's 20 px rows centre the capitals on Notepad's
/// (cap band 94..104 in the owner's screenshot).
pub const TEXT_LEFT: i32 = 16;
pub const TEXT_TOP: i32 = 89;
/// Status bar height.
pub const STATUS: u32 = 32;
/// Toolbar glyphs are 16 px; drop-down chevrons 12 px.
pub const GLYPH: u32 = 16;

/// The centred formatting toolbar, left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Headings,
    Lists,
    Bold,
    Italic,
    Strike,
    Link,
    Table,
    Clear,
}
impl Tool {
    pub const ALL: [Tool; 8] = [
        Tool::Headings,
        Tool::Lists,
        Tool::Bold,
        Tool::Italic,
        Tool::Strike,
        Tool::Link,
        Tool::Table,
        Tool::Clear,
    ];
    /// Drop-down buttons are 56 px (glyph and chevron), toggles 32 px.
    pub fn width(self) -> u32 {
        if self.dropdown() { 56 } else { 32 }
    }
    pub fn dropdown(self) -> bool {
        matches!(self, Tool::Headings | Tool::Lists | Tool::Table)
    }
    /// Accessible name, with the Windows shortcut.
    pub fn name(self) -> &'static str {
        match self {
            Tool::Headings => "Headings",
            Tool::Lists => "Lists",
            Tool::Bold => "Bold (Ctrl+B)",
            Tool::Italic => "Italic (Ctrl+I)",
            Tool::Strike => "Strikethrough (Ctrl+Shift+X)",
            Tool::Link => "Link (Ctrl+K)",
            Tool::Table => "Table",
            Tool::Clear => "Clear formatting (Ctrl+Space)",
        }
    }
}
/// Total toolbar width (328 px).
pub fn toolbar_width() -> u32 {
    Tool::ALL.iter().map(|t| t.width()).sum()
}

/// One window's chrome geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chrome {
    pub width: u32,
    pub height: u32,
    pub tabs: usize,
    pub active: usize,
}
impl Chrome {
    /// Width the tabs share: up to the "+" and the window controls.
    fn tab_space(self) -> u32 {
        self.width
            .saturating_sub(TAB_LEFT as u32 + 3 + PLUS.1 + SYSTEM_BUTTONS)
    }
    /// Tabs shown at once (each at least 96 px, as Explorer's strip).
    pub fn capacity(self) -> usize {
        (self.tab_space() / 96).max(1) as usize
    }
    pub fn start(self) -> usize {
        self.active
            .min(self.tabs.saturating_sub(1))
            .saturating_sub(self.capacity() - 1)
    }
    pub fn range(self) -> Range<usize> {
        let s = self.start();
        s..(s + self.capacity()).min(self.tabs)
    }
    pub fn tab(self, index: usize) -> Option<Bounds> {
        if !self.range().contains(&index) {
            return None;
        }
        let shown = self.range().len().max(1) as u32;
        let stride = (self.tab_space() / shown).min(TAB_WIDTH + TAB_GAP);
        (stride >= 64).then(|| Bounds {
            x: TAB_LEFT + (index - self.start()) as i32 * stride as i32,
            y: TAB_TOP,
            width: stride - TAB_GAP,
            height: TAB_HEIGHT,
        })
    }
    pub fn close(self, index: usize) -> Option<Bounds> {
        let r = self.tab(index)?;
        Some(Bounds {
            x: r.x + r.width as i32 - CLOSE_FROM_RIGHT,
            y: CLOSE.0,
            width: CLOSE.1,
            height: CLOSE.2,
        })
    }
    pub fn plus(self) -> Option<Bounds> {
        let x = self
            .range()
            .filter_map(|i| self.tab(i))
            .map(|r| r.x + r.width as i32 + 3)
            .max()
            .unwrap_or(TAB_LEFT);
        (x + PLUS.1 as i32 <= self.width as i32 - SYSTEM_BUTTONS as i32).then_some(Bounds {
            x,
            y: PLUS.0,
            width: PLUS.1,
            height: PLUS.2,
        })
    }
    /// The client-owned title bar: the first tab through "+".
    pub fn caption_width(self) -> u32 {
        self.plus()
            .map(|r| r.x + r.width as i32)
            .or_else(|| {
                self.range()
                    .filter_map(|i| self.tab(i))
                    .map(|r| r.x + r.width as i32)
                    .max()
            })
            .map_or(1, |right| (right - TAB_LEFT).max(1) as u32)
    }
    /// File, Edit and View from their measured label advances.
    pub fn menu(self, labels: [u32; 3]) -> [Bounds; 3] {
        let mut x = MENU_LEFT;
        core::array::from_fn(|i| {
            let width = labels[i] + 2 * MENU_PADDING;
            let r = Bounds {
                x,
                y: BAND_TOP,
                width,
                height: BAND_HEIGHT,
            };
            x += width as i32 + MENU_GAP;
            r
        })
    }
    /// What's new (30 x 30) and Settings (30 x 32) at the band's right end.
    pub fn whats_new(self) -> Bounds {
        Bounds {
            x: self.width as i32 - 72,
            y: BAND_TOP + 2,
            width: 30,
            height: 30,
        }
    }
    pub fn settings(self) -> Bounds {
        Bounds {
            x: self.width as i32 - 38,
            y: BAND_TOP,
            width: 30,
            height: BAND_HEIGHT,
        }
    }
    /// A toolbar button, centred in the window, after the menu bar (163)
    /// and before What's new; buttons that do not fit are left out.
    pub fn tool(self, tool: Tool) -> Option<Bounds> {
        let left = ((self.width as i32 - toolbar_width() as i32) / 2).max(163);
        let mut x = left;
        for t in Tool::ALL {
            if t == tool {
                return (x + t.width() as i32 <= self.width as i32 - 80).then_some(Bounds {
                    x,
                    y: BAND_TOP,
                    width: t.width(),
                    height: BAND_HEIGHT,
                });
            }
            x += t.width() as i32;
        }
        None
    }
    /// A button's glyph origin (16 px) and, for drop-downs, its chevron's (12 px).
    pub fn glyph(tool: Tool, r: Bounds) -> ((i32, i32), Option<(i32, i32)>) {
        let y = r.y + (r.height as i32 - GLYPH as i32) / 2;
        if tool.dropdown() {
            (
                (r.x + 10, y),
                Some((r.x + 33, r.y + (r.height as i32 - 12) / 2)),
            )
        } else {
            ((r.x + (r.width as i32 - GLYPH as i32) / 2, y), None)
        }
    }
    pub fn status_top(self) -> i32 {
        self.height as i32 - STATUS as i32
    }
    /// The text area: 16 px padding at both sides, from the first line's
    /// top down to the status bar (or the window bottom without it).
    pub fn body(self, status: bool) -> Bounds {
        let bottom = if status {
            self.status_top()
        } else {
            self.height as i32
        };
        Bounds {
            x: TEXT_LEFT,
            y: TEXT_TOP,
            width: self.width.saturating_sub(2 * TEXT_LEFT as u32),
            height: (bottom - TEXT_TOP).max(0) as u32,
        }
    }
    /// Status bar columns, measured on the owner's screenshot: text x and
    /// the 1 x 16 px separator before it. Left: position, character count
    /// and the Formatted / Plain text toggle ("Aa" glyph, then its label);
    /// right-aligned: zoom, line ending and encoding. Columns that would
    /// collide at narrow widths are `None`.
    pub fn status_columns(self) -> StatusColumns {
        let w = self.width as i32;
        StatusColumns {
            position: 22,
            characters: (self.width >= 480).then_some((105, 96)),
            format: (self.width >= 640).then_some((313, 333, 304)),
            zoom: (self.width >= 800).then_some((w - 368, w - 380)),
            eol: (self.width >= 800).then_some((w - 299, w - 312)),
            encoding: (w - 131, w - 144),
        }
    }
    /// The Formatted / Plain text toggle in the status bar.
    pub fn format_toggle(self) -> Option<Bounds> {
        self.status_columns()
            .format
            .map(|(_, _, separator)| Bounds {
                x: separator + 1,
                y: self.status_top(),
                width: 120,
                height: STATUS,
            })
    }
}
/// Status bar x positions (see [`Chrome::status_columns`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusColumns {
    pub position: i32,
    pub characters: Option<(i32, i32)>,
    /// Glyph x, label x and separator x.
    pub format: Option<(i32, i32, i32)>,
    pub zoom: Option<(i32, i32)>,
    pub eol: Option<(i32, i32)>,
    pub encoding: (i32, i32),
}

/// What a line is in Formatted mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    Plain,
    /// `#` .. `######` then a space.
    Heading(u8),
    /// `- `, `* ` or `+ `; the byte of the marker.
    Bullet(usize),
    /// `1. ` and so on.
    Numbered,
    /// A `| a | b |` row.
    TableRow,
    /// A `| --- | --- |` rule under a table header.
    TableRule,
}
/// How a run of a line is shown in Formatted mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// Markup characters, shown dimmed.
    Marker,
    Heading,
    Bold,
    Italic,
    Strike,
    /// Link text, shown as a link.
    Link,
    /// A link's address, shown dimmed.
    Url,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub mark: Mark,
}
fn span(range: Range<usize>, mark: Mark) -> Span {
    Span { range, mark }
}
/// Lines longer than this are shown as plain text (bounded per frame).
pub const MAX_FORMATTED_LINE: usize = 4096;

/// The block kind and inline runs of one line (without its line ending).
pub fn parse_line(line: &str) -> (Block, Vec<Span>) {
    let mut spans = Vec::new();
    if line.len() > MAX_FORMATTED_LINE {
        return (Block::Plain, spans);
    }
    let bytes = line.as_bytes();
    let indent = bytes.iter().take_while(|b| **b == b' ').count();
    let rest = &line[indent..];
    let hashes = rest.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&hashes) && rest.as_bytes().get(hashes) == Some(&b' ') {
        let body = indent + hashes + 1;
        spans.push(span(indent..body, Mark::Marker));
        inline(line, body, &mut spans);
        if body < line.len() {
            spans.push(span(body..line.len(), Mark::Heading));
        }
        return (Block::Heading(hashes as u8), spans);
    }
    if is_table_rule(rest) {
        return (Block::TableRule, spans);
    }
    if rest.starts_with('|') {
        for (i, b) in bytes.iter().enumerate() {
            if *b == b'|' {
                spans.push(span(i..i + 1, Mark::Marker));
            }
        }
        inline(line, indent, &mut spans);
        return (Block::TableRow, spans);
    }
    if matches!(rest.as_bytes(), [b'-' | b'*' | b'+', b' ', ..]) {
        inline(line, indent + 2, &mut spans);
        return (Block::Bullet(indent), spans);
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=9).contains(&digits) && rest.as_bytes()[digits..].starts_with(b". ") {
        inline(line, indent + digits + 2, &mut spans);
        return (Block::Numbered, spans);
    }
    inline(line, 0, &mut spans);
    (Block::Plain, spans)
}
fn is_table_rule(text: &str) -> bool {
    let t = text.trim_end();
    if !t.contains("---") || !t.contains('|') {
        return false;
    }
    t.split('|')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .all(|c| {
            let c = c.trim_start_matches(':').trim_end_matches(':');
            c.len() >= 3 && c.bytes().all(|b| b == b'-')
        })
}
/// `**bold**`, `~~strike~~`, `*italic*` / `_italic_` and `[text](url)`
/// from `from` on. Unclosed markers stay plain text.
fn inline(line: &str, from: usize, spans: &mut Vec<Span>) {
    let b = line.as_bytes();
    let mut i = from;
    while i < b.len() {
        let pair = |m: &[u8]| b[i..].starts_with(m);
        let closing = |m: &str, start: usize| line[start..].find(m).map(|at| start + at);
        if pair(b"**") || pair(b"~~") {
            let m = &line[i..i + 2];
            if let Some(end) = closing(m, i + 2).filter(|e| *e > i + 2) {
                let mark = if m == "**" { Mark::Bold } else { Mark::Strike };
                spans.push(span(i..i + 2, Mark::Marker));
                spans.push(span(i + 2..end, mark));
                spans.push(span(end..end + 2, Mark::Marker));
                i = end + 2;
                continue;
            }
        } else if (b[i] == b'*' || b[i] == b'_')
            && b.get(i + 1).is_some_and(|c| !c.is_ascii_whitespace())
        {
            let m = &line[i..i + 1];
            if let Some(end) = closing(m, i + 1).filter(|e| *e > i + 1) {
                spans.push(span(i..i + 1, Mark::Marker));
                spans.push(span(i + 1..end, Mark::Italic));
                spans.push(span(end..end + 1, Mark::Marker));
                i = end + 1;
                continue;
            }
        } else if b[i] == b'['
            && let Some(close) = closing("](", i + 1)
            && let Some(end) = closing(")", close + 2)
        {
            spans.push(span(i..i + 1, Mark::Marker));
            spans.push(span(i + 1..close, Mark::Link));
            spans.push(span(close..close + 2, Mark::Marker));
            spans.push(span(close + 2..end, Mark::Url));
            spans.push(span(end..end + 1, Mark::Marker));
            i = end + 1;
            continue;
        }
        i += line[i..].chars().next().map_or(1, char::len_utf8);
    }
}

/// The line with its heading set to `level` (0: body text).
pub fn set_heading(line: &str, level: u8) -> String {
    let indent = line.bytes().take_while(|b| *b == b' ').count();
    let rest = &line[indent..];
    let hashes = rest.bytes().take_while(|b| *b == b'#').count();
    let body = if hashes > 0 && rest.as_bytes().get(hashes) == Some(&b' ') {
        &rest[hashes + 1..]
    } else {
        rest
    };
    let prefix = "#".repeat(level.min(6) as usize);
    if level == 0 {
        format!("{}{body}", &line[..indent])
    } else {
        format!("{}{prefix} {body}", &line[..indent])
    }
}
/// The line made a bulleted (`- `) or numbered (`n. `) item, or back to
/// plain text when it already is that kind of item.
pub fn toggle_list(line: &str, numbered: bool, number: usize) -> String {
    let indent = line.bytes().take_while(|b| *b == b' ').count();
    let rest = &line[indent..];
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let (was_bullet, was_number) = (
        matches!(rest.as_bytes(), [b'-' | b'*' | b'+', b' ', ..]),
        digits > 0 && rest.as_bytes()[digits..].starts_with(b". "),
    );
    let body = if was_bullet {
        &rest[2..]
    } else if was_number {
        &rest[digits + 2..]
    } else {
        rest
    };
    let pad = &line[..indent];
    match (numbered, was_bullet, was_number) {
        (false, true, _) | (true, _, true) => format!("{pad}{body}"),
        (false, ..) => format!("{pad}- {body}"),
        (true, ..) => format!("{pad}{number}. {body}"),
    }
}
/// `selected` wrapped in `mark` (Bold `**`, Italic `*`, Strikethrough `~~`),
/// or unwrapped when it already is. Returns the text and the inner range.
pub fn toggle_wrap(selected: &str, mark: &str) -> (String, Range<usize>) {
    if selected.len() >= 2 * mark.len() && selected.starts_with(mark) && selected.ends_with(mark) {
        let inner = &selected[mark.len()..selected.len() - mark.len()];
        return (String::from(inner), 0..inner.len());
    }
    let n = mark.len();
    (format!("{mark}{selected}{mark}"), n..n + selected.len())
}
/// A link around `selected`; the returned range is the address to type over.
pub fn link(selected: &str) -> (String, Range<usize>) {
    let text = if selected.is_empty() {
        "link"
    } else {
        selected
    };
    let at = text.len() + 3;
    (format!("[{text}](https://)"), at..at + 8)
}
/// A `columns` x `rows` table (header, rule and empty body rows) using
/// `eol`; the returned range is the first header cell to type over.
pub fn table(columns: usize, rows: usize, eol: &str) -> (String, Range<usize>) {
    let columns = columns.clamp(1, 8);
    let mut out = String::new();
    for c in 0..columns {
        out.push_str(&format!("| Column {} ", c + 1));
    }
    out.push('|');
    out.push_str(eol);
    out.push_str(&"| --- ".repeat(columns));
    out.push('|');
    out.push_str(eol);
    for _ in 0..rows.saturating_sub(1).max(1) {
        out.push_str(&"|  ".repeat(columns));
        out.push('|');
        out.push_str(eol);
    }
    (out, 2..10)
}
/// `text` without any formatting markup (headings, list markers, bold,
/// italic, strikethrough, links keep their text); other text is unchanged.
pub fn clear(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let end = rest.find('\n').map_or(rest.len(), |i| i + 1);
        let (line, next) = rest.split_at(end);
        let body = line.trim_end_matches(['\r', '\n']);
        let ending = &line[body.len()..];
        let (block, spans) = parse_line(body);
        let mut skip: Vec<Range<usize>> = spans
            .iter()
            .filter(|s| matches!(s.mark, Mark::Marker | Mark::Url))
            .map(|s| s.range.clone())
            .collect();
        match block {
            Block::Bullet(at) => skip.push(at..at + 2),
            Block::Numbered => {
                let indent = body.bytes().take_while(|b| *b == b' ').count();
                let digits = body[indent..]
                    .bytes()
                    .take_while(u8::is_ascii_digit)
                    .count();
                skip.push(indent..indent + digits + 2);
            }
            // Table pipes and rules are structure, not formatting.
            Block::TableRow | Block::TableRule => skip.clear(),
            _ => (),
        }
        for (i, c) in body.char_indices() {
            if !skip.iter().any(|r| r.contains(&i)) {
                out.push(c);
            }
        }
        out.push_str(ending);
        rest = next;
    }
    out
}

/// A stroke from one point to another on the 16-unit glyph grid.
pub type Segment = ((f32, f32), (f32, f32));
/// An anti-aliased stroked glyph (8-bit coverage, `size` x `size` px at
/// `scale`), drawn on a 16-unit grid from line segments and circles: the
/// toolbar's table, clear-formatting and What's new glyphs, in Segoe
/// Fluent's 1-unit outline style.
pub fn stroke_glyph(scale: u32, segments: &[Segment], dots: &[(f32, f32, f32)]) -> Vec<u8> {
    let side = (GLYPH * scale.max(1)) as usize;
    let unit = side as f32 / 16.0;
    let half = 0.5;
    let mut mask = alloc::vec![0u8; side * side];
    for y in 0..side {
        for x in 0..side {
            let p = ((x as f32 + 0.5) / unit, (y as f32 + 0.5) / unit);
            let mut d = f32::MAX;
            for &((ax, ay), (bx, by)) in segments {
                let (vx, vy) = (bx - ax, by - ay);
                let len = vx * vx + vy * vy;
                let t = if len == 0.0 {
                    0.0
                } else {
                    (((p.0 - ax) * vx + (p.1 - ay) * vy) / len).clamp(0.0, 1.0)
                };
                let (dx, dy) = (p.0 - (ax + t * vx), p.1 - (ay + t * vy));
                d = d.min(sqrt(dx * dx + dy * dy) - half);
            }
            for &(cx, cy, r) in dots {
                let (dx, dy) = (p.0 - cx, p.1 - cy);
                d = d.min(sqrt(dx * dx + dy * dy) - r);
            }
            // Coverage over one device pixel.
            let c = (0.5 - d * unit).clamp(0.0, 1.0);
            mask[y * side + x] = (c * 255.0 + 0.5) as u8;
        }
    }
    mask
}
fn sqrt(v: f32) -> f32 {
    if v <= 0.0 {
        return 0.0;
    }
    let mut x = if v > 1.0 { v } else { 1.0 };
    for _ in 0..12 {
        x = 0.5 * (x + v / x);
    }
    x
}
/// Segoe Fluent-style outline glyphs the shell glyph set lacks.
pub fn table_glyph(scale: u32) -> Vec<u8> {
    let r = [
        ((2.5, 2.5), (13.5, 2.5)),
        ((13.5, 2.5), (13.5, 13.5)),
        ((13.5, 13.5), (2.5, 13.5)),
        ((2.5, 13.5), (2.5, 2.5)),
        ((2.5, 6.5), (13.5, 6.5)),
        ((2.5, 10.0), (13.5, 10.0)),
        ((7.0, 2.5), (7.0, 13.5)),
    ];
    stroke_glyph(scale, &r, &[])
}
pub fn clear_glyph(scale: u32) -> Vec<u8> {
    // An "A" with an eraser stroke across its lower right.
    let r = [
        ((2.0, 12.5), (6.0, 2.5)),
        ((6.0, 2.5), (10.0, 12.5)),
        ((3.6, 8.8), (8.4, 8.8)),
        ((9.0, 15.0), (14.5, 9.5)),
        ((11.0, 9.5), (14.5, 13.0)),
    ];
    stroke_glyph(scale, &r, &[])
}
pub fn megaphone_glyph(scale: u32) -> Vec<u8> {
    let r = [
        ((2.0, 6.0), (2.0, 10.0)),
        ((2.0, 6.0), (5.0, 6.0)),
        ((2.0, 10.0), (5.0, 10.0)),
        ((5.0, 6.0), (13.0, 2.5)),
        ((5.0, 10.0), (13.0, 13.5)),
        ((13.0, 2.5), (13.0, 13.5)),
        ((5.5, 10.2), (6.8, 14.0)),
        ((6.8, 14.0), (9.0, 14.0)),
        ((9.0, 14.0), (8.6, 11.8)),
    ];
    stroke_glyph(scale, &r, &[])
}
/// `mask` (`from` x `from` coverage) resampled to `to` x `to` by area:
/// the 20 px shell glyphs drawn at the toolbar's 16 px.
pub fn resample(mask: &[u8], from: usize, to: usize) -> Vec<u8> {
    let mut out = alloc::vec![0u8; to * to];
    if from == 0 || to == 0 || mask.len() < from * from {
        return out;
    }
    for y in 0..to {
        for x in 0..to {
            // Source box [x0, x1) x [y0, y1) in source pixels.
            let (x0, x1) = (x * from, (x + 1) * from);
            let (y0, y1) = (y * from, (y + 1) * from);
            let mut sum = 0u64;
            for sy in y0 / to..y1.div_ceil(to) {
                let wy = (y1.min((sy + 1) * to) - y0.max(sy * to)) as u64;
                for sx in x0 / to..x1.div_ceil(to) {
                    let wx = (x1.min((sx + 1) * to) - x0.max(sx * to)) as u64;
                    sum += mask[sy * from + sx] as u64 * wx * wy;
                }
            }
            out[y * to + x] = ((sum + (from * from) as u64 / 2) / (from * from) as u64) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rect(b: Bounds) -> (i32, i32, u32, u32) {
        (b.x, b.y, b.width, b.height)
    }
    /// The owner's Notepad: a 1268 px client with one tab.
    #[test]
    fn chrome_matches_the_26h2_notepad_reference() {
        let c = Chrome {
            width: 1268,
            height: 737,
            tabs: 1,
            active: 0,
        };
        assert_eq!(CAPTION, 41);
        assert_eq!(rect(c.tab(0).unwrap()), (44, 9, 246, 32));
        assert_eq!(rect(c.close(0).unwrap()), (249, 13, 32, 24));
        assert_eq!(rect(c.plus().unwrap()), (293, 15, 32, 24));
        assert_eq!(c.caption_width(), 281);
        // File 41, Edit 44, View 50 px wide from the measured labels.
        let m = c.menu([21, 24, 30]);
        assert_eq!(
            m.map(rect),
            [(4, 41, 41, 32), (53, 41, 44, 32), (105, 41, 50, 32)]
        );
        assert_eq!(toolbar_width(), 328);
        let x: Vec<i32> = Tool::ALL.iter().map(|t| c.tool(*t).unwrap().x).collect();
        assert_eq!(x, [470, 526, 582, 614, 646, 678, 710, 766]);
        assert_eq!(
            Chrome::glyph(Tool::Headings, c.tool(Tool::Headings).unwrap()),
            ((480, 49), Some((503, 51)))
        );
        assert_eq!(
            Chrome::glyph(Tool::Bold, c.tool(Tool::Bold).unwrap()).0,
            (590, 49)
        );
        assert_eq!(rect(c.whats_new()), (1196, 43, 30, 30));
        assert_eq!(rect(c.settings()), (1230, 41, 30, 32));
        assert_eq!((SEPARATOR, BODY_TOP, TEXT_LEFT, TEXT_TOP), (73, 74, 16, 89));
        assert_eq!(rect(c.body(true)), (16, 89, 1236, 616));
        assert_eq!(rect(c.body(false)), (16, 89, 1236, 648));
        assert_eq!(c.status_top(), 705);
        // Screenshot ink (x / 1.5): "Ln" 23.3, "25" 106, "Aa" 313.3,
        // "Formatted" 334, "100%" 900.7, "Windows" 968.7, "UTF-8" 1137.3;
        // separators 96, 304, 888, 956 and 1124.
        let s = c.status_columns();
        assert_eq!(
            (s.position, s.characters, s.format),
            (22, Some((105, 96)), Some((313, 333, 304)))
        );
        assert_eq!(
            (s.zoom, s.eol, s.encoding),
            (Some((900, 888)), Some((969, 956)), (1137, 1124))
        );
        assert_eq!(rect(c.format_toggle().unwrap()), (305, 705, 120, 32));
        // Narrow windows drop the middle columns before they collide.
        for width in [240, 479, 639, 799] {
            let s = Chrome { width, ..c }.status_columns();
            let mut right = s.encoding.1;
            for (x, separator) in [s.eol, s.zoom].into_iter().flatten() {
                assert!(x < right && separator < x);
                right = separator;
            }
            let left = [s.characters.map(|c| c.0 + 90), s.format.map(|f| f.1 + 70)]
                .into_iter()
                .flatten()
                .max()
                .unwrap_or(s.position + 70);
            assert!(left < right, "{width}: {left} {right}");
        }
    }
    #[test]
    fn tabs_plus_and_toolbar_never_reach_the_window_controls() {
        for width in [240, 490, 742, 896, 1268, 1920] {
            for tabs in 1..=8 {
                let c = Chrome {
                    width,
                    height: 500,
                    tabs,
                    active: tabs - 1,
                };
                for i in c.range() {
                    if let Some(t) = c.tab(i) {
                        assert!(t.x >= TAB_LEFT && t.x + t.width as i32 <= width as i32 - 144);
                        let x = c.close(i).unwrap();
                        assert!(x.x > t.x && x.x + x.width as i32 <= t.x + t.width as i32);
                    }
                }
                assert!(TAB_LEFT as u32 + c.caption_width() <= width.saturating_sub(138).max(45));
                for t in Tool::ALL {
                    if let Some(r) = c.tool(t) {
                        assert!(r.x >= 163 && r.x + r.width as i32 <= width as i32 - 80);
                    }
                }
            }
        }
    }
    #[test]
    fn markup_is_found_per_line() {
        let (b, s) = parse_line("## Plan **now**");
        assert_eq!(b, Block::Heading(2));
        assert!(s.contains(&span(0..3, Mark::Marker)));
        assert!(s.contains(&span(10..13, Mark::Bold)));
        assert!(s.contains(&span(3..15, Mark::Heading)));
        let (b, s) = parse_line("- a ~~b~~ *c* _d_ [e](f)");
        assert_eq!(b, Block::Bullet(0));
        for (r, m) in [
            (6..7, Mark::Strike),
            (11..12, Mark::Italic),
            (15..16, Mark::Italic),
            (19..20, Mark::Link),
            (22..23, Mark::Url),
        ] {
            assert!(s.contains(&span(r.clone(), m)), "{r:?} {m:?} in {s:?}");
        }
        assert_eq!(parse_line("12. x").0, Block::Numbered);
        assert_eq!(parse_line("| a | b |").0, Block::TableRow);
        assert_eq!(parse_line("| --- | :---: |").0, Block::TableRule);
        // Unclosed or spaced markers, hashes without a space: plain text.
        for plain in ["#tag", "a * b", "**open", "[x](y", "2*3*4 = 24"] {
            let (b, s) = parse_line(plain);
            assert!(
                b == Block::Plain && s.iter().all(|s| s.mark != Mark::Bold),
                "{plain}: {b:?} {s:?}"
            );
        }
        assert_eq!(parse_line(&"*".repeat(MAX_FORMATTED_LINE + 1)).1, []);
    }
    #[test]
    fn toolbar_edits_round_trip_as_plain_text() {
        assert_eq!(set_heading("Title", 1), "# Title");
        assert_eq!(set_heading("# Title", 3), "### Title");
        assert_eq!(set_heading("### Title", 0), "Title");
        assert_eq!(toggle_list("item", false, 1), "- item");
        assert_eq!(toggle_list("- item", false, 1), "item");
        assert_eq!(toggle_list("- item", true, 3), "3. item");
        assert_eq!(toggle_list("3. item", true, 3), "item");
        assert_eq!(toggle_wrap("word", "**"), ("**word**".into(), 2..6));
        assert_eq!(toggle_wrap("**word**", "**"), ("word".into(), 0..4));
        assert_eq!(toggle_wrap("", "~~"), ("~~~~".into(), 2..2));
        assert_eq!(link("Door"), ("[Door](https://)".into(), 7..15));
        let (t, first) = table(2, 2, "\r\n");
        assert_eq!(t, "| Column 1 | Column 2 |\r\n| --- | --- |\r\n|  |  |\r\n");
        assert_eq!(&t[first], "Column 1");
        assert_eq!(parse_line("| --- | --- |").0, Block::TableRule);
        assert_eq!(
            clear("# **Bold** and [link](u)\r\n- ~~gone~~ *it*\n| a | b |"),
            "Bold and link\r\ngone it\n| a | b |"
        );
    }
    #[test]
    fn glyphs_have_ink_inside_their_box_at_every_density() {
        for scale in 1..=3 {
            for mask in [
                table_glyph(scale),
                clear_glyph(scale),
                megaphone_glyph(scale),
            ] {
                let side = (16 * scale) as usize;
                assert_eq!(mask.len(), side * side);
                let ink = mask.iter().filter(|c| **c > 128).count();
                assert!(ink > 10 * scale as usize * scale as usize, "{ink}");
                // A clear margin: the outer row is empty.
                assert!(mask[..side].iter().all(|c| *c < 64));
            }
        }
        let full = alloc::vec![255u8; 400];
        assert!(resample(&full, 20, 16).iter().all(|c| *c == 255));
        let mut half = alloc::vec![0u8; 400];
        half[..200].fill(255);
        let r = resample(&half, 20, 16);
        assert_eq!((r[0], r[16 * 15]), (255, 0));
    }
}
