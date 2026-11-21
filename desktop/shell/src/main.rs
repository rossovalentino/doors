//! `door-desktop-shell`: Init's restartable desktop shell process (manifest
//! `DESKTOP_SHELL`, one per signed-in session, in the Start search host
//! slot). See the library documentation for the architecture.
#![no_std]
#![no_main]
extern crate alloc;
use alloc::{format, string::String, vec::Vec};
use door_abi::Status;
use door_abi::platform::{StartupV2, inherited};
use door_desktop_shell::{
    Effect, Shell, model,
    notices::Notice,
    registry::{self, Database, Target},
    senders,
};
use door_service_client::{
    Disposition, Error, Notifications, READ, SHARE_ALL, Storage, WRITE, window::WindowClient,
};
use door_service_protocol::{BODY_BYTES, Code, notifications as n, shell as s, window as w};
use door_workspace::shell_search::{self, Provider};

#[global_allocator]
static HEAP: door_rt::HeapAllocator = door_rt::HeapAllocator::new();
door_rt::entry_v2!(main);

/// Notification service state is read this often (10 ms ticks).
const NOTICE_POLL_TICKS: u64 = 20;
/// Sender settings (Settings > System > Notifications) are re-read this often.
const SENDER_POLL_TICKS: u64 = 200;
/// Shell events handled per iteration before yielding to search/painting.
const EVENTS_PER_PASS: usize = 32;

fn main(startup: StartupV2) -> i64 {
    if let Err(error) = HEAP.initialize_with_bytes(40 * 1024 * 1024) {
        door_rt::diagnostic(format_args!("DOOR_P14 SHELL HEAP FAILED {error:?}\n"));
        return -240;
    }
    match run(&startup) {
        Ok(()) => 0,
        Err(e) => {
            door_rt::diagnostic(format_args!("DOOR_P14 SHELL STOPPED {e:?}\n"));
            -241
        }
    }
}
fn transient(error: &Error) -> bool {
    matches!(
        error,
        Error::Transport(
            Status::PeerClosed | Status::Timeout | Status::NotFound | Status::WouldBlock
        ) | Error::Service(Code::WouldBlock | Code::Closing)
    )
}
struct Context {
    registry: u64,
    directory: u64,
    home: String,
    storage: Storage,
    notices: Option<Notifications>,
    /// pid -> Init's admitted executable path (None: not an EXECUTE job).
    identities: Vec<(u64, Option<String>)>,
    notice_generation: u32,
    notice_due: u64,
    senders_due: u64,
    /// Last (groups, windows) reported on serial (bounded diagnostics).
    taskbar_logged: (usize, usize),
    /// Taskbar groups last accepted by this window-server connection.
    published: Option<Vec<model::Group>>,
}
/// Test images only: `DIAGNOSTICS_FILE` (installed by the image's install
/// manifest) arms a fault hook. A user-created `FAULT_FILE` in the profile
/// then makes this incarnation crash, so RECOVER-01 can prove that a shell
/// failure keeps every app alive. Production images lack the file.
const DIAGNOSTICS_FILE: &str = "/system/share/shell-diagnostics.conf";
const DIAGNOSTICS_FLAG: &[u8] = b"shell-diagnostics-v1
";
const FAULT_FILE: &str = "shell-fault";
fn run(startup: &StartupV2) -> Result<(), Error> {
    let registry = startup.base.inherited[inherited::REGISTRY].handle;
    let storage = loop {
        match Storage::connect(registry) {
            Ok(s) => break s,
            Err(error) if transient(&error) => door_sys::sleep(door_sys::time()? + 10)?,
            Err(error) => return Err(error),
        }
    };
    let mut cx = Context {
        registry,
        directory: startup.base.inherited[inherited::DIRECTORY].handle,
        home: format!("/users/{}", startup.identity.uid),
        storage,
        notices: None,
        identities: Vec::new(),
        notice_generation: 0,
        notice_due: 0,
        senders_due: 0,
        taskbar_logged: (usize::MAX, 0),
        published: None,
    };
    // Init runs scheduled tasks and services in this session with this
    // user's working-folder authority.
    if let Err(e) = door_service_client::Applications::connect(registry)
        .and_then(|mut apps| apps.bind_directory(cx.directory))
    {
        door_rt::diagnostic(format_args!(
            "DOOR_P16 SHELL SESSION_DIRECTORY failed {e:?}\n"
        ));
    }
    let diagnostics = matches!(
        read_file(&mut cx.storage, DIAGNOSTICS_FILE),
        Ok(Some(bytes)) if bytes == DIAGNOSTICS_FLAG
    );
    if diagnostics {
        door_rt::diagnostic(format_args!(
            "DOOR_P14 SHELL DIAGNOSTICS armed=1
"
        ));
    }
    let mut shell = Shell::new(load_database(&mut cx));
    restore_vpns(&mut cx, startup.identity.uid);
    let mut provider = Provider::new()
        .with_home(cx.home.clone())
        .delay_until(door_sys::time()?.saturating_add(shell_search::START_DELAY_TICKS));
    loop {
        let outcome = session(&mut cx, &mut shell, &mut provider, diagnostics);
        match outcome {
            Ok(()) => return Ok(()),
            Err(error) if transient(&error) => {
                // The window server restarted (or is not ready yet): every
                // publication is rebuilt on the next connection.
                shell.taskbar_dirty = true;
                shell.catalog_dirty = true;
                shell.notices_dirty = true;
                door_sys::sleep(door_sys::time()? + 10)?
            }
            Err(error) => return Err(error),
        }
    }
}
/// Built-in registrations, installed Windows executables and the user's pins.
fn load_database(cx: &mut Context) -> Database {
    let mut db = Database::builtin();
    let mut names: Vec<String> = Vec::new();
    let mut cookie = 0;
    while names.len() < registry::MAX_EXECUTABLES * 4 {
        match cx.storage.list("/system/bin", cookie) {
            Ok(Some(entry)) => {
                cookie = entry.cookie;
                if !entry.info.directory {
                    names.push(String::from_utf16_lossy(&entry.name[..entry.name_len]));
                }
            }
            _ => break,
        }
    }
    let added = db.discover("/system/bin", names.iter().map(String::as_str));
    let path = format!("{}/{}", cx.home, registry::PROFILE_FILE);
    let profile = read_file(&mut cx.storage, &path);
    let pins = match profile {
        Ok(Some(bytes)) => db.apply_profile(&bytes).map_or("malformed", |()| "loaded"),
        Ok(None) => "none",
        Err(_) => "unreadable",
    };
    door_rt::diagnostic(format_args!(
        "DOOR_P14 SHELL REGISTRATIONS total={} executables={added} pins={pins} taskbar_pins={}\n",
        db.entries.len(),
        db.taskbar_pins.len()
    ));
    db
}
/// The VPN connections Settings saved (the user's phonebook) back in
/// door.network after a restart: the ones it does not hold already.
fn restore_vpns(cx: &mut Context, uid: u32) {
    use door_workspace::phonebook;
    let saved = match phonebook::read(&mut cx.storage, &cx.home) {
        Ok(text) => phonebook::entries(&text),
        Err(_) => return,
    };
    if saved.is_empty() {
        return;
    }
    // door.network starts with the desktop: wait for it a little.
    let mut network = None;
    for _ in 0..50 {
        match door_service_client::Network::connect(cx.registry) {
            Ok(n) => {
                network = Some(n);
                break;
            }
            Err(_) => {
                let _ = door_sys::time().and_then(|t| door_sys::sleep(t + 10));
            }
        }
    }
    let Some(mut network) = network else {
        door_rt::diagnostic(format_args!("DOOR_P15 SHELL VPN RESTORE no network\n"));
        return;
    };
    let mut held = Vec::new();
    for index in 0..=u8::MAX {
        match network.vpn_entry(uid, index) {
            Ok(Some(e)) => held.push(e.name().to_ascii_lowercase()),
            _ => break,
        }
    }
    let mut restored = 0;
    for (name, text) in &saved {
        if !held.contains(&name.to_ascii_lowercase()) && network.vpn_save(uid, name, text).is_ok() {
            restored += 1;
        }
    }
    door_rt::diagnostic(format_args!(
        "DOOR_P15 SHELL VPN RESTORED count={restored} saved={}\n",
        saved.len()
    ));
}
fn read_file(storage: &mut Storage, path: &str) -> Result<Option<Vec<u8>>, Error> {
    let file = match storage.open(path, Disposition::OpenExisting, READ, SHARE_ALL) {
        Ok(f) => f,
        Err(Error::Service(Code::NotFound)) => return Ok(None),
        Err(e) => return Err(e),
    };
    let result = (|| {
        let info = storage.info(&file)?;
        if info.bytes > registry::MAX_BYTES as u64 {
            return Err(Error::Malformed);
        }
        let mut bytes = Vec::new();
        let mut chunk = [0; door_service_protocol::CHUNK_BYTES];
        while (bytes.len() as u64) < info.bytes {
            let count = (info.bytes as usize - bytes.len()).min(chunk.len());
            let got = storage.read(&file, bytes.len() as u64, &mut chunk[..count])?;
            if got == 0 {
                return Err(Error::Malformed);
            }
            bytes.extend_from_slice(&chunk[..got]);
        }
        Ok(bytes)
    })();
    let closed = storage.close(file);
    let bytes = result?;
    closed?;
    Ok(Some(bytes))
}
fn persist(cx: &mut Context, db: &Database) -> Result<(), Error> {
    for dir in registry::PROFILE_DIRS {
        match cx.storage.mkdir(&format!("{}/{dir}", cx.home)) {
            Ok(()) | Err(Error::Service(Code::Exists)) => (),
            Err(e) => return Err(e),
        }
    }
    let path = format!("{}/{}", cx.home, registry::PROFILE_FILE);
    let bytes = db.encode_profile();
    let file = cx
        .storage
        .open(&path, Disposition::OpenAlways, READ | WRITE, SHARE_ALL)?;
    let result = cx
        .storage
        .truncate(&file, 0)
        .and_then(|()| cx.storage.write_all(&file, 0, bytes.as_bytes()))
        .and_then(|()| cx.storage.flush());
    let closed = cx.storage.close(file);
    result?;
    closed
}
fn session(
    cx: &mut Context,
    shell: &mut Shell,
    provider: &mut Provider,
    diagnostics: bool,
) -> Result<(), Error> {
    let mut client = WindowClient::connect_with(cx.registry, w::CAP_SEARCH_PROVIDER)?;
    let hello = loop {
        match client.shell_hello() {
            Ok(h) => break h,
            // Init has not reported this process to the window server yet.
            Err(Error::Service(Code::WouldBlock)) => door_sys::sleep(door_sys::time()? + 5)?,
            Err(e) => return Err(e),
        }
    };
    enumerate(&mut client, shell)?;
    door_rt::diagnostic(format_args!(
        "DOOR_P14 SHELL READY windows={} generation={} locked={} search={}\n",
        shell.windows.len(),
        hello.generation,
        u8::from(hello.locked),
        u8::from(client.is_search_provider())
    ));
    if client.is_search_provider() {
        door_rt::diagnostic(format_args!(
            "DOOR_P12 SEARCH_HOST READY items={}\n",
            provider.index().len()
        ));
    }
    shell.taskbar_dirty = true;
    shell.catalog_dirty = true;
    shell.notices_dirty = true;
    cx.notice_generation = 0;
    cx.published = None;
    loop {
        let now = door_sys::time()?;
        let mut busy = false;
        if cx.storage.needs_reconnect()
            && let Ok(s) = Storage::connect(cx.registry)
        {
            cx.storage = s;
        }
        // Start file search (window events for the provider).
        let polled = client.poll()?;
        if let Some(e) = polled {
            busy = true;
            if let Some((request, fallback)) =
                provider.event(&mut client, &mut cx.storage, e, now)?
            {
                dispatch(&mut client, request, fallback)?;
            }
        }
        // door.shell events.
        for _ in 0..EVENTS_PER_PASS {
            let Some(e) = client.shell_poll()? else { break };
            busy = true;
            for effect in shell.on_event(e) {
                apply(cx, shell, &mut client, effect)?;
            }
        }
        if now >= cx.notice_due {
            cx.notice_due = now + NOTICE_POLL_TICKS;
            refresh_notices(cx, shell, now);
            if diagnostics {
                let fault = alloc::format!("{}/{FAULT_FILE}", cx.home);
                if cx.storage.remove(&fault).is_ok() {
                    door_rt::diagnostic(format_args!(
                        "DOOR_P14 SHELL FAULT_INJECTED
"
                    ));
                    panic!("injected desktop shell fault (diagnostics image)");
                }
            }
        }
        shell.tick(now);
        publish(cx, shell, &mut client, now)?;
        if !cx.storage.needs_reconnect() {
            provider.service(&mut cx.storage, &mut client, now)?;
        }
        if !busy && polled.is_none() && !client.pending_events() {
            let spent = door_sys::time()?.saturating_sub(now);
            let mut until = now + spent + provider.idle_ticks(now).min(NOTICE_POLL_TICKS);
            if let Some(deadline) = shell.presenter.next_deadline() {
                until = until.min(deadline.max(now + 1));
            }
            door_sys::sleep(until)?;
        }
    }
}
fn enumerate(client: &mut WindowClient, shell: &mut Shell) -> Result<(), Error> {
    let mut windows = Vec::new();
    let mut after = 0;
    let mut out = [0; s::WindowPage::MAX_BYTES];
    loop {
        let n = client.shell_windows(after, &mut out)?;
        let page = s::WindowPage::decode(&out[..n]).map_err(|_| Error::Malformed)?;
        let Some(w) = page.window else { break };
        after = w.id;
        windows.push(model::Window::from_record(&w));
        if windows.len() > 64 {
            break;
        }
    }
    shell.set_windows(windows);
    Ok(())
}
/// One window's current record (the first id >= `id`).
fn fetch(client: &mut WindowClient, shell: &mut Shell, id: u64) -> Result<(), Error> {
    let mut out = [0; s::WindowPage::MAX_BYTES];
    let n = client.shell_windows(id.saturating_sub(1), &mut out)?;
    let page = s::WindowPage::decode(&out[..n]).map_err(|_| Error::Malformed)?;
    match page.window {
        Some(w) if w.id == id => shell.upsert(model::Window::from_record(&w)),
        _ => shell.remove(id),
    }
    Ok(())
}
fn notifications(cx: &mut Context) -> Option<&mut Notifications> {
    if cx.notices.is_none() {
        cx.notices = Notifications::connect(cx.registry).ok();
    }
    cx.notices.as_mut()
}
fn apply(
    cx: &mut Context,
    shell: &mut Shell,
    client: &mut WindowClient,
    effect: Effect,
) -> Result<(), Error> {
    match effect {
        Effect::Enumerate => enumerate(client, shell)?,
        Effect::Fetch(id) => fetch(client, shell, id)?,
        Effect::Launch(target) => {
            if let Err(error) = launch(cx, client, &target) {
                let name = match &target {
                    Target::Native(c) => shell.db.by_command(*c).map(|r| r.name.clone()),
                    Target::Executable(p) => shell.db.by_path(p).map(|r| r.name.clone()),
                }
                .unwrap_or_else(|| String::from("the app"));
                let code = match error {
                    Error::Service(code) => Some(code),
                    _ => None,
                };
                let (title, body) = door_desktop_shell::launch_failure(&name, code);
                notice_call(cx, |c| {
                    c.post(&n::Post {
                        icon: s::icon::WINDOWS_APP,
                        flags: 0,
                        title: &title,
                        body: &body,
                        actions: ["", ""],
                    })
                    .map(|_| ())
                });
            }
        }
        Effect::OpenApp(command) => {
            let _ = client.launch_app(u32::from(command));
        }
        Effect::Close(id) => {
            let _ = client.shell_action(id, s::action::CLOSE);
        }
        Effect::Persist => {
            let result = persist(cx, &shell.db);
            door_rt::diagnostic(format_args!(
                "DOOR_P14 SHELL PINS taskbar={} start={} saved={}\n",
                shell.db.taskbar_pins.len(),
                shell.db.start_pins.len(),
                u8::from(result.is_ok())
            ));
        }
        Effect::NoticeClear(id) => notice_call(cx, |c| c.clear(id)),
        Effect::NoticeDnd(on) => notice_call(cx, |c| c.set_dnd(on)),
        Effect::NoticeActivate(id, action) => notice_call(cx, |c| c.activate(id, action)),
        Effect::NoticeSeen => notice_call(cx, |c| c.mark_read()),
        Effect::FocusStart(minutes) => notice_call(cx, |c| c.focus_start(minutes)),
        Effect::FocusStop => notice_call(cx, |c| c.focus_stop()),
        Effect::FocusLength(minutes) => notice_call(cx, |c| c.focus_length(minutes)),
    }
    Ok(())
}
fn notice_call(cx: &mut Context, call: impl FnOnce(&mut Notifications) -> Result<(), Error>) {
    if let Some(c) = notifications(cx)
        && call(c).is_err()
    {
        cx.notices = None;
    }
    // Read the outcome back at once.
    cx.notice_due = 0;
    cx.notice_generation = 0;
}
fn launch(cx: &mut Context, client: &mut WindowClient, target: &Target) -> Result<(), Error> {
    let result = match target {
        Target::Native(command) => client.launch_app(u32::from(*command)),
        Target::Executable(path) => {
            // Like a Start shortcut whose "Start in" is the user profile: the
            // installed program folder is read-only for the user.
            let folder = cx.home.as_str();
            // Argument boundaries are preserved: argv[0] is the path alone;
            // no display name is ever concatenated into a command line.
            let mut arguments = Vec::from(path.as_bytes());
            arguments.push(0);
            door_service_client::Applications::connect(cx.registry)
                .and_then(|mut apps| apps.execute_detached(path, folder, &arguments, cx.directory))
                .map(|child| {
                    let _ = door_sys::close(child);
                })
        }
    };
    let what = match target {
        Target::Native(c) => format!("native:{c}"),
        Target::Executable(p) => format!("exe:{p}"),
    };
    match &result {
        Ok(()) => door_rt::diagnostic(format_args!("DOOR_P14 SHELL LAUNCH {what} ok=1\n")),
        Err(e) => door_rt::diagnostic(format_args!(
            "DOOR_P14 SHELL LAUNCH {what} ok=0 error={e:?}\n"
        )),
    }
    result
}
fn refresh_notices(cx: &mut Context, shell: &mut Shell, now: u64) {
    let known = cx.notice_generation;
    let Some(c) = notifications(cx) else { return };
    let state = match c.state() {
        Ok(state) => state,
        Err(_) => {
            cx.notices = None;
            return;
        }
    };
    if let Ok(status) = c.status() {
        shell.set_status(status, now);
    }
    // Settings edits a sender's flags in the registry: re-read the entries
    // whose key version moved (one open per sender, every few polls).
    if now >= cx.senders_due {
        cx.senders_due = now + SENDER_POLL_TICKS;
        let mut changed = false;
        for key in shell.senders.keys() {
            let known = shell.senders.version(&key).unwrap_or(0);
            match senders::store::refresh(&mut cx.storage, &key, known) {
                Ok(None) => changed |= shell.senders.forget(&key),
                Ok(Some(Some((flags, version)))) => {
                    changed |= shell.senders.learn(&key, flags, version)
                }
                Ok(Some(None)) | Err(_) => (),
            }
        }
        if changed {
            shell.sender_changed(now);
        }
    }
    let Some(c) = notifications(cx) else { return };
    if state.generation == known && shell.notices.len() == state.count as usize {
        create_senders(cx, shell, now);
        return;
    }
    let mut list = Vec::new();
    let mut out = [0; BODY_BYTES];
    for id in &state.ids[..state.count as usize] {
        if let Ok(len) = c.get(*id, &mut out)
            && let Ok(r) = n::Record::decode(&out[..len])
        {
            list.push(Notice::from_record(&r));
        }
    }
    cx.notice_generation = state.generation;
    shell.set_notices(list, state.dnd, state.unread, now);
    create_senders(cx, shell, now);
}
/// Give every sender that posted without a registry entry its entry, so
/// Settings lists it (Windows adds a sender the first time it notifies).
fn create_senders(cx: &mut Context, shell: &mut Shell, now: u64) {
    let mut changed = false;
    for (key, name) in shell.senders.take_pending() {
        if let Ok((flags, version)) = senders::store::create(&mut cx.storage, &key, &name) {
            changed |= shell.senders.learn(&key, flags, version);
        }
    }
    if changed {
        shell.sender_changed(now);
    }
}
/// Init's admitted executable for `pid`, cached for the process lifetime.
fn path_of(cx: &mut Context, pid: u64) -> Option<String> {
    if let Some((_, path)) = cx.identities.iter().find(|(p, _)| *p == pid) {
        return path.clone();
    }
    let mut out = [0; BODY_BYTES];
    let path = notifications(cx)
        .and_then(|c| c.app_identity(pid, &mut out).ok())
        .and_then(|len| {
            n::Identity::decode(&out[..len])
                .ok()
                .map(|i| String::from(i.path))
        })
        .filter(|p| !p.is_empty());
    if cx.identities.len() >= 64 {
        cx.identities.remove(0);
    }
    cx.identities.push((pid, path.clone()));
    path
}
fn publish(
    cx: &mut Context,
    shell: &mut Shell,
    client: &mut WindowClient,
    now: u64,
) -> Result<(), Error> {
    if shell.taskbar_dirty {
        shell.taskbar_dirty = false;
        let pids: Vec<u64> = shell.windows.iter().map(|w| w.pid).collect();
        let paths: Vec<(u64, Option<String>)> =
            pids.into_iter().map(|p| (p, path_of(cx, p))).collect();
        let groups = model::taskbar(&shell.db, &shell.windows, |pid| {
            paths
                .iter()
                .find(|(p, _)| *p == pid)
                .and_then(|(_, path)| path.clone())
        });
        // Unchanged groups (a focus change among catalog apps): nothing to
        // send, nothing for the window server to repaint.
        let unchanged = cx.published.as_ref() == Some(&groups);
        let generation = shell.next_generation();
        let count = groups.len() as u8;
        let result = if unchanged {
            Ok(())
        } else if groups.is_empty() {
            client.shell_publish(&s::Entry::clear(s::kind::TASKBAR, generation))
        } else {
            groups
                .iter()
                .enumerate()
                .try_for_each(|(i, g)| client.shell_publish(&g.entry(generation, i as u8, count)))
        };
        let shown = (
            groups.len(),
            groups.iter().map(|g| g.ids.len()).sum::<usize>(),
        );
        if result.is_ok() {
            cx.published = Some(groups.clone());
        }
        match result {
            Ok(()) if shown != cx.taskbar_logged => {
                cx.taskbar_logged = shown;
                door_rt::diagnostic(format_args!(
                    "DOOR_P14 SHELL TASKBAR groups={} windows={}\n",
                    shown.0, shown.1
                ))
            }
            Ok(()) => (),
            // A window closed meanwhile: its REMOVED follows; publish again.
            Err(Error::Service(Code::NotFound | Code::InvalidRequest)) => {
                shell.taskbar_dirty = true
            }
            Err(e) => return Err(e),
        }
    }
    if shell.catalog_dirty {
        shell.catalog_dirty = false;
        let catalog = model::catalog(&shell.db);
        let generation = shell.next_generation();
        let count = catalog.len() as u8;
        if catalog.is_empty() {
            client.shell_publish(&s::Entry::clear(s::kind::CATALOG, generation))?;
        }
        for (i, (key, icon, flags, label)) in catalog.iter().enumerate() {
            client.shell_publish(&s::Entry {
                kind: s::kind::CATALOG,
                generation,
                index: i as u8,
                count,
                key: *key,
                icon: *icon,
                flags: *flags,
                progress_state: 0,
                progress: 0,
                windows: 0,
                ids: [0; s::GROUP_WINDOWS],
                label: s::clip(label, s::LABEL_BYTES),
            })?;
        }
    }
    if shell.notices_dirty {
        shell.notices_dirty = false;
        let views = shell.presenter.views(&shell.db, &shell.notices, now);
        let generation = shell.next_generation();
        let badge = shell.unread;
        let focus = shell.focus_view(now);
        if views.is_empty() {
            client.shell_notify_view(
                &s::NoticeView::empty(generation, shell.dnd).with_focus(focus),
            )?;
        } else {
            let count = views.len() as u8;
            for (i, v) in views.iter().enumerate() {
                client.shell_notify_view(
                    &v.record(generation, i as u8, count, shell.dnd, badge, focus),
                )?;
            }
        }
        door_rt::diagnostic(format_args!(
            "DOOR_P14 SHELL NOTICES count={} toasts={} unread={} dnd={}\n",
            views.len(),
            views.iter().filter(|v| v.toast).count(),
            badge,
            u8::from(shell.dnd)
        ));
    }
    Ok(())
}
/// Send an OPEN_DOCUMENT request, falling back to File Explorer.
fn dispatch(
    client: &mut WindowClient,
    request: shell_search::Request,
    fallback: Option<shell_search::Request>,
) -> Result<(), Error> {
    match client.open_document(request.app, &request.payload) {
        Ok(()) => Ok(()),
        Err(Error::Service(_)) => match fallback {
            Some(f) => match client.open_document(f.app, &f.payload) {
                Ok(()) | Err(Error::Service(_)) => Ok(()),
                Err(error) => Err(error),
            },
            None => Ok(()),
        },
        Err(error) => Err(error),
    }
}
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    door_rt::panic_with_info(info)
}
