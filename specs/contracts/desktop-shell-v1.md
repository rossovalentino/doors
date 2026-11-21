# Desktop shell, trusted sign-in and fidelity contract v1 (P14)

Status: draft, 26 October 2026. Decision record: ADR 0020. Plan:
`docs/design/p14-plan-20261026.md`. The shell and notification sections
describe WS-A's protocol as written in `crates/service-protocol/src/shell.rs`
and `notifications.rs`; WS-A owns them and this text follows their final
report. The credential, entropy, token and fidelity sections are normative for
WS-C code that exists today.

Specification: P14 row :2649, DESKTOP-A :4221, DESKTOP-B :4222, GUI-F :1367,
GUI-G :1368, GUI-H :1369, sections 60-63 (:1383-:1431), 111/112 (:3815,
:3847), 114 (:3899), 137.6 (:4880), 140 (:5204-:5262), VIS-01 :4277, SEC-02
:4294, RECOVER-01 :4296.

## 1. Gate

`cargo xtask verify --phase P14` runs everything `verify --phase P13` runs,
then the P14 commands (`cargo xtask verify --list --phase P14` prints them)
and six suites, each `cargo xtask test desktop --suite <name> [--image
<production-image.json>]`:

| Suite | Check | Owner | Requirements |
|---|---|---|---|
| desktop-a-shell | p14-desktop-a-shell | WS-A | P14-SHELL, P14-REGISTRY, P14-NOTIFY, DESKTOP-A |
| recover-01 | p14-recover-01 | WS-A | P14-RECOVERY |
| desktop-a-files | p14-desktop-a-files | WS-B | P14-FILEOPS, DESKTOP-A |
| desktop-a-apps | p14-desktop-a-apps | WS-B | P14-SETTINGS, P14-TERMINAL, DESKTOP-A |
| sec-02-trusted | p14-sec-02-trusted | WS-C | P14-LOGON |
| vis-01 | p14-vis-01 | WS-C | P14-REFERENCE, P14-TOKENS, P14-MARK, P14-VIS, DESKTOP-B |

A runner is `fn(&mut Run, image: &str, test_image: &str) -> Result<Value,
String>` registered in `scenarios::RUNNERS` through a `desktop::*_suite`
wrapper (gate owner: WS-C). Its result object must be non-empty and may carry
`reference_comparison`. Until registered, the gate fails before any build.

## 2. Shell protocol `door.shell` (WS-A, window endpoint 0x5300-0x531F)

The desktop shell (`door-desktop-shell`, manifest `DESKTOP_SHELL`) is an
ordinary restartable session process. It enumerates the window server's
authorized top-level windows (`WINDOWS`), follows changes by polling (`POLL`,
`RESYNC` on overflow), publishes taskbar groups and the search catalog from its
registration database (`PUBLISH`, generation-atomic), and asks the server to act
(`ACTION`: activate, minimize, restore, close request, toggle). Any window owner
may set attention and progress on its own top-level window
(`SET_TASKBAR_STATE`). The window server stays authoritative: it validates
every request against its own window list, never calls into the shell and
never holds its lock across a client call (GUI-H), and it draws an ordinary
taskbar from its own list while no shell is connected. A new shell incarnation
replaces the previous role holder (`HELLO`) and rebuilds from `WINDOWS`
(RECOVER-01): applications are not restarted.

## 3. Notifications `door.notifications` (WS-A, Init endpoint 0x1120-0x1129)

Any session process may `POST`; Init records the poster's kernel identity as the
source, keeps a bounded centre (16) per session, and lets only the session's
shell list, clear, activate, mark read and toggle Do not disturb. A poster
withdraws its own notifications and takes its own activations. Network and
audio report unavailable until P15; nothing is fabricated.

## 4. Credentials `door.credential` (WS-C, session service 0x4010-0x4015)

| Opcode | Caller | Body | Reply |
|---|---|---|---|
| STATE 0x4010 | window server (session 0), Init or Settings | empty | `State` (16 bytes) |
| ENROLL 0x4011 | window server (session 0) | `[kind][len][secret]` | `State` |
| VERIFY 0x4012 | window server (session 0) | `[kind][len][secret]` | `Verdict` (16 bytes) |
| CHANGE 0x4013 | window server or Settings | `[kind][cur len][new len][current][new]` | `Outcome` (32 bytes) |
| ADD 0x4014 | window server or Settings | `[kind][cur len][new len][current][new]` | `Outcome` |
| REMOVE 0x4015 | window server or Settings | `[kind][cur len][0][current]` | `Outcome` |

- Kinds: 1 password (4-64 UTF-8 bytes, no controls), 2 PIN (4-16 ASCII
  digits). The local account has a password, a PIN, or both. The service
  re-validates every secret.
- Callers: "window server" is the session-less sign-in surface; "Settings" is
  the workspace manifest (`WORKSPACE`, no capabilities, uid >= 1000) of the
  logged-on session, matched on the session id, epoch and uid the session
  service learned when it created the session at LOGON (cleared at LOGOFF).
  Any other peer, any process of another or revoked session and any
  transferred handle get `Denied`. Settings never enrolls or verifies.
- `State`: byte 0 the kind the surface signs in with (0 none; the PIN
  whenever one is enrolled, as Windows), persisted flag, byte 2 the set of
  enrolled kinds (bit 0 password, bit 1 PIN; 0 is accepted as the original
  single-kind form), failure count, ticks until the next accepted attempt.
  `Verdict`: accepted, rejected (failures, retry_after) or throttled
  (failures, retry_after). `Outcome`: a `Verdict` on the current secret
  followed by the `State` afterwards. None carries secret, tag or salt.
- ENROLL succeeds once, for the first credential (`Exists` afterwards); a
  damaged store answers `Corrupt` to everything (fail closed). VERIFY checks
  the only record whatever kind the secret is framed as, or with two kinds
  the record of the framed kind.
- CHANGE replaces the secret of an enrolled kind, proved with its current
  secret. ADD adds the kind that is not enrolled, proved with the other kind
  (a PIN with the password, a password with the PIN). REMOVE removes a kind,
  proved with the other kind, which stays: the account never ends up without
  a credential. Codes before any check: `InvalidRequest` (framing, or a
  secret outside its policy; clients refuse such input before sending),
  `NotFound` (CHANGE of a kind that is not enrolled, or no other kind to prove
  ADD or REMOVE with), `Exists` (ADD of an enrolled kind), `Corrupt`. Then the
  throttle and the proof: `Throttled` or `Rejected` change nothing and say
  only that the PIN or password is incorrect; `Accepted` applies the update.
  After an accepted proof `Io` (the write failed; nothing changed, the store
  reloads before its next use), `Resources` (entropy) or `WouldBlock` (the
  proof took over 1.5 s, so the second Argon2 derivation of CHANGE or ADD is
  not started inside the 5 s reply deadline; retry) are possible.
- Record: `DOORCRD1`, uid 1001, kind, Argon2id v0x13 m=4096 KiB t=3 p=1,
  16-byte salt from `ENTROPY_READ`, 32-byte tag, FNV-64 check (96 bytes).
  Store format v2: one or two records, at most one per kind, password first
  (96 or 192 bytes). A v1 store (one record) is a valid v2 store, so existing
  stores keep signing in without migration; an empty file (a first enrollment
  interrupted after creating the file) holds no credential.
- Throttle: free attempts 5, then 3,000 ticks (30 s) doubling to 30,000 ticks
  (5 min); owned by the service, shared by VERIFY, CHANGE, ADD and REMOVE and
  by both kinds, mirrored by the gate.
- Persistence: `/state/credentials/local.v1`. **Interface for the storage
  owner (WS-B):** at volume initialization and, for existing volumes, at mount,
  the storage service's system subject creates `/state/credentials` if absent
  with ACL owner uid 2 (the session service), integrity 2, one entry
  `Principal::User(2)` full access, no inherited `Everyone` read. The session
  service creates the first store with `CreateNew`, replaces it afterwards
  with `replace_checked` (one durable payload/length/version transaction, so a
  crash leaves the old or the new credentials) and reads it with
  `OpenExisting`; if `/state/credentials` is absent or denied the store is
  volatile and reports `persisted=0`.

Serial evidence (never contains secret material): `DOOR_P14 SIGNIN
surface=enroll|signin kind=pin|password purpose=signin|unlock persisted=0|1`
(also when "Sign-in options" switches the kind), `surface=unavailable`,
`rejected failures=N retry_ms=M`, `throttled failures=N retry_ms=M
[source=gate]`, `enrolled kind=K persisted=P`, `accepted
purpose=signin|unlock`; the service writes `DOOR_P14 CREDENTIAL enrolled
persisted=P` and `DOOR_P14 CREDENTIAL updated op=change|add|remove
kind=pin|password persisted=P`.

## 5. Trusted sign-in surface (window server)

`door_window::login::Gate` (state machine, host-tested) and `draw_gate`;
`graphics/window-server/src/signin.rs` routes every signed-out and locked key
and pointer event to it (`input.rs`), paints it (`present.rs`) and ticks it each
loop (`main.rs`). Granted while signed out starts the session through Init
(`sign_in`); granted while locked unlocks. Win+L (`desktop.rs lock`) re-arms
the gate for unlock. There is no passwordless path; an unreachable service
keeps the gate closed and is retried each second. Typing uses the US layout.
With a password and a PIN both enrolled, sign-in and unlock ask for the PIN
and offer "Sign-in options" (Tab, then Enter or Space, or a click) with one
tile per kind (PIN, password; Tab or the arrows, Enter or Space); choosing
one wipes the typed secret and reports the new `surface=signin kind=...`
line. With one kind the card, its keys and its markers are unchanged.
Runners answer the gate with `desktop::sign_in_keys` and the test PIN of their
private volume; WS-A removes the now-bypassed `lock_key`/`lock_button`/`unlock`
in `desktop.rs`, and the extracted shell must not paint or receive input while
locked.

## 6. Kernel entropy (WS-C)

`ENTROPY_STATUS` (0x402) returns `door_abi::entropy` bits to any caller:
READY 1, RDSEED 2, RDRAND 4, JITTER 8, failures bits 8-15, reseed generation
bits 16-31. `ENTROPY_READ` (0x405, pointer, 1-64 bytes) returns conditioned
DRBG bytes to Session- or Init-capability holders (`AccessDenied` otherwise,
`Unsupported` while not READY).

## 7. Design tokens `door-design-tokens` (WS-C; consumers WS-A, WS-B)

`Theme::{Light, Dark, HighContrast}`; typed accessors `color(ColorToken,
Theme) -> Color` (premultiplied BGRA), `spacing(SpacingToken) -> u16`,
`radius(RadiusToken) -> u16`, `type_ramp(TypeToken) -> TypeStyle`,
`elevation(ElevationToken, Theme) -> Elevation`, `motion(MotionToken) ->
Motion` and `motion_with(token, reduced_motion)`; registry `tokens()`,
`token(id)`, `lookup(id, theme) -> Option<Value>`; each token carries
`Provenance` (source kind, confidence, measured themes). `desktop/workspace/src/
tokens.rs` and the window-server palettes migrate to these accessors through
their owners; the token values, not local constants, are what VIS-01 measures
against.

## 8. Reference profile and fidelity (WS-C)

`profiles/windows-reference/reference-26h2-x64.json`: Windows 11 Pro 26H2, OS
build 26300.9711, 2560x1600 at 150 %, dark theme; 15 surfaces (6 claimed, 9
excluded with reasons, lock/logon among them). `tools/xtask/src/fidelity`:
`compare`, `measure`, `summary`, `vis_01_suite`; thresholds frozen 26 October
2026 (geometry 2 logical px, colour CIEDE2000 5, typography 1.5 px,
iconography 1.5 px, motion 50 ms). A changed threshold or reference byte is
refused; an unresolved probe fails closed.
