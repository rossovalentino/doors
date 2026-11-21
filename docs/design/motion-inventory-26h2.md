# Windows 11 26H2 shell motion inventory

Every shell animation Windows 11 26H2 shows, what Doors does today, and what
remains. Sources: the owner's PC (build 26300), captured as region bursts
(one frame every 4-9 ms) in `reports/artifacts/reference-26h2/motion/`; the WinUI
timing resources (ControlFasterAnimationDuration 83 ms,
ControlFastAnimationDuration 167 ms, ControlNormalAnimationDuration 250 ms,
ControlFastOutSlowInKeySpline 0,0,0,1); and Microsoft's Fluent motion
guidance. Burst timestamps are capture times, not DWM presentation times:
they show the sequence and shape of a transition, and every duration below is
an estimate unless its design token says it is specified. Smoothness on real
hardware is qualified with native GPU composition (P19); the foundations
here are time-based (real elapsed µs from the calibrated transition clock),
so the same code runs at whatever rate the compositor presents.

Foundations in Doors:
- `graphics/compositor/src/engine/animation.rs`: window open/close/minimize/
  restore/relocate transitions from retained pixels (ghost layers).
- `graphics/window-server/src/motion.rs`: `Transition` (open/close surfaces:
  Start, flyouts, snap bar, Snap Assist) and `Tween` (any 0-1000 value easing
  toward a target from where it is: hover, press, toggles, rotation), with
  the Faster/Fast/Normal tokens of `door_design_tokens`.
- `taskbar::Feedback`: press and hover feedback of taskbar buttons.

Status: **done** (implemented, tested), **gap** (not yet; reason), **blocked**
(needs a later phase's device or service).

## Taskbar

| Animation | Windows 11 26H2 | Doors |
|---|---|---|
| App/Start button press | Icon shrinks to about 80 % on pointer-down (a 36 px icon at 150 % draws 29 px; `taskbar-press-down.png`) and stays small while held; it grows back within a few frames of release (`taskbar-press-release.png`). Moving off before release cancels the click. | **done**: `Feedback::press`, icon scale tween 83 ms down (decelerate) / 167 ms back, area-filtered icon (`apps::draw_icon_scaled`); the search glyph keeps its 87.5 % press |
| Hover plate | Fades in and out (Faster). | **done**: `Feedback::hover`, the old plate fades out while the new one fades in |
| Running indicator | The pill under an app widens when it becomes active and narrows when another app activates (Fast). | gap: indicator width jumps |
| Show hidden icons (^) | Clicking rotates the chevron 180° clockwise (^ → > → v) over about 150 ms after the plate appears; the overflow flyout slides up about 12 px and fades in (`taskbar-overflow-chevron-open.png`); closing rotates it back. | gap: Doors has no notification-area icons yet, so the button is hidden, as Windows hides it with nothing to show; rotation needs a rotated-glyph painter on top of `Tween` |
| Hover tooltip | App name after the system hover time (about 400 ms), faded in. | gap: no taskbar tooltips |
| Hover thumbnail preview | Thumbnails fade and rise a few px after the hover delay; moving to a neighbour button switches without a new delay. | done (preview, thumbnails, close); gap: fade/rise-in |
| Launch and attention | Pinned icon bounces while an app starts; attention plate pulses. | gap: launch progress arrives too late; the attention plate is static |

## Start, flyouts and menus

| Animation | Windows | Doors |
|---|---|---|
| Start open/close | Slide up and fade (250 ms decelerate); close 150 ms accelerate. | done (`Transition`, StartOpen/StartClose) |
| Quick settings, calendar, notification center | Same slide and fade. | done (FlyoutOpen) |
| Quick settings tile toggle | Fill crossfades between subtle and accent (Fast); the glyph colour follows. | blocked: Wi-Fi, Bluetooth, airplane mode and energy saver need P15 network/power services; tiles stay unavailable and the fill tween is a `Tween` per tile once they act |
| Quick settings sliders | Inner thumb dot grows on hover (×1.167) and shrinks on press (×0.71), Fast; the track fill follows the thumb; the volume glyph switches Mute / 0 / 1-33 / 34-66 / 67-100 levels (E74F, E992-E995). | blocked: volume needs the P15 audio service, brightness a display-brightness control (none under QEMU) |
| Context menus | Fade with a slight slide from the invocation point. | done |
| Toast | Slides in from the right (333 ms). | done (Notification) |

## Windows and snap

| Animation | Windows | Doors |
|---|---|---|
| Open / close | 95 % scale and fade. | done (WindowOpen/Close) |
| Minimize / restore | Shrink toward and grow from the taskbar button. | done (taskbar anchors) |
| Maximize / snap glide | Point-to-point curve, 200 ms. | done (WindowMaximize) |
| Snap layouts flyout (hover maximize, Win+Z) | Fades in under the maximize button; zones highlight with the accent. | done (layouts, SnapPreview); gap: zone highlight fade |
| Drag-to-top snap bar | Slides down from the top edge; layouts expand on approach. | done |
| Snap placement preview | The target zone shows a blurred plate of what is behind it with an accent outline, and gliding between zones moves the plate instead of jumping. | gap: glide between zones; the blurred plate uses the existing acrylic backdrop path |
| Snap Assist | Remaining windows fade in as thumbnails. | done |
| Alt+Tab / Task View | Thumbnails fade in; the selection ring moves. | done (static ring); gap: ring glide |

## Inside apps

| Animation | Windows | Doors |
|---|---|---|
| File copy dialog | Window open/close (DWM). | done: fade and scale on open and close (`files/operations.rs`) |
| Button press | Background to SubtlePressed. | done (state colours, no tween) |
| Toggle switch | Knob slides 167 ms and stretches while pressed. | gap (shared control library) |
| Settings navigation pill | Stretches and slides between items. | gap |

## Not claimed

Frame pacing, latency and smoothness are not established by these captures.
MOTION-01 measures Doors' own presentation trace; sustained 60/120 Hz is a
P19 qualification.
