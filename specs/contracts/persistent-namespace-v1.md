# Persistent namespace and configuration v1

## P14 file operations, Recycle Bin, change subscriptions (2026-10-26)

Development implementation; guest qualification is the `desktop-a-files` suite.
The catalog format is unchanged (no catalog version bump): every P14 state is an
ordinary namespace file or folder, staged in the same redo transaction as the
data it describes, so P07 recovery covers it without new replay rules.

**Engine.** `fs/file-ops` (`door-file-ops`, allocation-free) owns operation
IDs, progress, pause/cancel/resume, conflict decisions and recovery records; the
storage service drives it through `persistence::fileops::OpVolume`, which runs
every data operation with the authenticated user's subject (current ACLs,
sharing and protected-node policy are re-checked on each step) and the records
with the storage manifest's system subject. Opcodes 0x2028..0x202e are
FILEOP_BEGIN/ADD/STEP/QUERY/RESOLVE/CONTROL/LIST (`files::OpProgress` replies).
A step commits once: at most 64 KiB of file data or 16 namespace mutations plus
the updated record; it first commits unrelated staging so one step always fits
the 64-page journal. Copies write a hidden `~$fop<op>.tmp` in the destination
folder and publish it with one rename (replace only for a regular file, chosen
by Replace); same-volume moves are one atomic rename per item; folders onto
existing folders merge; Delete is post-order. Cancel removes the partial item;
finished items stay. Disk full (`NoSpace` without uncertainty) ends the operation
`Failed` after removing the partial item in the same transaction; any I/O
failure poisons the owner as before and the last committed record describes
the last committed data.

**Record `DOORFOP1`** (`/state/machine/fileops/op<16 hex>`, system ACL, at most
8192 bytes, layout in `fs/file-ops/src/plan.rs`): mode, policy, state,
pending decision, counters, depth, per-depth object-ID listing cursors, the
temp-file offset, and the source/destination/relative paths and names, with a
trailing CRC32. Decoding validates every field (enums, depth, flags, source
count, reserved bytes, UTF-8, name rules, a destination outside every source)
before any action; a corrupt record is never replayed and is reported
`Corrupt`. At mount, started operations in Scanning/Running come back Paused,
unstarted ones (setup possibly incomplete, nothing changed) are removed.
Replaying a step is idempotent: the temp file is cut back to the recorded
offset. Records belong to their user (others read NotFound); at most 16 per
user, finished ones pruned first.

**Recycle Bin.** `/users/<uid>/$Recycle.Bin` (hidden attribute 0x02, inherits the
home folder's owner ACL). Recycling is one transaction: rename the item (a whole
tree) to `$R<12 hex id>[.ext]` and create `$I<id>` (`DOORRCY1`: ID, deletion
FILETIME, recursive bytes and items, folder flag, original path, CRC32; layout
in `fs/file-ops/src/recycle.rs`). IDs are max(existing)+1, floored by the
wall-clock second. The limit is 10 % of the volume (at least 1 MiB): larger
items return TooBig (the caller confirms permanent deletion) and the oldest
entries are purged first (record, then tree). Restore recreates missing parent
folders, refuses an occupied name (`Exists`) and removes the record in the same
transaction. Purge/empty are Delete operations. Opcodes 0x202f..0x2033 are
RECYCLE, RECYCLE_LIST, RECYCLE_RESTORE, RECYCLE_PURGE, RECYCLE_USAGE.

**Change subscriptions.** The volume keeps a volatile 32-entry change ring
(`Change::Folder(object)` from every binding/attribute/content change of a
child, `Change::Settings(uid)` from settings and HKCU values, `Settings(0)` for
HKLM). WATCH (0x2034) subscribes to a folder (READ|TRAVERSE) or the caller's
settings, bound to channel/PID/session/epoch (16 service-wide, 4 per channel);
WATCH_POLL (0x2035) reports changed/overflow/gone since the last poll and
re-checks the subscriber's current ancestry and folder rights (revoked rights
drop the watch with Denied); UNWATCH (0x2036) and disconnect release it. The
ring confers no authority and is not persisted.

**Credential folder (WS-C).** In SESSION and production modes every mount
creates `/state/credentials` when missing (fresh volumes and older volumes
alike), owned by the session service (uid 2) with an ACL granting only uid 2.
Storage admits the session service manifest (uid 2, session 0) as a subject.

**Migration.** None is required: older volumes gain `/state/credentials` at
their first P14 session mount and `/state/machine/fileops` / `$Recycle.Bin` on
first use. Older readers see them as ordinary folders and files.

## P12 file timestamps (catalog v4/v5, 2026-10-24)

Each node records created, modified and accessed times as little-endian u64
FILETIME (100 ns since 1601-01-01 UTC, `door_abi::clock`, values <= i64::MAX).
Zero means "not recorded". Created/modified occupy node bytes 208..224 (formerly
reserved zero); accessed is a per-slot u64 table at 15488 (slots 0..32) and, in
the production catalog, 66560 (slots 32..256). Catalog version 4 is the 32-node
layout and 5 the 256-node layout; sizes are unchanged (16384/69632 bytes), so
the catalog stays inside its existing ext2 blocks and every timestamp change is
part of the same redo transaction as the operation that caused it.

Migration: versions 1/3 still decode; their timestamp areas must remain zero and
read as not recorded. No migration write is forced; the next committed mutation
writes v4/v5. Older readers reject v4/v5 as an unsupported version (they never
misinterpret them). Empty slots require a zero access entry; nonzero bytes past
the access tables remain corruption.

Policy (the owner's trusted service supplies the time with `set_clock` before
each request, from `operation::WALL_TIME`; 0 when unavailable): creation stamps
all three times and the parent folder's modified time; content write, truncate
and checked replacement stamp modified and accessed; create, remove and rename
stamp the affected folders' modified time while a renamed item keeps its own
times (NTFS behaviour). Read-only access never updates accessed
(`ACCESS_UPDATES = false`, like NtfsDisableLastAccessUpdate). A mutation while
no clock exists stores 0 instead of keeping a time that would be false. The
16 baseline folders are initialized without a clock. Times are not part of the
object version and confer no authority. Storage METADATA, FILE_INFO and
METADATA_FOR replies are 56 bytes (FileInfo then created/modified/accessed);
LIST replies carry the same 56 bytes before the name length.

Settings 7 `TimeZone` and 8 `RegionalFormat` are per-user HKCU registry DWORDs
(value names `TimeZone`, `RegionalFormat`) validated against the stable ids of
`door_locale::tz::ZONES` (default 16, UTC) and `door_locale::region::REGIONS`
(default 1, en-US). They need no catalog or settings-schema change.

## P12 staged whole-file replacement (development, 2026-10-24)

Opcodes 0x201b..0x201e are STAGE_BEGIN/WRITE/COMMIT/ABORT. BEGIN is the
32-byte checked-replacement request (file token, object, version, length) with
length up to 4,243,456 bytes (the ext2 direct + single-indirect cap); it
returns a nonwrapping staging token. WRITE is file token, staging token, exact
sequential u64 offset and 1..192 bytes; COMMIT/ABORT are the two tokens; COMMIT
replies with the 32-byte FileInfo. Two staging slots exist service-wide, one per
channel/PID/session/epoch/file; exhausted slots return Resources, a duplicate
WouldBlock. Closing the file or the channel aborts its candidate.

BEGIN checks the WRITE grant, exact object/version, a regular non-pending file,
a free node slot and free space for the candidate (data plus indirect block)
before any mutation; refusals, including over-limit lengths (InvalidRequest),
leave the owner usable. It stages a parentless, delete-pending catalog file
node (name `staged`, the target's ACL) with an empty backing `obj<id>`. No path,
listing or checked object lookup reaches it. WRITE appends to that backing;
before each write the owner commits once 24 pages are staged, so any length
fits the 64-page journal. Intermediate commits make only that unreachable node
durable; after a crash the validated remount reclaims it like any pending node.

COMMIT revalidates the grant, object/version, complete length and the backing
size, commits all earlier staging, then performs one transaction: unlink the
old `obj<target>` payload, rename `obj<staged>` to `obj<target>`, rebind the
node (same object ID, version + 1, modified/accessed times), drop the staging
node and commit. Recovery therefore selects the complete old or complete new
file at every journal boundary; replay is idempotent. Every COMMIT attempt
consumes the candidate (a refused one is reclaimed). Any failure after the
swap begins poisons the owner until remount; a failed commit may have reached
the durable marker. ABORT reclaims the node and backing in the current
transaction. Existing handles keep referring to the object and read the new
payload, as with checked replacement. Host tests cover all five crash
boundaries, interrupted staging, abort, refused commits and limits.
`door_workspace::document_save::save` routes every save through checked (both
sizes at most 4 KiB) or staged replacement, with no sibling copy; Notes' older
`.~save` copy-remove-rename path in `notes.rs` is to be replaced by that call.

## P12 checked child removal extension (development)

Opcode0x2017 is exactly three little-endian nonzero u64 values: parent, object,
observed version. The owner authenticates the request and rechecks current
ancestry/TRAVERSE, source DELETE, protected/pending policy, parent binding,
version, compatible delete sharing and all child bindings including pending ones.
IDs confer no authority. Only files and empty folders are admitted; recursive
removal and Recycle Bin/restore are separate. No disk format change follows.

The serialized owner preflights, durably commits earlier staging, rechecks,
stages deletion and commits before returning success. Existing compatible open
handles retain their stream until final close; persisted pending nodes are
reclaimed during validated remount. Every failure after marking removal poisons
live mutation, including a backend NoSpace preflight failure. The original
checked-document/dialog checkpoints remain separate; new native qualification
for this extension is pending. See docs/design/files-deletion.md.

## P12 checked bounded stream replacement extension

Development implementation; qualifying evidence is tracked separately in
docs/design/checked-document-save.md. The on-disk format is unchanged.

Opcodes0x2013..0x2016 are BEGIN/CHUNK/COMMIT/ABORT. BEGIN is exactly32bytes:
little-endian u64 authenticated file token, expected object, expected version,
and candidate length. Tokens/object/version are nonzero; length is at most4096.
Both old and new streams must fit4096. BEGIN returns a nonwrapping u64 candidate
token. Four static slots exist, one per file token; exhausted capacity returns
Resources, duplicate staging WouldBlock.

CHUNK is file token, candidate token and exact sequential u64 offset, followed by
1..192bytes. COMMIT/ABORT are exactly two u64 tokens. Candidates bind to
channel/PID/session/epoch/file. Malformed, foreign, incomplete and out-of-order
requests cannot publish candidate bytes. Close/disconnect discard candidates.
A complete COMMIT consumes its candidate on success or error; an incomplete
attempt leaves it available for upload/abort.

The owner revalidates the live WRITE grant, object/version, regular/non-pending
type and nonwrapping object/catalog counters. Existing captured grants/sharing
retain their contracts. Unrelated staging is committed first; the owner must
remain usable. Then one serialized request writes/truncates, increments version
once and durably commits payload/length/catalog. No request checkpoints midway.
Any error after replacement begins poisons the owner, including capacity failure.
Recovery selects complete old/new bytes; a failed commit may have a durable
marker. Success returns32-byte FileInfo from known committed metadata without
a fallible post-commit read. Atomic namespace creation/SaveAs and larger streams
are excluded.

## P12 checked same-parent rename extension

`rename_child_checked(subject,parent,object,observed_version,name_units)` accepts
nonzero numeric identity hints and1..48 lossless UTF16 units. IDs confer no
authority. The owner rechecks every current parent ancestor's TRAVERSE right,
pending/type/root validity with a capacity bound, source DELETE, parent CREATE,
compatible delete sharing, protected-node policy, exact observed source version
and collision policy before mutation. Dot components are not names. The object
must still be a child of that parent; mismatches return Changed. Exact name
no-ops retain version; actual binding changes increment it without wrapping.

Source versions also advance on actual legacy path/handle rename and ACL/
attribute changes, in addition to content changes. This closes namespace/policy
ABA against an observed entry. Existing handle grants retain their documented
semantics; checked rename uses current authority. No disk format changes.

The core operation remains staged. Storage's opcode0x200f handler performs the
operation and durable journal commit under its one serialized ownership before
returning success. Failure follows the existing sticky-poison/remount contract.
The fixed128-byte wire record has three little-endian u64 IDs/version, a u16
length at24, zero reserved bytes26..32, then48 little-endian UTF16 units with
zero unused tail. Changed is service result15. Unknown/malformed lengths, zero
identity fields, embedded NUL and nonzero reserved/tail bytes are rejected.

This contract describes the P07 development implementation in
`fs/storage-core/src/persistence`. Full Win32 API, NTFS, SID or registry-hive compatibility
is not implied. The selected profile has one serialized owner and one C: mapping.

## Ownership, transactions and failure

`PersistentVolume` borrows caller-owned `State` and a 16384-byte codec buffer.
Initialize only with trusted system authority on a filesystem without an existing
`door.catalog`; missing or corrupt existing metadata never creates defaults.
Before mounting ext2, recover the enclosing journal and begin a transaction.
`flush` writes catalog bytes and stages the clean ext2 superblock. Only specialized
`commit`/`commit_with` on `Ext2<Journal>` acknowledges device durability, commits
all staged pages and begins the next transaction without invalidating live file
or key handles. Successful application leaves the ext2 cache coherent because
the home blocks equal the staged view.

Failed backend mutation, backend read or commit makes the owner sticky-poisoned.
All operations then reject access, including cached settings and registry reads.
Drop the owner/cache and discard its unpublished staging state; recover the disk
and mount new state. Validation failures before mutation (ACL, sharing, invalid
name/type/setting, handle capacity) leave the owner usable. A failed commit may
already have reached the durable marker; recovery decides the complete old/new
state. The final representable journal generation may commit successfully but
retires the owner when no next generation can be reserved. It never reports that
durable commit as failed. No operation rolls a counter back or wraps it.

`into_inner` stages outstanding state; it does not independently commit a journal.
The trusted caller controls the transaction lifetime. File-data changes, namespace
bindings, ACLs, attributes, native settings and registry changes are all covered by
the same commit. A configuration API result before that point means accepted into
the current transaction, not durable acknowledgement.

## Limits and object lifecycle

- 32 namespace records, including 16 protected baseline directories; 16 live file
  handles; 48 UTF-16 units per component; 16 components and 1024 units per path.
- 24 registry keys, 24 values, 16 key handles, 64 bytes per value, eight settings
  owners; registry paths have at most 16 components and 768 units.
- Four ordered ACEs per namespace object; 32 numeric group bits; local numeric
  user identities; integrity levels 0–2. Empty DACL denies requested access.
- ext2 retains 48 KiB regular-file size, one 4 KiB cache page, and its documented
  direct-block/feature limits. The catalog occupies four regular-file blocks.

All handles have private mount/incarnation fields. Close consumes the owned
handle; a handle or directory context from another mount cannot target a reused
object. Handle allocation is preflighted before create or truncate. Dispositions
are CreateNew, OpenExisting, OpenAlways, CreateAlways and TruncateExisting. The
returned Boolean reports creation. Truncation requires write access, reclaims
blocks, zeroes a retained tail and preserves file identity. The low-level ext2
truncate currently supports shrinking; growth uses bounded writes with zero gaps.

For each open pair, requested read/write/delete categories must be subsets of
the other's sharing categories in both directions. A new open of a delete-pending
file fails. Existing handles continue reading/writing. Rename requires a delete
grant, compatible delete sharing and create authority on the destination parent.
Replacement checks target deletion authority and sharing. An open replaced target
retains its original contents while the new name refers to the source. Last close
reclaims ext2 data and inode together with the catalog record. ext2 retains the
unallocated inode's generation field; reuse increments it, never aliases it.
After reboot all process-local opens are gone, so pending deletes are completed
in the mount transaction. Unsupported directory deletion, mappings, byte-range
locks, alternate streams, reparse data and async file I/O are not advertised.

ACL access bits are READ=1, WRITE=2, DELETE=4, TRAVERSE=8, CREATE=16,
CHANGE_ACL=32. Evaluate ACEs in stored order against remaining requested bits:
applicable deny rejects remaining bits, applicable allow removes granted bits,
and complete grant terminates evaluation. A lower integrity subject cannot
write/delete/create/change ACL at a higher level. Directory ACL inheritance copies
only inheritable ACEs. Every traversed ancestor, including before `..`, requires
TRAVERSE. Resolution, authorization and share insertion occur under one owner;
there is no check-path/reopen-path race. Attributes currently support hidden 0x02
and archive 0x20; unknown bits fail explicitly.

## Namespace and paths

Baseline IDs are 1 `/`, 2 `/system`, 3 `/users`, 4 `/state`, 5 `/apps`, 6 `/cache`,
7 `/tmp`, 8 `/volumes`, 9 `/devices`, 10 `/compat`, 11 `/compat/Windows`,
12 `/compat/Windows/System32`, 13 `/system/versions`, 14 `/system/current`,
15 `/state/registry`, 16 `/state/machine`. They are owned by system identity 1.
Their ACL grants system all rights and everyone READ/TRAVERSE, both inheritable.
Ordinary users cannot create compatibility modules or alter native system roots.

Native paths are absolute UTF-8 decoded losslessly to UTF-16 and compared exactly.
Windows paths are lossless UTF-16 arrays, including unpaired surrogates, compared
by ASCII-only case folding. Creation rejects a collision under the Windows policy
even through a native path. No Unicode normalization or locale folding occurs.
The restricted component alphabet rejects control/NUL and `\ / : " * < > ? |`;
native names share this component restriction. Display conversion is not lookup.

DOS absolute, rooted, process-relative and C:-relative forms are supported.
Directory contexts keep process and C: current directories separately and bind
to their mount. Windows directory changes update both; selecting a native path
inside the C: view changes only the process directory. A context cannot select
native system-only objects outside the C: view. C: root `..` clamps to C: root,
and the Users alias traverses back to C: root. Regular DOS components trim trailing
dots/spaces; `.`/`..` are processed while checking traversal authority. Extended
`\\?\C:\...` and scoped NT DOS `\??\C:\...` preserve trailing characters and
require an absolute path; dot traversal in those forms is explicitly unsupported.
UNC, other drives, device paths, streams and reparse traversal fail explicitly.
Before ordinary DOS root parsing, every leading pair of slash/backslash separators
is classified as unsupported UNC. Exact extended prefixes are recognized first
and retain their no-slash-conversion rule. Native repeated leading `/` separators
remain native root syntax; they do not enter the Windows UNC namespace.
Reserved CON/PRN/AUX/NUL and COM/LPT 1–9 including superscript 1/2/3 are rejected.

## Registry and settings

HKLM maps machine-owned keys; mutation requires trusted system identity. HKCU maps
the captured effective identity, never a requested username. All hierarchical
keys inherit their root's scope. The selected 32- and 64-bit views are separate
throughout the registry. HKCR is a read-only merge of bound
HKCU/HKLM `Software\Classes` keys, with user values/subkeys taking precedence.
It binds existing identities when opened; newly introduced overlay roots require
reopening. Writes through HKCR, hive import/export, registry virtualization and
Win32 transaction APIs are unsupported.

Key handles capture READ/WRITE/DELETE grants. Key deletion requires a leaf key;
all values are removed and existing handles return KeyDeleted until close.
Deleted ancestors remain while deleted descendants or open handles reference them.
Closing handles reclaims unreferenced tombstones from leaves upward. A live key
under a deleted ancestor remains corrupt; deleted ancestry still requires matching
root, owner and view and an acyclic identity chain. The entire retained chain can
be committed with unrelated updates, including after its parent handle closes.
Fresh mounts discard process-local opens and all deleted records. Recreating the
same spelling allocates new IDs and cannot revive the old handles.
Default values use an empty name. Types are NONE=0, SZ=1, EXPAND_SZ=2, BINARY=3,
DWORD=4, MULTI_SZ=7 and QWORD=11. SZ/EXPAND_SZ require even byte length and a final
UTF-16 NUL; MULTI_SZ requires a final double NUL; DWORD/QWORD require exactly 4/8
bytes. Expand strings are stored typed data; environment expansion is not claimed.
Value and child-key enumeration require an observed version. A changed version
returns Changed; version capture/check implements race-free polling notification
under the owner. Mutating values or immediate child bindings increments the key
version. Asynchronous notification delivery and subtree filters are later work.

Native settings have an independent schema and owner, with no registry path.
Theme values 0/1/2 default to 0. Display scale (ScalePercent) is 0 = automatic (the default:
the display host hint when present, else the integer heuristic) or 100–300 percent in
steps of 25. Schema 4 (2026-10-23) reads a schema 2/3 scale of 100 (the old default,
which no UI could choose) as 0 = automatic and marks the catalog dirty; other stored
values are kept.
Identity 1 may administer owners; ordinary subjects may access only their own
record. Invalid updates preserve unrelated values. Schema 1 migrates its theme
and supplies scale 0 (automatic); migration marks the catalog dirty and requires explicit
commit. Schema3 adds wallpaper ID0..3 at byte24. Existing schema2 records read with
wallpaper0 and are upgraded in memory without forcing a read-only boot to write.
DesktopAppearance setting ID2 reads/writes `(wallpaper << 1) | dark` in0..7;
its single mutation updates both wallpaper and theme atomically, preserving scale.
Theme/scale IDs0/1 retain their text-session semantics. Schema4 has the schema3 layout;
records are written as schema4, which needs a schema4 reader; unknown future schemas
are rejected. No credentials are stored here.

## Catalog wire format

All integer fields are little endian. Unused records/fields/padding are zero.
CRC32 uses polynomial 0xedb88320 and covers all 16384 bytes with bytes 40–43 zero.
The decoder rejects unknown versions, duplicate identities/names, cycles,
invalid parents, invalid types, inconsistent file IDs, invalid ACLs and padding.

Header: bytes 0–7 `DOORCAT1`; u32 version 1 (4 with timestamps; 3/5 for the 256-node production layout) at 8; u32 size 16384 at 12;
u64 next object ID at 16; u64 next key ID at 24; u64 namespace revision at 32;
u32 CRC at 40; u32 settings schema 2 at 44; zero through byte 127.

32 node records of 224 bytes begin at 128: u64 object/parent at 0/8; u8 kind
(1 file, 2 directory)/pending/name length at 16/17/18; u32 attributes at 20;
u64 content version at 24; u32 ext2 inode/generation at 32/36; 16-byte volume UUID
at 40; 48 UTF-16 name units at 56; u32 ACL owner at 152; u8 integrity/count at
156/157. Four 12-byte ACEs begin at 160: u8 principal type (0 everyone, 1 user,
2 group), deny, inherit, zero; u16 access at 4; zero u16 at 6; u32 principal at 8.
The backing file name is `obj` plus sixteen lowercase hexadecimal object-ID digits.

24 key records of 128 bytes begin at 7296: u64 key/parent at 0/8; u32 owner at 16;
u8 root (1 machine, 2 user), view (32/64), deleted, name length at 20–23;
u64 version at 24; 48 UTF-16 units at 32. A root has parent 0 and empty name.
Machine owner is 0; user owner is the authenticated numeric identity.

24 value records of 192 bytes begin at 10368: u64 key at 0; u8 name length/type at
8/9; u16 data length at 10; 48 UTF-16 name units at 16; up to 64 data bytes at 112.
Eight settings records of 64 bytes begin at 14976: u32 owner/schema/theme/scale at
0/4/8/12; u64 version at16; schema3/4 u32 wallpaper at24 and zero bytes28..64.
Schema1/2 require zero bytes24..64. Bytes15488–16383 are reserved zero.

Host tests cover actual ext2 write/reopen, handle capacity, path corpus, sharing,
ordered ACLs, protected paths, replacement, truncate/unlink, deleted-key lifetime,
merged views, notifications, typed settings, malformed metadata and migration.
Production journal tests cover real full-disk rollback and failed commit. The
guest gate additionally binds raw disk bytes, ACL associations and configuration
values across interrupted commit/recovery boots. Physical power-loss behavior,
concurrent SMP storage and full Windows compatibility remain unqualified.
