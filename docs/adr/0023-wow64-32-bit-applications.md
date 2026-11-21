# ADR 0023: WOW64 — 32-bit Windows applications on real 32-bit DLL builds

Status: adopted 11 November 2026 (P17 M9); amended the same day after the
P17 critic review (items 1, 2 and 3, and the limits). Plan and progress:
`docs/design/p17-plan-20261109.md`.

## Context

P17 asks for 32-bit (x86) Windows programs: PE32 images, the x86 calling
conventions, SEH through `fs:[0]`, TEB32/PEB32, and the WOW64 views
Windows gives such a process (System32 → SysWOW64, WOW6432Node, the
(x86) Program Files folders and environment, IsWow64Process). Windows
itself runs a 32-bit process on genuine 32-bit builds of ntdll, kernel32,
user32 and the rest, switching processor mode only at the system call
boundary (wow64cpu). The alternative, a thunk layer that translates every
32-bit API call into the 64-bit DLLs, needs a hand-written marshaller for
every structure, every callback in the other direction (window procedures,
COM vtables, hooks, thread starts) and every pointer an API returns.

## Decision

1. **The compatibility DLLs build twice from the same sources**: for
   `x86_64-pc-windows-msvc` and for `i686-pc-windows-msvc` (stdcall
   exports, staged at `windows32/<name>.dll`, 16 MiB catalog slots from
   0x0200_0000). All 21 compatibility DLLs and msvcrt.dll do (`NAMES32`);
   the catalog's remaining slot is the P10 test DLL. There are no
   reverse thunks: window procedures, COM vtables, hooks, thread starts,
   TLS callbacks and SEH handlers stay 32-bit end to end.
2. **One mode switch**: `door_sys::raw` on i686 widens its arguments into
   64-bit words and far-calls the 64-bit user code segment (0x33) to
   execute `syscall`, then far-returns. Kernel ABI structures are
   fixed-width, so a pointer below 2 GB means the same on both sides. The
   kernel gives a 32-bit process GDT selector 0x3B (32-bit user code), its
   images below 2 GB (the loader refuses higher bases; executables too
   large for the space below the DLL slots go to 0x2000_0000), FS = its
   TEB32 (one page per thread, as Windows'), and delivers faults as
   EXCEPTION_RECORD32/CONTEXT32 to 32-bit ntdll's SEH dispatcher. Every
   return to user mode reloads DS, ES and FS with 0x2B, so no thread's
   selectors reach another. CSTAR (syscall from compatibility mode) is an
   invalid-opcode fault; SYSENTER faults (its MSRs are cleared).
3. **Pointer width is explicit**: Windows-facing exports take `usize`,
   widened at entry with sign extension (every address of a 32-bit Doors
   process is below 2 GB; -1/-2 pseudo-handles keep their meaning) and
   narrowed on return; `Door*` private exports keep 64-bit parameters.
   Structures in program memory are described once as field kinds
   (`door_windows_abi::layout`) and laid out as MSVC lays them out for the
   build's width and packing. Handle values the DLLs make themselves are
   32-bit-safe (`door_windows_abi::tagged`): below 2 GB in a 32-bit
   process, clear of kernel handle values (from 0x1_0000 by 4). Unsigned
   pointer-width values (SIZE_T, WPARAM) are sign-extended like pointers;
   values of 2 GB and more therefore differ from Windows' WOW64, which
   zero-extends them (recorded limit).
4. **WOW64's views live in the 32-bit DLLs** (`door_windows_abi::wow64`),
   with SysWOW64 holding the programs a 32-bit setup starts by name
   (msiexec.exe, since 11 November 2026 an i686 build of the same program
   over the 32-bit msi.dll: it installs 32-bit packages, ADR 0022 item 5):
   32-bit ntdll redirects System32 to SysWOW64 (Sysnative and the exempt
   folders as on Windows; per-thread switch in TEB32 0x3F8), 32-bit
   advapi32 maps HKLM\SOFTWARE to WOW6432Node with Windows' shared-key and
   Classes lists, the process environment carries the x86 values, shell32
   answers the (x86) known folders. PROCESS_MACHINE tells IsWow64Process
   what a process handle runs.

## Consequences

- One source tree serves both widths; every DLL change is checked for both
  targets (`verify --phase P17` lints the 21 DLLs on i686) and 32-bit
  programs exercise the same code paths as 64-bit ones (X86-A, X86-B,
  X86-C compare MSVC x86 probes with Windows 11's own output).
- Width bugs are ordinary bugs in shared code: a structure read at the
  wrong width or a handle value above 2 GB affects only 32-bit programs and
  is caught by the x86 probes, not by the x64 suites.
- Not covered yet (recorded in the plan): SafeSEH enforcement, SEHOP,
  nested exception frames, C++ exceptions (32-bit msvcrt.dll's
  `__CxxFrameHandler3` is a stub that continues the search); FS holds
  the 0x2B data selector with its base at the TEB32, where Windows uses a
  dedicated 0x53 selector, so `mov ax, fs` reads 0x2B while CONTEXT32's
  SegFs reports Windows' 0x53, and a program reloading FS itself loses the
  TEB32 base; PEB32 ProcessParameters (the
  32-bit layout) fill only the image path, command line and environment;
  SysWOW64 holds no DLL files; the 32-bit registry view's
  enumeration omits shared keys; oleaut32's universal marshaler
  (type-library proxies and stubs) is x64-only (LocalServer32 activation
  crosses bitness: a client tries its own registry view, then the other);
  breakpoints, x87 and
  SIMD faults and general-protection faults reach SEH (a #GP as an access
  violation or, for HLT, CLI, STI, IN and OUT, a privileged instruction);
  debug, alignment-check and machine-check faults end the process.
