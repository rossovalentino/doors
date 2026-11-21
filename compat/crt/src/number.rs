//! strtol and strtod's parsing, over units (narrow bytes or UTF-16), with
//! the C library's rules: leading white space, a sign, base prefixes
//! ("0x" for base 16 or 0, "0" for base 0), overflow clamped with ERANGE,
//! and the end position for `endptr` (the start when nothing converted).

/// The result of an integer conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Integer {
    /// The magnitude read (saturated at u64::MAX on overflow).
    pub magnitude: u64,
    pub negative: bool,
    /// Units consumed (0 when no conversion was performed).
    pub end: usize,
    pub overflow: bool,
}

pub const fn is_space(u: u16) -> bool {
    matches!(u, 0x20 | 0x09..=0x0D)
}

fn digit(u: u16) -> Option<u32> {
    match u {
        0x30..=0x39 => Some(u32::from(u - 0x30)),
        0x41..=0x5A => Some(u32::from(u - 0x41) + 10),
        0x61..=0x7A => Some(u32::from(u - 0x61) + 10),
        _ => None,
    }
}

/// Parse an integer in `base` (0, or 2 to 36) from `text` (`get(i)`
/// returns unit `i`, 0 past the end).
pub fn integer(get: &dyn Fn(usize) -> u16, base: u32) -> Option<Integer> {
    if base == 1 || base > 36 {
        return None;
    }
    let mut i = 0;
    while is_space(get(i)) {
        i += 1;
    }
    let mut negative = false;
    if get(i) == u16::from(b'+') || get(i) == u16::from(b'-') {
        negative = get(i) == u16::from(b'-');
        i += 1;
    }
    let mut base = base;
    let x = |u: u16| u == u16::from(b'x') || u == u16::from(b'X');
    if (base == 0 || base == 16)
        && get(i) == u16::from(b'0')
        && x(get(i + 1))
        && digit(get(i + 2)).is_some_and(|d| d < 16)
    {
        base = 16;
        i += 2;
    } else if base == 0 {
        base = if get(i) == u16::from(b'0') { 8 } else { 10 };
    }
    let start = i;
    let mut magnitude = 0u64;
    let mut overflow = false;
    while let Some(d) = digit(get(i)).filter(|d| *d < base) {
        match magnitude
            .checked_mul(u64::from(base))
            .and_then(|m| m.checked_add(u64::from(d)))
        {
            Some(m) => magnitude = m,
            None => {
                overflow = true;
                magnitude = u64::MAX;
            }
        }
        i += 1;
    }
    if i == start {
        return Some(Integer {
            magnitude: 0,
            negative: false,
            end: 0,
            overflow: false,
        });
    }
    Some(Integer {
        magnitude,
        negative,
        end: i,
        overflow,
    })
}

/// strtol-style clamping of `r` into a signed range of `bits` bits:
/// (value, out of range).
pub fn signed(r: Integer, bits: u32) -> (i64, bool) {
    let max = (1u64 << (bits - 1)) - 1;
    if r.negative {
        if r.overflow || r.magnitude > max + 1 {
            return (-(max as i64) - 1, true);
        }
        ((r.magnitude as i64).wrapping_neg(), false)
    } else {
        if r.overflow || r.magnitude > max {
            return (max as i64, true);
        }
        (r.magnitude as i64, false)
    }
}
/// strtoul-style: a negative value wraps (as C specifies), out of range
/// gives the maximum.
pub fn unsigned(r: Integer, bits: u32) -> (u64, bool) {
    let max = if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    if r.overflow || r.magnitude > max {
        return (max, true);
    }
    let v = if r.negative {
        r.magnitude.wrapping_neg() & max
    } else {
        r.magnitude
    };
    (v, false)
}

/// strtod: (value, units consumed, out of range). Decimal and hexadecimal
/// floats, "inf"/"infinity" and "nan"/"nan(...)", case-insensitively.
pub fn float(get: &dyn Fn(usize) -> u16) -> (f64, usize, bool) {
    let mut i = 0;
    while is_space(get(i)) {
        i += 1;
    }
    let mut negative = false;
    if get(i) == u16::from(b'+') || get(i) == u16::from(b'-') {
        negative = get(i) == u16::from(b'-');
        i += 1;
    }
    let lower = |u: u16| u8::try_from(u).map_or(0, |b| b.to_ascii_lowercase());
    let word = |at: usize, w: &[u8]| w.iter().enumerate().all(|(k, &c)| lower(get(at + k)) == c);
    let sign = |v: f64| if negative { -v } else { v };
    if word(i, b"inf") {
        let n = if word(i, b"infinity") { 8 } else { 3 };
        return (sign(f64::INFINITY), i + n, false);
    }
    if word(i, b"nan") {
        let mut n = i + 3;
        if get(n) == u16::from(b'(') {
            let mut k = n + 1;
            while get(k) != 0 && (get(k) == u16::from(b'_') || digit(get(k)).is_some()) {
                k += 1;
            }
            if get(k) == u16::from(b')') {
                n = k + 1;
            }
        }
        return (sign(f64::NAN), n, false);
    }
    if get(i) == u16::from(b'0') && (lower(get(i + 1)) == b'x') {
        if let Some((v, end)) = hex_float(get, i + 2) {
            return (sign(v), end, v.is_infinite());
        }
        // "0x" with nothing after: the "0" alone converts.
        return (sign(0.0), i + 1, false);
    }
    // Decimal: collect the digits into a buffer core's parser accepts.
    let mut text = [0u8; 800];
    let mut n = 0;
    let mut digits = 0;
    let start = i;
    let mut push = |c: u8, n: &mut usize| {
        if *n < text.len() {
            text[*n] = c;
            *n += 1;
        }
    };
    while let Some(d) = digit(get(i)).filter(|d| *d < 10) {
        push(b'0' + d as u8, &mut n);
        digits += 1;
        i += 1;
    }
    if get(i) == u16::from(b'.') {
        push(b'.', &mut n);
        i += 1;
        while let Some(d) = digit(get(i)).filter(|d| *d < 10) {
            push(b'0' + d as u8, &mut n);
            digits += 1;
            i += 1;
        }
    }
    if digits == 0 {
        return (0.0, 0, false);
    }
    let _ = start;
    if lower(get(i)) == b'e' {
        let mut k = i + 1;
        let mut exponent = [0u8; 24];
        let mut m = 0;
        if get(k) == u16::from(b'+') || get(k) == u16::from(b'-') {
            exponent[m] = get(k) as u8;
            m += 1;
            k += 1;
        }
        let first = k;
        while let Some(d) = digit(get(k)).filter(|d| *d < 10) {
            if m < exponent.len() {
                exponent[m] = b'0' + d as u8;
                m += 1;
            }
            k += 1;
        }
        if k > first {
            push(b'e', &mut n);
            for &c in &exponent[..m] {
                push(c, &mut n);
            }
            i = k;
        }
    }
    let v = core::str::from_utf8(&text[..n])
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let range = v.is_infinite()
        || (v == 0.0
            && text[..n].iter().any(|&c| (b'1'..=b'9').contains(&c))
            && !mantissa_zero(&text[..n]));
    (sign(v), i, range)
}
fn mantissa_zero(text: &[u8]) -> bool {
    text.iter()
        .take_while(|&&c| c != b'e')
        .all(|&c| c == b'0' || c == b'.')
}

/// The digits of a hexadecimal float after "0x": (value, end).
fn hex_float(get: &dyn Fn(usize) -> u16, mut i: usize) -> Option<(f64, usize)> {
    let mut mantissa = 0u64;
    let mut exponent = 0i64;
    let mut digits = 0;
    let mut sticky = false;
    while let Some(d) = digit(get(i)).filter(|d| *d < 16) {
        if mantissa >> 60 == 0 {
            mantissa = mantissa << 4 | u64::from(d);
        } else {
            exponent += 4;
            sticky |= d != 0;
        }
        digits += 1;
        i += 1;
    }
    if get(i) == u16::from(b'.') {
        i += 1;
        while let Some(d) = digit(get(i)).filter(|d| *d < 16) {
            if mantissa >> 60 == 0 {
                mantissa = mantissa << 4 | u64::from(d);
                exponent -= 4;
            } else {
                sticky |= d != 0;
            }
            digits += 1;
            i += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if get(i) == u16::from(b'p') || get(i) == u16::from(b'P') {
        let mut k = i + 1;
        let mut negative = false;
        if get(k) == u16::from(b'+') || get(k) == u16::from(b'-') {
            negative = get(k) == u16::from(b'-');
            k += 1;
        }
        let first = k;
        let mut e = 0i64;
        while let Some(d) = digit(get(k)).filter(|d| *d < 10) {
            e = (e * 10 + i64::from(d)).min(100_000);
            k += 1;
        }
        if k > first {
            exponent += if negative { -e } else { e };
            i = k;
        }
    }
    if mantissa == 0 {
        return Some((0.0, i));
    }
    // Normalize to 53 bits with round-to-nearest-even.
    let lead = 63 - mantissa.leading_zeros() as i64;
    let mut e2 = exponent + lead;
    let value = if lead > 52 {
        let shift = (lead - 52) as u32;
        let half = 1u64 << (shift - 1);
        let rest = mantissa & ((1u64 << shift) - 1);
        let mut m = mantissa >> shift;
        if rest > half || (rest == half && (sticky || m & 1 == 1)) {
            m += 1;
            if m >> 53 != 0 {
                m >>= 1;
                e2 += 1;
            }
        }
        (m, e2)
    } else {
        (mantissa << (52 - lead), e2)
    };
    let (m, e2) = value;
    Some((scale(m, e2 - 52), i))
}
/// m × 2^e for a 53-bit m (subnormals and overflow included).
fn scale(m: u64, e: i64) -> f64 {
    let mut v = m as f64;
    let mut e = e;
    while e > 0 {
        let step = e.min(1000);
        v *= f64::from_bits(((1023 + step) as u64) << 52);
        e -= step;
        if v.is_infinite() {
            return v;
        }
    }
    while e < 0 {
        let step = (-e).min(1000);
        v /= f64::from_bits(((1023 + step) as u64) << 52);
        e += step;
        if v == 0.0 {
            return v;
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    fn of(s: &str) -> impl Fn(usize) -> u16 + '_ {
        move |i| s.as_bytes().get(i).copied().map_or(0, u16::from)
    }
    #[test]
    fn integers_follow_strtol() {
        let r = integer(&of("  -0x1fz"), 0).unwrap();
        assert_eq!((r.magnitude, r.negative, r.end), (0x1f, true, 7));
        assert_eq!(integer(&of("0755"), 0).unwrap().magnitude, 0o755);
        assert_eq!(integer(&of("0x"), 16).unwrap().end, 1);
        assert_eq!(integer(&of("zz"), 36).unwrap().magnitude, 35 * 36 + 35);
        assert_eq!(integer(&of("  +"), 10).unwrap().end, 0);
        assert!(integer(&of("1"), 1).is_none());
        let big = integer(&of("99999999999999999999999"), 10).unwrap();
        assert!(big.overflow);
        assert_eq!(signed(big, 32), (i32::MAX as i64, true));
        assert_eq!(
            signed(integer(&of("-2147483648"), 10).unwrap(), 32),
            (i32::MIN as i64, false)
        );
        assert_eq!(
            signed(integer(&of("-2147483649"), 10).unwrap(), 32),
            (i32::MIN as i64, true)
        );
        assert_eq!(
            unsigned(integer(&of("-1"), 10).unwrap(), 32),
            (u32::MAX as u64, false)
        );
        assert_eq!(
            unsigned(integer(&of("4294967296"), 10).unwrap(), 32),
            (u32::MAX as u64, true)
        );
        assert_eq!(
            unsigned(integer(&of("18446744073709551615"), 10).unwrap(), 64),
            (u64::MAX, false)
        );
    }
    #[test]
    fn floats_follow_strtod() {
        assert_eq!(float(&of(" 3.25xyz")), (3.25, 5, false));
        assert_eq!(float(&of("-1e3")), (-1000.0, 4, false));
        assert_eq!(float(&of("1e")), (1.0, 1, false));
        assert_eq!(float(&of(".5")), (0.5, 2, false));
        assert_eq!(float(&of(".")), (0.0, 0, false));
        assert_eq!(float(&of("0x1.8p1")), (3.0, 7, false));
        assert_eq!(float(&of("0x")), (0.0, 1, false));
        assert_eq!(float(&of("-INFINITY")), (f64::NEG_INFINITY, 9, false));
        let (nan, end, _) = float(&of("nan(123)x"));
        assert!(nan.is_nan());
        assert_eq!(end, 8);
        assert!(float(&of("1e999")).2);
        assert!(float(&of("1e-999")).2);
        assert!(!float(&of("0e-999")).2);
        assert_eq!(float(&of("0x1p-1074")).0, f64::from_bits(1));
    }
}
