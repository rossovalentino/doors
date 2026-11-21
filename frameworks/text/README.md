# door-graphics-text (Door Text, P11/P12)

`no_std` + `alloc`, `unsafe`-forbidden native text for private premultiplied
BGRA8888 buffers. Two layers share one `Target`/`Clip` drawing contract:

* **`FontAtlas`** — the original bounded ASCII caches: packed hinted Segoe UI
  Variable (`from_packed`, `from_packed_percent`, `windows_reference_scaled`),
  Cascadia Mono (`monospace_from`, `monospace_from_percent`,
  `monospace_reference_*`), static Segoe UI and Selawik. Drawing ASCII allocates
  nothing and never grows a cache.
* **`TextSystem`** — Unicode text from caller-supplied fonts: UAX #29 graphemes
  and words, UAX #14 line breaking, UAX #9 bidi with visual reordering, UAX #24
  script itemization, HarfRust shaping (kerning, ligatures, marks, Arabic
  joining, Indic reordering, CJK), Windows-like font fallback, hinted runtime
  rasterization at any size and fractional scale (LCD, grayscale, COLR v0/v1
  colour emoji), bounded glyph and shaped-run caches, cluster maps, hit
  testing, carets with affinity, selection rectangles and ellipsis.

## Fonts (`FontSet`)

```rust
let fonts = FontSet::from_files(&[
    (seg_ui_var, FontRole::UiVariable),      // Segoe UI Variable (wght/opsz)
    (segoe_ui, FontRole::Ui),                // static Segoe UI: Arabic, Hebrew, ...
    (segoe_ui_semibold, FontRole::Ui),
    (segoe_ui_symbol, FontRole::Symbol),
])?
.with_provider(Box::new(LazySections));     // optional: map CJK/emoji on first use
let text = Arc::new(TextSystem::new(fonts));
```

Bytes are `&'static [u8]` (shared read-only sections live for the process; host
tests leak their buffers): HarfRust 0.14 and Skrifa share one `read-fonts`
parser and borrow the data for the process lifetime. Every face is validated
(sfnt directory, head/maxp/hhea/cmap, outlines or COLR) before publication;
malformed bytes return `Error::InvalidFont`. Collections (`.ttc`) contribute the
role's UI family: YaHei UI, JhengHei UI, Yu Gothic UI (in `YuGothM.ttc`),
Nirmala UI. `FontSet::describe_file` lists a file's faces.

`FontProvider::load(role) -> Vec<&'static [u8]>` is called at most once per role,
only when a cluster is not covered by the faces loaded so far. Faces are only
appended, so layouts stay valid.

Fallback per grapheme cluster (first face mapping every non-ignorable character,
keeping the previous fallback face for runs such as CJK punctuation): Emoji first
for emoji presentation (VS16, modifiers, ZWJ, keycaps, tags, Emoji_Presentation);
Monospace for `Family::Monospace`; then Segoe UI Variable, Segoe UI; script faces
(Han by locale: `zh-Hant/TW/HK/MO` JhengHei, `ja` Yu Gothic, `ko` Malgun, else
YaHei; kana Yu Gothic; Hangul Malgun; Indic Nirmala UI); Symbol; CJK for Common;
Historic; Fallback; Emoji. Weight: nearest static face, variable `wght` axis, or
synthetic emboldening (static face below 550 asked for 600+).

## Layout and drawing

```rust
let format = TextFormat::new(14.0)           // logical em size
    .with_weight(600).with_scale(1.5)        // fractional DPI
    .with_locale(Locale::new("ja")).with_line_height(20.0);
let layout = text.layout(s, &format, &LayoutOptions { max_width: Some(320.0), ..Default::default() })?;
text.draw(&layout, &mut target, clip, x_physical, top_physical, [24, 24, 24, 255])?;
let hit = layout.hit_test(x, y);             // offset + Affinity
let caret = layout.caret(hit.offset, hit.affinity);
let rects = layout.selection_rects(a..b);    // one per visual span (bidi)
```

* `TextFormat::from_style(TextStyle, scale)` reproduces an atlas style (size,
  weight, line box). Baseline = ceil(ascent at ppem), identical to the atlases.
* Geometry queries are logical pixels relative to the first line-box top-left;
  `draw` takes a physical origin. `TextLayout::{metrics, lines, clusters,
  glyphs, move_visual, line_of}`; `TextSystem::{ink, cache_stats, clear_caches,
  layout_line}`.
* `LayoutOptions`: `max_width`, `wrap` (`None`, `Word`, `Character`),
  `direction` (`Auto`, `Ltr`, `Rtl`), `align`, `max_lines`, `ellipsis`,
  `tab_spaces`. CR LF, LF, CR, U+2028 and U+2029 end lines.
* Helpers: `next_grapheme`, `prev_grapheme`, `grapheme_floor`,
  `grapheme_boundaries`, `next_word`, `prev_word`, `word_at`, `line_breaks`,
  `utf8_to_utf16`, `utf16_to_utf8`, `utf16_len` (never splits surrogates),
  `cluster_is_emoji`, `is_emoji_presentation`.

Rasterization reuses the offline exporter pipeline: Skrifa's TrueType
interpreter (LCD smooth, symmetric, linear metrics), an area raster at three
horizontal samples per pixel and the five-tap RGB filter. All 94 printable ASCII
glyphs of the eight styles at 1x/2x/3x are pixel-identical to `hinted-*.dfa`
(test `runtime_raster_matches_the_packed_hinted_atlases`). `RasterMode::Lcd`
stores RGB coverage (opaque targets get LCD, translucent ones grayscale);
`RasterMode::Grayscale` stores the centre subpixel, identical to LCD on a
`Target::grayscale()` destination. `subpixel_positioning` adds third-pixel phases.

Cache keys (§52): glyphs by face fingerprint (sfnt table directory checksums,
length, collection index) and slot, glyph id, ppem in 1/64 px, variation
instance, raster mode, subpixel phase, hinting/emboldening flags and, for colour
glyphs, the foreground; shaped runs (font units, so all sizes share them) by
run text plus shaping context, face, instance, script, language, direction and
paragraph edges. `CacheLimits` default to 4 MiB of glyphs and 512 KiB of runs;
LRU eviction runs in batches to three quarters of the budget.

## FontAtlas compatibility

Any UTF-8 up to `MAX_TEXT_BYTES` is accepted (`Error::UnsupportedText` is no
longer returned). `FontAtlas::attach_unicode(Arc<TextSystem>, Family)` (or
`with_unicode`) routes non-ASCII `draw`, `draw_physical`, `measure` and
`label_ink` through the engine with the atlas style metrics and scale; ASCII
keeps the cached path and its exact pixels. Without an engine, each grapheme
cluster outside ASCII draws one '?' and control or default-ignorable clusters
are zero width (what apps previously did by hand).

## Measured (host, release, Windows 11 x64)

Typical UI line, 14 px: warm layout 8–12 µs and warm draw 8–16 µs at 100%
(15–35 µs at 150%); a shaping-cache miss adds 3–6 µs; the first appearance of a
line at a new size costs 0.4–1.5 ms (hinting setup plus about 10 µs per new
glyph). Run `cargo test --release -p door-graphics-text --test perf -- --ignored --nocapture`.

## Tests

`cargo test -p door-graphics-text` runs the atlas unit tests and
`tests/corpus.rs`: the multilingual corpus `tests/fixtures/text/corpus.txt`
(Latin with diacritics, Greek, Cyrillic, Arabic, Hebrew with numbers, Hindi,
Chinese, Japanese, Korean, ZWJ/skin-tone/keycap emoji, combining marks, mixed
direction), fallback roles, kerning and em advances, Arabic joining, bidi order
and mirroring, Devanagari conjuncts/reordering, emoji clusters in colour,
carets/hit testing/selection, UTF-16 offsets, wrapping/ellipsis, cache bounds and
eviction, malformed/missing fonts, lazy providers, the atlas shim, raster
equivalence with the packed atlases, grayscale equivalence and golden renders at
1x and 1.5x (`tests/fixtures/text/golden/*.qoi`; `DOOR_TEXT_BLESS=1` rewrites
them, with a PNG preview in the temp directory). Fonts are read from
`assets/fonts` (owner-local; see `assets/fonts/windows-unicode/provenance.json`).

## Guest packaging

`profiles/build/app-assets.txt` carries the base Unicode fonts (Segoe UI Variable,
Segoe UI, Segoe UI Semibold, Segoe UI Symbol: 6,291,940 bytes). The CJK and emoji
files (YaHei UI 19.7 MB, JhengHei UI 21.4 MB, Yu Gothic UI 13.9 MB, Malgun 13.5 MB,
Emoji 12.9 MB, Nirmala UI 5.3 MB, Historic 1.6 MB) exceed the 32 MiB pack and
64 MiB initfs limits together; they need separate on-demand sections behind a
`FontProvider` (or subsetting). No QEMU run qualifies the Unicode path yet; this
is not Microsoft DirectWrite/ClearType and does not claim pixel-identical Windows text.

Selawik 1.01 (`FontAtlas::new`) remains embedded under OFL 1.1 with its licence as
`FONT_LICENSE` (assets/fonts/selawik); keep it with any binary embedding it.
