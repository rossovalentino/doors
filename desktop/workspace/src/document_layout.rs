//! Visual layout of Notes text (master specification §52), separate from the
//! document model and from painting. Each logical line is cut into segments
//! of at most `SEGMENT_BYTES`; each segment (a *block*) is laid out on its own,
//! with the editor atlas for ASCII at 100% zoom (exact hinted pixels) and
//! otherwise through Door Text (shaping, font fallback, bidi, COLR emoji).
//! Viewports only ever lay out the blocks they show, so document size does
//! not change the cost of a frame.
extern crate alloc;
use crate::document::{Affinity, Document};
use alloc::{string::String, vec::Vec};
use core::ops::Range;
use door_graphics_text::{
    self as text, Family, FontAtlas, LayoutOptions, TextFormat, TextLayout, TextStyle, TextSystem,
    Wrap,
};

/// Longest block. A longer line continues in the next block (a visual row
/// break, preferably after a space), much as classic Notepad bounded lines.
pub const SEGMENT_BYTES: usize = 4096;
/// Editor row height at 100% zoom (logical pixels).
pub const BASE_LINE_HEIGHT: u32 = 20;
/// Notepad's zoom range and step (percent).
pub const MIN_ZOOM: u32 = 10;
pub const MAX_ZOOM: u32 = 500;
pub const ZOOM_STEP: u32 = 10;
pub const STYLE: TextStyle = TextStyle::Regular14;
/// Atlas measurement chunk (its bounded run is 4096 bytes).
const MEASURE_CHUNK: usize = 2048;

/// A caret position with its soft-wrap side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caret {
    pub byte: usize,
    pub affinity: Affinity,
}
/// The editor fonts and zoom one layout pass uses: the editor atlas (whose
/// attached Unicode engine, if any, shapes non-ASCII text) and the zoom.
#[derive(Clone, Copy)]
pub struct Engine<'a> {
    pub fonts: &'a FontAtlas,
    pub zoom: u32,
}
impl<'a> Engine<'a> {
    pub fn new(fonts: &'a FontAtlas, zoom: u32) -> Self {
        Self { fonts, zoom }
    }
    pub fn system(&self) -> Option<&'a TextSystem> {
        self.fonts.unicode().map(|s| &**s)
    }
    /// The zoom in effect: Door Text scales the editor font; without it the
    /// atlas has one size and zoom stays 100%.
    pub fn zoom(&self) -> u32 {
        if self.system().is_some() {
            self.zoom.clamp(MIN_ZOOM, MAX_ZOOM)
        } else {
            100
        }
    }
    pub fn line_height(&self) -> u32 {
        ((BASE_LINE_HEIGHT * self.zoom() + 50) / 100).max(1)
    }
    /// Atlas advance of `text` with Notepad's four-column tab stops.
    pub fn atlas_width(&self, text: &str) -> u32 {
        let shown = display(text);
        let mut rest = shown.as_str();
        let mut width = 0;
        while !rest.is_empty() {
            let mut n = rest.len().min(MEASURE_CHUNK);
            while !rest.is_char_boundary(n) {
                n -= 1;
            }
            width += self
                .fonts
                .measure(&rest[..n], STYLE)
                .map_or(0, |m| m.advance_width);
            rest = &rest[n..];
        }
        width
    }
    fn format(&self) -> TextFormat {
        let mut format = TextFormat::from_style(STYLE, self.fonts.percent() as f32 / 100.0)
            .with_family(Family::Monospace);
        if self.zoom() != 100 {
            format.size = STYLE.pixel_size() as f32 * self.zoom() as f32 / 100.0;
            format.line_height = Some(self.line_height() as f32);
        }
        format
    }
    fn uses_atlas(&self, text: &str) -> bool {
        self.system().is_none() || (self.zoom() == 100 && text.is_ascii())
    }
}
/// Visible text of a row: tabs expanded to four-column stops (storage keeps
/// the tab bytes).
pub fn display(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut column = 0;
    for c in text.chars() {
        if c == '\t' {
            let n = 4 - column % 4;
            out.extend(core::iter::repeat_n(' ', n));
            column += n;
        } else {
            out.push(c);
            column += 1;
        }
    }
    out
}
fn floor_char(doc: &Document, mut at: usize) -> usize {
    while doc.byte(at).is_some_and(|b| (b & 0xc0) == 0x80) {
        at -= 1;
    }
    at
}
fn segment_end(doc: &Document, start: usize, line_end: usize) -> usize {
    if line_end - start <= SEGMENT_BYTES {
        return line_end;
    }
    let limit = start + SEGMENT_BYTES;
    (limit.saturating_sub(256).max(start + 1)..=limit)
        .rev()
        .find(|&i| doc.byte(i - 1) == Some(b' '))
        .unwrap_or_else(|| {
            let g = doc.grapheme_floor(limit);
            if g > start { g } else { floor_char(doc, limit) }
        })
}
/// The block containing `at`: (start, end of its text, start of the next
/// block — after the line ending when this is the line's last segment). A
/// segment boundary belongs to the following block.
pub fn block_bounds(doc: &Document, at: usize) -> (usize, usize, usize) {
    let line = doc.line_of(at.min(doc.len()));
    let (line_end, line_next) = (doc.line_end(line), doc.line_next(line));
    let mut start = doc.line_start(line);
    loop {
        let end = segment_end(doc, start, line_end);
        if at < end || end == line_end {
            return (start, end, if end == line_end { line_next } else { end });
        }
        start = end;
    }
}
#[derive(Clone, Copy, Debug)]
struct Preedit {
    /// Document offset and text offset of the composition.
    doc: usize,
    at: usize,
    len: usize,
    cursor: usize,
}
/// One laid-out block: its text (with any IME preedit spliced in at the
/// caret), its visual rows as text ranges and, on the Door Text path, the
/// shaped layout that owns glyphs, carets, hit testing and selection.
pub struct Block {
    pub start: usize,
    pub end: usize,
    pub next: usize,
    text: String,
    preedit: Option<Preedit>,
    rows: Vec<Range<usize>>,
    layout: Option<TextLayout>,
    line_height: u32,
    /// The line continues in the next block at `end` (a segment boundary).
    soft_tail: bool,
}
/// Nearest integer (no_std has no `f32::round`).
fn round(v: f32) -> i32 {
    (v + if v >= 0.0 { 0.5 } else { -0.5 }) as i32
}
fn affinity_in(a: Affinity) -> text::Affinity {
    match a {
        Affinity::Upstream => text::Affinity::Upstream,
        Affinity::Downstream => text::Affinity::Downstream,
    }
}
fn affinity_out(a: text::Affinity) -> Affinity {
    match a {
        text::Affinity::Upstream => Affinity::Upstream,
        text::Affinity::Downstream => Affinity::Downstream,
    }
}
impl Block {
    /// Lay out the block starting at `start` (a block start), wrapping at
    /// `width` logical pixels when given.
    pub fn build(doc: &Document, start: usize, engine: &Engine<'_>, width: Option<u32>) -> Self {
        let (start, end, next) = block_bounds(doc, start);
        let mut text = doc.text(start..end).into_owned();
        let preedit = doc
            .composition()
            .filter(|c| block_bounds(doc, c.at).0 == start)
            .map(|c| {
                text.insert_str(c.at - start, &c.text);
                Preedit {
                    doc: c.at,
                    at: c.at - start,
                    len: c.text.len(),
                    cursor: c.cursor.unwrap_or(c.text.len()),
                }
            });
        let mut block = Self {
            start,
            end,
            next,
            text,
            preedit,
            rows: Vec::new(),
            layout: None,
            line_height: engine.line_height(),
            soft_tail: next == end && end < doc.len(),
        };
        if !engine.uses_atlas(&block.text)
            && let Some(system) = engine.system()
        {
            let options = LayoutOptions {
                max_width: width.map(|w| w as f32),
                wrap: if width.is_some() {
                    Wrap::Word
                } else {
                    Wrap::None
                },
                ..LayoutOptions::default()
            };
            if let Ok(layout) = system.layout(&block.text, &engine.format(), &options) {
                block.rows = layout.lines().into_iter().map(|l| l.text).collect();
                block.layout = Some(layout);
            }
        }
        if block.layout.is_none() {
            block.rows = atlas_rows(&block.text, width, engine);
        }
        if block.rows.is_empty() {
            block.rows.push(0..0);
        }
        block
    }
    pub fn rows(&self) -> usize {
        self.rows.len()
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn layout(&self) -> Option<&TextLayout> {
        self.layout.as_ref()
    }
    pub fn row_text(&self, row: usize) -> &str {
        &self.text[self.rows[row].clone()]
    }
    /// Text range of the IME preedit, if it is shown in this block.
    pub fn preedit(&self) -> Option<Range<usize>> {
        self.preedit.map(|p| p.at..p.at + p.len)
    }
    fn to_text(&self, at: usize) -> usize {
        let t = at.clamp(self.start, self.end) - self.start;
        match self.preedit {
            Some(p) if t > p.at => t + p.len,
            _ => t,
        }
    }
    fn to_doc(&self, t: usize) -> usize {
        match self.preedit {
            Some(p) if t >= p.at + p.len => self.start + t - p.len,
            Some(p) if t > p.at => p.doc,
            _ => self.start + t,
        }
    }
    /// Document range shown by a row.
    pub fn row_range(&self, row: usize) -> Range<usize> {
        self.to_doc(self.rows[row].start)..self.to_doc(self.rows[row].end)
    }
    /// Where the following row starts (the next block after the last row).
    pub fn row_next(&self, row: usize) -> usize {
        if row + 1 < self.rows.len() {
            self.to_doc(self.rows[row + 1].start)
        } else {
            self.next
        }
    }
    /// Whether `at` ends one row and starts the next inside this block.
    pub fn soft_wrap_at(&self, at: usize) -> bool {
        let t = self.to_text(at);
        self.rows
            .windows(2)
            .any(|w| w[0].end == t && w[1].start == t)
    }
    fn text_row(&self, t: usize, affinity: Affinity) -> usize {
        if let Some(layout) = &self.layout {
            return layout
                .line_of(t, affinity_in(affinity))
                .min(self.rows.len() - 1);
        }
        let mut row = self
            .rows
            .partition_point(|r| r.end < t)
            .min(self.rows.len() - 1);
        if affinity == Affinity::Downstream && self.rows[row].end == t && row + 1 < self.rows.len()
        {
            row += 1;
        }
        row
    }
    /// Row and x (logical, from the text origin) of a text offset.
    fn text_caret(&self, engine: &Engine<'_>, t: usize, affinity: Affinity) -> (usize, i32) {
        if let Some(layout) = &self.layout {
            let c = layout.caret(t, affinity_in(affinity));
            return (c.line.min(self.rows.len() - 1), round(c.x));
        }
        let row = self.text_row(t, affinity);
        let from = self.rows[row].start.min(t);
        (row, engine.atlas_width(&self.text[from..t]) as i32)
    }
    /// Caret row and x for a document offset; inside a composition the
    /// caret is the preedit cursor.
    pub fn caret(&self, engine: &Engine<'_>, at: usize, affinity: Affinity) -> (usize, i32) {
        let t = match self.preedit {
            Some(p) if p.doc == at => p.at + p.cursor,
            _ => self.to_text(at),
        };
        self.text_caret(engine, t, affinity)
    }
    /// Nearest caret stop to logical `x` in `row`, with the side to show it on.
    pub fn hit(&self, engine: &Engine<'_>, row: usize, x: i32) -> (usize, Affinity) {
        let row = row.min(self.rows.len() - 1);
        if let Some(layout) = &self.layout {
            let h = layout.hit_test(x as f32, (row as f32 + 0.5) * self.line_height as f32);
            return (self.to_doc(h.offset), affinity_out(h.affinity));
        }
        let range = self.rows[row].clone();
        let stops = text::grapheme_boundaries(&self.text[range.clone()]);
        let width =
            |i: usize| engine.atlas_width(&self.text[range.start..range.start + stops[i]]) as i64;
        let x = i64::from(x.max(0));
        // Largest stop whose advance is at most x, then the nearer neighbour.
        let (mut low, mut high) = (0, stops.len());
        while low + 1 < high {
            let middle = (low + high) / 2;
            if width(middle) <= x {
                low = middle
            } else {
                high = middle
            }
        }
        let index = if low + 1 < stops.len() && (width(low + 1) - x) <= (x - width(low)) {
            low + 1
        } else {
            low
        };
        let t = range.start + stops[index];
        let soft =
            t == range.end && t > range.start && (row + 1 < self.rows.len() || self.soft_tail);
        (
            self.to_doc(t),
            if soft {
                Affinity::Upstream
            } else {
                Affinity::Downstream
            },
        )
    }
    /// The side a caret at the end of `row` is shown on (Home/End).
    pub fn end_affinity(&self, row: usize) -> Affinity {
        let r = &self.rows[row];
        if r.end > r.start && (row + 1 < self.rows.len() || self.soft_tail) {
            Affinity::Upstream
        } else {
            Affinity::Downstream
        }
    }
    /// Horizontal spans (logical, from the text origin) of a text range in a row.
    pub fn text_spans(
        &self,
        engine: &Engine<'_>,
        row: usize,
        range: Range<usize>,
    ) -> Vec<(i32, i32)> {
        let r = &self.rows[row];
        let (a, b) = (range.start.max(r.start), range.end.min(r.end));
        if a >= b {
            return Vec::new();
        }
        if let Some(layout) = &self.layout {
            let top = (row as u32 * self.line_height) as f32;
            return layout
                .selection_rects(a..b)
                .into_iter()
                .filter(|rect| (rect.y - top).abs() < 0.5)
                .map(|rect| (round(rect.x), round(rect.x + rect.width)))
                .collect();
        }
        let x = |t: usize| engine.atlas_width(&self.text[r.start..t]) as i32;
        alloc::vec![(x(a), x(b))]
    }
    /// Selection spans of a row and whether the highlight runs on to the body
    /// edge (the selection continues past the row's end).
    pub fn selection(
        &self,
        engine: &Engine<'_>,
        row: usize,
        selected: Range<usize>,
    ) -> (Vec<(i32, i32)>, bool) {
        if selected.is_empty() {
            return (Vec::new(), false);
        }
        let range = self.row_range(row);
        let next = self.row_next(row);
        let hard = next > range.end && selected.start < next && selected.end > range.end;
        let soft = selected.start < range.end && selected.end > range.end;
        let spans = self.text_spans(
            engine,
            row,
            self.to_text(selected.start.max(range.start))
                ..self.to_text(selected.end.min(range.end)),
        );
        (spans, hard || soft)
    }
    /// Advance of a whole row (logical).
    pub fn row_width(&self, engine: &Engine<'_>, row: usize) -> i32 {
        let r = self.rows[row].clone();
        if let Some(layout) = &self.layout {
            return layout.lines().get(row).map_or(0, |l| round(l.x + l.width));
        }
        engine.atlas_width(&self.text[r]) as i32
    }
    /// For long atlas rows: the text range covering logical x `from..to` and
    /// the x where it starts, so painting stays within the atlas run bound.
    pub fn atlas_window(
        &self,
        engine: &Engine<'_>,
        row: usize,
        from: i32,
        to: i32,
    ) -> (Range<usize>, i32) {
        let r = self.rows[row].clone();
        if r.len() <= MEASURE_CHUNK / 2 {
            return (r, 0);
        }
        let locate = |x: i32| {
            let (at, _) = self.hit(engine, row, x);
            self.to_text(at)
        };
        let (a, b) = (
            locate(from).saturating_sub(4).max(r.start),
            (locate(to) + 4).min(r.end),
        );
        let (mut a, mut b) = (a, b);
        while !self.text.is_char_boundary(a) {
            a -= 1;
        }
        while !self.text.is_char_boundary(b) {
            b += 1;
        }
        (a..b, engine.atlas_width(&self.text[r.start..a]) as i32)
    }
}
/// Greedy atlas wrapping: the longest prefix that fits, broken after the
/// last space when a word would be split; spaces at a break stay on the row.
fn atlas_rows(text: &str, width: Option<u32>, engine: &Engine<'_>) -> Vec<Range<usize>> {
    let Some(width) = width else {
        return alloc::vec![0..text.len()];
    };
    let bounds: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(core::iter::once(text.len()))
        .collect();
    let last = bounds.len() - 1;
    let mut rows = Vec::new();
    let mut i = 0;
    while i < last {
        let start = bounds[i];
        let fits = |j: usize| engine.atlas_width(&text[start..bounds[j]]) <= width;
        let mut low = i + 1;
        if fits(low) {
            let mut step = 1;
            let mut high = low;
            while high < last {
                high = (low + step).min(last);
                if !fits(high) {
                    break;
                }
                low = high;
                step *= 2;
            }
            while high > low + 1 {
                let middle = (low + high) / 2;
                if fits(middle) {
                    low = middle
                } else {
                    high = middle
                }
            }
        }
        let mut j = low;
        if j < last {
            if matches!(text.as_bytes()[bounds[j]], b' ' | b'\t') {
                while j < last && matches!(text.as_bytes()[bounds[j]], b' ' | b'\t') {
                    j += 1;
                }
            } else if let Some(space) = text[start..bounds[j]].rfind([' ', '\t']) {
                j = bounds.partition_point(|&b| b < start + space + 1);
            }
        }
        rows.push(start..bounds[j]);
        i = j;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    fn mono() -> FontAtlas {
        FontAtlas::monospace_reference_scaled(1).unwrap()
    }
    fn unicode() -> FontAtlas {
        mono().with_unicode(crate::assets::text_system().unwrap(), Family::Monospace)
    }
    #[test]
    fn long_lines_split_into_bounded_blocks_at_spaces_and_graphemes() {
        let word = "wörd ".repeat(2000); // 12000 bytes, one line
        let doc = Document::new(&format!("{word}\r\nnext")).unwrap();
        let (s0, e0, n0) = block_bounds(&doc, 0);
        assert_eq!(s0, 0);
        assert!(e0 <= SEGMENT_BYTES && e0 > SEGMENT_BYTES - 256);
        assert_eq!(doc.byte(e0 - 1), Some(b' '), "breaks after a space");
        assert_eq!(n0, e0, "a segment boundary continues the line");
        let (s1, ..) = block_bounds(&doc, e0);
        assert_eq!(s1, e0, "the boundary belongs to the next block");
        let (_, end, next) = block_bounds(&doc, word.len() - 1);
        assert_eq!((end, next), (word.len(), word.len() + 2));
        assert_eq!(
            block_bounds(&doc, word.len() + 1),
            block_bounds(&doc, word.len() - 1),
            "inside CRLF"
        );
        assert_eq!(
            block_bounds(&doc, doc.len()),
            (word.len() + 2, doc.len(), doc.len())
        );
        // No spaces: a grapheme boundary, never inside a cluster.
        let flags = "🇮🇹".repeat(1000);
        let doc = Document::new(&flags).unwrap();
        let (_, end, _) = block_bounds(&doc, 0);
        assert_eq!(end % 8, 0);
        assert!(end <= SEGMENT_BYTES);
    }
    #[test]
    fn atlas_wrap_is_the_longest_fit_and_prefers_word_breaks() {
        let fonts = mono();
        let engine = Engine::new(&fonts, 100);
        let advance = engine.atlas_width("W") as usize;
        let doc = Document::new(&"W".repeat(160)).unwrap();
        let block = Block::build(&doc, 0, &engine, Some(856));
        let count = (1..=160)
            .take_while(|n| engine.atlas_width(&"W".repeat(*n)) <= 856)
            .last()
            .unwrap();
        assert_eq!(block.row_range(0), 0..count);
        assert_eq!(block.row_range(1), count..160);
        assert!(count >= 856 / advance - 1);
        let doc = Document::new("alpha beta gamma").unwrap();
        let block = Block::build(&doc, 0, &engine, Some(engine.atlas_width("alpha beta ga")));
        assert_eq!(
            (block.row_text(0), block.row_text(1)),
            ("alpha beta ", "gamma")
        );
        // Soft-wrap carets: the row end is upstream, the next start downstream.
        assert_eq!(block.caret(&engine, 11, Affinity::Upstream).0, 0);
        assert_eq!(block.caret(&engine, 11, Affinity::Downstream), (1, 0));
        assert_eq!(block.hit(&engine, 0, 10_000), (11, Affinity::Upstream));
        assert_eq!(block.hit(&engine, 1, -5), (11, Affinity::Downstream));
        assert!(block.soft_wrap_at(11));
        // Tabs advance to four-column stops.
        assert_eq!(display("a\tb\t"), "a   b   ");
        assert_eq!(engine.atlas_width("\t"), engine.atlas_width("    "));
    }
    #[test]
    fn door_text_blocks_shape_cjk_emoji_and_place_rtl_carets_visually() {
        let fonts = unicode();
        let engine = Engine::new(&fonts, 100);
        let doc = Document::new("日本語 😀 abc").unwrap();
        let block = Block::build(&doc, 0, &engine, None);
        assert!(block.layout().is_some(), "non-ASCII uses Door Text");
        let (_, after_cjk) = block.caret(&engine, "日本語".len(), Affinity::Downstream);
        assert!(
            after_cjk > engine.atlas_width("abc") as i32,
            "CJK is wider than three ASCII cells"
        );
        assert_eq!(block.hit(&engine, 0, after_cjk).0, "日本語".len());
        // Arabic is laid out right to left: its logical start is at the right.
        let doc = Document::new("abc مرحبا").unwrap();
        let block = Block::build(&doc, 0, &engine, None);
        let start = "abc ".len();
        let (_, first) = block.caret(&engine, start + "م".len(), Affinity::Downstream);
        let (_, last) = block.caret(&engine, doc.len(), Affinity::Upstream);
        assert!(
            first > last,
            "RTL: later characters are further left ({first} > {last})"
        );
        assert!(!block.text_spans(&engine, 0, start..doc.len()).is_empty());
        // ASCII at 100% stays on the atlas; zoom moves everything to Door Text.
        let doc = Document::new("plain").unwrap();
        assert!(Block::build(&doc, 0, &engine, None).layout().is_none());
        let zoomed = Engine::new(&fonts, 200);
        assert_eq!(zoomed.line_height(), 40);
        let wide = Block::build(&doc, 0, &zoomed, None);
        assert!(wide.layout().is_some());
        assert!(
            wide.row_width(&zoomed, 0)
                > Block::build(&doc, 0, &engine, None).row_width(&engine, 0) * 3 / 2
        );
        assert_eq!(
            Engine::new(&mono(), 200).zoom(),
            100,
            "the atlas alone cannot zoom"
        );
    }
    #[test]
    fn preedit_is_spliced_into_its_block_and_maps_back_to_the_caret() {
        let fonts = unicode();
        let engine = Engine::new(&fonts, 100);
        let mut doc = Document::new("ab").unwrap();
        doc.place(1, false);
        doc.set_composition("にほん", Some(3));
        let block = Block::build(&doc, 0, &engine, None);
        assert_eq!(block.text(), "aにほんb");
        assert_eq!(block.preedit(), Some(1..10));
        let (_, caret) = block.caret(&engine, 1, Affinity::Downstream);
        let (_, a) = block.caret(&engine, 0, Affinity::Downstream);
        assert!(caret > a, "the caret follows the preedit cursor");
        assert_eq!(block.row_range(0), 0..2);
        assert_eq!(block.hit(&engine, 0, 10_000).0, 2);
    }
}
