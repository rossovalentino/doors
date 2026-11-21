# Door OS engineering workflow

The user requested one phase at a time, professional organization, coordinated
feature integration, updated documentation, and a critic review after each phase.
Treat the root specification as reference requirements, within that authorized scope.

Before editing, read `project/STATUS.md`, `project/NEXT_STEPS.md`, the phase ledger,
relevant contracts and ADRs. Preserve user changes. Implement only the active phase;
finish its tests, integration and documentation before starting another phase.
(2026-10-23, owner decision 6: P13 proceeds in parallel with the selected P12 under
the ledger's `parallel_authorization`; P13 cannot pass before P12. See ADR 0017.)

Create directories and crates when they have real behavior. Use `door-` package
names, keep guest libraries allocation-free where required, and keep host tooling in
`tools/xtask`. Extend that orchestrator instead of retaining disposable scripts.
Do not add empty kernel, service or GUI scaffolds. Build target selection is explicit.

Use `cargo xtask verify` for the selected cumulative gate. Check actual logs and exit codes. Update
status, roadmap, requirements, source/dependency records, risks and handoff together.
Host results cannot establish guest, compatibility or visual claims.
Passed historical phases retain their original evidence. (Amended 2026-10-23 with
owner approval: the owner deleted the P00–P11 qualifying reports for disk space, so
those phases carry the reviewed `evidence_disposition` "unavailable_owner_confirmed"
of ADR 0017. Their run IDs and critic files remain identity; never recreate a report
under an old ID. The cumulative verify of the selected phase re-executes every
inherited command and guest scenario on current source. No other phase may use it.)
(Amended 2026-11-11 with owner approval: the owner attested that P12-P14
passed and deleted that evidence before it was recorded. Those three phases
carry the weaker `passed_owner_attested` disposition of ADR 0027: no report,
run ID or critic is claimed or may be recreated. P15 is the selected phase;
its cumulative verify re-executes every P12-P14 check on current source. No
other phase may use the attestation, and every later gate's report is kept.
The owner chose one cumulative `verify --phase P17` to close P15-P17: P15
and P16 cite that report, which must contain all of their commands.)
The selected phase needs current input identity, required checks and a critic bound
to the qualifying report.
P02 additionally requires actual guest artifacts, all six boot/failure outcomes,
diagnostic key/screen checks and preserved guest evidence. Assembly/linker/config
inputs belong in source identity. Hosted screenshots do not qualify guest execution.
P03 adds the acquired ACPI/processor contracts, frame/heap/mapping tests, owned
descriptor/exception diagnostics, calibrated APIC delivery, deliberate UD/double
fault/RO/NX/guard cases and the separately pinned 256 MiB minimum profile. Preserve
the historical 2 GiB profile and passed P02 evidence.
P04 adds actual timer-preempted contexts, enabled GPR/TLS/x87/SSE preservation,
two/four-CPU profiles, observed AP readiness/degraded mode, native wait races and
acknowledged remote TLB retirement. Selecting another context does not retire the
old stack: only the architecture post-switch commit may publish its migration or
reaping. Preserve AP idle/emergency stacks while those CPUs remain live. Host
scheduler tests cannot qualify actual preemption or SMP; ring-0 tasks cannot
qualify the later protected native execution phase.
P05 requires independently built native ELF artifacts inside authenticated initfs,
private user address spaces, trusted syscall stacks and validated returns. Keep VM
ownership separate from metadata/scheduler locks; a CPU contending for the VM lock
must service retirement IPIs. Validate and pin complete user buffers before capture
or output. Object references distinguish handles, queued transfers and mapped views.
Contain user faults and malformed returns, retain stable waitable exit records, and
reclaim user frames only after every context retires. Preserve all earlier actual
boot, protection and SMP cases. Native fixture failures include stage/status data.
P06 adds private raw-disk images, validated PCI resources, modern VirtIO queues,
acknowledged DMA quiescence, mirrored GPT validation and bounded VFS backends.
Keep filesystem identity separate from pathname spelling. A failed write/flush
must poison mutation until validated remount; partial reads never populate cache.
Qualify persistence across separate guest boots and independently inspect raw disk
bytes. Preserve immutable historical profiles and every prior guest regression.
P07 must coordinate file bytes, namespace and authorization in the same durable
transaction. Validate all journal records before replay, preserve the last durable
checkpoint during payload reuse, and poison failed live state until remount.
Crash tests use actual separate guest boots; preserve interrupted images and prove
replay idempotence. Lossless UTF-16 lookup, effective identity and symmetric sharing
checks cannot be replaced by path normalization or caller-supplied names.
Reopen a changed selected phase before verification; extend schemas/gates explicitly
when introducing later phases.

P08 requires real protected service executables and an independent native SDK app,
kernel-authenticated peer identities, readiness/restart/session revocation and
retirement of every thread before reclaiming its resources. Keep the filesystem
implementation shared with earlier tests; a service owns the volume while kernel
drivers own DMA. A custom password hash or deterministic seed is not an entropy or
credential provider. Preserve the declared development capability-authentication
boundary and explicit N0/N1/N2-foundation levels. Require kernel-owned incarnation
records and independent persistent-byte audits, not service self-report alone.

The owner authorized sequential implementation through P15, with this full routine
for each phase. After P10, qualify the combined P00–P10 subsystem suite before
starting P11 GUI foundations. Phase order and review obligations still apply;
implementation agents may own disjoint work while the selected phase is active.

P09 adds a trusted Windows x64 process personality, bounded PE dependency mapping,
original PE compatibility DLLs and an independently compiled ordinary Windows
fixture. Preserve the fixture bytes across Windows reference and Door guest runs.
Keep user GS/TEB distinct from native FS and trusted kernel GS. Validate complete
image/fixup plans and roll construction back before publishing any runnable task.
DLLs capture user pointers through the native checked-copy boundary and route
console output through the existing authenticated service. Initial single-thread,
no-CRT scope does not qualify P10 DLL/TLS/unwind or broad application support.

When implementation and documentation for a phase are ready, spawn a critic agent
to independently review code, integration, negative cases and evidence. Have it
report concrete bugs and limitations; fix actionable findings and rerun affected
checks before closing the phase. Review does not guarantee absence of all bugs.
Keep critic work read-only unless a separate edit area is explicitly assigned.

Keep local build output and raw evidence under ignored `target/` and `reports/runs/`.
Do not remove failed evidence to make a gate appear clean. No project license has
been selected. Git is initialized locally; commits/publication are separate actions.

