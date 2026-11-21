//! Paragraph layout: bidi (UAX #9), script itemization, per-cluster font
//! fallback, shaping, line breaking (UAX #14), visual reordering, the cluster
//! map, hit testing, carets with affinity and bidi selection rectangles.
//!
//! Geometry is computed in physical pixels at `format.scale`; the public
//! query API speaks logical pixels (physical / scale) relative to the layout's
//! top-left line-box corner.
use crate::analysis::{self, Break, cluster_is_emoji, is_default_ignorable, shaping_script};
use crate::font::{Face, Faces, Family, FontRole, FontSet, Locale, fallback_roles};
use crate::raster::RasterMode;
use crate::shape::ShapeRequest;
use crate::system::Caches;
use crate::{Error, Result, TextStyle};
use alloc::vec::Vec;
use core::ops::Range;
#[allow(unused_imports)]
use core_maths::CoreFloat;
use unicode_bidi::{Level, ParagraphBidiInfo};
use unicode_script::Script;
use unicode_segmentation::UnicodeSegmentation;

/// Largest text accepted by one layout.
pub const MAX_LAYOUT_BYTES: usize = 1 << 20;
/// Bytes of neighbouring text given to the shaper as context.
const CONTEXT_CHARS: usize = 5;

/// Font size, weight, scale and rendering options for a layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextFormat {
    /// Em size in logical pixels (Segoe UI "14 px" body text is 14.0).
    pub size: f32,
    /// OpenType weight class (400 regular, 600 semibold, 700 bold).
    pub weight: u16,
    /// Line box height in logical pixels; `None` uses the primary font's
    /// ascent + descent + line gap.
    pub line_height: Option<f32>,
    /// Device pixels per logical pixel (1.0, 1.25, 1.5, 1.75, 2.0, ...).
    pub scale: f32,
    pub family: Family,
    pub locale: Locale,
    pub raster: RasterMode,
    /// Apply TrueType hinting (Skrifa interpreter) for hinted faces.
    pub hinting: bool,
    /// Place glyphs at third-pixel phases instead of whole pixels.
    pub subpixel_positioning: bool,
}

impl TextFormat {
    pub fn new(size: f32) -> Self {
        Self {
            size,
            weight: 400,
            line_height: None,
            scale: 1.0,
            family: Family::Ui,
            locale: Locale::default(),
            raster: RasterMode::Lcd,
            hinting: true,
            subpixel_positioning: false,
        }
    }
    /// The format matching one of the atlas styles (identical size, weight and
    /// line box) at a display scale.
    pub fn from_style(style: TextStyle, scale: f32) -> Self {
        Self {
            weight: if matches!(
                style,
                TextStyle::Semibold14
                    | TextStyle::Semibold16
                    | TextStyle::Semibold20
                    | TextStyle::Semibold28
            ) {
                600
            } else {
                400
            },
            line_height: Some(style.line_height() as f32),
            scale,
            ..Self::new(style.pixel_size() as f32)
        }
    }
    pub fn with_weight(mut self, weight: u16) -> Self {
        self.weight = weight;
        self
    }
    pub fn with_scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }
    pub fn with_locale(mut self, locale: Locale) -> Self {
        self.locale = locale;
        self
    }
    pub fn with_line_height(mut self, line_height: f32) -> Self {
        self.line_height = Some(line_height);
        self
    }
    pub fn with_family(mut self, family: Family) -> Self {
        self.family = family;
        self
    }
    pub fn with_raster(mut self, raster: RasterMode) -> Self {
        self.raster = raster;
        self
    }
    pub(crate) fn ppem(&self) -> f32 {
        self.size * self.scale
    }
    pub(crate) fn validate(&self) -> Result<()> {
        let ok = self.size.is_finite()
            && self.size > 0.0
            && self.scale.is_finite()
            && (0.25..=8.0).contains(&self.scale)
            && self.ppem() <= 512.0
            && (1..=1000).contains(&self.weight)
            && self
                .line_height
                .is_none_or(|h| h.is_finite() && h > 0.0 && h * self.scale <= 4096.0);
        if ok {
            Ok(())
        } else {
            Err(Error::InvalidFormat)
        }
    }
}

/// Line wrapping policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Wrap {
    /// Only explicit line breaks end lines.
    None,
    /// UAX #14 opportunities; an unbreakable word wider than the line breaks
    /// at grapheme boundaries.
    #[default]
    Word,
    /// Any grapheme boundary.
    Character,
}

/// Paragraph base direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BaseDirection {
    /// First strong character (UAX #9 P2/P3), left-to-right if none.
    #[default]
    Auto,
    Ltr,
    Rtl,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    /// Left for left-to-right paragraphs, right for right-to-left.
    #[default]
    Start,
    End,
    Left,
    Right,
    Center,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutOptions {
    /// Wrapping and alignment width in logical pixels.
    pub max_width: Option<f32>,
    pub wrap: Wrap,
    pub direction: BaseDirection,
    pub align: Align,
    pub max_lines: Option<u32>,
    /// Truncate overflowing text with U+2026 at the logical end of the last line.
    pub ellipsis: bool,
    /// Tab stops every `tab_spaces` space advances of the primary face.
    pub tab_spaces: u8,
}
impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            max_width: None,
            wrap: Wrap::Word,
            direction: BaseDirection::Auto,
            align: Align::Start,
            max_lines: None,
            ellipsis: false,
            tab_spaces: 4,
        }
    }
}

/// Which side of a boundary a caret belongs to when the two sides are
/// visually apart (bidi boundaries and wrapped line ends).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Affinity {
    /// Attached to the character before the offset (its trailing edge).
    Upstream,
    /// Attached to the character after the offset (its leading edge).
    #[default]
    Downstream,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Caret {
    pub x: f32,
    pub top: f32,
    pub height: f32,
    pub line: usize,
    /// Direction of the character the caret is attached to.
    pub rtl: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HitTest {
    pub offset: usize,
    pub affinity: Affinity,
    pub line: usize,
    /// True if the point lies inside a cluster's box.
    pub inside: bool,
}

/// One grapheme cluster of the cluster map (logical order).
#[derive(Clone, Debug, PartialEq)]
pub struct ClusterInfo {
    pub text: Range<usize>,
    pub line: usize,
    /// Visual left edge and width in logical pixels.
    pub x: f32,
    pub width: f32,
    pub rtl: bool,
    /// Glyph indices (into `TextLayout::glyphs`) shaped for this cluster's
    /// shaping cluster (shared by every grapheme of a ligature).
    pub glyphs: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LineInfo {
    pub text: Range<usize>,
    pub top: f32,
    pub height: f32,
    pub baseline: f32,
    pub x: f32,
    /// Width without trailing whitespace.
    pub width: f32,
    pub rtl: bool,
}

/// A positioned glyph (for inspection, accessibility and tests).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphInfo {
    pub face: usize,
    pub glyph: u32,
    /// Pen position in logical pixels relative to the layout origin (baseline y).
    pub x: f32,
    pub y: f32,
    pub cluster: usize,
    pub rtl: bool,
}

/// Size of a layout in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutMetrics {
    pub width: f32,
    pub height: f32,
    pub lines: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Run {
    pub face: u16,
    pub wght: f32,
    pub opsz: f32,
    pub embolden: bool,
    pub rtl: bool,
    pub glyphs: Range<u32>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Glyph {
    pub id: u32,
    /// Physical pen x relative to the line origin (includes the shaper offset).
    pub x: f32,
    /// Physical offset below the baseline.
    pub y: f32,
    pub cluster: u32,
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Line {
    pub text: Range<usize>,
    pub runs: Range<u32>,
    pub x: f32,
    pub width: f32,
    pub top: f32,
    pub rtl: bool,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Cluster {
    pub start: u32,
    pub end: u32,
    pub line: u32,
    pub x: f32,
    pub width: f32,
    pub rtl: bool,
    pub glyphs: (u32, u32),
}

/// An immutable laid-out text. Draw it with the `TextSystem` that built it.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLayout {
    pub(crate) identity: u64,
    pub(crate) format: TextFormat,
    pub(crate) text_len: usize,
    pub(crate) line_height: f32,
    pub(crate) baseline: f32,
    pub(crate) width: f32,
    pub(crate) box_width: f32,
    pub(crate) truncated: bool,
    pub(crate) lines: Vec<Line>,
    pub(crate) runs: Vec<Run>,
    pub(crate) glyphs: Vec<Glyph>,
    pub(crate) clusters: Vec<Cluster>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Text,
    Tab,
    Control,
}

#[derive(Clone, Debug)]
struct Item {
    range: Range<usize>,
    face: u16,
    script: Script,
    level: Level,
    kind: Kind,
    embolden: bool,
}

struct Grapheme {
    range: Range<usize>,
    item: u32,
    width: f32,
    space: bool,
}

/// Per-layout constant data.
pub(crate) struct Context<'a> {
    pub set: &'a FontSet,
    pub faces: Faces,
    pub format: TextFormat,
    pub options: LayoutOptions,
    pub ppem: f32,
    pub primary: u16,
    pub tab: f32,
    chains: Vec<(Script, bool, Vec<FontRole>)>,
}

impl Context<'_> {
    fn chain(&mut self, script: Script, emoji: bool) -> Vec<FontRole> {
        if let Some(c) = self.chains.iter().find(|c| c.0 == script && c.1 == emoji) {
            return c.2.clone();
        }
        let roles = fallback_roles(self.format.family, script, self.format.locale, emoji);
        self.chains.push((script, emoji, roles.clone()));
        roles
    }
    /// Faces of a role (nearest weight first), loading a provided role lazily.
    fn role_faces(&mut self, role: FontRole) -> Vec<u16> {
        if !self.faces.iter().any(|f| f.info.role == role) && self.set.ensure(role) {
            self.faces = self.set.snapshot();
        }
        let weight = f32::from(self.format.weight);
        let mut out: Vec<u16> = self
            .faces
            .role_face(role, weight)
            .map(|f| f as u16)
            .into_iter()
            .collect();
        if role == FontRole::Fallback {
            let extra: Vec<u16> = self
                .faces
                .iter()
                .enumerate()
                .filter(|(i, f)| f.info.role == role && !out.contains(&(*i as u16)))
                .map(|(i, _)| i as u16)
                .collect();
            out.extend(extra);
        }
        out
    }
    /// Variation for a face: `wght` and `opsz` = logical size in points.
    pub fn variation(&self, face: u16) -> (f32, f32) {
        let f = &self.faces[usize::from(face)];
        let wght = f
            .wght
            .map_or(0.0, |a| a.clamp(f32::from(self.format.weight)));
        let opsz = f.opsz.map_or(0.0, |a| a.clamp(self.format.size * 0.75));
        (wght, opsz)
    }
    fn embolden(&self, face: u16) -> bool {
        let f = &self.faces[usize::from(face)];
        self.format.weight >= 600 && f.wght.is_none() && f.info.weight < 550.0 && !f.info.color
    }
}

fn covers(face: &Face, cluster: &str, all: bool) -> bool {
    let mut chars = cluster
        .chars()
        .filter(|c| !is_default_ignorable(*c) && !c.is_control());
    if all {
        chars.all(|c| face.covers(c))
    } else {
        chars.next().is_none_or(|c| face.covers(c))
    }
}

/// First face of the cluster's fallback chain that maps every character
/// (keeping the previous fallback face for runs such as CJK punctuation),
/// else the first that maps its base character, else the primary face.
fn select_face(
    ctx: &mut Context<'_>,
    cluster: &str,
    script: Script,
    emoji: bool,
    previous: Option<u16>,
) -> u16 {
    let roles = ctx.chain(script, emoji);
    let mut base_only: Option<u16> = None;
    for (step, role) in roles.iter().enumerate() {
        for face in ctx.role_faces(*role) {
            let f = &ctx.faces[usize::from(face)];
            if covers(f, cluster, true) {
                return face;
            }
            if base_only.is_none() && covers(f, cluster, false) {
                base_only = Some(face);
            }
        }
        if step == 0
            && !emoji
            && let Some(previous) = previous
            && covers(&ctx.faces[usize::from(previous)], cluster, true)
        {
            return previous;
        }
    }
    // Faces outside the chain (for example a second weight of a role).
    for (index, f) in ctx.faces.iter().enumerate() {
        if covers(f, cluster, true) {
            return index as u16;
        }
    }
    base_only.unwrap_or(ctx.primary)
}

fn floor_boundary(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}
fn context_before(text: &str, at: usize) -> &str {
    let start = text[..at]
        .char_indices()
        .rev()
        .nth(CONTEXT_CHARS - 1)
        .map_or(0, |(i, _)| i);
    &text[start..at]
}
fn context_after(text: &str, at: usize) -> &str {
    let end = text[at..]
        .char_indices()
        .nth(CONTEXT_CHARS)
        .map_or(text.len(), |(i, _)| at + i);
    &text[at..floor_boundary(text, end)]
}

/// A shaped piece: glyph output plus its scale to physical pixels.
struct Piece {
    glyphs: Vec<crate::shape::ShapedGlyph>,
    scale: f32,
    face: u16,
    wght: f32,
    opsz: f32,
}

fn shape_piece(
    ctx: &Context<'_>,
    caches: &mut Caches,
    para: &str,
    range: Range<usize>,
    face: u16,
    script: Script,
    rtl: bool,
) -> Piece {
    let (wght, opsz) = ctx.variation(face);
    let f = &ctx.faces[usize::from(face)];
    let instance = caches.instance(face, f, wght, opsz);
    let request = ShapeRequest {
        face: usize::from(face),
        fingerprint: f.info.fingerprint,
        instance: instance.id,
        text: &para[range.clone()],
        pre: context_before(para, range.start),
        post: context_after(para, range.end),
        rtl,
        script: shaping_script(script),
        locale: ctx.format.locale,
        bot: range.start == 0,
        eot: range.end == para.len(),
    };
    let glyphs = caches.shapes.shape(f, &instance.font, &request).to_vec();
    Piece {
        glyphs,
        scale: ctx.ppem / f32::from(f.info.units_per_em),
        face,
        wght,
        opsz,
    }
}

/// Split a piece's glyphs into shaping clusters: (cluster start, glyph range,
/// advance) in visual order.
fn cluster_groups(piece: &Piece) -> Vec<(u32, Range<usize>, f32)> {
    let mut groups: Vec<(u32, Range<usize>, f32)> = Vec::new();
    for (index, glyph) in piece.glyphs.iter().enumerate() {
        let advance = glyph.advance as f32 * piece.scale;
        match groups.last_mut() {
            Some(last) if last.0 == glyph.cluster => {
                last.1.end = index + 1;
                last.2 += advance;
            }
            _ => groups.push((glyph.cluster, index..index + 1, advance)),
        }
    }
    groups
}

/// Width of each grapheme of `graphemes` (relative offsets within the piece),
/// splitting ligature advances equally among the graphemes they cover.
fn grapheme_widths(piece: &Piece, starts: &[usize], len: usize) -> Vec<f32> {
    let mut widths = alloc::vec![0.0f32; starts.len()];
    let groups = cluster_groups(piece);
    let mut sorted: Vec<(u32, f32)> = groups.iter().map(|g| (g.0, g.2)).collect();
    sorted.sort_by_key(|g| g.0);
    for (i, &(start, advance)) in sorted.iter().enumerate() {
        let end = sorted.get(i + 1).map_or(len as u32, |g| g.0);
        let covered: Vec<usize> = (0..starts.len())
            .filter(|&g| starts[g] as u32 >= start && (starts[g] as u32) < end.max(start + 1))
            .collect();
        if covered.is_empty() {
            // Cluster starting inside a grapheme: attribute to the containing one.
            if let Some(g) = starts.iter().rposition(|&s| s as u32 <= start) {
                widths[g] += advance;
            }
            continue;
        }
        let share = advance / covered.len() as f32;
        for g in covered {
            widths[g] += share;
        }
    }
    widths
}

pub(crate) fn build(
    set: &FontSet,
    caches: &mut Caches,
    text: &str,
    format: &TextFormat,
    options: &LayoutOptions,
) -> Result<TextLayout> {
    format.validate()?;
    if text.len() > MAX_LAYOUT_BYTES {
        return Err(Error::TextTooLong);
    }
    if options.max_width.is_some_and(|w| !w.is_finite() || w < 0.0) {
        return Err(Error::InvalidFormat);
    }
    let ppem = format.ppem();
    let primary_role = if format.family == Family::Monospace {
        FontRole::Monospace
    } else {
        FontRole::UiVariable
    };
    let faces = set.snapshot();
    let primary = faces
        .role_face(primary_role, f32::from(format.weight))
        .or_else(|| faces.role_face(FontRole::Ui, f32::from(format.weight)))
        .unwrap_or(0) as u16;
    let mut ctx = Context {
        set,
        faces,
        format: *format,
        options: *options,
        ppem,
        primary,
        tab: 0.0,
        chains: Vec::new(),
    };
    // Vertical metrics from the primary face at its instance location.
    let (wght, opsz) = ctx.variation(primary);
    let face = ctx.faces[usize::from(primary)].clone();
    let instance = caches.instance(primary, &face, wght, opsz);
    let metrics = skrifa::MetadataProvider::metrics(
        &face.font,
        skrifa::instance::Size::new(ppem),
        skrifa::instance::LocationRef::new(&instance.coords),
    );
    let baseline = metrics.ascent.ceil().max(1.0);
    let line_height = match format.line_height {
        Some(h) => (h * format.scale).round().max(1.0),
        None => (metrics.ascent - metrics.descent + metrics.leading)
            .round()
            .max(1.0),
    };
    let space = face.glyph(' ').map_or(ppem * 0.25, |g| {
        let shaper = harfrust::ShaperFont::new(&instance.font);
        shaper.glyph_h_advance(harfrust::GlyphId::new(g)) as f32 * ppem
            / f32::from(face.info.units_per_em)
    });
    ctx.tab = (space * f32::from(options.tab_spaces.max(1))).max(1.0);
    let max_width = options.max_width.map(|w| w * format.scale);

    let mut layout = TextLayout {
        identity: set.identity(),
        format: *format,
        text_len: text.len(),
        line_height,
        baseline,
        width: 0.0,
        box_width: 0.0,
        truncated: false,
        lines: Vec::new(),
        runs: Vec::new(),
        glyphs: Vec::new(),
        clusters: Vec::new(),
    };
    // Paragraphs separated by CR LF, LF, CR, U+2028 and U+2029.
    let mut start = 0;
    let mut paragraphs = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < text.len() {
        let ch = text[index..].chars().next().expect("char boundary");
        let len = ch.len_utf8();
        if matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
            paragraphs.push(start..index);
            index += if ch == '\r' && bytes.get(index + 1) == Some(&b'\n') {
                2
            } else {
                len
            };
            start = index;
        } else {
            index += len;
        }
    }
    paragraphs.push(start..text.len());
    let max_lines = options.max_lines.map_or(usize::MAX, |n| n.max(1) as usize);
    for (number, range) in paragraphs.iter().enumerate() {
        if layout.lines.len() >= max_lines {
            layout.truncated = true;
            break;
        }
        let last_paragraph = number + 1 == paragraphs.len();
        paragraph(
            &mut ctx,
            caches,
            &mut layout,
            text,
            range.clone(),
            max_width,
            max_lines,
            last_paragraph,
        )?;
    }
    if layout.lines.len() >= max_lines && paragraphs.len() > 1 {
        // Content after the last kept line was dropped.
        let kept_end = layout.lines.last().map_or(0, |l| l.text.end);
        if kept_end < paragraphs.last().map_or(0, |p| p.end) && !layout.truncated {
            layout.truncated = true;
        }
    }
    let width = layout.lines.iter().fold(0.0f32, |w, l| w.max(l.width));
    layout.width = width;
    layout.box_width = max_width.unwrap_or(width);
    // Alignment.
    for line in layout.lines.iter_mut() {
        let free = layout.box_width - line.width;
        let left = match (options.align, line.rtl) {
            (Align::Left, _) | (Align::Start, false) | (Align::End, true) => 0.0,
            (Align::Right, _) | (Align::Start, true) | (Align::End, false) => free,
            (Align::Center, _) => free / 2.0,
        };
        line.x += left;
    }
    Ok(layout)
}

#[allow(clippy::too_many_arguments)]
fn paragraph(
    ctx: &mut Context<'_>,
    caches: &mut Caches,
    layout: &mut TextLayout,
    text: &str,
    range: Range<usize>,
    max_width: Option<f32>,
    max_lines: usize,
    last_paragraph: bool,
) -> Result<()> {
    let base = range.start;
    let para = &text[range.clone()];
    let level = match ctx.options.direction {
        BaseDirection::Auto => None,
        BaseDirection::Ltr => Some(Level::ltr()),
        BaseDirection::Rtl => Some(Level::rtl()),
    };
    let bidi = ParagraphBidiInfo::new(para, level);
    let rtl = bidi.paragraph_level.is_rtl();
    if para.is_empty() {
        layout.lines.push(Line {
            text: base..base,
            runs: layout.runs.len() as u32..layout.runs.len() as u32,
            x: 0.0,
            width: 0.0,
            top: layout.lines.len() as f32 * layout.line_height,
            rtl,
        });
        return Ok(());
    }
    // Graphemes, scripts and fallback items.
    let scripts = analysis::script_runs(para);
    let mut script_at = scripts.iter().peekable();
    let mut items: Vec<Item> = Vec::new();
    let mut graphemes: Vec<Grapheme> = Vec::new();
    let mut previous_face: Option<u16> = None;
    for (start, cluster) in para.grapheme_indices(true) {
        let end = start + cluster.len();
        while script_at.peek().is_some_and(|s| s.0.end <= start) {
            script_at.next();
        }
        let script = script_at.peek().map_or(Script::Common, |s| s.1);
        let first = cluster.chars().next().unwrap_or(' ');
        let kind = if first == '\t' {
            Kind::Tab
        } else if cluster
            .chars()
            .all(|c| c.is_control() || is_default_ignorable(c))
            && first != '\u{200D}'
        {
            Kind::Control
        } else {
            Kind::Text
        };
        let face = match kind {
            Kind::Text => {
                let emoji = cluster_is_emoji(cluster);
                // Common case: the primary face maps the whole cluster.
                let face = if !emoji
                    && ctx.format.family == Family::Ui
                    && covers(&ctx.faces[usize::from(ctx.primary)], cluster, true)
                {
                    ctx.primary
                } else {
                    select_face(ctx, cluster, script, emoji, previous_face)
                };
                if face != ctx.primary {
                    previous_face = Some(face);
                } else {
                    previous_face = None;
                }
                face
            }
            _ => ctx.primary,
        };
        let level = bidi.levels[start];
        let embolden = kind == Kind::Text && ctx.embolden(face);
        let joins = items.last().is_some_and(|last: &Item| {
            last.kind == kind
                && kind != Kind::Tab
                && last.face == face
                && last.script == script
                && last.level == level
                && last.range.end == start
        });
        if joins {
            items.last_mut().expect("item").range.end = end;
        } else {
            items.push(Item {
                range: start..end,
                face,
                script,
                level,
                kind,
                embolden,
            });
        }
        graphemes.push(Grapheme {
            range: start..end,
            item: (items.len() - 1) as u32,
            width: 0.0,
            space: cluster.chars().all(char::is_whitespace),
        });
    }
    // Logical grapheme widths (needed for wrapping and truncation).
    let wrapping = max_width.is_some() && (ctx.options.wrap != Wrap::None || ctx.options.ellipsis);
    if wrapping {
        let mut cursor = 0usize;
        for (index, item) in items.iter().enumerate() {
            let first = cursor;
            while cursor < graphemes.len() && graphemes[cursor].item as usize == index {
                cursor += 1;
            }
            match item.kind {
                Kind::Tab => graphemes[first].width = ctx.tab,
                Kind::Control => {}
                Kind::Text => {
                    let piece = shape_piece(
                        ctx,
                        caches,
                        para,
                        item.range.clone(),
                        item.face,
                        item.script,
                        item.level.is_rtl(),
                    );
                    let starts: Vec<usize> = graphemes[first..cursor]
                        .iter()
                        .map(|g| g.range.start - item.range.start)
                        .collect();
                    let widths = grapheme_widths(&piece, &starts, item.range.len());
                    for (g, w) in graphemes[first..cursor].iter_mut().zip(widths) {
                        g.width = w;
                    }
                }
            }
        }
    }
    // Line breaking over grapheme indices.
    let mut lines: Vec<Range<usize>> = Vec::new();
    if let (Some(limit), true) = (max_width, wrapping && ctx.options.wrap != Wrap::None) {
        let opportunities: Vec<usize> = match ctx.options.wrap {
            Wrap::Character => graphemes.iter().map(|g| g.range.start).collect(),
            _ => analysis::line_breaks(para)
                .into_iter()
                .filter(|(_, kind)| matches!(kind, Break::Allowed | Break::Mandatory))
                .map(|(at, _)| at)
                .collect(),
        };
        let mut line_start = 0usize;
        let mut pen = 0.0f32;
        let mut visible = 0.0f32;
        let mut candidate: Option<usize> = None;
        let mut next_opportunity = 0usize;
        let mut g = 0usize;
        while g < graphemes.len() {
            let start = graphemes[g].range.start;
            while next_opportunity < opportunities.len() && opportunities[next_opportunity] < start
            {
                next_opportunity += 1;
            }
            if g > line_start && opportunities.get(next_opportunity) == Some(&start) {
                candidate = Some(g);
            }
            let width = if items[graphemes[g].item as usize].kind == Kind::Tab {
                ((pen / ctx.tab).floor() + 1.0) * ctx.tab - pen
            } else {
                graphemes[g].width
            };
            pen += width;
            if !graphemes[g].space {
                visible = pen;
            }
            if visible > limit + 0.01 && g > line_start {
                let end = candidate.filter(|&c| c > line_start).unwrap_or(g);
                lines.push(line_start..end);
                if lines.len() >= max_lines.saturating_sub(layout.lines.len()) {
                    line_start = end;
                    break;
                }
                line_start = end;
                candidate = None;
                pen = 0.0;
                visible = 0.0;
                g = end;
                continue;
            }
            g += 1;
        }
        if line_start < graphemes.len()
            && lines.len() < max_lines.saturating_sub(layout.lines.len())
        {
            lines.push(line_start..graphemes.len());
        } else if line_start < graphemes.len() {
            layout.truncated = true;
        }
    } else {
        lines.push(0..graphemes.len());
    }
    let line_count = lines.len();
    for (number, mut range) in lines.into_iter().enumerate() {
        let available = max_lines.saturating_sub(layout.lines.len());
        if available == 0 {
            layout.truncated = true;
            break;
        }
        let last_kept = available == 1 && (number + 1 < line_count || !last_paragraph);
        let overflow = max_width.is_some_and(|limit| {
            graphemes[range.clone()]
                .iter()
                .map(|g| g.width)
                .sum::<f32>()
                > limit + 0.01
        });
        let mut ellipsis = false;
        if ctx.options.ellipsis
            && (overflow || last_kept || (layout.truncated && number + 1 == line_count))
        {
            #[allow(clippy::collapsible_if)]
            if let Some(limit) = max_width {
                let mark = ellipsis_width(ctx, caches);
                let mut width: f32 = graphemes[range.clone()].iter().map(|g| g.width).sum();
                while range.end > range.start && width + mark > limit + 0.01 {
                    range.end -= 1;
                    width -= graphemes[range.end].width;
                }
                ellipsis = true;
                layout.truncated = true;
            }
        }
        let text_range = if range.is_empty() {
            let at = graphemes
                .get(range.start)
                .map_or(para.len(), |g| g.range.start);
            at..at
        } else {
            graphemes[range.start].range.start..graphemes[range.end - 1].range.end
        };
        place_line(
            ctx, caches, layout, para, base, &bidi, &items, text_range, rtl, ellipsis,
        );
    }
    Ok(())
}

fn ellipsis_width(ctx: &Context<'_>, caches: &mut Caches) -> f32 {
    let piece = shape_piece(
        ctx,
        caches,
        "\u{2026}",
        0..3,
        ctx.primary,
        Script::Common,
        false,
    );
    piece
        .glyphs
        .iter()
        .map(|g| g.advance as f32 * piece.scale)
        .sum()
}

#[allow(clippy::too_many_arguments)]
fn place_line(
    ctx: &Context<'_>,
    caches: &mut Caches,
    layout: &mut TextLayout,
    para: &str,
    base: usize,
    bidi: &ParagraphBidiInfo<'_>,
    items: &[Item],
    range: Range<usize>,
    rtl: bool,
    ellipsis: bool,
) {
    let line_index = layout.lines.len() as u32;
    let levels = if range.is_empty() {
        bidi.levels.clone()
    } else {
        bidi.reordered_levels(range.clone())
    };
    // Line pieces: items clipped to the line and split at L1 level changes.
    struct LinePiece {
        range: Range<usize>,
        item: Option<usize>,
        level: Level,
    }
    let mut pieces: Vec<LinePiece> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let start = item.range.start.max(range.start);
        let end = item.range.end.min(range.end);
        if start >= end {
            continue;
        }
        let mut at = start;
        while at < end {
            let level = levels[at];
            let mut stop = at;
            while stop < end && levels[stop] == level {
                stop += 1;
            }
            pieces.push(LinePiece {
                range: at..stop,
                item: Some(index),
                level,
            });
            at = stop;
        }
    }
    if ellipsis {
        pieces.push(LinePiece {
            range: range.end..range.end,
            item: None,
            level: bidi.paragraph_level,
        });
    }
    let order: Vec<usize> = if pieces.iter().any(|p| p.level.is_rtl()) {
        let piece_levels: Vec<Level> = pieces.iter().map(|p| p.level).collect();
        ParagraphBidiInfo::reorder_visual(&piece_levels)
    } else {
        (0..pieces.len()).collect()
    };
    let run_start = layout.runs.len() as u32;
    let mut line_clusters: Vec<Cluster> = Vec::new();
    let mut pen = 0.0f32;
    for &p in &order {
        let piece = &pieces[p];
        let piece_rtl = piece.level.is_rtl();
        let glyph_start = layout.glyphs.len() as u32;
        let Some(item_index) = piece.item else {
            // Ellipsis.
            let shaped = shape_piece(
                ctx,
                caches,
                "\u{2026}",
                0..3,
                ctx.primary,
                Script::Common,
                piece_rtl,
            );
            for g in &shaped.glyphs {
                layout.glyphs.push(Glyph {
                    id: g.glyph,
                    x: pen + g.x_offset as f32 * shaped.scale,
                    y: -(g.y_offset as f32) * shaped.scale,
                    cluster: (base + range.end) as u32,
                });
                pen += g.advance as f32 * shaped.scale;
            }
            layout.runs.push(Run {
                face: ctx.primary,
                wght: shaped.wght,
                opsz: shaped.opsz,
                embolden: false,
                rtl: piece_rtl,
                glyphs: glyph_start..layout.glyphs.len() as u32,
            });
            continue;
        };
        let item = &items[item_index];
        let graphemes: Vec<usize> = para[piece.range.clone()]
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .collect();
        match item.kind {
            Kind::Tab | Kind::Control => {
                for &g in &graphemes {
                    let start = piece.range.start + g;
                    let end = next_in(para, &piece.range, start);
                    let width = if item.kind == Kind::Tab {
                        ((pen / ctx.tab).floor() + 1.0) * ctx.tab - pen
                    } else {
                        0.0
                    };
                    line_clusters.push(Cluster {
                        start: (base + start) as u32,
                        end: (base + end) as u32,
                        line: line_index,
                        x: pen,
                        width,
                        rtl: piece_rtl,
                        glyphs: (glyph_start, glyph_start),
                    });
                    pen += width;
                }
            }
            Kind::Text => {
                let shaped = shape_piece(
                    ctx,
                    caches,
                    para,
                    piece.range.clone(),
                    item.face,
                    item.script,
                    piece_rtl,
                );
                let groups = cluster_groups(&shaped);
                // Cluster boundaries (relative) sorted logically, for ligature splits.
                let mut starts: Vec<u32> = groups.iter().map(|g| g.0).collect();
                starts.sort_unstable();
                starts.dedup();
                for (cluster, glyph_range, advance) in &groups {
                    let group_x = pen;
                    let first_glyph = layout.glyphs.len() as u32;
                    for g in &shaped.glyphs[glyph_range.clone()] {
                        layout.glyphs.push(Glyph {
                            id: g.glyph,
                            x: pen + g.x_offset as f32 * shaped.scale,
                            y: -(g.y_offset as f32) * shaped.scale,
                            cluster: (base + piece.range.start + g.cluster as usize) as u32,
                        });
                        pen += g.advance as f32 * shaped.scale;
                    }
                    let glyphs = (first_glyph, layout.glyphs.len() as u32);
                    let next = starts
                        .iter()
                        .find(|&&s| s > *cluster)
                        .map_or(piece.range.len(), |&s| s as usize);
                    let members: Vec<usize> = graphemes
                        .iter()
                        .copied()
                        .filter(|&g| g >= *cluster as usize && g < next)
                        .collect();
                    let count = members.len().max(1) as f32;
                    let share = advance / count;
                    for (k, &g) in members.iter().enumerate() {
                        let start = piece.range.start + g;
                        let end = graphemes
                            .iter()
                            .find(|&&n| n > g)
                            .map_or(piece.range.end, |&n| piece.range.start + n);
                        let x = if piece_rtl {
                            group_x + advance - (k as f32 + 1.0) * share
                        } else {
                            group_x + k as f32 * share
                        };
                        line_clusters.push(Cluster {
                            start: (base + start) as u32,
                            end: (base + end) as u32,
                            line: line_index,
                            x,
                            width: share,
                            rtl: piece_rtl,
                            glyphs,
                        });
                    }
                }
                let (wght, opsz) = (shaped.wght, shaped.opsz);
                layout.runs.push(Run {
                    face: shaped.face,
                    wght,
                    opsz,
                    embolden: item.embolden,
                    rtl: piece_rtl,
                    glyphs: glyph_start..layout.glyphs.len() as u32,
                });
                continue;
            }
        }
    }
    line_clusters.sort_by_key(|c| c.start);
    // Visible width excludes trailing (logical end) whitespace.
    let mut trailing = 0.0f32;
    for cluster in line_clusters.iter().rev() {
        let s = &para[(cluster.start as usize - base)..(cluster.end as usize - base)];
        if s.chars().all(char::is_whitespace) {
            trailing += cluster.width;
        } else {
            break;
        }
    }
    let visible = (pen - trailing).max(0.0);
    layout.clusters.extend(line_clusters);
    layout.lines.push(Line {
        text: base + range.start..base + range.end,
        runs: run_start..layout.runs.len() as u32,
        x: if rtl { -trailing } else { 0.0 },
        width: visible,
        top: line_index as f32 * layout.line_height,
        rtl,
    });
}

fn next_in(para: &str, range: &Range<usize>, start: usize) -> usize {
    analysis::next_grapheme(para, start).min(range.end)
}

impl TextLayout {
    fn s(&self) -> f32 {
        self.format.scale
    }
    pub fn format(&self) -> &TextFormat {
        &self.format
    }
    pub fn metrics(&self) -> LayoutMetrics {
        LayoutMetrics {
            width: self.width / self.s(),
            height: self.lines.len() as f32 * self.line_height / self.s(),
            lines: self.lines.len(),
            truncated: self.truncated,
        }
    }
    /// Physical-pixel size (rounded up).
    pub fn physical_size(&self) -> (u32, u32) {
        (
            self.width.ceil() as u32,
            (self.lines.len() as f32 * self.line_height) as u32,
        )
    }
    pub fn line_height(&self) -> f32 {
        self.line_height / self.s()
    }
    /// Baseline offset below a line box top, in logical pixels.
    pub fn baseline(&self) -> f32 {
        self.baseline / self.s()
    }
    pub fn lines(&self) -> Vec<LineInfo> {
        self.lines
            .iter()
            .map(|l| LineInfo {
                text: l.text.clone(),
                top: l.top / self.s(),
                height: self.line_height / self.s(),
                baseline: (l.top + self.baseline) / self.s(),
                x: l.x / self.s(),
                width: l.width / self.s(),
                rtl: l.rtl,
            })
            .collect()
    }
    /// The cluster map: every grapheme cluster in logical order.
    pub fn clusters(&self) -> Vec<ClusterInfo> {
        self.clusters.iter().map(|c| self.cluster_info(c)).collect()
    }
    fn cluster_info(&self, c: &Cluster) -> ClusterInfo {
        let line = &self.lines[c.line as usize];
        ClusterInfo {
            text: c.start as usize..c.end as usize,
            line: c.line as usize,
            x: (line.x + c.x) / self.s(),
            width: c.width / self.s(),
            rtl: c.rtl,
            glyphs: c.glyphs.0 as usize..c.glyphs.1 as usize,
        }
    }
    pub fn glyphs(&self) -> Vec<GlyphInfo> {
        let mut out = Vec::with_capacity(self.glyphs.len());
        for line in &self.lines {
            for run in &self.runs[line.runs.start as usize..line.runs.end as usize] {
                for g in &self.glyphs[run.glyphs.start as usize..run.glyphs.end as usize] {
                    out.push(GlyphInfo {
                        face: usize::from(run.face),
                        glyph: g.id,
                        x: (line.x + g.x) / self.s(),
                        y: (line.top + self.baseline + g.y) / self.s(),
                        cluster: g.cluster as usize,
                        rtl: run.rtl,
                    });
                }
            }
        }
        out
    }
    fn line_at_y(&self, y: f32) -> usize {
        let physical = y * self.s();
        let index = (physical / self.line_height).floor();
        (index.max(0.0) as usize).min(self.lines.len().saturating_sub(1))
    }
    fn line_clusters(&self, line: usize) -> &[Cluster] {
        let start = self.clusters.partition_point(|c| (c.line as usize) < line);
        let end = self.clusters.partition_point(|c| (c.line as usize) <= line);
        &self.clusters[start..end]
    }
    /// Map a point (logical pixels) to the nearest caret position.
    pub fn hit_test(&self, x: f32, y: f32) -> HitTest {
        let line_index = self.line_at_y(y);
        let Some(line) = self.lines.get(line_index) else {
            return HitTest {
                offset: 0,
                affinity: Affinity::Downstream,
                line: 0,
                inside: false,
            };
        };
        let px = x * self.s() - line.x;
        let clusters = self.line_clusters(line_index);
        if clusters.is_empty() {
            return HitTest {
                offset: line.text.start,
                affinity: Affinity::Downstream,
                line: line_index,
                inside: false,
            };
        }
        let mut best: Option<(&Cluster, f32)> = None;
        for c in clusters {
            let distance = if px < c.x {
                c.x - px
            } else if px >= c.x + c.width {
                px - (c.x + c.width) + if c.width == 0.0 { 0.001 } else { 0.0 }
            } else {
                -1.0
            };
            if best.is_none_or(|(_, d)| distance < d) {
                best = Some((c, distance));
            }
        }
        let (cluster, distance) = best.expect("non-empty");
        let inside =
            distance < 0.0 && y >= 0.0 && y * self.s() < self.lines.len() as f32 * self.line_height;
        let leading_half = px < cluster.x + cluster.width / 2.0;
        // Left half of an LTR cluster or right half of an RTL cluster: before it.
        let before = leading_half != cluster.rtl;
        let (mut offset, mut affinity) = if before {
            (cluster.start as usize, Affinity::Downstream)
        } else {
            (cluster.end as usize, Affinity::Upstream)
        };
        // Past the end of a hard line, the caret belongs to this line's end.
        if offset == line.text.end && affinity == Affinity::Downstream {
            affinity = Affinity::Upstream;
        }
        if offset > self.text_len {
            offset = self.text_len;
        }
        HitTest {
            offset,
            affinity,
            line: line_index,
            inside,
        }
    }
    /// Caret geometry for a byte offset with affinity (logical pixels).
    pub fn caret(&self, offset: usize, affinity: Affinity) -> Caret {
        let offset = offset.min(self.text_len);
        let s = self.s();
        let caret = |line: usize, x: f32, rtl: bool| Caret {
            x: (self.lines[line].x + x) / s,
            top: self.lines[line].top / s,
            height: self.line_height / s,
            line,
            rtl,
        };
        let after = self.clusters.iter().find(|c| c.start as usize == offset);
        let before = self.clusters.iter().find(|c| c.end as usize == offset);
        let leading = |c: &Cluster| {
            caret(
                c.line as usize,
                if c.rtl { c.x + c.width } else { c.x },
                c.rtl,
            )
        };
        let trailing = |c: &Cluster| {
            caret(
                c.line as usize,
                if c.rtl { c.x } else { c.x + c.width },
                c.rtl,
            )
        };
        match (affinity, before, after) {
            (Affinity::Upstream, Some(b), _) => trailing(b),
            (Affinity::Downstream, _, Some(a)) => leading(a),
            (_, Some(b), None) => trailing(b),
            (_, None, Some(a)) => leading(a),
            (_, None, None) => {
                // Empty line or offset inside a cluster: the containing cluster or line start.
                if let Some(c) = self
                    .clusters
                    .iter()
                    .find(|c| (c.start as usize) < offset && offset < c.end as usize)
                {
                    return leading(c);
                }
                let line = self
                    .lines
                    .iter()
                    .position(|l| l.text.start <= offset && offset <= l.text.end)
                    .unwrap_or(self.lines.len().saturating_sub(1));
                let x = if self.lines.get(line).is_some_and(|l| l.rtl) {
                    self.lines[line].width
                } else {
                    0.0
                };
                if self.lines.is_empty() {
                    return Caret {
                        x: 0.0,
                        top: 0.0,
                        height: self.line_height / s,
                        line: 0,
                        rtl: false,
                    };
                }
                caret(line, x, self.lines[line].rtl)
            }
        }
    }
    /// Selection rectangles (logical pixels) for a byte range: one per
    /// visually contiguous span per line.
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<Rect> {
        let mut out = Vec::new();
        if range.start >= range.end {
            return out;
        }
        let s = self.s();
        for (index, line) in self.lines.iter().enumerate() {
            let mut spans: Vec<(f32, f32)> = self
                .line_clusters(index)
                .iter()
                .filter(|c| (c.start as usize) < range.end && (c.end as usize) > range.start)
                .map(|c| (c.x, c.x + c.width))
                .collect();
            spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
            let mut merged: Vec<(f32, f32)> = Vec::new();
            for span in spans {
                match merged.last_mut() {
                    Some(last) if span.0 <= last.1 + 0.01 => last.1 = last.1.max(span.1),
                    _ => merged.push(span),
                }
            }
            for (a, b) in merged {
                out.push(Rect {
                    x: (line.x + a) / s,
                    y: line.top / s,
                    width: (b - a) / s,
                    height: self.line_height / s,
                });
            }
        }
        out
    }
    /// Visual caret movement (arrow keys) within a line; crossing to the
    /// adjacent line at the ends. Returns the new offset and affinity.
    pub fn move_visual(&self, offset: usize, affinity: Affinity, right: bool) -> (usize, Affinity) {
        let current = self.caret(offset, affinity);
        let line = current.line;
        let x = current.x * self.s() - self.lines.get(line).map_or(0.0, |l| l.x);
        // Caret stops of this line: every cluster edge with its logical position.
        let mut stops: Vec<(f32, usize, Affinity)> = Vec::new();
        for c in self.line_clusters(line) {
            let (left, right_edge) = (c.x, c.x + c.width);
            if c.rtl {
                stops.push((right_edge, c.start as usize, Affinity::Downstream));
                stops.push((left, c.end as usize, Affinity::Upstream));
            } else {
                stops.push((left, c.start as usize, Affinity::Downstream));
                stops.push((right_edge, c.end as usize, Affinity::Upstream));
            }
        }
        let candidate = if right {
            stops
                .iter()
                .filter(|s| s.0 > x + 0.01)
                .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal))
        } else {
            stops
                .iter()
                .filter(|s| s.0 < x - 0.01)
                .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal))
        };
        if let Some(&(_, offset, affinity)) = candidate {
            return (offset, affinity);
        }
        // Line end: move to the adjacent line in reading order.
        let rtl = self.lines.get(line).is_some_and(|l| l.rtl);
        let forward = right != rtl;
        if forward && line + 1 < self.lines.len() {
            (self.lines[line + 1].text.start, Affinity::Downstream)
        } else if !forward && line > 0 {
            (self.lines[line - 1].text.end, Affinity::Upstream)
        } else {
            (offset, affinity)
        }
    }
    /// Line index containing a caret position.
    pub fn line_of(&self, offset: usize, affinity: Affinity) -> usize {
        self.caret(offset, affinity).line
    }
}
