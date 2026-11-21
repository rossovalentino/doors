# Measured text application workflows

The P10 cumulative run is
`reports/runs/host-1792235592894575300-22840/report.json`, SHA256
`64261b5ced696019b1fa79c6502bbc7d9cbaba46c190d32b81c7fd2c38cf6361`.
Its `session-build.json`, `ordinary-session-suite.json`, command streams and two
ordinary-boot serial logs retain the application bytes, inputs and observations.
The profile is QEMU q35/TCG, AMD64 UEFI, 2 GiB RAM and one service vCPU.
The suite passes without a diagnostic boot token or debug-exit device.

| Application | Origin and exact artifact | Observed workflow |
|---|---|---|
| Door tools | Independent native ELF; SHA256 `2f3ea3ccf769c44923e49c1a75cbe9baec3ec322b9c3311b1d8473173c4b2bef` | File-selected launch, copied ELF launch, quoted arguments, environment/cwd, word count, repeated exits, cancellation and large-file generation. |
| Ed Davis Tiny Basic | Unchanged upstream interpreter at commit `fe4c942711c63cc932574b2f37a9ee628b001e31`; original Win32 C platform source port. PE SHA256 `1e00735caeb2387d4f5b9debec6990f84f3c656b499c38115a33a2dba13c9de3` | The same executable computes 55 and 1024 on Windows and Door. Guest editing with Backspace, loops, save/new/load/run, a 229,893-byte saved program, an unrelated subsequent write and fresh-boot argument-based loading pass. |

Both applications and the shell have matching independent rebuilds. The ordinary
suite records 87 shell receipts, 18 Tiny Basic receipts and an actual guest reboot.
It exercises real file creation, append, rename, deletion, 36 extra directories,
byte-preserving binary redirection, rejected access/malformed programs, theme
persistence and separate-boot reopening. Independent host decoding checks the
actual journal, ext2 allocation graph, catalog, file hashes and persisted bytes.
The same saved/reopened disk hash is
`053b4ccd529ca5eb1c6a9f5f2bf1e50582713a03139065f0ed59b09fc49c0e94`.
Both ordinary sessions retire their resources with zero reported process-frame
leaks and eight DMA pages reclaimed; the reopened sessions perform no disk writes.

This qualifies one third-party **source port**, not a distributed stock Windows
application. No stock Windows application corpus has been qualified. The C
platform is intentionally narrow: ASCII paths, four open files, a 384 KiB heap,
bounded formatting and public Win32 imports supplied by Door's original DLLs.
The upstream interpreter is not a hardened untrusted-language sandbox. General
DLL search, C++ runtimes, GUI applications, networking and physical hardware
remain unqualified. Finite workflows do not prove the absence of every defect.

For the commands to use the system, see [text-session.md](../build/text-session.md).
Current phase and critic acceptance are recorded in `project/phases.json`.
