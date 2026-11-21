//! File Explorer's caption tab strip for single-document apps (Preview,
//! Terminal): tabs from the window's left edge (icon, title, close), a "+"
//! button, then empty caption that the window service keeps draggable up to
//! its trusted window controls. Geometry is `document_tabs::Strip`, the same
//! model Explorer's caption uses; colors match Explorer's caption and tabs.
use crate::{View, document_tabs::Strip, explorer_layout::Bounds, icons::Icon, ui::display_ascii};
use door_graphics_text::TextStyle;

/// Caption height requested from the window service (as File Explorer).
pub const HEIGHT: u32 = 40;
/// Tabs start at the left edge of the window.
pub const LEFT: i32 = 8;

pub fn strip(width: u32, count: usize, active: usize) -> Strip {
    Strip {
        width,
        left: LEFT,
        count,
        active,
    }
}
/// The client-owned caption region for `set_tabbed_caption`: the first tab
/// through the "+" button. Everything right of it stays draggable caption.
pub fn region(s: Strip) -> (u32, u32) {
    let right = s
        .plus()
        .map(|r| r.x + r.width as i32)
        .or_else(|| {
            s.range()
                .filter_map(|i| s.tab(i))
                .map(|r| r.x + r.width as i32)
                .max()
        })
        .unwrap_or(LEFT + 1);
    (LEFT as u32, (right - LEFT).max(1) as u32)
}
/// A tab's 16 px icon: a themed shell glyph/association, or native-density
/// color artwork (16, 32 or 48 px square premultiplied BGRA for scale 1-3).
#[derive(Clone, Copy)]
pub enum TabIcon<'a> {
    Shell(Icon),
    Color(&'a [u8]),
}
/// Colors for one strip. `explorer` is File Explorer's opaque caption; apps
/// over a live acrylic backdrop pass premultiplied translucent colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabColors {
    pub caption: [u8; 4],
    /// Selected tab; it continues into the app surface below the caption.
    pub selected: [u8; 4],
    /// Hover fills; translucent values are overlays blended onto the strip.
    pub hover: [u8; 4],
    pub pressed: [u8; 4],
    pub divider: [u8; 4],
    pub foreground: [u8; 4],
}
impl TabColors {
    pub fn explorer(dark: bool) -> Self {
        if dark {
            Self {
                caption: [31, 11, 0, 255],
                selected: [35, 31, 27, 255],
                hover: [48, 43, 38, 255],
                pressed: [56, 52, 48, 255],
                divider: [61, 56, 50, 255],
                foreground: [240, 240, 240, 255],
            }
        } else {
            Self {
                caption: [243, 235, 225, 255],
                selected: [252, 251, 249, 255],
                hover: [247, 242, 237, 255],
                pressed: [240, 233, 226, 255],
                divider: [223, 218, 213, 255],
                foreground: [24, 24, 24, 255],
            }
        }
    }
}
/// Explorer's caption band.
pub fn caption_color(dark: bool) -> [u8; 4] {
    TabColors::explorer(dark).caption
}
/// The selected tab, which continues into the app surface directly below it.
pub fn surface_color(dark: bool) -> [u8; 4] {
    TabColors::explorer(dark).selected
}
pub fn paint_band(v: &mut View<'_>) {
    let c = caption_color(v.palette.dark);
    v.rect(0, 0, v.width, HEIGHT, c);
}
fn hit(r: Bounds, p: Option<(i32, i32)>) -> bool {
    p.is_some_and(|(x, y)| r.contains(x, y))
}
/// One caption tab with File Explorer's colors.
pub fn paint_tab(
    v: &mut View<'_>,
    r: Bounds,
    close: Option<Bounds>,
    icon: TabIcon<'_>,
    title: &str,
    selected: bool,
) {
    let colors = TabColors::explorer(v.palette.dark);
    paint_tab_colored(v, r, close, icon, title, selected, colors);
}
/// Selected-tab fill with rounded top corners and a square bottom. A
/// translucent fill is stored, not blended over the band, so the tab has
/// exactly the surface's translucency; its 8 px corners are row insets.
fn selected_fill(v: &mut View<'_>, r: Bounds, color: [u8; 4]) {
    if color[3] == 255 {
        v.rounded(r.x, r.y, r.width, r.height, 8, color);
        v.rect(
            r.x,
            r.y + r.height as i32 / 2,
            r.width,
            r.height - r.height / 2,
            color,
        );
        return;
    }
    const INSET: [u32; 8] = [5, 3, 2, 1, 1, 0, 0, 0];
    for (row, inset) in INSET.iter().enumerate() {
        v.rect(
            r.x + *inset as i32,
            r.y + row as i32,
            r.width.saturating_sub(inset * 2),
            1,
            color,
        );
    }
    v.rect(r.x, r.y + 8, r.width, r.height.saturating_sub(8), color);
}
/// One caption tab. `close` is the strip's close bounds for the tab.
pub fn paint_tab_colored(
    v: &mut View<'_>,
    r: Bounds,
    close: Option<Bounds>,
    icon: TabIcon<'_>,
    title: &str,
    selected: bool,
    colors: TabColors,
) {
    if r.width < 64 || r.height < 24 {
        return;
    }
    if selected {
        selected_fill(v, r, colors.selected);
    } else if hit(r, v.hover) {
        v.rounded(r.x, r.y, r.width, r.height, 8, colors.hover);
    } else {
        // `rounded` blends, so a translucent divider overlays the band.
        v.rounded(
            r.x + r.width as i32,
            r.y + 8,
            1,
            r.height.saturating_sub(16),
            0,
            colors.divider,
        );
    }
    let fg = colors.foreground;
    let iy = r.y + (r.height as i32 - 16) / 2;
    match icon {
        TabIcon::Shell(icon) => v.compact_icon(r.x + 12, iy, icon, fg),
        TabIcon::Color(pixels) => v.color_pixels(r.x + 12, iy, 16, pixels),
    }
    v.text_clip_centered(
        r.x + 36,
        (r.y, r.height),
        r.width.saturating_sub(72),
        &display_ascii(title),
        if selected {
            TextStyle::Semibold14
        } else {
            TextStyle::Regular14
        },
        fg,
    );
    if let Some(c) = close {
        if hit(c, v.hover) {
            v.rounded(c.x, c.y, c.width, c.height, 4, colors.pressed);
        }
        let mask = match v.scale {
            3 => door_workspace::asset!("../../../assets/icons/windows-shell/chrome-close.36.mask"),
            2 => door_workspace::asset!("../../../assets/icons/windows-shell/chrome-close.24.mask"),
            _ => door_workspace::asset!("../../../assets/icons/windows-shell/chrome-close.12.mask"),
        };
        v.mask_pixels(
            c.x + (c.width as i32 - 12) / 2,
            c.y + (c.height as i32 - 12) / 2,
            12,
            mask,
            fg,
            None,
        );
    }
}
/// Explorer's "+" new-tab button.
pub fn paint_plus(v: &mut View<'_>, r: Bounds) {
    let colors = TabColors::explorer(v.palette.dark);
    paint_plus_colored(v, r, colors);
}
pub fn paint_plus_colored(v: &mut View<'_>, r: Bounds, colors: TabColors) {
    if hit(r, v.hover) {
        let fill = if hit(r, v.pressed) {
            colors.pressed
        } else {
            colors.hover
        };
        v.rounded(r.x, r.y, r.width, r.height, 5, fill);
    }
    v.compact_icon(
        r.x + (r.width as i32 - 16) / 2,
        r.y + (r.height as i32 - 16) / 2,
        Icon::Add,
        colors.foreground,
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn region_covers_tabs_and_plus_and_never_the_window_controls() {
        for width in [240, 490, 900, 1280, 1920] {
            for count in 1..=8 {
                let s = strip(width, count, count - 1);
                let (left, w) = region(s);
                assert_eq!(left, LEFT as u32);
                assert!(left + w <= width.saturating_sub(138), "{width} {count}");
                for i in s.range() {
                    if let Some(t) = s.tab(i) {
                        assert!(t.x >= left as i32 && t.x + t.width as i32 <= (left + w) as i32);
                    }
                }
                if let Some(p) = s.plus() {
                    assert_eq!(p.x + p.width as i32, (left + w) as i32);
                }
            }
        }
    }
}
