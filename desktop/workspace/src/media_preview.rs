//! Preview pane media (P15): archive listings (ZIP, TAR, gzip-compressed
//! TAR, GZIP, RAR 4 and RAR 5, 7-Zip), audio cover art (ID3v2, FLAC, MP4),
//! WAV waveforms and video poster frames (Motion-JPEG in AVI and MP4, cover
//! art in MP4 and Matroska). Like `file_metadata`, every parser reads
//! through a `Sparse` and reports the range it is missing (`Miss::Need`), so
//! the caller fetches only what is looked at. Archive members are never
//! extracted and patented video codecs are never decoded; pictures come back
//! still encoded (JPEG or PNG) for `explorer_preview::decode`.
extern crate alloc;
use crate::file_metadata::{self, Miss, Outcome, Report, Sparse, be32, latin1, le16, le32, le64};
use alloc::{borrow::ToOwned, string::String, vec::Vec};

/// Storage reads and bytes one media preview may spend.
const MAX_ROUNDS: u32 = 400;
const MAX_HELD: u64 = 2 * MAX_PICTURE + MAX_INFLATE_INPUT + 1024 * 1024;
pub const DAMAGED: &str = "This file is damaged or incomplete.";

/// Drives `read`, then `file_metadata::read` for the facts shown under
/// the picture (title, artist, length), over one `Sparse`: the preview pane
/// fetches each `Step::Fetch` range until `Step::Done`.
pub struct Reader {
    data: Sparse,
    preview: Option<Preview>,
    rounds: u32,
}
pub enum Step {
    Fetch(u64, u64),
    Done(Preview, Report),
}
impl Reader {
    pub fn new(size: u64) -> Self {
        Self {
            data: Sparse::new(size),
            preview: None,
            rounds: 0,
        }
    }
    pub fn insert(&mut self, at: u64, bytes: &[u8]) {
        self.data.insert(at, bytes);
    }
    pub fn step(&mut self, name: &str) -> Step {
        let spent = self.rounds >= MAX_ROUNDS || self.data.held() >= MAX_HELD;
        if self.preview.is_none() {
            match read(name, &self.data) {
                Ok(p) => self.preview = Some(p),
                Err(Miss::Need(at, len)) if !spent => {
                    self.rounds += 1;
                    return Step::Fetch(at, len);
                }
                Err(Miss::Need(..)) => {
                    self.preview = Some(Preview {
                        partial: true,
                        ..Preview::default()
                    })
                }
                Err(Miss::Truncated) => {
                    self.preview = Some(Preview {
                        note: DAMAGED,
                        ..Preview::default()
                    })
                }
            }
        }
        let report = match file_metadata::read(name, &self.data) {
            Outcome::Need { offset, len, .. } if !spent => {
                self.rounds += 1;
                return Step::Fetch(offset, len);
            }
            Outcome::Need { partial, .. } => partial,
            Outcome::Done(report) => report,
            Outcome::Stream(_) => Report::default(),
        };
        Step::Done(self.preview.take().unwrap_or_default(), report)
    }
}

/// Archive entries listed (the count goes on past it).
pub const MAX_ENTRIES: usize = 256;
/// Largest embedded picture read (cover art, poster frame).
pub const MAX_PICTURE: u64 = 2 * 1024 * 1024;
/// Waveform columns.
pub const BUCKETS: usize = 96;
/// Bytes of each waveform column that are read.
const WINDOW: u64 = 2048;
/// Compressed bytes of a .tar.gz inflated for its listing, and the most
/// they may expand to.
pub const MAX_INFLATE_INPUT: u64 = 1024 * 1024;
const MAX_INFLATE_OUTPUT: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Path inside the archive, '/'-separated.
    pub name: String,
    pub size: u64,
    pub folder: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preview {
    /// Archive format ("ZIP archive"); empty for audio and video.
    pub format: &'static str,
    pub entries: Vec<Entry>,
    /// Entries the archive holds (more than `entries` when capped).
    pub count: u64,
    /// The listing stopped early (the read limit, a damaged header).
    pub partial: bool,
    /// Why the archive's names can't be shown.
    pub note: &'static str,
    /// An embedded picture, still encoded (JPEG or PNG).
    pub picture: Option<Vec<u8>>,
    /// Waveform peaks, 0..=255 per column (WAV PCM and float).
    pub peaks: Vec<u8>,
}
impl Preview {
    /// The archive's top level as Explorer shows a compressed folder:
    /// folders first, then files, each name once.
    pub fn root(&self) -> Vec<Entry> {
        let mut out: Vec<Entry> = Vec::new();
        for e in &self.entries {
            let path = e.name.trim_start_matches("./").trim_start_matches('/');
            let (first, rest) = path.split_once('/').unwrap_or((path, ""));
            if first.is_empty() || first == "." {
                continue;
            }
            let folder = e.folder || !rest.is_empty();
            match out.iter_mut().find(|o| o.name == first) {
                Some(o) => {
                    if folder && !o.folder {
                        o.folder = true;
                        o.size = 0;
                    }
                }
                None => out.push(Entry {
                    name: first.to_owned(),
                    size: if folder { 0 } else { e.size },
                    folder,
                }),
            }
        }
        out.sort_by(|a, b| {
            b.folder
                .cmp(&a.folder)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        out
    }
}

/// What the preview pane shows for the file `name`, from the parts of it
/// `d` holds.
pub fn read(name: &str, d: &Sparse) -> Result<Preview, Miss> {
    let mut p = Preview::default();
    if d.size() == 0 {
        return Ok(p);
    }
    let head = d.clip(0, 64)?;
    let starts = |magic: &[u8]| head.starts_with(magic);
    let riff = |kind: &[u8]| head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == kind;
    let lower = name.to_ascii_lowercase();
    if starts(b"PK\x03\x04") || starts(b"PK\x05\x06") {
        zip(d, &mut p)?;
    } else if starts(b"Rar!\x1A\x07\x01\x00") {
        rar5(d, &mut p)?;
    } else if starts(b"Rar!\x1A\x07\x00") {
        rar4(d, &mut p)?;
    } else if starts(b"7z\xBC\xAF\x27\x1C") {
        seven_zip(d, &mut p)?;
    } else if starts(b"\x1F\x8B") {
        gzip(d, &mut p, &lower)?;
    } else if d.size() >= 512 && (d.clip(257, 5)? == b"ustar" || lower.ends_with(".tar")) {
        p.format = "TAR archive";
        let size = d.size();
        tar(|at, len| d.get(at, len), size, &mut p)?;
    } else if riff(b"WAVE") {
        wav(d, &mut p)?;
    } else if riff(b"AVI ") {
        p.picture = avi_frame(d)?;
    } else if starts(b"ID3") {
        p.picture = id3_picture(d)?;
    } else if starts(b"fLaC") {
        p.picture = flac_picture(d)?;
    } else if head.len() >= 8 && &head[4..8] == b"ftyp" {
        p.picture = mp4_picture(d)?;
    } else if starts(b"\x1A\x45\xDF\xA3") {
        p.picture = mkv_picture(d)?;
    }
    Ok(p)
}

fn is_picture(b: &[u8]) -> bool {
    b.starts_with(b"\xFF\xD8\xFF") || b.starts_with(b"\x89PNG\r\n\x1a\n")
}
fn push(p: &mut Preview, name: String, size: u64, folder: bool) {
    p.count += 1;
    if p.entries.len() < MAX_ENTRIES {
        let name = name.replace('\\', "/");
        let folder = folder || name.ends_with('/');
        p.entries.push(Entry {
            name: String::from(name.trim_end_matches('/')),
            size: if folder { 0 } else { size },
            folder,
        });
    }
}

// ----------------------------------------------------------------------------
// Archives.

fn zip(d: &Sparse, p: &mut Preview) -> Result<(), Miss> {
    p.format = "ZIP archive";
    let size = d.size();
    let tail_len = size.min(65_557);
    let tail = d.get(size - tail_len, tail_len)?;
    let Some(eocd) = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|i| tail[*i..].starts_with(b"PK\x05\x06"))
    else {
        return Err(Miss::Truncated);
    };
    let e = &tail[eocd..];
    let mut entries = u64::from(le16(e, 10).unwrap_or(0));
    let mut cd_size = u64::from(le32(e, 12).unwrap_or(0));
    let mut cd_offset = u64::from(le32(e, 16).unwrap_or(0));
    let eocd_at = size - tail_len + eocd as u64;
    if (entries == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF) && eocd_at >= 20 {
        let locator = d.get(eocd_at - 20, 20)?;
        if locator.starts_with(b"PK\x06\x07") {
            let at = le64(locator, 8)
                .unwrap_or(u64::MAX)
                .min(size.saturating_sub(56));
            let record = d.get(at, 56)?;
            if record.starts_with(b"PK\x06\x06") {
                entries = le64(record, 32).unwrap_or(0);
                cd_size = le64(record, 40).unwrap_or(0);
                cd_offset = le64(record, 48).unwrap_or(0);
            }
        }
    }
    let read = cd_size.min(512 * 1024).min(size.saturating_sub(cd_offset));
    let cd = d.get(cd_offset.min(size), read)?;
    let mut at = 0usize;
    while p.entries.len() < MAX_ENTRIES {
        let Some(h) = cd.get(at..at + 46).filter(|h| h.starts_with(b"PK\x01\x02")) else {
            break;
        };
        let flags = le16(h, 8).unwrap_or(0);
        let mut original = u64::from(le32(h, 24).unwrap_or(0));
        let (nlen, xlen, clen) = (
            usize::from(le16(h, 28).unwrap_or(0)),
            usize::from(le16(h, 30).unwrap_or(0)),
            usize::from(le16(h, 32).unwrap_or(0)),
        );
        let name = cd.get(at + 46..at + 46 + nlen).unwrap_or(&[]);
        let extra = cd.get(at + 46 + nlen..at + 46 + nlen + xlen).unwrap_or(&[]);
        let mut x = 0;
        while original == 0xFFFF_FFFF && x + 4 <= extra.len() {
            let (id, len) = (
                le16(extra, x).unwrap_or(0),
                usize::from(le16(extra, x + 2).unwrap_or(0)),
            );
            if id == 1 {
                original = le64(extra, x + 4).unwrap_or(original);
            }
            x += 4 + len;
        }
        // Bit 11: UTF-8 names; otherwise code page 437, read as Latin-1.
        let name = if flags & 0x800 != 0 {
            String::from_utf8_lossy(name).into_owned()
        } else {
            latin1(name)
        };
        push(p, name, original, false);
        p.count -= 1;
        at += 46 + nlen + xlen + clen;
    }
    p.count = entries.max(p.entries.len() as u64);
    Ok(())
}

/// TAR headers from `get` (the archive, or an inflated .tar.gz prefix):
/// GNU long names and pax `path` records apply to the next entry.
fn tar<'a>(
    get: impl Fn(u64, u64) -> Result<&'a [u8], Miss>,
    size: u64,
    p: &mut Preview,
) -> Result<(), Miss> {
    let octal = |b: &[u8]| -> Option<u64> {
        let text = b.iter().take_while(|c| **c != 0 && **c != b' ');
        let mut v = 0u64;
        let mut any = false;
        for c in text.skip_while(|c| **c == b' ') {
            if !(b'0'..=b'7').contains(c) {
                return None;
            }
            v = v.checked_mul(8)? + u64::from(c - b'0');
            any = true;
        }
        any.then_some(v)
    };
    let mut at = 0u64;
    let mut long_name: Option<String> = None;
    for _ in 0..100_000 {
        if p.entries.len() >= MAX_ENTRIES {
            p.partial = true;
            break;
        }
        if at + 512 > size {
            break;
        }
        let h = match get(at, 512) {
            Ok(h) => h,
            Err(Miss::Truncated) => {
                p.partial = true;
                break;
            }
            Err(e) => return Err(e),
        };
        if h.iter().all(|c| *c == 0) {
            break;
        }
        let Some(len) = octal(&h[124..136]) else {
            p.partial = true;
            break;
        };
        let data = at + 512;
        match h[156] {
            b'L' | b'x' => {
                let body = match get(data, len.min(64 * 1024)) {
                    Ok(b) => b,
                    Err(Miss::Truncated) => {
                        p.partial = true;
                        break;
                    }
                    Err(e) => return Err(e),
                };
                long_name = if h[156] == b'L' {
                    Some(latin1(body))
                } else {
                    pax_path(body).or(long_name)
                };
            }
            b'g' | b'K' => (),
            kind => {
                let name = long_name.take().unwrap_or_else(|| {
                    let base = latin1(&h[..100]);
                    let prefix = if &h[257..262] == b"ustar" {
                        latin1(&h[345..500])
                    } else {
                        String::new()
                    };
                    if prefix.is_empty() {
                        base
                    } else {
                        alloc::format!("{prefix}/{base}")
                    }
                });
                push(p, name, len, kind == b'5');
            }
        }
        at = data.saturating_add(len.div_ceil(512) * 512);
    }
    Ok(())
}
fn pax_path(b: &[u8]) -> Option<String> {
    // Records: "<length> <key>=<value>\n".
    let mut at = 0usize;
    while at < b.len() {
        let space = at + b[at..].iter().position(|c| *c == b' ')?;
        let len: usize = core::str::from_utf8(&b[at..space]).ok()?.parse().ok()?;
        let record = b.get(space + 1..at.checked_add(len)?)?;
        if let Some(value) = record.strip_prefix(b"path=") {
            return Some(
                String::from_utf8_lossy(value.strip_suffix(b"\n").unwrap_or(value)).into_owned(),
            );
        }
        at = at.checked_add(len.max(1))?;
    }
    None
}

fn gzip(d: &Sparse, p: &mut Preview, lower: &str) -> Result<(), Miss> {
    let h = d.get(0, 10)?;
    let flags = h[3];
    let mut at = 10u64;
    if flags & 4 != 0 {
        at += 2 + u64::from(d.le16(at)?);
    }
    let mut original = String::new();
    for flag in [8u8, 16] {
        if flags & flag != 0 {
            let b = d.clip(at, 1024)?;
            let end = b.iter().position(|c| *c == 0).ok_or(Miss::Truncated)?;
            if flag == 8 {
                original = latin1(&b[..end]);
            }
            at += end as u64 + 1;
        }
    }
    if flags & 2 != 0 {
        at += 2;
    }
    let file = lower.rsplit(['/', '\\']).next().unwrap_or(lower);
    let inner = if !original.is_empty() {
        original
    } else if let Some(stem) = file.strip_suffix(".tgz") {
        alloc::format!("{stem}.tar")
    } else {
        String::from(file.strip_suffix(".gz").unwrap_or(file))
    };
    if inner.to_ascii_lowercase().ends_with(".tar") && h[2] == 8 && at < d.size() {
        p.format = "Compressed TAR archive (gzip)";
        let available = d.size() - at;
        let input = d.get(at, available.min(MAX_INFLATE_INPUT))?;
        let options = zune_inflate::DeflateOptions::default()
            .set_limit(MAX_INFLATE_OUTPUT)
            .set_size_hint(64 * 1024);
        let mut decoder = zune_inflate::DeflateDecoder::new_with_options(input, options);
        let (out, complete) = match decoder.decode_deflate() {
            Ok(out) => (out, true),
            Err(e) => (e.data, false),
        };
        let size = out.len() as u64;
        tar(
            |at, len| {
                usize::try_from(at)
                    .ok()
                    .zip(usize::try_from(at.saturating_add(len)).ok())
                    .and_then(|(s, e)| out.get(s..e))
                    .ok_or(Miss::Truncated)
            },
            size,
            p,
        )?;
        p.partial |= !complete || available > MAX_INFLATE_INPUT;
    } else {
        p.format = "GZIP archive";
        // ISIZE: the original size modulo 2^32, in the last four bytes.
        let original = if d.size() >= 18 {
            u64::from(d.le32(d.size() - 4)?)
        } else {
            0
        };
        push(p, inner, original, false);
    }
    Ok(())
}

/// RAR 5 variable-length integer (7 bits per byte, low first).
fn vint(b: &[u8], at: &mut usize) -> Result<u64, Miss> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let c = *b.get(*at).ok_or(Miss::Truncated)?;
        *at += 1;
        v |= u64::from(c & 0x7F) << shift;
        if c & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err(Miss::Truncated)
}
/// RAR 5 (rarlab "RAR 5.0 archive format"): block headers carry a CRC, a
/// size, a type and flags; file headers (type 2) name each entry.
fn rar5(d: &Sparse, p: &mut Preview) -> Result<(), Miss> {
    p.format = "RAR archive";
    let size = d.size();
    let mut at = 8u64;
    for _ in 0..100_000 {
        if p.entries.len() >= MAX_ENTRIES {
            p.partial = true;
            break;
        }
        if at + 6 > size {
            break;
        }
        let pre = d.clip(at, 4 + 10)?;
        let mut i = 4usize;
        let header = vint(pre, &mut i)?;
        if header == 0 || header > 2 * 1024 * 1024 {
            p.partial = true;
            break;
        }
        let start = at + i as u64;
        let h = d.get(start, header)?;
        let mut j = 0usize;
        let kind = vint(h, &mut j)?;
        let flags = vint(h, &mut j)?;
        if flags & 1 != 0 {
            vint(h, &mut j)?;
        }
        let data = if flags & 2 != 0 { vint(h, &mut j)? } else { 0 };
        match kind {
            2 => {
                let file_flags = vint(h, &mut j)?;
                let unpacked = vint(h, &mut j)?;
                vint(h, &mut j)?; // attributes
                j += if file_flags & 2 != 0 { 4 } else { 0 }
                    + if file_flags & 4 != 0 { 4 } else { 0 };
                vint(h, &mut j)?; // compression
                vint(h, &mut j)?; // host OS
                let len = usize::try_from(vint(h, &mut j)?).map_err(|_| Miss::Truncated)?;
                let name = h
                    .get(j..j.checked_add(len).ok_or(Miss::Truncated)?)
                    .ok_or(Miss::Truncated)?;
                push(
                    p,
                    String::from_utf8_lossy(name).into_owned(),
                    if file_flags & 8 != 0 { 0 } else { unpacked },
                    file_flags & 1 != 0,
                );
            }
            4 => {
                p.note = "The file names in this archive are encrypted.";
                break;
            }
            5 => break,
            _ => (),
        }
        // Sizes come from the file: an overflowing block ends the listing.
        let Some(next) = start.checked_add(header).and_then(|n| n.checked_add(data)) else {
            p.partial = true;
            break;
        };
        at = next;
    }
    Ok(())
}
/// RAR 4 ("RAR 4.x technote"): 7-byte block headers; file blocks (0x74)
/// hold the packed size, the unpacked size and the name.
fn rar4(d: &Sparse, p: &mut Preview) -> Result<(), Miss> {
    p.format = "RAR archive";
    let size = d.size();
    let mut at = 7u64;
    for _ in 0..100_000 {
        if p.entries.len() >= MAX_ENTRIES {
            p.partial = true;
            break;
        }
        if at + 7 > size {
            break;
        }
        let h = d.get(at, 7)?;
        let (kind, flags, header) = (
            h[2],
            le16(h, 3).unwrap_or(0),
            u64::from(le16(h, 5).unwrap_or(0)),
        );
        if header < 7 {
            p.partial = true;
            break;
        }
        let mut add = if kind == 0x74 || flags & 0x8000 != 0 {
            u64::from(d.le32(at + 7)?)
        } else {
            0
        };
        match kind {
            0x73 if flags & 0x80 != 0 => {
                p.note = "The file names in this archive are encrypted.";
                break;
            }
            0x74 => {
                let b = d.get(at, header)?;
                let mut unpacked = u64::from(le32(b, 11).unwrap_or(0));
                let len = usize::from(le16(b, 26).unwrap_or(0));
                let mut name_at = 32;
                if flags & 0x100 != 0 {
                    add |= u64::from(le32(b, 32).unwrap_or(0)) << 32;
                    unpacked |= u64::from(le32(b, 36).unwrap_or(0)) << 32;
                    name_at = 40;
                }
                let raw = b.get(name_at..name_at + len).ok_or(Miss::Truncated)?;
                // Unicode names keep an ASCII form before the first NUL.
                let raw = raw.split(|c| *c == 0).next().unwrap_or(raw);
                let name = if flags & 0x200 != 0 {
                    String::from_utf8_lossy(raw).into_owned()
                } else {
                    latin1(raw)
                };
                push(p, name, unpacked, flags & 0xE0 == 0xE0);
            }
            0x7B => break,
            _ => (),
        }
        at = at.saturating_add(header).saturating_add(add);
    }
    Ok(())
}
/// 7-Zip keeps its file list at the end, nearly always LZMA-compressed
/// (an encoded header), which Doors does not decompress yet.
/// 7-Zip archives up to this size are listed in the preview pane (the
/// whole file is read: the file list sits after the data and is usually
/// LZMA-compressed).
const SEVEN_ZIP_LIST_BYTES: u64 = 8 * 1024 * 1024;
fn seven_zip(d: &Sparse, p: &mut Preview) -> Result<(), Miss> {
    use crate::sevenzip::{Archive, Fault};
    p.format = "7-Zip archive";
    if d.size() > SEVEN_ZIP_LIST_BYTES {
        p.note = "This 7-Zip archive is large. Open it in Preview or extract it to see its files.";
        return Ok(());
    }
    let bytes = d.get(0, d.size())?;
    match Archive::open(bytes, None, 64 << 20) {
        Ok(a) => {
            for e in &a.entries {
                push(p, e.path.clone(), e.size, e.folder);
            }
        }
        Err(Fault::Password) => p.note = "The file names in this archive are encrypted.",
        Err(Fault::Unsupported) => {
            p.note = "This 7-Zip archive uses a compression method Doors doesn't read (BCJ2, PPMd or BZip2)."
        }
        Err(_) => p.note = "The archive is damaged.",
    }
    Ok(())
}

// ----------------------------------------------------------------------------
// Audio.

fn wav(d: &Sparse, p: &mut Preview) -> Result<(), Miss> {
    let size = d.size();
    let (mut fmt, mut data) = (None, None);
    let mut at = 12u64;
    for _ in 0..64 {
        if at + 8 > size {
            break;
        }
        let c = d.get(at, 8)?;
        let len = u64::from(le32(c, 4).unwrap_or(0));
        if &c[..4] == b"fmt " {
            let f = d.get(at + 8, len.min(40).min(size - at - 8))?;
            let mut tag = le16(f, 0).unwrap_or(0);
            if tag == 0xFFFE {
                tag = le16(f, 24).unwrap_or(0);
            }
            fmt = Some((
                tag,
                le16(f, 2).unwrap_or(0),
                le16(f, 12).unwrap_or(0),
                le16(f, 14).unwrap_or(0),
            ));
        } else if &c[..4] == b"data" {
            data = Some((at + 8, len.min(size - at - 8)));
            break;
        }
        at = at.saturating_add(8 + len + (len & 1));
    }
    let (Some((tag, channels, align, bits)), Some((start, len))) = (fmt, data) else {
        return Ok(());
    };
    let width = usize::from(bits / 8);
    let pcm = tag == 1 && matches!(bits, 8 | 16 | 24 | 32);
    let float = tag == 3 && bits == 32;
    if !(pcm || float) || channels == 0 || usize::from(align) < width * usize::from(channels) {
        return Ok(());
    }
    let align = u64::from(align);
    let frames = len / align;
    if frames == 0 {
        return Ok(());
    }
    let mut peaks = Vec::with_capacity(BUCKETS);
    for column in 0..BUCKETS as u64 {
        let first = frames * column / BUCKETS as u64;
        let last = (frames * (column + 1) / BUCKETS as u64).max(first + 1);
        let count = (last - first).min((WINDOW / align).max(1));
        let b = d.get(start + first * align, count * align)?;
        let mut peak = 0u32;
        for frame in b.chunks_exact(align as usize) {
            for ch in 0..usize::from(channels) {
                let s = &frame[ch * width..ch * width + width];
                let level = match (float, width) {
                    (true, _) => {
                        let v = f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                        if v.is_finite() {
                            ((if v < 0.0 { -v } else { v }).min(1.0) * 255.0) as u32
                        } else {
                            0
                        }
                    }
                    (_, 1) => u32::from(s[0].abs_diff(128)) * 2,
                    (_, 2) => i16::from_le_bytes([s[0], s[1]]).unsigned_abs() as u32 >> 7,
                    (_, 3) => (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8).unsigned_abs() >> 15,
                    _ => i32::from_le_bytes([s[0], s[1], s[2], s[3]]).unsigned_abs() >> 23,
                };
                peak = peak.max(level);
            }
        }
        peaks.push(peak.min(255) as u8);
    }
    p.peaks = peaks;
    Ok(())
}

fn syncsafe(b: &[u8]) -> u64 {
    b.iter()
        .take(4)
        .fold(0, |v, c| v << 7 | u64::from(c & 0x7F))
}
/// ID3v2 APIC (v2.3/2.4) or PIC (v2.2): the first embedded picture.
fn id3_picture(d: &Sparse) -> Result<Option<Vec<u8>>, Miss> {
    let h = d.get(0, 10)?;
    let (major, flags) = (h[3], h[5]);
    if flags & 0x80 != 0 || !(2..=4).contains(&major) {
        return Ok(None);
    }
    let tag_end = (10 + syncsafe(&h[6..10])).min(d.size());
    let mut at = 10u64;
    if flags & 0x40 != 0 && major >= 3 {
        let b = d.get(at, 4)?;
        at += if major == 4 {
            syncsafe(b)
        } else {
            u64::from(be32(b, 0).unwrap_or(0)) + 4
        };
    }
    let header = if major == 2 { 6 } else { 10 };
    for _ in 0..512 {
        if at + header > tag_end {
            break;
        }
        let fh = d.get(at, header)?;
        if fh[0] == 0 {
            break;
        }
        let (id, len) = match major {
            2 => (
                &fh[..3],
                u64::from(fh[3]) << 16 | u64::from(fh[4]) << 8 | u64::from(fh[5]),
            ),
            4 => (&fh[..4], syncsafe(&fh[4..8])),
            _ => (&fh[..4], u64::from(be32(fh, 4).unwrap_or(0))),
        };
        let body = at + header;
        if body + len > tag_end {
            break;
        }
        if (id == b"APIC" || id == b"PIC") && len <= MAX_PICTURE {
            let b = d.get(body, len)?;
            let encoding = *b.first().ok_or(Miss::Truncated)?;
            let mut i = 1usize;
            if major == 2 {
                i += 3;
            } else {
                i += b[i..].iter().position(|c| *c == 0).ok_or(Miss::Truncated)? + 1;
            }
            i += 1; // picture type
            if matches!(encoding, 1 | 2) {
                while i + 1 < b.len() && !(b[i] == 0 && b[i + 1] == 0) {
                    i += 2;
                }
                i += 2;
            } else {
                i += b
                    .get(i..)
                    .and_then(|r| r.iter().position(|c| *c == 0))
                    .ok_or(Miss::Truncated)?
                    + 1;
            }
            if let Some(picture) = b.get(i..).filter(|x| is_picture(x)) {
                return Ok(Some(picture.to_vec()));
            }
        }
        at = body + len;
    }
    Ok(None)
}
/// FLAC METADATA_BLOCK_PICTURE (block type 6).
fn flac_picture(d: &Sparse) -> Result<Option<Vec<u8>>, Miss> {
    let mut at = 4u64;
    for _ in 0..128 {
        let h = d.get(at, 4)?;
        let (last, kind) = (h[0] & 0x80 != 0, h[0] & 0x7F);
        let len = u64::from(h[1]) << 16 | u64::from(h[2]) << 8 | u64::from(h[3]);
        if kind == 6 && len <= MAX_PICTURE + 4096 {
            let b = d.get(at + 4, len)?;
            let field = |i: usize| be32(b, i).map(|v| v as usize).ok_or(Miss::Truncated);
            let mut i = 8 + field(4)?;
            i += 4 + field(i)? + 16;
            let data = field(i)?;
            return Ok(b
                .get(i + 4..i + 4 + data)
                .filter(|x| is_picture(x))
                .map(<[u8]>::to_vec));
        }
        if last {
            break;
        }
        at += 4 + len;
    }
    Ok(None)
}

// ----------------------------------------------------------------------------
// Video.

/// Boxes between `start` and `end`: (type, body, end).
fn boxes(d: &Sparse, start: u64, end: u64) -> Result<Vec<([u8; 4], u64, u64)>, Miss> {
    let mut out = Vec::new();
    let mut at = start;
    while at + 8 <= end && out.len() < 512 {
        let h = d.get(at, 8)?;
        let mut size = u64::from(be32(h, 0).unwrap_or(0));
        let kind = [h[4], h[5], h[6], h[7]];
        let mut body = at + 8;
        if size == 1 {
            size = d.be64(at + 8)?;
            body += 8;
        } else if size == 0 {
            size = end - at;
        }
        let stop = at
            .checked_add(size)
            .filter(|e| *e <= end && size >= body - at)
            .ok_or(Miss::Truncated)?;
        out.push((kind, body, stop));
        at = stop;
    }
    Ok(out)
}
fn child(d: &Sparse, parent: (u64, u64), kind: &[u8; 4]) -> Result<Option<(u64, u64)>, Miss> {
    Ok(boxes(d, parent.0, parent.1)?
        .into_iter()
        .find(|b| &b.0 == kind)
        .map(|b| (b.1, b.2)))
}
/// MP4/MOV: the iTunes cover (moov/udta/meta/ilst/covr), else the first
/// frame of a Motion-JPEG track.
fn mp4_picture(d: &Sparse) -> Result<Option<Vec<u8>>, Miss> {
    let Some(moov) = child(d, (0, d.size()), b"moov")? else {
        return Ok(None);
    };
    if let Some(udta) = child(d, moov, b"udta")?
        && let Some(meta) = child(d, udta, b"meta")?
    {
        // ISO meta is a full box (4 bytes of version and flags); QuickTime's
        // is not.
        let body = if meta.0 + 8 <= meta.1 && d.get(meta.0 + 4, 4)? == b"hdlr" {
            meta.0
        } else {
            meta.0 + 4
        };
        if let Some(ilst) = child(d, (body, meta.1), b"ilst")?
            && let Some(covr) = child(d, ilst, b"covr")?
            && let Some(data) = child(d, covr, b"data")?
            && data.1 - data.0 > 8
            && data.1 - data.0 - 8 <= MAX_PICTURE
        {
            let b = d.get(data.0 + 8, data.1 - data.0 - 8)?;
            if is_picture(b) {
                return Ok(Some(b.to_vec()));
            }
        }
    }
    for (kind, body, end) in boxes(d, moov.0, moov.1)? {
        if &kind != b"trak" {
            continue;
        }
        let Some(stbl) = child(d, (body, end), b"mdia")?
            .map(|m| child(d, m, b"minf"))
            .transpose()?
            .flatten()
            .map(|m| child(d, m, b"stbl"))
            .transpose()?
            .flatten()
        else {
            continue;
        };
        let Some(stsd) = child(d, stbl, b"stsd")? else {
            continue;
        };
        if !matches!(d.get(stsd.0 + 12, 4)?, b"jpeg" | b"mjpa" | b"mjpb") {
            continue;
        }
        let offset = if let Some(stco) = child(d, stbl, b"stco")? {
            u64::from(d.be32(stco.0 + 8)?)
        } else if let Some(co64) = child(d, stbl, b"co64")? {
            d.be64(co64.0 + 8)?
        } else {
            continue;
        };
        let Some(stsz) = child(d, stbl, b"stsz")? else {
            continue;
        };
        let fixed = d.be32(stsz.0 + 4)?;
        let len = u64::from(if fixed != 0 {
            fixed
        } else {
            d.be32(stsz.0 + 12)?
        });
        if len > MAX_PICTURE || offset.saturating_add(len) > d.size() {
            continue;
        }
        let b = d.get(offset, len)?;
        if is_picture(b) {
            return Ok(Some(with_huffman_tables(b)));
        }
    }
    Ok(None)
}
/// AVI: the first video chunk of `movi` when it is a JPEG (Motion-JPEG).
fn avi_frame(d: &Sparse) -> Result<Option<Vec<u8>>, Miss> {
    let size = d.size();
    let riff_end = (8 + u64::from(d.le32(4)?)).min(size);
    let mut at = 12u64;
    for _ in 0..256 {
        if at + 12 > riff_end {
            break;
        }
        let c = d.get(at, 12)?;
        let len = u64::from(le32(c, 4).unwrap_or(0));
        if &c[..4] == b"LIST" && &c[8..12] == b"movi" {
            let end = (at + 8 + len).min(riff_end);
            let mut m = at + 12;
            for _ in 0..4096 {
                if m + 8 > end {
                    break;
                }
                let s = d.get(m, 8)?;
                let chunk = u64::from(le32(s, 4).unwrap_or(0));
                if &s[..4] == b"LIST" {
                    m += 12;
                    continue;
                }
                if &s[2..4] == b"dc" || &s[2..4] == b"db" {
                    if chunk == 0 || chunk > MAX_PICTURE || m + 8 + chunk > end {
                        return Ok(None);
                    }
                    let b = d.get(m + 8, chunk)?;
                    return Ok(b.starts_with(b"\xFF\xD8").then(|| with_huffman_tables(b)));
                }
                m += 8 + chunk + (chunk & 1);
            }
            break;
        }
        at = at.saturating_add(8 + len + (len & 1));
    }
    Ok(None)
}

/// EBML element header at `at`: (id, data start, size; `u64::MAX` when the
/// size is unknown).
fn ebml(d: &Sparse, at: u64) -> Result<(u64, u64, u64), Miss> {
    let first = d.byte(at)?;
    let id_len = u64::from(first.leading_zeros()) + 1;
    if id_len > 4 {
        return Err(Miss::Truncated);
    }
    let id = d
        .get(at, id_len)?
        .iter()
        .fold(0, |v, c| v << 8 | u64::from(*c));
    let s = d.byte(at + id_len)?;
    let size_len = u64::from(s.leading_zeros()) + 1;
    if size_len > 8 {
        return Err(Miss::Truncated);
    }
    let raw = d.get(at + id_len, size_len)?;
    let mask = if size_len == 8 { 0 } else { 0xFF >> size_len };
    let size = raw[1..]
        .iter()
        .fold(u64::from(raw[0] & mask), |v, c| v << 8 | u64::from(*c));
    let unknown = size == (1u64 << (7 * size_len)) - 1;
    Ok((
        id,
        at + id_len + size_len,
        if unknown { u64::MAX } else { size },
    ))
}
/// Matroska attachments (cover.jpg and friends): the first JPEG or PNG,
/// preferring one whose name starts with "cover".
fn mkv_picture(d: &Sparse) -> Result<Option<Vec<u8>>, Miss> {
    let size = d.size();
    let (_, start, len) = ebml(d, 0)?;
    let mut at = start
        .checked_add(len)
        .filter(|a| *a < size)
        .ok_or(Miss::Truncated)?;
    let (id, body, len) = ebml(d, at)?;
    if id != 0x1853_8067 {
        return Ok(None);
    }
    let end = if len == u64::MAX {
        size
    } else {
        (body + len).min(size)
    };
    at = body;
    for _ in 0..4096 {
        if at + 2 > end {
            break;
        }
        let (id, data, len) = ebml(d, at)?;
        if len == u64::MAX {
            break;
        }
        if id == 0x1941_A469 {
            if len > 4 * MAX_PICTURE {
                return Ok(None);
            }
            let b = d.get(data, len.min(end - data))?;
            let mut best: Option<(bool, Vec<u8>)> = None;
            for (id, file) in ebml_children(b) {
                if id != 0x61A7 {
                    continue;
                }
                let (mut name, mut picture) = (String::new(), None);
                for (id, value) in ebml_children(file) {
                    match id {
                        0x466E => name = String::from_utf8_lossy(value).to_ascii_lowercase(),
                        0x465C if is_picture(value) => picture = Some(value),
                        _ => (),
                    }
                }
                if let Some(picture) = picture {
                    let cover = name.starts_with("cover");
                    if best.as_ref().is_none_or(|(c, _)| cover && !c) {
                        best = Some((cover, picture.to_vec()));
                    }
                }
            }
            return Ok(best.map(|(_, b)| b));
        }
        at = data.saturating_add(len);
    }
    Ok(None)
}
fn ebml_children(b: &[u8]) -> Vec<(u64, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < b.len() && out.len() < 256 {
        let id_len = b[at].leading_zeros() as usize + 1;
        let Some(id) = b.get(at..at + id_len).filter(|_| id_len <= 4) else {
            break;
        };
        let id = id.iter().fold(0, |v, c| v << 8 | u64::from(*c));
        let Some(&s) = b.get(at + id_len) else {
            break;
        };
        let size_len = s.leading_zeros() as usize + 1;
        let Some(raw) = b
            .get(at + id_len..at + id_len + size_len)
            .filter(|_| size_len <= 8)
        else {
            break;
        };
        let mask = if size_len == 8 { 0 } else { 0xFF >> size_len };
        let size = raw[1..]
            .iter()
            .fold(u64::from(raw[0] & mask), |v, c| v << 8 | u64::from(*c));
        let start = at + id_len + size_len;
        let end = usize::try_from(size)
            .ok()
            .and_then(|n| start.checked_add(n))
            .filter(|e| *e <= b.len())
            .unwrap_or(b.len());
        out.push((id, &b[start..end]));
        at = end;
    }
    out
}

/// Motion-JPEG frames usually leave out their Huffman tables and rely on
/// the standard ones (ITU T.81 Annex K.3); a frame without a DHT segment
/// gets them inserted after SOI so an ordinary JPEG decoder reads it.
fn with_huffman_tables(frame: &[u8]) -> Vec<u8> {
    let mut at = 2usize;
    while at + 4 <= frame.len() && frame[at] == 0xFF {
        let marker = frame[at + 1];
        if marker == 0xC4 {
            return frame.to_vec();
        }
        if marker == 0xDA {
            break;
        }
        at += 2 + usize::from(u16::from_be_bytes([frame[at + 2], frame[at + 3]]));
    }
    let mut out = Vec::with_capacity(frame.len() + 432);
    out.extend_from_slice(&frame[..2]);
    let tables: [(u8, &[u8; 16], &[u8]); 4] = [
        (0x00, &DC_BITS[0], &DC_VALUES),
        (0x10, &AC_LUMA_BITS, &AC_LUMA_VALUES),
        (0x01, &DC_BITS[1], &DC_VALUES),
        (0x11, &AC_CHROMA_BITS, &AC_CHROMA_VALUES),
    ];
    let len = 2 + tables.iter().map(|t| 17 + t.2.len()).sum::<usize>();
    out.extend_from_slice(&[0xFF, 0xC4]);
    out.extend_from_slice(&(len as u16).to_be_bytes());
    for (class, bits, values) in tables {
        out.push(class);
        out.extend_from_slice(bits);
        out.extend_from_slice(values);
    }
    out.extend_from_slice(&frame[2..]);
    out
}
pub(crate) const DC_BITS: [[u8; 16]; 2] = [
    [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0],
    [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0],
];
pub(crate) const DC_VALUES: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
pub(crate) const AC_LUMA_BITS: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
pub(crate) const AC_LUMA_VALUES: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0,
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7,
    0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5,
    0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2,
    0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];
pub(crate) const AC_CHROMA_BITS: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
pub(crate) const AC_CHROMA_VALUES: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71,
    0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0,
    0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26,
    0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
    0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5,
    0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3,
    0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda,
    0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    /// Reads `bytes` the way the preview pane does: fetch what is asked
    /// for until the parser has everything.
    fn run(name: &str, bytes: &[u8]) -> Preview {
        let mut d = Sparse::new(bytes.len() as u64);
        for _ in 0..10_000 {
            match read(name, &d) {
                Ok(p) => return p,
                Err(Miss::Need(at, len)) => {
                    let end = (at + len.max(512)).min(bytes.len() as u64);
                    d.insert(at, &bytes[at as usize..end as usize]);
                }
                Err(Miss::Truncated) => panic!("{name}: truncated"),
            }
        }
        panic!("{name}: no answer")
    }
    fn tar_header(name: &str, size: usize, kind: u8) -> Vec<u8> {
        let mut h = vec![0u8; 512];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..107].copy_from_slice(b"0000644");
        let octal = alloc::format!("{size:011o}");
        h[124..135].copy_from_slice(octal.as_bytes());
        h[156] = kind;
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        h[148..156].copy_from_slice(b"        ");
        let sum: u32 = h.iter().map(|c| u32::from(*c)).sum();
        h[148..155].copy_from_slice(alloc::format!("{sum:06o}\0").as_bytes());
        h
    }
    fn tar_bytes() -> Vec<u8> {
        let mut t = tar_header("docs/", 0, b'5');
        t.extend(tar_header("docs/readme.txt", 5, b'0'));
        t.extend(b"hello");
        t.resize(t.len().next_multiple_of(512), 0);
        t.extend(tar_header("top.bin", 3, b'0'));
        t.extend(b"abc");
        t.resize(t.len().next_multiple_of(512) + 1024, 0);
        t
    }

    #[test]
    fn zip_central_directory_lists_names_and_the_top_level() {
        // A stored ZIP: local headers are skipped, the central directory is read.
        let mut z = Vec::new();
        let mut central = Vec::new();
        for (name, data) in [("a/b.txt", &b"12345"[..]), ("c.txt", b"xy"), ("a/", b"")] {
            let offset = z.len() as u32;
            z.extend(b"PK\x03\x04");
            z.extend([0u8; 22]);
            z.extend((name.len() as u16).to_le_bytes());
            z.extend([0, 0]);
            z.extend(name.as_bytes());
            z.extend(data);
            central.extend(b"PK\x01\x02");
            central.extend([0u8; 4]);
            central.extend(0x800u16.to_le_bytes());
            central.extend([0u8; 10]);
            central.extend((data.len() as u32).to_le_bytes());
            central.extend((data.len() as u32).to_le_bytes());
            central.extend((name.len() as u16).to_le_bytes());
            central.extend([0u8; 12]);
            central.extend(offset.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let cd_at = z.len() as u32;
        z.extend(&central);
        z.extend(b"PK\x05\x06");
        z.extend([0u8; 4]);
        z.extend(3u16.to_le_bytes());
        z.extend(3u16.to_le_bytes());
        z.extend((central.len() as u32).to_le_bytes());
        z.extend(cd_at.to_le_bytes());
        z.extend([0, 0]);
        let p = run("x.zip", &z);
        assert_eq!((p.format, p.count), ("ZIP archive", 3));
        assert_eq!(
            p.entries[0],
            Entry {
                name: "a/b.txt".into(),
                size: 5,
                folder: false
            }
        );
        assert!(p.entries[2].folder);
        let root = p.root();
        assert_eq!(root.len(), 2);
        assert!(root[0].folder && root[0].name == "a");
        assert_eq!((root[1].name.as_str(), root[1].size), ("c.txt", 2));
    }

    #[test]
    fn tar_and_tar_gz_list_entries_with_folders() {
        let t = tar_bytes();
        let p = run("x.tar", &t);
        assert_eq!(p.format, "TAR archive");
        let names: Vec<_> = p
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e.size, e.folder))
            .collect();
        assert_eq!(
            names,
            [
                ("docs", 0, true),
                ("docs/readme.txt", 5, false),
                ("top.bin", 3, false)
            ]
        );
        // gzip of stored deflate blocks.
        let mut g = vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 3];
        for (i, block) in t.chunks(1000).enumerate() {
            let last = i == t.len().div_ceil(1000) - 1;
            g.push(u8::from(last));
            g.extend((block.len() as u16).to_le_bytes());
            g.extend((!(block.len() as u16)).to_le_bytes());
            g.extend(block);
        }
        g.extend([0u8; 4]);
        g.extend((t.len() as u32).to_le_bytes());
        let p = run("backup.tar.gz", &g);
        assert_eq!(p.format, "Compressed TAR archive (gzip)");
        assert_eq!(p.count, 3);
        assert!(!p.partial);
        // A plain .gz names its one member and its size.
        let p = run("notes.txt.gz", &g);
        assert_eq!(p.format, "GZIP archive");
        assert_eq!(p.entries[0].name, "notes.txt");
        assert_eq!(p.entries[0].size, t.len() as u64);
    }

    fn rar5_vint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let c = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                out.push(c);
                return;
            }
            out.push(c | 0x80);
        }
    }
    fn rar5_block(kind: u64, flags: u64, data: u64, body: &[u8]) -> Vec<u8> {
        let mut h = Vec::new();
        rar5_vint(kind, &mut h);
        rar5_vint(flags, &mut h);
        if flags & 2 != 0 {
            rar5_vint(data, &mut h);
        }
        h.extend(body);
        let mut out = vec![0u8; 4];
        rar5_vint(h.len() as u64, &mut out);
        out.extend(h);
        out.extend(vec![0u8; data as usize]);
        out
    }
    fn rar5_file(name: &str, size: u64, folder: bool, packed: u64) -> Vec<u8> {
        let mut b = Vec::new();
        rar5_vint(u64::from(folder), &mut b);
        rar5_vint(size, &mut b);
        rar5_vint(0x20, &mut b);
        rar5_vint(0, &mut b);
        rar5_vint(1, &mut b);
        rar5_vint(name.len() as u64, &mut b);
        b.extend(name.as_bytes());
        rar5_block(2, 2, packed, &b)
    }

    #[test]
    fn rar5_and_rar4_headers_list_entries() {
        let mut r = b"Rar!\x1A\x07\x01\x00".to_vec();
        r.extend(rar5_block(1, 0, 0, &[0]));
        r.extend(rar5_file("photos", 0, true, 0));
        r.extend(rar5_file("photos/cat.jpg", 123_456, false, 300));
        r.extend(rar5_file("readme.md", 42, false, 20));
        r.extend(rar5_block(5, 0, 0, &[0]));
        let p = run("x.rar", &r);
        assert_eq!(p.format, "RAR archive");
        let names: Vec<_> = p
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e.size, e.folder))
            .collect();
        assert_eq!(
            names,
            [
                ("photos", 0, true),
                ("photos/cat.jpg", 123_456, false),
                ("readme.md", 42, false)
            ]
        );
        // Encrypted headers: no names, an explanation.
        let mut e = b"Rar!\x1A\x07\x01\x00".to_vec();
        e.extend(rar5_block(4, 0, 0, &[0, 0, 0]));
        let p = run("secret.rar", &e);
        assert!(p.entries.is_empty() && !p.note.is_empty());

        let mut r4 = b"Rar!\x1A\x07\x00".to_vec();
        r4.extend([0, 0, 0x73, 0, 0, 13, 0, 0, 0, 0, 0, 0, 0]);
        for (name, size, folder) in [("dir", 0u32, true), ("dir\\a.txt", 77, false)] {
            let mut b = vec![0, 0, 0x74];
            let flags: u16 = 0x8000 | if folder { 0xE0 } else { 0 };
            b.extend(flags.to_le_bytes());
            b.extend((32 + name.len() as u16).to_le_bytes());
            b.extend(5u32.to_le_bytes());
            b.extend(size.to_le_bytes());
            b.extend([0u8; 10]);
            b.push(0x30);
            b.extend((name.len() as u16).to_le_bytes());
            b.extend([0u8; 4]);
            b.extend(name.as_bytes());
            b.extend([9u8; 5]);
            r4.extend(b);
        }
        r4.extend([0, 0, 0x7B, 0, 0x40, 7, 0]);
        let p = run("old.rar", &r4);
        let names: Vec<_> = p
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e.size, e.folder))
            .collect();
        assert_eq!(names, [("dir", 0, true), ("dir/a.txt", 77, false)]);
    }

    #[test]
    fn seven_zip_lists_its_files_and_hides_encrypted_names() {
        let entries = alloc::vec![
            (String::from("music"), None),
            (String::from("music/a.flac"), Some(alloc::vec![7u8; 900])),
            (String::from("notes.txt"), Some(b"hello".to_vec())),
        ];
        let s = crate::sevenzip::write(&entries, None, || [1; 16]);
        let p = run("x.7z", &s);
        assert_eq!(p.format, "7-Zip archive");
        assert_eq!(p.count, 3, "{:?}", p);
        assert!(
            p.entries
                .iter()
                .any(|e| e.name == "music/a.flac" && e.size == 900)
        );
        let locked = crate::sevenzip::write(&entries, Some("pw"), || [2; 16]);
        let p = run("y.7z", &locked);
        assert!(p.note.contains("encrypted"), "{}", p.note);
        let mut bad = s.clone();
        bad[13] ^= 0xFF; // the start header (its CRC no longer matches)
        assert!(!run("z.7z", &bad).note.is_empty());
    }

    fn wav(samples: &[i16]) -> Vec<u8> {
        let mut w = b"RIFF".to_vec();
        w.extend((36 + samples.len() as u32 * 2).to_le_bytes());
        w.extend(b"WAVEfmt ");
        w.extend(16u32.to_le_bytes());
        w.extend([1, 0, 1, 0]);
        w.extend(8000u32.to_le_bytes());
        w.extend(16000u32.to_le_bytes());
        w.extend([2, 0, 16, 0]);
        w.extend(b"data");
        w.extend((samples.len() as u32 * 2).to_le_bytes());
        for s in samples {
            w.extend(s.to_le_bytes());
        }
        w
    }

    #[test]
    fn wav_waveform_follows_the_signal_level() {
        // Silence for the first half, a full-scale square wave after.
        let samples: Vec<i16> = (0..9600)
            .map(|i| {
                if i < 4800 {
                    0
                } else if i % 2 == 0 {
                    i16::MAX
                } else {
                    i16::MIN
                }
            })
            .collect();
        let p = run("tone.wav", &wav(&samples));
        assert_eq!(p.peaks.len(), BUCKETS);
        assert!(p.peaks[..BUCKETS / 2 - 1].iter().all(|v| *v == 0));
        assert!(p.peaks[BUCKETS / 2 + 1..].iter().all(|v| *v == 255));
    }

    const JPEG: &[u8] =
        b"\xFF\xD8\xFF\xE0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00\xFF\xD9";

    #[test]
    fn cover_art_comes_from_id3_flac_and_mp4() {
        // ID3v2.3 APIC: encoding, MIME, type, description, picture.
        let mut frame = vec![0u8];
        frame.extend(b"image/jpeg\0");
        frame.push(3);
        frame.extend(b"cover\0");
        frame.extend(JPEG);
        let mut id3 = b"ID3\x03\x00\x00".to_vec();
        let size = 10 + frame.len();
        id3.extend([0, 0, (size >> 7) as u8, (size & 0x7F) as u8]);
        id3.extend(b"APIC");
        id3.extend((frame.len() as u32).to_be_bytes());
        id3.extend([0, 0]);
        id3.extend(&frame);
        id3.extend([0xFF, 0xFB, 0x90, 0x00]);
        assert_eq!(run("song.mp3", &id3).picture.as_deref(), Some(JPEG));

        let mut flac = b"fLaC".to_vec();
        let mut block = Vec::new();
        block.extend(3u32.to_be_bytes());
        block.extend(10u32.to_be_bytes());
        block.extend(b"image/jpeg");
        block.extend(0u32.to_be_bytes());
        block.extend([0u8; 16]);
        block.extend((JPEG.len() as u32).to_be_bytes());
        block.extend(JPEG);
        flac.extend([0, 0, 0, 34]);
        flac.extend([0u8; 34]);
        flac.push(0x86);
        flac.extend(&(block.len() as u32).to_be_bytes()[1..]);
        flac.extend(&block);
        assert_eq!(run("song.flac", &flac).picture.as_deref(), Some(JPEG));

        let atom = |kind: &[u8], body: &[u8]| {
            let mut a = ((8 + body.len()) as u32).to_be_bytes().to_vec();
            a.extend(kind);
            a.extend(body);
            a
        };
        let mut data = vec![0, 0, 0, 13, 0, 0, 0, 0];
        data.extend(JPEG);
        let ilst = atom(b"ilst", &atom(b"covr", &atom(b"data", &data)));
        let mut meta = vec![0u8; 4];
        meta.extend(atom(b"hdlr", &[0u8; 25]));
        meta.extend(ilst);
        let moov = atom(b"moov", &atom(b"udta", &atom(b"meta", &meta)));
        let mut mp4 = atom(b"ftyp", b"M4A \0\0\0\0");
        mp4.extend(moov);
        assert_eq!(run("song.m4a", &mp4).picture.as_deref(), Some(JPEG));
    }

    #[test]
    fn motion_jpeg_avi_frames_get_standard_huffman_tables() {
        let mut movi = b"movi".to_vec();
        movi.extend(b"00dc");
        movi.extend((JPEG.len() as u32).to_le_bytes());
        movi.extend(JPEG);
        let mut avi = b"RIFF".to_vec();
        avi.extend((4 + 8 + movi.len() as u32).to_le_bytes());
        avi.extend(b"AVI ");
        avi.extend(b"LIST");
        avi.extend((movi.len() as u32).to_le_bytes());
        avi.extend(&movi);
        let picture = run("clip.avi", &avi).picture.unwrap();
        assert!(picture.starts_with(b"\xFF\xD8\xFF\xC4"));
        assert_eq!(&picture[picture.len() - JPEG.len() + 2..], &JPEG[2..]);
        assert_eq!(
            AC_LUMA_BITS.iter().map(|b| usize::from(*b)).sum::<usize>(),
            AC_LUMA_VALUES.len()
        );
        assert_eq!(
            AC_CHROMA_BITS
                .iter()
                .map(|b| usize::from(*b))
                .sum::<usize>(),
            AC_CHROMA_VALUES.len()
        );
        // A frame that already has tables is left alone.
        let tabled = with_huffman_tables(&picture);
        assert_eq!(tabled, picture);
    }

    #[test]
    fn matroska_attachments_supply_the_cover() {
        let element = |id: &[u8], body: &[u8]| {
            let mut e = id.to_vec();
            e.push(0x01);
            e.extend((body.len() as u64).to_be_bytes()[1..].iter());
            e.extend(body);
            e
        };
        let mut file = element(&[0x46, 0x6E], b"cover.jpg");
        file.extend(element(&[0x46, 0x60], b"image/jpeg"));
        file.extend(element(&[0x46, 0x5C], JPEG));
        let attachments = element(&[0x19, 0x41, 0xA4, 0x69], &element(&[0x61, 0xA7], &file));
        let mut segment = element(&[0x15, 0x49, 0xA9, 0x66], &[0u8; 8]);
        segment.extend(attachments);
        let mut mkv = element(
            &[0x1A, 0x45, 0xDF, 0xA3],
            &[
                0x42, 0x82, 0x88, b'm', b'a', b't', b'r', b'o', b's', b'k', b'a',
            ],
        );
        mkv.extend(element(&[0x18, 0x53, 0x80, 0x67], &segment));
        assert_eq!(run("film.mkv", &mkv).picture.as_deref(), Some(JPEG));
    }
}
