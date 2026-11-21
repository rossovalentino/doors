# ADR 0004 — Bounded initial memory ownership and four-level mapping

Status: implemented for the P03 single-CPU development profile; phase qualification
is tracked by the central phase ledger and reports.

The kernel library exports testable memory metadata separately from privileged
mapping operations. `PhysicalAddress`, `VirtualAddress`, `ByteCount`, `PageCount`,
`FrameNumber`, `FileOffset`, and `DeviceAddress` make conversions explicit. Physical
values are bounded to 52 architectural bits; dynamic bootstrap mappings are bounded
to the selected profile's first 4 GiB. Virtual arithmetic rejects the LA48 hole and
wrapping. A device bus address does not imply a CPU pointer or mapping.

The frame bitmap begins with all frames unavailable. Only non-overlapping UEFI
conventional regions explicitly marked `UsableAfterHandoff` become available. The
legacy low MiB and usable frames above bitmap coverage remain retained and counted.
Boot-service, runtime, loader, image, stack, table, framebuffer and reserved memory
never enter this initial free pool. A static 128 KiB bitmap covers 4 GiB without a
large stack allocation. There is no implicit firmware reclamation transition.

The allocator grants non-cloneable `OwnedFrames`, with a unique allocator identity,
generation and exact allocation record. Free requires that record; stale, foreign,
forged and repeated frees cannot make reserved pages available. Contiguous aligned
requests and the 320-record metadata limit (raised from 128 on 2026-10-25 with the
mapping records: every mapping of owned RAM holds one allocation) fail explicitly. Dropping an ownership
token retains its memory. The kernel establishes one allocator over the real boot
map; pure host constructors and tests only manipulate metadata.

`PageTables` adopts and scans the loader's reserved identity-mapped table pool. It
rejects table cycles, external pointers, large-page links and incompatible upper
permissions. Existing tables remain intact, and unused pages from the fixed512-page
pool supply new structures. Kernel mappings use four-level supervisor 4 KiB leaves;
the permission enum cannot express writable-executable pages. Any physical alias
with a boot or current dynamic mapping is rejected, independently of virtual overlap.

`map_frames` consumes frame ownership, initially maps RW/NX, invalidates local stale
translations, zeroes the RAM, and applies its requested final protection. The resulting
`MappedRegion` holds the frame token. Callers reserve an explicit virtual interval
and leave surrounding pages absent for guards. `protect` and `unmap` prevalidate every
leaf before changing any. `unmap` returns frame authority only after each leaf is
removed and invalidated. RAM must have no live Rust references or DMA users when it
is unmapped. Device MMIO mapping is a separate unsafe operation, applies PCD/PWT and
NX, never zeroes device registers, and carries no RAM frame authority.

A failed mapping rolls back published leaves and every newly allocated table link,
including invalidation before structural pages are reused. Successful unmap retains
structural table pages until address-space retirement; this bounded policy can return
exhaustion after many disjoint virtual regions, but cannot silently recycle a live
translation. There are 320 concurrent mapping records (raised from 128 on 2026-10-25:
one table holds every kernel alias of every process, and `process/vm.rs` asserts
that the declared process, thread and section capacity fits) and at most 65536
pages per map.
The unsafe API requires one online CPU, exclusive table ownership, the expected active
CR3, LA48 without PCID, and disabled interrupts. P04 must extend retirement to remote
CPUs before allowing concurrent mapping changes. No process/user mapping claim is made.
The active CPU must retain CR0.WP and EFER.NXE. Its PAT slot 0 must be write-back and
slot 3 uncacheable; unexpected firmware PAT state fails explicitly, so PCD/PWT device
mappings cannot silently acquire a cached memory type.

The bootstrap heap borrows a committed RAM slice and returns checked aligned blocks.
Allocation is fallible and zeroes each returned block. Failed requests leave the
cursor unchanged. It derives one base pointer from its original exclusive backing
borrow and retains that borrow's lifetime with `PhantomData`; later allocations use
checked offsets and raw writes only within fresh disjoint blocks. They never create
a mutable reference spanning previously issued pointers. Blocks, alignment padding,
heap frames and emergency allocations
are intentionally retained; there is no global allocator, implicit realloc/free,
sleeping allocator or allocating panic path. General heaps and service/process
accounting require later explicit lifetime and ownership policies.

Host tests cover address boundaries, overflow, firmware exclusion, low/high retained
memory, fragmentation, alignment, large requests, allocation-record exhaustion,
invalid/double/stale/foreign free, heap exhaustion/zeroing and access to older blocks
after later allocations, table-pool exhaustion,
partial rollback, guard holes, aliases, protected leaves, stale map tokens and malformed
table graphs. Host paging tests use inert tables and never execute `INVLPG` or dereference
guest virtual mappings. The guest integration must separately exercise zero/write/read,
protection faults, unmap/reuse and guarded exception stacks.

Architecture reference: Intel 64 and IA-32 Architectures Software Developer's Manual,
Volume 3, order 325384-081US, September 2023, sections 4.5, 4.6 and 4.10.4. Source:
<https://cdrdv2-public.intel.com/789582/325384-sdm-vol-3abcd.pdf>.

P04 amendment: mapping metadata grows to 128 bounded records for guarded task and
per-CPU stacks. The initial local invalidator is replaced before AP startup by a
synchronous retirement callback covering every online CPU. All publish/protect/
unmap/rollback paths call it before allowing backing reuse; failure is fatal and
never returns allocation authority. Structural tables remain retained. Only the
validated low startup code page can transition a loader-owned mapping from RW/NX
to RX, after preparation and before SIPI. The runtime requires a sole writer with
IRQs disabled and prohibits holding the scheduler lock across remote retirement.
See ADR0007 and the P04 guest mapping-reuse fixture for the extended contract.

P05 amendment: `ProcessSpace` owns a private 32-page table pool through an owned,
RW/NX supervisor alias. Its PML4 starts with an empty lower half and copies only
supervisor upper-half root entries. Physical table addresses are translated into
that alias before access; private process roots do not inherit firmware identity
mappings. Shared kernel mappings must stay beneath already-present upper root
entries. The selected kernel arena uses the existing PML4 entry 511.

Each process has at most 32 generation-checked reservations, each at most 4096
pages, in `[0x10000, 0x800000000000)`. Region IDs also bind the address-space
incarnation. Reserve does not commit; whole-region commit allocates and zeroes
owned RAM, copies bounded initial bytes, then publishes four-level user mappings.
Every intermediate table and leaf grants U/S explicitly. User mappings are never
global, huge, writable-executable, or backed by a device mapping. Unreserved pages
remain guard holes. Native whole-region reserve/commit is a bounded initial
contract, separate from future Windows allocation granularity and partial commit.

Private user RX and RO pages retain RO/NX kernel aliases. A transition to user RW
first removes executable user permission and acknowledges retirement, then permits
the kernel alias to write. A transition to RX first removes writable kernel alias
permission. Shared section views support RO/RW only; the external section owner
authorizes maximum rights and retains its kernel mapping until all views retire.
View teardown does not free section backing. The executive owns view reference
accounting and must update it after successful unmapping or process destruction.

Mutation and validate-then-copy run under one exclusive VM guard, with IRQs off
and trusted kernel CR3 active. The guard cannot be held alongside the scheduler
lock during remote retirement. `validate_user_range` checks the entire buffer,
reservation/commit state and effective U/S, RW and NX at every table level before
the architecture's fault-contained raw copy. No ordinary Rust reference into user
memory is created. A zero-length buffer still requires an in-range user address.
The VM guard must span validation and copy; pending operations may not outlive it
without a separate pin/lifetime protocol.

The process retirement callback covers every CPU that could retain the private
root, including paging-structure caches. PCID is disabled and all entries are
non-global; architecture trap entry and return reload CR3. The callback must obtain
remote acknowledgements before returning, and a timeout must fail closed. Failed
table publication removes every new leaf and link before pool reuse. Successful
unmap retains intermediate tables until process destruction. Data frames return to
the allocator only after user and kernel aliases retire. Page-table pool frames
return only after every context's architecture post-switch retirement and final
whole-root acknowledgement. Dropping the owner intentionally retains memory.

The native ELF loader separates structural inspection from execution policy.
It accepts at most eight static load segments, a 16 MiB file, and a 16 MiB rounded
memory total/span. Segments are page aligned, non-overlapping after rounding and
entirely in the user range; entry is in file-backed RX bytes. Failed loading rolls
back this load's committed regions and reservations, preserving earlier regions.
Stack, TLS, immutable startup bytes and guarded layout remain process policy.
Demand paging, file-backed sections, COW, swapping, ASLR and executable shared
sections are outside this bounded P05 mechanism. Host tests exercise private table
isolation, effective permissions, boundary validation, reservation identity/limits,
partial table rollback and ELF policy; actual CPL3 isolation/copy/retirement remains
a separate guest gate.

The privileged VM manager retains eight process slots and eight section backings.
Its original native image policy additionally bounds load addresses to
`0x400000..0x1400000`. User stack pages occupy
`0x70001000..0x70011000`, with absent pages on both sides. Startup is one RO page
at `0x71000000`; FS-based TLS is one RW page at `0x71002000`, initialized with its
self pointer and PID. Shared views use `0x72000000..0x80000000` and sections are
at most 16 pages. Supplied startup identity, entry and layout are validated before
allocation. Failed startup destroys the private root and all committed backing.

Kernel aliases use `0xffffffffd4000000 + slot * 0x04000000`; the table pool starts
there, image/data aliases start one MiB later and a guarded 64 KiB kernel stack
starts at offset `0x02001000`. Section aliases use the separate
`0xffffffffc8000000 + slot * 0x20000` arena, avoiding every process slot and the
P04 diagnostic fixtures. Fixed arenas are reused; intermediate shared kernel
tables still consume the bounded512-page pool. Destroying a section checks that
no process retains its shared backing identity in addition to requiring executive
reference retirement. Creating or tearing down mappings never acquires the
executive lock or scheduler lock inside the VM manager.

The guest startup rollback diagnostic uses a bounded construction budget on the
same builder and ELF loader used by ordinary creation. It fails once immediately
before and once immediately after table-pool construction, each individual ELF
segment, user stack, startup page, TLS page and kernel stack. An injected failure
is distinguished from an unexpected real failure. Each attempt must restore the
complete frame statistics and process/section counts, leave no process or stack
owner in the slot, and pass an actual page-table walk over every page in the
slot's 64 MiB kernel arena with no present leaves. Retained shared kernel
intermediate tables are intentional; they remain bounded by the original pool.

After exhausting all checkpoints, one real process construction must succeed in
that same arena. The diagnostic validates its actual executable entry, writable
stack/TLS, immutable readable startup and distinct live process publication before
explicit teardown and another complete baseline/arena check. No user context is
created during these attempts. The returned count is the observed injected
failures: `10 + 2 * PT_LOAD count`, at most 26. This kernel-only diagnostic has no
native ABI selector; production creation uses an unlimited budget through the
identical construction path. Host parser/model tests do not qualify these guest
allocation, mapping, shootdown or reclamation outcomes.

## P06 device-resource amendment

The initial RAM allocator remains below 4 GiB. Modern PCI MMIO BARs can reside
above that boundary: device mappings now check the CPU-reported CPUID 80000008
physical width, retain uncached RW/NX permissions, and never return device memory
to the RAM allocator. Normal RAM mappings retain the original limit. Before
mapping a BAR, a separate validator rejects overlap with firmware RAM/runtime,
boot mappings, framebuffer and local/I/O APIC resources. Tests cover the width
boundaries, RAM rejection, MMIO page-table flags and mapping-only reclamation.
The first real P06 boot exposed the previous hardcoded 4 GiB rejection; its failure
is retained at reports/runs/host-1791826365383963300-30108/report.json.

### P12 shared display surface reclamation

Large fonts/wallpaper and maximized surfaces exposed exhaustion of the supervisor
table pool. Unmapping now clears and synchronously retires all leaves, then scans
one time per touched2MiB range. Empty PTs and PDs have their shared parent entry
cleared, retired on every online CPU, and only then their pool bit is reusable.
PML4-linked PDPT pages stay pinned because process roots copy those links. Live
neighbor mappings prevent reclamation. The pool is expanded to512 pages with a fixed multiword allocation bitmap,
shared with the loader through the boot-protocol constant. The server may own a
224MiB heap in separate supervisor aliases at0xfffffffe00000000 +slot*224MiB,
never overlapping neighboring64MiB process arenas. Private reservations allow
at most65536 pages; per-manifest budgets still independently restrict other roles.
Actual simultaneous/widescreen resource checks remain required.

P12 native-output refinement: initfs permits32MiB immutable resource entries;
artifact inspector and native loader keep independent16MiB executable-file and
image-span caps. Total archive remains64MiB. The24,802,294-byte offline resource
pack occupies one read-only mapped Section, outside the compositor heap; ten
global Sections retain eight graphics slots plus a transient/recovery slot.
Wallpaper replacement reuses its destination allocation after validating decoded
source bytes. Supervisor/private table pools are512/256 pages. Rollback scratch is
bounded to134 links for a contiguous256MiB mapping (at most129 PT,2 PD,2 PDPT),
independent of pool capacity, preserving the128KiB guarded kernel stack. Empty PT
and PD tables retire synchronously; PML4-linked parent tables remain shared.
