# Windows compatibility components

The native executive remains authoritative for identity, VM, handles, threads,
I/O and cleanup. These components implement Windows-facing contracts over it.

| Directory | Ownership |
| --- | --- |
| `abi` | Pinned public Windows views, private versioned bootstrap and scalar translation types |
| `pe` | Portable immutable PE validation/dependency/mapping/fixup plans |
| `dlls/ntdll` | Original CPL3 bootstrap, checked pointer capture and native operation adapters |
| `dlls/kernel32` | Thin initial Win32 exports over original ntdll imports |

No DLL executes on the host during normal workspace tests. The reusable xtask
build explicitly selects `guest-pe`, freestanding DLL link flags and isolated
target/import-library directories. Those guest DLLs contain Door syscall traps;
they must not be loaded into ordinary Windows processes. The independent C/MASM
fixture uses Windows SDK imports and runs unchanged on the installed reference.

Current scope and operation contracts are in `specs/abi/windows-x64-p09.md`, with
the ownership decision in `docs/adr/0015-first-windows-process.md`. P09 is an initial
single-thread, static-DLL, no-CRT profile. Source inspection and host tests do not
establish guest execution; the cumulative gate and independent critic do.
