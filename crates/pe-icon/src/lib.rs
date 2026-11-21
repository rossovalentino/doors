//! Windows icons. A program's icon groups as File Explorer, User Account
//! Control and ExtractIcon read them (`RT_GROUP_ICON`/`RT_ICON` of the PE
//! resource directory, by index or id), `.ico`/`.cur` files, the frame a
//! size calls for, and their pixels.
//!
//! Reads go through `Source`, which may answer "fetch these bytes first"
//! (`Need`), so a caller can read a large program in bounded ranges. Every
//! offset and count is checked; a malformed file yields no icon, never a
//! panic. DIB frames (1, 4, 8, 24 and 32 bits with the AND mask) decode to
//! premultiplied BGRA; PNG frames (Vista and later, usually 256 px) decode
//! with the `png` feature and Doors' QOI assets with `qoi`.
#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;
use alloc::vec::Vec;

const RT_ICON: u32 = 3;
const RT_GROUP_ICON: u32 = 14;
/// Resource tables longer than this are malformed for this purpose.
const MAX_ENTRIES: usize = 512;
/// A frame's payload is at most this large (a 256 px 32-bit DIB is 263 KiB).
pub const MAX_FRAME: u64 = 512 * 1024;

/// Bytes the source does not hold yet: fetch `len` bytes at `offset`, then
/// ask again. `None` offset means the file is malformed or truncated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Miss {
    Need(u64, u64),
    Malformed,
}
pub trait Source {
    fn size(&self) -> u64;
    fn bytes(&self, offset: u64, len: u64) -> Result<&[u8], Miss>;
}
impl Source for [u8] {
    fn size(&self) -> u64 {
        self.len() as u64
    }
    fn bytes(&self, offset: u64, len: u64) -> Result<&[u8], Miss> {
        let start = usize::try_from(offset).map_err(|_| Miss::Malformed)?;
        let end = start
            .checked_add(usize::try_from(len).map_err(|_| Miss::Malformed)?)
            .ok_or(Miss::Malformed)?;
        self.get(start..end).ok_or(Miss::Malformed)
    }
}
fn u16_at<S: Source + ?Sized>(s: &S, at: u64) -> Result<u32, Miss> {
    let b = s.bytes(at, 2)?;
    Ok(u32::from(u16::from_le_bytes([b[0], b[1]])))
}
fn u32_at<S: Source + ?Sized>(s: &S, at: u64) -> Result<u32, Miss> {
    let b = s.bytes(at, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// One frame of an icon group (`GRPICONDIRENTRY`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Pixels (a stored 0 means 256).
    pub width: u32,
    pub height: u32,
    pub bits: u32,
    /// The `RT_ICON` resource id holding the frame.
    pub id: u32,
}
/// The resource directory: its file offset and the section map turning
/// data RVAs into file offsets.
struct Resources {
    base: u64,
    sections: Vec<(u64, u64, u64)>,
}
impl Resources {
    fn locate<S: Source + ?Sized>(s: &S) -> Result<Option<Self>, Miss> {
        if s.size() < 0x40 || s.bytes(0, 2)? != b"MZ" {
            return Ok(None);
        }
        let pe = u64::from(u32_at(s, 0x3C)?);
        if pe.checked_add(24).is_none_or(|end| end > s.size()) || s.bytes(pe, 4)? != b"PE\0\0" {
            return Ok(None);
        }
        let count = u64::from(u16_at(s, pe + 6)?).min(96);
        let optional = pe + 24;
        let optional_size = u64::from(u16_at(s, pe + 20)?);
        let dirs = optional
            + match u16_at(s, optional)? {
                0x10B => 96,
                0x20B => 112,
                _ => return Ok(None),
            };
        if dirs + 24 > optional + optional_size || u32_at(s, dirs - 4)? < 3 {
            return Ok(None);
        }
        let rva = u64::from(u32_at(s, dirs + 16)?);
        if rva == 0 {
            return Ok(None);
        }
        let table = optional + optional_size;
        let mut sections = Vec::new();
        for i in 0..count {
            let at = table + i * 40;
            let va = u64::from(u32_at(s, at + 12)?);
            let raw_size = u64::from(u32_at(s, at + 16)?);
            let raw = u64::from(u32_at(s, at + 20)?);
            // Only the raw (file-backed) part of a section maps to bytes.
            sections.push((va, raw_size, raw));
        }
        let mut r = Self { base: 0, sections };
        match r.file(rva) {
            Some(base) => {
                r.base = base;
                Ok(Some(r))
            }
            None => Ok(None),
        }
    }
    fn file(&self, rva: u64) -> Option<u64> {
        self.sections
            .iter()
            .find(|(va, size, _)| *va <= rva && rva < va + size)
            .map(|(va, _, raw)| raw + (rva - va))
    }
    /// The entries of the directory at `offset` (relative to the root):
    /// (name or id, high bit set for a name; target offset).
    fn entries<S: Source + ?Sized>(&self, s: &S, offset: u64) -> Result<Vec<(u32, u32)>, Miss> {
        let at = self.base + offset;
        let count = (u16_at(s, at + 12)? + u16_at(s, at + 14)?) as usize;
        if count > MAX_ENTRIES {
            return Err(Miss::Malformed);
        }
        let raw = s.bytes(at + 16, count as u64 * 8)?;
        Ok(raw
            .chunks_exact(8)
            .map(|e| {
                (
                    u32::from_le_bytes([e[0], e[1], e[2], e[3]]),
                    u32::from_le_bytes([e[4], e[5], e[6], e[7]]),
                )
            })
            .collect())
    }
    /// Directory → its subdirectory offset.
    fn subdirectory(target: u32) -> Option<u64> {
        (target & 0x8000_0000 != 0).then(|| u64::from(target & 0x7FFF_FFFF))
    }
    /// The first language's data (file offset, size) under name directory `offset`.
    fn data<S: Source + ?Sized>(&self, s: &S, offset: u64) -> Result<Option<(u64, u64)>, Miss> {
        let Some(&(_, target)) = self.entries(s, offset)?.first() else {
            return Ok(None);
        };
        if target & 0x8000_0000 != 0 {
            return Err(Miss::Malformed);
        }
        let at = self.base + u64::from(target);
        let rva = u64::from(u32_at(s, at)?);
        let size = u64::from(u32_at(s, at + 4)?);
        Ok(self
            .file(rva)
            .filter(|f| f.checked_add(size).is_some_and(|end| end <= s.size()))
            .map(|f| (f, size)))
    }
    fn type_directory<S: Source + ?Sized>(&self, s: &S, kind: u32) -> Result<Option<u64>, Miss> {
        Ok(self
            .entries(s, 0)?
            .into_iter()
            .find(|(id, _)| *id == kind)
            .and_then(|(_, target)| Self::subdirectory(target)))
    }
}
/// The frames of the program's first icon group; `None` without one.
pub fn group<S: Source + ?Sized>(s: &S) -> Result<Option<Vec<Entry>>, Miss> {
    group_at(s, Which::Index(0))
}
/// An icon group as ExtractIcon names it: the n-th group (named groups
/// first, then ids ascending), or the group with a resource id (a
/// negative ExtractIcon index).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which {
    Index(u32),
    Id(u32),
}
/// How many icon groups the program has (ExtractIcon with index -1).
pub fn group_count<S: Source + ?Sized>(s: &S) -> Result<u32, Miss> {
    let Some(r) = Resources::locate(s)? else {
        return Ok(0);
    };
    let Some(groups) = r.type_directory(s, RT_GROUP_ICON)? else {
        return Ok(0);
    };
    Ok(r.entries(s, groups)?.len() as u32)
}
/// The frames of one icon group; `None` without it.
pub fn group_at<S: Source + ?Sized>(s: &S, which: Which) -> Result<Option<Vec<Entry>>, Miss> {
    let Some(r) = Resources::locate(s)? else {
        return Ok(None);
    };
    let Some(groups) = r.type_directory(s, RT_GROUP_ICON)? else {
        return Ok(None);
    };
    let entries = r.entries(s, groups)?;
    let found = match which {
        Which::Index(n) => entries.get(n as usize),
        Which::Id(id) => entries.iter().find(|(name, _)| *name == id),
    };
    let Some(&(_, first)) = found else {
        return Ok(None);
    };
    let Some(name) = Resources::subdirectory(first) else {
        return Ok(None);
    };
    let Some((at, size)) = r.data(s, name)? else {
        return Ok(None);
    };
    if !(6..=6 + 14 * 256).contains(&size) {
        return Ok(None);
    }
    Ok(directory(s.bytes(at, size)?))
}
/// A `GRPICONDIR` (an `RT_GROUP_ICON` resource, as LookupIconIdFromDirectory
/// takes it): its frames, or `None` when malformed.
pub fn directory(raw: &[u8]) -> Option<Vec<Entry>> {
    if raw.len() < 6 || u16::from_le_bytes([raw[2], raw[3]]) != 1 {
        return None;
    }
    let count = usize::from(u16::from_le_bytes([raw[4], raw[5]]));
    if count > 256 || 6 + count * 14 > raw.len() {
        return None;
    }
    Some(
        raw[6..6 + count * 14]
            .chunks_exact(14)
            .map(|e| Entry {
                width: if e[0] == 0 { 256 } else { u32::from(e[0]) },
                height: if e[1] == 0 { 256 } else { u32::from(e[1]) },
                bits: u32::from(u16::from_le_bytes([e[6], e[7]])),
                id: u32::from(u16::from_le_bytes([e[12], e[13]])),
            })
            .collect(),
    )
}
/// The encoded frame (a DIB without file header, or a PNG) of `entry`.
pub fn frame<'a, S: Source + ?Sized>(s: &'a S, entry: &Entry) -> Result<Option<&'a [u8]>, Miss> {
    let Some(r) = Resources::locate(s)? else {
        return Ok(None);
    };
    let Some(icons) = r.type_directory(s, RT_ICON)? else {
        return Ok(None);
    };
    let Some((_, target)) = r
        .entries(s, icons)?
        .into_iter()
        .find(|(id, _)| *id == entry.id)
    else {
        return Ok(None);
    };
    let Some(name) = Resources::subdirectory(target) else {
        return Ok(None);
    };
    let Some((at, size)) = r.data(s, name)? else {
        return Ok(None);
    };
    if size > MAX_FRAME {
        return Ok(None);
    }
    s.bytes(at, size).map(Some)
}
/// The frame to draw at `pixels` physical size: the smallest square one at
/// least that large with the most colours, else the largest.
pub fn choose(entries: &[Entry], pixels: u32) -> Option<Entry> {
    let square = || entries.iter().filter(|e| e.width == e.height);
    let best_depth = |size: u32| {
        square()
            .filter(|e| e.width == size)
            .max_by_key(|e| e.bits)
            .copied()
    };
    let size = square()
        .map(|e| e.width)
        .filter(|w| *w >= pixels)
        .min()
        .or_else(|| square().map(|e| e.width).max())?;
    best_depth(size)
}
pub fn is_png(frame: &[u8]) -> bool {
    frame.starts_with(b"\x89PNG\r\n\x1a\n")
}
/// A decoded frame: premultiplied BGRA rows, top first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}
/// Decode a DIB icon frame (`BITMAPINFOHEADER`, colour bitmap then the AND
/// mask, both bottom-up, rows padded to 4 bytes).
pub fn decode_dib(b: &[u8]) -> Option<Image> {
    let le32 = |at: usize| Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?));
    let le16 = |at: usize| Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?));
    let header = le32(0)? as usize;
    let width = le32(4)? as i32;
    let height = le32(8)? as i32 / 2;
    let bits = u32::from(le16(14)?);
    if header < 40 || !(1..=256).contains(&width) || !(1..=256).contains(&height) || le32(16)? != 0
    {
        return None;
    }
    let (w, h) = (width as usize, height as usize);
    let colors = match bits {
        1 | 4 | 8 => match le32(32)? {
            0 => 1usize << bits,
            n => (n as usize).min(1 << bits),
        },
        24 | 32 => 0,
        _ => return None,
    };
    let palette = b.get(header..header + colors * 4)?;
    let pixels = header + colors * 4;
    let stride = (w * bits as usize).div_ceil(32) * 4;
    let mask_stride = w.div_ceil(32) * 4;
    let mask = pixels + stride * h;
    // A 32-bit frame without the AND mask is accepted (alpha only).
    let has_mask = b.len() >= mask + mask_stride * h;
    if b.len() < mask {
        return None;
    }
    let mut out = alloc::vec![0u8; w * h * 4];
    let mut any_alpha = false;
    for y in 0..h {
        let row = &b[pixels + (h - 1 - y) * stride..][..stride];
        for x in 0..w {
            let (bgr, a) = match bits {
                32 => {
                    let p = &row[x * 4..x * 4 + 4];
                    any_alpha |= p[3] != 0;
                    ([p[0], p[1], p[2]], p[3])
                }
                24 => {
                    let p = &row[x * 3..x * 3 + 3];
                    ([p[0], p[1], p[2]], 255)
                }
                _ => {
                    let per = 8 / bits as usize;
                    let byte = row[x / per];
                    let shift = 8 - bits as usize * (x % per + 1);
                    let index = usize::from((byte >> shift) & ((1u16 << bits) - 1) as u8);
                    let p = palette.get(index * 4..index * 4 + 3)?;
                    ([p[0], p[1], p[2]], 255)
                }
            };
            let o = (y * w + x) * 4;
            out[o..o + 3].copy_from_slice(&bgr);
            out[o + 3] = a;
        }
    }
    // Without per-pixel alpha the AND mask decides transparency.
    if bits != 32 || !any_alpha {
        if !has_mask {
            return None;
        }
        for y in 0..h {
            let row = &b[mask + (h - 1 - y) * mask_stride..][..mask_stride];
            for x in 0..w {
                let transparent = row[x / 8] & (0x80 >> (x % 8)) != 0;
                out[(y * w + x) * 4 + 3] = if transparent { 0 } else { 255 };
            }
        }
    }
    for p in out.chunks_exact_mut(4) {
        let a = u16::from(p[3]);
        for c in &mut p[..3] {
            *c = ((u16::from(*c) * a + 127) / 255) as u8;
        }
    }
    Some(Image {
        width: width as u32,
        height: height as u32,
        bgra: out,
    })
}

#[cfg(test)]
mod tests;

/// One frame of an `.ico` or `.cur` file (`ICONDIRENTRY`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileFrame {
    pub entry: Entry,
    /// The frame's bytes in the file.
    pub offset: u64,
    pub size: u64,
    /// A cursor's hot spot (an icon's planes and bit count are kept in
    /// `entry.bits`).
    pub hotspot: (u16, u16),
}
/// The frames of an `.ico` (`cursor` false) or `.cur` file; `None` when it
/// is neither or malformed. Frame payloads are DIBs or PNGs, as in PE
/// resources.
pub fn icon_file(b: &[u8]) -> Option<(bool, Vec<FileFrame>)> {
    let le16 = |at: usize| Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?));
    let le32 = |at: usize| Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?));
    if le16(0)? != 0 {
        return None;
    }
    let cursor = match le16(2)? {
        1 => false,
        2 => true,
        _ => return None,
    };
    let count = usize::from(le16(4)?);
    if count == 0 || count > 256 || b.len() < 6 + count * 16 {
        return None;
    }
    let mut frames = Vec::new();
    for (i, e) in b[6..6 + count * 16].chunks_exact(16).enumerate() {
        let at = 6 + i * 16;
        let size = u64::from(le32(at + 8)?);
        let offset = u64::from(le32(at + 12)?);
        if size == 0 || size > MAX_FRAME || offset + size > b.len() as u64 {
            return None;
        }
        let (a, c) = (le16(at + 4)?, le16(at + 6)?);
        frames.push(FileFrame {
            entry: Entry {
                width: if e[0] == 0 { 256 } else { u32::from(e[0]) },
                height: if e[1] == 0 { 256 } else { u32::from(e[1]) },
                bits: if cursor { 32 } else { u32::from(c) },
                id: i as u32,
            },
            offset,
            size,
            hotspot: if cursor { (a, c) } else { (0, 0) },
        });
    }
    Some((cursor, frames))
}
/// Decode a frame (a DIB or, with the `png` feature, a PNG) to
/// premultiplied BGRA.
pub fn decode(frame: &[u8]) -> Option<Image> {
    if is_png(frame) {
        decode_png(frame)
    } else {
        decode_dib(frame)
    }
}
#[cfg(feature = "png")]
fn decode_png(b: &[u8]) -> Option<Image> {
    use zune_png::zune_core::{bytestream::ZCursor, options::DecoderOptions};
    let options = DecoderOptions::default()
        .set_max_width(256)
        .set_max_height(256)
        .set_use_unsafe(false)
        .inflate_set_limit(256 * 256 * 8 + 32768)
        .set_strict_mode(true)
        .png_set_confirm_crc(true)
        .png_set_decode_animated(false)
        .png_set_strip_to_8bit(true);
    let mut d = zune_png::PngDecoder::new_with_options(ZCursor::new(b), options);
    d.decode_headers().ok()?;
    let (w, h) = d.dimensions()?;
    if !(1..=256).contains(&w) || !(1..=256).contains(&h) {
        return None;
    }
    let raw = d.decode_raw().ok()?;
    let components = raw.len() / (w * h);
    if !(1..=4).contains(&components) || raw.len() != w * h * components {
        return None;
    }
    let mut bgra = Vec::with_capacity(w * h * 4);
    for p in raw.chunks_exact(components) {
        let (r, g, bl, a) = match components {
            1 => (p[0], p[0], p[0], 255),
            2 => (p[0], p[0], p[0], p[1]),
            3 => (p[0], p[1], p[2], 255),
            _ => (p[0], p[1], p[2], p[3]),
        };
        let m = |c: u8| ((u16::from(c) * u16::from(a) + 127) / 255) as u8;
        bgra.extend_from_slice(&[m(bl), m(g), m(r), a]);
    }
    Some(Image {
        width: w as u32,
        height: h as u32,
        bgra,
    })
}
#[cfg(not(feature = "png"))]
fn decode_png(_: &[u8]) -> Option<Image> {
    None
}
/// A QOI image (the Doors asset format) to premultiplied BGRA; the asset
/// pack stores it premultiplied already.
#[cfg(feature = "qoi")]
pub fn decode_qoi(b: &[u8]) -> Option<Image> {
    let header = qoi::decode_header(b).ok()?;
    if header.width == 0 || header.height == 0 || header.width > 512 || header.height > 512 {
        return None;
    }
    let (header, rgba) = qoi::decode_to_vec(b).ok()?;
    let n = header.channels.as_u8() as usize;
    if rgba.len() != header.width as usize * header.height as usize * n {
        return None;
    }
    let bgra = rgba
        .chunks_exact(n)
        .flat_map(|p| [p[2], p[1], p[0], if n == 4 { p[3] } else { 255 }])
        .collect();
    Some(Image {
        width: header.width,
        height: header.height,
        bgra,
    })
}
/// The ICO frame to draw at `width` x `height` (0: the system size
/// `default`), as LookupIconIdFromDirectoryEx chooses: the closest size
/// not smaller when one exists, then the most colours.
pub fn choose_size(entries: &[Entry], width: u32, height: u32) -> Option<Entry> {
    let want = |w: u32, h: u32| (w.max(1), h.max(1));
    let (w, h) = want(width, height);
    let fits = entries.iter().filter(|e| e.width >= w && e.height >= h);
    let pick = fits
        .min_by_key(|e| ((e.width - w) + (e.height - h), u32::MAX - e.bits))
        .or_else(|| {
            entries
                .iter()
                .max_by_key(|e| (e.width.min(w) + e.height.min(h), e.bits))
        });
    pick.copied()
}
