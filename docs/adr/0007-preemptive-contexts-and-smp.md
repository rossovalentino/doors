# ADR0007: Preemptive contexts and observed CPU startup

Status: implemented for P04 qualification. Actual runtime outcomes belong in the
phase's immutable guest reports and independent review.

Every interrupt now owns a 528-byte aligned context containing FXSAVE64 state, a
pointer to the normalized 176-byte trap frame and IA32_FS_BASE. Timer, yield and
reschedule vectors may ask the scheduler for a different context. The assembly
epilogue switches RSP first, then calls the retirement commit hook on the selected
stack before restoring state. Until that hook acknowledges the outgoing context,
the scheduler must quarantine it from migration, reuse and destruction. The old
stack is never accessed after switching RSP. The initial-context constructor gives
each new task clean x87/SSE state, a canonical FS base and a SysV entry stack.

All 15 general registers, 16 XMM registers, MXCSR, x87/MMX and FS base survive the
common path. OSXSAVE remains disabled; the enabled state contract is still SSE2.
A dedicated assembly fixture holds distinct thread patterns across actual timer
interrupts, independently checks MMX and x87 phases, and validates FS:0. It has no
cooperative yield and waits for a per-thread counter advanced by timer dispatch.
Host layout tests verify offsets; only real guest preemption qualifies restoration.

Up to four registered CPUs own separate GDT/TSS/IDT storage, exception stacks,
timer calibration, interrupt counters and fatal-path state. CPU identity is read
from CPUID and matched to the validated ACPI APIC membership. No requested vCPU
count is used as evidence that a CPU started.

The loader reserves a contiguous pair below 1 MiB. Kernel preparation copies a
position-independent real/protected/long-mode trampoline into the first page and
places a small temporary GDT and four APIC-ID-keyed mailboxes in the second page.
The mapping owner promotes only code to RX before INIT/SIPI. Mailboxes remain
RW/NX. No code patch occurs after promotion. An AP uses its own mailbox and guarded
boot stack, installs owned descriptors and independently calibrates its timer
before publishing its observed online bit. A bounded startup timeout permanently
parks late arrivals. The missing-AP fixture deliberately suppresses the final
target's IPI and runs the same observed-online timeout path.

IPI command writes preserve local interrupt state and serialize the destination
and command register pair. The synchronous mapping-retirement hook publishes a
generation/address, invalidates locally, sends a dedicated remote IPI and waits
for every observed online CPU's release acknowledgment before returning. The IPI
handler never acquires a scheduler or page-table lock. Callers serialize page-table
mutation and must not wait for a remote CPU while holding a lock that CPU needs
with IRQs disabled. A missing acknowledgment is an error; no frame reuse is then
authorized. Online CPU membership is fixed after startup in this phase.

Source: the previously acquired Intel SDM 325384-081US (September 2023), sections
3.4.4 (FS/GS bases), 4.10 (translation invalidation), 6.14 (interrupt return),
9.4 (INIT/SIPI startup), 11.6 (ICR/IPI delivery) and 14.1 (extended state).
The implementation is original Door code. ACPI PM timer details retain the exact
ACPI 6.5 source recorded for P03. Physical hardware, x2APIC, hotplug and extended
XSAVE/AVX state require later explicit profiles and qualification.
