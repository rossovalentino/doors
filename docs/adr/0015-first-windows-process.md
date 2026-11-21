# ADR0015: first Windows x64 process through original compatibility DLLs

Status: accepted and qualified for the declared P09 scope.

The first Windows milestone needs real protected execution without duplicating
the native process, storage or console systems. A host PE inspector or a patched
fixture would not establish the required compatibility boundary.

Use a portable bounded PE dependency/fixup planner and the existing transactional
kernel VM constructor. The kernel assigns a trusted Windows personality, maps the
original PE DLLs and EXE, prepares pinned public Windows views, and enters an
original ntdll bootstrap in user mode. Thin kernel32 exports call ntdll adapters;
checked pointer capture and authenticated native console IPC produce actual output.
The ordinary independently compiled EXE uses only its declared Windows imports.

Initial mapping remains kernel-coordinated so no new untrusted executable-mapping
authority is needed merely to launch the first program. The portable plan separates
parser/policy logic from privileged mapping, with full reservation/protection and
rollback. Native FS, Windows user GS and trusted kernel GS have distinct ownership.
Single-thread and `/NOENTRY` DLL constraints are enforced, not implied by a fixture.

The detailed export, pointer, error, state-layout and evidence contract is
`specs/abi/windows-x64-p09.md`. Its limitations include bounded ASCII output,
no CRT, no dynamic DLL/TLS lifecycle and no Windows unwind dispatch. Unknown
requirements fail explicitly. The installed Windows reference and SDK provide
comparison/layout evidence; no system DLL bytes are copied into Door.

P10 must extend module/thread/exception lifetime and buffer stability together,
before removing these restrictions. It must keep the original P09 same-binary
comparison and every earlier native service/persistence regression. Record actual
qualification and critic findings here when the phase closes.

Exploratory integration `host-1792065024570271200-13884` passes actual same-binary
CPL3 execution/output/exit73, Win64 register checks, seven malformed-load cases,
79 rollback checkpoints and full process/DMA retirement. The first scratch mapping
exposed intentionally persistent kernel structural tables; the diagnostic now reuses
the already retired P05 alias span with explicit ownership, guard, frame and table
baseline checks. No page-table pruning policy was changed for a diagnostic.

P09 closed with final gate `reports/runs/host-1792150739645891100-11748/report.json`
(SHA256 `48fa8a34f832a2b5de8762d7de4deeb307de72b2e1c1a85ec40960c943bac1ad`).
310 host tests, all40 guest scenarios, both interactive checks and two matching
13-artifact builds passed with unchanged inputs. The exact independent Windows
EXE runs on Windows and Door with verified output/exit73, real relocation,
invalid handles/buffers and ABI preservation. Seven malformed-load cases and79
construction rollback points pass; process/DMA resources retire and the P08 disk
remains byte-identical. `reports/critics/p09-critic.json` accepts248 build inputs,1202
hash bindings and eight rejected weakened gates with zero unresolved findings.
The supported scope remains one no-CRT Windows thread, static original DLLs and
bounded synchronous ASCII output. P10 owns DLL/TLS/unwind and broader base APIs.