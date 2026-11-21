# ADR 0014: Native user platform and authority

Status: adopted; P08 cumulative guest gate and independent critic passed.

Four protected, independently linked native ELF processes own initial userland:
init supervises dependencies and endpoint publication; storage owns one journaled
volume and file/config/registry policy; session consumes logon credentials and
revokes session scope; console owns terminal input/output routing. An ordinary
SDK application uses these interfaces and has no service or raw-device authority.

The original portable storage/persistence implementation moves to storage-core.
Kernel re-exports retain prior regression callers. This avoids divergent filesystem
implementations while keeping large filesystem/parser frames out of syscall stacks.
Kernel block calls capture at most4096 bytes and restrict writes to validated home
and journal extents on the selected private data disk.

Processes own address spaces, identity and handles. Threads own guarded stacks,
FS TLS and execution contexts. Forced termination marks the process stopping,
cancels waits and retires threads at safe boundaries; active kernel operations keep
their resources until completion. Stack switching alone does not prove retirement.
Port connections create channels with the correct immutable endpoint owners.
Forced stop can abort a condition wait without reacquiring a mutex held by a live
foreign process. This abort resumes only kernel cleanup and terminal exit; it
cannot return to user code. Ordinary cancellation preserves mutex reacquisition.
Thread completion retains its runtime lease until after the completion event has
been signalled. Closing the user handle early cannot retire that event between
publishing the exit value and signalling waiters.
Each bound process retains its session object until full retirement. The last
handle, queued grant or process reference reclaims the session slot; failed
publication therefore cannot permanently consume the bounded session table.
Process publication revalidates session authority atomically with installing the
runtime records. Revocation before this boundary rolls construction back; revocation
after it sees the installed process, including a not-yet-started thread. Scheduler
generation validation and stop publication share one lock so stale stop snapshots
cannot overwrite the terminal state of a reused task slot.

The development authentication provider accepts kernel-issued single-use LogonPermit
capabilities for explicitly provisioned identities. It does not claim password
authentication. Session epochs are revocable kernel state; numbers or strings in
messages cannot forge them. The previous unqualified custom KDF and seed-generated
bearer tokens are removed. Entropy availability is explicitly unavailable; later
providers must be acquired, reviewed and tested before security consumers use them.

N1 uses a real mapped1MiB heap, bounded metadata and coalescing free extents. Native
runtime adapters supply threads, bounded TLS destructors, waits, clocks, IPC and
service files as N2-foundation, separately from any maintained Rust std port.

Qualification retains all earlier guest cases and independently checks exact app
bytes/settings, reboot/reopen, service restart and corrupt-state refusal. The host
binds service assertions to kernel-owned PID/manifest/instance/session records,
executable hashes, termination results and final resource counts.

The final gate `host-1792060739947826700-17608` passed 281 host tests, all 39 guest
scenarios and both interactive checks with matching builds and unchanged inputs.
`reports/critics/p08-critic.json` accepts that exact report with zero unresolved findings.
The four fixed findings cover foreign-mutex forced stop, console record framing,
completion-event lease ordering and process publication during session revocation.
