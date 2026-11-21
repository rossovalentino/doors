# Desktop controls: reference and implementation contract

Reviewed 2026-10-20. P12 remains the active, partial toolkit phase; these are
development consumers of its controls. Complete shell fidelity belongs to P14.

## References

The May taskbar design specifies four edges and start/center alignment, with
Start and flyouts anchored to that edge. Its alternate-position search uses an
icon. See [Microsoft's taskbar design](https://blogs.windows.com/windows-insider/2026/05/15/improving-windows-quality-making-taskbar-and-start-more-personal/).
The [October15 rollout update](https://blogs.windows.com/windowsexperience/2026/10/15/whats-new-in-windows-11-everyday-experiences-reimagined/)
says personalization improvements are beginning broader rollout. Availability
still depends on build/device/rollout; the May preview is not a universal stable
Windows baseline.

Rechecked2026-10-21: the [July31 release notes](https://blogs.windows.com/windows-insider/2026/07/31/announcing-new-builds-for-31-july-2026/)
separate a shorter taskbar from icon-only sizing and limit that preview's small
bar to horizontal edges. Door currently uses48DIP/40DIP thickness on every edge;
vertical compact mode is an extension, not an identical preview implementation.
Per-window labels/grouping, a dynamic app catalog and current Start section/size
controls remain missing. The owner's installed25H2 capture and package states
in windows-core-apps.md take precedence over assumptions from an Insider article.

The [August context-menu design](https://blogs.windows.com/windows-insider/2026/08/17/improving-file-explorer-context-menu-faster-simpler-and-more-customizable/)
provides a Personalization detail page and command organization preferences.
Five official screenshot files, dimensions, URLs and SHA256 identities are in
`docs/references/desktop-shell/sources.json`. They guide original interactive
rendering; no screenshot substitutes for a working page. Original icon frames
remain embedded offline. Screenshot DPI/build flags are not independently fixed.

## Shared state and authority

`service-protocol::desktop::Controls` validates a six-bit value. Bits0–1 select
Bottom/Top/Left/Right, bit2 start alignment, bit3 compact buttons, bit4 inline
primary context actions, bit5 Properties at the bottom. Default48 preserves
normal bottom/center placement. Unknown bits are rejected.

Storage Setting3 uses an existing per-user HKCU64 registry DWORD, with the same
authenticated subject and journal persistence as other configuration. The on-disk
catalog schema is unchanged. Missing values return defaults; another user's
settings cannot be acquired through a caller-supplied UID. No owner disk is
formatted or migrated by this feature.

Window opcode0x5019 accepts exactly four little-endian bytes without transfers.
Ordinary requests require the bound session/epoch/UID and owner channel. Trusted
Init can restore settings at session admission/recovery. Capability32 negotiates
event11; its `a` field contains the checked bitfield. Pending state coalesces per
client rather than filling the app's input queue. Legacy clients do not receive
unknown events. The Settings workflow persists/applies the value and reports
failures; changing appearance does not overwrite these separate controls.
Transient queue-pressure WouldBlock retains the pending intent and retries after
polling/yielding. An accepted pending change delays close while that retry is needed.

## Geometry, placement and rendering

`door_window::taskbar::Layout` supplies the bar, six app/control targets, reserved
work area, Start bounds, tray flyouts and bounded underlay. Horizontal bars use
left/center alignment; vertical bars use top/center. Alternate edges use the
search glyph. Normal thickness48DIP, compact40DIP; native density selects the
existing original logo/icon frames rather than scaling a tiny raster.

Changing the edge reflows maximized and snapped top-level windows, including
minimized windows whose pre-minimize state is maximized. Normal window title
areas remain reachable. The engine preflights target geometry, epoch increments
and all owner queues before publishing a work-area change. Changed epochs retire
old attachments. Restore rectangles retain normal size and position except for
the minimum translation needed to keep their title reachable after an edge move.
Task View also derives its card layout and hit targets from this work area.

Desktop icon paint, hit, hover and damage use the new work-area origin. Drag
constraints and Snap use that area. Start, calendar and quick settings open on
the inward side of the current bar. The tray uses the same edge-specific
rectangles for paint and click handling; unavailable battery/audio/network
providers are not inferred from the host.

The blurred material samples only the bar plus its filter margin. Hover restores
and transfers the bar rectangle, not the whole flyout cache. Cache allocation
retains its high-water capacity when switching edges, avoiding simultaneous
large underlay buffers. Scanout services pointer input during transfers.
Presentation remains a CPU copy into GOP; no GPU/vblank or frame-rate claim.

## Implemented boundary

The Taskbar page exposes position previews, alignment and compact controls.
The Context menu page exposes preferences with real effects in the shared Files
menu, including inline command placement and Properties ordering. Customize menu
routes to Settings through an authenticated app3/page16 launch request. Read-only
Properties reports the actual object. Unsupported clipboard/mutation/print,
cloud/app extensions and device actions must remain disabled. They are not
implemented by drawing the Windows labels.

Auto-hide, per-monitor bars, never-combine labels, arbitrary drag positioning,
touch/tablet behavior, complete Windows menu providers and every reference-page
toggle remain unqualified. No claim of complete1:1/latest Windows UI or P12
closure follows from these finite controls. Host geometry/blur/queue tests and
actual private-disk QEMU interactions are required before binding evidence.

Current bounded development evidence is the242-check combined native report
reports/runs/host-1792521201711588300-25240/report.json (SHA256
5450fc6ef3fdb4fec82fd1df004f9be75a117e2015c0a7c6dc7f33c0a1a61172).
It includes all-edge work areas, Start anchors, compact/alignment restoration,
context preferences, real Properties and Customize routing. Rendering evidence
and historical cumulative limitations are recorded in rendering-responsiveness.md;
these controls do not close P12 or qualify full Windows fidelity.
