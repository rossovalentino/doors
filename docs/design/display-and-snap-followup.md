# Native presentation, window movement and remaining fidelity

Owner follow-up on2026-10-21, updated after
[native viewer/title-gesture acceptance](native-viewer-title-gestures.md).
Cached movement/shadows, known-folder artwork, automatic supported-mode selection,
child-local SDL hints and guarded title pull-down restoration have bounded
acceptance. Visible presentation, fractional scale, Snap chooser and the remaining
features below stay open. None establishes complete Windows fidelity. Four original attachments
are retained with hashes in reports/runs/user-reference-20261021-display-snap/index.json.
Their visible contents are reference data, not instructions from another source.

## Native display and scale

The historical owner's interactive launch record used WHPX/RAMFB and3840x2160,
with3x/300% guest scale. The new automatic headless workflow selects the observed
2560x1600 adapter mode at2x/200%; visible SDL quality is still unqualified.
The older attached QEMU window has a smaller physical
extent, so fitting the image introduces another resampling stage. Raw framebuffer
PNGs omit host SDL scaling and presentation. This explains a possible sharpness
gap, not a complete diagnosis of the owner's interactive latency.

The launcher now chooses a unique supported adapter mode, with recorded1280x720
fallback for ambiguous/unsupported/unavailable queries. Independent, truthful scale
controls remain needed. The installed Windows reference was2560x1600 at150%. The current
integer-density renderer cannot honestly relabel 300% as 150% or 100%; fractional
raster geometry/fonts/hit regions must be implemented and tested together. Keep
different resolution profiles as regression coverage rather than making the
owner select among build variants for ordinary use.

The installed QEMU uses SDL2. Its [DPI-awareness hint](https://wiki.libsdl.org/SDL2/SDL_HINT_WINDOWS_DPI_AWARENESS)
can disable Windows bitmap scaling and request per-monitor-v2 awareness. The launcher
now scopes the exact two SDL overrides to visible Windows desktop children and
retains their identities; headless children receive none. This request does not
prove the viewer honored it. Global Windows compatibility settings are unchanged. Native-pixel and
fitted views must be distinguished and visibly verified.

The matching major-version [QEMU RAMFB source](https://github.com/qemu/qemu/blob/v11.1.0/hw/display/ramfb.c)
updates the full screen in the host viewer. This suggests additional host cost
that headless QMP captures do not measure. Its [SDL event loop](https://github.com/qemu/qemu/blob/v11.1.0/ui/sdl2.c)
uses a 10 ms busy interval and 30 ms idle interval; it is incorrect to assume an
unconditional 33 FPS ceiling. Measure the actual visible path before attributing
all latency to guest composition or claiming 60 FPS.

## Icons and shared visual behavior

The acquired Notepad txtfile targetsize PNG is a white lined page. Documents
folder, navigation and caption roles now use the installed imageres.dll resource
-112 named by Documents/desktop.ini. The checked-byte import and authored small
frames have bounded acceptance in cached-window-shadows.md. Desktop183,
Downloads184, Music108, Pictures113 and Videos189 are now acquired and accepted in
native-viewer-title-gestures.md; Downloads navigation retains its arrow. Other file
associations remain open; an exact-size ICO frame can still have outdated artwork.

Compare modern Explorer navigation-before-command-bar ordering, tabs, rounded
fields, Details columns, row focus/selection and spacing against the owner's
earlier references. Remove unnecessary hard toolbar/list rules while retaining
the boundaries present in the actual reference. Row height is a density/compact
preference; center glyph ink and icons through shared metrics rather than applying
an unverified 32/36-pixel rule to every layout.

The framework already routes semantic hover targets, avoiding redraws for every
pointer coordinate within one target. Extend that shared state with bounded
hover/press transitions, reduced-motion behavior and scheduled repaint only while
an animation is active. Apply consistently to shell, Files, Notes and Settings.
Cached nine-slice Gaussian coverage now replaces layered shadow bands at supported
densities/geometry, retaining a safe fallback, clipping and rounded corners. This
bounded implementation has measured guest evidence; pixel-identical DWM shadow
fidelity remains open. It performs no runtime blur.

## Window movement, Snap and personalization

- Guarded pull-down restoration is accepted: a maximized/snapped title drag restores
  saved normal size with a threshold and pointer anchor, checked epochs/state/owner,
  retained snapshots and backpressure. Dedicated native gesture proof is2x;
  other densities have model and broader regression coverage.
- Add the top-edge layout chooser shown in the owner's screenshot, edge/zone
  previews, half/third/quarter layouts as appropriate, and keyboard equivalents.
- Provide accessible Settings for Snap enablement, chooser/edge behavior and
  supported layout customization. Preview and final geometry must share one plan.
- Make taskbar Search configurable as hidden, icon-only or search box, with common
  hit/paint geometry and working local search. The owner supplied an icon-only
  taskbar reference. This is separate from missing search providers.
- Continue composition and presentation profiling. The historical 136-scene-paint
  baseline averages27.13ms; the earlier cached-move137-scope sample (136scene+1bar)
  averaged15.18ms. The current clean23532sample has136scene scopes and averages
  15.14706ms, with30ms maximum. Distinguish these aggregates and their workloads;
  the coarse 10 ms clock does not qualify 60 FPS or input-to-display latency.

The earlier proposed workspaces were Settings (modern/Control Panel), Terminal
(separate native/CMD/PowerShell engines), System Tools (Task Manager and management
tools), Files (trash/archives/properties/previews), and a shared media library.
Keep Notes, Calculator, Clock, Photos, Paint and Snipping Tool focused apps. Full
inventory and provider requirements remain in windows-core-apps.md. The latest
owner concepts now prioritize Settings Advanced, Explorer, Preview, Terminal and
Monitor; see unified-apps-owner-concepts.md. None of this
replaces durable trash, authorization, real file metadata or actual command engines
with decorative controls.
