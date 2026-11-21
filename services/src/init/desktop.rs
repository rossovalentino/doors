//! Optional desktop service lifecycle, separate from the foundational three-service
//! dependency graph. Restarts withdraw the endpoint before advertising a new incarnation.
use super::*;
use door_service_protocol::accessibility as a11y;
#[derive(Default)]
pub(super) struct State {
    server: Option<LaunchResult>,
    clients: [Option<LaunchResult>; door_service_protocol::application::DESKTOP_APPS],
    ready: bool,
    bound: bool,
    /// Session binding attempts that timed out in a row, and when to try
    /// again (`bind_session`).
    bind_failures: u32,
    bind_retry_at: u64,
    restarts: u32,
    deadline: u64,
    retry_at: u64,
    started: bool,
    signin_waiting: bool,
    probe: Option<LaunchResult>,
    probe_started: bool,
    probe_phase: u8,
    session_identity: Option<(u64, u64, u32)>,
    /// The Task Scheduler's next check (ticks).
    tasks_due: u64,
    /// The desktop shell (P14, manifest DESKTOP_SHELL; one per signed-in
    /// session). It also hosts Start file search, so it holds the former
    /// search-host slot of Init's desktop catalog.
    search: Option<LaunchResult>,
    search_restarts: u32,
    search_retry_at: u64,
    search_retiring: bool,
    /// When the running shell was launched (its restart budget recovers
    /// after `SHELL_STABLE_TICKS`), and incarnations launched so far.
    search_launched_at: u64,
    shell_incarnations: u32,
    /// Session input method editor (`door-ime`, manifest IME): one per
    /// signed-in session, restarted like the search host.
    ime: Option<LaunchResult>,
    ime_restarts: u32,
    ime_retry_at: u64,
    /// When the running IME was launched (its restart budget recovers
    /// after `IME_STABLE_TICKS` of uptime).
    ime_launched_at: u64,
    ime_retiring: bool,
    /// The adversarial window-test image: no search host and no IME.
    test_image: bool,
    /// Start-search accessories (`application::ACCESSORY_COMMANDS`): the
    /// component gallery and the Captions assistive client, at most one each.
    accessories: [Option<LaunchResult>; door_service_protocol::application::ACCESSORIES],
    accessories_retiring: bool,
    /// The elevated Terminal (User Account Control approved it).
    admin_terminal: Option<LaunchResult>,
    /// The P15 system services (manifests NETWORK and AUDIO), started with
    /// the desktop and restarted with backoff (at most three times).
    network: SystemService,
    audio: SystemService,
}
/// One desktop system service child and its restart state.
#[derive(Default)]
pub(super) struct SystemService {
    child: Option<LaunchResult>,
    ready: bool,
    restarts: u32,
    retry_at: u64,
    deadline: u64,
}
/// `window::PERSIST_SETTING`: the pending slot for a valid write of setting
/// id `setting`, or None. Slot 0 is `DesktopAppearance` (values below
/// `APPEARANCE_VALUES`, as storage accepts them), slots 1.. the preference
/// words 10..=`preferences::LAST`.
pub(super) fn persist_slot(setting: u32, value: u32) -> Option<usize> {
    match setting {
        2 if value < door_service_protocol::window::APPEARANCE_VALUES => Some(0),
        10..=door_service_protocol::preferences::LAST
            if door_service_protocol::preferences::valid(setting, value) =>
        {
            Some(setting as usize - 9)
        }
        _ => None,
    }
}
/// The setting id stored in pending slot `slot` (`persist_slot`).
fn persist_setting(slot: usize) -> u32 {
    if slot == 0 { 2 } else { slot as u32 + 9 }
}
/// A step of binding the session to the (possibly restarted) window server
/// failed: say which and why. A server that did not answer in time, or is
/// still starting or closing, is `Timeout` (retried by `bind_session`'s
/// caller); anything else stays fatal (`INIT FAIL stage=40`).
fn bind_failed(step: u32, error: door_service_client::Error) -> Status {
    use door_service_client::Error as E;
    use door_service_protocol::Code;
    line!("DOOR_P11 BIND FAILED step={} error={:?}", step, error);
    match error {
        E::Transport(Status::Timeout | Status::WouldBlock)
        | E::Service(Code::WouldBlock | Code::Closing | Code::StaleInstance) => Status::Timeout,
        _ => Status::IoFailure,
    }
}
/// Binding attempts that time out before the server is restarted, and the
/// ticks between them (10 s in all).
const BIND_ATTEMPTS: u32 = 20;
const BIND_RETRY_TICKS: u64 = 50;
/// Exit code Init gives the search host when the session ends.
pub(super) const SEARCH_HOST_RETIRED: i64 = -150;
/// Exit code Init gives the IME when the session ends.
pub(super) const IME_RETIRED: i64 = -151;
/// IME restarts allowed before Init gives up on the session IME.
const IME_RESTARTS: u32 = 3;
/// An IME that ran this long (60 s of 10 ms ticks) earns its full restart
/// budget back: only crash loops exhaust it.
const IME_STABLE_TICKS: u64 = 6000;
/// Restarts used after an IME exit at `now` that ran since `launched_at`,
/// and the earliest relaunch tick (backoff grows with each restart).
fn ime_after_exit(restarts: u32, launched_at: u64, now: u64) -> (u32, u64) {
    let restarts = if now.saturating_sub(launched_at) >= IME_STABLE_TICKS {
        1
    } else {
        restarts + 1
    };
    (restarts, now + 100 * u64::from(restarts))
}
/// Toast text for a rejected launch, with its diagnostic identifier (spec
/// §60: unsupported or failed launches say why, never silently).
fn launch_reason(error: Status) -> &'static str {
    match error {
        Status::ResourceExhausted => {
            "No room for another app (DOOR-LAUNCH-RESOURCES). Close an app and try again."
        }
        Status::NotFound => "The app isn't installed in this image (DOOR-LAUNCH-NOTFOUND).",
        Status::AccessDenied => "The app isn't allowed to start now (DOOR-LAUNCH-DENIED).",
        _ => "The app failed to start (DOOR-LAUNCH-FAILED).",
    }
}
/// Desktop shell restarts allowed in a row before Init gives up (the
/// window server keeps its fallback taskbar and every app meanwhile).
const SHELL_RESTARTS: u32 = 5;
/// A shell that ran this long (30 s) earns its restart budget back.
const SHELL_STABLE_TICKS: u64 = 3000;
/// Restarts used after a shell exit at `now` that ran since `launched_at`,
/// and the earliest relaunch tick: exponential backoff from 100 ms, capped
/// at 3.2 s.
fn shell_after_exit(restarts: u32, launched_at: u64, now: u64) -> (u32, u64) {
    let restarts = if now.saturating_sub(launched_at) >= SHELL_STABLE_TICKS {
        1
    } else {
        restarts + 1
    };
    (restarts, now + (10u64 << restarts.min(5)))
}
/// Exit code Init gives an accessory still running when the session ends.
pub(super) const ACCESSORY_RETIRED: i64 = -152;
/// Accessory launch identity (`ACCESSORY_COMMANDS` order).
const ACCESSORY_MANIFESTS: [u64; door_service_protocol::application::ACCESSORIES] = [
    door_abi::platform::manifest::GALLERY,
    door_abi::platform::manifest::ASSIST,
    door_abi::platform::manifest::CALCULATOR,
    door_abi::platform::manifest::CLOCK,
    door_abi::platform::manifest::PAINT,
    door_abi::platform::manifest::SNIPPING_TOOL,
    door_abi::platform::manifest::WEATHER,
    door_abi::platform::manifest::MAGNIFIER,
    door_abi::platform::manifest::NARRATOR,
    door_abi::platform::manifest::ON_SCREEN_KEYBOARD,
    door_abi::platform::manifest::VOICE_ACCESS,
];
/// What `State::activity` returns.
type Activity = (
    [u64; door_service_protocol::application::DESKTOP_APPS + 4],
    u32,
    u32,
    Option<(u64, u64, u32)>,
);
impl State {
    fn activity(&self) -> Activity {
        let mut pids = [0; door_service_protocol::application::DESKTOP_APPS + 4];
        pids[door_service_protocol::application::DESKTOP_APPS + 2] =
            self.search.as_ref().map_or(0, |p| p.pid);
        pids[door_service_protocol::application::DESKTOP_APPS + 3] =
            self.ime.as_ref().map_or(0, |p| p.pid);
        pids[0] = self.server.as_ref().map_or(0, |p| p.pid);
        pids[1] = self.probe.as_ref().map_or(0, |p| p.pid);
        for (i, child) in self.clients.iter().enumerate() {
            pids[i + 2] = child.as_ref().map_or(0, |p| p.pid);
        }
        let flags = u32::from(self.ready)
            | u32::from(self.bound) << 1
            | u32::from(self.started) << 2
            | u32::from(self.signin_waiting) << 3
            | u32::from(self.probe_started) << 4
            | u32::from(self.probe_phase) << 8;
        (pids, flags, self.restarts, self.session_identity)
    }
}
impl Init {
    pub(super) fn desktop_applications(&mut self, now: u64) -> Result<(), Status> {
        let mut desktop = core::mem::take(&mut self.desktop);
        let before = desktop.activity();
        let result = self.desktop_step(&mut desktop, now);
        if desktop.activity() != before {
            self.server.note_progress();
        }
        self.desktop = desktop;
        result
    }
    /// Bind the signed-in session to the window server (after sign-in, and
    /// after every server restart): its identity, theme, controls, layout,
    /// scale, input methods, region, accent and preference words. A server
    /// that is slow to answer (it just restarted) is retried, not fatal:
    /// binding is idempotent for the same session.
    fn bind_session(&mut self, d: &mut State) -> Result<(), Status> {
        let (session, epoch, uid) = d.session_identity.ok_or(Status::NotFound)?;
        let mut storage = door_service_client::Storage::connect_port(self.registry.ports[0])
            .map_err(|e| bind_failed(1, e))?;
        // A damaged stored value (InvalidRequest) binds the default: the
        // session's typed settings registry resets it on disk and tells
        // the user once (P14-REGISTRY), as Windows' shell falls back to
        // its defaults over a bad per-user value.
        let stored = |storage: &mut door_service_client::Storage, setting| match storage
            .get_setting_for(uid, setting)
        {
            Ok(value) => Ok(Some(value)),
            Err(door_service_client::Error::Service(
                door_service_protocol::Code::InvalidRequest,
            )) => Ok(None),
            Err(e) => Err(bind_failed(7, e)),
        };
        let theme = stored(
            &mut storage,
            door_service_client::Setting::DesktopAppearance,
        )?
        .filter(|v| *v < door_service_protocol::window::APPEARANCE_VALUES)
        .unwrap_or(0);
        let controls = stored(&mut storage, door_service_client::Setting::DesktopControls)?
            .and_then(door_service_protocol::desktop::Controls::from_bits)
            .unwrap_or(door_service_protocol::desktop::Controls::DEFAULT);
        // The existing adversarial image exercises every accepted binding
        // form. Each SDK call opens a fresh connection; require completed
        // scene records in the host suite so a later40 cannot mask legacy24.
        let mut flag = [0; 32];
        match door_sys::boot_file_read(self.authority, "system/window-tests.conf", 0, &mut flag) {
            Ok(n) if &flag[..n] == b"window-tests-v1\n" => {
                door_service_client::window::WindowClient::bind(
                    self.registry.window,
                    session,
                    epoch,
                    uid,
                )
                .map_err(|e| bind_failed(2, e))?;
                door_service_client::window::WindowClient::bind_theme(
                    self.registry.window,
                    session,
                    epoch,
                    uid,
                    theme & 1 != 0,
                )
                .map_err(|e| bind_failed(3, e))?;
                // DISPLAY-01: only this image arms the presentation
                // fault chord in the window server.
                door_service_client::window::WindowClient::enable_test_hooks(self.registry.window)
                    .map_err(|e| bind_failed(4, e))?;
            }
            Err(Status::NotFound) => (),
            _ => return Err(Status::InvalidArgument),
        }
        door_service_client::window::WindowClient::bind_controls(
            self.registry.window,
            session,
            epoch,
            uid,
            theme,
            controls,
        )
        .map_err(|e| bind_failed(5, e))?;
        // The session user's persisted shell layout (pins, Start view,
        // desktop sort). Unset values read as the Windows defaults.
        let flags = storage
            .get_setting_for(uid, door_service_client::Setting::ShellLayout)
            .unwrap_or(0);
        let order = storage
            .get_setting_for(uid, door_service_client::Setting::ShellOrder)
            .unwrap_or(0);
        if let Some(layout) = door_service_protocol::desktop::ShellLayout::from_parts(flags, order)
            && layout != door_service_protocol::desktop::ShellLayout::DEFAULT
        {
            door_service_client::window::WindowClient::bind_shell_layout(
                self.registry.window,
                layout,
            )
            .map_err(|e| bind_failed(6, e))?;
        }
        // Display scale (spec §57): the stored preference, 0 (unset) =
        // automatic, which the server already uses. A value this output
        // does not offer is refused and automatic stays in effect.
        let scale = storage
            .get_setting_for(uid, door_service_client::Setting::ScalePercent)
            .unwrap_or(0);
        if scale != 0 {
            match door_service_client::window::WindowClient::bind_display_scale(
                self.registry.window,
                scale,
            ) {
                Ok(effective) => {
                    line!(
                        "DOOR_P12 DISPLAY_SCALE bound preference={} effective={}",
                        scale,
                        effective
                    )
                }
                Err(_) => line!("DOOR_P12 DISPLAY_SCALE bound preference={} refused", scale),
            }
        }
        // Keyboards and input methods (Settings > Time & language >
        // Typing). Unset or invalid values keep the US keyboard.
        let methods = storage
            .get_setting_for(uid, door_service_client::Setting::InputMethods)
            .unwrap_or(1);
        if methods != 1 {
            match door_service_client::text_input::apply_input_methods(
                self.registry.window,
                methods,
            ) {
                Ok(()) => line!("DOOR_P12 INPUT_METHODS bound preferences={}", methods),
                Err(_) => line!(
                    "DOOR_P12 INPUT_METHODS bound preferences={} refused",
                    methods
                ),
            }
        }
        // Time zone and regional format (Settings > Time & language).
        // Unset values are UTC and English (United States), which the
        // taskbar clock already uses.
        let zone = storage
            .get_setting_for(uid, door_service_client::Setting::TimeZone)
            .unwrap_or(16);
        let region = storage
            .get_setting_for(uid, door_service_client::Setting::RegionalFormat)
            .unwrap_or(1);
        if (zone, region) != (16, 1) {
            match door_service_client::window::WindowClient::bind_time_locale(
                self.registry.window,
                zone,
                region,
            ) {
                Ok(()) => line!("DOOR_P12 TIME_LOCALE bound zone={} region={}", zone, region),
                Err(_) => line!(
                    "DOOR_P12 TIME_LOCALE bound zone={} region={} refused",
                    zone,
                    region
                ),
            }
        }
        // Accent colour (Settings > Personalization > Colors). Unset is
        // Door's default blue, which the window server already uses.
        let accent = storage
            .get_setting_for(uid, door_service_client::Setting::AccentColor)
            .ok()
            .and_then(door_service_protocol::desktop::Accent::from_bits)
            .unwrap_or_default();
        if accent != door_service_protocol::desktop::Accent::DEFAULT {
            match door_service_client::window::WindowClient::bind_accent(
                self.registry.window,
                accent,
            ) {
                Ok(()) => line!("DOOR_P14 ACCENT bound value={:#x}", accent.bits()),
                Err(_) => line!("DOOR_P14 ACCENT bound value={:#x} refused", accent.bits()),
            }
        }
        // P14 preference words (accent surfaces, dark-mode schedule,
        // sign-in-away lock, lock screen, regional overrides, clipboard,
        // search history, End task, notifications): every word, unset
        // ones as their clean-Windows default.
        let mut refused = 0;
        for p in door_service_protocol::preferences::ALL {
            let value = door_service_client::Setting::from_id(p.id)
                .and_then(|s| storage.get_setting_for(uid, s).ok())
                .filter(|v| (p.valid)(*v))
                .unwrap_or(p.default);
            if door_service_client::window::WindowClient::bind_preference(
                self.registry.window,
                p.id,
                value,
            )
            .is_err()
            {
                refused += 1;
            }
        }
        line!("DOOR_P14 PREFERENCES bound refused={}", refused);
        // System > Notifications: Do not disturb (manual and scheduled)
        // and Focus for door.notifications (P14).
        self.registry.bind_notifications(&mut storage, uid);
        d.bound = true;
        line!("DOOR_P11 READY session={} restart={}", session, d.restarts);
        Ok(())
    }
    /// The registry slot holding a system service's port.
    fn service_port(&mut self, manifest: u64) -> &mut Handle {
        if manifest == door_abi::platform::manifest::AUDIO {
            &mut self.registry.audio
        } else {
            &mut self.registry.network
        }
    }
    /// Start a desktop system service (door-network, door-audio), take its
    /// port when it reports ready, and restart it (with backoff, at most
    /// three times) if it exits. `name` labels its serial records.
    fn service_step(
        &mut self,
        s: &mut SystemService,
        manifest: u64,
        name: &str,
        test_image: bool,
        now: u64,
    ) -> Result<(), Status> {
        if test_image {
            return Ok(());
        }
        if let Some(child) = s.child.as_ref() {
            if !s.ready {
                let mut bytes = [0; door_abi::MAX_PACKET];
                let mut transfers = [Transfer::default(); door_abi::MAX_TRANSFERS];
                match door_sys::channel_receive(child.control, &mut bytes, &mut transfers) {
                    Ok(meta) => {
                        let frame = Frame::decode(&bytes[..meta.packet_bytes as usize]);
                        let identity = door_sys::identity(child.control)?;
                        let valid = frame.is_ok_and(|f| {
                            f.header.opcode == opcode::READY
                                && f.header.request_id == 1
                                && f.code == Code::Success
                                && f.instance == identity.instance
                                && f.body == manifest.to_le_bytes()
                                && meta.handle_count == 1
                        });
                        if !valid {
                            for t in &transfers[..meta.handle_count as usize] {
                                let _ = door_sys::close(t.handle);
                            }
                            let _ = door_sys::process_terminate(child.process, -110);
                        } else {
                            server::transferred_capability(transfers[0].handle, HandleKind::Port)?;
                            *self.service_port(manifest) = transfers[0].handle;
                            s.ready = true;
                            line!("DOOR_P15 {} READY pid={}", name, child.pid);
                            if manifest == door_abi::platform::manifest::NETWORK {
                                self.firewall_start(transfers[0].handle);
                            }
                        }
                    }
                    Err(Status::WouldBlock) => {
                        if now >= s.deadline {
                            let _ = door_sys::process_terminate(child.process, -110);
                        }
                    }
                    Err(Status::PeerClosed) => (),
                    Err(e) => return Err(e),
                }
            }
            match door_sys::process_wait(child.process, now) {
                Err(Status::Timeout | Status::WouldBlock) => (),
                result => {
                    line!(
                        "DOOR_P15 {} EXIT pid={} code={}",
                        name,
                        child.pid,
                        result.unwrap_or(-1)
                    );
                    let child = s.child.take().unwrap();
                    let _ = door_sys::close(child.control);
                    let _ = door_sys::close(child.process);
                    let port = core::mem::take(self.service_port(manifest));
                    if port != 0 {
                        let _ = door_sys::close(port);
                    }
                    s.ready = false;
                    s.restarts += 1;
                    s.retry_at = now + 100 * u64::from(s.restarts);
                }
            }
            return Ok(());
        }
        if s.restarts > 3 || now < s.retry_at {
            return Ok(());
        }
        match door_sys::process_launch(self.authority, manifest, 0, self.server.port, mode::DESKTOP)
        {
            Ok(child) => {
                s.child = Some(child);
                s.deadline = now + 500;
            }
            Err(Status::NotFound) => {
                // This image carries no such service.
                s.restarts = 4;
                line!("DOOR_P15 {} ABSENT", name);
            }
            Err(_) => {
                s.restarts += 1;
                s.retry_at = now + 100 * u64::from(s.restarts);
            }
        }
        Ok(())
    }
    /// Stop a system service (neither keeps state to save).
    fn service_retire(&mut self, s: &mut SystemService, manifest: u64) {
        if let Some(child) = s.child.take() {
            let _ = door_sys::process_terminate(child.process, -1);
            let _ = door_sys::close(child.control);
            let _ = door_sys::close(child.process);
        }
        let port = core::mem::take(self.service_port(manifest));
        if port != 0 {
            let _ = door_sys::close(port);
        }
        s.ready = false;
    }
    fn network_step(&mut self, d: &mut State, now: u64) -> Result<(), Status> {
        let test_image = d.test_image;
        self.service_step(
            &mut d.network,
            door_abi::platform::manifest::NETWORK,
            "NETWORK",
            test_image,
            now,
        )?;
        self.service_step(
            &mut d.audio,
            door_abi::platform::manifest::AUDIO,
            "AUDIO",
            test_image,
            now,
        )
    }
    fn network_retire(&mut self, d: &mut State) {
        self.service_retire(&mut d.network, door_abi::platform::manifest::NETWORK);
        self.service_retire(&mut d.audio, door_abi::platform::manifest::AUDIO);
    }
    fn desktop_step(&mut self, d: &mut State, now: u64) -> Result<(), Status> {
        if self.stopping || self.registry.power.is_some() {
            self.network_retire(d);
            // The search host and the IME have no windows or documents.
            // Retire both before the display service they answer, and wait
            // until both are reaped.
            if let Some(child) = d.ime.as_ref()
                && !d.ime_retiring
            {
                let _ = door_sys::process_terminate(child.process, IME_RETIRED);
                d.ime_retiring = true;
            }
            let mut waiting = false;
            // Accessories still open when the session ends are retired with
            // the IME (their windows hold no documents).
            if !d.accessories_retiring {
                for child in d.accessories.iter().flatten() {
                    let _ = door_sys::process_terminate(child.process, ACCESSORY_RETIRED);
                }
                if let Some(child) = d.admin_terminal.as_ref() {
                    let _ = door_sys::process_terminate(child.process, ACCESSORY_RETIRED);
                }
                d.accessories_retiring = true;
            }
            if let Some(child) = d.admin_terminal.as_ref() {
                match door_sys::process_wait(child.process, now) {
                    Err(Status::Timeout | Status::WouldBlock) => waiting = true,
                    _ => {
                        let child = d.admin_terminal.take().unwrap();
                        self.registry.admin_terminal = None;
                        let _ = door_sys::close(child.control);
                        let _ = door_sys::close(child.process);
                    }
                }
            }
            for slot in &mut d.accessories {
                if let Some(child) = slot.as_ref() {
                    match door_sys::process_wait(child.process, now) {
                        Err(Status::Timeout | Status::WouldBlock) => waiting = true,
                        result => {
                            line!(
                                "DOOR_P12 ACCESSORY EXIT pid={} code={}",
                                child.pid,
                                result.unwrap_or(ACCESSORY_RETIRED)
                            );
                            let child = slot.take().unwrap();
                            let _ = door_sys::close(child.control);
                            let _ = door_sys::close(child.process);
                        }
                    }
                }
            }
            if let Some(child) = d.ime.as_ref() {
                match door_sys::process_wait(child.process, now) {
                    Err(Status::Timeout | Status::WouldBlock) => waiting = true,
                    result => {
                        line!(
                            "DOOR_P12 IME EXIT pid={} code={}",
                            child.pid,
                            result.unwrap_or(IME_RETIRED)
                        );
                        let child = d.ime.take().unwrap();
                        let _ = door_sys::close(child.control);
                        let _ = door_sys::close(child.process);
                    }
                }
            }
            if let Some(child) = d.search.as_ref() {
                if !d.search_retiring {
                    let _ = door_sys::process_terminate(child.process, SEARCH_HOST_RETIRED);
                    d.search_retiring = true;
                }
                match door_sys::process_wait(child.process, now) {
                    Err(Status::Timeout | Status::WouldBlock) => return Ok(()),
                    result => {
                        line!(
                            "DOOR_P12 SEARCH_HOST EXIT pid={} code={}",
                            child.pid,
                            result.unwrap_or(SEARCH_HOST_RETIRED)
                        );
                        let child = d.search.take().unwrap();
                        self.registry.search_host = None;
                        self.registry.search_instance = 0;
                        let _ = door_sys::close(child.control);
                        let _ = door_sys::close(child.process);
                    }
                }
            }
            if waiting {
                return Ok(());
            }
            if let Some(child) = d.server.take() {
                let _ = door_sys::process_terminate(child.process, -1);
                let _ = door_sys::close(child.control);
                let _ = door_sys::close(child.process);
            }
            if self.registry.window != 0 {
                let _ = door_sys::close(self.registry.window);
                self.registry.window = 0;
            }
            if !self.stopping {
                self.finish_session()?;
            }
            return Ok(());
        }
        if !self.all_ready() {
            return Ok(());
        }
        self.network_step(d, now)?;
        if let Some(child) = d.server.as_ref() {
            match door_sys::process_wait(child.process, now) {
                Ok(code) => {
                    line!("DOOR_P11 SERVER_EXIT code={} restart={}", code, d.restarts);
                    let child = d.server.take().unwrap();
                    door_sys::close(child.control)?;
                    door_sys::close(child.process)?;
                    if self.registry.window != 0 {
                        door_sys::close(self.registry.window)?;
                        self.registry.window = 0;
                    }
                    self.registry.window_instance = 0;
                    self.registry.desktop_admission.restart();
                    d.ready = false;
                    d.bound = false;
                    d.restarts += 1;
                    d.retry_at = now + 100 * (d.restarts as u64);
                    if d.restarts > 3 {
                        for child in d.clients.iter_mut().filter_map(Option::take) {
                            let _ = door_sys::process_terminate(child.process, -1);
                            let _ = door_sys::close(child.process);
                            let _ = door_sys::close(child.control);
                        }
                        self.finish_session()?;
                        return Ok(());
                    }
                }
                Err(Status::Timeout | Status::WouldBlock) => (),
                Err(e) => return Err(e),
            }
        }
        if d.server.is_none() {
            if now < d.retry_at {
                return Ok(());
            }
            d.server = Some(door_sys::process_launch(
                self.authority,
                door_abi::platform::manifest::WINDOW,
                0,
                self.server.port,
                mode::DESKTOP,
            )?);
            d.deadline = now + 500;
            return Ok(());
        }
        if !d.ready {
            let child = d.server.as_ref().unwrap();
            let mut bytes = [0; door_abi::MAX_PACKET];
            let mut transfers = [Transfer::default(); door_abi::MAX_TRANSFERS];
            match door_sys::channel_receive(child.control, &mut bytes, &mut transfers) {
                Ok(meta) => {
                    let frame = Frame::decode(&bytes[..meta.packet_bytes as usize]);
                    let identity = door_sys::identity(child.control)?;
                    let valid = frame.is_ok_and(|f| {
                        f.header.opcode == opcode::READY
                            && f.header.request_id == 1
                            && f.code == Code::Success
                            && f.instance == identity.instance
                            && f.body == door_abi::platform::manifest::WINDOW.to_le_bytes()
                            && meta.handle_count == 1
                    });
                    if !valid {
                        for t in &transfers[..meta.handle_count as usize] {
                            let _ = door_sys::close(t.handle);
                        }
                        return Err(Status::AccessDenied);
                    }
                    server::transferred_capability(transfers[0].handle, HandleKind::Port)?;
                    self.registry.window = transfers[0].handle;
                    self.registry.window_instance = identity.instance;
                    d.ready = true;
                }
                Err(Status::WouldBlock) => {
                    if now >= d.deadline {
                        door_sys::process_terminate(child.process, -110)?;
                    }
                    return Ok(());
                }
                Err(Status::PeerClosed) => return Ok(()),
                Err(e) => return Err(e),
            }
        }
        if !d.started {
            if !d.signin_waiting {
                line!("DOOR_P12 SIGNIN AUTHORITY READY");
                d.signin_waiting = true;
            }
            if !self.registry.signin_requested {
                return Ok(());
            }
            self.registry.signin_requested = false;
            self.install_userland()?;
            let mut sessions = door_service_client::Session::connect_port(self.registry.ports[1])
                .map_err(|_| Status::IoFailure)?;
            // The account the sign-in surface accepted: the kernel
            // provisions one logon permit per local account.
            let permit = if self.registry.signin_second {
                inherited::LOGON_SECOND
            } else {
                inherited::LOGON_FIRST
            };
            self.session = sessions
                .logon(self.startup.base.inherited[permit].handle)
                .map_err(|_| Status::IoFailure)?;
            let identity = door_sys::session_info(self.session)?;
            d.session_identity = Some(identity);
            self.registry.session = self.session;
            self.registry.session_id = identity.0;
            d.started = true;
            line!("DOOR_P12 SESSION_READY uid={} autostart=0", identity.2);
            // The service account's logon session, once a boot (its permit
            // is Init's from the boot): Windows services run in it as
            // LocalSystem, whoever signs in or out. The Service Control
            // Manager, kept for the boot: Doors' own services, then every
            // service the registry configures.
            self.registry.session_uid = identity.2;
            if self.registry.service_session == 0 {
                // A failure leaves the user's session as it is: services
                // then answer FAILED_SERVICE_CONTROLLER_CONNECT.
                let opened = sessions
                    .logon(self.startup.base.inherited[inherited::LOGON_SERVICE].handle)
                    .map_err(|_| Status::IoFailure)
                    .and_then(|service| {
                        door_sys::session_info(service).map(|identity| (service, identity))
                    });
                match opened {
                    Ok((service, (id, _, uid))) => {
                        self.registry.service_session = service;
                        self.registry.service_session_id = id;
                        self.registry.service_profile();
                        line!("DOOR_P16 SERVICE SESSION uid={}", uid);
                    }
                    Err(e) => line!("DOOR_P16 SERVICE SESSION failed status={:?}", e),
                }
            }
            if !self.registry.scm_ready {
                self.registry.scm_ready = true;
                for (name, display) in [
                    ("DoorsStorage", "Doors Storage"),
                    ("DoorsSession", "Doors Session"),
                    ("DoorsNetwork", "Doors Network"),
                    ("DoorsAudio", "Doors Audio"),
                ] {
                    self.registry.scm.builtin(name, display, 0);
                }
                self.registry.scm_load();
            }
        }
        if !d.bound {
            if now < d.bind_retry_at {
                return Ok(());
            }
            match self.bind_session(d) {
                Ok(()) => d.bind_failures = 0,
                Err(Status::Timeout) => {
                    d.bind_failures += 1;
                    if d.bind_failures >= BIND_ATTEMPTS {
                        // The server never answered: restart it (its own
                        // restart budget still applies).
                        line!("DOOR_P11 BIND GAVE UP attempts={}", d.bind_failures);
                        d.bind_failures = 0;
                        if let Some(child) = d.server.as_ref() {
                            door_sys::process_terminate(child.process, -110)?;
                        }
                    } else {
                        d.bind_retry_at = now + BIND_RETRY_TICKS;
                    }
                    return Ok(());
                }
                Err(e) => return Err(e),
            }
        }
        // Persist display-service preferences for the session user: the shell
        // layout, and desktop controls changed from the desktop (applied back
        // through SET_DESKTOP_CONTROLS exactly like a Settings change).
        if d.bound
            && (self.registry.shell_layout_pending.is_some()
                || self.registry.controls_pending.is_some())
            && let Some((_, _, uid)) = d.session_identity
        {
            let mut storage = door_service_client::Storage::connect_port(self.registry.ports[0])
                .map_err(|_| Status::IoFailure)?;
            if let Some(layout) = self.registry.shell_layout_pending.take() {
                storage
                    .set_setting_for(
                        uid,
                        door_service_client::Setting::ShellLayout,
                        layout.flags(),
                    )
                    .and_then(|()| {
                        storage.set_setting_for(
                            uid,
                            door_service_client::Setting::ShellOrder,
                            layout.order(),
                        )
                    })
                    .map_err(|_| Status::IoFailure)?;
                line!(
                    "DOOR_P12 SHELL_LAYOUT saved flags={} order={}",
                    layout.flags(),
                    layout.order()
                );
            }
            if let Some(controls) = self.registry.controls_pending.take() {
                storage
                    .set_setting_for(
                        uid,
                        door_service_client::Setting::DesktopControls,
                        controls.bits(),
                    )
                    .map_err(|_| Status::IoFailure)?;
                door_service_client::window::WindowClient::apply_controls(
                    self.registry.window,
                    controls,
                )
                .map_err(|_| Status::IoFailure)?;
                line!("DOOR_P12 CONTROLS saved bits={}", controls.bits());
            }
        }
        // Settings the window server changed in place (window::PERSIST_SETTING).
        if d.bound
            && self.registry.settings_pending.iter().any(Option::is_some)
            && let Some((_, _, uid)) = d.session_identity
        {
            let mut storage = door_service_client::Storage::connect_port(self.registry.ports[0])
                .map_err(|_| Status::IoFailure)?;
            for slot in 0..self.registry.settings_pending.len() {
                let Some(value) = self.registry.settings_pending[slot].take() else {
                    continue;
                };
                let Some(setting) = door_service_client::Setting::from_id(persist_setting(slot))
                else {
                    continue;
                };
                let saved = storage.set_setting_for(uid, setting, value).is_ok();
                line!(
                    "DOOR_P14 SETTING saved id={} value={:#x} ok={}",
                    persist_setting(slot),
                    value,
                    u8::from(saved)
                );
            }
        }
        if !d.probe_started {
            let mut flag = [0; 16];
            match door_sys::boot_file_read(self.authority, "system/window-tests.conf", 0, &mut flag)
            {
                Ok(n) if &flag[..n] == b"window-tests-v1\n" => {
                    // Only the adversarial test image starts clients automatically.
                    for index in 0..2 {
                        self.launch_desktop_app(d, index)?;
                    }
                    line!(
                        "DOOR_P11 CLIENTS uid={} files={} notes={}",
                        d.session_identity.unwrap().2,
                        d.clients[0].as_ref().unwrap().pid,
                        d.clients[1].as_ref().unwrap().pid
                    );
                    let child = door_sys::process_launch(
                        self.authority,
                        manifest::WORKLOAD,
                        self.session,
                        self.server.port,
                        mode::DESKTOP,
                    )?;
                    self.registry.gui_clients[door_service_protocol::application::DESKTOP_APPS] =
                        Some((child.pid, child.process));
                    line!("DOOR_P11 ADVERSARY LAUNCHED pid={}", child.pid);
                    d.probe = Some(child);
                    d.probe_phase = 1;
                }
                Err(Status::NotFound) => (),
                _ => return Err(Status::InvalidArgument),
            }
            d.test_image = d.probe.is_some();
            d.probe_started = true;
        }
        self.search_host_step(d, now)?;
        self.ime_step(d, now)?;
        // Scheduled tasks that are due (and OnLogon ones once).
        if d.bound
            && let Some((_, _, uid)) = d.session_identity
            && now >= d.tasks_due
        {
            d.tasks_due = now + 100;
            self.registry.tasks_tick(uid);
        }
        if d.probe_phase == 2 && d.ready && d.bound && d.restarts == 1 {
            let child = door_sys::process_launch(
                self.authority,
                manifest::WORKLOAD,
                self.session,
                self.server.port,
                mode::WINDOW_TEST_HANG,
            )?;
            self.registry.gui_clients[door_service_protocol::application::DESKTOP_APPS] =
                Some((child.pid, child.process));
            d.probe = Some(child);
            d.probe_phase = 3;
        }
        if let Some(child) = d.probe.as_ref() {
            match door_sys::process_wait(child.process, now) {
                Ok(code) => {
                    let child = d.probe.take().unwrap();
                    self.registry.gui_clients[door_service_protocol::application::DESKTOP_APPS] =
                        None;
                    door_sys::close(child.control)?;
                    door_sys::close(child.process)?;
                    if d.probe_phase == 3 {
                        if code != -130 {
                            return Err(Status::IoFailure);
                        }
                        d.probe_phase = 4;
                        line!("DOOR_P11 HUNG_CLOSE PASS terminated=1");
                        return Ok(());
                    }
                    if code != 73 {
                        return Err(Status::IoFailure);
                    }
                    d.probe_phase = 2;
                    // Test-image-only real service loss. App processes/data survive;
                    // they must reconnect through a new authenticated incarnation.
                    if let Some(server) = d.server.as_ref() {
                        door_sys::process_terminate(server.process, -311)?;
                    }
                    line!("DOOR_P11 RECOVERY INJECT service-loss=1 clients-preserved=2");
                }
                Err(Status::Timeout | Status::WouldBlock) => (),
                Err(e) => return Err(e),
            }
        }
        for (index, slot) in d.clients.iter_mut().enumerate() {
            if let Some(child) = slot.as_ref() {
                match door_sys::process_wait(child.process, now) {
                    Ok(code) => {
                        line!("DOOR_P11 CLIENT_EXIT pid={} code={}", child.pid, code);
                        self.registry.gui_clients[index] = None;
                        self.registry.gui_instances[index] = 0;
                        let child = slot.take().unwrap();
                        door_sys::close(child.control)?;
                        door_sys::close(child.process)?;
                    }
                    Err(Status::Timeout | Status::WouldBlock) => (),
                    Err(e) => return Err(e),
                }
            }
        }
        if let Some(child) = d.admin_terminal.as_ref() {
            match door_sys::process_wait(child.process, now) {
                Ok(code) => {
                    line!(
                        "DOOR_P16 ELEVATED TERMINAL EXIT pid={} code={}",
                        child.pid,
                        code
                    );
                    self.registry.admin_terminal = None;
                    let child = d.admin_terminal.take().unwrap();
                    door_sys::close(child.control)?;
                    door_sys::close(child.process)?;
                }
                Err(Status::Timeout | Status::WouldBlock) => (),
                Err(e) => return Err(e),
            }
        }
        for slot in &mut d.accessories {
            if let Some(child) = slot.as_ref() {
                match door_sys::process_wait(child.process, now) {
                    Ok(code) => {
                        line!("DOOR_P12 ACCESSORY EXIT pid={} code={}", child.pid, code);
                        for a in &mut self.registry.accessories {
                            if a.is_some_and(|(pid, _)| pid == child.pid) {
                                *a = None;
                            }
                        }
                        let child = slot.take().unwrap();
                        door_sys::close(child.control)?;
                        door_sys::close(child.process)?;
                    }
                    Err(Status::Timeout | Status::WouldBlock) => (),
                    Err(e) => return Err(e),
                }
            }
        }
        if let Some(command) = self.registry.desktop_admission.take() {
            if command == door_service_protocol::application::SIGN_OUT_COMMAND {
                if let Err(error) = self.sign_out(d, now) {
                    line!("DOOR_P16 SIGNOUT failed error={:?}", error);
                }
            } else if matches!(command, 4 | 5) {
                self.registry.power = Some(command == 5);
            } else if command == door_service_protocol::application::ELEVATED_TERMINAL_COMMAND {
                if let Err(error) = self.launch_admin_terminal(d) {
                    line!("DOOR_P16 ELEVATED TERMINAL rejected error={:?}", error);
                    self.registry
                        .notify_system("Couldn't open the app", launch_reason(error));
                }
            } else if let Some(index) = door_service_protocol::application::accessory_index(command)
            {
                if let Err(error) = self.launch_accessory(d, index) {
                    line!("DOOR_P12 LAUNCH_REJECTED app={} error={:?}", command, error);
                    self.registry
                        .notify_system("Couldn't open the app", launch_reason(error));
                }
            } else if let Err(error) = self.launch_desktop_app(
                d,
                door_service_protocol::application::desktop_index(command)
                    .ok_or(Status::InvalidArgument)?,
            ) {
                // Admission failure is contained; it must not kill the desktop.
                line!("DOOR_P12 LAUNCH_REJECTED app={} error={:?}", command, error);
                self.registry
                    .notify_system("Couldn't open the app", launch_reason(error));
            }
        }
        Ok(())
    }
    /// Desktop shell lifecycle (P14, GUI-F): launched once the session is
    /// bound, restarted with exponential backoff after any exit (a crash loop
    /// stops after `SHELL_RESTARTS`; 30 s of uptime restores the budget).
    /// Apps are never restarted with it: the window server keeps them and
    /// shows its fallback taskbar until the new incarnation re-enumerates.
    /// The shell also hosts Start file search (the former search host).
    fn search_host_step(&mut self, d: &mut State, now: u64) -> Result<(), Status> {
        if let Some(child) = d.search.as_ref() {
            match door_sys::process_wait(child.process, now) {
                Ok(code) => {
                    line!("DOOR_P12 SEARCH_HOST EXIT pid={} code={}", child.pid, code);
                    let child = d.search.take().unwrap();
                    self.registry.search_host = None;
                    self.registry.search_instance = 0;
                    door_sys::close(child.control)?;
                    door_sys::close(child.process)?;
                    (d.search_restarts, d.search_retry_at) =
                        shell_after_exit(d.search_restarts, d.search_launched_at, now);
                    line!(
                        "DOOR_P14 SHELL EXIT pid={} code={} restarts={} retry_at={}",
                        child.pid,
                        code,
                        d.search_restarts,
                        d.search_retry_at
                    );
                    if d.search_restarts <= SHELL_RESTARTS {
                        self.registry.notify_system(
                            "Desktop restarted",
                            "The taskbar was rebuilt. Your apps kept running.",
                        );
                    }
                }
                Err(Status::Timeout | Status::WouldBlock) => (),
                Err(e) => return Err(e),
            }
            return Ok(());
        }
        if !d.bound
            || d.test_image
            || d.search_restarts > SHELL_RESTARTS
            || now < d.search_retry_at
            || self.registry.desktop_admission.frozen()
        {
            return Ok(());
        }
        match door_sys::process_launch(
            self.authority,
            door_abi::platform::manifest::DESKTOP_SHELL,
            self.session,
            self.server.port,
            mode::DESKTOP,
        ) {
            Ok(child) => {
                self.registry.search_host = Some((child.pid, child.process));
                self.registry.search_instance = door_sys::identity(child.control)?.instance;
                d.shell_incarnations += 1;
                d.search_launched_at = now;
                line!("DOOR_P12 SEARCH_HOST LAUNCHED pid={}", child.pid);
                line!(
                    "DOOR_P14 SHELL LAUNCHED pid={} incarnation={}",
                    child.pid,
                    d.shell_incarnations
                );
                d.search = Some(child);
            }
            Err(error) => {
                line!("DOOR_P12 SEARCH_HOST FAILED error={:?}", error);
                d.search_restarts += 1;
                d.search_retry_at = now + 500;
            }
        }
        Ok(())
    }
    /// Session IME lifecycle (Windows' text input host): launched once the
    /// session is bound, windowless, restarted with backoff (at most 3
    /// times in a row; 60 s of uptime restores the budget). Without it every input method types its keyboard layout
    /// only and Settings reports the IMEs as unavailable.
    fn ime_step(&mut self, d: &mut State, now: u64) -> Result<(), Status> {
        if let Some(child) = d.ime.as_ref() {
            match door_sys::process_wait(child.process, now) {
                Ok(code) => {
                    line!("DOOR_P12 IME EXIT pid={} code={}", child.pid, code);
                    let child = d.ime.take().unwrap();
                    door_sys::close(child.control)?;
                    door_sys::close(child.process)?;
                    (d.ime_restarts, d.ime_retry_at) =
                        ime_after_exit(d.ime_restarts, d.ime_launched_at, now);
                }
                Err(Status::Timeout | Status::WouldBlock) => (),
                Err(e) => return Err(e),
            }
            return Ok(());
        }
        if !d.bound
            || d.test_image
            || d.ime_restarts > IME_RESTARTS
            || now < d.ime_retry_at
            || self.registry.desktop_admission.frozen()
        {
            return Ok(());
        }
        match door_sys::process_launch(
            self.authority,
            door_abi::platform::manifest::IME,
            self.session,
            self.server.port,
            mode::DESKTOP,
        ) {
            Ok(child) => {
                line!("DOOR_P12 IME LAUNCHED pid={}", child.pid);
                d.ime = Some(child);
                d.ime_launched_at = now;
            }
            Err(error) => {
                line!("DOOR_P12 IME FAILED error={:?}", error);
                d.ime_restarts += 1;
                d.ime_retry_at = now + 500;
            }
        }
        Ok(())
    }
    /// Start-search accessory `index` (`ACCESSORY_COMMANDS` order) under its
    /// own manifest, so the window server can tell the Captions client from
    /// every other process by kernel identity alone.
    fn launch_accessory(&mut self, d: &mut State, index: usize) -> Result<(), Status> {
        if d.accessories[index].is_some() {
            return Ok(());
        }
        // Captions writes what it reads (app text) to serial only when the
        // desktop-diagnostics image stages its flag file.
        let mut flag = [0; 32];
        let diagnostics = ACCESSORY_MANIFESTS[index] == door_abi::platform::manifest::ASSIST
            && matches!(
                door_sys::boot_file_read(self.authority, a11y::ASSIST_DIAGNOSTICS_FILE, 0, &mut flag),
                Ok(n) if &flag[..n] == a11y::ASSIST_DIAGNOSTICS_FLAG
            );
        let child = door_sys::process_launch(
            self.authority,
            ACCESSORY_MANIFESTS[index],
            self.session,
            self.server.port,
            if diagnostics {
                a11y::ASSIST_DIAGNOSTIC_MODE
            } else {
                mode::DESKTOP
            },
        )?;
        self.registry.accessories[index] =
            Some((child.pid, door_sys::identity(child.control)?.instance));
        d.accessories_retiring = false;
        line!(
            "DOOR_P12 APP_LAUNCHED app={} pid={}",
            door_service_protocol::application::ACCESSORY_COMMANDS[index],
            child.pid
        );
        d.accessories[index] = Some(child);
        Ok(())
    }
    /// Terminal with the full token, in its own window (User Account
    /// Control approved it; the window server is the only requester).
    /// Sign out: the session service logs the session off (the kernel ends
    /// every process of it: apps, the shell, the IME, accessories, services
    /// the SCM started), Init forgets the session's children and state, and
    /// the window server restarts at the sign-in screen, as at boot. The
    /// network and audio services run outside the session and stay up.
    fn sign_out(&mut self, d: &mut State, now: u64) -> Result<(), Status> {
        let uid = self.registry.session_uid;
        if self.session != 0 {
            let mut sessions = door_service_client::Session::connect_port(self.registry.ports[1])
                .map_err(|_| Status::IoFailure)?;
            sessions
                .logoff(self.session)
                .map_err(|_| Status::IoFailure)?;
            door_sys::close(self.session)?;
            self.session = 0;
            // The account's boot permit admitted that session: a fresh one
            // lets it sign in again on this boot.
            let slot = if uid == 1002 {
                inherited::LOGON_SECOND
            } else {
                inherited::LOGON_FIRST
            };
            let permit = door_sys::permit_renew(self.authority, uid)?;
            self.startup.base.inherited[slot].handle = permit;
        }
        let ended = d
            .clients
            .iter_mut()
            .filter_map(Option::take)
            .chain(d.accessories.iter_mut().filter_map(Option::take))
            .chain(d.search.take())
            .chain(d.ime.take())
            .chain(d.admin_terminal.take());
        for child in ended {
            let _ = door_sys::close(child.control);
            let _ = door_sys::close(child.process);
        }
        d.search_restarts = 0;
        d.search_retry_at = 0;
        d.search_retiring = false;
        d.ime_restarts = 0;
        d.ime_retry_at = 0;
        d.ime_retiring = false;
        d.accessories_retiring = false;
        d.tasks_due = 0;
        let r = &mut self.registry;
        r.search_host = None;
        r.search_instance = 0;
        r.admin_terminal = None;
        r.gui_clients = [None; door_service_protocol::application::DESKTOP_APPS + 1];
        r.gui_instances = [0; door_service_protocol::application::DESKTOP_APPS];
        r.shell = 0;
        r.session = 0;
        r.session_id = 0;
        r.session_uid = 0;
        if r.session_directory != 0 {
            let _ = door_sys::close(r.session_directory);
            r.session_directory = 0;
        }
        // Services keep running (the service account's session stays);
        // the user's scheduled tasks go with the user.
        r.tasks = tasks::Scheduler::default();
        r.notifications.reset();
        r.signin_requested = false;
        r.signin_second = false;
        // The window server comes back at the sign-in screen.
        if let Some(child) = d.server.take() {
            let _ = door_sys::process_terminate(child.process, -1);
            let _ = door_sys::close(child.control);
            let _ = door_sys::close(child.process);
        }
        if self.registry.window != 0 {
            let _ = door_sys::close(self.registry.window);
            self.registry.window = 0;
        }
        self.registry.window_instance = 0;
        self.registry.desktop_admission.restart();
        d.ready = false;
        d.bound = false;
        d.started = false;
        d.signin_waiting = false;
        d.session_identity = None;
        d.retry_at = now;
        line!("DOOR_P16 SIGNOUT uid={}", uid);
        Ok(())
    }
    fn launch_admin_terminal(&mut self, d: &mut State) -> Result<(), Status> {
        if d.admin_terminal.is_some() {
            return Ok(());
        }
        let child = door_sys::process_launch(
            self.authority,
            door_abi::platform::manifest::WORKSPACE,
            self.session,
            self.server.port,
            mode::TERMINAL_ELEVATED,
        )?;
        let instance = door_sys::identity(child.control)?.instance;
        self.registry.admin_terminal = Some((child.pid, instance));
        d.accessories_retiring = false;
        line!("DOOR_P16 ELEVATED TERMINAL launched pid={}", child.pid);
        d.admin_terminal = Some(child);
        Ok(())
    }
    fn launch_desktop_app(&mut self, d: &mut State, index: usize) -> Result<(), Status> {
        if d.clients[index].is_some() {
            return Ok(());
        }
        let app_mode = [
            mode::DESKTOP,
            mode::SESSION,
            mode::SETTINGS,
            mode::TERMINAL,
            mode::DESKTOP,
            mode::DESKTOP,
        ][index];
        let app_manifest = match index {
            4 => door_abi::platform::manifest::PREVIEW,
            5 => door_abi::platform::manifest::MONITOR,
            _ => door_abi::platform::manifest::WORKSPACE,
        };
        let child = door_sys::process_launch(
            self.authority,
            app_manifest,
            self.session,
            self.server.port,
            app_mode,
        )?;
        self.registry.gui_clients[index] = Some((child.pid, child.process));
        self.registry.gui_instances[index] = door_sys::identity(child.control)?.instance;
        if index == 0 {
            self.registry.shell = child.pid;
        }
        line!(
            "DOOR_P12 APP_LAUNCHED app={} pid={}",
            door_service_protocol::application::DESKTOP_COMMANDS[index],
            child.pid
        );
        d.clients[index] = Some(child);
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_restarts_back_off_exponentially_and_recover_when_stable() {
        let (mut restarts, mut launched, mut now) = (0, 0, 10);
        let mut gaps = [0u64; 8];
        for i in 0..=SHELL_RESTARTS {
            let (r, retry) = shell_after_exit(restarts, launched, now);
            gaps[i as usize] = retry - now;
            (restarts, launched, now) = (r, retry, retry + 5);
        }
        assert_eq!(&gaps[..6], &[20, 40, 80, 160, 320, 320]);
        assert!(restarts > SHELL_RESTARTS, "a crash loop stops");
        let (r, retry) = shell_after_exit(SHELL_RESTARTS, 100, 100 + SHELL_STABLE_TICKS);
        assert_eq!((r, retry - (100 + SHELL_STABLE_TICKS)), (1, 20));
        assert!(launch_reason(Status::ResourceExhausted).contains("DOOR-LAUNCH-RESOURCES"));
    }
    #[test]
    fn ime_restart_budget_recovers_after_a_stable_period() {
        // A crash loop exhausts the budget with growing backoff.
        let (mut restarts, mut launched) = (0, 0);
        let mut now = 50;
        for expected in 1..=IME_RESTARTS + 1 {
            let (r, retry) = ime_after_exit(restarts, launched, now);
            assert_eq!((r, retry), (expected, now + 100 * u64::from(expected)));
            (restarts, launched, now) = (r, retry, retry + 50);
        }
        assert!(restarts > IME_RESTARTS, "no more relaunches");
        // An IME that ran a minute before failing starts a fresh budget.
        let (r, retry) = ime_after_exit(IME_RESTARTS, 1_000, 1_000 + IME_STABLE_TICKS);
        assert_eq!((r, retry), (1, 1_000 + IME_STABLE_TICKS + 100));
        assert_eq!(ime_after_exit(2, 1_000, 1_000 + IME_STABLE_TICKS - 1).0, 3);
    }
    #[test]
    fn window_server_persists_only_validated_appearance_and_preference_words() {
        use door_service_protocol::preferences as p;
        // The dark-mode schedule switching to dark on background 2.
        assert_eq!(persist_slot(2, 5), Some(0));
        assert_eq!(persist_setting(0), 2);
        assert_eq!(persist_slot(2, 35), Some(0), "dark Flow 4");
        assert_eq!(
            persist_slot(2, door_service_protocol::window::APPEARANCE_VALUES),
            None,
            "storage rejects it too"
        );
        // Win+V "Turn on" (clipboard history).
        let slot = persist_slot(p::CLIPBOARD, p::CLIPBOARD_HISTORY).unwrap();
        assert_eq!(persist_setting(slot), p::CLIPBOARD);
        assert_eq!(persist_slot(p::CLIPBOARD, 2), None);
        // Every preference word has its own slot; nothing else is accepted.
        for word in p::ALL {
            let slot = persist_slot(word.id, word.default).unwrap();
            assert!((1..=p::ALL.len()).contains(&slot));
            assert_eq!(persist_setting(slot), word.id);
        }
        for id in [0, 1, 3, 4, 5, 6, 7, 8, 9, p::LAST + 1] {
            assert_eq!(persist_slot(id, 0), None, "id {id}");
        }
    }
}
