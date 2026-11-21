//! The Windows 11 compact context menu painter, shared by File Explorer and
//! the Settings context-menu customizer's live preview, so the preview is
//! the real menu: rounded panel, icon row (optionally labelled), separators,
//! hover plates, shortcuts, submenus and the user's `Look`. Included as a
//! module by each binary (and the host tests that compile them), so it paints
//! with that crate's `ui`.
#![allow(dead_code)]
use crate::icons::Icon;
use crate::ui;
use alloc::vec::Vec;
use door_graphics_text::TextStyle;
use door_workspace::context_menu::{self as cm, Action, Entry, Glyph, MenuState, Panel, Style};
use door_workspace::explorer_layout::Bounds;

/// Newton square root (no_std has no float sqrt without libm).
fn sqrt(v: f32) -> f32 {
    if v <= 0.0 {
        return 0.0;
    }
    let mut x = if v > 1.0 { v } else { 1.0 };
    for _ in 0..16 {
        x = 0.5 * (x + v / x);
    }
    x
}
/// A 12 px chevron stroked at the device density (1 logical px wide, 4x8
/// ink like the reference), pointing right or, while its flyout is open, down.
pub fn chevron_mask(scale: u32, down: bool) -> Vec<u8> {
    let points: [(f32, f32); 3] = if down {
        [(2.5, 4.5), (6.0, 8.0), (9.5, 4.5)]
    } else {
        [(4.5, 2.5), (8.0, 6.0), (4.5, 9.5)]
    };
    stroke_mask(12, scale, &points, 0.55)
}
/// Windows 10's submenu arrow: a thin chevron, 7x12 ink in a 14 px box.
pub fn win10_chevron_mask(scale: u32) -> Vec<u8> {
    stroke_mask(14, scale, &[(4.5, 1.5), (10.5, 7.0), (4.5, 12.5)], 0.5)
}
/// A polyline `points` (logical px) stroked `2 * half` wide in a square
/// `side` px box at the device density.
fn stroke_mask(side: usize, scale: u32, points: &[(f32, f32)], half: f32) -> Vec<u8> {
    let size = side * scale as usize;
    let s = scale as f32;
    let mut mask = alloc::vec![0u8; size * size];
    let half = half * s;
    for py in 0..size {
        for px in 0..size {
            let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
            let mut d = f32::MAX;
            for pair in points.windows(2) {
                let (ax, ay) = (pair[0].0 * s, pair[0].1 * s);
                let (bx, by) = (pair[1].0 * s, pair[1].1 * s);
                let (dx, dy) = (bx - ax, by - ay);
                let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
                let (cx, cy) = (ax + t * dx - x, ay + t * dy - y);
                let dist = sqrt(cx * cx + cy * cy);
                if dist < d {
                    d = dist;
                }
            }
            let coverage = (half + 0.5 - d).clamp(0.0, 1.0);
            mask[py * size + px] = (coverage * 255.0 + 0.5) as u8;
        }
    }
    mask
}

/// Theme colors of the 25H2 menu surface (opaque fallback of the acrylic).
pub struct MenuInk {
    pub panel: [u8; 4],
    pub border: [u8; 4],
    pub hover: [u8; 4],
    pub pressed: [u8; 4],
    pub text: [u8; 4],
    pub secondary: [u8; 4],
    pub disabled: [u8; 4],
    pub rule: [u8; 4],
}
/// Menu colours. Fills are painted literally (`rounded_asset`), so a
/// contrast theme takes its system colours directly: Window panel,
/// WindowText border and rules, Highlight for lit rows. Text keeps its light
/// roles, which the view maps (WindowText, GrayText; HighlightText on lit
/// rows, see `draw_panel`).
pub fn ink(palette: door_workspace::palette::Palette) -> MenuInk {
    if let Some(s) = palette.system() {
        let rgb = |c: [u8; 3]| [c[2], c[1], c[0], 255];
        return MenuInk {
            panel: rgb(s.window),
            border: rgb(s.window_text),
            hover: rgb(s.highlight),
            pressed: rgb(s.highlight),
            text: [26, 26, 26, 255],
            secondary: [96, 96, 96, 255],
            disabled: [160, 160, 160, 255],
            rule: rgb(s.window_text),
        };
    }
    if palette.dark {
        MenuInk {
            panel: [44, 44, 44, 255],
            border: [60, 60, 60, 255],
            hover: [58, 58, 58, 255],
            pressed: [52, 52, 52, 255],
            text: [255; 4],
            secondary: [206, 206, 206, 255],
            disabled: [120, 120, 120, 255],
            rule: [64, 64, 64, 255],
        }
    } else {
        MenuInk {
            panel: [250, 250, 250, 255],
            border: [222, 222, 222, 255],
            hover: [239, 239, 239, 255],
            pressed: [243, 243, 243, 255],
            text: [26, 26, 26, 255],
            secondary: [96, 96, 96, 255],
            disabled: [160, 160, 160, 255],
            rule: [232, 232, 232, 255],
        }
    }
}
/// Windows 10 menu colours (`Look::win10`), sampled from 100% captures of
/// the light desktop menu and the dark menu: #A0A0A0 border, #F2F2F2 /
/// #2B2B2B panel, #D9D9D9 / #414141 hot plate, #919191 / #808080 rules,
/// black / white text and shortcuts, #6D6D6D disabled text. Contrast
/// themes keep their system colours.
pub fn win10_ink(palette: door_workspace::palette::Palette) -> MenuInk {
    if palette.system().is_some() {
        return ink(palette);
    }
    if palette.dark {
        MenuInk {
            panel: [43, 43, 43, 255],
            border: [160, 160, 160, 255],
            hover: [65, 65, 65, 255],
            pressed: [65, 65, 65, 255],
            text: [255; 4],
            secondary: [255; 4],
            disabled: [109, 109, 109, 255],
            rule: [128, 128, 128, 255],
        }
    } else {
        MenuInk {
            panel: [242, 242, 242, 255],
            border: [160, 160, 160, 255],
            hover: [217, 217, 217, 255],
            pressed: [217, 217, 217, 255],
            text: [0, 0, 0, 255],
            secondary: [0, 0, 0, 255],
            disabled: [109, 109, 109, 255],
            rule: [145, 145, 145, 255],
        }
    }
}
pub fn action_icon(action: Action, label: &str) -> Option<Icon> {
    Some(match action {
        Action::Cut => Icon::Cut,
        Action::Copy => Icon::Copy,
        Action::Paste => Icon::Paste,
        Action::Rename | Action::RenameTag(_) => Icon::Rename,
        Action::Share => Icon::Share,
        Action::Delete | Action::DeleteTag(_) => Icon::Delete,
        Action::Open => Icon::Open,
        Action::OpenNewTab => Icon::NewTab,
        Action::OpenNewWindow => Icon::NewWindow,
        Action::PinQuickAccess => Icon::Pin,
        Action::Favorite => Icon::StarOutline,
        Action::RemoveRecent => Icon::Close,
        Action::OpenLocation => Icon::Open,
        Action::RunAsAdministrator => Icon::Shield,
        Action::Install => Icon::Open,
        Action::AddTag => Icon::Tag,
        Action::CopyPath => Icon::Link,
        Action::Compress => Icon::Zip,
        Action::Properties => Icon::Properties,
        Action::ShowMore => Icon::More,
        Action::OpenTerminal => Icon::Terminal,
        Action::Refresh => Icon::Refresh,
        Action::NewFolder => Icon::NewFolder,
        Action::NewTextDocument => Icon::GlyphDocument,
        Action::Sort(_) | Action::Ascending | Action::Descending => return None,
        Action::RemoveTag(_) => Icon::Close,
        Action::FilterTag(_) => Icon::Tag,
        Action::Submenu if label == "Sort by" => Icon::Sort,
        Action::Submenu if label == "New" => Icon::AddCircle,
        Action::Submenu if label == "Manage file" => Icon::GlyphDocument,
        Action::Submenu if label == "Open with" => Icon::Open,
        Action::Customize => Icon::Settings,
        Action::Restore | Action::RestoreAll => Icon::Back,
        Action::EmptyRecycleBin => Icon::RecycleBin,
        _ => return None,
    })
}
/// The 26H2 command icon of an action (`command_pixels` key), or None where
/// Windows draws no icon or a caller-specific one.
pub fn command_key(action: Action, label: &str) -> Option<&'static str> {
    Some(match action {
        Action::Cut => "cut",
        Action::Copy => "copy",
        Action::Paste => "paste",
        Action::Rename | Action::RenameTag(_) => "rename",
        Action::Share => "share",
        Action::Delete | Action::DeleteTag(_) => "delete",
        Action::Open => "open-folder",
        Action::OpenNewTab => "open-new-tab",
        Action::OpenNewWindow => "open-new-window",
        Action::OpenLocation => "open-location",
        Action::PinQuickAccess if label.starts_with("Unpin") => "unpin",
        Action::PinQuickAccess => "pin",
        Action::Favorite => "favorite",
        Action::CopyPath => "copy-path",
        Action::Compress => "zip",
        Action::ExtractAll => "extract",
        Action::Convert => "convert",
        Action::Properties => "properties",
        Action::ShowMore => "show-more",
        Action::Refresh => "refresh",
        Action::NewFolder => "new-folder",
        Action::Customize => "customize",
        Action::Restore => "restore",
        Action::RestoreAll => "restore-all",
        Action::EmptyRecycleBin => "empty-bin",
        Action::SelectAll => "select-all",
        Action::SelectNone => "select-none",
        Action::InvertSelection => "invert-selection",
        Action::RemoveRecent | Action::RemoveTag(_) => "remove",
        Action::Submenu => match label {
            "View" => "view",
            "Sort by" => "sort",
            "New" => "new",
            "Manage file" => "more-actions",
            "Open with" => "open-with",
            _ => return None,
        },
        _ => return None,
    })
}
/// 26H2 draws a few command glyphs in the accent colour (the Add to
/// Favorites star) instead of the text colour.
pub fn accent_ink(action: Action, dark: bool) -> Option<[u8; 4]> {
    match action {
        Action::Favorite => Some(if dark {
            [255, 194, 76, 255]
        } else {
            [192, 103, 0, 255]
        }),
        _ => None,
    }
}
pub fn view_icon(index: u8) -> Icon {
    if index < 4 {
        Icon::LargeIcons
    } else {
        Icon::Details
    }
}

/// Paints a caller-specific glyph at (x, y) in a colour; false skips it.
pub type GlyphPainter = dyn Fn(&mut ui::View<'_>, Glyph, (i32, i32), [u8; 4]) -> bool;
/// One menu panel. `glyph` paints the glyphs that need the caller's data
/// (known folders, tags, application icons); it returns false to skip.
#[allow(clippy::too_many_arguments)]
pub fn draw_panel(
    v: &mut ui::View<'_>,
    m: &MenuState,
    p: &Panel,
    entries: &[Entry],
    focus: Option<usize>,
    focus_icon: Option<usize>,
    open_sub: Option<usize>,
    glyph: &GlyphPainter,
) {
    // The classic-menu setting paints Windows 10's flat menu.
    let flat = cm::win10(m.menu.style, m.menu.look);
    let ink = if flat {
        win10_ink(v.palette)
    } else {
        ink(v.palette)
    };
    let contrast = v.palette.contrast.is_some();
    // The genuine 26H2 two-tone icon for an enabled command (contrast
    // themes and disabled commands keep the single-colour glyph).
    let (dark, density) = (v.palette.dark, v.scale.clamp(1, 3));
    let colour = move |e: &Entry| -> Option<&'static [u8]> {
        if contrast || !e.enabled {
            return None;
        }
        command_key(e.action, &e.label).and_then(|k| command_pixels(k, dark, density))
    };
    let b = p.bounds;
    let style = m.menu.style;
    if flat {
        // Windows' menu drop shadow: 4 px fading strips on the right and
        // bottom, offset 4 px from the top and left edges.
        for (i, alpha) in [72u8, 46, 22, 8].into_iter().enumerate() {
            let i = i as i32;
            v.rounded_asset(
                b.x + b.width as i32 + i,
                b.y + 4,
                1,
                b.height.saturating_sub(4) + i as u32,
                0,
                [0, 0, 0, alpha],
            );
            v.rounded_asset(
                b.x + 4,
                b.y + b.height as i32 + i,
                b.width.saturating_sub(4) + i as u32,
                1,
                0,
                [0, 0, 0, alpha],
            );
        }
        v.rounded_asset(b.x, b.y, b.width, b.height, 0, ink.border);
        v.rounded_asset(
            b.x + 1,
            b.y + 1,
            b.width.saturating_sub(2),
            b.height.saturating_sub(2),
            0,
            ink.panel,
        );
    } else {
        v.elevation(
            b.x,
            b.y + 4,
            b.width,
            b.height,
            if v.palette.dark { 110 } else { 60 },
        );
        v.rounded_asset(b.x, b.y, b.width, b.height, cm::metrics::RADIUS, ink.border);
        v.rounded_asset(
            b.x + 1,
            b.y + 1,
            b.width.saturating_sub(2),
            b.height.saturating_sub(2),
            cm::metrics::RADIUS - 1,
            ink.panel,
        );
    }
    let (hover, press) = (v.hover, v.pressed);
    let pointer = move |r: Bounds| hover.is_some_and(|(x, y)| r.contains(x, y));
    let down = move |r: Bounds| press.is_some_and(|(x, y)| r.contains(x, y));
    for (i, r) in p.icons.iter().enumerate() {
        let Some(entry) = m.menu.icons.get(i) else {
            continue;
        };
        if entry.enabled && (pointer(*r) || focus_icon == Some(i)) {
            v.rounded_asset(
                r.x,
                r.y,
                r.width,
                r.height,
                4,
                if down(*r) { ink.pressed } else { ink.hover },
            );
        }
        if let Some(icon) = action_icon(entry.action, &entry.label) {
            let lit = contrast && entry.enabled && (pointer(*r) || focus_icon == Some(i));
            let color = if lit {
                [255; 4]
            } else if entry.enabled {
                ink.text
            } else {
                ink.disabled
            };
            // A labelled row (`Look::labels`) stacks the icon over a caption.
            let labelled = m.menu.look.labels;
            let (ix, iy) = if labelled {
                (r.x + (r.width as i32 - 16) / 2, r.y + 8)
            } else {
                (r.x + 8, r.y + 8)
            };
            match colour(entry) {
                Some(px) => v.color_pixels(ix, iy, 16, px),
                None => v.mask_pixels(ix, iy, 16, icon.mask_scaled(16, v.scale), color, None),
            }
            if labelled {
                let w = r.width.saturating_sub(4);
                let text = door_workspace::explorer_path::ellipsize(&entry.label, w, |s| {
                    v.measure_style(s, TextStyle::Regular12)
                });
                v.text_in_box(
                    (r.x + 2, r.y + 28, w, 18),
                    &text,
                    TextStyle::Regular12,
                    if entry.enabled {
                        ink.text
                    } else {
                        ink.disabled
                    },
                );
            }
        }
    }
    if let Some(y) = p.icon_rule {
        v.rounded_asset(b.x + 1, y, b.width.saturating_sub(2), 1, 0, ink.rule);
    }
    for y in &p.separators {
        // Windows 10 rules stop 11 px short of each border.
        let inset = if flat { 11 } else { 1 };
        v.rounded_asset(
            b.x + inset,
            *y,
            b.width.saturating_sub(2 * inset as u32),
            1,
            0,
            ink.rule,
        );
    }
    let bullets = style != Style::Flyout && entries.iter().any(|e| e.checked);
    let classic = style == Style::Classic;
    let text_style = if classic {
        TextStyle::Regular12
    } else {
        TextStyle::Regular14
    };
    for (i, entry) in entries.iter().enumerate() {
        let Some(r) = p.rows.get(i).copied().flatten() else {
            continue;
        };
        // Windows 10 hot-tracks disabled rows under the pointer too.
        let lit = (entry.enabled && (focus == Some(i) || open_sub == Some(i)))
            || ((entry.enabled || flat) && pointer(r));
        if lit {
            v.rounded_asset(
                r.x,
                r.y,
                r.width,
                r.height,
                if flat { 0 } else { 4 },
                if down(r) { ink.pressed } else { ink.hover },
            );
        }
        // Lit rows in a contrast theme carry HighlightText ink.
        let color = if lit && contrast {
            [255; 4]
        } else if entry.enabled {
            ink.text
        } else {
            ink.disabled
        };
        let mut x = r.x + if classic { 8 } else { 10 };
        if bullets {
            if entry.checked {
                let dot = if contrast {
                    v.palette.foreground(color)
                } else {
                    color
                };
                // Windows 10 draws the bullet in the icon column.
                let dx = if flat { 5 } else { 3 };
                v.rounded_asset(x + dx, r.y + r.height as i32 / 2 - 3, 6, 6, 3, dot);
            }
            if !flat {
                x += 16;
            }
        }
        let icon_y = r.y + (r.height as i32 - 16) / 2;
        // `Look::plain`: commands without icons (their column closes up).
        let plain = style == Style::Modern && m.menu.look.plain;
        match if plain { Glyph::None } else { entry.glyph } {
            Glyph::None => (),
            Glyph::Action => {
                if let Some(px) = colour(entry) {
                    v.color_pixels(x, icon_y, 16, px);
                } else if let Some(icon) = action_icon(entry.action, &entry.label) {
                    let ink = if entry.enabled && !contrast {
                        accent_ink(entry.action, v.palette.dark).unwrap_or(color)
                    } else {
                        color
                    };
                    v.mask_pixels(x, icon_y, 16, icon.mask_scaled(16, v.scale), ink, None);
                }
            }
            Glyph::ViewMode(i) => match command_pixels("view", v.palette.dark, v.scale.clamp(1, 3))
                .filter(|_| !contrast && entry.enabled)
            {
                Some(px) => v.color_pixels(x, icon_y, 16, px),
                None => v.mask_pixels(
                    x,
                    icon_y,
                    16,
                    view_icon(i).mask_scaled(16, v.scale),
                    color,
                    None,
                ),
            },
            Glyph::Folder => v.compact_icon(x, icon_y, Icon::Folder, color),
            Glyph::Drive => v.compact_icon(x, icon_y, Icon::Drive, color),
            Glyph::ThisPc => v.compact_icon(x, icon_y, Icon::Computer, color),
            Glyph::Network => v.compact_icon(x, icon_y, Icon::Network, color),
            Glyph::Home => v.compact_icon(x, icon_y, Icon::Home, color),
            other => {
                glyph(v, other, (x, icon_y), color);
            }
        }
        // Windows 10: text 36 px from the border, shortcuts end 20 px from
        // it, the arrow box 23 px.
        let text_x = if flat {
            r.x + 33
        } else if classic {
            r.x + 36
        } else if plain {
            x + 4
        } else {
            x + 30
        };
        let right = r.x + r.width as i32
            - match (flat, entry.children.is_empty()) {
                (true, true) => 17,
                (true, false) => 24,
                (false, true) => 12,
                (false, false) => 32,
            };
        let shortcut_w =
            if entry.shortcut.is_empty() || (style == Style::Modern && m.menu.look.no_shortcuts) {
                0
            } else {
                v.measure_style(entry.shortcut, text_style)
            };
        let label_w = (right - text_x - shortcut_w as i32 - if shortcut_w > 0 { 16 } else { 0 })
            .max(0) as u32;
        let label_style = if style == Style::Flyout && entry.checked {
            TextStyle::Semibold14
        } else {
            text_style
        };
        let label = door_workspace::explorer_path::ellipsize(
            &ui::display_ascii(&entry.label),
            label_w,
            |s| v.measure_style(s, label_style),
        );
        v.text_clip_centered(
            text_x,
            (r.y, r.height),
            label_w + 2,
            &label,
            label_style,
            color,
        );
        if shortcut_w > 0 {
            v.text_clip_centered(
                right - shortcut_w as i32,
                (r.y, r.height),
                shortcut_w + 2,
                entry.shortcut,
                text_style,
                if lit && contrast {
                    [255; 4]
                } else if entry.enabled {
                    ink.secondary
                } else {
                    ink.disabled
                },
            );
        }
        if !entry.children.is_empty() && flat {
            let mask = win10_chevron_mask(v.scale);
            v.mask_pixels(
                r.x + r.width as i32 - 20,
                r.y + (r.height as i32 - 14) / 2,
                14,
                &mask,
                color,
                None,
            );
        } else if !entry.children.is_empty() {
            let mask = chevron_mask(v.scale, false);
            v.mask_pixels(
                r.x + r.width as i32 - 24,
                r.y + (r.height as i32 - 12) / 2,
                12,
                &mask,
                color,
                None,
            );
        }
    }
    for (more, y) in [
        (p.more_above, b.y + 1),
        (p.more_below, b.y + b.height as i32 - 13),
    ] {
        if more {
            v.rounded_asset(b.x + 1, y, b.width.saturating_sub(2), 12, 0, ink.panel);
            let mask = chevron_mask(v.scale, true);
            let flipped: Vec<u8> = if y == b.y + 1 {
                let side = 12 * v.scale as usize;
                mask.chunks_exact(side).rev().flatten().copied().collect()
            } else {
                mask
            };
            v.mask_pixels(
                b.x + b.width as i32 / 2 - 6,
                y,
                12,
                &flipped,
                ink.secondary,
                None,
            );
        }
    }
}

/// The 26H2 two-tone command icon `key` (`assets/icons/explorer-commands`)
/// for a theme at an integer density, premultiplied BGRA (16 px logical).
pub fn command_pixels(key: &str, dark: bool, scale: u32) -> Option<&'static [u8]> {
    macro_rules! frames {
        ($k:literal) => {
            match (dark, scale) {
                (false, 1) => door_workspace::asset!(concat!(
                    "../../../assets/icons/explorer-commands/",
                    $k,
                    ".light.16.bgra"
                )),
                (false, 2) => door_workspace::asset!(concat!(
                    "../../../assets/icons/explorer-commands/",
                    $k,
                    ".light.32.bgra"
                )),
                (false, _) => door_workspace::asset!(concat!(
                    "../../../assets/icons/explorer-commands/",
                    $k,
                    ".light.48.bgra"
                )),
                (true, 1) => door_workspace::asset!(concat!(
                    "../../../assets/icons/explorer-commands/",
                    $k,
                    ".dark.16.bgra"
                )),
                (true, 2) => door_workspace::asset!(concat!(
                    "../../../assets/icons/explorer-commands/",
                    $k,
                    ".dark.32.bgra"
                )),
                (true, _) => door_workspace::asset!(concat!(
                    "../../../assets/icons/explorer-commands/",
                    $k,
                    ".dark.48.bgra"
                )),
            }
        };
    }
    Some(match key {
        "cut" => frames!("cut"),
        "copy" => frames!("copy"),
        "paste" => frames!("paste"),
        "rename" => frames!("rename"),
        "share" => frames!("share"),
        "delete" => frames!("delete"),
        "open-folder" => frames!("open-folder"),
        "open-new-tab" => frames!("open-new-tab"),
        "open-new-window" => frames!("open-new-window"),
        "open-with" => frames!("open-with"),
        "open-location" => frames!("open-location"),
        "pin" => frames!("pin"),
        "unpin" => frames!("unpin"),
        "favorite" => frames!("favorite"),
        "copy-path" => frames!("copy-path"),
        "zip" => frames!("zip"),
        "extract" => frames!("extract"),
        "properties" => frames!("properties"),
        "show-more" => frames!("show-more"),
        "refresh" => frames!("refresh"),
        "new" => frames!("new"),
        "new-folder" => frames!("new-folder"),
        "view" => frames!("view"),
        "sort" => frames!("sort"),
        "customize" => frames!("customize"),
        "send-to" => frames!("send-to"),
        "wallpaper" => frames!("wallpaper"),
        "empty-bin" => frames!("empty-bin"),
        "restore-all" => frames!("restore-all"),
        "restore" => frames!("restore"),
        "select-all" => frames!("select-all"),
        "select-none" => frames!("select-none"),
        "invert-selection" => frames!("invert-selection"),
        "more-actions" => frames!("more-actions"),
        "extensions" => frames!("extensions"),
        "remove" => frames!("remove"),
        "personalize" => frames!("personalize"),
        "display" => frames!("display"),
        "undo" => frames!("undo"),
        "redo" => frames!("redo"),
        _ => return None,
    })
}
