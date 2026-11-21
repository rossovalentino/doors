//! Theme roles for original control geometry and text. Bitmap assets bypass this
//! palette: photographs, wallpaper previews and Windows color ICOs retain bytes.
//! Reference: Microsoft XAML theme resources and Support dark-mode screenshot.
//!
//! Contrast themes (Settings > Accessibility > Contrast themes) map every
//! painted colour onto the theme's system colours, as Windows' forced colours
//! do for classic controls. The shell sends light appearance while a contrast
//! theme is on, so painters hand this palette their light-theme colours, and
//! the colour's role is read from it: near-white backgrounds become Window,
//! darker neutral strokes WindowText, accent and selection fills Highlight;
//! dark text WindowText, white text (drawn on accent) HighlightText, grey
//! (disabled) text GrayText, coloured text and glyphs Hotlight.
use door_service_protocol::desktop::{ContrastTheme, SystemColors};
#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// The active contrast theme, if any.
    pub contrast: Option<ContrastTheme>,
}
impl Default for Palette {
    /// This process's current presentation: light, with the session's
    /// contrast theme (the runtime then sets `dark` from APPEARANCE).
    fn default() -> Self {
        Self {
            dark: false,
            contrast: door_service_client::window::desktop_controls().contrast(),
        }
    }
}
/// Accessibility > Text size this process paints with, in percent: the
/// runtime sets it together with the UI atlas it installs (`FontAtlas::
/// text_percent`), from the session's delivered desktop controls. Layout
/// that holds text grows by it (`text_grow`), so hit targets, semantics
/// bounds and paint agree; icons and chrome keep their size, as on Windows.
static TEXT_PERCENT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(100);
/// Settings' calibrated page background and control fill (light, dark),
/// BGRA: the Mica veils resolve to them over the background (settings/
/// theme.rs), and host checks of guest captures recompute the same veils.
pub const SETTINGS_BG: ([u8; 4], [u8; 4]) = ([250, 247, 244, 255], [36, 32, 31, 255]);
pub const SETTINGS_CONTROL: ([u8; 4], [u8; 4]) = ([253, 253, 253, 255], [59, 56, 55, 255]);
/// The premultiplied white (lighter) or black (darker) veil that turns
/// `base` into `fill`: how Settings paints a fill over Mica (a dark card is
/// white at about 5 %, as WinUI's).
pub fn mica_overlay(fill: [u8; 4], base: [u8; 4]) -> [u8; 4] {
    let mean = |c: [u8; 4]| (u32::from(c[0]) + u32::from(c[1]) + u32::from(c[2])) / 3;
    let (f, b) = (mean(fill), mean(base));
    if f >= b {
        let a = ((f - b) * 255 / (255 - b).max(1)).min(255) as u8;
        [a, a, a, a]
    } else {
        let a = ((b - f) * 255 / b.max(1)).min(255) as u8;
        [0, 0, 0, a]
    }
}
pub fn text_percent() -> u32 {
    TEXT_PERCENT.load(core::sync::atomic::Ordering::Relaxed)
}
/// Record the text size of the atlas now installed (runtime, host reviews).
pub fn set_text_percent(percent: u32) {
    TEXT_PERCENT.store(
        percent.clamp(
            door_graphics_text::TEXT_PERCENT_MIN,
            door_graphics_text::TEXT_PERCENT_MAX,
        ),
        core::sync::atomic::Ordering::Relaxed,
    );
}
/// The text size the session asks for (last delivered controls).
pub fn requested_text_percent() -> u32 {
    door_service_client::window::desktop_controls().text_percent()
}
/// A logical text length at the applied text size.
pub fn text_len(px: u32) -> u32 {
    door_graphics_text::text_len(px, text_percent())
}
/// A text-holding layout length (row, button, menu item, card) grown by the
/// growth of `style`'s line box at the applied text size.
pub fn text_grow(base: u32, style: door_graphics_text::TextStyle) -> u32 {
    door_graphics_text::text_grow(base, style, text_percent())
}
/// `text_grow` for signed layout coordinates.
pub fn text_grow_i(base: i32, style: door_graphics_text::TextStyle) -> i32 {
    base + (text_grow(0, style) as i32)
}
fn bgra(rgb: [u8; 3], alpha: u8) -> [u8; 4] {
    [rgb[2], rgb[1], rgb[0], alpha]
}
/// Perceived lightness (0..=255) of a BGRA colour.
fn luma([b, g, r, _]: [u8; 4]) -> u32 {
    (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000
}
/// Neutral greys and the warm/cool near-greys of the Windows 11 surfaces.
fn neutral([b, g, r, _]: [u8; 4]) -> bool {
    b.max(g).max(r) - b.min(g).min(r) <= 24
}
/// Contrast role of a background, fill or stroke.
fn contrast_surface(s: SystemColors, color: [u8; 4], dark: bool) -> [u8; 4] {
    let alpha = color[3];
    if !neutral(color) {
        return bgra(s.highlight, alpha);
    }
    let l = luma(color);
    let window = if dark { l <= 70 } else { l >= 232 };
    bgra(if window { s.window } else { s.window_text }, alpha)
}
/// Contrast role of text and glyph ink.
fn contrast_foreground(s: SystemColors, color: [u8; 4], dark: bool) -> [u8; 4] {
    let alpha = color[3];
    if !neutral(color) {
        return bgra(s.hotlight, alpha);
    }
    let l = luma(color);
    let role = if dark {
        match l {
            150.. => s.window_text,
            0..=60 => s.highlight_text,
            _ => s.gray_text,
        }
    } else {
        match l {
            236.. => s.highlight_text,
            0..=150 => s.window_text,
            _ => s.gray_text,
        }
    };
    bgra(role, alpha)
}
impl Palette {
    pub fn light() -> Self {
        Self {
            dark: false,
            contrast: None,
        }
    }
    pub fn with_dark(dark: bool) -> Self {
        Self {
            dark,
            contrast: None,
        }
    }
    /// The active contrast theme's system colours.
    pub fn system(self) -> Option<SystemColors> {
        self.contrast.map(ContrastTheme::colors)
    }
    pub fn surface(self, color: [u8; 4]) -> [u8; 4] {
        if let Some(s) = self.system() {
            return contrast_surface(s, color, self.dark);
        }
        if !self.dark {
            return color;
        }
        let [b, g, r, a] = color;
        let neutral = b.abs_diff(g) <= 8 && g.abs_diff(r) <= 8 && b.abs_diff(r) <= 8;
        if neutral {
            let value = match r {
                253..=255 => 43, // content/control
                247..=252 => 40, // secondary layer
                240..=246 => 32, // base
                230..=239 => 56, // selected/hover layer
                180..=229 => 65, // divider and border
                _ => return color,
            };
            [value, value, value, a]
        } else if b > r.saturating_add(15) && r >= 150 && g > r {
            [83, 62, 45, a] // pale selection fill -> dark selected layer
        } else {
            color
        }
    }
    pub fn foreground(self, color: [u8; 4]) -> [u8; 4] {
        if let Some(s) = self.system() {
            return contrast_foreground(s, color, self.dark);
        }
        if !self.dark {
            return color;
        }
        let [b, g, r, a] = color;
        if b == g && g == r {
            let value = match r {
                0..=64 => 240,
                65..=140 => 180,
                141..=200 => 150,
                _ => r,
            };
            [value, value, value, a]
        } else if b > 150 && r < 80 && g > 60 {
            [255, 183, 96, a] // text/icon accent
        } else {
            color
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_surfaces_and_assets_have_distinct_roles_and_light_is_exact() {
        let dark = Palette::with_dark(true);
        assert_eq!(dark.surface([243, 243, 243, 255]), [32, 32, 32, 255]);
        assert_eq!(dark.surface([255; 4]), [43, 43, 43, 255]);
        assert_eq!(dark.foreground([32, 32, 32, 255]), [240, 240, 240, 255]);
        assert_eq!(dark.foreground([255; 4]), [255; 4]);
        let accent = [196, 103, 0, 255];
        assert_eq!(dark.surface(accent), accent);
        for color in [[243, 243, 243, 255], [255; 4], accent] {
            assert_eq!(Palette::light().surface(color), color);
            assert_eq!(Palette::light().foreground(color), color);
        }
    }
    #[test]
    fn contrast_themes_map_light_roles_onto_system_colours() {
        for theme in ContrastTheme::ALL {
            let s = theme.colors();
            let p = Palette {
                dark: false,
                contrast: Some(theme),
            };
            let rgb = |c: [u8; 4]| [c[2], c[1], c[0]];
            // Backgrounds, layers and hover plates: Window.
            for bg in [
                [255; 4],
                [250, 247, 244, 255],
                [243, 243, 243, 255],
                [253, 252, 251, 255],
            ] {
                assert_eq!(rgb(p.surface(bg)), s.window, "{theme:?} {bg:?}");
            }
            // Borders and dark strokes: WindowText.
            for line in [
                [224, 224, 224, 255],
                [229, 229, 229, 255],
                [190, 190, 190, 255],
                [32, 32, 32, 255],
            ] {
                assert_eq!(rgb(p.surface(line)), s.window_text, "{theme:?} {line:?}");
            }
            // Accent and selection fills: Highlight.
            for fill in [[196, 103, 0, 255], [203, 103, 0, 255], [250, 233, 213, 255]] {
                assert_eq!(rgb(p.surface(fill)), s.highlight, "{theme:?} {fill:?}");
            }
            // Text roles.
            assert_eq!(rgb(p.foreground([32, 32, 32, 255])), s.window_text);
            assert_eq!(rgb(p.foreground([96, 96, 96, 255])), s.window_text);
            assert_eq!(rgb(p.foreground([255; 4])), s.highlight_text);
            assert_eq!(rgb(p.foreground([160, 160, 160, 255])), s.gray_text);
            assert_eq!(rgb(p.foreground([196, 103, 0, 255])), s.hotlight);
            // Alpha is kept (shadows, dims and anti-aliased edges).
            assert_eq!(p.surface([0, 0, 0, 80])[3], 80);
        }
    }
}
