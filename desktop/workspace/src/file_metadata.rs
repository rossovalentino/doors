//! Per-type file metadata for Explorer's details pane, read the way Windows'
//! details pane and Properties > Details present it, from the file's own
//! bytes only.
//!
//! Parsers never read a whole large file. They ask for byte ranges through
//! `Sparse` (`read` returns `Outcome::Need` until everything it wants is
//! held), so a caller fetches headers, chunk tables and small tail sections
//! in rounds bounded by `MAX_FETCH` and `MAX_ROUNDS`. Text-like formats (plain
//! text, source, Markdown, OBJ, ASCII STL, glTF JSON) are counted by a
//! streaming `Stream` over at most `MAX_STREAM` bytes instead.
//!
//! Every loop and length is bounded; truncated or damaged input yields the
//! fields found so far with a note, never a panic or a hang.
extern crate alloc;
use alloc::{format, string::String, vec::Vec};

mod archive;
mod audio;
mod document;
mod exec;
mod font;
mod image;
mod model;
mod text;
mod video;
pub use text::{Stream, StreamKind};

/// Most bytes a reader fetches for one file.
pub const MAX_FETCH: u64 = 1024 * 1024;
/// Most `Need` rounds for one file.
pub const MAX_ROUNDS: u32 = 512;
/// Streaming formats read at most this much; their counts are then lower
/// limits ("At least").
pub const MAX_STREAM: u64 = 8 * 1024 * 1024;
const MAX_FIELDS: usize = 64;
const MAX_VALUE: usize = 160;
/// Note for a file that ends early or whose structure is damaged.
pub const TRUNCATED: &str = "The file ends early or is damaged, so some details may be missing.";
/// Note for a read stopped by the bounds.
pub const LIMITED: &str = "Only part of the file was read, so some details may be missing.";

/// Section headers, in the pane's order (as Windows' Details groups them).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    Description,
    Origin,
    /// A program's Authenticode signature (`signature`), after its origin.
    Signature,
    Media,
    Video,
    Audio,
    Image,
    Camera,
    Photo,
    Gps,
    Content,
    Archive,
    Model,
    Program,
    Font,
}
impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Self::Description => "Description",
            Self::Origin => "Origin",
            Self::Media => "Media",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Image => "Image",
            Self::Camera => "Camera",
            Self::Photo => "Advanced photo",
            Self::Gps => "GPS",
            Self::Content => "Content",
            Self::Archive => "Archive",
            Self::Model => "3D model",
            Self::Program => "Program",
            Self::Font => "Font",
            Self::Signature => "Digital signature",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub section: Section,
    pub label: &'static str,
    pub value: String,
}

/// What a file's bytes say about it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Format recognized from the content ("PNG image").
    pub format: Option<&'static str>,
    pub fields: Vec<Field>,
    /// Why some details may be missing.
    pub note: Option<&'static str>,
}
impl Report {
    /// Adds a field; the first value of a label wins, empty values are
    /// dropped, and control characters and over-long values are cleaned.
    pub fn add(&mut self, section: Section, label: &'static str, value: impl AsRef<str>) {
        if self.fields.len() >= MAX_FIELDS || self.fields.iter().any(|f| f.label == label) {
            return;
        }
        let value = clean(value.as_ref());
        if !value.is_empty() {
            self.fields.push(Field {
                section,
                label,
                value,
            });
        }
    }
    pub fn get(&self, label: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|f| f.label == label)
            .map(|f| f.value.as_str())
    }
    /// Fields grouped by section, sections in the pane's order.
    pub fn sections(&self) -> Vec<(Section, Vec<&Field>)> {
        let mut out: Vec<(Section, Vec<&Field>)> = Vec::new();
        for field in &self.fields {
            match out.iter_mut().find(|(s, _)| *s == field.section) {
                Some((_, list)) => list.push(field),
                None => out.push((field.section, alloc::vec![field])),
            }
        }
        out.sort_by_key(|(s, _)| *s);
        out
    }
    fn format(&mut self, name: &'static str) {
        self.format.get_or_insert(name);
    }
}
fn clean(value: &str) -> String {
    let mut out = String::new();
    for (count, c) in value.chars().enumerate() {
        if count == MAX_VALUE {
            out.push_str("...");
            break;
        }
        out.push(if c.is_control() { ' ' } else { c });
    }
    String::from(out.trim())
}

/// Why a parser stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Miss {
    /// Bytes `offset..offset + len` are not held yet.
    Need(u64, u64),
    /// The file ends before the structure does, or it is damaged.
    Truncated,
}

/// The fetched parts of one file: sorted, merged byte spans.
#[derive(Clone, Debug, Default)]
pub struct Sparse {
    size: u64,
    spans: Vec<(u64, Vec<u8>)>,
}
impl Sparse {
    pub fn new(size: u64) -> Self {
        Self {
            size,
            spans: Vec::new(),
        }
    }
    pub fn size(&self) -> u64 {
        self.size
    }
    /// Bytes held.
    pub fn held(&self) -> u64 {
        self.spans.iter().map(|(_, b)| b.len() as u64).sum()
    }
    /// Stores bytes read at `offset` (clipped to the file), merging spans.
    pub fn insert(&mut self, offset: u64, bytes: &[u8]) {
        if bytes.is_empty() || offset >= self.size {
            return;
        }
        let len = (bytes.len() as u64).min(self.size - offset);
        let bytes = &bytes[..len as usize];
        let (mut start, mut end) = (offset, offset + len);
        let touching: Vec<usize> = (0..self.spans.len())
            .filter(|i| {
                let (s, b) = &self.spans[*i];
                *s <= end && s + b.len() as u64 >= start
            })
            .collect();
        for i in &touching {
            let (s, b) = &self.spans[*i];
            start = start.min(*s);
            end = end.max(s + b.len() as u64);
        }
        let mut merged = alloc::vec![0u8; (end - start) as usize];
        for i in &touching {
            let (s, b) = &self.spans[*i];
            let at = (s - start) as usize;
            merged[at..at + b.len()].copy_from_slice(b);
        }
        let at = (offset - start) as usize;
        merged[at..at + bytes.len()].copy_from_slice(bytes);
        for i in touching.iter().rev() {
            self.spans.remove(*i);
        }
        let position = self
            .spans
            .iter()
            .position(|(s, _)| *s > start)
            .unwrap_or(self.spans.len());
        self.spans.insert(position, (start, merged));
    }
    /// Exactly `len` bytes at `offset`.
    pub(crate) fn get(&self, offset: u64, len: u64) -> Result<&[u8], Miss> {
        let end = offset.checked_add(len).ok_or(Miss::Truncated)?;
        if end > self.size {
            return Err(Miss::Truncated);
        }
        if len == 0 {
            return Ok(&[]);
        }
        match self
            .spans
            .iter()
            .find(|(s, b)| *s <= offset && offset < s + b.len() as u64)
        {
            Some((s, b)) => {
                let at = (offset - s) as usize;
                let held_end = s + b.len() as u64;
                if held_end >= end {
                    Ok(&b[at..at + len as usize])
                } else {
                    Err(Miss::Need(held_end, end - held_end))
                }
            }
            None => Err(Miss::Need(offset, len)),
        }
    }
    /// Up to `len` bytes at `offset`, clipped at the end of the file.
    pub(crate) fn clip(&self, offset: u64, len: u64) -> Result<&[u8], Miss> {
        self.get(
            offset.min(self.size),
            len.min(self.size.saturating_sub(offset)),
        )
    }
    pub(crate) fn byte(&self, at: u64) -> Result<u8, Miss> {
        Ok(self.get(at, 1)?[0])
    }
    pub(crate) fn be16(&self, at: u64) -> Result<u16, Miss> {
        Ok(be16(self.get(at, 2)?, 0).unwrap_or(0))
    }
    pub(crate) fn le16(&self, at: u64) -> Result<u16, Miss> {
        Ok(le16(self.get(at, 2)?, 0).unwrap_or(0))
    }
    pub(crate) fn be32(&self, at: u64) -> Result<u32, Miss> {
        Ok(be32(self.get(at, 4)?, 0).unwrap_or(0))
    }
    pub(crate) fn le32(&self, at: u64) -> Result<u32, Miss> {
        Ok(le32(self.get(at, 4)?, 0).unwrap_or(0))
    }
    pub(crate) fn be64(&self, at: u64) -> Result<u64, Miss> {
        Ok(be64(self.get(at, 8)?, 0).unwrap_or(0))
    }
}

/// The next step for the caller.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Fetch `offset..offset + len` into the `Sparse` and call `read` again.
    /// `partial` holds what was found so far, for when the bounds are hit.
    Need {
        offset: u64,
        len: u64,
        partial: Report,
    },
    /// Everything the format offers within the bounds.
    Done(Report),
    /// A text-like format: feed the file to `Stream::new(kind)` instead.
    Stream(StreamKind),
}

/// Reads what `data` holds of the file `name`.
pub fn read(name: &str, data: &Sparse) -> Outcome {
    let mut report = Report::default();
    match dispatch(name, data, &mut report) {
        Ok(None) => Outcome::Done(report),
        Ok(Some(kind)) => Outcome::Stream(kind),
        Err(Miss::Need(offset, len)) => {
            report.note.get_or_insert(LIMITED);
            Outcome::Need {
                offset,
                len,
                partial: report,
            }
        }
        Err(Miss::Truncated) => {
            report.note.get_or_insert(TRUNCATED);
            Outcome::Done(report)
        }
    }
}

/// Lower-case extension of a file name (at most 8 characters).
fn extension(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && ext.len() <= 8 => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

fn dispatch(name: &str, d: &Sparse, r: &mut Report) -> Result<Option<StreamKind>, Miss> {
    if d.size() == 0 {
        return Ok(None);
    }
    let ext = extension(name);
    let head = d.clip(0, 64)?;
    let starts = |magic: &[u8]| head.starts_with(magic);
    let riff = |kind: &[u8]| head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == kind;
    let done = |result: Result<(), Miss>| result.map(|()| None);
    if starts(b"\x89PNG\r\n\x1a\n") {
        return done(image::png(d, r));
    }
    if starts(b"\xFF\xD8\xFF") {
        return done(image::jpeg(d, r));
    }
    if starts(b"GIF87a") || starts(b"GIF89a") {
        return done(image::gif(d, r));
    }
    if starts(b"qoif") {
        return done(image::qoi(d, r));
    }
    if riff(b"WEBP") {
        return done(image::webp(d, r));
    }
    if riff(b"WAVE") {
        return done(audio::wav(d, r));
    }
    if riff(b"AVI ") {
        return done(video::avi(d, r));
    }
    if starts(b"BM") && image::is_bmp(head) {
        return done(image::bmp(d, r));
    }
    if starts(b"II*\0") || starts(b"MM\0*") {
        return done(image::tiff_file(d, r));
    }
    if (starts(b"\0\0\x01\0") || starts(b"\0\0\x02\0"))
        && (matches!(ext.as_str(), "ico" | "cur") || image::is_ico(head))
    {
        return done(image::ico(d, r));
    }
    if head.len() >= 3
        && head[0] == b'P'
        && (b'1'..=b'7').contains(&head[1])
        && head[2].is_ascii_whitespace()
    {
        return done(image::pnm(d, r));
    }
    if starts(b"fLaC") {
        return done(audio::flac(d, r, 0));
    }
    if starts(b"ID3") {
        return done(audio::id3_file(d, r, &ext));
    }
    if starts(b"OggS") {
        return done(audio::ogg(d, r));
    }
    if audio::is_mpeg_frame(head) && matches!(ext.as_str(), "mp3" | "mp2" | "mpga") {
        return done(audio::mp3(d, r, 0));
    }
    if head.len() >= 8 && video::is_mp4(head) {
        return done(video::mp4(d, r));
    }
    if starts(b"\x1A\x45\xDF\xA3") {
        return done(video::mkv(d, r));
    }
    if starts(b"PK\x03\x04") || starts(b"PK\x05\x06") || starts(b"PK\x07\x08") {
        return done(archive::zip(d, r, &ext));
    }
    if starts(b"\x1F\x8B") {
        return done(archive::gzip(d, r, name));
    }
    if starts(b"MZ") {
        return done(exec::pe(d, r, &ext));
    }
    if starts(b"\x7FELF") {
        return done(exec::elf(d, r));
    }
    if starts(b"\0\x01\0\0")
        || starts(b"OTTO")
        || starts(b"true")
        || starts(b"ttcf")
        || starts(b"wOFF")
        || starts(b"wOF2")
    {
        return done(font::sfnt(d, r));
    }
    if starts(b"glTF") {
        return done(model::glb(d, r));
    }
    if starts(b"ply\n") || starts(b"ply\r\n") {
        return done(model::ply(d, r));
    }
    if ext == "stl" {
        return model::stl(d, r);
    }
    if d.size() >= 512 && (d.clip(257, 5)? == b"ustar" || ext == "tar") {
        return done(archive::tar(d, r));
    }
    if let Some(at) = document::pdf_header(d.clip(0, 1024)?) {
        return done(document::pdf(d, r, at));
    }
    match ext.as_str() {
        "obj" => return Ok(Some(StreamKind::Obj)),
        "gltf" => return Ok(Some(StreamKind::Gltf)),
        _ => (),
    }
    if let Some(language) = text::language(&ext) {
        return Ok(Some(StreamKind::Text(language)));
    }
    // Unknown extension: plain text when its start reads as text.
    if text::looks_like_text(d.clip(0, 4096)?) {
        return Ok(Some(StreamKind::Text("Plain text")));
    }
    Ok(None)
}

// ----------------------------------------------------------------------------
// Byte helpers (never panic: out-of-range reads are None).

pub(crate) fn take<const N: usize>(b: &[u8], at: usize) -> Option<[u8; N]> {
    b.get(at..at.checked_add(N)?)?.try_into().ok()
}
pub(crate) fn be16(b: &[u8], at: usize) -> Option<u16> {
    take(b, at).map(u16::from_be_bytes)
}
pub(crate) fn le16(b: &[u8], at: usize) -> Option<u16> {
    take(b, at).map(u16::from_le_bytes)
}
pub(crate) fn be32(b: &[u8], at: usize) -> Option<u32> {
    take(b, at).map(u32::from_be_bytes)
}
pub(crate) fn le32(b: &[u8], at: usize) -> Option<u32> {
    take(b, at).map(u32::from_le_bytes)
}
pub(crate) fn be64(b: &[u8], at: usize) -> Option<u64> {
    take(b, at).map(u64::from_be_bytes)
}
pub(crate) fn le64(b: &[u8], at: usize) -> Option<u64> {
    take(b, at).map(u64::from_le_bytes)
}
pub(crate) fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|i| &hay[*i..*i + needle.len()] == needle)
}
pub(crate) fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (0..=hay.len() - needle.len())
        .rev()
        .find(|i| &hay[*i..*i + needle.len()] == needle)
}

// ----------------------------------------------------------------------------
// Text decoding and value formatting.

/// ISO-8859-1 bytes up to the first NUL.
pub(crate) fn latin1(b: &[u8]) -> String {
    b.iter()
        .take_while(|c| **c != 0)
        .take(512)
        .map(|c| char::from(*c))
        .collect()
}
/// UTF-8 up to the first NUL, invalid sequences replaced.
pub(crate) fn utf8(b: &[u8]) -> String {
    let end = b.iter().position(|c| *c == 0).unwrap_or(b.len()).min(2048);
    String::from_utf8_lossy(&b[..end]).into_owned()
}
/// UTF-16 up to the first NUL unit.
pub(crate) fn utf16(b: &[u8], little: bool) -> String {
    let units = b.chunks_exact(2).take(1024).map(|p| {
        if little {
            u16::from_le_bytes([p[0], p[1]])
        } else {
            u16::from_be_bytes([p[0], p[1]])
        }
    });
    char::decode_utf16(units.take_while(|u| *u != 0))
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}
/// A printable four-character code ("avc1"), trimmed.
pub(crate) fn fourcc(b: &[u8]) -> Option<String> {
    let b = b.get(..4)?;
    if !b.iter().all(|c| (0x20..0x7F).contains(c)) {
        return None;
    }
    let s = core::str::from_utf8(b).ok()?.trim();
    (!s.is_empty()).then(|| String::from(s))
}
pub(crate) fn grouped(n: u64) -> String {
    let digits = format!("{n}");
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}
/// "1.2 MB (1,234,567 bytes)", as Properties shows sizes.
pub(crate) fn size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} bytes", grouped(bytes))
    } else {
        format!(
            "{} ({} bytes)",
            crate::explorer_preview::size(bytes),
            grouped(bytes)
        )
    }
}
/// "00:03:25", as Windows shows Length (rounded to the second).
pub(crate) fn length_ms(ms: u64) -> String {
    let s = ms.saturating_add(500) / 1000;
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}
/// Milliseconds of `units` at `per_second` units, without overflow.
pub(crate) fn ms(units: u64, per_second: u64) -> Option<u64> {
    (per_second > 0).then(|| {
        (u128::from(units) * 1000 / u128::from(per_second)).min(u128::from(u64::MAX / 2)) as u64
    })
}
pub(crate) fn kbps(bits_per_second: u64) -> String {
    format!("{} kbps", grouped((bits_per_second + 500) / 1000))
}
pub(crate) fn hz(rate: u64) -> String {
    if rate >= 1000 && !rate.is_multiple_of(1000) {
        format!("{}.{} kHz", rate / 1000, rate % 1000 / 100)
    } else if rate >= 1000 {
        format!("{} kHz", rate / 1000)
    } else {
        format!("{rate} Hz")
    }
}
pub(crate) fn channels(n: u64) -> String {
    match n {
        1 => "1 (mono)".into(),
        2 => "2 (stereo)".into(),
        6 => "6 (5.1)".into(),
        8 => "8 (7.1)".into(),
        n => grouped(n),
    }
}
pub(crate) fn dims(w: u64, h: u64) -> String {
    format!("{w} x {h}")
}
/// Dots per inch from dots per metre.
pub(crate) fn dpi_from_ppm(ppm: u64) -> u64 {
    (ppm * 254 + 5000) / 10000
}
/// "2024-05-01 13:45:07 UTC" from Unix seconds.
pub(crate) fn date(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}
/// A plausible timestamp (1980 .. 2100) as a date.
/// Programs whose embedded signature File Explorer checks, whole-file
/// bounded (the image digest covers every byte).
pub const MAX_SIGNED: u64 = 32 * 1024 * 1024;
pub fn signable(name: &str) -> bool {
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    ["exe", "dll", "sys", "ocx", "cpl", "scr", "com", "efi"]
        .iter()
        .any(|e| ext.eq_ignore_ascii_case(e))
}
/// The certificates (DER) of the machine's Trusted Root store, which
/// Windows' chain engine trusts beside the shipped roots (at most 256).
pub fn root_store(storage: &mut door_service_client::Storage) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let _ = storage.root_store_blobs(256, |blob| {
        if let Some(cert) = door_authenticode::roots::store_certificate(blob) {
            out.push(cert.to_vec());
        }
    });
    out
}
/// Every anchor a signature may chain to: the shipped roots and the
/// certificates of the Root store (`store`, from `root_store`).
pub fn anchors(store: &[Vec<u8>]) -> Vec<door_tls::verify::Anchor<'_>> {
    let mut all = door_authenticode::roots::of(store);
    all.extend(door_authenticode::roots::trusted());
    all
}
/// The Digital signature rows for `file`, as Windows' Properties >
/// Digital Signatures reports an embedded Authenticode signature at Unix
/// time `now`. The tab, like Windows', shows embedded signatures only, not
/// a catalog's.
pub fn signature(report: &mut Report, file: &[u8], now: i64) {
    signature_with(report, file, now, &[]);
}
/// `signature`, also trusting the Root store's certificates (`store`).
pub fn signature_with(report: &mut Report, file: &[u8], now: i64, store: &[Vec<u8>]) {
    let anchors = anchors(store);
    let status = match door_authenticode::verify(file, now, &anchors) {
        Ok(s) => {
            report.add(Section::Signature, "Signed by", &s.publisher);
            report.add(Section::Signature, "Issued by", &s.issuer);
            if let Some(when) = s.timestamp.and_then(plausible_date) {
                report.add(Section::Signature, "Signed on", when);
            }
            report.add(Section::Signature, "Digest", s.digest);
            "Verified"
        }
        Err(door_authenticode::Error::Unsigned) => "Not signed",
        Err(door_authenticode::Error::Modified | door_authenticode::Error::BadSignature) => {
            "Not valid: the file changed after it was signed"
        }
        Err(door_authenticode::Error::Untrusted | door_authenticode::Error::Chaining) => {
            "Not trusted: issued by an unknown authority"
        }
        Err(door_authenticode::Error::Revoked) => "Not valid: the certificate was revoked",
        Err(door_authenticode::Error::Expired) => "Not valid: the certificate expired",
        Err(door_authenticode::Error::WrongUsage) => "Not valid: the certificate isn't for code",
        Err(_) => "Can't be checked",
    };
    // Status first among the section's rows.
    report.add(Section::Signature, "Status", status);
    if let Some(at) = report
        .fields
        .iter()
        .position(|f| f.section == Section::Signature && f.label == "Status")
    {
        let row = report.fields.remove(at);
        let first = report
            .fields
            .iter()
            .position(|f| f.section == Section::Signature)
            .unwrap_or(report.fields.len());
        report.fields.insert(first, row);
    }
}
pub(crate) fn plausible_date(unix: i64) -> Option<String> {
    (315_532_800..4_102_444_800)
        .contains(&unix)
        .then(|| date(unix))
}

#[cfg(test)]
pub(crate) mod testing {
    //! Runs a parser as Explorer's background job does, over in-memory bytes.
    extern crate std;
    use super::*;
    /// The report for `bytes`, fetching like the job (4 KiB minimum reads,
    /// the same bounds), plus the stream result for text-like formats.
    pub(crate) fn run(name: &str, bytes: &[u8]) -> Report {
        let mut data = Sparse::new(bytes.len() as u64);
        for round in 0.. {
            match read(name, &data) {
                Outcome::Done(report) => return report,
                Outcome::Stream(kind) => {
                    let mut stream = Stream::new(kind);
                    let limit = (bytes.len() as u64).min(MAX_STREAM) as usize;
                    for chunk in bytes[..limit].chunks(192) {
                        stream.feed(chunk);
                    }
                    return stream.finish(limit == bytes.len());
                }
                Outcome::Need {
                    offset,
                    len,
                    partial,
                } => {
                    assert!(len > 0, "{name}: empty need");
                    if round >= MAX_ROUNDS || data.held() >= MAX_FETCH {
                        return partial;
                    }
                    let end = (offset + len.max(4096)).min(bytes.len() as u64);
                    assert!(offset < end, "{name}: need past the end");
                    data.insert(offset, &bytes[offset as usize..end as usize]);
                }
            }
        }
        unreachable!()
    }
    /// Every prefix and a few corruptions of `bytes` parse without panicking.
    pub(crate) fn fuzz(name: &str, bytes: &[u8]) {
        let step = (bytes.len() / 64).max(1);
        for len in (0..bytes.len()).step_by(step) {
            let _ = run(name, &bytes[..len]);
        }
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..24 {
            let mut copy = bytes.to_vec();
            for _ in 0..8 {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                if !copy.is_empty() {
                    let at = (seed % copy.len() as u64) as usize;
                    copy[at] = (seed >> 32) as u8;
                }
            }
            let _ = run(name, &copy);
        }
        for fill in [0u8, 0xFF] {
            let mut copy = bytes.to_vec();
            for b in copy.iter_mut().skip(bytes.len().min(16)) {
                *b = fill;
            }
            let _ = run(name, &copy);
        }
    }
    pub(crate) fn be(v: u64, n: usize) -> Vec<u8> {
        v.to_be_bytes()[8 - n..].to_vec()
    }
    pub(crate) fn le(v: u64, n: usize) -> Vec<u8> {
        v.to_le_bytes()[..n].to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    #[test]
    fn sparse_merges_spans_and_names_the_missing_range() {
        let mut d = Sparse::new(100);
        assert_eq!(d.get(10, 5), Err(Miss::Need(10, 5)));
        d.insert(10, &[1; 10]);
        assert_eq!(d.get(12, 4).unwrap(), &[1; 4]);
        assert_eq!(d.get(15, 10), Err(Miss::Need(20, 5)));
        d.insert(20, &[2; 10]);
        d.insert(5, &[3; 5]);
        assert_eq!(d.spans.len(), 1);
        assert_eq!(d.get(5, 25).unwrap().len(), 25);
        assert_eq!(d.get(95, 10), Err(Miss::Truncated));
        assert_eq!(d.clip(95, 10), Err(Miss::Need(95, 5)));
        d.insert(98, &[9; 10]);
        assert_eq!(d.held(), 27);
        assert_eq!(d.get(u64::MAX, 2), Err(Miss::Truncated));
    }
    #[test]
    fn values_read_like_windows() {
        assert_eq!(date(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(date(1_714_570_507), "2024-05-01 13:35:07 UTC");
        assert_eq!(date(-86_400), "1969-12-31 00:00:00 UTC");
        assert_eq!(length_ms(205_400), "00:03:25");
        assert_eq!(length_ms(3_725_000), "01:02:05");
        assert_eq!(hz(44_100), "44.1 kHz");
        assert_eq!(hz(48_000), "48 kHz");
        assert_eq!(size(1536), "1.5 KB (1,536 bytes)");
        assert_eq!(dpi_from_ppm(3780), 96);
        assert_eq!(fourcc(b"avc1"), Some("avc1".into()));
        assert_eq!(fourcc(b"\0\x01ab"), None);
        assert_eq!(utf16(&[0x48, 0, 0x69, 0, 0, 0, 0x41, 0], true), "Hi");
        let mut r = Report::default();
        r.add(Section::Image, "Width", "\u{1}x".repeat(300));
        assert!(r.get("Width").unwrap().ends_with("..."));
        r.add(Section::Image, "Width", "again");
        r.add(Section::Description, "Title", "  ");
        assert_eq!(r.fields.len(), 1);
    }
    /// Garbage after every recognized signature, at several lengths: the
    /// readers end (within the bounds) without panicking.
    #[test]
    fn garbage_after_every_signature_ends_cleanly() {
        let signatures: [(&str, &[u8]); 30] = [
            ("a.png", b"\x89PNG\r\n\x1a\n"),
            ("a.jpg", b"\xFF\xD8\xFF"),
            ("a.gif", b"GIF89a"),
            ("a.qoi", b"qoif"),
            ("a.webp", b"RIFF\xFF\xFF\xFF\x7FWEBP"),
            ("a.wav", b"RIFF\xFF\xFF\xFF\x7FWAVE"),
            ("a.avi", b"RIFF\xFF\xFF\xFF\x7FAVI "),
            ("a.bmp", b"BM\0\0\0\0\0\0\0\0\0\0\0\0\x28\0\0\0"),
            ("a.tif", b"II*\0"),
            ("a.ico", b"\0\0\x01\0"),
            ("a.ppm", b"P6\n"),
            ("a.flac", b"fLaC"),
            ("a.mp3", b"ID3\x04\0\0"),
            ("b.mp3", b"\xFF\xFB\x90\x44"),
            ("a.ogg", b"OggS"),
            ("a.mp4", b"\0\0\0\x18ftypisom"),
            ("a.mkv", b"\x1A\x45\xDF\xA3"),
            ("a.zip", b"PK\x03\x04"),
            ("a.gz", b"\x1F\x8B\x08\x1F"),
            ("a.exe", b"MZ"),
            ("a.elf", b"\x7FELF\x02\x01"),
            ("a.ttf", b"\0\x01\0\0"),
            ("a.ttc", b"ttcf"),
            ("a.glb", b"glTF"),
            ("a.ply", b"ply\n"),
            ("a.stl", b"solid "),
            ("a.tar", b""),
            ("a.pdf", b"%PDF-1.4\n"),
            ("a.obj", b"v 1 2 3\n"),
            ("a.gltf", b"{\"asset\":["),
        ];
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for (name, magic) in signatures {
            for len in [0usize, 1, 3, 17, 300, 5000, 70_000] {
                for _ in 0..3 {
                    let mut bytes = magic.to_vec();
                    for _ in 0..len {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        bytes.push((seed >> 24) as u8);
                    }
                    let _ = run(name, &bytes);
                }
            }
        }
    }
    #[test]
    fn unknown_binary_has_no_fields_and_empty_files_none() {
        assert_eq!(
            run("blob.bin", &[0, 1, 2, 3, 0xFF, 0xFE, 0, 0, 7]),
            Report::default()
        );
        assert_eq!(run("empty.png", &[]), Report::default());
        let _ = (be(1, 2), le(1, 2));
    }
    #[test]
    fn programs_report_their_signature_status() {
        let status = |bytes: &[u8]| {
            let mut r = Report::default();
            signature(&mut r, bytes, 1_793_577_600);
            r.get("Status").map(String::from)
        };
        assert_eq!(
            status(include_bytes!(
                "../../../crates/authenticode/tests/data/unsigned.exe"
            ))
            .as_deref(),
            Some("Not signed")
        );
        assert_eq!(
            status(include_bytes!(
                "../../../crates/authenticode/tests/data/tampered.exe"
            ))
            .as_deref(),
            Some("Not valid: the file changed after it was signed")
        );
        // The test PKI is not a public root.
        assert_eq!(
            status(include_bytes!(
                "../../../crates/authenticode/tests/data/signed.exe"
            ))
            .as_deref(),
            Some("Not trusted: issued by an unknown authority")
        );
        assert!(signable("Setup.EXE") && signable("x.dll") && !signable("notes.txt"));
    }
}
