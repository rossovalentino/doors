# Contributing

Work on the first incomplete phase in `project/phases.json`. Before implementation,
read its contract, tests and ADRs, then run the relevant baseline. Keep each feature's
code, interface definitions, negative tests and documentation in the same change.
Use `cargo xtask verify` before completing the selected implemented phase.

Put portable wire formats in dependency-free `no_std` libraries. Parse integer
fields explicitly; Rust struct layout is not a serialized format. Privileged code,
host tooling and compatibility interfaces must stay separate. Unsafe project code
is forbidden in the current workspace; future assembly/FFI needs a scoped ADR,
documented invariants and independent tests rather than a workspace-wide exemption.

Reuse `tools/xtask` for repeatable commands. Temporary experiments belong in ignored
output directories and are removed after use. Preserve failed execution evidence.
Do not expand the directory tree before a component has a real implementation.

Finish one phase, update all linked records and request its critic review before
starting the next. Claims must identify exact test scope. A critic can discover
defects; it cannot certify perfect correctness or legal clean-room separation.

Use only permitted public contracts and original implementation. Record third-party
dependencies, build scripts, unsafe boundaries and license notices. Read
`PROVENANCE.md` before acquiring new sources. There is no project redistribution
license yet. Do not publish or choose one implicitly.

