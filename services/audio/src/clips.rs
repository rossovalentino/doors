//! The system sound scheme (Windows 11 26H2 defaults, read from the owner's
//! PC): each `audio::sound` event's WAV, built into the service from
//! `assets/sounds/windows-media` (private build, never redistributed), and
//! the RIFF reader that finds its PCM.
use door_service_protocol::audio::{Format, sound};

#[cfg(windows_media)]
macro_rules! wav {
    ($name:literal) => {
        include_bytes!(concat!(
            "../../../assets/sounds/windows-media/",
            $name,
            ".wav"
        ))
    };
}
// Without the private scheme (see build.rs) every event is silent.
#[cfg(not(windows_media))]
macro_rules! wav {
    ($name:literal) => {
        &[]
    };
}
static BACKGROUND: &[u8] = wav!("background");
static FOREGROUND: &[u8] = wav!("foreground");
static GENERIC: &[u8] = wav!("notify-system-generic");
static CALENDAR: &[u8] = wav!("notify-calendar");
static EMAIL: &[u8] = wav!("notify-email");
static MESSAGING: &[u8] = wav!("notify-messaging");
static UNLOCK: &[u8] = wav!("unlock");
static UAC: &[u8] = wav!("user-account-control");
static INSERT: &[u8] = wav!("hardware-insert");
static REMOVE: &[u8] = wav!("hardware-remove");
static FAIL: &[u8] = wav!("hardware-fail");
static ALARM: &[u8] = wav!("alarm01");
static NUDGE: &[u8] = wav!("message-nudge");

/// The WAV file the scheme assigns to `event` (None when the private
/// scheme is not in this build).
pub fn file(event: u32) -> Option<&'static [u8]> {
    file_any(event).filter(|b| !b.is_empty())
}
fn file_any(event: u32) -> Option<&'static [u8]> {
    Some(match event {
        sound::DEFAULT_BEEP | sound::ASTERISK | sound::EXCLAMATION | sound::SYSTEM_NOTIFICATION => {
            BACKGROUND
        }
        sound::CRITICAL_STOP => FOREGROUND,
        sound::NOTIFICATION => GENERIC,
        sound::REMINDER => CALENDAR,
        sound::MAIL => EMAIL,
        sound::MESSAGING => MESSAGING,
        sound::UNLOCK => UNLOCK,
        sound::UAC => UAC,
        sound::DEVICE_CONNECT => INSERT,
        sound::DEVICE_DISCONNECT => REMOVE,
        sound::DEVICE_FAIL => FAIL,
        sound::ALARM => ALARM,
        sound::NUDGE => NUDGE,
        _ => return None,
    })
}

/// A PCM WAV's format and sample bytes (whole frames), or None for anything
/// else (compressed formats, truncated chunks, odd sizes).
pub fn parse(wav: &[u8]) -> Option<(Format, &[u8])> {
    if wav.len() < 12 || &wav[0..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return None;
    }
    let mut at = 12;
    let mut format = None;
    while at + 8 <= wav.len() {
        let id = &wav[at..at + 4];
        let len = u32::from_le_bytes(wav[at + 4..at + 8].try_into().ok()?) as usize;
        let body = wav.get(at + 8..at + 8 + len.min(wav.len() - at - 8))?;
        match id {
            b"fmt " if body.len() >= 16 => {
                let tag = u16::from_le_bytes([body[0], body[1]]);
                let channels = u16::from_le_bytes([body[2], body[3]]);
                let rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
                let bits = u16::from_le_bytes([body[14], body[15]]);
                // PCM, or WAVE_FORMAT_EXTENSIBLE carrying PCM.
                let pcm = tag == 1
                    || (tag == 0xFFFE && body.len() >= 26 && body[24] == 1 && body[25] == 0);
                if !pcm
                    || !(1..=2).contains(&channels)
                    || !matches!(bits, 8 | 16 | 24 | 32)
                    || !(8_000..=192_000).contains(&rate)
                {
                    return None;
                }
                format = Some(Format {
                    rate,
                    channels: channels as u8,
                    bits: bits as u8,
                });
            }
            b"data" => {
                let f = format?;
                let whole = body.len() / f.frame() * f.frame();
                return Some((f, &body[..whole]));
            }
            _ => (),
        }
        at += 8 + len + (len & 1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_scheme_event_has_a_pcm_wav() {
        for event in 0..sound::COUNT {
            let (format, pcm) = parse(file(event).unwrap()).unwrap();
            assert_eq!((format.channels, format.bits), (2, 16), "event {event}");
            assert!(pcm.len() > 4 * format.rate as usize / 10, "event {event}");
        }
        assert!(file(sound::COUNT).is_none());
    }
    #[test]
    fn malformed_wavs_are_refused() {
        assert!(parse(b"RIFF\0\0\0\0WAVE").is_none());
        let mut wav = file(sound::NOTIFICATION).unwrap().to_vec();
        wav[20] = 2; // ADPCM
        assert!(parse(&wav).is_none());
        // A data chunk before fmt has no format.
        let mut odd = b"RIFF\0\0\0\0WAVEdata\x04\0\0\0abcd".to_vec();
        assert!(parse(&odd).is_none());
        odd.truncate(18);
        assert!(parse(&odd).is_none());
    }
}
