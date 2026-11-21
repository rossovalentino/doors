# Read-only native diagnostics v1 (P12 development)

SYSTEM_SNAPSHOT0x403 takes exactly a private output pointer and608-byte length;
remaining arguments are zero. Global native dispatch revalidates live authority.
The immutable Monitor manifest14 additionally requires a nonzero live session.
No caller-provided PID, UID, path, command, handle or sampling source is accepted.
Other manifests are denied. User output is completely validated under the VM
lock; no shared output alias, kernel pointer or process capability is returned.

The96-byte little-endian header has version1/count at0/4; BSP ticks at8; online
CPU count at16; reserved20..24; total/busy timer observations at24/32; managed/
allocated physical pages at40/48; device read/write/flush request counters at56/
64/72; reserved80..96. Eight64-byte process slots follow. A live record contains
PID/immutable incarnation at0/8, manifest/UID/launch mode/state/thread count at
16/20/24/28/32, reserved36..40, timer observations at40, private committed bytes
at48 and reserved56..64. Unused records/reserved bytes are zero. Version, lengths,
bounds, identities, totals, state/thread counts and page alignment are checked.

Visible records are the fixed service manifests1..4|11 and ordinary processes in
the caller's exact UID/session/epoch. Aggregate physical memory and device request
counts describe the whole machine. Process private bytes exclude reservations,
shared views and page tables; they are not a Windows working-set measurement.
PID/incarnation is captured before VM observation and checked again afterward,
so a replacement in the same slot cannot inherit another process's charge.
Metadata, VM and scheduler/device locks are acquired separately, never nested.
This is a bounded observational snapshot, not one atomic timestamp for every field.

CPU is sampled once for each advancing BSP timer observation across online CPUs.
A selected managed task contributes one observation; idle contributes no busy
observation. Duplicate ticks do not charge work. Missed observations are not
invented as exact runtime. Percentages use differences between real observations;
first/reset/resumed intervals are unknown. Device rates use request-counter and
actual tick differences; disk busy%, byte throughput, GPU/network, service health,
event logs and startup configuration are not inferred. Counters are bounded and
presentation arithmetic uses checked differences and widened products.

The API grants no termination, memory access, service control or tracing authority.
The current native recovery report binds a denied manifest5 caller to its kernel
retirement record. Malformed pointers and high-density stress remain separate
evidence requirements; portable schema tests alone cannot qualify those boundaries.
