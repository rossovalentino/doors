//! `TextSystem`: the shared native text facade (fonts, caches, layout, drawing).
use crate::color::rasterize_color;
use crate::font::{Face, FontSet, Locale};
use crate::layout::{self, LayoutOptions, TextFormat, TextLayout};
use crate::raster::{
    CacheStats, GlyphCache, GlyphKey, HintCache, Pixels, RasterMode, rasterize_outline,
};
use crate::shape::ShapeCache;
use crate::{Clip, Error, Result, Target};
use alloc::vec::Vec;
#[allow(unused_imports)]
use core_maths::CoreFloat;
use skrifa::instance::NormalizedCoord;

/// Cache budgets. Defaults: 4 MiB of glyph bitmaps, 512 KiB of shaped runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheLimits {
    pub glyph_bytes: usize,
    pub shape_bytes: usize,
}
impl Default for CacheLimits {
    fn default() -> Self {
        Self {
            glyph_bytes: 4 << 20,
            shape_bytes: 512 << 10,
        }
    }
}

/// Result of drawing a layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrawStats {
    pub glyphs: usize,
    pub painted_pixels: usize,
}

/// Painted ink box in physical pixels relative to the layout origin
/// (`left..right`, `top..bottom`); empty when nothing paints.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InkBox {
    pub left: i64,
    pub top: i64,
    pub right: i64,
    pub bottom: i64,
}
impl InkBox {
    pub fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }
}

#[derive(Clone)]
pub(crate) struct InstanceRef {
    pub id: u64,
    pub font: harfrust::Font,
    pub coords: Vec<NormalizedCoord>,
}

struct Instance {
    face: u16,
    wght: u32,
    opsz: u32,
    value: InstanceRef,
    used: u64,
}

const MAX_INSTANCES: usize = 64;

pub(crate) struct Caches {
    pub glyphs: GlyphCache,
    pub shapes: ShapeCache,
    pub hints: HintCache,
    instances: Vec<Instance>,
    scratch: Vec<u8>,
    clock: u64,
}

impl Caches {
    /// Variation instance for a face at (`wght`, `opsz`); (0, 0) is the default.
    pub fn instance(&mut self, face: u16, f: &Face, wght: f32, opsz: f32) -> InstanceRef {
        self.clock += 1;
        let (wb, ob) = (wght.to_bits(), opsz.to_bits());
        if let Some(entry) = self
            .instances
            .iter_mut()
            .find(|i| i.face == face && i.wght == wb && i.opsz == ob)
        {
            entry.used = self.clock;
            return entry.value.clone();
        }
        let mut settings: Vec<(&str, f32)> = Vec::new();
        if f.wght.is_some() && wght > 0.0 {
            settings.push(("wght", wght));
        }
        if f.opsz.is_some() && opsz > 0.0 {
            settings.push(("opsz", opsz));
        }
        let value = if settings.is_empty() {
            InstanceRef {
                id: 0,
                font: f.shaper.clone(),
                coords: Vec::new(),
            }
        } else {
            let location =
                skrifa::MetadataProvider::axes(&f.font).location(settings.iter().copied());
            let mut id = 0xcbf2_9ce4_8422_2325u64;
            for v in [
                u64::from(face),
                u64::from(wb),
                u64::from(ob),
                f.info.fingerprint,
            ] {
                id = (id ^ v).wrapping_mul(0x0100_0000_01b3);
            }
            InstanceRef {
                id: id.max(1),
                font: f
                    .shaper
                    .instance_builder()
                    .variations(settings.iter().copied())
                    .build(),
                coords: location.coords().to_vec(),
            }
        };
        if self.instances.len() >= MAX_INSTANCES
            && let Some(oldest) = self
                .instances
                .iter()
                .enumerate()
                .min_by_key(|(_, i)| i.used)
                .map(|(i, _)| i)
        {
            self.instances.swap_remove(oldest);
        }
        self.instances.push(Instance {
            face,
            wght: wb,
            opsz: ob,
            value: value.clone(),
            used: self.clock,
        });
        value
    }
}

/// Shared native text facade: an immutable `FontSet` plus bounded caches
/// behind a spin lock (`Send + Sync`; uncontended in single-threaded apps).
pub struct TextSystem {
    fonts: FontSet,
    caches: spin::Mutex<Caches>,
    limits: CacheLimits,
}

impl TextSystem {
    pub fn new(fonts: FontSet) -> Self {
        Self::with_limits(fonts, CacheLimits::default())
    }
    pub fn with_limits(fonts: FontSet, limits: CacheLimits) -> Self {
        Self {
            fonts,
            caches: spin::Mutex::new(Caches {
                glyphs: GlyphCache::new(limits.glyph_bytes),
                shapes: ShapeCache::new(limits.shape_bytes),
                hints: HintCache::new(),
                instances: Vec::new(),
                scratch: Vec::new(),
                clock: 0,
            }),
            limits,
        }
    }
    pub fn fonts(&self) -> &FontSet {
        &self.fonts
    }
    pub fn limits(&self) -> CacheLimits {
        self.limits
    }
    /// Lay out `text` (any UTF-8, up to `MAX_LAYOUT_BYTES`).
    pub fn layout(
        &self,
        text: &str,
        format: &TextFormat,
        options: &LayoutOptions,
    ) -> Result<TextLayout> {
        let mut caches = self.caches.lock();
        layout::build(&self.fonts, &mut caches, text, format, options)
    }
    /// Single-line convenience layout.
    pub fn layout_line(&self, text: &str, format: &TextFormat) -> Result<TextLayout> {
        self.layout(
            text,
            format,
            &LayoutOptions {
                wrap: layout::Wrap::None,
                ..LayoutOptions::default()
            },
        )
    }
    pub fn cache_stats(&self) -> CacheStats {
        let caches = self.caches.lock();
        CacheStats {
            glyphs: caches.glyphs.len(),
            glyph_bytes: caches.glyphs.bytes(),
            glyph_budget: caches.glyphs.budget,
            glyph_hits: caches.glyphs.hits,
            glyph_misses: caches.glyphs.misses,
            glyph_evictions: caches.glyphs.evictions,
            shaped_runs: caches.shapes.len(),
            shaped_bytes: caches.shapes.bytes(),
            shape_hits: caches.shapes.hits,
            shape_misses: caches.shapes.misses,
            hinting_instances: caches.hints.len(),
        }
    }
    /// Drop every cached glyph, shaped run, plan and hinting instance.
    pub fn clear_caches(&self) {
        let mut caches = self.caches.lock();
        caches.glyphs.clear();
        caches.shapes.clear();
        caches.hints.clear();
        caches.instances.clear();
    }

    /// Draw a layout with its first line box top-left at physical (`x`, `y`).
    /// `color` is premultiplied BGRA. Colour emoji keep their palette colours
    /// and take the text alpha. Fails before painting on a foreign layout or an
    /// invalid colour.
    pub fn draw(
        &self,
        layout: &TextLayout,
        target: &mut Target<'_>,
        clip: Clip,
        x: i32,
        y: i32,
        color: [u8; 4],
    ) -> Result<DrawStats> {
        if color[..3].iter().any(|c| *c > color[3]) {
            return Err(Error::InvalidColor);
        }
        if layout.identity != self.fonts.identity() {
            return Err(Error::InvalidFont);
        }
        let mut stats = DrawStats::default();
        let mut caches = self.caches.lock();
        let caches = &mut *caches;
        let bounds = target.bounds();
        let (cx0, cy0) = (i64::from(clip.x.max(0)), i64::from(clip.y.max(0)));
        let cx1 = (i64::from(clip.x) + i64::from(clip.width)).min(i64::from(bounds.width));
        let cy1 = (i64::from(clip.y) + i64::from(clip.height)).min(i64::from(bounds.height));
        if cx0 >= cx1 || cy0 >= cy1 {
            return Ok(stats);
        }
        let reach = (layout.format.ppem() * 4.0) as i64 + 4;
        let fg = u32::from_le_bytes(color);
        self.each_glyph(
            layout,
            caches,
            i64::from(x),
            i64::from(y),
            fg,
            |gx, gy, bitmap| {
                let left = gx + i64::from(bitmap.left);
                let top = gy + i64::from(bitmap.top);
                if left >= cx1
                    || top >= cy1
                    || left + i64::from(bitmap.width) <= cx0
                    || top + i64::from(bitmap.height) <= cy0
                {
                    return;
                }
                stats.glyphs += 1;
                let (w, h) = (bitmap.width as usize, bitmap.height as usize);
                let bpp = bitmap.pixels.bytes();
                for row in 0..h {
                    let py = top + row as i64;
                    if py < cy0 || py >= cy1 {
                        continue;
                    }
                    for column in 0..w {
                        let px = left + column as i64;
                        if px < cx0 || px >= cx1 {
                            continue;
                        }
                        let at = (row * w + column) * bpp;
                        let hit = match bitmap.pixels {
                            Pixels::Lcd => target.blend_lcd(
                                clip,
                                px,
                                py,
                                color,
                                [bitmap.data[at], bitmap.data[at + 1], bitmap.data[at + 2]],
                            ),
                            Pixels::Gray => target.blend(clip, px, py, color, bitmap.data[at]),
                            Pixels::Bgra => target.blend_premultiplied(
                                clip,
                                px,
                                py,
                                [
                                    bitmap.data[at],
                                    bitmap.data[at + 1],
                                    bitmap.data[at + 2],
                                    bitmap.data[at + 3],
                                ],
                                color[3],
                            ),
                        };
                        if hit {
                            stats.painted_pixels += 1;
                        }
                    }
                }
            },
            Some((cx0 - reach, cy0 - reach, cx1 + reach, cy1 + reach)),
        );
        Ok(stats)
    }

    /// Exact painted ink of a layout (or one line) in physical pixels relative
    /// to the layout origin.
    pub fn ink(&self, layout: &TextLayout, line: Option<usize>) -> Result<InkBox> {
        if layout.identity != self.fonts.identity() {
            return Err(Error::InvalidFont);
        }
        let mut caches = self.caches.lock();
        let caches = &mut *caches;
        let mut ink: Option<InkBox> = None;
        let line_top = line.and_then(|l| layout.lines.get(l)).map(|l| l.top as i64);
        let line_range = line_top.map(|t| (t, t + layout.line_height as i64));
        self.each_glyph(
            layout,
            caches,
            0,
            0,
            0xff00_0000,
            |gx, gy, bitmap| {
                if let Some((t, b)) = line_range {
                    let baseline = gy - layout.baseline as i64;
                    if baseline < t || baseline >= b {
                        return;
                    }
                }
                let (w, h) = (bitmap.width as usize, bitmap.height as usize);
                let bpp = bitmap.pixels.bytes();
                let lit = |r: usize, c: usize| {
                    let at = (r * w + c) * bpp;
                    let px = &bitmap.data[at..at + bpp];
                    if bitmap.pixels == Pixels::Bgra {
                        px[3] != 0
                    } else {
                        px.iter().any(|v| *v != 0)
                    }
                };
                let mut found: Option<(usize, usize, usize, usize)> = None;
                for r in 0..h {
                    for c in 0..w {
                        if lit(r, c) {
                            found = Some(match found {
                                None => (c, c + 1, r, r + 1),
                                Some((l, rr, t, b)) => {
                                    (l.min(c), rr.max(c + 1), t.min(r), b.max(r + 1))
                                }
                            });
                        }
                    }
                }
                if let Some((l, r, t, b)) = found {
                    let bx = InkBox {
                        left: gx + i64::from(bitmap.left) + l as i64,
                        right: gx + i64::from(bitmap.left) + r as i64,
                        top: gy + i64::from(bitmap.top) + t as i64,
                        bottom: gy + i64::from(bitmap.top) + b as i64,
                    };
                    ink = Some(match ink {
                        None => bx,
                        Some(i) => InkBox {
                            left: i.left.min(bx.left),
                            right: i.right.max(bx.right),
                            top: i.top.min(bx.top),
                            bottom: i.bottom.max(bx.bottom),
                        },
                    });
                }
            },
            None,
        );
        Ok(ink.unwrap_or_default())
    }

    /// Visit every glyph bitmap at its physical pen position (origin on the baseline).
    #[allow(clippy::type_complexity, clippy::too_many_arguments)]
    fn each_glyph(
        &self,
        layout: &TextLayout,
        caches: &mut Caches,
        x: i64,
        y: i64,
        foreground: u32,
        mut visit: impl FnMut(i64, i64, &crate::raster::GlyphBitmap),
        cull: Option<(i64, i64, i64, i64)>,
    ) {
        let format = &layout.format;
        let ppem = format.ppem();
        let ppem64 = (ppem * 64.0).round() as u32;
        let fg_bytes = foreground.to_le_bytes();
        let faces = self.fonts.snapshot();
        for line in &layout.lines {
            let baseline = y as f32 + line.top + layout.baseline;
            for run in &layout.runs[line.runs.start as usize..line.runs.end as usize] {
                let face_index = usize::from(run.face);
                let Some(face) = faces.get(face_index) else {
                    continue;
                };
                let instance = caches.instance(run.face, face, run.wght, run.opsz);
                let hinted = format.hinting && face.info.hinted && ppem <= 256.0;
                for glyph in &layout.glyphs[run.glyphs.start as usize..run.glyphs.end as usize] {
                    let pen = x as f32 + line.x + glyph.x;
                    let (gx, phase) = if format.subpixel_positioning {
                        let q = (pen * 3.0 + 0.5).floor() as i64;
                        (q.div_euclid(3), q.rem_euclid(3) as u8)
                    } else {
                        ((pen + 0.5).floor() as i64, 0)
                    };
                    let gy = (baseline + glyph.y + 0.5).floor() as i64;
                    if let Some((x0, y0, x1, y1)) = cull
                        && (gx < x0 || gx > x1 || gy < y0 || gy > y1 + (ppem as i64) * 2)
                    {
                        continue;
                    }
                    let color = face.info.color
                        && face.colors.get(skrifa::GlyphId::new(glyph.id)).is_some();
                    let key = GlyphKey {
                        font: face.info.fingerprint,
                        face: run.face,
                        glyph: glyph.id,
                        ppem: ppem64,
                        instance: instance.id,
                        mode: if color {
                            2
                        } else if format.raster == RasterMode::Lcd {
                            0
                        } else {
                            1
                        },
                        phase: if color { 0 } else { phase },
                        flags: u8::from(hinted && !color) | (u8::from(run.embolden && !color) << 1),
                        foreground: if color { foreground } else { 0 },
                    };
                    let slot = match caches.glyphs.lookup(&key) {
                        Some(slot) => slot,
                        None => {
                            let bitmap = if color {
                                rasterize_color(face, glyph.id, ppem, &instance.coords, fg_bytes)
                                    .unwrap_or_else(|| {
                                        crate::raster::GlyphBitmap::empty(Pixels::Bgra)
                                    })
                            } else {
                                let hint = if hinted {
                                    caches.hints.get(
                                        run.face,
                                        face,
                                        instance.id,
                                        &instance.coords,
                                        ppem,
                                    )
                                } else {
                                    None
                                };
                                let samples = if run.embolden {
                                    ((ppem / 8.0).round() as usize).max(1)
                                } else {
                                    0
                                };
                                let hint = hint.map(|h| caches.hints.instance(h));
                                rasterize_outline(
                                    face,
                                    glyph.id,
                                    ppem,
                                    &instance.coords,
                                    hint,
                                    format.raster,
                                    phase,
                                    samples,
                                    &mut caches.scratch,
                                )
                            };
                            caches.glyphs.insert(key, bitmap, u64::MAX)
                        }
                    };
                    visit(gx, gy, caches.glyphs.get(slot));
                }
            }
        }
    }

    /// Metrics of a format's primary face: (ascent, descent, line height,
    /// baseline offset) in logical pixels.
    pub fn line_metrics(&self, format: &TextFormat) -> Result<(f32, f32, f32, f32)> {
        let layout = self.layout_line("", format)?;
        Ok((
            layout.baseline(),
            layout.line_height() - layout.baseline(),
            layout.line_height(),
            layout.baseline(),
        ))
    }
    /// Locale-aware convenience for the common case.
    pub fn format(&self, size: f32, weight: u16, scale: f32, locale: &str) -> TextFormat {
        TextFormat::new(size)
            .with_weight(weight)
            .with_scale(scale)
            .with_locale(Locale::new(locale))
    }
}
