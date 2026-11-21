# Synchronous service reply latency

P12 development investigation; not phase qualification or an app/FPS result.

The synchronous SDK client currently receives nonblocking replies, then sleeps
until the next 10ms monotonic tick after WouldBlock. On the single-CPU guest the
service often cannot run before that first receive. Multiple short RPCs can
therefore accumulate timer waits during app startup, sign-in and file operations.
The native measurement below confirms this fixed-wait cost in the private fixture.

The unchanged-client baseline is now observed: report
`host-1792526591845892300-13872` and its authenticated native child
`host-1792526595350418800-13872` passed the exact private-disk audit and measured
128 ticks for 128 metadata requests (1,280ms total). The core image came from
`host-1792526544009417500-10340`; all three reports have unchanged source inputs.
The immutable baseline is `out/qemu/host-1792525481462118000-8952/data-after.raw`,
SHA256 `5736ae96df86cdfc5ffd8194bb7b5d670268416ba7b8207b8d4c5762ddba6c4d`.

The explicit private SDK `rename` exercise now measures 128 sequential metadata
requests after its durable rename roundtrip. It compares every reply against the
same file identity and metadata, performs no logging inside the timed loop, and
reports coarse native ticks. The host requires one exact marker attributed to
the authenticated, retired SDK process; it independently checks the private disk
and original baseline. Legacy save/reopen/restart/demo paths do not measure or
perform the rename exercise.

Build a proper diagnostic core image with
`cargo xtask build --profile dev-qemu-x64 --through P11`; the explicit earlier
foundation selection avoids treating the unfinished P12 ledger as a qualified
schema. This is one development build, not a cumulative gate. Then run
`cargo xtask test files-sdk --image <image.json> --baseline <saved-private.raw>`.
The launcher clones the baseline and verifies its hash remains unchanged.

Record the unchanged client's result first. Only then consider a bounded sequence
of cooperative yields before the existing timer backoff. Preserve the five-second
deadline, authenticated reply matching, single outstanding request and poisoned
connection behavior. Never replace waits with an unbounded poll loop.

Compare the same 128 calls, pinned single-CPU TCG profile and immutable saved disk
before and after a scheduling change. The 10ms clock cannot establish sub-tick
latency, whole-app responsiveness, input-to-display latency or a refresh rate.
Service failure, restart, deadline and desktop regressions remain required before
accepting a shared client change. The owner disk is excluded.

The candidate gives the server at most eight cooperative turns, then retains
one-tick sleeps and the original absolute deadline. The async begin/poll path and
reply authentication remain unchanged. Tests of the actual shared waiting loop
cover an immediately ready reply, a server runnable before a tick, a stalled
server reaching its original timeout, deadline advancement/overflow boundaries,
and propagation of malformed replies and clock/yield/sleep errors.

The candidate native SDK report `host-1792526765174846600-11240` and child
`host-1792526768686136500-11240` passed using core build
`host-1792526718288789500-1944`, all with unchanged inputs. The same immutable
baseline and pinned profile measured 32 ticks (320ms) for 128 calls: a 75% reduction
for this fixture. Neither run overlapped another QEMU test. This is one coarse
baseline and one initial candidate sample, not a statistical latency benchmark.
A fresh final-validator sample repeated 32 ticks in
`host-1792527438356981800-14616` / child `host-1792527442032093400-14616`.

## Accepted bounded regression

Independent read-only critic `reports/critics/p12-rpc-latency-critic.json` accepted this
finite change with no unresolved actionable finding. SHA256:
`c7725ddba7dd1b5ca626cd67d2c30e858c4438c215dfe1ce80938d28903bdf3d`.
The final executable test snapshot is
`207417f574ec4cff520638986045c05a3e24916c5affff8bbc008253df36081a`.
Later coordinated documentation does not change the tested guest code.

| Evidence run | Observed result |
|---|---|
| host-1792526934388014000-20232 | 537 host tests across 33 executables |
| host-1792526934366402500-25776 | Complete desktop build; five independent app pairs |
| host-1792527296439482100-25876 | 15 native 4K WHPX/RAMFB checks |
| host-1792526934366269000-25320 | 42 expected foundation outcomes, including negative cases and actual SMP |
| host-1792527296462985500-5396 | 242 combined desktop checks; nine clean native sessions |
| host-1792527989649022200-23484 | 19 Files checks, normal density, two boots |
| host-1792527989649650100-18228 | 19 Files checks, 4K/3x, two boots |

All reports passed with unchanged inputs. The critic independently rehashed the
bound artifacts and compared complete disk bytes for legacy persistence, corruption
refusal and the permitted standard-console theme update. Both Files runs preserve
object/inode/generation and contents across rename/reboot. Diagnostic budgets and
clean process/thread retirement remain intact. Strict affected host/native Clippy
passes. No dependency version, feature or Cargo.lock change was made.

The reviewer found one host evidence gap before acceptance: summary fields were
emitted correctly but not recomputed during revalidation. Every included latency
block now matches the sealed authenticated serial exactly, including count, ticks,
tick duration, derived milliseconds and scope. Negative tests mutate/delete fields,
change attribution and reject null/duplicate/noncanonical markers. Retained historical
rename records without measurement remain valid; standard consoles cannot carry it.

This does not qualify whole-app/sign-in latency, FPS, Windows equivalence or P12
closure. Cooperative yields are scheduling opportunities, not a guaranteed next
server. The unchanged cumulative gate still lacks the historical P00 report.
Fresh `cargo xtask verify` in `host-1792528391705912300-23092` retained that exact
missing-report failure with unchanged inputs; no historical record was fabricated.
