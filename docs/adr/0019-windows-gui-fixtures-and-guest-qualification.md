# ADR 0019: Windows GUI fixtures, reference and guest qualification

Status: adopted 26 October 2026 (P13 workstream C). Contract:
`specs/contracts/windows-gui-v1.md`. Subject to the P13 critic review.

## Context

P13 must show an independently built, unmodified Windows GUI program working in
Door (GUI-E, WIN-GUI, GUI-01/02/03, scenarios 141.5/141.6) with the fixture bytes
preserved between the Windows reference and the Door guest (P09 rule). The owner
allowed short reference runs on their PC without input injection or setting
changes (decision 4). The P13 loader and GUI DLLs are being written in parallel
(P13-A, P13-B), so the guest runners must exist before the code they qualify.

## Decision

1. **Three fixtures, four executables.** F1 (self-test), F2 (Notepad-like
   editor, code and resource variants from one source) and F3 (hang). Each has a
   non-interactive mode for the reference and the guest, and F2/F3 an
   interactive mode driven only by real input in the guest.
2. **Results files, not only standard output.** Every fixture writes the same
   deterministic lines to standard output and to a results file in its working
   directory. An Explorer-launched GUI process has no console, and a results
   file is real kernel32 file I/O that the runner audits on the raw disk.
3. **Published records beside the sources.** `imports.json` (the import
   inventory P13-A/P13-B implement against) and `reference.json` (results,
   metrics, hashes, OS identity) are regenerated only by `cargo xtask build
   --profile windows-gui-reference` and refused by the guest suites when stale.
   The Windows reference results are the expected values; fixtures keep internal
   hard checks for invariants.
4. **Side effect undone value by value.** `OFN_DONTADDTORECENT` keeps Recent
   Items unchanged, but the Explorer-style dialog still writes its ComDlg32 MRU
   lists. The reference runner exports
   `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\ComDlg32` before the
   first launch (the snapshot is kept in the working directory and the run
   directory) and, also when the run fails, undoes only the run's own changes:
   values absent from the snapshot are deleted (with any subkey the run
   created), and changed or missing values (such as `MRUListEx`) are re-imported
   byte-for-byte from the snapshot's own export data. The key itself is never
   deleted (an earlier `reg delete /f` plus whole-key import was replaced on
   2026-10-26 after the P13 critic review). The run fails unless the restored
   values and subkeys equal the snapshot and RecentDocs is unchanged. The
   published reference (run host-1792952263026238500-16188) predates this
   restore and added 16 MRU entries; the owner approved removing them on
   2026-10-26, with the backup at
   `C:\Users\win\Desktop\Doors OS\comdlg32-mru-backup-20261026.reg`
   (`reference.json` `registry.comdlg32_restore`). Because Windows may prefer its
   last-visited folder, the self-test hooks type full target paths.
5. **Ordinary paths for Windows children.** `Run` starts children in the
   drive-letter form of the canonical directory, and the reference passes the
   executable's drive-letter path: the shell mishandled verbatim `\\?\` working
   directories and module paths (observed: Save landed in the previous run's
   folder).
6. **Private image variant for the suites.** The suites stage `desktop-win32`
   from the production desktop initfs plus the fixtures, the GUI DLLs and an
   extended install manifest, instead of changing the shipping initfs profile
   before the DLLs exist.
7. **Implementation-independent evidence.** A process is identified by the
   kernel's module record carrying the fixture SHA-256 and retired by the
   kernel's process record (exit code and uid); content claims read the raw disk;
   screendumps support visual claims (and window geometry: content displaced by
   exactly the pointer delta). Amended 2026-10-26 (P13 critic follow-up): hung
   checks follow the Windows semantics (no message retrieval for 5 s, the
   "(Not Responding)" ghost, End task or Wait on Alt+F4) through the window
   server's `DOOR_P13 WIN32 NOT_RESPONDING`, `NOT_RESPONDING_PROMPT`, `WAIT` and
   `END_TASK` records, and owner accounting needs a `DOOR_P13 OWNER_RELEASED`
   record with every count zero (requested from the window-server owner; the
   check fails closed until it exists).
8. **Correct, not lenient.** Interactive F2 results must equal the computed
   13-line script exactly (dialogs, byte counts, CRC-32, chrome geometry); the
   gate stays closed until the guest matches.
9. **Evidence outlives `out/`** (2026-10-26). Sealing a guest session copies
   its serial log into the owning run (`reports/runs/<id>/retained/...`,
   `serial_retained`, whose hash is `serial_sha256`), and every report copies
   each cited `out/` serial log, staged image record and screenshot there
   (`retained_evidence`). Report `verification_state`, `guest_tests` and
   `reference_comparison` derive from the guest result and the passed
   acceptance suites (each suite reports its own reference comparison).

## Consequences

* The five P13 suites are registered; `verify --phase P13` passes preflight and
  fails at the first P13 command or guest step until P13-A/P13-B and the stage-2
  Explorer/Terminal launch path (WIN-LAUNCH) land.
* Any fixture change requires a new reference run on the owner's PC; the runner
  undoes its own ComDlg32 MRU changes value by value (section 4).
* Multilingual Win32 input and DPI-scaled coordinates are
  recorded gaps, not claims.
