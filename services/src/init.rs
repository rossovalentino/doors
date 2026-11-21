//! User-mode service graph, authenticated endpoint registry and session launch.
mod applications;
use crate::server::{self, Handler, Response, Server};
use core::fmt::Write;
use door_abi::{
    Handle, HandleKind, Status, Transfer,
    platform::{Identity, LaunchResult, StartupV2, inherited, mode},
};
use door_service_protocol::{
    Code, Frame, Message, manifest, opcode,
    supervisor::{Action, State, Supervisor},
};

struct Log {
    bytes: [u8; 256],
    len: usize,
}
impl core::fmt::Write for Log {
    fn write_str(&mut self, text: &str) -> core::fmt::Result {
        let end = self.len.checked_add(text.len()).ok_or(core::fmt::Error)?;
        if end > self.bytes.len() {
            return Err(core::fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}
fn log(arguments: core::fmt::Arguments<'_>) {
    let mut output = Log {
        bytes: [0; 256],
        len: 0,
    };
    if output.write_fmt(arguments).is_ok() && output.write_str("\n").is_ok() {
        let _ = door_sys::write(&output.bytes[..output.len]);
    }
}
macro_rules! line { ($($arguments:tt)*) => { log(format_args!($($arguments)*)) }; }
mod classes;
mod com;
mod desktop;
mod device;
mod eventlog;
mod firewall;
mod scm;
mod tasks;
struct Child {
    launch: LaunchResult,
    identity: Identity,
    policy_instance: u64,
}
impl Registry {
    /// A process of the signed-in user's session, or of the service
    /// account's (a Windows service and what it started).
    fn admits(&self, peer: Identity) -> bool {
        admitted(
            peer,
            (self.session != 0).then_some(self.session_id),
            (self.service_session != 0).then_some(self.service_session_id),
        )
    }
}
/// `Registry::admits` with the open sessions' ids: an authenticated
/// session peer of the signed-in user's session, or the service account's
/// own process in the service session (a user's process that names the
/// service session's id is not one).
fn admitted(peer: Identity, user: Option<u64>, service: Option<u64>) -> bool {
    server::session_peer(peer)
        && (user == Some(peer.session)
            || (service == Some(peer.session) && server::service_peer(peer)))
}
impl Handler for Registry {
    fn request(
        &mut self,
        channel: Handle,
        peer: Identity,
        operation: u32,
        body: &[u8],
        transfers: &[Transfer],
    ) -> Response {
        if operation == door_service_protocol::application::DESKTOP_STATE_V2 {
            if peer.manifest != door_abi::platform::manifest::WINDOW
                || peer.session != 0
                || peer.instance != self.window_instance
                || self.window_instance == 0
                || self.session == 0
                || !body.is_empty()
                || !transfers.is_empty()
            {
                return Response::error(Code::Denied);
            }
            let mut state = door_service_protocol::application::DesktopState {
                count: door_service_protocol::application::ELEVATED_TERMINAL_SLOT + 1,
                power: 0,
                frozen: self.desktop_admission.frozen(),
                processes: [None; 8],
            };
            state.power = match self.gui_power.take() {
                None => 0,
                Some(false) => 1,
                Some(true) => 2,
            };
            for i in 0..door_service_protocol::application::DESKTOP_APPS {
                if let Some((pid, _)) = self.gui_clients[i] {
                    state.processes[i] = Some(door_service_protocol::application::DesktopProcess {
                        pid,
                        instance: self.gui_instances[i],
                    });
                }
            }
            if let Some((pid, _)) = self.search_host {
                state.processes[door_service_protocol::application::SEARCH_HOST_SLOT] =
                    Some(door_service_protocol::application::DesktopProcess {
                        pid,
                        instance: self.search_instance,
                    });
            }
            if let Some((pid, instance)) = self.admin_terminal {
                state.processes[door_service_protocol::application::ELEVATED_TERMINAL_SLOT] =
                    Some(door_service_protocol::application::DesktopProcess { pid, instance });
            }
            let mut out = [0; door_service_protocol::application::DesktopState::MAX_BYTES];
            return match state.encode(&mut out) {
                Ok(len) => Response::bytes(&out[..len]),
                Err(e) => Response::error(e),
            };
        }
        if matches!(
            operation,
            door_service_protocol::application::SHELL_LAYOUT
                | door_service_protocol::application::DESKTOP_CONTROLS_PERSIST
        ) {
            // Authenticated current display service, signed-in session only.
            // Values are validated here and again by storage; the desktop
            // step writes them for the session user (and applies controls).
            if peer.manifest != door_abi::platform::manifest::WINDOW
                || peer.session != 0
                || peer.instance != self.window_instance
                || self.window_instance == 0
                || self.session == 0
                || !transfers.is_empty()
            {
                return Response::error(Code::Denied);
            }
            let word = |at: usize| u32::from_le_bytes(body[at..at + 4].try_into().unwrap());
            return match (operation, body.len()) {
                (door_service_protocol::application::SHELL_LAYOUT, 8) => {
                    match door_service_protocol::desktop::ShellLayout::from_parts(word(0), word(4))
                    {
                        Some(layout) => {
                            self.shell_layout_pending = Some(layout);
                            Response::empty()
                        }
                        None => Response::error(Code::InvalidRequest),
                    }
                }
                (door_service_protocol::application::DESKTOP_CONTROLS_PERSIST, 4) => {
                    match door_service_protocol::desktop::Controls::from_bits(word(0)) {
                        Some(controls) => {
                            self.controls_pending = Some(controls);
                            Response::empty()
                        }
                        None => Response::error(Code::InvalidRequest),
                    }
                }
                _ => Response::error(Code::InvalidRequest),
            };
        }
        if operation == door_service_protocol::window::PERSIST_SETTING {
            // Authenticated current display service, signed-in session only:
            // a setting the shell changed in place (dark-mode schedule, Win+V
            // "Turn on"). The desktop step writes it for the session user.
            if peer.manifest != door_abi::platform::manifest::WINDOW
                || peer.session != 0
                || peer.instance != self.window_instance
                || self.window_instance == 0
                || self.session == 0
                || !transfers.is_empty()
            {
                return Response::error(Code::Denied);
            }
            if body.len() != 8 {
                return Response::error(Code::InvalidRequest);
            }
            let word = |at: usize| u32::from_le_bytes(body[at..at + 4].try_into().unwrap());
            return match desktop::persist_slot(word(0), word(4)) {
                Some(slot) => {
                    self.settings_pending[slot] = Some(word(4));
                    Response::empty()
                }
                None => Response::error(Code::InvalidRequest),
            };
        }
        if operation == door_service_protocol::application::DESKTOP {
            if peer.manifest != door_abi::platform::manifest::WINDOW
                || peer.session != 0
                || peer.instance != self.window_instance
                || self.window_instance == 0
                || body.len() != 1
                || !transfers.is_empty()
                || !(body[0] <= 8
                    || door_service_protocol::application::desktop_index(body[0]).is_some()
                    || door_service_protocol::application::accessory_index(body[0]).is_some()
                    || body[0] == door_service_protocol::application::ELEVATED_TERMINAL_COMMAND
                    || body[0] == door_service_protocol::application::SIGN_IN_SECOND_COMMAND
                    || body[0] == door_service_protocol::application::SIGN_OUT_COMMAND)
                || self.power.is_some()
            {
                return Response::error(Code::Denied);
            }
            if body[0] == 8 || body[0] == door_service_protocol::application::SIGN_IN_SECOND_COMMAND
            {
                if self.session != 0 {
                    return Response::error(Code::InvalidRequest);
                }
                self.signin_requested = true;
                self.signin_second =
                    body[0] == door_service_protocol::application::SIGN_IN_SECOND_COMMAND;
                line!(
                    "DOOR_P16 SIGNIN account={}",
                    if self.signin_second { 1002 } else { 1001 }
                );
                return Response::empty();
            }
            if self.session == 0 {
                return Response::error(Code::Denied);
            }
            if body[0] == 0 {
                let mut out = [0; 104];
                out[4..8]
                    .copy_from_slice(&u32::from(self.desktop_admission.frozen()).to_le_bytes());
                out[..4].copy_from_slice(
                    &match self.gui_power.take() {
                        None => 0u32,
                        Some(false) => 1,
                        Some(true) => 2,
                    }
                    .to_le_bytes(),
                );
                for index in 0..4 {
                    if let Some((pid, _)) = self.gui_clients[index] {
                        let record = &mut out[8 + index * 24..8 + (index + 1) * 24];
                        record[..4].copy_from_slice(&(index as u32 + 1).to_le_bytes());
                        record[8..16].copy_from_slice(&pid.to_le_bytes());
                        record[16..24].copy_from_slice(&self.gui_instances[index].to_le_bytes());
                    }
                }
                return Response::bytes(&out);
            }
            return match self.desktop_admission.request(body[0]) {
                Ok(()) => Response::empty(),
                Err(error) => Response::error(error),
            };
        }
        if operation == door_service_protocol::application::EXECUTE_ELEVATED {
            // User Account Control is the window server's: no session app
            // can start a program through this operation.
            if peer.manifest != door_abi::platform::manifest::WINDOW
                || peer.session != 0
                || peer.instance != self.window_instance
                || self.window_instance == 0
            {
                return Response::error(Code::Denied);
            }
            return self.execute_elevated(body, transfers);
        }
        if operation == door_service_protocol::window::TERMINATE_CLIENT {
            if peer.manifest != door_abi::platform::manifest::WINDOW
                || peer.session != 0
                || peer.instance != self.window_instance
                || body.len() != 8
                || !transfers.is_empty()
            {
                return Response::error(Code::Denied);
            }
            let pid = u64::from_le_bytes(body.try_into().unwrap());
            // Desktop GUI clients, then EXECUTE-launched children (Windows GUI
            // applications started from Explorer or the terminal).
            let process = self
                .gui_clients
                .iter()
                .flatten()
                .find(|c| c.0 == pid)
                .map(|c| c.1)
                .or_else(|| self.job_process(pid));
            let Some(process) = process else {
                // Authenticated current Window service only: a reaped process
                // completes an idempotent pending termination request.
                return Response::error(Code::NotFound);
            };
            return match door_sys::process_terminate(process, -130) {
                Ok(()) => Response::empty(),
                Err(e) => Response::error(server::status(e)),
            };
        }
        if (door_service_protocol::notifications::POST..=door_service_protocol::notifications::LAST)
            .contains(&operation)
        {
            return self.notification_request(peer, operation, body, transfers);
        }
        // Rename this PC (Settings app only; HKLM needs Init's system access).
        if (door_service_protocol::eventlog::FIRST..=door_service_protocol::eventlog::LAST)
            .contains(&operation)
        {
            if !transfers.is_empty() {
                return Response::error(Code::InvalidRequest);
            }
            return self.eventlog_request(channel, peer, operation, body);
        }
        if (door_service_protocol::tasks::FIRST..=door_service_protocol::tasks::LAST)
            .contains(&operation)
        {
            if !transfers.is_empty() {
                return Response::error(Code::InvalidRequest);
            }
            return self.tasks_request(peer, operation, body);
        }
        if (door_service_protocol::scm::FIRST..=door_service_protocol::scm::LAST)
            .contains(&operation)
        {
            let now = door_sys::time().unwrap_or(0);
            return self.scm_request(peer, operation, body, transfers, now);
        }
        if (door_service_protocol::firewall::FIRST..=door_service_protocol::firewall::LAST)
            .contains(&operation)
        {
            return self.firewall_request(peer, operation, body, transfers);
        }
        // COM activation broker and ncalrpc endpoints: this session's
        // processes only.
        if (door_service_protocol::com::FIRST..=door_service_protocol::com::LAST)
            .contains(&operation)
        {
            if !transfers.is_empty() || !self.admits(peer) {
                return Response::error(Code::Denied);
            }
            let now = door_sys::time().unwrap_or(0);
            return self.com.request(channel, peer, operation, body, now);
        }
        if operation == door_service_protocol::device::SET_NAME {
            return self.device_name_request(peer, body, transfers);
        }
        if operation != opcode::RESOLVE {
            return self.application_request(peer, operation, body, transfers);
        }
        if body.len() != 8 || !transfers.is_empty() {
            return Response::error(Code::InvalidRequest);
        }
        if peer.session == 0
            && peer.manifest != manifest::INIT
            && !(manifest::STORAGE..=manifest::CONSOLE).contains(&peer.manifest)
            && peer.manifest != door_abi::platform::manifest::WINDOW
        {
            return Response::error(Code::Denied);
        }
        let id = u64::from_le_bytes(body.try_into().unwrap());
        if id == door_abi::platform::manifest::NETWORK || id == door_abi::platform::manifest::AUDIO
        {
            // Session apps and the window server (taskbar network and
            // volume state, quick settings).
            let session_app = peer.session != 0 && self.admits(peer);
            if !session_app && peer.manifest != door_abi::platform::manifest::WINDOW {
                return Response::error(Code::Denied);
            }
            let port = if id == door_abi::platform::manifest::AUDIO {
                self.audio
            } else {
                self.network
            };
            return if port == 0 {
                Response::error(Code::WouldBlock)
            } else {
                Response::handle(port, manifest::PORT_RIGHTS, false)
            };
        }
        if id == door_abi::platform::manifest::WINDOW {
            // Services' windows go to the signed-in user's desktop (Doors
            // has no separate non-interactive desktop yet).
            if self.session == 0 || peer.session == 0 || !self.admits(peer) {
                return Response::error(Code::Denied);
            }
            return if self.window == 0 {
                Response::error(Code::WouldBlock)
            } else {
                Response::handle(self.window, manifest::PORT_RIGHTS, false)
            };
        }
        let Some(index) = manifest::IDS.iter().position(|&m| m == id) else {
            return Response::error(Code::NotFound);
        };
        if self.ports[index] == 0 {
            return Response::error(Code::WouldBlock);
        }
        Response::handle(self.ports[index], manifest::PORT_RIGHTS, false)
    }
    fn disconnected(&mut self, channel: Handle) {
        self.com.disconnected(channel);
        self.eventlog_disconnected(channel);
    }
}
struct Init {
    desktop: desktop::State,
    startup: StartupV2,
    authority: Handle,
    policy: Supervisor,
    registry: Registry,
    server: Server,
    children: [Option<Child>; 3],
    launched: [u64; 3],
    last_exit: [u64; 3],
    backoff_observed: u64,
    readiness_timeout: bool,
    restarted: bool,
    restarting: bool,
    old_instance: u64,
    old_port: Handle,
    app: Option<LaunchResult>,
    revoking: bool,
    probe_revoked: bool,
    probe_done: bool,
    probe_deadline: u64,
    probe_session_id: u64,
    probe_mutex: Handle,
    session: Handle,
    completed: usize,
    stopping: bool,
    failure: bool,
    stage: u8,
}
impl Init {
    fn new(startup: StartupV2) -> Result<Self, Status> {
        let server = Server::new(&startup)?;
        let authority = startup.base.inherited[inherited::AUTHORITY].handle;
        let policy = Supervisor::new(&manifest::SERVICES).map_err(|_| Status::InvalidArgument)?;
        Ok(Self {
            desktop: desktop::State::default(),
            startup,
            authority,
            policy,
            registry: Registry {
                ports: [0; 3],
                launch: authority,
                port: server.port,
                session: 0,
                session_id: 0,
                service_session: 0,
                service_session_id: 0,
                scm_ready: false,
                services_autostarted: false,
                shell: 0,
                power: None,
                jobs: [None; 8],
                argument_stage: applications::Arguments::EMPTY,
                standard: [0; 3],
                long_arguments: [applications::Arguments::EMPTY; 4],
                scm: scm::Manager::default(),
                last_launched: None,
                event_logs: [eventlog::Ring::default(); 4],
                event_text: [eventlog::NO_STAGE; eventlog::STAGES],
                tasks: tasks::Scheduler::default(),
                com: com::Broker::default(),
                session_uid: 0,
                session_directory: 0,
                window: 0,
                window_instance: 0,
                network: 0,
                audio: 0,
                gui_clients: [None; door_service_protocol::application::DESKTOP_APPS + 1],
                desktop_admission: door_service_protocol::application::DesktopAdmission::default(),
                gui_instances: [0; door_service_protocol::application::DESKTOP_APPS],
                gui_power: None,
                search_host: None,
                search_instance: 0,
                signin_requested: false,
                signin_second: false,
                shell_layout_pending: None,
                controls_pending: None,
                settings_pending: [None; 1 + door_service_protocol::preferences::ALL.len()],
                notifications: crate::notifications::Center::new(),
                accessories: [None; door_service_protocol::application::ACCESSORIES],
                admin_terminal: None,
            },
            server,
            children: core::array::from_fn(|_| None),
            launched: [0; 3],
            last_exit: [0; 3],
            backoff_observed: 0,
            readiness_timeout: false,
            restarted: false,
            restarting: false,
            old_instance: 0,
            old_port: 0,
            app: None,
            revoking: false,
            probe_revoked: false,
            probe_done: false,
            probe_deadline: 0,
            probe_session_id: 0,
            probe_mutex: 0,
            session: 0,
            completed: 0,
            stopping: false,
            failure: false,
            stage: 0,
        })
    }
    fn act(&mut self, action: Action, now: u64) -> Result<(), Status> {
        self.server.note_progress();
        match action {
            Action::Launch { service, instance } => {
                self.stage = 10 + service as u8;
                let child_mode = if matches!(self.startup.mode, mode::SESSION | mode::DESKTOP) {
                    mode::SESSION
                } else if service == 2
                    && self.startup.mode == mode::RESTART
                    && self.launched[2] == 0
                {
                    mode::FAILURE
                } else if service == 0 && self.startup.mode == mode::SAVE && self.launched[0] == 0 {
                    mode::SAVE
                } else if self.startup.mode == mode::FAILURE {
                    mode::FAILURE
                } else {
                    mode::REOPEN
                };
                if self.last_exit[service] != 0
                    && matches!(self.policy.snapshot(service), Some(s) if s.failures > 0)
                {
                    let failures = self.policy.snapshot(service).unwrap().failures;
                    let delay = (10u64 << (failures - 1).min(2)).min(40);
                    if now < self.last_exit[service].saturating_add(delay) {
                        return Err(Status::IoFailure);
                    }
                    self.backoff_observed += 1;
                }
                let launch = match door_sys::process_launch(
                    self.authority,
                    manifest::IDS[service],
                    0,
                    self.server.port,
                    child_mode,
                ) {
                    Ok(launch) => launch,
                    Err(error) => {
                        self.policy
                            .exited(service, instance, now)
                            .map_err(|_| Status::IoFailure)?;
                        return Err(error);
                    }
                };
                let identity = door_sys::identity(launch.control)?;
                self.stage = 13 + service as u8;
                if identity.pid != launch.pid
                    || identity.manifest != manifest::IDS[service]
                    || identity.uid != manifest::SERVICES[service].account
                    || identity.capabilities != manifest::CONTRACTS[service + 1].policy.caps
                    || identity.session != 0
                {
                    return Err(Status::AccessDenied);
                }
                self.launched[service] += 1;
                self.children[service] = Some(Child {
                    launch,
                    identity,
                    policy_instance: instance,
                });
            }
            Action::Stop { service, .. } => {
                self.stage = 20 + service as u8;
                if self.children[service].is_some() {
                    self.withdraw(service);
                    let child = self.children[service].as_ref().unwrap();
                    let message = Message::new(
                        opcode::STOP,
                        2,
                        child.identity.instance,
                        Code::Success,
                        &[],
                        0,
                    )?;
                    match door_sys::channel_send(child.launch.control, message.bytes(), &[]) {
                        Ok(()) | Err(Status::PeerClosed) => (),
                        Err(error) => return Err(error),
                    }
                }
            }
            Action::Kill { service, .. } => {
                self.stage = 23 + service as u8;
                self.withdraw(service);
                if let Some(child) = &self.children[service] {
                    if service == 2 && self.startup.mode == mode::RESTART && self.launched[2] == 1 {
                        self.readiness_timeout = true;
                    }
                    door_sys::process_terminate(child.launch.process, -150)?;
                }
            }
            Action::Fatal { .. } => {
                self.failure = true;
                self.stopping = true;
                self.policy.begin_shutdown();
            }
        }
        Ok(())
    }
    fn withdraw(&mut self, service: usize) {
        if self.registry.ports[service] != 0 {
            let _ = door_sys::close(self.registry.ports[service]);
            self.registry.ports[service] = 0;
        }
    }
    fn observations(&mut self, now: u64) -> Result<(), Status> {
        for service in 0..3 {
            self.stage = 30 + service as u8;
            let Some(child) = &self.children[service] else {
                continue;
            };
            match door_sys::process_wait(child.launch.process, now) {
                Ok(code) => {
                    self.server.note_progress();
                    if self.startup.mode == mode::FAILURE && service == 0 && code != -120 {
                        return Err(Status::IoFailure);
                    }
                    let child = self.children[service].take().unwrap();
                    self.withdraw(service);
                    self.policy
                        .exited(service, child.policy_instance, now)
                        .map_err(|_| Status::IoFailure)?;
                    self.last_exit[service] = now;
                    door_sys::close(child.launch.control)?;
                    door_sys::close(child.launch.process)?;
                    continue;
                }
                Err(Status::Timeout | Status::WouldBlock) => (),
                Err(error) => return Err(error),
            }
            if !matches!(self.policy.state(service), Some(State::Starting { .. })) {
                continue;
            }
            let child = self.children[service].as_ref().unwrap();
            let mut bytes = [0; door_abi::MAX_PACKET];
            let mut transfers = [Transfer::default(); door_abi::MAX_TRANSFERS];
            match door_sys::channel_receive(child.launch.control, &mut bytes, &mut transfers) {
                Ok(metadata) => {
                    self.server.note_progress();
                    self.stage = 33 + service as u8;
                    let frame = Frame::decode(&bytes[..metadata.packet_bytes as usize])?;
                    let identity = door_sys::identity(child.launch.control)?;
                    let good = metadata.sender_pid == child.identity.pid
                        && identity == child.identity
                        && frame.header.opcode == opcode::READY
                        && frame.header.request_id == 1
                        && frame.instance == child.identity.instance
                        && frame.code == Code::Success
                        && frame.body == manifest::IDS[service].to_le_bytes()
                        && metadata.handle_count == 1
                        && frame.header.handle_count == 1;
                    if !good {
                        for transfer in &transfers[..metadata.handle_count as usize] {
                            let _ = door_sys::close(transfer.handle);
                        }
                        return Err(Status::AccessDenied);
                    }
                    server::transferred_capability(transfers[0].handle, HandleKind::Port)?;
                    self.registry.ports[service] = transfers[0].handle;
                    self.policy
                        .ready(service, child.policy_instance, now)
                        .map_err(|_| Status::IoFailure)?;
                    line!(
                        "DOOR_P08 READY manifest={} pid={} instance={}",
                        manifest::IDS[service],
                        identity.pid,
                        identity.instance
                    );
                }
                Err(Status::WouldBlock | Status::PeerClosed) => (),
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
    fn all_ready(&self) -> bool {
        (0..3).all(|i| matches!(self.policy.state(i), Some(State::Ready { .. })))
    }
    fn applications(&mut self, now: u64) -> Result<(), Status> {
        self.stage = 40;
        if self.startup.mode == mode::DESKTOP {
            return self.desktop_applications(now);
        }
        if self.startup.mode == mode::SESSION {
            return self.ordinary_applications(now);
        }
        let application_manifest =
            if matches!(self.startup.mode, mode::WINDOWS_BASE | mode::WINDOWS_REOPEN) {
                door_abi::platform::manifest::WINDOWS_BASE
            } else if self.startup.mode == mode::WINDOWS {
                door_abi::platform::manifest::WINDOWS
            } else {
                manifest::WORKLOAD
            };
        if self.stopping || !self.all_ready() {
            return Ok(());
        }
        if self.startup.mode == mode::RESTART && !self.restarted {
            if !self.restarting {
                let child = self.children[0].as_ref().unwrap();
                self.old_instance = child.identity.instance;
                self.old_port = door_sys::duplicate(self.registry.ports[0], manifest::PORT_RIGHTS)?;
                door_sys::process_terminate(child.launch.process, -151)?;
                self.restarting = true;
                return Ok(());
            }
            let instance = self.children[0].as_ref().unwrap().identity.instance;
            if instance == self.old_instance {
                return Ok(());
            }
            match door_sys::port_connect(self.old_port) {
                Err(Status::PeerClosed | Status::AccessDenied) => (),
                Ok(channel) => {
                    let _ = door_sys::close(channel);
                    return Err(Status::IoFailure);
                }
                Err(error) => return Err(error),
            }
            door_sys::close(self.old_port)?;
            self.old_port = 0;
            if !self.readiness_timeout || self.backoff_observed == 0 {
                return Err(Status::IoFailure);
            }
            let _ = door_sys::write(b"DOOR_P08 READINESS PASS timeout=1 backoff=1\n");
            line!(
                "DOOR_P08 RESTART PASS old={} new={} stale=1",
                self.old_instance,
                instance
            );
            self.restarted = true;
        }
        if self.revoking {
            if !self.revocation_probe(now)? {
                return Ok(());
            }
        } else if let Some(app) = &self.app {
            self.stage = 41;
            match door_sys::process_wait(app.process, now) {
                Ok(code) => {
                    let expected =
                        if matches!(self.startup.mode, mode::WINDOWS_BASE | mode::WINDOWS_REOPEN) {
                            74
                        } else if self.startup.mode == mode::WINDOWS {
                            73
                        } else {
                            71 + self.completed as i64
                        };
                    if code != expected {
                        return Err(Status::IoFailure);
                    }
                    let app = self.app.take().unwrap();
                    if self.startup.mode == mode::WINDOWS {
                        line!("DOOR_P09 APPLICATION PASS pid={} exit={}", app.pid, code);
                    }
                    if matches!(self.startup.mode, mode::WINDOWS_BASE | mode::WINDOWS_REOPEN) {
                        line!("DOOR_P10 APPLICATION PASS pid={} exit={}", app.pid, code);
                    }
                    door_sys::close(app.control)?;
                    door_sys::close(app.process)?;
                    if self.startup.mode == mode::RESTART && self.completed == 1 {
                        self.stage = 45;
                        self.app = Some(door_sys::process_launch(
                            self.authority,
                            manifest::WORKLOAD,
                            self.session,
                            self.server.port,
                            mode::FAILURE,
                        )?);
                        self.probe_deadline =
                            now.checked_add(500).ok_or(Status::ResourceExhausted)?;
                        self.revoking = true;
                        return Ok(());
                    }
                    let mut sessions =
                        door_service_client::Session::connect_port(self.registry.ports[1])
                            .map_err(|_| Status::IoFailure)?;
                    self.stage = 42;
                    sessions
                        .logoff(self.session)
                        .map_err(|_| Status::IoFailure)?;
                    // Revoked capability cannot authorize a new process even when
                    // init retains the handle: epoch state, not handle possession.
                    if !matches!(
                        door_sys::process_launch(
                            self.authority,
                            application_manifest,
                            self.session,
                            self.server.port,
                            mode::REOPEN
                        ),
                        Err(Status::AccessDenied)
                    ) {
                        return Err(Status::IoFailure);
                    }
                    door_sys::close(self.session)?;
                    self.session = 0;
                    self.completed += 1;
                }
                Err(Status::Timeout | Status::WouldBlock) => return Ok(()),
                Err(error) => return Err(error),
            }
        }
        let target = if matches!(
            self.startup.mode,
            mode::DEMO | mode::WINDOWS | mode::WINDOWS_BASE | mode::WINDOWS_REOPEN
        ) {
            1
        } else {
            2
        };
        if self.completed == target {
            self.stopping = true;
            self.policy.begin_shutdown();
            return Ok(());
        }
        let permit = self.startup.base.inherited[if self.completed == 0 {
            inherited::LOGON_FIRST
        } else {
            inherited::LOGON_SECOND
        }]
        .handle;
        let mut sessions = door_service_client::Session::connect_port(self.registry.ports[1])
            .map_err(|_| Status::IoFailure)?;
        self.stage = 43;
        self.session = sessions.logon(permit).map_err(|error| {
            log(format_args!("DOOR_P08 INIT LOGON error={error:?}"));
            Status::IoFailure
        })?;
        let app_mode = if self.startup.mode == mode::RESTART {
            mode::REOPEN
        } else {
            self.startup.mode
        };
        self.app = Some(door_sys::process_launch(
            self.authority,
            application_manifest,
            self.session,
            self.server.port,
            app_mode,
        )?);
        self.stage = 44;
        Ok(())
    }
    fn revocation_probe(&mut self, now: u64) -> Result<bool, Status> {
        self.stage = 46;
        if now >= self.probe_deadline {
            return Err(Status::Timeout);
        }
        let app = self.app.as_ref().ok_or(Status::IoFailure)?;
        if !self.probe_revoked {
            let mut bytes = [0; door_abi::MAX_PACKET];
            let mut transfers = [Transfer::default(); door_abi::MAX_TRANSFERS];
            let metadata = match door_sys::channel_receive(app.control, &mut bytes, &mut transfers)
            {
                Ok(metadata) => metadata,
                Err(Status::WouldBlock) => return Ok(false),
                Err(error) => return Err(error),
            };
            let validate = || {
                let peer = door_sys::identity(app.control)?;
                let frame = Frame::decode(&bytes[..metadata.packet_bytes as usize])?;
                let required =
                    door_abi::rights::QUERY | door_abi::rights::WAIT | door_abi::rights::WRITE;
                if peer.pid != app.pid
                    || peer.manifest != manifest::WORKLOAD
                    || peer.uid != 1002
                    || peer.session == 0
                    || metadata.sender_pid != app.pid
                    || metadata.handle_count != 1
                    || frame.header.opcode != opcode::REVOCATION_READY
                    || frame.header.request_id != 1
                    || frame.header.handle_count != 1
                    || frame.code != Code::Success
                    || frame.instance != peer.instance
                    || frame.body != 2u64.to_le_bytes()
                    || transfers[0].rights != required
                    || transfers[0].mode != 0
                {
                    return Err(Status::AccessDenied);
                }
                let grant = door_sys::inspect(transfers[0].handle)?;
                if grant[0] != HandleKind::Mutex as u64 || grant[1] != required {
                    return Err(Status::AccessDenied);
                }
                Ok(peer)
            };
            let peer = match validate() {
                Ok(peer) => peer,
                Err(error) => {
                    for transfer in &transfers[..metadata.handle_count as usize] {
                        let _ = door_sys::close(transfer.handle);
                    }
                    return Err(error);
                }
            };
            self.probe_session_id = peer.session;
            self.probe_mutex = transfers[0].handle;
            // The worker owns this mutex before entering CONDITION_WAIT. This
            // acquisition proves it released that ownership into the wait. Keep
            // it owned by this live foreign process throughout forced retirement.
            if door_sys::wait(
                self.probe_mutex,
                now.checked_add(100).ok_or(Status::ResourceExhausted)?,
            )? != 0
            {
                return Err(Status::IoFailure);
            }
            self.stage = 47;
            let mut sessions = door_service_client::Session::connect_port(self.registry.ports[1])
                .map_err(|_| Status::IoFailure)?;
            sessions
                .logoff(self.session)
                .map_err(|_| Status::IoFailure)?;
            if !matches!(
                door_sys::process_launch(
                    self.authority,
                    manifest::WORKLOAD,
                    self.session,
                    self.server.port,
                    mode::REOPEN
                ),
                Err(Status::AccessDenied)
            ) {
                return Err(Status::IoFailure);
            }
            self.probe_revoked = true;
        }
        self.stage = 48;
        match door_sys::process_wait(app.process, now) {
            Ok(-13) => {
                if self.probe_mutex == 0 {
                    return Err(Status::IoFailure);
                }
                // A canceled victim must retire without acquiring this mutex.
                // Successful unlock here also proves init retained ownership.
                door_sys::signal(self.probe_mutex, 1)?;
                door_sys::close(self.probe_mutex)?;
                self.probe_mutex = 0;
                let app = self.app.take().unwrap();
                door_sys::close(app.control)?;
                door_sys::close(app.process)?;
                door_sys::close(self.session)?;
                self.session = 0;
                self.completed += 1;
                self.probe_done = true;
                self.revoking = false;
                line!(
                    "DOOR_P08 REVOKE PASS session={} threads=2 denied=1 foreign_mutex=1",
                    self.probe_session_id
                );
                Ok(true)
            }
            Ok(_) => Err(Status::IoFailure),
            Err(Status::Timeout | Status::WouldBlock) => Ok(false),
            Err(error) => Err(error),
        }
    }
    fn step(&mut self) -> Result<bool, Status> {
        let now = door_sys::time()?;
        if matches!(self.startup.mode, mode::SESSION | mode::DESKTOP) {
            let jobs = self.registry.jobs.iter().flatten().count();
            self.registry.reap_jobs(now)?;
            if jobs != self.registry.jobs.iter().flatten().count() {
                self.server.note_progress();
            }
        }
        self.observations(now)?;
        for action in self.policy.step(now).iter() {
            self.act(action, now)?;
        }
        self.server.poll(&mut self.registry)?;
        self.applications(now)?;
        if self.stopping && self.policy.quiescent() {
            self.server.disconnect_all(&mut self.registry);
            if self.failure {
                if self.startup.mode != mode::FAILURE
                    || self.launched[0] != 4
                    || self.backoff_observed != 3
                    || self.completed != 0
                {
                    return Err(Status::IoFailure);
                }
                let _ = door_sys::write(
                    b"DOOR_P08 FAILURE PASS launches=4 backoff=1 refused=1 shutdown=1\n",
                );
            } else {
                if self.startup.mode == mode::RESTART && !self.probe_done {
                    return Err(Status::IoFailure);
                }
                line!(
                    "DOOR_P08 SUPERVISOR PASS services=3 apps={} restart={} sessions={} shutdown=1",
                    self.completed,
                    u8::from(self.restarted),
                    self.completed
                );
            }
            return Ok(false);
        }
        Ok(true)
    }
}
pub fn run(startup: StartupV2) -> i64 {
    let mut init = match Init::new(startup) {
        Ok(init) => init,
        Err(_) => return -160,
    };
    loop {
        match init.step() {
            Ok(true) => {
                let stable_desktop = init.startup.mode == mode::DESKTOP
                    && init.all_ready()
                    && !init.stopping
                    && init.registry.power.is_none();
                let result = if stable_desktop {
                    init.server.idle_backoff()
                } else {
                    door_sys::yield_now()
                };
                if result.is_err() {
                    return -161;
                }
            }
            Ok(false) => {
                return if init.registry.power == Some(true) {
                    81
                } else {
                    80
                };
            }
            Err(error) => {
                line!(
                    "DOOR_P08 INIT FAIL stage={} status={}",
                    init.stage,
                    error as u64
                );
                return -162;
            }
        }
    }
}
struct Registry {
    ports: [Handle; 3],
    launch: Handle,
    port: Handle,
    session: Handle,
    session_id: u64,
    /// The service account's logon session (LocalSystem, opened at the
    /// first sign-in of the boot and kept for the boot): services run in
    /// it.
    service_session: Handle,
    service_session_id: u64,
    /// The manager's services loaded (once a boot), and the automatic
    /// ones started (once a boot, as Windows starts them at boot).
    scm_ready: bool,
    services_autostarted: bool,
    shell: u64,
    power: Option<bool>,
    jobs: [Option<applications::Job>; 8],
    /// Launch arguments a caller is staging (ARGUMENTS_STAGE).
    argument_stage: applications::Arguments,
    /// The pipe sections of the EXECUTE being handled (STARTF_USESTDHANDLES).
    standard: [Handle; 3],
    /// Staged arguments of started children, until each fetches its own.
    long_arguments: [applications::Arguments; 4],
    /// The Service Control Manager (`door_service_protocol::scm`).
    scm: scm::Manager,
    /// The process ID of the last successful EXECUTE (service launches).
    last_launched: Option<u64>,
    /// The Event Log's rings (`door_service_protocol::eventlog::Log`).
    event_logs: [eventlog::Ring; 4],
    /// Message text connections sent ahead of a REPORT (REPORT_TEXT).
    event_text: [Option<eventlog::Staged>; eventlog::STAGES],
    /// The Task Scheduler.
    tasks: tasks::Scheduler,
    com: com::Broker,
    /// The signed-in user, and the working-folder authority the desktop
    /// shell handed over (0: none yet).
    session_uid: u32,
    session_directory: Handle,
    window: Handle,
    window_instance: u64,
    /// door.network's port (P15), once the network service is ready.
    network: Handle,
    /// door.audio's port (P15), once the sound service is ready.
    audio: Handle,
    gui_clients: [Option<(u64, Handle)>; door_service_protocol::application::DESKTOP_APPS + 1],
    desktop_admission: door_service_protocol::application::DesktopAdmission,
    gui_instances: [u64; door_service_protocol::application::DESKTOP_APPS],
    gui_power: Option<bool>,
    /// The session's windowless search host (pid, process) and instance.
    search_host: Option<(u64, Handle)>,
    search_instance: u64,
    signin_requested: bool,
    /// The sign-in is the second local account's (uid 1002).
    signin_second: bool,
    /// Display-service preferences waiting for the desktop step to persist.
    shell_layout_pending: Option<door_service_protocol::desktop::ShellLayout>,
    controls_pending: Option<door_service_protocol::desktop::Controls>,
    /// Settings the window server changed in place (`window::PERSIST_SETTING`),
    /// by `persist_slot`: DesktopAppearance, then the preference words.
    settings_pending: [Option<u32>; 1 + door_service_protocol::preferences::ALL.len()],
    /// door.notifications for the signed-in session (P14).
    notifications: crate::notifications::Center,
    /// Running Start-search accessories (pid, instance), `ACCESSORY_COMMANDS` order.
    accessories: [Option<(u64, u64)>; door_service_protocol::application::ACCESSORIES],
    /// The elevated Terminal (pid, instance), when running.
    admin_terminal: Option<(u64, u64)>,
}
#[cfg(test)]
mod admission_tests {
    use super::{Identity, admitted};
    const USER: u64 = 7;
    const SERVICE: u64 = 9;
    fn peer(uid: u32, session: u64) -> Identity {
        Identity {
            pid: 40,
            uid,
            session,
            epoch: 1,
            ..Identity::default()
        }
    }
    #[test]
    fn the_elevation_and_session_boundaries() {
        let system = door_abi::platform::SERVICE_ACCOUNT;
        // The signed-in user's processes, and services in their session.
        assert!(admitted(peer(1001, USER), Some(USER), Some(SERVICE)));
        assert!(admitted(peer(system, SERVICE), Some(USER), Some(SERVICE)));
        // A user's process naming the service session is not a service.
        assert!(!admitted(peer(1001, SERVICE), Some(USER), Some(SERVICE)));
        // Before the service session opens, or with nobody signed in.
        assert!(!admitted(peer(system, SERVICE), Some(USER), None));
        assert!(!admitted(peer(1001, USER), None, Some(SERVICE)));
        // Another user's session, unauthenticated peers, system uids and
        // capability-bearing (Doors service) identities.
        assert!(!admitted(peer(1002, 8), Some(USER), Some(SERVICE)));
        assert!(!admitted(
            Identity {
                epoch: 0,
                ..peer(1001, USER)
            },
            Some(USER),
            Some(SERVICE)
        ));
        assert!(!admitted(peer(2, USER), Some(USER), Some(SERVICE)));
        assert!(!admitted(
            Identity {
                capabilities: 1,
                ..peer(1001, USER)
            },
            Some(USER),
            Some(SERVICE)
        ));
    }
}
