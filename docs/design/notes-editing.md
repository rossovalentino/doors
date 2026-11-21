# Bounded Notes editing and durable document consumer

P12 remains partial. This bounded batch implements an original shared document
model and connects Notes to it. Host/native checks and independent review pass.
It does not establish Windows Notepad visual or functional parity.

The acquired Windows11 Notepad reference and the
[Microsoft keyboard reference](https://support.microsoft.com/en-au/windows/keyboard-shortcuts-in-windows-dcc61a57-8ff0-cffe-9796-cb9706c75eec)
guide Select All, Undo and Redo. The current ASCII consumer supports Ctrl+A,
Ctrl+Z, Ctrl+Y/Ctrl+Shift+Z, Shift selection, Ctrl word movement/deletion and
logical line/document Home/End. Selection replacement is one undo command.
History holds at most128 commands and65536 logical payload bytes across undo
and redo. Each typed character is a command; word coalescing is not implemented.
The document is capped at4096 bytes, admitting printable ASCII and tab/CR/LF.
CRLF is one navigable ending. Invalid or oversized edits leave state intact.

Dirty state compares exact bytes with the last successful save checkpoint.
Reopen refuses to discard dirty bytes. Load checks object metadata/version before
and after the complete bounded read and publishes a new document only after a
successful close. Save uses the real Storage write/truncate/flush/close path and
marks the checkpoint only after all succeed. The operations share the existing
staging/checkpoint policy; interleaved requests can force intermediate commits.
Compound atomic replacement and concurrent-writer conflict prevention remain open.

Visual lines retain separate hard-ending and soft-wrap boundaries. Tabs expand
only for display, preserving raw bytes. Selected glyphs use a clean accent
backdrop and the same full glyph run clipped to the selection, avoiding doubled
antialiasing. Native-density raster tests compare the result against independent
white-on-clean-accent painting at1x/2x/3x, light/dark, with nonempty glyph edges
and preserved viewport/scroll. This uses Door's current LCD cache; no Microsoft
ClearType/DirectWrite equivalence is claimed.

The new `desktop-notes` and `desktop-notes-accelerated` workflows use isolated
fresh volumes, actual keyboard input, whole-frame settled captures, visible
document comparisons and independent raw-disk inspection. They test selection
replacement, undo/redo, newline deletion, the dirty reopen guard, clean reopen,
light/dark rendering, save and a second boot. Both pass13 checks. No owner volume
is created or used. Shutdown validates the authenticated five base processes
plus two clients on the first boot and one on the second:7/7 then6/6 processes/
threads with zero leaked frames. Device reclamation separately frees8 DMA pages.

Still open: mouse selection/caret placement, vertical visual navigation,
clipboard, Unicode/shaping/IME, tabs, open/save dialogs, robust multi-document
close prompts and the modern Notepad layout. Existing close behavior auto-saves
the fixed document and cancels close on save failure; the narrower reopen guard
does not add a Save/Discard/Cancel prompt. Full app inventory and unified
Settings/Terminal architecture remain in windows-core-apps.md and the project
Windows app inventory.

Qualification bindings:

- Complete reproducible build22876: reports/runs/host-1792536807943522600-22876/report.json,
  SHA2564f35d4ef5a344858a57ec4af9657aefe211df00563edb1ff9e85d16d2ba3f751.
  Manifest b0d51657fb99a124b82240d580d84e23349eee73f855150809bb702851d858c3;
  initfs a273290737ef14c1f99a86a9951bcddd52f77dab36471366722242c301e628e3.
  Only workspace differs from accepted16216; its ELF is11,381,776B and rounded
  load sum/span11,325,440B, below16MiB. All26 artifact/pair files were rehashed.
  Unchanged core foundation42 evidence is inherited, with no fresh run claimed.
- Final host2116: reports/runs/host-1792537808178202700-2116/report.json,
  SHA2562daa3c39c17704f0ecd49e157cc13a861433768335710228c4aff21d5a38b91a;
  567 tests across33 executables. Affected host/native strict Clippy passes.
- Notes1x14572: reports/runs/host-1792537808120243300-14572/report.json,
  SHA2561fa7d7d973bfbec888e11db0c8ebb5ea83ab1ad8d3aa611218e8f7682f7ea60f.
- Notes3x16204: reports/runs/host-1792537808134859900-16204/report.json,
  SHA25638c6a1566eb21dc8f877992a70545b5ebb049d59913ff16ee551e58b2cc7b915.
  Each has13 checks/two boots and53 independently rehashed references.
- Combined6840: reports/runs/host-1792537048229967600-6840/report.json,
  SHA256136943c300abefbef661f4493ac067abd72d24a0e6032607f8020875c68820aa;
  243 checks/nine sessions,504 references and seven independently decoded disks.

Build source95e8fa2ae563c43cab4f7355483decc7ec377ba4f7133bf61626917836d633eb,
combined sourceea931b33aa8df0c78a5235d5eceb11f580b1e0eae09aecb354bce20e46430ff7
and final source6854f83cd20752cb505918ec729afff29bafc56600955d8f4bc9e9dcd0851e0a
differ in executable inputs only at host tools/xtask/src/desktop/notes.rs. Guest,
Cargo, assets and profiles are identical. Every report has unchanged start/end
inputs. The critic rehashed1,009 current executable/asset/profile inputs.

All six Notes snapshots independently decode to object26/inode17/generation1,
versions3/5/7, valid catalog/journal CRCs and clean journals. Both backends produce
the same whole-disk hashes at each stage: guarded14B `Original text` plus LF,
cf54593c44c2e709ad43212ef08bc91ba221e0b360131edc8e7ea4bd708302b5;
saved11B `Final text` plus LF,
8c574196ec30d41119d481371b704a7a3076fbb19acbb36d9645a5326bf0296e;
final20B `Final text`/`Reopened` each followed by LF,
41c0e6383e6379ed470ba67d21273077901b2be7cd048ac6d94bde6030422c35.
The native workflows do not inject I/O failure or concurrent-writer races.

Independent attributed review: reports/critics/p12-notes-editing-critic.json,
SHA256585bee910480b5c21d4ba76b16cdd685a240b2dd78f30353c41af9a3ad5a5dfd.
Both paint findings are repaired. Initial native5324/16328 failures are retained:
the critic incorrectly conflated8 device pages with7 processes; product cleanup
succeeded. Earlier host12772/14796 passes remain, superseded for the final oracle.
Fresh cumulative1544 (SHAa6fb813fbfd02aece2ebf879ae3516e4009f704aabeab16aaa8ee72bff0a21a1)
fails on the unchanged original missing P00 report. No cumulative/P12 closure.
