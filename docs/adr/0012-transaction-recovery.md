# ADR0012 — Ordered redo transactions for the development ext2 volume

Status: implemented for P07; qualifying guest evidence and independent acceptance
remain owned by the phase ledger.

P06 established coherent standard-format ext2 reads/writes and clean-shutdown
persistence. Its dirty marker correctly rejected interrupted metadata writes but
could not recover them. P07 needs file contents, names, security metadata and
configuration to recover as one bounded operation without replacing ext2 or
claiming an unimplemented ext4 journal.

The selected development profile adds a separate 2 MiB GPT journal partition.
An original 64-page redo controller wraps the existing ext2 block view. All home
writes stage in a caller-owned static working set. Ext2's existing cache remains
the single buffered filesystem path; the journal is its exclusive transaction
overlay. Overlay flush is a staging barrier. The owning namespace/controller
acknowledges durability only after explicit commit completes.

Before writing the log, commit compares every staged page with the previous
durable home state and removes identical pages while preserving target order.
This avoids logging ext2's dirty-to-clean marker when its final bytes are unchanged
and ensures the first-home crash boundary applies a genuinely changed page.
Comparisons use static scratch. Failure poisons the transaction without publishing
anything; a completely unchanged transaction succeeds without a new generation.

Commit orders payload flush, mirrored committed records, home application/flush
and mirrored checkpoints. Every header copy is individually flushed. Recovery
first validates both record structure and every payload, then applies the complete
transaction. It preserves a valid header while repairing its peer, and repairs a
stale committed peer before allowing payload reuse. Same-generation checkpoints
are authoritative only because they are written after the complete home flush.
The exact wire/API/selection rules are in
[`storage-journal-v1.md`](../../specs/contracts/storage-journal-v1.md).

The limits are intentional: 64 distinct 4 KiB home pages per operation, one exclusive
controller, no background writeback, no generation wrap, no hidden format on mount
failure, and no heap-backed or stack-sized transaction object. Capacity is checked
for a whole block request before mutation. Failed I/O poisons the controller; an
uncertain commit requires fresh recovery, not an in-memory retry or abort that
could conceal durable publication. Dependent filesystem/namespace caches must be
discarded after abort or recovery; successful commits may retain coherent caches.

The crash protocol assumes correctly implemented device flush ordering, but does
not assume a 4 KiB header write is atomic. Host tests persist torn byte prefixes
and interrupt both commit and recovery. Guest crash callbacks include a separately
flushed first-home write, so partial replay is real persisted state. Independent
host decoding and subsequent guest remounts qualify the selected QEMU image. CRC32
and duplicate mirrors detect the declared accidental-corruption model; they do not
authenticate an attacker or establish physical-drive guarantees.

This is a narrow recovery layer for the original ext2 development subset, not a
new general filesystem and not JBD/JBD2. The release-oriented ext4 target remains
separate. A future implementation must replace or migrate this development format
through an explicit reviewed compatibility/recovery plan. No physical installation,
Windows hive interoperability or general fsck repair is implied.
