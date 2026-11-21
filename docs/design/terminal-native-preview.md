# Current audit validation - 22 October 2026

Current application audit validated: theme, measured alignment, cursor and redirect completion fixes passed the full host suite and the focused two-boot common-image native check. Settings now lists and launches Terminal. See current-app-fidelity-audit-20261022.md for exact evidence and remaining engine/rendering limits.

# Current reference audit — 22 October 2026

Terminal now follows live desktop light/dark application themes independently of
its console scheme, uses metric-centered controls, correct text cursors and checked
redirection completion. Installed Windows Terminal artwork is pinned in
terminal-current.source.json with authored small frames. Settings includes its
actual launch action. See current-app-fidelity-audit-20261022.md for current proof
and precise remaining limits. This is still partial native functionality.

# Terminal native preview

The owner requested Terminal on 22 October 2026, based on the attached unified
terminal concept. The app is named **Terminal**. The reference's sample command
output, versions and hardware are illustrative, not system inventory.

Current implementation stays within the partial P12 native desktop. Terminal is
a fourth catalog application, launched through authenticated Init admission and
the existing Workspace executable (startup mode 14). Desktop command 9 launches
it; commands 4/5 keep their existing power semantics. Catalog records use app
identity 4 for Terminal, separately from launch command 9. Desktop/taskbar/Start,
workspace placement, foreground intent, shutdown and process retirement include
the fourth app. The adversarial workload keeps its own fifth tracking slot.

The interface follows the reference's pale navigation rail, tab strip, blue
active indicator and dark command canvas. Home, Settings, About
and Profiles are native painted pages. The existing trusted Window server frame now hosts the integrated
tab caption, with its own authenticated move/resize/buttons/shadow policy. This is an implementation inspired by the reference, not pixel-exact
or Windows 11 visual qualification. No ImageGen assets were needed: controls use
the existing native toolkit and pinned WindowsIcons artwork.

Native shell sessions execute authenticated file operations. Implemented commands
are help, pwd, cd, ls, stat, cat, echo, write, append, touch, mkdir, cp, mv, rm,
env, whoami, clear and exit. Text producers support > and >> through the native
parser. Native paths use slash separators and native quoting. This does not
implement CMD or PowerShell syntax. File reads are bounded to 64 KiB, directory
enumeration to 512 entries, and checked file replacements/copies to 4096 bytes.
Replacement stages an object/version-bound candidate and uses the existing
durable commit service. Explicit native rm permanently removes a single file or
empty directory under storage authorization, as in the existing text shell.

Tabs independently retain directory, command input, scrollback and history.
Limits are eight tabs, 512 scrollback lines of at most 2048 UTF-8 bytes each,
2048 input bytes and 64 history entries per tab. History remains in RAM and can
be disabled/cleared in Settings. Ctrl+T adds a native tab; Ctrl+W closes a tab;
Ctrl+Tab/Shift+Ctrl+Tab switch; arrows/Home/End/Delete edit; Up/Down recall input;
Ctrl+C cancels the input line; Ctrl+L clears; Ctrl+F searches; PgUp/PgDn scroll;
Escape returns Home; Ctrl+comma opens Settings. Text uses fixed cell positioning
of owner-installed Cascadia Mono, with three sizes and light/midnight palettes.
Segoe UI remains the interface font. The monospace atlas is built once per
connection/density and does not rasterize during painting.

Settings apply immediately in RAM. Explicit Save writes the strict versioned
eight-byte terminal.conf in the authenticated home using a checked replacement.
Malformed or concurrently changed settings never silently overwrite durable data.
An uncertain commit disables further saves until the app is reopened and loads
the actual durable checkpoint. No failed commit is automatically replayed.

CMD and PowerShell are visible profiles with explicit runtime requirements.
Neither is advertised as active: Door OS has no qualified cmd.exe engine,
PowerShell engine/.NET runtime or per-session pseudoconsole provider. Their
profile views never invoke the native parser. .bat/.cmd/.ps1 execution and full
Windows command and DLL compatibility remain **unimplemented**. WSL is explicitly
**Work in progress**; no Linux environment is claimed. DLLs require the process
loader and compatible APIs, rather than being commands or shell scripts.

Other open work: real child stdin/stdout/stderr routing and lifetime/interrupt
semantics, terminal escape emulation, full-screen programs, split panes,
clipboard/output selection, custom engine-backed profiles, accessibility,
Unicode shaping and small-window scrolling. Tiny restored windows may clip the
informational pages; ordinary and maximized layouts are the current target.

Validation uses terminal_model unit tests, the actual native painter at 1x/2x/3x
in both desktop themes, native target compilation, and cargo xtask test
terminal-current. That workflow builds a real guest, uses a fresh private disk,
injects native input, captures pages, checks real file commands, saves preferences,
shuts down, independently inspects raw persistent bytes, then reboots/reopens.
It never uses out/session/user.raw. Results and independent critic bindings are
recorded separately; historical phase evidence remains unchanged. P12 stays open.

Owner refinement, 22 October: Terminal, Settings and information icons are from
HaydenReeve/WindowsIcons commit9442e3ef014520d4337409a24be10b970a36d66b, pinned in
assets/icons/windows/manifest.json. This collection lacks distinct CMD, PowerShell
and Ubuntu icons; profiles use its genuine Terminal icon with explicit labels.
No arbitrary substitute engine icon is drawn. Controls follow an8px spacing
grid,32px action controls,70px settings rows, rounded surfaces and the existing
cached native window shadows. Settings has actual scheme/text/history controls
and a fixed Save footer/Ctrl+S. Primary references: Microsoft Learn Windows
Terminal Appearance and Startup settings pages. Installed Windows Terminal
1.24.12741.0 was identified read-only. Computer Use initialization failed before
opening any window (kernel assets path error), including reset/retry, so no live
installed Settings inspection is claimed.

https://github.com/HaydenReeve/WindowsIcons
https://learn.microsoft.com/en-us/windows/terminal/customize-settings/appearance
https://learn.microsoft.com/en-us/windows/terminal/customize-settings/startup

Independent review repaired four initial findings: explicit command9 retirement
validation, logical-catalog-to-backing-object disk auditing, visual-row scroll and
search, and checked recovery of an empty initial preferences file. A subsequent
resize/font wrap-count finding is fixed by clamping effective painting offsets
and normalizing on the next scroll, with a narrow-to-wide regression. Empty
preferences remain object/version checked; no foreign replacement is removed.

The owner requested another direct-reference correction. Terminal now uses the existing Window server client-caption path, extended for a56DIP tab strip and bounded interactive span. Trusted frame buttons, shadows, rounded outer clipping, resize, move and close remain server-owned. A cached reduced wallpaper blur supplies the premultiplied frame/sidebar material; no client background screenshot or per-hover blur is used. Settings/About/Profiles use solid light content. The sidebar is208DIP, with64DIP Home,40DIP profile rows, standard20DIP Segoe Fluent outline glyphs, actual official blue PowerShell/CMD/Ubuntu artwork, and12DIP tab close glyphs. The owner explicitly approved official sources beyond WindowsIcons for proper profile icons. Pinned asset URLs and hashes are in assets/icons/windows/manifest.json. WindowsIcons application assets retain their existing provenance; official Microsoft and Canonical marks do not acquire an MIT license merely from being in a public repository.

Four focused Terminal tests (plus one shared icon-cache regression) cover actual painting and shared Engine hit agreement at320/520/1050/1128 widths,8tabs and1/2/3 density. Two new core tests cover owner boundaries, tall caption controls, cached material, partial/full caption equivalence and in-flight invalidation. All73 core tests pass. Whole-workspace host coverage passed in two isolated invocations because another chat held the default tools-test executable:609 non-tools tests plus147 tools tests,756 total. Native development run reports/runs/host-1792605818661788500-4920/report.json completed two boots,11 page captures, checked native-file bytes/preferences/removal/journal auditing and process/device retirement. Its final source-stability check failed because Explorer/Settings/tool/docs sources changed during the run; retain it as diagnostic evidence. A final stable run is pending. Cumulative verify reports/runs/host-1792606052917956000-23184/report.json stops at the dependency-review lock mismatch. No full phase, CMD/PowerShell runtime, exact1to1 or frame-cadence qualification follows.

Owner cleanup on22October2026 removed the entire reports/runs directory to reclaim over100GB. The earlier diagnostic paths above are historical observations whose underlying run records are now absent; they are not retained or qualifying evidence. The owner explicitly authorized regenerating only current needed checks/screenshots. Do not reconstruct deleted history. Current source/fonts/icons and target host logs remain; a fresh shared-image Terminal report is pending.
