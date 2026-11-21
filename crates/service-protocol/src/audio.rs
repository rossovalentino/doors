//! door.audio (P15): the sound service's status, master volume, playback
//! streams and capture. A stream plays PCM the client hands over in sections
//! (`door_workspace::section_from`): the service maps each one read-only,
//! resamples it to the device format (48 kHz stereo, 16-bit) and mixes it.
//! A capture stream fills a ring in a section the client hands over
//! ([`capture`]): the service writes, the client reads. All integers are
//! little endian.

pub mod op {
    /// Device and mixer state (`Status`).
    pub const STATUS: u32 = 0x6100;
    /// Master output (`Output`): volume, mute, mono, balance.
    pub const SET_OUTPUT: u32 = 0x6101;
    /// Open a stream (`Format`): answers its id.
    pub const OPEN: u32 = 0x6110;
    /// Queue PCM for a stream (id u32; one section transferred, its byte
    /// count after the id as u32): answers the bytes queued so far.
    pub const QUEUE: u32 = 0x6111;
    /// Stream state (id u32): `StreamStatus`.
    pub const STREAM: u32 = 0x6112;
    /// Pause (1) or resume (0) a stream (id u32, flag u8).
    pub const PAUSE: u32 = 0x6113;
    /// Per-stream volume (id u32, volume u8 0..=100): the volume mixer.
    pub const STREAM_VOLUME: u32 = 0x6114;
    /// Close a stream; its queued sound stops at once.
    pub const CLOSE: u32 = 0x6115;
    /// Settings' Test button: a 440 Hz tone, one second, through the mixer.
    pub const TEST_TONE: u32 = 0x6120;
    /// Play a system sound (`sound` event u32), as the Windows sound scheme
    /// assigns it; any client may (MessageBeep is unprivileged).
    pub const PLAY_SOUND: u32 = 0x6121;
    /// Open a capture stream into the ring of the one transferred section
    /// (`capture` layout, header filled by the client; the body is the
    /// caller's display name, UTF-8, at most `NAME_BYTES`): answers its id;
    /// `Denied` when the microphone policy refuses the caller,
    /// `Unsupported` without an input device.
    pub const CAPTURE_OPEN: u32 = 0x6130;
    /// Close a capture stream (id u32); its ring is released.
    pub const CAPTURE_CLOSE: u32 = 0x6131;
    /// The microphone policy (`capture` policy word u32): the shell and
    /// Settings only. Refused streams stop with ring state `REVOKED`.
    pub const CAPTURE_POLICY: u32 = 0x6132;
    /// Capture state and recent activity (`CaptureStatus`).
    pub const CAPTURE_STATUS: u32 = 0x6133;
    /// The microphone level (`Input`), Settings > System > Sound > Input >
    /// Volume; any client may set it (Windows lets programs change an
    /// endpoint's level), and the shell adopts a program's change.
    pub const SET_INPUT: u32 = 0x6134;
}
/// Streams one client may hold, and in total.
pub const STREAMS_PER_CLIENT: usize = 4;
pub const STREAMS: usize = 8;
/// Largest section one QUEUE hands over (about 43 s of 48 kHz stereo;
/// the service maps each into one 8 MiB slot).
pub const MAX_SECTION: usize = 8 * 1024 * 1024;
/// Device format.
pub const RATE: u32 = 48_000;
pub const CHANNELS: u8 = 2;

/// Master output settings (Settings > System > Sound and quick settings).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Output {
    /// 0..=100.
    pub volume: u8,
    pub muted: bool,
    /// Accessibility > Audio > Mono audio.
    pub mono: bool,
    /// Left/right balance, -100 (left only) ..= 100 (right only).
    pub balance: i8,
}
impl Default for Output {
    fn default() -> Self {
        Self {
            volume: 50,
            muted: false,
            mono: false,
            balance: 0,
        }
    }
}
pub const OUTPUT_BYTES: usize = 4;
impl Output {
    pub fn encode(&self) -> [u8; OUTPUT_BYTES] {
        [
            self.volume,
            u8::from(self.muted) | u8::from(self.mono) << 1,
            self.balance as u8,
            0,
        ]
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        let balance = *b.get(2)? as i8;
        if b.len() != OUTPUT_BYTES || b[0] > 100 || b[1] & !3 != 0 || b[3] != 0 || balance < -100 {
            return None;
        }
        Some(Self {
            volume: b[0],
            muted: b[1] & 1 != 0,
            mono: b[1] & 2 != 0,
            balance,
        })
    }
    /// The preference word that stores it (`preferences::SOUND`).
    pub fn word(&self) -> u32 {
        u32::from_le_bytes(self.encode())
    }
    pub fn from_word(word: u32) -> Option<Self> {
        Self::decode(&word.to_le_bytes())
    }
}

/// A stream's PCM format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    /// 8000..=192000 Hz.
    pub rate: u32,
    /// 1 or 2.
    pub channels: u8,
    /// Bits per sample: 8 (unsigned), 16, 24 or 32 (signed integer).
    pub bits: u8,
}
pub const FORMAT_BYTES: usize = 8;
impl Format {
    pub fn encode(&self) -> [u8; FORMAT_BYTES] {
        let mut b = [0u8; FORMAT_BYTES];
        b[..4].copy_from_slice(&self.rate.to_le_bytes());
        b[4] = self.channels;
        b[5] = self.bits;
        b
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != FORMAT_BYTES || b[6..].iter().any(|v| *v != 0) {
            return None;
        }
        let f = Self {
            rate: u32::from_le_bytes(b[..4].try_into().ok()?),
            channels: b[4],
            bits: b[5],
        };
        ((8000..=192_000).contains(&f.rate)
            && matches!(f.channels, 1 | 2)
            && matches!(f.bits, 8 | 16 | 24 | 32))
        .then_some(f)
    }
    /// Bytes per frame (one sample per channel).
    pub fn frame(&self) -> usize {
        usize::from(self.channels) * usize::from(self.bits / 8)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// An output device exists (virtio-sound).
    pub present: bool,
    /// The device is playing (some stream has sound queued).
    pub running: bool,
    /// The device has an input to capture from, and capture streams open.
    pub capture: bool,
    pub capturing: u8,
    pub output: Output,
    pub streams: u8,
    /// Device format.
    pub rate: u32,
    pub channels: u8,
    pub bits: u8,
    /// 10 ms periods played, and periods the device ran dry while playing.
    pub played: u64,
    pub underruns: u64,
    /// The microphone level applied to every captured period.
    pub input: Input,
}
pub const STATUS_BYTES: usize = 32;
impl Status {
    pub fn encode(&self) -> [u8; STATUS_BYTES] {
        let mut b = [0u8; STATUS_BYTES];
        b[0] = u8::from(self.present) | u8::from(self.running) << 1 | u8::from(self.capture) << 2;
        b[1] = self.streams;
        b[2] = self.channels;
        b[3] = self.bits;
        b[4..8].copy_from_slice(&self.output.encode());
        b[8..12].copy_from_slice(&self.rate.to_le_bytes());
        b[12] = self.capturing;
        b[13..15].copy_from_slice(&self.input.encode());
        b[16..24].copy_from_slice(&self.played.to_le_bytes());
        b[24..32].copy_from_slice(&self.underruns.to_le_bytes());
        b
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != STATUS_BYTES || b[0] & !7 != 0 || b[15] != 0 {
            return None;
        }
        let input = Input::decode(&b[13..15])?;
        Some(Self {
            present: b[0] & 1 != 0,
            running: b[0] & 2 != 0,
            capture: b[0] & 4 != 0,
            capturing: b[12],
            streams: b[1],
            channels: b[2],
            bits: b[3],
            output: Output::decode(&b[4..8])?,
            rate: u32::from_le_bytes(b[8..12].try_into().ok()?),
            played: u64::from_le_bytes(b[16..24].try_into().ok()?),
            underruns: u64::from_le_bytes(b[24..32].try_into().ok()?),
            input,
        })
    }
}

/// The microphone's level: 0..=100 (amplitude, applied by door.audio to
/// what it captures) and mute. Default full and unmuted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Input {
    pub volume: u8,
    pub muted: bool,
}
impl Default for Input {
    fn default() -> Self {
        Self {
            volume: 100,
            muted: false,
        }
    }
}
pub const INPUT_BYTES: usize = 2;
impl Input {
    pub fn encode(&self) -> [u8; INPUT_BYTES] {
        [self.volume, u8::from(self.muted)]
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        (b.len() == INPUT_BYTES && b[0] <= 100 && b[1] & !1 == 0).then(|| Self {
            volume: b[0],
            muted: b[1] != 0,
        })
    }
    /// Scales one captured 16-bit sample.
    pub fn apply(&self, sample: i16) -> i16 {
        if self.muted {
            0
        } else {
            (i32::from(sample) * i32::from(self.volume) / 100) as i16
        }
    }
}

/// A stream's playback state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamStatus {
    pub paused: bool,
    /// Every queued section has been played.
    pub drained: bool,
    pub volume: u8,
    /// Source frames played and still queued.
    pub played: u64,
    pub queued: u64,
}
pub const STREAM_BYTES: usize = 24;
impl StreamStatus {
    pub fn encode(&self) -> [u8; STREAM_BYTES] {
        let mut b = [0u8; STREAM_BYTES];
        b[0] = u8::from(self.paused) | u8::from(self.drained) << 1;
        b[1] = self.volume;
        b[8..16].copy_from_slice(&self.played.to_le_bytes());
        b[16..24].copy_from_slice(&self.queued.to_le_bytes());
        b
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != STREAM_BYTES || b[0] & !3 != 0 || b[1] > 100 {
            return None;
        }
        Some(Self {
            paused: b[0] & 1 != 0,
            drained: b[0] & 2 != 0,
            volume: b[1],
            played: u64::from_le_bytes(b[8..16].try_into().ok()?),
            queued: u64::from_le_bytes(b[16..24].try_into().ok()?),
        })
    }
}

/// Capture rings and the microphone policy.
///
/// A ring is a section the client creates and maps: a 64-byte header, then
/// `capacity` bytes of 48 kHz stereo 16-bit PCM (the device format; whole
/// 4-byte frames). The service (producer) copies each captured period after
/// `write`, then advances `write`; the client (consumer) copies from
/// `read` up to `write`, then advances `read`. Both are byte counts since
/// the stream opened (position = count % capacity). A period that does not
/// fit is dropped and counted in `overruns`.
pub mod capture {
    /// Header fields (byte offsets).
    pub const MAGIC: usize = 0;
    pub const CAPACITY: usize = 4;
    pub const WRITE: usize = 8;
    pub const READ: usize = 16;
    pub const OVERRUNS: usize = 24;
    pub const STATE: usize = 28;
    pub const HEADER: usize = 64;
    /// "DCAP".
    pub const MAGIC_VALUE: u32 = 0x5041_4344;
    /// `STATE` values: capturing; stopped because the policy now refuses the
    /// stream; stopped because the input device went away.
    pub const RUNNING: u32 = 0;
    pub const REVOKED: u32 = 1;
    pub const LOST: u32 = 2;
    /// Ring capacity: whole 10 ms periods, 100 ms to 4 s.
    pub const PERIOD_BYTES: usize = 1920;
    pub const MIN_CAPACITY: usize = 10 * PERIOD_BYTES;
    pub const MAX_CAPACITY: usize = 400 * PERIOD_BYTES;
    pub const fn valid_capacity(bytes: usize) -> bool {
        bytes >= MIN_CAPACITY && bytes <= MAX_CAPACITY && bytes.is_multiple_of(PERIOD_BYTES)
    }
    /// Policy word bits (`preferences::MICROPHONE`), as Settings > Privacy &
    /// security > Microphone: 0 is Windows' default, everything allowed.
    /// "Microphone access" off.
    pub const ACCESS_OFF: u32 = 1;
    /// "Let apps access your microphone" off (Doors' own apps).
    pub const APPS_OFF: u32 = 2;
    /// "Let desktop apps access your microphone" off (Windows programs).
    pub const DESKTOP_OFF: u32 = 4;
    pub const POLICY_ALL: u32 = 7;
    /// The preference word also keeps the microphone level: 100 minus the
    /// volume in bits 8..16 (so 0, the default, is full) and mute in bit 16.
    pub const INPUT_BITS: u32 = 0x1_FF00;
    pub const fn input_of(word: u32) -> super::Input {
        let down = (word >> 8) & 0xFF;
        super::Input {
            volume: if down > 100 { 0 } else { 100 - down as u8 },
            muted: word & (1 << 16) != 0,
        }
    }
    pub const fn with_input(word: u32, input: super::Input) -> u32 {
        (word & !INPUT_BITS)
            | ((100 - input.volume as u32) << 8)
            | if input.muted { 1 << 16 } else { 0 }
    }
    /// A valid preference word: policy bits and a level.
    pub const fn valid_word(word: u32) -> bool {
        word & !(POLICY_ALL | INPUT_BITS) == 0 && (word >> 8) & 0xFF <= 100
    }
    /// May a desktop (Windows) program, or one of Doors' own apps, capture?
    pub const fn allows(policy: u32, desktop: bool) -> bool {
        policy & ACCESS_OFF == 0 && policy & if desktop { DESKTOP_OFF } else { APPS_OFF } == 0
    }
}

/// Longest display name a capture stream records (UTF-8 bytes).
pub const NAME_BYTES: usize = 32;
/// One app's microphone use, for the Microphone page's recent activity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CaptureUse {
    pub pid: u32,
    pub manifest: u32,
    /// The name the app gave (`name_len` UTF-8 bytes), e.g. its executable.
    pub name: [u8; NAME_BYTES],
    pub name_len: u8,
    /// The service's monotonic milliseconds when it last captured.
    pub last: u64,
    /// Capturing now.
    pub active: bool,
    /// A Windows program (a desktop app); else one of Doors' own apps.
    pub desktop: bool,
}
impl CaptureUse {
    pub fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..usize::from(self.name_len).min(NAME_BYTES)]).unwrap_or("")
    }
    /// `name` cut at a character boundary to `NAME_BYTES`.
    pub fn with_name(mut self, name: &str) -> Self {
        let mut n = name.len().min(NAME_BYTES);
        while !name.is_char_boundary(n) {
            n -= 1;
        }
        self.name = [0; NAME_BYTES];
        self.name[..n].copy_from_slice(&name.as_bytes()[..n]);
        self.name_len = n as u8;
        self
    }
}
/// Most recent users `CaptureStatus` lists.
pub const RECENT: usize = 4;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CaptureStatus {
    /// The device has an input.
    pub present: bool,
    /// The capture policy word.
    pub policy: u32,
    /// The service's monotonic milliseconds now.
    pub now: u64,
    /// Most recent first; `used` entries are valid.
    pub used: u8,
    pub recent: [CaptureUse; RECENT],
}
const USE_BYTES: usize = 24 + NAME_BYTES;
pub const CAPTURE_STATUS_BYTES: usize = 16 + RECENT * USE_BYTES;
impl CaptureStatus {
    pub fn encode(&self) -> [u8; CAPTURE_STATUS_BYTES] {
        let mut b = [0u8; CAPTURE_STATUS_BYTES];
        b[0] = u8::from(self.present);
        b[1] = self.used.min(RECENT as u8);
        b[4..8].copy_from_slice(&self.policy.to_le_bytes());
        b[8..16].copy_from_slice(&self.now.to_le_bytes());
        for (i, u) in self.recent.iter().take(usize::from(b[1])).enumerate() {
            let at = 16 + i * USE_BYTES;
            b[at..at + 4].copy_from_slice(&u.pid.to_le_bytes());
            b[at + 4..at + 8].copy_from_slice(&u.manifest.to_le_bytes());
            b[at + 8..at + 16].copy_from_slice(&u.last.to_le_bytes());
            b[at + 16] = u8::from(u.active) | u8::from(u.desktop) << 1;
            b[at + 17] = u.name_len.min(NAME_BYTES as u8);
            b[at + 24..at + 24 + NAME_BYTES].copy_from_slice(&u.name);
        }
        b
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != CAPTURE_STATUS_BYTES || b[0] > 1 || usize::from(b[1]) > RECENT {
            return None;
        }
        let policy = u32::from_le_bytes(b[4..8].try_into().ok()?);
        if policy & !capture::POLICY_ALL != 0 {
            return None;
        }
        let mut s = Self {
            present: b[0] == 1,
            policy,
            now: u64::from_le_bytes(b[8..16].try_into().ok()?),
            used: b[1],
            recent: [CaptureUse::default(); RECENT],
        };
        for i in 0..usize::from(b[1]) {
            let at = 16 + i * USE_BYTES;
            if b[at + 16] & !3 != 0 || usize::from(b[at + 17]) > NAME_BYTES {
                return None;
            }
            let mut name = [0u8; NAME_BYTES];
            name.copy_from_slice(&b[at + 24..at + 24 + NAME_BYTES]);
            core::str::from_utf8(&name[..usize::from(b[at + 17])]).ok()?;
            s.recent[i] = CaptureUse {
                name,
                name_len: b[at + 17],
                pid: u32::from_le_bytes(b[at..at + 4].try_into().ok()?),
                manifest: u32::from_le_bytes(b[at + 4..at + 8].try_into().ok()?),
                last: u64::from_le_bytes(b[at + 8..at + 16].try_into().ok()?),
                active: b[at + 16] & 1 != 0,
                desktop: b[at + 16] & 2 != 0,
            };
        }
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_policy_and_status_round_trip() {
        assert!(capture::allows(0, true) && capture::allows(0, false));
        assert!(!capture::allows(capture::ACCESS_OFF, false));
        assert!(
            !capture::allows(capture::DESKTOP_OFF, true)
                && capture::allows(capture::DESKTOP_OFF, false)
        );
        assert!(
            !capture::allows(capture::APPS_OFF, false) && capture::allows(capture::APPS_OFF, true)
        );
        assert!(
            capture::valid_capacity(capture::MIN_CAPACITY)
                && !capture::valid_capacity(capture::MIN_CAPACITY + 4)
        );
        assert!(!capture::valid_capacity(
            capture::MAX_CAPACITY + capture::PERIOD_BYTES
        ));
        let mut s = CaptureStatus {
            present: true,
            policy: capture::DESKTOP_OFF,
            now: 99,
            used: 2,
            ..Default::default()
        };
        s.recent[0] = CaptureUse {
            pid: 12,
            manifest: 10,
            last: 90,
            active: true,
            desktop: true,
            ..Default::default()
        }
        .with_name("soundprobe.exe");
        s.recent[1] = CaptureUse {
            pid: 7,
            manifest: 21,
            last: 40,
            active: false,
            desktop: false,
            ..Default::default()
        }
        .with_name("Sound Recorder");
        assert_eq!(s.recent[0].name(), "soundprobe.exe");
        // Names are cut at a character boundary.
        assert_eq!(
            CaptureUse::default()
                .with_name("ééééééééééééééééé")
                .name()
                .len(),
            32
        );
        assert_eq!(CaptureStatus::decode(&s.encode()), Some(s));
        let mut bad = s.encode();
        bad[4] = 8;
        assert_eq!(CaptureStatus::decode(&bad), None);
        bad = s.encode();
        bad[1] = RECENT as u8 + 1;
        assert_eq!(CaptureStatus::decode(&bad), None);
        const _: () = assert!(CAPTURE_STATUS_BYTES <= crate::wire::BODY_BYTES);
    }
    #[test]
    fn records_round_trip_and_reject_malformed_words() {
        let o = Output {
            volume: 73,
            muted: true,
            mono: true,
            balance: -40,
        };
        assert_eq!(Output::decode(&o.encode()), Some(o));
        assert_eq!(Output::from_word(o.word()), Some(o));
        assert_eq!(Output::decode(&[101, 0, 0, 0]), None);
        assert_eq!(Output::decode(&[50, 0, (-101i8) as u8, 0]), None);
        let f = Format {
            rate: 44_100,
            channels: 2,
            bits: 16,
        };
        assert_eq!(Format::decode(&f.encode()), Some(f));
        assert_eq!(f.frame(), 4);
        let mut bad = f.encode();
        bad[5] = 12;
        assert_eq!(Format::decode(&bad), None);
        let s = Status {
            present: true,
            running: true,
            capture: true,
            capturing: 2,
            output: o,
            streams: 2,
            rate: RATE,
            channels: CHANNELS,
            bits: 16,
            played: 1234,
            underruns: 3,
            input: Input {
                volume: 35,
                muted: true,
            },
        };
        assert_eq!(Status::decode(&s.encode()), Some(s));
        // The Microphone word keeps the policy bits and the level.
        use capture::{DESKTOP_OFF, input_of, valid_word, with_input};
        assert_eq!(input_of(0), Input::default());
        let word = with_input(DESKTOP_OFF, s.input);
        assert!(valid_word(word));
        assert_eq!(
            (input_of(word), word & capture::POLICY_ALL),
            (s.input, DESKTOP_OFF)
        );
        assert!(!valid_word(101 << 8));
        assert_eq!(s.input.apply(1000), 0);
        assert_eq!(
            Input {
                volume: 50,
                muted: false
            }
            .apply(-1000),
            -500
        );
        let t = StreamStatus {
            paused: false,
            drained: true,
            volume: 100,
            played: 48_000,
            queued: 0,
        };
        assert_eq!(StreamStatus::decode(&t.encode()), Some(t));
        const _: () = assert!(STATUS_BYTES <= crate::wire::BODY_BYTES);
    }
}

/// System sound events (Windows 11 26H2 default scheme, `HKCU\AppEvents`):
/// the scheme maps each to its sound; events the scheme leaves silent
/// (Empty Recycle Bin, Navigation, Close, Minimize, ...) have no id.
pub mod sound {
    /// `.Default` (Default Beep): Windows Background.
    pub const DEFAULT_BEEP: u32 = 0;
    /// `SystemAsterisk`: Windows Background.
    pub const ASTERISK: u32 = 1;
    /// `SystemExclamation`: Windows Background.
    pub const EXCLAMATION: u32 = 2;
    /// `SystemHand` (Critical Stop): Windows Foreground.
    pub const CRITICAL_STOP: u32 = 3;
    /// `SystemNotification`: Windows Background.
    pub const SYSTEM_NOTIFICATION: u32 = 4;
    /// `Notification.Default` (toasts): Windows Notify System Generic.
    pub const NOTIFICATION: u32 = 5;
    /// `Notification.Reminder`: Windows Notify Calendar.
    pub const REMINDER: u32 = 6;
    /// `Notification.Mail`: Windows Notify Email.
    pub const MAIL: u32 = 7;
    /// `Notification.IM`: Windows Notify Messaging.
    pub const MESSAGING: u32 = 8;
    /// `WindowsUnlock`: Windows Unlock.
    pub const UNLOCK: u32 = 9;
    /// `WindowsUAC`: Windows User Account Control.
    pub const UAC: u32 = 10;
    /// `DeviceConnect` / `DeviceDisconnect` / `DeviceFail`.
    pub const DEVICE_CONNECT: u32 = 11;
    pub const DEVICE_DISCONNECT: u32 = 12;
    pub const DEVICE_FAIL: u32 = 13;
    /// `Notification.Looping.Alarm`: Alarm01 (Clock alarms and timers).
    pub const ALARM: u32 = 14;
    /// `MessageNudge`.
    pub const NUDGE: u32 = 15;
    pub const COUNT: u32 = 16;
    /// Names as Control Panel > Sound > Sounds lists them.
    pub const NAMES: [&str; COUNT as usize] = [
        "Default Beep",
        "Asterisk",
        "Exclamation",
        "Critical Stop",
        "System Notification",
        "Notification",
        "Calendar Reminder",
        "New Mail Notification",
        "Instant Message Notification",
        "Windows Unlock",
        "Windows User Account Control",
        "Device Connect",
        "Device Disconnect",
        "Device Failed to Connect",
        "Alarm",
        "Message Nudge",
    ];
}
