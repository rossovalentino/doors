# P14: complete Settings and native customization

Owner request (31 October 2026):

* Implement the advanced Settings pages proposed in
  `settings-unification.md`, so Doors behaves like Windows 11 26H2 and
  improves on the legacy behaviour.
* Find out which mods and tweaks people commonly apply to their Windows 11
  installation, and build them into Doors natively.

This document is the plan and the record of what was done. The page
inventory, the route table and the redirect table stay in
`settings-unification.md` and in `frameworks/settings-map`.

## Principles

1. **Windows first.**
   * Where 26H2 has a setting, Doors puts it on the same page, with the same
     name, control and default.
   * A Doors extra (a feature that comes from a popular mod) sits where a
     Windows user would look for it: on the page whose subject it changes,
     at the end of that page, inside a collapsed expander named after the
     subject (for example "More taskbar options").
   * Every default matches clean Windows. Nothing changes until the user
     turns it on.
2. **No dead controls.** A control either works (it persists, takes effect
   at once, and survives sign-out and reboot) or is disabled with its
   reason, as today. This work turns every software-only "Unavailable"
   reason into a working control. The reasons that remain are hardware ones
   (audio, network, Bluetooth, camera, touch, pen; P15 and later) or
   genuinely not applicable ones (domains, the Store, online accounts).
3. **Search finds it.** Every new row gets `settings-map` keywords,
   including the mod's common name ("TranslucentTB", "taskbar on top",
   "FancyZones", "Auto Dark Mode", "QuickLook", "PowerToys Run"). Any legacy
   command or registry tweak name that users look for resolves to the row.
4. **One store.** User settings are typed registry values under
   `HKCU\Software\Doors\…` (machine settings under HKLM). Each has a
   validator and a default, as with `Setting::registry`. Because the store
   currently holds only 24 keys and 24 values per volume, enlarging it is
   the first task.
5. **Live.** A change reaches its consumer through the existing
   APPEARANCE_CHANGED / settings-change notifications, without a restart,
   the way Windows applies settings.

## Research: what people change on Windows 11

The sources were Win11Debloat's option list, PCWorld's "18 essential
Windows 11 tweaks", Windhawk's most-installed mods (Taskbar Styler, Start
Menu Styler, Taskbar height and icon size, Taskbar on top or vertical,
Taskbar labels, Multirow taskbar, Taskbar clock customization, Better file
sizes in Explorer details, Middle-click to close), ExplorerPatcher and
StartAllBack, TranslucentTB, Auto Dark Mode, QuickLook, and the PowerToys
utilities (FancyZones, PowerToys Run, Peek, Always on Top, Keyboard
Manager, Color Picker, PowerRename, Find My Mouse, Shortcut Guide).

| Area | Common tweak or mod | Doors |
| --- | --- | --- |
| Taskbar | Left alignment; hide Search, Task view and Widgets | 26H2 rows; check that each takes effect |
| Taskbar | Never combine, show labels (StartAllBack, Taskbar Labels) | 26H2 "Combine taskbar buttons and hide labels": Always / When full / Never |
| Taskbar | Seconds and custom format in the clock | 26H2 "Show seconds"; extra: clock format (date, weekday, 24-hour) |
| Taskbar | Taskbar on top, left or right (Windhawk, ExplorerPatcher) | Extra: Taskbar position |
| Taskbar | Smaller or larger taskbar (Taskbar height and icon size) | Extra: Taskbar size Small / Default / Large |
| Taskbar | Transparent or clear taskbar (TranslucentTB) | Extra: Taskbar look Default / Acrylic / Transparent / Opaque |
| Taskbar | Auto-hide | 26H2 row; make it work |
| Taskbar | End task on right-click; middle-click to close; last active click | 26H2 End task (System > Advanced); extras: middle-click closes, last-active click |
| Start | Hide Recommended; more pins; folders next to power | 26H2 layout and section toggles; make "Folders" work |
| Start / Search | No Bing or web results | Doors has no web search; "Show web results" off and disabled, with the reason |
| Explorer | Show extensions and hidden files; open to This PC | 26H2 System > Advanced > File Explorer, plus a Folder options page: open to, hidden and protected files, full path in title, compact view, checkboxes, navigation pane items |
| Explorer | Folder sizes in Details (Better file sizes) | Extra: "Calculate folder sizes" |
| Explorer | Space to preview (QuickLook, Peek) | Extra: Quick look on Space, using the preview pane renderers |
| Explorer | Bulk rename (PowerRename) | Extra: Rename several items with find/replace and numbering |
| Context menu | Classic menu; Copy as path; Open in Terminal | 26H2 already has the classic toggle; add both items |
| Appearance | Dark mode on a schedule (Auto Dark Mode) | Extra on Colors: Mode Light / Dark / Custom / Scheduled (fixed times) |
| Appearance | Accent on title bars and Start/taskbar | 26H2 rows; wire up the compositor's SET_ACCENT and a 48-swatch picker |
| Appearance | No rounded corners; Mica everywhere (MicaForEveryone) | Extras: Window corners Rounded / Square; Mica in all apps |
| Appearance | Desktop icons (This PC, Recycle Bin, user folder), size, arrangement | Desktop icon settings and the desktop View menu (Small / Medium / Large, Auto arrange, Align to grid) |
| Appearance | Wallpaper fit, slideshow, Spotlight-style rotation | 26H2 Background: Picture / Solid color / Slideshow with fit modes |
| Motion | Turn off animations; faster menus | 26H2 Animation effects; extra: Animation speed Fast / Normal / Slow |
| Multitasking | Turn off snap, Snap Assist and shake | 26H2 Multitasking rows, including Title bar window shake |
| Multitasking | Custom zones (FancyZones) | Extra: Custom snap layouts (presets plus a grid editor); Shift-drag shows the zones |
| Multitasking | Always on top (PowerToys) | Extra: Win+Ctrl+T, with an accent border |
| Input | Turn off mouse acceleration; pointer speed; scroll inactive windows; swap buttons | 26H2 Mouse page; make it real in the window server |
| Input | Key repeat delay and rate | Keyboard page (legacy `control keyboard`) |
| Input | Remap keys and shortcuts (Keyboard Manager) | Extra: Keyboard > Remap keys and shortcuts |
| Input | Turn off the Sticky Keys shortcut | Accessibility > Keyboard: Sticky / Filter / Toggle keys and their shortcuts |
| Input | Clipboard history (Win+V) | 26H2 System > Clipboard; Win+V flyout |
| Input | Quick launcher (PowerToys Run, Alt+Space) | Extra: Alt+Space opens a compact Start search |
| Tools | Color Picker; Find My Mouse; Shortcut Guide | Extras: Win+Shift+C picks a colour; press Ctrl twice to find the pointer; hold Win for the shortcut guide |
| Tools | Screen capture (Print Screen) | 26H2 "Use the Print screen key to open screen capture": region, window or full screen, saved to Pictures\Screenshots |
| Accessibility | Night light on a schedule; colour filters; magnifier; cursor size and colour; text cursor indicator | 26H2 rows; compositor transforms and pointer rendering |
| Privacy | Telemetry, ads, tips and suggestions off (Win11Debloat) | Doors collects nothing. The 26H2 rows exist, default off, each with an InfoBar saying Doors sends nothing |
| Startup | Turn off startup apps | 26H2 Apps > Startup, driven by the scheduler's at-logon tasks |
| Maintenance | Storage Sense, cleanup recommendations | 26H2 Storage: temporary files, Recycle Bin older than N days, Downloads; real sizes |

## Work, in order (one agent per batch)

### Batch 1: the settings store and the software providers

* **Registry store.** Raise the capacity to at least 512 keys and 2048
  values per volume, with values up to 2 KiB, while keeping the persistence
  format versioned and migrating old volumes. Add delete and rename for
  keys and values, and `.reg` export/import in Registry Editor. Fix the
  "The registry is full" limit.
* **Environment variables.** System variables (HKLM, administrators), delete
  a variable, and a PATH list editor (one entry per row: Add, Edit, Move
  up/down, Delete), as Windows' editor works.
* **Colors.** Accent picker with Automatic, Manual and the 48 Windows
  swatches; "Show accent colour on Start and taskbar" and "on title bars
  and window borders"; and a dark-mode schedule.
* **Notifications.** Per-app senders (banners, sound, Notification Center,
  priority), Do not disturb with a schedule, Focus, and tips and
  suggestions (default off). The notifications service backs all of it.
* **Default apps.** Per-type and per-app associations from the registration
  database, used by Explorer's Open and Open with.
* **Sign-in options.** Change password, change PIN, add or remove a PIN,
  "require sign-in after the display turns off", and the lock screen
  (Win+L, background, idle lock).
* **About and Time & language.**
  * Rename this PC: one device name, shown in About and in Explorer's
    This PC.
  * Time zone, 24-hour time, first day of week and custom short or long
    date formats, used by the clock and Explorer.
* **Clipboard.** History of 25 items (text and images) with Win+V, pinning
  and Clear.
* **Start search history.** Toggle and Clear.
* **End task in the taskbar.** Enabled through the shell.

### Batch 2: taskbar, Start, desktop and appearance

* **Taskbar.**
  * Position (bottom, top, left or right) and size (small, default or
    large).
  * Combine buttons and labels: Always, When full or Never.
  * Look: default, acrylic, transparent or opaque.
  * Auto-hide; seconds and clock format; show or hide the Search, Task view
    and Widgets buttons.
  * Middle-click to close; last-active click; the Show desktop corner.
* **Start.**
  * Folders next to power: Settings, File Explorer, Documents, Downloads,
    Music, Pictures, Videos, Network and Personal folder.
  * Every layout and section toggle works.
  * "Show web results" is disabled, with its reason.
* **Desktop.**
  * Icons: This PC, Recycle Bin, user folder and Settings (Control Panel's
    successor). The View menu offers icon size, Auto arrange and Align to
    grid.
  * Background: Picture, Solid colour and Slideshow; fit modes Fill, Fit,
    Stretch, Tile, Centre and Span; slideshow interval and shuffle.
* **Windows.**
  * Corner style: rounded or square.
  * Mica in all apps; every Doors app opts in.
  * Animation speed: fast, normal or slow.

### Batch 3: window management, input and accessibility tools

* **Multitasking.** Every 26H2 row, Title bar window shake, custom snap
  layouts with Shift-drag zones, and Always on top.
* **Mouse.** Pointer speed, enhanced precision (off means no acceleration),
  scroll lines, scroll inactive windows, primary button and double-click
  time.
* **Keyboard.** Repeat delay and rate, key and shortcut remapping, and
  Sticky / Filter / Toggle keys with their shortcuts.
* **Accessibility.** Night light with a schedule and strength; colour
  filters; Magnifier (Win+Plus and Win+Esc); pointer size and colour; text
  cursor indicator and thickness.
* **Tools.** Find my mouse, Color Picker, Shortcut guide, Alt+Space quick
  launcher, and Print Screen capture.

### Batch 4: system services and File Explorer

* **Task scheduler service.**
  * Triggers: at logon, at a time, daily, weekly and on idle.
  * Actions: launch an app or a command, persisted.
  * Settings > Scheduled tasks can create, edit, delete, enable, disable
    and Run now, and shows the last result.
  * Apps > Startup runs on its at-logon tasks.
* **Event log service.**
  * Logs: System, Application and Security.
  * Producers: Init (service start, stop and failure), the session
    (sign-in success and failure, lock, unlock), app launch and crash,
    Settings changes, and storage errors.
  * Settings > Event logs adds level, source and date filters, details and
    Clear log.
* **Services.** Restart and stop for the services Init supervises and can
  restart; a disabled control, with its reason, for critical ones.
* **Storage.** Real category sizes, cleanup recommendations, Storage Sense
  (temporary files, Recycle Bin older than 1, 14, 30 or 60 days, Downloads
  never or older than N days) with a schedule through the scheduler.
* **File Explorer.**
  * A Folder options page (System > Advanced > File Explorer > Folder
    options) with every View option of `control folders` that Doors can
    honour.
  * Calculate folder sizes; Quick look on Space; bulk rename.
  * Copy as path and Open in Terminal in the context menu.

## Testing

* Each batch: host tests for every new setting (the default, the validator,
  persistence, the live effect on its consumer's model) and for every new
  service. Workspace clippy (`-D warnings`) and fmt. Settings page layout
  tests at the seven widths in both themes. `settings-map` keyword tests.
* Guest: after each batch the faster harness runs the Settings, shell and
  Explorer scenarios. Each batch extends `p14-settings` with a
  persist-across-reboot check for a sample of its new settings, with probe
  points taken from the layout functions.

## Status

### Batch 1: done; guest runs pass (see "Guest integration")

* **Registry store.**
  * The catalog keeps its 24 key and 24 value slots. Every later record
    lives in the hive `door.registry`, written in the catalog's journal
    transaction only when those records change (`codec/hive.rs`).
  * Capacity is 128 keys and 384 values per volume. Values stay at most
    64 bytes, the wire limit (`MAX_PAYLOAD` 256).
  * Old volumes need no migration: no hive means no records past the
    catalog slots. A damaged hive refuses the mount.
  * The independent disk auditors (`tools/xtask/src/persistence/catalog.rs`
    and the desktop scenarios) read and check the hive.
* **Registry Editor.**
  * Storage service operations REGISTRY_DELETE_VALUE and
    REGISTRY_DELETE_KEY.
  * The value Edit dialog has Save, Delete and Cancel, and renames a value
    when its name changes.
  * "Delete this key" asks for confirmation; a key with subkeys stays.
  * "Export this key" writes a Windows `.reg` file (version 5.00, four
    levels deep) to Documents.
* **Environment variables.** The Edit dialog has Delete.
* **Colors.**
  * Accent color: Manual or Automatic, plus the 48 Windows colors as
    swatches.
  * Accent on Start and taskbar: tints the taskbar material with the
    accent's Dark2, in dark mode only.
  * Accent on title bars and window borders: the engine's `title_accent`.
  * More color options (Doors extra): switch modes on a schedule, whole
    hours, applied by the window server's schedule tick.
* **Notifications.**
  * The global switch, Do not disturb, the automatic DND schedule (from,
    to, repeats), Focus (length, hide badges, hide flashing, DND), sounds,
    and the three tips switches (default off).
  * The shell applies each sender's stored flags (banners, centre,
    priority) and creates a sender's registry entry the first time it
    notifies.
  * The Focus footer's timer and its start, stop and length actions work.
* **Default apps** (new page 53, Apps > Default apps).
  * Every file type with its current default and a Choose menu, backed
    by `default_apps`. The same model drives Explorer's Open with.
  * The window server accepts OPEN_DOCUMENT from desktop-controls
    clients, so Explorer can hand files to Notes and Preview.
* **Sign-in options** (new page 51).
  * Change PIN, set up a PIN (from the password) and change password, in
    masked ContentDialogs. The secrets go to the session's Argon2id store
    and are verified there; throttling is reported.
  * "If you've been away" locks after the chosen idle minutes.
* **Lock screen** (new page 52): Same as desktop background, or one of
    the four Doors pictures, rendered once per lock.
* **Rename this PC.** The About card shows the stored name. The dialog
  validates like Windows and stores the name through Init.
* **Regional format.** First day of week and 12- or 24-hour clock (the
  taskbar clock follows the overrides).
* **Clipboard** (new page 50): history switch, and Clear, which empties the
  history and keeps pinned items.
* **Search history.** The switch is on by default; turning it off clears
  Start's history.
* **End task** (System > Advanced): adds "End task" to taskbar jump lists;
  it retires the app's processes through Init.

### Batch 2: done on the host; guest smoke run passed

Guest smoke run (desktop image built from this tree): `p14-settings` and
`settings-reference` pass. `explorer` failed on stale Tags points; fixed in
the guest-integration step below.

* **New preference words.**
  * 21 `TASKBAR`, 22 `START_FOLDERS`, 23 `BACKGROUND`, 24 `WINDOWS`.
  * The table, Init's binding and persist slots, the client `Setting` ids
    and the window server's `Words` all follow `preferences::ALL` and
    `LAST`.
* **Taskbar.**
  * Combine buttons and hide labels: Always, When taskbar is full, or
    Never. Labelled buttons are 160 px wide and show the app's name;
    windows stay grouped per app.
  * Automatically hide the taskbar: the work area takes the whole screen.
    The bar moves past its edge until the pointer reaches the 2 px edge,
    and hides again when the pointer leaves it (with Start, flyouts and
    menus closed).
  * Show seconds: the long time format, repainted every second.
  * Task view on or off (the layout drops its slot).
  * The far-corner show-desktop target, and attention flashing, each on
    or off.
  * Badges: the switch is stored, but no badge source exists yet.
  * More taskbar options (Doors extras):
    * Look: default acrylic, translucent, opaque or clear (material
      weights).
    * Middle-click closes an app's windows.
    * Last active: a click on an active group minimizes it instead of
      cycling.
* **Start.** Folders next to Power: Settings, File Explorer, Documents,
  Downloads, Music, Pictures, Videos, Network, and the personal folder.
  New 16 px glyphs are exported, and each folder opens in Explorer or
  Settings.
* **Background.**
  * Picture, Solid color (eight colours) or Slideshow, which rotates the
    four Doors pictures every 1, 10, 30 or 60 minutes, optionally
    shuffled.
  * Fit: Fill, Fit, Stretch, Tile, Center or Span (Span is Fill on one
    display), in `wallpaper::load_fitted`.
* **Window style** (Doors extras): square corners (the engine's radius),
  Mica in every app (`Engine::set_mica_everywhere`), and animation speed:
  fast, normal or slow (`set_animation_speed`).
* **Tools.** `out/latest-*` metadata may reach 128 MiB, the same bound
  the readers accept.

### Batch 3: done on the host

* **New preference words.** 25 `MOUSE`, 26 `MULTITASK`, 27 `VISION`.
* **New Settings pages.** 54 System > Multitasking, 55 Bluetooth & devices
  > Mouse, 56 Accessibility > Color filters. `main.cpl`,
  `control mouse`, `ms-settings:mousetouchpad`, `ms-settings:multitasking`
  and `ms-settings:easeofaccess-colorfilter` now open them.
* **Mouse.**
  * Primary button: Left or Right (button swap).
  * Pointer speed 1 to 20, where 10 is 1:1. Sub-pixel remainders are
    carried, so slow speeds still move.
  * Enhance pointer precision (acceleration). It starts off in Doors,
    because the VM pointer already carries the host's acceleration;
    Windows starts with it on.
  * Lines to scroll, 1 to 20 (apps scroll three lines per notch; thirds
    are carried).
  * Scroll inactive windows: when off, only the focused window scrolls.
* **Multitasking.**
  * Snap windows. Turning it off stops edge snapping, Windows logo key +
    arrows, the drag-to-top layouts and the maximize hover.
  * The three snap sub-options: hover layouts, drag-to-top, Snap Assist.
  * Title bar window shake: four quick turns of at least 24 px within a
    second minimize every other window of the desktop.
* **Night light and color filters.**
  * Night light: Display > Night light (an expander with its strength).
  * Color filters: grayscale, inverted, grayscale inverted, and the
    deuteranopia, protanopia and tritanopia filters.
  * Both are an affine RGB transform applied to every span the scanout
    copies (`scanout::vision_matrix`). The scene stays untransformed, and
    nothing is computed while both are off.
* **Guest key paths.** Display > Scale is now three Tabs away (past the
  Night light expander and its switch), in `display_scale.rs` and the host
  test.

### Guest integration (31 October 2026)

Guest runs on images built from this tree: `p14-settings`,
`settings-reference`, `desktop-wide`, `display-01`, `explorer`,
`desktop-suite`, `desktop-remove` and `sec-02-trusted` pass. Fixes:

* **The desktop shell was never in the image.** `door-desktop-shell` was
  missing from `profiles/build/initfs-desktop.json` and
  `initfs-desktop-test.json`, so Init's launch of manifest 19 failed with
  NotFound (`SEARCH_HOST FAILED`) in every guest run: no Start recents or
  file search, no notification senders. Both profiles now ship it.
* **Unlock re-locked.** Keys typed while locked go to the credential gate,
  so the Win/Alt shortcut tracker never saw the release of Win+L and the
  next "l" locked again. `gate_unlock` now resets it (host test in
  `policy/shortcuts.rs`).
* **Explorer Inspector points.** The Tags tab and Create tag field points
  come from the layout and are pinned by
  `files::scenario_points::modern_inspector_tag_points_follow_the_layout`.
* **Permanent delete through the P14 engine.** Explorer emits
  `DOOR_P12 FILES REMOVED op= items=` when a permanent delete finishes.
  The desktop-remove auditor accepts the engine's own records
  (`/state/machine/fileops`, created on first use, and records allocated
  during the operation); every other catalog byte, backing file and
  allocation stays exact. Non-empty folders now delete recursively, as in
  Windows (qualified by `p14-files`), so that P12 case holds Delete and
  presses Esc instead: no write, capture cancelled, release ignored.
* **Probes.** Notes' command bar is composited over the position-dependent
  Mica backdrop, so `NOTES_BAR` matches within 8 per channel (white content
  still fails). `SCENE_WALLPAPER` is pinned to the wallpaper decoder.
* **SEC-02 timing.** Guest time runs behind the host under parallel TCG
  load: the final unlock retries while the gate reports a remaining delay
  (bounded; an early unlock still fails), and the post-unlock save reads
  its serial offset before Ctrl+S.
* **Dependency lock.** `specs/sources/dependencies.json` records the new
  lock (`p14_customization_lock_20261031`): four local crates and local
  edges, no registry change.

### Owner UI pass (1 November 2026)

* **Taskbar.** The Task Manager (Monitor) tile now fills 22 of 24 units
  with one unit of padding, like the other app icons (`monitor.svg`,
  re-exported; `preview-monitor.source.json` hashes updated).
* **Explorer navigation pane.**
  * Icons are centred in the 30 px row, on the label's cap-band centre
    (they were placed for a 32 px row, about 2 px low).
  * Its scroll bar shows only while the pointer is over the pane (or it is
    being dragged), as in Windows, so it no longer covers the pins.
  * The pane width is draggable from its right edge (east-west pointer,
    `window::CURSOR_SIZE_WE`): every band, label and column reflows live,
    labels ellipsize, and the width is clamped to 96 px .. min(600, window
    - 320) and saved in `.door-explorer-views` (`sidebar N`). The width lives
    in `ExplorerLayout::sidebar`; `SIDEBAR` is the default.
* **Folder sizes (Doors extra).** Windows leaves Details' Size column blank
  for folders. View > Show > Folder sizes (saved, off by default) fills it
  from the recursive size scan the details pane already uses: the inspected
  folder first, then the visible folder rows, one bounded job at a time
  ("> size" when a listing was capped). F5 clears the remembered totals.
* **Window resizing.** Each side resizes alone and corners resize both axes
  (Windows corner resizing is free, not aspect-locked). Normal framed
  windows now also take a 7 px invisible band outside the frame and 16 px
  corner zones along each edge, as Windows 11 does
  (`CursorIcon::RESIZE_OUTSIDE` / `RESIZE_CORNER`).

Not done: a tooltip with the full name of an ellipsized pane label, and the
rest of View > Show (navigation, preview pane, check boxes, extensions,
hidden items; Batch 4).

Left from Batch 3:

* A night light schedule.
* Pointer size and colour, and a text cursor indicator.
* Magnifier.
* Sticky, Filter and Toggle keys, and key repeat (PS/2 typematic comes
  from the device).
* Key remapping.
* Custom snap zones, and Always on top.
* Find my mouse, Color Picker, Shortcut guide, the Alt+Space launcher and
  Print Screen capture.

Left from Batch 2:

* Never-combine as one button per window.
* Desktop icon settings (This PC, user folder) and the View menu's icon
  sizes.
* A badge source.

Left from Batch 1:

* `.reg` import.
* A PATH list editor, and system (HKLM) variables.
* The per-app sender list on the Notifications page (the shell already
  honours the flags).
* Custom date patterns, and a custom accent colour dialog.
* Values over 64 bytes: they need chunked registry operations.
