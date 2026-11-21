//! Locale and character-type queries a runtime makes at start-up (P18 M0c,
//! the first part of M0e): the user and system locale (en-US until the
//! Settings language feeds it), code-page information for UTF-8, the
//! character classes of GetStringTypeW from Unicode's own tables, case
//! mapping and sort keys in LCMapStringEx, and GetLocaleInfoEx for en-US
//! and the invariant locale. Other locales fail with
//! ERROR_INVALID_PARAMETER rather than answer with en-US data.

#[link(name = "ntdll", kind = "dylib")]
unsafe extern "system" {
    fn RtlSetLastWin32Error(value: u32);
}
pub(crate) fn last_error(value: u32) {
    unsafe { RtlSetLastWin32Error(value) };
}
pub(crate) const INVALID_PARAMETER: u32 = 87;
const INSUFFICIENT_BUFFER: u32 = 122;
pub(crate) const INVALID_FLAGS: u32 = 1004;
const CP_UTF8: u32 = 65001;
const EN_US: u32 = 0x0409;
const INVARIANT: u32 = 0x007f;

// ---- UTF-16 helpers ---------------------------------------------------------

/// The UTF-16 string at `at`: `length` units, or up to its NUL at -1.
pub(crate) fn units<'a>(at: usize, length: i32) -> Option<&'a [u16]> {
    if at == 0 {
        return None;
    }
    let length = if length < 0 {
        let mut n = 0usize;
        while unsafe { ((at as *const u16).add(n)).read_unaligned() } != 0 {
            n += 1;
        }
        n
    } else {
        length as usize
    };
    Some(unsafe { core::slice::from_raw_parts(at as *const u16, length) })
}
/// Copy `text` (plus a NUL) to `buffer` of `capacity` units: the units
/// written including the NUL, the units needed when `capacity` is 0, or 0
/// with ERROR_INSUFFICIENT_BUFFER.
pub(crate) fn give(text: &[u16], buffer: usize, capacity: i32) -> i32 {
    let needed = text.len() + 1;
    if capacity == 0 {
        return needed as i32;
    }
    if (capacity as usize) < needed || buffer == 0 {
        last_error(INSUFFICIENT_BUFFER);
        return 0;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(text.as_ptr(), buffer as *mut u16, text.len());
        ((buffer as *mut u16).add(text.len())).write_unaligned(0);
    }
    needed as i32
}
pub(crate) fn give_str(text: &str, buffer: usize, capacity: i32) -> i32 {
    let mut wide = [0u16; 96];
    let mut n = 0;
    for unit in text.encode_utf16() {
        wide[n] = unit;
        n += 1;
    }
    give(&wide[..n], buffer, capacity)
}

// ---- locale identity ---------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn GetUserDefaultLCID() -> u32 {
    EN_US
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemDefaultLCID() -> u32 {
    EN_US
}
#[unsafe(no_mangle)]
pub extern "system" fn GetThreadLocale() -> u32 {
    EN_US
}
#[unsafe(no_mangle)]
pub extern "system" fn GetUserDefaultLangID() -> u16 {
    EN_US as u16
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemDefaultLangID() -> u16 {
    EN_US as u16
}
#[unsafe(no_mangle)]
pub extern "system" fn GetUserDefaultUILanguage() -> u16 {
    EN_US as u16
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemDefaultUILanguage() -> u16 {
    EN_US as u16
}
#[unsafe(no_mangle)]
pub extern "system" fn GetUserDefaultLocaleName(buffer: usize, capacity: i32) -> i32 {
    give_str("en-US", buffer, capacity)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetSystemDefaultLocaleName(buffer: usize, capacity: i32) -> i32 {
    give_str("en-US", buffer, capacity)
}

/// Which locale `name` (a UTF-16 locale name, null for the user default)
/// is: Some(true) en-US, Some(false) invariant, None unknown.
pub(crate) fn which(name: usize) -> Option<bool> {
    let Some(name) = units(name, -1) else {
        return Some(true);
    };
    let mut lower = [0u8; 32];
    if name.len() > lower.len() {
        return None;
    }
    for (slot, unit) in lower.iter_mut().zip(name) {
        *slot = u8::try_from(*unit).ok()?.to_ascii_lowercase();
    }
    match &lower[..name.len()] {
        b"" => Some(false),
        b"en-us" | b"en" | b"!x-sys-default-locale" => Some(true),
        _ => None,
    }
}

// ---- code pages ----------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn GetConsoleOutputCP() -> u32 {
    CP_UTF8
}
#[unsafe(no_mangle)]
pub extern "system" fn GetConsoleCP() -> u32 {
    CP_UTF8
}
/// CPINFO { UINT MaxCharSize; BYTE DefaultChar[2]; BYTE LeadByte[12] }.
#[unsafe(no_mangle)]
pub extern "system" fn GetCPInfo(code_page: u32, info: usize) -> i32 {
    let size = match code_page {
        0..=3 | CP_UTF8 => 4,
        20127 | 1252 | 437 | 28591 => 1,
        _ => {
            last_error(INVALID_PARAMETER);
            return 0;
        }
    };
    unsafe {
        core::ptr::write_bytes(info as *mut u8, 0, 20);
        (info as *mut u32).write_unaligned(size);
        ((info + 4) as *mut u8).write(b'?');
    }
    1
}

// ---- character types ----------------------------------------------------------------

pub(crate) fn ctype1(unit: u16) -> u16 {
    const UPPER: u16 = 0x1;
    const LOWER: u16 = 0x2;
    const DIGIT: u16 = 0x4;
    const SPACE: u16 = 0x8;
    const PUNCT: u16 = 0x10;
    const CNTRL: u16 = 0x20;
    const BLANK: u16 = 0x40;
    const XDIGIT: u16 = 0x80;
    const ALPHA: u16 = 0x100;
    const DEFINED: u16 = 0x200;
    let Some(c) = char::from_u32(u32::from(unit)) else {
        // A lone surrogate code unit.
        return DEFINED;
    };
    let mut t = DEFINED;
    if c.is_uppercase() {
        t |= UPPER;
    }
    if c.is_lowercase() {
        t |= LOWER;
    }
    if c.is_ascii_digit() {
        t |= DIGIT;
    }
    if c.is_whitespace() {
        t |= SPACE;
    }
    if c == ' ' || c == '\t' || c == '\u{a0}' || c == '\u{3000}' {
        t |= BLANK;
    }
    if c.is_control() {
        t |= CNTRL;
    }
    if c.is_ascii_hexdigit() {
        t |= XDIGIT;
    }
    if c.is_alphabetic() {
        t |= ALPHA;
    } else if !c.is_whitespace() && !c.is_control() && !c.is_numeric() {
        t |= PUNCT;
    }
    t
}
fn ctype2(unit: u16) -> u16 {
    // Bidi categories, coarsely: L, EN, WS, ON.
    match char::from_u32(u32::from(unit)) {
        Some(c) if c.is_ascii_digit() => 3,
        Some(c) if c.is_whitespace() => 10,
        Some(c) if c.is_alphabetic() => 1,
        Some(c) if c.is_control() => 0,
        _ => 11,
    }
}
fn ctype3(unit: u16) -> u16 {
    const SYMBOL: u16 = 0x8;
    const KATAKANA: u16 = 0x10;
    const HIRAGANA: u16 = 0x20;
    const HALFWIDTH: u16 = 0x40;
    const FULLWIDTH: u16 = 0x80;
    const IDEOGRAPH: u16 = 0x100;
    const HIGH: u16 = 0x800;
    const LOW: u16 = 0x1000;
    const ALPHA: u16 = 0x8000;
    match unit {
        0xd800..=0xdbff => return HIGH,
        0xdc00..=0xdfff => return LOW,
        _ => {}
    }
    let c = char::from_u32(u32::from(unit)).unwrap_or('\0');
    let mut t = 0;
    match unit {
        0x3041..=0x309f => t |= HIRAGANA | FULLWIDTH,
        0x30a0..=0x30ff => t |= KATAKANA | FULLWIDTH,
        0xff66..=0xff9f => t |= KATAKANA | HALFWIDTH,
        0x4e00..=0x9fff | 0x3400..=0x4dbf | 0xf900..=0xfaff => t |= IDEOGRAPH | FULLWIDTH,
        0xff01..=0xff5e => t |= FULLWIDTH,
        0x0020..=0x007e => t |= HALFWIDTH,
        _ => {}
    }
    if c.is_alphabetic() {
        t |= ALPHA;
    } else if !c.is_whitespace() && !c.is_control() && !c.is_alphanumeric() {
        t |= SYMBOL;
    }
    t
}
#[unsafe(no_mangle)]
pub extern "system" fn GetStringTypeW(kind: u32, text: usize, length: i32, types: usize) -> i32 {
    let classify: fn(u16) -> u16 = match kind {
        1 => ctype1,
        2 => ctype2,
        4 => ctype3,
        _ => {
            last_error(INVALID_FLAGS);
            return 0;
        }
    };
    let Some(text) = units(text, length).filter(|_| length != 0 && types != 0) else {
        last_error(INVALID_PARAMETER);
        return 0;
    };
    for (i, unit) in text.iter().enumerate() {
        unsafe { ((types as *mut u16).add(i)).write_unaligned(classify(*unit)) };
    }
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn GetStringTypeExW(
    _locale: u32,
    kind: u32,
    text: usize,
    length: i32,
    types: usize,
) -> i32 {
    GetStringTypeW(kind, text, length, types)
}

// ---- case mapping and sort keys ------------------------------------------------------

const LCMAP_LOWERCASE: u32 = 0x100;
const LCMAP_UPPERCASE: u32 = 0x200;
const LCMAP_TITLECASE: u32 = 0x300;
const LCMAP_SORTKEY: u32 = 0x400;
const LCMAP_BYTEREV: u32 = 0x800;
const LCMAP_LINGUISTIC_CASING: u32 = 0x0100_0000;
const NORM_IGNORECASE: u32 = 0x1;
const NORM_IGNORENONSPACE: u32 = 0x2;
const NORM_IGNORESYMBOLS: u32 = 0x4;
const NORM_IGNOREKANATYPE: u32 = 0x1_0000;
const NORM_IGNOREWIDTH: u32 = 0x2_0000;
const LINGUISTIC_IGNORECASE: u32 = 0x10;
const LINGUISTIC_IGNOREDIACRITIC: u32 = 0x20;
const SORT_STRINGSORT: u32 = 0x1000;
const SORT_DIGITSASNUMBERS: u32 = 0x8;
/// One BMP unit through a single-unit case mapping (Windows' simple case
/// mapping keeps the length).
pub(crate) fn map_case(unit: u16, upper: bool) -> u16 {
    let Some(c) = char::from_u32(u32::from(unit)) else {
        return unit;
    };
    let mut buffer = [' '; 3];
    let mut n = 0;
    if upper {
        for m in c.to_uppercase() {
            if n < 3 {
                buffer[n] = m;
            }
            n += 1;
        }
    } else {
        for m in c.to_lowercase() {
            if n < 3 {
                buffer[n] = m;
            }
            n += 1;
        }
    }
    let mut mapped = buffer[..n.min(3)].iter().copied();
    match (mapped.next(), mapped.next()) {
        (Some(m), None) if (m as u32) <= 0xffff => m as u32 as u16,
        _ => unit,
    }
}
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "system" fn LCMapStringEx(
    locale: usize,
    flags: u32,
    source: usize,
    length: i32,
    destination: usize,
    capacity: i32,
    _version: usize,
    _reserved: usize,
    _sort_handle: usize,
) -> i32 {
    if which(locale).is_none() {
        last_error(INVALID_PARAMETER);
        return 0;
    }
    let Some(text) = units(source, length).filter(|_| length != 0 && capacity >= 0) else {
        last_error(INVALID_PARAMETER);
        return 0;
    };
    let with_nul = length < 0;
    let ignore = NORM_IGNORECASE
        | NORM_IGNORENONSPACE
        | NORM_IGNORESYMBOLS
        | NORM_IGNOREKANATYPE
        | NORM_IGNOREWIDTH
        | LINGUISTIC_IGNORECASE
        | LINGUISTIC_IGNOREDIACRITIC
        | SORT_STRINGSORT
        | SORT_DIGITSASNUMBERS;
    if flags & LCMAP_SORTKEY != 0 {
        if flags & !(LCMAP_SORTKEY | ignore) != 0 {
            last_error(INVALID_FLAGS);
            return 0;
        }
        // Primary weights (case-folded units, big-endian), then the case
        // level unless case is ignored, then Windows' terminators.
        let fold = flags & (NORM_IGNORECASE | LINGUISTIC_IGNORECASE) != 0;
        let skip_symbols = flags & NORM_IGNORESYMBOLS != 0;
        let kept = |u: &&u16| !(skip_symbols && ctype1(**u) & 0x10 != 0) && **u != 0;
        let primary = text.iter().filter(kept).count() * 2;
        let case = if fold {
            0
        } else {
            text.iter().filter(kept).count()
        };
        let needed = primary + 1 + 1 + case + 1 + 1;
        if capacity == 0 {
            return needed as i32;
        }
        if (capacity as usize) < needed || destination == 0 {
            last_error(INSUFFICIENT_BUFFER);
            return 0;
        }
        let out = destination as *mut u8;
        let mut at = 0;
        let mut put = |byte: u8| unsafe {
            out.add(at).write(byte);
            at += 1;
        };
        for unit in text.iter().filter(kept) {
            let folded = map_case(*unit, true);
            put((folded >> 8) as u8);
            put(folded as u8);
        }
        put(1); // end of primary weights
        put(1); // no diacritic weights
        if !fold {
            for unit in text.iter().filter(kept) {
                put(if map_case(*unit, true) != *unit { 2 } else { 1 });
            }
        }
        put(1); // end of case weights
        put(0);
        return needed as i32;
    }
    let mapping = flags & !(LCMAP_LINGUISTIC_CASING | NORM_IGNORENONSPACE | NORM_IGNORESYMBOLS);
    if !matches!(
        mapping,
        LCMAP_LOWERCASE | LCMAP_UPPERCASE | LCMAP_TITLECASE | LCMAP_BYTEREV | 0
    ) {
        last_error(INVALID_FLAGS);
        return 0;
    }
    let needed = text.len() + usize::from(with_nul);
    if capacity == 0 {
        return needed as i32;
    }
    if (capacity as usize) < needed || destination == 0 {
        last_error(INSUFFICIENT_BUFFER);
        return 0;
    }
    let mut word_start = true;
    for (i, unit) in text.iter().enumerate() {
        let mapped = match mapping {
            LCMAP_LOWERCASE => map_case(*unit, false),
            LCMAP_UPPERCASE => map_case(*unit, true),
            LCMAP_TITLECASE => {
                let m = map_case(*unit, word_start);
                let m = if word_start {
                    m
                } else {
                    map_case(*unit, false)
                };
                word_start = ctype1(*unit) & 0x100 == 0;
                m
            }
            LCMAP_BYTEREV => unit.swap_bytes(),
            _ => *unit,
        };
        unsafe { ((destination as *mut u16).add(i)).write_unaligned(mapped) };
    }
    if with_nul {
        unsafe { ((destination as *mut u16).add(text.len())).write_unaligned(0) };
    }
    needed as i32
}
#[unsafe(no_mangle)]
pub extern "system" fn LCMapStringW(
    _locale: u32,
    flags: u32,
    source: usize,
    length: i32,
    destination: usize,
    capacity: i32,
) -> i32 {
    LCMapStringEx(0, flags, source, length, destination, capacity, 0, 0, 0)
}

// ---- locale data -----------------------------------------------------------------

const LOCALE_RETURN_NUMBER: u32 = 0x2000_0000;
const LOCALE_NOUSEROVERRIDE: u32 = 0x8000_0000;
const LOCALE_USE_CP_ACP: u32 = 0x4000_0000;
/// (LCTYPE, en-US value, invariant value) for string data.
const STRINGS: &[(u32, &str, &str)] = &[
    (0x0001, "0409", "007f"), // ILANGUAGE
    (
        0x0002,
        "English (United States)",
        "Invariant Language (Invariant Country)",
    ), // SLANGUAGE
    (0x0003, "ENU", "IVL"),   // SABBREVLANGNAME
    (0x0004, "English", "Invariant Language"), // SNATIVELANGNAME
    (0x0005, "1", "1"),       // ICOUNTRY
    (0x0006, "United States", "Invariant Country"), // SCOUNTRY
    (0x0007, "USA", "IVC"),   // SABBREVCTRYNAME
    (0x0008, "United States", "Invariant Country"), // SNATIVECTRYNAME
    (0x000c, ",", ","),       // SLIST
    (0x000d, "1", "0"),       // IMEASURE (US units)
    (0x000e, ".", "."),       // SDECIMAL
    (0x000f, ",", ","),       // STHOUSAND
    (0x0010, "3;0", "3;0"),   // SGROUPING
    (0x0011, "2", "2"),       // IDIGITS
    (0x0012, "1", "1"),       // ILZERO
    (0x0013, "0123456789", "0123456789"), // SNATIVEDIGITS
    (0x0014, "$", "\u{a4}"),  // SCURRENCY
    (0x0015, "USD", "XDR"),   // SINTLSYMBOL
    (0x0016, ".", "."),       // SMONDECIMALSEP
    (0x0017, ",", ","),       // SMONTHOUSANDSEP
    (0x0018, "3;0", "3;0"),   // SMONGROUPING
    (0x0019, "2", "2"),       // ICURRDIGITS
    (0x001b, "0", "0"),       // ICURRENCY
    (0x001c, "0", "0"),       // INEGCURR
    (0x001d, "/", "/"),       // SDATE
    (0x001e, ":", ":"),       // STIME
    (0x001f, "M/d/yyyy", "MM/dd/yyyy"), // SSHORTDATE
    (0x0020, "dddd, MMMM d, yyyy", "dddd, dd MMMM yyyy"), // SLONGDATE
    (0x0028, "AM", "AM"),     // S1159
    (0x0029, "PM", "PM"),     // S2359
    (0x0050, "", "+"),        // SPOSITIVESIGN
    (0x0051, "-", "-"),       // SNEGATIVESIGN
    (0x0059, "en", "iv"),     // SISO639LANGNAME
    (0x005a, "US", "IV"),     // SISO3166CTRYNAME
    (0x005c, "en-US", ""),    // SNAME
    (0x006d, "en", ""),       // SPARENT
    (0x0067, "eng", "ivl"),   // SISO639LANGNAME2
    (0x1001, "English", "Invariant Language"), // SENGLISHLANGUAGENAME
    (0x1002, "United States", "Invariant Country"), // SENGLISHCOUNTRYNAME
    (0x000b, "437", "437"),   // IDEFAULTCODEPAGE (OEM)
    (0x1003, "h:mm:ss tt", "HH:mm:ss"), // STIMEFORMAT
    (0x1004, "1252", "1252"), // IDEFAULTANSICODEPAGE
    (0x1007, "US Dollar", "International Monetary Fund"), // SENGCURRNAME
    (0x1008, "US Dollar", "International Monetary Fund"), // SNATIVECURRNAME
    (0x1009, "1", "1"),       // ICALENDARTYPE (Gregorian)
    (0x100c, "6", "0"),       // IFIRSTDAYOFWEEK (Sunday / Monday)
    (0x100d, "0", "0"),       // IFIRSTWEEKOFYEAR
    (0x1010, "1", "1"),       // INEGNUMBER
    (0x1011, "0", "0"),       // IDEFAULTMACCODEPAGE
    (0x1014, "0", "0"),       // IDIGITSUBSTITUTION
    (0x006a, "-\u{221e}", "-Infinity"), // SNEGINFINITY
    (0x0069, "\u{221e}", "Infinity"), // SPOSINFINITY
    (0x006b, "NaN", "NaN"),   // SNAN
    (
        0x0074,
        "English (United States)",
        "Invariant Language (Invariant Country)",
    ), // SENGLISHDISPLAYNAME
    (
        0x0073,
        "English (United States)",
        "Invariant Language (Invariant Country)",
    ), // SNATIVEDISPLAYNAME
    (
        0x0072,
        "English (United States)",
        "Invariant Language (Invariant Country)",
    ), // SLOCALIZEDDISPLAYNAME
    (0x0078, "%", "%"),       // SPERCENT
    (0x0077, "\u{2030}", "\u{2030}"), // SPERMILLE
    (0x0075, "0", "0"),       // INEGATIVEPERCENT
    (0x0076, "0", "0"),       // IPOSITIVEPERCENT
    // What .NET's CultureData asks in NLS mode, as Windows 11 answers.
    (0x0021, "0", "1"),                         // IDATE
    (0x0022, "0", "1"),                         // ILDATE
    (0x0023, "0", "1"),                         // ITIME
    (0x0024, "1", "1"),                         // ICENTURY
    (0x0025, "0", "1"),                         // ITLZERO
    (0x0026, "0", "1"),                         // IDAYLZERO
    (0x0027, "0", "1"),                         // IMONLZERO
    (0x005b, "244", "244"),                     // IGEOID
    (0x005d, "h:mm:ss", "h:mm:ss"),             // SDURATION
    (0x005e, "0409:00000409", "0409:00000409"), // SKEYBOARDSTOINSTALL
    (0x0068, "USA", "IVC"),                     // SISO3166CTRYNAME2
    (0x006c, "Latn;", "Latn;"),                 // SSCRIPTS
    (0x006e, "en-US", ""),                      // SCONSOLEFALLBACKNAME
    (0x0070, "0", "0"),                         // IREADINGLAYOUT
    (0x0071, "0", "0"),                         // INEUTRAL
    (0x0079, "h:mm tt", "HH:mm"),               // SSHORTTIME
    (0x007a, "ENG", "dflt"),                    // SOPENTYPELANGUAGETAG
    (0x007b, "en-US", ""),                      // SSORTLOCALE
    (0x007c, "dddd, MMMM d", "dddd, dd MMMM"),  // SRELATIVELONGDATE
    (0x007e, "a", "a"),                         // SSHORTESTAM
    (0x007f, "p", "p"),                         // SSHORTESTPM
    (0x1005, "0", "0"),                         // ITIMEMARKPOSN
    (0x1006, "MMMM yyyy", "yyyy MMMM"),         // SYEARMONTH
    (0x100b, "0", "0"),                         // IOPTIONALCALENDAR
    (0x1012, "037", "037"),                     // IDEFAULTEBCDICCODEPAGE
    (0x1013, "Default", "Default"),             // SSORTNAME
];
#[unsafe(no_mangle)]
pub extern "system" fn GetLocaleInfoEx(
    locale: usize,
    kind: u32,
    data: usize,
    capacity: i32,
) -> i32 {
    let Some(en_us) = which(locale) else {
        last_error(INVALID_PARAMETER);
        return 0;
    };
    // LOCALE_ALLOW_NEUTRAL_NAMES and LOCALE_RETURN_GENITIVE_NAMES change
    // nothing for these locales.
    let base = kind
        & !(LOCALE_RETURN_NUMBER
            | LOCALE_NOUSEROVERRIDE
            | LOCALE_USE_CP_ACP
            | 0x0800_0000
            | 0x1000_0000);
    let neutral = units(locale, -1).is_some_and(|n| matches!(n, [101 | 69, 110 | 78]));
    let text = if let Some(text) = neutral.then(|| neutral_value(base)).flatten() {
        text
    } else if let Some(text) = date_name(base) {
        text
    } else {
        let Some(&(_, us, invariant)) = STRINGS.iter().find(|(k, ..)| *k == base) else {
            last_error(INVALID_FLAGS);
            return 0;
        };
        if en_us { us } else { invariant }
    };
    if kind & LOCALE_RETURN_NUMBER != 0 {
        // The value as a DWORD (two UTF-16 units of the buffer).
        let radix = if base == 0x0001 { 16 } else { 10 };
        let Ok(value) = u32::from_str_radix(text, radix) else {
            last_error(INVALID_FLAGS);
            return 0;
        };
        if capacity == 0 {
            return 2;
        }
        if capacity < 2 || data == 0 {
            last_error(INSUFFICIENT_BUFFER);
            return 0;
        }
        unsafe { (data as *mut u32).write_unaligned(value) };
        return 2;
    }
    give_str(text, data, capacity)
}
#[unsafe(no_mangle)]
pub extern "system" fn GetLocaleInfoW(locale: u32, kind: u32, data: usize, capacity: i32) -> i32 {
    // LOCALE_USER_DEFAULT 0x400, LOCALE_SYSTEM_DEFAULT 0x800, en-US,
    // LOCALE_INVARIANT 0x7f.
    match locale {
        0x0400 | 0x0800 | EN_US | 0 => GetLocaleInfoEx(0, kind, data, capacity),
        INVARIANT => {
            let empty = [0u16; 1];
            GetLocaleInfoEx(empty.as_ptr() as usize, kind, data, capacity)
        }
        _ => {
            last_error(INVALID_PARAMETER);
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn IsValidLocaleName(name: usize) -> i32 {
    i32::from(which(name).is_some())
}
#[unsafe(no_mangle)]
pub extern "system" fn LocaleNameToLCID(name: usize, _flags: u32) -> u32 {
    match which(name) {
        Some(true) => EN_US,
        Some(false) => INVARIANT,
        None => {
            last_error(INVALID_PARAMETER);
            0
        }
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn LCIDToLocaleName(
    lcid: u32,
    buffer: usize,
    capacity: i32,
    _flags: u32,
) -> i32 {
    match lcid {
        EN_US | 0x0400 | 0x0800 => give_str("en-US", buffer, capacity),
        INVARIANT => give_str("", buffer, capacity),
        _ => {
            last_error(INVALID_PARAMETER);
            0
        }
    }
}
/// The neutral culture "en": its own name and display names, no country,
/// neutral (LOCALE_INEUTRAL), its parent the invariant culture; the rest is
/// en-US's, as Windows answers a neutral name.
fn neutral_value(kind: u32) -> Option<&'static str> {
    Some(match kind {
        0x005c => "en",
        0x0071 => "1",
        0x006d => "",
        0x0002 | 0x0072 | 0x0073 | 0x0074 => "English",
        0x005a | 0x0068 | 0x0006 | 0x0008 | 0x1002 => "",
        0x007b => "en",
        _ => return None,
    })
}
/// Day and month names (LOCALE_SDAYNAME1.., SABBREVDAYNAME1..,
/// SMONTHNAME1..13, SABBREVMONTHNAME1..13, SSHORTESTDAYNAME1..), the same
/// in en-US and the invariant culture.
fn date_name(kind: u32) -> Option<&'static str> {
    const DAYS: [&str; 7] = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
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
    let k = kind as usize;
    Some(match kind {
        0x2a..=0x30 => DAYS[k - 0x2a],
        0x31..=0x37 => &DAYS[k - 0x31][..3],
        0x38..=0x43 => MONTHS[k - 0x38],
        0x44..=0x4f => &MONTHS[k - 0x44][..3],
        0x100e | 0x100f => "",
        0x60..=0x66 => &DAYS[k - 0x60][..2],
        _ => return None,
    })
}
