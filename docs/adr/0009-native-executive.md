# ADR0009: Bounded native authority and IPC lifetime

Status: implemented for P05 integration; actual execution and closure are bound
to the selected phase's guest report and independent critic. Requirements are
master sections 11, 12, 18, 19, 135 and 136.1. The exact original binary contract
is `specs/abi/native-x64-v1.md`; its executable schema is `door-abi`.

## Decision and limits

Use an allocation-free executive with explicit object references and process-local
grant tables. P05 supports eight process records, 32 grants per process, 48 object
records, four bidirectional channel pairs, eight active sections, 256 payload bytes,
four grants per packet and four queued packets per receiving endpoint. A section
has zeroed, page-multiple backing up to 64 KiB. The scheduler still has 16 task slots;
the user runtime is N0 and does not own a general-purpose heap.

The master's 64 KiB payload and sixteen-handle limits were proposed defaults.
Keeping those defaults would multiply fixed packet storage and bounded copy work
before queues, allocation failure and resource cleanup have been qualified. The
chosen 256/four/four limits make queue and rollback costs explicit under the current
task/object limits. Sections carry larger data with independent mapping rights.
These limits are encoded in ABI discovery and host/guest tests; raising them is a
reviewed capacity and memory-budget change, not removal of a check.

## Authority and retirement

Trusted creation fixes process identity and executable personality. A user handle
is only an opaque identifier: every operation resolves it in the current process's
table and checks required rights. The executive issues global monotonic u64 handle
incarnations and process/object identities without wrapping; exhaustion fails before
publication. This avoids a guessed slot value accidentally naming a different grant
or reusing a stale incarnation. The exact counter representation remains private.

Duplicate and transfer only reduce authority. COPY creates another ownership
reference; MOVE invalidates the source grant at successful enqueue. Source grants,
packet headers, requested rights, duplicate transfer entries and queue capacity
are checked before the transaction commits. Queued grants retain object ownership
until receipt, cancellation, expiry or receiver closure. Receive prevalidates all
output extents/capacities and their disjointness before dequeuing and installing
grants. Authenticated sender and peer identities come from the channel state, never
from claimed payload credentials. Endpoint grants stay with their original owners;
transferring them is rejected to avoid authority confusion and endpoint cycles.

Section mappings hold explicit leases independently of handles. Closing a handle
does not revoke another grant or an active mapping. Final object retirement queues
backing cleanup for the VM owner after metadata locks are released. User roots,
kernel transition stacks and physical backing are reclaimed only after actual
context retirement and required translation invalidation. A successful exit has
stable process-object state so another process can wait even after the task stops.

## Cancellation and deadlines

Transport state is created, queued, then dispatched on successful receive. Application
completion is an upper-layer protocol event; the kernel does not fabricate it.
Send accepts strictly increasing nonzero request IDs per sending endpoint. Failed
queue-full or already-expired sends do not advance that committed sequence or move
a source grant. An explicit deadline is an absolute BSP monotonic tick value.

ChannelCancel is serialized with queue/receive operations. It returns 0 when it
removes a queued request; 2 on later observations of cancellation; 1 if receive
already committed; and 3 if a queued deadline expired. Unknown IDs return NotFound
and zero is invalid. Sixteen terminal outcomes per sender are retained; eviction
forgets the reported outcome but preserves the committed request-ID high-water mark.
No successful cancellation recreates a MOVE source handle: queued ownership is
released, because the original send already committed that ownership change.

Timed send/receive/cancel operations observe and retire expired queued packets.
Expiry drops queued grants and records one Expired outcome. It does not require an
unbounded timer list, a heap or a callback running under metadata locks. Recipient
closure releases/cancels its pending receive queue; sender closure permits the peer
to drain committed messages before PeerClosed. Dispatched work cannot be rolled
back by a later cancellation; its outcome must be resolved by the service protocol.

The lock provides the linearization point for cancellation/receive races and one
terminal transport outcome. Process metadata never holds its lock across page-table
changes, waits or remote translation acknowledgements. User-copy lifetime is held
by the VM lock/pinning discipline; preliminary pointer validation alone is insufficient.

## Evidence and limits of the decision

Host tests exercise forged/stale/foreign handles, rights reduction, resource
exhaustion, malformed framing, transactional transfer, queue pressure, cancellation,
expiry, history eviction and closure. Native guest fixtures additionally exercise
actual copy/MOVE grants, shared read-only views with live updates, rights denial,
bounded queues, precommit deadline rollback, queued expiry and cancellation after
delivery. A survivor waits for a contained user fault. All acquired grants and
section backing must return to the recorded baseline at teardown.

This bounds an early native transport, not a filesystem/network/device I/O contract,
an unbounded service bus, a durable transaction system or a Rust standard-library
port. Service request completion, restart/idempotency protocols and larger negotiated
limits require their own later-phase implementations and evidence.
