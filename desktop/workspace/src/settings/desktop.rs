//! Desktop previews painted from the real wallpaper exports and the live
//! appearance/Controls state: device thumbnail, Personalization preview
//! and theme tiles.
use super::theme::Theme;
use super::*;
use door_workspace::settings_catalog::Rect as R;
/// Device thumbnail (Home, System, About): 116 x 76 wallpaper in a bezel.
pub(super) fn thumbnail(v: &mut ui::View<'_>, x: i32, y: i32, background: u32) {
    if let Some(s) = v.palette.system() {
        // Contrast themes: the desktop is the solid Window colour.
        v.rounded_asset(x, y, 124, 82, 7, sys(s.window_text));
        v.rounded_asset(x + 4, y + 3, 116, 76, 4, sys(s.window));
        return;
    }
    v.rounded_asset(x, y, 124, 82, 7, [18, 18, 18, 255]);
    super::pages::wallpaper(v, x + 4, y + 3, 116, background, 4);
}
/// A system colour (RGB) as opaque BGRA.
fn sys(c: [u8; 3]) -> [u8; 4] {
    [c[2], c[1], c[0], 255]
}
/// Personalization > Themes preview (26H2): the wallpaper in an 8 px black
/// bezel with a translucent sample window in the current mode (lines and an
/// accent button), no taskbar. `solid` (0xRRGGBB) replaces the picture.
pub(super) fn theme_preview(
    v: &mut ui::View<'_>,
    t: &Theme,
    x: i32,
    y: i32,
    appearance: u32,
    solid: Option<u32>,
) {
    use door_workspace::settings_catalog::THEME_PREVIEW;
    let (w, h) = THEME_PREVIEW;
    if let Some(s) = v.palette.system() {
        // Contrast themes: the same shapes in the system colors.
        v.rounded_asset(x, y, w, h, 8, sys(s.window_text));
        v.rounded_asset(x + 8, y + 8, w - 16, h - 16, 2, sys(s.window));
        v.rounded_asset(x + 138, y + 29, 95, 96, 4, sys(s.window_text));
        v.rounded_asset(x + 139, y + 30, 93, 94, 3, sys(s.window));
        for row in [43, 52, 61] {
            v.rect(x + 150, y + row, 71, 1, sys(s.window_text));
        }
        v.rounded_asset(x + 197, y + 105, 24, 8, 2, sys(s.highlight));
        return;
    }
    v.rounded_asset(x, y, w, h, 8, [0, 0, 0, 255]);
    match solid {
        Some(rgb) => v.rounded_asset(
            x + 8,
            y + 8,
            w - 16,
            h - 16,
            2,
            [rgb as u8, (rgb >> 8) as u8, (rgb >> 16) as u8, 255],
        ),
        None => super::pages::wallpaper_cover(v, x + 8, y + 8, w - 16, h - 16, appearance >> 1, 2),
    }
    // Premultiplied: Windows' sample window lets the wallpaper through.
    let (surface, ink) = if appearance & 1 != 0 {
        ([29, 29, 29, 153], [202, 207, 223, 255])
    } else {
        ([176, 176, 178, 204], [81, 83, 89, 255])
    };
    v.rounded_asset(x + 138, y + 29, 95, 96, 4, surface);
    for (row, length) in [(43, 71), (52, 71), (61, 71), (71, 41)] {
        v.rect(x + 150, y + row, length, 1, ink);
    }
    v.rounded_asset(x + 197, y + 105, 24, 8, 2, t.accent);
}
/// Theme tile: wallpaper, a window in the theme's mode, its accent.
pub(super) fn theme_tile(
    v: &mut ui::View<'_>,
    t: &Theme,
    r: R,
    def: door_workspace::settings_layout::ThemeDef,
    selected: bool,
    hover: bool,
) {
    let appearance = def.appearance();
    if selected {
        v.rounded(r.x - 3, r.y - 3, r.w + 6, r.h + 6, 8, t.accent);
    } else if hover {
        v.rounded(r.x - 2, r.y - 2, r.w + 4, r.h + 4, 7, t.strong);
    }
    super::pages::wallpaper(v, r.x, r.y, 110, appearance >> 1, 5);
    v.rounded_asset(
        r.x + 62,
        r.y + 42,
        42,
        38,
        4,
        if appearance & 1 != 0 {
            [40, 37, 36, 255]
        } else {
            [249, 249, 249, 255]
        },
    );
    let accent = [
        def.accent as u8,
        (def.accent >> 8) as u8,
        (def.accent >> 16) as u8,
        255,
    ];
    v.rounded_asset(r.x + 70, r.y + 66, 26, 6, 3, accent);
}
