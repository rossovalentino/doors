//! Number conversions (api-ms-win-crt-convert): strtol and strtod's
//! families over `door_crt::number`, the `_itoa` family and their secure
//! forms, and UTF-8 / UTF-16 conversions (mbstowcs, wcstombs, mbtowc...)
//! for the UTF-8 process code page.
use super::{errno, invalid_parameter, set_errno};
use door_crt::number;

/// Unit `i` of the string at `s`, never reading past its terminator (the
/// parsers look ahead): units up to `checked` are known to precede it.
unsafe fn reader<T: Copy + Into<u32>>(s: *const T) -> impl Fn(usize) -> u16 {
    let checked = core::cell::Cell::new(0usize);
    let ended = core::cell::Cell::new(false);
    move |i| {
        while checked.get() < i {
            if ended.get() {
                return 0;
            }
            let u: u32 = unsafe { *s.add(checked.get()) }.into();
            if u == 0 {
                ended.set(true);
                return 0;
            }
            checked.set(checked.get() + 1);
        }
        let u: u32 = unsafe { *s.add(i) }.into();
        if u == 0 {
            ended.set(true);
        }
        u as u16
    }
}
unsafe fn integer<T: Copy + Into<u32>>(
    s: *const T,
    end: *mut *const T,
    base: i32,
) -> Option<number::Integer> {
    if s.is_null() || !(base == 0 || (2..=36).contains(&base)) {
        invalid_parameter();
        set_errno(errno::EINVAL);
        if !end.is_null() {
            unsafe { *end = s };
        }
        return None;
    }
    let get = unsafe { reader(s) };
    let r = number::integer(&get, base as u32)?;
    if !end.is_null() {
        unsafe { *end = s.add(r.end) };
    }
    Some(r)
}
macro_rules! signed {
    ($($name:ident: $t:ty, $ret:ty, $bits:expr;)*) => {$(
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(s: *const $t, end: *mut *const $t, base: i32) -> $ret {
            let Some(r) = (unsafe { integer(s, end, base) }) else { return 0 };
            let (v, range) = number::signed(r, $bits);
            if range {
                set_errno(errno::ERANGE);
            }
            v as $ret
        }
    )*};
}
macro_rules! unsigned {
    ($($name:ident: $t:ty, $ret:ty, $bits:expr;)*) => {$(
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(s: *const $t, end: *mut *const $t, base: i32) -> $ret {
            let Some(r) = (unsafe { integer(s, end, base) }) else { return 0 };
            let (v, range) = number::unsigned(r, $bits);
            if range {
                set_errno(errno::ERANGE);
            }
            v as $ret
        }
    )*};
}
signed! {
    strtol: u8, i32, 32;
    wcstol: u16, i32, 32;
    strtoll: u8, i64, 64;
    wcstoll: u16, i64, 64;
    _strtoi64: u8, i64, 64;
    _wcstoi64: u16, i64, 64;
    strtoimax: u8, i64, 64;
    wcstoimax: u16, i64, 64;
}
unsigned! {
    strtoul: u8, u32, 32;
    wcstoul: u16, u32, 32;
    strtoull: u8, u64, 64;
    wcstoull: u16, u64, 64;
    _strtoui64: u8, u64, 64;
    _wcstoui64: u16, u64, 64;
    strtoumax: u8, u64, 64;
    wcstoumax: u16, u64, 64;
}
macro_rules! ascii_to {
    ($($name:ident: $t:ty, $ret:ty, $bits:expr;)*) => {$(
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(s: *const $t) -> $ret {
            if s.is_null() {
                invalid_parameter();
                set_errno(errno::EINVAL);
                return 0;
            }
            let get = unsafe { reader(s) };
            let Some(r) = number::integer(&get, 10) else { return 0 };
            let (v, range) = number::signed(r, $bits);
            if range {
                set_errno(errno::ERANGE);
            }
            v as $ret
        }
    )*};
}
ascii_to! {
    atoi: u8, i32, 32;
    atol: u8, i32, 32;
    atoll: u8, i64, 64;
    _atoi64: u8, i64, 64;
    _wtoi: u16, i32, 32;
    _wtol: u16, i32, 32;
    _wtoll: u16, i64, 64;
    _wtoi64: u16, i64, 64;
}
unsafe fn float<T: Copy + Into<u32>>(s: *const T, end: *mut *const T) -> f64 {
    if s.is_null() {
        invalid_parameter();
        set_errno(errno::EINVAL);
        return 0.0;
    }
    let get = unsafe { reader(s) };
    let (v, n, range) = number::float(&get);
    if !end.is_null() {
        unsafe { *end = s.add(n) };
    }
    if range {
        set_errno(errno::ERANGE);
    }
    v
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtod(s: *const u8, end: *mut *const u8) -> f64 {
    unsafe { float(s, end) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcstod(s: *const u16, end: *mut *const u16) -> f64 {
    unsafe { float(s, end) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtold(s: *const u8, end: *mut *const u8) -> f64 {
    unsafe { float(s, end) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcstold(s: *const u16, end: *mut *const u16) -> f64 {
    unsafe { float(s, end) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn strtof(s: *const u8, end: *mut *const u8) -> f32 {
    let v = unsafe { float(s, end) };
    let f = v as f32;
    if f.is_infinite() && v.is_finite() {
        set_errno(errno::ERANGE);
    }
    f
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcstof(s: *const u16, end: *mut *const u16) -> f32 {
    let v = unsafe { float(s, end) };
    let f = v as f32;
    if f.is_infinite() && v.is_finite() {
        set_errno(errno::ERANGE);
    }
    f
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atof(s: *const u8) -> f64 {
    unsafe { float(s, core::ptr::null_mut()) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wtof(s: *const u16) -> f64 {
    unsafe { float(s, core::ptr::null_mut()) }
}
// The `_l` (locale) forms: the "C" locale's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _strtod_l(s: *const u8, end: *mut *const u8, _l: usize) -> f64 {
    unsafe { float(s, end) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcstod_l(s: *const u16, end: *mut *const u16, _l: usize) -> f64 {
    unsafe { float(s, end) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _strtol_l(s: *const u8, end: *mut *const u8, base: i32, _l: usize) -> i32 {
    unsafe { strtol(s, end, base) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _strtoul_l(
    s: *const u8,
    end: *mut *const u8,
    base: i32,
    _l: usize,
) -> u32 {
    unsafe { strtoul(s, end, base) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcstol_l(
    s: *const u16,
    end: *mut *const u16,
    base: i32,
    _l: usize,
) -> i32 {
    unsafe { wcstol(s, end, base) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _wcstoul_l(
    s: *const u16,
    end: *mut *const u16,
    base: i32,
    _l: usize,
) -> u32 {
    unsafe { wcstoul(s, end, base) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _strtoi64_l(
    s: *const u8,
    end: *mut *const u8,
    base: i32,
    _l: usize,
) -> i64 {
    unsafe { _strtoi64(s, end, base) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn _strtoui64_l(
    s: *const u8,
    end: *mut *const u8,
    base: i32,
    _l: usize,
) -> u64 {
    unsafe { _strtoui64(s, end, base) }
}

/// The `_itoa_s` family: `value` in `radix` into `buffer` (`capacity`
/// units), negative only for signed decimal.
unsafe fn to_text<T: Copy>(
    value: u64,
    negative: bool,
    buffer: *mut T,
    capacity: usize,
    radix: i32,
    unit: fn(u8) -> T,
) -> i32 {
    if buffer.is_null() || capacity == 0 || !(2..=36).contains(&radix) {
        if !buffer.is_null() && capacity > 0 {
            unsafe { *buffer = unit(0) };
        }
        invalid_parameter();
        set_errno(errno::EINVAL);
        return errno::EINVAL;
    }
    let mut digits = [0u8; 66];
    let mut n = 0;
    let mut v = value;
    loop {
        digits[n] = b"0123456789abcdefghijklmnopqrstuvwxyz"[(v % radix as u64) as usize];
        n += 1;
        v /= radix as u64;
        if v == 0 {
            break;
        }
    }
    let total = n + usize::from(negative);
    if total + 1 > capacity {
        unsafe { *buffer = unit(0) };
        invalid_parameter();
        set_errno(errno::ERANGE);
        return errno::ERANGE;
    }
    let mut k = 0;
    if negative {
        unsafe { *buffer = unit(b'-') };
        k = 1;
    }
    for i in (0..n).rev() {
        unsafe { *buffer.add(k) = unit(digits[i]) };
        k += 1;
    }
    unsafe { *buffer.add(k) = unit(0) };
    0
}
fn narrow(b: u8) -> u8 {
    b
}
fn wide(b: u8) -> u16 {
    u16::from(b)
}
macro_rules! to_text {
    ($($name:ident, $plain:ident: $v:ty, $t:ty, $unit:ident, $signed:expr;)*) => {$(
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(value: $v, buffer: *mut $t, capacity: usize, radix: i32) -> i32 {
            let negative = $signed && radix == 10 && (value as i64) < 0;
            let magnitude = if negative {
                (value as i64).unsigned_abs()
            } else if $signed {
                // A negative value in another radix prints its bits.
                (value as i64 as u64) & (u64::MAX >> (64 - 8 * core::mem::size_of::<$v>()))
            } else {
                value as u64
            };
            unsafe { to_text(magnitude, negative, buffer, capacity, radix, $unit) }
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $plain(value: $v, buffer: *mut $t, radix: i32) -> *mut $t {
            unsafe { $name(value, buffer, 66, radix) };
            buffer
        }
    )*};
}
to_text! {
    _itoa_s, _itoa: i32, u8, narrow, true;
    _ltoa_s, _ltoa: i32, u8, narrow, true;
    _ultoa_s, _ultoa: u32, u8, narrow, false;
    _i64toa_s, _i64toa: i64, u8, narrow, true;
    _ui64toa_s, _ui64toa: u64, u8, narrow, false;
    _itow_s, _itow: i32, u16, wide, true;
    _ltow_s, _ltow: i32, u16, wide, true;
    _ultow_s, _ultow: u32, u16, wide, false;
    _i64tow_s, _i64tow: i64, u16, wide, true;
    _ui64tow_s, _ui64tow: u64, u16, wide, false;
}

// UTF-8 (the process code page) to and from UTF-16.

/// Decode one UTF-8 character at `s` (at most `max` bytes): (code point,
/// bytes), None for an invalid or incomplete sequence.
pub(crate) unsafe fn decode(s: *const u8, max: usize) -> Option<(u32, usize)> {
    if max == 0 {
        return None;
    }
    let first = unsafe { *s };
    let (n, mut c) = match first {
        0..=0x7F => return Some((u32::from(first), 1)),
        0xC2..=0xDF => (2, u32::from(first & 0x1F)),
        0xE0..=0xEF => (3, u32::from(first & 0x0F)),
        0xF0..=0xF4 => (4, u32::from(first & 0x07)),
        _ => return None,
    };
    if max < n {
        return None;
    }
    for k in 1..n {
        let b = unsafe { *s.add(k) };
        if b & 0xC0 != 0x80 {
            return None;
        }
        c = c << 6 | u32::from(b & 0x3F);
    }
    let ok = match n {
        3 => c >= 0x800 && !(0xD800..0xE000).contains(&c),
        4 => (0x10000..=0x10FFFF).contains(&c),
        _ => true,
    };
    ok.then_some((c, n))
}
/// Encode `c` as UTF-8 into `out`: the byte count.
pub(crate) fn encode(c: u32, out: &mut [u8; 4]) -> usize {
    char::from_u32(c).map_or(0, |ch| ch.encode_utf8(out).len())
}
/// UTF-16 to UTF-8 for wide output: (code point, units used).
pub(crate) fn wide_char(units: &[u16]) -> (u32, usize) {
    match units {
        [high @ 0xD800..=0xDBFF, low @ 0xDC00..=0xDFFF, ..] => (
            0x10000 + ((u32::from(*high) - 0xD800) << 10) + (u32::from(*low) - 0xDC00),
            2,
        ),
        [u, ..] => (u32::from(*u), 1),
        [] => (0, 0),
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn ___mb_cur_max_func() -> i32 {
    4
}
#[unsafe(no_mangle)]
pub extern "C" fn ___mb_cur_max_l_func(_l: usize) -> i32 {
    4
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbtowc(out: *mut u16, s: *const u8, max: usize) -> i32 {
    if s.is_null() {
        return 0;
    }
    match unsafe { decode(s, max) } {
        Some((0, _)) => {
            if !out.is_null() {
                unsafe { *out = 0 };
            }
            0
        }
        Some((c, n)) if c < 0x10000 => {
            if !out.is_null() {
                unsafe { *out = c as u16 };
            }
            n as i32
        }
        _ => {
            set_errno(errno::EILSEQ);
            -1
        }
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblen(s: *const u8, max: usize) -> i32 {
    unsafe { mbtowc(core::ptr::null_mut(), s, max) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wctomb(out: *mut u8, c: u16) -> i32 {
    if out.is_null() {
        return 0;
    }
    let mut b = [0u8; 4];
    let n = encode(u32::from(c), &mut b);
    if n == 0 {
        set_errno(errno::EILSEQ);
        return -1;
    }
    unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), out, n) };
    n as i32
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wctomb_s(written: *mut i32, out: *mut u8, capacity: usize, c: u16) -> i32 {
    let mut b = [0u8; 4];
    let n = encode(u32::from(c), &mut b);
    if n == 0 {
        set_errno(errno::EILSEQ);
        return errno::EILSEQ;
    }
    if !out.is_null() {
        if capacity < n {
            invalid_parameter();
            return errno::ERANGE;
        }
        unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), out, n) };
    }
    if !written.is_null() {
        unsafe { *written = n as i32 };
    }
    0
}
/// mbstowcs: UTF-8 to UTF-16, at most `count` units (none when `out` is
/// null: the length needed). -1 (as usize) on an invalid sequence.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbstowcs(out: *mut u16, s: *const u8, count: usize) -> usize {
    let mut i = 0usize;
    let mut n = 0usize;
    loop {
        if !out.is_null() && n >= count {
            return n;
        }
        let Some((c, used)) = (unsafe { decode(s.add(i), 4) }) else {
            set_errno(errno::EILSEQ);
            return usize::MAX;
        };
        if c == 0 {
            if !out.is_null() && n < count {
                unsafe { *out.add(n) = 0 };
            }
            return n;
        }
        let mut units = [0u16; 2];
        let encoded = char::from_u32(c).map_or(&[][..], |ch| ch.encode_utf16(&mut units));
        if !out.is_null() && n + encoded.len() > count {
            return n;
        }
        for (k, &u) in encoded.iter().enumerate() {
            if !out.is_null() {
                unsafe { *out.add(n + k) = u };
            }
        }
        n += encoded.len();
        i += used;
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcstombs(out: *mut u8, s: *const u16, count: usize) -> usize {
    let mut i = 0usize;
    let mut n = 0usize;
    loop {
        let units = [
            unsafe { *s.add(i) },
            if unsafe { *s.add(i) } == 0 {
                0
            } else {
                unsafe { *s.add(i + 1) }
            },
        ];
        let (c, used) = wide_char(&units);
        if c == 0 {
            if !out.is_null() && n < count {
                unsafe { *out.add(n) = 0 };
            }
            return n;
        }
        if (0xD800..0xE000).contains(&c) {
            set_errno(errno::EILSEQ);
            return usize::MAX;
        }
        let mut b = [0u8; 4];
        let k = encode(c, &mut b);
        if !out.is_null() {
            if n + k > count {
                return n;
            }
            unsafe { core::ptr::copy_nonoverlapping(b.as_ptr(), out.add(n), k) };
        }
        n += k;
        i += used;
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbstowcs_s(
    converted: *mut usize,
    out: *mut u16,
    capacity: usize,
    s: *const u8,
    count: usize,
) -> i32 {
    let needed = unsafe { mbstowcs(core::ptr::null_mut(), s, usize::MAX) };
    if needed == usize::MAX {
        return errno::EILSEQ;
    }
    let take = if count == usize::MAX {
        needed
    } else {
        needed.min(count)
    };
    if !out.is_null() {
        if take + 1 > capacity {
            if capacity > 0 {
                unsafe { *out = 0 };
            }
            invalid_parameter();
            return errno::ERANGE;
        }
        unsafe {
            mbstowcs(out, s, take);
            *out.add(take) = 0;
        }
    }
    if !converted.is_null() {
        unsafe { *converted = take + 1 };
    }
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcstombs_s(
    converted: *mut usize,
    out: *mut u8,
    capacity: usize,
    s: *const u16,
    count: usize,
) -> i32 {
    let needed = unsafe { wcstombs(core::ptr::null_mut(), s, usize::MAX) };
    if needed == usize::MAX {
        return errno::EILSEQ;
    }
    let take = if count == usize::MAX {
        needed
    } else {
        needed.min(count)
    };
    if !out.is_null() {
        if take + 1 > capacity {
            if capacity > 0 {
                unsafe { *out = 0 };
            }
            invalid_parameter();
            return errno::ERANGE;
        }
        unsafe {
            wcstombs(out, s, take);
            *out.add(take) = 0;
        }
    }
    if !converted.is_null() {
        unsafe { *converted = take + 1 };
    }
    0
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn btowc(c: i32) -> u16 {
    if (0..128).contains(&c) {
        c as u16
    } else {
        0xFFFF
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wctob(c: u16) -> i32 {
    if c < 128 { i32::from(c) } else { -1 }
}
