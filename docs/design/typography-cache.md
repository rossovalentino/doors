# Hinted variable-font evaluation

P12 development implementation. The native desktop and clients select the checked
cache; Windows fidelity and full P12 qualification remain separate requirements.
The accepted RPC/rendering/rename records remain separate.

The previous Windows reference path parsed static Segoe UI and eagerly rasterized
six ASCII styles in every client at startup. The selected path decodes immutable,
hinted Segoe UI Variable glyphs prepared once by the host asset exporter.

Microsoft's [typography guidance](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/typography)
selects Segoe UI Variable for modern controls. The [OpenType optical-size contract](https://learn.microsoft.com/en-us/typography/opentype/spec/dvaraxistag_opsz)
defines points and recommends choosing the default-zoom text size. Evaluate
`opsz = logical DIP * 72/96`, clamped by the actual font axis, with weight400/600;
do not multiply optical size by physical density. Raster size does follow density.

The host-only `cargo xtask assets typography-preview` compares current static
outlines against hinted variable outlines at1x/2x/3x on opaque light/dark surfaces.
The candidate uses pinned Skrifa0.48.0 with its embedded interpreter, horizontal
LCD target and preserved linear advances. Its hinted vector outline passes through
the existing ab_glyph area rasterizer at three horizontal samples per pixel and
the current five-tap RGB filter. The PNGs are labeled host evaluations, never guest
screens or independent Windows reference captures. Actual native evidence follows.

Checked immutable glyph caches are generated at build time rather than executing
a larger hinting engine on every guest app start.
Keep bounds, baseline, advance, kerning, clipping and RGB channel order consistent.
Require provenance/hash checks, strict format validation, bounded cache memory,
no draw-time allocation, native before/after startup and multi-density/theme tests.
Preserve the historical Selawik helper path and keep full Unicode shaping/IME as
an explicit remaining requirement; an ASCII atlas cannot establish it.

Skrifa is a host-only asset dependency; it is not a new guest runtime.
The source font is the owner's installed `C:/Windows/Fonts/SegUIVar.ttf`, pinned
under the existing local-use font provenance. No distribution rights are inferred.
This is an original renderer using an open font engine, not Microsoft ClearType.

The first host preview `host-1792528660253330100-25720` exposed an adapter y-axis
error and is retained as failed visual evidence despite its command succeeding.
It is not an accepted comparison. The corrected preview9376 and packed-cache
preview12152 render actual glyphs; nonempty outlines now require positive bounds
and real coverage.17 graphics-text tests pass, including malformed metadata/QOI,
integer extremes, exact RGB runs, sorted kerning, real ink and unchanged cache
allocation while drawing. Strict affected host/native Clippy passes.

The generated caches use165,463/379,015/582,029bytes at1x/2x/3x and decode to
223,962/779,940/1,673,001bytes of RGB coverage. The existing atlas memory ceiling
is unchanged. The shared decoder validates every header/offset, metric, glyph
extent, pair ordering/range and the complete RGB-only QOI stream before publishing
an atlas. This includes exact run counts/end marker and rejection of trailing data.
The static native baseline is now recorded: complete build
`host-1792529441653657000-24892` and15-check native4K/WHPX run
`host-1792529632331881200-1440`, both unchanged at source
`485f2c79d99d03edf56eefd865d160ed4fffd744a2249a414286b03f1974e519`.
The authenticated compositor constructor took5ticks; the three actual app
constructors each took4ticks. Each reported1,689,057coverage bytes and
4,423,680coverage capacity bytes. These numbers exclude metadata, kerning, mapped
assets and other process memory. They do not measure whole-app startup or sign-in.

After that baseline, Windows-reference constructors now select the generated
cache; the old static constructor and Selawik helper remain available separately.
17text/30window/27workspace/7DPI tests pass. The complete unchanged candidate
build21808 and isolated native4K run26524 use source
`1d64490e3e0a73b38eecb64fc6eee765369f8253dea6f31e48cefd37a5b883bc`.
Each of the four authenticated constructors reported no elapsed10ms ticks;
this does not mean zero execution time or measure whole-app startup/sign-in/FPS.
Their coverage vectors each contain1,673,001bytes with equal capacity. The
firmware and kernel are byte-identical to the static native baseline.

The current host run19452 passes542tests/33executables; assets24648 passes.
Combined desktop19868 passes242checks across nine clean native sessions,
including1x/2x/3x consumers, and Files19064/24464 each pass19checks over two
boots at1x/3x with real dark/light menus and editing. Foundation24756 passes
42expected native outcomes including crash/recovery/refusal/SMP cases.
Independent read-only review accepted this bounded selection with no unresolved
finding: `reports/critics/p12-typography-cache-critic.json`, SHA256
`836cbdff53a15ecaabae354a238fb522f6975c97be0ef66a5a5a6a936f0ac0cd`.
It rehashed504combined,93Files and814foundation evidence references, checked
actual serial identities/retirement and independently compared complete durable
disk bytes. The selected report bindings are in that attributed review and the
coordinated HANDOFF/requirements records. Later documentation does not change
the tested executable inputs.

P12 remains partial. Fresh cumulative verification17756 still fails on the
original missing historicalP00 report; no historical evidence or gate was altered.

The critic found that optional timing writes could fail a reconnect after log
credit exhaustion. Both markers now use one allocation-free256byte diagnostic
capture, with clock/format/write failures ignored. Fresh measurement runs still
require their actual markers. Original full-host run9344 also exposed two tests
choosing the same temporary directory in one clock tick; its failure is retained.
Fixture names now include a checked atomic sequence, with bounded create-new
collision retries. A16-thread test with identical forced timestamps passes.
The source/font asset check passed in `host-1792529560894439700-20544`.
