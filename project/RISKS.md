<!-- p15-p17-20261111 -->
# P15-P17 risks — 11 November 2026

- **Width bugs in shared DLL code.** The 21 DLLs build for x64 and x86
  from one source; a structure read at the wrong width or a Doors-made
  handle above 2 GB breaks only 32-bit programs. X86-A/B/C cover the
  common paths; less used structures (some winmm MCI parms, mixer
  details, wininet cache entries) have host layout tests but no 32-bit
  probe.
- **oleaut32's universal marshaler is x64-only**: a 32-bit program that
  marshals a type-library interface across processes gets
  CLASS_E_CLASSNOTAVAILABLE.
- **COM throughput and latency** (ADR 0025): 232-byte packets relayed by
  Init and a 10 ms STA poll; large arguments or chatty interfaces are slow.
- **Capacity**: 16 services, 13 processes, 8 kernel ports, 24 COM packets
  queued in Init, four private DLLs per process, and executables of at
  most 1 MiB (Init captures a program whole before mapping it; Run as
  administrator hashes the same bytes); installers that register many
  services or COM servers, and most setup programs and runtimes (P18's
  `dotnet.exe`, `java.exe`, `node.exe`), hit these first. Raising them
  needs the owner (capacity pass, P18 M0h).
- **Guest evidence depends on a quiet host**: under heavy host load
  (parallel builds) the image 9 regression saw a timed-out p16-crypt
  session, a keystroke lost in Terminal and MIDI sequencer jitter. Guest
  regressions run with nothing else building.
- **No evidence for device loss** (QEMU cannot remove the sound device),
  and SCM-R only runs under TCG (WHPX freezes on guest reset).
- **The registry store is small** (measured 11 November 2026): 128 keys and
  384 value records per volume, 48-unit names, 2 KiB values, and the hive
  rewritten whole in each change's journal transaction. One application
  with COM classes uses tens of keys; a .NET or Office-class installation
  needs thousands. Value names of 48 units also block AppCompatFlags
  layers and other path-named values. Raising the numbers in place would
  multiply every registry write's cost; a paged, variable-length hive with
  a migration from today's format is planned as the first milestone
  before P18's runtimes (NEXT_STEPS).
- **Services share the user's desktop.** Since 11 November 2026 services
  run as LocalSystem in the service account's own logon session and keep
  running across sign-out (ADR 0026 item 6), but Doors has no separate
  non-interactive desktop: a service's windows appear on the signed-in
  user's desktop (Windows keeps them in session 0), and services start
  at the first sign-in of the boot rather than at boot. The window
  server's non-interactive desktop is planned with the registry work.
- **A 64-bit package's 32-bit DLL custom actions fail** (no custom-action
  server of the other bitness); 32-bit packages run whole in the 32-bit
  msiexec.exe.

<!-- p14-wsc-20261026 -->
# P14 risks — 26 October 2026 (WS-C)

- **Every desktop launch now passes the credential gate.** Runners answer it
  with the test PIN (`desktop::sign_in_keys`); any runner that drove sign-in
  itself with Return must use the launch helpers. An unreachable session
  service keeps the gate closed (no passwordless fallback).
- **Volatile credentials** until `/state/credentials` is provisioned: a reboot
  asks for a new PIN; the evidence says `persisted=0`.
- **Guest entropy is jitter-only** (`qemu64-v1` has no RDRAND/RDSEED); health
  under TCG/WHPX is unproven. The P08 workload fixture now requires READY.
- **Argon2 cost under TCG.** 4 MiB x 3 passes per attempt; the client deadline
  is 5 s. Five wrong PINs in `sec-02-trusted` cost five derivations.
- **Session-service lock order.** The session service resolves storage through
  Init while serving the window server; Init must never wait on the session
  service at that time (it calls LOGON only after the gate grants).
- **Lockfile churn.** WS-A/WS-B crates change Cargo.lock; check-spec fails until
  `lockfile_sha256` is re-pinned.
- **VIS-01** is uncalibrated and its motion dimension fails closed.

<!-- p12-records-20261024 -->
# P12 risks — 24 October 2026

- **Staged large replacement is new and guest-unverified.** Notes saves up to the
  4,243,456-byte per-file cap through the storage service's staged whole-file
  replacement (BEGIN/WRITE/COMMIT/ABORT, one journal transaction at commit; two
  staging slots service-wide). Host crash-point tests pass; TEXT-01 must audit the
  saved bytes on the raw disk before this is treated as qualified.
- **Kernel process table.** `MAX_PROCESSES` was 13 and the desktop sat at that
  limit (the `multiwindow` scenario's six apps no longer fit beside the network
  and audio services). Owner decision of 13 November 2026: 32 (executive,
  telemetry ABI version 4, kernel arenas re-laid out in the top PML4 slot,
  alias/allocation records 320 to 640, loader table pool 1024 to 2048 pages).
  The nine session processes leave 23 slots. Threads (`MAX_TASKS` 16), kernel
  ports and per-process private DLLs keep their limits until P18's capacity
  pass (M0h), which the owner approved on 13 November 2026 (raise as the
  targets need).
- **Intermittent guest stall (13 November 2026).** About once in a hundred
  TCG runs of the P10 `windows-base` guest, progress stops with no fault or
  panic: once in the Windows fixture after its exception test (P17 gate
  attempt 10), once in Init's logoff call to the session service (stress
  rig), both at a disk write. Most likely cause, fixed the same day: a
  block request spins with interrupts disabled, its 2 s limit used the
  tick count (which stands still then, so only a 10-million-spin bound
  applied), and every spin read the device status register, a trap into
  QEMU under its global lock that can hold off the thread completing the
  request. The used ring is now read every spin and the status register and
  clock every 1,024; the clock is the ACPI PM timer (it runs with
  interrupts off), also for the network and sound drivers; a request is
  declared hung after 30 s. Not captured with registers, so the fix is
  verified only by the gate runs that follow; a kernel watchdog that dumps
  task states on a long all-idle stretch remains the next step if it
  recurs.
- **IME dictionary size.** `assets/ime/japanese.dict` is 31,411,385 bytes, 94% of
  the 32 MiB per-file limit; any dictionary growth fails the image build.
- **Archive ceiling.** `MAX_ARCHIVE` (`fs/initfs/src/lib.rs`) is 256 MiB, the
  UEFI allocation limit. Fonts, the asset pack and dictionaries share it, so the
  archive cannot simply be enlarged.
- **Gate length.** The P12 verify adds 18 desktop scenarios (accelerated variants
  need WHPX) to the inherited suite, so a full run is long and conflicts with the
  owner's interactive QEMU session.

# P12 and P13 risks — 23 October 2026

- **Fonts are private and not redistributable.** P12 uses the owner's installed
  Windows fonts (Segoe UI/Variable, Emoji, Symbol, Microsoft YaHei, Yu Gothic,
  Malgun Gothic) for this private build. Images, captures and archives containing
  them must never be published; provenance records "owner-local, not
  redistributable", and another machine cannot reproduce the image without its own
  licensed copies. Fonts load at runtime from initfs shared sections, not
  `include_bytes!`.
- **Binary size limit.** The kernel loader accepts at most 16 MiB per executable.
  Toolkit, Unicode data, IME engines and dictionaries push the apps over it unless
  the W9 per-app split and runtime assets hold. Track every binary size in the
  desktop build record.
- **QEMU time.** The cumulative P12 verify re-executes every inherited command, the
  42 diagnostic guest scenarios, the ordinary session, the P11 desktop workflows and
  five P12 suites; P13 adds five Windows GUI suites. Runs are long and conflict with
  the owner's interactive QEMU session, so the coordinator schedules them. Verify
  fails before any build when a suite has no runner, to avoid wasted runs.
- **Dictionary licences.** Pinyin, Japanese and Korean engines need openly licensed
  dictionaries; each licence, attribution, version and size must be recorded in the
  dependency and provenance records before use, and dictionaries also count against
  the image and binary budgets.
- **Historical evidence aging.** P00–P11 reports cannot be re-inspected. The
  reviewed disposition is limited to those phases, binds the original run IDs and
  critic files, and current claims rest only on the cumulative P12/P13 verify. A
  forged or incomplete disposition fails `check-spec`.
- **Parallel phases.** P13 depends on P12 primitives (text, input, DPI, window
  protocol). The ledger lets P13 progress but never pass first; protocol changes in
  `crates/service-protocol/src/window.rs` need coordinator sequencing with W8.
- **Concurrent edits without commits.** Several streams edit the tree at once; a
  crate mid-migration (currently `door-window`) blocks integrated `door-xtask`
  tests. Keep file ownership strict and rerun integrated checks once the tree is
  consistent.

# P13 critic limitations — 26 October 2026

The P13 critic (2026-10-26) reviewed the five passing Windows GUI runs (F1
`host-1792978557742642400-19916`, F2 `host-1792978759096854700-9180`, F3
`host-1792974192286403000-3732`, 141.6 `host-1792974407573999400-18584`, 141.5
`host-1792974609428174400-14028`). They predate the critic fixes and are replaced
by the final P13 verify. Recorded limitations (not claims):

- Hung containment was a fixed close deadline with no ghost (F3 shown and hung
  screendumps identical); being replaced by the Windows semantics (5 s without
  message retrieval, "(Not Responding)" ghost, End task or Wait). The F3/141.5
  runners check the window server's `DOOR_P13 WIN32 NOT_RESPONDING`,
  `NOT_RESPONDING_PROMPT`, `WAIT` and `END_TASK` records; owner accounting needs a
  new `DOOR_P13 OWNER_RELEASED` record (requested; the check fails closed).
- IsHungAppWindow and SMTO_ABORTIFHUNG timings are fitted to the reference.
- ASCII-only typing, no IME for Win32; same-process SendMessage only.
- Limits: 4 windows per process, 64 user32 thread lifetimes, 8 threads, 64 MiB
  heap (the 64-large-block heap limit is removed: large blocks now live in a
  growable sorted table).
- comctl32 is the status bar only.
- GDI: nearest-neighbour StretchBlt, no monochrome bitmaps, no italic or
  escapement, substituted faces, no DIBSECTION from GetObject.
- kernel32: no GMEM_MOVEABLE, ACP is UTF-8, `__CxxFrameHandler3` exits the process.
- 100 % DPI only; no Win32 accessibility.
- Serial evidence was not retained (fixed: runs now copy serial logs, staged image
  records and screenshots into `reports/runs/<id>/retained/`).
- MRU restore was unproven: the first reference run added 16 ComDlg32 MRU entries,
  which the owner approved removing on 2026-10-26 (backup
  `C:\Users\win\Desktop\Doors OS\comdlg32-mru-backup-20261026.reg`); the runner now
  undoes its own changes value by value and never deletes the key.
- Open critic findings owned by the GUI workstream: WM_CLOSE handling, thread-exit
  cleanup, window quota failures, key-state snapshots, timed-out send buffers,
  EnableWindow locking, GDI clipping/overflow, CAP_WIN32 enforcement.
- The P13 application-path rollback self-test (`DOOR_P13 APP_ROLLBACK`) runs once
  per boot before the first ordinary application and adds its sweep (one VM-guard
  hold per checkpoint) to that first launch.

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

The current Workspace PT_LOAD span is 16,617,472 bytes: only 159,744 bytes remain
under its 16 MiB bound. Further monolithic app growth needs an explicit executable/
asset split. CPU framebuffer presentation and unknown visible SDL resampling still
prevent a claim of measured Windows-level smoothness or pixel identity.

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

# Generated output retention

The owner's deletion of out removed dependencies of historically accepted reports;
unchanged report bytes do not imply their retained evidence is still available.
Restore verified prerequisites and use fresh run IDs. Automatic out-only20GB
maintenance preserves evidence/owner disks; protected data can exhaust admission.
Headroom/polling is a tooling safeguard, not an NTFS quota against unrelated writers.
Target/reports remain outside the owner-selected limit. Host642/foundation42 and
desktop253/9 sessions pass with bounded independent review. Out is8.10GB logical;
protected execution evidence grows. A new evidence layout is needed for repeated
qualification without accumulating protected out data. See docs/build/output-storage.md.

# Permanent Files removal limits

Shift+Delete permanently removes one observed file or empty folder. Ordinary
Delete/Recycle Bin, recursive removal and restore remain unavailable. GUI names
are bounded to32 printable ASCII characters until exact Unicode presentation and
overflow handling exist. Failed post-mutation reclaim, including deferred close
and revision exhaustion, retires the storage owner. Uncertain replies require a
fresh listing; acknowledged deletion is never retried because refresh failed.
Bounded native/integration tests and independent review pass; P12 remains partial.
Host fault injection does not qualify native injected I/O failures. Connection
retry/remount remain separate work; the current fresh-listing requirement does
not itself heal an unavailable Storage endpoint. Legacy generic replacement
rename's reclaim guard remains a separate follow-up; actual Ext2/Journal write
failure poisons its cache, and the checked-deletion path has the stronger guard.

# Checked Files rename limits

Object IDs and versions are hints; current ancestry, ACLs and delete sharing must
be checked again at mutation. Commit failure poisons live storage and must never
produce UI success. The native component bound is 48 UTF16 units, directory views
are bounded, and display text remains ASCII. Known-folder relocation, copy/move,
trash/restore and full Windows folding/namespace behavior remain unavailable.
The explicit SDK exercise owns a private disk clone; moving it into the normal
save/reopen flow would violate the preserved read-only/checkpoint contract.
GUI evidence binds build 16368; build 13324 changes only the SDK fixture and host
tools and has identical production firmware/kernel/initfs and application bytes.
Current test-image recovery is separate from that production-byte binding.

# Current rendering repair limits

The display capture bound changes to16KiB and uses the existing guarded64KiB
kernel stack. Complete caller validation and authority checks remain required.
BGRX REP copies must preserve padded stride; RGB format retains its conversion.
Native LCD text assumes an RGB-stripe panel at native presentation size; alpha
surfaces use grayscale and arbitrary stripe/rotation calibration is unavailable.
4K300% keeps1280x720 logical geometry. QOI previews are bounded, cached and must
remain within the unchanged16MiB executable and64MiB application heap limits.
The current combined native report25240 passes242 checks; final bounded critic
binding is recorded separately from phase closure.
The interactive launcher now defaults to WHPX/RAMFB; ordinary cumulative tests keep
TCG. RAMFB backing is live reserved BootData with normal RAM caching. CPU lookup
may bypass CPUID only after the kernel seals the validated single-CPU topology;
SMP regression passed15 current native cases. The final4K accelerated run24108
passed14 checks, with no sampled dragged frame at or above the80ms logging
threshold after the exact corner cache. Earlier pre-cache90â€“100ms drag timings
remain historical evidence. Coarse10ms ticks and a finite gesture do not establish
60Hz, vblank, tear-free output or universal smoothness.

Historical phase reports referenced by the ledger are currently absent from
reports/runs after external output cleanup. Preserve their recorded identities;
do not recreate old passing evidence or infer cumulative qualification from new
focused tests. The cumulative verifier must report the missing evidence honestly.

# Current risks and acquisition gates

The responsiveness/address batch changes regional CPU composition and fixed QHD
density, not the kernel/display ABI. Partial raw-pixel callbacks must honor the
physical dirty clip, preserve cached pre-overlay pixels and transfer every painted
region. Full-scene fallback remains for flyouts, overview and desktop-icon changes.
Exact private-frame comparisons and native lifecycle/capture tests cover this
boundary; GOP copies remain non-atomic and visible tearing/cascading is possible.
Host enlargement of1280x720 remains blurry; native QHD200% avoids that enlargement.
Explorer's C: display aliases are not complete Windows folder/API compatibility.

Current owner-requested desktop refinement: local3840x2400 resources, physical
scanout through4K, CPU composition and exact icon exports. App content/control/text
roles implement live dark/light palettes. Complete per-page fidelity/theme coverage,
dynamic DPI, Unicode/IME, network/audio/Bluetooth/update services and Windows GUI
compatibility remain outside this foundation batch. Finite tests/critic reviews do
not establish universal correctness or1:1 Windows fidelity. The current regression
must cover accepted input draining before close and wallpaper replacement memory.

The historical qualified P08 profile has eight process slots, sixteen scheduler task slots, 32 handles
per process, 96 executive objects, sixteen channel pairs and eight sections (each
at most 64 KiB). Manifest policy bounds each service to one thread and no anonymous
heap; the workload has two threads and at most 1 MiB of anonymous mappings. There
is no native standard-library port. Private roots retain 32 table pages each until complete process retirement.
The shared kernel pool remains 64 pages with 128 mapping records. VM mutations
never hold the scheduler lock across remote retirement; contending CPUs admit IPIs.
Native FS and P09 Windows user GS remain separate. NMI policy remains fatal.

P05 startup failure injection, channel lifetime tests, unchanged-input full
verification and independent critic acceptance passed. P05-C01 exposed partial
channel publication at artificial global handle exhaustion; the fix and both
ownership boundary regressions passed. Later subsystem claims retain their gates.

| ID | Observation / risk | Affected phase | Mitigation / next condition |
|---|---|---|---|
| P11-001 | CPU GOP composition/copy, fixed1024x768/96DPI, single-vCPU TCG; no vblank or acceleration | P11 | Retain explicit CPU transfer feedback and timings; P19 owns GPU qualification |
| P11-002 | Selawik differs from Segoe UI Variable; pinned Windows ICO/CUR references cover only selected assets | P11-P14 | Preserve provenance and measured reference gate; no exact-fidelity claim |
| P11-003 | Protected native Files/Notes use bounded ASCII surfaces and documents; not stock Windows GUI support | P11-P13 | P12 Unicode/IME/toolkit/accessibility and P13 User32/GDI qualify separately |
| P11-004 | Adversarial lifecycle/recovery, cumulative gate and independent critic passed for the declared P11 profile | P11/P12 | Preserve historical evidence; P12 changes need their own checks and full phase gate |
| READINESS-001 | Ordinary text workflows, generic file launch and persistence passed the current full gate and critic | P10 text profile | Resolved for the measured profile; requalify changes against the ordinary suite and cumulative regressions |
| READINESS-002 | Tiny Basic is a source port with a narrow original C platform and upstream parsing limits | P10 application scope | Pin/license upstream, compare exact PE on Windows/Door, qualify large saves and input; do not claim broad stock EXE support |
| READINESS-003 | Development UID capability login, one-vCPU services, bounded dense filesystem, ASCII keyboard and no full Rust std/password/entropy provider | Text OS | Keep bounds explicit; GUI does not implicitly qualify these independent facilities |
| ENV-001 | Exact QEMU/OVMF acquired; actual guest integration and critic passed | P02 | Preserve acquisition hashes/notices; requalify changes to this profile |
| ENV-002 | TCG selected and available; hardware acceleration unqualified | P02 | Test TCG boot first; do not infer WHPX performance/support from presence |
| ENV-003 | Recursive cleanup of ignored `out/critic-probes` rejected by execution policy | Local housekeeping | Disposable probe copy remains; raw regression evidence is separately preserved under reports/runs |
| ENV-004 | Automatic approval review rejected inactive-cache and later reference-closure deletion attempts | Output storage | No files deleted; latest validated plan identifies 60.3 GiB logical output in out/cleanup-plan.json. Earlier compression recovered space while preserving evidence and excluding the owner disk |
| GUI-REF-001 | Official UI kit/control resources are acquired references, but the actual Windows 11 shell capture profile remains unmeasured | P11Ã¢â‚¬â€œP14 preparation | Follow docs/design/windows11-reference.md, freeze a Windows build and capture states/DPI/themes before a fidelity claim; current Gallery release uses an experimental SDK |
| ABI-001 | P02 module/mapping/framebuffer schemas are checked at producer and consumer | P03 onward | Extend schemas and ownership transitions with corresponding tests before new resource use |
| IMAGE-001 | P09 PE subset qualified; broader PE requirements remain unsupported | P09 | Require actual rebased unmodified executable, failure rollback and original DLL bindings |
| IMAGE-002 | Staging is a readonly virtual FAT directory, not an installable raw disk | P02/P24 | Qualify the acquired QEMU backend; add raw disk layout only through its own tested contract |
| IMAGE-003 | General metadata inspection is broader than the executable loader policy | P09 | Selected loader enforces AMD64 console /NOENTRY DLL profile and precise unsupported-feature rejection |
| HARNESS-001 | Actual P02 gate and independent negative-evidence replay passed | P02 | Reverify changed artifacts, launch policy or resource limits |
| ENV-004 | Critic build-cache cleanup of ignored `out/critic-p01-build` was rejected by automatic execution policy | Local housekeeping | Cache remains ignored; small minimized repro evidence is preserved in reports/runs/critic-p01 |
| MEM-001 | P03 frame management covers conventional RAM from 1 MiB to below 4 GiB; 128 concurrent allocation tokens | P03/P04 | Limits fail explicitly; retain reserved/high frames and requalify expanded profiles |
| MEM-002 | ReclaimableAfterCopy is intentionally unavailable to the free pool | P02/P03 | Add explicit copy/retirement lifetime transitions before reclamation |
| BOOT-001 | P03's historical gate covers one CPU; P04 exploratory runs cover one/two/four online CPUs | P04 | Current full gate and critic must bind each distinct topology, observed readiness and degraded case |
| BOOT-002 | Diagnostic fixtures and polled input are development-only | P25 release | Remove injected modes in release profile; qualify full input/idle/security policy |
| BOOT-003 | P03 retains 64 table pages, 32 new mapping slots, a 512 KiB early heap and all boot/ACPI storage | P03 / new profile | Test fallible limits and rollback; structural tables and heap are deliberately retained |
| BOOT-004 | ACPI2/XSDT below 4 GiB and usable ACPI PM timer required; AML/IOAPIC routing unsupported | P03 / new profile | Reject unsupported roots/timers; preserve tables and qualify new platform support explicitly |
| SCHED-001 | Native scheduler is bounded to 16 tasks, four CPU slots, 32 objects and four-object wait-any | P04/P05 | Fallible limits, generation checks and unique membership tests; expand only with a new qualified profile |
| SCHED-002 | Context selection precedes outgoing stack retirement | P04/P05 | Keep the IRQ-safe scheduler lock through the real stack switch and commit acknowledgement before migration or reaping |
| SMP-001 | AP startup supports the selected xAPIC topology with observed contiguous online slots; the missing-AP fixture is deliberate degradation | P04 / new hardware | Reject unsupported discovery/startup modes and record actual online masks; physical hardware and hotplug remain unqualified |
| SMP-002 | Parked APs still use retained idle/emergency stacks and can acknowledge IPIs | P04/P05 | Do not reclaim those stacks or startup backing until an explicit CPU-offline retirement protocol exists |
| SMP-003 | Page-table writer is serialized; remote translation retirement uses acknowledged IPIs | P04/P05 | Preserve memory ordering and acknowledgement before reuse; test actual mapping observation across CPUs, not marker presence alone |
| WAIT-001 | Conditions require predicate discipline and mutex reacquisition; arbitrary owner suspension can prevent progress | P04/P05 | Native contract defines closure/cancel/timeout outcomes; higher-level process/debug policy must control suspension and lifetime |
| EVID-001 | Local mutable source/log snapshots are consistency evidence | Continuous | Reverify after changes; use immutable release evidence later |
| SOURCE-001 | Git is local with no independent backup | Continuous | Owner chooses an authorized independent destination; P00 snapshot is a manifest, not backup |
| LEGAL-001 | Project redistribution license and Windows reference asset rights remain unreviewed | Release/acquisition | Preserve pinned provenance for owner-requested local assets; no publication performed |
| COMPAT-001 | Initial Windows reference and same-binary guest subset qualified | P09 onward | Bind same independent EXE hash to measured Windows output/errors and actual Door execution; no broad claim |

No phase failure is hidden as an accepted deviation. Current code limitations
are recorded in `specs/contracts/boot-info-v1.md` and `docs/testing/README.md`.
Initial critic findings: source mutation during a run, simultaneous/unknown phase
selection, unbound critic acceptance and incomplete firmware acquisition reporting.
Fixes and regression tests passed independent follow-up, with no unresolved
actionable findings in P00 scope. Review: `reports/critics/p00-critic.json`. Initial repro
evidence is in `reports/runs/critic-p00-initial/`; follow-up evidence is in
`reports/runs/critic-p00-followup/`. Those findings no longer block P00.

P01 critic findings covered staged initfs bounds/metadata, independent rebuild
identity, guest result binding/states and non-load ELF file extents. All four fixes
passed the final independent replay and 50-test gate. There are zero unresolved
actionable findings in the documented P01 scope; review: `reports/critics/p01-critic.json`.
Reproduction and evidence checks remain under `reports/runs/critic-p01/`.

P02's initial critic found directory boot configuration, missing private-medium
validation, incomplete demo process/screen validation and a wire-name boundary
bug. Fixes and regressions pass the new 61-test full guest gate. Follow-up critic
acceptance has zero unresolved actionable findings; `reports/critics/p02-critic.json`
binds 66 exact inputs and 122 matched raw process streams. Original raw repros
remain in `reports/runs/critic-p02/`.

P03 closed with 88 passing host tests, twelve actual guest cases and its independent
critic. Its phase-confusion evidence defect was corrected and replayed. Fatal traps
are diagnosed failures; the enclosing suite checks their expected failure. These
historical results qualify P03's single-CPU scope.

P04's current 115 host tests and exploratory fifteen-case guest/demo suite pass,
including real preemption/waits, two/four CPUs and one deliberately missing AP.
The full current-source gate and independent critic acceptance passed with zero
unresolved actionable findings. Remote-acknowledgement timeout was code-reviewed;
this phase did not independently inject that timeout in a running guest.
No phase review establishes absence of all bugs. P05 now qualifies bounded native
userland and N0 runtime; physical hardware and P08Ã¢â‚¬â€œP29 remain unqualified.


P06's ext2 backend is explicitly development-only: one group, 128 inodes, 48 KiB per
file, one backing root directory, ASCII backing components and one coherent 4 KiB
cache page. P07 adds journaled logical namespace, ACL and sharing semantics; P08
exposes them through authenticated service APIs. Polled modern VirtIO is qualified
only on the pinned virtual storage profiles;
physical devices, IOMMU, multi-queue, interrupt dispatch and hotplug recovery remain
unqualified. Device reset invalidates dependent mounts/caches before reuse.
The first guest tests caught 64-bit BAR and stack-budget integration failures; their
fixes and raw failures are retained; the final 190-test/17-guest gate and critic passed.

P07 closed a bounded redo journal for the existing ext2 development
profile, not ext4/JBD2 interoperability. Recovery validates the entire committed record before applying any page and
coordinates contents with ACL/config state.
Crash checkpoints qualify this virtual flush protocol; physical drive power-loss
behavior still requires separate hardware evidence. P07 is qualified only for one serialized owner on the
single-CPU virtual profile; CRC32 does not authenticate media or prevent rollback.

P08 passed its cumulative gate and independent critic. Its single-CPU service profile has eight process
slots, sixteen thread slots and bounded IPC/object tables; exhaustion must fail
without partially publishing authority. Runtime service state is reconstructed
from the existing validated journaled volume after restart. The kernel owns device
DMA while the storage service owns filesystem policy and mutations.

Development authentication accepts kernel-issued single-use logon capabilities;
password verification, usable randomness and a complete native Rust standard
library remain unavailable. N1 allocation and N2-foundation adapters are separate
claims. Session revocation must retire live threads and in-flight kernel operations
before freeing their resources. No host test substitutes for the actual guest
service, restart and console gates.

The first P08 exploratory boot caught construction of expanded executive tables on
the guarded boot stack. Initialization now uses permanent storage. Follow-up found
a secondary-thread virtual reservation overlap and transient overflow in a valid
heap alias calculation; both are corrected with disjointness and arithmetic checks.
Failed runs remain retained. The second cumulative gate
`host-1792059442687006000-11976` passed 278 host tests, all 39 guest cases and both
interactive checks with unchanged inputs, before the latest critic fixes.

P08 critic regressions cover forced session stop while a live foreign process owns
a condition mutex, and console CR/LF injection into diagnostic evidence. Forced
stop bypasses reacquisition only for terminal kernel cleanup; ordinary cancellation
still reacquires. Console serial records escape untrusted text and terminate each
record independently. These P08-C01/C02 fixes passed that second cumulative gate.

P08-C03 exposed completion-event retirement between release of a runtime Thread
lease and its final signal. Retaining that lease through signalling passes the
independent probe and 89 kernel host tests. The early-handle-close guest workload
also passes targeted suite `host-1792060554365351700-4084`, including all four service
boots, disk audits and six console commands. P08-C04 now revalidates authority at final launch
publication and retires failed construction safely. Its independent probe passes;
the coordinated runtime passes 91 kernel host tests and functional critic review.
All four fixes passed final cumulative gate `host-1792060739947826700-17608`:
281 host tests, all 39 guest cases, both interactive checks and unchanged inputs.
`reports/critics/p08-critic.json` accepts the exact report with zero unresolved findings.
P09 cumulative gate and independent critic passed; P10 expands the bounded compatibility contract.

Historical P09 enforces one Windows thread, no dynamic heap, static original DLLs
without entry callbacks and synchronous ASCII output at most1024bytes. These bounds
remain historical; P10 has a separate expanded ownership/negative-test profile.
P09 buffer stability during service IPC relies on its single-thread personality.
The original compiler exception symbol terminates rather than unwinds; Windows
exceptions and C++ runtime behavior remain unsupported. Public PEB/TEB views are
pinned to SDK19041 and do not authorize access. An ordinary reference fixture's
success is evidence only for its measured scenarios, not an application family.

Automatic approval review rejected cleanup of `out/critic-p08/target` and
`out/critic-p08/disk-mutation.raw`. These ignored files remain; retained cleanup
artifacts do not block qualification, and the independent evidence stays intact.

P10 supports actual vectored fault continuation and bounded public x64 caller recovery. Handler-bearing/chained unwind, dynamic function tables and C++ unwinding remain outside this profile. Dynamic DLL/static TLS, bounded async reads and scoped CRT passed the cumulative gate and independent critic. Whole CONTEXT copies use the trusted manifest7-only4096-byte USER_COPY limit; earlier limits remain enforced. The phase ledger and STATUS record final qualification.

P10's declared profile is bounded to five catalog modules, one deferred application
DLL, four concurrent Windows threads,1MiB dynamic commitment, eight file/CRT heap
slots and one pending async request. Compiler TLS is256 bytes/module; API-set
redirects are explicit. Events/APCs, fault continuation and no-handler caller
recovery are implemented; wait-all/named synchronization, arbitrary DLL search,
language/C++ unwinding, dynamic function tables, IOCP/general async depth and full
UCRT/stdio/locale remain outside this qualified profile. See the contract for
precise unsupported/error behavior. These boundaries are not claims about broad
Windows application compatibility. The final critic reviewed unsafe mapping/TLS,
callback/exit, cancellation and retirement races with zero unresolved findings;
finite tests cannot prove that all defects are absent. Failed development evidence remains retained.

P10 async-write code received source review, while the independent Windows/Door
guest async fixture measures reads, cancellation, close and concurrent queries.
A separate unchanged-binary async-write comparison remains unqualified. Full
language unwinding, arbitrary DLLs and broad Windows application compatibility
remain outside this accepted profile. The exact accepted report and limitations
are recorded in reports/critics/p10-critic.json and project/STATUS.md.

The initial GUI repair used100%physical pixels at4K. Current4K output now uses
native200% geometry, typography, assets and input/layout agreement. Default
windowed1280x720 removes forced fullscreen; other modes remain100%. Native
raster/icon exports cannot replace the still-missing dynamic/fractional DPI path.
Nonblocking Init polling and input during scanout reduce long waits; scene
composition, taskbar blur, font initialization and surface capture still consume
CPU time. Bounded QMP cursor samples do not establish universal smoothness/FPS.

Current4K200% rendering has bounded guest regression evidence. It uses native
outlines/assets, not whole-frame scaling.192DPI is fixed for4K only; dynamic DPI,
fractional125/150/175%, multi-monitor output and atomic/vblank flips remain absent.
CPU scene composition and synchronous section snapshots can still delay input.
The256MiB developmentWindow heap and four preallocated large cells bound retained
capture residency to three large windows plus scratch; other quota-valid surfaces
may receive Resources. NoFPS/worst-case latency or zero-bug guarantee is claimed.


Current shell refinement uses a trusted passwordless gate, not credential-based
security. Start shows only three actual native apps. Missing Settings devices and
services remain unavailable. Per-page Windows reference captures, taskbar edges,
contextual desktop menus, drag/marquee and fractional DPI remain outstanding.
Host SDL letterboxing/stretching is separate from native raster quality. Caption
hover and capture interleaving improve bounded paths without qualifying worst-case
fluidity. Overlay transfer retains dismissal extent to prevent stale flyout pixels.

Session desktops are bounded to8, share wallpaper and do not survive reboot.
Bundled apps are global singletons. Full motion/blur, contextual Move to,
Snap Assist and Win+Z remain development gaps.

Sparse framebuffer points can accept partial CPU scanout. Current4K Overview
tests additionally wait for a new successful native transfer marker and actual
card/selection pixels; PNG encodes the same PPM bytes. Other pixel checks remain
bounded assertions, not proof that every captured frame is synchronized or tear-free.


Category refinement retains mixed historical2021/2023 reference images (Display
article updated2025), unknown build/DPI, modern owner Home shell, and old owner
Explorer. There is no single frozen latest-Windows baseline. Unsupported Settings
rows are noninteractive and explicit; row completeness is not service completeness.
Four identical full-frame samples are finite observed stability, not a GPU fence;
a new-state readiness pixel is still needed when old pixels already satisfy a test.

Desktop controls add a six-bit durable preference and authenticated event/page
routing; they do not add complete Windows shell menu providers. File Cut/Copy/
Rename/Share/Delete remain disabled until actual authorized backends exist.
A saved control preference whose Window RPC permanently fails is reported as saved but not
applied; Init retries restoration on subsequent session admission. Runtime apply
failure is not an atomic storage/display transaction. Transient WouldBlock preserves
the pending preference and retries after polling/yielding. Work-area queue preflight,
restore reachability and bounded underlay allocation need continued regression.
All reference screenshots have unverified build/DPI flags;1:1 remains unqualified.
# Bounded RPC wait repair

Cooperative yielding can improve short single-CPU RPCs but is not channel-driven
blocking and adds scheduler work. Keep the fixed eight-turn cap, then timer
backoff, absolute deadline, connection poisoning and authenticated replies. The
128-call coarse-clock sample cannot establish FPS or latency distributions.
Current service failure/restart, actual desktop/4K and Files/reboot regressions
pass and the bounded critic accepted the change. Channel-driven blocking remains
a separate design; preserve all deadline and failed-connection behavior.

# Offline typography candidate

Host font hinting is not guest or Windows equivalence evidence. Keep the cache's
exact RGB stream/offset/metric/kerning/ink bounds and no-growth draw behavior.
Skrifa and its new dependencies are host-only. QOI is the existing guest package.
Font source is owner-local and distribution remains unreviewed. ASCII/static
kerning is not full Unicode shaping. Native comparison/current regressions pass;
bounded independent review accepted cache selection. Four isolated zero-elapsed-tick
samples are not zero runtime or a latency distribution, and coverage-vector
capacity excludes metadata/kerning/mapped assets and other process memory.


Constant-alpha row blending must preserve every destination and padding byte. Skipping
signed-out taskbar material requires a fresh actual backdrop on admission. Native
comparison and consumer regressions now pass with bounded independent acceptance.
The isolated4K8â†’5compose-tick observation and identical retained sign-in PNGs do not
prove general frame rate, latency distributions or whole-sign-in speed. Sparse
avatar coverage is a separate candidate; preserve exact clamping and fallback.
The sparse asset adds mapped read-only native bytes to renderer consumers; actual
ELF memory spans are6,778,880(window)/11,313,152(workspace), below the existing16MiB
loader limit. Actual isolated4K and four-mode captures retain identical sign-in
pixels. Their finite first-frame timing does not establish general responsiveness.
Independent review accepted this bounded cache/arithmetic change; larger app and
Windows reference requirements remain open.
Interactive launch now defaults to WHPX with bounded independent acceptance. Hosts without Windows
Hypervisor Platform must select explicit TCG; failures stay failures, with no
automatic retry against the owner disk. Both actual native paths pass; visible SDL
presentation is untested by these headless runs. No universal host/FPS claim follows.


Notes admits4096 ASCII/tab/CR/LF bytes. Checked save replaces bounded streams
durably with identity/version protection. Close/reload now require explicit
Save/Don't save/Cancel when attention is pending; no close auto-save remains.
Native1x/3x two-boot workflows and bounded review pass. Injected native GUI I/O,
reload, close and approved-close service-loss cases remain unqualified; model
guards do not substitute for those injections. Storage-service consumer
reconnection, Save As, tabs, Unicode/IME/clipboard and accessibility remain open.


Measured text navigation passes native1x/3x and bounded independent review.
Dragging clamps to visible rows; edge autoscroll, Shift-click and unwrapped
horizontal scrolling remain missing. Font/document replacement requires explicit
cache reset, and None revisions force rebuild after counter saturation. Save
replacement with identity/version protection is independently accepted in its
bounded scope; legacy separate write/truncate/flush APIs remain noncompound.

Checked save has independent native/raw-byte acceptance. Unknown commit retains
attention across Undo; acknowledged bytes advance the checkpoint even if close
fails. Initial CreateNew remains separate and can leave an empty file on failure.
Explicit discard choices are accepted; conflict resolution/Save As/recovery UI
remain open. Shutdown canceled by a document choice requires a new power request.
See checked-document-save.md and document-dialogs.md.



