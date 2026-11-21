//! The "C" locale's character classes as the CRT's `_pctype` table holds
//! them (`__pctype_func`, `_isctype`): one u16 of `_UPPER _LOWER _DIGIT
//! _SPACE _PUNCT _CONTROL _BLANK _HEX _ALPHA` bits per byte, index -1
//! (EOF) first. Bytes above 0x7F have no class in the "C" locale.
pub const UPPER: u16 = 0x1;
pub const LOWER: u16 = 0x2;
pub const DIGIT: u16 = 0x4;
pub const SPACE: u16 = 0x8;
pub const PUNCT: u16 = 0x10;
pub const CONTROL: u16 = 0x20;
pub const BLANK: u16 = 0x40;
pub const HEX: u16 = 0x80;
pub const LEADBYTE: u16 = 0x8000;
pub const ALPHA: u16 = 0x0100 | UPPER | LOWER;

/// The class bits of byte `c` (0 to 255).
pub const fn class(c: u8) -> u16 {
    match c {
        b'A'..=b'F' => UPPER | HEX | 0x100,
        b'G'..=b'Z' => UPPER | 0x100,
        b'a'..=b'f' => LOWER | HEX | 0x100,
        b'g'..=b'z' => LOWER | 0x100,
        b'0'..=b'9' => DIGIT | HEX,
        b' ' => SPACE | BLANK,
        b'\t' => SPACE | CONTROL | BLANK,
        0x0A..=0x0D => SPACE | CONTROL,
        0..=0x1F | 0x7F => CONTROL,
        0x21..=0x2F | 0x3A..=0x40 | 0x5B..=0x60 | 0x7B..=0x7E => PUNCT,
        _ => 0,
    }
}
/// The table `__pctype_func` returns a pointer into (entry 0 is EOF; the
/// returned pointer is to entry 1, so `_pctype[-1]` is EOF).
pub static TABLE: [u16; 257] = {
    let mut t = [0u16; 257];
    let mut i = 0;
    while i < 256 {
        t[i + 1] = class(i as u8);
        i += 1;
    }
    t
};
/// The wide classes (`iswctype`, `_pwctype`): ASCII as above; beyond it the
/// letters, digits and spaces of the Unicode tables core knows.
pub fn wide_class(c: u16) -> u16 {
    if c < 128 {
        return class(c as u8);
    }
    let Some(ch) = char::from_u32(u32::from(c)) else {
        return 0;
    };
    let mut bits = 0;
    if ch.is_alphabetic() {
        bits |= 0x100;
        if ch.is_uppercase() {
            bits |= UPPER;
        }
        if ch.is_lowercase() {
            bits |= LOWER;
        }
    }
    if ch.is_numeric() && ch.is_ascii_digit() {
        bits |= DIGIT;
    }
    if ch.is_whitespace() {
        bits |= SPACE;
        if c == 0xA0 || c == 0x3000 || (0x2000..=0x200A).contains(&c) {
            bits |= BLANK;
        }
    }
    if ch.is_control() {
        bits |= CONTROL;
    }
    if bits == 0 && !ch.is_alphanumeric() && !ch.is_whitespace() && !ch.is_control() && c != 0xFFFF
    {
        bits |= PUNCT;
    }
    bits
}
/// towupper / towlower over the simple Unicode mappings.
pub fn upper(c: u16) -> u16 {
    if c < 128 {
        return u16::from((c as u8).to_ascii_uppercase());
    }
    single(char::from_u32(u32::from(c)).map(|ch| ch.to_uppercase()), c)
}
pub fn lower(c: u16) -> u16 {
    if c < 128 {
        return u16::from((c as u8).to_ascii_lowercase());
    }
    single(char::from_u32(u32::from(c)).map(|ch| ch.to_lowercase()), c)
}
fn single<I: Iterator<Item = char>>(mapped: Option<I>, c: u16) -> u16 {
    let Some(mut it) = mapped else { return c };
    match (it.next(), it.next()) {
        (Some(m), None) if (m as u32) < 0x10000 => m as u16,
        _ => c,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_c_locale_table() {
        assert_eq!(TABLE[0], 0);
        assert_eq!(TABLE[1 + b'A' as usize], 0x181);
        assert_eq!(TABLE[1 + b'z' as usize], 0x102);
        assert_eq!(TABLE[1 + b'7' as usize], 0x84);
        assert_eq!(TABLE[1 + b' ' as usize], 0x48);
        assert_eq!(TABLE[1 + b'\t' as usize], 0x68);
        assert_eq!(TABLE[1 + b'\n' as usize], 0x28);
        assert_eq!(TABLE[1 + b'!' as usize], 0x10);
        assert_eq!(TABLE[1 + 0xE9], 0);
    }
    #[test]
    fn wide_classes_and_case() {
        assert_eq!(wide_class(0xE9) & ALPHA, 0x102);
        assert_eq!(upper(0xE9), 0xC9);
        assert_eq!(lower(0x0130), 0x0130); // İ lowercases to two characters
        assert_eq!(upper(u16::from(b'q')), u16::from(b'Q'));
        assert_ne!(wide_class(0x3000) & SPACE, 0);
    }
}
