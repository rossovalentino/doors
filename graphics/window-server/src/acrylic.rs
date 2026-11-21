//! In-shell acrylic: a downsampled, blurred copy of the scene underneath a
//! Start/flyout surface, upsampled bilinearly under a high-opacity tint. Built
//! once per backdrop change; painting it never re-blurs the scene.
extern crate alloc;
use alloc::vec::Vec;
use door_graphics_core::{Canvas, Rect};
#[derive(Default)]
pub struct Acrylic {
    /// Physical region the cells cover.
    region: Option<Rect>,
    cell: usize,
    cols: usize,
    rows: usize,
    data: Vec<u16>,
    scratch: Vec<u16>,
}
impl Acrylic {
    pub fn region(&self) -> Option<Rect> {
        self.region
    }
    pub fn clear(&mut self) {
        self.region = None;
    }
    /// Sample `region` (physical) of a full-output BGRA `scene` with row
    /// `stride` bytes into `cell`-pixel cells, then blur the cell grid.
    pub fn build(&mut self, scene: &[u8], stride: usize, region: Rect, cell: usize) -> bool {
        let cell = cell.max(2);
        if region.x() < 0 || region.y() < 0 {
            return false;
        }
        let (x0, y0) = (region.x() as usize, region.y() as usize);
        let (w, h) = (region.width() as usize, region.height() as usize);
        if stride < (x0 + w) * 4 || scene.len() < (y0 + h) * stride {
            return false;
        }
        let cols = w.div_ceil(cell) + 2;
        let rows = h.div_ceil(cell) + 2;
        let n = cols * rows * 3;
        if self
            .data
            .try_reserve(n.saturating_sub(self.data.len()))
            .is_err()
            || self
                .scratch
                .try_reserve(n.saturating_sub(self.scratch.len()))
                .is_err()
        {
            return false;
        }
        self.data.clear();
        self.data.resize(n, 0);
        self.scratch.clear();
        self.scratch.resize(n, 0);
        // Cell (cx, cy) samples a 2×2 grid of pixels in its block; the one-cell
        // border replicates the edge so upsampling never reads outside.
        for cy in 0..rows {
            for cx in 0..cols {
                let bx = (cx.saturating_sub(1) * cell).min(w - 1);
                let by = (cy.saturating_sub(1) * cell).min(h - 1);
                let mut sum = [0u32; 3];
                let mut count = 0;
                for (dx, dy) in [
                    (cell / 4, cell / 4),
                    (3 * cell / 4, cell / 4),
                    (cell / 4, 3 * cell / 4),
                    (3 * cell / 4, 3 * cell / 4),
                ] {
                    let px = (bx + dx).min(w - 1) + x0;
                    let py = (by + dy).min(h - 1) + y0;
                    let at = py * stride + px * 4;
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s += u32::from(scene[at + c]);
                    }
                    count += 1;
                }
                let at = (cy * cols + cx) * 3;
                for (c, s) in sum.iter().enumerate() {
                    self.data[at + c] = (s / count) as u16;
                }
            }
        }
        // Three separable 3-tap box passes approximate a Gaussian of ~1.7
        // cells (≈ the 30 px Windows acrylic blur at 8 px cells).
        for _ in 0..3 {
            for cy in 0..rows {
                for cx in 0..cols {
                    for c in 0..3 {
                        let get = |x: usize| self.data[(cy * cols + x) * 3 + c] as u32;
                        let l = get(cx.saturating_sub(1));
                        let r = get((cx + 1).min(cols - 1));
                        self.scratch[(cy * cols + cx) * 3 + c] =
                            ((l + 2 * get(cx) + r + 2) / 4) as u16;
                    }
                }
            }
            for cy in 0..rows {
                for cx in 0..cols {
                    for c in 0..3 {
                        let get = |y: usize| self.scratch[(y * cols + cx) * 3 + c] as u32;
                        let u = get(cy.saturating_sub(1));
                        let d = get((cy + 1).min(rows - 1));
                        self.data[(cy * cols + cx) * 3 + c] =
                            ((u + 2 * get(cy) + d + 2) / 4) as u16;
                    }
                }
            }
        }
        self.region = Some(region);
        self.cell = cell;
        self.cols = cols;
        self.rows = rows;
        true
    }
    /// Paint the acrylic into logical `rect` (rounded by `radius`), offset by
    /// `dy` logical px relative to where it was sampled (slide animation).
    /// `tint` is opaque BGRA; `opacity` its coverage over the blur.
    pub fn paint(
        &self,
        c: &mut Canvas<'_>,
        rect: Rect,
        radius: u32,
        dy: i32,
        tint: [u8; 4],
        opacity: u8,
    ) {
        let scale = c.scale();
        let Ok(target) = scale.rect(rect) else {
            return;
        };
        let Some(clip) = c.clip_rect().and_then(|clip| clip.intersection(target)) else {
            return;
        };
        let stride = c.physical_dimensions().2;
        let radius = (scale.len(radius) as i32)
            .min(target.width() as i32 / 2)
            .min(target.height() as i32 / 2);
        let ta = u32::from(opacity);
        let tb = 255 - ta;
        let tint_part = [
            u32::from(tint[0]) * ta,
            u32::from(tint[1]) * ta,
            u32::from(tint[2]) * ta,
        ];
        let region = self.region;
        let cell = self.cell.max(1) as i32;
        let pixels = c.pixels_mut();
        let mut row = alloc::vec![0u32; self.cols * 3];
        for y in clip.y()..clip.bottom() {
            // Rounded corner span for this row.
            let ry = if y < target.y() + radius {
                target.y() + radius - y
            } else if y >= target.bottom() - radius {
                y - (target.bottom() - radius - 1)
            } else {
                0
            };
            let inset = if ry > 0 {
                let r2 = radius * radius;
                let dy2 = (ry * ry).min(r2);
                // Horizontal extent of the circle at this row.
                let mut dx = radius;
                while dx > 0 && dx * dx + dy2 > r2 {
                    dx -= 1;
                }
                radius - dx
            } else {
                0
            };
            let left = (target.x() + inset).max(clip.x());
            let right = (target.right() - inset).min(clip.right());
            if right <= left {
                continue;
            }
            let have_blur = if let Some(region) = region {
                // Source row (sampled before the slide offset).
                let sy = y - scale.px(dy) - region.y();
                let fy = (sy * 256 / cell + 128).max(0) as usize; // +1 border cell, centered
                let cy = (fy >> 8).min(self.rows - 2);
                let wy = (fy & 255) as u32;
                for cx in 0..self.cols {
                    for k in 0..3 {
                        let a = self.data[(cy * self.cols + cx) * 3 + k] as u32;
                        let b = self.data[((cy + 1) * self.cols + cx) * 3 + k] as u32;
                        row[cx * 3 + k] = a * (256 - wy) + b * wy;
                    }
                }
                true
            } else {
                false
            };
            let base = y as usize * stride;
            for x in left..right {
                let at = base + x as usize * 4;
                let blur = if have_blur {
                    let region = region.unwrap();
                    let sx = x - region.x();
                    let fx = (sx * 256 / cell + 128).max(0) as usize;
                    let cx = (fx >> 8).min(self.cols - 2);
                    let wx = (fx & 255) as u32;
                    core::array::from_fn::<u32, 3, _>(|k| {
                        (row[cx * 3 + k] * (256 - wx) + row[(cx + 1) * 3 + k] * wx) >> 16
                    })
                } else {
                    [u32::from(tint[0]), u32::from(tint[1]), u32::from(tint[2])]
                };
                for k in 0..3 {
                    pixels[at + k] = ((tint_part[k] + blur[k] * tb + 127) / 255).min(255) as u8;
                }
                pixels[at + 3] = 255;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn acrylic_blurs_and_tints_inside_rounded_bounds_only() {
        let (w, h) = (200usize, 120usize);
        let mut scene = alloc::vec![255u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                if (x / 10 + y / 10) % 2 == 0 {
                    scene[(y * w + x) * 4..(y * w + x) * 4 + 3].fill(0);
                }
            }
        }
        let mut a = Acrylic::default();
        assert!(a.build(&scene, w * 4, Rect::new(20, 10, 160, 100).unwrap(), 8));
        let mut out = alloc::vec![7u8; w * h * 4];
        let mut c = Canvas::new(&mut out, w as u32, h as u32, w * 4).unwrap();
        a.paint(
            &mut c,
            Rect::new(20, 10, 160, 100).unwrap(),
            8,
            0,
            [40, 40, 40, 255],
            200,
        );
        // Corners outside the radius keep the destination.
        assert_eq!(
            &out[(10 * w + 20) * 4..(10 * w + 20) * 4 + 4],
            &[7, 7, 7, 7]
        );
        // Interior is opaque and blurred toward the checkerboard mean.
        let mid = (60 * w + 100) * 4;
        assert_eq!(out[mid + 3], 255);
        let v = out[mid] as i32;
        let expected = (40 * 200 + 128 * 55) / 255;
        assert!((v - expected).abs() < 20, "{v} vs {expected}");
        // Neighbouring pixels are smooth (no checkerboard edges).
        assert!((out[mid] as i32 - out[mid + 4] as i32).abs() <= 3);
    }
}
