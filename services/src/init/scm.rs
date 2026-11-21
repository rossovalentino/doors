//! The Service Control Manager's state (host-testable, allocation-free as
//! Init is): each service's configuration as the registry holds it, its
//! SERVICE_STATUS_PROCESS, the process that runs it and the controls
//! waiting for that process. Init starts processes and reports their exits;
//! the manager decides what each request means, with Windows' error codes.
use door_abi::platform::SERVICE_ACCOUNT;
use door_service_protocol::scm::{Status, accept, action, control, state};

/// Win32 errors the manager answers.
pub mod error {
    pub const ACCESS_DENIED: u32 = 5;
    pub const NOT_ENOUGH_MEMORY: u32 = 8;
    pub const INVALID_PARAMETER: u32 = 87;
    pub const DEPENDENT_SERVICES_RUNNING: u32 = 1051;
    pub const INVALID_SERVICE_CONTROL: u32 = 1052;
    pub const SERVICE_REQUEST_TIMEOUT: u32 = 1053;
    pub const SERVICE_ALREADY_RUNNING: u32 = 1056;
    pub const SERVICE_DISABLED: u32 = 1058;
    pub const SERVICE_DOES_NOT_EXIST: u32 = 1060;
    pub const SERVICE_CANNOT_ACCEPT_CTRL: u32 = 1061;
    pub const SERVICE_NOT_ACTIVE: u32 = 1062;
    pub const FAILED_SERVICE_CONTROLLER_CONNECT: u32 = 1063;
    pub const PROCESS_ABORTED: u32 = 1067;
    pub const SERVICE_DEPENDENCY_FAIL: u32 = 1068;
    pub const CIRCULAR_DEPENDENCY: u32 = 1059;
    pub const SERVICE_DEPENDENCY_DELETED: u32 = 1075;
}

/// SERVICE_WIN32_OWN_PROCESS.
pub const OWN_PROCESS: u32 = 0x10;
/// Start types.
pub mod start {
    pub const AUTO: u32 = 2;
    pub const DEMAND: u32 = 3;
    pub const DISABLED: u32 = 4;
}
/// Services the manager holds (the Doors system services included).
pub const SERVICES: usize = 16;
/// Dependencies one service may name.
pub const DEPENDS: usize = 4;
/// Ticks a starting service has to connect, as Windows' 30-second
/// ServicesPipeTimeout.
pub const CONNECT_TICKS: u64 = 3_000;
/// Ticks a control handler has to return (Windows' 30 seconds, event
/// 7011).
pub const CONTROL_TICKS: u64 = 3_000;
/// Ticks per second (Init's clock counts 10 ms).
pub const TICKS_PER_SECOND: u64 = 100;
/// Failure actions a service keeps (FailureActions).
pub const ACTIONS: usize = 4;

/// SERVICE_FAILURE_ACTIONS as the registry's FailureActions holds it:
/// `[reset period u32][0 u32][0 u32][count u32][20 u32]` then `[type u32]
/// [delay ms u32]` per action. The last action repeats for later failures.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Failure {
    /// Seconds without failures after which the count starts over
    /// (u32::MAX: never).
    pub reset: u32,
    pub actions: [(u32, u32); ACTIONS],
    pub count: usize,
    /// FailureActionsOnNonCrashFailures: a service that stops itself with
    /// a non-zero exit code has failed too.
    pub non_crash: bool,
}
impl Failure {
    pub fn decode(data: &[u8]) -> Option<Self> {
        let w = |at: usize| {
            data.get(at..at + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        let count = w(12)? as usize;
        let mut f = Self {
            reset: w(0)?,
            ..Self::default()
        };
        for i in 0..count.min(ACTIONS) {
            f.actions[i] = (w(20 + 8 * i)?, w(24 + 8 * i)?);
            f.count += 1;
        }
        Some(f)
    }
    #[cfg(test)]
    pub fn encode(&self, out: &mut [u8; 20 + 8 * ACTIONS]) -> usize {
        out.fill(0);
        out[0..4].copy_from_slice(&self.reset.to_le_bytes());
        out[12..16].copy_from_slice(&(self.count as u32).to_le_bytes());
        out[16..20].copy_from_slice(&20u32.to_le_bytes());
        for (i, (kind, delay)) in self.actions[..self.count].iter().enumerate() {
            out[20 + 8 * i..24 + 8 * i].copy_from_slice(&kind.to_le_bytes());
            out[24 + 8 * i..28 + 8 * i].copy_from_slice(&delay.to_le_bytes());
        }
        20 + 8 * self.count
    }
    /// The action for failure number `n` (1-based).
    fn action(&self, n: u32) -> Option<(u32, u32)> {
        (self.count > 0).then(|| self.actions[(n as usize).clamp(1, self.count) - 1])
    }
}

/// A fixed-capacity UTF-8 string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Text<const N: usize> {
    bytes: [u8; N],
    len: usize,
}
impl<const N: usize> Default for Text<N> {
    fn default() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }
}
impl<const N: usize> Text<N> {
    /// `text`, cut at a character boundary when it is longer than N bytes.
    pub fn new(text: &str) -> Self {
        let mut end = text.len().min(N);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let mut t = Self::default();
        t.bytes[..end].copy_from_slice(&text.as_bytes()[..end]);
        t.len = end;
        t
    }
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}
impl<const N: usize> core::fmt::Write for Text<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let end = self.len + s.len();
        if end > N {
            return Err(core::fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}
pub type Name = Text<80>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub display: Text<128>,
    /// The program, as a Windows path (ImagePath, expanded).
    pub image: Text<200>,
    pub start: u32,
    pub kind: u32,
    pub depends: [Name; DEPENDS],
    pub depend_count: usize,
    pub failure: Failure,
    /// FailureCommand: the program SC_ACTION_RUN_COMMAND runs.
    pub command: Text<256>,
}
impl Config {
    pub fn depends(&self) -> impl Iterator<Item = &str> {
        self.depends[..self.depend_count].iter().map(Name::as_str)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Service {
    pub name: Name,
    pub config: Config,
    pub status: Status,
    /// A Doors system service: always running, not controllable here.
    pub builtin: bool,
    /// The process running it (from Init's launch), and whether that
    /// process has connected (StartServiceCtrlDispatcher).
    pub process: Option<u64>,
    pub connected: bool,
    /// Queued controls: (control, sequence, ticks when queued).
    controls: [(u32, u32, u64); 4],
    queued: usize,
    /// The control the service's handler is running: (sequence, since).
    handling: Option<(u32, u64)>,
    /// The last sequence handed out, the last one whose handler returned
    /// and the last one that timed out.
    sequence: u32,
    done: u32,
    timed_out: u32,
    pub deleted: bool,
    /// When a start began (ticks), for the connection timeout.
    started: u64,
    /// The user whose start launched it (a recovery restart runs it again
    /// as them).
    pub uid: u32,
    /// Failures counted for its failure actions, and the last one's time.
    pub failures: u32,
    last_failure: u64,
    /// A recovery action (SC_ACTION type) due at this tick.
    recovery: Option<(u32, u64)>,
    /// Started, waiting for its dependencies to run before it launches.
    pub deferred: bool,
    /// The arguments of the StartService that launched it, each
    /// NUL-terminated, until its process takes them.
    arguments: [u8; door_service_protocol::scm::ARGUMENT_BYTES],
    argument_bytes: usize,
}
impl Service {
    fn fresh(name: &str, config: Config) -> Self {
        Self {
            name: Name::new(name),
            config,
            status: Status {
                kind: OWN_PROCESS,
                state: state::STOPPED,
                ..Status::default()
            },
            builtin: false,
            process: None,
            connected: false,
            controls: [(0, 0, 0); 4],
            queued: 0,
            handling: None,
            sequence: 0,
            done: 0,
            timed_out: 0,
            deleted: false,
            started: 0,
            uid: 0,
            failures: 0,
            last_failure: 0,
            recovery: None,
            deferred: false,
            arguments: [0; door_service_protocol::scm::ARGUMENT_BYTES],
            argument_bytes: 0,
        }
    }
}
/// A service process ended without the service stopping cleanly (or
/// stopping with an error when non-crash failures count): Windows' event
/// 7031 (a corrective action follows) or 7034 (none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crash {
    pub name: Name,
    pub display: Text<128>,
    pub failures: u32,
    /// (SC_ACTION type, delay in milliseconds), when one is configured.
    pub action: Option<(u32, u32)>,
}
/// What the manager's clock decides (`expire`). Init is allocation-free:
/// the restart variant stays inline (a transient value).
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expired {
    /// A service that did not connect in time: end its process.
    Terminate(u64),
    /// A recovery restart is due: launch `image` as `uid`.
    Restart {
        name: Name,
        image: Text<200>,
        uid: u32,
    },
    /// A control handler did not return in time (event 7011).
    ControlTimeout { name: Name },
    /// A recovery program is due: run `command` (FailureCommand) as `uid`.
    RunCommand {
        name: Name,
        command: Text<256>,
        uid: u32,
        failures: u32,
    },
    /// Restarting the computer is due.
    Reboot { name: Name },
}

/// What Init must do after a request.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Launch the service's image (a Windows path).
    Launch { image: Text<200> },
    /// Started; it launches once its dependencies run (`next_ready`).
    Deferred,
}

pub struct Manager {
    pub services: [Option<Service>; SERVICES],
}
impl Default for Manager {
    fn default() -> Self {
        Self {
            services: [None; SERVICES],
        }
    }
}

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

impl Manager {
    pub fn iter(&self) -> impl Iterator<Item = &Service> {
        self.services.iter().flatten().filter(|s| !s.deleted)
    }
    pub fn find(&self, name: &str) -> Option<&Service> {
        self.iter().find(|s| same(s.name.as_str(), name))
    }
    fn find_mut(&mut self, name: &str) -> Option<&mut Service> {
        self.services
            .iter_mut()
            .flatten()
            .find(|s| same(s.name.as_str(), name) && !s.deleted)
    }
    fn insert(&mut self, service: Service) -> bool {
        match self.services.iter_mut().find(|s| s.is_none()) {
            Some(slot) => {
                *slot = Some(service);
                true
            }
            None => false,
        }
    }
    /// A Doors system service, running in process `pid`.
    pub fn builtin(&mut self, name: &str, display: &str, pid: u64) {
        for slot in &mut self.services {
            if slot.is_some_and(|s| same(s.name.as_str(), name)) {
                *slot = None;
            }
        }
        let mut s = Service::fresh(
            name,
            Config {
                display: Text::new(display),
                start: start::AUTO,
                kind: OWN_PROCESS,
                ..Config::default()
            },
        );
        s.status = Status {
            kind: OWN_PROCESS,
            state: state::RUNNING,
            accepted: accept::SHUTDOWN,
            pid: pid as u32,
            ..Status::default()
        };
        s.builtin = true;
        s.process = Some(pid);
        s.connected = true;
        self.insert(s);
    }
    /// The registry's configuration of `name` (on boot, RELOAD).
    pub fn configure(&mut self, name: &str, config: Config) -> Result<(), u32> {
        if let Some(s) = self.find_mut(name) {
            if !s.builtin {
                s.config = config;
            }
            return Ok(());
        }
        if self.insert(Service::fresh(name, config)) {
            Ok(())
        } else {
            Err(error::NOT_ENOUGH_MEMORY)
        }
    }
    /// DeleteService: gone now if stopped, else once it stops.
    pub fn delete(&mut self, name: &str) -> Result<(), u32> {
        let s = self.find_mut(name).ok_or(error::SERVICE_DOES_NOT_EXIST)?;
        if s.builtin {
            return Err(error::ACCESS_DENIED);
        }
        s.deleted = true;
        self.sweep();
        Ok(())
    }
    /// Deleted services that have stopped go.
    fn sweep(&mut self) {
        for slot in &mut self.services {
            if slot.is_some_and(|s| s.deleted && s.status.state == state::STOPPED) {
                *slot = None;
            }
        }
    }
    pub fn query(&self, name: &str) -> Result<Status, u32> {
        self.find(name)
            .map(|s| s.status)
            .ok_or(error::SERVICE_DOES_NOT_EXIST)
    }
    /// What StartService of `name` starts, dependencies first, each once
    /// (`name` last): ERROR_CIRCULAR_DEPENDENCY for a cycle,
    /// ERROR_SERVICE_DEPENDENCY_DELETED for a dependency that does not
    /// exist, ERROR_SERVICE_DEPENDENCY_FAIL for a disabled one.
    pub fn start_order(&self, name: &str) -> Result<([Name; SERVICES], usize), u32> {
        let mut order = [Name::default(); SERVICES];
        let mut count = 0;
        // 0 unvisited, 1 on the path, 2 done.
        let mut marks = [0u8; SERVICES];
        self.visit(name, true, &mut marks, &mut order, &mut count)?;
        Ok((order, count))
    }
    fn visit(
        &self,
        name: &str,
        target: bool,
        marks: &mut [u8; SERVICES],
        order: &mut [Name; SERVICES],
        count: &mut usize,
    ) -> Result<(), u32> {
        let Some(index) = self
            .services
            .iter()
            .position(|s| s.is_some_and(|s| !s.deleted && same(s.name.as_str(), name)))
        else {
            return Err(if target {
                error::SERVICE_DOES_NOT_EXIST
            } else {
                error::SERVICE_DEPENDENCY_DELETED
            });
        };
        match marks[index] {
            1 => return Err(error::CIRCULAR_DEPENDENCY),
            2 => return Ok(()),
            _ => (),
        }
        let s = self.services[index].unwrap();
        if !target && s.config.start == start::DISABLED && s.status.state == state::STOPPED {
            return Err(error::SERVICE_DEPENDENCY_FAIL);
        }
        marks[index] = 1;
        for d in s.config.depends() {
            self.visit(d, false, marks, order, count)?;
        }
        marks[index] = 2;
        if *count < SERVICES {
            order[*count] = s.name;
            *count += 1;
        }
        Ok(())
    }
    /// A started service whose dependencies all run now: it launches.
    pub fn next_ready(&mut self, now: u64) -> Option<(Name, Text<200>)> {
        let ready = self.services.iter().flatten().find(|s| {
            s.deferred
                && s.config.depends().all(|d| {
                    self.find(d)
                        .is_some_and(|dep| dep.status.state == state::RUNNING)
                })
        })?;
        let name = ready.name;
        let s = self.find_mut(name.as_str())?;
        s.deferred = false;
        s.started = now;
        Some((name, s.config.image))
    }
    /// StartService of one service whose dependencies were started: Init
    /// launches it now, or once they run (`Deferred`).
    pub fn start(&mut self, name: &str, now: u64) -> Result<Action, u32> {
        let s = *self.find(name).ok_or(error::SERVICE_DOES_NOT_EXIST)?;
        if s.status.state != state::STOPPED {
            return Err(error::SERVICE_ALREADY_RUNNING);
        }
        if s.config.start == start::DISABLED {
            return Err(error::SERVICE_DISABLED);
        }
        let mut waiting = false;
        for d in s.config.depends() {
            match self.find(d) {
                Some(dep) if dep.status.state == state::RUNNING => (),
                Some(dep) if dep.status.state == state::START_PENDING && !dep.deleted => {
                    waiting = true
                }
                Some(_) => return Err(error::SERVICE_DEPENDENCY_FAIL),
                None => return Err(error::SERVICE_DEPENDENCY_DELETED),
            }
        }
        let s = self.find_mut(name).unwrap();
        s.status = Status {
            kind: s.config.kind,
            state: state::START_PENDING,
            wait_hint: 2000,
            ..Status::default()
        };
        s.started = now;
        s.connected = false;
        s.queued = 0;
        s.handling = None;
        // A manual start replaces a pending recovery restart.
        if s.recovery.is_some_and(|(kind, _)| kind == action::RESTART) {
            s.recovery = None;
        }
        s.deferred = waiting;
        if waiting {
            return Ok(Action::Deferred);
        }
        Ok(Action::Launch {
            image: s.config.image,
        })
    }
    /// The user whose start launched `name` (a recovery restart runs it
    /// again as them).
    pub fn started_by(&mut self, name: &str, uid: u32) {
        if let Some(s) = self.find_mut(name) {
            s.uid = uid;
        }
    }
    /// Init launched the service's process (or could not: `pid` None).
    pub fn launched(&mut self, name: &str, pid: Option<u64>, failure: u32) {
        if let Some(s) = self.find_mut(name) {
            match pid {
                Some(pid) => {
                    s.process = Some(pid);
                    s.status.pid = pid as u32;
                }
                None => {
                    s.status = Status {
                        kind: s.config.kind,
                        state: state::STOPPED,
                        exit_code: failure,
                        ..Status::default()
                    };
                }
            }
        }
    }
    /// A service process connects (StartServiceCtrlDispatcher): the
    /// service the manager started it for. An own-process service's
    /// dispatch table name is ignored, as Windows ignores it; the reply
    /// names the service.
    pub fn register(&mut self, pid: u64, name: &str) -> Result<Name, u32> {
        // A process the manager did not start for a service (whatever
        // name it gives, known or not) is not one: Windows'
        // StartServiceCtrlDispatcher fails with 1063.
        let named = self
            .find(name)
            .is_some_and(|s| !s.builtin && s.process == Some(pid));
        let s = if named {
            self.find_mut(name)
        } else {
            self.services.iter_mut().flatten().find(|s| {
                !s.builtin
                    && !s.deleted
                    && s.process == Some(pid)
                    && s.config.kind & OWN_PROCESS != 0
            })
        };
        let Some(s) = s else {
            return Err(error::FAILED_SERVICE_CONTROLLER_CONNECT);
        };
        s.connected = true;
        Ok(s.name)
    }
    /// The arguments the next StartService of `name` passes (validated by
    /// the caller: `wire::arguments_valid`); empty for a plain start.
    pub fn set_arguments(&mut self, name: &str, arguments: &[u8]) -> Result<(), u32> {
        let s = self.find_mut(name).ok_or(error::SERVICE_DOES_NOT_EXIST)?;
        if arguments.len() > s.arguments.len() {
            return Err(error::INVALID_PARAMETER);
        }
        s.arguments[..arguments.len()].copy_from_slice(arguments);
        s.argument_bytes = arguments.len();
        Ok(())
    }
    /// The service's own connected process takes its start arguments
    /// (once) into `out`: their length.
    pub fn take_arguments(&mut self, pid: u64, name: &str, out: &mut [u8]) -> Result<usize, u32> {
        let s = self.find_mut(name).ok_or(error::SERVICE_DOES_NOT_EXIST)?;
        if s.builtin || s.process != Some(pid) || !s.connected {
            return Err(error::ACCESS_DENIED);
        }
        let n = s.argument_bytes.min(out.len());
        out[..n].copy_from_slice(&s.arguments[..n]);
        s.argument_bytes = 0;
        Ok(n)
    }
    /// SetServiceStatus from the service's own process.
    pub fn set_status(&mut self, pid: u64, name: &str, mut status: Status) -> Result<(), u32> {
        let s = self.find_mut(name).ok_or(error::SERVICE_DOES_NOT_EXIST)?;
        if s.builtin || s.process != Some(pid) || !s.connected {
            return Err(error::ACCESS_DENIED);
        }
        status.pid = if status.state == state::STOPPED {
            0
        } else {
            pid as u32
        };
        status.kind = s.config.kind;
        s.status = status;
        Ok(())
    }
    /// ControlService: INTERROGATE answers at once; the others queue for
    /// the service when it accepts them.
    pub fn control(&mut self, name: &str, code: u32, now: u64) -> Result<(Status, u32), u32> {
        if !(control::STOP..=control::SHUTDOWN).contains(&code) && !(128..=255).contains(&code) {
            return Err(error::INVALID_SERVICE_CONTROL);
        }
        // Stopping a service others depend on is refused while they run.
        if code == control::STOP
            && self.iter().any(|d| {
                d.status.state != state::STOPPED && d.config.depends().any(|x| same(x, name))
            })
        {
            return Err(error::DEPENDENT_SERVICES_RUNNING);
        }
        let s = self.find_mut(name).ok_or(error::SERVICE_DOES_NOT_EXIST)?;
        if s.status.state == state::STOPPED {
            return Err(error::SERVICE_NOT_ACTIVE);
        }
        if code == control::INTERROGATE {
            return Ok((s.status, 0));
        }
        let needed = match code {
            control::STOP => accept::STOP,
            control::PAUSE | control::CONTINUE => accept::PAUSE_CONTINUE,
            control::SHUTDOWN => accept::SHUTDOWN,
            _ => 0,
        };
        if s.builtin
            || !s.connected
            || (needed != 0 && s.status.accepted & needed == 0)
            || matches!(s.status.state, state::START_PENDING | state::STOP_PENDING)
            || s.queued == s.controls.len()
        {
            return Err(error::SERVICE_CANNOT_ACCEPT_CTRL);
        }
        s.sequence = s.sequence.wrapping_add(1).max(1);
        s.controls[s.queued] = (code, s.sequence, now);
        s.queued += 1;
        Ok((s.status, s.sequence))
    }
    /// The service process asks for its next control: (control, sequence).
    /// Its handler runs from now until `control_done`.
    pub fn next_control(
        &mut self,
        pid: u64,
        name: &str,
        now: u64,
    ) -> Result<Option<(u32, u32)>, u32> {
        let s = self.find_mut(name).ok_or(error::SERVICE_DOES_NOT_EXIST)?;
        if s.process != Some(pid) {
            return Err(error::ACCESS_DENIED);
        }
        if s.queued == 0 {
            return Ok(None);
        }
        let (code, sequence, _) = s.controls[0];
        s.controls.copy_within(1.., 0);
        s.queued -= 1;
        s.handling = Some((sequence, now));
        Ok(Some((code, sequence)))
    }
    /// The service's handler returned for control `sequence`.
    pub fn control_done(&mut self, pid: u64, name: &str, sequence: u32) -> Result<(), u32> {
        let s = self.find_mut(name).ok_or(error::SERVICE_DOES_NOT_EXIST)?;
        if s.process != Some(pid) {
            return Err(error::ACCESS_DENIED);
        }
        if s.handling.is_some_and(|(q, _)| q == sequence) {
            s.handling = None;
        }
        s.done = s.done.max(sequence);
        Ok(())
    }
    /// Where control `sequence` stands: its handler returned, timed out
    /// (ControlService fails with ERROR_SERVICE_REQUEST_TIMEOUT), or is
    /// still queued or running. A control whose service has stopped is
    /// done.
    pub fn control_state(&self, name: &str, sequence: u32) -> Result<u8, u32> {
        use door_service_protocol::scm::{CONTROL_DONE_STATE, CONTROL_PENDING, CONTROL_TIMED_OUT};
        let s = self.find(name).ok_or(error::SERVICE_DOES_NOT_EXIST)?;
        Ok(
            if sequence <= s.done || s.status.state == state::STOPPED && s.process.is_none() {
                CONTROL_DONE_STATE
            } else if sequence <= s.timed_out {
                CONTROL_TIMED_OUT
            } else {
                CONTROL_PENDING
            },
        )
    }
    /// A process ended: its services stop (with ERROR_PROCESS_ABORTED when
    /// they had not reported STOPPED), and deleted ones go. A failure (an
    /// end without STOPPED, or STOPPED with an error when non-crash
    /// failures count) is counted for the failure actions: a restart is
    /// scheduled after its delay, and the crash is reported for the event
    /// log.
    pub fn exited(&mut self, pid: u64, code: u32, now: u64) -> Option<Crash> {
        let mut crash = None;
        for s in self.services.iter_mut().flatten() {
            if s.process == Some(pid) && !s.builtin {
                let clean = s.status.state == state::STOPPED;
                let exit_code = if clean {
                    s.status.exit_code
                } else if code == 0 {
                    error::PROCESS_ABORTED
                } else {
                    code
                };
                let failed = !clean || (s.config.failure.non_crash && exit_code != 0);
                s.status = Status {
                    kind: s.config.kind,
                    state: state::STOPPED,
                    exit_code,
                    ..Status::default()
                };
                s.process = None;
                s.connected = false;
                s.queued = 0;
                s.handling = None;
                if failed && !s.deleted {
                    let f = s.config.failure;
                    let reset = u64::from(f.reset).saturating_mul(TICKS_PER_SECOND);
                    if f.reset != u32::MAX
                        && s.failures > 0
                        && now.saturating_sub(s.last_failure) > reset
                    {
                        s.failures = 0;
                    }
                    s.failures = s.failures.saturating_add(1);
                    s.last_failure = now;
                    let action = f
                        .action(s.failures)
                        .filter(|(kind, _)| *kind != action::NONE);
                    if let Some((kind, delay)) = action {
                        s.recovery = Some((kind, now + u64::from(delay) / 10));
                    }
                    crash = Some(Crash {
                        name: s.name,
                        display: s.config.display,
                        failures: s.failures,
                        action,
                    });
                }
            }
        }
        self.sweep();
        crash
    }
    /// The manager's clock: a service that did not connect in time stops
    /// with ERROR_SERVICE_REQUEST_TIMEOUT (its process to end); a control
    /// whose handler has not returned (or not been taken) in 30 seconds
    /// times out; a due recovery restart starts. Call until None.
    pub fn expire(&mut self, now: u64) -> Option<Expired> {
        // A service waiting for a dependency that stopped (failed to start)
        // fails with it, as Windows' 1068.
        let stopped_dependency = |m: &Manager, s: &Service| {
            s.config.depends().any(|d| {
                m.find(d)
                    .is_none_or(|dep| dep.status.state == state::STOPPED && !dep.deferred)
            })
        };
        for i in 0..SERVICES {
            let Some(s) = self.services[i] else {
                continue;
            };
            if s.deferred
                && (stopped_dependency(self, &s) || now.saturating_sub(s.started) > CONNECT_TICKS)
            {
                let s = self.services[i].as_mut().unwrap();
                s.deferred = false;
                s.status = Status {
                    kind: s.config.kind,
                    state: state::STOPPED,
                    exit_code: error::SERVICE_DEPENDENCY_FAIL,
                    ..Status::default()
                };
            }
        }
        for s in self.services.iter_mut().flatten() {
            if s.status.state == state::START_PENDING
                && !s.deferred
                && !s.connected
                && now.saturating_sub(s.started) > CONNECT_TICKS
            {
                let process = s.process.take();
                s.status = Status {
                    kind: s.config.kind,
                    state: state::STOPPED,
                    exit_code: error::SERVICE_REQUEST_TIMEOUT,
                    ..Status::default()
                };
                if let Some(pid) = process {
                    return Some(Expired::Terminate(pid));
                }
                continue;
            }
            // The handler running a control, or the oldest control waiting
            // for it, past its 30 seconds: timed out once.
            let oldest = s
                .handling
                .or((s.queued > 0).then(|| (s.controls[0].1, s.controls[0].2)));
            if let Some((sequence, since)) = oldest
                && sequence > s.timed_out
                && now.saturating_sub(since) > CONTROL_TICKS
            {
                s.timed_out = sequence;
                return Some(Expired::ControlTimeout { name: s.name });
            }
            if let Some((kind, at)) = s.recovery
                && now >= at
            {
                s.recovery = None;
                if s.deleted {
                    continue;
                }
                match kind {
                    // The program runs as the service's user, with %1% the
                    // failure count.
                    action::RUN_COMMAND if !s.config.command.is_empty() => {
                        return Some(Expired::RunCommand {
                            name: s.name,
                            command: s.config.command,
                            uid: s.uid,
                            failures: s.failures,
                        });
                    }
                    action::REBOOT => return Some(Expired::Reboot { name: s.name }),
                    action::RESTART if s.status.state == state::STOPPED => (),
                    _ => continue,
                }
                s.status = Status {
                    kind: s.config.kind,
                    state: state::START_PENDING,
                    wait_hint: 2000,
                    ..Status::default()
                };
                s.started = now;
                s.connected = false;
                s.queued = 0;
                s.handling = None;
                return Some(Expired::Restart {
                    name: s.name,
                    image: s.config.image,
                    uid: s.uid,
                });
            }
        }
        None
    }
}

use super::log;
use crate::server::Response;
use door_abi::{Handle, Transfer, platform::Identity};
use door_service_client::{RegistryRoot, RegistryView, Storage};
use door_service_protocol::{Code, scm as wire};

// ---- Init's side: the registry database and the requests ----------------

const SERVICES_KEY: &str = "SYSTEM\\CurrentControlSet\\Services";

/// UTF-16 of `text` (registry paths hold at most 96 units).
fn units(text: &str) -> ([u16; 96], usize) {
    let mut out = [0u16; 96];
    let mut n = 0;
    for u in text.encode_utf16() {
        if n == out.len() {
            return (out, 0);
        }
        out[n] = u;
        n += 1;
    }
    (out, n)
}
/// `%SystemRoot%`/`%windir%` expanded and surrounding quotes removed (the
/// program, not its arguments).
pub(super) fn expand(path: &str) -> Text<200> {
    let mut out = Text::<200>::default();
    let path = path.trim();
    let path = match path.strip_prefix('"') {
        Some(rest) => rest.split('"').next().unwrap_or(rest),
        None => path,
    };
    let mut rest = path;
    while !rest.is_empty() {
        let lower = rest.as_bytes();
        let var = ["%systemroot%", "%windir%"].into_iter().find(|v| {
            lower.len() >= v.len() && lower[..v.len()].eq_ignore_ascii_case(v.as_bytes())
        });
        match var {
            Some(v) => {
                let _ = core::fmt::Write::write_str(&mut out, "C:\\Windows");
                rest = &rest[v.len()..];
            }
            None => {
                let ch = rest.chars().next().unwrap();
                let mut b = [0u8; 4];
                let _ = core::fmt::Write::write_str(&mut out, ch.encode_utf8(&mut b));
                rest = &rest[ch.len_utf8()..];
            }
        }
    }
    out
}
fn utf16_text<const N: usize>(data: &[u8]) -> Text<N> {
    let mut out = Text::<N>::default();
    for c in char::decode_utf16(
        data.chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .take_while(|u| *u != 0),
    ) {
        let mut b = [0u8; 4];
        if core::fmt::Write::write_str(&mut out, c.unwrap_or('\u{fffd}').encode_utf8(&mut b))
            .is_err()
        {
            break;
        }
    }
    out
}
/// The registry's configuration of service `name` (None: no key or no
/// ImagePath).
pub(super) fn read_config(storage: &mut Storage, name: &str) -> Option<Config> {
    let mut path = Text::<200>::default();
    core::fmt::Write::write_fmt(&mut path, format_args!("{SERVICES_KEY}\\{name}")).ok()?;
    let (path, n) = units(path.as_str());
    if n == 0 {
        return None;
    }
    let key = storage
        .registry_open(
            RegistryRoot::LocalMachine,
            RegistryView::Bits64,
            &path[..n],
            1,
            false,
        )
        .ok()?;
    let version = storage.registry_version(&key).unwrap_or(0);
    let mut config = Config {
        start: start::DEMAND,
        kind: OWN_PROCESS,
        display: Text::new(name),
        ..Config::default()
    };
    for i in 0..32 {
        let Ok(e) = storage.registry_enumerate_value(&key, i, version) else {
            break;
        };
        let value: Text<32> = {
            let mut t = Text::default();
            for c in char::decode_utf16(e.name.units().iter().copied()) {
                let mut b = [0u8; 4];
                let _ = core::fmt::Write::write_str(&mut t, c.unwrap_or('?').encode_utf8(&mut b));
            }
            t
        };
        let data = e.value.data();
        let number = data
            .get(..4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        let v = value.as_str();
        if v.eq_ignore_ascii_case("ImagePath") {
            config.image = expand(utf16_text::<200>(data).as_str());
        } else if v.eq_ignore_ascii_case("DisplayName") {
            config.display = utf16_text(data);
        } else if v.eq_ignore_ascii_case("Start") {
            config.start = number.unwrap_or(start::DEMAND);
        } else if v.eq_ignore_ascii_case("Type") {
            config.kind = number.unwrap_or(OWN_PROCESS);
        } else if v.eq_ignore_ascii_case("FailureActions") {
            let non_crash = config.failure.non_crash;
            config.failure = Failure::decode(data).unwrap_or_default();
            config.failure.non_crash = non_crash;
        } else if v.eq_ignore_ascii_case("FailureActionsOnNonCrashFailures") {
            config.failure.non_crash = number.is_some_and(|n| n != 0);
        } else if v.eq_ignore_ascii_case("FailureCommand") {
            config.command = utf16_text(data);
        } else if v.eq_ignore_ascii_case("DependOnService") {
            let mut item = Vec16::default();
            for unit in data
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
            {
                if unit == 0 {
                    if item.len > 0 && config.depend_count < DEPENDS {
                        config.depends[config.depend_count] = item.text();
                        config.depend_count += 1;
                    }
                    item = Vec16::default();
                } else {
                    item.push(unit);
                }
            }
        }
    }
    let _ = storage.registry_close(key);
    (!config.image.is_empty()).then_some(config)
}
/// A dependency name being collected (UTF-16 units).
struct Vec16 {
    units: [u16; 80],
    len: usize,
}
impl Default for Vec16 {
    fn default() -> Self {
        Self {
            units: [0; 80],
            len: 0,
        }
    }
}
impl Vec16 {
    fn push(&mut self, u: u16) {
        if self.len < self.units.len() {
            self.units[self.len] = u;
            self.len += 1;
        }
    }
    fn text(&self) -> Name {
        let mut t = Name::default();
        for c in char::decode_utf16(self.units[..self.len].iter().copied()) {
            let mut b = [0u8; 4];
            let _ = core::fmt::Write::write_str(&mut t, c.unwrap_or('?').encode_utf8(&mut b));
        }
        t
    }
}
/// Every configured service in the registry, into the manager (sign-in).
pub(super) fn load(manager: &mut Manager, storage: &mut Storage) {
    let (path, n) = units(SERVICES_KEY);
    let Ok(key) = storage.registry_open(
        RegistryRoot::LocalMachine,
        RegistryView::Bits64,
        &path[..n],
        1,
        false,
    ) else {
        return;
    };
    let version = storage.registry_version(&key).unwrap_or(0);
    let mut names = [Name::default(); SERVICES];
    let mut count = 0;
    for i in 0..SERVICES as u32 {
        match storage.registry_enumerate_key(&key, i, version) {
            Ok(name) => {
                let mut t = Name::default();
                for c in char::decode_utf16(name.units().iter().copied()) {
                    let mut b = [0u8; 4];
                    let _ =
                        core::fmt::Write::write_str(&mut t, c.unwrap_or('?').encode_utf8(&mut b));
                }
                names[count] = t;
                count += 1;
            }
            Err(_) => break,
        }
    }
    let _ = storage.registry_close(key);
    for name in &names[..count] {
        if let Some(config) = read_config(storage, name.as_str()) {
            let _ = manager.configure(name.as_str(), config);
        }
    }
}
/// A Windows program path as the storage path Init launches (C:\Users\x
/// is /users/x; everything else of C: is /compat).
pub(super) fn native(windows: &str) -> Option<Text<200>> {
    // A storage path already (GetModuleFileName gives `/system/bin/x.exe`).
    if windows.starts_with('/') && !windows.contains("..") {
        let t = Text::<200>::new(windows);
        return (t.as_str().len() == windows.len()).then_some(t);
    }
    let b = windows.as_bytes();
    if b.len() < 3
        || !b[0].eq_ignore_ascii_case(&b'c')
        || b[1] != b':'
        || !matches!(b[2], b'\\' | b'/')
    {
        return None;
    }
    let rest = &windows[2..];
    let mut out = Text::<200>::default();
    let users = rest.len() >= 7 && rest[..7].eq_ignore_ascii_case("\\Users\\");
    let (prefix, tail) = if users {
        ("/users/", &rest[7..])
    } else {
        ("/compat/", &rest[1..])
    };
    core::fmt::Write::write_str(&mut out, prefix).ok()?;
    for ch in tail.chars() {
        let mut b = [0u8; 4];
        let s = if ch == '\\' {
            "/"
        } else {
            ch.encode_utf8(&mut b)
        };
        core::fmt::Write::write_str(&mut out, s).ok()?;
    }
    Some(out)
}

// ---- the requests (door_service_protocol::scm) ---------------------------

fn reply(code: u32, payload: &[u8]) -> Response {
    // The longest payload: a service's start arguments (ARGUMENTS), a
    // name (REGISTER), or a status and a sequence (CONTROL).
    const LONGEST: usize = {
        let a = if wire::NAME_BYTES > wire::STATUS_BYTES + 4 {
            wire::NAME_BYTES
        } else {
            wire::STATUS_BYTES + 4
        };
        if wire::ARGUMENT_BYTES > a {
            wire::ARGUMENT_BYTES
        } else {
            a
        }
    };
    let mut b = [0u8; 4 + LONGEST];
    b[..4].copy_from_slice(&code.to_le_bytes());
    b[4..4 + payload.len()].copy_from_slice(payload);
    Response::bytes(&b[..4 + payload.len()])
}

impl super::Registry {
    /// Every SCM request; replies lead with a Win32 code (0: success).
    /// Session processes ask; START, CONTROL (but INTERROGATE) and RELOAD
    /// need the Administrators group; a service's own process registers,
    /// reports status and fetches its controls.
    pub(super) fn scm_request(
        &mut self,
        peer: Identity,
        operation: u32,
        body: &[u8],
        transfers: &[Transfer],
        now: u64,
    ) -> Response {
        if !self.admits(peer) {
            return Response::error(Code::Denied);
        }
        // START may carry the caller's working-folder authority; services
        // themselves run as the service account, from their own folder.
        if transfers.len() > usize::from(operation == wire::START) {
            return Response::error(Code::InvalidRequest);
        }
        let admin = peer.groups & door_abi::platform::group::ADMINISTRATORS != 0;
        let (control, rest) = match operation {
            wire::CONTROL | wire::CONTROL_STATE | wire::CONTROL_DONE if body.len() > 4 => (
                u32::from_le_bytes([body[0], body[1], body[2], body[3]]),
                &body[4..],
            ),
            wire::SET_STATUS if body.len() > wire::STATUS_BYTES => (0, &body[wire::STATUS_BYTES..]),
            // `[length u8][arguments][name]`: the arguments are checked below.
            wire::START_ARGUMENTS if body.len() > 1 + usize::from(body[0]) => {
                (0, &body[1 + usize::from(body[0])..])
            }
            wire::START_ARGUMENTS => return reply(error::INVALID_PARAMETER, &[]),
            _ => (0, body),
        };
        if operation == wire::LIST {
            return self.scm_list(body);
        }
        // An own-process service program may give no name (Windows
        // ignores it); everything else names its service.
        let Some(name) =
            wire::name(rest).or((operation == wire::REGISTER && rest.is_empty()).then_some(""))
        else {
            return reply(error::INVALID_PARAMETER, &[]);
        };
        let result: Result<Option<Status>, u32> = match operation {
            wire::QUERY => {
                self.scm_expire(now);
                self.scm.query(name).map(Some)
            }
            wire::START | wire::START_ARGUMENTS if !admin => Err(error::ACCESS_DENIED),
            wire::START | wire::START_ARGUMENTS => {
                self.scm_reload(name);
                // The service runs as the service account whoever starts it.
                let _ = transfers;
                // Its StartService arguments (none for a plain start; the
                // dependencies it starts get none, as on Windows).
                let arguments = if operation == wire::START_ARGUMENTS {
                    &body[1..1 + usize::from(body[0])]
                } else {
                    &[][..]
                };
                if !wire::arguments_valid(arguments) {
                    return reply(error::INVALID_PARAMETER, &[]);
                }
                let running = self
                    .scm
                    .query(name)
                    .is_ok_and(|s| s.state != state::STOPPED);
                if !running {
                    let _ = self.scm.set_arguments(name, arguments);
                }
                self.scm_start_tree(name, SERVICE_ACCOUNT, 0, now)
                    .and_then(|()| self.scm.query(name))
                    .map(Some)
            }
            wire::CONTROL if control != wire::control::INTERROGATE && !admin => {
                Err(error::ACCESS_DENIED)
            }
            wire::CONTROL => match self.scm.control(name, control, now) {
                Ok((status, sequence)) => {
                    let mut b = [0u8; wire::STATUS_BYTES + 4];
                    b[..wire::STATUS_BYTES].copy_from_slice(&status.encode());
                    b[wire::STATUS_BYTES..].copy_from_slice(&sequence.to_le_bytes());
                    return reply(0, &b);
                }
                Err(e) => Err(e),
            },
            wire::CONTROL_STATE => {
                self.scm_expire(now);
                match self.scm.control_state(name, control) {
                    Ok(state) => return reply(0, &[state]),
                    Err(e) => Err(e),
                }
            }
            wire::CONTROL_DONE => self
                .scm
                .control_done(peer.pid, name, control)
                .map(|()| None),
            wire::RELOAD if !admin => Err(error::ACCESS_DENIED),
            wire::RELOAD => {
                self.scm_reload(name);
                Ok(None)
            }
            wire::REGISTER => match self.scm.register(peer.pid, name) {
                Ok(real) => return reply(0, real.as_str().as_bytes()),
                Err(e) => Err(e),
            },
            wire::ARGUMENTS => {
                let mut out = [0u8; wire::ARGUMENT_BYTES];
                match self.scm.take_arguments(peer.pid, name, &mut out) {
                    Ok(n) => return reply(0, &out[..n]),
                    Err(e) => Err(e),
                }
            }
            wire::SET_STATUS => match Status::decode(&body[..wire::STATUS_BYTES]) {
                Some(s) => {
                    let before = self.scm.query(name).map(|x| x.state).unwrap_or(0);
                    let result = self.scm.set_status(peer.pid, name, s).map(|()| None);
                    if result.is_ok() {
                        log(format_args!(
                            "DOOR_P16 SCM status name={name} state={}",
                            s.state
                        ));
                        if before != s.state {
                            self.service_event(name, s.state);
                        }
                    }
                    result
                }
                None => Err(error::INVALID_PARAMETER),
            },
            wire::NEXT_CONTROL => match self.scm.next_control(peer.pid, name, now) {
                Ok(Some((code, sequence))) => {
                    let mut b = [0u8; 8];
                    b[..4].copy_from_slice(&code.to_le_bytes());
                    b[4..].copy_from_slice(&sequence.to_le_bytes());
                    return reply(0, &b);
                }
                Ok(None) => Ok(None),
                Err(e) => Err(e),
            },
            _ => return Response::error(Code::InvalidRequest),
        };
        match result {
            Ok(Some(status)) => reply(0, &status.encode()),
            Ok(None) => reply(0, &[]),
            Err(code) => reply(code, &[]),
        }
    }
    /// LIST: `[code u32][total u16][count u16]` then per service
    /// `[length u8][name][state u8][pid u32]`, from index `first`.
    fn scm_list(&mut self, body: &[u8]) -> Response {
        let first = body
            .get(..2)
            .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
            .unwrap_or(0);
        let mut b = [0u8; 236];
        let mut at = 8;
        let mut count = 0u16;
        let total = self.scm.iter().count();
        for s in self.scm.iter().skip(first) {
            let n = s.name.as_str().as_bytes();
            if at + 6 + n.len() > b.len() {
                break;
            }
            b[at] = n.len() as u8;
            b[at + 1..at + 1 + n.len()].copy_from_slice(n);
            b[at + 1 + n.len()] = s.status.state as u8;
            b[at + 2 + n.len()..at + 6 + n.len()].copy_from_slice(&s.status.pid.to_le_bytes());
            at += 6 + n.len();
            count += 1;
        }
        b[4..6].copy_from_slice(&(total as u16).to_le_bytes());
        b[6..8].copy_from_slice(&count.to_le_bytes());
        Response::bytes(&b[..at])
    }
    /// Reread `name`'s registry key (gone from the registry: deleted).
    pub(super) fn scm_reload(&mut self, name: &str) {
        let Ok(mut storage) = Storage::connect_port(self.ports[0]) else {
            return;
        };
        match read_config(&mut storage, name) {
            Some(config) => {
                let _ = self.scm.configure(name, config);
            }
            None => {
                if self.scm.find(name).is_some_and(|s| !s.builtin) {
                    let _ = self.scm.delete(name);
                }
            }
        }
    }
    /// The registered services, at sign-in.
    pub(super) fn scm_load(&mut self) {
        if let Ok(mut storage) = Storage::connect_port(self.ports[0]) {
            load(&mut self.scm, &mut storage);
        }
    }
    /// The service's program, detached (no console), as the session's
    /// user with the full token; its process ID.
    fn launch_service(&mut self, uid: u32, image: &str, cwd: Handle) -> Result<u64, u32> {
        // ImagePath is a command line: the program (quoted when its path
        // has spaces), then the service's own arguments. An unquoted path
        // with spaces and no arguments is the program as a whole.
        let mut arguments = [0u8; 256];
        let whole = !image.trim_start().starts_with('"')
            && image
                .trim_end()
                .rsplit_once('.')
                .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("exe"));
        let (program_end, len) = if whole {
            let program = image.trim().as_bytes();
            if program.len() + 1 > arguments.len() {
                return Err(error::INVALID_PARAMETER);
            }
            arguments[..program.len()].copy_from_slice(program);
            (program.len(), program.len() + 1)
        } else {
            super::tasks::split(image, &mut arguments).ok_or(error::INVALID_PARAMETER)?
        };
        let program = core::str::from_utf8(&arguments[..program_end])
            .map_err(|_| error::INVALID_PARAMETER)?;
        let path = native(program).ok_or(2u32)?;
        let path = path.as_str();
        let folder = &path[..path.rfind('/').unwrap_or(0).max(1)];
        let request = door_service_protocol::application::Execute {
            path,
            cwd: folder,
            arguments: &arguments[..len],
            staged: false,
            options: door_service_protocol::application::ExecuteOptions::NONE,
        };
        self.last_launched = None;
        let _ = self.execute(0, uid, true, &request, cwd, None, true);
        self.last_launched.take().ok_or(2)
    }
    /// A recovery program (`command`, a command line) as `uid`, from its
    /// program's folder.
    fn launch_command(&mut self, uid: u32, command: &str) -> Result<u64, u32> {
        // The service account's own working-folder grant (the kernel
        // gives the program one).
        let cwd = 0;
        if self.service_session == 0 {
            return Err(error::FAILED_SERVICE_CONTROLLER_CONNECT);
        }
        let mut arguments = [0u8; 256];
        let (program_end, len) =
            super::tasks::split(command, &mut arguments).ok_or(error::INVALID_PARAMETER)?;
        let program = core::str::from_utf8(&arguments[..program_end])
            .map_err(|_| error::INVALID_PARAMETER)?;
        let path = native(program).ok_or(2u32)?;
        let path = path.as_str();
        let folder = &path[..path.rfind('/').unwrap_or(0).max(1)];
        let request = door_service_protocol::application::Execute {
            path,
            cwd: folder,
            arguments: &arguments[..len],
            staged: false,
            options: door_service_protocol::application::ExecuteOptions::NONE,
        };
        self.last_launched = None;
        // The service's account: elevated, as the service runs.
        let _ = self.execute(0, uid, true, &request, cwd, None, true);
        self.last_launched.take().ok_or(2)
    }
}
impl super::Registry {
    /// StartService: the services `name` depends on that are stopped,
    /// then `name`; each launches once what it depends on runs.
    fn scm_start_tree(&mut self, name: &str, uid: u32, cwd: Handle, now: u64) -> Result<(), u32> {
        let (order, count) = self.scm.start_order(name)?;
        for entry in &order[..count] {
            let entry = entry.as_str();
            let target = same(entry, name);
            if !target {
                self.scm_reload(entry);
                if self
                    .scm
                    .find(entry)
                    .is_some_and(|s| s.status.state != state::STOPPED)
                {
                    continue;
                }
            }
            match self.scm.start(entry, now)? {
                Action::Launch { image } => {
                    self.scm_launch(entry, uid, image.as_str(), cwd, false)?
                }
                Action::Deferred => {
                    self.scm.started_by(entry, uid);
                    log(format_args!("DOOR_P16 SCM waiting name={entry}"));
                }
            }
        }
        Ok(())
    }
    /// Launch a started service's process.
    fn scm_launch(
        &mut self,
        name: &str,
        uid: u32,
        image: &str,
        cwd: Handle,
        automatic: bool,
    ) -> Result<(), u32> {
        // Services run in the service account's session (cwd 0: the kernel
        // gives the process its own working-folder grant).
        let launched = if self.service_session == 0 {
            Err(error::FAILED_SERVICE_CONTROLLER_CONNECT)
        } else {
            self.launch_service(uid, image, cwd)
        };
        match launched {
            Ok(pid) => {
                self.scm.launched(name, Some(pid), 0);
                self.scm.started_by(name, uid);
                log(format_args!(
                    "DOOR_P16 SCM started name={name} pid={pid}{}",
                    if automatic { " auto=1" } else { "" }
                ));
                Ok(())
            }
            Err(code) => {
                self.scm.launched(name, None, code);
                log(format_args!(
                    "DOOR_P16 SCM start failed name={name} error={code}"
                ));
                Err(code)
            }
        }
    }
    /// Services that never connected: stopped, their processes ended;
    /// services whose dependencies now run: launched.
    pub(super) fn scm_expire(&mut self, now: u64) {
        while let Some((name, image)) = self.scm.next_ready(now) {
            let _ = self.scm_launch(name.as_str(), SERVICE_ACCOUNT, image.as_str(), 0, false);
        }
        while let Some(expired) = self.scm.expire(now) {
            match expired {
                Expired::Terminate(pid) => {
                    if let Some(process) = self.job_process(pid) {
                        let _ = door_sys::process_terminate(process, -1);
                    }
                }
                Expired::ControlTimeout { name } => {
                    let display: Text<128> = self
                        .scm
                        .find(name.as_str())
                        .map_or_else(|| Text::new(name.as_str()), |s| s.config.display);
                    let mut message = Text::<400>::default();
                    let _ = core::fmt::Write::write_fmt(
                        &mut message,
                        format_args!(
                            "A timeout ({} milliseconds) was reached while waiting for a transaction response from the {} service.",
                            wire::CONTROL_TIMEOUT_MS,
                            display.as_str()
                        ),
                    );
                    log(format_args!(
                        "DOOR_P16 SCM control timeout name={}",
                        name.as_str()
                    ));
                    self.system_event(
                        door_service_protocol::eventlog::kind::ERROR,
                        7011,
                        "Service Control Manager",
                        message.as_str(),
                    );
                }
                Expired::RunCommand {
                    name,
                    command,
                    uid,
                    failures,
                } => {
                    // %1% becomes the failure count (the Recovery tab's
                    // "/fail=%1%").
                    let mut line = Text::<300>::default();
                    let mut rest = command.as_str();
                    while let Some(at) = rest.find("%1%") {
                        let _ = core::fmt::Write::write_fmt(
                            &mut line,
                            format_args!("{}{failures}", &rest[..at]),
                        );
                        rest = &rest[at + 3..];
                    }
                    let _ = core::fmt::Write::write_str(&mut line, rest);
                    match self.launch_command(uid, line.as_str()) {
                        Ok(pid) => log(format_args!(
                            "DOOR_P16 SCM recovery program name={} pid={pid}",
                            name.as_str()
                        )),
                        Err(code) => log(format_args!(
                            "DOOR_P16 SCM recovery program failed name={} error={code}",
                            name.as_str()
                        )),
                    }
                }
                Expired::Reboot { name } => {
                    log(format_args!("DOOR_P16 SCM reboot name={}", name.as_str()));
                    // As Start > Power > Restart: the desktop closes its
                    // windows first when it is up.
                    if self.window != 0 {
                        self.gui_power = Some(true);
                    } else {
                        self.power = Some(true);
                    }
                }
                Expired::Restart { name, image, uid } => {
                    let launched = if self.service_session == 0 {
                        Err(error::FAILED_SERVICE_CONTROLLER_CONNECT)
                    } else {
                        self.launch_service(uid, image.as_str(), 0)
                    };
                    match launched {
                        Ok(pid) => {
                            self.scm.launched(name.as_str(), Some(pid), 0);
                            log(format_args!(
                                "DOOR_P16 SCM restarted name={} pid={pid}",
                                name.as_str()
                            ));
                        }
                        Err(code) => {
                            self.scm.launched(name.as_str(), None, code);
                            log(format_args!(
                                "DOOR_P16 SCM restart failed name={} error={code}",
                                name.as_str()
                            ));
                        }
                    }
                }
            }
        }
    }
    /// Windows' event 7031 (a corrective action follows) or 7034 (none)
    /// when a service ended without stopping cleanly.
    pub(super) fn service_failure_event(&mut self, crash: &Crash) {
        let mut message = Text::<512>::default();
        let display = crash.display.as_str();
        let n = crash.failures;
        let id = match crash.action {
            Some((kind, delay)) => {
                let what = match kind {
                    action::RESTART => "Restart the service",
                    action::REBOOT => "Restart the computer",
                    _ => "Run the configured recovery program",
                };
                let _ = core::fmt::Write::write_fmt(
                    &mut message,
                    format_args!(
                        "The {display} service terminated unexpectedly.  It has done this {n} time(s).  The following corrective action will be taken in {delay} milliseconds: {what}."
                    ),
                );
                7031
            }
            None => {
                let _ = core::fmt::Write::write_fmt(
                    &mut message,
                    format_args!(
                        "The {display} service terminated unexpectedly.  It has done this {n} time(s)."
                    ),
                );
                7034
            }
        };
        log(format_args!(
            "DOOR_P16 SCM failure name={} count={n} event={id}",
            crash.name.as_str()
        ));
        self.system_event(
            door_service_protocol::eventlog::kind::ERROR,
            id,
            "Service Control Manager",
            message.as_str(),
        );
    }
}

impl super::Registry {
    /// Windows' event 7036 from the Service Control Manager: "The <display
    /// name> service entered the <running|stopped|paused> state."
    pub(super) fn service_event(&mut self, name: &str, state: u32) {
        let word = match state {
            wire::state::RUNNING => "running",
            wire::state::STOPPED => "stopped",
            wire::state::PAUSED => "paused",
            _ => return,
        };
        let display = self
            .scm
            .find(name)
            .map(|s| s.config.display)
            .unwrap_or_else(|| Text::new(name));
        let mut message = Text::<400>::default();
        let _ = core::fmt::Write::write_fmt(
            &mut message,
            format_args!("The {} service entered the {word} state.", display.as_str()),
        );
        self.system_event(
            door_service_protocol::eventlog::kind::INFORMATION,
            7036,
            "Service Control Manager",
            message.as_str(),
        );
    }
    /// Automatic services, once the session's working folder is known.
    /// The service account's profile, where LocalSystem's is
    /// (C:\Windows\System32\config\systemprofile), with its AppData
    /// folders (%TEMP% among them).
    pub(super) fn service_profile(&mut self) {
        let Ok(mut storage) = door_service_client::Storage::connect_port(self.ports[0]) else {
            return;
        };
        for path in [
            "/compat/Windows/System32/config",
            "/compat/Windows/System32/config/systemprofile",
            "/compat/Windows/System32/config/systemprofile/AppData",
            "/compat/Windows/System32/config/systemprofile/AppData/Local",
            "/compat/Windows/System32/config/systemprofile/AppData/Local/Temp",
            "/compat/Windows/System32/config/systemprofile/AppData/Roaming",
        ] {
            let _ = storage.mkdir(path);
        }
    }
    pub(super) fn scm_autostart(&mut self) {
        let now = door_sys::time().unwrap_or(0);
        let mut names = [Name::default(); SERVICES];
        let mut count = 0;
        for s in self.scm.iter() {
            if !s.builtin && s.config.start == start::AUTO && s.status.state == state::STOPPED {
                names[count] = s.name;
                count += 1;
            }
        }
        let (uid, cwd) = (SERVICE_ACCOUNT, 0);
        // Once a boot: a service an administrator stopped stays stopped
        // when someone signs in again.
        if self.service_session == 0 || self.services_autostarted {
            return;
        }
        self.services_autostarted = true;
        // Each with what it depends on first (an automatic service may
        // depend on a demand-start one, which then starts too).
        for name in &names[..count] {
            let name = name.as_str();
            if self
                .scm
                .find(name)
                .is_some_and(|s| s.status.state != state::STOPPED)
            {
                continue;
            }
            if let Err(code) = self.scm_start_tree(name, uid, cwd, now) {
                log(format_args!(
                    "DOOR_P16 SCM autostart failed name={name} error={code}"
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config(image: &str, depends: &[&str]) -> Config {
        let mut c = Config {
            display: Text::new("Test"),
            image: Text::new(image),
            start: start::DEMAND,
            kind: OWN_PROCESS,
            ..Config::default()
        };
        for d in depends {
            c.depends[c.depend_count] = Name::new(d);
            c.depend_count += 1;
        }
        c
    }
    fn running(m: &mut Manager, name: &str, pid: u64) {
        let action = m.start(name, 0).unwrap();
        assert!(matches!(action, Action::Launch { .. }));
        m.launched(name, Some(pid), 0);
        report_running(m, name, pid);
    }
    /// A launched service's process connects and reports it runs.
    fn report_running(m: &mut Manager, name: &str, pid: u64) {
        m.register(pid, name).unwrap();
        let status = Status {
            state: state::RUNNING,
            accepted: accept::STOP | accept::PAUSE_CONTINUE,
            ..Status::default()
        };
        m.set_status(pid, name, status).unwrap();
    }
    #[test]
    fn start_arguments_go_to_the_services_own_process_once() {
        let mut m = Manager::default();
        m.configure("Echo", config("C:\\svc\\echo.exe", &[]))
            .unwrap();
        m.set_arguments("Echo", b"alpha\0\0").unwrap();
        m.start("Echo", 10).unwrap();
        m.launched("Echo", Some(50), 0);
        let mut out = [0u8; door_service_protocol::scm::ARGUMENT_BYTES];
        // Only the service's connected process takes them.
        assert_eq!(
            m.take_arguments(51, "Echo", &mut out),
            Err(error::ACCESS_DENIED)
        );
        assert_eq!(
            m.take_arguments(50, "Echo", &mut out),
            Err(error::ACCESS_DENIED)
        );
        m.register(50, "Echo").unwrap();
        assert_eq!(m.take_arguments(50, "Echo", &mut out), Ok(7));
        assert_eq!(&out[..7], b"alpha\0\0");
        assert_eq!(m.take_arguments(50, "Echo", &mut out), Ok(0));
        assert_eq!(
            m.set_arguments("Missing", b""),
            Err(error::SERVICE_DOES_NOT_EXIST)
        );
    }
    #[test]
    fn a_service_starts_runs_and_stops() {
        let mut m = Manager::default();
        m.builtin("DoorsAudio", "Doors Audio", 7);
        m.configure("Echo", config("C:\\svc\\echo.exe", &["DoorsAudio"]))
            .unwrap();
        assert_eq!(m.query("echo").unwrap().state, state::STOPPED);
        assert_eq!(
            m.start("Echo", 10),
            Ok(Action::Launch {
                image: Text::new("C:\\svc\\echo.exe")
            })
        );
        assert_eq!(m.start("Echo", 11), Err(error::SERVICE_ALREADY_RUNNING));
        m.launched("Echo", Some(50), 0);
        assert_eq!(
            m.control("Echo", control::STOP, 0).map(|(s, _)| s),
            Err(error::SERVICE_CANNOT_ACCEPT_CTRL)
        );
        assert_eq!(
            m.register(51, "Echo"),
            Err(error::FAILED_SERVICE_CONTROLLER_CONNECT)
        );
        // A program started outside the manager, under a name it does not
        // know: still 1063, as StartServiceCtrlDispatcher answers.
        assert_eq!(
            m.register(52, "Unknown"),
            Err(error::FAILED_SERVICE_CONTROLLER_CONNECT)
        );
        // An own-process service program's table name is ignored: the
        // process connects as the service it was started for.
        assert_eq!(m.register(50, "Whatever"), Ok(Name::new("Echo")));
        assert_eq!(m.register(50, ""), Ok(Name::new("Echo")));
        let running = Status {
            state: state::RUNNING,
            accepted: accept::STOP,
            ..Status::default()
        };
        assert_eq!(m.set_status(51, "Echo", running), Err(error::ACCESS_DENIED));
        m.set_status(50, "Echo", running).unwrap();
        let s = m.query("Echo").unwrap();
        assert_eq!((s.state, s.pid, s.kind), (state::RUNNING, 50, OWN_PROCESS));
        assert_eq!(
            m.control("Echo", control::PAUSE, 0).map(|(s, _)| s),
            Err(error::SERVICE_CANNOT_ACCEPT_CTRL)
        );
        assert_eq!(
            m.control("Echo", control::INTERROGATE, 0)
                .map(|(s, _)| s)
                .unwrap()
                .state,
            state::RUNNING
        );
        m.control("Echo", control::STOP, 0).map(|(s, _)| s).unwrap();
        assert_eq!(m.next_control(50, "Echo", 0), Ok(Some((control::STOP, 1))));
        assert_eq!(m.next_control(50, "Echo", 0), Ok(None));
        let stopped = Status {
            state: state::STOPPED,
            ..Status::default()
        };
        m.set_status(50, "Echo", stopped).unwrap();
        m.exited(50, 0, 0);
        let s = m.query("Echo").unwrap();
        assert_eq!((s.state, s.exit_code, s.pid), (state::STOPPED, 0, 0));
        assert_eq!(
            m.control("DoorsAudio", control::STOP, 0).map(|(s, _)| s),
            Err(error::SERVICE_CANNOT_ACCEPT_CTRL)
        );
        assert_eq!(m.query("Missing"), Err(error::SERVICE_DOES_NOT_EXIST));
    }
    #[test]
    fn dependencies_start_first_and_cycles_are_refused() {
        let mut m = Manager::default();
        m.configure("Base", config("base.exe", &[])).unwrap();
        m.configure("Middle", config("middle.exe", &["Base"]))
            .unwrap();
        m.configure("Top", config("top.exe", &["Middle", "Base"]))
            .unwrap();
        let (order, count) = m.start_order("Top").unwrap();
        let names = [order[0].as_str(), order[1].as_str(), order[2].as_str()];
        assert_eq!(count, 3);
        assert_eq!(names, ["Base", "Middle", "Top"]);
        // Base launches; Middle and Top wait for what they depend on.
        assert!(matches!(m.start("Base", 0), Ok(Action::Launch { .. })));
        m.launched("Base", Some(20), 0);
        assert_eq!(m.start("Middle", 0), Ok(Action::Deferred));
        assert_eq!(m.start("Top", 0), Ok(Action::Deferred));
        assert_eq!(m.next_ready(1), None);
        report_running(&mut m, "Base", 20);
        assert_eq!(
            m.next_ready(2).map(|(n, _)| n.as_str() == "Middle"),
            Some(true)
        );
        assert_eq!(m.next_ready(2), None, "Top waits for Middle too");
        m.launched("Middle", Some(21), 0);
        report_running(&mut m, "Middle", 21);
        assert_eq!(
            m.next_ready(3).map(|(n, _)| n.as_str() == "Top"),
            Some(true)
        );
        // A cycle, a dependency that does not exist, a disabled one.
        m.configure("A", config("a.exe", &["B"])).unwrap();
        m.configure("B", config("b.exe", &["A"])).unwrap();
        assert_eq!(
            m.start_order("A").map(|_| ()),
            Err(error::CIRCULAR_DEPENDENCY)
        );
        m.configure("Lost", config("lost.exe", &["Nowhere"]))
            .unwrap();
        assert_eq!(
            m.start_order("Lost").map(|_| ()),
            Err(error::SERVICE_DEPENDENCY_DELETED)
        );
        let mut off = config("off.exe", &[]);
        off.start = start::DISABLED;
        m.configure("Off", off).unwrap();
        m.configure("NeedsOff", config("n.exe", &["Off"])).unwrap();
        assert_eq!(
            m.start_order("NeedsOff").map(|_| ()),
            Err(error::SERVICE_DEPENDENCY_FAIL)
        );
        // A dependency that fails to start fails the one waiting for it.
        m.configure("Flaky", config("flaky.exe", &[])).unwrap();
        m.configure("OnFlaky", config("on.exe", &["Flaky"]))
            .unwrap();
        m.start("Flaky", 100).unwrap();
        m.launched("Flaky", Some(30), 0);
        assert_eq!(m.start("OnFlaky", 100), Ok(Action::Deferred));
        // Flaky's process ends before it ever runs.
        m.exited(30, 3, 101);
        while m.expire(101).is_some() {}
        // A dependency already stopped is refused at once.
        assert_eq!(m.start("OnFlaky", 102), Err(error::SERVICE_DEPENDENCY_FAIL));
        assert_eq!(
            m.query("OnFlaky").unwrap().exit_code,
            error::SERVICE_DEPENDENCY_FAIL
        );
        assert_eq!(m.query("OnFlaky").unwrap().state, state::STOPPED);
    }
    #[test]
    fn crashes_dependencies_timeouts_deletion_and_capacity() {
        let mut m = Manager::default();
        m.configure("Base", config("base.exe", &[])).unwrap();
        m.configure("Top", config("top.exe", &["Base"])).unwrap();
        assert_eq!(m.start("Top", 0), Err(error::SERVICE_DEPENDENCY_FAIL));
        running(&mut m, "Base", 10);
        running(&mut m, "Top", 11);
        assert_eq!(
            m.control("Base", control::STOP, 0).map(|(s, _)| s),
            Err(error::DEPENDENT_SERVICES_RUNNING)
        );
        m.exited(11, 0, 0);
        assert_eq!(m.query("Top").unwrap().exit_code, error::PROCESS_ABORTED);
        m.control("Base", control::STOP, 2000)
            .map(|(s, _)| s)
            .unwrap();
        m.configure("Slow", config("slow.exe", &[])).unwrap();
        m.start("Slow", 1000).unwrap();
        m.launched("Slow", Some(12), 0);
        assert_eq!(m.expire(1000 + CONNECT_TICKS), None);
        assert_eq!(m.expire(1001 + CONNECT_TICKS), Some(Expired::Terminate(12)));
        assert_eq!(m.expire(1001 + CONNECT_TICKS), None);
        assert_eq!(
            m.query("Slow").unwrap().exit_code,
            error::SERVICE_REQUEST_TIMEOUT
        );
        let mut disabled = config("off.exe", &[]);
        disabled.start = start::DISABLED;
        m.configure("Off", disabled).unwrap();
        assert_eq!(m.start("Off", 0), Err(error::SERVICE_DISABLED));
        m.delete("Base").unwrap();
        assert!(m.find("Base").is_none(), "marked for delete");
        m.exited(10, 0, 0);
        assert!(
            !m.services
                .iter()
                .flatten()
                .any(|s| s.name.as_str() == "Base")
        );
        assert_eq!(
            m.control("Top", 99, 0).map(|(s, _)| s),
            Err(error::INVALID_SERVICE_CONTROL)
        );
        // Sixteen services at most.
        let mut full = Manager::default();
        for i in 0..SERVICES {
            let mut name = Name::default();
            core::fmt::Write::write_fmt(&mut name, format_args!("S{i}")).unwrap();
            full.configure(name.as_str(), config("x.exe", &[])).unwrap();
        }
        assert_eq!(
            full.configure("One more", config("x.exe", &[])),
            Err(error::NOT_ENOUGH_MEMORY)
        );
        assert_eq!(Text::<4>::new("abcdé").as_str(), "abcd");
        assert_eq!(Text::<5>::new("abcdé").as_str(), "abcd");
    }
    #[test]
    fn failure_actions_restart_a_crashing_service_then_give_up() {
        // Restart after 1 s twice, then nothing; the count starts over
        // after a day without failures (as `sc failure` writes it).
        let failure = Failure {
            reset: 86_400,
            actions: [
                (action::RESTART, 1000),
                (action::RESTART, 1000),
                (action::NONE, 0),
                (0, 0),
            ],
            count: 3,
            non_crash: false,
        };
        let mut raw = [0u8; 20 + 8 * ACTIONS];
        let n = failure.encode(&mut raw);
        assert_eq!(n, 44);
        assert_eq!(
            &raw[..20],
            &[
                0x80, 0x51, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0, 0, 0, 20, 0, 0, 0
            ]
        );
        assert_eq!(Failure::decode(&raw[..n]), Some(failure));
        assert_eq!(Failure::decode(&raw[..10]), None);
        let mut m = Manager::default();
        let mut c = config("crash.exe", &[]);
        c.failure = failure;
        m.configure("Crash", c).unwrap();
        running(&mut m, "Crash", 20);
        m.started_by("Crash", 1001);
        // First crash at t=100: 7031, restart due at t=200.
        let crash = m.exited(20, 0xC000_0005, 100).unwrap();
        assert_eq!(
            (crash.failures, crash.action),
            (1, Some((action::RESTART, 1000)))
        );
        assert_eq!(m.query("Crash").unwrap().exit_code, 0xC000_0005);
        assert_eq!(m.expire(199), None);
        let Some(Expired::Restart { name, image, uid }) = m.expire(200) else {
            panic!("no restart");
        };
        assert_eq!(
            (name.as_str(), image.as_str(), uid),
            ("Crash", "crash.exe", 1001)
        );
        assert_eq!(m.query("Crash").unwrap().state, state::START_PENDING);
        m.launched("Crash", Some(21), 0);
        m.register(21, "Crash").unwrap();
        // Second crash: restart again; third: no action (7034).
        assert_eq!(
            m.exited(21, 1, 300).unwrap().action,
            Some((action::RESTART, 1000))
        );
        assert!(matches!(m.expire(400), Some(Expired::Restart { .. })));
        m.launched("Crash", Some(22), 0);
        let third = m.exited(22, 1, 500).unwrap();
        assert_eq!((third.failures, third.action), (3, None));
        assert_eq!(m.expire(10_000), None);
        assert_eq!(m.query("Crash").unwrap().state, state::STOPPED);
        // A clean stop is no failure; after the reset period the count
        // starts over.
        running(&mut m, "Crash", 23);
        m.set_status(
            23,
            "Crash",
            Status {
                state: state::STOPPED,
                ..Status::default()
            },
        )
        .unwrap();
        assert_eq!(m.exited(23, 0, 600), None);
        running(&mut m, "Crash", 24);
        let later = 500 + 86_400 * TICKS_PER_SECOND + 1;
        assert_eq!(m.exited(24, 1, later).unwrap().failures, 1);
        // A manual start cancels a pending restart.
        assert!(m.start("Crash", later + 1).is_ok());
        assert_eq!(m.expire(later + 1000), None);
    }
    #[test]
    fn failure_actions_run_a_program_then_restart_the_computer() {
        let mut m = Manager::default();
        let mut c = config("crash.exe", &[]);
        c.failure = Failure {
            reset: u32::MAX,
            actions: [
                (action::RUN_COMMAND, 500),
                (action::REBOOT, 2000),
                (0, 0),
                (0, 0),
            ],
            count: 2,
            non_crash: false,
        };
        c.command = Text::new(r#""C:\Tools\notify.exe" /fail=%1%"#);
        m.configure("Crash", c).unwrap();
        running(&mut m, "Crash", 20);
        m.started_by("Crash", 1001);
        let crash = m.exited(20, 1, 100).unwrap();
        assert_eq!(crash.action, Some((action::RUN_COMMAND, 500)));
        assert_eq!(m.expire(149), None);
        let Some(Expired::RunCommand {
            name,
            command,
            uid,
            failures,
        }) = m.expire(150)
        else {
            panic!("no recovery program");
        };
        assert_eq!(
            (name.as_str(), command.as_str(), uid, failures),
            ("Crash", r#""C:\Tools\notify.exe" /fail=%1%"#, 1001, 1)
        );
        // The service stays stopped; a manual start does not cancel the
        // reboot the second failure schedules.
        assert_eq!(m.query("Crash").unwrap().state, state::STOPPED);
        running(&mut m, "Crash", 21);
        assert_eq!(
            m.exited(21, 1, 300).unwrap().action,
            Some((action::REBOOT, 2000))
        );
        assert!(m.start("Crash", 301).is_ok());
        let Some(Expired::Reboot { name }) = m.expire(500) else {
            panic!("no reboot");
        };
        assert_eq!(name.as_str(), "Crash");
        assert_eq!(m.expire(10_000), None);
        // No command configured: nothing runs.
        let mut quiet = config("quiet.exe", &[]);
        quiet.failure.actions[0] = (action::RUN_COMMAND, 0);
        quiet.failure.count = 1;
        m.configure("Quiet", quiet).unwrap();
        running(&mut m, "Quiet", 30);
        assert!(m.exited(30, 1, 600).is_some());
        assert_eq!(m.expire(700), None);
    }
    #[test]
    fn a_control_handler_that_does_not_return_times_out() {
        use door_service_protocol::scm::{CONTROL_DONE_STATE, CONTROL_PENDING, CONTROL_TIMED_OUT};
        let mut m = Manager::default();
        m.configure("Hang", config("hang.exe", &[])).unwrap();
        running(&mut m, "Hang", 30);
        let (_, first) = m.control("Hang", control::PAUSE, 1000).unwrap();
        assert_eq!(m.control_state("Hang", first), Ok(CONTROL_PENDING));
        assert_eq!(
            m.next_control(30, "Hang", 1010),
            Ok(Some((control::PAUSE, first)))
        );
        assert_eq!(m.control_done(31, "Hang", first), Err(error::ACCESS_DENIED));
        m.control_done(30, "Hang", first).unwrap();
        assert_eq!(m.control_state("Hang", first), Ok(CONTROL_DONE_STATE));
        // STOP is taken, and its handler never returns: 7011 once, and
        // the caller's wait ends with ERROR_SERVICE_REQUEST_TIMEOUT.
        let (_, stop) = m.control("Hang", control::STOP, 2000).unwrap();
        assert_eq!(
            m.next_control(30, "Hang", 2005),
            Ok(Some((control::STOP, stop)))
        );
        assert_eq!(m.expire(2005 + CONTROL_TICKS), None);
        assert_eq!(
            m.expire(2006 + CONTROL_TICKS),
            Some(Expired::ControlTimeout {
                name: Name::new("Hang")
            })
        );
        assert_eq!(m.expire(2007 + CONTROL_TICKS), None);
        assert_eq!(m.control_state("Hang", stop), Ok(CONTROL_TIMED_OUT));
        // When the service does stop, its controls are done.
        m.set_status(
            30,
            "Hang",
            Status {
                state: state::STOPPED,
                ..Status::default()
            },
        )
        .unwrap();
        assert_eq!(m.exited(30, 0, 9000), None);
        assert_eq!(m.control_state("Hang", stop), Ok(CONTROL_DONE_STATE));
        // A control nobody takes times out too.
        running(&mut m, "Hang", 32);
        let (_, waiting) = m.control("Hang", control::STOP, 20_000).unwrap();
        assert_eq!(
            m.expire(20_001 + CONTROL_TICKS),
            Some(Expired::ControlTimeout {
                name: Name::new("Hang")
            })
        );
        assert_eq!(m.control_state("Hang", waiting), Ok(CONTROL_TIMED_OUT));
    }
}
