//! COLR v0/v1 colour glyph painting into premultiplied BGRA.
//!
//! An original small software painter for Skrifa's `ColorPainter` callbacks:
//! affine transforms, glyph and box clips (area coverage masks), solid,
//! linear, two-point conical and sweep gradients with pad/repeat/reflect,
//! layers and Porter-Duff/separable blend composites. Non-separable blend
//! modes (hue, saturation, colour, luminosity) fall back to source-over.
use crate::font::Face;
use crate::raster::{GlyphBitmap, MAX_GLYPH_EDGE, PathPen, Pixels, Segment, coverage};
use ab_glyph_rasterizer::point;
use alloc::{vec, vec::Vec};
#[allow(unused_imports)]
use core_maths::CoreFloat;
use skrifa::{
    GlyphId,
    color::{Brush, ColorPainter, ColorStop, CompositeMode, Extend, Transform},
    instance::{LocationRef, NormalizedCoord, Size},
    metrics::BoundingBox,
    outline::DrawSettings,
};

type Affine = [f32; 6];
fn mul(a: Affine, b: Affine) -> Affine {
    // a * b: apply b first, then a.
    [
        a[0] * b[0] + a[2] * b[1],
        a[1] * b[0] + a[3] * b[1],
        a[0] * b[2] + a[2] * b[3],
        a[1] * b[2] + a[3] * b[3],
        a[0] * b[4] + a[2] * b[5] + a[4],
        a[1] * b[4] + a[3] * b[5] + a[5],
    ]
}
fn invert(m: Affine) -> Option<Affine> {
    let det = m[0] * m[3] - m[1] * m[2];
    if det.abs() < 1e-12 || !det.is_finite() {
        return None;
    }
    let inv = 1.0 / det;
    let (a, b, c, d) = (m[3] * inv, -m[1] * inv, -m[2] * inv, m[0] * inv);
    Some([a, b, c, d, -(a * m[4] + c * m[5]), -(b * m[4] + d * m[5])])
}
fn from_transform(t: Transform) -> Affine {
    [t.xx, t.yx, t.xy, t.yy, t.dx, t.dy]
}

/// Collects device-space bounds of every clip glyph/box (first pass).
struct Bounds<'a> {
    face: &'a Face,
    coords: &'a [NormalizedCoord],
    transforms: Vec<Affine>,
    min: (f32, f32),
    max: (f32, f32),
    invalid: bool,
}
impl Bounds<'_> {
    fn add(&mut self, x: f32, y: f32) {
        if !x.is_finite() || !y.is_finite() {
            self.invalid = true;
            return;
        }
        self.min = (self.min.0.min(x), self.min.1.min(y));
        self.max = (self.max.0.max(x), self.max.1.max(y));
    }
}
impl ColorPainter for Bounds<'_> {
    fn push_transform(&mut self, transform: Transform) {
        let top = *self
            .transforms
            .last()
            .unwrap_or(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        self.transforms.push(mul(top, from_transform(transform)));
    }
    fn pop_transform(&mut self) {
        if self.transforms.len() > 1 {
            self.transforms.pop();
        }
    }
    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        let mut pen = PathPen::new(1.0e6);
        pen.transform = self.transforms.last().copied();
        if let Some(outline) = self.face.outlines.get(glyph_id) {
            let _ = outline.draw(
                DrawSettings::unhinted(Size::unscaled(), LocationRef::new(self.coords)),
                &mut pen,
            );
        }
        if let Some((a, b)) = pen.bounds() {
            self.add(a.x, a.y);
            self.add(b.x, b.y);
        }
    }
    fn push_clip_box(&mut self, clip_box: BoundingBox) {
        let m = *self
            .transforms
            .last()
            .unwrap_or(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        for (x, y) in [
            (clip_box.x_min, clip_box.y_min),
            (clip_box.x_max, clip_box.y_min),
            (clip_box.x_max, clip_box.y_max),
            (clip_box.x_min, clip_box.y_max),
        ] {
            self.add(m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
        }
    }
    fn pop_clip(&mut self) {}
    fn fill(&mut self, _brush: Brush<'_>) {}
    fn push_layer(&mut self, _composite_mode: CompositeMode) {}
}

struct Painter<'a> {
    face: &'a Face,
    coords: &'a [NormalizedCoord],
    foreground: [f32; 4],
    width: usize,
    height: usize,
    transforms: Vec<Affine>,
    clips: Vec<Vec<f32>>,
    layers: Vec<(Vec<[f32; 4]>, CompositeMode)>,
    budget: usize,
}

impl Painter<'_> {
    fn transform(&self) -> Affine {
        *self.transforms.last().expect("base transform")
    }
    fn mask(&self, pen: &PathPen) -> Vec<f32> {
        let mut out = vec![0.0f32; self.width * self.height];
        if pen.invalid {
            return out;
        }
        // Device space is y down; flip into the rasterizer's y-up convention.
        let flipped: Vec<Segment> = pen
            .segments
            .iter()
            .map(|s| {
                let f = |p: ab_glyph_rasterizer::Point| point(p.x, -p.y);
                match *s {
                    Segment::Line(a, b) => Segment::Line(f(a), f(b)),
                    Segment::Quad(a, b, c) => Segment::Quad(f(a), f(b), f(c)),
                    Segment::Cubic(a, b, c, d) => Segment::Cubic(f(a), f(b), f(c), f(d)),
                }
            })
            .collect();
        let min = point(0.0, -(self.height as f32));
        let max = point(self.width as f32, 0.0);
        if let Some((_, _, w, h, raw)) = coverage(&flipped, min, max, 1.0, 0.0) {
            for y in 0..h.min(self.height) {
                for x in 0..w.min(self.width) {
                    out[y * self.width + x] = f32::from(raw[y * w + x]) / 255.0;
                }
            }
        }
        out
    }
    fn push_mask(&mut self, mut mask: Vec<f32>) {
        if let Some(parent) = self.clips.last() {
            for (m, p) in mask.iter_mut().zip(parent) {
                *m *= *p;
            }
        }
        self.clips.push(mask);
    }
    fn color(&self, index: u16, alpha: f32) -> [f32; 4] {
        let base = if index == 0xFFFF {
            self.foreground
        } else {
            self.face
                .palette
                .get(usize::from(index))
                .map_or([0.0; 4], |c| {
                    let a = f32::from(c[3]) / 255.0;
                    [
                        f32::from(c[0]) / 255.0 * a,
                        f32::from(c[1]) / 255.0 * a,
                        f32::from(c[2]) / 255.0 * a,
                        a,
                    ]
                })
        };
        base.map(|c| c * alpha.clamp(0.0, 1.0))
    }
    fn stop_color(&self, stops: &[ColorStop], t: f32) -> [f32; 4] {
        let Some(first) = stops.first() else {
            return [0.0; 4];
        };
        if t <= first.offset {
            return self.color(first.palette_index, first.alpha);
        }
        for pair in stops.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if t <= b.offset {
                let span = b.offset - a.offset;
                let f = if span <= 0.0 {
                    1.0
                } else {
                    (t - a.offset) / span
                };
                let (ca, cb) = (
                    self.color(a.palette_index, a.alpha),
                    self.color(b.palette_index, b.alpha),
                );
                return [0, 1, 2, 3].map(|i| ca[i] + (cb[i] - ca[i]) * f);
            }
        }
        let last = stops[stops.len() - 1];
        self.color(last.palette_index, last.alpha)
    }
}

fn extend(t: f32, mode: Extend) -> f32 {
    match mode {
        Extend::Repeat => t - t.floor(),
        Extend::Reflect => {
            let r = t.rem_euclid(2.0);
            if r > 1.0 { 2.0 - r } else { r }
        }
        _ => t.clamp(0.0, 1.0),
    }
}

fn composite(mode: CompositeMode, s: [f32; 4], d: [f32; 4]) -> [f32; 4] {
    use CompositeMode as M;
    let (sa, da) = (s[3], d[3]);
    let pd = |fs: f32, fd: f32| [0, 1, 2, 3].map(|i| s[i] * fs + d[i] * fd);
    let separable = |f: fn(f32, f32) -> f32| {
        let mut out = [0.0; 4];
        for i in 0..3 {
            let (sc, dc) = (
                if sa > 0.0 { s[i] / sa } else { 0.0 },
                if da > 0.0 { d[i] / da } else { 0.0 },
            );
            out[i] = s[i] * (1.0 - da) + d[i] * (1.0 - sa) + sa * da * f(sc, dc);
        }
        out[3] = sa + da - sa * da;
        out
    };
    match mode {
        M::Clear => [0.0; 4],
        M::Src => s,
        M::Dest => d,
        M::DestOver => pd(1.0 - da, 1.0),
        M::SrcIn => pd(da, 0.0),
        M::DestIn => pd(0.0, sa),
        M::SrcOut => pd(1.0 - da, 0.0),
        M::DestOut => pd(0.0, 1.0 - sa),
        M::SrcAtop => pd(da, 1.0 - sa),
        M::DestAtop => pd(1.0 - da, sa),
        M::Xor => pd(1.0 - da, 1.0 - sa),
        M::Plus => [0, 1, 2, 3].map(|i| (s[i] + d[i]).min(1.0)),
        M::Screen => separable(|s, d| s + d - s * d),
        M::Overlay => separable(|s, d| {
            if d <= 0.5 {
                2.0 * s * d
            } else {
                1.0 - 2.0 * (1.0 - s) * (1.0 - d)
            }
        }),
        M::Darken => separable(f32::min),
        M::Lighten => separable(f32::max),
        M::ColorDodge => separable(|s, d| {
            if d <= 0.0 {
                0.0
            } else if s >= 1.0 {
                1.0
            } else {
                (d / (1.0 - s)).min(1.0)
            }
        }),
        M::ColorBurn => separable(|s, d| {
            if d >= 1.0 {
                1.0
            } else if s <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - d) / s).min(1.0)
            }
        }),
        M::HardLight => separable(|s, d| {
            if s <= 0.5 {
                2.0 * s * d
            } else {
                1.0 - 2.0 * (1.0 - s) * (1.0 - d)
            }
        }),
        M::SoftLight => separable(|s, d| {
            if s <= 0.5 {
                d - (1.0 - 2.0 * s) * d * (1.0 - d)
            } else {
                let g = if d <= 0.25 {
                    ((16.0 * d - 12.0) * d + 4.0) * d
                } else {
                    d.sqrt()
                };
                d + (2.0 * s - 1.0) * (g - d)
            }
        }),
        M::Difference => separable(|s, d| (s - d).abs()),
        M::Exclusion => separable(|s, d| s + d - 2.0 * s * d),
        M::Multiply => separable(|s, d| s * d),
        _ => pd(1.0, 1.0 - sa),
    }
}

impl ColorPainter for Painter<'_> {
    fn push_transform(&mut self, transform: Transform) {
        let next = mul(self.transform(), from_transform(transform));
        self.transforms.push(next);
    }
    fn pop_transform(&mut self) {
        if self.transforms.len() > 1 {
            self.transforms.pop();
        }
    }
    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        let mut pen = PathPen::new(1.0e6);
        pen.transform = Some(self.transform());
        if let Some(outline) = self.face.outlines.get(glyph_id) {
            let _ = outline.draw(
                DrawSettings::unhinted(Size::unscaled(), LocationRef::new(self.coords)),
                &mut pen,
            );
        }
        let mask = self.mask(&pen);
        self.push_mask(mask);
    }
    fn push_clip_box(&mut self, clip_box: BoundingBox) {
        use skrifa::outline::OutlinePen;
        let mut pen = PathPen::new(1.0e6);
        pen.transform = Some(self.transform());
        pen.move_to(clip_box.x_min, clip_box.y_min);
        pen.line_to(clip_box.x_max, clip_box.y_min);
        pen.line_to(clip_box.x_max, clip_box.y_max);
        pen.line_to(clip_box.x_min, clip_box.y_max);
        pen.close();
        let mask = self.mask(&pen);
        self.push_mask(mask);
    }
    fn pop_clip(&mut self) {
        self.clips.pop();
    }
    fn fill(&mut self, brush: Brush<'_>) {
        if self.budget == 0 {
            return;
        }
        self.budget -= 1;
        let inverse = invert(self.transform());
        let (width, height) = (self.width, self.height);
        let clip = self.clips.last();
        let mut colors = vec![[0.0f32; 4]; width * height];
        for y in 0..height {
            for x in 0..width {
                let cover = clip.map_or(1.0, |c| c[y * width + x]);
                if cover <= 0.0 {
                    continue;
                }
                let color = match &brush {
                    Brush::Solid {
                        palette_index,
                        alpha,
                    } => self.color(*palette_index, *alpha),
                    gradient => {
                        let Some(m) = inverse else { continue };
                        let (dx, dy) = (x as f32 + 0.5, y as f32 + 0.5);
                        let (px, py) = (m[0] * dx + m[2] * dy + m[4], m[1] * dx + m[3] * dy + m[5]);
                        match gradient {
                            Brush::LinearGradient {
                                p0,
                                p1,
                                color_stops,
                                extend: e,
                            } => {
                                let (vx, vy) = (p1.x - p0.x, p1.y - p0.y);
                                let len = vx * vx + vy * vy;
                                let t = if len <= 0.0 {
                                    0.0
                                } else {
                                    ((px - p0.x) * vx + (py - p0.y) * vy) / len
                                };
                                self.stop_color(color_stops, extend(t, *e))
                            }
                            Brush::RadialGradient {
                                c0,
                                r0,
                                c1,
                                r1,
                                color_stops,
                                extend: e,
                            } => {
                                let (cdx, cdy, dr) = (c1.x - c0.x, c1.y - c0.y, r1 - r0);
                                let (pdx, pdy) = (px - c0.x, py - c0.y);
                                let a = cdx * cdx + cdy * cdy - dr * dr;
                                let b = pdx * cdx + pdy * cdy + r0 * dr;
                                let c = pdx * pdx + pdy * pdy - r0 * r0;
                                let t = if a.abs() < 1e-6 {
                                    if b.abs() < 1e-9 {
                                        None
                                    } else {
                                        Some(c / (2.0 * b))
                                    }
                                } else {
                                    let disc = b * b - a * c;
                                    if disc < 0.0 {
                                        None
                                    } else {
                                        let root = disc.sqrt();
                                        let (t1, t2) = ((b + root) / a, (b - root) / a);
                                        let (hi, lo) = if t1 > t2 { (t1, t2) } else { (t2, t1) };
                                        if r0 + hi * dr >= 0.0 {
                                            Some(hi)
                                        } else if r0 + lo * dr >= 0.0 {
                                            Some(lo)
                                        } else {
                                            None
                                        }
                                    }
                                };
                                match t {
                                    Some(t) => self.stop_color(color_stops, extend(t, *e)),
                                    None => continue,
                                }
                            }
                            Brush::SweepGradient {
                                c0,
                                start_angle,
                                end_angle,
                                color_stops,
                                extend: e,
                            } => {
                                // Skrifa reports clockwise angles; undo that in y-up font space.
                                let ccw =
                                    (py - c0.y).atan2(px - c0.x).to_degrees().rem_euclid(360.0);
                                let angle = (360.0 - ccw).rem_euclid(360.0);
                                let span = end_angle - start_angle;
                                let t = if span.abs() < 1e-6 {
                                    0.0
                                } else {
                                    (angle - start_angle) / span
                                };
                                self.stop_color(color_stops, extend(t, *e))
                            }
                            Brush::Solid { .. } => unreachable!(),
                        }
                    }
                };
                colors[y * width + x] = color.map(|c| c * cover);
            }
        }
        if let Some((layer, _)) = self.layers.last_mut() {
            for (d, s) in layer.iter_mut().zip(colors) {
                if s[3] > 0.0 {
                    *d = composite(CompositeMode::SrcOver, s, *d);
                }
            }
        }
    }
    fn push_layer(&mut self, composite_mode: CompositeMode) {
        if self.layers.len() < 64 {
            self.layers
                .push((vec![[0.0; 4]; self.width * self.height], composite_mode));
        }
    }
    fn pop_layer(&mut self) {
        if self.layers.len() < 2 {
            return;
        }
        let (layer, mode) = self.layers.pop().expect("layer");
        let (parent, _) = self.layers.last_mut().expect("parent layer");
        for (d, s) in parent.iter_mut().zip(layer) {
            *d = composite(mode, s, *d);
        }
    }
}

/// Paint a colour glyph at `ppem`. Returns `None` when the glyph has no COLR
/// representation or its paint graph is invalid or unbounded.
pub(crate) fn rasterize_color(
    face: &Face,
    glyph: u32,
    ppem: f32,
    coords: &[NormalizedCoord],
    foreground: [u8; 4],
) -> Option<GlyphBitmap> {
    let color_glyph = face.colors.get(GlyphId::new(glyph))?;
    let scale = ppem / f32::from(face.info.units_per_em);
    let base: Affine = [scale, 0.0, 0.0, -scale, 0.0, 0.0];
    let mut bounds = Bounds {
        face,
        coords,
        transforms: vec![base],
        min: (f32::INFINITY, f32::INFINITY),
        max: (f32::NEG_INFINITY, f32::NEG_INFINITY),
        invalid: false,
    };
    color_glyph
        .paint(LocationRef::new(coords), &mut bounds)
        .ok()?;
    if bounds.invalid || bounds.min.0 > bounds.max.0 || bounds.min.1 > bounds.max.1 {
        return Some(GlyphBitmap::empty(Pixels::Bgra));
    }
    // Clip to a generous box around the em square to bound memory.
    let limit = (ppem * 2.0).max(4.0);
    let left = bounds.min.0.max(-limit).floor() as i32;
    let top = bounds.min.1.max(-limit * 2.0).floor() as i32;
    let right = bounds.max.0.min(limit * 2.0).ceil() as i32;
    let bottom = bounds.max.1.min(limit).ceil() as i32;
    let (width, height) = (
        (right - left).max(0) as usize,
        (bottom - top).max(0) as usize,
    );
    if width == 0
        || height == 0
        || width > MAX_GLYPH_EDGE as usize
        || height > MAX_GLYPH_EDGE as usize
    {
        return Some(GlyphBitmap::empty(Pixels::Bgra));
    }
    let origin: Affine = [scale, 0.0, 0.0, -scale, -left as f32, -top as f32];
    let fa = f32::from(foreground[3]) / 255.0;
    let mut painter = Painter {
        face,
        coords,
        foreground: [
            f32::from(foreground[0]) / 255.0,
            f32::from(foreground[1]) / 255.0,
            f32::from(foreground[2]) / 255.0,
            fa,
        ],
        width,
        height,
        transforms: vec![origin],
        clips: Vec::new(),
        layers: vec![(vec![[0.0; 4]; width * height], CompositeMode::SrcOver)],
        budget: 4096,
    };
    color_glyph
        .paint(LocationRef::new(coords), &mut painter)
        .ok()?;
    let (layer, _) = painter.layers.swap_remove(0);
    let mut data = vec![0u8; width * height * 4];
    for (out, px) in data.chunks_exact_mut(4).zip(layer) {
        let a = px[3].clamp(0.0, 1.0);
        for i in 0..3 {
            out[i] = (px[i].clamp(0.0, a) * 255.0 + 0.5) as u8;
        }
        out[3] = (a * 255.0 + 0.5) as u8;
    }
    Some(GlyphBitmap {
        left,
        top,
        width: width as u32,
        height: height as u32,
        pixels: Pixels::Bgra,
        data,
    })
}
