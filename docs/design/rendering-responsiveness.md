# Native density and CPU presentation repair

The later [cached-move checkpoint](cached-window-shadows.md) accepts soft shadows,
opaque rows and guarded client pixel reuse. Its final4K sample averages15.18ms
across137 drag paint scopes, with30ms maximum; the60FPS target remains open.
The earlier27.13ms baseline below has136scene scopes, not an identical aggregate. Native host
presentation, proper scale controls and additional composition work are tracked
in [display-and-snap-followup.md](display-and-snap-followup.md).

Development batch, 2026-10-21. P12 remains partial. Actual guest validation is
recorded below; the bounded independent critic is separate from phase closure.

## Findings and repairs

- Explorer's old 80px bicubic export had one channel above alpha. The checked
  compositor correctly rejected the entire bitmap. The exporter now clamps
  filtered channels to alpha and prefers authored exact-size ICO DIB32 frames
  over reducing the large PNG illustration. Larger exports reduce the nearest
  larger source; no runtime downloads are involved.
- Fixed 200% at 4K produced a 1920x1080 logical desktop, making controls smaller
  relative to the owner's familiar 1280x720 workspace. QHD uses 200%; 4K now
  uses 300%/288 DPI. Fonts, colored icons, command masks, avatar glyphs, previews,
  title controls, hit regions and actual Windows cursor frames follow that
  density. Dynamic/fractional DPI remains separate work.
- A 4K full frame required 8,100 display syscalls at 4096 bytes. The checked
  transport now captures at most 16 KiB on the guarded 64 KiB kernel stack:
  2,025 calls for the same full frame. GOP BGRX uses a volatile assembly REP
  scanline copy; RGB GOP retains explicit channel conversion. Physical stride
  padding is preserved. There is no imported GPU driver or vblank guarantee.
- Large app rectangles reuse scanlines instead of constructing millions of
  identical pixels in a loop. Opaque rounded interiors use spans; corner
  coverage uses an exact precomputed table for radii through 60 physical pixels,
  retaining the original antialiasing calculation for larger radii. Existing immutable
  client snapshots remain the source during moves; moving does not reconfigure
  or recapture client content. Desktop hover damage under windows is regional.
- The four capture extents reserve capacity without clearing 140 MiB before
  sign-in. Capture and wallpaper decode initialize their actual lengths.
- Native 300% previews initially grew the workspace executable above its 16 MiB
  admission limit. Lossless QOI previews now decode into a bounded twelve-entry
  per-process cache, reused across repaint. The protected loader limit remains.
- Slow-frame records separate composition ticks from transfer ticks and report
  bytes/calls. They diagnose this CPU backend; they do not establish FPS.
  Logging every presented frame exhausted the unchanged 16-KiB process allowance
  during a long workflow. Scene markers now follow state/activation transitions,
  using the generation captured before composition; slow samples stop at sixteen.
  A native long-workflow audit requires remaining diagnostic credit after shutdown.
- The launcher previously forced TCG. An explicit experimental WHPX profile keeps
  the same one-vCPU/2-GiB authority and isolated-disk boundaries. WHPX with VGA
  made 4K scanout slower (about 1.3â€“3.6 seconds), so it is not the default. The
  dedicated profile uses QEMU RAMFB instead: loader-owned BootData stays reserved
  and maps with normal RAM cache attributes; firmware GOP keeps its device policy.
- Repeated CPUID calls in CPU-local lookup caused costly hypervisor exits. A
  single-CPU fast path is sealed only after boot validates the ACPI processor
  count, online mask, initialized BSP and absence of registered APs. Multi-CPU
  profiles retain actual identity lookup and cannot register APs after sealing.
  A native 4K WHPX/RAMFB run reduced the initial transfer from 125 ticks to three;
  pre-cache dragged frames still took seven/eight composition plus one/two transfer
  ticks (10 ms per tick). The final corner-cache run no longer produced sampled
  drag warnings at the80ms threshold. This is not a60-Hz qualification.
- The 73,810-byte coverage asset stores the exact sixteen-sample rounded-corner
  result. Host tests compare every cached radius/corner and clamped tiny geometry
  against the original sampler. Final native accelerated and combined regressions
  include this optimization.

Primary implementation references: [QEMU WHPX](https://www.qemu.org/docs/master/system/whpx.html),
[fw_cfg DMA](https://www.qemu.org/docs/master/specs/fw_cfg.html),
[RAMFB](https://github.com/qemu/qemu/blob/master/hw/display/ramfb.c) and
[UEFI allocation rules](https://uefi.org/specs/UEFI/2.11/07_Services_Boot_Services.html).

## Text

The pinned Segoe outlines now produce filtered RGB stripe coverage using three
horizontal samples per physical pixel at each native density. Opaque targets
blend each color channel independently. Translucent text or targets use one
grayscale alpha, preserving correct premultiplication during later composition.
This is Door's own LCD renderer, not Microsoft's ClearType/DirectWrite engine;
calibration, BGR/rotated/OLED stripe layouts and universal visual
equivalence remain unqualified. Host enlargement or shrinking of a guest output
can still blur it; inspect screenshots at native size. The later typography cache
adds host-executed hinting and variable outlines; see typography-cache.md.

Microsoft documents both the [DirectWrite subpixel approach](https://learn.microsoft.com/en-us/windows/win32/directwrite/introducing-directwrite)
and the [transparent-target limitation](https://learn.microsoft.com/en-us/windows/win32/api/dwrite_1/ne-dwrite_1-dwrite_text_antialias_mode).

## Taskbar reference comparison

Microsoft's [current taskbar guide](https://support.microsoft.com/en-us/windows/experience/personalization/customize-the-taskbar-in-windows)
shows centered Start/Search/Task View/apps, active/open indicators and a separate
system tray. Door already shares paint/hit geometry for those regions and the
four-edge controls described in [desktop-controls.md](desktop-controls.md).
The observed 4K mismatch was density and disappearing artwork, not missing edge
placement. This repair increases all physical taskbar metrics together and uses
authored icons instead of blurry reductions. Unavailable audio/network/battery
providers must not be represented as functional services.

The installed Windows 11 build is the owner's newly selected primary reference.
Its version, scale, app versions and controlled dark/light Settings observations
are recorded in windows-core-apps.md and the machine-readable inventory. Earlier screenshots remain useful historical
references, not a claim of pixel-identical Windows behavior.

## Verification boundary

Host coverage includes all colored icon sizes at 1/2/3x, real LCD channel
differences with opaque alpha and transparent fallback, exact QOI/raw pixel
equivalence, clipping, scene composition and transfer planning. Native density
tests sample actual opaque icon detail, perform a held window drag with no new
client capture, maximize/minimize/restore three independent apps and audit saved
note bytes after clean shutdown. All tests use isolated disks. Owner data is not
a fixture. Full P12/P14 fidelity and broad Windows application support are not
established by these tests.

### Retained current evidence

- Product build15932: reports/runs/host-1792518976867677100-15932/report.json,
  SHA256501a4698e1975ee3ee1348d19f0e72710672b17cb46a522ce60b67be8c599d65.
  Its five independent product pairs match; the workspace executable remains
  12,115,376 bytes, below the unchanged16MiB limit.
- Host15540: reports/runs/host-1792521218487727200-15540/report.json,
  SHA256c370f27764236d5e911d74c2d62211b836f899a32d345667f9ed35f806cde75d;
  519 tests across33 executables, unchanged inputs.
- Combined25240: reports/runs/host-1792521201711588300-25240/report.json,
  SHA2565450fc6ef3fdb4fec82fd1df004f9be75a117e2015c0a7c6dc7f33c0a1a61172;
  242 native checks, unchanged inputs. Persistence/reopen, recovery, long workflows,
  four-edge controls, context routing, themes and four output modes are covered.
- Focused23560: reports/runs/host-1792520856571892400-23560/report.json,
  SHA2560bcc325b2ccbf68a33fe3e4dc8f825c5999a3afa2b54ca929c631edc3932cadd;
  52 checks at1920x1080,2560x1600,3840x2160 and2560x1440.
- Explicit accelerated24108: reports/runs/host-1792519129016593900-24108/report.json,
  SHA2565f9333c63c359d2e499469b832d344b19c640cca9fa997d8391dbaa05d5a7c7b;
  14 native4K WHPX/RAMFB checks, including cached moves without app recapture.
- SMP26016: reports/runs/host-1792517930372207700-26016/report.json,
  SHA2566ebd240ae9347fa6485e267edb671cb0c567d657030748e293371de2a93689bb;
  15 actual two/four-CPU, degraded and remote-TLB cases. Kernel/UEFI bytes are
  unchanged in the final product build; this is an explicit cross-report binding.
- Assets25664: reports/runs/host-1792518256488161100-25664/report.json,
  SHA25601c9d6ca2e399fe09d255ad42760f9635465bae3229e9d27e063bb413e9aff79.

The final host/native test source identity is1065b9a1db875e0bf325cffb97cebf2894a04e677f80282585c9404d12b2f8e4.
Relative to15932, only Cargo.lock, dependency metadata, xtask's local dependencies,
controls.rs and resolution.rs differ. These are host harness changes: fresh
foreground waits, exact LCD/shadow label comparison and a background probe kept
clear of the Personalization scrollbar. No guest source changed. Documentation
updates after these reports are not an identical-whole-source claim.

The label oracle compares5928/23712/53352/23712 RGB pixels against pinned native
coverage and background. It checks placement/raster integration, not independent
Windows typography quality. Earlier failed reports15976 and2948 retain the obsolete
fully-white label test and scrollbar sample failures. Earlier diagnostic-budget,
activation and actual rendering failures also remain retained.

The cumulative verifier22956 failed because historical P00 evidence is absent:
reports/runs/host-1792517986061230500-22956/report.json,
SHA256d724644c46009567d7e476373b651d3e1e1b6fdd28ecb7d8c5dd3780aaf5263e.
Do not recreate a prior passing report or infer cumulative qualification from
these current focused results. The next work remains within partial P12.

Independent read-only critic: reports/critics/p12-rendering-responsiveness-critic.json,
SHA2561345bf33648c3205f123f018d0d3c9e4b6cd73d12fd144dd1ab1ff4b293ab20b.
No unresolved actionable finding remains within this finite batch. The critic
rehashed the retained artifacts and source deltas; review is not a guarantee of
all typography, smoothness, application or Windows compatibility requirements.
