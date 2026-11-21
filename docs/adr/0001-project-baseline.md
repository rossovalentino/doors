# ADR 0001: bounded P00 foundation

Status: adopted for P00. Owner: Door OS project engineering.

The workspace was empty and the user requested phase-by-phase development with
integrated docs/tests and critic review. Adopt the master specification's x86-64
UEFI, original Rust modular hybrid kernel, ELF native images, PE Windows boundaries
and independent native ABI. No OS behavior exists yet. The architecture defaults
in section 5 remain authoritative for later contracts.

Create only the P00 host tool and portable BootInfo crate. Use
`crates/boot-protocol` from sections 2/131 rather than the future `libs/` example in
section 85. This avoids a needless relocation during bootstrap. Package names are
`door-boot-protocol` and `door-xtask`; `cargo xtask` is the discoverable command
alias, replacing the document's illustrative `-p xtask` spelling.

Pin installed Rust 1.94.1 with its bundled LLVM/LLD and both installed guest targets.
Host binaries use the installed MSVC linker; its actual banner is recorded and
pinned in the build profile. No nightly/custom target or unacquired emulator pin
is invented. JSON is the canonical machine-record format so one reviewed host
serializer suffices; Markdown provides human navigation. YAML examples in the
specification are data-model references, not an additional runtime dependency.

BootInfo v1 defines only the envelope and revision-1 memory table in P00. Other
table kinds remain reserved; unknown optional data is range-checked and skipped,
while unsupported required tables fail. Use explicit encoders instead of Rust ABI
casts. Normalize memory regions to sorted, disjoint, page-aligned entries. Add the
BootData ownership class to protect the live handoff. Only conventional memory
explicitly marked usable may enter the initial free pool. Reclaim transitions,
module/framebuffer checks and actual handoff execution are P02/P03 work.

Alternative: scaffold all subsystems or implement boot before build/profile gates.
Rejected because this creates untested interfaces and conflicts with one-phase
scope. Alternative: raw struct casts. Rejected because arbitrary alignment and
Rust layout would weaken the public wire contract.

Trust boundary: untrusted immutable boot bytes; host tools remain host-only.
Tradeoff: table overlap checking is quadratic with a hard limit of 64 descriptors;
memory validation is linear over at most 16,384 sorted entries and allocates nothing.
ABI extensions require schema/tests and version review. Paths can be migrated in a
coordinated change when a real additional library makes that useful. Revisit for a
measured firmware limitation, new target, ownership transition or ABI requirement.
Validation: host negative/boundary tests and both target compilation checks.

