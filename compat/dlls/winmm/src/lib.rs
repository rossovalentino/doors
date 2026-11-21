#![no_std]
//! Original Windows Multimedia (winmm.dll) over door.audio: waveOut
//! playback with callbacks, PlaySound/sndPlaySound (system sound aliases,
//! WAV files, memory and resources), multimedia timers and time functions,
//! the device-count and error-text functions, capture (waveIn), MIDI
//! output through a software synthesizer (midiOut and midiStream), the
//! mixer API over the master volume, and MCI command strings for the
//! waveaudio and sequencer devices. As on a Windows PC without MIDI input
//! or auxiliary hardware, midiIn and aux report no device.
//!
//! This module is the host-testable part: WAVEFORMATEX and RIFF/WAVE
//! parsing, sample conversion to what door.audio streams take, the system
//! sound alias table, the MMSYSERR and MIDIERR texts, the synthesizer,
//! Standard MIDI Files and the MCI command parser.
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
pub mod mci;
pub mod smf;
pub mod synth;

/// MMSYSERR_* and WAVERR_* results.
pub mod mmsyserr {
    pub const NOERROR: u32 = 0;
    pub const ERROR: u32 = 1;
    pub const BADDEVICEID: u32 = 2;
    pub const NOTENABLED: u32 = 3;
    pub const ALLOCATED: u32 = 4;
    pub const INVALHANDLE: u32 = 5;
    pub const NODRIVER: u32 = 6;
    pub const NOMEM: u32 = 7;
    pub const NOTSUPPORTED: u32 = 8;
    pub const BADERRNUM: u32 = 9;
    pub const INVALFLAG: u32 = 10;
    pub const INVALPARAM: u32 = 11;
    pub const HANDLEBUSY: u32 = 12;
    pub const BADFORMAT: u32 = 32;
    pub const STILLPLAYING: u32 = 33;
    pub const UNPREPARED: u32 = 34;
    pub const SYNC: u32 = 35;
}

/// The texts waveOutGetErrorText returns (read from Windows 11 26H2).
pub fn error_text(code: u32) -> Option<&'static str> {
    Some(match code {
        0 => "The specified command was carried out.",
        1 => "Undefined external error.",
        2 => "A device ID has been used that is out of range for your system.",
        3 => "The driver was not enabled.",
        4 => "The specified device is already in use.  Wait until it is free, and then try again.",
        5 => "The specified device handle is invalid.",
        6 => "There is no driver installed on your system.",
        7 => {
            "There is not enough memory available for this task.  Quit one or more applications to increase available memory, and then try again."
        }
        8 => {
            "This function is not supported.  Use the Capabilities function to determine which functions and messages the driver supports."
        }
        9 => "An error number was specified that is not defined in the system.",
        10 => "An invalid flag was passed to a system function.",
        11 => "An invalid parameter was passed to a system function.",
        12 => "Handle being used simultaneously on another thread (eg callback).",
        32 => {
            "The specified format is not supported or cannot be translated.  Use the Capabilities function to determine the supported formats."
        }
        33 => {
            "Cannot perform this operation while media data is still playing.  Reset the device, or wait until the data is finished playing."
        }
        34 => {
            "The wave header was not prepared.  Use the Prepare function to prepare the header, and then try again."
        }
        35 => {
            "Cannot open the device without using the WAVE_ALLOWSYNC flag.  Use the flag, and then try again."
        }
        // MIDIERR_* (midiOutGetErrorText, read from Windows 11).
        64 => {
            "The MIDI header was not prepared.  Use the Prepare function to prepare the header, and then try again."
        }
        65 => {
            "Cannot perform this operation while media data is still playing.  Reset the device, or wait until the data is finished playing."
        }
        66 => {
            "A MIDI map was not found.  There may be a problem with the driver, or the MIDIMAP.CFG file may be corrupt or missing."
        }
        67 => {
            "The port is transmitting data to the device.  Wait until the data has been transmitted, and then try again."
        }
        68 => {
            "The current MIDI Mapper setup refers to a MIDI device that is not installed on the system.  Use MIDI Mapper to edit the setup."
        }
        69 => {
            "The current MIDI setup is damaged.  Copy the original MIDIMAP.CFG file to the Doors SYSTEM directory, and then try again."
        }
        70 => {
            "A MIDI call was made which is invalid with the current open mode. Reopen the device with the correct mode."
        }
        71 => "Driver condition - do not callback this input event",
        _ => return None,
    })
}

/// WAVE_FORMAT_* tags.
pub const WAVE_FORMAT_PCM: u16 = 1;
pub const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
pub const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;
/// KSDATAFORMAT_SUBTYPE_PCM and _IEEE_FLOAT share this tail after their
/// first two bytes (the format tag).
const SUBTYPE_TAIL: [u8; 14] = [
    0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

/// The sample encoding of a wave format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// Integer PCM: 8-bit unsigned, 16/24/32-bit signed.
    Pcm,
    /// 32-bit IEEE float.
    Float,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wave {
    pub encoding: Encoding,
    pub channels: u16,
    pub rate: u32,
    pub bits: u16,
    pub block_align: u16,
}
impl Wave {
    /// A WAVEFORMATEX (18 bytes plus `cbSize`), as waveOutOpen and the fmt
    /// chunk carry it. None for formats door.audio cannot play: other tags,
    /// more than two channels, rates outside 8..=192 kHz, inconsistent
    /// block alignment.
    pub fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < 16 {
            return None;
        }
        let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let mut tag = u16_at(0);
        let channels = u16_at(2);
        let rate = u32_at(4);
        let average = u32_at(8);
        let block_align = u16_at(12);
        let bits = u16_at(14);
        if tag == WAVE_FORMAT_EXTENSIBLE {
            // cbSize >= 22: wValidBitsPerSample, dwChannelMask, SubFormat.
            if b.len() < 40 || u16_at(16) < 22 || b[26..40] != SUBTYPE_TAIL {
                return None;
            }
            tag = u16_at(24);
        }
        let encoding = match (tag, bits) {
            (WAVE_FORMAT_PCM, 8 | 16 | 24 | 32) => Encoding::Pcm,
            (WAVE_FORMAT_IEEE_FLOAT, 32) => Encoding::Float,
            _ => return None,
        };
        if !(1..=2).contains(&channels)
            || !(8000..=192_000).contains(&rate)
            || block_align != channels * bits / 8
            || average != rate * u32::from(block_align)
        {
            return None;
        }
        Some(Self {
            encoding,
            channels,
            rate,
            bits,
            block_align,
        })
    }
    /// door.audio's stream format: float becomes 16-bit integer.
    pub fn stream_bits(&self) -> u8 {
        match self.encoding {
            Encoding::Float => 16,
            Encoding::Pcm => self.bits as u8,
        }
    }
    /// Bytes per frame in the stream format.
    pub fn stream_frame(&self) -> usize {
        usize::from(self.channels) * usize::from(self.stream_bits()) / 8
    }
    /// Caller bytes to stream bytes (whole frames only).
    pub fn convert(&self, data: &[u8]) -> Vec<u8> {
        let whole = data.len() / usize::from(self.block_align) * usize::from(self.block_align);
        let mut out = alloc::vec![0u8; whole / usize::from(self.block_align) * self.stream_frame()];
        self.convert_into(&data[..whole], &mut out);
        out
    }
    /// Converts whole samples of `data` into `out`; the stream bytes written.
    pub fn convert_into(&self, data: &[u8], out: &mut [u8]) -> usize {
        match self.encoding {
            Encoding::Pcm => {
                let n = data.len().min(out.len());
                out[..n].copy_from_slice(&data[..n]);
                n
            }
            Encoding::Float => {
                let mut n = 0;
                for (s, o) in data.chunks_exact(4).zip(out.chunks_exact_mut(2)) {
                    let v = f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                    let v = if v.is_nan() { 0.0 } else { v.clamp(-1.0, 1.0) };
                    o.copy_from_slice(&((v * 32767.0) as i16).to_le_bytes());
                    n += 2;
                }
                n
            }
        }
    }
}

/// A RIFF/WAVE image: its format and the `data` chunk's byte range.
pub fn parse_wav(b: &[u8]) -> Option<(Wave, core::ops::Range<usize>)> {
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return None;
    }
    let end = (u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize)
        .saturating_add(8)
        .min(b.len());
    let (mut at, mut format, mut data) = (12usize, None, None);
    while at + 8 <= end {
        let length = u32::from_le_bytes(b[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = at + 8;
        let stop = body.checked_add(length)?.min(end);
        match &b[at..at + 4] {
            b"fmt " => format = Wave::parse(&b[body..stop]),
            b"data" => data = Some(body..stop),
            _ => (),
        }
        at = body.checked_add(length)?.checked_add(length & 1)?;
    }
    Some((format?, data?))
}

/// door.audio's capture format: 48 kHz stereo 16-bit (4-byte frames).
pub const CAPTURE_RATE: u32 = 48_000;
/// Captured PCM to a waveIn format: linear resampling from 48 kHz, a mono
/// downmix (the channels' mean) and the sample encoding (8-bit unsigned,
/// 16/24/32-bit signed or 32-bit float).
pub struct Recorder {
    wave: Wave,
    /// Source frames per output frame, 16.16 fixed point.
    step: u64,
    /// Position past `prev`, 16.16 (below 1.0 while an output is due).
    position: u64,
    prev: [i64; 2],
    primed: bool,
}
impl Recorder {
    pub fn new(wave: Wave) -> Self {
        Self {
            wave,
            step: (u64::from(CAPTURE_RATE) << 16) / u64::from(wave.rate),
            position: 0,
            prev: [0; 2],
            primed: false,
        }
    }
    /// Converts the whole frames of `captured`, appending to `out`.
    pub fn push(&mut self, captured: &[u8], out: &mut Vec<u8>) {
        for frame in captured.chunks_exact(4) {
            let next = [
                i64::from(i16::from_le_bytes([frame[0], frame[1]])),
                i64::from(i16::from_le_bytes([frame[2], frame[3]])),
            ];
            if !self.primed {
                self.prev = next;
                self.primed = true;
                continue;
            }
            while self.position < 1 << 16 {
                let t = self.position as i64;
                let l = self.prev[0] + (((next[0] - self.prev[0]) * t) >> 16);
                let r = self.prev[1] + (((next[1] - self.prev[1]) * t) >> 16);
                self.emit(l, r, out);
                self.position += self.step;
            }
            self.position -= 1 << 16;
            self.prev = next;
        }
    }
    fn emit(&self, l: i64, r: i64, out: &mut Vec<u8>) {
        let mono = [(l + r) / 2];
        let stereo = [l, r];
        let samples: &[i64] = if self.wave.channels == 1 {
            &mono
        } else {
            &stereo
        };
        for &v in samples {
            let v = v.clamp(-32768, 32767) as i32;
            match (self.wave.encoding, self.wave.bits) {
                (Encoding::Pcm, 8) => out.push(((v >> 8) + 128) as u8),
                (Encoding::Pcm, 16) => out.extend_from_slice(&(v as i16).to_le_bytes()),
                (Encoding::Pcm, 24) => out.extend_from_slice(&(v << 8).to_le_bytes()[..3]),
                (Encoding::Pcm, _) => out.extend_from_slice(&(v << 16).to_le_bytes()),
                (Encoding::Float, _) => out.extend_from_slice(&(v as f32 / 32768.0).to_le_bytes()),
            }
        }
    }
}

/// MMTIME types.
pub const TIME_MS: u32 = 1;
pub const TIME_SAMPLES: u32 = 2;
pub const TIME_BYTES: u32 = 4;
/// A position of `frames` in the type asked for. Like Windows 11, only
/// samples and bytes are kept; every other type (TIME_MS included) is
/// answered in bytes, with wType saying so.
pub fn position(frames: u64, wave: &Wave, kind: u32) -> (u32, u32) {
    match kind {
        TIME_SAMPLES => (TIME_SAMPLES, frames as u32),
        _ => (TIME_BYTES, (frames * u64::from(wave.block_align)) as u32),
    }
}

/// The system sound a PlaySound alias names (door.audio's sound events),
/// as the default Windows sound scheme assigns them. `Some(None)`: a known
/// alias with no sound assigned (plays nothing).
pub fn alias(name: &str) -> Option<Option<u32>> {
    use door_service_protocol::audio::sound;
    let name = name.to_ascii_lowercase();
    Some(match name.as_str() {
        ".default" | "systemdefault" => Some(sound::DEFAULT_BEEP),
        "systemasterisk" => Some(sound::ASTERISK),
        "systemexclamation" => Some(sound::EXCLAMATION),
        "systemhand" => Some(sound::CRITICAL_STOP),
        "systemnotification" => Some(sound::SYSTEM_NOTIFICATION),
        "notification.default" => Some(sound::NOTIFICATION),
        "notification.reminder" => Some(sound::REMINDER),
        "notification.mail" | "mailbeep" => Some(sound::MAIL),
        "notification.im" => Some(sound::MESSAGING),
        "notification.looping.alarm" => Some(sound::ALARM),
        "windowsunlock" => Some(sound::UNLOCK),
        "windowsuac" => Some(sound::UAC),
        "deviceconnect" => Some(sound::DEVICE_CONNECT),
        "devicedisconnect" => Some(sound::DEVICE_DISCONNECT),
        "devicefail" => Some(sound::DEVICE_FAIL),
        "messagenudge" => Some(sound::NUDGE),
        // In the scheme, silent by default.
        "systemquestion" | "systemstart" | "systemexit" | "systemwelcome" | "windowslogon"
        | "windowslogoff" | "close" | "open" | "minimize" | "maximize" | "restoredown"
        | "restoreup" | "menucommand" | "menupopup" | "emptyrecyclebin" => None,
        _ => return None,
    })
}
/// The alias of a SND_ALIAS_ID value (`sndAlias('S', c)`).
pub fn alias_id(id: u32) -> Option<&'static str> {
    Some(match id {
        0x2A53 => "SystemAsterisk",
        0x3F53 => "SystemQuestion",
        0x4853 => "SystemHand",
        0x4553 => "SystemExit",
        0x5353 => "SystemStart",
        0x5753 => "SystemWelcome",
        0x2153 => "SystemExclamation",
        0x4453 => "SystemDefault",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    fn format(tag: u16, channels: u16, rate: u32, bits: u16) -> Vec<u8> {
        let align = channels * bits / 8;
        let mut b = Vec::new();
        b.extend_from_slice(&tag.to_le_bytes());
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * u32::from(align)).to_le_bytes());
        b.extend_from_slice(&align.to_le_bytes());
        b.extend_from_slice(&bits.to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        b
    }
    #[test]
    fn formats_parse_and_refuse() {
        let w = Wave::parse(&format(1, 2, 44_100, 16)).unwrap();
        assert_eq!(
            (w.encoding, w.block_align, w.stream_bits()),
            (Encoding::Pcm, 4, 16)
        );
        let f = Wave::parse(&format(3, 1, 48_000, 32)).unwrap();
        assert_eq!(
            (f.encoding, f.stream_bits(), f.stream_frame()),
            (Encoding::Float, 16, 2)
        );
        assert!(Wave::parse(&format(1, 6, 48_000, 16)).is_none());
        assert!(Wave::parse(&format(2, 2, 44_100, 4)).is_none());
        assert!(Wave::parse(&format(1, 2, 4_000, 16)).is_none());
        let mut bad = format(1, 2, 44_100, 16);
        bad[12] = 3;
        assert!(Wave::parse(&bad).is_none());
        // WAVE_FORMAT_EXTENSIBLE with the PCM subtype.
        let mut ext = format(WAVE_FORMAT_EXTENSIBLE, 2, 48_000, 16);
        ext[16..18].copy_from_slice(&22u16.to_le_bytes());
        ext.extend_from_slice(&16u16.to_le_bytes());
        ext.extend_from_slice(&3u32.to_le_bytes());
        ext.extend_from_slice(&[1, 0]);
        ext.extend_from_slice(&SUBTYPE_TAIL);
        assert_eq!(Wave::parse(&ext).map(|w| w.encoding), Some(Encoding::Pcm));
    }
    #[test]
    fn captured_audio_converts_to_the_requested_format() {
        let frames = |n: usize, l: i16, r: i16| -> Vec<u8> {
            (0..n)
                .flat_map(|_| [l.to_le_bytes(), r.to_le_bytes()].concat())
                .collect()
        };
        // 48 kHz stereo to 44.1 kHz mono 16-bit: a second in, a second out.
        let mut rec = Recorder::new(Wave::parse(&format(1, 1, 44_100, 16)).unwrap());
        let mut out = Vec::new();
        rec.push(&frames(48_000, 1000, 3000), &mut out);
        let n = out.len() / 2;
        assert!((44_090..=44_100).contains(&n), "{n}");
        assert!(
            out.chunks_exact(2)
                .all(|s| i16::from_le_bytes([s[0], s[1]]) == 2000)
        );
        // Same rate stereo passes samples through; 8-bit is unsigned.
        let mut same = Recorder::new(Wave::parse(&format(1, 2, 48_000, 16)).unwrap());
        out.clear();
        same.push(&frames(10, -5, 7), &mut out);
        assert_eq!(out.len(), 9 * 4, "the first frame primes the interpolation");
        assert_eq!(
            &out[..4],
            &[(-5i16).to_le_bytes(), 7i16.to_le_bytes()].concat()[..]
        );
        let mut byte = Recorder::new(Wave::parse(&format(1, 1, 8_000, 8)).unwrap());
        out.clear();
        byte.push(&frames(600, i16::MIN, i16::MIN), &mut out);
        assert!(!out.is_empty() && out.iter().all(|b| *b == 0));
        let mut float = Recorder::new(Wave::parse(&format(3, 1, 48_000, 32)).unwrap());
        out.clear();
        float.push(&frames(3, 16384, 16384), &mut out);
        assert_eq!(f32::from_le_bytes(out[..4].try_into().unwrap()), 0.5);
    }
    #[test]
    fn float_converts_and_wav_files_parse() {
        let f = Wave::parse(&format(3, 1, 48_000, 32)).unwrap();
        let samples: Vec<u8> = [0.5f32, -1.5, f32::NAN]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(
            f.convert(&samples),
            [16383i16, -32767, 0]
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>()
        );
        let mut wav = b"RIFF\0\0\0\0WAVE".to_vec();
        let fmt = format(1, 1, 8_000, 8);
        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
        wav.extend_from_slice(&fmt);
        wav.extend_from_slice(b"LIST\x03\0\0\0abc\0");
        wav.extend_from_slice(b"data\x04\0\0\0\x80\x90\xa0\xb0");
        let size = (wav.len() - 8) as u32;
        wav[4..8].copy_from_slice(&size.to_le_bytes());
        let (w, data) = parse_wav(&wav).unwrap();
        assert_eq!(
            (w.rate, w.bits, &wav[data]),
            (8_000, 8, &[0x80, 0x90, 0xa0, 0xb0][..])
        );
        assert!(parse_wav(b"RIFX....WAVE").is_none());
        assert_eq!(
            position(
                48_000,
                &Wave::parse(&format(1, 2, 48_000, 16)).unwrap(),
                TIME_MS
            ),
            (TIME_BYTES, 192_000)
        );
        assert_eq!(
            position(10, &Wave::parse(&format(1, 2, 48_000, 16)).unwrap(), 8),
            (TIME_BYTES, 40)
        );
        let _ = vec![0u8];
    }
    #[test]
    fn aliases_and_texts() {
        use door_service_protocol::audio::sound;
        assert_eq!(alias("SystemAsterisk"), Some(Some(sound::ASTERISK)));
        assert_eq!(alias("SystemQuestion"), Some(None));
        assert_eq!(alias("NoSuchSound"), None);
        assert_eq!(alias_id(0x2153), Some("SystemExclamation"));
        assert!(
            error_text(32)
                .unwrap()
                .starts_with("The specified format is not supported")
        );
        assert!(error_text(99).is_none());
    }
}
