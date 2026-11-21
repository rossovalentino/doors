# Explicit document close and reload

P12 bounded checkpoint independently accepted. The final host suite passes613
tests; the reproducible desktop build passes253 checks across9 actual native
sessions. Dedicated Notes workflows pass39 checks/two boots at each1x/3x;
Files regressions pass19 each. The checked replacement checkpoint remains
separate. No full Notepad or phase closure follows from these controls.

The installed Notepad package was inspected earlier. Further host computer input
stopped on the owner's Escape. This batch uses the recorded baseline and Microsoft's
[ContentDialog reference](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/dialogs-and-flyouts/dialogs),
accessed2026-10-21: one modal per window, explicit action buttons, safe Cancel on
the right and Escape to cancel. This is an original generic control, not a claim
to reproduce modern Notepad's optional session-retention behavior.

A shared allocation-free Dialog model controls Tab/Shift+Tab focus, Enter/Space,
Escape and measured button hit regions. Painting uses the same Layout, an opaque
rounded panel and density-specific hinted glyphs over a dimmed client. The compact
240x178 client keeps its question/warning and three horizontal buttons. Text is
converted by Unicode scalar to the toolkit's explicit ASCII fallback before its
byte-based wrapping; this prevents UTF-8 slicing panics without claiming Unicode.
Paragraphs wrap at measured word boundaries, with scalar-safe long-token fallback.
Button borders use explicit theme foreground/surface roles for contrast.

Notes holds Close or Reload intent separately from dirty bytes and from the
checked-save ClosePending/Uncertain states. Close with attention first cancels
the Window server's close request, preserving the existing native window. Save
uses actual checked replacement and only continues on success. Don't save/close
does no new Storage write; Don't save/reload publishes only a coherent successful
load. Cancel retains the document and history. Failed save/reload retains the
modal, intent and current RAM; reload failure is reported even if Save succeeded.

Approved close blocks subsequent editor input until the server delivers CLOSE.
If the service disconnects after acknowledging Close but before delivering its
event, recreation restores a visible Close prompt with the document retained.
All keys, text, context menus, pointer editing and shortcuts are trapped behind
the modal. Captured releases use the existing same-target activation. Both opening
and dismissal clear old pressed/hover/capture state; resize and trusted transient
dismissal cancel capture while preserving the dialog. Other application windows
remain usable. Repeated close keeps one existing prompt and its original intent.
Modal state forces the arrow even when opening by keyboard clears hover, avoiding
a stationary I-beam over the confirmation. A rejected shutdown close aborts the
current power sequence through the existing server contract; the user can request
shutdown again after resolving the document.

Close also retires the app's local uncommitted-attachment hint because the
compositor cancels that attachment without a producer release. A COMMIT/Closing
reply and an ordered CLOSE event both clear the hint; published-buffer ownership
and availability remain unchanged. After canceling Close, Notes and Settings
re-acknowledge the current configure epoch so an interrupted ACK can resume.
Without these transitions, a repaint could skip ATTACH and fail InvalidRequest,
terminating an edited document. Actual repeated-Close native failures are retained;
the repaired guest passes the fresh qualification recorded below.

Limits: fixed document only; no Save As, full file picker, conflict-resolution
workflow, accessibility tree, Unicode/shaping/IME/clipboard or modern Notepad tabs.
An uncertain prior commit may already be on disk; Don't save discards this window's
RAM, not that committed transaction. Initial CreateNew remains separate. Actual
injected save/close/reload I/O-failure GUI coverage is not established by model tests.

Required evidence: host control and intent tests; strict native checks; reproducible
guest build; Notes actual light/dark1x/3x workflows with Cancel/Save/Don't save,
minimum client, capture/focus/resize and exact durable bytes across reboot; Files
regression and combined desktop; independent read-only critic. Unchanged core,
Storage/service and Window binaries may inherit the current foundation only with
explicit artifact identity proof. No owner disk or further host UI control is used.

The host shutdown fixtures now choose Save in the actual fresh Close prompt,
wait for that native client to retire and request power again. The canonical
validator derives actor/incarnation/session identity, Save order and exact kernel
retirement/device counts from sealed serial, instead of trusting summary flags.
Both native-density boots are checked explicitly alongside the two other modes.
`cargo xtask test desktop-current` builds twice and runs the complete current
desktop batch under one source snapshot. `cargo xtask revalidate desktop --report
reports/runs/<id>/report.json` reopens that immutable successful evidence and writes
a separate report, distinguishing guest execution source from validator source.
It does not execute guests. Power gesture counts remain controller observations;
guest Close requests do not establish the number of power gestures.

Current validation also waits for the complete newline-terminated final kernel
device record before sealing serial. Maximized captures use four identical whole
frames. The Settings probe includes newly exposed lower rows; Files/Notes use
the body margin because their lower edge contains real footer controls. The old
2x partial-scanout frame is retained, and an unchanged-guest focused run shows
the full243 background at1400/1424/1450/1500 pixels. Modern Settings15-page
coverage is derived from captures/serial as well as the summary, preventing a
changed count from bypassing the Display/Installed apps/About detail evidence.

## Accepted evidence

The final execution source is
`5a26471576fe6d1453b4b6140d3cacb773c4643a3f98078a6d2071cc2ecf78cb`.
Dedicated GUI source2ec1255b differs only in reviewed documentation and six host
files. All21 final guest artifacts equal27548 byte for byte; all5 independent
application pairs match. Twenty artifacts also match the preceding22924 build;
Workspace alone changed. Thus the42-case26940 foundation is inherited only for
unchanged runtime core/services/Window, not represented as new execution.

| Evidence | Run under reports/runs | SHA256 of report |
|---|---|---|
| Build +253 complete checks/9 sessions | host-1792549001362687000-624 | f97d5a41d15d6d8d4b533bd3628b4e7c2b87cb0225c68bf13acd315a2180af3d |
| Host613/33 executables | host-1792549001382234400-13184 | 03d719ebf017ca6a1089d66e86279fb49470410e10d5d24877c7bd32da85c35e |
| Retained-evidence revalidation | host-1792549985477805800-3528 | 822ffdbad27bb79906f4565bab29fafce2c7dc2f9a7d1b90acd7980b3ba534e1 |
| Notes1x39/two boots | host-1792545881713124100-26836 | 066f114f270a4b822290ca4403cff3573adeb4e82bcfa40a278ae7218fb2752a |
| Notes3x39/two boots | host-1792545881673347000-11840 | 09aa3b8b7f9cc2ce3a6fa8d5ad518a1614296455a041f6b75e40342ae6c42567 |
| Files1x19/two boots | host-1792545881719104500-20356 | b17e8844d02ea57d3889dd75a1fe2b2185cc295e97c1fec9263d6c839e13702d |
| Files3x19/two boots | host-1792545881783842800-20960 | 1f561e01baadfffe8436b1f78c252a6d6d1fc70fc5fef21ba5c308a5d9deca46 |
| Focused2x readiness11 | host-1792548771301253500-24944 | 9b233be63d179e355ea8c103ac907fd4d96d1e2a0095f7c7a3cf93b9393724c6 |

Build manifest SHA256 is bde52a117f079288875a6f60c9f4933f5f248de56afc50fbb132683684cf76c7;
production initfs SHA256 is cc1b4e38845a74d384e6fc99bc08d4e49be82589328cad5e1f988d546e48ccfa.
Workspace's mapped span11354112bytes remains below16777216. The revalidation
checkpoint SHA256 is235c0fbeffa4f0f7dc0ab97eac0ce7f9e63b30256387e51be78585826eee3976.
It retains the original report hash, execution/validation source and12 actual
ordered event references, while executing no new guests.

The independent critic rehashed763 references,33 image entries and9 actual argv
receipts; compared178 PPM/PNG pairs and176 pixel probes; decoded8 raw disks with
valid catalog and both journal-header CRCs. Combined shutdown preserves object32/
inode17/generation1 and version28, with catalog/journal868/79 to883/94 after
reboot. Dedicated Notes retain object26/inode17/generation1 and exact corresponding
images across densities, including no-write discard pairs. Nine retirement counts
are7/7,7/7,10/11(recovery),18/18,7/7,9/9,9/9,8/8,8/8, all leaked0/DEVICE8.
The legacy standalone report's generic guest_tests field is not_run; actual native
execution is established by those nine bound launch/image/serial records.

Attributed independent acceptance is in reports/critics/p12-document-dialogs-critic.json,
SHA25657bd280f5265c24d09d6287b00f60a795fec89962561746419d95dfa2b7ffe85.
Its11 retained failed attempts include real repeat-Close crashes24092/19840,
fixture ordering/geometry defects, unresolved prompt22716, old Settings validator
27600, partial scanout21356 and the incorrect footer probe25112. None is promoted
to success. The agent-stopped25384 private guest is retained separately from a
product failure. No files were deleted and no owner disk was used.

Post-qualification changes are this coordinated documentation and the attributed
review record only. The original missing P00 archive remains a cumulative blocker.
The post-documentation `cargo xtask verify` failure is retained in
reports/runs/host-1792550616205521500-25056/report.json,
SHA256470f91953f58ebc4e55c8a05148b4c0b09af357ce46f9a9a7d4e98c6a8577f95,
with unchanged inputs and that same missing original report. It is not a phase pass.
