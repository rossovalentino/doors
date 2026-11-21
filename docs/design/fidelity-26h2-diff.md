# Windows 11 26H2 fidelity diff (owner PC reference, 2026-10-26)

A prioritized list of differences between Door OS and the owner's Windows 11 26H2, measured on
the owner's PC. Each row gives the component, the Windows measurement, the Door measurement, the
fix, and the likely Door source file. Source files were located with grep and nothing was edited.
The rows are ordered by visual impact.

## Reference setup

| Item | Value |
|---|---|
| Windows | 26H2, build 26300.9711 (Windows 11 Pro) |
| Display | 2560×1600 at 150 % (AppliedDPI 144); taskbar 2560×72 px = 48 epx |
| Theme | **Dark** for apps and system, transparency on. Light theme was not captured: switching it is a settings change. |
| Accent | Owner palette (Light3…Dark3): `#F0C0F4 #DB9EE5 #B763CB #A94DC1 #8E3AA7 #692782 #400E59`. Accent-filled controls in dark mode measure **`#DB9EE5`** (Light2). DWM AccentColor is `#680081`. |
| Owner configuration that affects the reference | Taskbar centred; search shown as an icon only; Task View and Widgets off; Start without a Recommended/Recent section; **classic context menu forced** by the registry key `HKCU\Software\Classes\CLSID\{86ca1aa0-34aa-4e8b-a509-50c905bae2a2}\InprocServer32`; no Home or Gallery entries in the Explorer navigation pane; Explorer details pane on; the navigation pane width may have been resized by the owner; clipboard history off. |
| Units | **px** means physical pixels at 150 %. **epx** means effective pixels (px ÷ 1.5), which equal Door's logical pixels. Door guest captures are 1280×720 at 100 % (1 px = 1 epx), mostly in the **light** theme. |
| Windows captures | `reports/artifacts/reference-26h2/` (about 49 MB). `*.png` are PrintWindow captures (PW_RENDERFULLCONTENT) or screen captures; `*.uia.tsv` hold the UI Automation geometry (control type, class, name, rect in px), which is the precise source for every px number below. `icons/` holds reference-only shell icons at 16–96 px. `environment.txt` and `capture-log.txt` record the setup and each step. `sample-folder/` contains test files. Files starting with `_` are zoomed crops and contact sheets. |
| Capture scripts | `%TEMP%\claude\…\scratchpad\ref26h2\` (`RefCap.cs`, `common.ps1`, `explorer*.ps1`, `settings.ps1`, `apps.ps1`, `icons.ps1`, `burst.ps1`, `Px.cs` / `px.ps1` for pixel sampling). |
| Door captures | `reports/runs/host-1792994476553899100-12788/` (2026-10-26 14:42–14:59, the latest run). This run has no `display01-*` or `settings-reference` captures, and no `win32-*` captures are newer than `host-1792990125882807700-18904`. |
| Extra Windows captures | Another session is writing `reports/artifacts/reference-26h2/settings-unification/` (about 50 `ms-settings-*` pages and legacy Control Panel pages). It is not part of this diff, but it is usable as a reference. |

## Top 15 by visual impact

1. Window shadow: Door's is 12 epx, symmetric and 21 %; Windows' is offset down, extends 80/50/32 epx (bottom/sides/top) and peaks at 58 % (S1).
2. Explorer pins are hollow and should be filled (P0-1).
3. Monitor uses its own icon instead of the Task Manager icon (P0-2).
4. The Preview icon reads as a pale framed photo instead of a full-bleed blue→violet landscape (P0-3).
5. The Explorer chrome is 18 epx too tall: navigation band 58 vs 48, command bar 56 vs 45, content top 154 vs 136 (E1).
6. Explorer surfaces are split: the navigation pane tint, command bar and selected tab differ, where Windows has one surface each (E2).
7. No Mica: Settings, Task Manager and the Explorer tab strip are flat where Windows shows a wallpaper gradient (S3).
8. The accent is fixed blue; the owner's purple accent drives every accent surface (S2, G10).
9. Subpixel text everywhere, where XAML surfaces use grayscale (S4).
10. Explorer navigation tree: 38 epx rows (Windows 32), a 190 epx pane (Windows ≈155), no separator, and a duplicate C: (E4, E5).
11. Quick access: 32 epx icons in 240 epx tiles, where Windows uses 48 epx icons in 270 epx tiles, plus the 26H2 Favorites/Shared pills (E6, E7).
12. Details view: the header is 34 epx with a border line (Windows 27, no line); rows 30 vs 28.7 epx (E8).
13. Alt+Tab: white plates with an always-visible ✕ and no app icons (H7).
14. Notification center and calendar: 360 epx wide (Windows 336), no Focus footer or DND/Clear-all buttons (H5).
15. Task Manager: rows 32 vs 28 epx, caption 48 vs 52 epx; Settings cards 66–67 vs 69.3 epx; navigation items inset 8 vs 16.7 epx with a persistent scrollbar (T1, T5, G2, G5).

---

## P0 — the owner's three items

### 1. Explorer pin glyphs must be filled, not hollow

| Component | Windows (measured) | Door (measured) | Fix | Door source |
|---|---|---|---|---|
| Navigation pane pin (pinned folders) | **Solid** pin: 16×16 px = 10.7 epx box, colour `#95A0A6` on `#191919` (normal row) and on `#333333` (hover/selected row). Right-aligned: the glyph box starts 214 px from the frame, against a navigation pane 231 px wide. See `_zoom-win-navpin.png`. | **Outline** pin: 12×12 epx box, grey (~`#8B8D8E`) on `#E9F0F6`, drawn at x = 170 (`_zoom-door-navpin.png`). | The mask is generated from Segoe Fluent **U+E718 (Pin, outline)**. Windows draws the filled pin as two layers: **U+E840 Pinned (outline) + U+E841 PinnedFill**. (U+E718 + U+E842 renders the same.) Verified by rendering the repo font (`_pin-layered.png`). Regenerate `explorer-pin` as the union of E840 and E841 at every size. Use ~11 epx instead of 12 and tint it `TextFillColorSecondary`-like (dark `#95A0A6`; light: use the same role). | `assets/icons/windows-shell/manifest.json` (`explorer-pin` → codepoint `e718`), regenerated by `tools/xtask/src/assets/shell.rs`; drawn by `desktop/workspace/src/files/modern/paint.rs` (`tiny_icon(170, …, Icon::Pin, MUTED)`) through `desktop/workspace/src/icons.rs` (`Self::Pin => windows!("explorer-pin")`) |
| Home > Quick access tile pin | Solid pin, 15 px = 10 epx, `#757575` (dark), placed on the **third text line** directly under "Stored locally", left-aligned with the text column. | Outline pin at the tile's **bottom-right corner** (`cell.x + width - 22`). The tile is 240 epx wide, so the pin is often clipped or off-screen. | Use the same filled mask and move it to the text column (x = text left, y = second line bottom + 2 epx). | `desktop/workspace/src/files/home.rs` (`if section == QUICK { v.tiny_icon(cell.x + cell.width - 22, …) }`) |
| Start / jump-list "pinned" state | Filled pin wherever the state is "pinned". | `shell-pin` also uses `e718`. | Apply the same E840+E841 union to `shell-pin`; `shell-unpin` stays `e77a`. | `assets/icons/windows-shell/manifest.json` (`shell-pin`) |

### 2. Monitor → Task Manager: icon and naming

| Component | Windows | Door | Fix | Door source |
|---|---|---|---|---|
| App icon (taskbar, Start, Search, desktop shortcut, Alt+Tab, title bar, process list, Settings hero) | Task Manager icon: a **landscape "area chart"**. A dark-to-mid blue gradient band at the top, a light-blue waveform area below, and **no rounded-square plate** (`icons/taskmgr-shell-{16..96}.png`, `_icons-photos-vs-preview-taskmgr-vs-monitor.png`). The source is `C:\Windows\System32\Taskmgr.exe`, first `RT_GROUP_ICON` = **30651** (the ID `ExtractAssociatedIcon` uses). | Original "monitor" mark: a blue rounded square with a white pulse line and a darker offset base (`assets/branding/monitor.*`). | Acquire the installed icon with the existing private-reference pipeline, as already done for `terminal-current`: `cargo xtask assets acquire-icon taskmgr-current --module C:\Windows\System32\Taskmgr.exe --id 30651`. This writes `assets/icons/windows/taskmgr-current.{ico,source.json}` plus BGRA frames under the `installed_windows_icon_reference` provenance. Then switch every consumer from `branding/monitor` to it. Do **not** use the reference PNGs in `reports/artifacts/reference-26h2/icons/`: they are for comparison only. The owner's private-build licensing decision applies, exactly as for `explorer`/`notepad`/`settings`/`terminal-current`. | `graphics/window-server/src/apps.rs:338` (`App::Monitor => branding_icon!("monitor", …)`); `desktop/system-monitor/src/presentation.rs:394` and `:1230` (`color16!(s, "branding/monitor")`); `desktop/workspace/src/settings/pages.rs:1909–1915`; `tools/xtask/src/assets/ico.rs:92` (name list) |
| Name | "Task Manager" everywhere. Process image name `Taskmgr.exe`. | Already "Task Manager" in the shell label (`apps.rs:75`), Start/taskbar/menus (`menus.rs:148,324`), window title (`desktop/system-monitor/src/lib.rs:610`) and the process list (`desktop/system-monitor/src/model.rs:152`). Remaining user-visible "monitor" text: the process **image name `door-monitor`** (Details tab) and the Run alias `monitor`, which can stay as an extra alias. | Show `Taskmgr.exe` (or keep the Door naming scheme and document it) in the Details "Name" column for this process; keep `monitor` only as a search/Run keyword. | `desktop/system-monitor/src/model.rs:153` |

### 3. Preview needs a better, original icon (match how Photos reads)

| Aspect | Windows Photos (`icons/photos-shell-96.png`) | Door Preview (`assets/branding/preview.svg/png`) | Design target (original Door asset) |
|---|---|---|---|
| Plate | A full-bleed rounded square, 84/96 of the canvas (inset 6 px at 96), corner radius ≈ 12 % of the plate, **flat** (no base or drop layer). | A rounded square plus a separate **offset darker base** (`rect y=2.5 … #123b91 @24%`) that reads as a thick button. A white inner stroke at 16 %. | A flat plate, inset ≈ 1/16 of the canvas, radius ≈ 12–13 %. Drop the base rect: shell plating already supplies depth. |
| Background gradient | Diagonal from **top-left sky blue `#1591DD`** through `#2A75C5` / `#3567BA` to **violet `#725BB5`** at the right and bottom-right. Deep and saturated: the image reads blue-to-purple. | Cyan `#46D9FF` → `#078FE8` → `#124BC0`, which is much lighter and cyan-heavy, with no violet. | Use a blue→violet diagonal in the same value range: start ≈ `#1890DE`, mid ≈ `#3A63B6`, end ≈ `#6F5AB4`. The Door "sky" variant can shift hue slightly (for example more indigo) to stay original. |
| Motif | **Landscape fills the lower half edge to edge, with no inner photo frame.** A large front hill in cyan bands (`#4EE3FE` → `#40D0F7` → `#32BDEF`) from the lower-left, and a back hill in royal blue (`#0278D0` → `#0E8BDB` → `#2199E1`) to the right, with a thin dark seam (`#025DA3` / `#016BBB`) between them. The hills are rounded ridges, not sharp triangles. | A white **picture frame** inside the tile with a pale sky, two small sharp triangle mountains (`#1977C6`, `#39B5DF`) and a bottom stroke line. It reads as "a photo in a tile", like older Gallery icons. | Remove the inner frame. Use two overlapping soft ridges with distinct, Door-original curves (for example ridges that echo the Door "open door" diagonal), in the same colour roles: light cyan front, saturated blue back. Keep a dark 1-unit seam. |
| Sun | A white disc, ≈ 15 % of the canvas wide, centred at (68 %, 25 %). | A white disc, ≈ 11 %, at (62 %, 41 %), inside the frame. | A white or near-white disc of 14–16 %, upper-right, partly overlapping the gradient (no frame). |
| Small sizes | At 16/24 px the silhouette stays a blue-violet square with a bright cyan wedge, lower-left. | Reads as a light-cyan square with a white box. | Hand-tune the 16/20/24 masks so the cyan wedge and the sun survive. Export with `cargo xtask assets branding build` and update `preview-monitor.source.json`. |

Consumers to keep in sync: `graphics/window-server/src/apps.rs:337`,
`desktop/preview/src/presentation.rs:881–883`, `desktop/system-monitor/src/presentation.rs:393`,
`desktop/workspace/src/settings/pages.rs:1900–1906`.

---

## P1 — system-wide rendering (affects every page)

| # | Component | Windows (measured) | Door (measured) | Fix | Door source |
|---|---|---|---|---|---|
| S1 | **Active window shadow** | Measured on an own white backdrop (`terminal-shadow-white.png` and `explorer-shadow-active-white.png` are identical profiles). It is **offset downward and asymmetric**. Darkening of the backdrop at distance d from the window edge: **below** 58 % at 0, 53 % at 8 epx, 44 % at 16, 35 % at 24, 25 % at 32, 18 % at 40, 12 % at 48, 7 % at 56, 3.5 % at 64, 1.6 % at 72, ~0 at 81 epx. **Sides** 36 % at 0, 23 % at 8, 12 % at 16, 5 % at 24, 2 % at 32, 0 at ~50 epx. **Top** 25 % at 0, 14 % at 8, 5.5 % at 16, 1 % at 24, 0 at ~32 epx. The shadow is neutral black. | `SHADOW_MARGIN` = **12 epx, symmetric** (quadrant-mirrored texture), peak alpha 54/255 = **21 %** active and 36/255 = 14 % inactive. Windows look flat and float less. | Replace the mirrored quadrant texture with an edge-specific profile: bottom extent 80 epx, sides 50, top 32, peak 58/36/25 %. A practical model is two layers: an ambient σ ≈ 8 epx, α ≈ 0.3 with no offset, plus a key σ ≈ 24 epx, α ≈ 0.45 offset +12 epx. Fit it to the table above. Cached-move eligibility and damage strips must use the larger asymmetric margins. The inactive shadow was not captured (see "Not captured"). | `graphics/compositor/src/render.rs` (`SHADOW_MARGIN`, `SoftShadow`, `shadow_opacity`); `assets/raster/soft-shadow.a8`; `graphics/compositor/src/engine/move_reuse.rs` (margins) |
| S2 | **Accent colour** | Every accent surface follows the user accent: nav selection indicator, toggles, sliders, QS active tiles, calendar today, Alt+Tab ring, taskbar active indicator, Task Manager heat map, focused search underline, links. Dark mode uses Light2 (`#DB9EE5` here). Light mode uses Dark1. | Fixed blue: dark `#60CDFF`, light `#005FB8`/`#0067C0`. Settings > Colors says "The accent color is fixed in this build". | Implement the accent picker and propagate `AccentPalette` (Light3…Dark3) through `Theme::with_accent`. Map dark controls to Light2, light controls to Dark1, text to Light3/Dark2. A default Windows install still uses blue, so this mostly matters for parity with the owner's PC. | `frameworks/ui/src/theme.rs` (`with_accent`); `desktop/workspace/src/settings_catalog.rs:2978`; shell accents in `graphics/window-server/src/{taskbar,tray,overview,multitask}.rs` |
| S3 | **Mica backdrop** (Settings content, Explorer tab strip, Task Manager title/nav, Notepad tab strip) | The wallpaper-tinted Mica **varies across the window**. Settings page `#1D2124` top-left → `#271B27` bottom-right. The active Explorer tab strip is `#0D0425`–`#16011C` (purple from the owner's wallpaper). Inactive windows fall back to solid `#202020`. | The Settings page is a flat `#1F2024` (dark) / `#F4F7FA` (light). Explorer's tab strip uses a flat tint `#E1EBF3`. `static_base` only gives Mica-like material to tabbed captions and acrylic windows. | Sample the blurred wallpaper under the window (Mica = heavily blurred wallpaper plus a luminosity tint: dark ≈ `#202020` at ~80 %, light ≈ `#F3F3F3` at ~50 %). Apply it to the whole client background of Settings, Task Manager and Photos, and to the caption/tab strip of Explorer, Notepad and Terminal. Swap to the solid fallback when inactive. | `graphics/compositor/src/render.rs` (`static_base`, `transparency_fallback`); `frameworks/ui/src/theme.rs` (`background`) |
| S4 | **Text antialiasing** | XAML surfaces use **grayscale AA**: 0 colour-fringed pixels in the Settings nav, Settings cards and Explorer command bar. Only Win32 DirectUI and Win32 surfaces use ClearType: Explorer navigation tree (24 % fringed), items view (12 %) and classic menus. | Subpixel (ClearType-like) everywhere: 30 % fringed in the Settings nav and cards, 27 % in the Explorer command bar. | Render grayscale coverage for Settings, Start, Search, flyouts, Task Manager, Photos/Preview, Notepad chrome and the Explorer tab/address/command bars and Home page. Keep subpixel only for the Explorer tree and items view, classic menus and Win32 apps. | `frameworks/text/src/atlas.rs` (subpixel atlases); text call sites per surface |
| S5 | Window corner radius | 8 epx (outer contour ≈ 12 px at 150 %, with a 2 px border). | `WINDOW_RADIUS` = 8. | Matches. | `graphics/compositor/src/render.rs` |
| S6 | Typography | Segoe UI Variable. Body text width matches Door to ±1 epx (for example "Display resolution": Windows 110.7 epx, Door 111). | Static Segoe UI atlases (`segoeui.ttf`/`seguisb.ttf`); Segoe UI Variable exists in `assets/fonts/segoe-ui/SegUIVar.ttf`. | Low priority. Metrics already match; switch atlases to Segoe UI Variable Text (≤ 14 epx) and Display (≥ 20 epx) for glyph-shape parity. | `frameworks/text/src/atlas.rs`, `font.rs` |

---

## File Explorer

Windows reference: `explorer-home.png`, `explorer-thispc.png`, `explorer-folder-details.png`,
`explorer-folder-tiles.png`, `explorer-view-popup-0.png`, `explorer-new-popup-0.png`,
`explorer-file-selected.png`, `explorer-context-*`, `explorer-shadow-active-white.png`,
and the `*.uia.tsv` files. The window was 1778×1139 px (1185×759 epx).
Door reference: `desktop-explorer-*.png` (light theme, maximized at 1280×720), `desktop-files-context.png`
and `desktop-settings-theme-0.png`.

| # | Component | Windows (measured) | Door (measured) | Fix | Door source |
|---|---|---|---|---|---|
| E1 | Vertical chrome bands | Tab strip **40 epx** (60 px). Navigation/address band **48 epx** (72 px) with a 32 epx address box centred (8.7 epx top margin). A 1 epx separator. Command bar **45.3 epx** (68 px), filled edge to edge by the buttons, which are 44.7 epx tall. A 1 epx separator. Content starts at **136 epx**. | Tab strip 40 ✓. Navigation band **58** (address box at 53..85, 13 epx margins). Command bar **56** (40 epx buttons). Content at **154**: 18 epx taller in total. | Navigation band 48: `ADDRESS_TOP` 48, band 40..88, `command_top` 88. Command bar 45 with buttons filling the height (44 epx). `CONTENT_TOP` 136. | `desktop/workspace/src/explorer_layout.rs` (`CONTENT_TOP`, `ADDRESS_TOP`, `command_top`, `command_height`) |
| E2 | Band colours | Selected tab = address band (`#2C2C2C` dark: one continuous surface). Tab strip = Mica (`#202020` inactive). Address box `#383838`. Separators `#3A3A3A` 1 epx. **Command bar = content = navigation pane** (`#191919`). The details pane top is `#222222`. | Light: selected tab `#F9FBFC` ≠ address band `#F3F7FA`. Command bar `#FAFCFD` ≠ content `#FFFFFF`. **Navigation pane tinted `#E9F0F6`** ≠ content. The draw_rail base is `[246,240,233]`. | Make the selected tab and address band one fill. Make the command bar, navigation pane and content one fill (light: the content colour). Keep only the 1 epx separators. Verify in Door dark: no dark Explorer capture exists. | `desktop/workspace/src/files/modern/paint.rs` (`draw_rail`, band fills); `desktop/workspace/src/palette.rs` |
| E3 | Tab geometry | Tab 248×32 epx (372×48 px), top at 8 epx, icon 16 epx at +12 epx, title at +38 epx, close button 32×24 epx; "+" button 32×24 epx. | Tab 290 epx wide, top at **3** epx (37 tall). | Tab top 8 epx (32 tall), width 248 at the default size. | `desktop/workspace/src/caption_tabs.rs` |
| E4 | Navigation tree rows | Row pitch **32 epx** (48 px). Hover/selected plate 30.7 epx tall, `#333333`, inset 2–3 epx. Pane width ≈ **154–157 epx** (231 px plus a 2+2 px divider); the owner may have resized it. A 1 epx separator line (`#383838`, x 11–149 epx) sits between the pinned group and This PC. | `ROW_PITCH` **38** / `ROW_HEIGHT` 36. `SIDEBAR` **190**. No separator line, only a 17 epx gap. | Pitch 32 and height 30. Default pane width ≈ 155 (check against a default profile). Draw a 1 epx separator. | `desktop/workspace/src/explorer_tree.rs` (`ROW_PITCH`, `ROW_HEIGHT`, `SEPARATOR_PITCH`); `explorer_layout.rs` (`SIDEBAR`) |
| E5 | Navigation tree content | Pinned folders, separator, This PC (with Local Disk (C:) as its child), Network. No root duplicate of a fixed drive. | "Home" at the top and **"Local Disk (C:)" listed twice**: under This PC and again as a root shortcut. | Remove the root shortcut for fixed drives; Windows lists only removable drives at the root. The owner has no Home/Gallery entries: keep Home behind a setting matching "Show Home"/"Show Gallery". | `desktop/workspace/src/explorer_tree.rs` (root drive alias) |
| E6 | Home > Quick access tiles | Tile **270×64 epx**, column pitch **284**, row pitch **60**. Folder icon **48 epx** (72 px). Text column 11 epx right of the icon: name 14 epx (16 epx line), "Stored locally" secondary `#CBCBCB` 12–14 epx, filled pin on line 3. The section header "Quick access" is 14 epx semibold, with a 16 epx chevron at x+11 epx. | `TILE` **240×64**, icon **32 epx**, text at +52, row pitch 72 (in the capture), pin at the bottom-right. | Use the Windows numbers: icon 48, tile 270×64, pitch 284×60, pin under the text. | `desktop/workspace/src/files/home.rs` (`TILE`, `sized_icon(…, 32, …)`, label offsets) |
| E7 | Home sections | "Quick access" (collapsible), then **pill tabs "Favorites | Shared"** (102×31 / 91×31 epx pills; selected pill filled with the accent Light2), and an empty state with a 148 epx illustration and two centred lines. The owner hid Recent. | Three plain collapsible text headers, "Quick access", "Favorites", "Recent", with one-line empty states. | Implement the 26H2 Home: a pill tab selector (Favorites / Shared / Recent when enabled), the illustration empty state, and the accent-filled selected pill. | `desktop/workspace/src/files/home.rs` |
| E8 | Details view | Column header **27.3 epx** (41 px), **no bottom border**, and the first row starts 6 epx below it. Row pitch **28.7 epx** (43 px); hover plate 24.7 epx. Item x +14 epx from the list edge; name text +22 epx from the item; icon 16 epx. Status bar **26 epx** (39 px); "N items" at x 12.7 epx; view toggles 26×26 epx at the right. | Header **34 epx** with a 1 px `#E7EBEE` line under it. Row pitch **30**. Status bar **24**. | Header 27, no border (dividers only on hover). Row pitch 28/29 (`ROW_H` + padding). Status bar 26. | `desktop/workspace/src/files/details.rs` (`ROW_H`, header); `explorer_layout.rs` (`status_height`) |
| E9 | Command bar items | New 86×44.7 epx; icon buttons 40×44.7 epx at a 48 epx pitch, icons 16 epx; Sort 84 epx and View 89 epx (text + chevron); "…" 48 epx; Details 85 epx at the right. Disabled glyphs about 40 % opacity. | Similar order. Buttons 40 epx tall in a 56 epx band. The "View" icon differs (a monitor glyph instead of the list/view glyph). | Fill the band height. Use the Windows View glyph (the three-line "list" icon from the Fluent set used by the View menu). | `desktop/workspace/src/files/modern/paint.rs` (`command_glyph`); `desktop/workspace/src/icons.rs` (`View`) |
| E10 | Details pane | Width 299 epx (448 px). The top preview area (`#222222`) holds a 96 epx thumbnail. Title 18 epx semibold; info card with the "Select a single file…" text and an info glyph. | Toggle exists; not captured in Door. | Capture and compare (missing Door capture). | `desktop/workspace/src/explorer_preview.rs` |
| E11 | View menu (XAML MenuFlyout) | Menu 190 epx wide (285 px), radius **8 epx**, background `#2B2B2B` (dark acrylic fallback), 1 epx border, separators `#3C3C3C` 1 epx. Item height **27.3 epx**, pitch **31.3 epx**, inset 5.3 epx; check-mark column, icon at +39 epx (16 epx), text at +67 px / 44.7 epx, 14 epx. The keyboard focus rect is 2 epx white plus 1 epx black. Items: 8 layouts, "Details pane" / "Preview pane" toggles, and "Show ›". | Not captured in Door. | Capture; check the 31 epx pitch and the check-column offset. | `desktop/workspace/src/files/menus.rs`, `desktop/workspace/src/menu.rs` |
| E12 | New menu | 274 epx wide (411 px). The same item metrics; icon at +11 epx, text at +39 epx. Lists Folder, Shortcut and every registered ShellNew type. | Context submenu "New ›" with Folder / Text Document only (`desktop-files-context.png`). The command-bar New menu was not captured. | Add Shortcut and the known ShellNew types the guest supports; match the flyout metrics. | `desktop/workspace/src/files/menus.rs` |
| E13 | File context menu | The owner forces the **classic** menu (both the Menu key and Shift+F10 show `#32768`). Dark classic: background `#2C2C2C`, border `#3E3E3E` 1 epx, radius ≈ 4 epx (6 px), row pitch **24 epx** (36 px), separators `#3E3E3E`, underlined access keys, 16 epx icons. 26H2's default XAML menu could not be captured. | Modern menu: rows at a 32 epx pitch (✓ against the XAML 31.3), width 270, icon at +21 epx (Windows +16.7), text at +43 epx (Windows +44.7), "Show more options" plus "Customize menu". | Icon inset 17 epx. Verify that Door's **"Show more options" classic menu** uses the dark classic metrics above (24 epx rows, 4 epx radius, access-key underlines). | `desktop/workspace/src/context_menu.rs`, `files/menus.rs` |
| E14 | Tiles view | Item 250×58 epx (375×87 px); icon 48 epx (72 px); three lines (name, type, size) at 14/12 epx; column pitch 254 epx (381 px); row pitch 62 epx (93 px). | Not captured in Door. | Capture and compare. | `desktop/workspace/src/files/details.rs` / icon views |
| E15 | This PC | "Devices and drives" group header 24 epx with a collapse chevron. The owner's view is Large icons (75×106 epx item), so no capacity bars appear. | Not captured. | Capture Door's This PC (default Tiles with capacity bar). | `desktop/workspace/src/files/modern.rs` |

## Task Manager (Door "Monitor")

Windows reference: `taskmgr-screen-initial.png` (Processes, compact navigation, 768×584 epx window).
Other pages were not capturable (see below). **No Door capture of Task Manager exists**, so the
Door values below come from code constants.

| # | Component | Windows (measured) | Door (code) | Fix | Door source |
|---|---|---|---|---|---|
| T1 | Title bar | **52 epx** (78 px), Mica-tinted (`#221D28` here). Icon 16 epx at x≈19; "Task Manager" 12 epx; search box 260×31 epx ("Type a name, publisher, or PID…") in the title bar. | `CAPTION` 48. | 52 epx; put the search box in the caption. | `desktop/system-monitor/src/presentation.rs` (`CAPTION`) |
| T2 | Navigation rail | Compact width **49 epx** (74 px) at this window width, on Mica. Hamburger at the top; items 36 epx tall at a 40 epx pitch (selected plate `#2E2B34`, 3 epx accent pill); Settings at the bottom. | `PITCH` 40 ✓, `ITEM` 36 ✓, `NAV_TOP` 96. | Check `NAV_TOP` and the compact/expanded breakpoint (Windows is compact at 768 epx width). | `presentation.rs` |
| T3 | Page header | "Processes" title (semibold); band 49 epx (52→101) with a 1 epx separator; commands Run new task / End task / Efficiency mode / "…" at the right. | `HEADER_Y` 56. | Header band 52..101. | `presentation.rs` |
| T4 | Column header | Two-line header (percent + label) **51 epx** tall (104→155); sort chevron above the name. | `TABLE_Y` 100. | Header at 104 with a 51 epx two-line block; rows start at 156. | `presentation.rs` |
| T5 | Rows | Pitch **28 epx** (42 px). Group headers "Apps (n)" / "Background processes (n)" 20 epx, taller. 16 epx icons, 12 epx chevrons. The **heat map** uses accent shades (purple here) on CPU, Memory and Disk cells. | `ROW` **32**. | 28 epx rows; drive the heat map from the accent ramp. | `presentation.rs` (`ROW`) |
| T6 | Icon | See P0 item 2. | `branding/monitor`. | Use the Task Manager icon. | — |

## Settings

Windows reference: `settings-{home,display,colors,personalization,taskbar,dateandtime,easeofaccess-display,windowsupdate,about}.png`
plus `.uia.tsv`. The window is maximized at 2560×1528 px (1706 epx wide).
Door reference: `desktop-settings-page-{0..11}.png`, `desktop-settings-{display,about}.png`, and
`desktop-wide-dark-settings.png` (dark, Colors).

| # | Component | Windows (measured) | Door (measured) | Fix | Door source |
|---|---|---|---|---|---|
| G1 | Page background | Mica gradient `#1D2124` → `#271B27` (dark). | Flat `#1F2024` (dark) / `#F4F7FA` (light). | See S3. | `frameworks/ui/src/theme.rs` |
| G2 | Navigation items | Item x **16.7 epx** (25 px), width 279 epx, height 36, pitch 40. Icon 20 epx at +10 epx. Selected plate `#2A2E31`; accent indicator **3×13–14 epx**, `#DB9EE5`. The scroll indicator is hidden at rest. | Item x **8**, width 284, height 36 ✓, pitch 40 ✓. Icon at +13. Selected plate `#2E3034` (dark) / `#E6E9ED` (light). Indicator **16** epx tall, `#60CDFF`. A **persistent 2 epx grey scrollbar** sits at x 296. | Inset the items to x 16–17, width 279. Indicator 3×14 epx with accent Light2. Auto-hide the pane scrollbar. | `desktop/workspace/src/settings_navigation.rs`, `settings_layout.rs` |
| G3 | Account card | 289×75 epx at (6, 49), avatar 60 epx, name 14 epx semibold, email 12 epx. | Equivalent ("Local account / User 1001"). | — | `settings_layout.rs` |
| G4 | Title | Breadcrumb "System › Display" in Segoe UI Variable Display semibold **28 epx**, text box height 37 epx, at y 51 epx. | Same size and width ("Display" is 92 epx on both). | ✓ | — |
| G5 | Setting cards | Max width **1000 epx**. Min height **69.3 epx** (104 px). Gap 4 epx. Radius 4 epx. Border 1 epx (`#1A1E20` dark: darker than the page). Fill `#292C2F`–`#292B41` (translucent over Mica). Text column at +59 epx; title 14 epx / 18.7 epx line; description 12 epx / 16 epx line. Chevron button 32×32 epx; toggle 40×20 track. | Height **66–67**, gap 4–5 (pitch 72; Windows 73.3). Radius 4 ✓. Border 1 px ✓ (`#1A1B1E` dark, `#E4E6E9` light). Fill `#2B2C2F` / `#FBFCFD`. | Card min height 68 + 1 epx border on each edge (≈ 70 total); pitch 73–74. Translucent card fill over Mica: dark `#FFFFFF0D`, light `#FFFFFFB3`. | `desktop/workspace/src/settings_layout.rs`, `frameworks/ui/src/controls/containers.rs` |
| G6 | Section header and gaps | Section header 14 epx semibold. Header → first card 26.7 epx. Last card → next header **31 epx**. | Header → card ≈ 20; card → header ≈ 33. | Match the 26.7 / 31 rhythm. | `settings_layout.rs` |
| G7 | Search box | 515×32.7 epx pill at y 8. Fill `#2A2E31` with a 1 epx lighter top edge `#323639`. Focused: accent underline. | 516×32 ✓. Fill `#37383B` (dark), border `#424346`: too light. | Dark fill = ControlFillColorDefault over Mica (≈ `#2A2E31` here); bottom stroke on focus = accent. | `settings_layout.rs` |
| G8 | Caption buttons | 46×32 epx (69×48 px). | 46-epx pitch ✓. | ✓ | — |
| G9 | Home | At the 1706 epx window width: **2 columns** (breakpoint not measured), "Recommended settings" + "Bluetooth devices" / "Personalize your device" (theme thumbnails 87×70 epx, Color mode, Browse more…). "Get help" / "Give feedback" links at the bottom. The device card shows the wallpaper thumbnail 93×59 epx and the name 20 epx semibold. | Single column at 1280 (matches Windows' narrow layout). Device card ✓. | Verify the 2-column breakpoint at wide widths (missing Door wide capture of Home). | `desktop/workspace/src/settings/pages.rs` |
| G10 | Personalization > Colors | Theme **preview image** (195×122 epx) at the top. Mode, Transparency, **Accent color expander**: Manual/Automatic dropdown, Recent colors (5 swatches), **Windows colors grid 48 swatches, 39 epx at a 42 epx pitch**, Custom colors "View colors". Then "Show accent color on Start/taskbar" and "title bars". | No preview. Accent row disabled ("fixed in this build"). The two "Show accent color" toggles are disabled. | Implement it (see S2). | `settings_catalog.rs:2978`, `settings/pages.rs` |
| G11 | Personalization > Taskbar | Expanders: Taskbar items (Search mode dropdown, Task view, Resume), System tray icons, Other system tray icons, Taskbar behaviors with a **visual "Taskbar position" picker** (4 thumbnails 47×31 epx: Bottom / Top / Left / Right), alignment, **Taskbar size** (Default / Small), and the rest. | Present (`desktop-controls-*`), with position as a dropdown. | Use the thumbnail picker for position. | `settings/pages.rs` |
| G12 | Time & language > Date & time | A **large clock header** ("5:20 PM" 28 epx semibold + long date) with Time zone and Region at the right. Rows: auto time zone, auto time, set manually (Change), show time and date in the tray (expander), format, show time in Notification Center, Sync now, additional calendars, related links. | The category landing page exists (`page-7`); the Date & time subpage was not captured. | Capture and compare; add the clock header. | `settings/pages.rs` |
| G13 | Accessibility > Text size | "Text size preview" card (720×110 epx), slider "A ——— A" with the % value bubble, **Apply** button, Related settings, Related support (expander). | Not captured. | Capture and compare. | `settings/pages.rs` |
| G14 | Windows Update | Header: 48 epx sync icon + status ("Updates failed / Last checked") + "Retry all" accent button. List of available updates, **More options** (Get the latest updates toggle, Pause updates "Pick a date", Update history, Advanced options, Windows Insider Program), carbon note, Related support. Nav label "**Windows Update**". | "Update & recovery" page ("Updates unavailable"); Home tile "Doors Update". | Naming choice (trademark); match the layout (status header + More options list). | `settings_catalog.rs` |
| G15 | System > About | 26H2: a **row of 4 hardware tiles** (Storage, Graphics card, Installed RAM, Processor; 156×103 epx), then a name card "(account) / (device name)" with **Rename this PC**, then Device specifications (expander with **Copy**) and a **Related links** row (Domain or workgroup, System protection, Advanced system settings), then Windows specifications. | A hero image card (wallpaper thumbnail + name), Device specifications with Copy, Doors specifications. No hardware tiles and no related-links row. | Add the 4 tiles (data that is unavailable shows as "—") and the related-links row; drop the hero image on About. | `settings/pages.rs`, `settings_catalog.rs` |
| G16 | System > Display | "Scale" row has a dropdown **and** a › chevron button (custom scaling). Brightness slider row. Night light / HDR toggle + chevron rows. A "Related settings" section. | Scale row has a dropdown only; resolution/orientation are read-only values. | Add the chevron to Scale. | `settings/pages.rs` |
| G17 | Text antialiasing | Grayscale. | Subpixel. | See S4. | — |

## Shell: taskbar, Start, Search, Quick settings, notification center, Win+V, Alt+Tab

Windows reference: `taskbar-strip.png`/`taskbar.uia.tsv`, `shell-*-screen.png`/`shell-*.uia.tsv`.
These were captured over the owner's Claude window, which therefore appears in the background.

| # | Component | Windows (measured) | Door (measured) | Fix | Door source |
|---|---|---|---|---|---|
| H1 | Taskbar | Height **48 epx**. App buttons **44 epx** wide (66 px); icons **24 epx**; Start button 45.3 epx. Clock 70 epx wide, two lines. Show-desktop strip 12 epx; Notifications 24 epx. Top edge: a 1 epx light line (`#3F4545` dark). Dark acrylic tinted by what is behind it. Active-app indicator: 16×3 epx accent pill (`#DB9EE5`); running: short grey pill. | 48 ✓, button pitch 44 ✓, icons 24 ✓. Top line `#CBD5E4` 1 px (light). Active indicator `#005FB8` 3 px ✓. Shows a search **box** (the owner uses icon-only: a config difference, not a bug). | Mostly matches. Check the indicator lengths (active 16, running 6 epx) and the accent colour (S2). | `graphics/window-server/src/taskbar.rs`, `material.rs` |
| H2 | Start | At 1706×1066 epx: panel **833×865 epx**, bottom 12 epx above the taskbar, radius 8 epx. Top search band ≈ 65 epx with a **pill search box 768×32 epx**. "Pinned" header + "Show less ⌃". Pinned grid **8 columns at a 96 epx pitch**, 32 epx icons. "All" + "View: Category ⌄", category folders (153 epx tiles, 2×2 icons + label). Bottom account bar 64 epx (avatar 28 epx + name; power at the right). | At 1280×720: 640×648. Top band 65 ✓, search 576×32, Pinned pitch 96 ✓ (6 columns), "View: Category" ✓, category tiles ~150 ✓, bottom bar 64 ✓. Shows "Recent". | Verify the Automatic width rule against 26H2 (Windows widens to 8 columns on large screens). No Door capture exists at 2560×1600 150 % with Start open (only `desktop-*-native-start-icons`). | `graphics/window-server/src/start.rs` |
| H3 | Search (Win+S) | The same panel size as Start (833×865 epx). Search box 768×32 epx. Two columns: **"Recent"** list (24 epx icons, rows at a 46 epx pitch) and, on the right, **"Quick searches"** (a card of pill buttons: Focus/Sound/Bluetooth/Display/Color/Search settings) plus **"Top apps"** as a 3×2 grid of **tile plates 146×91 epx** (32 epx icon + label). Account initial and "…" at the top right. No filter pills before typing. | A filter pill row (All / Apps / Documents / Web / Settings / Folders / Photos) is always shown. Recent list ✓. "Top apps" as bare icons (no plates). No "Quick searches". | Hide the filter pills until text is typed; add the Quick searches card and the Top apps tile plates. | `graphics/window-server/src/search.rs`, `start.rs` |
| H4 | Quick settings (Win+A) | Panel **360×388 epx**, 12 epx from the right and the taskbar, background `#292929`, 1 epx border `#3A3A3A`. Tiles **96×48 epx** at a 108 epx pitch, labels below. On: accent fill `#DB9EE5`; off: `#363636` with a `#494949` border. Sliders with an accent fill and an accent thumb. Footer **48 epx** `#1F1F1F` with battery % at the left and a settings gear at the right. Vertical page dots + "▾" at the right edge. | 360×386 ✓, tiles 96×47 ✓, footer ✓ (gear only). Tiles and sliders are disabled (no hardware: correct). No page dots. | Add the page-dot / edit affordance; set the accent per S2. | `graphics/window-server/src/tray.rs` |
| H5 | Notification center (Win+N) | Panels **336 epx** wide (504 px), 12 epx right margin, 8 epx top, **12 epx gap**. Notifications panel: header "Notifications" 14 epx semibold, **DND toggle button 40×40 epx**, "Clear all" 62×24 epx, per-app headers (icon + name, 29 epx), cards with image previews. Calendar panel: header "Monday, October 26" + collapse button (24 epx), month header 36 epx with ▲▼, weekday row, **day cells 41×41 epx at a 47×42 epx pitch**, today = an accent circle. **Focus footer**: "− 30 mins +" and "▶ Focus" (72×24 epx). Calendar panel height 457 epx. | Panels **360** wide, gap 14. Calendar 360×404 with a "UTC" label and no Focus footer. Day pitch 46×44. "No new notifications" empty state only. | Width 336, gap 12, day pitch 47×42, add the Focus footer and the DND / Clear all header buttons. | `graphics/window-server/src/tray.rs` (`flyout_sized(360, …)`, `CALENDAR_HEIGHT`) |
| H6 | Win+V | An **"Emoji and more" panel**, ≈ 357×397 epx, at the caret/centre: a header "Emoji and more" + mode chips; a tab row (Recently used, Emoji, GIF, Kaomoji, Symbols, Clipboard), with an accent underline on the active tab. With history off, the Clipboard tab shows an illustration, "Let's get started", body text and a **Turn on** button. | Clipboard history panel only (no Door capture). | Capture it. Add the tab row (Emoji, Kaomoji and Symbols are local; GIF needs a network) and the "history off" empty state. | `graphics/window-server/src/clipboard_view.rs`, `clipboard_history.rs` |
| H7 | Alt+Tab | Panel spans ~**83 % of the screen width** (1408×350 epx at 1706 epx), a grey acrylic plate, radius 8. Each window is a **header (16 epx app icon + title 12 epx) above the thumbnail**, with no card plate; thumbnails are 238 epx tall. Selected window: a **3 epx accent ring** (rounded 8) around the header and thumbnail. The close button (40×39 epx) shows only on hover/selection. | Compact panel. **White card plates** with title text, an **always-visible ✕** and **no app icon**; blue selection ring ✓. | Drop the plates, add app icons, show ✕ only on hover/selection, size the panel to the screen width rule. | `graphics/window-server/src/overview.rs`, `multitask.rs` |

## Notepad (Door "Notes")

Windows: `notepad-window.png` (1271×741 epx). Door: `desktop-focus-notes-ready.png`.

| # | Component | Windows | Door | Fix | Door source |
|---|---|---|---|---|---|
| N1 | Tab strip | Height **41 epx**; tab 246×32 epx at y 9; app icon in the caption at x 25. | 38 epx; app icon ✓. | 41/32. | `desktop/workspace/src/document_tabs.rs`, `caption_tabs.rs` |
| N2 | Menu + format bar | One 33 epx band: File / Edit / View (41–50 epx items). The centred format toolbar has **H1 ⌄, list ⌄, table ⌄ drop-downs (56 epx)**, B / I / S / link (32 epx) and **Clear formatting**. At the right: **What's new** (with a badge dot) and Settings. | No ⌄ chevrons on H1, list or table. No Clear formatting. No What's new. | Add the drop-down chevrons and Clear formatting; What's new is optional. | `desktop/workspace/src/notes.rs`, `document_layout.rs` |
| N3 | Status bar | 32 epx: "Ln 1, Col 1", "25 characters", "Plain text", zoom "100%", "Windows (CRLF)", "UTF-8", with thin separators. | ✓ (Door shows "Unix (LF)" for LF files: correct). | Check the height (Door ≈ 32). | `document_view.rs` |

Applied 2026-10-27 from the owner's dark screenshot (`desktop/workspace/src/document_format.rs` `Chrome`, painted by
`notes.rs`): 41 px tab row with the app icon, 246 x 32 tabs (unsaved dot that turns into the close glyph on hover) and
"+"; one 32 px band with File / Edit / View, the centred toolbar (H1, list and table drop-downs with chevrons, B, I, S,
link, Clear formatting) and What's new (accent dot) + Settings; 16 px text padding, first row at 89; a 32 px status bar
"Ln, Col | n characters | Aa Formatted" and right-aligned zoom | line ending | encoding with 1 x 16 px separators.
Colours: Mica caption, LayerOnMicaBaseAlt band/tab/status layer, #272727 editor (dark), #CDCDCD status text. The toolbar
edits Markdown source (the file stays plain text); Formatted mode dims markup and shows bold/headings, italic, strike,
links, bullets and table rules; the status toggle switches to Plain text. Ctrl+B/I/K, Ctrl+Shift+X and Ctrl+Space apply
the same edits. The TEXT-01/desktop-notes points follow (`tests/notes_scenario_points.rs`, xtask `desktop/notes.rs`).

## Windows Terminal (Door "Terminal")

Windows: `terminal-window.png`, `terminal-shadow-white.png`. **No Door Terminal capture exists**, so
compare after capturing. Reference: caption/tab strip **40 epx**, tab 248×32 epx (icon 16 epx at +13,
title at +39), "+" / "⌄" split button 64×24 epx, caption buttons 46 epx, text area `#0C0C0C`,
scrollbar 16 epx (hidden at rest). Source: `desktop/workspace/src/terminal*.rs`.

## Photos viewer (Door "Preview")

Windows: `photos-window.png` (1268×745 epx). **No Door Preview capture exists.** Reference: the title
bar is **48 epx** and holds the **Edit** button (73×36 epx) and **Rotate / Delete / Print / Share / …**
(44×48 epx each), the file name centred (12 epx), an **"Open with ⌄"** drop-down (109×32 epx) and caption
buttons 48×48 epx. Image area `#202020` with a **Next ›** arrow at the right edge (rounded 24×46 epx
plate). The bottom bar is **32 epx**: filmstrip, slideshow, **file info "JPG | 3840 x 2400 | 1.5 MB"**,
info, scan text, view actual size, **zoom combo (88 epx) "41%"**, zoom −, **slider (130 epx) with an
accent thumb**, zoom +, full screen. Source: `desktop/preview/src/presentation.rs`.

## App icons (reference only)

`reports/artifacts/reference-26h2/icons/{taskmgr,photos,notepad,explorer,settings,terminal}-shell-{16,24,32,36,48,64,96}.png`
come from `IShellItemImageFactory` on the shell AUMIDs, which is exactly what Start, the taskbar and
Search draw. `taskmgr-assoc-48.png` and `explorer-assoc-48.png` come from `ExtractAssociatedIcon`.
These files are for comparison only and are never shipped. Door already ships owner-installed
references for File Explorer, Notepad, Settings and Terminal (`assets/icons/windows/*`). At the shell's
24/32/48 px sizes those match the Windows silhouettes, so only Task Manager and Preview differ
(see P0).

---

## Missing features and pages

1. Accent colour personalization (picker, palette, propagation), and the Colors page preview image and swatch grid.
2. Mica for the client backgrounds, plus active/inactive material switching.
3. Explorer Home 26H2 pill tabs (Favorites / Shared / Recent), the illustration empty states, and Quick access tiles with 48 epx icons.
4. Explorer: Shortcut and the ShellNew types in New; a Large-icons This PC with a capacity bar (Tiles); a dark-theme pass.
5. Notification center: the Focus session footer, the DND and Clear all header buttons, and app-grouped notification cards.
6. Search: the Quick searches card, Top apps tile plates, and filter pills only after typing.
7. Win+V: the "Emoji and more" container (Emoji, Kaomoji, Symbols tabs) and the "clipboard history off" state.
8. Alt+Tab: app icons in the window headers, hover-only close, and full-width panel sizing.
9. Settings: the About hardware tiles and Related links row; the Date & time clock header; the Taskbar position thumbnail picker; the Text size preview and Apply; the Display > Scale chevron; Windows Update "More options"; Home 2-column layout verification.
10. Notepad: the heading/list/table drop-down chevrons and Clear formatting.
11. Task Manager: the 52 epx caption with the search box, 28 epx rows, and the accent heat map.
12. Window shadow: asymmetric and larger (S1).

## Door captures missing (needed to finish the diff)

- Task Manager (Monitor): every page, with the navigation collapsed and expanded.
- Preview: viewer and gallery.
- Terminal.
- Explorer: dark theme, This PC, Tiles view, details pane, command-bar New and View menus, and Home with Quick access expanded at 100 %.
- Settings: Text size, the Date & time subpage, Home at a wide width, and Colors in light theme.
- Win+V panel.
- Start and Search at 2560×1600 150 % (to compare the Automatic size).
- No `display01-*` or `settings-reference*` captures exist under `reports/runs/`.

## Not captured on Windows, and why

- **Task Manager pages other than Processes, and the navigation collapse/expand.** `Taskmgr.exe`
  re-launches itself **elevated** (silent auto-elevation), even when started with
  `__COMPAT_LAYER=RunAsInvoker` or `runas /trustlevel:0x20000`. UIPI then blocks PrintWindow, UIA,
  input and `WM_CLOSE` from the non-elevated capture shell. Only the screen capture of the
  initial page (Processes, compact navigation) was possible. The two elevated windows left open
  were closed by the owner.
- **The 26H2 XAML file context menu and "Show more options".** The owner's registry override forces
  the classic menu, and changing it is a setting change. The XAML MenuFlyout metrics come from
  Explorer's View and New menus instead.
- **Light theme.** It is not switchable without changing settings.
- **Inactive window shadow and caption-button hover states.** The inactive state was not
  reproduced on the white backdrop, and UIA could not resolve Explorer's caption buttons. The
  Notepad shadow capture was blank (its window never became foreground) and was deleted.
- **Animations and motion timings.** Only static captures were taken: no frame-timed recording of
  Start, flyouts or snap.
- **Clipboard history contents.** History is off on this PC; the "Let's get started" state was captured.

Side effects on the owner's PC: Explorer windows opened in the background, Settings and Terminal
took the foreground briefly, and two foreground bursts ran only after the owner had been idle for
90 s or more. Every window opened by this work was closed (the two Task Manager windows by the
owner). The test files are in `reports/artifacts/reference-26h2/sample-folder/`. Changing that folder's view
to Tiles wrote the usual per-folder Explorer view state ("Bags"). No settings were changed.

## Applied: compositor and shell (2026-10-31)

| Row | Change | Where |
|---|---|---|
| S1 | Window shadow: two Gaussian layers fitted to the captured profiles (ambient + key offset 36.8 epx down; RMS error 0.29 % active, 0.14 % inactive), support 52/35/52/89 epx, masks cached per clamped size, scale and state. Host test compares every profile sample with the captures (±1.5 %). | `graphics/compositor/src/shadow.rs`, `assets/raster/gaussian-cdf.u16` |
| S3 | Mica: `MATERIAL_MICA` (SET_MATERIAL value 2): the wallpaper grid blurred further, moved to the tint luminosity and tinted (dark #202020 80 %, light #F3F3F3 50 %), once per wallpaper and theme; inactive or reduced transparency use the solid fallback; also behind the trusted title bar. Apps opt in. | `engine/material.rs`, `render.rs`, `specs/contracts/native-window-v1.md` |
| S2 | System accent: `Accent` setting word (Default / Manual RGB / Automatic from the wallpaper), `AccentColor` storage setting, Init binding, `SET_ACCENT` (0x502d), APPEARANCE_CHANGED `a` carries the resolved colour, palette derivation (Light3..Dark3) and Windows swatches in `door_design_tokens::accent`; the shell palette, Task view and Alt+Tab follow it. | `service-protocol`, `storage-core`, `services/src/init/desktop.rs`, `paint.rs` |
| S4 | Text targets default to grayscale; `Target::cleartype()` keeps ClearType for Win32 surfaces (desktop labels, server-drawn captions, GDI, Run dialog). | `frameworks/text/src/target.rs` |
| H5 | Calendar and notification centre 336 wide; calendar 457 tall with the darker date band, 47 x 42 day pitch, 41 px cells and the Focus footer; DND 40 x 40 and Clear all 62 x 24. | `tray.rs`, `shell_view.rs` |
| H4 | Quick settings page dots and chevron. | `tray.rs` |
| H3 | Search home: no filter pills before typing, Quick searches card (Settings pills), Top apps plates. | `start.rs` |
| H7 | Alt+Tab 26H2 items (header + thumbnail, accent ring, hover close); layout shared with the guest probes. | `overview.rs` |
| — | Desktop Recycle Bin (installed imageres art, empty/full from File Explorer's `SET_RECYCLE_BIN`), Open / Empty Recycle Bin / Properties through Explorer. | `desktop.rs`, `apps.rs`, `menus.rs` |
| Motion | Window tracks from `door_design_tokens` (point-to-point curve for snap and maximize); Start/flyout tables tied to the token splines; MOTION-01 reads the same constants. | `animation.rs`, `motion.rs`, `motion_timing.rs` |

Still open: apps opting into Mica and painting transparent page backgrounds; the
Settings accent picker and clients applying APPEARANCE_CHANGED `a`; ClearType for the
Explorer tree and items view (`Target::cleartype()`); the bin state before Explorer
first runs; a Focus session backend; a frame-timed Windows motion capture; toasts and
Start's Automatic width were not changed.
