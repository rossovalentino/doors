# Desktop shell references

Retrieved2026-10-19. Images and SHA256 identities are retained in
`docs/references/desktop-shell/sources.json`. No screenshot is used as guest UI.

| Surface | Selected picture | Behavior source and limits |
| --- | --- | --- |
| Taskbar/tray | Microsoft taskbar diagram, system tray | Microsoft Support Customize the Taskbar; actual profile has no network/audio/battery devices |
| Quick settings | Microsoft quick-settings.png | Supported controls require actual subsystem state; absent devices remain unavailable |
| Calendar/notifications | Microsoft notification-center-1.png | No fabricated notifications or connectivity; clock requires a real hardware time source |
| Taskbar position controls | WinTotal26H2 capture, German locale | Microsoft May15 Experimental announcement documents four edges and per-edge alignment; Support still describes bottom-only. Rollout/reference distinction is preserved |
| Passwordless sign-in | Actual Japanese local-account capture on Microsoft Q&A | Account avatar/name and one centered Sign in control; no password provider is claimed |
| Desktop context menu | Actual Microsoft Community Hub screenshot | Contextual View/Sort/Refresh/New/Display/Personalize families; expose only implemented commands |
| Door identity | Original repository vector |Transparent cyan gradient mark, native exports and theme accent; taskbar optical weight/placement follows diagram |

Primary behavior sources:

- https://support.microsoft.com/en-us/windows/experience/personalization/customize-the-taskbar-in-windows
- https://blogs.windows.com/windows-insider/2026/05/15/improving-windows-quality-making-taskbar-and-start-more-personal/
- https://learn.microsoft.com/en-us/windows/win32/uxguide/inter-mouse
- https://learn.microsoft.com/en-us/windows/apps/design/iconography/segoe-fluent-icons-font

Desktop label width is measured with the same pinned Segoe UI12 atlas used to
paint it. Its baseline and horizontal center follow the icon; the shadow is a
semitransparent one-pixel offset. The Door icon preserves its cyan vector gradients in transparent native exports. Images do not establish motion timing,
full DPI behavior, every rollout state or a pixel-identical Windows implementation.
The new shell surfaces remain development work until actual guest checks pass.

The owner supplied an open cyan door reference2026-10-19. Original SVG geometry
adapts it to a transparent vector with28/56px native taskbar exports for100/200%DPI.
Microsoft dark-mode and Mica dark images are retained beside the other pictures.
Dark app control/text roles follow the published theme resources; this original
renderer is not the Windows DWM Mica implementation. Civil time is UTC from QEMU
RTC. No per-user time-zone setting is implemented. Position/context references remain acquisition/design inputs. Trusted passwordless login is implemented and focused guest-tested.


The retained Microsoft Design2025 Start picture guides the unified grid/list/search
layout; only installed Door apps are shown. The retained Microsoft title-bar diagram
and its design article guide32px logical caption controls, full-bleed hover/press
backplates and native outline glyphs. Close turns red/white; other controls use
neutral theme backplates. A gesture acts on release and cancels when released outside.
The owner cyan-door adaptation now exports portable PNG/ICO alongside native BGRA.
Settings categories use the supplied Home picture and offline Windows category icons;
the primary category inventory is Microsoft Support /exploring-windows-settings.
A complete per-page screenshot baseline is still missing; thirteen Door page captures
are behavior evidence, not thirteen matched Windows screenshots.

Taskbar refinement2026-10-20 follows the Microsoft diagram: shared centered layout,
48px height,36px (26H2: 25H2 32px + 4px) rounded 216px Search field (icon on narrow outputs), actual20px
Search/Task View masks,24px app frames, optical28px Doors mark, rounded running
indicators and real Ringer glyph. Removed the extra dark top rule; clock/date
make room for the bell. Battery/overflow slots require actual providers.

Retained task-view/multiple-desktops/snap/alt-tab pictures guide organization and
held switching. Native previews cache actual commit revisions over the wallpaper;
full blur/motion and quantitative1:1 comparison remain outstanding.
See ../audits/p11-desktop-progress.md for gaps and primary behavior links.
