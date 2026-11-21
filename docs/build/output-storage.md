# Generated output storage

The narrowed desktop v2 producer migration has bounded acceptance; see
[versioned layout](output-layout-v2.md). It preserves the out-only budget and
legacy8GB reservation while retaining new desktop proof directly with its run.
Existing out currently retains16,758,530,855 logical bytes of protected outputs.

The owner requested a20GB limit for `out` only on2026-10-21. `target` and
`reports/runs` are explicitly outside that limit. `cargo xtask cache status`
reports logical file lengths, a conservative accounting measure for compressed
files. `cargo xtask cache maintain` performs the same cleanup used automatically
before and after output-producing commands.

Only completed Cargo intermediate directories (`deps`, `build`, `.fingerprint`,
`incremental`, `examples` below a build's release/debug directory) are candidates.
Nested retained JSON references veto removal, including absolute Windows paths,
map records and arrays. Invalid evidence vetoes cleanup. Final artifacts, maps,
images, private execution evidence, acquisitions and the entire `out/session`
directory remain protected. Cleanup never formats, tests against or deletes the
owner volume. Reports and failed evidence are not deletion candidates.

A Windows exclusive file lease under `target` covers each xtask operation through
report sealing; process exit releases it. Nested foundation/desktop Run ledgers
reuse that in-process lease. Starting a second xtask receives a busy error, rather
than running cleanup against a live builder. Read-only cache status needs no lease.
Active QEMU/rustc or an uncertain process-tree cleanup prevents cache removal.
Session stop/send, disk inspection and host-only tests do not require new-output
admission space. A historical QEMU handoff can be cold even when its original
receipt remains `running`. Only a complete clean handoff of the exact hash-verified
pinned emulator is eligible for that exception; actual QEMU/rustc quiescence is
checked again immediately before deletion. This is cache eligibility, not a claim
that the historical guest succeeded. Unknown handoffs remain protected.

All JSON inspected by cleanup, including completion reports and output metadata,
has a64MiB input bound. The original complete desktop run is validated before and
after actual cache removal; its original report/source and artifact hashes remain
immutable. Any later host-only maintenance change needs an explicit source bridge.

Admission reserves8GB below the20GB ceiling. External command polling checks a
conservative18GB high-water mark once per second and stops controlled writers
when headroom is exhausted, retaining a failed receipt. Live accounting tolerates
files disappearing during ordinary Cargo renames; destructive cleanup is strict.
Before/after maintenance receipts retain byte counts and exact removed paths.

This is a managed tooling budget, not an NTFS quota. An unrelated writer or a
single uncontrolled burst can exceed the threshold between observations; the
receipt distinguishes an already-exceeded budget. Protected evidence grows over
time and can exhaust admission space. The tool then refuses a new run rather than
deleting proof or the owner disk. Future evidence producers/validators should put
immutable execution data under `reports/runs` directly; old paths must not be
silently moved or historical report bytes rewritten.

# Evidence availability after owner cleanup

After the bounded Files deletion review, the owner confirmed removing `out` to
reclaim space. Historical accepted report/critic bytes remain unchanged, but
their references to deleted builds/images/private serials/NVRAM are unavailable.
Those reports describe completed historical executions; they no longer provide a
self-contained retained revalidation set. See
`reports/artifacts/p12-output-availability.json`. Do not recreate old execution receipts or
reuse old run IDs to fill these gaps.

Pinned OVMF code, variable template and notices were restored from the installed
QEMU distribution only after exact SHA256 comparison against the acquisition
record. New builds and guest runs use new IDs. The separate, older missing P00
report still prevents cumulative phase qualification; P12 remains partial.

# Accepted bounded checkpoint

Independent read-only review accepted this finite policy with no unresolved
blocking finding. The root recorded the attributed verdict in
`reports/critics/p12-output-budget-critic.json` (SHA256
`e66ed0f6a36dcdd310e804336b037bf2ab84957a3962479bb7407d6d57874e31`).
P12 is still partial. Final qualifying host source is
`e4aec8720473ef1887dc109ed301c38e7db781ef9308761323810870adf1b197`.

| Evidence | Run suffix | Report SHA256 |
|---|---|---|
|642 host tests /33 executables|1792558594254806500-23988|9c6d23da0202bc2055d249487ce3992515e2e49564778f07d0145b119905baa1|
|253 desktop checks /9 native sessions +2 maintenance checks|1792556942691906000-13440|b2b5b29ad90014fc5e63f9b7ee7a73913290563bdf7365ed63be2f2da1a4a767|
|Retained validation before cleanup|1792558273167052000-22792|d4b7a0c3ab1ca4e1680859b424a5a8a6dd7bf2e78448f415a8b81265e60eff36|
|Actual cleanup|1792558388445482000-7984|d5568847a6b986815b0ea987763c4f1f4760e554d47e5674682c591b7d91d35e|
|Retained validation after cleanup|1792558466004416300-26920|7f7017c3a48f1bdea85e1de4e6d9fd6a2276f9f287a48c73c88915a08c07ba8b|
|42 foundation outcomes plus2 interactive checks|1792558705044300700-24104|792cb3a68677e2b92e880a333d220c01ed26f0ac0a4475983448b6341248d195|
|Cumulative verifier: original missing P00 only|1792559577791439900-20784|34a6c4bc3b938f5c4e5965c196967aa330d44e91869eaa774c95eec73668d4dd|

Each suffix identifies `reports/runs/host-<suffix>/report.json`. All successful
reports record unchanged inputs. Desktop execution source was
`d00548506785a45f3426c472becdc55117da0d60cb0121d73536282dae6fe152`;
the explicit bridge to e4 changes only this document, `tools/xtask/src/cache.rs`
and `tools/xtask/src/main.rs`. All21 guest artifacts match the previously
accepted product. Full retained validation before/after actual pruning produced
the identical checkpoint SHA256
`3562743432f1af3b1f89ee68ec4d6fd586390e7cd5f1a41e70c0521f5bfbe7ac`.

The first cleanup removed40 directories/891 files/205,694,692 logical bytes.
Foundation automatic cleanup removed40 directories/203,062,560 logical bytes.
Out finished at8,100,456,059 logical bytes. Final binaries, maps, import libraries,
images and private evidence survive and were independently rehashed. The18GB
writer threshold was not stress-tested to exhaustion in these executions.

The saved critic and coordinated status/source metadata were added after
qualification. This is an explicit documentation/metadata-only bridge, not a new
guest execution. No runtime changes belong in that bridge. Historical execution
and acceptance records remain immutable. Next work is Storage transport recovery;
future execution-evidence layout changes need an explicit versioned producer and
validator contract before repeated suites consume the remaining out headroom.
