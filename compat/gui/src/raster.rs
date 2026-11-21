//! GDI raster semantics on 32-bpp `0x00RRGGBB` pixels: the 16 binary (ROP2)
//! and 256 ternary (ROP3) raster operations, pattern brushes (solid, the six
//! 8x8 hatches, bitmaps), Windows-compatible line rasterization, PatBlt,
//! BitBlt and StretchBlt, all clipped to a device-coordinate region.
//!
//! Every operation is evaluated bitwise per channel exactly as GDI defines
//! it; nothing is approximated by alpha blending. The upper byte of every
//! written pixel is zero, as in a GDI 32-bpp DIB.
//!
//! Application coordinates reach this module unchanged (any `i32`), so every
//! derived quantity is computed in `i64`/`i128` and every iteration is clipped
//! to the canvas before it starts: the work of one call is bounded by the
//! canvas, never by the coordinates.
use crate::region::Region;
use crate::surface::mono_pixel;
use crate::{Error, Point, Rect, Result};
use alloc::vec::Vec;

const RGB: u32 = 0x00ff_ffff;

/// A mutable view of pixel rows with `stride` pixels per row. Rows are
/// addressed top-down; a bottom-up store (a positive-height DIB) sets `flip`.
/// A monochrome canvas (a 1-bpp DDB) realizes every color operand to black or
/// white and stores only those two values.
pub struct Canvas<'a> {
    px: &'a mut [u32],
    width: u32,
    height: u32,
    stride: usize,
    flip: bool,
    mono: bool,
}
impl<'a> Canvas<'a> {
    pub fn new(px: &'a mut [u32], width: u32, height: u32, stride: usize) -> Result<Self> {
        Self::with_order(px, width, height, stride, false)
    }
    pub fn with_order(
        px: &'a mut [u32],
        width: u32,
        height: u32,
        stride: usize,
        flip: bool,
    ) -> Result<Self> {
        let need = if height == 0 {
            0
        } else {
            stride
                .checked_mul(height as usize - 1)
                .and_then(|v| v.checked_add(width as usize))
                .ok_or(Error::InvalidParameter)?
        };
        if (stride as u64) < u64::from(width) || px.len() < need || width > i32::MAX as u32 {
            return Err(Error::InvalidParameter);
        }
        if height > i32::MAX as u32 {
            return Err(Error::InvalidParameter);
        }
        Ok(Self {
            px,
            width,
            height,
            stride,
            flip,
            mono: false,
        })
    }
    /// Mark the canvas monochrome (black and white only).
    pub fn with_mono(mut self, mono: bool) -> Self {
        self.mono = mono;
        self
    }
    pub fn is_mono(&self) -> bool {
        self.mono
    }
    /// A color operand as this canvas realizes it.
    pub fn realize(&self, pixel: u32) -> u32 {
        if self.mono {
            mono_pixel(pixel)
        } else {
            pixel & RGB
        }
    }
    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width as i32, self.height as i32)
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    fn row(&self, y: i32) -> usize {
        let y = if self.flip {
            self.height as usize - 1 - y as usize
        } else {
            y as usize
        };
        y * self.stride
    }
    pub fn get(&self, x: i32, y: i32) -> Option<u32> {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return None;
        }
        Some(self.px[self.row(y) + x as usize] & RGB)
    }
    /// Store one pixel inside the canvas; false outside it.
    pub fn put(&mut self, x: i32, y: i32, value: u32) -> bool {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return false;
        }
        self.mix(x, y, |_| value);
        true
    }
    /// Replace the in-bounds pixel `(x, y)` by `f(old)`, realized.
    fn mix(&mut self, x: i32, y: i32, f: impl FnOnce(u32) -> u32) {
        let at = self.row(y) + x as usize;
        let v = f(self.px[at]);
        self.px[at] = self.realize(v);
    }
    /// Copy `r` (clipped to the canvas) into an owned source block.
    pub fn read(&self, r: Rect) -> Result<Block> {
        self.read_with(r, RGB)
    }
    /// As `read`, keeping each pixel's upper byte (a 32-bpp DIB's alpha,
    /// which AlphaBlend's AC_SRC_ALPHA uses).
    pub fn read_alpha(&self, r: Rect) -> Result<Block> {
        self.read_with(r, u32::MAX)
    }
    fn read_with(&self, r: Rect, keep: u32) -> Result<Block> {
        let r = r.intersect(self.bounds());
        let (w, h) = (r.width().max(0) as usize, r.height().max(0) as usize);
        let mut px = Vec::new();
        px.try_reserve_exact(w * h).map_err(|_| Error::Quota)?;
        for y in r.top..r.bottom {
            let row = self.row(y);
            px.extend(
                self.px[row + r.left as usize..row + r.right as usize]
                    .iter()
                    .map(|p| p & keep),
            );
        }
        Ok(Block {
            origin: r.top_left(),
            width: w as u32,
            height: h as u32,
            px,
        })
    }
}

/// Owned copy of source pixels at `origin` (source device coordinates).
/// Copying first makes overlapping same-surface blits exact.
pub struct Block {
    pub origin: Point,
    pub width: u32,
    pub height: u32,
    pub px: Vec<u32>,
}
impl Block {
    pub fn get(&self, x: i32, y: i32) -> Option<u32> {
        self.get_at(i64::from(x), i64::from(y))
    }
    /// The pixel at wide source coordinates (none outside the block).
    pub fn get_at(&self, x: i64, y: i64) -> Option<u32> {
        let (lx, ly) = (x - i64::from(self.origin.x), y - i64::from(self.origin.y));
        if lx < 0 || ly < 0 || lx >= i64::from(self.width) || ly >= i64::from(self.height) {
            return None;
        }
        self.px
            .get(ly as usize * self.width as usize + lx as usize)
            .copied()
    }
}

/// Hatch bitmaps (HS_HORIZONTAL .. HS_DIAGCROSS); the most significant bit
/// of each row is the leftmost pixel. Set bits take the hatch color.
pub const HATCHES: [[u8; 8]; 6] = [
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff],
    [0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08],
    [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01],
    [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80],
    [0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0xff],
    [0x81, 0x42, 0x24, 0x18, 0x18, 0x24, 0x42, 0x81],
];

/// A realized brush. `None` from `at` means a transparent pixel (the
/// background of a hatch in TRANSPARENT mode): the destination is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fill {
    Null,
    Solid(u32),
    Hatch {
        style: usize,
        fg: u32,
        bg: Option<u32>,
    },
    Pattern {
        width: u32,
        height: u32,
        px: Vec<u32>,
    },
}
impl Fill {
    pub fn at(&self, x: i32, y: i32, org: Point) -> Option<u32> {
        match self {
            Self::Null => None,
            Self::Solid(p) => Some(*p & RGB),
            Self::Hatch { style, fg, bg } => {
                // Wrapping differences keep the exact value modulo 8.
                let row = HATCHES[(*style).min(5)][(y.wrapping_sub(org.y) & 7) as usize];
                if row >> (7 - (x.wrapping_sub(org.x) & 7)) & 1 != 0 {
                    Some(*fg & RGB)
                } else {
                    bg.map(|b| b & RGB)
                }
            }
            Self::Pattern { width, height, px } => {
                let (w, h) = (i64::from(*width), i64::from(*height));
                if w == 0 || h == 0 {
                    return None;
                }
                let px_x = (i64::from(x) - i64::from(org.x)).rem_euclid(w);
                let px_y = (i64::from(y) - i64::from(org.y)).rem_euclid(h);
                px.get((px_y * w + px_x) as usize).map(|p| p & RGB)
            }
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

/// Evaluate ROP3 index `index` (bits 16..23 of the DWORD code) bitwise.
pub fn rop3(index: u8, p: u32, s: u32, d: u32) -> u32 {
    match index {
        0xcc => return s & RGB,
        0xf0 => return p & RGB,
        0xaa => return d & RGB,
        0x00 => return 0,
        0xff => return RGB,
        _ => (),
    }
    let mut r = 0;
    for i in 0..8 {
        if index >> i & 1 != 0 {
            let pp = if i & 4 != 0 { p } else { !p };
            let ss = if i & 2 != 0 { s } else { !s };
            let dd = if i & 1 != 0 { d } else { !d };
            r |= pp & ss & dd;
        }
    }
    r & RGB
}
/// Evaluate a ROP2 mix (R2_BLACK = 1 .. R2_WHITE = 16).
pub fn rop2(mode: u32, pen: u32, d: u32) -> u32 {
    let table = mode.clamp(1, 16) - 1;
    let mut r = 0;
    for i in 0..4 {
        if table >> i & 1 != 0 {
            let pp = if i & 2 != 0 { pen } else { !pen };
            let dd = if i & 1 != 0 { d } else { !d };
            r |= pp & dd;
        }
    }
    r & RGB
}
pub const fn rop_index(rop: u32) -> u8 {
    (rop >> 16) as u8
}
pub const fn uses_source(rop: u32) -> bool {
    ((rop >> 2) ^ rop) & 0x0033_0000 != 0
}
pub const fn uses_pattern(rop: u32) -> bool {
    ((rop >> 4) ^ rop) & 0x000f_0000 != 0
}

fn clip_rects<'r>(clip: &'r Region, bounds: Rect, r: Rect) -> impl Iterator<Item = Rect> + 'r {
    clip.clipped(r.intersect(bounds))
}

/// Fill `r` with a brush mixed by a ROP2 mode (Rectangle/Ellipse interiors).
pub fn fill_rop2(canvas: &mut Canvas, r: Rect, fill: &Fill, org: Point, mode: u32, clip: &Region) {
    if fill.is_null() {
        return;
    }
    let bounds = canvas.bounds();
    for part in clip_rects(clip, bounds, r) {
        for y in part.top..part.bottom {
            for x in part.left..part.right {
                if let Some(p) = fill.at(x, y, org) {
                    let p = canvas.realize(p);
                    canvas.mix(x, y, |d| rop2(mode, p, d));
                }
            }
        }
    }
}

/// BitBlt-family core: `dst` is the device rectangle, `src` the optional
/// source block with the source point mapping to `dst.top_left()`.
pub fn blt(
    canvas: &mut Canvas,
    dst: Rect,
    src: Option<(&Block, Point)>,
    fill: &Fill,
    org: Point,
    rop: u32,
    clip: &Region,
) -> Result<()> {
    let index = rop_index(rop);
    let need_src = uses_source(rop);
    let need_pat = uses_pattern(rop);
    if need_src && src.is_none() {
        return Err(Error::InvalidParameter);
    }
    let bounds = canvas.bounds();
    for part in clip_rects(clip, bounds, dst) {
        for y in part.top..part.bottom {
            for x in part.left..part.right {
                let s = match src {
                    Some((block, at)) if need_src => {
                        let sx = i64::from(at.x) + i64::from(x) - i64::from(dst.left);
                        let sy = i64::from(at.y) + i64::from(y) - i64::from(dst.top);
                        match block.get_at(sx, sy) {
                            Some(s) => canvas.realize(s),
                            None => continue,
                        }
                    }
                    _ => 0,
                };
                let p = if need_pat {
                    match fill.at(x, y, org) {
                        Some(p) => canvas.realize(p),
                        None => continue,
                    }
                } else {
                    0
                };
                canvas.mix(x, y, |d| rop3(index, p, s, d));
            }
        }
    }
    Ok(())
}

/// Nearest-neighbour StretchBlt (COLORONCOLOR) from the source rectangle with
/// corners `source` onto the destination rectangle with corners `dst`; a
/// reversed extent (second corner before the first) mirrors that axis.
#[allow(clippy::too_many_arguments)]
pub fn stretch(
    canvas: &mut Canvas,
    dst: (Point, Point),
    src: &Block,
    source: (Point, Point),
    fill: &Fill,
    org: Point,
    rop: u32,
    clip: &Region,
) -> Result<()> {
    let ((da, db), (sa, sb)) = (dst, source);
    let extent = |a: i32, b: i32| i64::from(b) - i64::from(a);
    let (dw, dh) = (extent(da.x, db.x), extent(da.y, db.y));
    let (sw, sh) = (extent(sa.x, sb.x), extent(sa.y, sb.y));
    if dw == 0 || dh == 0 || sw == 0 || sh == 0 {
        return Ok(());
    }
    let index = rop_index(rop);
    let need_pat = uses_pattern(rop);
    let rect = Rect::new(da.x, da.y, db.x, db.y).normalized();
    let mirror_x = (dw < 0) != (sw < 0);
    let mirror_y = (dh < 0) != (sh < 0);
    let (adw, adh, asw, ash) = (
        i128::from(dw.abs()),
        i128::from(dh.abs()),
        i128::from(sw.abs()),
        i128::from(sh.abs()),
    );
    let s_left = i64::from(sa.x.min(sb.x));
    let s_top = i64::from(sa.y.min(sb.y));
    let bounds = canvas.bounds();
    for part in clip_rects(clip, bounds, rect) {
        for y in part.top..part.bottom {
            let mut ry = i128::from(y) - i128::from(rect.top);
            if mirror_y {
                ry = adh - 1 - ry;
            }
            let src_y = s_top + (ry * ash / adh) as i64;
            for x in part.left..part.right {
                let mut rx = i128::from(x) - i128::from(rect.left);
                if mirror_x {
                    rx = adw - 1 - rx;
                }
                let src_x = s_left + (rx * asw / adw) as i64;
                let Some(s) = src.get_at(src_x, src_y) else {
                    continue;
                };
                let s = canvas.realize(s);
                let p = if need_pat {
                    match fill.at(x, y, org) {
                        Some(p) => canvas.realize(p),
                        None => continue,
                    }
                } else {
                    0
                };
                canvas.mix(x, y, |d| rop3(index, p, s, d));
            }
        }
    }
    Ok(())
}

fn octant(dx: i64, dy: i64) -> u32 {
    let n = if dy > 0 {
        if dx > 0 {
            if dx > dy { 1 } else { 2 }
        } else if -dx > dy {
            4
        } else {
            3
        }
    } else if dx < 0 {
        if -dx > -dy { 5 } else { 6 }
    } else if dx > -dy {
        8
    } else {
        7
    };
    1 << (n - 1)
}

/// Closed form of GDI's Bresenham walk from `a` towards `b`. The walk takes
/// `n = |major delta|` steps; after `k` steps the minor offset is
/// `floor((n - 1 + bias + 2 * |minor delta| * k) / (2 * n))`, which is exactly
/// what the incremental error term produces (its value before step `k` is
/// `2 * |minor delta| * (k + 1) - n - 2 * n * minor(k)`). This lets every
/// caller start at the first visible step and stop after the last one.
struct Walk {
    major0: i64,
    minor0: i64,
    major_step: i64,
    minor_step: i64,
    /// |major delta|: the number of pixels (the end point is excluded).
    n: i64,
    /// |minor delta|.
    m: i64,
    bias: i64,
    x_major: bool,
}
impl Walk {
    fn new(a: Point, b: Point) -> Option<Self> {
        let (dx, dy) = (
            i64::from(b.x) - i64::from(a.x),
            i64::from(b.y) - i64::from(a.y),
        );
        if dx == 0 && dy == 0 {
            return None;
        }
        // Octants 3, 5, 6 and 8 take a bias, which fixes the choice at exact
        // half-pixel ties.
        let oct = octant(dx, dy);
        let bias = i64::from(oct & 0xb4 != 0);
        let x_major = oct & 0x99 != 0;
        let (ax, ay, sx, sy) = (i64::from(a.x), i64::from(a.y), dx.signum(), dy.signum());
        Some(if x_major {
            Self {
                major0: ax,
                minor0: ay,
                major_step: sx,
                minor_step: sy,
                n: dx.abs(),
                m: dy.abs(),
                bias,
                x_major,
            }
        } else {
            Self {
                major0: ay,
                minor0: ax,
                major_step: sy,
                minor_step: sx,
                n: dy.abs(),
                m: dx.abs(),
                bias,
                x_major,
            }
        })
    }
    /// Minor offset after `k` steps (`0 <= k <= n`).
    fn minor(&self, k: i64) -> i64 {
        let num =
            i128::from(self.n) - 1 + i128::from(self.bias) + 2 * i128::from(self.m) * i128::from(k);
        (num / (2 * i128::from(self.n))) as i64
    }
    /// The incremental error term before step `k` with minor offset `minor`.
    fn error(&self, k: i64, minor: i64) -> i64 {
        let (n, m) = (i128::from(self.n), i128::from(self.m));
        (2 * m * (i128::from(k) + 1) - n - 2 * n * i128::from(minor)) as i64
    }
    /// `(x, y)` of step `k` with minor offset `minor`.
    fn point(&self, k: i64, minor: i64) -> (i64, i64) {
        let major = self.major0 + self.major_step * k;
        let minor = self.minor0 + self.minor_step * minor;
        if self.x_major {
            (major, minor)
        } else {
            (minor, major)
        }
    }
    fn clamp(&self, k: i128) -> i64 {
        k.clamp(0, i128::from(self.n)) as i64
    }
    /// Steps `[k0, k1)` within `[0, n)` whose major coordinate lies in
    /// `[lo, hi]`.
    fn major_range(&self, lo: i64, hi: i64) -> (i64, i64) {
        let (lo, hi, a) = (i128::from(lo), i128::from(hi), i128::from(self.major0));
        let (k0, k1) = if self.major_step > 0 {
            (lo - a, hi - a + 1)
        } else {
            (a - hi, a - lo + 1)
        };
        (self.clamp(k0), self.clamp(k1))
    }
    /// Steps `[k0, k1)` within `[0, n)` whose minor coordinate lies in
    /// `[lo, hi]` (an interval: the minor offset never decreases).
    fn minor_range(&self, lo: i64, hi: i64) -> (i64, i64) {
        let (lo, hi, a) = (i128::from(lo), i128::from(hi), i128::from(self.minor0));
        let (low, high) = match self.minor_step {
            1 => (lo - a, hi - a),
            -1 => (a - hi, a - lo),
            _ => {
                return if lo <= a && a <= hi {
                    (0, self.n)
                } else {
                    (0, 0)
                };
            }
        };
        if low > high {
            return (0, 0);
        }
        // minor(k) >= low  <=>  c + d * k >= low * e
        // minor(k) <= high <=>  c + d * k <= (high + 1) * e - 1
        let c = i128::from(self.n) - 1 + i128::from(self.bias);
        let (d, e) = (2 * i128::from(self.m), 2 * i128::from(self.n));
        let k0 = -(c - low * e).div_euclid(d);
        let k1 = ((high + 1) * e - 1 - c).div_euclid(d) + 1;
        (self.clamp(k0), self.clamp(k1))
    }
    /// Steps whose pixel lies in `[x0, x1] x [y0, y1]`.
    fn visible(&self, x: (i64, i64), y: (i64, i64)) -> (i64, i64) {
        let (major, minor) = if self.x_major { (x, y) } else { (y, x) };
        let (a0, a1) = self.major_range(major.0, major.1);
        let (b0, b1) = self.minor_range(minor.0, minor.1);
        (a0.max(b0), a1.min(b1))
    }
}

/// Pixels of the cosmetic line from `a` towards `b`, excluding `b`, that lie
/// in `within`, using GDI's Bresenham variant: octants 3, 5, 6 and 8 take a
/// bias, which fixes the choice at exact half-pixel ties. Only visible steps
/// are walked: a line of any length costs at most the extent of `within`.
pub fn line_points(a: Point, b: Point, within: Rect, mut plot: impl FnMut(i32, i32)) {
    let Some(w) = Walk::new(a, b) else {
        return;
    };
    if within.is_empty() {
        return;
    }
    let (k0, k1) = w.visible(
        (i64::from(within.left), i64::from(within.right) - 1),
        (i64::from(within.top), i64::from(within.bottom) - 1),
    );
    if k0 >= k1 {
        return;
    }
    let mut minor = w.minor(k0);
    let mut err = w.error(k0, minor);
    let (add1, add2) = (2 * w.m - 2 * w.n, 2 * w.m);
    for k in k0..k1 {
        let (x, y) = w.point(k, minor);
        plot(x as i32, y as i32);
        if err + w.bias > 0 {
            minor += 1;
            err += add1;
        } else {
            err += add2;
        }
    }
}

/// Cosmetic one-pixel line with a ROP2 mix.
pub fn line(canvas: &mut Canvas, a: Point, b: Point, color: u32, mode: u32, clip: &Region) {
    let within = canvas.bounds().intersect(clip.bounds());
    let color = canvas.realize(color);
    line_points(a, b, within, |x, y| {
        if clip.contains(Point::new(x, y)) {
            canvas.mix(x, y, |d| rop2(mode, color, d));
        }
    });
}

/// Wide line: the union of the `width`-pixel squares centred (offset by
/// `width / 2`) on every pixel of the cosmetic line, each covered pixel mixed
/// once. Computed per column from the closed-form walk, so the work is
/// bounded by the visible area whatever the coordinates and width.
pub fn wide_line(
    canvas: &mut Canvas,
    a: Point,
    b: Point,
    width: i32,
    color: u32,
    mode: u32,
    clip: &Region,
) -> Result<()> {
    let Some(w) = Walk::new(a, b) else {
        return Ok(());
    };
    let area = canvas.bounds().intersect(clip.bounds());
    if area.is_empty() {
        return Ok(());
    }
    let size = i64::from(width.max(1));
    let half = size / 2;
    // Column c is covered by the squares of pixels with x in
    // [c - size + half + 1, c + half]; their rows form one interval because
    // the line is monotonic in both axes.
    let mut spans: Vec<(i32, i32)> = Vec::new();
    spans
        .try_reserve_exact(area.width() as usize)
        .map_err(|_| Error::Quota)?;
    for c in area.left..area.right {
        let (lo, hi) = (i64::from(c) - size + half + 1, i64::from(c) + half);
        let (k0, k1) = if w.x_major {
            w.major_range(lo, hi)
        } else {
            w.minor_range(lo, hi)
        };
        if k0 >= k1 {
            spans.push((0, 0));
            continue;
        }
        let (_, y0) = w.point(k0, w.minor(k0));
        let (_, y1) = w.point(k1 - 1, w.minor(k1 - 1));
        let top = (y0.min(y1) - half).max(i64::from(area.top));
        let bottom = (y0.max(y1) - half + size).min(i64::from(area.bottom));
        spans.push(if top < bottom {
            (top as i32, bottom as i32)
        } else {
            (0, 0)
        });
    }
    let color = canvas.realize(color);
    for part in clip.clipped(area) {
        for x in part.left..part.right {
            let (top, bottom) = spans[(x - area.left) as usize];
            for y in top.max(part.top)..bottom.min(part.bottom) {
                canvas.mix(x, y, |d| rop2(mode, color, d));
            }
        }
    }
    Ok(())
}

/// Single pixel with a ROP2 mix (SetPixel uses R2_COPYPEN).
pub fn plot(canvas: &mut Canvas, p: Point, color: u32, mode: u32, clip: &Region) -> bool {
    if !canvas.bounds().contains(p) || !clip.contains(p) {
        return false;
    }
    let color = canvas.realize(color);
    canvas.mix(p.x, p.y, |d| rop2(mode, color, d));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::{r2, rop};
    fn all(w: u32, h: u32) -> Region {
        Region::from_rect(Rect::new(0, 0, w as i32, h as i32))
    }
    const EVERYWHERE: Rect = Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);

    #[test]
    fn blend_composites_premultiplied_pixels_with_constant_alpha() {
        let mut px = [0x0000_00ffu32; 4];
        let mut c = Canvas::new(&mut px, 2, 2, 2).unwrap();
        // Opaque red, half-transparent white (straight), transparent.
        let mut src = [0xffff_0000u32, 0x80ff_ffff, 0x0012_3456, 0xffff_0000];
        premultiply(&mut src);
        let block = Block {
            origin: Point::new(0, 0),
            width: 2,
            height: 2,
            px: src.to_vec(),
        };
        let s = BlendSource {
            block: &block,
            use_alpha: true,
            constant: 255,
        };
        blend(
            &mut c,
            Rect::new(0, 0, 2, 2),
            s,
            Rect::new(0, 0, 2, 2),
            &all(2, 2),
        )
        .unwrap();
        assert_eq!(px[0], 0x00ff_0000);
        // 0x80 white over blue: 128 + 127 * (127 / 255) in blue.
        assert_eq!(px[1] >> 16 & 255, 128);
        assert!((px[1] & 255) >= 0xfe);
        assert_eq!(px[2], 0x0000_00ff, "a transparent pixel leaves the canvas");
        // Constant alpha halves an opaque source; no alpha means opaque.
        let mut px = [0u32; 1];
        let mut c = Canvas::new(&mut px, 1, 1, 1).unwrap();
        let s = BlendSource {
            block: &block,
            use_alpha: false,
            constant: 128,
        };
        blend(
            &mut c,
            Rect::new(0, 0, 1, 1),
            s,
            Rect::new(0, 0, 1, 1),
            &all(1, 1),
        )
        .unwrap();
        assert_eq!(px[0], 0x0080_0000);
    }

    /// The original full-iteration walk, kept as the reference the clipped
    /// walk must reproduce pixel for pixel.
    fn reference_points(a: Point, b: Point, mut plot: impl FnMut(i32, i32)) {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        if dx == 0 && dy == 0 {
            return;
        }
        let oct = octant(i64::from(dx), i64::from(dy));
        let bias = i64::from(oct & 0xb4 != 0);
        let x_major = oct & 0x99 != 0;
        let (adx, ady) = (i64::from(dx.unsigned_abs()), i64::from(dy.unsigned_abs()));
        let (xi, yi) = (dx.signum(), dy.signum());
        let (mut x, mut y) = (a.x, a.y);
        if x_major {
            let (mut err, add1, add2) = (2 * ady - adx, 2 * ady - 2 * adx, 2 * ady);
            for _ in 0..adx {
                plot(x, y);
                if err + bias > 0 {
                    y += yi;
                    err += add1;
                } else {
                    err += add2;
                }
                x += xi;
            }
        } else {
            let (mut err, add1, add2) = (2 * adx - ady, 2 * adx - 2 * ady, 2 * adx);
            for _ in 0..ady {
                plot(x, y);
                if err + bias > 0 {
                    x += xi;
                    err += add1;
                } else {
                    err += add2;
                }
                y += yi;
            }
        }
    }
    /// Deterministic xorshift for exhaustive-ish sampling.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn range(&mut self, lo: i32, hi: i32) -> i32 {
            lo + (self.next() % (hi - lo + 1) as u64) as i32
        }
    }

    #[test]
    fn rop3_codes_match_their_boolean_definitions() {
        let (p, s, d) = (0x00f0_f00f, 0x00cc_33cc, 0x00aa_55aa);
        let named = [
            (rop::SRCCOPY, s),
            (rop::SRCPAINT, s | d),
            (rop::SRCAND, s & d),
            (rop::SRCINVERT, s ^ d),
            (rop::SRCERASE, s & !d),
            (rop::NOTSRCCOPY, !s),
            (rop::NOTSRCERASE, !(s | d)),
            (rop::MERGECOPY, s & p),
            (rop::MERGEPAINT, !s | d),
            (rop::PATCOPY, p),
            (rop::PATPAINT, p | !s | d),
            (rop::PATINVERT, p ^ d),
            (rop::DSTINVERT, !d),
            (rop::BLACKNESS, 0),
            (rop::WHITENESS, RGB),
            (0x00b8_074a, (p & !s) | (d & s)),
            (0x00e2_0746, (s & p) | (d & !s)),
        ];
        for (code, expect) in named {
            assert_eq!(rop3(rop_index(code), p, s, d), expect & RGB, "{code:#x}");
        }
        assert!(uses_source(rop::SRCCOPY) && !uses_source(rop::PATCOPY));
        assert!(uses_pattern(rop::PATINVERT) && !uses_pattern(rop::SRCAND));
        assert!(!uses_source(rop::DSTINVERT) && !uses_pattern(rop::DSTINVERT));
    }
    #[test]
    fn rop2_modes() {
        let (p, d) = (0x00ff_0f00, 0x0000_ff0f);
        assert_eq!(rop2(r2::COPYPEN, p, d), p);
        assert_eq!(rop2(r2::XORPEN, p, d), p ^ d);
        assert_eq!(rop2(r2::NOT, p, d), !d & RGB);
        assert_eq!(rop2(r2::NOP, p, d), d);
        assert_eq!(rop2(r2::BLACK, p, d), 0);
        assert_eq!(rop2(r2::WHITE, p, d), RGB);
        assert_eq!(rop2(r2::MASKPENNOT, p, d), p & !d & RGB);
        assert_eq!(rop2(r2::MERGENOTPEN, p, d), (!p | d) & RGB);
    }
    #[test]
    fn lines_follow_the_gdi_bias_rule_and_exclude_the_end() {
        let mut pts = Vec::new();
        let walk = |a: Point, b: Point| {
            let mut v = Vec::new();
            line_points(a, b, EVERYWHERE, |x, y| v.push((x, y)));
            v
        };
        assert_eq!(
            walk(Point::new(0, 0), Point::new(4, 2)),
            [(0, 0), (1, 0), (2, 1), (3, 1)]
        );
        // Octant 3 (steep, leftwards, downwards) takes the bias.
        pts.extend(walk(Point::new(92, 3), Point::new(76, 27)));
        assert_eq!(pts.len(), 24);
        assert_eq!(&pts[..4], &[(92, 3), (91, 4), (91, 5), (90, 6)]);
        assert_eq!(
            walk(Point::new(4, 2), Point::new(0, 0)),
            [(4, 2), (3, 1), (2, 1), (1, 0)],
            "octant 5 bias"
        );
        assert!(walk(Point::new(5, 5), Point::new(5, 5)).is_empty());
    }
    #[test]
    fn clipped_walk_matches_the_full_reference_walk() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let windows = [
            EVERYWHERE,
            Rect::new(0, 0, 64, 48),
            Rect::new(-7, 5, 9, 30),
            Rect::new(20, -40, 21, 40),
            Rect::new(-100, 13, 100, 14),
        ];
        for i in 0..20_000 {
            // Small lines cover every octant and tie; larger ones cross the
            // windows partially.
            let span = if i % 4 == 0 { 300 } else { 40 };
            let a = Point::new(rng.range(-span, span), rng.range(-span, span));
            let b = Point::new(rng.range(-span, span), rng.range(-span, span));
            for within in windows {
                let mut expect = Vec::new();
                reference_points(a, b, |x, y| {
                    if within.contains(Point::new(x, y)) {
                        expect.push((x, y));
                    }
                });
                let mut got = Vec::new();
                line_points(a, b, within, |x, y| got.push((x, y)));
                assert_eq!(got, expect, "{a:?} -> {b:?} within {within:?}");
            }
        }
        // Exact diagonals, axis lines and the 2:1 ties of all eight octants.
        for (dx, dy) in [
            (5, 5),
            (-5, 5),
            (5, -5),
            (-5, -5),
            (7, 0),
            (-7, 0),
            (0, 7),
            (0, -7),
            (4, 2),
            (2, 4),
            (-2, 4),
            (-4, 2),
            (-4, -2),
            (-2, -4),
            (2, -4),
            (4, -2),
        ] {
            let (a, b) = (Point::new(3, 3), Point::new(3 + dx, 3 + dy));
            let mut expect = Vec::new();
            reference_points(a, b, |x, y| expect.push((x, y)));
            let mut got = Vec::new();
            line_points(a, b, Rect::new(-20, -20, 20, 20), |x, y| got.push((x, y)));
            assert_eq!(got, expect, "{dx},{dy}");
        }
    }
    #[test]
    fn hostile_line_lengths_cost_only_the_visible_part() {
        let started = std::time::Instant::now();
        let mut px = alloc::vec![0u32; 64 * 64];
        let mut c = Canvas::new(&mut px, 64, 64, 64).unwrap();
        let clip = all(64, 64);
        let far = [
            (Point::new(0, 0), Point::new(1 << 30, 1 << 29)),
            (Point::new(0, 0), Point::new(i32::MAX, 1 << 30)),
            (
                Point::new(i32::MIN, i32::MIN),
                Point::new(i32::MAX, i32::MAX),
            ),
            (Point::new(i32::MAX, 10), Point::new(i32::MIN, 20)),
            (Point::new(5, i32::MIN), Point::new(6, i32::MAX)),
            (Point::new(i32::MIN, 0), Point::new(i32::MIN + 3, i32::MAX)),
        ];
        for (a, b) in far {
            line(&mut c, a, b, 0x00ff_ffff, r2::COPYPEN, &clip);
            wide_line(&mut c, a, b, 9, 0x0000_ff00, r2::COPYPEN, &clip).unwrap();
            wide_line(&mut c, a, b, i32::MAX, 0x0000_00ff, r2::XORPEN, &clip).unwrap();
        }
        // The first line starts with exactly the pixels of its 2:1 slope.
        let mut first = Vec::new();
        line_points(far[0].0, far[0].1, Rect::new(0, 0, 4, 4), |x, y| {
            first.push((x, y))
        });
        assert_eq!(first, [(0, 0), (1, 0), (2, 1), (3, 1)]);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
    #[test]
    fn wide_lines_match_the_union_of_squares() {
        let mut rng = Rng(0x1234_5678_9abc_def1);
        for _ in 0..1500 {
            let a = Point::new(rng.range(-20, 52), rng.range(-20, 52));
            let b = Point::new(rng.range(-20, 52), rng.range(-20, 52));
            let width = rng.range(2, 9);
            let clip = if rng.next().is_multiple_of(2) {
                all(32, 32)
            } else {
                Region::from_rects(&[Rect::new(3, 0, 20, 12), Rect::new(10, 14, 32, 30)]).unwrap()
            };
            let mut expect = alloc::vec![0u32; 32 * 32];
            {
                let mut c = Canvas::new(&mut expect, 32, 32, 32).unwrap();
                let half = width / 2;
                let mut pts = Vec::new();
                reference_points(a, b, |x, y| pts.push(Point::new(x, y)));
                for p in pts {
                    let r = Rect::from_size(p.x - half, p.y - half, width, width);
                    fill_rop2(
                        &mut c,
                        r,
                        &Fill::Solid(0x00ab_cdef),
                        Point::default(),
                        13,
                        &clip,
                    );
                }
            }
            let mut got = alloc::vec![0u32; 32 * 32];
            let mut c = Canvas::new(&mut got, 32, 32, 32).unwrap();
            wide_line(&mut c, a, b, width, 0x00ab_cdef, 13, &clip).unwrap();
            assert_eq!(got, expect, "{a:?} -> {b:?} width {width}");
        }
    }
    #[test]
    fn hatch_patterns_align_to_the_brush_origin() {
        let h = Fill::Hatch {
            style: 5,
            fg: 0x00ff_8000,
            bg: Some(0x0040_0040),
        };
        assert_eq!(h.at(0, 0, Point::new(0, 0)), Some(0x00ff_8000));
        assert_eq!(h.at(1, 0, Point::new(0, 0)), Some(0x0040_0040));
        assert_eq!(h.at(9, 1, Point::new(0, 0)), Some(0x00ff_8000));
        assert_eq!(h.at(1, 0, Point::new(1, 0)), Some(0x00ff_8000));
        let t = Fill::Hatch {
            style: 0,
            fg: 1,
            bg: None,
        };
        assert_eq!(t.at(3, 2, Point::new(0, 0)), None);
        assert_eq!(t.at(3, 7, Point::new(0, 0)), Some(1));
        let pat = Fill::Pattern {
            width: 2,
            height: 1,
            px: alloc::vec![5, 6],
        };
        assert_eq!(pat.at(-1, 9, Point::new(0, 0)), Some(6));
        // Extreme brush origins neither overflow nor lose the phase.
        assert_eq!(
            h.at(0, 0, Point::new(i32::MIN, i32::MIN)),
            h.at(0, 0, Point::new(0, 0))
        );
        assert_eq!(pat.at(0, 0, Point::new(i32::MIN, i32::MAX)), Some(5));
        assert_eq!(pat.at(i32::MIN, 0, Point::new(i32::MAX, 0)), Some(6));
    }
    #[test]
    fn blits_clip_and_handle_overlap() {
        let mut px = alloc::vec![0u32; 16];
        for (i, p) in px.iter_mut().enumerate() {
            *p = i as u32;
        }
        let mut c = Canvas::new(&mut px, 4, 4, 4).unwrap();
        let block = c.read(Rect::new(0, 0, 3, 3)).unwrap();
        // Overlapping copy one pixel right/down.
        blt(
            &mut c,
            Rect::new(1, 1, 4, 4),
            Some((&block, Point::new(0, 0))),
            &Fill::Null,
            Point::default(),
            rop::SRCCOPY,
            &all(4, 4),
        )
        .unwrap();
        assert_eq!(c.get(1, 1), Some(0));
        assert_eq!(c.get(3, 3), Some(10));
        let clip = Region::from_rect(Rect::new(0, 0, 2, 4));
        blt(
            &mut c,
            Rect::new(0, 0, 4, 4),
            None,
            &Fill::Solid(0x00ab_cdef),
            Point::default(),
            rop::PATCOPY,
            &clip,
        )
        .unwrap();
        assert_eq!(c.get(1, 3), Some(0x00ab_cdef));
        assert_eq!(c.get(2, 3), Some(9));
        assert!(
            blt(
                &mut c,
                Rect::new(0, 0, 1, 1),
                None,
                &Fill::Null,
                Point::default(),
                rop::SRCCOPY,
                &clip
            )
            .is_err()
        );
        assert!(Canvas::new(&mut [0u32; 3], 2, 2, 2).is_err());
        assert!(Canvas::new(&mut [0u32; 3], 2, 2, 1).is_err());
    }
    #[test]
    fn hostile_blit_coordinates_clip_instead_of_overflowing() {
        let mut px = alloc::vec![7u32; 16];
        let mut c = Canvas::new(&mut px, 4, 4, 4).unwrap();
        let block = c.read(Rect::new(0, 0, 4, 4)).unwrap();
        for (dst, at) in [
            (
                Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX),
                Point::new(i32::MAX, i32::MAX),
            ),
            (Rect::new(i32::MIN, 0, 4, 4), Point::new(i32::MAX, 0)),
            (Rect::new(0, 0, 4, 4), Point::new(i32::MIN, i32::MIN)),
            (Rect::new(i32::MIN, i32::MIN, 4, 4), Point::new(0, 0)),
        ] {
            blt(
                &mut c,
                dst,
                Some((&block, at)),
                &Fill::Solid(1),
                Point::new(i32::MIN, i32::MAX),
                0x00b8_074a,
                &all(4, 4),
            )
            .unwrap();
        }
        let corners = [
            Point::new(i32::MIN, i32::MIN),
            Point::new(i32::MAX, i32::MAX),
            Point::new(0, 0),
            Point::new(4, 4),
            Point::new(i32::MAX, i32::MIN),
        ];
        for a in corners {
            for b in corners {
                for (sa, sb) in [(a, b), (Point::new(0, 0), Point::new(4, 4)), (b, a)] {
                    stretch(
                        &mut c,
                        (a, b),
                        &block,
                        (sa, sb),
                        &Fill::Null,
                        Point::default(),
                        rop::SRCCOPY,
                        &all(4, 4),
                    )
                    .unwrap();
                }
            }
        }
        assert!(px.iter().all(|p| *p <= 0x00ff_ffff));
    }
    #[test]
    fn stretch_scales_and_mirrors() {
        let src = Block {
            origin: Point::new(0, 0),
            width: 2,
            height: 1,
            px: alloc::vec![1, 2],
        };
        let mut px = alloc::vec![0u32; 4];
        let mut c = Canvas::new(&mut px, 4, 1, 4).unwrap();
        let corners = |x, y, w, h| (Point::new(x, y), Point::new(x + w, y + h));
        stretch(
            &mut c,
            corners(0, 0, 4, 1),
            &src,
            corners(0, 0, 2, 1),
            &Fill::Null,
            Point::default(),
            rop::SRCCOPY,
            &all(4, 1),
        )
        .unwrap();
        assert_eq!(px, [1, 1, 2, 2]);
        let mut c = Canvas::new(&mut px, 4, 1, 4).unwrap();
        stretch(
            &mut c,
            corners(4, 0, -4, 1),
            &src,
            corners(0, 0, 2, 1),
            &Fill::Null,
            Point::default(),
            rop::SRCCOPY,
            &all(4, 1),
        )
        .unwrap();
        assert_eq!(px, [2, 2, 1, 1]);
    }
    #[test]
    fn mono_canvases_store_black_and_white_only() {
        let mut px = alloc::vec![0u32; 4];
        let mut c = Canvas::new(&mut px, 4, 1, 4).unwrap().with_mono(true);
        let clip = all(4, 1);
        plot(&mut c, Point::new(0, 0), 0x00c0_c0c0, r2::COPYPEN, &clip);
        plot(&mut c, Point::new(1, 0), 0x0040_4040, r2::COPYPEN, &clip);
        plot(&mut c, Point::new(2, 0), 0x00ff_0000, r2::COPYPEN, &clip);
        c.put(3, 0, 0x00ff_ffff);
        assert_eq!(px, [0x00ff_ffff, 0, 0, 0x00ff_ffff]);
    }
}

/// Source pixels for [`blend`]: `0xAARRGGBB`, alpha premultiplied
/// (AlphaBlend's AC_SRC_ALPHA, a 32-bpp icon after `premultiply`) or
/// ignored (`use_alpha` false: every source pixel is opaque).
#[derive(Clone, Copy)]
pub struct BlendSource<'b> {
    pub block: &'b Block,
    pub use_alpha: bool,
    /// SourceConstantAlpha, 0..=255, applied on top of the pixel alpha.
    pub constant: u8,
}
/// Premultiply straight-alpha `0xAARRGGBB` pixels in place.
pub fn premultiply(px: &mut [u32]) {
    for p in px {
        let a = *p >> 24;
        let scale = |c: u32| ((c & 255) * a + 127) / 255;
        *p = a << 24 | scale(*p >> 16) << 16 | scale(*p >> 8) << 8 | scale(*p);
    }
}
/// AlphaBlend (AC_SRC_OVER): the corners `source` of the block scaled
/// (nearest neighbour) onto the destination corners `dst`, composited over
/// the canvas: `D = S * c + D * (1 - Sa * c)` per channel, `c` the constant
/// alpha. Unlike the raster operations this mixes, as AlphaBlend does.
pub fn blend(
    canvas: &mut Canvas,
    dst: Rect,
    src: BlendSource,
    source: Rect,
    clip: &Region,
) -> Result<()> {
    let (dw, dh) = (i128::from(dst.width()), i128::from(dst.height()));
    let (sw, sh) = (i128::from(source.width()), i128::from(source.height()));
    if dw <= 0 || dh <= 0 || sw <= 0 || sh <= 0 {
        return Ok(());
    }
    let c = u32::from(src.constant);
    let bounds = canvas.bounds();
    for part in clip_rects(clip, bounds, dst) {
        for y in part.top..part.bottom {
            let sy =
                i64::from(source.top) + ((i128::from(y) - i128::from(dst.top)) * sh / dh) as i64;
            for x in part.left..part.right {
                let sx = i64::from(source.left)
                    + ((i128::from(x) - i128::from(dst.left)) * sw / dw) as i64;
                let Some(s) = src.block.get_at(sx, sy) else {
                    continue;
                };
                let sa = if src.use_alpha { s >> 24 } else { 255 };
                // Source channels scaled by the constant alpha; the
                // destination keeps 1 - (source alpha * constant).
                let keep = 255 - (sa * c + 127) / 255;
                let s = if src.use_alpha { s } else { s | 0xff00_0000 };
                canvas.mix(x, y, |d| {
                    let ch = |shift: u32| {
                        let sc = (s >> shift & 255) * c / 255;
                        let dc = (d >> shift & 255) * keep / 255;
                        (sc + dc).min(255) << shift
                    };
                    ch(16) | ch(8) | ch(0)
                });
            }
        }
    }
    Ok(())
}
