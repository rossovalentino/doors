use crate::{Error, Result};

/// Half-open physical-pixel clip. Arithmetic uses widened coordinates, including
/// negative and completely offscreen requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clip {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl Clip {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    pub(crate) fn contains(self, x: i64, y: i64) -> bool {
        x >= self.x as i64
            && y >= self.y as i64
            && x < self.x as i64 + self.width as i64
            && y < self.y as i64 + self.height as i64
    }
}
/// A private premultiplied BGRA8888 destination. This must not borrow concurrent
/// client mappings; use a client-owned backbuffer or compositor-owned frame.
pub struct Target<'a> {
    pixels: &'a mut [u8],
    width: u32,
    height: u32,
    stride: usize,
    /// Grayscale antialiasing only (no LCD subpixel coverage): the default.
    grayscale: bool,
}
impl<'a> Target<'a> {
    pub fn new(pixels: &'a mut [u8], width: u32, height: u32, stride_bytes: usize) -> Result<Self> {
        let row = (width as usize)
            .checked_mul(4)
            .ok_or(Error::InvalidTarget)?;
        let extent = stride_bytes
            .checked_mul(height as usize)
            .ok_or(Error::InvalidTarget)?;
        if width == 0
            || height == 0
            || stride_bytes < row
            || !stride_bytes.is_multiple_of(4)
            || extent > pixels.len()
        {
            return Err(Error::InvalidTarget);
        }
        Ok(Self {
            pixels,
            width,
            height,
            stride: stride_bytes,
            grayscale: true,
        })
    }
    /// Grayscale antialiasing (the default): Windows 11 draws the shell and
    /// modern (XAML/WinUI) apps with grayscale coverage, never ClearType, on
    /// opaque and translucent surfaces alike (`reports/artifacts/reference-26h2`:
    /// no colour-fringed text pixels in Settings, Start or the Explorer
    /// command bar).
    pub fn grayscale(mut self) -> Self {
        self.grayscale = true;
        self
    }
    /// ClearType-style RGB-stripe LCD coverage on opaque pixels, as Windows
    /// keeps for Win32 (GDI/DirectUI) surfaces: classic menus and dialogs,
    /// the Explorer navigation tree and items view, the desktop icon labels.
    /// Translucent pixels and text still fall back to grayscale.
    pub fn cleartype(mut self) -> Self {
        self.grayscale = false;
        self
    }
    pub fn bounds(&self) -> Clip {
        Clip::new(0, 0, self.width, self.height)
    }
    /// RGB-stripe LCD coverage on opaque targets. Alpha surfaces and translucent
    /// text require one grayscale alpha so later composition remains valid.
    pub(crate) fn blend_lcd(
        &mut self,
        clip: Clip,
        x: i64,
        y: i64,
        color: [u8; 4],
        rgb: [u8; 3],
    ) -> bool {
        if rgb == [0; 3]
            || x < 0
            || y < 0
            || x >= self.width as i64
            || y >= self.height as i64
            || !clip.contains(x, y)
        {
            return false;
        }
        let at = y as usize * self.stride + x as usize * 4;
        if self.grayscale {
            // The centre subpixel's filtered coverage: centred on the pixel,
            // the same ink mass as a box-filtered grayscale raster, and less
            // horizontal blur than averaging the three filtered subpixels.
            return self.blend(clip, x, y, color, rgb[1]);
        }
        if self.pixels[at + 3] != 255 || color[3] != 255 {
            let gray = ((u32::from(rgb[0]) + u32::from(rgb[1]) + u32::from(rgb[2]) + 1) / 3) as u8;
            return self.blend(clip, x, y, color, gray);
        }
        for c in 0..3 {
            let a = u32::from(rgb[2 - c]);
            self.pixels[at + c] = ((u32::from(color[c]) * a
                + u32::from(self.pixels[at + c]) * (255 - a)
                + 127)
                / 255) as u8;
        }
        true
    }
    pub(crate) fn blend(
        &mut self,
        clip: Clip,
        x: i64,
        y: i64,
        color: [u8; 4],
        coverage: u8,
    ) -> bool {
        if coverage == 0
            || x < 0
            || y < 0
            || x >= self.width as i64
            || y >= self.height as i64
            || !clip.contains(x, y)
        {
            return false;
        }
        let offset = y as usize * self.stride + x as usize * 4;
        let source = color.map(|c| ((c as u32 * coverage as u32 + 127) / 255) as u8);
        let inverse = 255 - source[3] as u32;
        for (index, src) in source.iter().enumerate() {
            self.pixels[offset + index] = (*src as u32
                + (self.pixels[offset + index] as u32 * inverse + 127) / 255)
                .min(255) as u8;
        }
        true
    }
    /// Source-over of one premultiplied BGRA pixel (colour glyphs), scaled by
    /// the text alpha.
    pub(crate) fn blend_premultiplied(
        &mut self,
        clip: Clip,
        x: i64,
        y: i64,
        pixel: [u8; 4],
        alpha: u8,
    ) -> bool {
        if pixel[3] == 0
            || alpha == 0
            || x < 0
            || y < 0
            || x >= self.width as i64
            || y >= self.height as i64
            || !clip.contains(x, y)
        {
            return false;
        }
        let offset = y as usize * self.stride + x as usize * 4;
        let source = pixel.map(|c| ((u32::from(c) * u32::from(alpha) + 127) / 255) as u8);
        let inverse = 255 - u32::from(source[3]);
        for (index, src) in source.iter().enumerate() {
            self.pixels[offset + index] = (u32::from(*src)
                + (u32::from(self.pixels[offset + index]) * inverse + 127) / 255)
                .min(255) as u8;
        }
        true
    }
}
