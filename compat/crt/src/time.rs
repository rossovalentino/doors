//! Calendar time for `_gmtime64_s`, `_mktime64`, `_localtime64_s` (with
//! the zone's bias), `strftime` and `wcsftime` in the "C" locale.

/// `struct tm`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct Tm {
    pub sec: i32,
    pub min: i32,
    pub hour: i32,
    pub mday: i32,
    pub mon: i32,
    pub year: i32,
    pub wday: i32,
    pub yday: i32,
    pub isdst: i32,
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
pub const fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
/// The civil date of a day count since 1970-01-01: (year, month, day).
pub const fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `_gmtime64`: None outside the CRT's range (1970 to 3000-12-31 23:59:59).
pub fn gmtime(t: i64) -> Option<Tm> {
    if !(0..=32_535_215_999).contains(&t) {
        return None;
    }
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let yday = days - days_from_civil(y, 1, 1);
    Some(Tm {
        sec: (secs % 60) as i32,
        min: (secs / 60 % 60) as i32,
        hour: (secs / 3600) as i32,
        mday: d as i32,
        mon: m as i32 - 1,
        year: (y - 1900) as i32,
        wday: (days + 4).rem_euclid(7) as i32,
        yday: yday as i32,
        isdst: 0,
    })
}
/// `_mkgmtime64` (and `_mktime64` after the zone's bias): fields may be out
/// of range and are normalized; None outside the CRT's range.
pub fn timegm(tm: &Tm) -> Option<i64> {
    let mut year = i64::from(tm.year) + 1900;
    let mut mon = i64::from(tm.mon);
    year += mon.div_euclid(12);
    mon = mon.rem_euclid(12);
    let days = days_from_civil(year, mon as u32 + 1, 1) + i64::from(tm.mday) - 1;
    let t = days * 86_400 + i64::from(tm.hour) * 3600 + i64::from(tm.min) * 60 + i64::from(tm.sec);
    (0..=32_535_215_999).contains(&t).then_some(t)
}

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

/// strftime in the "C" locale: units into `out` (`push` returns false when
/// full); the count written without the terminator, or None when it did
/// not fit (strftime then returns 0). `format` is in units.
pub fn strftime(format: &[u16], tm: &Tm, push: &mut dyn FnMut(u16) -> bool) -> Option<usize> {
    let mut n = 0usize;
    fn put(push: &mut dyn FnMut(u16) -> bool, s: &str, n: &mut usize) -> bool {
        for b in s.bytes() {
            if !push(u16::from(b)) {
                return false;
            }
            *n += 1;
        }
        true
    }
    let mut i = 0;
    let mut buf = [0u8; 64];
    while i < format.len() && format[i] != 0 {
        let c = format[i];
        if c != u16::from(b'%') {
            if !push(c) {
                return None;
            }
            n += 1;
            i += 1;
            continue;
        }
        i += 1;
        // "#" (no leading zeros) is accepted and ignored for the "C" locale.
        if format.get(i) == Some(&u16::from(b'#')) {
            i += 1;
        }
        let spec = format.get(i).copied().unwrap_or(0);
        i += 1;
        let text = render(spec as u8, tm, &mut buf);
        if !put(push, text, &mut n) {
            return None;
        }
    }
    Some(n)
}
fn render<'a>(spec: u8, tm: &Tm, buf: &'a mut [u8; 64]) -> &'a str {
    use core::fmt::Write;
    struct W<'b>(&'b mut [u8; 64], usize);
    impl core::fmt::Write for W<'_> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let b = s.as_bytes();
            let end = (self.1 + b.len()).min(64);
            self.0[self.1..end].copy_from_slice(&b[..end - self.1]);
            self.1 = end;
            Ok(())
        }
    }
    let mut w = W(buf, 0);
    let wday = tm.wday.rem_euclid(7) as usize;
    let mon = tm.mon.rem_euclid(12) as usize;
    let year = i64::from(tm.year) + 1900;
    let hour12 = if tm.hour % 12 == 0 { 12 } else { tm.hour % 12 };
    let _ = match spec {
        b'a' => w.write_str(&DAYS[wday][..3]),
        b'A' => w.write_str(DAYS[wday]),
        b'b' | b'h' => w.write_str(&MONTHS[mon][..3]),
        b'B' => w.write_str(MONTHS[mon]),
        b'c' => write!(
            w,
            "{:02}/{:02}/{:02} {:02}:{:02}:{:02}",
            mon + 1,
            tm.mday,
            year.rem_euclid(100),
            tm.hour,
            tm.min,
            tm.sec
        ),
        b'C' => write!(w, "{:02}", year / 100),
        b'd' => write!(w, "{:02}", tm.mday),
        b'D' | b'x' => write!(
            w,
            "{:02}/{:02}/{:02}",
            mon + 1,
            tm.mday,
            year.rem_euclid(100)
        ),
        b'e' => write!(w, "{:2}", tm.mday),
        b'F' => write!(w, "{:04}-{:02}-{:02}", year, mon + 1, tm.mday),
        b'H' => write!(w, "{:02}", tm.hour),
        b'I' => write!(w, "{:02}", hour12),
        b'j' => write!(w, "{:03}", tm.yday + 1),
        b'm' => write!(w, "{:02}", mon + 1),
        b'M' => write!(w, "{:02}", tm.min),
        b'n' => w.write_str("\n"),
        b'p' => w.write_str(if tm.hour < 12 { "AM" } else { "PM" }),
        b'r' => write!(
            w,
            "{:02}:{:02}:{:02} {}",
            hour12,
            tm.min,
            tm.sec,
            if tm.hour < 12 { "AM" } else { "PM" }
        ),
        b'R' => write!(w, "{:02}:{:02}", tm.hour, tm.min),
        b'S' => write!(w, "{:02}", tm.sec),
        b't' => w.write_str("\t"),
        b'T' | b'X' => write!(w, "{:02}:{:02}:{:02}", tm.hour, tm.min, tm.sec),
        b'u' => write!(w, "{}", if wday == 0 { 7 } else { wday }),
        b'w' => write!(w, "{}", wday),
        b'y' => write!(w, "{:02}", year.rem_euclid(100)),
        b'Y' => write!(w, "{}", year),
        b'z' => w.write_str("+0000"),
        b'Z' => w.write_str("UTC"),
        b'%' => w.write_str("%"),
        b'U' => write!(w, "{:02}", (tm.yday + 7 - tm.wday) / 7),
        b'W' => write!(w, "{:02}", (tm.yday + 7 - (tm.wday + 6) % 7) / 7),
        b'G' | b'g' | b'V' => {
            let (iso_year, week) = iso_week(year, tm.yday, wday as i32);
            match spec {
                b'G' => write!(w, "{}", iso_year),
                b'g' => write!(w, "{:02}", iso_year.rem_euclid(100)),
                _ => write!(w, "{:02}", week),
            }
        }
        _ => Ok(()),
    };
    let len = w.1;
    core::str::from_utf8(&buf[..len]).unwrap_or("")
}
/// The ISO 8601 week-numbering year and week of a date.
fn iso_week(year: i64, yday: i32, wday: i32) -> (i64, i32) {
    let weeks = |y: i64| {
        let p =
            |y: i64| (y + y.div_euclid(4) - y.div_euclid(100) + y.div_euclid(400)).rem_euclid(7);
        if p(y) == 4 || p(y - 1) == 3 { 53 } else { 52 }
    };
    let iso_wday = if wday == 0 { 7 } else { wday };
    let week = (yday + 1 - iso_wday + 10).div_euclid(7);
    if week < 1 {
        return (year - 1, weeks(year - 1));
    }
    if week > weeks(year) {
        return (year + 1, 1);
    }
    (year, week)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::{string::String, vec::Vec};
    #[test]
    fn dates_round_trip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(19_000), (2022, 1, 8));
        let tm = gmtime(1_700_000_000).unwrap();
        assert_eq!(
            (
                tm.year, tm.mon, tm.mday, tm.hour, tm.min, tm.sec, tm.wday, tm.yday
            ),
            (123, 10, 14, 22, 13, 20, 2, 317)
        );
        assert_eq!(timegm(&tm), Some(1_700_000_000));
        let mut odd = tm;
        odd.mon += 14;
        odd.mday -= 40;
        assert_eq!(gmtime(timegm(&odd).unwrap()).unwrap().year, 124);
        assert!(gmtime(-1).is_none());
    }
    #[test]
    fn strftime_in_the_c_locale() {
        let tm = gmtime(1_700_000_000).unwrap();
        let f: Vec<u16> = "%a %b %d %Y %H:%M:%S %j %p %c %%".encode_utf16().collect();
        let mut out = Vec::new();
        let n = strftime(&f, &tm, &mut |u| {
            out.push(u);
            true
        })
        .unwrap();
        let s: String = String::from_utf16(&out).unwrap();
        assert_eq!(s, "Tue Nov 14 2023 22:13:20 318 PM 11/14/23 22:13:20 %");
        assert_eq!(n, s.len());
        let mut tiny = 0;
        assert_eq!(
            strftime(&f, &tm, &mut |_| {
                tiny += 1;
                tiny < 5
            }),
            None
        );
    }
}
