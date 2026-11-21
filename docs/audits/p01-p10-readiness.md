# P01–P10 implementation and usability audit

Date: 2026-10-17. Requested by the owner after using the QEMU console.

This document preserves the audit of the original deliverable. P10 has since
been reopened to repair these findings. Ordinary persistent session, shell/file
operations, captured ELF/PE admission and a Tiny Basic source port now exist in
code. Their cumulative qualification run is
`reports/runs/host-1792235592894575300-22840/report.json`, with unchanged inputs,
363 host tests, 42 diagnostic guest scenarios and the ordinary-session suite.
The [application inventory](../testing/application-trials.md) records the measured
workflows and remaining boundaries. See `project/STATUS.md`,
`specs/contracts/ordinary-session-v1.md` and `docs/testing/ordinary-session.md`.
The original verdict and observations below remain historical evidence, not a
description of every subsequent implementation change.

## Verdict

Door is a real experimental operating system executing on QEMU's emulated
hardware. Its loader, kernel, memory protection, context switches, user processes
and disk operations execute in the guest. Those mechanisms are not a host program
printing simulated outcomes.

The current deliverable is nevertheless a test-oriented OS prototype, not a usable
general-purpose OS or a generally Windows-compatible application platform. Most
functionality is reached through predetermined qualification scenarios. The visible
console is an interactive fixture. The previous phase-closure statements must be
read as acceptance of documented engineering subsets, not product readiness.

Resource limits alone do not make an OS a simulation. The more consequential
limitations here are fixed executable catalogs, test-driven boot/session selection,
fixture-only user interaction, and lack of ordinary application qualification.

## Method and evidence

This audit inspected the actual firmware handoff, kernel boot dispatch, scheduling
and user-entry code, process catalogs, storage driver/service, console application,
Windows startup policy, contracts, phase requirements and final qualification
reports. It did not infer usability from test totals or undertake a complete
security review. No implementation source or historical acceptance report changed.

The prior cumulative report is
`reports/runs/host-1792214999674635300-12500/report.json`, SHA256
`e8d8ce9cc07e83c3d5bde11385d5f8be3bc970ef35e52741cfe42b127d1eaa3d`.
Its 341 host tests, 42 guest cases and critic acceptance establish their measured
scenarios; they do not establish third-party application support.

A fresh headless normal boot used that same image:

```powershell
.\target\release\door-xtask.exe run --profile profiles/hardware/qemu-diagnostic-x64.json --image out/images/host-1792214999674635300-12500/image.json
```

Result: `reports/runs/host-1792216117971811700-21736/report.json`. The QEMU
process exited with expected debug-exit code 33 after approximately 2.64 seconds.
The guest serial shows real CPL3/IPC/protection fixture execution followed by
`KERNEL_DIAGNOSTIC PASS`. It did not enter a persistent ordinary user session.
This is a diagnostic observation, not a new full-suite qualification.

## Phase-by-phase findings

| Phase | Actual implementation | What its acceptance does not establish |
|---|---|---|
| P01 | Separate build targets, artifact inspection, image staging and an actual QEMU launcher. | Build infrastructure is not user-facing OS functionality or an installer. |
| P02 | UEFI reads the images, calls ExitBootServices, installs CR3/stack and jumps to Door's kernel. | An ordinary persistent boot/session flow. Default runtime remains diagnostic. |
| P03 | Frame/heap ownership, page tables, descriptor/exception setup and APIC/timer mechanisms. | General physical-machine support, unbounded allocation or release reliability. |
| P04 | Timer-driven context selection, assembly context switches, secondary CPU startup and native waits/TLB retirement. | General SMP service/Windows workloads. Current service profile requires one online CPU. |
| P05 | Native ELF segments map into private spaces; actual CPL3 entry, syscall handling, handles, IPC and contained faults exist. | A user can load an arbitrary executable from a file and pass arbitrary arguments/environment. Current admission selects trusted manifest catalogs. |
| P06 | Guest VirtIO descriptor submission/completion and real writes to a raw disk image; constrained ext2, FAT32 and initfs backends. | A general disk/filesystem platform. Writable ext2 has one group, 128 inodes and direct-block files up to 48 KiB. |
| P07 | Journaled logical namespace, access/sharing checks, durable file/settings/registry operations and recovery scenarios. | Full Windows filesystem/registry behavior or a usable file-management interface. |
| P08 | Protected init/storage/session/console processes, authenticated IPC, restart/session revocation, and an original native SDK workload. | A practical shell, normal user account/password management, full native std library, or general application registration/launching. |
| P09 | Real PE32+ mapping, imports/relocations, Windows x64 machine-code execution and original compatibility DLLs. | Broad EXE compatibility. The startup catalog selects the project's windows-basic.exe fixture. |
| P10 | Real scoped DLL/TLS/thread/memory/wait/file/exception/CRT behavior exercised by the project's windows-base.exe and lifecycle.dll. | Arbitrary EXE/DLL loading, full Windows APIs/runtime, third-party application workflows or usable Windows GUI applications. |

Implementation anchors:

- P01: `tools/xtask/src/build.rs`, `tools/xtask/src/image.rs` and `emulator.rs`.
- P02: `boot/uefi/src/main.rs:164` and `boot/uefi/src/handoff.rs`.
- P03/P04: `kernel/src/mm`, `kernel/src/arch/x86_64`, and
  `kernel/src/execution.rs:87`. The portable scheduler metadata is connected to
  actual interrupt/assembly context switches; it is not the entire runtime.
- P05: `kernel/src/arch/x86_64/user.rs:138`, `interrupts.S`,
  `kernel/src/process/image.rs` and `platform.rs:288`.
- P06/P07: `kernel/src/drivers/virtio_block.rs:141`,
  `fs/storage-core/src/storage/ext2.rs:92`, and
  `fs/storage-core/src/persistence/namespace.rs:205`.
- P08: `services/src/init.rs`, `storage.rs`, `session.rs` and `console.rs`.
- P09/P10: `kernel/src/process/windows.rs:18`, `windows.rs:54`,
  `kernel/src/process/vm/windows.rs` and `compat/dlls`.

## Product-blocking gaps

1. **Boot remains organized as a test harness.**
   `kernel/src/main.rs:44` runs task/process qualification before service modes;
   `main.rs:184` ends normal boot with debug exit. The interactive mode is selected
   by QEMU fw_cfg. `boot/config/boot.txt` still describes P04 diagnostics; the
   kernel checks its presence rather than using it as a normal boot policy.

2. **Process admission is coupled to fixture identities.**
   `kernel/src/process/platform.rs:288` selects executable bytes by manifest ID;
   `platform.rs:561` registers five fixed native images. Windows uses fixed
   project EXE/DLL names and bases in `windows.rs:54`; command lines are
   constructed for those fixtures in `windows.rs:206`. A generic ELF/PE parser
   underneath does not provide a file-based exec or CreateProcess workflow.

3. **The interactive application is a fixture rather than a general shell.**
   `tests/fixtures/native-workload/src/main.rs:472` accepts six fixed commands.
   `file` checks the known workload file; it cannot browse or edit user-selected
   files. `theme` persists a setting number but does not apply a visible theme.
   `threads` really creates and joins a thread, then reports that scenario's result.
   These operations have real side effects but provide little practical utility.

4. **Demo launches do not resume user state.**
   `tools/xtask/src/main.rs:83` selects a saved qualification baseline.
   `tools/xtask/src/services/demo.rs:110` requires that baseline's saved fixture
   state and copies it to a fresh disk per launch. Changes remain in the old private
   disk, but plain demo does not reopen that disk on the next launch. This is a
   testing isolation policy, not a normal persistent user profile.

5. **Windows compatibility has no third-party application qualification.**
   The tested applications are original project fixtures, compiled with ordinary
   Windows interfaces and run unchanged in both environments. No independently
   distributed Windows application/version/workflow is qualified. P10's five-module,
   four-thread, 1 MiB commitment and scoped runtime limits remain real constraints.
   Async-write source review also lacks a separate unchanged-binary comparison.

6. **Development foundations are presented too easily as complete facilities.**
   Session handles enforce real authority, but authentication uses provisioned
   capabilities for fixed development UIDs; password/entropy providers are absent.
   Native N1/N2-foundation adapters are not a full Rust std port. P08 service policy
   also bounds execution to one CPU. These are implementation boundaries, not
   facilities a GUI will automatically complete.

## Required direction

Preserve the real mechanisms and their regression tests. Productization must add
the missing paths instead of replacing those mechanisms or merely widening
constants:

1. Separate ordinary boot/session lifecycle from diagnostic and failure-injection
   modes. Ordinary boot must remain usable without a test token or pass markers.
2. Own a persistent user disk/profile that reopens by default; keep test clones as
   explicit testing behavior. Demonstrate user-chosen data across shutdown/restart.
3. Expose general file operations and a useful shell through existing storage
   authority, including paths, enumeration, create/read/write/rename/delete and
   errors. Tests must use multiple user-chosen names and contents.
4. Load admitted programs from files with real executable identities, arguments,
   environment and working directories. Preserve capability and resource policy;
   a general launch path must not grant every program test-fixture privileges.
5. Establish a versioned third-party application trial inventory. Record real
   workflows and exact missing APIs/dependencies without substituting success
   stubs. Console compatibility and GUI/runtime-dependent applications need
   separate support records.
6. Build P11 on real protected clients and these application/session paths.
   P11 supplies window/composition foundations; P12 editing, P13 Win32 GUI and
   P14 desktop workflows remain separate requirements. A visual layer alone
   cannot resolve the earlier product gaps.

This audit does not silently revoke historical scenario evidence or claim that
all phase requirements were fully implemented. Implementation changes to earlier
code must reopen and requalify the affected active scope through the established
test/critic/documentation routine. Product usability needs its own explicit
acceptance conditions in addition to those retained subsystem gates.
