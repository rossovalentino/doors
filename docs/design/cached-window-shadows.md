# Cached soft shadows and opaque client rows

Accepted bounded P12 checkpoint, independently reviewed in
reports/critics/p12-cached-move-critic.json. Host693/34, native255checks/nine sessions,
isolated4K WHPX19checks and retained revalidation pass. Tested source is
aa7821ae43f50f20bcb2f8251f6547ca9b429b1babfd69975aab4d0d61c7477b.
The earlier baseline averaged27.13ms elapsed drag paint work; P12 remains partial.

Simple moves can now translate the previous private opaque client interior.
Eligibility requires an unchanged normal focused topmost window, unchanged epoch,
old/new bounds inside the work area, no caption damage, and explicit shell opt-in
with no current/previous overlay, pending icons or taskbar changes. Other dirty
mutations cancel the plan; failed transfer forces full invalidation. Coalesced
input retains the first source geometry without mutating in-flight pixels.
Overlap-safe row copies exclude title text, rounded corners and shadow. The four
remaining damage strips are composed normally, then the shell overlay runs once
against the completed scene and original clip. The backend restores its taskbar
underlay only before the first piece. Display transfer bytes are not skipped.

Differential tests compare all move directions, overlapping/disjoint/coalesced
moves, three densities, both themes, translucent snapshots and position-dependent
background/title painting. Other cases cover failed transfers, content/state
invalidation, offscreen fallback, in-flight immutability, and all four work-area
edges with once-only overlay invocation. The first real successful reused transfer
emits one bounded actor-attributed marker; the motion validator recomputes its
positive canonical counters and geometry/transfer bounds when present. Historical
logs without that marker retain their original aggregate shape.

Exploratory focused runs (all 136 dragged paints, same accepted bytes/calls):
the earlier accepted baseline had 303 composition/66 transfer ticks (27.13 ms
mean); cached shadows and direct opaque rows had 202/47 (18.31 ms); initial move
reuse had 170/45 (15.81 ms). Each tick is 10 ms elapsed time including scheduling.
These individual runs are not cadence, latency or 60-FPS qualification. The last
run predates the single-overlay repair and does not qualify that runtime delta.
The final runtime (including the single-overlay repair) records137 drag scopes:
136scene and1bar,133 composition/75 transfer ticks, zero failures,30ms maximum,
and15.18248ms mean elapsed work. It transfers2,443,226,472B in246,669 calls.
The extra taskbar paint adds2,211,840B/135calls versus the136scene baseline;
this is not an identical-work aggregate comparison or a60FPS qualification.
The first successful reuse copies3,201,660 client pixels within the private
composition at serial40. That frame's checked display transfer succeeds; the
marker is attributable to WindowPID5 before the recorded motion cutoff.

Final reports: host18476, desktop16756, isolated motion5768, retained23656
under reports/runs/ (full paths/hashes in the critic record). Independent audit
reopens755 combined file references,178 RGB screenshot pairs,176 probes,
36 private/staged image entries and eight raw snapshots; isolated audit covers
32 references,10 RGB pairs and seven probes plus exact20-byte saved content.
All native sessions retire with zero leaks and DEVICE8. Actual artifact comparison
finds19/21 unchanged; Window and Workspace differ and their five repeat pairs
match. Rounded load spans6,807,552B and11,485,184B remain under16MiB. No new
package, dependency, service ABI, kernel or unsafe code is introduced.

Cumulative verify fails before execution at the legacy8GB admission reserve
(out16,892,989,473B versus12GB admission ceiling, under20GB retention cap).
check-spec still references the original missing P00 report32560. These failed
reports are preserved. Visible SDL/native scaling, Windows shadow identity,
60FPS/input latency, modern Explorer, Snap, Search, hover transitions, durable
trash and real app providers remain open.

The same development batch replaces the old Documents library/sidebar role with
the installed Windows known-folder reference: imageres.dll.mun group112, as named
by the owner's Documents/desktop.ini. Eight authored frames (16,20,24,32,40,48,64,
256) are pinned in documents-current.ico with checked-byte provenance; source
SHA256 ad4a4a6caf61187a390050124da1070ab7665d0548fbd15d3c0c25060dc981f4.
Both folder and navigation Documents roles use it; the separate white Notepad txt
association remains unchanged. The gray library pictogram with blue base was the
wrong role for these navigation entries. Other old library roles remain open.

Opaque snapshot rows now copy accepted client bytes directly, filling only an
uncovered tail or row while an older snapshot is retained during resize. An
independent general source-over comparison covers clipped source pixels, growth,
native scales and both themes. Translucent client pixels keep source-over blending.

Window shadows use original finite Gaussian coverage exported for the current
three integer densities: 4-DIP sigma, 12-DIP support and 8-DIP corner radius.
Three mirrored quadrants, including center edges, occupy 14,723 bytes. The
compositor stretches middle spans and blends a single cached layer, without
runtime blur, heap allocation or a larger damage margin. Small geometry and
unsupported densities retain the previous safe shadow path. Full scene and
caption-only repaint use the same coverage and active/inactive opacity.

This is a proposed soft visual treatment, not a measured pixel-identical DWM
shadow. The owner's reference guides comparison. Host tests compare the exported
separable filter with direct 2D convolution, and the actual nine-slice renderer
with a separately blurred full-size shape, including clipping, negative origins,
stride padding and untouched bytes.

Exact `cargo xtask test desktop-motion-current` builds through the same desktop
producer and runs only the private 4K motion workflow in retained v2 output. It
exists for measured iteration and does not replace the complete nine-session
desktop gate, foundation evidence or independent review. Extra arguments and
other unadmitted producer graphs retain legacy admission.
