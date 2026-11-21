//! Locale-formatted dates, times, numbers and currency (GetDateFormat(Ex),
//! GetTimeFormat(Ex), GetNumberFormatEx, GetCurrencyFormatEx) for the
//! locales Doors has: English (United States) and the invariant locale,
//! with Windows' format pictures (d, dd, ddd, dddd, M ... MMMM, y, yy,
//! yyyy, gg, h, hh, H, HH, m, mm, s, ss, t, tt and quoted text).
use crate::locale::which;

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn RtlSetLastWin32Error(value: u32);
}

const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_FLAGS: u32 = 1004;
const DATE_SHORTDATE: u32 = 0x1;
const DATE_LONGDATE: u32 = 0x2;
const DATE_YEARMONTH: u32 = 0x8;
const DATE_MONTHDAY: u32 = 0x80;
const LOCALE_NOUSEROVERRIDE: u32 = 0x8000_0000;
const LOCALE_USE_CP_ACP: u32 = 0x4000_0000;
const TIME_NOMINUTESORSECONDS: u32 = 0x1;
const TIME_NOSECONDS: u32 = 0x2;
const TIME_NOTIMEMARKER: u32 = 0x4;
const TIME_FORCE24HOURFORMAT: u32 = 0x8;

const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn fail(code: u32) -> i32 {
    unsafe { RtlSetLastWin32Error(code) };
    0
}
/// Output built in UTF-16 units, bounded.
struct Out {
    units: [u16; 256],
    len: usize,
}
impl Out {
    fn new() -> Self {
        Self {
            units: [0; 256],
            len: 0,
        }
    }
    fn push_str(&mut self, s: &str) {
        for u in s.encode_utf16() {
            self.push(u);
        }
    }
    fn push(&mut self, u: u16) {
        if self.len < self.units.len() {
            self.units[self.len] = u;
            self.len += 1;
        }
    }
    fn number(&mut self, value: u32, digits: usize) {
        let mut buf = [0u8; 10];
        let mut n = 0;
        let mut v = value;
        loop {
            buf[n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        for _ in n..digits {
            self.push(u16::from(b'0'));
        }
        for i in (0..n).rev() {
            self.push(u16::from(buf[i]));
        }
    }
    /// Into the caller's buffer of `capacity` units (0: the size needed),
    /// with its NUL: the units written including the NUL.
    fn deliver(&self, buffer: usize, capacity: i32) -> i32 {
        let needed = self.len as i32 + 1;
        if capacity == 0 {
            return needed;
        }
        if capacity < needed || buffer == 0 {
            return fail(ERROR_INSUFFICIENT_BUFFER);
        }
        for (i, u) in self.units[..self.len].iter().chain([&0]).enumerate() {
            // SAFETY: the caller's buffer of `capacity` units.
            unsafe { (buffer as *mut u16).add(i).write_unaligned(*u) };
        }
        needed
    }
}
/// SYSTEMTIME, or the local time now.
#[derive(Clone, Copy)]
struct Time {
    year: u16,
    month: u16,
    weekday: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
}
fn time_of(pointer: usize) -> Option<Time> {
    let mut raw = [0u16; 8];
    if pointer == 0 {
        crate::machine::GetLocalTime(raw.as_mut_ptr() as usize);
    } else {
        for (i, slot) in raw.iter_mut().enumerate() {
            // SAFETY: the caller's SYSTEMTIME.
            *slot = unsafe { (pointer as *const u16).add(i).read_unaligned() };
        }
    }
    let t = Time {
        year: raw[0],
        month: raw[1],
        weekday: raw[2],
        day: raw[3],
        hour: raw[4],
        minute: raw[5],
        second: raw[6],
    };
    ((1..=12).contains(&t.month)
        && (1..=31).contains(&t.day)
        && t.hour < 24
        && t.minute < 60
        && t.second < 60)
        .then_some(t)
}
/// Day of the week a date falls on (Sakamoto's method).
fn weekday(year: u16, month: u16, day: u16) -> u16 {
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut y = i32::from(year);
    if month < 3 {
        y -= 1;
    }
    ((y + y / 4 - y / 100 + y / 400 + T[usize::from(month - 1)] + i32::from(day)) % 7) as u16
}
/// A format picture (wide, NUL-terminated or `None`) into its units.
fn picture(pointer: usize, out: &mut [u16; 128]) -> Option<usize> {
    if pointer == 0 {
        return None;
    }
    let mut n = 0;
    while n < out.len() {
        // SAFETY: the caller's NUL-terminated picture.
        let u = unsafe { (pointer as *const u16).add(n).read_unaligned() };
        if u == 0 {
            break;
        }
        out[n] = u;
        n += 1;
    }
    Some(n)
}
/// Expand a date or time picture.
fn expand(t: Time, picture: &[u16], out: &mut Out, time: bool, force24: bool, marker: bool) {
    let mut i = 0;
    while i < picture.len() {
        let c = picture[i];
        if c == u16::from(b'\'') {
            i += 1;
            while i < picture.len() {
                if picture[i] == u16::from(b'\'') {
                    if picture.get(i + 1) == Some(&u16::from(b'\'')) {
                        out.push(c);
                        i += 2;
                        continue;
                    }
                    break;
                }
                out.push(picture[i]);
                i += 1;
            }
            i += 1;
            continue;
        }
        let run = picture[i..].iter().take_while(|&&u| u == c).count();
        let letter = u8::try_from(c).unwrap_or(0);
        match (letter, time) {
            (b'd', false) => match run {
                1 => out.number(u32::from(t.day), 1),
                2 => out.number(u32::from(t.day), 2),
                3 => out.push_str(&DAYS[usize::from(t.weekday % 7)][..3]),
                _ => out.push_str(DAYS[usize::from(t.weekday % 7)]),
            },
            (b'M', false) => match run {
                1 => out.number(u32::from(t.month), 1),
                2 => out.number(u32::from(t.month), 2),
                3 => out.push_str(&MONTHS[usize::from(t.month - 1)][..3]),
                _ => out.push_str(MONTHS[usize::from(t.month - 1)]),
            },
            (b'y', false) => match run {
                1 => out.number(u32::from(t.year % 100), 1),
                2 => out.number(u32::from(t.year % 100), 2),
                _ => out.number(u32::from(t.year), 4),
            },
            (b'g', false) => out.push_str("A.D."),
            (b'h', true) | (b'H', true) => {
                let hour = if letter == b'H' || force24 {
                    t.hour
                } else if t.hour.is_multiple_of(12) {
                    12
                } else {
                    t.hour % 12
                };
                out.number(u32::from(hour), run.min(2));
            }
            (b'm', true) => out.number(u32::from(t.minute), run.min(2)),
            (b's', true) => out.number(u32::from(t.second), run.min(2)),
            (b't', true) => {
                if marker && !force24 {
                    let am = t.hour < 12;
                    if run == 1 {
                        out.push_str(if am { "A" } else { "P" });
                    } else {
                        out.push_str(if am { "AM" } else { "PM" });
                    }
                }
            }
            _ => {
                for _ in 0..run {
                    out.push(c);
                }
            }
        }
        i += run;
    }
    // A time marker left out leaves its separating space behind: trim it.
    while time && out.len > 0 && out.units[out.len - 1] == u16::from(b' ') {
        out.len -= 1;
    }
}
fn date_format(
    english: bool,
    flags: u32,
    time: usize,
    format: usize,
    buffer: usize,
    capacity: i32,
) -> i32 {
    let flags = flags & !(LOCALE_NOUSEROVERRIDE | LOCALE_USE_CP_ACP);
    let Some(mut t) = time_of(time) else {
        return fail(ERROR_INVALID_PARAMETER);
    };
    t.weekday = weekday(t.year, t.month, t.day);
    let mut custom = [0u16; 128];
    let default: &str = match flags & !0x30 {
        0 | DATE_SHORTDATE => {
            if english {
                "M/d/yyyy"
            } else {
                "MM/dd/yyyy"
            }
        }
        DATE_LONGDATE => {
            if english {
                "dddd, MMMM d, yyyy"
            } else {
                "dddd, dd MMMM yyyy"
            }
        }
        DATE_YEARMONTH => "MMMM yyyy",
        DATE_MONTHDAY => "MMMM d",
        _ => return fail(ERROR_INVALID_FLAGS),
    };
    let mut fallback = [0u16; 128];
    let pic: &[u16] = match picture(format, &mut custom) {
        Some(n) => {
            if flags != 0 {
                return fail(ERROR_INVALID_FLAGS);
            }
            &custom[..n]
        }
        None => {
            let n = default.encode_utf16().count();
            for (slot, u) in fallback.iter_mut().zip(default.encode_utf16()) {
                *slot = u;
            }
            &fallback[..n]
        }
    };
    let mut out = Out::new();
    expand(t, pic, &mut out, false, false, true);
    out.deliver(buffer, capacity)
}
fn time_format(
    english: bool,
    flags: u32,
    time: usize,
    format: usize,
    buffer: usize,
    capacity: i32,
) -> i32 {
    let flags = flags & !(LOCALE_NOUSEROVERRIDE | LOCALE_USE_CP_ACP);
    if flags & !0xf != 0 {
        return fail(ERROR_INVALID_FLAGS);
    }
    let Some(t) = time_of(time) else {
        return fail(ERROR_INVALID_PARAMETER);
    };
    let mut custom = [0u16; 128];
    let base = if english { "h:mm:ss tt" } else { "HH:mm:ss" };
    let mut text = [0u16; 128];
    let n = match picture(format, &mut custom) {
        Some(n) => {
            text[..n].copy_from_slice(&custom[..n]);
            n
        }
        None => {
            let mut n = 0;
            for u in base.encode_utf16() {
                text[n] = u;
                n += 1;
            }
            n
        }
    };
    // Flags remove parts of the picture: seconds (with the separator
    // before them), minutes and seconds, the marker.
    let mut kept = [0u16; 128];
    let mut k = 0;
    let mut i = 0;
    while i < n {
        let c = text[i];
        let drop_unit = (c == u16::from(b's')
            && flags & (TIME_NOSECONDS | TIME_NOMINUTESORSECONDS) != 0)
            || (c == u16::from(b'm') && flags & TIME_NOMINUTESORSECONDS != 0);
        if drop_unit {
            // Remove the separator written before it.
            while k > 0 && !(kept[k - 1] as u8 as char).is_ascii_alphanumeric() {
                k -= 1;
            }
            while i < n && text[i] == c {
                i += 1;
            }
            continue;
        }
        kept[k] = c;
        k += 1;
        i += 1;
    }
    let mut out = Out::new();
    expand(
        t,
        &kept[..k],
        &mut out,
        true,
        flags & TIME_FORCE24HOURFORMAT != 0,
        flags & TIME_NOTIMEMARKER == 0,
    );
    out.deliver(buffer, capacity)
}
/// LCID-based callers: English (United States), the user's and system
/// defaults, and the invariant locale.
fn lcid_english(lcid: u32) -> Option<bool> {
    match lcid {
        0 | 0x400 | 0x800 | 0x409 => Some(true),
        0x7f => Some(false),
        _ => None,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetDateFormatEx(
    locale: usize,
    flags: u32,
    time: usize,
    format: usize,
    buffer: usize,
    capacity: i32,
    _calendar: usize,
) -> i32 {
    let Some(english) = which(locale) else {
        return fail(ERROR_INVALID_PARAMETER);
    };
    date_format(english, flags, time, format, buffer, capacity)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetDateFormatW(
    lcid: u32,
    flags: u32,
    time: usize,
    format: usize,
    buffer: usize,
    capacity: i32,
) -> i32 {
    let Some(english) = lcid_english(lcid) else {
        return fail(ERROR_INVALID_PARAMETER);
    };
    date_format(english, flags, time, format, buffer, capacity)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetTimeFormatEx(
    locale: usize,
    flags: u32,
    time: usize,
    format: usize,
    buffer: usize,
    capacity: i32,
) -> i32 {
    let Some(english) = which(locale) else {
        return fail(ERROR_INVALID_PARAMETER);
    };
    time_format(english, flags, time, format, buffer, capacity)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetTimeFormatW(
    lcid: u32,
    flags: u32,
    time: usize,
    format: usize,
    buffer: usize,
    capacity: i32,
) -> i32 {
    let Some(english) = lcid_english(lcid) else {
        return fail(ERROR_INVALID_PARAMETER);
    };
    time_format(english, flags, time, format, buffer, capacity)
}

// ---- numbers -------------------------------------------------------------------

/// NUMBERFMTW / CURRENCYFMTW overrides (or the locale's).
struct NumberStyle {
    digits: u32,
    leading_zero: bool,
    grouping: u32,
    decimal: [u16; 4],
    thousand: [u16; 4],
    negative: u32,
    positive: u32,
    symbol: [u16; 8],
}
fn separator(pointer: usize, out: &mut [u16; 4]) -> Option<()> {
    if pointer == 0 {
        return None;
    }
    *out = [0; 4];
    for (i, slot) in out.iter_mut().enumerate().take(3) {
        // SAFETY: the caller's NUL-terminated separator.
        let u = unsafe { (pointer as *const u16).add(i).read_unaligned() };
        if u == 0 {
            break;
        }
        *slot = u;
    }
    Some(())
}
fn units_of(text: &[u16; 4]) -> &[u16] {
    let n = text.iter().position(|&u| u == 0).unwrap_or(4);
    &text[..n]
}
/// A number's sign, integer digits (and count) and fraction digits (and
/// count).
type Parsed = (bool, [u8; 64], usize, [u8; 64], usize);
/// The value text ("-1234.567").
fn parse_value(pointer: usize) -> Option<Parsed> {
    let mut text = [0u16; 128];
    let n = picture(pointer, &mut text)?;
    let mut i = 0;
    let negative = text.first() == Some(&u16::from(b'-'));
    if negative {
        i = 1;
    }
    let (mut int, mut ni, mut frac, mut nf) = ([0u8; 64], 0, [0u8; 64], 0);
    let mut seen_point = false;
    while i < n {
        let c = u8::try_from(text[i]).ok()?;
        match c {
            b'0'..=b'9' if !seen_point && ni < 64 => {
                int[ni] = c;
                ni += 1;
            }
            b'0'..=b'9' if seen_point && nf < 64 => {
                frac[nf] = c;
                nf += 1;
            }
            b'.' if !seen_point => seen_point = true,
            _ => return None,
        }
        i += 1;
    }
    (ni + nf > 0).then_some((negative, int, ni, frac, nf))
}
/// The number text: grouped integer digits, the decimal separator and
/// `digits` fraction digits (rounded half up).
fn format_number(value: usize, style: &NumberStyle, out: &mut Out) -> Option<bool> {
    let (negative, mut int, mut ni, mut frac, nf) = parse_value(value)?;
    let digits = style.digits as usize;
    // Round at `digits`.
    if nf > digits && frac[digits] >= b'5' {
        let mut carry = true;
        for d in frac[..digits].iter_mut().rev() {
            if !carry {
                break;
            }
            if *d == b'9' {
                *d = b'0';
            } else {
                *d += 1;
                carry = false;
            }
        }
        if carry {
            for d in int[..ni].iter_mut().rev() {
                if !carry {
                    break;
                }
                if *d == b'9' {
                    *d = b'0';
                } else {
                    *d += 1;
                    carry = false;
                }
            }
            if carry && ni < 64 {
                int.copy_within(0..ni, 1);
                int[0] = b'1';
                ni += 1;
            }
        }
    }
    for d in frac.iter_mut().take(digits).skip(nf) {
        *d = b'0';
    }
    let int_digits = &int[..ni];
    let start = int_digits.iter().position(|&d| d != b'0').unwrap_or(ni);
    let significant = &int_digits[start..];
    let zero = significant.is_empty() && frac[..digits].iter().all(|&d| d == b'0');
    if significant.is_empty() {
        if style.leading_zero || digits == 0 {
            out.push(u16::from(b'0'));
        }
    } else {
        let group = style.grouping as usize;
        for (i, &d) in significant.iter().enumerate() {
            let left = significant.len() - i;
            if i > 0 && group > 0 && left % group == 0 {
                for &u in units_of(&style.thousand) {
                    out.push(u);
                }
            }
            out.push(u16::from(d));
        }
    }
    if digits > 0 {
        for &u in units_of(&style.decimal) {
            out.push(u);
        }
        for &d in &frac[..digits] {
            out.push(u16::from(d));
        }
    }
    Some(negative && !zero)
}
fn read_style(format: usize, currency: bool, style: &mut NumberStyle) -> bool {
    if format == 0 {
        return true;
    }
    // NUMBERFMTW: NumDigits, LeadingZero, Grouping (UINT), then the two
    // separator pointers and NegativeOrder; CURRENCYFMTW adds
    // PositiveOrder and lpCurrencySymbol.
    let word = |at: usize| -> u32 {
        // SAFETY: the caller's format structure.
        unsafe { ((format + at) as *const u32).read_unaligned() }
    };
    let pointer = |at: usize| -> usize {
        // SAFETY: the caller's format structure.
        unsafe { ((format + at) as *const usize).read_unaligned() }
    };
    let p = core::mem::size_of::<usize>();
    let after_words = 12usize.next_multiple_of(p);
    style.digits = word(0);
    style.leading_zero = word(4) != 0;
    // Grouping 3 (or 30, "3;0") groups by three.
    style.grouping = match word(8) {
        30 | 3 => 3,
        0 => 0,
        g => g % 10,
    };
    if separator(pointer(after_words), &mut style.decimal).is_none()
        || separator(pointer(after_words + p), &mut style.thousand).is_none()
    {
        return false;
    }
    style.negative = word(after_words + 2 * p);
    if currency {
        style.positive = word(after_words + 2 * p + 4);
        let symbol = pointer((after_words + 2 * p + 8).next_multiple_of(p));
        if symbol == 0 {
            return false;
        }
        style.symbol = [0; 8];
        for (i, slot) in style.symbol.iter_mut().enumerate().take(7) {
            // SAFETY: the caller's NUL-terminated symbol.
            let u = unsafe { (symbol as *const u16).add(i).read_unaligned() };
            if u == 0 {
                break;
            }
            *slot = u;
        }
    }
    style.digits <= 9
}
fn locale_style(english: bool) -> NumberStyle {
    let mut symbol = [0u16; 8];
    symbol[0] = if english { u16::from(b'$') } else { 0xa4 };
    NumberStyle {
        digits: 2,
        leading_zero: true,
        grouping: 3,
        decimal: [u16::from(b'.'), 0, 0, 0],
        thousand: [u16::from(b','), 0, 0, 0],
        negative: 1,
        positive: 0,
        symbol,
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn GetNumberFormatEx(
    locale: usize,
    flags: u32,
    value: usize,
    format: usize,
    buffer: usize,
    capacity: i32,
) -> i32 {
    let Some(english) = which(locale) else {
        return fail(ERROR_INVALID_PARAMETER);
    };
    if format != 0 && flags != 0 {
        return fail(ERROR_INVALID_FLAGS);
    }
    let mut style = locale_style(english);
    if !read_style(format, false, &mut style) {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let mut body = Out::new();
    let Some(negative) = format_number(value, &style, &mut body) else {
        return fail(ERROR_INVALID_PARAMETER);
    };
    let mut out = Out::new();
    let digits = &body.units[..body.len];
    // NegativeOrder: 0 "(1.1)", 1 "-1.1", 2 "- 1.1", 3 "1.1-", 4 "1.1 -".
    let (before, after): (&str, &str) = if !negative {
        ("", "")
    } else {
        match style.negative {
            0 => ("(", ")"),
            2 => ("- ", ""),
            3 => ("", "-"),
            4 => ("", " -"),
            _ => ("-", ""),
        }
    };
    out.push_str(before);
    for &u in digits {
        out.push(u);
    }
    out.push_str(after);
    out.deliver(buffer, capacity)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetNumberFormatW(
    lcid: u32,
    flags: u32,
    value: usize,
    format: usize,
    buffer: usize,
    capacity: i32,
) -> i32 {
    if lcid_english(lcid).is_none() {
        return fail(ERROR_INVALID_PARAMETER);
    }
    GetNumberFormatEx(0, flags, value, format, buffer, capacity)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrencyFormatEx(
    locale: usize,
    flags: u32,
    value: usize,
    format: usize,
    buffer: usize,
    capacity: i32,
) -> i32 {
    let Some(english) = which(locale) else {
        return fail(ERROR_INVALID_PARAMETER);
    };
    if format != 0 && flags != 0 {
        return fail(ERROR_INVALID_FLAGS);
    }
    let mut style = locale_style(english);
    style.negative = 0;
    if !read_style(format, true, &mut style) {
        return fail(ERROR_INVALID_PARAMETER);
    }
    let mut body = Out::new();
    let Some(negative) = format_number(value, &style, &mut body) else {
        return fail(ERROR_INVALID_PARAMETER);
    };
    let symbol_len = style.symbol.iter().position(|&u| u == 0).unwrap_or(8);
    let symbol = &style.symbol[..symbol_len];
    let digits = &body.units[..body.len];
    let mut out = Out::new();
    let mut emit = |parts: &[&[u16]]| {
        for part in parts {
            for &u in *part {
                out.push(u);
            }
        }
    };
    let open: &[u16] = &[u16::from(b'(')];
    let close: &[u16] = &[u16::from(b')')];
    let minus: &[u16] = &[u16::from(b'-')];
    let space: &[u16] = &[u16::from(b' ')];
    if !negative {
        // PositiveOrder: 0 "$1.1", 1 "1.1$", 2 "$ 1.1", 3 "1.1 $".
        match style.positive {
            1 => emit(&[digits, symbol]),
            2 => emit(&[symbol, space, digits]),
            3 => emit(&[digits, space, symbol]),
            _ => emit(&[symbol, digits]),
        }
    } else {
        // NegativeOrder (the common ones): 0 "($1.1)", 1 "-$1.1",
        // 2 "$-1.1", 3 "$1.1-", 4 "(1.1$)", 5 "-1.1$", 8 "-1.1 $",
        // 9 "-$ 1.1", 12 "$ -1.1", 14 "($ 1.1)", 15 "(1.1 $)".
        match style.negative {
            1 => emit(&[minus, symbol, digits]),
            2 => emit(&[symbol, minus, digits]),
            3 => emit(&[symbol, digits, minus]),
            4 => emit(&[open, digits, symbol, close]),
            5 => emit(&[minus, digits, symbol]),
            8 => emit(&[minus, digits, space, symbol]),
            9 => emit(&[minus, symbol, space, digits]),
            12 => emit(&[symbol, space, minus, digits]),
            14 => emit(&[open, symbol, space, digits, close]),
            15 => emit(&[open, digits, space, symbol, close]),
            _ => emit(&[open, symbol, digits, close]),
        }
    }
    out.deliver(buffer, capacity)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetCurrencyFormatW(
    lcid: u32,
    flags: u32,
    value: usize,
    format: usize,
    buffer: usize,
    capacity: i32,
) -> i32 {
    if lcid_english(lcid).is_none() {
        return fail(ERROR_INVALID_PARAMETER);
    }
    GetCurrencyFormatEx(0, flags, value, format, buffer, capacity)
}
