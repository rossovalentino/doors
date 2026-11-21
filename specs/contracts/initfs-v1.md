# Door initfs v1

Owner: `door-initfs` for allocation-free parsing/header encoding and `door-xtask`
for manifest-driven assembly. Public ustar header layout: GNU-USTAR. This is a
constrained profile; archive metadata never grants privilege or creates writable state.

512-byte headers/padding, `ustar\0` magic at 257 and `00` version at 263. Regular
files (type `0`) and directories (`5`) only. Numeric fields are bounded ASCII octal,
terminated with NUL/spaces; binary numeric extensions are rejected. Header checksum
is the unsigned byte sum with bytes 148..156 treated as spaces. Name occupies
0..100 and must contain a NUL; prefix 345..500 and final padding must be zero.
Link name is zero. Hard/symbolic links, device nodes, PAX/GNU extensions and unknown
types are rejected. File padding and all bytes following two terminal zero blocks
are zero. Checksums, sizes, arithmetic and full extents are checked before views escape.

Paths are relative ASCII components containing alphanumeric, `.`, `_` or `-`.
Empty, dot/parent, absolute, drive/stream and backslash paths fail. Directories may
have a trailing slash. Duplicate file/directory identities and regular-file ancestor
conflicts fail. Bounds:64 MiB archive,32 MiB resource file,128 entries,names <100 bytes.
Native ELF files and their mapped image span remain independently capped at16 MiB.
Immutable desktop resources are separate from executable bytes.
An empty structurally valid archive still fails the boot role's required configuration.

`profiles/build/initfs.json` lists unique sorted destinations and confined source
files. The encoder uses fixed mode, zero uid/gid/mtime and zero ownership names;
contents come only from listed files. Every assembled archive is parsed again.
Host staging, firmware loading and kernel validation use the same reader. BootInfo
provides the retained physical extent, exact file length and SHA-256; the kernel
checks that identity before reading. Required boot file: `system/boot.txt`, type `0`.
The shared `Archive::file` lookup rejects directories at required file roles;
host inspection, firmware and kernel all use it. Encoder wire-name bounds include
any trailing directory slash and are checked before modifying the header.

No mount, user executable launch, writable filesystem or persistence is provided in
P02. Parser inputs must be stable slices; no physical pointer is dereferenced here.
Tests include truncation, mutation, checksum, traversal, duplicate/ancestor paths,
unsupported types, prefix/padding, missing terminators and size bounds.

P05 extends the build manifest with an `artifact` alternative to `source`. Exactly
one is allowed. Artifact names resolve only against the current controlled pass's
known native builds; unknown/duplicate names fail. Each ELF is inspected as
`native-user` before packing. Sorted destinations are `bin/native-probe.elf`,
`bin/native-runtime.elf`, then `system/boot.txt`. The archive retains their exact
bytes and the gate binds each file back to its separately inspected build record.
No generated source path points at a stale `latest` output. This changes archive
contents, not the ustar wire contract or immutable historical P02–P04 evidence.

ADR 0018 adds generated resource packs. An entry `{"artifact":"app-assets",
"class":"resource-pack"}` resolves only against the pack generated in the
current build pass from `profiles/build/app-assets.txt` (sorted, unique, valid
keys naming files under `assets/`). The packer validates it as DOORAST1 and never
inspects or accepts it as an executable; no other artifact may use the class.
Its destination is `system/app-assets.pack`; the 32 MiB resource bound applies.
The archive bound is 256 MiB (ADR 0018 addendum; 192 MiB before the IME,
raised to the UEFI loader's largest boot allocation): the desktop archive also
carries the on-demand fonts as `fonts/*` source entries and the IME
dictionaries as `ime/*.dict`, each within the 32 MiB file bound. An entry may
carry `"optional": true` with a `source`: it is packed only when that file
exists (the dictionaries are absent from builds that did not fetch them).
