//! Ordinary session orchestration and captured, user-authorized program admission.
use super::*;
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering},
};
use door_abi::platform::{
    ARGUMENT_BYTES, ENVIRONMENT_BYTES, EXECUTABLE_MAX_BYTES, ExecuteDescriptor,
};
use door_service_client::{Disposition, READ, SHARE_ALL, Storage, WRITE};
use door_service_protocol::application;

struct Images {
    busy: AtomicBool,
    bytes: UnsafeCell<[u8; EXECUTABLE_MAX_BYTES]>,
}
unsafe impl Sync for Images {}
static IMAGE: Images = Images {
    busy: AtomicBool::new(false),
    bytes: UnsafeCell::new([0; EXECUTABLE_MAX_BYTES]),
};

#[derive(Clone, Copy)]
pub(super) struct Job {
    owner: u64,
    child: LaunchResult,
    /// The admitted executable path (door.shell groups windows by it).
    path: [u8; door_service_protocol::notifications::PATH_BYTES],
    path_len: u8,
    /// The Terminal tab's console the program uses (0: the session's).
    console: u8,
}
impl Job {
    pub(super) fn pid(&self) -> u64 {
        self.child.pid
    }
    pub(super) fn path(&self) -> &str {
        core::str::from_utf8(&self.path[..usize::from(self.path_len)]).unwrap_or("")
    }
}
/// Launch arguments longer than the startup record: staged by a caller
/// (`pid` the caller), then held for the child (`pid` the child) until it
/// fetches them. `pid` 0 is a free slot.
#[derive(Clone, Copy)]
pub(super) struct Arguments {
    pid: u64,
    total: usize,
    received: usize,
    bytes: [u8; application::LONG_ARGUMENT_BYTES],
}
impl Arguments {
    pub(super) const EMPTY: Self = Self {
        pid: 0,
        total: 0,
        received: 0,
        bytes: [0; application::LONG_ARGUMENT_BYTES],
    };
    fn complete(&self) -> Option<&[u8]> {
        (self.pid != 0 && self.total != 0 && self.received == self.total)
            .then(|| &self.bytes[..self.total])
    }
}
/// What Init knows about a door.notifications caller (`notification_caller`).
struct NotificationCaller<'a> {
    source: u32,
    name: &'a str,
    sender: u32,
    shell: bool,
    workspace: bool,
}
struct Lease;
impl Drop for Lease {
    fn drop(&mut self) {
        IMAGE.busy.store(false, Ordering::Release);
    }
}
fn code(error: door_service_client::Error) -> Code {
    match error {
        door_service_client::Error::Service(code) => code,
        door_service_client::Error::Transport(status) => server::status(status),
        _ => Code::InvalidRequest,
    }
}
/// Offset of a PE image's OptionalHeader.Subsystem (the same in PE32 and
/// PE32+), when `head` holds it.
fn pe_subsystem_offset(head: &[u8]) -> Option<usize> {
    if head.get(..2)? != b"MZ" {
        return None;
    }
    let nt = u32::from_le_bytes(head.get(0x3c..0x40)?.try_into().ok()?) as usize;
    if head.get(nt..nt + 4)? != b"PE\0\0" {
        return None;
    }
    let at = nt + 24 + 68;
    head.get(at..at + 2)?;
    Some(at)
}
/// A Windows program's machine and subsystem (GUI 2 or console 3) from the
/// start of its file.
fn pe_identity(head: &[u8]) -> Option<(u16, u16)> {
    let at = pe_subsystem_offset(head)?;
    let nt = at - 24 - 68;
    let machine = u16::from_le_bytes(head.get(nt + 4..nt + 6)?.try_into().ok()?);
    let subsystem = u16::from_le_bytes(head.get(at..at + 2)?.try_into().ok()?);
    matches!(subsystem, 2 | 3).then_some((machine, subsystem))
}
fn initialized(result: Result<(), door_service_client::Error>) -> Result<(), Status> {
    match result {
        Ok(()) | Err(door_service_client::Error::Service(Code::Exists)) => Ok(()),
        _ => Err(Status::IoFailure),
    }
}
impl Registry {
    /// Process handle of a live EXECUTE-launched child (Windows GUI images
    /// included), for the window server's hung-window containment.
    pub(super) fn job_process(&self, pid: u64) -> Option<Handle> {
        self.jobs
            .iter()
            .flatten()
            .find(|job| job.child.pid == pid)
            .map(|job| job.child.process)
    }
    /// door.notifications identity of a session process, from what Init
    /// launched (kernel identities only): its source code, display name and
    /// sender hash, whether it is the shell host, and whether it is one of
    /// the session's workspace apps (Settings among them).
    fn notification_caller(&self, peer: Identity) -> NotificationCaller<'_> {
        use door_service_protocol::notifications::{sender, source};
        let mut who = NotificationCaller {
            source: source::SESSION,
            name: "",
            sender: 0,
            shell: false,
            workspace: false,
        };
        let shell = peer.manifest == door_abi::platform::manifest::DESKTOP_SHELL
            && self.search_host.is_some_and(|(pid, _)| pid == peer.pid)
            && self.search_instance == peer.instance;
        if shell {
            who.source = source::SHELL;
            who.shell = true;
            return who;
        }
        for (i, client) in self.gui_clients[..application::DESKTOP_APPS]
            .iter()
            .enumerate()
        {
            if client.is_some_and(|(pid, _)| pid == peer.pid)
                && self.gui_instances[i] == peer.instance
            {
                who.source = u32::from(application::DESKTOP_COMMANDS[i]);
                who.workspace = peer.manifest == door_abi::platform::manifest::WORKSPACE;
                return who;
            }
        }
        if self.admin_terminal == Some((peer.pid, peer.instance)) {
            who.source = u32::from(application::DESKTOP_COMMANDS[3]);
            who.workspace = peer.manifest == door_abi::platform::manifest::WORKSPACE;
            return who;
        }
        for (i, accessory) in self.accessories.iter().enumerate() {
            if *accessory == Some((peer.pid, peer.instance)) {
                who.source = u32::from(application::ACCESSORY_COMMANDS[i]);
                return who;
            }
        }
        for (slot, job) in self.jobs.iter().enumerate() {
            if let Some(job) = job
                && job.child.pid == peer.pid
            {
                let name = job.path().rsplit(['/', '\\']).next().unwrap_or("");
                who.source = source::JOB | slot as u32;
                who.name = door_service_protocol::shell::clip(
                    name,
                    door_service_protocol::notifications::SOURCE_BYTES,
                );
                who.sender = sender::path_hash(job.path());
                return who;
            }
        }
        who
    }
    /// door.notifications (P14): any process of the signed-in session posts,
    /// withdraws and takes activations; only the desktop shell lists and
    /// manages the centre; the shell and the session's workspace apps
    /// (Settings) toggle Do not disturb, run Focus and apply preference
    /// words. Words changed here (manual DND, the Focus length) are stored
    /// for the session user before the reply.
    pub(super) fn notification_request(
        &mut self,
        peer: Identity,
        operation: u32,
        body: &[u8],
        transfers: &[Transfer],
    ) -> Response {
        use door_service_protocol::notifications as n;
        if self.session == 0
            || peer.session != self.session_id
            || !server::session_peer(peer)
            || !transfers.is_empty()
        {
            return Response::error(Code::Denied);
        }
        let who = self.notification_caller(peer);
        let (source, shell) = (who.source, who.shell);
        let mut name_bytes = [0u8; n::SOURCE_BYTES];
        name_bytes[..who.name.len()].copy_from_slice(who.name.as_bytes());
        let name_len = who.name.len();
        let (sender, workspace) = (who.sender, who.workspace);
        if operation == n::APP_IDENTITY {
            if !shell || body.len() != 8 {
                return Response::error(if shell {
                    Code::InvalidRequest
                } else {
                    Code::Denied
                });
            }
            let pid = u64::from_le_bytes(body.try_into().unwrap());
            let job = self.jobs.iter().flatten().find(|j| j.child.pid == pid);
            let source = match job {
                Some(_) => {
                    n::source::JOB
                        | self
                            .jobs
                            .iter()
                            .position(|j| j.is_some_and(|j| j.child.pid == pid))
                            .unwrap_or(0) as u32
                }
                None => n::source::SESSION,
            };
            let identity = n::Identity {
                source,
                path: job.map_or("", |j| j.path()),
            };
            let mut out = [0; door_service_protocol::BODY_BYTES];
            return match identity.encode(&mut out) {
                Ok(len) => Response::bytes(&out[..len]),
                Err(code) => Response::error(code),
            };
        }
        let now = door_sys::time().unwrap_or(0);
        self.notification_schedule(now);
        let caller = crate::notifications::Caller {
            pid: peer.pid,
            instance: peer.instance,
            source,
            name: core::str::from_utf8(&name_bytes[..name_len]).unwrap_or(""),
            sender,
            shell,
            workspace,
        };
        let dnd = self.notifications.dnd();
        let focus = self.notifications.focus_running();
        let response = self.notifications.request(caller, operation, body, now);
        if response.code == Code::Success {
            match operation {
                n::POST => log(format_args!(
                    "DOOR_P14 NOTIFY POST source={source} pid={}",
                    peer.pid
                )),
                // A rule change takes effect at once.
                n::APPLY_PREFERENCE => self.notification_schedule(now),
                _ => (),
            }
        }
        if (dnd, focus) != (self.notifications.dnd(), self.notifications.focus_running()) {
            log(format_args!(
                "DOOR_P14 NOTIFY STATE dnd={} focus={}",
                u8::from(self.notifications.dnd()),
                u8::from(self.notifications.focus_running())
            ));
        }
        self.notification_store();
        response
    }
    /// Session binding (desktop step): the user's notification words and
    /// time zone. Unset or unreadable words are the Windows defaults.
    pub(super) fn bind_notifications(&mut self, storage: &mut Storage, uid: u32) {
        use door_service_client::Setting;
        use door_service_protocol::preferences as p;
        let mut word = |setting: Setting, id: u32| {
            storage
                .get_setting_for(uid, setting)
                .unwrap_or_else(|_| p::default(id))
        };
        let notifications = word(Setting::Notifications, p::NOTIFICATIONS);
        let schedule = word(Setting::DndSchedule, p::DND_SCHEDULE);
        let focus = word(Setting::Focus, p::FOCUS);
        let zone = storage
            .get_setting_for(uid, Setting::TimeZone)
            .unwrap_or(u32::from(door_locale::tz::DEFAULT_ID));
        let now = door_sys::time().unwrap_or(0);
        self.notifications.bind(uid, notifications, schedule, focus);
        self.notifications.set_zone(zone, now);
        self.notification_schedule(now);
        log(format_args!(
            "DOOR_P14 NOTIFY BIND words={notifications:#x},{schedule:#x},{focus:#x} dnd={}",
            u8::from(self.notifications.dnd())
        ));
    }
    /// "During these times": evaluated against the user's local time (the
    /// RTC's UTC in the stored time zone, re-read every minute).
    fn notification_schedule(&mut self, now: u64) {
        if !self.notifications.evaluation_due(now) {
            return;
        }
        if self.notifications.zone_due(now)
            && let Some(uid) = self.notifications.uid()
            && let Ok(mut storage) = Storage::connect_port(self.ports[0])
            && let Ok(zone) = storage.get_setting_for(uid, door_service_client::Setting::TimeZone)
        {
            self.notifications.set_zone(zone, now);
        }
        let zone = door_locale::tz::zone_or_utc(self.notifications.zone());
        let Some(local) = door_sys::wall_time()
            .ok()
            .and_then(|utc| zone.to_local(utc))
            .map(|l| l.time)
        else {
            return;
        };
        let Some(weekday) = local.weekday() else {
            return;
        };
        let before = self.notifications.dnd();
        self.notifications.evaluate(
            now,
            u32::from(local.hour) * 60 + u32::from(local.minute),
            u32::from(weekday),
        );
        if before != self.notifications.dnd() {
            log(format_args!(
                "DOOR_P14 NOTIFY SCHEDULE dnd={}",
                u8::from(self.notifications.dnd())
            ));
        }
    }
    /// Store the words this session changed (manual Do not disturb, the
    /// Focus length) for the session user; a failure retries next time.
    fn notification_store(&mut self) {
        use door_service_client::Setting;
        let pending = self.notifications.take_pending();
        if pending == crate::notifications::Pending::default() {
            return;
        }
        let Some(uid) = self.notifications.uid() else {
            return;
        };
        let stored = Storage::connect_port(self.ports[0]).and_then(|mut storage| {
            if let Some(word) = pending.notifications {
                storage.set_setting_for(uid, Setting::Notifications, word)?;
            }
            if let Some(word) = pending.focus {
                storage.set_setting_for(uid, Setting::Focus, word)?;
            }
            Ok(())
        });
        if stored.is_err() {
            self.notifications.restore_pending(pending);
            log(format_args!("DOOR_P14 NOTIFY STORE failed"));
        }
    }
    /// A notification from Init itself (shell recovery, launch failures).
    pub(super) fn notify_system(&mut self, title: &str, body: &str) {
        use door_service_protocol::notifications as n;
        if self.session == 0 {
            return;
        }
        let post = n::Post {
            icon: door_service_protocol::shell::icon::SYSTEM,
            flags: 0,
            title,
            body,
            actions: ["", ""],
        };
        let id = self.notifications.post(
            crate::notifications::Caller::SYSTEM,
            &post,
            door_sys::time().unwrap_or(0),
        );
        log(format_args!("DOOR_P14 NOTIFY SYSTEM id={id}"));
    }
    pub(super) fn reap_jobs(&mut self, now: u64) -> Result<(), Status> {
        self.scm_expire(now);
        for index in 0..self.jobs.len() {
            let Some(job) = self.jobs[index] else {
                continue;
            };
            match door_sys::process_wait(job.child.process, now) {
                Ok(code) => {
                    // A service process ended: its services stop; a task's
                    // run is over.
                    if let Some(crash) = self.scm.exited(job.child.pid, code as u32, now) {
                        self.service_failure_event(&crash);
                    }
                    self.task_exited(job.child.pid, code as u32);
                    self.jobs[index] = None;
                    door_sys::close(job.child.control)?;
                    door_sys::close(job.child.process)?;
                }
                Err(Status::WouldBlock | Status::Timeout) => (),
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
    pub(super) fn application_request(
        &mut self,
        peer: Identity,
        operation: u32,
        body: &[u8],
        transfers: &[Transfer],
    ) -> Response {
        if !self.admits(peer) {
            return Response::error(Code::Denied);
        }
        // The desktop shell's working-folder authority (once per sign-in):
        // scheduled tasks and services launch with it.
        if operation == application::SESSION_DIRECTORY {
            let desktop_shell = peer.manifest == door_abi::platform::manifest::DESKTOP_SHELL
                && self.search_host.is_some_and(|(pid, _)| pid == peer.pid)
                && self.search_instance == peer.instance;
            if !desktop_shell || !body.is_empty() || transfers.len() != 1 {
                return Response::error(Code::Denied);
            }
            // The server closes what a request carried: keep a copy.
            let kept = match door_sys::duplicate(
                transfers[0].handle,
                door_abi::rights::READ
                    | door_abi::rights::QUERY
                    | door_abi::rights::DUPLICATE
                    | door_abi::rights::TRANSFER,
            ) {
                Ok(h) => h,
                Err(e) => return Response::error(server::status(e)),
            };
            self.bind_directory(kept);
            self.scm_autostart();
            return Response::empty();
        }
        if operation == application::CANCEL_CHILDREN {
            // A console byte: the session's Terminal ends every program
            // using that tab's console (Ctrl+C no program handled).
            let console = match body {
                [] => None,
                [n] if *n != 0
                    && usize::from(*n) < door_service_protocol::console::CONSOLES
                    && peer.manifest == door_abi::platform::manifest::WORKSPACE =>
                {
                    Some(*n)
                }
                _ => return Response::error(Code::InvalidRequest),
            };
            if !transfers.is_empty() {
                return Response::error(Code::InvalidRequest);
            }
            let jobs = &self.jobs;
            // Started by `pid`, or by a program it started (and so on).
            let descends = |mut owner: u64| {
                for _ in 0..jobs.len() {
                    if owner == peer.pid {
                        return true;
                    }
                    match jobs.iter().flatten().find(|j| j.child.pid == owner) {
                        Some(parent) => owner = parent.owner,
                        None => return false,
                    }
                }
                false
            };
            for job in jobs.iter().flatten().filter(|job| match console {
                Some(n) => job.console == n && descends(job.owner),
                None => job.owner == peer.pid,
            }) {
                if let Err(error) = door_sys::process_terminate(job.child.process, -130) {
                    return Response::error(server::status(error));
                }
            }
            return Response::empty();
        }
        if operation == application::POWER {
            if peer.pid != self.shell || !transfers.is_empty() || body.len() != 1 || body[0] > 1 {
                return Response::error(Code::Denied);
            }
            if self.window != 0 {
                self.gui_power = Some(body[0] != 0);
            } else {
                self.power = Some(body[0] != 0);
            }
            return Response::empty();
        }
        if operation == application::ARGUMENTS_STAGE {
            return self.stage_arguments(peer, body, transfers);
        }
        if operation == application::ARGUMENTS_FETCH {
            return self.fetch_arguments(peer, body, transfers);
        }
        let detached = operation == application::EXECUTE_DETACHED;
        if !(operation == application::EXECUTE || detached)
            || transfers.is_empty()
            || self.power.is_some()
        {
            return Response::error(Code::InvalidRequest);
        }
        let request = match application::Execute::decode(body) {
            Ok(r) => r,
            Err(e) => return Response::error(e),
        };
        // STARTF_USESTDHANDLES: the pipe sections after the working folder,
        // one per standard handle the options name.
        let piped = request.options.standard;
        if transfers.len() != 1 + piped.count_ones() as usize {
            return Response::error(Code::InvalidRequest);
        }
        let mut next = 1;
        for (slot, handle) in self.standard.iter_mut().enumerate() {
            *handle = 0;
            if piped & 1 << slot != 0 {
                *handle = transfers[next].handle;
                next += 1;
            }
        }
        // A program an elevated process starts runs elevated too, as on
        // Windows (the child gets a copy of the parent's full token).
        let inherited = peer.groups & door_abi::platform::group::ADMINISTRATORS != 0;
        let response = self.execute(
            peer.pid,
            peer.uid,
            detached,
            &request,
            transfers[0].handle,
            None,
            inherited,
        );
        self.standard = [0; 3];
        response
    }
    /// ARGUMENTS_STAGE `[offset u16][whole u16][bytes]`: one caller stages
    /// at a time; offset 0 starts over.
    fn stage_arguments(&mut self, peer: Identity, body: &[u8], transfers: &[Transfer]) -> Response {
        if !transfers.is_empty() || body.len() <= 4 {
            return Response::error(Code::InvalidRequest);
        }
        let offset = usize::from(u16::from_le_bytes([body[0], body[1]]));
        let total = usize::from(u16::from_le_bytes([body[2], body[3]]));
        let bytes = &body[4..];
        let stage = &mut self.argument_stage;
        if stage.pid != 0 && stage.pid != peer.pid && offset == 0 && stage.received == stage.total {
            // A finished stage nobody launched with: replaced.
            stage.pid = 0;
        }
        if stage.pid != 0 && stage.pid != peer.pid {
            return Response::error(Code::WouldBlock);
        }
        if total == 0 || total > application::LONG_ARGUMENT_BYTES || offset + bytes.len() > total {
            return Response::error(Code::InvalidRequest);
        }
        if offset == 0 {
            stage.pid = peer.pid;
            stage.total = total;
            stage.received = 0;
        }
        if stage.total != total || stage.received != offset {
            return Response::error(Code::InvalidRequest);
        }
        stage.bytes[offset..offset + bytes.len()].copy_from_slice(bytes);
        stage.received += bytes.len();
        if stage.received == total
            && (stage.bytes[total - 1] != 0 || core::str::from_utf8(&stage.bytes[..total]).is_err())
        {
            *stage = Arguments::EMPTY;
            return Response::error(Code::InvalidRequest);
        }
        Response::empty()
    }
    /// ARGUMENTS_FETCH `[offset u16]` from a child started with staged
    /// arguments: `[whole u16][0; 2][bytes]`; the last part frees them.
    fn fetch_arguments(&mut self, peer: Identity, body: &[u8], transfers: &[Transfer]) -> Response {
        if !transfers.is_empty() || body.len() != 2 {
            return Response::error(Code::InvalidRequest);
        }
        let offset = usize::from(u16::from_le_bytes([body[0], body[1]]));
        let Some(slot) = self
            .long_arguments
            .iter_mut()
            .find(|a| a.pid == peer.pid && a.pid != 0)
        else {
            return Response::error(Code::NotFound);
        };
        if offset > slot.total {
            return Response::error(Code::InvalidRequest);
        }
        let mut response = Response::empty();
        let room = response.body.len() - 4;
        let n = (slot.total - offset).min(room);
        response.body[..2].copy_from_slice(&(slot.total as u16).to_le_bytes());
        response.body[4..4 + n].copy_from_slice(&slot.bytes[offset..offset + n]);
        response.len = 4 + n;
        if offset + n == slot.total {
            *slot = Arguments::EMPTY;
        }
        response
    }
    /// EXECUTE_ELEVATED (the caller is the authenticated current window
    /// server): User Account Control approved Run as administrator for the
    /// image whose SHA-256 leads the body. It runs as the session user with
    /// the full token (BUILTIN\Administrators enabled), without a console.
    pub(super) fn execute_elevated(&mut self, body: &[u8], transfers: &[Transfer]) -> Response {
        if self.session == 0 || transfers.len() != 1 || self.power.is_some() || body.len() < 32 {
            return Response::error(Code::InvalidRequest);
        }
        let mut image = [0; 32];
        image.copy_from_slice(&body[..32]);
        let request = match application::Execute::decode(&body[32..]) {
            Ok(r) => r,
            Err(e) => return Response::error(e),
        };
        let uid = match door_sys::session_info(self.session) {
            Ok((_, _, uid)) => uid,
            Err(e) => return Response::error(server::status(e)),
        };
        self.execute(
            0,
            uid,
            true,
            &request,
            transfers[0].handle,
            Some(image),
            false,
        )
    }
    /// Capture `request.path` as `uid` and start it in the session; with
    /// `image`, only if the captured bytes hash to it.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn execute(
        &mut self,
        owner: u64,
        uid: u32,
        detached: bool,
        request: &application::Execute<'_>,
        cwd: Handle,
        image: Option<[u8; 32]>,
        inherited: bool,
    ) -> Response {
        let Some(job_slot) = self.jobs.iter().position(Option::is_none) else {
            return Response::error(Code::Resources);
        };
        // The child's console: the Terminal tab's the request names, else
        // its parent's (a program the Terminal started, or theirs).
        let console = if detached {
            0
        } else if request.options.console != 0 {
            request.options.console
        } else {
            self.jobs
                .iter()
                .flatten()
                .find(|job| job.child.pid == owner && owner != 0)
                .map_or(0, |job| job.console)
        };
        // The service account's programs run in its logon session.
        let service = uid == door_abi::platform::SERVICE_ACCOUNT;
        if service && self.service_session == 0 {
            return Response::error(Code::WouldBlock);
        }
        let mut stage = "folder";
        let result = (|| -> Result<LaunchResult, Code> {
            let mut storage = Storage::connect_port(self.ports[0]).map_err(code)?;
            if !storage
                .metadata_for(uid, request.cwd)
                .map_err(code)?
                .directory
            {
                return Err(Code::InvalidRequest);
            }
            stage = "image";
            let file = storage.open_for(uid, request.path).map_err(code)?;
            let info = storage.info(&file).map_err(code)?;
            if info.directory || info.bytes == 0 {
                let _ = storage.close(file);
                return Err(Code::Resources);
            }
            if IMAGE
                .busy
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                let _ = storage.close(file);
                return Err(Code::Resources);
            }
            let _lease = Lease;
            let bytes = unsafe { &mut *IMAGE.bytes.get() };
            // A Windows program larger than the capture (Electron, VS Code,
            // CEF apps): the image host starts in its place, and ntdll's
            // loader lays the program out from its file.
            let large = info.bytes > EXECUTABLE_MAX_BYTES as u64;
            let wanted = if large { 4096 } else { info.bytes as usize };
            let captured = (|| {
                let mut offset = 0;
                while offset < wanted {
                    let end = (offset + 192).min(wanted);
                    let count = storage
                        .read(&file, offset as u64, &mut bytes[offset..end])
                        .map_err(code)?;
                    if count == 0 {
                        return Err(Code::Io);
                    }
                    offset += count;
                }
                Ok(())
            })();
            let closed = storage.close(file).map_err(code);
            captured.and(closed)?;
            let image_bytes = if large {
                // Elevation verifies the program byte for byte: not yet for
                // programs this large.
                if image.is_some() {
                    return Err(Code::Unsupported);
                }
                let (machine, subsystem) =
                    pe_identity(&bytes[..wanted]).ok_or(Code::Unsupported)?;
                let host = match machine {
                    0x8664 => application::IMAGE_HOST,
                    0x14c => application::IMAGE_HOST32,
                    _ => return Err(Code::Unsupported),
                };
                let mut n = 0;
                loop {
                    let count = door_sys::boot_file_read(
                        self.launch,
                        host,
                        n as u64,
                        &mut bytes[n..(n + 4096).min(EXECUTABLE_MAX_BYTES)],
                    )
                    .map_err(|_| Code::Io)?;
                    if count == 0 {
                        break;
                    }
                    n += count;
                    if n == EXECUTABLE_MAX_BYTES {
                        return Err(Code::Resources);
                    }
                }
                // The host runs as the program's subsystem (a console
                // program joins the console, a GUI one does not).
                let at = pe_subsystem_offset(&bytes[..n]).ok_or(Code::Io)?;
                bytes[at..at + 2].copy_from_slice(&subsystem.to_le_bytes());
                n
            } else {
                info.bytes as usize
            };
            let captured_bytes = image_bytes as u64;
            if let Some(image) = image {
                use sha2::Digest;
                // The approved program, byte for byte, or nothing.
                if sha2::Sha256::digest(&bytes[..captured_bytes as usize])[..] != image[..] {
                    return Err(Code::Denied);
                }
                let _ = door_sys::write(b"DOOR_P16 ELEVATED LAUNCH image verified\n");
            }

            stage = "format";
            let format = if bytes[..4] == *b"\x7fELF" {
                1
            } else if bytes[..2] == *b"MZ" {
                2
            } else {
                return Err(Code::Unsupported);
            };
            // Staged arguments: the startup record carries the program
            // name alone; the child fetches the rest (ARGUMENTS_FETCH).
            stage = "arguments";
            let staged = if request.staged {
                if self.argument_stage.pid != owner || owner == 0 {
                    return Err(Code::InvalidRequest);
                }
                Some(self.argument_stage.complete().ok_or(Code::InvalidRequest)?)
            } else {
                None
            };
            // An environment block after the arguments (lpEnvironment).
            let split = usize::from(request.options.environment);
            if split != 0 {
                let all = staged.ok_or(Code::InvalidRequest)?;
                if split > all.len() || all[split - 1] != 0 {
                    return Err(Code::InvalidRequest);
                }
            }
            let short = match staged {
                Some(all) => {
                    let end = all
                        .iter()
                        .position(|&b| b == 0)
                        .ok_or(Code::InvalidRequest)?
                        + 1;
                    if end > ARGUMENT_BYTES {
                        return Err(Code::InvalidRequest);
                    }
                    &all[..end]
                }
                None => request.arguments,
            };
            let mut description = ExecuteDescriptor {
                image_pointer: 1,
                image_bytes: captured_bytes,
                format,
                cwd,
                argument_bytes: short.len() as u16,
                environment_bytes: 0,
                // User Account Control approved this image, or an elevated
                // parent asked: the full token. A service (the service
                // account's program) always has it, as LocalSystem's.
                elevated: image.is_some() || inherited || service,
                suspended: request.options.suspended,
                standard: self.standard,
                arguments: [0; ARGUMENT_BYTES],
                environment: [0; ENVIRONMENT_BYTES],
            };
            description.arguments[..short.len()].copy_from_slice(short);
            stage = "environment";
            let mut environment = Log {
                bytes: [0; 256],
                len: 0,
            };
            let _ = write!(
                environment,
                "HOME=/users/{}\0USER={}\0PATH=/system/bin\0PWD={}\0",
                uid, uid, request.cwd
            );
            if detached && write!(environment, "{}\0", application::DETACHED_ENVIRONMENT).is_err() {
                return Err(Code::InvalidRequest);
            }
            // The console the program uses: the Terminal tab's it names, or
            // its parent's (Windows children inherit their parent's
            // console); none for a launch without a console.
            if console != 0
                && write!(
                    environment,
                    "{}{}\0",
                    door_service_protocol::console::ENVIRONMENT,
                    console
                )
                .is_err()
            {
                return Err(Code::InvalidRequest);
            }
            if large && write!(environment, "{}\0", application::IMAGE_ENVIRONMENT).is_err() {
                return Err(Code::InvalidRequest);
            }
            if staged.is_some()
                && write!(environment, "{}\0", application::LONG_ARGUMENTS_ENVIRONMENT).is_err()
            {
                return Err(Code::InvalidRequest);
            }
            if split != 0
                && write!(
                    environment,
                    "{}{}\0",
                    application::STAGED_ENVIRONMENT_PREFIX,
                    split
                )
                .is_err()
            {
                return Err(Code::InvalidRequest);
            }
            if environment.len > ENVIRONMENT_BYTES {
                return Err(Code::InvalidRequest);
            }
            description.environment[..environment.len]
                .copy_from_slice(&environment.bytes[..environment.len]);
            description.environment_bytes = environment.len as u16;
            stage = "kernel";
            door_sys::process_execute(
                self.launch,
                if service {
                    self.service_session
                } else {
                    self.session
                },
                self.port,
                &bytes[..captured_bytes as usize],
                &description,
            )
            .map_err(server::status)
        })();
        if request.staged && self.argument_stage.pid == owner && owner != 0 {
            // The staged block now belongs to the child (or, on failure, to
            // nobody); a full table drops the oldest unclaimed one.
            let mut held = self.argument_stage;
            self.argument_stage = Arguments::EMPTY;
            if let Ok(child) = &result {
                held.pid = child.pid;
                let slot = self
                    .long_arguments
                    .iter()
                    .position(|a| a.pid == 0)
                    .unwrap_or(0);
                if slot == 0 && self.long_arguments[0].pid != 0 {
                    self.long_arguments.rotate_left(1);
                    let last = self.long_arguments.len() - 1;
                    self.long_arguments[last] = held;
                } else {
                    self.long_arguments[slot] = held;
                }
            }
        }
        match result {
            Ok(child) => {
                self.last_launched = Some(child.pid);
                let mut path = [0; door_service_protocol::notifications::PATH_BYTES];
                let kept = door_service_protocol::shell::clip(request.path, path.len());
                path[..kept.len()].copy_from_slice(kept.as_bytes());
                self.jobs[job_slot] = Some(Job {
                    owner,
                    child,
                    path,
                    path_len: kept.len() as u8,
                    console,
                });
                // The firewall's program rules name it by its path.
                super::firewall::program(self.network, child.pid, request.path);
                // DUPLICATE: CreateProcess hands the launcher hProcess and an
                // hThread (a second handle to the same process, which
                // ResumeThread starts when the child was CREATE_SUSPENDED).
                Response::handle(
                    child.process,
                    door_abi::rights::QUERY
                        | door_abi::rights::WAIT
                        | door_abi::rights::TRANSFER
                        | door_abi::rights::DUPLICATE,
                    false,
                )
            }
            Err(error) => {
                super::log(format_args!(
                    "DOOR_P16 EXECUTE failed code={error:?} stage={stage} elevated={} path={}",
                    image.is_some(),
                    door_service_protocol::shell::clip(request.path, 96)
                ));
                Response::error(error)
            }
        }
    }
}
impl Init {
    pub(super) fn finish_session(&mut self) -> Result<(), Status> {
        self.registry.session = 0;
        self.registry.notifications.reset();
        if self.session != 0 {
            let mut sessions = door_service_client::Session::connect_port(self.registry.ports[1])
                .map_err(|_| Status::IoFailure)?;
            sessions
                .logoff(self.session)
                .map_err(|_| Status::IoFailure)?;
            door_sys::close(self.session)?;
            self.session = 0;
        }
        self.stopping = true;
        self.policy.begin_shutdown();
        Ok(())
    }
    pub(super) fn install_userland(&self) -> Result<(), Status> {
        let mut storage =
            Storage::connect_port(self.registry.ports[0]).map_err(|_| Status::IoFailure)?;
        for path in ["/system", "/system/bin", "/system/share"] {
            initialized(storage.mkdir(path))?;
        }
        let mut manifest = [0; 4096];
        let count =
            door_sys::boot_file_read(self.authority, "system/install.manifest", 0, &mut manifest)?;
        let text = core::str::from_utf8(&manifest[..count]).map_err(|_| Status::InvalidArgument)?;
        for line in text
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
        {
            let (path, source) = line.split_once(' ').ok_or(Status::InvalidArgument)?;
            // System seeds, and Windows' own programs (msiexec.exe) in
            // C:\Windows\System32 and, for 32-bit programs, SysWOW64.
            let system32 = ["/compat/Windows/System32/", "/compat/Windows/SysWOW64/"]
                .iter()
                .filter_map(|folder| path.strip_prefix(folder))
                .any(|name| !name.contains('/') && name.ends_with(".exe"));
            // Machine-wide data the image ships for every user, directly in
            // C:\ProgramData (its ACL lets everyone read, administrators
            // change).
            let program_data = path
                .strip_prefix("/compat/ProgramData/")
                .is_some_and(|name| !name.is_empty() && !name.contains('/'));
            if !(path.starts_with("/system/bin/")
                || path.starts_with("/system/share/")
                || system32
                || program_data)
            {
                return Err(Status::AccessDenied);
            }
            let file = storage
                .open(path, Disposition::OpenAlways, READ | WRITE, SHARE_ALL)
                .map_err(|_| Status::IoFailure)?;
            let install = (|| -> Result<(), Status> {
                let mut offset = 0;
                let mut fresh = [0; 4096];
                let mut existing = [0; 4096];
                loop {
                    let count =
                        door_sys::boot_file_read(self.authority, source, offset, &mut fresh)?;
                    if count == 0 {
                        break;
                    }
                    let mut present = 0;
                    while present < count {
                        let end = (present + 192).min(count);
                        let n = storage
                            .read(&file, offset + present as u64, &mut existing[present..end])
                            .map_err(|_| Status::IoFailure)?;
                        present += n;
                        if n == 0 {
                            break;
                        }
                    }
                    if present != count || existing[..count] != fresh[..count] {
                        storage
                            .write_all(&file, offset, &fresh[..count])
                            .map_err(|_| Status::IoFailure)?;
                    }
                    offset += count as u64;
                }
                if storage.info(&file).map_err(|_| Status::IoFailure)?.bytes != offset {
                    storage
                        .truncate(&file, offset)
                        .map_err(|_| Status::IoFailure)?;
                }
                storage.flush().map_err(|_| Status::IoFailure)
            })();
            let closed = storage.close(file).map_err(|_| Status::IoFailure);
            install.and(closed)?;
        }
        // A failed registration leaves COM activation without these
        // classes; the session still starts.
        match classes::register(&mut storage) {
            Ok(()) => super::log(format_args!("DOOR_P16 CLASSES registered")),
            Err(e) => super::log(format_args!("DOOR_P16 CLASSES failed {e:?}")),
        }
        Ok(())
    }
    pub(super) fn ordinary_applications(&mut self, now: u64) -> Result<(), Status> {
        if self.stopping || !self.all_ready() {
            return Ok(());
        }
        if let Some(app) = &self.app {
            if self.registry.power.is_some() {
                return self.finish_session();
            }
            match door_sys::process_wait(app.process, now) {
                Ok(exit) => {
                    log(format_args!("DOOR_SESSION SHELL_EXIT code={exit}"));
                    let app = self.app.take().unwrap();
                    let _ = door_sys::close(app.control);
                    let _ = door_sys::close(app.process);
                    self.finish_session()?;
                }
                Err(Status::Timeout | Status::WouldBlock) => (),
                Err(error) => return Err(error),
            }
            return Ok(());
        }
        self.install_userland()?;
        let mut sessions = door_service_client::Session::connect_port(self.registry.ports[1])
            .map_err(|_| Status::IoFailure)?;
        self.session = sessions
            .logon(self.startup.base.inherited[inherited::LOGON_FIRST].handle)
            .map_err(|_| Status::IoFailure)?;
        let child = door_sys::process_launch(
            self.authority,
            door_abi::platform::manifest::SHELL,
            self.session,
            self.server.port,
            mode::SESSION,
        )?;
        let identity = door_sys::identity(child.control)?;
        self.registry.session = self.session;
        self.registry.session_id = identity.session;
        self.registry.shell = child.pid;
        self.app = Some(child);
        log(format_args!(
            "DOOR_SESSION READY uid={} shell={}",
            identity.uid, identity.pid
        ));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The first bytes of a PE image: DOS header, PE signature, file header
    /// (machine), and the optional header's subsystem.
    fn head(machine: u16, subsystem: u16) -> [u8; 512] {
        let mut b = [0u8; 512];
        b[..2].copy_from_slice(b"MZ");
        b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        b[0x80 + 24 + 68..0x80 + 24 + 70].copy_from_slice(&subsystem.to_le_bytes());
        b
    }
    #[test]
    fn large_programs_are_identified_from_their_first_bytes() {
        assert_eq!(pe_identity(&head(0x8664, 2)), Some((0x8664, 2)));
        assert_eq!(pe_identity(&head(0x14c, 3)), Some((0x14c, 3)));
        assert_eq!(pe_subsystem_offset(&head(0x8664, 2)), Some(0x80 + 24 + 68));
        // Drivers, native images and truncated or foreign files are not
        // programs the host starts.
        assert_eq!(pe_identity(&head(0x8664, 1)), None);
        assert_eq!(pe_identity(&head(0x8664, 2)[..0x90]), None);
        assert_eq!(pe_identity(b"\x7fELF"), None);
        let mut bad = head(0x8664, 2);
        bad[0x80] = b'X';
        assert_eq!(pe_identity(&bad), None);
    }
}
