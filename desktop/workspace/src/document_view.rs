//! Notes editing viewport: visible-rows-only layout over `document_layout`
//! blocks, caret reveal, soft-wrap caret sides, preferred column, pointer
//! hits, wheel scrolling and horizontal scrolling when word wrap is off.
//! Layout state is cached per content revision behind interior mutability so
//! painting can bring the view up to date after edits made without fonts.
extern crate alloc;
use crate::document::{Affinity, Document};
use crate::document_layout::{Block, Caret, Engine, block_bounds};
use crate::palette::Palette;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::ops::Range;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Geometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub line_height: u32,
}
/// A visual row: the block that holds it (its start offset) and its index.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Row {
    pub block: usize,
    pub row: usize,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    revision: u64,
    width: Option<u32>,
    zoom: u32,
    percent: u32,
    unicode: bool,
}
#[derive(Default)]
struct Cache {
    key: Option<Key>,
    blocks: Vec<Block>,
}
const CACHED_BLOCKS: usize = 256;

pub struct DocumentView {
    geometry: Geometry,
    wrap: bool,
    top: Cell<Row>,
    hscroll: Cell<i32>,
    preferred_x: Option<i32>,
    dragging: bool,
    /// Wheel scrolling detached the viewport from the caret at this caret and
    /// content revision; any caret movement or edit re-attaches it.
    detached: Cell<Option<(Caret, Option<u64>)>>,
    cache: RefCell<Cache>,
}
impl Default for DocumentView {
    fn default() -> Self {
        Self {
            geometry: Geometry::default(),
            wrap: true,
            top: Cell::new(Row::default()),
            hscroll: Cell::new(0),
            preferred_x: None,
            dragging: false,
            detached: Cell::new(None),
            cache: RefCell::new(Cache::default()),
        }
    }
}
fn caret_of(doc: &Document) -> Caret {
    Caret {
        byte: doc.caret(),
        affinity: doc.affinity(),
    }
}
impl DocumentView {
    pub fn geometry(&self) -> Geometry {
        self.geometry
    }
    pub fn rows(&self) -> usize {
        self.geometry
            .height
            .checked_div(self.geometry.line_height)
            .unwrap_or(0) as usize
    }
    pub fn top(&self) -> Row {
        self.top.get()
    }
    /// Horizontal scroll in logical pixels (word wrap off).
    pub fn hscroll(&self) -> i32 {
        self.hscroll.get()
    }
    pub fn wrap(&self) -> bool {
        self.wrap
    }
    pub fn set_wrap(&mut self, wrap: bool) {
        if wrap != self.wrap {
            self.wrap = wrap;
            self.hscroll.set(0);
            self.edited();
            self.cancel_drag();
        }
    }
    /// Reset for a different Document; the layout cache is keyed by content
    /// revision, which a new document may repeat.
    pub fn reset(&mut self) {
        *self.cache.get_mut() = Cache::default();
        self.top.set(Row::default());
        self.hscroll.set(0);
        self.detached.set(None);
        self.edited();
        self.cancel_drag();
    }
    pub fn edited(&mut self) {
        self.preferred_x = None;
    }
    pub fn cancel_drag(&mut self) {
        self.dragging = false;
    }
    fn wrap_width(&self) -> Option<u32> {
        self.wrap.then_some(self.geometry.width)
    }
    fn key(&self, doc: &Document, engine: &Engine<'_>) -> Option<Key> {
        Some(Key {
            revision: doc.content_revision()?,
            width: self.wrap_width(),
            zoom: engine.zoom(),
            percent: engine.fonts.percent(),
            unicode: engine.system().is_some(),
        })
    }
    /// Run `f` on the (cached or freshly laid out) block containing `at`.
    pub fn with_block<R>(
        &self,
        doc: &Document,
        engine: &Engine<'_>,
        at: usize,
        f: impl FnOnce(&Block) -> R,
    ) -> R {
        let key = self.key(doc, engine);
        let start = block_bounds(doc, at).0;
        let mut cache = self.cache.borrow_mut();
        if key.is_none() || cache.key != key {
            cache.blocks.clear();
            cache.key = key;
        }
        let index = match cache.blocks.iter().position(|b| b.start == start) {
            Some(i) => i,
            None => {
                if cache.blocks.len() >= CACHED_BLOCKS {
                    cache.blocks.clear();
                }
                cache
                    .blocks
                    .push(Block::build(doc, start, engine, self.wrap_width()));
                cache.blocks.len() - 1
            }
        };
        f(&cache.blocks[index])
    }
    fn next_row(&self, doc: &Document, engine: &Engine<'_>, r: Row) -> Option<Row> {
        let (rows, end, next) =
            self.with_block(doc, engine, r.block, |b| (b.rows(), b.end, b.next));
        if r.row + 1 < rows {
            Some(Row {
                row: r.row + 1,
                ..r
            })
        } else if end == doc.len() {
            None
        } else {
            Some(Row {
                block: next,
                row: 0,
            })
        }
    }
    fn prev_row(&self, doc: &Document, engine: &Engine<'_>, r: Row) -> Option<Row> {
        if r.row > 0 {
            return Some(Row {
                row: r.row - 1,
                ..r
            });
        }
        if r.block == 0 {
            return None;
        }
        let start = block_bounds(doc, r.block - 1).0;
        let rows = self.with_block(doc, engine, start, |b| b.rows());
        Some(Row {
            block: start,
            row: rows - 1,
        })
    }
    fn walk(&self, doc: &Document, engine: &Engine<'_>, mut r: Row, delta: isize) -> Row {
        for _ in 0..delta.unsigned_abs() {
            let step = if delta > 0 {
                self.next_row(doc, engine, r)
            } else {
                self.prev_row(doc, engine, r)
            };
            match step {
                Some(next) => r = next,
                None => break,
            }
        }
        r
    }
    /// A stored row brought back onto a block start after edits.
    fn valid(&self, doc: &Document, engine: &Engine<'_>, r: Row) -> Row {
        let block = block_bounds(doc, r.block.min(doc.len())).0;
        let rows = self.with_block(doc, engine, block, |b| b.rows());
        Row {
            block,
            row: if block == r.block {
                r.row.min(rows - 1)
            } else {
                0
            },
        }
    }
    /// The caret's visual row and x (logical, from the text origin).
    pub fn caret_row(&self, doc: &Document, engine: &Engine<'_>) -> (Row, i32) {
        let (at, affinity) = (doc.caret(), doc.affinity());
        let (start, ..) = block_bounds(doc, at);
        // Upstream at a segment boundary (a long line split without a line
        // break): the end of the previous block. A hard line start has none.
        if affinity == Affinity::Upstream && at == start && start > 0 && doc.composition().is_none()
        {
            let previous = block_bounds(doc, start - 1);
            if previous.1 == start {
                let (rows, x) = self.with_block(doc, engine, previous.0, |b| {
                    (b.rows(), b.caret(engine, at, affinity).1)
                });
                return (
                    Row {
                        block: previous.0,
                        row: rows - 1,
                    },
                    x,
                );
            }
        }
        self.with_block(doc, engine, start, |b| {
            let (row, x) = b.caret(engine, at, affinity);
            (Row { block: start, row }, x)
        })
    }
    /// Keep the last row on the body's bottom edge rather than above it.
    fn clamp_bottom(&self, doc: &Document, engine: &Engine<'_>) {
        let rows = self.rows().max(1);
        let top = self.top.get();
        let mut r = top;
        let mut shown = 1;
        while shown < rows {
            match self.next_row(doc, engine, r) {
                Some(next) => {
                    r = next;
                    shown += 1;
                }
                None => break,
            }
        }
        if shown < rows {
            self.top
                .set(self.walk(doc, engine, top, -((rows - shown) as isize)));
        }
    }
    /// Bring the caret into view (unless wheel scrolling detached the view
    /// at the current caret and revision).
    pub fn refresh(&self, doc: &Document, engine: &Engine<'_>) {
        self.top.set(self.valid(doc, engine, self.top.get()));
        if self.detached.get() == Some((caret_of(doc), doc.content_revision())) {
            self.clamp_bottom(doc, engine);
            return;
        }
        self.detached.set(None);
        let (row, x) = self.caret_row(doc, engine);
        let rows = self.rows().max(1);
        let top = self.top.get();
        if row < top {
            self.top.set(row);
        } else {
            let mut r = top;
            let mut visible = false;
            for _ in 0..rows {
                if r == row {
                    visible = true;
                    break;
                }
                match self.next_row(doc, engine, r) {
                    Some(next) => r = next,
                    None => break,
                }
            }
            if !visible {
                self.top
                    .set(self.walk(doc, engine, row, -(rows as isize - 1)));
            }
        }
        self.clamp_bottom(doc, engine);
        if self.wrap {
            self.hscroll.set(0);
        } else {
            let (width, scroll) = (self.geometry.width as i32, self.hscroll.get());
            if x < scroll {
                self.hscroll.set((x - width / 4).max(0));
            } else if x > scroll + width - 2 {
                self.hscroll.set(x - width * 3 / 4);
            }
        }
    }
    /// Mouse-wheel scrolling: moves the viewport by `delta` rows without moving
    /// the caret, as Notepad does. The next caret move or edit brings the
    /// caret back into view. Returns whether the viewport moved.
    pub fn scroll_by(&self, doc: &Document, delta: isize, engine: &Engine<'_>) -> bool {
        let before = self.valid(doc, engine, self.top.get());
        self.top.set(self.walk(doc, engine, before, delta));
        self.clamp_bottom(doc, engine);
        self.detached
            .set(Some((caret_of(doc), doc.content_revision())));
        self.top.get() != before
    }
    pub fn configure(&mut self, doc: &Document, geometry: Geometry, engine: &Engine<'_>) {
        self.geometry = geometry;
        self.cancel_drag();
        self.edited();
        self.refresh(doc, engine);
    }
    pub fn body_contains(&self, x: i32, y: i32) -> bool {
        self.rows() != 0
            && i64::from(x) >= i64::from(self.geometry.x)
            && i64::from(x) < i64::from(self.geometry.x) + i64::from(self.geometry.width)
            && i64::from(y) >= i64::from(self.geometry.y)
            && i64::from(y)
                < i64::from(self.geometry.y)
                    + self.rows() as i64 * i64::from(self.geometry.line_height)
    }
    fn row_at(&self, doc: &Document, engine: &Engine<'_>, y: i32) -> Row {
        let index = (i64::from(y) - i64::from(self.geometry.y))
            .div_euclid(i64::from(self.geometry.line_height.max(1)))
            .clamp(0, self.rows().saturating_sub(1) as i64);
        self.walk(doc, engine, self.top.get(), index as isize)
    }
    fn place_pointer(
        &mut self,
        doc: &mut Document,
        x: i32,
        y: i32,
        extend: bool,
        engine: &Engine<'_>,
    ) {
        self.refresh(doc, engine);
        let row = self.row_at(doc, engine, y);
        let x = x
            .saturating_sub(self.geometry.x)
            .saturating_add(self.hscroll.get());
        let (at, affinity) = self.with_block(doc, engine, row.block, |b| b.hit(engine, row.row, x));
        doc.place_visual(at, extend, affinity);
        self.edited();
        self.refresh(doc, engine);
    }
    pub fn pointer_down(
        &mut self,
        doc: &mut Document,
        x: i32,
        y: i32,
        engine: &Engine<'_>,
    ) -> bool {
        if !self.body_contains(x, y) {
            return false;
        }
        self.place_pointer(doc, x, y, false, engine);
        self.dragging = true;
        true
    }
    pub fn pointer_motion(
        &mut self,
        doc: &mut Document,
        x: i32,
        y: i32,
        engine: &Engine<'_>,
    ) -> bool {
        if !self.dragging {
            return false;
        }
        let before = caret_of(doc);
        self.place_pointer(doc, x, y, true, engine);
        before != caret_of(doc)
    }
    /// Up/Down/PageUp/PageDown by visual rows at the preferred column.
    pub fn vertical(
        &mut self,
        doc: &mut Document,
        delta: isize,
        extend: bool,
        engine: &Engine<'_>,
    ) {
        let (row, x) = self.caret_row(doc, engine);
        let x = self.preferred_x.unwrap_or(x);
        let target = self.walk(doc, engine, row, delta);
        let (at, affinity) =
            self.with_block(doc, engine, target.block, |b| b.hit(engine, target.row, x));
        doc.place_visual(at, extend, affinity);
        self.preferred_x = Some(x);
        self.refresh(doc, engine);
    }
    /// Home/End of the caret's visual row (Notepad with word wrap).
    pub fn edge(&mut self, doc: &mut Document, end: bool, extend: bool, engine: &Engine<'_>) {
        let (row, _) = self.caret_row(doc, engine);
        let (at, affinity) = self.with_block(doc, engine, row.block, |b| {
            let range = b.row_range(row.row);
            if end {
                (range.end, b.end_affinity(row.row))
            } else {
                (range.start, Affinity::Downstream)
            }
        });
        doc.place_visual(at, extend, affinity);
        self.edited();
        self.refresh(doc, engine);
    }
    /// Whether `at` ends one visual row and starts the next (a soft wrap or
    /// a segment boundary; never the start of a line after a line break).
    fn soft_boundary(&self, doc: &Document, engine: &Engine<'_>, at: usize) -> bool {
        let (start, ..) = block_bounds(doc, at);
        if at == start && start > 0 && block_bounds(doc, start - 1).1 == start {
            return true;
        }
        self.with_block(doc, engine, start, |b| b.soft_wrap_at(at))
    }
    /// Left/Right by grapheme clusters; at a soft-wrap boundary the first
    /// press only moves the caret to the other row's side.
    pub fn horizontal(
        &mut self,
        doc: &mut Document,
        right: bool,
        extend: bool,
        engine: &Engine<'_>,
    ) {
        let at = doc.caret();
        if self.soft_boundary(doc, engine, at)
            && (extend || doc.selection().is_empty())
            && right == (doc.affinity() == Affinity::Upstream)
        {
            let side = if right {
                Affinity::Downstream
            } else {
                Affinity::Upstream
            };
            doc.place_visual(at, extend, side);
        } else {
            doc.horizontal(right, extend, false);
            let at = doc.caret();
            if right && self.soft_boundary(doc, engine, at) {
                doc.place_visual(at, extend, Affinity::Upstream);
            }
        }
        self.edited();
        self.refresh(doc, engine);
    }
    /// Visit each block with visible rows: the block, the y of its first row
    /// (it may be above the body) and the visible row range.
    pub fn visible(
        &self,
        doc: &Document,
        engine: &Engine<'_>,
        mut f: impl FnMut(&Block, i32, Range<usize>),
    ) {
        let line_height = self.geometry.line_height as i32;
        let mut remaining = self.rows() + 1;
        let mut r = self.valid(doc, engine, self.top.get());
        let mut y = self.geometry.y;
        while remaining > 0 {
            let (shown, next) = self.with_block(doc, engine, r.block, |b| {
                let shown = (b.rows() - r.row).min(remaining);
                f(b, y - r.row as i32 * line_height, r.row..r.row + shown);
                (shown, (b.end < doc.len()).then_some(b.next))
            });
            remaining -= shown;
            y += shown as i32 * line_height;
            match next {
                Some(block) => r = Row { block, row: 0 },
                None => break,
            }
        }
    }
    /// The caret's logical position (x, row top) if its row is visible.
    pub fn caret_point(&self, doc: &Document, engine: &Engine<'_>) -> Option<(i32, i32)> {
        let (row, x) = self.caret_row(doc, engine);
        let mut r = self.valid(doc, engine, self.top.get());
        for index in 0..=self.rows() {
            if r == row {
                return Some((
                    self.geometry.x + x - self.hscroll.get(),
                    self.geometry.y + index as i32 * self.geometry.line_height as i32,
                ));
            }
            r = self.next_row(doc, engine, r)?;
        }
        None
    }
}

/// Notes colours as light-theme roles. `ui::View` maps every role through
/// the session palette (dark layers, contrast-theme system colours), so the
/// editor carries no theme-specific colour except the warm light-mode Mica
/// tints, which resolve to the neutral layer roles in dark mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub text: [u8; 4],
    pub background: [u8; 4],
    pub caption: [u8; 4],
    pub bar: [u8; 4],
    pub hover: [u8; 4],
    pub divider: [u8; 4],
    pub selection: [u8; 4],
    pub selected_text: [u8; 4],
    pub caret: [u8; 4],
}
impl Theme {
    pub fn resolve(palette: Palette) -> Self {
        let warm = !palette.dark;
        Self {
            text: crate::tokens::TEXT.value,
            background: [255; 4],
            caption: if warm {
                [243, 235, 225, 255]
            } else {
                [243, 243, 243, 255]
            },
            bar: if warm {
                [250, 247, 243, 255]
            } else {
                [250, 250, 250, 255]
            },
            hover: if warm {
                [239, 234, 229, 255]
            } else {
                [235, 235, 235, 255]
            },
            divider: [220, 220, 220, 255],
            selection: crate::tokens::ACCENT.value,
            selected_text: [255; 4],
            caret: crate::tokens::ACCENT.value,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{format, string::String};
    use door_graphics_text::FontAtlas;
    fn mono() -> FontAtlas {
        FontAtlas::monospace_reference_scaled(1).unwrap()
    }
    fn geometry(width: u32, height: u32) -> Geometry {
        Geometry {
            x: 20,
            y: 86,
            width,
            height,
            line_height: 20,
        }
    }
    #[test]
    fn a_line_start_after_a_line_break_is_not_a_wrap_boundary() {
        // desktop-notes: Shift+Left selects the final newline, undo/redo
        // round-trips its deletion, and Right collapses after it. The caret
        // belongs on the empty last line, as the status bar says.
        let fonts = mono();
        let engine = Engine::new(&fonts, 100);
        let mut d = Document::new("Original text\n").unwrap();
        d.place(14, false);
        let mut v = DocumentView::default();
        v.configure(&d, geometry(400, 200), &engine);
        v.horizontal(&mut d, false, true, &engine);
        assert_eq!(d.caret(), 13);
        v.horizontal(&mut d, true, false, &engine);
        assert_eq!((d.caret(), d.affinity()), (14, Affinity::Downstream));
        assert_eq!(v.caret_row(&d, &engine).0, Row { block: 14, row: 0 });
        // Even an upstream caret there draws on its own line.
        d.place_visual(14, false, Affinity::Upstream);
        assert_eq!(v.caret_row(&d, &engine).0, Row { block: 14, row: 0 });
    }
    #[test]
    fn arrow_traversal_visits_both_sides_of_wrap_boundaries() {
        let fonts = mono();
        let engine = Engine::new(&fonts, 100);
        let cell = engine.atlas_width("a") as u32;
        let mut d = Document::new("abcdef").unwrap();
        d.place(0, false);
        let mut v = DocumentView::default();
        v.configure(&d, geometry(cell * 3, 200), &engine);
        let mut seen = Vec::new();
        for _ in 0..7 {
            v.horizontal(&mut d, true, false, &engine);
            seen.push((d.caret(), d.affinity()));
        }
        use Affinity::*;
        assert_eq!(
            seen,
            [
                (1, Downstream),
                (2, Downstream),
                (3, Upstream),
                (3, Downstream),
                (4, Downstream),
                (5, Downstream),
                (6, Downstream)
            ]
        );
        assert_eq!(v.caret_row(&d, &engine).0, Row { block: 0, row: 1 });
        v.horizontal(&mut d, false, false, &engine);
        v.horizontal(&mut d, false, false, &engine);
        v.horizontal(&mut d, false, false, &engine);
        assert_eq!((d.caret(), d.affinity()), (3, Downstream));
        v.horizontal(&mut d, false, false, &engine);
        assert_eq!((d.caret(), d.affinity()), (3, Upstream));
        assert_eq!(v.caret_row(&d, &engine).0.row, 0);
    }
    #[test]
    fn viewport_reveals_the_caret_and_lays_out_only_visible_blocks() {
        let fonts = mono();
        let engine = Engine::new(&fonts, 100);
        let text: String = (0..50_000).map(|i| format!("line {i}\r\n")).collect();
        let mut d = Document::new(&text).unwrap();
        assert!(d.len() > 512 * 1024);
        let mut v = DocumentView::default();
        v.configure(&d, geometry(400, 200), &engine);
        assert_eq!(v.rows(), 10);
        d.home_end(true, true, false);
        v.refresh(&d, &engine);
        let (x, y) = v.caret_point(&d, &engine).unwrap();
        assert_eq!(
            (x, y),
            (20, 86 + 9 * 20),
            "the last row sits on the bottom edge"
        );
        assert!(
            v.cache.borrow().blocks.len() <= 32,
            "only blocks near the viewport are laid out"
        );
        d.home_end(false, true, false);
        v.refresh(&d, &engine);
        assert_eq!(v.top(), Row::default());
        // Page navigation keeps the column; the wheel detaches the view.
        d.place(5, false);
        v.vertical(&mut d, 9, false, &engine);
        assert_eq!(&d.text(d.line_start(9)..d.caret()), "line ");
        assert!(v.scroll_by(&d, 30, &engine));
        assert_eq!(v.caret_point(&d, &engine), None);
        v.refresh(&d, &engine);
        assert_eq!(v.caret_point(&d, &engine), None, "still detached");
        v.horizontal(&mut d, true, false, &engine);
        assert!(
            v.caret_point(&d, &engine).is_some(),
            "a caret move re-attaches"
        );
        let mut visited = 0;
        v.visible(&d, &engine, |_, _, rows| visited += rows.len());
        assert_eq!(visited, 11);
    }
    #[test]
    fn pointer_hits_rows_and_wrap_off_scrolls_horizontally() {
        let fonts = mono();
        let engine = Engine::new(&fonts, 100);
        let cell = engine.atlas_width("a") as i32;
        let mut d = Document::new(&format!("Original text\n{}", "x".repeat(400))).unwrap();
        let mut v = DocumentView::default();
        v.configure(&d, geometry(400, 200), &engine);
        let x = 20 + engine.atlas_width("Original ") as i32;
        assert!(v.pointer_down(&mut d, x, 93, &engine));
        assert!(v.pointer_motion(&mut d, 10_000, 93, &engine));
        v.cancel_drag();
        assert_eq!(d.selection(), 9..13);
        v.set_wrap(false);
        d.home_end(true, true, false);
        v.refresh(&d, &engine);
        assert!(v.hscroll() > 0);
        let (cx, _) = v.caret_point(&d, &engine).unwrap();
        assert!((20..=420).contains(&cx), "{cx}");
        d.home_end(false, false, false);
        v.refresh(&d, &engine);
        assert_eq!(v.hscroll(), 0);
        assert!(v.pointer_down(&mut d, 20 + cell * 3, 113, &engine));
        assert_eq!(d.caret(), 14 + 3);
    }
    #[test]
    fn theme_roles_follow_the_palette() {
        let light = Theme::resolve(Palette::light());
        let dark = Theme::resolve(Palette::with_dark(true));
        assert_eq!(light.bar, [250, 247, 243, 255]);
        assert_eq!(
            Palette::with_dark(true).surface(dark.caption),
            [32, 32, 32, 255]
        );
        assert_eq!(
            Palette::with_dark(true).surface(dark.background),
            [43, 43, 43, 255]
        );
        assert_eq!(
            Palette::with_dark(true).foreground(dark.text),
            [240, 240, 240, 255]
        );
        assert_eq!(light.caret, dark.caret);
    }
}
