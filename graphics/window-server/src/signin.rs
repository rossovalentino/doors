//! Window-server side of the credential gate (P14-LOGON, ADR 0020): routes
//! signed-out and locked input to `door_window::login::Gate`, talks to the
//! session service's credential store without blocking the display loop, and
//! turns an accepted secret into sign-in or unlock. There is no passwordless
//! path: an unreachable store keeps the gate closed and is retried each second.
use super::*;
use door_window::login::{Action, Backend, Gate, Page, Polled, Purpose, Request};

/// Session-service connection behind the gate. The service's port is
/// resolved through Init without blocking: Init calls this server while it
/// binds a session (after sign-in and after every server restart), so a
/// blocking resolve here deadlocked both until their deadlines and the
/// restarted desktop never came back (`desktop-recovery`).
pub(crate) struct CredentialService {
    registry: Handle,
    client: Option<door_service_client::Credentials>,
    resolving: Option<door_service_client::Init>,
    /// Earliest tick for the next resolve after one failed.
    retry_at: u64,
}
impl CredentialService {
    pub(crate) fn new(registry: Handle) -> Self {
        Self {
            registry,
            client: None,
            resolving: None,
            retry_at: 0,
        }
    }
    /// Drive the connection without blocking; whether it is up.
    pub(crate) fn connected(&mut self, now: u64) -> bool {
        if self.client.as_ref().is_some_and(|c| c.needs_reconnect()) {
            self.client = None;
        }
        if self.client.is_some() {
            return true;
        }
        if let Some(init) = self.resolving.as_mut() {
            match init.poll_resolve(now) {
                Ok(None) => return false,
                Ok(Some(port)) => {
                    self.client = door_service_client::Credentials::connect_port(port).ok();
                    let _ = door_sys::close(port);
                }
                Err(_) => {}
            }
            self.resolving = None;
            if self.client.is_none() {
                self.retry_at = now.saturating_add(50);
            }
            return self.client.is_some();
        }
        if now >= self.retry_at {
            self.resolving = door_service_client::Init::connect_now(self.registry)
                .ok()
                .and_then(|mut init| {
                    init.begin_resolve(door_abi::platform::manifest::SESSION, now)
                        .ok()
                        .map(|()| init)
                });
            if self.resolving.is_none() {
                self.retry_at = now.saturating_add(50);
            }
        }
        false
    }
    fn client(&mut self) -> Option<&mut door_service_client::Credentials> {
        if self.client.as_ref().is_some_and(|c| c.needs_reconnect()) {
            self.client = None;
        }
        self.client.as_mut()
    }
}
impl Backend for CredentialService {
    fn accounts(&mut self) -> Option<door_service_protocol::credential::Accounts> {
        self.client()?.accounts().ok()
    }
    fn select(&mut self, uid: u32) -> bool {
        self.client().is_some_and(|c| c.select(uid).is_ok())
    }
    fn begin(&mut self, request: Request<'_>, now: u64) -> bool {
        // Not connected yet: the gate's caller connects first (`connected`).
        let Some(client) = self.client() else {
            return false;
        };
        match request {
            Request::State => client.begin_state(now),
            Request::Enroll(kind, secret) => client.begin_enroll(kind, secret, now),
            Request::Verify(kind, secret) => client.begin_verify(kind, secret, now),
        }
        .is_ok()
    }
    fn poll(&mut self, now: u64) -> Polled {
        let Some(client) = self.client.as_mut() else {
            return Polled::Failed;
        };
        match client.poll(now) {
            Ok(None) => Polled::Pending,
            Ok(Some(door_service_client::CredentialReply::State(state))) => Polled::State(state),
            Ok(Some(door_service_client::CredentialReply::Enrolled(state))) => {
                Polled::Enrolled(state)
            }
            Ok(Some(door_service_client::CredentialReply::Verdict(verdict))) => {
                Polled::Verdict(verdict)
            }
            // The gate never changes credentials (Settings does).
            Ok(Some(door_service_client::CredentialReply::Updated(_))) => Polled::Failed,
            Err(_) => {
                if client.needs_reconnect() {
                    self.client = None;
                }
                Polled::Failed
            }
        }
    }
}

/// Gate state owned by the window server.
pub(crate) struct SignIn {
    pub(crate) gate: Gate,
    pub(crate) service: CredentialService,
    /// A standard user's consent prompt is checking an administrator's
    /// password.
    pub(crate) uac_checking: bool,
    started: bool,
    /// The gate was reset for the current lock.
    armed_for_lock: bool,
    retry_at: u64,
}
impl SignIn {
    pub(crate) fn new(registry: Handle) -> Self {
        Self {
            gate: Gate::new(Purpose::SignIn),
            service: CredentialService::new(registry),
            uac_checking: false,
            started: false,
            armed_for_lock: false,
            retry_at: 0,
        }
    }
}

impl Desktop {
    fn gate_action(&mut self, action: Action) {
        match action {
            Action::None => (),
            Action::Redraw => self.engine.invalidate(),
            Action::Granted => {
                self.engine.invalidate();
                if self.shell.locked {
                    self.gate_unlock();
                } else if self.session.is_none() {
                    self.sign_in();
                }
            }
        }
    }
    /// Unlock after an accepted secret (the only unlock path).
    fn gate_unlock(&mut self) {
        self.shell.locked = false;
        self.lock_pixels = Vec::new();
        self.shell.lock_pressed = false;
        self.signin.armed_for_lock = false;
        // Locked keys went to the gate, so the Win/Alt shortcut tracker never
        // saw the releases of the chord that locked (Win+L): without this the
        // next "l" typed after unlocking would lock again.
        self.system_keys.reset();
        self.shortcut_keys = 0;
        self.engine.invalidate();
        self.play_sound(door_service_protocol::audio::sound::UNLOCK);
        self.restore_shell_focus();
        let _ = door_sys::write(b"DOOR_P12 UNLOCKED\n");
    }
    /// A consent prompt for this session: a standard user's gets the
    /// credential prompt with the first administrator account.
    pub(super) fn consent_for_session(
        &mut self,
        consent: door_window::uac::Consent,
    ) -> door_window::uac::Consent {
        let Some((_, _, uid)) = self.session else {
            return consent;
        };
        let Some(accounts) = self.signin.service.accounts() else {
            return consent;
        };
        if accounts.get(uid).is_none_or(|a| a.administrator) {
            return consent;
        }
        match accounts.iter().find(|a| a.administrator) {
            Some(admin) => consent.with_credential(admin.uid, admin.name()),
            None => consent,
        }
    }
    /// Yes on a credential prompt: check the administrator's password
    /// (the prompt stays up meanwhile). False when it couldn't start.
    pub(super) fn uac_check(&mut self) -> bool {
        let now = door_sys::time().unwrap_or(0);
        let Some(cred) = self.shell.uac.as_mut().and_then(|c| c.credential.as_mut()) else {
            return false;
        };
        let service = &mut self.signin.service;
        if !service.select(cred.uid)
            || !service.begin(
                Request::Verify(
                    door_service_protocol::credential::Kind::Password,
                    cred.secret(),
                ),
                now,
            )
        {
            cred.clear();
            cred.notice = door_window::uac::CredentialNotice::Incorrect;
            if let Some((_, _, uid)) = self.session {
                service.select(uid);
            }
            return false;
        }
        cred.clear();
        cred.notice = door_window::uac::CredentialNotice::Checking;
        self.signin.uac_checking = true;
        self.engine.invalidate();
        true
    }
    /// Polls a running credential check; an accepted password approves.
    fn uac_tick(&mut self, now: u64) {
        if !self.signin.uac_checking {
            return;
        }
        let polled = self.signin.service.poll(now);
        if polled == Polled::Pending {
            return;
        }
        self.signin.uac_checking = false;
        // The session's own account again (lock checks it).
        if let Some((_, _, uid)) = self.session {
            self.signin.service.select(uid);
        }
        let Some(consent) = self.shell.uac.as_mut() else {
            return;
        };
        use door_service_protocol::credential::Verdict;
        use door_window::uac::CredentialNotice;
        let accepted = matches!(polled, Polled::Verdict(Verdict::Accepted));
        if let Some(cred) = consent.credential.as_mut() {
            cred.notice = match polled {
                Polled::Verdict(Verdict::Throttled { retry_after, .. }) => {
                    CredentialNotice::Throttled(retry_after.div_ceil(100))
                }
                _ => CredentialNotice::Incorrect,
            };
        }
        let _ = door_sys::write(if accepted {
            b"DOOR_P16 UAC CREDENTIAL accepted\n" as &[u8]
        } else {
            b"DOOR_P16 UAC CREDENTIAL rejected\n"
        });
        if accepted {
            // Verified: the prompt approves as consent would.
            consent.credential = None;
            self.uac_answer(door_window::uac::Answer::Yes);
        }
        self.engine.invalidate();
    }
    /// Every loop iteration: start, poll and (re)arm the gate.
    pub(super) fn gate_tick(&mut self) -> Result<(), Status> {
        let now = door_sys::time()?;
        // The credential connection, driven without blocking every tick.
        let connected = self.signin.service.connected(now);
        self.uac_tick(now);
        let signed_out = self.session.is_none();
        let locked = !signed_out && self.shell.locked;
        if !signed_out && !locked {
            self.signin.armed_for_lock = false;
            return Ok(());
        }
        let SignIn {
            gate,
            service,
            started,
            armed_for_lock,
            retry_at,
            ..
        } = &mut self.signin;
        let mut action = Action::None;
        // Until the credential service is reachable the gate stays on its
        // Loading page (it is never started against a missing service).
        if !connected {
        } else if locked && !*armed_for_lock {
            *armed_for_lock = true;
            *started = true;
            // Unlock checks the signed-in user's secret.
            if let Some((_, _, uid)) = self.session {
                gate.select_account(uid);
            }
            action = gate.reset(Purpose::Unlock, service, now);
        } else if signed_out && !*started {
            *started = true;
            action = gate.start(service, now);
        } else if gate.page() == Page::Unavailable && now >= *retry_at {
            // Startup ordering or a restarted session service: retry.
            *retry_at = now.saturating_add(100);
            action = gate.reset(gate.purpose(), service, now);
        }
        if action == Action::None {
            action = gate.tick(service, now);
        }
        self.gate_action(action);
        Ok(())
    }
    /// Signed-out or locked key event; always consumed.
    pub(super) fn gate_key(&mut self, code: u32, flags: u32) {
        let now = door_sys::time().unwrap_or(0);
        let SignIn { gate, service, .. } = &mut self.signin;
        let action = gate.key(service, code, flags, now);
        self.gate_action(action);
    }
    /// Signed-out or locked pointer button; always consumed.
    pub(super) fn gate_button(&mut self, button: u8, down: bool) {
        if button != 0 {
            return;
        }
        let now = door_sys::time().unwrap_or(0);
        let scale = self.engine.scale();
        let r = self.engine.output_rect();
        let point = (scale.logical(self.x), scale.logical(self.y));
        let size = (scale.logical_len(r.width()), scale.logical_len(r.height()));
        let SignIn { gate, service, .. } = &mut self.signin;
        let action = gate.pointer(service, point, size, down, now);
        self.gate_action(action);
    }
    /// What the trusted card paints this frame.
    pub(super) fn gate_view(&self) -> door_window::login::View {
        self.signin.gate.view(door_sys::time().unwrap_or(0))
    }
}
