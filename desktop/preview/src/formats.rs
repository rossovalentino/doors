//! Bounded metadata and content providers; these never execute files or claim playback.
extern crate alloc;
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use door_workspace::explorer_preview::{Image, Kind};
pub struct Content {
    pub image: Option<Image>,
    pub text: String,
    pub details: Vec<(String, String)>,
}
fn u16at(b: &[u8], p: usize) -> Result<u16, &'static str> {
    Ok(u16::from_le_bytes(
        b.get(p..p + 2)
            .ok_or("Truncated file.")?
            .try_into()
            .unwrap(),
    ))
}
fn u32at(b: &[u8], p: usize) -> Result<u32, &'static str> {
    Ok(u32::from_le_bytes(
        b.get(p..p + 4)
            .ok_or("Truncated file.")?
            .try_into()
            .unwrap(),
    ))
}
pub fn inspect(kind: Kind, bytes: &[u8]) -> Result<Content, &'static str> {
    match kind {
        Kind::Archive => zip(bytes),
        Kind::Audio => wave(bytes),
        Kind::Model => obj(bytes),
        _ => Err(kind.unavailable()),
    }
}
fn zip(b: &[u8]) -> Result<Content, &'static str> {
    if b.len() < 22 {
        return Err("Invalid ZIP archive.");
    }
    let end = (b.len().saturating_sub(65557)..=b.len() - 22)
        .rev()
        .find(|&p| {
            b.get(p..p + 4) == Some(b"PK\x05\x06")
                && u16at(b, p + 20).is_ok_and(|n| p + 22 + n as usize == b.len())
        })
        .ok_or("ZIP directory not found.")?;
    let count = u16at(b, end + 10)? as usize;
    let size = u32at(b, end + 12)? as usize;
    let offset = u32at(b, end + 16)? as usize;
    if u16at(b, end + 4)? != 0
        || u16at(b, end + 6)? != 0
        || u16at(b, end + 8)? as usize != count
        || count > 256
        || offset.checked_add(size) != Some(end)
    {
        return Err("ZIP64, split or oversized archive is unsupported.");
    }
    let mut at = offset;
    let mut text = String::from("Name                              Size          Packed\n\n");
    let mut expanded = 0u64;
    for _ in 0..count {
        if b.get(at..at + 4) != Some(b"PK\x01\x02") {
            return Err("Invalid ZIP directory entry.");
        }
        let packed = u32at(b, at + 20)?;
        let bytes = u32at(b, at + 24)?;
        let n = u16at(b, at + 28)? as usize;
        let extra = u16at(b, at + 30)? as usize;
        let comment = u16at(b, at + 32)? as usize;
        if n == 0 || n > 192 || u16at(b, at + 34)? != 0 || packed == u32::MAX || bytes == u32::MAX {
            return Err("Unsupported ZIP entry.");
        }
        let name = core::str::from_utf8(b.get(at + 46..at + 46 + n).ok_or("Truncated ZIP name.")?)
            .map_err(|_| "This archive uses an unsupported filename encoding.")?;
        if name.chars().any(char::is_control) {
            return Err("Invalid archive filename.");
        }
        expanded = expanded
            .checked_add(bytes as u64)
            .ok_or("Invalid archive size.")?;
        text += &format!(
            "{name}    {}    {}{}\n",
            door_workspace::explorer_preview::size(bytes as u64),
            door_workspace::explorer_preview::size(packed as u64),
            if u16at(b, at + 8)? & 1 != 0 {
                "  [Encrypted]"
            } else {
                ""
            }
        );
        at = at
            .checked_add(46 + n + extra + comment)
            .filter(|&n| n <= end)
            .ok_or("Truncated ZIP directory.")?;
    }
    if at != end {
        return Err("ZIP directory length mismatch.");
    }
    Ok(Content {
        image: None,
        text,
        details: alloc::vec![
            ("Entries".into(), count.to_string()),
            (
                "Expanded size".into(),
                door_workspace::explorer_preview::size(expanded)
            ),
            (
                "Mode".into(),
                "Directory listing; files are not extracted".into()
            )
        ],
    })
}
fn wave(b: &[u8]) -> Result<Content, &'static str> {
    if b.get(..4) != Some(b"RIFF")
        || b.get(8..12) != Some(b"WAVE")
        || u32at(b, 4)? as usize + 8 != b.len()
    {
        return Err("This provider reads RIFF PCM WAV files; playback is not available.");
    }
    let mut at = 12;
    let mut format = None;
    let mut data = None;
    while at + 8 <= b.len() {
        let n = u32at(b, at + 4)? as usize;
        let payload = b.get(at + 8..at + 8 + n).ok_or("Truncated WAV chunk.")?;
        if &b[at..at + 4] == b"fmt " {
            if format.is_some() || n < 16 {
                return Err("Invalid WAV format.");
            }
            format = Some((
                u16at(payload, 0)?,
                u16at(payload, 2)?,
                u32at(payload, 4)?,
                u32at(payload, 8)?,
                u16at(payload, 12)?,
                u16at(payload, 14)?,
            ));
        }
        if &b[at..at + 4] == b"data" {
            if data.is_some() {
                return Err("Duplicate WAV data.");
            }
            data = Some(payload);
        }
        at = at
            .checked_add(8 + n + (n & 1))
            .ok_or("Invalid WAV chunk.")?;
    }
    if at != b.len() {
        return Err("Invalid WAV padding.");
    }
    let (code, channels, rate, byte_rate, align, bits) = format.ok_or("Missing WAV format.")?;
    let samples = data.ok_or("Missing WAV samples.")?;
    if code != 1
        || !(1..=8).contains(&channels)
        || !(8000..=192000).contains(&rate)
        || !matches!(bits, 8 | 16)
        || align != channels * (bits / 8)
        || byte_rate != rate * align as u32
        || !samples.len().is_multiple_of(align as usize)
    {
        return Err("Only valid8/16-bit PCM WAV is supported.");
    }
    let frames = samples.len() / align as usize;
    let mut pixels = alloc::vec![255;768*256*4];
    for x in 0..768 {
        let start = x * frames / 768;
        let end = ((x + 1) * frames / 768).max(start + 1).min(frames);
        let mut peak = 0i32;
        for frame in start..end {
            for channel in 0..channels as usize {
                let p = frame * align as usize + channel * (bits / 8) as usize;
                let value = if bits == 8 {
                    (samples[p] as i32 - 128) * 256
                } else {
                    i16::from_le_bytes([samples[p], samples[p + 1]]) as i32
                };
                peak = peak.max(value.abs());
            }
        }
        let amplitude = (peak * 116 / 32768) as usize;
        for y in 128 - amplitude..=128 + amplitude {
            let p = (y * 768 + x) * 4;
            pixels[p..p + 4].copy_from_slice(&[206, 119, 0, 255]);
        }
    }
    let millis = frames as u64 * 1000 / rate as u64;
    Ok(Content {
        image: Some(Image {
            width: 768,
            height: 256,
            source_width: 768,
            source_height: 256,
            bgra: pixels,
        }),
        text: String::new(),
        details: alloc::vec![
            (
                "Duration".into(),
                format!("{}.{:03} s", millis / 1000, millis % 1000)
            ),
            ("Sample rate".into(), format!("{rate} Hz")),
            ("Channels".into(), channels.to_string()),
            ("Sample format".into(), format!("PCM {bits}-bit")),
            (
                "Playback".into(),
                "Audio output provider unavailable".into()
            )
        ],
    })
}
fn obj(b: &[u8]) -> Result<Content, &'static str> {
    let text = core::str::from_utf8(b).map_err(|_| "This provider reads text OBJ models.")?;
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for line in text.lines() {
        let mut words = line.split('#').next().unwrap_or("").split_whitespace();
        match words.next() {
            Some("v") => {
                if vertices.len() >= 4096 {
                    return Err("Model exceeds4096 vertices.");
                }
                let mut p = [0.; 3];
                for value in &mut p {
                    *value = words
                        .next()
                        .ok_or("Incomplete OBJ vertex.")?
                        .parse::<f32>()
                        .map_err(|_| "Invalid OBJ vertex.")?;
                    if !value.is_finite() || value.abs() > 1e9 {
                        return Err("Invalid OBJ vertex range.");
                    }
                }
                vertices.push(p);
            }
            Some("f") | Some("l") => {
                let mut points = Vec::new();
                for word in words {
                    if points.len() >= 64 {
                        return Err("OBJ face is too large.");
                    }
                    let index = word
                        .split('/')
                        .next()
                        .unwrap_or("")
                        .parse::<i32>()
                        .map_err(|_| "Invalid OBJ index.")?;
                    let i = if index < 0 {
                        vertices.len() as i64 + index as i64
                    } else {
                        index as i64 - 1
                    };
                    if i < 0 || i >= vertices.len() as i64 {
                        return Err("OBJ index is outside the vertex list.");
                    }
                    points.push(i as usize);
                }
                if points.len() < 2 {
                    return Err("Incomplete OBJ face.");
                }
                for pair in points.windows(2) {
                    edges.push((pair[0], pair[1]));
                }
                if points.len() > 2 {
                    edges.push((*points.last().unwrap(), points[0]));
                }
                if edges.len() > 16384 {
                    return Err("Model exceeds wireframe edge budget.");
                }
            }
            _ => (),
        }
    }
    if vertices.is_empty() || edges.is_empty() {
        return Err("OBJ contains no viewable geometry.");
    }
    let projected: Vec<_> = vertices
        .iter()
        .map(|p| [(p[0] + p[2]) * 0.70710677, p[1] * 0.9 + (p[2] - p[0]) * 0.2])
        .collect();
    let mut min = [f32::MAX; 2];
    let mut max = [f32::MIN; 2];
    for p in &projected {
        for a in 0..2 {
            min[a] = min[a].min(p[a]);
            max[a] = max[a].max(p[a]);
        }
    }
    let scale = 440. / (max[0] - min[0]).max(max[1] - min[1]).max(0.001);
    let center = [(max[0] + min[0]) / 2., (max[1] + min[1]) / 2.];
    let points: Vec<_> = projected
        .iter()
        .map(|p| {
            [
                ((p[0] - center[0]) * scale + 256.) as i32,
                (256. - (p[1] - center[1]) * scale) as i32,
            ]
        })
        .collect();
    let mut pixels = alloc::vec![249;512*512*4];
    for p in pixels.chunks_exact_mut(4) {
        p[3] = 255;
    }
    for &(a, b) in &edges {
        let (mut x, mut y) = (points[a][0], points[a][1]);
        let (end_x, end_y) = (points[b][0], points[b][1]);
        let (dx, dy) = ((end_x - x).abs(), -(end_y - y).abs());
        let (sx, sy) = (
            if x < end_x { 1 } else { -1 },
            if y < end_y { 1 } else { -1 },
        );
        let mut error = dx + dy;
        for _ in 0..2048 {
            if (0..512).contains(&x) && (0..512).contains(&y) {
                let at = (y as usize * 512 + x as usize) * 4;
                pixels[at..at + 4].copy_from_slice(&[200, 110, 0, 255]);
            }
            if x == end_x && y == end_y {
                break;
            }
            let e = error * 2;
            if e >= dy {
                error += dy;
                x += sx;
            }
            if e <= dx {
                error += dx;
                y += sy;
            }
        }
    }
    Ok(Content {
        image: Some(Image {
            width: 512,
            height: 512,
            source_width: 512,
            source_height: 512,
            bgra: pixels,
        }),
        text: String::new(),
        details: alloc::vec![
            ("Vertices".into(), vertices.len().to_string()),
            ("Edges".into(), edges.len().to_string()),
            ("View".into(), "Orthographic wireframe".into())
        ],
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn valid_obj_is_projected_and_invalid_indices_and_nonfinite_vertices_fail() {
        let model = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        let c = obj(model).unwrap();
        assert!(
            c.image
                .unwrap()
                .bgra
                .chunks_exact(4)
                .any(|p| p == [200, 110, 0, 255])
        );
        assert!(obj(b"v NaN 0 0\nf 1 1 1").is_err());
        assert!(obj(b"v 0 0 0\nf 0 1 1").is_err());
        assert!(zip(b"PK\x05\x06").is_err());
        assert!(wave(b"RIFF").is_err());
    }
    #[test]
    fn pcm_wave_metadata_and_waveform_come_from_validated_samples() {
        let samples = [0, 128, 255, 128];
        let mut b = Vec::from(&b"RIFF"[..]);
        b.extend_from_slice(&(36u32 + samples.len() as u32).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&8000u32.to_le_bytes());
        b.extend_from_slice(&8000u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&8u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        b.extend(samples);
        assert!(wave(&b).unwrap().image.is_some());
        b[32] = 2;
        assert!(wave(&b).is_err());
    }
}

pub use door_workspace::explorer_preview::encode_png as png;
#[cfg(test)]
mod png_tests {
    use super::*;
    #[test]
    fn png_export_round_trips_pixels_alpha_and_stored_block_boundaries() {
        for width in [2, 16384 / 4, 17000 / 4] {
            let height = if width == 2 { 2 } else { 5 };
            let mut bgra = Vec::new();
            for n in 0..width * height {
                let a = if n % 2 == 0 { 255 } else { 127 };
                bgra.extend_from_slice(&[a / 3, a / 2, a, a]);
            }
            let image = Image {
                width,
                height,
                source_width: width,
                source_height: height,
                bgra,
            };
            if width > 4096 {
                continue;
            }
            let encoded = png(&image).unwrap();
            let decoded = door_workspace::explorer_preview::decode_full(&encoded).unwrap();
            assert_eq!(decoded.bgra, image.bgra);
            assert_eq!((decoded.width, decoded.height), (width, height));
        }
    }
}

/// A waveform of decoded audio (peak and RMS per column, mirrored about the
/// centre line) on a transparent 768 x 256 image.
pub fn waveform(pcm: &door_workspace::audio_codec::Pcm) -> Image {
    const W: usize = 768;
    const H: usize = 256;
    let mut pixels = alloc::vec![0u8; W * H * 4];
    let channels = usize::from(pcm.channels.max(1));
    let frames = pcm.frames().max(1);
    let full = (1i64 << (pcm.bits.clamp(2, 32) - 1)) as f64;
    for x in 0..W {
        let start = x * frames / W;
        let end = ((x + 1) * frames / W).max(start + 1).min(frames);
        let (mut peak, mut sum, mut n) = (0f64, 0f64, 0f64);
        for f in start..end {
            for c in 0..channels {
                let v = f64::from(pcm.samples.get(f * channels + c).copied().unwrap_or(0)) / full;
                let a = if v < 0.0 { -v } else { v };
                peak = peak.max(a);
                sum += v * v;
                n += 1.0;
            }
        }
        // RMS without libm: Newton steps on the mean square.
        let ms = if n > 0.0 { sum / n } else { 0.0 };
        let mut rms = ms.max(1e-9);
        for _ in 0..12 {
            rms = 0.5 * (rms + ms / rms);
        }
        let half = (H / 2 - 4) as f64;
        let p = (peak.min(1.0) * half) as usize;
        let r = (rms.min(1.0) * half) as usize;
        for dy in 0..=p {
            // Peaks in a lighter tint, the RMS body in the accent blue.
            let color = if dy <= r {
                [206, 119, 0, 255]
            } else {
                [240, 196, 153, 255]
            };
            for y in [H / 2 - dy, H / 2 + dy] {
                let at = (y * W + x) * 4;
                pixels[at..at + 4].copy_from_slice(&color);
            }
        }
    }
    Image {
        width: W,
        height: H,
        source_width: W,
        source_height: H,
        bgra: pixels,
    }
}
/// Information-pane rows for decoded audio.
pub fn audio_details(
    pcm: &door_workspace::audio_codec::Pcm,
    bytes: &[u8],
) -> Vec<(String, String)> {
    use door_workspace::audio_codec::{Format, duration_text, sniff};
    let codec = match sniff(bytes) {
        Some(Format::Flac) => "FLAC (lossless)",
        Some(Format::Mp3) => "MP3 (MPEG-1/2 Layer III)",
        _ => "PCM WAV",
    };
    let channels = match pcm.channels {
        1 => String::from("Mono"),
        2 => String::from("Stereo"),
        n => alloc::format!("{n} channels"),
    };
    let rate = if pcm.rate.is_multiple_of(1000) {
        alloc::format!("{} kHz", pcm.rate / 1000)
    } else {
        alloc::format!("{}.{} kHz", pcm.rate / 1000, pcm.rate % 1000 / 100)
    };
    alloc::vec![
        ("Length".into(), duration_text(pcm.millis())),
        ("Audio format".into(), codec.into()),
        ("Sample rate".into(), rate),
        ("Channels".into(), channels),
        ("Bit depth".into(), alloc::format!("{}-bit", pcm.bits)),
    ]
}
