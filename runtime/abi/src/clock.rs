//! Captured civil time from the declared QEMU RTC, expressed in UTC, and the
//! FILETIME representation (100 ns intervals since 1601-01-01 00:00:00 UTC)
//! returned by `operation::WALL_TIME` and stored in volume timestamps.
use crate::Status;
pub const BYTES: usize = 16;
/// FILETIME units per second (100 ns each).
pub const FILETIME_PER_SECOND: u64 = 10_000_000;
/// Days from 1601-01-01 to 1970-01-01.
pub const UNIX_EPOCH_DAYS: u64 = 134_774;
/// Days from 1601-01-01 to a proleptic Gregorian date in 1601..=9999
/// (the caller validates the date).
pub const fn days_from_civil(year: u16, month: u8, day: u8) -> u64 {
    let y = year as i64 - if month <= 2 { 1 } else { 0 };
    let era = y / 400;
    let yoe = y - era * 400;
    let mp = (month as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    // 1601-01-01 is day 584_694 of the March-based count from 0000-03-01.
    (era * 146_097 + doe - 584_694) as u64
}
/// `days_from_civil(9999, 12, 31)`.
pub const LAST_DAY: u64 = 3_067_670;
/// Inverse of `days_from_civil` for 1601..=9999 (None after 9999-12-31).
pub const fn civil_from_days(days: u64) -> Option<(u16, u8, u8)> {
    if days > LAST_DAY {
        return None;
    }
    let z = days as i64 + 584_694;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    Some((year as u16, month as u8, day as u8))
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WallClock {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}
pub const fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(400) || year.is_multiple_of(4) && !year.is_multiple_of(100) => 29,
        2 => 28,
        _ => 0,
    }
}
impl WallClock {
    pub fn valid(self) -> bool {
        (1601..=9999).contains(&self.year)
            && (1..=12).contains(&self.month)
            && self.day >= 1
            && self.day <= days_in_month(self.year, self.month)
            && self.hour < 24
            && self.minute < 60
            && self.second < 60
    }
    /// Gregorian weekday, Sunday=0. The valid year range keeps arithmetic signed-safe.
    pub fn weekday(self) -> Option<u8> {
        if !self.valid() {
            return None;
        }
        let year = u32::from(self.year) - u32::from(self.month < 3);
        let offsets = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        Some(
            ((year + year / 4 - year / 100
                + year / 400
                + offsets[self.month as usize - 1]
                + u32::from(self.day))
                % 7) as u8,
        )
    }
    /// FILETIME of this instant (whole seconds); None if invalid.
    pub fn to_filetime(self) -> Option<u64> {
        if !self.valid() {
            return None;
        }
        let seconds = days_from_civil(self.year, self.month, self.day) * 86_400
            + u64::from(self.hour) * 3600
            + u64::from(self.minute) * 60
            + u64::from(self.second);
        Some(seconds * FILETIME_PER_SECOND)
    }
    /// Civil time of a FILETIME (sub-seconds truncated); None after 9999.
    pub fn from_filetime(filetime: u64) -> Option<Self> {
        let seconds = filetime / FILETIME_PER_SECOND;
        let (year, month, day) = civil_from_days(seconds / 86_400)?;
        let rest = seconds % 86_400;
        Some(Self {
            year,
            month,
            day,
            hour: (rest / 3600) as u8,
            minute: (rest / 60 % 60) as u8,
            second: (rest % 60) as u8,
        })
    }
    pub fn encode(self) -> Result<[u8; BYTES], Status> {
        if !self.valid() {
            return Err(Status::InvalidArgument);
        }
        let mut bytes = [0; BYTES];
        bytes[..2].copy_from_slice(&self.year.to_le_bytes());
        bytes[2..7].copy_from_slice(&[self.month, self.day, self.hour, self.minute, self.second]);
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Status> {
        if bytes.len() != BYTES || bytes[7..].iter().any(|byte| *byte != 0) {
            return Err(Status::InvalidArgument);
        }
        let time = Self {
            year: u16::from_le_bytes([bytes[0], bytes[1]]),
            month: bytes[2],
            day: bytes[3],
            hour: bytes[4],
            minute: bytes[5],
            second: bytes[6],
        };
        if time.valid() {
            Ok(time)
        } else {
            Err(Status::InvalidArgument)
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calendar_and_checked_reserved_wire() {
        let time = WallClock {
            year: 2026,
            month: 10,
            day: 19,
            hour: 12,
            minute: 30,
            second: 59,
        };
        assert_eq!(time.weekday(), Some(1));
        assert_eq!(WallClock::decode(&time.encode().unwrap()), Ok(time));
        let mut bad = time.encode().unwrap();
        bad[15] = 1;
        assert!(WallClock::decode(&bad).is_err());
        assert_eq!(days_in_month(2000, 2), 29);
        // FILETIME: epoch, Unix epoch, a known instant and the 9999 bound.
        let epoch = WallClock {
            year: 1601,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        };
        assert_eq!(epoch.to_filetime(), Some(0));
        assert_eq!(days_from_civil(1970, 1, 1), UNIX_EPOCH_DAYS);
        assert_eq!(days_from_civil(9999, 12, 31), LAST_DAY);
        // 2026-10-19 12:30:59 UTC = Unix 1792413059.
        let unix = time.to_filetime().unwrap() / FILETIME_PER_SECOND - UNIX_EPOCH_DAYS * 86_400;
        assert_eq!(unix, 1_792_413_059);
        assert_eq!(
            WallClock::from_filetime(time.to_filetime().unwrap() + 9_999_999),
            Some(time)
        );
        let last = WallClock {
            year: 9999,
            month: 12,
            day: 31,
            hour: 23,
            minute: 59,
            second: 59,
        };
        assert_eq!(
            WallClock::from_filetime(last.to_filetime().unwrap()),
            Some(last)
        );
        assert_eq!(
            WallClock::from_filetime(last.to_filetime().unwrap() + FILETIME_PER_SECOND),
            None
        );
        for days in (0..=LAST_DAY).step_by(97) {
            let (y, m, d) = civil_from_days(days).unwrap();
            assert_eq!(days_from_civil(y, m, d), days);
            assert!(d >= 1 && d <= days_in_month(y, m));
        }
        assert_eq!(days_in_month(2100, 2), 28);
        assert!(
            !WallClock {
                day: 31,
                month: 4,
                ..time
            }
            .valid()
        );
    }
}
