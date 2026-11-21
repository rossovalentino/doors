//! Windows Terminal mouse selection over a `terminal_model::Tab`.
//!
//! A left drag selects cell by cell across the scrollback; a double click
//! selects a word and a triple click a whole logical line, and dragging
//! after either extends by that unit. Words are runs of one class, as
//! Windows Terminal classifies cells: blanks, `WORD_DELIMITERS`, other text.
//! Shift+click extends the selection; Alt+drag selects a block (a rectangle
//! of rows and columns). Endpoints address logical lines by serial number
//! and cell, so a selection stays on its text when the console re-wraps or
//! output arrives, and is trimmed or dropped as its lines leave the bounded
//! scrollback.
//!
//! Copied text matches Windows Terminal: rows soft-wrapped from one logical
//! line join without a break, each line that ends inside the selection is
//! trimmed of trailing blanks and ends in CRLF (the TEXT clipboard format
//! is Windows text), and a block copies each row's columns as its own
//! trimmed line.
//!
//! Paste goes to the single-line input, which only Enter submits, so a paste
//! never runs anything: escape and control sequences are removed, tabs
//! become spaces, and only the first non-empty line is inserted. Later lines
//! are counted for the status line, never submitted (`sanitize_paste`).
extern crate alloc;
use crate::terminal_model::Tab;
use alloc::{string::String, vec::Vec};

/// Multi-click window in `door_sys::time` units (as Files' double click).
pub const DOUBLE_CLICK: u64 = 50;
/// Pointer travel (px) a double or triple click tolerates.
pub const CLICK_SLOP: i32 = 4;
/// Windows Terminal's default `wordDelimiters`.
pub const WORD_DELIMITERS: &str = " /\\()\"'-.,:;<>~!@#$%^&*|+=[]{}~?\u{2502}";
/// Cell standing for "through the end of the line" (triple-click lines).
pub const LINE_END: usize = usize::MAX / 4;
/// Clipboard line break.
pub const NEWLINE: &str = "\r\n";

/// A cell of a logical line: `line` is the line's serial (`Tab::serial`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Point {
    pub line: u64,
    pub cell: usize,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unit {
    Cell,
    Word,
    Line,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Selection {
    /// The unit (cell, word or line) under the press, first and last cell.
    pub anchor: (Point, Point),
    /// The cell under the pointer (or the Shift+click).
    pub head: Point,
    pub unit: Unit,
    /// Alt: a rectangle of rows and columns.
    pub block: bool,
    /// Something is selected: the drag left its first cell, or the press
    /// selected a word or line. A click alone selects nothing.
    pub active: bool,
}
impl Selection {
    /// Keep what remains once lines before serial `first` have left the
    /// scrollback; false when nothing remains.
    pub fn retain_from(&mut self, first: u64) -> bool {
        if self
            .anchor
            .0
            .line
            .max(self.anchor.1.line)
            .max(self.head.line)
            < first
        {
            return false;
        }
        for p in [&mut self.anchor.0, &mut self.anchor.1, &mut self.head] {
            if p.line < first {
                *p = Point {
                    line: first,
                    cell: 0,
                };
            }
        }
        true
    }
}
/// Cell class for word selection: 0 blank, 1 delimiter, 2 other text.
fn class(c: char) -> u8 {
    if c <= ' ' {
        0
    } else if WORD_DELIMITERS.contains(c) {
        1
    } else {
        2
    }
}
/// The selected columns of each console row, computed once per paint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Highlight {
    /// First and last selected cell (inclusive) at `columns` wide.
    Stream {
        start: Point,
        end: Point,
        columns: usize,
    },
    /// Buffer rows and columns (inclusive).
    Block {
        rows: (usize, usize),
        cols: (usize, usize),
    },
}
impl Highlight {
    /// Selected columns (inclusive) of buffer row `row`, which shows wrapped
    /// row `wrap` of the line with serial `line`.
    pub fn columns(&self, row: usize, line: u64, wrap: usize) -> Option<(usize, usize)> {
        match *self {
            Self::Block { rows, cols } => (rows.0..=rows.1).contains(&row).then_some(cols),
            Self::Stream {
                start,
                end,
                columns,
            } => {
                let first = wrap * columns;
                let row_start = Point { line, cell: first };
                let row_end = Point {
                    line,
                    cell: first + columns - 1,
                };
                if row_end < start || row_start > end {
                    return None;
                }
                // Lines continue to the right edge; the ends stop at their cells.
                let from = if start > row_start {
                    start.cell - first
                } else {
                    0
                };
                let to = if end < row_end {
                    end.cell - first
                } else {
                    columns - 1
                };
                Some((from, to))
            }
        }
    }
}
/// Counts double and triple clicks.
#[derive(Clone, Copy, Debug, Default)]
pub struct Clicks {
    last: Option<(u64, i32, i32)>,
    count: u8,
}
impl Clicks {
    /// A press at `now`; returns 1, 2 or 3 (a fourth click starts over).
    pub fn press(&mut self, now: u64, x: i32, y: i32) -> u8 {
        self.count = match self.last {
            Some((at, px, py))
                if now.saturating_sub(at) <= DOUBLE_CLICK
                    && (px - x).abs() <= CLICK_SLOP
                    && (py - y).abs() <= CLICK_SLOP =>
            {
                self.count % 3 + 1
            }
            _ => 1,
        };
        self.last = Some((now, x, y));
        self.count
    }
}
impl Tab {
    /// The cell `column` of buffer row `row` at `columns` wide.
    pub fn point(&self, row: usize, column: usize, columns: usize) -> Option<Point> {
        let columns = columns.max(1);
        let (line, wrap) = self.locate_row(row, columns)?;
        Some(Point {
            line: self.serial(line),
            cell: wrap * columns + column.min(columns - 1),
        })
    }
    /// The word (run of one cell class) at `p`; past the text, the blanks
    /// from the end of the text to `p`.
    pub fn word_at(&self, p: Point) -> (Point, Point) {
        let Some(index) = self.index_of(p.line) else {
            return (p, p);
        };
        let chars: Vec<char> = self.lines[index].text().chars().collect();
        let at = |cell| Point { line: p.line, cell };
        if p.cell >= chars.len() {
            let start = chars
                .iter()
                .rposition(|c| class(*c) != 0)
                .map_or(0, |i| i + 1);
            return (at(start), p);
        }
        let kind = class(chars[p.cell]);
        let mut start = p.cell;
        while start > 0 && class(chars[start - 1]) == kind {
            start -= 1;
        }
        let mut end = p.cell;
        while end + 1 < chars.len() && class(chars[end + 1]) == kind {
            end += 1;
        }
        (at(start), at(end))
    }
    /// The whole logical line at `p`, across `MAX_LINE` continuations.
    pub fn line_at(&self, p: Point) -> (Point, Point) {
        let Some(mut last) = self.index_of(p.line) else {
            return (p, p);
        };
        let mut first = last;
        while first > 0 && self.lines[first - 1].continued() {
            first -= 1;
        }
        while last + 1 < self.lines.len() && self.lines[last].continued() {
            last += 1;
        }
        (
            Point {
                line: self.serial(first),
                cell: 0,
            },
            Point {
                line: self.serial(last),
                cell: LINE_END,
            },
        )
    }
    /// Left press on `p`: `clicks` 2 selects a word, 3 a line; `extend`
    /// (Shift) moves the end of an existing selection; `block` (Alt) starts
    /// a rectangle.
    pub fn press(&mut self, p: Point, clicks: u8, extend: bool, block: bool) {
        if extend && let Some(selection) = self.selection.as_mut() {
            selection.head = p;
            selection.active = true;
            return;
        }
        let (unit, anchor) = match clicks {
            2 => (Unit::Word, self.word_at(p)),
            3 => (Unit::Line, self.line_at(p)),
            _ => (Unit::Cell, (p, p)),
        };
        self.selection = Some(Selection {
            anchor,
            head: anchor.0,
            unit,
            block: block && unit == Unit::Cell,
            active: unit != Unit::Cell,
        });
    }
    /// The pointer moved to `p` while the button is down; true if the
    /// selection changed.
    pub fn drag(&mut self, p: Point) -> bool {
        let Some(selection) = self.selection.as_mut() else {
            return false;
        };
        let before = *selection;
        selection.head = p;
        if p != selection.anchor.0 {
            selection.active = true;
        }
        before != *selection
    }
    /// Button released: a click that selected nothing leaves no selection.
    pub fn release(&mut self) {
        if self.selection.is_some_and(|s| !s.active) {
            self.selection = None;
        }
    }
    /// First and last selected cells (inclusive), in text order.
    pub fn selection_bounds(&self) -> Option<(Point, Point)> {
        let s = self.selection.filter(|s| s.active)?;
        let (from, to) = match s.unit {
            Unit::Cell => (s.head, s.head),
            Unit::Word => self.word_at(s.head),
            Unit::Line => self.line_at(s.head),
        };
        Some((s.anchor.0.min(from), s.anchor.1.max(to)))
    }
    /// Buffer row and column of `p` at `columns` wide (cells past the text
    /// sit on the line's last row, at most in its last column).
    fn visual(&self, p: Point, columns: usize) -> Option<(usize, usize)> {
        let index = self.index_of(p.line)?;
        let row = self.row_index(index, p.cell, columns);
        let wrap = (p.cell / columns).min(self.lines[index].rows(columns) - 1);
        Some((row, (p.cell - wrap * columns).min(columns - 1)))
    }
    /// What to paint for the selection at `columns` wide.
    pub fn highlight(&self, columns: usize) -> Option<Highlight> {
        let columns = columns.max(1);
        let (start, end) = self.selection_bounds()?;
        let s = self.selection?;
        if s.block {
            let (a, b) = (
                self.visual(s.anchor.0, columns)?,
                self.visual(s.head, columns)?,
            );
            return Some(Highlight::Block {
                rows: (a.0.min(b.0), a.0.max(b.0)),
                cols: (a.1.min(b.1), a.1.max(b.1)),
            });
        }
        Some(Highlight::Stream {
            start,
            end,
            columns,
        })
    }
    /// The selection as clipboard text (`None` when it holds no text).
    pub fn selected_text(&self, columns: usize) -> Option<String> {
        let columns = columns.max(1);
        let mut out = String::new();
        match self.highlight(columns)? {
            Highlight::Block { rows, cols } => {
                for row in rows.0..=rows.1 {
                    let (line, wrap) = self.locate_row(row, columns)?;
                    let text: String = self
                        .row_text(line, wrap, columns)
                        .chars()
                        .skip(cols.0)
                        .take(cols.1 - cols.0 + 1)
                        .collect();
                    out.push_str(text.trim_end_matches(' '));
                    if row < rows.1 {
                        out.push_str(NEWLINE);
                    }
                }
            }
            Highlight::Stream { start, end, .. } => {
                let (first, last) = (self.index_of(start.line)?, self.index_of(end.line)?);
                for index in first..=last {
                    let line = &self.lines[index];
                    let from = if index == first { start.cell } else { 0 };
                    let to = if index == last {
                        end.cell.saturating_add(1).min(line.cells())
                    } else {
                        line.cells()
                    };
                    let mut text = line.slice(from, to);
                    // The line's end is inside the selection: trim its blanks
                    // unless the next entry continues it.
                    let ends = index < last || end.cell.saturating_add(1) >= line.cells();
                    if ends && !line.continued() {
                        text = text.trim_end_matches(' ');
                    }
                    out.push_str(text);
                    if index < last && !line.continued() {
                        out.push_str(NEWLINE);
                    }
                }
            }
        }
        (!out.is_empty()).then_some(out)
    }
}
/// The paste for the single-line input: its first non-empty line (CR, LF
/// or CRLF ends a line) with escape sequences (CSI, OSC and two-character
/// escapes, 7- or 8-bit) and other control characters removed, tabs as
/// spaces, trailing blanks trimmed, at most `limit` bytes. Also returns the
/// number of further non-empty lines, which are not pasted.
pub fn sanitize_paste(text: &str, limit: usize) -> (String, usize) {
    let mut lines = text
        .split(['\r', '\n'])
        .filter(|l| l.chars().any(|c| !c.is_whitespace()));
    let first = lines.next().unwrap_or("");
    let more = lines.count();
    let mut out = String::new();
    let mut chars = first.chars().peekable();
    while let Some(c) = chars.next() {
        let c = match c {
            '\u{1b}' => {
                match chars.next() {
                    Some('[') => skip_csi(&mut chars),
                    Some(']') => skip_osc(&mut chars),
                    _ => (),
                }
                continue;
            }
            '\u{9b}' => {
                skip_csi(&mut chars);
                continue;
            }
            '\u{9d}' => {
                skip_osc(&mut chars);
                continue;
            }
            '\t' => ' ',
            c if c.is_control() => continue,
            c => c,
        };
        if out.len() + c.len_utf8() > limit {
            break;
        }
        out.push(c);
    }
    out.truncate(out.trim_end().len());
    (out, more)
}
/// Parameters and intermediates through the final byte (0x40..=0x7e).
fn skip_csi(chars: &mut impl Iterator<Item = char>) {
    for c in chars.by_ref() {
        if ('\u{40}'..='\u{7e}').contains(&c) {
            break;
        }
    }
}
/// Through BEL or the string terminator (ESC \ or 0x9c).
fn skip_osc(chars: &mut core::iter::Peekable<impl Iterator<Item = char>>) {
    while let Some(c) = chars.next() {
        match c {
            '\u{7}' | '\u{9c}' => break,
            '\u{1b}' => {
                chars.next_if_eq(&'\\');
                break;
            }
            _ => (),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal_model::{MAX_LINE, MAX_LINES, Profile};
    fn tab(text: &str, columns: usize) -> Tab {
        let mut t = Tab::new(Profile::Native, "/".into());
        t.clear();
        t.reflow(columns);
        t.output(text);
        t
    }
    fn p(t: &Tab, index: usize, cell: usize) -> Point {
        Point {
            line: t.serial(index),
            cell,
        }
    }
    fn select_points(t: &mut Tab, from: Point, to: Point) {
        t.press(from, 1, false, false);
        t.drag(to);
        t.release();
    }
    /// Drag from `(line index, cell)` to another.
    fn select(t: &mut Tab, from: (usize, usize), to: (usize, usize)) {
        let (from, to) = (p(t, from.0, from.1), p(t, to.0, to.1));
        select_points(t, from, to);
    }
    #[test]
    fn a_click_selects_nothing_and_a_drag_selects_cells_across_lines() {
        let mut t = tab("alpha beta  \ngamma", 20);
        t.press(p(&t, 0, 2), 1, false, false);
        assert!(t.selection_bounds().is_none());
        t.release();
        assert!(t.selection.is_none(), "a click clears the selection");
        // Dragging backwards includes the anchor cell.
        select(&mut t, (0, 8), (0, 6));
        assert_eq!(t.selected_text(20).as_deref(), Some("bet"));
        // To the end of the line: trailing blanks are trimmed, CRLF joins.
        select(&mut t, (0, 6), (1, 2));
        assert_eq!(t.selected_text(20).as_deref(), Some("beta\r\ngam"));
        // Past the end of the last line copies no line break.
        select(&mut t, (1, 0), (1, 19));
        assert_eq!(t.selected_text(20).as_deref(), Some("gamma"));
        // Only blanks: nothing to copy.
        select(&mut t, (0, 11), (0, 15));
        assert_eq!(t.selected_text(20), None);
    }
    #[test]
    fn soft_wrapped_rows_copy_as_one_line_and_highlight_to_the_edge() {
        let mut t = tab("0123456789abcdef\nnext", 6);
        // Row 0: 012345, row 1: 6789ab, row 2: cdef, row 3: next.
        let start = t.point(0, 4, 6).unwrap();
        let end = t.point(3, 1, 6).unwrap();
        select_points(&mut t, start, end);
        assert_eq!(t.selected_text(6).as_deref(), Some("456789abcdef\r\nne"));
        let h = t.highlight(6).unwrap();
        let rows: Vec<_> = t
            .rows_in(0..4, 6)
            .iter()
            .enumerate()
            .map(|(row, &(line, wrap))| h.columns(row, t.serial(line), wrap))
            .collect();
        assert_eq!(
            rows,
            [Some((4, 5)), Some((0, 5)), Some((0, 5)), Some((0, 1))]
        );
        // The same selection reflows with its text.
        t.reflow(4);
        assert_eq!(t.selected_text(4).as_deref(), Some("456789abcdef\r\nne"));
        // Long-line continuations join without a break too.
        let long = "x".repeat(MAX_LINE + 10);
        let mut t = tab(&long, 80);
        assert!(t.lines[0].continued());
        let (from, to) = t.line_at(p(&t, 1, 3));
        assert_eq!((from, to.line), (p(&t, 0, 0), t.serial(1)));
        t.press(p(&t, 0, 0), 3, false, false);
        assert_eq!(t.selected_text(80), Some(long));
    }
    #[test]
    fn double_click_selects_words_and_triple_click_lines_and_dragging_extends_by_unit() {
        let mut t = tab("cd /users/me && ls...x\nsecond line", 80);
        t.press(p(&t, 0, 5), 2, false, false);
        assert_eq!(t.selected_text(80).as_deref(), Some("users"));
        t.drag(p(&t, 0, 10));
        assert_eq!(t.selected_text(80).as_deref(), Some("users/me"));
        t.release();
        assert!(t.selection.is_some(), "a word stays selected");
        t.press(p(&t, 0, 19), 2, false, false);
        assert_eq!(t.selected_text(80).as_deref(), Some("..."), "delimiter run");
        t.press(p(&t, 0, 2), 2, false, false);
        assert_eq!(t.selected_text(80).as_deref(), Some(" "), "a blank run");
        t.press(p(&t, 0, 40), 2, false, false);
        assert_eq!(t.selected_text(80), None, "blanks past the text");
        t.press(p(&t, 0, 3), 3, false, false);
        assert_eq!(
            t.selected_text(80).as_deref(),
            Some("cd /users/me && ls...x")
        );
        t.drag(p(&t, 1, 0));
        assert_eq!(
            t.selected_text(80).as_deref(),
            Some("cd /users/me && ls...x\r\nsecond line")
        );
        let h = t.highlight(80).unwrap();
        assert_eq!(
            h.columns(1, t.serial(1), 0),
            Some((0, 79)),
            "line to the edge"
        );
    }
    #[test]
    fn shift_click_extends_and_alt_drag_selects_a_block() {
        let mut t = tab("one two\nthree four\nfive", 80);
        select(&mut t, (0, 0), (0, 2));
        t.press(p(&t, 1, 4), 1, true, false);
        t.release();
        assert_eq!(t.selected_text(80).as_deref(), Some("one two\r\nthree"));
        // Shift+click without a selection starts one there.
        t.selection = None;
        t.press(p(&t, 2, 1), 1, true, false);
        assert!(t.selection_bounds().is_none());
        // Alt+drag: the same columns of each row, each row trimmed.
        t.press(p(&t, 0, 4), 1, false, true);
        t.drag(p(&t, 2, 6));
        t.release();
        assert_eq!(t.selected_text(80).as_deref(), Some("two\r\ne f\r\n"));
        let h = t.highlight(80).unwrap();
        assert_eq!(
            h,
            Highlight::Block {
                rows: (0, 2),
                cols: (4, 6)
            }
        );
        assert_eq!(h.columns(1, t.serial(1), 0), Some((4, 6)));
        assert_eq!(h.columns(3, t.serial(3), 0), None);
    }
    #[test]
    fn selection_spans_scrollback_and_survives_eviction_and_output() {
        let mut t = tab("", 80);
        for i in 0..MAX_LINES {
            t.output(&alloc::format!("line {i}"));
        }
        select(&mut t, (0, 0), (MAX_LINES - 1, 2));
        let text = t.selected_text(80).unwrap();
        assert!(text.starts_with("line 0\r\nline 1\r\n") && text.ends_with("\r\nlin"));
        assert_eq!(text.matches(NEWLINE).count(), MAX_LINES - 1);
        // New output evicts the first lines: the selection keeps the rest.
        t.output("more\nmore");
        let text = t.selected_text(80).unwrap();
        assert!(text.starts_with("line 2\r\n"), "{}", &text[..20]);
        // Once all of it has left the scrollback it is gone.
        select(&mut t, (0, 0), (0, 3));
        t.output("x");
        assert!(t.selection.is_none());
        t.clear();
        assert!(t.selection.is_none());
    }
    #[test]
    fn clicks_count_up_to_three_within_time_and_distance() {
        let mut c = Clicks::default();
        assert_eq!(c.press(100, 10, 10), 1);
        assert_eq!(c.press(120, 12, 9), 2);
        assert_eq!(c.press(140, 10, 10), 3);
        assert_eq!(c.press(150, 10, 10), 1);
        assert_eq!(c.press(150 + DOUBLE_CLICK + 1, 10, 10), 1, "too slow");
        assert_eq!(c.press(150 + DOUBLE_CLICK + 2, 30, 10), 1, "moved");
    }
    #[test]
    fn paste_inserts_one_sanitized_line_and_reports_the_rest() {
        assert_eq!(sanitize_paste("ls -l\r\n", 64), ("ls -l".into(), 0));
        assert_eq!(
            sanitize_paste("\n\necho one\r\necho two\n\nrm x\n", 64),
            ("echo one".into(), 2)
        );
        assert_eq!(
            sanitize_paste(
                "a\x1b[31mred\x1b[0m\tb\x07\x1b]0;title\x07!\u{9b}2Jz\x1bc",
                64
            ),
            ("ared b!z".into(), 0)
        );
        assert_eq!(
            sanitize_paste("\x1b]8;;u\x1b\\link", 64),
            ("link".into(), 0)
        );
        assert_eq!(sanitize_paste("文字abc", 7), ("文字a".into(), 0));
        assert_eq!(sanitize_paste("   \r\n\t", 64), (String::new(), 0));
    }
}
