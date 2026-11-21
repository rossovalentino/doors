#![no_std]
//! Original Core Audio (MMDevApi.dll) over door.audio: the
//! MMDeviceEnumerator class, the Doors speaker and microphone as audio
//! endpoints (IMMDevice, IMMEndpoint, IPropertyStore), and what an
//! endpoint activates: IAudioClient in shared mode with IAudioRenderClient,
//! IAudioCaptureClient, IAudioClock and ISimpleAudioVolume, and
//! IAudioEndpointVolume. Windows splits these between MMDevApi.dll and
//! AudioSes.dll; programs reach both only through COM, so Doors serves them
//! from one DLL.
//!
//! This module is the host-testable part: result codes, the mix format,
//! shared-mode format support and buffer sizes as Windows 11 answers them
//! (measured with `tests/fixtures/windows-sound/wasapiprobe.c`), and the
//! endpoint volume curve. The COM objects are in `guest`.
//!
//! The same sources build the x64 and the x86 DLL (P17): Windows-facing
//! exports take pointer-width parameters, widened at entry; a pointer or
//! handle result is narrowed from the body's 64-bit value (an immediately
//! called closure, so the body's own `return`s keep their meaning).
#![allow(clippy::redundant_closure_call)]
extern crate alloc;

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

/// HRESULTs (winerror.h, audioclient.h).
pub mod hr {
    pub const S_OK: u32 = 0;
    pub const S_FALSE: u32 = 1;
    pub const E_NOTIMPL: u32 = 0x8000_4001;
    pub const E_NOINTERFACE: u32 = 0x8000_4002;
    pub const E_POINTER: u32 = 0x8000_4003;
    pub const E_FAIL: u32 = 0x8000_4005;
    pub const E_ACCESSDENIED: u32 = 0x8007_0005;
    pub const E_OUTOFMEMORY: u32 = 0x8007_000E;
    pub const E_INVALIDARG: u32 = 0x8007_0057;
    /// HRESULT_FROM_WIN32(ERROR_NOT_FOUND).
    pub const E_NOTFOUND: u32 = 0x8007_0490;
    pub const CLASS_E_NOAGGREGATION: u32 = 0x8004_0110;
    pub const CLASS_E_CLASSNOTAVAILABLE: u32 = 0x8004_0111;
    pub const AUDCLNT_E_NOT_INITIALIZED: u32 = 0x8889_0001;
    pub const AUDCLNT_E_ALREADY_INITIALIZED: u32 = 0x8889_0002;
    pub const AUDCLNT_E_WRONG_ENDPOINT_TYPE: u32 = 0x8889_0003;
    pub const AUDCLNT_E_DEVICE_INVALIDATED: u32 = 0x8889_0004;
    pub const AUDCLNT_E_NOT_STOPPED: u32 = 0x8889_0005;
    pub const AUDCLNT_E_BUFFER_TOO_LARGE: u32 = 0x8889_0006;
    pub const AUDCLNT_E_OUT_OF_ORDER: u32 = 0x8889_0007;
    pub const AUDCLNT_E_UNSUPPORTED_FORMAT: u32 = 0x8889_0008;
    pub const AUDCLNT_E_INVALID_SIZE: u32 = 0x8889_0009;
    pub const AUDCLNT_E_BUFFER_OPERATION_PENDING: u32 = 0x8889_000B;
    pub const AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED: u32 = 0x8889_000E;
    pub const AUDCLNT_E_EVENTHANDLE_NOT_EXPECTED: u32 = 0x8889_0011;
    pub const AUDCLNT_E_EVENTHANDLE_NOT_SET: u32 = 0x8889_0014;
    pub const AUDCLNT_E_INVALID_DEVICE_PERIOD: u32 = 0x8889_0020;
    pub const AUDCLNT_S_BUFFER_EMPTY: u32 = 0x0889_0001;
}
/// AUDCLNT_STREAMFLAGS_* a shared-mode stream may carry.
pub mod flags {
    pub const CROSSPROCESS: u32 = 0x0001_0000;
    pub const LOOPBACK: u32 = 0x0002_0000;
    pub const EVENTCALLBACK: u32 = 0x0004_0000;
    pub const NOPERSIST: u32 = 0x0008_0000;
    pub const RATEADJUST: u32 = 0x0010_0000;
    pub const SRC_DEFAULT_QUALITY: u32 = 0x0800_0000;
    pub const AUTOCONVERTPCM: u32 = 0x8000_0000;
    /// AUDCLNT_SESSIONFLAGS_*.
    pub const SESSION: u32 = 0x3000_0000;
    pub const VALID: u32 = CROSSPROCESS
        | LOOPBACK
        | EVENTCALLBACK
        | NOPERSIST
        | RATEADJUST
        | SRC_DEFAULT_QUALITY
        | AUTOCONVERTPCM
        | SESSION;
}
pub const AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY: u32 = 1;
pub const AUDCLNT_BUFFERFLAGS_SILENT: u32 = 2;
pub const DEVICE_STATE_ACTIVE: u32 = 1;
pub const DEVICE_STATE_NOTPRESENT: u32 = 4;
pub const DEVICE_STATEMASK_ALL: u32 = 0xF;

/// The engine's rate and its period (100 ns units): GetDevicePeriod's
/// default and minimum.
pub const MIX_RATE: u32 = 48_000;
pub const PERIOD_DEFAULT: i64 = 100_000;
pub const PERIOD_MINIMUM: i64 = 26_667;
/// A shared stream's smallest buffer at the mix rate (22 ms).
pub const MIN_BUFFER_FRAMES: u32 = 1056;

/// GetMixFormat's WAVEFORMATEXTENSIBLE: 48 kHz stereo 32-bit float,
/// front left and right.
pub fn mix_format() -> [u8; 40] {
    extensible(3, 32, MIX_RATE)
}
/// The device format (PKEY_AudioEngine_DeviceFormat): door.audio's 48 kHz
/// stereo 16-bit PCM.
pub fn device_format() -> [u8; 40] {
    extensible(1, 16, MIX_RATE)
}
fn extensible(sub: u16, bits: u16, rate: u32) -> [u8; 40] {
    let mut f = [0u8; 40];
    let block = 2 * bits / 8;
    f[0..2].copy_from_slice(&0xFFFEu16.to_le_bytes());
    f[2..4].copy_from_slice(&2u16.to_le_bytes());
    f[4..8].copy_from_slice(&rate.to_le_bytes());
    f[8..12].copy_from_slice(&(rate * u32::from(block)).to_le_bytes());
    f[12..14].copy_from_slice(&block.to_le_bytes());
    f[14..16].copy_from_slice(&bits.to_le_bytes());
    f[16..18].copy_from_slice(&22u16.to_le_bytes());
    f[18..20].copy_from_slice(&bits.to_le_bytes());
    // SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT.
    f[20..24].copy_from_slice(&3u32.to_le_bytes());
    // KSDATAFORMAT_SUBTYPE_PCM / _IEEE_FLOAT.
    f[24..26].copy_from_slice(&sub.to_le_bytes());
    f[26..40].copy_from_slice(&[
        0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
    ]);
    f
}

/// IsFormatSupported in shared mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    /// S_OK: the engine takes it as it is.
    Yes,
    /// S_FALSE with the mix format as the closest match (Initialize needs
    /// AUTOCONVERTPCM for it).
    Closest,
    /// AUDCLNT_E_UNSUPPORTED_FORMAT.
    No,
}
/// Windows 11 answers S_OK for integer PCM and float at the mix rate and
/// channel count, S_FALSE (closest: the mix format) for other PCM-family
/// formats, even malformed ones such as 12-bit samples, and refuses other
/// tags.
pub fn support(f: &[u8]) -> Support {
    if f.len() < 16 {
        return Support::No;
    }
    let u16_at = |at: usize| u16::from_le_bytes([f[at], f[at + 1]]);
    let mut tag = u16_at(0);
    if tag == 0xFFFE {
        if f.len() < 40 {
            return Support::No;
        }
        tag = u16_at(24);
    }
    if !matches!(tag, 1 | 3) {
        return Support::No;
    }
    match winmm::Wave::parse(f) {
        Some(w) if w.rate == MIX_RATE && w.channels == 2 => Support::Yes,
        _ => Support::Closest,
    }
}
/// The frames a shared stream's buffer holds: the requested duration (100
/// ns units) at `rate`, at least the engine's 22 ms; a stream the engine
/// converts from another rate holds the converter's 65 frames more
/// (44.1 kHz: 1036, as measured).
pub fn buffer_frames(rate: u32, duration: i64) -> u32 {
    let minimum = if rate == MIX_RATE {
        MIN_BUFFER_FRAMES
    } else {
        (u64::from(MIN_BUFFER_FRAMES) * u64::from(rate)).div_ceil(u64::from(MIX_RATE)) as u32 + 65
    };
    let asked = (duration.max(0) as u64 * u64::from(rate)).div_ceil(10_000_000);
    (asked.min(u64::from(u32::MAX >> 4)) as u32).max(minimum)
}

/// IAudioEndpointVolume's range (dB) and steps, as Windows reports an HD
/// audio speaker: -65.25 dB to 0 dB in 0.75 dB steps, 51 UI steps.
pub const VOLUME_MIN_DB: f32 = -65.25;
pub const VOLUME_MAX_DB: f32 = 0.0;
pub const VOLUME_STEP_DB: f32 = 0.75;
pub const VOLUME_STEPS: u32 = 51;
/// The level in dB of a 0..=1 scalar: 20·log10, floored at the range.
pub fn scalar_to_db(scalar: f32) -> f32 {
    if scalar <= 0.0 {
        return VOLUME_MIN_DB;
    }
    (20.0 * log2(scalar) * core::f32::consts::LOG10_2).clamp(VOLUME_MIN_DB, VOLUME_MAX_DB)
}
/// The scalar of a level in dB: 10^(dB/20).
pub fn db_to_scalar(db: f32) -> f32 {
    if db <= VOLUME_MIN_DB {
        return 0.0;
    }
    exp2(db.min(VOLUME_MAX_DB) / 20.0 * core::f32::consts::LOG2_10).clamp(0.0, 1.0)
}
/// log2 without std: the exponent, then a polynomial on the mantissa.
fn log2(x: f32) -> f32 {
    let bits = x.to_bits();
    let exponent = ((bits >> 23) & 0xFF) as i32 - 127;
    let m = f32::from_bits((bits & 0x007F_FFFF) | 0x3F80_0000);
    // log2(m), 1 <= m < 2 (error < 1e-4).
    let t = m - 1.0;
    let poly = t
        * (core::f32::consts::LOG2_E
            + t * (-0.721_347_5 + t * (0.479_765 + t * (-0.346_573 + t * 0.145_738))));
    exponent as f32 + poly
}
/// 2^x without std.
fn exp2(x: f32) -> f32 {
    let whole = if x < 0.0 && (x as i32) as f32 != x {
        x as i32 - 1
    } else {
        x as i32
    };
    let frac = x - whole as f32;
    let poly = 1.0
        + frac
            * (core::f32::consts::LN_2
                + frac * (0.240_226_5 + frac * (0.055_504_1 + frac * 0.009_618_1)));
    if whole < -126 {
        return 0.0;
    }
    poly * f32::from_bits(((whole + 127) as u32) << 23)
}

/// An endpoint ID: `{0.0.<flow>.00000000}.{<guid>}` (flow 0 render,
/// 1 capture), the GUID lower case, 55 characters.
pub fn endpoint_id(capture: bool, guid: &[u8; 16]) -> [u8; 55] {
    let mut id = [0u8; 55];
    id[..18].copy_from_slice(b"{0.0.0.00000000}.{");
    if capture {
        id[5] = b'1';
    }
    id[18..54].copy_from_slice(&guid_text(guid));
    id[54] = b'}';
    id
}
/// A GUID as `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`, lower case.
pub fn guid_text(g: &[u8; 16]) -> [u8; 36] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let order = [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15];
    let mut out = [0u8; 36];
    let mut at = 0;
    for (k, &i) in order.iter().enumerate() {
        if matches!(k, 4 | 6 | 8 | 10) {
            out[at] = b'-';
            at += 1;
        }
        out[at] = HEX[usize::from(g[i] >> 4)];
        out[at + 1] = HEX[usize::from(g[i] & 15)];
        at += 2;
    }
    out
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;

    fn pcm(rate: u32, channels: u16, bits: u16) -> [u8; 18] {
        let block = channels * bits / 8;
        let mut b = [0u8; 18];
        b[0..2].copy_from_slice(&1u16.to_le_bytes());
        b[2..4].copy_from_slice(&channels.to_le_bytes());
        b[4..8].copy_from_slice(&rate.to_le_bytes());
        b[8..12].copy_from_slice(&(rate * u32::from(block)).to_le_bytes());
        b[12..14].copy_from_slice(&block.to_le_bytes());
        b[14..16].copy_from_slice(&bits.to_le_bytes());
        b
    }

    #[test]
    fn formats_answer_like_windows() {
        let mix = mix_format();
        assert_eq!(support(&mix), Support::Yes);
        assert_eq!(u32::from_le_bytes(mix[8..12].try_into().unwrap()), 384_000);
        assert_eq!(support(&pcm(48_000, 2, 16)), Support::Yes);
        assert_eq!(support(&pcm(44_100, 2, 16)), Support::Closest);
        assert_eq!(support(&pcm(48_000, 2, 12)), Support::Closest);
        assert_eq!(support(&pcm(48_000, 1, 16)), Support::Closest);
        let mut adpcm = pcm(48_000, 2, 4);
        adpcm[0] = 2;
        assert_eq!(support(&adpcm), Support::No);
        assert_eq!(support(&device_format()), Support::Yes);
    }

    #[test]
    fn buffers_are_sized_like_windows() {
        assert_eq!(buffer_frames(48_000, 0), 1056);
        assert_eq!(buffer_frames(48_000, 2_000_000), 9600);
        assert_eq!(buffer_frames(44_100, 0), 1036);
        assert_eq!(buffer_frames(48_000, 100_000), 1056);
    }

    #[test]
    fn volume_curve_round_trips() {
        assert_eq!(scalar_to_db(0.0), VOLUME_MIN_DB);
        assert!(scalar_to_db(1.0).abs() < 0.01);
        assert!((scalar_to_db(0.5) + 6.02).abs() < 0.05);
        for s in [0.1f32, 0.25, 0.5, 0.75, 1.0] {
            assert!((db_to_scalar(scalar_to_db(s)) - s).abs() < 0.005, "{s}");
        }
        assert_eq!(db_to_scalar(-100.0), 0.0);
    }

    #[test]
    fn endpoint_ids_have_windows_shape() {
        let g = [
            0x01, 0x5D, 0x02, 0xD0, 0xA4, 0x5E, 0x6B, 0x4A, 0x9C, 0x11, 0x2B, 0x6E, 0x15, 0x50,
            0x00, 0x01,
        ];
        let id = endpoint_id(false, &g);
        assert_eq!(
            &id[..],
            b"{0.0.0.00000000}.{d0025d01-5ea4-4a6b-9c11-2b6e15500001}"
        );
        assert_eq!(endpoint_id(true, &g)[5], b'1');
    }
}
