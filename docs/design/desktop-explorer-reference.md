# Owner desktop and Explorer reference

Historical reference: the owner supplied
`docs/references/windows11-desktop-explorer.png` on 2026-10-18. The owner's
2026-10-21 direction supersedes it with the installed Windows11 reference in
`windows-core-apps.md`. Modern tabs and navigation-before-command-bar are the
current target; this older geometry is retained for existing regression evidence.
It is a 1470x834 reference image with a Neowin watermark, a visible 9/19/2021
clock date and an untabbed Explorer window. Exact Windows build, DPI, font
rasterizer settings and app package identity are unknown. The image is retained
as a reference; its watermarks and browser/gallery affordances are not OS assets.

The historical October20 comparison reviewed toolbar/status/icons against
this untabbed baseline. It does not override the October21 installed reference.
Microsoft's [geometry reference](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/geometry)
specifies an 8-DIP top-level radius. The renderer keeps that radius and corrects
the continuous one-DIP border along both straight edges and antialiased corners.
The outline is weighted between inner/outer contours before applying outer alpha
once. Caption hover and partial composition share the same contour calculation.
Maximized windows retain square corners. Shadow work skips opaque interior spans
without changing the established shadow profile or traversing the entire body.

Files caption artwork follows its current location (This PC, common profile
folder, compatibility drive or ordinary folder) using original offline16/32px
frames. Address breadcrumbs use actual Regular12 font metrics for both drawing
and hit testing, avoiding the old clipped Home/Documents labels.

Behavior reference: Microsoft's [Explorer navigation guide](https://support.microsoft.com/en-us/accessibility/windows/use-a-screen-reader-to-explore-and-navigate-file-explorer-in-windows)
describes the command bar, address/breadcrumb navigation and bottom-right layout
buttons. Each exposed action must operate on the real current directory/selection;
unsupported clipboard/share/device operations remain explicitly disabled.

Measured logical geometry at the image's apparent 96-DPI scale:

| Element | Reference geometry | Current development implementation |
|---|---|---|
| Explorer outer frame | Approximately742x530, 8px corners |742x530 initial frame; actual move/resize and caption controls |
| Caption |32px; small app icon and12px label |32px, real color icon, local Segoe UI12 |
| Command row |48px, above address/navigation row | New; disabled edit commands; functional Sort/View/More |
| Navigation/address row |48px; address starts at132px | Shared paint/hit rectangles, back/forward/up and refresh |
| Address/search at742px width |262px address,10px gap,328px search | Same dimensions; Ctrl+L address editing, Ctrl+F/F3 search |
| Navigation pane |150px,24px item rows | Real standard folders, Notes, Applications and This PC |
| Folder group | Two columns, approximately244px pitch,54px row pitch | Six actual persistent folders with40px Windows folder artwork |
| Status strip | Approximately24px | Actual item/result counts and operation feedback |
| Measured light colors | CaptionRGB239/244/248; commandRGB240/244/247; selectionRGB204/232/254 | Same opaque colors; composited shadow/acrylic remain approximate |
| Taskbar |48px, centered app buttons | Bootstrap open-window switcher; real activation/restoration and stable IDs |
| Desktop background | Windows Bloom, fill/cover | Pinned clean source, centered cover export for1024x768 guest profile |

All six standard folders are created through authenticated Storage without
overwriting existing objects. Notes continues to use its existing persistent
`notes` directory. This PC links to the real Door data namespace. The image's
three Windows volumes and capacity values are not invented: the current Storage
protocol does not expose free-space accounting, so those bars are still pending.
Rename now uses the checked durable transaction described in `files-mutations.md`;
Cut/copy/paste/share/delete remain disabled. Sort toggles name order, View switches
the home overview/list, and More uses the same real commands as context menus.
Search is bounded ASCII matching within the listed directory, not recursive
Windows Search. An empty result has no selection and cannot activate a hidden file.

Local Segoe UI font hashes, Windows ICO/CUR source commits and Bloom identities
are recorded in their asset manifests and the guest notices. Selawik remains the
default reusable text helper font; the owner reference image explicitly selects
the local Windows fonts. Printable ASCII is still the text cache boundary.
Reference typography converts12 logical pixels (9pt at96DPI) through the font's
EM units into the rasterizer's ascent/descent scale. It does not treat12px as the
entire ascent/descent box. Draw and measure share that cached scale; an independent
FreeType measurement of Segoe UI12px Documents (60.375px) guards the conversion.

The bootstrap Door button opens a bounded switcher for up to three existing
windows. Windows key, arrows/Tab, Enter and Escape operate it. Desktop shortcuts
and taskbar buttons activate existing live windows; they do not claim process
launch, a complete Start menu, installed Chrome/Edge, a system tray or a wall clock.
The trusted48px work area protects the taskbar on maximize. Overlay input is
consumed above client windows, while preexisting client capture/drag is honored.

P12 remains partial. The retained controls/text/IME/clipboard/accessibility gates,
full P14 desktop, resizing/DPI coverage and measured screenshot comparison are
not closed by this visual refinement. A1024x768 software-GOP guest cannot be
called pixel-identical to the supplied1470x834 capture.

## Usability refinement:18 October2026

The owner supplied additional fullscreen/lag reports. Ordinary sessions now keep
an independent desktop lifetime, with three permanent application launch targets.
The current display incarnation submits only catalog command IDs to authenticated
Init; untrusted peers cannot supply an arbitrary launch mode, UID or shutdown.
Init queries its live Session grant through SESSION_QUERY before binding the empty
desktop, rather than launching a dummy identity process. Settings is a separate
protected workspace process/mode and shares the existing Storage settings service.

The original Door SVG uses a rounded gradient tile and open portal, rasterized at
24/32/40px by the existing asset exporter. Sidebar/list icons use16px exports,
pins/chevrons12px, taskbar24px and desktop/folder tiles40px. Microsoft Fluent Add
Circle and Text Bullet List Square replace the generic New/View symbols. The left
Sort arrow uses the reference blue accent. Quick access is an expandable branch;
Home and This PC are distinct destinations. Internal `/system/bin` is no longer
called Applications in the pinned navigation. Home's title/address no longer show
its internal UID when not editing the path. Real storage paths remain unchanged.

Taskbar motion compares visual hover targets. Movement inside an unchanged target
uses the bounded cursor update; target entry/exit redraws the scene. This removes
the unconditional full frame copy on every pointer packet. CPU composition still
has no vblank or native GPU acceleration, and window drags may remain slower than
Windows. This batch does not establish pixel-identical Windows appearance.
# Native-resolution repair status

Current repair awaits guest qualification. Monitor-matched GOP modes through
3840x2160 replace enlarged1280x720 as the visible default. The owner's2560x1600
monitor is16:10, explaining16:9 letterboxing. The3840x2400 wallpaper is stored
losslessly as QOI; a one-time centered cover cache supplies native BGRA pixels.
Settings and Notes use downloaded, pinned WindowsIcons ICO files. Settings uses its repository ICO; category assets follow Microsoft Support and
the original Segoe Fluent font. Original
Door branding uses SVG. All have premultiplied BGRA exports at their display sizes. Text targets
use checked backing extent. Taskbar hover without Start open repaints48 rows.
Logical scaling, multi-monitor and measured Windows fidelity remain pending.

## Application references and offline resources, 19 October 2026

Settings category navigation, neutral backgrounds, rounded cards and heading scale
follow Microsoft's [Settings guide](https://support.microsoft.com/en-us/windows/experience/exploring-windows-settings).
The implemented subset is System, Personalization and About; unavailable Windows
categories and search are not presented as working controls. Segoe UI headings use
a cached28px raster, with14px navigation and16px category icons.
Explorer retains the owner's older untabbed screenshot; current navigation behavior
is compared with [Microsoft's Explorer guide](https://support.microsoft.com/en-us/windows/experience/fileexplorer/file-explorer-in-windows).
Notes references [Microsoft's Notepad release notes](https://learn.microsoft.com/en-us/windows-insider/release-notes/apps/notepad);
the document menu and editor are a limited native implementation, without the full
modern Notepad tab/AI feature set.

All12 WindowsIcons ICO sources are stored in `assets/icons/windows`, pinned by
commit and SHA256 in its manifest. The host exporter verifies these local files,
then derives16/20/24/32/40px premultiplied BGRA from the largest embedded PNG frame.
Guest executables embed their selected exports. Asset loading needs no network,
repository access, API calls or scraping, including after installation.
The3840x2400 wallpaper and font files are likewise local embedded resources.


The expanded Settings layout and exact offline source inventory are documented in
settings-reference.md. The previous three-page Settings layout is superseded.


### Navigation/detail refinement

The reference Back/Forward/history/Up controls now have separate shared hit
rectangles. The history dropdown selects a retained visit without appending or
truncating its forward entries; storage enumeration completes before publishing
the new history position. Quick access collapse moves Home and This PC together
with their hit targets. Folders and Devices and drives headings collapse their
actual contents; hidden group headers/tiles have no active targets. The status
bar reports the current selection. There is one actual Doors data/home namespace;
reference machine volumes and free capacities are not fabricated. The earlier
2021 untabbed owner reference remains the selected Explorer baseline.

The address is now an actual bounded editor with native caret/selection and
clickable breadcrumbs. [Windows path presentation](windows-paths.md) documents
the existing C: compatibility/profile aliases and explicit missing-folder/API
boundaries. Path spelling never grants access or fabricates application support.
