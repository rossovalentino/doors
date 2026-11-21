# Door native platform ABI 1

The current P10 ordinary-session extension is specified in
`specs/contracts/ordinary-session-v1.md`: file-captured unprivileged ELF/PE launch,
real startup lists/cwd, shell/utility roles, terminal controls and cancellation.
The fixed initfs manifest paragraph below describes historical service/fixture
admission; it does not restrict the new captured application path.

P08 extends the tagged native ABI without changing the P05 operation numbers or
256-byte startup record. All new records use explicit little-endian encoding;
unknown flags, nonzero reserved fields and wrong lengths fail before mutation.
This implementation contract passed the P08 cumulative guest gate and independent
critic. Current qualification of the P09 additions is recorded in the phase ledger.

## Ownership and startup

Native services are independent CPL3 ELF executables. The kernel assigns immutable
`Identity` fields: PID, UID, integrity, group bits, session ID/epoch, manifest ID,
service instance and capability mask. Payload names and startup descriptions do
not establish authority. Identity is 64 bytes: u64 PID at0, u32 UID at8, u32
integrity at12, then u64 groups/session/epoch/manifest/instance/capabilities at
16/24/32/40/48/56. Capability bits are INIT=1, BLOCK=2, SESSION=4, CONSOLE=8.

Manifest IDs: init=1, storage=2, session=3, console=4, native-workload=5,
Windows initial program=6. P09 mode6 selects that program; its trusted personality
has one thread, zero dynamic heap allowance and no device capabilities.
Each launch creates a new process and service instance. Executable bytes come
from authenticated initfs and their digest is copied into startup. Kernel policy
binds each catalog entry to its privileges; init cannot rewrite that policy.

`StartupV2` is 1024 bytes. Bytes0..256 use the existing Startup layout with magic
DOORUSR2, major1/minor1 and total length1024. Identity occupies256..320; executable
SHA256 occupies320..352; cwd handle at352; mode at360; argument/environment byte
counts (u16) at368/370; bytes372..384 reserved zero. Arguments occupy384..512;
environment512..768; tail768..1024 reserved zero. Counted fields contain bounded
UTF-8 NUL-terminated entries. Empty argument entries are retained; environment
entries require a nonempty key and an equals separator. Environment is
informational. A Directory grant is the working-directory authority.

Inherited slots in V2: 0 parent control channel (zero for init), 1 endpoint-registry
connect port (zero until init creates it), 2 role authority (Launch/Block/Session/
Console), 3 first LogonPermit for init, 4 second LogonPermit for init, 5 cwd grant.
The init process creates its registry Port and passes reduced connect grants on
launch. Service readiness sends its Port through the parent control channel;
init publishes it only after checking the expected PID/instance and protocol.

Kernel-issued LogonPermit grants are single-use credentials for fixed provisioned
UIDs1001/1002 in the development profile. Only the session authority can consume
one and create a Session grant. There are no fabricated random secrets or custom
password algorithms. Password authentication is provided only by the session
service over kernel entropy (P14, ADR 0020); entropy bytes are kernel-conditioned
and restricted to Session/Init-capability services (ENTROPY_READ below). Kernel handle tables enforce possession;
monotonic, nonwrapping identifiers prevent stale incarnations from aliasing.

## New trap operations

All unused arguments are zero. Errors use existing Status; RDX is zero on error.
Pointers denote captured records or fully validated output, with rollback if
publication cannot complete. Blocking calls retain resource references, release
metadata/VM/scheduler locks and revalidate output before delivery.

| Number | Name | Arguments / result |
|---|---|---|
| 0x103 | IDENTITY | channel handle (0=self), output64; authenticated peer/self |
| 0x210 | VM_ALLOCATE | page-aligned address, byte length, permissions(READ8/WRITE16); result address |
| 0x211 | VM_RELEASE | exact address,length; user promises no live accesses |
| 0x212 | VM_PROTECT | exact address,length, READ8/WRITE16; W+X disallowed |
| 0x213 | USER_COPY | destination,source,length<=1024; complete range validation, captured memmove; returns byte count |
| 0x302 | PROCESS_LAUNCH | launch grant,manifest,session grant(0=system),registry port,mode,output24(process handle/control channel/PID) |
| 0x303 | PROCESS_TERMINATE | process handle,exit code; INIT policy or own process only |
| 0x310 | THREAD_CREATE | executable entry,argument; result waitable Thread handle; SysV RDI=argument,RSI=0 |
| 0x311 | THREAD_EXIT | exit code; does not return; last thread completes process |
| 0x312 | THREAD_JOIN | Thread handle,output i64,absolute deadline ticks |
| 0x401 | SLEEP | absolute deadline ticks; real scheduler wait |
| 0x402 | ENTROPY_STATUS | no arguments, any process; result status word (`door_abi::entropy`): bit0 READY (seeded by a source that passed its health tests and no later test failed), bit1 RDSEED, bit2 RDRAND, bit3 TSC-jitter source in use, bits8..15 saturating health-failure count, bits16..31 reseed generation (low 16 bits); other bits zero. Never returns entropy |
| 0x405 | ENTROPY_READ | output pointer, length1..=64; result bytes written. Only processes whose kernel capabilities include Session or Init (AccessDenied otherwise); Unsupported while not READY. Sources: RDSEED, then RDRAND (bounded retries, stuck/all-0/all-1/repeated-word rejection), then TSC jitter (SP 800-90B repetition-count cutoff21 and adaptive-proportion cutoff410/512 on 8-bit deltas, H=1 assessed); SHA-256 conditioned into a hash DRBG rekeyed after every request and reseeded every64 requests or after a failed batch |
| 0x404 | WALL_TIME | no arguments, any process; result UTC FILETIME (100 ns since 1601-01-01): validated RTC reading (UIP clear, two identical samples, VRT set, 1601..9999) anchored to the 100 Hz tick and re-read once a minute; within 2 s the later of projection/RTC wins (no step back), larger disagreements follow the RTC. Unsupported = no valid RTC or no device owner (never guessed); WouldBlock = RTC stayed mid-update before any anchor. ±1 s accuracy (RTC whole seconds) |
| 0x510 | PORT_CREATE | result owned Port handle; accept restricted to creator |
| 0x511 | PORT_CONNECT | Port grant with WRITE; result client-owned Channel; accepted end belongs to server |
| 0x512 | PORT_ACCEPT | Port grant with READ; result server Channel or WouldBlock |
| 0x600 | WAIT_CREATE | kind(1 event,2 mutex,3 condition),initial(0/1 for event),flags(1 manual event); result handle |
| 0x601 | WAIT | handle,absolute deadline; result0 or1(abandoned mutex) |
| 0x602 | SIGNAL | handle,count(1 event/unlock mutex;0 condition broadcast,1 signal); result0 |
| 0x603 | RESET | event handle; result0 |
| 0x604 | CONDITION_WAIT | condition,owned mutex,absolute deadline; reacquires mutex on signal/timeout/ordinary cancel |
| 0x610 | SESSION_CREATE | authority grant,LogonPermit,administrators (bit i: account 1001+i is an administrator, so its applications carry the deny-only Administrators group); consumes permit on successful Session publication |
| 0x611 | SESSION_REVOKE | authority grant,Session grant; blocks new work and retires bound processes |
| 0x800 | BLOCK_INFO | Block grant,output32(sector_bytes u64,sectors u64,writable u64,flush u64) |
| 0x801 | BLOCK_READ | Block grant,LBA,output,byte length(512-aligned,<=4096) |
| 0x802 | BLOCK_WRITE | Block grant,LBA,input,byte length(512-aligned,<=4096) |
| 0x803 | BLOCK_FLUSH | Block grant; durable device flush |
| 0x810 | CONSOLE_WRITE | Console grant,input UTF-8,length<=1024; result accepted length |
| 0x811 | CONSOLE_READ | Console grant; result key byte or WouldBlock |

HandleKind additions: Port4,Thread5,Event6,Mutex7,Condition8,LogonPermit9,Session10,
Directory11,Block12,Console13,Launch14. Existing rights bits stay unchanged. COPY/
MOVE of eligible capabilities keeps or reduces rights; channel ownership never
changes through transfer. A Port retains its server identity and pending channel
queue; owner death rejects connects and closes unaccepted endpoints. Failed
connect/accept must not leak a handle, endpoint, grant or queued request.

The kernel retains PCI/MMIO/DMA. Only the storage manifest receives a Block grant.
Reads address the selected private data disk; writes are restricted to its ext2
home and journal extents. GPT/FAT and all other disks are immutable through this
interface. Copies use a bounded kernel buffer; filesystem/journal parsers and their
large working state execute in the storage process. Raw device failures remain
errors and restart mounts/replays the same persistent state.

## Runtime and qualification

N1 supplies a real mapped heap and bounded coalescing allocation/free with explicit
OOM. N2-foundation supplies threads, independent TLS/destructors, waits, timers,
files through authenticated IPC, lifecycle and entropy status. This is not a Rust
std port. P05's original N0 fixtures and all35 P07 guest cases remain mandatory.

The shared manifest policy in `door-abi::platform` grants each service one thread
and no dynamic heap, and the ordinary workload two live threads and1 MiB of dynamic
mapping. The kernel enforces these budgets before publication. The workload checks
that a third live thread and a mapping beyond the heap budget fail without leaking
resources. Per-thread user stacks are guarded256 KiB mappings; syscall stacks are
guarded64 KiB mappings. The single-CPU service profile bounds the whole graph to
eight process slots, sixteen threads,32 handles per process and sixteen channels.

`door-service-protocol::manifest` is the service contract catalog: endpoint versions,
operations, readiness, connection/file/key quotas, persistent-state dependencies
and shutdown policy. Its privilege/resource entries reuse the kernel ABI policy.
Session epochs are volatile authority state, not a durable credential store.
Public RPC payloads are explicitly encoded and request/instance scoped.
Process construction revalidates session authority under the same metadata lock
that installs its runtime records. Revocation either prevents that publication and
rolls construction back, or observes the installed process and stops its threads,
including threads that have not started. Stop requests validate the live scheduler
generation while publishing their terminal state; a stale task ID cannot overwrite
a reused slot's stop request.

The restart scenario additionally launches a second application in the existing
UID1002 session. It holds mapped allocation, an open file/connection and two live
threads, one waiting on an event and one in a condition wait. An authenticated
control barrier transfers the mutex to init, which acquires and retains it across
session revocation. Forced-stop cancellation must release the kernel continuation
without reacquiring that foreign-owned mutex; it completes syscall cleanup and
exits at the terminal safe point without returning to user code. Ordinary condition
cancellation still reacquires its mutex. The kernel must retire both threads, report exit-13,
deny a launch with the revoked grant and reclaim all process resources. Normal
application success markers cannot substitute for this forced-exit result.
The app closes its worker Thread handle before revocation. The kernel retains the
thread's completion-event lease through completion signalling; an early handle
close cannot retire the event while the reaper still uses it.

Console output is untrusted text. Its framebuffer rendering remains separate from
serial evidence: each serial write is a newline-terminated `DOOR_CONSOLE` record
with quoted/escaped text. CR/LF and controls cannot introduce kernel/process records
or absorb the next record. The ordinary SAVE application tests both spoofed lines
and a partial console write; raw trusted process identity remains kernel-owned.

New qualification must execute the four services plus an independently built SDK
application, persistent save/reboot/reopen, shared heap/distinct TLS/timers, denied
authority and session revocation, readiness/crash-loop/backoff, storage restart,
corrupt-state refusal, console input/output and complete resource retirement.
Exact scenario/serial/disk expectations belong in the P08 evidence contract before
the final unchanged-input run. Host model tests alone do not close this phase.
