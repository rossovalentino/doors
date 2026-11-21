# Native system services

These are independent, statically linked CPL3 ELF programs for the Door native
ABI. `door-rt` supplies startup, `door-sys` supplies kernel mechanisms, and
`door-service-protocol` describes the user service messages. Native execution is
qualified by the active phase gate; host tests alone are insufficient.

| Executable | Authority and responsibility |
| --- | --- |
| `door-init` | Launch capability; dependency graph, authenticated endpoint registry, readiness deadlines, restart backoff and reverse dependency shutdown. |
| `door-storage` | Block capability; one serialized owner of the existing redo journal, ext2 filesystem, namespace, ACLs, typed settings and registry. |
| `door-session` | Session authority; consume kernel-issued single-use local logon permits, return session grants and revoke their epochs. |
| `door-console` | Console capability; bounded UTF-8 output and key input restricted to the active terminal session. |

The kernel assigns executable identity, UID, capabilities and service instance.
An IPC payload cannot choose these fields. Each request rechecks the kernel's
authenticated channel identity, including session revocation. Readiness arrives
on the launcher-created control channel and binds PID, manifest and instance.
The registry publishes connect-only Port grants after readiness validation.

`door-abi::platform::manifest_policy` is the shared launch-policy catalog; the
kernel uses it for identity, secondary-thread admission and total anonymous
allocation admission. `service-protocol::manifest::CONTRACTS` references that
policy and declares endpoint versions, connection/file/key capacities, readiness,
persistent-state ownership and shutdown behavior. The supervisor graph specifies
the exact executable, dependencies, readiness/stop deadlines and restart bounds.

| Manifest | Fixed UID | Capability | Total threads | Anonymous allocation budget |
| --- | --- | --- | --- | --- |
| Init | 1 | Launch | 1 | 0 |
| Storage | 1 | Block | 1 | 0 |
| Session | 2 | Session authority | 1 | 0 |
| Console | 3 | Console | 1 | 0 |
| Independent workload | Authenticated Session UID | None | 2 | 1 MiB |

These allocation budgets apply to explicit anonymous VM requests, including
requests outside the SDK heap address. Executable/static data and guarded stacks
are separate loader-owned mappings. The selected profile also retains global
bounds of eight processes, sixteen scheduler tasks, 32 handles per process,
96 executive objects, sixteen channel pairs and eight shared sections. They are
shared capacity limits, not reserved allocations per service. Every service has
eight client slots; storage additionally has sixteen file and sixteen key slots.

`storage-core` owns the single implementation of the P06/P07 filesystem and
persistent namespace. The kernel reexports it for historical diagnostics; the
storage service uses it directly through a bounded Block syscall adapter. A
separate static working set avoids copying the journal onto a thread stack.
Only explicit first-boot provisioning on a fresh generation-zero SAVE image may
create the catalog and the two development user directories. Missing or damaged
state during reopen/restart fails closed. No error triggers an empty default.

`server.rs` provides a fair serialized loop over eight authenticated connections.
The native channel header wraps a service payload with version 1, status and
service instance. Request IDs increase per connection; stale instances and
replayed IDs are rejected before dispatch. A failed response after mutation is
an ambiguous completion, and the SDK does not retry it automatically. Inline file
chunks are limited to 192 bytes, paths to 192 UTF-8 bytes, and registry values to 64
bytes. Larger files are transferred in chunks within the existing 48 KiB backend
limit. File and key grants are bound to connection, PID and session epoch, and
close on disconnect. SDK objects also retain connection and instance identity.

The storage RPC includes open/read/write/flush/close, per-user typed settings,
and lossless UTF-16 registry open/query/set/close. Native path lookup remains
subject to the existing ACL and sharing checks. Registry roots/views/types retain
the P07 semantics. Kernel Block writes are confined to the selected ext2 and
journal partitions; these services cannot replace GPT or write another disk.

Authentication is deliberately capability based: boot policy issues development
permits for UID 1001/1002, and the kernel consumes each credential object once,
including its duplicate handles. There is no password verifier, fabricated random
secret or claim of a Rust `std` port. The current terminal permits one active
session; closing its last connection releases ownership and queued input is
drained before a new session starts. GUI input authority is a later phase.

The qualification workload is built separately against `service-client`. SAVE
and REOPEN run both user identities; RESTART includes an intentionally unready
console, deadline termination, bounded restart, storage death/recovery and stale
endpoint rejection. Both identities set/query `HKCU64\Door\Epoch` as a DWORD
containing their UID before the same file/settings flush. Reopen validates these
values without mutation; machine writes, writes through read-only key grants and
absent 32-bit views are checked separately. RESTART then revokes the live second
session while a fresh app retains heap and a file connection. Its worker enters
a condition wait and transfers a reduced mutex grant to init; init acquires that
mutex and keeps ownership while the worker and main thread are forcibly stopped.
The probe requires process exit `-13` before init unlocks the mutex, proving that
forced condition-wait cleanup does not depend on reacquiring a mutex held by a
different live process. Ordinary condition timeout still reacquires its mutex.
The revoked Session grant must also fail another launch. FAILURE uses a damaged catalog and verifies four actual
storage refusals with 10/20/40-tick restart delays. These modes use real process
and service paths. The DEMO workload is an ordinary user process using the same
console and storage RPCs.

Portable checks: `cargo test -p door-storage-core -p door-service-protocol
-p door-service-client -p door-system-services --lib`. Build guest services with
the repository `cargo xtask build` workflow, which records linker inputs, source
identity, artifact hashes and authenticated initfs paths.
