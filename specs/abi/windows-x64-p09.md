# Windows x64 initial execution contract

Status: implemented P09 contract. The phase ledger records the current cumulative
qualification and critic acceptance; the evidence below also preserves development runs.
Master sections 30–36, 102, 107–108 and 136 define the work. This contract records
the deliberately bounded first executable profile; P10 owns its expansion.

## Ownership and loading

The kernel assigns `Personality::WindowsX64` from the trusted Windows manifest6,
with the ordinary authenticated user/session identity, no device capabilities and
one thread. Modifying startup bytes or a syscall number cannot change personality.
The native executive owns VM, handles, threads, cancellation and retirement.
Compatibility structures are user-visible views, never authorization sources.

`compat/pe` plans an immutable initial bundle of at most four PE32+ AMD64 modules.
Module0 is the EXE. Original pinned `ntdll.dll` and `kernel32.dll` are dependencies
from authenticated initfs, with no disk-wide search or replacement by user paths.
File offsets, RVAs, image bases and user addresses have separate types. All image
ranges, imports, forwarders, relocations and final protections must validate before
publication. Complete image reservations include inaccessible holes. At most24
image runs leave room for the native stack, startup/FS and Windows bootstrap/state.
The runtime constructs RW/NX pages, copies bytes/zero fill, applies bounded fixups,
then enforces final protections. Failure tears down all unstarted resources.

The EXE is independently compiled with MSVC and the installed Windows SDK, against
ordinary Kernel32 imports. Its machine code is never patched to enter Door. The
entire PE file SHA256 must match in the Windows reference, initfs and guest record.
The EXE deliberately contains an absolute pointer; placement away from its preferred
base requires a real DIR64 adjustment. Both original DLLs use `/NOENTRY`: no DLL
initializers or dynamic load/unload semantics are claimed in this phase. TLS,
delay loading, CLR and unsupported security requirements fail explicitly.

## Entry and public views

Windows functions use the Windows x64 calling convention, including register
arguments, shadow space, stack alignment and nonvolatile preservation. The first
kernel return enters original ntdll `DoorProcessBootstrap`, with RCX pointing to
the read-only private record below. That bootstrap connects to the authenticated
console service, then invokes the validated EXE PE entry. It is not a `main` wrapper.
Returning from the EXE entry terminates with its32-bit result; ExitProcess terminates
through the native executive and the existing wait/cleanup path.

| Region | Address | Initial access |
| --- | --- | --- |
| Native StartupV2 | `0x71000000` | read only, NX |
| Native private FS/TLS | `0x71002000` | read/write, NX |
| Windows bootstrap | `0x71004000` | read only, NX |
| Windows state16KiB | `0x71008000` | read/write, NX |
| TEB / user GS | `0x71008000` | inside state |
| PEB | `0x7100a000` | inside state |
| Process parameters | `0x7100b000` | inside state |

The bootstrap is256bytes, little endian: `DOORWIN1` at0, version1u32 at8,
length256u32 at12; sixu64 values at16 (native startup, EXE entry/base, TEB, PEB,
parameters); module countu32 at64; reserved zero68..72; four16byte base/entry
records at72; remaining136..256 zero. Module records contain DLLs in dependency
order, exclude the EXE, and have entry0 under the initial `/NOENTRY` policy.
All used addresses must be canonical user addresses; unused entries are zero.

Only acquired public SDK views are populated: NT_TIB stack bounds/Self, TEB PEB
pointer, PEB process parameters/session ID, and length-counted UTF16 image path and
command line. Layouts are pinned to SDK10.0.19041.0 x64, not asserted universal.
Strings allow127 UTF16 code units plus terminator, with byte-counted lengths;
unpaired code units are preserved. Loader lists and full Windows TLS are P10 work.
LastError is per-thread private FS offset512; it does not invent a reserved Windows
TEB offset. Native FS, Windows GS and trusted kernel GS remain separate.

## Implemented export contracts

All integer handles here are scoped translation entries. The console server and
kernel still enforce authenticated peer identity and session revocation. The
initial standard entries are input`0x100`, output`0x104` and error`0x108`.
Input acquisition exists, but ReadFile/input APIs are not exported in P09.

| Kernel32 export | Contract and failure behavior |
| --- | --- |
| GetStdHandle(DWORD) | Accepts the32-bit selectors -10/-11/-12, returns the live standard adapter. Invalid selector returns INVALID_HANDLE_VALUE and error6, as measured on the pinned reference. Success preserves LastError. |
| WriteFile(HANDLE,buffer,DWORD,count,overlapped) | Synchronous ASCII console output, at most1024bytes; writable nonnull DWORD count required and zeroed before work. Captures the full input before IPC. Invalid handle returns FALSE/error6; invalid input range FALSE/error1784 in the pinned reference scenario; invalid count range FALSE/error998. Nonnull overlapped is unsupported/error50. Zero-byte input need not be readable. Success writes actual byte count, returns TRUE and preserves LastError. |
| GetLastError / SetLastError | Read/write the calling thread's32-bit error slot without service effects. |
| ExitProcess(UINT) | Nonreturning native process termination; exit is waitable and resources retire through the existing executive. The initial DLLs have no detach callbacks. |

Non-ASCII WriteFile input explicitly fails with1113 until code-page and Unicode
console behavior are implemented. Input larger than1024bytes fails with87. Service
failure returns1117; earlier accepted chunks may already be visible, count remains0
on Win32 failure. No file, pipe, overlapped, CRT or general console claim follows.

Original ntdll exports `NtWriteFile` with the nine-argument NT ABI and16byte
IO_STATUS_BLOCK; only synchronous standard output/error, null event/APC/context,
null offset/key are accepted. It snapshots input before service IPC and records
actual completed bytes. `NtWriteVirtualMemory` supports only current-process
pseudohandle`-1`, bounded1024byte checked copying and optional eight-byte count.
Remote handles fail. `NtTerminateProcess` supports only self termination.
`RtlGetLastWin32Error`, `RtlSetLastWin32Error` and `RtlNtStatusToDosError` implement
the initial scalar translation set; unknown status maps to317, never success.
Private `DoorGetStandardHandle` and `DoorProcessBootstrap` coordinate original
DLLs/kernel; the independent EXE does not import them.

The native USER_COPY operation`0x213` takes destination/source/byte count, requires
remaining arguments zero, bounds count to1024 and validates both entire ranges
under one VM guard. It snapshots before writing, permitting overlap, and releases
the guard before returning. DLL code never forms a Rust slice from an unchecked
application address. P09's enforced single thread keeps validated output memory
stable during blocking IPC; P10 must replace this assumption before adding threads.

## Compiler runtime and unsupported unwinding

Original ntdll provides bounded compiler memory primitives memcpy/memmove/memset/
memcmp with volatile loops preventing self-recursive compiler substitution.
`_fltused` is a linkage marker. The pinned Windows core library references
`__CxxFrameHandler3`; the original implementation always terminates the process.
It is failure containment, not C++ exception handling. Kernel user faults remain
contained faults. Structurally checked unwind metadata does not qualify Windows
exception dispatch or permit foreign unwinds across Rust frames. Those require P10.

## Evidence obligations

The independent reference EXE runs on local Windows10Pro10.0.19045 x64. The first
passing reference report is `reports/runs/host-1792063270621924600-16208/report.json`:
two matching4096byte builds, SHA256
`798ea38509aa798c608479c90e85f037bf71f64be0aff03ddde351cd4697f6c2`, exact output
`WINDOWS_X64_OK relocation=1 invalid_handle=1 invalid_buffer=1 last_error=1` plus
CRLF, exit73. Invalid input buffer was measured as1784 with redirected stdout;
the initial998 assumption failed and its raw evidence remains retained.

The strengthened current reference at `host-1792064679197219400-15756` adds an
independent MASM Win64 caller which seeds/checks every nonvolatile GPR and XMM6–15,
tests stack entry/shadow space, and calls the ordinary APIs. C adds zero-byte output,
invalid selector6 and success LastError preservation. Two5120byte builds match SHA256
`624bdf13b479601ceb5c3691d52cb2cd434e6b13c907ecf7327710895da81296`;
the output appends ` abi=1 zero=1 selector=1` before CRLF, and exit73 follows the
final register comparison. Exact compiler/assembler/header identities accompany it.
The earlier reference remains historical evidence, not the selected final fixture.

Original ntdll additionally probes overlapping checked copies, invalid source and
partially mapped destination atomicity, size/reserved-argument limits, denied native
thread/heap/raw-console operations and GS/FS preservation across service IPC/yield.
It emits one PID-attributed native diagnostic receipt only after those checks pass.
This private adapter probe does not modify the independent EXE or its API boundary.

Actual Door CPL3 execution must reproduce the selected bytes/exit and report the same
hash, nonpreferred mapping, original DLL bindings, denied raw authority and final
resource reclamation. Host tests cover malformed images/fixups/metadata, dependency
failures and limits. Actual construction checkpoints must prove rollback. The
persisted P08 disk is reopened and independently audited; all earlier39 guest
scenarios and both interactive suites remain required. Only a complete cumulative
gate and independent critic can close P09.
