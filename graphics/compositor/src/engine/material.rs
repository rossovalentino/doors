//! Wallpaper material owned by the trusted compositor. No per-frame blur.
use super::*;
pub(crate) struct Backdrop {
    width: usize,
    height: usize,
    pixels: Vec<[u8; 4]>,
}
impl Backdrop {
    pub fn new(source: &[u8], width: u32, height: u32) -> Result<Self> {
        if source.len() != width as usize * height as usize * 4
            || source.chunks_exact(4).any(|p| p[3] != 255)
        {
            return Err(Error::InvalidGeometry);
        }
        let (w, h) = (width.div_ceil(16) as usize, height.div_ceil(16) as usize);
        let mut pixels = vec![[0, 0, 0, 255]; w * h];
        for gy in 0..h {
            for gx in 0..w {
                let mut sum = [0u32; 3];
                let mut count = 0;
                for y in gy * 16..((gy + 1) * 16).min(height as usize) {
                    for x in gx * 16..((gx + 1) * 16).min(width as usize) {
                        let at = (y * width as usize + x) * 4;
                        for c in 0..3 {
                            sum[c] += u32::from(source[at + c]);
                        }
                        count += 1;
                    }
                }
                for c in 0..3 {
                    pixels[gy * w + gx][c] = ((sum[c] + count / 2) / count) as u8;
                }
            }
        }
        // A separable five-tap box on area-prefiltered wallpaper gives a broad
        // frosted material. Both temporary grids are bounded below256KiB at4K.
        let mut scratch = pixels.clone();
        for y in 0..h {
            for x in 0..w {
                for c in 0..3 {
                    scratch[y * w + x][c] = ((-2..=2)
                        .map(|d| {
                            u32::from(
                                pixels[y * w + (x as i32 + d).clamp(0, w as i32 - 1) as usize][c],
                            )
                        })
                        .sum::<u32>()
                        / 5) as u8;
                }
            }
        }
        for y in 0..h {
            for x in 0..w {
                for c in 0..3 {
                    pixels[y * w + x][c] = ((-2..=2)
                        .map(|d| {
                            u32::from(
                                scratch[(y as i32 + d).clamp(0, h as i32 - 1) as usize * w + x][c],
                            )
                        })
                        .sum::<u32>()
                        / 5) as u8;
                }
            }
        }
        Ok(Self {
            width: w,
            height: h,
            pixels,
        })
    }
    /// Mica: this wallpaper grid blurred further (Mica samples the desktop
    /// background at very low frequency) under the theme's luminosity tint,
    /// computed once per wallpaper and theme. WinUI MicaBackdrop defaults:
    /// dark tint #202020 at 80 %, light #F3F3F3 at 50 %, luminosity opacity
    /// 1 (the wallpaper keeps its hue and saturation at the tint's
    /// luminosity). Over the owner's purple wallpaper this gives the
    /// measured Settings page gradient (`reports/artifacts/reference-26h2/settings-
    /// display.png`: #1D2124 to #271B27).
    pub(crate) fn mica(&self, theme: Theme) -> Self {
        let (w, h) = (self.width, self.height);
        let mut pixels = self.pixels.clone();
        let mut scratch = pixels.clone();
        // Three passes of a 9-cell (144 px) box per axis: about a 150 px
        // Gaussian on the already blurred 16 px grid.
        for _ in 0..3 {
            for y in 0..h {
                for x in 0..w {
                    for c in 0..3 {
                        let sum: u32 = (-4..=4)
                            .map(|d| {
                                let sx = (x as i32 + d).clamp(0, w as i32 - 1) as usize;
                                u32::from(pixels[y * w + sx][c])
                            })
                            .sum();
                        scratch[y * w + x][c] = ((sum + 4) / 9) as u8;
                    }
                }
            }
            for y in 0..h {
                for x in 0..w {
                    for c in 0..3 {
                        let sum: u32 = (-4..=4)
                            .map(|d| {
                                let sy = (y as i32 + d).clamp(0, h as i32 - 1) as usize;
                                u32::from(scratch[sy * w + x][c])
                            })
                            .sum();
                        pixels[y * w + x][c] = ((sum + 4) / 9) as u8;
                    }
                }
            }
        }
        let (tint, opacity) = mica_tint(theme);
        for p in &mut pixels {
            *p = mica_pixel(*p, tint, opacity);
        }
        Self {
            width: w,
            height: h,
            pixels,
        }
    }
    /// Vertically interpolated grid row for output row `y` (16 x the value),
    /// so a row of samples costs one horizontal lerp each; identical to
    /// `pixel` because the bilinear weights are separable integers.
    pub(crate) fn row(&self, y: i32, cells: &mut [[u16; 3]]) -> Option<render::RowBase> {
        if self.width > cells.len() {
            return None;
        }
        let sy = (y - 8).max(0) as usize;
        let y0 = (sy / 16).min(self.height - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let fy = (sy % 16) as u16;
        for (gx, cell) in cells[..self.width].iter_mut().enumerate() {
            let (a, b) = (
                self.pixels[y0 * self.width + gx],
                self.pixels[y1 * self.width + gx],
            );
            *cell = core::array::from_fn(|c| u16::from(a[c]) * (16 - fy) + u16::from(b[c]) * fy);
        }
        Some(render::RowBase {
            origin: 0,
            half: 8,
            block: 16,
            cells: self.width,
        })
    }
    pub fn pixel(&self, x: i32, y: i32) -> [u8; 4] {
        let sx = (x - 8).max(0) as usize;
        let sy = (y - 8).max(0) as usize;
        let (x0, y0) = (
            (sx / 16).min(self.width - 1),
            (sy / 16).min(self.height - 1),
        );
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (sx % 16, sy % 16);
        let mut result = [0, 0, 0, 255];
        for (c, component) in result[..3].iter_mut().enumerate() {
            let a = usize::from(self.pixels[y0 * self.width + x0][c]) * (16 - fx)
                + usize::from(self.pixels[y0 * self.width + x1][c]) * fx;
            let b = usize::from(self.pixels[y1 * self.width + x0][c]) * (16 - fx)
                + usize::from(self.pixels[y1 * self.width + x1][c]) * fx;
            *component = ((a * (16 - fy) + b * fy + 128) / 256) as u8;
        }
        result
    }
}
/// Mica tint (BGRA) and tint opacity (/255) of `theme` (WinUI defaults).
pub(crate) const fn mica_tint(theme: Theme) -> ([u8; 4], u32) {
    match theme {
        Theme::Dark => ([0x20, 0x20, 0x20, 255], 204),
        Theme::Light => ([0xf3, 0xf3, 0xf3, 255], 128),
    }
}
/// BT.709 luma of a BGRA pixel, 0..=255.
fn luma(p: [u8; 4]) -> i32 {
    (19 * i32::from(p[0]) + 183 * i32::from(p[1]) + 54 * i32::from(p[2]) + 128) >> 8
}
/// One Mica sample: the backdrop moved to the tint's luminosity (hue and
/// saturation kept), then the tint over it at `opacity`.
pub(crate) fn mica_pixel(backdrop: [u8; 4], tint: [u8; 4], opacity: u32) -> [u8; 4] {
    let shift = luma(tint) - luma(backdrop);
    let mut out = [0, 0, 0, 255];
    for c in 0..3 {
        let moved = (i32::from(backdrop[c]) + shift).clamp(0, 255) as u32;
        out[c] = ((u32::from(tint[c]) * opacity + moved * (255 - opacity) + 127) / 255) as u8;
    }
    out
}
