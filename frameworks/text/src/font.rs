//! Caller-supplied font faces, roles and the Windows-like fallback chain.
//!
//! Bytes are borrowed for the process lifetime (`&'static [u8]`): the guest maps
//! font files as shared read-only sections, and host tests leak their buffers.
//! Nothing here embeds a font; every face is validated before publication.
use crate::{Error, Result};
use alloc::{string::String, sync::Arc, vec::Vec};
use core::sync::atomic::{AtomicU32, Ordering};
use skrifa::{
    FontRef, MetadataProvider,
    charmap::Charmap,
    color::ColorGlyphCollection,
    instance::Size,
    outline::OutlineGlyphCollection,
    raw::{FileRef, TableProvider},
    string::StringId,
};

/// Maximum faces in one set; a set is a fixed configuration, not a font database.
pub const MAX_FACES: usize = 64;

/// What a font file contributes to the fallback chain. A role may be given
/// several files (for example Segoe UI Regular and Semibold); the nearest
/// weight is selected. Collections (`.ttc`) select the role's UI family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FontRole {
    /// Segoe UI Variable: primary Latin, Greek and Cyrillic UI face (`wght`/`opsz`).
    UiVariable,
    /// Static Segoe UI faces: Arabic, Hebrew, Armenian, Georgian and Latin fallback.
    Ui,
    /// Segoe UI Symbol: arrows, technical, dingbats, Braille and text-style symbols.
    Symbol,
    /// Segoe UI Emoji: COLR/CPAL colour emoji.
    Emoji,
    /// Segoe UI Historic: historic scripts.
    Historic,
    /// Microsoft YaHei UI (Simplified Chinese).
    ChineseSimplified,
    /// Microsoft JhengHei UI (Traditional Chinese).
    ChineseTraditional,
    /// Yu Gothic UI (Japanese).
    Japanese,
    /// Malgun Gothic (Korean).
    Korean,
    /// Nirmala UI (Devanagari and other Indic scripts).
    Indic,
    /// Cascadia Mono (terminal cells).
    Monospace,
    /// Any other face, searched after the script-specific faces.
    Fallback,
}

impl FontRole {
    fn preferred_families(self) -> &'static [&'static str] {
        match self {
            Self::ChineseSimplified => &["Microsoft YaHei UI", "Microsoft YaHei"],
            Self::ChineseTraditional => &["Microsoft JhengHei UI", "Microsoft JhengHei"],
            Self::Japanese => &["Yu Gothic UI", "Yu Gothic"],
            Self::Korean => &["Malgun Gothic"],
            Self::Indic => &["Nirmala UI"],
            Self::UiVariable => &["Segoe UI Variable", "Segoe UI Variable Text"],
            Self::Ui => &["Segoe UI"],
            _ => &[],
        }
    }
}

/// Public read-only description of a loaded face.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceInfo {
    pub role: FontRole,
    pub family: String,
    pub collection_index: u32,
    pub weight: f32,
    pub italic: bool,
    pub variable: bool,
    pub color: bool,
    pub hinted: bool,
    pub glyph_count: u32,
    pub units_per_em: u16,
    /// Content identity from the sfnt table directory (tags, checksums, lengths),
    /// the file length and the collection index.
    pub fingerprint: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Axis {
    pub min: f32,
    pub default: f32,
    pub max: f32,
}
impl Axis {
    pub fn clamp(self, value: f32) -> f32 {
        value.clamp(self.min, self.max)
    }
}

pub(crate) struct Face {
    pub info: FaceInfo,
    pub font: FontRef<'static>,
    pub charmap: Charmap<'static>,
    pub outlines: OutlineGlyphCollection<'static>,
    pub colors: ColorGlyphCollection<'static>,
    pub shaper: harfrust::Font,
    pub wght: Option<Axis>,
    pub opsz: Option<Axis>,
    pub palette: Vec<[u8; 4]>,
}

impl Face {
    pub fn glyph(&self, ch: char) -> Option<u32> {
        self.charmap.map(ch).map(|g| g.to_u32()).filter(|&g| g != 0)
    }
    pub fn covers(&self, ch: char) -> bool {
        self.glyph(ch).is_some()
    }
    /// Distance from the requested weight; variable faces cover their axis.
    fn weight_distance(&self, weight: f32) -> f32 {
        match self.wght {
            Some(axis) => (axis.clamp(weight) - weight).abs(),
            None => {
                let w = self.info.weight;
                // CSS-like preference: heavier for bold requests, lighter otherwise.
                let bias = if (weight >= 500.0) == (w >= weight) {
                    0.0
                } else {
                    0.5
                };
                (w - weight).abs() + bias
            }
        }
    }
}

fn fnv(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
}

fn family_name(font: &FontRef<'_>) -> String {
    for id in [StringId::FAMILY_NAME, StringId::TYPOGRAPHIC_FAMILY_NAME] {
        if let Some(name) = font
            .localized_strings(id)
            .english_or_first()
            .map(|s| s.chars().collect::<String>())
            .filter(|s| !s.is_empty())
        {
            return name;
        }
    }
    String::new()
}

fn parse_face(data: &'static [u8], index: u32, role: FontRole) -> Result<Face> {
    let font = FontRef::from_index(data, index).map_err(|_| Error::InvalidFont)?;
    let head = font.head().map_err(|_| Error::InvalidFont)?;
    let upem = head.units_per_em();
    let glyph_count = font
        .maxp()
        .map_err(|_| Error::InvalidFont)?
        .num_glyphs()
        .into();
    font.hhea().map_err(|_| Error::InvalidFont)?;
    if !(16..=16384).contains(&upem) || glyph_count == 0 {
        return Err(Error::InvalidFont);
    }
    let charmap = font.charmap();
    if !charmap.has_map() {
        return Err(Error::InvalidFont);
    }
    let outlines = font.outline_glyphs();
    let colors = font.color_glyphs();
    let color = font.colr().is_ok() && font.cpal().is_ok();
    if outlines.format().is_none() && !color {
        return Err(Error::InvalidFont);
    }
    let mut fingerprint = 0xcbf2_9ce4_8422_2325u64;
    fnv(&mut fingerprint, &(data.len() as u64).to_le_bytes());
    fnv(&mut fingerprint, &index.to_le_bytes());
    for record in font.table_directory.table_records() {
        fnv(&mut fingerprint, &record.tag().to_be_bytes());
        fnv(&mut fingerprint, &record.checksum().to_le_bytes());
        fnv(&mut fingerprint, &record.length().to_le_bytes());
    }
    let attributes = font.attributes();
    let axis = |tag: &[u8; 4]| {
        font.axes()
            .get_by_tag(skrifa::Tag::new(tag))
            .map(|a| Axis {
                min: a.min_value(),
                default: a.default_value(),
                max: a.max_value(),
            })
            .filter(|a| a.min.is_finite() && a.max.is_finite() && a.min <= a.max)
    };
    let (wght, opsz) = (axis(b"wght"), axis(b"opsz"));
    let hinted = font.table_data(skrifa::Tag::new(b"fpgm")).is_some()
        || font.table_data(skrifa::Tag::new(b"prep")).is_some();
    let metrics = font.metrics(Size::unscaled(), skrifa::instance::LocationRef::default());
    let (ascent, descent) = (metrics.ascent, -metrics.descent);
    if !ascent.is_finite() || !descent.is_finite() || ascent <= 0.0 {
        return Err(Error::InvalidFont);
    }
    let shaper = harfrust::Font::new(data, index).ok_or(Error::InvalidFont)?;
    let palette = font
        .color_palettes()
        .get(0)
        .map(|p| {
            p.colors()
                .iter()
                .map(|c| [c.blue, c.green, c.red, c.alpha])
                .collect()
        })
        .unwrap_or_default();
    Ok(Face {
        info: FaceInfo {
            role,
            family: family_name(&font),
            collection_index: index,
            weight: wght.map_or(attributes.weight.value(), |a| a.default),
            italic: !matches!(attributes.style, skrifa::attribute::Style::Normal),
            variable: wght.is_some(),
            color,
            hinted,
            glyph_count,
            units_per_em: upem,
            fingerprint,
        },
        charmap,
        outlines,
        colors,
        font,
        shaper,
        wght,
        opsz,
        palette,
    })
}

/// Supplies font files for a role on first use (for example by mapping a
/// separate shared read-only section only when CJK text or emoji appears).
/// Each role is requested at most once per `FontSet`.
pub trait FontProvider: Send + Sync {
    /// Files for `role`, mapped for the process lifetime; empty if unavailable.
    fn load(&self, role: FontRole) -> Vec<&'static [u8]>;
}

fn parse_file(data: &'static [u8], role: FontRole, faces: &mut Vec<Face>) -> Result<()> {
    let count = match FileRef::new(data).map_err(|_| Error::InvalidFont)? {
        FileRef::Font(_) => 1,
        FileRef::Collection(collection) => collection.len(),
    };
    if count == 0 || count > MAX_FACES as u32 {
        return Err(Error::InvalidFont);
    }
    if count == 1 {
        faces.push(parse_face(data, 0, role)?);
        return Ok(());
    }
    let mut parsed = Vec::new();
    for index in 0..count {
        parsed.push(parse_face(data, index, role)?);
    }
    let preferred = role
        .preferred_families()
        .iter()
        .find(|family| parsed.iter().any(|f| f.info.family == **family));
    match preferred {
        Some(family) => faces.extend(parsed.into_iter().filter(|f| f.info.family == *family)),
        None => faces.push(parsed.swap_remove(0)),
    }
    Ok(())
}

/// A snapshot of the loaded faces (append-only, so indices stay valid).
#[derive(Clone)]
pub(crate) struct Faces(pub Vec<Arc<Face>>);
impl core::ops::Deref for Faces {
    type Target = [Arc<Face>];
    fn deref(&self) -> &[Arc<Face>] {
        &self.0
    }
}
impl Faces {
    /// Best face of a role for a weight (nearest weight, upright preferred).
    pub fn role_face(&self, role: FontRole, weight: f32) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for (index, face) in self.0.iter().enumerate() {
            if face.info.role != role {
                continue;
            }
            let score = face.weight_distance(weight) + if face.info.italic { 1000.0 } else { 0.0 };
            if best.is_none_or(|(_, s)| score < s) {
                best = Some((index, score));
            }
        }
        best.map(|(i, _)| i)
    }
}

/// A validated set of faces: those supplied at construction plus roles a
/// `FontProvider` contributes on first use. Faces are only ever appended.
pub struct FontSet {
    faces: spin::Mutex<Faces>,
    identity: u64,
    provider: Option<alloc::boxed::Box<dyn FontProvider>>,
    attempted: AtomicU32,
}

impl FontSet {
    /// Load each file for its role. Collections contribute the faces whose
    /// family matches the role's preferred UI family (or their first face).
    /// Every face is parsed and validated before the set is published;
    /// malformed bytes fail with `Error::InvalidFont`, and an empty set too.
    pub fn from_files(files: &[(&'static [u8], FontRole)]) -> Result<Self> {
        let mut faces = Vec::new();
        for &(data, role) in files {
            parse_file(data, role, &mut faces)?;
        }
        if faces.is_empty() || faces.len() > MAX_FACES {
            return Err(Error::InvalidFont);
        }
        let mut identity = 0xcbf2_9ce4_8422_2325u64;
        for face in &faces {
            fnv(&mut identity, &face.info.fingerprint.to_le_bytes());
            fnv(&mut identity, &[face.info.role as u8]);
        }
        Ok(Self {
            faces: spin::Mutex::new(Faces(faces.into_iter().map(Arc::new).collect())),
            identity,
            provider: None,
            attempted: AtomicU32::new(0),
        })
    }
    /// Add a provider for roles not supplied at construction.
    pub fn with_provider(mut self, provider: alloc::boxed::Box<dyn FontProvider>) -> Self {
        self.provider = Some(provider);
        self
    }
    /// List the faces of a font file or collection as (index, family, weight),
    /// to choose which file supplies a role.
    pub fn describe_file(data: &'static [u8]) -> Result<Vec<(u32, String, f32)>> {
        let count = match FileRef::new(data).map_err(|_| Error::InvalidFont)? {
            FileRef::Font(_) => 1,
            FileRef::Collection(collection) => collection.len().min(MAX_FACES as u32),
        };
        (0..count)
            .map(|index| {
                parse_face(data, index, FontRole::Fallback)
                    .map(|f| (index, f.info.family, f.info.weight))
            })
            .collect()
    }
    pub(crate) fn snapshot(&self) -> Faces {
        self.faces.lock().clone()
    }
    /// Load a role through the provider once; true if faces were added.
    pub(crate) fn ensure(&self, role: FontRole) -> bool {
        let bit = 1u32 << (role as u32);
        let Some(provider) = &self.provider else {
            return false;
        };
        if self.attempted.fetch_or(bit, Ordering::AcqRel) & bit != 0 {
            return false;
        }
        let mut added = Vec::new();
        for data in provider.load(role) {
            // A malformed provided file is skipped; supplied faces stay usable.
            let mut parsed = Vec::new();
            if parse_file(data, role, &mut parsed).is_ok() {
                added.extend(parsed);
            }
        }
        if added.is_empty() {
            return false;
        }
        let mut faces = self.faces.lock();
        for face in added {
            if faces.0.len() < MAX_FACES {
                faces.0.push(Arc::new(face));
            }
        }
        true
    }
    /// Number of faces loaded so far.
    pub fn len(&self) -> usize {
        self.faces.lock().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn face_info(&self, index: usize) -> Option<FaceInfo> {
        self.faces.lock().get(index).map(|f| f.info.clone())
    }
    /// Descriptions of the faces loaded so far.
    pub fn faces(&self) -> Vec<FaceInfo> {
        self.faces.lock().iter().map(|f| f.info.clone()).collect()
    }
    /// Identity of the faces supplied at construction and their roles.
    pub fn identity(&self) -> u64 {
        self.identity
    }
    /// True if any loaded face maps the character to a real glyph.
    pub fn covers(&self, ch: char) -> bool {
        self.faces.lock().iter().any(|f| f.covers(ch))
    }
}

/// Which family a format starts from before script fallback.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Family {
    /// Segoe UI Variable, then static Segoe UI.
    #[default]
    Ui,
    /// Cascadia Mono, then the UI chain.
    Monospace,
}

/// BCP 47 language tag (up to 15 bytes) used for CJK font choice and shaping
/// language, for example `en-US`, `zh-Hans`, `zh-TW`, `ja`, `ko`, `hi`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Locale {
    bytes: [u8; 15],
    len: u8,
}
impl core::fmt::Debug for Locale {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}
impl Default for Locale {
    fn default() -> Self {
        Self::new("en-US")
    }
}
impl Locale {
    /// Over-long or non-ASCII tags fall back to `und`.
    pub fn new(tag: &str) -> Self {
        let mut bytes = [0u8; 15];
        if tag.is_empty()
            || tag.len() > 15
            || !tag
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            bytes[..3].copy_from_slice(b"und");
            return Self { bytes, len: 3 };
        }
        for (out, b) in bytes.iter_mut().zip(tag.bytes()) {
            *out = if b == b'_' { b'-' } else { b };
        }
        Self {
            bytes,
            len: tag.len() as u8,
        }
    }
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("und")
    }
    fn subtag(&self, index: usize) -> &str {
        self.as_str().split('-').nth(index).unwrap_or("")
    }
    fn language_is(&self, language: &str) -> bool {
        self.subtag(0).eq_ignore_ascii_case(language)
    }
    fn has_subtag(&self, wanted: &str) -> bool {
        self.as_str()
            .split('-')
            .skip(1)
            .any(|s| s.eq_ignore_ascii_case(wanted))
    }
    /// Preferred Han face role for this locale.
    pub(crate) fn han_role(&self) -> FontRole {
        if self.language_is("ja") {
            FontRole::Japanese
        } else if self.language_is("ko") {
            FontRole::Korean
        } else if self.language_is("zh")
            && (self.has_subtag("Hant") || ["TW", "HK", "MO"].iter().any(|r| self.has_subtag(r)))
        {
            FontRole::ChineseTraditional
        } else {
            FontRole::ChineseSimplified
        }
    }
    pub(crate) fn harfrust(&self) -> Option<harfrust::Language> {
        self.as_str().parse().ok()
    }
}

/// Roles to try for one cluster, in order (Windows-like system fallback).
pub(crate) fn fallback_roles(
    family: Family,
    script: unicode_script::Script,
    locale: Locale,
    emoji: bool,
) -> Vec<FontRole> {
    use FontRole::*;
    use unicode_script::Script as S;
    let mut roles: Vec<FontRole> = Vec::with_capacity(14);
    let mut push = |role: FontRole| {
        if !roles.contains(&role) {
            roles.push(role);
        }
    };
    if emoji {
        push(Emoji);
    }
    if family == Family::Monospace {
        push(Monospace);
    }
    push(UiVariable);
    push(Ui);
    let han = locale.han_role();
    match script {
        S::Han => {
            push(han);
            for role in [ChineseSimplified, Japanese, ChineseTraditional, Korean] {
                push(role);
            }
        }
        S::Hiragana | S::Katakana => {
            push(Japanese);
            push(han);
            push(ChineseSimplified);
        }
        S::Hangul => push(Korean),
        S::Bopomofo => {
            push(ChineseTraditional);
            push(ChineseSimplified);
        }
        S::Devanagari
        | S::Bengali
        | S::Gurmukhi
        | S::Gujarati
        | S::Oriya
        | S::Tamil
        | S::Telugu
        | S::Kannada
        | S::Malayalam
        | S::Sinhala
        | S::Ol_Chiki
        | S::Sora_Sompeng
        | S::Chakma
        | S::Meetei_Mayek => push(Indic),
        _ => {}
    }
    push(Symbol);
    if matches!(script, S::Common | S::Inherited | S::Unknown) {
        push(han);
        push(Japanese);
        push(Korean);
    }
    push(Historic);
    push(Indic);
    push(Fallback);
    push(Emoji);
    roles
}
