//! Runtime glyph rasterization and the bounded glyph cache.
//!
//! The outline path matches the offline hinted atlas exporter: Skrifa's
//! embedded TrueType interpreter (LCD smooth target, symmetric rendering,
//! preserved linear metrics), an area-coverage raster at three horizontal
//! samples per pixel and the existing five-tap RGB filter. Integer-phase
//! ASCII glyphs therefore equal the packed `hinted-*.dfa` coverage.
use crate::font::Face;
use ab_glyph_rasterizer::{Point, Rasterizer, point};
use alloc::{collections::BTreeMap, vec, vec::Vec};
#[allow(unused_imports)]
use core_maths::CoreFloat;
use skrifa::{
    GlyphId,
    instance::{LocationRef, NormalizedCoord, Size},
    outline::{
        DrawSettings, Engine, Hinting, HintingInstance, HintingOptions, OutlinePen, SmoothMode,
        Target as HintTarget,
    },
};

/// Coverage format requested for outline glyphs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RasterMode {
    /// RGB-stripe coverage (three bytes per pixel). Opaque targets receive LCD
    /// text; translucent targets/colours and grayscale targets reduce it.
    #[default]
    Lcd,
    /// One byte per pixel: the LCD raster's centre-subpixel coverage, identical
    /// to what a `Target::grayscale()` destination draws from LCD coverage.
    Grayscale,
}

/// Largest outline segment count and per-glyph bitmap edge (physical pixels).
pub(crate) const MAX_SEGMENTS: usize = 16 * 1024;
pub(crate) const MAX_GLYPH_EDGE: u32 = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct GlyphKey {
    /// Face content fingerprint and slot.
    pub font: u64,
    pub face: u16,
    pub glyph: u32,
    /// Pixels per em in 1/64 px (size times scale).
    pub ppem: u32,
    /// Variation coordinates identity (0 for the default location).
    pub instance: u64,
    /// 0 LCD, 1 grayscale, 2 colour.
    pub mode: u8,
    /// Horizontal subpixel phase in thirds of a pixel.
    pub phase: u8,
    /// bit 0 hinted, bit 1 synthetic emboldening.
    pub flags: u8,
    /// Foreground colour for COLR palette index 0xFFFF (colour mode only).
    pub foreground: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pixels {
    Lcd,
    Gray,
    /// Premultiplied BGRA.
    Bgra,
}
impl Pixels {
    pub fn bytes(self) -> usize {
        match self {
            Self::Lcd => 3,
            Self::Gray => 1,
            Self::Bgra => 4,
        }
    }
}

/// A rasterized glyph relative to its pen origin on the baseline (y down).
#[derive(Clone, Debug)]
pub(crate) struct GlyphBitmap {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub pixels: Pixels,
    pub data: Vec<u8>,
}
impl GlyphBitmap {
    pub fn empty(pixels: Pixels) -> Self {
        Self {
            left: 0,
            top: 0,
            width: 0,
            height: 0,
            pixels,
            data: Vec::new(),
        }
    }
    pub fn cost(&self) -> usize {
        self.data.capacity() + core::mem::size_of::<Self>() + core::mem::size_of::<GlyphKey>() + 32
    }
}

struct Slot {
    key: GlyphKey,
    bitmap: GlyphBitmap,
    used: u64,
}

/// Cache statistics (also the §52 cache-limit evidence).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub glyphs: usize,
    pub glyph_bytes: usize,
    pub glyph_budget: usize,
    pub glyph_hits: u64,
    pub glyph_misses: u64,
    pub glyph_evictions: u64,
    pub shaped_runs: usize,
    pub shaped_bytes: usize,
    pub shape_hits: u64,
    pub shape_misses: u64,
    pub hinting_instances: usize,
}

/// LRU glyph cache bounded by bytes. Eviction runs in batches down to three
/// quarters of the budget so the amortised insert cost stays constant.
pub(crate) struct GlyphCache {
    index: BTreeMap<GlyphKey, usize>,
    slots: Vec<Option<Slot>>,
    free: Vec<usize>,
    bytes: usize,
    pub budget: usize,
    clock: u64,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
}

impl GlyphCache {
    pub fn new(budget: usize) -> Self {
        Self {
            index: BTreeMap::new(),
            slots: Vec::new(),
            free: Vec::new(),
            bytes: 0,
            budget,
            clock: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.index.len()
    }
    pub fn bytes(&self) -> usize {
        self.bytes
    }
    pub fn clear(&mut self) {
        self.index.clear();
        self.slots.clear();
        self.free.clear();
        self.bytes = 0;
    }
    pub fn lookup(&mut self, key: &GlyphKey) -> Option<usize> {
        self.clock += 1;
        let slot = *self.index.get(key)?;
        if let Some(entry) = self.slots[slot].as_mut() {
            entry.used = self.clock;
        }
        self.hits += 1;
        Some(slot)
    }
    pub fn get(&self, slot: usize) -> &GlyphBitmap {
        &self.slots[slot].as_ref().expect("live glyph slot").bitmap
    }
    /// Insert, evicting least recently used glyphs first. Glyphs used in the
    /// current draw (`pinned` clock or later) are never evicted by it.
    pub fn insert(&mut self, key: GlyphKey, bitmap: GlyphBitmap, pinned: u64) -> usize {
        self.misses += 1;
        let cost = bitmap.cost();
        if self.bytes + cost > self.budget {
            self.evict(
                self.budget.saturating_sub(cost).saturating_mul(3) / 4,
                pinned,
            );
        }
        self.clock += 1;
        let entry = Slot {
            key,
            bitmap,
            used: self.clock,
        };
        let slot = if let Some(slot) = self.free.pop() {
            self.slots[slot] = Some(entry);
            slot
        } else {
            self.slots.push(Some(entry));
            self.slots.len() - 1
        };
        self.bytes += cost;
        self.index.insert(key, slot);
        slot
    }
    fn evict(&mut self, target: usize, pinned: u64) {
        let mut order: Vec<(u64, usize)> = self
            .slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().filter(|s| s.used < pinned).map(|s| (s.used, i)))
            .collect();
        order.sort_unstable();
        for (_, slot) in order {
            if self.bytes <= target {
                break;
            }
            if let Some(entry) = self.slots[slot].take() {
                self.bytes -= entry.bitmap.cost();
                self.index.remove(&entry.key);
                self.free.push(slot);
                self.evictions += 1;
            }
        }
    }
}

/// Collected outline segments in pixel space (y up), with checked bounds.
#[derive(Default)]
pub(crate) struct PathPen {
    pub segments: Vec<Segment>,
    current: Option<Point>,
    start: Option<Point>,
    pub invalid: bool,
    limit: f32,
    /// Optional affine transform applied to every point (colour glyph clips).
    pub transform: Option<[f32; 6]>,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Segment {
    Line(Point, Point),
    Quad(Point, Point, Point),
    Cubic(Point, Point, Point, Point),
}
impl PathPen {
    pub fn new(limit: f32) -> Self {
        Self {
            limit,
            ..Self::default()
        }
    }
    fn map(&self, x: f32, y: f32) -> Point {
        match self.transform {
            Some([a, b, c, d, e, f]) => point(a * x + c * y + e, b * x + d * y + f),
            None => point(x, y),
        }
    }
    fn push(&mut self, segment: Segment, points: &[Point]) {
        if self.segments.len() >= MAX_SEGMENTS
            || points.iter().any(|p| {
                !p.x.is_finite()
                    || !p.y.is_finite()
                    || p.x.abs() > self.limit
                    || p.y.abs() > self.limit
            })
        {
            self.invalid = true;
        } else if !self.invalid {
            self.segments.push(segment);
        }
    }
    pub fn bounds(&self) -> Option<(Point, Point)> {
        let mut min = point(f32::INFINITY, f32::INFINITY);
        let mut max = point(f32::NEG_INFINITY, f32::NEG_INFINITY);
        for segment in &self.segments {
            let points: &[Point] = match segment {
                Segment::Line(a, b) => &[*a, *b],
                Segment::Quad(a, b, c) => &[*a, *b, *c],
                Segment::Cubic(a, b, c, d) => &[*a, *b, *c, *d],
            };
            for p in points {
                min.x = min.x.min(p.x);
                min.y = min.y.min(p.y);
                max.x = max.x.max(p.x);
                max.y = max.y.max(p.y);
            }
        }
        (min.x <= max.x && min.y <= max.y).then_some((min, max))
    }
}
impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.current = Some(self.map(x, y));
        self.start = self.current;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let next = self.map(x, y);
        match self.current {
            Some(a) => self.push(Segment::Line(a, next), &[a, next]),
            None => self.invalid = true,
        }
        self.current = Some(next);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (b, c) = (self.map(cx, cy), self.map(x, y));
        match self.current {
            Some(a) => self.push(Segment::Quad(a, b, c), &[a, b, c]),
            None => self.invalid = true,
        }
        self.current = Some(c);
    }
    fn curve_to(&mut self, cx: f32, cy: f32, dx: f32, dy: f32, x: f32, y: f32) {
        let (b, c, d) = (self.map(cx, cy), self.map(dx, dy), self.map(x, y));
        match self.current {
            Some(a) => self.push(Segment::Cubic(a, b, c, d), &[a, b, c, d]),
            None => self.invalid = true,
        }
        self.current = Some(d);
    }
    fn close(&mut self) {
        if let (Some(a), Some(b)) = (self.current, self.start)
            && a != b
        {
            self.push(Segment::Line(a, b), &[a, b]);
        }
        self.current = None;
        self.start = None;
    }
}

/// Rasterize y-up segments with `h` horizontal samples per pixel and a
/// horizontal offset of `shift` samples. Returns (left sample, top row, width
/// in samples, height, coverage). Bounds follow ab_glyph's `px_bounds`.
pub(crate) fn coverage(
    segments: &[Segment],
    min: Point,
    max: Point,
    h: f32,
    shift: f32,
) -> Option<(i32, i32, usize, usize, Vec<u8>)> {
    let left = (min.x * h + shift).floor();
    let right = (max.x * h + shift).ceil();
    let top = (-max.y).floor();
    let bottom = (-min.y).ceil();
    let (width, height) = ((right - left) as usize, (bottom - top) as usize);
    if width == 0
        || height == 0
        || width > (MAX_GLYPH_EDGE * 3) as usize
        || height > MAX_GLYPH_EDGE as usize
    {
        return None;
    }
    let map = |p: Point| point(p.x * h + shift - left, -p.y - top);
    let mut raster = Rasterizer::new(width, height);
    for segment in segments {
        match *segment {
            Segment::Line(a, b) => raster.draw_line(map(a), map(b)),
            Segment::Quad(a, b, c) => raster.draw_quad(map(a), map(b), map(c)),
            Segment::Cubic(a, b, c, d) => raster.draw_cubic(map(a), map(b), map(c), map(d)),
        }
    }
    let mut out = vec![0u8; width * height];
    raster.for_each_pixel(|index, alpha| out[index] = (alpha.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
    Some((left as i32, top as i32, width, height, out))
}

/// Grow coverage horizontally by `samples` (synthetic emboldening).
fn embolden(raw: &mut Vec<u8>, width: &mut usize, height: usize, samples: usize) {
    let new_width = *width + samples;
    let mut out = vec![0u8; new_width * height];
    for y in 0..height {
        for x in 0..new_width {
            let lo = x.saturating_sub(samples);
            let hi = x.min(*width - 1);
            let mut best = 0;
            if lo <= hi {
                for sx in lo..=hi {
                    best = best.max(raw[y * *width + sx]);
                }
            }
            out[y * new_width + x] = best;
        }
    }
    *raw = out;
    *width = new_width;
}

/// Filter 3x coverage into RGB (or the centre subpixel for grayscale), exactly
/// as the hinted atlas exporter does.
pub(crate) fn filter(
    left3: i32,
    top: i32,
    raw_width: usize,
    height: usize,
    raw: &[u8],
    mode: RasterMode,
) -> GlyphBitmap {
    let left = left3.div_euclid(3) - 1;
    let right = (left3 + raw_width as i32 + 2).div_euclid(3) + 1;
    let width = (right - left) as usize;
    let channels = if mode == RasterMode::Lcd { 3 } else { 1 };
    let mut data = vec![0u8; width * height * channels];
    for y in 0..height {
        for column in 0..width {
            for sub in 0..3 {
                if channels == 1 && sub != 1 {
                    continue;
                }
                let position = left * 3 + (column * 3 + sub) as i32 - left3;
                let mut sum = 0u32;
                for (delta, weight) in [(-2, 1u32), (-1, 2), (0, 3), (1, 2), (2, 1)] {
                    let x = position + delta;
                    if (0..raw_width as i32).contains(&x) {
                        sum += u32::from(raw[y * raw_width + x as usize]) * weight;
                    }
                }
                let value = ((sum + 4) / 9) as u8;
                if channels == 3 {
                    data[(y * width + column) * 3 + sub] = value;
                } else {
                    data[y * width + column] = value;
                }
            }
        }
    }
    GlyphBitmap {
        left,
        top,
        width: width as u32,
        height: height as u32,
        pixels: if channels == 3 {
            Pixels::Lcd
        } else {
            Pixels::Gray
        },
        data,
    }
}

/// Hinting instances keyed by face, variation instance and ppem (LRU).
pub(crate) struct HintCache {
    entries: Vec<(u16, u64, u32, HintingInstance, u64)>,
    clock: u64,
}
pub(crate) const MAX_HINT_INSTANCES: usize = 24;
impl HintCache {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            clock: 0,
        }
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn get(
        &mut self,
        face_index: u16,
        face: &Face,
        instance: u64,
        coords: &[NormalizedCoord],
        ppem: f32,
    ) -> Option<usize> {
        self.clock += 1;
        let ppem64 = (ppem * 64.0) as u32;
        if let Some(i) = self
            .entries
            .iter()
            .position(|e| e.0 == face_index && e.1 == instance && e.2 == ppem64)
        {
            self.entries[i].4 = self.clock;
            return Some(i);
        }
        let hinting = HintingInstance::new(
            &face.outlines,
            Size::new(ppem),
            LocationRef::new(coords),
            HintingOptions {
                engine: Engine::Interpreter,
                target: HintTarget::Smooth {
                    mode: SmoothMode::Lcd,
                    symmetric_rendering: true,
                    preserve_linear_metrics: true,
                },
            },
        )
        .ok()?;
        if self.entries.len() >= MAX_HINT_INSTANCES {
            let oldest = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.4)
                .map(|(i, _)| i)?;
            self.entries.swap_remove(oldest);
        }
        self.entries
            .push((face_index, instance, ppem64, hinting, self.clock));
        Some(self.entries.len() - 1)
    }
    pub fn instance(&self, slot: usize) -> &HintingInstance {
        &self.entries[slot].3
    }
}

/// Draw one outline glyph (hinted when available) into a pixel-space pen.
pub(crate) fn outline(
    face: &Face,
    glyph: u32,
    ppem: f32,
    coords: &[NormalizedCoord],
    hint: Option<&HintingInstance>,
    scratch: &mut Vec<u8>,
) -> Option<PathPen> {
    let outline = face.outlines.get(GlyphId::new(glyph))?;
    let mut pen = PathPen::new(ppem * 16.0 + 64.0);
    let result = match hint {
        Some(instance) => {
            let size = outline.draw_memory_size(Hinting::Embedded);
            if size > 4 * 1024 * 1024 {
                return None;
            }
            scratch.clear();
            scratch.resize(size, 0);
            outline.draw(
                DrawSettings::hinted(instance, true).with_memory(Some(&mut scratch[..])),
                &mut pen,
            )
        }
        None => outline.draw(
            DrawSettings::unhinted(Size::new(ppem), LocationRef::new(coords)),
            &mut pen,
        ),
    };
    (result.is_ok() && !pen.invalid).then_some(pen)
}

/// Rasterize an outline glyph into LCD or grayscale coverage.
#[allow(clippy::too_many_arguments)]
pub(crate) fn rasterize_outline(
    face: &Face,
    glyph: u32,
    ppem: f32,
    coords: &[NormalizedCoord],
    hint: Option<&HintingInstance>,
    mode: RasterMode,
    phase: u8,
    embolden_samples: usize,
    scratch: &mut Vec<u8>,
) -> GlyphBitmap {
    let empty = GlyphBitmap::empty(if mode == RasterMode::Lcd {
        Pixels::Lcd
    } else {
        Pixels::Gray
    });
    let Some(pen) = outline(face, glyph, ppem, coords, hint, scratch) else {
        return empty;
    };
    let Some((min, max)) = pen.bounds() else {
        return empty;
    };
    let Some((left3, top, mut width, height, mut raw)) =
        coverage(&pen.segments, min, max, 3.0, f32::from(phase))
    else {
        return empty;
    };
    if embolden_samples > 0 {
        embolden(&mut raw, &mut width, height, embolden_samples);
    }
    if !raw.iter().any(|&c| c != 0) {
        return empty;
    }
    filter(left3, top, width, height, &raw, mode)
}
