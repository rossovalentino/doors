//! Protected service runtime and native threads. Address spaces and stack
//! contexts have independent lifetimes; retirement, never selection, frees them.
mod calls;
mod execute;
mod monitor;
mod threads;
use super::*;
pub use calls::dispatch;
use door_abi::{
    HandleKind,
    platform::{self as abi, Identity, LaunchResult, StartupV2},
};
use door_kernel::executive::Authority;
use sha2::{Digest, Sha256};
pub use threads::wait_process;
use threads::{thread_create, wait_record};

pub(super) struct Process {
    pid: ProcessId,
    capture: Option<execute::Capture>,
    identity: Identity,
    personality: Personality,
    mode: u64,
    pub(super) exception_entry: u64,
    exited: ObjectId,
    sleep: ObjectId,
    starting: bool,
    stopping: Option<i64>,
    exit_owner: Option<TaskId>,
    last_exit: i64,
    creating_threads: u32,
    /// System DLLs LoadLibrary brought in after startup (catalog indices,
    /// in load order): part of the application's plan from then on.
    windows_roots: [u8; door_pe_loader::MAX_MODULES],
    windows_root_count: usize,
    /// Private DLLs LoadLibrary brought in (not system DLLs), their bytes
    /// held like the executable's until the process retires; the roots
    /// list orders them (`windows::PRIVATE_ROOT | index`).
    windows_private: [Option<execute::Capture>; door_windows_abi::modules::PRIVATE_MODULES],
    /// The first thread of a CREATE_SUSPENDED launch, until PROCESS_RESUME.
    held: Option<TaskId>,
}
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum Personality {
    Native,
    WindowsX64,
}
impl Personality {
    fn for_manifest(manifest: u64) -> Self {
        if matches!(
            manifest,
            abi::manifest::WINDOWS
                | abi::manifest::WINDOWS_BASE
                | abi::manifest::WINDOWS_APPLICATION
                | abi::manifest::WINDOWS_GUI
        ) {
            Self::WindowsX64
        } else {
            Self::Native
        }
    }
}
pub(super) struct Thread {
    key: u64,
    pub memory: Option<usize>,
    lease: Option<Lease>,
    reaping: bool,
    pub(super) exception: Option<super::windows::exception::Pending>,
    /// SuspendThread count (the task is suspended while it is nonzero).
    suspends: u32,
    /// The main thread's own exit event, made when it first opens itself
    /// (its record's event is the process's, P18).
    own_exit: Option<ObjectId>,
}
static IMAGES: Storage<[&'static [u8]; 14]> = Storage(UnsafeCell::new(None));
static DESKTOP_ASSETS: Storage<&'static [u8]> = Storage(UnsafeCell::new(None));
/// Workspace application executables by launch mode (ADR 0018): one image
/// per app, one manifest (Workspace) for all of them, so app identity,
/// policy and window ownership stay keyed by (manifest, mode).
static WORKSPACE_IMAGES: Storage<[&'static [u8]; 5]> = Storage(UnsafeCell::new(None));
/// Validated `system/app-assets.pack`; copied once into one pinned resource
/// section on the first desktop application launch (key in APP_ASSETS_KEY).
static APP_ASSETS: Storage<&'static [u8]> = Storage(UnsafeCell::new(None));
static APP_ASSETS_KEY: AtomicU64 = AtomicU64::new(0);
/// On-demand read-only resources (`abi::resource::PATHS`, the full Windows
/// fonts) from the authenticated initfs; empty when absent. Each is copied
/// into its own pinned section on its first `RESOURCE_OPEN`.
static RESOURCES: Storage<[&'static [u8]; abi::resource::PATHS.len()]> =
    Storage(UnsafeCell::new(None));
static RESOURCE_KEYS: [AtomicU64; abi::resource::PATHS.len()] =
    [const { AtomicU64::new(0) }; abi::resource::PATHS.len()];
/// The session input-method executable (`bin/door-ime.elf`, manifest
/// `IME`); empty when the desktop archive does not carry it.
/// The inbox apps' image (`door-inbox`; every `abi::manifest::INBOX`
/// identity runs it). Optional: Init reports a failed launch without it.
static INBOX_IMAGE: Storage<&'static [u8]> = Storage(UnsafeCell::new(None));
fn inbox_image() -> Result<&'static [u8], Status> {
    unsafe { *INBOX_IMAGE.0.get() }
        .filter(|image| !image.is_empty())
        .ok_or(Status::NotFound)
}
static IME_IMAGE: Storage<&'static [u8]> = Storage(UnsafeCell::new(None));
fn ime_image() -> Result<&'static [u8], Status> {
    unsafe { *IME_IMAGE.0.get() }
        .filter(|image| !image.is_empty())
        .ok_or(Status::NotFound)
}
/// Accessory executables by manifest (`bin/door-assist.elf` for ASSIST,
/// `bin/door-gallery.elf` for GALLERY) and the P14 desktop shell
/// (`bin/door-desktop-shell.elf` for DESKTOP_SHELL); empty when the archive
/// lacks one (Init then reports the launch as failed).
const ACCESSORY_IMAGE_PATHS: [(u64, &str); 5] = [
    (abi::manifest::ASSIST, "bin/door-assist.elf"),
    (abi::manifest::GALLERY, "bin/door-gallery.elf"),
    (abi::manifest::DESKTOP_SHELL, "bin/door-desktop-shell.elf"),
    // The P15 network service; optional like the others.
    (abi::manifest::NETWORK, "bin/door-network.elf"),
    // The P15 sound service.
    (abi::manifest::AUDIO, "bin/door-audio.elf"),
];
static ACCESSORY_IMAGES: Storage<[&'static [u8]; 5]> = Storage(UnsafeCell::new(None));
fn accessory_image(manifest: u64) -> Result<&'static [u8], Status> {
    let slot = ACCESSORY_IMAGE_PATHS
        .iter()
        .position(|(m, _)| *m == manifest)
        .ok_or(Status::NotFound)?;
    let images = unsafe {
        (*ACCESSORY_IMAGES.0.get())
            .as_ref()
            .ok_or(Status::NotFound)?
    };
    Some(images[slot])
        .filter(|image| !image.is_empty())
        .ok_or(Status::NotFound)
}
/// Who may open on-demand resource `id`: the fonts go to painting desktop
/// processes, the input-method dictionaries only to the session IME.
fn resource_allowed(manifest: u64, id: u64) -> bool {
    match id {
        abi::resource::PINYIN_DICTIONARY | abi::resource::JAPANESE_DICTIONARY => {
            manifest == abi::manifest::IME
        }
        _ => {
            matches!(
                manifest,
                abi::manifest::WINDOW
                    | abi::manifest::WORKSPACE
                    | abi::manifest::PREVIEW
                    | abi::manifest::MONITOR
                    | abi::manifest::ASSIST
                    | abi::manifest::GALLERY
                    | abi::manifest::WINDOWS_GUI
            ) || abi::is_inbox(manifest)
        }
    }
}
const WORKSPACE_IMAGE_PATHS: [(u64, &str); 5] = [
    (abi::mode::DESKTOP, "bin/door-files.elf"),
    (abi::mode::SESSION, "bin/door-notes.elf"),
    (abi::mode::SETTINGS, "bin/door-settings.elf"),
    (abi::mode::TERMINAL, "bin/door-terminal.elf"),
    (abi::mode::SEARCH_HOST, "bin/door-search.elf"),
];
fn workspace_image(mode: u64) -> Result<&'static [u8], Status> {
    // The elevated Terminal is the same program with the full token.
    let mode = if mode == abi::mode::TERMINAL_ELEVATED {
        abi::mode::TERMINAL
    } else {
        mode
    };
    let slot = WORKSPACE_IMAGE_PATHS
        .iter()
        .position(|(m, _)| *m == mode)
        .ok_or(Status::NotFound)?;
    let images = unsafe {
        (*WORKSPACE_IMAGES.0.get())
            .as_ref()
            .ok_or(Status::NotFound)?
    };
    Some(images[slot])
        .filter(|image| !image.is_empty())
        .ok_or(Status::NotFound)
}
/// Desktop processes that paint (the search host is windowless); the
/// window server builds its Unicode font set from the pack too.
fn uses_app_assets(manifest: u64, mode: u64) -> bool {
    match manifest {
        abi::manifest::WINDOW => true,
        abi::manifest::WORKSPACE => mode != abi::mode::SEARCH_HOST,
        abi::manifest::PREVIEW
        | abi::manifest::MONITOR
        | abi::manifest::ASSIST
        | abi::manifest::GALLERY
        | abi::manifest::WINDOWS_GUI
        // Console programs draw with GDI (text, icons) as well.
        | abi::manifest::WINDOWS
        | abi::manifest::WINDOWS_APPLICATION => true,
        id => abi::is_inbox(id),
    }
}
/// Key of the one shared asset section, created on first use.
fn app_assets_section() -> Result<u64, Status> {
    let source = unsafe { (*APP_ASSETS.0.get()).ok_or(Status::NotFound)? };
    pinned_section(source, &APP_ASSETS_KEY)
}
/// RESOURCE_OPEN: a READ-only grant of on-demand resource `id` to a desktop
/// process, copying it into its pinned section on the first request.
pub(super) fn resource_open(r: Record, id: u64) -> Result<u64, Status> {
    if !resource_allowed(r.role - 100, id) {
        return Err(Status::AccessDenied);
    }
    let index = usize::try_from(id)
        .ok()
        .and_then(|id| id.checked_sub(1))
        .filter(|i| *i < abi::resource::PATHS.len())
        .ok_or(Status::InvalidArgument)?;
    let source = unsafe { (*RESOURCES.0.get()).as_ref().ok_or(Status::NotFound)?[index] };
    if source.is_empty() {
        return Err(Status::NotFound);
    }
    let key = pinned_section(source, &RESOURCE_KEYS[index])?;
    with_meta(|m| m.executive.grant_resource_section(r.pid, key))
}
/// One shared copy of immutable boot bytes in a pinned resource section,
/// created on first use and recorded in `slot` (released at shutdown).
fn pinned_section(source: &'static [u8], slot: &AtomicU64) -> Result<u64, Status> {
    let key = slot.load(Ordering::Acquire);
    if key != 0 {
        return Ok(key);
    }
    let size = (source.len() as u64 + 4095) & !4095;
    let key = next(&SECTION_KEY)?;
    with_vm(|v| unsafe {
        v.create_section(key, size)?;
        let address = v.section_capture_address(key, 0, size as usize)?;
        core::ptr::write_bytes(address as *mut u8, 0, size as usize);
        core::ptr::copy_nonoverlapping(source.as_ptr(), address as *mut u8, source.len());
        Ok::<(), Status>(())
    })?;
    if let Err(error) = with_meta(|m| m.executive.create_resource_section(key, size)) {
        with_vm(|v| unsafe { v.destroy_section(key) }).expect("resource section rollback");
        return Err(error);
    }
    if let Err(winner) = slot.compare_exchange(0, key, Ordering::AcqRel, Ordering::Acquire) {
        with_meta(|m| m.executive.release_resource_section(key)).expect("duplicate resource pin");
        cleanup_sections();
        return Ok(winner);
    }
    Ok(key)
}
static INSTANCE: AtomicU64 = AtomicU64::new(1);
static THREAD_KEY: AtomicU64 = AtomicU64::new(1);
static REAPING: AtomicBool = AtomicBool::new(false);
static INIT_RESULT: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(i64::MIN);
static INIT_DONE: AtomicBool = AtomicBool::new(false);
static PROCESSES_RETIRED: AtomicU64 = AtomicU64::new(0);
static THREADS_RETIRED: AtomicU64 = AtomicU64::new(0);

fn next(counter: &AtomicU64) -> Result<u64, Status> {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_add(1))
        .map_err(|_| Status::ResourceExhausted)
}
fn identity(pid: u64, a: Authority) -> Identity {
    Identity {
        pid,
        uid: a.uid,
        integrity: a.integrity,
        groups: a.groups,
        session: a.session,
        epoch: a.epoch,
        manifest: a.manifest,
        instance: a.instance,
        capabilities: a.capabilities,
    }
}
/// A launch refused for want of a kernel resource names the stage and the
/// VM's occupancy, so the exhausted bound is visible in the serial log.
fn launch_exhausted(stage: &str, error: Status) {
    if error == Status::ResourceExhausted {
        with_vm(|v| v.report_exhaustion(stage));
    } else {
        log(format_args!(
            "DOOR_P08 LAUNCH stage={stage} status={error:?}\n"
        ));
    }
}
fn task_error(error: door_kernel::task::Error) -> Status {
    use door_kernel::task::Error;
    match error {
        Error::Capacity | Error::Exhausted => Status::ResourceExhausted,
        Error::NotOwner => Status::AccessDenied,
        Error::InvalidObject => Status::BadHandle,
        _ => Status::InvalidArgument,
    }
}
fn wake(outcome: door_kernel::task::WaitOutcome) -> Result<u64, Status> {
    match outcome.reason {
        WakeReason::Signaled => Ok(u64::from(outcome.abandoned)),
        WakeReason::Timeout => Err(Status::Timeout),
        WakeReason::Cancelled => Err(Status::Canceled),
        WakeReason::Closed => Err(Status::BadHandle),
    }
}

pub fn cleanup_waits() {
    while let Some(wait) = with_meta(|m| m.executive.take_retired_wait()) {
        run::with(|s| s.close_object(wait)).expect("unreferenced native wait retirement");
    }
}
pub fn stop_process(pid: ProcessId, code: i64) {
    let (tasks, code): ([Option<TaskId>; door_kernel::task::MAX_TASKS], i64) = with_meta(|m| {
        let Some(process) = m.processes[pid.slot()].as_mut().filter(|p| p.pid == pid) else {
            return ([None; door_kernel::task::MAX_TASKS], code);
        };
        if process.stopping.is_none() {
            process.stopping = Some(code);
        }
        let decided = process.stopping.expect("decided process exit");
        (
            core::array::from_fn(|i| m.records[i].filter(|r| r.pid == pid).map(|r| r.task)),
            decided,
        )
    });
    for task in tasks.into_iter().flatten() {
        run::request_stop(task, code);
    }
}

/// One current Windows thread owns orderly shutdown. No VM or metadata lock
/// spans retirement. Retrying is safe while sibling construction rolls back.
fn quiesce(r: Record, code: u64) -> Result<u64, Status> {
    if code > u64::from(u32::MAX) {
        return Err(Status::InvalidArgument);
    }
    let tasks = with_meta(|m| {
        let process = m.processes[r.pid.slot()].as_mut().ok_or(Status::Canceled)?;
        if process.personality != Personality::WindowsX64 {
            return Err(Status::AccessDenied);
        }
        if process.exit_owner.is_some_and(|owner| owner != r.task) {
            return Err(Status::Canceled);
        }
        process.exit_owner = Some(r.task);
        if process.stopping.is_none() {
            process.stopping = Some(code as i64);
        }
        Ok::<_, Status>(
            core::array::from_fn::<_, { door_kernel::task::MAX_TASKS }, _>(|index| {
                m.records[index]
                    .filter(|record| record.pid == r.pid && record.task != r.task)
                    .map(|record| record.task)
            }),
        )
    })?;
    for task in tasks.into_iter().flatten() {
        run::request_stop(task, code as i64);
    }
    if with_meta(|m| {
        m.processes[r.pid.slot()].as_ref().unwrap().creating_threads != 0
            || m.records
                .iter()
                .flatten()
                .any(|record| record.pid == r.pid && record.task != r.task)
    }) {
        Err(Status::WouldBlock)
    } else {
        Ok(0)
    }
}

/// Every retired task was acknowledged on another stack. A single reaper claims
/// records before VM work; no metadata/scheduler guard spans a shootdown.
pub fn reap() {
    if REAPING
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return;
    }
    for slot in 0..door_kernel::task::MAX_TASKS {
        let record = with_meta(|m| m.records[slot].filter(|r| r.role >= 100));
        let Some(record) = record else {
            continue;
        };
        let code = run::with(|s| s.exit_result(record.task)).expect("live native thread");
        let Some(code) = code else {
            continue;
        };
        let memory = with_meta(|m| {
            let thread = m.threads[slot].as_mut().expect("native thread owner");
            assert!(!thread.reaping);
            thread.reaping = true;
            thread.memory
        });
        // reap returns only after architectural post-switch acknowledgement.
        run::with(|s| s.reap(record.task)).expect("retired native thread");
        if let Some(memory) = memory {
            with_vm(|v| unsafe { v.destroy_thread(memory) }).expect("retired secondary memory");
        }
        let completion = with_meta(|m| {
            let thread = m.threads[slot].take().unwrap();
            m.records[slot] = None;
            m.processes[record.pid.slot()].as_mut().unwrap().last_exit = code;
            if let Some(lease) = thread.lease {
                m.executive
                    .complete_thread(&lease, code)
                    .expect("thread completion once");
                Some((thread.own_exit.unwrap_or(record.exited), lease))
            } else {
                None
            }
        });
        if let Some((event, lease)) = completion {
            // Completion is visible before signaling, so another thread may
            // close the last user handle here. Keep the runtime reference until
            // after the signal; otherwise concurrent cleanup can close `event`.
            run::with(|s| s.signal_event(event)).expect("thread completion signal");
            with_meta(|m| m.executive.release_lease(lease));
        }
        THREADS_RETIRED.fetch_add(1, Ordering::Relaxed);
    }
    for slot in 0..MAX_PROCESSES {
        let retired = with_meta(|m| {
            let process = m.processes[slot].as_ref()?;
            if process.starting || m.records.iter().flatten().any(|r| r.pid == process.pid) {
                return None;
            }
            Some((
                process.pid,
                process.identity,
                process.exited,
                process.sleep,
                process.stopping.unwrap_or(process.last_exit),
            ))
        });
        let Some((pid, identity, event, sleep, code)) = retired else {
            continue;
        };
        crate::platform_device::display_gone(identity.pid);
        with_vm(|v| unsafe { v.destroy_process(slot) }).expect("all process contexts retired");
        with_meta(|m| {
            for index in 0..MAX_VIEWS {
                if let Some(view) = m.views[slot][index].take() {
                    assert!(!view.busy, "no live VM operation after all threads retired");
                    m.executive.release_lease(view.lease.unwrap());
                }
            }
            m.executive
                .exit(pid, code)
                .expect("native process exit once");
            m.processes[slot] = None;
        });
        run::with(|s| {
            s.signal_event(event).expect("process completion signal");
            s.close_object(sleep).expect("sleep retirement");
        });
        with_meta(|m| m.executive.reap(pid)).expect("native process ownership reap");
        PROCESSES_RETIRED.fetch_add(1, Ordering::Relaxed);
        log(format_args!(
            "DOOR_P08 PROCESS manifest={} pid={} instance={} uid={} session={} epoch={} exit={code}\n",
            identity.manifest,
            pid.raw(),
            identity.instance,
            identity.uid,
            identity.session,
            identity.epoch
        ));
        if identity.manifest == abi::manifest::INIT {
            INIT_RESULT.store(code, Ordering::Release);
            INIT_DONE.store(true, Ordering::Release);
        }
    }
    cleanup_sections();
    cleanup_waits();
    REAPING.store(false, Ordering::Release);
}

fn inherited(m: &Metadata, pid: ProcessId, handle: u64) -> Inherited {
    let info = m
        .executive
        .inspect(pid, handle)
        .expect("published inherited grant");
    Inherited {
        handle,
        rights: info.rights,
        kind: info.kind as u64,
    }
}

struct Prepared {
    pid: ProcessId,
    task: TaskId,
    process_handle: u64,
    control: u64,
}
fn prepare(
    parent: Option<Record>,
    authority: Authority,
    registry: u64,
    mode: u64,
    mut supplied: Option<(execute::Capture, abi::ExecuteDescriptor)>,
) -> Result<Prepared, Status> {
    let windows = if super::windows::application_manifest(authority.manifest) {
        Some(
            super::windows::application(
                supplied.as_ref().ok_or(Status::InvalidArgument)?.0.bytes(),
                authority.manifest,
            )
            .inspect_err(|e| launch_exhausted("windows-plan", *e))?,
        )
    } else if matches!(
        authority.manifest,
        abi::manifest::WINDOWS | abi::manifest::WINDOWS_BASE
    ) {
        Some(
            super::windows::image(authority.manifest)
                .inspect_err(|e| launch_exhausted("windows-image", *e))?,
        )
    } else {
        None
    };
    let image = if let Some((capture, _)) = &supplied {
        capture.bytes()
    } else if let Some(windows) = &windows {
        windows.executable()
    } else if authority.manifest == abi::manifest::WORKSPACE {
        workspace_image(mode)?
    } else if authority.manifest == abi::manifest::IME {
        ime_image()?
    } else if matches!(
        authority.manifest,
        abi::manifest::ASSIST
            | abi::manifest::GALLERY
            | abi::manifest::DESKTOP_SHELL
            | abi::manifest::NETWORK
            | abi::manifest::AUDIO
    ) {
        accessory_image(authority.manifest)?
    } else if abi::is_inbox(authority.manifest) {
        inbox_image()?
    } else {
        unsafe {
            (*IMAGES.0.get())
                .as_ref()
                .expect("authenticated service catalog")
        }
        .get((authority.manifest - 1) as usize)
        .copied()
        .ok_or(Status::NotFound)?
    };
    let entry = if let Some(windows) = &windows {
        windows.entry
    } else {
        ImagePlan::parse(image)
            .map_err(|_| Status::InvalidArgument)?
            .entry()
    };
    let pid = with_meta(|m| {
        let pid = m.executive.create_process()?;
        if let Err(error) = m.executive.assign_authority(pid, authority) {
            m.executive.exit(pid, -1).unwrap();
            m.executive.reap(pid).unwrap();
            return Err(error);
        }
        Ok(pid)
    })
    .inspect_err(|e| launch_exhausted("process", *e))?;
    let mut parent_process = 0;
    let mut parent_control = 0;
    let mut created_vm = false;
    let mut event = None;
    let mut sleep = None;
    let mut task = None;
    let outcome = (|| {
        let mut handles = [Inherited::default(); 8];
        let mut standard = [Inherited::default(); abi::STANDARD_HANDLES];
        with_meta(|m| {
            if let Some(parent) = parent {
                parent_process = m.executive.process_handle(
                    parent.pid,
                    pid,
                    rights::QUERY | rights::WAIT | rights::DUPLICATE | rights::TRANSFER,
                )?;
                let pair = m.executive.channel_pair(parent.pid, pid)?;
                parent_control = pair[0];
                handles[abi::inherited::CONTROL] = inherited(m, pid, pair[1]);
                if registry != 0 {
                    let info = m.executive.inspect(parent.pid, registry)?;
                    if info.kind != HandleKind::Port {
                        return Err(Status::InvalidArgument);
                    }
                    let copied = m.executive.copy_grant_to(
                        parent.pid,
                        registry,
                        pid,
                        rights::QUERY | rights::WRITE | rights::TRANSFER,
                    )?;
                    handles[abi::inherited::REGISTRY] = inherited(m, pid, copied);
                }
            }
            let kind = match authority.manifest {
                1 => Some(HandleKind::Launch),
                2 => Some(HandleKind::Block),
                3 => Some(HandleKind::Session),
                4 => Some(HandleKind::Console),
                11 => Some(HandleKind::Display),
                29 => Some(HandleKind::Network),
                30 => Some(HandleKind::Audio),
                _ => None,
            };
            if let Some(kind) = kind {
                let grant = m.executive.create_role_authority(pid, kind)?;
                handles[abi::inherited::AUTHORITY] = inherited(m, pid, grant);
            }
            if authority.manifest == abi::manifest::INIT {
                for (slot, uid) in [
                    (abi::inherited::LOGON_FIRST, 1001),
                    (abi::inherited::LOGON_SECOND, 1002),
                    (abi::inherited::LOGON_SERVICE, abi::SERVICE_ACCOUNT),
                ] {
                    let grant = m.executive.provision_permit(pid, uid)?;
                    handles[slot] = inherited(m, pid, grant);
                }
            }
            // A launch without a working-folder grant (cwd 0: Init starting
            // a service in the service account's session) gets the child's
            // own, as a first process does.
            let cwd = if let (Some(parent), Some((_, description))) = (parent, supplied.as_ref())
                && description.cwd != 0
            {
                if m.executive.directory_uid(parent.pid, description.cwd)? != authority.uid {
                    return Err(Status::AccessDenied);
                }
                m.executive.copy_grant_to(
                    parent.pid,
                    description.cwd,
                    pid,
                    rights::QUERY | rights::READ | rights::DUPLICATE | rights::TRANSFER,
                )?
            } else {
                m.executive.create_directory(pid)?
            };
            handles[abi::inherited::DIRECTORY] = inherited(m, pid, cwd);
            // A Windows launcher's pipe sections for the child's standard
            // handles (STARTF_USESTDHANDLES), copied with their rights.
            if let (Some(parent), Some((_, description))) = (parent, supplied.as_ref()) {
                for (slot, handle) in description.standard.iter().enumerate() {
                    if *handle == 0 {
                        continue;
                    }
                    let info = m.executive.inspect(parent.pid, *handle)?;
                    if info.kind != HandleKind::Section {
                        return Err(Status::InvalidArgument);
                    }
                    let copied = m.executive.copy_grant_to(
                        parent.pid,
                        *handle,
                        pid,
                        info.rights
                            & (rights::QUERY
                                | rights::READ
                                | rights::WRITE
                                | rights::MAP
                                | rights::DUPLICATE
                                | rights::TRANSFER),
                    )?;
                    standard[slot] = inherited(m, pid, copied);
                }
            }
            Ok::<(), Status>(())
        })?;
        if authority.manifest == abi::manifest::WINDOW {
            let source = unsafe { (*DESKTOP_ASSETS.0.get()).ok_or(Status::NotFound)? };
            let size = (source.len() as u64 + 4095) & !4095;
            let key = next(&SECTION_KEY)?;
            with_vm(|v| unsafe {
                v.create_section(key, size)?;
                let address = v.section_capture_address(key, 0, source.len())?;
                core::ptr::write_bytes(address as *mut u8, 0, size as usize);
                core::ptr::copy_nonoverlapping(source.as_ptr(), address as *mut u8, source.len());
                Ok::<(), Status>(())
            })?;
            let grant = with_meta(|m| {
                m.executive.create_section(
                    pid,
                    key,
                    size,
                    rights::READ | rights::MAP | rights::QUERY,
                )
            });
            match grant {
                Ok(handle) => {
                    handles[abi::inherited::DESKTOP_ASSETS] =
                        with_meta(|m| inherited(m, pid, handle))
                }
                Err(error) => {
                    with_vm(|v| unsafe { v.destroy_section(key) })
                        .expect("resource grant rollback");
                    return Err(error);
                }
            }
        }
        // Windows programs draw with the pack when the image carries it (the
        // desktop); the diagnostic Windows modes boot without one.
        let optional = matches!(
            authority.manifest,
            abi::manifest::WINDOWS
                | abi::manifest::WINDOWS_APPLICATION
                | abi::manifest::WINDOWS_GUI
        ) && unsafe { (*APP_ASSETS.0.get()).is_none() };
        if uses_app_assets(authority.manifest, mode) && !optional {
            let key = app_assets_section()?;
            handles[abi::inherited::APP_ASSETS] = with_meta(|m| {
                let handle = m.executive.grant_resource_section(pid, key)?;
                Ok::<_, Status>(inherited(m, pid, handle))
            })?;
        }
        let mut startup = StartupV2 {
            base: Startup {
                pid: pid.raw(),
                peer_pid: parent.map_or(0, |p| p.pid.raw()),
                role: 100 + authority.manifest,
                entry,
                tls: 0x7100_2000,
                stack_top: Vm::service_stack_top(),
                inherited: handles,
            },
            identity: identity(pid.raw(), authority),
            executable_sha256: Sha256::digest(image).into(),
            cwd: handles[abi::inherited::DIRECTORY].handle,
            mode,
            arguments: [0; abi::ARGUMENT_BYTES],
            argument_bytes: 0,
            environment: [0; abi::ENVIRONMENT_BYTES],
            environment_bytes: 0,
            standard,
        };
        let name = match authority.manifest {
            1 => b"door-init\0".as_slice(),
            2 => b"door-storage\0".as_slice(),
            3 => b"door-session\0".as_slice(),
            4 => b"door-console\0".as_slice(),
            6 => b"windows-basic.exe\0".as_slice(),
            7 => b"windows-base.exe\0".as_slice(),
            16 => b"door-ime\0".as_slice(),
            17 => b"door-assist\0".as_slice(),
            18 => b"door-gallery\0".as_slice(),
            29 => b"door-network\0".as_slice(),
            30 => b"door-audio\0".as_slice(),
            _ => b"native-workload\0".as_slice(),
        };
        startup.arguments[..name.len()].copy_from_slice(name);
        startup.argument_bytes = name.len() as u16;
        if authority.manifest == abi::manifest::WINDOWS_BASE && mode == abi::mode::WINDOWS_REOPEN {
            let argument = b"--reopen\0";
            startup.arguments[name.len()..name.len() + argument.len()].copy_from_slice(argument);
            startup.argument_bytes += argument.len() as u16;
        }
        let environment: &[u8] = if authority.uid == 1002 {
            b"HOME=/users/1002\0USER=1002\0PATH=/system/current/bin\0"
        } else if authority.uid == 1001 {
            b"HOME=/users/1001\0USER=1001\0PATH=/system/current/bin\0"
        } else {
            b"HOME=/\0USER=system\0PATH=/system/current/bin\0"
        };
        startup.environment[..environment.len()].copy_from_slice(environment);
        startup.environment_bytes = environment.len() as u16;
        if let Some((_, description)) = &supplied {
            startup.arguments = description.arguments;
            startup.argument_bytes = description.argument_bytes;
            startup.environment = description.environment;
            startup.environment_bytes = description.environment_bytes;
        }
        let mut bytes = [0; abi::STARTUP_V2_BYTES];
        startup.encode(&mut bytes)?;
        if let Some(windows) = &windows {
            // Once per boot, before the first ordinary application; each
            // attempt takes and releases the VM guard itself.
            windows
                .verify_application_rollback(pid.slot(), &startup, &bytes)
                .inspect_err(|e| launch_exhausted("vm", *e))?;
        }
        let layout = with_vm(|v| unsafe {
            if let Some(windows) = &windows {
                windows.create(v, pid.slot(), &startup, &bytes)
            } else {
                v.create_process(pid.slot(), image, &bytes, pid.raw())
            }
        })
        .inspect_err(|e| launch_exhausted("vm", *e))?;
        created_vm = true;
        // A 32-bit program's main thread: FS is the TEB32 at TEB_ADDRESS.
        let x86 = windows.is_some() && with_vm(|v| Ok::<_, Status>(v.windows_x86(pid.slot())))?;
        let context = unsafe {
            arch::create_user_context(arch::UserContext {
                compat: x86,
                entry: layout.entry,
                stack_pointer: layout.user_stack_top
                    - if x86 {
                        16
                    } else if windows.is_some() {
                        40
                    } else {
                        8
                    },
                fs_base: if x86 {
                    door_windows_abi::TEB_ADDRESS
                } else {
                    layout.tls
                },
                user_gs_base: if windows.is_some() && !x86 {
                    door_windows_abi::TEB_ADDRESS
                } else {
                    0
                },
                cr3: layout.cr3,
                kernel_stack_top: layout.kernel_stack_top,
                arguments: if windows.is_some() {
                    arch::EntryArguments::Windows {
                        bootstrap: door_windows_abi::BOOTSTRAP_ADDRESS,
                    }
                } else {
                    arch::EntryArguments::Native {
                        startup: layout.startup,
                        bytes: bytes.len() as u64,
                    }
                },
            })
        }
        .map_err(|_| Status::InvalidArgument)?;
        let new_event = || {
            run::with(|s| s.create_event(true, false))
                .map_err(task_error)
                .inspect_err(|e| launch_exhausted("event", *e))
        };
        event = Some(new_event()?);
        sleep = Some(new_event()?);
        let created = run::with(|s| s.create(context as u64, 4, 1))
            .map_err(task_error)
            .inspect_err(|e| launch_exhausted("task", *e))?;
        task = Some(created);
        let key = next(&THREAD_KEY)?;
        // Read before the metadata lock (the VM lock is never taken inside
        // it): the personality VM construction recorded.
        let x86 = with_vm(|vm| vm.windows_x86(pid.slot()));
        with_meta(|m| {
            // Revalidate session authority atomically with publishing records.
            // Revocation during VM construction must roll back; after this
            // commit it must find the task even before scheduler.start.
            m.executive.set_process_wait(pid, event.unwrap())?;
            m.executive.set_process_x86(pid, x86)?;
            m.processes[pid.slot()] = Some(Process {
                pid,
                capture: supplied.take().map(|(capture, _)| capture),
                identity: identity(pid.raw(), authority),
                mode,
                personality: Personality::for_manifest(authority.manifest),
                exception_entry: windows.as_ref().map_or(0, |image| image.exception_entry()),
                exited: event.unwrap(),
                sleep: sleep.unwrap(),
                starting: true,
                stopping: None,
                exit_owner: None,
                last_exit: 0,
                creating_threads: 0,
                windows_roots: [0; door_pe_loader::MAX_MODULES],
                windows_root_count: 0,
                windows_private: [const { None }; door_windows_abi::modules::PRIVATE_MODULES],
                held: None,
            });
            m.records[created.slot()] = Some(Record {
                pid,
                task: created,
                role: 100 + authority.manifest,
                exited: event.unwrap(),
            });
            m.threads[created.slot()] = Some(Thread {
                key,
                memory: None,
                lease: None,
                reaping: false,
                exception: None,
                suspends: 0,
                own_exit: None,
            });
            Ok::<(), Status>(())
        })?;
        if let Some(windows) = &windows {
            windows.report(pid.raw());
        }
        Ok(Prepared {
            pid,
            task: created,
            process_handle: parent_process,
            control: parent_control,
        })
    })();
    if outcome.is_err() {
        if let Some(task) = task {
            run::with(|s| {
                s.terminate(task, -1).unwrap();
                s.reap(task).unwrap();
            });
        }
        if created_vm {
            with_vm(|v| unsafe { v.destroy_process(pid.slot()) }).expect("launch VM rollback");
        }
        for wait in [event, sleep].into_iter().flatten() {
            run::with(|s| s.close_object(wait)).expect("launch event rollback");
        }
        with_meta(|m| {
            m.executive.exit(pid, -1).expect("unpublished process exit");
            m.executive.reap(pid).unwrap();
            if let Some(parent) = parent {
                if parent_process != 0 {
                    m.executive.close(parent.pid, parent_process).unwrap();
                }
                if parent_control != 0 {
                    m.executive.close(parent.pid, parent_control).unwrap();
                }
            }
        });
        cleanup_sections();
        cleanup_waits();
    }
    outcome
}
fn publish(prepared: &Prepared) -> Result<(), Status> {
    with_meta(|m| m.processes[prepared.pid.slot()].as_mut().unwrap().starting = false);
    run::with(|s| s.start(prepared.task)).map_err(task_error)
}

pub fn run_services(archive: &door_initfs::Archive<'static>, mode: u64) {
    assert_eq!(
        arch::online_mask(),
        1,
        "P08 declared single-CPU storage profile"
    );
    arch::disable_interrupts();
    execute::initialize(archive);
    if mode == abi::mode::DESKTOP {
        let source = archive
            .file("system/desktop-assets.blob")
            .expect("immutable desktop assets");
        door_initfs::desktop_assets::Pack::parse(source).expect("validated desktop assets");
        unsafe {
            *DESKTOP_ASSETS.0.get() = Some(source);
        }
        let pack = archive
            .file("system/app-assets.pack")
            .expect("immutable application assets");
        door_initfs::app_assets::Pack::parse(pack).expect("validated application assets");
        let workspace = WORKSPACE_IMAGE_PATHS.map(|(_, path)| {
            archive
                .file(path)
                .expect("authenticated workspace application image")
        });
        // Optional: a missing font or dictionary answers RESOURCE_OPEN
        // with NotFound (the IME then composes without candidates).
        let fonts = abi::resource::PATHS.map(|path| archive.file(path).unwrap_or(&[]));
        // Optional: without it Init reports the IME launch as failed and the
        // session types with keyboard layouts only.
        unsafe {
            *IME_IMAGE.0.get() = Some(archive.file("bin/door-ime.elf").unwrap_or(&[]));
            *INBOX_IMAGE.0.get() = Some(archive.file("bin/door-inbox.elf").unwrap_or(&[]));
        }
        // Optional accessories (Start search lists them; Init reports a
        // failed launch when the archive lacks one).
        let accessories = ACCESSORY_IMAGE_PATHS.map(|(_, path)| archive.file(path).unwrap_or(&[]));
        unsafe {
            *ACCESSORY_IMAGES.0.get() = Some(accessories);
        }
        unsafe {
            *APP_ASSETS.0.get() = Some(pack);
            *WORKSPACE_IMAGES.0.get() = Some(workspace);
        }
        unsafe {
            *RESOURCES.0.get() = Some(fonts);
        }
    }
    if matches!(mode, abi::mode::SESSION | abi::mode::DESKTOP) {
        super::windows::initialize_providers(archive).expect("Windows providers");
    }
    if matches!(
        mode,
        abi::mode::WINDOWS | abi::mode::WINDOWS_BASE | abi::mode::WINDOWS_REOPEN
    ) {
        super::windows::initialize(archive).expect("authenticated Windows image catalog");
    }
    let images = [
        "bin/door-init.elf",
        "bin/door-storage.elf",
        "bin/door-session.elf",
        "bin/door-console.elf",
        if mode == abi::mode::SESSION
            || (mode == abi::mode::DESKTOP && archive.file("bin/native-workload.elf").is_none())
        {
            ""
        } else {
            "bin/native-workload.elf"
        },
        "",
        "",
        "bin/door-shell.elf",
        "",
        "",
        if mode == abi::mode::DESKTOP {
            "bin/door-window.elf"
        } else {
            ""
        },
        // Workspace (12): per-mode executables, see WORKSPACE_IMAGES.
        "",
        if mode == abi::mode::DESKTOP {
            "bin/door-preview.elf"
        } else {
            ""
        },
        if mode == abi::mode::DESKTOP {
            "bin/door-monitor.elf"
        } else {
            ""
        },
    ]
    .map(|path| {
        if path.is_empty() || (path == "bin/door-shell.elf" && mode != abi::mode::SESSION) {
            &[]
        } else {
            archive
                .file(path)
                .expect("authenticated native service image")
        }
    });
    unsafe {
        assert!((*IMAGES.0.get()).is_none());
        *IMAGES.0.get() = Some(images);
    }
    let baseline = with_vm(|v| v.stats());
    let instance = next(&INSTANCE).expect("boot service incarnation");
    let policy = abi::manifest_policy(abi::manifest::INIT).expect("init manifest policy");
    let init = prepare(
        None,
        Authority {
            uid: policy.uid,
            integrity: 2,
            groups: 4,
            session: 0,
            epoch: 0,
            manifest: abi::manifest::INIT,
            instance,
            capabilities: policy.caps,
        },
        0,
        mode,
        None,
    )
    .expect("init process construction");
    publish(&init).expect("init launch");
    arch::enable_timer_service(true);
    arch::start_timer().expect("native platform timer");
    arch::enable_interrupts();
    let deadline = run::now().saturating_add(18000);
    while !INIT_DONE.load(Ordering::Acquire) {
        reap();
        if !matches!(
            mode,
            abi::mode::DEMO | abi::mode::SESSION | abi::mode::DESKTOP
        ) {
            assert!(run::now() < deadline, "native service graph deadline");
        }
        arch::wait_for_interrupt();
    }
    if !matches!(mode, abi::mode::SESSION | abi::mode::DESKTOP) {
        assert_eq!(
            INIT_RESULT.load(Ordering::Acquire),
            80,
            "init orchestration result"
        );
    }
    let shutdown_deadline = run::now().saturating_add(1000);
    let remaining: [Option<ProcessId>; MAX_PROCESSES] =
        with_meta(|m| core::array::from_fn(|i| m.processes[i].as_ref().map(|p| p.pid)));
    for pid in remaining.into_iter().flatten() {
        stop_process(pid, -1);
    }
    while with_meta(|m| m.processes.iter().any(Option::is_some)) {
        reap();
        assert!(
            run::now() < shutdown_deadline,
            "native service shutdown deadline"
        );
        arch::wait_for_interrupt();
    }
    arch::disable_interrupts();
    cleanup_waits();
    // Every grantee is gone; drop the boot pin so the shared copy retires.
    for slot in core::iter::once(&APP_ASSETS_KEY).chain(&RESOURCE_KEYS) {
        let key = slot.swap(0, Ordering::AcqRel);
        if key != 0 {
            with_meta(|m| m.executive.release_resource_section(key)).expect("resource pin");
        }
    }
    cleanup_sections();
    with_vm(|v| v.release_idle_pool()).expect("page pool release");
    let final_stats = with_vm(|v| v.stats());
    assert_eq!(final_stats.processes, 0);
    assert_eq!(final_stats.sections, 0);
    assert_eq!(final_stats.allocated_pages, baseline.allocated_pages);
    with_meta(|m| {
        assert_eq!(m.executive.object_count(), 0);
        assert_eq!(m.executive.active_sessions(), 0);
        assert!(m.records.iter().all(Option::is_none));
    });
    arch::enable_timer_service(false);
    arch::stop_timer();
    log(format_args!(
        "DOOR_P08 CLEANUP PASS processes={} threads={} leaked=0\n",
        PROCESSES_RETIRED.load(Ordering::Relaxed),
        THREADS_RETIRED.load(Ordering::Relaxed)
    ));
}

pub fn session_result() -> i64 {
    INIT_RESULT.load(Ordering::Acquire)
}
