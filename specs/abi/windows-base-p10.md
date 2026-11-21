# Windows base semantics — P10

The bounded Windows base profile includes ordinary-session integration.
Current qualification and critic acceptance are
recorded in project/phases.json and the exact reports it references.
File-supplied PE application admission, terminal input and durable ordinary writes
extend this profile under `specs/contracts/ordinary-session-v1.md`. Trusted system
providers remain original project DLLs. The Tiny Basic trial is explicitly a source
port with unchanged upstream interpreter and original narrow platform glue; it
does not qualify arbitrary distributed Windows applications.
Requirements: master sections30–36,102,104,107–109,135–136,139 and phase91.

## Ownership and supported profile

The portable `compat/pe` planner validates images, dependency order, fixups, TLS
and unwind metadata. Kernel process/VM/executive objects own mappings, identity,
resources and retirement. Original Ntdll/Kernel32 translate Windows entry points;
original Msvcrt supplies the scoped C runtime. Storage and console APIs use the
existing authenticated native services and shared durable namespace. Public
PEB/TEB bytes and user module names never grant native authority.

P09 retains its original four-module/24-run planner limit, one thread and no
heap. P10 permits five catalog modules/40 runs, four live threads and1MiB dynamic
commitment. ProcessSpace has64 bounded regions. The P10 catalog contains the EXE,
pinned Ntdll/Kernel32/Msvcrt and one application DLL, lifecycle.dll. Arbitrary
filesystem DLL search, replacement of system providers and unknown API contracts
are rejected. `api_set_host` defines nine explicit core contracts backed by
Kernel32; each requested export must still exist. Paths are bounded wide names.

## Module lifetime and TLS

The application DLL is absent from startup mappings. Startup import and forwarder
dependencies on that deferred module are rejected before process publication. LoadLibraryW maps/copies/
fixes/protects a validated immutable image, installs compiler TLS and invokes TLS
callbacks and DllMain outside kernel/VM/metadata locks. Duplicate loads increment
references. Final FreeLibrary detaches and synchronously retires private backing;
reload starts with fresh image and TLS bytes. System providers remain pinned.
Kernel allocation-budget probes fail each dynamic mapping/fixup/protection step,
then prove unchanged physical ownership, absent mappings and cleared TLS state.
FALSE DllMain initialization receives detach and rolls back. Callback exceptions
are contained process faults. Reentrant loads fail explicitly; concurrent loads,
unload and thread notifications serialize through the callback gate. Applications
retain module references while using exported functions or queued APC code.

Compiler TLS captures relocated mapped template bytes, including pointer initializers,
plus zero-fill up to256 bytes/module,
alignment through16 and eight validated RX callbacks. The kernel writes the TLS
index and supplies every live/new thread's private template and GS TLS vector,
including threads already present at late load. Templates and vector cells are
cleared on unload. Native FS storage remains separate from Windows GS/TEB and
trusted kernel GS. Dynamic TlsAlloc/Free/Get/Set supplies64 epoch-tagged slots;
index reuse cannot expose a previous generation's value. LastError and CRT errno
are private per-thread fields. Notification order is tested on Windows and Door.

## Threads, memory and exceptions

CreateThread uses native owned contexts, shadow space, private TEB/TLS and stable
waitable exit records. Initial scope accepts default stack, NULL security/TID
output and zero creation flags. Thread exit notifies modules; orderly process exit
elects a native owner, stops and retires siblings before process detach. Direct
termination skips orderly callbacks. Native retirement owns stack/frame reclamation.

VirtualAlloc/Free/Protect implement reserve, commit, repeated commit, decommit,
zero-filled recommit, whole release and whole-reservation protection. Initial
private mappings support READONLY/READWRITE; partial ranges, executable/guard/
large-page policies and committed NOACCESS fail explicitly. Reserved extents have
no frames. Commitment remains in the manifest budget and native VM records.

Vectored handlers receive SDK19041 x64 exception/context layouts for actual user
faults. NtContinue validates integer/control state and current RX/stack mappings;
trusted kernel state and the original enabled FX state remain kernel-owned.
RtlLookupFunctionEntry scans bounded, sorted/nonoverlapping mapped .pdata records.
RtlVirtualUnwind uses transactional v1 no-handler body/prologue/terminal-RET caller
recovery; malformed metadata cannot appear as successful leaf recovery. Unsupported
epilogues, handlers/chains, dynamic function tables, context-pointer output and
C++/language unwinding remain explicit failures. No foreign unwind enters native
Rust or kernel frames. Complete1232-byte CONTEXT captures/output use one native
VM-locked USER_COPY; only trusted manifest7 permits up to4096 bytes. P09/native
callers retain the1024-byte bound. Over-limit and partly writable copies are tested.

## Files, waits, async and CRT

CreateFileW carries lossless bounded UTF-16 to storage with authenticated UID,
creation/access/sharing/ACL results and generation-safe file handles. Native and
Windows operations share one journal/namespace owner. Relative paths begin in the
provisioned authenticated user directory. Synchronous transfers use owned buffers
through IPC; EOF, denied access, partial counts, flush, close and stale handles have
real side effects and errors. Initial limits are eight files,95 name units and
1024-byte transfers. Unicode file bytes and owner/ACL state are audited independently.

Events have native authority and manual/auto-reset semantics. Single-object and
wait-any multiple waits support event/thread records; alertable waits and SleepEx
deliver bounded FIFO APCs on the target thread. Polling granularity is10ms. Wait-all,
named events, file-handle waits and additional synchronization objects are outside
this profile. Details and failure contracts: `windows-io-waits-p10.md`.

One owned pending OVERLAPPED request retains its event and file lifetime, captures
write bytes before submission and publishes exactly one terminal completion.
Cancellation uses nonwrapping request generations. Close marks the file closing
before draining its worker; independent completion observers retain WAIT handles
before joining outside metadata locks. Reads, cancellation/completion races,
closed caller event handles, repeated queries and concurrent observers are tested.
No application buffer reference is borrowed across IPC. Invalid/unmapped completion
buffers fail through checked native copies. General IOCP/APC I/O completion and
unbounded queue depth are not claimed.

Original Msvcrt exports malloc/calloc/realloc/free, memcpy/memmove/memset/memcmp,
strlen/wcslen, abs and per-thread errno. Eight owned allocations use native private
commitment with checked rounding/overflow and failure-preserving realloc. Scoped
memory/string/heap behavior is tested through ordinary C ABI imports; this does not
qualify full UCRT, C++ runtime, stdio, locale or exception support. Installed Windows
DLL implementation bytes are never copied into Door images.

## Qualification

The independent SDK C/MASM EXE and DLL run unchanged on Windows and Door. Fixtures
check failed attach, duplicate references, unload/reload, API contracts, missing
modules/exports, late TLS allocation, callback order, concurrent references,
LastError/errno isolation, multi-threaded processing, allocation failure, actual
RO faults, caller recovery/continuation, event/APC behavior and async lifetime.
Windows reference and Door save/reopen independently verify the1024-byte Unicode
file result and preserve every P08 file/setting/registry value. Reopen writes zero
blocks. Combined development image host-1792174824725805700-14604 passes Door save
host-1792174865715473100-15160 and reopen host-1792174922197741800-20364, with38
threads retired, zero leaks, eight DMA pages reclaimed and34 dynamic rollback steps.

`cargo xtask verify` selects the real cumulative P00–P10 gate: all42 guest cases,
both interactive checks, all host/target checks, two reproducible16-artifact builds,
original P09 unchanged-binary/disk comparison, P10 exact source/command/header/
module/reference/disk bindings and independent audit. Final closure requires that
gate plus an independent critic bound to its report, resolved findings and updated
records. Finite tests do not guarantee absence of every bug. P11 remains unstarted.

Primary contracts: Microsoft PE/TLS specification and the acquired x64/SDK profile;
https://learn.microsoft.com/en-us/windows/win32/api/winnt/nf-winnt-rtlvirtualunwind
and https://learn.microsoft.com/en-us/cpp/build/exception-handling-x64 .
