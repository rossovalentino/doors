<!-- host-assets-20261121 -->
# Next — 21 November 2026 (supersedes below)

1. **ELECTRON-A to a rendered window** (`cargo xtask test p18-electron`,
   development rounds with `DOOR_XTASK_DEV_CACHE=1`): round 54 still aborts
   on "Unable to open nul device" after the NUL device was added; find which
   call fails (each round's serial log names failing calls; `DOOR_P18
   BACKTRACE` lines map through the build's `.map` files). Software
   rendering only; GPU acceleration is P19.
2. **VS Code and CEF** (cefclient) harnesses once Electron's window
   renders; then the M7 browser checks.
3. **Add P18 to the phase ledger** and move `P18_PENDING_FUNCTIONS`
   (tools/xtask/src/scenarios.rs) into a P18 desktop scenario list.
4. **Close the documented gaps** where a target app needs them: window
   hooks, GDI paths/metafiles, world transforms, cross-process clipboard
   and drag and drop, TaskDialog icons and expando, SSPI packages,
   DirectWrite text layout.
5. The media card above Quick Settings and the other earlier P18 GUI items.
6. **The P17 gate** when the owner asks for it again (keep 40 GB free).

<!-- p18-start-20261114 -->
# Next — 14 November 2026 (supersedes below)

1. **P18** (`docs/design/p18-plan-20261111.md`, "Progress" lists what is
   done): first guest run of the new memory, SEH and loader paths (the
   P10-P17 Windows scenarios on a P18 image), then file mapping, the UCRT
   (M0d), thread-pool timers, the SMP scheduler and stack growth (M0h),
   until the .NET runtime's JIT, PowerShell 7, CEF and Electron (VS Code)
   run.
2. **The P17 gate** when the owner asks for it again (deferred 14
   November); it now runs from this folder (no frozen copy). Keep 40 GB
   free before a run.
3. NoneDrive and the own-AI page (`docs/design/settings-parity-26h2.md`).

<!-- p17-pregate-20261113 -->
# Next — 13 November 2026 (supersedes below)

1. **The P17 gate** (`cargo xtask verify --phase P17` in the frozen copy),
   which closes P15, P16 and P17 (owner order: Settings first, then the
   gate). Every scenario and suite it runs passed in the pre-gate batch.
2. **NoneDrive** (WebDAV over HTTPS to the user's own server, with its
   setup page) and the own-AI page (local or API endpoint), per
   `docs/design/settings-parity-26h2.md`.
3. **P18** (`docs/design/p18-plan-20261111.md`), M0 first, until the .NET
   runtime's JIT, PowerShell 7, CEF and Electron apps run.
4. Still open from the list below: capture revoked while recording,
   firewall denial and device loss in the guest, INSTALL-A's reboot while
   installed, the negative elevation cases, the TLS alert on a refused
   handshake. Settings: the old page's fade-out and native 30 px symbol
   masks at 150 %.
<!-- p15-p17-close-20261111 -->
# Next — after P15-P17, 11 November 2026 (supersedes below for P15-P18)

P15, P16 and P17 are implemented, guest-tested and critic-reviewed; their
formal gates (`cargo xtask verify --phase P15|P16|P17`) still wait for
P12-P14 to pass. What remains, in order:

1. **Formal gates.** Once P12-P14 pass: `verify --phase P15`, then P16,
   then P17, each recorded with its report and critic.
2. **Evidence still missing for the gates** (record or close before each
   verify; HTTPS success and certificate errors are in CONNECT-A since 11
   November 2026, against xtask's TLS 1.3 test server `p15_tls`): capture
   revoked while recording (guest step plus a ring abstraction for a host
   test); a refused TLS handshake sends no alert (Schannel sends
   bad_certificate/unknown_ca) and the peer sees the connection only at
   its own timeout (CONNECT-A's TLS server logs 15 s timeouts for steps
   17-19); firewall denial in the guest (QEMU's user network keeps retrying
   a dropped SYN, so the step needs a listener of its own, not
   netprobe's); device loss (QEMU cannot remove the sound device: fault
   injection);
   INSTALL-A section 141.8's reboot while installed and before/after
   inventory; negative elevation-boundary cases (a standard user against
   the SCM, the hand-off key, the COM relay and Init's admission of the
   service session).
3. **P18 prerequisites, proposed as P18 M0** (docs/design/p18-plan-20261111.md):
   the registry hive v2 (capacity, 16,383-unit value names, 1 MiB values,
   migration) with the Compatibility tab after it; executable dynamic
   memory for JIT runtimes (owner decision); the Universal CRT; runtime
   support in ntdll (dynamic function tables, thread contexts); NLS,
   Schannel, activation contexts; the capacity pass (owner decision).
4. **Service isolation**: the window server's non-interactive desktop for
   the service account's session, and services starting at boot rather
   than at the first sign-in.
5. **Evidence records**: every artifact of the desktop record embeds the
   whole source snapshot (about 1.8 MB each); with the 32-bit builds the
   record passed 128 MiB (bounds raised to 512 MiB on 11 November 2026).
   Bind the snapshot once per record and have the verifiers accept both
   forms.
6. **P18 itself**, once the owner accepts the plan and its decisions.
<!-- p15-p17-20261111 -->
# Next — P15-P17, 11 November 2026 (supersedes below for P15-P17)

1. Owner decision: the compatibility settings page (Properties >
   Compatibility). Windows stores it under
   HKCU\Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers
   with the program's full path as the value name; Doors' registry value
   names hold 48 UTF-16 units (storage format), so the page needs either
   longer value names or a Doors-specific key.
2. p15-desktop: Preview's Ctrl+O path box does not open in the guest
   (pre-existing; Preview logs no DOOR_PREVIEW OPEN).
3. One read-only critic per phase (P15, P16, P17); fix what they find.
4. Once P12-P14 pass: `cargo xtask verify --phase P15`, then P16, then
   P17, each recorded with its report and critic.
5. Later, with the owner: the kernel capacity pass (processes, ports,
   handles) before bare-metal use; it would also let COM use direct
   channels instead of Init's relay (ADR 0025).

<!-- p15-20261103 -->
# Next — P15, 3 November 2026 (supersedes below for P15)

1. Capture: virtio-sound input queue, door.audio capture streams with
   brokered, revocable authority, Settings > Privacy & security > Microphone
   with the in-use indicator, winmm waveIn; SOUND-A denied/allowed capture.
2. DirectSound (dsound.dll) over door.audio; WASAPI/MMDevice after P16 COM.
3. Device loss and default-device change in door.audio and winmm.
4. VPN: wait for the owner's direction (paused).
5. Then P16 (tokens, COM/RPC, services, installers) and P17 (32-bit).

<!-- p14-wsc-20261026 -->
# Next — P14, 26 October 2026 (WS-C; supersedes below for P14)

1. Record P13 as passed only on the coordinator's confirmed report id.
2. Register `desktop-a-files` and `desktop-a-apps` (WS-B) and any WS-D runner.
3. WS-B: provision `/state/credentials` (owner uid 2, integrity 2, single
   `User(2)` entry) so credentials persist (contract section 4).
4. WS-A: remove the bypassed `lock_key`/`lock_button`/`unlock` in
   `desktop.rs`; keep the extracted shell inert while locked; migrate palettes
   to `door-design-tokens`; draw the door mask in the Start slot.
5. First guest runs (when QEMU is free): `sec-02-trusted`, `vis-01`, then every
   desktop scenario (all launches now answer the credential gate).
6. Record the Start-open motion reference sequence and calibrate VIS-01 probes.
7. Re-pin `specs/sources/dependencies.json` `lockfile_sha256` after the last
   Cargo.lock change, then the frozen `verify --phase P14` and its critic.

<!-- p13-integration-20261026 -->
# Next — P13 Windows GUI integration, 26 October 2026 (supersedes below)

P13-A/B/C are integrated and the five guest suites (`win32-gui-f1|f2|f3`,
`scenario-141-6`, `scenario-141-5`) pass individually on a full desktop build.
Integration fixes: all six original DLLs in every P09+ image (exact import
graph), Terminal program execution with the session console mirrored into the
tab (`CONSOLE_MIRROR`) and as `DOOR_CONSOLE`, Explorer `.exe` launch through
`EXECUTE_DETACHED` (address bar, Enter, double-click), Task Manager naming for
manifest 15, cmd-style command lines, Application-policy PE loading of
language-handler unwind frames and relocation-free ASLR images, UTF-8 console
writes, user32 fixes (dialog focus, menu mnemonics, MB_OK button id, first
window last error, ABORTIFHUNG wake-mask rule, session clipboard, frameless
popups), comdlg32 folder-view timing, window-server activation/move/Alt+Tab
fixes, Terminal `cp` of large files, and the reference runner's exact
ComDlg32 snapshot/restore. Remaining: the frozen-tree
`cargo xtask verify --phase P13`, the critic, records; the Win32 edit control
does not visibly repaint typed text (content and saves are correct).

<!-- p13c-records-20261026 -->
# Next — P13 Windows GUI, 26 October 2026

P12: all suites and scenarios passed individually; the last cumulative verify
passed 435 checks with no failures before the session was interrupted. P13 is in
progress, not verified. The final `cargo xtask verify --phase P13` re-executes
everything once P13-A and P13-B land.

1. P13-A/P13-B: implement against `tests/fixtures/windows-gui/imports.json`
   (every named import of the fixtures, by module). P13-B: make
   `door-user32/gdi32/comdlg32/comctl32` build as `cdylib` with `guest-pe`
   (`dlls::build_gui`: bases 0x188000000/0x18a000000/0x18c000000/0x18e000000,
   imports limited to the declared lower modules); update `Cargo.lock` for the new
   `door-windows-gui-guest` crate (currently `--locked` fails workspace-wide).
   P13-A: add `compat/pe/tests/p13_application.rs`, load GUI DLLs from initfs
   `windows/<name>.dll`, and keep the kernel `DOOR_P09 MODULE` / `DOOR_P08
   PROCESS` records.
2. Stage 2 (WIN-LAUNCH, coordinator assigns): Terminal program execution with a
   console mirrored as `DOOR_CONSOLE` records, and Explorer address-bar launch of
   `.exe` files through Init EXECUTE (contract `specs/contracts/windows-gui-v1.md`,
   "Launch contracts").
3. Then run, in order: `cargo xtask test compat --suite win32-gui-f1`, `-f2`,
   `-f3`, `scenario-141-6`, `scenario-141-5` (each stages `desktop-win32` from a
   fresh desktop build or `--image`). Fix runner input timing only with recorded
   evidence; do not loosen the expected results.
4. Any fixture change: rerun `cargo xtask build --profile windows-gui-reference`
   on the owner's PC (read-only; adds ComDlg32 MRU entries) to republish
   `imports.json` and `reference.json`.
5. Frozen tree: `cargo xtask verify --phase P13`, critic, records.

<!-- p12-records-20261024 -->
# Next — P12 first, 24 October 2026

P12 is in progress, not verified. P13 is authorized in parallel but not pursued
(owner: "do P12 first"). Check the gate with `cargo xtask verify --list`.

1. Stream T re-pins `specs/sources/dependencies.json` to the reviewed
   `Cargo.lock` so check-spec passes again.
2. Register the missing runners in `RUNNERS` in `tools/xtask/src/scenarios.rs`:
   `a11y-01` (U, being added), `text-01` (Notes TEXT-01 with IME and clipboard),
   `display-01` (W8 display scale) and `motion-01` (W7). Stage the gallery and
   assistive binaries in the session image so `component-gallery` can run, within
   the 13-process limit.
3. Implement text size application (P12-TEXT-SIZE), localization and time zones
   (P12-LOCALE) and filesystem timestamps (P12-TIMESTAMPS).
4. Make Notes saves over 4 KiB atomic, or keep the limitation explicit in TEXT-01.
5. When the tree is frozen, run `cargo xtask verify` (P12), then the critic, and
   update the records together.

# Next — P12 and P13 gate, 23 October 2026 (superseded by 24 October)

P12 is the selected phase and P13 runs in parallel (owner decision 6). Neither may
be marked passed; P13 cannot pass before P12. Follow the stream ownership in
[the plan](../docs/design/p12-p13-plan-20261023.md) and the `p12_tasks`/`p13_tasks`
DAGs in `project/phases.json`. Never recreate a P00–P11 report (ADR 0017).

1. When `door-window` compiles again, run `cargo test -p door-xtask --locked` and
   `cargo build -p door-xtask --locked` on the integrated crate; the gate modules
   passed only in the isolated harness so far.
2. Stream T reviews the changed `Cargo.lock` and re-pins
   `specs/sources/dependencies.json`; then `cargo xtask check-spec` should report
   P00–P11 as owner-confirmed unavailable and pass.
3. Each owning stream implements its acceptance runner and asks G to register it in
   `RUNNERS` in `tools/xtask/src/scenarios.rs`: `text-01` (Notes editor with T, I,
   W5), `a11y-01` and `component-gallery` (U), `display-01` (W8), `motion-01` (W7;
   coordinator to confirm), and for P13 `win32-gui-f1`, `win32-gui-f2`,
   `win32-gui-f3`, `scenario-141-5`, `scenario-141-6` (P13-C in
   `tools/xtask/src/desktop/win32.rs`). A runner is
   `fn(&mut Run, image, test_image) -> Result<Value, String>` and returns a
   non-empty evidence object from real guest effects. `cargo xtask test ui --suite
   <name>` or `test compat --suite <name>` runs one suite on a fresh desktop build.
4. As new packages settle (toolkit, accessibility, keyboard layouts, gallery;
   compat/gui and the user32/gdi32/comdlg32/comctl32 DLLs), G adds their guest-target
   checks to `phase_commands` for P12/P13.
5. While suites are pending, the coordinator may run the inherited regression
   `cargo xtask test integrated-desktop`; it does not certify P12.
6. Close P12 with a frozen-tree `cargo xtask verify`, records and an independent
   critic; then close P13 with `cargo xtask verify --phase P13` (and as the selected
   phase once P12 passes).

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

# Reference revision follow-up — active P12

The bounded Monitor/Preview presentation revision passed its selected native and
host checks. Keep P12 partial. The [revision design](../docs/design/reference-apps-20261022.md)
binds the final image, unchanged-binary bridges, 58 captures and review receipt.
Continue measured fidelity and missing providers below. Preserve original phase
history, absent owner storage and the out-only 20 GB cap. Required cumulative
gate receipts and the documentation-only source bridge belong to the ignored
review receipt; missing original P00 evidence must not be reconstructed.

# Next work — active P12, 22 October 2026

The Preview/Monitor checkpoint is validated within the bounded scope described in
[the design](../docs/design/preview-monitor-expansion-20261022.md). Keep P12 partial.
Do not recreate the deleted P00 report or initialize owner storage implicitly.

1. Measure visible SDL frame cadence and input latency; replace remaining polling
   with readiness-driven waits, then profile CPU composition and checked transfers.
   Current 4K paced paint work is 20.11 ms average/30 ms maximum, outside a reliable
   60 FPS claim. Preserve exact pixel/reuse and source-identity checks.
2. Qualify shared UI reconnect with Preview/Monitor alive, native one-megapixel
   image pressure, and configure/close plus CreateNew/cancel/rename reply loss.
   Host state-machine tests are not those native fault injections.
3. Add trustworthy transactional timestamps and schema migration, then Explorer
   metadata, Recycle Bin/restore and robust launch/multiwindow handoff.
4. Extend Preview editing/providers and real Terminal engines/settings without
   advertising unimplemented PDF/media/WSL/PowerShell functionality.
5. Continue measured reference comparisons for both themes, fractional DPI,
   Unicode/IME, modern context menus and small spacing/icon/selection details.
   Complete remaining Monitor providers and authorized controls separately.

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

Next priorities: establish native presentation/input timing and visible QEMU
scaling behavior; implement fractional display scaling and durable Trash; split
new application growth away from the nearly full Workspace executable before
adding Preview/Monitor or further backend providers. Continue reference comparisons
at matching client extent and DPI, with both themes and actual data.

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

# Current repair and expanded owner request

Preserve the accepted out-only20GB maintenance checkpoint: host642, foundation42,
desktop253/9 sessions and identical retained validation before/after actual cleanup.
See reports/critics/p12-output-budget-critic.json and docs/build/output-storage.md.
Original accepted reports remain historical; deleted out dependencies are
unavailable. Never fabricate old execution receipts. Resume Storage transport
recovery, preserving unsaved RAM and never replaying uncertain mutations.

Preserve the accepted shared dialog checkpoint in document-dialogs.md: host613,
complete desktop253/9 actual sessions, Notes39 and Files19 checks/two boots each
1x/3x, strict retained validation and independent review. Explicit Shift+Delete
now has a captured object/version, current authority, durable backend and safe
default-Cancel modal with independent bounded acceptance: host631, foundation42,
combined253/9, SDK three boots and GUI15/two boots each1x/3x plus Notes39/rename19.
Preserve files-deletion.md and its critic/source/build bindings. Next repair
Storage transport reconnection without replaying uncertain mutations or losing
Notes RAM; keep poisoned-volume remount/session revocation as separate contracts.
Normal Delete/Recycle Bin needs
real transactional trash/restore. Keep uncertain outcomes separate from refresh,
and qualify failure/sharing/ancestry/reboot behavior. Storage-service consumer
reconnection remains open. Continue within P12; do not skip later phase gates.

1. Preserve the242-check combined rendering report25240,519 host tests15540,
   four-resolution52 checks23560 and their bounded critic binding. Keep failed
   evidence, explicit acceleration/SMP reports and historical missing-report
   limitation. No full P12 or Windows1:1 closure follows.
2. Preserve the checked-rename evidence in docs/design/files-mutations.md:
   531 host tests, both 19-check/two-boot GUI runs, 242 desktop checks, explicit
   SDK raw-disk roundtrip, 42 foundation scenarios and current recovery. Keep the
   strict legacy reopen/checkpoint assertions and all failed evidence.
3. Extend the acquired Windows11 build26200.9711 reference and 35-family inventory
   with per-page/state evidence. Calculator/Notepad/Explorer/Settings were inspected;
   light/dark Colors was restored to dark. Use primary references for missing states.
4. Continue P12 toolkit/functional Files/Notes/Settings work, including real
   mutation backends and consumer tests. Propose unified settings/terminal
   architecture with an explicit compatibility matrix; decorative controls do
   not qualify functionality. Keep P13/P14/P15 phase prerequisites intact.
5. Preserve the accepted bounded RPC repair in docs/design/rpc-responsiveness.md:
   host537, foundation42, combined242, native4K15 and Files19 checks at1x/3x,
   plus the critic binding. The private sample measured128ticks before and32
   after for128metadata calls; retain deadline/failure/raw-disk assertions.
   Preserve the hinted variable-font cache evidence in typography-cache.md.
   Static4K native baseline is5ticks compositor/4ticks each app; the isolated
   candidate constructors each reported no elapsed10ms ticks. Host542,combined242,
   native4K15,Files19 at1x/3x and foundation42 pass; bounded critic accepted.
   Preserve optional allocation-free diagnostics, bounded parser and failed
   infrastructure/preview evidence. Full ClearType/Unicode/fidelity is not established.
6. Preserve the accepted exact row-composition batch in cpu-composition.md:
   host547,combined243,native4K15,Files19 at1x/3x and foundation42 pass. The isolated
   first frame measured8â†’5compose ticks with identical retained sign-in pixels;
   this is not a latency distribution/FPS claim. Keep full24/32/40-byte binding
   admission evidence and the independent critic. Next evaluate exact sparse
   coverage for the96/192/288px sign-in avatar radii, with bounded data, differential
   pixel tests and actual native evidence before acceptance. The implemented sparse
   cache now passes host550/native4K15/resolution52/combined243 with independent
   bounded acceptance. Next address the ordinary CLI's TCG default
   using the already-tested WHPX/RAMFB profile, with an explicit software override.
   Shared launch is accepted: host555, default4K16 and software1x16 native checks;
   preserve headless-only qualification and current host-only build bridge.
   Continue actual shared text selection/undo and durable app backends next.

# Earlier roadmap notes

The rendering regression is now recorded above. QHD uses native200%,4K300%; retain normal
windowed startup and preserve prior failed/accepted evidence.

1. Preserve current178-check GUI evidence in13360, its independent18720 build,
   critic and all earlier failed evidence. Historical P11 qualification and the
   earlier273-check integrated core report remain separate records.
2. Finish reusable P12 command menus, keyboard focus/semantics, text selection,
   Unicode/shaping/IME, clipboard and gallery/editor gates. Continue real
   Files/Notes/Settings consumers before decorative applications.
3. Complete durable Explorer rename/copy/move/delete and target-specific context
   behavior with shared controls, then marquee/drop. Terminal/log viewer needs
   actual process/console integration; see the audit's app priority matrix.
4. Finish native validation of the implemented four-edge/alignment/compact
   taskbar and context controls. Preserve shared work-area/anchors, durable
   preferences and all-edge Snap/flyout/hit/DPI tests when extending them.
5. P13 stock Windows GUI, P14 full shell/services/fidelity and P15 network/audio
   remain separate obligations. Acquire a controlled build/DPI reference corpus
   before claiming current Windows1:1 equivalence.

Default remains windowed1280x720; explicit native4K renders at300%. Dynamic and
fractional DPI, GPU/vblank presentation and full accessibility remain unavailable.

Preserve the accepted Notes567-host/243-combined/13-check two-boot1x/3x evidence and independent review. Continue shared mouse/vertical text editing, dialogs and stronger checked compound-save backends, then durable Files mutations. Keep the35-family inventory and phase prerequisites.


Preserve accepted text-navigation.md evidence:579host, Notes26/two boots each1x/3x,
Files19each and default4K16 with independent source/pixel/raw-byte review. Continue
checked compound document replacement and save-conflict protection under P12;
design/check requirements are in checked-document-save.md. No complete Windows
editing or cumulative qualification claim.

Preserve the accepted checked-document-save.md checkpoint:595 host,42 expected foundation outcomes,243 combined checks, Notes26/Files19 at1x/3x and SDKtwo-boot/raw audits, with final independent critic. Continue shared modal controls and Notes Save/Don't save/Cancel for close/reopen. Preserve first-save empty-file and unknown-outcome limits; P12 partial.



