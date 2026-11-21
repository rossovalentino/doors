//! Regional formats (Settings > Time & language > Language & region >
//! Regional format): the Windows 11 default patterns for each locale.
//!
//! Patterns use the Windows/.NET custom format letters: d dd dddd, M MM
//! MMMM, yy yyyy, h hh H HH, m mm, s ss, t tt and 'quoted' literals (ddd and
//! MMM print the full name; no abbreviations are carried). Settings store the
//! stable `Region::id`.
use core::fmt::{self, Write};
use door_abi::clock::WallClock;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Region {
    /// Stable persisted identifier (`Setting::RegionalFormat`).
    pub id: u16,
    pub tag: &'static str,
    /// Name in the Regional format list (Windows display language: English).
    pub name: &'static str,
    pub short_date: &'static str,
    pub long_date: &'static str,
    pub short_time: &'static str,
    pub long_time: &'static str,
    /// Calendar flyout heading (weekday, day and month).
    pub month_day: &'static str,
    /// Calendar month heading ("October 2026").
    pub year_month: &'static str,
    /// Numeric month and day (narrow taskbar clock).
    pub numeric_month_day: &'static str,
    /// 0 = Sunday.
    pub first_day: u8,
    pub decimal: &'static str,
    pub group: &'static str,
    pub am: &'static str,
    pub pm: &'static str,
    pub months: [&'static str; 12],
    /// Sunday first.
    pub days: [&'static str; 7],
    /// Calendar grid column headings, Sunday first.
    pub short_days: [&'static str; 7],
}

const EN_MONTHS: [&str; 12] = [
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
const EN_DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const EN_SHORT: [&str; 7] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
const CJK_MONTHS: [&str; 12] = [
    "1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月",
];

/// Display order of the Regional format list.
pub static REGIONS: [Region; 7] = [
    Region {
        id: 1,
        tag: "en-US",
        name: "English (United States)",
        short_date: "M/d/yyyy",
        long_date: "dddd, MMMM d, yyyy",
        short_time: "h:mm tt",
        long_time: "h:mm:ss tt",
        month_day: "dddd, MMMM d",
        year_month: "MMMM yyyy",
        numeric_month_day: "M/d",
        first_day: 0,
        decimal: ".",
        group: ",",
        am: "AM",
        pm: "PM",
        months: EN_MONTHS,
        days: EN_DAYS,
        short_days: EN_SHORT,
    },
    Region {
        id: 2,
        tag: "en-GB",
        name: "English (United Kingdom)",
        short_date: "dd/MM/yyyy",
        long_date: "dd MMMM yyyy",
        short_time: "HH:mm",
        long_time: "HH:mm:ss",
        month_day: "dddd d MMMM",
        year_month: "MMMM yyyy",
        numeric_month_day: "dd/MM",
        first_day: 1,
        decimal: ".",
        group: ",",
        am: "am",
        pm: "pm",
        months: EN_MONTHS,
        days: EN_DAYS,
        short_days: EN_SHORT,
    },
    Region {
        id: 3,
        tag: "de-DE",
        name: "German (Germany)",
        short_date: "dd.MM.yyyy",
        long_date: "dddd, d. MMMM yyyy",
        short_time: "HH:mm",
        long_time: "HH:mm:ss",
        month_day: "dddd, d. MMMM",
        year_month: "MMMM yyyy",
        numeric_month_day: "dd.MM.",
        first_day: 1,
        decimal: ",",
        group: ".",
        am: "",
        pm: "",
        months: [
            "Januar",
            "Februar",
            "März",
            "April",
            "Mai",
            "Juni",
            "Juli",
            "August",
            "September",
            "Oktober",
            "November",
            "Dezember",
        ],
        days: [
            "Sonntag",
            "Montag",
            "Dienstag",
            "Mittwoch",
            "Donnerstag",
            "Freitag",
            "Samstag",
        ],
        short_days: ["So", "Mo", "Di", "Mi", "Do", "Fr", "Sa"],
    },
    Region {
        id: 4,
        tag: "fr-FR",
        name: "French (France)",
        short_date: "dd/MM/yyyy",
        long_date: "dddd d MMMM yyyy",
        short_time: "HH:mm",
        long_time: "HH:mm:ss",
        month_day: "dddd d MMMM",
        year_month: "MMMM yyyy",
        numeric_month_day: "dd/MM",
        first_day: 1,
        decimal: ",",
        // Windows 10 1809+ uses NARROW NO-BREAK SPACE for fr-FR grouping.
        group: "\u{202f}",
        am: "",
        pm: "",
        months: [
            "janvier",
            "février",
            "mars",
            "avril",
            "mai",
            "juin",
            "juillet",
            "août",
            "septembre",
            "octobre",
            "novembre",
            "décembre",
        ],
        days: [
            "dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi",
        ],
        short_days: ["di", "lu", "ma", "me", "je", "ve", "sa"],
    },
    Region {
        id: 5,
        tag: "zh-CN",
        name: "Chinese (Simplified, China)",
        short_date: "yyyy/M/d",
        long_date: "yyyy'年'M'月'd'日'",
        short_time: "H:mm",
        long_time: "H:mm:ss",
        month_day: "M'月'd'日' dddd",
        year_month: "yyyy'年'M'月'",
        numeric_month_day: "M/d",
        first_day: 1,
        decimal: ".",
        group: ",",
        am: "上午",
        pm: "下午",
        months: [
            "一月",
            "二月",
            "三月",
            "四月",
            "五月",
            "六月",
            "七月",
            "八月",
            "九月",
            "十月",
            "十一月",
            "十二月",
        ],
        days: [
            "星期日",
            "星期一",
            "星期二",
            "星期三",
            "星期四",
            "星期五",
            "星期六",
        ],
        short_days: ["日", "一", "二", "三", "四", "五", "六"],
    },
    Region {
        id: 6,
        tag: "ja-JP",
        name: "Japanese (Japan)",
        short_date: "yyyy/MM/dd",
        long_date: "yyyy'年'M'月'd'日'",
        short_time: "H:mm",
        long_time: "H:mm:ss",
        month_day: "M'月'd'日' dddd",
        year_month: "yyyy'年'M'月'",
        numeric_month_day: "MM/dd",
        first_day: 0,
        decimal: ".",
        group: ",",
        am: "午前",
        pm: "午後",
        months: CJK_MONTHS,
        days: [
            "日曜日",
            "月曜日",
            "火曜日",
            "水曜日",
            "木曜日",
            "金曜日",
            "土曜日",
        ],
        short_days: ["日", "月", "火", "水", "木", "金", "土"],
    },
    Region {
        id: 7,
        tag: "ko-KR",
        name: "Korean (Korea)",
        short_date: "yyyy-MM-dd",
        long_date: "yyyy'년' M'월' d'일' dddd",
        short_time: "tt h:mm",
        long_time: "tt h:mm:ss",
        month_day: "M'월' d'일' dddd",
        year_month: "yyyy'년' M'월'",
        numeric_month_day: "MM-dd",
        first_day: 0,
        decimal: ".",
        group: ",",
        am: "오전",
        pm: "오후",
        months: [
            "1월", "2월", "3월", "4월", "5월", "6월", "7월", "8월", "9월", "10월", "11월", "12월",
        ],
        days: [
            "일요일",
            "월요일",
            "화요일",
            "수요일",
            "목요일",
            "금요일",
            "토요일",
        ],
        short_days: ["일", "월", "화", "수", "목", "금", "토"],
    },
];
/// `Setting::RegionalFormat` default: English (United States).
pub const DEFAULT_ID: u16 = 1;
pub fn by_id(id: u32) -> Option<&'static Region> {
    REGIONS.iter().find(|r| u32::from(r.id) == id)
}
pub fn index_of(id: u32) -> Option<usize> {
    REGIONS.iter().position(|r| u32::from(r.id) == id)
}
pub fn valid(id: u32) -> bool {
    by_id(id).is_some()
}
pub fn region_or_default(id: u32) -> &'static Region {
    by_id(id).unwrap_or(&REGIONS[0])
}

/// A fixed-capacity UTF-8 buffer for allocation-free formatting.
#[derive(Clone, Copy)]
pub struct Text<const N: usize> {
    bytes: [u8; N],
    len: usize,
}
impl<const N: usize> Text<N> {
    pub const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}
impl<const N: usize> Default for Text<N> {
    fn default() -> Self {
        Self::new()
    }
}
impl<const N: usize> Write for Text<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self
            .len
            .checked_add(s.len())
            .filter(|e| *e <= N)
            .ok_or(fmt::Error)?;
        self.bytes[self.len..end].copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

impl Region {
    /// Apply a Windows custom date/time pattern to a local civil time.
    pub fn format(&self, pattern: &str, t: &WallClock, out: &mut dyn Write) -> fmt::Result {
        let mut chars = pattern.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\'' {
                for literal in chars.by_ref() {
                    if literal == '\'' {
                        break;
                    }
                    out.write_char(literal)?;
                }
                continue;
            }
            if !"dMyhHmst".contains(c) {
                out.write_char(c)?;
                continue;
            }
            let mut run = 1;
            while chars.peek() == Some(&c) {
                chars.next();
                run += 1;
            }
            let hour12 = match t.hour % 12 {
                0 => 12,
                h => h,
            };
            let designator = if t.hour < 12 { self.am } else { self.pm };
            match (c, run) {
                ('d', 1) => write!(out, "{}", t.day)?,
                ('d', 2) => write!(out, "{:02}", t.day)?,
                ('d', _) => out.write_str(self.days[t.weekday().unwrap_or(0) as usize])?,
                ('M', 1) => write!(out, "{}", t.month)?,
                ('M', 2) => write!(out, "{:02}", t.month)?,
                ('M', _) => out.write_str(self.months[usize::from(t.month.clamp(1, 12)) - 1])?,
                ('y', 1) => write!(out, "{}", t.year % 100)?,
                ('y', 2) => write!(out, "{:02}", t.year % 100)?,
                ('y', _) => write!(out, "{:04}", t.year)?,
                ('h', 1) => write!(out, "{hour12}")?,
                ('h', _) => write!(out, "{hour12:02}")?,
                ('H', 1) => write!(out, "{}", t.hour)?,
                ('H', _) => write!(out, "{:02}", t.hour)?,
                ('m', 1) => write!(out, "{}", t.minute)?,
                ('m', _) => write!(out, "{:02}", t.minute)?,
                ('s', 1) => write!(out, "{}", t.second)?,
                ('s', _) => write!(out, "{:02}", t.second)?,
                ('t', 1) => {
                    if let Some(first) = designator.chars().next() {
                        out.write_char(first)?;
                    }
                }
                _ => out.write_str(designator)?,
            }
        }
        Ok(())
    }
    pub fn short_date(&self, t: &WallClock, out: &mut dyn Write) -> fmt::Result {
        self.format(self.short_date, t, out)
    }
    pub fn long_date(&self, t: &WallClock, out: &mut dyn Write) -> fmt::Result {
        self.format(self.long_date, t, out)
    }
    pub fn short_time(&self, t: &WallClock, out: &mut dyn Write) -> fmt::Result {
        self.format(self.short_time, t, out)
    }
    pub fn long_time(&self, t: &WallClock, out: &mut dyn Write) -> fmt::Result {
        self.format(self.long_time, t, out)
    }
    /// Explorer's Date modified / Created text: short date, space, short time.
    pub fn date_time(&self, t: &WallClock, out: &mut dyn Write) -> fmt::Result {
        self.short_date(t, out)?;
        out.write_char(' ')?;
        self.short_time(t, out)
    }
    /// An unsigned integer with this region's digit grouping (groups of 3).
    pub fn integer(&self, value: u64, out: &mut dyn Write) -> fmt::Result {
        let mut digits = [0u8; 20];
        let mut n = 0;
        let mut v = value;
        loop {
            digits[n] = b'0' + (v % 10) as u8;
            n += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        for i in (0..n).rev() {
            out.write_char(char::from(digits[i]))?;
            if i > 0 && i % 3 == 0 {
                out.write_str(self.group)?;
            }
        }
        Ok(())
    }
    /// `units / 10^scale` with `scale` fraction digits, grouped, with this
    /// region's decimal separator (no rounding: the caller supplies units).
    pub fn decimal(&self, units: u64, scale: u32, out: &mut dyn Write) -> fmt::Result {
        let divisor = 10u64.checked_pow(scale).ok_or(fmt::Error)?;
        self.integer(units / divisor, out)?;
        if scale > 0 {
            out.write_str(self.decimal)?;
            write!(out, "{:0width$}", units % divisor, width = scale as usize)?;
        }
        Ok(())
    }
    /// This region with the user's Regional format overrides applied
    /// (Settings > Language & region > Regional format: 24-hour time, first
    /// day of week and the four formats). Pattern picks come first; the
    /// clock choice then converts the time patterns to 12 or 24 hours.
    pub fn customized(&self, o: Overrides) -> Region {
        let mut r = *self;
        let pick = |list: &[&'static str], index: u8| {
            usize::from(index)
                .checked_sub(1)
                .and_then(|i| list.get(i).copied())
        };
        if let Some(p) = pick(&SHORT_DATES, o.short_date) {
            r.short_date = p;
        }
        if let Some(p) = pick(&LONG_DATES, o.long_date) {
            r.long_date = p;
        }
        if let Some(p) = pick(&SHORT_TIMES, o.short_time) {
            r.short_time = p;
        }
        if let Some(p) = pick(&LONG_TIMES, o.long_time) {
            r.long_time = p;
        }
        if (1..=7).contains(&o.first_day) {
            r.first_day = o.first_day - 1;
        }
        match o.clock {
            1 => {
                r.short_time = twelve_hour(r.short_time, false);
                r.long_time = twelve_hour(r.long_time, true);
            }
            2 => {
                r.short_time = twenty_four_hour(r.short_time, false);
                r.long_time = twenty_four_hour(r.long_time, true);
            }
            _ => (),
        }
        r
    }
    /// The short time shows 24 hours (no AM/PM designator).
    pub fn uses_24_hour(&self) -> bool {
        !self.short_time.contains('t')
    }
}

/// Settings > Language & region > Regional format > Short date choices
/// (index + 1 is stored in the overrides word; 0 keeps the region's).
pub const SHORT_DATES: [&str; 10] = [
    "M/d/yyyy",
    "M/d/yy",
    "MM/dd/yy",
    "MM/dd/yyyy",
    "yy/MM/dd",
    "yyyy-MM-dd",
    "dd-MMM-yy",
    "dd/MM/yyyy",
    "dd.MM.yyyy",
    "yyyy/M/d",
];
/// Long date choices.
pub const LONG_DATES: [&str; 6] = [
    "dddd, MMMM d, yyyy",
    "MMMM d, yyyy",
    "dddd, d MMMM, yyyy",
    "d MMMM, yyyy",
    "dddd d MMMM yyyy",
    "dd MMMM yyyy",
];
/// Short time choices.
pub const SHORT_TIMES: [&str; 4] = ["h:mm tt", "hh:mm tt", "H:mm", "HH:mm"];
/// Long time choices.
pub const LONG_TIMES: [&str; 4] = ["h:mm:ss tt", "hh:mm:ss tt", "H:mm:ss", "HH:mm:ss"];

const fn field(bits: u32, shift: u32, mask: u32, max: u32) -> u8 {
    let v = (bits >> shift) & mask;
    if v > max { 0 } else { v as u8 }
}
fn twelve_hour(pattern: &'static str, long: bool) -> &'static str {
    if pattern.contains('t') {
        pattern
    } else if long {
        LONG_TIMES[if pattern.contains("HH") { 1 } else { 0 }]
    } else {
        SHORT_TIMES[if pattern.contains("HH") { 1 } else { 0 }]
    }
}
fn twenty_four_hour(pattern: &'static str, long: bool) -> &'static str {
    if !pattern.contains('t') {
        pattern
    } else if long {
        LONG_TIMES[if pattern.contains("hh") { 3 } else { 2 }]
    } else {
        SHORT_TIMES[if pattern.contains("hh") { 3 } else { 2 }]
    }
}

/// The user's Regional format overrides, decoded from the
/// `preferences::REGIONAL_OVERRIDES` word (`door_service_protocol`): every
/// field 0 keeps the region's own. `clock` 1 = 12-hour, 2 = 24-hour;
/// `first_day` 1..=7 = Sunday..Saturday; the rest are 1-based indices into
/// [`SHORT_DATES`], [`LONG_DATES`], [`SHORT_TIMES`] and [`LONG_TIMES`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Overrides {
    pub clock: u8,
    pub first_day: u8,
    pub short_date: u8,
    pub long_date: u8,
    pub short_time: u8,
    pub long_time: u8,
}
impl Overrides {
    pub const NONE: Self = Self {
        clock: 0,
        first_day: 0,
        short_date: 0,
        long_date: 0,
        short_time: 0,
        long_time: 0,
    };
    /// Decode the overrides word (bits 0..2 clock, 2..5 first day, then
    /// 4-bit short date, long date, short time and long time indices).
    /// Out-of-range fields read as 0 (the region's own).
    pub const fn from_word(bits: u32) -> Self {
        Self {
            clock: field(bits, 0, 3, 2),
            first_day: field(bits, 2, 7, 7),
            short_date: field(bits, 5, 15, SHORT_DATES.len() as u32),
            long_date: field(bits, 9, 15, LONG_DATES.len() as u32),
            short_time: field(bits, 13, 15, SHORT_TIMES.len() as u32),
            long_time: field(bits, 17, 15, LONG_TIMES.len() as u32),
        }
    }
    pub const fn word(self) -> u32 {
        self.clock as u32
            | (self.first_day as u32) << 2
            | (self.short_date as u32) << 5
            | (self.long_date as u32) << 9
            | (self.short_time as u32) << 13
            | (self.long_time as u32) << 17
    }
}

#[cfg(test)]
mod overrides_tests {
    use super::*;
    #[test]
    fn overrides_customize_patterns_clock_and_first_day() {
        let us = region_or_default(1);
        assert!(!us.uses_24_hour());
        let o = Overrides {
            clock: 2,
            ..Overrides::NONE
        };
        let r = us.customized(o);
        assert_eq!((r.short_time, r.long_time), ("H:mm", "H:mm:ss"));
        assert!(r.uses_24_hour());
        let gb = by_id(REGIONS[1].id.into()).unwrap();
        let r = gb.customized(Overrides {
            clock: 1,
            first_day: 1,
            short_date: 6,
            ..Overrides::NONE
        });
        assert_eq!(
            (r.short_time, r.first_day, r.short_date),
            ("hh:mm tt", 0, "yyyy-MM-dd")
        );
        assert_eq!(Overrides::from_word(o.word()), o);
        assert_eq!(Overrides::from_word(3), Overrides::NONE);
        assert_eq!(us.customized(Overrides::NONE), *us);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::string::String;
    fn f(region: &Region, pattern: &str, t: &WallClock) -> String {
        let mut s = String::new();
        region.format(pattern, t, &mut s).unwrap();
        s
    }
    const T: WallClock = WallClock {
        year: 2026,
        month: 10,
        day: 4,
        hour: 15,
        minute: 7,
        second: 9,
    };
    const MORNING: WallClock = WallClock {
        year: 2026,
        month: 3,
        day: 1,
        hour: 0,
        minute: 5,
        second: 0,
    };
    #[test]
    fn windows_default_formats_for_seven_regions() {
        let expect = [
            (
                "en-US",
                "10/4/2026",
                "Sunday, October 4, 2026",
                "3:07 PM",
                "3:07:09 PM",
                "12:05 AM",
            ),
            (
                "en-GB",
                "04/10/2026",
                "04 October 2026",
                "15:07",
                "15:07:09",
                "00:05",
            ),
            (
                "de-DE",
                "04.10.2026",
                "Sonntag, 4. Oktober 2026",
                "15:07",
                "15:07:09",
                "00:05",
            ),
            (
                "fr-FR",
                "04/10/2026",
                "dimanche 4 octobre 2026",
                "15:07",
                "15:07:09",
                "00:05",
            ),
            (
                "zh-CN",
                "2026/10/4",
                "2026年10月4日",
                "15:07",
                "15:07:09",
                "0:05",
            ),
            (
                "ja-JP",
                "2026/10/04",
                "2026年10月4日",
                "15:07",
                "15:07:09",
                "0:05",
            ),
            (
                "ko-KR",
                "2026-10-04",
                "2026년 10월 4일 일요일",
                "오후 3:07",
                "오후 3:07:09",
                "오전 12:05",
            ),
        ];
        assert_eq!(REGIONS.len(), expect.len());
        for (r, (tag, short, long, time, long_time, morning)) in REGIONS.iter().zip(expect) {
            assert_eq!(r.tag, tag);
            assert_eq!(f(r, r.short_date, &T), short, "{tag}");
            assert_eq!(f(r, r.long_date, &T), long, "{tag}");
            assert_eq!(f(r, r.short_time, &T), time, "{tag}");
            assert_eq!(f(r, r.long_time, &T), long_time, "{tag}");
            assert_eq!(f(r, r.short_time, &MORNING), morning, "{tag}");
        }
        let de = by_id(3).unwrap();
        assert_eq!(f(de, "MMMM", &MORNING), "März");
        let mut both = String::new();
        REGIONS[0].date_time(&T, &mut both).unwrap();
        assert_eq!(both, "10/4/2026 3:07 PM");
        assert_eq!(f(&REGIONS[0], "'Today:' yy-M t", &T), "Today: 26-10 P");
        assert_eq!(
            f(&REGIONS[0], REGIONS[0].month_day, &T),
            "Sunday, October 4"
        );
        assert_eq!(f(&REGIONS[4], REGIONS[4].year_month, &T), "2026年10月");
        assert_eq!(f(de, de.numeric_month_day, &T), "04.10.");
    }
    #[test]
    fn first_day_numbers_and_ids() {
        let first: std::vec::Vec<_> = REGIONS.iter().map(|r| r.first_day).collect();
        assert_eq!(first, [0, 1, 1, 1, 1, 0, 0]);
        let n = |r: &Region, units, scale| {
            let mut s = String::new();
            r.decimal(units, scale, &mut s).unwrap();
            s
        };
        assert_eq!(n(&REGIONS[0], 123_456_789, 2), "1,234,567.89");
        assert_eq!(n(&REGIONS[2], 123_456_789, 2), "1.234.567,89");
        assert_eq!(n(&REGIONS[3], 1_234_567, 0), "1\u{202f}234\u{202f}567");
        assert_eq!(n(&REGIONS[1], 999, 0), "999");
        assert_eq!(n(&REGIONS[0], 5, 3), "0.005");
        assert_eq!(n(&REGIONS[0], u64::MAX, 0), "18,446,744,073,709,551,615");
        assert!(valid(1) && valid(7) && !valid(0) && !valid(8));
        assert_eq!(region_or_default(99).tag, "en-US");
        let mut small = Text::<8>::new();
        assert!(REGIONS[0].long_date(&T, &mut small).is_err());
        let mut text = Text::<32>::new();
        REGIONS[6].short_time(&T, &mut text).unwrap();
        assert_eq!(text.as_str(), "오후 3:07");
    }
}
