# Shared control rows and acquired icon associations

A later bounded [cached-move checkpoint](cached-window-shadows.md) adds soft
shadows, guarded client pixel reuse and installed Documents role artwork. Its
final sample is15.18ms mean elapsed drag work, not60FPS qualification.

P12 detail follow-up, accepted within the bounded review below. No complete Windows
fidelity or phase closure.

`FontAtlas::ink_metrics` derives a stable capital/descender reference from the
cached H/g glyph bounds and hinted physical baseline. The reference is conservatively
rounded to logical units. Every label of the same style shares a baseline regardless
of the letters it contains. `centered_top` positions this reference in a control row;
the existing general TextRun top still means the top of its line box. Carets cover
the same reference with one logical pixel of breathing room, clipped to the line.
These methods inspect cached metadata; no glyph rasterization or allocation occurs.

Workspace command, popup, dialog-button, Explorer breadcrumb/sidebar/entry/footer
and Notes tab/menu/footer labels opt into this common row placement. Notes selection
and hit rows remain20DIP; caret and painted text use the same row reference. Blank
Notes keeps its caret visible. Files F2 uses a centered20DIP edit row within the same
26DIP details or50DIP tile; switching into rename preserves the baseline. The edit
border uses an existing theme-mapped neutral. Command icons and chevrons center in
the actual30DIP control instead of carrying independent offsets.

Files partial address/F2 selection now repaints the full original glyph run through
a clean selected-region clip. Repainting only a substring at a rounded prefix origin
lost cross-boundary kerning and blended its white edges over dark glyph coverage.
The actual painter is compared pixel-for-pixel with an independent native atlas run
for partial AVATAR.txt selection at all densities/themes. The previous painter fails
this regression; the repaired painter passes. The empty Notes caret oracle requires
every pixel of its strip to survive, not just visible endpoints.

The owner-provided installed Windows25H2 reference supplies two precise associations:

- Explorer Home: shell32.dll.mun RT_GROUP_ICON51380, as registered by the Home CLSID.
  Pinned ICO SHA256202b17d444dfca5555639969c40ec550b4ebeda955a1d86095781c9b19303bc4.
- Text files: installed Notepad11.2607.14.0 AppX DefaultIcon targetsize assets,
  eight authored PNGs, preserved exactly inside the pinned ICO. ICO SHA256
  c3bdd6f182c39d2363316edcc5230f806edcdfccd5e1e9785a858ec11f28f20c.

Per-icon `.source.json` records input paths, actual buffer hashes and frame counts.
The manifest binds icon/provenance filenames and hash; normal export/check is offline
and verifies those identities/counts. Native16/20/40DIP uses separate1x/2x/3x exports;
the nearest authored larger image is reduced only when an exact native frame is absent.
Only case-insensitive `.txt` is newly specialized. Other associations, executable
embedded icons and a complete registered-filetype provider remain open.

The explicit acquisition command reads resources as bounded bytes; it never loads
or runs the source module. The safe PE reader is separate from guest executable
admission. Root/section/resource extents, fixed directory levels, language choice,
duplicate leaves and per-payload/total budgets are checked. PNG and module reads are
bounded before allocation. The digest of the parsed buffer must equal the before/after
source identity. A conflicting existing manifest pin fails before asset writes.
Installed reference artwork is local development input; distribution rights are not
established by this acquisition. Microsoft resource-format documentation:
[icons](https://learn.microsoft.com/en-us/windows/win32/menurc/about-icons),
[resource files](https://learn.microsoft.com/en-us/windows/win32/menurc/resource-file-formats).

Host pixel tests compile the actual app painters and pinned native atlases/assets.
They cover command/menu/shortcut/dialog rows, empty Notes caret, text/selection/footer,
Files details/tile F2 baseline stability, both themes and all three densities. RGB
coverage is detected per channel: requiring all channels to be dark misses thin
subpixel stems. Native guest Notes/Files workflows remain required evidence.

To rerun these consumers without growing protected historical out, the exact
desktop-notes/files/remove commands (including their accelerated variants) also
select the existing retained-desktop v2 layout. They only launch private QEMU and
write run-local captures/disks; no build or new unbounded out writer is introduced.
The declaration/actual argv/path validators and legacy command reserve remain.
This extension has current native qualification; it does not alter the original
accepted desktop-current migration or its immutable receipts.

RGB subpixel text on opaque surfaces is already implemented by Door's renderer.
This work does not claim Microsoft ClearType output identity, Unicode/IME support,
new blink timing, vblank synchronization or60FPS. Recycle Bin, additional taskbar
preferences, modern/classic context-menu fidelity and the remaining app inventory
continue in windows-detail-followup.md and windows-core-apps.md.

## Additional reference observation

The owner reauthorized desktop inspection. The installed Explorer was inspected
through the Computer Use skill, with no file or setting changes. Its navigation/
address row precedes the command bar; Door's current order remains different.
The observed selected-folder context menu was classic/expanded and included
installed third-party extensions. This is evidence of that configured state, not
proof of the default modern menu or a clean Windows install. Settings launch exposed
no targetable window. Physical Escape then stopped Computer Use again.
The owner subsequently confirmed the classic menu was intentionally enabled.
Another authorized inspection attempt was still stopped; no Task Manager or
Control Panel state is claimed from that attempt.

Microsoft's [August17 context-menu announcement](https://blogs.windows.com/windows-insider/2026/08/17/improving-file-explorer-context-menu-faster-simpler-and-more-customizable/)
documents customization of built-in commands, extension placement, inline primary
actions and bottom Properties, rolling to Insiders. Preserve that rollout distinction.
The [composition architecture](https://learn.microsoft.com/en-us/windows/win32/directcomp/architecture-and-components)
uses retained batches, dirty-region composition, occlusion avoidance and vblank
scheduled flipping; [responsive UI guidance](https://learn.microsoft.com/en-us/windows/apps/develop/performance/keep-ui-thread-responsive)
also separates expensive work from input. Door retains window content but still
uses CPU GOP scanout with no vblank/page flip. These references inform the next
measured optimization batch; this alignment batch does not implement that driver.

## Qualified checkpoint

The independent read-only critic accepted reports/critics/p12-control-alignment-critic.json.
The unchanged product source is d40843fde9aff7f29b0c3a51ca254b3368e9f5c55b972b178cc07d1366a1e9fb;
the final host validator source is c646ba25e6119e45b2ec42579439b675414269505a75f9c69959863ae533c31d.
Both snapshots have2073 files. The only intervening file is the host Files fixture:
three border probes now expect the theme-mapped190, the tile border follows its
centered20DIP row at y230, and the dark caret probe samples its solid midpoint y228.
The original failed probes/captures remain retained. Runtime/assets did not change.

| Evidence | Run | Result |
| --- | --- | --- |
| Final host | host-1792575398772802800-24816 |668 tests /34 executables |
| Complete native desktop | host-1792571769273006100-24104 |253 consumer +2 maintenance checks /9 sessions |
| Notes1x /3x | host-1792572815571802500-21160 / host-1792572962584047200-25120 |39 consumer +2 maintenance each /2 boots each |
| Files1x /3x | host-1792574404677893800-12176 / host-1792574509439569700-9676 |19 consumer +2 maintenance each /2 boots each |
| Permanent removal1x /3x | host-1792574679342281900-4804 / host-1792574810294437100-10308 |15 consumer +2 maintenance each /2 boots each |
| Retained validation | host-1792575514788093600-10952 |Complete original evidence reopened; no new boots |

The final host run contains fixture-only-source-bridge.json. The full desktop run
contains guest-artifact-alignment-bridge.json: actual21 files versus the accepted
Storage build,19 identical, only Window/Workspace changed; all5 independent app
pairs match. Foundation inheritance is limited to those actual unchanged artifacts,
not a new42-scenario execution. Native audits independently rechecked captures,
actual PID/UUID/argv bindings, exact document identity/payloads and catalog/journal
CRCs across boots. Every qualified session reports zero leaks and DEVICE reclaimed8.

Two full host runs (6232 and11100) aborted in Windows ucrtbase with fatal application
exit0xc0000409/subcode7. Their cause remains unestablished; the code alone does not
prove a stack overwrite, allocation failure or cleanup panic. The final full run
passed. Exact xtask diagnostics also passed127 serial,127 parallel uncaptured and
the individual captured storage test. Failure reports and hashed diagnostic logs
remain immutable; see11100/host-crash-diagnostics.json. This is an open tooling risk,
not a claimed repaired cause.

Normal taskbar optical bounds were sampled using an explicit alpha>32 cutoff in
24104/taskbar-icon-optical-bounds.json. Compact Start sizing, shell caption rows,
Settings rows, all file associations, modern Explorer ordering/tabs/rounded fields,
Details columns and focused/selected outlines remain separate work. The owner's
additional reference index is reports/runs/user-reference-20261021-details/index.json.
Current Home still navigates the guest profile directory; this icon checkpoint
does not establish Windows Home's recent/favorite virtual-view behavior.
