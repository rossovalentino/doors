# Original administration artwork

The sixteen vector tiles in settings.svg were authored for Door OS from the
owner's requested administration categories. They are original artwork, not
extracted Windows resources. Existing ordinary category icons retain their
separate pinned Windows provenance.

The guest consumes consecutive premultiplied BGRA tiles from offline exports.
Reproduce with `cargo xtask assets advanced build`; verify using the normal
`cargo xtask assets check`. The export order is the stable `Page::TOOLS`. Native 1x, 2x
and 3x rail/hero assets are rasterized at their actual source density.

## Original device illustrations, 22 October 2026

`device-laptop.svg`, `device-monitor.svg`, and `device-tablet.svg` are original
200x136 original vector device frames. Their screens use the classic Windows
Bloom wallpaper from the existing pinned wallpaper manifests.
Exports at 1x, 2x and 3x preserve premultiplied transparency without runtime scaling.
Firmware ACPI Preferred_PM_Profile selects laptop (2), tablet (8), or monitor
(other profiles). Unspecified firmware uses the monitor without claiming desktop.
The sixteen icon tile order is the stable `Page::TOOLS`, independent of the six
visible Advanced destinations. The original SVGs remain editable sources.

`device-categories.svg` supplies eight original color hardware category tiles
(display, processor, audio, network, storage, Bluetooth, USB and other devices),
exported at32/64/96 pixels for native-density32px rows.

The current device page chooses `{device}-light` or `{device}-dark` exports from
its actual Settings palette. Both themes are exported at1x/2x/3x, as raw BGRA
reference pixels and lossless QOI for the native application. The existing base
filenames alias dark Bloom for earlier consumers. The template SVG links the dark
Bloom source; the exporter substitutes light Bloom when generating light assets.
It uses the existing area-prefiltered PNG composition path, without adding runtime
SVG/raster codecs. Device frames and category icons are original; Bloom screens
retain the separate Windows wallpaper source provenance in
`assets/wallpapers/windows/manifest.json` and `collection.json`.

The original device-hero-fold.svg supplies translucent glass folds, with1x/2x/3x lossless QOI exports. The16 original tool tiles also have individual lossless QOI exports for each native size; raw BGRA atlases remain reproducible references. Windows Registry/Services/Computer category assets are selected from the existing pinned Windows resource inventory.
