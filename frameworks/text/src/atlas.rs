use crate::{Clip, Error, MAX_TEXT_BYTES, Result, Target};
use crate::{Family, LayoutOptions, TextFormat, TextSystem, Wrap};
use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use alloc::{boxed::Box, sync::Arc, vec::Vec};
use unicode_segmentation::UnicodeSegmentation;
pub mod packed;

const ASCII_START: u8 = 32;
const ASCII_COUNT: usize = 95;
pub const MAX_ATLAS_BYTES: usize = 160 * 1024;
pub const REGULAR_FONT: &[u8] = include_bytes!("../../../assets/fonts/selawik/selawk.ttf");
pub const SEMIBOLD_FONT: &[u8] = include_bytes!("../../../assets/fonts/selawik/selawksb.ttf");
pub const FONT_LICENSE: &str = include_str!("../../../assets/fonts/selawik/LICENSE.txt");
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextStyle {
    Regular14,
    Regular20,
    Semibold14,
    Regular12,
    Semibold28,
    Semibold20,
    Regular16,
    Semibold16,
}
/// Stable style order used by the checked build-time atlas format.
pub const TEXT_STYLES: [TextStyle; 8] = [
    TextStyle::Regular14,
    TextStyle::Regular20,
    TextStyle::Semibold14,
    TextStyle::Regular12,
    TextStyle::Semibold28,
    TextStyle::Semibold20,
    TextStyle::Regular16,
    TextStyle::Semibold16,
];
impl TextStyle {
    fn index(self) -> usize {
        match self {
            Self::Regular14 => 0,
            Self::Regular20 => 1,
            Self::Semibold14 => 2,
            Self::Regular12 => 3,
            Self::Semibold28 => 4,
            Self::Semibold20 => 5,
            Self::Regular16 => 6,
            Self::Semibold16 => 7,
        }
    }
    pub const fn pixel_size(self) -> u32 {
        match self {
            Self::Regular20 | Self::Semibold20 => 20,
            Self::Regular12 => 12,
            Self::Regular16 | Self::Semibold16 => 16,
            Self::Semibold28 => 28,
            _ => 14,
        }
    }
    pub const fn line_height(self) -> u32 {
        match self {
            Self::Regular20 | Self::Semibold20 => 28,
            Self::Regular12 => 18,
            Self::Regular16 | Self::Semibold16 => 24,
            Self::Semibold28 => 38,
            _ => 20,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontMetrics {
    pub pixel_size: u32,
    pub line_height: u32,
    pub baseline: u32,
    pub descent: i32,
}
/// Stable optical row reference: capital height plus descenders, in logical
/// pixels. Derived from the cached H/g glyphs rather than per-label text, so
/// neighboring controls retain a common baseline even without descenders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InkMetrics {
    pub top: i32,
    pub height: u32,
}
/// Physical-pixel ink of a single text line (see `FontAtlas::label_ink`).
/// `right <= left` means the text paints nothing (empty or spaces only).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LabelInk {
    pub left: i64,
    pub right: i64,
    pub cap_top: i64,
    pub cap_bottom: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextMetrics {
    pub advance_width: u32,
    pub height: u32,
    pub lines: u32,
    pub painted_pixels: usize,
}
#[derive(Clone, Copy, Debug)]
pub struct TextRun<'a> {
    pub text: &'a str,
    pub x: i32,
    pub top: i32,
    pub style: TextStyle,
    pub color: [u8; 4],
}
#[derive(Clone, Copy, Default)]
struct CachedGlyph {
    offset: usize,
    width: u16,
    height: u16,
    left: i16,
    top: i16,
    advance: i32,
}
struct CachedFont {
    kerning: Kerning,
    glyphs: [CachedGlyph; ASCII_COUNT],
    metrics: FontMetrics,
}
enum Kerning {
    Outline(Box<FontRef<'static>>, PxScale),
    Pairs(Vec<(u16, i32)>),
}
impl CachedFont {
    fn kern(&self, before: u8, after: u8) -> i32 {
        match &self.kerning {
            Kerning::Outline(font, scale) => fixed(
                font.as_scaled(*scale)
                    .kern(font.glyph_id(before as char), font.glyph_id(after as char)),
            ),
            Kerning::Pairs(pairs) => {
                let key = u16::from(before - ASCII_START) * ASCII_COUNT as u16
                    + u16::from(after - ASCII_START);
                pairs
                    .binary_search_by_key(&key, |&(key, _)| key)
                    .map_or(0, |index| pairs[index].1)
            }
        }
    }
}
/// Eight eager ASCII caches. All outline parsing/raster allocation happens in
/// `new`; drawing and measuring never rasterize or grow a cache.
pub struct FontAtlas {
    fonts: [CachedFont; 8],
    /// Display scale in percent (100 = 1x); see `scaled_len`. Positions,
    /// clips and logical metrics convert with it alone.
    percent: u32,
    /// Accessibility > Text size in percent (100..=225). Glyphs are cached at
    /// `percent x text` and the logical text metrics grow with it, while the
    /// display scale (and so every non-text edge) stays `percent`.
    text: u32,
    coverage: Vec<u8>,
    channels: usize,
    /// Optional Unicode engine for text outside the cached ASCII set.
    unicode: Option<(Arc<TextSystem>, Family)>,
}
/// Display scales with exported or rasterizable atlases (Windows scales).
pub const fn valid_percent(percent: u32) -> bool {
    matches!(percent, 100 | 125 | 150 | 175 | 200 | 225 | 250 | 300)
}
/// Logical -> physical with the compositor's rounding (`Scale::px`:
/// nearest, exact halves down), so text edges match window chrome edges.
fn scaled(percent: u32, logical: i64) -> i64 {
    (logical * i64::from(percent) + 49).div_euclid(100)
}
fn scaled_len(percent: u32, logical: u32) -> u32 {
    scaled(percent, i64::from(logical)) as u32
}
fn logical_floor(percent: u32, physical: i64) -> i64 {
    (physical * 100).div_euclid(i64::from(percent))
}
fn logical_ceil(percent: u32, physical: i64) -> i64 {
    (physical * 100 + i64::from(percent) - 1).div_euclid(i64::from(percent))
}
/// Glyph extent bound: 64 logical pixels.
const fn glyph_bound(percent: u32) -> u32 {
    (64 * percent).div_ceil(100)
}
/// Accessibility > Text size range (Windows: 100..=225%).
pub const TEXT_PERCENT_MIN: u32 = 100;
pub const TEXT_PERCENT_MAX: u32 = 225;
/// A logical text length at a text size: nearest, halves up.
pub const fn text_len(logical: u32, text_percent: u32) -> u32 {
    (logical * text_percent + 50) / 100
}
/// A text-holding layout length (row, button, menu item, tile) at a text
/// size: `base` grows by exactly the growth of `style`'s line box, so its
/// padding is kept and the larger text is never clipped. Icons and chrome
/// don't call this; they keep their size, as on Windows.
pub const fn text_grow(base: u32, style: TextStyle, text_percent: u32) -> u32 {
    base + text_len(style.line_height(), text_percent) - style.line_height()
}
/// Coverage bytes per channel allowed for one atlas at `percent`.
pub const fn atlas_capacity(percent: u32) -> usize {
    (MAX_ATLAS_BYTES * percent as usize * percent as usize).div_ceil(10_000)
}
fn fixed(value: f32) -> i32 {
    if value >= 0.0 {
        (value * 64.0 + 0.5) as i32
    } else {
        (value * 64.0 - 0.5) as i32
    }
}
impl FontAtlas {
    /// Decode a bounded immutable atlas produced by the offline host exporter.
    /// No font engine is invoked; all metadata is validated before publication.
    pub fn from_packed(bytes: &[u8], factor: u32) -> Result<Self> {
        Self::from_packed_percent(bytes, factor.saturating_mul(100))
    }
    /// Decode an exported atlas for a display scale in percent (100..=300):
    /// the runtime-loading path for fractional scales (125/150/175% atlases
    /// are `hinted-125.dfa`, `hinted-150.dfa`, `hinted-175.dfa`).
    pub fn from_packed_percent(bytes: &[u8], percent: u32) -> Result<Self> {
        if !valid_percent(percent) {
            return Err(Error::InvalidFont);
        }
        packed::decode(bytes, percent)
    }
    /// An exported atlas used for a text size: `bytes` must be the atlas for
    /// the effective density `display x text` (for example the 150% atlas
    /// for 100% display and 150% text, or the 3x atlas for 150% x 200%).
    /// Positions keep the display scale; text metrics grow by `text`.
    pub fn from_packed_text(bytes: &[u8], display: u32, text: u32) -> Result<Self> {
        let raster = Self::hinted_percent(display, text).ok_or(Error::InvalidFont)?;
        Self::from_packed_percent(bytes, raster)?.for_text_size(display, text)
    }
    /// Use this atlas for another display scale and text size with the same
    /// glyph density (for example the 2x atlas as 100% display x 200% text):
    /// identical glyphs, positions at `display`, text metrics grown by `text`.
    pub fn for_text_size(mut self, display: u32, text: u32) -> Result<Self> {
        match Self::raster_percent(display, text) {
            Some(raster) if raster == self.percent * self.text => {
                self.percent = display;
                self.text = text;
                Ok(self)
            }
            _ => Err(Error::InvalidFont),
        }
    }
    /// Effective glyph density (hundredths of a percent) of a display scale
    /// and text size, or `None` outside the Windows ranges.
    pub const fn raster_percent(display: u32, text: u32) -> Option<u32> {
        if valid_percent(display) && text >= TEXT_PERCENT_MIN && text <= TEXT_PERCENT_MAX {
            Some(display * text)
        } else {
            None
        }
    }
    /// The exported atlas density (percent) that serves `display x text`
    /// exactly, if the hinted set (1x, 125, 150, 175, 2x, 3x) has one.
    pub const fn hinted_percent(display: u32, text: u32) -> Option<u32> {
        match Self::raster_percent(display, text) {
            Some(raster) if raster % 100 == 0 => match raster / 100 {
                p @ (100 | 125 | 150 | 175 | 200 | 300) => Some(p),
                _ => None,
            },
            _ => None,
        }
    }
    /// Static outlines (`regular`, `semibold`; Segoe UI from the asset pack)
    /// rasterized at `display x text` for a text size with no hinted atlas.
    /// Same subpixel coverage format as the exported atlases.
    pub fn outline_text(
        regular: &'static [u8],
        semibold: &'static [u8],
        display: u32,
        text: u32,
    ) -> Result<Self> {
        Self::raster_percent(display, text).ok_or(Error::InvalidFont)?;
        Self::from_fonts_text(regular, semibold, true, display, text)
    }
    /// Host reference: Segoe UI at a display scale and text size, from the
    /// hinted atlas when one serves it exactly (1x/2x/3x), otherwise from
    /// the static outlines. Native processes use `from_packed_text` and
    /// `outline_text` over the asset pack instead (no embedded fonts).
    pub fn windows_reference_text(display: u32, text: u32) -> Result<Self> {
        if let Some(p) = Self::hinted_percent(display, text)
            && p.is_multiple_of(100)
        {
            return Self::windows_reference_scaled(p / 100)?.for_text_size(display, text);
        }
        Self::outline_text(
            include_bytes!("../../../assets/fonts/segoe-ui/segoeui.ttf"),
            include_bytes!("../../../assets/fonts/segoe-ui/seguisb.ttf"),
            display,
            text,
        )
    }
    pub fn new() -> Result<Self> {
        Self::from_fonts(REGULAR_FONT, SEMIBOLD_FONT, false, 100)
    }
    /// Pinned host-installed Segoe UI resources for the owner's local reference image.
    pub fn windows_reference() -> Result<Self> {
        Self::windows_reference_scaled(1)
    }
    /// Owner-installed Cascadia Mono, cached once for native terminal cells.
    pub fn monospace_reference_scaled(factor: u32) -> Result<Self> {
        Self::monospace_reference_percent(factor.saturating_mul(100))
    }
    /// Cascadia Mono rasterized at a display scale in percent.
    pub fn monospace_reference_percent(percent: u32) -> Result<Self> {
        if !valid_percent(percent) {
            return Err(Error::InvalidFont);
        }
        let font = include_bytes!("../../../assets/fonts/cascadia-mono/CascadiaMono.ttf");
        Self::from_fonts(font, font, true, percent)
    }
    /// Terminal/editor cells from caller-supplied monospace font bytes (the
    /// runtime application asset pack), cached exactly as
    /// `monospace_reference_scaled`, so applications need not embed the font.
    pub fn monospace_from(font: &'static [u8], factor: u32) -> Result<Self> {
        Self::monospace_from_percent(font, factor.saturating_mul(100))
    }
    /// `monospace_from` at a display scale in percent.
    pub fn monospace_from_percent(font: &'static [u8], percent: u32) -> Result<Self> {
        if !valid_percent(percent) {
            return Err(Error::InvalidFont);
        }
        Self::from_fonts(font, font, true, percent)
    }
    pub fn windows_reference_scaled(factor: u32) -> Result<Self> {
        let bytes: &[u8] = match factor {
            1 => include_bytes!("../../../assets/fonts/segoe-ui/hinted-1x.dfa"),
            2 => include_bytes!("../../../assets/fonts/segoe-ui/hinted-2x.dfa"),
            3 => include_bytes!("../../../assets/fonts/segoe-ui/hinted-3x.dfa"),
            _ => return Err(Error::InvalidFont),
        };
        Self::from_packed(bytes, factor)
    }
    /// Retained static-outline baseline for host comparison and regression.
    pub fn windows_outline_reference_scaled(factor: u32) -> Result<Self> {
        Self::windows_outline_reference_percent(factor.saturating_mul(100))
    }
    /// Static Segoe UI outlines rasterized at a display scale in percent.
    pub fn windows_outline_reference_percent(percent: u32) -> Result<Self> {
        if !valid_percent(percent) {
            return Err(Error::InvalidFont);
        }
        Self::from_fonts(
            include_bytes!("../../../assets/fonts/segoe-ui/segoeui.ttf"),
            include_bytes!("../../../assets/fonts/segoe-ui/seguisb.ttf"),
            true,
            percent,
        )
    }
    /// Display scale of this atlas in percent.
    pub fn percent(&self) -> u32 {
        self.percent
    }
    /// Accessibility > Text size this atlas applies, in percent.
    pub fn text_percent(&self) -> u32 {
        self.text
    }
    /// A logical text length at this atlas's text size (`text_len`).
    pub fn text_len(&self, logical: u32) -> u32 {
        text_len(logical, self.text)
    }
    /// Line box height of `style` in logical pixels at this text size.
    pub fn line_height(&self, style: TextStyle) -> u32 {
        text_len(style.line_height(), self.text)
    }
    /// A text-holding layout length grown for this text size (`text_grow`).
    pub fn text_grow(&self, base: u32, style: TextStyle) -> u32 {
        text_grow(base, style, self.text)
    }

    /// Route text outside ASCII through a Unicode `TextSystem` (shaping,
    /// fallback, bidi, colour emoji) at this atlas's scale and style metrics.
    /// ASCII-only text keeps the cached atlas path and its exact pixels.
    pub fn attach_unicode(&mut self, system: Arc<TextSystem>, family: Family) {
        self.unicode = Some((system, family));
    }
    pub fn with_unicode(mut self, system: Arc<TextSystem>, family: Family) -> Self {
        self.attach_unicode(system, family);
        self
    }
    pub fn unicode(&self) -> Option<&Arc<TextSystem>> {
        self.unicode.as_ref().map(|(system, _)| system)
    }
    /// The attached engine's layout for non-ASCII text (atlas style metrics).
    fn unicode_layout(
        &self,
        text: &str,
        style: TextStyle,
    ) -> Option<Result<(crate::TextLayout, &TextSystem)>> {
        let (system, family) = self.unicode.as_ref().filter(|_| !text.is_ascii())?;
        if text.len() > MAX_TEXT_BYTES {
            return Some(Err(Error::TextTooLong));
        }
        // Physical line advance = text-scaled logical line box x display.
        let mut format = TextFormat::from_style(style, self.percent as f32 / 100.0)
            .with_line_height(self.line_height(style) as f32)
            .with_family(*family);
        format.size = style.pixel_size() as f32 * self.text as f32 / 100.0;
        let options = LayoutOptions {
            wrap: Wrap::None,
            ..LayoutOptions::default()
        };
        Some(
            system
                .layout(text, &format, &options)
                .map(|layout| (layout, &**system)),
        )
    }
    fn from_fonts(
        regular: &'static [u8],
        semibold: &'static [u8],
        em: bool,
        percent: u32,
    ) -> Result<Self> {
        Self::from_fonts_text(regular, semibold, em, percent, 100)
    }
    /// Rasterize the eight styles at `display x text` (hundredths of a
    /// percent, so fractional products such as 337.5% stay exact).
    fn from_fonts_text(
        regular: &'static [u8],
        semibold: &'static [u8],
        em: bool,
        display: u32,
        text: u32,
    ) -> Result<Self> {
        let regular = FontRef::try_from_slice(regular).map_err(|_| Error::InvalidFont)?;
        let semibold = FontRef::try_from_slice(semibold).map_err(|_| Error::InvalidFont)?;
        let channels = if em { 3 } else { 1 };
        let percent = display * text;
        let mut coverage = Vec::with_capacity(atlas_capacity(percent.div_ceil(100)) * channels);
        let a = Self::build(
            regular.clone(),
            TextStyle::Regular14,
            &mut coverage,
            em,
            percent,
        )?;
        let b = Self::build(
            regular.clone(),
            TextStyle::Regular20,
            &mut coverage,
            em,
            percent,
        )?;
        let c = Self::build(
            semibold.clone(),
            TextStyle::Semibold14,
            &mut coverage,
            em,
            percent,
        )?;
        let d = Self::build(
            regular.clone(),
            TextStyle::Regular12,
            &mut coverage,
            em,
            percent,
        )?;
        let e = Self::build(
            semibold.clone(),
            TextStyle::Semibold28,
            &mut coverage,
            em,
            percent,
        )?;
        let f = Self::build(
            semibold.clone(),
            TextStyle::Semibold20,
            &mut coverage,
            em,
            percent,
        )?;
        let g = Self::build(regular, TextStyle::Regular16, &mut coverage, em, percent)?;
        let h = Self::build(semibold, TextStyle::Semibold16, &mut coverage, em, percent)?;
        Ok(Self {
            fonts: [a, b, c, d, e, f, g, h],
            coverage,
            percent: display,
            text,
            channels,
            unicode: None,
        })
    }
    /// One style's cache at `raster` hundredths of a percent.
    fn build(
        font: FontRef<'static>,
        style: TextStyle,
        coverage: &mut Vec<u8>,
        em: bool,
        raster: u32,
    ) -> Result<CachedFont> {
        let density = raster as f32 / 10_000.0;
        let scale = if em {
            font.pt_to_px_scale(style.pixel_size() as f32 * density * 0.75)
                .ok_or(Error::InvalidFont)?
        } else {
            PxScale::from(style.pixel_size() as f32 * density)
        };
        let percent = raster.div_ceil(100);
        let scaled = font.as_scaled(scale);
        let physical =
            |logical: u32| ((u64::from(logical) * u64::from(raster) + 5_000) / 10_000) as u32;
        let metrics = FontMetrics {
            pixel_size: physical(style.pixel_size()),
            line_height: physical(style.line_height()),
            baseline: (scaled.ascent() + 0.9999) as u32,
            descent: fixed(scaled.descent()) / 64,
        };
        let mut glyphs = [CachedGlyph::default(); ASCII_COUNT];
        for (index, cached) in glyphs.iter_mut().enumerate() {
            let id = font.glyph_id((ASCII_START + index as u8) as char);
            if id.0 == 0 {
                return Err(Error::InvalidFont);
            }
            cached.advance = fixed(scaled.h_advance(id));
            let raster_scale = if em {
                PxScale {
                    x: scale.x * 3.0,
                    y: scale.y,
                }
            } else {
                scale
            };
            if let Some(outline) =
                font.outline_glyph(id.with_scale_and_position(raster_scale, point(0.0, 0.0)))
            {
                let bounds = outline.px_bounds();
                let channels = if em { 3 } else { 1 };
                // Filter one physical pixel beyond the outline on either side.
                // RGB stripe coverage comes from a genuinely 3x horizontal
                // outline raster, never from stretching grayscale pixels.
                let left = if em {
                    (bounds.min.x as i32).div_euclid(3) - 1
                } else {
                    bounds.min.x as i32
                };
                let right = if em {
                    (bounds.max.x as i32 + 2).div_euclid(3) + 1
                } else {
                    bounds.max.x as i32
                };
                let width = (right - left) as usize;
                let height = bounds.height() as usize;
                if width > glyph_bound(percent) as usize
                    || height > glyph_bound(percent) as usize
                    || left < -(glyph_bound(percent) as i32)
                    || bounds.min.y < -(glyph_bound(percent) as f32)
                    || left > glyph_bound(percent) as i32
                    || bounds.min.y > 64.0
                {
                    return Err(Error::InvalidFont);
                }
                let offset = coverage.len();
                let end = offset
                    .checked_add(width * height * channels)
                    .ok_or(Error::AtlasFull)?;
                if end > atlas_capacity(percent) * channels {
                    return Err(Error::AtlasFull);
                }
                coverage.resize(end, 0);
                if em {
                    let raw_width = bounds.width() as usize;
                    let mut raw = alloc::vec![0u8; raw_width * height];
                    outline.draw(|x, y, alpha| {
                        raw[y as usize * raw_width + x as usize] =
                            (alpha.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    });
                    for y in 0..height {
                        for sub in 0..width * 3 {
                            let position = left * 3 + sub as i32 - bounds.min.x as i32;
                            let mut sum = 0u32;
                            for (delta, weight) in [(-2, 1), (-1, 2), (0, 3), (1, 2), (2, 1)] {
                                let x = position + delta;
                                if (0..raw_width as i32).contains(&x) {
                                    sum += u32::from(raw[y * raw_width + x as usize]) * weight;
                                }
                            }
                            coverage[offset + y * width * 3 + sub] = ((sum + 4) / 9) as u8;
                        }
                    }
                } else {
                    outline.draw(|x, y, alpha| {
                        coverage[offset + y as usize * width + x as usize] =
                            (alpha.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    });
                }
                *cached = CachedGlyph {
                    offset,
                    width: width as u16,
                    height: height as u16,
                    left: left as i16,
                    top: bounds.min.y as i16,
                    advance: cached.advance,
                };
            }
        }
        Ok(CachedFont {
            kerning: Kerning::Outline(Box::new(font), scale),
            glyphs,
            metrics,
        })
    }
    pub fn metrics(&self, style: TextStyle) -> FontMetrics {
        let mut metrics = self.fonts[style.index()].metrics;
        metrics.pixel_size = self.text_len(style.pixel_size());
        metrics.line_height = self.line_height(style);
        metrics.baseline = logical_ceil(self.percent, i64::from(metrics.baseline)) as u32;
        metrics.descent = (i64::from(metrics.descent) * 100 / i64::from(self.percent)) as i32;
        metrics
    }
    pub fn ink_metrics(&self, style: TextStyle) -> InkMetrics {
        let font = &self.fonts[style.index()];
        let h = font.glyphs[(b'H' - ASCII_START) as usize];
        let g = font.glyphs[(b'g' - ASCII_START) as usize];
        let baseline = i32::try_from(font.metrics.baseline).unwrap_or(0);
        let top = baseline + i32::from(h.top.min(g.top));
        let bottom = baseline
            + (i32::from(h.top) + i32::from(h.height)).max(i32::from(g.top) + i32::from(g.height));
        let top = logical_floor(self.percent, i64::from(top)) as i32;
        let bottom = logical_ceil(self.percent, i64::from(bottom)) as i32;
        InkMetrics {
            top,
            height: bottom.saturating_sub(top).max(1) as u32,
        }
    }
    /// Return a TextRun line-box top whose stable ink reference is centered in
    /// the control row. Font leading must not be mistaken for visible ink.
    pub fn centered_top(&self, style: TextStyle, row_top: i32, row_height: u32) -> i32 {
        let ink = self.ink_metrics(style);
        (i64::from(row_top) + (i64::from(row_height) - i64::from(ink.height)).div_euclid(2)
            - i64::from(ink.top))
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    }
    /// One pixel of breathing room around that same font reference, clipped to
    /// the line box. Drawing and caret positioning share the exact metrics.
    pub fn caret_metrics(&self, style: TextStyle) -> InkMetrics {
        let ink = self.ink_metrics(style);
        let top = ink
            .top
            .saturating_sub(1)
            .max(0)
            .min(self.line_height(style) as i32 - 1);
        InkMetrics {
            top,
            height: ink
                .height
                .saturating_add(2)
                .min(self.line_height(style) - top as u32),
        }
    }
    pub fn cache_bytes(&self) -> usize {
        self.coverage.len()
    }
    pub fn cache_capacity(&self) -> usize {
        self.coverage.capacity()
    }
    pub fn uses_packed_cache(&self) -> bool {
        matches!(&self.fonts[0].kerning, Kerning::Pairs(_))
    }
    /// Any UTF-8 up to `MAX_TEXT_BYTES` is accepted. Without an attached
    /// Unicode engine, each grapheme cluster outside printable ASCII draws the
    /// atlas '?' glyph, and control/default-ignorable clusters are zero width.
    fn validate_text(text: &str) -> Result<()> {
        if text.len() > MAX_TEXT_BYTES {
            return Err(Error::TextTooLong);
        }
        Ok(())
    }
    /// Exact painted extent of a glyph, in physical pixels relative to its
    /// bitmap origin: first/last columns and rows with nonzero coverage as
    /// grayscale targets (the default) paint it: an RGB-stripe atlas's
    /// centre subpixel, which is the coverage `Target` draws grayscale.
    fn glyph_ink(&self, glyph: CachedGlyph) -> Option<(i64, i64, i64, i64)> {
        let (w, h) = (glyph.width as usize, glyph.height as usize);
        let lit = |row: usize, column: usize| {
            let at = glyph.offset + (row * w + column) * self.channels;
            if self.channels == 3 {
                self.coverage[at + 1] != 0
            } else {
                self.coverage[at..at + self.channels]
                    .iter()
                    .any(|c| *c != 0)
            }
        };
        let columns = (0..w).filter(|&c| (0..h).any(|r| lit(r, c)));
        let rows = (0..h).filter(|&r| (0..w).any(|c| lit(r, c)));
        let (left, right) = (columns.clone().next()?, columns.clone().next_back()?);
        let (top, bottom) = (rows.clone().next()?, rows.clone().next_back()?);
        Some((left as i64, right as i64 + 1, top as i64, bottom as i64 + 1))
    }
    /// Painted ink of one line of `text` in physical pixels, relative to the
    /// pen origin and line-box top used by `draw`/`draw_physical`: columns
    /// `[left, right)` with nonzero coverage, plus the style's capital band
    /// `[cap_top, cap_bottom)` (the painted rows of 'H'). Labels are centred in
    /// a box on these, as Segoe UI text is optically centred on cap height.
    pub fn label_ink(&self, text: &str, style: TextStyle) -> Result<LabelInk> {
        let font = &self.fonts[style.index()];
        let baseline = i64::from(font.metrics.baseline);
        let h = font.glyphs[(b'H' - ASCII_START) as usize];
        let (cap_top, cap_bottom) =
            self.glyph_ink(h)
                .map_or((baseline - 1, baseline), |(_, _, t, b)| {
                    let top = baseline + i64::from(h.top);
                    (top + t, top + b)
                });
        if let Some(result) = self.unicode_layout(text, style) {
            let (layout, system) = result?;
            let ink = system.ink(&layout, Some(0))?;
            let (left, right) = if ink.is_empty() {
                (0, 0)
            } else {
                (ink.left, ink.right)
            };
            return Ok(LabelInk {
                left,
                right,
                cap_top,
                cap_bottom,
            });
        }
        let mut ink: Option<(i64, i64)> = None;
        self.walk(text, style, |glyph, x, y| {
            if y != 0 {
                return;
            }
            if let Some((l, r, _, _)) = self.glyph_ink(glyph) {
                let left = x + i64::from(glyph.left);
                let span = (left + l, left + r);
                ink = Some(ink.map_or(span, |(a, b)| (a.min(span.0), b.max(span.1))));
            }
        })?;
        let (left, right) = ink.unwrap_or((0, 0));
        Ok(LabelInk {
            left,
            right,
            cap_top,
            cap_bottom,
        })
    }
    pub fn measure(&self, text: &str, style: TextStyle) -> Result<TextMetrics> {
        if let Some(result) = self.unicode_layout(text, style) {
            let (layout, _) = result?;
            let lines = layout.metrics().lines as u32;
            return Ok(TextMetrics {
                advance_width: logical_ceil(self.percent, i64::from(layout.physical_size().0))
                    as u32,
                height: lines * self.line_height(style),
                lines,
                painted_pixels: 0,
            });
        }
        self.walk(text, style, |_, _, _| {})
            .map(|(width, lines)| TextMetrics {
                advance_width: logical_ceil(self.percent, i64::from(width)) as u32,
                height: lines * self.line_height(style),
                lines,
                painted_pixels: 0,
            })
    }
    /// `top` is a line-box top; baseline is top + metrics(style).baseline.
    /// Unsupported input fails before any destination byte changes. Newline and
    /// four-space tab stops are supported; CR is ignored. No wrapping/shaping.
    pub fn draw(
        &self,
        target: &mut Target<'_>,
        clip: Clip,
        run: TextRun<'_>,
    ) -> Result<TextMetrics> {
        let p = self.percent;
        let (x0, y0) = (scaled(p, i64::from(clip.x)), scaled(p, i64::from(clip.y)));
        let x1 = scaled(p, i64::from(clip.x) + i64::from(clip.width));
        let y1 = scaled(p, i64::from(clip.y) + i64::from(clip.height));
        let clip = Clip::new(
            x0.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            y0.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
            (x1 - x0).clamp(0, i64::from(u32::MAX)) as u32,
            (y1 - y0).clamp(0, i64::from(u32::MAX)) as u32,
        );
        self.draw_at(
            target,
            clip,
            run,
            (scaled(p, i64::from(run.x)), scaled(p, i64::from(run.top))),
        )
    }
    /// Native glyph coverage with physical origin and clip, for windows moving
    /// by individual pixels rather than whole logical density units.
    pub fn draw_physical(
        &self,
        target: &mut Target<'_>,
        clip: Clip,
        run: TextRun<'_>,
    ) -> Result<TextMetrics> {
        self.draw_at(target, clip, run, (i64::from(run.x), i64::from(run.top)))
    }
    fn draw_at(
        &self,
        target: &mut Target<'_>,
        clip: Clip,
        run: TextRun<'_>,
        origin: (i64, i64),
    ) -> Result<TextMetrics> {
        if run.color[..3].iter().any(|c| *c > run.color[3]) {
            return Err(Error::InvalidColor);
        }
        if let Some(result) = self.unicode_layout(run.text, run.style) {
            let (layout, system) = result?;
            let clamp = |v: i64| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
            let stats = system.draw(
                &layout,
                target,
                clip,
                clamp(origin.0),
                clamp(origin.1),
                run.color,
            )?;
            let lines = layout.metrics().lines as u32;
            return Ok(TextMetrics {
                advance_width: logical_ceil(self.percent, i64::from(layout.physical_size().0))
                    as u32,
                height: lines * self.line_height(run.style),
                lines,
                painted_pixels: stats.painted_pixels,
            });
        }
        let mut painted = 0;
        let baseline = self.fonts[run.style.index()].metrics.baseline as i64;
        let (width, lines) = self.walk(run.text, run.style, |glyph, x, y| {
            let left = origin.0 + x + glyph.left as i64;
            let top = origin.1 + y + baseline + glyph.top as i64;
            for row in 0..glyph.height as usize {
                for column in 0..glyph.width as usize {
                    let at = glyph.offset + (row * glyph.width as usize + column) * self.channels;
                    let hit = if self.channels == 3 {
                        target.blend_lcd(
                            clip,
                            left + column as i64,
                            top + row as i64,
                            run.color,
                            self.coverage[at..at + 3].try_into().unwrap(),
                        )
                    } else {
                        target.blend(
                            clip,
                            left + column as i64,
                            top + row as i64,
                            run.color,
                            self.coverage[at],
                        )
                    };
                    if hit {
                        painted += 1;
                    }
                }
            }
        })?;
        Ok(TextMetrics {
            advance_width: logical_ceil(self.percent, i64::from(width)) as u32,
            height: lines * self.line_height(run.style),
            lines,
            painted_pixels: painted,
        })
    }
    fn walk(
        &self,
        text: &str,
        style: TextStyle,
        mut glyph: impl FnMut(CachedGlyph, i64, i64),
    ) -> Result<(u32, u32)> {
        Self::validate_text(text)?;
        if text.is_empty() {
            return Ok((0, 0));
        }
        let font = &self.fonts[style.index()];
        let mut pen = 0i64;
        let mut width = 0i64;
        let mut line = 0u32;
        let mut previous = None;
        let tab = (font.glyphs[0].advance as i64 * 4).max(64);
        // ASCII bytes as-is; otherwise one symbol per grapheme cluster.
        let mut clusters = (!text.is_ascii()).then(|| text.graphemes(true));
        let mut bytes = text.bytes();
        loop {
            let byte = match clusters.as_mut() {
                None => match bytes.next() {
                    Some(byte) => byte,
                    None => break,
                },
                Some(iter) => match iter.next() {
                    None => break,
                    Some(cluster) if cluster.len() == 1 => cluster.as_bytes()[0],
                    Some("\r\n") => b'\n',
                    Some(cluster)
                        if cluster.chars().all(|c| {
                            c.is_control() || crate::analysis::is_default_ignorable(c)
                        }) =>
                    {
                        continue;
                    }
                    Some(_) => b'?',
                },
            };
            if byte == b'\r' {
                continue;
            }
            if byte == b'\n' {
                width = width.max(pen);
                pen = 0;
                line += 1;
                previous = None;
                continue;
            }
            if byte == b'\t' {
                pen = (pen / tab + 1) * tab;
                previous = None;
                continue;
            }
            if !(32..=126).contains(&byte) {
                previous = None;
                continue;
            }
            if let Some(previous) = previous {
                pen += i64::from(font.kern(previous, byte));
            }
            let cached = font.glyphs[(byte - ASCII_START) as usize];
            glyph(
                cached,
                (pen + 32).div_euclid(64),
                scaled(
                    self.percent,
                    i64::from(line) * i64::from(self.line_height(style)),
                ),
            );
            pen += cached.advance as i64;
            previous = Some(byte);
        }
        width = width.max(pen);
        Ok((((width.max(0) + 63) / 64) as u32, line + 1))
    }
}
