# Bounded Windows image planning

`door-pe-loader` builds immutable, allocation-free plans for the P09 Windows x64
personality. It reuses `door-image-format::PeImage`; it does not introduce a second
PE parser or execute code on the host. The kernel owns namespace selection, process
authority, reservations, page ownership, final permissions and rollback.

`BundlePlan::new` accepts an explicit catalog containing one console executable
followed by supplied DLLs. Names are case-insensitive ASCII basenames; paths, empty
components and duplicate names are refused. There is no disk search, implicit
download, API-set substitution or generic missing-export stub. All supplied images
are validated, including their unused exports. Import and forwarder dependencies
produce a bounded dependency-first order; module cycles are refused in this profile.

The source bytes remain borrowed and immutable throughout construction. File
offsets, RVAs, selected image bases and user addresses have distinct types. Every
module owns its complete image extent. The run stream covers headers, sections and
inaccessible gaps without releasing holes for another mapping. Headers are RO/NX;
sections are RO/NX, RW/NX or RX. No final writable-executable run is accepted.
File spans specify exact copies, followed by zero-fill within committed runs.

Construction validates every relocation and resolved import before returning the
plan. Fixup callbacks subsequently stream DIR64 values and IAT values without a
large stack-resident array. Callback errors remain distinct from validation errors;
the kernel must retire all partially acquired resources before returning failure.
Every error identifies the source module, an RVA where applicable and a typed
reason. Successful planning alone is not guest execution evidence.

## Initial policy

Limits are four modules, eight sections per module, sixteen import descriptors per
module, 1,024 imported symbols and 4,096 DIR64 fixups per bundle, sixteen forwarder
steps, 4,096 exception entries per module, 16 MiB per file/image and twenty-four total
runs including reserved gaps. The plan itself is smaller than 1 KiB.

Imports support names and ordinals, with an original thunk table or an unbound IAT
fallback. Forwarders support names and decimal ordinals. Generic exports and
imports may resolve to readable code or data, including section zero-fill. Trusted
startup uses `resolve_executable_export` to separately require file-backed RX code
before invoking a bootstrap or callback. Reserved gaps and section padding cannot
be exported. Ordinal holes are valid but do not resolve. Initial
IAT contents must match the original thunk table. IAT extents cannot overlap each
other or relocation targets. DIR64 records are ordered and nonoverlapping; their
original values must point inside their own preferred image extent. Relocation-free
images can load only at their preferred address. ABSOLUTE relocation padding is
ignored within a bounded record budget.

Both original compatibility DLLs use `/NOENTRY`. DLL initialization, TLS, delay or
bound imports, load configuration, CLR, resources/activation manifests, code
integrity requirements, application-container and CFG requirements are refused
explicitly. These policies deliberately limit P09; later phase work must expand
them with matching behavior and tests. Certificate directory extents are checked
by the shared format parser as file offsets; no certificate authentication or trust
claim follows from format validity.

Ordinary x64 `.pdata` and version-one `.xdata` are structurally validated, including
ordered executable ranges, complete padded records, opcode sizes, register classes
and frame metadata. Handler-bearing/chained records and other versions are refused.
P09 does not dispatch or unwind Windows exceptions. Fault containment remains a
kernel responsibility; native Rust unwinding never crosses a Windows ABI boundary.

## Validation

`cargo test -p door-pe-loader` covers mapping/zero-fill/reservations, named and ordinal
imports, dependency and forwarder failures, unsafe names, collisions, relocation
failures, unsupported features, unwind metadata, resource boundaries and callback
failure propagation. A deterministic 20,000-case mutation test checks that accepted
plans keep all runs and writes inside their reservations. These are host contract
tests. The required cumulative P09 gate must separately build real DLLs and the
independent Windows fixture, run the unchanged fixture in Windows and Door, and
test actual kernel rollback and cleanup. Host planning tests do not qualify those
pending integration checks.

Format/metadata references: Microsoft's [PE/COFF specification](https://learn.microsoft.com/en-us/windows/win32/debug/pe-format)
and [x64 exception metadata](https://learn.microsoft.com/en-us/cpp/build/exception-handling-x64?view=msvc-170).

## P10 implementation checkpoint: initial bundle lifecycle

The PE planner now has an explicit DllCallbacks policy. The default Initial policy
retains the P09 restriction. Callback entries must point into file-backed RX bytes;
headers, zero-fill, data and reserved holes cannot become callable. Other unsupported
features, including TLS directories, remain rejected rather than implicitly enabled.

compat/pe/src/lifecycle.rs implements an allocation-free initial bundle transaction
built from the validated dependency order. It excludes the EXE from DLL callbacks,
serializes outstanding callbacks, refuses stale/duplicate completions within the
transaction, attaches dependencies first and detaches in reverse. Mappings remain
owned until cleanup finishes. FALSE attach receives detach before predecessor
cleanup; an attach exception skips that DLL's detach. Detach return values are
ignored. Static attach/termination versus failed-load cleanup distinguishes the
reserved parameter. This is a host-tested policy model, not guest callback execution
or a dynamic module registry. Completion tickets belong to one caller-owned
transaction; they are not kernel authority or cross-transaction capabilities.

Validation: all22 existing PE tests plus5 new lifecycle tests pass; strict Clippy
and x86_64-unknown-none checks pass. Guest callback invocation, independent DLL
fixtures, dynamic identity/refcounts, TLS, threads, memory/handles/waits, unwind,
file/async/CRT integration and the full cumulative P10 gate remain unfinished.
P10 stays in_progress and has no acceptance or final critic claim.

Reference checked: [Microsoft DllMain](https://learn.microsoft.com/en-us/windows/win32/dlls/dllmain),
including FALSE versus exception cleanup and reserved-parameter conventions.
