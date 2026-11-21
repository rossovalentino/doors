# Windows 11 visual reference for P11–P14

> Build identity correction (26 October 2026): the owner PC is Windows 11 Pro
> 26H2, OS build 26300.9711 ([Environment]::OSVersion 10.0.26300.0; CurrentVersion
> CurrentBuild 26300, UBR 9711; the registry ProductName shows the legacy "Windows
> 10 Pro"). The 25H2/26200.9711 text below is wrong; the frozen P14 reference is
> profiles/windows-reference/reference-26h2-x64.json.

Current selected reference is the owner's installed Windows11 Pro25H2
build26200.9711 and app-package observations in `windows-core-apps.md` and
`project/windows-app-inventory.json`. Its Explorer has actual tabs followed by
navigation/address/search and the command bar. The earlier untabbed screenshot
in `desktop-explorer-reference.md` remains historical evidence. Public release
research below is separate from the installed reference and its rollout state.

Updated 2026-10-18. This is an acquired source index and measurement plan;
P11 native implementation is qualified for its declared software profile. The original specification, especially sections
41–48, 112, 114 and 140, remains the project requirement reference.

Use Microsoft's **Windows UI Kit**, **WinUI Gallery**, and **WinUI control theme
resources** together. They provide detailed design and implementation references.
They do not, by themselves, establish a pixel-identical copy of a particular
Windows desktop. Shell fidelity additionally needs controlled captures from one
identified Windows 11 build. No complete desktop capture baseline was acquired
during this research, and no 1:1 fidelity has been qualified.

## Primary reference sources

### Current release and UI changes

As checked on **2026-10-18**, the [official release table](https://learn.microsoft.com/en-us/windows/release-health/windows11-release-information)
lists **26H2**, general availability **2026-09-29**, with **26300.9711** /
[KB5127648, 2026-10-13](https://support.microsoft.com/en-us/servicing/os/windows-11/2026/10/kb5127648-windows-11-24h2-25h2-26h2-security-update).
This is the chosen public desktop reference target. Hardware-scoped 26H1 and
Insider previews are separate profiles. A release-table lookup is not a capture
of that desktop, and no reference VM has yet been acquired.

The [26H2 feature notes](https://learn.microsoft.com/en-us/windows/whats-new/whats-new-windows-11-version-26h2)
describe configurable Start sections, category/grid app views, taskbar placement
and size options, Explorer navigation improvements and search previews. Reference
captures must freeze those choices, account type, app versions and rollout state;
one old screenshot cannot define every current configuration. Do not add inactive
AI/account controls to Door merely to resemble a screenshot. Their functional
services require separate implementation and qualification.

| Source | What to use | Acquisition / limit |
|---|---|---|
| [Microsoft Windows design resources](https://learn.microsoft.com/en-us/windows/apps/design/downloads/) | Official entry point for design components, patterns, colors, typography and spacing. | Public documentation inspected. |
| [Windows UI Kit for Figma](https://www.figma.com/community/file/1440832812269040007/windows-ui-kit) | Editable Windows component and layout reference, linked by Microsoft through [its official redirect](https://aka.ms/WinUI/3.0-figma-toolkit). | Redirect and public HTML verified. Canvas contents, component revisions and an exported kit were not acquired. Record those before treating an export as frozen. |
| [WinUI Gallery](https://github.com/microsoft/WinUI-Gallery) | Interactive control examples, sample markup, typography, color and accessibility demonstrations. | Source pinned below; app was not installed or executed during this research. |
| [WinUI source](https://github.com/microsoft/microsoft-ui-xaml) | Detailed resource dictionaries, control templates, visual states and material implementation references. | Source paths and commit pinned below; no dependency or source port added to Door. |
| [Fluent Windows overview](https://fluent2.microsoft.design/components/windows) | Windows-specific Fluent context. | Prefer the Windows guidance over unrelated web/mobile component defaults. |

### Pinned source locations

The GitHub release metadata and full repository trees were inspected, without
cloning either repository or retaining a large source archive.

WinUI Gallery release **v3.0.4** resolves to commit
`71b5f04d3f7e3d02f91dc0d86d4d92d6d31c719b`:

- [Design guidance and color examples](https://github.com/microsoft/WinUI-Gallery/tree/71b5f04d3f7e3d02f91dc0d86d4d92d6d31c719b/WinUIGallery/Controls/DesignGuidance).
- [Typography example](https://github.com/microsoft/WinUI-Gallery/blob/71b5f04d3f7e3d02f91dc0d86d4d92d6d31c719b/WinUIGallery/Controls/DesignGuidance/TypographyControl.xaml).
- [Package versions](https://github.com/microsoft/WinUI-Gallery/blob/71b5f04d3f7e3d02f91dc0d86d4d92d6d31c719b/standalone.props).

That release's package file selects Windows App SDK **2.5.4-experimental**.
Consequently its newest examples are reference material, not evidence of the
appearance shipped in the eventual Windows 11 comparison build. Record the
actual control/runtime version used for any measured reference application.

WinUI release **winui3/release/2.5.1** resolves to commit
`ba3a8d59e6de4e2b86ca4f61c78bc26ccf129afd`:

- [Shared control resources](https://github.com/microsoft/microsoft-ui-xaml/blob/ba3a8d59e6de4e2b86ca4f61c78bc26ccf129afd/src/controls/dev/CommonStyles/Common_themeresources.xaml).
- [Button resources and visual states](https://github.com/microsoft/microsoft-ui-xaml/blob/ba3a8d59e6de4e2b86ca4f61c78bc26ccf129afd/src/controls/dev/CommonStyles/Button_themeresources.xaml).
- [NavigationView resources](https://github.com/microsoft/microsoft-ui-xaml/blob/ba3a8d59e6de4e2b86ca4f61c78bc26ccf129afd/src/controls/dev/NavigationView/NavigationView_themeresources.xaml).
- [Title-bar resources](https://github.com/microsoft/microsoft-ui-xaml/blob/ba3a8d59e6de4e2b86ca4f61c78bc26ccf129afd/src/controls/dev/TitleBar/TitleBar_themeresources.xaml).
- [Acrylic implementation](https://github.com/microsoft/microsoft-ui-xaml/blob/ba3a8d59e6de4e2b86ca4f61c78bc26ccf129afd/src/controls/dev/Materials/Acrylic/AcrylicBrush.cpp).

These are references for an original Door implementation. They are not a complete
Windows shell implementation or a proposed WinUI runtime dependency.

## Details to carry into the visual token system

| Area | Published reference | Implementation / measurement rule |
|---|---|---|
| Geometry | [Windows geometry](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/geometry) describes 4-unit common-control and 8-unit overlay/window rounding, plus square intersecting and maximized/snapped edges. | Preserve exceptions such as tooltips and connected controls; never apply one radius everywhere. Validate each target surface and scale. |
| Typography | [Windows typography](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/typography) specifies Segoe UI Variable, weight and optical-size behavior. Caption is 12/16, body 14/20, subtitle 20/28 and title 28/36 in effective pixels. | Store font size, line height, weight, optical size, baseline and fallback separately. Effective pixels are not physical framebuffer pixels. |
| Window chrome | [Title-bar design](https://learn.microsoft.com/en-us/windows/apps/design/basics/titlebar-design) recommends 32-unit standard and 48-unit interactive title bars, with a 16-unit icon. | Measure actual application variants, caption-button geometry and resize/drag regions; these recommendations are not universal shell dimensions. |
| Colors and states | Pinned theme dictionaries and the Gallery color examples above. | Extract semantic roles and state transitions for light, dark and high contrast; avoid copying an isolated screenshot color across different surfaces. |
| Mica | [Mica guidance](https://learn.microsoft.com/en-us/windows/apps/design/style/mica): an opaque wallpaper/theme-derived foundation, with inactive and accessibility fallbacks. | Keep wallpaper-derived base, content layers and focus state distinct. Mica is not live translucent blur of underlying windows. |
| Acrylic | [Acrylic guidance](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic): translucent material for transient surfaces, including flyouts and menus. | Calibrate tint, opacity, noise, blur and fallback against the selected surface. Keep its composition path separate from Mica. |
| Elevation | [Windows elevation](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/layering) distinguishes windows, dialogs, flyouts, tooltips and controls. | Preserve theme/state-specific contour and shadow behavior. An elevation value is not automatically a blur radius or a z-order number. |
| Motion | [Windows motion](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/motion). | Measure trajectories and interruption behavior from timed captures; do not invent one universal duration. Test reduced motion and retargeting. |

Every Door visual token must retain its unit, source URL/commit or capture,
reference profile, affected controls and whether it is published, measured or
provisional. This research does not replace section 140's measurement gate.

## Reference acquisition before claiming 1:1 fidelity

Per-family layouts, control states and phase owners are tracked in
`windows11-surface-inventory.md`. App package/Insider rollouts are separate from
the OS build; preview documentation does not replace the stable target.

Freeze the Windows edition, build/update revision, app/control versions, locale,
font files/versions, display size, DPI scale, text scaling, theme/accent, wallpaper,
transparency, animation and taskbar settings. These fields remain unmeasured here.

Capture desktop/taskbar/Start/search; tray/quick settings/notifications/calendar;
task switching/desktops/snapping; Explorer and file operations; Settings; standard
dialogs; window chrome; and lock/logon/elevation. Each applicable surface needs
normal, hover, pressed, focused, selected, disabled, loading, empty and error states.
Include inactive windows, keyboard focus restoration, high contrast, reduced
motion, and multiple DPI scales. Control dynamic content before comparing images.

P11 should first establish real surfaces, ownership, composition, clipping,
damage, window lifetime, input routing and focus against the working session.
P12 supplies the richer text/control platform; P14 qualifies complete shell
appearance and workflows. P19 separately qualifies accelerated presentation.
An attractive screenshot or host preview does not qualify those guest behaviors.

## Asset boundary

[Segoe Fluent Icons documentation](https://learn.microsoft.com/en-us/windows/apps/design/iconography/segoe-fluent-icons-font)
provides glyph names and recommended sizes, and explicitly limits shipping its
downloaded font on another platform. Treat it as a design reference, not a font to
copy into Door's image. Record any separately acquired redistribution permission.

[Microsoft Fluent System Icons](https://github.com/microsoft/fluentui-system-icons)
is now acquired at commit `a347af95478f989038cccbf085978865264c048a`.
Twelve **20-unit regular** SVGs actually used by Files/Notes are retained under
`assets/icons/fluent`, with MIT license, individual source URLs and SHA256 hashes.
These are real Microsoft open assets; they are not claimed to be Explorer's
private app artwork. `cargo xtask assets build` uses pinned host-only resvg 0.45.1
to create 400-byte alpha masks. `cargo xtask assets check` rerenders and compares
every byte. No SVG parser or raster-export dependency is linked into the guest.

[Selawik](https://github.com/microsoft/Selawik), release 1.01, regular and semibold,
is bundled under OFL with its notice. The official typography page identifies it
as an open metric-compatible alternative to Segoe UI. It is not Segoe UI Variable;
optical sizing, outlines and full multilingual behavior are not equivalent.
The current guest uses antialiased 14-unit body/semibold and 20-unit heading
glyphs. P12 owns shaping, Unicode editing, fallback and scale calibration.

P11 command buttons use documented 4-unit corner guidance; heights, spacing,
colors and app layouts remain explicitly provisional. Resolved token metadata
lives in `desktop/workspace/src/tokens.rs`. P12 now uses three pinned Windows ICO
references from HaydenReeve/WindowsIcons and six Windows CUR references from
bartekl1/windows-ui-assets, as requested for this local image. Manifests record
the exact upstream commits, paths, hashes and exported pixel formats. A complete
Windows wallpaper, Segoe UI Variable font and full app asset set remain absent.
Preserve the master's original Door-mark exception when evaluating fidelity.
