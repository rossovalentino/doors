# Image validation v1

Owner: `door-image-format` for allocation-free structural parsing; `door-xtask`
for artifact personality, hashing, manifests and host file acquisition. Inputs must
be stable immutable slices. No parsing operation maps memory or executes code.
Sources: ELF-GABI-HEADER, ELF-GABI-PROGRAM, MICROSOFT-PE in the source ledger.

The initial profile bounds files to 256 MiB. Checked reads never cast unaligned
bytes into Rust structs. Host tools inspect actual contents rather than extensions.
The library supports host tests and both installed guest targets without dependencies.

ELF scope: little-endian ELF64, x86-64, static ET_EXEC, fixed 56-byte program headers,
at most 128 program headers and 4096 section headers. The producer emits sorted,
disjoint PT_LOAD extents with 4096-byte alignment, congruent file/virtual alignment,
read permission and no writable executable segment. File sizes cannot exceed
memory sizes. Virtual extents must be canonical for 48-bit paging; their sum is
bounded to 1 GiB. The entry must be in file-backed executable bytes. Interpreters,
dynamic linking, TLS, unknown segment kinds and nonempty relocation sections are
rejected in this initial profile. PT_NULL is ignored, including its undefined fields.
NOTE, PHDR, nonexecutable GNU_STACK and GNU_RELRO are supported as non-load metadata
with checked file extents. There is no KASLR/dynamic loader claim.

The host kernel personality additionally requires all loaded segments and the
entry in the chosen high kernel half at/above `0xffffffff80000000`; native-user
segments must stay in the low user half. These are original Door bootstrap rules,
not universal ELF requirements. Mapping, zero-fill execution and physical ownership
remain P02/P03 obligations. The manifest records zero-fill extents, not a performed fill.

PE scope: x86-64 PE32+, bounded DOS/COFF/optional/section records, at most 96 sections
and 16 directories. File-backed section and directory ranges are checked, including
the certificate directory's file-offset convention. Sections are disjoint with
valid file/section alignment and no writable executable section. Executable entry
points must be file-backed; a DLL may have an intentionally absent entry. Delay
imports are rejected. Import DLL strings and named exports are bounded and decoded;
named export ordinal indexes are range checked. Relocation directory location/size
is recorded. Full thunk imports, ordinal-only exports, relocation application,
unwind execution and ABI equivalence are future loader/compatibility work.

Artifact classes are host, firmware, kernel, native-user, Windows DLL and independent
Windows fixture. Firmware requires an EFI application subsystem and no host imports;
host/fixture executables require console/GUI subsystems. A DLL requires its DLL flag.
Classification and inspection do not prove that an image performs its intended role.
No real firmware/kernel implementation existed in P01. P02 now builds the real
firmware/kernel and uses these checks; Windows execution remains unimplemented.

Errors distinguish truncation, identity/version support, limits, overflow, bounds,
alignment, overlap, permissions, invalid entry, dynamic dependencies and strings.
No partial validated image escapes failure. There is no allocator, FFI, cancellation
or durable state in the parser. Callers establish readable backing and concurrency
invariants; hashing is identity, not provenance authentication.

Tests: `crates/image-format/tests/images.rs` and host artifact/staging tests.
Original synthetic image fixtures are shared in `tests/support/images.rs`. Their
bytes exercise format semantics and are never reported as functioning OS binaries.

P05 native artifact inspection and `kernel/src/process/image.rs` impose the stricter
protected profile: 16 MiB file, eight load segments, at most 16 MiB page-rounded
committed memory and virtual span, no page-rounded overlaps, and all extents above
the 64 KiB null guard within the user half. Actual native launch further confines
image mappings to 0x400000..0x1400000, separate from stack/startup/TLS/section regions.
The VM loader allocates zeroed owned backing, copies only validated file bytes and
applies final user RX/R/RW permissions. Parser or host inspection success still
does not establish execution; P05's independent native ELF fixtures, contained
faults and protected process gate provide that separate evidence.

