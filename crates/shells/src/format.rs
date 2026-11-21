//! Number, date and time text as the en-US Windows console prints it.
use crate::host::LocalTime;
use alloc::{format, string::String};

/// `33532461056` -> `33,532,461,056`.
pub fn grouped(n: u64) -> String {
    let digits = format!("{n}");
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
pub fn grouped_i(n: i64) -> String {
    if n < 0 {
        format!("-{}", grouped(n.unsigned_abs()))
    } else {
        grouped(n as u64)
    }
}
/// `11/02/2026`.
pub fn date(t: &LocalTime) -> String {
    format!("{:02}/{:02}/{:04}", t.month, t.day, t.year)
}
pub const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
pub const WEEKDAYS_LONG: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
pub const MONTHS: [&str; 12] = [
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
/// `%DATE%` / `date /t`: `Mon 11/02/2026`.
pub fn date_long(t: &LocalTime) -> String {
    format!("{} {}", WEEKDAYS[t.weekday as usize % 7], date(t))
}
/// `12:16 PM` (`dir`, `time /t`).
pub fn time_short(t: &LocalTime) -> String {
    let (h, pm) = match t.hour {
        0 => (12, false),
        1..=11 => (t.hour, false),
        12 => (12, true),
        h => (h - 12, true),
    };
    format!("{h:02}:{:02} {}", t.minute, if pm { "PM" } else { "AM" })
}
/// `%TIME%`: ` 9:05:01.23` (24-hour, space-padded hour).
pub fn time_long(t: &LocalTime) -> String {
    format!(
        "{:>2}:{:02}:{:02}.{:02}",
        t.hour, t.minute, t.second, t.centis
    )
}
/// `h:mm:ss AM` as systeminfo and ipconfig print times.
pub fn time_seconds(t: &LocalTime) -> String {
    let (h, pm) = match t.hour {
        0 => (12, false),
        1..=11 => (t.hour, false),
        12 => (12, true),
        h => (h - 12, true),
    };
    format!(
        "{h}:{:02}:{:02} {}",
        t.minute,
        t.second,
        if pm { "PM" } else { "AM" }
    )
}
/// `Monday, November 2, 2026 12:16:45 PM` (ipconfig lease times).
pub fn date_time_full(t: &LocalTime) -> String {
    format!(
        "{}, {} {}, {} {}",
        WEEKDAYS_LONG[t.weekday as usize % 7],
        MONTHS[(t.month.max(1) - 1) as usize % 12],
        t.day,
        t.year,
        time_seconds(t)
    )
}
/// Local civil time of a FILETIME shifted by `offset_minutes`.
pub fn civil(filetime: u64, offset_minutes: i64) -> LocalTime {
    let total = (filetime / 10_000) as i64 + offset_minutes * 60_000;
    let ms = total.rem_euclid(86_400_000);
    let days = total.div_euclid(86_400_000);
    // Days since 1601-01-01 (a Monday) -> civil date.
    let z = days - 134_774 + 719_468; // to days since 0000-03-01 via 1970 epoch
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    LocalTime {
        year: y as u32,
        month: m as u32,
        day: d as u32,
        hour: (ms / 3_600_000) as u32,
        minute: (ms / 60_000 % 60) as u32,
        second: (ms / 1000 % 60) as u32,
        centis: (ms % 1000 / 10) as u32,
        weekday: ((days + 1).rem_euclid(7)) as u32,
    }
}
/// FILETIME of a civil UTC time.
pub fn filetime(t: &LocalTime) -> u64 {
    let (y, m) = if t.month <= 2 {
        (t.year as i64 - 1, t.month as i64 + 9)
    } else {
        (t.year as i64, t.month as i64 - 3)
    };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + t.day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days_1970 = era * 146_097 + doe - 719_468;
    let days = days_1970 + 134_774;
    let ms = days * 86_400_000
        + (t.hour as i64 * 3600 + t.minute as i64 * 60 + t.second as i64) * 1000
        + t.centis as i64 * 10;
    (ms.max(0) as u64) * 10_000
}
/// `1.2 MB`-style size for Doorsh listings.
pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else if v < 10.0 {
        format!("{:.1} {}", v, UNITS[u])
    } else {
        format!("{} {}", v as u64, UNITS[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbers_and_times() {
        assert_eq!(grouped(33532461056), "33,532,461,056");
        assert_eq!(grouped(39), "39");
        assert_eq!(grouped(1000), "1,000");
        // 2026-11-02 12:16:00 UTC.
        let t = LocalTime {
            year: 2026,
            month: 11,
            day: 2,
            hour: 12,
            minute: 16,
            second: 0,
            centis: 0,
            weekday: 1,
        };
        let ft = filetime(&t);
        assert_eq!(civil(ft, 0), t);
        assert_eq!(date_long(&t), "Mon 11/02/2026");
        assert_eq!(time_short(&t), "12:16 PM");
        let morning = LocalTime {
            hour: 9,
            minute: 5,
            second: 1,
            centis: 23,
            ..t
        };
        assert_eq!(time_long(&morning), " 9:05:01.23");
        assert_eq!(time_short(&LocalTime { hour: 0, ..t }), "12:16 AM");
        // 1601-01-01 is a Monday.
        assert_eq!(civil(0, 0).weekday, 1);
        assert_eq!(civil(0, 0).year, 1601);
        assert_eq!(human(1536), "1.5 KB");
    }
}
