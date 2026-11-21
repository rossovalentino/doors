# Current application fidelity audit — 22 October 2026

Bounded P12 development. Exact Windows identity, full Windows functionality,
fractional DPI, visible SDL resampling and sustained60FPS remain unqualified.
This pass does not close P12 or replace missing historical phase evidence.

## References

The owner's six Advanced Settings references and input hashes are retained under
`docs/references/advanced-settings-20261022`; Explorer references are under
`docs/references/explorer-20261022`. The actual installed dark Explorer was observed
through Computer Use this turn: tabs/navigation/command order, rounded fields,
known-folder rail, details columns and right information pane. No host files or
preferences changed. Settings launch exposed no targetable window on refresh.
The Computer Use skill excludes terminal app automation, so Terminal application
and console theme distinctions use Microsoft's published primary references:
https://learn.microsoft.com/en-us/windows/terminal/customize-settings/appearance
https://learn.microsoft.com/en-us/windows/terminal/customize-settings/profile-appearance
Segoe Fluent glyph roles use Microsoft's symbol catalogue:
https://learn.microsoft.com/en-us/windows/apps/design/iconography/segoe-fluent-icons-font

## Changes and actual behavior

Terminal no longer discards the desktop application palette before drawing its
rail, tabs, menus, Settings, About and Profiles. Premultiplied material roles are
explicit for both themes; neutral surfaces and foregrounds use the existing
palette. The console scheme remains independently selectable. Light console
prompts now use contrasting dark ink. Rail/tab/preference/toggle/Save/menu labels
use measured vertical centering; Save centers horizontally. Settings rows and
menu items have target-based hover fills with visible dark contrast.

Text hit routing excludes menus, controls, scrollbars, out-of-bounds coordinates
and unavailable-engine pages. Terminal participates in the shared open-menu cursor
policy. The disabled pane control uses the genuine Windows OpenPane glyph;
split-pane functionality remains unavailable.

Command completion includes path resolution and checked output redirection. Failed
commands never call the writer; uncertain redirects are attempted once, report
failure and keep the distinct 'Write not confirmed' message. Success diagnostics
are emitted only after the final result. Tests exercise failed redirects and
commands, no-redirection output and successful writes; the private native script
also rejects a root-escaping output path before ordinary valid file operations.

Settings Installed apps previously omitted Terminal despite its actual catalog
entry. A shared four-app inventory now drives count, labels, row targets and
commands1/2/3/9. Terminal's row313 maps to9; it must never use command4, which has
power semantics elsewhere. Its real Windows icon and scrolling extent are present.
The native Settings check opens Terminal through this row, waits for actual
activation and validates exact retirement of both launched applications.

## Icon provenance and rendering

Eight authored targetsize PNGs from installed Windows Terminal1.24.12741.0 were
losslessly archived by the existing checked importer. App, desktop, Start and
taskbar use terminal-current.ico, SHA256:
af7211c63e587adc88053873aa42937c73ed5601bc347aa0b8d57977654dd59d.
Per-frame paths/hashes are in assets/icons/windows/terminal-current.source.json.
Existing pinned icons are preserved. Distribution rights remain unqualified.

The gray top strip belongs to the official logo at all sizes. An attempted
master-only selection was withdrawn after pixel inspection and independent review:
there is no evidence of mixed branding. Native exports retain exact authored
small artwork, then the nearest larger source for other sizes. No forced256px
selection or additional runtime image resize/cache path remains.

## Progress and remaining providers

Settings has six Advanced destinations, bounded typed registry editing, real
read-only disk/firmware inventory, endpoint discovery and saved appearance controls.
Registered endpoints are not Windows service health/control. Policy enforcement,
scheduling, driver installation, partition mutation and many administrative
providers remain absent. The reference's sample hardware/status/tasks are not data
for this OS. See advanced-settings.md for exact limits.

Explorer has caption tabs, checked object-bound operations, bounded actual image/
text previews and durable user-created tags. Trash, clipboard transfer, sharing,
recorded timestamps, recursive sizes and several file viewers remain open. See
explorer-modern.md. Terminal has native file builtins and checked preferences;
CMD, PowerShell and WSL still require actual engines. See terminal-native-preview.md.

A suspected narrow Settings panic was retracted: has_main already guards panels
below280 logical pixels. Actual six-page painter checks now cover240/320/516/600/
612-pixel clients, both themes, opacity and hidden-action bounds. No speculative
production workaround was introduced.

## Validation scope

Final current host/native/asset results are recorded below.
Native workflows use private disks; owner storage is never a fixture. Captures
prove actual native scanout and selected pixel probes, not all-page pixel identity,
visible SDL quality, input latency or frame cadence. Most pages are retained visual
captures rather than semantic pixel-oracle qualifications. Native checks use a
checked development relink from a fresh independent build; protected core sources
and every reused artifact are validated. That relink does not qualify an independent
full phase rebuild.

The initial fresh Terminal run34064 passed26checks/two boots before final fixes.
Later stable common-image checks at sourcec8e328f3907113f39e87a6c9c8999e63a1851d99939768cb7a121e34c5c40bb3
also remain prior-revision evidence. No failed or earlier run was removed.
Preparatory35744 was aborted before native build; legacy build19468 failed its
live output inventory with Access denied (os5), not a source compilation error.
Both are preserved. The last strict all-target Clippy attempt reported existing
shared UI/Explorer style findings; no passing all-target lint claim is made.

A formatter invocation normalized existing Rust whitespace beyond the primary
changes. Two graphics Clippy style findings were corrected without changing
material interpolation or conditional behavior. Earlier deleted report history
was not reconstructed. Current evidence remains compact and out stays capped at20GB.


## Final bounded validation

The workspace host suite passed 762 tests in 36 executables. Log:
`target/current-app-audit-host-final.log`, SHA256
`053105732fee36f562c2740bb1af41ba119fba102d240b3fc085bd494b455ce4`.
Afterward, independent review found Explorer's fixture only checked generic clean
shutdown markers. The host fixture now calls the canonical authenticated retirement
validator after each sealed serial, requiring commands [1,2] on boot 0 and [1] on
boot 1, and retains both results. Five existing negative/ordering/identity tests
passed and the affected two-boot native workflow was rerun successfully. No guest
source or asset changed for that final harness correction.

| Workflow | Report | Checks | Result |
| --- | --- | --- | --- |
| Settings | `reports/runs/host-1792610572224355700-12140/report.json` | 16 | Passed, inputs unchanged |
| Terminal | `reports/runs/host-1792610679121708300-22728/report.json` | 19 | Passed, inputs unchanged |
| Explorer | `reports/runs/host-1792611027387645200-32312/report.json` | 9 | Passed, inputs unchanged |
| Assets | `reports/runs/host-1792610846718714300-20748/report.json` | 0 | Passed, inputs unchanged |

Report SHA256 values, respectively:

- Settings: `fbb7e3cd6abb13d642c5ff033ae2558acb6d89b463b61b76730316c3806264a2`
- Terminal: `3a103f04295f58ab4a3a94c5100173bc183facc3ba0f00f623ce0b4d41cab12b`
- Explorer: `fd3bf1fda77edb557a5851d8dc8f0aedcb59036a4726a6c1cc5aaf898f6a1193`
- Assets: `1d6add503109ce003b6c445e5dff1a42d2dab324e23cc022995c625fffb9aeb7`

Settings, Terminal and assets bind source
`5e28483991e1c032dc7471b12d45045314346933b44f0b7d17fde0c9c04e38f8`.
The stricter Explorer fixture binds
`d20c229e3f824d51088b288e4a3b6308331ad535b16510c05f4d6a1fc5a51058`;
independent source comparison found only
`tools/xtask/src/desktop/explorer_modern.rs` changed. All use the unchanged guest
image `out/images/host-1792610572224355700-12140-desktop-dev/image.json`.
The three workflows cover five private boots and 29 PNG/PPM capture pairs. Review
checks include image/process launch identity, source/artifact hashes, actual
selected probes, retirement ordering, journal/CRC and exact persistent bytes.

Selected `cargo xtask verify` was attempted and stops at the original missing P00
report `reports/runs/host-1791804427914736400-32560/report.json`. Its current failure
receipt is `reports/runs/host-1792610948804522400-32680/report.json`, SHA256
`a2a9f49bc4ae994484b0acb62652e361e3f4907fe59b34840a21fc69925050e6`.
Earlier full-build/aborted and prior-revision diagnostic runs are preserved.
The final independent review is
`reports/runs/current-app-fidelity-review-20261022/review.json`.
Documentation changes after testing are bridged by an enumerated source comparison
under that review directory; a bridge cannot substitute for a phase rebuild.

## Remaining fidelity and architecture work

No matching-DPI, full-page pixel oracle has established exact fidelity. The current
visible SDL window, fractional scaling, Windows ClearType identity, animations,
input-to-display latency and presentation cadence remain open. RGB-stripe LCD
coverage exists on opaque targets; translucent composition uses scalar alpha.
This is not a claim that the raster matches Windows ClearType.

The final Workspace PT_LOAD span is 16,617,472 bytes, leaving only 159,744 bytes
under the existing 16 MiB bound. Further apps need an explicit executable/asset
split before expanding this monolithic binary. Do not silently raise resource
limits or replace real providers with reference sample data.

Recommended bundled organization remains Settings (including supported Advanced
administration), Explorer (file management and metadata), Terminal (shared UI for
actual shell engines), Preview (format-specific viewing/editing/conversion), and
Monitor (actual process/resource diagnostics). Notes remains a small text editor.
Preview and Monitor and the missing shell/administrative providers remain future
work. Durable Trash, clipboard transfer, recorded timestamps, fractional scaling,
full Unicode/IME and measured frame pacing remain substantive gaps.
