# Native Settings reference

The owner-supplied1129x787 Home screenshot is the layout target. Its account,
connected devices and cloud usage are example Windows data, not Door state.
The native app uses a local account, actual output dimensions and real persisted
appearance. No connection, update success, cloud usage or paired device is invented.
The reference does not establish a frozen Windows build/DPI qualification.

Reference geometry: centered516x32 search at(306,10), profile at(26,66),273x36
navigation rows atx8/y146 with40px pitch,20px category icons,14px labels, main column
atx310,28px semibold heading aty64,8px card radii and4px card gaps. Home uses two
columns, with a single-column fallback when narrow. Undrawn panes have no actions.
Page Up/Down and pointer scrolling preserve the header/profile/navigation.

The shared client-caption contract places search inside the real title bar.
Move/resize and system buttons remain authoritative Window operations. Files
activation is a bounded user request to Window, which alone contacts Init's broker.

| Page | Primary reference | Current Door behavior |
| --- | --- | --- |
| Home | [Settings overview](https://support.microsoft.com/en-us/windows/experience/exploring-windows-settings), supplied image | Local files, recommended navigation, device status, theme previews |
| System | Retained original category image | Full category row inventory; working Display and About details |
| Bluetooth & devices | Overview; official `bluetooth-devices.png` Devices screenshot | PS/2 input profile; Bluetooth absent |
| Network & internet | Overview, Network category | No adapter in the declared `-nic none` profile |
| Personalization | [Microsoft Themes](https://support.microsoft.com/en-us/windows/experience/personalization/personalize-your-windows-experience-with-themes) | Local Bloom previews; durable light/dark desktop setting |
| Apps | Retained original category image | Separate Installed apps detail with three native launch actions |
| Accounts | Retained original category image | Local identity hero and explicit account-provider boundaries |
| Time & language | Overview, Time category | Current US ASCII input; unsupported controls identified |
| Gaming | Overview, Gaming category | GPU/Game Bar/capture unavailable |
| Accessibility | Overview, Accessibility category | Pointer/keyboard scope and outstanding accessibility work |
| Privacy & security | Overview, Privacy category | Protected-process and authenticated-file boundaries |
| Doors Update | Overview, Update category | Unavailable on Doors; no false check/success |
| About | [Device information](https://support.microsoft.com/en-us/windows/experience/find-information-about-your-windows-device) | Native platform and development edition |

Home and retained Themes/About images provide direct geometry references.
Category-specific original screenshots and hashes are now retained in
`docs/references/settings-pages/sources.json`: nine Windows Central originals
(2021), an annotated ASUS Gaming capture (2023), and two ASUS Display captures
(image URLs dated2021, article updated2025). Build and DPI are unknown. These
are historical composition references, not a complete current2026 baseline.
The modern owner-supplied Home/top-search shell is retained; the old sidebar
search placement is not mixed into it. Complete Windows functionality, dynamic/fractional DPI, IME and full accessibility
remain unfinished P12/P14 work. Current apps do follow live dark/light palettes.

Offline assets:12 pinned WindowsIcons ICOs,9 Microsoft Support category PNGs,
and E702/E895 Bluetooth/Sync glyphs from Microsoft's downloaded Segoe Fluent Icons
font. Local manifests pin identities. The host creates premultiplied BGRA20/32/48px
exports; the guest embeds them. Neither runtime loading nor export reproduction
contacts a source URL. The System category uses the actual pinned monitor ICO.
Four original3840x2400 backgrounds (Bloom, dark Bloom, Glow, Sunrise) are converted
losslessly to QOI and packed separately from the executable; only small matching
previews are embedded in Settings. Six presets combine backgrounds and color mode.
Light/dark controls preserve the selected background. Selection is durable and
reapplied after boot/server restart. Apps rows open/activate the actual singleton;
Apps > Installed apps opens the three real launch actions. Compact Home
routes only its three visible rows; blank space and row gaps have no actions.

## Raster quality and taskbar comparison

Large image reductions now area-prefilter before bicubic sampling. Sampling only
four original pixels while reducing3840px to116px caused visible fold/edge aliasing;
larger original files alone did not fix that. Icon exports use the same prefilter
on premultiplied channels. Device-screen, large preview and theme images use real
antialiased rounded clipping (4px inner radius), rather than painting a square
bitmap over a rounded frame. The guest wallpaper similarly prefilters reductions;
native4K source samples remain unchanged. The4K profile now rasterizes at192DPI/200%; other fixed modes use96DPI.

Microsoft's [work-area contract](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-systemparametersinfoa)
excludes the taskbar; [window behavior](https://learn.microsoft.com/en-us/windows/win32/winmsg/about-windows)
maximizes into that area. Doors reserves48logical pixels, keeps the initial Settings window above
it, and allows manually moved normal windows behind the trusted bar. Taskbar input
takes priority there. This fixes the previous8px initial overlap at720p.
[Acrylic guidance](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic)
uses a frosted backdrop; sharp underlying app text must not simply show through.
Door approximates this with a cached25x25 scene blur plus tint. It refreshes when
the scene changes, while hover keeps the existing48-row transfer path. This is a
bounded CPU approximation, not a qualified reproduction of Microsoft's compositor.
The visible launcher now opens a normal1280x720 window. Automatic monitor-sized
fullscreen made100%-DPI controls too small; it has been removed. Explicit native
modes remain available. Native4K logical sizing is200%; dynamic and fractional DPI remain unfinished.
Enlarging the final raster in QEMU is not a sharpness fix.

The offline Microsoft Support Themes screenshot is retained as
docs/references/desktop-shell/themes.png with its hash/source in sources.json.
It shows literal Light/Dark window swatches and a separated theme gallery.
Target-theme swatches bypass the active control palette while preserving shared
rounded AA/clip/density. Personalization uses shared gallery paint/hit positions,
heading separation and content bottom for scrolling. This fixes overlap; it does
not turn the combined development Personalization page into the full referenced
Themes page. Capture build/DPI are unknown; latest per-page fidelity remains open.


## Category composition refinement

`settings_catalog` shares each section/row's geometry with hit testing and scroll
extent. Category rows use68px height,72px pitch,4px corners,14px group labels,
distinct original Segoe Fluent20/40px masks, and an action chevron only for an
available destination. Home keeps its own8px card vocabulary. Accessibility now
separates Vision, Hearing and Interaction; Privacy separates Security, Doors
permissions and App permissions. Accounts has a local identity hero. Update has
a truthful unavailable status and disabled check action above More options.
Long lists cull off-screen rows before glyph rasterization.

The15 surfaces comprise12 navigation categories plus About, Display and Installed
apps. Display reports actual boot output and native scale in wide and narrow
layouts; its value fields are read-only. Search includes these detail pages when
queried. Home/End and PageUp/Down scroll the current page without moving the shell.
Home's Display recommendation opens the real Display detail. Visual effects is
explicitly a limited shortcut to the working Personalization theme controls.

Unsupported devices/services are identified in row descriptions and expose no
hit action. Replicating reference labels never establishes pairing, networking,
printing, update, security-dashboard, accessibility or account-management services.
The category inventory is substantially closer; complete feature coverage and
quantitative1:1/latest-build visual equivalence remain unqualified.

## New2026 Personalization details

Taskbar and Context menu append pages15/16, preserving existing page identities.
The official May taskbar and August context-menu pictures are retained with hashes
in docs/references/desktop-shell/sources.json. Four position previews, alignment
combo, compact control and context inline/Properties toggles have durable live
effects. Unsupported auto-hide/multi-display/menu-extension actions remain
disabled. See [the shared contract](desktop-controls.md) for exact supported scope.
There are17 current surfaces; the prior15-page category packet is historical.
