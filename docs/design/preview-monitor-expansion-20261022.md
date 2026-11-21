# Historical Preview/Monitor integration checkpoint

This document retains its original integration evidence. Its references to
“current”, eleven boots and earlier source hashes describe that checkpoint.
The later [reference revision](reference-apps-20261022.md) supplies the current
Monitor/Preview presentation, final image and selected evidence. P12 stays partial.

# Preview and Monitor checkpoint — 22 October 2026

Active P12 development. Seven focused workflows passed across eleven private guest
boots on the current development image. Exact reference fidelity, all requested app
functionality and sustained 60 FPS/input latency remain open. Earlier reports are
historical checkpoints; the current evidence and its limits are below.

## Current implementation

Preview and Monitor are separate unprivileged native executables using door-app-ui
for shared Window/Storage lifecycle, native Segoe UI metrics and Windows control
icons. The six-app versioned catalog enrolls both apps in Init, Window, Start/
search, desktop/taskbar and Installed apps. Legacy104-byte queries keep four records.
The reserved test probe uses its own catalog slot. Eight process slots permit at
most three ordinary apps alongside five services; this bound was not raised.

Preview reads authenticated Storage progressively and revalidates object/version
and pathname binding after capture. It supports up to eight tabs, full bounded
PNG/JPEG/QOI/BMP/P3-PPM rendering, UTF8/UTF16 text, ZIP central-directory inspection,
PCM8/16-bit WAV metadata/waveforms and bounded OBJ wireframes. Media playback,
PDF/Office engines, archive extraction and broader3D codecs are not provided.
Image decode is capped at one megapixel; raw text input is separately capped at
16 KiB before UTF8/UTF16 decoding. Standalone encoded input is capped at 6 MiB;
Explorer retains its1MiB thumbnail input cap. The viewer has a96MiB heap ceiling
and16MiB aggregate decoded-image admission checked before decode and publication.
Host allocation tracking reserves full4K canvas,3x fonts, retained images, full
icon-cache allowance and extra encoded-input headroom; native heap fragmentation
and large-image high-density pressure still need qualification.

Zoom is an actual clipped percentage transform. View rotation/flipping preserves
source bytes. PNG and QOI image export and UTF8 text export create a new staging
file in the requested parent, upload bounded chunks, flush and recheck identity,
then publish with durable collision/version-checked rename. Existing destination
and source files are not overwritten. A lost final rename response can leave the
new destination published; failure identifies both destination and staging and
never retries automatically. Cancellation/crash may leave a named.part file.
Close is cancelled while export runs; Esc explicitly stops it. Export does not
preserve EXIF/document metadata. Destination format follows the explicit extension.

CreateNew intent is retained before issuing the request. Lost create and publish
replies preserve both exact recovery names through repeated provider disconnects.
Cancellation happens before publication and retains the exact staging name. A paged, measured notice keeps long names readable even in a 240×120
window; its three footer controls share paint/hit geometry. Close cancellation
acknowledges the current configure epoch; stale/repeated-close cases defer safely
to the ordinary event loop. These boundaries have host tests, not native reply-loss
or configure-during-export fault injection.

Shared line editors retain kerning, clip one complete glyph run, show selection,
reveal the caret horizontally and use native vertical metrics. Metadata is paged
when it cannot fit. Visible caption tabs and filmstrip cells share paint/hit bounds;
trusted caption intervals update at native density. Display reconnect retains
application/Storage state; Storage loss cancels uncertain jobs and reconnects
without replaying writes. Fresh discovery retries transient unavailability.

Monitor obtains a sanitized kernel snapshot once per second: real coarse CPU timer
observations, private committed memory, physical memory allocation, process
identities and device read/write/flush request counts. It supports pause/resume,
process filtering/sorting, apps/service-process views and System/Storage summaries.
No invented GPU, network, disk busy%, throughput, endpoint health, startup state or
logs are displayed. No process termination/service control is granted. Provider
limits are visible. See specs/contracts/native-diagnostics-v1.md.

Terminal has one premultiplied console layer with saved50..100% opacity. DTRMv2
retains eight bytes; v1 migrates to95%. Rail/caption and console materials are
separate. Explorer has monitor/chevron breadcrumbs, measured spacing, correct
cyan Details glyph/pane materials and responsive colored tag pills. Tag creation
reveals its complete editor or refuses an impossible24px scroll step; resize
rechecks feasibility. Small Windows icon frames use lossless QOI; a96-entry/8MiB
cache avoids repeated decoding. Thirty selectors save833,833 embedded bytes before
linking; actual ELF load spans remain subject to the existing16MiB limit.

The initial Explorer window now fits the native logical output with its footer
above the standard taskbar. Previously its fixed 700-DIP height extended outside
the work area in common modes, which also disabled translated opaque-pixel reuse.
The contained 4K motion workflow now requires canonical evidence of that reuse,
exact displaced/exposed pixels and zero client recaptures. Geometry changes and
opaque admission retain the existing safe fallback when a window crosses an edge.

Core services and Init previously remained runnable while yielding through empty
polls. After 32 empty scans they now request a one-tick sleep; actual work resets
the backoff and keeps cooperative progress. Init records real lifecycle changes.
Window uses the same backoff after idle polling rather than sleeping after every
successful client exchange, allowing progressive reads/exports to continue without
an unconditional post-reply delay. This remains polling: a cold request can wait
one requested 10 ms tick plus scheduling delay. It is not a latency guarantee or
readiness-driven wakeup mechanism.

## Reference and validation scope

Owner concepts are retained when their files remain available. Installed Explorer
was inspected through Computer Use. The Computer Use skill prohibits terminal
app automation, so Terminal settings references use Microsoft documentation:
[profile appearance](https://learn.microsoft.com/en-us/windows/terminal/customize-settings/profile-appearance)
and [startup](https://learn.microsoft.com/en-us/windows/terminal/customize-settings/startup).
PowerShell 7.6 requires its actual .NET 10 engine; native shell builtins are not CMD,
PowerShell or WSL compatibility. Full Terminal settings/runtime providers remain open.

The owner's Explorer currently uses the classic context menu. Modern-menu references
include Microsoft's [2021 shell guidance](https://blogs.windows.com/blog/2021/07/19/extending-the-context-menu-and-share-dialog-in-windows-11/)
and [August 2026 Insider experiment](https://blogs.windows.com/windows-insider/2026/08/17/improving-file-explorer-context-menu-faster-simpler-and-more-customizable/).
The experiment is not claimed as the installed machine's default menu. Terminal
opacity is real premultiplied transparency; full Windows acrylic blur is unfinished.
Existing RGB subpixel coverage on opaque text targets is not Microsoft's ClearType
implementation; transparent targets retain appropriate coverage blending.

## Current validation

The current image is
`out/images/host-1792623233102212300-12916-desktop-dev/image.json`.
Its build report is `reports/runs/host-1792623233102212300-12916/report.json`.
The independent base build graph completed in `host-1792619937184232000-24708`,
whose combined workflow then failed the earlier Monitor activation gate. That
failed report is preserved. The later image is an explicit development relink,
not an independent full P12 qualification.

| Workflow | Current report under reports/runs | Bounded proof |
|---|---|---|
| Preview | host-1792623443701456900-33232/report.json | Three boots; original bytes preserved; rotated PNG/QOI export/reopen; collision refusal; 3x tabs/close |
| Monitor | host-1792624034785284200-31336/report.json | Kernel-attributed live counters, settled interval, pause/filter/apps/System and clean retirement |
| 4K motion | host-1792623343001520100-30932/report.json | Exact moved/exposed pixels, translated reuse, zero client recaptures, canonical paint aggregates |
| Recovery | host-1792623604134590600-8972/report.json | Real display-service loss, unsaved Notes recovery, denied foreign diagnostics caller and hung-client termination |
| Terminal | host-1792623707293393400-27460/report.json | Two boots, file operations, preferences, tabs and live light/dark application themes |
| Explorer | host-1792623866730823500-13324/report.json | Two boots, actual file preview, tabs/rename and durable tag identity |
| Settings | host-1792623943934333500-24036/report.json | Six advanced pages, actual registry mutation and independent persistent-byte audit |

All seven reports have success and unchanged inputs. Hashes, per-report source
identities and limitations are coordinated in project/phases.json, requirements,
handoff, application inventory and dependency records. The build source is
`54ecd9d2b214bb5492de9f4d554a5026e03566eefdda8394ab9c2e1f9c19dc92`;
the native-run source is
`a4394932a64fa93a9456bc7e550ac53a9ae0e0694a1138d79a5fb369ee140e48`.
Their only differences are a cfg(test) geometry contract in explorer_layout.rs
and the host motion harness's contained gestures/explicit reuse requirement.
Subsequent coordinated edits are documentation only. The independent review receipt
at `reports/runs/preview-expansion-review-20261022/review.json` binds that final bridge.

The full host suite passed 797 tests across 40 executables before final Window and
Explorer changes. Affected 47-test and 327-test suites passed afterward, followed by
two geometry checks and 29 harness checks. These counts overlap. Tests cover the
actual painters in both themes and 1x/2x/3x, measured line editors, uncertainty
notices, format bounds, independent codec round trips, the 96 MiB Preview admission
model and strict telemetry/evidence parsing. Host allocation tests do not establish
native heap fragmentation or one-megapixel high-density pressure.

The 4K paced gesture recorded 88 drag paints, 89 compose ticks plus 88 transfer
ticks, 10 ms per tick: **20.11 ms average paint work, 30 ms maximum**. It recorded
4,888,512 translated pixels and zero client recaptures. Timings include input
serviced during transfer and exclude cursor-only/idle work. This is not sustained
FPS, input latency, vblank observation or a matched before/after speedup. Geometry
and workloads differ from earlier measurements. The latest quiet Monitor interval
recorded **1 busy observation out of 401**, a coarse observation rather than CPU time.

Tests use private disks. No owner volume was initialized; out/session/user.raw is
currently absent. The Monitor run's output receipt measures 6,038,176,889 bytes of the
20,000,000,000-byte out-only limit; target/reports are excluded. The capped image
metadata reader accepts up to 32 MiB, matching its writer, with both pre-read and
post-read bounds. A legal six-app receipt exceeded its former 16 MiB reader bound;
the guest loader's 16 MiB executable-span bound remains unchanged.

Cumulative verification still requires the original deleted P00 report
`reports/runs/host-1791804427914736400-32560/report.json`. History is preserved,
not recreated. P12 remains partial. Older legacy-resolution probes still require
separate current-source maintenance; the focused workflows do not qualify them.

Real filesystem creation/modification times need trustworthy clock capture and a
versioned transactional metadata schema/migration. Owner UID is not file author.
Full Unicode/IME, fractional DPI, visible SDL quality, input timing, broad format
editing/conversion and exact reference comparisons remain required work. Preview
does not yet edit source text, merge documents, provide undo/crop, play audio/video,
render PDF/Office, perform OCR or extract archives. Typical multi-megapixel camera
photos exceed its current one-megapixel bound. Explorer launch handoff, multiple
instances/windows and Recycle Bin/restore remain open. Monitor's missing GPU,
network, startup/log and authorized process/service controls remain explicit.

Asset reproduction check passed in `host-1792624264319260600-30368`.
`check-spec` (`host-1792624233390723900-14720`) and the required cumulative
`verify` (`host-1792624386895948500-16972`) both stopped at the original missing
P00 report. They did not pass the cumulative gate or qualify later checks. The
report hashes and actual failure strings are retained in the coordinated records.
