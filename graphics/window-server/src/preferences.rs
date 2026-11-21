//! The P14 per-user preference words (`door_service_protocol::preferences`)
//! as the window server keeps and applies them: Init sends every word at
//! session binding and Settings sends a word when the user changes it
//! (`window::SET_PREFERENCE`). Words the server does not consume
//! (notifications, Do not disturb, Focus) are stored for the shell parts
//! that read them. Pure policy here; `authority.rs` and `desktop.rs` apply
//! it to the live shell.
use door_service_protocol::preferences::{self as p, LockScreen, ThemeSchedule};

/// Preference ids 10..=20.
pub const FIRST: u32 = 10;
pub const COUNT: usize = p::ALL.len();
const _: () = assert!(p::ALL[0].id == FIRST && p::ALL[COUNT - 1].id == FIRST + COUNT as u32 - 1);

/// Every preference word, starting at the clean-Windows defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Words([u32; COUNT]);
impl Default for Words {
    fn default() -> Self {
        Self(core::array::from_fn(|i| p::ALL[i].default))
    }
}
impl Words {
    /// The current word `id` (its default for unknown ids).
    pub fn get(&self, id: u32) -> u32 {
        id.checked_sub(FIRST)
            .and_then(|i| self.0.get(i as usize).copied())
            .unwrap_or_else(|| p::default(id))
    }
    /// Store a validated word. None for unknown ids and invalid words;
    /// Some(true) when the word changed.
    pub fn set(&mut self, id: u32, value: u32) -> Option<bool> {
        if !p::valid(id, value) {
            return None;
        }
        let slot = &mut self.0[(id - FIRST) as usize];
        let changed = *slot != value;
        *slot = value;
        Some(changed)
    }
    /// "Show accent colour on Start and taskbar". Windows applies it only in
    /// dark mode (light mode keeps the word and shows nothing), and never
    /// under a contrast theme.
    pub fn accent_on_start(&self, dark: bool, contrast: bool) -> bool {
        dark && !contrast && self.get(p::ACCENT_SURFACES) & p::ACCENT_START_TASKBAR != 0
    }
    /// "Show accent colour on title bars and window borders" (not under a
    /// contrast theme, whose system colours own the frame).
    pub fn accent_on_title_bars(&self, contrast: bool) -> bool {
        !contrast && self.get(p::ACCENT_SURFACES) & p::ACCENT_TITLE_BARS != 0
    }
    pub fn theme_schedule(&self) -> ThemeSchedule {
        ThemeSchedule::from_bits(self.get(p::THEME_SCHEDULE)).unwrap_or(ThemeSchedule::DEFAULT)
    }
    /// Minutes without input before the session locks; 0 = Never.
    pub fn sign_in_away(&self) -> u32 {
        self.get(p::SIGN_IN_AWAY)
    }
    pub fn lock_screen(&self) -> LockScreen {
        LockScreen::from_bits(self.get(p::LOCK_SCREEN)).unwrap_or(LockScreen::DESKTOP)
    }
    pub fn regional_overrides(&self) -> u32 {
        self.get(p::REGIONAL_OVERRIDES)
    }
    pub fn clipboard_history(&self) -> bool {
        self.get(p::CLIPBOARD) & p::CLIPBOARD_HISTORY != 0
    }
    pub fn search_history(&self) -> bool {
        self.get(p::SEARCH_HISTORY) & p::SEARCH_HISTORY_ON != 0
    }
    pub fn end_task(&self) -> bool {
        self.get(p::END_TASK) & p::END_TASK_ON != 0
    }
}

/// The accent shade Start, the taskbar and the tray flyouts tint with when
/// "Show accent colour on Start and taskbar" applies (Windows: the accent's
/// Dark2), as opaque BGRA.
pub fn start_tint(accent_rgb: u32) -> [u8; 4] {
    let c = door_design_tokens::accent::palette(accent_rgb).dark2();
    let rgb = door_design_tokens::accent::rgb24(c);
    [rgb as u8, (rgb >> 8) as u8, (rgb >> 16) as u8, 255]
}
/// The accent title bars and window borders paint with: the accent itself
/// (Windows uses the accent colour unchanged), as opaque BGRA.
pub fn title_accent(accent_rgb: u32) -> [u8; 4] {
    [
        accent_rgb as u8,
        (accent_rgb >> 8) as u8,
        (accent_rgb >> 16) as u8,
        255,
    ]
}

/// Personalization > Colors: switch Light/Dark at the scheduled local
/// times. The mode due now is applied when the schedule is turned on (or
/// bound at sign-in) and again at each boundary; a manual change in between
/// holds until the next boundary, as Auto Dark Mode does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Schedule {
    /// Mode due at the last evaluation (None: re-apply the next one).
    due: Option<bool>,
}
impl Schedule {
    /// Apply the mode due now at the next `tick` (binding, word changed).
    pub fn reset(&mut self) {
        self.due = None;
    }
    /// `minute` is the local minute of the day (None: the clock is not
    /// read yet). Returns the mode to switch to (true = dark) when one is
    /// due now; nothing while the schedule is off.
    pub fn tick(&mut self, schedule: ThemeSchedule, minute: Option<u32>) -> Option<bool> {
        if !schedule.enabled {
            self.due = None;
            return None;
        }
        let dark = schedule.dark_at(minute?);
        (self.due.replace(dark) != Some(dark)).then_some(dark)
    }
}
/// Local minute of the day of a wall-clock time.
pub fn minute_of_day(time: door_abi::clock::WallClock) -> u32 {
    u32::from(time.hour) * 60 + u32::from(time.minute)
}

/// Accounts > Sign-in options > "If you've been away": the session locks
/// after this many minutes without input (0 = Never).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Idle {
    last_input: u64,
}
pub const TICKS_PER_MINUTE: u64 = 60_000 / door_abi::platform::MONOTONIC_TICK_MILLIS;
impl Idle {
    /// Keyboard or pointer input (or a session event that counts as one:
    /// sign-in, unlock) at tick `now`.
    pub fn input(&mut self, now: u64) {
        self.last_input = self.last_input.max(now);
    }
    /// Lock now: signed in, unlocked, a timeout set and that long idle.
    pub fn due(&self, minutes: u32, now: u64, signed_in: bool, locked: bool) -> bool {
        signed_in
            && !locked
            && minutes != 0
            && now.saturating_sub(self.last_input) >= u64::from(minutes) * TICKS_PER_MINUTE
    }
}

/// What the lock screen shows behind the sign-in card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockBackdrop {
    /// The desktop background (Doors' original lock screen).
    Desktop,
    /// One of the four Doors pictures (`wallpaper::Pack::background`).
    Picture(u32),
    /// A solid colour (opaque BGRA).
    Solid([u8; 4]),
}
/// The lock screen for the `LOCK_SCREEN` word while the desktop shows
/// picture `background`: the same picture reuses the desktop's pixels.
pub fn lock_backdrop(word: LockScreen, background: u32) -> LockBackdrop {
    match word.kind() {
        1..=4 if word.kind() - 1 != background => LockBackdrop::Picture(word.kind() - 1),
        LockScreen::SOLID => {
            let rgb = word.rgb();
            LockBackdrop::Solid([rgb as u8, (rgb >> 8) as u8, (rgb >> 16) as u8, 255])
        }
        _ => LockBackdrop::Desktop,
    }
}

/// Bluetooth & devices > Mouse: a relative motion scaled by the pointer
/// speed (10 is 1:1, 1 is 1/10, 20 is 2x) and, with "Enhance pointer
/// precision", accelerated for fast movements. `rest` keeps sub-pixel
/// remainders (1/256 px) so slow speeds still move.
pub fn pointer_motion(word: u32, dx: i32, dy: i32, rest: &mut (i32, i32)) -> (i32, i32) {
    use door_service_protocol::preferences::mouse as m;
    let mut factor = m::speed(word).clamp(1, 20) as i32 * 256 / 10;
    if word & m::NO_PRECISION == 0 {
        let fast = dx.abs().max(dy.abs());
        factor = factor
            * match fast {
                0..=2 => 4,
                3..=8 => 5,
                _ => 7,
            }
            / 5;
    }
    let x = dx * factor + rest.0;
    let y = dy * factor + rest.1;
    let out = (x.div_euclid(256), y.div_euclid(256));
    *rest = (x.rem_euclid(256), y.rem_euclid(256));
    out
}
/// "Primary mouse button": Right swaps the first two buttons.
pub fn mouse_buttons(word: u32, flags: u32) -> u32 {
    if word & door_service_protocol::preferences::mouse::SWAP_BUTTONS == 0 {
        return flags;
    }
    (flags & !3) | (flags & 1) << 1 | (flags & 2) >> 1
}
/// "Lines to scroll at a time": apps scroll three lines per notch, so a
/// notch becomes lines / 3 notches (thirds carried in `rest`).
pub fn wheel_notches(word: u32, notches: i32, rest: &mut i32) -> i32 {
    let lines = door_service_protocol::preferences::mouse::lines(word).clamp(1, 20) as i32;
    let thirds = notches * lines + *rest;
    let out = thirds / 3;
    *rest = thirds - out * 3;
    out
}

/// Search home Recent list rows: (recent searches, recent files) within
/// `rows`. Searches come first; when both have more than fit, files keep
/// half the rows (rounded down) and searches take the rest.
pub fn recent_rows(searches: usize, files: usize, rows: usize) -> (usize, usize) {
    let searches = searches.min(rows - files.min(rows / 2));
    (searches, files.min(rows - searches))
}

#[cfg(test)]
mod tests {
    use super::*;
    use door_abi::clock::WallClock;
    #[test]
    fn words_start_at_windows_defaults_and_validate() {
        let mut w = Words::default();
        for pref in p::ALL {
            assert_eq!(w.get(pref.id), pref.default, "{}", pref.name);
        }
        assert!(!w.accent_on_start(true, false) && !w.accent_on_title_bars(false));
        assert!(!w.theme_schedule().enabled);
        assert_eq!(w.sign_in_away(), 0, "Never");
        assert_eq!(w.lock_screen(), LockScreen::DESKTOP);
        assert_eq!(w.regional_overrides(), 0);
        assert!(!w.clipboard_history() && w.search_history() && !w.end_task());
        assert_eq!(w.set(p::END_TASK, 1), Some(true));
        assert_eq!(w.set(p::END_TASK, 1), Some(false), "unchanged");
        assert!(w.end_task());
        assert_eq!(w.set(p::END_TASK, 2), None);
        assert_eq!(w.set(p::SIGN_IN_AWAY, 2), None, "not a Windows choice");
        assert_eq!(w.set(9, 0), None);
        assert_eq!(w.set(p::LAST + 1, 0), None);
        assert!(w.end_task(), "a refused word leaves the old one");
        // Words the server does not consume are kept for the shell.
        assert_eq!(
            w.set(p::FOCUS, p::Focus::DEFAULT.bits() & !(1 << 11)),
            Some(true)
        );
        assert_eq!(w.get(p::FOCUS) & (1 << 11), 0);
    }
    #[test]
    fn accent_on_start_applies_only_in_dark_mode_and_title_bars_always() {
        let mut w = Words::default();
        w.set(p::ACCENT_SURFACES, p::ACCENT_START_TASKBAR).unwrap();
        assert!(w.accent_on_start(true, false));
        assert!(
            !w.accent_on_start(false, false),
            "light mode keeps the word only"
        );
        assert!(!w.accent_on_start(true, true), "contrast themes win");
        assert!(!w.accent_on_title_bars(false));
        w.set(p::ACCENT_SURFACES, p::ACCENT_TITLE_BARS).unwrap();
        assert!(w.accent_on_title_bars(false) && !w.accent_on_title_bars(true));
        assert!(!w.accent_on_start(true, false));
        // Default blue: title bars use it unchanged, Start its Dark2 shade.
        assert_eq!(title_accent(0x0078d4), [0xd4, 0x78, 0x00, 255]);
        let tint = start_tint(0x0078d4);
        assert!(tint[0] > tint[2], "still blue");
        assert!(
            u32::from(tint[0]) + u32::from(tint[1]) < 0xd4 + 0x78,
            "darker"
        );
    }
    #[test]
    fn schedule_applies_the_due_mode_at_binding_and_boundaries_only() {
        let on = ThemeSchedule {
            enabled: true,
            light_from: 7 * 60,
            dark_from: 19 * 60,
        };
        let mut s = Schedule::default();
        assert_eq!(s.tick(ThemeSchedule::DEFAULT, Some(23 * 60)), None, "off");
        assert_eq!(s.tick(on, None), None, "no clock yet");
        assert_eq!(s.tick(on, Some(12 * 60)), Some(false), "binding: light due");
        assert_eq!(s.tick(on, Some(12 * 60 + 1)), None);
        assert_eq!(s.tick(on, Some(18 * 60 + 59)), None);
        assert_eq!(s.tick(on, Some(19 * 60)), Some(true), "dark from 19:00");
        assert_eq!(s.tick(on, Some(2 * 60)), None, "still dark after midnight");
        assert_eq!(s.tick(on, Some(7 * 60)), Some(false), "light from 07:00");
        // Turning it off and on again (or a new word) re-applies at once.
        assert_eq!(s.tick(ThemeSchedule::DEFAULT, Some(8 * 60)), None);
        assert_eq!(s.tick(on, Some(20 * 60)), Some(true));
        s.reset();
        assert_eq!(s.tick(on, Some(20 * 60)), Some(true));
        let t = WallClock {
            year: 2026,
            month: 10,
            day: 31,
            hour: 19,
            minute: 5,
            second: 0,
        };
        assert_eq!(minute_of_day(t), 19 * 60 + 5);
    }
    #[test]
    fn idle_lock_waits_for_the_chosen_minutes_and_never_by_default() {
        let mut idle = Idle::default();
        idle.input(1_000);
        let minute = TICKS_PER_MINUTE;
        assert_eq!(minute, 6_000);
        assert!(!idle.due(0, 1_000 + 600 * minute, true, false), "Never");
        assert!(!idle.due(1, 1_000 + minute - 1, true, false));
        assert!(idle.due(1, 1_000 + minute, true, false));
        assert!(
            !idle.due(1, 1_000 + minute, false, false),
            "never while signed out"
        );
        assert!(!idle.due(1, 1_000 + minute, true, true), "already locked");
        idle.input(1_000 + minute);
        assert!(
            !idle.due(1, 1_000 + minute + 10, true, false),
            "input resets it"
        );
        idle.input(5);
        assert!(
            idle.due(1, 1_000 + 2 * minute, true, false),
            "time never runs back"
        );
    }
    #[test]
    fn lock_screen_word_picks_desktop_picture_or_solid_backdrop() {
        assert_eq!(lock_backdrop(LockScreen::DESKTOP, 2), LockBackdrop::Desktop);
        let picture = LockScreen::picture(1).unwrap();
        assert_eq!(lock_backdrop(picture, 0), LockBackdrop::Picture(1));
        assert_eq!(
            lock_backdrop(picture, 1),
            LockBackdrop::Desktop,
            "same pixels"
        );
        assert_eq!(
            lock_backdrop(LockScreen::solid(0x102030), 0),
            LockBackdrop::Solid([0x30, 0x20, 0x10, 255])
        );
    }
    #[test]
    fn recent_rows_share_the_search_home_list() {
        assert_eq!(recent_rows(0, 0, 8), (0, 0));
        assert_eq!(recent_rows(3, 2, 8), (3, 2));
        assert_eq!(recent_rows(10, 0, 8), (8, 0));
        assert_eq!(recent_rows(0, 6, 8), (0, 6));
        assert_eq!(recent_rows(10, 6, 8), (4, 4));
        assert_eq!(recent_rows(2, 6, 8), (2, 6));
        assert_eq!(recent_rows(10, 2, 8), (6, 2));
        assert_eq!(recent_rows(5, 5, 3), (2, 1));
    }
}
#[cfg(test)]
mod mouse_tests {
    use super::*;
    use door_service_protocol::preferences::mouse as m;
    #[test]
    fn mouse_words_scale_motion_swap_buttons_and_wheel_lines() {
        let mut rest = (0, 0);
        // Precision on (speed 10): slow motion slows, fast motion accelerates.
        let precise = m::DEFAULT & !m::NO_PRECISION;
        assert_eq!(pointer_motion(precise, 1, -2, &mut rest), (0, -2));
        let mut rest = (0, 0);
        assert_eq!(pointer_motion(precise, 20, 0, &mut rest).0, 27);
        // The default is exactly 1:1.
        let mut rest = (0, 0);
        assert_eq!(pointer_motion(m::DEFAULT, 7, -3, &mut rest), (7, -3));
        // Precision off: exactly 1:1 at speed 10, half at speed 5.
        let plain = m::DEFAULT | m::NO_PRECISION;
        let mut rest = (0, 0);
        assert_eq!(pointer_motion(plain, 20, 10, &mut rest), (20, 10));
        let half = (plain & !m::SPEED_MASK) | 5;
        let mut rest = (0, 0);
        let moved: i32 = (0..10)
            .map(|_| pointer_motion(half, 1, 0, &mut rest).0)
            .sum();
        assert_eq!(moved, 5, "sub-pixel remainders add up");
        assert_eq!(mouse_buttons(m::DEFAULT, 1), 1);
        assert_eq!(mouse_buttons(m::DEFAULT | m::SWAP_BUTTONS, 1), 2);
        assert_eq!(mouse_buttons(m::DEFAULT | m::SWAP_BUTTONS, 6), 5);
        let mut rest = 0;
        assert_eq!(
            wheel_notches(m::DEFAULT, 2, &mut rest),
            2,
            "3 lines: unchanged"
        );
        let six = (m::DEFAULT & !m::LINES_MASK) | 6 << m::LINES_SHIFT;
        assert_eq!(wheel_notches(six, 1, &mut rest), 2);
        let one = (m::DEFAULT & !m::LINES_MASK) | 1 << m::LINES_SHIFT;
        let total: i32 = (0..3).map(|_| wheel_notches(one, 1, &mut rest)).sum();
        assert_eq!(total, 1);
    }
}
