# Unified applications: owner concepts, 21 October 2026

The five primary concept images and hashes are preserved in
`reports/runs/user-reference-20261021-unified-apps/index.json`. They supersede the
earlier grouping proposal where different. Their text, names, versions, hardware,
files and graphs are illustrative. They do not establish installed capabilities.
Implement Settings first, followed by Explorer, Preview, Terminal and Monitor.
This is the implementation inventory, not a completion claim.

Settings retains its familiar categories and adds a collapsible Advanced group.
The navigation rail scrolls independently of the selected page; search finds
advanced pages even when the group is collapsed. Back/forward navigation,
keyboard focus, narrow windows and both themes share the same geometry. Existing
supported preferences keep their current persistent service implementation.
Administrative pages live inside Settings, with contextual links from related
ordinary settings, rather than opening another legacy configuration window.

| Advanced page | Required real behavior and provider |
| --- | --- |
| Devices & drivers | Enumerate acquired guest devices, driver binding, resources and errors; authorized driver operations |
| Disk management | Actual disks, volumes, capacity and filesystem state; transactional, authorized storage operations |
| Network administration | Guest adapters, addresses, routes, name resolution and connection policy |
| Services | Supervisor-owned identity, status, dependencies and authorized start/stop/restart |
| Environment variables | Typed user/system values, durable edits, correct expansion and inheritance by newly launched processes |
| System configuration | Boot/startup/service policies with validated persistence and explicit restart effects |
| Registry | Real roots, views, enumeration, typed values, permissions and atomic durable mutation |
| Group policy | Effective policy and source/precedence, with enforcement in the affected providers |
| Event viewer | Authentic structured events, filtering, bounded retention and export |
| Task scheduler | Persistent owned tasks, real clocks/triggers, process launch and execution history |
| Users & groups | Authenticated identities, membership, account lifecycle and permissions |
| Permissions & sharing | Object identities, ACL/inheritance, effective access and genuine sharing providers |
| Performance & diagnostics | Link to Monitor with contextual tests and actual results |
| Power management | Acquired power capabilities, policies and supported device readings |
| Backup & restore | Consistent snapshots/backups, verification and checked restoration |
| Developer & tools | Installed SDK/runtime capabilities, diagnostics and developer policies |

Current reusable foundations include checked persistent registry open/query/set,
filesystem identities/versions and transactions, authenticated service transport,
supervised processes and session authority. Registry enumeration and a public
system-inventory/process-metrics provider are missing. Add those contracts and
their negative tests as real capabilities; a graphical toggle alone cannot supply
them. Never replace unsupported operations with simulated success or sample data.
Kernel/device control continues through capabilities and provider authorization.

Explorer uses tabs, navigation/address/search above commands, and an optional
right pane for preview, properties, contents, tags and relevant quick actions.
Each tab owns its location, history, selection, scroll and pending operations.
New windows and tab transfer preserve object identity and handle ownership.
Details columns use actual metadata; unknown dates stay absent until timestamps
exist. File size and folder-child counts must not trigger recursive scans during
paint. Thumbnails and previews use bounded asynchronous caches and cancellation.
Open, rename, copy, move, recycle/restore, permissions, archives and launch-in-
terminal actions must use the same checked storage and launch providers.

Preview is a focused multi-format app with tabs, a content canvas, optional
information/actions pane and an optional filmstrip. Inspection, editing,
conversion and OCR are distinct capabilities. Decode/parse untrusted files with
bounded resources; bind metadata and edits to the opened object/version. Save As
and conversion use durable checked writes, report errors and preserve the source
unless replacement is explicitly selected. Image rotation/crop/markup, PDF tools,
media, archives, 3D and OCR each require their real format or runtime provider.
Do not fill unsupported tabs with a screenshot of apparently rendered content.

Terminal shares tabs, split panes, profile management, selection, scrollback,
search, copy/paste, fonts, themes and command discovery. Profiles are distinct
execution engines: native shell, CMD, PowerShell and an actual Linux environment
when present. Their quoting, pipelines, redirection, environment, jobs, interrupt
and exit semantics need conformance tests. A renamed native parser cannot qualify
PowerShell or WSL. The profile rail may collapse to keep working space clear.

Monitor integrates process/resource monitoring with diagnostics. Use one summary
per metric and one detailed view when selected. Do not duplicate Network or Disk
charts in the same overview, or invent two compositor processes from the picture.
Sample actual providers into bounded histories away from painting; update only
changed visible regions. Support filtering/sorting by stable process incarnation,
accurate units, pause/live modes and authorized process/service actions. Missing
GPU, battery, network or temperature providers produce an accurate availability
state. The mockup's sample percentages and process list are never runtime values.

All five apps share native-size icons, measured text baselines, rounded controls,
focus/hover/press treatment, cached shadows, themes, capability/error presentation
and asynchronous operations. Extend the established Windows/vector asset pipeline
for icons; avoid generating a bitmap imitation where an existing symbol suffices.
Use the current source/evidence/critic routine for each bounded implementation.
Visible native quality, fractional DPI, Snap/Search controls, durable trash and
verified display cadence remain open alongside this application work.
