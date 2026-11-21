//! Device-independent bitmaps: validation of captured `BITMAPINFO` bytes and
//! conversion between DIB rows and the 32-bpp pixel stores. Supported
//! formats are uncompressed 1, 4, 8, 16 (5-5-5 or `BI_BITFIELDS`), 24 and 32
//! bits per pixel; RLE, JPEG and PNG compressions are rejected explicitly.
use crate::msg::bi;
use crate::surface::{MAX_EDGE, MAX_PIXELS};
use crate::{Error, Result};
use alloc::vec::Vec;

pub const HEADER_BYTES: usize = 40;
pub const V4_BYTES: usize = 108;
pub const V5_BYTES: usize = 124;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DibInfo {
    pub width: u32,
    pub height: u32,
    pub top_down: bool,
    pub bit_count: u16,
    /// (red, green, blue) masks for 16/32 bpp.
    pub masks: (u32, u32, u32),
    /// Colors of the table as `0x00RRGGBB` (1/4/8 bpp).
    pub palette: Vec<u32>,
    /// Bytes from the start of `BITMAPINFO` to the end of the color table.
    pub header_bytes: usize,
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl DibInfo {
    /// Bytes of the fixed header and color table that `parse` will need,
    /// given the first 40 bytes; lets the caller capture exactly that much.
    pub fn needed(head: &[u8]) -> Result<usize> {
        if head.len() < 4 {
            return Err(Error::Malformed);
        }
        let size = u32_at(head, 0) as usize;
        if !matches!(size, HEADER_BYTES | V4_BYTES | V5_BYTES) || head.len() < HEADER_BYTES {
            return Err(Error::Malformed);
        }
        let bits = u16::from_le_bytes([head[14], head[15]]);
        let compression = u32_at(head, 16);
        let used = u32_at(head, 32) as usize;
        let colors = match bits {
            1 | 4 | 8 => {
                let max = 1usize << bits;
                if used == 0 { max } else { used.min(max) }
            }
            _ => 0,
        };
        let masks = if size == HEADER_BYTES && compression == bi::BITFIELDS {
            12
        } else {
            0
        };
        Ok(size + masks + colors * 4)
    }
    /// `usage` is DIB_RGB_COLORS (0) or DIB_PAL_COLORS (1, rejected: no
    /// logical palette is realized on this true-color device).
    pub fn parse(b: &[u8], usage: u32) -> Result<Self> {
        let need = Self::needed(b)?;
        if b.len() < need {
            return Err(Error::Malformed);
        }
        if usage != bi::DIB_RGB_COLORS {
            return Err(Error::Unsupported);
        }
        let size = u32_at(b, 0) as usize;
        let width = u32_at(b, 4) as i32;
        let height = u32_at(b, 8) as i32;
        let planes = u16::from_le_bytes([b[12], b[13]]);
        let bit_count = u16::from_le_bytes([b[14], b[15]]);
        let compression = u32_at(b, 16);
        if planes != 1 || width <= 0 || height == 0 || height == i32::MIN {
            return Err(Error::InvalidParameter);
        }
        let (w, h) = (width as u32, height.unsigned_abs());
        if w > MAX_EDGE || h > MAX_EDGE || u64::from(w) * u64::from(h) > MAX_PIXELS {
            return Err(Error::Quota);
        }
        let masks = match (bit_count, compression) {
            (1 | 4 | 8 | 24, bi::RGB) => (0, 0, 0),
            (16, bi::RGB) => (0x7c00, 0x03e0, 0x001f),
            (32, bi::RGB) => (0x00ff_0000, 0x0000_ff00, 0x0000_00ff),
            (16 | 32, bi::BITFIELDS) => {
                // Masks follow the 40-byte core of every header version.
                let at = HEADER_BYTES;
                let m = (u32_at(b, at), u32_at(b, at + 4), u32_at(b, at + 8));
                if m.0 == 0 || m.1 == 0 || m.2 == 0 {
                    return Err(Error::InvalidParameter);
                }
                m
            }
            (_, bi::RLE8 | bi::RLE4 | bi::JPEG | bi::PNG) => return Err(Error::Unsupported),
            _ => return Err(Error::InvalidParameter),
        };
        let table_at = size
            + if size == HEADER_BYTES && compression == bi::BITFIELDS {
                12
            } else {
                0
            };
        let palette = b[table_at..need]
            .chunks_exact(4)
            .map(|q| u32::from(q[2]) << 16 | u32::from(q[1]) << 8 | u32::from(q[0]))
            .collect();
        Ok(Self {
            width: w,
            height: h,
            top_down: height < 0,
            bit_count,
            masks,
            palette,
            header_bytes: need,
        })
    }
    /// Bytes per row, padded to a DWORD boundary.
    pub fn stride(&self) -> usize {
        (self.width as usize * self.bit_count as usize).div_ceil(32) * 4
    }
    pub fn image_bytes(&self) -> usize {
        self.stride() * self.height as usize
    }
    /// Offset of the bytes of top-down row `y`.
    pub fn row_offset(&self, y: u32) -> usize {
        let stored = if self.top_down {
            y
        } else {
            self.height - 1 - y
        };
        stored as usize * self.stride()
    }
    fn channel(value: u32, mask: u32) -> u32 {
        let shift = mask.trailing_zeros();
        let bits = (mask >> shift).count_ones();
        let v = (value & mask) >> shift;
        if bits >= 8 {
            v >> (bits - 8)
        } else if bits == 0 {
            0
        } else {
            // Replicate the high bits as GDI expands 5-bit channels.
            (v << (8 - bits)) | (v >> (2 * bits).saturating_sub(8).min(bits))
        }
    }
    /// Decode one stored row into `0x00RRGGBB` pixels.
    pub fn decode_row(&self, row: &[u8], out: &mut [u32]) {
        let w = (self.width as usize).min(out.len());
        let pal = |i: usize| self.palette.get(i).copied().unwrap_or(0);
        for (x, o) in out[..w].iter_mut().enumerate() {
            *o = match self.bit_count {
                1 => pal(usize::from(row[x / 8] >> (7 - x % 8) & 1)),
                4 => pal(usize::from(
                    row[x / 2] >> if x % 2 == 0 { 4 } else { 0 } & 15,
                )),
                8 => pal(usize::from(row[x])),
                16 => {
                    let v = u32::from(u16::from_le_bytes([row[2 * x], row[2 * x + 1]]));
                    Self::channel(v, self.masks.0) << 16
                        | Self::channel(v, self.masks.1) << 8
                        | Self::channel(v, self.masks.2)
                }
                24 => {
                    u32::from(row[3 * x + 2]) << 16
                        | u32::from(row[3 * x + 1]) << 8
                        | u32::from(row[3 * x])
                }
                _ => {
                    let v = u32_at(row, 4 * x);
                    if self.masks == (0x00ff_0000, 0x0000_ff00, 0x0000_00ff) {
                        v & 0x00ff_ffff
                    } else {
                        Self::channel(v, self.masks.0) << 16
                            | Self::channel(v, self.masks.1) << 8
                            | Self::channel(v, self.masks.2)
                    }
                }
            };
        }
    }
    fn nearest(&self, pixel: u32) -> u8 {
        let (r, g, b) = (
            (pixel >> 16 & 255) as i32,
            (pixel >> 8 & 255) as i32,
            (pixel & 255) as i32,
        );
        let mut best = (i32::MAX, 0u8);
        for (i, p) in self.palette.iter().enumerate() {
            let (pr, pg, pb) = (
                (p >> 16 & 255) as i32,
                (p >> 8 & 255) as i32,
                (p & 255) as i32,
            );
            let d = (r - pr).pow(2) + (g - pg).pow(2) + (b - pb).pow(2);
            if d < best.0 {
                best = (d, i as u8);
            }
        }
        best.1
    }
    fn encode_channel(v: u32, mask: u32) -> u32 {
        let shift = mask.trailing_zeros();
        let bits = (mask >> shift).count_ones().min(8);
        ((v >> (8 - bits)) << shift) & mask
    }
    /// Encode `0x00RRGGBB` pixels into one stored row.
    pub fn encode_row(&self, px: &[u32], row: &mut [u8]) {
        let w = (self.width as usize).min(px.len());
        for (x, &p) in px[..w].iter().enumerate() {
            match self.bit_count {
                1 => {
                    let bit = self.nearest(p) & 1;
                    let m = 0x80u8 >> (x % 8);
                    row[x / 8] = if bit != 0 {
                        row[x / 8] | m
                    } else {
                        row[x / 8] & !m
                    };
                }
                4 => {
                    let v = self.nearest(p) & 15;
                    row[x / 2] = if x % 2 == 0 {
                        (row[x / 2] & 0x0f) | v << 4
                    } else {
                        (row[x / 2] & 0xf0) | v
                    };
                }
                8 => row[x] = self.nearest(p),
                16 => {
                    let v = Self::encode_channel(p >> 16 & 255, self.masks.0)
                        | Self::encode_channel(p >> 8 & 255, self.masks.1)
                        | Self::encode_channel(p & 255, self.masks.2);
                    row[2 * x..2 * x + 2].copy_from_slice(&(v as u16).to_le_bytes());
                }
                24 => {
                    row[3 * x] = p as u8;
                    row[3 * x + 1] = (p >> 8) as u8;
                    row[3 * x + 2] = (p >> 16) as u8;
                }
                _ => {
                    let v = if self.masks == (0x00ff_0000, 0x0000_ff00, 0x0000_00ff) {
                        p & 0x00ff_ffff
                    } else {
                        Self::encode_channel(p >> 16 & 255, self.masks.0)
                            | Self::encode_channel(p >> 8 & 255, self.masks.1)
                            | Self::encode_channel(p & 255, self.masks.2)
                    };
                    row[4 * x..4 * x + 4].copy_from_slice(&v.to_le_bytes());
                }
            }
        }
    }
}

/// `BITMAPINFOHEADER` for a top-down 32-bpp image (`GetDIBits` replies).
pub fn header32(width: u32, height: u32, top_down: bool) -> [u8; HEADER_BYTES] {
    let mut h = [0u8; HEADER_BYTES];
    h[..4].copy_from_slice(&(HEADER_BYTES as u32).to_le_bytes());
    h[4..8].copy_from_slice(&width.to_le_bytes());
    let height = if top_down {
        -(height as i32)
    } else {
        height as i32
    };
    h[8..12].copy_from_slice(&height.to_le_bytes());
    h[12..14].copy_from_slice(&1u16.to_le_bytes());
    h[14..16].copy_from_slice(&32u16.to_le_bytes());
    h[20..24].copy_from_slice(&(width * 4 * height.unsigned_abs()).to_le_bytes());
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    fn header(w: i32, h: i32, bits: u16, compression: u32, used: u32) -> Vec<u8> {
        let mut b = alloc::vec![0u8; 40];
        b[..4].copy_from_slice(&40u32.to_le_bytes());
        b[4..8].copy_from_slice(&w.to_le_bytes());
        b[8..12].copy_from_slice(&h.to_le_bytes());
        b[12..14].copy_from_slice(&1u16.to_le_bytes());
        b[14..16].copy_from_slice(&bits.to_le_bytes());
        b[16..20].copy_from_slice(&compression.to_le_bytes());
        b[32..36].copy_from_slice(&used.to_le_bytes());
        b
    }
    #[test]
    fn top_down_32bpp_rows() {
        let b = header(16, -16, 32, bi::RGB, 0);
        let info = DibInfo::parse(&b, 0).unwrap();
        assert!(info.top_down);
        assert_eq!((info.stride(), info.image_bytes()), (64, 1024));
        assert_eq!(info.row_offset(1), 64);
        let up = DibInfo::parse(&header(3, 2, 24, bi::RGB, 0), 0).unwrap();
        assert_eq!(up.stride(), 12);
        assert_eq!(up.row_offset(0), 12, "bottom-up");
        let mut row = [0u8; 12];
        up.encode_row(&[0x0011_2233, 0x0044_5566, 0x0077_8899], &mut row);
        let mut back = [0u32; 3];
        up.decode_row(&row, &mut back);
        assert_eq!(back, [0x0011_2233, 0x0044_5566, 0x0077_8899]);
    }
    #[test]
    fn palettes_and_bitfields() {
        let mut b = header(9, 1, 1, bi::RGB, 2);
        b.extend_from_slice(&[0, 0, 0, 0, 0xff, 0xff, 0xff, 0]);
        assert_eq!(DibInfo::needed(&b), Ok(48));
        let info = DibInfo::parse(&b, 0).unwrap();
        let mut row = [0u8; 4];
        info.encode_row(&[0x00ff_ffff, 0, 0x00f0_f0f0], &mut row);
        assert_eq!(row[0], 0b1010_0000);
        let mut px = [7u32; 3];
        info.decode_row(&row, &mut px);
        assert_eq!(px, [0x00ff_ffff, 0, 0x00ff_ffff]);
        let mut f = header(1, 1, 16, bi::BITFIELDS, 0);
        f.extend_from_slice(&0xf800u32.to_le_bytes());
        f.extend_from_slice(&0x07e0u32.to_le_bytes());
        f.extend_from_slice(&0x001fu32.to_le_bytes());
        let info = DibInfo::parse(&f, 0).unwrap();
        let mut px = [0u32];
        info.decode_row(&0xffffu16.to_le_bytes(), &mut px);
        assert_eq!(px[0], 0x00ff_ffff);
    }
    #[test]
    fn hostile_headers_are_rejected() {
        assert!(DibInfo::parse(&header(0, 1, 32, 0, 0), 0).is_err());
        assert!(DibInfo::parse(&header(1, 0, 32, 0, 0), 0).is_err());
        assert!(DibInfo::parse(&header(1, i32::MIN, 32, 0, 0), 0).is_err());
        assert_eq!(
            DibInfo::parse(&header(100_000, 1, 32, 0, 0), 0),
            Err(Error::Quota)
        );
        assert_eq!(
            DibInfo::parse(
                &[header(1, 1, 8, bi::RLE8, 1), alloc::vec![0; 4]].concat(),
                0
            ),
            Err(Error::Unsupported)
        );
        assert!(DibInfo::parse(&header(1, 1, 7, 0, 0), 0).is_err());
        assert!(
            DibInfo::parse(&header(1, 1, 8, 0, 0), 0).is_err(),
            "table missing"
        );
        assert_eq!(
            DibInfo::parse(&header(1, 1, 32, 0, 0), 1),
            Err(Error::Unsupported)
        );
        let mut bad = header(1, 1, 32, 0, 0);
        bad[0] = 41;
        assert!(DibInfo::needed(&bad).is_err());
        assert!(DibInfo::needed(&[40, 0]).is_err());
        let h = header32(2, 3, true);
        assert_eq!(DibInfo::parse(&h, 0).unwrap().height, 3);
    }
}
