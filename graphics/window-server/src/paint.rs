//! Shared trusted-shell drawing primitives: logical-pixel text, Segoe Fluent
//! glyph masks, rounded plates and the Windows 11 shell palette.
use door_graphics_core::{Canvas, Rect, Scale};
use door_graphics_text::{Clip, FontAtlas, Target, TextRun, TextStyle};
/// Premultiplied BGRA color with `alpha` coverage applied.
pub const fn alpha(color: [u8; 4], alpha: u8) -> [u8; 4] {
    let a = alpha as u32;
    [
        (color[0] as u32 * a / 255) as u8,
        (color[1] as u32 * a / 255) as u8,
        (color[2] as u32 * a / 255) as u8,
        (color[3] as u32 * a / 255) as u8,
    ]
}
/// Opaque gray / color from an sRGB triple (stored BGRA).
pub const fn rgb(r: u8, g: u8, b: u8) -> [u8; 4] {
    [b, g, r, 255]
}
pub const fn white(a: u8) -> [u8; 4] {
    [a, a, a, a]
}
pub const fn black(a: u8) -> [u8; 4] {
    [0, 0, 0, a]
}
/// The session's contrast theme as the shell last applied it
/// (`set_contrast`): 0 off, else 1 + the `ContrastTheme` value.
static CONTRAST: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
/// Apply (or clear) a contrast theme to every shell palette from now on.
pub fn set_contrast(theme: Option<door_service_protocol::desktop::ContrastTheme>) {
    CONTRAST.store(
        theme.map_or(0, |t| t as u32 + 1),
        core::sync::atomic::Ordering::Relaxed,
    );
}
/// Door's default accent colour (Windows 11 default blue), 0xRRGGBB.
pub const DEFAULT_ACCENT: u32 = 0x0078D4;
/// The system accent colour the shell paints with (`set_accent`), 0xRRGGBB.
static ACCENT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(DEFAULT_ACCENT);
/// Apply a resolved accent colour to every shell palette from now on.
pub fn set_accent(rgb: u32) {
    ACCENT.store(rgb & 0x00ff_ffff, core::sync::atomic::Ordering::Relaxed);
}
/// Settings > Personalization > Colors "Show accent colour on Start and
/// taskbar" as the shell last applied it (`set_start_tint`): the opaque
/// BGRA tint as a word, 0 when off.
static START_TINT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
/// Tint Start, the taskbar and the tray flyouts with `tint` (opaque BGRA)
/// in dark mode from now on; None paints them neutral again.
pub fn set_start_tint(tint: Option<[u8; 4]>) {
    START_TINT.store(
        tint.map_or(0, |t| u32::from_le_bytes([t[0], t[1], t[2], 255])),
        core::sync::atomic::Ordering::Relaxed,
    );
}
/// The accent tint Start, the taskbar and the tray flyouts paint with in
/// `dark` mode, if the user turned it on (Windows shows it only in dark
/// mode and never under a contrast theme).
pub fn start_tint(dark: bool) -> Option<[u8; 4]> {
    match START_TINT.load(core::sync::atomic::Ordering::Relaxed) {
        0 => None,
        _ if !dark || contrast().is_some() => None,
        word => Some(word.to_le_bytes()),
    }
}
/// The applied accent palette (Light3..Dark3).
pub fn accent_palette() -> door_design_tokens::accent::Palette {
    door_design_tokens::accent::palette(ACCENT.load(core::sync::atomic::Ordering::Relaxed))
}
/// Accessibility > Text size the shell atlas applies (`set_text_percent`,
/// with every atlas install). Shell layout that holds text grows by it;
/// icons, glyph buttons and the taskbar itself keep their size.
static TEXT_PERCENT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(100);
pub fn set_text_percent(percent: u32) {
    TEXT_PERCENT.store(
        percent.clamp(
            door_graphics_text::TEXT_PERCENT_MIN,
            door_graphics_text::TEXT_PERCENT_MAX,
        ),
        core::sync::atomic::Ordering::Relaxed,
    );
}
pub fn text_percent() -> u32 {
    TEXT_PERCENT.load(core::sync::atomic::Ordering::Relaxed)
}
/// A text-holding shell length (menu row, tile label band, flyout row)
/// grown by the growth of `style`'s line box at the applied text size.
pub fn text_grow(base: u32, style: TextStyle) -> u32 {
    door_graphics_text::text_grow(base, style, text_percent())
}
/// `text_grow` for signed coordinates.
pub fn text_grow_i(base: i32, style: TextStyle) -> i32 {
    base + text_grow(0, style) as i32
}
/// A logical text extent (a text column's width, a caret) at the applied
/// text size: text-holding widths scale with their text.
pub fn text_len(logical: u32) -> u32 {
    door_graphics_text::text_len(logical, text_percent())
}
/// The contrast theme shell surfaces paint with, if any.
pub fn contrast() -> Option<door_service_protocol::desktop::ContrastTheme> {
    match CONTRAST.load(core::sync::atomic::Ordering::Relaxed) {
        0 => None,
        v => Some(door_service_protocol::desktop::ContrastTheme::from_index(
            v - 1,
        )),
    }
}
/// Windows 11 shell palette for one theme (values sampled from the owner's
/// 25H2 references and the WinUI theme resources).
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub dark: bool,
    /// A contrast theme's system colours: solid, no translucent plates.
    pub contrast: bool,
    pub text: [u8; 4],
    pub text_secondary: [u8; 4],
    pub text_disabled: [u8; 4],
    /// Start/flyout acrylic tint (opaque) and its translucent overlays.
    pub surface: [u8; 4],
    pub surface_header: [u8; 4],
    pub surface_divider: [u8; 4],
    pub surface_border: [u8; 4],
    pub card: [u8; 4],
    pub field: [u8; 4],
    pub field_border: [u8; 4],
    pub subtle_hover: [u8; 4],
    pub subtle_pressed: [u8; 4],
    pub menu: [u8; 4],
    pub menu_border: [u8; 4],
    pub accent: [u8; 4],
    pub accent_text: [u8; 4],
    pub focus: [u8; 4],
    pub tile: [u8; 4],
    pub tile_border: [u8; 4],
}
impl Palette {
    /// The shell palette for `dark`, or the active contrast theme's.
    pub fn new(dark: bool) -> Self {
        match contrast() {
            Some(theme) => Self::contrast_theme(theme),
            None => Self::themed(dark).with_accent(&accent_palette()),
        }
    }
    /// Accent fills follow the system accent: Light2 in dark, Dark1 in
    /// light (WinUI AccentFillColorDefault); text on them keeps the WinUI
    /// TextOnAccentFillColorPrimary (black in dark, white in light).
    pub fn with_accent(mut self, palette: &door_design_tokens::accent::Palette) -> Self {
        if !self.contrast {
            self.accent = palette.fill(self.dark);
        }
        self
    }
    /// A contrast theme (Windows forced colours): Window surfaces, WindowText
    /// text and strokes, Highlight accents with HighlightText, GrayText for
    /// disabled text. Hover and pressed plates stay solid Window.
    pub fn contrast_theme(theme: door_service_protocol::desktop::ContrastTheme) -> Self {
        let s = theme.colors();
        let c = |v: [u8; 3]| rgb(v[0], v[1], v[2]);
        Self {
            dark: theme.dark(),
            contrast: true,
            text: c(s.window_text),
            text_secondary: c(s.window_text),
            text_disabled: c(s.gray_text),
            surface: c(s.window),
            surface_header: c(s.window),
            surface_divider: c(s.window_text),
            surface_border: c(s.window_text),
            card: c(s.window),
            field: c(s.window),
            field_border: c(s.window_text),
            subtle_hover: c(s.window),
            subtle_pressed: c(s.window),
            menu: c(s.window),
            menu_border: c(s.window_text),
            accent: c(s.highlight),
            accent_text: c(s.highlight_text),
            focus: c(s.window_text),
            tile: c(s.window),
            tile_border: c(s.window_text),
        }
    }
    pub const fn themed(dark: bool) -> Self {
        if dark {
            Self {
                dark,
                contrast: false,
                text: rgb(255, 255, 255),
                text_secondary: rgb(197, 197, 197),
                text_disabled: rgb(120, 120, 120),
                surface: rgb(42, 42, 42),
                surface_header: rgb(35, 35, 35),
                surface_divider: rgb(30, 30, 30),
                surface_border: rgb(58, 58, 58),
                card: white(10),
                field: rgb(31, 31, 31),
                field_border: rgb(50, 50, 50),
                subtle_hover: white(15),
                subtle_pressed: white(10),
                menu: rgb(44, 44, 44),
                menu_border: rgb(64, 64, 64),
                accent: rgb(96, 205, 255),
                accent_text: rgb(0, 0, 0),
                focus: rgb(255, 255, 255),
                tile: rgb(55, 55, 55),
                tile_border: rgb(65, 65, 65),
            }
        } else {
            Self {
                dark,
                contrast: false,
                text: rgb(26, 26, 26),
                text_secondary: rgb(96, 96, 96),
                text_disabled: rgb(160, 160, 160),
                surface: rgb(242, 242, 242),
                surface_header: rgb(235, 235, 235),
                surface_divider: rgb(224, 224, 224),
                surface_border: rgb(208, 208, 208),
                card: white(150),
                field: rgb(251, 251, 251),
                field_border: rgb(222, 222, 222),
                subtle_hover: black(10),
                subtle_pressed: black(6),
                menu: rgb(249, 249, 249),
                menu_border: rgb(220, 220, 220),
                accent: rgb(0, 95, 184),
                accent_text: rgb(255, 255, 255),
                focus: rgb(0, 0, 0),
                tile: rgb(251, 251, 251),
                tile_border: rgb(222, 222, 222),
            }
        }
    }
}
pub fn fill(c: &mut Canvas<'_>, r: Rect, color: [u8; 4]) {
    let _ = c.fill_rect(r, color);
}
pub fn rect(c: &mut Canvas<'_>, x: i32, y: i32, w: u32, h: u32, color: [u8; 4]) {
    if let Ok(r) = Rect::new(x, y, w, h) {
        let _ = c.fill_rect(r, color);
    }
}
pub fn rounded(c: &mut Canvas<'_>, r: Rect, radius: u32, color: [u8; 4]) {
    let _ = c.fill_rounded_rect(r, radius, color);
}
pub fn inset(r: Rect, dx: i32, dy: i32) -> Option<Rect> {
    let w = r.width() as i32 - 2 * dx;
    let h = r.height() as i32 - 2 * dy;
    (w > 0 && h > 0)
        .then(|| Rect::new(r.x() + dx, r.y() + dy, w as u32, h as u32).ok())
        .flatten()
}
/// A 1 px antialiased rounded outline (toggle switches, small frames).
pub fn ring(c: &mut Canvas<'_>, r: Rect, radius: u32, color: [u8; 4]) {
    let _ = c.stroke_rounded_rect(r, radius, 1, color);
}
/// A 1 px framed rounded surface: border ring then opaque fill.
pub fn framed(c: &mut Canvas<'_>, r: Rect, radius: u32, border: [u8; 4], body: [u8; 4]) {
    rounded(c, r, radius, border);
    if let Some(inner) = inset(r, 1, 1) {
        rounded(c, inner, radius.saturating_sub(1), body);
    }
}
/// Hairline outline of a rounded rectangle that leaves the interior untouched
/// (for translucent focus rings and control borders).
pub fn outline(c: &mut Canvas<'_>, r: Rect, color: [u8; 4], thickness: u32) {
    let t = thickness.min(r.width() / 2).min(r.height() / 2).max(1);
    rect(c, r.x(), r.y(), r.width(), t, color);
    rect(c, r.x(), r.bottom() - t as i32, r.width(), t, color);
    rect(c, r.x(), r.y() + t as i32, t, r.height() - 2 * t, color);
    rect(
        c,
        r.right() - t as i32,
        r.y() + t as i32,
        t,
        r.height() - 2 * t,
        color,
    );
}
/// Windows 11 keyboard focus visual: 2 px outer ring outside the control,
/// 3 px in contrast themes.
pub fn focus_ring(c: &mut Canvas<'_>, r: Rect, palette: &Palette) {
    let k = if palette.contrast { 3 } else { 2 };
    if let Ok(outer) = Rect::new(
        r.x() - k,
        r.y() - k,
        r.width() + 2 * k as u32,
        r.height() + 2 * k as u32,
    ) {
        outline(c, outer, palette.focus, k as u32);
    }
}
pub fn measure(fonts: &FontAtlas, label: &str, style: TextStyle) -> u32 {
    fonts.measure(label, style).map_or(0, |m| m.advance_width)
}
/// Ink coverage (per channel, out of 255) counted as a label's visible ink.
pub const INK_THRESHOLD: u8 = 32;
/// Text antialiasing. Windows 11 draws shell text on acrylic and other
/// translucent materials (taskbar, Start, Quick Settings and the other tray
/// flyouts, context menus, Task View, Snap) with grayscale coverage, and keeps
/// ClearType (LCD subpixel) coverage for opaque Win32 surfaces such as the
/// Run dialog. Shell helpers default to `Gray`; `lcd::*` draws `Lcd`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Aa {
    #[default]
    Gray,
    Lcd,
}
/// Physical ink box of `text` rastered with its line-box top-left at the
/// physical origin: (left, right, top, bottom), right/bottom exclusive.
/// Measured from the real glyph coverage (bearings) so labels centre on what
/// is visible, not on advance widths. Grayscale, as shell text is drawn.
pub fn ink_box(
    fonts: &FontAtlas,
    text: &str,
    style: TextStyle,
    scale: Scale,
) -> Option<(i32, i32, i32, i32)> {
    ink_box_aa(fonts, text, style, scale, Aa::Gray)
}
/// `ink_box` for text drawn with `aa` (LCD fringes widen the ink).
pub fn ink_box_aa(
    fonts: &FontAtlas,
    text: &str,
    style: TextStyle,
    scale: Scale,
    aa: Aa,
) -> Option<(i32, i32, i32, i32)> {
    if text.is_empty() {
        return None;
    }
    let advance = fonts.measure(text, style).ok()?.advance_width;
    let pad = scale.len(8) as i32;
    let (w, h) = (
        (scale.len(advance) as i32 + 2 * pad).max(1) as usize,
        (scale.len(fonts.line_height(style)) as i32 + 2 * pad) as usize,
    );
    let mut pixels = alloc::vec![0u8; w * h * 4];
    for p in pixels.chunks_exact_mut(4) {
        p[3] = 255;
    }
    {
        let mut target = Target::new(&mut pixels, w as u32, h as u32, w * 4).ok()?;
        if aa == Aa::Lcd {
            target = target.cleartype();
        }
        fonts
            .draw_physical(
                &mut target,
                Clip::new(0, 0, w as u32, h as u32),
                TextRun {
                    text,
                    x: pad,
                    top: pad,
                    style,
                    color: [255, 255, 255, 255],
                },
            )
            .ok()?;
    }
    let (mut left, mut right, mut top, mut bottom) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
    for (i, p) in pixels.chunks_exact(4).enumerate() {
        if p[0].max(p[1]).max(p[2]) >= INK_THRESHOLD {
            let (x, y) = ((i % w) as i32, (i / w) as i32);
            left = left.min(x);
            right = right.max(x + 1);
            top = top.min(y);
            bottom = bottom.max(y + 1);
        }
    }
    (left < right).then_some((left - pad, right - pad, top - pad, bottom - pad))
}
/// Physical line-box top that centres the style's cap height (the ink of
/// "H": cap top to baseline) on `center2`, twice the physical centre. Labels
/// with or without descenders then share one optical centre, as in Windows.
pub fn cap_centered_top(fonts: &FontAtlas, style: TextStyle, scale: Scale, center2: i32) -> i32 {
    match ink_box(fonts, "H", style, scale) {
        Some((_, _, top, bottom)) => (center2 - top - bottom).div_euclid(2),
        None => scale.px(fonts.centered_top(style, scale.logical(center2.div_euclid(2)), 0)),
    }
}
/// Draw text whose line box starts at physical (`x`, `top`), clipped to
/// `clip` (logical) intersected with the canvas clip. Grayscale coverage.
#[allow(clippy::too_many_arguments)]
pub fn text_physical(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    x: i32,
    top: i32,
    label: &str,
    style: TextStyle,
    color: [u8; 4],
    clip: Rect,
) {
    text_physical_aa(c, fonts, x, top, label, style, color, clip, Aa::Gray);
}
/// `text_physical` with an explicit antialiasing mode.
#[allow(clippy::too_many_arguments)]
pub fn text_physical_aa(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    x: i32,
    top: i32,
    label: &str,
    style: TextStyle,
    color: [u8; 4],
    clip: Rect,
    aa: Aa,
) {
    let Ok(physical) = c.scale().rect(clip) else {
        return;
    };
    let Some(region) = c.clip_rect().and_then(|cc| cc.intersection(physical)) else {
        return;
    };
    let (pw, ph, stride) = c.physical_dimensions();
    if let Ok(target) = Target::new(c.pixels_mut(), pw, ph, stride) {
        let mut target = if aa == Aa::Gray {
            target.grayscale()
        } else {
            target.cleartype()
        };
        let _ = fonts.draw_physical(
            &mut target,
            Clip::new(region.x(), region.y(), region.width(), region.height()),
            TextRun {
                text: label,
                x,
                top,
                style,
                color,
            },
        );
    }
}
/// Draw text whose line box starts at logical (`x`, `top`), clipped to
/// `clip` (logical) intersected with the canvas clip.
#[allow(clippy::too_many_arguments)]
pub fn text_clipped(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    x: i32,
    top: i32,
    label: &str,
    style: TextStyle,
    color: [u8; 4],
    clip: Rect,
) {
    let scale = c.scale();
    text_physical(
        c,
        fonts,
        scale.px(x),
        scale.px(top),
        label,
        style,
        color,
        clip,
    );
}
/// Logical clip for a row: the row and the label's line box, with slack.
/// `line` is the label's text-scaled line box (`FontAtlas::line_height`).
fn row_clip(
    x: i32,
    width: u32,
    row_top: i32,
    row_height: u32,
    top: i32,
    line: u32,
    s: Scale,
) -> Option<Rect> {
    let line_top = s.logical(top);
    let y0 = row_top.min(line_top) - 1;
    let y1 = (row_top + row_height as i32).max(line_top + line as i32 + 1) + 2;
    Rect::new(x, y0, width.max(1), (y1 - y0) as u32).ok()
}
/// Text vertically centred (cap height) in a row (`row_top`, `row_height`),
/// left aligned at `x` and clipped to `max_width` logical pixels.
#[allow(clippy::too_many_arguments)]
pub fn text_row(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    x: i32,
    row_top: i32,
    row_height: u32,
    label: &str,
    style: TextStyle,
    color: [u8; 4],
    max_width: u32,
) {
    text_row_aa(
        c,
        fonts,
        x,
        row_top,
        row_height,
        label,
        style,
        color,
        max_width,
        Aa::Gray,
    );
}
#[allow(clippy::too_many_arguments)]
fn text_row_aa(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    x: i32,
    row_top: i32,
    row_height: u32,
    label: &str,
    style: TextStyle,
    color: [u8; 4],
    max_width: u32,
    aa: Aa,
) {
    let s = c.scale();
    let top = cap_centered_top(
        fonts,
        style,
        s,
        s.px(row_top) + s.px(row_top + row_height as i32),
    );
    if let Some(clip) = row_clip(
        x,
        max_width,
        row_top,
        row_height,
        top,
        fonts.line_height(style),
        s,
    ) {
        text_physical_aa(c, fonts, s.px(x), top, label, style, color, clip, aa);
    }
}
/// Text centred on the span `left`..`left + width` (its visible ink, to the
/// physical pixel) and on the row's cap height. A label wider than the span
/// starts at its left edge and is clipped.
#[allow(clippy::too_many_arguments)]
pub fn text_span(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    left: i32,
    width: u32,
    row_top: i32,
    row_height: u32,
    label: &str,
    style: TextStyle,
    color: [u8; 4],
) {
    text_span_aa(
        c,
        fonts,
        left,
        width,
        row_top,
        row_height,
        label,
        style,
        color,
        Aa::Gray,
    );
}
#[allow(clippy::too_many_arguments)]
fn text_span_aa(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    left: i32,
    width: u32,
    row_top: i32,
    row_height: u32,
    label: &str,
    style: TextStyle,
    color: [u8; 4],
    aa: Aa,
) {
    let s = c.scale();
    let top = cap_centered_top(
        fonts,
        style,
        s,
        s.px(row_top) + s.px(row_top + row_height as i32),
    );
    let x = if measure(fonts, label, style) > width {
        s.px(left)
    } else {
        match ink_box_aa(fonts, label, style, s, aa) {
            Some((l, r, _, _)) => (s.px(left) + s.px(left + width as i32) - l - r).div_euclid(2),
            None => s.px(left),
        }
    };
    if let Some(clip) = row_clip(
        left,
        width,
        row_top,
        row_height,
        top,
        fonts.line_height(style),
        s,
    ) {
        text_physical_aa(c, fonts, x, top, label, style, color, clip, aa);
    }
}
/// Label centred in `r` both ways (ink horizontally, cap height vertically).
pub fn text_in(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    r: Rect,
    label: &str,
    style: TextStyle,
    color: [u8; 4],
) {
    text_span(
        c,
        fonts,
        r.x(),
        r.width(),
        r.y(),
        r.height(),
        label,
        style,
        color,
    );
}
/// Text centred horizontally on `cx` within a row, at most `max_width` wide.
#[allow(clippy::too_many_arguments)]
pub fn text_center(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    cx: i32,
    row_top: i32,
    row_height: u32,
    label: &str,
    style: TextStyle,
    color: [u8; 4],
    max_width: u32,
) {
    let max_width = max_width.max(1);
    text_span(
        c,
        fonts,
        cx - (max_width / 2) as i32,
        max_width - max_width % 2,
        row_top,
        row_height,
        label,
        style,
        color,
    );
}
/// Right-aligned text ending at `right`.
#[allow(clippy::too_many_arguments)]
pub fn text_right(
    c: &mut Canvas<'_>,
    fonts: &FontAtlas,
    right: i32,
    row_top: i32,
    row_height: u32,
    label: &str,
    style: TextStyle,
    color: [u8; 4],
) {
    let width = measure(fonts, label, style);
    text_row(
        c,
        fonts,
        right - width as i32,
        row_top,
        row_height,
        label,
        style,
        color,
        width + 2,
    );
}
/// ClearType (LCD subpixel) variants of the row helpers, for text on opaque
/// Win32 surfaces (the Run dialog). Shell text on acrylic stays grayscale.
pub mod lcd {
    use super::{Aa, Canvas, FontAtlas, Rect, TextStyle, measure, text_row_aa, text_span_aa};
    #[allow(clippy::too_many_arguments)]
    pub fn text_row(
        c: &mut Canvas<'_>,
        fonts: &FontAtlas,
        x: i32,
        row_top: i32,
        row_height: u32,
        label: &str,
        style: TextStyle,
        color: [u8; 4],
        max_width: u32,
    ) {
        text_row_aa(
            c,
            fonts,
            x,
            row_top,
            row_height,
            label,
            style,
            color,
            max_width,
            Aa::Lcd,
        );
    }
    #[allow(clippy::too_many_arguments)]
    pub fn text_right(
        c: &mut Canvas<'_>,
        fonts: &FontAtlas,
        right: i32,
        row_top: i32,
        row_height: u32,
        label: &str,
        style: TextStyle,
        color: [u8; 4],
    ) {
        let width = measure(fonts, label, style);
        text_row(
            c,
            fonts,
            right - width as i32,
            row_top,
            row_height,
            label,
            style,
            color,
            width + 2,
        );
    }
    pub fn text_in(
        c: &mut Canvas<'_>,
        fonts: &FontAtlas,
        r: Rect,
        label: &str,
        style: TextStyle,
        color: [u8; 4],
    ) {
        text_span_aa(
            c,
            fonts,
            r.x(),
            r.width(),
            r.y(),
            r.height(),
            label,
            style,
            color,
            Aa::Lcd,
        );
    }
}
/// Fit `label` into `max_width`, replacing the tail with "..." when needed.
pub fn ellipsize(
    fonts: &FontAtlas,
    label: &str,
    style: TextStyle,
    max_width: u32,
) -> alloc::string::String {
    ellipsize_by(label, max_width, |s| measure(fonts, s, style))
}
/// Bytes of a label kept per logical pixel of room: no visible grapheme is
/// narrower than a pixel or longer than this, so a longer tail cannot fit
/// and is never measured.
const ELLIPSIZE_BYTES_PER_PX: usize = 32;
/// `ellipsize` with the measurement supplied: a binary search over grapheme
/// boundaries, so a label of any length costs at most
/// `2 + log2(32 * max_width)` measurements of at most `32 * max_width` bytes.
pub fn ellipsize_by(
    label: &str,
    max_width: u32,
    mut measure: impl FnMut(&str) -> u32,
) -> alloc::string::String {
    use alloc::string::String;
    use door_graphics_text::{grapheme_floor, next_grapheme};
    let cap = (max_width as usize).saturating_mul(ELLIPSIZE_BYTES_PER_PX);
    let label = if label.len() > cap {
        &label[..grapheme_floor(label, cap)]
    } else if measure(label) <= max_width {
        return String::from(label);
    } else {
        label
    };
    let candidate = |end: usize| alloc::format!("{}...", label[..end].trim_end());
    // The prefix ending at `lo` fits (or is empty); the one at `hi` does not.
    let (mut lo, mut hi) = (0usize, label.len());
    while hi - lo > 1 {
        let mut mid = grapheme_floor(label, lo + (hi - lo) / 2);
        if mid <= lo {
            mid = next_grapheme(label, lo);
            if mid >= hi {
                break;
            }
        }
        if measure(&candidate(mid)) <= max_width {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    if lo == 0 {
        String::from("...")
    } else {
        candidate(lo)
    }
}
extern crate alloc;
/// Coverage mask set for one logical glyph size at densities 1x/2x/3x.
pub type Mask = [&'static [u8]; 3];
#[macro_export]
macro_rules! shell_mask {
    ($name:literal, 12) => {
        $crate::shell_mask!(@ $name, "12", "24", "12-3x")
    };
    ($name:literal, 16) => {
        $crate::shell_mask!(@ $name, "16", "32", "16-3x")
    };
    ($name:literal, 20) => {
        $crate::shell_mask!(@ $name, "20", "40", "20-3x")
    };
    ($name:literal, 24) => {
        $crate::shell_mask!(@ $name, "24", "48", "24-3x")
    };
    (@ $name:literal, $one:literal, $two:literal, $three:literal) => {
        [
            &include_bytes!(concat!("../../../assets/icons/windows-shell/", $name, ".", $one, ".mask"))[..],
            &include_bytes!(concat!("../../../assets/icons/windows-shell/", $name, ".", $two, ".mask"))[..],
            &include_bytes!(concat!("../../../assets/icons/windows-shell/", $name, ".", $three, ".mask"))[..],
        ]
    };
}
/// Tint a native-density Segoe Fluent coverage mask at logical (`x`, `y`).
/// Fractional scales area-filter the next larger native density.
pub fn glyph(c: &mut Canvas<'_>, x: i32, y: i32, size: u32, mask: Mask, color: [u8; 4]) {
    let scale = c.scale();
    let factor = scale.asset_factor();
    let Some(bytes) = mask.get(factor as usize - 1) else {
        return;
    };
    let source = (size * factor) as usize;
    if bytes.len() != source * source {
        return;
    }
    let physical = scale.len(size) as usize;
    let ox = scale.px(x);
    let oy = scale.px(y);
    let Some(clip) = c.clip_rect() else {
        return;
    };
    for i in 0..physical * physical {
        let (gx, gy) = (i % physical, i / physical);
        let a = door_graphics_core::area_sample(bytes, source, physical, gx, gy);
        if a == 0 {
            continue;
        }
        let px = ox + gx as i32;
        let py = oy + gy as i32;
        if clip.contains(px, py) {
            let _ = c.pixel(px, py, alpha(color, a));
        }
    }
}
/// Glyph centered in a logical rectangle.
pub fn glyph_in(c: &mut Canvas<'_>, r: Rect, size: u32, mask: Mask, color: [u8; 4]) {
    glyph(
        c,
        r.x() + (r.width() as i32 - size as i32) / 2,
        r.y() + (r.height() as i32 - size as i32) / 2,
        size,
        mask,
        color,
    );
}
/// Filled antialiased circle.
pub fn circle(c: &mut Canvas<'_>, cx: i32, cy: i32, radius: u32, color: [u8; 4]) {
    if let Ok(r) = Rect::new(
        cx - radius as i32,
        cy - radius as i32,
        radius * 2,
        radius * 2,
    ) {
        let _ = c.fill_rounded_rect(r, radius, color);
    }
}
/// Blend the physical pixels of `region` back toward `underlay` so the region
/// shows `opacity`/255 of what was painted (whole-surface fade animation).
/// `underlay` is a full-output BGRA image with the canvas stride.
pub fn fade(c: &mut Canvas<'_>, region: Rect, underlay: &[u8], opacity: u8) {
    if opacity == 255 {
        return;
    }
    let Some(r) = c.clip_rect().and_then(|clip| clip.intersection(region)) else {
        return;
    };
    let stride = c.physical_dimensions().2;
    let pixels = c.pixels_mut();
    if underlay.len() < pixels.len() {
        return;
    }
    let a = opacity as u32;
    let inv = 255 - a;
    for y in r.y() as usize..r.bottom() as usize {
        let start = y * stride + r.x() as usize * 4;
        let end = start + r.width() as usize * 4;
        for (p, u) in pixels[start..end].iter_mut().zip(&underlay[start..end]) {
            *p = ((*p as u32 * a + *u as u32 * inv + 127) / 255) as u8;
        }
    }
}
/// Physical rectangle for a logical one (edges snapped per `Scale::rect`).
pub fn physical(r: Rect, scale: Scale) -> Rect {
    scale.rect(r).unwrap_or(r)
}
/// Union of two rectangles.
pub fn union(a: Rect, b: Rect) -> Rect {
    let x = a.x().min(b.x());
    let y = a.y().min(b.y());
    let right = a.right().max(b.right());
    let bottom = a.bottom().max(b.bottom());
    Rect::new(x, y, (right - x) as u32, (bottom - y) as u32).unwrap_or(a)
}
/// Grow a rectangle by `margin` on every side, clamped to `bounds`.
pub fn grow(r: Rect, margin: i32, bounds: Rect) -> Rect {
    let x = (r.x() - margin).max(bounds.x());
    let y = (r.y() - margin).max(bounds.y());
    let right = (r.right() + margin).min(bounds.right());
    let bottom = (r.bottom() + margin).min(bounds.bottom());
    if right <= x || bottom <= y {
        return r;
    }
    Rect::new(x, y, (right - x) as u32, (bottom - y) as u32).unwrap_or(r)
}
/// Layered soft shadow under a flyout (Windows 11 "Flyout" elevation). Only
/// the ring outside the (opaque, painted-next) surface is touched, so a large
/// Start panel costs its perimeter rather than its area.
pub fn shadow(c: &mut Canvas<'_>, r: Rect, radius: u32, dark: bool) {
    if contrast().is_some() {
        return; // Contrast themes draw flat, bordered surfaces.
    }
    let layers: [(i32, i32, u8); 4] = if dark {
        [(16, 8, 10), (10, 5, 18), (5, 2, 28), (1, 1, 40)]
    } else {
        [(16, 8, 6), (10, 5, 10), (5, 2, 16), (1, 1, 24)]
    };
    let Some(saved) = c.clip_rect() else {
        return;
    };
    let scale = c.scale();
    let m = 16 + 8;
    let inner = radius as i32;
    // Four strips around the surface; the corner squares (radius) are
    // included in the top/bottom strips so rounded corners get shadow too.
    let strips = [
        (r.x() - m, r.y() - m, r.width() as i32 + 2 * m, m + inner),
        (
            r.x() - m,
            r.bottom() - inner,
            r.width() as i32 + 2 * m,
            m + inner,
        ),
        (r.x() - m, r.y() + inner, m, r.height() as i32 - 2 * inner),
        (r.right(), r.y() + inner, m, r.height() as i32 - 2 * inner),
    ];
    for (x, y, w, h) in strips {
        if w <= 0 || h <= 0 {
            continue;
        }
        let Ok(strip) = Rect::new(x, y, w as u32, h as u32).and_then(|r| scale.rect(r)) else {
            continue;
        };
        let Some(clip) = saved.intersection(strip) else {
            continue;
        };
        c.set_clip(clip);
        for (spread, drop, a) in layers {
            if let Ok(s) = Rect::new(
                r.x() - spread,
                r.y() - spread + drop,
                r.width() + 2 * spread as u32,
                r.height() + 2 * spread as u32,
            ) {
                rounded(c, s, radius + spread as u32, black(a));
            }
        }
    }
    c.set_clip(saved);
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    /// Shell accent fills follow the system accent palette: Light2 in dark
    /// (the owner's #DB9EE5), Dark1 in light; the default keeps the WinUI
    /// blue; contrast themes keep Highlight.
    #[test]
    fn shell_accent_follows_the_system_accent_palette() {
        use door_design_tokens::accent::{DEFAULT, OWNER_REFERENCE};
        assert_eq!(
            Palette::themed(true).with_accent(&DEFAULT).accent,
            rgb(96, 205, 255)
        );
        assert_eq!(
            Palette::themed(false).with_accent(&DEFAULT).accent,
            rgb(0, 95, 184)
        );
        assert_eq!(
            Palette::themed(true).with_accent(&OWNER_REFERENCE).accent,
            rgb(0xdb, 0x9e, 0xe5)
        );
        assert_eq!(
            Palette::themed(false).with_accent(&OWNER_REFERENCE).accent,
            rgb(0x8e, 0x3a, 0xa7)
        );
        let hc = Palette::contrast_theme(door_service_protocol::desktop::ContrastTheme::Aquatic);
        assert_eq!(hc.with_accent(&OWNER_REFERENCE).accent, hc.accent);
        assert_eq!(accent_palette(), DEFAULT);
    }
    /// Ink box (physical, max exclusive) inside `r` (physical): pixels moved
    /// at least `INK_THRESHOLD`/255 of the way from `bg` towards `fg` in some
    /// channel (the same rule `ink_box` uses).
    pub(crate) fn ink(
        bytes: &[u8],
        width: usize,
        r: Rect,
        bg: [u8; 4],
        fg: [u8; 4],
    ) -> Option<(i32, i32, i32, i32)> {
        let (mut l, mut rr, mut t, mut b) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
        for y in r.y()..r.bottom() {
            for x in r.x()..r.right() {
                let p = &bytes[(y as usize * width + x as usize) * 4..][..3];
                let hit = (0..3).any(|ch| {
                    let range = (i32::from(fg[ch]) - i32::from(bg[ch])).abs();
                    range > 0
                        && (i32::from(p[ch]) - i32::from(bg[ch])).abs() * 255
                            >= range * i32::from(INK_THRESHOLD)
                });
                if hit {
                    l = l.min(x);
                    rr = rr.max(x + 1);
                    t = t.min(y);
                    b = b.max(y + 1);
                }
            }
        }
        (l < rr).then_some((l, rr, t, b))
    }
    pub(crate) fn physical(r: Rect, s: i32) -> Rect {
        Rect::new(
            r.x() * s,
            r.y() * s,
            r.width() * s as u32,
            r.height() * s as u32,
        )
        .unwrap()
    }
    /// Largest per-pixel spread, in 1/255 coverage units, between the R, G
    /// and B coverage of text drawn in `fg` over a uniform `bg` inside `r`
    /// (physical), over pixels with at least 16/255 coverage in some channel.
    /// Grayscale antialiasing keeps it at rounding level; ClearType fringes
    /// put one channel far ahead of the others.
    pub(crate) fn channel_spread(
        bytes: &[u8],
        width: usize,
        r: Rect,
        bg: [u8; 4],
        fg: [u8; 4],
    ) -> (i32, usize) {
        let (mut spread, mut edges) = (0, 0);
        for y in r.y()..r.bottom() {
            for x in r.x()..r.right() {
                let p = &bytes[(y as usize * width + x as usize) * 4..][..3];
                let cov: [i32; 3] = core::array::from_fn(|ch| {
                    let range = i32::from(fg[ch]) - i32::from(bg[ch]);
                    (i32::from(p[ch]) - i32::from(bg[ch])) * 255 / range
                });
                let (lo, hi) = (cov.iter().min().unwrap(), cov.iter().max().unwrap());
                if *hi >= 16 {
                    edges += 1;
                    spread = spread.max(hi - lo);
                }
            }
        }
        (spread, edges)
    }
    /// Shell text helpers draw grayscale coverage on opaque, tinted (acrylic)
    /// pixels, at 1x and 2x; the Run dialog's `lcd` helpers keep ClearType.
    #[test]
    fn shell_text_is_grayscale_and_run_dialog_text_keeps_cleartype() {
        // An acrylic-like tint, opaque in the frame: the old LCD path applied.
        let (bg, fg) = (rgb(214, 226, 240), rgb(96, 96, 96));
        for scale in [1u32, 2] {
            let s = scale as i32;
            let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
            let (pw, ph) = (320 * scale as usize, 80 * scale as usize);
            let region = physical(Rect::new(0, 0, 320, 80).unwrap(), s);
            let draw = |lcd: bool| {
                let mut bytes = alloc::vec![0u8; pw * ph * 4];
                for p in bytes.chunks_exact_mut(4) {
                    p.copy_from_slice(&bg);
                }
                let mut c = Canvas::new(&mut bytes, pw as u32, ph as u32, pw * 4).unwrap();
                c.set_scale(door_graphics_core::Scale::from_factor(scale).unwrap())
                    .unwrap();
                let r = Rect::new(8, 44, 140, 28).unwrap();
                if lcd {
                    lcd::text_row(
                        &mut c,
                        &fonts,
                        12,
                        8,
                        36,
                        "Search the web",
                        TextStyle::Regular14,
                        fg,
                        280,
                    );
                    lcd::text_in(&mut c, &fonts, r, "Cancel", TextStyle::Regular14, fg);
                    lcd::text_right(
                        &mut c,
                        &fonts,
                        300,
                        44,
                        28,
                        "8:20 PM",
                        TextStyle::Regular12,
                        fg,
                    );
                } else {
                    text_row(
                        &mut c,
                        &fonts,
                        12,
                        8,
                        36,
                        "Search the web",
                        TextStyle::Regular14,
                        fg,
                        280,
                    );
                    text_in(&mut c, &fonts, r, "Cancel", TextStyle::Regular14, fg);
                    text_right(
                        &mut c,
                        &fonts,
                        300,
                        44,
                        28,
                        "8:20 PM",
                        TextStyle::Regular12,
                        fg,
                    );
                }
                bytes
            };
            let (gray, edges) = channel_spread(&draw(false), pw, region, bg, fg);
            assert!(edges > 100, "{scale}x: text drawn");
            assert!(
                gray <= 4,
                "{scale}x shell text has colour fringes: spread {gray}/255"
            );
            let (cleartype, _) = channel_spread(&draw(true), pw, region, bg, fg);
            assert!(
                cleartype >= 64,
                "{scale}x Run text lost ClearType: spread {cleartype}/255"
            );
            // Ink is measured in the mode it is drawn: centred labels stay
            // centred both ways.
            let r = physical(Rect::new(8, 44, 140, 28).unwrap(), s);
            for lcd in [false, true] {
                let (l, rr, _, _) = ink(&draw(lcd), pw, r, bg, fg).unwrap();
                assert!(
                    (l + rr - (2 * r.x() + r.width() as i32)).abs() <= s,
                    "{scale}x lcd={lcd}: {l}..{rr} in {r:?}"
                );
            }
        }
    }
    /// Every boxed label kind in the shell, at 1x and 2x: centred labels put
    /// their ink centre within 0.5 logical px of the box centre; every row
    /// (centred or left aligned) puts the cap height ("H") centre within
    /// 0.5 logical px of the row centre.
    #[test]
    fn boxed_labels_centre_on_ink_and_cap_height_at_1x_and_2x() {
        use crate::taskbar::{Layout, TrayTarget};
        let layout = Layout::new(1280, 720);
        let fonts1 = FontAtlas::windows_reference_scaled(1).unwrap();
        let g = crate::start::Geometry::new(layout);
        let tile = crate::tray::tile(layout, 3);
        let mut centred: alloc::vec::Vec<(Rect, &str, TextStyle)> = alloc::vec::Vec::new();
        for f in crate::search::Filter::ALL {
            centred.push((g.chip(&fonts1, f), f.label(), TextStyle::Regular14));
        }
        let pinned = g.pinned_tile(0, 0).unwrap();
        let card = g.card(0, 0);
        centred.extend([
            (
                layout.tray(TrayTarget::Language),
                "ENG",
                TextStyle::Regular12,
            ),
            (
                Rect::new(tile.x() - 6, tile.bottom() + 8, tile.width() + 12, 18).unwrap(),
                "Accessibility",
                TextStyle::Regular12,
            ),
            (
                Rect::new(900, 400, 40, 40).unwrap(),
                "22",
                TextStyle::Regular14,
            ),
            (
                Rect::new(940, 360, 40, 28).unwrap(),
                "Su",
                TextStyle::Regular12,
            ),
            (
                Rect::new(pinned.x(), pinned.y() + 44, pinned.width(), 20).unwrap(),
                "File Explorer",
                TextStyle::Regular12,
            ),
            (
                Rect::new(card.x() - 12, card.bottom() + 13, card.width() + 24, 20).unwrap(),
                "Utilities & tools",
                TextStyle::Regular12,
            ),
            (
                Rect::new(500, 300, 301, 28).unwrap(),
                "Settings",
                TextStyle::Semibold20,
            ),
            (
                Rect::new(500, 340, 300, 24).unwrap(),
                "No new notifications",
                TextStyle::Regular14,
            ),
        ]);
        // Left-aligned rows: menu items, Start rows, header toggles, the
        // taskbar search pill, account button, Snap Assist / Task View titles.
        let rows: [(i32, u32, TextStyle); 7] = [
            (100, 36, TextStyle::Regular14),
            (150, 40, TextStyle::Regular14),
            (200, 28, TextStyle::Regular14),
            (240, 32, TextStyle::Regular14),
            (280, 24, TextStyle::Regular12),
            (310, 20, TextStyle::Regular12),
            (340, 24, TextStyle::Semibold14),
        ];
        let (bg, fg) = (rgb(251, 251, 251), rgb(26, 26, 26));
        for scale in [1u32, 2] {
            let s = scale as i32;
            let fonts = FontAtlas::windows_reference_scaled(scale).unwrap();
            let (pw, ph) = (1280 * scale as usize, 720 * scale as usize);
            let mut bytes = alloc::vec![0u8; pw * ph * 4];
            let fill = |bytes: &mut [u8]| {
                for p in bytes.chunks_exact_mut(4) {
                    p.copy_from_slice(&bg);
                }
            };
            for (r, label, style) in &centred {
                for (text, vertical) in [(*label, false), ("H", true)] {
                    fill(&mut bytes);
                    let mut c = Canvas::new(&mut bytes, pw as u32, ph as u32, pw * 4).unwrap();
                    c.set_scale(door_graphics_core::Scale::from_factor(scale).unwrap())
                        .unwrap();
                    text_in(&mut c, &fonts, *r, text, *style, fg);
                    let (l, rr, t, b) = ink(&bytes, pw, physical(*r, s), bg, fg)
                        .unwrap_or_else(|| panic!("{text} not drawn in {r:?}"));
                    if vertical {
                        let target = (2 * r.y() + r.height() as i32) * s;
                        assert!(
                            (t + b - target).abs() <= s,
                            "{scale}x cap of {label:?} in {r:?}: {t}..{b} vs {target}/2"
                        );
                    } else {
                        let target = (2 * r.x() + r.width() as i32) * s;
                        assert!(
                            (l + rr - target).abs() <= s,
                            "{scale}x {label:?} in {r:?}: ink {l}..{rr} vs {target}/2"
                        );
                    }
                }
            }
            for (top, height, style) in rows {
                fill(&mut bytes);
                let mut c = Canvas::new(&mut bytes, pw as u32, ph as u32, pw * 4).unwrap();
                c.set_scale(door_graphics_core::Scale::from_factor(scale).unwrap())
                    .unwrap();
                text_row(&mut c, &fonts, 40, top, height, "H", style, fg, 200);
                let r = Rect::new(40, top - 8, 200, height + 16).unwrap();
                let (_, _, t, b) = ink(&bytes, pw, physical(r, s), bg, fg).unwrap();
                let target = (2 * top + height as i32) * s;
                assert!(
                    (t + b - target).abs() <= s,
                    "{scale}x row {top}+{height}: {t}..{b}"
                );
            }
        }
    }
}
