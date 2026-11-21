use crate::{Error, Result};
pub const MAX_WIDTH: u32 = 3840;
pub const MAX_HEIGHT: u32 = 2160;
pub const MAX_SURFACE_BYTES: usize = MAX_WIDTH as usize * MAX_HEIGHT as usize * 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    PremultipliedBgra8888,
}

/// Immutable validated metadata. Padding is allowed, but is charged to quotas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceDescriptor {
    pub width: u32,
    pub height: u32,
    pub stride_bytes: usize,
    pub backing_bytes: usize,
    pub format: PixelFormat,
}
impl SurfaceDescriptor {
    pub fn required_bytes(self) -> Result<usize> {
        self.validate()?;
        self.stride_bytes
            .checked_mul(self.height as usize)
            .ok_or(Error::InvalidSurface)
    }
    pub fn new(width: u32, height: u32, stride_bytes: usize, backing_bytes: usize) -> Result<Self> {
        let value = Self {
            width,
            height,
            stride_bytes,
            backing_bytes,
            format: PixelFormat::PremultipliedBgra8888,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(self) -> Result<()> {
        let row = (self.width as usize)
            .checked_mul(4)
            .ok_or(Error::InvalidSurface)?;
        let size = self
            .stride_bytes
            .checked_mul(self.height as usize)
            .ok_or(Error::InvalidSurface)?;
        if self.width == 0
            || self.height == 0
            || self.width > MAX_WIDTH
            || self.height > MAX_HEIGHT
            || self.stride_bytes < row
            || !self.stride_bytes.is_multiple_of(4)
            || size > self.backing_bytes
            || self.backing_bytes > MAX_SURFACE_BYTES
        {
            return Err(Error::InvalidSurface);
        }
        Ok(())
    }
    pub(crate) fn validate_pixels(self, pixels: &[u8]) -> Result<bool> {
        self.validate()?;
        if pixels.len() != self.backing_bytes {
            return Err(Error::InvalidSurface);
        }
        let mut opaque = true;
        for y in 0..self.height as usize {
            for p in pixels[y * self.stride_bytes..y * self.stride_bytes + self.width as usize * 4]
                .chunks_exact(4)
            {
                if p[0] > p[3] || p[1] > p[3] || p[2] > p[3] {
                    return Err(Error::InvalidPixels);
                }
                opaque &= p[3] == 255;
            }
        }
        Ok(opaque)
    }
    /// Per-row translucency of already validated pixels: opaque rows of a
    /// partly translucent frame keep the compositor's row-copy path.
    pub(crate) fn translucent_rows(self, pixels: &[u8]) -> alloc::vec::Vec<bool> {
        let bytes = self.width as usize * 4;
        (0..self.height as usize)
            .map(|y| {
                let at = y * self.stride_bytes;
                pixels[at..at + bytes].chunks_exact(4).any(|p| p[3] != 255)
            })
            .collect()
    }
    /// Validates whole stride-sized rows starting at `top`; returns whether
    /// every visible pixel in them is opaque.
    pub(crate) fn validate_band(self, top: u32, band: &[u8]) -> Result<bool> {
        self.validate()?;
        let rows = band.len() / self.stride_bytes;
        if rows == 0
            || !band.len().is_multiple_of(self.stride_bytes)
            || top as usize + rows > self.height as usize
        {
            return Err(Error::InvalidSurface);
        }
        let mut opaque = true;
        for row in band.chunks_exact(self.stride_bytes) {
            for p in row[..self.width as usize * 4].chunks_exact(4) {
                if p[0] > p[3] || p[1] > p[3] || p[2] > p[3] {
                    return Err(Error::InvalidPixels);
                }
                opaque &= p[3] == 255;
            }
        }
        Ok(opaque)
    }
    /// Exact changed-pixel bounds between already validated, equal descriptors.
    /// Padding is storage, not visible pixels. No hash or producer damage claim
    /// may suppress a changed pixel in the trusted snapshot.
    pub(crate) fn changed_bounds(self, old: &[u8], new: &[u8]) -> Option<crate::Rect> {
        let mut left = self.width as usize;
        let mut right = 0;
        let mut top = self.height as usize;
        let mut bottom = 0;
        for y in 0..self.height as usize {
            let at = y * self.stride_bytes;
            let bytes = self.width as usize * 4;
            let a = &old[at..at + bytes];
            let b = &new[at..at + bytes];
            if a == b {
                continue;
            }
            let first = a
                .chunks_exact(4)
                .zip(b.chunks_exact(4))
                .position(|(a, b)| a != b)
                .unwrap();
            let last = a
                .chunks_exact(4)
                .zip(b.chunks_exact(4))
                .rposition(|(a, b)| a != b)
                .unwrap();
            left = left.min(first);
            right = right.max(last + 1);
            top = top.min(y);
            bottom = y + 1;
        }
        (right > left).then(|| {
            crate::Rect::new(
                left as i32,
                top as i32,
                (right - left) as u32,
                (bottom - top) as u32,
            )
            .unwrap()
        })
    }
}
