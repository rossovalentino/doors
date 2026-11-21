# P14 inbox apps — 1 November 2026

Owner direction (P14): every app Windows 11 26H2 ships in the box gets an
identical GUI, its official icon and real function. One protected image,
`bin/door-inbox.elf` (crate `desktop/inbox`, package `door-inbox`), runs the app
its kernel-authenticated manifest names (`door_abi::platform::manifest::INBOX`,
policy: session user, no capabilities, one thread, 96 MiB). Init launches each
as an accessory (`application::ACCESSORY_COMMANDS` 14–22) at most once per
session; the window server lists them (`apps::Tool`), the desktop shell gives a
running one its taskbar group, Task Manager names it, and Start's All list
shows it (`apps::Entry`; the component gallery stays search-only).

| Manifest | Command | App | Reference |
|---|---|---|---|
| 20 `CALCULATOR` | 14 | Calculator | Microsoft.WindowsCalculator 11.2607 |
| 21 `CLOCK` | 15 | Clock | Microsoft.WindowsAlarms 11.2607 |
| 22 `PAINT` | 16 | Paint (with Paint 3D's 3D shapes) | Microsoft.Paint 11.2605.81 |
| 23 `SNIPPING_TOOL` | 17 | Snipping Tool | Microsoft.ScreenSketch 11.2608.41 |
| 24 `WEATHER` | 18 | Weather | Microsoft.BingWeather 4.54 |
| 25 `MAGNIFIER` | 19 | Magnifier | Magnify.exe (26H2) |
| 26 `NARRATOR` | 20 | Narrator Home | Narrator.exe |
| 27 `ON_SCREEN_KEYBOARD` | 21 | On-Screen Keyboard | osk.exe |
| 28 `VOICE_ACCESS` | 22 | Voice access | voiceaccess.exe |

Live captions is the existing `door-assist` Captions client renamed (manifest
`ASSIST`, command 13).

## Assets (installed-reference acquisitions, local development only)

App icons: `cargo xtask assets acquire-icon` from each package's unplated
AppList PNGs or the executables' icon groups (`Magnify.exe.mun`). Paint's
toolbar artwork: the package's per-theme SVGs (`assets acquire-paint-icons`,
67 icons, rasterized offline to 16/32/48 px and 20/40 px for the large ones).
Glyph fonts beside Segoe Fluent: Paint's `PaintIcons.ttf` (`"font": "paint"`)
and Snipping Tool's `SnSkFluent.ttf` (`"font": "snip"`), each pinned by
`provenance.json`. None of these is redistributed.

## Apps

**Paint** (`desktop/inbox/src/paint`). Measured at 150%: 32 px title, 36 px menu
bar (File, Edit, View; Save, Share, Undo, Redo; feedback, Settings), 110 px
ribbon card (Selection, Image, Tools, Brushes, Shapes, Colors, 3D shapes,
Layers), the canvas over the tinted backdrop with the floating size/opacity
sliders, and the 46 px status bar. The canvas is real (`paint/raster.rs`):
layers (up to six; add, duplicate, merge down, delete, hide), pencil, nine
build-up-free brushes, the eraser, 23 shapes with outline/fill, flood fill with
tolerance, text, colour picker, magnifier, rectangle and free-form selections
that move, cut, copy, paste (system clipboard images too, such as a snip) and
crop, rotation and flips, resize and skew, canvas resize from eight handles,
Remove background (edge-connected background to transparency on a new layer),
Paint 3D's lit 3D shapes (cube, sphere, cylinder, cone, pyramid, capsule; exact
ray intersections), zoom 12.5–800%, rulers, gridlines, a byte-bounded undo
history, PNG save to Pictures and Open of PNG/JPEG/BMP/QOI from the user's
folders, and the "save your work?" prompt. Copilot's cloud features are not
offered; the 3D shapes group takes its place, joining Paint and Paint 3D.

**Snipping Tool** (`snipping.rs`) and the shell's **snip overlay**
(`graphics/window-server/src/snip.rs`, `snip_glue.rs`). Win+Shift+S, Print
Screen or the app's New (`window::SNIP_BEGIN`, Snipping Tool's manifest only,
with a 0/3/5/10 s delay) freezes the screen once the requester's windows have
minimized, dims it and shows the capture bar (Rectangle, Window, Full screen,
Free-form, Close). The snip goes to the clipboard as `clipboard::IMAGE`; the
shell restores or launches Snipping Tool and sends `SNIP_READY` once it has
focus (clipboard reads need it). The app saves it automatically to
Pictures\Screenshots, grows to fit, and offers the ballpoint pen (eight
colours), highlighter, eraser (markup layer only), crop, undo/redo, zoom,
Save as, Copy. Recording and text actions say they are unavailable.

**Weather** (`weather.rs`). MSN Weather 4.54's chrome (accent title bar,
48 px rail, header with Refresh, Favorite, Pin, units and "Search for
location"). Without a network (P15) every forecast page says "You're offline"
instead of inventing weather; Settings (units, mode) and Favorites work.

**Magnifier** (`access.rs`; shell `magnify.rs`, `access_glue.rs`). The toolbar
(zoom out, level, zoom in, Views: Docked/Full screen/Lens, settings). The
shell magnifies its composed output (`window::MAGNIFY`, Magnifier's manifest
only): full screen follows the pointer and moves the cursor plane with the
view; lens and docked overlay a framed magnified region. The view ends when
the Magnifier process exits.

**On-Screen Keyboard**. osk.exe's layout (main block and navigation block,
one-shot Shift/Ctrl/Alt/Win, Caps Lock). Keys reach the focused window through
`window::INJECT_KEY` (its manifest only), taking the session layout's
translation like physical keys; Win chords go through the system-key policy.
The keyboard's windows never take focus (press activation is skipped for them).

**Narrator** and **Voice access**. Narrator Home and the Voice access bar.
Speech output and a microphone need P15 audio; both say so plainly.

## Shell follow-ups in the same pass

**Run (Win+R)** is Windows 11 26H2's modern Run dialog (the WinUI 3 rebuild
Microsoft published in April 2026, light and dark): one 500 x 62 acrylic bar
24 px from the work area's left and 16 px above the taskbar with the Run icon
(shell32.dll group 25), "Type the name of a command to run" and a Run button
(accent once there is text; no Browse). Recent commands and suggestions with
icons list above the bar. Commands: Windows executable and URI names of Doors
apps and inbox tools (`calc`, `mspaint`, `snippingtool`, `osk`, `magnify`,
`narrator`, `ms-clock:`, `ms-screenclip:`, ...), folders and paths in File
Explorer (`~\`, `C:\...`, `%USERPROFILE%`, `shell:` folders, profile-relative
names, resolved by Explorer's address bar via `Open::Typed`), and every legacy
Control Panel / `*.cpl` / `*.msc` / `ms-settings:` entry point redirected into
the unified Settings by `door_settings_map`. Unknown commands keep the dialog
open with Windows' error text.

**File operations** run inline for up to 100 ms when they start, so copying,
moving or deleting a few items completes with no dialog; work still running
after a second shows Windows 11's copy dialog (447 px window, title bar with
the clock icon, "NN% complete", disabled maximize and close; summary with
folder links, pause and cancel, the green bar, More details), which opens and
closes with a fade-and-scale approximating the DWM transitions (timings are
estimates, not measured). Closing it cancels its operations.

## Verification

Host: `cargo test -p door-inbox` (raster engine, Paint tools/undo, Snipping
Tool markup), `cargo test -p door-window` (snip crop and free-form mask,
magnifier planning, shortcuts). Guest: `p14-inbox` launches every app from
Start search, draws in Paint and answers its save prompt, takes a rectangle
snip through Snipping Tool's New and the overlay, magnifies to 200%, injects a
key from the On-Screen Keyboard and closes each app with exit code 0.

## Known limits

- Snipping Tool: no screen recording or OCR; the snip clipboard read is
  chunked through IPC (slow for full-screen 1080p snips).
- Paint: images up to two megapixels; no Copilot/Cocreator; text is one font.
- Magnifier: no "read from here" (needs audio); docked view overlays rather
  than reserving screen space.
- Weather/Narrator/Voice access: honest unavailable states until P15.
- Clock keeps UTC for alarms until the apps read the user's time zone.
