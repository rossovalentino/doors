# Checked compound document replacement

P12 bounded implementation and native evidence are independently accepted below. The accepted
Notes document and measured navigation checkpoints remain separate. This work
does not establish complete Notepad, Windows file semantics or P12 closure.

The previous editor sent write, truncate and flush as separate authenticated
requests. The service can checkpoint between requests. Saving also lacks an
observed-version check. A checked bounded replacement stages at most4096
candidate bytes outside the filesystem, then authenticate the file capability,
verify its expected object/version and replace its bytes/length/catalog version
under one serialized durable journal commit.

Four static service staging slots bind candidates to channel, kernel peer
PID/session/epoch and open-file token. Begin validates bounds and write authority;
ordered chunks cannot mutate the volume. Commit requires the complete candidate
and repeats identity/version/authority checks. Abort, close and disconnect discard
the private candidate. Slots and identifiers are bounded and never wrap. Ordinary
legacy file I/O remains available; this operation does not change its contracts.

The core first commits unrelated existing staging, then performs one bounded
replacement with one object-version increment. Both old and new files must fit
4096bytes. Metadata, payload and length share the existing redo transaction. Any
failure after replacement starts poisons the owner until fresh recovery. Success
returns precomputed committed metadata, without a fallible post-commit read.

Notes retains the object/version observed when loading or saving. Existing
documents use OpenExisting; a missing document uses CreateNew, so another file
appearing at that path cannot be silently opened and overwritten. A conflict
retains unsaved RAM bytes. Acknowledged committed metadata is retained even if
close subsequently fails. The durable byte checkpoint advances on acknowledged COMMIT. Separate ClosePending attention guards save/reopen/close until close succeeds; an unchanged retry verifies metadata without rewriting. Undo compares against the new durable bytes. Uncertain commit outcome survives Undo to the old checkpoint and Changed retries; byte equality cannot bypass the guards.

Limits: this is stream replacement on an authenticated open file, not atomic
namespace creation or Save As. A failed first save can leave a newly created
empty file. An uncertain commit may have persisted the complete replacement;
the editor must retain RAM and avoid a blind stale overwrite. No full-size
transactional file API, multi-file transaction, Unicode/IME, clipboard, conflict
dialog or physical-device power-loss guarantee is implied.

Required checks include wire/ownership/staging negatives, actual core replacement
and stale/ACL/sharing/counter cases, shrink/empty/max bounds, journal interruption
and I/O poisoning, private SDK cross-connection tests, independent raw-byte and
identity audits across separate guest boots, Notes1x/3x regressions, a fresh full
foundation suite and independent review. No owner disk is used.


## Accepted bounded checkpoint

Independent read-only review accepted this batch with no unresolved actionable
finding. Record: reports/critics/p12-document-save-critic.json, SHA256
7135f4ef2ae93662a1fed477a3f48fcd238cf670dbed09eb08e664892acbd60b.
P12 remains partial; no phase or cumulative closure follows.

Build/focused GUI source: 585bb6c57a70d46b5054c7f0c7119f988a4ec26e7ccf65749a30bf67759d4708.
Final host/SDK/foundation/combined source: 8e8a7f95b04b145bbc60831b0765f665b7b5f5502199bc8372a49e10927fef12.
The sole executable-input bridge is tools/xtask/src/services/checked_replace.rs:
add the existing threads command to both SDK boots and skip its disk snapshot.
Guest artifacts are unchanged; the critic rehashed2012 final snapshot files.

| Scope | Report | SHA256 |
| --- | --- | --- |
| Build | reports/runs/host-1792541350568704600-22924/report.json | ee27b13801f16250fd25f37316cdefc369afac97ed7c983610797564e55d291e |
| Host595/33 | reports/runs/host-1792542217145977800-25236/report.json | 5867ecc809bff5ddf11b9f52dd4aa2d590475b9df30cd87acb0dea4a1733424d |
| Notes1x26/two boots | reports/runs/host-1792541461932981300-9276/report.json | 4fab22c7385420d9e4b45180beef6c7cd222d261d4e0e9d68157d2d01e24e161 |
| Notes3x26/two boots | reports/runs/host-1792541461963270400-24428/report.json | 8283a4b072739e529f742e99ae0b2fafbc70927e49124067a67f7c5304f21bfd |
| Files1x19/two boots | reports/runs/host-1792541730462262600-26124/report.json | 88a59c62b4eb7f195ea39cd517e6d39118d121b3e2f99db57269c666b1643ba0 |
| Files3x19/two boots | reports/runs/host-1792541730466082500-584/report.json | 00fdb519ae025fc34f1effd2dd7feba247b53e0aabf0cf253c931fac336b9f4b |
| SDKtwo boots | reports/runs/host-1792542217178513100-12460/report.json | ca9e6438b2a143833d419043d7fc450c4ae576bffde9d5f93e025b4a9128b129 |
| Foundation42 expected outcomes | reports/runs/host-1792542217211656500-26940/report.json | 1e9f99f889c1f2e5d6aef656b1eadd3a952bfd7bd6a89adb10be275cee9ae384 |
| Combined243/9 sessions | reports/runs/host-1792542217146676900-22836/report.json | 24f346ea0d815a10dd7ec4fb6773fa7bdfc9d86a2545ead078e9e4837d777bf5 |
| Cumulative (failed original missing P00) | reports/runs/host-1792542970032920000-21496/report.json | 4f95eeefd6a2a437f2e2d48cab3652034cea4983f6cc67501242547eb159698f |

The reproducible build's manifest SHA256 is 9d44c8e559df74d8089504743f0b91dd2bd0516ad39be5719cf0128ad523e9bc;
product initfs SHA256 73e4b28b1a149d06255e560d50e8cd3b0caef0ba6c3cbe013c0a4856443a6666.
The critic checked21 artifacts plus5 independent application pairs (26 distinct
files), and the native Workspace mapped span11,341,824bytes remains below16MiB.
The final host suite counted595 tests/33 executables. Nine actual core tests cover
max/shrink/empty/regrow, identity/version/grant/ACL/counter preconditions, all five
journal interruption boundaries and short-I/O/NoSpace poisoning. Three Document
SaveState regressions cover acknowledged commit plus close failure, uncertain
commit plus Undo/stale rejection, and definite versus uncertain errors.
These new crash/I/O and editor failure cases are hosted, not native injection.

The native SDK uses authenticated raw IPC for foreign-candidate negatives,
ordered/incomplete upload and lifecycle checks, plus real stale-writer tests.
Both boots retire5 processes/7 threads with zero leaks and8 DMA pages.
Five independently decoded raw snapshots preserve object19/inode13/generation1:
version23/catalog57/journal3 -> short26B version27/catalog61/journal7 -> restored
4096B version28/catalog62/journal8. Four restored/reopened disks have exact SHA256
3fea4d5d1c060dbdb42bc63db8474c0cf31509d3753ce018d7a03fe4a5e3e3ce.
The short payload is inspected while the first boot is paused; the separate
read-only reboot reopens the maximum restoration. Catalog/journal CRC, EOF zeros,
allowed byte changes and unrelated partitions were independently checked.

Notes at each density passed26 checks across two boots:7 decoded raw snapshots,
11 pixel equalities, exact caret and held-drag cancellation probes, real saved
bytes and light/dark bodies. Files at each density passed19 checks/two boots,
6 decoded raw snapshots and stable rename identity/payload. The critic rehashed
75 Notes and59 Files references per density.

The fresh foundation suite matches42 expected outcomes (27 Passed,6 KernelPanic,
5 Interrupted,2 StartupFailure,1 GuestFailure,1 Timeout), with zero mismatches.
The critic verified845 references,42 argv/private-image bindings and126 image
entries; both independently built16-artifact core passes equal the qualifying
build. The combined suite passed243 checks in9 sessions and504 referenced files,
with clean retirements/device8, first-frame diagnostics retained within budgets,
and kernel-attributed legacy24/32/40-byte binding presentations. Existing standard
console and save/reopen/restart exact raw-disk invariants remain intact.

Two earlier broad attempts are retained:3108 failed explicitly with ENOSPC after
191 checks (SHA 762697b1c940c2b7fe29bc54446bf4295358c1806b319f5858ec4888196b56fd);9260 lacked a child evidence file
(SHA 9bf4753c4253c36e9461db4690a0e04e7f23741d89327447a1986da7dc68f11d), although its final guest exited33 with clean
retirement. The latter is consistent with disk exhaustion, not demonstrated guest
failure. Reversible NTFS compression of499 inactive private raw images freed space;
no files were deleted and the owner disk was excluded. The complete reruns above
supersede these attempts. Original missing P00 evidence remains a cumulative blocker.

No new dependency/version/feature or Cargo.lock change. Static candidate staging
extends the existing guarded SCRATCH UnsafeCell with its ownership rationale;
there is no new unsafe impl. Resolved review findings include advancing durable
bytes on acknowledged COMMIT, keeping ClosePending/Uncertain attention separate
from dirty equality, and proving foreign ownership at the server boundary.

Save/Don't save/Cancel, conflict recovery and Save As remain next P12 work.
No physical-device power-loss, complete Windows semantics, Microsoft ClearType
engine, frame-rate or full Notepad fidelity qualification is implied.

