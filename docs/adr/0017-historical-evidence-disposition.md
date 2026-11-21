# ADR 0017: Historical evidence disposition and parallel P12/P13 gate

Status: adopted 23 October 2026 on the owner's decisions 1 and 6
(`docs/design/p12-p13-plan-20261023.md`). This is a qualification-gate change
under specification section 86 ("changes to qualification gates require rationale
and review"); it records the rationale here and in `AGENTS.md` and is subject to
the P12/P13 critic review.

## Context

P00–P11 passed their cumulative gates with qualifying run reports under
`reports/runs/` and critic files under `reports/`. On 22 October 2026 the owner
deleted `reports/runs/` (more than 100 GB) for disk space. On 23 October 2026 the
owner stated that P00–P11 were completed and their reports deleted, and asked that
they be re-run only where strictly necessary. The critic files remain.

`spec::check` opened every passed phase's report, revalidated its gate, logs and
critic binding. With the reports gone it failed at P00, so no cumulative P12 gate
could run at all. Recreating reports under the original run IDs would fabricate
evidence (sections 90 and 143.4: a record referring to a missing artifact is
incomplete, not a durable pass). Re-running twelve historical phases under their
own historical gates would re-qualify superseded source, not the current tree.

## Decision

1. **Explicit, reviewed disposition.** Each of P00–P11 in `project/phases.json`
   carries `evidence_disposition` with `state: "unavailable_owner_confirmed"`,
   `owner_confirmed: true`, `date: "2026-10-23"`, the owner statement, the original
   run ID, the original report digest where the critic recorded one, and this ADR
   as `decision_record`. The ledger's `evidence_report` and `critic_review` keep the
   original identities unchanged.
2. **What `spec::check` still verifies.** For a phase with that exact record and a
   missing report it checks: the record fields above; that the run ID matches the
   ledger's `reports/runs/<id>/report.json` identity; that the critic file exists,
   passed with zero unresolved findings, names the same phase and the same original
   report, records its reviewed inputs and binds the same report digest wherever
   the critic recorded one (every phase except P00; mandatory from P03, matching
   `validate_critic`). It reports these phases as "owner-confirmed unavailable" and
   never opens, recreates or requalifies a report under the original ID.
3. **Strict everywhere else.** A phase without the record still fails when its
   report is missing. A present report is fully revalidated even with the record.
   The record is rejected for the selected phase and for any phase after P11; a new
   disposition needs a new owner decision and a reviewed code change.
4. **The strictly necessary re-run.** The cumulative verify for the selected phase
   (P12, or P13 under decision 6) re-executes every inherited phase's mandatory
   commands (`phase_commands`), all inherited diagnostic guest scenarios
   (`scenarios::cases`, 42 today), the ordinary session and the P11 desktop
   workflows on current source. That run, its critic and its report are the current
   evidence; the historical records are identity only.
5. **Parallel P12/P13 (decision 6).** P13 is `in_progress` with an owner-confirmed
   `parallel_authorization` naming P12. `validate_active_phase` accepts later
   phases only as a gap-free chain of authorized `in_progress`/`blocked` phases with
   implemented schemas; none may pass before the selected phase.
   `cargo xtask verify` certifies P12; `cargo xtask verify --phase P13` certifies P13
   and includes every P12 command, guest scenario, desktop workflow and acceptance
   suite.
6. **Acceptance suites fail closed.** P12 (TEXT-01, A11Y-01, DISPLAY-01, MOTION-01,
   component gallery) and P13 (WIN-GUI F1/F2/F3, scenarios 141.5 and 141.6) suites
   are declared in `tools/xtask/src/scenarios.rs`. A suite without a registered
   runner fails verify before any build with "acceptance suite not implemented",
   and a current P12/P13 report without a bound passing check for every suite fails
   the phase gate.

## Aging (section 90)

The historical P00–P11 results are explicitly aged: they describe the sources
reviewed by their critics, not the current tree, and they can no longer be
re-inspected. No current capability claim rests on them. Current claims rest on the
cumulative P12/P13 verify reports and critics.

## Consequences

`check-spec` passes again with P00–P11 reported as owner-confirmed unavailable,
provided the specification, required paths and dependency lock still match. The
integration checkpoint and bounded P10 historical qualification remain recorded as
historical identities with missing reports; they are not gate inputs. If the
original reports are ever restored, full revalidation applies automatically.

## Amendment (24 October 2026)

- The cumulative P12 verify also runs the 18 existing desktop scenarios outside
  `desktop::verify_all` (`scenarios::DESKTOP_SCENARIOS`), each in its own child run
  whose report digest is bound by a `p12-desktop-scenario` check; a current P12/P13
  report without one passing check per scenario fails the phase gate. A coverage
  test fails when a `desktop::*_test` scenario is in neither list.
- P12 adds guest-target clippy and named host test commands for the toolkit,
  accessibility, gallery, text, keyboard-layout, IME and locale crates and the
  window-server text input, clipboard and drag-and-drop tests (`phase_commands`).
- `cargo xtask verify --list [--phase P13]` prints the gate composition without
  building or running anything.
- The owner directed "do P12 first": P13 keeps its parallel authorization and
  schema but is not pursued, and plain `cargo xtask verify` selects P12. The
  disposition rules above are unchanged.
