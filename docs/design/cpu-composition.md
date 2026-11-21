# Constant-source CPU composition

Separate P12 development batch following the accepted typography cache. The
isolated4K WHPX/RAMFB run26524 retained the first full-frame timing:
8composition ticks and1transfer tick, with10ms ticks. This is one coarse sample,
not an input-latency or frame-rate measurement. Window moves already reuse
immutable client snapshots; this work does not change that contract.

The constant-alpha rectangle path intersected its physical clip, then called
the general pixel function for every pixel, repeating clip/coordinate checks.
It now blends active row slices using the unchanged integer source-over helper.
Every destination remains independent; no blended row is copied over different
wallpaper rows. Alpha0 is a no-op; the existing alpha255 scanline fill remains.
All four channels, saturation, stride padding and guards retain their behavior.
No unsafe code, SIMD assumption, allocation or dependency is added.

Signed-out composition no longer computes a taskbar blur that the login branch
never reads. Signed-in composition initializes the backdrop from the actual
scene before consuming it. Full sign-in damage must cover the bar; actual native
admission/theme/taskbar behavior remains a required regression.

The first real frame is now sampled even below the80ms slow threshold, within
the same16sample budget. The diagnostic uses bounded stack formatting and a
best-effort write; no fabricated tick value or per-frame lifetime log is used.

The57graphics-core and31window tests pass, including differential comparisons
against the unchanged public pixel path for every alpha and destination-byte
value, scaled clipping and padded/trailing bytes. Strict affected host/native
Clippy passes. Native comparison and full consumer regressions now pass;
independent review accepted this bounded change. P12 remains open.

Read-only review found a real dependency in the blur change: legacy24-byte
session binding previously relied on the signed-out material initialization,
whereas32/40-byte forms also applied a theme/full redraw. Every successful form
now explicitly invalidates the full scene. The existing privileged adversarial
image exercises fresh-connection24/32/40 bindings; production keeps its modern
binding. The server records the actual completed frame, full damage and fresh
material after successful transfer. The host requires three distinct ordered
frames after a presented login, recomputes the summary from sealed serial, and
rejects missing/foreign/duplicate/noncanonical markers. This prevents a later
modern bind from hiding an unrendered legacy admission.

Pre-fix build24132 and host22860 passed their existing checks at source8e446f39,
but are retained diagnostic candidates and do not qualify this repaired scope.
Actual native comparisons must use the later repaired build. Frame-budget
reports now call the total `frame_samples` and identify first-frame inclusion;
the total does not imply that every sampled frame was slow.

The repaired complete build17480 is source8edde3a0b80ac681cbb3bbeafe2827cdc23293fb529b00da86b8f40d0146775e.
Final host/native validation is sourcec324501dd1f9270647711689c5b7767378ab95c80aac86fff492610186072fdf.
Only three host files bridge these snapshots: desktop/recovery.rs,
desktop/validate.rs and services/scenarios.rs under tools/xtask/src. Guest
source, Cargo inputs and assets are identical across that bridge. The critic
rehashed26build artifacts/pairs, including five independent app pairs.

Final host23156 passes547tests/33executables; assets19556 passes. Isolated4K9424
passes15checks. Files19324/24844 each pass19checks/two boots at1x/3x, retaining
object26/inode17/generation1 and payload across rename and reboot. Combined19508
passes243checks/nine sessions, including actual full24/32/40-byte admissions.
Foundation25484 matches all42expected outcomes:27pass,6panic,5interrupted,
2startupfailure,1guestfailure and1timeout. All final test inputs were unchanged.
Full report/hash bindings are in the coordinated HANDOFF/requirements records.

The isolated4K first frame changed from8compose/1transfer ticks in26524 to
5compose/1transfer ticks in9424, with10ms ticks,33,177,600transfer bytes and2025calls.
This is one coarse observation per version, not a statistical/FPS/whole-sign-in
claim. The two retained signed-out3840x2160PNGs have exactly identical decoded
RGB pixels and the same SHA256627ddad4158db13081b25772645859cdff0834099fff0aa865614e49f658f3a4.
That equality applies only to those captures.

Independent attributed review: reports/critics/p12-cpu-composition-critic.json,
SHA256a06e1d75765bf8f5ab3843dce5300d20812ab8a43a2ba7e07b52fcb35c07e07f.
It found no unresolved actionable issue in this finite scope, rehashed504combined,
93Files and845foundation references, and independently audited the durable disks.
The original missing P00 report still blocks cumulative qualification.
Fresh cumulative attempt23564 fails on that same original missing report;
the gate and historical evidence remain unchanged.
