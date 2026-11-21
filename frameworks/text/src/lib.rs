#![no_std]
#![forbid(unsafe_code)]
//! Door Text: the shared native text facade.
//!
//! * `FontAtlas` — the original bounded ASCII caches (packed hinted Segoe UI
//!   Variable, Selawik, Cascadia Mono). Drawing allocates nothing. Non-ASCII
//!   text renders through an attached `TextSystem`, or as one replacement glyph
//!   per grapheme cluster when none is attached.
//! * `TextSystem` — Unicode text from caller-supplied fonts (`FontSet`):
//!   UAX #29 segmentation, UAX #14 line breaking, UAX #9 bidi, script
//!   itemization, HarfRust shaping, Windows-like font fallback, hinted runtime
//!   rasterization (LCD, grayscale, COLR v0/v1 colour), bounded caches, cluster
//!   maps, hit testing, carets and selection geometry.
extern crate alloc;
mod analysis;
mod atlas;
mod color;
mod font;
mod layout;
mod raster;
mod shape;
mod system;
mod target;
pub use analysis::{
    Break, cluster_is_emoji, grapheme_boundaries, grapheme_floor, is_emoji_presentation,
    line_breaks, next_grapheme, next_word, prev_grapheme, prev_word, utf8_to_utf16, utf16_len,
    utf16_to_utf8, word_at,
};
pub use atlas::*;
pub use font::{FaceInfo, Family, FontProvider, FontRole, FontSet, Locale, MAX_FACES};
pub use layout::{
    Affinity, Align, BaseDirection, Caret, ClusterInfo, GlyphInfo, HitTest, LayoutMetrics,
    LayoutOptions, LineInfo, MAX_LAYOUT_BYTES, Rect, TextFormat, TextLayout, Wrap,
};
pub use raster::{CacheStats, RasterMode};
pub use system::{CacheLimits, DrawStats, InkBox, TextSystem};
pub use target::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidFont,
    AtlasFull,
    InvalidTarget,
    InvalidColor,
    /// Retained for compatibility; Unicode text is no longer rejected.
    UnsupportedText,
    TextTooLong,
    /// Non-finite or out-of-range size, scale, weight, line height or width.
    InvalidFormat,
}
pub type Result<T> = core::result::Result<T, Error>;
/// Bound for one `FontAtlas` draw/measure call (the Unicode layout bound is
/// `MAX_LAYOUT_BYTES`).
pub const MAX_TEXT_BYTES: usize = 4096;

#[cfg(test)]
extern crate std;
#[cfg(test)]
mod tests;
