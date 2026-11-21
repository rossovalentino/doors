# GUI, Explorer and software-compositor refresh (2026-10-22)

Working record for the owner's Windows 11 fidelity/responsiveness request
(Preview navigation, Explorer navigation/views/details, Settings headings,
Start/taskbar, CPU compositor, terminal backdrop, motion, typography, icons,
logo). It separates what is implemented, what is host-tested, and what still
needs native (QEMU) validation. Host tests cannot establish guest visual or
timing claims.

Pre-change source snapshot (for diffs): the session scratchpad
`source-before.tgz` taken 2026-10-22 19:16 local. The repository has no
commits; compare against that archive.

Concurrent work: `desktop/workspace/src/runtime.rs` (`InputDrain`),
`graphics/window-server/src/policy.rs` (`events_per_owner: 256`),
`tools/xtask/src/qmp.rs` and `tools/xtask/src/evidence.rs` changed after the
snapshot outside this workstream. They are preserved as-is.

## Architecture notes (confirmed by reading the code)

- Apps are separate native processes built on `door-workspace`
  (`runtime.rs` event loop, `ui::View` painter, `fluent` controls). Every dirty
  frame clears and repaints the whole window, then copies it to the shared
  section; the window server copies again on COMMIT and per-pixel
  validates/diffs it (`authority.rs`, `graphics-core/src/surface.rs`).
- The window server (`graphics/window-server`) composes on the CPU
  (`graphics-core/src/render.rs`) and scans out in 16 KiB `display_write`
  spans to the GOP framebuffer. Input is polled PS/2 on timer ticks.
- Hover repaint is triggered only when the target id under the pointer
  changes (`runtime.rs`), so hit-test identity is the hover identity.

## Status

| Area | State | Evidence |
|---|---|---|
| Explorer navigation state (A→B stale address/details) | Implemented: single `navigate`/`push_history` path, This PC is a history location (`THIS_PC`), sidebar drive row is its own destination id 501, `install` bumps `nav_generation` and resets details scroll/sticky click, failed history steps keep the current location | `cargo test -p door-workspace` (navigation tests in `files`/`files/modern.rs`) |
| Preview navigation pane | Implemented on shared `nav_pane` + `fluent::nav_row/nav_header/nav_separator/pane_thumb`: one layout for paint and hit test, real Library/Tools disclosures (ids 424/425), pixel scroll with draggable thumb (426), pinned Settings, selection reveal, collapsed-group selection marker, compact rail shows all destinations with separators. Removed the stray scroll chevrons (old ids 420/421). | `cargo test -p door-preview` (3 nav tests), `cargo test -p door-workspace nav_pane` (3), review captures `target/preview-review/nav-*.ppm` |
| Explorer views | Implemented: `explorer_layout::ViewMode` (Extra large/Large/Medium/Small icons, List, Details, Tiles, Content) with per-mode flow (grid, column-major List, rows), step-aligned scrolling, flow-aware arrow keys, Ctrl+Shift+1..8, 8-entry View menu, label/rename geometry per mode; selection and details survive view changes; default Details for folders, Tiles for This PC. Ctrl+wheel not wired: the guest has no wheel input. | `explorer_layout` tests (6), `view_review::every_view_mode_paints_and_hit_tests_its_items`, captures `target/explorer-review/view-*.ppm` |
| Explorer sorting / Details columns | Implemented: `explorer_sort` (folders first, Name/Type/Size, case-insensitive natural names); Sort menu Name/Type/Size/Ascending/Descending; Details header click sorts/reverses with caret, column edges drag-resize (48–640 px); sort and widths are per tab, inherited by new tabs. No Date columns: the ext2 volume stores no timestamps. | `explorer_sort` tests (2), view_review |
| This PC / volumes | Implemented: This PC lists the mounted-volume model (`DRIVES`) as "Devices and drives" with a capacity bar and "X free of Y" from a new `VOLUME_USAGE` storage opcode (ext2 free-block accounting via `Vfs::usage`); query failure shows "Capacity unavailable". Known folders no longer appear on This PC. | host + guest `cargo check` of storage service; view_review `this-pc-*.ppm` |
| Navigation tree | Implemented: `explorer_tree` (pins, separator, This PC > drives, root drive alias, Network), stable hierarchy keys, lazy subfolder listing replaced (never appended) on expand, chevrons toggle without navigating (ids 1400+), rows open locations (1100+), clicked alias stays highlighted, Network reports no provider. | `explorer_tree` tests (3), `tree_rows_open_locations_and_chevrons_only_toggle`, `aliased_drive_highlights_...`, capture `tree-expanded.ppm` |
| Explorer details pane | Implemented (`files/details.rs`): one measured layout for paint, hit test and scrolling. Selection shows Type (+extension), Size ("1.2 KB (1,234 bytes)"), Location, image Dimensions (decoded preview or a bounded header scan: PNG/JPEG SOFn/BMP/QOI/GIF/PPM/WebP, `explorer_preview::dimensions`), text Lines (preview bytes, or a background count up to 8 MiB), Object ID, Version; folders add async recursive Size/Contains and direct item count; drives show Capacity/Used/Free from `VOLUME_USAGE` with a bar; no selection shows the open folder (This PC shows its drive summary); several selected items show counts and total file size. Modified/Created read "Not recorded by this volume". Tag chips (color dot, x to remove, "Add tag" inline editor), quick actions: Open in new tab, Open in Terminal (home folder only — Terminal cannot start elsewhere), Create archive, Compress and email (disabled: no mail app), Convert to PNG/BMP (images decodable in-crate), Rename, Manage tags, Refresh. Disabled actions show their reason. | `shell_review::details_pane_reports_real_metadata_for_each_kind`, `header_tests`, `details::tests`, captures `target/explorer-review/details-*.ppm` |
| Explorer address bar | Implemented (`files/address.rs`, `explorer_path::crumb_layout`): location glyph (Desktop/This PC monitor, known-folder glyphs), known folders root their chain ("Desktop > Doors OS >"), trailing chevron, constant spacing measured from the 25H2 reference (ink to chevron center 22 px, chevron to next ink 23 px at 100%; host ink measurement within 1.5 px), procedural 1 px chevrons, hover/pressed plates, chevron flyouts listing subfolders (current child bold, chevron points down while open), root chevron listing Quick access folders/This PC/drives, "..." overflow listing hidden ancestors, editing via empty-area click/icon/Ctrl+L/Alt+D/F4, Enter/Escape. 32 px address/search row (`ADDRESS_TOP/HEIGHT`). | `crumb_tests` (2), `shell_review::address_bar_follows_the_reference_spacing_and_hit_tests_every_plate`, captures `address-*.ppm` |
| Explorer menus | Implemented (`context_menu.rs` model + `files/menus.rs`): 25H2 compact menu with icon row (Cut/Copy/Rename/Share/Delete), per-subject groups for file, folder, multiple items, drive, This PC, folder/This PC background (View/Sort by submenus, Refresh, Paste, New > Folder/Text Document, Properties, Open in Terminal), tree rows, tags; "Show more options" opens the classic menu, `Controls::classic_context_menu()` opens it directly; icon row moves to the bottom when the menu opens upward; submenus flip left at the edge; disabled commands (no clipboard, no share provider, fixed Quick access pins) show their reason as a tooltip; keyboard: Shift+F10/menu key, arrows (wrapping, skipping disabled), Home/End, Tab to the icon row, Left/Right, Enter, Esc. Command-bar New/Sort/View/See more and right-click Back/Forward use the same flyouts. Multi-selection via Ctrl+A, Shift+arrows, Select all/none/Invert (pointer modifiers are not delivered to apps). New Text Document creates a unique empty file and starts rename. | `context_menu::tests` (5), `shell_review::context_menus_match_their_subject_and_paint_hit_test_alike`, captures `menu-*.ppm` |
| Explorer archive / convert / wheel | Implemented: stored ZIP writer (CRC-32, local headers patched after data, central directory, UTF-8 name flag, 1980 DOS dates since the volume has none, bounded 4096 entries / 256 MiB, no ZIP64) as a background job (64 storage calls per tick), unique "Name.zip" next to the items, partial output removed on failure; Convert writes stored-deflate PNG or 24-bit BMP. `Files::wheel`: flyouts, details pane and navigation pane by 48 px, items by whole rows (3 Details rows per notch), List by columns. Window size left at 1000x640 logical (52%x62% of a 1080p work area; the runtime clamps smaller outputs). | `archive::tests` (3), host + guest `cargo check`; guest archive/wheel not yet exercised |
| Explorer 26H2 deltas | Implemented: context menu decluttered (Compress to ZIP file and Copy as path in a "Manage file" flyout), "Customize menu" is the last entry of every modern menu and opens Settings page 16 (Personalization > Context menu) through `open_settings_page`; `CONTEXT_HIDE_FILE_ACTIONS` hides Manage file / classic Send to, `CONTEXT_HIDE_MORE_OPTIONS` hides Show more options; This PC Shift+F10 tail is Properties, Show more options, Customize menu. Address bar accepts `~`, `~/sub` and `~\sub` (profile folder). View and sort are remembered per folder (This PC separately), in memory and in `%USERPROFILE%\.door-explorer-views` (bounded to 64 folders, hidden in listings, merged across windows, unreadable files are left untouched); collapsed sections are remembered (This PC "Devices and drives", navigation tree, Home sections). This PC search uses the shared name index `door_workspace::explorer_search::Index` (also the Start search provider entry point, with `explorer_search::query` and the `Open` request that opens a location with an item selected or a This PC search in a new Explorer window) of Local Disk (C:), built in the background (96 listing calls per tick, at most 20,000 names, depth 32, names only, first 128 matches shown); the status bar states coverage and index state. The tab close X is now a foreground glyph (was a theme-remapped rect, invisible on dark tabs). | `context_menu::tests` (6), `crumb/tilde` tests, `prefs::tests`, `search::tests`, `shell_review::{folders_reopen_with_their_own_view_and_sort_26h2, shift_f10_on_this_pc_matches_the_controls_scenario, tab_close_glyph_stands_out_in_both_themes}` |
| Explorer Home | Implemented (`files/home.rs`, `recent.rs`): Home is the first navigation-pane row (above the pins and This PC) and where new windows and new tabs (Ctrl+T, "+") open; address bar "Home" with the Home glyph, root flyout and history list it. Collapsible sections, collapse state remembered in the views file: Quick access (the six known folders, always pinned, plus up to 16 user pins from "Pin to Quick access"; only user pins unpin), Favorites ("Add to Favorites" / "Remove from Favorites", up to 64), Recent (Name and Location columns, no date column). Empty sections use Windows' wording. Item menus add "Open file location" (opens the parent with the item selected) and, for Recent, "Remove from list"; command bar, details pane (Home summary with section counts), keyboard (arrows across sections, kept in view), wheel and view modes (Details lists, icon views tile) work on Home items; rename/delete are refused with a reason; Sort is disabled. Recent list: `door_workspace::recent` keeps the last 50 files/folders opened through Explorer, Notes (open/reopen and save) and Preview (open) in `%USERPROFILE%\.door-recent` (deduplicated, open counter, ordered by a recency sequence; changes are replayed onto the current file so windows and apps merge; a missing file starts empty, an unreadable or malformed one leaves the list session-only and is never overwritten; hidden from listings and the search index). Terminal has no command that opens files, so it records nothing. Guest scenarios: lifecycle (desktop-wide) Explorer steps recalibrated for Home and a maximized window, `explorer::exercise` breadcrumb/footer/view points and view markers; points asserted by `files/scenario_points.rs`. | `recent::tests` (2), `home_review` (3), `scenario_points` (3), captures `target/explorer-review/home-*.ppm`; guest scenarios not run here |
| Explorer details pane: file metadata | Implemented (`file_metadata.rs` + `file_metadata/*`, pure `no_std` + alloc; `files/details.rs` job): the pane follows the selection (selected item, else the open folder, else the summary) whatever changed it; inline rename no longer clears it (owner report: Home + F2 showed "notes, 1 item"); Properties on another subject holds until the selection changes. Single files get Windows-style sections (Description, Origin, Media, Video, Audio, Image, Camera, Advanced photo, GPS, Content, Archive, 3D model, Program, Font) before "File" (Type with the detected format, Size, Location, Modified/Created "Not recorded by this volume", Object ID, Version), "Reading..." while a background job (48 RPCs per tick) fetches only the ranges a parser asks for (`Outcome::Need`, at most 1 MiB and 512 rounds per file) or streams text-like files (at most 8 MiB; counts then "At least"). Formats: PNG (IHDR, pHYs dpi, tEXt/iTXt, eXIf, sRGB/iCCP, APNG frames), JPEG (SOF size/depth/encoding, JFIF dpi, Exif: camera, date taken, orientation, exposure, f-stop, ISO, focal length, bias, flash, metering, 35 mm, color space, GPS, XP title/comment/author/tags/subject, rating), GIF (frames, loops, length), BMP, QOI, PBM/PGM/PPM/PAM, WebP (VP8/VP8L/VP8X, frames, Exif), ICO/CUR, TIFF; WAV (format, channels, rate, bits, bit rate, length, INFO), MP3 (ID3v2.2-2.4 text/comment/cover/TLEN, ID3v1, MPEG header, Xing/Info/VBRI or CBR length), FLAC (STREAMINFO, Vorbis comments, picture), Ogg Vorbis/Opus/FLAC/Theora (heads, tags, length from the last granule); MP4/MOV/M4A (brand, mvhd length and creation, tkhd/stsd size, codec, frame rate from stts, audio channels/rate, iTunes ilst tags), AVI (avih/strh/strf, INFO), Matroska/WebM (doc type, duration, title, apps, date, tracks: codec, size, frame rate, audio rate/channels); ZIP (central directory incl. ZIP64: items, files, folders, sizes, space saved, method, encryption, newest item, comment; Office Open XML/EPUB/OpenDocument/JAR/APK detection), TAR (checksummed headers), GZIP (name, comment, mtime, OS, ISIZE); PDF (version, pages via linearization or the classic xref page tree, else an uncompressed page tree near the start; encryption; Info title/subject/keywords/author/creator/producer/dates); text and source (language, lines, words, characters, encoding UTF-8/BOM/UTF-16/ASCII/ANSI, line endings, Markdown headings); OBJ, ASCII/binary STL, glTF/GLB, PLY; PE (type, machine, PE32/PE32+, subsystem, .NET, linker time, version resource: file/product version, description, product, company, copyright, original name, language), ELF (class, endianness, machine, type, OS ABI, linking, entry; Doors native layout recognized); TrueType/OpenType/collections (family, style, full name, version, glyphs, weight, copyright, trademarks), WOFF/WOFF2 container only. Not read: compressed data (PDF object/xref streams, PNG zTXt, WOFF tables, docx properties inside ZIP), HEIF/AVIF item properties, MKV tags, ZIP data descriptors beyond the central directory. | `file_metadata::*::tests` (26, each format from synthetic bytes; every prefix, random corruption and garbage after every signature), `shell_review::{details_pane_reports_real_metadata_for_each_kind, details_pane_follows_the_selection_through_rename}`, captures `details-*.ppm`; guest not yet run |
| Explorer text fitting | Implemented: every Explorer label (Details cells and headers, tiles, Home lists, status bar) that does not fit ends with "..." at its edge instead of a glyph cut in half (`modern/paint.rs::label`/`fit`). Details columns fit the list: the user's widths while they fit; otherwise Name gives way first (down to 96 px), then Type and Size shrink in proportion to their spare width (minimums 72 / 64), and a list narrower than the minimums scales them; column drags start from the shown width. The breadcrumb's collapse is minimal (a leading crumb is hidden only when the rest cannot fit): at 896x524 `C:\Users\1001\notes` needs 431 px of crumbs but the 420 px address box leaves 356 after the glyph and root chevron, so "... > Users > 1001 > notes" is correct there, while `C:\Users\1001` (355 px) shows This PC and Local Disk (C:). | `fit_review::{details_cells_end_with_an_ellipsis_inside_their_columns, narrow_lists_shrink_name_first_then_type_and_size_proportionally, breadcrumbs_hide_leading_crumbs_only_when_they_do_not_fit}` at 896x524 and 717x562, pane open and closed; captures `target/explorer-review/fit-*.ppm` |
| Settings page heading | Implemented: `settings_navigation::{ancestor, heading}` derive "Parent > Page" from `Page::parent` (route model, not Back history); muted ancestor is a link (id 260 → `go(parent)`, hover/pressed tints), Advanced pages show a non-link "Advanced" group, current page is never a link, the old 14 px Advanced crumb and its Back-popping target are gone, narrow windows drop the ancestor before clipping the title, and the heading stops before Advanced header actions. | `heading_hierarchy_comes_from_routes_and_degrades_without_overlap`, `page_heading_shows_route_ancestors_and_only_ancestors_are_links`, captures `target/settings-review/heading-*.ppm` |
| Frame submission (app → server) | Implemented band commits: server-advertised `CAP_BAND_COMMIT` and 24-byte COMMIT(window, epoch, top, rows); the server copies, validates and exactly diffs only those rows and patches the trusted snapshot in place (per-row translucency keeps `opaque` exact), falling back to full capture without a same-buffer snapshot. The SDK surface compares its private frame with the section by raw row copies, writes only changed rows with a bulk copy (was per-pixel volatile stores), and resyncs fully after any failed submission or resize. Host split before the change, 1460×900 Explorer frame in release: paint 1.32 ms, section copy 0.29 ms, server validate+diff 0.76 ms. | `band_commits_patch_only_validated_rows_and_match_full_captures`, `band_commits_require_a_snapshot_from_the_same_buffer`, host+guest `cargo check` of door-window/workspace/preview; guest timing not yet measured (see blocker) |
| Start / taskbar / shell (26H2) | Implemented as host-tested `door_window` modules (`start`, `taskbar`, `tray`, `menu(s)`, `snap`, `snap_assist`, `run`, `search`, `motion`, `acrylic`, `apps`); the bin `desktop.rs` is a controller. Start: search pill, Pinned/Recent/All (Category/Grid/List), 26H2 size Automatic/Small/Large and per-section hide bits, hide account, opens from the taskbar edge. Taskbar: 4 edges, both alignments, Small size, tray, two-line clock, jump lists (pin/unpin, close), Task Manager entry. Quick settings/notification centre/calendar with honest disabled tiles. Snap layouts + Snap Assist (`Engine::snap_window_rect`, 200 ms glide, stretched last frame until repaint). Fluent Run dialog (Win+R). Win+X/L/M/Z/R/Q/S/1..9, Ctrl+Shift+Esc. Pins/order/All view persist per user (`ShellLayout`/`ShellOrder` via Init); desktop icons off by default and write through to Controls. Start search: apps, Settings pages (ids 0–45) and files from a persistent search host (`mode::SEARCH_HOST`, delayed index start, name index ≤20 000, p50 0.12 ms / p95 0.16 ms per query on a 5 102-item host profile); Recent from `recent.rs`. | `door-window` 71 lib tests + `shell_review`; guest: Start/taskbar/Quick settings captured in QEMU (desktop-shell focus section passes; tray probes being recalibrated for acrylic) |
| Taskbar Search (26H2) | Implemented: coloured search glyph (`search_glyph.rs`, measured from the owner's 150 % capture: sweep-gradient ring, frosted disc, #0077D4 handle) rasterized as an antialiased vector at the canvas scale in the search box and icon-only/button modes, monochrome system colour in contrast themes, 87.5 % scale-down while pressed. Taskbar Search / Win+S open the dedicated Search panel (`start::Mode::Search`, Start's surface, engine, provider, acrylic and motion; Start's anchor, 720 px, no account/Power footer): pills All/Apps/Documents/Web/Settings/Folders/Photos (Web and Photos disabled, no backend; trailing pills that don't fit large text are left out), home = Recent (provider) + Top apps (Start's first pins; no usage history), typing = grouped results with best match and details pane. Typing in Start switches to it; Esc/emptying the query returns to Start. Esc and outside click dismiss. Semantics: window "Search", own search box node (20), disabled pills, Recent/Top apps lists; taskbar Search is expandable. Not done: web highlights/Quick searches (no network service), app actions beyond Open (Pin/Uninstall). | `tests/search_panel.rs` (6), `search_glyph` unit test; guest desktop-shell `host-1792857080609488800-22608`, a11y-01 `host-1792857023020207000-4772`, motion-01 `host-1792857238333000700-7096` pass |
| Compositor / scheduling / damage | Implemented: wheel end to end (PS/2 IntelliMouse negotiation, `CAP_WHEEL`, routing to capture/pointer owner, coalescing), up to 4 damage regions per frame, translucent row-span blending, motion coalescing, deferred interactive resize, packed 2-lane blend. Host release timings at 1080p (before → after): full frame 3 windows 24.9 → 4.8 ms, translucent 1200×800 window 23.3 → 3.9 ms, partial 400×300 3.3 → 0.48/0.33 ms, 3 px drag 1.78 → 0.86 ms, two distant updates 21.2 → 0.026 ms. | `door-graphics-core` 95 tests (+3 ignored benches), `compositor_review`; guest FRAMETIME lines below |
| Terminal live backdrop | Implemented: `SET_MATERIAL`/`CAP_ACRYLIC`; the server samples the scene beneath the window (8-DIP blocks, two radius-2 box passes, bilinear), damage within a 40-DIP margin recomposes it; Reduce transparency renders solid. Terminal paints premultiplied translucent backgrounds (dark 78 %, light 80 %). | Guest capture `desktop-terminal-reference-window` shows the blurred wallpaper through rail and console |
| Motion | Implemented: open (95→100 % scale + fade, 200 ms decelerate, after first frame), close (150 ms), minimize to the taskbar button (250 ms), snap glide (200 ms), Start/flyout slide (250 ms open, 150 ms close); Reduce motion disables all. `TransitionClock` (10 ms ticks refined by rdtsc); idle scenes queue nothing. | Transition tests incl. 60-cycle leak test; timing on the guest only via FRAMETIME (screenshots do not establish animation timing) |
| Typography / icons / logo | Text uses the existing hinted Segoe UI atlases with LCD coverage (unchanged). Icons: Segoe Fluent Icons masks enumerated in `assets/icons/windows-shell/manifest.json` (+48 shell/Explorer glyphs this pass, `assets check` passes). Logo: `assets/branding/door.svg` master with exported sizes, used for the Start button, boot splash (with a static 26H2-style "donut" indicator) and Settings About. Licensing note: `assets/fonts/segoe-ui` holds Windows-supplied Segoe UI copied from the host in an earlier session ("distribution unreviewed"); the open-licensed Selawik fallback is in the tree. | `door-graphics-text` 19 tests; guest captures |
| Settings (26H2) | Every page on one catalog model (hero, sections, cards, expanders) with real bindings to Controls/appearance; unavailable rows disabled with reasons. 26H2: Start size/section toggles, Taskbar behaviors (position, Small/Default), Context menu page (id 16), Privacy & security sections, System > Advanced (id 44), Search (id 45). Wheel scrolling, keyboard model, deep links. | `settings_reference_painter` 17 tests; guest `settings-reference` passes |
| Task Manager | Rewritten from real kernel snapshots: Processes (grouping, heat map, sort, search), Performance (CPU/Memory/Disk graphs), App history, Startup apps (honest empty), Users, Details (Isolation column), Services; Run new task launches real apps; End task etc. disabled (no terminate authority). | `door-monitor` 15 tests; guest `monitor` passes |

## Root causes fixed

- Preview: "Library"/"Tools" were painted text at fixed y with decorative,
  non-interactive `Down` glyphs; rows used three hard-coded bands; the pane
  scrolled in 4-row steps through two 24×32 buttons whose lone `Down` glyph sat
  under "Edit" at ~736–776 px heights (the isolated chevron). Rows that did not
  fit were silently dropped from paint and hit testing (OCR unreachable for
  890 ≤ h < 904). Hover was computed from unclipped rectangles.
- Explorer: see the status table; the confirmed primary cause was the sidebar
  "Local Disk (C:)" row sharing target 500 with the This PC tile, whose single
  click only opened details.


## Guest validation (QEMU 11.1, WHPX, 1 vCPU, 2 GiB, RAMFB)

Disk budget: `out/` was over the 12 GB Legacy admission ceiling; with the owner's
approval the 70 oldest `out/qemu` session folders were deleted (10 newest kept),
taking `out/` from 12.24 GB to 8.24 GB. The admission limit is now 20 GB
(`cargo xtask cache status`: 9.06 GB on 2026-10-23).

### 2026-10-23 full image and 11-scenario batch

Image: `reports/runs/host-1792734893293928700-30428` (full `desktop-qemu-x64`
build; `door-workspace` 15.6 MB against the 16 MiB native loader limit — an
earlier 17.2 MB build was refused and fixed by sourcing the Settings wallpaper
previews from one 2x QOI per wallpaper, averaged at 1x and bilinearly
resampled at 3x). Host: `cargo test --workspace` 70 suites, 0 failures.

| Scenario | Result | Evidence |
|---|---|---|
| `test explorer` | pass | `host-1792735096918923200-15452` |
| `test preview` | pass | `host-1792735159122227600-28716` |
| `test settings-reference` | pass | `host-1792735303787452500-28152` |
| `test multiwindow` | pass | `host-1792735373213538000-22788` (Explorer and Preview tab tear-off/re-attach) |
| `test terminal` | pass | `host-1792735452701991200-31872` (live acrylic visible in the capture) |
| `test monitor` | pass | `host-1792735550357791200-10596` (Task Manager, all pages) |
| `test desktop-shell` | pass | `host-1792735613612981400-21956` (focus round trips, acrylic tray, caption buttons, Start search) |
| `test desktop-files` | pass | `host-1792735749048292000-28012` (details pane shows the selected file's metadata) |
| `test desktop-notes` | pass | `host-1792735851126694900-20196` |
| `test desktop-motion` (3840×2160, scale 3) | pass | `host-1792736341535868700-14736` |
| `test desktop-wide` | fail at the final audit | `host-1792736013852484600-25668`: every interaction passed; the shutdown audit expects the old "Save aborts power, power again" sequence while the session now finishes shutdown after the saved document closes. Being resolved (Windows semantics: Save/Don't save continue, Cancel aborts). |

Product bugs found only by the guest runs and fixed:
- Input batching merged a button release with the following pointer move, so
  `input.rs` applied the move first and cancelled the click (desktop icon
  double-click never launched). A button change now closes the batch.
- The Maximize-hover snap flyout covered the Maximize button.
- Notes kept a hover plate after a modal was dismissed while the button was
  held; clients now also receive a pointer-leave motion when the target changes.
- Settings could not maximize at 3x: `Surface::reconfigure` reserved the 24 MiB
  margin and the new 29.5 MiB frame while the old frame was alive (96 MiB
  first-fit heap). The old frame is now released first; refusals are logged.
- Start's fade-out never reported a completed foreground scene.
- Re-opening the newest Recent item rewrote `.door-recent` on disk.

### Frame timing (guest `DOOR_P12 FRAMETIME`, µs, p50/p95/p99/max)

| Run | Frames | Work (frame build) | Compose | Input → presented |
|---|---|---|---|---|
| Explorer, 1920×1080 | 286 | 5617 / 15559 / 253465 / 455255 | 3878 / 12941 / 250945 / 453575 | 13246 / 79735 / 192197 / 209455 |
| Preview, 1920×1080 | 579 | 1688 / 16280 / 25801 / 628401 | 433 / 12191 / 18473 / 626638 | 12594 / 39021 / 99087 / 213907 |
| Settings, 1920×1080 | 206 | 4298 / 13783 / 31201 / 636603 | 1688 / 10719 / 21907 / 25200 | 11453 / 32882 / 67165 / 73465 |
| desktop-motion, 3840×2160 | 279 | 35412 / 87528 / 598981 / 610787 | 28038 / 77781 / 592346 / 603377 | 57564 / 233707 / 335728 / 335728 |

Single vCPU under WHPX. 1080p frames are mostly inside one 60 Hz budget at
p50; the p99 tails (0.25–0.6 s) and the 4K numbers are not smooth and remain
open work. Screenshots establish no timing.

## Remaining validation

- `desktop-wide` final shutdown audit (Save/Don't save continue the shutdown,
  Cancel aborts it) and the scenarios outside this batch (`native_default`,
  `recovery`, `desktop-resolutions`) that still assume older window placements.
- Fractional display scaling (125/150/175 %) is not implemented: the engine,
  protocol and apps use integer scales (1x below 2560 px wide, then 2x and 3x),
  so a 2560×1600 laptop gets a 1280×800 logical desktop where Windows at 150 %
  has 1707×1067. This, not the controls' own sizes, is why Doors windows look
  larger relative to the screen than Windows on the owner's laptop.
- Frame-time tails (p99 0.25–0.6 s) and 4K latency.

## P12 gate fixes (2026-10-25)

- `multiwindow`: Terminal's surface failed with `SECTION_MAP MappingLimit
  mappings=128`. That table (`mm::MAX_MAPPINGS`) is kernel-global, not
  per-process: every process's table pool, stacks, startup, TLS, segments and
  heap regions, every section and every boot mapping share it, and the frame
  allocator's records (`mm::MAX_ALLOCATIONS`) mirror it. The `door-rt` heap
  never grows (it maps its plan once); its extent table behind a guard page
  added one region per scaled-heap process, which pushed 12 processes past
  128, and the 13th (Monitor) then hit the 128 frame records. Both are now 320,
  and `process/vm.rs` asserts the declared capacity (13 processes x 15, 16
  threads x 3, 32 sections, 16 boot) fits both (ADR 0004). Launch exhaustion
  now logs `LAUNCH_EXHAUSTED stage=... processes= threads= sections= mappings=
  tables= free_pages=`. Pass: `host-1792935519110335200-18444`.
- `multiwindow-pressure`: with 70 % x 78 % placement the maximized source left
  no room to *register* the window the server should refuse. The scenario now
  restores the source and opens windows until the server refuses one after
  registration; the count comes from `pressure_budget`, a replay of the
  server's own limits (`policy::limits`, `capture::{CELLS, RETAINED_CELLS,
  LARGE_BYTES}`) over the sizes it actually captured, and the proof requires
  the server's `CAPTURE RESOURCES` line for that window's first frame. Pass:
  `host-1792935998101809700-12108`.
- Same image: `desktop-remove-accelerated` `host-1792936115074752500-18892`,
  `desktop-notes` `host-1792936320920251200-19784`, `terminal`
  `host-1792936533617146500-1756`.
