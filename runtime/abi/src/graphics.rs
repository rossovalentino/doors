//! P11 device transport. Display authority never crosses a client window channel.
use crate::{Status, get32, get64, put32, put64};
pub const INFO_BYTES: usize = 32;
pub const INPUT_BYTES: usize = 32;
pub const SECTION_MAX_BYTES: u64 = 32 * 1024 * 1024;
/// One bounded display capture; fits the guarded 64 KiB syscall stack.
pub const MAX_COPY_BYTES: usize = 16 * 1024;
pub const SECTION_COPY_BYTES: usize = 64 * 1024;
pub const CPU_COPY: u64 = 1;
pub const PARTIAL_COPY: u64 = 2;
pub const POINTER: u64 = 4;
/// The pointing device reports wheel detents (PS/2 IntelliMouse protocol).
pub const WHEEL: u64 = 8;
/// Largest wheel delta carried by one MOTION event, in detents.
pub const MAX_WHEEL: i32 = 127;
/// Host-suggested display scale in percent, carried in capability bits
/// 32..48 (QEMU fw_cfg `opt/door/scale`; 0 = no hint). Only Windows
/// display scales are reported: 100..=300 in steps of 25.
pub const SCALE_HINT_SHIFT: u32 = 32;
pub const SCALE_HINT_MASK: u64 = 0xffff << SCALE_HINT_SHIFT;
/// Parse the ASCII decimal `opt/door/scale` value ("150", optional trailing
/// newline); anything else, or a scale Windows does not offer, is no hint.
pub fn scale_hint_from_text(text: &[u8]) -> u32 {
    let digits = text.strip_suffix(b"\n").unwrap_or(text);
    if digits.is_empty() || digits.len() > 3 || !digits.iter().all(u8::is_ascii_digit) {
        return 0;
    }
    let value = digits
        .iter()
        .fold(0u32, |v, d| v * 10 + u32::from(d - b'0'));
    if value != 0 && valid_scale_hint(value) {
        value
    } else {
        0
    }
}
pub const fn valid_scale_hint(percent: u32) -> bool {
    percent == 0 || (percent >= 100 && percent <= 300 && percent.is_multiple_of(25))
}
pub const KEY: u32 = 1;
/// Relative motion `x`/`y`, held buttons in `flags`, and (with `WHEEL`) the
/// signed wheel delta in `code` as two's complement: detents away from the
/// user (scroll up) are positive. Consumers ignoring `code` see the original
/// three-button motion event unchanged.
pub const MOTION: u32 = 2;
/// Device input epoch lost: cancel focus/capture/pressed state before routing
/// subsequent events. A bounded input overflow must not kill the desktop.
pub const RESET: u32 = 3;
/// KEY events (§53, `specs/contracts/text-input-v1.md`). `code` is the
/// physical PS/2 set-1 key: 0x01..=0x7F, 0x100 | 0x01..=0x7F for E0-prefixed
/// keys, or `HANJA`/`HANGUL` (make-only Korean keys: one DOWN, never an up).
/// `text` is the legacy US-layout ASCII translation (0..=127); the window
/// server translates the session keyboard layout itself. `flags` carries the
/// complete modifier and lock state *after* this event. The aggregate
/// SHIFT/CONTROL/ALT bits keep their original meaning; a sided bit implies
/// its aggregate bit, and REPEAT implies DOWN.
pub const DOWN: u32 = 1;
pub const SHIFT: u32 = 2;
pub const CONTROL: u32 = 4;
pub const ALT: u32 = 8;
/// Typematic repeat: a make code for a key that is already down.
pub const REPEAT: u32 = 16;
/// Right Alt held (AltGr on layouts that define one). ALT is also set.
pub const RIGHT_ALT: u32 = 32;
/// Either Windows logo key held.
pub const LOGO: u32 = 64;
/// Lock states (toggled on make, never on repeat).
pub const CAPS_LOCK: u32 = 128;
pub const NUM_LOCK: u32 = 256;
pub const SCROLL_LOCK: u32 = 512;
pub const LEFT_SHIFT: u32 = 1024;
pub const RIGHT_SHIFT: u32 = 2048;
pub const LEFT_CONTROL: u32 = 4096;
pub const RIGHT_CONTROL: u32 = 8192;
pub const LEFT_ALT: u32 = 16384;
pub const LEFT_LOGO: u32 = 32768;
pub const RIGHT_LOGO: u32 = 65536;
pub const KEY_FLAGS: u32 = 0x1_ffff;
/// Flags the window server packs into legacy window key codes (bits 9..16,
/// `(flags & PACKED_FLAGS) << 9`); never reaches the ASCII byte at bit 16.
pub const PACKED_FLAGS: u32 = SHIFT | CONTROL | ALT | REPEAT | RIGHT_ALT | LOGO;
/// Make-only Korean keys (set-1 0xF1/0xF2 without a prefix).
pub const HANJA: u32 = 0xf1;
pub const HANGUL: u32 = 0xf2;
/// A KEY event's sided bits and REPEAT are consistent with its aggregates.
pub const fn key_flags_consistent(flags: u32) -> bool {
    flags & !KEY_FLAGS == 0
        && (flags & (LEFT_SHIFT | RIGHT_SHIFT) == 0 || flags & SHIFT != 0)
        && (flags & (LEFT_CONTROL | RIGHT_CONTROL) == 0 || flags & CONTROL != 0)
        && (flags & (LEFT_ALT | RIGHT_ALT) == 0 || flags & ALT != 0)
        && (flags & (LEFT_LOGO | RIGHT_LOGO) == 0 || flags & LOGO != 0)
        && (flags & REPEAT == 0 || flags & DOWN != 0)
}
/// Physical set-1 key codes a KEY event may carry.
pub const fn valid_key_code(code: u32) -> bool {
    matches!(code, 0x01..=0x7f | 0x101..=0x17f) || code == HANJA || code == HANGUL
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DisplayInfo {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
    pub capabilities: u64,
    pub max_copy: u64,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_transport_rejects_bad_extent_stride_format_and_capability() {
        let info = DisplayInfo {
            width: 1024,
            height: 768,
            stride: 1056,
            format: 1,
            capabilities: CPU_COPY | PARTIAL_COPY | POINTER,
            max_copy: MAX_COPY_BYTES as u64,
        };
        let mut b = [0; INFO_BYTES];
        info.encode(&mut b).unwrap();
        assert_eq!(DisplayInfo::decode(&b), Ok(info));
        for n in 0..INFO_BYTES {
            assert!(DisplayInfo::decode(&b[..n]).is_err());
        }
        for (offset, value) in [(0, 0), (4, 2161), (8, 1), (12, 2)] {
            let mut changed = b;
            put32(&mut changed, offset, value);
            assert!(DisplayInfo::decode(&changed).is_err());
        }
        // Bit 3 is WHEEL; the next undefined capability bit is rejected.
        let mut changed = b;
        put64(&mut changed, 16, 16);
        assert!(DisplayInfo::decode(&changed).is_err());
    }
    #[test]
    fn key_events_carry_sided_modifiers_locks_and_repeat_consistently() {
        let full = Input {
            kind: KEY,
            flags: DOWN
                | REPEAT
                | SHIFT
                | RIGHT_SHIFT
                | ALT
                | RIGHT_ALT
                | CONTROL
                | LEFT_CONTROL
                | LOGO
                | LEFT_LOGO
                | CAPS_LOCK
                | NUM_LOCK
                | SCROLL_LOCK,
            code: 0x11,
            text: u32::from(b'w'),
            timestamp: 7,
            ..Input::default()
        };
        let mut b = [0; INPUT_BYTES];
        full.encode(&mut b).unwrap();
        assert_eq!(Input::decode(&b), Ok(full));
        assert_eq!(full.flags & PACKED_FLAGS, full.flags & 126);
        assert!(
            (full.flags & PACKED_FLAGS) << 9 < 1 << 16,
            "packing never reaches text"
        );
        for flags in [
            RIGHT_SHIFT,
            LEFT_CONTROL,
            RIGHT_ALT,
            RIGHT_LOGO,
            REPEAT,
            KEY_FLAGS + 1,
        ] {
            let mut bad = b;
            put32(&mut bad, 4, flags);
            assert!(Input::decode(&bad).is_err(), "flags {flags:#x}");
        }
        for code in [0, 0x80, 0x100, 0x180, 0xf0, 0xf3, 512] {
            let mut bad = b;
            put32(&mut bad, 16, code);
            assert!(Input::decode(&bad).is_err(), "code {code:#x}");
        }
        for code in [HANJA, HANGUL, 0x15b, 0x7f] {
            let mut ok = b;
            put32(&mut ok, 16, code);
            assert!(Input::decode(&ok).is_ok(), "code {code:#x}");
        }
        let mut bad = b;
        put32(&mut bad, 8, 1);
        assert!(Input::decode(&bad).is_err(), "keys carry no position");
    }
    #[test]
    fn input_roundtrip_preserves_negative_motion_release_and_time() {
        for input in [
            Input {
                kind: RESET,
                timestamp: 9,
                ..Input::default()
            },
            Input {
                kind: MOTION,
                x: -255,
                y: 255,
                flags: 7,
                timestamp: u64::MAX,
                ..Input::default()
            },
            Input {
                kind: KEY,
                code: 0x148,
                flags: ALT,
                text: 0,
                timestamp: 4,
                ..Input::default()
            },
        ] {
            let mut b = [0; INPUT_BYTES];
            input.encode(&mut b).unwrap();
            assert_eq!(Input::decode(&b), Ok(input));
            for n in 0..INPUT_BYTES {
                assert!(Input::decode(&b[..n]).is_err());
            }
            put32(&mut b, 4, 0x8000);
            assert!(Input::decode(&b).is_err());
        }
    }
    #[test]
    fn wheel_motion_roundtrips_signed_detents_and_rejects_out_of_range() {
        for wheel in [-MAX_WHEEL, -1, 0, 1, MAX_WHEEL] {
            let input = Input {
                kind: MOTION,
                x: 3,
                y: -4,
                flags: 1,
                code: wheel as u32,
                timestamp: 5,
                ..Input::default()
            };
            let mut b = [0; INPUT_BYTES];
            input.encode(&mut b).unwrap();
            let decoded = Input::decode(&b).unwrap();
            assert_eq!(decoded, input);
            assert_eq!(decoded.wheel(), wheel);
        }
        for wheel in [MAX_WHEEL + 1, -MAX_WHEEL - 1, i32::MIN] {
            let mut b = [0; INPUT_BYTES];
            Input {
                kind: MOTION,
                code: wheel as u32,
                ..Input::default()
            }
            .encode(&mut b)
            .unwrap();
            assert!(Input::decode(&b).is_err());
        }
        let key = Input {
            kind: KEY,
            code: 30,
            ..Input::default()
        };
        assert_eq!(key.wheel(), 0);
        let info = DisplayInfo {
            width: 1024,
            height: 768,
            stride: 1024,
            format: 1,
            capabilities: CPU_COPY | PARTIAL_COPY | POINTER | WHEEL,
            max_copy: MAX_COPY_BYTES as u64,
        };
        let mut b = [0; INFO_BYTES];
        info.encode(&mut b).unwrap();
        assert_eq!(DisplayInfo::decode(&b), Ok(info));
    }
    #[test]
    fn display_scale_hint_roundtrips_only_windows_scales() {
        let base = DisplayInfo {
            width: 2560,
            height: 1600,
            stride: 2560,
            format: 1,
            capabilities: CPU_COPY | PARTIAL_COPY | POINTER,
            max_copy: MAX_COPY_BYTES as u64,
        };
        assert_eq!(base.scale_hint(), 0);
        for percent in [100, 125, 150, 175, 200, 225, 250, 275, 300] {
            let info = base.with_scale_hint(percent);
            assert_eq!(info.scale_hint(), percent);
            let mut b = [0; INFO_BYTES];
            info.encode(&mut b).unwrap();
            assert_eq!(DisplayInfo::decode(&b), Ok(info));
        }
        for (text, hint) in [
            (&b"150"[..], 150),
            (b"125\n", 125),
            (b"300", 300),
            (b"100", 100),
            (b"0", 0),
            (b"130", 0),
            (b"1500", 0),
            (b"", 0),
            (b"15 0", 0),
            (b"-150", 0),
            (b"150\n\n", 0),
        ] {
            assert_eq!(scale_hint_from_text(text), hint, "{text:?}");
        }
        // Invalid hints never enter the capabilities; forged ones fail decode.
        assert_eq!(base.with_scale_hint(130).scale_hint(), 0);
        assert_eq!(base.with_scale_hint(150).with_scale_hint(0).scale_hint(), 0);
        for forged in [130u64, 75, 325, 0xffff] {
            let mut b = [0; INFO_BYTES];
            DisplayInfo {
                capabilities: base.capabilities | forged << SCALE_HINT_SHIFT,
                ..base
            }
            .encode(&mut b)
            .unwrap();
            assert!(DisplayInfo::decode(&b).is_err(), "{forged}");
        }
        // Bits above the hint stay reserved.
        let mut b = [0; INFO_BYTES];
        DisplayInfo {
            capabilities: base.capabilities | 1 << 48,
            ..base
        }
        .encode(&mut b)
        .unwrap();
        assert!(DisplayInfo::decode(&b).is_err());
    }
}
impl DisplayInfo {
    /// Host display-scale hint in percent, 0 when absent.
    pub const fn scale_hint(&self) -> u32 {
        ((self.capabilities & SCALE_HINT_MASK) >> SCALE_HINT_SHIFT) as u32
    }
    /// Capabilities carrying a validated scale hint (0 clears it).
    pub const fn with_scale_hint(mut self, percent: u32) -> Self {
        self.capabilities &= !SCALE_HINT_MASK;
        if valid_scale_hint(percent) {
            self.capabilities |= (percent as u64) << SCALE_HINT_SHIFT;
        }
        self
    }
    pub fn encode(self, b: &mut [u8]) -> Result<(), Status> {
        if b.len() != INFO_BYTES {
            return Err(Status::InvalidArgument);
        }
        for (i, v) in [self.width, self.height, self.stride, self.format]
            .into_iter()
            .enumerate()
        {
            put32(b, i * 4, v);
        }
        put64(b, 16, self.capabilities);
        put64(b, 24, self.max_copy);
        Ok(())
    }
    pub fn decode(b: &[u8]) -> Result<Self, Status> {
        if b.len() != INFO_BYTES {
            return Err(Status::InvalidArgument);
        }
        let v = Self {
            width: get32(b, 0),
            height: get32(b, 4),
            stride: get32(b, 8),
            format: get32(b, 12),
            capabilities: get64(b, 16),
            max_copy: get64(b, 24),
        };
        if v.width == 0
            || v.height == 0
            || v.width > 3840
            || v.height > 2160
            || v.stride < v.width
            || v.format > 1
            || v.max_copy != MAX_COPY_BYTES as u64
            || v.capabilities & !(CPU_COPY | PARTIAL_COPY | POINTER | WHEEL | SCALE_HINT_MASK) != 0
            || !valid_scale_hint(v.scale_hint())
        {
            return Err(Status::Unsupported);
        }
        Ok(v)
    }
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Input {
    pub kind: u32,
    pub flags: u32,
    pub x: i32,
    pub y: i32,
    pub code: u32,
    pub text: u32,
    pub timestamp: u64,
}
impl Input {
    /// Signed wheel detents of a MOTION event (> 0 = away from the user).
    pub fn wheel(&self) -> i32 {
        if self.kind == MOTION {
            self.code as i32
        } else {
            0
        }
    }
    pub fn encode(self, b: &mut [u8]) -> Result<(), Status> {
        if b.len() != INPUT_BYTES {
            return Err(Status::InvalidArgument);
        }
        for (i, v) in [
            self.kind,
            self.flags,
            self.x as u32,
            self.y as u32,
            self.code,
            self.text,
        ]
        .into_iter()
        .enumerate()
        {
            put32(b, i * 4, v)
        }
        put64(b, 24, self.timestamp);
        Ok(())
    }
    pub fn decode(b: &[u8]) -> Result<Self, Status> {
        if b.len() != INPUT_BYTES {
            return Err(Status::InvalidArgument);
        }
        let v = Self {
            kind: get32(b, 0),
            flags: get32(b, 4),
            x: get32(b, 8) as i32,
            y: get32(b, 12) as i32,
            code: get32(b, 16),
            text: get32(b, 20),
            timestamp: get64(b, 24),
        };
        if ![KEY, MOTION, RESET].contains(&v.kind)
            || (v.kind == KEY
                && (!key_flags_consistent(v.flags)
                    || !valid_key_code(v.code)
                    || v.text > 127
                    || v.x != 0
                    || v.y != 0))
            || (v.kind == MOTION
                && (v.flags & !7 != 0
                    || !(-MAX_WHEEL..=MAX_WHEEL).contains(&(v.code as i32))
                    || v.text != 0))
            || (v.kind == RESET
                && (v.flags != 0 || v.x != 0 || v.y != 0 || v.code != 0 || v.text != 0))
        {
            return Err(Status::InvalidArgument);
        }
        Ok(v)
    }
}
