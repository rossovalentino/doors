# ADR0005: Owned single-CPU descriptors, traps and timer

Status: accepted for P03 implementation; guest qualification is recorded by the
phase report. This ADR does not itself establish a passed guest result.

The first post-handoff initialization owns GDT, TSS and all 256 IDT entries before
frame allocation or page-table mutation. Long-mode code/data descriptors and a
64-bit TSS replace inherited firmware descriptor state. Four retained 16 KiB kernel
BSS stacks cover this early interval. Once mapping support exists, four distinct
32 KiB RW/NX stacks with absent guards replace the TSS pointers. General exceptions,
double fault, NMI and machine check each have an independent IST. P03 never retires
the bootstrap backing. The initial BSS stacks are not themselves guarded.

All gates are ring 0 interrupt gates, so ordinary IRQ nesting is disabled on entry.
Assembly normalizes CPU error-code and non-error-code vectors, saves all 15 GPRs,
captures CR2 before compiler code and retains the hardware RIP/CS/RFLAGS/RSP/SS.
It clears direction flag and aligns a separate 512-byte FXSAVE area before calling
Rust. FXSAVE64/FXRSTOR64 preserve x87, MMX, MXCSR and all 16 XMM registers. The kernel
enables SSE2/OSFXSR, initializes x87/MXCSR and disables OSXSAVE; AVX/XSAVE-only state
is outside this execution profile. Compiler code still requires the no-red-zone
kernel build setting. Later user execution and scheduling must extend the state
contract before enabling additional features. An armed INT3 probe checks actual
return, a GPR/SSE pattern and the exact owned IST frame address.

Unarmed exceptions are fatal kernel faults. Their bounded, allocation-free serial
records include vector, error, RIP, CR2 and interrupted register context. A fatal
guard prevents recursive display/formatting work. Recursive failure writes only a
static UART marker and exits; it cannot be mistaken for a diagnostic pass. Separate
fixtures exercise invalid opcode, page protections, guard access and failed
exception-stack delivery to the double-fault IST. P03 does not yet recover user
faults, unwind foreign ABIs or schedule interrupt work.

The local APIC physical base and enabled CPU membership come from validated,
copied ACPI data. CPUID BSP ID, the APIC ID register and IA32_APIC_BASE must agree.
P03 requires one CPU in xAPIC mode. The mapper verifies the PAT cache interpretation
before exposing the RW/NX uncached device page. Legacy PIC inputs are masked when
the MADT PCAT flag declares them; firmware LVT sources are masked. The owned local
timer uses vector 64 and acknowledges EOI; vector 255 is spurious and does not EOI.
I/O APIC routing and SMP initialization remain later work.

The timer measures an interval against the FADT System-I/O PM timer, whose width
is 24 or 32 bits and clock is 3.579545 MHz. Calibration handles wrap and has both a PM
deadline and an iteration bound, then normalizes the LAPIC count to 10 ms at divisor 16.
The diagnostic starts periodic delivery, observes at least three actual interrupts,
then stops the timer and returns with IRQs disabled. The P03 owner demo therefore
retains exclusive polled input/serial use. P04 will supply the scheduler lifecycle.

Sources used: Intel SDM Volume 3, order 325384-081US, September 2023, sections 6.14
(gates, long-mode stack frame, IRET and IST), 8.7 (TSS), 11.5.4 (APIC timer), 14.1.3 (SSE),
and ACPI 6.5 sections 4.8.2.1/4.8.3.3 (PM timer). The exact official source bytes and
hashes are recorded in the source ledger; no foreign kernel code was adopted.
