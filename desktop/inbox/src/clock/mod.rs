//! Clock's window, measured from Windows 11 26H2 Clock at 150%: a 32 px title
//! bar over a 280 px navigation pane (Focus sessions, Timer, Alarm,
//! Stopwatch, World clock; Settings at the bottom; 36 px rows on a 40 px
//! pitch, a 3 px accent pill on the selected row) and the content layer.
//! Every page runs on real time: the runtime's monotonic tick for focus
//! sessions, timers and the stopwatch, the RTC wall time in the user's time
//! zone for alarms and the world clock; finished timers and ringing alarms
//! post toasts.
pub mod model;
use alloc::{format, string::String, vec::Vec};
use door_app_ui::{Application, View};
use door_graphics_text::{FontAtlas, TextStyle};
use door_service_client::{Error, Storage};
pub use model::*;

pub const TITLE: i32 = 32;
pub const PANE: u32 = 280;
const ROW_TOP: i32 = 40;
const ROW_PITCH: i32 = 40;
const ROW_H: u32 = 36;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Focus,
    Timer,
    Alarm,
    Stopwatch,
    WorldClock,
    Settings,
}
impl Page {
    pub const NAV: [Self; 5] = [
        Self::Focus,
        Self::Timer,
        Self::Alarm,
        Self::Stopwatch,
        Self::WorldClock,
    ];
    pub const fn label(self) -> &'static str {
        match self {
            Self::Focus => "Focus sessions",
            Self::Timer => "Timer",
            Self::Alarm => "Alarm",
            Self::Stopwatch => "Stopwatch",
            Self::WorldClock => "World clock",
            Self::Settings => "Settings",
        }
    }
    const fn glyph(self) -> &'static str {
        match self {
            Self::Focus => "clock-focus",
            Self::Timer => "clock-timer",
            Self::Alarm => "clock-alarm",
            Self::Stopwatch => "clock-stopwatch",
            Self::WorldClock => "clock-world",
            Self::Settings => "clock-settings",
        }
    }
}

mod id {
    pub const NAV: u32 = 1; // 1..=5
    pub const SETTINGS: u32 = 9;
    pub const FOCUS_START: u32 = 20;
    pub const FOCUS_UP: u32 = 21;
    pub const FOCUS_DOWN: u32 = 22;
    pub const FOCUS_SKIP: u32 = 23;
    pub const FOCUS_STOP: u32 = 24;
    pub const FOCUS_PAUSE: u32 = 25;
    pub const ADD: u32 = 30;
    pub const STOPWATCH_TOGGLE: u32 = 40;
    pub const STOPWATCH_LAP: u32 = 41;
    pub const STOPWATCH_RESET: u32 = 42;
    pub const THEME: u32 = 45; // 45..=47 light, dark, system
    /// Timer i: 100 + 4i + (toggle, reset, delete).
    pub const TIMER: u32 = 100;
    /// Alarm i: 300 + 4i + (switch, delete).
    pub const ALARM: u32 = 300;
    /// World clock i: 500 + 2i + (delete).
    pub const CITY: u32 = 500;
    /// Editor (add timer/alarm/city) controls.
    pub const EDIT: u32 = 700; // 700..=720 steppers, 730 save, 731 cancel, 740.. options
    pub const DISMISS: u32 = 800;
    pub const SNOOZE: u32 = 801;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}
impl Rect {
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }
    pub fn contains(self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w as i32 && py < self.y + self.h as i32
    }
    fn right(self) -> i32 {
        self.x + self.w as i32
    }
    fn bottom(self) -> i32 {
        self.y + self.h as i32
    }
}

/// The add/edit sheet: a new timer, alarm or world-clock city.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Editor {
    Timer { hours: u8, minutes: u8, seconds: u8 },
    Alarm { hour: u8, minute: u8, days: u8 },
    City { first: usize, chosen: Option<usize> },
}

/// A running focus session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub plan: FocusPlan,
    pub periods: Vec<(bool, u64)>,
    pub index: usize,
    pub timer: Timer,
}

pub struct Clock {
    pub width: u32,
    pub height: u32,
    pub page: Page,
    pub now: u64,
    /// Local wall time (y, mo, d, h, mi, s) and weekday, when the RTC is valid.
    pub local: Option<(door_abi::clock::WallClock, u8, u32)>,
    pub zone: u32,
    pub plan: FocusPlan,
    pub session: Option<Session>,
    pub focused_today: u64,
    pub stopwatch: Stopwatch,
    pub timers: Vec<Timer>,
    pub alarms: Vec<Alarm>,
    pub cities: Vec<u16>,
    pub editor: Option<Editor>,
    pub ringing: Option<usize>,
    pub theme: u8,
    /// Toasts to post (title, body); the runtime drains them in `tick`.
    pub toasts: Vec<(String, String)>,
    registry: u64,
    /// door.audio while an alarm rings, and when its sound plays next.
    alarm_audio: Option<door_service_client::audio::Audio>,
    alarm_at: u64,
    ready: bool,
}

impl Default for Clock {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Clock {
    pub fn new(registry: u64) -> Self {
        Self {
            width: 1000,
            height: 680,
            page: Page::Timer,
            now: 0,
            local: None,
            zone: 0,
            plan: FocusPlan {
                minutes: 30,
                skip_breaks: false,
            },
            session: None,
            focused_today: 0,
            stopwatch: Stopwatch::default(),
            // Windows' default timers and alarm.
            timers: alloc::vec![
                Timer::new("1 min", 60_000),
                Timer::new("3 min", 180_000),
                Timer::new("5 min", 300_000),
                Timer::new("10 min", 600_000),
            ],
            alarms: alloc::vec![Alarm::new("Good morning", 7, 0)],
            cities: Vec::new(),
            editor: None,
            ringing: None,
            theme: 2,
            toasts: Vec::new(),
            registry,
            alarm_audio: None,
            alarm_at: 0,
            ready: false,
        }
    }
    fn content(&self) -> Rect {
        Rect::new(
            PANE as i32,
            TITLE,
            self.width.saturating_sub(PANE),
            self.height.saturating_sub(TITLE as u32),
        )
    }
    fn nav_rect(i: usize) -> Rect {
        Rect::new(4, ROW_TOP + i as i32 * ROW_PITCH, PANE - 8, ROW_H)
    }
    fn settings_rect(&self) -> Rect {
        Rect::new(4, self.height as i32 - 8 - ROW_H as i32, PANE - 8, ROW_H)
    }
    fn add_rect(&self) -> Rect {
        let c = self.content();
        Rect::new(c.right() - 24 - 48, c.bottom() - 24 - 48, 48, 48)
    }

    // ---- page geometry ----
    /// Timer cards: 300 x 320 on a flowing grid.
    fn card(&self, i: usize, w: u32, h: u32) -> Rect {
        let c = self.content();
        let gap = 12;
        let cols = ((c.w as i32 - 48 + gap) / (w as i32 + gap)).max(1);
        let (col, row) = (i as i32 % cols, i as i32 / cols);
        Rect::new(
            c.x + 24 + col * (w as i32 + gap),
            c.y + 72 + row * (h as i32 + gap),
            w,
            h,
        )
    }
    fn timer_card(&self, i: usize) -> Rect {
        self.card(i, 300, 320)
    }
    fn timer_button(&self, i: usize, k: usize) -> Rect {
        let r = self.timer_card(i);
        match k {
            0 => Rect::new(r.x + r.w as i32 / 2 - 52, r.bottom() - 56, 48, 40),
            1 => Rect::new(r.x + r.w as i32 / 2 + 4, r.bottom() - 56, 48, 40),
            _ => Rect::new(r.right() - 44, r.y + 8, 36, 32),
        }
    }
    fn alarm_card(&self, i: usize) -> Rect {
        self.card(i, 300, 200)
    }
    fn alarm_switch(&self, i: usize) -> Rect {
        let r = self.alarm_card(i);
        Rect::new(r.right() - 64, r.y + 16, 40, 20)
    }
    fn alarm_delete(&self, i: usize) -> Rect {
        let r = self.alarm_card(i);
        Rect::new(r.right() - 44, r.bottom() - 44, 36, 32)
    }
    fn city_card(&self, i: usize) -> Rect {
        let c = self.content();
        Rect::new(
            c.x + 24,
            c.y + 72 + i as i32 * 84,
            c.w.saturating_sub(48),
            76,
        )
    }
    fn city_delete(&self, i: usize) -> Rect {
        let r = self.city_card(i);
        Rect::new(r.right() - 48, r.y + 22, 36, 32)
    }
    fn stopwatch_buttons(&self) -> [Rect; 3] {
        let c = self.content();
        let cx = c.x + c.w as i32 / 2;
        let y = c.y + c.h as i32 / 2 + 24;
        [
            Rect::new(cx - 24, y, 48, 48),
            Rect::new(cx + 36, y, 48, 48),
            Rect::new(cx - 84, y, 48, 48),
        ]
    }
    fn focus_card(&self) -> Rect {
        let c = self.content();
        let w = c.w.saturating_sub(48).min(520);
        Rect::new(c.x + 24, c.y + 72, w, 360)
    }
    fn focus_controls(&self) -> [Rect; 5] {
        let f = self.focus_card();
        let cx = f.x + f.w as i32 / 2;
        [
            Rect::new(cx - 100, f.bottom() - 64, 200, 32), // start / stop
            Rect::new(cx + 64, f.y + 110, 40, 40),         // up
            Rect::new(cx + 64, f.y + 160, 40, 40),         // down
            Rect::new(cx - 70, f.y + 264, 140, 24),        // skip breaks
            Rect::new(cx - 100, f.bottom() - 112, 200, 32), // pause
        ]
    }
    fn editor_sheet(&self) -> Rect {
        let w = 400.min(self.width.saturating_sub(32));
        let h = 460.min(self.height.saturating_sub(64));
        Rect::new(
            (self.width as i32 - w as i32) / 2,
            (self.height as i32 - h as i32) / 2,
            w,
            h,
        )
    }
    fn editor_buttons(&self) -> [Rect; 2] {
        let s = self.editor_sheet();
        let w = (s.w - 24 - 8) / 2;
        [
            Rect::new(s.x + 12, s.bottom() - 48, w, 32),
            Rect::new(s.x + 12 + w as i32 + 8, s.bottom() - 48, w, 32),
        ]
    }
    /// Editor steppers: (up, down) for field k (hours/minutes/seconds or
    /// hour/minute/AM-PM).
    fn stepper(&self, k: usize) -> (Rect, Rect) {
        let s = self.editor_sheet();
        let x = s.x + s.w as i32 / 2 - 150 + k as i32 * 104;
        (
            Rect::new(x + 8, s.y + 64, 80, 32),
            Rect::new(x + 8, s.y + 176, 80, 32),
        )
    }
    fn day_chip(&self, d: usize) -> Rect {
        let s = self.editor_sheet();
        Rect::new(s.x + 24 + d as i32 * 50, s.y + 240, 44, 32)
    }
    fn zone_row(&self, i: usize) -> Rect {
        let s = self.editor_sheet();
        Rect::new(s.x + 12, s.y + 56 + i as i32 * 36, s.w - 24, 34)
    }
    fn zone_rows(&self) -> usize {
        ((self.editor_sheet().h as i32 - 56 - 64) / 36).max(0) as usize
    }
    fn ring_buttons(&self) -> [Rect; 2] {
        let s = self.editor_sheet();
        [
            Rect::new(s.x + 24, s.bottom() - 56, 170, 32),
            Rect::new(s.right() - 194, s.bottom() - 56, 170, 32),
        ]
    }

    pub fn hits(&self) -> Vec<(Rect, u32)> {
        let mut out = Vec::new();
        if self.ringing.is_some() {
            let [dismiss, snooze] = self.ring_buttons();
            out.push((dismiss, id::DISMISS));
            out.push((snooze, id::SNOOZE));
            out.push((Rect::new(0, 0, self.width, self.height), u32::MAX));
            return out;
        }
        if let Some(e) = &self.editor {
            let [save, cancel] = self.editor_buttons();
            out.push((save, id::EDIT + 30));
            out.push((cancel, id::EDIT + 31));
            match e {
                Editor::City { first, .. } => {
                    for i in 0..self.zone_rows() {
                        if first + i < door_locale::tz::ZONES.len() {
                            out.push((self.zone_row(i), id::EDIT + 40 + i as u32));
                        }
                    }
                }
                _ => {
                    for k in 0..3 {
                        let (up, down) = self.stepper(k);
                        out.push((up, id::EDIT + 2 * k as u32));
                        out.push((down, id::EDIT + 2 * k as u32 + 1));
                    }
                    if matches!(e, Editor::Alarm { .. }) {
                        for d in 0..7 {
                            out.push((self.day_chip(d), id::EDIT + 10 + d as u32));
                        }
                    }
                }
            }
            out.push((Rect::new(0, 0, self.width, self.height), u32::MAX));
            return out;
        }
        for i in 0..Page::NAV.len() {
            out.push((Self::nav_rect(i), id::NAV + i as u32));
        }
        out.push((self.settings_rect(), id::SETTINGS));
        match self.page {
            Page::Focus => {
                let [start, up, down, skip, pause] = self.focus_controls();
                out.push((
                    start,
                    if self.session.is_some() {
                        id::FOCUS_STOP
                    } else {
                        id::FOCUS_START
                    },
                ));
                if self.session.is_some() {
                    out.push((pause, id::FOCUS_PAUSE));
                } else {
                    out.push((up, id::FOCUS_UP));
                    out.push((down, id::FOCUS_DOWN));
                    out.push((skip, id::FOCUS_SKIP));
                }
            }
            Page::Timer => {
                for i in 0..self.timers.len() {
                    for k in 0..3 {
                        out.push((self.timer_button(i, k), id::TIMER + 4 * i as u32 + k as u32));
                    }
                }
                out.push((self.add_rect(), id::ADD));
            }
            Page::Alarm => {
                for i in 0..self.alarms.len() {
                    out.push((self.alarm_switch(i), id::ALARM + 4 * i as u32));
                    out.push((self.alarm_delete(i), id::ALARM + 4 * i as u32 + 1));
                }
                out.push((self.add_rect(), id::ADD));
            }
            Page::Stopwatch => {
                let [toggle, lap, reset] = self.stopwatch_buttons();
                out.push((toggle, id::STOPWATCH_TOGGLE));
                out.push((lap, id::STOPWATCH_LAP));
                out.push((reset, id::STOPWATCH_RESET));
            }
            Page::WorldClock => {
                for i in 0..self.cities.len() {
                    out.push((self.city_delete(i + 1), id::CITY + 2 * i as u32));
                }
                out.push((self.add_rect(), id::ADD));
            }
            Page::Settings => {
                let c = self.content();
                for t in 0..3 {
                    out.push((
                        Rect::new(c.x + 40, c.y + 120 + t * 32, 200, 28),
                        id::THEME + t as u32,
                    ));
                }
            }
        }
        out
    }

    pub fn activate(&mut self, target: u32) -> bool {
        let now = self.now;
        match target {
            t if (id::NAV..id::NAV + 5).contains(&t) => {
                self.page = Page::NAV[(t - id::NAV) as usize];
                // The guest scenarios read the page and stopwatch state.
                door_rt::diagnostic(format_args!(
                    "DOOR_INBOX CLOCK page={}\n",
                    self.page.label()
                ));
            }
            id::SETTINGS => self.page = Page::Settings,
            id::FOCUS_UP => {
                self.plan.minutes =
                    (self.plan.minutes + if self.plan.minutes < 60 { 5 } else { 15 }).min(240)
            }
            id::FOCUS_DOWN => {
                self.plan.minutes = self
                    .plan
                    .minutes
                    .saturating_sub(if self.plan.minutes <= 60 { 5 } else { 15 })
                    .max(5)
            }
            id::FOCUS_SKIP => self.plan.skip_breaks = !self.plan.skip_breaks,
            id::FOCUS_START => {
                let periods = self.plan.periods();
                let mut timer = Timer::new("Focus", periods[0].1);
                timer.toggle(now);
                self.session = Some(Session {
                    plan: self.plan,
                    periods,
                    index: 0,
                    timer,
                });
            }
            id::FOCUS_STOP => self.session = None,
            id::FOCUS_PAUSE => {
                if let Some(s) = &mut self.session {
                    s.timer.toggle(now);
                }
            }
            id::ADD => {
                self.editor = Some(match self.page {
                    Page::Timer => Editor::Timer {
                        hours: 0,
                        minutes: 15,
                        seconds: 0,
                    },
                    Page::Alarm => Editor::Alarm {
                        hour: 7,
                        minute: 0,
                        days: 0,
                    },
                    _ => Editor::City {
                        first: 0,
                        chosen: None,
                    },
                })
            }
            id::STOPWATCH_TOGGLE => {
                self.stopwatch.toggle(now);
                door_rt::diagnostic(format_args!(
                    "DOOR_INBOX CLOCK stopwatch running={}\n",
                    u8::from(self.stopwatch.running())
                ));
            }
            id::STOPWATCH_LAP => self.stopwatch.lap(now),
            id::STOPWATCH_RESET => self.stopwatch.reset(),
            t if (id::THEME..id::THEME + 3).contains(&t) => self.theme = (t - id::THEME) as u8,
            t if (id::TIMER..id::TIMER + 400).contains(&t) => {
                let (i, k) = (((t - id::TIMER) / 4) as usize, (t - id::TIMER) % 4);
                match k {
                    0 => self.timers[i].toggle(now),
                    1 => self.timers[i].reset(),
                    _ => {
                        self.timers.remove(i);
                    }
                }
            }
            t if (id::ALARM..id::ALARM + 200).contains(&t) => {
                let (i, k) = (((t - id::ALARM) / 4) as usize, (t - id::ALARM) % 4);
                match k {
                    0 => {
                        self.alarms[i].enabled = !self.alarms[i].enabled;
                        self.alarms[i].rang = None;
                    }
                    _ => {
                        self.alarms.remove(i);
                    }
                }
            }
            t if (id::CITY..id::CITY + 200).contains(&t) => {
                self.cities.remove(((t - id::CITY) / 2) as usize);
            }
            t if (id::EDIT..id::EDIT + 50).contains(&t) => self.edit(t - id::EDIT),
            id::DISMISS => self.ringing = None,
            id::SNOOZE => {
                // Snooze: a one-time copy ten minutes later.
                if let (Some(i), Some((w, _, _))) = (self.ringing.take(), self.local) {
                    let total = u16::from(w.hour) * 60 + u16::from(w.minute) + 10;
                    let mut a = Alarm::new(
                        &self.alarms.get(i).map_or(String::new(), |a| a.name.clone()),
                        (total / 60 % 24) as u8,
                        (total % 60) as u8,
                    );
                    a.enabled = true;
                    self.alarms.push(a);
                }
            }
            _ => return false,
        }
        true
    }
    fn edit(&mut self, k: u32) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };
        match (editor, k) {
            (_, 31) => self.editor = None,
            (
                Editor::Timer {
                    hours,
                    minutes,
                    seconds,
                },
                30,
            ) => {
                let ms =
                    (u64::from(*hours) * 3600 + u64::from(*minutes) * 60 + u64::from(*seconds))
                        * 1000;
                if ms > 0 && self.timers.len() < 24 {
                    let name = format!("Timer ({})", self.timers.len() + 1);
                    self.timers.push(Timer::new(&name, ms));
                }
                self.editor = None;
            }
            (Editor::Alarm { hour, minute, days }, 30) => {
                if self.alarms.len() < 24 {
                    let mut a = Alarm::new("Alarm", *hour, *minute);
                    a.days = *days;
                    a.enabled = true;
                    self.alarms.push(a);
                }
                self.editor = None;
            }
            (Editor::City { chosen, .. }, 30) => {
                if let Some(i) = *chosen {
                    let id = door_locale::tz::ZONES[i].id;
                    if !self.cities.contains(&id) && self.cities.len() < 16 {
                        self.cities.push(id);
                    }
                }
                self.editor = None;
            }
            (Editor::City { first, chosen }, k) if k >= 40 => {
                *chosen = Some(*first + (k - 40) as usize)
            }
            (
                Editor::Timer {
                    hours,
                    minutes,
                    seconds,
                },
                k,
            ) if k < 6 => {
                let (field, limit) = match k / 2 {
                    0 => (hours, 99),
                    1 => (minutes, 59),
                    _ => (seconds, 59),
                };
                *field = if k % 2 == 0 {
                    if *field >= limit { 0 } else { *field + 1 }
                } else if *field == 0 {
                    limit
                } else {
                    *field - 1
                };
            }
            (Editor::Alarm { hour, minute, .. }, k) if k < 6 => match k {
                0 => *hour = (*hour % 12 + 1) % 12 + if *hour >= 12 { 12 } else { 0 },
                1 => *hour = (*hour % 12 + 11) % 12 + if *hour >= 12 { 12 } else { 0 },
                2 => *minute = (*minute + 1) % 60,
                3 => *minute = (*minute + 59) % 60,
                _ => *hour = (*hour + 12) % 24,
            },
            (Editor::Alarm { days, .. }, k) if (10..17).contains(&k) => *days ^= 1 << (k - 10),
            _ => (),
        }
    }

    /// Advances real-time state: finished timers and focus periods, due
    /// alarms. Returns whether anything visible changed.
    pub fn advance(
        &mut self,
        now: u64,
        local: Option<(door_abi::clock::WallClock, u8, u32)>,
    ) -> bool {
        self.now = now;
        let minute_changed =
            local.map(|l| (l.0.hour, l.0.minute)) != self.local.map(|l| (l.0.hour, l.0.minute));
        self.local = local;
        for t in &mut self.timers {
            if t.running() && t.done(now) && !t.announced {
                t.announced = true;
                self.toasts
                    .push(("Timer".into(), format!("{} is done", t.name)));
            }
        }
        if let Some(s) = &mut self.session
            && s.timer.done(now)
        {
            let (focus, ms) = s.periods[s.index];
            if focus {
                self.focused_today += ms;
            }
            if s.index + 1 < s.periods.len() {
                s.index += 1;
                let (focus, ms) = s.periods[s.index];
                s.timer = Timer::new(if focus { "Focus" } else { "Break" }, ms);
                s.timer.toggle(now);
                self.toasts.push((
                    "Focus sessions".into(),
                    if focus {
                        "Break is over. Time to focus.".into()
                    } else {
                        "Time for a break.".into()
                    },
                ));
            } else {
                self.session = None;
                self.toasts.push((
                    "Focus sessions".into(),
                    "Focus session complete. Nice work!".into(),
                ));
            }
        }
        if let Some((w, weekday, day)) = local
            && minute_changed
        {
            for (i, a) in self.alarms.iter_mut().enumerate() {
                if a.due(day, weekday, w.hour, w.minute) {
                    self.ringing = Some(i);
                    self.toasts.push((a.name.clone(), a.time_text()));
                }
            }
        }
        true
    }

    // ---- painting ----
    pub fn paint(&self, v: &mut View<'_>) {
        let dark = match self.theme {
            0 => false,
            1 => true,
            _ => v.palette.dark,
        };
        v.palette.dark = dark;
        let k = ink(dark);
        if v.palette.system().is_some() {
            v.rect(0, 0, v.width, v.height, k.mica);
        } else {
            v.material_rect(0, 0, v.width, v.height, k.mica, 220);
        }
        let c = self.content();
        // The content layer: lighter, rounded top-left, hairline edge.
        v.rounded_asset(c.x, c.y, c.w + 8, c.h + 8, 8, k.layer_edge);
        v.rounded_asset(c.x + 1, c.y + 1, c.w + 8, c.h + 8, 8, k.layer);
        if let Some(px) = super::icon_frames(super::App::Clock, v.scale) {
            v.color_pixels(16, 8, 16, px);
        }
        v.text_clip_centered(
            46,
            (0, TITLE as u32),
            200,
            "Clock",
            TextStyle::Regular12,
            k.text,
        );
        for (i, page) in Page::NAV.iter().enumerate() {
            self.nav_row(v, &k, Self::nav_rect(i), *page);
        }
        self.nav_row(v, &k, self.settings_rect(), Page::Settings);
        match self.page {
            Page::Focus => self.paint_focus(v, &k),
            Page::Timer => self.paint_timers(v, &k),
            Page::Alarm => self.paint_alarms(v, &k),
            Page::Stopwatch => self.paint_stopwatch(v, &k),
            Page::WorldClock => self.paint_world(v, &k),
            Page::Settings => self.paint_settings(v, &k),
        }
        if self.editor.is_some() {
            self.paint_editor(v, &k);
        }
        if self.ringing.is_some() {
            self.paint_ringing(v, &k);
        }
    }
    fn nav_row(&self, v: &mut View<'_>, k: &Ink, r: Rect, page: Page) {
        let selected = self.page == page;
        let hover = v.hover.is_some_and(|(x, y)| r.contains(x, y));
        if selected || hover {
            v.rounded_asset(
                r.x,
                r.y,
                r.w,
                r.h,
                4,
                if selected { k.selected } else { k.subtle },
            );
        }
        if selected {
            v.rounded_asset(r.x + 1, r.y + 10, 3, 16, 1, k.accent);
        }
        super::glyph(v, page.glyph(), r.x + 12, r.y + 10, 16, k.text);
        v.text_clip_centered(
            r.x + 44,
            (r.y, r.h),
            r.w - 52,
            page.label(),
            TextStyle::Regular14,
            k.text,
        );
    }
    fn header(&self, v: &mut View<'_>, k: &Ink, title: &str) {
        let c = self.content();
        v.text_clip_centered(
            c.x + 24,
            (c.y + 16, 40),
            c.w.saturating_sub(48),
            title,
            TextStyle::Semibold28,
            k.text,
        );
    }
    fn add_button(&self, v: &mut View<'_>, k: &Ink) {
        let r = self.add_rect();
        v.elevation(r.x, r.y + 2, r.w, r.h, 40);
        v.rounded_asset(r.x, r.y, r.w, r.h, 24, k.accent);
        super::glyph(v, "calc-add", r.x + 16, r.y + 16, 16, k.on_accent);
    }
    fn card_bg(&self, v: &mut View<'_>, k: &Ink, r: Rect) {
        v.rounded_asset(r.x, r.y, r.w, r.h, 8, k.card_edge);
        v.rounded_asset(r.x + 1, r.y + 1, r.w - 2, r.h - 2, 7, k.card);
    }
    fn round_button(&self, v: &mut View<'_>, k: &Ink, r: Rect, glyph: &str, accent: bool) {
        let hover = v.hover.is_some_and(|(x, y)| r.contains(x, y));
        let fill = if accent {
            k.accent
        } else if hover {
            k.button_hover
        } else {
            k.button
        };
        v.rounded_asset(r.x, r.y, r.w, r.h, r.h.min(r.w) / 2, fill);
        let color = if accent { k.on_accent } else { k.text };
        super::glyph(
            v,
            glyph,
            r.x + r.w as i32 / 2 - 8,
            r.y + r.h as i32 / 2 - 8,
            16,
            color,
        );
    }
    fn big(&self, v: &mut View<'_>, text: &str, size: f32, cx: i32, top: i32, color: [u8; 4]) {
        let mut s = size;
        if !super::big_text_centered(v, text, &mut s, cx, top, color) {
            let w = v.measure_style(text, TextStyle::Semibold28);
            v.text_clip_centered(
                cx - w as i32 / 2,
                (top, 40),
                w + 2,
                text,
                TextStyle::Semibold28,
                color,
            );
        }
    }
    fn paint_focus(&self, v: &mut View<'_>, k: &Ink) {
        self.header(v, k, "Focus sessions");
        let f = self.focus_card();
        self.card_bg(v, k, f);
        let cx = f.x + f.w as i32 / 2;
        let [start, up, down, skip, pause] = self.focus_controls();
        match &self.session {
            None => {
                let title = "Get ready to focus";
                let w = v.measure_style(title, TextStyle::Semibold20);
                v.text_clip_centered(
                    cx - w as i32 / 2,
                    (f.y + 20, 32),
                    w + 2,
                    title,
                    TextStyle::Semibold20,
                    k.text,
                );
                let sub = "We\u{2019}ll turn off notifications and app alerts during each session.";
                v.text_in_box(
                    (f.x + 24, f.y + 56, f.w - 48, 40),
                    sub,
                    TextStyle::Regular14,
                    k.secondary,
                );
                // The minutes picker.
                let mins = format!("{}", self.plan.minutes);
                self.big(v, &mins, 46.0, cx - 20, f.y + 112, k.text);
                v.text_clip_centered(
                    cx - 20 - 16,
                    (f.y + 176, 20),
                    40,
                    "mins",
                    TextStyle::Regular14,
                    k.secondary,
                );
                self.round_button(v, k, up, "clock-up", false);
                self.round_button(v, k, down, "clock-down", false);
                let breaks = self.plan.breaks_text();
                let w = v.measure_style(&breaks, TextStyle::Regular14);
                v.text_clip_centered(
                    cx - w as i32 / 2,
                    (f.y + 232, 24),
                    w + 2,
                    &breaks,
                    TextStyle::Regular14,
                    k.text,
                );
                // Skip breaks check box.
                let b = Rect::new(skip.x, skip.y + 2, 20, 20);
                if self.plan.skip_breaks {
                    v.rounded_asset(b.x, b.y, b.w, b.h, 4, k.accent);
                    super::glyph(v, "clock-check", b.x + 2, b.y + 2, 16, k.on_accent);
                } else {
                    v.rounded_asset(b.x, b.y, b.w, b.h, 4, k.secondary);
                    v.rounded_asset(b.x + 1, b.y + 1, b.w - 2, b.h - 2, 3, k.card);
                }
                v.text_clip_centered(
                    skip.x + 28,
                    (skip.y, skip.h),
                    skip.w - 28,
                    "Skip breaks",
                    TextStyle::Regular14,
                    k.text,
                );
                self.text_button(v, k, start, "Start focus session", true);
            }
            Some(s) => {
                let (focus, _) = s.periods[s.index];
                let left = s.timer.remaining(self.now);
                let label = if focus {
                    let n = s.periods[..=s.index].iter().filter(|p| p.0).count();
                    let total = s.periods.iter().filter(|p| p.0).count();
                    format!("Focus period {n} of {total}")
                } else {
                    "Break".into()
                };
                let w = v.measure_style(&label, TextStyle::Semibold20);
                v.text_clip_centered(
                    cx - w as i32 / 2,
                    (f.y + 20, 32),
                    w + 2,
                    &label,
                    TextStyle::Semibold20,
                    k.text,
                );
                super::ring(
                    v,
                    cx,
                    f.y + 150,
                    84,
                    6,
                    s.timer.permille(self.now),
                    k.track,
                    k.accent,
                );
                let mins = format!("{}", left.div_ceil(60_000));
                self.big(v, &mins, 46.0, cx, f.y + 118, k.text);
                v.text_clip_centered(
                    cx - 16,
                    (f.y + 178, 20),
                    40,
                    "min",
                    TextStyle::Regular14,
                    k.secondary,
                );
                self.text_button(
                    v,
                    k,
                    pause,
                    if s.timer.running() { "Pause" } else { "Resume" },
                    false,
                );
                self.text_button(v, k, start, "End session", false);
            }
        }
        // Daily progress.
        let p = Rect::new(f.x, f.bottom() + 16, f.w, 96);
        self.card_bg(v, k, p);
        v.text_clip_centered(
            p.x + 20,
            (p.y + 12, 24),
            p.w - 40,
            "Daily progress",
            TextStyle::Semibold14,
            k.text,
        );
        let done = format!("{} minutes", self.focused_today / 60_000);
        v.text_clip_centered(
            p.x + 20,
            (p.y + 44, 20),
            160,
            "Completed today",
            TextStyle::Regular12,
            k.secondary,
        );
        v.text_clip_centered(
            p.x + 20,
            (p.y + 62, 24),
            200,
            &done,
            TextStyle::Semibold16,
            k.text,
        );
    }
    fn text_button(&self, v: &mut View<'_>, k: &Ink, r: Rect, label: &str, accent: bool) {
        let hover = v.hover.is_some_and(|(x, y)| r.contains(x, y));
        let fill = match (accent, hover) {
            (true, _) => k.accent,
            (false, true) => k.button_hover,
            (false, false) => k.button,
        };
        v.rounded_asset(r.x, r.y, r.w, r.h, 4, fill);
        let w = v.measure_style(label, TextStyle::Regular14);
        v.text_clip_centered(
            r.x + (r.w as i32 - w as i32) / 2,
            (r.y, r.h),
            w + 2,
            label,
            TextStyle::Regular14,
            if accent { k.on_accent } else { k.text },
        );
    }
    fn paint_timers(&self, v: &mut View<'_>, k: &Ink) {
        self.header(v, k, "Timer");
        for (i, t) in self.timers.iter().enumerate() {
            let r = self.timer_card(i);
            self.card_bg(v, k, r);
            v.text_clip_centered(
                r.x + 16,
                (r.y + 12, 24),
                r.w - 64,
                &t.name,
                TextStyle::Semibold14,
                k.text,
            );
            let cx = r.x + r.w as i32 / 2;
            super::ring(
                v,
                cx,
                r.y + 150,
                100,
                4,
                t.permille(self.now),
                k.track,
                k.accent,
            );
            let text = timer_text(t.remaining(self.now));
            self.big(v, &text, 32.0, cx, r.y + 132, k.text);
            self.round_button(
                v,
                k,
                self.timer_button(i, 0),
                if t.running() && !t.done(self.now) {
                    "clock-pause"
                } else {
                    "clock-play"
                },
                true,
            );
            self.round_button(v, k, self.timer_button(i, 1), "clock-reset", false);
            self.round_button(v, k, self.timer_button(i, 2), "calc-delete", false);
        }
        self.add_button(v, k);
    }
    fn paint_alarms(&self, v: &mut View<'_>, k: &Ink) {
        self.header(v, k, "Alarm");
        let local = self.local;
        for (i, a) in self.alarms.iter().enumerate() {
            let r = self.alarm_card(i);
            self.card_bg(v, k, r);
            let color = if a.enabled { k.text } else { k.secondary };
            self.big_left(v, &a.time_text(), 40.0, r.x + 20, r.y + 16, color);
            let until = local.and_then(|(w, wd, _)| a.minutes_until(wd, w.hour, w.minute));
            if let Some(m) = until {
                v.text_clip_centered(
                    r.x + 20,
                    (r.y + 72, 20),
                    r.w - 40,
                    &rings_in(m),
                    TextStyle::Regular12,
                    k.secondary,
                );
            }
            v.text_clip_centered(
                r.x + 20,
                (r.y + 100, 24),
                r.w - 40,
                &a.name,
                TextStyle::Semibold14,
                color,
            );
            v.text_clip_centered(
                r.x + 20,
                (r.y + 126, 20),
                r.w - 80,
                &a.days_text(),
                TextStyle::Regular12,
                k.secondary,
            );
            let s = self.alarm_switch(i);
            super::switch(
                v,
                (s.x, s.y, s.w, s.h),
                a.enabled,
                k.accent,
                k.secondary,
                k.on_accent,
                k.card,
            );
            self.round_button(v, k, self.alarm_delete(i), "calc-delete", false);
        }
        if self.alarms.is_empty() {
            let c = self.content();
            v.text_clip_centered(
                c.x + 24,
                (c.y + 80, 24),
                c.w - 48,
                "No alarms",
                TextStyle::Regular14,
                k.secondary,
            );
        }
        self.add_button(v, k);
    }
    fn big_left(&self, v: &mut View<'_>, text: &str, size: f32, x: i32, top: i32, color: [u8; 4]) {
        let mut s = size;
        let w = super::text_width(v, text, size).unwrap_or(0) as i32;
        if w == 0 || !super::big_text_centered(v, text, &mut s, x + w / 2, top, color) {
            v.text_clip_centered(x, (top, 40), 400, text, TextStyle::Semibold28, color);
        }
    }
    fn paint_stopwatch(&self, v: &mut View<'_>, k: &Ink) {
        self.header(v, k, "Stopwatch");
        let c = self.content();
        let cx = c.x + c.w as i32 / 2;
        let text = stopwatch_text(self.stopwatch.elapsed(self.now));
        self.big(v, &text, 68.0, cx, c.y + c.h as i32 / 2 - 80, k.text);
        let labels = ["hr", "min", "sec"];
        let _ = labels;
        let [toggle, lap, reset] = self.stopwatch_buttons();
        self.round_button(
            v,
            k,
            toggle,
            if self.stopwatch.running() {
                "clock-pause"
            } else {
                "clock-play"
            },
            true,
        );
        self.round_button(v, k, lap, "clock-flag", false);
        self.round_button(v, k, reset, "clock-reset", false);
        let rows = self.stopwatch.lap_rows();
        if !rows.is_empty() {
            let top = toggle.bottom() + 24;
            let w = 420.min(c.w as i32 - 48);
            let x = cx - w / 2;
            for (col, (label, at)) in [("Laps", 0), ("Time", w / 3), ("Total", 2 * w / 3)]
                .iter()
                .enumerate()
            {
                let _ = col;
                v.text_clip_centered(
                    x + at,
                    (top, 24),
                    (w / 3) as u32,
                    label,
                    TextStyle::Regular12,
                    k.secondary,
                );
            }
            for (i, (n, lap, total)) in rows
                .iter()
                .take(((c.bottom() - top - 32) / 32).max(0) as usize)
                .enumerate()
            {
                let y = top + 28 + i as i32 * 32;
                v.text_clip_centered(
                    x,
                    (y, 28),
                    (w / 3) as u32,
                    &format!("{n}"),
                    TextStyle::Regular14,
                    k.text,
                );
                v.text_clip_centered(
                    x + w / 3,
                    (y, 28),
                    (w / 3) as u32,
                    &stopwatch_text(*lap),
                    TextStyle::Regular14,
                    k.text,
                );
                v.text_clip_centered(
                    x + 2 * w / 3,
                    (y, 28),
                    (w / 3) as u32,
                    &stopwatch_text(*total),
                    TextStyle::Regular14,
                    k.text,
                );
            }
        }
    }
    fn paint_world(&self, v: &mut View<'_>, k: &Ink) {
        self.header(v, k, "World clock");
        let utc = super::wall_utc();
        let local_zone = door_locale::tz::zone_or_utc(self.zone);
        let mut zones: Vec<&door_locale::tz::Zone> = alloc::vec![local_zone];
        zones.extend(
            self.cities
                .iter()
                .filter_map(|id| door_locale::tz::by_id(u32::from(*id))),
        );
        let local_offset = utc
            .and_then(|u| local_zone.to_local(u))
            .map_or(0, |l| l.offset);
        for (i, z) in zones.iter().enumerate() {
            let r = self.city_card(i);
            self.card_bg(v, k, r);
            let city = if i == 0 {
                String::from("Local time")
            } else {
                city_name(z.iana)
            };
            v.text_clip_centered(
                r.x + 20,
                (r.y + 12, 28),
                r.w / 2,
                &city,
                TextStyle::Semibold16,
                k.text,
            );
            if let Some(l) = utc.and_then(|u| z.to_local(u)) {
                let diff = i32::from(l.offset) - i32::from(local_offset);
                let rel = if diff == 0 {
                    String::from("Today")
                } else {
                    let h = diff / 60;
                    let m = (diff % 60).abs();
                    let sign = if diff > 0 { "+" } else { "-" };
                    if m == 0 {
                        format!("{sign}{} hours", h.abs())
                    } else {
                        format!("{sign}{}:{m:02} hours", h.abs())
                    }
                };
                v.text_clip_centered(
                    r.x + 20,
                    (r.y + 42, 20),
                    r.w / 2,
                    &rel,
                    TextStyle::Regular12,
                    k.secondary,
                );
                let time = clock_text(l.time.hour, l.time.minute);
                let w = super::text_width(v, &time, 28.0)
                    .unwrap_or(v.measure_style(&time, TextStyle::Semibold28))
                    as i32;
                let right = r.right() - if i == 0 { 24 } else { 64 };
                self.big(v, &time, 28.0, right - w / 2, r.y + 20, k.text);
            } else {
                v.text_clip_centered(
                    r.x + 20,
                    (r.y + 42, 20),
                    r.w / 2,
                    "Time unavailable (no clock)",
                    TextStyle::Regular12,
                    k.secondary,
                );
            }
            if i > 0 {
                self.round_button(v, k, self.city_delete(i), "calc-delete", false);
            }
        }
        self.add_button(v, k);
    }
    fn paint_settings(&self, v: &mut View<'_>, k: &Ink) {
        self.header(v, k, "Settings");
        let c = self.content();
        v.text_clip_centered(
            c.x + 24,
            (c.y + 80, 24),
            300,
            "App theme",
            TextStyle::Semibold14,
            k.text,
        );
        for (t, label) in ["Light", "Dark", "Use system setting"].iter().enumerate() {
            let r = Rect::new(c.x + 40, c.y + 120 + t as i32 * 32, 200, 28);
            let on = self.theme as usize == t;
            v.rounded_asset(
                r.x,
                r.y + 4,
                20,
                20,
                10,
                if on { k.accent } else { k.secondary },
            );
            v.rounded_asset(
                r.x + if on { 6 } else { 1 },
                r.y + if on { 10 } else { 5 },
                if on { 8 } else { 18 },
                if on { 8 } else { 18 },
                if on { 4 } else { 9 },
                if on { k.on_accent } else { k.card },
            );
            v.text_clip_centered(
                r.x + 32,
                (r.y, r.h),
                r.w - 32,
                label,
                TextStyle::Regular14,
                k.text,
            );
        }
        v.text_clip_centered(
            c.x + 24,
            (c.y + 240, 24),
            300,
            "About",
            TextStyle::Semibold14,
            k.text,
        );
        v.text_clip_centered(
            c.x + 24,
            (c.y + 268, 20),
            c.w - 48,
            "Clock \u{00b7} Doors",
            TextStyle::Regular14,
            k.secondary,
        );
    }
    fn paint_editor(&self, v: &mut View<'_>, k: &Ink) {
        v.rect(0, 0, v.width, v.height, [0, 0, 0, 0x4c]);
        let s = self.editor_sheet();
        v.elevation(s.x, s.y, s.w, s.h, 90);
        v.rounded_asset(s.x, s.y, s.w, s.h, 8, k.card_edge);
        v.rounded_asset(s.x + 1, s.y + 1, s.w - 2, s.h - 2, 7, k.sheet);
        let Some(e) = &self.editor else {
            return;
        };
        let title = match e {
            Editor::Timer { .. } => "Add new timer",
            Editor::Alarm { .. } => "Add new alarm",
            Editor::City { .. } => "Add new location",
        };
        v.text_clip_centered(
            s.x + 24,
            (s.y + 16, 32),
            s.w - 48,
            title,
            TextStyle::Semibold20,
            k.text,
        );
        match e {
            Editor::Timer {
                hours,
                minutes,
                seconds,
            } => {
                for (i, n) in [*hours, *minutes, *seconds].iter().enumerate() {
                    self.stepper_field(v, k, i, &format!("{n:02}"));
                }
            }
            Editor::Alarm { hour, minute, days } => {
                let h = match hour % 12 {
                    0 => 12,
                    h => h,
                };
                self.stepper_field(v, k, 0, &format!("{h:02}"));
                self.stepper_field(v, k, 1, &format!("{minute:02}"));
                self.stepper_field(v, k, 2, if *hour < 12 { "AM" } else { "PM" });
                for (d, name) in DAYS.iter().enumerate() {
                    let r = self.day_chip(d);
                    let on = days & (1 << d) != 0;
                    v.rounded_asset(r.x, r.y, r.w, r.h, 16, if on { k.accent } else { k.button });
                    let w = v.measure_style(name, TextStyle::Regular12);
                    v.text_clip_centered(
                        r.x + (r.w as i32 - w as i32) / 2,
                        (r.y, r.h),
                        w + 2,
                        name,
                        TextStyle::Regular12,
                        if on { k.on_accent } else { k.text },
                    );
                }
            }
            Editor::City { first, chosen } => {
                for i in 0..self.zone_rows() {
                    let Some(z) = door_locale::tz::ZONES.get(first + i) else {
                        break;
                    };
                    let r = self.zone_row(i);
                    let on = *chosen == Some(first + i);
                    if on || v.hover.is_some_and(|(x, y)| r.contains(x, y)) {
                        v.rounded_asset(
                            r.x,
                            r.y,
                            r.w,
                            r.h,
                            4,
                            if on { k.selected } else { k.subtle },
                        );
                    }
                    v.text_clip_centered(
                        r.x + 12,
                        (r.y, r.h),
                        r.w - 24,
                        z.display,
                        TextStyle::Regular14,
                        k.text,
                    );
                }
            }
        }
        let [save, cancel] = self.editor_buttons();
        self.text_button(v, k, save, "Save", true);
        self.text_button(v, k, cancel, "Cancel", false);
    }
    fn stepper_field(&self, v: &mut View<'_>, k: &Ink, i: usize, text: &str) {
        let (up, down) = self.stepper(i);
        self.round_button(v, k, up, "clock-up", false);
        self.round_button(v, k, down, "clock-down", false);
        self.big(
            v,
            text,
            40.0,
            up.x + up.w as i32 / 2,
            up.bottom() + 16,
            k.text,
        );
    }
    fn paint_ringing(&self, v: &mut View<'_>, k: &Ink) {
        v.rect(0, 0, v.width, v.height, [0, 0, 0, 0x4c]);
        let s = self.editor_sheet();
        v.rounded_asset(s.x, s.y, s.w, s.h, 8, k.card_edge);
        v.rounded_asset(s.x + 1, s.y + 1, s.w - 2, s.h - 2, 7, k.sheet);
        if let Some(a) = self.ringing.and_then(|i| self.alarms.get(i)) {
            super::glyph(
                v,
                "clock-alarm",
                s.x + s.w as i32 / 2 - 10,
                s.y + 40,
                20,
                k.accent,
            );
            self.big(
                v,
                &a.time_text(),
                46.0,
                s.x + s.w as i32 / 2,
                s.y + 80,
                k.text,
            );
            let w = v.measure_style(&a.name, TextStyle::Semibold20);
            v.text_clip_centered(
                s.x + (s.w as i32 - w as i32) / 2,
                (s.y + 150, 32),
                w + 2,
                &a.name,
                TextStyle::Semibold20,
                k.text,
            );
        }
        let [dismiss, snooze] = self.ring_buttons();
        self.text_button(v, k, dismiss, "Dismiss", true);
        self.text_button(v, k, snooze, "Snooze (10 min)", false);
    }
}

/// "Asia/Tokyo" -> "Tokyo", "America/New_York" -> "New York".
pub fn city_name(iana: &str) -> String {
    iana.rsplit('/').next().unwrap_or(iana).replace('_', " ")
}

struct Ink {
    mica: [u8; 4],
    layer: [u8; 4],
    layer_edge: [u8; 4],
    card: [u8; 4],
    card_edge: [u8; 4],
    sheet: [u8; 4],
    selected: [u8; 4],
    subtle: [u8; 4],
    button: [u8; 4],
    button_hover: [u8; 4],
    accent: [u8; 4],
    on_accent: [u8; 4],
    text: [u8; 4],
    secondary: [u8; 4],
    track: [u8; 4],
}
const fn white(a: u8) -> [u8; 4] {
    [a, a, a, a]
}
const fn black(a: u8) -> [u8; 4] {
    [0, 0, 0, a]
}
const fn rgb(r: u8, g: u8, b: u8) -> [u8; 4] {
    [b, g, r, 255]
}
/// Sampled at 150%: the navigation pane is Mica, the content layer lifts it
/// (35,39,50 over 29,32,37), the selected row is white 6%; light uses the
/// WinUI layer and subtle fills.
fn ink(dark: bool) -> Ink {
    if dark {
        Ink {
            mica: rgb(32, 32, 32),
            layer: white(0x0d),
            layer_edge: black(0x19),
            card: white(0x0d),
            card_edge: black(0x19),
            sheet: rgb(44, 44, 44),
            selected: white(0x0f),
            subtle: white(0x0b),
            button: white(0x0f),
            button_hover: white(0x15),
            accent: rgb(0x4c, 0xc2, 0xff),
            on_accent: rgb(0, 0, 0),
            text: rgb(255, 255, 255),
            secondary: rgb(206, 206, 206),
            track: rgb(72, 72, 72),
        }
    } else {
        Ink {
            mica: rgb(243, 243, 243),
            layer: white(0x80),
            layer_edge: black(0x0f),
            card: white(0xb3),
            card_edge: black(0x0f),
            sheet: rgb(249, 249, 249),
            selected: black(0x09),
            subtle: black(0x06),
            button: white(0xb3),
            button_hover: [0xf6, 0xf6, 0xf6, 0x80],
            accent: rgb(0x00, 0x5f, 0xb8),
            on_accent: rgb(255, 255, 255),
            text: rgb(26, 26, 26),
            secondary: rgb(96, 96, 96),
            track: rgb(214, 214, 214),
        }
    }
}

impl Application for Clock {
    fn title(&self) -> &str {
        "Clock"
    }
    fn options(&self) -> door_workspace::runtime::WindowOptions {
        door_workspace::runtime::WindowOptions {
            width: 1000,
            height: 680,
            caption_height: TITLE as u32,
            caption_left: PANE,
            centered: true,
        }
    }
    fn caption(&self) -> (u32, u32) {
        (PANE, 1)
    }
    fn configure(&mut self, width: u32, height: u32) {
        self.width = width.max(PANE + 200);
        self.height = height.max(240);
        if !self.ready {
            self.ready = true;
            door_rt::diagnostic(format_args!("DOOR_INBOX READY app=clock\n"));
        }
    }
    fn draw(&self, v: &mut View<'_>) {
        self.paint(v);
    }
    fn target(&self, x: i32, y: i32) -> Option<u32> {
        self.hits()
            .into_iter()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, t)| t)
            .filter(|t| *t != u32::MAX)
    }
    fn click(&mut self, id: u32, _s: &mut Storage) -> Result<bool, Error> {
        Ok(self.activate(id))
    }
    fn key(&mut self, key: u32, _s: &mut Storage) -> Result<bool, Error> {
        let scan = key & 511;
        match scan {
            1 if self.editor.is_some() => self.editor = None,
            1 if self.ringing.is_some() => self.ringing = None,
            57 if self.page == Page::Stopwatch => self.stopwatch.toggle(self.now),
            _ => return Ok(false),
        }
        Ok(true)
    }
    fn wheel(&mut self, x: i32, y: i32, notches: i32, _f: &FontAtlas) -> bool {
        if let Some(Editor::City { first, .. }) = &mut self.editor {
            let _ = (x, y);
            let max = door_locale::tz::ZONES.len().saturating_sub(1);
            *first = if notches > 0 {
                first.saturating_sub(3)
            } else {
                (*first + 3).min(max)
            };
            return true;
        }
        false
    }
    fn tick(&mut self, _s: &mut Storage, now: u64) -> Result<bool, Error> {
        let local = super::local_now(self.zone);
        let visible = self.advance(now, local);
        if !self.toasts.is_empty() {
            super::post_toasts(self.registry, super::App::Clock, &mut self.toasts);
        }
        // A ringing alarm loops the scheme's Alarm sound until dismissed or
        // snoozed (Notification.Looping.Alarm: Alarm01, about 5.6 s).
        if self.ringing.is_some() {
            if now >= self.alarm_at && self.registry != 0 {
                if self.alarm_audio.is_none() {
                    self.alarm_audio =
                        door_service_client::audio::Audio::connect(self.registry).ok();
                }
                let played = self.alarm_audio.as_mut().is_some_and(|a| {
                    a.play_sound(door_service_protocol::audio::sound::ALARM)
                        .is_ok()
                });
                if !played {
                    self.alarm_audio = None;
                }
                self.alarm_at = now + if played { 5_600 } else { 2_000 };
            }
        } else if self.alarm_audio.take().is_some() {
            self.alarm_at = 0;
        }
        // Repaint only while something is moving or a minute turned.
        let moving = self.stopwatch.running()
            || self.timers.iter().any(|t| t.running() && !t.done(now))
            || self.session.is_some()
            || self.page == Page::WorldClock
            || self.page == Page::Alarm;
        Ok(visible && moving)
    }
    fn menu_open(&self) -> bool {
        self.editor.is_some()
    }
    fn dismiss_menu(&mut self) {
        self.editor = None;
    }
}
