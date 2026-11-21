# Verification and evidence

The P10 gate includes the [ordinary-session suite](ordinary-session.md),
including actual user commands, independent raw disk audits, fresh-boot reopen
and a versioned third-party source-port trial. Historical diagnostic totals alone
cannot qualify this repair scope.
The [application trial inventory](application-trials.md) records exact native/PE
hashes, observed real workflows and the source-port compatibility boundary.

`cargo xtask test host` runs workspace tests. `cargo xtask verify` is the selected
P00–P10 gate. The boot protocol tests exercise valid encoded bytes, every truncated prefix,
incorrect versions/endianness, reserved fields, length/alignment/overflow limits,
overlapping tables/regions, unknown required/optional data, excessive counts,
runtime/free ownership conflicts, live boot-data reservation and unaligned input.
Systematic byte mutations check that malformed input does not panic.

P01 adds ELF/PE malformed/boundary fixtures, artifact personality checks, deterministic
image staging and tamper/collision/traversal cases. Profile and result tests distinguish
ordered kernel evidence from unrelated pass text. Real child tests exercise stdout
and watched-serial output limits. Original fixtures in `tests/support/images.rs`
are structural data; classifier tests are host simulations. The gate also performs
two inspected/smoke-tested host release builds and compares their complete hashes.

Host tooling tests distinguish missing tools, nonzero exits and timeouts, verify
stderr hashes, source snapshot changes and evidence path confinement. Windows
timeout cleanup kills the process tree and reaps the direct child. Cleanup failure
is recorded and cannot be reported as success. Non-Windows process-tree cleanup is
not qualified. The emulator harness was host-tested in P01 and has actual acquired
QEMU/firmware execution evidence from P02 onward.

Each run has a unique local directory, captured raw streams, argument arrays,
resolved executable where discoverable, actual/expected exit codes, timeout,
duration and SHA-256 hashes. Reports include a sorted source manifest that excludes
`.git`, `target`, `out` and generated local reports. An uncommitted content snapshot
is not a Git commit, independent backup or cryptographic proof of OS execution.
The working-tree manifest is maintained at `reports/source-snapshot.json`. Inputs
are captured before any checks and compared again afterward. A change during the
run produces failed evidence and a nonzero exit, even if individual commands passed.

`check-spec` verifies the supplied specification hash and section inventory,
required artifacts, all 30 phase entries and the file bytes referenced by a passed
phase's logs. It requires a full gate run and enforces one declared active phase,
completed predecessors and unstarted later phases. Critic acceptance must identify
the qualifying report and the exact reviewed build-input manifest. These are local
consistency checks; they cannot authenticate a review or infer OS behavior from a
pass string. Current evidence must be regenerated after code or build-input changes.
Passed historical phases retain their original source/review scope. P01 binds
current hardware profiles as well as source/build/contract inputs and checks the
continued bytes/smoke-test evidence of both rebuilt executables.
If the selected passed phase is changed, explicitly set it back to in_progress before fresh
verification; stale evidence cannot silently keep it qualified.

There were no guest tests in P00/P01. P02 introduces actual independent boot;
Windows-reference, desktop-fidelity and physical-hardware tests remain `not_run`.
Phase
completion requires independent critic review and resolution of actionable bugs.

P01's historical host-foundation closure passed 50 tests: 16 boot-contract,
10 image-format and 24 host tooling.
The final run also passed formatting, strict Clippy, both libraries on both guest
targets and two distinct release builds with PE inspection, smoke execution and
matching full executable hashes. The critic independently checked all 44 raw log
hashes, the 35 current build inputs and all four corrected repros. See
`reports/artifacts/p01-critic-host-foundation.json` for those original inputs.
The acquisition amendment passes 51 tests, 26 commands and 52 raw streams with
the real emulator preflight. Removal of its check, changed profile, absent/failed
preflight commands and changed acquired bytes are rejected by persistent validation.
See `reports/critics/p01-critic.json` and `project/STATUS.md` for the current reviewed gate.

P02 adds module/mapping/framebuffer roundtrips and malformed ownership/alias/stride
cases; strict ustar truncation/checksum/type/path/padding/limit cases; and real
firmware/kernel target lint/build/inspection. Its two controlled guest rebuilds
compare whole firmware/kernel/archive bytes and retain symbol/map artifacts.

Actual scenarios: normal completion (exit 33), controlled panic (exit 35), bad
handoff rejection (35), kernel-owned intentional hang (timeout), private malformed
kernel/archive fixture rejection (35). Expected failure does not turn the guest
run itself into a pass; the suite records expected/observed separately and rechecks
the raw process/serial evidence. Every child binds the qualifying source and image.

Headless demo tests send H/D/R/P through local QMP, observe real kernel input markers,
require successful reboot readiness again, then capture emergency panic output.
Normal/diagnostic/panic PNGs and completed serial evidence are retained by hash.
Regressions reject directory boot configuration, oversized directory wire names
without header writes, missing/altered private media and changed launch identities.
The interactive process controller is exercised for nonzero exit and output-limit
termination/reaping; completed demos retain their actual process and raw stream
records. Persistent validation derives argv and reopens private artifact bytes,
checks private variables, and requires three distinct captures.
The visible owner demo is a separate live session; mutable logs do not replace
completed gate evidence. No renderer screenshot is created on the host.

P02 closure passes 61 host tests (19 boot, 10 image, 4 initfs, 28 xtask), all target
checks and the actual guest suite. Independent follow-up accepts the corrected
four findings and binds 66 reviewed inputs/122 raw stream hashes. See
`reports/critics/p02-critic.json` and `project/STATUS.md` for immutable acceptance pointers.

P03 closed with the reviewed full gate
`reports/runs/host-1791819087216060600-33544/report.json` and independent critic
`reports/critics/p03-critic.json`. Its historical host suite has 88 tests: 27 boot protocol,
10 image format, 4 initfs, 18 kernel library and 29 xtask. New fixtures cover ACPI
checksums/truncations/counts/duplicate identities, copied platform reservations,
physical-frame alignment/exhaustion/fragmentation/stale or foreign frees, heap
zeroing/exhaustion, table-construction rollback, permission/alias/token validation,
CPU layouts and PM-timer rollover. Host page-table models cannot establish actual
instruction faults or interrupt delivery.

Exploratory guest suite `reports/runs/host-1791817879599149500-32036/report.json`
passed all twelve processes and H/D/R/P with captured screens. The added cases
are `trap-ud`, `trap-double`, `fault-readonly`, `fault-nx`, `fault-guard` and a normal
256 MiB boot. Fault cases require the expected kernel diagnostic vector/error
evidence and exit 35; their guest runs stay failed/attempted. Normal cases require
real frame allocation and reuse/zeroing, mapping protection and retirement, heap
checks, an INT3 register/FXSAVE roundtrip and at least three calibrated APIC IRQs.
The 2 GiB profile, six P02 failures/outcomes and all four console controls remain
part of the selected P03 suite. Persistent validation binds the selected scenario
set, each exact profile/image invocation and raw diagnostic bytes.

P03 is bounded to one CPU and ring 0. Mapping operations require disabled IRQs and
exclusive ownership; successful unmaps invalidate local translations before frame
ownership returns, while structural page tables remain retained. These tests do
not qualify cross-CPU TLB retirement, scheduling, user access, firmware reclamation,
external device routing or physical hardware. P04 onward adds its own gates.
The initial critic reproduced a historical P02 report accepted by a P03 request.
The phase validator and guest validator now both bind to the caller's requested
phase, and P03 critic acceptance also checks its phase and exact report digest.
Regression tests cover current and historical checks; the fresh 88-test full gate
`reports/runs/host-1791819087216060600-33544/report.json` passed. Critic acceptance
`reports/critics/p03-critic.json` binds 83 inputs/182 raw streams with zero unresolved findings.
Independent allocator/ACPI probe evidence is retained under
`reports/runs/critic-p03/`.

P04 implementation currently passes 115 host tests: 33 boot protocol, 10 image
format, 4 initfs, 35 kernel library and 33 xtask. The full current-source qualifying
gate and independent critic acceptance passed. Host additions cover
reserved startup-page records, task/object generations, unique queue ownership,
retirement tickets, priority/FIFO/aging, affinity, all 24 signal/timeout/cancel/close
orders, semaphore accounting, mutex donation/abandonment, condition reacquisition,
suspension and repeated four-CPU metadata transitions. These are deterministic
state tests; they do not start host threads or establish guest SMP by simulation.

Exploratory actual guest evidence:
`reports/runs/host-1791820833100390500-14220/report.json`.
All fifteen cases and completed H/D/R/P demo pass. P04 retains the twelve P03
cases and adds `normal-smp2`, `normal-smp4` and `smp-degraded`. The last intentionally
withholds one secondary startup and must report discovered=2, online=1, degraded=1.
The current source inventory owns exact per-case markers and profile identities;
missing, duplicate or conflicting P04 online/retirement records fail validation.

Normal guest runs create twelve real kernel workers. Four workers hold distinct
GPR/FS-TLS/x87/SSE patterns across timer preemption and visit every online CPU.
Other workers exercise event/semaphore publication, cancellation, object closure,
timeout, condition release/reacquisition and priority donation with owner exit and
an abandoned mutex result. Each task's terminal result is checked and its stale
identity rejected after reaping. The scheduler lock crosses the architecture stack
switch; the commit hook acknowledges retirement before another CPU may dispatch
or reap the outgoing context.

Mapping stress alternates two distinct physical frames through one virtual address
for 32 epochs. Every online CPU observes each published value. Protection changes
and unmaps require synchronous remote translation acknowledgements before backing
can be reused. Worker stacks are subsequently retired; AP idle/emergency stacks
remain live and retained. Parked APs can still acknowledge IPIs until QEMU exits.

This qualifies a bounded ring-0 foundation only when its current full gate and
critic pass. Protected native execution, arbitrary process/user-pointer safety,
Windows waits/APCs, CPU hotplug, general async-I/O lifetime handling and physical
hardware remain later gates. Requested vCPU count, portable tests or serial text
alone cannot establish those capabilities.

P05 preserves all fifteen cases and H/D/R/P interaction. Positive boots additionally
load six actual protected processes from the inspected native ELF artifacts. The
client/server authenticate sender metadata, transfer reduced section rights and
observe shared backing updates. They exercise stale handles, denied escalation,
MOVE ownership, queue saturation, partial output and bounded handle exhaustion.
The malicious process reads supervisor kernel text; only that process exits -14.
A second invalid-return process exits -257. The survivor waits and continues; a
separately assembled caller checks literal selectors, all six argument registers,
preserved registers, FS TLS, x87 and selected SSE state before exiting 55.

Startup diagnostics force failure at every allocation/mapping construction stage,
check complete frame-stat restoration and scan the entire process alias arena for
leftover mappings. The same arena must then build and retire successfully. Bounded
copy recovery reads across a known stack guard and checks the exact copied prefix
and restored kernel CR3. Teardown checks zero executive objects/processes/sections
and the initial physical-allocation count after every scheduler context retires.
Queued IPC cancellation and deadlines are checked at each terminal ordering, with
committed MOVE ownership distinguished from pre-enqueue failures.

The P05 validator requires exactly attributed user markers, unique trusted
role/PID/exit records, subsystem summaries, exact per-profile boot evidence and
five reproducible artifacts per build. ABI/sys/rt undergo strict target checks;
native binaries receive independent ELF inspection and link maps. A weaker earlier
phase report, user-forged unprefixed output, missing native artifact or duplicate
process result cannot qualify P05. Only the full unchanged-input report and its
bound independent critic close the selected phase.


P05 final gate: reports/runs/host-1791824568839707600-11968/report.json; 157 host tests, fifteen actual guest cases and interactive controls passed with unchanged inputs. Critic acceptance: reports/critics/p05-critic.json (zero unresolved findings). P05-C01 counter-exhaustion regression covers both endpoint-owner cases before publication. Closure check-spec host-1791824978309709100-25580 passed 154 required paths. P06 is selected next.


## P06 storage integration

The current suite adds storage-write and storage-reopen to all 15 P05 cases. Both
run actual prior scheduler/native diagnostics, negotiate modern VirtIO VERSION_1
and FLUSH, validate GPT/FAT32/ext2/initfs, and retain stable VFS identity through
rename and a cross-block overwrite. The 6000-byte fixture is flushed and recovered
in a separate QEMU boot. An independent host parser checks payloadCRCaaa29e86,
all reachable allocations, immutable partition regions and exact raw snapshots.

Guest failures include bounds/alignment, queue saturation, cancellation/reset,
stale request identity and production cache propagation of injected partial I/O.
The metadata-prefix short write reaches the real disk but stops before ext2's
dirty-state field; its mount is poisoned. Host tests additionally interrupt
writes after the dirty marker and require unclean remount rejection. A failed
flush poisons mutation as well. Successful shutdown proves device reset and
bus-master disable before all eight DMA pages return to the frame allocator.

Storage block callbacks sample the existing guarded 128 KiBbootstack and require
at least 16 KiB remaining; this is sampled headroom, not an absolute high-water claim.
P06 closed with 190 host tests and all seventeen guest cases in
`reports/runs/host-1791827135134883500-12352/report.json`, followed by its independent
critic and closure check. Earlier exploratory failures remain retained.

## P07 journal and namespace qualification

The P07 gate has 35 actual QEMU processes plus the console interactions: all 17
prior cases, save/reopen, five interrupted journal boundaries each followed by
fresh recovery and a second idempotent reopen, and a separate small-filesystem
capacity case. The gate binds source/profile/contract identities, exact launch
arguments, private raw disks, before/after snapshots and process results. An
intentional interruption is a distinct outcome requiring exit 37, the run's
ordered firmware-exit/interruption token records and the exact stage/generation.
It never qualifies as normal kernel completion or as a generic process failure.

The host's independent journal decoder checks both header CRCs, identity/extent
bindings, state/generation, unique bounded targets and every committed payload
CRC before reconstructing a recovered home partition. Physical crash bytes prove
the selected stage: no home mutation before commit, exactly the first ordered
home-page application at first-home, and the full home after home flush. All five
branches must contain the same prepared transaction payload. Recovery must match
the independently reconstructed full home, normalize checkpoint copies, and leave
unrelated partitions untouched. A second reopen must leave every raw byte unchanged.

The independent ext2/catalog audit checks directory reachability, all allocation
counts, inode/block crosslinks, unowned backing files, file identity and ACL
association, Unicode fixture names, exact old/new payloads, typed native settings
and HKCU registry values, and the unrelated file. Capacity uses an actual 32-block
filesystem with nine committed 8192-byte files and zero free blocks. The guest
also checks sharing, denied access, rename/open coherence, replacement, pending
deletion, truncate and stale identities. Host parser tests mutate checksummed
records to test semantics beyond checksum failure, including target duplication,
wrong volume, catalog/data mismatches and weaker or unbound interruption markers.

P07 closed with `reports/runs/host-1791830810540264100-10572/report.json`
(SHA256 `22e9620d1b2c11971793921500035b91f88568ca9f046277a5d8b04355fbd00f`):
228 host tests, all 35 actual guest cases, diagnostic interactions and unchanged
inputs. `reports/critics/p07-critic.json` binds that report with zero unresolved findings.
The final suite includes the critic's deleted-registry-ancestor, contradictory
journal-mirror and mixed-slash UNC regressions. Sampled stack headroom was 67616
bytes for save and 41824 for capacity. Historical evidence is retained unchanged.
Malformed evidence structure fails before hashing large retained disks; valid
evidence still undergoes complete byte and allocation checks.

The exact host command `cargo test --workspace --locked --all-targets` has a
300-second process budget. P06 workspace testing approached the ordinary
120-second bound while independent tests hash and audit real 128 MiB disk
fixtures. Other required host commands retain 120 seconds; guest boot deadlines
remain unchanged. Timeout, process-tree cleanup and output limits remain enforced.

The independent P07 journal auditor rejects valid-CRC but contradictory mirror
records. Equal-generation equal-state copies must match byte for byte; a checkpoint
dominates a committed peer at the same generation. Different generations must be
adjacent, with an older checkpoint and a newer committed record. The host test
matrix covers both mirror orders, publication and completion boundaries, conflicting
manifests, generation gaps, invalid state pairs and the nonwrapping u64 limit.

## P08 platform qualification

The second cumulative gate at
`reports/runs/host-1792059442687006000-11976/report.json` (SHA256
`7ed484dd2e3717856b2da7282a6e6205ed777fdcd43ededb5e39749241358cc3`)
passed 278 host tests, strict checks, two matching ten-artifact builds, all 39 guest
cases and both interactive checks with unchanged inputs. It qualifies the C01
foreign-mutex stop and C02 console-record fixes. Later C03 Thread-lease changes
pass 89 kernel host tests and the independent interleaving probe. C04 final launch
authority validation and failed-construction retirement also pass the independent
probe; the coordinated runtime passes 91 kernel host tests and functional critic
review. Targeted suite `host-1792060554365351700-4084` then passed all four service
boots, C03 early-handle-close revocation, independent disk audits and six console
commands using image `host-1792060510454242600-18856`. These intermediate reports
remain historical evidence.

The final gate `reports/runs/host-1792060739947826700-17608/report.json` (SHA256
`3e8f1d80fa25b33021bc1d55a926199b6277abe20b7003798870b31170b8231a`)
passed 281 host tests, all 39 guest cases, both interactive checks, two matching
ten-artifact builds and unchanged inputs. `reports/critics/p08-critic.json` (SHA256
`86307c383b96eef8953e257e33e9939f977547087528c5c117f81cf105eadba2`)
accepts all four C01–C04 fixes with zero unresolved findings. Its audit binds
217 build inputs, 42 reports, 260 commands, 520 streams and 995 hashes; 418 serial,
15 gate and 24 disk mutations were rejected. P08 closure checks passed; P09 is active.

The P08 gate extends the retained P07 suite; it cannot qualify from host model tests
or a service's self-reported name. The native build inventory requires the original
runtime/raw-ABI fixtures, four separate service executables and an independently
linked SDK workload. Each artifact is bound to its successful package command,
target flags, inspected ELF headers, link map, source snapshot and exact initfs
bytes in both reproducibility passes. `Phase` accepts only implemented canonical
phase names; historical certifications retain their original command and source
path requirements. P08 binds moved storage/persistence code in `door-storage-core`.

The independent P08 disk auditor expects eighteen directories and two owned
`/users/<uid>/workload.bin` files, with exactly 4096 bytes each. Bytes are
`(i*29+17)%251` for UID1001 and `(i*29+53)%251` for UID1002. The owner-only
inherited ACL, untouched protected-directory ACLs, native theme/scale 1/125 and
2/150, four HKCU64 keys with each user's `Door/Epoch` DWORD equal to their UID,
the complete allocation graph and checkpoint generation3
are checked. Reopen and service restart must preserve every disk byte. The corrupt
branch changes exactly bit0 of the catalog CRC, then requires actual service
refusal and unchanged corrupt disk bytes. Semantic mutations with recomputed
checksums, wrong ownership/settings and half-file corruption are host regressions.

`services-save`, `services-reopen`, `services-restart` and `services-failure`
use the distinct pinned services profile. Their serial records bind each user
marker to the kernel's retired PID/manifest/instance/UID/session/exit inventory.
Restart also launches a third workload in UID1002's existing session, with a live
allocation, storage handle and two waiting threads. Init authenticates its readiness,
acquires the transferred mutex while the worker is in a condition wait and retains
ownership until process retirement. This covers forced stop with a live foreign
mutex owner, separately from ordinary condition cancellation/reacquisition. Init
revokes that session and requires exit -13 plus denied reuse of the session grant.
The app closes the worker's Thread handle before that revocation, exercising the
runtime's completion-event lease through final signalling and wait-object retirement.
The host binds the revocation marker to that same session/epoch and a newer process
instance, requiring eleven retired processes and fourteen threads without leaks.
The SAVE app also submits CR/LF-prefixed forged kernel/user records and a partial
console write through ordinary RPC. Escaped, independently terminated console
serial records preserve the trusted parser boundary; the framebuffer text is separate.
The console uses QMP keyboard input for help/status/file/theme/threads/exit, six
hashed guest screenshots and exact resource retirement. Its private disk clone
must change only UID1001's theme 1→2, the setting/catalog revisions, and the journal
checkpoint 3→4. All other persisted bytes remain unchanged. The second gate exercised
these paths before the latest early-Thread-handle-close and launch/revoke changes;
the subsequent targeted suite and final cumulative gate cover the updated guest
case. The independent critic accepted the final source and exact report.

For long retained-image audits, build the existing orchestrator with
`cargo build --locked --release --package door-xtask`, then run
`.\target\release\door-xtask.exe verify`. This changes the host tool's optimization
level, retaining every required gate command, guest process and full byte audit.
The standard `cargo xtask verify` entry point remains supported.

### Gate wall-clock time

From P11 on, `verify` runs the P11 desktop gate steps, the desktop scenarios and
the acceptance suites on a bounded guest worker pool (`tools/xtask/src/parallel.rs`).
`DOOR_GUEST_JOBS=N` sets the worker count; `DOOR_GUEST_JOBS=1` runs everything
in declared order, stopping at the first failure, exactly as the sequential gate
did. The default is the scarcest of `(logical processors - 2) / 3`,
`(free MiB - 4096) / (2048 guest + 1536 overhead)` and
`(free output-volume bytes - 25 GB) / 3 GB`, clamped to 1..=8; the chosen value
and its inputs are recorded as the report's `scheduling`.

Each worker has private guest state: scenarios keep their own child runs, while
gate steps and suites record into forked ledgers of the gate run (same identity
and directory, shared `command-NN` numbering) that are appended to the report in
declared order, so check identities and records are unchanged. QEMU session
directories (ESP copy, firmware variables, serial log) are named by a
process-unique stamp and each guest gets its own QMP port; ESP retirement never
touches a session of the running process. Timing-sensitive work runs in an
exclusive slot with no other guest: the `desktop-recovery` (crashed window
server racing its restart; it failed under four concurrent guests and passes
alone), `desktop-wide` (cursor latency), `desktop-resolutions` (recorded drag
timings) and `desktop-native-default` (automatic host backend) gate steps, the
`desktop-shell`, `desktop-accelerated`, `desktop-default`, `desktop-software` and
`desktop-motion` scenarios, and the `motion-01`, `recover-01` and `vis-01` suites. The Windows GUI fixture image is staged once
before the pool starts. After the first failure no new work starts; running
guests finish and every failure is reported with its report path.

The build and its controlled rebuild (diagnostic guest image passes, text and
desktop userland passes) run concurrently in their separate target directories.
Userland applications pass their link map and linker script only to the final
executable (`cargo rustc ... -- -C link-arg=...`), so a pass compiles the shared
dependency graph once instead of once per executable. Source snapshots hash on
a quarter of the logical processors, one snapshot at a time.

The report's `timings` records every stage, image build, guest case, gate step,
scenario and suite; `verify` prints the longest steps and host commands at the
end, whether it passed or failed.
`cargo xtask test guest-batch --phase P14 --image <image.json> --only a,b,...`
re-runs named gate steps, scenarios and suites of a staged build on the same
pool; it certifies nothing.

`cargo xtask test services --image <relative-image.json>` runs the same P08 service
and console evidence checks on fresh private disk branches for integration work.
Its report is explicitly exploratory and cannot replace the cumulative gate,
unchanged source identity or independent critic acceptance.

## P09 cumulative Windows qualification

P09 retains all 39 previous guest cases and adds `windows-basic` on a distinct
pinned hardware profile. It runs the exact unchanged independently built EXE used
on the installed Windows reference. Expected output includes relocation, invalid
handle, invalid buffer, LastError, GPR/SIMD ABI, zero write and selector checks;
the reference and guest must both exit73. Original DLL bytes, mapped addresses,
entry points, archive membership, import libraries and successful build commands
are rebound to retained artifacts during acceptance, rather than trusting a
summary. The host report records a completed same-binary reference comparison.

Seven actual malformed-load rejections must leave baseline allocation state
unchanged. Every real Windows VM construction checkpoint is failed and rolled
back; the gate derives the required checkpoint count from the built mapping plan.
The bootstrap receipt additionally proves full-range checked copy, failed-write
atomicity, copy limits, forbidden thread/heap/raw-console authority, and separate
GS/FS state. Kernel-owned process identities attribute the bootstrap and ordinary
EXE results to manifest6, UID1001 and its session. Five processes/five threads must
retire with zero process-frame leaks and all eight DMA pages reclaimed. The
private saved native disk must remain byte-identical and pass independent audits.

Host regressions remove and duplicate every required P09 record and mutate
identity, status, resource, permission, pointer-copy and output values. C, MASM
and header inputs participate in phase/critic source identity. The historical P09
gate and independent critic passed as recorded below; dynamic DLL loading,
Windows TLS and unwind belong to the separate P10 qualification.

P09 closed with final gate `reports/runs/host-1792150739645891100-11748/report.json`
(SHA256 `48fa8a34f832a2b5de8762d7de4deeb307de72b2e1c1a85ec40960c943bac1ad`).
310 host tests, all40 guest scenarios, both interactive checks and two matching
13-artifact builds passed with unchanged inputs. The exact independent Windows
EXE runs on Windows and Door with verified output/exit73, real relocation,
invalid handles/buffers and ABI preservation. Seven malformed-load cases and79
construction rollback points pass; process/DMA resources retire and the P08 disk
remains byte-identical. `reports/critics/p09-critic.json` accepts248 build inputs,1202
hash bindings and eight rejected weakened gates with zero unresolved findings.
The supported scope remains one no-CRT Windows thread, static original DLLs and
bounded synchronous ASCII output. P10 owns DLL/TLS/unwind and broader base APIs.

## P10 cumulative qualification

P10 implements the bounded Windows base profile described in
specs/abi/windows-base-p10.md. Its full gate extends inherited coverage to42 guest
cases, both interactive checks,16 reproducible artifacts, original P09 reference
comparison, P10 Windows reference and actual save/fresh-boot reopen byte audits.
The ordinary C/MASM fixture includes failed DllMain initialization, duplicate
references, unload/reload, existing-thread compiler TLS, callback ordering,
concurrent references/errno, memory allocation failures, actual fault continuation
and caller recovery, event/APC ordering and pending I/O cancellation/close/query
races. Native kernel probes inject every dynamic DLL acquisition failure before
publication and verify absent mappings/TLS plus baseline frame ownership. No
finite suite guarantees absence of every defect. Final cumulative report
reports/runs/host-1792214999674635300-12500/report.json and independent
reports/critics/p10-critic.json passed: 341 host tests, 42 guest cases, both interactive
checks, two matching 16-artifact builds and zero unresolved review findings.


Critic regressions require relocated TLS pointer initializers on Windows and Door,
reject startup import/forwarder dependencies on deferred modules, and reject
contradictory mandatory host/child command receipts. Immutable historical P00
records alone may omit the later output-limit field; current receipts require it.
Final qualification includes all critic corrections. Its report is
reports/runs/host-1792214999674635300-12500/report.json (SHA256
e8d8ce9cc07e83c3d5bde11385d5f8be3bc970ef35e52741cfe42b127d1eaa3d).
The independent audit verified 285 build inputs, 1,404 hash bindings and 630
streams; all 18 weakened gates and 140 serial mutations were rejected.

P10 save and fresh-boot reopen each retire 38 threads with zero process-frame
leaks, exercise 103 startup and 35 dynamic-module rollback checkpoints, and
reclaim eight DMA pages. Reopen has zero writes/flushes. Async guest comparison
covers reads, cancellation, close and concurrent result observers; async-write
source was reviewed without a separate unchanged-binary write comparison.
