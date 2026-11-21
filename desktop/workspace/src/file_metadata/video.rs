//! Video: MP4/MOV/M4A (ISO base media), AVI and Matroska/WebM.
use super::Section::{Audio, Description, Media, Origin, Video};
use super::audio::{frame_rate, riff_info};
use super::*;

fn codec_name(code: &[u8]) -> String {
    let name = match code {
        b"avc1" | b"avc3" | b"H264" | b"h264" | b"X264" | b"x264" | b"AVC1" => "H.264 (AVC)",
        b"hvc1" | b"hev1" | b"HEVC" => "H.265 (HEVC)",
        b"av01" => "AV1",
        b"vp08" => "VP8",
        b"vp09" => "VP9",
        b"mp4v" | b"XVID" | b"xvid" | b"DIVX" | b"divx" | b"DX50" | b"FMP4" => "MPEG-4 Part 2",
        b"MJPG" | b"mjpa" | b"jpeg" => "Motion JPEG",
        b"apch" | b"apcn" | b"apcs" | b"apco" | b"ap4h" => "Apple ProRes",
        b"mp4a" => "AAC",
        b"ac-3" => "Dolby Digital (AC-3)",
        b"ec-3" => "Dolby Digital Plus (E-AC-3)",
        b"alac" => "Apple Lossless",
        b"Opus" => "Opus",
        b"fLaC" => "FLAC",
        b".mp3" => "MP3",
        b"lpcm" | b"sowt" | b"twos" => "PCM",
        _ => "",
    };
    if name.is_empty() {
        fourcc(code).unwrap_or_default()
    } else {
        String::from(name)
    }
}

pub(super) fn is_mp4(head: &[u8]) -> bool {
    matches!(
        head.get(4..8),
        Some(b"ftyp" | b"moov" | b"mdat" | b"wide" | b"pnot")
    )
}

/// One box: (type, body start, end) within `limit`.
fn mp4_box(d: &Sparse, at: u64, limit: u64) -> Result<([u8; 4], u64, u64), Miss> {
    let h = d.get(at, 8)?;
    let kind = take(h, 4).unwrap_or([0; 4]);
    let (header, len) = match be32(h, 0).unwrap_or(0) {
        1 => (16, d.be64(at + 8)?),
        0 => (8, limit - at),
        n => (8, u64::from(n)),
    };
    if len < header {
        return Err(Miss::Truncated);
    }
    Ok((kind, at + header, at.saturating_add(len).min(limit)))
}
/// Children of a container box, at most 256.
fn mp4_children(d: &Sparse, start: u64, end: u64) -> Result<Vec<([u8; 4], u64, u64)>, Miss> {
    let mut out = Vec::new();
    let mut at = start;
    while at + 8 <= end && out.len() < 256 {
        let child = mp4_box(d, at, end)?;
        at = child.2;
        out.push(child);
    }
    Ok(out)
}

#[derive(Default)]
struct Track {
    handler: [u8; 4],
    codec: [u8; 4],
    width: u64,
    height: u64,
    channels: u64,
    rate: u64,
    timescale: u64,
    duration: u64,
    samples: u64,
}

fn mp4_trak(d: &Sparse, start: u64, end: u64) -> Result<Track, Miss> {
    let mut t = Track::default();
    for (kind, body, end) in mp4_children(d, start, end)? {
        match &kind {
            b"tkhd" => {
                let v1 = d.byte(body)? == 1;
                let at = body + if v1 { 88 } else { 76 };
                if at + 8 <= end {
                    t.width = u64::from(d.be32(at)? >> 16);
                    t.height = u64::from(d.be32(at + 4)? >> 16);
                }
            }
            b"mdia" => {
                for (kind, body, end) in mp4_children(d, body, end)? {
                    match &kind {
                        b"mdhd" => {
                            let v1 = d.byte(body)? == 1;
                            t.timescale = u64::from(d.be32(body + if v1 { 20 } else { 12 })?);
                            t.duration = if v1 {
                                d.be64(body + 24)?
                            } else {
                                u64::from(d.be32(body + 16)?)
                            };
                        }
                        b"hdlr" => t.handler = take(d.get(body + 8, 4)?, 0).unwrap_or([0; 4]),
                        b"minf" => {
                            for (kind, body, end) in mp4_children(d, body, end)? {
                                if &kind == b"stbl" {
                                    mp4_stbl(d, body, end, &mut t)?;
                                }
                            }
                        }
                        _ => (),
                    }
                }
            }
            _ => (),
        }
    }
    Ok(t)
}
fn mp4_stbl(d: &Sparse, start: u64, end: u64, t: &mut Track) -> Result<(), Miss> {
    for (kind, body, end) in mp4_children(d, start, end)? {
        match &kind {
            b"stsd" if body + 16 <= end => {
                let entry = body + 8;
                t.codec = take(d.get(entry + 4, 4)?, 0).unwrap_or([0; 4]);
                if &t.handler == b"vide" && entry + 36 <= end {
                    t.width = t.width.max(u64::from(d.be16(entry + 32)?));
                    t.height = t.height.max(u64::from(d.be16(entry + 34)?));
                } else if &t.handler == b"soun" && entry + 36 <= end {
                    t.channels = u64::from(d.be16(entry + 24)?);
                    t.rate = u64::from(d.be32(entry + 32)? >> 16);
                }
            }
            b"stts" if body + 8 <= end => {
                let count = u64::from(d.be32(body + 4)?)
                    .min(4096)
                    .min((end - body - 8) / 8);
                let table = d.get(body + 8, count * 8)?;
                t.samples = table
                    .chunks_exact(8)
                    .map(|e| u64::from(be32(e, 0).unwrap_or(0)))
                    .sum();
            }
            _ => (),
        }
    }
    Ok(())
}
/// iTunes-style metadata (udta/meta/ilst).
fn mp4_ilst(d: &Sparse, start: u64, end: u64, r: &mut Report) -> Result<(), Miss> {
    for (kind, body, end) in mp4_children(d, start, end)? {
        let Some((_, data, data_end)) = mp4_children(d, body, end)?
            .into_iter()
            .find(|c| &c.0 == b"data")
        else {
            continue;
        };
        if data + 8 > data_end {
            continue;
        }
        let value = d.get(data + 8, (data_end - data - 8).min(512))?;
        let text = || utf8(value);
        match &kind {
            b"\xA9nam" => r.add(Description, "Title", text()),
            b"\xA9ART" => r.add(Media, "Contributing artists", text()),
            b"aART" => r.add(Media, "Album artist", text()),
            b"\xA9alb" => r.add(Media, "Album", text()),
            b"\xA9day" => r.add(Media, "Year", text()),
            b"\xA9gen" => r.add(Media, "Genre", text()),
            b"\xA9wrt" => r.add(Media, "Composers", text()),
            b"\xA9cmt" => r.add(Description, "Comments", text()),
            b"\xA9too" => r.add(Origin, "Encoded with", text()),
            b"cprt" => r.add(Origin, "Copyright", text()),
            b"covr" => r.add(Media, "Cover art", "Embedded"),
            b"trkn" if value.len() >= 6 => {
                let (n, of) = (be16(value, 2).unwrap_or(0), be16(value, 4).unwrap_or(0));
                if n > 0 {
                    r.add(
                        Media,
                        "#",
                        if of > 0 {
                            format!("{n}/{of}")
                        } else {
                            format!("{n}")
                        },
                    );
                }
            }
            _ => (),
        }
    }
    Ok(())
}
fn mp4_meta(d: &Sparse, body: u64, end: u64, r: &mut Report) -> Result<(), Miss> {
    // ISO 'meta' is a full box (4 bytes version/flags); QuickTime's is not.
    let start = if body + 12 <= end && d.be32(body)? == 0 {
        body + 4
    } else {
        body
    };
    for (kind, body, end) in mp4_children(d, start, end)? {
        if &kind == b"ilst" {
            mp4_ilst(d, body, end, r)?;
        }
    }
    Ok(())
}

pub(super) fn mp4(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    let size = d.size();
    let mut brand = [0u8; 4];
    let mut moov = None;
    let mut at = 0;
    for _ in 0..256 {
        if at + 8 > size {
            break;
        }
        let (kind, body, end) = mp4_box(d, at, size)?;
        match &kind {
            b"ftyp" if body + 4 <= end => brand = take(d.get(body, 4)?, 0).unwrap_or([0; 4]),
            b"moov" => moov = Some((body, end)),
            _ => (),
        }
        at = end;
        if moov.is_some() {
            break;
        }
    }
    let format = match &brand {
        b"qt  " => "QuickTime movie",
        b"M4A " => "MPEG-4 audio",
        b"M4B " => "MPEG-4 audiobook",
        b"heic" | b"heix" | b"mif1" | b"msf1" => "HEIF image",
        b"avif" | b"avis" => "AVIF image",
        b if b.starts_with(b"3g") => "3GPP video",
        _ => "",
    };
    let Some((start, end)) = moov else {
        r.format(if format.is_empty() {
            "MPEG-4 media"
        } else {
            format
        });
        return Ok(());
    };
    let mut tracks = Vec::new();
    let (mut timescale, mut duration, mut created) = (0, 0, 0);
    for (kind, body, end) in mp4_children(d, start, end)? {
        match &kind {
            b"mvhd" => {
                let v1 = d.byte(body)? == 1;
                if v1 {
                    created = d.be64(body + 4)?;
                    timescale = u64::from(d.be32(body + 20)?);
                    duration = d.be64(body + 24)?;
                } else {
                    created = u64::from(d.be32(body + 4)?);
                    timescale = u64::from(d.be32(body + 12)?);
                    duration = u64::from(d.be32(body + 16)?);
                }
            }
            b"trak" if tracks.len() < 16 => tracks.push(mp4_trak(d, body, end)?),
            b"udta" => {
                for (kind, body, end) in mp4_children(d, body, end)? {
                    if &kind == b"meta" {
                        mp4_meta(d, body, end, r)?;
                    }
                }
            }
            b"meta" => mp4_meta(d, body, end, r)?,
            _ => (),
        }
    }
    let video = tracks.iter().find(|t| &t.handler == b"vide");
    let sound = tracks.iter().find(|t| &t.handler == b"soun");
    r.format(match (format, video.is_some()) {
        ("", true) => "MPEG-4 video",
        ("", false) => "MPEG-4 audio",
        (f, _) => f,
    });
    let length = ms(duration, timescale).filter(|ms| *ms > 0);
    if let Some(ms) = length {
        r.add(Media, "Length", length_ms(ms));
    }
    // Seconds since 1904-01-01.
    if let Some(when) = created
        .checked_sub(2_082_844_800)
        .and_then(|s| plausible_date(s as i64))
    {
        r.add(Origin, "Media created", when);
    }
    if let Some(v) = video {
        if v.width > 0 && v.height > 0 {
            r.add(Video, "Frame width", grouped(v.width));
            r.add(Video, "Frame height", grouped(v.height));
        }
        if v.samples > 0 && v.duration > 0 && v.timescale > 0 {
            r.add(
                Video,
                "Frame rate",
                frame_rate(
                    u64::try_from(
                        u128::from(v.samples) * u128::from(v.timescale) * 100
                            / u128::from(v.duration),
                    )
                    .unwrap_or(0),
                ),
            );
        }
        r.add(Video, "Video codec", codec_name(&v.codec));
        if let Some(ms) = length {
            r.add(Video, "Total bitrate", kbps(d.size() * 8 * 1000 / ms));
        }
    } else if let Some(ms) = length {
        r.add(Audio, "Bit rate", kbps(d.size() * 8 * 1000 / ms));
    }
    if let Some(a) = sound {
        r.add(Audio, "Audio codec", codec_name(&a.codec));
        if a.channels > 0 {
            r.add(Audio, "Channels", channels(a.channels));
        }
        if a.rate > 0 {
            r.add(Audio, "Sample rate", hz(a.rate));
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------------
// AVI.

fn wave_codec(tag: u16) -> &'static str {
    match tag {
        1 => "PCM",
        0x55 => "MP3",
        0xFF | 0x1610 => "AAC",
        0x2000 => "Dolby Digital (AC-3)",
        0x2001 => "DTS",
        0x161 => "Windows Media Audio",
        _ => "",
    }
}
pub(super) fn avi(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    r.format("AVI video");
    let end = d.size();
    let mut at = 12u64;
    for _ in 0..64 {
        if at + 12 > end {
            break;
        }
        let h = d.get(at, 12)?;
        let len = u64::from(le32(h, 4).unwrap_or(0));
        if &h[..4] == b"LIST" {
            match &h[8..12] {
                b"hdrl" => avi_hdrl(d, at + 12, (at + 8 + len).min(end), r)?,
                b"INFO" if len <= 65_536 => {
                    riff_info(d.get(at + 12, len.saturating_sub(4).min(end - at - 12))?, r)
                }
                b"movi" => (),
                _ => (),
            }
        }
        at = at + 8 + len + (len & 1);
    }
    Ok(())
}
fn avi_hdrl(d: &Sparse, start: u64, end: u64, r: &mut Report) -> Result<(), Miss> {
    let mut at = start;
    for _ in 0..32 {
        if at + 8 > end {
            break;
        }
        let h = d.get(at, 8)?;
        let len = u64::from(le32(h, 4).unwrap_or(0));
        let body = at + 8;
        match &h[..4] {
            b"avih" if len >= 40 => {
                let b = d.get(body, 40)?;
                let per_frame = u64::from(le32(b, 0).unwrap_or(0));
                let frames = u64::from(le32(b, 16).unwrap_or(0));
                if per_frame > 0 {
                    r.add(Media, "Length", length_ms(frames * per_frame / 1000));
                    r.add(Video, "Frame rate", frame_rate(100_000_000 / per_frame));
                }
                r.add(
                    Video,
                    "Frame width",
                    grouped(u64::from(le32(b, 32).unwrap_or(0))),
                );
                r.add(
                    Video,
                    "Frame height",
                    grouped(u64::from(le32(b, 36).unwrap_or(0))),
                );
            }
            b"LIST" if len >= 4 => {
                let list = d.get(body, 4)?;
                if list == b"strl" {
                    avi_strl(d, body + 4, (body + len).min(end), r)?;
                }
            }
            _ => (),
        }
        at = body + len + (len & 1);
    }
    Ok(())
}
fn avi_strl(d: &Sparse, start: u64, end: u64, r: &mut Report) -> Result<(), Miss> {
    let mut at = start;
    let mut kind = [0u8; 4];
    let mut handler = [0u8; 4];
    for _ in 0..8 {
        if at + 8 > end {
            break;
        }
        let h = d.get(at, 8)?;
        let len = u64::from(le32(h, 4).unwrap_or(0));
        let body = at + 8;
        match &h[..4] {
            b"strh" if len >= 8 => {
                let b = d.get(body, 8)?;
                kind = take(b, 0).unwrap_or([0; 4]);
                handler = take(b, 4).unwrap_or([0; 4]);
            }
            b"strf" if &kind == b"vids" && len >= 20 => {
                let b = d.get(body, 20)?;
                let compression = &b[16..20];
                let codec = if compression.iter().all(|c| *c == 0) {
                    codec_name(&handler)
                } else {
                    codec_name(compression)
                };
                r.add(Video, "Video codec", codec);
            }
            b"strf" if &kind == b"auds" && len >= 16 => {
                let b = d.get(body, 16)?;
                r.add(Audio, "Audio codec", wave_codec(le16(b, 0).unwrap_or(0)));
                r.add(
                    Audio,
                    "Channels",
                    channels(u64::from(le16(b, 2).unwrap_or(0))),
                );
                r.add(Audio, "Sample rate", hz(u64::from(le32(b, 4).unwrap_or(0))));
                r.add(
                    Audio,
                    "Bit rate",
                    kbps(u64::from(le32(b, 8).unwrap_or(0)) * 8),
                );
            }
            _ => (),
        }
        at = body + len + (len & 1);
    }
    Ok(())
}

// ----------------------------------------------------------------------------
// Matroska / WebM (EBML).

const UNKNOWN: u64 = u64::MAX;
/// An element header at `at`: (id, data start, data size or UNKNOWN).
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
    let all_ones = size == (1u64 << (7 * size_len)) - 1;
    Ok((
        id,
        at + id_len + size_len,
        if all_ones { UNKNOWN } else { size },
    ))
}
/// Children of an element already read into memory: (id, data).
fn ebml_children(b: &[u8]) -> Vec<(u64, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < b.len() && out.len() < 512 {
        let first = b[at];
        let id_len = first.leading_zeros() as usize + 1;
        let Some(id_bytes) = b.get(at..at + id_len).filter(|_| id_len <= 4) else {
            break;
        };
        let id = id_bytes.iter().fold(0, |v, c| v << 8 | u64::from(*c));
        let Some(&s) = b.get(at + id_len) else { break };
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
fn ebml_uint(b: &[u8]) -> u64 {
    b.iter().take(8).fold(0, |v, c| v << 8 | u64::from(*c))
}
fn ebml_float(b: &[u8]) -> Option<f64> {
    let v = match b.len() {
        4 => f64::from(f32::from_bits(be32(b, 0)?)),
        8 => f64::from_bits(be64(b, 0)?),
        _ => return None,
    };
    (v.is_finite() && v >= 0.0).then_some(v)
}
fn mkv_codec(id: &str) -> String {
    let name = match id {
        "V_MPEG4/ISO/AVC" => "H.264 (AVC)",
        "V_MPEGH/ISO/HEVC" => "H.265 (HEVC)",
        "V_VP8" => "VP8",
        "V_VP9" => "VP9",
        "V_AV1" => "AV1",
        "V_MPEG4/ISO/ASP" => "MPEG-4 Part 2",
        "V_THEORA" => "Theora",
        "A_OPUS" => "Opus",
        "A_VORBIS" => "Vorbis",
        "A_FLAC" => "FLAC",
        "A_AC3" => "Dolby Digital (AC-3)",
        "A_EAC3" => "Dolby Digital Plus (E-AC-3)",
        "A_MPEG/L3" => "MP3",
        "A_DTS" => "DTS",
        _ if id.starts_with("A_AAC") => "AAC",
        _ if id.starts_with("A_PCM") => "PCM",
        _ => "",
    };
    String::from(if name.is_empty() { id } else { name })
}

pub(super) fn mkv(d: &Sparse, r: &mut Report) -> Result<(), Miss> {
    let (id, start, size) = ebml(d, 0)?;
    if id != 0x1A45_DFA3 || size == UNKNOWN {
        return Ok(());
    }
    let header = d.get(start, size.min(256).min(d.size().saturating_sub(start)))?;
    let webm = ebml_children(header)
        .iter()
        .any(|(id, v)| *id == 0x4282 && *v == b"webm");
    let mut at = start + size;
    let (segment, seg_start, seg_size) = ebml(d, at)?;
    if segment != 0x1853_8067 {
        r.format(if webm { "WebM video" } else { "Matroska video" });
        return Ok(());
    }
    let seg_end = if seg_size == UNKNOWN {
        d.size()
    } else {
        (seg_start + seg_size).min(d.size())
    };
    at = seg_start;
    let (mut info, mut tracks) = (false, false);
    let mut has_video = false;
    for _ in 0..256 {
        if at >= seg_end || (info && tracks) {
            break;
        }
        let (id, body, len) = ebml(d, at)?;
        if len == UNKNOWN {
            break;
        }
        match id {
            0x1549_A966 => {
                info = true;
                let b = d.get(body, len.min(65_536).min(seg_end.saturating_sub(body)))?;
                let fields = ebml_children(b);
                let scale = fields
                    .iter()
                    .find(|(id, _)| *id == 0x2A_D7B1)
                    .map_or(1_000_000, |(_, v)| ebml_uint(v));
                for (id, v) in &fields {
                    match id {
                        0x4489 => {
                            if let Some(dur) = ebml_float(v) {
                                let ms = dur * scale as f64 / 1_000_000.0;
                                if (1.0..1.0e15).contains(&ms) {
                                    r.add(Media, "Length", length_ms(ms as u64));
                                }
                            }
                        }
                        0x7BA9 => r.add(Description, "Title", utf8(v)),
                        0x4D80 => r.add(Origin, "Encoded with", utf8(v)),
                        0x5741 => r.add(Origin, "Program name", utf8(v)),
                        0x4461 if v.len() == 8 => {
                            let ns = be64(v, 0).unwrap_or(0) as i64;
                            if let Some(when) = plausible_date(978_307_200 + ns / 1_000_000_000) {
                                r.add(Origin, "Media created", when);
                            }
                        }
                        _ => (),
                    }
                }
            }
            0x1654_AE6B => {
                tracks = true;
                let b = d.get(body, len.min(262_144).min(seg_end.saturating_sub(body)))?;
                for (id, entry) in ebml_children(b) {
                    if id != 0xAE {
                        continue;
                    }
                    let fields = ebml_children(entry);
                    let kind = fields
                        .iter()
                        .find(|(id, _)| *id == 0x83)
                        .map_or(0, |(_, v)| ebml_uint(v));
                    let codec = fields
                        .iter()
                        .find(|(id, _)| *id == 0x86)
                        .map(|(_, v)| mkv_codec(&utf8(v)))
                        .unwrap_or_default();
                    if kind == 1 && !has_video {
                        has_video = true;
                        r.add(Video, "Video codec", codec);
                        if let Some((_, v)) = fields.iter().find(|(id, _)| *id == 0xE0) {
                            for (id, x) in ebml_children(v) {
                                match id {
                                    0xB0 => r.add(Video, "Frame width", grouped(ebml_uint(x))),
                                    0xBA => r.add(Video, "Frame height", grouped(ebml_uint(x))),
                                    _ => (),
                                }
                            }
                        }
                        if let Some((_, v)) = fields.iter().find(|(id, _)| *id == 0x23_E383) {
                            let ns = ebml_uint(v);
                            if ns > 0 {
                                r.add(Video, "Frame rate", frame_rate(100_000_000_000 / ns));
                            }
                        }
                    } else if kind == 2 {
                        r.add(Audio, "Audio codec", codec);
                        if let Some((_, v)) = fields.iter().find(|(id, _)| *id == 0xE1) {
                            for (id, x) in ebml_children(v) {
                                match id {
                                    0xB5 => {
                                        if let Some(rate) = ebml_float(x).filter(|f| *f < 1.0e7) {
                                            r.add(Audio, "Sample rate", hz(rate as u64));
                                        }
                                    }
                                    0x9F => r.add(Audio, "Channels", channels(ebml_uint(x))),
                                    _ => (),
                                }
                            }
                        }
                    }
                }
            }
            _ => (),
        }
        at = body.saturating_add(len);
    }
    r.format(match (webm, has_video) {
        (true, true) => "WebM video",
        (true, false) => "WebM audio",
        (false, true) => "Matroska video",
        (false, false) => "Matroska audio",
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    fn bx(kind: &[u8], body: &[u8]) -> Vec<u8> {
        [be(body.len() as u64 + 8, 4), kind.to_vec(), body.to_vec()].concat()
    }
    fn full(kind: &[u8], version: u8, body: &[u8]) -> Vec<u8> {
        bx(
            kind,
            &[alloc::vec![version, 0, 0, 0], body.to_vec()].concat(),
        )
    }
    fn mp4_file() -> Vec<u8> {
        let mvhd = full(
            b"mvhd",
            0,
            &[
                be(3_786_912_000, 4),
                be(0, 4),
                be(1000, 4),
                be(125_000, 4),
                alloc::vec![0; 80],
            ]
            .concat(),
        );
        let mut tkhd = alloc::vec![0; 80];
        tkhd[72..76].copy_from_slice(&(1920u32 << 16).to_be_bytes());
        tkhd[76..80].copy_from_slice(&(1080u32 << 16).to_be_bytes());
        let mut entry = be(86, 4);
        entry.extend(b"avc1");
        entry.resize(86, 0);
        let stsd = full(b"stsd", 0, &[be(1, 4), entry].concat());
        let stts = full(b"stts", 0, &[be(1, 4), be(3000, 4), be(1001, 4)].concat());
        let stbl = bx(b"stbl", &[stsd, stts].concat());
        let minf = bx(b"minf", &stbl);
        let mdhd = full(
            b"mdhd",
            0,
            &[
                be(0, 4),
                be(0, 4),
                be(24_000, 4),
                be(3_003_000, 4),
                alloc::vec![0; 4],
            ]
            .concat(),
        );
        let hdlr = full(
            b"hdlr",
            0,
            &[be(0, 4), b"vide".to_vec(), alloc::vec![0; 13]].concat(),
        );
        let mdia = bx(b"mdia", &[mdhd, hdlr, minf].concat());
        let trak = bx(b"trak", &[full(b"tkhd", 0, &tkhd), mdia].concat());
        let data = bx(b"data", &[be(1, 4), be(0, 4), b"Holiday".to_vec()].concat());
        let ilst = bx(b"ilst", &bx(b"\xA9nam", &data));
        let meta = full(
            b"meta",
            0,
            &[
                full(
                    b"hdlr",
                    0,
                    &[be(0, 4), b"mdir".to_vec(), alloc::vec![0; 13]].concat(),
                ),
                ilst,
            ]
            .concat(),
        );
        let udta = bx(b"udta", &meta);
        let moov = bx(b"moov", &[mvhd, trak, udta].concat());
        [
            bx(b"ftyp", b"isom\0\0\x02\0isomavc1"),
            bx(b"mdat", &[0; 5000]),
            moov,
        ]
        .concat()
    }
    #[test]
    fn mp4_length_resolution_codec_rate_and_tags() {
        let r = run("clip.mp4", &mp4_file());
        assert_eq!(r.format, Some("MPEG-4 video"));
        assert_eq!(r.get("Length"), Some("00:02:05"));
        assert_eq!(
            (r.get("Frame width"), r.get("Frame height")),
            (Some("1,920"), Some("1,080"))
        );
        assert_eq!(r.get("Video codec"), Some("H.264 (AVC)"));
        assert_eq!(r.get("Frame rate"), Some("23.97 frames/second"));
        assert_eq!(r.get("Title"), Some("Holiday"));
        assert_eq!(r.get("Media created"), Some("2024-01-01 00:00:00 UTC"));
        fuzz("clip.mp4", &mp4_file());
    }
    fn chunk(id: &[u8], body: &[u8]) -> Vec<u8> {
        [id.to_vec(), le(body.len() as u64, 4), body.to_vec()].concat()
    }
    #[test]
    fn avi_main_and_stream_headers() {
        let mut avih = alloc::vec![0; 56];
        avih[0..4].copy_from_slice(&40_000u32.to_le_bytes());
        avih[16..20].copy_from_slice(&750u32.to_le_bytes());
        avih[32..36].copy_from_slice(&640u32.to_le_bytes());
        avih[36..40].copy_from_slice(&360u32.to_le_bytes());
        let mut strh = alloc::vec![0; 56];
        strh[..8].copy_from_slice(b"vidsXVID");
        let mut strf = alloc::vec![0; 40];
        strf[16..20].copy_from_slice(b"XVID");
        let strl = [
            b"strl".to_vec(),
            chunk(b"strh", &strh),
            chunk(b"strf", &strf),
        ]
        .concat();
        let mut ah = alloc::vec![0; 56];
        ah[..4].copy_from_slice(b"auds");
        let mut af = alloc::vec![0; 16];
        af[..2].copy_from_slice(&0x55u16.to_le_bytes());
        af[2..4].copy_from_slice(&2u16.to_le_bytes());
        af[4..8].copy_from_slice(&48_000u32.to_le_bytes());
        af[8..12].copy_from_slice(&16_000u32.to_le_bytes());
        let strl2 = [b"strl".to_vec(), chunk(b"strh", &ah), chunk(b"strf", &af)].concat();
        let hdrl = [
            b"hdrl".to_vec(),
            chunk(b"avih", &avih),
            chunk(b"LIST", &strl),
            chunk(b"LIST", &strl2),
        ]
        .concat();
        let body = [
            b"AVI ".to_vec(),
            chunk(b"LIST", &hdrl),
            chunk(b"LIST", &[b"movi".to_vec(), alloc::vec![0; 100]].concat()),
        ]
        .concat();
        let avi = chunk(b"RIFF", &body);
        let r = run("a.avi", &avi);
        assert_eq!(
            (r.get("Length"), r.get("Frame rate"), r.get("Frame width")),
            (Some("00:00:30"), Some("25 frames/second"), Some("640"))
        );
        assert_eq!(
            (
                r.get("Video codec"),
                r.get("Audio codec"),
                r.get("Bit rate")
            ),
            (Some("MPEG-4 Part 2"), Some("MP3"), Some("128 kbps"))
        );
        fuzz("a.avi", &avi);
    }
    fn el(id: u64, body: &[u8]) -> Vec<u8> {
        let id_bytes: Vec<u8> = id
            .to_be_bytes()
            .into_iter()
            .skip_while(|b| *b == 0)
            .collect();
        [
            id_bytes,
            alloc::vec![0x01],
            be(body.len() as u64, 7),
            body.to_vec(),
        ]
        .concat()
    }
    #[test]
    fn matroska_info_and_tracks() {
        let header = el(0x1A45_DFA3, &el(0x4282, b"webm"));
        let info = el(
            0x1549_A966,
            &[
                el(0x2A_D7B1, &be(1_000_000, 3)),
                el(0x4489, &90_500.0f64.to_be_bytes()),
                el(0x5741, b"Recorder 2"),
            ]
            .concat(),
        );
        let video = el(
            0xAE,
            &[
                el(0x83, &[1]),
                el(0x86, b"V_VP9"),
                el(
                    0xE0,
                    &[el(0xB0, &be(1280, 2)), el(0xBA, &be(720, 2))].concat(),
                ),
                el(0x23_E383, &be(33_333_333, 4)),
            ]
            .concat(),
        );
        let audio = el(
            0xAE,
            &[
                el(0x83, &[2]),
                el(0x86, b"A_OPUS"),
                el(
                    0xE1,
                    &[el(0xB5, &48_000.0f64.to_be_bytes()), el(0x9F, &[2])].concat(),
                ),
            ]
            .concat(),
        );
        let tracks = el(0x1654_AE6B, &[video, audio].concat());
        let cluster = el(0x1F43_B675, &[0; 2000]);
        let mkv = [header, el(0x1853_8067, &[info, tracks, cluster].concat())].concat();
        let r = run("a.webm", &mkv);
        assert_eq!(r.format, Some("WebM video"));
        assert_eq!(
            (r.get("Length"), r.get("Frame width"), r.get("Video codec")),
            (Some("00:01:31"), Some("1,280"), Some("VP9"))
        );
        assert_eq!(
            (
                r.get("Audio codec"),
                r.get("Sample rate"),
                r.get("Channels")
            ),
            (Some("Opus"), Some("48 kHz"), Some("2 (stereo)"))
        );
        assert_eq!(r.get("Frame rate"), Some("30 frames/second"));
        assert_eq!(r.get("Program name"), Some("Recorder 2"));
        fuzz("a.webm", &mkv);
    }
}
