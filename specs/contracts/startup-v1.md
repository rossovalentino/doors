# Door secondary-CPU startup backing v1

P04 allocates BootInfo kind `9`, revision `1`, one required 64-byte record. Kind 6
remains command-line data and is not repurposed. The portable `Startup` value has
`code_physical` and `data_physical`; `BootInfo::validate_startup()` returns this
owned summary only after validating backing and initial mappings.

| Offset | Encoding | Meaning |
|---|---|---|
| 0 | u64 | Startup code physical page |
| 8 | u64 | Adjacent startup data physical page |
| 16 / 24 | u64 / u64 | Code/data extents, each exactly 4096 |
| 32 / 40 | u64 / u64 | Initial virtual addresses, equal to physical addresses |
| 48 / 56 | u64 / u64 | Reserved flags and padding, both zero |

All fields are little endian. Both pages are 4 KiB aligned, the code address is
nonzero, data immediately follows code, and the whole pair ends at or below
physical `0x100000`. The loader reserves two contiguous pages using UEFI
AllocatePages with AllocateMaxAddress `0xfffff` and memory type LOADER_DATA.
The maximum constrains the allocation's final byte. It validates the returned
extent before zeroing its exclusively allocated pages. Allocation failure fails
loader initialization; it never overwrites an assumed fixed address such as 0x8000.

The final BootInfo memory map must classify each entire page as original firmware
type 2 with Door BootData ownership. Mapping role 9 is startup code and role 10 is
startup data: exactly one 4 KiB identity mapping of each with RW/NX permissions at
handoff. Other declared mappings cannot alias either physical page or overlap its
virtual range. Startup pages cannot overlap the live BootInfo blob. The shared
`validate_boot()` checks this extension when present; P04 additionally requires
its presence through `validate_startup()`. Old P02/P03 handoffs without the new
record remain structurally readable by their original validators.

The pages are allocated even for a one-CPU boot, so the handoff has one stable
schema across qualified P04 profiles. Both pages remain reserved for the boot
lifetime and never enter the physical-frame free pool. Firmware protocols and
allocation handles do not cross the boundary; only checked integer addresses do.

Kernel architecture code owns the trampoline bytes and the data-page mailbox/GDT
layout. It copies the trampoline while the code mapping is RW/NX, verifies its
size fits one page, installs the per-CPU mailbox data, then asks the exclusive
page-table owner to transition only the code page to RX and retire stale local
translations before sending INIT/SIPI. The data page remains RW/NX. The SIPI vector
is the checked code physical address divided by 4096. No writable executable
mapping or mutable alias is introduced. Initial BootInfo mapping records describe
handoff state; subsequent protected transitions belong to the kernel mapper.

AP startup must use observed per-CPU readiness, bounded waits and separate mailbox
ownership for delayed processors. Requested CPU count does not establish online
count. AP code, shared data and the active page-table root stay alive throughout
startup, failure and any later AP use. Kernel startup/thread/remote-TLB contracts
and actual SMP tests establish those later behaviors; reserving pages alone does
not qualify them.

Authority: acquired UEFI boot-services release `82804ed40f173589b372fa303d1e69956dcfa153`,
AllocatePages/AllocateMaxAddress (source ledger `UEFI-BOOT-RELEASE`), and Intel SDM
325384-081US, September 2023, sections 9.4.3 and 9.4.4.1 (`INTEL-SDM-3-081`). Intel
specifies a startup code page within the lower MiB and the SIPI vector's page
address semantics. Door's 64-byte record, reserved ownership and page roles are
original project contracts. Tests cover truncation, high/unaligned/wrapping and
overlapping backing, changed ownership, missing/duplicate/aliased mappings,
incorrect initial permissions, reserved bytes and descriptor revisions/sizes.
