//! Bounded PS/2 motion decoder; no host cursor dependency. A standard mouse
//! sends three-byte packets; an IntelliMouse (device ID 3, enabled by the
//! 200/100/80 sample-rate sequence) appends a fourth signed wheel byte, and an
//! IntelliMouse Explorer (ID 4) carries a four-bit wheel in its low nibble.
use door_abi::graphics::{Input, MOTION};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Protocol {
    #[default]
    Standard,
    /// IMPS/2: byte 3 is a signed wheel delta (negative = wheel up).
    Wheel,
    /// IMEX: bits 0..4 of byte 3 are a signed four-bit wheel delta.
    Explorer,
}
impl Protocol {
    /// Device ID reported by GET ID (0xF2) after the IntelliMouse sequence.
    pub const fn from_device_id(id: u8) -> Self {
        match id {
            3 => Self::Wheel,
            4 => Self::Explorer,
            _ => Self::Standard,
        }
    }
    pub const fn has_wheel(self) -> bool {
        !matches!(self, Self::Standard)
    }
    const fn packet_bytes(self) -> usize {
        match self {
            Self::Standard => 3,
            _ => 4,
        }
    }
}
#[derive(Default)]
pub struct Decoder {
    bytes: [u8; 4],
    count: usize,
    protocol: Protocol,
}
impl Decoder {
    pub const fn new() -> Self {
        Self::with_protocol(Protocol::Standard)
    }
    pub const fn with_protocol(protocol: Protocol) -> Self {
        Self {
            bytes: [0; 4],
            count: 0,
            protocol,
        }
    }
    pub const fn protocol(&self) -> Protocol {
        self.protocol
    }
    /// Feed one auxiliary-port byte. A completed packet yields one MOTION
    /// event; its `code` is the signed wheel delta in detents (> 0 = up).
    pub fn feed(&mut self, byte: u8) -> Option<Input> {
        if self.count == 0 && byte & 8 == 0 {
            return None;
        }
        self.bytes[self.count] = byte;
        self.count += 1;
        if self.count < self.protocol.packet_bytes() {
            return None;
        }
        self.count = 0;
        let [flags, x, y, z] = self.bytes;
        let overflow = flags & 0xc0 != 0;
        let dx = if overflow {
            0
        } else {
            i32::from(x) - if flags & 16 != 0 { 256 } else { 0 }
        };
        let dy = if overflow {
            0
        } else {
            -(i32::from(y) - if flags & 32 != 0 { 256 } else { 0 })
        };
        // Device positive Z is "toward the user"; the ABI reports detents
        // away from the user (scroll up) as positive.
        let dz = match self.protocol {
            Protocol::Standard => 0,
            Protocol::Wheel => i32::from(z as i8),
            Protocol::Explorer => i32::from(((z << 4) as i8) >> 4),
        };
        Some(Input {
            kind: MOTION,
            flags: u32::from(flags & 7),
            x: dx,
            y: dy,
            code: (-dz).clamp(-127, 127) as u32,
            ..Input::default()
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_motion_resynchronization_and_overflow_keep_buttons() {
        let mut d = Decoder::new();
        assert!(d.feed(0).is_none());
        assert!(d.feed(0x39).is_none());
        assert!(d.feed(255).is_none());
        let e = d.feed(254).unwrap();
        assert_eq!((e.x, e.y, e.flags, e.code), (-1, 2, 1, 0));
        d.feed(0xc9);
        d.feed(255);
        let e = d.feed(255).unwrap();
        assert_eq!((e.x, e.y, e.flags), (0, 0, 1));
    }
    #[test]
    fn intellimouse_packets_carry_signed_wheel_detents_and_resynchronize() {
        assert_eq!(Protocol::from_device_id(0), Protocol::Standard);
        assert_eq!(Protocol::from_device_id(3), Protocol::Wheel);
        assert_eq!(Protocol::from_device_id(4), Protocol::Explorer);
        assert!(!Protocol::from_device_id(9).has_wheel());
        let mut d = Decoder::with_protocol(Protocol::Wheel);
        // Junk before a synchronized header is ignored.
        assert!(d.feed(0x00).is_none());
        // Wheel up one detent (device Z = -1), no motion, left button held.
        for byte in [0x09, 0, 0] {
            assert!(d.feed(byte).is_none());
        }
        let e = d.feed(0xff).unwrap();
        assert_eq!(
            (e.kind, e.x, e.y, e.flags, e.code as i32),
            (MOTION, 0, 0, 1, 1)
        );
        // Wheel down three detents together with signed motion.
        for byte in [0x18, 0xfe, 0x03] {
            assert!(d.feed(byte).is_none());
        }
        let e = d.feed(3).unwrap();
        assert_eq!((e.x, e.y, e.code as i32), (-2, -3, -3));
        // Overflowed X/Y never suppresses the independent wheel byte.
        for byte in [0xc8, 0x10, 0x20] {
            d.feed(byte);
        }
        let e = d.feed(0x80).unwrap();
        assert_eq!((e.x, e.y, e.code as i32), (0, 0, 127));
        // IMEX keeps buttons 4/5 in the high nibble; only the low nibble is Z.
        let mut d = Decoder::with_protocol(Protocol::Explorer);
        for byte in [0x08, 1, 1] {
            d.feed(byte);
        }
        let e = d.feed(0x3f).unwrap();
        assert_eq!((e.x, e.y, e.code as i32), (1, -1, 1));
        for byte in [0x08, 0, 0] {
            d.feed(byte);
        }
        assert_eq!(d.feed(0x02).unwrap().code as i32, -2);
        // A standard decoder still completes on the third byte.
        let mut d = Decoder::new();
        d.feed(0x08);
        d.feed(5);
        let e = d.feed(0).unwrap();
        assert_eq!((e.x, e.code), (5, 0));
    }
}
