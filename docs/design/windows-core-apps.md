# Installed Windows reference and core application inventory

Owner direction, 2026-10-21. The primary reference is the installed Windows 11
Pro 25H2, build 26200.9711, with a 2560x1600 display at 150% and dark app mode.
These values were read from the actual About and Display pages. The package
inventory in `project/windows-app-inventory.json` records locally observed app
versions. This is an owner-configured installation, not a clean-image manifest;
bundled apps also vary with edition, region and servicing.

## Reference observations

Computer Use inspected Explorer (folder details, This PC and its navigation
context menu) and Settings (About, Home, System, Display, Personalization and
Taskbar), Calculator and Notepad, including their settings and mode menus.
Colors was temporarily switched from dark to light and restored to dark; no host
files were edited. Account names, mail addresses, hardware
identifiers and network names are not part of the saved design reference.

- Explorer has tabs, then navigation/address/search, then the command bar. This
  PC shows drives; profile folders remain in navigation. Door's older untabbed
  folder-grid screenshot is superseded as the primary layout target.
- Navigation has expanded/collapsed groups, pinned locations and Recycle Bin.
  Selection controls Cut/Copy/Rename/Delete availability. Context menus depend
  on the target; This PC's menu differs from a file or background menu.
- Settings uses a shared left rail and centered content with an approximate
  1000-DIP maximum. Home has device/service status, recommendation cards,
  Bluetooth and theme cards. Card content depends on real capabilities/state.
- About has Storage/Graphics/RAM/CPU summary cards, device specifications,
  Windows specifications and contextual related links.
- Display separates brightness/color, scale/layout and related settings.
  Brightness, Night light, color profiles, HDR, scale, resolution, orientation,
  multiple displays, advanced display and graphics are distinct functions.
- Taskbar has Taskbar items, System tray icons, Other system tray icons and
  Taskbar behaviors. The observed build includes four position previews, icon
  alignment and size, as well as Search presentation, Task View, Resume,
  emoji, pen and touch-keyboard visibility. Door currently implements only a
  subset. Host enablement is not evidence of a corresponding Door provider.

The reference is organized by window/control identity, theme, scale, state,
input sequence, visible result and backend result. A screenshot alone cannot
qualify file mutation, process control, permissions or device functionality.
Unavailable states use primary documentation; do not change host security or
privacy settings to manufacture a reference.

Notepad's observed version is 11.2607.14: tabs, File/Edit/View, formatting tools,
and status fields for line/column, character count, zoom, line endings and encoding.
Its settings include theme, font, wrapping, formatting, tab/window opening and
startup behavior. Calculator 11.2607.0.0 has memory controls, four keypad columns,
History/Memory, five calculation modes and fourteen converter families. These
are implementation requirements, not implemented Door functionality.

The local command inventory records 298 Windows PowerShell 5.1.26100.9711
core command/function/alias entries and 83 CMD help entries. Enumerating these
commands establishes a reference index only; each needs semantic tests before
compatibility can be claimed.

Additional installed references: Personalization > Background has preview,
picture selection, recent images, Browse photos and Fill/Fit/Stretch/Tile/Center/
Span. Storage has a capacity/usage bar, category rows and Storage Management
sections for Storage Sense, cleanup recommendations and advanced settings.
These host controls were inspected without applying changes or deleting data.

Paint11.2605.81.0 has File/Edit/View, save/share/undo/redo, Selection/Image/Tools/
Brushes/Shapes/Colors/Copilot/Layers, size/opacity controls, canvas resize handles
and status/zoom controls. Save As offers PNG/JPEG/BMP/GIF/Other format; Settings
offers Light/Dark/System. No image was edited. Desktop control subsequently
stopped on the owner's Escape; recorded observations do not authorize ignoring
that stop or establish that any of these Paint functions exist in Door.

## Complete core-family work inventory

The machine inventory distinguishes shell surfaces, independent local apps,
administration tools and device/network/account-dependent apps. Each entry has
its current Door state and required provider. It includes all core families
identified so far; per-page/command/state coverage must be expanded as inspected.
An entry marked missing is a requirement, not a working implementation.

The essential local experience comprises shell surfaces, Files/Recycle Bin,
Settings, Terminal, Notepad, Calculator, Clock, Task Manager/System Tools,
Paint, Snipping Tool, Photos, archive handling, accessibility and recovery.
Network/account/device families include browser, mail/calendar, media, recording,
camera, printing, app installation/store, backup, remote desktop and security.
Cloud/AI/news/gaming/phone companions are recorded separately from necessities.
Office, third-party developer tools, games and VPN clients are not stock-core
requirements merely because they are installed on this machine.

## Unified design proposals

| Workspace | Unified user experience | Backend separation that remains necessary |
|---|---|---|
| Settings | One searchable category/detail system for modern pages and advanced dialogs; preserve deep links, breadcrumbs, keyboard focus and disclosure controls. | Typed capability-aware providers for display, devices, storage, network, identity and policy. No separate writable copies of the same setting. |
| Terminal | One Windows-style tab/split host with profiles, settings, scrollback, selection, clipboard and process lifetime controls. | Native, CMD and PowerShell are distinct language engines. Preserve batch syntax, exit codes and object-pipeline semantics; aliases do not establish PowerShell compatibility. |
| System Tools | One coherent navigation frame for processes, performance, services, devices, disks, logs and scheduled tasks. | Separate least-authority services and explicit permission boundaries; unprivileged UI cannot terminate arbitrary processes or edit kernel state. |
| Files | Modern/classic context presentations, archive views, previews and properties share command identities and transaction state. | VFS mutations, trash/restore, clipboard transfers, associations and extensions are providers, not duplicated menu implementations. |
| Media | Shared library, playback and device selection with consistent theme/controls. | Decoders, audio/video drivers and recording ownership remain isolated. Keep photo/paint editing workspaces focused on their distinct tasks. |

Microsoft already defines [Terminal as a host for separate shells](https://learn.microsoft.com/en-us/windows/terminal/faq).
The proposal extends that separation to Door's original implementation; it does
not replace distinct languages with a superficial shared command parser.

## Backend acceptance requirements

### Proposed bundled workspaces

The owner requested the proposed grouping explicitly. This is the target bundle;
it does not change the current three-app catalog or claim implemented providers.

| User-facing workspace | Included functions |
| --- | --- |
| Files | Explorer, Recycle Bin/restore, archives, previews, properties and file associations |
| Settings | Modern Settings and relevant Control Panel configuration in one searchable system, with advanced subpages and deep links |
| Terminal | One tab/split host and profiles; separate native, CMD and PowerShell language engines and compatibility tests |
| System Tools | Task Manager, resource/performance monitoring, startup apps, services, devices, disks, events and scheduled tasks |
| Notes | Text editor, tabs, find/replace, encoding/line endings, font/zoom and document status |
| Calculator and Clock | Calculation modes/converters/history; alarms/timers/stopwatch/calendar |
| Paint, Photos and Snipping Tool | Shared image/color/codec infrastructure, distinct editing, library and capture workflows |
| Media Player | Audio/video libraries, playback and output-device controls |
| Optional device/network apps | Camera, Sound Recorder, Browser, Mail/Calendar, Backup, Remote Desktop and Package Manager after their driver/network/security gates |

Start, local Search, taskbar/tray, notifications, Quick Settings and lightweight
optional Widgets are shell surfaces. Keep visible modules customizable and
preserve private local use without a promotional/news feed. Each writable setting
has one authoritative provider. Unavailable hardware has no fictional readings.

- Files: enumerate/stat/open/save, binary-preserving copy, atomic rename/move,
  collision handling, permissions and symmetric sharing, cancel/failure recovery,
  recursive traversal bounds, durable Recycle Bin metadata and restore, independent
  raw-byte audit after reboot. Permanent deletion must be explicitly identified.
- Terminal: a protected terminal/PTY service, child launch and working directory,
  environment, stdin/stdout/stderr, encoding, command grammar, pipelines and
  redirection, exit status, Ctrl+C and process cleanup. The native shell's current
  real commands are recorded separately from unimplemented CMD/PowerShell commands.
- Settings: typed query/apply/read-back and durable per-user/system scope; reject
  unavailable hardware/functions. Dynamic DPI, GPU modes, sound, network and
  account/security providers require actual implementation and negative cases.
- All apps: theme/scale/state capture pairs, real keyboard/pointer behavior,
  accessibility semantics, bounded resources, storage/IPC failure containment and
  clean process/window retirement. Preview pixels never substitute for behavior.

The official [Settings URI index](https://learn.microsoft.com/en-us/windows/apps/develop/launch/launch-settings)
and [Windows command reference](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/windows-commands)
are coverage indexes. Availability is checked against the installed build and
actual providers. Full command compatibility and pixel-equivalence remain open
requirements, with no completion percentage claimed.

## Implementation order within the existing phase gates

The rendering, checked-rename and synchronous RPC batches have reviewed native
evidence; see `rendering-responsiveness.md`, `files-mutations.md` and
`rpc-responsiveness.md`. Continue shared P12
text, menus, dialogs, tabs and accessibility with Files/Notes/Settings consumers.
Add the remaining durable Files
mutations and the protected terminal provider before exposing enabled UI commands.
Use local Calculator/Clock/editor consumers to validate shared primitives.
Stock Windows GUI adapters remain P13; full shell/app fidelity P14; network/audio
and connected app functionality P15. The global eight-process budget and fixed
three-app launch/query catalog must be replaced or explicitly bounded before a
broader app catalog is admitted. Tests must exercise exhaustion and recovery.

Microsoft's current [typography guidance](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/typography)
uses Segoe UI Variable with weight/optical-size axes for modern controls. Door's
native renderer now selects pinned hinted Segoe UI Variable caches at1x/2x/3x,
with RGB LCD coverage on opaque surfaces. See typography-cache.md for measured
native constructor behavior, checked bounds and regression evidence. This is an
original renderer, not Microsoft ClearType or a pixel-identical DirectWrite path;
full Unicode shaping/IME and independent Windows pixel comparison remain open.
The [TabView guidance](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/tab-view)
also requires actual active content, selection after closing, keyboard commands
and preserved title-bar drag/system-button regions. A painted tab strip alone
does not satisfy the modern Explorer requirement.

## Latest owner concepts

The five retained concepts in reports/runs/user-reference-20261021-unified-apps/
now direct implementation: Settings Advanced first, then Explorer, Preview,
Terminal and Monitor. See unified-apps-owner-concepts.md for the complete page
and provider inventory. Preview combines inspection/edit/conversion capabilities;
Monitor avoids redundant metrics. Pictured versions, system values and applications
are illustrative and are not implemented-runtime evidence.
