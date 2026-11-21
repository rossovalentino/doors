//! Windows 11 26H2 top-level window shadow: two Gaussian layers (a tight
//! ambient one and a wide key one offset downward), so the shadow reaches
//! about 80 logical px below a window, 50 at the sides and 32 above it.
//!
//! The layers were fitted to the darkening profiles measured on the owner's
//! PC over a white backdrop (`reports/artifacts/reference-26h2/explorer-shadow-active-
//! white.png`, identical to `terminal-shadow-white.png`, and the inactive
//! `photos-shadow-white.png`; 150 % captures, so logical = physical / 1.5):
//! the residual is 0.29 % (active) and 0.14 % (inactive) RMS over the bottom,
//! top and side profiles (`shadow_tests`). Each layer is a blurred rectangle,
//! which is separable: a pixel's coverage is the product of a horizontal and
//! a vertical profile read from a Gaussian CDF table (`assets/raster/
//! gaussian-cdf.u16`), with no floating point.
//!
//! For a window larger than the layers' reach, every shadow pixel is a pure
//! function of its distance from the nearest corner or edge, so one mask
//! (corner blocks, one constant per edge row, one row for the sides) serves
//! every larger size: masks are cached per (clamped size, scale, state) and
//! composition only reads them.
use crate::*;
use alloc::{rc::Rc, vec::Vec};

/// One Gaussian layer: alpha in 1/65536, lengths in 1/16 logical px. The
/// layer is the window's outer rectangle grown by `spread` on every side and
/// moved down by `offset`, blurred with standard deviation `sigma`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Layer {
    pub alpha: u32,
    pub sigma: i64,
    pub spread: i64,
    pub offset: i64,
}
/// Active window: ambient a=0.417 sigma=12.26 spread=+0.66 offset=-1.94,
/// key a=0.564 sigma=22.10 spread=-8.0 offset=+36.8 (logical px).
pub(crate) const ACTIVE: [Layer; 2] = [
    Layer {
        alpha: 27_335,
        sigma: 196,
        spread: 11,
        offset: -31,
    },
    Layer {
        alpha: 36_943,
        sigma: 354,
        spread: -128,
        offset: 589,
    },
];
/// Inactive window: ambient a=0.215 sigma=8.90 spread=-4.0 offset=+0.76,
/// key a=0.176 sigma=22.69 spread=-11.75 offset=+41.56.
pub(crate) const INACTIVE: [Layer; 2] = [
    Layer {
        alpha: 14_103,
        sigma: 142,
        spread: -64,
        offset: 12,
    },
    Layer {
        alpha: 11_541,
        sigma: 363,
        spread: -188,
        offset: 665,
    },
];
/// Extent (logical px) of a window shadow beyond each outer edge: every
/// pixel outside rounds to zero alpha in both states at every scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShadowMargins {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}
pub const WINDOW_SHADOW: ShadowMargins = ShadowMargins {
    left: 52,
    top: 35,
    right: 52,
    bottom: 89,
};
impl ShadowMargins {
    /// Physical margins at `scale`.
    pub fn scaled(self, scale: Scale) -> Self {
        Self {
            left: scale.len(self.left),
            top: scale.len(self.top),
            right: scale.len(self.right),
            bottom: scale.len(self.bottom),
        }
    }
    /// `rect` grown by these (already physical) margins.
    pub fn around(self, rect: Rect) -> Option<Rect> {
        let x = rect.x().checked_sub(i32::try_from(self.left).ok()?)?;
        let y = rect.y().checked_sub(i32::try_from(self.top).ok()?)?;
        let width = rect
            .width()
            .checked_add(self.left)?
            .checked_add(self.right)?;
        let height = rect
            .height()
            .checked_add(self.top)?
            .checked_add(self.bottom)?;
        i32::try_from(width).ok()?;
        i32::try_from(height).ok()?;
        Rect::new(x, y, width, height).ok()
    }
}
/// Physical bounds of the shadow of a window whose outer rectangle is
/// `outer` at `scale` (the window itself included).
pub fn window_shadow_bounds(outer: Rect, scale: Scale) -> Option<Rect> {
    WINDOW_SHADOW.scaled(scale).around(outer)
}
const CDF: &[u8; 1026] = include_bytes!("../../../assets/raster/gaussian-cdf.u16");
#[inline]
fn table(i: usize) -> u32 {
    u32::from(u16::from_le_bytes([CDF[2 * i], CDF[2 * i + 1]]))
}
/// Standard normal CDF of `num / den` sigmas (den > 0), 0..=65535, linearly
/// interpolated in 1/64-sigma steps and exactly saturated beyond 4 sigma.
#[inline]
fn phi(num: i64, den: i64) -> u32 {
    let pos = (num.saturating_mul(64 * 256)).div_euclid(den) + 256 * 256;
    if pos <= 0 {
        return 0;
    }
    if pos >= 512 * 256 {
        return 65_535;
    }
    let (i, f) = ((pos >> 8) as usize, (pos & 255) as u32);
    (table(i) * (256 - f) + table(i + 1) * f + 128) >> 8
}
/// Coverage (0..=65535) of a blurred interval [`a`, `b`) (physical 1/16 px)
/// at physical pixel `p`.
#[inline]
fn interval(p: i64, a: i64, b: i64, sigma: i64) -> u32 {
    if b <= a {
        return 0;
    }
    let center = 16 * p + 8;
    phi(center - a, sigma).saturating_sub(phi(center - b, sigma))
}
/// Physical 1/16 px of logical 1/16 px `v` (rounded to nearest).
fn physical16(v: i64, percent: u32) -> i64 {
    (v * i64::from(percent) + v.signum() * 50) / 100
}
/// Coverage of a layer at a pixel: the product of its two profiles (exact
/// when either is saturated, so edge rows and columns need one profile).
#[inline]
fn product(x: u32, y: u32) -> u32 {
    ((u64::from(x) * u64::from(y) + 32_767) / 65_535) as u32
}
/// Combined alpha (0..=255) of the layers from their per-layer coverages.
#[inline]
fn combine(layers: &[Layer; 2], c: [u32; 2]) -> u8 {
    let a0 = ((u64::from(c[0]) * u64::from(layers[0].alpha)) >> 16) as u32;
    let a1 = ((u64::from(c[1]) * u64::from(layers[1].alpha)) >> 16) as u32;
    let total = a0 + a1 - ((u64::from(a0) * u64::from(a1)) >> 16) as u32;
    ((u64::from(total) * 255 + 32_767) >> 16) as u8
}
/// Per-pixel profiles of one window size: `x[layer][column]` and
/// `y[layer][row]` coverages over the shadow bounds.
struct Profiles {
    x: [Vec<u32>; 2],
    y: [Vec<u32>; 2],
}
impl Profiles {
    fn new(
        layers: &[Layer; 2],
        margins: ShadowMargins,
        (width, height): (u32, u32),
        percent: u32,
    ) -> Option<Self> {
        let bw = (margins.left + width + margins.right) as usize;
        let bh = (margins.top + height + margins.bottom) as usize;
        let mut x: [Vec<u32>; 2] = [Vec::new(), Vec::new()];
        let mut y: [Vec<u32>; 2] = [Vec::new(), Vec::new()];
        for (i, layer) in layers.iter().enumerate() {
            let sigma = physical16(layer.sigma, percent).max(1);
            let spread = physical16(layer.spread, percent);
            let offset = physical16(layer.offset, percent);
            let left = 16 * i64::from(margins.left) - spread;
            let right = 16 * i64::from(margins.left + width) + spread;
            let top = 16 * i64::from(margins.top) - spread + offset;
            let bottom = 16 * i64::from(margins.top + height) + spread + offset;
            x[i].try_reserve_exact(bw).ok()?;
            y[i].try_reserve_exact(bh).ok()?;
            x[i].extend((0..bw as i64).map(|p| interval(p, left, right, sigma)));
            y[i].extend((0..bh as i64).map(|p| interval(p, top, bottom, sigma)));
        }
        Some(Self { x, y })
    }
}
/// Leading and trailing pixels before and after the run where every
/// profile is saturated (the whole length when there is no such run).
fn edges(profiles: &[Vec<u32>; 2]) -> (usize, usize) {
    let full = |i: usize| profiles.iter().all(|p| p[i] == 65_535);
    let n = profiles[0].len();
    match (0..n).find(|&i| full(i)) {
        Some(first) => {
            let last = (0..n).rev().find(|&i| full(i)).unwrap_or(first);
            (first, n - 1 - last)
        }
        None => (n, 0),
    }
}
/// Shadow alpha of one window size at one scale and state.
pub(crate) struct ShadowMask {
    /// Physical window outer size the mask was built for (sizes past the
    /// layers' reach are clamped to `canonical`).
    pub(crate) size: (u32, u32),
    pub(crate) percent: u32,
    pub(crate) active: bool,
    margins: ShadowMargins,
    /// Edge columns before/after the saturated middle, edge rows above/below.
    left: usize,
    right: usize,
    top: usize,
    bottom: usize,
    /// Edge rows x edge columns.
    corners: Vec<u8>,
    /// Alpha of the saturated middle columns of each edge row.
    rows: Vec<u8>,
    /// Alpha of the edge columns of every middle row.
    side: Vec<u8>,
    /// Saturated alpha (middle row and column; always under the window).
    center: u8,
}
impl ShadowMask {
    fn layers(active: bool) -> &'static [Layer; 2] {
        if active { &ACTIVE } else { &INACTIVE }
    }
    /// The size whose mask equals the mask of every larger window: twice
    /// the deepest reach of a layer inside the window, plus slack.
    fn canonical(percent: u32) -> u32 {
        let reach = ACTIVE
            .iter()
            .chain(INACTIVE.iter())
            .map(|l| 4 * l.sigma.abs() + l.spread.abs() + l.offset.abs())
            .max()
            .unwrap_or(0);
        2 * (physical16(reach, percent) / 16) as u32 + 8
    }
    /// Mask key of a window: sizes past the canonical extent share one.
    pub(crate) fn key(outer: Rect, scale: Scale) -> (u32, u32) {
        let cap = Self::canonical(scale.percent());
        (outer.width().min(cap), outer.height().min(cap))
    }
    pub(crate) fn build(size: (u32, u32), scale: Scale, active: bool) -> Option<Self> {
        let percent = scale.percent();
        let layers = Self::layers(active);
        let margins = WINDOW_SHADOW.scaled(scale);
        let profiles = Profiles::new(layers, margins, size, percent)?;
        let (left, right) = edges(&profiles.x);
        let (top, bottom) = edges(&profiles.y);
        let (bw, bh) = (profiles.x[0].len(), profiles.y[0].len());
        let columns = || (0..left).chain(bw - right..bw);
        let edge_rows = || (0..top).chain(bh - bottom..bh);
        let mut corners = Vec::new();
        corners
            .try_reserve_exact((top + bottom).checked_mul(left + right)?)
            .ok()?;
        for y in edge_rows() {
            for x in columns() {
                corners.push(combine(
                    layers,
                    [
                        product(profiles.x[0][x], profiles.y[0][y]),
                        product(profiles.x[1][x], profiles.y[1][y]),
                    ],
                ));
            }
        }
        let mut rows = Vec::new();
        rows.try_reserve_exact(top + bottom).ok()?;
        rows.extend(edge_rows().map(|y| combine(layers, [profiles.y[0][y], profiles.y[1][y]])));
        let mut side = Vec::new();
        side.try_reserve_exact(left + right).ok()?;
        side.extend(columns().map(|x| combine(layers, [profiles.x[0][x], profiles.x[1][x]])));
        Some(Self {
            size,
            percent,
            active,
            margins,
            left,
            right,
            top,
            bottom,
            corners,
            rows,
            side,
            center: combine(layers, [65_535, 65_535]),
        })
    }
    /// Shadow bounds of a window at `outer` (which must have this mask's key).
    pub(crate) fn bounds(&self, outer: Rect) -> Option<Rect> {
        self.margins.around(outer)
    }
    /// Edge-row index of bounds row `y` of a `height`-tall bounds, or None
    /// for a middle row.
    #[inline]
    fn row(&self, y: usize, height: usize) -> Option<usize> {
        if y < self.top {
            Some(y)
        } else if y >= height - self.bottom {
            Some(self.top + y - (height - self.bottom))
        } else {
            None
        }
    }
    #[inline]
    fn column(&self, x: usize, width: usize) -> Option<usize> {
        if x < self.left {
            Some(x)
        } else if x >= width - self.right {
            Some(self.left + x - (width - self.right))
        } else {
            None
        }
    }
    /// Alpha at bounds pixel (`x`, `y`) of `bounds` (as from `bounds`).
    pub(crate) fn alpha(&self, bounds: Rect, x: i32, y: i32) -> u8 {
        if !bounds.contains(x, y) {
            return 0;
        }
        let (w, h) = (bounds.width() as usize, bounds.height() as usize);
        let (bx, by) = ((x - bounds.x()) as usize, (y - bounds.y()) as usize);
        match (self.row(by, h), self.column(bx, w)) {
            (Some(r), Some(c)) => self.corners[r * (self.left + self.right) + c],
            (Some(r), None) => self.rows[r],
            (None, Some(c)) => self.side[c],
            (None, None) => self.center,
        }
    }
}
/// Bounded cache of shadow masks (a few windows' sizes in both states).
#[derive(Default)]
pub(crate) struct ShadowCache {
    masks: Vec<Rc<ShadowMask>>,
}
const CACHED_MASKS: usize = 8;
impl ShadowCache {
    /// The mask of a window with outer rectangle `outer`, built on a miss.
    pub(crate) fn get(
        &mut self,
        outer: Rect,
        scale: Scale,
        active: bool,
    ) -> Option<Rc<ShadowMask>> {
        let size = ShadowMask::key(outer, scale);
        if let Some(i) = self
            .masks
            .iter()
            .position(|m| m.size == size && m.percent == scale.percent() && m.active == active)
        {
            let mask = self.masks.remove(i);
            self.masks.push(mask.clone());
            return Some(mask);
        }
        let mask = Rc::new(ShadowMask::build(size, scale, active)?);
        if self.masks.len() >= CACHED_MASKS {
            self.masks.remove(0);
        }
        self.masks.push(mask.clone());
        Some(mask)
    }
}
/// Every channel of a packed BGRA word times `a`/255 (see render.rs).
#[inline(always)]
fn scale_word(c: u32, a: u32) -> u32 {
    let rb = (c & 0x00ff_00ff) * a + 0x0080_0080;
    let rb = ((rb + ((rb >> 8) & 0x00ff_00ff)) >> 8) & 0x00ff_00ff;
    let ga = ((c >> 8) & 0x00ff_00ff) * a + 0x0080_0080;
    let ga = (ga + ((ga >> 8) & 0x00ff_00ff)) & 0xff00_ff00;
    rb | ga
}
/// Black at `alpha` over the premultiplied pixel at byte `at`.
#[inline(always)]
fn darken(pixels: &mut [u8], at: usize, alpha: u8) {
    if alpha == 0 {
        return;
    }
    let d = u32::from_le_bytes(pixels[at..at + 4].try_into().unwrap());
    let a = u32::from(alpha);
    let out = (a << 24) + scale_word(d, 255 - a);
    pixels[at..at + 4].copy_from_slice(&out.to_le_bytes());
}
impl Canvas<'_> {
    /// The window shadow of `mask` around the opaque window `opaque` (radius
    /// `radius`), within the clip. Pixels fully covered by the window body,
    /// which is painted next, are skipped; its antialiased corners keep the
    /// shadow beneath them.
    pub(crate) fn window_shadow(&mut self, opaque: Rect, radius: u32, mask: &ShadowMask) {
        let Some(bounds) = mask.bounds(opaque) else {
            return;
        };
        let Some(clip) = self.clip_rect().and_then(|c| bounds.intersection(c)) else {
            return;
        };
        let bh = bounds.height() as usize;
        let columns = mask.left + mask.right;
        let stride = self.physical_dimensions().2;
        let mid_start = bounds.x() + mask.left as i32;
        let mid_end = bounds.right() - mask.right as i32;
        let r = radius as i32;
        for y in clip.y()..clip.bottom() {
            let row = mask.row((y - bounds.y()) as usize, bh);
            let (alphas, constant) = match row {
                Some(i) => (&mask.corners[i * columns..][..columns], mask.rows[i]),
                None => (&mask.side[..], mask.center),
            };
            let inside = (opaque.y()..opaque.bottom()).contains(&y);
            // Rows within the corner radius keep the window's antialiased
            // corners: only fully covered pixels there are skipped.
            let corner_row = inside && (y < opaque.y() + r || y >= opaque.bottom() - r);
            let skip = if inside {
                let inset = if corner_row { r } else { 0 };
                (opaque.x() + inset, opaque.right() - inset)
            } else {
                (clip.right(), clip.right())
            };
            let left = skip.0.clamp(clip.x(), clip.right());
            let right = skip.1.clamp(left, clip.right());
            let line = y as usize * stride;
            let pixels = self.pixels_mut();
            for (start, end) in [(clip.x(), left), (right, clip.right())] {
                // Edge columns before the middle, the saturated middle (one
                // alpha per row) and edge columns after it.
                for (from, to, first) in [
                    (start.max(bounds.x()), end.min(mid_start), 0),
                    (start.max(mid_start), end.min(mid_end), usize::MAX),
                    (start.max(mid_end), end.min(bounds.right()), mask.left),
                ] {
                    if from >= to {
                        continue;
                    }
                    let base = if first == 0 { bounds.x() } else { mid_end };
                    let span = &mut pixels[line + from as usize * 4..line + to as usize * 4];
                    for (i, pixel) in span.chunks_exact_mut(4).enumerate() {
                        let x = from + i as i32;
                        let alpha = if first == usize::MAX {
                            constant
                        } else {
                            if corner_row
                                && opaque.contains(x, y)
                                && crate::render::rounded_coverage(opaque, radius, x, y) == 255
                            {
                                continue;
                            }
                            alphas[first + (x - base) as usize]
                        };
                        darken(pixel, 0, alpha);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "window_shadow_tests.rs"]
mod tests;
