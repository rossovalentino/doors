//! Live acrylic backdrop: the scene already composed beneath one window in
//! this frame (before its own shadow), area-downsampled and blurred, then
//! sampled bilinearly as that window's material base. Unlike the wallpaper
//! material it shows the windows behind at their current position/stacking.
use super::*;

/// Block size of the area prefilter, in device-independent pixels.
const BLOCK: usize = 8;
/// Scene sampled beyond the window edge so blurred edges see real content:
/// two passes of a radius-2 box over 8-DIP blocks reach 32 DIPs, plus one
/// block for the bilinear tap. Physical pixels at `scale`.
pub(crate) fn margin(scale: Scale) -> i32 {
    scale.len(5 * BLOCK as u32) as i32
}

pub(crate) struct LiveBackdrop {
    origin: (i32, i32),
    block: usize,
    width: usize,
    height: usize,
    pixels: Vec<[u8; 4]>,
}
impl LiveBackdrop {
    /// `area` must lie inside the frame and hold only pixels composed beneath
    /// the window in this frame; the engine expands damage to guarantee it.
    pub(crate) fn capture(frame: &[u8], stride: usize, area: Rect, scale: Scale) -> Option<Self> {
        let block = scale.len(BLOCK as u32).max(1) as usize;
        let (w, h) = (
            (area.width() as usize).div_ceil(block),
            (area.height() as usize).div_ceil(block),
        );
        let mut pixels = Vec::new();
        pixels.try_reserve_exact(w * h).ok()?;
        pixels.resize(w * h, [0, 0, 0, 255]);
        let mut scratch = Vec::new();
        scratch.try_reserve_exact(w * h).ok()?;
        scratch.resize(w * h, [0, 0, 0, 255]);
        for gy in 0..h {
            let y0 = area.y() as usize + gy * block;
            let y1 = (y0 + block).min(area.bottom() as usize);
            for gx in 0..w {
                let x0 = area.x() as usize + gx * block;
                let x1 = (x0 + block).min(area.right() as usize);
                let mut sum = [0u32; 3];
                for y in y0..y1 {
                    for p in frame[y * stride + x0 * 4..y * stride + x1 * 4].chunks_exact(4) {
                        sum[0] += u32::from(p[0]);
                        sum[1] += u32::from(p[1]);
                        sum[2] += u32::from(p[2]);
                    }
                }
                let count = ((y1 - y0) * (x1 - x0)).max(1) as u32;
                let cell = &mut pixels[gy * w + gx];
                for c in 0..3 {
                    cell[c] = ((sum[c] + count / 2) / count) as u8;
                }
            }
        }
        // Two separable radius-2 box passes approximate a Gaussian.
        for _ in 0..2 {
            blur(&pixels, &mut scratch, w, h, true);
            blur(&scratch, &mut pixels, w, h, false);
        }
        Some(Self {
            origin: (area.x(), area.y()),
            block,
            width: w,
            height: h,
            pixels,
        })
    }
    /// Vertically interpolated grid row for output row `y`; see
    /// `material::Backdrop::row`. Identical to per-pixel `pixel` samples.
    pub(crate) fn row(&self, y: i32, cells: &mut [[u16; 3]]) -> Option<render::RowBase> {
        if self.width > cells.len() {
            return None;
        }
        let block = self.block;
        let sy = (y - self.origin.1 - block as i32 / 2).max(0) as usize;
        let y0 = (sy / block).min(self.height - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let fy = (sy % block) as u16;
        let weight = block as u16;
        for (gx, cell) in cells[..self.width].iter_mut().enumerate() {
            let (a, b) = (
                self.pixels[y0 * self.width + gx],
                self.pixels[y1 * self.width + gx],
            );
            *cell =
                core::array::from_fn(|c| u16::from(a[c]) * (weight - fy) + u16::from(b[c]) * fy);
        }
        Some(render::RowBase {
            origin: self.origin.0,
            half: block as i32 / 2,
            block,
            cells: self.width,
        })
    }
    /// Bilinear sample at an output pixel; samples beyond the captured area
    /// clamp to its edge cells.
    pub(crate) fn pixel(&self, x: i32, y: i32) -> [u8; 4] {
        let block = self.block;
        let half = block as i32 / 2;
        let sx = (x - self.origin.0 - half).max(0) as usize;
        let sy = (y - self.origin.1 - half).max(0) as usize;
        let (x0, y0) = (
            (sx / block).min(self.width - 1),
            (sy / block).min(self.height - 1),
        );
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (sx % block, sy % block);
        let (a, b) = (
            &self.pixels[y0 * self.width..],
            &self.pixels[y1 * self.width..],
        );
        let mut out = [0, 0, 0, 255];
        for (c, component) in out[..3].iter_mut().enumerate() {
            let top = usize::from(a[x0][c]) * (block - fx) + usize::from(a[x1][c]) * fx;
            let bottom = usize::from(b[x0][c]) * (block - fx) + usize::from(b[x1][c]) * fx;
            *component =
                ((top * (block - fy) + bottom * fy + block * block / 2) / (block * block)) as u8;
        }
        out
    }
}
fn blur(source: &[[u8; 4]], out: &mut [[u8; 4]], w: usize, h: usize, horizontal: bool) {
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 3];
            for d in -2i32..=2 {
                let (sx, sy) = if horizontal {
                    ((x as i32 + d).clamp(0, w as i32 - 1) as usize, y)
                } else {
                    (x, (y as i32 + d).clamp(0, h as i32 - 1) as usize)
                };
                let p = source[sy * w + sx];
                sum[0] += u32::from(p[0]);
                sum[1] += u32::from(p[1]);
                sum[2] += u32::from(p[2]);
            }
            out[y * w + x] = [
                (sum[0] / 5) as u8,
                (sum[1] / 5) as u8,
                (sum[2] / 5) as u8,
                255,
            ];
        }
    }
}
