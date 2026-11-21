//! OLE Automation dates (days since 1899-12-30, the fraction the time of
//! day; a negative date's fraction still counts forward from midnight) as
//! VarBstrFromDate writes and VarDateFromStr reads them for en-US, measured
//! on Windows 11.
use alloc::{string::String, vec::Vec};

use crate::hr;

/// The earliest and latest dates oleaut32 accepts (1/1/100, 12/31/9999).
pub const MIN: f64 = -657_434.0;
pub const MAX: f64 = 2_958_465.999_999_99;

/// Days from 1970-01-01 of a proleptic Gregorian date (Howard Hinnant).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (i64::from(m) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
/// 1899-12-30 as days from 1970-01-01.
const EPOCH: i64 = -25_569;
pub fn days(y: i64, m: u32, d: u32) -> i64 {
    days_from_civil(y, m, d) - EPOCH
}
fn leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
pub fn month_days(y: i64, m: u32) -> u32 {
    match m {
        2 if leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// A date split into its parts (seconds rounded half up, as Windows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parts {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub day_of_week: u32,
    /// The whole-day part was zero (the time alone).
    pub no_date: bool,
}
pub fn split(v: f64) -> Result<Parts, u32> {
    if !(MIN..=MAX).contains(&v) || v.is_nan() {
        return Err(hr::E_INVALIDARG);
    }
    let mut whole = crate::float::trunc(v) as i64;
    let mut seconds = (crate::float::fract(v).abs() * 86_400.0 + 0.5) as i64;
    if seconds >= 86_400 {
        seconds -= 86_400;
        whole += 1;
    }
    let (year, month, day) = civil_from_days(whole + EPOCH);
    Ok(Parts {
        year,
        month,
        day,
        hour: (seconds / 3600) as u32,
        minute: (seconds / 60 % 60) as u32,
        second: (seconds % 60) as u32,
        day_of_week: (whole + EPOCH + 4).rem_euclid(7) as u32,
        no_date: whole == 0,
    })
}
/// VarBstrFromDate for en-US: M/D/YYYY, then h:mm:ss AM/PM; a midnight
/// time is left out, and so is the date of a time alone (day zero).
pub fn format(v: f64) -> Result<String, u32> {
    let p = split(v)?;
    let time = p.hour != 0 || p.minute != 0 || p.second != 0;
    let mut out = String::new();
    if !p.no_date {
        out.push_str(&alloc::format!("{}/{}/{}", p.month, p.day, p.year));
    }
    if time || p.no_date {
        if !out.is_empty() {
            out.push(' ');
        }
        let h12 = match p.hour % 12 {
            0 => 12,
            h => h,
        };
        out.push_str(&alloc::format!(
            "{}:{:02}:{:02} {}",
            h12,
            p.minute,
            p.second,
            if p.hour < 12 { "AM" } else { "PM" }
        ));
    }
    Ok(out)
}
/// A date from its parts (no range check of the time).
pub fn from_parts(y: i64, m: u32, d: u32, h: u32, min: u32, s: u32) -> f64 {
    let day = days(y, m, d) as f64;
    let time = f64::from(h * 3600 + min * 60 + s) / 86_400.0;
    if day < 0.0 { day - time } else { day + time }
}

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];
const WEEKDAYS: [&str; 7] = [
    "sunday",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
];
#[derive(Debug)]
enum Token {
    Number(u32, usize),
    Month(u32),
    Weekday,
    Am,
    Pm,
    Colon,
    Slash,
    Dash,
    Comma,
}
fn tokens(text: &str) -> Option<Vec<Token>> {
    let mut out = Vec::new();
    let b: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() {
            let start = i;
            let mut v: u32 = 0;
            while i < b.len() && b[i].is_ascii_digit() {
                v = v.checked_mul(10)?.checked_add(b[i] as u32 - '0' as u32)?;
                i += 1;
            }
            out.push(Token::Number(v, i - start));
        } else if c.is_alphabetic() {
            let start = i;
            while i < b.len() && b[i].is_alphabetic() {
                i += 1;
            }
            let word: String = b[start..i].iter().collect::<String>().to_lowercase();
            if word == "am" || word == "a" {
                out.push(Token::Am);
            } else if word == "pm" || word == "p" {
                out.push(Token::Pm);
            } else if let Some(m) = MONTHS
                .iter()
                .position(|m| word.len() >= 3 && m.starts_with(&word))
            {
                out.push(Token::Month(m as u32 + 1));
            } else if WEEKDAYS
                .iter()
                .any(|d| word.len() >= 3 && d.starts_with(&word))
            {
                out.push(Token::Weekday);
            } else {
                return None;
            }
        } else {
            out.push(match c {
                ':' => Token::Colon,
                '/' => Token::Slash,
                '-' => Token::Dash,
                ',' => Token::Comma,
                _ => return None,
            });
            i += 1;
        }
    }
    Some(out)
}
fn year_of(v: u32, digits: usize) -> i64 {
    if digits <= 2 {
        // Windows' 2029 cutoff.
        if v < 30 {
            2000 + i64::from(v)
        } else {
            1900 + i64::from(v)
        }
    } else {
        i64::from(v)
    }
}
/// VarDateFromStr for en-US: a date (M/D/Y, Y/M/D when the year comes
/// first, or with a month name; - or / between numbers), a time
/// (h:mm[:ss] [AM|PM]), or both; a weekday name is ignored.
/// DISP_E_TYPEMISMATCH otherwise.
pub fn parse(text: &str) -> Result<f64, u32> {
    let mismatch = hr::DISP_E_TYPEMISMATCH;
    let t = tokens(text).ok_or(mismatch)?;
    let mut date_numbers: Vec<(u32, usize)> = Vec::new();
    let mut month_name: Option<u32> = None;
    let mut time: Vec<u32> = Vec::new();
    let mut meridiem: Option<bool> = None;
    let mut i = 0;
    while i < t.len() {
        match t[i] {
            Token::Number(v, n) => {
                if matches!(t.get(i + 1), Some(Token::Colon)) {
                    // h:mm[:ss]
                    if !time.is_empty() {
                        return Err(mismatch);
                    }
                    time.push(v);
                    i += 1;
                    while let (Some(Token::Colon), Some(Token::Number(x, _))) =
                        (t.get(i), t.get(i + 1))
                    {
                        time.push(*x);
                        i += 2;
                    }
                    if time.len() > 3 {
                        return Err(mismatch);
                    }
                    continue;
                }
                date_numbers.push((v, n));
            }
            Token::Month(m) => {
                if month_name.is_some() {
                    return Err(mismatch);
                }
                month_name = Some(m);
            }
            Token::Am | Token::Pm => {
                if time.is_empty() || meridiem.is_some() {
                    return Err(mismatch);
                }
                meridiem = Some(matches!(t[i], Token::Pm));
            }
            // Windows refuses a weekday name ("Thursday, January 02, 2003").
            Token::Weekday => return Err(mismatch),
            Token::Slash | Token::Dash | Token::Comma => (),
            Token::Colon => return Err(mismatch),
        }
        i += 1;
    }
    if date_numbers.is_empty() && month_name.is_none() && time.is_empty() {
        return Err(mismatch);
    }
    // The date.
    let date = match (month_name, date_numbers.as_slice()) {
        (None, []) => None,
        (Some(m), [(d, _), (y, n)]) => Some((year_of(*y, *n), m, *d)),
        (None, [(a, an), (b, _), (c, cn)]) => {
            if *an >= 3 || *a > 31 {
                Some((year_of(*a, *an), *b, *c))
            } else {
                Some((year_of(*c, *cn), *a, *b))
            }
        }
        _ => return Err(mismatch),
    };
    let mut day = 0.0;
    if let Some((y, m, d)) = date {
        if !(1..=12).contains(&m) || d == 0 || d > month_days(y, m) || !(100..=9999).contains(&y) {
            return Err(mismatch);
        }
        day = days(y, m, d) as f64;
    }
    let mut seconds = 0u32;
    if !time.is_empty() {
        let (mut h, min, s) = (
            time[0],
            time.get(1).copied().unwrap_or(0),
            time.get(2).copied().unwrap_or(0),
        );
        if min > 59 || s > 59 {
            return Err(mismatch);
        }
        match meridiem {
            Some(pm) => {
                if h == 0 || h > 12 {
                    return Err(mismatch);
                }
                h %= 12;
                if pm {
                    h += 12;
                }
            }
            None if h > 23 => return Err(mismatch),
            None => (),
        }
        seconds = h * 3600 + min * 60 + s;
    }
    let fraction = f64::from(seconds) / 86_400.0;
    Ok(if day < 0.0 {
        day - fraction
    } else {
        day + fraction
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bits(v: f64) -> String {
        v.to_le_bytes()
            .iter()
            .map(|b| alloc::format!("{b:02x}"))
            .collect()
    }
    #[test]
    fn formatting_matches_windows() {
        for (raw, text) in [
            ("e90b21e7fdffef3f", "12/31/1899"),
            ("479ca1faffffef3f", "12/31/1899"),
            ("1deed9290c00f03f", "12/31/1899 12:00:01 AM"),
            ("00000000d0d5e140", "1/1/2000 12:00:00 PM"),
            ("000000000000d0bf", "6:00:00 AM"),
            ("0000008040924641", "12/31/9999"),
            ("dafe3adcb353d83e", "12:00:01 AM"),
            ("50caffffff5ee240", "1/3/2003"),
            ("26bf5872e55ee240", "1/2/2003 4:05:06 AM"),
            ("0000000000000000", "12:00:00 AM"),
            ("000000000000e03f", "12:00:00 PM"),
        ] {
            let mut b = [0u8; 8];
            for (i, out) in b.iter_mut().enumerate() {
                *out = u8::from_str_radix(&raw[2 * i..2 * i + 2], 16).unwrap();
            }
            assert_eq!(format(f64::from_le_bytes(b)).unwrap(), text, "{raw}");
        }
        assert_eq!(format(-1.25).unwrap(), "12/29/1899 6:00:00 AM");
        assert_eq!(format(1.75).unwrap(), "12/31/1899 6:00:00 PM");
        assert_eq!(format(2_958_465.999_99).unwrap(), "12/31/9999 11:59:59 PM");
        assert_eq!(format(-657_434.0).unwrap(), "1/1/100");
        assert_eq!(format(2.5).unwrap(), "1/1/1900 12:00:00 PM");
    }
    #[test]
    fn parsing_matches_windows() {
        for (text, raw) in [
            ("1/2/2003", Some("00000000e05ee240")),
            ("Jan 2, 2003", Some("00000000e05ee240")),
            ("2 January 2003", Some("00000000e05ee240")),
            ("4:05 PM", Some("1cc7711cc771e53f")),
            ("13/13/2003", None),
            ("2003/1/2 16:05", Some("721cc771f55ee240")),
            ("Thursday, January 02, 2003", None),
            ("12/31/1899", Some("000000000000f03f")),
            ("0:00", Some("0000000000000000")),
            ("1/2/03", Some("00000000e05ee240")),
            ("1.2.2003", None),
            ("Feb 29, 2001", None),
            ("1/2/2003 4:05:06 AM", Some("26bf5872e55ee240")),
            ("2003-01-02", Some("00000000e05ee240")),
            ("12:30", Some("abaaaaaaaaaae03f")),
            ("123", None),
        ] {
            assert_eq!(parse(text).ok().map(bits).as_deref(), raw, "{text}");
        }
    }
}
