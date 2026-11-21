# Storage redo journal v1

Owner: `kernel/src/storage/journal.rs`; host formatting and independent inspection
belong to `tools/xtask`. This original bounded transaction layer surrounds the
existing standard ext2 development partition. It is neither JBD/JBD2 nor an ext4
journal, and it does not change ext2's supported feature bitmap. The release
target remains a future constrained ext4 implementation with its own qualification.

The selected P07 image uses 512-byte sectors. The ext2 home partition starts at
LBA 133120 and contains 65536 sectors. The journal partition starts at LBA 202752 and
contains 4096 sectors. Their original identifiers are the following exact 16 bytes:

| Identity | Bytes |
| --- | --- |
| Disk GUID | `DoorP07DiskGUID0` |
| Home partition GUID | `DoorP07ExtGUID00` |
| Journal partition GUID | `DoorP07LogGUID00` |
| Journal partition type | `DoorRedoJournal1` |

These fixture identifiers do not authorize formatting a physical disk. The host
formatter initializes only an explicitly owned fresh image. `Journal::open` never
formats, clears corruption or substitutes an empty store. Its caller obtains the
home and journal partitions from validated GPT and selects them by their declared
types/identities. The controller checks distinct nonzero identities, device and
partition bounds, non-overlap, write access, sector size and flush support.

## Ownership and API

`WorkingSet::new()` is a const initializer for caller-owned static guest storage.
It contains 64 home pages, two header scratch pages and bounded target/checksum
arrays, approximately 265 KiB. It must not be constructed on the kernel stack.
`Journal` holds an exclusive mutable borrow of that storage and owns its device;
it does not copy the working set, allocate, wait for a lock or start background I/O.
The trusted caller serializes all access and retains device/DMA lifetime.

`Journal::open(device, home, log, disk_id, work)` validates and recovers before
returning `(Journal, RecoveryReport)`. The report contains `generation`, `pages`,
`replayed` and `repaired_header`. A clean checkpoint reports zero pages. A replay
reports the exact committed page count; a second open of the completed recovery
reports no replay. `begin` admits one transaction; a second begin returns `Busy`.
Generation exhaustion returns `NoSpace`, with no wrap or reuse.

The `BlockDevice` view addresses only the home partition. Reads consult staged
pages before home storage. Exact writes update an exclusive RAM overlay, including
sector-sized updates inside a 4 KiB page. A request needing more than 64 distinct
pages returns `NoSpace` before changing any staged bytes. All missing pages are
read before any already-staged page is changed. A failed read poisons the controller.
The view's `flush` is explicitly a staging barrier, not durable completion.

Begin before mounting ext2 on the journal. Flush and end the filesystem borrow
before explicit `commit`, or use the private ext2/namespace integration that first
flushes the filesystem, then commits and begins its next transaction. Retaining
the ext2 cache is valid only across a successful commit, because the applied home
contents equal its overlay. Abort/recovery require discarding dependent caches and
remounting. No generic untrusted caller may mutate the controller behind a cache.

`abort` discards a healthy unpublished transaction without disk writes. After a
commit starts, any I/O error makes the outcome uncertain and poisons the controller;
abort cannot clear it. Drop all dependent state, then reopen/recover. `into_inner`
rejects active or poisoned state. Dropping a controller does not silently commit.
`active`, `is_poisoned`, `staged_pages` and `generation` expose bounded diagnostics.

## Wire format

Two 4096-byte headers occupy journal-relative sectors 0–7 and 8–15. Payload page `i`
occupies relative sectors `16+8*i` through `23+8*i`, for 0 ≤ i < 64. Unused partition space
is reserved. Payload bytes may remain after checkpointing and are ignored unless
referenced by a validated committed header. All integers are little endian.

| Offset | Bytes | Field |
| --- | ---: | --- |
| 0 | 8 | ASCII `DOORJNL1` |
| 8 | 4 | Version 1 |
| 12 | 4 | Header bytes 4096 |
| 16 | 4 | State: 0 checkpoint, 1 committed |
| 20 | 4 | Page count: 0 for checkpoint, 1–64 for committed |
| 24 | 8 | Generation; a committed record requires a nonzero generation |
| 32 | 16 | Expected disk GUID |
| 48 | 16 | Expected home partition GUID |
| 64 | 16 | Expected journal partition GUID |
| 80 | 8 | Home first LBA |
| 88 | 8 | Home sector count |
| 96 | 8 | Journal first LBA |
| 104 | 8 | Journal sector count |
| 112 | 4 | Logical sector bytes 512 |
| 116 | 4 | Home page bytes 4096 |
| 120 | 4 | CRC32 of all 4096 bytes with this field zeroed |
| 124 | 4 | Reserved zero |
| 128 | 512 | Up to 64 entries: home block `u32`, payload CRC32 `u32` |
| 640 | 3456 | Reserved zero |

Unused entries are zero. Targets are unique and strictly less than the home block
count; target 0 is valid and includes ext2's superblock. Every payload CRC covers
exactly 4096 bytes. Fresh formatted headers are identical generation 0 checkpoints.
The volume binding is the GPT disk/home partition identity; ext2 separately checks
its own UUID and allocation graph at mount. CRC32 detects accidental corruption;
this format does not authenticate maliciously rewritten media or prevent rollback
of a complete disk image.

## Ordered commit and recovery

A nonempty transaction follows these exact durable boundaries:

Before any commit write, compare every staged page against current home storage.
The exclusive controller's previous checkpoint makes that home state durable.
Discard byte-identical pages and retain changed pages in their original staging
order. Comparison uses static header scratch; it adds no page-sized stack buffer.
A failed comparison poisons the controller and publishes no payload or header.

1. Write all payload pages, then flush: `PrepareFlushed`.
2. Write committed header 0 and flush; write header 1 and flush: `CommitFlushed`.
3. Apply the first home page and flush: `FirstHomeFlushed`. Apply remaining home
   pages and flush: `HomeFlushed`.
4. Write checkpoint header 0 and flush; write header 1 and flush:
   `CheckpointFlushed`. The generation remains the committed generation and the
   checkpoint clears count/entries. Only then return successful `CommitReport`.

`commit_with` invokes the callback at these boundaries for actual guest immediate
exit diagnostics. The callback cannot reenter the exclusively borrowed journal.
The first-home flush intentionally persists a changed page, making partial-home
state observable when another changed page remains. An empty transaction performs
no I/O. A transaction containing only unchanged pages performs comparison reads
but no writes or flushes. Both return zero pages without consuming a generation
or invoking checkpoint callbacks.

On open, checksum-invalid headers may be torn copies; two invalid headers fail.
A checksum-valid header with an invalid structure, geometry, identity, count or
target fails even if its peer is valid. Same-generation committed copies must
match exactly. A checkpoint dominates a committed peer at the same generation
because its publication follows the home flush. Different valid generations must
be adjacent, with an older checkpoint and a newer committed record. Otherwise the
controller rejects contradictory state. A sole valid copy remains authoritative.

Before any replay write, validate every target, duplicate, count, reserved field,
identity and checksum, and read/check every referenced payload into the working
set. Then replay all pages, flush home storage and publish matching checkpoints.
Replay can be interrupted and repeated. A corrupt last payload never permits the
first valid payload to be written home.

When a selected checkpoint has a stale committed or invalid peer, repair the peer
and flush it before replacing the selected header. This normalization completes
before permitting payload reuse. It prevents a later torn commit from exposing
an old committed header against new payload. Recovery checkpoint publication uses
the same preserve-the-selected-copy ordering. Every mirror write has its own
flush; successful writes to both copies without an intervening flush are
insufficient under the declared torn-write model.

## Evidence and limits

Host tests use a volatile/stable media model with actual persisted write prefixes,
all five crash boundaries, all commit write/flush failures, interrupted recovery,
repeated replay, contradictory/torn headers, corrupt final payloads, full working
sets and short reads. They assert exact flush ordering and zero home writes before
complete validation. The controller size is checked below 512 bytes; its large
working set is external.
Compaction tests cover mixed changed/unchanged targets, deterministic retained
order, no-change commits after an earlier durable generation, and a final short
comparison read after earlier pages have already moved within the working set.

Guest qualification uses separate QEMU processes on the same private image,
including prepare-only, committed, first-home-flushed, home-flushed and checkpoint
cuts. Independent host decoding verifies the complete recovered home image and
file/security/configuration state. These observations qualify the selected virtual
profile; they do not establish physical drive power-loss behavior, arbitrary
concurrent writers, ext4 interoperability or general filesystem repair. Corruption
is reported for explicit recovery action rather than silently resetting state.
