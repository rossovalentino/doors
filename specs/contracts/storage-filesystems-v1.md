# Storage and filesystems v1

P10 readiness extension: dense ext2 regular files now support twelve direct and
one single-indirect block (4,243,456 bytes maximum). Mount validates ownership of
the indirect block and every data pointer, unused pointer zeros and sector counts.
Truncation/unlink reclaim the complete graph. The original direct-only text below
records the historical P06 subset; current production limits are specified in
`ordinary-session-v1.md`. The shared implementation lives in `fs/storage-core`.

Owner: `kernel/src/storage`; the hardware transport is owned by
`kernel/src/drivers`, and image creation/independent inspection by `tools/xtask`.
Wire formats remain the published GPT, FAT32 and ext2 formats. No host filesystem
function is called by the guest implementation. See ADR0011 for decisions/limits.

`BlockDevice` exposes `geometry`, `read_sectors`, `write_sectors`, and `flush`.
Transfers must be nonempty and a multiple of logical sector size; checked LBA
arithmetic rejects overflow and capacity violations. Success means the complete
requested transfer. `ShortIo`, `Io`, `Timeout`, `DeviceLost`, `ReadOnly` and
`Unsupported` remain distinguishable. Error buffers may contain a transferred
prefix. Filesystem callers hold exclusive device/mount access; reset or external
mutation requires destroying all dependent caches and mounts before reuse.

`Gpt::read` requires a 512-byte device, protective MBR, revision 1 headers of 92
bytes and two matching arrays of 128 entries, each 128 bytes. Both CRC32 checks and
backup positions are validated, including all used partition ranges and unique
IDs. `find(type_guid)` rejects ambiguous matches. `PartitionDevice` offsets and
bounds all access and applies the partition's read-only attribute. The original
development image is 128 MiB: FAT32 at LBA 2048 for 131072 sectors, ext2 at LBA
133120 for 65536 sectors. Disk and partition GUIDs belong to fixture identity,
not a permission grant to format another disk.

`Vfs` provides open, metadata, read, create, write, rename and flush. Read returns
the actual count at EOF and leaves the unused output suffix unchanged. Read-only
backends reject mutation. `Mount::new(prefix, backend)` assigns a nonwrapping
incarnation, resolves only exact prefix boundaries and checks it on each open
file operation. The trusted caller serializes mount access. `OpenFile.id` is
`FileId { volume, inode, generation }`; it remains equal after a successful rename.
Namespace replacement/deletion and security checks are unsupported, not inferred
from stored POSIX mode bits. Names outside the declared backend subset fail.

`Ext2::mount` validates the entire constrained allocation graph before allowing
writes. It needs writable media and a working flush operation. The profile is
one group, 4 KiB blocks, 128 inodes, 128-byte inode records, root inode 2, reserved
inodes 1–10, first ordinary inode 11, direct blocks only, `FILETYPE=2`, all other
feature bitmaps zero. All logical file bytes are dense; extension gaps are zeroed
even when an imported last-block tail contains nonzero bytes. Create and growth
reserve sufficient directory/inode/block capacity in memory before dirtying disk.
Rename preserves identity and rejects replacing an existing different name.

Each ext2 mount owns one 4 KiB write-through cache page. All data and metadata use
it. A write becomes cache-visible only after exact device success. Any failed
mutation invalidates cached data and poisons the mount. The first mutation writes
and flushes the unclean state; explicit flush orders completed data/metadata before
the clean marker and flushes again. Dirty mount, unsupported feature, malformed
graph and failed I/O are errors. `into_inner` first flushes and therefore cannot
extract a poisoned filesystem as if clean. Dropping a dirty filesystem does not
silently flush it. No journal/replay or crash-atomic rename is implemented.
Read-only metadata borrows the cache page. Validation retains compact bitmaps and
offsets, and `Ext2` with a borrowed device is bounded to 6144 bytes by a host check.
The guest keeps its original 128 KiB guarded boot stack and samples remaining
stack space at actual device I/O; cache growth requires revisiting that budget.

`Fat32::mount` validates the selected read-only exchange profile: 32 reserved
sectors, two FATs, one-sector clusters, root cluster 2, boot backup 6, FSInfo 1 and
backup 7, clean FAT status, at least 65525 clusters and no more than 131072.
Root capacity is 16 short entries; regular files are at most 1 MiB. Lookup applies
ASCII case folding to short names. Every used chain is checked for bounds,
crosslinks and exact termination; each accessed FAT sector must match its mirror.
LFNs, directories and writes fail rather than pretending supported behavior.

`InitFs::new` adapts the already validated immutable archive. Directory/file kind,
entry index, bounds and read-only policy are preserved; no disk flush is needed
for immutable storage. Unknown/stale identities and foreign mount handles fail.

Required evidence: host malformed metadata, overlap/identity, ENOSPC, coherent
cross-block reads/writes, dirty-media and I/O failure tests; guest actual VirtIO
partition/FAT/initfs/ext2 reads, 6000-byte write plus 1024-byte patch at offset
3584, rename while open, flush, independent same-image boot/reopen and exact host
payload verification. Initial byte `i` is `(i*37+11)%251`; patch-relative byte `j`
is `(j*13+7)%251`. The published path changes from `pending.bin` to `persist.bin`.
Do not report durability from a host-only adapter or an acknowledged unflushed
write. P07 is responsible for a separate recovery and security-metadata contract.

Primary references: UEFI 2.10 section 5; Microsoft FAT specification v1.03,
December 6, 2000; upstream kernel ext4 legacy superblock, group descriptor,
bitmap, inode/direct block and directory format documents. Acquired bytes,
URLs and identities are pinned in `specs/sources/index.json` by the phase owner.
