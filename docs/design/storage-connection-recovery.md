# Storage connection recovery

P12 finite checkpoint independently accepted within the bounds below.
This repairs client channel lifetime and authenticated endpoint discovery. It
does not remount a poisoned volume or change security session revocation.

A fatal send/receive failure or timeout retires a Connection: take and zero its
handle, clear the pending request, mark it unavailable, then close once. Drop
uses the same path. No request is replayed. A nonblocking unpublished WouldBlock
can be attempted again; synchronous send failure retires the connection.

All Storage operations validate remote replies within a retirement boundary.
Malformed payloads cannot leave a channel live after a decoded successful frame.
Zero file/key/staging tokens, zero metadata object/version, nonzero directory
length, invalid LIST component units and noncanonical error payloads/handles are
malformed. LIST preserves lossless UTF16, including unpaired surrogates. Ordinary
service precondition errors and invalid locally constructed requests do not
retire otherwise healthy connections.

StorageConnect begins a single Init RESOLVE without sleeping. Its poll validates
the authenticated response and transferred port, connects to the pinned Storage
manifest and closes temporary authority. A terminal attempt cannot be reused.
Workspace polls this discovery beside input/render work and backs off100 ticks
after failure. It installs a fresh connection only; it does not repeat the
failed save, delete, rename or preference mutation.

Notes RAM, undo history and separate save attention remain owned by the app.
Files invalidates the prior removal/rename intent, cancels held capture, and
requires a fresh listing before another deletion. An existing deletion notice
remains a one-button recovery notice. Acknowledged mutations remain complete
even if subsequent refresh fails. Cached appearance lives outside Window
recreation so an unavailable Storage provider cannot repeatedly create Windows
while trying to reread the theme.

The private native SDK exercise explicitly retires64 channels while holding
file, registry and replacement-staging tokens. Each fresh asynchronously resolved
connection rejects all stale tokens, reopens exclusively, checks unchanged
metadata and4096 file bytes, and proves bounded broker capacity can be reclaimed.
The read-only existing-open probe may wait up to100 ticks on a definite sharing
refusal while the broker scans older closed slots; no mutation is retried.
A separate read-only boot verifies whole-disk equality to the fresh baseline.
New raw evidence for this exercise goes directly under reports/runs; existing
QEMU private image layout and historical receipts are unchanged. Run:

`cargo xtask test storage-connect-sdk --image <current-core-image.json> --baseline <fresh-services-data.raw>`

Host cases exercise real envelope/body parsers, retirement/no-replay state,
nonblocking backpressure, stale observation and lossless name validation. Native
host/GUI build checks do not establish injected GUI failure behavior. Actual
malformed replies, channel timeout and callback input interleaving still need a
private diagnostic image before making those native claims. Existing synchronous
Storage operations can still wait up to500 ticks; this batch makes discovery
pollable, not all app I/O asynchronous. Init/Window connection recreation still
uses existing paths. Storage death can revoke Session and terminate consumers by
P08 contract; retaining document RAM across that security event is not promised.

Full desktop, foundation, SDK/reboot/byte audits and independent review passed.
P12 remains partial. Cumulative verification stops at the original missing P00
report; later historical dependencies were not thereby validated.

## Accepted evidence

Independent read-only review is recorded in
`reports/critics/p12-storage-connect-critic.json` (SHA256
`e65a6ff56f9cc105e16d3647e1b1859d92104116ccf8aab999a53f29f4b3357b`).
The qualifying source is
`021b83a6619bac9b37703c910a8c09a04bfebb3eb8df62103e01225110aeb9c7`.

| Evidence | Report under reports/runs | Result |
| --- | --- | --- |
| Host | host-1792561439334369300-21252/report.json |651 tests /33 executables |
| SDK | host-1792561159414076200-24192/report.json |64 explicit retirement cycles /2 actual boots |
| Desktop | host-1792561538750199300-21160/report.json |253 desktop +2 maintenance checks /9 sessions |
| Foundation | host-1792562620715950700-17280/report.json |42 expected scenarios +2 interactive runs |
| Cumulative | host-1792562596884006400-23072/report.json |Stops at missing original P00; later historical dependencies not validated |

The SDK executed source016325ab3cf2d7552e7d6adefbece320b1915ebf483537f7dd763844676fdb0f.
Only Workspace's status-strip background color differs from the final source.
Its core image came from development build26936/source297600f3936740d17fc2caeaba08f81b89585c532589cbd8867c34252cd00f73;
the intervening changes were Workspace UI and host reconnect-harness validation.
Both fresh foundation builds reproduce all16 SDK core artifacts exactly.
Neither deliberately stopped development desktop run is promoted to a pass.
The critic independently checked authenticated PID/UUID/argv and marker order,
whole-disk equality, catalog/journal CRCs and source/artifact bindings. All SDK
baseline/final disks have SHA256
`5736ae96df86cdfc5ffd8194bb7b5d670268416ba7b8207b8d4c5762ddba6c4d`.

After maintenance, out retains16,758,519,063 logical bytes and all pinned proof.
The owner's20GB limit applies only to out. The8GB conservative admission reserve
now blocks further heavy suites. Implement versioned generated-output retention
before new qualification; do not lower that reserve or delete pinned evidence.
