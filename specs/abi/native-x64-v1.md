# Door Native ABI 1 — P05 N0 profile

This is an original Door contract. The reviewed executable schema is
`runtime/abi/src/lib.rs`, `startup.rs` and `wire.rs`: operations, statuses, limits and
checked little-endian encoders/decoders are shared by kernel and native libraries.
`door-sys` implements the trap boundary; `door-rt` implements allocation-free N0
startup/termination. Kernel code depends on `door-abi`, never either user library.
No stable Rust plugin ABI, C SDK, libc, allocator or Rust `std` port is claimed.
The independent assembly fixture intentionally repeats published literals so a
matching kernel/wrapper mistake cannot establish ABI correctness by itself.

## Executable and startup

Executables are static ELF64 AMD64, explicitly built with the pinned Rust
`x86_64-unknown-none` target, small code model, static relocation, no red zone and
`lto=off`. The supplied core library has a different code-model LTO flag; disabling
LTO prevents mixing those incompatible module flags. The target uses its soft-float
compiler ABI. Explicit x87/MMX/SSE2 state is initialized/preserved by the kernel;
AVX and OSXSAVE remain disabled. Integer/pointer calls use the target's SysV-style
convention; this does not qualify a complete interlanguage floating-point ABI.

The fixed link base is 0x400000, with separate RX, R and RW segments, 4 KiB alignment
and no page shared by differently protected segments. Loader bounds are eight
PT_LOAD segments, 8 MiB file, 16 MiB rounded committed memory and virtual span,
and user addresses in [0x10000, 0x0000800000000000). Dynamic/interpreted executables,
W+X, non-user addresses, malformed extents and page-rounded overlap fail.
The initial VM launch policy further confines image segments to
[0x400000, 0x1400000). It places the guarded stack at 0x70001000..0x70011000,
startup at 0x71000000 (R), TLS at 0x71002000 (RW), and section mappings in
0x72000000..0x80000000. Each committed backing is zeroed before file bytes are
copied, then assigned its final user permissions.

At `door_user_entry`, RDI addresses the read-only 256-byte startup record, RSI is
256, and RSP mod 16 is 8 as at a SysV function entry. Stack pages are zeroed and
guarded; `stack_top` below describes the aligned top before the initial eight-byte
slot. FS points to separate writable per-thread TLS: u64 self pointer at offset 0
and trusted process ID at offset 8. GS does not alias this native FS contract.
The entry copies and validates the record before calling Rust code.

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | ASCII `DOORUSR1` |
| 8, 10 | 2 each | Major 1, minor 0 |
| 12 | 4 | Total bytes 256 |
| 16, 24 | 8 each | Process ID, channel peer process ID (0 if absent) |
| 32 | 8 | Initial fixture role |
| 40, 48, 56 | 8 each | Entry, aligned TLS base, aligned stack top |
| 64 | 192 | Eight inherited slots, each `{handle, rights, kind}` as three u64 |

Unused inherited slots are entirely zero. Nonzero handles have known kinds and
rights, and no duplicate handle appears. Record process identity/personality and
rights are informational copies; changing user bytes never changes kernel authority.
P05 supplies no argument/environment/file-directory facility. Adding those fields
requires an explicit layout/version extension rather than unused pointer scaffolds.

## Trap convention and result

RAX = `0xD001000000000000 | operation_u32`; bits 32–47 must be zero. Arguments 0–5
are RDI, RSI, RDX, R10, R8, R9. RAX returns unsigned Door status; RDX is the scalar
result and is zero on failure. RCX and R11 are architectural clobbers. Other GPRs,
RSP, FS TLS and enabled extended state are preserved. Wrappers declare memory
reads/writes and actual clobbers; they never use `nomem` or `readonly` for a trap.
Untagged selectors fail InvalidOperation. Nonzero reserved selector bits fail
VersionMismatch. Unknown operation numbers fail InvalidOperation before accessing
arguments; known unavailable facilities fail Unsupported.

The trusted entry switches to a kernel-owned stack and retains user return state
separately. Return addresses, user stack, flags and selectors must pass architecture
validation. The initial implementation uses validated IRET return. A noncanonical
user RSP is a contained process failure, never permission to dereference it as a
trusted kernel stack. Trusted process personality is not selected by a syscall tag.

Statuses are project-owned u64 values, not NTSTATUS or Win32 errors:
0 Success, 1 InvalidOperation, 2 VersionMismatch, 3 InvalidArgument, 4 BadAddress,
5 BadHandle, 6 AccessDenied, 7 NotFound, 8 AlreadyExists, 9 Unsupported,
10 ResourceExhausted, 11 WouldBlock, 12 Timeout, 13 Canceled, 14 PeerClosed,
15 DeviceLost, 16 IoFailure.

## Operations

Diagnostic Write is bounded to 1024 bytes per call and a 16 KiB lifetime budget
per process (`PROCESS_DIAGNOSTIC_BUDGET`). Oversized calls fail InvalidArgument;
budget exhaustion fails ResourceExhausted without output or consuming remaining
credit. Invalid buffers/UTF-8/carriage returns fail before charging. Every output
line receives a trusted process attribution prefix. A new process has its own
budget; duplicated handles never transfer diagnostic credit.

Unused arguments are zero. All addresses and lengths are u64; fixed output records
use little-endian bytes, without casting arbitrary user buffers to Rust structs.
Exit codes are signed i64 carried as their two's-complement u64 representation.

| ID | Operation and arguments | Success result |
|---|---|---|
| 0x0000 | Query(output, size=64) | 64-byte ABI information |
| 0x0001 | Exit(code) | Nonreturning; stable exit state wakes process waiters |
| 0x0002 | Write(bytes, length<=1024) | RDX written count; attributed literal output |
| 0x0100 | Close(handle) | 0 |
| 0x0101 | Duplicate(handle, reduced_rights) | New process-local handle |
| 0x0102 | Inspect(handle, output32) | `{kind, rights, object_id, generation}` u64 |
| 0x0200 | SectionCreate(bytes, rights) | Zero-filled section handle |
| 0x0201 | SectionMap(handle, page_aligned_address, rights) | Mapped byte count |
| 0x0202 | SectionUnmap(address, bytes) | 0 after translation retirement |
| 0x0300 | ProcessWait(handle, output_i64, deadline_ticks) | Stable exit code in output |
| 0x0301 | Yield() | 0 |
| 0x0400 | Time() | Monotonic BSP ticks, nominal 100 Hz |
| 0x0500 | ChannelCreate(output_pair16) | Two handles, eight bytes each |
| 0x0501 | ChannelSend(handle, packet, packet_bytes, transfers, count, flags=0) | 0 |
| 0x0502 | ChannelReceive(handle, packet_output, capacity, grants_output, grant_capacity, metadata_output) | Packet, grants, metadata |
| 0x0503 | ChannelPeer(handle) | Kernel-bound peer process ID |
| 0x0504 | ChannelCancel(handle, request_id) | Canceled=0, Dispatched=1, AlreadyCanceled=2, Expired=3 |
| 0x0505 | ChannelSendDeadline(handle, packet, packet_bytes, transfers, count, absolute_tick_deadline) | 0 if queued before deadline |
| 0x0700 | Probe(a0,a1,a2,a3,a4,a5) | Diagnostic checksum described below |

Query64: major/minor/pointer bits are u16 at 0/2/4; 6..8 zero. Supported family bits
are u64 at 8 (families 0,1,2,3,4,5,7); this means the documented operation prefix,
not every reserved ID. Payload/transfer/queue/startup/diagnostic limits are u32 at
16/20/24/28/32. Bytes 36..40 are zero, tick Hz is u64 at 40, and 48..64 are zero.

Probe result is `rol(a0,1) ^ rol(a1,7) ^ rol(a2,13) ^ rol(a3,23) ^ rol(a4,37) ^
rol(a5,53) ^ 0xa5c36f19d2478be0`. This bounded diagnostic performs no pointer access
or authority change. The raw caller includes large values and two's-complement -7.

## Handles, sections and message ownership

Handles are nonzero opaque u64 grants validated against the caller's process table
and incarnation. The P05 executive issues globally unique increasing handle values,
checked for exhaustion without wrapping; it does not expose table-slot bits as an ABI.
The inspection generation is an opaque incarnation token. Kinds are Channel=1,
Section=2, Process=3. Rights: QUERY=1,
DUPLICATE=2, TRANSFER=4, READ=8, WRITE=16, WAIT=32, MAP=64; unknown bits fail.
Duplication and transfer can only reduce rights. Closing one handle does not revoke
other copies or a live section mapping. Reuse must not make a stale handle valid.
Section map requests allow READ or READ|WRITE; WRITE alone is InvalidArgument
before authority lookup. A valid READ|WRITE request through a read-only grant is
AccessDenied. Section map rights cannot exceed the grant, and user-map changes require safe
retirement before backing reuse. The unsafe Rust mapping wrappers expose the
caller's obligation to avoid invalidating live Rust pointers or using unsynchronized
cross-process aliases. Fixtures use volatile access and channel-ordered updates.

P05 bounds are 256 inline payload bytes, four transferred grants and queue depth
four per endpoint. These intentionally smaller limits replace the master's proposed
64 KiB/16-handle defaults for this bounded diagnostic profile. Larger content uses
sections. The executive is bounded to eight processes including the supervisor,
32 handles per process, 48 objects, four channel pairs and eight active sections.
Sections are page-sized multiples up to 64 KiB. The rationale and ownership model
are recorded in ADR0009 (`docs/adr/0009-native-executive.md`).

Packets use exactly the master's 32-byte header: magic u32 0x524f4f44 at 0,
major/minor u16 at 4/6, opcode u32 at 8, zero flags u32 at 12, request ID u64 at 16,
payload bytes u32 at 24, transfer count u16 at 28 and zero u16 at 30. Total packet
bytes must equal 32+payload bytes. IDs are nonzero and strictly increasing per
sending endpoint; a failed queue-full attempt does not consume its ID.

A transfer record is 32 bytes: handle, reduced rights, mode, reserved, all u64;
mode is copy=0 or move=1 and reserved is zero. Source handles are checked together,
and delivery atomically installs reduced destination grants. Failed sends preserve
source ownership and destination state. Receive returns the installed handles in
the same format with mode=0. Endpoints remain bound to their process owners and
cannot be transferred across owners; section/process grants can be transferred.

Receive metadata is 32 bytes of u64 sender_pid, packet_bytes, handle_count and
request_id. Sender identity comes from kernel state. Caller-supplied names/PIDs do
not authenticate peers. Empty receive returns WouldBlock; queued packets drain
before PeerClosed. An insufficient or invalid output does not dequeue or partly
write a message. All output extents are validated before delivery; small request
descriptors are captured before interpretation. The P05 execution path must keep
captured user mappings stable for the synchronous copy; checking a pointer alone
does not establish this lifetime.

Deadline sends use absolute BSP monotonic ticks, not relative durations. A deadline
at or before the dispatch time returns Timeout before enqueue, leaves MOVE grants
owned by the sender and does not consume the request ID. The ordinary send keeps
its existing no-deadline behavior. A queued deadline is evaluated on timed send,
receive or cancel; expiry removes the packet, releases queued grants and records
Expired. It is not a hard realtime timer or an application-side rollback.

Cancel requires the sending endpoint and a nonzero request ID. A queued request
is removed atomically and returns Canceled (0); observing that result again returns
AlreadyCanceled (2). If receive already committed, the result is Dispatched (1)
and no delivered grant or side effect is rolled back. An expired queued request
returns Expired (3), including repeated observations. Unknown IDs return NotFound;
ID zero returns InvalidArgument. Sixteen terminal outcomes are retained per sending
endpoint (`CANCEL_HISTORY_DEPTH`); older evicted outcomes return NotFound while the
highest committed request ID remains recorded, so forgotten IDs still cannot be
reused. Closing a recipient cancels its queued messages; closing a sender allows
the recipient to drain previously committed outgoing messages before PeerClosed.

MOVE ownership commits when send successfully queues. Cancel/expiry releases the
queued ownership rather than recreating the old source handle. COPY keeps the
original grant. Applications must distinguish this transport commit from service
completion; receiving an operation does not prove its application effects completed.
These calls report one serialized terminal outcome, not an ambiguous promise that
a dispatched request was undone.

## Independently built fixtures and evidence

`tests/fixtures/native` produces `native-runtime` and `native-probe` in separate
Cargo invocations and link maps. Each controlled pass inspects both full ELF bytes
and stores them in `bin/native-runtime.elf` and `bin/native-probe.elf` in initfs.
The two-pass record retains firmware/kernel/initfs first, then those two artifacts.
Every native file has its own source binding, hash, inspection, link map and exact
archive destination. Historical P02–P04 three-artifact records stay historical.

Runtime roles: client=1, server=2, malicious read=3, survivor=4, probe=5,
invalid return=6. Slot 0 carries a WAIT|QUERY grant to the malicious process;
client/server slot 1 is their own channel and slot 2 may carry the peer process.
The client writes a zero-filled shared page at 0x72000000, transfers only QUERY|
READ|MAP, and changes its value between two acknowledged exchanges. The server
must see both values while duplication escalation and writable mapping are denied.
The client tests partial output, invalid user extent, stale handles, queue saturation,
failed-send ID rollback, MOVE invalidation, handle-table exhaustion, queued cancellation,
duplicate cancellation, precommit/queued deadlines, dispatched cancellation and endpoint
closure, with cleanup of each successfully acquired grant. It emits
`DOOR_P05 IPC_LIFETIME PASS` only after these lifecycle checks. The survivor observes exit -14 from
the supervisor-page read fault and continues. Invalid return must exit -257.
Successful exits are client11, server22, survivor44 and raw probe55.
Runtime failures retain `role`, stable `stage` and numeric `status` in one bounded
attributed diagnostic line, and exit with `-(1000 + stage*100 + status)`.
Stages group startup (1), client exchange (10–13), server exchange (20–24),
negative pointer/handle/queue/MOVE/pressure cases (30–34), cancellation/deadline
stages (35–38) and survivor wait (40).

The raw assembly fixture imports no ABI/runtime symbols. It checks literal startup
layout/alignment, all six syscall arguments, non-return GPRs, RSP, FS, x87 and
XMM0/XMM15, namespace rejection and partial-output failure. Source compilation,
host parser tests or matching ELF hashes alone cannot qualify protected execution;
actual CPL3 execution and independent critic acceptance remain phase gate evidence.
