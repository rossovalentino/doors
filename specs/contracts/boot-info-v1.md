# Door BootInfo v1: portable envelope and P02 handoff

Owner: `door-boot-protocol`. Sources: MASTER-001 section 133, RUST-LAYOUT,
UEFI-MEMORY-211 in `specs/sources/index.json`. Original Door wire format, not a
UEFI/Windows structure. Scope: envelope, memory entries and ownership validation,
with checked P02 module/mapping/framebuffer extensions. Implementation:
`crates/boot-protocol/src`; producer/consumer: `boot/uefi` and `kernel`.

All multibyte values are little endian. Encoders write fixed records into supplied
buffers; they allocate nothing and reject invalid values/short buffers before
writing. `Header::decode` reads/validates the fixed header; `BootInfo::parse` also
checks the full claimed extent before returning a borrowed immutable view. Trailing
input beyond `total_bytes` is ignored. Values are parsed explicitly, so the slice's
host alignment is unrestricted. No physical address is dereferenced by this crate.

| Header offset | Field | Representation |
|---|---|---|
| 0 | magic | 8 bytes `DOORBOOT` |
| 8 / 10 | major / minor | u16 = 1 / u16 = 0; other versions rejected |
| 12 | header_bytes | u32 = 64 |
| 16 | total_bytes | u64, 64..1 MiB |
| 24 | flags | u64 = 0 |
| 32 / 36 | table_count / table_entry_bytes | u32 <= 64 / u32 = 32 |
| 40 | tables_offset | u64, 8-byte aligned and >= 64 |
| 48 | self_phys | u64, 8-byte aligned, checked full extent |
| 56 | reserved | u64 = 0 |

The descriptor array fits the declared blob. Every payload is nonempty, 8-byte
aligned, within the blob and disjoint from the header, descriptor array and all
other payloads. Descriptor kinds are unique. Descriptors are 32 bytes:

| Offset | Field | Representation |
|---|---|---|
| 0 / 4 / 6 | kind / revision / flags | u32 / u16 / u16 |
| 8 / 16 | offset / length | u64 / u64 |
| 24 / 28 | entry_bytes / reserved | nonzero u32 / zero u32 |

Flag bit 0 means required; other bits fail. Payload length must be divisible by
entry size. Only kind 1, revision 1 is interpreted in P00 and it must be required.
Unknown optional kinds/revisions are skipped after validation; unknown required
ones fail. Kinds 2..8 are reserved for modules, mappings, framebuffer, roots,
command line, entropy and build identity. P02 defines modules/mappings/framebuffer;
P03 adds the copied platform summary. A valid P00 memory envelope is not sufficient
for kernel boot.

The required memory table has 40-byte entries and at most 16,384 entries:

| Offset | Field | Representation |
|---|---|---|
| 0 / 8 | physical start / byte length | u64 / u64; nonzero page-aligned length |
| 16 | original firmware attributes | u64 preserved |
| 24 | original firmware type | u32 preserved |
| 28 | Door ownership | u32 checked against the table below |
| 32 | reserved | zero u64 |

Physical extents are half open, checked for u64 wrap and aligned to 4096 bytes.
The producer normalizes entries to ascending order with no overlap, including
overlaps with identical ownership. Hardware-specific address-width validation is
deferred to the acquired P02/P03 CPU/profile contract; u64 is not a promise that
every representable address is backed by RAM.

| Value | Ownership | Initial allocation allowed |
|---|---|---|
| 0 | UsableAfterHandoff | Only UEFI conventional memory, type 7 |
| 1 | PermanentlyReserved | No |
| 2 | ReclaimableAfterCopy | No; needs an explicit future lifetime transition |
| 3 | KernelModule | No |
| 4 | FramebufferMmio | No |
| 5 | FirmwareRuntime | No |
| 6 | BootData | No |

UEFI runtime code/data (types 5/6) or the runtime attribute (bit 63) requires
FirmwareRuntime ownership. Other original firmware data is retained; further
class-specific conversion is a P02 producer obligation. The whole live BootInfo
extent must fit in one BootData, KernelModule or PermanentlyReserved entry. It
cannot be usable, reclaimable, MMIO or firmware runtime. `is_allocatable` accepts
only a range wholly contained in a UsableAfterHandoff region. It does not perform
allocation, claim rights, or prove that firmware services have exited.

Failures use typed `Error` values for truncation, bad identity/version/size,
reserved bits, limits, overflow, alignment, range/overlap, unsupported required
tables, absent maps, invalid/runtime ownership and live boot-data reservation.
No partial BootInfo view escapes failed validation. Out-of-range memory indexes
return errors. Calls are synchronous, allocation-free and side-effect-free apart
from successful encoding. There is no cancellation or persistence contract.

Caller ownership: keep the input stable and readable for the view's lifetime.
No mutable shared buffer may be presented as an immutable Rust slice without
establishing the mapping/concurrency invariants. Authentication, actual physical
backing, ExitBootServices, reclaim completion and mapping changes are later gates.

Tests live in `crates/boot-protocol/tests/boot_info.rs`. Future producer/consumer
integration must test required modules/mappings, framebuffer arithmetic and the
actual post-firmware entry. Evolve contracts, encoders, consumers and tests together
when adding those tables. P00's memory-only envelope remains readable; it cannot
satisfy `validate_boot`. P02 uses actual producer/consumer execution.

P02 required tables are memory (1), modules (2), mappings (3) and framebuffer (4),
revision 1. `parse` checks each supported record; `validate_boot` requires complete
roles, live reservations, needed mappings and framebuffer identity. P03 additionally
requires `validate_platform()` for kind 5 revision 1, a fixed 1024-byte copied ACPI
summary defined in `platform-v1.md`; its roots stay reserved without being used
as kernel pointers. Tables 7/8 remain future extensions. Kind 6 revision 1 is bounded ASCII command-line data;
the initial producer omits it and gets diagnostic modes from profile-owned fw_cfg.

Module entries are 80 bytes: role u32 (1 kernel, 2 initfs), reserved zero u32,
physical backing u64, page-aligned storage bytes u64, exact file bytes u64, virtual
start/extent u64 each, then SHA-256[32]. Two unique roles are required. Kernel
backing is the retained original ELF file; its high virtual extent describes the
separately loaded segments. Initfs backing and virtual identity are the same.
Nonempty file length fits retained storage; hashes are nonzero content identities.
This supplies no signature or Secure Boot authentication.

Mapping entries are 40 bytes: virtual/physical/length u64, permissions u32 using
ELF R/W/X bits (4, 5 or 6 only), role u32 and reserved zero u64. At most 128 records;
page-aligned checked extents, canonical LA48 virtual ranges, no virtual overlap,
and no writable alias of executable physical backing. Roles: 1 loaded kernel,
2 initfs, 3 BootInfo, 4 stack, 5 page-table pool, 6 transition code, 7 framebuffer,
8 original kernel file. RAM backing stays reserved; only role 7 denotes device
backing outside RAM. Kernel/file/initfs/BootInfo/framebuffer/stack mappings are
checked before use; the kernel verifies actual PTEs against declared mappings.
P04 adds role 9 secondary-startup code and role 10 its adjacent data page. Both
arrive as reserved 4 KiB RW/NX identity mappings below 1 MiB. Kind 9 revision 1
contains their fixed 64-byte backing record; `startup-v1.md` defines exact bounds,
alias rejection and the kernel's later code-page RX transition. Kind 6 retains
its command-line meaning. `validate_boot` validates startup metadata when present;
P04 explicitly requires `validate_startup`, while historical handoffs may omit it.

Framebuffer entries are 40 bytes: physical base and byte extent u64, width/height/
stride/format u32, virtual base u64. Format 0 is byte RGB, 1 byte BGR (32-bit pixels).
The current profile uses identity-mapped page-aligned backing. Width/height <=4096,
stride >=width and <=8192, byte extent <=512 MiB; checked stride*height*4 fits backing.
Unsupported bitmask/Blt-only formats fail before drawing. The fixed table pool may
reject a valid but oversized mapping request; actual acquired GOP mode is recorded
through the preserved guest framebuffer/screens.

Extension encoders validate in scratch records before committing output, preserving
the P00 no-write-on-error guarantee. The entry mapping/lifetime, interrupt/stack and
post-exit ownership contract is `kernel-entry-v1.md`. No loader frames are reclaimed
in P02. P03 validates copied platform roots and introduces physical allocation,
heap and local mapping retirement; a valid blob alone does not prove those operations.
The final map still retains loader, boot-service, ACPI and runtime pages. Entropy
initialization and cryptographic readiness are not supplied by the platform table.

