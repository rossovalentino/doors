//! Private 32-bpp pixel stores. Pixels are `0x00RRGGBB` (BGRX in memory, the
//! GDI 32-bpp DIB layout) in top-down rows with `stride == width`. A store is
//! always owned by the DLL that created it; application-visible DIB-section
//! memory is never represented by this type (gdi32 moves rows in and out of
//! such memory with raw checked copies).
use crate::{Error, Rect, Result, try_zeroed};
use alloc::vec::Vec;

/// Largest supported bitmap edge. GDI accepts larger values on some devices,
/// but this profile bounds every surface explicitly.
pub const MAX_EDGE: u32 = 16_384;
/// Largest single surface (64 MiB of pixels).
pub const MAX_PIXELS: u64 = 16 * 1024 * 1024;
const WHITE: u32 = 0x00ff_ffff;

/// The black-or-white pixel a monochrome surface realizes for `pixel`: the
/// nearer of the two by RGB distance (white when `r + g + b > 382`), as GDI
/// matches a color against the two-entry palette of a 1-bpp DDB.
pub const fn mono_pixel(pixel: u32) -> u32 {
    let sum = (pixel >> 16 & 0xff) + (pixel >> 8 & 0xff) + (pixel & 0xff);
    if sum > 382 { WHITE } else { 0 }
}

/// `bmWidthBytes` of a device-dependent bitmap: rows are WORD-aligned.
pub fn ddb_stride(width: u32, bits_pixel: u16) -> usize {
    (width as usize * usize::from(bits_pixel)).div_ceil(16) * 2
}

/// Map a block between a monochrome and a color DC as BitBlt does: a mono
/// source paints its 0 bits in the destination's text color and its 1 bits
/// in the destination's background color; a color source becomes white
/// (1) exactly where it equals the source's background color, black
/// elsewhere. `text`/`background` are `0x00RRGGBB` pixels of the DC named.
pub fn convert_depth(px: &mut [u32], from_mono: bool, to_mono: bool, text: u32, background: u32) {
    match (from_mono, to_mono) {
        (true, false) => {
            for p in px {
                *p = if *p & WHITE == WHITE {
                    background & WHITE
                } else {
                    text & WHITE
                };
            }
        }
        (false, true) => {
            for p in px {
                *p = if *p & WHITE == background & WHITE {
                    WHITE
                } else {
                    0
                };
            }
        }
        _ => (),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Surface {
    width: u32,
    height: u32,
    /// Monochrome device-dependent bitmaps store only 0 (black) or 0x00ffffff
    /// (white); conversions to/from color follow the GDI text/background rules.
    mono: bool,
    pixels: Vec<u32>,
}

impl Surface {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        Self::with_format(width, height, false)
    }
    pub fn new_mono(width: u32, height: u32) -> Result<Self> {
        Self::with_format(width, height, true)
    }
    fn with_format(width: u32, height: u32, mono: bool) -> Result<Self> {
        let count = Self::checked_count(width, height)?;
        Ok(Self {
            width,
            height,
            mono,
            pixels: try_zeroed(count)?,
        })
    }
    /// `CreateBitmap` store: 1 bpp (monochrome, most significant bit first,
    /// a set bit is white) or 32 bpp (BGRX), top-down WORD-aligned rows of
    /// `ddb_stride` bytes. Without `bits` the bitmap starts black.
    pub fn from_ddb(width: u32, height: u32, bits_pixel: u16, bits: Option<&[u8]>) -> Result<Self> {
        let mono = match bits_pixel {
            1 => true,
            32 => false,
            _ => return Err(Error::Unsupported),
        };
        let mut s = Self::with_format(width, height, mono)?;
        let Some(bits) = bits else {
            return Ok(s);
        };
        let stride = ddb_stride(width, bits_pixel);
        if bits.len() < stride * height as usize {
            return Err(Error::InvalidParameter);
        }
        for (y, row) in bits.chunks_exact(stride).take(height as usize).enumerate() {
            let out = s.row_mut(y as u32);
            for (x, o) in out.iter_mut().enumerate() {
                *o = if mono {
                    if row[x / 8] >> (7 - x % 8) & 1 != 0 {
                        WHITE
                    } else {
                        0
                    }
                } else {
                    u32::from_le_bytes([row[4 * x], row[4 * x + 1], row[4 * x + 2], 0])
                };
            }
        }
        Ok(s)
    }
    /// Validate dimensions; zero-sized surfaces are rejected (GDI creates a
    /// 1x1 stock monochrome bitmap instead, which callers model explicitly).
    pub fn checked_count(width: u32, height: u32) -> Result<usize> {
        if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE {
            return Err(Error::InvalidParameter);
        }
        let count = u64::from(width) * u64::from(height);
        if count > MAX_PIXELS {
            return Err(Error::Quota);
        }
        Ok(count as usize)
    }
    pub fn bytes(&self) -> u64 {
        self.pixels.len() as u64 * 4
    }
    pub const fn width(&self) -> u32 {
        self.width
    }
    pub const fn height(&self) -> u32 {
        self.height
    }
    pub const fn is_mono(&self) -> bool {
        self.mono
    }
    pub fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width as i32, self.height as i32)
    }
    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }
    pub fn pixels_mut(&mut self) -> &mut [u32] {
        &mut self.pixels
    }
    pub fn row(&self, y: u32) -> &[u32] {
        let w = self.width as usize;
        &self.pixels[y as usize * w..(y as usize + 1) * w]
    }
    pub fn row_mut(&mut self, y: u32) -> &mut [u32] {
        let w = self.width as usize;
        &mut self.pixels[y as usize * w..(y as usize + 1) * w]
    }
    pub fn get(&self, x: i32, y: i32) -> Option<u32> {
        (x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height)
            .then(|| self.pixels[y as usize * self.width as usize + x as usize])
    }
    /// Raw store without clipping or ROP; callers clip first.
    pub fn set(&mut self, x: i32, y: i32, pixel: u32) {
        if x >= 0 && y >= 0 && (x as u32) < self.width && (y as u32) < self.height {
            let p = self.normalize(pixel);
            self.pixels[y as usize * self.width as usize + x as usize] = p;
        }
    }
    /// Monochrome stores keep only black/white (`mono_pixel`).
    pub fn normalize(&self, pixel: u32) -> u32 {
        if self.mono {
            mono_pixel(pixel)
        } else {
            pixel & WHITE
        }
    }
    pub fn fill(&mut self, pixel: u32) {
        let p = self.normalize(pixel);
        self.pixels.fill(p);
    }
    /// Resize preserving the top-left overlap; new pixels are `fill`.
    pub fn resize(&mut self, width: u32, height: u32, fill: u32) -> Result<()> {
        let count = Self::checked_count(width, height)?;
        let mut next: Vec<u32> = try_zeroed(count)?;
        let fill = self.normalize(fill);
        next.fill(fill);
        let w = self.width.min(width) as usize;
        for y in 0..self.height.min(height) as usize {
            let src = &self.pixels[y * self.width as usize..y * self.width as usize + w];
            next[y * width as usize..y * width as usize + w].copy_from_slice(src);
        }
        self.pixels = next;
        self.width = width;
        self.height = height;
        Ok(())
    }
    /// Copy rows `[top, top + rows)` as little-endian BGRA bytes with alpha
    /// forced to 0xff (window surfaces are committed opaque).
    pub fn rows_opaque_bgra(&self, top: u32, rows: u32, out: &mut [u8]) -> Result<()> {
        let end = top.checked_add(rows).ok_or(Error::InvalidParameter)?;
        let need = rows as usize * self.width as usize * 4;
        if end > self.height || out.len() < need {
            return Err(Error::InvalidParameter);
        }
        let w = self.width as usize;
        for (i, p) in self.pixels[top as usize * w..end as usize * w]
            .iter()
            .enumerate()
        {
            out[i * 4..i * 4 + 4].copy_from_slice(&(p | 0xff00_0000).to_le_bytes());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn surfaces_are_bounded_and_mono_normalizes() {
        assert_eq!(Surface::new(0, 1), Err(Error::InvalidParameter));
        assert_eq!(Surface::new(MAX_EDGE + 1, 1), Err(Error::InvalidParameter));
        assert_eq!(Surface::new(MAX_EDGE, MAX_EDGE), Err(Error::Quota));
        let mut m = Surface::new_mono(3, 1).unwrap();
        m.set(0, 0, 0x00ff_ffff);
        m.set(1, 0, 0x007f_7f7f);
        m.set(2, 0, 0x0080_8080);
        assert_eq!(m.pixels(), &[0x00ff_ffff, 0, 0x00ff_ffff]);
        let mut s = Surface::new(2, 2).unwrap();
        s.set(1, 1, 0xff12_3456);
        assert_eq!(s.get(1, 1), Some(0x0012_3456));
        assert_eq!(s.get(2, 0), None);
        s.resize(3, 1, 7).unwrap();
        assert_eq!(s.pixels(), &[0, 0, 7]);
        let mut out = [0u8; 12];
        s.rows_opaque_bgra(0, 1, &mut out).unwrap();
        assert_eq!(&out[8..], &[7, 0, 0, 0xff]);
        assert!(s.rows_opaque_bgra(1, 1, &mut out).is_err());
    }
    #[test]
    fn ddb_bits_strides_and_depth_conversion() {
        assert_eq!(ddb_stride(1, 1), 2);
        assert_eq!(ddb_stride(17, 1), 4);
        assert_eq!(ddb_stride(3, 32), 12);
        // 10x2 mono: rows of 2 bytes; bit 1 is white.
        let bits = [0b1010_0000, 0b1100_0000, 0x00, 0b0100_0000];
        let m = Surface::from_ddb(10, 2, 1, Some(&bits)).unwrap();
        assert!(m.is_mono());
        let w = 0x00ff_ffff;
        assert_eq!(m.row(0), &[w, 0, w, 0, 0, 0, 0, 0, w, w]);
        assert_eq!(m.row(1), &[0, 0, 0, 0, 0, 0, 0, 0, 0, w]);
        assert_eq!(
            Surface::from_ddb(10, 2, 1, Some(&bits[..3])),
            Err(Error::InvalidParameter)
        );
        let c = Surface::from_ddb(1, 1, 32, Some(&[0x56, 0x34, 0x12, 0xff])).unwrap();
        assert_eq!(c.pixels(), &[0x0012_3456]);
        assert_eq!(Surface::from_ddb(1, 1, 8, None), Err(Error::Unsupported));
        assert_eq!(Surface::from_ddb(4, 4, 1, None).unwrap().pixels(), &[0; 16]);
        // Mono -> color: 0 takes the text color, 1 the background color.
        let mut px = [0, w];
        convert_depth(&mut px, true, false, 0x0011_2233, 0x0044_5566);
        assert_eq!(px, [0x0011_2233, 0x0044_5566]);
        // Color -> mono: the source background color becomes white.
        let mut px = [0x0044_5566, 0x0044_5567, w, 0];
        convert_depth(&mut px, false, true, 0, 0x0044_5566);
        assert_eq!(px, [w, 0, 0, 0]);
        let mut same = [5, 6];
        convert_depth(&mut same, true, true, 1, 2);
        convert_depth(&mut same, false, false, 1, 2);
        assert_eq!(same, [5, 6]);
        assert_eq!(mono_pixel(0x0080_7f80), w);
        assert_eq!(mono_pixel(0x007f_7f80), 0);
    }
}
