//! Lossless typed value editing, including embedded NUL and unpaired UTF-16.
extern crate alloc;
use alloc::{string::String, vec::Vec};
use core::fmt::Write;
use door_service_client::RegistryValue;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Invalid {
    Type,
    Number,
    Hex,
    Escape,
    TooLong,
}
pub const KINDS: [u8; 7] = [1, 2, 3, 4, 7, 11, 0];
pub fn label(kind: u8) -> &'static str {
    match kind {
        0 => "REG_NONE",
        1 => "REG_SZ",
        2 => "REG_EXPAND_SZ",
        3 => "REG_BINARY",
        4 => "REG_DWORD",
        7 => "REG_MULTI_SZ",
        11 => "REG_QWORD",
        _ => "Unknown",
    }
}
pub fn format(value: RegistryValue) -> String {
    let bytes = value.data();
    if RegistryValue::new(value.kind, bytes).is_err() {
        return String::from("Invalid value");
    }
    let mut out = String::new();
    match value.kind {
        4 => {
            let _ = write!(
                out,
                "0x{:08X}",
                u32::from_le_bytes(bytes.try_into().unwrap())
            );
        }
        11 => {
            let _ = write!(
                out,
                "0x{:016X}",
                u64::from_le_bytes(bytes.try_into().unwrap())
            );
        }
        1 | 2 | 7 => {
            let tail = if value.kind == 7 { 4 } else { 2 };
            for p in bytes[..bytes.len() - tail].chunks_exact(2) {
                let u = u16::from_le_bytes(p.try_into().unwrap());
                match u {
                    0 => out.push_str("\\0"),
                    9 => out.push_str("\\t"),
                    10 => out.push_str("\\n"),
                    13 => out.push_str("\\r"),
                    92 => out.push_str("\\\\"),
                    32..=126 => out.push(u as u8 as char),
                    _ => {
                        let _ = write!(out, "\\u{u:04X}");
                    }
                }
            }
        }
        _ => {
            for (i, b) in bytes.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                let _ = write!(out, "{b:02X}");
            }
        }
    }
    out
}
pub fn parse(kind: u8, text: &str) -> Result<RegistryValue, Invalid> {
    if text.len() > 512 {
        return Err(Invalid::TooLong);
    }
    let mut bytes = Vec::new();
    match kind {
        4 | 11 => {
            let text = text.trim();
            let number =
                if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
                    if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                        return Err(Invalid::Number);
                    }
                    u64::from_str_radix(hex, 16).map_err(|_| Invalid::Number)?
                } else {
                    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
                        return Err(Invalid::Number);
                    }
                    text.parse::<u64>().map_err(|_| Invalid::Number)?
                };
            if kind == 4 {
                bytes.extend_from_slice(
                    &u32::try_from(number)
                        .map_err(|_| Invalid::Number)?
                        .to_le_bytes(),
                );
            } else {
                bytes.extend_from_slice(&number.to_le_bytes());
            }
        }
        0 | 3 => {
            let hex: Vec<_> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
            if !hex.len().is_multiple_of(2) {
                return Err(Invalid::Hex);
            }
            if hex.len() > 128 {
                return Err(Invalid::TooLong);
            }
            for pair in hex.chunks_exact(2) {
                let a = (pair[0] as char).to_digit(16).ok_or(Invalid::Hex)?;
                let b = (pair[1] as char).to_digit(16).ok_or(Invalid::Hex)?;
                bytes.push((a * 16 + b) as u8);
            }
        }
        1 | 2 | 7 => {
            let mut chars = text.chars();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    let unit = match chars.next().ok_or(Invalid::Escape)? {
                        '\\' => 92,
                        '0' => 0,
                        'n' => 10,
                        'r' => 13,
                        't' => 9,
                        'u' => {
                            let mut unit = 0;
                            for _ in 0..4 {
                                unit = unit * 16
                                    + chars
                                        .next()
                                        .and_then(|c| c.to_digit(16))
                                        .ok_or(Invalid::Escape)?
                                        as u16;
                            }
                            unit
                        }
                        _ => return Err(Invalid::Escape),
                    };
                    bytes.extend_from_slice(&unit.to_le_bytes());
                } else {
                    for unit in c.encode_utf16(&mut [0; 2]) {
                        bytes.extend_from_slice(&unit.to_le_bytes());
                    }
                }
                if bytes.len() > 60 + (if kind == 7 { 0 } else { 2 }) {
                    return Err(Invalid::TooLong);
                }
            }
            bytes.extend_from_slice(&[0; 2]);
            if kind == 7 {
                bytes.extend_from_slice(&[0; 2]);
            }
        }
        _ => return Err(Invalid::Type),
    }
    RegistryValue::new(kind, &bytes).map_err(|_| Invalid::TooLong)
}
pub fn empty(kind: u8) -> Result<RegistryValue, Invalid> {
    RegistryValue::new(
        kind,
        match kind {
            1 | 2 => &[0; 2],
            7 => &[0; 4],
            4 => &[0; 4],
            11 => &[0; 8],
            _ => &[],
        },
    )
    .map_err(|_| Invalid::Type)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_supported_type_roundtrips_exact_bytes_without_expanding_environment_or_unicode() {
        for kind in KINDS {
            let empty = empty(kind).unwrap();
            assert_eq!(parse(kind, &format(empty)), Ok(empty));
        }
        for value in [
            RegistryValue::dword(u32::MAX),
            RegistryValue::new(11, &u64::MAX.to_le_bytes()).unwrap(),
            RegistryValue::new(3, &[0xa5; 64]).unwrap(),
            RegistryValue::new(0, &[0xff; 64]).unwrap(),
            RegistryValue::new(1, &[0, 0xd8, 0, 0, 92, 0, 10, 0, 0, 0]).unwrap(),
            RegistryValue::new(2, &[37, 0, 65, 0, 37, 0, 0, 0]).unwrap(),
            RegistryValue::new(7, &[65, 0, 0, 0, 0, 0, 0, 0]).unwrap(),
        ] {
            assert_eq!(parse(value.kind, &format(value)), Ok(value));
        }
        assert_eq!(
            parse(4, "4294967295").unwrap(),
            RegistryValue::dword(u32::MAX)
        );
        assert_eq!(
            parse(1, "\\uD800\\0\\\\").unwrap().data(),
            &[0, 0xd8, 0, 0, 92, 0, 0, 0]
        );
        assert_eq!(parse(3, "00 ff\n A5").unwrap().data(), &[0, 255, 165]);
    }
    #[test]
    fn malformed_or_oversized_edits_cannot_truncate_or_wrap() {
        for text in ["-1", "+1", "4294967296", "0x100000000", "0x", "1e2", ""] {
            assert!(parse(4, text).is_err(), "{text}");
        }
        assert!(parse(11, "18446744073709551616").is_err());
        for text in ["\\", "\\x00", "\\uD80", "\\uZZZZ"] {
            assert_eq!(parse(1, text), Err(Invalid::Escape));
        }
        for text in ["0", "0x01", "FF GG", "é"] {
            assert!(parse(3, text).is_err());
        }
        assert_eq!(parse(3, &"ff".repeat(65)), Err(Invalid::TooLong));
        assert!(parse(1, &"A".repeat(31)).is_ok());
        assert_eq!(parse(1, &"A".repeat(32)), Err(Invalid::TooLong));
        assert!(parse(7, &"A".repeat(30)).is_ok());
        assert_eq!(parse(7, &"A".repeat(31)), Err(Invalid::TooLong));
        assert_eq!(parse(5, "00"), Err(Invalid::Type));
    }
}
