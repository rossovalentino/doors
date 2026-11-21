//! CPU backend: replays a display list onto a private premultiplied BGRA8
//! frame (the same pixel format as `door_graphics_core::Canvas`, which it
//! uses for elevation shadows), clipped to the damaged region.
//!
//! Geometry snaps from logical edges (`LRect::physical`), so layout metrics
//! and hit areas never depend on the backend. Antialiasing uses 4x4
//! supersampled coverage in exact integer arithmetic (deterministic on every
//! host, as golden images require). Text uses the hinted atlas of the exact
//! effective density (display x text scale) when one is installed; other
//! densities rasterize at the next larger atlas and area-resample
//! (grayscale). Non-ASCII text goes through the Door Text engine attached to
//! the atlas.
use crate::display::{DisplayList, Icon, Images, Op};
use crate::theme::Color;
use crate::units::{LPoint, LRect, Lu, PRect};
use alloc::vec::Vec;
use door_graphics_core::Scale;
use door_graphics_text::{Clip, FontAtlas, Target, TextRun};

/// Hinted atlases by display scale in percent. The 100/200/300% Windows
/// reference atlases load on first use; the platform installs others (the
/// 125/150/175% exports from the runtime asset pack, atlases with a Door
/// Text engine attached for Unicode).
#[derive(Default)]
pub struct Fonts {
    atlases: Vec<(u32, FontAtlas)>,
    failed: Vec<u32>,
}
impl Fonts {
    pub fn new() -> Self {
        Self::default()
    }
    /// Install (or replace) the atlas for its own display percent.
    pub fn install(&mut self, atlas: FontAtlas) {
        let p = atlas.percent();
        self.atlases.retain(|(q, _)| *q != p);
        self.atlases.push((p, atlas));
    }
    pub fn with(mut self, atlas: FontAtlas) -> Self {
        self.install(atlas);
        self
    }
    fn load(&mut self, percent: u32) -> bool {
        if self.atlases.iter().any(|(p, _)| *p == percent) {
            return true;
        }
        if !percent.is_multiple_of(100)
            || !(100..=300).contains(&percent)
            || self.failed.contains(&percent)
        {
            return false;
        }
        // Guest binaries never embed atlases: the platform installs them
        // from the shared asset pack.
        #[cfg(target_os = "none")]
        let loaded: Result<FontAtlas, ()> = Err(());
        #[cfg(not(target_os = "none"))]
        let loaded = FontAtlas::windows_reference_scaled(percent / 100);
        match loaded {
            Ok(a) => {
                self.atlases.push((percent, a));
                true
            }
            Err(_) => {
                self.failed.push(percent);
                false
            }
        }
    }
    /// The atlas for exactly `percent`, if installed or loadable.
    pub fn exact(&mut self, percent: u32) -> Option<&FontAtlas> {
        if self.load(percent) {
            self.atlases
                .iter()
                .find(|(p, _)| *p == percent)
                .map(|(_, a)| a)
        } else {
            None
        }
    }
    /// The smallest available atlas at or above `percent` (else the largest),
    /// as the source for area resampling.
    pub fn at_least(&mut self, percent: u32) -> Option<&FontAtlas> {
        for p in [100, 200, 300] {
            if p >= percent {
                self.load(p);
            }
        }
        self.load(300);
        let best = self
            .atlases
            .iter()
            .filter(|(p, _)| *p >= percent)
            .min_by_key(|(p, _)| *p)
            .or_else(|| self.atlases.iter().max_by_key(|(p, _)| *p))
            .map(|(p, _)| *p)?;
        self.atlases
            .iter()
            .find(|(p, _)| *p == best)
            .map(|(_, a)| a)
    }
    pub fn loaded_bytes(&self) -> usize {
        self.atlases.iter().map(|(_, a)| a.cache_bytes()).sum()
    }
}

/// A private destination frame.
pub struct Frame<'a> {
    pixels: &'a mut [u8],
    width: u32,
    height: u32,
    stride: usize,
}
impl<'a> Frame<'a> {
    pub fn new(pixels: &'a mut [u8], width: u32, height: u32, stride: usize) -> Option<Self> {
        let row = (width as usize).checked_mul(4)?;
        let need = stride.checked_mul(height as usize)?;
        (width > 0
            && height > 0
            && stride >= row
            && stride.is_multiple_of(4)
            && need <= pixels.len())
        .then_some(Self {
            pixels,
            width,
            height,
            stride,
        })
    }
    pub fn bounds(&self) -> PRect {
        PRect {
            x0: 0,
            y0: 0,
            x1: self.width as i32,
            y1: self.height as i32,
        }
    }
    pub fn pixels(&self) -> &[u8] {
        self.pixels
    }
    fn blend(&mut self, x: i32, y: i32, c: Color, coverage: u8) {
        if coverage == 0 {
            return;
        }
        let at = y as usize * self.stride + x as usize * 4;
        let s = if coverage == 255 {
            c
        } else {
            crate::theme::fade(c, coverage)
        };
        if s[3] == 255 {
            self.pixels[at..at + 4].copy_from_slice(&s);
            return;
        }
        let inv = 255 - u32::from(s[3]);
        for (d, s) in self.pixels[at..at + 4].iter_mut().zip(s) {
            *d = (u32::from(s) + (u32::from(*d) * inv + 127) / 255).min(255) as u8;
        }
    }
    fn fill(&mut self, r: PRect, c: Color) {
        if c[3] == 0 {
            return;
        }
        if c[3] == 255 {
            for y in r.y0..r.y1 {
                let at = y as usize * self.stride + r.x0 as usize * 4;
                for p in self.pixels[at..at + r.width() as usize * 4].chunks_exact_mut(4) {
                    p.copy_from_slice(&c);
                }
            }
            return;
        }
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                self.blend(x, y, c, 255);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderStats {
    pub ops: usize,
    pub drawn: usize,
    pub skipped: usize,
    pub damage_pixels: u64,
}

/// Coverage accumulator over a pixel box: shapes take the maximum per pixel
/// and the result is blended once, so joints never double-blend.
struct Mask {
    r: PRect,
    cov: Vec<u8>,
}
/// 4x4 sample offsets inside a pixel, in 1/8 px.
const SAMPLES: [i64; 4] = [1, 3, 5, 7];
impl Mask {
    fn new(r: PRect) -> Self {
        Self {
            r,
            cov: alloc::vec![0; r.area() as usize],
        }
    }
    fn put(&mut self, x: i32, y: i32, c: u8) {
        let i = (y - self.r.y0) as usize * self.r.width() as usize + (x - self.r.x0) as usize;
        if self.cov[i] < c {
            self.cov[i] = c;
        }
    }
    /// Shape given by an inside test on 1/8-px sample coordinates, limited to
    /// `bbox` (1/8 px).
    fn shape(&mut self, bbox: (i64, i64, i64, i64), inside: impl Fn(i64, i64) -> bool) {
        let x0 = (bbox.0.div_euclid(8) as i32).max(self.r.x0);
        let y0 = (bbox.1.div_euclid(8) as i32).max(self.r.y0);
        let x1 = ((bbox.2 + 7).div_euclid(8) as i32).min(self.r.x1);
        let y1 = ((bbox.3 + 7).div_euclid(8) as i32).min(self.r.y1);
        for y in y0..y1 {
            for x in x0..x1 {
                let mut n = 0u32;
                for sy in SAMPLES {
                    for sx in SAMPLES {
                        if inside(i64::from(x) * 8 + sx, i64::from(y) * 8 + sy) {
                            n += 1;
                        }
                    }
                }
                if n > 0 {
                    self.put(x, y, (n * 255 / 16) as u8);
                }
            }
        }
    }
    fn segment(&mut self, a: (i64, i64), b: (i64, i64), half: i64) {
        let bbox = (
            a.0.min(b.0) - half,
            a.1.min(b.1) - half,
            a.0.max(b.0) + half,
            a.1.max(b.1) + half,
        );
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len2 = dx * dx + dy * dy;
        self.shape(bbox, |x, y| {
            let (px, py) = (x - a.0, y - a.1);
            let (cx, cy) = if len2 == 0 {
                (a.0, a.1)
            } else {
                let t = (px * dx + py * dy).clamp(0, len2);
                (a.0 + dx * t / len2, a.1 + dy * t / len2)
            };
            let (ex, ey) = (x - cx, y - cy);
            ex * ex + ey * ey <= half * half
        });
    }
    fn disc(&mut self, c: (i64, i64), r: i64) {
        self.shape((c.0 - r, c.1 - r, c.0 + r, c.1 + r), |x, y| {
            let (dx, dy) = (x - c.0, y - c.1);
            dx * dx + dy * dy <= r * r
        });
    }
    /// Ring between radii, limited to an angular range (degrees clockwise
    /// from 12 o'clock).
    fn ring(&mut self, c: (i64, i64), inner: i64, outer: i64, start: i32, sweep: i32) {
        let full = sweep.abs() >= 360;
        let (start, sweep) = if sweep < 0 {
            (start + sweep, -sweep)
        } else {
            (start, sweep)
        };
        let start16 = i64::from(start.rem_euclid(360)) * 16;
        let sweep16 = i64::from(sweep) * 16;
        self.shape(
            (c.0 - outer, c.1 - outer, c.0 + outer, c.1 + outer),
            |x, y| {
                let (dx, dy) = (x - c.0, y - c.1);
                let d2 = dx * dx + dy * dy;
                if d2 > outer * outer || d2 < inner * inner {
                    return false;
                }
                full || (angle16(dx, dy) - start16).rem_euclid(360 * 16) <= sweep16
            },
        );
    }
    fn blend(&self, frame: &mut Frame<'_>, clip: PRect, c: Color) {
        let Some(r) = self.r.intersect(clip) else {
            return;
        };
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let i =
                    (y - self.r.y0) as usize * self.r.width() as usize + (x - self.r.x0) as usize;
                frame.blend(x, y, c, self.cov[i]);
            }
        }
    }
}
/// Angle in 1/16 degree, clockwise from 12 o'clock (screen y grows down),
/// from an integer octant arctangent (max error about 0.3 degrees).
fn angle16(dx: i64, dy: i64) -> i64 {
    // atan(t) for t in [0,1] in 1/16 degree: 720t - 220 t(t-1)... use the
    // rational approximation atan(t) ~ t*(45 + 15.6*(1-t)) degrees.
    fn atan_ratio(n: i64, d: i64) -> i64 {
        if d == 0 {
            return 720;
        }
        // t = n/d in [0, 1]; result * 16.
        let t = n * 4096 / d;
        (t * (45 * 4096 + 15 * (4096 - t) + (4096 - t) * 6 / 10) / 4096) * 16 / 4096
    }
    let (ax, ay) = (dx.abs(), dy.abs());
    // Angle from the +x axis (counter-clockwise, y up), first octant logic.
    let base = if ax >= ay {
        atan_ratio(ay, ax)
    } else {
        1440 - atan_ratio(ax, ay)
    };
    let up = -dy; // math orientation
    let math = match (dx >= 0, up >= 0) {
        (true, true) => base,
        (false, true) => 2880 - base,
        (false, false) => 2880 + base,
        (true, false) => 5760 - base,
    };
    // Convert: clockwise from 12 o'clock = 90 degrees - math angle.
    (1440 - math).rem_euclid(5760)
}

fn icon_shapes(icon: Icon) -> &'static [Shape] {
    use Shape::*;
    // 64 units = 16 DIPs; stroke width 4 units (1 DIP).
    match icon {
        Icon::ChevronDown => &[Poly(&[(16, 24), (32, 40), (48, 24)])],
        Icon::ChevronUp => &[Poly(&[(16, 40), (32, 24), (48, 40)])],
        Icon::ChevronRight => &[Poly(&[(24, 16), (40, 32), (24, 48)])],
        Icon::ChevronLeft => &[Poly(&[(40, 16), (24, 32), (40, 48)])],
        Icon::Check => &[Poly(&[(10, 34), (24, 48), (54, 16)])],
        Icon::Dash => &[Poly(&[(14, 32), (50, 32)])],
        Icon::Close => &[Poly(&[(14, 14), (50, 50)]), Poly(&[(50, 14), (14, 50)])],
        Icon::More => &[Dot(12, 32, 4), Dot(32, 32, 4), Dot(52, 32, 4)],
        Icon::Menu => &[
            Poly(&[(8, 16), (56, 16)]),
            Poly(&[(8, 32), (56, 32)]),
            Poly(&[(8, 48), (56, 48)]),
        ],
        Icon::Add => &[Poly(&[(32, 8), (32, 56)]), Poly(&[(8, 32), (56, 32)])],
        Icon::Remove => &[Poly(&[(8, 32), (56, 32)])],
        Icon::Search => &[Ring(26, 26, 16), Poly(&[(38, 38), (56, 56)])],
        Icon::Back => &[
            Poly(&[(56, 32), (8, 32)]),
            Poly(&[(28, 12), (8, 32), (28, 52)]),
        ],
        Icon::Forward => &[
            Poly(&[(8, 32), (56, 32)]),
            Poly(&[(36, 12), (56, 32), (36, 52)]),
        ],
        Icon::Info => &[Dot(32, 18, 4), Poly(&[(32, 28), (32, 48)])],
        Icon::Warning => &[Poly(&[(32, 14), (32, 38)]), Dot(32, 49, 4)],
        Icon::Error => &[Poly(&[(20, 20), (44, 44)]), Poly(&[(44, 20), (20, 44)])],
        Icon::Success => &[Poly(&[(18, 33), (28, 43), (47, 22)])],
        Icon::Home => &[
            Poly(&[(6, 32), (32, 8), (58, 32)]),
            Poly(&[(14, 26), (14, 56), (50, 56), (50, 26)]),
        ],
        Icon::Document => &[
            Poly(&[(14, 6), (38, 6), (50, 18), (50, 58), (14, 58), (14, 6)]),
            Poly(&[(38, 6), (38, 18), (50, 18)]),
        ],
        Icon::Folder => &[Poly(&[
            (6, 14),
            (24, 14),
            (30, 20),
            (58, 20),
            (58, 52),
            (6, 52),
            (6, 14),
        ])],
        Icon::Star => &[Poly(&[
            (32, 6),
            (39, 23),
            (58, 24),
            (43, 36),
            (48, 55),
            (32, 44),
            (16, 55),
            (21, 36),
            (6, 24),
            (25, 23),
            (32, 6),
        ])],
        Icon::Settings => &[Ring(32, 32, 20), Ring(32, 32, 8)],
        Icon::Copy => &[
            Poly(&[(20, 14), (54, 14), (54, 54), (20, 54), (20, 14)]),
            Poly(&[(12, 46), (12, 6), (44, 6)]),
        ],
        Icon::Cut => &[
            Ring(18, 48, 8),
            Ring(46, 48, 8),
            Poly(&[(22, 41), (46, 6)]),
            Poly(&[(42, 41), (18, 6)]),
        ],
        Icon::Paste => &[
            Poly(&[(14, 10), (50, 10), (50, 58), (14, 58), (14, 10)]),
            Poly(&[(24, 6), (40, 6), (40, 16), (24, 16), (24, 6)]),
        ],
        Icon::Save => &[
            Poly(&[(8, 8), (46, 8), (56, 18), (56, 56), (8, 56), (8, 8)]),
            Poly(&[(20, 8), (20, 22), (42, 22), (42, 8)]),
            Poly(&[(18, 56), (18, 38), (46, 38), (46, 56)]),
        ],
    }
}
enum Shape {
    Poly(&'static [(i16, i16)]),
    Ring(i16, i16, i16),
    Dot(i16, i16, i16),
}

struct State {
    translate: Vec<LPoint>,
    clip: Vec<Option<PRect>>,
    opacity: Vec<u8>,
}
fn opacity(c: Color, o: u8) -> Color {
    if o == 255 {
        c
    } else {
        crate::theme::fade(c, o)
    }
}

/// Replay `list` onto `frame`. Only ops whose recorded bounds intersect
/// `damage` (physical; None = whole frame) are drawn.
pub fn render(
    list: &DisplayList,
    frame: &mut Frame<'_>,
    scale: Scale,
    text_scale: u32,
    fonts: &mut Fonts,
    images: &Images,
    damage: Option<PRect>,
) -> RenderStats {
    let mut stats = RenderStats {
        ops: list.len(),
        ..Default::default()
    };
    let Some(base) = damage.map_or(Some(frame.bounds()), |d| d.intersect(frame.bounds())) else {
        return stats;
    };
    stats.damage_pixels = base.area();
    let damage_l = LRect::new(
        Lu::from_physical(base.x0, scale) - Lu::ONE,
        Lu::from_physical(base.y0, scale) - Lu::ONE,
        Lu::from_physical(base.width(), scale) + Lu::ONE * 3,
        Lu::from_physical(base.height(), scale) + Lu::ONE * 3,
    );
    let mut st = State {
        translate: alloc::vec![LPoint::default()],
        clip: alloc::vec![Some(base)],
        opacity: alloc::vec![255],
    };
    for (i, op) in list.ops().iter().enumerate() {
        let offset = *st.translate.last().unwrap();
        let clip = *st.clip.last().unwrap();
        let alpha = *st.opacity.last().unwrap();
        match op {
            Op::PushTranslate(d) => st.translate.push(offset.offset(*d)),
            Op::PopTranslate => {
                if st.translate.len() > 1 {
                    st.translate.pop();
                }
            }
            Op::PushClip(r) => {
                let next = clip.and_then(|c| {
                    r.translate(offset)
                        .physical(scale)
                        .and_then(|p| p.intersect(c))
                });
                st.clip.push(next);
            }
            Op::PopClip => {
                if st.clip.len() > 1 {
                    st.clip.pop();
                }
            }
            Op::PushOpacity(o) => st
                .opacity
                .push(((u32::from(alpha) * u32::from(*o) + 127) / 255) as u8),
            Op::PopOpacity => {
                if st.opacity.len() > 1 {
                    st.opacity.pop();
                }
            }
            _ => {
                let Some(clip) = clip else {
                    stats.skipped += 1;
                    continue;
                };
                if list.op_bounds(i).is_some_and(|b| !b.intersects(damage_l)) {
                    stats.skipped += 1;
                    continue;
                }
                stats.drawn += 1;
                draw(
                    op, list, frame, clip, offset, alpha, scale, text_scale, fonts, images,
                );
            }
        }
    }
    stats
}

/// 1/8-px coordinate of a logical coordinate.
fn eighths(v: Lu, scale: Scale) -> i64 {
    (i64::from(v.0) * i64::from(scale.percent()) * 8 + 3200).div_euclid(6400)
}
fn rounded_inside(r: PRect, radius8: i64, x: i64, y: i64) -> bool {
    let (l, t, rr, b) = (
        i64::from(r.x0) * 8,
        i64::from(r.y0) * 8,
        i64::from(r.x1) * 8,
        i64::from(r.y1) * 8,
    );
    if x < l || x >= rr || y < t || y >= b {
        return false;
    }
    let rad = radius8.min((rr - l) / 2).min((b - t) / 2).max(0);
    let cx = if x < l + rad {
        l + rad
    } else if x >= rr - rad {
        rr - rad
    } else {
        return true;
    };
    let cy = if y < t + rad {
        t + rad
    } else if y >= b - rad {
        b - rad
    } else {
        return true;
    };
    let (dx, dy) = (x - cx, y - cy);
    dx * dx + dy * dy <= rad * rad
}
fn fill_rounded(frame: &mut Frame<'_>, clip: PRect, r: PRect, radius8: i64, c: Color) {
    let Some(vis) = r.intersect(clip) else { return };
    let rad_px = ((radius8 + 7) / 8) as i32;
    for y in vis.y0..vis.y1 {
        let corner_row = y < r.y0 + rad_px || y >= r.y1 - rad_px;
        let (core0, core1) = if corner_row {
            (r.x0 + rad_px, r.x1 - rad_px)
        } else {
            (r.x0, r.x1)
        };
        let (c0, c1) = (core0.max(vis.x0), core1.min(vis.x1));
        if c1 > c0 {
            frame.fill(
                PRect {
                    x0: c0,
                    y0: y,
                    x1: c1,
                    y1: y + 1,
                },
                c,
            );
        }
        for x in (vis.x0..c0.min(vis.x1)).chain(c1.max(vis.x0)..vis.x1) {
            let mut n = 0u32;
            for sy in SAMPLES {
                for sx in SAMPLES {
                    if rounded_inside(r, radius8, i64::from(x) * 8 + sx, i64::from(y) * 8 + sy) {
                        n += 1;
                    }
                }
            }
            frame.blend(x, y, c, (n * 255 / 16) as u8);
        }
    }
}
fn border(frame: &mut Frame<'_>, clip: PRect, r: PRect, radius8: i64, width8: i64, c: Color) {
    let Some(vis) = r.intersect(clip) else { return };
    let w_px = ((width8 + 7) / 8) as i32;
    let rad_px = ((radius8 + 7) / 8) as i32;
    let inner_rad = (radius8 - width8).max(0);
    for y in vis.y0..vis.y1 {
        for x in vis.x0..vis.x1 {
            // Skip the guaranteed interior quickly.
            let edge = x < r.x0 + w_px.max(rad_px)
                || x >= r.x1 - w_px.max(rad_px)
                || y < r.y0 + w_px.max(rad_px)
                || y >= r.y1 - w_px.max(rad_px);
            if !edge {
                continue;
            }
            let mut n = 0i32;
            for sy in SAMPLES {
                for sx in SAMPLES {
                    let (px, py) = (i64::from(x) * 8 + sx, i64::from(y) * 8 + sy);
                    if rounded_inside(r, radius8, px, py) {
                        let inner = (
                            i64::from(r.x0) * 8 + width8,
                            i64::from(r.y0) * 8 + width8,
                            i64::from(r.x1) * 8 - width8,
                            i64::from(r.y1) * 8 - width8,
                        );
                        let inside_inner =
                            px >= inner.0 && px < inner.2 && py >= inner.1 && py < inner.3 && {
                                let rad = inner_rad
                                    .min((inner.2 - inner.0) / 2)
                                    .min((inner.3 - inner.1) / 2)
                                    .max(0);
                                let cx = if px < inner.0 + rad {
                                    Some(inner.0 + rad)
                                } else if px >= inner.2 - rad {
                                    Some(inner.2 - rad)
                                } else {
                                    None
                                };
                                let cy = if py < inner.1 + rad {
                                    Some(inner.1 + rad)
                                } else if py >= inner.3 - rad {
                                    Some(inner.3 - rad)
                                } else {
                                    None
                                };
                                match (cx, cy) {
                                    (Some(cx), Some(cy)) => {
                                        (px - cx).pow(2) + (py - cy).pow(2) <= rad * rad
                                    }
                                    _ => true,
                                }
                            };
                        if !inside_inner {
                            n += 1;
                        }
                    }
                }
            }
            frame.blend(x, y, c, (n as u32 * 255 / 16) as u8);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw(
    op: &Op,
    list: &DisplayList,
    frame: &mut Frame<'_>,
    clip: PRect,
    offset: LPoint,
    alpha: u8,
    scale: Scale,
    text_scale: u32,
    fonts: &mut Fonts,
    images: &Images,
) {
    let pr = |r: LRect| r.translate(offset).physical(scale);
    match op {
        Op::Rect { rect, color } => {
            if let Some(r) = pr(*rect).and_then(|r| r.intersect(clip)) {
                frame.fill(r, opacity(*color, alpha));
            }
        }
        Op::RoundRect {
            rect,
            radius,
            color,
        } => {
            if let Some(r) = pr(*rect) {
                fill_rounded(
                    frame,
                    clip,
                    r,
                    eighths(*radius, scale),
                    opacity(*color, alpha),
                );
            }
        }
        Op::Border {
            rect,
            radius,
            width,
            color,
        } => {
            if let Some(r) = pr(*rect) {
                // Stroke width snaps to whole pixels (at least one), as
                // WinUI borders stay crisp at every scale.
                let w8 = (eighths(*width, scale) + 4) / 8 * 8;
                border(
                    frame,
                    clip,
                    r,
                    eighths(*radius, scale),
                    w8.max(8),
                    opacity(*color, alpha),
                );
            }
        }
        Op::Ellipse { rect, color } => {
            let r = rect.translate(offset);
            let (x0, y0, x1, y1) = (
                eighths(r.x, scale),
                eighths(r.y, scale),
                eighths(r.right(), scale),
                eighths(r.bottom(), scale),
            );
            let Some(bbox) = pr(*rect) else { return };
            let (cx, cy) = ((x0 + x1) / 2, (y0 + y1) / 2);
            let (rx, ry) = (((x1 - x0) / 2).max(1), ((y1 - y0) / 2).max(1));
            let mut m = Mask::new(bbox);
            m.shape((x0, y0, x1, y1), |x, y| {
                let (dx, dy) = (x - cx, y - cy);
                dx * dx * ry * ry + dy * dy * rx * rx <= rx * rx * ry * ry
            });
            m.blend(frame, clip, opacity(*color, alpha));
        }
        Op::Arc {
            rect,
            width,
            start,
            sweep,
            color,
        } => {
            let r = rect.translate(offset);
            let Some(bbox) = pr(*rect) else { return };
            let (x0, y0, x1, y1) = (
                eighths(r.x, scale),
                eighths(r.y, scale),
                eighths(r.right(), scale),
                eighths(r.bottom(), scale),
            );
            let outer = ((x1 - x0).min(y1 - y0)) / 2;
            let w = eighths(*width, scale).max(8);
            let mut m = Mask::new(bbox);
            m.ring(
                ((x0 + x1) / 2, (y0 + y1) / 2),
                (outer - w).max(0),
                outer,
                *start,
                *sweep,
            );
            m.blend(frame, clip, opacity(*color, alpha));
        }
        Op::Line {
            from,
            to,
            width,
            color,
        } => {
            let a = (
                eighths(from.x + offset.x, scale),
                eighths(from.y + offset.y, scale),
            );
            let b = (
                eighths(to.x + offset.x, scale),
                eighths(to.y + offset.y, scale),
            );
            let half = (eighths(*width, scale) / 2).max(4);
            let bbox = PRect {
                x0: ((a.0.min(b.0) - half) / 8) as i32 - 1,
                y0: ((a.1.min(b.1) - half) / 8) as i32 - 1,
                x1: ((a.0.max(b.0) + half) / 8) as i32 + 2,
                y1: ((a.1.max(b.1) + half) / 8) as i32 + 2,
            };
            let Some(bbox) = bbox.intersect(clip) else {
                return;
            };
            let mut m = Mask::new(bbox);
            m.segment(a, b, half);
            m.blend(frame, clip, opacity(*color, alpha));
        }
        Op::Icon { icon, rect, color } => {
            let r = rect.translate(offset);
            let Some(bbox) = r.inflate(Lu::ONE).physical(scale) else {
                return;
            };
            let Some(bbox) = bbox.intersect(clip) else {
                return;
            };
            let (x0, y0) = (eighths(r.x, scale), eighths(r.y, scale));
            let size = eighths(r.w.min(r.h), scale);
            let p = |gx: i16, gy: i16| {
                (
                    x0 + i64::from(gx) * size / 64,
                    y0 + i64::from(gy) * size / 64,
                )
            };
            let half = (size / 32).max(4);
            let mut m = Mask::new(bbox);
            for shape in icon_shapes(*icon) {
                match shape {
                    Shape::Poly(points) => {
                        for w in points.windows(2) {
                            m.segment(p(w[0].0, w[0].1), p(w[1].0, w[1].1), half);
                        }
                    }
                    Shape::Ring(cx, cy, rad) => {
                        let c = p(*cx, *cy);
                        let rad = i64::from(*rad) * size / 64;
                        m.ring(c, rad - half, rad + half, 0, 360);
                    }
                    Shape::Dot(cx, cy, rad) => {
                        let c = p(*cx, *cy);
                        m.disc(c, i64::from(*rad) * size / 64);
                    }
                }
            }
            m.blend(frame, clip, opacity(*color, alpha));
        }
        Op::Text {
            text,
            origin,
            role,
            color,
        } => {
            let o = origin.offset(offset);
            draw_text(
                frame,
                clip,
                list.text(text),
                o,
                *role,
                opacity(*color, alpha),
                scale,
                text_scale,
                fonts,
            );
        }
        Op::Image { image, rect } => {
            let Some((w, h, px)) = images.get(*image) else {
                return;
            };
            let Some(r) = pr(*rect) else { return };
            let Some(vis) = r.intersect(clip) else { return };
            for y in vis.y0..vis.y1 {
                let sy = ((y - r.y0) as u64 * u64::from(h) / r.height() as u64) as usize;
                for x in vis.x0..vis.x1 {
                    let sx = ((x - r.x0) as u64 * u64::from(w) / r.width() as u64) as usize;
                    let at = (sy * w as usize + sx) * 4;
                    let c = [px[at], px[at + 1], px[at + 2], px[at + 3]];
                    frame.blend(x, y, opacity(c, alpha), 255);
                }
            }
        }
        Op::Shadow {
            rect,
            radius,
            depth,
        } => {
            let Some(r) = pr(*rect) else { return };
            shadow(
                frame,
                clip,
                r,
                eighths(*radius, scale),
                *depth,
                alpha,
                scale,
            );
        }
        _ => (),
    }
}

/// Elevation shadow: the compositor's cached Gaussian (graphics-core) for
/// window-sized surfaces, layered translucent outlines for small ones.
fn shadow(
    frame: &mut Frame<'_>,
    clip: PRect,
    r: PRect,
    radius8: i64,
    depth: u8,
    alpha: u8,
    scale: Scale,
) {
    let opacity = ((u32::from(depth.min(8)) * 9 * u32::from(alpha)) / 255).min(72) as u8;
    let (w, h, stride) = (frame.width, frame.height, frame.stride);
    if let Ok(rect) = door_graphics_core::Rect::new(r.x0, r.y0, r.width() as u32, r.height() as u32)
        && let Ok(mut canvas) = door_graphics_core::Canvas::new(frame.pixels, w, h, stride)
    {
        if let Ok(c) = door_graphics_core::Rect::new(
            clip.x0,
            clip.y0,
            clip.width() as u32,
            clip.height() as u32,
        ) {
            canvas.set_clip(c);
        }
        if canvas.soft_shadow(rect, scale, opacity) {
            return;
        }
    }
    let spread = i32::from(depth.min(8)).max(1) * scale.percent() as i32 / 100;
    for i in (1..=spread).rev() {
        let a = (u32::from(opacity) * (spread - i + 1) as u32 / (spread as u32 * 3)).max(1) as u8;
        let ring = PRect {
            x0: r.x0 - i,
            y0: r.y0 - i + spread / 3,
            x1: r.x1 + i,
            y1: r.y1 + i + spread / 3,
        };
        fill_rounded(frame, clip, ring, radius8 + i64::from(i) * 8, [0, 0, 0, a]);
    }
}

/// Rasterize a glyph run with the atlas of the exact effective density
/// (display percent x text percent) when one exists, otherwise from the next
/// larger atlas, area-resampled in grayscale.
#[allow(clippy::too_many_arguments)]
fn draw_text(
    frame: &mut Frame<'_>,
    clip: PRect,
    text: &str,
    origin: LPoint,
    role: crate::theme::TextRole,
    color: Color,
    scale: Scale,
    text_scale: u32,
    fonts: &mut Fonts,
) {
    if text.is_empty() || color[3] == 0 {
        return;
    }
    let style = role.style();
    // Effective density in 1/100 percent.
    let eff_num = u64::from(scale.percent()) * u64::from(text_scale);
    let x = origin.x.physical(scale);
    let y = origin.y.physical(scale);
    let gclip = Clip::new(clip.x0, clip.y0, clip.width() as u32, clip.height() as u32);
    if eff_num % 100 == 0 {
        let unicode = fonts
            .exact((eff_num / 100) as u32)
            .map(|a| a.unicode().is_some());
        if let Some(unicode) = unicode {
            let shown = if unicode {
                alloc::borrow::Cow::Borrowed(text)
            } else {
                crate::text::display(text)
            };
            let shown = clip_text(&shown);
            let atlas = fonts.exact((eff_num / 100) as u32).unwrap();
            let (w, h, s) = (frame.width, frame.height, frame.stride);
            if let Ok(mut target) = Target::new(frame.pixels, w, h, s) {
                if color[3] != 255 {
                    target = target.grayscale();
                }
                let _ = atlas.draw_physical(
                    &mut target,
                    gclip,
                    TextRun {
                        text: shown,
                        x,
                        top: y,
                        style,
                        color,
                    },
                );
            }
            return;
        }
    }
    let Some(atlas) = fonts.at_least(eff_num.div_ceil(100) as u32) else {
        return;
    };
    let source = u64::from(atlas.percent());
    let shown = if atlas.unicode().is_some() {
        alloc::borrow::Cow::Borrowed(text)
    } else {
        crate::text::display(text)
    };
    let shown = clip_text(&shown);
    let Ok(m) = atlas.measure(shown, style) else {
        return;
    };
    let margin = (2 * source / 100) as u32 + 1;
    let sw = (u64::from(m.advance_width) * source / 100) as u32 + margin * 2;
    let sh = (u64::from(style.line_height()) * source / 100) as u32 * m.lines.max(1);
    if sw == 0 || sh == 0 || u64::from(sw) * u64::from(sh) > 16 * 1024 * 1024 {
        return;
    }
    let mut scratch = alloc::vec![0u8; sw as usize * sh as usize * 4];
    {
        let Ok(mut target) = Target::new(&mut scratch, sw, sh, sw as usize * 4) else {
            return;
        };
        target = target.grayscale();
        let _ = atlas.draw_physical(
            &mut target,
            Clip::new(0, 0, sw, sh),
            TextRun {
                text: shown,
                x: margin as i32,
                top: 0,
                style,
                color: [255; 4],
            },
        );
    }
    // Destination box: source extent times eff/source (16.16 ratio).
    let ratio = ((eff_num << 16) / (source * 100)).max(1);
    let dw = ((u64::from(sw) * ratio) >> 16).max(1) as i32;
    let dh = ((u64::from(sh) * ratio) >> 16).max(1) as i32;
    let dx0 = x - ((u64::from(margin) * ratio) >> 16) as i32;
    let dest = PRect {
        x0: dx0,
        y0: y,
        x1: dx0 + dw,
        y1: y + dh,
    };
    let Some(vis) = dest.intersect(clip) else {
        return;
    };
    let src = |sx: i64, sy: i64| -> u32 {
        if sx < 0 || sy < 0 || sx >= i64::from(sw) || sy >= i64::from(sh) {
            0
        } else {
            u32::from(scratch[(sy as usize * sw as usize + sx as usize) * 4 + 3])
        }
    };
    // Box filter over each destination pixel's source footprint (16.16).
    let inv = ((source * 100) << 16) / eff_num;
    for py in vis.y0..vis.y1 {
        let fy0 = ((py - dest.y0) as u64 * inv) as i64;
        let fy1 = fy0 + inv as i64;
        for px in vis.x0..vis.x1 {
            let fx0 = ((px - dest.x0) as u64 * inv) as i64;
            let fx1 = fx0 + inv as i64;
            let mut acc = 0u64;
            let mut area = 0u64;
            let mut sy = fy0 >> 16;
            while sy << 16 < fy1 {
                let ty = ((sy + 1) << 16).min(fy1) - (sy << 16).max(fy0);
                let mut sx = fx0 >> 16;
                while sx << 16 < fx1 {
                    let tx = ((sx + 1) << 16).min(fx1) - (sx << 16).max(fx0);
                    let wgt = (tx as u64 * ty as u64) >> 16;
                    acc += u64::from(src(sx, sy)) * wgt;
                    area += wgt;
                    sx += 1;
                }
                sy += 1;
            }
            if area > 0 {
                frame.blend(px, py, color, (acc / area).min(255) as u8);
            }
        }
    }
}

/// At most `MAX_TEXT_BYTES` of `s`, cut on a character boundary.
fn clip_text(s: &str) -> &str {
    let mut end = s.len().min(door_graphics_text::MAX_TEXT_BYTES);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn angles_are_clockwise_from_twelve() {
        let deg = |dx, dy| angle16(dx, dy) / 16;
        assert_eq!(deg(0, -10), 0);
        assert!((deg(10, 0) - 90).abs() <= 1);
        assert!((deg(0, 10) - 180).abs() <= 1);
        assert!((deg(-10, 0) - 270).abs() <= 1);
        assert!((deg(10, -10) - 45).abs() <= 1);
        assert!((deg(-10, 10) - 225).abs() <= 1);
    }
}
