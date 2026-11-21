//! Checked session desktop preferences shared by clients, storage and shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Edge {
    Bottom = 0,
    Top = 1,
    Left = 2,
    Right = 3,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Controls(u32);
impl Controls {
    pub const DEFAULT: Self = Self((1 << 4) | (1 << 5));
    /// Every bit is defined. Bits 7..=8 hold a Start layout and bits 13..=14
    /// a Start size (3 is invalid in both fields); bits 25..=31 hold the text
    /// size step (at most `TEXT_STEP_MAX`).
    pub const fn from_bits(bits: u32) -> Option<Self> {
        if (bits >> 7) & 3 != 3
            && (bits >> 13) & 3 != 3
            && bits >> Self::TEXT_SHIFT <= Self::TEXT_STEP_MAX
        {
            Some(Self(bits))
        } else {
            None
        }
    }
    /// Accessibility > Contrast themes: the selected theme (bits 22..=23, a
    /// `ContrastTheme`) is kept while contrast is off, so Left Alt+Left
    /// Shift+Print Screen turns the last one back on. Night sky is 0.
    pub const CONTRAST_THEME_SHIFT: u32 = 22;
    pub const CONTRAST_ON: u32 = 1 << 24;
    /// Accessibility > Text size: 100% plus the step (0..=125, so 100..=225%),
    /// applied to text only.
    pub const TEXT_SHIFT: u32 = 25;
    pub const TEXT_STEP_MAX: u32 = 125;
    /// Personalization > Themes > Desktop icon settings. Off by default, as on
    /// a clean Windows 11 desktop.
    pub const SHOW_DESKTOP_ICONS: u32 = 1 << 6;
    pub const START_MORE_PINS: u32 = 1 << 7;
    pub const START_MORE_RECOMMENDATIONS: u32 = 2 << 7;
    /// Personalization > Colors > Transparency effects off.
    pub const REDUCE_TRANSPARENCY: u32 = 1 << 9;
    /// Personalization > Context menu: open the full classic menu directly.
    pub const CLASSIC_CONTEXT_MENU: u32 = 1 << 10;
    /// Personalization > Taskbar > Search: icon only instead of a search box.
    pub const SEARCH_ICON_ONLY: u32 = 1 << 11;
    /// Accessibility > Visual effects > Animation effects off.
    pub const REDUCE_MOTION: u32 = 1 << 12;
    /// Personalization > Start > Start size (26H2): 0 Automatic, 1 Small,
    /// 2 Large.
    pub const START_SIZE_SMALL: u32 = 1 << 13;
    pub const START_SIZE_LARGE: u32 = 2 << 13;
    /// Personalization > Start section toggles (26H2). Each bit hides its
    /// section, so the Windows default (all shown) stays zero.
    pub const START_HIDE_PINNED: u32 = 1 << 15;
    pub const START_HIDE_RECENT: u32 = 1 << 16;
    pub const START_HIDE_ALL: u32 = 1 << 17;
    /// Personalization > Start: hide the account name and picture.
    pub const START_HIDE_ACCOUNT: u32 = 1 << 18;
    /// Personalization > Start: no file recommendations in Recent; recently
    /// installed apps stay.
    pub const START_HIDE_RECENT_FILES: u32 = 1 << 19;
    /// Personalization > Context menu > File actions (Send to, Open with).
    pub const CONTEXT_HIDE_FILE_ACTIONS: u32 = 1 << 20;
    /// Personalization > Context menu > Show more options.
    pub const CONTEXT_HIDE_MORE_OPTIONS: u32 = 1 << 21;
    pub const fn show_desktop_icons(self) -> bool {
        self.0 & Self::SHOW_DESKTOP_ICONS != 0
    }
    /// 0 default, 1 more pins, 2 more recommendations.
    pub const fn start_layout(self) -> u32 {
        (self.0 >> 7) & 3
    }
    pub const fn reduce_transparency(self) -> bool {
        self.0 & Self::REDUCE_TRANSPARENCY != 0
    }
    pub const fn classic_context_menu(self) -> bool {
        self.0 & Self::CLASSIC_CONTEXT_MENU != 0
    }
    pub const fn search_icon_only(self) -> bool {
        self.0 & Self::SEARCH_ICON_ONLY != 0
    }
    pub const fn reduce_motion(self) -> bool {
        self.0 & Self::REDUCE_MOTION != 0
    }
    /// 0 Automatic, 1 Small, 2 Large.
    pub const fn start_size(self) -> u32 {
        (self.0 >> 13) & 3
    }
    /// The active contrast theme, if contrast is on.
    pub const fn contrast(self) -> Option<ContrastTheme> {
        if self.0 & Self::CONTRAST_ON != 0 {
            Some(self.contrast_theme())
        } else {
            None
        }
    }
    /// The selected contrast theme (also while contrast is off).
    pub const fn contrast_theme(self) -> ContrastTheme {
        ContrastTheme::from_index((self.0 >> Self::CONTRAST_THEME_SHIFT) & 3)
    }
    /// Turn contrast on with `theme`, or off (`None`) keeping the selection.
    pub const fn with_contrast(self, theme: Option<ContrastTheme>) -> Self {
        match theme {
            Some(theme) => Self(
                (self.0 & !(3 << Self::CONTRAST_THEME_SHIFT))
                    | ((theme as u32) << Self::CONTRAST_THEME_SHIFT)
                    | Self::CONTRAST_ON,
            ),
            None => Self(self.0 & !Self::CONTRAST_ON),
        }
    }
    /// Left Alt+Left Shift+Print Screen: the selected theme on, or off.
    pub const fn toggle_contrast(self) -> Self {
        Self(self.0 ^ Self::CONTRAST_ON)
    }
    /// Text size in percent (100..=225).
    pub const fn text_percent(self) -> u32 {
        100 + (self.0 >> Self::TEXT_SHIFT)
    }
    /// The text size `percent`, clamped to 100..=225.
    pub const fn with_text_percent(self, percent: u32) -> Self {
        let step = if percent < 100 {
            0
        } else if percent - 100 > Self::TEXT_STEP_MAX {
            Self::TEXT_STEP_MAX
        } else {
            percent - 100
        };
        Self((self.0 & ((1 << Self::TEXT_SHIFT) - 1)) | (step << Self::TEXT_SHIFT))
    }
    pub const fn has(self, flag: u32) -> bool {
        self.0 & flag != 0
    }
    pub const fn with(self, flag: u32, on: bool) -> Self {
        Self(if on { self.0 | flag } else { self.0 & !flag })
    }
    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn edge(self) -> Edge {
        match self.0 & 3 {
            1 => Edge::Top,
            2 => Edge::Left,
            3 => Edge::Right,
            _ => Edge::Bottom,
        }
    }
    pub const fn start_aligned(self) -> bool {
        self.0 & 4 != 0
    }
    pub const fn small(self) -> bool {
        self.0 & 8 != 0
    }
    pub const fn inline_actions(self) -> bool {
        self.0 & 16 != 0
    }
    pub const fn properties_bottom(self) -> bool {
        self.0 & 32 != 0
    }
}
impl Default for Controls {
    fn default() -> Self {
        Self::DEFAULT
    }
}
/// The system accent colour setting (Settings > Personalization > Colors >
/// Accent color), stored as the user's `AccentColor` word: bits 24..25 the
/// mode, bits 0..23 the manual colour 0xRRGGBB. Every other bit is zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Accent(u32);
/// How the accent colour is chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AccentMode {
    /// Door's default blue (the Windows 11 default accent).
    Default,
    /// "Manual": a Windows colour or a custom colour, 0xRRGGBB.
    Manual(u32),
    /// "Automatic": picked from the desktop background, again whenever it
    /// changes.
    Automatic,
}
impl Accent {
    pub const DEFAULT: Self = Self(0);
    pub const AUTOMATIC: Self = Self(2 << 24);
    pub const fn manual(rgb: u32) -> Self {
        Self(1 << 24 | (rgb & 0x00ff_ffff))
    }
    pub const fn from_bits(bits: u32) -> Option<Self> {
        match bits >> 24 {
            0 | 2 if bits & 0x00ff_ffff == 0 => Some(Self(bits)),
            1 => Some(Self(bits)),
            _ => None,
        }
    }
    pub const fn bits(self) -> u32 {
        self.0
    }
    pub const fn mode(self) -> AccentMode {
        match self.0 >> 24 {
            1 => AccentMode::Manual(self.0 & 0x00ff_ffff),
            2 => AccentMode::Automatic,
            _ => AccentMode::Default,
        }
    }
    /// The APPEARANCE_CHANGED `a` word for a resolved accent 0xRRGGBB.
    pub const fn event_word(rgb: u32) -> u32 {
        ACCENT_PRESENT | (rgb & 0x00ff_ffff)
    }
    /// The resolved accent 0xRRGGBB an APPEARANCE_CHANGED `a` word carries
    /// (None from servers that send no accent: use the default blue).
    pub const fn from_event_word(a: u32) -> Option<u32> {
        if a & ACCENT_PRESENT != 0 {
            Some(a & 0x00ff_ffff)
        } else {
            None
        }
    }
}
impl Default for Accent {
    fn default() -> Self {
        Self::DEFAULT
    }
}
const ACCENT_PRESENT: u32 = 1 << 24;
/// Windows 11 contrast themes (Settings > Accessibility > Contrast themes).
/// The discriminant is the stored value: Night sky, Windows' former High
/// Contrast Black, is 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ContrastTheme {
    NightSky = 0,
    Aquatic = 1,
    Desert = 2,
    Dusk = 3,
}
/// A contrast theme's system colours (RGB): the Win32 colour roles every
/// contrast-aware surface draws with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemColors {
    /// Background of windows and content.
    pub window: [u8; 3],
    /// Text on `window`, and borders.
    pub window_text: [u8; 3],
    /// Selected or focused item background.
    pub highlight: [u8; 3],
    /// Text on `highlight`.
    pub highlight_text: [u8; 3],
    pub button_face: [u8; 3],
    pub button_text: [u8; 3],
    /// Disabled text.
    pub gray_text: [u8; 3],
    /// Hyperlinks.
    pub hotlight: [u8; 3],
}
impl ContrastTheme {
    /// Windows' Settings list order (after "None").
    pub const ALL: [Self; 4] = [Self::Aquatic, Self::Desert, Self::Dusk, Self::NightSky];
    pub const fn from_index(index: u32) -> Self {
        match index & 3 {
            1 => Self::Aquatic,
            2 => Self::Desert,
            3 => Self::Dusk,
            _ => Self::NightSky,
        }
    }
    pub const fn name(self) -> &'static str {
        match self {
            Self::Aquatic => "Aquatic",
            Self::Desert => "Desert",
            Self::Dusk => "Dusk",
            Self::NightSky => "Night sky",
        }
    }
    /// Desert is the only light contrast theme.
    pub const fn dark(self) -> bool {
        !matches!(self, Self::Desert)
    }
    /// The Windows 11 theme colours.
    pub const fn colors(self) -> SystemColors {
        match self {
            Self::Aquatic => SystemColors {
                window: [0x20, 0x20, 0x20],
                window_text: [0xff, 0xff, 0xff],
                highlight: [0x8e, 0xe3, 0xf0],
                highlight_text: [0x26, 0x3b, 0x50],
                button_face: [0x20, 0x20, 0x20],
                button_text: [0xff, 0xff, 0xff],
                gray_text: [0xa6, 0xa6, 0xa6],
                hotlight: [0x75, 0xe9, 0xfc],
            },
            Self::Desert => SystemColors {
                window: [0xff, 0xfa, 0xef],
                window_text: [0x3d, 0x3d, 0x3d],
                highlight: [0x90, 0x39, 0x09],
                highlight_text: [0xff, 0xf5, 0xe3],
                button_face: [0xff, 0xfa, 0xef],
                button_text: [0x3d, 0x3d, 0x3d],
                gray_text: [0x67, 0x67, 0x67],
                hotlight: [0x1c, 0x5e, 0x75],
            },
            Self::Dusk => SystemColors {
                window: [0x2d, 0x32, 0x36],
                window_text: [0xff, 0xff, 0xff],
                highlight: [0xa1, 0xbf, 0xde],
                highlight_text: [0x21, 0x2d, 0x3b],
                button_face: [0x2d, 0x32, 0x36],
                button_text: [0xb6, 0xf6, 0xf0],
                gray_text: [0xa6, 0xa6, 0xa6],
                hotlight: [0x70, 0xeb, 0xde],
            },
            Self::NightSky => SystemColors {
                window: [0x00, 0x00, 0x00],
                window_text: [0xff, 0xff, 0xff],
                highlight: [0xd6, 0xb4, 0xfd],
                highlight_text: [0x2b, 0x2b, 0x2b],
                button_face: [0x00, 0x00, 0x00],
                button_text: [0xff, 0xee, 0x32],
                gray_text: [0xa6, 0xa6, 0xa6],
                hotlight: [0x80, 0x80, 0xff],
            },
        }
    }
}
/// Per-user shell layout persisted beside `DesktopControls` as two dwords
/// (registry values `ShellLayout` and `ShellOrder`). App bits/positions index
/// `application::DESKTOP_COMMANDS`.
///
/// `flags`: bits 0..=5 taskbar-unpinned mask, 6..=11 Start-unpinned mask,
/// 12..=13 Start All view (0 Category, 1 Grid, 2 List; 3 invalid), 14 desktop
/// icons sorted by name; bits 15..=31 must be zero.
///
/// `order`: bits 0..=9 Start pin order and 10..=19 taskbar pin order, each the
/// Lehmer rank (< 720) of a permutation of the six apps (pinned apps first, in
/// pin order, then the rest ascending); bits 20..=31 must be zero.
///
/// Both zero is the Windows default: every app pinned in catalog order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShellLayout {
    flags: u32,
    order: u32,
}
const SHELL_APPS: usize = crate::application::DESKTOP_APPS;
const PERMUTATIONS: u32 = 720;
impl ShellLayout {
    pub const DEFAULT: Self = Self { flags: 0, order: 0 };
    pub const SORTED: u32 = 1 << 14;
    pub const fn flags_valid(flags: u32) -> bool {
        flags >> 15 == 0 && (flags >> 12) & 3 != 3
    }
    pub const fn order_valid(order: u32) -> bool {
        order >> 20 == 0 && order & 0x3ff < PERMUTATIONS && (order >> 10) & 0x3ff < PERMUTATIONS
    }
    pub const fn from_parts(flags: u32, order: u32) -> Option<Self> {
        if Self::flags_valid(flags) && Self::order_valid(order) {
            Some(Self { flags, order })
        } else {
            None
        }
    }
    pub const fn flags(self) -> u32 {
        self.flags
    }
    pub const fn order(self) -> u32 {
        self.order
    }
    /// 0 Category, 1 Grid, 2 List.
    pub const fn view(self) -> u32 {
        (self.flags >> 12) & 3
    }
    pub const fn sorted(self) -> bool {
        self.flags & Self::SORTED != 0
    }
    /// Build from ordered pin lists of app indices (unique, < 6).
    pub fn new(taskbar: &[usize], start: &[usize], view: u32, sorted: bool) -> Option<Self> {
        if view > 2 {
            return None;
        }
        let (taskbar_mask, taskbar_rank) = encode(taskbar)?;
        let (start_mask, start_rank) = encode(start)?;
        Self::from_parts(
            taskbar_mask | (start_mask << 6) | (view << 12) | if sorted { Self::SORTED } else { 0 },
            start_rank | (taskbar_rank << 10),
        )
    }
    /// Taskbar pins in order: (indices, count).
    pub fn taskbar_pins(self) -> ([usize; SHELL_APPS], usize) {
        decode(self.flags & 0x3f, (self.order >> 10) & 0x3ff)
    }
    /// Start pins in order: (indices, count).
    pub fn start_pins(self) -> ([usize; SHELL_APPS], usize) {
        decode((self.flags >> 6) & 0x3f, self.order & 0x3ff)
    }
}
fn encode(pins: &[usize]) -> Option<(u32, u32)> {
    if pins.len() > SHELL_APPS {
        return None;
    }
    let mut perm = [0u8; SHELL_APPS];
    let mut used = 0u32;
    for (slot, &app) in pins.iter().enumerate() {
        if app >= SHELL_APPS || used & (1 << app) != 0 {
            return None;
        }
        used |= 1 << app;
        perm[slot] = app as u8;
    }
    let mut slot = pins.len();
    for app in 0..SHELL_APPS {
        if used & (1 << app) == 0 {
            perm[slot] = app as u8;
            slot += 1;
        }
    }
    let mask = !used & ((1 << SHELL_APPS) - 1);
    Some((mask, rank(perm)))
}
fn decode(unpinned: u32, rank: u32) -> ([usize; SHELL_APPS], usize) {
    let perm = unrank(rank);
    let mut out = [0usize; SHELL_APPS];
    let mut count = 0;
    for app in perm {
        if unpinned & (1 << app) == 0 {
            out[count] = app as usize;
            count += 1;
        }
    }
    (out, count)
}
/// Lehmer rank of a permutation of 0..6.
fn rank(perm: [u8; SHELL_APPS]) -> u32 {
    let mut r = 0u32;
    for i in 0..SHELL_APPS {
        let smaller = perm[i + 1..].iter().filter(|&&p| p < perm[i]).count() as u32;
        r = r * (SHELL_APPS - i) as u32 + smaller;
    }
    r
}
fn unrank(mut r: u32) -> [u8; SHELL_APPS] {
    let mut digits = [0u32; SHELL_APPS];
    for i in (0..SHELL_APPS).rev() {
        let base = (SHELL_APPS - i) as u32;
        digits[i] = r % base;
        r /= base;
    }
    let mut pool = [0u8, 1, 2, 3, 4, 5];
    let mut left = SHELL_APPS;
    let mut out = [0u8; SHELL_APPS];
    for (i, d) in digits.iter().enumerate() {
        let d = (*d as usize).min(left - 1);
        out[i] = pool[d];
        pool.copy_within(d + 1..left, d);
        left -= 1;
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_accepted_bitfield_has_exact_semantics_and_unknown_bits_fail() {
        assert_eq!(Controls::DEFAULT.bits(), 48);
        for bits in 0..64 {
            let c = Controls::from_bits(bits).unwrap();
            assert_eq!(c.bits(), bits);
            assert_eq!(c.edge() as u32, bits & 3);
            assert_eq!(c.start_aligned(), bits & 4 != 0);
            assert_eq!(c.small(), bits & 8 != 0);
            assert_eq!(c.inline_actions(), bits & 16 != 0);
            assert_eq!(c.properties_bottom(), bits & 32 != 0);
        }
        // Contrast theme and on bit, then the text size step (at most 125).
        for bit in 22..=24 {
            assert!(Controls::from_bits(48 | (1 << bit)).is_some());
        }
        assert_eq!(
            Controls::from_bits(48 | (125 << 25))
                .unwrap()
                .text_percent(),
            225
        );
        assert_eq!(Controls::from_bits(48 | (126 << 25)), None);
        assert_eq!(
            Controls::from_bits(48 | (3 << 7)),
            None,
            "Start layout 3 is undefined"
        );
        assert_eq!(
            Controls::from_bits(48 | (3 << 13)),
            None,
            "Start size 3 is undefined"
        );
        let s = Controls::from_bits(Controls::START_SIZE_LARGE | Controls::START_HIDE_ALL).unwrap();
        assert_eq!(s.start_size(), 2);
        assert!(s.has(Controls::START_HIDE_ALL) && !s.has(Controls::START_HIDE_PINNED));
        let c = Controls::from_bits(
            48 | Controls::START_MORE_RECOMMENDATIONS | Controls::REDUCE_MOTION,
        )
        .unwrap();
        assert_eq!(c.start_layout(), 2);
        assert!(c.reduce_motion() && !c.show_desktop_icons() && !c.reduce_transparency());
        assert!(
            c.with(Controls::SHOW_DESKTOP_ICONS, true)
                .show_desktop_icons()
        );
    }
    #[test]
    fn contrast_and_text_size_fields_round_trip_without_touching_other_bits() {
        let base = Controls::from_bits(48 | Controls::REDUCE_MOTION).unwrap();
        assert_eq!(base.contrast(), None);
        assert_eq!(base.contrast_theme(), ContrastTheme::NightSky);
        assert_eq!(base.text_percent(), 100);
        for theme in ContrastTheme::ALL {
            let on = base.with_contrast(Some(theme));
            assert_eq!(on.contrast(), Some(theme));
            assert!(on.reduce_motion() && on.inline_actions());
            // Off keeps the selection; the shortcut toggles it back on.
            let off = on.with_contrast(None);
            assert_eq!((off.contrast(), off.contrast_theme()), (None, theme));
            assert_eq!(off.toggle_contrast(), on);
            assert_eq!(Controls::from_bits(on.bits()), Some(on));
        }
        for (percent, stored) in [(100, 100), (150, 150), (225, 225), (99, 100), (300, 225)] {
            let c = base
                .with_contrast(Some(ContrastTheme::Dusk))
                .with_text_percent(percent);
            assert_eq!(c.text_percent(), stored);
            assert_eq!(c.contrast(), Some(ContrastTheme::Dusk));
            assert_eq!(Controls::from_bits(c.bits()), Some(c));
        }
        assert_eq!(Controls::DEFAULT.text_percent(), 100);
    }
    /// WCAG contrast ratio of two colours.
    fn ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
        extern crate std;
        let l = |c: [u8; 3]| {
            let lin = |v: u8| {
                let v = f64::from(v) / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    std::primitive::f64::powf((v + 0.055) / 1.055, 2.4)
                }
            };
            0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
        };
        let (x, y) = (l(a), l(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }
    #[test]
    fn contrast_themes_keep_windows_text_contrast() {
        for theme in ContrastTheme::ALL {
            let c = theme.colors();
            for (name, fg, bg) in [
                ("text", c.window_text, c.window),
                ("button", c.button_text, c.button_face),
                ("selection", c.highlight_text, c.highlight),
            ] {
                // 7:1, as Windows publishes them (Desert's selection pair rounds
                // to 7.00:1 from its 8-bit colours).
                assert!(
                    ratio(fg, bg) >= 6.995,
                    "{theme:?} {name}: {}",
                    ratio(fg, bg)
                );
            }
            // Links and disabled text stay readable (WCAG AA).
            assert!(ratio(c.hotlight, c.window) >= 4.5, "{theme:?} link");
            assert!(ratio(c.gray_text, c.window) >= 4.5, "{theme:?} disabled");
            assert_eq!(theme.dark(), theme != ContrastTheme::Desert);
            assert_eq!(ContrastTheme::from_index(theme as u32), theme);
        }
    }
    #[test]
    fn shell_layout_round_trips_pins_order_view_and_rejects_invalid_values() {
        let d = ShellLayout::DEFAULT;
        assert_eq!(d.taskbar_pins(), ([0, 1, 2, 3, 4, 5], 6));
        assert_eq!(d.start_pins(), ([0, 1, 2, 3, 4, 5], 6));
        assert_eq!((d.view(), d.sorted()), (0, false));
        assert_eq!(
            ShellLayout::new(&[0, 1, 2, 3, 4, 5], &[0, 1, 2, 3, 4, 5], 0, false),
            Some(d)
        );
        let l = ShellLayout::new(&[5, 0, 3], &[2, 4, 1, 0], 2, true).unwrap();
        let (t, n) = l.taskbar_pins();
        assert_eq!(&t[..n], &[5, 0, 3]);
        let (s, n) = l.start_pins();
        assert_eq!(&s[..n], &[2, 4, 1, 0]);
        assert_eq!((l.view(), l.sorted()), (2, true));
        assert_eq!(ShellLayout::from_parts(l.flags(), l.order()), Some(l));
        assert_eq!(
            ShellLayout::new(&[], &[], 1, false)
                .unwrap()
                .taskbar_pins()
                .1,
            0
        );
        // Every permutation rank round-trips.
        for r in 0..720 {
            assert_eq!(rank(unrank(r)), r);
        }
        assert_eq!(ShellLayout::new(&[1, 1], &[], 0, false), None);
        assert_eq!(ShellLayout::new(&[6], &[], 0, false), None);
        assert_eq!(ShellLayout::new(&[], &[], 3, false), None);
        assert_eq!(ShellLayout::from_parts(1 << 15, 0), None);
        assert_eq!(ShellLayout::from_parts(3 << 12, 0), None);
        assert_eq!(ShellLayout::from_parts(0, 720), None);
        assert_eq!(ShellLayout::from_parts(0, 720 << 10), None);
        assert_eq!(ShellLayout::from_parts(0, 1 << 20), None);
    }
}
#[cfg(test)]
mod accent_tests {
    use super::*;
    #[test]
    fn accent_words_round_trip_and_reject_stray_bits() {
        assert_eq!(Accent::from_bits(0), Some(Accent::DEFAULT));
        assert_eq!(Accent::DEFAULT.mode(), AccentMode::Default);
        assert_eq!(Accent::AUTOMATIC.mode(), AccentMode::Automatic);
        let owner = Accent::manual(0xA94DC1);
        assert_eq!(owner.mode(), AccentMode::Manual(0xA94DC1));
        assert_eq!(Accent::from_bits(owner.bits()), Some(owner));
        for bad in [1, 2 << 24 | 5, 3 << 24, 1 << 26, u32::MAX] {
            assert_eq!(Accent::from_bits(bad), None, "{bad:x}");
        }
        let word = Accent::event_word(0xDB9EE5);
        assert_eq!(Accent::from_event_word(word), Some(0xDB9EE5));
        assert_eq!(Accent::from_event_word(0), None);
        assert_ne!(crate::window::SET_ACCENT, crate::window::SET_RESPONDING);
    }
}
