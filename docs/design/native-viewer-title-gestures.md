# Native viewer defaults and title gestures

Bounded independent acceptance follows the cached-move checkpoint:701 host tests
across34 executables,265 desktop checks/ten sessions,19 isolated4K WHPX checks and
retained revalidation pass. See reports/critics/p12-native-viewer-title-critic.json. Tested
source is8dacdc0ee5cc6f05626cfd68b94ea23954410167f4ed20dfcc14858339828b3e,
2176files. This does not close P12 or establish Windows fidelity or60FPS.

The ordinary launcher now defaults to `--resolution auto`. A bounded, recorded,
read-only Win32_VideoController query selects one distinct supported current mode.
Inactive adapters are ignored. Ambiguous, unsupported or unavailable modes use
the conservative 1280x720 fallback; the record states why. Explicit supported
resolutions remain available for debugging and regression coverage. The current
machine reports 2560x1600. This is adapter configuration, not a primary-monitor
selection policy for multiple distinct active modes.

For visible Windows desktop children, structured process-local overrides request
SDL per-monitor-v2 awareness and pixel coordinates, before video initialization.
Both the process receipt and session record retain the exact overrides. The
parent environment and global Windows settings are unchanged. An actual child
test checks the received values and sealed output. The SDL request does not prove
that a particular binary honored it or establish visible native-pixel quality.
See the primary [DPI-awareness](https://wiki.libsdl.org/SDL2/SDL_HINT_WINDOWS_DPI_AWARENESS)
and [coordinate-scaling](https://wiki.libsdl.org/SDL2/SDL_HINT_WINDOWS_DPI_SCALING)
contracts. Fractional guest DPI and proper live scale controls remain open;
existing percentages still describe the actual integer renderer density.

Title dragging uses a four-DIP movement threshold. A maximized or snapped window
restores its saved normal size, preserving the horizontal press fraction and
vertical title offset under the pointer. The engine configures one new epoch,
retains the previous safe snapshot, and publishes normal state only after the
configure succeeds. Owner/state/press geometry and queue backpressure are checked.
Gesture epochs cancel stale input after another configure, close or retirement.
Release without a real drag does not snap or overwrite saved restore geometry.
Unchanged-position moves no longer dirty the scene.

The native default workflow uses a private volume and the actual auto-selected
WHPX/RAMFB mode. It captures native extents, opens Files, performs an actual top
edge drop and pull-down restore, then a left-edge drop and pull-down restore.
Mandatory pixels check restored chrome, content and exposed background. Paired
PNG/PPM bytes, query selection, process arguments, private catalog/journal state
and canonical retirement are reopened during validation. The full desktop gate
now includes this additional session; historical reports retain their old scope.

Remaining known-folder roles now use installed imageres.dll.mun groups named by
the owner's desktop.ini files: Desktop 183, Downloads 184, Music 108, Pictures 113
and Videos 189. Documents 112 was accepted in the previous checkpoint. All source
frames and module hashes are pinned by the checked-byte importer. Navigation,
folder and caption roles use the acquired known-folder artwork; Downloads navigation
retains the modern downward-arrow glyph, independently of its folder artwork. Home captions
use the acquired Explorer Home icon. Other file associations remain separate work.

Top Snap layout chooser, configurable Snap behavior, Search presentation,
fractional scale, modern Explorer headers/columns/selection, shared hover
transitions, durable Recycle Bin and the wider app/provider inventory remain open.
Owner data is never a fixture. External D: remains unconfigured; the 20 GB cap
continues to apply only to out.

The qualifying clean4K sample is23532:136scene drag paints,141compose+65transfer
ticks at10ms,15.14706ms mean elapsed scope,30ms maximum,zero failures,
2,441,014,632accepted bytes/246,534calls. These are elapsed work scopes, not FPS,
pure CPU time or input latency. Exploratory26628 may overlap an evidence audit;
failed20972 and9464 are preserved unqualified. Only Window/Workspace change among
21actual artifacts; all five repeat pairs match. Current cumulative verify stops
before tests at the Legacy8GB reservation; check-spec still needs the original
missing P00 report32560. The latest owner direction starts Settings Advanced;
see [unified application concepts](unified-apps-owner-concepts.md).
