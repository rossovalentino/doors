# Checked permanent Files deletion

Availability update: after this completed review, the owner cleared out to reclaim
space. Original report/critic bytes remain unchanged, but deleted out dependencies
cannot be revalidated. This is historical bounded acceptance; new evidence uses
fresh IDs. See reports/artifacts/p12-output-availability.json and docs/build/output-storage.md.
The new complete desktop/foundation runs and out-only20GB maintenance have bounded
acceptance in reports/critics/p12-output-budget-critic.json; original execution receipts
and the historical deletion review remain unchanged.

P12 bounded checkpoint independently accepted after the document dialog checkpoint.
Checked permanent removal and the default-Cancel Files modal pass631 host tests,
42 expected foundation scenarios,253 complete desktop checks/9 actual sessions,
SDK three boots, deletion GUI15/two boots each1x/3x, Notes39 each and rename19 each.
The independent review is reports/critics/p12-files-deletion-critic.json,
SHA256d085856ad7ea987b2bb4ebb2f6f959a50fc5f0142e38e1c282d635940a365133.
P12 remains partial; this does not establish complete Explorer or Windows fidelity.

The first operation is explicit Shift+Delete permanent removal of one file or an
empty folder. Ordinary Delete/Recycle Bin needs a separate durable trash/restore
contract. The confirmation will use the shared modal, identify the captured item
and default to Cancel. It must never turn a changed selection into a new target.
Recorded installed Explorer references and Microsoft's [dialog guidance](https://learn.microsoft.com/en-us/windows/apps/develop/ui/controls/dialogs-and-flyouts/dialogs)
inform the control. [Closing/deleting files](https://learn.microsoft.com/en-us/windows/win32/fileio/closing-and-deleting-files)
and [shell undo semantics](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-shfileopstructw)
are separate contracts; permanent deletion is not secure erasure.

Opcode0x2017 carries exactly24 little-endian bytes: nonzero parent/object/observed
version. They are comparison hints, not capabilities. The authenticated Storage
owner validates current bounded ancestry/TRAVERSE, current DELETE, protected and
pending policy, exact parent binding and version, symmetric SHARE_DELETE and
absence of all children, including hidden pending children. Directory listing
visibility cannot establish emptiness. No parent CREATE grant is needed to delete.
Rename reuses the extracted ancestry helper without changing its policy.

The owner preflights, commits unrelated staging, rechecks and stages the removal,
then durably commits before success. With compatible open handles the namespace
becomes pending/hidden while the backing stream stays alive until final close;
mount completes persisted pending deletions after handles disappear. A failed
unlink poisons the owner after namespace mutation, even if the backend says a
NoSpace rejection changed no filesystem bytes. Otherwise a later unrelated flush
could accidentally publish the failed removal. The path-based remover shares
this stronger post-mutation guard. No disk format or dependency change is added.

Current host cases cover stale/namespace ABA, sharing and live stream lifetime,
current ACL/parent/corrupt ancestry/counters, an actually moved denied ancestor,
nonempty/pending-child folders, all five journal interruption boundaries with
idempotent remount, commit-failure poison, pending-delete reboot reclamation and
preflight-unlink NoSpace after namespace mutation. Wire tests reject zero identity
and short/long records. These are host results, not native execution.

Twelve removal host cases also cover deferred-last-close NoSpace, revision
exhaustion after persisted pending deletion, final journal-generation prelude
isolation and independent raw bitmap/free-counter plus unrelated binary payload
and identity preservation. All76 storage-core and64 Workspace model tests pass.
The shared Dialog accepts an explicit initial focus without changing Notes' default.
Files captures parent/object/version and name once; Enter initially cancels.
Definite permission/sharing/nonempty errors retain that observation and restore
Cancel focus. Changed/missing/pending and uncertain outcomes show a single Close
notice and require successful fresh enumeration before another removal. A known
durable acknowledgement completes the intent before the fallible listing refresh.
Keyboard input revokes any held modal mouse press, including when an error replaces
the dialog without changing modal=true; a later release cannot retry Delete.

Only Shift+Delete in a listing activates permanent removal. Standard known folders
and protected objects remain unavailable; ordinary Delete/Recycle Bin remains
disabled. Current confirmation names are restricted to32 printable ASCII characters
so their exact spelling and short warnings fit the minimum240x178 logical client.
The backend accepts numeric observed identity independently of presentation spelling.
No recursive removal, undo, secure erasure, trash/restore, Unicode GUI confirmation
or complete Explorer fidelity is claimed.

## Accepted final integration

All final reports below have unchanged source
68f613962734bb11949bab2a9fc47502ca38c11999d1b7216e0d300055dc1099.
The current build independently reproduces every one of build5808's21 artifacts
and five application pairs; its initfs is
e1d92788b7dc6a74546f5b22b66cb7e597650c7dcc919f97d74fb4d168e4a27d.
The SDK/deletion GUI earlier-source bridge is independently reviewed below.

| Evidence | Report | SHA256 |
|---|---|---|
| Current host631/33 executables | host-1792553435297672200-22156 |a85a7dff622bc933bf2d10545248b6bf69e708ea9ab164d9e9d80961b0d5be5e|
| Fresh foundation42 expected outcomes | host-1792553453965410200-17560 |81e08e36f0f7a3423c224273192925e3f80362ffe51a0ce6044b3f15e74519d5|
| Reproducible build and complete desktop253/9 actual sessions | host-1792553435241221300-12020 |0de94c7369c551eff3289863a66292c64c730d0033f5dd3b64db6d1749ac35b9|
| Notes1x39/two boots | host-1792553675090894500-11764 |5087271460530d6ee780fd65494bfa205ed3e58cd90582a158802508a116ffb6|
| Notes3x39/two boots | host-1792553675106269500-10852 |6c8f3421b2b40ca395bac8bbbcac8d5b85d02273297d15d1a6b8cb0d386d8c4f|
| Rename1x19/two boots | host-1792553848265104000-11100 |18371a5d442ca7d113aa7baccf7cb30f0379aafddb2a946c9e24ebf123fef05c|
| Rename3x19/two boots | host-1792553981575661300-9252 |77c597845ed95ed95ddd98772713a7f3c8cd14a036172062ce5980e7983a6a6f|
| Strict retained validation, no new guests | host-1792554451739835500-13940 |d7ee39db0d2bccbceca19e6f23941b6df13f8c52babc12ac0357becaa0fb7b97|

Build manifest SHAa49e2f07e17c418095515850177931d93cef6952a1ed6141bd2e02327a30b895;
boot-suite SHA697cc30c29d3c6c8b295b36420b2a4925627b1fcbc4cb27c3c1b6d227ed52a91;
desktop-suite SHAc7d4c1021a14aa6bc28bdda3de2b6884e217e8f15218b0695d5fe4c8334ce03a;
retained checkpoint SHAcc9b9a5e6e0628f6cb5a53c28d2d1f03703ee438dcb0c19933b8156eb5c79a9b.
The critic independently rehashed788 combined references,9 actual launch receipts,
27 private image entries,178 exact PPM/PNG pairs,176 pixel probes and8 raw states.
Foundation42 actual receipts/126 private image entries and32 core files validate;
all expected27Pass/6Panic/5Interrupted/2StartupFailure/1GuestFailure/1Timeout match.
All12 Notes raw states and3 rename raw states match across density. Root also ran
strict full-workspace/all-target Clippy, affected native Clippy and format checks.
Workspace rounded load11,362,304 and Window6,778,880 remain below16MiB. This is not
a frame-rate claim. The reviewer separately verified current canonical shutdown.

## Build and early native evidence

| Evidence | Report | SHA256 |
|---|---|---|
| Reproducible guest build | host-1792552613701479600-5808 |19b4e897f3bf7b782669737bbd3c5fbec12646ecff72a3e916ea3d15eac61375|
| Initial host630 tests/33 executables | host-1792552626370770800-7084 |acc917af6d318ee62e7670ed08d6ed6c4e05d1181b42f00eb36ccfc256745466|
| SDK three actual boots | host-1792553040234287300-18096 |2f6a1a3b7daf5072695c353604eedd20c8a684e67918950701ae91bd2a2c20b5|
| GUI1x15 checks/two boots | host-1792553040201685300-26476 |31b949b16b100f61d07478eed14989f83b643fa95c06ce785d730bb7b8db694b|
| GUI4K3x15 checks/two boots | host-1792553040201647700-24584 |318902aa505d501583023d3adae853015fa12824c98ff1465ed6c63e5e905dd6|

Reports are under reports/runs/<id>/report.json. Build/host source is
36471bed3ae63ad0c47e107b2582dd0fa91434ebc897be93af638cd3df2f60a9;
native fixture source is8c435b6207ea20fa40d92e66c9b8103e72a1b242ad5e46c27e21baeb3c982be1.
Only three host-tool files changed between those source snapshots: checked_remove,
file_deletion and shutdown. Guest artifacts remain the same build5808. A later
host-only guard additionally rejects Files markers before launch/after retirement.
The final qualification above binds that source and its independent guest rebuilds.

The SDK binds pending object21/version4/17binary bytes to authenticated SDK output,
checks exact temporary-object allocation order, all baseline namespace/ACL/settings
bytes and unrelated backing payload/identity/block lists. Live close and read-only
reboot retain catalog69 and the same raw hash8d557ef66d190d25c1eb7953a7af4885fae18de7186714137439b68d0f2155f5.
Reboot from the actual paused pending image reclaims its stream at catalog68,
raw47a3cde84d7dee527ffaac9314b6306a2f13d54a9b34e1ab3570972cfb65f6a3.
Each SDK boot retires5processes/7threads, leaked0, DEVICE8. GUI boots retire8/8
then6/6, leaked0, DEVICE8, with unique kernel identities, Init launch mapping,
Files marker authority and ordered service shutdown. Dark panel43 and focus240
are sampled from actual Files pixels; minimum question/name/warning remain visible.
The root visually inspected the retained minimum1x and dark3x PNGs.

Retained failed attempts (no promotion): SDK parent267869/child268765 reached the
real pending state but its suffix-exact waiter was incorrectly given a short prefix.
Their hashes are225025b70fec8fe1bda1f0b78fe7f7477c1a21955673d9216efbe2c76415b399
and af1602181104df545202cc748e1da46436c96ae75758e1dbe70fea73da633b48.
GUI15156/9880 probed a rounded focus-button corner, correctly seeing white; the
straight border was independently sampled and repaired. Their hashes are
a4f5b87bc6f4cfa13a7b43d7999d8478e3d33ef56e8c89790f8689ca5c313b16
and978ed1f5f58a2ab73c0b970de9a5efe86d5d448aec0a7e8565a79109389ef809.
Full failed IDs are host-1792552678696166600-21080,
host-1792552687650423500-21080, host-1792552726047432000-15156 and
host-1792552726067163200-9880. Another retained attempt,
host-1792553435241115000-23932 SHA c103819aefd42d54ec01b25f1e0f9abcc0fe64353e2e11be51fa5af033db8b25,
refused the unimplemented P12 boot schema before any commands/guests. The fresh
foundation uses explicit `test boot --through P11`; P12 is not certified closed.

Native verification commands:
`cargo xtask test remove-sdk --image <image.json> --baseline <saved-private.raw>`
retains prepared/pending/final raw images, binds pending identity to kernel-attributed
SDK output, verifies malformed/foreign/sharing requests, and runs separate read-only
and pending-reclamation boots. Its surrogate-name case is a checked rename roundtrip,
not deletion while the surrogate spelling exists. `test desktop-remove` and
`test desktop-remove-accelerated` exercise actual1x/3x input, safe Cancel/no-write,
held-button cancellation/rejected-action release, minimum client, nonempty folder,
empty folder/file removal, light/dark and separate reboot. Reports above qualify
these finite paths; native injected I/O failures remain open.

Storage transport reconnection and poisoned-volume remount are separate follow-ups.
A successful fresh enumeration is required after an uncertain removal; this guard
does not itself repair an unavailable connection. Do not automatically replay an
uncertain save/removal. Session revocation and preserving RAM across security-session
loss must not be conflated with a connection retry. Legacy generic handle-based
replacement rename still directly reclaims a staged target: a generic Vfs definite
NoSpace can leave that legacy owner reusable. Actual Ext2/Journal writes poison
their cache, and checked deletion never calls that path; the critic found no
production checked-deletion blocker. Keep this as a separate robustness follow-up.

The selected cumulative gate still fails the original missing P00 report,
reports/runs/host-1791804427914736400-32560/report.json. Current attempt
host-1792554561349782200-24044 SHA85798f304583c606bb8d65c728a3e7a910b05e4ade3dd11724bbfb2add3b5330
has unchanged68f source. Preserve the missing historical evidence; no P12/full
Windows completion or cumulative acceptance follows from this bounded checkpoint.
Post-review changes coordinate documentation/metadata and the attributed critic
record only; the accepted runtime remains the qualified68f build.
