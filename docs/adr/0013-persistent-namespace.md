# ADR 0013: One persistent namespace and configuration owner

Status: implemented for the P07 development profile; guest qualification and
independent review are recorded in the phase ledger, not inferred from this ADR.

The ext2 backend has stable inodes and coherent buffered I/O, but its constrained
root directory cannot directly represent Windows paths, ACLs, sharing state or
registry views. A separate uncoordinated metadata database would permit a crash to
attach old permissions to new contents. P07 therefore places a reserved catalog
and stable object backing files inside the same ext2 partition. Every changed
ext2 block, payload block and catalog block is staged in the one redo transaction
described by ADR 0012. No catalog or configuration acknowledgement establishes
durability until the journal commit succeeds.

`kernel/src/persistence` is the exclusive, allocation-free owner. A caller lends
its state and 16 KiB codec buffer; the journal lends its independent 64-page
working set. This prevents large transient construction values on kernel stacks.
The owner serializes path resolution, ancestor authorization, sharing checks and
publication through an exclusive mutable borrow. P08 may expose this owner over
authenticated IPC; P07 does not claim a running configuration service or Windows
binary security descriptor compatibility.

File identity is a nonwrapping object ID plus the validated ext2 volume/inode/
generation tuple. `obj` followed by sixteen hexadecimal digits identifies the
reserved backing file. Catalog bindings hold lossless UTF-16 names, parent IDs,
ACLs and attributes. Rename changes a binding; it does not reopen the stream by
name. Replacing an open file detaches its binding and retains its stream until
the last handle closes. Delete-pending records surviving a reboot are reclaimed
when the recovered catalog is mounted, and their reclamation joins the next
explicit transaction. Failed filesystem mutation or commit retires the entire
owner: all subsequent reads, settings and registry requests fail until remount.
The controller must discard its staged transaction and mount fresh state.

The security boundary accepts an immutable `Subject` from the unsafe trusted
authentication constructor. Numeric identities and group membership are claims
the caller must already have authenticated. Names never establish identity.
Ordered allow/deny ACE evaluation, inheritable ACEs, requested rights, integrity
checks and the symmetric read/write/delete sharing matrix are enforced. Existing
handles retain their captured grants after ACL changes. Four ACEs and a local
numeric identity domain are deliberate development limits; arbitrary Windows
SIDs, SACL auditing, privilege adjustment and impersonation dispatch remain later
contracts.

Native absolute paths and the C: compatibility view are distinct path types.
`C:\Users` maps to `/users`; `C:\Windows` maps to protected `/compat/Windows`.
The fixed native directory tree reserves the specified system/state roots.
`/system/current` is a protected reserved directory in this phase; atomic system
activation and a populated versioned package view are not claimed. The selected
comparison policy folds ASCII A–Z only and preserves every other UTF-16 code unit.
Unsupported namespaces are rejected; the resolver does not reinterpret UNC,
device, alternate-stream or reparse paths as ordinary local files.

Native settings and registry objects are distinct typed models in the catalog.
Registry machine/user roots are logical authorities, each with explicit 32-/64-bit
views. HKCR binds a read-only user-over-machine Classes view to existing key IDs.
Values, subkeys, deletion and versioned polling/enumeration are implemented; an
open merged key retains its bound identities until reopened. Native settings use
owner-scoped schema 2, validated theme/scale values, defaults, and a bounded
schema-1 migration. Secrets and binary Windows hive interoperability are excluded.

Registry tombstones retain deleted ancestors until child references and open
handles retire. Reclamation proceeds from leaves upward; codec validation permits
deleted ancestry only for deleted descendants. This keeps a child-then-parent
deletion committable in either handle-close order. Windows UNC classification
also precedes ordinary slash conversion/root traversal: mixed separator prefixes
cannot silently enter the local C: namespace. Both cases have public-owner
regressions, with production ext2/journal persistence for retained tombstones.

The resulting implementation is a bounded metadata layer above standard ext2,
not a new general filesystem. Its limits and wire format are specified in
`specs/contracts/persistent-namespace-v1.md`. Host tests use the original real
ext2 fixture and production journal, including physical allocation exhaustion,
failed commit, malformed catalog, stale capabilities, replacement and recovery.
The cumulative gate must additionally verify actual separate QEMU boots, host
inspection of raw ext2/catalog associations, and unchanged unrelated contents.
