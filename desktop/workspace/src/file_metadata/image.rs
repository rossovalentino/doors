//! Images: PNG, JPEG (with Exif), GIF, BMP, QOI, PNM, WebP, ICO/CUR and TIFF.
use super::Section::{Camera, Description, Gps, Image, Origin, Photo};
use super::*;

const MAX_CHUNKS: usize = 256;

fn pixels(r: &mut Report, w: u64, h: u64) {
    if w == 0 || h == 0 {
        return;
    }
    r.add(Image, "Dimensions", dims(w, h));
    r.add(Image, "Width", format!("{} pixels", grouped(w)));
    r.add(Image, "Height", format!("{} pixels", grouped(h)));
}
fn resolution(r: &mut Report, x: u64, y: u64) {
    if x > 0 {
        r.add(Image, "Horizontal resolution", format!("{x} dpi"));
    }
    if y > 0 {
        r.add(Image, "Vertical resolution", format!("{y} dpi"));
    }
}

pub(super) fn png(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    r.format("PNG image");
    let mut at = 8u64;
    let mut frames = None;
    for _ in 0..MAX_CHUNKS {
        let h = d.get(at, 8)?;
        let len = u64::from(be32(h, 0).unwrap_or(0));
        let kind: [u8; 4] = take(h, 4).unwrap_or([0; 4]);
        if len > 0x7FFF_FFFF {
            return Err(Miss::Truncated);
        }
        let body = at + 8;
        match &kind {
            b"IHDR" => {
                let b = d.get(body, 13)?;
                pixels(
                    r,
                    u64::from(be32(b, 0).unwrap_or(0)),
                    u64::from(be32(b, 4).unwrap_or(0)),
                );
                let (depth, color) = (u64::from(b[8]), b[9]);
                let (channels, name) = match color {
                    0 => (1, "Greyscale"),
                    2 => (3, "Truecolor (RGB)"),
                    3 => (1, "Indexed color (palette)"),
                    4 => (2, "Greyscale with alpha"),
                    6 => (4, "Truecolor with alpha (RGBA)"),
                    _ => (0, ""),
                };
                if channels > 0 {
                    r.add(Image, "Bit depth", grouped(depth * channels));
                    r.add(Image, "Color type", name);
                }
                if b[12] == 1 {
                    r.add(Image, "Interlaced", "Yes (Adam7)");
                }
            }
            b"pHYs" if len >= 9 => {
                let b = d.get(body, 9)?;
                if b[8] == 1 {
                    let x = u64::from(be32(b, 0).unwrap_or(0));
                    let y = u64::from(be32(b, 4).unwrap_or(0));
                    resolution(r, dpi_from_ppm(x), dpi_from_ppm(y));
                }
            }
            b"acTL" if len >= 8 => {
                let b = d.get(body, 8)?;
                frames = Some((be32(b, 0).unwrap_or(0), be32(b, 4).unwrap_or(0)));
            }
            b"tEXt" | b"iTXt" if len <= 8192 => {
                let b = d.get(body, len)?;
                let Some(nul) = b.iter().position(|c| *c == 0) else {
                    continue_at(&mut at, body, len);
                    continue;
                };
                let key = &b[..nul];
                let value = if &kind == b"tEXt" {
                    latin1(&b[nul + 1..])
                } else {
                    // compression flag, method, language\0, translated keyword\0, text
                    let rest = b.get(nul + 1..).unwrap_or(&[]);
                    if rest.first() != Some(&0) {
                        String::new()
                    } else {
                        let mut parts = rest.get(2..).unwrap_or(&[]).splitn(3, |c| *c == 0);
                        let _ = (parts.next(), parts.next());
                        utf8(parts.next().unwrap_or(&[]))
                    }
                };
                png_text(r, key, &value);
            }
            b"eXIf" if len <= 65_536 => {
                let b = d.get(body, len)?;
                let _ = tiff(Src::Slice(b), r, false);
            }
            b"sRGB" => r.add(Image, "Color representation", "sRGB"),
            b"iCCP" if len <= 8192 => {
                let b = d.get(body, len.min(80))?;
                r.add(Image, "Color profile", latin1(b));
            }
            b"IDAT" | b"IEND" => break,
            _ => (),
        }
        at = body + len + 4;
    }
    if let Some((count, plays)) = frames {
        r.add(Image, "Frames", grouped(u64::from(count)));
        r.add(
            Image,
            "Animation",
            if plays == 0 {
                String::from("Repeats forever")
            } else {
                format!("Plays {} times", grouped(u64::from(plays)))
            },
        );
    }
    Ok(())
}
fn continue_at(at: &mut u64, body: u64, len: u64) {
    *at = body + len + 4;
}
fn png_text(r: &mut Report, key: &[u8], value: &str) {
    match key {
        b"Title" => r.add(Description, "Title", value),
        b"Description" | b"Comment" => r.add(Description, "Comments", value),
        b"Author" => r.add(Origin, "Authors", value),
        b"Copyright" => r.add(Origin, "Copyright", value),
        b"Software" => r.add(Origin, "Program name", value),
        b"Creation Time" => r.add(Origin, "Creation time", value),
        b"Source" => r.add(Origin, "Source", value),
        _ => (),
    }
}

pub(super) fn jpeg(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    r.format("JPEG image");
    let mut at = 2u64;
    let mut jfif = None;
    for _ in 0..512 {
        if d.byte(at)? != 0xFF {
            break;
        }
        let mut marker = d.byte(at + 1)?;
        for _ in 0..16 {
            if marker != 0xFF {
                break;
            }
            at += 1;
            marker = d.byte(at + 1)?;
        }
        at += 2;
        match marker {
            0xD8 | 0x01 | 0xD0..=0xD7 => continue,
            0xD9 | 0xDA => break,
            _ => (),
        }
        let len = u64::from(d.be16(at)?);
        if len < 2 {
            return Err(Miss::Truncated);
        }
        let (body, blen) = (at + 2, len - 2);
        match marker {
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) => {
                let b = d.get(body, 6)?;
                pixels(
                    r,
                    u64::from(be16(b, 3).unwrap_or(0)),
                    u64::from(be16(b, 1).unwrap_or(0)),
                );
                let (precision, components) = (u64::from(b[0]), u64::from(b[5]));
                r.add(Image, "Bit depth", grouped(precision * components));
                r.add(
                    Image,
                    "Color model",
                    match components {
                        1 => "Greyscale",
                        3 => "YCbCr (color)",
                        4 => "CMYK",
                        _ => "",
                    },
                );
                r.add(
                    Image,
                    "Encoding",
                    match marker {
                        0xC0 | 0xC1 => "Baseline",
                        0xC2 | 0xC6 | 0xCA | 0xCE => "Progressive",
                        0xC3 | 0xC7 | 0xCB | 0xCF => "Lossless",
                        _ => "Arithmetic coded",
                    },
                );
            }
            0xE0 if blen >= 14 => {
                let b = d.get(body, 14)?;
                if b.starts_with(b"JFIF\0") {
                    let (x, y) = (
                        u64::from(be16(b, 8).unwrap_or(0)),
                        u64::from(be16(b, 10).unwrap_or(0)),
                    );
                    jfif = match b[7] {
                        1 => Some((x, y)),
                        2 => Some(((x * 254 + 50) / 100, (y * 254 + 50) / 100)),
                        _ => None,
                    };
                }
            }
            0xE1 if blen >= 8 => {
                let b = d.get(body, blen)?;
                if let Some(t) = b.strip_prefix(b"Exif\0\0") {
                    let _ = tiff(Src::Slice(t), r, false);
                }
            }
            0xFE => {
                let b = d.get(body, blen.min(1024))?;
                r.add(Description, "Comments", utf8(b));
            }
            _ => (),
        }
        at = body + blen;
    }
    if let Some((x, y)) = jfif {
        resolution(r, x, y);
    }
    Ok(())
}

pub(super) fn gif(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    r.format("GIF image");
    let h = d.get(0, 13)?;
    pixels(
        r,
        u64::from(le16(h, 6).unwrap_or(0)),
        u64::from(le16(h, 8).unwrap_or(0)),
    );
    let packed = h[10];
    r.add(Image, "Bit depth", format!("{}", (packed & 7) + 1));
    let mut at = 13u64;
    if packed & 0x80 != 0 {
        at += 3 << ((packed & 7) + 1);
    }
    let (mut frames, mut delay, mut loops) = (0u64, 0u64, None);
    let walk = (|| -> Result<(), Miss> {
        for _ in 0..200_000 {
            match d.byte(at)? {
                0x3B => return Ok(()),
                0x21 => {
                    let label = d.byte(at + 1)?;
                    at += 2;
                    let first = u64::from(d.byte(at)?);
                    let block = d.get(at + 1, first)?;
                    match label {
                        0xF9 if block.len() >= 3 => delay += u64::from(le16(block, 1).unwrap_or(0)),
                        0xFF if block == b"NETSCAPE2.0" => {
                            let sub = d.get(at + 1 + first, 4)?;
                            if sub[0] >= 3 && sub[1] == 1 {
                                loops = Some(le16(sub, 2).unwrap_or(0));
                            }
                        }
                        0xFE => r.add(Description, "Comments", latin1(block)),
                        _ => (),
                    }
                    at = skip_blocks(d, at)?;
                }
                0x2C => {
                    frames += 1;
                    let b = d.get(at, 10)?;
                    at += 10;
                    if b[9] & 0x80 != 0 {
                        at += 3 << ((b[9] & 7) + 1);
                    }
                    at = skip_blocks(d, at + 1)?;
                }
                _ => return Err(Miss::Truncated),
            }
        }
        Ok(())
    })();
    if frames > 0 {
        let prefix = if walk.is_ok() { "" } else { "At least " };
        r.add(Image, "Frames", format!("{prefix}{}", grouped(frames)));
        if frames > 1 && walk.is_ok() {
            r.add(
                Image,
                "Animation length",
                format!("{}.{:02} seconds", delay / 100, delay % 100),
            );
        }
    }
    if let Some(n) = loops {
        r.add(
            Image,
            "Animation",
            if n == 0 {
                String::from("Repeats forever")
            } else {
                format!("Repeats {n} times")
            },
        );
    }
    walk
}
fn skip_blocks(d: &Sparse, mut at: u64) -> Result<u64, Miss> {
    for _ in 0..1_000_000 {
        let n = d.byte(at)?;
        at += 1;
        if n == 0 {
            return Ok(at);
        }
        at += u64::from(n);
    }
    Err(Miss::Truncated)
}

pub(super) fn is_bmp(head: &[u8]) -> bool {
    matches!(le32(head, 14), Some(12 | 40 | 52 | 56 | 64 | 108 | 124))
}
pub(super) fn bmp(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    r.format("Bitmap image");
    let dib = d.le32(14)?;
    if dib == 12 {
        let b = d.get(0, 26)?;
        pixels(
            r,
            u64::from(le16(b, 18).unwrap_or(0)),
            u64::from(le16(b, 20).unwrap_or(0)),
        );
        r.add(Image, "Bit depth", format!("{}", le16(b, 24).unwrap_or(0)));
        return Ok(());
    }
    let b = d.get(0, 46)?;
    let w = le32(b, 18).unwrap_or(0) as i32;
    let h = le32(b, 22).unwrap_or(0) as i32;
    pixels(r, u64::from(w.unsigned_abs()), u64::from(h.unsigned_abs()));
    r.add(Image, "Bit depth", format!("{}", le16(b, 28).unwrap_or(0)));
    r.add(
        Image,
        "Compression",
        match le32(b, 30).unwrap_or(99) {
            0 => "Uncompressed",
            1 => "RLE 8-bit",
            2 => "RLE 4-bit",
            3 | 6 => "Bit fields",
            4 => "JPEG",
            5 => "PNG",
            _ => "",
        },
    );
    let x = le32(b, 38).unwrap_or(0) as i32;
    let y = le32(b, 42).unwrap_or(0) as i32;
    if x > 0 && y > 0 {
        resolution(r, dpi_from_ppm(x as u64), dpi_from_ppm(y as u64));
    }
    Ok(())
}

pub(super) fn qoi(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    r.format("QOI image");
    let b = d.get(0, 14)?;
    pixels(
        r,
        u64::from(be32(b, 4).unwrap_or(0)),
        u64::from(be32(b, 8).unwrap_or(0)),
    );
    if matches!(b[12], 3 | 4) {
        r.add(Image, "Bit depth", format!("{}", u32::from(b[12]) * 8));
        r.add(Image, "Color type", if b[12] == 4 { "RGBA" } else { "RGB" });
    }
    r.add(
        Image,
        "Color representation",
        if b[13] == 1 { "Linear" } else { "sRGB" },
    );
    Ok(())
}

/// PBM, PGM, PPM (ASCII and binary) and PAM headers.
pub(super) fn pnm(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    let b = d.clip(0, 1024)?;
    let kind = b[1];
    r.format(match kind {
        b'1' | b'4' => "Portable bitmap (PBM)",
        b'2' | b'5' => "Portable graymap (PGM)",
        b'7' => "Portable arbitrary map (PAM)",
        _ => "Portable pixmap (PPM)",
    });
    if kind != b'7' {
        r.add(
            Image,
            "Encoding",
            if kind <= b'3' { "ASCII" } else { "Binary" },
        );
    }
    let mut numbers = Vec::new();
    let mut fields: Vec<(&[u8], u64)> = Vec::new();
    let mut comment = None;
    let text = &b[2..];
    if kind == b'7' {
        for line in text.split(|c| *c == b'\n').take(32) {
            let mut words = line
                .split(|c| c.is_ascii_whitespace())
                .filter(|w| !w.is_empty());
            match (words.next(), words.next()) {
                (Some(b"ENDHDR"), _) => break,
                (Some(key), Some(v)) => fields.push((key, number(v).unwrap_or(0))),
                _ => (),
            }
        }
        let get = |k: &[u8]| fields.iter().find(|(key, _)| *key == k).map(|(_, v)| *v);
        let (w, h, depth, max) = (get(b"WIDTH"), get(b"HEIGHT"), get(b"DEPTH"), get(b"MAXVAL"));
        let (Some(w), Some(h)) = (w, h) else {
            return Err(Miss::Truncated);
        };
        pixels(r, w, h);
        if let (Some(depth), Some(max)) = (depth, max) {
            r.add(Image, "Bit depth", grouped(depth.min(64) * bits(max)));
        }
        return Ok(());
    }
    let wanted = if matches!(kind, b'1' | b'4') { 2 } else { 3 };
    let mut i = 0;
    while numbers.len() < wanted && i < text.len() {
        let c = text[i];
        if c == b'#' {
            let end = text[i..]
                .iter()
                .position(|c| *c == b'\n')
                .map_or(text.len(), |p| i + p);
            comment.get_or_insert_with(|| latin1(&text[i + 1..end]));
            i = end;
        } else if c.is_ascii_digit() {
            let end = text[i..]
                .iter()
                .position(|c| !c.is_ascii_digit())
                .map_or(text.len(), |p| i + p);
            if end == text.len() {
                break;
            }
            numbers.push(number(&text[i..end]).unwrap_or(0));
            i = end;
        } else {
            i += 1;
        }
    }
    if numbers.len() < wanted {
        return Err(Miss::Truncated);
    }
    pixels(r, numbers[0], numbers[1]);
    let depth = match kind {
        b'1' | b'4' => 1,
        b'2' | b'5' => bits(numbers[2]),
        _ => 3 * bits(numbers[2]),
    };
    r.add(Image, "Bit depth", grouped(depth));
    if let Some(c) = comment {
        r.add(Description, "Comments", c);
    }
    Ok(())
}
fn number(b: &[u8]) -> Option<u64> {
    core::str::from_utf8(b).ok()?.trim().parse().ok()
}
/// Bits needed for a maximum sample value.
fn bits(max: u64) -> u64 {
    u64::from(64 - max.max(1).leading_zeros())
}

pub(super) fn webp(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    r.format("WebP image");
    let end = (8 + u64::from(d.le32(4)?)).min(d.size());
    let mut at = 12u64;
    let mut frames = 0u64;
    let walk = (|| -> Result<(), Miss> {
        for _ in 0..4096 {
            if at + 8 > end {
                return Ok(());
            }
            let h = d.get(at, 8)?;
            let len = u64::from(le32(h, 4).unwrap_or(0));
            let body = at + 8;
            match &h[..4] {
                b"VP8 " => {
                    let b = d.get(body, 10)?;
                    if b[3..6] == [0x9D, 0x01, 0x2A] {
                        pixels(
                            r,
                            u64::from(le16(b, 6).unwrap_or(0) & 0x3FFF),
                            u64::from(le16(b, 8).unwrap_or(0) & 0x3FFF),
                        );
                    }
                    r.add(Image, "Compression", "Lossy (VP8)");
                    r.add(Image, "Bit depth", "24");
                }
                b"VP8L" => {
                    let b = d.get(body, 5)?;
                    if b[0] == 0x2F {
                        let v = u64::from(le32(b, 1).unwrap_or(0));
                        pixels(r, (v & 0x3FFF) + 1, ((v >> 14) & 0x3FFF) + 1);
                        r.add(
                            Image,
                            "Bit depth",
                            if (v >> 28) & 1 == 1 { "32" } else { "24" },
                        );
                    }
                    r.add(Image, "Compression", "Lossless (VP8L)");
                }
                b"VP8X" => {
                    let b = d.get(body, 10)?;
                    let w = u64::from(b[4]) | u64::from(b[5]) << 8 | u64::from(b[6]) << 16;
                    let h = u64::from(b[7]) | u64::from(b[8]) << 8 | u64::from(b[9]) << 16;
                    pixels(r, w + 1, h + 1);
                    if b[0] & 0x10 != 0 {
                        r.add(Image, "Transparency", "Yes");
                    }
                }
                b"ANIM" => {
                    let b = d.get(body, 6)?;
                    let n = le16(b, 4).unwrap_or(0);
                    r.add(
                        Image,
                        "Animation",
                        if n == 0 {
                            String::from("Repeats forever")
                        } else {
                            format!("Plays {n} times")
                        },
                    );
                }
                b"ANMF" => frames += 1,
                b"EXIF" if len <= 65_536 => {
                    let b = d.get(body, len)?;
                    let _ = tiff(
                        Src::Slice(b.strip_prefix(b"Exif\0\0").unwrap_or(b)),
                        r,
                        false,
                    );
                }
                b"ICCP" => r.add(Image, "Color profile", "Embedded"),
                _ => (),
            }
            at = body + len + (len & 1);
        }
        Ok(())
    })();
    if frames > 0 {
        r.add(
            Image,
            "Frames",
            format!(
                "{}{}",
                if walk.is_ok() { "" } else { "At least " },
                grouped(frames)
            ),
        );
    }
    walk
}

pub(super) fn is_ico(head: &[u8]) -> bool {
    let count = le16(head, 4).unwrap_or(0);
    (1..=255).contains(&count) && head.get(9) == Some(&0) && matches!(le16(head, 10), Some(0 | 1))
}
pub(super) fn ico(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    let h = d.get(0, 6)?;
    r.format(if le16(h, 2) == Some(2) {
        "Cursor"
    } else {
        "Icon"
    });
    let count = u64::from(le16(h, 4).unwrap_or(0)).min(256);
    let table = d.get(6, count * 16)?;
    let mut sizes: Vec<(u64, u64)> = Vec::new();
    let mut depth = 0;
    for e in table.chunks_exact(16) {
        let w = if e[0] == 0 { 256 } else { u64::from(e[0]) };
        let h = if e[1] == 0 { 256 } else { u64::from(e[1]) };
        if !sizes.contains(&(w, h)) {
            sizes.push((w, h));
        }
        depth = depth.max(le16(e, 6).unwrap_or(0));
    }
    sizes.sort();
    r.add(Image, "Images", grouped(count));
    if let Some((w, h)) = sizes.last() {
        r.add(Image, "Dimensions", dims(*w, *h));
    }
    let list: Vec<String> = sizes.iter().take(12).map(|(w, h)| dims(*w, *h)).collect();
    r.add(Image, "Sizes", list.join(", "));
    if depth > 0 {
        r.add(Image, "Bit depth", format!("{depth}"));
    }
    Ok(())
}

pub(super) fn tiff_file(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    r.format("TIFF image");
    tiff(Src::File(d), r, true)
}

/// Where a TIFF structure lives: an Exif block already read, or the file.
pub(super) enum Src<'a> {
    Slice(&'a [u8]),
    File(&'a Sparse),
}
impl<'a> Src<'a> {
    fn get(&self, offset: u64, len: u64) -> Result<&'a [u8], Miss> {
        match self {
            Self::Slice(b) => {
                let b: &'a [u8] = b;
                let start = usize::try_from(offset).map_err(|_| Miss::Truncated)?;
                let end = start
                    .checked_add(usize::try_from(len).map_err(|_| Miss::Truncated)?)
                    .ok_or(Miss::Truncated)?;
                b.get(start..end).ok_or(Miss::Truncated)
            }
            Self::File(d) => {
                let d: &'a Sparse = d;
                d.get(offset, len)
            }
        }
    }
}
struct Entry {
    tag: u16,
    kind: u16,
    count: u32,
    value: [u8; 4],
}
struct Tiff<'a> {
    src: Src<'a>,
    little: bool,
}
impl Tiff<'_> {
    fn u16(&self, b: &[u8], at: usize) -> u16 {
        if self.little {
            le16(b, at)
        } else {
            be16(b, at)
        }
        .unwrap_or(0)
    }
    fn u32(&self, b: &[u8], at: usize) -> u32 {
        if self.little {
            le32(b, at)
        } else {
            be32(b, at)
        }
        .unwrap_or(0)
    }
    fn entries(&self, offset: u64) -> Result<Vec<Entry>, Miss> {
        let count = u64::from(self.u16(self.src.get(offset, 2)?, 0));
        if count == 0 || count > 512 {
            return Ok(Vec::new());
        }
        let raw = self.src.get(offset + 2, count * 12)?;
        Ok(raw
            .chunks_exact(12)
            .map(|e| Entry {
                tag: self.u16(e, 0),
                kind: self.u16(e, 2),
                count: self.u32(e, 4),
                value: take(e, 8).unwrap_or([0; 4]),
            })
            .collect())
    }
    fn bytes(&self, e: &Entry) -> Result<Vec<u8>, Miss> {
        let unit: u64 = match e.kind {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            5 | 10 | 12 => 8,
            _ => return Ok(Vec::new()),
        };
        let size = (unit * u64::from(e.count)).min(4096);
        if size <= 4 {
            return Ok(e.value[..size as usize].to_vec());
        }
        Ok(self
            .src
            .get(u64::from(self.u32(&e.value, 0)), size)?
            .to_vec())
    }
    fn uint(&self, e: &Entry) -> Option<u64> {
        match e.kind {
            1 | 7 => Some(u64::from(e.value[0])),
            3 => Some(u64::from(self.u16(&e.value, 0))),
            4 | 9 => Some(u64::from(self.u32(&e.value, 0))),
            _ => None,
        }
    }
    fn rationals(&self, e: &Entry) -> Result<Vec<(i64, i64)>, Miss> {
        if !matches!(e.kind, 5 | 10) {
            return Ok(Vec::new());
        }
        let b = self.bytes(e)?;
        Ok(b.chunks_exact(8)
            .map(|p| {
                let (n, d) = (self.u32(p, 0), self.u32(p, 4));
                if e.kind == 10 {
                    (i64::from(n as i32), i64::from(d as i32))
                } else {
                    (i64::from(n), i64::from(d))
                }
            })
            .collect())
    }
    fn rational(&self, e: &Entry) -> Result<Option<(i64, i64)>, Miss> {
        Ok(self.rationals(e)?.first().copied().filter(|(_, d)| *d != 0))
    }
    fn text(&self, e: &Entry) -> Result<String, Miss> {
        Ok(match e.kind {
            2 => latin1(&self.bytes(e)?),
            // Windows' XP* tags: UTF-16LE in BYTE fields.
            1 => utf16(&self.bytes(e)?, true),
            _ => String::new(),
        })
    }
}
/// "2.8", "0.3", "-1.7": one decimal, trailing ".0" dropped.
fn decimal(n: i64, d: i64) -> String {
    if d == 0 {
        return String::new();
    }
    let tenths = (n * 10 + if (n < 0) == (d < 0) { d / 2 } else { -d / 2 }) / d;
    let sign = if tenths < 0 { "-" } else { "" };
    let t = tenths.unsigned_abs();
    if t.is_multiple_of(10) {
        format!("{sign}{}", t / 10)
    } else {
        format!("{sign}{}.{}", t / 10, t % 10)
    }
}
fn exif_date(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() >= 19 && b[..19].is_ascii() && b[4] == b':' && b[7] == b':' {
        format!("{}-{}-{} {}", &s[..4], &s[5..7], &s[8..10], &s[11..19])
    } else {
        String::from(s)
    }
}
/// Exif/TIFF tags as Windows' Details shows them. `file`: a whole TIFF
/// file (its image tags count), not an Exif block of another format.
pub(super) fn tiff(src: Src<'_>, r: &mut Report, file: bool) -> Result<(), Miss> {
    let h = src.get(0, 8)?;
    let little = match &h[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Ok(()),
    };
    let t = Tiff { src, little };
    if t.u16(h, 2) != 42 {
        return Ok(());
    }
    let mut queue = alloc::vec![(u64::from(t.u32(h, 4)), 0u8)];
    let mut unit = 2;
    let mut resolution_xy = (None, None);
    let mut gps: [Option<Vec<(i64, i64)>>; 3] = [None, None, None];
    let mut refs = [0u8; 3];
    let mut seen = 0;
    while let Some((offset, kind)) = queue.pop() {
        seen += 1;
        if seen > 4 || offset == 0 {
            continue;
        }
        for e in t.entries(offset)? {
            match (kind, e.tag) {
                (0, 0x8769) => queue.push((t.uint(&e).unwrap_or(0), 1)),
                (0, 0x8825) => queue.push((t.uint(&e).unwrap_or(0), 2)),
                (0, 0x0100) if file => {
                    if let Some(w) = t.uint(&e) {
                        r.add(Image, "Width", format!("{} pixels", grouped(w)));
                    }
                }
                (0, 0x0101) if file => {
                    if let Some(h) = t.uint(&e) {
                        r.add(Image, "Height", format!("{} pixels", grouped(h)));
                    }
                }
                (0, 0x0102) if file => {
                    let b = t.bytes(&e)?;
                    let sum: u64 = b
                        .chunks_exact(2)
                        .take(8)
                        .map(|p| u64::from(t.u16(p, 0)))
                        .sum();
                    r.add(Image, "Bit depth", grouped(sum));
                }
                (0, 0x0103) if file => r.add(
                    Image,
                    "Compression",
                    match t.uint(&e).unwrap_or(0) {
                        1 => "Uncompressed",
                        2..=4 => "CCITT",
                        5 => "LZW",
                        6 | 7 => "JPEG",
                        8 | 32946 => "Deflate",
                        32773 => "PackBits",
                        _ => "",
                    },
                ),
                (0, 0x010E) => r.add(Description, "Title", t.text(&e)?),
                (0, 0x010F) => r.add(Camera, "Camera maker", t.text(&e)?),
                (0, 0x0110) => r.add(Camera, "Camera model", t.text(&e)?),
                (0, 0x0112) => r.add(
                    Photo,
                    "Orientation",
                    match t.uint(&e).unwrap_or(0) {
                        1 => "Normal",
                        2 => "Flip horizontal",
                        3 => "Rotate 180 degrees",
                        4 => "Flip vertical",
                        5 => "Transpose",
                        6 => "Rotate 90 degrees clockwise",
                        7 => "Transverse",
                        8 => "Rotate 270 degrees clockwise",
                        _ => "",
                    },
                ),
                (0, 0x011A) => resolution_xy.0 = t.rational(&e)?,
                (0, 0x011B) => resolution_xy.1 = t.rational(&e)?,
                (0, 0x0128) => unit = t.uint(&e).unwrap_or(2),
                (0, 0x0131) => r.add(Origin, "Program name", t.text(&e)?),
                (0, 0x013B) => r.add(Origin, "Authors", t.text(&e)?),
                (0, 0x8298) => r.add(Origin, "Copyright", t.text(&e)?),
                (0, 0x4746) => {
                    if let Some(stars) = t.uint(&e).filter(|s| (1..=5).contains(s)) {
                        r.add(
                            Description,
                            "Rating",
                            format!("{stars} {}", if stars == 1 { "star" } else { "stars" }),
                        );
                    }
                }
                (0, 0x9C9B) => r.add(Description, "Title", t.text(&e)?),
                (0, 0x9C9C) => r.add(Description, "Comments", t.text(&e)?),
                (0, 0x9C9D) => r.add(Origin, "Authors", t.text(&e)?),
                (0, 0x9C9E) => r.add(Description, "Tags", t.text(&e)?),
                (0, 0x9C9F) => r.add(Description, "Subject", t.text(&e)?),
                (1, 0x829A) => {
                    if let Some((n, d)) = t.rational(&e)?.filter(|(n, _)| *n > 0) {
                        r.add(
                            Camera,
                            "Exposure time",
                            if n < d {
                                format!("1/{} sec.", (d + n / 2) / n)
                            } else {
                                format!("{} sec.", decimal(n, d))
                            },
                        );
                    }
                }
                (1, 0x829D) => {
                    if let Some((n, d)) = t.rational(&e)? {
                        r.add(Camera, "F-stop", format!("f/{}", decimal(n, d)));
                    }
                }
                (1, 0x8827) => {
                    if let Some(iso) = t.uint(&e) {
                        r.add(Camera, "ISO speed", format!("ISO-{iso}"));
                    }
                }
                (1, 0x9003) => r.add(Origin, "Date taken", exif_date(&t.text(&e)?)),
                (1, 0x9204) => {
                    if let Some((n, d)) = t.rational(&e)? {
                        let v = decimal(n, d);
                        r.add(
                            Camera,
                            "Exposure bias",
                            format!("{}{v} step", if n > 0 && d > 0 { "+" } else { "" }),
                        );
                    }
                }
                (1, 0x9207) => r.add(
                    Camera,
                    "Metering mode",
                    match t.uint(&e).unwrap_or(0) {
                        1 => "Average",
                        2 => "Center weighted average",
                        3 => "Spot",
                        4 => "Multi spot",
                        5 => "Pattern",
                        6 => "Partial",
                        _ => "",
                    },
                ),
                (1, 0x9209) => {
                    if let Some(flash) = t.uint(&e) {
                        r.add(
                            Camera,
                            "Flash mode",
                            if flash & 1 == 1 { "Flash" } else { "No flash" },
                        );
                    }
                }
                (1, 0x920A) => {
                    if let Some((n, d)) = t.rational(&e)? {
                        r.add(Camera, "Focal length", format!("{} mm", decimal(n, d)));
                    }
                }
                (1, 0xA405) => {
                    if let Some(f) = t.uint(&e).filter(|f| *f > 0) {
                        r.add(Camera, "35mm focal length", format!("{f}"));
                    }
                }
                (1, 0xA001) => match t.uint(&e) {
                    Some(1) => r.add(Image, "Color representation", "sRGB"),
                    Some(0xFFFF) => r.add(Image, "Color representation", "Uncalibrated"),
                    _ => (),
                },
                (2, 1) => refs[0] = e.value[0],
                (2, 3) => refs[1] = e.value[0],
                (2, 5) => refs[2] = e.value[0],
                (2, 2) => gps[0] = Some(t.rationals(&e)?),
                (2, 4) => gps[1] = Some(t.rationals(&e)?),
                (2, 6) => gps[2] = Some(t.rationals(&e)?),
                _ => (),
            }
        }
    }
    if let (Some((xn, xd)), Some((yn, yd))) = resolution_xy {
        let scale = |n: i64, d: i64| {
            let v = (n.max(0) as u64 * 100 / d.max(1) as u64 + 50) / 100;
            if unit == 3 { (v * 254 + 50) / 100 } else { v }
        };
        if unit != 1 {
            resolution(r, scale(xn, xd), scale(yn, yd));
        }
    }
    for (i, label) in [(0, "Latitude"), (1, "Longitude")] {
        if let Some(parts) = gps[i]
            .as_ref()
            .filter(|p| p.len() == 3 && p.iter().all(|(_, d)| *d > 0))
        {
            // Micro-degrees, integer only.
            let micro: i64 = parts
                .iter()
                .zip([1i64, 60, 3600])
                .map(|((n, d), div)| n.saturating_mul(1_000_000) / d.saturating_mul(div))
                .sum();
            let hemisphere = char::from(refs[i]);
            let hemisphere = if hemisphere.is_ascii_alphabetic() {
                hemisphere
            } else {
                ' '
            };
            r.add(
                Gps,
                label,
                format!(
                    "{}.{:06} {hemisphere}",
                    micro / 1_000_000,
                    micro % 1_000_000
                ),
            );
        }
    }
    if let Some((n, d)) = gps[2]
        .as_ref()
        .and_then(|p| p.first().copied())
        .filter(|(_, d)| *d > 0)
    {
        r.add(
            Gps,
            "Altitude",
            format!("{}{} m", if refs[2] == 1 { "-" } else { "" }, decimal(n, d)),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    fn chunk(kind: &[u8], body: &[u8]) -> Vec<u8> {
        let mut out = be(body.len() as u64, 4);
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out.extend_from_slice(&[0; 4]);
        out
    }
    fn png_file() -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = be(640, 4);
        ihdr.extend(be(480, 4));
        ihdr.extend([8, 6, 0, 0, 0]);
        png.extend(chunk(b"IHDR", &ihdr));
        let mut phys = be(3780, 4);
        phys.extend(be(3780, 4));
        phys.push(1);
        png.extend(chunk(b"pHYs", &phys));
        png.extend(chunk(b"tEXt", b"Title\0Harbour at dusk"));
        png.extend(chunk(b"tEXt", b"Software\0Paint 3D"));
        png.extend(chunk(b"acTL", &[0, 0, 0, 12, 0, 0, 0, 0]));
        png.extend(chunk(b"IDAT", &[0; 64]));
        png.extend(chunk(b"IEND", &[]));
        png
    }
    #[test]
    fn png_header_text_density_and_frames() {
        let r = run("a.png", &png_file());
        assert_eq!(r.format, Some("PNG image"));
        assert_eq!(r.get("Dimensions"), Some("640 x 480"));
        assert_eq!(r.get("Bit depth"), Some("32"));
        assert_eq!(r.get("Color type"), Some("Truecolor with alpha (RGBA)"));
        assert_eq!(r.get("Horizontal resolution"), Some("96 dpi"));
        assert_eq!(r.get("Title"), Some("Harbour at dusk"));
        assert_eq!(r.get("Program name"), Some("Paint 3D"));
        assert_eq!(r.get("Frames"), Some("12"));
        assert_eq!(r.note, None);
        fuzz("a.png", &png_file());
    }
    /// A little-endian Exif block: IFD0 (Make, Model, Orientation, X/Y
    /// resolution, Software, Exif pointer), Exif IFD and a GPS IFD.
    pub(crate) fn exif() -> Vec<u8> {
        let mut t = b"II*\0".to_vec();
        t.extend(le(8, 4));
        let ifd0 = [
            (0x010F, 2, 6, 200),
            (0x0110, 2, 8, 210),
            (0x0112, 3, 1, 6),
            (0x011A, 5, 1, 220),
            (0x011B, 5, 1, 220),
            (0x0131, 2, 5, 230),
            (0x8769, 4, 1, 120),
            (0x8825, 4, 1, 300),
        ];
        t.extend(le(ifd0.len() as u64, 2));
        for (tag, kind, count, value) in ifd0 {
            t.extend(le(tag, 2));
            t.extend(le(kind, 2));
            t.extend(le(count, 4));
            t.extend(le(value, 4));
        }
        t.extend(le(0, 4));
        t.resize(120, 0);
        let exif_ifd = [
            (0x829A, 5, 1, 240),
            (0x829D, 5, 1, 248),
            (0x8827, 3, 1, 200),
            (0x9003, 2, 20, 256),
            (0x920A, 5, 1, 280),
            (0x9209, 3, 1, 0),
        ];
        t.extend(le(exif_ifd.len() as u64, 2));
        for (tag, kind, count, value) in exif_ifd {
            t.extend(le(tag, 2));
            t.extend(le(kind, 2));
            t.extend(le(count, 4));
            t.extend(le(value, 4));
        }
        t.resize(200, 0);
        t.extend(b"Canon\0\0\0\0\0");
        t.resize(210, 0);
        t.extend(b"EOS R6\0\0\0\0");
        t.resize(220, 0);
        t.extend(le(72, 4));
        t.extend(le(1, 4));
        t.resize(230, 0);
        t.extend(b"GIMP\0");
        t.resize(240, 0);
        t.extend(le(1, 4));
        t.extend(le(250, 4));
        t.extend(le(28, 4));
        t.extend(le(10, 4));
        t.resize(256, 0);
        t.extend(b"2024:05:01 10:20:30\0");
        t.resize(280, 0);
        t.extend(le(50, 4));
        t.extend(le(1, 4));
        t.resize(300, 0);
        let gps = [(1, 2, 2, u64::from(b'N')), (2, 5, 3, 360)];
        t.extend(le(gps.len() as u64, 2));
        for (tag, kind, count, value) in gps {
            t.extend(le(tag, 2));
            t.extend(le(kind, 2));
            t.extend(le(count, 4));
            t.extend(le(value, 4));
        }
        t.resize(360, 0);
        for (n, d) in [(47, 1), (36, 1), (2223, 100)] {
            t.extend(le(n, 4));
            t.extend(le(d, 4));
        }
        t
    }
    fn jpeg_file() -> Vec<u8> {
        let mut j = b"\xFF\xD8".to_vec();
        j.extend([0xFF, 0xE0]);
        j.extend(be(16, 2));
        j.extend(b"JFIF\0\x01\x02\x01\0\x60\0\x60\0\0");
        let block = [b"Exif\0\0".to_vec(), exif()].concat();
        j.extend([0xFF, 0xE1]);
        j.extend(be(block.len() as u64 + 2, 2));
        j.extend(block);
        j.extend([0xFF, 0xC2]);
        j.extend(be(17, 2));
        j.extend([8]);
        j.extend(be(3000, 2));
        j.extend(be(4000, 2));
        j.extend([3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
        j.extend([0xFF, 0xDA, 0, 8, 1, 2, 3, 4, 5, 6]);
        j.extend([0x12; 300]);
        j.extend([0xFF, 0xD9]);
        j
    }
    #[test]
    fn jpeg_frame_exif_camera_and_gps() {
        let r = run("p.jpg", &jpeg_file());
        assert_eq!(r.get("Dimensions"), Some("4000 x 3000"));
        assert_eq!(r.get("Bit depth"), Some("24"));
        assert_eq!(r.get("Encoding"), Some("Progressive"));
        assert_eq!(r.get("Camera maker"), Some("Canon"));
        assert_eq!(r.get("Camera model"), Some("EOS R6"));
        assert_eq!(r.get("Orientation"), Some("Rotate 90 degrees clockwise"));
        assert_eq!(r.get("Horizontal resolution"), Some("72 dpi"));
        assert_eq!(r.get("Program name"), Some("GIMP"));
        assert_eq!(r.get("Exposure time"), Some("1/250 sec."));
        assert_eq!(r.get("F-stop"), Some("f/2.8"));
        assert_eq!(r.get("ISO speed"), Some("ISO-200"));
        assert_eq!(r.get("Date taken"), Some("2024-05-01 10:20:30"));
        assert_eq!(r.get("Focal length"), Some("50 mm"));
        assert_eq!(r.get("Flash mode"), Some("No flash"));
        assert_eq!(r.get("Latitude"), Some("47.606175 N"));
        fuzz("p.jpg", &jpeg_file());
    }
    fn gif_file() -> Vec<u8> {
        let mut g = b"GIF89a".to_vec();
        g.extend(le(320, 2));
        g.extend(le(200, 2));
        g.extend([0xF7, 0, 0]);
        g.extend([0; 768]);
        g.extend([0x21, 0xFF, 11]);
        g.extend(b"NETSCAPE2.0");
        g.extend([3, 1, 0, 0, 0]);
        for _ in 0..3 {
            g.extend([0x21, 0xF9, 4, 0, 10, 0, 0, 0]);
            g.extend([0x2C, 0, 0, 0, 0, 0x40, 1, 0xC8, 0, 0, 8, 3, 1, 2, 3, 0]);
        }
        g.push(0x3B);
        g
    }
    #[test]
    fn gif_frames_loops_and_length() {
        let r = run("a.gif", &gif_file());
        assert_eq!(r.get("Dimensions"), Some("320 x 200"));
        assert_eq!(r.get("Bit depth"), Some("8"));
        assert_eq!(r.get("Frames"), Some("3"));
        assert_eq!(r.get("Animation length"), Some("0.30 seconds"));
        assert_eq!(r.get("Animation"), Some("Repeats forever"));
        fuzz("a.gif", &gif_file());
    }
    #[test]
    fn bmp_qoi_pnm_webp_ico_and_tiff_headers() {
        let mut bmp = b"BM".to_vec();
        bmp.extend([0; 12]);
        bmp.extend(le(40, 4));
        bmp.extend(le(800, 4));
        bmp.extend(le((-600i32) as u32 as u64, 4));
        bmp.extend(le(1, 2));
        bmp.extend(le(24, 2));
        bmp.extend(le(0, 4));
        bmp.extend(le(0, 4));
        bmp.extend(le(2835, 4));
        bmp.extend(le(2835, 4));
        bmp.extend([0; 8]);
        let r = run("a.bmp", &bmp);
        assert_eq!(
            (
                r.get("Dimensions"),
                r.get("Bit depth"),
                r.get("Horizontal resolution")
            ),
            (Some("800 x 600"), Some("24"), Some("72 dpi"))
        );
        fuzz("a.bmp", &bmp);
        let qoi = [
            b"qoif".to_vec(),
            be(5, 4),
            be(7, 4),
            alloc::vec![4, 1],
            alloc::vec![0; 8],
        ]
        .concat();
        let r = run("a.qoi", &qoi);
        assert_eq!(
            (
                r.get("Dimensions"),
                r.get("Bit depth"),
                r.get("Color representation")
            ),
            (Some("5 x 7"), Some("32"), Some("Linear"))
        );
        fuzz("a.qoi", &qoi);
        let ppm = b"P6\n# made by hand\n3 2\n65535\n".to_vec();
        let r = run("a.ppm", &[ppm.clone(), alloc::vec![0; 36]].concat());
        assert_eq!(
            (r.get("Dimensions"), r.get("Bit depth"), r.get("Comments")),
            (Some("3 x 2"), Some("48"), Some("made by hand"))
        );
        fuzz("a.ppm", &ppm);
        let mut webp = b"RIFF".to_vec();
        webp.extend(le(30, 4));
        webp.extend(b"WEBPVP8X");
        webp.extend(le(10, 4));
        webp.extend([0x12, 0, 0, 0, 0x7F, 0x07, 0, 0x37, 0x04, 0]);
        webp.extend(b"ANMF");
        webp.extend(le(0, 4));
        let r = run("a.webp", &webp);
        assert_eq!(
            (r.get("Dimensions"), r.get("Transparency"), r.get("Frames")),
            (Some("1920 x 1080"), Some("Yes"), Some("1"))
        );
        fuzz("a.webp", &webp);
        let mut ico = alloc::vec![0, 0, 1, 0, 2, 0];
        ico.extend([16, 16, 0, 0, 1, 0, 32, 0, 0, 4, 0, 0, 38, 0, 0, 0]);
        ico.extend([0, 0, 0, 0, 1, 0, 32, 0, 0, 4, 0, 0, 38, 4, 0, 0]);
        let r = run("app.ico", &ico);
        assert_eq!(
            (r.get("Images"), r.get("Dimensions"), r.get("Sizes")),
            (Some("2"), Some("256 x 256"), Some("16 x 16, 256 x 256"))
        );
        fuzz("app.ico", &ico);
        let tif = exif();
        let r = run("scan.tif", &tif);
        assert_eq!(
            (r.format, r.get("Camera model")),
            (Some("TIFF image"), Some("EOS R6"))
        );
        fuzz("scan.tif", &tif);
    }
}
