# Exact sparse avatar coverage

Separate P12 candidate after the accepted constant-source composition change.
The sign-in avatar has a96DIP radius, producing96/192/288physical pixels at the
three supported fixed densities. Those radii fell outside the existing1..60
rounded-corner cache and repeated sixteen circle samples for every corner pixel.

The original asset exporter now also emits129,024bytes for only those three
top-left quadrants, at offsets0/9,216/46,080. The existing73,810byte1..60asset is
unchanged. Coverage uses the same sixteen offsets and integer rounding. Runtime
selection happens after radius clamping; mirroring preserves all four corners.
Every other radius retains the original sampler. No allocation, unsafe code,
dependency or approximation is introduced. This is CPU geometry optimization,
not a change to font/icon rasterization or a GPU backend.

The public rounded fill now saturates radius-times-scale before applying the
existing geometric clamps. This preserves the mathematically clamped result and
prevents overflow for a caller-supplied large radius on a small canvas.

All60graphics-core tests pass, including exhaustive cached-corner comparison to
the original sixteen-sample function, uncached neighboring/clamped radii and
whole-canvas differential avatar fills at1x/2x/3x with negative origins, clipping,
padded stride, trailing guards and u32::MAX radii. Independent review additionally
found a pre-existing squared-distance overflow for enormous valid rectangles.
Coordinates/clamps remain i64; products/comparison now use i128. Two tiny-canvas
tests use known uncovered/fully-covered geometry, independent of squaring, and
check hit testing as well as public fill behavior. Initial host24120/build4504
passed but predate this repair and remain diagnostic candidates.

Final sourcea24345fd28ff978456545a5864ae5870a1c49a54d48197240cce674c4c5380b4
passes host25580(550tests/33executables), complete independent build16216,
assets20052, isolated native4K14272(15checks), four-resolution11080(52checks)
and combined23852(243checks/nine sessions), all with unchanged inputs.
Only window/workspace binaries differ from accepted17480; all16core artifacts,
including firmware/kernel/INIT/services, are byte-identical. Foundation25484
therefore remains inherited evidence for unchanged core inputs, not a fresh run.

Actual rounded PT_LOAD totals and virtual spans are6,778,880bytes for window and
11,313,152for workspace, below the existing16MiB loader bound. File sizes are
6,855,000and11,367,080respectively; file length alone does not establish mapped
memory cost. The sparse asset SHA256 is
a6da12051d239c3f7b57d51b1c905bb025ab7f9d3737cf696c23849955479f90.

The isolated4K first frame measured3compose/1transfer ticks, versus5/1in9424,
with10ms ticks. This is one coarse observation per version, not FPS or a latency
distribution. The exact retained sign-in PNG remains SHA256
627ddad4158db13081b25772645859cdff0834099fff0aa865614e49f658f3a4.
Independent decoded-RGB comparisons also match the accepted captures at1920x1080,
2560x1600and2560x1440, exercising all three cached radii. Independent final
review accepted this bounded change with no unresolved actionable finding:
reports/critics/p12-avatar-coverage-critic.json,
SHA256c6d92c8b9da0270cbd9640fddebba26aa4734b0cafae53d6c61f604d8bfc9f3a.
It independently regenerated every asset byte, audited ELF memory spans and
rehashed39isolated/139resolution/504combined references. No phase closure follows.
