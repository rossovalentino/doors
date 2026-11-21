//! Host exporter and labeled comparisons for the checked native font caches.
use crate::evidence::{Run, file_hash, relative, write_json};
use ab_glyph::{
    Font, FontRef, Outline, OutlineCurve, OutlinedGlyph, Point, PxScaleFactor, ScaleFont,
    VariableFont, point,
};
use door_graphics_text::{Clip, FontAtlas, TEXT_STYLES, Target, TextRun, TextStyle, packed};
use serde_json::json;
use sha2::{Digest, Sha256};
use skrifa::{
    MetadataProvider,
    instance::Size,
    outline::{
        DrawSettings, Engine, Hinting, HintingInstance, HintingOptions, OutlinePen, SmoothMode,
        Target as HintTarget,
    },
};
use std::fs;
use std::{path::Path, time::Instant};

const VARIABLE_SHA: &str = "27ca1ab4bd5ad3b0404ee6a9a03b143408dbd7e5a4c9e4b2054639c03fc0f682";
const ROWS: [(TextStyle, &str); 6] = [
    (TextStyle::Regular12, "File Explorer  Downloads  notes.txt"),
    (
        TextStyle::Regular14,
        "The quick brown fox jumps over the lazy dog.",
    ),
    (TextStyle::Semibold14, "Settings  -  Network & internet"),
    (TextStyle::Regular20, "Personalization"),
    (TextStyle::Semibold20, "System > Display"),
    (TextStyle::Semibold28, "Welcome back"),
];
#[derive(Default)]
struct Pen {
    curves: Vec<OutlineCurve>,
    current: Option<Point>,
    start: Option<Point>,
    invalid: bool,
}
impl Pen {
    fn push(&mut self, curve: OutlineCurve, points: &[Point]) {
        if self.curves.len() >= 1024
            || points.iter().any(|p| {
                !p.x.is_finite() || !p.y.is_finite() || p.x.abs() > 1024.0 || p.y.abs() > 1024.0
            })
        {
            self.invalid = true;
        } else if !self.invalid {
            self.curves.push(curve);
        }
    }
    fn outline(self) -> Result<Option<OutlinedGlyph>, String> {
        if self.invalid {
            return Err("hinted outline exceeded checked bounds".into());
        }
        if self.curves.is_empty() {
            return Ok(None);
        }
        let mut min = point(f32::INFINITY, f32::INFINITY);
        let mut max = point(f32::NEG_INFINITY, f32::NEG_INFINITY);
        for curve in &self.curves {
            let points: &[Point] = match curve {
                OutlineCurve::Line(a, b) => &[*a, *b],
                OutlineCurve::Quad(a, b, c) => &[*a, *b, *c],
                OutlineCurve::Cubic(a, b, c, d) => &[*a, *b, *c, *d],
            };
            for p in points {
                min.x = min.x.min(p.x);
                min.y = min.y.min(p.y);
                max.x = max.x.max(p.x);
                max.y = max.y.max(p.y);
            }
        }
        Ok(Some(OutlinedGlyph::new(
            ab_glyph::GlyphId(1).with_scale(1.0),
            Outline {
                // Outline stores a y-up top/bottom pair; px_bounds flips y.
                bounds: ab_glyph::Rect {
                    min: point(min.x, max.y),
                    max: point(max.x, min.y),
                },
                curves: self.curves,
            },
            PxScaleFactor {
                horizontal: 3.0,
                vertical: 1.0,
            },
        )))
    }
}
impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.current = Some(point(x, y));
        self.start = self.current;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let next = point(x, y);
        if let Some(a) = self.current {
            self.push(OutlineCurve::Line(a, next), &[a, next]);
        } else {
            self.invalid = true;
        }
        self.current = Some(next);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (b, c) = (point(cx, cy), point(x, y));
        if let Some(a) = self.current {
            self.push(OutlineCurve::Quad(a, b, c), &[a, b, c]);
        } else {
            self.invalid = true;
        }
        self.current = Some(c);
    }
    fn curve_to(&mut self, cx: f32, cy: f32, dx: f32, dy: f32, x: f32, y: f32) {
        let (b, c, d) = (point(cx, cy), point(dx, dy), point(x, y));
        if let Some(a) = self.current {
            self.push(OutlineCurve::Cubic(a, b, c, d), &[a, b, c, d]);
        } else {
            self.invalid = true;
        }
        self.current = Some(d);
    }
    fn close(&mut self) {
        if let (Some(a), Some(b)) = (self.current, self.start) {
            if a != b {
                self.push(OutlineCurve::Line(a, b), &[a, b]);
            }
        } else {
            self.invalid = true;
        }
        self.current = None;
        self.start = None;
    }
}
struct Glyph {
    left: i32,
    top: i32,
    width: usize,
    height: usize,
    rgb: Vec<u8>,
    advance: i64,
}
struct Candidate<'a> {
    font: FontRef<'a>,
    scale: ab_glyph::PxScale,
    baseline: i32,
    glyphs: Vec<Glyph>,
}
impl<'a> Candidate<'a> {
    fn new(data: &'a [u8], style: TextStyle, percent: u32) -> Result<Self, String> {
        let weight = if matches!(
            style,
            TextStyle::Semibold14
                | TextStyle::Semibold16
                | TextStyle::Semibold20
                | TextStyle::Semibold28
        ) {
            600.0
        } else {
            400.0
        };
        let optical = style.pixel_size() as f32 * 0.75;
        let mut font = FontRef::try_from_slice(data).map_err(|_| "variable font")?;
        if !font.set_variation(b"wght", weight) || !font.set_variation(b"opsz", optical) {
            return Err("required variable axes absent".into());
        }
        let scale = font
            .pt_to_px_scale(style.pixel_size() as f32 * percent as f32 / 100.0 * 0.75)
            .ok_or("variable em scale")?;
        let baseline = font.as_scaled(scale).ascent().ceil() as i32;
        let face = skrifa::FontRef::new(data).map_err(|_| "hint font")?;
        let location = face.axes().location(&[("wght", weight), ("opsz", optical)]);
        let outlines = face.outline_glyphs();
        let hints = HintingInstance::new(
            &outlines,
            Size::new(style.pixel_size() as f32 * percent as f32 / 100.0),
            &location,
            HintingOptions {
                engine: Engine::Interpreter,
                target: HintTarget::Smooth {
                    mode: SmoothMode::Lcd,
                    symmetric_rendering: true,
                    preserve_linear_metrics: true,
                },
            },
        )
        .map_err(|e| format!("font hint instance: {e:?}"))?;
        let mut glyphs = Vec::with_capacity(95);
        let mut total = 0;
        for byte in 32..=126 {
            let id = face
                .charmap()
                .map(char::from(byte))
                .ok_or("missing ASCII glyph")?;
            let glyph = outlines.get(id).ok_or("missing ASCII outline")?;
            let memory_bytes = glyph.draw_memory_size(Hinting::Embedded);
            if memory_bytes > 2 * 1024 * 1024 {
                return Err("hint scratch bound".into());
            }
            let mut scratch = vec![0; memory_bytes];
            let mut pen = Pen::default();
            glyph
                .draw(
                    DrawSettings::hinted(&hints, true).with_memory(Some(&mut scratch)),
                    &mut pen,
                )
                .map_err(|e| format!("hinted glyph {byte}: {e:?}"))?;
            let advance = (font
                .as_scaled(scale)
                .h_advance(font.glyph_id(char::from(byte)))
                * 64.0)
                .round() as i64;
            let mut cached = Glyph {
                left: 0,
                top: 0,
                width: 0,
                height: 0,
                rgb: Vec::new(),
                advance,
            };
            if let Some(outline) = pen.outline()? {
                let b = outline.px_bounds();
                if b.width() <= 0.0 || b.height() <= 0.0 {
                    return Err("nonempty outline has invalid pixel bounds".into());
                }
                let left = (b.min.x as i32).div_euclid(3) - 1;
                let right = (b.max.x as i32 + 2).div_euclid(3) + 1;
                let (width, height) = ((right - left) as usize, b.height() as usize);
                let bound = (64 * percent).div_ceil(100) as usize;
                if width > bound || height > bound {
                    return Err("glyph dimensions".into());
                }
                let raw_width = b.width() as usize;
                let mut raw = vec![0u8; raw_width * height];
                outline.draw(|x, y, a| {
                    raw[y as usize * raw_width + x as usize] =
                        (a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
                });
                if !raw.iter().any(|&coverage| coverage != 0) {
                    return Err("nonempty hinted glyph has no coverage".into());
                }
                let mut rgb = vec![0; width * height * 3];
                for y in 0..height {
                    for sub in 0..width * 3 {
                        let position = left * 3 + sub as i32 - b.min.x as i32;
                        let mut sum = 0u32;
                        for (delta, weight) in [(-2, 1), (-1, 2), (0, 3), (1, 2), (2, 1)] {
                            let x = position + delta;
                            if (0..raw_width as i32).contains(&x) {
                                sum += u32::from(raw[y * raw_width + x as usize]) * weight;
                            }
                        }
                        rgb[y * width * 3 + sub] = ((sum + 4) / 9) as u8;
                    }
                }
                total += rgb.len();
                if total > door_graphics_text::atlas_capacity(percent) * 3 {
                    return Err("candidate atlas bound".into());
                }
                cached = Glyph {
                    left,
                    top: b.min.y as i32,
                    width,
                    height,
                    rgb,
                    advance,
                };
            }
            glyphs.push(cached);
        }
        Ok(Self {
            font,
            scale,
            baseline,
            glyphs,
        })
    }
}
fn reference_data(root: &Path) -> Result<Vec<u8>, String> {
    let path = root.join("assets/fonts/segoe-ui/SegUIVar.ttf");
    if file_hash(&path)? != VARIABLE_SHA {
        return Err("variable reference font changed".into());
    }
    fs::read(path).map_err(|e| e.to_string())
}
fn put16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}
fn put32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn pack(data: &[u8], percent: u32) -> Result<Vec<u8>, String> {
    let fonts: Vec<_> = TEXT_STYLES
        .iter()
        .map(|style| Candidate::new(data, *style, percent))
        .collect::<Result<_, _>>()?;
    let mut bytes = vec![0; packed::HEADER_BYTES + TEXT_STYLES.len() * packed::STYLE_BYTES];
    bytes[..8].copy_from_slice(packed::MAGIC);
    let [factor_byte, fraction_byte] = packed::scale_bytes(percent);
    bytes[8..12].copy_from_slice(&[factor_byte, TEXT_STYLES.len() as u8, 3, fraction_byte]);
    let scaled = |logical: u32| ((logical * percent + 49) / 100) as u16;
    let mut coverage = Vec::new();
    for (index, (font, style)) in fonts.iter().zip(TEXT_STYLES).enumerate() {
        let header = packed::HEADER_BYTES + index * packed::STYLE_BYTES;
        put16(&mut bytes, header, scaled(style.pixel_size()));
        put16(&mut bytes, header + 2, scaled(style.line_height()));
        put16(&mut bytes, header + 4, font.baseline as u16);
        put16(
            &mut bytes,
            header + 6,
            ((font.font.as_scaled(font.scale).descent() * 64.0).round() as i32 / 64) as i16 as u16,
        );
        let offset = bytes.len() as u32;
        put32(&mut bytes, header + 8, offset);
        for glyph in &font.glyphs {
            bytes.extend_from_slice(&(coverage.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(glyph.width as u16).to_le_bytes());
            bytes.extend_from_slice(&(glyph.height as u16).to_le_bytes());
            bytes.extend_from_slice(&(glyph.left as i16).to_le_bytes());
            bytes.extend_from_slice(&(glyph.top as i16).to_le_bytes());
            bytes.extend_from_slice(&(glyph.advance as i32).to_le_bytes());
            coverage.extend_from_slice(&glyph.rgb);
        }
        let offset = bytes.len() as u32;
        put32(&mut bytes, header + 12, offset);
        let mut pairs = 0u16;
        let scaled = font.font.as_scaled(font.scale);
        for before in 32u8..=126 {
            for after in 32u8..=126 {
                let delta = (scaled.kern(
                    font.font.glyph_id(char::from(before)),
                    font.font.glyph_id(char::from(after)),
                ) * 64.0)
                    .round() as i32;
                if delta != 0 {
                    let key = u16::from(before - 32) * 95 + u16::from(after - 32);
                    bytes.extend_from_slice(&key.to_le_bytes());
                    bytes.extend_from_slice(&delta.to_le_bytes());
                    pairs += 1;
                }
            }
        }
        put16(&mut bytes, header + 16, pairs);
    }
    let offset = bytes.len() as u32;
    put32(&mut bytes, 12, offset);
    put32(&mut bytes, 16, coverage.len() as u32);
    bytes.extend_from_slice(
        &qoi::encode_to_vec(&coverage, coverage.len() as u32 / 3, 1).map_err(|e| e.to_string())?,
    );
    let length = bytes.len() as u32;
    put32(&mut bytes, 20, length);
    FontAtlas::from_packed_percent(&bytes, percent)
        .map_err(|e| format!("packed font preflight: {e:?}"))?;
    Ok(bytes)
}
pub fn export(root: &Path, check: bool) -> Result<(), String> {
    let data = reference_data(root)?;
    let directory = root.join("assets/fonts/segoe-ui");
    let mut records = Vec::new();
    // Integer atlases keep their names and bytes; fractional display scales
    // (W8) add hinted-125/150/175.dfa at fractional ppem.
    for percent in [100, 200, 300, 125, 150, 175] {
        let bytes = pack(&data, percent)?;
        let filename = if percent % 100 == 0 {
            format!("hinted-{}x.dfa", percent / 100)
        } else {
            format!("hinted-{percent}.dfa")
        };
        let path = directory.join(&filename);
        if check {
            if fs::read(&path).map_err(|e| e.to_string())? != bytes {
                return Err(format!("stale hinted atlas {filename}"));
            }
        } else {
            fs::write(&path, &bytes).map_err(|e| e.to_string())?;
        }
        let mut entry = json!({"file":filename,"percent":percent,"sha256":format!("{:x}",Sha256::digest(&bytes)),"bytes":bytes.len(),"coverage_bytes":u32::from_le_bytes(bytes[16..20].try_into().unwrap())});
        if percent % 100 == 0 {
            entry["factor"] = json!(percent / 100);
        }
        records.push(entry);
    }
    let record = json!({"schema_version":1,"font_sha256":VARIABLE_SHA,"generator":"Original Door exporter / Skrifa0.48.0 embedded hint interpreter / ab_glyph area raster / existing RGB filter","scope":"Eight ASCII styles; original static/legacy kerning policy, not complete Unicode shaping or Microsoft ClearType","atlases":records});
    let mut bytes = serde_json::to_vec_pretty(&record).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    let path = directory.join("hinted-atlases.json");
    if check {
        if fs::read(path).map_err(|e| e.to_string())? != bytes {
            return Err("stale hinted atlas manifest".into());
        }
    } else {
        fs::write(path, bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub fn preview(run: &mut Run) -> Result<(), String> {
    let data = reference_data(&run.root)?;
    let mut records = Vec::new();
    for factor in 1..=3 {
        let started = Instant::now();
        let current =
            FontAtlas::windows_outline_reference_scaled(factor).map_err(|e| format!("{e:?}"))?;
        let old_micros = started.elapsed().as_micros();
        let bytes = pack(&data, factor * 100)?;
        let started = Instant::now();
        let candidate = FontAtlas::from_packed(&bytes, factor).map_err(|e| format!("{e:?}"))?;
        let decode_micros = started.elapsed().as_micros();
        for (theme, background, foreground) in [("light", 255, 24), ("dark", 32, 245)] {
            let (width, height) = (1100 * factor, 420 * factor);
            let mut pixels = vec![background; (width * height * 4) as usize];
            for pixel in pixels.chunks_exact_mut(4) {
                pixel[3] = 255;
            }
            let draw_current = |atlas: &FontAtlas,
                                pixels: &mut [u8],
                                text: &str,
                                x,
                                top,
                                style|
             -> Result<(), String> {
                atlas
                    .draw(
                        &mut Target::new(pixels, width, height, width as usize * 4)
                            .map_err(|e| format!("{e:?}"))?
                            .cleartype(),
                        Clip::new(x, top, 510, 46),
                        TextRun {
                            text,
                            x,
                            top,
                            style,
                            color: [foreground, foreground, foreground, 255],
                        },
                    )
                    .map_err(|e| format!("preview text: {e:?}"))?;
                Ok(())
            };
            draw_current(
                &current,
                &mut pixels,
                "Baseline: static Segoe / unhinted RGB LCD",
                24,
                16,
                TextStyle::Regular12,
            )?;
            draw_current(
                &current,
                &mut pixels,
                "Candidate: variable Segoe / hinted RGB LCD",
                574,
                16,
                TextStyle::Regular12,
            )?;
            for (index, (style, text)) in ROWS.iter().enumerate() {
                let top = 60 + index as i32 * 52;
                draw_current(&current, &mut pixels, text, 24, top, *style)?;
                draw_current(&candidate, &mut pixels, text, 574, top, *style)?;
            }
            draw_current(
                &current,
                &mut pixels,
                "Host evaluation; not a guest or Windows reference capture.",
                24,
                396,
                TextStyle::Regular12,
            )?;
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            let mut image = resvg::tiny_skia::Pixmap::new(width, height).ok_or("preview image")?;
            image.data_mut().copy_from_slice(&pixels);
            let path = run
                .directory
                .join(format!("typography-{factor}x-{theme}.png"));
            image.save_png(&path).map_err(|e| e.to_string())?;
            records.push(json!({"factor":factor,"theme":theme,"path":relative(&run.root,&path)?,"sha256":file_hash(&path)?,"host_single_sample_static_build_micros":old_micros,"host_single_sample_cache_decode_micros":decode_micros,"cache_file_bytes":bytes.len(),"coverage_bytes":candidate.cache_bytes(),"native_timing_claim":false}));
        }
    }
    let record = json!({"schema_version":1,"record_kind":"host_typography_candidate_preview","font_sha256":VARIABLE_SHA,
        "backend":"Skrifa0.48 embedded interpreter; preserved linear advances; horizontal LCD and existing RGB filter",
        "production_selection":"Packed candidate, native comparison and review pending","guest_tested":false,"images":records});
    write_json(&run.directory.join("typography-preview.json"), &record)?;
    run.check("p12-host-typography-candidate-preview", record);
    Ok(())
}
