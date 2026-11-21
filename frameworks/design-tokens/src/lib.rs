#![no_std]
//! Door semantic design tokens (master specification sections 111 and 140.3).
//!
//! One allocation-free registry of colour, typography, spacing/dimension,
//! radius, elevation/shadow and motion tokens. Every token carries a light,
//! dark and high-contrast value plus its provenance: the section 111 source
//! kind (`measured_reference`, `public_specification`, `bootstrap_proposal`,
//! `approved_deviation`), the capture or document it came from, which themes
//! were actually measured, and whether the value is still unresolved.
//!
//! The frozen reference is `profiles/windows-reference/reference-26h2-x64.json`
//! (Windows 11 Pro 26H2, OS build 26300.9711, 2560x1600 at 144 DPI = 150 %,
//! dark mode, accent #680081). Lengths are logical units at the 96-DPI
//! reference scale (physical / 1.5 for the 150 % captures). Colours are
//! premultiplied BGRA8, the pixel format of `door-graphics-core` and `door-ui`.
//!
//! Consumers migrate from local constants (`desktop/workspace/src/tokens.rs`,
//! window-server palettes, `door-ui` theme snapshots) through the typed
//! accessors: [`color`], [`spacing`], [`radius`], [`type_ramp`],
//! [`elevation`] and [`motion`]; generic tools use [`tokens`] and [`lookup`].

#[cfg(test)]
extern crate std;

/// Premultiplied BGRA8.
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

/// The three appearance variants every token resolves for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Theme {
    Light,
    Dark,
    /// The default Windows 11 contrast theme (Aquatic); other contrast themes
    /// remap the same roles from their system colours.
    HighContrast,
}
impl Theme {
    pub const ALL: [Self; 3] = [Self::Light, Self::Dark, Self::HighContrast];
    const fn bit(self) -> u8 {
        match self {
            Self::Light => 1,
            Self::Dark => 2,
            Self::HighContrast => 4,
        }
    }
}

/// Token category (section 140.3 categories grouped as the registry stores them).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Color,
    Typography,
    /// Spacing increments and measured component dimensions.
    Spacing,
    Radius,
    /// Elevation and shadow extents.
    Elevation,
    Motion,
}

/// Section 111 source kinds. Release gates reject `BootstrapProposal` tokens
/// in a claimed surface set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceKind {
    MeasuredReference,
    PublicSpecification,
    BootstrapProposal,
    ApprovedDeviation,
}
impl SourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MeasuredReference => "measured_reference",
            Self::PublicSpecification => "public_specification",
            Self::BootstrapProposal => "bootstrap_proposal",
            Self::ApprovedDeviation => "approved_deviation",
        }
    }
}

/// How far a value can be trusted for a fidelity claim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Confidence {
    /// Measured from the frozen reference capture.
    Measured,
    /// Taken from a published specification (WinUI resources, type ramp).
    Specified,
    /// Provisional development value; must be replaced before a claim.
    Provisional,
    /// Measured, but the reference value depends on an uncontrolled input
    /// (acrylic over the owner's wallpaper): reported, never pass/fail.
    Uncontrolled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Provenance {
    pub source_kind: SourceKind,
    /// Capture path (relative to the repository) or document reference.
    pub source: &'static str,
    /// Themes whose value was measured from a capture (`Theme` bits).
    pub measured_themes: u8,
    pub confidence: Confidence,
}
impl Provenance {
    /// Whether `theme`'s value came from a reference capture.
    pub const fn measured(&self, theme: Theme) -> bool {
        self.measured_themes & theme.bit() != 0
    }
    /// A bootstrap proposal or provisional value: not usable for a claim.
    pub const fn unresolved(&self) -> bool {
        matches!(self.source_kind, SourceKind::BootstrapProposal)
            || matches!(self.confidence, Confidence::Provisional)
    }
}
const fn measured(source: &'static str, themes: u8) -> Provenance {
    Provenance {
        source_kind: SourceKind::MeasuredReference,
        source,
        measured_themes: themes,
        confidence: Confidence::Measured,
    }
}
const fn uncontrolled(source: &'static str, themes: u8) -> Provenance {
    Provenance {
        source_kind: SourceKind::MeasuredReference,
        source,
        measured_themes: themes,
        confidence: Confidence::Uncontrolled,
    }
}
const fn spec(source: &'static str) -> Provenance {
    Provenance {
        source_kind: SourceKind::PublicSpecification,
        source,
        measured_themes: 0,
        confidence: Confidence::Specified,
    }
}
const fn bootstrap(source: &'static str) -> Provenance {
    Provenance {
        source_kind: SourceKind::BootstrapProposal,
        source,
        measured_themes: 0,
        confidence: Confidence::Provisional,
    }
}
const DARK: u8 = 2;

/// Font roles of the reference type ramp (Segoe UI Variable optical sizes on
/// the reference; Door substitutes are an approved deviation recorded in the
/// reference profile).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontRole {
    Small,
    Text,
    Display,
    Monospace,
}

/// One type-ramp entry in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TypeStyle {
    pub role: FontRole,
    pub size: u16,
    pub line_height: u16,
    /// OpenType weight class (400 regular, 600 semibold).
    pub weight: u16,
}
const fn ty(role: FontRole, size: u16, line_height: u16, weight: u16) -> TypeStyle {
    TypeStyle {
        role,
        size,
        line_height,
        weight,
    }
}

/// Elevation: WinUI ThemeShadow depth plus the measured shadow extents (logical
/// px beyond the surface edge on each side) and the darkest shadow colour
/// directly outside the bottom edge over a white backdrop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Elevation {
    pub depth: u16,
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
    pub color: Color,
}
const fn el(depth: u16, [left, top, right, bottom]: [u16; 4], color: Color) -> Elevation {
    Elevation {
        depth,
        left,
        top,
        right,
        bottom,
        color,
    }
}
/// Contrast themes draw borders, not shadows.
const NO_SHADOW: Elevation = el(0, [0; 4], [0; 4]);

/// Easing curves; control points in thousandths.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Easing {
    Linear,
    CubicBezier { x1: u16, y1: u16, x2: u16, y2: u16 },
}
impl Easing {
    /// Eased progress in thousandths at time `t` (thousandths of the
    /// duration, clamped): the curve's y where its x equals `t`.
    pub const fn sample(self, t: u32) -> u32 {
        let t = if t > 1000 { 1000 } else { t };
        match self {
            Easing::Linear => t,
            Easing::CubicBezier { x1, y1, x2, y2 } => {
                // Bisection on the curve parameter s (millionths).
                let target = t as u128 * 1000;
                let (mut lo, mut hi) = (0u128, 1_000_000u128);
                let mut i = 0;
                while i < 40 {
                    let mid = (lo + hi) / 2;
                    if bezier(x1, x2, mid) < target {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                    i += 1;
                }
                (bezier(y1, y2, hi) / 1000) as u32
            }
        }
    }
}
/// One cubic Bezier coordinate from 0 to 1000 (end points 0 and 1), control
/// values in thousandths, at `s` millionths; the result in millionths.
const fn bezier(p1: u16, p2: u16, s: u128) -> u128 {
    let u = 1_000_000 - s;
    (3 * u * u * s * p1 as u128 / 1000 + 3 * u * s * s * p2 as u128 / 1000 + s * s * s)
        / 1_000_000_000_000
}
/// WinUI ControlFastOutSlowInKeySpline (0,0,0,1).
pub const FAST_OUT_SLOW_IN: Easing = Easing::CubicBezier {
    x1: 0,
    y1: 0,
    x2: 0,
    y2: 1000,
};
/// Accelerating exit curve (1,0,1,1).
pub const ACCELERATE: Easing = Easing::CubicBezier {
    x1: 1000,
    y1: 0,
    x2: 1000,
    y2: 1000,
};
/// Fluent "point to point" curve (0.55,0.55,0,1): a surface moving between
/// two resting places (snap, maximize).
pub const POINT_TO_POINT: Easing = Easing::CubicBezier {
    x1: 550,
    y1: 550,
    x2: 0,
    y2: 1000,
};
/// Strong deceleration (0.1,0.9,0.2,1; Fluent 2 `curveDecelerateMax`): the
/// shell's slide-in for Start and the tray flyouts.
pub const DECELERATE_MAX: Easing = Easing::CubicBezier {
    x1: 100,
    y1: 900,
    x2: 200,
    y2: 1000,
};
/// Soft acceleration (0.7,0,1,0.5): the shell's slide-out for Start and the
/// tray flyouts.
pub const ACCELERATE_SOFT: Easing = Easing::CubicBezier {
    x1: 700,
    y1: 0,
    x2: 1000,
    y2: 500,
};

/// A named motion track: duration and easing. Reduced motion makes it zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Motion {
    pub duration_ms: u16,
    pub easing: Easing,
}
const fn mo(duration_ms: u16, easing: Easing) -> Motion {
    Motion {
        duration_ms,
        easing,
    }
}

/// A resolved token value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Value {
    Color(Color),
    /// Logical pixels at the 96-DPI reference scale.
    Length(u16),
    Type(TypeStyle),
    Elevation(Elevation),
    Motion(Motion),
}

/// Unit of a token's value (section 140.3 `unit`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unit {
    PremultipliedBgra8,
    LogicalPx,
    TypeRamp,
    Shadow,
    Milliseconds,
}
impl Unit {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PremultipliedBgra8 => "premultiplied BGRA8",
            Self::LogicalPx => "logical px (96 DPI)",
            Self::TypeRamp => "logical px size/line height, weight class",
            Self::Shadow => "ThemeShadow depth, logical px extents, BGRA8",
            Self::Milliseconds => "ms",
        }
    }
}

/// One registry entry of a typed table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Entry<T> {
    pub id: &'static str,
    pub light: T,
    pub dark: T,
    pub contrast: T,
    pub provenance: Provenance,
    /// Controls and surfaces the token affects.
    pub controls: &'static str,
}
impl<T: Copy> Entry<T> {
    pub const fn get(&self, theme: Theme) -> T {
        match theme {
            Theme::Light => self.light,
            Theme::Dark => self.dark,
            Theme::HighContrast => self.contrast,
        }
    }
}

/// A token of any kind, as the generic registry API presents it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Token {
    pub id: &'static str,
    pub kind: Kind,
    pub unit: Unit,
    pub light: Value,
    pub dark: Value,
    pub contrast: Value,
    pub provenance: Provenance,
    pub controls: &'static str,
}
impl Token {
    pub const fn value(&self, theme: Theme) -> Value {
        match theme {
            Theme::Light => self.light,
            Theme::Dark => self.dark,
            Theme::HighContrast => self.contrast,
        }
    }
}

macro_rules! table {
    (
        $(#[$meta:meta])*
        $name:ident: $ty:ty, $table:ident {
            $($variant:ident = $id:literal ($light:expr, $dark:expr, $contrast:expr, $provenance:expr, $controls:literal);)*
        }
    ) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum $name {
            $($variant,)*
        }
        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];
            pub const fn id(self) -> &'static str {
                $table[self as usize].id
            }
            pub const fn entry(self) -> &'static Entry<$ty> {
                &$table[self as usize]
            }
        }
        pub const $table: &[Entry<$ty>] = &[
            $(Entry {
                id: $id,
                light: $light,
                dark: $dark,
                contrast: $contrast,
                provenance: $provenance,
                controls: $controls,
            },)*
        ];
    };
}

const WINUI_COLORS: &str = "WinUI theme resources (Common_themeresources_any.xaml) as calibrated in frameworks/ui/src/theme.rs";
/// Source of every `Theme::HighContrast` colour value.
pub const CONTRAST_SOURCE: &str = "Windows 11 Aquatic contrast theme system colours (door-service-protocol ContrastTheme::Aquatic)";
const PALETTE: &str =
    "reports/artifacts/reference-26h2/environment.txt AccentPalette (Light3..Dark3)";
const HC_WINDOW: Color = rgb(0x20, 0x20, 0x20);
const HC_TEXT: Color = rgb(0xff, 0xff, 0xff);
const HC_HIGHLIGHT: Color = rgb(0x8e, 0xe3, 0xf0);
const HC_HIGHLIGHT_TEXT: Color = rgb(0x26, 0x3b, 0x50);
const HC_GRAY: Color = rgb(0xa6, 0xa6, 0xa6);
const HC_HOTLIGHT: Color = rgb(0x75, 0xe9, 0xfc);

table! {
    /// Semantic colour roles.
    ColorToken: Color, COLORS {
        TextPrimary = "color.text.primary" (rgb(0x1a, 0x1a, 0x1a), rgb(0xff, 0xff, 0xff), HC_TEXT, spec(WINUI_COLORS), "body text, labels, titles");
        TextSecondary = "color.text.secondary" (rgb(0x60, 0x60, 0x60), rgb(0xce, 0xce, 0xcf), HC_TEXT, spec(WINUI_COLORS), "descriptions, secondary labels");
        TextDisabled = "color.text.disabled" (rgb(0xa0, 0xa0, 0xa0), rgb(0x76, 0x76, 0x77), HC_GRAY, spec(WINUI_COLORS), "disabled controls (exempt from 4.5:1)");
        TextOnAccent = "color.text.on_accent" (rgb(0xff, 0xff, 0xff), rgb(0x00, 0x00, 0x00), HC_HIGHLIGHT_TEXT, spec(WINUI_COLORS), "accent buttons, selected toggles");
        AccentDefault = "color.accent.default" (rgb(0x8e, 0x3a, 0xa7), rgb(0xdb, 0x9e, 0xe5), HC_HIGHLIGHT, measured("reports/artifacts/reference-26h2/settings-home.png toggle (2176,558) and environment.txt AccentPalette", DARK), "accent fills, toggles, selection indicator");
        AccentText = "color.accent.text" (rgb(0x69, 0x27, 0x82), rgb(0xf0, 0xc0, 0xf4), HC_HOTLIGHT, spec(PALETTE), "links, accent text");
        SurfaceWindow = "color.surface.window" (rgb(0xf3, 0xf3, 0xf3), rgb(0x20, 0x20, 0x20), HC_WINDOW, spec("WinUI SolidBackgroundFillColorBase (Mica fallback)"), "window background without Mica");
        SurfaceContent = "color.surface.content" (rgb(0xff, 0xff, 0xff), rgb(0x19, 0x19, 0x19), HC_WINDOW, measured("reports/artifacts/reference-26h2/explorer-home.png (900,600)", DARK), "File Explorer item view");
        SurfaceCaption = "color.surface.caption" (rgb(0xf3, 0xf3, 0xf3), rgb(0x2b, 0x2b, 0x2b), HC_WINDOW, measured("reports/artifacts/reference-26h2/explorer-home.png (VIS-01 probe explorer.caption, dominant colour)", DARK), "title and tab bars");
        SurfaceCard = "color.surface.card" (rgb(0xfb, 0xfc, 0xfd), rgb(0x2b, 0x2c, 0x2f), HC_WINDOW, spec(WINUI_COLORS), "Settings cards and rows");
        SurfaceFlyout = "color.surface.flyout" (rgb(0xf9, 0xf9, 0xf9), rgb(0x29, 0x29, 0x29), HC_WINDOW, measured("reports/artifacts/reference-26h2/shell-start-screen.png (VIS-01 probe start.background, dominant colour)", DARK), "Start, menus, flyouts (acrylic fallback)");
        SurfaceSearchField = "color.surface.search_field" (rgb(0xff, 0xff, 0xff), rgb(0x1f, 0x1f, 0x1f), HC_WINDOW, measured("reports/artifacts/reference-26h2/shell-start-screen.png (1664,261)", DARK), "Start search box");
        SurfaceTaskbar = "color.surface.taskbar" (rgb(0xee, 0xee, 0xee), rgb(0x1b, 0x23, 0x22), HC_WINDOW, uncontrolled("reports/artifacts/reference-26h2/taskbar-strip.png (VIS-01 probe taskbar.background): acrylic over the owner's wallpaper", DARK), "taskbar background");
        SurfaceNotepadEditor = "color.surface.notepad_editor" (rgb(0xff, 0xff, 0xff), rgb(0x27, 0x27, 0x27), HC_WINDOW, measured("reports/artifacts/reference-26h2/notepad-window.png (900,600)", DARK), "Notepad-like editor document area");
        SurfaceTerminal = "color.surface.terminal" (rgb(0x0c, 0x0c, 0x0c), rgb(0x0c, 0x0c, 0x0c), rgb(0x0c, 0x0c, 0x0c), measured("reports/artifacts/reference-26h2/terminal-window.png (900,600) (Campbell scheme)", DARK), "Terminal buffer background");
        FillControl = "color.fill.control" (rgb(0xfd, 0xfd, 0xfd), rgb(0x37, 0x38, 0x3b), HC_WINDOW, spec(WINUI_COLORS), "standard buttons, combo boxes");
        FillControlHover = "color.fill.control_hover" (rgb(0xf9, 0xf9, 0xf9), rgb(0x3d, 0x3e, 0x41), HC_HIGHLIGHT, spec(WINUI_COLORS), "hovered standard controls");
        FillControlPressed = "color.fill.control_pressed" (rgb(0xf5, 0xf5, 0xf5), rgb(0x30, 0x31, 0x34), HC_HIGHLIGHT, spec(WINUI_COLORS), "pressed standard controls");
        FillSubtleHover = "color.fill.subtle_hover" (rgb(0xeb, 0xee, 0xf2), rgb(0x28, 0x29, 0x2d), HC_HIGHLIGHT, spec(WINUI_COLORS), "list, menu and navigation row hover");
        FillSubtleSelected = "color.fill.subtle_selected" (rgb(0xe6, 0xe9, 0xed), rgb(0x2b, 0x2e, 0x30), HC_HIGHLIGHT, measured("reports/artifacts/reference-26h2/settings-home.png (320,243) selected Home item", DARK), "selected navigation row");
        StrokeControl = "color.stroke.control" (rgb(0xe5, 0xe5, 0xe5), rgb(0x42, 0x43, 0x46), HC_TEXT, spec(WINUI_COLORS), "control borders");
        StrokeDivider = "color.stroke.divider" (rgb(0xe9, 0xea, 0xeb), rgb(0x1c, 0x1d, 0x20), HC_TEXT, spec(WINUI_COLORS), "dividers, card separators");
        StrokeFocusOuter = "color.stroke.focus_outer" (rgb(0x1a, 0x1a, 0x1a), rgb(0xff, 0xff, 0xff), HC_TEXT, spec("WinUI FocusStrokeColorOuter"), "keyboard focus visual (outer)");
        StrokeFocusInner = "color.stroke.focus_inner" (rgb(0xff, 0xff, 0xff), rgb(0x00, 0x00, 0x00), HC_WINDOW, spec("WinUI FocusStrokeColorInner"), "keyboard focus visual (inner)");
        StatusCritical = "color.status.critical" (rgb(0xc4, 0x2b, 0x1c), rgb(0xff, 0x99, 0xa4), HC_TEXT, spec(WINUI_COLORS), "errors, sign-in failure message");
        StatusCaution = "color.status.caution" (rgb(0x9d, 0x5d, 0x00), rgb(0xfc, 0xe1, 0x00), HC_TEXT, spec(WINUI_COLORS), "warnings");
        StatusSuccess = "color.status.success" (rgb(0x0f, 0x7b, 0x0f), rgb(0x6c, 0xcb, 0x5f), HC_TEXT, spec(WINUI_COLORS), "success states");
        StatusAttention = "color.status.attention" (rgb(0x00, 0x5f, 0xb7), rgb(0x60, 0xcd, 0xff), HC_TEXT, spec(WINUI_COLORS), "informational states");
        Smoke = "color.smoke" (rgba(0, 0, 0, 77), rgba(0, 0, 0, 77), [0, 0, 0, 0], spec("WinUI SmokeFillColorDefault"), "dialog backdrop dim");
    }
}

const RAMP: &str = "WinUI type ramp (learn.microsoft.com/windows/apps/design/style/typography)";
table! {
    /// Type ramp roles.
    TypeToken: TypeStyle, TYPE_RAMP {
        Caption = "type.caption" (ty(FontRole::Small, 12, 16, 400), ty(FontRole::Small, 12, 16, 400), ty(FontRole::Small, 12, 16, 400), spec(RAMP), "captions, taskbar clock lines");
        Body = "type.body" (ty(FontRole::Text, 14, 20, 400), ty(FontRole::Text, 14, 20, 400), ty(FontRole::Text, 14, 20, 400), spec(RAMP), "body text, navigation items");
        BodyStrong = "type.body_strong" (ty(FontRole::Text, 14, 20, 600), ty(FontRole::Text, 14, 20, 600), ty(FontRole::Text, 14, 20, 600), spec(RAMP), "Start section headers, emphasized labels");
        BodyLarge = "type.body_large" (ty(FontRole::Text, 18, 24, 400), ty(FontRole::Text, 18, 24, 400), ty(FontRole::Text, 18, 24, 400), spec(RAMP), "large body text");
        Subtitle = "type.subtitle" (ty(FontRole::Display, 20, 28, 600), ty(FontRole::Display, 20, 28, 600), ty(FontRole::Display, 20, 28, 600), spec(RAMP), "card titles, dialog titles");
        Title = "type.title" (ty(FontRole::Display, 28, 36, 600), ty(FontRole::Display, 28, 36, 600), ty(FontRole::Display, 28, 36, 600), spec(RAMP), "Settings page titles, sign-in account name");
        TitleLarge = "type.title_large" (ty(FontRole::Display, 40, 52, 600), ty(FontRole::Display, 40, 52, 600), ty(FontRole::Display, 40, 52, 600), spec(RAMP), "large titles");
        Display = "type.display" (ty(FontRole::Display, 68, 92, 600), ty(FontRole::Display, 68, 92, 600), ty(FontRole::Display, 68, 92, 600), spec(RAMP), "lock-screen clock");
        Monospace = "type.monospace" (ty(FontRole::Monospace, 14, 20, 400), ty(FontRole::Monospace, 14, 20, 400), ty(FontRole::Monospace, 14, 20, 400), bootstrap("Cascadia Mono 12 pt default of the reference Terminal; size unmeasured"), "Terminal buffer");
    }
}

const SPACING: &str = "master specification 140.3 bootstrap spacing increments";
const TASKBAR: &str = "reports/artifacts/reference-26h2/taskbar.uia.tsv (physical @150 % / 1.5)";
const SETTINGS: &str =
    "reports/artifacts/reference-26h2/settings-home.uia.tsv (physical @150 % / 1.5)";
table! {
    /// Spacing increments and measured component dimensions (logical px).
    SpacingToken: u16, SPACING_TABLE {
        Xxs = "space.xxs" (2, 2, 2, bootstrap(SPACING), "hairline gaps");
        Xs = "space.xs" (4, 4, 4, bootstrap(SPACING), "icon-label gaps");
        S = "space.s" (8, 8, 8, bootstrap(SPACING), "control padding");
        M = "space.m" (12, 12, 12, bootstrap(SPACING), "control padding");
        L = "space.l" (16, 16, 16, bootstrap(SPACING), "card padding");
        Xl = "space.xl" (24, 24, 24, bootstrap(SPACING), "section spacing");
        Xxl = "space.xxl" (32, 32, 32, bootstrap(SPACING), "page margins");
        ControlHeight = "size.control_height" (32, 32, 32, spec("WinUI standard control height"), "buttons, text boxes, combo boxes");
        TaskbarHeight = "size.taskbar_height" (48, 48, 48, measured(TASKBAR, DARK), "taskbar (72 px @150 %)");
        TaskbarButton = "size.taskbar_button" (44, 44, 44, measured(TASKBAR, DARK), "taskbar app buttons (66 px @150 %)");
        TaskbarStartButton = "size.taskbar_start_button" (45, 45, 45, measured(TASKBAR, DARK), "Start button (68 px @150 %, 45.3 logical)");
        TaskbarIcon = "size.taskbar_icon" (24, 24, 24, measured(TASKBAR, DARK), "taskbar glyph and app icons (36 px @150 %)");
        TaskbarStartGlyph = "size.taskbar_start_glyph" (23, 23, 23, measured("reports/artifacts/reference-26h2/taskbar-strip.png (VIS-01 probe taskbar.start_glyph: 35x35 px ink @150 %, 23.3 logical)", DARK), "Start button mark ink height (door mask: draw the 24-unit grid at 28 logical px)");
        TrayShowDesktop = "size.tray_show_desktop" (12, 12, 12, measured(TASKBAR, DARK), "Show desktop sliver (18 px @150 %)");
        TrayClockWidth = "size.tray_clock_width" (70, 70, 70, measured(TASKBAR, DARK), "clock button, two-line en-US time (105 px @150 %)");
        CaptionButtonWidth = "size.caption_button_width" (46, 46, 46, measured(SETTINGS, DARK), "minimize/maximize/close (69 px @150 %)");
        CaptionHeight = "size.caption_height" (32, 32, 32, measured(SETTINGS, DARK), "title bar (48 px @150 %)");
        NavPaneWidth = "size.nav_pane_width" (300, 300, 300, measured(SETTINGS, DARK), "Settings navigation pane (450 px @150 %)");
        NavItemHeight = "size.nav_item_height" (36, 36, 36, measured(SETTINGS, DARK), "navigation items (54 px @150 %)");
        StartWidth = "size.start_width" (826, 826, 826, measured("reports/artifacts/reference-26h2/shell-start-screen.png (VIS-01 probe start.width: interior run, borders excluded)", DARK), "Start menu interior, default size, category view");
    }
}

const GEOMETRY: &str = "learn.microsoft.com/windows/apps/design/signature-experiences/geometry";
table! {
    /// Corner radii (logical px).
    RadiusToken: u16, RADII {
        Control = "radius.control" (4, 4, 4, spec(GEOMETRY), "buttons, text boxes, list items");
        Overlay = "radius.overlay" (8, 8, 8, spec(GEOMETRY), "flyouts, menus, Start, dialogs");
        Window = "radius.window" (8, 8, 8, spec(GEOMETRY), "top-level windows (not maximized or snapped)");
        Bar = "radius.bar" (2, 2, 2, bootstrap("selection and progress indicator pills; unmeasured"), "navigation selection indicator, taskbar running indicator");
    }
}

const SHADOW: &str = "WinUI ThemeShadow depths; extents unmeasured";
table! {
    /// Elevation and shadows.
    ElevationToken: Elevation, ELEVATIONS {
        Card = "elevation.card" (el(0, [0; 4], [0; 4]), el(0, [0; 4], [0; 4]), NO_SHADOW, spec("WinUI cards use strokes, not shadows"), "Settings cards");
        Tooltip = "elevation.tooltip" (el(16, [8, 8, 8, 8], rgba(0, 0, 0, 40)), el(16, [8, 8, 8, 8], rgba(0, 0, 0, 80)), NO_SHADOW, bootstrap(SHADOW), "tooltips");
        Flyout = "elevation.flyout" (el(32, [16, 16, 16, 16], rgba(0, 0, 0, 56)), el(32, [16, 16, 16, 16], rgba(0, 0, 0, 96)), NO_SHADOW, bootstrap(SHADOW), "menus, Start, flyouts");
        WindowInactive = "elevation.window_inactive" (el(32, [31, 13, 31, 69], rgb(0xc9, 0xc9, 0xc9)), el(32, [31, 13, 31, 69], rgb(0xc9, 0xc9, 0xc9)), NO_SHADOW, measured("reports/artifacts/reference-26h2/photos-shadow-white.png (47/20/47/104 px @150 % over #FFFFFF)", DARK), "inactive top-level windows");
        WindowActive = "elevation.window_active" (el(64, [40, 28, 41, 77], rgb(0x6b, 0x6b, 0x6b)), el(64, [40, 28, 41, 77], rgb(0x6b, 0x6b, 0x6b)), NO_SHADOW, measured("reports/artifacts/reference-26h2/explorer-shadow-active-white.png (60/42/62/116 px @150 % over #FFFFFF)", DARK), "active top-level window");
        Dialog = "elevation.dialog" (el(128, [32, 32, 32, 32], rgba(0, 0, 0, 72)), el(128, [32, 32, 32, 32], rgba(0, 0, 0, 120)), NO_SHADOW, bootstrap(SHADOW), "content dialogs, trusted prompts");
    }
}

const WINUI_MOTION: &str =
    "WinUI ControlFaster/Fast/NormalAnimationDuration and ControlFastOutSlowInKeySpline";
const UNMEASURED_TRACK: &str = "shell track not yet captured from the reference (section 111 motion capture); curve and duration step from the Fluent motion guidance (learn.microsoft.com/windows/apps/design/motion/timing-and-easing, Fluent 2 motion tokens)";
table! {
    /// Named motion tracks.
    MotionToken: Motion, MOTIONS {
        Faster = "motion.faster" (mo(83, FAST_OUT_SLOW_IN), mo(83, FAST_OUT_SLOW_IN), mo(83, FAST_OUT_SLOW_IN), spec(WINUI_MOTION), "hover and press feedback");
        Fast = "motion.fast" (mo(167, FAST_OUT_SLOW_IN), mo(167, FAST_OUT_SLOW_IN), mo(167, FAST_OUT_SLOW_IN), spec(WINUI_MOTION), "toggles, small transitions");
        Normal = "motion.normal" (mo(250, FAST_OUT_SLOW_IN), mo(250, FAST_OUT_SLOW_IN), mo(250, FAST_OUT_SLOW_IN), spec(WINUI_MOTION), "page and flyout transitions");
        StartOpen = "motion.start_open" (mo(167, FAST_OUT_SLOW_IN), mo(167, FAST_OUT_SLOW_IN), mo(167, FAST_OUT_SLOW_IN), measured("reports/artifacts/reference-26h2/motion/start-open/sequence.json: Windows' Start rises from below the screen edge on the fast-out-slow-in spline (0,0,0,1) without a fade; the offsets tracked in runs 2 and 3 fit 171 ms and about 850 epx (rms 3 physical px), 167 ms being the Fluent duration step; the visible span is 163.7 ms (VIS-01 start.open)", DARK), "Start rise from the screen edge (graphics/window-server/src/motion.rs, desktop.rs)");
        StartClose = "motion.start_close" (mo(150, ACCELERATE_SOFT), mo(150, ACCELERATE_SOFT), mo(150, ACCELERATE_SOFT), bootstrap(UNMEASURED_TRACK), "Start dismiss");
        FlyoutOpen = "motion.flyout_open" (mo(167, DECELERATE_MAX), mo(167, DECELERATE_MAX), mo(167, DECELERATE_MAX), bootstrap(UNMEASURED_TRACK), "quick settings, notification center, calendar: a short decelerating slide and fade (not captured; Start's measured rise does not carry over)");
        WindowOpen = "motion.window_open" (mo(200, FAST_OUT_SLOW_IN), mo(200, FAST_OUT_SLOW_IN), mo(200, FAST_OUT_SLOW_IN), bootstrap(UNMEASURED_TRACK), "window open: 95 % scale and fade in (graphics/compositor/src/engine/animation.rs)");
        WindowClose = "motion.window_close" (mo(150, ACCELERATE), mo(150, ACCELERATE), mo(150, ACCELERATE), bootstrap(UNMEASURED_TRACK), "window close: 95 % scale and fade out");
        WindowMinimize = "motion.window_minimize" (mo(250, ACCELERATE), mo(250, ACCELERATE), mo(250, ACCELERATE), bootstrap(UNMEASURED_TRACK), "minimize to taskbar");
        WindowRestore = "motion.window_restore" (mo(250, FAST_OUT_SLOW_IN), mo(250, FAST_OUT_SLOW_IN), mo(250, FAST_OUT_SLOW_IN), bootstrap(UNMEASURED_TRACK), "restore from taskbar");
        WindowMaximize = "motion.window_maximize" (mo(200, POINT_TO_POINT), mo(200, POINT_TO_POINT), mo(200, POINT_TO_POINT), bootstrap(UNMEASURED_TRACK), "maximize (Win+Up) and snap glide, Fluent point-to-point curve");
        SnapPreview = "motion.snap_preview" (mo(167, FAST_OUT_SLOW_IN), mo(167, FAST_OUT_SLOW_IN), mo(167, FAST_OUT_SLOW_IN), bootstrap(UNMEASURED_TRACK), "snap layout preview");
        Notification = "motion.notification" (mo(333, FAST_OUT_SLOW_IN), mo(333, FAST_OUT_SLOW_IN), mo(333, FAST_OUT_SLOW_IN), bootstrap(UNMEASURED_TRACK), "toast slide-in");
        PageEnter = "motion.page_enter" (mo(300, DECELERATE_MAX), mo(300, DECELERATE_MAX), mo(300, DECELERATE_MAX), measured("reports/artifacts/reference-26h2/motion/settings-page/sequence.json: Windows 26H2 Settings, rail navigation System to Personalization at 150 %: the new page rises 140 epx (210 px) on (0.1,0.9,0.2,1) over 300 ms, opaque from its first frame (three runs, offsets within 1 % of the curve); the old page first fades out over about 150 ms (not reproduced)", DARK), "Settings page entrance (desktop/workspace/src/settings.rs)");
        LockDismiss = "motion.lock_dismiss" (mo(250, FAST_OUT_SLOW_IN), mo(250, FAST_OUT_SLOW_IN), mo(250, FAST_OUT_SLOW_IN), bootstrap(UNMEASURED_TRACK), "lock screen to sign-in transition");
    }
}

/// Resolved colour of a role in `theme`.
pub const fn color(token: ColorToken, theme: Theme) -> Color {
    token.entry().get(theme)
}
/// Spacing or dimension (theme-independent, logical px).
pub const fn spacing(token: SpacingToken) -> u16 {
    token.entry().light
}
/// Corner radius (theme-independent, logical px).
pub const fn radius(token: RadiusToken) -> u16 {
    token.entry().light
}
/// Type-ramp entry (theme-independent).
pub const fn type_ramp(token: TypeToken) -> TypeStyle {
    token.entry().light
}
/// Elevation in `theme` (contrast themes have no shadow).
pub const fn elevation(token: ElevationToken, theme: Theme) -> Elevation {
    token.entry().get(theme)
}
/// Motion track (theme-independent).
pub const fn motion(token: MotionToken) -> Motion {
    token.entry().light
}
/// Motion track honouring Accessibility > Animation effects off: every
/// duration is zero, so animations land on their final state at once.
pub const fn motion_with(token: MotionToken, reduced_motion: bool) -> Motion {
    let mut track = motion(token);
    if reduced_motion {
        track.duration_ms = 0;
    }
    track
}

fn view<T: Copy>(
    table: &'static [Entry<T>],
    kind: Kind,
    unit: Unit,
    wrap: fn(T) -> Value,
) -> impl Iterator<Item = Token> {
    table.iter().map(move |e| Token {
        id: e.id,
        kind,
        unit,
        light: wrap(e.light),
        dark: wrap(e.dark),
        contrast: wrap(e.contrast),
        provenance: e.provenance,
        controls: e.controls,
    })
}

/// Every token of every kind, in registry order.
pub fn tokens() -> impl Iterator<Item = Token> {
    view(COLORS, Kind::Color, Unit::PremultipliedBgra8, Value::Color)
        .chain(view(
            TYPE_RAMP,
            Kind::Typography,
            Unit::TypeRamp,
            Value::Type,
        ))
        .chain(view(
            SPACING_TABLE,
            Kind::Spacing,
            Unit::LogicalPx,
            Value::Length,
        ))
        .chain(view(RADII, Kind::Radius, Unit::LogicalPx, Value::Length))
        .chain(view(
            ELEVATIONS,
            Kind::Elevation,
            Unit::Shadow,
            Value::Elevation,
        ))
        .chain(view(
            MOTIONS,
            Kind::Motion,
            Unit::Milliseconds,
            Value::Motion,
        ))
}
/// The token with this id.
pub fn token(id: &str) -> Option<Token> {
    tokens().find(|t| t.id == id)
}
/// The value of token `id` in `theme`.
pub fn lookup(id: &str, theme: Theme) -> Option<Value> {
    token(id).map(|t| t.value(theme))
}

pub mod accent;

#[cfg(test)]
mod tests;
