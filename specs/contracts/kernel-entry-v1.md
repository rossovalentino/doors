# Door x64 kernel entry v1

Owner: `boot/uefi` producer and `kernel` consumer. Sources: MASTER-001 sections
132–133, UEFI-BOOT-RELEASE, UEFI-RS-041, Rust's UEFI/none target documentation.
This entry convention is original Door infrastructure, separate from Windows ABI.

Firmware PE uses `efiapi` through the pinned `uefi` entry wrapper. Native kernel
is a fixed ET_EXEC ELF64 linked at `0xffffffff80000000`, with page-separated RX text,
R/NX data and RW/NX data/BSS. The loader validates program headers, caps the initial
profile at eight segments/32 MiB mapped span, allocates distinct backing, copies
file-backed bytes and starts with zeroed pages for BSS. ELF physical-address fields
do not dictate allocations. The original file remains in a separate reserved RO mapping.

Firmware file/protocol guards drop before `exit_boot_services`. Pinned support
code obtains a final map, attempts exit with its current key and retries once on
failure using the existing map buffer; no filesystem/GOP/console operation occurs
between final acquisition and exit. Unrecoverable wrapper errors reset rather than
claim handoff. The map is sorted/translated into BootInfo after successful exit;
its owner performs no firmware free. No post-exit firmware allocator or protocol is used.

Allocations are firmware-owned LOADER_DATA pages below 4 GiB. Final memory descriptors
retain original type/attributes: conventional is usable, loader pages remain BootData,
boot-service pages remain reclaimable-after-copy, ACPI is permanently reserved and
runtime stays reserved. P02/P03 perform no firmware or loader reclamation. Kernel
image/initfs hashes identify file contents, without Secure Boot or authentication.
P03 allocation starts only from transferred conventional frames.

The narrow transfer stub disables interrupts, sets NXE and CR0.WP, installs owned
four-level page tables, selects the independent 128 KiB stack and jumps to ELF entry.
RDI carries the BootInfo mapping; RSI carries its trusted mapped byte extent. RSP is
16-byte aligned before the kernel stub calls ordinary SysV Rust. The entry clears DF
and frame pointer and is nonreturning. Guard pages on both stack ends are absent.
BootInfo, raw images, stack, table pool and framebuffer have explicit mappings.
Only the transfer code's two pages retain a loader executable identity mapping.
No writable executable leaf or writable alias of executable physical backing is allowed.
The kernel checks every declared mapping against live CR3/PTE permissions before use.

Firmware descriptor structures are not passed as borrowed firmware protocols. GOP's
physical framebuffer is recorded and mapped with device cache flags. The explicit
QEMU RAMFB development profile instead uses loader-owned BootData pages with
normal RAM cache attributes; its live role7 mapping remains reserved from the
frame allocator. Firmware GOP retains its uncached policy. Checked stride,
extent/pixel format precede clipped volatile writes. Serial/fw_cfg/PS2 diagnostics
belong to the sole kernel CPU. The entry arrives with interrupts disabled. P03
installs owned GDT/TSS/IDT before parsing external data or allocating. A validated
copied ACPI summary supplies platform data; its roots stay reserved without kernel
mappings. The LAPIC page is mapped uncached RW/NX. Entropy remains absent.

Diagnostic modes are pinned development fixtures through QEMU fw_cfg: normal, panic,
bad-handoff, hang, bad-kernel, bad-initfs and demo. P03 adds trap-ud, trap-double,
fault-readonly, fault-nx and fault-guard. Firmware corrupts only its private
in-memory copy for malformed fixtures. The production boot profile must remove this
fixture surface. Actual kernel records, expected exits and timeout evidence qualify
P02. Demo polls PS2 only for H/D/P/R; it is an early diagnostic console, not userspace.

P03 physical memory uses a 4 KiB bitmap covering addresses below 4 GiB, retaining
the entire low MiB and all non-conventional reservations. At most 128 concurrent
allocation tokens may each own multiple contiguous frames; the bound is not 128
frames. Allocation is fallible, alignment checked and accounted. Tokens carry an
allocator identity and generation, cannot be cloned and require explicit free.
Dropping a token retains ownership instead of reclaiming potentially live backing.

The active four-level manager adopts the reserved512-page table pool and supports
128 concurrent new mappings, each at most65536 pages. It checks live CR3/WP/NXE/PAT,
preserves initial mappings and rejects aliases, overlaps and writable-executable
states. Failed construction rolls back new leaves/tables and returns the original
frame token. Committed RAM is zeroed through RW/NX mappings before final permissions
are exposed. Protection/unmap require exclusive access on the sole online CPU with
IRQs disabled; every removed translation receives INVLPG before its frame token
returns. Successful unmaps retain structural tables. SMP retirement belongs to P04.

Four retained 16 KiB image-backed bootstrap exception stacks make early faults
diagnosable; after memory initialization their TSS pointers are replaced with four
owned 32 KiB stacks, with absent guard pages at both ends. General exceptions,
double fault, NMI and machine check have separate IST selections. Interrupt entry
saves GPRs and enabled x87/MMX/SSE state with FXSAVE64/FXRSTOR64; OSXSAVE is disabled
and AVX is unavailable in this profile. The compiler uses its soft-float ABI.
Normalized trap frames include vector/error, instruction/stack addresses and flags;
fatal diagnostics include CR2, use a nonrecursive emergency path and fail the guest.
User transitions, scheduling and broader SIMD contexts require later phases.

The early heap retains its 512 KiB RW/NX mapping for the boot lifetime. It supports
fallible aligned zeroed allocations; it has no global allocator, realloc or free.
The kernel retains heap, exception-stack and LAPIC mappings with their ownership
tokens. Mapping/frame diagnostics verify that reuse zeroes prior contents.

The timer requires exactly the discovered BSP, checked APIC base/identity, xAPIC
mode and a valid ACPI PM timer. It calibrates a nominal 10 ms LAPIC period against
the 3.579545 MHz PM source with bounded polling. A diagnostic enables real periodic
interrupts, requires at least three deliveries with EOI, then stops the timer and
returns with IRQs disabled. Legacy PIC and LAPIC external inputs are masked. I/O
APIC metadata is retained for later routing; P03 enables no general external device
interrupts or scheduler preemption.

Sources: `INTEL-SDM-3-081`, `ACPI-65-SOFTWARE` and `ACPI-65-HARDWARE` in the source
ledger. Hardware behavior requires the actual P03 guest gate on both RAM profiles.

P04 extends the loader payload with two contiguous, zeroed low-memory pages from
UEFI AllocatePages(MaxAddress 0xfffff), retained as LOADER_DATA/BootData. The new
`Startup` record and mapping roles 9/10 are defined in `startup-v1.md`. Both pages
arrive RW/NX; the kernel owns copying its trampoline and promoting only the code
page to RX before SIPI. The same backing is reserved on one-CPU boots. No firmware
protocol, hard-coded low-memory address or allocator service is used after exit.

P04 retains the initial entry ABI and supersedes the preceding P03 single-CPU
runtime limits. After its local diagnostics, the kernel prepares guarded AP idle
and emergency stacks and starts up to four discovered xAPIC CPUs. Each AP loads
its own descriptors and calibrates its own timer before publishing readiness.
Native threads then enter the IRQ-safe scheduler through timer/yield/reschedule
vectors; the selected context owns GPRs, x87/MMX/SSE2 and FS base. Stack retirement
is acknowledged after assembly changes RSP, before another CPU can dispatch or
reap the outgoing task. Details and wait semantics are in ADRs 0006 and 0007.

Mapping metadata now has 128 bounded slots. Before secondary startup, the sole
page-table writer installs a synchronous retirement hook. Every leaf publication,
protection change, removal and partial table rollback invalidates locally and
waits for all observed online CPUs. Failed acknowledgement does not return frame
ownership. The final 32-round fixture alternates two distinct physical frames at
one virtual address, checking every CPU's read before retiring the old mapping.
After task reaping, guarded task stacks are unmapped and their frames returned;
AP idle/emergency stacks and page-table structures remain retained. Parked APs
stop their timers but continue accepting retirement IPIs. CPU hotplug, process
address spaces, user transitions and general device routing remain later work.
