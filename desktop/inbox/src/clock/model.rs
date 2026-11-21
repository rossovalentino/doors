//! Clock's models, driven by real time: the monotonic millisecond clock the
//! runtime's `tick` passes (stopwatch, timers, focus sessions) and the RTC's
//! UTC wall time in a `door_locale::tz` zone (alarms, world clock).
use alloc::{format, string::String, vec::Vec};

/// "hh:mm:ss.cc", as Windows' Stopwatch shows it.
pub fn stopwatch_text(ms: u64) -> String {
    let cs = ms / 10;
    format!(
        "{:02}:{:02}:{:02}.{:02}",
        cs / 360_000,
        cs / 6000 % 60,
        cs / 100 % 60,
        cs % 100
    )
}
/// "hh:mm:ss" for timers, rounding a partial second up (a timer showing
/// 00:00:01 still has time left).
pub fn timer_text(ms: u64) -> String {
    let s = ms.div_ceil(1000);
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stopwatch {
    since: Option<u64>,
    banked: u64,
    /// Lap end times (total elapsed at each lap), oldest first.
    pub laps: Vec<u64>,
}
impl Stopwatch {
    pub fn running(&self) -> bool {
        self.since.is_some()
    }
    pub fn elapsed(&self, now: u64) -> u64 {
        self.banked + self.since.map_or(0, |s| now.saturating_sub(s))
    }
    pub fn toggle(&mut self, now: u64) {
        match self.since.take() {
            Some(s) => self.banked += now.saturating_sub(s),
            None => self.since = Some(now),
        }
    }
    pub fn lap(&mut self, now: u64) {
        if self.running() && self.laps.len() < 999 {
            self.laps.push(self.elapsed(now));
        }
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// (lap number, lap time, total) newest first, as the laps table lists.
    pub fn lap_rows(&self) -> Vec<(usize, u64, u64)> {
        let mut prev = 0;
        let mut rows: Vec<_> = self
            .laps
            .iter()
            .enumerate()
            .map(|(i, &t)| {
                let row = (i + 1, t - prev, t);
                prev = t;
                row
            })
            .collect();
        rows.reverse();
        rows
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timer {
    pub name: String,
    pub duration: u64,
    since: Option<u64>,
    /// Time left when last paused.
    left: u64,
    /// The finish was announced (toast) for this run.
    pub announced: bool,
}
impl Timer {
    pub fn new(name: &str, duration: u64) -> Self {
        Self {
            name: name.into(),
            duration,
            since: None,
            left: duration,
            announced: false,
        }
    }
    pub fn running(&self) -> bool {
        self.since.is_some()
    }
    pub fn remaining(&self, now: u64) -> u64 {
        match self.since {
            Some(s) => self.left.saturating_sub(now.saturating_sub(s)),
            None => self.left,
        }
    }
    pub fn done(&self, now: u64) -> bool {
        self.remaining(now) == 0
    }
    /// Fraction left in [0, 1000] (the progress ring).
    pub fn permille(&self, now: u64) -> u32 {
        if self.duration == 0 {
            0
        } else {
            (self.remaining(now) * 1000 / self.duration) as u32
        }
    }
    /// Start, pause, or (once finished) start again from the full duration.
    pub fn toggle(&mut self, now: u64) {
        if self.done(now) {
            self.left = self.duration;
            self.announced = false;
            self.since = Some(now);
            return;
        }
        match self.since.take() {
            Some(s) => self.left = self.left.saturating_sub(now.saturating_sub(s)),
            None => self.since = Some(now),
        }
    }
    pub fn reset(&mut self) {
        self.since = None;
        self.left = self.duration;
        self.announced = false;
    }
}

/// Days an alarm repeats on, bit 0 = Sunday (Windows' week order).
pub const DAYS: [&str; 7] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alarm {
    pub name: String,
    pub hour: u8,
    pub minute: u8,
    pub days: u8,
    pub enabled: bool,
    /// The (day number, minute) it last rang, so it rings once.
    pub rang: Option<(u32, u16)>,
}
impl Alarm {
    pub fn new(name: &str, hour: u8, minute: u8) -> Self {
        Self {
            name: name.into(),
            hour,
            minute,
            days: 0,
            enabled: false,
            rang: None,
        }
    }
    /// "7:00 AM" (12-hour clock, as en-US shows it).
    pub fn time_text(&self) -> String {
        clock_text(self.hour, self.minute)
    }
    /// "Only once" or the repeat days ("Mon, Tue", "Every day", "Weekdays").
    pub fn days_text(&self) -> String {
        match self.days {
            0 => "Only once".into(),
            0x7f => "Every day".into(),
            0x3e => "Weekdays".into(),
            0x41 => "Weekends".into(),
            d => {
                let names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
                let mut out = String::new();
                for (i, n) in names.iter().enumerate() {
                    if d & (1 << i) != 0 {
                        if !out.is_empty() {
                            out.push_str(", ");
                        }
                        out.push_str(n);
                    }
                }
                out
            }
        }
    }
    /// Whether the alarm rings at local `weekday` (0 = Sunday) `hour:minute`
    /// on day number `day` (days since an epoch, to ring once).
    pub fn due(&mut self, day: u32, weekday: u8, hour: u8, minute: u8) -> bool {
        if !self.enabled || hour != self.hour || minute != self.minute {
            return false;
        }
        if self.days != 0 && self.days & (1 << weekday) == 0 {
            return false;
        }
        let key = (day, u16::from(hour) * 60 + u16::from(minute));
        if self.rang == Some(key) {
            return false;
        }
        self.rang = Some(key);
        if self.days == 0 {
            // A one-time alarm turns itself off once it has rung.
            self.enabled = false;
        }
        true
    }
    /// Minutes from local `weekday hour:minute` until it next rings.
    pub fn minutes_until(&self, weekday: u8, hour: u8, minute: u8) -> Option<u32> {
        if !self.enabled {
            return None;
        }
        let now = u32::from(hour) * 60 + u32::from(minute);
        let at = u32::from(self.hour) * 60 + u32::from(self.minute);
        (0..8).find_map(|d| {
            let day = (u32::from(weekday) + d) % 7;
            let ok = self.days == 0 || self.days & (1 << day) != 0;
            let minutes = d * 1440 + at;
            (ok && minutes > now).then(|| minutes - now)
        })
    }
}
/// "7:00 AM", "12:05 PM".
pub fn clock_text(hour: u8, minute: u8) -> String {
    let h = match hour % 12 {
        0 => 12,
        h => h,
    };
    format!("{h}:{minute:02} {}", if hour < 12 { "AM" } else { "PM" })
}
/// "Rings in 9 hours, 55 minutes".
pub fn rings_in(minutes: u32) -> String {
    let (h, m) = (minutes / 60, minutes % 60);
    let unit = |n: u32, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    match (h, m) {
        (0, m) => format!("Rings in {}", unit(m, "minute", "minutes")),
        (h, 0) => format!("Rings in {}", unit(h, "hour", "hours")),
        (h, m) => format!(
            "Rings in {}, {}",
            unit(h, "hour", "hours"),
            unit(m, "minute", "minutes")
        ),
    }
}

/// A focus session's plan: focus periods with 5-minute breaks between them.
/// Windows gives sessions of 45 minutes or more one break per 30 minutes
/// after the first (30 min: none, 45 or 60: one, 90: two).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FocusPlan {
    pub minutes: u32,
    pub skip_breaks: bool,
}
pub const BREAK_MINUTES: u32 = 5;
impl FocusPlan {
    pub fn breaks(self) -> u32 {
        if self.skip_breaks || self.minutes < 45 {
            0
        } else {
            self.minutes.div_ceil(30) - 1
        }
    }
    /// "You'll have no breaks." / "You'll have 1 break." / "2 breaks".
    pub fn breaks_text(self) -> String {
        match self.breaks() {
            0 => "You\u{2019}ll have no breaks.".into(),
            1 => "You\u{2019}ll have 1 break.".into(),
            n => format!("You\u{2019}ll have {n} breaks."),
        }
    }
    /// The (focus, break) periods in milliseconds, in order.
    pub fn periods(self) -> Vec<(bool, u64)> {
        let breaks = self.breaks();
        let focus_total = u64::from(self.minutes - breaks * BREAK_MINUTES) * 60_000;
        let parts = u64::from(breaks + 1);
        let mut out = Vec::new();
        for i in 0..parts {
            let share = focus_total / parts + u64::from(i < focus_total % parts);
            out.push((true, share));
            if i + 1 < parts {
                out.push((false, u64::from(BREAK_MINUTES) * 60_000));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stopwatch_laps_and_text() {
        let mut s = Stopwatch::default();
        s.toggle(1000);
        s.lap(2500);
        s.lap(4000);
        s.toggle(5000);
        assert_eq!(s.elapsed(9000), 4000);
        assert_eq!(s.lap_rows(), alloc::vec![(2, 1500, 3000), (1, 1500, 1500)]);
        assert_eq!(stopwatch_text(3_723_450), "01:02:03.45");
        s.reset();
        assert_eq!(s.elapsed(10_000), 0);
    }
    #[test]
    fn timers_count_down_pause_and_restart() {
        let mut t = Timer::new("Timer (1)", 60_000);
        assert_eq!(timer_text(t.remaining(0)), "00:01:00");
        t.toggle(0);
        assert_eq!(timer_text(t.remaining(30_500)), "00:00:30");
        t.toggle(30_500);
        assert_eq!(t.remaining(99_000), 29_500);
        t.toggle(100_000);
        assert!(t.done(130_000) && t.permille(130_000) == 0);
        t.toggle(140_000);
        assert_eq!(t.remaining(140_000), 60_000);
    }
    #[test]
    fn alarms_ring_once_on_their_days() {
        let mut a = Alarm::new("Good morning", 7, 0);
        assert_eq!(
            (a.time_text().as_str(), a.days_text().as_str()),
            ("7:00 AM", "Only once")
        );
        assert!(!a.due(1, 1, 7, 0));
        a.enabled = true;
        assert_eq!(a.minutes_until(1, 21, 5), Some(595));
        assert_eq!(rings_in(595), "Rings in 9 hours, 55 minutes");
        assert!(a.due(1, 1, 7, 0));
        assert!(!a.enabled, "one-time alarms switch off");
        let mut w = Alarm::new("Work", 8, 30);
        w.enabled = true;
        w.days = 0x3e;
        assert_eq!(w.days_text(), "Weekdays");
        assert!(!w.due(3, 0, 8, 30), "not on Sunday");
        assert!(w.due(4, 1, 8, 30));
        assert!(!w.due(4, 1, 8, 30), "rings once per minute");
        assert_eq!(clock_text(12, 5), "12:05 PM");
        assert_eq!(clock_text(0, 0), "12:00 AM");
    }
    #[test]
    fn focus_sessions_add_breaks_like_windows() {
        let p = |m| FocusPlan {
            minutes: m,
            skip_breaks: false,
        };
        assert_eq!(
            (
                p(30).breaks(),
                p(45).breaks(),
                p(60).breaks(),
                p(90).breaks()
            ),
            (0, 1, 1, 2)
        );
        assert_eq!(
            p(60).periods(),
            alloc::vec![(true, 1_650_000), (false, 300_000), (true, 1_650_000)]
        );
        assert_eq!(
            FocusPlan {
                minutes: 60,
                skip_breaks: true
            }
            .breaks(),
            0
        );
        assert_eq!(p(30).breaks_text(), "You\u{2019}ll have no breaks.");
    }
}
