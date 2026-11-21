#![no_std]
//! Original DirectSound (dsound.dll) over door.audio. DirectSound's
//! hardware model is reproduced in software: secondary buffers are mixed
//! (volume, pan, frequency) into one door.audio stream at the primary
//! buffer's rate, play and write cursors follow what door.audio has
//! played and what has been mixed, and position notifications set events
//! as the play cursor passes them.
//!
//! This module is the host-testable part: the result codes and flags of
//! dsound.h, WAVEFORMATEX parsing, the dB-to-gain law, the resampling
//! mixer and notification crossing. The COM objects are in `guest`.
//!
//! The same sources build the x64 and the x86 DLL (P17): Windows-facing
//! exports take pointer-width parameters, widened at entry; a pointer or
//! handle result is narrowed from the body's 64-bit value (an immediately
//! called closure, so the body's own `return`s keep their meaning).
#![allow(clippy::redundant_closure_call)]
extern crate alloc;
use alloc::vec::Vec;

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

/// HRESULTs DirectSound returns (dsound.h, winerror.h).
pub mod hr {
    pub const DS_OK: u32 = 0;
    pub const S_FALSE: u32 = 1;
    pub const DSERR_ALLOCATED: u32 = 0x8878_000A;
    pub const DSERR_CONTROLUNAVAIL: u32 = 0x8878_001E;
    pub const DSERR_INVALIDPARAM: u32 = 0x8007_0057;
    pub const DSERR_INVALIDCALL: u32 = 0x8878_0032;
    pub const DSERR_GENERIC: u32 = 0x8000_4005;
    pub const DSERR_PRIOLEVELNEEDED: u32 = 0x8878_0046;
    pub const DSERR_OUTOFMEMORY: u32 = 0x8007_000E;
    pub const DSERR_BADFORMAT: u32 = 0x8878_0064;
    pub const DSERR_UNSUPPORTED: u32 = 0x8000_4001;
    pub const DSERR_NODRIVER: u32 = 0x8878_0078;
    pub const DSERR_ALREADYINITIALIZED: u32 = 0x8878_0082;
    pub const DSERR_NOAGGREGATION: u32 = 0x8004_0110;
    pub const DSERR_UNINITIALIZED: u32 = 0x8878_00AA;
    pub const DSERR_NOINTERFACE: u32 = 0x8000_4002;
    pub const DSERR_BUFFERTOOSMALL: u32 = 0x8878_00B4;
    pub const DSERR_DS8_REQUIRED: u32 = 0x8878_00BE;
    pub const CLASS_E_CLASSNOTAVAILABLE: u32 = 0x8004_0111;
    pub const E_POINTER: u32 = 0x8000_4003;
}
/// DSBCAPS_* (buffer description and caps flags).
pub mod caps {
    pub const PRIMARYBUFFER: u32 = 0x1;
    pub const STATIC: u32 = 0x2;
    pub const LOCHARDWARE: u32 = 0x4;
    pub const LOCSOFTWARE: u32 = 0x8;
    pub const CTRL3D: u32 = 0x10;
    pub const CTRLFREQUENCY: u32 = 0x20;
    pub const CTRLPAN: u32 = 0x40;
    pub const CTRLVOLUME: u32 = 0x80;
    pub const CTRLPOSITIONNOTIFY: u32 = 0x100;
    pub const CTRLFX: u32 = 0x200;
    pub const STICKYFOCUS: u32 = 0x4000;
    pub const GLOBALFOCUS: u32 = 0x8000;
    pub const GETCURRENTPOSITION2: u32 = 0x1_0000;
    pub const MUTE3DATMAXDISTANCE: u32 = 0x2_0000;
    pub const LOCDEFER: u32 = 0x4_0000;
    pub const TRUEPLAYPOSITION: u32 = 0x8_0000;
    /// Flags a buffer description may carry.
    pub const VALID: u32 = 0xF_C3FF;
}
pub const DSBPLAY_LOOPING: u32 = 1;
pub const DSBSTATUS_PLAYING: u32 = 0x1;
pub const DSBSTATUS_LOOPING: u32 = 0x4;
pub const DSBSTATUS_LOCSOFTWARE: u32 = 0x10;
pub const DSBLOCK_FROMWRITECURSOR: u32 = 1;
pub const DSBLOCK_ENTIREBUFFER: u32 = 2;
pub const DSBPN_OFFSETSTOP: u32 = 0xFFFF_FFFF;
pub const DSSCL_NORMAL: u32 = 1;
pub const DSSCL_WRITEPRIMARY: u32 = 4;
pub const DSBSIZE_MIN: u32 = 4;
pub const DSBSIZE_MAX: u32 = 0x0FFF_FFFF;
pub const DSBFREQUENCY_MIN: u32 = 100;
pub const DSBFREQUENCY_MAX: u32 = 200_000;
pub const DSBVOLUME_MIN: i32 = -10_000;
pub const DSBPAN_RIGHT: i32 = 10_000;
/// The mix: what door.audio plays (DirectSound's default primary buffer is
/// 22.05 kHz 8-bit stereo; Doors mixes at 48 kHz 16-bit stereo and
/// reports the format the application last set on the primary buffer).
pub const MIX_RATE: u32 = 48_000;

pub const WAVE_FORMAT_PCM: u16 = 1;
pub const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
pub const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
/// A buffer's sample format (WAVEFORMATEX / WAVEFORMATEXTENSIBLE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub tag: u16,
    pub channels: u16,
    pub rate: u32,
    pub bits: u16,
    pub block: u16,
}
impl Format {
    pub const DEFAULT_PRIMARY: Self = Self {
        tag: WAVE_FORMAT_PCM,
        channels: 2,
        rate: 22_050,
        bits: 8,
        block: 2,
    };
    /// The format in `b` (at least WAVEFORMATEX's 18 bytes, or the 16 of
    /// a PCMWAVEFORMAT); None unless PCM 8/16/24/32-bit or 32-bit float,
    /// one or two channels, a consistent block align and a rate DirectSound
    /// accepts.
    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < 16 {
            return None;
        }
        let u16_at = |at: usize| u16::from_le_bytes([b[at], b[at + 1]]);
        let mut tag = u16_at(0);
        let channels = u16_at(2);
        let rate = u32::from_le_bytes(b[4..8].try_into().ok()?);
        let block = u16_at(12);
        let bits = u16_at(14);
        if tag == WAVE_FORMAT_EXTENSIBLE {
            // cbSize 22; the sub-format GUID's first word is the tag.
            if b.len() < 40 || u16_at(16) < 22 {
                return None;
            }
            tag = u16_at(24);
        }
        let valid_bits = match tag {
            WAVE_FORMAT_PCM => matches!(bits, 8 | 16 | 24 | 32),
            WAVE_FORMAT_IEEE_FLOAT => bits == 32,
            _ => false,
        };
        let f = Self {
            tag,
            channels,
            rate,
            bits,
            block,
        };
        (valid_bits
            && matches!(channels, 1 | 2)
            && (DSBFREQUENCY_MIN..=DSBFREQUENCY_MAX).contains(&rate)
            && u32::from(block) == u32::from(channels) * u32::from(bits) / 8)
            .then_some(f)
    }
    /// What CreateSoundBuffer answers for a format `parse` refused, as
    /// Windows does: DSERR_UNSUPPORTED for anything but PCM and float (a
    /// compressed format needs ACM), DSERR_INVALIDPARAM for a malformed
    /// PCM or float one (more than two channels without
    /// WAVEFORMATEXTENSIBLE, an inconsistent block align, odd sizes).
    pub fn refusal(b: &[u8]) -> u32 {
        let tag = u16::from_le_bytes([b[0], b[1]]);
        let tag = if tag == WAVE_FORMAT_EXTENSIBLE && b.len() >= 26 {
            u16::from_le_bytes([b[24], b[25]])
        } else {
            tag
        };
        if matches!(tag, WAVE_FORMAT_PCM | WAVE_FORMAT_IEEE_FLOAT) {
            hr::DSERR_INVALIDPARAM
        } else {
            hr::DSERR_UNSUPPORTED
        }
    }
    /// The 18-byte WAVEFORMATEX GetFormat writes.
    pub fn encode(&self) -> [u8; 18] {
        let mut b = [0u8; 18];
        b[0..2].copy_from_slice(&self.tag.to_le_bytes());
        b[2..4].copy_from_slice(&self.channels.to_le_bytes());
        b[4..8].copy_from_slice(&self.rate.to_le_bytes());
        b[8..12].copy_from_slice(&(self.rate * u32::from(self.block)).to_le_bytes());
        b[12..14].copy_from_slice(&self.block.to_le_bytes());
        b[14..16].copy_from_slice(&self.bits.to_le_bytes());
        b
    }
    /// One channel's sample of frame `frame` in `data`, as -1.0..1.0
    /// scaled to i32 Q15 (so 16-bit samples are unchanged).
    fn sample(&self, data: &[u8], frame: usize, channel: usize) -> i32 {
        let at = frame * usize::from(self.block) + channel * usize::from(self.bits / 8);
        let Some(s) = data.get(at..at + usize::from(self.bits / 8)) else {
            return 0;
        };
        match (self.tag, self.bits) {
            (WAVE_FORMAT_PCM, 8) => (i32::from(s[0]) - 128) << 8,
            (WAVE_FORMAT_PCM, 16) => i32::from(i16::from_le_bytes([s[0], s[1]])),
            (WAVE_FORMAT_PCM, 24) => i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 16,
            (WAVE_FORMAT_PCM, 32) => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) >> 16,
            _ => {
                let v = f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                (v.clamp(-1.0, 1.0) * 32767.0) as i32
            }
        }
    }
    /// Frames in `bytes`.
    pub fn frames(&self, bytes: usize) -> usize {
        bytes / usize::from(self.block.max(1))
    }
}

/// Q16 gain of a volume in hundredths of a dB (DSBVOLUME_MIN..=0):
/// 10^(v/2000); -10000 (100 dB down) is silence.
pub fn gain(volume: i32) -> u32 {
    if volume <= DSBVOLUME_MIN {
        return 0;
    }
    // 10^(v/2000) = 2^(v * log2(10) / 2000), evaluated as 2^int * 2^frac.
    let exponent = volume as f64 * core::f64::consts::LOG2_10 / 2000.0;
    // floor and round without std (no_std has no f64::floor).
    let truncated = exponent as i64;
    let whole = if (truncated as f64) > exponent {
        truncated - 1
    } else {
        truncated
    } as f64;
    let frac = exponent - whole;
    // 2^frac for 0 <= frac < 1 by a short polynomial (error < 1e-4).
    let pow_frac = 1.0
        + frac
            * (core::f64::consts::LN_2
                + frac * (0.240_226_51 + frac * (0.055_504_11 + frac * 0.009_618_13)));
    let value = pow_frac * 65536.0 / (1u64 << (-whole as u32).min(63)) as f64;
    (value + 0.5) as u32
}
/// Left and right Q16 gains of a volume and a pan (DSBPAN_LEFT..=RIGHT):
/// the pan attenuates the opposite side only, as DirectSound does.
pub fn gains(volume: i32, pan: i32) -> (u32, u32) {
    let v = gain(volume);
    let left = if pan > 0 { gain(-pan) } else { 65536 };
    let right = if pan < 0 { gain(pan) } else { 65536 };
    (
        ((u64::from(v) * u64::from(left)) >> 16) as u32,
        ((u64::from(v) * u64::from(right)) >> 16) as u32,
    )
}

/// One secondary buffer's playback state, as the mixer sees it.
#[derive(Clone, Debug)]
pub struct Voice {
    pub format: Format,
    /// Read position in source frames, Q32.32.
    pub position: u64,
    /// Source frames per mixed frame, Q32.32 (frequency / MIX_RATE).
    pub step: u64,
    pub left: u32,
    pub right: u32,
    pub looping: bool,
    pub playing: bool,
}
impl Voice {
    pub fn new(format: Format) -> Self {
        let mut v = Self {
            format,
            position: 0,
            step: 0,
            left: 65536,
            right: 65536,
            looping: false,
            playing: false,
        };
        v.set_frequency(format.rate);
        v
    }
    pub fn set_frequency(&mut self, rate: u32) {
        self.step = (u64::from(rate) << 32) / u64::from(MIX_RATE);
    }
    /// The read position in bytes.
    pub fn byte_position(&self) -> usize {
        (self.position >> 32) as usize * usize::from(self.format.block)
    }
    pub fn set_byte_position(&mut self, bytes: usize) {
        self.position = (self.format.frames(bytes) as u64) << 32;
    }
    /// Adds `out.len() / 2` stereo frames of this voice (linearly
    /// interpolated) to `out` (interleaved Q15 sums). A one-shot voice
    /// stops at the buffer's end; answers whether it stopped.
    pub fn mix(&mut self, data: &[u8], out: &mut [i32]) -> bool {
        let frames = self.format.frames(data.len());
        if !self.playing || frames == 0 {
            return false;
        }
        let stereo = self.format.channels == 2;
        for pair in out.chunks_exact_mut(2) {
            let index = (self.position >> 32) as usize;
            if index >= frames {
                if self.looping {
                    self.position -= (frames as u64) << 32;
                } else {
                    self.position = (frames as u64) << 32;
                    self.playing = false;
                    return true;
                }
            }
            let index = (self.position >> 32) as usize;
            let next = if index + 1 < frames {
                index + 1
            } else if self.looping {
                0
            } else {
                index
            };
            let t = ((self.position >> 16) & 0xFFFF) as i64;
            let channel = |c: usize, frame: usize| {
                self.format.sample(data, frame, if stereo { c } else { 0 })
            };
            for (c, slot) in pair.iter_mut().enumerate() {
                let a = i64::from(channel(c, index));
                let b = i64::from(channel(c, next));
                let s = a + (((b - a) * t) >> 16);
                let g = i64::from(if c == 0 { self.left } else { self.right });
                *slot += ((s * g) >> 16) as i32;
            }
            self.position += self.step;
        }
        false
    }
}
/// Mixed Q15 sums as 16-bit little-endian samples, clipped.
pub fn to_pcm16(mix: &[i32], out: &mut [u8]) {
    for (s, o) in mix.iter().zip(out.chunks_exact_mut(2)) {
        o.copy_from_slice(&((*s).clamp(-32768, 32767) as i16).to_le_bytes());
    }
}
/// Notification offsets the play cursor passed moving from `from` to `to`
/// (bytes, in a buffer of `size`; wrapping when `to < from`), in order.
pub fn crossed(positions: &[(u32, u64)], from: usize, to: usize, size: usize) -> Vec<u64> {
    let passed = |offset: usize| {
        if from == to {
            false
        } else if from < to {
            offset >= from && offset < to
        } else {
            offset >= from || offset < to
        }
    };
    let mut hits: Vec<(usize, u64)> = positions
        .iter()
        .filter(|(o, _)| *o != DSBPN_OFFSETSTOP && (*o as usize) < size && passed(*o as usize))
        .map(|(o, e)| {
            (
                if (*o as usize) >= from {
                    *o as usize - from
                } else {
                    *o as usize + size - from
                },
                *e,
            )
        })
        .collect();
    hits.sort_by_key(|(d, _)| *d);
    hits.into_iter().map(|(_, e)| e).collect()
}
/// Lock's two regions for `bytes` at `offset` of a `size` buffer:
/// (offset, length) of each; the second is (0, 0) without wrap.
pub fn lock_regions(
    offset: usize,
    bytes: usize,
    size: usize,
) -> Option<((usize, usize), (usize, usize))> {
    if offset >= size || bytes == 0 || bytes > size {
        return None;
    }
    let first = bytes.min(size - offset);
    Some(((offset, first), (0, bytes - first)))
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use alloc::vec;

    fn pcm(channels: u16, rate: u32, bits: u16) -> Vec<u8> {
        let block = channels * bits / 8;
        let mut b = vec![0u8; 18];
        b[0..2].copy_from_slice(&WAVE_FORMAT_PCM.to_le_bytes());
        b[2..4].copy_from_slice(&channels.to_le_bytes());
        b[4..8].copy_from_slice(&rate.to_le_bytes());
        b[8..12].copy_from_slice(&(rate * u32::from(block)).to_le_bytes());
        b[12..14].copy_from_slice(&block.to_le_bytes());
        b[14..16].copy_from_slice(&bits.to_le_bytes());
        b
    }

    #[test]
    fn formats_parse_like_directsound() {
        let f = Format::parse(&pcm(2, 44_100, 16)).unwrap();
        assert_eq!((f.channels, f.rate, f.bits, f.block), (2, 44_100, 16, 4));
        assert_eq!(Format::parse(&f.encode()), Some(f));
        assert!(Format::parse(&pcm(1, 8_000, 8)).is_some());
        assert!(
            Format::parse(&pcm(3, 44_100, 16)).is_none(),
            "more than two channels"
        );
        assert!(Format::parse(&pcm(2, 50, 16)).is_none(), "below 100 Hz");
        assert!(Format::parse(&pcm(2, 44_100, 12)).is_none());
        let mut bad = pcm(2, 44_100, 16);
        bad[12] = 3;
        assert!(Format::parse(&bad).is_none(), "inconsistent block align");
        // WAVEFORMATEXTENSIBLE with the float sub-format.
        let mut ext = pcm(2, 48_000, 32);
        ext[0..2].copy_from_slice(&WAVE_FORMAT_EXTENSIBLE.to_le_bytes());
        ext[16..18].copy_from_slice(&22u16.to_le_bytes());
        ext.resize(40, 0);
        ext[24..26].copy_from_slice(&WAVE_FORMAT_IEEE_FLOAT.to_le_bytes());
        assert_eq!(Format::parse(&ext).unwrap().tag, WAVE_FORMAT_IEEE_FLOAT);
        // Refusals as Windows answers them.
        assert_eq!(Format::refusal(&pcm(3, 44_100, 16)), hr::DSERR_INVALIDPARAM);
        let mut adpcm = pcm(1, 44_100, 4);
        adpcm[0] = 2;
        assert_eq!(Format::refusal(&adpcm), hr::DSERR_UNSUPPORTED);
    }

    #[test]
    fn volume_and_pan_follow_the_decibel_law() {
        assert_eq!(gain(0), 65536);
        assert_eq!(gain(DSBVOLUME_MIN), 0);
        // -6.02 dB halves, -20 dB is a tenth.
        assert!((gain(-602) as i64 - 32768).abs() < 40);
        assert!((gain(-2000) as i64 - 6554).abs() < 10);
        assert_eq!(gains(0, 0), (65536, 65536));
        let (l, r) = gains(0, -2000);
        assert_eq!(l, 65536);
        assert!((r as i64 - 6554).abs() < 10);
        let (l, r) = gains(-602, DSBPAN_RIGHT);
        assert_eq!(l, 0);
        assert!((r as i64 - 32768).abs() < 40);
    }

    #[test]
    fn voices_resample_loop_and_stop() {
        // A 16-bit mono ramp at 24 kHz plays at half speed in the 48 kHz mix.
        let f = Format::parse(&pcm(1, 24_000, 16)).unwrap();
        let data: Vec<u8> = (0..4i16).flat_map(|s| (s * 1000).to_le_bytes()).collect();
        let mut v = Voice::new(f);
        v.playing = true;
        let mut out = vec![0i32; 10 * 2];
        assert!(
            v.mix(&data, &mut out),
            "eight mixed frames use up four source frames"
        );
        // Interpolated between frames, same on both sides (mono).
        assert_eq!(&out[..6], &[0, 0, 500, 500, 1000, 1000]);
        assert!(!v.playing);
        // Looping wraps to the start.
        let mut v = Voice::new(f);
        v.playing = true;
        v.looping = true;
        let mut out = vec![0i32; 20 * 2];
        assert!(!v.mix(&data, &mut out));
        assert!(v.playing);
        assert_eq!(out[16], 0, "frame 8 of the mix is source frame 0 again");
        // Mixing sums and clips at 16 bits.
        let mut pcm16 = vec![0u8; 4];
        to_pcm16(&[40_000, -40_000], &mut pcm16);
        assert_eq!(pcm16, [0xFF, 0x7F, 0x00, 0x80]);
    }

    #[test]
    fn notifications_fire_in_play_order_across_the_wrap() {
        let positions = [(100, 1), (900, 2), (0, 3), (DSBPN_OFFSETSTOP, 4)];
        assert_eq!(crossed(&positions, 50, 150, 1000), vec![1]);
        assert_eq!(crossed(&positions, 850, 120, 1000), vec![2, 3, 1]);
        assert!(crossed(&positions, 150, 150, 1000).is_empty());
        assert_eq!(lock_regions(900, 300, 1000), Some(((900, 100), (0, 200))));
        assert_eq!(lock_regions(0, 1000, 1000), Some(((0, 1000), (0, 0))));
        assert_eq!(lock_regions(1000, 1, 1000), None);
    }
}
