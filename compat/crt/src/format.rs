//! The printf family's formatting engine, as the Universal CRT formats:
//! flags `-+ #0`, width and precision (also `*`), the length modifiers
//! `hh h l ll L j z t w I I32 I64`, and `d i u o x X c C s S Z p n f F e E
//! g G a A %`. Wide and narrow output share one engine (`Sink` takes UTF-16
//! or UTF-8 units); arguments come from a C `va_list` through `Args`.
//!
//! UCRT details kept: two-digit exponents (three with
//! `LEGACY_THREE_DIGIT_EXPONENTS`), `inf`/`nan` spellings (`inf`, `nan`,
//! `nan(ind)` for the default NaN, upper case for `F E G A`), `%p` as 16
//! (or 8) upper-case hex digits, legacy wide specifiers (`%s` in a wide
//! format is a wide string unless the standard mode is asked for), and
//! `%n` refused (UCRT disables it unless `_set_printf_count_output`).

/// `_CRT_INTERNAL_PRINTF_*` options of `__stdio_common_v*printf`.
pub mod options {
    pub const LEGACY_VSPRINTF_NULL_TERMINATION: u64 = 0x1;
    pub const STANDARD_SNPRINTF_BEHAVIOR: u64 = 0x2;
    pub const LEGACY_WIDE_SPECIFIERS: u64 = 0x4;
    pub const LEGACY_MSVCRT_COMPATIBILITY: u64 = 0x8;
    pub const LEGACY_THREE_DIGIT_EXPONENTS: u64 = 0x10;
    pub const STANDARD_ROUNDING: u64 = 0x20;
}

/// Where formatted units go: UTF-8 bytes for the narrow functions, UTF-16
/// units for the wide ones. `put` returns false when the sink is full or
/// failed; the engine keeps counting (snprintf's "would have written").
pub trait Sink {
    fn put(&mut self, unit: u16) -> bool;
    /// True for a wide (UTF-16) sink.
    fn wide(&self) -> bool;
}

/// A C argument list. Integers come out as 64 bits (the caller asked for
/// `size` bytes: 4 or 8; smaller types are promoted to int).
pub trait Args {
    fn int(&mut self, size: usize) -> u64;
    fn double(&mut self) -> f64;
    fn pointer(&mut self) -> usize;
    /// Read the `index`th unit of the string at `pointer` (narrow: a byte,
    /// wide: a UTF-16 unit). Returns 0 past the end the caller cannot see.
    fn unit(&mut self, pointer: usize, index: usize, wide: bool) -> u16;
    /// `%n`: store `count` into the int at `pointer` of `size` bytes.
    fn store(&mut self, pointer: usize, size: usize, count: u64);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A malformed or refused conversion (`%n` while disabled): -1, EINVAL.
    Invalid,
}

#[derive(Clone, Copy, Default)]
struct Spec {
    left: bool,
    plus: bool,
    space: bool,
    alternate: bool,
    zero: bool,
    width: usize,
    precision: Option<usize>,
}

/// Counts what is written, passing units to the sink while it accepts.
struct Out<'a> {
    sink: &'a mut dyn Sink,
    count: usize,
    full: bool,
}
impl Out<'_> {
    fn unit(&mut self, unit: u16) {
        if !self.full && !self.sink.put(unit) {
            self.full = true;
        }
        self.count += 1;
    }
    /// A character (a Unicode scalar): UTF-16 or UTF-8 units.
    fn char(&mut self, c: char) {
        if self.sink.wide() {
            let mut b = [0u16; 2];
            for u in c.encode_utf16(&mut b) {
                self.unit(*u);
            }
        } else {
            let mut b = [0u8; 4];
            for u in c.encode_utf8(&mut b).bytes() {
                self.unit(u16::from(u));
            }
        }
    }
    fn ascii(&mut self, s: &[u8]) {
        for &b in s {
            self.unit(u16::from(b));
        }
    }
    fn repeat(&mut self, c: u8, n: usize) {
        for _ in 0..n {
            self.unit(u16::from(c));
        }
    }
}

/// Format `format` (units of the sink's width) with `args` into `sink`:
/// the number of units the whole result has (some may not have fit).
pub fn format(
    format: &[u16],
    args: &mut dyn Args,
    sink: &mut dyn Sink,
    options: u64,
) -> Result<usize, Error> {
    let wide_format = sink.wide();
    let mut out = Out {
        sink,
        count: 0,
        full: false,
    };
    let mut i = 0;
    let at = |i: usize| format.get(i).copied().unwrap_or(0);
    while i < format.len() {
        let c = format[i];
        if c != u16::from(b'%') {
            out.unit(c);
            i += 1;
            continue;
        }
        i += 1;
        let mut spec = Spec::default();
        // Flags.
        loop {
            match at(i) {
                0x2D => spec.left = true,
                0x2B => spec.plus = true,
                0x20 => spec.space = true,
                0x23 => spec.alternate = true,
                0x30 => spec.zero = true,
                _ => break,
            }
            i += 1;
        }
        // Width.
        if at(i) == u16::from(b'*') {
            let w = args.int(4) as u32 as i32;
            if w < 0 {
                spec.left = true;
                spec.width = w.unsigned_abs() as usize;
            } else {
                spec.width = w as usize;
            }
            i += 1;
        } else {
            while (0x30..=0x39).contains(&at(i)) {
                spec.width = spec.width.saturating_mul(10) + usize::from(at(i) - 0x30);
                i += 1;
            }
        }
        // Precision.
        if at(i) == u16::from(b'.') {
            i += 1;
            if at(i) == u16::from(b'*') {
                let p = args.int(4) as u32 as i32;
                spec.precision = (p >= 0).then_some(p as usize);
                i += 1;
            } else {
                let mut p = 0usize;
                while (0x30..=0x39).contains(&at(i)) {
                    p = p.saturating_mul(10) + usize::from(at(i) - 0x30);
                    i += 1;
                }
                spec.precision = Some(p);
            }
        }
        // Length modifier: the argument's size in bytes (0: default),
        // and for strings and characters a forced width.
        let mut size = 0usize;
        let mut long_double = false;
        let mut force_wide: Option<bool> = None;
        loop {
            match at(i) {
                0x68 /* h */ => {
                    size = if size == 2 { 1 } else { 2 };
                    force_wide = Some(false);
                }
                0x6C /* l */ => {
                    size = if size == 4 || size == 8 && at(i - 1) == u16::from(b'l') { 8 } else { 4 };
                    force_wide = Some(true);
                }
                0x4C /* L */ => long_double = true,
                0x6A /* j */ | 0x7A /* z */ | 0x74 /* t */ => size = core::mem::size_of::<usize>().max(if at(i) == 0x6A { 8 } else { 0 }),
                0x77 /* w */ => force_wide = Some(true),
                0x49 /* I */ => {
                    if at(i + 1) == u16::from(b'3') && at(i + 2) == u16::from(b'2') {
                        size = 4;
                        i += 2;
                    } else if at(i + 1) == u16::from(b'6') && at(i + 2) == u16::from(b'4') {
                        size = 8;
                        i += 2;
                    } else {
                        size = core::mem::size_of::<usize>();
                    }
                }
                _ => break,
            }
            i += 1;
        }
        let _ = long_double; // long double is double on Windows.
        let conversion = at(i);
        i += 1;
        let conversion = u8::try_from(conversion).map_err(|_| Error::Invalid)?;
        match conversion {
            b'%' => out.unit(u16::from(b'%')),
            b'd' | b'i' => {
                let v = signed(args, size);
                integer(&mut out, &spec, v < 0, v.unsigned_abs(), 10, false, None);
            }
            b'u' | b'o' | b'x' | b'X' => {
                let v = unsigned(args, size);
                let (base, upper) = match conversion {
                    b'o' => (8, false),
                    b'x' => (16, false),
                    b'X' => (16, true),
                    _ => (10, false),
                };
                let prefix = if spec.alternate && v != 0 {
                    match conversion {
                        b'x' => Some(&b"0x"[..]),
                        b'X' => Some(&b"0X"[..]),
                        _ => None,
                    }
                } else {
                    None
                };
                let spec = Spec {
                    plus: false,
                    space: false,
                    ..spec
                };
                integer(&mut out, &spec, false, v, base, upper, prefix);
            }
            b'p' => {
                let v = args.pointer() as u64;
                let digits = core::mem::size_of::<usize>() * 2;
                let spec = Spec {
                    precision: Some(digits),
                    plus: false,
                    space: false,
                    alternate: false,
                    ..spec
                };
                integer(&mut out, &spec, false, v, 16, true, None);
            }
            b'c' | b'C' => {
                // `%c` in a wide format is wide (legacy) unless forced.
                let wide = character_wide(conversion, force_wide, wide_format, options);
                let v = args.int(4) as u32;
                let c = if wide {
                    char::from_u32(v & 0xFFFF).unwrap_or('\u{FFFD}')
                } else {
                    char::from(v as u8)
                };
                pad_before(&mut out, &spec, 1);
                out.char(c);
                pad_after(&mut out, &spec, 1);
            }
            b's' | b'S' | b'Z' => {
                let pointer = args.pointer();
                let wide = character_wide(conversion, force_wide, wide_format, options);
                string(&mut out, &spec, args, pointer, wide);
            }
            b'n' => return Err(Error::Invalid),
            b'f' | b'F' | b'e' | b'E' | b'g' | b'G' | b'a' | b'A' => {
                let v = args.double();
                float(&mut out, &spec, v, conversion, options);
            }
            _ => return Err(Error::Invalid),
        }
    }
    Ok(out.count)
}

/// Whether `%c`/`%s` (and `%C`/`%S`) take wide characters.
fn character_wide(conversion: u8, forced: Option<bool>, wide_format: bool, options: u64) -> bool {
    if let Some(forced) = forced {
        return forced;
    }
    let upper = conversion.is_ascii_uppercase() && conversion != b'Z';
    let legacy = options & options::LEGACY_WIDE_SPECIFIERS != 0;
    if wide_format {
        // Legacy: %s wide, %S narrow. Standard: %s narrow, %S wide.
        if legacy { !upper } else { upper }
    } else {
        upper
    }
}

fn signed(args: &mut dyn Args, size: usize) -> i64 {
    let raw = args.int(if size == 8 { 8 } else { 4 });
    match size {
        1 => raw as u8 as i8 as i64,
        2 => raw as u16 as i16 as i64,
        8 => raw as i64,
        _ => raw as u32 as i32 as i64,
    }
}
fn unsigned(args: &mut dyn Args, size: usize) -> u64 {
    let raw = args.int(if size == 8 { 8 } else { 4 });
    match size {
        1 => raw as u8 as u64,
        2 => raw as u16 as u64,
        8 => raw,
        _ => raw as u32 as u64,
    }
}

fn pad_before(out: &mut Out<'_>, spec: &Spec, length: usize) {
    if !spec.left && spec.width > length {
        out.repeat(b' ', spec.width - length);
    }
}
fn pad_after(out: &mut Out<'_>, spec: &Spec, length: usize) {
    if spec.left && spec.width > length {
        out.repeat(b' ', spec.width - length);
    }
}

#[allow(clippy::too_many_arguments)]
fn integer(
    out: &mut Out<'_>,
    spec: &Spec,
    negative: bool,
    value: u64,
    base: u64,
    upper: bool,
    prefix: Option<&[u8]>,
) {
    let mut digits = [0u8; 24];
    let mut n = 0;
    let mut v = value;
    let table: &[u8; 16] = if upper {
        b"0123456789ABCDEF"
    } else {
        b"0123456789abcdef"
    };
    while v != 0 {
        digits[n] = table[(v % base) as usize];
        v /= base;
        n += 1;
    }
    // Precision is the minimum digit count; ".0" with 0 prints nothing.
    let precision = spec.precision.unwrap_or(1);
    let mut zeros = precision.saturating_sub(n);
    // "#o" makes sure the result starts with 0.
    if base == 8 && spec.alternate && zeros == 0 && (n == 0 || digits[n - 1] != b'0') {
        zeros = 1;
    }
    let sign: &[u8] = if negative {
        b"-"
    } else if spec.plus {
        b"+"
    } else if spec.space {
        b" "
    } else {
        b""
    };
    let prefix = prefix.unwrap_or(b"");
    let body = sign.len() + prefix.len() + zeros + n;
    // The 0 flag pads with zeros after the sign unless a precision is given.
    let zero_pad = spec.zero && !spec.left && spec.precision.is_none();
    if zero_pad && spec.width > body {
        zeros += spec.width - body;
    } else {
        pad_before(out, spec, body);
    }
    out.ascii(sign);
    out.ascii(prefix);
    out.repeat(b'0', zeros);
    for k in (0..n).rev() {
        out.unit(u16::from(digits[k]));
    }
    if !zero_pad || spec.width <= body {
        pad_after(out, spec, body);
    }
}

fn string(out: &mut Out<'_>, spec: &Spec, args: &mut dyn Args, pointer: usize, wide: bool) {
    // A null string prints "(null)" as the UCRT does.
    let null: &[u8] = b"(null)";
    let limit = spec.precision.unwrap_or(usize::MAX);
    // First the length in characters (UTF-16 surrogate pairs and UTF-8
    // sequences count as one character each for padding, as code points).
    let mut length = 0usize;
    let mut units = 0usize;
    if pointer == 0 {
        length = null.len().min(limit);
    } else {
        loop {
            let u = args.unit(pointer, units, wide);
            if u == 0 || length >= limit {
                break;
            }
            units += 1;
            if wide {
                if (0xD800..0xDC00).contains(&u) {
                    let next = args.unit(pointer, units, wide);
                    if (0xDC00..0xE000).contains(&next) {
                        units += 1;
                    }
                }
            } else {
                while units < usize::MAX && (args.unit(pointer, units, wide) & 0xC0) == 0x80 {
                    units += 1;
                }
            }
            length += 1;
        }
    }
    pad_before(out, spec, length);
    if pointer == 0 {
        out.ascii(&null[..length]);
    } else {
        let mut k = 0;
        while k < units {
            if wide {
                let u = args.unit(pointer, k, true);
                k += 1;
                let c = if (0xD800..0xDC00).contains(&u) && k < units {
                    let low = args.unit(pointer, k, true);
                    k += 1;
                    char::from_u32(
                        0x10000 + ((u32::from(u) - 0xD800) << 10) + (u32::from(low) - 0xDC00),
                    )
                } else {
                    char::from_u32(u32::from(u))
                };
                out.char(c.unwrap_or('\u{FFFD}'));
            } else {
                // Narrow strings are UTF-8 (the process code page).
                let mut bytes = [0u8; 4];
                let first = args.unit(pointer, k, false) as u8;
                let n = match first {
                    0..=0x7F => 1,
                    0xC0..=0xDF => 2,
                    0xE0..=0xEF => 3,
                    0xF0..=0xF7 => 4,
                    _ => 1,
                }
                .min(units - k);
                for (j, b) in bytes.iter_mut().enumerate().take(n) {
                    *b = args.unit(pointer, k + j, false) as u8;
                }
                k += n;
                match core::str::from_utf8(&bytes[..n]) {
                    Ok(s) => s.chars().for_each(|c| out.char(c)),
                    Err(_) => out.char('\u{FFFD}'),
                }
            }
        }
    }
    pad_after(out, spec, length);
}

/// A small buffer for formatted digits (`core::fmt` into the stack).
struct Buffer {
    data: [u8; 1100],
    len: usize,
}
impl core::fmt::Write for Buffer {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let b = s.as_bytes();
        if self.len + b.len() > self.data.len() {
            return Err(core::fmt::Error);
        }
        self.data[self.len..self.len + b.len()].copy_from_slice(b);
        self.len += b.len();
        Ok(())
    }
}
impl Buffer {
    const fn new() -> Self {
        Self {
            data: [0; 1100],
            len: 0,
        }
    }
    fn bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }
}

/// The decimal digits of `v` (finite, non-negative) rounded to `digits`
/// significant digits, and the decimal exponent of the first digit.
fn significant(v: f64, digits: usize) -> ([u8; 800], usize, i32) {
    use core::fmt::Write;
    let mut b = Buffer::new();
    let digits = digits.clamp(1, 760);
    let _ = write!(b, "{:.*e}", digits - 1, v);
    // "d.ddddde±x" (Rust prints "e-5", "e10").
    let text = b.bytes();
    let e = text.iter().position(|&c| c == b'e').unwrap_or(text.len());
    let mut out = [0u8; 800];
    let mut n = 0;
    for &c in &text[..e] {
        if c.is_ascii_digit() {
            out[n] = c;
            n += 1;
        }
    }
    let exp = core::str::from_utf8(&text[e + 1..])
        .ok()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0);
    (out, n, exp)
}

fn float(out: &mut Out<'_>, spec: &Spec, value: f64, conversion: u8, options: u64) {
    let upper = conversion.is_ascii_uppercase();
    let negative = value.is_sign_negative();
    let sign: &[u8] = if negative {
        b"-"
    } else if spec.plus {
        b"+"
    } else if spec.space {
        b" "
    } else {
        b""
    };
    if !value.is_finite() {
        let text: &[u8] = match (value.is_nan(), upper) {
            (false, false) => b"inf",
            (false, true) => b"INF",
            // The default NaN (sign set, quiet) prints "nan(ind)" in UCRT.
            (true, false) if negative && value.to_bits() == 0xFFF8_0000_0000_0000 => b"nan(ind)",
            (true, true) if negative && value.to_bits() == 0xFFF8_0000_0000_0000 => b"NAN(IND)",
            (true, false) => b"nan",
            (true, true) => b"NAN",
        };
        let length = sign.len() + text.len();
        pad_before(out, spec, length);
        out.ascii(sign);
        out.ascii(text);
        pad_after(out, spec, length);
        return;
    }
    let v = value.abs();
    let mut body = Buffer::new();
    match conversion {
        b'f' | b'F' => fixed(&mut body, v, spec.precision.unwrap_or(6), spec.alternate),
        b'e' | b'E' => exponent(
            &mut body,
            v,
            spec.precision.unwrap_or(6),
            spec.alternate,
            upper,
            options,
        ),
        b'g' | b'G' => general(
            &mut body,
            v,
            spec.precision.unwrap_or(6),
            spec.alternate,
            upper,
            options,
        ),
        _ => hexadecimal(&mut body, v, spec.precision, spec.alternate, upper),
    }
    let digits = body.bytes();
    let length = sign.len() + digits.len();
    if spec.zero && !spec.left && spec.width > length {
        out.ascii(sign);
        // Hex floats keep "0x" before the zeros.
        let (prefix, rest) = if matches!(conversion, b'a' | b'A') && digits.len() >= 2 {
            digits.split_at(2)
        } else {
            (&digits[..0], digits)
        };
        out.ascii(prefix);
        out.repeat(b'0', spec.width - length);
        out.ascii(rest);
        return;
    }
    pad_before(out, spec, length);
    out.ascii(sign);
    out.ascii(digits);
    pad_after(out, spec, length);
}

fn fixed(b: &mut Buffer, v: f64, precision: usize, alternate: bool) {
    use core::fmt::Write;
    let precision = precision.min(500);
    let _ = write!(b, "{:.*}", precision, v);
    if precision == 0 && alternate {
        let _ = b.write_str(".");
    }
}

fn exponent_text(b: &mut Buffer, exp: i32, upper: bool, options: u64) {
    use core::fmt::Write;
    let _ = b.write_str(if upper { "E" } else { "e" });
    let _ = b.write_str(if exp < 0 { "-" } else { "+" });
    if options & options::LEGACY_THREE_DIGIT_EXPONENTS != 0 {
        let _ = write!(b, "{:03}", exp.unsigned_abs());
    } else {
        let _ = write!(b, "{:02}", exp.unsigned_abs());
    }
}

fn exponent(b: &mut Buffer, v: f64, precision: usize, alternate: bool, upper: bool, options: u64) {
    use core::fmt::Write;
    let precision = precision.min(500);
    let (digits, n, exp) = if v == 0.0 {
        ([b'0'; 800], precision + 1, 0)
    } else {
        significant(v, precision + 1)
    };
    let _ = b.write_char(digits[0] as char);
    if precision > 0 || alternate {
        let _ = b.write_str(".");
    }
    for &d in &digits[1..n.max(1)] {
        let _ = b.write_char(d as char);
    }
    exponent_text(b, exp, upper, options);
}

fn general(b: &mut Buffer, v: f64, precision: usize, alternate: bool, upper: bool, options: u64) {
    use core::fmt::Write;
    let p = if precision == 0 {
        1
    } else {
        precision.min(500)
    };
    let exp = if v == 0.0 { 0 } else { significant(v, p).2 };
    let mut tmp = Buffer::new();
    let use_exponent = exp < -4 || exp >= p as i32;
    if use_exponent {
        exponent(&mut tmp, v, p - 1, alternate, upper, options);
    } else {
        let decimals = (p as i32 - 1 - exp).max(0) as usize;
        fixed(&mut tmp, v, decimals, alternate);
    }
    let text = tmp.bytes();
    if alternate {
        let _ = b.write_str(core::str::from_utf8(text).unwrap_or(""));
        return;
    }
    // Without '#', trailing zeros (and a trailing point) go.
    let e = text
        .iter()
        .position(|&c| c == b'e' || c == b'E')
        .unwrap_or(text.len());
    let (mantissa, rest) = text.split_at(e);
    let mut end = mantissa.len();
    if mantissa.contains(&b'.') {
        while end > 0 && mantissa[end - 1] == b'0' {
            end -= 1;
        }
        if end > 0 && mantissa[end - 1] == b'.' {
            end -= 1;
        }
    }
    let _ = b.write_str(core::str::from_utf8(&mantissa[..end]).unwrap_or(""));
    let _ = b.write_str(core::str::from_utf8(rest).unwrap_or(""));
}

fn hexadecimal(b: &mut Buffer, v: f64, precision: Option<usize>, alternate: bool, upper: bool) {
    use core::fmt::Write;
    let bits = v.to_bits();
    let raw_exp = ((bits >> 52) & 0x7FF) as i32;
    let mut mantissa = bits & ((1u64 << 52) - 1);
    let (lead, exp) = if raw_exp == 0 {
        if mantissa == 0 {
            (0u64, 0)
        } else {
            (0u64, -1022)
        }
    } else {
        (1u64, raw_exp - 1023)
    };
    let digits = b"0123456789abcdef";
    let digits_upper = b"0123456789ABCDEF";
    let table = if upper { digits_upper } else { digits };
    let mut lead = lead;
    // 13 hex digits of fraction; round to the precision asked for.
    let mut count = 13usize;
    if let Some(p) = precision
        && p < 13
    {
        let shift = (13 - p) * 4;
        let half = 1u64 << (shift - 1);
        let rest = mantissa & ((1u64 << shift) - 1);
        mantissa >>= shift;
        if rest > half || (rest == half && mantissa & 1 == 1) {
            mantissa += 1;
            if mantissa >> (p * 4) != 0 {
                mantissa &= (1u64 << (p * 4)) - 1;
                lead += 1;
            }
        }
        count = p;
    }
    let _ = b.write_str(if upper { "0X" } else { "0x" });
    let _ = b.write_char(table[lead as usize] as char);
    let mut fraction = [0u8; 13];
    for k in 0..count {
        fraction[k] = table[((mantissa >> ((count - 1 - k) * 4)) & 15) as usize];
    }
    // Without a precision the UCRT prints all 13 digits (no trimming).
    let shown = count;
    if shown > 0 || alternate {
        let _ = b.write_str(".");
    }
    for &c in &fraction[..shown] {
        let _ = b.write_char(c as char);
    }
    if let Some(p) = precision {
        for _ in 13..p {
            let _ = b.write_char('0');
        }
    }
    let _ = write!(
        b,
        "{}{}{}",
        if upper { "P" } else { "p" },
        if exp < 0 { "-" } else { "+" },
        exp.unsigned_abs()
    );
}

#[cfg(test)]
#[allow(clippy::approx_constant)] // printf test values, not pi.
mod tests {
    extern crate std;
    use super::*;
    use std::{string::String, vec::Vec};

    enum A {
        I(u64),
        F(f64),
        S(&'static str),
        W(Vec<u16>),
        P(usize),
    }
    struct List(Vec<A>, usize);
    impl Args for List {
        fn int(&mut self, _: usize) -> u64 {
            self.1 += 1;
            match self.0[self.1 - 1] {
                A::I(v) => v,
                _ => 0,
            }
        }
        fn double(&mut self) -> f64 {
            self.1 += 1;
            match self.0[self.1 - 1] {
                A::F(v) => v,
                _ => 0.0,
            }
        }
        fn pointer(&mut self) -> usize {
            self.1 += 1;
            match &self.0[self.1 - 1] {
                A::S(_) | A::W(_) => self.1,
                A::P(p) => *p,
                _ => 0,
            }
        }
        fn unit(&mut self, pointer: usize, index: usize, wide: bool) -> u16 {
            match &self.0[pointer - 1] {
                A::S(s) if !wide => s.as_bytes().get(index).copied().map_or(0, u16::from),
                A::W(w) if wide => w.get(index).copied().unwrap_or(0),
                _ => 0,
            }
        }
        fn store(&mut self, _: usize, _: usize, _: u64) {}
    }
    struct Narrow(Vec<u8>, usize);
    impl Sink for Narrow {
        fn put(&mut self, unit: u16) -> bool {
            if self.0.len() >= self.1 {
                return false;
            }
            self.0.push(unit as u8);
            true
        }
        fn wide(&self) -> bool {
            false
        }
    }
    fn f(fmt: &str, args: Vec<A>) -> String {
        let units: Vec<u16> = fmt.bytes().map(u16::from).collect();
        let mut sink = Narrow(Vec::new(), usize::MAX);
        let n = format(
            &units,
            &mut List(args, 0),
            &mut sink,
            options::LEGACY_WIDE_SPECIFIERS,
        )
        .unwrap();
        assert_eq!(n, sink.0.len());
        String::from_utf8(sink.0).unwrap()
    }
    #[test]
    fn integers_match_the_ucrt() {
        assert_eq!(
            f(
                "%d|%5d|%-5d|%05d|%+d|% d",
                std::vec![
                    A::I(42),
                    A::I(-42i64 as u64),
                    A::I(7),
                    A::I(-7i64 as u64),
                    A::I(3),
                    A::I(3)
                ]
            ),
            "42|  -42|7    |-0007|+3| 3"
        );
        assert_eq!(
            f(
                "%x %X %#x %o %#o %u",
                std::vec![
                    A::I(255),
                    A::I(255),
                    A::I(255),
                    A::I(8),
                    A::I(8),
                    A::I(u32::MAX as u64)
                ]
            ),
            "ff FF 0xff 10 010 4294967295"
        );
        assert_eq!(
            f(
                "%lld %I64u %hd %hhu",
                std::vec![A::I(-5i64 as u64), A::I(u64::MAX), A::I(70000), A::I(300)]
            ),
            "-5 18446744073709551615 4464 44"
        );
        assert_eq!(
            f("%.3d|%.0d|%8.3x", std::vec![A::I(5), A::I(0), A::I(10)]),
            "005||     00a"
        );
        assert_eq!(
            f(
                "%*d|%-*d|%.*d",
                std::vec![A::I(4), A::I(1), A::I(3), A::I(2), A::I(2), A::I(9)]
            ),
            "   1|2  |09"
        );
        assert_eq!(f("%p", std::vec![A::P(0x1234)]), "0000000000001234");
        assert_eq!(f("100%%", std::vec![]), "100%");
    }
    #[test]
    fn strings_and_characters() {
        assert_eq!(
            f(
                "[%s] [%5s] [%-5s] [%.2s] [%c]",
                std::vec![A::S("hi"), A::S("hi"), A::S("hi"), A::S("hello"), A::I(65)]
            ),
            "[hi] [   hi] [hi   ] [he] [A]"
        );
        assert_eq!(f("%s", std::vec![A::P(0)]), "(null)");
        assert_eq!(
            f(
                "%ls|%S",
                std::vec![
                    A::W("wide".encode_utf16().collect()),
                    A::W("Wé".encode_utf16().collect())
                ]
            ),
            "wide|Wé"
        );
        assert_eq!(f("%s", std::vec![A::S("héllo")]), "héllo");
    }
    #[test]
    fn floats_match_the_ucrt() {
        assert_eq!(
            f(
                "%f|%.2f|%10.3f|%-8.1f|%08.2f",
                std::vec![
                    A::F(3.14159),
                    A::F(2.005),
                    A::F(-1.5),
                    A::F(2.25),
                    A::F(-3.5)
                ]
            ),
            "3.141590|2.00|    -1.500|2.2     |-0003.50"
        );
        assert_eq!(
            f(
                "%e|%.2E|%e",
                std::vec![A::F(12345.678), A::F(0.000123), A::F(0.0)]
            ),
            "1.234568e+04|1.23E-04|0.000000e+00"
        );
        assert_eq!(
            f(
                "%g|%g|%g|%g|%G|%#g",
                std::vec![
                    A::F(100000.0),
                    A::F(1000000.0),
                    A::F(0.0001),
                    A::F(0.00001),
                    A::F(1.5e-10),
                    A::F(1.0)
                ]
            ),
            "100000|1e+06|0.0001|1e-05|1.5E-10|1.00000"
        );
        assert_eq!(
            f(
                "%.0f|%.0f|%#.0f",
                std::vec![A::F(0.5), A::F(1.5), A::F(2.0)]
            ),
            "0|2|2."
        );
        assert_eq!(
            f(
                "%f|%F|%f|%e",
                std::vec![
                    A::F(f64::INFINITY),
                    A::F(f64::NEG_INFINITY),
                    A::F(f64::NAN),
                    A::F(-f64::NAN)
                ]
            ),
            "inf|-INF|nan|-nan(ind)"
        );
        assert_eq!(
            f(
                "%a|%A|%.2a|%a",
                std::vec![A::F(1.0), A::F(-2.5), A::F(1.0 / 3.0), A::F(0.0)]
            ),
            "0x1.0000000000000p+0|-0X1.4000000000000P+1|0x1.55p-2|0x0.0000000000000p+0"
        );
        assert_eq!(
            f("%.17g|%.3g", std::vec![A::F(0.1), A::F(123456.0)]),
            "0.10000000000000001|1.23e+05"
        );
    }
    #[test]
    fn the_sink_limit_counts_what_would_be_written() {
        let units: Vec<u16> = "abc%d".bytes().map(u16::from).collect();
        let mut sink = Narrow(Vec::new(), 2);
        assert_eq!(
            format(&units, &mut List(std::vec![A::I(12345)], 0), &mut sink, 0),
            Ok(8)
        );
        assert_eq!(sink.0, b"ab");
        let n: Vec<u16> = "%n".bytes().map(u16::from).collect();
        assert_eq!(
            format(
                &n,
                &mut List(std::vec![A::P(8)], 0),
                &mut Narrow(Vec::new(), 9),
                0
            ),
            Err(Error::Invalid)
        );
    }
}
