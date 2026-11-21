//! Shared private-pixel drawing for the two P11 consumers. A small original
//! application view, not a replacement for the P12 UI toolkit.
use crate::{icons::Icon, tokens};
use door_graphics_text::{FontAtlas, Target, TextRun, TextStyle};
#[path = "ui/dialog.rs"]
mod dialog;
/// Decoded pixels keyed by (source address, decode kind).
type CachedImages = alloc::vec::Vec<((usize, bool), alloc::vec::Vec<u8>)>;
/// Bounded decoded preview cache shared across redraws, not per-frame decoding.
#[derive(Default)]
pub struct ImageCache(core::cell::RefCell<CachedImages>);
/// Cache key of a derived form (`kind` 1 QOI decode, 2 mask resample) of
/// `source` at `target` physical pixels. User addresses stay below bit 48.
fn derived_key(source: &[u8], kind: usize, target: usize) -> usize {
    source.as_ptr() as usize ^ ((target & 0xfff) << 48) ^ (kind << 60)
}
impl ImageCache {
    fn remember(cache: &mut CachedImages, key: (usize, bool), pixels: alloc::vec::Vec<u8>) {
        // Static icons decode once, with explicit resident-byte and entry limits.
        // A mixed Explorer page must not evict its own icons every repaint.
        const BYTES: usize = 8 * 1024 * 1024;
        while !cache.is_empty()
            && (cache.len() >= 96
                || cache.iter().map(|(_, p)| p.len()).sum::<usize>() + pixels.len() > BYTES)
        {
            cache.remove(0);
        }
        cache.push((key, pixels));
    }
}
pub struct Command {
    pub y: i32,
    pub id: u32,
    pub x: i32,
    pub width: u32,
    pub label: &'static str,
    pub icon: Icon,
    pub accent: bool,
}
/// Command button height: 30 px grown with its 12 px label's line box.
pub fn command_height() -> u32 {
    door_workspace::palette::text_grow(tokens::CONTROL_HEIGHT.value, TextStyle::Regular12)
}
impl Command {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        (self.y..self.y + command_height() as i32).contains(&y)
            && (self.x..self.x + self.width as i32).contains(&x)
    }
    pub fn draw(&self, v: &mut View<'_>) {
        v.button(
            self.x,
            self.y,
            self.width,
            self.label,
            self.icon,
            self.accent,
        );
    }
}
/// Physical pixels saved from a view (`View::snapshot`).
pub struct Snapshot {
    x: i32,
    y: i32,
    width: usize,
    height: usize,
    pixels: alloc::vec::Vec<u8>,
}
pub struct View<'a> {
    pixels: &'a mut [u8],
    pub palette: door_workspace::palette::Palette,
    pub explorer_style: bool,
    pub materials: bool,
    pub width: u32,
    pub height: u32,
    /// Density of the native asset frames (1x/2x/3x) sampled at this zoom,
    /// `zoom.asset_factor()`. Masks and bitmaps chosen by it are resampled to
    /// their physical size when the zoom is fractional.
    pub scale: u32,
    /// Display scale: logical edges map to physical pixels with `zoom.px`
    /// (snapped edges, so abutting rectangles never seam or overlap).
    pub zoom: door_graphics_core::Scale,
    physical_width: u32,
    physical_height: u32,
    fonts: &'a FontAtlas,
    mono_fonts: Option<&'a FontAtlas>,
    images: Option<&'a ImageCache>,
    clip: door_graphics_core::Rect,
    scroll_y: i32,
    pub hover: Option<(i32, i32)>,
    pub pressed: Option<(i32, i32)>,
}
impl<'a> View<'a> {
    pub fn measure_small(&self, text: &str) -> u32 {
        self.fonts
            .measure(text, TextStyle::Regular12)
            .map_or(0, |m| m.advance_width)
    }
    pub fn measure(&self, text: &str) -> u32 {
        self.fonts
            .measure(text, TextStyle::Regular14)
            .map_or(0, |m| m.advance_width)
    }
    /// Line box height of `style` at the applied text size (logical px).
    pub fn line_height(&self, style: TextStyle) -> u32 {
        self.fonts.line_height(style)
    }
    pub fn measure_style(&self, text: &str, style: TextStyle) -> u32 {
        self.fonts
            .measure(text, style)
            .map_or(0, |m| m.advance_width)
    }
    /// Geometry and hit targets remain in96-DPI logical units. Backing pixels,
    /// font glyphs and selected offline icon exports use the negotiated density.
    pub fn new_scaled(
        pixels: &'a mut [u8],
        physical_width: u32,
        physical_height: u32,
        fonts: &'a FontAtlas,
        scale: u32,
    ) -> Self {
        assert!(matches!(scale, 1..=3));
        let zoom = door_graphics_core::Scale::from_factor(scale).expect("integer scale");
        Self::new_zoomed(pixels, physical_width, physical_height, fonts, zoom)
    }
    /// A view at any display scale (125%, 150%, ...); `fonts` is the atlas
    /// for the same percent.
    pub fn new_zoomed(
        pixels: &'a mut [u8],
        physical_width: u32,
        physical_height: u32,
        fonts: &'a FontAtlas,
        zoom: door_graphics_core::Scale,
    ) -> Self {
        let width = zoom.logical_len(physical_width);
        let height = zoom.logical_len(physical_height);
        Self {
            pixels,
            palette: door_workspace::palette::Palette::default(),
            explorer_style: false,
            materials: false,
            width,
            height,
            scale: zoom.asset_factor(),
            zoom,
            physical_width,
            physical_height,
            fonts,
            mono_fonts: None,
            images: None,
            clip: door_graphics_core::Rect::new(0, 0, width, height).expect("owned surface"),
            scroll_y: 0,
            hover: None,
            pressed: None,
        }
    }
    pub fn with_images(mut self, images: &'a ImageCache) -> Self {
        self.images = Some(images);
        self
    }
    pub fn with_monospace(mut self, fonts: Option<&'a FontAtlas>) -> Self {
        self.mono_fonts = fonts;
        self
    }
    /// Use the editor atlas for one coherent layout/selection/caret paint pass.
    /// The atlas editor text is laid out and painted with (`with_monospace`,
    /// else the UI atlas), so input handling and paint share one layout.
    pub fn editor_fonts(&self) -> &'a FontAtlas {
        self.mono_fonts.unwrap_or(self.fonts)
    }
    /// Paint a Door Text layout with its first line box top-left at logical
    /// (`x`, `top`), clipped to the viewport; `color` maps like `text`.
    pub fn text_layout(
        &mut self,
        x: i32,
        top: i32,
        system: &door_graphics_text::TextSystem,
        layout: &door_graphics_text::TextLayout,
        color: [u8; 4],
    ) {
        let color = self.palette.foreground(color);
        let clip = self.physical_clip();
        let (px, py) = (
            self.px(i64::from(x)),
            self.px(i64::from(top - self.scroll_y)),
        );
        let fit = |v: i64| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        let mut target = Target::new(
            self.pixels,
            self.physical_width,
            self.physical_height,
            self.physical_width as usize * 4,
        )
        .expect("owned surface");
        let clip = door_graphics_text::Clip::new(clip.x(), clip.y(), clip.width(), clip.height());
        let _ = system.draw(layout, &mut target, clip, fit(px), fit(py), color);
    }
    pub fn with_editor_font<T>(&mut self, paint: impl FnOnce(&mut Self) -> T) -> T {
        let previous = self.fonts;
        self.fonts = self.mono_fonts.unwrap_or(previous);
        let result = paint(self);
        self.fonts = previous;
        result
    }
    pub fn monospace_text(
        &mut self,
        x: i32,
        y: i32,
        width: u32,
        text: &str,
        style: TextStyle,
        color: [u8; 4],
    ) {
        let previous = self.fonts;
        self.fonts = self.mono_fonts.unwrap_or(previous);
        self.text_clip(x, y, width, text, style, color);
        self.fonts = previous;
    }
    /// Physical coordinate of a logical one (`zoom.px`, 64-bit).
    fn px(&self, logical: i64) -> i64 {
        (logical * i64::from(self.zoom.percent()) + 49).div_euclid(100)
    }
    /// Physical rectangle of a logical one (window coordinates, no scroll).
    fn physical_rect(&self, x: i32, y: i32, w: u32, h: u32) -> Option<door_graphics_core::Rect> {
        let (left, top) = (self.px(i64::from(x)), self.px(i64::from(y)));
        let right = self.px(i64::from(x) + i64::from(w));
        let bottom = self.px(i64::from(y) + i64::from(h));
        let fit = |v: i64| i32::try_from(v).ok();
        door_graphics_core::Rect::new(
            fit(left)?,
            fit(top)?,
            u32::try_from(right - left).ok()?,
            u32::try_from(bottom - top).ok()?,
        )
        .ok()
    }
    /// Physical length of a logical length measured from a snapped edge.
    pub fn physical_len(&self, logical: u32) -> u32 {
        self.zoom.len(logical)
    }
    fn physical_clip(&self) -> door_graphics_core::Rect {
        let right = if self.clip.right() == self.width as i32 {
            self.physical_width
        } else {
            self.px(i64::from(self.clip.right().max(0))) as u32
        };
        let bottom = if self.clip.bottom() == self.height as i32 {
            self.physical_height
        } else {
            self.px(i64::from(self.clip.bottom().max(0))) as u32
        };
        let x = self.px(i64::from(self.clip.x())) as i32;
        let y = self.px(i64::from(self.clip.y())) as i32;
        door_graphics_core::Rect::new(
            x,
            y,
            right.saturating_sub(x.max(0) as u32),
            bottom.saturating_sub(y.max(0) as u32),
        )
        .expect("physical viewport")
    }
    pub fn viewport(&mut self, rect: door_graphics_core::Rect, scroll_y: i32) {
        self.clip = rect;
        self.scroll_y = scroll_y;
    }
    pub fn reset_viewport(&mut self) {
        self.clip =
            door_graphics_core::Rect::new(0, 0, self.width, self.height).expect("owned surface");
        self.scroll_y = 0;
    }
    fn surface_color(&self, color: [u8; 4]) -> [u8; 4] {
        if !self.explorer_style || !self.palette.dark {
            return self.palette.surface(color);
        }
        match color {
            [255, 255, 255, 255] => [25, 25, 25, 255],
            [250, 247, 243, 255] => [36, 31, 27, 255],
            [243, 235, 225, 255] => [31, 11, 0, 255],
            [252, 251, 249, 255] => [38, 33, 28, 255],
            [253, 252, 250, 255] => [25, 25, 25, 255],
            [253, 251, 248, 255] => [25, 25, 25, 255],
            [246, 240, 233, 255] => [25, 25, 25, 255],
            [230, 226, 221, 255] | [250, 249, 247, 255] => [55, 45, 39, 255],
            [250, 233, 213, 255] => [61, 61, 61, 255],
            [240, 220, 197, 255] => [73, 73, 73, 255],
            [249, 243, 235, 255] => [47, 47, 47, 255],
            [238, 235, 231, 255] | [233, 228, 222, 255] | [236, 231, 224, 255] => [57, 57, 57, 255],
            _ => self.palette.surface(color),
        }
    }
    pub fn rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: [u8; 4]) {
        let color = self.surface_color(color);
        let y = y - self.scroll_y;
        let clip = self.physical_clip();
        let left = self.px(i64::from(x)).max(0).max(i64::from(clip.x())) as u32;
        let top = self.px(i64::from(y)).max(0).max(i64::from(clip.y())) as u32;
        let logical_right = i64::from(x) + i64::from(w);
        let logical_bottom = i64::from(y) + i64::from(h);
        let right = if logical_right == i64::from(self.width) {
            self.physical_width.min(clip.right().max(0) as u32)
        } else {
            self.px(logical_right).clamp(
                0,
                i64::from(self.physical_width.min(clip.right().max(0) as u32)),
            ) as u32
        };
        let bottom = if logical_bottom == i64::from(self.height) {
            self.physical_height.min(clip.bottom().max(0) as u32)
        } else {
            self.px(logical_bottom).clamp(
                0,
                i64::from(self.physical_height.min(clip.bottom().max(0) as u32)),
            ) as u32
        };
        if left >= right || top >= bottom {
            return;
        }
        // Seed one scanline and double-copy it, then reuse it for every row.
        // In particular, a maximized 4K app must not rebuild eight million
        // identical four-byte background pixels in a Rust loop on every hover.
        let first = (top * self.physical_width + left) as usize * 4;
        let bytes = (right - left) as usize * 4;
        self.pixels[first..first + 4].copy_from_slice(&color);
        let mut filled = 4;
        while filled < bytes {
            let count = filled.min(bytes - filled);
            self.pixels
                .copy_within(first..first + count, first + filled);
            filled += count;
        }
        for y in top + 1..bottom {
            let at = (y * self.physical_width + left) as usize * 4;
            self.pixels.copy_within(first..first + bytes, at);
        }
    }
    pub fn text(&mut self, x: i32, y: i32, text: &str, style: TextStyle, color: [u8; 4]) {
        let color = self.palette.foreground(color);
        let y = y - self.scroll_y;
        let mut target = Target::new(
            self.pixels,
            self.physical_width,
            self.physical_height,
            self.physical_width as usize * 4,
        )
        .expect("owned surface");
        let bounds = door_graphics_text::Clip::new(
            self.clip.x(),
            self.clip.y(),
            self.clip.width(),
            self.clip.height(),
        );
        let _ = self.fonts.draw(
            &mut target,
            bounds,
            TextRun {
                text,
                x,
                top: y,
                style,
                color,
            },
        );
    }
    /// Preserve the full glyph run, replacing selected pixels with a clean
    /// accent backdrop before painting white coverage once through its clip.
    pub fn selected_text(
        &mut self,
        x: i32,
        y: i32,
        text: &str,
        style: TextStyle,
        color: [u8; 4],
        selection: Option<door_graphics_core::Rect>,
    ) {
        self.text(x, y, text, style, color);
        if let Some(selection) = selection {
            let previous = self.clip;
            let projected = door_graphics_core::Rect::new(
                selection.x(),
                selection.y() - self.scroll_y,
                selection.width(),
                selection.height(),
            )
            .ok()
            .and_then(|r| r.intersection(previous));
            if let Some(projected) = projected {
                self.rect(
                    selection.x(),
                    selection.y(),
                    selection.width(),
                    selection.height(),
                    [196, 103, 0, 255],
                );
                self.clip = projected;
                self.text(x, y, text, style, [255; 4]);
                self.clip = previous;
            }
        }
    }
    /// Single-line editor: clip the full original run, including its kerning,
    /// to the actual edit row. Selection is repainted once on a clean backdrop.
    pub fn selected_text_clipped(
        &mut self,
        origin: (i32, i32),
        bounds: door_graphics_core::Rect,
        text: &str,
        style: TextStyle,
        color: [u8; 4],
        selection: Option<door_graphics_core::Rect>,
    ) {
        let previous = self.clip;
        let Some(projected) = door_graphics_core::Rect::new(
            bounds.x(),
            bounds.y() - self.scroll_y,
            bounds.width(),
            bounds.height(),
        )
        .ok()
        .and_then(|r| r.intersection(previous)) else {
            return;
        };
        self.clip = projected;
        self.selected_text(origin.0, origin.1, text, style, color, selection);
        self.clip = previous;
    }
    /// Shared native single-line metrics, selection, horizontal reveal and caret.
    pub fn line_editor(
        &mut self,
        x: i32,
        y: i32,
        width: u32,
        edit: &door_workspace::line_edit::LineEdit,
        caret_visible: bool,
    ) {
        let text = edit.text();
        let cursor = edit.cursor();
        let mut start = 0;
        while start < cursor && self.measure(&display_ascii(&text[start..cursor])) + 3 > width {
            start += text[start..]
                .chars()
                .next()
                .expect("cursor boundary")
                .len_utf8();
        }
        let selection = edit.selection();
        let a = selection.start.max(start);
        let b = selection.end.max(start);
        let left = self.measure(&display_ascii(&text[start..a])).min(width);
        let right = self.measure(&display_ascii(&text[start..b])).min(width);
        // The field's text box is one body line box (20 px at 100% text).
        let line = self.fonts.line_height(TextStyle::Regular14);
        let text_top = self.centered_text_top(y, line, TextStyle::Regular14);
        let Some(bounds) = door_graphics_core::Rect::new(x, y, width, line).ok() else {
            return;
        };
        self.selected_text_clipped(
            (x, text_top),
            bounds,
            &display_ascii(&text[start..]),
            TextStyle::Regular14,
            [24, 24, 24, 255],
            (right > left).then(|| {
                door_graphics_core::Rect::new(x + left as i32, y, right - left, line)
                    .expect("nonempty selection")
            }),
        );
        if caret_visible && right <= left {
            let caret = self
                .measure(&display_ascii(&text[start..cursor]))
                .min(width.saturating_sub(1));
            let ink = if self.palette.dark {
                [170, 170, 170, 255]
            } else {
                [24, 24, 24, 255]
            };
            self.text_caret(x + caret as i32, text_top, TextStyle::Regular14, ink);
        }
    }
    pub fn label(&mut self, x: i32, y: i32, text: &str) {
        self.text(x, y, text, TextStyle::Regular14, tokens::TEXT.value);
    }
    /// Line-box top that centres the style's capital band (the painted rows
    /// of 'H') in the row `y..y + height`, as Segoe UI text is optically
    /// centred in Windows controls. Logical precision, so every label in a
    /// row shares one baseline; the error is at most half a logical pixel.
    pub fn centered_text_top(&self, y: i32, height: u32, style: TextStyle) -> i32 {
        let Ok(ink) = self.fonts.label_ink("", style) else {
            return self.fonts.centered_top(style, y, height);
        };
        // Twice the physical pen top that centres the cap band in the
        // snapped physical row; pick the logical top whose snapped edge is
        // nearest (ties to the larger one, as the integer rule did).
        let row = (
            self.px(i64::from(y)),
            self.px(i64::from(y) + i64::from(height)),
        );
        let twice = row.0 + row.1 - (ink.cap_top + ink.cap_bottom);
        let p = i64::from(self.zoom.percent());
        let floor = (twice * 100).div_euclid(2 * p);
        let error = |l: i64| (2 * self.px(l) - twice).abs();
        let best = (floor - 1..=floor + 2)
            .min_by_key(|&l| (error(l), core::cmp::Reverse(l)))
            .unwrap_or(floor);
        best.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    }
    /// Document line placement (Notes body): the font's cap-plus-descender
    /// band centred in the line row, so editor rows keep their line metrics.
    /// Boxed labels and single-line fields use `centered_text_top`.
    pub fn line_text_top(&self, y: i32, height: u32, style: TextStyle) -> i32 {
        self.fonts.centered_top(style, y, height)
    }
    /// A label centred in the box `(x, y, w, h)`: horizontally on its painted
    /// ink at physical-pixel precision, vertically on the style's cap height
    /// (`centered_text_top`). Clipped to the box width; text wider than the
    /// box starts at its left edge. Every boxed control label uses this.
    pub fn text_in_box(
        &mut self,
        r: (i32, i32, u32, u32),
        text: &str,
        style: TextStyle,
        color: [u8; 4],
    ) {
        let (x, y, w, h) = r;
        if w == 0 || h == 0 || text.is_empty() {
            return;
        }
        let Ok(ink) = self.fonts.label_ink(text, style) else {
            return;
        };
        let left = self.px(i64::from(x));
        let span = self.px(i64::from(x) + i64::from(w)) - left;
        let pen = if ink.right - ink.left > span {
            left - ink.left
        } else {
            (2 * left + span - (ink.left + ink.right) + 1).div_euclid(2)
        };
        let top = self.centered_text_top(y, h, style) - self.scroll_y;
        let Some(clip) = door_graphics_core::Rect::new(x, top, w, self.fonts.line_height(style))
            .ok()
            .and_then(|r| r.intersection(self.clip))
        else {
            return;
        };
        let color = self.palette.foreground(color);
        let Some(physical) = self.physical_rect(clip.x(), clip.y(), clip.width(), clip.height())
        else {
            return;
        };
        let pen_top = self
            .px(i64::from(top))
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        if let Ok(mut target) = Target::new(
            self.pixels,
            self.physical_width,
            self.physical_height,
            self.physical_width as usize * 4,
        ) {
            let _ = self.fonts.draw_physical(
                &mut target,
                door_graphics_text::Clip::new(
                    physical.x(),
                    physical.y(),
                    physical.width(),
                    physical.height(),
                ),
                TextRun {
                    text,
                    x: pen.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
                    top: pen_top,
                    style,
                    color,
                },
            );
        }
    }
    /// Centres a control's content group in the span `x..x + w`: an optional
    /// leading icon `(width, gap after it)`, the label and an optional
    /// trailing glyph `(gap before it, width)`; zero widths omit a part.
    /// Returns the icon, label and trailing glyph x. A group wider than
    /// `w - 2 * pad` starts at `x + pad`. Command bar buttons, icon-only
    /// buttons and chips share this rather than per-site offsets.
    #[allow(clippy::too_many_arguments)]
    pub fn content_group(
        &self,
        x: i32,
        w: u32,
        lead: (u32, u32),
        text: &str,
        style: TextStyle,
        trail: (u32, u32),
        pad: u32,
    ) -> (i32, i32, i32) {
        let label = if text.is_empty() {
            0
        } else {
            self.measure_style(text, style)
        };
        let parts = (lead.0 > 0) as u32 + (label > 0) as u32 + (trail.1 > 0) as u32;
        let lead_w = lead.0 + if lead.0 > 0 && parts > 1 { lead.1 } else { 0 };
        let trail_w = if trail.1 > 0 {
            trail.1 + if label > 0 { trail.0 } else { 0 }
        } else {
            0
        };
        let group = lead_w + label + trail_w;
        let start = if group + 2 * pad <= w {
            x + (w - group) as i32 / 2
        } else {
            x + pad as i32
        };
        let text_x = start + lead_w as i32;
        (
            start,
            text_x,
            text_x + label as i32 + (trail_w - trail.1) as i32,
        )
    }
    /// Painted ink width of a label in logical pixels (rounded up): the
    /// content width a control sizes its padding around.
    pub fn ink_width(&self, text: &str, style: TextStyle) -> u32 {
        self.fonts.label_ink(text, style).map_or(0, |ink| {
            ((ink.right - ink.left).max(0) as u64 * 100).div_ceil(u64::from(self.zoom.percent()))
                as u32
        })
    }
    pub fn text_centered(
        &mut self,
        x: i32,
        y: i32,
        height: u32,
        text: &str,
        style: TextStyle,
        color: [u8; 4],
    ) {
        let top = self.centered_text_top(y, height, style);
        self.text(x, top, text, style, color);
    }
    pub fn text_clip_centered(
        &mut self,
        x: i32,
        row: (i32, u32),
        width: u32,
        text: &str,
        style: TextStyle,
        color: [u8; 4],
    ) {
        let top = self.centered_text_top(row.0, row.1, style);
        self.text_clip(x, top, width, text, style, color);
    }
    pub fn text_caret(&mut self, x: i32, text_top: i32, style: TextStyle, color: [u8; 4]) {
        let caret = self.fonts.caret_metrics(style);
        self.rect(x, text_top + caret.top, 1, caret.height, color);
    }
    pub fn rounded(&mut self, x: i32, y: i32, w: u32, h: u32, r: u32, color: [u8; 4]) {
        self.rounded_impl(x, y, w, h, r, self.surface_color(color));
    }
    /// A 1 px (logical) antialiased outline along the inside edge of the
    /// rounded rect, blended, leaving its interior untouched: the border of a
    /// translucent fill (controls and cards over Mica) without stacking.
    pub fn rounded_ring(&mut self, x: i32, y: i32, w: u32, h: u32, r: u32, color: [u8; 4]) {
        let color = self.surface_color(color);
        let clip = self.physical_clip();
        let radius = self.zoom.len(r);
        let line = self.zoom.len(1).max(1);
        if let Some(rect) = self.physical_rect(x, y - self.scroll_y, w, h)
            && let Ok(mut canvas) = door_graphics_core::Canvas::new(
                self.pixels,
                self.physical_width,
                self.physical_height,
                self.physical_width as usize * 4,
            )
        {
            canvas.set_clip(clip);
            let _ = canvas.stroke_rounded_rect(rect, radius, line, color);
        }
    }
    /// Shared finite Gaussian coverage; no blur allocation or filtering while painting.
    pub fn elevation(&mut self, x: i32, y: i32, w: u32, h: u32, opacity: u8) {
        let clip = self.physical_clip();
        if let Some(rect) = self.physical_rect(x, y - self.scroll_y, w, h)
            && let Ok(mut canvas) = door_graphics_core::Canvas::new(
                self.pixels,
                self.physical_width,
                self.physical_height,
                self.physical_width as usize * 4,
            )
        {
            canvas.set_clip(clip);
            let _ = canvas.soft_shadow(rect, self.zoom, opacity);
        }
    }
    /// Literal asset colors describe their own appearance, including a preview
    /// of a theme other than the current one. Keep geometry/AA/clip shared.
    pub fn rounded_asset(&mut self, x: i32, y: i32, w: u32, h: u32, r: u32, color: [u8; 4]) {
        self.rounded_impl(x, y, w, h, r, color);
    }
    pub fn small(&mut self, x: i32, y: i32, text: &str, color: [u8; 4]) {
        self.text(x, y, text, TextStyle::Regular12, color);
    }
    pub fn small_clip(&mut self, x: i32, y: i32, width: u32, text: &str, color: [u8; 4]) {
        self.text_clip(x, y, width, text, TextStyle::Regular12, color);
    }
    pub fn text_clip(
        &mut self,
        x: i32,
        y: i32,
        width: u32,
        text: &str,
        style: TextStyle,
        color: [u8; 4],
    ) {
        let color = self.palette.foreground(color);
        let y = y - self.scroll_y;
        if width == 0 {
            return;
        }
        let Some(clip) = door_graphics_core::Rect::new(x, y, width, self.fonts.line_height(style))
            .ok()
            .and_then(|r| r.intersection(self.clip))
        else {
            return;
        };
        if let Ok(mut target) = Target::new(
            self.pixels,
            self.physical_width,
            self.physical_height,
            self.physical_width as usize * 4,
        ) {
            let _ = self.fonts.draw(
                &mut target,
                door_graphics_text::Clip::new(clip.x(), clip.y(), clip.width(), clip.height()),
                TextRun {
                    text,
                    x,
                    top: y,
                    style,
                    color,
                },
            );
        }
    }
    pub fn large_icon(&mut self, x: i32, y: i32, icon: Icon) {
        let icon = icon.themed(self.palette.dark);
        let Some(pixels) = icon.color_scaled(40, self.scale) else {
            self.icon(x, y, icon, tokens::TEXT.value);
            return;
        };
        self.color_pixels(x, y, 40, pixels);
    }
    /// `size` is logical; the export must contain exactly(size*scale)^2 pixels.
    pub fn color_pixels(&mut self, x: i32, y: i32, size: usize, pixels: &[u8]) {
        self.color_image(x, y, size, size, pixels);
    }
    /// Native-density premultiplied BGRA, including rectangular original artwork.
    pub fn color_image(&mut self, x: i32, y: i32, width: usize, height: usize, pixels: &[u8]) {
        // Native frames hold (width x scale)^2 pixels; at a fractional zoom
        // they are area-filtered once to the physical size and cached.
        let (sw, sh) = (width * self.scale as usize, height * self.scale as usize);
        let (tw, th) = (
            self.zoom.len(width as u32) as usize,
            self.zoom.len(height as u32) as usize,
        );
        if pixels.starts_with(b"qoif") {
            let decode = || -> Option<alloc::vec::Vec<u8>> {
                let header = qoi::decode_header(pixels).ok()?;
                if header.width as usize != sw
                    || header.height as usize != sh
                    || header.width > 600
                    || header.height > 600
                {
                    return None;
                }
                let (_, rgba) = qoi::decode_to_vec(pixels).ok()?;
                if rgba.len() != sw * sh * 4 {
                    return None;
                }
                let bgra: alloc::vec::Vec<u8> = rgba
                    .chunks_exact(4)
                    .flat_map(|p| [p[2], p[1], p[0], p[3]])
                    .collect();
                Some(if (sw, sh) == (tw, th) {
                    bgra
                } else {
                    door_graphics_core::area_resample(&bgra, 4, (sw, sh), (tw, th))
                })
            };
            let Some(cache) = self.images else {
                // Standalone views still paint assets correctly. Ordinary native
                // windows always provide the persistent decoded-image cache.
                if let Some(bgra) = decode() {
                    self.blit_bgra(x, y, tw, th, &bgra);
                }
                return;
            };
            let mut cache = cache.0.borrow_mut();
            let key = (derived_key(pixels, 1, tw), true);
            if !cache.iter().any(|(id, _)| *id == key) {
                let Some(bgra) = decode() else {
                    return;
                };
                ImageCache::remember(&mut cache, key, bgra);
            }
            let data = &cache.iter().find(|(id, _)| *id == key).unwrap().1;
            self.blit_bgra(x, y, tw, th, data);
            return;
        }
        if tw == 0 || th == 0 {
            return;
        }
        if pixels.len() == tw * th * 4 {
            self.blit_bgra(x, y, tw, th, pixels);
        } else if pixels.len() == sw * sh * 4 {
            let resampled = door_graphics_core::area_resample(pixels, 4, (sw, sh), (tw, th));
            self.blit_bgra(x, y, tw, th, &resampled);
        }
    }
    /// Physical pixels covered by the logical span `x..x + w` (snapped
    /// edges; at fractional scales it depends on `x`).
    pub fn physical_span(&self, x: i32, w: u32) -> u32 {
        (self.px(i64::from(x) + i64::from(w)) - self.px(i64::from(x))).max(0) as u32
    }
    /// Content rendered at physical resolution (graphs) for the logical
    /// cell at (`x`, `y`): `pw` x `ph` = `physical_span` of its width and height.
    pub fn color_image_physical(&mut self, x: i32, y: i32, pw: u32, ph: u32, pixels: &[u8]) {
        self.blit_bgra(x, y, pw as usize, ph as usize, pixels);
    }
    /// Premultiplied BGRA `width` x `height` drawn at `num`/`den` physical
    /// pixels per source pixel with its top-left at physical (`ox`, `oy`),
    /// clipped to the logical `clip` rectangle: nearest-neighbour sampling,
    /// as an image editor's zoom shows pixels.
    #[allow(clippy::too_many_arguments)]
    pub fn image_scaled(
        &mut self,
        clip: (i32, i32, u32, u32),
        ox: i64,
        oy: i64,
        num: u32,
        den: u32,
        width: usize,
        height: usize,
        pixels: &[u8],
    ) {
        if width == 0 || height == 0 || num == 0 || den == 0 || pixels.len() != width * height * 4 {
            return;
        }
        let Some(view) = self.physical_rect(clip.0, clip.1 - self.scroll_y, clip.2, clip.3) else {
            return;
        };
        let bounds = self.physical_clip();
        let dw = (width as i64 * num as i64 / den as i64).max(1);
        let dh = (height as i64 * num as i64 / den as i64).max(1);
        let left = (view.x() as i64).max(bounds.x() as i64).max(ox).max(0);
        let top = (view.y() as i64).max(bounds.y() as i64).max(oy).max(0);
        let right = (view.x() as i64 + view.width() as i64)
            .min(bounds.x() as i64 + bounds.width() as i64)
            .min(ox + dw)
            .min(self.physical_width as i64);
        let bottom = (view.y() as i64 + view.height() as i64)
            .min(bounds.y() as i64 + bounds.height() as i64)
            .min(oy + dh)
            .min(self.physical_height as i64);
        if right <= left || bottom <= top {
            return;
        }
        let stride = self.physical_width as usize * 4;
        if num < den {
            // Reduced: each output pixel averages its source box (an area
            // filter, as Photos and Snipping Tool show a fitted picture);
            // at most 8 x 8 samples per pixel keep large reductions cheap.
            let span = |p: i64| -> (usize, usize) {
                let a = (((p) * den as i64) / num as i64).max(0) as usize;
                let b = ((((p + 1) * den as i64) / num as i64) as usize).max(a + 1);
                (a.min(width - 1), b.min(width))
            };
            let spany = |p: i64| -> (usize, usize) {
                let a = (((p) * den as i64) / num as i64).max(0) as usize;
                let b = ((((p + 1) * den as i64) / num as i64) as usize).max(a + 1);
                (a.min(height - 1), b.min(height))
            };
            let columns: alloc::vec::Vec<(usize, usize)> =
                (left..right).map(|px| span(px - ox)).collect();
            for py in top..bottom {
                let (y0, y1) = spany(py - oy);
                let ystep = ((y1 - y0) / 8).max(1);
                let out = &mut self.pixels[py as usize * stride..][..stride];
                for (i, &(x0, x1)) in columns.iter().enumerate() {
                    let xstep = ((x1 - x0) / 8).max(1);
                    let mut acc = [0u32; 4];
                    let mut n = 0u32;
                    let mut sy = y0;
                    while sy < y1 {
                        let row = &pixels[sy * width * 4..(sy + 1) * width * 4];
                        let mut sx = x0;
                        while sx < x1 {
                            for c in 0..4 {
                                acc[c] += u32::from(row[sx * 4 + c]);
                            }
                            n += 1;
                            sx += xstep;
                        }
                        sy += ystep;
                    }
                    let s = [
                        (acc[0] / n) as u8,
                        (acc[1] / n) as u8,
                        (acc[2] / n) as u8,
                        (acc[3] / n) as u8,
                    ];
                    let at = (left as usize + i) * 4;
                    if s[3] == 255 {
                        out[at..at + 4].copy_from_slice(&s);
                    } else if s[3] != 0 {
                        let ia = 255 - u32::from(s[3]);
                        for c in 0..4 {
                            out[at + c] = (u32::from(s[c])
                                + (u32::from(out[at + c]) * ia + 127) / 255)
                                .min(255) as u8;
                        }
                    }
                }
            }
            return;
        }
        let columns: alloc::vec::Vec<usize> = (left..right)
            .map(|px| (((px - ox) * den as i64 / num as i64) as usize).min(width - 1))
            .collect();
        for py in top..bottom {
            let sy = (((py - oy) * den as i64 / num as i64) as usize).min(height - 1);
            let row = &pixels[sy * width * 4..(sy + 1) * width * 4];
            let out = &mut self.pixels[py as usize * stride..][..stride];
            for (i, &sx) in columns.iter().enumerate() {
                let s = &row[sx * 4..sx * 4 + 4];
                let at = (left as usize + i) * 4;
                if s[3] == 255 {
                    out[at..at + 4].copy_from_slice(s);
                } else if s[3] != 0 {
                    let ia = 255 - u32::from(s[3]);
                    for c in 0..4 {
                        out[at + c] = (u32::from(s[c]) + (u32::from(out[at + c]) * ia + 127) / 255)
                            .min(255) as u8;
                    }
                }
            }
        }
    }
    /// Premultiplied BGRA of exactly `w` x `h` physical pixels at logical
    /// (`x`, `y`) (snapped origin), source-over through the clip.
    fn blit_bgra(&mut self, x: i32, y: i32, w: usize, h: usize, pixels: &[u8]) {
        if w == 0 || pixels.len() != w * h * 4 {
            return;
        }
        let clip = self.physical_clip();
        let x = self.px(i64::from(x)) as i32;
        let y = self.px(i64::from(y - self.scroll_y)) as i32;
        for (i, p) in pixels.chunks_exact(4).enumerate() {
            let (px, py) = (x + (i % w) as i32, y + (i / w) as i32);
            if !clip.contains(px, py)
                || px < 0
                || py < 0
                || px >= self.physical_width as i32
                || py >= self.physical_height as i32
            {
                continue;
            }
            let at = (py as u32 * self.physical_width + px as u32) as usize * 4;
            for (c, &v) in p.iter().enumerate() {
                self.pixels[at + c] = (u32::from(v)
                    + (u32::from(self.pixels[at + c]) * u32::from(255 - p[3]) + 127) / 255)
                    .min(255) as u8;
            }
        }
    }
    /// Fit a premultiplied image; lossless authored assets decode once into the bounded cache.
    pub fn image_fit(
        &mut self,
        bounds: (i32, i32, u32, u32),
        width: usize,
        height: usize,
        pixels: &[u8],
    ) {
        if pixels.starts_with(b"qoif") {
            let Some(cache) = self.images else {
                return;
            };
            let mut cache = cache.0.borrow_mut();
            let key = (pixels.as_ptr() as usize, true);
            if !cache.iter().any(|(id, _)| *id == key) {
                let Ok(header) = qoi::decode_header(pixels) else {
                    return;
                };
                if header.width != width as u32
                    || header.height != height as u32
                    || width > 600
                    || height > 600
                {
                    return;
                }
                let Ok((_, rgba)) = qoi::decode_to_vec(pixels) else {
                    return;
                };
                if rgba.len() != width * height * 4 {
                    return;
                }
                let bgra = rgba
                    .chunks_exact(4)
                    .flat_map(|p| [p[2], p[1], p[0], p[3]])
                    .collect();
                ImageCache::remember(&mut cache, key, bgra);
            }
            let data = &cache.iter().find(|(id, _)| *id == key).unwrap().1;
            self.image_fit(bounds, width, height, data);
            return;
        }
        let (x, y, w, h) = bounds;
        if width == 0 || height == 0 || w == 0 || h == 0 || pixels.len() != width * height * 4 {
            return;
        }
        let (dw, dh) = if u64::from(w) * height as u64 <= u64::from(h) * width as u64 {
            (
                w,
                (u64::from(w) * height as u64 / width as u64).max(1) as u32,
            )
        } else {
            (
                (u64::from(h) * width as u64 / height as u64).max(1) as u32,
                h,
            )
        };
        let ox = self.px(i64::from(x + (w - dw) as i32 / 2)) as i32;
        let oy = self.px(i64::from(y + (h - dh) as i32 / 2 - self.scroll_y)) as i32;
        let (dw, dh) = (self.zoom.len(dw).max(1), self.zoom.len(dh).max(1));
        let clip = self.physical_clip();
        let start_x = ox.max(clip.x()).max(0);
        let start_y = oy.max(clip.y()).max(0);
        let end_x = (ox + dw as i32)
            .min(clip.x() + clip.width() as i32)
            .min(self.physical_width as i32);
        let end_y = (oy + dh as i32)
            .min(clip.y() + clip.height() as i32)
            .min(self.physical_height as i32);
        for py in start_y..end_y {
            let sy = (py - oy) as usize * height / dh as usize;
            for px in start_x..end_x {
                let sx = (px - ox) as usize * width / dw as usize;
                let p = &pixels[(sy * width + sx) * 4..][..4];
                let at = (py as usize * self.physical_width as usize + px as usize) * 4;
                for (c, value) in p.iter().enumerate() {
                    self.pixels[at + c] = (u32::from(*value)
                        + (u32::from(self.pixels[at + c]) * u32::from(255 - p[3]) + 127) / 255)
                        .min(255) as u8;
                }
            }
        }
    }
    /// Exact percentage zoom centered on the viewport, clipped rather than fitted
    /// back down. Physical sampling preserves the native density of image detail.
    pub fn image_zoom(
        &mut self,
        bounds: (i32, i32, u32, u32),
        width: usize,
        height: usize,
        pixels: &[u8],
        percent: u32,
    ) {
        let (x, y, w, h) = bounds;
        if width == 0
            || height == 0
            || w == 0
            || h == 0
            || pixels.len() != width * height * 4
            || !(25..=400).contains(&percent)
        {
            return;
        }
        let z = u64::from(self.zoom.percent());
        let dw = (width as u64 * percent as u64 * z / 10_000).max(1) as i32;
        let dh = (height as u64 * percent as u64 * z / 10_000).max(1) as i32;
        let Some(view) = self.physical_rect(x, y - self.scroll_y, w, h) else {
            return;
        };
        let (vx, vy) = (view.x(), view.y());
        let (vw, vh) = (view.width() as i32, view.height() as i32);
        let ox = vx + (vw - dw) / 2;
        let oy = vy + (vh - dh) / 2;
        let clip = self.physical_clip();
        let left = vx.max(ox).max(clip.x()).max(0);
        let top = vy.max(oy).max(clip.y()).max(0);
        let right = (vx + vw)
            .min(ox + dw)
            .min(clip.x() + clip.width() as i32)
            .min(self.physical_width as i32);
        let bottom = (vy + vh)
            .min(oy + dh)
            .min(clip.y() + clip.height() as i32)
            .min(self.physical_height as i32);
        for py in top..bottom {
            let sy = (py - oy) as usize * height / dh as usize;
            for px in left..right {
                let sx = (px - ox) as usize * width / dw as usize;
                let source = &pixels[(sy * width + sx) * 4..][..4];
                let at = (py as usize * self.physical_width as usize + px as usize) * 4;
                for c in 0..4 {
                    self.pixels[at + c] = (u32::from(source[c])
                        + (u32::from(self.pixels[at + c]) * u32::from(255 - source[3]) + 127) / 255)
                        .min(255) as u8;
                }
            }
        }
    }
    /// Color shell icon at any logical size: the smallest exported frame at
    /// least as large as the physical target is downscaled (never enlarged
    /// unless no larger frame exists), so 16/32/48 px land on exact frames.
    pub fn sized_icon(&mut self, x: i32, y: i32, size: u32, icon: Icon) {
        const FRAMES: [(u32, u32); 10] = [
            (16, 1),
            (20, 1),
            (16, 2),
            (40, 1),
            (16, 3),
            (20, 3),
            (80, 1),
            (40, 3),
            (80, 2),
            (80, 3),
        ];
        let icon = icon.themed(self.palette.dark);
        let target = self.zoom.len(size);
        let frame = |&(s, f): &(u32, u32)| (s * f, icon.color_scaled(s, f));
        let chosen = FRAMES
            .iter()
            .map(frame)
            .filter(|(_, p)| p.is_some())
            .find(|(px, _)| *px >= target)
            .or_else(|| FRAMES.iter().rev().map(frame).find(|(_, p)| p.is_some()));
        match chosen {
            Some((px, Some(pixels))) => {
                self.image_fit((x, y, size, size), px as usize, px as usize, pixels)
            }
            _ => self.large_icon(x + (size as i32 - 40) / 2, y + (size as i32 - 40) / 2, icon),
        }
    }
    pub fn hero_icon(&mut self, x: i32, y: i32, size: u32, icon: Icon) {
        let icon = icon.themed(self.palette.dark);
        if let Some(pixels) = icon.color_scaled(80, 3) {
            let source = 240usize;
            self.image_fit((x, y, size, size), source, source, pixels);
        } else if let Some(pixels) = icon.color_scaled(40, 3) {
            self.image_fit((x, y, size, size), 120, 120, pixels);
        } else {
            self.large_icon(x + (size as i32 - 40) / 2, y + (size as i32 - 40) / 2, icon);
        }
    }
    fn rounded_impl(&mut self, x: i32, y: i32, w: u32, h: u32, r: u32, color: [u8; 4]) {
        let clip = self.physical_clip();
        let radius = self.zoom.len(r);
        if let Some(rect) = self.physical_rect(x, y - self.scroll_y, w, h)
            && let Ok(mut canvas) = door_graphics_core::Canvas::new(
                self.pixels,
                self.physical_width,
                self.physical_height,
                self.physical_width as usize * 4,
            )
        {
            canvas.set_clip(clip);
            let _ = canvas.fill_rounded_rect(rect, radius, color);
        }
    }
    pub fn bitmap_bgr_rounded(&mut self, x: i32, y: i32, width: usize, pixels: &[u8], radius: u32) {
        if pixels.starts_with(b"qoif") {
            let Some(images) = self.images else {
                return;
            };
            let mut images = images.0.borrow_mut();
            let target = self.zoom.len(width as u32);
            // User pointers stay below bit 47, so the target width keeps one
            // source decoded at two densities apart.
            let key = (pixels.as_ptr() as usize ^ ((target as usize) << 48), false);
            if !images.iter().any(|(id, _)| *id == key) {
                let Ok(header) = qoi::decode_header(pixels) else {
                    return;
                };
                if header.width == 0 || header.height > 600 || target > 2 * header.width {
                    return;
                }
                let Ok((_, rgba)) = qoi::decode_to_vec(pixels) else {
                    return;
                };
                let bgr: alloc::vec::Vec<u8> = rgba
                    .chunks_exact(4)
                    .flat_map(|p| [p[2], p[1], p[0]])
                    .collect();
                let bgr = if header.width == target {
                    bgr
                } else {
                    resample_bgr(&bgr, header.width, header.height, target)
                };
                ImageCache::remember(&mut images, key, bgr);
            }
            let data = &images.iter().find(|(id, _)| *id == key).unwrap().1;
            self.bitmap_bgr_rounded(x, y, width, data, radius);
            return;
        }
        let width = self.zoom.len(width as u32) as usize;
        if width == 0 || !pixels.len().is_multiple_of(width * 3) {
            return;
        }
        let clip = self.physical_clip();
        if let Ok(rect) = door_graphics_core::Rect::new(
            self.px(i64::from(x)) as i32,
            self.px(i64::from(y - self.scroll_y)) as i32,
            width as u32,
            (pixels.len() / 3 / width) as u32,
        ) && let Ok(mut canvas) = door_graphics_core::Canvas::new(
            self.pixels,
            self.physical_width,
            self.physical_height,
            self.physical_width as usize * 4,
        ) {
            canvas.set_clip(clip);
            let _ = canvas.blit_rounded_bgr(rect, self.zoom.len(radius), pixels);
        }
    }
    /// A QOI picture cover-cropped (centred) to `width` x `height` logical
    /// pixels and area-averaged to the display density: wallpaper thumbnails
    /// made at run time from one larger preview.
    pub fn bitmap_cover_rounded(
        &mut self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        qoi: &[u8],
        radius: u32,
    ) {
        let Some(images) = self.images else {
            return;
        };
        let (tw, th) = (self.zoom.len(width), self.zoom.len(height));
        if tw == 0 || th == 0 {
            return;
        }
        let key = (derived_key(qoi, 3, tw as usize), true);
        let mut cache = images.0.borrow_mut();
        if !cache.iter().any(|(id, _)| *id == key) {
            let Ok(header) = qoi::decode_header(qoi) else {
                return;
            };
            if header.width > 1024
                || header.height > 1024
                || tw > header.width
                || th > header.height
            {
                return;
            }
            let Ok((_, rgba)) = qoi::decode_to_vec(qoi) else {
                return;
            };
            let channels = header.channels.as_u8() as usize;
            let bgr = cover_bgr(&rgba, channels, header.width, header.height, tw, th);
            ImageCache::remember(&mut cache, key, bgr);
        }
        let data = &cache.iter().find(|(id, _)| *id == key).unwrap().1;
        let clip = self.physical_clip();
        if let Ok(rect) = door_graphics_core::Rect::new(
            self.px(i64::from(x)) as i32,
            self.px(i64::from(y - self.scroll_y)) as i32,
            tw,
            th,
        ) && let Ok(mut canvas) = door_graphics_core::Canvas::new(
            self.pixels,
            self.physical_width,
            self.physical_height,
            self.physical_width as usize * 4,
        ) {
            canvas.set_clip(clip);
            let _ = canvas.blit_rounded_bgr(rect, self.zoom.len(radius), data);
        }
    }
    pub fn compact_icon(&mut self, x: i32, y: i32, icon: Icon, color: [u8; 4]) {
        let icon = icon.themed(self.palette.dark);
        if let Some(pixels) = icon.color_scaled(16, self.scale) {
            self.color_pixels(x, y, 16, pixels);
            return;
        }
        self.mask_pixels(x, y, 16, icon.mask_scaled(16, self.scale), color, None);
    }
    /// Vertically mirrored 12 px glyph: the ascending sort caret is the
    /// mirrored descending chevron, so no second glyph asset is needed.
    pub fn tiny_icon_flipped(&mut self, x: i32, y: i32, icon: Icon, color: [u8; 4]) {
        let mask = icon.mask_scaled(12, self.scale);
        let side = 12 * self.scale as usize;
        let mut flipped = alloc::vec::Vec::with_capacity(mask.len());
        for row in mask.chunks_exact(side).rev() {
            flipped.extend_from_slice(row);
        }
        self.mask_pixels(x, y, 12, &flipped, color, None);
    }
    pub fn tiny_icon(&mut self, x: i32, y: i32, icon: Icon, color: [u8; 4]) {
        self.mask_pixels(x, y, 12, icon.mask_scaled(12, self.scale), color, None);
    }
    pub fn mask_pixels(
        &mut self,
        x: i32,
        y: i32,
        size: usize,
        pixels: &[u8],
        color: [u8; 4],
        left_color: Option<[u8; 4]>,
    ) {
        // Native masks hold (size x scale)^2 coverage; a fractional zoom
        // area-filters them once to the physical size (cached when possible).
        let source = size * self.scale as usize;
        let size = self.zoom.len(size as u32) as usize;
        if size == 0 {
            return;
        }
        let resampled;
        let pixels = if pixels.len() == size * size {
            pixels
        } else if source > 0 && pixels.len() == source * source {
            let key = (derived_key(pixels, 2, size), true);
            match self.images {
                Some(cache) => {
                    let mut cache = cache.0.borrow_mut();
                    if !cache.iter().any(|(id, _)| *id == key) {
                        let mask = door_graphics_core::area_resample(
                            pixels,
                            1,
                            (source, source),
                            (size, size),
                        );
                        ImageCache::remember(&mut cache, key, mask);
                    }
                    resampled = cache.iter().find(|(id, _)| *id == key).unwrap().1.clone();
                }
                None => {
                    resampled = door_graphics_core::area_resample(
                        pixels,
                        1,
                        (source, source),
                        (size, size),
                    );
                }
            }
            &resampled[..]
        } else {
            return;
        };
        let color = self.palette.foreground(color);
        let left_color = left_color.map(|color| self.palette.foreground(color));
        let clip = self.physical_clip();
        let x = self.px(i64::from(x)) as i32;
        let y = self.px(i64::from(y - self.scroll_y)) as i32;
        for (i, &alpha) in pixels.iter().enumerate() {
            let (px, py) = (x + (i % size) as i32, y + (i / size) as i32);
            if !clip.contains(px, py)
                || px < 0
                || py < 0
                || px >= self.physical_width as i32
                || py >= self.physical_height as i32
            {
                continue;
            }
            let color = if i % size < size / 2 {
                left_color.unwrap_or(color)
            } else {
                color
            };
            let at = (py as u32 * self.physical_width + px as u32) as usize * 4;
            for (c, &fg) in color.iter().enumerate() {
                self.pixels[at + c] = ((u32::from(fg) * u32::from(alpha)
                    + u32::from(self.pixels[at + c]) * u32::from(255 - alpha)
                    + 127)
                    / 255) as u8;
            }
        }
    }
    pub fn icon(&mut self, x: i32, y: i32, icon: Icon, color: [u8; 4]) {
        if let Some(pixels) = icon.color_scaled(20, self.scale) {
            self.color_pixels(x, y, 20, pixels);
            return;
        }
        self.mask_pixels(x, y, 20, icon.mask_scaled(20, self.scale), color, None);
    }
    pub fn button(&mut self, x: i32, y: i32, w: u32, label: &str, icon: Icon, accent: bool) {
        let height = command_height();
        let radius = tokens::CONTROL_RADIUS.value;
        let contains = |p: Option<(i32, i32)>| {
            p.is_some_and(|(px, py)| {
                px >= x && px < x + w as i32 && py >= y && py < y + height as i32
            })
        };
        let mut bg = if accent {
            tokens::ACCENT.value
        } else {
            [247, 244, 240, 255]
        };
        if contains(self.hover) {
            let shade = if contains(self.pressed) { 18 } else { 8 };
            for channel in &mut bg[..3] {
                *channel = channel.saturating_sub(shade);
            }
        }
        self.rounded(x, y, w, height, radius, bg);
        self.rounded(
            x + 1,
            y + 1,
            w.saturating_sub(2),
            height - 2,
            radius - 1,
            bg,
        );
        let fg = if accent {
            [255, 255, 255, 255]
        } else {
            tokens::TEXT.value
        };
        // Icon and label form one content group centred in the space left of
        // the optional dropdown chevron, as command bar buttons size to it.
        let chevron = matches!(icon, Icon::Add | Icon::Sort | Icon::View);
        let room = w.saturating_sub(if chevron { 24 } else { 0 });
        let group = 16 + 10 + self.measure_small(label);
        let ix = x + (room.saturating_sub(group) / 2).max(8) as i32;
        if matches!(icon, Icon::Sort) {
            self.mask_pixels(
                ix,
                y + (height as i32 - 16) / 2,
                16,
                icon.mask_scaled(16, self.scale),
                fg,
                Some([197, 121, 58, 255]),
            );
        } else {
            self.compact_icon(ix, y + (height as i32 - 16) / 2, icon, fg);
        }
        self.text_clip_centered(
            ix + 26,
            (y, height),
            (x + room as i32 - ix - 26).max(0) as u32,
            label,
            TextStyle::Regular12,
            fg,
        );
        if chevron {
            self.tiny_icon(
                x + w as i32 - 16,
                y + (height as i32 - 12) / 2,
                Icon::Chevron,
                [128, 128, 128, 255],
            );
        }
    }
    /// An anti-aliased stroke between logical points, `width` logical wide
    /// (glyphs drawn as lines, such as the copy dialog's cancel cross).
    pub fn stroke(&mut self, from: (f32, f32), to: (f32, f32), width: f32, color: [u8; 4]) {
        let color = self.palette.foreground(color);
        let s = self.zoom.percent() as f32 / 100.0;
        let scroll = self.scroll_y as f32;
        let (ax, ay) = (from.0 * s, (from.1 - scroll) * s);
        let (bx, by) = (to.0 * s, (to.1 - scroll) * s);
        let half = (width * s / 2.0).max(0.5);
        let clip = self.physical_clip();
        let left = (ax.min(bx) - half - 1.0).max(clip.x() as f32).max(0.0) as i32;
        let top = (ay.min(by) - half - 1.0).max(clip.y() as f32).max(0.0) as i32;
        let right = ((ax.max(bx) + half + 2.0) as i32)
            .min(clip.right())
            .min(self.physical_width as i32);
        let bottom = ((ay.max(by) + half + 2.0) as i32)
            .min(clip.bottom())
            .min(self.physical_height as i32);
        let (dx, dy) = (bx - ax, by - ay);
        let len2 = (dx * dx + dy * dy).max(1e-6);
        for py in top..bottom {
            for px in left..right {
                let (cx, cy) = (px as f32 + 0.5, py as f32 + 0.5);
                let t = (((cx - ax) * dx + (cy - ay) * dy) / len2).clamp(0.0, 1.0);
                let (ex, ey) = (cx - (ax + t * dx), cy - (ay + t * dy));
                // Distance to the segment, linearised about the edge (no
                // square root in the guest): exact where coverage is partial.
                let d = half + (ex * ex + ey * ey - half * half) / (2.0 * half);
                let cover = (half + 0.5 - d).clamp(0.0, 1.0);
                if cover <= 0.0 {
                    continue;
                }
                let a = (cover * f32::from(color[3])) as u32;
                let at = (py as u32 * self.physical_width + px as u32) as usize * 4;
                for (c, &fg) in color.iter().enumerate() {
                    self.pixels[at + c] =
                        ((u32::from(fg) * a + u32::from(self.pixels[at + c]) * (255 - a) + 127)
                            / 255) as u8;
                }
            }
        }
    }
    /// The physical pixels under logical (`x`, `y`, `w`, `h`), to paint over
    /// and put back (`restore`) or replay animated (`composite`).
    pub fn snapshot(&self, x: i32, y: i32, w: u32, h: u32) -> Option<Snapshot> {
        let r = self.physical_rect(x, y - self.scroll_y, w, h)?;
        let left = r.x().max(0);
        let top = r.y().max(0);
        let right = (r.x() + r.width() as i32).min(self.physical_width as i32);
        let bottom = (r.y() + r.height() as i32).min(self.physical_height as i32);
        if right <= left || bottom <= top {
            return None;
        }
        let (width, height) = ((right - left) as usize, (bottom - top) as usize);
        let stride = self.physical_width as usize * 4;
        let mut pixels = alloc::vec::Vec::with_capacity(width * height * 4);
        for row in top as usize..bottom as usize {
            let at = row * stride + left as usize * 4;
            pixels.extend_from_slice(&self.pixels[at..at + width * 4]);
        }
        Some(Snapshot {
            x: left,
            y: top,
            width,
            height,
            pixels,
        })
    }
    /// Puts a snapshot's pixels back where they were taken.
    pub fn restore(&mut self, s: &Snapshot) {
        let stride = self.physical_width as usize * 4;
        for (row, line) in s.pixels.chunks_exact(s.width * 4).enumerate() {
            let at = (s.y as usize + row) * stride + s.x as usize * 4;
            self.pixels[at..at + line.len()].copy_from_slice(line);
        }
    }
    /// Composites an opaque snapshot where it was taken, scaled about its
    /// centre by `scale` / 1000 and faded to `alpha`, with corners rounded
    /// by `radius` (logical): a window opening or closing.
    pub fn composite(&mut self, s: &Snapshot, alpha: u8, scale: u32, radius: u32) {
        if alpha == 0 || scale == 0 || s.width == 0 || s.height == 0 {
            return;
        }
        let k = scale as f32 / 1000.0;
        let (w, h) = (s.width as f32, s.height as f32);
        let (dw, dh) = (w * k, h * k);
        let left = s.x as f32 + (w - dw) / 2.0;
        let top = s.y as f32 + (h - dh) / 2.0;
        let r = self.zoom.len(radius) as f32 * k;
        let clip = self.physical_clip();
        let x0 = (left.max(clip.x() as f32).max(0.0)) as i32;
        let y0 = (top.max(clip.y() as f32).max(0.0)) as i32;
        let x1 = ((left + dw + 1.0) as i32)
            .min(clip.right())
            .min(self.physical_width as i32);
        let y1 = ((top + dh + 1.0) as i32)
            .min(clip.bottom())
            .min(self.physical_height as i32);
        let sample = |x: usize, y: usize, c: usize| f32::from(s.pixels[(y * s.width + x) * 4 + c]);
        for py in y0..y1 {
            for px in x0..x1 {
                // Destination pixel centre in the scaled rectangle.
                let (qx, qy) = (px as f32 + 0.5 - left, py as f32 + 0.5 - top);
                if qx < 0.0 || qy < 0.0 || qx > dw || qy > dh {
                    continue;
                }
                let mut cover = (qx.min(dw - qx) + 0.5).min(qy.min(dh - qy) + 0.5).min(1.0);
                let (ex, ey) = ((r - qx).max(qx - (dw - r)), (r - qy).max(qy - (dh - r)));
                if r > 0.5 && ex > 0.0 && ey > 0.0 {
                    let d = r + (ex * ex + ey * ey - r * r) / (2.0 * r);
                    cover = cover.min((r + 0.5 - d).clamp(0.0, 1.0));
                }
                if cover <= 0.0 {
                    continue;
                }
                // Bilinear sample of the source.
                let fx = (qx / k - 0.5).clamp(0.0, w - 1.0);
                let fy = (qy / k - 0.5).clamp(0.0, h - 1.0);
                let (ix, iy) = (fx as usize, fy as usize);
                let (jx, jy) = ((ix + 1).min(s.width - 1), (iy + 1).min(s.height - 1));
                let (tx, ty) = (fx - ix as f32, fy - iy as f32);
                let a = (cover * f32::from(alpha)) as u32;
                let at = (py as u32 * self.physical_width + px as u32) as usize * 4;
                for c in 0..4 {
                    let top_row = sample(ix, iy, c) * (1.0 - tx) + sample(jx, iy, c) * tx;
                    let bottom_row = sample(ix, jy, c) * (1.0 - tx) + sample(jx, jy, c) * tx;
                    let v = (top_row * (1.0 - ty) + bottom_row * ty) as u32;
                    self.pixels[at + c] =
                        ((v * a + u32::from(self.pixels[at + c]) * (255 - a) + 127) / 255) as u8;
                }
            }
        }
    }
    pub fn line(&mut self, y: i32) {
        self.rect(0, y, self.width, 1, [230, 230, 230, 255]);
    }
}
/// Bilinear BGR resample to `target` width, height kept proportional.
/// Sampling at pixel centers makes an exact 2:1 reduction a 2x2 average.
/// The centred `tw`:`th` crop of a `width` x `height` picture (`channels`
/// bytes per RGB(A) pixel), area-averaged to `tw` x `th` BGR: each target
/// pixel averages the source rectangle it covers, with fractional edge
/// weights in 1/256 px.
fn cover_bgr(
    src: &[u8],
    channels: usize,
    width: u32,
    height: u32,
    tw: u32,
    th: u32,
) -> alloc::vec::Vec<u8> {
    // Crop in 1/256 source px: the largest centred rectangle of the target aspect.
    let (w, h) = (u64::from(width) * 256, u64::from(height) * 256);
    let (cw, ch) = if w * u64::from(th) > h * u64::from(tw) {
        (h * u64::from(tw) / u64::from(th), h)
    } else {
        (w, w * u64::from(th) / u64::from(tw))
    };
    let (ox, oy) = ((w - cw) / 2, (h - ch) / 2);
    let span = |t: u32, n: u32, c: u64, o: u64| -> (u64, u64) {
        (
            o + c * u64::from(t) / u64::from(n),
            o + c * u64::from(t + 1) / u64::from(n),
        )
    };
    let mut out = alloc::vec::Vec::with_capacity(tw as usize * th as usize * 3);
    for ty in 0..th {
        let (y0, y1) = span(ty, th, ch, oy);
        for tx in 0..tw {
            let (x0, x1) = span(tx, tw, cw, ox);
            let mut sum = [0u64; 3];
            let mut total = 0u64;
            let mut sy = y0 & !255;
            while sy < y1 {
                let wy = (sy + 256).min(y1) - sy.max(y0);
                let row = (sy / 256).min(u64::from(height) - 1) as usize;
                let mut sx = x0 & !255;
                while sx < x1 {
                    let wx = (sx + 256).min(x1) - sx.max(x0);
                    let column = (sx / 256).min(u64::from(width) - 1) as usize;
                    let at = (row * width as usize + column) * channels;
                    let weight = wx * wy;
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s += u64::from(src[at + c]) * weight;
                    }
                    total += weight;
                    sx += 256;
                }
                sy += 256;
            }
            let total = total.max(1);
            for c in [2, 1, 0] {
                out.push(((sum[c] + total / 2) / total) as u8);
            }
        }
    }
    out
}
fn resample_bgr(src: &[u8], width: u32, height: u32, target: u32) -> alloc::vec::Vec<u8> {
    let rows = ((u64::from(height) * u64::from(target) + u64::from(width) / 2) / u64::from(width))
        .max(1) as u32;
    // Source coordinate of a target pixel center in 1/256 px.
    let map = |t: u32, from: u32, to: u32| -> (usize, usize, u32) {
        let s =
            ((2 * u64::from(t) + 1) * u64::from(from) * 128 / u64::from(to)).saturating_sub(128);
        let i = (s >> 8) as u32;
        let i0 = i.min(from - 1);
        let i1 = (i + 1).min(from - 1);
        (i0 as usize, i1 as usize, (s & 255) as u32)
    };
    let stride = width as usize * 3;
    let mut out = alloc::vec::Vec::with_capacity(target as usize * rows as usize * 3);
    for ty in 0..rows {
        let (y0, y1, fy) = map(ty, height, rows);
        for tx in 0..target {
            let (x0, x1, fx) = map(tx, width, target);
            for c in 0..3 {
                let p = |y: usize, x: usize| u32::from(src[y * stride + x * 3 + c]);
                let v = p(y0, x0) * (256 - fx) * (256 - fy)
                    + p(y0, x1) * fx * (256 - fy)
                    + p(y1, x0) * (256 - fx) * fy
                    + p(y1, x1) * fx * fy;
                out.push(((v + 32768) >> 16) as u8);
            }
        }
    }
    out
}
/// Display form of untrusted text: every script, symbol and emoji is kept
/// (non-ASCII text renders through the attached Unicode engine); only control
/// characters other than newline and tab become '?'. The name is historical:
/// this once replaced all non-ASCII text.
pub fn display_ascii(text: &str) -> alloc::string::String {
    text.chars()
        .map(|c| {
            if c == '\n' || c == '\t' || !c.is_control() {
                c
            } else {
                '?'
            }
        })
        .collect()
}

impl View<'_> {
    pub fn popup(&mut self, menu: &door_workspace::menu::Menu, items: &[&str]) {
        let Some((x, y)) = menu.origin() else { return };
        let (offset, rows) = menu.viewport();
        let row = door_workspace::menu::Menu::row_height();
        let height = rows as u32 * row + 8 + menu.header();
        self.rounded(x + 2, y + 3, 224, height, 8, [210, 210, 210, 255]);
        self.rounded(x, y, 224, height, 8, [252, 252, 252, 255]);
        for (i, text) in items.iter().enumerate().skip(offset).take(rows) {
            let top = y + 4 + menu.header() as i32 + (i - offset) as i32 * row as i32;
            if i == menu.selected() && menu.enabled(i) {
                self.rounded(x + 4, top, 216, row, 4, [237, 237, 237, 255]);
            }
            let icon = if text.starts_with("Save") {
                Some(Icon::Save)
            } else if text.starts_with("Reopen") {
                Some(Icon::Open)
            } else if text.starts_with("New") {
                Some(Icon::Add)
            } else if text == &"Folder" {
                Some(Icon::Folder)
            } else if text.starts_with("Large icons") {
                Some(Icon::LargeIcons)
            } else if text.starts_with("Details") {
                Some(Icon::Details)
            } else if text == &"Cut" {
                Some(Icon::Cut)
            } else if text == &"Copy" {
                Some(Icon::Copy)
            } else if text == &"Paste" {
                Some(Icon::Paste)
            } else if text == &"Rename" {
                Some(Icon::Rename)
            } else if text == &"Share" {
                Some(Icon::Share)
            } else if text == &"Delete" {
                Some(Icon::Delete)
            } else if text == &"Properties" {
                Some(Icon::Info)
            } else if text.starts_with("Refresh") {
                Some(Icon::Refresh)
            } else if text.starts_with("Up") {
                Some(Icon::Up)
            } else if text.starts_with("Shut") {
                Some(Icon::Power)
            } else {
                None
            };
            if let Some(icon) = icon {
                self.icon(
                    x + 12,
                    top + (row as i32 - 20) / 2,
                    icon,
                    if menu.enabled(i) {
                        [32, 32, 32, 255]
                    } else {
                        [160, 160, 160, 255]
                    },
                );
            }
            let (label, shortcut) = text
                .split_once("  ")
                .map_or((*text, ""), |(a, b)| (a, b.trim()));
            self.text_centered(
                x + 40,
                top,
                row,
                label,
                TextStyle::Regular12,
                if menu.enabled(i) {
                    tokens::TEXT.value
                } else {
                    [155, 155, 155, 255]
                },
            );
            let advance = self
                .fonts
                .measure(shortcut, TextStyle::Regular12)
                .map_or(0, |m| m.advance_width);
            self.text_centered(
                x + 212 - advance as i32,
                top,
                row,
                shortcut,
                TextStyle::Regular12,
                [110, 110, 110, 255],
            );
        }
    }
}

#[cfg(test)]
mod explorer_cache_tests {
    use super::*;
    #[test]
    fn wallpaper_resample_halves_by_average_and_scales_up_proportionally() {
        // 4x2 source: columns 0,100,200,250 (all channels), two equal rows.
        let row = [0u8, 100, 200, 250]
            .iter()
            .flat_map(|&v| [v; 3])
            .collect::<alloc::vec::Vec<_>>();
        let src = [row.clone(), row].concat();
        let half = resample_bgr(&src, 4, 2, 2);
        assert_eq!(half, alloc::vec![50, 50, 50, 225, 225, 225]);
        let up = resample_bgr(&src, 4, 2, 6);
        assert_eq!(up.len(), 6 * 3 * 3);
        assert_eq!(&up[..3], &[0, 0, 0]);
        assert_eq!(&up[15..18], &[250, 250, 250]);
        assert!(
            up.chunks_exact(3)
                .take(5)
                .zip(up.chunks_exact(3).skip(1))
                .all(|(a, b)| a[0] <= b[0])
        );
    }
    #[test]
    fn mixed_small_and_hero_icons_do_not_evict_their_own_next_frame() {
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let cache = ImageCache::default();
        let mut p = alloc::vec![0;320*320*4];
        let icons = [
            Icon::Folder,
            Icon::Desktop,
            Icon::Documents,
            Icon::Pictures,
            Icon::Pdf,
            Icon::Word,
            Icon::Sheet,
            Icon::ImageFile,
            Icon::VideoFile,
            Icon::CodeFile,
            Icon::ArchiveFile,
            Icon::FolderBack,
            Icon::FolderFront,
            Icon::ModelFile,
        ];
        let draw = |pixels: &mut [u8]| {
            let mut v = View::new_scaled(pixels, 320, 320, &fonts, 1).with_images(&cache);
            for i in icons {
                v.hero_icon(0, 0, 208, i);
                v.compact_icon(0, 0, i, [0, 0, 0, 255]);
                v.icon(0, 0, i, [0, 0, 0, 255]);
            }
        };
        draw(&mut p);
        let identities = || {
            cache
                .0
                .borrow()
                .iter()
                .map(|(k, p)| (*k, p.as_ptr() as usize))
                .collect::<alloc::vec::Vec<_>>()
        };
        let before = identities();
        assert!(before.len() > 32);
        draw(&mut p);
        assert_eq!(identities(), before);
        assert!(cache.0.borrow().iter().map(|(_, p)| p.len()).sum::<usize>() <= 8 * 1024 * 1024);
        let mut records = alloc::vec::Vec::new();
        for i in 0..110 {
            ImageCache::remember(&mut records, (i, false), alloc::vec![0;128*1024]);
        }
        assert!(
            records.len() <= 96
                && records.iter().map(|(_, p)| p.len()).sum::<usize>() <= 8 * 1024 * 1024
        );
    }
    #[test]
    fn mixed_windows_hero_icons_stay_decoded_across_unchanged_frames() {
        let fonts = FontAtlas::windows_reference_scaled(1).unwrap();
        let cache = ImageCache::default();
        let mut pixels = alloc::vec![0;320*320*4];
        let icons = [
            Icon::Folder,
            Icon::Desktop,
            Icon::Documents,
            Icon::Pictures,
            Icon::Pdf,
            Icon::Word,
            Icon::Sheet,
            Icon::ImageFile,
            Icon::VideoFile,
            Icon::CodeFile,
            Icon::ArchiveFile,
            Icon::FolderBack,
            Icon::FolderFront,
            Icon::ModelFile,
        ];
        {
            let mut v = View::new_scaled(&mut pixels, 320, 320, &fonts, 1).with_images(&cache);
            for icon in icons {
                v.hero_icon(0, 0, 208, icon);
            }
        }
        let before: alloc::vec::Vec<_> = cache
            .0
            .borrow()
            .iter()
            .map(|(key, bytes)| (*key, bytes.as_ptr() as usize))
            .collect();
        assert_eq!(before.len(), icons.len());
        {
            let mut v = View::new_scaled(&mut pixels, 320, 320, &fonts, 1).with_images(&cache);
            for icon in icons {
                v.hero_icon(0, 0, 208, icon);
            }
        }
        let after: alloc::vec::Vec<_> = cache
            .0
            .borrow()
            .iter()
            .map(|(key, bytes)| (*key, bytes.as_ptr() as usize))
            .collect();
        assert_eq!(before, after);
    }
}
#[cfg(test)]
mod fractional_tests {
    use super::*;
    use door_graphics_core::Scale;
    fn atlas(percent: u32) -> FontAtlas {
        let key = alloc::format!("fonts/segoe-ui/hinted-{percent}.dfa");
        FontAtlas::from_packed_percent(door_workspace::assets::get(&key), percent).unwrap()
    }
    #[test]
    fn fractional_rects_abut_without_seams_and_one_pixel_lines_stay_crisp() {
        for percent in [125, 150, 175] {
            let zoom = Scale::from_percent(percent).unwrap();
            let fonts = atlas(percent);
            let (w, h) = (zoom.len(120), zoom.len(40));
            let mut pixels = alloc::vec![0; (w * h * 4) as usize];
            let mut v = View::new_zoomed(&mut pixels, w, h, &fonts, zoom);
            assert_eq!((v.width, v.height, v.scale), (120, 40, 2));
            let (a, b, line) = ([200, 40, 40, 255], [40, 200, 40, 255], [20, 20, 20, 255]);
            v.rect(0, 0, 120, 40, [255; 4]);
            // Abutting one-logical-pixel columns tile the band exactly.
            for x in 0..120 {
                v.rect(x, 0, 1, 10, if x % 2 == 0 { a } else { b });
            }
            v.rect(0, 20, 120, 1, line);
            let px = |x: u32, y: u32| {
                let at = ((y * w + x) * 4) as usize;
                <[u8; 4]>::try_from(&pixels[at..at + 4]).unwrap()
            };
            let mut widths = [0u32; 3];
            let mut run = 1;
            for x in 1..w {
                for y in 0..zoom.len(10) {
                    assert!(
                        px(x, y) == a || px(x, y) == b,
                        "{percent}%: seam at {x},{y}"
                    );
                }
                if px(x, 0) == px(x - 1, 0) {
                    run += 1;
                } else {
                    widths[run as usize] += 1;
                    run = 1;
                }
            }
            assert!(widths[1] > 0 && widths[2] > 0, "{percent}%: {widths:?}");
            let rows: alloc::vec::Vec<u32> = (0..h).filter(|y| px(5, *y) == line).collect();
            assert!(
                matches!(rows.len(), 1 | 2),
                "{percent}%: line rows {rows:?}"
            );
            assert_eq!(rows[0], zoom.px(20) as u32);
            for x in 0..w {
                for y in zoom.len(10)..h {
                    assert!(
                        px(x, y) == line || px(x, y) == [255; 4],
                        "{percent}%: blend at {x},{y}"
                    );
                }
            }
            assert_eq!(
                door_workspace::raster_check::blurred_lines(&pixels, w as usize, h as usize),
                0
            );
        }
    }
    #[test]
    fn fractional_text_and_icons_use_the_percent_atlas_and_physical_frames() {
        let zoom = Scale::from_percent(150).unwrap();
        let fonts = atlas(150);
        let base = FontAtlas::windows_reference_scaled(1).unwrap();
        assert_eq!(fonts.percent(), 150);
        let (w, h) = (zoom.len(200), zoom.len(60));
        let mut pixels = alloc::vec![255; (w * h * 4) as usize];
        let mut v = View::new_zoomed(&mut pixels, w, h, &fonts, zoom);
        v.text(10, 8, "HHH", TextStyle::Regular14, [0, 0, 0, 255]);
        v.mask_pixels(
            100,
            20,
            16,
            Icon::Search.mask_scaled(16, v.scale),
            [0, 0, 0, 255],
            None,
        );
        let mut expected = alloc::vec![255; (w * h * 4) as usize];
        fonts
            .draw(
                &mut Target::new(&mut expected, w, h, (w * 4) as usize).unwrap(),
                door_graphics_text::Clip::new(0, 0, 200, 60),
                TextRun {
                    text: "HHH",
                    x: 10,
                    top: 8,
                    style: TextStyle::Regular14,
                    color: [0, 0, 0, 255],
                },
            )
            .unwrap();
        let band = |p: &[u8], x0: u32, x1: u32| {
            let rows: alloc::vec::Vec<u32> = (0..h)
                .filter(|y| (x0..x1).any(|x| p[((y * w + x) * 4) as usize] < 128))
                .collect();
            (rows[0], *rows.last().unwrap() + 1)
        };
        // Text is exactly the 150% atlas run at the snapped origin.
        let text_columns = zoom.len(90);
        for y in 0..h {
            let at = (y * w * 4) as usize;
            assert_eq!(
                pixels[at..at + (text_columns * 4) as usize],
                expected[at..at + (text_columns * 4) as usize],
                "row {y}"
            );
        }
        let cap = |a: &FontAtlas| {
            let ink = a.label_ink("H", TextStyle::Regular14).unwrap();
            ink.cap_bottom - ink.cap_top
        };
        assert!(
            (cap(&fonts) * 2 - cap(&base) * 3).abs() <= 2,
            "{} vs {}",
            cap(&fonts),
            cap(&base)
        );
        // The 16 px mask lands in its 24 px physical cell.
        let (top, bottom) = band(&pixels, zoom.px(100) as u32, zoom.px(116) as u32);
        assert!(
            top >= zoom.px(20) as u32 && bottom <= zoom.px(36) as u32,
            "{top}..{bottom}"
        );
    }
}

/// Explorer/Notepad-style document tab. Caption authority and close hit bounds
/// remain with the caller; this painter never draws over trusted system buttons.
impl View<'_> {
    #[allow(clippy::too_many_arguments)]
    pub fn document_tab(
        &mut self,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        icon: Icon,
        title: &str,
        selected: bool,
    ) {
        if width < 64 || height < 24 {
            return;
        }
        let hovered = self.hover.is_some_and(|(px, py)| {
            px >= x && px < x + width as i32 && py >= y && py < y + height as i32
        });
        let bg = if selected {
            if self.palette.dark {
                [35, 31, 27, 255]
            } else {
                [252, 251, 249, 255]
            }
        } else if self.palette.dark {
            [48, 43, 38, 255]
        } else {
            [247, 242, 237, 255]
        };
        if selected || hovered {
            self.rounded(x, y, width, height, 8, bg);
            if selected {
                self.rect(x, y + height as i32 / 2, width, height - height / 2, bg);
            }
        } else {
            self.rect(
                x + width as i32,
                y + 8,
                1,
                height.saturating_sub(16),
                if self.palette.dark {
                    [61, 56, 50, 255]
                } else {
                    [223, 218, 213, 255]
                },
            );
        }
        let fg = if self.palette.dark {
            [240, 240, 240, 255]
        } else {
            [24, 24, 24, 255]
        };
        self.compact_icon(x + 12, y + (height as i32 - 16) / 2, icon, fg);
        self.text_clip_centered(
            x + 36,
            (y, height),
            width.saturating_sub(70),
            &display_ascii(title),
            if selected {
                TextStyle::Semibold14
            } else {
                TextStyle::Regular14
            },
            fg,
        );
        let cx = x + width as i32 - 22;
        let cy = y + height as i32 / 2;
        if self.hover.is_some_and(|(px, py)| {
            (cx - 9..cx + 10).contains(&px) && (cy - 10..cy + 10).contains(&py)
        }) {
            self.rounded(
                cx - 10,
                cy - 10,
                20,
                20,
                4,
                if self.palette.dark {
                    [65, 59, 53, 255]
                } else {
                    [235, 231, 226, 255]
                },
            );
        }
        // A foreground glyph (as `caption_tabs`), not `rect`: surfaces are
        // theme-remapped, so a light rect "X" vanished on dark tabs.
        let mask = match self.scale {
            3 => door_workspace::asset!("../../../assets/icons/windows-shell/chrome-close.36.mask"),
            2 => door_workspace::asset!("../../../assets/icons/windows-shell/chrome-close.24.mask"),
            _ => door_workspace::asset!("../../../assets/icons/windows-shell/chrome-close.12.mask"),
        };
        self.mask_pixels(cx - 6, cy - 6, 12, mask, fg, None);
    }
}

impl View<'_> {
    /// Native tabbed windows resolve this premultiplied tint over the compositor's
    /// cached wallpaper frost. Host painters deliberately use the opaque fallback.
    pub fn material_rect(&mut self, x: i32, y: i32, w: u32, h: u32, color: [u8; 4], alpha: u8) {
        let mut c = color;
        if self.materials {
            for v in &mut c[..3] {
                *v = ((u16::from(*v) * u16::from(alpha) + 127) / 255) as u8;
            }
            c[3] = alpha;
        }
        self.rect(x, y, w, h, c);
    }
}

/// Faux styles for text painted from a face without bold or italic variants
/// (Notes' Formatted mode). Both work on the pixels already painted inside a
/// logical rectangle, measuring ink as the distance from `background`.
impl View<'_> {
    fn ink_rect(&self, x: i32, y: i32, w: u32, h: u32) -> Option<door_graphics_core::Rect> {
        self.physical_rect(x, y - self.scroll_y, w, h)?
            .intersection(self.physical_clip())
            .and_then(|r| {
                door_graphics_core::Rect::new(0, 0, self.physical_width, self.physical_height)
                    .ok()?
                    .intersection(r)
            })
    }
    fn ink(pixel: &[u8], background: [u8; 4]) -> u32 {
        (0..3)
            .map(|c| u32::from(pixel[c].abs_diff(background[c])))
            .sum()
    }
    /// Bold: each pixel takes the inkiest of itself and its left
    /// neighbours (one physical pixel per 1.5 logical px of density).
    pub fn embolden(&mut self, x: i32, y: i32, w: u32, h: u32, background: [u8; 4]) {
        let background = self.surface_color(background);
        let Some(r) = self.ink_rect(x, y, w, h) else {
            return;
        };
        let spread = ((self.px(2) / 3).max(1)) as usize;
        let stride = self.physical_width as usize * 4;
        for py in r.y() as usize..r.bottom() as usize {
            let row = py * stride;
            for px in (r.x() as usize + 1..r.right() as usize).rev() {
                let mut best = row + px * 4;
                let mut most = Self::ink(&self.pixels[best..best + 4], background);
                for k in 1..=spread.min(px - r.x() as usize) {
                    let at = row + (px - k) * 4;
                    let ink = Self::ink(&self.pixels[at..at + 4], background);
                    if ink > most {
                        (best, most) = (at, ink);
                    }
                }
                let here = row + px * 4;
                if best != here {
                    self.pixels.copy_within(best..best + 4, here);
                }
            }
        }
    }
    /// Italic: rows slant right by one fifth of their height above the
    /// rectangle's bottom; ink may lean `extend` logical px past its right
    /// edge (over the following text, ink only).
    pub fn shear(&mut self, x: i32, y: i32, w: u32, h: u32, extend: u32, background: [u8; 4]) {
        let background = self.surface_color(background);
        let Some(r) = self.ink_rect(x, y, w, h) else {
            return;
        };
        let limit = (self.px(i64::from(x) + i64::from(w) + i64::from(extend)) as usize)
            .min(self.physical_clip().right().max(0) as usize)
            .min(self.physical_width as usize);
        let stride = self.physical_width as usize * 4;
        let (left, right) = (r.x() as usize, r.right() as usize);
        let mut row_pixels = alloc::vec::Vec::with_capacity((right - left) * 4);
        for py in r.y() as usize..r.bottom() as usize {
            let shift = (r.bottom() as usize - 1 - py) / 5;
            if shift == 0 {
                continue;
            }
            let row = py * stride;
            row_pixels.clear();
            row_pixels.extend_from_slice(&self.pixels[row + left * 4..row + right * 4]);
            for px in left..right {
                self.pixels[row + px * 4..row + px * 4 + 4].copy_from_slice(&background);
            }
            for (i, pixel) in row_pixels.chunks_exact(4).enumerate() {
                let to = left + i + shift;
                if to < limit && Self::ink(pixel, background) > 0 {
                    self.pixels[row + to * 4..row + to * 4 + 4].copy_from_slice(pixel);
                }
            }
        }
    }
}
impl View<'_> {
    /// A solid run in a text colour (underlines, strikethrough): mapped
    /// like text (`Palette::foreground`), not like a surface.
    pub fn text_rule(&mut self, x: i32, y: i32, w: u32, h: u32, color: [u8; 4]) {
        let color = self.palette.foreground(color);
        let (palette, explorer) = (self.palette, self.explorer_style);
        self.palette = door_workspace::palette::Palette::light();
        self.explorer_style = false;
        self.rect(x, y, w, h, color);
        (self.palette, self.explorer_style) = (palette, explorer);
    }
}
impl View<'_> {
    /// Fill `bounds` with a premultiplied image, cropping the overflow
    /// evenly (a folder's thumbnail window), where `image_fit` letterboxes.
    pub fn image_cover(
        &mut self,
        bounds: (i32, i32, u32, u32),
        width: usize,
        height: usize,
        pixels: &[u8],
    ) {
        let (x, y, w, h) = bounds;
        if width == 0 || height == 0 || w == 0 || h == 0 {
            return;
        }
        let (cw, ch) = if u64::from(w) * height as u64 >= u64::from(h) * width as u64 {
            (
                w,
                (u64::from(w) * height as u64).div_ceil(width as u64) as u32,
            )
        } else {
            (
                (u64::from(h) * width as u64).div_ceil(height as u64) as u32,
                h,
            )
        };
        let previous = self.clip;
        let Some(clip) = door_graphics_core::Rect::new(x, y - self.scroll_y, w, h)
            .ok()
            .and_then(|r| r.intersection(previous))
        else {
            return;
        };
        self.clip = clip;
        self.image_fit(
            (x - (cw - w) as i32 / 2, y - (ch - h) as i32 / 2, cw, ch),
            width,
            height,
            pixels,
        );
        self.clip = previous;
    }
}
