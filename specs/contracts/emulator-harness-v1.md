# Emulator harness v1

Owner: `tools/xtask`. Sources: QEMU-INVOKE and QEMU-IMAGES. P01 implements and
host-tests the harness and P02 executes actual guest artifacts. QEMU/OVMF inputs are acquired and pinned in
`specs/sources/emulator-acquisition.json`; `check-emulator` validates actual hashes,
version and supported machine/CPU/accelerator. Guest behavior remains a P02 gate.
Unresolved/null profiles are still rejected for execution.

A runnable diagnostic profile requires a versioned `pc-q35-*` machine, named CPU,
exact QEMU version/executable/hash, OVMF code and variable-template paths/hashes,
TCG and a supported vCPU count. The unchanged one-CPU baseline has 2048 MiB;
P03 adds the separately pinned one-CPU 256 MiB minimum profile. P04 adds distinct
`qemu-smp2-x64` and `qemu-smp4-x64` profiles with two/four CPUs and 2048 MiB.
Network and physical-disk passthrough must be disabled.
Files are checked against hashes; QEMU's version and machine/CPU/accelerator help
output must agree with the pins. The profile cannot inject arbitrary QEMU options.
Code/template acquisitions remain read-only; variable state is copied per run.

The runner creates an exclusive `out/qemu/<run-id>` with private firmware variables,
a rehashed copy of the three image entries and serial capture. Arguments select
the versioned machine/CPU, TCG, explicit VGA, hidden display, no monitor/network,
readonly pflash code, private mutable pflash variables and readonly virtual FAT
attached through VirtIO. Actual QEMU 11.1.0 rejects readonly IDE hard disks; VirtIO
accepts this medium. `-no-shutdown` is omitted so debug-exit completes on this build.
QEMU option commas in file paths are escaped. No shell interpolates profile values.
The executable, firmware and image identities are checked again after execution.

Subprocesses have a 30-second guest timeout and 120-second build timeout. A watchdog
observes stdout/stderr (8 MiB each) and guest serial (1 MiB) while the child runs.
Crossing a limit terminates/reaps the process tree on the tested Windows host and
caps retained streams. File polling can temporarily overshoot the threshold; it is
a local diagnostic bound, not a strict adversarial disk quota. Cleanup failures,
timeouts and limit failures remain non-pass outcomes. Normal host execution uses
argument arrays and scoped environment overrides. Non-Windows cleanup is unqualified.

The initial boot test protocol is deliberately scoped to the sole diagnostic kernel,
before applications exist. A per-run token is delivered through `opt/door/test-token`
in fw_cfg. This is correlation/isolation, not cryptographic authentication. Serial
records must occur in this order:

```text
DOOR_TEST v1 <token> POST_EXIT_BOOT_SERVICES
DOOR_TEST v1 <token> KERNEL_DIAGNOSTIC PASS
```

The debug-exit device must then return host exit 33 (guest write value 16). Matching
`KERNEL_PANIC` or `FAIL`, duplicate/out-of-order records, timeout or abnormal exit
cannot pass. A plain `PASS` or another token cannot satisfy the boot gate. P02 must
implement the actual kernel-owned operations behind the records; P01 classifier
tests are host simulations and supply no guest execution evidence.

Outcomes distinguish missing/spawn tool errors, startup failure, output limit,
timeout, cleanup failure, kernel panic, guest failure, abnormal exit, incomplete
evidence and expected completion. Reports retain exact argv, profile, image hashes,
private paths, serial digest and observed outcome. The enclosing host report binds
the guest-result file by hash. Both records use attempted after an unsuccessful
execution and guest_tested after verified completion. The QEMU child command
records expected exit 33 explicitly.
P02 runs normal, panic, bad-handoff, hang, bad-kernel and bad-initfs as separate
private processes. Negative guest records remain failures/attempted even when the
enclosing suite matches the expected failure. Actual process flags, serial bytes,
image/build identity and child report hashes are retained and persistently validated.
Validation reopens the private staged image, checks its actual three artifact bytes
against the rebuilt image, checks retained private variable-state hashes, and
derives exact argv from the pinned profile, run token and confined private paths.
Merely hashing an arbitrary declared argv or medium cannot establish acceptance.
Hang requires the kernel-owned intentional-hang marker plus actual timeout/cleanup.

P03 preserves those six cases and adds UD, deliberate double fault, read-only,
NX and guard-page faults on the baseline profile, plus normal completion on the
minimum-memory profile. The suite binds twelve distinct child executions and the
existing demo. Fault outcomes remain guest failures, requiring exact selected
vector/error/address markers; generic failure or panic cannot replace a protection
trap. Both normal boots require CPU/memory/timer markers before ordered kernel PASS.
Scenario definitions are shared by execution and persistent validation; profile
identity is checked per child. Historical P02 retains its original six-case
selection and launch semantics.
The ledger-selected phase must match the report certification before any guest
inventory is selected. Historical P02 evidence cannot qualify P03 even when the
historical source comparison is inactive. The critic's phase must also agree.
P03 closure additionally binds critic acceptance to the exact full report SHA256,
as well as its path and complete selected build-input manifest. A changed report
requires another review; matching source alone cannot carry an old acceptance.

P04 retains all twelve P03 cases and adds `normal-smp2`, `normal-smp4` and
`smp-degraded`, for fifteen distinct child executions. The last case uses the
two-CPU profile with the explicit `smp-degraded` mode and requires one online CPU;
it is a successful degraded diagnostic run with debug exit 33. Every normal and
degraded run also requires the P03 foundations and exactly one complete summary
line for each P04 subsystem: `DOOR_P04 TASKS PASS`, `DOOR_P04 WAITS PASS`,
`DOOR_P04 SMP PASS discovered=N online=M degraded=D` and
`DOOR_P04 TLB PASS cpus=M rounds=R`. The discovered/online/degraded fields must
match the selected profile and mode, and R must be a positive integer. Normal
runs require all discovered CPUs online and D=0; the degraded run requires
N=2, M=1 and D=1. TLB evidence covers actual local retirement for one CPU and
remote acknowledgements when more CPUs are online. Missing, duplicate or
conflicting subsystem records cannot qualify a run. Deliberate P03 fault cases
keep their original markers because they stop before P04 runtime execution.
The P04 gate and critic bind the exact P04 report phase, report digest, source
inputs and each child's profile/arguments. P05 and later gates remain unsupported.
These are acceptance requirements; closure still requires actual guest evidence
and independent critic review.

Demo uses the same inspected artifacts, private copies and fw_cfg mode. Local QMP
is a bounded loopback control channel, distinct from disabled guest networking;
UUID checks bind stop to the created VM. Tests drive H/D/R/P, observe kernel markers,
require reboot readiness twice and capture different normal/diagnostic/panic screens.
Completed demos use a checked interactive process controller with a 90-second total
deadline, the same stream/serial caps, recorded PID/argv/exit/cleanup state and raw
stream hashes. QMP requests are bounded and loops poll the process/limits. The gate
requires exact derived invocation, clean exit zero, all four controls and three
different capture hashes; missing profile/process data or duplicate screens fail.
The visible SDL launch remains running for the owner after readiness. It has no
watchdog once handed to the owner; logs remain local and manually stopped by window
close or `stop-demo`. This is a diagnostic kernel, not a desktop/userland scenario.

P05 preserves the complete fifteen-case inventory and all hardware profile hashes.
Positive cases and demo additionally run six protected native processes and require
exact trusted role/PID/exit records plus PID-attributed fixture output. Each user
output line gets a kernel-created attribution prefix, including embedded newlines.
Checks cover independent ABI, authenticated IPC, section rights, cancellation and
deadlines, startup rollback, two contained faults and complete frame reclamation.
Two independent native ELF files, their link maps, target/build flags and exact
packaged initfs bytes join firmware/kernel/archive reproducibility evidence.
Selected phase and critic/report identity remain explicit. The visible console
describes protected native diagnostics; general desktop interaction is a later gate.

