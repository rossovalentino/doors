# Owner detail and responsiveness follow-up

The later [cached-move checkpoint](cached-window-shadows.md) accepts cached soft
shadows, guarded client reuse and installed Documents artwork. Its15.18ms mean
elapsed drag scope (137scopes,30ms maximum) leaves the60FPS target open. The
27.13ms earlier baseline below is historical and has a different path mix. Latest owner
references and display/Snap/icon/hover/shadow requirements are recorded in
[display-and-snap-followup.md](display-and-snap-followup.md).

Owner steering on2026-10-21 extends the existing partial P12 work. None of the
following is closed by the output-layout support checkpoint. The narrower
control-row-alignment.md checkpoint now accepts shared Workspace rows, Notes
caret/selection/footer, Files F2 painting and the Home/txt artwork. Remaining
shell/Settings/Explorer layout details below are still open.

Three supplied images are retained in
reports/runs/user-reference-20261021-alignment/index.json. Attachments1â€“3 were
missing from their temporary paths. Available references show Notes selection,
caret/text misalignment and Explorer's permanent-delete modal/address bar.
The owner reauthorized desktop comparison, but Computer Use was stopped again
by physical Escape. A later inspection observed Explorer's row ordering and the
classic menu, which the owner explicitly confirmed was a customization. Another
authorized attempt was still stopped. No Task Manager/Control Panel inspection
is claimed from that attempt. Three additional retained screenshots are indexed
in reports/runs/user-reference-20261021-details/index.json; two others were missing.

## Concrete work

- Align icon boxes, font line boxes and control rows through shared geometry.
  Explorer breadcrumbs, sidebar rows, commands and Notes status/caret currently
  use unrelated hard-coded offsets. Check1x/2x/3x and light/dark actual pixels.
- Use correct Windows file-type artwork and Explorer Home artwork, with pinned
  sources and native-density exports. File associations determine per-type icons;
  a generic document glyph is not the Windows text-file icon. Check optical Start
  icon alignment against other taskbar artwork as well as nominal box sizes.
- Compare modern/classic file, folder and background context states independently.
  Share command identity and real availability; preserve keyboard/menu semantics.
- Implement normal Delete as durable per-user Recycle Bin movement and restore,
  retaining original location/name, collision handling, permissions/sharing and
  crash recovery in the same transaction. Shift+Delete remains explicit permanent
  removal. Existing code only admits Shift+Delete; normal trash is not implemented.
- Complete taskbar personalization with actual shell/provider behavior. Keep
  unsupported hardware/network actions truthful while their providers are built.
- Target60FPS smooth interaction with measured frame pacing/input latency. Current
  CPU GOP transfer is not vblank synchronized; a stable60FPS claim is unqualified.
  All-frame bounded aggregates and actual accelerated4K drag measurements must
  replace inference from the first+slow16 diagnostic samples. Timer ticks are10ms.
- Continue core apps with real providers and the existing unified Settings,
  Terminal and System Tools architecture. The current8-process/fixed3-app budget,
  phase order and backend qualification remain constraints, not app completeness.

## Current taskbar references

Additional owner details are explicit requirements: Details columns for Name,
Date modified, Type and Size with dividers/sorting; distinct focus-only outline,
selected fill/outline and tight rename selection; rounded address/search fields;
correct location artwork; navigation above commands, tabs, spacing after New and
the right-side Details control. Shell captions, Settings rows and section headers
still need the shared font reference. Date/size values require real metadata;
do not fabricate current timestamps or add per-frame synchronous metadata RPCs.

Taskbar/Start requirements include the filled Search glyph variant, hardware-aware
power/battery status and optional percentage, visible-module controls, optional
left Widgets affordance, Start spacing/colors/sections/pins and persistently
customizable behavior. Battery data must come from a real provider and appear only
when a battery exists. Widgets should offer compact local calendar, timers, notes
and system status; no news/promotional feed by default, no continuous animation or
polling while hidden. Device/network widgets require their actual providers.

Search must include local apps, Settings deep links and authorized guest files,
with explicit result types/paths, deterministic exact/prefix ranking, cancellation,
bounded asynchronous indexing/query work, mutation-driven freshness and keyboard
navigation. No implicit web query or unrelated online promotion. Stale indexed
entries must recheck identity/access before opening. Microsoft documents a
[separate indexing/query pipeline](https://learn.microsoft.com/en-us/windows/win32/search/-search-indexing-process-overview)
and [new search controls](https://blogs.windows.com/windows-insider/2026/07/13/improving-windows-search-box-with-less-clutter-and-more-control/);
the latter is an Insider rollout, not proof of host feature enablement.

The unified Settings coverage index should map both modern Settings and
[Control Panel canonical families](https://learn.microsoft.com/en-us/windows/win32/shell/controlpanel-canonical-names)
to one searchable category/detail hierarchy. The canonical-name list includes
historical/deprecated items, so each needs current relevance and real provider
validation. System Tools includes Task Manager's processes/performance/startup/
users/details/services, plus device, disk, event and scheduling workspaces.
These are requirements, not enabled Door functions.

Microsoft's [25H2 August update](https://support.microsoft.com/en-us/servicing/os/windows-11/2026/08/kb5120998-windows-11-24h2-25h2-update)
documents four taskbar edges, orientation-aware flyouts, a Taskbar size/Small
setting (compact/search-box support on horizontal taskbars), and expanded behavior
settings. Door already supports four edges/alignment/compact geometry, but its
older size-toggle presentation, grouped icons without labels, and missing auto-hide
behavior need further work. Start size, section visibility and profile visibility
are also distinct missing preferences. The installed26200.9711 build belongs to
[October servicing](https://support.microsoft.com/en-us/servicing/os/windows-11/2026/10/kb5127648-windows-11-24h2-25h2-26h2-security-update);
build identity alone does not establish every gradual-rollout feature's state.

The earlier installed reference remains the primary state evidence; generic
support pages may mix Windows10 and older Windows11 directions. Current references
supplement the per-state inventory in windows-core-apps.md.

## Storage offer

D: is a non-system USB NTFS volume labelled Backup, about318GB free when inspected.
No output was moved. Existing receipts bind actual original paths; copying and
deleting local out without location/provenance support would break retained proof.
Prefer explicitly enrolled external retention for future bulk writes, then migrate
old proof only with verified location bindings. Do not use an unverified junction,
rewrite old argv or silently bypass confinement/budget checks.

The first-run tooling batch is now accepted separately in ../build/desktop-first-run.md:
new owner volume initialized explicitly, valid interactive QEMU graph retained,
private WHPX4K/TCG1x and674host tests reviewed. This enables owner debugging without
claiming any of the open shell/app/provider features above. The owner volume is
mutable data and must never be reused as a fixture or reinitialized after use.
