# P11/P12 audit against the preserved master specification

22 October 2026. Implementation audit, not a new qualification certificate.
The authoritative phase table is in `DOOR_OS_MASTER_SPECIFICATION.md` section 91
(lines2646–2649); detailed slices are WINDOW-A/B and UI-A/B (lines4216–4219).

## Phase position

The ledger already records P11 as historically passed and P12 as active/partial.
P11's surviving independent review is `reports/critics/p11-critic.json`, which binds
report `reports/runs/host-1792303566097472600-17116/report.json` and SHA-256
`b3550258872aae95b17dd2437a9bdc34c961cadd436d8abd0f4fb9f48ade8c01`.
That underlying run report is currently absent. The original P00 report is also
absent. Neither report may be reconstructed or represented as fresh evidence.

There is no need to start P12 again. Continue P12 development and qualify the
changed foundations with new reports. This does not authorize claiming that
current P11 evidence or the cumulative P00–P12 gate has passed. P13 and P14 remain
separate gates. The master assigns complete shell/app fidelity to P14; implementing
some app surfaces early does not satisfy that gate.

## P11: implementation versus the master

| Master requirement | Implementation found | Remaining qualification or scope limit |
| --- | --- | --- |
| Sections41–44: protected window authority, IDs, ownership, protocol and input | Separate protected Window service; kernel-authenticated owners; native Window RPC; per-window IDs, epochs, focus, capture, move/resize and stacking | Preserve adversarial foreign-owner, stale-ID/epoch, malformed resource and lifecycle tests on the changed implementation. |
| Section45: capture/release/producer ownership | Kernel checked capture into private snapshots; producer never receives display authority; explicit attach/commit/release | CPU completion is not GPU completion, scanout/vblank or a presentation fence. Multiwindow admission must not discard the source model on failure. |
| Sections46/114: software composition, clipping/damage and output | CPU renderer, immutable transfer, cursor, cached wallpaper material and translated move reuse | Shared surfaces, queued input, close/configure races and recovery need actual guest evidence. Host screenshots are insufficient. |
| Section47: native GPU path | CPU fallback exists; no working accelerated device backend | The phase table schedules native GPU acceleration separately in P19. Do not call the CPU compositor GPU accelerated. |
| Section48: scheduling and latency | Cached moves and limited invalidation reduce work | No measured sustained60FPS or visible input-to-photon latency claim. Earlier coarse paint timing is not that measurement. |
| WINDOW-A/B exit: independent clients and malicious-buffer/lifecycle rejection | Historical critic accepted the bounded1024x768 Q35/TCG slice; later app diagnostics exercised independent clients | Historical run report is missing. This revision changes process/resource capacities, producer mapping allocation and shared app lifecycle: rerun the applicable native foundation and app cases before accepting it. |

The current multiwindow revision is deliberately bounded: separate window models
inside each app process, one authenticated connection per app, same-owner tab
handoff, fallible destination admission and explicit resource quotas. It is not
an unlimited process/window claim. Historical eight-process profiles are history;
the proposed twelve-process development capacity needs fresh validation.

## P12: requirements from the master

| Master requirement | Current state | Still needed to close P12 |
| --- | --- | --- |
| Sections49–51/UI-A: application model, layout, controls and painting | Shared native runtime/painter, measured fixed-size fonts, theme roles, dialogs, menus, document editing, lists and bounded image decoding | General reusable layout/control semantics, keyboard traversal and deterministic component gallery across the required states; asynchronous cancellation and invalidation evidence. |
| Section52: text, Unicode, font fallback/shaping, bidi, grapheme/caret semantics | Bounded ASCII document editor and fixed ASCII font atlas; some file decoders accept UTF-8/UTF-16 | Full Unicode editing/layout, shaping/fallback, combining marks, bidi/grapheme selection, multilingual fixtures and correct clipboard text conversions. A visible filename fallback is not Unicode support. |
| Sections53/109/UI-B: input and IME | Keyboard/pointer input, capture, selection, basic commands and input-reset handling | Real IME composition protocol, multilingual input, layout changes during composition, secure context rules and broader device-input coverage. |
| Sections57/UI-B: DPI, themes, presentation preferences | Integer1x/2x/3x rasterization and light/dark themes | Fractional/dynamic DPI and monitor transitions, high contrast, color policy, reduced motion/transparency controls and their evidence. |
| Sections58/113/UI-B: accessibility, clipboard, drag/drop and focus | Some focus/keyboard/dialog state machines; current tab detach/reattach work is in progress | Semantic accessibility tree, assistive-tool evidence, keyboard-only workflows, clipboard formats/security, interoperable drag/drop, focus restoration and cancellation across destruction/reconnect. |
| Native editor exit | Checked save/reopen, version-conflict/uncertain-save guards, selection/undo and new tab/window work | Complete interactive file workflow plus multilingual/IME, keyboard-only and accessibility/scale/theme qualification. |

## Acceptance order for this revision

1. Finish the multiwindow model and admission/rollback behavior, then test real
   detach/reattach, simultaneous independent edits, cancellation and close/restart.
2. Rebuild the kernel and services for the resource-capacity changes; a development
   relink that reuses the earlier kernel cannot qualify this revision.
3. Run applicable native window adversarial/recovery cases and selected app
   persistence workflows with fresh, authenticated process/window identities.
4. Compare native light/dark renders to the owner concepts and actual Windows
   references. Keep unsupported providers and measured differences explicit.
5. Update the coordinated ledger, requirements, source/dependency records and
   independent review. Attempt the cumulative gate and retain its actual failure
   if historical prerequisites remain absent.

Preview media/PDF/Office/OCR engines, hardware GPU/network providers and complete
Windows application compatibility are separate substantial providers. Their
absence cannot be solved by enabling decorative buttons or sample telemetry.
