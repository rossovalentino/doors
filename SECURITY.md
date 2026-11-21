# Security and trust boundaries

P00/P01 have no guest security implementation or production qualification. BootInfo
parsing treats input bytes as untrusted. It bounds records and loops, uses checked
arithmetic, validates memory ownership, and forms no references to physical memory.
The caller must supply immutable, readable bytes for the lifetime of the view.
Firmware/page-map authenticity and hardware physical-address limits require P02/P03.
The image inspector validates bounded layouts and permissions; host personality
policy keeps native address halves separate. Import names do not establish API
semantics. See `specs/contracts/image-validation-v1.md` for precise limits.

P02 owns a single-CPU bootstrap address space and checks live permissions/content
identity. It is a development diagnostic kernel: no user isolation, Secure Boot,
trusted entropy or production security claim. Hardware operations are isolated in
reviewed guest modules; portable parsers/host tooling retain unsafe prohibition.
IRQ/exception/descriptor ownership and loader retirement require P03 before broader use.

Host commands execute trusted, locally installed tools using argument arrays. Logs
are written to private local run directories. Local paths and host inventory may
identify a workstation; review/sanitize records before any authorized publication.
Staging rejects traversal/device/alternate-stream paths, links and Windows reparse
points. The runner uses private image/variable copies, disabled network and bounded
process output/time. Source snapshots reject symbolic links. Evidence checking is a local consistency
check, not protection against a hostile process concurrently rewriting the tree.

There is no remote reporting endpoint. Record a reproducible issue privately with
the project owner, the affected revision/snapshot, command, input and actual result.
Do not place credentials, signing keys, private dumps or proprietary observations
in repository sources or general evidence.

