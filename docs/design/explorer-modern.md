# Current audit validation - 22 October 2026

Current common-image two-boot Explorer check passed, including previews, tabs, stable object rename and exact durable tag bytes. The final host fixture strictly validates authenticated per-boot app retirement. See current-app-fidelity-audit-20261022.md for current evidence; historical deleted diagnostics below remain historical only.

# Explorer reference revision, 22 October 2026

Active P12 development; bounded native diagnostic passed, final shared-source validation pending. This supersedes the older untabbed
Explorer geometry. The owner supplied a 1536x1024 concept and subsequently the
installed Windows 11 Explorer, a full This PC breadcrumb and explicit direction
to use HaydenReeve/WindowsIcons. The earlier invented file-type atlas was removed.
Names and sample sizes in host painter fixtures are test data, never guest data.

The native app has location tabs, navigation/address/search above commands, a
190 DIP rail, responsive 272 DIP grid rows, details columns, rounded fields and
selection without checkboxes. The existing window service supplies its actual
8 DIP frame and cached shadows. Tabs are integrated in a 40 DIP client-painted title area; the trusted window
service retains resize, drag and system-button authority. The SDK negotiates the
actual tabbed-caption capability. Only the visible tab/overflow/new-button span
receives app input, with the rest draggable. Eight tabs scroll in narrow windows
without overlapping trusted system buttons. Tab tear-out remains open.
The latest installed-Windows screenshots supersede the first concept's typography
and grid alignment. This is not a pixel-identical Windows claim. Segoe UI 14 DIP is used in the new
Explorer. Breadcrumb labels and targets use the same font measurements and row.
The leading icon is This PC; wide paths show This PC, Local Disk (C:), Users,
the actual authenticated profile, and real directory components. Compact paths
collapse middle components without changing their navigation identities.

Each tab retains location, history, selection by object, search, view, tag filter,
scroll offset and pane mode. Returning to a tab re-enumerates its real directory.
There are at most eight tabs. Ctrl+T, Ctrl+Tab/Shift+Ctrl+Tab and Ctrl+W operate
them; left/right overflow buttons expose clipped tabs; closing the final tab requests the actual window close. Pending rename
must finish or remain visibly rejected before a tab can switch. Single clicks
inspect, timed double clicks open, and Enter opens. The former "click an already
selected item to open" behavior is removed. Back/forward, Alt+Up, Ctrl+L/Alt+D,
search, F2, new folder and checked permanent removal use the existing providers.
Recycle Bin, clipboard transfer, archives and sharing remain unimplemented.

The optional pane opens on selection, Properties/Alt+Enter or its toolbar button.
Alt+P selects preview; Alt+Shift+P selects details. It shows actual type/size,
bounded direct children, user tags and available actions. Storage does not have
creation/modification timestamps or a recursive directory-size provider; dates
are explicitly not recorded and folder byte totals are not invented. The pane and user-tag rail
have real bounded scrollbars and pointer capture for dragging; physical mouse
wheel input is not supplied by the current native input contract. Very narrow
clients below 560 DIP hide the pane; its state survives enlargement.

PNG/JPEG/QOI images and UTF-8 text/code are read-only previews. Input is capped at
1 MiB for images, 16 KiB for text, 1 megapixel for decoded images, 4096 per axis,
24 progressive JPEG scans, 128 direct children and eight preview results. Image
data is downsampled once to a 480x320 cache with premultiplied alpha. PNG chunks
are bounds-checked before decoder allocation; unnecessary ancillary/animation
chunks are excluded. Decoder defaults/intrinsics are disabled and limits are
explicit. PDF/Office/media/archive/3D rendering requires providers not present in
this OS; real format icons do not imply those applications or engines exist.

One Storage read or enumeration step runs per event-loop iteration, outside
painting, after queued input. New selections close prior handles. Reads compare
opened identity/version before and after capture and verify the path still
binds that object. Errors and oversized files are cached rather than retried
continuously. Only visible image tiles request background thumbnails; an idle
completed scan does no provider work. Preview decoding is still bounded CPU work
in the app process, not a sandboxed decoder worker or guaranteed latency budget.

Tags have a real persistent catalog at the authenticated user's
`.door-explorer-tags`. It starts empty. Users create up to eight names (24 ASCII
characters each) and assign/remove them from up to 128 stable object IDs. The
side rail filters the current directory using those saved bindings. Rename does
not change tag identity. The versioned binary catalog is bounded to 4096 bytes;
malformed/future data is refused, not silently reset. Each explicit change
re-reads the opened catalog, validates it, and uses `replace_checked` for one
durable payload/length/version transaction. Concurrent changes are rejected by
the provider. There is no invented Work/University/Important/Travel catalog.
A valid internal metadata file is excluded from ordinary Explorer listings. A
malformed catalog stays visible for repair, produces an explicit tag error, and
never prevents ordinary browsing. Refresh clears obsolete tag filters and clamps
rail scroll after catalog changes. The rail shows a tag section only after users
have saved names. Searching rebinds or clears inspection immediately.

Color file icons are downloaded from the repository at the existing pinned
commit, with source URL and SHA256 in `assets/icons/windows/manifest.json`.
PDF artwork is the installed Edge `IDR_X007_PDF_DOC` resource identified by the
host's actual PDF association; the repository has no PDF icon. Acquisition reads
PE bytes without loading code and records module/group identity. The authored
monochrome 3D Viewer logo uses Windows foreground tint in light mode. Commands
use the acquired Segoe Fluent Icons font and its manifest, including Open,
NewTab, Tag, Close, Preview and the details pane. No guest network/SVG parsing is
needed. High-density exports use lossless QOI and the existing bounded asset
cache to preserve the 16 MiB native application admission limit. Large heroes
select the existing 240-pixel source; they do not enlarge 80-pixel sources at1x.
The fixed 32-entry decode cache survives mixed-format redraws without repeated
decoding. The eight-image preview cache and sixteen-directory icon cache are
separate, and navigation invalidates stale entries. Each directory preview reads
at most two children and validates final directory identity/version before using
the original liveback/livefront Windows folder layers. No file payload is invented.

At2048 logical pixels, the tested geometry has a190-pixel rail,360-pixel details
pane, six grid columns, centered208-pixel heroes and centered file names. The
full-width56-pixel command bar follows navigation, with rounded38-pixel address
and search fields. Actual Segoe Fluent command geometry uses neutral/blue roles.
The dark content25/25/25, header34/34/34 and address39/45/55 RGB roles are sampled
from the supplied installed Explorer. Native-density glyph/selection baselines
are checked at1x/2x/3x in both themes. Fractional scale and Windows LCD subpixel
raster parity are not implemented. Current native text caches cover ASCII, so
non-ASCII display uses explicit replacement; lossless UTF-16 file identity is
preserved and uneditable names cannot accidentally bind another filename.

New folder, Rename, checked Delete, Sort, View and Details are service-backed.
Cut, Copy, Paste and Share retain visibly disabled command states until clipboard,
checked cross-directory transfer and sharing providers exist. They do not claim
successful work. The owner explicitly requested those Windows controls to be
visible; complete Windows backend parity remains outside the current providers.

Dependencies added for real image decoding: zune-jpeg 0.5.15, zune-png 0.5.2,
zune-core 0.5.3 and zune-inflate 0.2.54; default features disabled. Exact package
checksums are in Cargo.lock. Existing QOI remains pinned at 0.4.1.

Validation must include current host tests, native compilation, asset checks,
private-volume GUI interaction, two-boot tag/rename persistence, independent raw
disk inspection and a read-only critic. Historical desktop pixel probes describe
the old geometry and are not rewritten to pretend the new layout passed them.
Full cumulative verification remains subject to its existing historical evidence
and output-admission requirements. Owner storage is never a regression fixture.


The first two-boot diagnostic was observed at
`reports/runs/host-1792606069303592400-11016/report.json`: real text preview,
user-created tag/assignment, checked rename, isolated tab state, restart and exact
independent raw-disk payload/tag audit passed. Stable object26 survived rename
and reboot, and both shutdowns reported leaked=0. The aggregate report deliberately
fails source stability because concurrent Settings/Terminal documentation and
Settings source changed during the run. Explorer guest sources were unchanged.
The owner subsequently confirmed deleting reports/runs during cleanup, so that
diagnostic report/screenshots/raw volume are unavailable and are not reconstructed.
Host verification passed757 tests across36 executables (target/explorer-all-host.log).
The image is a development relink, not a cumulative phase qualification. Compact
current native evidence is being regenerated. Below360DIP the app suppresses tab
controls to preserve the actual160DIP admitted window's trusted system buttons;
Ctrl+Tab/Ctrl+W retain keyboard access. The last bounds regression covers160through1000DIP.
The final coordinated run and critic record must retain these distinctions.
