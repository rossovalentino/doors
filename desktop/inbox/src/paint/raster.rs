//! Paint's pixel engine: a document of premultiplied BGRA layers (the
//! background opaque white), build-up-free strokes for the nine brushes, the
//! eraser and the pencil (a per-stroke coverage mask over a copy of the
//! layer), anti-aliased shapes (outline and fill), flood fill with
//! tolerance, whole-image transforms, Paint 3D's lit 3D shapes and an undo
//! history of changed rectangles bounded by bytes.
use alloc::{vec, vec::Vec};

/// A straight (not premultiplied) colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}
impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    /// Premultiplied BGRA for painting.
    pub const fn bgra(self) -> [u8; 4] {
        let a = self.a as u32;
        [
            (self.b as u32 * a / 255) as u8,
            (self.g as u32 * a / 255) as u8,
            (self.r as u32 * a / 255) as u8,
            self.a,
        ]
    }
    /// From premultiplied BGRA.
    pub fn from_bgra(p: [u8; 4]) -> Self {
        if p[3] == 0 {
            return Self {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            };
        }
        let un = |c: u8| ((c as u32 * 255 + p[3] as u32 / 2) / p[3] as u32).min(255) as u8;
        Self {
            r: un(p[2]),
            g: un(p[1]),
            b: un(p[0]),
            a: p[3],
        }
    }
}

pub struct Layer {
    pub px: Vec<u8>,
    pub visible: bool,
}
pub struct Doc {
    pub w: usize,
    pub h: usize,
    pub layers: Vec<Layer>,
    pub active: usize,
}
pub const MAX_LAYERS: usize = 6;
impl Doc {
    pub fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            layers: vec![Layer {
                px: white(w, h),
                visible: true,
            }],
            active: 0,
        }
    }
    pub fn layer(&self) -> &[u8] {
        &self.layers[self.active].px
    }
    pub fn layer_mut(&mut self) -> &mut Vec<u8> {
        &mut self.layers[self.active].px
    }
    /// The visible layers, bottom first, over transparency.
    pub fn composite(&self, out: &mut Vec<u8>) {
        out.clear();
        out.resize(self.w * self.h * 4, 0);
        let mut first = true;
        for l in self.layers.iter().filter(|l| l.visible) {
            if first {
                out.copy_from_slice(&l.px);
                first = false;
                continue;
            }
            for (d, s) in out.chunks_exact_mut(4).zip(l.px.chunks_exact(4)) {
                over(d, [s[0], s[1], s[2], s[3]]);
            }
        }
    }
    /// The visible image as an opaque export (transparency over white).
    pub fn flatten(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.composite(&mut out);
        for p in out.chunks_exact_mut(4) {
            over_under_white(p);
        }
        out
    }
    pub fn pixel(&self, x: i32, y: i32) -> Option<[u8; 4]> {
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= self.h {
            return None;
        }
        let mut out = [0u8; 4];
        for l in self.layers.iter().filter(|l| l.visible) {
            let at = (y as usize * self.w + x as usize) * 4;
            over(
                &mut out,
                [l.px[at], l.px[at + 1], l.px[at + 2], l.px[at + 3]],
            );
        }
        Some(out)
    }
}
pub fn white(w: usize, h: usize) -> Vec<u8> {
    vec![255; w * h * 4]
}
/// Source-over of premultiplied `s` onto `d`.
#[inline]
pub fn over(d: &mut [u8], s: [u8; 4]) {
    let ia = 255 - s[3] as u32;
    for c in 0..4 {
        d[c] = (s[c] as u32 + (d[c] as u32 * ia + 127) / 255).min(255) as u8;
    }
}
fn over_under_white(p: &mut [u8]) {
    let ia = 255 - p[3] as u32;
    for c in p.iter_mut().take(3) {
        *c = (*c as u32 + ia).min(255) as u8;
    }
    p[3] = 255;
}
/// `color` at coverage `k` (0..=255) over `d`.
#[inline]
fn blend(d: &mut [u8], color: [u8; 4], k: u32) {
    if k == 0 {
        return;
    }
    let s = [
        (color[0] as u32 * k / 255) as u8,
        (color[1] as u32 * k / 255) as u8,
        (color[2] as u32 * k / 255) as u8,
        (color[3] as u32 * k / 255) as u8,
    ];
    over(d, s);
}
/// Erase to transparency by coverage `k`.
#[inline]
fn erase(d: &mut [u8], k: u32) {
    let keep = 255 - k;
    for c in d.iter_mut().take(4) {
        *c = (*c as u32 * keep / 255) as u8;
    }
}

/// An integer pixel rectangle, half-open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}
impl Bounds {
    pub fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self { x0, y0, x1, y1 }
    }
    pub fn union(self, o: Self) -> Self {
        Self::new(
            self.x0.min(o.x0),
            self.y0.min(o.y0),
            self.x1.max(o.x1),
            self.y1.max(o.y1),
        )
    }
    pub fn clamp(self, w: usize, h: usize) -> Option<Self> {
        let b = Self::new(
            self.x0.max(0),
            self.y0.max(0),
            self.x1.min(w as i32),
            self.y1.min(h as i32),
        );
        (b.x0 < b.x1 && b.y0 < b.y1).then_some(b)
    }
    pub fn width(self) -> usize {
        (self.x1 - self.x0).max(0) as usize
    }
    pub fn height(self) -> usize {
        (self.y1 - self.y0).max(0) as usize
    }
}
pub fn crop(px: &[u8], w: usize, b: Bounds) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.width() * b.height() * 4);
    for y in b.y0..b.y1 {
        let at = (y as usize * w + b.x0 as usize) * 4;
        out.extend_from_slice(&px[at..at + b.width() * 4]);
    }
    out
}
pub fn paste(px: &mut [u8], w: usize, b: Bounds, src: &[u8]) {
    let row = b.width() * 4;
    for (i, y) in (b.y0..b.y1).enumerate() {
        let at = (y as usize * w + b.x0 as usize) * 4;
        px[at..at + row].copy_from_slice(&src[i * row..(i + 1) * row]);
    }
}

// ---- brushes ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Brush {
    Brush,
    CalligraphyBrush,
    CalligraphyPen,
    Airbrush,
    Oil,
    Crayon,
    Marker,
    NaturalPencil,
    Watercolor,
}
impl Brush {
    pub const ALL: [Self; 9] = [
        Self::Brush,
        Self::CalligraphyBrush,
        Self::CalligraphyPen,
        Self::Airbrush,
        Self::Oil,
        Self::Crayon,
        Self::Marker,
        Self::NaturalPencil,
        Self::Watercolor,
    ];
    pub const fn label(self) -> &'static str {
        match self {
            Self::Brush => "Brush",
            Self::CalligraphyBrush => "Calligraphy brush",
            Self::CalligraphyPen => "Calligraphy pen",
            Self::Airbrush => "Airbrush",
            Self::Oil => "Oil brush",
            Self::Crayon => "Crayon",
            Self::Marker => "Marker",
            Self::NaturalPencil => "Natural pencil",
            Self::Watercolor => "Watercolor brush",
        }
    }
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Brush => "brush",
            Self::CalligraphyBrush => "calligraphy-brush",
            Self::CalligraphyPen => "calligraphy-pen",
            Self::Airbrush => "airbrush",
            Self::Oil => "oil",
            Self::Crayon => "crayon",
            Self::Marker => "marker",
            Self::NaturalPencil => "natural-pencil",
            Self::Watercolor => "watercolor",
        }
    }
    /// Media opacity on top of the opacity slider.
    const fn strength(self) -> u32 {
        match self {
            Self::Marker => 190,
            Self::NaturalPencil => 200,
            Self::Watercolor => 110,
            _ => 255,
        }
    }
}
/// What a stroke lays down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tip {
    /// Hard square pixels (the pencil).
    Pencil,
    Brush(Brush),
    /// Square, anti-aliased; erases (layers) or paints the secondary colour
    /// (the background layer).
    Eraser,
}

/// Deterministic noise in 0..=255.
#[inline]
fn hash(x: i32, y: i32, seed: u32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x27d4_eb2d) ^ (y as u32).wrapping_mul(0x1656_67b1) ^ seed;
    h ^= h >> 15;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h & 255
}
#[inline]
fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

/// One stroke or shape preview in progress over a copy of the layer.
pub struct Edit {
    pub base: Vec<u8>,
    mask: Vec<u8>,
    pub dirty: Option<Bounds>,
    last: (f32, f32),
    tip: Tip,
    radius: f32,
    color: [u8; 4],
    opacity: u32,
    erase_to: Option<[u8; 4]>,
    seed: u32,
    travelled: f32,
    fill_bounds: Option<Bounds>,
}
impl Edit {
    /// `erase_to`: the eraser's colour on the background layer (None erases).
    pub fn begin(
        doc: &Doc,
        tip: Tip,
        size: u32,
        color: Rgba,
        opacity: u8,
        erase_to: Option<Rgba>,
    ) -> Self {
        Self {
            base: doc.layer().to_vec(),
            mask: vec![0; doc.w * doc.h],
            dirty: None,
            last: (0.0, 0.0),
            tip,
            radius: size.max(1) as f32 / 2.0,
            color: color.bgra(),
            opacity: opacity as u32,
            erase_to: erase_to.map(Rgba::bgra),
            seed: 0x9e37_79b9,
            travelled: 0.0,
            fill_bounds: None,
        }
    }
    fn mark(&mut self, b: Bounds) {
        self.dirty = Some(self.dirty.map_or(b, |d| d.union(b)));
    }
    /// Coverage of one stamp at (cx, cy) for pixel (x, y) (0..=1).
    fn coverage(&self, x: i32, y: i32, cx: f32, cy: f32, direction: (f32, f32)) -> f32 {
        let (px, py) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
        let r = self.radius;
        match self.tip {
            Tip::Pencil => {
                let half = r.max(0.5);
                if px.abs() <= half && py.abs() <= half {
                    1.0
                } else {
                    0.0
                }
            }
            Tip::Eraser => clamp01(r + 0.5 - px.abs()) * clamp01(r + 0.5 - py.abs()),
            Tip::Brush(b) => match b {
                Brush::Brush | Brush::Marker => clamp01(r + 0.5 - super::sqrt_f(px * px + py * py)),
                Brush::CalligraphyBrush | Brush::CalligraphyPen => {
                    // A flat nib at 45° (the pen at -45°, thinner).
                    let h = core::f32::consts::FRAC_1_SQRT_2;
                    let (ux, uy) = if b == Brush::CalligraphyBrush {
                        (h, -h)
                    } else {
                        (h, h)
                    };
                    let along = px * ux + py * uy;
                    let across = -px * uy + py * ux;
                    let thin = if b == Brush::CalligraphyBrush {
                        0.22
                    } else {
                        0.12
                    };
                    let a = r;
                    let bb = (r * thin).max(0.7);
                    let d = (along / a) * (along / a) + (across / bb) * (across / bb);
                    clamp01((1.0 - d) * bb)
                }
                Brush::Oil => {
                    let d = super::sqrt_f(px * px + py * py);
                    let edge = clamp01(r + 0.5 - d);
                    // Bristle streaks across the stroke direction.
                    let across = -px * direction.1 + py * direction.0;
                    let k = hash((across * 1.7) as i32, 7, self.seed) as f32 / 255.0;
                    edge * (0.55 + 0.45 * k)
                }
                Brush::Crayon => {
                    let d = super::sqrt_f(px * px + py * py);
                    let k = hash(x, y, self.seed) as f32 / 255.0;
                    if k > 0.38 { clamp01(r + 0.5 - d) } else { 0.0 }
                }
                Brush::NaturalPencil => {
                    let d = super::sqrt_f(px * px + py * py);
                    let k = hash(x, y, self.seed ^ 77) as f32 / 255.0;
                    clamp01(r + 0.5 - d) * (0.45 + 0.55 * k)
                }
                Brush::Watercolor => {
                    let d = super::sqrt_f(px * px + py * py) / r.max(1.0);
                    clamp01(1.0 - d * d) * 0.85 + if d > 0.8 && d < 1.0 { 0.15 } else { 0.0 }
                }
                Brush::Airbrush => 0.0,
            },
        }
    }
    fn stamp(&mut self, w: usize, h: usize, cx: f32, cy: f32, direction: (f32, f32)) {
        let r = self.radius + 1.5;
        let Some(b) = Bounds::new(
            (cx - r) as i32 - 1,
            (cy - r) as i32 - 1,
            (cx + r) as i32 + 2,
            (cy + r) as i32 + 2,
        )
        .clamp(w, h) else {
            return;
        };
        if self.tip == Tip::Brush(Brush::Airbrush) {
            // A spray of single dots, accumulating.
            let n = (self.radius * self.radius * 0.35).clamp(3.0, 400.0) as u32;
            for i in 0..n {
                self.seed = self
                    .seed
                    .wrapping_mul(1_664_525)
                    .wrapping_add(1_013_904_223);
                let a = (self.seed >> 8) as f32 / 16_777_216.0 * core::f32::consts::TAU;
                let d = super::sqrt_f((hash(i as i32, 3, self.seed) as f32) / 255.0) * self.radius;
                let (s, c) = super::sin_cos_f(a);
                let (x, y) = ((cx + c * d) as i32, (cy + s * d) as i32);
                if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                    let m = &mut self.mask[y as usize * w + x as usize];
                    *m = (*m as u32 + 160).min(255) as u8;
                }
            }
            self.mark(b);
            return;
        }
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let k = (self.coverage(x, y, cx, cy, direction) * 255.0) as u32;
                let m = &mut self.mask[y as usize * w + x as usize];
                if k > *m as u32 {
                    *m = k as u8;
                }
            }
        }
        self.mark(b);
    }
    pub fn start(&mut self, doc: &mut Doc, x: f32, y: f32) {
        self.last = (x, y);
        self.stamp(doc.w, doc.h, x, y, (1.0, 0.0));
        self.apply(doc);
    }
    /// Continue the stroke to (x, y), stamping along the segment.
    pub fn to(&mut self, doc: &mut Doc, x: f32, y: f32) {
        let (x0, y0) = self.last;
        let (dx, dy) = (x - x0, y - y0);
        let len = super::sqrt_f(dx * dx + dy * dy);
        if len < 0.01 {
            return;
        }
        let dir = (dx / len, dy / len);
        let spacing = match self.tip {
            Tip::Pencil => 0.5,
            Tip::Brush(Brush::Airbrush) => (self.radius * 0.6).max(2.0),
            _ => (self.radius * 0.25).clamp(0.5, 4.0),
        };
        let mut t = spacing - self.travelled.min(spacing);
        while t <= len {
            self.stamp(doc.w, doc.h, x0 + dir.0 * t, y0 + dir.1 * t, dir);
            t += spacing;
        }
        self.travelled = len - (t - spacing);
        self.last = (x, y);
        self.apply(doc);
    }
    /// Recompose the dirty rectangle from the base and the mask.
    pub fn apply(&mut self, doc: &mut Doc) {
        let Some(b) = self.dirty.and_then(|d| d.clamp(doc.w, doc.h)) else {
            return;
        };
        let w = doc.w;
        let strength = match self.tip {
            Tip::Brush(br) => br.strength(),
            _ => 255,
        };
        let layer = &mut doc.layers[doc.active].px;
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let i = y as usize * w + x as usize;
                let at = i * 4;
                let d = &mut layer[at..at + 4];
                d.copy_from_slice(&self.base[at..at + 4]);
                let m = self.mask[i] as u32;
                if m == 0 {
                    continue;
                }
                let k = m * self.opacity / 255 * strength / 255;
                match (self.tip, self.erase_to) {
                    (Tip::Eraser, None) => erase(d, k),
                    (Tip::Eraser, Some(c)) => blend(d, c, k),
                    _ => blend(d, self.color, k),
                }
            }
        }
    }
    /// Clear the coverage (a shape preview restarts each motion).
    pub fn reset(&mut self, doc: &mut Doc) {
        if let Some(b) = self.dirty.and_then(|d| d.clamp(doc.w, doc.h)) {
            for y in b.y0..b.y1 {
                let i = y as usize * doc.w;
                self.mask[i + b.x0 as usize..i + b.x1 as usize].fill(0);
            }
            self.fill_bounds = None;
            self.apply(doc);
            self.dirty = None;
        }
    }
    /// Shape coverage straight into the mask: `fill` in `fill_color` first
    /// (painted at once), then the outline in the stroke colour.
    pub fn shape(
        &mut self,
        doc: &mut Doc,
        poly: &[(f32, f32)],
        closed: bool,
        outline: Option<(Rgba, u32)>,
        fill: Option<Rgba>,
    ) {
        self.reset(doc);
        self.fill_bounds = None;
        if poly.len() < 2 {
            return;
        }
        let (w, h) = (doc.w, doc.h);
        if let Some(fc) = fill
            && closed
        {
            // Fill composited directly over the base (it may differ from the
            // outline colour), recorded as dirty.
            let mut cov = vec![0u8; 0];
            if let Some(b) = fill_polygon(poly, w, h, &mut cov) {
                let layer = &mut doc.layers[doc.active].px;
                let c = fc.bgra();
                let bw = b.width();
                for y in b.y0..b.y1 {
                    for x in b.x0..b.x1 {
                        let k = cov[(y - b.y0) as usize * bw + (x - b.x0) as usize] as u32;
                        let at = (y as usize * w + x as usize) * 4;
                        let mut px = [
                            self.base[at],
                            self.base[at + 1],
                            self.base[at + 2],
                            self.base[at + 3],
                        ];
                        blend(&mut px, c, k * self.opacity / 255);
                        layer[at..at + 4].copy_from_slice(&px);
                    }
                }
                self.mark(b);
                self.fill_bounds = Some(b);
            }
        }
        if let Some((c, size)) = outline {
            self.color = c.bgra();
            self.radius = size.max(1) as f32 / 2.0;
            let n = poly.len();
            let segments = if closed { n } else { n - 1 };
            for i in 0..segments {
                let (a, b) = (poly[i], poly[(i + 1) % n]);
                self.capsule(w, h, a, b);
            }
        }
        self.apply_over_fill(doc);
    }
}

// The shape path keeps the fill composited in a shadow layer so `apply`
// (which restarts from the base) can lay the outline over it.
impl Edit {
    fn capsule(&mut self, w: usize, h: usize, a: (f32, f32), b: (f32, f32)) {
        let r = self.radius;
        let Some(bb) = Bounds::new(
            (a.0.min(b.0) - r) as i32 - 2,
            (a.1.min(b.1) - r) as i32 - 2,
            (a.0.max(b.0) + r) as i32 + 3,
            (a.1.max(b.1) + r) as i32 + 3,
        )
        .clamp(w, h) else {
            return;
        };
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = (dx * dx + dy * dy).max(1e-6);
        for y in bb.y0..bb.y1 {
            for x in bb.x0..bb.x1 {
                let (px, py) = (x as f32 + 0.5 - a.0, y as f32 + 0.5 - a.1);
                let t = ((px * dx + py * dy) / len2).clamp(0.0, 1.0);
                let (ex, ey) = (px - dx * t, py - dy * t);
                let d = super::sqrt_f(ex * ex + ey * ey);
                let k = (clamp01(r + 0.5 - d) * 255.0) as u32;
                let m = &mut self.mask[y as usize * w + x as usize];
                if k > *m as u32 {
                    *m = k as u8;
                }
            }
        }
        self.mark(bb);
    }
    fn apply_over_fill(&mut self, doc: &mut Doc) {
        let Some(b) = self.dirty.and_then(|d| d.clamp(doc.w, doc.h)) else {
            return;
        };
        let w = doc.w;
        let filled = self.fill_bounds;
        let layer = &mut doc.layers[doc.active].px;
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let i = y as usize * w + x as usize;
                let at = i * 4;
                let inside_fill =
                    filled.is_some_and(|f| x >= f.x0 && x < f.x1 && y >= f.y0 && y < f.y1);
                if !inside_fill {
                    layer[at..at + 4].copy_from_slice(&self.base[at..at + 4]);
                }
                let m = self.mask[i] as u32;
                if m != 0 {
                    blend(&mut layer[at..at + 4], self.color, m * self.opacity / 255);
                }
            }
        }
    }
}

/// Anti-aliased even-odd polygon coverage (4 sub-scanlines, exact spans);
/// `cov` receives the bounds' coverage row-major.
pub fn fill_polygon(poly: &[(f32, f32)], w: usize, h: usize, cov: &mut Vec<u8>) -> Option<Bounds> {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in poly {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let b = Bounds::new(x0 as i32 - 1, y0 as i32 - 1, x1 as i32 + 2, y1 as i32 + 2).clamp(w, h)?;
    let bw = b.width();
    cov.clear();
    cov.resize(bw * b.height(), 0);
    let mut acc = vec![0f32; bw + 1];
    let mut xs: Vec<f32> = Vec::new();
    const SUB: usize = 4;
    for y in b.y0..b.y1 {
        acc.iter_mut().for_each(|a| *a = 0.0);
        for s in 0..SUB {
            let sy = y as f32 + (s as f32 + 0.5) / SUB as f32;
            xs.clear();
            for i in 0..poly.len() {
                let (a, c) = (poly[i], poly[(i + 1) % poly.len()]);
                if (a.1 <= sy && c.1 > sy) || (c.1 <= sy && a.1 > sy) {
                    xs.push(a.0 + (sy - a.1) / (c.1 - a.1) * (c.0 - a.0));
                }
            }
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
            for pair in xs.chunks_exact(2) {
                let (l, r) = (
                    (pair[0] - b.x0 as f32).max(0.0),
                    (pair[1] - b.x0 as f32).min(bw as f32),
                );
                if r <= l {
                    continue;
                }
                let (li, ri) = (l as usize, r as usize);
                if li == ri {
                    acc[li] += r - l;
                } else {
                    acc[li] += (li + 1) as f32 - l;
                    for a in acc.iter_mut().take(ri).skip(li + 1) {
                        *a += 1.0;
                    }
                    if ri < bw {
                        acc[ri] += r - ri as f32;
                    }
                }
            }
        }
        let row = (y - b.y0) as usize * bw;
        for x in 0..bw {
            cov[row + x] = (acc[x] / SUB as f32 * 255.0).min(255.0) as u8;
        }
    }
    Some(b)
}

// ---- shapes ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Line,
    Curve,
    Oval,
    Rectangle,
    RoundedRectangle,
    Polygon,
    Triangle,
    RightTriangle,
    Diamond,
    Pentagon,
    Hexagon,
    RightArrow,
    LeftArrow,
    UpArrow,
    DownArrow,
    Star4,
    Star5,
    Star6,
    RoundedCallout,
    OvalCallout,
    CloudCallout,
    Heart,
    Lightning,
}
impl Shape {
    /// The gallery, in Paint's order.
    pub const ALL: [Self; 23] = [
        Self::Line,
        Self::Curve,
        Self::Oval,
        Self::Rectangle,
        Self::RoundedRectangle,
        Self::Polygon,
        Self::Triangle,
        Self::RightTriangle,
        Self::Diamond,
        Self::Pentagon,
        Self::Hexagon,
        Self::RightArrow,
        Self::LeftArrow,
        Self::UpArrow,
        Self::DownArrow,
        Self::Star4,
        Self::Star5,
        Self::Star6,
        Self::RoundedCallout,
        Self::OvalCallout,
        Self::CloudCallout,
        Self::Heart,
        Self::Lightning,
    ];
    pub const fn icon(self) -> &'static str {
        match self {
            Self::Line => "line",
            Self::Curve => "curve",
            Self::Oval => "oval",
            Self::Rectangle => "rectangle",
            Self::RoundedRectangle => "rounded-rectangle",
            Self::Polygon => "polygon",
            Self::Triangle => "triangle",
            Self::RightTriangle => "right-triangle",
            Self::Diamond => "diamond",
            Self::Pentagon => "pentagon",
            Self::Hexagon => "hexagon",
            Self::RightArrow => "right-arrow",
            Self::LeftArrow => "left-arrow",
            Self::UpArrow => "up-arrow",
            Self::DownArrow => "down-arrow",
            Self::Star4 => "star4",
            Self::Star5 => "star5",
            Self::Star6 => "star6",
            Self::RoundedCallout => "rounded-callout",
            Self::OvalCallout => "oval-callout",
            Self::CloudCallout => "cloud-callout",
            Self::Heart => "heart",
            Self::Lightning => "lightning",
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Line => "Line",
            Self::Curve => "Curve",
            Self::Oval => "Oval",
            Self::Rectangle => "Rectangle",
            Self::RoundedRectangle => "Rounded rectangle",
            Self::Polygon => "Polygon",
            Self::Triangle => "Triangle",
            Self::RightTriangle => "Right triangle",
            Self::Diamond => "Diamond",
            Self::Pentagon => "Pentagon",
            Self::Hexagon => "Hexagon",
            Self::RightArrow => "Right arrow",
            Self::LeftArrow => "Left arrow",
            Self::UpArrow => "Up arrow",
            Self::DownArrow => "Down arrow",
            Self::Star4 => "Four-point star",
            Self::Star5 => "Five-point star",
            Self::Star6 => "Six-point star",
            Self::RoundedCallout => "Rounded rectangular callout",
            Self::OvalCallout => "Oval callout",
            Self::CloudCallout => "Cloud callout",
            Self::Heart => "Heart",
            Self::Lightning => "Lightning",
        }
    }
    /// Open strokes (no fill).
    pub const fn open(self) -> bool {
        matches!(self, Self::Line | Self::Curve)
    }
}
fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32, from: f32, to: f32, out: &mut Vec<(f32, f32)>) {
    let n = ((rx.abs() + ry.abs()) * 0.6).clamp(12.0, 160.0) as usize;
    for i in 0..=n {
        let a = from + (to - from) * i as f32 / n as f32;
        let (s, c) = super::sin_cos_f(a);
        out.push((cx + c * rx, cy + s * ry));
    }
}
fn regular(
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    n: usize,
    inner: Option<f32>,
    out: &mut Vec<(f32, f32)>,
) {
    let k = if inner.is_some() { 2 * n } else { n };
    for i in 0..k {
        let a = -core::f32::consts::FRAC_PI_2 + core::f32::consts::TAU * i as f32 / k as f32;
        let f = if i % 2 == 1 {
            inner.unwrap_or(1.0)
        } else {
            1.0
        };
        let (s, c) = super::sin_cos_f(a);
        out.push((cx + c * rx * f, cy + s * ry * f));
    }
}
/// The outline of `shape` in the box from (x0, y0) to (x1, y1) (drag
/// order: a line keeps its direction).
pub fn outline(shape: Shape, x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<(f32, f32)> {
    let (l, t, r, b) = (x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1));
    let (w, h) = (r - l, b - t);
    let (cx, cy) = ((l + r) / 2.0, (t + b) / 2.0);
    let mut p = Vec::new();
    use core::f32::consts::{FRAC_PI_2, PI, TAU};
    match shape {
        Shape::Line | Shape::Curve | Shape::Polygon => {
            p.push((x0, y0));
            p.push((x1, y1));
        }
        Shape::Oval => ellipse(cx, cy, w / 2.0, h / 2.0, 0.0, TAU, &mut p),
        Shape::Rectangle => p.extend([(l, t), (r, t), (r, b), (l, b)]),
        Shape::RoundedRectangle | Shape::RoundedCallout => {
            let rad = (w.min(h) * 0.18).min(24.0);
            let body_b = if shape == Shape::RoundedCallout {
                t + h * 0.78
            } else {
                b
            };
            ellipse(r - rad, t + rad, rad, rad, -FRAC_PI_2, 0.0, &mut p);
            ellipse(r - rad, body_b - rad, rad, rad, 0.0, FRAC_PI_2, &mut p);
            if shape == Shape::RoundedCallout {
                p.push((l + w * 0.42, body_b));
                p.push((l + w * 0.22, b));
                p.push((l + w * 0.28, body_b));
            }
            ellipse(l + rad, body_b - rad, rad, rad, FRAC_PI_2, PI, &mut p);
            ellipse(l + rad, t + rad, rad, rad, PI, PI + FRAC_PI_2, &mut p);
        }
        Shape::OvalCallout => {
            let body = h * 0.8;
            let (ecy, ery) = (t + body / 2.0, body / 2.0);
            ellipse(cx, ecy, w / 2.0, ery, -1.2, 1.75, &mut p);
            p.push((l + w * 0.22, b));
            ellipse(cx, ecy, w / 2.0, ery, 2.05, TAU - 1.2, &mut p);
        }
        Shape::CloudCallout => {
            // Bumps around an ellipse; the tail is two small puffs.
            let body = h * 0.78;
            let (ecy, rx, ry) = (t + body / 2.0, w / 2.0, body / 2.0);
            let bumps = 9;
            for i in 0..bumps {
                let a0 = TAU * i as f32 / bumps as f32;
                let a1 = TAU * (i + 1) as f32 / bumps as f32;
                let (s0, c0) = super::sin_cos_f(a0);
                let (s1, c1) = super::sin_cos_f(a1);
                let (mx, my) = (
                    cx + (c0 + c1) * 0.5 * rx * 0.86,
                    ecy + (s0 + s1) * 0.5 * ry * 0.86,
                );
                let (ex, ey) = ((c1 - c0) * rx, (s1 - s0) * ry);
                let br = super::sqrt_f(ex * ex + ey * ey) * 0.62;
                let mid = (a0 + a1) / 2.0;
                ellipse(mx, my, br, br, mid - 1.6, mid + 1.6, &mut p);
            }
        }
        Shape::Triangle => p.extend([(cx, t), (r, b), (l, b)]),
        Shape::RightTriangle => p.extend([(l, t), (r, b), (l, b)]),
        Shape::Diamond => p.extend([(cx, t), (r, cy), (cx, b), (l, cy)]),
        Shape::Pentagon => regular(cx, cy + h * 0.05, w / 2.0, h / 2.0 * 1.05, 5, None, &mut p),
        Shape::Hexagon => p.extend([
            (l + w * 0.25, t),
            (l + w * 0.75, t),
            (r, cy),
            (l + w * 0.75, b),
            (l + w * 0.25, b),
            (l, cy),
        ]),
        Shape::RightArrow => p.extend([
            (l, t + h * 0.3),
            (l + w * 0.6, t + h * 0.3),
            (l + w * 0.6, t),
            (r, cy),
            (l + w * 0.6, b),
            (l + w * 0.6, t + h * 0.7),
            (l, t + h * 0.7),
        ]),
        Shape::LeftArrow => p.extend([
            (r, t + h * 0.3),
            (l + w * 0.4, t + h * 0.3),
            (l + w * 0.4, t),
            (l, cy),
            (l + w * 0.4, b),
            (l + w * 0.4, t + h * 0.7),
            (r, t + h * 0.7),
        ]),
        Shape::UpArrow => p.extend([
            (l + w * 0.3, b),
            (l + w * 0.3, t + h * 0.4),
            (l, t + h * 0.4),
            (cx, t),
            (r, t + h * 0.4),
            (l + w * 0.7, t + h * 0.4),
            (l + w * 0.7, b),
        ]),
        Shape::DownArrow => p.extend([
            (l + w * 0.3, t),
            (l + w * 0.3, t + h * 0.6),
            (l, t + h * 0.6),
            (cx, b),
            (r, t + h * 0.6),
            (l + w * 0.7, t + h * 0.6),
            (l + w * 0.7, t),
        ]),
        Shape::Star4 => regular(cx, cy, w / 2.0, h / 2.0, 4, Some(0.38), &mut p),
        Shape::Star5 => regular(
            cx,
            cy + h * 0.05,
            w / 2.0,
            h / 2.0 * 1.05,
            5,
            Some(0.42),
            &mut p,
        ),
        Shape::Star6 => regular(cx, cy, w / 2.0, h / 2.0, 6, Some(0.58), &mut p),
        Shape::Heart => {
            let n = 120;
            for i in 0..n {
                let a = TAU * i as f32 / n as f32;
                let (s, c) = super::sin_cos_f(a);
                let (s3, c2, c3, c4) = (
                    s * s * s,
                    (2.0 * a).cos_f(),
                    (3.0 * a).cos_f(),
                    (4.0 * a).cos_f(),
                );
                let hx = 16.0 * s3;
                let hy = 13.0 * c - 5.0 * c2 - 2.0 * c3 - c4;
                p.push((cx + hx / 34.0 * w, cy - hy / 30.0 * h - h * 0.06));
            }
        }
        Shape::Lightning => {
            let pts = [
                (0.38, 0.0),
                (0.62, 0.32),
                (0.52, 0.36),
                (0.8, 0.62),
                (0.68, 0.66),
                (1.0, 1.0),
                (0.42, 0.72),
                (0.54, 0.68),
                (0.18, 0.44),
                (0.32, 0.4),
                (0.0, 0.14),
            ];
            p.extend(pts.iter().map(|(u, v)| (l + u * w, t + v * h)));
        }
    }
    p
}
trait CosF {
    fn cos_f(self) -> f32;
}
impl CosF for f32 {
    fn cos_f(self) -> f32 {
        super::sin_cos_f(self).1
    }
}
/// A quadratic Bézier from a to b bent toward c.
pub fn bezier(a: (f32, f32), c: (f32, f32), b: (f32, f32)) -> Vec<(f32, f32)> {
    let n = 48;
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            (
                u * u * a.0 + 2.0 * u * t * c.0 + t * t * b.0,
                u * u * a.1 + 2.0 * u * t * c.1 + t * t * b.1,
            )
        })
        .collect()
}

// ---- fill ----

/// Flood fill the active layer from (x, y) with `color`; `tolerance` is
/// 0..=100 (percent of the largest channel difference). Returns the
/// changed bounds.
pub fn flood(
    doc: &mut Doc,
    x: i32,
    y: i32,
    color: Rgba,
    tolerance: u32,
    opacity: u8,
) -> Option<Bounds> {
    let (w, h) = (doc.w, doc.h);
    if x < 0 || y < 0 || x as usize >= w || y as usize >= h {
        return None;
    }
    let layer = &mut doc.layers[doc.active].px;
    let at = (y as usize * w + x as usize) * 4;
    let seed = [layer[at], layer[at + 1], layer[at + 2], layer[at + 3]];
    let limit = tolerance.min(100) * 255 / 100;
    let mut fill = color;
    fill.a = (fill.a as u32 * opacity as u32 / 255) as u8;
    let paint = fill.bgra();
    if paint == seed && opacity == 255 {
        return None;
    }
    let close = |p: &[u8]| (0..4).all(|c| (p[c] as i32 - seed[c] as i32).unsigned_abs() <= limit);
    let mut seen = vec![false; w * h];
    let mut stack = vec![(x as usize, y as usize)];
    let mut b = Bounds::new(x, y, x + 1, y + 1);
    while let Some((sx, sy)) = stack.pop() {
        let row = sy * w;
        if seen[row + sx] {
            continue;
        }
        let (mut l, mut r) = (sx, sx);
        while l > 0 && !seen[row + l - 1] && close(&layer[(row + l - 1) * 4..][..4]) {
            l -= 1;
        }
        while r + 1 < w && !seen[row + r + 1] && close(&layer[(row + r + 1) * 4..][..4]) {
            r += 1;
        }
        for i in l..=r {
            seen[row + i] = true;
        }
        for ny in [sy.wrapping_sub(1), sy + 1] {
            if ny >= h {
                continue;
            }
            let nrow = ny * w;
            let mut i = l;
            while i <= r {
                if !seen[nrow + i] && close(&layer[(nrow + i) * 4..][..4]) {
                    stack.push((i, ny));
                    while i <= r && !seen[nrow + i] && close(&layer[(nrow + i) * 4..][..4]) {
                        i += 1;
                    }
                }
                i += 1;
            }
        }
        b = b.union(Bounds::new(
            l as i32,
            sy as i32,
            r as i32 + 1,
            sy as i32 + 1,
        ));
    }
    for (i, s) in seen.iter().enumerate() {
        if *s {
            let d = &mut layer[i * 4..i * 4 + 4];
            if opacity == 255 && color.a == 255 {
                d.copy_from_slice(&paint);
            } else {
                over(d, paint);
            }
        }
    }
    Some(b)
}

// ---- transforms ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    Right,
    Left,
    Half,
    FlipHorizontal,
    FlipVertical,
}
pub fn transform(px: &[u8], w: usize, h: usize, turn: Turn) -> (Vec<u8>, usize, usize) {
    let (nw, nh) = match turn {
        Turn::Right | Turn::Left => (h, w),
        _ => (w, h),
    };
    let mut out = vec![0; px.len()];
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = match turn {
                Turn::Right => (h - 1 - y, x),
                Turn::Left => (y, w - 1 - x),
                Turn::Half => (w - 1 - x, h - 1 - y),
                Turn::FlipHorizontal => (w - 1 - x, y),
                Turn::FlipVertical => (x, h - 1 - y),
            };
            out[(ny * nw + nx) * 4..][..4].copy_from_slice(&px[(y * w + x) * 4..][..4]);
        }
    }
    (out, nw, nh)
}
/// Bilinear resample (premultiplied).
pub fn resample(px: &[u8], w: usize, h: usize, nw: usize, nh: usize) -> Vec<u8> {
    let mut out = vec![0; nw * nh * 4];
    for y in 0..nh {
        let fy = ((y as f32 + 0.5) * h as f32 / nh as f32 - 0.5).max(0.0);
        let (y0, ty) = (fy as usize, fy - (fy as usize) as f32);
        let y1 = (y0 + 1).min(h - 1);
        for x in 0..nw {
            let fx = ((x as f32 + 0.5) * w as f32 / nw as f32 - 0.5).max(0.0);
            let (x0, tx) = (fx as usize, fx - (fx as usize) as f32);
            let x1 = (x0 + 1).min(w - 1);
            for c in 0..4 {
                let p = |xx: usize, yy: usize| px[(yy * w + xx) * 4 + c] as f32;
                let top = p(x0, y0) * (1.0 - tx) + p(x1, y0) * tx;
                let bottom = p(x0, y1) * (1.0 - tx) + p(x1, y1) * tx;
                out[(y * nw + x) * 4 + c] = (top * (1.0 - ty) + bottom * ty + 0.5) as u8;
            }
        }
    }
    out
}

// ---- Paint 3D: lit 3D shapes ----

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Solid {
    Cube,
    Sphere,
    Cylinder,
    Cone,
    Pyramid,
    Capsule,
}
impl Solid {
    pub const ALL: [Self; 6] = [
        Self::Cube,
        Self::Sphere,
        Self::Cylinder,
        Self::Cone,
        Self::Pyramid,
        Self::Capsule,
    ];
    pub const fn label(self) -> &'static str {
        match self {
            Self::Cube => "Cube",
            Self::Sphere => "Sphere",
            Self::Cylinder => "Cylinder",
            Self::Cone => "Cone",
            Self::Pyramid => "Pyramid",
            Self::Capsule => "Capsule",
        }
    }
}
/// Ray-cast `solid` into a `w` x `h` premultiplied BGRA tile, lit from the
/// upper left (Lambert plus a soft specular), in `color`.
pub fn render_solid(solid: Solid, w: usize, h: usize, color: Rgba) -> Vec<u8> {
    let mut out = vec![0u8; w * h * 4];
    let light = norm((-0.45, -0.6, 0.66));
    // Object space: x right, y down, z toward the viewer, unit box.
    // A gentle 3/4 view for the polyhedra: rotate 30° around y, 20° around x.
    let (sy, cy) = super::sin_cos_f(0.52);
    let (sx, cx) = super::sin_cos_f(-0.35);
    let rot = |v: (f32, f32, f32)| -> (f32, f32, f32) {
        let (x, y, z) = v;
        let (x, z) = (x * cy + z * sy, -x * sy + z * cy);
        let (y, z) = (y * cx - z * sx, y * sx + z * cx);
        (x, y, z)
    };
    let inv = |v: (f32, f32, f32)| -> (f32, f32, f32) {
        let (x, y, z) = v;
        let (y, z) = (y * cx + z * sx, -y * sx + z * cx);
        let (x, z) = (x * cy - z * sy, x * sy + z * cy);
        (x, y, z)
    };
    const SS: usize = 3;
    for py in 0..h {
        for px in 0..w {
            let mut acc = [0f32; 4];
            for sj in 0..SS {
                for si in 0..SS {
                    let u = ((px as f32 + (si as f32 + 0.5) / SS as f32) / w as f32) * 2.0 - 1.0;
                    let v = ((py as f32 + (sj as f32 + 0.5) / SS as f32) / h as f32) * 2.0 - 1.0;
                    // Orthographic ray along -z in view space.
                    let hit = match solid {
                        Solid::Sphere => sphere_hit(u, v, 0.0, 0.92),
                        Solid::Capsule => {
                            // Vertical capsule: radius 0.45, segment |y| <= 0.45.
                            let yy = v.clamp(-0.47, 0.47);
                            sphere_hit(u, v - yy, 0.0, 0.47)
                        }
                        _ => {
                            // March the view ray through the rotated solid.
                            let o = inv((u, v, 3.0));
                            let d = inv((0.0, 0.0, -1.0));
                            solid_hit(solid, o, d).map(|n| (rot(n), 0.0))
                        }
                    };
                    if let Some((n, _)) = hit {
                        let n = norm(n);
                        let diffuse = (n.0 * light.0 + n.1 * light.1 + n.2 * light.2).max(0.0);
                        let half = norm((light.0, light.1, light.2 + 1.0));
                        let spec = (n.0 * half.0 + n.1 * half.1 + n.2 * half.2).max(0.0);
                        let spec = pow24(spec) * 0.35;
                        let k = 0.28 + 0.72 * diffuse;
                        let c = [color.b as f32, color.g as f32, color.r as f32];
                        for i in 0..3 {
                            acc[i] += (c[i] * k + 255.0 * spec).min(255.0);
                        }
                        acc[3] += 255.0;
                    }
                }
            }
            let n = (SS * SS) as f32;
            let at = (py * w + px) * 4;
            for c in 0..4 {
                // Accumulated colour is already weighted by hit count:
                // premultiplied by coverage.
                out[at + c] = (acc[c] / n).min(255.0) as u8;
            }
        }
    }
    out
}
fn pow24(v: f32) -> f32 {
    let v2 = v * v;
    let v4 = v2 * v2;
    let v8 = v4 * v4;
    v8 * v8 * v8
}
fn norm(v: (f32, f32, f32)) -> (f32, f32, f32) {
    let l = super::sqrt_f(v.0 * v.0 + v.1 * v.1 + v.2 * v.2).max(1e-6);
    (v.0 / l, v.1 / l, v.2 / l)
}
fn sphere_hit(u: f32, v: f32, cz: f32, r: f32) -> Option<((f32, f32, f32), f32)> {
    let d = r * r - u * u - v * v;
    if d < 0.0 {
        return None;
    }
    let z = super::sqrt_f(d);
    Some(((u / r, v / r, z / r), cz + z))
}
/// Ray (origin, direction) against the solid in object space: the normal
/// at the nearest hit (exact intersections, no marching).
fn solid_hit(solid: Solid, o: (f32, f32, f32), d: (f32, f32, f32)) -> Option<(f32, f32, f32)> {
    match solid {
        Solid::Cube => convex(
            o,
            d,
            &[
                ((1.0, 0.0, 0.0), 0.5),
                ((-1.0, 0.0, 0.0), 0.5),
                ((0.0, 1.0, 0.0), 0.5),
                ((0.0, -1.0, 0.0), 0.5),
                ((0.0, 0.0, 1.0), 0.5),
                ((0.0, 0.0, -1.0), 0.5),
            ],
        ),
        Solid::Pyramid => {
            // Apex (0, -0.6, 0), square base |x|,|z| <= 0.55 at y = 0.5:
            // each side plane n·p <= c through the apex and a base edge.
            let (h, r) = (1.1f32, 0.55f32);
            let l = super::sqrt_f(h * h + r * r);
            let (ny, nh) = (-r / l, h / l);
            let c = -0.6 * ny;
            convex(
                o,
                d,
                &[
                    ((0.0, 1.0, 0.0), 0.5),
                    ((nh, ny, 0.0), c),
                    ((-nh, ny, 0.0), c),
                    ((0.0, ny, nh), c),
                    ((0.0, ny, -nh), c),
                ],
            )
        }
        Solid::Cylinder => {
            let (r, h) = (0.5f32, 0.55f32);
            let mut best: Option<(f32, (f32, f32, f32))> = None;
            let a = d.0 * d.0 + d.2 * d.2;
            if a > 1e-8 {
                let b = 2.0 * (o.0 * d.0 + o.2 * d.2);
                let c = o.0 * o.0 + o.2 * o.2 - r * r;
                let disc = b * b - 4.0 * a * c;
                if disc >= 0.0 {
                    let t = (-b - super::sqrt_f(disc)) / (2.0 * a);
                    let y = o.1 + d.1 * t;
                    if t > 0.0 && y.abs() <= h {
                        best = Some((t, (o.0 + d.0 * t, 0.0, o.2 + d.2 * t)));
                    }
                }
            }
            for cap in [-h, h] {
                if d.1.abs() > 1e-8 {
                    let t = (cap - o.1) / d.1;
                    let (x, z) = (o.0 + d.0 * t, o.2 + d.2 * t);
                    if t > 0.0 && x * x + z * z <= r * r && best.is_none_or(|(bt, _)| t < bt) {
                        best = Some((t, (0.0, cap.signum(), 0.0)));
                    }
                }
            }
            best.map(|b| b.1)
        }
        Solid::Cone => {
            // x² + z² = k²(y + 0.6)², apex y = -0.6, base y = 0.55.
            let k = 0.5f32 / 1.15;
            let k2 = k * k;
            let oy = o.1 + 0.6;
            let a = d.0 * d.0 + d.2 * d.2 - k2 * d.1 * d.1;
            let b = 2.0 * (o.0 * d.0 + o.2 * d.2 - k2 * oy * d.1);
            let c = o.0 * o.0 + o.2 * o.2 - k2 * oy * oy;
            let mut best: Option<(f32, (f32, f32, f32))> = None;
            let disc = b * b - 4.0 * a * c;
            if a.abs() > 1e-8 && disc >= 0.0 {
                let sq = super::sqrt_f(disc);
                for t in [(-b - sq) / (2.0 * a), (-b + sq) / (2.0 * a)] {
                    let p = (o.0 + d.0 * t, o.1 + d.1 * t, o.2 + d.2 * t);
                    if t > 0.0 && (-0.6..=0.55).contains(&p.1) && best.is_none_or(|(bt, _)| t < bt)
                    {
                        best = Some((t, (p.0, -k2 * (p.1 + 0.6), p.2)));
                    }
                }
            }
            if d.1.abs() > 1e-8 {
                let t = (0.55 - o.1) / d.1;
                let (x, z) = (o.0 + d.0 * t, o.2 + d.2 * t);
                if t > 0.0 && x * x + z * z <= 0.25 && best.is_none_or(|(bt, _)| t < bt) {
                    best = Some((t, (0.0, 1.0, 0.0)));
                }
            }
            best.map(|b| b.1)
        }
        Solid::Sphere | Solid::Capsule => None,
    }
}
/// Ray against the intersection of half-spaces n·p <= c: the entering
/// plane's normal.
fn convex(
    o: (f32, f32, f32),
    d: (f32, f32, f32),
    planes: &[((f32, f32, f32), f32)],
) -> Option<(f32, f32, f32)> {
    let (mut near, mut far) = (f32::MIN, f32::MAX);
    let mut normal = (0.0, 0.0, 1.0);
    for &(n, c) in planes {
        let denom = n.0 * d.0 + n.1 * d.1 + n.2 * d.2;
        let dist = c - (n.0 * o.0 + n.1 * o.1 + n.2 * o.2);
        if denom.abs() < 1e-8 {
            if dist < 0.0 {
                return None;
            }
            continue;
        }
        let t = dist / denom;
        if denom < 0.0 {
            if t > near {
                near = t;
                normal = n;
            }
        } else if t < far {
            far = t;
        }
    }
    (near <= far && near > 0.0).then_some(normal)
}
/// Composite a premultiplied tile onto the active layer at (x, y).
pub fn stamp_tile(
    doc: &mut Doc,
    x: i32,
    y: i32,
    tw: usize,
    th: usize,
    tile: &[u8],
) -> Option<Bounds> {
    let b = Bounds::new(x, y, x + tw as i32, y + th as i32).clamp(doc.w, doc.h)?;
    let w = doc.w;
    let layer = &mut doc.layers[doc.active].px;
    for yy in b.y0..b.y1 {
        for xx in b.x0..b.x1 {
            let s = &tile[(((yy - y) as usize) * tw + (xx - x) as usize) * 4..][..4];
            over(
                &mut layer[(yy as usize * w + xx as usize) * 4..][..4],
                [s[0], s[1], s[2], s[3]],
            );
        }
    }
    Some(b)
}

// ---- history ----

pub enum Change {
    /// Pixels of one layer inside a rectangle (before, after).
    Patch {
        layer: usize,
        bounds: Bounds,
        before: Vec<u8>,
        after: Vec<u8>,
    },
    /// The whole document (size, layer set or order changed).
    Whole { before: Snapshot, after: Snapshot },
}
pub struct Snapshot {
    pub w: usize,
    pub h: usize,
    pub layers: Vec<(Vec<u8>, bool)>,
    pub active: usize,
}
impl Snapshot {
    pub fn of(doc: &Doc) -> Self {
        Self {
            w: doc.w,
            h: doc.h,
            layers: doc
                .layers
                .iter()
                .map(|l| (l.px.clone(), l.visible))
                .collect(),
            active: doc.active,
        }
    }
    pub fn restore(&self, doc: &mut Doc) {
        doc.w = self.w;
        doc.h = self.h;
        doc.layers = self
            .layers
            .iter()
            .map(|(px, v)| Layer {
                px: px.clone(),
                visible: *v,
            })
            .collect();
        doc.active = self.active.min(doc.layers.len() - 1);
    }
    fn bytes(&self) -> usize {
        self.layers.iter().map(|l| l.0.len()).sum()
    }
}
impl Change {
    fn bytes(&self) -> usize {
        match self {
            Self::Patch { before, after, .. } => before.len() + after.len(),
            Self::Whole { before, after } => before.bytes() + after.bytes(),
        }
    }
}
/// Undo/redo, at most `BUDGET` bytes of history (the oldest go first).
#[derive(Default)]
pub struct History {
    undo: Vec<Change>,
    redo: Vec<Change>,
}
const BUDGET: usize = 40 * 1024 * 1024;
impl History {
    pub fn push(&mut self, c: Change) {
        self.redo.clear();
        self.undo.push(c);
        while self.undo.len() > 1 && self.undo.iter().map(Change::bytes).sum::<usize>() > BUDGET {
            self.undo.remove(0);
        }
    }
    /// Record a finished edit of the active layer from its pre-edit copy.
    pub fn patch(&mut self, doc: &Doc, base: &[u8], bounds: Bounds) {
        let Some(b) = bounds.clamp(doc.w, doc.h) else {
            return;
        };
        let before = crop(base, doc.w, b);
        let after = crop(doc.layer(), doc.w, b);
        if before != after {
            self.push(Change::Patch {
                layer: doc.active,
                bounds: b,
                before,
                after,
            });
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo(&mut self, doc: &mut Doc) -> bool {
        let Some(c) = self.undo.pop() else {
            return false;
        };
        apply_change(doc, &c, false);
        self.redo.push(c);
        true
    }
    pub fn redo(&mut self, doc: &mut Doc) -> bool {
        let Some(c) = self.redo.pop() else {
            return false;
        };
        apply_change(doc, &c, true);
        self.undo.push(c);
        true
    }
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}
fn apply_change(doc: &mut Doc, c: &Change, forward: bool) {
    match c {
        Change::Patch {
            layer,
            bounds,
            before,
            after,
        } => {
            if let Some(l) = doc.layers.get_mut(*layer) {
                paste(
                    &mut l.px,
                    doc.w,
                    *bounds,
                    if forward { after } else { before },
                );
            }
        }
        Change::Whole { before, after } => (if forward { after } else { before }).restore(doc),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strokes_do_not_build_up_and_undo_restores_pixels() {
        let mut d = Doc::new(64, 32);
        let mut e = Edit::begin(&d, Tip::Brush(Brush::Brush), 8, Rgba::BLACK, 128, None);
        e.start(&mut d, 10.0, 16.0);
        e.to(&mut d, 30.0, 16.0);
        e.to(&mut d, 10.0, 16.0); // back over itself
        let p = d.pixel(20, 16).unwrap();
        // Half opacity black over white: ~127, not darker where it crossed.
        assert!((120..=135).contains(&p[0]), "{p:?}");
        let mut h = History::default();
        h.patch(&d, &e.base, e.dirty.unwrap());
        assert!(h.undo(&mut d));
        assert_eq!(d.pixel(20, 16).unwrap(), [255, 255, 255, 255]);
        assert!(h.redo(&mut d));
        assert_eq!(d.pixel(20, 16).unwrap(), p);
    }
    #[test]
    fn flood_fill_stops_at_edges_and_respects_tolerance() {
        let mut d = Doc::new(20, 20);
        let mut e = Edit::begin(&d, Tip::Pencil, 1, Rgba::BLACK, 255, None);
        e.start(&mut d, 10.5, 0.5);
        e.to(&mut d, 10.5, 19.5);
        let b = flood(&mut d, 2, 2, Rgba::rgb(255, 0, 0), 0, 255).unwrap();
        assert_eq!((b.x0, b.x1), (0, 10));
        assert_eq!(d.pixel(2, 2).unwrap(), [0, 0, 255, 255]);
        assert_eq!(d.pixel(15, 2).unwrap(), [255, 255, 255, 255]);
    }
    #[test]
    fn shapes_fill_and_outline_inside_their_box() {
        let mut d = Doc::new(100, 100);
        let mut e = Edit::begin(&d, Tip::Brush(Brush::Brush), 2, Rgba::BLACK, 255, None);
        let poly = outline(Shape::Rectangle, 20.0, 20.0, 80.0, 60.0);
        e.shape(
            &mut d,
            &poly,
            true,
            Some((Rgba::BLACK, 2)),
            Some(Rgba::rgb(0, 0, 255)),
        );
        assert_eq!(d.pixel(50, 40).unwrap(), [255, 0, 0, 255], "filled blue");
        assert_eq!(d.pixel(50, 20).unwrap()[0], 0, "outlined black");
        assert_eq!(d.pixel(50, 90).unwrap(), [255, 255, 255, 255]);
        for s in Shape::ALL {
            assert!(outline(s, 0.0, 0.0, 40.0, 30.0).len() >= 2, "{s:?}");
        }
    }
    #[test]
    fn transforms_turn_the_canvas() {
        let mut px = vec![0u8; 3 * 2 * 4];
        px[0] = 9; // (0, 0)
        let (r, w, h) = transform(&px, 3, 2, Turn::Right);
        assert_eq!((w, h), (2, 3));
        assert_eq!(r[4], 9, "top-left goes to top-right");
        let (f, ..) = transform(&px, 3, 2, Turn::FlipHorizontal);
        assert_eq!(f[2 * 4], 9);
    }
    #[test]
    fn solids_render_lit_and_opaque_inside() {
        for s in Solid::ALL {
            let t = render_solid(s, 40, 40, Rgba::rgb(0, 120, 215));
            assert_eq!(t[(20 * 40 + 20) * 4 + 3], 255, "{s:?} centre is covered");
            assert_eq!(t[3], 0, "{s:?} corner is clear");
        }
    }
}
