# Ordinary text session and file admission

P10 ordinary-session scope; current qualification is recorded in `project/phases.json`.
The production initfs contains `system/session.conf`. Without an explicit
diagnostic fw_cfg selection, the kernel boots protected init/storage/session/
console services and the shell, remains alive, and uses ACPI shutdown/reset.
Diagnostic manifests and intentional faults remain separate regression paths.

`cargo xtask build --profile text-qemu-x64` stages production artifacts.
`cargo xtask session` (also `demo`) opens `out/session/user.raw`, formatting only
when absent. Later launches reopen that same disk. Regression suites create
private volumes under their own run directories. `session stop` verifies the
recorded QMP UUID before stopping that emulator; `shutdown` in the shell is the
normal flush/revoke/service-stop/power-off path.

Trusted init installs only manifest-listed `/system/bin` and `/system/share`
assets from authenticated boot files, comparing existing bytes and checkpointing
bounded writes. User files remain separate. This is a development image upgrade
path, not a general package manager or rollback-safe installer.

The shell exposes paths, enumeration, metadata, directories, text creation and
append, raw file copying, rename/delete, theme/clear, exit status, and file-based
execution. `cat ... > file` preserves bytes. Console display converts invalid
UTF-8 only in bounded chunks. Shell input is US set-1 ASCII, quoted arguments and
simple output redirection; there are no pipelines or arbitrary process-output
redirections. Typed commands are bounded to 2048 bytes, 32 words. Foreground jobs
share the terminal within one development session. A shell-only interrupt poll
queues ordinary typeahead instead of stealing child stdin; the queue is 256
bytes, excess input receives a notice, and Ctrl-C remains examined. Final session
disconnect zeros buffered input. This is not future window focus isolation.
Console serial records contain byte counts and JSON-escaped text; control keys
cannot inject trusted records. Live readers consume only newline-complete records,
validate UTF-8/JSON/counts and retain incomplete tails for the next poll.

Init's authenticated launch endpoint captures caller-readable disk bytes under
the caller's UID, validates the cwd directory, and asks the kernel to execute an
ELF or PE image. The kernel validates launch authority before reading the image,
captures up to 1 MiB in a process-owned arena, preserves exact image identity,
maps a private address space and retains capture until all contexts retire.
Untrusted applications receive zero service/device capability bits. Args and
environment are counted, validated UTF-8 NUL lists, limited to 128/256 bytes.
Cwd authority comes from a transferred Directory grant validated for that UID.
Init retains query/control ownership for at most eight live jobs; clients receive
query/wait handles. Cancellation targets only the authenticated caller's jobs.

Native syscall `PROCESS_EXECUTE=0x304` consumes a 448-byte ExecuteDescriptor:
image pointer/length/format/cwd u64 at 0/8/16/24; argument/environment u16 counts
at 32/34; reserved 36..64; args 64..192; environment 192..448.
Format 1 is ELF, 2 PE. `BOOT_FILE_READ=0x305` is launch-authority-only and copies
up to 4096 bytes from immutable initfs via supervisor capture.
Application broker operations are EXECUTE 0x1110, POWER 0x1111, CANCEL_CHILDREN
0x1112. Power requests require the actual shell PID in the active session.
Console CONTROL 0x3002 applies clear/theme; INTERRUPT 0x3003 is shell-only.

Production namespace schema 3 appends 224 extra node slots to the original
schema-1 layout: 256 nodes, 69632-byte catalog, same checksum algorithm. Existing
schema-1 regression volumes retain their original 32 slots/16384 bytes. The ext2
backend supports 12 direct plus one 1024-entry indirect block, dense files up to
4,243,456 bytes, 128 inodes, one 32 MiB group. Backing files consume inodes; 256
namespace slots do not imply 256 available file inodes. Native sequential writes
checkpoint at most 4 KiB at a time; already committed prefixes survive later
errors. Ordinary synchronous Windows writes checkpoint each bounded call.
The storage owner checkpoints before a new RPC once staging reaches 24 pages,
reserving room for schema-3 catalog and metadata. A single write may extend a
dense file by at most 4 KiB; larger gaps return Resources before mutation.
Explicit truncate growth is split into 4 KiB committed steps and may leave a
committed prefix on failure. Historical transactional fixture modes retain their
explicit commit boundaries. Large ordinary writes do not depend on callers
remembering the journal capacity.
Safe ext2 allocator exhaustion is recoverable: existing reads and deletes remain
available, and freeing space permits new allocation. The namespace checks the
backend's tracked mutation state before treating NoSpace as preflight rejection.
Actual write/flush failures and journal staging failures still poison the mount
until validated remount. Unsupported backends default to uncertainty.

PE admission uses file-supplied application bytes with trusted original system
providers; arbitrary DLL filesystem search is not implemented. The Tiny Basic
source port is a versioned practical application trial, not proof of broad stock
Windows compatibility. Password/entropy providers and a full native Rust std
port remain unsupported development boundaries, separate from this text workflow.
