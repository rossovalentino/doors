# ADR 0011 — Bounded standard-format storage and coherent file identity

Status: implemented for P06; qualifying guest evidence and critic acceptance are
tracked by the central ledger. This is original Rust code, not a filesystem port.

Storage uses one exact-transfer `BlockDevice` contract. Geometry reports logical
sector size, capacity, write protection and flush support. An error may follow a
partial transfer; it never means that the remaining bytes succeeded. The selected
profile uses 512-byte sectors. `PartitionDevice` bounds every request and preserves
device errors and partition write protection. Discard, FUA, physical-sector
topology and arbitrary scatter/gather are not filesystem capabilities in this phase.

The GPT reader validates the protective MBR, both header checksums, both entry-array
checksums, matching copies, usable bounds, nonzero unique IDs and non-overlapping
partitions. It accepts the explicitly selected 128-entry/128-byte layout and fails
closed when either copy is inconsistent. Labels never authorize formatting or
select an erase target. See UEFI 2.10 section 5 in the acquired source ledger.

The exchange mount is read-only FAT32. Its bounded profile uses 512-byte clusters,
two mirrored FATs, matching boot/FSInfo backups, a single root cluster, short ASCII
8.3 names and files up to 1 MiB. Chains are bounded by file size; missing endings,
cycles, crosslinks, invalid clusters and disagreement between used FAT sectors are
rejected. LFN lookup, subdirectories and FAT writes are unsupported. The original
implementation follows the acquired Microsoft FAT specification v1.03; it does not
claim general FAT/Windows namespace compatibility.

The writable development backend is standard ext2: 4 KiB blocks, one block group,
128 inodes of 128 bytes and only `INCOMPAT_FILETYPE` enabled. `COMPAT` and
`RO_COMPAT` must be zero. The formatter supplies 8192 blocks; valid last partial
groups of 16–8192 blocks permit bounded media-pressure tests with the same code.
Metadata occupies blocks 0–7, root directory block 8, and regular-file allocation
begins at block 9. The root has one directory block; ASCII component names are at
most 63 bytes. Files use at most twelve direct blocks, 48 KiB, with dense zero-filled
gaps. Unsupported indirect blocks, extents, journals, ACL/xattr blocks, fragments,
symlinks, subdirectories and additional feature bits are rejected. Mount validation
checks bitmap/count agreement, reserved metadata, inode layouts, directory records,
unique names, link counts, crosslinks and orphan allocation. It requires a clean
superblock. The acquired upstream kernel ext4 on-disk documentation defines the
shared legacy superblock/group/inode/directory layouts; this implementation's
accepted ext2 subset is narrower.

There is one 4 KiB cache page per ext2 mount. Inode, directory, bitmap and
file reads all use it. Writes reach the device synchronously, then update that
same cache entry. A failed write invalidates the cache and poisons the mount;
later reads, writes and flush cannot claim success. An errored read is never
installed as valid data. There is no independent inode/path content cache and no
unbounded dirty-page queue. A file extension explicitly zeros newly exposed bytes
past the old EOF, including preexisting on-disk garbage in the old final block.
Read-only metadata decoding borrows that page instead of copying full pages onto
the stack. Mount validation retains only 1024 block-bitmap bytes, 16 inode-bitmap
bytes, a 1024-byte allocation witness and compact directory offsets across reads.
A host size check bounds `Ext2` with a borrowed device to 6144 bytes; actual guest
I/O samples additionally check the unchanged 128 KiB guarded boot-stack margin.

Before mutation, ext2 writes and flushes an unclean superblock marker. Successful
write completion is not a persistence claim. Explicit `flush` first flushes all
completed data/metadata writes, writes the clean marker, then flushes that marker.
Failures after the initial marker poison the live mount; an unclean remount is
rejected for external repair. There is no journal, replay, automatic repair or
power-loss atomicity guarantee. ENOSPC and naming conflicts are preflighted before
modifying media. Partial metadata I/O can require repair; P07 owns recovery design.

VFS handles contain mount incarnation plus `(volume, inode, generation)`, not a
pathname. Rename modifies the directory while preserving the inode and every open
handle. A fresh mount rejects old mount handles. Ext2 UUID/inode/generation remain
stable through clean remount. FAT serial and initfs entry identity are scoped to
the mount; they are not globally authenticated volume identities. The immutable
initfs adapter uses the already validated archive and preserves read-only behavior.
Mount prefixes route native paths; permissions, Windows sharing/delete semantics,
Unicode normalization, notifications and file mappings are later contracts.

Host tests use real standard-format sectors and exact I/O adapters, including
cross-block writes, rename/open identity, read coherence, remount, full data space,
zero-filled extension, malformed metadata and short read/write/flush failures.
They also interrupt four metadata-write boundaries after the dirty marker and
require remount rejection. Guest qualification separately reads the formatted
image through VirtIO, writes and patches a 6000-byte file across a 4 KiB boundary,
renames it while handles remain open, flushes and reopens it in another QEMU boot.
The host verifier independently checks the final file and permitted mutation scope.
These tests qualify their observed clean-shutdown profile, not arbitrary physical
drive power-loss behavior or a production filesystem.
