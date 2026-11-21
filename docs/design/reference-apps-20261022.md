# Monitor and Preview reference revision — 22 October 2026

P12 development, incomplete. The owner concepts are retained with original hashes
in `reports/runs/reference-apps-owner-20261022/index.json`. They are design
requirements, not Windows behavior, hardware measurements or working providers.
Exact pixel equality is not established. Earlier integration evidence remains
historical and is not relabeled as a fresh run.

## Presentation and interaction

`door-app-ui::fluent` provides native control glyphs, themed layers, rounded cards,
measured labels, wrapped descriptions and hover/press states. Forty-six Segoe
Fluent selectors are exported from the pinned local font at 20/40/60 physical
pixels. CPU, RAM, Wi-Fi, four-cell Apps and bulleted Processes have correct distinct
selectors. The 3D glyph is Microsoft's pinned MIT Fluent Cube SVG. Existing font
and asset provenance restrictions remain; no new font binary or dependency was
added. These app-only masks do not enlarge Workspace's nearly full executable.

Monitor has a 236-DIP rail, 20-DIP content inset, four 176-DIP resource cards,
42-DIP process rows and optional 304-DIP details column. Performance, Processes,
Apps, Startup, Services, System, Storage, Network, Logs and Settings are reachable;
CPU and Memory open full graphs. Filtering, sorting, stable PID/incarnation
selection, pause/resume, one-shot refresh, update interval, graph fill and status
preferences operate on real session state. Refresh while paused samples once
without resuming the timer. Graph strokes remain connected over steep changes.
Process entry/exit observations are bounded to 96 entries. Scroll positions clamp
after removals, filtering and clear; hidden searches release on page transitions.
Logs headers/rows share their actual column coordinates. Settings are session-only.

Preview has a 220-DIP rail, 168-DIP caption tabs, toolbar centered at y90, canvas
starting x232/y128, 108x88 filmstrip items and a full-height 388-DIP information
pane. Caption and navigation spacing follow measured concept landmarks; the
trusted caption's authority/height is unchanged. The pane has an 80-DIP thumbnail,
34-DIP metadata rows, 112x86 action cells with horizontally measured label centers,
and aligned conversion controls. Toolbar, tabs, pane buttons and compact dialogs
share paint/hit bounds. The empty pane wraps its instruction; stacked conversion
omits a redundant overlapping sentence. Text export's fixed UTF-8 selector looks
disabled; images expose PNG/QOI choices. Unsupported tabs cannot begin export.

Library pages filter opened tabs. Sixteen session recents reopen through Storage;
failed paths preserve open tabs and display an error. Existing zoom, rotation,
flip, metadata inspection and new-file export remain real operations. Modal
open/export dialogs block underlying actions and remain reachable down to 240x120.
Compact rails scroll to every destination. Export creates a new file through the
existing durable staging/rename path; originals are not overwritten and uncertain
replies retain recovery paths.

## Provider and fidelity limits

Monitor uses the existing authenticated 608-byte kernel snapshot with actual timer
observations, private committed memory and global device request counts. Disk I/O
means requests/rate, not bytes, busy percentage or volume capacity. Startup,
GPU/network measurements, process termination and service control are unavailable.
Logs are session observations, not Windows Event Log. Unsupported pages state
those limits instead of manufacturing successful operations or telemetry.

Preview retains eight tabs, one-megapixel image decode, 16 KiB raw text, 6 MiB
encoded input, 16 MiB aggregate decoded-image admission and a 96 MiB heap ceiling.
PNG/JPEG/QOI/BMP/P3, UTF-8/UTF-16, ZIP listings, WAV waveforms/metadata and bounded
OBJ wireframes are supported. Playback, PDF/Office, crop/adjust/markup/erase, OCR,
recording/capture, archive extraction, sharing and batch engines remain absent.
Exports are new PNG, QOI or UTF-8 text; metadata/EXIF preservation is not implied.

The client material is opaque themed color, not Mica/acrylic backdrop blur.
Shadows and trusted caption controls remain compositor-owned. Integer 1x/2x/3x
and the ASCII atlas remain; non-ASCII labels show visible fallback. Fractional
DPI, Unicode/IME, native high-density large-image pressure and visible SDL latency
are not qualified. No sustained 60 FPS claim follows from host screenshots or the
earlier coarse 4K paint result of 20.11 ms average/30 ms maximum.

## Validation and identities

Every Monitor destination/drilldown and Preview destination is painted in light
and dark at 490x358, 900x600 and 1472x952 with all three densities. Host fixture
values, including the sample photo's synthetic file metadata, are explicit test
data. The 58 retained PNG/PPM pairs are host rendering, not guest execution or
pixel equality to the owner's JPEGs.

The full host suite passed 812 tests across 40 target summaries (34 nonzero,
6 zero). After two final Preview paint-only fixes, all 16 affected tests passed
again (15 library plus one binary). Counts overlap and must not be summed.
Three selected native suites cover six private boots: Monitor one, Preview three,
Explorer two. Monitor exercises all ten pages and CPU/Memory drilldowns with 14
fresh authenticated click routes; Preview exercises 17 destinations, real read,
rotate/export, reboot reopening and 3x caption interaction. Persistent bytes,
object identity, process retirement, clean journals and image pixels are audited.
Terminal and Settings native workflows were not repeated in this revision.

Final runtime source: `ee8eab4e4161ecd5133f9ea4eaabb83fbad7817b241a0bb51002d20995183d88`.
Final development image: `out/images/host-1792646238237329400-21896-desktop-dev/image.json`.
Monitor/Explorer ran on the earlier31cd image. Exact source bridges differ only
in `desktop/preview/src/presentation.rs`; 22 of 23 artifact files and 19 of 20 final initfs
file payloads are identical. Full-host/assets receipts retain their c552 source
identity. Only Preview changed; its final native suite uses the final image.
These explicit bridges do not turn older reports into final-source executions.

- build: `reports/runs/host-1792646238237329400-21896/report.json`, SHA-256 `77b0e4021ce10d772b88bc524632ee3f04211bbb28ac482290b4fab53ea092c6`; source `ee8eab4e4161ecd5133f9ea4eaabb83fbad7817b241a0bb51002d20995183d88`.
- full_host: `reports/runs/host-1792645717405923900-29208/report.json`, SHA-256 `f0269d86fe9ce952b0f3cb12d8b9d4e5499f97122df8527ed3bb5b92c12a7d9c`; source `c5520c4503bc8682dd2e011b713aef595dbc2bdc60fde3e5855093513ab08a43`.
- captures: `reports/runs/host-1792646345819545300-29844/report.json`, SHA-256 `8443e16d9cbc71e0356f90279f0c72d1e481a53abdf4fa3e458556c9ff11f096`; source `ee8eab4e4161ecd5133f9ea4eaabb83fbad7817b241a0bb51002d20995183d88`.
- monitor: `reports/runs/host-1792645257563497100-10380/report.json`, SHA-256 `c6dd34ce1a9ae794ef30f926c4e2256d9bc9e6af5bec78ebaf2d313cef4f2f2a`; source `31cd65392198291ddf9d19f06d470d5f296fc89458e2d9aea8a384270fb0791d`.
- preview: `reports/runs/host-1792646365322138800-33104/report.json`, SHA-256 `0e1bbebb0b2234c8798d6605c5848578dd178df009718bf74ecec8db99381e43`; source `ee8eab4e4161ecd5133f9ea4eaabb83fbad7817b241a0bb51002d20995183d88`.
- explorer: `reports/runs/host-1792645459154832100-30028/report.json`, SHA-256 `b0a42a1850c8aa1ee708b2b1387725247b3eab42aea65564fc65d8d7ff7596c0`; source `31cd65392198291ddf9d19f06d470d5f296fc89458e2d9aea8a384270fb0791d`.
- assets: `reports/runs/host-1792646037420132000-17112/report.json`, SHA-256 `87053efbc8485b95a89da6927a4c3d303cf0e5b51122d0c2519aba3e93f19b7b`; source `c5520c4503bc8682dd2e011b713aef595dbc2bdc60fde3e5855093513ab08a43`.

The affected host log is `target/reference-wrap-host.log`, SHA-256
`1f7cd61c00ba9840e790af5dd852ca9d2f519222a39fd9490bde2068a5c98db2`. Structured copies of these receipts are coordinated in handoff,
requirements, phase ledger, app inventory and source records. The ignored
`reports/runs/reference-apps-review-20261022/review.json` binds the independent
review, exact final documentation-only source bridge, gate attempts and hashes.
Documentation is stabilized before those gate attempts to avoid circular hashes.

Cumulative qualification remains blocked by the missing original P00 report
`reports/runs/host-1791804427914736400-32560/report.json`; it must not be recreated.
The owner volume is absent and was not initialized. Out alone remains limited to
20,000,000,000 bytes; target and reports are excluded. The latest selected native
receipt measures 9187571074 bytes within that limit. No historical evidence or
failed run was deleted to produce this checkpoint.
