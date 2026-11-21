# Retained desktop output layout v2

The original P12 support batch is independently accepted within the bounds below;
no phase closure. Its exact `cargo xtask test desktop-current` migration was later
extended to the six standalone consumers, then validated interactive launch and
the default/software consumers documented below. Other build graphs keep legacy
placement and the8GB admission reserve; the explicit initializer has a fixed-size
reservation. This avoids
changing foundation's parent-owned storage fixtures used by child boot processes.

New builds, staged images and private QEMU trees are created directly under
`reports/runs/<owner>/generated/{builds,images,qemu}/...` before tools
execute. Owner identity appears once: build leaves are guest/desktop-userland,
image leaves are core/desktop/desktop-test, and QEMU leaves are session-stamp.
Full private instance identity remains declared in each launch. Nothing is moved
after execution. Compiler outputs, actual argv, sealed serials and retained
references therefore describe the same paths. Existing disks and screenshots
already under the run root retain their layout. Staged-image manifests remain
content identities independent of layout or owner.

Run, desktop-build and native-session records declare version, kind and owner.
Retained validation rejects unknown/mixed declarations and checks generated roots,
private instance ownership and actual argv. Historical absent declarations mean
v1; validators never choose a layout by testing file existence. Existing v1 files
and receipts are not rewritten, relocated or deleted.

The owner's20GB budget still covers only out, with the18GB live-writer guard.
V2 builders and interactive launch write only bounded latest-image/session metadata there: each latest pointer,
including staging, is limited to32MiB. Its128MiB admission reservation also leaves
the existing2GB writer margin. Legacy commands keep the8GB reserve. Report bulk
is outside the user's cap; this change is not a total-disk quota or capacity claim.

The only new cleanup candidates are Cargo intermediate leaves under the exact
canonical run's generated/builds tree: deps, build, .fingerprint, incremental and
examples below release/debug. A sealed completion or the completed current run,
matching v2 declaration, no retained JSON reference, no links/reparse points and
actual QEMU/rustc quiescence are required. Compiler-cache metadata is skipped in
reference scans to avoid self-pinning. External references remain binding.
Final artifacts, link maps, images, disks, logs, failed execution proof and owner
storage stay protected. Reclaimed report-cache bytes are reported separately;
they do not count as a reduction in out bytes.

Qualification requires current host tests, all nine desktop workflows, independent
artifact comparison with the accepted Storage checkpoint, v1 retained validation
and v2 retained validation after actual cache pruning. Foundation was not rerun by
this migration and can be inherited only for byte-identical guest artifacts.
Future foundation/ordinary/interactive command migration remains separate work.

## Accepted evidence

Review: `reports/critics/p12-output-layout-v2-critic.json`, SHA256
`f76b17c65fbb9656f2e66faf0ba5e0d7a4c49143ff238b1efdd92e9d6c2d68ed`.
Tested source: `aa1e6789d41cedb70e6cc7bf5c38d3f49c7ac6fb287c3ce0bf5c8a9336042b6e`.
Reports below are under reports/runs.

| Run | Evidence |
| --- | --- |
| host-1792565842257331500-6732 |657 host tests /33 executables |
| host-1792565964145546900-15756 |253 desktop +2 maintenance checks /9 actual sessions |
| host-1792566987176410400-1500 |Complete v2 retained validation after actual pruning |
| host-1792567132971595700-10352 |Complete historical v1 retained validation with final validator |

Independent audit rehashed825 references,33 image entries,178 exact PNG/PPM RGB
pairs/176 probes, all9 owner-instance/PID/UUID/argv bindings and8 independently
decoded raw disks. Every expected document and catalog/journal CRC validates.
All21 actual rebuilt guest artifacts and5 independent app pairs match the accepted
Storage checkpoint byte for byte. This is limited artifact inheritance, not new
foundation execution. Seven disks match that earlier run byte for byte; recovery
differs in42 namespace/CRC bytes because notes/Desktop directory IDs are swapped,
while the logical path and exact38-byte recovery document remain correct. Startup
mkdir ordering is a code-consistent explanation, not a serial-attested fact.

Actual cleanup removed40 retained Cargo leaves/205,996,028 bytes. Out before/after
cleanup remained16,758,530,855 bytes: report-cache cleanup is not an out reduction.
All21 artifacts,18 maps,3 import libraries and5 app pairs remain hash valid.
The earlier long-path run9300 remains a failed pre-guest development run with
original report/logs. Its duplicated owner exceeded MSVC tooling path limits;
the shortened mapper subsequently compiled successfully. No failure is promoted.

The owner subsequently offered external D: (USB NTFS, label Backup). It was
identified read-only; no paths were relocated or budget checks disabled. External
retention needs explicit location/provenance support before removing local proof.

## Subsequent standalone consumer extension

The newer bounded alignment checkpoint also enrolls exactly test desktop-notes,
desktop-files and desktop-remove plus their accelerated variants with --image.
All six actual command graphs passed native two-boot workflows and independent
review. They write private disks/captures and the shared QEMU tree with their run;
no new build or unbounded out producer is introduced. See
../design/control-row-alignment.md and reports/critics/p12-control-alignment-critic.json.
The original desktop-current review above remains unchanged. Interactive desktop
launch and owner-volume initialization are still separate work.

## Subsequent first-run interactive extension

The separately reviewed first-run batch also enrolls validated interactive desktop
options and exact desktop-default/software --image consumers in retained v2.
No build graph is added. Interactive launches use an existing protected mutable
owner disk and write a bounded latest-session pointer; all QEMU staging is run-local.
The exact init-user-volume command is a separate fixed-size writer with320MiB
reservation for staged/public logical names and the unchanged2GB writer margin.
Other legacy build admission remains8GB. See desktop-first-run.md and
reports/critics/p12-first-run-critic.json. Original migration receipts are unchanged.
