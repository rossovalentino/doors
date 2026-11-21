<!-- host-assets-20261121 -->
# Microsoft assets from the host, published repository — 21 November 2026

**Status: P18 in progress (guest-run, not gate-passed).**

* **Microsoft-owned assets left the repository.** Windows fonts, icons,
  cursors, sounds and wallpapers are copied from the Windows PC by
  `cargo host-assets` (tools/host-assets, then `cargo xtask host-assets
  acquire|export`) and listed in `assets/host-assets.json` (367 inputs,
  7,261 derived files). On this PC the import reproduces 7,492 files byte
  for byte; Segoe UI regular/semibold (updated by Windows) and the
  wallpapers (decoded by zune-jpeg instead of Pillow) were re-pinned.
  Builds refuse to run until the import completes.
* **Minimal repository:** generated assets (Doors' own icon, branding and
  raster exports, 594 files) and the Japanese IME dictionary (Mozc pinned to
  commit ffa0d785) are rebuilt by `cargo host-assets` too; a clean-room run
  reproduced all 8,224 files byte for byte. The pinyin dictionary stays
  committed (its CC-CEDICT source changes daily upstream).
* **Private captures stay local:** `reports/artifacts`, `reports/reference-26h2`,
  `docs/references` and `assets/references` are ignored.
* **NUL device** (kernel32/ntdll): Node's start-up opens `NUL` for closed
  standard streams; round 54 still reports the abort (under investigation).
* Host suite: 2,641 passed; the two stale tests (.ps1 associations, the P18
  scenario list) were corrected and pass.
* **P18 in the guest:** CRT-A, DOTNET-A (.NET 10.0.15 with its JIT),
  PWSH-A and PWSH-B (PowerShell 7.7.0) pass; ELECTRON-A runs Electron 45
  through Node's start-up into Chromium's browser set-up on the two-CPU
  desktop profile. 28 new system DLLs (catalog 62; x86 system slots 8 MiB),
  demand-paged VirtualAlloc commits and a growable page-table pool.
  dxgi/d3d11/d3d12/dcomp report DXGI_ERROR_UNSUPPORTED until P19.
* **Known gaps:** window hooks are registered but never called; GDI paths
  and metafiles are accepted but not drawn; world transforms are ignored;
  clipboard and OLE drag and drop work within one process; TaskDialog has
  no icons, progress bars or expando; SSPI has no packages; DirectWrite has
  no text layout or colour glyphs.

<!-- p18-start-20261114 -->
# P17 gate deferred, one repository, P18 under way — 14 November 2026

**Status: P15-P17 stay implemented and guest-tested but not gate-passed;
P18 work has started in this repository (host-tested, not yet guest-run).**

* **The P17 gate is deferred (owner, 14 November: stop trying to pass the
  gate, work on P18).** Attempt 19 ran out of disk during its guest
  scenarios (the P18 build cache had grown to 6 GB; every later scenario
  failed at start). Attempt 20 passed its host stage (521 commands) and all
  44 boot cases, then was stopped by the owner one scenario into the guest
  pool. No P15-P17 phase is marked passed; `project/phases.json` is
  unchanged.
* **One repository (owner, 14 November).** The frozen gate copy (`dv`), the
  P18 working copy and the nested `door-os` folder are gone: this folder is
  the repository. Old run evidence keeps its reports, logs and screenshots;
  the regenerable build output inside runs (initfs tars, images, raw disks,
  about 20 GB) was removed. `out/acquisitions` keeps the downloads (OVMF,
  Settings reference data, .NET 10.0.15 and PowerShell 7.7.0).
* **P18 so far** (details in `docs/design/p18-plan-20261111.md`,
  "Progress"): Windows memory over per-page reservations of a page pool
  (VirtualAlloc/Protect/Query, W^X, guard pages), images mapped into
  page-pool memory, futex waits and the SRW/critical-section/condition
  family, a complete x64 unwinder and SEH dispatcher, Get/SetThreadContext,
  128 threads, kernel32 runtime/locale/path/message functions, and a
  user-mode DLL loader in ntdll. Measured gap to start .NET: file mapping,
  the UCRT, thread-pool timers, an SMP scheduler.

<!-- p17-pregate-20261113 -->
# Settings parity, Mica, and the pre-gate guest batch — 13 November 2026

**Status: every guest scenario and acceptance suite of the P17 gate re-run on
the final image before the gate; the fixes below came out of that batch.**

* **Settings parity (owner order: settings first, then the gate).** Settings
  over Mica (translucent fills over the compositor's Mica, opaque dialogs and
  menus), the 26H2 page entrance, 18 wallpapers / 36 appearance words through
  every layer (service client, runtime, storage codec, window server,
  settings registry). `docs/design/settings-parity-26h2.md`.
* **P14-REGISTRY finished (owner decision, 13 November).** The typed settings
  registry was never wired into the guest, so `desktop-a-apps` could not
  pass. Now: the desktop appearance is the per-user HKCU dword
  `DesktopAppearance` (read through to the old settings record until first
  written; Theme moves its dark bit); Settings commits the mode and picture
  through the registry (`SETTINGS commit`); every Files, Notes and Terminal
  process observes changes live (`SETTINGS observed`, storage settings
  watch); a damaged value is reset to its default, rewritten and reported
  once with a notification (`SETTINGS reset ... reason=corrupt`).
  `desktop/workspace/src/settings_watch.rs`.
* **Kernel process table 13 -> 32 (owner decision, 13 November).** Kernel
  arenas re-laid out in the top PML4 slot, alias/allocation records 320 to
  640, loader page-table pool 1024 to 2048 pages, telemetry ABI v4, the
  kernel table's mapping records out of line (the 128 KiB boot stack, pinned
  by P06/P07, stays). Task Manager's snapshot lists the network and audio
  services. `multiwindow`'s six apps now run beside the session.
* **Bugs the batch found (host tests where possible):**
  - Settings and Files dropped an app launch (or document) when a close
    arrived behind the click; the close now waits for the request.
  - Twelve exported Settings icons had a channel above alpha; refused over
    Mica, so Settings failed at 300 %. Exporter clamps; fractional test.
  - Init rewrote every COM/installer registry value at each sign-in (since
    P16), so a read-only boot changed the disk; it now writes only changes.
  - At 4K a band commit's copy buffer could not be allocated beside
    composition; the window froze. The capture pool's spare cell serves it.
  - The compositor refused the splitter cursor the protocol offers
    (`CURSOR_SIZE_WE`), killing File Explorer on its navigation-pane edge.
  - Preview wrote a conversion in the Convert list's format whatever
    extension was typed (PNG bytes in a .qoi); a typed extension now picks
    its format, as Windows' save dialog does.
  - Terminal `cp` wrote a copy as one durable write per 192-byte chunk (a
    4 MiB copy ran past five minutes); a multi-chunk copy up to the large
    replacement cap is now one staged replacement fed by 64 KiB sections.
  - Init refused to start a session whose stored appearance or desktop
    controls were damaged; it now binds the defaults and the session's
    registry resets the value and tells the user (Windows' shell falls back
    to defaults over a bad per-user value).
  - A registry commit reported coupled keys in key order, so a mode change
    logged `console.theme` before the key the user changed; the committed
    key is reported first.
  - Window-server diagnostics trimmed to the 16 KiB process budget.
  - Failure-only diagnostics kept: `PRESENT_STALL`, `PRESENT_REFUSED`,
    `CLIENT_FAILED at=<line>`, `COMMIT_INVALID_PIXELS`.
* **Scenario updates** (behaviour changed since they were written): Settings
  search/rail waits for the settled page; UAC consent for regedit and
  gpedit.msc (P16); Narrator group announcement; fixture cursor shape 2;
  Preview's export name; the file-operation engine's record folder in the
  drag audit; the P14 large seed at C:\ProgramData (Doorsh is Windows-path
  only); `desktop-a-apps` reads the registry hive's values as well as the
  catalog's and takes the reset's posted notification, not Init's
  notification binding.
* **Last runs (frozen copy):** image `host-1794559638596879600-16868`
  desktop-a-apps `host-1794560426940429400-20416` (scenario fix) then
  passed; image `host-1794558216507225600-20228` desktop-a-files
  `host-1794559102918602800-21468` (4 MiB `cp` in 331 s); image
  `host-1794545886948661000-9428` desktop-wide, desktop-resolutions,
  settings-reference, p14-settings `host-1794555744410262600-22892`;
  earlier images the rest of the 64. The three image runs were deleted
  for disk space before the gate (their batch reports are kept).
* **Gate attempts 8-10 (13 November):** 8 stopped at a dead-code lint in
  the guest build of Settings (fixed), 9 was cut off with the session,
  10 failed in the inherited P10 `windows-base` boot: the Windows fixture
  stopped after its exception test and the guest ran out its 30 s. The
  same guest passes in about 10 s; a stress rig reproduced a stop once in
  about a hundred runs (there in Init's logoff call), without a capture.
  Both stops were at a disk write: the block driver polled with interrupts
  off, measured its limit with the frozen tick count and trapped into QEMU
  on every spin. Fixed (RISKS, "Intermittent guest stall"); attempt 11 was
  stopped for the fix. Attempt 12 passed every host command and inherited
  boot (both `windows-base` runs in about 10 s), then failed
  `desktop-recovery`: after the injected window-server crash, Init's first
  call to the restarted server (BIND_SESSION) timed out and Init treated it
  as fatal. Cause: a deadlock. The restarted server's sign-in gate looked
  up the credential service with a blocking call to Init while Init was
  inside its blocking bind call to the server; both waited out their 5 s
  deadline, again and again. The gate now resolves the service through
  Init without blocking (as the tray already did) and starts only once it
  is connected. Init also retries a bind that times out every half second
  (restarting the server after 10 s) and names the failing step
  (`DOOR_P11 BIND FAILED step=`). The bind also no longer waits behind a
  full accept queue on Init: the display service's background lookups
  connect without waiting.
* **Gate attempt 13 (14 November):** every host command, inherited boot
  and the first 12 of 65 guest scenarios passed; it stopped on the out/
  budget (18.7 GB of 20 GB: a full gate keeps one 240 MB boot-disk copy per
  desktop session for its own end-of-run checks). Owner, 14 November 2026:
  the budget does not matter as long as the disk is not filled; the out/
  limit is now 32 GB (`tools/xtask/src/cache.rs`; the 20 GB in earlier
  records stays as their historical value), and unreferenced debug runs
  were removed to keep about 44 GB free.
* **Gate attempt 14 (14 November):** the same 12 scenarios passed, then
  `p15-midi` failed: the recording held only half a second of the last
  of four tones. door.audio stopped the device as soon as the device had
  taken every queued buffer; QEMU (like a real controller's DMA ring and
  FIFO) takes buffers before it plays them, and the stop discarded the
  rest. door.audio now feeds 600 ms of silence after the last audible
  period before it stops the device. A second cause showed in the
  recording (about 10 ms gaps mid-tone, one run in three): the mixer
  padded a stream with silence whenever its client was a few tens of
  milliseconds late, and winmm's synthesizer rendered only 40 ms ahead.
  The mixer now waits for a whole period from every playing stream while
  the device holds at least 120 ms (it pads only when the device would
  run dry), streams may queue six sections instead of two, and the
  synthesizer renders 120 ms ahead, as Windows' software synthesizer does.
* **Gate attempt 15 (14 November):** 19 scenarios passed (all sound ones
  among them), then `desktop-notes` missed a close marker: a race in the
  scenario, which pressed Alt+F4 between Notes' prompt marker and Notes
  handing the first close back. It now presses again, at most three
  times. Attempt 16 runs on it.
<!-- p15-p17-close-20261111 -->
# P15, P16 and P17 ready for their gate — 11 November 2026 (late)

**Status: implemented, critic-reviewed and guest-tested; one cumulative
gate left.** The owner attested P12-P14 as passed (evidence deleted for
disk space before it was recorded; ADR 0027), so P15 is the selected phase
with P16 and P17 in parallel. On the owner's word, one `cargo xtask verify
--phase P17` closes all three: P15 and P16 cite its report, which must
contain every one of their commands (`spec::CUMULATIVE_CITATIONS`).

* **Critic.** One read-only critic per phase and a second read of the
  fixes; every finding fixed or recorded
  (`reports/critics/p15-p17-critic-findings-20261111.md`). The owner waived
  a further critic run; the gate's report gets a bound close-out check.
* **Guest regression (frozen copy, images 10-12):** all 29 P15-P17 and
  desktop scenarios pass on the current fixes, among them p15-connect with
  HTTPS against a TLS 1.3 test server (steps 16-19, identical to Windows
  11), p16-scm with services as LocalSystem (SID, elevation, profile, start
  arguments), p16-install with a 32-bit registrations package, p17-x86 with
  a 32-bit private DLL and data-file resources, and p17-wow64. Last runs:
  image 12 (`host-1794408832746412000-6124`) p16-install
  `host-1794411857712100600-11428`, p16-scm
  `host-1794409734462281300-18200`, p16-security, p16-com, p16-paths;
  image 11 (`host-1794405472233936400-19252`) p17-x86, p17-wow64,
  p16-tsched, win32-gui-f1, p15-desktop, p16-cmd, p15-connect; image 10
  the remaining scenarios.
* **Fixed by those runs:** registry key names now take Windows' characters
  (MIME types), C:\Windows\Fonts, 32-bit private DLLs in the kernel,
  launches without a working folder, IsWow64Process on an exited child, the
  elevation request bounded by one message, Init's Task Scheduler storage
  churn, the Doorsh fixture paths.
* **Records.** ADRs 0022-0027 amended or added; plan close-out segments in
  `docs/design/p15-plan-20261101.md`, `p16-plan-20261102.md`,
  `p17-plan-20261109.md`; requirements, RISKS and NEXT_STEPS updated; P18
  plan `docs/design/p18-plan-20261111.md`.
* **Next:** the P17 gate when the owner says, then P18.
<!-- p15-p17-20261111 -->
# P15, P16 and P17 — 11 November 2026

**Status: all three in progress, implemented, not verified.** Owner
decisions of 1-3 November 2026: implement P15-P17 now (M3 skipped by the
owner on 8 November), close each phase with a read-only critic and its
`cargo xtask verify --phase` once P12-P14 pass. The ledger records P16 and
P17 in progress under the same parallel authorization as P15.

* **P17 (M9) done.** 32-bit Windows programs run under WOW64 on real i686
  builds of all 21 compatibility DLLs (ADR 0023): PE32 loading, the 32-bit
  code selector and far-call system call gate, TEB32/PEB32, SEH through
  fs:[0], structures laid out per width (`door_windows_abi::layout`),
  32-bit-safe handle values (`door_windows_abi::tagged`), System32 →
  SysWOW64 with Sysnative, WOW6432Node, the (x86) folders and environment,
  IsWow64Process, Task Manager's "(32 bit)" names. Image
  `host-1794379951983230100-20464` (frozen copy): p17-x86 X86-A and X86-B
  pass (`host-1794380632958803400-9672`), p17-wow64 X86-C passes with the
  elevated 32-bit installer and its 64-bit helper
  (`host-1794380766843320600-11460`).
* **Gate.** `Phase::P17` inherits P16's gate and adds the i686 lints of the
  21 DLLs, the door-windows-abi and door-pe-loader suites and the p17-x86
  and p17-wow64 scenarios (`cargo xtask verify --list --phase P17`).
* **Records.** ADRs 0024 (tokens), 0025 (COM over Init's relay), 0026
  (Service Control Manager); requirements P15-* refreshed, P16-* and
  P17-X86 added; plans `docs/design/p15-plan-20261101.md`,
  `p16-plan-20261102.md`, `p17-plan-20261109.md`.
* **Open.** The compatibility settings page waits for an owner decision
  (AppCompatFlags\Layers value names are full paths; registry value names
  hold 48 units). p15-desktop's Preview step sent Ctrl+O before Preview's
  window existed; the scenario now waits for the window (to be rerun).
  Critic reviews per phase; formal gates after P12-P14.

<!-- p16-m8-20261108 -->
# P16 M8 Windows Installer and EXE installers — 8 November 2026

**INSTALL-A passes in the guest** (`cargo xtask test p16-install`, runs
`host-1794106846162546900-24500` and, with the fidelity fixes,
`host-1794108666047304600-24756`): msi.dll, msiexec.exe and door-msi
install, upgrade, repair, remove and roll back MSI packages written by
`door_msi::write`; DLL and EXE custom actions; the package's own dialogs
with User Account Control naming the .msi; an EXE installer with a
requireAdministrator manifest; Settings > Installed apps uninstalling both;
an installation cut off by a power loss rolled back by the next session.
New kernel call `WINDOWS_PRIVATE_LOAD` maps an application's own DLLs
(ADR 0022). Kernel handle values are multiples of four from 0x10000.
P16 records: `docs/design/p16-plan-20261102.md` (progress of 8 November).
P16 is still in progress, not verified.

<!-- p15-20261103 -->
# P15 network, audio and media — 3 November 2026

**P15 status: in progress, not verified.** Owner decisions of 1 and 2
November 2026 (ADR 0021): all of P15; implement P15-P17 now and close each
phase once P12-P14 pass. P15 is recorded in progress under a parallel
authorization with the selected P12.

* **Gate.** `Phase::P15` and `Phase::P16` (parse, cases, check_id, serial
  matching, demo title); fail-closed tests moved to P17. `verify --phase P15`
  adds clippy of door-network and door-audio (guest target) and of the six
  P15 DLLs (PE target), their host suites, and the desktop scenarios
  `p15-desktop`, `p15-connect` and `p15-sound` (exclusive); P16 adds
  `p16-cmd`. The xtask suite passes (262 tests).
* **Guest (3 November 2026).** CONNECT-A 42/42: Winsock, IP Helper and the
  DNS client, WinHTTP and WinINet run as MSVC programs in the guest against
  host endpoints. SOUND-A 15/15: winmm waveOut (callbacks, reset, position,
  loops), PlaySound, MessageBeep, timers, two devices mixed and an underrun,
  with every tone found in QEMU's wav capture. httpprobe, inetprobe and
  soundprobe (silent mode) give the same results on the owner's PC.
* **Records.** Requirements P15-NET, -WINSOCK, -NAMES, -HTTP, -AUDIO,
  -WINMM, -CAPTURE, -MEDIA-APIS, -VPN; ADR 0021; plan
  `docs/design/p15-plan-20261101.md`.
* **Remaining.** Capture (virtio-sound input, waveIn, Privacy > Microphone),
  DirectSound, WASAPI after COM, device loss, overlapped Winsock and IPv6;
  VPN work is paused pending the owner's direction.

<!-- p14-wsc-20261026 -->
# P14 complete desktop and visual fidelity — 26 October 2026 (WS-C)

**P14 status: in progress, not verified.** Owner decision of 26 October 2026
(ADR 0020): do all of P14; sign-in is a real password or PIN. P14 is recorded
in progress under a parallel authorization with the selected P12; it cannot
pass before P12 and P13. Guest runs of `sec-02-trusted`, `p14-settings`,
`settings-reference`, `explorer` and the inherited desktop suites pass on
images built on 31 October 2026 (`docs/design/p14-customization-plan.md`,
"Guest integration"); no cumulative P14 verify has passed yet.

* **Gate.** `Phase::P14` (parse, cases, check_id, serial matching, demo
  title, `phase_commands >= P14`); fail-closed tests moved to P15. `cargo xtask
  verify --phase P14` adds the `desktop` family: `desktop-a-shell`,
  `recover-01` (WS-A, registered), `desktop-a-files`, `desktop-a-apps` (WS-B,
  pending), `sec-02-trusted`, `vis-01` (WS-C, registered). P14 commands:
  guest clippy for door-desktop-shell, door-design-tokens, door-file-ops,
  door-settings-registry, door-settings-map; their host tests; named host
  suites `door-kernel --lib entropy`, `door-system-services --lib
  credentials`, `door-window --lib login`.
* **Records.** Requirements P14-SHELL, -REGISTRY, -NOTIFY, -FILEOPS, -SETTINGS,
  -TERMINAL, -LOGON, -RECOVERY, -REFERENCE, -TOKENS, -MARK, -VIS, DESKTOP-A/B;
  ADR 0020; contract `specs/contracts/desktop-shell-v1.md` (draft); plan
  `docs/design/p14-plan-20261026.md`.
* **Sign-in (host-tested).** Kernel entropy (RDSEED/RDRAND, health tests,
  TSC-jitter fallback, SHA-256 DRBG; real `ENTROPY_STATUS`, new
  `ENTROPY_READ`); Argon2id credential store in the session service (pinned
  argon2 0.5.3); window-server credential gate (first-run PIN/password
  creation, reveal while held, wrong-secret message, throttling) for sign-in
  and Win+L unlock; `sec-02-trusted` runner. The store is volatile until the
  storage service provisions `/state/credentials`.
* **Reference and fidelity (host-tested).** `profiles/windows-reference/
  reference-26h2-x64.json` (Windows 11 Pro 26H2, OS build 26300.9711),
  `door-design-tokens`, the door mask, the five-dimension harness with
  frozen thresholds and the `vis-01` runner (uncalibrated; motion fails
  closed without a reference sequence).

<!-- p13c-records-20261026 -->
# P13-C Windows GUI fixtures, reference and guest runners — 26 October 2026

**P12 status (recorded for the coordinator).** Every P12 acceptance suite and
desktop scenario has passed individually; the last cumulative `cargo xtask verify`
passed 435 checks with no failures before the session was interrupted, so no
qualifying P12 report exists yet. One final `cargo xtask verify --phase P13`
re-executes everything (all P12 commands, guest scenarios, desktop workflows and
suites, then P13). P12 is therefore still recorded as in progress, not verified.

**P13 status: in progress, not verified.** Workstream C landed:

* **Fixtures** (`tests/fixtures/windows-gui/`): F1 `selftest.c` gained a results
  file and `OFN_DONTADDTORECENT` with full-path dialog hooks; new F2 `editor.c`
  (Notepad-like: menus, accelerators, edit control, comctl32 status bar, Open/Save
  dialogs, unsaved prompt, custom chrome by `WM_NCHITTEST`, reentrant SendMessage
  orderings; code and `editor.rc` resource variants) and F3 `hang.c` (stops
  pumping on command; self-test of IsHungAppWindow/SendMessageTimeoutW, destroyed
  destination, rejected creation; `--reject` application).
* **Build**: `profiles/build/windows-x64.json` pins User32/Gdi32/ComDlg32/ComCtl32.Lib
  (SDK 10.0.19041.0), rc.exe/rcdll.dll and cvtres.exe; `tools/xtask/src/windows/gui.rs`
  builds each fixture twice (identical bytes) and publishes the import inventory
  `tests/fixtures/windows-gui/imports.json` (kernel32 24, user32 80, gdi32 21,
  comdlg32 3, comctl32 1 named imports).
* **Windows reference** (`cargo xtask build --profile windows-gui-reference`, run
  host-1792952263026238500-16188 on Windows 11 Pro 10.0.26300): all nine launches
  passed, every self-test repeated identically, binaries unchanged;
  `tests/fixtures/windows-gui/reference.json` holds results, F1 probe metrics and
  hashes. The run report itself is flagged only because other workstreams edited
  sources during it. RecentDocs unchanged; the common dialog still updated its
  ComDlg32 MRU lists despite `OFN_DONTADDTORECENT` (recorded, not restored).
* **Guest runners** (`tools/xtask/src/desktop/win32.rs`): `win32-gui-f1`,
  `win32-gui-f2`, `win32-gui-f3`, `scenario-141-5`, `scenario-141-6` registered in
  `scenarios::RUNNERS`; they stage image variant `desktop-win32` (fixtures and GUI
  DLLs in initfs, installed under `/system/bin`), use real QMP input, kernel
  module/process records, screendumps and raw-disk audits. Not run (QEMU deferred
  to the coordinator); they fail until P13-A/P13-B and the Terminal/Explorer
  `.exe` launch path land.
* **Gate**: `verify --phase P13` adds `cargo test -p door-windows-gui`,
  `cargo test -p door-pe-loader --test p13_application` and guest-pe clippy for
  door-user32/gdi32/comdlg32/comctl32 (`cargo xtask verify --list --phase P13`
  shows all ten suites registered).
* **Records**: contract `specs/contracts/windows-gui-v1.md`, ADR 0019.

<!-- p12-records-20261024 -->
# P12 gate and records — 24 October 2026

**P12 status: in progress, not verified.** P12 is the selected phase. P13 keeps its
owner-confirmed parallel authorization and schema but is not pursued now (owner:
"do P12 first"); plain `cargo xtask verify` selects P12 and P13 is never the active
phase. No P12 requirement has guest evidence and none has passed.

**Gate.** `cargo xtask verify --list` prints the composition without building or
running anything. `cargo xtask verify` for P12 runs, in order: doctor and
check-spec (P00–P11 accepted only as owner-confirmed unavailable history, never for
the selected phase); a preflight that fails before any build when a declared suite
has no runner; every inherited mandatory command plus the P12 commands; the 42
inherited diagnostic guest scenarios and demo; the ordinary session; the P11
desktop gate (workflows, recovery, wide lifecycle, resolutions, native default);
18 further desktop scenarios, each in its own child run bound by report digest
(`desktop-notes`, `desktop-files`, `desktop-remove` with their accelerated
variants, `desktop-shell`, `desktop-accelerated`, `desktop-default`,
`desktop-software`, `desktop-motion`, `explorer`, `multiwindow`,
`multiwindow-pressure`, `monitor`, `preview`, `terminal`,
`settings-reference`); and the acceptance suites `text-01`, `a11y-01`,
`display-01`, `motion-01` and `component-gallery`. The P12 commands are
guest-target clippy for door-ui, door-ui-native, door-accessibility,
door-keyboard-layouts, door-locale, door-gallery and door-ime (with door-app-ui,
door-preview and door-monitor); host tests for door-ui, door-ui-native,
door-accessibility, door-gallery, door-graphics-text, door-keyboard-layouts,
door-ime (plus its `corpus` test) and door-locale; and the door-window
`text_input`, `clipboard` and `dnd` tests. A coverage test fails if a
`desktop::*_test` scenario is in neither the P11 gate nor the P12 list.

**Required but missing.** Only `component-gallery` has a registered runner.
`text-01` (Notes TEXT-01 with IME and clipboard), `a11y-01` (being added),
`display-01` (display scale) and `motion-01` have none, so verify fails before any
build. No separate IME scenario exists; IME composition belongs to TEXT-01. Before
that, check-spec still stops at "dependency lock changed" (Cargo.lock differs from
`specs/sources/dependencies.json`, stream T). When this was written the gallery
binaries were not in the session staging table, so the gallery runner cannot pass
yet either.

**Requirements.** `project/requirements.json` `p12_status_20261024` maps the 15
plan gap rows to records. Implemented and host-tested, not guest-verified:
P12-TOOLKIT, P12-CONTROLS, P12-GALLERY, P12-TEXT, P12-EDITOR (Notes, 4 MiB),
P12-LAYOUTS, P12-INPUT-IME (dictionaries in initfs, resources 8/9), P12-DPI,
P12-THEMES (high contrast), P12-A11Y, P12-CLIPBOARD-DND and P12-ASSETS. Missing:
P12-TEXT-SIZE, P12-LOCALE and P12-TIMESTAMPS (the last two in progress). The
`p12_tasks` DAG in `project/phases.json` carries the same states.

**Validation.** Host only: `cargo test -p door-xtask --locked` (see the handoff
for the result). No QEMU run, build or verify was performed for this record.

# P12 and P13 gate and records — 23 October 2026 (superseded by 24 October)

P12 (native UI, text and accessibility) and P13 (Windows GUI integration) are both
in progress by owner decision 6. P12 stays the selected phase; P13 runs under an
owner-confirmed `parallel_authorization` and cannot pass before P12. Neither phase
has passed and no P12 or P13 requirement has passed. Decisions, audited gaps and
stream ownership: [the plan](../docs/design/p12-p13-plan-20261023.md).

**Historical evidence.** The owner deleted the P00–P11 qualifying reports for disk
space. Each of those phases now carries the reviewed `evidence_disposition`
`unavailable_owner_confirmed` ([ADR 0017](../docs/adr/0017-historical-evidence-disposition.md),
dated `AGENTS.md` amendment). `check-spec` verifies the original run ID and the
retained critic file (same phase, same original report, passed, zero findings, the
report digest wherever the critic recorded one). It never opens, recreates or
requalifies a report under an original ID. Any phase without the record stays
strict; the record is rejected for the selected phase and for P12 onward. The
strictly necessary re-run is the cumulative P12/P13 verify on current source.

**Gate.** `cargo xtask verify` certifies P12; `cargo xtask verify --phase P13`
certifies P13 and includes everything P12 includes: every inherited mandatory
command plus guest-target clippy for door-app-ui, door-preview and door-monitor, the
42 inherited diagnostic guest scenarios, the ordinary session, the P11 desktop
workflows and the P12 acceptance suites text-01, a11y-01, display-01, motion-01 and
component-gallery. P13 adds win32-gui-f1, win32-gui-f2, win32-gui-f3,
scenario-141-5 and scenario-141-6. No suite has a registered runner yet, so both
commands stop right after doctor and check-spec with "acceptance suite not
implemented", naming every missing suite; a current P12/P13 report without bound
passing checks for every suite also fails the phase gate. Neither command was run in
this checkpoint (the owner has an interactive QEMU session; the coordinator schedules
guest runs).

**Records.** `project/requirements.json` gains 11 P12 and 10 P13 requirement
records with spec line references; all are `verification_state: not_run`, 14 are
in progress and 7 not started. `project/phases.json` gains the `p12_tasks` (18) and
`p13_tasks` (13) DAGs with owners, prerequisites, interfaces, risks and acceptance.

**Open conditions outside the gate change.** The baseline required path
`desktop/workspace/src/main.rs` was rebound to the five W9 per-app entry points
(`files`, `notes`, `settings`, `terminal`, `search` `_main.rs`). `check-spec` still
stops at "dependency lock changed": `Cargo.lock` no longer matches the hash pinned
by the dependency review in `specs/sources/dependencies.json` (owned by stream T),
which must review and re-pin it. The full `door-xtask` crate does not compile at the moment because
`door-window` is mid-migration to the W8 `Scale` type; the gate modules were
tested in an isolated harness (below) and must be re-run in the crate.

**Validation.** An isolated host harness under ignored `target/gate-g-harness`
compiled the real `phase.rs`, `spec.rs`, `scenarios.rs`, `test_support.rs`,
`phase_commands`, the verify/acceptance region of `main.rs`, `demo::arguments` and
`layout::canonical_owner` against minimal stubs for unrelated modules: 32 tests
passed (including the disposition, parallel-phase, acceptance and repository-ledger
tests) and clippy reported no warnings beyond harness-only dead code. A read-only
probe on the real repository returned P12 for `verify`, P13 for `verify --phase P13`,
a refusal for P14, and the "acceptance suite not implemented" lists for both phases.
`cargo test -p door-xtask --locked` and `cargo build -p door-xtask --locked` remain
to be run once `door-window` compiles.

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

# Output budget and evidence availability

Out-only20GB maintenance has independent bounded acceptance in
reports/critics/p12-output-budget-critic.json. Final host642/33, fresh foundation42 plus
two interactive checks, complete desktop253/9 sessions and retained validation
before/after actual cleanup pass. At that historical checkpoint out was8,100,456,059 logical bytes.
Target and reports are excluded. Qualification and source bridges are recorded in
docs/build/output-storage.md; P12 remains partial. Storage recovery is now accepted in the bounded checkpoint above.

The owner cleared out after the completed Files deletion review. Historical
acceptance/report bytes remain unchanged, but their deleted out dependencies are
unavailable for retained revalidation. Pinned firmware was hash-verified and
restored; fresh qualification uses new IDs. See reports/artifacts/p12-output-availability.json.
Automatic out-only20GB maintenance is accepted within its bounded policy.
Only unreferenced completed Cargo intermediates may be removed; owner storage and
execution evidence remain protected. See docs/build/output-storage.md for complete
fresh build/guest validation and review bindings. Preserve the accepted Storage recovery checkpoint above.

# Historical permanent Files deletion work

Checked permanent removal and the default-Cancel Shift+Delete modal have bounded
independent acceptance. Host631, fresh foundation42, complete desktop253/9 actual
sessions, SDK three boots, deletion GUI15/two boots each1x/3x, Notes39 each and
rename19 each pass, with immutable retained validation and matching guest rebuilds.
The review repaired deferred-close poison/counter and held-keyboard/mouse retry
paths. See docs/design/files-deletion.md and reports/critics/p12-files-deletion-critic.json.
Next repair Storage transport reconnection without replaying uncertain mutations.
P12 remains partial: ASCII32 confirmations, no recursive deletion/trash/restore,
no native GUI I/O injection or full Windows/ClearType/FPS claim. Original missing
P00 evidence still blocks cumulative qualification. Owner disk remains absent.

# Current shared document dialog checkpoint

Notes now has real Save/Don't save/Cancel Close and Reload choices, with shared
focus/capture/keyboard handling and retained RAM on rejection. Repeated Close also
repairs canceled attachment/configure state. Host613, complete desktop253/9 actual
sessions, retained-evidence revalidation, Notes39/two boots each1x/3x and Files19
each pass. Independent bounded review accepted reports/critics/p12-document-dialogs-critic.json.
See docs/design/document-dialogs.md for bindings, repaired failures and limits.
P12 stays partial; next implement checked permanent Files deletion, then remaining
toolkit/storage recovery/app providers. Windows1:1, ClearType/FPS parity and all
Windows apps are not established. Original missing P00 evidence still blocks
cumulative qualification.

# Current checked document save checkpoint

Private bounded candidates now commit4096-byte old/new streams with authenticated
object/version checks and one durable replacement. Notes preserves its RAM and
separate save attention on conflicts or uncertain outcomes. An acknowledged save
advances the durable byte checkpoint immediately, including when close later fails.
The independent critic accepted this finite batch:595 host tests,42 expected
foundation scenarios,243 combined checks, Notes26 and Files19 checks at both1x/3x,
plus actual SDK two-boot/raw-byte audits. See docs/design/checked-document-save.md.
P12 remains partial. Shared choices are accepted in the newer checkpoint above.
Original missing P00 evidence still blocks cumulative qualification.
# Current bounded Notes editing checkpoint

Shared selection/undo/redo and dirty-reopen protection now use a bounded document model. Native light/dark1x/3x workflows pass two boots each and preserve real file bytes/identity. Host567 and combined243 checks pass; independent review accepted the finite scope. Unicode/IME, clipboard, mouse/vertical navigation, atomic replacement and full Notepad fidelity remain open. See docs/design/notes-editing.md. Original missing P00 evidence still blocks cumulative qualification.

# Current accepted bounded RPC latency repair

The native private-disk baseline measured 128 ticks for 128 metadata calls
(1,280ms total), confirming the client's fixed timer wait. A candidate now gives
the server up to eight cooperative turns before the existing sleep backoff.
Actual-loop host cases pass, including stalled deadlines and scheduler errors.
The candidate measured 32 ticks (320ms) on the same 128 calls, a 75% reduction
in this fixture, repeated by the final validator. Current host537, foundation42,
combined desktop242, 4K15 and both Files19-check/two-boot regressions pass with
unchanged inputs. Independent critic reports/critics/p12-rpc-latency-critic.json accepts
the finite change with no unresolved actionable finding. See
docs/design/rpc-responsiveness.md. The accepted rename/rendering records below
remain separate; P12 remains partial.

# Current checked Files rename

F2, command-bar and modern/classic context Rename use an observed object/version
and current ancestry/ACL/sharing checks followed by durable Storage commit.
Editing preserves extensions; Escape cancels and errors retain the edit.
The final host suite passes 531 tests; strict host/native Clippy passes. Both
1x TCG and 4K/3x WHPX GUI runs pass 19 checks and two boots, preserving the original
identity and payload. The SDK exercise passes exact raw-disk/identity checks and
the full foundation rerun matches 42 scenarios. The combined desktop passes
242 checks; current adversarial recovery passes. Product build 13324 is byte-
identical to the GUI-tested build 16368. See docs/design/files-mutations.md for
bindings and the corrected SDK fixture integration. Independent critic
reports/critics/p12-files-rename-critic.json found no unresolved actionable finding in
this finite scope. P12 remains partial; copy/move/trash/restore and other apps
remain open.

# Current rendering verification

P12 remains partial. The combined native desktop regression passed242 checks
with unchanged inputs in reports/runs/host-1792521201711588300-25240/report.json
(SHA2565450fc6ef3fdb4fec82fd1df004f9be75a117e2015c0a7c6dc7f33c0a1a61172).
Current host519 tests passed in15540; focused four-resolution52 checks passed
in23560. The explicit4K WHPX/RAMFB profile passed14 checks in24108; native
two/four-CPU and degraded-mode regression passed15 cases in26016. Full report
bindings and limits are in docs/design/rendering-responsiveness.md. Independent
critic reports/critics/p12-rendering-responsiveness-critic.json found no unresolved
actionable issue in this bounded scope; no P12 closure or60FPS/1:1 claim.

The installed Windows11 reference and35-family inventory include Settings,
Explorer, Calculator, Notepad and Paint. Desktop control stopped on the owner's
Escape; further work uses recorded references and isolated guests. The next
consumer work extends real Files mutations and shared P12 text/controls.

The cumulative verifier failed on the missing historical P00 report after
external output cleanup. Recorded historical identities are preserved; focused
new evidence is not a replacement for that missing cumulative chain.

# Earlier development status (historical)

## Native density and transfer repair before final verification

P12 development repair implemented; native regression and independent critic
binding are pending. Explorer's invalid 80px alpha export is repaired; authored
ICO frames, native4K300%, RGB LCD text with transparent fallback, 16KiB display
captures, BGRX scanline transfers, app/rounded fill spans, bounded QOI preview
caches and deferred capture-cell initialization address observed defects/costs.
Host218 tests and native strict clippy passed before final harness/doc edits.
See docs/design/rendering-responsiveness.md. No FPS or Windows1:1 claim.
The owner now requests installed Windows11 as the primary reference and complete
functional app inventory; proceed within phase order after this repair is tested.

# Current taskbar and context controls batch

Implemented against Microsoft's May/August2026 designs and October15 rollout
notice: four taskbar edges, start/center alignment, compact size, inward Start/tray
anchors and durable per-user preferences. Settings adds Taskbar and Context menu
detail pages (17 surfaces total); Files uses inline command/Properties ordering
and real read-only Properties. Unsupported mutations/providers remain disabled.

Work-area changes preflight geometry/queues, reflow maximized/snapped windows,
keep normal/restore titles reachable and reposition Task View. Bar-only blur/cache
regions reduce transfer work. Actual native combined regression and final critic
binding are pending; no complete1:1, frame-rate or new phase-closure claim.
See docs/design/desktop-controls.md for references, contract and remaining controls.

# Current responsiveness and Explorer addressing batch

Implemented and host-checked; native combined regression and critic binding are
not yet final. Exact validated snapshot differences, idle repeated activation,
regional composition, contiguous scanout spans, input during large flyouts and
cursor-before-compose reduce avoidable CPU work. QHD and larger outputs render
at native200%; default startup remains windowed1280x720. Explorer now has actual
C: aliases, component-by-component authenticated resolution, breadcrumbs and
caret/selection editing. No owner-data migration or complete Windows API claim.

Previous accepted development evidence is retained below unchanged. P12 remains
partial; historical P11 acceptance is unchanged.

# Current category and navigation refinement

The final native GUI suite passed178 checks with unchanged inputs:
reports/runs/host-1792460182885722800-13360/report.json
(SHA25613913fe88db67bc1e849fa8f15f8721156349e2c52882d1fb7ee4d9368ec5fd6).
169 affected host tests, strict native/host clippy and offline asset checks passed.
The independent18720 build reproduces all five product pairs. Only the host
scene fixture changed afterward: its bare-desktop click now stays above the bar.

Implemented:15 Settings surfaces with category-specific section/row inventories,
original Fluent row symbols and capability-aware actions; filled local avatars;
Explorer history positions, collapsed navigation/groups; Win+A/N; trusted
Start/tray focus restoration and actual outside right-click delivery.13 native
focus gestures verify26 edits/flushes with independent disk-byte audits. Narrow
Display retains matching native scale/resolution/orientation glyph coverage.

Critic binding: reports/critics/p12-category-refinement-critic.json (accepted finite development scope; no unresolved actionable findings).
Critic SHA256 eba39b5b95cc9507622845dbf745c74d1cde04fbea2c71d093b61a2dba98b7d3.
All earlier failures remain retained, including16640 (incorrect fully-covered
small-glyph test),5808 (animated-caret stillness),20760 (cursor-covered sample),
and2508 (old desktop-click coordinate on the clock). The repaired178-check run
supersedes these attempts. The owner disk was excluded from every regression.
No new phase closure, latest-build1:1 fidelity, GPU/vblank or zero-defect claim.

# Previous desktop development checkpoint

P12 remains active and partial; P11 qualification below is historical.
The owner-requested shell refinement now includes:

- Windowed 1280x720 startup, no forced fullscreen; explicit display modes remain.
- Native 200% rasterization at 3840x2160 for app geometry, Segoe outlines,
  original Windows ICO frames, rounded previews and actual 64px Windows cursors.
- Original cyan open-door SVG adapted from the supplied idea, transparent native
  16/20/24/28/32/40/48/56px BGRA, portable 256px PNG and seven-size PNG-backed ICO.
- Passwordless trusted sign-in before user-session admission; no app autostart.
- Searchable Start grid/list, actual bundled-app singletons, UTC RTC clock,
  navigable Gregorian calendar, absent-device quick settings and Show desktop.
- Caption hover/press visuals and release/cancel gestures; dark/light app palettes.
- Four fixed large capture cells and recycled wallpaper scratch, reused shrink
  surfaces, input between capture/scanout batches and partial caption/bar transfer.
  Closed flyouts transfer 48 logical rows; dismissal retains the previous extent.

The integrated regression passed 273 checks with unchanged inputs:
reports/runs/host-1792436289607278800-7072/report.json. After the final ownership,
capture, literal theme-preview and gallery-spacing repairs, the complete native
GUI suite passed 160 checks
with unchanged inputs: reports/runs/host-1792442174995919700-18536/report.json.
Focused native 4K passed 16 checks in host-1792440070123858000-19104.
The 148 core/tooling and 16 application host tests passed, as did strict native/host
clippy checks. The integrated 273-check run predates the final owner guard,
screenshot and theme repairs;
the fresh GUI run covers those changes. No new phase qualification is claimed.

New development behavior: Doors frontend names, actual caption glyphs, reference
Search field/Task View/bell, shared bar paint/hit geometry, session workspaces,
revision-cached previews, held MRU Alt+Tab and keyboard/edge Snap.
Earlier failed runs and screenshot-timing failures remain retained. Combined4899
exposed a damaged UTF8 test literal; repaired with explicit Rust Unicode escape.
Critic dual-Alt release finding is fixed and has a physical two-Alt guest regression.
Foreign close denial now precedes gesture mutation; actual held drag/caption
regressions pass. A critic-found partial4K Alt+Tab capture is retained in7644;
the current test waits for successful CPU transfer completion and checks actual
selection/preview pixels. PNGs encode the exact asserted PPM, with a roundtrip test.
See docs/audits/p11-desktop-progress.md for requirement/Settings gaps, references,
customization research and ordered next work. No phase closure is implied.

Development critic: reports/critics/p12-desktop-refinement-critic.json accepted the finite
scope after hash/pixel review. Generic sparse captures and CPU timing retain the
limits described in RISKS; no phase closure. Earlier3092 predates the final visual fixes.

Remaining: taskbar edges/alignment, target-specific shell contexts, marquee/drop,
complete Settings service/page coverage, P12 text/accessibility and dynamicDPI.
No1:1, GPU/vblank or zero-defect claim. Host SDL resizing can letterbox fixed GOP.

# Door OS status

P11 native window/composition foundations passed their declared software-display gate.
Qualifying report: reports/runs/host-1792303566097472600-17116/report.json.
Critic: reports/critics/p11-critic.json;394 reviewed inputs,375 evidence files,zero unresolved findings.
P10's owner-requested pre-GUI usability repairs are qualified and closed. P00ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬ÃƒÂ¢Ã¢â€šÂ¬Ã…â€œP09
retain their historical qualification. Door currently provides
an experimental persistent text environment on the declared QEMU profile.

Implemented: ordinary service/shell boot without a diagnostic token, reusable
owner disk, paths/directories/files and visible themes, disk-supplied unprivileged
ELF/PE processes with arguments/environment/cwd, orderly session revocation and
power-off, larger ext2 files and production catalog, and the pinned Tiny Basic
source port. See specs/contracts/ordinary-session-v1.md and docs/build/text-session.md.
The full cumulative gate and independent critic accepted this integrated scope.

Qualifying report: reports/runs/host-1792235592894575300-22840/report.json
SHA256 64261b5ced696019b1fa79c6502bbc7d9cbaba46c190d32b81c7fd2c38cf6361.
Critic: reports/critics/p10-readiness-critic.json
SHA256 550938cd97118a409c0e9b34159eb01c3e307234ac60db6a824b811b9c03777f.
The run succeeded with unchanged inputs and no required command failures:
363 host tests, strict target checks, two matching 16-artifact core builds,
three matching application rebuild pairs, 42 diagnostic guest scenarios, both
interactive regressions and same-binary Windows reference comparisons.

The ordinary suite adds 87 shell checks, 18 Tiny Basic checks, separate-boot
reopen, actual guest reboot and independent disk audits. It verifies real file
operations, copied executables, startup data, binary-preserving redirection,
36 extra directories, input editing, cancellation and a 229,893-byte BASIC save.
Saved/reopened disk hashes match. Shutdowns report zero process-frame leaks and
eight DMA pages reclaimed; reopened sessions perform no writes/flushes.
The critic reviewed 321 unchanged inputs and independently checked 373 additional
evidence hashes. See docs/testing/application-trials.md for exact application
hashes, measured workflows and limits. Finite review does not prove every bug absent.

The visible owner session uses the final image and out/session/user.raw. Existing
user files were preserved and read back. Its locator is out/native-session.json;
use cargo xtask session to reopen it after shutdown. Automatic approval review
blocked cache deletion. NTFS compression recovered about 64.4 GiB from 522 retained
inactive raw images without deleting their evidence; build-cache compression also
reduces output storage. The owner disk was excluded from this cleanup.

The later requested deletion audit found 54,365 candidate files (60.3 GiB logical)
outside retained output references or in reproducible compiler caches. Automatic
approval review rejected deletion as blocked by policy; no files were deleted.
The reference-closure plan and blocked result are out/cleanup-plan.json and
out/deletion-summary.json. P00ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬ÃƒÂ¢Ã¢â€šÂ¬Ã…â€œP10 consistency passed before the attempt.

P11 reference research is recorded in docs/design/windows11-reference.md: the
official Windows UI Kit, pinned Gallery/WinUI sources and detailed visual guidance.
The chosen Windows 11 desktop capture profile remains unmeasured. This research
does not establish 1:1 GUI fidelity. P11 now has a protected display/window server,
independent persistent Files/Notes consumers, checked section snapshots, queued
input, captured composition, standard caption gestures and restart recovery code.
The historical P11 apps used ten Fluent icons and Selawik. The current P12 batch
uses twelve Fluent icons, three pinned Windows ICOs and six Windows CURs.
The focused development suite passed move/resize/state/close, exact note bytes
across two boots, malicious-client rejection, real server restart with unsaved text,
hung-client termination and zero process-frame leaks. A historical focused pass is
reports/runs/host-1792303350826952300-25676/report.json. Files now uses correct
new-folder/refresh/history shortcuts and transactional bounded navigation. Ctrl+W
closes both apps; text/resize pointer artwork follows authoritative hit
regions and clipped hotspots. The suite independently samples pointer pixels and
audits three created directories. A cumulative attempt exposed a queued-close vs
repaint race; clients now drain close instead of reopening the window. The final unchanged cumulative gate and critic acceptance passed (408 host tests,42 diagnostic guest scenarios,87 ordinary shell checks and three GUI boots). See
docs/testing/native-desktop.md and docs/design/application-interactions.md.

Historical bounded P00ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬ÃƒÂ¢Ã¢â€šÂ¬Ã…â€œP10 checkpoint:
reports/runs/host-1792214999674635300-12500/report.json
SHA256 e8d8ce9cc07e83c3d5bde11385d5f8be3bc970ef35e52741cfe42b127d1eaa3d.
Historical critic reports/critics/p10-critic.json, SHA256
9f49c413d6284d8966f8e7dc465d6331c6a691f0b18405d16bc24774e273b100.
That run passed 341 host tests, 42 actual diagnostic QEMU scenarios, both
interactive fixture checks, matching 16-artifact builds and same-binary Windows
reference comparisons. Those claims apply to that bounded engineering subset.

The original development authentication uses fixed capability-provisioned UIDs;
password/entropy providers and full native Rust std remain unsupported. Ordinary
service workloads use one vCPU. Windows support retains bounded original system
providers, four concurrent Windows threads, 1 MiB dynamic commitment and scoped
CRT/unwind/I/O semantics. Arbitrary DLL search, general IOCP/C++/GUI and broad
stock Windows applications remain unqualified. Physical hardware is unqualified.
# Current typography evaluation

Hinted Segoe UI Variable caches now serve native1x/2x/3x text. Static native4K
constructors measured5ticks compositor/4ticks each app; the isolated candidate
reported no elapsed10ms ticks for each constructor. This is not zero execution
time or whole-app/sign-in/FPS evidence. Coverage capacity at3x fell from4,423,680
to1,673,001bytes per atlas; other process memory is outside this measurement.
Current host542, combined242, native4K15, Files19 at1x/3x and foundation42 pass
with unchanged inputs. Independent review accepted this bounded cache selection. See
docs/design/typography-cache.md. Full Unicode and Windows fidelity remain open.


Current CPU composition change accepted: exact clipped row blending and unused
signed-out taskbar blur avoidance pass547host tests,243desktop checks,15native4K,
Files19checks/two boots at1x/3x and42foundation outcomes. Each supported session
binding produces its own full frame with fresh taskbar material. The isolated4K
first frame measured5compose/1transfer ticks versus8/1before; the retained sign-in
PNGs are pixel-identical. This is one coarse observation, not FPS or whole-sign-in
latency. See docs/design/cpu-composition.md and its bounded independent critic.
Next: sparse exact avatar coverage, then real shared P12 app controls/backends.
The sparse avatar candidate now passes550host tests,243desktop checks,52resolution
checks and15isolated4K checks. Retained sign-in pixels match at all four modes;
the isolated first frame measured3/1compose/transfer ticks versus5/1before.
Native image spans remain below16MiB. Independent bounded review accepted;
see docs/design/avatar-coverage.md.
Interactive launch now defaults to the already-tested WHPX/RAMFB
profile, with explicit TCG override and shared option/actual-argv validation
before spawning. Host555 and both16-check native paths pass; independent bounded
review accepted. These native runs are headless. Historical
regression helpers stay TCG. See docs/design/desktop-launch.md.


Notes batch accepted with567 host tests,243 combined checks and13 checks/two boots at1x/3x. Independent review and raw-byte audits pass; see docs/design/notes-editing.md. P12 remains partial.


Text navigation bounded acceptance: host579, Notes26 checks/two boots each1x/3x,
Files19 each1x/3x and default4K16 pass with unchanged inputs. Independent pixel,
raw-byte/identity/CRC and source/artifact review accepted; see text-navigation.md
and reports/critics/p12-text-navigation-critic.json. Earlier Notes evidence remains
separate; visible-row dragging and ASCII limits do not establish Notepad parity.
Next P12 work addresses checked compound document replacement and save conflicts.

Historical compound-save development note is superseded by the accepted save and
dialog checkpoints above; candidate isolation and independent known/uncertain
attention guards remain required. See checked-document-save.md.



