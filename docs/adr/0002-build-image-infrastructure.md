# ADR 0002: shared inspection and bounded host infrastructure

Status: adopted for P01. Owner: project engineering.

P00 established a portable handoff and host verification. P01 needs genuine build,
inspection, image planning and process outcomes before P02 can create the OS.
Adopt `crates/image-format` for allocation-free checked ELF64/PE32+ structures and
reuse it from the host orchestrator. The future loader reuses checked semantics
instead of maintaining another unreviewed parser. Personality and filesystem
policy stay in `tools/xtask`, outside the portable format crate.

Extend the existing xtask with responsibility-specific modules rather than extra
single-use scripts or separate empty tools. Shared original binary fixtures live
under `tests/support` and are used by parser and staging tests. Tests create and
remove their own temporary workspaces; generated builds/evidence remain ignored.

Use a read-only QEMU virtual FAT ESP directory for bootstrap staging. Alternative:
write a new GPT/FAT implementation before a real loader exists. Deferred because
P01 requires deterministic image planning/staging and P02 provides the first real
boot artifacts. The directory is explicitly identified; installable disks and
raw-sector reproducibility are separate future gates. No host disk is attached.

Use fresh copies of image files and firmware variables per run. Argument arrays,
restricted topology, content hashes, bounded outputs and process cleanup preserve
ownership. A token/ordered kernel diagnostic protocol plus debug-exit prevents
ordinary pass-text from accidentally satisfying a boot check. It is not an
authentication boundary against a hostile host or a general application test API.

Keep P00 evidence historical when P01 changes shared tooling. Check its original
gate/log/review bindings, then require fresh complete P01 verification and critic
acceptance for current inputs. Version 1 review scope stays readable; P01 scope
also binds hardware profiles. A passed currently selected phase rejects stale
inputs. Changing historical records to imply new execution is prohibited.

The actual host release rebuilds compare complete executable hashes with path
remapping and the pinned linker's deterministic option. Scope is this tested
workstation/executable; no guest or clean-machine result is inferred. The initial
closure lacked QEMU/OVMF. An owner-authorized amendment now acquires exact pins and
checks live identities and capabilities; the host harness remains host_tested
until real P02 execution. Revisit after actual firmware/controller constraints, new host/ISA,
device topology, runtime loader support or installable disk requirements.

Tests cover malformed images, ownership/personality boundaries, path confinement,
staging identities/collisions, process limits, result ordering and controlled builds.
ABI/scope evolution changes the contract, implementation, fixtures and docs together.
