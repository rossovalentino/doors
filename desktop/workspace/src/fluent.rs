//! Reference-matched app chrome and native Segoe Fluent control glyphs.
use crate::View;
use door_graphics_text::TextStyle;
/// Labels keep every script and emoji; only control characters are replaced.
fn display(text: &str) -> alloc::borrow::Cow<'_, str> {
    if !text
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        alloc::borrow::Cow::Borrowed(text)
    } else {
        alloc::borrow::Cow::Owned(crate::ui::display_ascii(text))
    }
}
pub const FG: [u8; 4] = [24, 24, 24, 255];
pub const MUTED: [u8; 4] = [112, 112, 112, 255];
pub const ACCENT: [u8; 4] = [204, 112, 0, 255];
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
    pub fn hit(self, x: i32, y: i32) -> bool {
        x >= self.x
            && y >= self.y
            && i64::from(x) < i64::from(self.x) + i64::from(self.w)
            && i64::from(y) < i64::from(self.y) + i64::from(self.h)
    }
    /// Horizontal padding removed from both sides (content box).
    pub fn inset(self, pad: u32) -> Self {
        let pad = pad.min(self.w / 2);
        Self::new(self.x + pad as i32, self.y, self.w - 2 * pad, self.h)
    }
}
#[derive(Clone, Copy)]
pub enum Glyph {
    Model,
    Cpu,
    Display,
    Bold,
    Italic,
    Strike,
    Link,
    Memory,
    Wifi,
    Performance,
    Apps,
    Settings,
    Power,
    Network,
    Drive,
    Folder,
    Document,
    Home,
    History,
    People,
    Image,
    Video,
    Audio,
    Archive,
    Edit,
    Convert,
    Batch,
    Camera,
    Record,
    Scan,
    Crop,
    Rotate,
    Adjust,
    Erase,
    Hand,
    Fit,
    ZoomIn,
    ZoomOut,
    More,
    Down,
    Info,
    Save,
    Back,
    Forward,
    Add,
    Close,
    Search,
    Filter,
    Refresh,
    Processes,
    Check,
    Shield,
    /// "Aa": Notes' Formatted / Plain text status toggle.
    Font,
}
impl Glyph {
    pub fn mask(self, scale: u32) -> &'static [u8] {
        macro_rules! mask {
            ($name:literal) => {
                match scale {
                    3 => &door_workspace::asset!(concat!(
                        "../../../assets/icons/windows-shell/",
                        $name,
                        ".60.mask"
                    ))[..],
                    2 => &door_workspace::asset!(concat!(
                        "../../../assets/icons/windows-shell/",
                        $name,
                        ".40.mask"
                    ))[..],
                    _ => &door_workspace::asset!(concat!(
                        "../../../assets/icons/windows-shell/",
                        $name,
                        ".20.mask"
                    ))[..],
                }
            };
        }
        match self {
            Self::Model => match scale {
                3 => door_workspace::asset!(
                    "../../../assets/icons/fluent/ic_fluent_cube_20_regular.60.a8"
                ),
                2 => door_workspace::asset!(
                    "../../../assets/icons/fluent/ic_fluent_cube_20_regular.40.a8"
                ),
                _ => door_workspace::asset!(
                    "../../../assets/icons/fluent/ic_fluent_cube_20_regular.a8"
                ),
            },
            Self::Bold => mask!("app-bold"),
            Self::Italic => mask!("app-italic"),
            Self::Strike => mask!("app-strike"),
            Self::Link => mask!("setting-link"),
            Self::Display => mask!("setting-display"),
            Self::Cpu => mask!("app-cpu"),
            Self::Memory => mask!("app-memory"),
            Self::Wifi => mask!("app-wifi"),
            Self::Performance => mask!("app-performance"),
            Self::Apps => mask!("app-apps"),
            Self::Settings => mask!("app-settings"),
            Self::Power => mask!("app-power"),
            Self::Network => mask!("app-network"),
            Self::Drive => mask!("app-drive"),
            Self::Folder => mask!("app-folder"),
            Self::Document => mask!("app-document"),
            Self::Home => mask!("app-home"),
            Self::History => mask!("app-history"),
            Self::People => mask!("app-people"),
            Self::Image => mask!("app-image"),
            Self::Video => mask!("app-video"),
            Self::Audio => mask!("app-audio"),
            Self::Archive => mask!("app-archive"),
            Self::Edit => mask!("app-edit"),
            Self::Convert => mask!("app-convert"),
            Self::Batch => mask!("app-batch"),
            Self::Camera => mask!("app-camera"),
            Self::Record => mask!("app-record"),
            Self::Scan => mask!("app-scan"),
            Self::Crop => mask!("app-crop"),
            Self::Rotate => mask!("app-rotate"),
            Self::Adjust => mask!("app-adjust"),
            Self::Erase => mask!("app-erase"),
            Self::Hand => mask!("app-hand"),
            Self::Fit => mask!("app-fit"),
            Self::ZoomIn => mask!("app-zoomin"),
            Self::ZoomOut => mask!("app-zoomout"),
            Self::More => mask!("app-more"),
            Self::Down => mask!("app-down"),
            Self::Info => mask!("app-info"),
            Self::Save => mask!("app-save"),
            Self::Back => mask!("app-back"),
            Self::Forward => mask!("app-forward"),
            Self::Add => mask!("app-add"),
            Self::Close => mask!("app-close"),
            Self::Search => mask!("app-search"),
            Self::Filter => mask!("app-filter"),
            Self::Refresh => mask!("app-refresh"),
            Self::Processes => mask!("app-processes"),
            Self::Check => mask!("app-check"),
            Self::Shield => mask!("app-shield"),
            Self::Font => mask!("setting-font"),
        }
    }
}
pub fn glyph(v: &mut View<'_>, x: i32, y: i32, g: Glyph, color: [u8; 4]) {
    v.mask_pixels(x, y, 20, g.mask(v.scale), color, None);
}
pub fn base(v: &mut View<'_>, rail: u32) {
    let dark = v.palette.dark;
    v.rect(
        rail as i32,
        48,
        v.width.saturating_sub(rail),
        v.height.saturating_sub(48),
        if dark {
            [32, 30, 29, 255]
        } else {
            [252, 248, 243, 255]
        },
    );
    v.material_rect(
        0,
        0,
        rail,
        v.height,
        if dark {
            [38, 34, 31, 255]
        } else {
            [247, 239, 228, 255]
        },
        220,
    );
    v.material_rect(
        rail as i32,
        0,
        v.width.saturating_sub(rail),
        48,
        if dark {
            [45, 39, 35, 255]
        } else {
            [248, 235, 222, 255]
        },
        220,
    );
}
pub fn card(v: &mut View<'_>, r: Rect) {
    v.rounded(
        r.x,
        r.y,
        r.w,
        r.h,
        8,
        if v.palette.dark {
            [56, 52, 49, 255]
        } else {
            [243, 239, 233, 255]
        },
    );
    if r.w > 2 && r.h > 2 {
        v.rounded(
            r.x + 1,
            r.y + 1,
            r.w - 2,
            r.h - 2,
            7,
            if v.palette.dark {
                [43, 40, 37, 255]
            } else {
                [255, 253, 250, 255]
            },
        );
    }
}
pub fn button(
    v: &mut View<'_>,
    r: Rect,
    label: &str,
    g: Option<Glyph>,
    accent: bool,
    enabled: bool,
) {
    let hot = enabled && v.hover.is_some_and(|(x, y)| r.hit(x, y));
    let pressed = hot && v.pressed.is_some_and(|(x, y)| r.hit(x, y));
    let fill = if accent && enabled {
        if pressed {
            [165, 86, 0, 255]
        } else if hot {
            [190, 101, 0, 255]
        } else {
            ACCENT
        }
    } else if pressed {
        [219, 219, 219, 255]
    } else if hot {
        [236, 236, 236, 255]
    } else {
        [255; 4]
    };
    v.rounded(r.x, r.y, r.w, r.h, 5, fill);
    let color = if !enabled {
        [155, 155, 155, 255]
    } else if accent {
        [255; 4]
    } else {
        FG
    };
    let safe = display(label);
    let middle = r.y + (r.h as i32 - 20) / 2;
    match g {
        // ComboBox: value left-aligned, chevron at the right edge.
        Some(Glyph::Down) => {
            glyph(v, r.x + r.w as i32 - 28, middle, Glyph::Down, color);
            v.text_clip_centered(
                r.x + 12,
                (r.y, r.h),
                r.w.saturating_sub(48),
                &safe,
                TextStyle::Regular14,
                color,
            );
        }
        // Icon and label are one content group, centred as a WinUI button's.
        Some(g) => {
            let group = 20 + 8 + v.measure(&safe);
            let start = if group + 24 <= r.w {
                r.x + (r.w - group) as i32 / 2
            } else {
                r.x + 12
            };
            glyph(v, start, middle, g, color);
            let text = start + 28;
            v.text_clip_centered(
                text,
                (r.y, r.h),
                (r.x + r.w as i32 - 8 - text).max(0) as u32,
                &safe,
                TextStyle::Regular14,
                color,
            );
        }
        None => boxed_label(v, r.inset(8), &safe, TextStyle::Regular14, color),
    }
}
pub fn tool(v: &mut View<'_>, r: Rect, g: Glyph, enabled: bool, selected: bool) {
    if selected || enabled && v.hover.is_some_and(|(x, y)| r.hit(x, y)) {
        v.rounded(
            r.x,
            r.y,
            r.w,
            r.h,
            4,
            if enabled && v.pressed.is_some_and(|(x, y)| r.hit(x, y)) {
                [219, 219, 219, 255]
            } else if selected {
                [250, 233, 213, 255]
            } else {
                [233, 233, 233, 255]
            },
        );
    }
    glyph(
        v,
        r.x + (r.w as i32 - 20) / 2,
        r.y + (r.h as i32 - 20) / 2,
        g,
        if enabled { FG } else { [155, 155, 155, 255] },
    );
}
pub fn nav(v: &mut View<'_>, r: Rect, label: &str, g: Glyph, selected: bool, enabled: bool) {
    nav_colored(v, r, label, g, selected, enabled, FG);
}
pub fn nav_colored(
    v: &mut View<'_>,
    r: Rect,
    label: &str,
    g: Glyph,
    selected: bool,
    enabled: bool,
    icon_color: [u8; 4],
) {
    nav_spaced(v, r, label, g, selected, enabled, icon_color, (16, 52));
}
#[allow(clippy::too_many_arguments)]
pub fn nav_spaced(
    v: &mut View<'_>,
    r: Rect,
    label: &str,
    g: Glyph,
    selected: bool,
    enabled: bool,
    icon_color: [u8; 4],
    spacing: (i32, i32),
) {
    let state = RowState {
        selected,
        enabled,
        hovered: v.hover.is_some_and(|(x, y)| r.hit(x, y)),
        pressed: v.pressed.is_some_and(|(x, y)| r.hit(x, y)),
    };
    nav_row(v, r, label, g, state, icon_color, spacing);
}
/// Interaction state resolved by the caller's own hit test, so a row clipped by
/// a scrolling pane never shows hover for a pointer outside its visible part.
#[derive(Clone, Copy, Default)]
pub struct RowState {
    pub selected: bool,
    pub enabled: bool,
    pub hovered: bool,
    pub pressed: bool,
}
fn row_fill(v: &mut View<'_>, r: Rect, s: RowState) {
    let fill = if s.enabled && s.pressed && s.hovered {
        [240, 220, 197, 255]
    } else if s.selected {
        [250, 233, 213, 255]
    } else if s.enabled && s.hovered {
        [235, 235, 235, 255]
    } else {
        return;
    };
    v.rounded(r.x, r.y, r.w, r.h, 5, fill);
}
pub fn nav_row(
    v: &mut View<'_>,
    r: Rect,
    label: &str,
    g: Glyph,
    s: RowState,
    icon_color: [u8; 4],
    spacing: (i32, i32),
) {
    let (selected, enabled) = (s.selected, s.enabled);
    row_fill(v, r, s);
    if selected {
        v.rounded(r.x, r.y + 9, 3, r.h.saturating_sub(18), 1, ACCENT);
    }
    glyph(
        v,
        r.x + spacing.0,
        r.y + (r.h as i32 - 20) / 2,
        g,
        if selected {
            ACCENT
        } else if enabled {
            icon_color
        } else {
            MUTED
        },
    );
    v.text_clip_centered(
        r.x + spacing.1,
        (r.y, r.h),
        r.w.saturating_sub(spacing.1 as u32 + 10),
        &display(label),
        TextStyle::Regular16,
        if enabled { FG } else { MUTED },
    );
}
/// A collapsible group header: muted label with a disclosure chevron at the
/// row's right edge. `selected` marks a hidden selected child (collapsed group).
pub fn nav_header(v: &mut View<'_>, r: Rect, label: &str, expanded: bool, s: RowState) {
    row_fill(
        v,
        r,
        RowState {
            selected: false,
            ..s
        },
    );
    if s.selected {
        v.rounded(r.x, r.y + 9, 3, r.h.saturating_sub(18), 1, ACCENT);
    }
    v.text_clip_centered(
        r.x + 12,
        (r.y, r.h),
        r.w.saturating_sub(44),
        &display(label),
        TextStyle::Semibold14,
        MUTED,
    );
    v.tiny_icon(
        r.x + r.w as i32 - 26,
        r.y + (r.h as i32 - 12) / 2,
        if expanded {
            crate::icons::Icon::Chevron
        } else {
            crate::icons::Icon::ChevronRight
        },
        MUTED,
    );
}
/// Compact-pane group boundary: a hairline centered in its separator row.
pub fn nav_separator(v: &mut View<'_>, r: Rect) {
    v.rect(
        r.x + 8,
        r.y + r.h as i32 / 2,
        r.w.saturating_sub(16),
        1,
        [222, 222, 222, 255],
    );
}
/// Overlay scroll thumb used by navigation panes (Settings and Preview).
pub fn pane_thumb(v: &mut View<'_>, x: i32, top: i32, height: i32, active: bool) {
    v.rounded(
        x,
        top,
        if active { 4 } else { 3 },
        height.max(0) as u32,
        2,
        if active {
            [133, 133, 133, 255]
        } else {
            [155, 155, 155, 255]
        },
    );
}
pub fn label(v: &mut View<'_>, r: Rect, text: &str, style: TextStyle, color: [u8; 4]) {
    v.text_clip_centered(r.x, (r.y, r.h), r.w, &display(text), style, color);
}
/// The shared boxed label: centred in `r` on its painted ink horizontally
/// and on cap height vertically (`View::text_in_box`).
pub fn boxed_label(v: &mut View<'_>, r: Rect, text: &str, style: TextStyle, color: [u8; 4]) {
    v.text_in_box((r.x, r.y, r.w, r.h), &display(text), style, color);
}
pub fn centered_label14(v: &mut View<'_>, r: Rect, text: &str, color: [u8; 4]) {
    boxed_label(v, r, text, TextStyle::Regular14, color);
}
/// Word wrap informational copy using the same measured 14px face as its paint.
pub fn paragraph(v: &mut View<'_>, r: Rect, text: &str, color: [u8; 4]) {
    let safe = display(text);
    let mut line = alloc::string::String::new();
    let mut y = r.y;
    for word in safe.split_whitespace() {
        let candidate = if line.is_empty() {
            word.into()
        } else {
            alloc::format!("{line} {word}")
        };
        if !line.is_empty() && v.measure(&candidate) > r.w {
            if y + 20 > r.y + r.h as i32 {
                return;
            }
            label(
                v,
                Rect::new(r.x, y, r.w, 20),
                &line,
                TextStyle::Regular14,
                color,
            );
            y += 20;
            line = word.into();
        } else {
            line = candidate;
        }
    }
    if y + 20 <= r.y + r.h as i32 {
        label(
            v,
            Rect::new(r.x, y, r.w, 20),
            &line,
            TextStyle::Regular14,
            color,
        );
    }
}
