//! Windows 11 Settings page painter. Every rectangle comes from the shared
//! `settings_catalog` placement used for hit testing and keyboard focus.
use super::theme::Theme;
use super::*;
use door_workspace::settings_catalog::{Block, Control, Fact, Hero, Placed, Rect as R, Row};
use door_workspace::settings_layout::{BACKGROUNDS, GALLERY, INCLUDED_APPS, THEMES};
pub(super) const APP_MENU: [&str; 3] = ["Open", "Advanced options", "Uninstall"];
/// A desktop program's "..." menu.
pub(super) const PROGRAM_MENU: [&str; 2] = ["Modify", "Uninstall"];
/// The flyout under Uninstall.
pub(super) const CONFIRM_UNINSTALL: [&str; 2] = [
    "This app and its related info will be uninstalled.",
    "Uninstall",
];
pub(super) fn preview(background: u32, size: u32, scale: u32) -> &'static [u8] {
    macro_rules! image {
        ($name:literal) => {
            match (size, scale) {
                (110, 3) => &door_workspace::asset!(concat!(
                    "../../../../assets/wallpapers/windows/",
                    $name,
                    "-330x258.qoi"
                ))[..],
                (110, 2) => &door_workspace::asset!(concat!(
                    "../../../../assets/wallpapers/windows/",
                    $name,
                    "-220x172.qoi"
                ))[..],
                // One 2x source: the painter averages it 2:1 at 1x and
                // resamples it bilinearly at 3x (loader size budget).
                (320, _) => &door_workspace::asset!(concat!(
                    "../../../../assets/wallpapers/windows/",
                    $name,
                    "-640x400.qoi"
                ))[..],
                (_, 3) => &door_workspace::asset!(concat!(
                    "../../../../assets/wallpapers/windows/",
                    $name,
                    "-348x228.qoi"
                ))[..],
                (_, 2) => &door_workspace::asset!(concat!(
                    "../../../../assets/wallpapers/windows/",
                    $name,
                    "-232x152.qoi"
                ))[..],
                (110, _) => &door_workspace::asset!(concat!(
                    "../../../../assets/wallpapers/windows/",
                    $name,
                    "-110x86.qoi"
                ))[..],
                _ => &door_workspace::asset!(concat!(
                    "../../../../assets/wallpapers/windows/",
                    $name,
                    "-116x76.qoi"
                ))[..],
            }
        };
    }
    match background {
        1 => image!("bloom-dark"),
        2 => image!("glow"),
        3 => image!("sunrise"),
        _ => image!("bloom"),
    }
}
/// The 2x Settings preview of a picture new in version 2 of the pack
/// (`door_initfs::desktop_assets`), from which its thumbnails are made.
fn large_preview(background: u32) -> &'static [u8] {
    use door_workspace::asset;
    match background {
        4 => asset!("../../../../assets/wallpapers/windows/26h2/img21-640x400.qoi"),
        5 => asset!("../../../../assets/wallpapers/windows/26h2/img22-640x400.qoi"),
        6 => asset!("../../../../assets/wallpapers/windows/26h2/img23-640x400.qoi"),
        7 => asset!("../../../../assets/wallpapers/windows/26h2/img24-640x400.qoi"),
        8 => asset!("../../../../assets/wallpapers/windows/26h2/img25-640x400.qoi"),
        9 => asset!("../../../../assets/wallpapers/windows/26h2/img26-640x400.qoi"),
        10 => asset!("../../../../assets/wallpapers/windows/26h2/img27-640x400.qoi"),
        11 => asset!("../../../../assets/wallpapers/windows/26h2/img29-640x400.qoi"),
        12 => asset!("../../../../assets/wallpapers/windows/26h2/img30-640x400.qoi"),
        13 => asset!("../../../../assets/wallpapers/windows/26h2/img31-640x400.qoi"),
        14 => asset!("../../../../assets/wallpapers/windows/26h2/img32-640x400.qoi"),
        15 => asset!("../../../../assets/wallpapers/windows/26h2/img33-640x400.qoi"),
        16 => asset!("../../../../assets/wallpapers/windows/26h2/img34-640x400.qoi"),
        _ => asset!("../../../../assets/wallpapers/windows/26h2/img35-640x400.qoi"),
    }
}
/// A wallpaper thumbnail `size` logical pixels wide (110: theme tiles and
/// the lock screen, 116: device and recent images, 320: the desktop
/// preview), from the exact exports Bloom, dark Bloom, Glow and Sunrise
/// have, else cover-cropped from the picture's 2x preview.
pub(super) fn wallpaper(
    v: &mut ui::View<'_>,
    x: i32,
    y: i32,
    size: u32,
    background: u32,
    radius: u32,
) {
    if background < 4 {
        v.bitmap_bgr_rounded(
            x,
            y,
            size as usize,
            preview(background, size, v.scale),
            radius,
        );
        return;
    }
    let height = match size {
        110 => 86,
        116 => 76,
        _ => size * 5 / 8,
    };
    v.bitmap_cover_rounded(x, y, size, height, large_preview(background), radius);
}
/// Background `background` cropped to cover `width` x `height` (any size;
/// the 640x400 previews).
pub(super) fn wallpaper_cover(
    v: &mut ui::View<'_>,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    background: u32,
    radius: u32,
) {
    let source = if background < 4 {
        preview(background, 320, v.scale)
    } else {
        large_preview(background)
    };
    v.bitmap_cover_rounded(x, y, width, height, source, radius);
}
/// The Doors mark at a logical row/header size (native density frames).
/// `text` as it fits in `width`: whole, or cut at a character and ended with
/// "..." (WinUI CharacterEllipsis), never a glyph cut by the clip.
pub(super) fn fitted(v: &ui::View<'_>, text: &str, width: u32, style: TextStyle) -> String {
    if v.measure_style(text, style) <= width {
        return text.into();
    }
    let mut end = text.len();
    while end > 0 {
        end -= 1;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let candidate = format!("{}...", text[..end].trim_end());
        if v.measure_style(&candidate, style) <= width {
            return candidate;
        }
    }
    String::new()
}
fn doors_logo(v: &mut ui::View<'_>, x: i32, y: i32, size: u32) {
    let pixels: &[u8] = match (size, v.scale) {
        (24, 3) => door_workspace::asset!("../../../../assets/branding/door.72.bgra"),
        (24, 2) => door_workspace::asset!("../../../../assets/branding/door.48.bgra"),
        (24, _) => door_workspace::asset!("../../../../assets/branding/door.24.bgra"),
        (_, 3) => door_workspace::asset!("../../../../assets/branding/door.120.bgra"),
        (_, 2) => door_workspace::asset!("../../../../assets/branding/door.80.bgra"),
        _ => door_workspace::asset!("../../../../assets/branding/door.40.bgra"),
    };
    v.color_pixels(x, y, if size == 24 { 24 } else { 40 }, pixels);
}
fn inflate(r: R, by: i32) -> R {
    R::new(
        r.x - by,
        r.y - by,
        (r.w as i32 + 2 * by).max(0) as u32,
        (r.h as i32 + 2 * by).max(0) as u32,
    )
}
/// Card: 1 px stroke and fill.
pub(super) fn card(v: &mut ui::View<'_>, t: &Theme, r: R, radius: u32, fill: [u8; 4]) {
    if fill[3] < 255 {
        // Over Mica the fill is a translucent veil: paint it once and lay
        // the 1 px stroke on top as a ring, so the layers never stack.
        v.rounded(r.x, r.y, r.w, r.h, radius, fill);
        v.rounded_ring(r.x, r.y, r.w, r.h, radius, t.card_border);
        return;
    }
    v.rounded(r.x, r.y, r.w, r.h, radius, t.card_border);
    v.rounded(
        r.x + 1,
        r.y + 1,
        r.w.saturating_sub(2),
        r.h.saturating_sub(2),
        radius.saturating_sub(1),
        fill,
    );
}
/// Keyboard focus visual: a 2 px ring 3 px outside the element; contrast
/// themes thicken it to 3 px, as Windows does.
fn ring(v: &mut ui::View<'_>, t: &Theme, r: R) {
    let o = inflate(r, 3);
    let c = 3;
    let k = if v.palette.contrast.is_some() { 3 } else { 2 };
    v.rect(o.x + c, o.y, o.w.saturating_sub(2 * c as u32), k, t.focus);
    v.rect(
        o.x + c,
        o.bottom() - k as i32,
        o.w.saturating_sub(2 * c as u32),
        k,
        t.focus,
    );
    v.rect(o.x, o.y + c, k, o.h.saturating_sub(2 * c as u32), t.focus);
    v.rect(
        o.right() - k as i32,
        o.y + c,
        k,
        o.h.saturating_sub(2 * c as u32),
        t.focus,
    );
    for (x, y) in [
        (o.x + 1, o.y + 1),
        (o.right() - 3, o.y + 1),
        (o.x + 1, o.bottom() - 3),
        (o.right() - 3, o.bottom() - 3),
    ] {
        v.rect(x, y, 2, 2, t.focus);
    }
}
pub(super) fn avatar(v: &mut ui::View<'_>, t: &Theme, x: i32, y: i32, size: u32) {
    v.rounded(
        x,
        y,
        size,
        size,
        size / 2,
        if t.dark {
            [72, 70, 69, 255]
        } else {
            [218, 216, 214, 255]
        },
    );
    let (glyph, mask): (u32, &[u8]) = match (size >= 96, v.scale) {
        (true, 3) => (
            48,
            door_workspace::asset!("../../../../assets/icons/windows-shell/contact.144.mask"),
        ),
        (false, 3) => (
            24,
            door_workspace::asset!("../../../../assets/icons/windows-shell/contact.72.mask"),
        ),
        (true, 2) => (
            48,
            door_workspace::asset!("../../../../assets/icons/windows-shell/contact.96.mask"),
        ),
        (true, _) | (false, 2) => {
            if size >= 96 {
                (
                    48,
                    door_workspace::asset!(
                        "../../../../assets/icons/windows-shell/contact.48.mask"
                    ),
                )
            } else {
                (
                    24,
                    door_workspace::asset!(
                        "../../../../assets/icons/windows-shell/contact.48.mask"
                    ),
                )
            }
        }
        _ => (
            24,
            door_workspace::asset!("../../../../assets/icons/windows-shell/contact.24.mask"),
        ),
    };
    v.mask_pixels(
        x + (size - glyph) as i32 / 2,
        y + (size - glyph) as i32 / 2,
        glyph as usize,
        mask,
        t.secondary,
        None,
    );
}
/// Title-bar search box (25H2 pill with an accent underline when focused).
pub(super) fn search_box(
    v: &mut ui::View<'_>,
    t: &Theme,
    s: door_workspace::settings_layout::SearchBox,
    query: &str,
    focused: bool,
) {
    let radius = s.h / 2;
    let hot = v.hover.is_some_and(|(x, y)| s.contains(x, y));
    if focused {
        v.rounded(s.x, s.y, s.w, s.h, radius, t.accent);
        v.rounded(
            s.x,
            s.y,
            s.w,
            s.h - 2,
            radius,
            if t.dark { t.card } else { [255; 4] },
        );
        v.rounded(
            s.x + 1,
            s.y + 1,
            s.w.saturating_sub(2),
            s.h - 4,
            radius - 1,
            if t.dark { [30, 27, 26, 255] } else { [255; 4] },
        );
    } else if t.control[3] < 255 {
        // Over Mica: the control veil once, its border as a ring.
        let fill = if hot { t.control_hover } else { t.control };
        v.rounded(s.x, s.y, s.w, s.h, radius, fill);
        v.rounded_ring(s.x, s.y, s.w, s.h, radius, t.control_border);
    } else {
        v.rounded(s.x, s.y, s.w, s.h, radius, t.control_border);
        v.rounded(
            s.x + 1,
            s.y + 1,
            s.w.saturating_sub(2),
            s.h - 2,
            radius - 1,
            if hot { t.control_hover } else { t.control },
        );
    }
    v.mask_pixels(
        s.x + 14,
        s.y + (s.h as i32 - 16) / 2,
        16,
        Icon::Search.mask_scaled(16, v.scale),
        t.secondary,
        None,
    );
    let x = s.x + 40;
    let width = s.w.saturating_sub(56);
    let shown = ui::display_ascii(query);
    if query.is_empty() {
        v.text_clip_centered(
            x,
            (s.y, s.h),
            width,
            "Find a setting",
            TextStyle::Regular14,
            t.secondary,
        );
    } else {
        v.text_clip_centered(x, (s.y, s.h), width, &shown, TextStyle::Regular14, t.text);
    }
    if focused {
        let caret = if query.is_empty() {
            0
        } else {
            v.measure(&shown).min(width.saturating_sub(1))
        };
        let top = v.centered_text_top(s.y, s.h, TextStyle::Regular14);
        v.text_caret(x + caret as i32, top, TextStyle::Regular14, t.text);
    }
}
pub(super) fn suggestions(
    v: &mut ui::View<'_>,
    t: &Theme,
    layout: Layout,
    hits: &[door_settings_map::Hit],
    selected: usize,
) {
    let Some((x, y, w, pitch)) = navigation::suggestion_box(layout) else {
        return;
    };
    let pages = hits;
    let rows = pages.len().max(1) as i32;
    let h = (8 + rows * pitch) as u32;
    v.elevation(x, y, w, h, 60);
    v.rounded(x, y, w, h, 8, t.popup_border);
    v.rounded(x + 1, y + 1, w - 2, h - 2, 7, t.popup);
    if pages.is_empty() {
        v.text_clip_centered(
            x + 16,
            (y + 4, pitch as u32),
            w - 32,
            "No results found",
            TextStyle::Regular14,
            t.secondary,
        );
        return;
    }
    for (index, hit) in pages.iter().enumerate() {
        let top = y + 4 + index as i32 * pitch;
        if index == selected {
            v.rounded(x + 4, top, w - 8, pitch as u32, 4, t.popup_selected);
            v.rounded(x + 4, top + 10, 3, 16, 2, t.accent);
        }
        category(v, hit.page(), x + 16, top + 8, 20);
        let (title, secondary) = navigation::suggestion_text(*hit);
        let title = fitted(v, title, w.saturating_sub(64), TextStyle::Regular14);
        let tw = v.measure(&title);
        v.text_clip_centered(
            x + 48,
            (top, pitch as u32),
            w.saturating_sub(64),
            &title,
            TextStyle::Regular14,
            t.text,
        );
        if !secondary.is_empty() {
            let room = w.saturating_sub(72 + tw);
            v.text_clip_centered(
                x + 48 + tw as i32 + 8,
                (top, pitch as u32),
                room,
                &fitted(v, secondary, room, TextStyle::Regular12),
                TextStyle::Regular12,
                t.secondary,
            );
        }
    }
}
/// WinUI standard Button (32 px): control fill, stroke with a darker bottom
/// edge, hover and pressed fills.
pub(super) fn button(v: &mut ui::View<'_>, t: &Theme, r: R, label: &str, hot: bool, down: bool) {
    field(v, t, r, label, true, hot, down, false);
}
/// Keyboard focus ring (2 px, 3 px outside; thicker in contrast themes).
pub(super) fn focus_ring(v: &mut ui::View<'_>, t: &Theme, r: R) {
    ring(v, t, r);
}
/// The deep-link highlight: a 2 px accent outline around the target that
/// fades out over its last half second (`alpha` 0..=255).
fn highlight_ring(v: &mut ui::View<'_>, t: &Theme, r: R, alpha: u8) {
    if alpha == 0 {
        return;
    }
    let a = u16::from(alpha);
    let c = t.accent;
    let color = [
        (u16::from(c[0]) * a / 255) as u8,
        (u16::from(c[1]) * a / 255) as u8,
        (u16::from(c[2]) * a / 255) as u8,
        alpha,
    ];
    let o = inflate(r, 2);
    v.rounded_asset(o.x + 4, o.y, o.w.saturating_sub(8), 2, 0, color);
    v.rounded_asset(o.x + 4, o.bottom() - 2, o.w.saturating_sub(8), 2, 0, color);
    v.rounded_asset(o.x, o.y + 4, 2, o.h.saturating_sub(8), 0, color);
    v.rounded_asset(o.right() - 2, o.y + 4, 2, o.h.saturating_sub(8), 0, color);
    for (x, y) in [
        (o.x + 1, o.y + 1),
        (o.right() - 4, o.y + 1),
        (o.x + 1, o.bottom() - 4),
        (o.right() - 4, o.bottom() - 4),
    ] {
        v.rounded_asset(x, y, 3, 3, 1, color);
    }
}
/// Wall time (`operation::WALL_TIME`) in time zone `zone`; None without a
/// valid RTC (and on host builds).
pub(super) fn local_now(zone: u32) -> Option<door_abi::clock::WallClock> {
    let utc = door_sys::wall_time().ok()?;
    Some(door_locale::tz::zone_or_utc(zone).to_local(utc)?.time)
}
pub(super) fn popup(
    v: &mut ui::View<'_>,
    t: &Theme,
    popup: &catalog::Popup,
    labels: &[&str],
    current: Option<usize>,
) {
    let r = popup.rect;
    v.elevation(r.x, r.y, r.w, r.h, 70);
    v.rounded(r.x, r.y, r.w, r.h, 8, t.popup_border);
    v.rounded(r.x + 1, r.y + 1, r.w - 2, r.h - 2, 7, t.popup);
    let shown = popup.first..popup.count.min(popup.first + popup.visible());
    for (index, label) in labels.iter().enumerate().take(popup.count) {
        if !shown.contains(&index) {
            continue;
        }
        let o = popup.option(index);
        let enabled = popup.enabled(index);
        if index == popup.selected && enabled {
            v.rounded(o.x, o.y + 2, o.w, o.h - 4, 4, t.popup_selected);
        }
        if current == Some(index) {
            v.rounded(o.x, o.y + 10, 3, 16, 2, t.accent);
        }
        v.text_clip_centered(
            o.x + 12,
            (o.y, o.h),
            o.w.saturating_sub(20),
            label,
            TextStyle::Regular14,
            if enabled { t.text } else { t.disabled },
        );
    }
}
/// WinUI ToggleSwitch (40 x 20).
pub(super) fn toggle(
    v: &mut ui::View<'_>,
    t: &Theme,
    r: R,
    on: bool,
    enabled: bool,
    hot: bool,
    down: bool,
) {
    let knob = if (down || hot) && enabled { 14 } else { 12 };
    if on {
        let fill = if !enabled {
            t.strong_disabled
        } else if hot {
            t.accent_hover
        } else {
            t.accent
        };
        v.rounded(r.x, r.y, r.w, r.h, r.h / 2, fill);
        let k = knob as i32;
        v.rounded(
            r.right() - 4 - k + if knob > 12 { 1 } else { 0 },
            r.center_y() - k / 2,
            knob,
            knob,
            knob / 2,
            if enabled { t.on_accent } else { t.card },
        );
    } else {
        let stroke = if enabled { t.strong } else { t.strong_disabled };
        v.rounded(r.x, r.y, r.w, r.h, r.h / 2, stroke);
        v.rounded(
            r.x + 1,
            r.y + 1,
            r.w - 2,
            r.h - 2,
            r.h / 2 - 1,
            if hot && enabled { t.card_hover } else { t.card },
        );
        let k = knob as i32;
        v.rounded(
            r.x + 4 - if knob > 12 { 1 } else { 0 },
            r.center_y() - k / 2,
            knob,
            knob,
            knob / 2,
            stroke,
        );
    }
}
/// Combo box or button field.
#[allow(clippy::too_many_arguments)]
fn field(
    v: &mut ui::View<'_>,
    t: &Theme,
    r: R,
    label: &str,
    enabled: bool,
    hot: bool,
    down: bool,
    combo: bool,
) {
    let fill = if !enabled {
        t.control_disabled
    } else if down && hot {
        t.control_pressed
    } else if hot {
        t.control_hover
    } else {
        t.control
    };
    if fill[3] < 255 {
        // Over Mica: the veil once, then its border ring and the darker
        // bottom edge (WinUI's control elevation border).
        v.rounded(r.x, r.y, r.w, r.h, 4, fill);
        v.rounded_ring(r.x, r.y, r.w, r.h, 4, t.control_border);
        if enabled {
            v.rect(
                r.x + 4,
                r.y + r.h as i32 - 1,
                r.w.saturating_sub(8),
                1,
                t.control_bottom,
            );
        }
    } else {
        v.rounded(r.x, r.y, r.w, r.h, 4, t.control_border);
        v.rounded(
            r.x,
            r.y + 1,
            r.w,
            r.h - 1,
            4,
            if enabled {
                t.control_bottom
            } else {
                t.control_border
            },
        );
        v.rounded(r.x + 1, r.y + 1, r.w - 2, r.h - 3, 3, fill);
    }
    let color = if enabled { t.text } else { t.disabled };
    if combo {
        v.text_clip_centered(
            r.x + 11,
            (r.y, r.h),
            r.w.saturating_sub(44),
            label,
            TextStyle::Regular14,
            color,
        );
        v.mask_pixels(
            r.right() - 28,
            r.center_y() - 6,
            12,
            Icon::Chevron.mask_scaled(12, v.scale),
            if enabled { t.secondary } else { t.disabled },
            None,
        );
    } else {
        v.text_in_box(
            (r.x + 8, r.y, r.w.saturating_sub(16), r.h),
            label,
            TextStyle::Regular14,
            color,
        );
    }
}
/// WinUI Slider: a 4 px track, the accent fill up to the value and a 20 px
/// thumb whose accent core grows on hover and shrinks while pressed.
fn slider(v: &mut ui::View<'_>, t: &Theme, r: R, value: u32, hot: bool, down: bool) {
    let cy = r.center_y();
    let thumb = catalog::slider_thumb(r, value);
    v.rounded(r.x, cy - 2, r.w, 4, 2, t.strong);
    v.rounded(r.x, cy - 2, (thumb - r.x).max(0) as u32, 4, 2, t.accent);
    v.rounded(thumb - 10, cy - 10, 20, 20, 10, t.control_border);
    v.rounded(thumb - 9, cy - 9, 18, 18, 9, t.control);
    let core = if down {
        10
    } else if hot {
        14
    } else {
        12
    };
    let half = core as i32 / 2;
    v.rounded(thumb - half, cy - half, core, core, core / 2, t.accent);
}
/// `slider` for a card control whose thumb sits at `thumb` (any range).
fn card_slider(v: &mut ui::View<'_>, t: &Theme, r: R, thumb: i32, hot: bool, down: bool) {
    let cy = r.center_y();
    v.rounded(r.x, cy - 2, r.w, 4, 2, t.strong);
    v.rounded(r.x, cy - 2, (thumb - r.x).max(0) as u32, 4, 2, t.accent);
    v.rounded(thumb - 10, cy - 10, 20, 20, 10, t.control_border);
    v.rounded(thumb - 9, cy - 9, 18, 18, 9, t.control);
    let core = if down {
        10
    } else if hot {
        14
    } else {
        12
    };
    let half = core as i32 / 2;
    v.rounded(thumb - half, cy - half, core, core, core / 2, t.accent);
}
fn chevron_right(v: &mut ui::View<'_>, t: &Theme, (cx, cy): (i32, i32)) {
    v.mask_pixels(
        cx - 6,
        cy - 6,
        12,
        Icon::ChevronRight.mask_scaled(12, v.scale),
        t.text,
        None,
    );
}
/// Interaction state of the page's items for one frame.
struct Hot {
    hover: Option<usize>,
    down: Option<usize>,
    focus: Option<usize>,
}
impl Settings {
    /// System > Sound text from the sound service's status.
    pub(super) fn audio_fact(&self, kind: catalog::AudioFact) -> String {
        use catalog::AudioFact as F;
        if kind == F::Requests {
            return match self.admin.microphone.len() {
                0 => String::from("No requests"),
                1 => String::from("1 request"),
                n => format!("{n} requests"),
            };
        }
        let Some(a) = self.audio.filter(|a| a.present) else {
            return String::from(match kind {
                F::Device => "No output devices found",
                F::Input => "No input devices found",
                F::Streams => "No apps are playing sound",
                _ => "Not available",
            });
        };
        let output = door_service_protocol::audio::Output::from_word(
            self.state().pref(door_service_protocol::preferences::SOUND),
        )
        .unwrap_or(a.output);
        match kind {
            F::Device => String::from("Speakers (VirtIO Sound)"),
            F::Format => format!(
                "{} channels, {} bit, {} Hz (Studio Quality)",
                a.channels, a.bits, a.rate
            ),
            F::Level if output.muted => String::from("Muted"),
            F::Level => format!("{}%", output.volume),
            F::Streams => match a.streams {
                0 => String::from("No apps are playing sound"),
                1 => String::from("1 app is playing sound"),
                n => format!("{n} apps are playing sound"),
            },
            F::Input if a.capture => String::from("Microphone (VirtIO Sound)"),
            F::Input => String::from("No input devices found"),
            F::Requests => unreachable!(),
        }
    }
    /// Network & internet text from the network service's status.
    /// Settings > VPN > Advanced options: the open connection's properties.
    pub(super) fn vpn_fact(&self, kind: catalog::VpnFact) -> String {
        use catalog::VpnFact as F;
        let selected = self.admin.vpn_selected.as_deref();
        let Some(e) = self
            .admin
            .vpn_entries
            .iter()
            .find(|e| Some(e.name()) == selected)
        else {
            return String::from("This connection was removed");
        };
        let bytes = |n: u64| match n {
            0..1024 => format!("{n} bytes"),
            1024..1_048_576 => format!("{:.1} KB", n as f64 / 1024.0),
            _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
        };
        match kind {
            F::Name => String::from(e.name()),
            F::Server => String::from(e.server()),
            F::Kind => String::from("WireGuard"),
            F::Status => self
                .admin
                .vpns
                .iter()
                .find(|v| v.0 == e.name())
                .map(|v| v.1.clone())
                .unwrap_or_default(),
            F::Address => {
                let a = e.address;
                format!("{}.{}.{}.{}/{}", a[0], a[1], a[2], a[3], e.prefix)
            }
            F::PublicKey => door_wireguard::config::encode_key(&e.public_key),
            F::Handshake => match e.handshake_age {
                u32::MAX => String::from("None yet"),
                0..60 => format!("{} seconds ago", e.handshake_age),
                s => format!("{} minutes, {} seconds ago", s / 60, s % 60),
            },
            F::Traffic => format!("{} / {}", bytes(e.rx_bytes), bytes(e.tx_bytes)),
        }
    }
    pub(super) fn net_fact(&self, kind: catalog::NetFact) -> String {
        use catalog::NetFact as F;
        use door_service_protocol::net::Dhcp;
        let ip = |a: [u8; 4]| format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3]);
        let Some(n) = self.net.filter(|n| n.present) else {
            return String::from(match kind {
                F::State => "Not connected: no network adapter was found",
                F::Adapter => "No network adapter",
                _ => "Not available",
            });
        };
        match kind {
            F::State => String::from(if !n.enabled {
                "Disabled"
            } else if n.address != [0; 4] {
                "Connected"
            } else if n.link {
                "Getting a network address"
            } else {
                "Network cable unplugged"
            }),
            F::Address if n.address != [0; 4] => format!("{} / {}", ip(n.address), n.prefix),
            F::Gateway if n.gateway != [0; 4] => ip(n.gateway),
            F::Dns => {
                let list: Vec<String> = n
                    .dns
                    .iter()
                    .filter(|a| **a != [0; 4])
                    .map(|a| ip(*a))
                    .collect();
                if list.is_empty() {
                    String::from("None")
                } else {
                    list.join(", ")
                }
            }
            F::Mac => n
                .mac
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join("-"),
            F::Assignment => String::from(match n.dhcp {
                Dhcp::Static => "Manual",
                _ => "Automatic (DHCP)",
            }),
            F::Traffic => format!(
                "Sent {} ({} packets), received {} ({} packets)",
                door_workspace::explorer_preview::size(n.bytes_out),
                n.frames_out,
                door_workspace::explorer_preview::size(n.bytes_in),
                n.frames_in
            ),
            F::Adapter => String::from("Red Hat VirtIO Ethernet Adapter"),
            F::Ipv6Address | F::LinkLocal6 | F::Dns6 => {
                let v6 = |a: [u8; 16]| format!("{}", core::net::Ipv6Addr::from(a));
                let Some(s) = self.net6.filter(|_| n.link && n.enabled) else {
                    return String::from("Not assigned");
                };
                match kind {
                    F::Ipv6Address if s.global != [0; 16] => v6(s.global),
                    // Windows shows the interface index after the address.
                    F::LinkLocal6 if s.link_local != [0; 16] => {
                        format!("{}%{}", v6(s.link_local), s.scope)
                    }
                    F::Dns6 => {
                        let list: Vec<String> = s
                            .dns
                            .iter()
                            .filter(|a| **a != [0; 16])
                            .map(|a| v6(*a))
                            .collect();
                        if list.is_empty() {
                            String::from("None")
                        } else {
                            list.join(", ")
                        }
                    }
                    _ => String::from("Not assigned"),
                }
            }
            _ => String::from("Not assigned"),
        }
    }
    /// Wall time (`operation::WALL_TIME`) in the chosen time zone; None
    /// without a valid RTC (and on host builds).
    fn local_now(&self) -> Option<door_abi::clock::WallClock> {
        local_now(self.state().zone)
    }
    fn fact(&self, fact: Fact, v: &ui::View<'_>) -> String {
        let gb = |bytes: u64| {
            let tenths = (bytes * 10 + (1 << 29)) >> 30;
            format!("{}.{} GB", tenths / 10, tenths % 10)
        };
        match fact {
            Fact::Resolution if self.output.0 > 0 => {
                format!("{} x {}", self.output.0, self.output.1)
            }
            Fact::Resolution => String::from("Not reported"),
            Fact::Scale => format!("{}%", self.effective_scale().unwrap_or(v.zoom.percent())),
            Fact::Orientation => String::from("Landscape"),
            Fact::CurrentTheme => door_workspace::settings_layout::THEMES
                .iter()
                .find(|def| self.theme_current(**def))
                .map_or(String::from("Custom"), |def| String::from(def.name)),
            Fact::VolumeUsed => self.facts.volume.map_or(String::from("Unavailable"), |u| {
                gb(u.total_bytes - u.free_bytes)
            }),
            Fact::VolumeFree => self
                .facts
                .volume
                .map_or(String::from("Unavailable"), |u| gb(u.free_bytes)),
            Fact::Processors => match self.facts.processors {
                Some(1) => String::from("x86-64 processor, 1 logical processor"),
                Some(n) => format!("x86-64 processor, {n} logical processors"),
                None => String::from("x86-64 processor"),
            },
            Fact::SystemType => String::from("64-bit operating system, x64-based processor"),
            Fact::Edition => String::from("Doors (development build)"),
            Fact::Version => String::from(env!("CARGO_PKG_VERSION")),
            Fact::NativeAbi => format!("Native ABI {}.{}", door_abi::MAJOR, door_abi::MINOR),
            Fact::CurrentTime => {
                let region = door_locale::region::region_or_default(self.state().region);
                match self.local_now() {
                    Some(now) => {
                        let mut s = String::new();
                        let _ = region.long_date(&now, &mut s);
                        s.push_str("  ");
                        let _ = region.short_time(&now, &mut s);
                        s
                    }
                    None => String::from("Unavailable: no valid real-time clock"),
                }
            }
            Fact::Region(kind) => {
                let r = door_locale::region::region_or_default(self.state().region);
                let pattern = match kind {
                    0 => return String::from(r.days[usize::from(r.first_day % 7)]),
                    1 => r.short_date,
                    2 => r.long_date,
                    3 => r.short_time,
                    _ => r.long_time,
                };
                // Examples use the current local time; without a clock the
                // pattern itself is shown rather than an invented date.
                match self.local_now() {
                    Some(now) => {
                        let mut s = String::new();
                        let _ = r.format(pattern, &now, &mut s);
                        s
                    }
                    None => String::from(pattern),
                }
            }
            Fact::DisplayLanguage => String::from("English (United States)"),
            Fact::Keyboard => String::from(self.state().input_methods().current().keyboard),
            Fact::Account => match &self.admin.own_account {
                Some((name, _)) if !name.is_empty() => name.clone(),
                _ => format!("User {}", self.uid),
            },
            Fact::AccountType => String::from(match self.admin.own_account {
                Some((_, true)) => "Administrator",
                Some((_, false)) => "Standard user",
                None => "Local account",
            }),
            Fact::Static(text) => String::from(text),
            Fact::Net(kind) => self.net_fact(kind),
            Fact::Vpn(kind) => self.vpn_fact(kind),
            Fact::Audio(kind) => self.audio_fact(kind),
            Fact::ProcessorCount => {
                match self
                    .admin
                    .hardware
                    .map(|h| h.processors)
                    .or(self.facts.processors)
                {
                    Some(1) => String::from("1 logical processor"),
                    Some(n) => format!("{n} logical processors"),
                    None => String::from("Not reported"),
                }
            }
            Fact::Chassis => String::from(
                match self
                    .admin
                    .hardware
                    .map(|h| h.illustration())
                    .or(self.facts.illustration)
                {
                    Some(door_abi::hardware::Illustration::Laptop) => "Laptop",
                    Some(door_abi::hardware::Illustration::Tablet) => "Tablet",
                    Some(door_abi::hardware::Illustration::Monitor) => "Desktop",
                    None => "Not reported",
                },
            ),
            Fact::PowerProfile => self
                .admin
                .hardware
                .map_or(String::from("Not reported"), |h| {
                    format!("Profile {}", h.power_profile)
                }),
            Fact::TimerRate => self
                .facts
                .tick_hz
                .map_or(String::from("Not reported"), |hz| format!("{hz} Hz")),
            Fact::DiskCapacity => self
                .admin
                .disk
                .map_or(String::from("Unavailable"), |d| gb(d.sectors * 512)),
            Fact::DiskStatus => String::from(if self.admin.disk.is_some() {
                "Online"
            } else {
                "Unavailable: the disk didn't answer"
            }),
            Fact::DiskAccess => self.admin.disk.map_or(String::from("Unavailable"), |d| {
                String::from(if d.writable {
                    "Read and write"
                } else {
                    "Read only"
                })
            }),
            Fact::DiskFlush => self.admin.disk.map_or(String::from("Unavailable"), |d| {
                String::from(if d.flush {
                    "Supported"
                } else {
                    "Not supported"
                })
            }),
            Fact::DiskSectors => self
                .admin
                .disk
                .map_or(String::from("Unavailable"), |d| format!("{}", d.sectors)),
            Fact::VolumeSize => self
                .admin
                .disk
                .map_or(String::from("Unavailable"), |d| gb(d.volume_sectors * 512)),
            Fact::VolumeExtent => self.admin.disk.map_or(String::from("Unavailable"), |d| {
                format!(
                    "Sectors {} to {}",
                    d.volume_first,
                    d.volume_first + d.volume_sectors - 1
                )
            }),
            Fact::JournalSize => self.admin.disk.map_or(String::from("Unavailable"), |d| {
                let mb = d.journal_sectors * 512 / (1024 * 1024);
                if mb > 0 {
                    format!("{mb} MB")
                } else {
                    format!("{} KB", d.journal_sectors / 2)
                }
            }),
            Fact::JournalExtent => self.admin.disk.map_or(String::from("Unavailable"), |d| {
                format!(
                    "Sectors {} to {}",
                    d.journal_first,
                    d.journal_first + d.journal_sectors - 1
                )
            }),
            Fact::Service(index) => String::from(
                match self
                    .admin
                    .services
                    .get(usize::from(index))
                    .copied()
                    .flatten()
                {
                    Some(true) => "Running",
                    Some(false) => "Not found",
                    None => "Unavailable: Init didn't answer",
                },
            ),
            Fact::Uid => format!("{}", self.admin.identity.uid),
            Fact::Groups => format!("Membership 0x{:X}", self.admin.identity.groups),
            Fact::Session => format!(
                "Session {}, sign-in {}",
                self.admin.identity.session, self.admin.identity.epoch
            ),
            Fact::Integrity => format!("Level {}", self.admin.identity.integrity),
            Fact::Check(index) => match self.admin.checks.get(usize::from(index)) {
                Some(Some((_, detail))) => detail.clone(),
                _ => String::from("Not run yet"),
            },
            Fact::CheckedPath => self
                .admin
                .checked
                .as_ref()
                .map_or(String::from("Not checked yet"), |(path, _)| path.clone()),
            Fact::CheckedKind => match &self.admin.checked {
                Some((_, Ok(info))) => String::from(if info.directory { "Folder" } else { "File" }),
                Some((_, Err(reason))) => String::from(*reason),
                None => String::from("Not checked yet"),
            },
            Fact::CheckedSize => match &self.admin.checked {
                Some((_, Ok(info))) if info.directory => String::from("Not applicable to folders"),
                Some((_, Ok(info))) => format!("{} bytes", info.bytes),
                Some((_, Err(_))) => String::from("Unavailable"),
                None => String::from("Not checked yet"),
            },
            Fact::RegistryPath => self.admin.location.display(),
        }
    }
    fn hot(&self, v: &ui::View<'_>) -> Hot {
        let at = |p: Option<(i32, i32)>| {
            p.and_then(|(x, y)| self.target(x, y))
                .and_then(|id| id.checked_sub(ITEM))
                .filter(|i| *i < POPUP_BACKGROUND - ITEM)
                .map(|i| i as usize)
        };
        let hover = at(v.hover);
        Hot {
            hover,
            down: at(v.pressed).filter(|d| Some(*d) == hover),
            focus: self.focus.filter(|_| self.keyboard_focus),
        }
    }
    pub(super) fn draw_page(&self, v: &mut ui::View<'_>, t: &Theme) {
        let layout = self.page_layout();
        let hot = self.hot(v);
        let top = self.scroll.value as i32 + Layout::content_top() - 8;
        let bottom = self.scroll.value as i32 + v.height as i32 + 8;
        for placed in &layout.blocks {
            if placed.rect.bottom() < top || placed.rect.y > bottom {
                continue;
            }
            self.block(v, t, placed, &layout, &hot);
        }
        if let Some((page, title, until)) = self.highlight
            && page == self.page
            && let Some(r) = catalog::section_rect(&layout, title)
        {
            let left = until.saturating_sub(self.last_tick);
            let alpha = (left.min(50) * 255 / 50) as u8;
            highlight_ring(v, t, r, alpha);
        }
        if let Some(index) = hot.focus
            && self.dialog.is_none()
            && let Some(item) = layout.items.get(index)
        {
            ring(v, t, focus_rect(item));
        }
    }
    /// A VPN connection, painted as Windows' expander: symbol, name over
    /// status, Connect (or Disconnect) and the chevron; open, Advanced
    /// options and Remove under a divider.
    #[allow(clippy::too_many_arguments)]
    fn vpn_entry(
        &self,
        v: &mut ui::View<'_>,
        t: &Theme,
        list: R,
        index: usize,
        layout: &catalog::PageLayout,
        hot: &Hot,
    ) {
        let Some((name, status, up)) = self.admin.vpns.get(index) else {
            return;
        };
        let (header, body) = catalog::vpn_entry(list, self.open, index);
        let (hover, down) = self.item_state(layout, hot, |i| {
            i.rect == header && matches!(i.action, Action::Expand(_))
        });
        let fill = if down {
            t.card_pressed
        } else if hover {
            t.card_hover
        } else {
            t.card
        };
        expander_frame(v, t, header, body.map(|b| b.bottom()), fill);
        v.mask_pixels(
            header.x + catalog::CARD_ICON,
            header.center_y() - 10,
            20,
            symbols::mask("vpn", v.scale),
            t.text,
            None,
        );
        let button = catalog::vpn_connect_button(header, *up);
        let (bh, bd) = self.item_state(layout, hot, |i| i.rect == button);
        field(
            v,
            t,
            button,
            if *up { "Disconnect" } else { "Connect" },
            true,
            bh,
            bd,
            false,
        );
        v.mask_pixels(
            header.right() - catalog::CARD_CHEVRON - 8,
            header.center_y() - 8,
            16,
            disclosure(v, body.is_some()),
            t.text,
            None,
        );
        let room = (button.x - 16 - (header.x + catalog::CARD_TEXT)).max(0) as u32;
        let line = v.line_height(TextStyle::Regular14) as i32;
        let cy = header.center_y();
        v.text_clip(
            header.x + catalog::CARD_TEXT,
            cy - line + 2,
            room,
            &fitted(v, name, room, TextStyle::Regular14),
            TextStyle::Regular14,
            t.text,
        );
        v.text_clip(
            header.x + catalog::CARD_TEXT,
            cy + 1,
            room,
            &fitted(v, status, room, TextStyle::Regular12),
            TextStyle::Regular12,
            t.secondary,
        );
        if let Some(b) = body {
            for (r, label) in [
                (catalog::vpn_options_button(b), "Advanced options"),
                (catalog::vpn_remove_button(b), "Remove"),
            ] {
                let (h, d) = self.item_state(layout, hot, |i| i.rect == r);
                field(v, t, r, label, true, h, d, false);
            }
        }
    }
    /// One card of a run-time list: symbol, title over description, and
    /// the entry's action (chevron for a subkey, Edit for a value).
    #[allow(clippy::too_many_arguments)]
    fn list_entry(
        &self,
        v: &mut ui::View<'_>,
        t: &Theme,
        list: catalog::List,
        index: usize,
        row: R,
        layout: &catalog::PageLayout,
        hot: &Hot,
    ) {
        let Some((title, description, _)) = self.admin.entry(list, index) else {
            return;
        };
        let open = list.entry_action() == Some(true);
        let (hover, down) = if open {
            self.item_state(layout, hot, |i| i.rect == row)
        } else {
            (false, false)
        };
        let fill = if down {
            t.card_pressed
        } else if hover {
            t.card_hover
        } else {
            t.card
        };
        card(v, t, row, 4, fill);
        let glyph = match list {
            catalog::List::RegistryKeys => "folder",
            catalog::List::RegistryValues => "code",
            catalog::List::UserVariables | catalog::List::LaunchVariables => "code",
            catalog::List::Events => "history",
            catalog::List::FileTypes => "apps",
            catalog::List::MicrophoneApps | catalog::List::MicrophoneActivity => "microphone",
            catalog::List::VpnConnections => "vpn",
            catalog::List::OtherUsers => "people",
            catalog::List::ApplicationEvents => "apps",
            catalog::List::SystemEvents => "history",
            catalog::List::ScheduledTasks => "history",
            catalog::List::InstalledServices => "settings",
        };
        v.mask_pixels(
            row.x + catalog::CARD_ICON,
            row.center_y() - 10,
            20,
            symbols::mask(glyph, v.scale),
            t.text,
            None,
        );
        let right = if open {
            chevron_right(v, t, (row.right() - catalog::CARD_CHEVRON, row.center_y()));
            row.right() - 52
        } else if list.entry_action() == Some(false) {
            let b = catalog::list_edit_button(row);
            let (hover, down) = self.item_state(layout, hot, |i| i.rect == b);
            let label = if list == catalog::List::FileTypes {
                "Choose"
            } else {
                "Edit"
            };
            field(v, t, b, label, true, hover, down, false);
            b.x - 16
        } else {
            row.right() - 24
        };
        let room = (right - (row.x + catalog::CARD_TEXT)).max(0) as u32;
        let line = v.line_height(TextStyle::Regular14) as i32;
        let cy = row.center_y();
        v.text_clip(
            row.x + catalog::CARD_TEXT,
            cy - line + 2,
            room,
            &fitted(v, &ui::display_ascii(&title), room, TextStyle::Regular14),
            TextStyle::Regular14,
            t.text,
        );
        v.text_clip(
            row.x + catalog::CARD_TEXT,
            cy + 1,
            room,
            &fitted(
                v,
                &ui::display_ascii(&description),
                room,
                TextStyle::Regular12,
            ),
            TextStyle::Regular12,
            t.secondary,
        );
    }
    fn item_state(
        &self,
        layout: &catalog::PageLayout,
        hot: &Hot,
        pred: impl Fn(&catalog::Item) -> bool,
    ) -> (bool, bool) {
        let index = layout.items.iter().position(pred);
        (
            index.is_some() && hot.hover == index,
            index.is_some() && hot.down == index,
        )
    }
    fn block(
        &self,
        v: &mut ui::View<'_>,
        t: &Theme,
        placed: &Placed,
        layout: &catalog::PageLayout,
        hot: &Hot,
    ) {
        let width = placed.rect.w;
        match placed.block {
            Block::Header(text) => {
                v.text_clip(
                    placed.rect.x,
                    placed.rect.y,
                    width,
                    text,
                    TextStyle::Semibold14,
                    t.text,
                );
            }
            Block::Note(text) => {
                v.text_clip(
                    placed.rect.x + 2,
                    placed.rect.y + 4,
                    width,
                    text,
                    TextStyle::Regular14,
                    t.secondary,
                );
            }
            Block::Intro(text) => {
                // The page's description, wrapped by measuring, in body text.
                let message = ui::display_ascii(text);
                let room = width.saturating_sub(4);
                let count = catalog::intro_lines(text, width) as usize;
                let lines = door_workspace::dialog::lines(&message, room, |s| v.measure(s));
                let line = v.line_height(TextStyle::Regular14) as i32;
                for (i, range) in lines.iter().take(count).enumerate() {
                    v.text_clip(
                        placed.rect.x + 2,
                        placed.rect.y + i as i32 * line,
                        room,
                        &message[range.clone()],
                        TextStyle::Regular14,
                        t.text,
                    );
                }
            }
            Block::Hero(hero) => self.hero(v, t, hero, placed.rect, layout, hot, false),
            Block::Info(text) => {
                // Windows InfoBar (informational: attention background, the
                // accent info symbol) with its message.
                let r = placed.rect;
                card(v, t, r, 4, t.card);
                v.mask_pixels(
                    r.x + 16,
                    r.y + 14,
                    20,
                    symbols::mask("info", v.scale),
                    t.accent,
                    None,
                );
                let room = width.saturating_sub(68);
                let message = ui::display_ascii(text);
                let count = catalog::info_lines(text, width) as usize;
                let lines = door_workspace::dialog::lines(&message, room, |s| v.measure(s));
                let line = v.line_height(TextStyle::Regular14) as i32;
                let shown = lines.len().min(count);
                for (i, range) in lines.iter().take(count).enumerate() {
                    // A final line that still overflows ends with "...".
                    let part = if i + 1 == shown && lines.len() > count {
                        fitted(v, &message[range.start..], room, TextStyle::Regular14)
                    } else {
                        String::from(&message[range.clone()])
                    };
                    v.text_clip(
                        r.x + 52,
                        r.y + 14 + i as i32 * line,
                        room,
                        &part,
                        TextStyle::Regular14,
                        t.text,
                    );
                }
            }
            Block::List(catalog::List::VpnConnections) => {
                for index in 0..self.admin.vpns.len() {
                    self.vpn_entry(v, t, placed.rect, index, layout, hot);
                }
            }
            Block::List(list) => {
                let count = self.admin.count(list);
                if count == 0 {
                    v.text_clip(
                        placed.rect.x + 2,
                        placed.rect.y + 4,
                        width,
                        self.admin.empty_text(list),
                        TextStyle::Regular14,
                        t.secondary,
                    );
                }
                let top = self.scroll.value as i32 + Layout::content_top() - 8;
                let bottom = self.scroll.value as i32 + v.height as i32 + 8;
                for index in 0..count {
                    let row = catalog::list_entry(placed.rect, index);
                    if row.bottom() < top || row.y > bottom {
                        continue;
                    }
                    self.list_entry(v, t, list, index, row, layout, hot);
                }
            }
            Block::Card(row) => {
                let r = placed.rect;
                let clickable = matches!(row.control, Control::Link(_) | Control::ToggleLink(..));
                let (hover, down) = if clickable {
                    self.item_state(layout, hot, |i| i.rect == r)
                } else {
                    (false, false)
                };
                let fill = if down {
                    t.card_pressed
                } else if hover {
                    t.card_hover
                } else {
                    t.card
                };
                card(v, t, r, 4, fill);
                self.row(v, t, &row, r, width, layout, hot, true);
            }
            Block::Group(g) => {
                let r = placed.rect;
                card(v, t, r, 8, t.card);
                if !g.title.is_empty() {
                    v.text(r.x + 24, r.y + 22, g.title, TextStyle::Semibold20, t.text);
                    if !g.subtitle.is_empty() {
                        let grown = v.line_height(TextStyle::Semibold20) as i32
                            - TextStyle::Semibold20.line_height() as i32;
                        v.text_clip(
                            r.x + 24,
                            r.y + 60 + grown,
                            width.saturating_sub(48),
                            g.subtitle,
                            TextStyle::Regular14,
                            t.secondary,
                        );
                    }
                }
                if let Some((hero, hr)) = placed.hero {
                    self.hero(v, t, hero, hr, layout, hot, true);
                }
                let count = placed.rows.len();
                for (index, (row, rr)) in placed.rows.iter().enumerate() {
                    if index > 0 || !g.title.is_empty() || placed.hero.is_some() {
                        v.rect(rr.x + 1, rr.y, rr.w - 2, 1, t.divider);
                    }
                    let clickable =
                        matches!(row.control, Control::Link(_) | Control::ToggleLink(..));
                    let (hover, down) = if clickable {
                        self.item_state(layout, hot, |i| i.rect == *rr)
                    } else {
                        (false, false)
                    };
                    if hover || down {
                        let fill = if down { t.card_pressed } else { t.card_hover };
                        let inner = R::new(rr.x + 1, rr.y + 1, rr.w - 2, rr.h - 1);
                        if index + 1 == count {
                            v.rounded(inner.x, inner.y, inner.w, inner.h - 1, 7, fill);
                            v.rect(inner.x, inner.y, inner.w, 8, fill);
                        } else {
                            v.rect(inner.x, inner.y, inner.w, inner.h, fill);
                        }
                    }
                    self.row(v, t, row, *rr, width, layout, hot, false);
                }
            }
            Block::Expander(e) => {
                let header = placed.header;
                let (hover, down) = self.item_state(layout, hot, |i| {
                    i.rect == header && matches!(i.action, Action::Expand(_))
                });
                let fill = if down {
                    t.card_pressed
                } else if hover {
                    t.card_hover
                } else {
                    t.card
                };
                if placed.open {
                    // Header and body read as one card split by a divider.
                    card(v, t, placed.rect, 4, t.card);
                    v.rounded(
                        header.x + 1,
                        header.y + 1,
                        header.w - 2,
                        header.h - 1,
                        3,
                        fill,
                    );
                    v.rect(header.x + 1, header.y + 4, header.w - 2, header.h - 4, fill);
                    v.rect(header.x + 1, header.bottom(), header.w - 2, 1, t.divider);
                } else {
                    card(v, t, header, 4, fill);
                }
                self.row(
                    v,
                    t,
                    &e.header,
                    catalog::expander_controls(header),
                    width,
                    layout,
                    hot,
                    true,
                );
                let chevron: &[u8] = if placed.open {
                    match v.scale {
                        3 => door_workspace::asset!(
                            "../../../../assets/icons/windows-shell/chevron-up.48.mask"
                        ),
                        2 => door_workspace::asset!(
                            "../../../../assets/icons/windows-shell/chevron-up.32.mask"
                        ),
                        _ => door_workspace::asset!(
                            "../../../../assets/icons/windows-shell/chevron-up.16.mask"
                        ),
                    }
                } else {
                    Icon::Chevron.mask_scaled(16, v.scale)
                };
                v.mask_pixels(
                    header.right() - catalog::CARD_CHEVRON - 8,
                    header.center_y() - 8,
                    16,
                    chevron,
                    t.text,
                    None,
                );
                if let Some((hero, hr)) = placed.hero {
                    self.hero(v, t, hero, hr, layout, hot, true);
                }
                for (index, (row, rr)) in placed.rows.iter().enumerate() {
                    if index > 0 || placed.hero.is_some() {
                        v.rect(rr.x + 1, rr.y, rr.w - 2, 1, t.divider);
                    }
                    self.row(v, t, row, *rr, width, layout, hot, false);
                }
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn row(
        &self,
        v: &mut ui::View<'_>,
        t: &Theme,
        row: &Row,
        r: R,
        width: u32,
        layout: &catalog::PageLayout,
        hot: &Hot,
        standalone: bool,
    ) {
        let state = self.state();
        let g = catalog::row_geometry(row, r, width);
        // Rows without a provider read as disabled, like a disabled SettingsCard.
        let enabled = !matches!(row.control, Control::Unavailable)
            && !row.description.starts_with("Unavailable");
        let cy = if catalog::wrapped(row, width) {
            catalog::wrapped_center(r)
        } else {
            r.center_y()
        };
        let title_line = v.line_height(TextStyle::Regular14) as i32;
        let fg = if enabled { t.text } else { t.disabled };
        let text_x = if row.glyph.is_empty() {
            // A card without a symbol starts at its padding (WinUI
            // SettingsCard without a HeaderIcon); rows inside a card keep
            // the text column.
            r.x + if standalone {
                catalog::CARD_PADDING
            } else {
                catalog::CARD_TEXT
            }
        } else {
            let mask = symbols::mask(row.glyph, v.scale);
            v.mask_pixels(r.x + catalog::CARD_ICON, cy - 10, 20, mask, fg, None);
            r.x + catalog::CARD_TEXT
        };
        // A read-only value keeps its full width; title and description stop
        // 16 px before it.
        let value = match row.control {
            Control::Value(fact) => {
                let text = ui::display_ascii(&self.fact(fact, v));
                let w = v
                    .measure(&text)
                    .min((g.text_right - text_x - 48).max(0) as u32);
                Some((text, w))
            }
            // A setting Doors cannot provide yet says so where its control
            // would be.
            Control::Unavailable => {
                let w = v
                    .measure(UNAVAILABLE)
                    .min((g.text_right - text_x - 48).max(0) as u32);
                Some((UNAVAILABLE.into(), w))
            }
            _ => None,
        };
        let text_right = value
            .as_ref()
            .map_or(g.text_right, |(_, w)| g.text_right - *w as i32 - 16);
        let text_width = (text_right - text_x).max(0) as u32;
        let title = fitted(v, row.title, text_width, TextStyle::Regular14);
        // A chosen scale is stored; applying it live arrives with the scale
        // engine, so until then the row says how it takes effect.
        let description =
            if self.scale_note && matches!(row.control, Control::Combo(catalog::Choice::Scale)) {
                "Sign out to apply the new scale"
            } else {
                row.description
            };
        if description.is_empty() {
            v.text_clip_centered(
                text_x,
                (cy - title_line / 2, title_line as u32),
                text_width,
                &title,
                TextStyle::Regular14,
                fg,
            );
        } else {
            v.text_clip(
                text_x,
                cy - title_line + 2,
                text_width,
                &title,
                TextStyle::Regular14,
                fg,
            );
            v.text_clip(
                text_x,
                cy + 1,
                text_width,
                &fitted(v, description, text_width, TextStyle::Regular12),
                TextStyle::Regular12,
                if enabled { t.secondary } else { t.disabled },
            );
        }
        if let Some(c) = g.chevron {
            chevron_right(v, t, c);
        }
        match row.control {
            Control::Toggle(toggle) | Control::ToggleLink(toggle, _) => {
                let sw = g.toggle.unwrap();
                let on = toggle.value(state);
                let live = toggle.live(state);
                let (hover, down) = if live {
                    self.item_state(layout, hot, |i| {
                        matches!(i.action, Action::Toggle(_))
                            && i.rect.contains(sw.x + 20, sw.center_y())
                    })
                } else {
                    (false, false)
                };
                let label = if matches!(toggle, catalog::Toggle::Unavailable(_)) {
                    UNAVAILABLE
                } else if on {
                    "On"
                } else {
                    "Off"
                };
                if width >= 400 {
                    let lw = v.measure(label);
                    v.text_clip_centered(
                        sw.x - 12 - lw as i32,
                        (sw.y - 4, 28),
                        lw + 2,
                        label,
                        TextStyle::Regular14,
                        if live { t.text } else { t.disabled },
                    );
                }
                toggle_switch(v, t, sw, on, live, hover, down);
            }
            Control::Combo(choice) => {
                let f = g.field.unwrap();
                let live = choice.enabled();
                let (hover, down) = if live {
                    self.item_state(layout, hot, |i| i.rect == f)
                } else {
                    (false, false)
                };
                let open = self
                    .popup
                    .is_some_and(|(_, k)| k == PopupKind::Choice(choice));
                field(
                    v,
                    t,
                    f,
                    choice.label(choice.current(state), state),
                    live,
                    hover || open,
                    down,
                    true,
                );
            }
            Control::Button(label) => {
                let f = g.field.unwrap();
                if width >= 400 {
                    let lw = v.measure(UNAVAILABLE);
                    v.text_clip_centered(
                        f.x - 12 - lw as i32,
                        (f.center_y() - 10, 20),
                        lw + 2,
                        UNAVAILABLE,
                        TextStyle::Regular14,
                        t.disabled,
                    );
                }
                field(v, t, f, label, false, false, false, false);
            }
            Control::Command(label, command) => {
                let f = g.field.unwrap();
                let live = self.command_enabled(command);
                let (hover, down) = if live {
                    self.item_state(layout, hot, |i| i.rect == f)
                } else {
                    (false, false)
                };
                if command.accent() && live {
                    accent_field(v, t, f, label, hover, down);
                } else {
                    field(v, t, f, label, live, hover, down, false);
                }
            }
            Control::Value(_) => {
                if let Some((text, w)) = &value {
                    v.text_clip_centered(
                        g.text_right - *w as i32,
                        (cy - 10, 20),
                        *w,
                        text,
                        TextStyle::Regular14,
                        t.secondary,
                    );
                }
            }
            Control::Slider(slider) => {
                let f = g.field.unwrap();
                let (hover, down) = self.item_state(layout, hot, |i| i.rect == f);
                let grabbed = self.slider_grab == Some(slider);
                let value = slider.value(state);
                card_slider(
                    v,
                    t,
                    f,
                    slider.thumb(f, value),
                    hover || grabbed,
                    down || grabbed,
                );
            }
            Control::Unavailable => {
                if let Some((text, w)) = &value {
                    v.text_clip_centered(
                        g.text_right - *w as i32,
                        (cy - 10, 20),
                        *w,
                        text,
                        TextStyle::Regular14,
                        t.disabled,
                    );
                }
            }
            Control::Link(_) | Control::None => (),
        }
    }
}
/// What a setting without a provider on Doors shows in place of its control.
const UNAVAILABLE: &str = "Currently unavailable";
fn toggle_switch(
    v: &mut ui::View<'_>,
    t: &Theme,
    r: R,
    on: bool,
    enabled: bool,
    hot: bool,
    down: bool,
) {
    toggle(v, t, r, on, enabled, hot, down);
}
/// A pressable status tile of the Home/System header (Windows 11 Home's
/// network and Windows Update buttons): a subtle plate on hover, darker while
/// pressed. Content is what Doors actually knows: no network adapter, and no
/// update service (so never an attention dot).
fn status_tile(v: &mut ui::View<'_>, t: &Theme, r: R, page: Page, hover: bool, down: bool) {
    if down {
        v.rounded(r.x, r.y, r.w, r.h, 4, t.rail_pressed);
    } else if hover {
        v.rounded(r.x, r.y, r.w, r.h, 4, t.rail_hover);
    }
    let cy = r.center_y();
    let (title, detail) = if page == Page::Network {
        // Network & internet's disconnected glyph (16 px).
        v.mask_pixels(
            r.x + 14,
            cy - 8,
            16,
            symbols::network_off(v.scale),
            t.text,
            None,
        );
        ("Network", "Not connected")
    } else {
        // Update & recovery's colored update artwork.
        category(v, page, r.x + 12, cy - 10, 20);
        ("Doors Update", "Unavailable on this device")
    };
    let x = r.x + 44;
    let room = (r.right() - 12 - x).max(0) as u32;
    v.text_clip(
        x,
        r.y + 9,
        room,
        &fitted(v, title, room, TextStyle::Semibold14),
        TextStyle::Semibold14,
        t.text,
    );
    v.text_clip(
        x,
        r.y + 31,
        room,
        &fitted(v, detail, room, TextStyle::Regular12),
        TextStyle::Regular12,
        t.secondary,
    );
}
/// The text size sample in the closest atlas size (exact text scaling
/// arrives with the text-scaled atlas).
fn preview_style(percent: u32) -> TextStyle {
    match 14 * percent / 100 {
        0..=15 => TextStyle::Regular14,
        16..=18 => TextStyle::Regular16,
        19..=24 => TextStyle::Regular20,
        _ => TextStyle::Semibold28,
    }
}
/// The colours a contrast theme preview shows (premultiplied BGRA): window,
/// text, hyperlink, disabled text, selection, selected text, button face and
/// button text. `None` previews the light theme.
fn preview_colors(theme: Option<door_service_protocol::desktop::ContrastTheme>) -> [[u8; 4]; 8] {
    let bgra = |c: [u8; 3]| [c[2], c[1], c[0], 255];
    match theme.map(|theme| theme.colors()) {
        Some(s) => [
            bgra(s.window),
            bgra(s.window_text),
            bgra(s.hotlight),
            bgra(s.gray_text),
            bgra(s.highlight),
            bgra(s.highlight_text),
            bgra(s.button_face),
            bgra(s.button_text),
        ],
        None => [
            [255, 255, 255, 255],
            [26, 26, 26, 255],
            [196, 103, 0, 255],
            [160, 160, 160, 255],
            [196, 103, 0, 255],
            [255, 255, 255, 255],
            [253, 253, 253, 255],
            [26, 26, 26, 255],
        ],
    }
}
/// The element a keyboard focus ring surrounds for an item.
fn focus_rect(item: &catalog::Item) -> R {
    match item.action {
        Action::Toggle(_) => R::new(item.rect.right() - 44, item.rect.center_y() - 10, 40, 20),
        _ => item.rect,
    }
}
impl Settings {
    /// Accessibility > Contrast themes: a preview of the listed theme's
    /// colours, the list, Apply and (unavailable) Edit, and the shortcut.
    fn contrast_hero(
        &self,
        v: &mut ui::View<'_>,
        t: &Theme,
        r: R,
        layout: &catalog::PageLayout,
        hot: &Hot,
    ) {
        let g = catalog::contrast_geometry(r);
        let [window, text, link, gray, selection, selected, face, button] =
            preview_colors(catalog::picked_theme(self.contrast_pick));
        // The preview shows the theme's own colours exactly, whatever theme
        // is active now.
        let active = v.palette;
        v.palette = door_workspace::palette::Palette::light();
        let p = g.preview;
        v.rounded(p.x, p.y, p.w, p.h, 8, text);
        v.rounded(p.x + 2, p.y + 2, p.w - 4, p.h - 4, 7, window);
        let x = p.x + 20;
        let room = p.w.saturating_sub(40);
        v.text_clip(x, p.y + 16, room, "Text", TextStyle::Regular14, text);
        v.text_clip(x, p.y + 46, room, "Hyperlink", TextStyle::Regular14, link);
        let underline = v.measure("Hyperlink").min(room);
        v.rect(x, p.y + 66, underline, 1, link);
        v.text_clip(
            x,
            p.y + 76,
            room,
            "Disabled text",
            TextStyle::Regular14,
            gray,
        );
        let sw = (v.measure("Selected text") + 16).min(room);
        v.rect(x, p.y + 104, sw, 24, selection);
        v.text_clip(
            x + 8,
            p.y + 106,
            sw.saturating_sub(16),
            "Selected text",
            TextStyle::Regular14,
            selected,
        );
        let bw = (v.measure("Button text") + 32).min(room);
        v.rounded(x, p.y + 144, bw, 32, 4, button);
        v.rounded(x + 2, p.y + 146, bw.saturating_sub(4), 28, 3, face);
        v.text_in_box(
            (x, p.y + 144, bw, 32),
            "Button text",
            TextStyle::Regular14,
            button,
        );
        v.palette = active;
        let state = self.state();
        let choice = catalog::Choice::Contrast;
        let (hover, down) = self.item_state(layout, hot, |i| i.rect == g.list);
        let open = self
            .popup
            .is_some_and(|(_, k)| k == PopupKind::Choice(choice));
        field(
            v,
            t,
            g.list,
            choice.label(choice.current(state), state),
            true,
            hover || open,
            down,
            true,
        );
        let (hover, down) = self.item_state(layout, hot, |i| i.rect == g.apply);
        field(v, t, g.apply, "Apply", true, hover, down, false);
        field(v, t, g.edit, "Edit", false, false, false, false);
        let line = "Press left Alt + left Shift + Print screen to turn contrast themes on and off";
        v.text_clip(
            r.x,
            g.shortcut,
            r.w,
            &fitted(v, line, r.w, TextStyle::Regular12),
            TextStyle::Regular12,
            t.secondary,
        );
    }
    /// Time & language > Typing: "Installed keyboards" with Add a keyboard,
    /// then one card per installed keyboard or input method (language,
    /// layout, honest IME status, "In use" for the current one, "...").
    fn keyboards_hero(
        &self,
        v: &mut ui::View<'_>,
        t: &Theme,
        r: R,
        layout: &catalog::PageLayout,
        hot: &Hot,
    ) {
        let state = self.state();
        let installed = state.input_methods();
        v.text_clip(
            r.x,
            r.y + 10,
            r.w.saturating_sub(170),
            "Installed keyboards",
            TextStyle::Semibold14,
            t.text,
        );
        let add = catalog::keyboard_add_button(r);
        let enabled = installed.count() < door_keyboard_layouts::methods::MAX_METHODS;
        let (hover, down) = self.item_state(layout, hot, |i| i.rect == add);
        let open = self.popup.is_some_and(|(_, k)| k == PopupKind::AddKeyboard);
        field(
            v,
            t,
            add,
            "Add a keyboard",
            enabled,
            hover || open,
            down,
            false,
        );
        let current = installed.current_index();
        for (index, method) in installed.methods().enumerate() {
            let row = catalog::keyboard_row(r, index);
            if row.bottom() < self.scroll.value as i32 + Layout::content_top()
                || row.y > self.scroll.value as i32 + v.height as i32
            {
                continue;
            }
            card(v, t, row, 4, t.card);
            v.mask_pixels(
                row.x + 20,
                row.center_y() - 10,
                20,
                symbols::mask("keyboard", v.scale),
                t.text,
                None,
            );
            let room = row.w.saturating_sub(68 + 140);
            v.text_clip(
                row.x + 60,
                row.center_y() - 20,
                room,
                &format!("{} - {}", method.language, method.keyboard),
                TextStyle::Regular14,
                t.text,
            );
            let status = catalog::keyboard_status(state, method);
            v.text_clip(
                row.x + 60,
                row.center_y() + 1,
                room,
                &fitted(v, status, room, TextStyle::Regular12),
                TextStyle::Regular12,
                t.secondary,
            );
            let button = catalog::app_menu_button(row);
            if index == current {
                let w = v.measure("In use");
                v.text(
                    button.x - 12 - w as i32,
                    row.center_y() - 10,
                    "In use",
                    TextStyle::Regular14,
                    t.secondary,
                );
            }
            let (hover, down) = self.item_state(layout, hot, |i| i.rect == button);
            let open = self
                .popup
                .is_some_and(|(_, k)| k == PopupKind::KeyboardMenu(index));
            if hover || down || open {
                v.rounded(
                    button.x,
                    button.y,
                    button.w,
                    button.h,
                    4,
                    if down {
                        t.control_pressed
                    } else {
                        t.control_hover
                    },
                );
            }
            v.mask_pixels(
                button.x + 6,
                button.y + 6,
                20,
                symbols::mask("more", v.scale),
                t.text,
                None,
            );
        }
    }
    /// Accessibility > Text size: the preview and the size slider with its
    /// value and Apply.
    fn text_size_hero(
        &self,
        v: &mut ui::View<'_>,
        t: &Theme,
        r: R,
        layout: &catalog::PageLayout,
        hot: &Hot,
    ) {
        let g = catalog::text_size_geometry(r);
        let p = g.preview;
        card(v, t, p, 8, t.card);
        v.text_clip(
            p.x + 24,
            p.y + 14,
            p.w.saturating_sub(48),
            "Text size preview",
            TextStyle::Regular12,
            t.secondary,
        );
        let style = preview_style(self.text_pick);
        let sample = "Drag the slider until the text here is easy to read";
        let room = p.w.saturating_sub(48);
        v.text_clip(
            p.x + 24,
            v.centered_text_top(p.y + 36, p.h - 44, style),
            room,
            &fitted(v, sample, room, style),
            style,
            t.text,
        );
        let c = g.card;
        card(v, t, c, 4, t.card);
        let narrow = g.slider.y > c.center_y();
        let title_room = if narrow {
            c.w.saturating_sub(48)
        } else {
            (g.slider.x - 16 - (c.x + 24)).max(0) as u32
        };
        if narrow {
            v.text_clip(
                c.x + 24,
                c.y + 16,
                title_room,
                "Text size",
                TextStyle::Regular14,
                t.text,
            );
        } else {
            v.text_clip_centered(
                c.x + 24,
                (c.y, c.h),
                title_room,
                &fitted(v, "Text size", title_room, TextStyle::Regular14),
                TextStyle::Regular14,
                t.text,
            );
        }
        let (hover, down) = self.item_state(layout, hot, |i| i.rect == g.slider);
        slider(
            v,
            t,
            g.slider,
            self.text_pick,
            hover || self.slider_grab.is_some(),
            down || self.slider_grab.is_some(),
        );
        v.text_clip_centered(
            g.value_x,
            (g.slider.y - 4, 32),
            52,
            &format!("{}%", self.text_pick),
            TextStyle::Regular14,
            t.text,
        );
        let (hover, down) = self.item_state(layout, hot, |i| i.rect == g.apply);
        let changed = self.text_pick != self.controls.text_percent();
        field(v, t, g.apply, "Apply", changed, hover, down, false);
    }
    #[allow(clippy::too_many_arguments)]
    fn hero(
        &self,
        v: &mut ui::View<'_>,
        t: &Theme,
        hero: Hero,
        r: R,
        layout: &catalog::PageLayout,
        hot: &Hot,
        in_card: bool,
    ) {
        let width = r.w;
        match hero {
            Hero::Home | Hero::Device => {
                super::desktop::thumbnail(v, r.x, r.y, self.appearance >> 1);
                let tiles = catalog::status_tiles(hero, r, width);
                // The name and edition stop 16 px before tiles beside them.
                let room = match tiles.first() {
                    Some((_, tile)) if !catalog::status_wrapped(hero, width) => {
                        (tile.x - 16 - (r.x + 140)).max(0) as u32
                    }
                    _ => width.saturating_sub(140),
                };
                v.text_clip(
                    r.x + 140,
                    r.y + 6,
                    room,
                    &fitted(v, &self.device_name, room, TextStyle::Semibold20),
                    TextStyle::Semibold20,
                    t.text,
                );
                v.text_clip(
                    r.x + 140,
                    r.y + 36,
                    room,
                    &fitted(v, "Doors", room, TextStyle::Regular14),
                    TextStyle::Regular14,
                    t.text,
                );
                v.text_clip(
                    r.x + 140,
                    r.y + 57,
                    room.min(120),
                    "Rename",
                    TextStyle::Regular14,
                    t.disabled,
                );
                for (page, tile) in tiles {
                    let (hover, down) = self.item_state(layout, hot, |i| i.rect == tile);
                    status_tile(v, t, tile, page, hover, down);
                }
            }
            Hero::Network => {
                card(v, t, r, 8, t.card);
                v.mask_pixels(
                    r.x + 24,
                    r.center_y() - 10,
                    20,
                    symbols::mask("ethernet", v.scale),
                    t.text,
                    None,
                );
                v.text(
                    r.x + 64,
                    r.y + 12,
                    "Ethernet",
                    TextStyle::Semibold20,
                    t.text,
                );
                let room = if width >= 560 {
                    width - 140 - 16 - 64
                } else {
                    width.saturating_sub(88)
                };
                let status = self.net_fact(catalog::NetFact::State);
                v.text_clip(
                    r.x + 64,
                    r.y + 42,
                    room,
                    &fitted(v, &status, room, TextStyle::Regular14),
                    TextStyle::Regular14,
                    t.secondary,
                );
                if width >= 560 {
                    field(
                        v,
                        t,
                        R::new(r.right() - 140, r.center_y() - 16, 116, 32),
                        "Properties",
                        false,
                        false,
                        false,
                        false,
                    );
                }
            }
            Hero::Update => {
                card(v, t, r, 8, t.card);
                category(v, Page::Update, r.x + 20, r.y + 16, 32);
                v.text(
                    r.x + 68,
                    r.y + 12,
                    "Updates unavailable",
                    TextStyle::Semibold20,
                    t.text,
                );
                let room = if width >= 560 {
                    width - 180 - 16 - 68
                } else {
                    width.saturating_sub(92)
                };
                v.text_clip(
                    r.x + 68,
                    r.y + 42,
                    room,
                    &fitted(
                        v,
                        "No update service is installed on this device",
                        room,
                        TextStyle::Regular14,
                    ),
                    TextStyle::Regular14,
                    t.secondary,
                );
                if width >= 560 {
                    field(
                        v,
                        t,
                        R::new(r.right() - 180, r.center_y() - 16, 156, 32),
                        "Check for updates",
                        false,
                        false,
                        false,
                        false,
                    );
                }
            }
            Hero::Account => {
                // Windows: the account's name, then "Local account" and its
                // type under it.
                avatar(v, t, r.x, r.y + 4, 96);
                v.text_clip(
                    r.x + 116,
                    r.y + 22,
                    width.saturating_sub(116),
                    &self.fact(Fact::Account, v),
                    TextStyle::Semibold20,
                    t.text,
                );
                v.text_clip(
                    r.x + 116,
                    r.y + 54,
                    width.saturating_sub(116),
                    "Local account",
                    TextStyle::Regular14,
                    t.secondary,
                );
                v.text_clip(
                    r.x + 116,
                    r.y + 76,
                    width.saturating_sub(116),
                    &self.fact(Fact::AccountType, v),
                    TextStyle::Regular14,
                    t.secondary,
                );
            }
            Hero::Contrast => self.contrast_hero(v, t, r, layout, hot),
            Hero::TextSize => self.text_size_hero(v, t, r, layout, hot),
            Hero::Bluetooth => {
                if !in_card {
                    card(v, t, r, 4, t.card);
                }
                let cy = r.center_y();
                v.mask_pixels(
                    r.x + 24,
                    cy - 10,
                    20,
                    symbols::mask("devices", v.scale),
                    t.disabled,
                    None,
                );
                v.text(
                    r.x + 64,
                    cy - 20,
                    "Bluetooth",
                    TextStyle::Regular14,
                    t.disabled,
                );
                let room = width.saturating_sub(190);
                v.text_clip(
                    r.x + 64,
                    cy + 1,
                    room,
                    &fitted(
                        v,
                        "Unavailable: no Bluetooth adapter on this device",
                        room,
                        TextStyle::Regular12,
                    ),
                    TextStyle::Regular12,
                    t.disabled,
                );
                let sw = R::new(r.right() - 64, cy - 10, 40, 20);
                if width >= 400 {
                    v.text_clip_centered(
                        sw.x - 12 - v.measure("Off") as i32,
                        (sw.y - 4, 28),
                        40,
                        "Off",
                        TextStyle::Regular14,
                        t.disabled,
                    );
                }
                toggle(v, t, sw, false, false, false, false);
            }
            Hero::LockPreview => self.lock_preview(v, t, r),
            Hero::Wallpaper => {
                super::desktop::theme_preview(v, t, r.x, r.y, self.appearance, self.solid())
            }
            Hero::Personalization => {
                let split = catalog::personalization_split(width);
                let preview_y = if split {
                    r.y + catalog::PERSONALIZATION_LABEL + 2
                } else {
                    r.y
                };
                super::desktop::theme_preview(v, t, r.x, preview_y, self.appearance, self.solid());
                let (x, y) = if split {
                    (r.x + catalog::PERSONALIZATION_GALLERY_X, r.y)
                } else {
                    (r.x, r.y + catalog::THEME_PREVIEW.1 as i32 + 24)
                };
                v.text(
                    x,
                    y + 2,
                    "Select a theme to apply",
                    TextStyle::Semibold14,
                    t.text,
                );
                self.tiles(v, t, hero, r, layout, hot);
            }
            Hero::Themes => self.tiles(v, t, hero, r, layout, hot),
            Hero::ThemePreview => {
                super::desktop::theme_preview(v, t, r.x, r.y, self.appearance, self.solid());
                let (entries, save) = catalog::theme_preview_geometry(r, width);
                let background = door_service_protocol::preferences::Background::from_bits(
                    self.state()
                        .pref(door_service_protocol::preferences::BACKGROUND),
                )
                .map_or(0, |b| b.kind);
                let picture = match background {
                    1 => "Solid color",
                    2 => "Slideshow",
                    _ => door_workspace::settings_layout::BACKGROUNDS
                        .get((self.appearance >> 1) as usize)
                        .copied()
                        .unwrap_or("Bloom"),
                };
                let color =
                    if self.accent() == door_service_protocol::desktop::Accent::DEFAULT.bits() {
                        "Blue"
                    } else {
                        "Custom"
                    };
                for (i, (glyph, title, value)) in [
                    ("g-e91b", "Background", picture),
                    ("color", "Color", color),
                    ("volume", "Sounds", "Doors Default"),
                    ("pointer", "Mouse cursor", "Doors Default"),
                ]
                .into_iter()
                .enumerate()
                {
                    let e = entries[i];
                    let (hover, down) = self.item_state(layout, hot, |item| item.rect == e);
                    if down {
                        v.rounded(e.x, e.y, e.w, e.h, 4, t.card_pressed);
                    } else if hover {
                        v.rounded(e.x, e.y, e.w, e.h, 4, t.card_hover);
                    }
                    let mask = symbols::mask(glyph, v.scale);
                    v.mask_pixels(e.x + 10, e.y + 10, 20, mask, t.accent, None);
                    v.text_clip(
                        e.x + 44,
                        e.y + 4,
                        e.w - 48,
                        title,
                        TextStyle::Regular14,
                        t.text,
                    );
                    v.text_clip(
                        e.x + 44,
                        e.y + 22,
                        e.w - 48,
                        value,
                        TextStyle::Regular14,
                        t.text,
                    );
                }
                field(v, t, save, "Save", false, false, false, false);
            }
            Hero::HomeThemes => self.tiles(v, t, hero, r, layout, hot),
            Hero::Accents => {
                v.text(
                    r.x + 58,
                    r.y + 11,
                    "Recent colors",
                    TextStyle::Regular14,
                    t.text,
                );
                v.text(
                    r.x + 58,
                    r.y + catalog::ACCENT_WINDOWS_Y - 25,
                    "Doors colors",
                    TextStyle::Regular14,
                    t.text,
                );
                self.tiles(v, t, hero, r, layout, hot);
            }
            Hero::SolidColors => {
                v.text(
                    r.x + 58,
                    r.y + 10,
                    "Choose your background color",
                    TextStyle::Regular14,
                    t.text,
                );
                self.tiles(v, t, hero, r, layout, hot);
            }
            Hero::Backgrounds => {
                v.text(
                    r.x + 58,
                    r.y + 10,
                    "Recent images",
                    TextStyle::Regular14,
                    t.text,
                );
                self.tiles(v, t, hero, r, layout, hot);
            }
            Hero::Monitor => {
                card(v, t, r, 8, t.card);
                let x = r.x + (width as i32 - 180) / 2;
                v.rounded(x, r.y + 30, 180, 102, 4, t.accent);
                v.text_centered(
                    x + 82,
                    r.y + 30,
                    102,
                    "1",
                    TextStyle::Semibold28,
                    t.on_accent,
                );
                v.text_clip(
                    r.x + 24,
                    r.y + 148,
                    width.saturating_sub(48),
                    "One display connected",
                    TextStyle::Regular14,
                    t.secondary,
                );
            }
            Hero::Storage => {
                card(v, t, r, 8, t.card);
                v.large_icon(r.x + 20, r.y + 18, Icon::Drive);
                v.text(
                    r.x + 72,
                    r.y + 16,
                    "Local Disk (C:)",
                    TextStyle::Semibold14,
                    t.text,
                );
                let bar = R::new(r.x + 72, r.y + 70, width.saturating_sub(96), 6);
                v.rounded(bar.x, bar.y, bar.w, bar.h, 3, t.divider);
                match self.facts.volume {
                    Some(u) if u.total_bytes > 0 => {
                        let used = u.total_bytes - u.free_bytes;
                        v.text(
                            r.x + 72,
                            r.y + 38,
                            &self.fact(Fact::VolumeUsed, v).replace(" GB", " GB used"),
                            TextStyle::Regular14,
                            t.text,
                        );
                        let fill = ((used as u128 * bar.w as u128) / u.total_bytes as u128) as u32;
                        if fill > 0 {
                            v.rounded(bar.x, bar.y, fill.max(6), bar.h, 3, t.accent);
                        }
                        let free = format!("{} free", self.fact(Fact::VolumeFree, v));
                        let fw = v.measure_small(&free);
                        v.text(
                            bar.right() - fw as i32,
                            r.y + 84,
                            &free,
                            TextStyle::Regular12,
                            t.secondary,
                        );
                        let total = (u.total_bytes * 10 + (1 << 29)) >> 30;
                        v.text(
                            r.x + 72,
                            r.y + 84,
                            &format!("{}.{} GB total", total / 10, total % 10),
                            TextStyle::Regular12,
                            t.secondary,
                        );
                    }
                    _ => {
                        v.text(
                            r.x + 72,
                            r.y + 38,
                            "Capacity unavailable",
                            TextStyle::Regular14,
                            t.secondary,
                        );
                    }
                }
            }
            Hero::AboutTiles => {
                // 26H2 About: Storage, Graphics card, Installed RAM and
                // Processor tiles with what this device reports ("—" for
                // what it doesn't).
                let gb = |bytes: u64| {
                    let tenths = (bytes * 10 + (1 << 29)) >> 30;
                    format!("{}.{} GB", tenths / 10, tenths % 10)
                };
                let storage = match self.facts.volume {
                    Some(u) if u.total_bytes > 0 => (
                        gb(u.total_bytes),
                        format!("{} of {} used", gb(u.used_bytes()), gb(u.total_bytes)),
                    ),
                    _ => (String::from("—"), String::from("Capacity unavailable")),
                };
                let processors = match self.facts.processors {
                    Some(1) => String::from("1 logical processor"),
                    Some(n) => format!("{n} logical processors"),
                    None => String::from("Not reported"),
                };
                let tiles: [(&str, &str, String, String); 4] = [
                    ("storage", "Storage", storage.0, storage.1),
                    (
                        "display",
                        "Graphics Card",
                        String::from("—"),
                        String::from("UEFI graphics output"),
                    ),
                    (
                        "devices",
                        "Installed RAM",
                        String::from("—"),
                        String::from("Not reported by this device"),
                    ),
                    ("devices", "Processor", String::from("x86-64"), processors),
                ];
                for (tile, (glyph, label, value, detail)) in
                    catalog::about_tiles(r).into_iter().zip(tiles)
                {
                    card(v, t, tile, 8, t.card);
                    let room = tile.w.saturating_sub(34);
                    v.mask_pixels(
                        tile.x + 17,
                        tile.y + 15,
                        16,
                        symbols::mask(glyph, v.scale),
                        t.secondary,
                        None,
                    );
                    v.text_clip(
                        tile.x + 37,
                        tile.y + 14,
                        room.saturating_sub(20),
                        label,
                        TextStyle::Regular12,
                        t.text,
                    );
                    v.text_clip(
                        tile.x + 17,
                        tile.y + 38,
                        room,
                        &fitted(v, &value, room, TextStyle::Semibold16),
                        TextStyle::Semibold16,
                        t.text,
                    );
                    v.text_clip(
                        tile.x + 17,
                        tile.bottom() - 34,
                        room,
                        &fitted(v, &detail, room, TextStyle::Regular12),
                        TextStyle::Regular12,
                        t.text,
                    );
                }
            }
            Hero::Clock => {
                // 26H2 Date & time: the time in a large semibold line and
                // the long date under it; time zone and region at the right.
                let region = door_locale::region::region_or_default(self.state().region);
                let zone = door_locale::tz::zone_or_utc(self.state().zone);
                let (time, date) = match self.local_now() {
                    Some(now) => {
                        let (mut time, mut date) = (String::new(), String::new());
                        let _ = region.short_time(&now, &mut time);
                        let _ = region.long_date(&now, &mut date);
                        (time, date)
                    }
                    None => (String::from("—"), String::from("No valid real-time clock")),
                };
                let right = width >= 640;
                let left_room = if right { width / 2 } else { width };
                v.text_clip(r.x, r.y, left_room, &time, TextStyle::Semibold28, t.text);
                v.text_clip(
                    r.x,
                    r.y + 40,
                    left_room,
                    &fitted(
                        v,
                        &ui::display_ascii(&date),
                        left_room,
                        TextStyle::Regular14,
                    ),
                    TextStyle::Regular14,
                    t.text,
                );
                if right {
                    let x0 = r.x + width as i32 / 2 + 16;
                    let room = width / 2 - 16;
                    let region_w = 150u32.min(room / 3);
                    let zone_room = room.saturating_sub(region_w + 36 + 24);
                    for (x, w, glyph, label, value) in [
                        (x0, zone_room, "world", "Time zone", zone.display),
                        (
                            r.right() - region_w as i32,
                            region_w.saturating_sub(36),
                            "location",
                            "Region",
                            region.name,
                        ),
                    ] {
                        v.mask_pixels(x, r.y + 14, 20, symbols::mask(glyph, v.scale), t.text, None);
                        v.text_clip(x + 36, r.y + 10, w, label, TextStyle::Regular14, t.text);
                        v.text_clip(
                            x + 36,
                            r.y + 31,
                            w,
                            &fitted(v, value, w, TextStyle::Regular12),
                            TextStyle::Regular12,
                            t.secondary,
                        );
                    }
                }
            }
            Hero::AboutDevice => {
                // 26H2: the device name card (no picture) with Rename this PC.
                card(v, t, r, 4, t.card);
                let button_w = 130u32.min(width / 3);
                let room = width.saturating_sub(button_w + 17 + 34);
                v.text_clip(
                    r.x + 17,
                    r.y + 17,
                    room,
                    &self.device_name,
                    TextStyle::Regular14,
                    t.text,
                );
                let model = match self.facts.illustration {
                    Some(door_abi::hardware::Illustration::Laptop) => "Doors laptop",
                    Some(door_abi::hardware::Illustration::Tablet) => "Doors tablet",
                    _ => "Doors desktop",
                };
                v.text_clip(
                    r.x + 17,
                    r.y + 37,
                    room,
                    model,
                    TextStyle::Regular12,
                    t.secondary,
                );
                if width >= 360 {
                    let b = catalog::about_rename_button(r);
                    let (hover, down) = self.item_state(layout, hot, |i| i.rect == b);
                    field(v, t, b, "Rename this PC", true, hover, down, false);
                    let _ = button_w;
                }
            }
            Hero::DeviceSpecs | Hero::DoorsSpecs => {
                card(v, t, r, 4, t.card);
                let header = catalog::card() as i32;
                let cy = r.y + header / 2;
                if hero == Hero::DeviceSpecs {
                    v.mask_pixels(
                        r.x + catalog::CARD_ICON,
                        cy - 10,
                        20,
                        symbols::mask("info", v.scale),
                        t.text,
                        None,
                    );
                } else {
                    doors_logo(v, r.x + catalog::CARD_ICON - 2, cy - 12, 24);
                }
                v.text_clip_centered(
                    r.x + catalog::CARD_TEXT,
                    (r.y, header as u32),
                    width.saturating_sub(200),
                    if hero == Hero::DeviceSpecs {
                        "Device specifications"
                    } else {
                        "Doors specifications"
                    },
                    TextStyle::Regular14,
                    t.text,
                );
                if width >= 420 {
                    field(
                        v,
                        t,
                        R::new(r.right() - catalog::CARD_RIGHT - 90, cy - 16, 90, 32),
                        "Copy",
                        false,
                        false,
                        false,
                        false,
                    );
                }
                v.rect(r.x + 1, r.y + header, r.w - 2, 1, t.divider);
                let facts: &[(&str, Fact)] = if hero == Hero::DeviceSpecs {
                    &[
                        ("Device name", Fact::Static("DOORS-PC")),
                        ("Processor", Fact::Processors),
                        ("Installed RAM", Fact::Static("Not reported by this device")),
                        ("System type", Fact::SystemType),
                        (
                            "Pen and touch",
                            Fact::Static("No pen or touch input is available for this display"),
                        ),
                    ]
                } else {
                    &[
                        ("Edition", Fact::Edition),
                        ("Version", Fact::Version),
                        ("OS build", Fact::NativeAbi),
                        ("Scheduler", Fact::Static("")),
                    ]
                };
                let column = if width >= 640 {
                    catalog::SPECS_VALUE - catalog::CARD_TEXT
                } else {
                    110
                };
                for (index, (label, fact)) in facts.iter().enumerate() {
                    let y = r.y + header + 15 + index as i32 * catalog::SPECS_ROW;
                    let value = match fact {
                        Fact::Static("DOORS-PC") => self.device_name.clone(),
                        Fact::Static("") => {
                            self.facts.tick_hz.map_or(String::from("Preemptive"), |hz| {
                                format!("Preemptive, {hz} Hz timer")
                            })
                        }
                        _ => self.fact(*fact, v),
                    };
                    v.text_clip(
                        r.x + catalog::CARD_TEXT,
                        y,
                        column as u32 - 8,
                        label,
                        TextStyle::Regular14,
                        t.text,
                    );
                    let room = width.saturating_sub((catalog::CARD_TEXT + column) as u32 + 24);
                    v.text_clip(
                        r.x + catalog::CARD_TEXT + column,
                        y,
                        room,
                        &fitted(v, &ui::display_ascii(&value), room, TextStyle::Regular14),
                        TextStyle::Regular14,
                        t.secondary,
                    );
                }
                if hero == Hero::DeviceSpecs {
                    // Related links (26H2): a strip at the bottom of the card.
                    let top = r.bottom() - catalog::ABOUT_LINKS_HEIGHT as i32;
                    v.rect(r.x + 1, top, r.w - 2, 1, t.divider);
                    v.text_clip(
                        r.x + 17,
                        top + 18,
                        100,
                        "Related links",
                        TextStyle::Semibold14,
                        t.text,
                    );
                    for ((link, page), (label, _)) in catalog::about_links(r)
                        .into_iter()
                        .zip(catalog::ABOUT_LINKS)
                    {
                        if link.w == 0 {
                            continue;
                        }
                        let (hover, down) = if page.is_some() {
                            self.item_state(layout, hot, |i| i.rect == link)
                        } else {
                            (false, false)
                        };
                        let color = if page.is_none() {
                            t.disabled
                        } else if down {
                            t.secondary
                        } else if hover {
                            t.accent_hover
                        } else {
                            t.accent
                        };
                        v.text_clip(
                            link.x,
                            link.y + 1,
                            link.w,
                            &fitted(v, label, link.w, TextStyle::Regular14),
                            TextStyle::Regular14,
                            color,
                        );
                    }
                }
            }
            Hero::Keyboards => self.keyboards_hero(v, t, r, layout, hot),
            Hero::Apps => {
                let count = INCLUDED_APPS.len() + self.admin.programs.len();
                v.text(
                    r.x,
                    r.y + 14,
                    &format!("{count} apps found"),
                    TextStyle::Regular14,
                    t.secondary,
                );
                // Every app by name, as Windows sorts them: the included
                // apps and the desktop programs installers registered.
                for (index, entry) in self.app_order().into_iter().enumerate() {
                    let row = catalog::app_row(r, index);
                    if row.bottom() < self.scroll.value as i32 + Layout::content_top()
                        || row.y > self.scroll.value as i32 + v.height as i32
                    {
                        continue;
                    }
                    card(v, t, row, 4, t.card);
                    let (name, details, size): (&str, String, String) = match entry {
                        Ok(included) => {
                            let (name, command) = INCLUDED_APPS[included];
                            app_icon(v, row.x + 20, row.center_y() - 16, command, included);
                            (
                                name,
                                String::from("Macrohard Corporation  |  Included with Doors"),
                                String::new(),
                            )
                        }
                        Err(program) => {
                            let p = &self.admin.programs[program];
                            v.sized_icon(row.x + 20, row.center_y() - 16, 32, Icon::Application);
                            (p.name.as_str(), p.details(), p.size())
                        }
                    };
                    let size_width: u32 = if size.is_empty() { 0 } else { 96 };
                    v.text_clip(
                        row.x + 68,
                        row.center_y() - 20,
                        row.w.saturating_sub(140 + size_width),
                        name,
                        TextStyle::Regular14,
                        t.text,
                    );
                    v.text_clip(
                        row.x + 68,
                        row.center_y() + 1,
                        row.w.saturating_sub(140 + size_width),
                        &details,
                        TextStyle::Regular12,
                        t.secondary,
                    );
                    let button = catalog::app_menu_button(row);
                    if !size.is_empty() {
                        let w = v.measure_style(&size, TextStyle::Regular14) as i32;
                        v.text(
                            button.x - 16 - w,
                            row.center_y() - 10,
                            &size,
                            TextStyle::Regular14,
                            t.secondary,
                        );
                    }
                    let (hover, down) = self.item_state(layout, hot, |i| i.rect == button);
                    let open = self.popup.is_some_and(|(_, k)| {
                        k == PopupKind::AppMenu(index) || k == PopupKind::ConfirmUninstall(index)
                    });
                    if hover || down || open {
                        v.rounded(
                            button.x,
                            button.y,
                            button.w,
                            button.h,
                            4,
                            if down {
                                t.control_pressed
                            } else {
                                t.control_hover
                            },
                        );
                    }
                    v.mask_pixels(
                        button.x + 6,
                        button.y + 6,
                        20,
                        symbols::mask("more", v.scale),
                        t.text,
                        None,
                    );
                }
            }
        }
    }
    /// Lock screen (26H2): the lock picture in the preview bezel with the
    /// time and the date (no year) in white near the top.
    fn lock_preview(&self, v: &mut ui::View<'_>, t: &Theme, r: R) {
        use door_service_protocol::preferences::{self as p, LockScreen};
        let (w, h) = catalog::THEME_PREVIEW;
        if v.palette.system().is_some() {
            super::desktop::theme_preview(v, t, r.x, r.y, self.appearance, self.solid());
            return;
        }
        let lock =
            LockScreen::from_bits(self.state().pref(p::LOCK_SCREEN)).unwrap_or(LockScreen::DESKTOP);
        v.rounded_asset(r.x, r.y, w, h, 8, [0, 0, 0, 255]);
        let (x, y, iw, ih) = (r.x + 8, r.y + 8, w - 16, h - 16);
        let solid = match lock.kind() {
            LockScreen::SOLID => Some(lock.bits() >> 8),
            0 => self.solid(),
            _ => None,
        };
        match solid {
            Some(rgb) => v.rounded_asset(
                x,
                y,
                iw,
                ih,
                2,
                [rgb as u8, (rgb >> 8) as u8, (rgb >> 16) as u8, 255],
            ),
            None => {
                let picture = match lock.kind() {
                    k @ 1..=4 => k - 1,
                    _ => self.appearance >> 1,
                };
                wallpaper_cover(v, x, y, iw, ih, picture, 2);
            }
        }
        let Some(now) = self.local_now() else {
            return;
        };
        let region = door_locale::region::region_or_default(self.state().region);
        let (mut time, mut date) = (String::new(), String::new());
        let _ = region.short_time(&now, &mut time);
        let _ = region.long_date(&now, &mut date);
        // The lock screen shows the date without its year.
        let digits = date.len() - date.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 4 {
            date.truncate(date.len() - 4);
            date.truncate(date.trim_end_matches([',', ' ']).len());
        }
        let date = ui::display_ascii(&date);
        let white = [255, 255, 255, 255];
        for (text, style, top) in [
            (time.as_str(), TextStyle::Semibold28, 18),
            (date.as_str(), TextStyle::Semibold14, 62),
        ] {
            let tw = v.measure_style(text, style).min(iw);
            v.text_clip(
                r.x + (w as i32 - tw as i32) / 2,
                r.y + top,
                iw,
                text,
                style,
                white,
            );
        }
    }
    /// The solid desktop color while the background is Solid color.
    fn solid(&self) -> Option<u32> {
        use door_service_protocol::preferences::{self as p, Background};
        Background::from_bits(self.state().pref(p::BACKGROUND))
            .filter(|b| b.kind == 1)
            .map(|b| if b.color == 0 { 0x0063B1 } else { b.color })
    }
    /// Gallery tiles of a hero (themes, backgrounds, taskbar positions).
    fn tiles(
        &self,
        v: &mut ui::View<'_>,
        t: &Theme,
        hero: Hero,
        r: R,
        layout: &catalog::PageLayout,
        hot: &Hot,
    ) {
        for index in 0..catalog::hero_count(hero) {
            let Some(tile) = catalog::tile(hero, r, index, r.w) else {
                continue;
            };
            let (hover, down) = self.item_state(layout, hot, |i| i.rect == tile);
            match hero {
                Hero::Accents => {
                    // Slots 0..5 are Recent colors (only the used ones).
                    let color = if index < catalog::RECENT_ACCENTS {
                        match self.recent_accents().colors().nth(index) {
                            Some(color) => color as usize,
                            None => continue,
                        }
                    } else {
                        index - catalog::RECENT_ACCENTS
                    };
                    let rgb = door_design_tokens::accent::WINDOWS_COLORS[color];
                    let bgra = [rgb as u8, (rgb >> 8) as u8, (rgb >> 16) as u8, 255];
                    let selected = index >= catalog::RECENT_ACCENTS
                        && door_service_protocol::desktop::Accent::from_bits(self.accent())
                            .is_some_and(|a| {
                                a.mode() == door_service_protocol::desktop::AccentMode::Manual(rgb)
                            });
                    if selected {
                        // 26H2: a 2 px text-colored frame and a 20 px badge
                        // with a check in the top-right corner (theme colors
                        // already, so no surface remapping).
                        v.rounded_asset(tile.x, tile.y, tile.w, tile.h, 4, t.text);
                        v.rounded_asset(tile.x + 2, tile.y + 2, tile.w - 4, tile.h - 4, 2, bgra);
                        let badge = (tile.x + tile.w as i32 - 22, tile.y + 2);
                        v.rounded_asset(badge.0, badge.1, 20, 20, 0, t.text);
                        let check = symbols::mask("g-e73e", v.scale);
                        v.mask_pixels(badge.0, badge.1, 20, check, t.card, None);
                    } else {
                        if hover {
                            v.rounded(tile.x - 2, tile.y - 2, tile.w + 4, tile.h + 4, 5, t.strong);
                        }
                        swatch(v, tile, 4, bgra);
                    }
                    let _ = down;
                }
                Hero::SolidColors => {
                    use door_service_protocol::preferences::{self as p, Background};
                    let rgb = catalog::SOLID_PALETTE[index];
                    let word = Background::from_bits(self.state().pref(p::BACKGROUND))
                        .unwrap_or(Background::DEFAULT);
                    if word.kind == 1 && word.color == rgb {
                        // Theme colors already: no surface remapping.
                        v.rounded_asset(tile.x - 3, tile.y - 3, tile.w + 6, tile.h + 6, 6, t.text);
                        v.rounded_asset(tile.x - 1, tile.y - 1, tile.w + 2, tile.h + 2, 5, t.card);
                    } else if hover {
                        v.rounded(tile.x - 2, tile.y - 2, tile.w + 4, tile.h + 4, 5, t.strong);
                    }
                    // Exact colors: the palette's whites and grays are not
                    // theme surfaces.
                    let color = [rgb as u8, (rgb >> 8) as u8, (rgb >> 16) as u8, 255];
                    swatch(v, tile, 4, color);
                    let _ = down;
                }
                Hero::Backgrounds => {
                    use door_service_protocol::preferences::{self as p, RecentBackgrounds};
                    let recent =
                        RecentBackgrounds::from_bits(self.state().pref(p::BACKGROUND_RECENT))
                            .unwrap_or(RecentBackgrounds::DEFAULT);
                    let background = recent.0[index];
                    // Windows marks no recent image as current; hover only.
                    if hover {
                        v.rounded(tile.x - 2, tile.y - 2, tile.w + 4, tile.h + 4, 6, t.strong);
                    }
                    wallpaper_cover(v, tile.x, tile.y, tile.w, tile.h, background, 4);
                    let _ = (down, BACKGROUNDS[background as usize]);
                }
                _ => {
                    let def = THEMES[if hero == Hero::Themes {
                        index
                    } else {
                        GALLERY[index]
                    }];
                    let selected = self.theme_current(def);
                    super::desktop::theme_tile(v, t, tile, def, selected, hover);
                }
            }
        }
    }
}
/// A color swatch in its exact color; contrast themes map it onto the
/// system colors like every other surface.
fn swatch(v: &mut ui::View<'_>, tile: R, radius: u32, bgra: [u8; 4]) {
    if v.palette.system().is_some() {
        v.rounded(tile.x, tile.y, tile.w, tile.h, radius, bgra);
    } else {
        v.rounded_asset(tile.x, tile.y, tile.w, tile.h, radius, bgra);
    }
}
fn app_icon(v: &mut ui::View<'_>, x: i32, y: i32, command: u8, index: usize) {
    let pixels: Option<&[u8]> = match (command, v.scale) {
        (9, 3) => Some(door_workspace::asset!(
            "../../../../assets/icons/windows/terminal-current.96.bgra"
        )),
        (9, 2) => Some(door_workspace::asset!(
            "../../../../assets/icons/windows/terminal-current.64.bgra"
        )),
        (9, _) => Some(door_workspace::asset!(
            "../../../../assets/icons/windows/terminal-current.32.bgra"
        )),
        (10, 3) => Some(door_workspace::asset!(
            "../../../../assets/branding/preview.96.bgra"
        )),
        (10, 2) => Some(door_workspace::asset!(
            "../../../../assets/branding/preview.64.bgra"
        )),
        (10, _) => Some(door_workspace::asset!(
            "../../../../assets/branding/preview.32.bgra"
        )),
        (11, 3) => Some(door_workspace::asset!(
            "../../../../assets/icons/windows/taskmgr.96.bgra"
        )),
        (11, 2) => Some(door_workspace::asset!(
            "../../../../assets/icons/windows/taskmgr.64.bgra"
        )),
        (11, _) => Some(door_workspace::asset!(
            "../../../../assets/icons/windows/taskmgr.32.bgra"
        )),
        _ => None,
    };
    match pixels {
        Some(pixels) => v.color_pixels(x, y, 32, pixels),
        None => v.sized_icon(
            x,
            y,
            32,
            [Icon::Explorer, Icon::Notepad, Icon::Settings][index.min(2)],
        ),
    }
}
/// An expander's card: the header alone, or (open, `bottom` given) header
/// and body as one card split by a divider, as `Block::Expander` paints it.
fn expander_frame(v: &mut ui::View<'_>, t: &Theme, header: R, bottom: Option<i32>, fill: [u8; 4]) {
    match bottom {
        Some(bottom) => {
            card(
                v,
                t,
                R {
                    h: (bottom - header.y).max(0) as u32,
                    ..header
                },
                4,
                t.card,
            );
            v.rounded(
                header.x + 1,
                header.y + 1,
                header.w - 2,
                header.h - 1,
                3,
                fill,
            );
            v.rect(header.x + 1, header.y + 4, header.w - 2, header.h - 4, fill);
            v.rect(header.x + 1, header.bottom(), header.w - 2, 1, t.divider);
        }
        None => card(v, t, header, 4, fill),
    }
}
/// The disclosure chevron: down while closed, up while open.
fn disclosure(v: &ui::View<'_>, open: bool) -> &'static [u8] {
    if !open {
        return Icon::Chevron.mask_scaled(16, v.scale);
    }
    match v.scale {
        3 => door_workspace::asset!("../../../../assets/icons/windows-shell/chevron-up.48.mask"),
        2 => door_workspace::asset!("../../../../assets/icons/windows-shell/chevron-up.32.mask"),
        _ => door_workspace::asset!("../../../../assets/icons/windows-shell/chevron-up.16.mask"),
    }
}
/// A WinUI AccentButton (the page's primary action).
#[allow(clippy::too_many_arguments)]
pub(super) fn accent_field(
    v: &mut ui::View<'_>,
    t: &Theme,
    r: R,
    label: &str,
    hot: bool,
    down: bool,
) {
    // Pressed (AccentFillColorTertiary) moves twice as far from the
    // accent as hover (Secondary) does.
    let fill = if down {
        core::array::from_fn(|i| {
            let (a, h) = (i32::from(t.accent[i]), i32::from(t.accent_hover[i]));
            (a + 2 * (h - a)).clamp(0, 255) as u8
        })
    } else if hot {
        t.accent_hover
    } else {
        t.accent
    };
    v.rounded(r.x, r.y, r.w, r.h, 4, fill);
    v.text_in_box(
        (r.x + 8, r.y, r.w.saturating_sub(16), r.h),
        label,
        TextStyle::Regular14,
        t.on_accent,
    );
}
