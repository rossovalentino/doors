# Checked Files mutations

P12 development, 2026-10-21. Same-parent rename is implemented and guest-tested.
Copy/move/trash/restore and full Windows filesystem compatibility remain separate
work. This batch does not close P12.

The installed Windows11 reference uses F2 inline editing, preserves a file's
extension in the initial selection and exposes Rename through the command bar
and target-specific context menus. The shared LineEdit keeps UTF8 cursor and
selection boundaries; native file identity never comes from the ASCII display
projection. Names that cannot be decoded losslessly are not edited. The native
48UTF16-unit component limit is explicit; this is not Windows' full name limit.

Rename uses a fixed128-byte request carrying parent/object IDs, the observed
version and lossless UTF16 units. These are optimistic identity hints, not
capabilities. Storage authenticates the peer, walks the parent's current bounded
ancestry, checks traversal/current DELETE/current parent CREATE/share-delete,
rejects protected or pending nodes and checks the observed source version before
changing the binding. A moved parent cannot bypass an unreadable new ancestor.
Case-fold collision checks use the existing declared ASCII policy.

The serialized Storage handler commits the same namespace/content/ACL journal
before replying. A transport or commit failure never produces a success message;
the UI distinguishes definite rejection from uncertain completion and asks for
reopen/refresh when appropriate. Failed commit poisons the owner until remount.
An existing file handle keeps its stream and object identity after rename.

Real namespace, content, ACL and attribute changes advance the source version.
Exact no-op names/policies do not. The older path and handle rename APIs now
advance it too, preventing rename-away/back from evading stale-entry detection.
Native handle case-only rename no longer collides with itself. Overflow is
rejected before mutation. No catalog format or authority limit is enlarged.

F2, the enabled command-bar icon, inline context icon and classic Rename row
route to the same transaction. Escape cancels without IPC. Validation/collision/
sharing/permission failures retain the edit. Successful refresh reselects by
object identity; filters cannot retain a hidden selection. Known-folder relocation
is disabled until a provider can update persistent associations and pinned links.
Clipboard/Share/Delete remain disabled until their actual contracts exist.

Starting rename reveals the selected filtered item in its current layout.
Resizing keeps that observed item visible or cancels the edit when no item slot
fits, so keyboard input cannot mutate an item through an invisible editor. Native
tests cover active resize, view-switch reveal and zero-capacity cancellation.
The GUI identity oracle retains a private disk snapshot before any rename and
compares object/inode/generation and exact payload with first-shutdown and reboot
snapshots. It also exercises the classic row and dark inline editor/caret.

Host tests cover malformed wire input, strict UTF16 identity, reserved/trailing/
forbidden shell names, byte-safe extension selection, stale versions and ABA,
collisions, sharing, permission changes, pending nodes, bounded corrupt ancestry,
a real parent move under a denied ancestor, overflow, binary bytes, journal
remount and failed-commit poison. Native GUI tests establish the real RPC path,
input/capture, separate guest boots and independent disk identity/content/allocation
audits. Native test commands are:

```text
cargo xtask test desktop-files --image <production-image.json>
cargo xtask test desktop-files-accelerated --image <production-image.json>
cargo xtask test files-sdk --image <foundation-image.json> --baseline <private-saved-sdk.raw>
```

The first uses an isolated1280x720 TCG guest; the second an isolated4K300%
WHPX/RAMFB guest. Neither uses the owner's volume. This finite work does not
establish all File Explorer functions, Unicode shaping/IME or Windows1:1 fidelity.

## Current evidence

The complete independent build in `reports/runs/host-1792524945754259600-13324/`
reproduces all five application pairs. The host suite in
`reports/runs/host-1792524945731516700-4824/` passes 531 tests across 33 executables;
strict host/native Clippy also passed. The 104 affected namespace/protocol/UI tests
are part of that full suite.

Both qualifying GUI reports pass 19 checks with unchanged inputs:
`reports/runs/host-1792523990852207500-21108/report.json` at 1x TCG and
`reports/runs/host-1792523990903138900-16640/report.json` at 4K/3x WHPX/RAMFB.
Each exercises all four entry points, light/dark state, resizing/view changes,
invalid/colliding names and two boots. Independent retained disks preserve the
original object 26, inode 17, generation 1 and exact 28-byte file content.
The combined desktop regression in
`reports/runs/host-1792524027514902100-19056/report.json` passes 242 checks.

Those GUI runs use full build 16368 at source
`2f75f8a8b1b59d4366437edd0ec59aedb33c9058093374c4a20306e3c0803f9e`.
Final build 13324 at source
`6a2186673ea0e2da07dd843b054b9d19c2edc09f6a77fc86c935092b672ad6dc`
changes only the SDK fixture and four host-tool files. Every application pair,
firmware/kernel and production initfs is byte-identical; product initfs SHA256 is
`7117756ad6b3a543ec1aadc64a421899336e6ca820c398da2e1a09faebd49522`.
Fresh adversarial recovery against the changed test image passes in
`reports/runs/host-1792525116311615100-23568/report.json`.

The explicit SDK console exercise in
`reports/runs/host-1792525250421265300-17776/report.json` passes with an authenticated
retired workload identity and zero leaks. It rejects a stale version, a live
share-delete conflict, invalid component and a guessed foreign parent, then
roundtrips a raw UTF16 name while retaining its open stream. The raw disk audit
requires checkpoint 3→5, catalog revision 57→59 and source version 23→25, unchanged
inode 13/generation 1 and all 4096 independently computed pattern bytes. Only the
two version counters, catalog CRC and journal may differ.

The SDK exercise is an explicit command on its own private clone. An earlier
unconditional placement changed the legacy save/reopen contract; failed foundation
report `reports/runs/host-1792523990931422400-8644/report.json` is retained. The
corrected full rerun in `reports/runs/host-1792524945730431400-8952/report.json`
matches all 42 expected foundation scenarios plus the original diagnostic and
service-console demos. Existing read-only reopen/restart and exact checkpoint
assertions remain intact. Earlier failed GUI gestures and development relinks
remain evidence of the repairs, not qualifying builds.

Historical cumulative verification still cannot pass because its original P00
report was externally removed. Current focused reports do not replace that chain.

Independent read-only review is saved in `reports/critics/p12-files-rename-critic.json`
(SHA256 `fc30a9c03017a7ce89c61efad10591235b1f38786b590a3330b6584561a6d09c`).
It found no unresolved actionable issue in this finite scope, independently
rehashed bound artifacts and decoded retained disk bytes. It is not P12 closure
or a guarantee that all defects are absent.
