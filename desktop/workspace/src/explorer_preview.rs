//! Bounded, read-only preview decoding. No service calls or painting here.
extern crate alloc;
use alloc::{format, string::String, vec::Vec};
use zune_jpeg::zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions};

pub const MAX_INPUT: usize = 1024 * 1024;
/// Separate standalone viewer input ceiling, including its own lossless exports.
pub const MAX_VIEW_INPUT: usize = 6 * 1024 * 1024;
pub const MAX_TEXT: usize = 16 * 1024;
pub const MAX_PIXELS: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    Image,
    Text,
    Code,
    Pdf,
    Document,
    Spreadsheet,
    Presentation,
    Video,
    Audio,
    Archive,
    Model,
    File,
}
impl Kind {
    pub fn of(name: &str, directory: bool) -> Self {
        if directory {
            return Self::Folder;
        }
        let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
        let is = |values: &[&str]| values.iter().any(|v| ext.eq_ignore_ascii_case(v));
        if is(&["jpg", "jpeg", "png", "qoi", "bmp", "ppm", "pnm"]) {
            Self::Image
        } else if is(&["txt", "md", "log", "csv", "tsv", "ini"]) {
            Self::Text
        } else if is(&[
            "json", "rs", "toml", "xml", "yaml", "yml", "js", "ts", "tsx", "jsx", "py", "c", "h",
            "cpp", "css", "html", "svg", "sh", "bat", "ps1", "psm1", "psd1", "ps1xml",
        ]) {
            Self::Code
        } else if is(&["pdf"]) {
            Self::Pdf
        } else if is(&["doc", "docx", "odt", "rtf"]) {
            Self::Document
        } else if is(&["xlsx", "xls", "ods"]) {
            Self::Spreadsheet
        } else if is(&["ppt", "pptx", "odp"]) {
            Self::Presentation
        } else if is(&["mp4", "m4v", "mkv", "webm", "avi", "mov"]) {
            Self::Video
        } else if is(&["mp3", "wav", "flac", "ogg", "opus", "m4a"]) {
            Self::Audio
        } else if is(&["zip", "7z", "tar", "gz", "tgz", "rar"]) {
            Self::Archive
        } else if is(&["glb", "gltf", "obj", "stl", "fbx"]) {
            Self::Model
        } else {
            Self::File
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Folder => "File folder",
            Self::Image => "Image",
            Self::Text => "Text document",
            Self::Code => "Source file",
            Self::Pdf => "PDF document",
            Self::Document => "Document",
            Self::Spreadsheet => "Spreadsheet",
            Self::Presentation => "Presentation",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Archive => "Archive",
            Self::Model => "3D model",
            Self::File => "File",
        }
    }
    pub fn previewable(self) -> bool {
        matches!(
            self,
            Self::Image | Self::Text | Self::Code | Self::Audio | Self::Video | Self::Archive
        )
    }
    pub fn unavailable(self) -> &'static str {
        match self {
            Self::Pdf => "PDF preview is not available yet.",
            Self::Document | Self::Spreadsheet | Self::Presentation => {
                "Document preview is not available yet."
            }
            Self::Model => "3D preview is not available yet.",
            _ => "No preview available for this file.",
        }
    }
}
/// File Explorer's Type column: the friendly name Windows 11 26H2 gives a
/// type's default association (Doors for Windows), else Windows' "EXT File",
/// and "File" for a name without an extension.
pub fn type_name(name: &str, directory: bool) -> String {
    if directory {
        return String::from("File folder");
    }
    let Some((_, ext)) = name
        .rsplit_once('.')
        .filter(|(stem, e)| !stem.is_empty() && !e.is_empty())
    else {
        return String::from("File");
    };
    let known = match ext.to_ascii_lowercase().as_str() {
        "exe" | "com" => "Application",
        "bat" => "Doors Batch File",
        "cmd" => "Doors Command Script",
        "msi" => "Doors Installer Package",
        "msp" => "Doors Installer Patch",
        "dll" => "Application extension",
        "sys" => "System file",
        "drv" => "Device driver",
        "ocx" => "ActiveX control",
        "cpl" => "Control panel item",
        "scr" => "Screen saver",
        "ini" => "Configuration settings",
        "inf" => "Setup Information",
        "reg" => "Registration Entries",
        "ps1" => "Windows PowerShell Script",
        "psm1" => "Windows PowerShell Script Module",
        "psd1" => "Windows PowerShell Data File",
        "ps1xml" => "Windows PowerShell XML Document",
        "lnk" => "Shortcut",
        "url" => "Internet Shortcut",
        "ttf" | "ttc" => "TrueType font file",
        "otf" => "OpenType font file",
        "fon" => "Font file",
        "iso" | "img" => "Disc Image File",
        "vhd" | "vhdx" => "Hard Disk Image File",
        "cab" => "Cabinet File",
        "zip" => "Compressed (zipped) Folder",
        "7z" | "rar" | "tar" | "gz" | "tgz" => "Compressed Archive Folder",
        "txt" | "log" => "Text Document",
        "pdf" => "PDF Document",
        "htm" | "html" => "HTML Document",
        "cer" | "crt" => "Security Certificate",
        "pfx" => "Personal Information Exchange",
        "theme" => "Doors Theme File",
        _ => return format!("{} File", ext.to_ascii_uppercase()),
    };
    String::from(known)
}
pub fn size(bytes: u64) -> String {
    for (unit, scale) in [("GB", 1u64 << 30), ("MB", 1 << 20), ("KB", 1 << 10)] {
        if bytes >= scale {
            return format!(
                "{}.{:01} {unit}",
                bytes / scale,
                (bytes % scale) * 10 / scale
            );
        }
    }
    format!("{bytes} bytes")
}
#[derive(Debug)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub source_width: usize,
    pub source_height: usize,
    pub bgra: Vec<u8>,
}
fn extent(w: usize, h: usize) -> Result<(), &'static str> {
    if w == 0 || h == 0 || w > 4096 || h > 4096 || w.checked_mul(h).is_none_or(|n| n > MAX_PIXELS) {
        Err("Image exceeds the 1 megapixel preview limit.")
    } else {
        Ok(())
    }
}
/// Box-filter to a bounded cache once. Alpha is premultiplied before filtering.
fn raster(
    w: usize,
    h: usize,
    pixels: &[u8],
    components: usize,
    downsample: bool,
) -> Result<Image, &'static str> {
    extent(w, h)?;
    if !(1..=4).contains(&components) || pixels.len() != w * h * components {
        return Err("Invalid image pixels.");
    }
    let ratio = if downsample {
        w.div_ceil(480).max(h.div_ceil(320)).max(1)
    } else {
        1
    };
    let (width, height) = (w.div_ceil(ratio), h.div_ceil(ratio));
    let mut bgra = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let mut sum = [0u32; 4];
            let mut count = 0;
            for sy in y * ratio..((y + 1) * ratio).min(h) {
                for sx in x * ratio..((x + 1) * ratio).min(w) {
                    let p = &pixels[(sy * w + sx) * components..][..components];
                    let (r, g, b, a) = match components {
                        1 => (p[0], p[0], p[0], 255),
                        2 => (p[0], p[0], p[0], p[1]),
                        3 => (p[0], p[1], p[2], 255),
                        _ => (p[0], p[1], p[2], p[3]),
                    };
                    for (i, c) in [b, g, r].into_iter().enumerate() {
                        sum[i] += (u32::from(c) * u32::from(a) + 127) / 255;
                    }
                    sum[3] += u32::from(a);
                    count += 1;
                }
            }
            bgra.extend(sum.map(|s| (s / count) as u8));
        }
    }
    Ok(Image {
        width,
        height,
        source_width: w,
        source_height: h,
        bgra,
    })
}
pub fn decode(bytes: &[u8]) -> Result<Image, &'static str> {
    decode_impl(bytes, true, MAX_INPUT)
}
/// Full bounded pixels for the standalone viewer, never a scaled thumbnail.
pub fn decode_full(bytes: &[u8]) -> Result<Image, &'static str> {
    decode_impl(bytes, false, MAX_VIEW_INPUT)
}
fn decode_impl(bytes: &[u8], downsample: bool, input_limit: usize) -> Result<Image, &'static str> {
    if bytes.len() > input_limit {
        return Err("File exceeds the encoded image preview limit.");
    }
    let options = DecoderOptions::default()
        .set_max_width(4096)
        .set_max_height(4096)
        .set_use_unsafe(false)
        .inflate_set_limit(MAX_PIXELS * 8 + 32768)
        .set_strict_mode(true)
        .jpeg_set_max_scans(24)
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .png_set_confirm_crc(true)
        .png_set_decode_animated(false)
        .png_set_strip_to_8bit(true);
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let safe = png_image_chunks(bytes)?;
        let mut d = zune_png::PngDecoder::new_with_options(ZCursor::new(&safe), options);
        d.decode_headers().map_err(|_| "Invalid PNG image.")?;
        let (w, h) = d.dimensions().ok_or("Missing image dimensions.")?;
        extent(w, h)?;
        let n = d.output_buffer_size().ok_or("Invalid PNG size.")?;
        if n > MAX_PIXELS * 4 {
            return Err("Image exceeds preview memory limit.");
        }
        let rgba = d
            .decode_raw()
            .map_err(|_| "Cannot decode this PNG image.")?;
        raster(w, h, &rgba, rgba.len() / (w * h), downsample)
    } else if bytes.starts_with(&[255, 216]) {
        let mut d = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
        d.decode_headers().map_err(|_| "Invalid JPEG image.")?;
        let (w, h) = d.dimensions().ok_or("Missing image dimensions.")?;
        extent(w, h)?;
        let rgba = d.decode().map_err(|_| "Cannot decode this JPEG image.")?;
        raster(w, h, &rgba, rgba.len() / (w * h), downsample)
    } else if bytes.starts_with(b"qoif") {
        let header = qoi::decode_header(bytes).map_err(|_| "Invalid QOI image.")?;
        extent(header.width as usize, header.height as usize)?;
        let (_, rgba) = qoi::decode_to_vec(bytes).map_err(|_| "Cannot decode this QOI image.")?;
        raster(
            header.width as usize,
            header.height as usize,
            &rgba,
            header.channels.as_u8() as usize,
            downsample,
        )
    } else if bytes.starts_with(b"P3") {
        ppm(bytes, downsample)
    } else if bytes.starts_with(b"BM") {
        bmp(bytes, downsample)
    } else {
        Err("This image format is not supported. Use PNG, JPEG, BMP or QOI.")
    }
}
fn ppm(bytes: &[u8], downsample: bool) -> Result<Image, &'static str> {
    let source = core::str::from_utf8(bytes).map_err(|_| "Invalid PPM text.")?;
    let mut words = source
        .lines()
        .map(|l| l.split('#').next().unwrap_or(""))
        .flat_map(str::split_whitespace);
    if words.next() != Some("P3") {
        return Err("Unsupported PPM encoding.");
    }
    let w: usize = words
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or("Invalid PPM width.")?;
    let h: usize = words
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or("Invalid PPM height.")?;
    extent(w, h)?;
    let max: u32 = words
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or("Invalid PPM range.")?;
    if max == 0 || max > 65535 {
        return Err("Invalid PPM range.");
    }
    let mut rgb = Vec::with_capacity(w * h * 3);
    for _ in 0..w * h * 3 {
        let n: u32 = words
            .next()
            .and_then(|s| s.parse().ok())
            .filter(|n| *n <= max)
            .ok_or("Invalid PPM sample.")?;
        rgb.push(((n * 255 + max / 2) / max) as u8);
    }
    if words.next().is_some() {
        return Err("Unexpected PPM samples.");
    }
    raster(w, h, &rgb, 3, downsample)
}
fn bmp(bytes: &[u8], downsample: bool) -> Result<Image, &'static str> {
    if bytes.len() < 54 {
        return Err("Truncated BMP header.");
    }
    let u32at = |at| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let (offset, header, w, h) = (
        u32at(10) as usize,
        u32at(14),
        u32at(18) as i32,
        u32at(22) as i32,
    );
    let bits = u16::from_le_bytes(bytes[28..30].try_into().unwrap());
    if header < 40
        || offset < 14 + header as usize
        || w <= 0
        || h == 0
        || h == i32::MIN
        || !matches!(bits, 24 | 32)
        || bytes[26..28] != [1, 0]
        || u32at(30) != 0
    {
        return Err("Unsupported BMP layout. Use uncompressed 24 or 32 bit RGB.");
    }
    let (width, height) = (w as usize, h.unsigned_abs() as usize);
    extent(width, height)?;
    let row = (width * (bits as usize / 8)).div_ceil(4) * 4;
    if offset
        .checked_add(row * height)
        .is_none_or(|end| end > bytes.len())
    {
        return Err("Truncated BMP pixels.");
    }
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let sy = if h > 0 { height - y - 1 } else { y };
        for x in 0..width {
            let at = offset + sy * row + x * (bits as usize / 8);
            rgba.extend_from_slice(&[bytes[at + 2], bytes[at + 1], bytes[at], 255]);
        }
    }
    raster(width, height, &rgba, 4, downsample)
}
impl Image {
    pub fn rotate_clockwise(&mut self) {
        let mut output = alloc::vec![0;self.bgra.len()];
        for y in 0..self.height {
            for x in 0..self.width {
                let from = (y * self.width + x) * 4;
                let to = (x * self.height + self.height - y - 1) * 4;
                output[to..to + 4].copy_from_slice(&self.bgra[from..from + 4]);
            }
        }
        core::mem::swap(&mut self.width, &mut self.height);
        core::mem::swap(&mut self.source_width, &mut self.source_height);
        self.bgra = output;
    }
    pub fn flip_horizontal(&mut self) {
        for y in 0..self.height {
            for x in 0..self.width / 2 {
                let a = (y * self.width + x) * 4;
                let b = (y * self.width + self.width - x - 1) * 4;
                for c in 0..4 {
                    self.bgra.swap(a + c, b + c);
                }
            }
        }
    }
    pub fn export_qoi(&self) -> Result<Vec<u8>, &'static str> {
        let rgba: Vec<_> = self
            .bgra
            .chunks_exact(4)
            .flat_map(|p| {
                let channel = |c: u8| {
                    if p[3] == 0 {
                        0
                    } else {
                        ((u32::from(c) * 255 + u32::from(p[3]) / 2) / u32::from(p[3])).min(255)
                            as u8
                    }
                };
                [channel(p[2]), channel(p[1]), channel(p[0]), p[3]]
            })
            .collect();
        qoi::encode_to_vec(&rgba, self.width as u32, self.height as u32)
            .map_err(|_| "Image encoding failed.")
    }
}
/// Validate every claimed chunk before a decoder can allocate it. Ancillary
/// metadata (including compressed profiles/text and animation) is unnecessary
/// for a first-frame preview and is never passed to a decompressor.
fn png_image_chunks(bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut output = Vec::from(&bytes[..8]);
    let mut at = 8usize;
    let mut count = 0;
    while at < bytes.len() {
        let head = bytes.get(at..at + 8).ok_or("Truncated PNG chunk.")?;
        let length = u32::from_be_bytes(head[..4].try_into().unwrap()) as usize;
        let end = at
            .checked_add(12)
            .and_then(|n| n.checked_add(length))
            .filter(|n| *n <= bytes.len())
            .ok_or("Invalid PNG chunk length.")?;
        let kind = &head[4..8];
        if at == 8 {
            if kind != b"IHDR" || length != 13 {
                return Err("Invalid PNG header.");
            }
            let w = u32::from_be_bytes(bytes[at + 8..at + 12].try_into().unwrap()) as usize;
            let h = u32::from_be_bytes(bytes[at + 12..at + 16].try_into().unwrap()) as usize;
            extent(w, h)?;
        }
        if [b"IHDR", b"PLTE", b"tRNS", b"IDAT", b"IEND"].contains(&kind.try_into().unwrap()) {
            output.extend_from_slice(&bytes[at..end]);
        } else if kind[0].is_ascii_uppercase() {
            return Err("Unknown critical PNG chunk.");
        }
        count += 1;
        if count > 4096 {
            return Err("Too many PNG chunks.");
        }
        at = end;
        if kind == b"IEND" {
            if length != 0 || at != bytes.len() {
                return Err("Invalid PNG ending.");
            }
            return Ok(output);
        }
    }
    Err("Missing PNG ending.")
}
/// Result of reading image dimensions from the leading bytes of a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Header {
    Size(u32, u32),
    /// More bytes are needed (JPEG metadata can precede the frame header).
    NeedMore,
    Unsupported,
}
/// Image dimensions from the file header only: PNG, JPEG (SOFn), BMP, QOI,
/// GIF, PPM/PGM (P1-P6) and WebP (VP8, VP8L, VP8X). No pixels are decoded.
pub fn dimensions(bytes: &[u8]) -> Header {
    let be32 = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
    };
    let le32 = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    };
    let le16 = |at: usize| {
        bytes
            .get(at..at + 2)
            .map(|b| u32::from(u16::from_le_bytes(b.try_into().unwrap())))
    };
    let need = |v: Option<(u32, u32)>| v.map_or(Header::NeedMore, |(w, h)| Header::Size(w, h));
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        if bytes.len() >= 16 && &bytes[12..16] != b"IHDR" {
            return Header::Unsupported;
        }
        return need(be32(16).zip(be32(20)));
    }
    if bytes.starts_with(b"qoif") {
        return need(be32(4).zip(be32(8)));
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return need(le16(6).zip(le16(8)));
    }
    if bytes.starts_with(b"BM") {
        return need(
            le32(18)
                .zip(le32(22))
                .map(|(w, h)| (w, (h as i32).unsigned_abs())),
        );
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return match bytes.get(12..16) {
            Some(b"VP8 ") => need(
                le16(26)
                    .zip(le16(28))
                    .map(|(w, h)| (w & 0x3fff, h & 0x3fff)),
            ),
            Some(b"VP8L") => need(le32(21).map(|b| ((b & 0x3fff) + 1, ((b >> 14) & 0x3fff) + 1))),
            Some(b"VP8X") => need(bytes.get(24..30).map(|b| {
                (
                    (u32::from(b[0]) | u32::from(b[1]) << 8 | u32::from(b[2]) << 16) + 1,
                    (u32::from(b[3]) | u32::from(b[4]) << 8 | u32::from(b[5]) << 16) + 1,
                )
            })),
            Some(_) => Header::Unsupported,
            None => Header::NeedMore,
        };
    }
    if bytes.len() >= 2 && bytes[0] == b'P' && (b'1'..=b'6').contains(&bytes[1]) {
        let text = core::str::from_utf8(&bytes[..bytes.len().min(512)]).unwrap_or("");
        let mut words = text
            .lines()
            .map(|l| l.split('#').next().unwrap_or(""))
            .flat_map(str::split_whitespace)
            .skip(1);
        return match (
            words.next().map(str::parse::<u32>),
            words.next().map(str::parse::<u32>),
        ) {
            (Some(Ok(w)), Some(Ok(h)))
                if words.next().is_some() || bytes[1] == b'1' || bytes[1] == b'4' =>
            {
                Header::Size(w, h)
            }
            (Some(Err(_)), _) | (_, Some(Err(_))) => Header::Unsupported,
            _ if bytes.len() >= 512 => Header::Unsupported,
            _ => Header::NeedMore,
        };
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        let mut at = 2;
        loop {
            let Some(&[marker_start, marker]) = bytes
                .get(at..at + 2)
                .map(|b| <&[u8; 2]>::try_from(b).unwrap())
            else {
                return Header::NeedMore;
            };
            if marker_start != 0xff {
                return Header::Unsupported;
            }
            if marker == 0xff {
                at += 1;
                continue;
            }
            if marker == 0xd8 || (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
                at += 2;
                continue;
            }
            let Some(length) = bytes
                .get(at + 2..at + 4)
                .map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])))
            else {
                return Header::NeedMore;
            };
            if length < 2 || marker == 0xd9 || marker == 0xda {
                return Header::Unsupported;
            }
            let sof = (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc);
            if sof {
                return match bytes.get(at + 5..at + 9) {
                    Some(b) => Header::Size(
                        u32::from(u16::from_be_bytes([b[2], b[3]])),
                        u32::from(u16::from_be_bytes([b[0], b[1]])),
                    ),
                    None => Header::NeedMore,
                };
            }
            at += 2 + length;
        }
    }
    Header::Unsupported
}
/// Lines in a text file: newline count, plus a final unterminated line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineCount {
    pub newlines: u64,
    pub bytes: u64,
    pub last: u8,
}
impl LineCount {
    pub fn feed(&mut self, chunk: &[u8]) {
        self.newlines += chunk.iter().filter(|b| **b == b'\n').count() as u64;
        self.bytes += chunk.len() as u64;
        if let Some(b) = chunk.last() {
            self.last = *b;
        }
    }
    pub fn lines(&self) -> u64 {
        self.newlines + u64::from(self.bytes > 0 && self.last != b'\n')
    }
}
/// Original bounded PNG writer. Stored DEFLATE blocks avoid a new codec runtime;
/// alpha and dimensions are preserved, with no invented source metadata.
pub fn encode_png(image: &Image) -> Result<Vec<u8>, &'static str> {
    if image.width == 0
        || image.height == 0
        || image.width > 4096
        || image.height > 4096
        || image.width * image.height > MAX_PIXELS
        || image.bgra.len() != image.width * image.height * 4
    {
        return Err("Invalid export image.");
    }
    const fn table() -> [u32; 256] {
        let mut t = [0; 256];
        let mut i = 0;
        while i < 256 {
            let mut c = i as u32;
            let mut n = 0;
            while n < 8 {
                c = if c & 1 != 0 {
                    0xedb88320 ^ (c >> 1)
                } else {
                    c >> 1
                };
                n += 1;
            }
            t[i] = c;
            i += 1;
        }
        t
    }
    const CRC: [u32; 256] = table();
    fn checksum(bytes: &[u8]) -> u32 {
        let mut c = !0u32;
        for &byte in bytes {
            c = CRC[((c ^ byte as u32) & 255) as usize] ^ (c >> 8);
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        let crc = checksum(&out[start..]);
        out.extend_from_slice(&crc.to_be_bytes());
    }
    let stride = image.width * 4 + 1;
    let raw = stride * image.height;
    let size = raw + raw.div_ceil(65535) * 5 + 6;
    let mut out = Vec::with_capacity(size + 57);
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut header = [0; 13];
    header[..4].copy_from_slice(&(image.width as u32).to_be_bytes());
    header[4..8].copy_from_slice(&(image.height as u32).to_be_bytes());
    header[8] = 8;
    header[9] = 6;
    chunk(&mut out, b"IHDR", &header);
    out.extend_from_slice(&(size as u32).to_be_bytes());
    let crc_start = out.len();
    out.extend_from_slice(b"IDAT");
    out.extend_from_slice(&[0x78, 0x01]);
    let (mut offset, mut a, mut b) = (0usize, 1u32, 0u32);
    while offset < raw {
        let count = (raw - offset).min(65535);
        out.push(u8::from(offset + count == raw));
        out.extend_from_slice(&(count as u16).to_le_bytes());
        out.extend_from_slice(&(!(count as u16)).to_le_bytes());
        for at in offset..offset + count {
            let column = at % stride;
            let value = if column == 0 {
                0
            } else {
                let component = (column - 1) % 4;
                let p = &image.bgra[(at / stride * image.width + (column - 1) / 4) * 4..][..4];
                if component == 3 {
                    p[3]
                } else if p[3] == 0 {
                    0
                } else {
                    ((p[[2, 1, 0][component]] as u32 * 255 + p[3] as u32 / 2) / p[3] as u32)
                        .min(255) as u8
                }
            };
            out.push(value);
            a = (a + value as u32) % 65521;
            b = (b + a) % 65521;
        }
        offset += count;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let crc = checksum(&out[crc_start..]);
    out.extend_from_slice(&crc.to_be_bytes());
    chunk(&mut out, b"IEND", &[]);
    Ok(out)
}
#[cfg(test)]
mod header_tests {
    use super::*;
    #[test]
    fn dimensions_come_from_headers_and_partial_input_asks_for_more() {
        let png = include_bytes!("../tests/fixtures/preview-alpha.png");
        assert_eq!(dimensions(png), Header::Size(32, 16));
        assert_eq!(dimensions(&png[..20]), Header::NeedMore);
        let jpg = include_bytes!("../tests/fixtures/preview-color.jpg");
        assert_eq!(dimensions(jpg), Header::Size(32, 16));
        for n in 2..jpg.len().min(400) {
            assert!(
                matches!(
                    dimensions(&jpg[..n]),
                    Header::NeedMore | Header::Size(32, 16)
                ),
                "{n}"
            );
        }
        let qoi = qoi::encode_to_vec([1, 2, 3, 255, 1, 2, 3, 255], 2, 1).unwrap();
        assert_eq!(dimensions(&qoi), Header::Size(2, 1));
        assert_eq!(
            dimensions(b"P6\n# c\n640 480\n255\n"),
            Header::Size(640, 480)
        );
        assert_eq!(dimensions(b"GIF89a\x10\x00\x20\x00"), Header::Size(16, 32));
        let mut bmp = alloc::vec![0u8; 30];
        bmp[..2].copy_from_slice(b"BM");
        bmp[18..22].copy_from_slice(&7u32.to_le_bytes());
        bmp[22..26].copy_from_slice(&(-9i32).to_le_bytes());
        assert_eq!(dimensions(&bmp), Header::Size(7, 9));
        assert_eq!(dimensions(b"plain text"), Header::Unsupported);
        let mut lines = LineCount::default();
        lines.feed(b"a\nb");
        assert_eq!(lines.lines(), 2);
        lines.feed(b"\n");
        assert_eq!(lines.lines(), 2);
        assert_eq!(LineCount::default().lines(), 0);
    }
}
/// Preserve decoded text for viewers/editors. Display fallback is a painter concern.
pub fn text_document(bytes: &[u8]) -> Result<String, &'static str> {
    if bytes.len() > MAX_TEXT {
        return Err("Text exceeds the 16 KB viewer limit.");
    }
    let decoded = if bytes.starts_with(&[255, 254]) || bytes.starts_with(&[254, 255]) {
        if !(bytes.len() - 2).is_multiple_of(2) {
            return Err("Truncated UTF-16 text.");
        }
        let little = bytes[0] == 255;
        let units = bytes[2..].chunks_exact(2).map(|p| {
            if little {
                u16::from_le_bytes([p[0], p[1]])
            } else {
                u16::from_be_bytes([p[0], p[1]])
            }
        });
        core::char::decode_utf16(units)
            .collect::<Result<String, _>>()
            .map_err(|_| "Invalid UTF-16 text.")?
    } else {
        String::from(
            core::str::from_utf8(bytes.strip_prefix(&[239, 187, 191]).unwrap_or(bytes))
                .map_err(|_| "Use UTF-8 or UTF-16 text with a byte-order mark.")?,
        )
    };
    if decoded
        .bytes()
        .any(|b| b == 0 || b < 32 && !matches!(b, b'\n' | b'\r' | b'\t'))
    {
        return Err("This file contains binary data.");
    }
    Ok(decoded)
}
pub fn text(bytes: &[u8]) -> Result<String, &'static str> {
    // Every script previews as itself (the Unicode text engine renders it).
    text_document(bytes).map(|s| s.chars().map(|c| if c == '\t' { ' ' } else { c }).collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_documents_preserve_unicode_and_reject_broken_utf16_without_replacement() {
        let value = "A\u{1f642}\n";
        for little in [true, false] {
            let mut bytes = if little {
                alloc::vec![255, 254]
            } else {
                alloc::vec![254, 255]
            };
            for c in value.encode_utf16() {
                bytes.extend_from_slice(&if little {
                    c.to_le_bytes()
                } else {
                    c.to_be_bytes()
                });
            }
            assert_eq!(text_document(&bytes).unwrap(), value);
            bytes.pop();
            assert!(text_document(&bytes).is_err());
        }
        assert!(text_document(&[255, 254, 0, 216]).is_err());
        assert_eq!(text_document(value.as_bytes()).unwrap(), value);
    }
    #[test]
    fn full_view_preserves_pixels_and_bmp_padding_orientation_is_checked() {
        let rgba = alloc::vec![255u8;640*2*4];
        let encoded = qoi::encode_to_vec(&rgba, 640, 2).unwrap();
        assert_eq!(decode_full(&encoded).unwrap().width, 640);
        assert!(decode(&encoded).unwrap().width < 640);
        let mut bmp = alloc::vec![0u8;70];
        bmp[..2].copy_from_slice(b"BM");
        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
        bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&2i32.to_le_bytes());
        bmp[22..26].copy_from_slice(&2i32.to_le_bytes());
        bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
        bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
        bmp[54..60].copy_from_slice(&[255, 0, 0, 255, 255, 255]);
        bmp[62..68].copy_from_slice(&[0, 0, 255, 0, 255, 0]);
        let mut image = decode_full(&bmp).unwrap();
        let original = image.bgra.clone();
        assert_eq!(&image.bgra[..8], &[0, 0, 255, 255, 0, 255, 0, 255]);
        for _ in 0..4 {
            image.rotate_clockwise();
        }
        assert_eq!(image.bgra, original);
        image.flip_horizontal();
        image.flip_horizontal();
        assert_eq!(image.bgra, original);
        assert_eq!(
            decode_full(&image.export_qoi().unwrap()).unwrap().bgra,
            original
        );
        for len in 0..70 {
            assert!(decode_full(&bmp[..len]).is_err());
        }
        bmp[22..26].copy_from_slice(&(-2i32).to_le_bytes());
        assert_eq!(&decode_full(&bmp).unwrap().bgra[..4], &[255, 0, 0, 255]);
        bmp[30] = 1;
        assert!(decode_full(&bmp).is_err());
    }
    #[test]
    fn bounds_reject_bombs_and_broken_inputs() {
        assert!(extent(4096, 4096).is_err());
        assert!(extent(0, 1).is_err());
        for n in 0..128 {
            assert!(decode(&alloc::vec![0;n]).is_err());
        }
        let mut q = alloc::vec::Vec::from(&b"qoif"[..]);
        q.extend_from_slice(&4096u32.to_be_bytes());
        q.extend_from_slice(&4096u32.to_be_bytes());
        q.extend_from_slice(&[4, 0]);
        assert!(decode(&q).is_err());
        assert!(text(b"a\0b").is_err());
        assert_eq!(text(b"a\nline\t2").unwrap(), "a\nline 2");
    }
    #[test]
    fn real_png_jpeg_and_qoi_decode_with_the_declared_bounds() {
        let png = decode(include_bytes!("../tests/fixtures/preview-alpha.png")).unwrap();
        assert_eq!((png.width, png.height), (32, 16));
        assert_eq!(png.bgra[3], 128);
        assert!(
            png.bgra
                .chunks_exact(4)
                .all(|p| p[..3].iter().all(|v| *v <= p[3]))
        );
        let jpg = decode(include_bytes!("../tests/fixtures/preview-color.jpg")).unwrap();
        assert_eq!((jpg.width, jpg.height), (32, 16));
        assert!(
            jpg.bgra
                .chunks_exact(4)
                .all(|p| p[3] == 255 && p[2] > 200 && p[0] < 40)
        );
        let packed = qoi::encode_to_vec([200, 100, 20, 128], 1, 1).unwrap();
        let q = decode(&packed).unwrap();
        assert_eq!(q.bgra, [10, 50, 100, 128]);
    }
    #[test]
    fn transparent_pixels_are_premultiplied_and_box_filtered_once() {
        let i = raster(2, 1, &[255, 0, 0, 128, 0, 255, 0, 255], 4, true).unwrap();
        assert_eq!(i.bgra, [0, 0, 128, 128, 0, 255, 0, 255]);
        assert_eq!(type_name("setup.EXE", false), "Application");
        assert_eq!(type_name("fix.bat", false), "Doors Batch File");
        assert_eq!(type_name("tool.msi", false), "Doors Installer Package");
        assert_eq!(type_name("photo.jpg", false), "JPG File");
        assert_eq!(type_name("tools", false), "File");
        assert_eq!(type_name(".profile", false), "File");
        assert_eq!(type_name("bin", true), "File folder");
        assert_eq!(Kind::of("photo.JPEG", false), Kind::Image);
        assert_eq!(Kind::of("config.json", false), Kind::Code);
        assert_eq!(Kind::of("photo.jpg", true), Kind::Folder);
        assert_eq!(size(5_242_880), "5.0 MB");
    }
}
