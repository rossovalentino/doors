# Native development desktop

Settings now includes four offline3840x2400 backgrounds and six selectable themes.
Personalization keeps light/dark mode independent of the current wallpaper.
Page Up/Down or the scroll thumb reaches the theme grid; changes save automatically
and survive boot. Apps > Installed apps opens the bundled applications. These are foundation
consumers; networking, pairing, audio, updates and runtime DPI remain unavailable.

P11 passed its declared software-display gate and independent critic. Native QEMU
windows support typing, durable Notes save/reopen, window management and server
recovery. P12 app/control refinement is active and partial; Windows fidelity is
unqualified.

From the repository directory:

The latest independently built desktop can be run immediately in PowerShell:

```powershell
Set-Location 'C:\Users\win\Desktop\Doors OS\door-os'
cargo xtask desktop
```

The interactive launcher now selects WHPX/RAMFB and requires Windows Hypervisor
Platform. It opens a normal1280x720 QEMU window and stops at Sign in. Press Enter to enter
the desktop. Win+I opens Settings; Win+Tab opens Task View. Rebuild and run the
current native workflows when source changes require a new image:

```powershell
cargo xtask test desktop-current
cargo xtask desktop
```

The owner launcher uses existing `out/session/user.raw`. The current new volume is
ready. On a fresh checkout, `cargo xtask init-user-volume` creates it explicitly
without replacing existing data; see [first-run/debug instructions](desktop-first-run.md).
Never use the owner disk as a regression
fixture. `cargo xtask test desktop --image <relative-image.json>` makes its own disk.
Development relinks may reuse a previous `desktop-build.json` with
`cargo xtask build --profile desktop-dev --from <relative-desktop-build.json>`.
The tool rejects changed kernel/firmware transitive inputs; this fast path is
explicitly unqualified. Full verification uses independent complete builds.
For the complete cumulative gate, build the host orchestrator in release mode
with `cargo build -p door-xtask --release --locked --target-dir target/p11-host`,
then use `.\target\p11-host\release\door-xtask.exe` for the same commands.
The P11 cumulative gate is historical; P12 full verification is not implemented
and fails closed. Focused desktop runs do not close P12.

The primary fidelity reference is now the installed Windows11 baseline in
`docs/design/windows-core-apps.md`. The current Explorer still has the older
command row above address/search, a150px navigation pane and standard folder
tiles; its missing modern tabs/navigation ordering remain tracked work.
The Door/taskbar launcher opens or activates the three native applications; Windows key, arrows/Tab,
Enter and Escape operate it. The full P14 shell remains incomplete.

File Explorer displays actual guest paths. Standard folders and Home navigate
real directories; click a selected directory or press Enter to open it. Up/
Alt+Up goes to the parent; Alt+Left/Backspace and Alt+Right navigate the bounded
32-location history. Refresh/F5/Ctrl+R refreshes; New folder/Ctrl+Shift+N creates a
durable directory. Selecting/opening a regular file shows a bounded text preview.
Ctrl+L replaces the address; Enter navigates. Ctrl+F/F3 edits a bounded directory
filter; Enter applies it and Escape clears it. Empty results cannot activate a
hidden item. Sort toggles name order; View switches the home overview/list.
More/context > Shut down uses the authenticated power broker. F2, command-bar
Rename and modern/classic context Rename perform a durable checked rename.
Enter saves and Escape cancels; the initial selection preserves a file extension.
Errors retain the edit. Known-folder relocation is unavailable until pinned
associations can follow it. Clipboard, Share and Delete remain disabled.
Notes edits `/users/1001/notes/desktop.txt`, supports caret movement, Home/End,
Backspace/Delete and a4096-byte ASCII document. Ctrl+S or File > Save writes and
flushes; Ctrl+O or File > Reopen reloads saved bytes. Word wrap toggles in the
menu row. Right-click or Shift+F10/Menu opens command menus in both apps;
Up/Down selects, Enter invokes, Escape or an outside click dismisses.
Closing Notes attempts save and cancels the close
on failure, retaining the dirty document. Operation errors
remain visible.

Native title bars support move, border resize, minimize, maximize/restore and
close; Alt+Tab switches/restores windows and Alt+F4 or Ctrl+W requests close. Files
also supports Home/End and PageUp/PageDown selection. The pointer changes to a text
I-beam in Notes and to the appropriate resize direction at each window edge/corner.
These are P11 consumers, not the complete P12 toolkit or P14 Explorer/shell. A Windows EXE with
User32/GDI requirements still needs P13. The GOP CPU backend does not provide VSync.

See `specs/contracts/native-window-v1.md`, the two graphics-crate READMEs and
`docs/design/windows11-reference.md`. The historical P11 profile used Selawik, an OFL font substitution; its
provenance is `assets/fonts/selawik/provenance.json`. A pixel-identical Windows 11
claim requires a frozen Windows reference configuration and measured comparisons.

## Current P12 desktop development behavior

Run from PowerShell in the repository:

```powershell
Set-Location 'C:\Users\win\Desktop\Doors OS\door-os'
.\target\p11-host\release\door-xtask.exe desktop
```

The visible launcher opens a normal1280x720 host window. It never requests
fullscreen automatically or selects the whole monitor at100% DPI. This keeps
current fixed-pixel controls usable without enlarging the rendered framebuffer.
Use Ctrl+Alt+F to toggle fullscreen explicitly; Ctrl+Alt+G releases mouse/keyboard.
Close the host window to exit QEMU; use guest Shut down to flush work first.
The guest explicitly falls back to1024x768 if firmware lacks the requested mode. A mismatched requested
mode can still be letterboxed by QEMU. Dynamic guest display resize and fractional DPI changes are not implemented. Exact16/20/24/32/40px premultiplied BGRA icon exports avoid runtime nearest
neighbor resampling. Original SVG app branding remains resolution independent
at export; the wallpaper uses its lossless3840x2400 source. Outputs2560pixels wide
use native200%;3840x2160 uses native300%/288DPI and1280x720 logical geometry.
Smaller outputs use96DPI. Glyphs and icons are rasterized at each physical density.

Desktop shortcuts select on the first click, with a blue icon-and-label rectangle.
Double-click within500ms opens; a delayed second click only selects. Clicking
blank desktop clears selection. Up/Down and Enter operate a selected shortcut
when no app has focus; Escape clears it. Start and pinned taskbar apps still
activate on one click. Selection/hover repaint a bounded desktop strip when it
is unobscured, preserving overlapping windows and shadows.
App admission and catalog replies are polled without sleeping in the compositor;
full framebuffer transfer also services input between32KiB batches. Composition
and capture remain CPU work; these changes do not establish a frame-rate guarantee.

Explicit modes, including Full HD and4K:

```powershell
cargo xtask desktop --resolution 1920x1080
cargo xtask desktop --resolution 2560x1600
cargo xtask desktop --resolution 3840x2160
```

Also supported:1024x768,1280x720,1920x1200,2560x1440. The display device
uses64MiB VRAM for these modes. The1280x720 regression remains explicit.

The interactive hardware-assisted profile uses WHPX with a RAM-backed framebuffer,
one CPU and2GiB. It passed the isolated native4K regression on this host; ordinary
cumulative regressions retain TCG. It requires Windows Hypervisor Platform and the
existing owner volume. `--accelerator whpx` remains an explicit equivalent:

```powershell
cargo xtask desktop --accelerator whpx --resolution 3840x2160
```

Software CPU emulation remains available explicitly:

```powershell
cargo xtask desktop --accelerator tcg
```

Options may be ordered freely, including `--headless`; duplicates, missing values
and unsupported modes fail before any image/storage access. The launcher does not
change host features or switch backends after a failed launch. It records the
selected accelerator and validates actual argv before spawning. The shared-path
default4K/software1x regressions and bounded independent review pass; see
`docs/design/desktop-launch.md`.

WHPX with legacy VGA was slower and is excluded from this profile. No host
security setting is changed by this command. The backend remains CPU composition
and checked framebuffer copies; no GPU/vblank/60FPS claim follows. See
`docs/design/rendering-responsiveness.md` for measured evidence and limitations.

After Sign in, an ordinary boot starts an empty desktop. Start (Door button or Windows key), the
pinned taskbar and desktop icons open File Explorer, Notes and Settings. Existing
windows are activated/restored; closed applications can be launched again. Closing
all applications keeps the desktop alive. Start's Shut down ends the session.
The private adversarial image alone starts Files/Notes automatically.

Settings exposes actual output dimensions, About and persistent Light/Dark title
bar/taskbar appearance. App control/text roles follow the live dark/light palette. Use Up/Down for pages,
Tab to choose an appearance, and Enter/Space to apply it. Unsupported features are
not presented as working settings. Friendly Home labels do not rename the private
UID-based storage paths; Ctrl+L still edits the real path.

`test boot --through P11` explicitly reruns implemented foundation regressions while
P12 remains active; it neither qualifies nor closes the still-incomplete P12 gate.

Settings now uses the reference Home shell and twelve categories plus About, with
local Windows assets and actual device/session information. Ctrl+F or a title-bar
search click filters categories; Enter opens the first match and Escape clears.
Up/Down changes categories; Alt+Left/Back navigates history. Page Up/Down or the
scrollbar moves the content. Personalization selects persistent light/dark desktop
appearance. Home opens real File Explorer through the trusted Window broker.
See docs/design/settings-reference.md for the visual sources and incomplete scope.


## Trusted access and current density

Boot now stops at Local account / Sign in. Click Sign in or press Enter; user
session admission occurs only after this trusted gesture. No password is configured.
The desktop starts empty. Windows key opens the searchable app grid/list; type an
app name, arrows/Tab select, Enter opens, Escape dismisses. The right tray displays
real UTC time; click it for calendar/month navigation. Device quick settings opens
Settings and reports the actual no-network/no-audio profile. The narrow right edge toggles Show desktop. Caption buttons highlight on hover/press; moving away before
release cancels their action. Settings follows the current light/dark palette.

AtQHD, controls/fonts/icons/cursors use200%; at4K they use300% from source outlines
or native asset frames. Do not enlarge a1280x720 QEMU surface to judge native4K
quality. A host window with another aspect ratio can have SDL black bands; the
fixed GOP guest scanout itself has no unused top/bottom rows. Ctrl+Alt+0 returns
QEMU to unscaled output, Ctrl+Alt+F toggles fullscreen, Ctrl+Alt+G releases capture.

For a sharper QHD guest with the same1280x720 logical workspace, use
`target/p11-host/release/door-xtask.exe desktop --resolution 2560x1440`.
This remains windowed and does not force fullscreen. QEMU host resizing can still
filter or letterbox a fixed-mode guest; dynamic guest modes and fractional DPI
are not implemented. Ordinary default startup remains1280x720 at100%.

Explorer Ctrl+L/Alt+D enters an editable address; supported `C:\Users\1001`,
`C:\Windows\System32`, relative paths and profile shortcuts resolve through real
Storage. See [path coverage](../design/windows-paths.md) for the precise boundary.

## Current trusted multitasking

Win+E Files; Win+I Settings; Win+S Search; Win+D toggles Show desktop.
Win+Tab opens Task View. Win+Ctrl+D creates a desktop, Win+Ctrl+F4 closes/merges
it; Win+Ctrl+Left/Right switches. Task View activates/closes real windows and
accepts preview dragging onto desktop tiles. Alt+Tab holds real MRU previews
until Alt release; Shift reverses, Escape cancels. Win+Left/Right snaps halves,
Win+Up maximizes, Win+Down restores then minimizes. Drag-top maximizes; side/
corner releases snap halves/quarters. Snap Assist and Win+Z remain unavailable.

Search and Task View use native controls, running indicators and the right UTC
clock/date, notification/calendar and disconnected/muted glyphs. Paint and hit
geometry are shared, including the wide reference Search field. Battery is hidden
without a guest provider; US ASCII has no multilingual switcher yet.


Settings now separates12 categories and Display/Installed apps/About details.
Search includes detail pages. Home/End jumps to the scroll limits; PageUp/Down
and the scrollbar retain the fixed header. Display values remain readable when
snapped. Unsupported providers show an unavailable description and no action.
Win+A opens quick settings; Win+N opens notifications/calendar; Escape dismisses.
Explorer's history chevron preserves Forward entries, and Quick access/Folders/
Devices and drives can be collapsed with their painted targets.

Notes Close and Ctrl+O now present Save/Don't save/Cancel when edits or uncertain
save attention remain. Tab/Shift+Tab changes the choice, Enter/Space activates,
Escape cancels; mouse buttons invoke the same real operation. Cancel keeps RAM
and history, and failed actions keep the prompt. Resolving an interrupted shutdown
does not restart that canceled power sequence: request shutdown again afterward.
This remains a fixed-file bounded editor, not complete modern Notepad.

Current reproducible desktop regression and immutable evidence validation:

```powershell
cargo xtask test desktop-current
cargo xtask revalidate desktop --report reports/runs/<execution-id>/report.json
```

The first builds independently and runs9 private native sessions. The second
reopens an existing successful report, preserving its source/hash and executing
no guests. Both preserve failed evidence. See document-dialogs.md for the current
accepted bindings, screenshot readiness and declared failure-injection limits.

Permanent Files removal uses Shift+Delete on a selected file or empty folder.
Cancel is the initial action; ordinary Delete/Recycle Bin remains unavailable.
The current exact confirmation supports names up to32 printable ASCII characters.
Separate disposable regression workflows (bounded independent acceptance):

```powershell
cargo xtask test remove-sdk --image <diagnostic-image.json> --baseline <saved-private.raw>
cargo xtask test desktop-remove --image <desktop-image.json>
cargo xtask test desktop-remove-accelerated --image <desktop-image.json>
```

The SDK clones its baseline and retains three actual boots, including a paused
pending-deletion image reboot. GUI workflows create their own raw disks; neither
uses out/session/user.raw. See files-deletion.md for evidence and limitations.
