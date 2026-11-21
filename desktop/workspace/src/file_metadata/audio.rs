//! Audio: WAV, MP3 (ID3v2/ID3v1, Xing/Info/VBRI), FLAC and Ogg
//! (Vorbis, Opus, FLAC, Theora).
use super::Section::{Audio, Description, Media, Origin, Video};
use super::*;

/// RIFF INFO list entries (WAV and AVI).
pub(super) fn riff_info(b: &[u8], r: &mut Report) {
    let mut at = 0usize;
    for _ in 0..64 {
        let (Some(id), Some(len)) = (b.get(at..at + 4), le32(b, at + 4)) else {
            return;
        };
        let len = len as usize;
        let value = b
            .get(at + 8..(at + 8).saturating_add(len))
            .map(latin1)
            .unwrap_or_default();
        match id {
            b"INAM" => r.add(Description, "Title", value),
            b"IART" => r.add(Media, "Contributing artists", value),
            b"IPRD" => r.add(Media, "Album", value),
            b"ICRD" => r.add(Media, "Year", value),
            b"IGNR" => r.add(Media, "Genre", value),
            b"ICMT" => r.add(Description, "Comments", value),
            b"ICOP" => r.add(Origin, "Copyright", value),
            b"ISFT" => r.add(Origin, "Encoding software", value),
            b"ITRK" | b"IPRT" => r.add(Media, "#", value),
            _ => (),
        }
        at = at.saturating_add(8).saturating_add(len + (len & 1));
    }
}

fn wave_format(tag: u16) -> &'static str {
    match tag {
        1 => "PCM",
        2 => "Microsoft ADPCM",
        3 => "IEEE float",
        6 => "A-law",
        7 => "mu-law",
        0x11 => "IMA ADPCM",
        0x55 => "MP3",
        0xFF | 0x1610 => "AAC",
        0x2000 => "Dolby Digital (AC-3)",
        _ => "",
    }
}

pub(super) fn wav(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    r.format("WAVE audio");
    let end = d.size();
    let mut at = 12u64;
    let (mut byte_rate, mut data) = (0u64, None);
    for _ in 0..64 {
        if at + 8 > end {
            break;
        }
        let h = d.get(at, 8)?;
        let len = u64::from(le32(h, 4).unwrap_or(0));
        let body = at + 8;
        match &h[..4] {
            b"fmt " if len >= 16 => {
                let b = d.get(body, len.min(40))?;
                let mut tag = le16(b, 0).unwrap_or(0);
                if tag == 0xFFFE {
                    tag = le16(b, 24).unwrap_or(0);
                }
                r.add(Audio, "Format", wave_format(tag));
                r.add(
                    Audio,
                    "Channels",
                    channels(u64::from(le16(b, 2).unwrap_or(0))),
                );
                r.add(Audio, "Sample rate", hz(u64::from(le32(b, 4).unwrap_or(0))));
                byte_rate = u64::from(le32(b, 8).unwrap_or(0));
                r.add(Audio, "Bit rate", kbps(byte_rate * 8));
                let bits = le16(b, 14).unwrap_or(0);
                if bits > 0 {
                    r.add(Audio, "Sample size", format!("{bits} bit"));
                }
            }
            b"data" => {
                // Streaming writers leave the size open (0 or 0xFFFFFFFF).
                let open = len == 0 || len == 0xFFFF_FFFF;
                data = Some(if open {
                    end.saturating_sub(body)
                } else {
                    len.min(end.saturating_sub(body))
                });
            }
            b"LIST" if (4..=65_536).contains(&len) => {
                let b = d.get(body, len.min(end - body))?;
                if b.starts_with(b"INFO") {
                    riff_info(&b[4..], r);
                }
            }
            _ => (),
        }
        at = body.saturating_add(len + (len & 1));
    }
    if let (Some(bytes), true) = (data, byte_rate > 0)
        && let Some(ms) = ms(bytes, byte_rate)
    {
        r.add(Media, "Length", length_ms(ms));
    }
    Ok(())
}

// ----------------------------------------------------------------------------
// ID3 and MPEG audio.

const GENRES: [&str; 80] = [
    "Blues",
    "Classic Rock",
    "Country",
    "Dance",
    "Disco",
    "Funk",
    "Grunge",
    "Hip-Hop",
    "Jazz",
    "Metal",
    "New Age",
    "Oldies",
    "Other",
    "Pop",
    "R&B",
    "Rap",
    "Reggae",
    "Rock",
    "Techno",
    "Industrial",
    "Alternative",
    "Ska",
    "Death Metal",
    "Pranks",
    "Soundtrack",
    "Euro-Techno",
    "Ambient",
    "Trip-Hop",
    "Vocal",
    "Jazz+Funk",
    "Fusion",
    "Trance",
    "Classical",
    "Instrumental",
    "Acid",
    "House",
    "Game",
    "Sound Clip",
    "Gospel",
    "Noise",
    "AlternRock",
    "Bass",
    "Soul",
    "Punk",
    "Space",
    "Meditative",
    "Instrumental Pop",
    "Instrumental Rock",
    "Ethnic",
    "Gothic",
    "Darkwave",
    "Techno-Industrial",
    "Electronic",
    "Pop-Folk",
    "Eurodance",
    "Dream",
    "Southern Rock",
    "Comedy",
    "Cult",
    "Gangsta",
    "Top 40",
    "Christian Rap",
    "Pop/Funk",
    "Jungle",
    "Native American",
    "Cabaret",
    "New Wave",
    "Psychedelic",
    "Rave",
    "Showtunes",
    "Trailer",
    "Lo-Fi",
    "Tribal",
    "Acid Punk",
    "Acid Jazz",
    "Polka",
    "Retro",
    "Musical",
    "Rock & Roll",
    "Hard Rock",
];
fn genre(value: &str) -> String {
    let v = value.trim();
    let (number, rest) = match v.strip_prefix('(').and_then(|s| s.split_once(')')) {
        Some((n, rest)) => (n.parse::<usize>().ok(), rest.trim()),
        None => (v.parse::<usize>().ok(), ""),
    };
    if !rest.is_empty() {
        return String::from(rest);
    }
    match number {
        Some(n) => GENRES
            .get(n)
            .map_or_else(|| String::from(v), |g| String::from(*g)),
        None => String::from(v),
    }
}
fn syncsafe(b: &[u8]) -> u64 {
    b.iter()
        .take(4)
        .fold(0, |v, c| (v << 7) | u64::from(c & 0x7F))
}
/// An ID3v2 text value: encoding byte, then text (first of several values).
fn id3_text(b: &[u8]) -> String {
    let Some((encoding, text)) = b.split_first() else {
        return String::new();
    };
    match encoding {
        1 => match text {
            [0xFE, 0xFF, rest @ ..] => utf16(rest, false),
            [0xFF, 0xFE, rest @ ..] => utf16(rest, true),
            _ => utf16(text, true),
        },
        2 => utf16(text, false),
        3 => utf8(text),
        _ => latin1(text),
    }
}
/// COMM: encoding, language, NUL-terminated description, then the text.
fn id3_comment(b: &[u8]) -> String {
    let Some(&encoding) = b.first() else {
        return String::new();
    };
    let rest = b.get(4..).unwrap_or(&[]);
    let skip = if matches!(encoding, 1 | 2) {
        (0..rest.len() / 2)
            .find(|i| rest[2 * i] == 0 && rest[2 * i + 1] == 0)
            .map(|i| 2 * i + 2)
    } else {
        rest.iter().position(|c| *c == 0).map(|i| i + 1)
    };
    let text = rest.get(skip.unwrap_or(rest.len())..).unwrap_or(&[]);
    let mut owned = alloc::vec![encoding];
    if encoding == 1 && !text.starts_with(&[0xFF, 0xFE]) && !text.starts_with(&[0xFE, 0xFF]) {
        // The BOM belongs to the description; reuse its byte order.
        owned.extend_from_slice(rest.get(..2).unwrap_or(&[]));
    }
    owned.extend_from_slice(text);
    id3_text(&owned)
}
/// Parses an ID3v2 tag at offset 0; returns where the audio starts and the
/// length the tag states (TLEN, milliseconds), if any.
fn id3v2(d: &Sparse, r: &mut Report) -> Result<(u64, Option<u64>), Miss> {
    let h = d.get(0, 10)?;
    let (major, flags) = (h[3], h[5]);
    let size = syncsafe(&h[6..10]);
    let end = 10 + size + if flags & 0x10 != 0 { 10 } else { 0 };
    let tag_end = (10 + size).min(d.size());
    let mut length = None;
    if flags & 0x80 != 0 || !(2..=4).contains(&major) {
        // Whole-tag unsynchronisation (rare): the frames are not read.
        return Ok((end, None));
    }
    let mut at = 10u64;
    if flags & 0x40 != 0 && major >= 3 {
        let b = d.get(at, 4)?;
        at += if major == 4 {
            syncsafe(b)
        } else {
            u64::from(be32(b, 0).unwrap_or(0)) + 4
        };
    }
    for _ in 0..256 {
        let header = if major == 2 { 6 } else { 10 };
        if at + header > tag_end {
            break;
        }
        let fh = d.get(at, header)?;
        if fh[0] == 0 {
            break;
        }
        let (id, len) = if major == 2 {
            (
                &fh[..3],
                u64::from(fh[3]) << 16 | u64::from(fh[4]) << 8 | u64::from(fh[5]),
            )
        } else if major == 4 {
            (&fh[..4], syncsafe(&fh[4..8]))
        } else {
            (&fh[..4], u64::from(be32(fh, 4).unwrap_or(0)))
        };
        let body = at + header;
        if body + len > tag_end {
            break;
        }
        let target = match id {
            b"TIT2" | b"TT2" => Some((Description, "Title")),
            b"TIT3" | b"TT3" => Some((Description, "Subtitle")),
            b"TPE1" | b"TP1" => Some((Media, "Contributing artists")),
            b"TPE2" | b"TP2" => Some((Media, "Album artist")),
            b"TALB" | b"TAL" => Some((Media, "Album")),
            b"TYER" | b"TYE" | b"TDRC" => Some((Media, "Year")),
            b"TRCK" | b"TRK" => Some((Media, "#")),
            b"TCON" | b"TCO" => Some((Media, "Genre")),
            b"TCOM" | b"TCM" => Some((Media, "Composers")),
            b"TPOS" | b"TPA" => Some((Media, "Part of set")),
            b"TBPM" | b"TBP" => Some((Media, "Beats-per-minute")),
            b"TPUB" | b"TPB" => Some((Origin, "Publisher")),
            b"TENC" | b"TEN" => Some((Origin, "Encoded by")),
            b"TSSE" | b"TSS" => Some((Origin, "Encoding software")),
            b"TCOP" | b"TCR" => Some((Origin, "Copyright")),
            _ => None,
        };
        if let Some((section, label)) = target {
            let text = id3_text(d.get(body, len.min(1024))?);
            let text = match label {
                "Genre" => genre(&text),
                "Year" => String::from(
                    text.get(..4)
                        .filter(|y| y.bytes().all(|c| c.is_ascii_digit()))
                        .unwrap_or(&text),
                ),
                _ => text,
            };
            r.add(section, label, text);
        } else if id == b"COMM" || id == b"COM" {
            r.add(
                Description,
                "Comments",
                id3_comment(d.get(body, len.min(1024))?),
            );
        } else if id == b"TLEN" || id == b"TLE" {
            length = id3_text(d.get(body, len.min(32))?)
                .trim()
                .parse::<u64>()
                .ok()
                .filter(|ms| *ms > 0);
        } else if id == b"APIC" || id == b"PIC" {
            r.add(Media, "Cover art", "Embedded");
        }
        at = body + len;
    }
    Ok((end, length))
}
/// The ID3v1 tag in the last 128 bytes (fills only what ID3v2 left out).
fn id3v1(d: &Sparse, r: &mut Report) -> Result<bool, Miss> {
    if d.size() < 128 + 3 {
        return Ok(false);
    }
    let b = d.get(d.size() - 128, 128)?;
    if &b[..3] != b"TAG" {
        return Ok(false);
    }
    r.add(Description, "Title", latin1(&b[3..33]));
    r.add(Media, "Contributing artists", latin1(&b[33..63]));
    r.add(Media, "Album", latin1(&b[63..93]));
    r.add(Media, "Year", latin1(&b[93..97]));
    if b[125] == 0 && b[126] != 0 {
        r.add(Description, "Comments", latin1(&b[97..125]));
        r.add(Media, "#", format!("{}", b[126]));
    } else {
        r.add(Description, "Comments", latin1(&b[97..127]));
    }
    if let Some(g) = GENRES.get(usize::from(b[127])) {
        r.add(Media, "Genre", *g);
    }
    Ok(true)
}

#[derive(Clone, Copy)]
struct Frame {
    /// 10 = MPEG-1, 20 = MPEG-2, 25 = MPEG-2.5.
    version: u8,
    layer: u8,
    kbps: u64,
    rate: u64,
    padding: u64,
    mode: u8,
}
impl Frame {
    fn parse(b: &[u8]) -> Option<Self> {
        let b = b.get(..4)?;
        if b[0] != 0xFF || b[1] & 0xE0 != 0xE0 {
            return None;
        }
        let version = match (b[1] >> 3) & 3 {
            0 => 25,
            2 => 20,
            3 => 10,
            _ => return None,
        };
        let layer = match (b[1] >> 1) & 3 {
            1 => 3,
            2 => 2,
            3 => 1,
            _ => return None,
        };
        let index = usize::from(b[2] >> 4);
        if index == 0 || index == 15 {
            return None;
        }
        let table: [u64; 14] = match (version, layer) {
            (10, 1) => [
                32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448,
            ],
            (10, 2) => [
                32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
            ],
            (10, _) => [
                32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
            ],
            (_, 1) => [
                32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256,
            ],
            _ => [8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
        };
        let base = match (b[2] >> 2) & 3 {
            0 => 44_100,
            1 => 48_000,
            2 => 32_000,
            _ => return None,
        };
        let rate = match version {
            10 => base,
            20 => base / 2,
            _ => base / 4,
        };
        Some(Self {
            version,
            layer,
            kbps: table[index - 1],
            rate,
            padding: u64::from((b[2] >> 1) & 1),
            mode: b[3] >> 6,
        })
    }
    fn len(self) -> u64 {
        match self.layer {
            1 => (12 * self.kbps * 1000 / self.rate + self.padding) * 4,
            3 if self.version != 10 => 72 * self.kbps * 1000 / self.rate + self.padding,
            _ => 144 * self.kbps * 1000 / self.rate + self.padding,
        }
    }
    fn samples(self) -> u64 {
        match self.layer {
            1 => 384,
            3 if self.version != 10 => 576,
            _ => 1152,
        }
    }
}
pub(super) fn is_mpeg_frame(head: &[u8]) -> bool {
    Frame::parse(head).is_some()
}
/// The first frame at or after `start` whose successor also syncs.
fn first_frame(window: &[u8]) -> Option<(usize, Frame)> {
    (0..window.len().saturating_sub(4)).find_map(|i| {
        let f = Frame::parse(&window[i..])?;
        let next = i + f.len() as usize;
        let confirmed = next + 4 > window.len()
            || Frame::parse(&window[next..])
                .is_some_and(|g| g.version == f.version && g.layer == f.layer);
        confirmed.then_some((i, f))
    })
}

pub(super) fn id3_file(d: &Sparse, r: &mut Report, ext: &str) -> Result<(), Miss> {
    let (start, length) = id3v2(d, r)?;
    if start + 4 <= d.size() && d.get(start, 4)? == b"fLaC" {
        return flac(d, r, start);
    }
    if let Some(ms) = length {
        r.add(Media, "Length", length_ms(ms));
    }
    let _ = ext;
    mp3(d, r, start)
}

pub(super) fn mp3(d: &Sparse, r: &mut Report, start: u64) -> Result<(), Miss> {
    r.format("MP3 audio");
    let mut found = None;
    for limit in [4096u64, 65_536] {
        let window = d.clip(start, limit)?;
        if let Some((i, f)) = first_frame(window) {
            found = Some((start + i as u64, f, window.get(i..).unwrap_or(&[]).to_vec()));
            break;
        }
        if (window.len() as u64) < limit {
            break;
        }
    }
    let tail = id3v1(d, r)?;
    let Some((first, f, frame)) = found else {
        return Ok(());
    };
    r.add(
        Audio,
        "Format",
        format!(
            "MPEG-{} Layer {}",
            match f.version {
                10 => "1",
                20 => "2",
                _ => "2.5",
            },
            match f.layer {
                1 => "I",
                2 => "II",
                _ => "III",
            }
        ),
    );
    r.add(Audio, "Sample rate", hz(f.rate));
    r.add(
        Audio,
        "Channels",
        match f.mode {
            0 => "2 (stereo)",
            1 => "2 (joint stereo)",
            2 => "2 (dual channel)",
            _ => "1 (mono)",
        },
    );
    let mono = f.mode == 3;
    let side = match (f.version == 10, mono) {
        (true, true) => 17,
        (true, false) => 32,
        (false, true) => 9,
        (false, false) => 17,
    };
    let (mut frames, mut bytes, mut vbr) = (None, None, false);
    if let Some(x) = frame
        .get(4 + side..)
        .filter(|x| x.starts_with(b"Xing") || x.starts_with(b"Info"))
    {
        let flags = be32(x, 4).unwrap_or(0);
        let mut at = 8;
        if flags & 1 != 0 {
            frames = be32(x, at).map(u64::from);
            at += 4;
        }
        if flags & 2 != 0 {
            bytes = be32(x, at).map(u64::from);
        }
        vbr = x.starts_with(b"Xing");
    } else if let Some(v) = frame.get(36..).filter(|v| v.starts_with(b"VBRI")) {
        bytes = be32(v, 10).map(u64::from);
        frames = be32(v, 14).map(u64::from);
        vbr = true;
    }
    let audio = d
        .size()
        .saturating_sub(first)
        .saturating_sub(if tail { 128 } else { 0 });
    let (ms_total, bits) = match frames.filter(|n| *n > 0) {
        Some(n) => {
            let ms_total = ms(n * f.samples(), f.rate).unwrap_or(0);
            let bits = if ms_total > 0 {
                bytes.unwrap_or(audio) * 8 * 1000 / ms_total
            } else {
                f.kbps * 1000
            };
            (Some(ms_total), bits)
        }
        None => (ms(audio * 8, f.kbps * 1000), f.kbps * 1000),
    };
    r.add(
        Audio,
        "Bit rate",
        format!("{}{}", kbps(bits), if vbr { " (variable)" } else { "" }),
    );
    if let Some(ms) = ms_total.filter(|ms| *ms > 0) {
        r.add(Media, "Length", length_ms(ms));
    }
    Ok(())
}

// ----------------------------------------------------------------------------
// FLAC and Ogg.

/// Vorbis comments (FLAC VORBIS_COMMENT, Ogg Vorbis/Opus tags).
pub(super) fn vorbis_comments(b: &[u8], r: &mut Report) {
    let Some(vendor) = le32(b, 0) else { return };
    let mut at = 4usize.saturating_add(vendor as usize);
    if let Some(v) = b.get(4..at) {
        r.add(Origin, "Encoding software", utf8(v));
    }
    let count = le32(b, at).unwrap_or(0).min(512);
    at += 4;
    for _ in 0..count {
        let Some(len) = le32(b, at) else { return };
        at += 4;
        let Some(c) = b.get(at..at.saturating_add(len as usize)) else {
            return;
        };
        at += len as usize;
        let Some(eq) = c.iter().position(|x| *x == b'=') else {
            continue;
        };
        let (key, value) = (&c[..eq], utf8(&c[eq + 1..]));
        let is = |k: &str| key.eq_ignore_ascii_case(k.as_bytes());
        if is("TITLE") {
            r.add(Description, "Title", value);
        } else if is("ARTIST") {
            r.add(Media, "Contributing artists", value);
        } else if is("ALBUMARTIST") || is("ALBUM ARTIST") {
            r.add(Media, "Album artist", value);
        } else if is("ALBUM") {
            r.add(Media, "Album", value);
        } else if is("DATE") || is("YEAR") {
            r.add(Media, "Year", value);
        } else if is("TRACKNUMBER") {
            r.add(Media, "#", value);
        } else if is("GENRE") {
            r.add(Media, "Genre", value);
        } else if is("COMPOSER") {
            r.add(Media, "Composers", value);
        } else if is("DISCNUMBER") {
            r.add(Media, "Part of set", value);
        } else if is("COMMENT") || is("DESCRIPTION") {
            r.add(Description, "Comments", value);
        } else if is("COPYRIGHT") {
            r.add(Origin, "Copyright", value);
        } else if is("ORGANIZATION") || is("LABEL") {
            r.add(Origin, "Publisher", value);
        } else if is("METADATA_BLOCK_PICTURE") {
            r.add(Media, "Cover art", "Embedded");
        }
    }
}
/// STREAMINFO: (sample rate, channels, bits per sample, total samples).
fn streaminfo(b: &[u8]) -> Option<(u64, u64, u64, u64)> {
    let v = be64(b, 10)?;
    Some((
        v >> 44,
        ((v >> 41) & 7) + 1,
        ((v >> 36) & 0x1F) + 1,
        v & 0xF_FFFF_FFFF,
    ))
}
fn stream_fields(r: &mut Report, rate: u64, ch: u64, bits: Option<u64>) {
    r.add(Audio, "Sample rate", hz(rate));
    r.add(Audio, "Channels", channels(ch));
    if let Some(bits) = bits {
        r.add(Audio, "Sample size", format!("{bits} bit"));
    }
}

pub(super) fn flac(d: &Sparse, r: &mut Report, start: u64) -> Result<(), Miss> {
    r.format("FLAC audio");
    r.add(Audio, "Format", "FLAC (lossless)");
    let mut at = start + 4;
    let mut info = None;
    for _ in 0..128 {
        let h = d.get(at, 4)?;
        let (last, kind) = (h[0] & 0x80 != 0, h[0] & 0x7F);
        let len = u64::from(h[1]) << 16 | u64::from(h[2]) << 8 | u64::from(h[3]);
        let body = at + 4;
        match kind {
            0 if len >= 18 => info = streaminfo(d.get(body, 18)?),
            4 => vorbis_comments(
                d.get(body, len.min(65_536).min(d.size().saturating_sub(body)))?,
                r,
            ),
            6 => r.add(Media, "Cover art", "Embedded"),
            _ => (),
        }
        at = body + len;
        if last {
            break;
        }
    }
    if let Some((rate, ch, bits, total)) = info {
        stream_fields(r, rate, ch, Some(bits));
        if let Some(ms) = ms(total, rate).filter(|ms| *ms > 0) {
            r.add(Media, "Length", length_ms(ms));
            r.add(
                Audio,
                "Bit rate",
                kbps(d.size().saturating_sub(at) * 8 * 1000 / ms),
            );
        }
    }
    Ok(())
}

pub(super) fn ogg(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    let mut at = 0u64;
    let mut serial = None;
    let mut packets: Vec<Vec<u8>> = Vec::new();
    let mut current = Vec::new();
    'pages: for _ in 0..64 {
        let h = d.get(at, 27)?;
        if &h[..4] != b"OggS" {
            break;
        }
        let s = le32(h, 14).unwrap_or(0);
        let first = *serial.get_or_insert(s);
        let segments = d.get(at + 27, u64::from(h[26]))?.to_vec();
        let mut body = at + 27 + u64::from(h[26]);
        for len in segments {
            if s == first && current.len() < 65_536 {
                current.extend_from_slice(d.get(body, u64::from(len))?);
            }
            body += u64::from(len);
            if len < 255 && s == first {
                packets.push(core::mem::take(&mut current));
                if packets.len() == 2 {
                    break 'pages;
                }
            }
        }
        at = body;
    }
    let id = packets.first().map(Vec::as_slice).unwrap_or(&[]);
    let tags = packets.get(1).map(Vec::as_slice).unwrap_or(&[]);
    // (sample rate for the granule clock, granules to skip)
    let mut clock = None;
    if id.starts_with(b"\x01vorbis") {
        r.format("Ogg Vorbis audio");
        r.add(Audio, "Format", "Vorbis");
        let rate = u64::from(le32(id, 12).unwrap_or(0));
        stream_fields(r, rate, u64::from(*id.get(11).unwrap_or(&0)), None);
        let nominal = le32(id, 20).unwrap_or(0) as i32;
        if nominal > 0 {
            r.add(Audio, "Bit rate", kbps(nominal as u64));
        }
        clock = Some((rate, 0));
        if let Some(c) = tags.strip_prefix(b"\x03vorbis") {
            vorbis_comments(c, r);
        }
    } else if id.starts_with(b"OpusHead") {
        r.format("Ogg Opus audio");
        r.add(Audio, "Format", "Opus");
        stream_fields(
            r,
            u64::from(le32(id, 12).unwrap_or(0)),
            u64::from(*id.get(9).unwrap_or(&0)),
            None,
        );
        clock = Some((48_000, u64::from(le16(id, 10).unwrap_or(0))));
        if let Some(c) = tags.strip_prefix(b"OpusTags") {
            vorbis_comments(c, r);
        }
    } else if id.starts_with(b"\x7FFLAC") {
        r.format("Ogg FLAC audio");
        r.add(Audio, "Format", "FLAC (lossless)");
        if let Some((rate, ch, bits, _)) = id.get(13..).and_then(streaminfo) {
            stream_fields(r, rate, ch, Some(bits));
            clock = Some((rate, 0));
        }
        if tags.first().map(|b| b & 0x7F) == Some(4) {
            vorbis_comments(tags.get(4..).unwrap_or(&[]), r);
        }
    } else if id.starts_with(b"\x80theora") {
        r.format("Ogg Theora video");
        let w = id.get(14..17).map_or(0, |b| {
            u64::from(b[0]) << 16 | u64::from(b[1]) << 8 | u64::from(b[2])
        });
        let h = id.get(17..20).map_or(0, |b| {
            u64::from(b[0]) << 16 | u64::from(b[1]) << 8 | u64::from(b[2])
        });
        r.add(Video, "Frame width", grouped(w));
        r.add(Video, "Frame height", grouped(h));
        let (n, dn) = (be32(id, 22).unwrap_or(0), be32(id, 26).unwrap_or(0));
        if n > 0 && dn > 0 {
            r.add(
                Video,
                "Frame rate",
                frame_rate(u64::from(n) * 100 / u64::from(dn)),
            );
        }
        r.add(Video, "Video codec", "Theora");
    } else {
        r.format("Ogg media");
    }
    if let (Some((rate, skip)), Some(serial)) = (clock, serial) {
        let tail_len = d.size().min(65_536);
        let tail = d.get(d.size() - tail_len, tail_len)?;
        let mut end = tail.len();
        for _ in 0..256 {
            let Some(i) = rfind(&tail[..end], b"OggS") else {
                break;
            };
            if le32(tail, i + 14) == Some(serial) {
                let granule = le64(tail, i + 6).unwrap_or(0);
                if granule != u64::MAX
                    && let Some(ms) = ms(granule.saturating_sub(skip), rate).filter(|ms| *ms > 0)
                {
                    r.add(Media, "Length", length_ms(ms));
                    r.add(Audio, "Bit rate", kbps(d.size() * 8 * 1000 / ms));
                }
                break;
            }
            end = i;
        }
    }
    Ok(())
}
/// "29.97 frames/second" from hundredths.
pub(super) fn frame_rate(hundredths: u64) -> String {
    if hundredths.is_multiple_of(100) {
        format!("{} frames/second", hundredths / 100)
    } else {
        format!("{}.{:02} frames/second", hundredths / 100, hundredths % 100)
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    #[test]
    fn wav_format_length_and_info() {
        let mut w = b"RIFF".to_vec();
        w.extend(le(0, 4));
        w.extend(b"WAVEfmt ");
        w.extend(le(16, 4));
        w.extend(le(1, 2));
        w.extend(le(2, 2));
        w.extend(le(44_100, 4));
        w.extend(le(176_400, 4));
        w.extend(le(4, 2));
        w.extend(le(16, 2));
        w.extend(b"LIST");
        w.extend(le(18, 4));
        w.extend(b"INFOINAM");
        w.extend(le(6, 4));
        w.extend(b"Intro\0");
        w.extend(b"data");
        w.extend(le(352_800, 4));
        w.extend(alloc::vec![0; 352_800]);
        let r = run("a.wav", &w);
        assert_eq!(r.get("Format"), Some("PCM"));
        assert_eq!(r.get("Channels"), Some("2 (stereo)"));
        assert_eq!(r.get("Sample rate"), Some("44.1 kHz"));
        assert_eq!(r.get("Sample size"), Some("16 bit"));
        assert_eq!(r.get("Bit rate"), Some("1,411 kbps"));
        assert_eq!(r.get("Length"), Some("00:00:02"));
        assert_eq!(r.get("Title"), Some("Intro"));
        fuzz("a.wav", &w[..400]);
    }
    fn text_frame(id: &[u8], text: &str) -> Vec<u8> {
        let mut f = id.to_vec();
        f.extend(be(text.len() as u64 + 1, 4));
        f.extend([0, 0, 3]);
        f.extend(text.as_bytes());
        f
    }
    fn mp3_file() -> Vec<u8> {
        let mut frames = Vec::new();
        frames.extend(text_frame(b"TIT2", "Song"));
        frames.extend(text_frame(b"TPE1", "Band"));
        frames.extend(text_frame(b"TALB", "Record"));
        frames.extend(text_frame(b"TDRC", "2021-04-02"));
        frames.extend(text_frame(b"TRCK", "3/12"));
        frames.extend(text_frame(b"TCON", "(17)"));
        let mut apic = b"APIC".to_vec();
        apic.extend(be(2000, 4));
        apic.extend([0, 0]);
        apic.extend([0; 2000]);
        frames.extend(apic);
        frames.extend([0; 20]);
        let mut m = b"ID3\x03\0\0".to_vec();
        let size = frames.len() as u64;
        m.extend([
            (size >> 21) as u8 & 0x7F,
            (size >> 14) as u8 & 0x7F,
            (size >> 7) as u8 & 0x7F,
            size as u8 & 0x7F,
        ]);
        m.extend(frames);
        // MPEG-1 Layer III, 128 kbps, 44.1 kHz, joint stereo; Xing header.
        let header = [0xFF, 0xFB, 0x90, 0x44];
        let len = 144 * 128_000 / 44_100;
        let mut first = header.to_vec();
        first.extend([0; 32]);
        first.extend(b"Xing");
        first.extend(be(3, 4));
        first.extend(be(1000, 4));
        first.extend(be(400_000, 4));
        first.resize(len, 0);
        m.extend(first);
        for _ in 0..3 {
            let mut f = header.to_vec();
            f.resize(len, 0);
            m.extend(f);
        }
        m
    }
    #[test]
    fn mp3_id3v2_tags_and_xing_length() {
        let r = run("song.mp3", &mp3_file());
        assert_eq!(r.format, Some("MP3 audio"));
        assert_eq!(r.get("Title"), Some("Song"));
        assert_eq!(r.get("Contributing artists"), Some("Band"));
        assert_eq!(r.get("Album"), Some("Record"));
        assert_eq!(r.get("Year"), Some("2021"));
        assert_eq!(r.get("#"), Some("3/12"));
        assert_eq!(r.get("Genre"), Some("Rock"));
        assert_eq!(r.get("Cover art"), Some("Embedded"));
        assert_eq!(r.get("Format"), Some("MPEG-1 Layer III"));
        assert_eq!(r.get("Channels"), Some("2 (joint stereo)"));
        // 1000 frames x 1152 samples at 44.1 kHz = 26.1 s.
        assert_eq!(r.get("Length"), Some("00:00:26"));
        assert_eq!(r.get("Bit rate"), Some("123 kbps (variable)"));
        fuzz("song.mp3", &mp3_file());
        // CBR estimate without Xing, and an ID3v1 tag.
        let mut cbr = alloc::vec![];
        for _ in 0..100 {
            let mut f = alloc::vec![0xFF, 0xFB, 0x90, 0x00];
            f.resize(417, 0);
            cbr.extend(f);
        }
        let mut v1 = b"TAG".to_vec();
        v1.extend(b"Old title");
        v1.resize(127, 0);
        v1.push(17);
        cbr.extend(v1);
        let r = run("old.mp3", &cbr);
        assert_eq!(
            (r.get("Title"), r.get("Genre"), r.get("Bit rate")),
            (Some("Old title"), Some("Rock"), Some("128 kbps"))
        );
        assert_eq!(r.get("Length"), Some("00:00:03"));
        fuzz("old.mp3", &cbr);
    }
    fn comments(entries: &[&str]) -> Vec<u8> {
        let mut c = le(6, 4);
        c.extend(b"Tester");
        c.extend(le(entries.len() as u64, 4));
        for e in entries {
            c.extend(le(e.len() as u64, 4));
            c.extend(e.as_bytes());
        }
        c
    }
    #[test]
    fn flac_streaminfo_and_comments() {
        let mut f = b"fLaC".to_vec();
        f.extend([0, 0, 0, 34]);
        let mut info = alloc::vec![0; 34];
        // 48 kHz, 2 channels, 24 bit, 480000 samples (10 s).
        let v: u64 = (48_000 << 44) | (1 << 41) | (23 << 36) | 480_000;
        info[10..18].copy_from_slice(&v.to_be_bytes());
        f.extend(info);
        let c = comments(&["TITLE=Take Five", "artist=Quartet", "DATE=1959"]);
        f.push(0x84);
        f.extend(be(c.len() as u64, 3));
        f.extend(c);
        f.extend([0; 1000]);
        let r = run("a.flac", &f);
        assert_eq!(
            (
                r.get("Sample rate"),
                r.get("Channels"),
                r.get("Sample size")
            ),
            (Some("48 kHz"), Some("2 (stereo)"), Some("24 bit"))
        );
        assert_eq!(
            (
                r.get("Length"),
                r.get("Title"),
                r.get("Contributing artists"),
                r.get("Year")
            ),
            (
                Some("00:00:10"),
                Some("Take Five"),
                Some("Quartet"),
                Some("1959")
            )
        );
        assert_eq!(r.get("Encoding software"), Some("Tester"));
        fuzz("a.flac", &f);
    }
    fn page(serial: u32, granule: u64, packets: &[&[u8]]) -> Vec<u8> {
        let mut segments = Vec::new();
        let mut body = Vec::new();
        for p in packets {
            let mut left = p.len();
            loop {
                let n = left.min(255);
                segments.push(n as u8);
                left -= n;
                if n < 255 {
                    break;
                }
            }
            body.extend_from_slice(p);
        }
        let mut out = b"OggS\0\0".to_vec();
        out.extend(le(granule, 8));
        out.extend(le(u64::from(serial), 4));
        out.extend([0; 8]);
        out.push(segments.len() as u8);
        out.extend(segments);
        out.extend(body);
        out
    }
    #[test]
    fn ogg_opus_and_vorbis_heads_tags_and_length() {
        let mut head = b"OpusHead\x01\x02".to_vec();
        head.extend(le(312, 2));
        head.extend(le(44_100, 4));
        head.extend([0, 0, 0]);
        let tags = [b"OpusTags".to_vec(), comments(&["TITLE=Voice memo"])].concat();
        let mut o = page(7, 0, &[&head]);
        o.extend(page(7, 0, &[&tags]));
        o.extend(page(7, 48_000 * 90 + 312, &[&[1; 300]]));
        let r = run("memo.opus", &o);
        assert_eq!(
            (r.format, r.get("Channels"), r.get("Title"), r.get("Length")),
            (
                Some("Ogg Opus audio"),
                Some("2 (stereo)"),
                Some("Voice memo"),
                Some("00:01:30")
            )
        );
        fuzz("memo.opus", &o);
        let mut vorbis = b"\x01vorbis".to_vec();
        vorbis.extend(le(0, 4));
        vorbis.push(1);
        vorbis.extend(le(22_050, 4));
        vorbis.extend(le(0, 4));
        vorbis.extend(le(96_000, 4));
        vorbis.extend(le(0, 4));
        vorbis.extend([0xB8, 1]);
        let long = alloc::vec![b'x'; 600];
        let c = [
            b"\x03vorbis".to_vec(),
            comments(&[
                "ALBUM=Field recordings",
                core::str::from_utf8(&[b"COMMENT=".to_vec(), long].concat()).unwrap(),
            ]),
        ]
        .concat();
        let mut v = page(1, 0, &[&vorbis]);
        v.extend(page(1, 0, &[&c]));
        v.extend(page(1, 22_050 * 5, &[&[0; 10]]));
        let r = run("a.ogg", &v);
        assert_eq!(
            (
                r.get("Channels"),
                r.get("Bit rate"),
                r.get("Album"),
                r.get("Length")
            ),
            (
                Some("1 (mono)"),
                Some("96 kbps"),
                Some("Field recordings"),
                Some("00:00:05")
            )
        );
        fuzz("a.ogg", &v);
    }
}
