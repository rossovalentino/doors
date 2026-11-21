//! Numbers as oleaut32 reads and writes them for the en-US locale (LCID
//! 0x409, what Doors' sessions use): VarParseNumFromStr's grammar and
//! VarBstrFromR8/R4/Cy/Dec's formatting, measured on Windows 11.
use alloc::{string::String, vec::Vec};

use crate::hr;

/// A parsed number: decimal digits (no leading zeros), the power of ten to
/// scale them by, and the sign. Hexadecimal and octal literals are 32-bit
/// two's complement values (`&HFFFFFFFF` is -1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parsed {
    pub negative: bool,
    pub digits: Vec<u8>,
    pub exponent: i32,
}
impl Parsed {
    pub fn zero() -> Self {
        Self {
            negative: false,
            digits: Vec::new(),
            exponent: 0,
        }
    }
    fn from_i64(v: i64) -> Self {
        let mut digits: Vec<u8> = alloc::format!("{}", v.unsigned_abs())
            .bytes()
            .map(|b| b - b'0')
            .collect();
        while digits.first() == Some(&0) {
            digits.remove(0);
        }
        Self {
            negative: v < 0,
            digits,
            exponent: 0,
        }
    }
    /// The value as a double (correctly rounded through the text form).
    pub fn to_f64(&self) -> Result<f64, u32> {
        if self.digits.is_empty() {
            return Ok(if self.negative { -0.0 } else { 0.0 });
        }
        let mut s = String::new();
        if self.negative {
            s.push('-');
        }
        for d in &self.digits {
            s.push((b'0' + d) as char);
        }
        s.push_str(&alloc::format!("e{}", self.exponent));
        let v: f64 = s.parse().map_err(|_| hr::DISP_E_TYPEMISMATCH)?;
        if v.is_infinite() {
            return Err(hr::DISP_E_OVERFLOW);
        }
        Ok(v)
    }
    /// The value rounded to an integer (banker's rounding, as VarI4FromStr),
    /// within i128; None when it is far out of range.
    pub fn to_integer(&self) -> Option<i128> {
        if self.digits.is_empty() {
            return Some(0);
        }
        let int_len = self.digits.len() as i32 + self.exponent;
        if int_len > 30 {
            return None;
        }
        let mut whole: i128 = 0;
        for i in 0..int_len.max(0) {
            let d = self.digits.get(i as usize).copied().unwrap_or(0);
            whole = whole * 10 + i128::from(d);
        }
        // The fraction: compare with one half.
        let fraction: Vec<u8> = if int_len >= 0 {
            self.digits[(int_len as usize).min(self.digits.len())..].to_vec()
        } else {
            let mut f = alloc::vec![0u8; (-int_len) as usize];
            f.extend_from_slice(&self.digits);
            f
        };
        let half = match fraction.first() {
            None => core::cmp::Ordering::Less,
            Some(&d) if d < 5 => core::cmp::Ordering::Less,
            Some(&d) if d > 5 => core::cmp::Ordering::Greater,
            Some(_) => {
                if fraction[1..].iter().any(|d| *d != 0) {
                    core::cmp::Ordering::Greater
                } else {
                    core::cmp::Ordering::Equal
                }
            }
        };
        let up = match half {
            core::cmp::Ordering::Greater => true,
            core::cmp::Ordering::Equal => whole % 2 == 1,
            core::cmp::Ordering::Less => false,
        };
        if up {
            whole += 1;
        }
        Some(if self.negative { -whole } else { whole })
    }
}

fn space(u: u16) -> bool {
    matches!(u, 0x20 | 0x09 | 0x0A | 0x0B | 0x0C | 0x0D | 0xA0)
}
/// VarParseNumFromStr with the flags VarXxxFromStr use (NUMPRS_STD): white
/// space around it, a leading or trailing sign (once), parentheses for a
/// negative value, the currency symbol, thousands separators in the whole
/// part, one decimal point, an exponent (e or d), &H hexadecimal and &O
/// octal. DISP_E_TYPEMISMATCH otherwise.
pub fn parse(text: &[u16]) -> Result<Parsed, u32> {
    let mismatch = hr::DISP_E_TYPEMISMATCH;
    let mut s: &[u16] = text;
    while s.first().is_some_and(|u| space(*u)) {
        s = &s[1..];
    }
    while s.last().is_some_and(|u| space(*u)) {
        s = &s[..s.len() - 1];
    }
    let c = |u: u16| char::from_u32(u32::from(u)).unwrap_or('\u{fffd}');
    // &H / &O literals.
    if s.len() >= 2 && s[0] == u16::from(b'&') {
        let radix = match c(s[1]).to_ascii_uppercase() {
            'H' => 16,
            'O' => 8,
            _ => return Err(mismatch),
        };
        let digits = &s[2..];
        if digits.is_empty() {
            return Err(mismatch);
        }
        let mut v: u64 = 0;
        for &u in digits {
            let d = c(u).to_digit(radix).ok_or(mismatch)?;
            v = v * u64::from(radix) + u64::from(d);
            if v > 0xFFFF_FFFF {
                return Err(hr::DISP_E_OVERFLOW);
            }
        }
        return Ok(Parsed::from_i64(i64::from(v as u32 as i32)));
    }
    let mut negative = false;
    let mut signed = false;
    let mut paren = false;
    let mut currency = false;
    let mut i = 0;
    // Leading sign, currency symbol, parenthesis (any order, each once).
    loop {
        match s.get(i).map(|u| c(*u)) {
            Some('+' | '-') if !signed && !paren => {
                negative = s[i] == u16::from(b'-');
                signed = true;
                i += 1;
            }
            Some('$') if !currency => {
                currency = true;
                i += 1;
            }
            Some('(') if !paren && !signed => {
                paren = true;
                i += 1;
            }
            _ => break,
        }
    }
    let mut digits: Vec<u8> = Vec::new();
    let mut exponent: i32 = 0;
    let mut seen_digit = false;
    let mut point = false;
    while let Some(u) = s.get(i) {
        let ch = c(*u);
        if ch.is_ascii_digit() {
            let d = ch as u8 - b'0';
            seen_digit = true;
            if point {
                exponent -= 1;
            }
            digits.push(d);
            i += 1;
        } else if ch == ',' && !point && seen_digit {
            i += 1;
        } else if ch == '.' && !point {
            point = true;
            i += 1;
        } else {
            break;
        }
    }
    if !seen_digit {
        return Err(mismatch);
    }
    // Exponent.
    if let Some('e' | 'E' | 'd' | 'D') = s.get(i).map(|u| c(*u)) {
        i += 1;
        let mut minus = false;
        if let Some('+' | '-') = s.get(i).map(|u| c(*u)) {
            minus = s[i] == u16::from(b'-');
            i += 1;
        }
        let start = i;
        let mut e: i64 = 0;
        while let Some(d) = s.get(i).and_then(|u| c(*u).to_digit(10)) {
            e = (e * 10 + i64::from(d)).min(100_000);
            i += 1;
        }
        if i == start {
            return Err(mismatch);
        }
        exponent += if minus { -(e as i32) } else { e as i32 };
    }
    // Trailing currency symbol, sign, parenthesis.
    loop {
        match s.get(i).map(|u| c(*u)) {
            Some('+' | '-') if !signed && !paren => {
                negative = s[i] == u16::from(b'-');
                signed = true;
                i += 1;
            }
            Some('$') if !currency => {
                currency = true;
                i += 1;
            }
            Some(')') if paren => {
                paren = false;
                negative = true;
                i += 1;
            }
            _ => break,
        }
    }
    if paren || i != s.len() {
        return Err(mismatch);
    }
    // Normalise: no leading zeros, no trailing zeros.
    while digits.first() == Some(&0) {
        digits.remove(0);
    }
    while digits.last() == Some(&0) {
        digits.pop();
        exponent += 1;
    }
    if digits.is_empty() {
        exponent = 0;
    }
    Ok(Parsed {
        negative,
        digits,
        exponent,
    })
}

/// A double in shortest-roundtrip decimal digits and exponent
/// (value = 0.d1d2d3... × 10^point).
fn decimal_digits(v: f64) -> (Vec<u8>, i32) {
    let s = alloc::format!("{:e}", v.abs());
    let (mantissa, exp) = s.split_once('e').unwrap_or((s.as_str(), "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let digits: Vec<u8> = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    (digits, exp)
}
/// Digits of `v` rounded to `precision` significant digits (round half to
/// even on the exact binary value, as the C runtime's %.*G does), and the
/// decimal exponent of the first digit.
fn rounded(v: f64, precision: usize) -> (Vec<u8>, i32) {
    // %.{p-1}e prints the exact value rounded correctly.
    let s = alloc::format!("{:.*e}", precision - 1, v.abs());
    let (mantissa, exp) = s.split_once('e').unwrap_or((s.as_str(), "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let mut digits: Vec<u8> = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    while digits.len() > 1 && digits.last() == Some(&0) {
        digits.pop();
    }
    (digits, exp)
}
/// VarBstrFromR8 (15 significant digits) and VarBstrFromR4 (7): plain
/// digits while the value's decimal exponent is within (-precision,
/// precision), else d.dddE+xx.
pub fn format_real(v: f64, precision: usize) -> String {
    if v.is_nan() {
        return String::from("nan");
    }
    if v.is_infinite() {
        return String::from(if v < 0.0 { "-inf" } else { "inf" });
    }
    if v == 0.0 {
        return String::from("0");
    }
    let (digits, exponent) = rounded(v, precision);
    // Scientific when the value reaches 10^precision, or when its rounded
    // digits start at 10^-precision or below (Windows: 999999999999999.75
    // is "1000000000000000", 9.99999997E-7 as R4 "0.000001").
    let (_, raw_exponent) = decimal_digits(v);
    let mut out = String::new();
    if v < 0.0 {
        out.push('-');
    }
    let p = precision as i32;
    if raw_exponent >= p || exponent <= -p {
        out.push((b'0' + digits[0]) as char);
        if digits.len() > 1 {
            out.push('.');
            for d in &digits[1..] {
                out.push((b'0' + d) as char);
            }
        }
        out.push('E');
        out.push(if exponent < 0 { '-' } else { '+' });
        out.push_str(&alloc::format!("{:02}", exponent.unsigned_abs()));
        return out;
    }
    if exponent < 0 {
        out.push('0');
        out.push('.');
        for _ in 0..(-exponent - 1) {
            out.push('0');
        }
        for d in &digits {
            out.push((b'0' + d) as char);
        }
        return out;
    }
    let whole = exponent as usize + 1;
    for i in 0..whole.max(digits.len()) {
        if i == whole {
            out.push('.');
        }
        out.push((b'0' + digits.get(i).copied().unwrap_or(0)) as char);
    }
    out
}
/// A scaled integer (CY: scale 4; DECIMAL: 0..=28) as text: trailing zeros
/// of the fraction dropped.
pub fn format_scaled(negative: bool, magnitude: u128, scale: u32) -> String {
    let text = alloc::format!("{magnitude}");
    let scale = scale as usize;
    let (whole, fraction) = if text.len() > scale {
        let (w, f) = text.split_at(text.len() - scale);
        (String::from(w), String::from(f))
    } else {
        (
            String::from("0"),
            alloc::format!("{}{}", "0".repeat(scale - text.len()), text),
        )
    };
    let fraction = fraction.trim_end_matches('0');
    let mut out = String::new();
    if negative && magnitude != 0 {
        out.push('-');
    }
    out.push_str(&whole);
    if !fraction.is_empty() {
        out.push('.');
        out.push_str(fraction);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    fn bits(v: f64) -> String {
        v.to_le_bytes()
            .iter()
            .map(|b| alloc::format!("{b:02x}"))
            .collect()
    }
    #[test]
    fn real_formatting_matches_windows() {
        // VarBstrFromR8 / VarBstrFromR4 on Windows 11 (LCID 0x409).
        for (raw, text) in [
            (0x3f1a36e2eb1c432d_u64, "0.0001"),
            (0x3e7ad7f29abcaf48, "0.0000001"),
            (0x42d6bcc41e900000, "100000000000000"),
            (0x430c6bf526340000, "1E+15"),
            (0x430c6bf52633fffe, "1000000000000000"),
            (0x4341c37937e08000, "1E+16"),
            (0x43792dfb57e6ab32, "1.1339891124068E+17"),
            (0x3fd3333333333334, "0.3"),
            (0x3fe5555555555555, "0.666666666666667"),
            (0x41f0a8b1a8f3c3c6, "4471855759.2353"),
            (0xbe7ad7f29abcaf48, "-0.0000001"),
            (0x7fefffffffffffff, "1.79769313486232E+308"),
            (0x1, "4.94065645841247E-324"),
            (0x7ff8000000000000, "nan"),
            (0x7ff0000000000000, "inf"),
            (0x3ff0000000000001, "1"),
            (0x400921fb54442d18, "3.14159265358979"),
            (0x4058fffffffffffe, "100"),
            (0x412e847fffffffff, "1000000"),
            (0x46293e5939a08cea, "1E+30"),
            (0x408f400000000000, "1000"),
        ] {
            assert_eq!(format_real(f64::from_bits(raw), 15), text, "{raw:x}");
        }
        for (raw, text) in [
            (0x3dcccccd_u32, "0.1"),
            (0x4b189680, "1E+07"),
            (0x3f9e0652, "1.234568"),
            (0x7f7fffff, "3.402823E+38"),
            (0x358637bd, "0.000001"),
            (0x4479ffff, "999.9999"),
            (0x40490fdb, "3.141593"),
        ] {
            assert_eq!(
                format_real(f64::from(f32::from_bits(raw)), 7),
                text,
                "{raw:x}"
            );
        }
        // The small-number threshold: 1.5E-15 and 1.5E-07 are scientific.
        assert_eq!(format_real(1.5e-14, 15), "0.000000000000015");
        assert_eq!(format_real(1.5e-15, 15), "1.5E-15");
        assert_eq!(format_real(f64::from(1.5e-6_f32), 7), "0.0000015");
        assert_eq!(format_real(f64::from(1.5e-7_f32), 7), "1.5E-07");
        assert_eq!(format_scaled(true, 12345, 4), "-1.2345");
        assert_eq!(format_scaled(false, 50000, 4), "5");
        assert_eq!(format_scaled(false, 12345678, 4), "1234.5678");
    }
    #[test]
    fn parsing_matches_windows() {
        // VarR8FromStr / VarI4FromStr on Windows 11 (LCID 0x409).
        for (text, r8, i4) in [
            ("1.5e-7", Some(0x3e8421f5f40d8376_u64), Some(0)),
            ("  -12.5  ", Some(0xc029000000000000), Some(-12)),
            ("+3", Some(0x4008000000000000), Some(3)),
            ("3-", Some(0xc008000000000000), Some(-3)),
            ("1,2,3", Some(0x405ec00000000000), Some(123)),
            ("1.2.3", None, None),
            ("&O17", Some(0x402e000000000000), Some(15)),
            ("&HFFFF", Some(0x40efffe000000000), Some(65535)),
            ("&HFFFFFFFF", Some(0xbff0000000000000), Some(-1)),
            ("0x10", None, None),
            ("1d3", Some(0x408f400000000000), Some(1000)),
            (".5", Some(0x3fe0000000000000), Some(0)),
            ("5.", Some(0x4014000000000000), Some(5)),
            ("(1,000.25)", Some(0xc08f420000000000), Some(-1000)),
            ("$-5", Some(0xc014000000000000), Some(-5)),
            ("-$5", Some(0xc014000000000000), Some(-5)),
            ("1 000", None, None),
            ("\u{a0}5", Some(0x4014000000000000), Some(5)),
            ("\u{661}", None, None),
            ("&H80000000", Some(0xc1e0000000000000), Some(i32::MIN)),
            ("e3", None, None),
            ("1e", None, None),
            ("--1", None, None),
            ("+-1", None, None),
            ("1e-400", Some(0), Some(0)),
        ] {
            let p = parse(&w(text));
            let r = p
                .as_ref()
                .ok()
                .and_then(|p| p.to_f64().ok())
                .map(f64::to_bits);
            assert_eq!(
                r,
                r8,
                "{text} -> {}",
                r.map_or(String::new(), |b| bits(f64::from_bits(b)))
            );
            let i = p
                .as_ref()
                .ok()
                .and_then(|p| p.to_integer())
                .and_then(|v| i32::try_from(v).ok());
            assert_eq!(i, i4, "{text}");
        }
        assert_eq!(
            parse(&w("1e400")).unwrap().to_f64(),
            Err(hr::DISP_E_OVERFLOW)
        );
    }
}
