# CPU paint measurement and pending damage

A later bounded [cached-move checkpoint](cached-window-shadows.md) adds soft
shadows, guarded client pixel reuse and installed Documents role artwork. Its
final sample is15.18ms mean elapsed drag work, not60FPS qualification.

P12 bounded independent review accepted. The 60 FPS target remains open.

The Window service aggregates every returned scene, caption, desktop-icon and
taskbar paint scope, keeping the drag state captured before composition. Separate
compose/transfer ticks, checked-copy bytes/calls, maximum work duration, failures,
seven duration bins and four paint-path counts use fixed saturating counters.
No per-frame allocation or lifetime log is added. Cursor-only work and idle waits
are outside these scopes; input/cursor handling during scanout is inside them.
Successful timing ends at checked-transfer completion, before final cursor/logging.
The10ms clock is too coarse to establish60Hz cadence or input latency.

Two bounded records freeze the counters before the first final power RPC. Init
may forcibly retire Window, so Drop alone cannot reliably report this boundary.
Service-stop/scope-exit fallbacks report once; the measurement validator requires
the explicit power-attempt cutoff between canonical final client and Window
retirement. Failed power attempts can leave later paints outside the snapshot.
Each source record including newline fits256bytes, two fit512; the unchanged
16KiB diagnostic credit can still be exhausted during sufficiently long sessions.
Missing measurements fail the dedicated test, without gating desktop rendering.

The host rejects missing/duplicate/foreign/unwrapped records, incompatible fields,
noncanonical or saturated counters, failure samples, contradictory histograms,
time/byte bounds and diagnostic over-budget evidence. Identity and ordered cutoff
come from the actual sealed kernel-owned shutdown chain. Retained full-suite
density reports with aggregates are recomputed from their serial evidence;
historical reports without these optional aggregates remain historical.

`cargo xtask test desktop-motion --image <production-image.json>` uses a private
4K WHPX/RAMFB disk and the ordinary launcher. Eight additional round trips reuse
cached Files content, restore the final geometry, and retain actual pixel and
client-recapture checks. Pointer steps are50physical pixels with70ms waits and
100ms button transitions: these paced gestures are not an FPS benchmark. The
dedicated test requires drag scene samples and all four non-drag paint paths.

Caption and icon transfers now use the same checked16KiB span iterator as scene
and taskbar transfers. Partial rows preserve gaps; full-width rows coalesce.
A byte-exact differential test covers both paths, output guards and untouched
gaps. Tested4K caption-strip calls fall288to96 and384to90 respectively; that
planner result does not measure native speed. Input is serviced at 32 KiB
thresholds between bounded spans (less than 48 KiB between service points),
and new icon damage queued during that service remains pending.

When input serviced during a transfer has already queued another repaint, the
main loop yields cooperatively rather than inserting another10ms timer sleep.
An idle loop retains its sleep. Transfer snapshots and existing pending-damage
rules remain unchanged. No kernel copy limit, stack budget, ABI, GPU driver or
vblank/page-flip provider is changed.

## Qualified observations

Frozen source `8c33d952fb3130129ce38681eb8bb513cffe6ecddffa1a456affe2685683efb9`:

- Host `host-1792580358816014400-22272`: 684 tests, 34 executables, all passed.
- Complete desktop `host-1792579067070844800-12664`: 253 native plus two maintenance
  checks, nine sessions. All actual identity, pixel, persistent-byte and clean
  retirement checks pass. Twenty of 21 actual artifacts match the accepted
  alignment build; only Window changes. Five independent app build pairs match.
  Foundation is inherited by actual byte identity, without a fresh 42-case claim.
- Private motion `host-1792580220114666900-13752`: 17 native plus two maintenance
  checks, one 4K WHPX/RAMFB session. Non-drag paths are 88 scene, 11 caption,
  seven icons and six bar; 136 drag scopes are scene paints. Both groups have
  zero failures. The two records consume 226 source bytes, Window total 2,855.
- Retained gate `host-1792580479064519200-19836` passes; it recomputes the new
  full-suite density aggregates from sealed serial. The independent review and
  report hashes are in `reports/critics/p12-motion-measurement-critic.json`.

The drag group sums 303 composition ticks and 66 transfer ticks. At 10 ms per
tick, the means are 22.28 ms composing, 4.85 ms transferring and 27.13 ms combined.
These are monotonic **elapsed paint-scope intervals**, including scheduling and
serviced input, not pure CPU time/cycles. The maximum is four measured ticks;
histogram counts are 71 at two ticks, 33 at three, and 32 at four through seven
ticks (the maximum restricts this last occupied bin to four here). Quantization,
paced input, excluded idle work and unobserved host presentation prevent FPS,
input-latency distribution or before/after speedup conclusions. More optimization
is required for the owner's 60 FPS target.

Review repaired two validator findings: a missing retained aggregate cannot bypass
recomputation when serial has MOTION, and histogram bounds require an attained
maximum while capping every bin at it. Six parser tests include both counter
contradictions, malformed identity/ordering, saturation and over-budget records.
The earlier intermittent host-tool fatal exit cause remains unresolved.

`cargo xtask verify` run `host-1792580654244707700-25556` is a failed cumulative
attempt, blocked before execution by the unchanged legacy 8 GB reservation with
16,892,906,266 bytes in out. The owner's out-only 20 GB limit is still respected;
protected evidence and mutable owner volume were preserved. The historical
missing P00 evidence also remains unresolved. This checkpoint does not close P12.
