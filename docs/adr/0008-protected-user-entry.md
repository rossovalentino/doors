# ADR0008: Protected user entry and bounded memory transfer

Status: implemented for P05 integration; actual guest qualification remains in
the selected phase report. This extends ADR 0007's P04 context format.

Every saved context is now 560 bytes aligned to 16. After the 512-byte FXSAVE area,
it contains the trap-frame pointer, native FS base, original CR3, owned kernel
stack top, GS-domain restoration flag and actual user GS base. The 176-byte GPR
and hardware frame stays unchanged. Process roots share supervisor-only high
kernel mappings; their lower halves are independently owned. Entry saves the
original CR3 and loads the boot kernel CR3 before any Rust handler. The epilogue
loads the selected root after switching stacks and committing retirement. PCID
and global user mappings are unsupported; CR3 reloads discard private translations.

Each CPU owns a high-half GS entry record and descriptors. Native TLS uses FS.
User GS starts at zero and is saved independently so migration does not borrow
another thread's future compatibility TLS. FSGSBASE is disabled. Interrupt entry
checks the actual GS-base domain after saving registers, including the interval
where a nonmaskable exception can observe kernel CS on either side of SWAPGS.
The recorded domain is restored during the epilogue. NMI and machine-check policy
remains fatal; this bookkeeping is not evidence of NMI stress qualification.

The GDT adds DPL3 data/code selectors 0x2b/0x33. Before a selected user context runs,
the post-stack-switch hook updates both the CPU's trusted syscall stack and TSS
RSP0. SYSCALL enables the documented STAR/LSTAR/FMASK path, acquires its stack
through GS before touching stack memory, and builds the same normalized trap
frame as an interrupt. RAX and the six Door ABI argument registers pass unchanged
to the executive. RCX/R11 retain their architectural clobber semantics. Maskable
IRQs stay disabled during the bounded syscall dispatch; scheduling operations
return a selected context through the ordinary retirement protocol.

Returns always use IRETQ. User selectors, low-half RIP/RSP/FS/GS, supported flags,
aligned private CR3 and trusted kernel-stack bounds are checked. Invalid caller
return state is a contained process fault with synthetic vector 0x101. Invalid
kernel-owned selected state during an already pending scheduler handoff is a
kernel invariant failure. The process VM owner independently validates mapping
permissions and lifetime before publishing initial contexts. Hardware user faults
are dispatched to the executive's termination hook, which must select a new
context rather than resume the shared exception IST. Kernel faults retain the
precise allocation-free fatal path.

User copying accepts a trusted kernel slice and an integer user address, never a
Rust reference into user memory. The VM owner must hold its lock or pins across
effective U/S and read/write validation and the copy. A transfer is bounded to
64 KiB, with overflow and lower-half checks. IRQs remain disabled while a CPU-local
recovery record is active. A short assembly routine enters the process root and
uses REP MOVSB. Only #PF/#GP at that exact instruction is recoverable; #PF must
also address the declared user extent. Recovery returns the precise copied prefix
and restores the original root. A fault in a kernel buffer or unrelated kernel
instruction remains fatal. Partial output is explicit and is not rolled back.

FXSAVE remains the full enabled extended-state contract: x87/MMX/SSE2, including
MXCSR and all sixteen XMM registers. OSXSAVE, FSGSBASE, PKU and CET are disabled;
there is no unsupported extended state silently omitted from context switching.
Existing P03 trap and P04 preemption/SMP fixtures remain required regressions.

The implementation is original Door code against Intel SDM Volume 3,
325384-081US, September 2023, particularly sections 3.4.4, 5.8.8, 6.14, 8.7 and 14.1.
The exact previously acquired PDF and its hash remain recorded as
INTEL-SDM-3-081. Native operation selectors, status values and payload semantics
belong to the separately versioned Door ABI contract.
