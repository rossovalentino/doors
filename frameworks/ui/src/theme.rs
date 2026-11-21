//! Immutable theme snapshots resolved from semantic design tokens (master
//! specification sections 55 and 113). Controls never hardcode colours: they
//! read roles from the snapshot. Light and dark values are the Windows 11
//! (WinUI) theme resources as already calibrated for Door's Settings app;
//! contrast themes map the same roles onto the session's system colours
//! (`door_service_protocol::desktop::ContrastTheme::colors`, owned by the
//! desktop-controls stream). Colours are premultiplied BGRA8.
use door_graphics_text::TextStyle;
use door_service_protocol::desktop::{ContrastTheme, Controls, SystemColors};

pub type Color = [u8; 4];
/// Opaque colour from RGB.
pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
    [b, g, r, 255]
}
/// Premultiplied colour from straight RGB and alpha.
pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    const fn m(c: u8, a: u8) -> u8 {
        ((c as u32 * a as u32 + 127) / 255) as u8
    }
    [m(b, a), m(g, a), m(r, a), a]
}
/// Scale a premultiplied colour's opacity by `alpha` / 255.
pub const fn fade(c: Color, alpha: u8) -> Color {
    const fn m(c: u8, a: u8) -> u8 {
        ((c as u32 * a as u32 + 127) / 255) as u8
    }
    [
        m(c[0], alpha),
        m(c[1], alpha),
        m(c[2], alpha),
        m(c[3], alpha),
    ]
}
const fn sys(c: [u8; 3]) -> Color {
    rgb(c[0], c[1], c[2])
}

/// Semantic colour roles (WinUI theme resource names in brackets).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Colors {
    /// Window background [SolidBackgroundFillColorBase / Mica fallback].
    pub background: Color,
    /// Card and layer surface [CardBackgroundFillColorDefault].
    pub card: Color,
    pub card_hover: Color,
    pub card_pressed: Color,
    pub card_stroke: Color,
    pub divider: Color,
    /// [TextFillColorPrimary/Secondary/Disabled].
    pub text: Color,
    pub text_secondary: Color,
    pub text_disabled: Color,
    /// Text on accent fills [TextOnAccentFillColorPrimary].
    pub text_on_accent: Color,
    pub text_on_accent_disabled: Color,
    /// [AccentFillColorDefault/Secondary/Tertiary/Disabled].
    pub accent: Color,
    pub accent_hover: Color,
    pub accent_pressed: Color,
    pub accent_disabled: Color,
    /// Accent-coloured text and glyphs [AccentTextFillColorPrimary].
    pub accent_text: Color,
    /// Standard control fill [ControlFillColorDefault/Secondary/Tertiary/Disabled].
    pub control: Color,
    pub control_hover: Color,
    pub control_pressed: Color,
    pub control_disabled: Color,
    /// Text on a hovered/pressed control (differs only in contrast themes).
    pub control_text_hover: Color,
    /// [ControlStrokeColorDefault] and its elevated bottom edge
    /// [ControlStrokeColorSecondary].
    pub control_stroke: Color,
    pub control_stroke_bottom: Color,
    /// Check box / switch / radio outlines and knobs [ControlStrongStrokeColorDefault].
    pub strong: Color,
    pub strong_disabled: Color,
    /// Unchecked check box fill [ControlAltFillColorSecondary].
    pub control_alt: Color,
    pub control_alt_hover: Color,
    /// Text field focused background [TextControlBackgroundFocused].
    pub field_focused: Color,
    /// List/menu/navigation row plates [SubtleFillColorSecondary/Tertiary].
    pub subtle_hover: Color,
    pub subtle_pressed: Color,
    /// Selected row plate.
    pub subtle_selected: Color,
    /// Flyouts, menus and combo drop-downs.
    pub flyout: Color,
    pub flyout_stroke: Color,
    /// [FocusStrokeColorOuter/Inner].
    pub focus_outer: Color,
    pub focus_inner: Color,
    /// Dialog backdrop [SmokeFillColorDefault].
    pub smoke: Color,
    pub tooltip: Color,
    pub tooltip_stroke: Color,
    pub scrollbar: Color,
    /// Hyperlinks (Hotlight in contrast themes).
    pub link: Color,
    /// InfoBar severities: foreground icon fill and background.
    pub critical: Color,
    pub critical_background: Color,
    pub caution: Color,
    pub caution_background: Color,
    pub success: Color,
    pub success_background: Color,
    pub attention: Color,
    pub attention_background: Color,
}

/// Named typography roles (WinUI type ramp). Sizes come from the hinted
/// atlas styles available today; text scaling multiplies them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextRole {
    /// 12/16.
    Caption,
    /// 14/20.
    Body,
    /// 14/20 semibold.
    BodyStrong,
    /// 16/24 (WinUI BodyLarge is 18/24; nearest available face).
    BodyLarge,
    BodyLargeStrong,
    /// 20/28 semibold.
    Subtitle,
    /// 28/36 semibold.
    Title,
}
impl TextRole {
    pub const ALL: [Self; 7] = [
        Self::Caption,
        Self::Body,
        Self::BodyStrong,
        Self::BodyLarge,
        Self::BodyLargeStrong,
        Self::Subtitle,
        Self::Title,
    ];
    pub const fn style(self) -> TextStyle {
        match self {
            Self::Caption => TextStyle::Regular12,
            Self::Body => TextStyle::Regular14,
            Self::BodyStrong => TextStyle::Semibold14,
            Self::BodyLarge => TextStyle::Regular16,
            Self::BodyLargeStrong => TextStyle::Semibold16,
            Self::Subtitle => TextStyle::Semibold20,
            Self::Title => TextStyle::Semibold28,
        }
    }
}

/// Spacing, sizing and shape tokens in whole DIPs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Metrics {
    pub control_height: i32,
    pub control_radius: i32,
    pub overlay_radius: i32,
    pub spacing_xs: i32,
    pub spacing_s: i32,
    pub spacing_m: i32,
    pub spacing_l: i32,
    pub spacing_xl: i32,
    /// Focus visual: primary (outer) and secondary (inner) thickness and
    /// how far it sits outside the control [FocusVisualMargin -3].
    pub focus_outer: i32,
    pub focus_inner: i32,
    pub focus_margin: i32,
    pub list_row_height: i32,
    pub menu_row_height: i32,
}
pub const METRICS: Metrics = Metrics {
    control_height: 32,
    control_radius: 4,
    overlay_radius: 8,
    spacing_xs: 4,
    spacing_s: 8,
    spacing_m: 12,
    spacing_l: 16,
    spacing_xl: 24,
    focus_outer: 2,
    focus_inner: 1,
    focus_margin: 3,
    list_row_height: 36,
    menu_row_height: 32,
};

/// Motion durations in milliseconds; all zero under reduced motion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Motion {
    pub fast: u32,
    pub normal: u32,
    pub slow: u32,
    /// ToolTipService initial show delay.
    pub tooltip_delay: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Variant {
    Light,
    Dark,
    /// A contrast theme (`ContrastTheme as u32`); `dark` tells whether its
    /// window colour is dark.
    HighContrast {
        theme: u32,
        dark: bool,
    },
}

/// One immutable resolved theme. Changing any setting produces a new
/// snapshot; the toolkit invalidates paint (and layout when text size
/// changed) without discarding widget state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Theme {
    pub variant: Variant,
    pub colors: Colors,
    pub metrics: Metrics,
    pub motion: Motion,
    /// Accessibility > Text size, percent (100..=225).
    pub text_scale: u32,
    pub reduced_motion: bool,
    /// Effects off: flyouts and dialogs use opaque fills, no backdrop smoke
    /// transparency beyond the dim.
    pub reduced_transparency: bool,
}

const LIGHT: Colors = Colors {
    background: [250, 247, 244, 255],
    card: [253, 252, 251, 255],
    card_hover: [249, 248, 246, 255],
    card_pressed: [246, 245, 243, 255],
    card_stroke: [233, 230, 228, 255],
    divider: [235, 234, 233, 255],
    text: [26, 26, 26, 255],
    text_secondary: [96, 96, 96, 255],
    text_disabled: [160, 160, 160, 255],
    text_on_accent: [255, 255, 255, 255],
    text_on_accent_disabled: [255, 255, 255, 255],
    accent: [203, 103, 0, 255],
    accent_hover: [212, 118, 25, 255],
    accent_pressed: [184, 94, 4, 255],
    accent_disabled: [196, 196, 196, 255],
    accent_text: [184, 94, 0, 255],
    control: [253, 253, 253, 255],
    control_hover: [249, 249, 249, 255],
    control_pressed: [245, 245, 245, 255],
    control_disabled: [245, 245, 245, 255],
    control_text_hover: [26, 26, 26, 255],
    control_stroke: [229, 229, 229, 255],
    control_stroke_bottom: [205, 205, 205, 255],
    strong: [128, 128, 128, 255],
    strong_disabled: [196, 196, 196, 255],
    control_alt: [246, 246, 246, 255],
    control_alt_hover: [238, 238, 238, 255],
    field_focused: [255, 255, 255, 255],
    subtle_hover: [242, 238, 235, 255],
    subtle_pressed: [239, 235, 232, 255],
    subtle_selected: [237, 233, 230, 255],
    flyout: [252, 252, 252, 255],
    flyout_stroke: [220, 220, 220, 255],
    focus_outer: [26, 26, 26, 255],
    focus_inner: [255, 255, 255, 255],
    smoke: [0, 0, 0, 77],
    tooltip: [249, 249, 249, 255],
    tooltip_stroke: [220, 220, 220, 255],
    scrollbar: [138, 138, 138, 255],
    link: [184, 94, 0, 255],
    critical: rgb(0xc4, 0x2b, 0x1c),
    critical_background: rgb(0xfd, 0xe7, 0xe9),
    caution: rgb(0x9d, 0x5d, 0x00),
    caution_background: rgb(0xff, 0xf4, 0xce),
    success: rgb(0x0f, 0x7b, 0x0f),
    success_background: rgb(0xdf, 0xf6, 0xdd),
    attention: rgb(0x00, 0x5f, 0xb7),
    attention_background: rgb(0xf6, 0xf6, 0xf6),
};
const DARK: Colors = Colors {
    background: [36, 32, 31, 255],
    card: [47, 44, 43, 255],
    card_hover: [54, 51, 50, 255],
    card_pressed: [43, 40, 39, 255],
    card_stroke: [30, 27, 26, 255],
    divider: [32, 29, 28, 255],
    text: [255, 255, 255, 255],
    text_secondary: [207, 206, 206, 255],
    text_disabled: [119, 118, 118, 255],
    text_on_accent: [1, 0, 0, 255],
    text_on_accent_disabled: [135, 135, 135, 255],
    accent: [255, 205, 96, 255],
    accent_hover: [240, 192, 88, 255],
    accent_pressed: [224, 178, 84, 255],
    accent_disabled: [86, 85, 85, 255],
    accent_text: [255, 205, 96, 255],
    control: [59, 56, 55, 255],
    control_hover: [65, 62, 61, 255],
    control_pressed: [52, 49, 48, 255],
    control_disabled: [50, 47, 46, 255],
    control_text_hover: [255, 255, 255, 255],
    control_stroke: [70, 67, 66, 255],
    control_stroke_bottom: [82, 79, 78, 255],
    strong: [166, 165, 165, 255],
    strong_disabled: [86, 85, 85, 255],
    control_alt: [41, 38, 37, 255],
    control_alt_hover: [52, 49, 48, 255],
    field_focused: [30, 30, 30, 255],
    subtle_hover: [45, 41, 40, 255],
    subtle_pressed: [41, 37, 36, 255],
    subtle_selected: [52, 48, 46, 255],
    flyout: [48, 45, 44, 255],
    flyout_stroke: [24, 22, 21, 255],
    focus_outer: [255, 255, 255, 255],
    focus_inner: [0, 0, 0, 255],
    smoke: [0, 0, 0, 77],
    tooltip: [44, 44, 44, 255],
    tooltip_stroke: [24, 22, 21, 255],
    scrollbar: [155, 154, 154, 255],
    link: [255, 205, 96, 255],
    critical: rgb(0xff, 0x99, 0xa4),
    critical_background: rgb(0x44, 0x27, 0x26),
    caution: rgb(0xfc, 0xe1, 0x00),
    caution_background: rgb(0x43, 0x35, 0x19),
    success: rgb(0x6c, 0xcb, 0x5f),
    success_background: rgb(0x39, 0x3d, 0x1b),
    attention: rgb(0x60, 0xcd, 0xff),
    attention_background: rgb(0x2e, 0x2e, 0x2e),
};
/// Contrast themes: every role maps onto a system colour, as WinUI's
/// HighContrast theme dictionary does. No translucency.
const fn contrast(s: SystemColors) -> Colors {
    let window = sys(s.window);
    let text = sys(s.window_text);
    let hi = sys(s.highlight);
    let hi_text = sys(s.highlight_text);
    let gray = sys(s.gray_text);
    let face = sys(s.button_face);
    let link = sys(s.hotlight);
    Colors {
        background: window,
        card: window,
        card_hover: window,
        card_pressed: window,
        card_stroke: text,
        divider: text,
        text,
        text_secondary: text,
        text_disabled: gray,
        text_on_accent: hi_text,
        text_on_accent_disabled: window,
        accent: hi,
        accent_hover: hi,
        accent_pressed: hi,
        accent_disabled: gray,
        accent_text: link,
        control: face,
        control_hover: hi,
        control_pressed: hi,
        control_disabled: window,
        control_text_hover: hi_text,
        control_stroke: text,
        control_stroke_bottom: text,
        strong: text,
        strong_disabled: gray,
        control_alt: window,
        control_alt_hover: window,
        field_focused: window,
        subtle_hover: hi,
        subtle_pressed: hi,
        subtle_selected: hi,
        flyout: window,
        flyout_stroke: text,
        focus_outer: text,
        focus_inner: window,
        smoke: [0, 0, 0, 0],
        tooltip: window,
        tooltip_stroke: text,
        scrollbar: text,
        link,
        critical: text,
        critical_background: window,
        caution: text,
        caution_background: window,
        success: text,
        success_background: window,
        attention: text,
        attention_background: window,
    }
}

impl Theme {
    const MOTION: Motion = Motion {
        fast: 83,
        normal: 167,
        slow: 250,
        tooltip_delay: 400,
    };
    pub const fn light() -> Self {
        Self {
            variant: Variant::Light,
            colors: LIGHT,
            metrics: METRICS,
            motion: Self::MOTION,
            text_scale: 100,
            reduced_motion: false,
            reduced_transparency: false,
        }
    }
    pub const fn dark() -> Self {
        Self {
            variant: Variant::Dark,
            colors: DARK,
            ..Self::light()
        }
    }
    pub const fn high_contrast(theme: ContrastTheme) -> Self {
        Self {
            variant: Variant::HighContrast {
                theme: theme as u32,
                dark: theme.dark(),
            },
            colors: contrast(theme.colors()),
            reduced_transparency: true,
            ..Self::light()
        }
    }
    /// The theme a session's desktop controls ask for: dark/light app mode,
    /// contrast theme, text size, animation and transparency effects.
    pub fn from_controls(dark: bool, controls: Controls) -> Self {
        let base = match controls.contrast() {
            Some(theme) => Self::high_contrast(theme),
            None if dark => Self::dark(),
            None => Self::light(),
        };
        base.with_text_scale(controls.text_percent())
            .with_reduced_motion(controls.reduce_motion())
            .with_reduced_transparency(controls.reduce_transparency() || base.is_high_contrast())
    }
    pub const fn with_text_scale(mut self, percent: u32) -> Self {
        self.text_scale = if percent < 100 {
            100
        } else if percent > 225 {
            225
        } else {
            percent
        };
        self
    }
    pub const fn with_reduced_motion(mut self, on: bool) -> Self {
        self.reduced_motion = on;
        self.motion = if on {
            Motion {
                fast: 0,
                normal: 0,
                slow: 0,
                tooltip_delay: Self::MOTION.tooltip_delay,
            }
        } else {
            Self::MOTION
        };
        self
    }
    pub const fn with_reduced_transparency(mut self, on: bool) -> Self {
        self.reduced_transparency = on;
        self
    }
    /// Replace the accent ramp (personalization accent colour, light themes
    /// derive hover/pressed by darkening).
    pub fn with_accent(mut self, accent: Color) -> Self {
        if self.is_high_contrast() {
            return self;
        }
        let shade = |c: Color, f: u32| {
            [
                (c[0] as u32 * f / 100) as u8,
                (c[1] as u32 * f / 100) as u8,
                (c[2] as u32 * f / 100) as u8,
                c[3],
            ]
        };
        self.colors.accent = accent;
        self.colors.accent_hover = shade(accent, 92);
        self.colors.accent_pressed = shade(accent, 84);
        self.colors.accent_text = if self.is_dark() {
            accent
        } else {
            shade(accent, 90)
        };
        self.colors.link = self.colors.accent_text;
        self
    }
    pub const fn is_dark(&self) -> bool {
        match self.variant {
            Variant::Dark => true,
            Variant::HighContrast { dark, .. } => dark,
            Variant::Light => false,
        }
    }
    pub const fn is_high_contrast(&self) -> bool {
        matches!(self.variant, Variant::HighContrast { .. })
    }
    /// Stable fingerprint of every token, for snapshot identity and caches.
    pub fn fingerprint(&self) -> u64 {
        use core::hash::{Hash, Hasher};
        let mut h = crate::display::Fnv::default();
        self.hash(&mut h);
        h.finish()
    }
    /// Short name used in fixtures: light, dark, hc-night-sky, ...
    pub fn name(&self) -> &'static str {
        match self.variant {
            Variant::Light => "light",
            Variant::Dark => "dark",
            Variant::HighContrast { theme, .. } => match ContrastTheme::from_index(theme) {
                ContrastTheme::Aquatic => "hc-aquatic",
                ContrastTheme::Desert => "hc-desert",
                ContrastTheme::Dusk => "hc-dusk",
                ContrastTheme::NightSky => "hc-night-sky",
            },
        }
    }
}
impl Default for Theme {
    fn default() -> Self {
        Self::light()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshots_resolve_every_variant_from_tokens() {
        let l = Theme::light();
        let d = Theme::dark();
        assert!(!l.is_dark() && d.is_dark());
        assert_ne!(l.fingerprint(), d.fingerprint());
        for theme in ContrastTheme::ALL {
            let h = Theme::high_contrast(theme);
            let s = theme.colors();
            assert_eq!(h.colors.background, sys(s.window));
            assert_eq!(h.colors.text, sys(s.window_text));
            assert_eq!(h.colors.accent, sys(s.highlight));
            assert_eq!(h.colors.text_on_accent, sys(s.highlight_text));
            assert_eq!(h.colors.text_disabled, sys(s.gray_text));
            assert_eq!(h.is_dark(), theme.dark());
            assert!(h.reduced_transparency);
            // No translucent roles except the transparent smoke.
            assert!(h.colors.card[3] == 255 && h.colors.flyout[3] == 255);
        }
        let c = Controls::DEFAULT.with_text_percent(150);
        let t = Theme::from_controls(true, c);
        assert!(t.is_dark());
        assert_eq!(t.text_scale, 150);
        let t = Theme::from_controls(false, c.with_contrast(Some(ContrastTheme::Desert)));
        assert!(t.is_high_contrast() && !t.is_dark());
        assert_eq!(Theme::light().with_text_scale(400).text_scale, 225);
        let r = Theme::light().with_reduced_motion(true);
        assert_eq!((r.motion.fast, r.motion.normal), (0, 0));
        assert_eq!(rgba(255, 0, 0, 128), [0, 0, 128, 128]);
    }
}
