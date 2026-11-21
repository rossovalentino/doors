# ADR 0020: P14 complete desktop, trusted sign-in and visual fidelity

Status: adopted 26 October 2026 (P14 workstream C). Contract:
`specs/contracts/desktop-shell-v1.md`. Plan: `docs/design/p14-plan-20261026.md`.
Subject to the P14 critic review.

## Context

P14 (specification :2649) closes the desktop: shell, taskbar and launcher,
files, settings and terminal backed by real services (DESKTOP-A :4221), trusted
logon, shell recovery (RECOVER-01 :4296, GUI-F :1367) and a frozen Windows
reference with tokens, logo, motion and a comparison baseline (DESKTOP-B :4222,
GUI-G :1368, VIS-01 :4277, sections 111, 112, 137.6 and 140). The sign-in
surface was a passwordless gate, `ENTROPY_STATUS` returned 0, and AGENTS.md
forbids a custom password hash or a deterministic seed as a credential or
entropy provider (P08 rule).

Owner decisions of 26 October 2026: do all of P14; sign-in is a real password
or PIN hashed with Argon2 from a pinned open-source crate (RustCrypto `argon2`,
no_std), seeded from a new kernel entropy source (RDSEED/RDRAND plus TSC-jitter
conditioning) behind the existing `ENTROPY_STATUS` path; Win+L requires it;
reference capture on the owner's PC may open apps and flyouts and send
navigation hotkeys, with no setting changes.

## Decision

1. **Schema.** P14 is an explicit phase in `tools/xtask` (parse, cases,
   `check_id`, serial matching, demo title, `phase_commands >= P14`); unknown
   phases now fail closed from P15. `verify --phase P14` includes every P13 (and
   P12) command, guest scenario, desktop workflow and suite, then six suites in
   one family, `cargo xtask test desktop --suite <name>`: `desktop-a-shell`,
   `recover-01` (WS-A), `desktop-a-files`, `desktop-a-apps` (WS-B),
   `sec-02-trusted`, `vis-01` (WS-C). A suite without a registered runner fails
   the gate before any build. The ledger records P14 in progress under a
   parallel authorization with the selected phase; P14 cannot pass before P12
   and P13.
2. **Entropy.** The kernel detects RDSEED and RDRAND, draws with bounded
   retries and health tests (startup, stuck and repeated words), falls back to
   TSC jitter checked by repetition-count and adaptive-proportion tests, and
   conditions everything with SHA-256 into a hash DRBG that rekeys after each
   read. `ENTROPY_STATUS` returns the real status word (READY, sources, failure
   count, reseed generation) to any caller; the new `ENTROPY_READ` (0x405, 1-64
   bytes) is limited to processes holding the Session or Init capability. The
   QEMU profiles (`qemu64-v1`) expose neither instruction, so the guest uses the
   jitter source.
3. **Credential store.** The session service alone holds one record per local
   user: Argon2id v0x13 (`argon2` 0.5.3, no default features), 4 MiB, three
   passes, one lane, a 16-byte salt from `ENTROPY_READ` (a constant salt is
   refused), a 32-byte tag compared in constant time. The working memory comes
   from the session binary's heap (the manifest heap limit rises from 0 to
   8 MiB) and is wiped after every derivation. Only the session-less window
   server may enroll or verify; Init may read the state; session processes are
   denied. Replies never carry the secret, tag or salt. After five consecutive
   failures the service imposes 30 s, doubling to a 5 min cap, and the gate
   refuses earlier attempts locally too. A damaged record fails closed. The
   record persists in `/state/credentials/local.v1`, a directory the storage
   service must provision for uid 2 (integrity 2, no other entry); until it
   exists the store is volatile and says so (`persisted=0`).
4. **Trusted surface.** The window server owns the gate (`login::Gate`): first
   run is a Windows OOBE-like "Create a PIN" page (confirmation box, "Use a
   password instead"), sign-in and unlock show a password/PIN box with a
   reveal button that works only while held, "The PIN is incorrect" and a
   throttle countdown. Signed-out and locked input reaches only the gate; there
   is no passwordless path, and an unreachable service keeps the gate closed
   and is retried every second. Typing uses the US layout until sign-in.
5. **Reference and tokens.** `profiles/windows-reference/reference-26h2-x64.json`
   follows the 137.6 template with the 15-surface inventory (claimed surfaces,
   explicit exclusions such as the uncapturable secure desktop) on Windows 11
   Pro 26H2, OS build 26300.9711, read from the owner's PC. `door-design-tokens`
   is the token registry (colour, type, spacing, radius, elevation, motion;
   light, dark and high contrast; provenance per token); applications migrate
   to it through their owners. The door mark gains the monochrome mask of
   140.4.
6. **Fidelity.** `tools/xtask/src/fidelity` compares guest captures with the
   profile in five dimensions with thresholds frozen on 26 October 2026
   (geometry 2 logical px, colour CIEDE2000 5, typography and iconography
   1.5 px, motion 50 ms); a changed threshold or reference byte is refused, and
   an unresolved probe fails closed. `vis-01` runs it on the guest.

## Consequences

- Every desktop launch now answers the gate: runners enroll or sign in with a
  test PIN on their private test volumes (`desktop::sign_in_keys`); the owner's
  interactive desktop leaves sign-in to the owner.
- Credentials survive a reboot only after the storage provisioning lands;
  until then a reboot asks for a new PIN, which the records state.
- The P08 workload fixture now expects a READY entropy status and a denied
  `ENTROPY_READ` for ordinary processes. (P15, 2 November 2026: apps of a
  signed-in session may read it too, for TLS keys and archive salts.)
- Host tests cannot qualify the guest: entropy health under TCG/WHPX, the gate
  on screen, throttling and lock isolation need `sec-02-trusted`; the profile
  needs `vis-01` and a reviewed baseline.

## Amendment (31 October 2026): two credential kinds and Settings (P14 batch 1)

- The local account may hold a password, a PIN, or both. The store keeps one
  `DOORCRD1` record per kind in the same file (format v2: one or two records,
  password first); a v1 store is a valid v2 store, so existing credentials
  sign in unchanged. The session service replaces the store atomically
  (`replace_checked`) after the first write, and wipes candidate tags and
  replaced records.
- New operations CHANGE, ADD and REMOVE (0x4013-0x4015) verify a current
  secret before changing anything (the same kind for CHANGE, the other kind
  for ADD and REMOVE), count failures against the one shared throttle, and
  never remove the last credential. A second Argon2 derivation (CHANGE, ADD)
  starts only if the request has used at most 1.5 s, inside the 5 s reply
  deadline.
- Besides the sign-in surface and Init, the Settings app of the logged-on
  session (workspace manifest; the session id, epoch and uid the service
  learned at LOGON) may read the state and use the three updates; it never
  enrolls or verifies.
- With both kinds the gate asks for the PIN and offers "Sign-in options" to
  switch to the password, at sign-in and unlock alike; with one kind nothing
  changes. Contract: `desktop-shell-v1` section 4.
