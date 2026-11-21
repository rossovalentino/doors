//! Audio codecs for Preview's playback and conversions (P15): PCM WAV read
//! and write, FLAC (RFC 9639) decode and encode, and MP3 decode (MPEG-1/2
//! Layer III through `nanomp3`, a no_std port of minimp3). Licensed or
//! patent-encumbered encoders (AAC, Opus containers, MP3 encoding) are not
//! offered.
//!
//! Decoded audio is interleaved signed integers at the source bit depth
//! (`Pcm::bits`, 8..=32); playback narrows it to 16 bits.
extern crate alloc;
use alloc::{string::String, vec::Vec};

/// Decoded audio: interleaved samples, each within `bits` signed bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcm {
    pub rate: u32,
    pub channels: u16,
    pub bits: u16,
    pub samples: Vec<i32>,
}
impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }
    /// Length in milliseconds.
    pub fn millis(&self) -> u64 {
        self.frames() as u64 * 1000 / u64::from(self.rate.max(1))
    }
    /// Interleaved little-endian 16-bit PCM (for door.audio streams).
    pub fn to_i16_bytes(&self) -> Vec<u8> {
        let shift = i32::from(self.bits) - 16;
        let mut out = Vec::with_capacity(self.samples.len() * 2);
        for &s in &self.samples {
            let v = if shift >= 0 { s >> shift } else { s << -shift };
            out.extend_from_slice(&(v.clamp(-32768, 32767) as i16).to_le_bytes());
        }
        out
    }
}

/// Why audio could not be read or written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Not this format, or a structure the reader does not accept.
    Malformed,
    /// A valid file using a feature this decoder does not implement.
    Unsupported,
    /// Larger than the decode limit.
    TooLarge,
}
impl Fault {
    pub const fn message(self) -> &'static str {
        match self {
            Self::Malformed => "The audio file is damaged or not in the format its name says.",
            Self::Unsupported => "This audio file uses an encoding Preview can't decode.",
            Self::TooLarge => "This audio file is too long to open in Preview.",
        }
    }
}
/// Decoded samples Preview holds at once (about 10 minutes of 48 kHz
/// stereo).
pub const MAX_SAMPLES: usize = 64 * 1024 * 1024 / 4;

/// The format of `bytes` by signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Wav,
    Flac,
    Mp3,
}
pub fn sniff(bytes: &[u8]) -> Option<Format> {
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        Some(Format::Wav)
    } else if bytes.starts_with(b"fLaC") {
        Some(Format::Flac)
    } else if bytes.starts_with(b"ID3")
        || (bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0)
    {
        Some(Format::Mp3)
    } else {
        None
    }
}
/// Decode any supported file.
pub fn decode(bytes: &[u8]) -> Result<Pcm, Fault> {
    match sniff(bytes).ok_or(Fault::Malformed)? {
        Format::Wav => decode_wav(bytes),
        Format::Flac => decode_flac(bytes),
        Format::Mp3 => decode_mp3(bytes),
    }
}

// ---- WAV ----

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
/// RIFF WAVE with integer PCM (8..32 bits) or IEEE float (32/64 bits,
/// stored as 24-bit integers), plain or WAVE_FORMAT_EXTENSIBLE.
pub fn decode_wav(b: &[u8]) -> Result<Pcm, Fault> {
    if sniff(b) != Some(Format::Wav) {
        return Err(Fault::Malformed);
    }
    let mut at = 12;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let len = u32_at(b, at + 4).ok_or(Fault::Malformed)? as usize;
        let end = (at + 8)
            .checked_add(len)
            .ok_or(Fault::Malformed)?
            .min(b.len());
        let body = &b[at + 8..end];
        if id == b"fmt " {
            let mut tag = u16_at(body, 0).ok_or(Fault::Malformed)?;
            let channels = u16_at(body, 2).ok_or(Fault::Malformed)?;
            let rate = u32_at(body, 4).ok_or(Fault::Malformed)?;
            let bits = u16_at(body, 14).ok_or(Fault::Malformed)?;
            if tag == 0xFFFE {
                tag = u16_at(body, 24).ok_or(Fault::Malformed)?;
            }
            fmt = Some((tag, channels, rate, bits));
        } else if id == b"data" {
            let (tag, channels, rate, bits) = fmt.ok_or(Fault::Malformed)?;
            if channels == 0 || channels > 8 || !(1_000..=384_000).contains(&rate) {
                return Err(Fault::Malformed);
            }
            let width = usize::from(bits.div_ceil(8));
            let (out_bits, read): (u16, fn(&[u8]) -> i32) = match (tag, bits) {
                (1, 8) => (8, |s| i32::from(s[0]) - 128),
                (1, 16) => (16, |s| i32::from(i16::from_le_bytes([s[0], s[1]]))),
                (1, 24) => (24, |s| i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8),
                (1, 32) => (32, |s| i32::from_le_bytes([s[0], s[1], s[2], s[3]])),
                (3, 32) => (24, |s| {
                    let f = f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                    (f.clamp(-1.0, 1.0) * 8_388_607.0) as i32
                }),
                (3, 64) => (24, |s| {
                    let f = f64::from_le_bytes(s[..8].try_into().unwrap());
                    (f.clamp(-1.0, 1.0) * 8_388_607.0) as i32
                }),
                (1 | 3, _) => return Err(Fault::Unsupported),
                _ => return Err(Fault::Unsupported),
            };
            let frame = width * usize::from(channels);
            let count = body.len() / frame * usize::from(channels);
            if count > MAX_SAMPLES {
                return Err(Fault::TooLarge);
            }
            let samples = body.chunks_exact(width).take(count).map(read).collect();
            return Ok(Pcm {
                rate,
                channels,
                bits: out_bits,
                samples,
            });
        }
        at = end + (len & 1);
    }
    Err(Fault::Malformed)
}
/// RIFF WAVE, integer PCM at `pcm.bits` rounded up to whole bytes.
pub fn encode_wav(pcm: &Pcm) -> Vec<u8> {
    let width = usize::from(pcm.bits.div_ceil(8)).clamp(1, 4);
    let bits = (width * 8) as u16;
    let shift = u32::from(bits) - u32::from(pcm.bits.min(bits));
    let data = pcm.samples.len() * width;
    let mut out = Vec::with_capacity(44 + data);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + data) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&pcm.channels.to_le_bytes());
    out.extend_from_slice(&pcm.rate.to_le_bytes());
    let block = width as u32 * u32::from(pcm.channels);
    out.extend_from_slice(&(pcm.rate * block).to_le_bytes());
    out.extend_from_slice(&(block as u16).to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data as u32).to_le_bytes());
    for &s in &pcm.samples {
        let v = s << shift;
        match width {
            1 => out.push((v + 128) as u8),
            2 => out.extend_from_slice(&(v as i16).to_le_bytes()),
            3 => out.extend_from_slice(&v.to_le_bytes()[..3]),
            _ => out.extend_from_slice(&v.to_le_bytes()),
        }
    }
    out
}

// ---- FLAC ----

struct Bits<'a> {
    b: &'a [u8],
    pos: usize,
}
impl<'a> Bits<'a> {
    fn new(b: &'a [u8], byte: usize) -> Self {
        Self { b, pos: byte * 8 }
    }
    fn bit(&mut self) -> Result<u32, Fault> {
        let byte = *self.b.get(self.pos / 8).ok_or(Fault::Malformed)?;
        let v = (byte >> (7 - self.pos % 8)) & 1;
        self.pos += 1;
        Ok(u32::from(v))
    }
    fn bits(&mut self, n: u32) -> Result<u32, Fault> {
        let mut v: u64 = 0;
        let mut left = n;
        while left > 0 {
            let byte = *self.b.get(self.pos / 8).ok_or(Fault::Malformed)?;
            let used = (self.pos % 8) as u32;
            let take = (8 - used).min(left);
            let chunk = (u32::from(byte) >> (8 - used - take)) & ((1 << take) - 1);
            v = (v << take) | u64::from(chunk);
            self.pos += take as usize;
            left -= take;
        }
        Ok(v as u32)
    }
    fn signed(&mut self, n: u32) -> Result<i32, Fault> {
        if n == 0 {
            return Ok(0);
        }
        if n > 32 {
            // 33-bit side channels of 32-bit audio are not supported.
            return Err(Fault::Unsupported);
        }
        let v = self.bits(n)?;
        Ok(((v << (32 - n)) as i32) >> (32 - n))
    }
    fn unary(&mut self) -> Result<u32, Fault> {
        let mut n = 0;
        while self.bit()? == 0 {
            n += 1;
            if n > 1 << 20 {
                return Err(Fault::Malformed);
            }
        }
        Ok(n)
    }
    fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
    fn byte(&self) -> usize {
        self.pos / 8
    }
}
fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}
fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in data {
        crc ^= u16::from(b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc
}
struct StreamInfo {
    rate: u32,
    channels: u16,
    bits: u16,
    total: u64,
}
/// Decode a FLAC stream (every subframe type, Rice and Rice2 residuals,
/// all stereo decorrelations; frame CRCs are checked).
pub fn decode_flac(b: &[u8]) -> Result<Pcm, Fault> {
    if !b.starts_with(b"fLaC") {
        return Err(Fault::Malformed);
    }
    let mut at = 4;
    let mut info = None;
    loop {
        let head = *b.get(at).ok_or(Fault::Malformed)?;
        let len = (usize::from(*b.get(at + 1).ok_or(Fault::Malformed)?) << 16)
            | (usize::from(*b.get(at + 2).ok_or(Fault::Malformed)?) << 8)
            | usize::from(*b.get(at + 3).ok_or(Fault::Malformed)?);
        let body = b.get(at + 4..at + 4 + len).ok_or(Fault::Malformed)?;
        if head & 0x7F == 0 {
            if len < 34 {
                return Err(Fault::Malformed);
            }
            let mut r = Bits::new(body, 0);
            r.bits(16)?;
            r.bits(16)?;
            r.bits(24)?;
            r.bits(24)?;
            let rate = r.bits(20)?;
            let channels = r.bits(3)? as u16 + 1;
            let bits = r.bits(5)? as u16 + 1;
            let total = (u64::from(r.bits(4)?) << 32) | u64::from(r.bits(32)?);
            if rate == 0 || bits < 4 {
                return Err(Fault::Malformed);
            }
            info = Some(StreamInfo {
                rate,
                channels,
                bits,
                total,
            });
        }
        at += 4 + len;
        if head & 0x80 != 0 {
            break;
        }
    }
    let info = info.ok_or(Fault::Malformed)?;
    let expected = info.total as usize * usize::from(info.channels);
    if expected > MAX_SAMPLES {
        return Err(Fault::TooLarge);
    }
    let mut samples: Vec<i32> = Vec::with_capacity(expected);
    let mut channels: Vec<Vec<i32>> = Vec::new();
    while at + 2 <= b.len() {
        if b[at] != 0xFF || b[at + 1] & 0xFE != 0xF8 {
            // Trailing tags (ID3) or padding after the last frame.
            if info.total != 0 && samples.len() >= expected {
                break;
            }
            at += 1;
            continue;
        }
        let start = at;
        let mut r = Bits::new(b, at);
        r.bits(15)?; // sync and reserved bit
        let _variable = r.bit()?;
        let size_code = r.bits(4)?;
        let rate_code = r.bits(4)?;
        let assignment = r.bits(4)?;
        let depth_code = r.bits(3)?;
        if r.bit()? != 0 {
            return Err(Fault::Malformed);
        }
        // Coded frame/sample number (UTF-8 style, up to 7 bytes).
        let lead = r.bits(8)?;
        let extra = match lead {
            0x00..=0x7F => 0,
            0xC0..=0xDF => 1,
            0xE0..=0xEF => 2,
            0xF0..=0xF7 => 3,
            0xF8..=0xFB => 4,
            0xFC..=0xFD => 5,
            0xFE => 6,
            _ => return Err(Fault::Malformed),
        };
        for _ in 0..extra {
            if r.bits(8)? & 0xC0 != 0x80 {
                return Err(Fault::Malformed);
            }
        }
        let block = match size_code {
            1 => 192,
            2..=5 => 576 << (size_code - 2),
            6 => r.bits(8)? + 1,
            7 => r.bits(16)? + 1,
            8..=15 => 256 << (size_code - 8),
            _ => return Err(Fault::Malformed),
        } as usize;
        match rate_code {
            12 => {
                r.bits(8)?;
            }
            13 | 14 => {
                r.bits(16)?;
            }
            15 => return Err(Fault::Malformed),
            _ => (),
        }
        let header_end = r.byte();
        let crc = r.bits(8)? as u8;
        if crc8(&b[start..header_end]) != crc {
            return Err(Fault::Malformed);
        }
        let depth = match depth_code {
            0 => info.bits,
            1 => 8,
            2 => 12,
            4 => 16,
            5 => 20,
            6 => 24,
            7 => 32,
            _ => return Err(Fault::Malformed),
        };
        let count = match assignment {
            0..=7 => assignment as usize + 1,
            8..=10 => 2,
            _ => return Err(Fault::Malformed),
        };
        if count != usize::from(info.channels) {
            return Err(Fault::Malformed);
        }
        channels.resize(count, Vec::new());
        for (ch, out) in channels.iter_mut().enumerate() {
            let side = matches!((assignment, ch), (8, 1) | (9, 0) | (10, 1));
            subframe(&mut r, block, u32::from(depth) + u32::from(side), out)?;
        }
        r.align();
        let footer = r.byte();
        let crc = r.bits(16)? as u16;
        if crc16(&b[start..footer]) != crc {
            return Err(Fault::Malformed);
        }
        at = r.byte();
        for i in 0..block {
            match assignment {
                8 => {
                    let (l, s) = (channels[0][i], channels[1][i]);
                    samples.push(l);
                    samples.push(l.wrapping_sub(s));
                }
                9 => {
                    let (s, rt) = (channels[0][i], channels[1][i]);
                    samples.push(s.wrapping_add(rt));
                    samples.push(rt);
                }
                10 => {
                    let (m, s) = (i64::from(channels[0][i]), i64::from(channels[1][i]));
                    let m = (m << 1) | (s & 1);
                    samples.push(((m + s) >> 1) as i32);
                    samples.push(((m - s) >> 1) as i32);
                }
                _ => {
                    for c in &channels {
                        samples.push(c[i]);
                    }
                }
            }
        }
        if samples.len() > MAX_SAMPLES {
            return Err(Fault::TooLarge);
        }
    }
    if info.total != 0 {
        samples.truncate(expected);
    }
    Ok(Pcm {
        rate: info.rate,
        channels: info.channels,
        bits: info.bits,
        samples,
    })
}
fn subframe(r: &mut Bits<'_>, block: usize, bps: u32, out: &mut Vec<i32>) -> Result<(), Fault> {
    out.clear();
    if r.bit()? != 0 {
        return Err(Fault::Malformed);
    }
    let kind = r.bits(6)?;
    let wasted = if r.bit()? == 1 { r.unary()? + 1 } else { 0 };
    if bps > 32 {
        return Err(Fault::Unsupported);
    }
    let bps = bps
        .checked_sub(wasted)
        .filter(|b| *b > 0)
        .ok_or(Fault::Malformed)?;
    match kind {
        0 => {
            let v = r.signed(bps)?;
            out.resize(block, v);
        }
        1 => {
            for _ in 0..block {
                out.push(r.signed(bps)?);
            }
        }
        8..=12 => {
            let order = (kind - 8) as usize;
            if order > block {
                return Err(Fault::Malformed);
            }
            for _ in 0..order {
                out.push(r.signed(bps)?);
            }
            residual(r, block, order, out)?;
            for i in order..block {
                let s = |k: usize| i64::from(out[i - k]);
                let p = match order {
                    0 => 0,
                    1 => s(1),
                    2 => 2 * s(1) - s(2),
                    3 => 3 * s(1) - 3 * s(2) + s(3),
                    _ => 4 * s(1) - 6 * s(2) + 4 * s(3) - s(4),
                };
                out[i] = (i64::from(out[i]) + p) as i32;
            }
        }
        32..=63 => {
            let order = (kind - 31) as usize;
            if order > block {
                return Err(Fault::Malformed);
            }
            for _ in 0..order {
                out.push(r.signed(bps)?);
            }
            let precision = r.bits(4)?;
            if precision == 15 {
                return Err(Fault::Malformed);
            }
            let shift = r.signed(5)?;
            if shift < 0 {
                return Err(Fault::Unsupported);
            }
            let mut coefs = [0i64; 32];
            for c in coefs.iter_mut().take(order) {
                *c = i64::from(r.signed(precision + 1)?);
            }
            residual(r, block, order, out)?;
            for i in order..block {
                let mut sum = 0i64;
                for (j, c) in coefs.iter().take(order).enumerate() {
                    sum += c * i64::from(out[i - 1 - j]);
                }
                out[i] = (i64::from(out[i]) + (sum >> shift)) as i32;
            }
        }
        _ => return Err(Fault::Malformed),
    }
    if wasted > 0 {
        for v in out.iter_mut() {
            *v = v.wrapping_shl(wasted);
        }
    }
    Ok(())
}
/// Residual samples appended to `out` (after `order` warm-up samples).
fn residual(r: &mut Bits<'_>, block: usize, order: usize, out: &mut Vec<i32>) -> Result<(), Fault> {
    let method = r.bits(2)?;
    let (param_bits, escape) = match method {
        0 => (4, 15),
        1 => (5, 31),
        _ => return Err(Fault::Malformed),
    };
    let partition_order = r.bits(4)?;
    let partitions = 1usize << partition_order;
    if !block.is_multiple_of(partitions) || (block >> partition_order) < order {
        return Err(Fault::Malformed);
    }
    for p in 0..partitions {
        let n = (block >> partition_order) - if p == 0 { order } else { 0 };
        let param = r.bits(param_bits)?;
        if param == escape {
            let raw = r.bits(5)?;
            for _ in 0..n {
                out.push(r.signed(raw)?);
            }
        } else {
            for _ in 0..n {
                let q = r.unary()?;
                let v = (q << param) | r.bits(param)?;
                out.push(((v >> 1) as i32) ^ -((v & 1) as i32));
            }
        }
    }
    Ok(())
}

/// MSB-first bit writer.
struct Writer {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}
impl Writer {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            acc: 0,
            n: 0,
        }
    }
    fn put(&mut self, bits: u32, value: u64) {
        if bits == 0 {
            return;
        }
        let value = if bits == 64 {
            value
        } else {
            value & ((1u64 << bits) - 1)
        };
        if bits > 32 {
            self.put(bits - 32, value >> 32);
            self.put(32, value & 0xFFFF_FFFF);
            return;
        }
        self.acc = (self.acc << bits) | value;
        self.n += bits;
        while self.n >= 8 {
            self.n -= 8;
            self.out.push((self.acc >> self.n) as u8);
        }
        self.acc &= (1u64 << self.n) - 1;
    }
    fn signed(&mut self, bits: u32, v: i64) {
        self.put(bits, v as u64);
    }
    fn unary(&mut self, zeros: u32) {
        let mut left = zeros;
        while left >= 32 {
            self.put(32, 0);
            left -= 32;
        }
        self.put(left + 1, 1);
    }
    fn align(&mut self) {
        if self.n > 0 {
            self.put(8 - self.n, 0);
        }
    }
}
/// The bits a Rice partition takes with parameter `k`.
fn rice_cost(residual: &[i64], k: u32) -> u64 {
    residual
        .iter()
        .map(|&v| {
            let u = ((v << 1) ^ (v >> 63)) as u64;
            (u >> k) + 1 + u64::from(k)
        })
        .sum()
}
fn best_rice(residual: &[i64], max: u32) -> (u32, u64) {
    let mut best = (0, u64::MAX);
    for k in 0..=max {
        let c = rice_cost(residual, k);
        if c < best.1 {
            best = (k, c);
        }
    }
    best
}
/// Partition order and per-partition parameters with the smallest size.
fn plan_residual(residual: &[i64], block: usize, order: usize) -> Plan {
    let mut best: Plan = (0, Vec::new(), u64::MAX);
    for po in 0..=8u32 {
        let parts = 1usize << po;
        if !block.is_multiple_of(parts) || (block >> po) <= order {
            break;
        }
        let mut params = Vec::with_capacity(parts);
        let mut total = 6u64;
        let mut start = 0;
        for p in 0..parts {
            let n = (block >> po) - if p == 0 { order } else { 0 };
            let (k, c) = best_rice(&residual[start..start + n], 30);
            params.push(k);
            total += 5 + c;
            start += n;
        }
        if total < best.2 {
            best = (po, params, total);
        }
    }
    best
}
fn fixed_residual(x: &[i64], order: usize) -> Vec<i64> {
    (order..x.len())
        .map(|i| {
            let s = |k: usize| x[i - k];
            x[i] - match order {
                0 => 0,
                1 => s(1),
                2 => 2 * s(1) - s(2),
                3 => 3 * s(1) - 3 * s(2) + s(3),
                _ => 4 * s(1) - 6 * s(2) + 4 * s(3) - s(4),
            }
        })
        .collect()
}
/// Quantized LPC coefficients (precision 15) of `order` by Levinson-Durbin
/// over the block's autocorrelation; None when the block is silent.
fn lpc(x: &[i64], order: usize) -> Option<(Vec<i64>, i32)> {
    let mut r = [0f64; 13];
    for (lag, v) in r.iter_mut().enumerate().take(order + 1) {
        *v = (lag..x.len())
            .map(|i| x[i] as f64 * x[i - lag] as f64)
            .sum();
    }
    if r[0] <= 0.0 {
        return None;
    }
    let mut a = [0f64; 13];
    let mut err = r[0];
    for i in 1..=order {
        let mut k = r[i];
        for j in 1..i {
            k -= a[j] * r[i - j];
        }
        k /= err;
        let prev = a;
        a[i] = k;
        for j in 1..i {
            a[j] = prev[j] - k * prev[i - j];
        }
        err *= 1.0 - k * k;
        if err <= 0.0 {
            return None;
        }
    }
    let max = a[1..=order]
        .iter()
        .fold(0f64, |m, v| m.max(if *v < 0.0 { -*v } else { *v }));
    // Shift so the largest coefficient fits 15 signed bits.
    let mut shift = 14i32;
    while shift > 0 && max * f64::from(1 << shift) > 16383.0 {
        shift -= 1;
    }
    let q = a[1..=order]
        .iter()
        .map(|c| {
            let v = c * f64::from(1 << shift);
            (if v < 0.0 { v - 0.5 } else { v + 0.5 }) as i64
        })
        .collect();
    Some((q, shift))
}
fn lpc_residual(x: &[i64], coefs: &[i64], shift: i32) -> Vec<i64> {
    let order = coefs.len();
    (order..x.len())
        .map(|i| {
            let p: i64 = coefs
                .iter()
                .enumerate()
                .map(|(j, c)| c * x[i - 1 - j])
                .sum();
            x[i] - (p >> shift)
        })
        .collect()
}
/// Write one subframe of `x` at `bps` bits, choosing the smallest of
/// constant, fixed orders 0..=4, LPC orders 8 and 12, and verbatim.
/// The residual plan: partition order, Rice parameters, total bits.
type Plan = (u32, Vec<u32>, u64);
fn write_subframe(w: &mut Writer, x: &[i64], bps: u32) {
    if x.iter().all(|v| *v == x[0]) {
        w.put(8, 0);
        w.signed(bps, x[0]);
        return;
    }
    let block = x.len();
    let verbatim = bps as u64 * block as u64;
    enum Pick {
        Fixed(usize),
        Lpc(Vec<i64>, i32),
    }
    let mut best: Option<(Pick, Vec<i64>, Plan)> = None;
    for order in 0..=4.min(block - 1) {
        let res = fixed_residual(x, order);
        let plan = plan_residual(&res, block, order);
        let cost = plan.2 + order as u64 * u64::from(bps);
        if best.as_ref().is_none_or(|b| cost < b.2.2) {
            best = Some((Pick::Fixed(order), res, (plan.0, plan.1, cost)));
        }
    }
    for order in [8usize, 12] {
        if block <= order * 2 {
            continue;
        }
        let Some((coefs, shift)) = lpc(x, order) else {
            continue;
        };
        let res = lpc_residual(x, &coefs, shift);
        let plan = plan_residual(&res, block, order);
        let cost = plan.2 + order as u64 * (u64::from(bps) + 15) + 9;
        if best.as_ref().is_none_or(|b| cost < b.2.2) {
            best = Some((Pick::Lpc(coefs, shift), res, (plan.0, plan.1, cost)));
        }
    }
    let Some((pick, res, (po, params, cost))) = best else {
        return;
    };
    if cost >= verbatim {
        w.put(8, 1 << 1);
        for &v in x {
            w.signed(bps, v);
        }
        return;
    }
    let order = match &pick {
        Pick::Fixed(order) => {
            w.put(8, (8 + *order as u64) << 1);
            *order
        }
        Pick::Lpc(coefs, _) => {
            w.put(8, (31 + coefs.len() as u64) << 1);
            coefs.len()
        }
    };
    for &v in &x[..order] {
        w.signed(bps, v);
    }
    if let Pick::Lpc(coefs, shift) = &pick {
        w.put(4, 14); // precision 15
        w.signed(5, i64::from(*shift));
        for &c in coefs {
            w.signed(15, c);
        }
    }
    let rice2 = params.iter().any(|k| *k > 14);
    w.put(2, u64::from(rice2));
    w.put(4, u64::from(po));
    let mut start = 0;
    for (p, &k) in params.iter().enumerate() {
        let n = (block >> po) - if p == 0 { order } else { 0 };
        w.put(if rice2 { 5 } else { 4 }, u64::from(k));
        for &v in &res[start..start + n] {
            let u = ((v << 1) ^ (v >> 63)) as u64;
            w.unary((u >> k) as u32);
            w.put(k, u & ((1u64 << k) - 1));
        }
        start += n;
    }
}
/// The FLAC block size this encoder uses.
pub const FLAC_BLOCK: usize = 4096;
/// Encode `pcm` (1..=8 channels, 4..=32 bits) as FLAC; stereo frames pick
/// mid/side when it is smaller. STREAMINFO's MD5 is left zero ("unknown").
pub fn encode_flac(pcm: &Pcm) -> Result<Vec<u8>, Fault> {
    let channels = usize::from(pcm.channels);
    if !(1..=8).contains(&channels)
        || !(4..=32).contains(&pcm.bits)
        || pcm.rate == 0
        || pcm.rate >= 1 << 20
    {
        return Err(Fault::Unsupported);
    }
    let frames = pcm.frames();
    let mut w = Writer::new();
    w.out.extend_from_slice(b"fLaC");
    w.put(8, 0x80); // last metadata block, STREAMINFO
    w.put(24, 34);
    let size = frames.clamp(16, FLAC_BLOCK) as u64;
    w.put(16, size);
    w.put(16, size);
    w.put(24, 0);
    w.put(24, 0);
    w.put(20, u64::from(pcm.rate));
    w.put(3, channels as u64 - 1);
    w.put(5, u64::from(pcm.bits) - 1);
    w.put(36, frames as u64);
    w.put(64, 0);
    w.put(64, 0);
    let bps = u32::from(pcm.bits);
    let depth_code = match pcm.bits {
        8 => 1,
        12 => 2,
        16 => 4,
        20 => 5,
        24 => 6,
        32 => 7,
        _ => 0,
    };
    let mut start = 0;
    let mut number = 0u64;
    while start < frames {
        let block = (frames - start).min(FLAC_BLOCK);
        let mut f = Writer::new();
        f.put(14, 0x3FFE);
        f.put(1, 0);
        f.put(1, 0); // fixed block size
        let size_code = if block == FLAC_BLOCK { 12 } else { 7 };
        f.put(4, size_code);
        f.put(4, 0); // rate from STREAMINFO
        let column = |c: usize| -> Vec<i64> {
            (0..block)
                .map(|i| i64::from(pcm.samples[(start + i) * channels + c]))
                .collect()
        };
        let cols: Vec<Vec<i64>> = (0..channels).map(column).collect();
        // Stereo: mid/side when its channels are smaller (32-bit samples
        // stay independent: the side channel would need 33 bits).
        let mid_side = channels == 2 && bps < 32 && {
            let side: Vec<i64> = cols[0].iter().zip(&cols[1]).map(|(l, r)| l - r).collect();
            let mid: Vec<i64> = cols[0]
                .iter()
                .zip(&cols[1])
                .map(|(l, r)| (l + r) >> 1)
                .collect();
            let est = |x: &[i64]| {
                fixed_residual(x, 2.min(x.len() - 1))
                    .iter()
                    .map(|v| v.unsigned_abs())
                    .sum::<u64>()
            };
            est(&mid) + est(&side) < est(&cols[0]) + est(&cols[1])
        };
        f.put(4, if mid_side { 10 } else { channels as u64 - 1 });
        f.put(3, depth_code);
        f.put(1, 0);
        // Frame number, UTF-8 coded.
        if number < 0x80 {
            f.put(8, number);
        } else {
            let mut bytes = Vec::new();
            let mut n = number;
            while n >= 0x40 >> bytes.len() {
                bytes.push(0x80 | (n & 0x3F) as u8);
                n >>= 6;
            }
            let lead = (0xFF00u16 >> (bytes.len() + 1)) as u8 | n as u8;
            f.put(8, u64::from(lead));
            for b in bytes.iter().rev() {
                f.put(8, u64::from(*b));
            }
        }
        if size_code == 7 {
            f.put(16, block as u64 - 1);
        }
        let header = f.out.clone();
        f.put(8, u64::from(crc8(&header)));
        if mid_side {
            let mid: Vec<i64> = cols[0]
                .iter()
                .zip(&cols[1])
                .map(|(l, r)| (l + r) >> 1)
                .collect();
            let side: Vec<i64> = cols[0].iter().zip(&cols[1]).map(|(l, r)| l - r).collect();
            write_subframe(&mut f, &mid, bps);
            write_subframe(&mut f, &side, bps + 1);
        } else {
            for c in &cols {
                write_subframe(&mut f, c, bps);
            }
        }
        f.align();
        let crc = crc16(&f.out);
        f.put(16, u64::from(crc));
        w.out.extend_from_slice(&f.out);
        start += block;
        number += 1;
    }
    Ok(w.out)
}

// ---- MP3 ----

/// Decode MPEG audio Layer III (MPEG-1, 2 and 2.5) to 16-bit PCM; ID3v2
/// tags are skipped. Free-format and Layer I/II streams are refused.
pub fn decode_mp3(b: &[u8]) -> Result<Pcm, Fault> {
    let mut at = 0;
    if b.starts_with(b"ID3") && b.len() >= 10 {
        let size = b[6..10]
            .iter()
            .fold(0usize, |s, v| (s << 7) | usize::from(v & 0x7F));
        at = 10 + size + if b[5] & 0x10 != 0 { 10 } else { 0 };
    }
    let mut decoder = nanomp3::Decoder::new();
    let mut pcm = alloc::vec![0f32; nanomp3::MAX_SAMPLES_PER_FRAME];
    let mut samples = Vec::new();
    let mut format: Option<(u32, u16)> = None;
    while at < b.len() {
        let (used, info) = decoder.decode(&b[at..], &mut pcm);
        if used == 0 {
            break;
        }
        at += used;
        if let Some(info) = info {
            let channels = info.channels.num() as u16;
            let rate = info.sample_rate;
            let f = *format.get_or_insert((rate, channels));
            if f != (rate, channels) {
                continue;
            }
            let n = info.samples_produced * usize::from(channels);
            samples.extend(
                pcm[..n]
                    .iter()
                    .map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i32),
            );
            if samples.len() > MAX_SAMPLES {
                return Err(Fault::TooLarge);
            }
        }
    }
    let (rate, channels) = format.ok_or(Fault::Malformed)?;
    Ok(Pcm {
        rate,
        channels,
        bits: 16,
        samples,
    })
}

/// "3:07"-style duration text.
pub fn duration_text(millis: u64) -> String {
    let s = millis / 1000;
    if s >= 3600 {
        alloc::format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        alloc::format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tone(rate: u32, channels: u16, bits: u16, frames: usize) -> Pcm {
        let amp = ((1i64 << (bits - 1)) - 1) as f64 * 0.6;
        let mut samples = Vec::new();
        let mut seed = 1u32;
        for i in 0..frames {
            let t = i as f64 / rate as f64;
            for c in 0..channels {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                let noise = (seed >> 16) as f64 / 65536.0 - 0.5;
                let x = libm_sin(2.0 * core::f64::consts::PI * (440.0 + 110.0 * c as f64) * t);
                samples.push((x * amp + noise * 30.0) as i32);
            }
        }
        Pcm {
            rate,
            channels,
            bits,
            samples,
        }
    }
    fn libm_sin(x: f64) -> f64 {
        let x = x % (2.0 * core::f64::consts::PI);
        let (mut term, mut sum, mut n) = (x, 0.0, 1.0);
        for _ in 0..12 {
            sum += term;
            term *= -x * x / ((n + 1.0) * (n + 2.0));
            n += 2.0;
        }
        sum
    }
    #[test]
    fn flac_round_trips_bit_exactly_and_compresses() {
        for (rate, channels, bits, frames) in [
            (44_100, 2, 16, 10_000),
            (48_000, 1, 24, 5_000),
            (22_050, 2, 8, 4096),
            (8_000, 3, 12, 100),
        ] {
            let pcm = tone(rate, channels, bits, frames);
            let flac = encode_flac(&pcm).unwrap();
            assert_eq!(sniff(&flac), Some(Format::Flac));
            let back = decode_flac(&flac).unwrap();
            assert_eq!(back, pcm, "{rate} {channels}ch {bits}-bit");
            if bits >= 16 && frames > 1000 {
                let raw = pcm.samples.len() * usize::from(bits / 8);
                assert!(flac.len() < raw * 3 / 4, "{} vs {raw}", flac.len());
            }
        }
        // Silence is constant subframes.
        let quiet = Pcm {
            rate: 48_000,
            channels: 2,
            bits: 16,
            samples: alloc::vec![0; 9000],
        };
        assert!(encode_flac(&quiet).unwrap().len() < 200);
        assert_eq!(decode_flac(&encode_flac(&quiet).unwrap()).unwrap(), quiet);
    }
    #[test]
    fn wav_round_trips_and_reads_float() {
        let pcm = tone(48_000, 2, 16, 2000);
        let wav = encode_wav(&pcm);
        assert_eq!(decode_wav(&wav).unwrap(), pcm);
        let p24 = tone(44_100, 1, 24, 300);
        assert_eq!(decode_wav(&encode_wav(&p24)).unwrap(), p24);
        // IEEE float 32-bit, mono.
        let mut f = encode_wav(&Pcm {
            rate: 8000,
            channels: 1,
            bits: 32,
            samples: alloc::vec![0, 0],
        });
        f[20] = 3;
        f[44..48].copy_from_slice(&0.5f32.to_le_bytes());
        f[48..52].copy_from_slice(&(-1.0f32).to_le_bytes());
        let d = decode_wav(&f).unwrap();
        assert_eq!(
            (d.bits, d.samples[0], d.samples[1]),
            (24, 4_194_303, -8_388_607)
        );
        assert_eq!(pcm.to_i16_bytes().len(), pcm.samples.len() * 2);
    }
    #[test]
    fn damaged_flac_frames_are_refused() {
        let pcm = tone(44_100, 2, 16, 5000);
        let mut flac = encode_flac(&pcm).unwrap();
        let n = flac.len();
        flac[n / 2] ^= 0x10;
        assert_eq!(decode_flac(&flac), Err(Fault::Malformed));
        assert_eq!(decode_flac(b"fLaC\x80\0\0\x02ab"), Err(Fault::Malformed));
        assert_eq!(decode(b"not audio"), Err(Fault::Malformed));
        assert_eq!(duration_text(187_000), "3:07");
    }
}

/// Cross-checks against an independent codec (Windows Media Foundation's
/// FLAC and MP3 encoders and its FLAC decoder): run with
/// `DOOR_AUDIO_REF=<dir with ref.wav, ref.flac, ref.mp3>`; when
/// `<dir>/door.flac` is decoded back by the reference into `door.wav`, that
/// is compared too.
#[cfg(test)]
mod reference {
    extern crate std;
    use super::*;
    #[test]
    #[ignore = "needs DOOR_AUDIO_REF reference files"]
    fn decodes_reference_encoders_and_writes_flac_they_read() {
        let dir = std::env::var("DOOR_AUDIO_REF").expect("DOOR_AUDIO_REF");
        let read = |n: &str| std::fs::read(std::format!("{dir}/{n}")).unwrap();
        let wav = decode_wav(&read("ref.wav")).unwrap();
        let flac = decode_flac(&read("ref.flac")).unwrap();
        assert_eq!((flac.rate, flac.channels), (wav.rate, wav.channels));
        // Media Foundation stores 16-bit sources as 24-bit FLAC.
        let shift = flac.bits - wav.bits;
        let narrowed: Vec<i32> = flac.samples.iter().map(|s| s >> shift).collect();
        assert_eq!(narrowed, wav.samples, "reference FLAC decodes losslessly");
        assert!(flac.samples.iter().all(|s| s & ((1 << shift) - 1) == 0));
        let mp3 = decode_mp3(&read("ref.mp3")).unwrap();
        assert_eq!((mp3.rate, mp3.channels), (wav.rate, wav.channels));
        // Encoder delay and padding: within a few frames of the source.
        let diff = mp3.frames().abs_diff(wav.frames());
        assert!(
            diff < 1152 * 4,
            "mp3 frames {} vs {}",
            mp3.frames(),
            wav.frames()
        );
        let ours = encode_flac(&wav).unwrap();
        std::fs::write(std::format!("{dir}/door.flac"), &ours).unwrap();
        if let Ok(back) = std::fs::read(std::format!("{dir}/door.wav")) {
            assert_eq!(
                decode_wav(&back).unwrap().samples,
                wav.samples,
                "reference decodes our FLAC"
            );
        }
        std::println!(
            "ref.flac {} bytes, door.flac {} bytes",
            read("ref.flac").len(),
            ours.len()
        );
    }
}

/// Writes `assets/sounds/samples/doors-chime.flac`, the original sample the
/// test image installs to `/system/share` (a 660 Hz bell with a 2.4 s
/// exponential decay, 48 kHz stereo 16-bit): `DOOR_WRITE_CHIME=<path>`.
#[cfg(test)]
mod chime {
    extern crate std;
    use super::*;
    #[test]
    #[ignore = "writes the sample asset"]
    fn write_doors_chime() {
        let path = std::env::var("DOOR_WRITE_CHIME").expect("DOOR_WRITE_CHIME");
        let rate = 48_000u32;
        let frames = rate as usize * 12 / 5;
        let w = 2.0 * core::f64::consts::PI * 660.0 / f64::from(rate);
        // sin(w) and cos(w) by series (no libm), then the recurrence.
        let (w2, w3) = (w * w, w * w * w);
        let cos = 1.0 - w2 / 2.0 + w2 * w2 / 24.0 - w2 * w2 * w2 / 720.0;
        let sin = w - w3 / 6.0 + w3 * w2 / 120.0 - w3 * w2 * w2 / 5040.0;
        let (mut y1, mut y2) = (0.0f64, -sin);
        let mut samples = Vec::with_capacity(frames * 2);
        let mut envelope = 1.0f64;
        let decay = 1.0 - 4.5 / f64::from(rate);
        for i in 0..frames {
            let y = 2.0 * cos * y1 - y2;
            y2 = y1;
            y1 = y;
            let attack = (i as f64 / 480.0).min(1.0);
            let v = (y * envelope * attack * 0.5 * 32767.0) as i32;
            envelope *= decay;
            samples.push(v);
            samples.push(v);
        }
        let pcm = Pcm {
            rate,
            channels: 2,
            bits: 16,
            samples,
        };
        std::fs::write(&path, encode_flac(&pcm).unwrap()).unwrap();
    }
}
