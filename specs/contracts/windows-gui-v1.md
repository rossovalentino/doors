# Windows GUI fixtures, reference and guest qualification (v1)

Status: fixtures, toolchain pins, reproducible build, import inventory and the
Windows reference run implemented (26 October 2026, workstream P13-C). Guest
runners are registered and wired into `cargo xtask verify --phase P13`; they fail
until the P13-A loader/personality and P13-B GUI DLLs land. Decision record:
ADR 0019. Requirements: WIN-FIXTURES, WIN-REFERENCE, WIN-LAUNCH, WIN-USER32,
WIN-GDI32, WIN-COMDLG32, WIN-COMCTL32; master spec GUI-E (1366), WIN-GUI (4220),
GUI-01/02/03 (4271-4273), scenarios 141.5 (5285) and 141.6 (5289), section 86
`test compat --suite win32-gui` (2493).

## Fixtures

Ordinary Windows SDK C, no CRT, `/SUBSYSTEM:WINDOWS,6.02`, entry `entry`, no
Door headers or conditional paths. Sources in `tests/fixtures/windows-gui/`.

| Id | Executable | Source | Modes (one exact argument) and exit |
|---|---|---|---|
| F1 | `wingui-selftest.exe` | `selftest.c` | none: self-test, 75; `--reopen`: 76; `--probe`: measured values, raster file, 77 |
| F2 | `wingui-editor.exe` | `editor.c` | none: interactive editor, 0; `--selftest`: 80; `--reopen`: 81 |
| F2-RC | `wingui-editor-rc.exe` | `editor.c` + `editor.rc`, `/DEDITOR_RESOURCES` | as F2; menu, accelerators, strings, About dialog and version from resources |
| F3 | `wingui-hang.exe` | `hang.c` | none: interactive, stops pumping on `h`; `--selftest`: 82; `--reject`: 83 |

Every fixture writes deterministic CRLF-terminated result lines to standard
output (when it has one) and to a results file in its working directory, flushed
per line: `wingui-selftest[-reopen|-probe].results` (created), and
`wingui-editor.results`, `wingui-editor-rc.results`, `wingui-hang.results`
(appended, so a relaunch accumulates). A failed check writes one `*_FAIL code=N
error=E` line and exits N. Paths never appear in results (file names only), and
timings are reported only as thresholds.

* F1 covers creation/show/paint message traces, raster ROPs and a CRC, posted
  and sent ordering, edit/button/static input, menus and accelerators, a timer,
  MessageBox, nested and cross-thread SendMessage, destroyed destinations,
  rejected creation, IsHungAppWindow/SendMessageTimeoutW and Save/Open dialogs.
* F2 is a Notepad-like editor: menu bar (File, Edit, View, Help), accelerators
  (Ctrl+N/O/S, Ctrl+Shift+S, Ctrl+A, Ctrl+K), a multiline edit control, a
  comctl32 status bar (`msctls_statusbar32`, parts, text, `NM_CLICK`), Open/Save
  common dialogs, saving UTF-8 without BOM (CRLF from the edit control), an
  unsaved-changes prompt, and an owned custom-chrome popup (`WS_POPUP |
  WS_THICKFRAME`, `WM_NCCALCSIZE` = whole window, `WM_NCHITTEST` caption,
  close box, bottom-right grip, `WM_GETMINMAXINFO` 200x120, Esc closes). The
  self-test adds three reentrant SendMessage orderings (`EDITOR_ORDER`): outer
  sent message with a synchronous child notification (1,2,3); a subclassed
  control sending to its parent which re-enters the control (11,40,31,50,21);
  cross-thread send serviced while the caller waits (60,65,70), plus a
  destination destroying itself inside a sent message.
* F3's self-test measures a visible top-level window on a second thread that
  stops pumping on command, before and after the five-second hung threshold,
  release, a self-destroying destination and a rejected `WM_CREATE`. `--reject`
  is an application whose main window rejects `WM_NCCREATE`.

Dialogs pass `OFN_DONTADDTORECENT`; the self-test hooks type the full target
path into the file-name box (`CDM_SETCONTROLTEXT`), so the result does not
depend on the initial folder the system chooses.

## Toolchain and build

`profiles/build/windows-x64.json` pins (path, version, SHA-256) MSVC 14.44.35207
`cl.exe`/`link.exe`/`c1.dll`/`c2.dll`/`mspdb140.dll`/`cvtres.exe`, SDK
10.0.19041.0 `kernel32.lib`, `User32.Lib`, `Gdi32.Lib`, `ComDlg32.Lib`,
`ComCtl32.Lib`, and `rc.exe`/`rcdll.dll` 10.0.19041.5609
(`gui_import_libraries`, `resource_compiler`). `tools/xtask/src/windows/gui.rs`
compiles with `/O1 /GS- /Zl /W4 /WX /X /Brepro`, links `/NODEFAULTLIB /Brepro
/DYNAMICBASE /NXCOMPAT /BASE:0x140000000 /STACK:1048576,65536`, builds every
fixture twice and requires identical executables, and accepts only named imports
from kernel32, user32, gdi32, comdlg32 and comctl32 (both kernel32 and user32
required).

## Import inventory

`tests/fixtures/windows-gui/imports.json` (`windows_gui_import_inventory`):
per fixture its SHA-256, subsystem and sorted named imports by module, plus the
union and counts (26 Oct 2026: kernel32 24, user32 80, gdi32 21, comdlg32 3,
comctl32 1). A Door compatibility DLL must export every symbol of its module;
a missing export fails the load (no success stubs). The guest suites refuse a
stale inventory.

## Windows reference

`cargo xtask build --profile windows-gui-reference` builds twice, publishes the
inventory, then runs on the owner's PC (owner decision 4): fresh working
directories under `out/windows-gui-reference/<run>/`, ordinary drive-letter
working directory and executable path (the shell mishandles verbatim `\\?\`
paths), a 60 s watcher (IsHungAppWindow needs more than 5 s), F1 default,
`--reopen` and `--probe`, F2 and F2-RC `--selftest`/`--reopen`, F3
`--selftest`/`--reject`. Every self-test runs twice and must repeat; each results
file must equal the concatenated standard output of its runs; binaries must be
unchanged. `tests/fixtures/windows-gui/reference.json`
(`windows_gui_reference`) records the OS identity, fixture hashes, every results
text, saved-document hashes, F1 probe metrics (GDI measurements, traces, raster
CRC and file hash) and a registry snapshot.

Read-only policy: every input is a message to the fixture's own windows; no
input injection into other applications and no setting changes. Measured
side effect: `HKCU\...\Explorer\RecentDocs` is unchanged, but the Explorer-style
common dialog updates its own `ComDlg32` MRU lists (FirstFolder,
LastVisitedPidlMRU, OpenSavePidlMRU, CIDSizeMRU) even with
`OFN_DONTADDTORECENT`. The runner therefore exports
`HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\ComDlg32` with
`reg export` before the first launch (kept as `comdlg32-before.reg` in the
working directory and the run) and afterwards, also when the run fails, undoes
only the run's changes: it deletes the values absent from the snapshot (and any
subkey the run created) and re-imports changed or missing values, such as
`MRUListEx`, byte-for-byte from the snapshot's own export data. The key itself
is never deleted. The record lists the deleted and restored values, both export
hashes and a `reg query` snapshot, and the run fails unless the restored values
and subkeys equal the snapshot and RecentDocs is unchanged (ADR 0019 section 4).
The published reference predates this restore: its run added 16 MRU entries,
which the owner approved removing on 2026-10-26 (backup
`comdlg32-mru-backup-20261026.reg` beside the repository;
`registry.comdlg32_restore` in `reference.json`). Rerun the reference only when
fixture bytes change.

## Guest staging

Each suite rebuilds the fixtures twice and requires the reference hashes, builds
the GUI DLLs (`dlls::build_gui`: ntdll, kernel32, gdi32, user32, comctl32,
comdlg32 with `guest-pe`, fixed bases user32 0x188000000, gdi32 0x18a000000,
comdlg32 0x18c000000, comctl32 0x18e000000; each may import only its declared
lower modules), and stages image variant `desktop-win32` from the desktop
build's initfs (`initfs::with_files`): `seed/wingui-*.exe` and
`system/install.manifest` extended with `/system/bin/wingui-*.exe` lines.
Every P09+ image build compiles all six DLLs (`dlls::NAMES`, order ntdll,
kernel32, gdi32, user32, comctl32, comdlg32; exact import graph gdi32 → ntdll,
user32 → gdi32+ntdll, comctl32 → user32+gdi32+ntdll, comdlg32 → user32+ntdll)
and the desktop initfs ships `windows/{gdi32,user32,comctl32,comdlg32}.dll`;
the suite requires those bytes to equal its own rebuild (an older image without
them gets them staged). Firmware and kernel are the desktop build's.

## Launch contracts the suites rely on

* Terminal: any name that is not a native built-in runs a program
  (`NAME.exe [ARG]` is `/system/bin/NAME.exe`; a name with `/` is a path)
  through Init EXECUTE with the tab's working directory (as the text shell).
  The program joins the session console service: its writes reach the kernel
  console device (`DOOR_CONSOLE` records on the desktop) and the console
  service mirrors them to the Terminal (`CONSOLE_MIRROR`, Workspace peers
  only), which shows them in the tab. Ctrl+C cancels the program (Init
  `CANCEL_CHILDREN`). The command completes with `DOOR_P12 TERMINAL COMMAND
  name=NAME.exe ok=1 bytes=B exit=E` after the process retires.
* Explorer: the address bar (Ctrl+L) accepts the executable's Windows path
  (`C:\Users\1001\<folder>\<name>.exe`), and Enter or double-click on an
  `.exe` item opens it; either starts it through Init `EXECUTE_DETACHED` as the
  signed-in user, with the containing folder as working directory. Init adds
  `DOOR_CONSOLE=none` to the environment, so a GUI image runs with NULL
  standard handles (`DOOR_P12 FILES LAUNCH path=...`).
* Task Manager lists a manifest-15 process as "Windows application"
  (`windows-gui-app`, Apps group, Windows personality isolation).
* Process identity: the kernel's `DOOR_P09 MODULE pid=P index=0 ... sha256=S`
  record (S = fixture bytes, later indexes = mapped DLLs, which must include the
  staged GUI DLL bytes of every imported GUI module) and `DOOR_P08 PROCESS ...
  pid=P ... uid=1001 ... exit=E`.
* Close and not responding (Windows 11; native-window-v1 "Win32 close and not
  responding"): closing a window that still retrieves messages delivers
  WM_CLOSE, which user32 acknowledges at once, so a slow WM_CLOSE or an
  unsaved-changes MessageBox is never timed out. A window whose thread has
  not retrieved messages for five seconds while messages wait becomes a ghost
  (frozen frame, whitened, " (Not Responding)" title): `DOOR_P13 WIN32
  NOT_RESPONDING pid=P window=W`. Close (Alt+F4, caption) on the ghost opens
  "<title> is not responding" with End task and Wait (Wait focused; Esc =
  Wait, Tab then Enter = End task): `DOOR_P13 WIN32 NOT_RESPONDING_PROMPT
  pid=P window=W end_task=X,Y,W,H wait=X,Y,W,H`. End task logs `DOOR_P13 WIN32
  END_TASK pid=P window=W`, then `DOOR_P13 OWNER_RELEASED pid=P windows=0
  surfaces=0 buffers=0 attachments=0 bytes=0` and P's retirement with a
  nonzero exit (Init terminates the GUI child); Wait logs `DOOR_P13 WIN32 WAIT`
  and keeps P. Nothing else terminates a Win32 client except the native
  containment at shutdown and sign-out (`DOOR_P11 CLOSE TIMEOUT revoked` for a
  client that never retrieves its CLOSE).

## Suites (`cargo xtask test compat --suite ...`)

| Suite | Flow | Pass |
|---|---|---|
| `win32-gui-f1` | Terminal: F1, then `--reopen` | exits 75/76 as uid 1001; window seen in screendumps; results files on the raw disk equal the reference; stdout in the console stream; saved document hash equals the reference; installed executables are the reference bytes |
| `win32-gui-f2` | Terminal: F2 and F2-RC `--selftest`/`--reopen`; Explorer: F2 interactive (type, Ctrl+S Save As, Ctrl+K chrome, caption drag +120,+80, grip drag +40,+30, close box, Alt+F X), relaunch (Ctrl+O, Ctrl+End, append, Ctrl+S, Alt+F4); F2-RC About (Alt+H A, Enter) | self-test results equal the reference; interactive results file exactly the expected 13 lines (dialogs, saves with byte counts and CRC-32, chrome geometry 880,200 360x230, two clean exits); saved document bytes on the raw disk |
| `win32-gui-f3` | Terminal: `--selftest`, `--reject`; Explorer: interactive, idle 6 s, `h`, ghost after 5 s without retrieval, Alt+F4 then Wait, Alt+F4 then End task; then Notes saves | results equal the reference plus `HANG_START/READY/STOP` and nothing after; an idle pumping window is not hung; ghost record and changed caption; Wait keeps the process; End task gives a nonzero retirement, never a close deadline; window server `DOOR_P13 OWNER_RELEASED` with zero counts (requested); Notes document saved; records `DOOR_P13 WIN32 NOT_RESPONDING`, `NOT_RESPONDING_PROMPT`, `WAIT`, `END_TASK` |
| `scenario-141-6` | Terminal: F3 `--selftest` while Notes saves; then F2 `--selftest`, F1 | Notes saves before F3 retires; ordering/probe records equal the reference |
| `scenario-141-5` | Notes and Explorer native windows, F2 from Explorer: type and save, Win+Up/Down, minimize, Alt+Tab restore, type after restore, Ctrl+A/C into Notes and a Notes line (Shift+Home, Ctrl+C) pasted into F2, Ctrl+K chrome grip resize +30,+20, a held grip resize closed by Alt+F4 before the button is up, caption drag of the main window +160,+90 and corner resize +80,+60, close, relaunch and reopen, then the F3 hung flow and Notes again | exact document bytes, Notes content (both clipboard directions and responsiveness); maximize, minimize and restore change the screen; chrome records open, grown `moved` (350x220) and no Esc/close-box record; live resize then disappearance in screendumps; the main window content displaced by exactly the drag, status bar and edit scroll bar displaced by the resize (client 704x440); two clean editor exits each followed by `DOOR_P13 OWNER_RELEASED` with zero counts; F3 hung records |

Every suite also requires the kernel's once-per-boot application-path rollback
record `DOOR_P13 APP_ROLLBACK PASS checkpoints=N modules=M runs=R tls=T leaks=0`
(every construction checkpoint of the first ordinary application failed once,
nothing leaked) and keeps its evidence: the sealed serial log, the staged image
record and the screendumps are copied into `reports/runs/<id>/retained/` and the
run directory, and the report cites the copies.

All content checks read paused-guest disk copies and the final image (journal
replay, ext2 tree, catalog); the final image must have a clean journal.

## P13 gate commands

`cargo xtask verify --phase P13` adds, after every P12 command:
`cargo test --locked --package door-windows-gui`, `cargo test --locked
--package door-pe-loader --test p13_application`, and `cargo clippy --locked
--package door-{user32,gdi32,comdlg32,comctl32} --features guest-pe --target
x86_64-pc-windows-msvc -- -D warnings`.

## Not covered (recorded gaps)

Multilingual Win32 text input through an IME, cross-process SendMessage
(declared non-goal), DPI-scaled Win32 coordinates (suites run at 100 % on
1920x1080), comctl32 beyond the status bar, pixel assertions of the "not
responding" ghost (its markers are asserted, its screendump only recorded),
and the Windows "app is preventing shutdown" screen (a Win32 save prompt
during shutdown waits for its answer, as native prompts do).
