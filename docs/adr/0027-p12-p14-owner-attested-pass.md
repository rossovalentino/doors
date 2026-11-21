# ADR 0027: P12-P14 recorded as passed on the owner's attestation

Status: adopted 11 November 2026 on the owner's decision of that day. This
is a qualification-gate change under specification section 86 ("changes to
qualification gates require rationale and review"), recorded here, in
`AGENTS.md` and in the ledger.

## Context

P12, P13 and P14 ran under the parallel authorization of ADR 0017 and ADR
0020. On 11 November 2026 the owner stated that P12-P14 passed and that they
deleted that evidence for disk space before this session saw it: "p12-14
passed but i deleted evidence before you saw it cuz there was no space on
pc". The ledger had recorded none of those runs: P12-P14 carry no
`evidence_report` and no critic file, so ADR 0017's disposition (which keeps
each phase's original run ID and critic file as identity) cannot apply.

The ledger lets P15 pass only after P12-P14 have passed, and the owner asked
for the P15-P17 gates to be closed. Re-verifying P12-P14 first would repeat
the inherited work that P15's cumulative verify performs anyway.

## Decision

1. **A separate, weaker disposition.** P12, P13 and P14 carry
   `phase_status: "passed"` with `evidence_disposition` of
   `state: "passed_owner_attested"`, `owner_confirmed: true`,
   `date: "2026-11-11"`, the owner's statement verbatim and this ADR as
   `decision_record`. Their `evidence_report` and `critic_review` stay null:
   no run ID, report or critic is invented, and none may be recreated later
   under a claimed identity.
2. **Bounded.** `spec::check` accepts this state for P12, P13 and P14 only,
   never for the selected phase, and reports them separately from ADR 0017's
   owner-confirmed-unavailable history ("owner-attested without retained
   evidence").
3. **Re-execution, not trust alone.** `cargo xtask verify --phase P15` is
   cumulative: it re-executes every P00-P14 mandatory command, guest scenario,
   desktop workflow and acceptance suite on the current source before the
   P15 commands. A P15 pass therefore also shows that P12-P14's checks pass
   on the code P15 qualifies. A P15 failure in an inherited P12-P14 check is
   treated as a regression to repair before P15 can pass.
4. **One cumulative gate for P15-P17** (amended the same day, owner
   decision). P15 becomes the selected phase (`active_phase`), P16 and P17
   remain in progress under the owner's parallel authorization with it, and
   one `cargo xtask verify --phase P17` closes all three: P15 and P16 may
   cite P17's report (`spec::CUMULATIVE_CITATIONS`, those two only), which
   `spec::check` accepts only when the report contains every mandatory
   command of the citing phase, as the cumulative P17 gate does. The
   report, its logs and the check bound to it are kept; on recording the
   pass P17 becomes the selected phase, so its report is checked against
   the current source.

## Consequences

- The ledger states plainly which passes have retained evidence (P15
  onwards), which have identity without reports (P00-P11, ADR 0017) and which
  rest on the owner's word (P12-P14).
- The only evidence of P12-P14's behaviour on current source is the
  inherited part of each later cumulative report.
- Future gate evidence must be kept until the phase that follows it passes;
  `reports/runs/` cleanups keep every report the ledger names.
