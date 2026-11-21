//! Font files, faces and collections as DirectWrite models them: each face
//! of a file (and each named instance of a variable font) is a font with a
//! weight, stretch and style in a family named by its WWS, typographic or
//! legacy family name. Metrics follow DirectWrite's rules (Windows ascent
//! and descent unless USE_TYPO_METRICS); fonts match requests with CSS
//! font matching, simulating bold and oblique where no face fits.
use alloc::{string::String, sync::Arc, vec::Vec};
#[allow(unused_imports)] // std builds (host tests) have these inherently.
use core_maths::CoreFloat;
use skrifa::{
    FontRef, GlyphId, MetadataProvider, Tag,
    instance::{LocationRef, NormalizedCoord, Size},
    raw::{FileRef, TableProvider},
    string::StringId,
};

pub const WEIGHT_NORMAL: u32 = 400;
pub const STRETCH_NORMAL: u32 = 5;
pub const STYLE_NORMAL: u32 = 0;
pub const STYLE_OBLIQUE: u32 = 1;
pub const STYLE_ITALIC: u32 = 2;
pub const SIMULATION_BOLD: u32 = 1;
pub const SIMULATION_OBLIQUE: u32 = 2;
/// Shear of an oblique simulation (DirectWrite's 20 degrees ~ tan 0.364;
/// Windows uses about 0.3346).
pub const OBLIQUE_SHEAR: f32 = 0.3346;

/// Font file bytes: a shared read-only mapping, or a copy of a file a
/// program supplied through its own loader.
#[derive(Clone)]
pub enum Bytes {
    Static(&'static [u8]),
    Shared(Arc<[u8]>),
}
impl Bytes {
    pub fn get(&self) -> &[u8] {
        match self {
            Self::Static(b) => b,
            Self::Shared(b) => b,
        }
    }
}
/// A font file: its bytes, its path (system and local files) and, for
/// the guest, the reference key and loader it was created with.
pub struct File {
    pub bytes: Bytes,
    pub path: Vec<u16>,
    pub key: Vec<u8>,
    /// The program's IDWriteFontFileLoader (0: Doors' local loader).
    pub loader: u64,
}

/// One font: a face of a file at a variation location.
pub struct Face {
    pub file: Arc<File>,
    pub index: u32,
    pub coords: Vec<NormalizedCoord>,
    pub weight: u32,
    pub stretch: u32,
    pub style: u32,
    pub simulations: u32,
    /// (locale, name) pairs.
    pub family: Vec<(String, String)>,
    pub face_names: Vec<(String, String)>,
}

/// DWRITE_FONT_METRICS, in design units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Metrics {
    pub units_per_em: u16,
    pub ascent: u16,
    pub descent: u16,
    pub line_gap: i16,
    pub cap_height: u16,
    pub x_height: u16,
    pub underline_position: i16,
    pub underline_thickness: u16,
    pub strikethrough_position: i16,
    pub strikethrough_thickness: u16,
}
impl Metrics {
    pub fn to_bytes(self) -> [u8; 20] {
        let mut out = [0u8; 20];
        let values = [
            self.units_per_em,
            self.ascent,
            self.descent,
            self.line_gap as u16,
            self.cap_height,
            self.x_height,
            self.underline_position as u16,
            self.underline_thickness,
            self.strikethrough_position as u16,
            self.strikethrough_thickness,
        ];
        for (i, v) in values.iter().enumerate() {
            out[2 * i..2 * i + 2].copy_from_slice(&v.to_le_bytes());
        }
        out
    }
}
/// DWRITE_GLYPH_METRICS, in design units.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GlyphMetrics {
    pub left_side_bearing: i32,
    pub advance_width: u32,
    pub right_side_bearing: i32,
    pub top_side_bearing: i32,
    pub advance_height: u32,
    pub bottom_side_bearing: i32,
    pub vertical_origin_y: i32,
}
impl GlyphMetrics {
    pub fn to_bytes(self) -> [u8; 28] {
        let mut out = [0u8; 28];
        let values = [
            self.left_side_bearing as u32,
            self.advance_width,
            self.right_side_bearing as u32,
            self.top_side_bearing as u32,
            self.advance_height,
            self.bottom_side_bearing as u32,
            self.vertical_origin_y as u32,
        ];
        for (i, v) in values.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        out
    }
}

/// The locale name DirectWrite reports for a name record's language.
fn locale(language: Option<&str>) -> String {
    let tag = language.unwrap_or("en-US");
    if tag.is_empty() || tag == "en" {
        String::from("en-us")
    } else {
        tag.to_ascii_lowercase()
    }
}
/// Every localized string of a name ID, (locale, text), English first.
pub fn names(font: &FontRef<'_>, id: u16) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for s in font.localized_strings(StringId::new(id)) {
        let text: String = s.chars().collect();
        let l = locale(s.language());
        if text.is_empty() || out.iter().any(|(o, _)| *o == l) {
            continue;
        }
        if l == "en-us" {
            out.insert(0, (l, text));
        } else {
            out.push((l, text));
        }
    }
    out
}
fn first_names(font: &FontRef<'_>, ids: &[u16]) -> Vec<(String, String)> {
    ids.iter()
        .map(|id| names(font, *id))
        .find(|n| !n.is_empty())
        .unwrap_or_default()
}
/// The CSS stretch (1..9) of an OS/2 width class or a `wdth` percentage.
fn stretch_of_width(percent: f32) -> u32 {
    const STEPS: [f32; 9] = [50.0, 62.5, 75.0, 87.5, 100.0, 112.5, 125.0, 150.0, 200.0];
    STEPS
        .iter()
        .enumerate()
        .min_by(|a, b| {
            (a.1 - percent)
                .abs()
                .partial_cmp(&(b.1 - percent).abs())
                .unwrap_or(core::cmp::Ordering::Equal)
        })
        .map_or(STRETCH_NORMAL, |(i, _)| i as u32 + 1)
}

/// What Analyze reports for file bytes: (file type, face type, faces).
/// DWRITE_FONT_FILE_TYPE: TRUETYPE 3, TRUETYPE_COLLECTION 4, CFF 2,
/// OPENTYPE_COLLECTION 4; DWRITE_FONT_FACE_TYPE: CFF 0, TRUETYPE 1,
/// OPENTYPE_COLLECTION 2.
pub fn analyze(bytes: &[u8]) -> Option<(u32, u32, u32)> {
    match FileRef::new(bytes).ok()? {
        FileRef::Font(f) => {
            let cff = f.table_data(Tag::new(b"CFF ")).is_some()
                || f.table_data(Tag::new(b"CFF2")).is_some();
            Some(if cff { (2, 0, 1) } else { (3, 1, 1) })
        }
        FileRef::Collection(c) => Some((4, 2, c.len())),
    }
}

/// The fonts of `file`: each face, and each named instance of a variable
/// face (deduplicated by weight, stretch and style).
pub fn faces(file: &Arc<File>) -> Vec<Face> {
    let count = analyze(file.bytes.get()).map_or(0, |(_, _, n)| n);
    let mut out = Vec::new();
    for index in 0..count.min(64) {
        let Ok(font) = FontRef::from_index(file.bytes.get(), index) else {
            continue;
        };
        if font.head().is_err() || font.maxp().is_err() || !font.charmap().has_map() {
            continue;
        }
        // DirectWrite family: WWS, typographic, then legacy names.
        let family = first_names(&font, &[21, 16, 1]);
        if family.is_empty() {
            continue;
        }
        let attributes = font.attributes();
        let os2 = font.os2().ok();
        let base_weight = attributes.weight.value().round() as u32;
        let base_style = match attributes.style {
            skrifa::attribute::Style::Normal => STYLE_NORMAL,
            skrifa::attribute::Style::Italic => STYLE_ITALIC,
            _ => STYLE_OBLIQUE,
        };
        let base_stretch = os2.as_ref().map_or(STRETCH_NORMAL, |o| {
            u32::from(o.us_width_class()).clamp(1, 9)
        });
        let instances = font.named_instances();
        let axes = font.axes();
        if instances.is_empty() || axes.is_empty() {
            out.push(Face {
                file: file.clone(),
                index,
                coords: Vec::new(),
                weight: base_weight.clamp(1, 999),
                stretch: base_stretch,
                style: base_style,
                simulations: 0,
                family,
                face_names: first_names(&font, &[22, 17, 2]),
            });
            continue;
        }
        let opsz = axes.get_by_tag(Tag::new(b"opsz"));
        for instance in instances.iter() {
            let coords: Vec<f32> = instance.user_coords().collect();
            let axis_value = |tag: &[u8; 4]| {
                axes.iter()
                    .position(|a| a.tag() == Tag::new(tag))
                    .and_then(|i| coords.get(i).copied())
            };
            // One optical size per family: the default.
            if let (Some(axis), Some(v)) = (&opsz, axis_value(b"opsz"))
                && (v - axis.default_value()).abs() > 0.01
            {
                continue;
            }
            let weight = axis_value(b"wght").map_or(base_weight, |w| w.round() as u32);
            let stretch = axis_value(b"wdth").map_or(base_stretch, stretch_of_width);
            let style = match axis_value(b"ital") {
                Some(i) if i >= 0.5 => STYLE_ITALIC,
                _ if axis_value(b"slnt").is_some_and(|s| s.abs() > 0.01) => STYLE_OBLIQUE,
                _ => base_style,
            };
            if out.iter().any(|f: &Face| {
                f.index == index && f.weight == weight && f.stretch == stretch && f.style == style
            }) {
                continue;
            }
            let mut location = alloc::vec![NormalizedCoord::default(); axes.len()];
            instance.location_to_slice(&mut location);
            let mut face_names = names(&font, instance.subfamily_name_id().to_u16());
            if face_names.is_empty() {
                face_names = first_names(&font, &[22, 17, 2]);
            }
            out.push(Face {
                file: file.clone(),
                index,
                coords: location,
                weight: weight.clamp(1, 999),
                stretch,
                style,
                simulations: 0,
                family: family.clone(),
                face_names,
            });
        }
    }
    out
}

/// Face `index` of `file` at its default location (CreateFontFace).
pub fn face_at(file: &Arc<File>, index: u32, simulations: u32) -> Option<Face> {
    let font = FontRef::from_index(file.bytes.get(), index).ok()?;
    font.head().ok()?;
    let attributes = font.attributes();
    Some(Face {
        file: file.clone(),
        index,
        coords: Vec::new(),
        weight: (attributes.weight.value().round() as u32).clamp(1, 999),
        stretch: font.os2().map_or(STRETCH_NORMAL, |o| {
            u32::from(o.us_width_class()).clamp(1, 9)
        }),
        style: match attributes.style {
            skrifa::attribute::Style::Normal => STYLE_NORMAL,
            skrifa::attribute::Style::Italic => STYLE_ITALIC,
            _ => STYLE_OBLIQUE,
        },
        simulations,
        family: first_names(&font, &[21, 16, 1]),
        face_names: first_names(&font, &[22, 17, 2]),
    })
}

impl Face {
    pub fn font(&self) -> Option<FontRef<'_>> {
        FontRef::from_index(self.file.bytes.get(), self.index).ok()
    }
    pub fn location(&self) -> LocationRef<'_> {
        LocationRef::new(&self.coords)
    }
    /// The same face with other simulations.
    pub fn simulated(&self, simulations: u32) -> Face {
        Face {
            file: self.file.clone(),
            index: self.index,
            coords: self.coords.clone(),
            weight: self.weight,
            stretch: self.stretch,
            style: self.style,
            simulations,
            family: self.family.clone(),
            face_names: self.face_names.clone(),
        }
    }
    pub fn family_name(&self) -> &str {
        self.family.first().map_or("", |(_, n)| n.as_str())
    }
    pub fn glyph_count(&self) -> u16 {
        self.font()
            .and_then(|f| f.maxp().ok())
            .map_or(0, |m| m.num_glyphs())
    }
    pub fn glyph(&self, code_point: u32) -> u16 {
        let Some(font) = self.font() else { return 0 };
        font.charmap()
            .map(code_point)
            .map_or(0, |g| g.to_u32().min(0xFFFF) as u16)
    }
    pub fn has(&self, code_point: u32) -> bool {
        self.glyph(code_point) != 0
    }
    /// A symbol font: its cmap maps the Windows symbol encoding (3, 0).
    pub fn is_symbol(&self) -> bool {
        self.font().and_then(|f| f.cmap().ok()).is_some_and(|c| {
            c.encoding_records().iter().any(|r| {
                r.platform_id() == skrifa::raw::tables::cmap::PlatformId::Windows
                    && r.encoding_id() == 0
            })
        })
    }
    pub fn table(&self, tag: [u8; 4]) -> Option<&[u8]> {
        let font = self.font()?;
        let data = font.table_data(Tag::new(&tag))?;
        let start = data.as_bytes().as_ptr() as usize - self.file.bytes.get().as_ptr() as usize;
        self.file.bytes.get().get(start..start + data.len())
    }
    pub fn metrics(&self) -> Metrics {
        let Some(font) = self.font() else {
            return Metrics::default();
        };
        let upem = font.head().map_or(2048, |h| h.units_per_em());
        let hhea = font.hhea().ok();
        let os2 = font.os2().ok();
        let post = font.post().ok();
        let (ascent, descent, line_gap) = match &os2 {
            Some(o) if o.fs_selection().bits() & 0x80 != 0 => (
                i32::from(o.s_typo_ascender()),
                -i32::from(o.s_typo_descender()),
                i32::from(o.s_typo_line_gap()),
            ),
            Some(o) => {
                let (a, d) = (i32::from(o.us_win_ascent()), i32::from(o.us_win_descent()));
                let gap = hhea.as_ref().map_or(0, |h| {
                    i32::from(h.ascender().to_i16()) - i32::from(h.descender().to_i16())
                        + i32::from(h.line_gap().to_i16())
                        - (a + d)
                });
                (a, d, gap.max(0))
            }
            None => hhea.as_ref().map_or((upem as i32, 0, 0), |h| {
                (
                    i32::from(h.ascender().to_i16()),
                    -i32::from(h.descender().to_i16()),
                    i32::from(h.line_gap().to_i16()),
                )
            }),
        };
        // Cap and x heights from OS/2 version 2, else from H and x.
        let measured = |c: char| {
            let gm = font.glyph_metrics(Size::unscaled(), self.location());
            font.charmap()
                .map(c)
                .and_then(|g| gm.bounds(g))
                .map_or(0, |b| b.y_max.round().max(0.0) as u16)
        };
        let cap = os2
            .as_ref()
            .and_then(|o| o.s_cap_height())
            .filter(|v| *v > 0)
            .map_or_else(|| measured('H'), |v| v as u16);
        let x = os2
            .as_ref()
            .and_then(|o| o.sx_height())
            .filter(|v| *v > 0)
            .map_or_else(|| measured('x'), |v| v as u16);
        let (underline_position, underline_thickness) =
            post.as_ref()
                .map_or((-(upem as i16) / 10, (upem / 14).max(1)), |p| {
                    (
                        p.underline_position().to_i16(),
                        (p.underline_thickness().to_i16().max(1)) as u16,
                    )
                });
        let (strike_position, strike_thickness) = os2
            .as_ref()
            .map_or(((x / 2) as i16, underline_thickness), |o| {
                (o.y_strikeout_position(), o.y_strikeout_size().max(1) as u16)
            });
        Metrics {
            units_per_em: upem,
            ascent: ascent.clamp(0, 0xFFFF) as u16,
            descent: descent.clamp(0, 0xFFFF) as u16,
            line_gap: line_gap.clamp(-0x8000, 0x7FFF) as i16,
            cap_height: cap,
            x_height: x,
            underline_position,
            underline_thickness,
            strikethrough_position: strike_position,
            strikethrough_thickness: strike_thickness,
        }
    }
    /// A glyph's design metrics (horizontal; vertical from the font's
    /// ascent and descent).
    pub fn glyph_metrics(&self, glyph: u16) -> GlyphMetrics {
        let Some(font) = self.font() else {
            return GlyphMetrics::default();
        };
        let metrics = self.metrics();
        let gm = font.glyph_metrics(Size::unscaled(), self.location());
        let id = GlyphId::new(u32::from(glyph));
        let advance = gm.advance_width(id).unwrap_or(0.0).round().max(0.0) as u32;
        let origin = i32::from(metrics.ascent);
        let advance_height = u32::from(metrics.ascent) + u32::from(metrics.descent);
        let mut out = GlyphMetrics {
            left_side_bearing: 0,
            advance_width: advance,
            right_side_bearing: advance as i32,
            top_side_bearing: origin,
            advance_height,
            bottom_side_bearing: advance_height as i32 - origin,
            vertical_origin_y: origin,
        };
        if let Some(b) = gm.bounds(id)
            && b.x_max > b.x_min
        {
            let (x0, x1) = (b.x_min.floor() as i32, b.x_max.ceil() as i32);
            let (y0, y1) = (b.y_min.floor() as i32, b.y_max.ceil() as i32);
            out.left_side_bearing = x0;
            out.right_side_bearing = advance as i32 - x1;
            out.top_side_bearing = origin - y1;
            out.bottom_side_bearing = advance_height as i32 - (origin - y0);
        }
        if self.simulations & SIMULATION_BOLD != 0 {
            let extra = bold_extra(metrics.units_per_em);
            out.advance_width += extra;
        }
        out
    }
    /// Informational strings (DWRITE_INFORMATIONAL_STRING_ID).
    pub fn informational(&self, id: u32) -> Vec<(String, String)> {
        let name_id: u16 = match id {
            1 => 0,
            2 => 5,
            3 => 7,
            4 => 8,
            5 => 9,
            6 => 12,
            7 => 10,
            8 => 11,
            9 => 13,
            10 => 14,
            11 => 1,
            12 => 2,
            13 => 16,
            14 => 17,
            15 => 19,
            16 => 4,
            17 => 6,
            18 => 20,
            _ => return Vec::new(),
        };
        let Some(font) = self.font() else {
            return Vec::new();
        };
        // A named instance's full and PostScript names.
        if id == 16 && !self.coords.is_empty() {
            let family = self.family_name();
            let face = self
                .face_names
                .first()
                .map_or("Regular", |(_, n)| n.as_str());
            let mut name = String::from(family);
            if face != "Regular" {
                name.push(' ');
                name.push_str(face);
            }
            return alloc::vec![(String::from("en-us"), name)];
        }
        names(&font, name_id)
    }
}
/// Bold simulation widens glyphs by about 1/64 em (DirectWrite's 2%).
pub fn bold_extra(units_per_em: u16) -> u32 {
    (u32::from(units_per_em) / 50).max(1)
}

/// A family: its names and its fonts.
pub struct Family {
    pub names: Vec<(String, String)>,
    pub fonts: Vec<Arc<Face>>,
}
/// A font collection.
#[derive(Default)]
pub struct Collection {
    pub families: Vec<Family>,
    /// Alternative family names (lower case) and the family they mean.
    pub aliases: Vec<(String, usize)>,
}
impl Collection {
    pub fn from_faces(faces: Vec<Face>) -> Self {
        let mut c = Collection::default();
        for face in faces {
            let key = face.family_name().to_ascii_lowercase();
            let i = match c.families.iter().position(|f| {
                f.names
                    .first()
                    .is_some_and(|(_, n)| n.to_ascii_lowercase() == key)
            }) {
                Some(i) => i,
                None => {
                    c.families.push(Family {
                        names: face.family.clone(),
                        fonts: Vec::new(),
                    });
                    c.families.len() - 1
                }
            };
            c.families[i].fonts.push(Arc::new(face));
        }
        for f in &mut c.families {
            f.fonts.sort_by_key(|x| (x.stretch, x.style, x.weight));
        }
        c
    }
    /// The family named `name` in any locale (case-insensitive), or the
    /// family an alias names.
    pub fn find(&self, name: &str) -> Option<usize> {
        let lower = name.to_ascii_lowercase();
        self.families
            .iter()
            .position(|f| f.names.iter().any(|(_, n)| n.to_ascii_lowercase() == lower))
            .or_else(|| {
                self.aliases
                    .iter()
                    .find(|(a, _)| *a == lower)
                    .map(|(_, i)| *i)
            })
    }
    pub fn alias(&mut self, name: &str, target: &str) {
        if self.find(name).is_none()
            && let Some(i) = self.find(target)
        {
            self.aliases.push((name.to_ascii_lowercase(), i));
        }
    }
    /// The family and position of a face (by file, index and location).
    pub fn locate(&self, face: &Face) -> Option<(usize, usize)> {
        self.families.iter().enumerate().find_map(|(fi, f)| {
            f.fonts
                .iter()
                .position(|x| {
                    Arc::ptr_eq(&x.file, &face.file)
                        && x.index == face.index
                        && x.coords == face.coords
                })
                .map(|i| (fi, i))
        })
    }
}

/// CSS font matching of `fonts` against a request: the fonts in order of
/// preference, with the simulations each needs.
pub fn matching(fonts: &[Arc<Face>], weight: u32, stretch: u32, style: u32) -> Vec<(usize, u32)> {
    let stretch_rank = |s: u32| -> u32 {
        if s == stretch {
            0
        } else if stretch <= STRETCH_NORMAL {
            // Narrower first, then wider.
            if s < stretch {
                stretch - s
            } else {
                10 + s - stretch
            }
        } else if s > stretch {
            s - stretch
        } else {
            10 + stretch - s
        }
    };
    let style_rank = |s: u32| -> u32 {
        match (style, s) {
            (a, b) if a == b => 0,
            (STYLE_ITALIC, STYLE_OBLIQUE) | (STYLE_OBLIQUE, STYLE_ITALIC) => 1,
            (_, STYLE_NORMAL) => 2,
            _ => 3,
        }
    };
    let weight_rank = |w: u32| -> u32 {
        let desired = weight.clamp(1, 999);
        if w == desired {
            return 0;
        }
        if (400..=500).contains(&desired) {
            // Up to 500, then lighter, then heavier.
            if w > desired && w <= 500 {
                w - desired
            } else if w < desired {
                1000 + desired - w
            } else {
                2000 + w - desired
            }
        } else if desired < 400 {
            if w < desired {
                desired - w
            } else {
                1000 + w - desired
            }
        } else if w > desired {
            w - desired
        } else {
            1000 + desired - w
        }
    };
    let mut order: Vec<usize> = (0..fonts.len()).collect();
    order.sort_by_key(|i| {
        let f = &fonts[*i];
        (
            stretch_rank(f.stretch),
            style_rank(f.style),
            weight_rank(f.weight),
        )
    });
    order
        .into_iter()
        .map(|i| {
            let f = &fonts[i];
            let mut simulations = 0;
            if weight >= 600 && f.weight + 150 <= weight && f.weight < 600 {
                simulations |= SIMULATION_BOLD;
            }
            if style != STYLE_NORMAL && f.style == STYLE_NORMAL {
                simulations |= SIMULATION_OBLIQUE;
            }
            (i, simulations)
        })
        .collect()
}
