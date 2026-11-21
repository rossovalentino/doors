# Door identity

`door.svg` is the vector master of the Doors open-door mark, redrawn on 2026-10-22
from the owner-supplied reference (a 1254×1254 raster on a white canvas with a soft
drop shadow). The reference was measured, not traced automatically:

- front leaf: x 319–665 px, top edge slope −1/2, bottom edge slope ≈ 0.27, corner
  radii ≈ 30 px (left) and ≈ 20 px (right); diagonal gradient from deep navy
  (#0230a0, bottom-left) through #0a76f3 to cyan (#66e8ff, top-right), plus a cyan
  highlight along the free edge;
- pale rear panel: x 665–934 px, y 348–930 px, ≈ 28 px corners, light from the hinge
  (near-white #f8feff at mid-height) to #86bdfb at the far edge.

The mark is mapped into the 24-unit box with one unit of vertical padding and
optical horizontal centering. The canvas and drop shadow are not part of the mark:
the master is transparent, with no white rectangle or halo, and keeps the pale rear
panel. Exports (`cargo xtask assets branding build`, or all assets with
`cargo xtask assets build`; `assets check` reproduces every byte):

- premultiplied BGRA `door.{16,20,24,28,32,40,48,56,60,64,72,80,84,96,120,128,160,192,240,288}.bgra`
  (taskbar Start button 24/28 px at 1×–3×, boot splash 120/160/288 px);
- `door.png`: transparent 256 px preview;
- `door.ico`: PNG frames at 16/24/32/48/64/128/256 px.

Uses: taskbar Start button (window server `desktop.rs`), kernel boot splash for
desktop boots (`kernel/src/console/mod.rs` `splash`), Settings > About. The source
image remains owner material and is not an executable input.

## Monochrome mask (`door-mask.svg`, master specification 140.4)

The default product mark is mask-based: one flat colour, no gradients, shadows,
text, perspective detail or handle. Geometry (24-unit grid, project-owned, not
traced from any Windows asset): a doorway frame on the right (x 12–21, y 3–21,
1.5-unit jambs and head, open to the floor) and one door leaf swung toward the
viewer on the left (x 3–10.5, hinge edge y 2–22, free edge y ≈ 4.4–19.6), with a
1.5-unit opening between them. The visible shape fills the centred 18×20 optical
area; edges sit on the 1.5-unit grid so the opening is a clear pixel column at
16 px. It is the same identity as the colour master (front leaf + doorway), reduced
to a silhouette.

Exports (`cargo xtask assets branding build`; `assets check` reproduces every byte):

- `door-mask.{16,20,24,28,32,36,40,48,56,60,64,72,80,84,96,120,128,160,192,240,288}.mask`:
  8-bit coverage, `size × size` bytes, the window server glyph format
  (`tray::glyph`), tinted with the surrounding control's foreground/accent colour
  for light, dark, hover, pressed, disabled and contrast states;
- `door-mask.png`: 256 px preview.

The exporter rejects gradients/opacity in the source and fails if the opening
between leaf and frame stops being visible at any exported size. The colour
`door.svg` remains for identity surfaces that use full colour (boot splash, About);
choosing the mask for the taskbar Start button is the shell owner's switch.
Provenance: original project geometry, 2026-10-26; no third-party asset.
