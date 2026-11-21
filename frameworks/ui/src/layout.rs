//! Layout (master specification section 50): row/column stacks with flex
//! weights, grids with fixed/auto/star tracks, overlays, scroll containers,
//! baseline alignment, minimum/maximum sizes, margins and padding, in 26.6
//! fixed-point logical units. Measurement results are cached per node and
//! available size; recursion is bounded.
use crate::id::WidgetId;
use crate::units::{Insets, LPoint, LRect, LSize, Lu};
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Dim {
    /// Size to content.
    #[default]
    Auto,
    Fixed(Lu),
    /// Take the available space (finite) along this axis.
    Fill,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Align {
    Start,
    Center,
    End,
    #[default]
    Stretch,
    /// Align first baselines (cross axis of horizontal stacks).
    Baseline,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Axis {
    Horizontal,
    #[default]
    Vertical,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Justify {
    #[default]
    Start,
    Center,
    End,
    SpaceBetween,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Track {
    Fixed(Lu),
    Auto,
    /// Proportional share of the remaining space.
    Star(u16),
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Sized by the widget's own measurement; children (if any) are not laid out.
    #[default]
    Leaf,
    Stack {
        axis: Axis,
        gap: Lu,
        justify: Justify,
    },
    Grid {
        columns: Vec<Track>,
        rows: Vec<Track>,
        column_gap: Lu,
        row_gap: Lu,
    },
    /// Children share the content box, each aligned by its own alignment.
    Overlay,
    /// One scrolled child, unbounded along the scrolling axes.
    Scroll { horizontal: bool, vertical: bool },
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Cell {
    pub row: u16,
    pub column: u16,
    pub row_span: u16,
    pub column_span: u16,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Style {
    pub kind: Kind,
    pub width: Dim,
    pub height: Dim,
    pub min: LSize,
    pub max: LSize,
    pub margin: Insets,
    pub padding: Insets,
    /// Flex weight along the parent stack's axis (0: fixed to content).
    pub grow: u16,
    pub h_align: Align,
    pub v_align: Align,
    pub cell: Cell,
    /// Collapsed nodes take no space, paint nothing and are not focusable.
    pub visible: bool,
    /// Clip children (and own paint after `paint`) to the border box.
    pub clip: bool,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            kind: Kind::Leaf,
            width: Dim::Auto,
            height: Dim::Auto,
            min: LSize::ZERO,
            max: LSize::INFINITE,
            margin: Insets::ZERO,
            padding: Insets::ZERO,
            grow: 0,
            h_align: Align::Stretch,
            v_align: Align::Stretch,
            cell: Cell {
                row: 0,
                column: 0,
                row_span: 1,
                column_span: 1,
            },
            visible: true,
            clip: false,
        }
    }
}
impl Style {
    pub fn stack(axis: Axis, gap: i32) -> Self {
        Self {
            kind: Kind::Stack {
                axis,
                gap: Lu::px(gap),
                justify: Justify::Start,
            },
            ..Self::default()
        }
    }
    pub fn width(mut self, w: Dim) -> Self {
        self.width = w;
        self
    }
    pub fn height(mut self, h: Dim) -> Self {
        self.height = h;
        self
    }
    pub fn fixed(mut self, w: i32, h: i32) -> Self {
        self.width = Dim::Fixed(Lu::px(w));
        self.height = Dim::Fixed(Lu::px(h));
        self
    }
    pub fn padding(mut self, p: Insets) -> Self {
        self.padding = p;
        self
    }
    pub fn margin(mut self, m: Insets) -> Self {
        self.margin = m;
        self
    }
    pub fn grow(mut self, g: u16) -> Self {
        self.grow = g;
        self
    }
    pub fn align(mut self, h: Align, v: Align) -> Self {
        self.h_align = h;
        self.v_align = v;
        self
    }
    pub fn cell(mut self, row: u16, column: u16) -> Self {
        self.cell = Cell {
            row,
            column,
            row_span: 1,
            column_span: 1,
        };
        self
    }
    pub fn span(mut self, rows: u16, columns: u16) -> Self {
        self.cell.row_span = rows.max(1);
        self.cell.column_span = columns.max(1);
        self
    }
    pub fn min_size(mut self, w: i32, h: i32) -> Self {
        self.min = LSize::px(w, h);
        self
    }
    pub fn max_size(mut self, w: i32, h: i32) -> Self {
        self.max = LSize::px(w, h);
        self
    }
}

/// A measurement: border-box size plus first baseline from its top.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Measured {
    pub size: LSize,
    pub baseline: Option<Lu>,
}
impl Measured {
    pub fn new(size: LSize) -> Self {
        Self {
            size,
            baseline: None,
        }
    }
    pub fn with_baseline(size: LSize, baseline: Lu) -> Self {
        Self {
            size,
            baseline: Some(baseline),
        }
    }
}

/// What the engine needs from the widget tree.
pub(crate) trait Host {
    fn style(&self, id: WidgetId) -> &Style;
    fn padding(&self, id: WidgetId) -> Insets;
    /// Laid-out (visible) children in order.
    fn layout_children(&self, id: WidgetId) -> Vec<WidgetId>;
    fn measure_leaf(&mut self, id: WidgetId, content: LSize) -> Measured;
    fn cached(&self, id: WidgetId, available: LSize) -> Option<Measured>;
    fn cache(&mut self, id: WidgetId, available: LSize, m: Measured);
    fn scroll(&self, id: WidgetId) -> LPoint;
    fn store(
        &mut self,
        id: WidgetId,
        rect: LRect,
        baseline: Option<Lu>,
        content: LSize,
        scroll: LPoint,
    );
    fn note_measure(&mut self, id: WidgetId);
}

pub(crate) const MAX_DEPTH: usize = 128;

fn clamp_size(s: &Style, size: LSize) -> LSize {
    LSize::new(
        size.w.clamp(s.min.w, s.max.w),
        size.h.clamp(s.min.h, s.max.h),
    )
}
fn shrink(a: LSize, i: Insets) -> LSize {
    LSize::new(
        if a.w.is_infinite() {
            a.w
        } else {
            (a.w - i.horizontal()).max(Lu::ZERO)
        },
        if a.h.is_infinite() {
            a.h
        } else {
            (a.h - i.vertical()).max(Lu::ZERO)
        },
    )
}

/// Desired border-box size of `id` given the space available to its border
/// box (margins already removed by the parent).
pub(crate) fn measure(h: &mut dyn Host, id: WidgetId, available: LSize, depth: usize) -> Measured {
    if depth > MAX_DEPTH {
        return Measured::default();
    }
    if let Some(m) = h.cached(id, available) {
        return m;
    }
    h.note_measure(id);
    let style = h.style(id).clone();
    if !style.visible {
        return Measured::default();
    }
    // Explicit dimensions constrain what the content may use.
    let mut avail = available;
    if let Dim::Fixed(w) = style.width {
        avail.w = w;
    }
    if let Dim::Fixed(hh) = style.height {
        avail.h = hh;
    }
    avail = LSize::new(avail.w.min(style.max.w), avail.h.min(style.max.h));
    let pad = h.padding(id);
    let inner = shrink(avail, pad);
    let content = match &style.kind {
        Kind::Leaf => h.measure_leaf(id, inner),
        Kind::Stack { axis, gap, .. } => {
            let kids = h.layout_children(id);
            let mut main = Lu::ZERO;
            let mut cross = Lu::ZERO;
            let mut baseline = None;
            for (i, c) in kids.iter().enumerate() {
                let cs = h.style(*c).clone();
                let m = cs.margin;
                let child_avail = match axis {
                    Axis::Vertical => LSize::new(inner.w - m.horizontal(), Lu::INFINITE),
                    Axis::Horizontal => LSize::new(Lu::INFINITE, inner.h - m.vertical()),
                };
                let cm = measure(h, *c, child_avail, depth + 1);
                if i > 0 {
                    main += *gap;
                }
                match axis {
                    Axis::Vertical => {
                        if i == 0 {
                            baseline = cm.baseline.map(|b| b + m.top);
                        }
                        main += cm.size.h + m.vertical();
                        cross = cross.max(cm.size.w + m.horizontal());
                    }
                    Axis::Horizontal => {
                        if let Some(b) = cm.baseline {
                            let b = b + m.top;
                            baseline = Some(baseline.map_or(b, |x: Lu| x.max(b)));
                        }
                        main += cm.size.w + m.horizontal();
                        cross = cross.max(cm.size.h + m.vertical());
                    }
                }
            }
            let size = match axis {
                Axis::Vertical => LSize::new(cross, main),
                Axis::Horizontal => LSize::new(main, cross),
            };
            Measured { size, baseline }
        }
        Kind::Grid {
            columns,
            rows,
            column_gap,
            row_gap,
        } => {
            let kids = h.layout_children(id);
            let (cols, rws) =
                grid_tracks(h, &kids, columns, rows, *column_gap, *row_gap, inner, depth);
            let w = cols.iter().copied().fold(Lu::ZERO, |a, b| a + b)
                + *column_gap * (cols.len().max(1) as i32 - 1);
            let hh = rws.iter().copied().fold(Lu::ZERO, |a, b| a + b)
                + *row_gap * (rws.len().max(1) as i32 - 1);
            Measured::new(LSize::new(w, hh))
        }
        Kind::Overlay => {
            let kids = h.layout_children(id);
            let mut size = LSize::ZERO;
            let mut baseline = None;
            for c in kids {
                let m = h.style(c).margin;
                let cm = measure(h, c, shrink(inner, m), depth + 1);
                size = size.max(LSize::new(
                    cm.size.w + m.horizontal(),
                    cm.size.h + m.vertical(),
                ));
                baseline = baseline.or(cm.baseline.map(|b| b + m.top));
            }
            Measured { size, baseline }
        }
        Kind::Scroll {
            horizontal,
            vertical,
        } => {
            let kids = h.layout_children(id);
            let mut size = LSize::ZERO;
            for c in kids.into_iter().take(1) {
                let m = h.style(c).margin;
                let a = LSize::new(
                    if *horizontal {
                        Lu::INFINITE
                    } else {
                        inner.w - m.horizontal()
                    },
                    if *vertical {
                        Lu::INFINITE
                    } else {
                        inner.h - m.vertical()
                    },
                );
                let cm = measure(h, c, a, depth + 1);
                size = LSize::new(cm.size.w + m.horizontal(), cm.size.h + m.vertical());
            }
            // A scroller wants its content but settles for what it gets.
            Measured::new(LSize::new(
                if inner.w.is_infinite() {
                    size.w
                } else {
                    size.w.min(inner.w)
                },
                if inner.h.is_infinite() {
                    size.h
                } else {
                    size.h.min(inner.h)
                },
            ))
        }
    };
    let mut size = LSize::new(
        content.size.w + pad.horizontal(),
        content.size.h + pad.vertical(),
    );
    match style.width {
        Dim::Fixed(w) => size.w = w,
        Dim::Fill if !available.w.is_infinite() => size.w = available.w,
        _ => (),
    }
    match style.height {
        Dim::Fixed(hh) => size.h = hh,
        Dim::Fill if !available.h.is_infinite() => size.h = available.h,
        _ => (),
    }
    let size = clamp_size(&style, size);
    let m = Measured {
        size,
        baseline: content.baseline.map(|b| b + pad.top),
    };
    h.cache(id, available, m);
    m
}

#[allow(clippy::too_many_arguments)]
fn grid_tracks(
    h: &mut dyn Host,
    kids: &[WidgetId],
    columns: &[Track],
    rows: &[Track],
    column_gap: Lu,
    row_gap: Lu,
    inner: LSize,
    depth: usize,
) -> (Vec<Lu>, Vec<Lu>) {
    let resolve = |tracks: &[Track], total: Lu, gap: Lu, desired: &[Lu]| -> Vec<Lu> {
        let tracks: Vec<Track> = if tracks.is_empty() {
            alloc::vec![Track::Star(1)]
        } else {
            tracks.to_vec()
        };
        let mut sizes: Vec<Lu> = tracks
            .iter()
            .enumerate()
            .map(|(i, t)| match t {
                Track::Fixed(v) => *v,
                Track::Auto => desired.get(i).copied().unwrap_or(Lu::ZERO),
                Track::Star(_) => Lu::ZERO,
            })
            .collect();
        let used =
            sizes.iter().copied().fold(Lu::ZERO, |a, b| a + b) + gap * (tracks.len() as i32 - 1);
        let stars: i32 = tracks
            .iter()
            .map(|t| {
                if let Track::Star(w) = t {
                    i32::from(*w)
                } else {
                    0
                }
            })
            .sum();
        if stars > 0 {
            if total.is_infinite() {
                for (i, t) in tracks.iter().enumerate() {
                    if matches!(t, Track::Star(_)) {
                        sizes[i] = desired.get(i).copied().unwrap_or(Lu::ZERO);
                    }
                }
            } else {
                let remaining = (total - used).max(Lu::ZERO);
                let mut given = Lu::ZERO;
                let mut seen = 0;
                for (i, t) in tracks.iter().enumerate() {
                    if let Track::Star(w) = t {
                        seen += i32::from(*w);
                        // Cumulative rounding: shares sum exactly to `remaining`.
                        let upto = remaining.mul_div(seen, stars);
                        sizes[i] = upto - given;
                        given = upto;
                    }
                }
            }
        }
        sizes
    };
    let ncols = columns.len().max(1);
    let nrows = rows.len().max(1);
    let mut want_w = alloc::vec![Lu::ZERO; ncols];
    let mut want_h = alloc::vec![Lu::ZERO; nrows];
    let mut measured = Vec::with_capacity(kids.len());
    for c in kids {
        let s = h.style(*c).clone();
        let cm = measure(h, *c, shrink(LSize::INFINITE, s.margin), depth + 1);
        let col = (s.cell.column as usize).min(ncols - 1);
        let row = (s.cell.row as usize).min(nrows - 1);
        if s.cell.column_span <= 1 {
            want_w[col] = want_w[col].max(cm.size.w + s.margin.horizontal());
        }
        if s.cell.row_span <= 1 {
            want_h[row] = want_h[row].max(cm.size.h + s.margin.vertical());
        }
        measured.push(cm);
    }
    let cols = resolve(columns, inner.w, column_gap, &want_w);
    // Rows: re-measure with the resolved column widths (text wraps).
    for (i, c) in kids.iter().enumerate() {
        let s = h.style(*c).clone();
        let col = (s.cell.column as usize).min(ncols - 1);
        let span = (s.cell.column_span.max(1) as usize).min(ncols - col);
        let w: Lu = cols[col..col + span]
            .iter()
            .copied()
            .fold(Lu::ZERO, |a, b| a + b)
            + column_gap * (span as i32 - 1);
        let cm = measure(
            h,
            *c,
            LSize::new(w - s.margin.horizontal(), Lu::INFINITE),
            depth + 1,
        );
        let row = (s.cell.row as usize).min(nrows - 1);
        if s.cell.row_span <= 1 {
            want_h[row] = want_h[row].max(cm.size.h + s.margin.vertical());
        }
        measured[i] = cm;
    }
    let rws = resolve(rows, inner.h, row_gap, &want_h);
    (cols, rws)
}

fn align_in(a: Align, start: Lu, room: Lu, size: Lu) -> (Lu, Lu) {
    match a {
        Align::Start | Align::Baseline => (start, size.min(room)),
        Align::Center => (start + (room - size).max(Lu::ZERO) / 2, size.min(room)),
        Align::End => (start + (room - size).max(Lu::ZERO), size.min(room)),
        Align::Stretch => (start, room),
    }
}

/// Place `id` at `rect` (border box) and lay out its subtree.
pub(crate) fn arrange(h: &mut dyn Host, id: WidgetId, rect: LRect, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let style = h.style(id).clone();
    let pad = h.padding(id);
    let inner = rect.inset(pad);
    let kids = h.layout_children(id);
    let own = measure(h, id, rect.size(), depth);
    let mut content = inner.size();
    let mut scroll = h.scroll(id);
    match &style.kind {
        Kind::Leaf => (),
        Kind::Stack { axis, gap, justify } => {
            let vertical = *axis == Axis::Vertical;
            let mut sizes = Vec::with_capacity(kids.len());
            let mut total = Lu::ZERO;
            let mut weights = 0i32;
            for c in &kids {
                let cs = h.style(*c).clone();
                let m = cs.margin;
                let a = if vertical {
                    LSize::new(inner.w - m.horizontal(), Lu::INFINITE)
                } else {
                    LSize::new(Lu::INFINITE, inner.h - m.vertical())
                };
                let cm = measure(h, *c, a, depth + 1);
                total += if vertical {
                    cm.size.h + m.vertical()
                } else {
                    cm.size.w + m.horizontal()
                };
                weights += i32::from(cs.grow);
                sizes.push((cs, cm));
            }
            total += *gap * (kids.len().max(1) as i32 - 1);
            let room = if vertical { inner.h } else { inner.w };
            let free = room - total;
            let mut pos = if vertical { inner.y } else { inner.x };
            let between = if weights == 0 && free > Lu::ZERO {
                match justify {
                    Justify::Start => Lu::ZERO,
                    Justify::Center => {
                        pos += free / 2;
                        Lu::ZERO
                    }
                    Justify::End => {
                        pos += free;
                        Lu::ZERO
                    }
                    Justify::SpaceBetween if kids.len() > 1 => free / (kids.len() as i32 - 1),
                    Justify::SpaceBetween => Lu::ZERO,
                }
            } else {
                Lu::ZERO
            };
            let max_baseline = sizes
                .iter()
                .filter(|(s, _)| s.v_align == Align::Baseline)
                .filter_map(|(s, m)| m.baseline.map(|b| b + s.margin.top))
                .fold(Lu::ZERO, Lu::max);
            let mut seen = 0i32;
            let mut given = Lu::ZERO;
            for (i, c) in kids.iter().enumerate() {
                let (cs, cm) = &sizes[i];
                let m = cs.margin;
                let mut main = if vertical { cm.size.h } else { cm.size.w };
                // Weighted children share the free space; when the content
                // overflows they give it back (a star track: a growing
                // ScrollViewer gets the remaining viewport, not its content).
                if cs.grow > 0 && free != Lu::ZERO && weights > 0 {
                    seen += i32::from(cs.grow);
                    let upto = free.mul_div(seen, weights);
                    main += upto - given;
                    given = upto;
                    let floor = if vertical { cs.min.h } else { cs.min.w };
                    main = main.max(floor).max(Lu::ZERO);
                }
                let r = if vertical {
                    let (x, w) = align_in(
                        cs.h_align,
                        inner.x + m.left,
                        inner.w - m.horizontal(),
                        cm.size.w,
                    );
                    LRect::new(x, pos + m.top, w, main)
                } else {
                    let (y, hh) = if cs.v_align == Align::Baseline {
                        let b = cm.baseline.unwrap_or(cm.size.h);
                        (inner.y + max_baseline - b, cm.size.h)
                    } else {
                        align_in(
                            cs.v_align,
                            inner.y + m.top,
                            inner.h - m.vertical(),
                            cm.size.h,
                        )
                    };
                    LRect::new(pos + m.left, y, main, hh)
                };
                arrange(h, *c, r, depth + 1);
                pos +=
                    main + if vertical {
                        m.vertical()
                    } else {
                        m.horizontal()
                    } + *gap
                        + between;
            }
        }
        Kind::Grid {
            columns,
            rows,
            column_gap,
            row_gap,
        } => {
            let (cols, rws) = grid_tracks(
                h,
                &kids,
                columns,
                rows,
                *column_gap,
                *row_gap,
                inner.size(),
                depth,
            );
            let starts = |sizes: &[Lu], origin: Lu, gap: Lu| {
                let mut v = Vec::with_capacity(sizes.len() + 1);
                let mut p = origin;
                for s in sizes {
                    v.push(p);
                    p += *s + gap;
                }
                v.push(p - gap);
                v
            };
            let xs = starts(&cols, inner.x, *column_gap);
            let ys = starts(&rws, inner.y, *row_gap);
            for c in &kids {
                let cs = h.style(*c).clone();
                let col = (cs.cell.column as usize).min(cols.len() - 1);
                let row = (cs.cell.row as usize).min(rws.len() - 1);
                let ce = (col + cs.cell.column_span.max(1) as usize).min(cols.len());
                let re = (row + cs.cell.row_span.max(1) as usize).min(rws.len());
                let cell = LRect::new(
                    xs[col],
                    ys[row],
                    xs[ce]
                        - xs[col]
                        - if ce < cols.len() {
                            *column_gap
                        } else {
                            Lu::ZERO
                        },
                    ys[re] - ys[row] - if re < rws.len() { *row_gap } else { Lu::ZERO },
                );
                let cell = cell.inset(cs.margin);
                let cm = measure(h, *c, cell.size(), depth + 1);
                let (x, w) = align_in(cs.h_align, cell.x, cell.w, cm.size.w);
                let (y, hh) = align_in(cs.v_align, cell.y, cell.h, cm.size.h);
                arrange(h, *c, LRect::new(x, y, w, hh), depth + 1);
            }
        }
        Kind::Overlay => {
            for c in &kids {
                let cs = h.style(*c).clone();
                let cell = inner.inset(cs.margin);
                let cm = measure(h, *c, cell.size(), depth + 1);
                let (x, w) = align_in(cs.h_align, cell.x, cell.w, cm.size.w);
                let (y, hh) = align_in(cs.v_align, cell.y, cell.h, cm.size.h);
                arrange(h, *c, LRect::new(x, y, w, hh), depth + 1);
            }
        }
        Kind::Scroll {
            horizontal,
            vertical,
        } => {
            if let Some(c) = kids.first() {
                let cs = h.style(*c).clone();
                let m = cs.margin;
                let a = LSize::new(
                    if *horizontal {
                        Lu::INFINITE
                    } else {
                        inner.w - m.horizontal()
                    },
                    if *vertical {
                        Lu::INFINITE
                    } else {
                        inner.h - m.vertical()
                    },
                );
                let cm = measure(h, *c, a, depth + 1);
                let w = if *horizontal {
                    cm.size.w.max(inner.w - m.horizontal())
                } else {
                    inner.w - m.horizontal()
                };
                let hh = if *vertical {
                    cm.size.h.max(inner.h - m.vertical())
                } else {
                    inner.h - m.vertical()
                };
                content = LSize::new(w + m.horizontal(), hh + m.vertical());
                scroll = LPoint::new(
                    scroll
                        .x
                        .clamp(Lu::ZERO, (content.w - inner.w).max(Lu::ZERO)),
                    scroll
                        .y
                        .clamp(Lu::ZERO, (content.h - inner.h).max(Lu::ZERO)),
                );
                arrange(
                    h,
                    *c,
                    LRect::new(inner.x + m.left, inner.y + m.top, w, hh),
                    depth + 1,
                );
            }
        }
    }
    h.store(id, rect, own.baseline, content, scroll);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::Arena;
    use alloc::collections::BTreeMap;
    struct T {
        nodes: Arena<(Style, Vec<WidgetId>, LSize, Option<Lu>)>,
        rects: BTreeMap<WidgetId, LRect>,
        cache: BTreeMap<(WidgetId, LSize), Measured>,
        measures: usize,
    }
    impl T {
        fn new() -> Self {
            Self {
                nodes: Arena::default(),
                rects: BTreeMap::new(),
                cache: BTreeMap::new(),
                measures: 0,
            }
        }
        fn leaf(&mut self, s: Style, w: i32, h: i32) -> WidgetId {
            self.nodes.insert((s, Vec::new(), LSize::px(w, h), None))
        }
        fn node(&mut self, s: Style, kids: &[WidgetId]) -> WidgetId {
            self.nodes.insert((s, kids.to_vec(), LSize::ZERO, None))
        }
    }
    impl Host for T {
        fn style(&self, id: WidgetId) -> &Style {
            &self.nodes.get(id).unwrap().0
        }
        fn padding(&self, id: WidgetId) -> Insets {
            self.style(id).padding
        }
        fn layout_children(&self, id: WidgetId) -> Vec<WidgetId> {
            self.nodes
                .get(id)
                .unwrap()
                .1
                .iter()
                .copied()
                .filter(|c| self.style(*c).visible)
                .collect()
        }
        fn measure_leaf(&mut self, id: WidgetId, _: LSize) -> Measured {
            let n = self.nodes.get(id).unwrap();
            Measured {
                size: n.2,
                baseline: n.3,
            }
        }
        fn cached(&self, id: WidgetId, a: LSize) -> Option<Measured> {
            self.cache.get(&(id, a)).copied()
        }
        fn cache(&mut self, id: WidgetId, a: LSize, m: Measured) {
            self.cache.insert((id, a), m);
        }
        fn scroll(&self, _: WidgetId) -> LPoint {
            LPoint::px(0, 30)
        }
        fn store(&mut self, id: WidgetId, r: LRect, _: Option<Lu>, _: LSize, _: LPoint) {
            self.rects.insert(id, r);
        }
        fn note_measure(&mut self, _: WidgetId) {
            self.measures += 1;
        }
    }
    impl PartialOrd for LSize {
        fn partial_cmp(&self, o: &Self) -> Option<core::cmp::Ordering> {
            Some(self.cmp(o))
        }
    }
    impl Ord for LSize {
        fn cmp(&self, o: &Self) -> core::cmp::Ordering {
            (self.w, self.h).cmp(&(o.w, o.h))
        }
    }
    #[test]
    fn stacks_flex_align_and_baselines() {
        let mut t = T::new();
        let a = t.leaf(Style::default(), 50, 20);
        let b = t.leaf(Style::default().grow(1), 10, 20);
        let c = t.leaf(
            Style::default().grow(2).align(Align::Center, Align::Center),
            10,
            10,
        );
        let row = t.node(
            Style::stack(Axis::Horizontal, 10).padding(Insets::all(5)),
            &[a, b, c],
        );
        arrange(&mut t, row, LRect::px(0, 0, 200, 40), 0);
        // 200 - 10 padding - 20 gaps = 170; fixed 70; free 100 -> 33.33/66.67.
        assert_eq!(t.rects[&a], LRect::px(5, 5, 50, 30));
        assert_eq!(t.rects[&b].x, Lu::px(65));
        let bw = t.rects[&b].w;
        let cw = t.rects[&c].w;
        assert_eq!(
            bw + cw,
            Lu::px(120),
            "weights consume exactly the free space"
        );
        assert_eq!(t.rects[&c].right(), Lu::px(195));
        assert_eq!(t.rects[&c].h, Lu::px(10));
        assert_eq!(t.rects[&c].y, Lu::px(15));
        // Baseline alignment.
        let mut t = T::new();
        let small = t.leaf(
            Style::default().align(Align::Start, Align::Baseline),
            30,
            16,
        );
        t.nodes.get_mut(small).unwrap().3 = Some(Lu::px(12));
        let big = t.leaf(
            Style::default().align(Align::Start, Align::Baseline),
            30,
            36,
        );
        t.nodes.get_mut(big).unwrap().3 = Some(Lu::px(28));
        let row = t.node(Style::stack(Axis::Horizontal, 0), &[small, big]);
        arrange(&mut t, row, LRect::px(0, 0, 100, 50), 0);
        assert_eq!(t.rects[&small].y + Lu::px(12), t.rects[&big].y + Lu::px(28));
    }
    #[test]
    fn grids_resolve_fixed_auto_and_star_tracks_with_spans() {
        let mut t = T::new();
        let a = t.leaf(Style::default().cell(0, 0), 80, 20);
        let b = t.leaf(Style::default().cell(0, 1), 40, 30);
        let c = t.leaf(Style::default().cell(1, 0).span(1, 3), 10, 10);
        let grid = t.node(
            Style {
                kind: Kind::Grid {
                    columns: alloc::vec![Track::Auto, Track::Star(1), Track::Fixed(Lu::px(50))],
                    rows: alloc::vec![Track::Auto, Track::Star(1)],
                    column_gap: Lu::px(4),
                    row_gap: Lu::px(4),
                },
                ..Style::default()
            },
            &[a, b, c],
        );
        arrange(&mut t, grid, LRect::px(0, 0, 300, 200), 0);
        assert_eq!(t.rects[&a], LRect::px(0, 0, 80, 30));
        // Star column: 300 - 80 - 50 - 8 = 162.
        assert_eq!(t.rects[&b], LRect::px(84, 0, 162, 30));
        assert_eq!(t.rects[&c], LRect::px(0, 34, 300, 166));
    }
    #[test]
    fn min_max_fixed_fill_and_scroll_content() {
        let mut t = T::new();
        let a = t.leaf(Style::default().min_size(100, 0).max_size(150, 10), 20, 40);
        let s = t.node(
            Style::stack(Axis::Vertical, 0).align(Align::Start, Align::Start),
            &[a],
        );
        let m = measure(&mut t, s, LSize::px(500, 500), 0);
        assert_eq!(m.size, LSize::px(100, 10));
        let mut t = T::new();
        let tall = t.leaf(Style::default(), 50, 900);
        let sc = t.node(
            Style {
                kind: Kind::Scroll {
                    horizontal: false,
                    vertical: true,
                },
                ..Style::default()
            },
            &[tall],
        );
        arrange(&mut t, sc, LRect::px(0, 0, 200, 300), 0);
        assert_eq!(t.rects[&tall], LRect::px(0, 0, 200, 900));
        // Cache: arranging twice measures nothing new.
        let before = t.measures;
        arrange(&mut t, sc, LRect::px(0, 0, 200, 300), 0);
        assert_eq!(t.measures, before);
    }
}
