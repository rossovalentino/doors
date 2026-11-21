<!-- p15-p17-close-20261111 -->
P15-P17 closed out (11 November 2026, later): the read-only critic of each
phase and a second read of the fixes were addressed (findings and fixes:
reports/critics/p15-p17-critic-findings-20261111.md; plan close-out
segments). Each phase is implementation_state "implemented" and stays
in_progress until its formal `cargo xtask verify --phase` runs after
P12-P14 pass. P18 is proposed (docs/design/p18-plan-20261111.md): M0
platform prerequisites (registry hive v2, dynamic code, UCRT, runtime
support, NLS, Schannel, activation contexts, capacity, CNG, the C++
runtime contract, named pipes and shared sections, diagnostics), then
.NET, Windows Forms, the .NET Framework (owner decision), other runtimes,
WinRT and packages, WPF and browsers; it starts when the owner accepts
the plan and its decisions.

<!-- p15-p17-20261111 -->
P15-P17 (11 November 2026): implemented under the owner's parallel
authorization of 2 November 2026 (P15 M1, M2, M4; M3 skipped by the owner;
P16 M5-M8; P17 M9) and guest-tested scenario by scenario. None is verified:
`cargo xtask verify --phase P15|P16|P17` runs once P12-P14 pass, each after
a read-only critic review. Plans: docs/design/p15-plan-20261101.md,
p16-plan-20261102.md, p17-plan-20261109.md; ADRs 0021-0026.

<!-- p14-wsc-20261026 -->
P14 (26 October 2026): authorized by the owner in parallel with P12/P13 (ADR
0020; plan docs/design/p14-plan-20261026.md). Workstreams WS-A (shell and
services), WS-B (service-backed apps) and WS-C (gate, reference, tokens,
trusted sign-in). Not verified.

<!-- p12-records-20261024 -->
# Roadmap — 24 October 2026

- **P12 (selected, in progress, not verified).** Implemented and host-tested:
  toolkit and control catalog, accessibility tree and assistive client, gallery,
  Unicode text, Notes as the TEXT-01 editor (4 MiB), keyboard layouts, IME with
  initfs dictionaries, fractional DPI (125/150/175%), high contrast, clipboard and
  drag and drop, per-app executables with the asset pack. Remaining: text size
  application, localization and time zones, filesystem timestamps, the TEXT-01,
  A11Y-01, DISPLAY-01 and MOTION-01 runners, gallery staging, then a frozen-tree
  `cargo xtask verify`, records and critic.
- **P13** keeps its parallel authorization but is not pursued until P12 is done
  (owner: "do P12 first").
- P14 and later are unchanged.

# Roadmap — P12 and P13 together, 23 October 2026 (superseded by 24 October)

P00–P11 passed; their reports were deleted by the owner and are recorded as
owner-confirmed unavailable (ADR 0017). The current roadmap position is:

- **P12 Native UI, text and accessibility (selected, in progress).** Stage 1
  streams: W5 clipboard/drag-and-drop, W7 themes/high contrast/text size, W8
  fractional DPI, W9 binary split and runtime assets, T Door Text with owner-local
  Windows fonts, U toolkit/accessibility/gallery, I keyboard layouts and IME, G gate
  and records. Stage 2: app migrations, Notes as the TEXT-01 editor, clipboard
  consumers, filesystem timestamps, localization and suite runners. Stage 3:
  frozen-tree verify, records and critic. Exit suites: TEXT-01, A11Y-01,
  DISPLAY-01, MOTION-01 and the component gallery.
- **P13 Windows GUI integration (in progress in parallel, owner decision 6).**
  P13-A loader/personality/kernel32, P13-B portable GUI core and
  user32/gdi32/comdlg32/comctl32, P13-C fixtures F1–F3, Windows reference run and
  guest integration, plus Explorer `.exe` launch. Exit suites: WIN-GUI F1/F2/F3 and
  scenarios 141.5 and 141.6, on top of every P12 suite. P13 closes after P12.
- **P14 Complete desktop and visual fidelity** follows P12 and P13 and remains
  not started; the 26H2 fidelity work so far is P12-partial development, not P14
  qualification. P15 remains the authorized limit.

# Multiwindow app revision — 22 October 2026

P12 remains active and partial. The shared runtime now retains separate window
models, admits new tab destinations before moving their owned documents, and
routes input/configure/close by window ID. Explorer and Preview share caption-tab
geometry; Notes has independent document tabs, checked saves, a Windows-style
menu/status layout and undoable Markdown source tools. Native tabbed chrome uses
the existing cached wallpaper material. Preview adds explicit crop Apply/Cancel
and one-level image undo. The twelve-process/resource and telemetry revision
requires a fresh kernel build; native validation and review are pending.

See [the implementation](../docs/design/multiwindow-apps-20261022.md) and
[the master audit](../docs/design/p11-p12-master-audit-20261022.md). The historical
P00 and P11 run reports are absent; preserve their ledger identities. No exact1:1,
full backend, P12 completion or frame-cadence claim is made. Owner storage stays
untouched and the out-only20GB budget remains in force.

# Monitor and Preview reference revision — 22 October 2026

P12 remains partial. All ten Monitor destinations and CPU/Memory drilldowns now
share the reference layout. Preview has aligned caption tabs, command strip,
colored library glyphs, canvas, filmstrip and a full-height information/actions
pane. Native Segoe Fluent glyphs, measured Segoe UI labels, light/dark palettes,
centered quick-action labels and shared paint/hit bounds replace the earlier UI.
Compact conversion, wrapped descriptions, filtering and modal controls are fixed.

The selected native suites cover six private boots: Monitor one, Preview three,
Explorer two. Preview's final image has 22 of 23 artifact files identical to the
Monitor/Explorer image; only Preview changed. Full host validation passed 812 tests,
followed by 16 overlapping Preview checks after the final paint-only fixes.
Fifty-eight host captures cover both themes; fixture data is not native telemetry.
[Revision details and receipts](../docs/design/reference-apps-20261022.md) record
source identities, provider limits and the independent review receipt.

Exact 1:1 fidelity, actual backdrop blur, missing Preview tools and GPU/network/
control providers remain unfinished. No sustained 60 FPS claim is made. The
missing original P00 report still prevents cumulative qualification; do not
fabricate it or initialize owner storage. Out alone stays capped at 20 GB.

The sections below describe earlier checkpoints; their “current” applies to
those earlier sources and evidence.

# Preview and Monitor checkpoint — 22 October 2026

P12 remains partial. The bundle is Explorer, Preview, Terminal, Settings, Monitor
and Notes. Separate Preview/Monitor apps, real new-file export, sanitized live
telemetry, Terminal opacity, Explorer alignment/tags and service idle fixes are
integrated. Explorer now opens wholly above the taskbar at supported densities.

Seven focused workflows passed across eleven private guest boots on the current
development image. The 4K drag test requires actual translated-pixel reuse and zero
client recaptures. Coarse paint work averaged 20.11 ms, maximum 30 ms; this does not
establish sustained 60 FPS, visible SDL quality or input latency. The latest quiet
Monitor interval recorded 1 busy observation out of 401, not CPU-time measurement.

The full host suite passed 797 tests before final Window/Explorer changes; affected
47-test and 327-test suites, two geometry checks and 29 harness checks then passed.
Counts overlap. Native reports, hashes, source bridges and limits are recorded in
[the checkpoint design](../docs/design/preview-monitor-expansion-20261022.md) and
matching JSON records. Development relinks do not close the cumulative gate.

Exact reference fidelity, broader editors/media providers, real filesystem dates,
full Terminal engines/settings, Recycle Bin and fractional DPI remain unfinished.
The original P00 report is absent after owner cleanup; preserve its historical
identity. No owner volume was initialized. The out-only 20 GB cap remains enforced;
latest size is recorded with its measured receipt in the JSON records.

# Earlier application checkpoint — before Preview/Monitor integration

Terminal now follows the application theme on its rail, tabs, menus and information
pages, with measured label centering, hover, corrected cursor routing and the
original installed Windows Terminal icon. Command completion includes checked
output redirection. Settings lists all four bundled apps and opens Terminal with
the correct catalog command. Explorer now validates exact app retirement per boot.

Validation: 762 host tests across 36 executables passed, followed by five existing
shutdown-validator tests after the host-only Explorer fixture correction. Settings,
Terminal and Explorer passed focused workflows on one checked development image:
five private guest boots, 29 captures and independent persistent-byte audits.
Assets passed. Source identities and report hashes are in
`docs/design/current-app-fidelity-audit-20261022.md` and the matching JSON records.
Independent review: `reports/runs/current-app-fidelity-review-20261022/review.json`.
The cumulative gate stops at the missing original P00 report after owner cleanup;
no historical evidence was recreated. P12 remains partial. Exact Windows rendering,
full Windows backends and sustained 60 FPS are not qualified. `out` is 2.13 GB of
its 20 GB limit; owner storage was untouched.

This completes the bounded current-app audit and fixes, not the active phase.
Backend administration, Trash, fractional DPI, presentation timing and remaining
bundled apps still require implementation and qualification in phase order.

# Current evidence availability after owner cleanup

The owner confirmed deleting reports/runs during cleanup on22October2026. Earlier
reports and current diagnostic captures are therefore unavailable; preserve their
historical identities without claiming they can be opened or newly qualified.
Regenerate only compact current evidence. Explorer host verification passed757
tests across36executables; its first observed two-boot pass is being rerun against
the final shared image. Source and assets remain intact. Owner volumes untouched.

# Generated evidence cleanup — owner action, 22 October 2026

The owner deleted the large generated report/test-data directories after they
exceeded100GB. Source and assets remain. Earlier report and screenshot paths
below may now be unavailable; they must not substitute for fresh validation.
New Settings output is limited to a focused private guest and compact reference
captures, sharing one current image with the other GUI chats. P12 remains partial.
# Explorer installed-Windows reference revision � native diagnostic passed

Explorer now has integrated caption tabs, Windows file-type/known-folder icons,
centered large-grid names, rounded navigation fields, full command strip, and
content-aware folder artwork from real bounded enumeration. PNG/JPEG/QOI and
text/code previews read actual provider bytes; tags start empty, are user-created,
and persist by stable object identity. No invented metadata or preset tags.

Focused host and native compilation pass. The first two-boot private-volume run
passed preview, tag create/assign, checked rename, tab operations, reboot and an
independent exact raw-disk audit with zero leaked resources. Its aggregate report
correctly fails source stability because concurrent Settings source and other
app documentation changed; final common-source validation is pending. See
[design and provider limits](../docs/design/explorer-modern.md) and
[diagnostic evidence](../reports/runs/host-1792606069303592400-11016/report.json).
Exact Windows visual/backend parity and frame cadence remain unqualified. P12
stays partial; historical reports and owner data are preserved.

# Advanced Settings visual refinement — 22 October 2026

The six-page owner references are being compared at their1472x952 client extent.
The current revision adds hinted16px regular/semibold typography, finite cached
Gaussian card shadows, Windows registry/service/computer icons, aligned table
columns, lighter list actions and tab treatments, and reference-position registry
panes. Laptop, monitor and tablet screens use the pinned classic Bloom in the
current light/dark theme. All18 lossless theme/device/density variants pass pixel
checks. Original frames and pinned Windows screen artwork have separate provenance.

The actual painter has twelve host review captures (six pages, two themes), with
explicit review fixtures excluded from guest builds. Latest native diagnostic
report host-1792603189607660900-11340 passed18 checks including six actual pages
and an independent durable registry audit, but failed source stability. It does
not qualify later refinements. The latest cumulative verify report
host-1792604089881300400-6712 stops on the dependency-review lock mismatch. Full
Windows providers, exact1:1 fidelity and smooth frame cadence remain unqualified;
P12 is partial and the owner volume is preserved. See docs/design/advanced-settings.md.

# Terminal reference revision — bounded native implementation

Terminal now uses the existing trusted Window server with an integrated56DIP tab caption, cached selected-wallpaper material, Segoe UI/Cascadia Mono, native Windows glyphs and pinned official profile icons. Home, Settings, About and Profiles work; native file commands and checked preferences persisted across two actual guest boots. CMD/PowerShell engines and broad script/DLL compatibility remain unavailable; WSL stays Work in progress. Exact1:1 and smooth-frame qualification remain open. The earlier native diagnostic passed12Terminal checks, but its report was subsequently removed by owner cleanup; current evidence is being regenerated.756host tests passed in isolated non-tools/tools invocations. P12 remains partial; final shared-tree validation is pending. See docs/design/terminal-native-preview.md.

# Six Advanced Settings reference revision — validation pending

Advanced now contains the six owner-requested pages, with the other ten tools
merged into ordinary categories. Original laptop/monitor/tablet SVG artwork uses
firmware-reported ACPI device type via a read-only native inventory call. Actual
GPT disk extents replace the earlier metadata placeholder. See
[design and exact limitations](../docs/design/advanced-settings.md).

Native reference layouts and typed registry edits are integrated. Full Windows
administration parity, pixel-exact fidelity and frame cadence remain unqualified;
missing providers are explicit. Host, native screenshot and independent review
checks are being refreshed. P12 remains partial; historical reports and owner data
are preserved. Earlier checkpoints below retain their original scope.
# Advanced Settings: partial native implementation

All sixteen requested Advanced destinations now have native Settings pages and
original vector artwork. Registry browsing, explicit path creation, typed checked
edits, endpoint discovery, inherited environment, authenticated identity, local
object metadata and requested diagnostics are integrated. Existing appearance,
taskbar and context-menu controls remain service-backed. Full Windows-equivalent
administration is not implemented: missing providers are explicit and inactive.
See docs/design/advanced-settings.md for the exact available behavior and limits.

Independent review identified draft target rebinding, pointer/search focus,
escaped address identity and pointer scroll issues; fixes and regressions are in
place. Final host/native validation is pending at this source checkpoint. P12
remains partial, historical evidence is unchanged, and no phase is closed.
# Native viewer and title gestures � bounded acceptance

Independent review accepts automatic supported adapter-mode selection, child-only
SDL DPI requests, thresholded title dragging with saved-size restoration, and
installed known-folder artwork. Validation passes701 host tests/34 executables,
265 desktop checks/ten sessions,19 clean isolated4K WHPX checks and retained audit.
Only Window/Workspace change among21 actual artifacts; five repeat pairs match.
See docs/design/native-viewer-title-gestures.md and
reports/critics/p12-native-viewer-title-critic.json. P12 remains partial.

The clean136scene drag paint scopes average15.14706ms on a10ms clock, maximum30ms,
zero failures. This is elapsed paint work, not60FPS, pure CPU or input latency.
The automatic launch was headless2560x1600/2x; visible SDL quality and fractional
scale remain unqualified. Out is16,893,274,320B below its20GB cap; owner data is
untouched and D: is unconfigured. Cumulative verify still stops at the Legacy8GB
reservation; check-spec still needs the original missing P00report32560.

Latest owner concepts start Settings' collapsible Advanced group and unified
administration pages, followed by Explorer, Preview, Terminal and Monitor. See
docs/design/unified-apps-owner-concepts.md and the five retained images in
reports/runs/user-reference-20261021-unified-apps/index.json. This is next work;
those apps/features are not claimed complete. Snap chooser, Search controls,
modern Explorer details, durable trash, display cadence and providers remain open.
The older checkpoints below retain their historical scope and results.

# Cached move rendering and Documents artwork

Bounded independent review accepts cached soft shadows, direct opaque rows,
guarded client-interior move reuse, one completed-scene overlay and installed
Documents artwork. Current validation passed693 host tests/34 executables,
255 desktop checks/nine sessions,19 isolated4K WHPX checks and retained audit.
Only Window/Workspace changed among21 actual artifacts; all five repeat pairs match.
See docs/design/cached-window-shadows.md and reports/critics/p12-cached-move-critic.json.
P12 remains partial; the older checkpoints below retain their historical results.

The final137 drag paint scopes (136scene+1bar) total133 compose/75 transfer ticks:
15.18ms mean elapsed work on a10ms clock, with30ms maximum and zero failures.
The baseline was136scene paints/27.13ms; these aggregates are not identical.
This does not establish60FPS, vblank, pure CPU time or input-to-display latency.
Visible SDL/native display scaling, modern Explorer and remaining Snap/Search/
hover/trash/application work stay open. The owner volume was untouched; D: is not
configured. Out is16,892,989,473B, below its20GB cap. Cumulative verify still stops
at the legacy8GB reservation, and check-spec at the original missing P00 report.

Next: Native host display/DPI and truthful scale controls, modern Explorer tabs/header/columns and selection, drag-to-restore/Snap, Search presentation and shared hover; then durable trash and real app providers. P12 remains partial.

# Earlier paint-measurement checkpoint

Bounded review accepts all returned scene/caption/icon/bar paint aggregates,
checked 16 KiB caption/icon spans and yielding when more damage is pending.
684 host tests/34 executables, 255 desktop checks/nine sessions, the dedicated
4K WHPX run (19 checks) and retained validation pass. Only Window changed among
21 actual guest artifacts. See docs/design/motion-measurement.md and
reports/critics/p12-motion-measurement-critic.json. P12 remains partial.

The 136 dragged paint scopes total 303 composition and 66 transfer ticks at
10 ms resolution: 27.13 ms mean monotonic elapsed work, including scheduling.
This is not pure CPU time, FPS, input latency or a before/after speedup proof.
Composition is the larger measured interval; further performance work is required.
The full cumulative verify remains blocked by the legacy 8 GB admission reserve;
out is below its 20 GB cap. Protected evidence and owner data were preserved.

Latest owner requirements: correct Documents/library artwork, one native display
default with proper scale controls and sharp SDL presentation, drag-to-restore,
configurable Snap layouts, icon-only Search, shared hover transitions and softer
cached shadows. Details are in docs/design/display-and-snap-followup.md. These
remain open alongside Explorer columns/layout, durable trash and real app providers.

# First-run desktop launch ready

An explicit init-user-volume command creates only a NEW persistent128MiB volume
using synced, independently audited staging and atomic no-replace publication.
Existing/partial/unknown owner data is refused. Valid interactive desktop options
and exact default/software private consumers use retained v2 launch output rather
than the unrelated8GB build reserve. Host674/34, private4K WHPX and1x TCG18 checks
each, retained desktop validation and actual initialization pass independent review.
See docs/build/desktop-first-run.md and reports/critics/p12-first-run-critic.json.

The new out/session/user.raw is now mutable owner data: never reinitialize it,
borrow it for a regression, or force its creation hash after the user starts it.
The user can run cargo xtask desktop or add --resolution 3840x2160. Visible SDL/actual owner boot were not tested by
root. The out-only20GB cap and18GB guard remain; no external drive was configured.
P12 remains partial. Next measure all-frame motion and transfer overhead, then
continue modern Explorer, Recycle Bin/restore and truthful app/provider work.
# Shared control rows and acquired icons

Bounded independent acceptance covers shared Workspace label/icon alignment,
Notes caret/selection/footer and Files F2/selection, plus installed Home/txt icons
and safe bounded acquisition. Host668/34, complete desktop253+2 checks/nine actual
sessions, Notes39/Files19/permanent-remove15 each at1x/3x over twelve boots, and
retained validation pass. See docs/design/control-row-alignment.md and
reports/critics/p12-control-alignment-critic.json. Only host pixel fixtures changed after
the qualified guest image; the exact source bridge is retained. Two intermittent
host-tool fatal exits remain unexplained; failed reports and passing diagnostics
remain available. P12 and cumulative qualification are still open.

The six exact standalone Notes/Files/remove consumers now also use retained v2
QEMU output; all other graph admission rules remain. Additional owner references
are in reports/runs/user-reference-20261021-details/index.json. The host classic
context menu is intentionally enabled by the owner, so modern menus need separate
primary references. Details columns, row focus/selection outlines, rounded fields,
modern row ordering/tabs, shell captions, Start/taskbar/widgets/power/search and
unified Settings/System Tools remain open. Next prepare safe first-run QEMU use,
then measured motion and durable Recycle Bin/providers. Never use the owner disk
as a regression fixture. The out-only20GB cap remains; D: is not configured.
# Accepted retained desktop layout; owner detail follow-up

Exact test desktop-current now retains new bulk builds/images/QEMU trees with its
run. Independent bounded acceptance covers657 host tests,253 desktop+2 maintenance
checks/nine sessions,21 byte-identical guest artifacts, old/new retained validation
and actual40-leaf/205,996,028-byte Cargo cache pruning. Out is16,758,530,855 bytes.
See docs/build/output-layout-v2.md and reports/critics/p12-output-layout-v2-critic.json.
Other commands keep legacy admission; cumulative qualification and P12 remain open.

Latest owner work: shared text/icon/caret alignment, correct file-type/Home icons,
current context menus/taskbar personalization, real Recycle Bin and restore,
measured60FPS drag target, then core apps. See windows-detail-followup.md for
references, backend requirements and limits. D: USB NTFS Backup has been identified;
no outputs moved and no drive configured. Computer Use was stopped again by Escape.
Next fix shared alignment and precise native motion measurements, preserving real
storage semantics and keeping new app/provider work within sequential phase gates.

# Accepted Storage connection recovery

Client retirement, semantic reply checks and pollable authenticated discovery have
bounded independent acceptance:651 host tests/33 executables,64 explicit retirement
cycles over two native SDK boots,253 desktop checks plus two maintenance checks in
nine sessions, and42 expected foundation scenarios plus two interactive runs.
See docs/design/storage-connection-recovery.md and reports/critics/p12-storage-connect-critic.json.
No uncertain mutation is replayed; injected native GUI failure behavior remains open.
P12 remains partial. Current out is16,758,519,063 logical bytes; target and reports
are excluded from the owner's20GB limit. Protected evidence exceeds the12GB heavy
build admission threshold. Next implement versioned output layout/retention before
more heavy suites, then measured4K motion, shadow/layout polish and P12 toolkit.
Cumulative verification stops at the original missing P00 report; later historical
evidence dependencies have not thereby been validated. Preserve all old reports.

# Owner-directed rendering and application work

Owner cleanup removed prior out dependencies after review. Preserve historical
acceptance and record current availability separately. Bounded out-only20GB
maintenance and fresh guest evidence are independently accepted in
reports/critics/p12-output-budget-critic.json. Preserve the accepted Storage recovery checkpoint above.
See docs/build/output-storage.md. P12 remains partial.

Permanent Files removal has bounded independent acceptance: host631, foundation42,
combined253/9 sessions and real SDK/1x/3x GUI plus consumer regressions. Keep P12 partial and preserve the
explicit permanent Shift+Delete scope, safe Cancel and real trash/restore backlog.
Next repair Storage connection recovery, then continue shared toolkit/providers
and the catalog/terminal foundations needed before expanding enabled apps.

Shared document dialogs now have bounded independent acceptance: host613,
complete desktop253/9 actual sessions, retained-evidence validation and Notes39 /
Files19 two-boot workflows at1x/3x. See document-dialogs.md. Checked permanent
Files deletion, remaining toolkit and provider work follow within partial P12.

The P12 density/transfer repair passed its current242-check native regression;
record its bounded critic and continue real Files mutations. Checked same-parent
rename now has full host, native SDK, two-density/two-boot GUI and foundation
evidence; its final binding is in docs/design/files-mutations.md. Copy/move/trash/
restore and shared text/controls remain active work. The installed Windows11
baseline and app/functionality inventory are recorded in docs/design/windows-core-apps.md
and project/windows-app-inventory.json. Real Files mutations and
unified terminal/settings designs remain active-phase consumers or later-phase
work according to their service dependencies. The owner's request for all core
apps does not remove backend, negative-case, evidence and critic obligations.

# Phase roadmap

One phase is implemented at a time. `phases.json` is the machine ledger; the
master specification section 91 defines full prerequisites and exit gates.
Phase passes describe measured engineering subsets. P00â€“P10 are closed for their
declared profiles, including the repaired ordinary persistent text session and its
file/application workflows. docs/audits/p01-p10-readiness.md preserves the earlier
audit; docs/testing/application-trials.md records current measured behavior.
One third-party source port is qualified; stock Windows application compatibility
remains unqualified. P11 native window/composition passed its declared profile.
Each closure needs current tests, integrated docs,
evidence and independent critic review. There are no percent-complete estimates.
P04 implementation, current full gate and independent critic acceptance passed.
P04 ring-0 worker execution does not satisfy P05 user isolation.

| Phase | Objective | State |
|---|---|---|
| P00 | Project foundation | passed |
| P01 | Build and image infrastructure | passed |
| P02 | Independent boot | passed |
| P03 | Memory and CPU fundamentals | passed |
| P04 | Scheduler, SMP and synchronization | passed |
| P05 | Protected native execution | passed |
| P06 | Block devices and filesystem base | passed |
| P07 | Persistence, registry and security metadata | passed |
| P08 | Native user platform and services | passed |
| P09 | First Windows x64 program | passed |
| P10 | Windows base semantics and pre-GUI usability repairs | passed |
| P11 | Native window and composition platform | passed; native software profile |
| P12 | Native UI, text and accessibility | in_progress; partial, unqualified |
| P13 | Windows GUI integration | not_started |
| P14 | Complete desktop and visual fidelity | in_progress (parallel authorization 2026-10-26, ADR 0020) |
| P15 | Network, audio and media foundation | in_progress (parallel authorization 2026-11-02, ADR 0021); implemented and critic-reviewed, formal gate after P12-P14 |
| P16 | COM, services and installers | in_progress (parallel authorization 2026-11-02, ADRs 0022, 0024-0026); implemented and critic-reviewed, formal gate after P12-P15 |
| P17 | 32-bit Windows applications | in_progress (parallel authorization 2026-11-02, ADR 0023); implemented and critic-reviewed, formal gate after P12-P16 |
| P18 | Managed and modern application frameworks | not_started (plan proposed 2026-11-11: docs/design/p18-plan-20261111.md) |
| P19 | Native GPU acceleration | not_started |
| P20 | Windows graphics and games | not_started |
| P21 | WDM binary-driver compatibility | not_started |
| P22 | WDF and specialist driver frameworks | not_started |
| P23 | WDDM and vendor driver stacks | not_started |
| P24 | Physical hardware and installer | not_started |
| P25 | Security, servicing and recovery qualification | not_started |
| P26 | Performance and expanded compatibility qualification | not_started |
| P27 | Qualified release and maintenance | not_started |
| P28 | Optional Linux environment | not_started |
| P29 | Additional ISAs and legacy environments | not_started |

The initial dependency spine is P00 â†’ P01 â†’ P02 â†’ P03 â†’ P04 â†’ P05.
Later branches retain all specification scope, including driver frameworks and
optional Linux/additional ISA work; none is scaffolded or claimed here.



P10 implements the bounded Windows base profile described in
specs/abi/windows-base-p10.md. Its full gate extends inherited coverage to42 guest
cases, both interactive checks,16 reproducible artifacts, original P09 reference
comparison, P10 Windows reference and actual save/fresh-boot reopen byte audits.
The ordinary C/MASM fixture includes failed DllMain initialization, duplicate
references, unload/reload, existing-thread compiler TLS, callback ordering,
concurrent references/errno, memory allocation failures, actual fault continuation
and caller recovery, event/APC ordering and pending I/O cancellation/close/query
races. Native kernel probes inject every dynamic DLL acquisition failure before
publication and verify absent mappings/TLS plus baseline frame ownership. No
finite suite guarantees absence of every defect. Final cumulative report
reports/runs/host-1792214999674635300-12500/report.json and independent
reports/critics/p10-critic.json passed: 341 host tests, 42 guest cases, both interactive
checks, two matching 16-artifact builds and zero unresolved review findings.

Current shell refinement and P12/P14 boundaries: docs/audits/p11-desktop-progress.md.
Task View/Snap are development work; P11 does not imply full shell completion.
Development2026-10-20: integrated273 checks and final GUI160 checks passed;
the latter includes final owner/gesture denial and complete4K Overview capture.
See STATUS/HANDOFF for report identities and the distinct tested revisions.
# RPC responsiveness development

The pinned private SDK metadata sample fell from 128 to 32 ticks with up to eight
cooperative turns before the existing timer backoff. Current host537, foundation42,
combined242, native4K15 and both Files19-check regressions pass; the independent
critic accepted this finite change. Preserve the evidence in rpc-responsiveness.md.
This finite sample does not establish whole-app or presentation latency.

# Hinted typography development

Checked build-time variable-font caches now serve1x/2x/3x. Preserve the static
baseline and isolated native constructor comparison, host542/combined242/4K15,
Files19 at1x/3x and foundation42, with the accepted bounded critic binding.
Full Unicode/IME and Windows pixel equivalence remain open. Continue measured
CPU-composition improvements and real shared-control/application functionality.


The finite cpu-composition.md batch is accepted with host547/combined243/native4K15,
Files19 at1x/3x and foundation42. Preserve exact row semantics and legacy admission
refresh. Next measure sparse exact avatar coverage; continue shared P12 controls
and actual app backends. GPU/vblank and full Windows fidelity remain open.
Sparse avatar candidate: original129,024byte coverage for96/192/288radii,
unchanged fallback and large-radius overflow repair. Native15/resolution52/combined243
and host550 now pass; independent bounded review accepted. Preserve its evidence.
Interactive launch selects the tested WHPX/RAMFB profile by default, with explicit
software mode. Host555 and both16-check native paths pass with independent bounded
acceptance; preserve its source/guest-build bridge and headless-only limits.


P12 Notes editing batch is independently accepted with567 host tests,243 combined checks and both13-check/two-boot workflows. Preserve notes-editing.md bindings. Continue real shared controls and durable app backends; full app inventory/Notepad parity/P12 closure remain open.


P12 measured text-navigation controller is independently accepted in its bounded
scope:579host,26Notes checks/two boots each1x/3x,19Files each and16default4K.
Visible-row drag and visual navigation build on the separate Notes checkpoint.
Checked compound document replacement/save-conflict protection is next; P12 partial.

P12 checked compound document replacement/save outcome independently accepted:595 host,42 expected foundation outcomes,243 combined, Notes26/Files19 each1x/3x and SDKtwo-boot/raw audits. Preserve checked-document-save.md evidence. Next shared modal controls and Notes Save/Don't save/Cancel; P12 remains partial.



