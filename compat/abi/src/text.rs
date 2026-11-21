//! Pure text and arithmetic helpers of kernel32 (`MulDiv`, UTF-8 code page).

/// `MulDiv`: `(a * b) / c` with a 64-bit intermediate, rounded half away
/// from zero; -1 when `c` is zero or the result does not fit in 32 bits.
pub fn mul_div(a: i32, b: i32, c: i32) -> i32 {
    if c == 0 {
        return -1;
    }
    let product = i64::from(a) * i64::from(b);
    let divisor = i64::from(c);
    let magnitude = (product.unsigned_abs() + divisor.unsigned_abs() / 2) / divisor.unsigned_abs();
    let negative = (product < 0) != (divisor < 0);
    let value = if negative {
        -(magnitude as i128)
    } else {
        magnitude as i128
    };
    i32::try_from(value).unwrap_or(-1)
}

/// Incremental UTF-8 decoder with maximal-subpart error recovery (each
/// ill-formed subsequence becomes one replacement), as Windows' UTF-8
/// `MultiByteToWideChar` does.
#[derive(Clone, Copy, Debug, Default)]
pub struct Utf8Decoder {
    needed: u8,
    scalar: u32,
    lower: u8,
    upper: u8,
}
/// One decoding outcome: `Some(Some(c))` a scalar, `Some(None)` an
/// ill-formed subsequence, `None` nothing yet.
pub type Decoded = Option<Option<char>>;
impl Utf8Decoder {
    pub const fn new() -> Self {
        Self {
            needed: 0,
            scalar: 0,
            lower: 0x80,
            upper: 0xbf,
        }
    }
    /// A multi-byte sequence is incomplete.
    pub fn pending(&self) -> bool {
        self.needed != 0
    }
    /// Feed one byte. The first result completes or rejects the previous
    /// sequence; the second is the restarted byte after a rejection.
    pub fn push(&mut self, byte: u8) -> (Decoded, Decoded) {
        if self.needed == 0 {
            return (self.start(byte), None);
        }
        if !(self.lower..=self.upper).contains(&byte) {
            *self = Self::new();
            return (Some(None), self.start(byte));
        }
        self.scalar = (self.scalar << 6) | u32::from(byte & 0x3f);
        self.needed -= 1;
        self.lower = 0x80;
        self.upper = 0xbf;
        if self.needed != 0 {
            return (None, None);
        }
        let scalar = char::from_u32(self.scalar);
        *self = Self::new();
        (Some(scalar), None)
    }
    fn start(&mut self, byte: u8) -> Decoded {
        let (needed, scalar, lower, upper) = match byte {
            0x00..=0x7f => return Some(Some(char::from(byte))),
            0xc2..=0xdf => (1, byte & 0x1f, 0x80, 0xbf),
            0xe0 => (2, 0, 0xa0, 0xbf),
            0xed => (2, 0xd, 0x80, 0x9f),
            0xe1..=0xef => (2, byte & 0x0f, 0x80, 0xbf),
            0xf0 => (3, 0, 0x90, 0xbf),
            0xf4 => (3, 4, 0x80, 0x8f),
            0xf1..=0xf3 => (3, byte & 0x07, 0x80, 0xbf),
            _ => return Some(None),
        };
        *self = Self {
            needed,
            scalar: u32::from(scalar),
            lower,
            upper,
        };
        None
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::{string::String, vec::Vec};

    fn decode(bytes: &[u8]) -> String {
        let mut decoder = Utf8Decoder::new();
        let mut out = String::new();
        let mut take = |value: Decoded| {
            if let Some(value) = value {
                out.push(value.unwrap_or('\u{fffd}'));
            }
        };
        for &byte in bytes {
            let (first, second) = decoder.push(byte);
            take(first);
            take(second);
        }
        if decoder.pending() {
            take(Some(None));
        }
        out
    }

    #[test]
    fn mul_div_rounds_half_away_from_zero_and_reports_overflow() {
        assert_eq!(mul_div(1, 1, 2), 1);
        assert_eq!(mul_div(-1, 1, 2), -1);
        assert_eq!(mul_div(1, -1, -2), 1);
        assert_eq!(mul_div(10, 96, 72), 13);
        assert_eq!(mul_div(9, 96, 72), 12);
        assert_eq!(mul_div(i32::MAX, i32::MAX, i32::MAX), i32::MAX);
        assert_eq!(mul_div(i32::MAX, 2, 1), -1);
        assert_eq!(mul_div(5, 5, 0), -1);
        assert_eq!(mul_div(i32::MIN, 1, 1), i32::MIN);
    }

    #[test]
    fn utf8_decoding_matches_the_standard_library_including_errors() {
        for sample in [
            &b"plain"[..],
            "\u{3bb} \u{9580} \u{1f600}".as_bytes(),
            b"\xc0\xaf",
            b"\xe0\x80\xaf",
            b"\xed\xa0\x80",
            b"\xf4\x90\x80\x80",
            b"a\xe2\x82",
            b"\xe2\x82a",
            b"\xf0\x9f\x98",
            b"\xff\xfe",
        ] {
            assert_eq!(
                decode(sample),
                String::from_utf8_lossy(sample),
                "{sample:?}"
            );
        }
        let all: Vec<u8> = (0..=255u8).collect();
        assert_eq!(decode(&all), String::from_utf8_lossy(&all));
    }
}
