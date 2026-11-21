# Build and image pipeline v1

Owner: `tools/xtask`. Supported host: the pinned Windows x64 Rust/MSVC profile.
The build plan retains six distinct artifact classes and prerequisites. Only the
host executable is built in P01. Guest classes remain visibly blocked for their
actual source/runtime gates; `build --profile dev-qemu-x64` fails explicitly.

`build --profile host-dev` creates an exclusive directory under `out/builds/`,
executes locked release compilation for the explicit host target, inspects the PE
bytes and runs the resulting executable's help command. `verify` performs two
builds in separate fresh target directories and compares complete executable hashes.
Compiler path remapping and stable PDB references are passed through encoded Cargo
Rust flags, so paths with spaces stay single arguments. The pinned MSVC linker
accepted `/Brepro` in actual builds; that switch is an observed supporting-tool
interface, not a claimed public Windows compatibility contract. `/PDBALTPATH` has
its own Microsoft source record. Flags remain scoped to the host build.

Manifests retain artifact class, format/machine, entry, ABI personality, imports,
named exports, relocation model/location, byte hash, size, source snapshot and
toolchain profile. The controlled rebuild claim applies to this host executable
and workstation. It does not claim clean-machine or guest reproducibility.

`image --manifest <relative-file>` consumes JSON schema version 1, record kind
`image_inputs`, and exactly three entries. Each contains `role`, confined repository
relative `path` and `sha256`. Roles are unique: firmware, kernel and initfs. Firmware
and kernel are inspected against their artifact personalities. Initfs must be
nonempty and at most 64 MiB; P02 must validate the ustar archive and file roles.
All validation happens before allocating an exclusive output directory.

The only destination mapping is:

| Input role | Staged destination |
|---|---|
| firmware | `esp/EFI/BOOT/BOOTX64.EFI` |
| kernel | `esp/Door/kernel.elf` |
| initfs | `esp/Door/initfs.tar` |

Copied bytes are rehashed. `image.json` contains only fixed destinations and
inspected identities, with `boot_qualified=false`. Its kind is a read-only QEMU
virtual FAT directory tree, rather than a physical GPT/FAT disk. This avoids a
second filesystem writer before actual boot. A later installable disk format
requires its own documented and tested contract. No timestamp-dependent virtual
FAT sector byte-reproducibility is claimed.

Paths reject traversal, absolute/device/alternate-stream inputs, symlinks and
Windows reparse points. Generated destinations stay in `out/`; existing outputs
are never reused or replaced. Unique directory creation detects collisions.
Run-time validation reapplies the same nonempty/size/identity checks to initfs and
reconstructs metadata for all roles. It rehashes/reinspects staged inputs and creates a fresh private
copy for each emulator run. Shared image inputs are never writable guest devices.
These checks assume a trusted local filesystem; hostile concurrent replacement
requires an immutable snapshot or handle-based confinement in later release gates.

Phase qualification requires two distinct artifacts in separate target directories,
each bound to its own successful build and smoke commands and the qualifying source
snapshot. Fresh inspection must match the preserved artifact metadata. Reusing one
artifact twice or removing either build cannot satisfy the rebuild gate.

Errors preserve command and raw failure evidence. A tool's zero exit is followed
by actual inspection. There is no guest pass from an archive, a build or staging.
Tests cover positive staging, deterministic manifests, altered bytes, missing and
duplicate roles, path traversal, artifact class mismatches and destination collisions.

P02 enables the actual firmware/kernel/initfs build. P05 adds two independent
native ELF builds in each controlled guest pass, using the N0 contract in
`specs/abi/native-x64-v1.md`. `tools/xtask/src/build/native.rs` owns the exact native
flags and distinct per-binary build commands/maps. Artifact inspection enforces
the protected loader's low-address guard, eight segments and 16 MiB memory/span
bounds. Native output hashes join firmware/kernel/initfs in the reproducibility
comparison and their bytes enter initfs through checked generated artifact IDs.
These build results establish byte identity; only guest execution qualifies ABI
behavior and process isolation. Windows DLL/fixture build classes remain blocked.

