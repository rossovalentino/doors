//! Preview's converter (P15): the formats a file can be turned into with
//! codecs Doors writes itself and that need no licence. Images: PNG, JPEG
//! (baseline, 4:2:0), BMP, QOI, TGA, PPM, ICO (PNG inside) and GIF (LZW,
//! dithered 252-colour palette); text: UTF-8 or UTF-16 text, Markdown and
//! HTML; any file into a ZIP (optionally WinZip AES-256), TAR or TAR.GZ
//! (`archive_codec`). Pure functions over decoded pixels and bytes.
extern crate alloc;
use crate::explorer_preview::{Image, Kind};
use crate::media_preview::{
    AC_CHROMA_BITS, AC_CHROMA_VALUES, AC_LUMA_BITS, AC_LUMA_VALUES, DC_BITS, DC_VALUES,
};
use alloc::{format, string::String, vec::Vec};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Png,
    Jpeg,
    Bmp,
    Qoi,
    Tga,
    Ppm,
    Ico,
    Gif,
    Text,
    TextUtf16,
    Markdown,
    Html,
    Zip,
    /// ZIP with every entry encrypted (WinZip AES-256).
    ZipProtected,
    /// 7-Zip (LZMA), and with data and file names encrypted (7zAES).
    SevenZip,
    SevenZipProtected,
    Tar,
    TarGz,
    /// Archives: extract every entry into a new folder.
    Folder,
    /// Audio (decoded WAV, FLAC or MP3): PCM WAV and lossless FLAC.
    Wav,
    Flac,
}
impl Target {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG image",
            Self::Jpeg => "JPEG image",
            Self::Bmp => "Bitmap (BMP)",
            Self::Qoi => "QOI image",
            Self::Tga => "Targa (TGA)",
            Self::Ppm => "Portable pixmap (PPM)",
            Self::Ico => "Icon (ICO)",
            Self::Gif => "GIF image",
            Self::Text => "Text (UTF-8)",
            Self::TextUtf16 => "Text (UTF-16)",
            Self::Markdown => "Markdown",
            Self::Html => "Web page (HTML)",
            Self::Zip => "ZIP archive",
            Self::ZipProtected => "ZIP archive with password (AES-256)",
            Self::SevenZip => "7-Zip archive (7z)",
            Self::SevenZipProtected => "7-Zip archive with password (AES-256)",
            Self::Tar => "TAR archive",
            Self::TarGz => "TAR.GZ archive",
            Self::Folder => "Extract to a folder",
            Self::Wav => "WAV audio",
            Self::Flac => "FLAC audio (lossless)",
        }
    }
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Bmp => "bmp",
            Self::Qoi => "qoi",
            Self::Tga => "tga",
            Self::Ppm => "ppm",
            Self::Ico => "ico",
            Self::Gif => "gif",
            Self::Text | Self::TextUtf16 => "txt",
            Self::Markdown => "md",
            Self::Html => "html",
            Self::Zip | Self::ZipProtected => "zip",
            Self::SevenZip | Self::SevenZipProtected => "7z",
            Self::Tar => "tar",
            Self::TarGz => "tar.gz",
            Self::Folder => "",
            Self::Wav => "wav",
            Self::Flac => "flac",
        }
    }
    pub const fn needs_password(self) -> bool {
        matches!(self, Self::ZipProtected | Self::SevenZipProtected)
    }
}
const IMAGES: [Target; 14] = [
    Target::Png,
    Target::Jpeg,
    Target::Bmp,
    Target::Qoi,
    Target::Tga,
    Target::Ppm,
    Target::Ico,
    Target::Gif,
    Target::Zip,
    Target::ZipProtected,
    Target::SevenZip,
    Target::SevenZipProtected,
    Target::Tar,
    Target::TarGz,
];
const TEXTS: [Target; 10] = [
    Target::Text,
    Target::TextUtf16,
    Target::Markdown,
    Target::Html,
    Target::Zip,
    Target::ZipProtected,
    Target::SevenZip,
    Target::SevenZipProtected,
    Target::Tar,
    Target::TarGz,
];
const AUDIO: [Target; 8] = [
    Target::Flac,
    Target::Wav,
    Target::Zip,
    Target::ZipProtected,
    Target::SevenZip,
    Target::SevenZipProtected,
    Target::Tar,
    Target::TarGz,
];
const PACKED: [Target; 6] = [
    Target::Zip,
    Target::ZipProtected,
    Target::SevenZip,
    Target::SevenZipProtected,
    Target::Tar,
    Target::TarGz,
];
const ARCHIVES: [Target; 7] = [
    Target::Folder,
    Target::Zip,
    Target::ZipProtected,
    Target::SevenZip,
    Target::SevenZipProtected,
    Target::Tar,
    Target::TarGz,
];
/// What a file of `kind` named `name` converts to (its own format left out).
pub fn targets(kind: Kind, name: &str) -> Vec<Target> {
    let ext = name
        .rsplit_once('.')
        .map_or("", |(_, e)| e)
        .to_ascii_lowercase();
    let own = |t: &Target| {
        let e = t.extension();
        e == ext || (e == "jpg" && ext == "jpeg")
    };
    let list: &[Target] = match kind {
        Kind::Image => &IMAGES,
        Kind::Text | Kind::Code => &TEXTS,
        Kind::Archive => &ARCHIVES,
        Kind::Audio => &AUDIO,
        _ => &PACKED,
    };
    // Archives repack into the other archive formats.
    list.iter()
        .copied()
        .filter(|t| {
            !own(t)
                || matches!(
                    t,
                    Target::ZipProtected | Target::SevenZipProtected | Target::TextUtf16
                )
        })
        .collect()
}

/// Straight (un-premultiplied) RGBA of `image`.
fn rgba(image: &Image) -> Vec<u8> {
    image
        .bgra
        .chunks_exact(4)
        .flat_map(|p| {
            let c = |v: u8| {
                if p[3] == 0 {
                    0
                } else {
                    ((u32::from(v) * 255 + u32::from(p[3]) / 2) / u32::from(p[3])).min(255) as u8
                }
            };
            [c(p[2]), c(p[1]), c(p[0]), p[3]]
        })
        .collect()
}
/// RGB composited over white (formats without alpha).
fn rgb(image: &Image) -> Vec<u8> {
    image
        .bgra
        .chunks_exact(4)
        .flat_map(|p| {
            let over = |c: u8| (u32::from(c) + 255 - u32::from(p[3])).min(255) as u8;
            [over(p[2]), over(p[1]), over(p[0])]
        })
        .collect()
}
/// `image` encoded as `target` (JPEG at `quality` 1..=100).
pub fn image(image: &Image, target: Target, quality: u8) -> Result<Vec<u8>, &'static str> {
    let (w, h) = (image.width, image.height);
    if w == 0 || h == 0 || image.bgra.len() != w * h * 4 {
        return Err("The image is empty.");
    }
    match target {
        Target::Png => crate::explorer_preview::encode_png(image),
        Target::Qoi => image.export_qoi(),
        Target::Jpeg => Ok(jpeg(&rgb(image), w, h, quality)),
        Target::Bmp => Ok(bmp(&rgb(image), w, h)),
        Target::Ppm => {
            let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
            out.extend(rgb(image));
            Ok(out)
        }
        Target::Tga => {
            let mut out = alloc::vec![0u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
            out.extend((w as u16).to_le_bytes());
            out.extend((h as u16).to_le_bytes());
            out.extend([32, 0x28]); // 32 bpp, 8 alpha bits, top-left origin
            for p in rgba(image).chunks_exact(4) {
                out.extend([p[2], p[1], p[0], p[3]]);
            }
            Ok(out)
        }
        Target::Ico => {
            let icon = fit_256(image);
            let png = crate::explorer_preview::encode_png(&icon)?;
            let mut out = alloc::vec![0u8, 0, 1, 0, 1, 0];
            out.push((icon.width % 256) as u8);
            out.push((icon.height % 256) as u8);
            out.extend([0, 0, 1, 0, 32, 0]);
            out.extend((png.len() as u32).to_le_bytes());
            out.extend(22u32.to_le_bytes());
            out.extend(png);
            Ok(out)
        }
        Target::Gif => Ok(gif(&rgb(image), w, h)),
        _ => Err("This format isn't an image format."),
    }
}
/// A copy no larger than 256 x 256 (box filter), for ICO.
fn fit_256(image: &Image) -> Image {
    let ratio = image
        .width
        .div_ceil(256)
        .max(image.height.div_ceil(256))
        .max(1);
    let (w, h) = (image.width.div_ceil(ratio), image.height.div_ceil(ratio));
    let mut bgra = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0u32; 4];
            let mut n = 0;
            for sy in y * ratio..((y + 1) * ratio).min(image.height) {
                for sx in x * ratio..((x + 1) * ratio).min(image.width) {
                    let p = &image.bgra[(sy * image.width + sx) * 4..][..4];
                    for c in 0..4 {
                        sum[c] += u32::from(p[c]);
                    }
                    n += 1;
                }
            }
            bgra.extend(sum.map(|s| (s / n) as u8));
        }
    }
    Image {
        width: w,
        height: h,
        source_width: w,
        source_height: h,
        bgra,
    }
}
fn bmp(rgb: &[u8], w: usize, h: usize) -> Vec<u8> {
    let row = (w * 3).div_ceil(4) * 4;
    let size = 54 + row * h;
    let mut b = Vec::with_capacity(size);
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&(size as u32).to_le_bytes());
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(&54u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(w as i32).to_le_bytes());
    b.extend_from_slice(&(h as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(&((row * h) as u32).to_le_bytes());
    b.extend_from_slice(&[0x13, 0x0b, 0, 0, 0x13, 0x0b, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    for y in (0..h).rev() {
        let start = b.len();
        for p in rgb[y * w * 3..(y + 1) * w * 3].chunks_exact(3) {
            b.extend_from_slice(&[p[2], p[1], p[0]]);
        }
        b.resize(start + row, 0);
    }
    b
}

/// Text targets: `title` names an HTML page.
pub fn text(text: &str, target: Target, title: &str) -> Result<Vec<u8>, &'static str> {
    Ok(match target {
        Target::Text | Target::Markdown => text.as_bytes().to_vec(),
        Target::TextUtf16 => {
            let mut out = alloc::vec![0xFF, 0xFE];
            for unit in text.encode_utf16() {
                out.extend(unit.to_le_bytes());
            }
            out
        }
        Target::Html => {
            let escape = |s: &str| {
                let mut out = String::with_capacity(s.len());
                for c in s.chars() {
                    match c {
                        '<' => out.push_str("&lt;"),
                        '>' => out.push_str("&gt;"),
                        '&' => out.push_str("&amp;"),
                        '"' => out.push_str("&quot;"),
                        c => out.push(c),
                    }
                }
                out
            };
            format!(
                "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><title>{}</title></head>\n<body><pre>{}</pre></body></html>\n",
                escape(title),
                escape(text)
            )
            .into_bytes()
        }
        _ => return Err("This format isn't a text format."),
    })
}

// ----------------------------------------------------------------------------
// JPEG (baseline DCT, 4:2:0, the standard Huffman tables of T.81 Annex K).

const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];
const LUMA_Q: [u16; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56,
    14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113,
    92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];
const CHROMA_Q: [u16; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99,
    47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];
/// cos(n * pi / 16) for n in 0..=8; others by symmetry.
const COS16: [f32; 9] = [
    1.0,
    0.980_785_3,
    0.923_879_5,
    0.831_469_6,
    0.707_106_77,
    0.555_570_24,
    0.382_683_43,
    0.195_090_32,
    0.0,
];
fn cos16(n: usize) -> f32 {
    match n % 32 {
        n @ 0..=8 => COS16[n],
        n @ 9..=16 => -COS16[16 - n],
        n @ 17..=24 => -COS16[n - 16],
        n => COS16[32 - n],
    }
}
fn scaled(table: &[u16; 64], quality: u8) -> [u16; 64] {
    let q = u32::from(quality.clamp(1, 100));
    let scale = if q < 50 { 5000 / q } else { 200 - 2 * q };
    table.map(|t| ((u32::from(t) * scale + 50) / 100).clamp(1, 255) as u16)
}
/// Canonical Huffman codes (length, code) per symbol from a DHT table.
fn codes(bits: &[u8; 16], values: &[u8]) -> [(u8, u16); 256] {
    let mut out = [(0u8, 0u16); 256];
    let (mut code, mut k) = (0u16, 0usize);
    for (len, count) in bits.iter().enumerate() {
        for _ in 0..*count {
            out[usize::from(values[k])] = (len as u8 + 1, code);
            code += 1;
            k += 1;
        }
        code <<= 1;
    }
    out
}
struct Bits {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}
impl Bits {
    fn put(&mut self, (len, code): (u8, u16)) {
        self.raw(u32::from(code), u32::from(len));
    }
    fn raw(&mut self, value: u32, len: u32) {
        self.acc = (self.acc << len) | (value & ((1 << len) - 1));
        self.n += len;
        while self.n >= 8 {
            let byte = (self.acc >> (self.n - 8)) as u8;
            self.out.push(byte);
            if byte == 0xFF {
                self.out.push(0);
            }
            self.n -= 8;
        }
    }
    fn flush(&mut self) {
        if self.n > 0 {
            self.raw((1 << (8 - self.n)) - 1, 8 - self.n);
        }
    }
}
/// Magnitude category and bits of a coefficient.
fn category(v: i32) -> (u32, u32) {
    let m = v.unsigned_abs();
    let size = 32 - m.leading_zeros();
    let bits = if v < 0 {
        (v - 1) as u32 & ((1 << size) - 1)
    } else {
        v as u32
    };
    (size, bits)
}
struct Huffman {
    dc: [(u8, u16); 256],
    ac: [(u8, u16); 256],
}
fn encode_block(
    bits: &mut Bits,
    block: &[f32; 64],
    q: &[u16; 64],
    previous: &mut i32,
    table: &Huffman,
) {
    // Forward DCT (separable, straight from the definition).
    let mut f = [0f32; 64];
    for v in 0..8 {
        for u in 0..8 {
            let mut sum = 0.0;
            for y in 0..8 {
                for x in 0..8 {
                    sum += block[y * 8 + x] * cos16((2 * x + 1) * u) * cos16((2 * y + 1) * v);
                }
            }
            let cu = if u == 0 {
                core::f32::consts::FRAC_1_SQRT_2
            } else {
                1.0
            };
            let cv = if v == 0 {
                core::f32::consts::FRAC_1_SQRT_2
            } else {
                1.0
            };
            f[v * 8 + u] = 0.25 * cu * cv * sum;
        }
    }
    let mut z = [0i32; 64];
    for (i, &at) in ZIGZAG.iter().enumerate() {
        let v = f[at] / f32::from(q[at]);
        z[i] = if v >= 0.0 {
            (v + 0.5) as i32
        } else {
            (v - 0.5) as i32
        };
    }
    let diff = z[0] - *previous;
    *previous = z[0];
    let (size, value) = category(diff);
    bits.put(table.dc[size as usize]);
    bits.raw(value, size);
    let mut run = 0;
    for &c in &z[1..] {
        if c == 0 {
            run += 1;
            continue;
        }
        while run > 15 {
            bits.put(table.ac[0xF0]);
            run -= 16;
        }
        let (size, value) = category(c);
        bits.put(table.ac[(run << 4 | size) as usize]);
        bits.raw(value, size);
        run = 0;
    }
    if run > 0 {
        bits.put(table.ac[0x00]);
    }
}
fn jpeg(rgb: &[u8], w: usize, h: usize, quality: u8) -> Vec<u8> {
    let (lq, cq) = (scaled(&LUMA_Q, quality), scaled(&CHROMA_Q, quality));
    let mut out = alloc::vec![0xFF, 0xD8];
    out.extend([0xFF, 0xE0, 0, 16]);
    out.extend(b"JFIF\0");
    out.extend([1, 1, 0, 0, 1, 0, 1, 0, 0]);
    for (id, q) in [(0u8, &lq), (1, &cq)] {
        out.extend([0xFF, 0xDB, 0, 67, id]);
        out.extend(ZIGZAG.iter().map(|&i| q[i] as u8));
    }
    out.extend([0xFF, 0xC0, 0, 17, 8]);
    out.extend((h as u16).to_be_bytes());
    out.extend((w as u16).to_be_bytes());
    out.extend([3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    for (class, bits, values) in [
        (0x00u8, &DC_BITS[0], &DC_VALUES[..]),
        (0x10, &AC_LUMA_BITS, &AC_LUMA_VALUES[..]),
        (0x01, &DC_BITS[1], &DC_VALUES[..]),
        (0x11, &AC_CHROMA_BITS, &AC_CHROMA_VALUES[..]),
    ] {
        out.extend([0xFF, 0xC4]);
        out.extend(((3 + 16 + values.len()) as u16).to_be_bytes());
        out.push(class);
        out.extend(bits);
        out.extend(values);
    }
    out.extend([0xFF, 0xDA, 0, 12, 3, 1, 0x00, 2, 0x11, 3, 0x11, 0, 63, 0]);
    let luma = Huffman {
        dc: codes(&DC_BITS[0], &DC_VALUES),
        ac: codes(&AC_LUMA_BITS, &AC_LUMA_VALUES),
    };
    let chroma = Huffman {
        dc: codes(&DC_BITS[1], &DC_VALUES),
        ac: codes(&AC_CHROMA_BITS, &AC_CHROMA_VALUES),
    };
    let pixel = |x: usize, y: usize| -> (f32, f32, f32) {
        let i = (y.min(h - 1) * w + x.min(w - 1)) * 3;
        let (r, g, b) = (
            f32::from(rgb[i]),
            f32::from(rgb[i + 1]),
            f32::from(rgb[i + 2]),
        );
        (
            0.299 * r + 0.587 * g + 0.114 * b,
            -0.168_736 * r - 0.331_264 * g + 0.5 * b + 128.0,
            0.5 * r - 0.418_688 * g - 0.081_312 * b + 128.0,
        )
    };
    let mut bits = Bits {
        out: Vec::new(),
        acc: 0,
        n: 0,
    };
    let (mut py, mut pcb, mut pcr) = (0, 0, 0);
    for my in (0..h).step_by(16) {
        for mx in (0..w).step_by(16) {
            for (bx, by) in [(0, 0), (8, 0), (0, 8), (8, 8)] {
                let mut block = [0f32; 64];
                for y in 0..8 {
                    for x in 0..8 {
                        block[y * 8 + x] = pixel(mx + bx + x, my + by + y).0 - 128.0;
                    }
                }
                encode_block(&mut bits, &block, &lq, &mut py, &luma);
            }
            let mut cb = [0f32; 64];
            let mut cr = [0f32; 64];
            for y in 0..8 {
                for x in 0..8 {
                    let mut s = (0.0, 0.0);
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (_, b, r) = pixel(mx + 2 * x + dx, my + 2 * y + dy);
                        s = (s.0 + b, s.1 + r);
                    }
                    cb[y * 8 + x] = s.0 / 4.0 - 128.0;
                    cr[y * 8 + x] = s.1 / 4.0 - 128.0;
                }
            }
            encode_block(&mut bits, &cb, &cq, &mut pcb, &chroma);
            encode_block(&mut bits, &cr, &cq, &mut pcr, &chroma);
        }
    }
    bits.flush();
    out.extend(bits.out);
    out.extend([0xFF, 0xD9]);
    out
}

// ----------------------------------------------------------------------------
// GIF (89a, one frame, a 6x7x6 colour cube with Floyd-Steinberg dithering).

fn palette() -> [[u8; 3]; 256] {
    let mut p = [[0u8; 3]; 256];
    let mut i = 0;
    for r in 0..6 {
        for g in 0..7 {
            for b in 0..6 {
                p[i] = [(r * 51) as u8, (g * 255 / 6) as u8, (b * 51) as u8];
                i += 1;
            }
        }
    }
    p
}
fn nearest(c: [i32; 3]) -> usize {
    let r = ((c[0].clamp(0, 255) + 25) / 51) as usize;
    let g = ((c[1].clamp(0, 255) * 6 + 127) / 255) as usize;
    let b = ((c[2].clamp(0, 255) + 25) / 51) as usize;
    r * 42 + g * 6 + b
}
/// Palette indices of `rgb` with error diffusion.
fn quantize(rgb: &[u8], w: usize, h: usize) -> Vec<u8> {
    let pal = palette();
    let mut err = alloc::vec![[0i32; 3]; (w + 2) * 2];
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        let (cur, next) = if y % 2 == 0 { (0, w + 2) } else { (w + 2, 0) };
        for e in &mut err[next..next + w + 2] {
            *e = [0; 3];
        }
        for x in 0..w {
            let i = (y * w + x) * 3;
            let e = err[cur + x + 1];
            let want = [
                i32::from(rgb[i]) + e[0] / 16,
                i32::from(rgb[i + 1]) + e[1] / 16,
                i32::from(rgb[i + 2]) + e[2] / 16,
            ];
            let k = nearest(want);
            out.push(k as u8);
            let d = [
                want[0] - i32::from(pal[k][0]),
                want[1] - i32::from(pal[k][1]),
                want[2] - i32::from(pal[k][2]),
            ];
            for c in 0..3 {
                err[cur + x + 2][c] += d[c] * 7;
                err[next + x][c] += d[c] * 3;
                err[next + x + 1][c] += d[c] * 5;
                err[next + x + 2][c] += d[c];
            }
        }
    }
    out
}
/// LZW codes of `indices` (minimum code size 8), packed LSB first.
fn lzw(indices: &[u8]) -> Vec<u8> {
    const CLEAR: u32 = 256;
    const END: u32 = 257;
    let mut table: alloc::collections::BTreeMap<(u32, u8), u32> =
        alloc::collections::BTreeMap::new();
    let (mut next, mut size) = (258u32, 9u32);
    let mut out = Vec::new();
    let (mut acc, mut n) = (0u32, 0u32);
    let mut emit = |code: u32, size: u32, out: &mut Vec<u8>| {
        acc |= code << n;
        n += size;
        while n >= 8 {
            out.push(acc as u8);
            acc >>= 8;
            n -= 8;
        }
    };
    emit(CLEAR, size, &mut out);
    let Some((&first, rest)) = indices.split_first() else {
        emit(END, size, &mut out);
        if n > 0 {
            out.push(acc as u8);
        }
        return out;
    };
    let mut prefix = u32::from(first);
    for &k in rest {
        if let Some(&code) = table.get(&(prefix, k)) {
            prefix = code;
            continue;
        }
        emit(prefix, size, &mut out);
        if next == 4096 {
            emit(CLEAR, size, &mut out);
            table.clear();
            next = 258;
            size = 9;
        } else {
            table.insert((prefix, k), next);
            if next == 1 << size && size < 12 {
                size += 1;
            }
            next += 1;
        }
        prefix = u32::from(k);
    }
    emit(prefix, size, &mut out);
    emit(END, size, &mut out);
    if n > 0 {
        out.push(acc as u8);
    }
    out
}
fn gif(rgb: &[u8], w: usize, h: usize) -> Vec<u8> {
    let mut out = b"GIF89a".to_vec();
    out.extend((w as u16).to_le_bytes());
    out.extend((h as u16).to_le_bytes());
    out.extend([0xF7, 0, 0]); // global table, 8-bit colour, 256 entries
    for c in palette() {
        out.extend(c);
    }
    out.push(0x2C);
    out.extend([0, 0, 0, 0]);
    out.extend((w as u16).to_le_bytes());
    out.extend((h as u16).to_le_bytes());
    out.push(0);
    out.push(8);
    for block in lzw(&quantize(rgb, w, h)).chunks(255) {
        out.push(block.len() as u8);
        out.extend(block);
    }
    out.extend([0, 0x3B]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn gradient(w: usize, h: usize) -> Image {
        let mut bgra = Vec::new();
        for y in 0..h {
            for x in 0..w {
                bgra.extend([(x * 255 / w) as u8, (y * 255 / h) as u8, 128, 255]);
            }
        }
        Image {
            width: w,
            height: h,
            source_width: w,
            source_height: h,
            bgra,
        }
    }
    #[test]
    fn jpeg_decodes_back_close_to_the_source() {
        let img = gradient(70, 45);
        let jpg = image(&img, Target::Jpeg, 90).unwrap();
        let back = crate::explorer_preview::decode_full(&jpg).unwrap();
        assert_eq!((back.width, back.height), (70, 45));
        let error: u64 = img
            .bgra
            .iter()
            .zip(&back.bgra)
            .map(|(a, b)| u64::from(a.abs_diff(*b)))
            .sum::<u64>()
            / img.bgra.len() as u64;
        assert!(error <= 4, "mean error {error}");
    }
    #[test]
    fn lossless_targets_round_trip_through_the_preview_decoder() {
        let img = gradient(33, 20);
        for target in [Target::Png, Target::Bmp, Target::Qoi] {
            let bytes = image(&img, target, 90).unwrap();
            let back = crate::explorer_preview::decode_full(&bytes).unwrap();
            assert_eq!(back.bgra, img.bgra, "{target:?}");
        }
        let ppm = image(&img, Target::Ppm, 90).unwrap();
        assert!(
            ppm.starts_with(
                b"P6
33 20
255
"
            ) && ppm.len() == 13 + 33 * 20 * 3
        );
        let tga = image(&img, Target::Tga, 90).unwrap();
        assert_eq!(tga.len(), 18 + 33 * 20 * 4);
        assert_eq!(&tga[18..22], &img.bgra[..4]);
        let ico = image(&gradient(512, 10), Target::Ico, 90).unwrap();
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 1, 0]);
        assert_eq!(ico[6], 0, "256 wide is stored as 0");
        assert!(ico[22..].starts_with(b"\x89PNG"));
    }
    #[test]
    fn gif_lzw_decodes_back_to_the_quantized_indices() {
        let img = gradient(40, 30);
        let indices = quantize(&rgb(&img), 40, 30);
        let packed = lzw(&indices);
        // A minimal LZW decoder (GIF variant).
        let (mut table, mut out): (Vec<Vec<u8>>, Vec<u8>) = (Vec::new(), Vec::new());
        let reset = |t: &mut Vec<Vec<u8>>| {
            t.clear();
            t.extend((0..=255u8).map(|b| alloc::vec![b]));
            t.push(Vec::new());
            t.push(Vec::new());
        };
        reset(&mut table);
        let (mut size, mut acc, mut n, mut prev): (u32, u32, u32, Option<Vec<u8>>) =
            (9, 0, 0, None);
        let mut bytes = packed.iter();
        loop {
            while n < size {
                acc |= u32::from(*bytes.next().unwrap()) << n;
                n += 8;
            }
            let code = acc & ((1 << size) - 1);
            acc >>= size;
            n -= size;
            if code == 256 {
                reset(&mut table);
                size = 9;
                prev = None;
                continue;
            }
            if code == 257 {
                break;
            }
            let entry = if (code as usize) < table.len() {
                table[code as usize].clone()
            } else {
                let p = prev.clone().unwrap();
                let mut e = p.clone();
                e.push(p[0]);
                e
            };
            out.extend(&entry);
            if let Some(p) = prev {
                let mut e = p;
                e.push(entry[0]);
                table.push(e);
                if table.len() == 1 << size && size < 12 {
                    size += 1;
                }
            }
            prev = Some(entry);
        }
        assert_eq!(out, indices);
        let g = image(&img, Target::Gif, 90).unwrap();
        assert!(g.starts_with(b"GIF89a") && g.ends_with(&[0, 0x3B]));
    }
    #[test]
    fn text_targets_and_the_target_lists() {
        assert_eq!(
            text("a<b", Target::TextUtf16, "t").unwrap(),
            [0xFF, 0xFE, b'a', 0, b'<', 0, b'b', 0]
        );
        let html = String::from_utf8(text("1 < 2 & 3", Target::Html, "n<1").unwrap()).unwrap();
        assert!(
            html.contains("<pre>1 &lt; 2 &amp; 3</pre>") && html.contains("<title>n&lt;1</title>")
        );
        let t = targets(Kind::Image, "photo.jpeg");
        assert!(!t.contains(&Target::Jpeg) && t.contains(&Target::Png) && t.contains(&Target::Gif));
        assert!(targets(Kind::Text, "a.txt").contains(&Target::Html));
        assert!(targets(Kind::Archive, "a.zip").contains(&Target::ZipProtected));
        assert!(!targets(Kind::Archive, "a.zip").contains(&Target::Zip));
        assert_eq!(targets(Kind::Archive, "a.tar")[0], Target::Folder);
    }
}
