//! Time zones: Windows display names with IANA-derived current rules.
//!
//! Source: IANA tz database (tzdata 2025b, public domain), zones `northamerica`,
//! `southamerica`, `europe`, `africa`, `asia`, `australasia`. Only the rule in
//! force since each zone's last change is kept (for example Mexico City and
//! Tehran without DST since 2022, Cairo with DST since 2023); instants before
//! a zone's last rule change convert with today's rule. Settings store the
//! stable `Zone::id`, never the table index.
use door_abi::clock::{FILETIME_PER_SECOND, WallClock, days_from_civil, days_in_month};

/// Clock basis of a transition's `minute`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Basis {
    /// Local wall-clock time in effect just before the transition.
    Wall,
    /// Local standard time.
    Standard,
    /// UTC.
    Utc,
}
/// The `week`-th (1..=4, 5 = last) `weekday` (0 = Sunday) of `month`, at
/// `minute` minutes after that day's midnight (may be 1440 = 24:00).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Transition {
    pub month: u8,
    pub week: u8,
    pub weekday: u8,
    pub minute: u16,
    pub basis: Basis,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Dst {
    pub start: Transition,
    pub end: Transition,
    /// Minutes added during daylight saving time.
    pub save: i16,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Zone {
    /// Stable persisted identifier (`Setting::TimeZone`).
    pub id: u16,
    pub iana: &'static str,
    /// Windows 11 Settings > Date & time display name.
    pub display: &'static str,
    /// Standard offset, minutes east of UTC.
    pub standard: i16,
    pub dst: Option<Dst>,
}
/// Local civil time of an instant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Local {
    pub time: WallClock,
    /// Offset in effect, minutes east of UTC.
    pub offset: i16,
    pub dst: bool,
}
/// Conversion of a local wall-clock time to UTC FILETIME.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalResult {
    Unique(u64),
    /// Repeated hour at the end of DST: both instants, earlier first.
    Ambiguous(u64, u64),
    /// Skipped hour at the start of DST: no instant has this local time.
    Nonexistent,
}

const fn t(month: u8, week: u8, weekday: u8, minute: u16, basis: Basis) -> Transition {
    Transition {
        month,
        week,
        weekday,
        minute,
        basis,
    }
}
/// US/Canada since 2007: second Sunday of March 02:00 to first Sunday of
/// November 02:00, local wall time.
const US: Option<Dst> = Some(Dst {
    start: t(3, 2, 0, 120, Basis::Wall),
    end: t(11, 1, 0, 120, Basis::Wall),
    save: 60,
});
/// EU since 1996: last Sunday of March to last Sunday of October, 01:00 UTC.
const EU: Option<Dst> = Some(Dst {
    start: t(3, 5, 0, 60, Basis::Utc),
    end: t(10, 5, 0, 60, Basis::Utc),
    save: 60,
});
/// Australia (NSW, Vic, ACT, SA) since 2008: first Sunday of October to first
/// Sunday of April, 02:00 standard time.
const AU: Option<Dst> = Some(Dst {
    start: t(10, 1, 0, 120, Basis::Standard),
    end: t(4, 1, 0, 120, Basis::Standard),
    save: 60,
});
/// New Zealand since 2007: last Sunday of September to first Sunday of
/// April, 02:00 standard time.
const NZ: Option<Dst> = Some(Dst {
    start: t(9, 5, 0, 120, Basis::Standard),
    end: t(4, 1, 0, 120, Basis::Standard),
    save: 60,
});
/// Egypt since 2023: last Friday of April 00:00 to last Thursday of October
/// 24:00, local wall time.
const EGYPT: Option<Dst> = Some(Dst {
    start: t(4, 5, 5, 0, Basis::Wall),
    end: t(10, 5, 4, 1440, Basis::Wall),
    save: 60,
});
const fn z(
    id: u16,
    iana: &'static str,
    display: &'static str,
    standard: i16,
    dst: Option<Dst>,
) -> Zone {
    Zone {
        id,
        iana,
        display,
        standard,
        dst,
    }
}
/// In Windows dropdown order (by offset). At most 64 entries (Settings combo).
pub static ZONES: [Zone; 49] = [
    z(1, "Pacific/Honolulu", "(UTC-10:00) Hawaii", -600, None),
    z(2, "America/Anchorage", "(UTC-09:00) Alaska", -540, US),
    z(
        3,
        "America/Los_Angeles",
        "(UTC-08:00) Pacific Time (US & Canada)",
        -480,
        US,
    ),
    z(4, "America/Phoenix", "(UTC-07:00) Arizona", -420, None),
    z(
        5,
        "America/Denver",
        "(UTC-07:00) Mountain Time (US & Canada)",
        -420,
        US,
    ),
    z(
        6,
        "America/Chicago",
        "(UTC-06:00) Central Time (US & Canada)",
        -360,
        US,
    ),
    z(
        7,
        "America/Mexico_City",
        "(UTC-06:00) Guadalajara, Mexico City, Monterrey",
        -360,
        None,
    ),
    z(8, "America/Regina", "(UTC-06:00) Saskatchewan", -360, None),
    z(
        9,
        "America/Bogota",
        "(UTC-05:00) Bogota, Lima, Quito, Rio Branco",
        -300,
        None,
    ),
    z(
        10,
        "America/New_York",
        "(UTC-05:00) Eastern Time (US & Canada)",
        -300,
        US,
    ),
    z(
        11,
        "America/Halifax",
        "(UTC-04:00) Atlantic Time (Canada)",
        -240,
        US,
    ),
    z(12, "America/St_Johns", "(UTC-03:30) Newfoundland", -210, US),
    z(13, "America/Sao_Paulo", "(UTC-03:00) Brasilia", -180, None),
    z(
        14,
        "America/Argentina/Buenos_Aires",
        "(UTC-03:00) City of Buenos Aires",
        -180,
        None,
    ),
    z(15, "Atlantic/Azores", "(UTC-01:00) Azores", -60, EU),
    z(16, "Etc/UTC", "(UTC) Coordinated Universal Time", 0, None),
    z(
        17,
        "Europe/London",
        "(UTC+00:00) Dublin, Edinburgh, Lisbon, London",
        0,
        EU,
    ),
    z(
        18,
        "Atlantic/Reykjavik",
        "(UTC+00:00) Monrovia, Reykjavik",
        0,
        None,
    ),
    z(
        19,
        "Europe/Berlin",
        "(UTC+01:00) Amsterdam, Berlin, Bern, Rome, Stockholm, Vienna",
        60,
        EU,
    ),
    z(
        20,
        "Europe/Budapest",
        "(UTC+01:00) Belgrade, Bratislava, Budapest, Ljubljana, Prague",
        60,
        EU,
    ),
    z(
        21,
        "Europe/Paris",
        "(UTC+01:00) Brussels, Copenhagen, Madrid, Paris",
        60,
        EU,
    ),
    z(
        22,
        "Europe/Warsaw",
        "(UTC+01:00) Sarajevo, Skopje, Warsaw, Zagreb",
        60,
        EU,
    ),
    z(
        23,
        "Africa/Lagos",
        "(UTC+01:00) West Central Africa",
        60,
        None,
    ),
    z(
        24,
        "Europe/Bucharest",
        "(UTC+02:00) Athens, Bucharest",
        120,
        EU,
    ),
    z(25, "Africa/Cairo", "(UTC+02:00) Cairo", 120, EGYPT),
    z(
        26,
        "Africa/Johannesburg",
        "(UTC+02:00) Harare, Pretoria",
        120,
        None,
    ),
    z(
        27,
        "Europe/Kyiv",
        "(UTC+02:00) Helsinki, Kyiv, Riga, Sofia, Tallinn, Vilnius",
        120,
        EU,
    ),
    z(28, "Europe/Istanbul", "(UTC+03:00) Istanbul", 180, None),
    z(29, "Asia/Riyadh", "(UTC+03:00) Kuwait, Riyadh", 180, None),
    z(
        30,
        "Europe/Moscow",
        "(UTC+03:00) Moscow, St. Petersburg",
        180,
        None,
    ),
    z(31, "Africa/Nairobi", "(UTC+03:00) Nairobi", 180, None),
    z(32, "Asia/Tehran", "(UTC+03:30) Tehran", 210, None),
    z(33, "Asia/Dubai", "(UTC+04:00) Abu Dhabi, Muscat", 240, None),
    z(
        34,
        "Asia/Karachi",
        "(UTC+05:00) Islamabad, Karachi",
        300,
        None,
    ),
    z(
        35,
        "Asia/Kolkata",
        "(UTC+05:30) Chennai, Kolkata, Mumbai, New Delhi",
        330,
        None,
    ),
    z(36, "Asia/Kathmandu", "(UTC+05:45) Kathmandu", 345, None),
    z(37, "Asia/Dhaka", "(UTC+06:00) Dhaka", 360, None),
    z(
        38,
        "Asia/Bangkok",
        "(UTC+07:00) Bangkok, Hanoi, Jakarta",
        420,
        None,
    ),
    z(
        39,
        "Asia/Shanghai",
        "(UTC+08:00) Beijing, Chongqing, Hong Kong, Urumqi",
        480,
        None,
    ),
    z(
        40,
        "Asia/Singapore",
        "(UTC+08:00) Kuala Lumpur, Singapore",
        480,
        None,
    ),
    z(41, "Australia/Perth", "(UTC+08:00) Perth", 480, None),
    z(42, "Asia/Taipei", "(UTC+08:00) Taipei", 480, None),
    z(
        43,
        "Asia/Tokyo",
        "(UTC+09:00) Osaka, Sapporo, Tokyo",
        540,
        None,
    ),
    z(44, "Asia/Seoul", "(UTC+09:00) Seoul", 540, None),
    z(45, "Australia/Adelaide", "(UTC+09:30) Adelaide", 570, AU),
    z(46, "Australia/Darwin", "(UTC+09:30) Darwin", 570, None),
    z(47, "Australia/Brisbane", "(UTC+10:00) Brisbane", 600, None),
    z(
        48,
        "Australia/Sydney",
        "(UTC+10:00) Canberra, Melbourne, Sydney",
        600,
        AU,
    ),
    z(
        49,
        "Pacific/Auckland",
        "(UTC+12:00) Auckland, Wellington",
        720,
        NZ,
    ),
];
/// `Setting::TimeZone` default: UTC, the RTC's own time base. No location
/// provider exists to choose another.
pub const DEFAULT_ID: u16 = 16;
pub fn by_id(id: u32) -> Option<&'static Zone> {
    ZONES.iter().find(|zone| u32::from(zone.id) == id)
}
pub fn index_of(id: u32) -> Option<usize> {
    ZONES.iter().position(|zone| u32::from(zone.id) == id)
}
pub fn valid(id: u32) -> bool {
    by_id(id).is_some()
}
/// The configured zone, or UTC for an unknown id.
pub fn zone_or_utc(id: u32) -> &'static Zone {
    by_id(id).unwrap_or_else(|| by_id(u32::from(DEFAULT_ID)).unwrap())
}

const SECOND: i64 = FILETIME_PER_SECOND as i64;
/// Day of `rule` in `year` (1-based).
fn rule_day(year: u16, rule: Transition) -> u8 {
    let weekday = |day: u8| ((days_from_civil(year, rule.month, day) + 1) % 7) as u8;
    if rule.week >= 5 {
        let last = days_in_month(year, rule.month);
        last - (weekday(last) + 7 - rule.weekday) % 7
    } else {
        1 + (rule.weekday + 7 - weekday(1)) % 7 + 7 * (rule.week - 1)
    }
}
impl Zone {
    pub fn observes_dst(&self) -> bool {
        self.dst.is_some()
    }
    /// UTC second (since 1601) of a transition in `year`.
    fn transition(&self, year: u16, rule: Transition, ending: bool, save: i16) -> i64 {
        let day = rule_day(year, rule);
        let local =
            days_from_civil(year, rule.month, day) as i64 * 86_400 + i64::from(rule.minute) * 60;
        let before = match rule.basis {
            Basis::Utc => 0,
            Basis::Standard => self.standard,
            Basis::Wall if ending => self.standard + save,
            Basis::Wall => self.standard,
        };
        local - i64::from(before) * 60
    }
    /// Offset (minutes east) and DST flag at a UTC second since 1601.
    fn offset_seconds(&self, second: i64) -> (i16, bool) {
        let Some(dst) = self.dst else {
            return (self.standard, false);
        };
        let local = second + i64::from(self.standard) * 60;
        let Some(day) = u64::try_from(local.div_euclid(86_400)).ok() else {
            return (self.standard, false);
        };
        let Some((year, _, _)) = door_abi::clock::civil_from_days(day) else {
            return (self.standard, false);
        };
        let start = self.transition(year, dst.start, false, dst.save);
        let end = self.transition(year, dst.end, true, dst.save);
        let on = if start < end {
            (start..end).contains(&second)
        } else {
            second >= start || second < end
        };
        if on {
            (self.standard + dst.save, true)
        } else {
            (self.standard, false)
        }
    }
    /// Offset (minutes east of UTC) and DST flag in effect at `utc`.
    pub fn offset_at(&self, utc: u64) -> (i16, bool) {
        self.offset_seconds((utc / FILETIME_PER_SECOND) as i64)
    }
    /// Local civil time of a UTC FILETIME; None outside 1601..=9999 local.
    pub fn to_local(&self, utc: u64) -> Option<Local> {
        let (offset, dst) = self.offset_at(utc);
        let local = i64::try_from(utc)
            .ok()?
            .checked_add(i64::from(offset) * 60 * SECOND)?;
        let time = WallClock::from_filetime(u64::try_from(local).ok()?)?;
        Some(Local { time, offset, dst })
    }
    /// UTC FILETIME of a local wall-clock time under this zone's rules.
    pub fn to_utc(&self, local: WallClock) -> LocalResult {
        let Some(ft) = local.to_filetime() else {
            return LocalResult::Nonexistent;
        };
        let wall = (ft / FILETIME_PER_SECOND) as i64;
        let save = self.dst.map_or(0, |d| d.save);
        let mut found = [None; 2];
        for (slot, offset) in [self.standard + save, self.standard]
            .into_iter()
            .enumerate()
        {
            let candidate = wall - i64::from(offset) * 60;
            if candidate >= 0
                && self.offset_seconds(candidate).0 == offset
                && (slot == 0 || found[0] != Some(candidate))
            {
                found[slot] = Some(candidate);
            }
        }
        let ft = |s: i64| s as u64 * FILETIME_PER_SECOND;
        match found {
            [Some(a), Some(b)] if save != 0 => LocalResult::Ambiguous(ft(a.min(b)), ft(a.max(b))),
            [Some(a), _] | [None, Some(a)] => LocalResult::Unique(ft(a)),
            [None, None] => LocalResult::Nonexistent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn at(year: u16, month: u8, day: u8, hour: u8, minute: u8) -> WallClock {
        WallClock {
            year,
            month,
            day,
            hour,
            minute,
            second: 0,
        }
    }
    fn utc(year: u16, month: u8, day: u8, hour: u8, minute: u8) -> u64 {
        at(year, month, day, hour, minute).to_filetime().unwrap()
    }
    fn zone(iana: &str) -> &'static Zone {
        ZONES.iter().find(|z| z.iana == iana).unwrap()
    }
    #[test]
    fn table_ids_are_unique_ordered_by_offset_and_fit_the_settings_combo() {
        assert!(ZONES.len() <= 64);
        for (i, a) in ZONES.iter().enumerate() {
            assert!(
                ZONES[i + 1..]
                    .iter()
                    .all(|b| b.id != a.id && b.iana != a.iana)
            );
            if i > 0 {
                assert!(ZONES[i - 1].standard <= a.standard, "{}", a.display);
            }
            assert!(a.display.starts_with("(UTC"));
        }
        assert_eq!(zone_or_utc(0).iana, "Etc/UTC");
        assert_eq!(by_id(u32::from(DEFAULT_ID)).unwrap().standard, 0);
        assert!(valid(49) && !valid(50) && !valid(0));
    }
    #[test]
    fn us_eu_southern_and_egypt_rules_switch_at_the_published_instants() {
        let ny = zone("America/New_York");
        // 2026-03-08 02:00 EST = 07:00 UTC; 2026-11-01 02:00 EDT = 06:00 UTC.
        assert_eq!(ny.offset_at(utc(2026, 3, 8, 6, 59)), (-300, false));
        assert_eq!(ny.offset_at(utc(2026, 3, 8, 7, 0)), (-240, true));
        assert_eq!(ny.offset_at(utc(2026, 11, 1, 5, 59)), (-240, true));
        assert_eq!(ny.offset_at(utc(2026, 11, 1, 6, 0)), (-300, false));
        let la = zone("America/Los_Angeles")
            .to_local(utc(2026, 10, 24, 19, 5))
            .unwrap();
        assert_eq!(
            (la.time.hour, la.time.minute, la.offset, la.dst),
            (12, 5, -420, true)
        );
        let paris = zone("Europe/Paris");
        assert_eq!(paris.offset_at(utc(2026, 3, 29, 0, 59)).0, 60);
        assert_eq!(paris.offset_at(utc(2026, 3, 29, 1, 0)).0, 120);
        assert_eq!(paris.offset_at(utc(2026, 10, 25, 0, 59)).0, 120);
        assert_eq!(paris.offset_at(utc(2026, 10, 25, 1, 0)).0, 60);
        assert_eq!(
            zone("Europe/London").offset_at(utc(2026, 7, 1, 0, 0)),
            (60, true)
        );
        let sydney = zone("Australia/Sydney");
        // DST ends 2026-04-05 03:00 AEDT (16:00 UTC on the 4th), starts
        // 2026-10-04 02:00 AEST (16:00 UTC on the 3rd).
        assert_eq!(sydney.offset_at(utc(2026, 1, 15, 0, 0)), (660, true));
        assert_eq!(sydney.offset_at(utc(2026, 4, 4, 15, 59)).0, 660);
        assert_eq!(sydney.offset_at(utc(2026, 4, 4, 16, 0)).0, 600);
        assert_eq!(sydney.offset_at(utc(2026, 10, 3, 15, 59)).0, 600);
        assert_eq!(sydney.offset_at(utc(2026, 10, 3, 16, 0)).0, 660);
        let auckland = zone("Pacific/Auckland");
        assert_eq!(auckland.offset_at(utc(2026, 9, 26, 13, 59)).0, 720);
        assert_eq!(auckland.offset_at(utc(2026, 9, 26, 14, 0)).0, 780);
        let cairo = zone("Africa/Cairo");
        // Last Friday of April 2026 (24th) 00:00 EET; last Thursday of
        // October (29th) 24:00 EEST.
        assert_eq!(cairo.offset_at(utc(2026, 4, 23, 21, 59)).0, 120);
        assert_eq!(cairo.offset_at(utc(2026, 4, 23, 22, 0)).0, 180);
        assert_eq!(cairo.offset_at(utc(2026, 10, 29, 20, 59)).0, 180);
        assert_eq!(cairo.offset_at(utc(2026, 10, 29, 21, 0)).0, 120);
        let kolkata = zone("Asia/Kolkata")
            .to_local(utc(2026, 12, 31, 18, 45))
            .unwrap();
        assert_eq!(
            (
                kolkata.time.year,
                kolkata.time.month,
                kolkata.time.day,
                kolkata.time.hour,
                kolkata.time.minute
            ),
            (2027, 1, 1, 0, 15)
        );
        assert_eq!(zone("Asia/Kathmandu").offset_at(0), (345, false));
        assert_eq!(
            zone("America/Phoenix").offset_at(utc(2026, 7, 1, 0, 0)),
            (-420, false)
        );
        // No local time before 1601: unavailable rather than wrapped.
        assert_eq!(zone("America/New_York").to_local(0), None);
    }
    #[test]
    fn local_times_in_gaps_and_overlaps_are_reported_not_guessed() {
        let ny = zone("America/New_York");
        assert_eq!(ny.to_utc(at(2026, 3, 8, 2, 30)), LocalResult::Nonexistent);
        assert_eq!(
            ny.to_utc(at(2026, 3, 8, 3, 0)),
            LocalResult::Unique(utc(2026, 3, 8, 7, 0))
        );
        assert_eq!(
            ny.to_utc(at(2026, 11, 1, 1, 30)),
            LocalResult::Ambiguous(utc(2026, 11, 1, 5, 30), utc(2026, 11, 1, 6, 30))
        );
        assert_eq!(
            ny.to_utc(at(2026, 7, 4, 12, 0)),
            LocalResult::Unique(utc(2026, 7, 4, 16, 0))
        );
        let tokyo = zone("Asia/Tokyo");
        assert_eq!(
            tokyo.to_utc(at(2026, 1, 1, 9, 0)),
            LocalResult::Unique(utc(2026, 1, 1, 0, 0))
        );
        for zone in &ZONES {
            let instant = utc(2026, 6, 15, 12, 0);
            let local = zone.to_local(instant).unwrap();
            match zone.to_utc(local.time) {
                LocalResult::Unique(u) => assert_eq!(u, instant, "{}", zone.iana),
                other => panic!("{} {other:?}", zone.iana),
            }
        }
    }
}
