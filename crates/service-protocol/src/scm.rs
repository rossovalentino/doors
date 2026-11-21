//! The Service Control Manager (Init hosts it). A service's configuration
//! lives where Windows keeps it, `HKLM\SYSTEM\CurrentControlSet\Services\
//! <name>` (ImagePath, DisplayName, Description, Start, Type,
//! ErrorControl, ObjectName, DependOnService), so advapi32 and `sc` read
//! and write it through the registry; these requests carry what only the
//! manager knows: each service's state, process and pending controls.
//! A name is at most `NAME_BYTES` UTF-8 bytes, compared case-insensitively.

pub const NAME_BYTES: usize = 80;
/// The first and last SCM operations (Init routes the range).
pub const FIRST: u32 = 0x7000;
pub const LAST: u32 = 0x700f;

/// Status of one service (body: name): `Status` (`STATUS_BYTES`).
pub const QUERY: u32 = 0x7000;
/// Start a stopped service (body: name). Administrators only.
pub const START: u32 = 0x7001;
/// Send a control (body: `[control u32][name]`): the status after it and
/// the control's sequence number (`[Status][sequence u32]`; 0 for
/// INTERROGATE, which is answered at once).
pub const CONTROL: u32 = 0x7002;
/// Every known service, a page at a time (body: first index u16):
/// `[total u16][count u16]` then per service `[length u8][name][state u8]
/// [pid u32]`.
pub const LIST: u32 = 0x7003;
/// The registry changed for this service (CreateService, DeleteService,
/// ChangeServiceConfig): reread it (body: name). Administrators only.
pub const RELOAD: u32 = 0x7004;
/// From a service process (StartServiceCtrlDispatcher): this process runs
/// service `name`.
pub const REGISTER: u32 = 0x7008;
/// From a service process (SetServiceStatus): `[Status][name]`.
pub const SET_STATUS: u32 = 0x7009;
/// From a service process: the next pending control for `name` as
/// `[control u32][sequence u32]`, or an empty reply when none is queued.
pub const NEXT_CONTROL: u32 = 0x700a;
/// From a service process (body: `[sequence u32][name]`): its control
/// handler returned for that control.
pub const CONTROL_DONE: u32 = 0x700b;
/// Where a control stands (body: `[sequence u32][name]`): `[CONTROL_*
/// u8]`. ControlService waits for the handler as Windows' does.
pub const CONTROL_STATE: u32 = 0x700c;
pub const CONTROL_PENDING: u8 = 0;
pub const CONTROL_DONE_STATE: u8 = 1;
/// The handler did not return within `CONTROL_TIMEOUT_MS` (event 7011;
/// ControlService fails with ERROR_SERVICE_REQUEST_TIMEOUT).
pub const CONTROL_TIMED_OUT: u8 = 2;
pub const CONTROL_TIMEOUT_MS: u32 = 30_000;
/// StartService with arguments (body: `[length u8][arguments][name]`):
/// as START, and the service's process gets the arguments (each
/// NUL-terminated UTF-8, `ARGUMENT_BYTES` at most) after its name in
/// ServiceMain's argv. Administrators only.
pub const START_ARGUMENTS: u32 = 0x700d;
/// From a service's own connected process (body: name): the arguments of
/// the StartService that launched it (`START_ARGUMENTS`), once; empty
/// afterwards or when there were none.
pub const ARGUMENTS: u32 = 0x700e;
pub const ARGUMENT_BYTES: usize = 128;
/// Whether `bytes` are StartService arguments: UTF-8 items each ending in
/// NUL (an empty item is a lone NUL), `ARGUMENT_BYTES` at most.
pub fn arguments_valid(bytes: &[u8]) -> bool {
    bytes.len() <= ARGUMENT_BYTES
        && bytes.last().is_none_or(|&b| b == 0)
        && core::str::from_utf8(bytes).is_ok()
}
/// SC_ACTION_TYPE: what the manager does when a service fails
/// (SERVICE_CONFIG_FAILURE_ACTIONS, the registry's FailureActions).
pub mod action {
    pub const NONE: u32 = 0;
    pub const RESTART: u32 = 1;
    pub const REBOOT: u32 = 2;
    pub const RUN_COMMAND: u32 = 3;
}
/// Services listed per LIST reply.
pub const PAGE: usize = 8;
/// The services Doors itself runs (no registry key; never deleted):
/// (name, display name, description).
pub const BUILTIN: [(&str, &str, &str); 4] = [
    (
        "DoorsStorage",
        "Doors Storage",
        "Stores files, settings and the registry.",
    ),
    (
        "DoorsSession",
        "Doors Session",
        "Signs users in and keeps their credentials.",
    ),
    (
        "DoorsNetwork",
        "Doors Network",
        "Connects this PC to networks and VPNs.",
    ),
    ("DoorsAudio", "Doors Audio", "Plays and records sound."),
];

/// SERVICE_STATUS states.
pub mod state {
    pub const STOPPED: u32 = 1;
    pub const START_PENDING: u32 = 2;
    pub const STOP_PENDING: u32 = 3;
    pub const RUNNING: u32 = 4;
    pub const CONTINUE_PENDING: u32 = 5;
    pub const PAUSE_PENDING: u32 = 6;
    pub const PAUSED: u32 = 7;
}
/// Service controls.
pub mod control {
    pub const STOP: u32 = 1;
    pub const PAUSE: u32 = 2;
    pub const CONTINUE: u32 = 3;
    pub const INTERROGATE: u32 = 4;
    pub const SHUTDOWN: u32 = 5;
}
/// SERVICE_ACCEPT_* bits.
pub mod accept {
    pub const STOP: u32 = 0x1;
    pub const PAUSE_CONTINUE: u32 = 0x2;
    pub const SHUTDOWN: u32 = 0x4;
}

/// SERVICE_STATUS_PROCESS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub kind: u32,
    pub state: u32,
    pub accepted: u32,
    pub exit_code: u32,
    pub specific_exit_code: u32,
    pub checkpoint: u32,
    pub wait_hint: u32,
    pub pid: u32,
    pub flags: u32,
}
pub const STATUS_BYTES: usize = 36;
impl Status {
    pub fn encode(&self) -> [u8; STATUS_BYTES] {
        let mut b = [0u8; STATUS_BYTES];
        for (i, v) in [
            self.kind,
            self.state,
            self.accepted,
            self.exit_code,
            self.specific_exit_code,
            self.checkpoint,
            self.wait_hint,
            self.pid,
            self.flags,
        ]
        .into_iter()
        .enumerate()
        {
            b[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < STATUS_BYTES {
            return None;
        }
        let w = |i: usize| u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap());
        let s = Self {
            kind: w(0),
            state: w(1),
            accepted: w(2),
            exit_code: w(3),
            specific_exit_code: w(4),
            checkpoint: w(5),
            wait_hint: w(6),
            pid: w(7),
            flags: w(8),
        };
        (1..=7).contains(&s.state).then_some(s)
    }
}
/// A service name from a request body: UTF-8, 1..=NAME_BYTES bytes, no
/// slashes or backslashes (Windows refuses them in service names).
pub fn name(b: &[u8]) -> Option<&str> {
    let s = core::str::from_utf8(b).ok()?;
    (!s.is_empty() && s.len() <= NAME_BYTES && !s.contains(['/', '\\']) && !s.contains('\0'))
        .then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn start_arguments() {
        assert!(arguments_valid(b""));
        assert!(arguments_valid(b"alpha\0\0"));
        assert!(!arguments_valid(b"alpha"));
        assert!(!arguments_valid(&[0xff, 0]));
        assert!(!arguments_valid(&[0; ARGUMENT_BYTES + 1]));
        const _: () = assert!(1 + ARGUMENT_BYTES + NAME_BYTES <= crate::wire::BODY_BYTES);
    }
    #[test]
    fn status_round_trips() {
        let s = Status {
            kind: 0x10,
            state: state::RUNNING,
            accepted: accept::STOP,
            pid: 42,
            ..Status::default()
        };
        assert_eq!(Status::decode(&s.encode()), Some(s));
        let mut bad = s.encode();
        bad[4] = 9;
        assert_eq!(Status::decode(&bad), None);
        assert_eq!(name(b"Spooler"), Some("Spooler"));
        assert_eq!(name(b"a\\b"), None);
        assert_eq!(name(b""), None);
    }
}
