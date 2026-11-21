//! Typed per-user preference words (P14 Batch 1). Each is an HKCU dword in
//! the user's root key, read and written through Storage `GET_SETTING` /
//! `SET_SETTING` with the id below (`GET_SETTING_FOR` / `SET_SETTING_FOR`
//! for Init), validated by storage-core with [`valid`]. Defaults are clean
//! Windows 11 26H2: nothing changes until the user acts.
//!
//! Consumers that run outside Settings receive the words through the window
//! server's `SET_PREFERENCE` op (Init sends every word at session binding,
//! Settings sends a word when the user changes it).

/// Personalization > Colors: "Show accent colour on Start and taskbar"
/// ([`ACCENT_START_TASKBAR`]) and "on title bars and window borders"
/// ([`ACCENT_TITLE_BARS`]). Default 0 (both off, as Windows).
pub const ACCENT_SURFACES: u32 = 10;
/// Personalization > Colors > More colour options: switch Light/Dark mode
/// at fixed times ([`ThemeSchedule`]). Default off, light from 07:00, dark
/// from 19:00.
pub const THEME_SCHEDULE: u32 = 11;
/// System > Notifications flags ([`notify`]). Default: notifications and
/// sounds on, everything else off.
pub const NOTIFICATIONS: u32 = 12;
/// System > Notifications > "Turn on do not disturb automatically" >
/// "During these times" ([`DndSchedule`]). Default 22:00 to 07:00, daily
/// (the rule itself is off until [`notify::DND_SCHEDULED`] is set).
pub const DND_SCHEDULE: u32 = 13;
/// System > Notifications > Focus ([`Focus`]): session length and options.
pub const FOCUS: u32 = 14;
/// Accounts > Sign-in options > "If you've been away, when should Doors
/// require you to sign in again?": minutes without input before the
/// session locks, one of [`SIGN_IN_AWAY_MINUTES`]; 0 = Never (default).
pub const SIGN_IN_AWAY: u32 = 15;
/// Personalization > Lock screen background ([`LockScreen`]). Default 0:
/// the desktop background, as Doors has always shown.
pub const LOCK_SCREEN: u32 = 16;
/// Time & language > Language & region > Regional format overrides
/// ([`RegionalOverrides`]). Default 0: every format follows the region.
pub const REGIONAL_OVERRIDES: u32 = 17;
/// System > Clipboard: bit 0 Clipboard history (default off, as Windows).
pub const CLIPBOARD: u32 = 18;
/// Privacy & security > Search: bit 0 "Search history on this device"
/// (default on, as Windows).
pub const SEARCH_HISTORY: u32 = 19;
/// System > Advanced > End task: bit 0 "End task" in the taskbar's
/// right-click menu (default off, as Windows).
pub const END_TASK: u32 = 20;
/// Personalization > Taskbar behaviours ([`taskbar`] bits). Default 0:
/// always combine, no auto-hide, no seconds, Task view and the show-desktop
/// corner shown, badges and flashing on, the default material.
pub const TASKBAR: u32 = 21;
/// Personalization > Start > Folders: the folders next to the power button
/// ([`start_folders`] bits). Default 0: none, as Windows.
pub const START_FOLDERS: u32 = 22;
/// Personalization > Background ([`Background`]). Default: a picture,
/// Fill, slideshow every 30 minutes.
pub const BACKGROUND: u32 = 23;
/// Window appearance ([`windows`] bits): corner style, animation speed,
/// Mica in every app. Default 0: rounded, normal speed, apps choose.
pub const WINDOWS: u32 = 24;
/// Bluetooth & devices > Mouse ([`mouse`] bits). Default: speed 10,
/// enhanced precision on, left primary button, 3 lines per notch, inactive
/// windows scroll.
pub const MOUSE: u32 = 25;
/// System > Multitasking ([`multitask`] bits). Default 0: every snap
/// feature on, title bar shake off (as 26H2).
pub const MULTITASK: u32 = 26;
/// Night light and colour filters ([`vision`] bits). Default: both off,
/// night light strength 50, grayscale filter.
pub const VISION: u32 = 27;
/// Optional Doors features (`features` bits): what File Explorer's
/// navigation pane shows when they are turned on.
pub const FEATURES: u32 = 28;
/// Sound output (`audio::Output::word`): master volume, mute, mono audio
/// and balance, restored to door.audio when the session starts.
pub const SOUND: u32 = 29;
/// Volume 50, not muted, stereo, centred.
pub const SOUND_DEFAULT: u32 = 50;
/// Privacy & security > Microphone (`audio::capture` policy bits), pushed to
/// door.audio when the session starts. Default 0: all access on, as Windows.
pub const MICROPHONE: u32 = 30;
/// Network & internet > VPN > Advanced settings for all VPN connections
/// ([`vpn`] bits store "off"; default 0: both on, as Windows).
pub const VPN: u32 = 31;
/// Personalization > Background > "Recent images": the five most recently
/// used built-in pictures, most recent first ([`RecentBackgrounds`]).
pub const BACKGROUND_RECENT: u32 = 32;
/// Personalization > Colors > "Recent colors": the five most recently
/// chosen Windows colors, most recent first ([`RecentAccents`]).
pub const ACCENT_RECENT: u32 = 33;
pub const LAST: u32 = ACCENT_RECENT;

/// [`VPN`] bits.
pub mod vpn {
    /// "Allow VPN over metered networks" is off.
    pub const NO_METERED: u32 = 1;
    /// "Allow VPN while roaming" is off.
    pub const NO_ROAMING: u32 = 2;
    pub const ALL: u32 = 3;
}

/// [`MOUSE`] bits.
pub mod mouse {
    /// Pointer speed 1..=20 (Windows' slider), bits 0..5.
    pub const SPEED_MASK: u32 = 31;
    /// "Enhanced pointer precision" off (no acceleration).
    pub const NO_PRECISION: u32 = 1 << 5;
    /// "Primary mouse button": Right.
    pub const SWAP_BUTTONS: u32 = 1 << 6;
    /// Lines to scroll per notch 1..=20, bits 7..12.
    pub const LINES_SHIFT: u32 = 7;
    pub const LINES_MASK: u32 = 31 << LINES_SHIFT;
    /// "Scroll inactive windows when hovering over them" off.
    pub const NO_INACTIVE_SCROLL: u32 = 1 << 12;
    pub const ALL: u32 = (1 << 12) - 1;
    /// Speed 10, 3 lines, and enhanced precision off: the VM pointer already
    /// carries the host's acceleration, so Doors starts 1:1 (Windows starts
    /// with precision on).
    pub const DEFAULT: u32 = 10 | NO_PRECISION | 3 << LINES_SHIFT;
    pub const fn speed(word: u32) -> u32 {
        word & SPEED_MASK
    }
    pub const fn lines(word: u32) -> u32 {
        (word & LINES_MASK) >> LINES_SHIFT
    }
    pub const fn valid(word: u32) -> bool {
        word & !ALL == 0
            && speed(word) >= 1
            && speed(word) <= 20
            && lines(word) >= 1
            && lines(word) <= 20
    }
}
/// [`MULTITASK`] bits.
pub mod features {
    /// NoneDrive: Doors' own cloud folder, synced with a private server the
    /// user configures (File Explorer shows it under Home, as OneDrive).
    pub const NONEDRIVE: u32 = 1;
    /// Linux: the Linux environment (File Explorer shows a Linux root after
    /// Network, as Windows Subsystem for Linux does).
    pub const LINUX: u32 = 1 << 1;
    pub const ALL: u32 = 3;
}
pub mod multitask {
    /// "Snap windows" off.
    pub const NO_SNAP: u32 = 1;
    /// "Show snap layouts when I hover over a window's maximize button" off.
    pub const NO_HOVER_LAYOUTS: u32 = 1 << 1;
    /// "Show snap layouts when I drag a window to the top of my screen" off.
    pub const NO_DRAG_TOP: u32 = 1 << 2;
    /// "Show what I can snap next to it when I snap a window" off.
    pub const NO_SNAP_ASSIST: u32 = 1 << 3;
    /// "Title bar window shake" on.
    pub const SHAKE: u32 = 1 << 4;
    pub const ALL: u32 = 31;
}
/// [`VISION`] bits.
pub mod vision {
    /// Display > Night light on.
    pub const NIGHT_LIGHT: u32 = 1;
    /// Night light strength 0..=100, bits 1..8.
    pub const STRENGTH_SHIFT: u32 = 1;
    pub const STRENGTH_MASK: u32 = 127 << STRENGTH_SHIFT;
    /// Accessibility > Color filters on.
    pub const FILTER: u32 = 1 << 8;
    /// Filter 0..=5, bits 9..12: grayscale, inverted, grayscale inverted,
    /// red-green (green weak, deuteranopia), red-green (red weak,
    /// protanopia), blue-yellow (tritanopia).
    pub const KIND_SHIFT: u32 = 9;
    pub const KIND_MASK: u32 = 7 << KIND_SHIFT;
    pub const KINDS: [&str; 6] = [
        "Grayscale",
        "Inverted",
        "Grayscale inverted",
        "Red-green (green weak, deuteranopia)",
        "Red-green (red weak, protanopia)",
        "Blue-yellow (tritanopia)",
    ];
    /// Display brightness, stored as the dimming below 100 (bits 12..19)
    /// so the default word (0) keeps full brightness. Scanout applies it in
    /// software: this profile's display has no backlight control.
    pub const DIM_SHIFT: u32 = 12;
    pub const DIM_MASK: u32 = 127 << DIM_SHIFT;
    pub const ALL: u32 = (1 << 19) - 1;
    pub const DEFAULT: u32 = 50 << STRENGTH_SHIFT;
    pub const fn strength(word: u32) -> u32 {
        (word & STRENGTH_MASK) >> STRENGTH_SHIFT
    }
    pub const fn kind(word: u32) -> u32 {
        (word & KIND_MASK) >> KIND_SHIFT
    }
    /// Brightness 0..=100.
    pub const fn brightness(word: u32) -> u32 {
        100u32.saturating_sub((word & DIM_MASK) >> DIM_SHIFT)
    }
    pub const fn with_brightness(word: u32, brightness: u32) -> u32 {
        let b = if brightness > 100 { 100 } else { brightness };
        (word & !DIM_MASK) | ((100 - b) << DIM_SHIFT)
    }
    pub const fn valid(word: u32) -> bool {
        word & !ALL == 0
            && strength(word) <= 100
            && kind(word) <= 5
            && (word & DIM_MASK) >> DIM_SHIFT <= 100
    }
}

/// [`TASKBAR`] bits.
pub mod taskbar {
    /// "Combine taskbar buttons and hide labels": 0 Always, 1 When taskbar
    /// is full, 2 Never.
    pub const COMBINE_MASK: u32 = 3;
    pub const AUTO_HIDE: u32 = 1 << 2;
    /// "Show seconds in system tray clock".
    pub const SECONDS: u32 = 1 << 3;
    pub const HIDE_TASK_VIEW: u32 = 1 << 4;
    /// "Select the far corner of the taskbar to show the desktop" off.
    pub const NO_DESKTOP_CORNER: u32 = 1 << 5;
    /// "Show badges on taskbar apps" off.
    pub const NO_BADGES: u32 = 1 << 6;
    /// "Show flashing on taskbar apps" off.
    pub const NO_FLASHING: u32 = 1 << 7;
    /// Doors extra (TranslucentTB): 0 Default, 1 Translucent, 2 Opaque, 3 Clear.
    pub const LOOK_SHIFT: u32 = 8;
    pub const LOOK_MASK: u32 = 3 << LOOK_SHIFT;
    /// Doors extras: middle-click closes, a click switches to the last
    /// active window of a group.
    pub const MIDDLE_CLOSE: u32 = 1 << 10;
    pub const LAST_ACTIVE: u32 = 1 << 11;
    /// Taskbar behaviors > Taskbar size: Large (a 56 px bar with 32 px
    /// icons, natively what the Windhawk height mod does). Small is the
    /// desktop-controls bit; Large wins when both are set.
    pub const LARGE: u32 = 1 << 12;
    pub const ALL: u32 = (1 << 13) - 1;
    pub const fn combine(word: u32) -> u32 {
        word & COMBINE_MASK
    }
    pub const fn look(word: u32) -> u32 {
        (word & LOOK_MASK) >> LOOK_SHIFT
    }
    pub const fn valid(word: u32) -> bool {
        word & !ALL == 0 && combine(word) < 3
    }
}
/// [`START_FOLDERS`] bits, in the order Start shows them.
pub mod start_folders {
    pub const NAMES: [&str; 9] = [
        "Settings",
        "File Explorer",
        "Documents",
        "Downloads",
        "Music",
        "Pictures",
        "Videos",
        "Network",
        "Personal folder",
    ];
    pub const ALL: u32 = (1 << NAMES.len()) - 1;
}
/// [`WINDOWS`] bits.
pub mod windows {
    /// Doors extra: square window corners.
    pub const SQUARE_CORNERS: u32 = 1;
    /// Doors extra: 0 normal, 1 fast, 2 slow animations.
    pub const SPEED_SHIFT: u32 = 1;
    pub const SPEED_MASK: u32 = 3 << SPEED_SHIFT;
    /// Doors extra (MicaForEveryone): Mica behind every app's title bar.
    pub const MICA_EVERYWHERE: u32 = 1 << 3;
    pub const ALL: u32 = 0xf;
    pub const fn speed(word: u32) -> u32 {
        (word & SPEED_MASK) >> SPEED_SHIFT
    }
    pub const fn valid(word: u32) -> bool {
        word & !ALL == 0 && speed(word) < 3
    }
}
/// [`BACKGROUND`]: bits 0..2 kind (0 Picture, 1 Solid color, 2 Slideshow),
/// 2..5 fit (0 Fill, 1 Fit, 2 Stretch, 3 Tile, 4 Center, 5 Span), 5..7
/// slideshow interval (0 1 minute, 1 10 minutes, 2 30 minutes, 3 1 hour),
/// bit 7 shuffle, 8..32 the solid colour 0xRRGGBB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Background {
    pub kind: u32,
    pub fit: u32,
    pub interval: u32,
    pub shuffle: bool,
    pub color: u32,
}
impl Background {
    pub const DEFAULT: Self = Self {
        kind: 0,
        fit: 0,
        interval: 2,
        shuffle: false,
        color: 0,
    };
    pub const KINDS: [&'static str; 3] = ["Picture", "Solid color", "Slideshow"];
    pub const FITS: [&'static str; 6] = ["Fill", "Fit", "Stretch", "Tile", "Center", "Span"];
    pub const INTERVALS: [&'static str; 4] = ["1 minute", "10 minutes", "30 minutes", "1 hour"];
    /// Slideshow interval in minutes.
    pub const MINUTES: [u32; 4] = [1, 10, 30, 60];
    pub const fn bits(self) -> u32 {
        self.kind
            | self.fit << 2
            | self.interval << 5
            | (self.shuffle as u32) << 7
            | self.color << 8
    }
    pub const fn from_bits(bits: u32) -> Option<Self> {
        let b = Self {
            kind: bits & 3,
            fit: (bits >> 2) & 7,
            interval: (bits >> 5) & 3,
            shuffle: bits & (1 << 7) != 0,
            color: bits >> 8,
        };
        if b.kind > 2 || b.fit > 5 {
            return None;
        }
        Some(b)
    }
}
/// [`ACCENT_RECENT`]: up to five distinct Windows color indices
/// (`door_design_tokens::accent::WINDOWS_COLORS`, 0..48), six bits each,
/// most recent in bits 0..6; [`RecentAccents::EMPTY`] fills the unused
/// slots, which all follow the used ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecentAccents(pub [u32; 5]);
impl RecentAccents {
    pub const EMPTY: u32 = 63;
    /// A new account has used only the default blue (index 16, 0078D4).
    pub const DEFAULT: Self = Self([16, Self::EMPTY, Self::EMPTY, Self::EMPTY, Self::EMPTY]);
    pub const fn bits(self) -> u32 {
        let mut bits = 0;
        let mut i = 0;
        while i < 5 {
            bits |= self.0[i] << (6 * i);
            i += 1;
        }
        bits
    }
    pub fn from_bits(bits: u32) -> Option<Self> {
        if bits >> 30 != 0 {
            return None;
        }
        let ids: [u32; 5] = core::array::from_fn(|i| (bits >> (6 * i)) & 63);
        let used = ids.iter().take_while(|id| **id != Self::EMPTY).count();
        let valid = ids[..used].iter().all(|id| *id < 48)
            && ids[used..].iter().all(|id| *id == Self::EMPTY)
            && (0..used).all(|i| ids[..i].iter().all(|id| *id != ids[i]));
        valid.then_some(Self(ids))
    }
    /// The used colors, most recent first.
    pub fn colors(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().copied().take_while(|id| *id != Self::EMPTY)
    }
    /// `index` moved (or added) to the front, the others keeping their
    /// order; the oldest falls off a full list.
    pub fn used(self, index: u32) -> Self {
        let mut ids = [Self::EMPTY; 5];
        ids[0] = index;
        let mut n = 1;
        for old in self.colors() {
            if old != index && n < 5 {
                ids[n] = old;
                n += 1;
            }
        }
        Self(ids)
    }
}
/// [`BACKGROUND_RECENT`]: five distinct picture ids (pack ids,
/// `window::WALLPAPERS`), five bits each, most recent in bits 0..5.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecentBackgrounds(pub [u32; 5]);
impl RecentBackgrounds {
    /// Bloom, dark Bloom, then the first Glow, Captured Motion and Sunrise
    /// pictures.
    pub const DEFAULT: Self = Self([0, 1, 2, 7, 3]);
    pub const fn bits(self) -> u32 {
        let mut bits = 0;
        let mut i = 0;
        while i < 5 {
            bits |= self.0[i] << (5 * i);
            i += 1;
        }
        bits
    }
    pub fn from_bits(bits: u32) -> Option<Self> {
        if bits >> 25 != 0 {
            return None;
        }
        let ids: [u32; 5] = core::array::from_fn(|i| (bits >> (5 * i)) & 31);
        let distinct = (0..5).all(|i| ids[..i].iter().all(|id| *id != ids[i]));
        (distinct && ids.iter().all(|id| *id < crate::window::WALLPAPERS)).then_some(Self(ids))
    }
    /// `id` moved to the front, the others keeping their order.
    pub fn used(self, id: u32) -> Self {
        let mut ids = [id; 5];
        let mut n = 1;
        for &old in &self.0 {
            if old != id && n < 5 {
                ids[n] = old;
                n += 1;
            }
        }
        Self(ids)
    }
}

pub const ACCENT_START_TASKBAR: u32 = 1;
pub const ACCENT_TITLE_BARS: u32 = 2;
pub const CLIPBOARD_HISTORY: u32 = 1;
pub const SEARCH_HISTORY_ON: u32 = 1;
pub const END_TASK_ON: u32 = 1;

/// Choices for [`SIGN_IN_AWAY`] (minutes; 0 = Never).
pub const SIGN_IN_AWAY_MINUTES: [u32; 6] = [0, 1, 3, 5, 15, 30];

/// System > Notifications flag bits ([`NOTIFICATIONS`]).
pub mod notify {
    /// "Notifications": get notifications from apps and other senders.
    pub const ON: u32 = 1 << 0;
    /// "Turn on do not disturb automatically" > "During these times".
    pub const DND_SCHEDULED: u32 = 1 << 1;
    /// "Allow notifications to play sounds".
    pub const SOUNDS: u32 = 1 << 2;
    /// "Do not disturb" (manual), kept across sign-out and restart.
    pub const DND: u32 = 1 << 3;
    /// Additional settings: "Show the Doors welcome experience after
    /// updates and when signed in to show what's new and suggested".
    pub const WELCOME: u32 = 1 << 4;
    /// Additional settings: "Suggest ways to get the most out of Doors and
    /// finish setting up this device".
    pub const FINISH_SETUP: u32 = 1 << 5;
    /// Additional settings: "Get tips and suggestions when using Doors".
    pub const TIPS: u32 = 1 << 6;
    pub const ALL: u32 = (1 << 7) - 1;
    pub const DEFAULT: u32 = ON | SOUNDS;
}

/// Minutes since midnight, 0..1440.
pub const MINUTES_PER_DAY: u32 = 1440;

/// [`THEME_SCHEDULE`]: bit 0 enabled, bits 1..12 light-from minute, bits
/// 12..23 dark-from minute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeSchedule {
    pub enabled: bool,
    pub light_from: u32,
    pub dark_from: u32,
}
impl ThemeSchedule {
    pub const DEFAULT: Self = Self {
        enabled: false,
        light_from: 7 * 60,
        dark_from: 19 * 60,
    };
    pub const fn bits(self) -> u32 {
        self.enabled as u32 | (self.light_from << 1) | (self.dark_from << 12)
    }
    pub const fn from_bits(bits: u32) -> Option<Self> {
        let light_from = (bits >> 1) & 0x7ff;
        let dark_from = (bits >> 12) & 0x7ff;
        if bits >> 23 != 0 || light_from >= MINUTES_PER_DAY || dark_from >= MINUTES_PER_DAY {
            return None;
        }
        Some(Self {
            enabled: bits & 1 != 0,
            light_from,
            dark_from,
        })
    }
    /// Dark mode is due at local `minute` (0..1440): from `dark_from` up to
    /// `light_from`, across midnight. Equal times mean always light.
    pub const fn dark_at(self, minute: u32) -> bool {
        let m = minute % MINUTES_PER_DAY;
        if self.light_from == self.dark_from {
            false
        } else if self.dark_from > self.light_from {
            m >= self.dark_from || m < self.light_from
        } else {
            m >= self.dark_from && m < self.light_from
        }
    }
}

/// [`DND_SCHEDULE`]: bits 0..11 from minute, 11..22 to minute, 22..24
/// repeat (0 daily, 1 weekdays, 2 weekends).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DndSchedule {
    pub from: u32,
    pub to: u32,
    pub repeat: u32,
}
impl DndSchedule {
    pub const DEFAULT: Self = Self {
        from: 22 * 60,
        to: 7 * 60,
        repeat: 0,
    };
    pub const REPEATS: [&'static str; 3] = ["Daily", "Weekdays", "Weekends"];
    pub const fn bits(self) -> u32 {
        self.from | (self.to << 11) | (self.repeat << 22)
    }
    pub const fn from_bits(bits: u32) -> Option<Self> {
        let from = bits & 0x7ff;
        let to = (bits >> 11) & 0x7ff;
        let repeat = (bits >> 22) & 3;
        if bits >> 24 != 0 || from >= MINUTES_PER_DAY || to >= MINUTES_PER_DAY || repeat > 2 {
            return None;
        }
        Some(Self { from, to, repeat })
    }
    /// The rule holds at local `minute` on `weekday` (0 = Sunday). A span
    /// across midnight belongs to the day it starts on.
    pub const fn active(self, minute: u32, weekday: u32) -> bool {
        let m = minute % MINUTES_PER_DAY;
        let (inside, started_yesterday) = if self.from == self.to {
            (false, false)
        } else if self.from < self.to {
            (m >= self.from && m < self.to, false)
        } else {
            (m >= self.from || m < self.to, m < self.to)
        };
        if !inside {
            return false;
        }
        let day = if started_yesterday {
            (weekday + 6) % 7
        } else {
            weekday % 7
        };
        match self.repeat {
            1 => day >= 1 && day <= 5,
            2 => day == 0 || day == 6,
            _ => true,
        }
    }
}

/// [`FOCUS`]: bits 0..8 session minutes (one of [`Focus::LENGTHS`]), bit 9
/// hide badges on taskbar apps, bit 10 hide flashing on taskbar apps, bit
/// 11 turn on do not disturb during the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Focus {
    pub minutes: u32,
    pub hide_badges: bool,
    pub hide_flashing: bool,
    pub dnd: bool,
}
impl Focus {
    pub const LENGTHS: [u32; 12] = [5, 10, 15, 20, 30, 45, 60, 90, 120, 150, 180, 240];
    pub const DEFAULT: Self = Self {
        minutes: 30,
        hide_badges: true,
        hide_flashing: true,
        dnd: true,
    };
    pub const fn bits(self) -> u32 {
        self.minutes
            | (self.hide_badges as u32) << 9
            | (self.hide_flashing as u32) << 10
            | (self.dnd as u32) << 11
    }
    pub const fn from_bits(bits: u32) -> Option<Self> {
        let minutes = bits & 0xff;
        if bits & !(0xff | 7 << 9) != 0 {
            return None;
        }
        let mut i = 0;
        while i < Self::LENGTHS.len() {
            if Self::LENGTHS[i] == minutes {
                return Some(Self {
                    minutes,
                    hide_badges: bits & 1 << 9 != 0,
                    hide_flashing: bits & 1 << 10 != 0,
                    dnd: bits & 1 << 11 != 0,
                });
            }
            i += 1;
        }
        None
    }
}

/// [`LOCK_SCREEN`]: bits 0..4 kind (0 the desktop background, 1..=4 one of
/// the four Doors pictures `kind - 1`, 5 solid colour), bits 8..32 the
/// solid colour 0xRRGGBB (zero unless kind is 5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockScreen(u32);
impl LockScreen {
    pub const DESKTOP: Self = Self(0);
    pub const SOLID: u32 = 5;
    pub const fn picture(index: u32) -> Option<Self> {
        if index < 4 {
            Some(Self(index + 1))
        } else {
            None
        }
    }
    pub const fn solid(rgb: u32) -> Self {
        Self(Self::SOLID | (rgb & 0x00ff_ffff) << 8)
    }
    pub const fn from_bits(bits: u32) -> Option<Self> {
        let kind = bits & 0xff;
        if kind > 5 || (kind != 5 && bits >> 8 != 0) {
            return None;
        }
        Some(Self(bits))
    }
    pub const fn bits(self) -> u32 {
        self.0
    }
    /// 0 desktop background, 1..=4 picture, 5 solid colour.
    pub const fn kind(self) -> u32 {
        self.0 & 0xff
    }
    pub const fn rgb(self) -> u32 {
        self.0 >> 8
    }
}

/// [`REGIONAL_OVERRIDES`]: each field 0 = the regional format's own.
/// bits 0..2 clock (1 12-hour, 2 24-hour), bits 2..5 first day of week
/// (1..=7 = Sunday..Saturday), then 4-bit pattern indices (1-based into
/// `door_locale`'s pattern lists) for short date (5..9), long date (9..13),
/// short time (13..17) and long time (17..21).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct RegionalOverrides {
    pub clock: u32,
    pub first_day: u32,
    pub short_date: u32,
    pub long_date: u32,
    pub short_time: u32,
    pub long_time: u32,
}
impl RegionalOverrides {
    pub const NONE: Self = Self {
        clock: 0,
        first_day: 0,
        short_date: 0,
        long_date: 0,
        short_time: 0,
        long_time: 0,
    };
    pub const fn bits(self) -> u32 {
        self.clock
            | self.first_day << 2
            | self.short_date << 5
            | self.long_date << 9
            | self.short_time << 13
            | self.long_time << 17
    }
    pub const fn from_bits(bits: u32) -> Option<Self> {
        let clock = bits & 3;
        let first_day = (bits >> 2) & 7;
        if bits >> 21 != 0 || clock > 2 {
            return None;
        }
        Some(Self {
            clock,
            first_day,
            short_date: (bits >> 5) & 15,
            long_date: (bits >> 9) & 15,
            short_time: (bits >> 13) & 15,
            long_time: (bits >> 17) & 15,
        })
    }
}

/// One preference word: id, HKCU value name, default and validator.
#[derive(Clone, Copy)]
pub struct Preference {
    pub id: u32,
    pub name: &'static str,
    pub default: u32,
    pub valid: fn(u32) -> bool,
}
pub const ALL: [Preference; 24] = [
    Preference {
        id: ACCENT_SURFACES,
        name: "AccentSurfaces",
        default: 0,
        valid: |v| v & !3 == 0,
    },
    Preference {
        id: THEME_SCHEDULE,
        name: "ThemeSchedule",
        default: ThemeSchedule::DEFAULT.bits(),
        valid: |v| ThemeSchedule::from_bits(v).is_some(),
    },
    Preference {
        id: NOTIFICATIONS,
        name: "Notifications",
        default: notify::DEFAULT,
        valid: |v| v & !notify::ALL == 0,
    },
    Preference {
        id: DND_SCHEDULE,
        name: "DoNotDisturbSchedule",
        default: DndSchedule::DEFAULT.bits(),
        valid: |v| DndSchedule::from_bits(v).is_some(),
    },
    Preference {
        id: FOCUS,
        name: "Focus",
        default: Focus::DEFAULT.bits(),
        valid: |v| Focus::from_bits(v).is_some(),
    },
    Preference {
        id: SIGN_IN_AWAY,
        name: "SignInAway",
        default: 0,
        valid: |v| {
            let mut i = 0;
            while i < SIGN_IN_AWAY_MINUTES.len() {
                if SIGN_IN_AWAY_MINUTES[i] == v {
                    return true;
                }
                i += 1;
            }
            false
        },
    },
    Preference {
        id: LOCK_SCREEN,
        name: "LockScreen",
        default: 0,
        valid: |v| LockScreen::from_bits(v).is_some(),
    },
    Preference {
        id: REGIONAL_OVERRIDES,
        name: "RegionalOverrides",
        default: 0,
        valid: |v| RegionalOverrides::from_bits(v).is_some(),
    },
    Preference {
        id: CLIPBOARD,
        name: "Clipboard",
        default: 0,
        valid: |v| v & !CLIPBOARD_HISTORY == 0,
    },
    Preference {
        id: SEARCH_HISTORY,
        name: "SearchHistory",
        default: SEARCH_HISTORY_ON,
        valid: |v| v & !SEARCH_HISTORY_ON == 0,
    },
    Preference {
        id: END_TASK,
        name: "EndTask",
        default: 0,
        valid: |v| v & !END_TASK_ON == 0,
    },
    Preference {
        id: TASKBAR,
        name: "Taskbar",
        default: 0,
        valid: taskbar::valid,
    },
    Preference {
        id: START_FOLDERS,
        name: "StartFolders",
        default: 0,
        valid: |v| v & !start_folders::ALL == 0,
    },
    Preference {
        id: BACKGROUND,
        name: "Background",
        default: Background::DEFAULT.bits(),
        valid: |v| Background::from_bits(v).is_some(),
    },
    Preference {
        id: WINDOWS,
        name: "WindowAppearance",
        default: 0,
        valid: windows::valid,
    },
    Preference {
        id: MOUSE,
        name: "Mouse",
        default: mouse::DEFAULT,
        valid: mouse::valid,
    },
    Preference {
        id: MULTITASK,
        name: "Multitasking",
        default: 0,
        valid: |v| v & !multitask::ALL == 0,
    },
    Preference {
        id: VISION,
        name: "Vision",
        default: vision::DEFAULT,
        valid: vision::valid,
    },
    Preference {
        id: FEATURES,
        name: "Features",
        default: 0,
        valid: |v| v & !features::ALL == 0,
    },
    Preference {
        id: SOUND,
        name: "SoundOutput",
        default: SOUND_DEFAULT,
        valid: |v| crate::audio::Output::from_word(v).is_some(),
    },
    Preference {
        id: MICROPHONE,
        name: "Microphone",
        default: 0,
        valid: crate::audio::capture::valid_word,
    },
    Preference {
        id: VPN,
        name: "VPN",
        default: 0,
        valid: |v| v & !vpn::ALL == 0,
    },
    Preference {
        id: BACKGROUND_RECENT,
        name: "RecentBackgrounds",
        default: RecentBackgrounds::DEFAULT.bits(),
        valid: |v| RecentBackgrounds::from_bits(v).is_some(),
    },
    Preference {
        id: ACCENT_RECENT,
        name: "RecentAccents",
        default: RecentAccents::DEFAULT.bits(),
        valid: |v| RecentAccents::from_bits(v).is_some(),
    },
];
pub fn find(id: u32) -> Option<&'static Preference> {
    ALL.iter().find(|p| p.id == id)
}
/// `value` is a valid word for preference `id`.
pub fn valid(id: u32, value: u32) -> bool {
    find(id).is_some_and(|p| (p.valid)(value))
}
/// The clean-Windows default of preference `id` (0 for unknown ids).
pub fn default(id: u32) -> u32 {
    find(id).map_or(0, |p| p.default)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_are_valid_and_ids_unique() {
        for (i, p) in ALL.iter().enumerate() {
            assert!((p.valid)(p.default), "{}", p.name);
            assert!(ALL[..i].iter().all(|q| q.id != p.id && q.name != p.name));
            assert!(p.id >= 10);
        }
        assert_eq!(default(NOTIFICATIONS), notify::ON | notify::SOUNDS);
        assert_eq!(default(SEARCH_HISTORY), 1);
        assert_eq!(default(END_TASK), 0);
        assert!(!valid(99, 0));
    }
    #[test]
    fn schedules_round_trip_and_span_midnight() {
        let s = ThemeSchedule {
            enabled: true,
            light_from: 420,
            dark_from: 1140,
        };
        assert_eq!(ThemeSchedule::from_bits(s.bits()), Some(s));
        assert!(s.dark_at(1200) && s.dark_at(60) && !s.dark_at(600));
        assert!(ThemeSchedule::from_bits(1440 << 1).is_none());
        let d = DndSchedule::DEFAULT;
        assert_eq!(DndSchedule::from_bits(d.bits()), Some(d));
        assert!(d.active(23 * 60, 3) && d.active(60, 3) && !d.active(12 * 60, 3));
        let weekends = DndSchedule { repeat: 2, ..d };
        // Saturday 01:00 started on Friday night: not a weekend span.
        assert!(!weekends.active(60, 6) && weekends.active(60, 0));
        assert!(DndSchedule::from_bits(3 << 22).is_none());
    }
    #[test]
    fn focus_lock_screen_and_overrides_validate() {
        assert_eq!(
            Focus::from_bits(Focus::DEFAULT.bits()),
            Some(Focus::DEFAULT)
        );
        assert!(Focus::from_bits(31).is_none());
        assert_eq!(LockScreen::picture(2).unwrap().kind(), 3);
        assert_eq!(LockScreen::solid(0x123456).rgb(), 0x123456);
        assert!(LockScreen::from_bits(1 | 1 << 8).is_none());
        let o = RegionalOverrides {
            clock: 2,
            first_day: 2,
            short_date: 3,
            long_date: 1,
            short_time: 4,
            long_time: 15,
        };
        assert_eq!(RegionalOverrides::from_bits(o.bits()), Some(o));
        assert!(RegionalOverrides::from_bits(3).is_none());
        assert!(valid(SIGN_IN_AWAY, 15) && !valid(SIGN_IN_AWAY, 2));
    }
    #[test]
    fn recent_accents_keep_five_distinct_colors_most_recent_first() {
        let r = RecentAccents::DEFAULT;
        assert_eq!(RecentAccents::from_bits(r.bits()), Some(r));
        assert!(r.colors().eq([16]));
        let r = r.used(9).used(30).used(16);
        assert_eq!(r.0, [16, 30, 9, 63, 63]);
        let full = r.used(1).used(2).used(3);
        assert_eq!(full.0, [3, 2, 1, 16, 30], "the oldest falls off");
        assert_eq!(RecentAccents::from_bits(full.bits()), Some(full));
        assert!(RecentAccents::from_bits(0).is_none(), "duplicates");
        assert!(RecentAccents::from_bits(RecentAccents([48, 63, 63, 63, 63]).bits()).is_none());
        assert!(RecentAccents::from_bits(RecentAccents([63, 1, 63, 63, 63]).bits()).is_none());
        assert!(valid(ACCENT_RECENT, RecentAccents::DEFAULT.bits()));
    }
    #[test]
    fn recent_backgrounds_move_the_used_picture_first() {
        let r = RecentBackgrounds::DEFAULT;
        assert_eq!(RecentBackgrounds::from_bits(r.bits()), Some(r));
        assert_eq!(r.used(7).0, [7, 0, 1, 2, 3]);
        assert_eq!(r.used(0), r);
        assert_eq!(r.used(17).0, [17, 0, 1, 2, 7]);
        assert!(RecentBackgrounds::from_bits(0).is_none(), "duplicates");
        assert!(RecentBackgrounds::from_bits(RecentBackgrounds([18, 0, 1, 2, 3]).bits()).is_none());
    }
}
