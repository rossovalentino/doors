# Responsiveness, density and Explorer refinement

The current batch is implemented and host-checked; final combined native evidence
and critic binding are pending. Prior accepted checkpoints below are historical.

- Exact comparisons of validated private snapshots identify changed pixels;
  identical commits release buffers without scheduling another redraw.
- Repeated activation of the already focused topmost window stays idle. Regional
  composition matches full composition through overlap, alpha and lifecycle.
- Ordinary frames restore only the 48 logical taskbar rows and update only changed
  cached underlay pixels. Unaffected blur is reused. Flyout/overview transitions
  and desktop-icon dirtiness retain the full-render fallback.
- Contiguous full-row scanout spans keep the existing 4 KiB display ABI. Input is
  serviced every 32 KiB during main/flyout scanout; cursor paint precedes compose.
- QHD/4K use native 200%. QHD1440 retains 1280x720 logical units. Default startup
  stays windowed 1280x720. Host enlargement/letterboxing and non-atomic GOP copies
  still limit perceived sharpness and motion; there is no FPS or vblank claim.
- Explorer has actual supported Windows spellings, caret/selection editing and
  measured breadcrumbs. See [folder/API coverage](../design/windows-paths.md).
  No owner data migration or complete Windows compatibility is implied.

The first full run 19744 passed actual path aliases/denials but failed a new
fixture that matched an old log before the edited address result arrived. The
corrected offset-scoped fixture passes in focused 12972 (122 unchanged checks).
Visual review then caught clipped breadcrumb labels from guessed character
widths; exact font metrics now drive both drawing and hit targets. Final evidence
must cover that repair rather than reusing 12972 as its proof.

## Settings proposals from established customization projects

These are proposed Doors preferences, not installed mods or completed controls.
Project feature lists show specific requests; they do not establish a global
ranking of hated features or installation counts.

| Proposed preference | Primary reference | Integration requirement |
| --- | --- | --- |
| Taskbar size, gaps, alignment, labels/grouping | [Windhawk Taskbar Styler](https://windhawk.net/mods/windows-11-taskbar-styler), [ExplorerPatcher](https://github.com/valinet/ExplorerPatcher/wiki/All-features) | Shared work area/hit geometry, persistence and every-edge Snap/tray tests |
| Clock seconds, date format, additional zones | [Windhawk clock customization](https://github.com/ramensoftware/windhawk-mods/blob/main/mods/taskbar-clock-customization.wh.cpp) | Real time-zone provider; avoid constant full-bar redraw |
| Start section visibility and grid/list density | [ExplorerPatcher](https://github.com/valinet/ExplorerPatcher) | Preserve keyboard/search and actual installed-app authority |
| Custom Snap zones/workspace layouts | [PowerToys FancyZones](https://learn.microsoft.com/en-us/windows/powertoys/fancyzones) | Checked geometry, accessible editor and restore semantics |
| Context-menu density/inline commands | [ExplorerPatcher features](https://github.com/valinet/ExplorerPatcher/wiki/All-features) | Shared commands and actual selected object/error/cancel state |
| Reduced motion/transparency and density controls | Doors performance/accessibility direction | Actual compositor/text behavior, preview/revert and persistence |

Prioritize density, reduced motion and shared commands before appearance-only
presets. Availability in Windows varies by build/device/rollout; this is not a
claim that all these features are absent from current Windows. App priorities
remain Files, Notes and Settings behavior before another decorative launcher.

# Native desktop audit and continuation plan

Latest finite refinement:178 unchanged GUI checks in
reports/runs/host-1792460182885722800-13360/report.json, independent18720 build,
169 affected host tests and strict checks passed. The13 Start/tray focus cases
include26 independently audited Notes writes; all three output profiles pass.
Critic record: reports/critics/p12-category-refinement-critic.json (accepted finite development scope; no unresolved actionable findings).
Critic SHA256 eba39b5b95cc9507622845dbf745c74d1cde04fbea2c71d093b61a2dba98b7d3.
Earlier160/167-check reports below remain historical. No phase closure.


Reviewed2026-10-20 against master section91 and WINDOW-A/B. P11 passed its
historical native software-display profile. P12 remains active and partial.
The master assigns complete shell/fidelity to P14, after toolkit P12 and Windows
GUI P13. These refinements do not establish broad Windows GUI compatibility.

## P11 requirements and actual implementation

| Requirement | Implemented | Remaining boundary |
| --- | --- | --- |
| Window authority | Authenticated owners, generation IDs, protected service, checked capture | Clients cannot acquire trusted global keyboard/display authority |
| Composition | Private snapshots, clipping, damage, stacking, rounded chrome, CPU transfer | No GPU/vblank or60FPS guarantee |
| Surface lifecycle | Configure/ack/attach/commit/release, stale epoch rejection, recycled captures | Fixed quotas and three large retained-client capture budget |
| Input | Focus/capture revocation, actual keyboard/pointer, RESET cancellation | Unicode/IME, text services and complete accessibility remain P12 |
| Containment | Bad buffers/owners, producer death, hung clients, broker termination, server restart | Historical evidence does not qualify every new shell gesture |
| Density | Native96DPI and4K192DPI, outline fonts, original ICO/CUR density frames | No fractional/dynamicDPI or multi-monitor negotiation |
| Clients | Independent native processes, actual files, notes and durable appearance | Not stock Win32 GUI apps; P13 unqualified |

Current refinement includes trusted passwordless sign-in, six presets/four offline
wallpapers, live themes, actual UTC clock/calendar, installed-app Start, caption
glyphs, Search and Task View. Shared taskbar geometry supplies paint/hit targets.
Unavailable battery/network/audio capabilities are not inferred from host hardware.

Current evidence: integrated273checks in
reports/runs/host-1792436289607278800-7072/report.json, then the final native GUI
suite160checks in reports/runs/host-1792442174995919700-18536/report.json after
the ownership/capture/theme-preview/gallery-spacing repairs. Both passed with unchanged inputs. The latter
includes private-disk persistence/recovery, actual gesture denial, dual-Alt release,
all three resolution profiles and light/dark4K previews.148 core/tooling and16 app host tests
and strict native/host checks passed. These are bounded functional samples.

The critic caught incomplete retained4K Alt+Tab evidence in7644. It remains
retained; the replacement waits for an offset-bound native CPU-transfer completion
marker and asserts actual card/selection pixels before Alt release. PNG and PPM
now share exact pixels. Focused19104 passed16checks and independent pixel review.

Multitasking has eight generation-checked session desktops, focus/capture revocation,
moving actual previews between desktops and close/merge without terminating clients.
Alt+Tab freezes MRU order for its held gesture and activates on Alt release.
Previews cache accepted snapshot revisions. Launches retain their originating
workspace. Bundled apps remain global singletons: activation switches to an
existing app's desktop. Desktops share wallpaper and do not survive reboot.

Independent finite-scope acceptance is recorded in
reports/critics/p12-desktop-refinement-critic.json. Earlier3092 predates the final visual
repairs. Sparse sequence captures do not establish every frame is fully settled.

## Settings inventory

The supplied Home image is a direct layout reference. The category inventory comes
from [Microsoft Settings overview](https://support.microsoft.com/en-us/windows/experience/exploring-windows-settings).
A category icon/overview does not establish that category's complete page layout.

| Page | Working Doors behavior | Missing service/layout qualification |
| --- | --- | --- |
| Home | Actual account/app navigation, themes/device status | Full size/theme comparison; cloud account/storage provider |
| System | Actual dimensions and About | Runtime scale/mode, sound, power and storage pages |
| Bluetooth & devices | Input profile, unavailable Bluetooth | Device management service and per-page reference |
| Network & internet | Truthful no-adapter state | Network configuration service and per-page reference |
| Personalization | Four backgrounds, six durable presets, live themes | Taskbar edges/alignment, density/accent/motion settings, page calibration |
| Apps | Three real catalog apps | General installation/uninstallation/inventory |
| Accounts | Authenticated local scope, files link | Multi-account editing and credential provider |
| Time & language | Actual UTC calendar access, US ASCII description | Time zones, layouts, Unicode/IME and locale formatting |
| Gaming | Explicit unavailable capabilities | Game Bar, capture and GPU services |
| Accessibility | Existing keyboard/pointer operation | Semantic tree, screen-reader evidence, text scale/high contrast |
| Privacy & security | Actual process/file boundaries | Permission/security-provider controls, page reference |
| Doors Update | Explicit unavailable servicing | Signed update/rollback service |
| About | Actual platform/development identity | Versioned release metadata and measured layout |

Reference account names, hardware, capacities and successful updates must never
be copied into live device data. Frontend names use Doors. Accurate Windows
provenance/source and compatibility ABI names remain in engineering material.

## Reference baseline

Offline pictures/hashes are in docs/references/desktop-shell/sources.json. The
Microsoft diagram guides48px bar height,32px rounded Search,24px app icons,
4px group gaps, running indicators and the right tray. Inventory determines which
slots exist. Keyboard language belongs in the right system tray; there is no
alternate layout switcher on the current US ASCII-only profile. Battery requires
a real guest provider and is absent on this QEMU desktop profile.

- [Taskbar reference](https://support.microsoft.com/en-us/windows/experience/personalization/customize-the-taskbar-in-windows)
- [Broad rollout15October2026](https://blogs.windows.com/windowsexperience/2026/10/15/whats-new-in-windows-11-everyday-experiences-reimagined/)
- [Start design](https://microsoft.design/articles/start-fresh-redesigning-windows-start-menu/)
- [Task View, Alt+Tab and desktops](https://support.microsoft.com/en-us/windows/how-to-multitask-in-windows-b4fa0333-98f8-ef43-e25c-06d4fb1d6960)
- [Snap](https://support.microsoft.com/en-us/windows/experience/snap-your-windows)
- [Caption glyphs/geometry](https://learn.microsoft.com/en-us/windows/apps/design/basics/titlebar-design)
- [Input language tray](https://support.microsoft.com/en-US/Windows/Hardware/Input-Devices/manage-the-language-and-keyboard-input-layout-settings-in-windows)

Experimental taskbar positioning/Search announcements are distinct from universal
public rollout. Build, flags, locale and DPI need an explicit capture baseline.
Pictures guide this original renderer; measured1:1 fidelity is not established.

## Ordered work

1. Maintain combined shell/multitasking, screenshot/raw-byte and critic regressions
   while extending controls. Preserve failures and the owner disk.
2. P12 shared target-specific menus: disabled state, separators, release/cancel,
   keyboard focus and submenus, reused by Files/Notes/shell. Then marquee/drop.
3. Taskbar edges/alignment through one shared work-area/anchor model and durable
   Settings. Rebase maximize/Snap/flyouts/hit/DPI together; test every edge.
4. Title-bar restore-on-drag/double-click, Snap Assist, Win+Z, quarter-snap keyboard
   sequences and Task View contextual Move to. Implemented: Win+E/I/S/D/Tab,
   Win+arrows, Win+Ctrl+D/F4/Left/Right and held Alt+Tab.
5. Complete P12 Unicode editing/shaping, IME, clipboard, semantics/accessibility
   and component gallery/editor gates.
6. P13 independent Win32 GUI/message/paint/dialog trials; P14 complete shell,
   notification producers and per-page quantitative visual/motion calibration.

## Researched customization opportunities

No authoritative global ranking of hated features or installed mods was found.
Microsoft issue summaries and primary project feature lists provide specific
requests. GitHub popularity is not an installation count.

| Opportunity | Primary source | Doors direction |
| --- | --- | --- |
| Fast configurable context menus | [Microsoft Explorer update](https://blogs.windows.com/windows-insider/2026/08/17/improving-file-explorer-context-menu-faster-simpler-and-more-customizable/) | Shared commands, then optional inline actions/density |
| Labels/grouping and classic menu/switcher options | [ExplorerPatcher](https://github.com/valinet/ExplorerPatcher/wiki) | Validated preferences after primitives; historical feature lists may be outdated |
| Taskbar style/spacing/behavior | [Windhawk](https://github.com/ramensoftware/windhawk), [mods](https://github.com/ramensoftware/windhawk-mods) | Built-in preferences without runtime process patching |
| Start/taskbar/Explorer appearance | [StartAllBack](https://www.startallback.com/) | Reversible optional shell presets, P14 |
| Custom window zones | [PowerToys FancyZones](https://learn.microsoft.com/en-us/windows/powertoys/fancyzones) | Extend authoritative Snap geometry, then zone editor |
| Start section visibility/all-app views | [Microsoft design](https://microsoft.design/articles/start-fresh-redesigning-windows-start-menu/) | Installed-app grid/list, then durable section preferences |

These are researched options, not claims that Windows2026 lacks them or Doors
already implements them. No customization utility was installed on the host.


## Category and P11 boundary recheck (20October2026)

The master phase table explicitly assigns window authority, surfaces, CPU
composition, input routing and owner/epoch/release safety toP11. Those foundations
retain their historical qualification, and the current native regression repeats
containment/lifecycle/recovery and resolution checks. This is not a new P11
closure, complete desktop, universal crash-free guarantee or Win32 GUI claim.
P12 still needs Unicode/IME, clipboard, semantic accessibility and a component
corpus. P13 owns stock Windows GUI behavior. P14 owns full shell/services/fidelity;
network/audio/media providers requireP15. These are real remaining boundaries,
not cosmetic tasks that can be solved with success-returning controls.

The new category packet contains12 original screenshots with provenance and
unknown build/DPI (nine2021 Windows Central, Gaming ASUS2023, two Display ASUS
images dated2021/article2025). The owner's newer Home/top-search shell and older
untabbed Explorer remain explicit separate baselines. Per-category section/row
inventories, tighter geometry and distinct native-density Fluent glyphs replace
the previous repeated three-card/category-icon layout. All15 Settings surfaces
remain capability-aware: unsupported rows cannot pretend to change a setting.

Sign-in and local Settings identity now use the filled ContactSolid silhouette
with shared trusted button geometry. The bar retains actual Search, Task View,
three app singletons, show-desktop, UTC date/time, bell/calendar and absent-device
status. Win+A/N now reach the same flyouts by trusted keyboard routing. Battery,
extra keyboard language and network/audio controls require real guest providers.

### Application priorities

| Priority | Actual consumer | Work needed before adding decorative apps |
| --- | --- | --- |
| 1 | Settings | Complete reusable control/focus/semantics and supported service detail workflows; quantitative capture baseline |
| 1 | File Explorer | Durable rename/copy/move/delete, selection/marquee/drop and context-specific commands after shared primitives |
| 1 | Notes | Unicode editing/IME, selection/clipboard/undo and open/save dialogs underP12/P14 |
| 2 | Terminal/log viewer | Native process/console session integration and real bounded output/history, not fake terminal text |
| 3 | Calculator | Native reusable keyboard/edit/control consumer afterP12 focus primitives |
| 3 | Task Manager | Authenticated read-only process/resource snapshot API first; never expose authority through caller-supplied identity |
| Later | Browser/store/media | Actual network/TLS/audio/media backends and application compatibility; no empty lookalike launchers |

No fourth application was inserted into the fixed three-app broker/taskbar catalog
without coordinating its protocol, quotas, authority, lifecycle and tests.

## Current2026 controls and customization priorities

Reviewed20October2026 against official May taskbar, August context-menu and
October15 rollout sources in [Desktop controls](../design/desktop-controls.md).
Four edges, start/center alignment, compact size and context preferences now have
working shared geometry and durable state. There are17 Settings surfaces.
These controls overlap current Windows rollout; they are parity work, not unique
features that Windows universally lacks. Unsupported file providers remain disabled.

The next proposals use primary project inventories, not an unverified ranking of
installations or most-hated features. GitHub stars measure interest, not installs.

| Priority | Doors proposal | Primary reference |
| --- | --- | --- |
| Foundation | Editable Snap zones, saved workspace layouts, Always on top | [PowerToys](https://github.com/microsoft/PowerToys): FancyZones, Workspaces, AlwaysOnTop |
| Foundation | Keyboard remapping, shortcut discovery, scoped per-app profiles | PowerToys Keyboard Manager and Shortcut Guide |
| After file transactions | Quick preview and batch rename with preview/undo | PowerToys Peek and PowerRename |
| P14 | Independent bar spacing/size and optional classic menu/switcher presets | [ExplorerPatcher](https://github.com/valinet/ExplorerPatcher), [Windhawk mods](https://github.com/ramensoftware/windhawk-mods), [StartAllBack](https://www.startallback.com/) |
| P14 | Per-state/app transparency, tint and contrast with safe preview/revert | [TranslucentTB](https://github.com/TranslucentTB/TranslucentTB), StartAllBack |
| P14 | User-managed Start groups/folders and optional floating layouts | [Start11](https://www.stardock.com/products/start11/) |
| Product policy | Local-first search, explicit cloud/AI opt-in, quiet notifications and reversible presets | [Microsoft quality response](https://blogs.windows.com/windows-insider/2026/03/20/our-commitment-to-windows-quality/) acknowledges unwanted entry points, interruptions, Explorer latency and search trust feedback |
| Later updater | Clear update timing/pause/restart controls | Same feedback; requires a real updater and recovery |

Historical mod feature lists do not prove a current stockWindows gap. Prioritize
latency, correct focus/drag/close, durable file operations, Unicode, clipboard and
accessibility before decorative options. No customization utility was installed
or injected into the host, and this table is a proposal rather than implemented UI.
