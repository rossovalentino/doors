//! The NLS functions .NET's globalization calls when no ICU is present
//! (its Windows fallback), for the locales `locale` knows (en-US and the
//! invariant locale): string comparison (ordinal and linguistic), search,
//! locale enumeration and resolution, Gregorian calendar data, time
//! formats, the user's UI languages and geographic location, and
//! code-page descriptions.
//!
//! Linguistic order follows Windows' sort for these locales in its shape:
//! word sort (hyphen and apostrophe ignored at the first level unless
//! SORT_STRINGSORT), symbols before digits before letters, letters
//! compared case-insensitively first and lowercase before uppercase at
//! the case level. Collation tables for other scripts and diacritic
//! levels are M0e's (ICU4X, if the owner approves it).
use crate::locale::{
    INVALID_FLAGS, INVALID_PARAMETER, ctype1, give_str, last_error, map_case, units, which,
};
use core::cmp::Ordering;

const CSTR_LESS_THAN: i32 = 1;
const CSTR_EQUAL: i32 = 2;
const CSTR_GREATER_THAN: i32 = 3;
const NORM_IGNORECASE: u32 = 0x1;
const NORM_IGNORENONSPACE: u32 = 0x2;
const NORM_IGNORESYMBOLS: u32 = 0x4;
const LINGUISTIC_IGNORECASE: u32 = 0x10;
const LINGUISTIC_IGNOREDIACRITIC: u32 = 0x20;
const NORM_IGNOREKANATYPE: u32 = 0x1_0000;
const NORM_IGNOREWIDTH: u32 = 0x2_0000;
const NORM_LINGUISTIC_CASING: u32 = 0x800_0000;
const SORT_STRINGSORT: u32 = 0x1000;
const SORT_DIGITSASNUMBERS: u32 = 0x8;
const COMPARE_FLAGS: u32 = NORM_IGNORECASE
    | NORM_IGNORENONSPACE
    | NORM_IGNORESYMBOLS
    | LINGUISTIC_IGNORECASE
    | LINGUISTIC_IGNOREDIACRITIC
    | NORM_IGNOREKANATYPE
    | NORM_IGNOREWIDTH
    | NORM_LINGUISTIC_CASING
    | SORT_STRINGSORT
    | SORT_DIGITSASNUMBERS;
const FIND_STARTSWITH: u32 = 0x10_0000;
const FIND_ENDSWITH: u32 = 0x20_0000;
const FIND_FROMSTART: u32 = 0x40_0000;
const FIND_FROMEND: u32 = 0x80_0000;
const PUNCT: u16 = 0x10;
const SPACE: u16 = 0x8;

fn result(order: Ordering) -> i32 {
    match order {
        Ordering::Less => CSTR_LESS_THAN,
        Ordering::Equal => CSTR_EQUAL,
        Ordering::Greater => CSTR_GREATER_THAN,
    }
}
fn upper(unit: u16) -> u16 {
    map_case(unit, true)
}
fn lower(unit: u16) -> u16 {
    map_case(unit, false)
}

// ---- ordinal ----------------------------------------------------------------------

/// `CompareStringOrdinal`: code unit order, case folded to uppercase (as
/// Windows' ordinal ignore-case does) when asked.
#[unsafe(no_mangle)]
pub extern "system" fn CompareStringOrdinal(
    first: usize,
    first_length: i32,
    second: usize,
    second_length: i32,
    ignore_case: i32,
) -> i32 {
    let (Some(a), Some(b)) = (units(first, first_length), units(second, second_length)) else {
        last_error(INVALID_PARAMETER);
        return 0;
    };
    let order = if ignore_case != 0 {
        a.iter().map(|u| upper(*u)).cmp(b.iter().map(|u| upper(*u)))
    } else {
        a.cmp(b)
    };
    result(order)
}
/// The first (or last) place `value` occurs in `source` under `same`:
/// its index, or None. STARTSWITH/ENDSWITH anchor it.
fn find(
    source: &[u16],
    value: &[u16],
    flags: u32,
    same: impl Fn(u16, u16) -> bool,
) -> Option<usize> {
    if value.len() > source.len() {
        return None;
    }
    let at = |i: usize| {
        source[i..i + value.len()]
            .iter()
            .zip(value)
            .all(|(a, b)| same(*a, *b))
    };
    let last = source.len() - value.len();
    if flags & FIND_STARTSWITH != 0 {
        return at(0).then_some(0);
    }
    if flags & FIND_ENDSWITH != 0 {
        return at(last).then_some(last);
    }
    if flags & FIND_FROMEND != 0 {
        (0..=last).rev().find(|&i| at(i))
    } else {
        (0..=last).find(|&i| at(i))
    }
}
/// `FindStringOrdinal`: the index of `value` in `source`, or -1 (with
/// ERROR_SUCCESS) when absent.
#[unsafe(no_mangle)]
pub extern "system" fn FindStringOrdinal(
    flags: u32,
    source: usize,
    source_length: i32,
    value: usize,
    value_length: i32,
    ignore_case: i32,
) -> i32 {
    let modes = FIND_STARTSWITH | FIND_ENDSWITH | FIND_FROMSTART | FIND_FROMEND;
    if flags & !modes != 0 || (flags & modes).count_ones() > 1 {
        last_error(INVALID_FLAGS);
        return -1;
    }
    let (Some(s), Some(v)) = (units(source, source_length), units(value, value_length)) else {
        last_error(INVALID_PARAMETER);
        return -1;
    };
    let found = if ignore_case != 0 {
        find(s, v, flags, |a, b| upper(a) == upper(b))
    } else {
        find(s, v, flags, |a, b| a == b)
    };
    last_error(0);
    found.map_or(-1, |i| i as i32)
}

// ---- linguistic -----------------------------------------------------------------------

/// Whether `unit` takes no part at the first level under `flags`.
fn ignorable(unit: u16, flags: u32) -> bool {
    // Combining marks carry only diacritic weight.
    if (0x300..0x370).contains(&unit) {
        return true;
    }
    if flags & SORT_STRINGSORT == 0 && matches!(unit, 0x2d | 0x27 | 0xad | 0x2010 | 0x2019) {
        return true;
    }
    flags & NORM_IGNORESYMBOLS != 0 && ctype1(unit) & (PUNCT | SPACE) != 0
}
/// First-level weight: symbols, then digits, then letters (case folded).
fn primary(unit: u16) -> u32 {
    let class = ctype1(unit);
    if class & 0x100 != 0 {
        0x3_0000 + u32::from(lower(unit))
    } else if class & 0x4 != 0 {
        0x2_0000 + u32::from(unit)
    } else {
        0x1_0000 + u32::from(unit)
    }
}
/// Order two strings as `CompareStringEx` does for `flags`.
fn linguistic(a: &[u16], b: &[u16], flags: u32) -> Ordering {
    let keep = |u: &&u16| !ignorable(**u, flags);
    let first = a
        .iter()
        .filter(keep)
        .map(|u| primary(*u))
        .cmp(b.iter().filter(keep).map(|u| primary(*u)));
    if first != Ordering::Equal || flags & (NORM_IGNORECASE | LINGUISTIC_IGNORECASE) != 0 {
        return first;
    }
    // Case level: lowercase sorts first.
    let case = |u: &u16| u8::from(lower(*u) != *u);
    let second = a
        .iter()
        .filter(keep)
        .map(case)
        .cmp(b.iter().filter(keep).map(case));
    if second != Ordering::Equal {
        return second;
    }
    // Ignored characters still order otherwise-equal strings, unless
    // symbols were asked to be ignored.
    if flags & NORM_IGNORESYMBOLS != 0 {
        Ordering::Equal
    } else {
        a.len().cmp(&b.len())
    }
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn CompareStringEx(
    locale: usize,
    flags: u32,
    first: usize,
    first_length: i32,
    second: usize,
    second_length: i32,
    _version: usize,
    _reserved: usize,
    _parameter: usize,
) -> i32 {
    if which(locale).is_none() {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    if flags & !COMPARE_FLAGS != 0 {
        last_error(INVALID_FLAGS);
        return 0;
    }
    let (Some(a), Some(b)) = (units(first, first_length), units(second, second_length)) else {
        last_error(INVALID_PARAMETER);
        return 0;
    };
    result(linguistic(a, b, flags))
}
/// `CompareStringW` by LCID (the user and system defaults, en-US and the
/// invariant locale).
#[unsafe(no_mangle)]
pub extern "system" fn CompareStringW(
    lcid: u32,
    flags: u32,
    first: usize,
    first_length: i32,
    second: usize,
    second_length: i32,
) -> i32 {
    if !matches!(lcid, 0 | 0x400 | 0x800 | 0x409 | 0x7f) {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    CompareStringEx(
        0,
        flags,
        first,
        first_length,
        second,
        second_length,
        0,
        0,
        0,
    )
}
/// `FindNLSStringEx`: `value` in `source` compared as `CompareStringEx`
/// compares characters (case folded when asked); the index, with the
/// length found, or -1.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn FindNLSStringEx(
    locale: usize,
    flags: u32,
    source: usize,
    source_length: i32,
    value: usize,
    value_length: i32,
    found_length: usize,
    _version: usize,
    _reserved: usize,
    _sort_handle: usize,
) -> i32 {
    let modes = FIND_STARTSWITH | FIND_ENDSWITH | FIND_FROMSTART | FIND_FROMEND;
    if which(locale).is_none() || flags & !(modes | COMPARE_FLAGS) != 0 {
        last_error(INVALID_FLAGS);
        return -1;
    }
    let (Some(s), Some(v)) = (units(source, source_length), units(value, value_length)) else {
        last_error(INVALID_PARAMETER);
        return -1;
    };
    let fold = flags & (NORM_IGNORECASE | LINGUISTIC_IGNORECASE) != 0;
    let found = find(s, v, flags, |a, b| {
        if fold { upper(a) == upper(b) } else { a == b }
    });
    if let Some(i) = found
        && found_length != 0
    {
        unsafe { (found_length as *mut i32).write_unaligned(v.len() as i32) };
        last_error(0);
        return i as i32;
    }
    last_error(0);
    found.map_or(-1, |i| i as i32)
}
/// Every assigned UTF-16 string is defined for NLS here.
#[unsafe(no_mangle)]
pub extern "system" fn IsNLSDefinedString(
    _function: u32,
    _flags: u32,
    _version: usize,
    text: usize,
    length: i32,
) -> i32 {
    match units(text, length) {
        Some(t) => i32::from(t.iter().all(|u| *u != 0xfffe && *u != 0xffff)),
        None => {
            last_error(INVALID_PARAMETER);
            0
        }
    }
}
/// The sort version .NET's SortVersion reports (NLSVERSIONINFOEX: size,
/// NLS version, defined version, effective LCID, custom-version GUID).
#[unsafe(no_mangle)]
pub extern "system" fn GetNLSVersionEx(_function: u32, locale: usize, info: usize) -> i32 {
    let Some(en_us) = which(locale) else {
        last_error(INVALID_PARAMETER);
        return 0;
    };
    if info == 0 || unsafe { (info as *const u32).read_unaligned() } < 32 {
        last_error(122);
        return 0;
    }
    let mut out = [0u8; 32];
    out[..4].copy_from_slice(&32u32.to_le_bytes());
    out[4..8].copy_from_slice(&0x0006_0305u32.to_le_bytes());
    out[8..12].copy_from_slice(&0x0006_0305u32.to_le_bytes());
    out[12..16].copy_from_slice(&(if en_us { 0x409u32 } else { 0x7f }).to_le_bytes());
    unsafe { core::ptr::copy_nonoverlapping(out.as_ptr(), info as *mut u8, 32) };
    1
}

// ---- locales -------------------------------------------------------------------------

/// `ResolveLocaleName`: the specific locale a name stands for.
#[unsafe(no_mangle)]
pub extern "system" fn ResolveLocaleName(name: usize, buffer: usize, capacity: i32) -> i32 {
    match which(name) {
        Some(true) => give_str("en-US", buffer, capacity),
        Some(false) => give_str("", buffer, capacity),
        None => {
            last_error(INVALID_PARAMETER);
            0
        }
    }
}
/// `EnumSystemLocalesEx`: en-US (a specific Windows locale) and its
/// neutral parent "en", for the kinds asked (LOCALE_ALL 0, WINDOWS 1,
/// NEUTRALDATA 0x10, SPECIFICDATA 0x20).
#[unsafe(no_mangle)]
pub extern "system" fn EnumSystemLocalesEx(
    callback: usize,
    flags: u32,
    parameter: isize,
    _reserved: usize,
) -> i32 {
    if callback == 0 {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    let callback: extern "system" fn(usize, u32, isize) -> i32 =
        unsafe { core::mem::transmute(callback) };
    for (name, kind) in [("en", 0x1 | 0x10), ("en-US", 0x1 | 0x20)] {
        if flags != 0 && flags & 0x1 == 0 && flags & kind & 0x30 == 0 {
            continue;
        }
        let mut wide = [0u16; 8];
        for (slot, unit) in wide.iter_mut().zip(name.encode_utf16()) {
            *slot = unit;
        }
        if callback(wide.as_ptr() as usize, kind, parameter) == 0 {
            break;
        }
    }
    1
}
/// `GetUserPreferredUILanguages`: en-US (MUI_LANGUAGE_NAME 8; ID 4 as
/// "0409"), a double-NUL-terminated list.
#[unsafe(no_mangle)]
pub extern "system" fn GetUserPreferredUILanguages(
    flags: u32,
    count: usize,
    buffer: usize,
    size: usize,
) -> i32 {
    if count == 0 || size == 0 {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    let text: &str = if flags & 0x4 != 0 { "0409" } else { "en-US" };
    let needed = text.len() + 2;
    let capacity = unsafe { (size as *const u32).read_unaligned() } as usize;
    unsafe { (count as *mut u32).write_unaligned(1) };
    unsafe { (size as *mut u32).write_unaligned(needed as u32) };
    if buffer == 0 {
        return 1;
    }
    if capacity < needed {
        last_error(122);
        return 0;
    }
    for (i, unit) in text.encode_utf16().chain([0, 0]).enumerate() {
        unsafe { (buffer as *mut u16).add(i).write_unaligned(unit) };
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetUserDefaultGeoName(buffer: usize, capacity: i32) -> i32 {
    give_str("US", buffer, capacity)
}
/// `GetUserGeoID(GEOCLASS_NATION)`: the United States (244).
#[unsafe(no_mangle)]
pub extern "system" fn GetUserGeoID(class: u32) -> i32 {
    if class == 16 { 244 } else { -1 }
}
/// `GetGeoInfoW` for the United States.
#[unsafe(no_mangle)]
pub extern "system" fn GetGeoInfoW(
    location: i32,
    kind: u32,
    data: usize,
    capacity: i32,
    _language: u16,
) -> i32 {
    if location != 244 {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    let text = match kind {
        1 => "244",                      // GEO_NATION
        2 => "39.45",                    // GEO_LATITUDE
        3 => "-98.908",                  // GEO_LONGITUDE
        4 => "US",                       // GEO_ISO2
        5 => "USA",                      // GEO_ISO3
        6 => "en-US",                    // GEO_RFC1766
        7 => "1033",                     // GEO_LCID
        8 => "United States",            // GEO_FRIENDLYNAME
        9 => "United States of America", // GEO_OFFICIALNAME
        12 => "840",                     // GEO_ISO_UN_NUMBER
        13 => "244",                     // GEO_PARENT (none here)
        14 => "+1",                      // GEO_DIALINGCODE
        15 => "USD",                     // GEO_CURRENCYCODE
        16 => "$",                       // GEO_CURRENCYSYMBOL
        17 => "US",                      // GEO_NAME
        _ => {
            last_error(INVALID_FLAGS);
            return 0;
        }
    };
    give_str(text, data, capacity)
}

// ---- calendars and formats ---------------------------------------------------------------

const DAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];
const DAYS_SHORT: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const DAYS_SHORTEST: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];
const MONTHS: [&str; 13] = [
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
    "",
];
const MONTHS_SHORT: [&str; 13] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec", "",
];
/// CALTYPE's text for the Gregorian calendar in en-US (the invariant
/// locale shares its names); None for an unknown type.
fn calendar_text(kind: u32) -> Option<&'static str> {
    Some(match kind {
        0x01 => "1",
        0x02 => "Gregorian Calendar",
        0x03 => "0",
        0x04 => "A.D.",
        0x05 => "M/d/yyyy",
        0x06 => "dddd, MMMM d, yyyy",
        0x07..=0x0d => DAYS[(kind - 0x07) as usize],
        0x0e..=0x14 => DAYS_SHORT[(kind - 0x0e) as usize],
        0x15..=0x21 => MONTHS[(kind - 0x15) as usize],
        0x22..=0x2e => MONTHS_SHORT[(kind - 0x22) as usize],
        0x2f => "MMMM yyyy",
        0x30 => "2049",
        0x31..=0x37 => DAYS_SHORTEST[(kind - 0x31) as usize],
        0x38 => "MMMM d",
        0x39 => "AD",
        0x3a => "dddd, MMMM d",
        0x3b => "A.D.",
        0x3c => "AD",
        _ => return None,
    })
}
const CAL_RETURN_NUMBER: u32 = 0x2000_0000;
const CAL_FLAGS: u32 = 0x8000_0000 | 0x4000_0000 | CAL_RETURN_NUMBER | 0x1000_0000;
#[unsafe(no_mangle)]
pub extern "system" fn GetCalendarInfoEx(
    locale: usize,
    calendar: u32,
    _reserved: usize,
    kind: u32,
    data: usize,
    capacity: i32,
    value: usize,
) -> i32 {
    if which(locale).is_none() || calendar != 1 {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    let Some(text) = calendar_text(kind & !CAL_FLAGS) else {
        last_error(INVALID_FLAGS);
        return 0;
    };
    if kind & CAL_RETURN_NUMBER != 0 {
        let Ok(number) = text.parse::<u32>() else {
            last_error(INVALID_FLAGS);
            return 0;
        };
        if value == 0 {
            last_error(INVALID_PARAMETER);
            return 0;
        }
        unsafe { (value as *mut u32).write_unaligned(number) };
        return 2;
    }
    give_str(text, data, capacity)
}
/// The values `EnumCalendarInfoExEx` lists for a CALTYPE (the date
/// formats Windows offers for en-US; one value for the other types).
fn calendar_values(kind: u32) -> &'static [&'static str] {
    match kind {
        0x05 => &[
            "M/d/yyyy",
            "M/d/yy",
            "MM/dd/yy",
            "MM/dd/yyyy",
            "yy/MM/dd",
            "yyyy-MM-dd",
            "dd-MMM-yy",
        ],
        0x06 => &[
            "dddd, MMMM d, yyyy",
            "MMMM d, yyyy",
            "dddd, d MMMM, yyyy",
            "d MMMM, yyyy",
        ],
        0x2f => &["MMMM yyyy"],
        0x38 => &["MMMM d"],
        _ => &[],
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn EnumCalendarInfoExEx(
    callback: usize,
    locale: usize,
    calendar: u32,
    _reserved: usize,
    kind: u32,
    parameter: isize,
) -> i32 {
    // ENUM_ALL_CALENDARS is 0xffffffff.
    if callback == 0 || which(locale).is_none() || !matches!(calendar, 1 | u32::MAX) {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    let callback: extern "system" fn(usize, u32, usize, isize) -> i32 =
        unsafe { core::mem::transmute(callback) };
    let kind = kind & !CAL_FLAGS;
    let single = [calendar_text(kind).unwrap_or("")];
    let values = match calendar_values(kind) {
        [] if calendar_text(kind).is_some() => &single[..],
        [] => {
            last_error(INVALID_FLAGS);
            return 0;
        }
        many => many,
    };
    for text in values {
        let mut wide = [0u16; 48];
        for (slot, unit) in wide.iter_mut().zip(text.encode_utf16()) {
            *slot = unit;
        }
        if callback(wide.as_ptr() as usize, 1, 0, parameter) == 0 {
            break;
        }
    }
    1
}
/// `EnumTimeFormatsEx`: en-US's time formats (TIME_NOSECONDS 2: the
/// short ones).
#[unsafe(no_mangle)]
pub extern "system" fn EnumTimeFormatsEx(
    callback: usize,
    locale: usize,
    flags: u32,
    parameter: isize,
) -> i32 {
    if callback == 0 || which(locale).is_none() || flags & !2 != 0 {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    let callback: extern "system" fn(usize, isize) -> i32 =
        unsafe { core::mem::transmute(callback) };
    let formats: &[&str] = if flags & 2 != 0 {
        &["h:mm tt", "hh:mm tt", "H:mm", "HH:mm"]
    } else {
        &["h:mm:ss tt", "hh:mm:ss tt", "H:mm:ss", "HH:mm:ss"]
    };
    for text in formats {
        let mut wide = [0u16; 16];
        for (slot, unit) in wide.iter_mut().zip(text.encode_utf16()) {
            *slot = unit;
        }
        if callback(wide.as_ptr() as usize, parameter) == 0 {
            break;
        }
    }
    1
}

// ---- code pages ---------------------------------------------------------------------------

/// `GetCPInfoExW`: CPINFOEXW (MaxCharSize, DefaultChar, LeadByte,
/// UnicodeDefaultChar, CodePage, CodePageName) for the code pages Doors
/// converts.
#[unsafe(no_mangle)]
pub extern "system" fn GetCPInfoExW(code_page: u32, _flags: u32, info: usize) -> i32 {
    let page = match code_page {
        0 | 3 => 1252,
        1 | 2 => 437,
        other => other,
    };
    let (size, name) = match page {
        65001 => (4, "65001 (UTF-8)"),
        1252 => (1, "1252  (ANSI - Latin I)"),
        437 => (1, "437   (OEM - United States)"),
        20127 => (1, "20127 (US-ASCII)"),
        28591 => (1, "28591 (ISO 8859-1 Latin I)"),
        _ => {
            last_error(INVALID_PARAMETER);
            return 0;
        }
    };
    if info == 0 {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    let mut out = [0u8; 544];
    out[..4].copy_from_slice(&(size as u32).to_le_bytes());
    out[4] = b'?';
    out[18..20].copy_from_slice(&0xfffdu16.to_le_bytes());
    out[20..24].copy_from_slice(&page.to_le_bytes());
    for (i, unit) in name.encode_utf16().enumerate() {
        out[24 + 2 * i..26 + 2 * i].copy_from_slice(&unit.to_le_bytes());
    }
    unsafe { core::ptr::copy_nonoverlapping(out.as_ptr(), info as *mut u8, out.len()) };
    1
}
