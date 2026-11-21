//! Providers behind the pages that replace legacy administration windows:
//! the kernel's boot inventory, the disk owner, Init's service registry,
//! the user's registry hive and this process's identity. Reads happen on
//! page entry, on Refresh (F5) or on an explicit command, never while
//! painting; a failed read leaves the value unavailable with its reason.
use super::*;
use door_abi::platform::{Identity, StartupV2, inherited};
use door_service_client::{Init, RegistryRoot, RegistryView};
use door_service_protocol::administration::DiskInfo;
use door_service_protocol::files::FileInfo;
use door_workspace::{registry_browser as registry, registry_value as value};

/// Doors services listed on the Services page: (Init id, name).
pub(super) const SERVICES: [(u64, &str); 4] = [
    (2, "Storage"),
    (3, "Session"),
    (4, "Console"),
    (11, "Window"),
];
/// Newest events listed per log on the Event Viewer page.
const EVENT_ROWS: u32 = 40;
/// Local date and time text in the user's time zone and regional format.
struct Clock(u32, u32);
impl Clock {
    fn read(storage: &mut Storage) -> Self {
        let zone = storage
            .get_setting(Setting::TimeZone)
            .ok()
            .filter(|id| door_locale::tz::valid(*id))
            .unwrap_or(door_locale::tz::DEFAULT_ID as u32);
        let region = storage
            .get_setting(Setting::RegionalFormat)
            .ok()
            .filter(|id| door_locale::region::valid(*id))
            .unwrap_or(door_locale::region::DEFAULT_ID as u32);
        Self(zone, region)
    }
    fn text(&self, filetime: u64) -> String {
        let mut text = String::new();
        let region = door_locale::region::region_or_default(self.1);
        if let Some(local) = door_locale::tz::zone_or_utc(self.0).to_local(filetime) {
            let _ = region.date_time(&local.time, &mut text);
        }
        text
    }
}
/// The user's environment variables (Windows: HKEY_CURRENT_USER\Environment).
pub(super) fn environment_key() -> registry::Location {
    registry::Location {
        root: RegistryRoot::CurrentUser,
        view: RegistryView::Bits64,
        path: "Environment".encode_utf16().collect(),
    }
}
/// What a failed provider call tells the user.
pub(super) fn reason(error: Error) -> &'static str {
    match error {
        Error::Service(Code::Denied) => "Access denied: your account can't open this",
        Error::Service(Code::Changed) => "Changed elsewhere: refresh and try again",
        Error::Service(Code::NotFound) => "Not found",
        Error::Service(Code::Resources) => "The registry is full",
        Error::Service(Code::Exists) => "That name is already used",
        Error::Service(Code::InvalidRequest) => "Doors couldn't accept this",
        _ => "The service didn't answer; nothing was changed",
    }
}
pub(super) struct Admin {
    pub identity: Identity,
    registry_port: door_abi::Handle,
    /// Environment Settings started with (name, value).
    pub launch: Vec<(String, String)>,
    pub disk: Option<DiskInfo>,
    pub hardware: Option<door_abi::hardware::HardwareInfo>,
    /// Services: Some(true) registered with Init, Some(false) not found.
    pub services: [Option<bool>; 4],
    /// Troubleshoot results: (passed, detail).
    pub checks: [Option<(bool, String)>; 3],
    /// File system: the last checked path and Storage's answer.
    pub checked: Option<(String, Result<FileInfo, &'static str>)>,
    /// Registry Editor: where it is and what it read there.
    pub location: registry::Location,
    pub snapshot: Option<registry::Snapshot>,
    pub registry_error: Option<&'static str>,
    /// Environment variables: HKEY_CURRENT_USER\Environment as read.
    pub variables: Option<registry::Snapshot>,
    pub variables_error: Option<&'static str>,
    /// Event logs: (time, text), newest first, this session only.
    pub events: Vec<(String, String)>,
    /// Default apps: the apps that open files and each type's default.
    pub default_apps: Option<door_workspace::default_apps::Apps>,
    pub file_types: Vec<door_workspace::default_apps::FileType>,
    pub file_types_error: Option<&'static str>,
    /// Microphone: recent uses as shown (name, detail, desktop app).
    pub microphone: Vec<(String, String, bool)>,
    /// VPN: the user's connections (name, status line, up or coming up),
    /// as door.network reported them, and the one Advanced options shows.
    pub vpns: Vec<(String, String, bool)>,
    pub vpn_entries: Vec<door_service_protocol::net::vpn::Entry>,
    pub vpn_selected: Option<String>,
    /// Other users: the local accounts besides yours (uid, name,
    /// administrator), and the one whose page is open.
    pub users: Vec<(u32, String, bool)>,
    pub user_selected: Option<u32>,
    /// The signed-in account in the account book: (name, administrator).
    pub own_account: Option<(String, bool)>,
    /// Event Viewer, Scheduled tasks and Services rows: (title, detail, value).
    pub application_events: Vec<(String, String, String)>,
    pub system_events: Vec<(String, String, String)>,
    pub scheduled: Vec<(String, String, String)>,
    pub services_rows: Vec<(String, String, String)>,
    /// Installed apps: the desktop programs installers registered.
    pub programs: Vec<door_workspace::installed_programs::Program>,
    /// The working-folder authority programs start with.
    pub directory: door_abi::Handle,
    loaded: Option<Page>,
}
impl Admin {
    pub fn new(startup: &StartupV2) -> Self {
        let launch = startup.environment[..startup.environment_bytes as usize]
            .split(|b| *b == 0)
            .filter_map(|entry| core::str::from_utf8(entry).ok()?.split_once('='))
            .map(|(name, data)| (String::from(name), String::from(data)))
            .collect();
        Self {
            identity: startup.identity,
            registry_port: startup.base.inherited[inherited::REGISTRY].handle,
            launch,
            disk: None,
            hardware: None,
            services: [None; 4],
            checks: [None, None, None],
            checked: None,
            location: registry::Location::default(),
            snapshot: None,
            registry_error: None,
            variables: None,
            variables_error: None,
            events: Vec::new(),
            default_apps: None,
            file_types: Vec::new(),
            file_types_error: None,
            microphone: Vec::new(),
            vpns: Vec::new(),
            vpn_entries: Vec::new(),
            vpn_selected: None,
            users: Vec::new(),
            user_selected: None,
            own_account: None,
            application_events: Vec::new(),
            system_events: Vec::new(),
            scheduled: Vec::new(),
            services_rows: Vec::new(),
            programs: Vec::new(),
            directory: startup.base.inherited[inherited::DIRECTORY].handle,
            loaded: None,
        }
    }
    /// Load the providers of `page` once per visit.
    /// Returns true when it read the providers now.
    pub fn ensure(&mut self, page: Page, storage: &mut Storage) -> bool {
        if self.loaded == Some(page) {
            return false;
        }
        self.loaded = Some(page);
        self.refresh(page, storage);
        true
    }
    /// The page was left: the next visit reads its providers again.
    pub fn leave(&mut self) {
        self.loaded = None;
    }
    pub fn refresh(&mut self, page: Page, storage: &mut Storage) {
        match page {
            Page::Registry => self.reload_registry(storage),
            Page::Environment => self.reload_variables(storage),
            Page::DevicesDrivers | Page::DiskManagement | Page::BackupRestore => {
                self.hardware = door_sys::hardware_info().ok();
                self.disk = storage.disk_info().ok();
            }
            Page::Services => {
                self.discover_services();
                self.reload_scm(storage);
            }
            Page::EventViewer => self.reload_events(storage),
            Page::TaskScheduler => self.reload_tasks(storage),
            Page::DefaultApps => self.reload_file_types(storage),
            Page::InstalledApps => {
                self.programs = door_workspace::installed_programs::list(storage);
            }
            _ => return,
        }
        let _ = door_sys::write(
            format!("DOOR_P14 SETTINGS REFRESH page={}\n", page as usize).as_bytes(),
        );
    }
    /// Default apps: read the apps and every file type's current default.
    /// Installed apps' Modify and Uninstall: the program (a Windows path,
    /// matched case-insensitively like Windows' file system) starts with its
    /// arguments; one whose manifest asks for administrator rights goes
    /// through User Account Control first.
    pub fn start_program(
        &mut self,
        storage: &mut Storage,
        c: &mut door_service_client::window::WindowClient,
        program: &str,
        arguments: &[String],
    ) -> Result<(), Error> {
        let native = native_path(storage, program).ok_or(Error::Service(Code::NotFound))?;
        let bytes = {
            let file = storage.open(
                &native,
                door_service_client::Disposition::OpenExisting,
                door_service_client::READ,
                door_service_client::SHARE_ALL,
            )?;
            let read = (|| {
                let info = storage.info(&file)?;
                if info.bytes == 0 || info.bytes > door_abi::platform::EXECUTABLE_MAX_BYTES as u64 {
                    return Err(Error::Service(Code::Resources));
                }
                let mut data = alloc::vec![0u8; info.bytes as usize];
                let n = storage.read_full(&file, 0, &mut data)?;
                data.truncate(n);
                Ok(data)
            })();
            let _ = storage.close(file);
            read?
        };
        let elevate = door_image_format::PeImage::parse_with_delay_imports(&bytes)
            .ok()
            .and_then(|image| image.manifest())
            .is_some_and(|m| {
                matches!(
                    door_image_format::execution_level(m),
                    door_image_format::ExecutionLevel::RequireAdministrator
                        | door_image_format::ExecutionLevel::HighestAvailable
                )
            });
        if elevate {
            let section = door_workspace::section_from(&bytes).map_err(Error::from)?;
            let sent = c.request_elevation(
                &native,
                0,
                Some((section, bytes.len() as u32)),
                self.directory,
            );
            let _ = door_sys::close(section);
            sent?;
            let _ = door_sys::write(format!("DOOR_P16 SETTINGS RUNAS path={native}\n").as_bytes());
            return Ok(());
        }
        let folder = match native.rsplit_once('/') {
            Some(("", _)) => String::from("/"),
            Some((f, _)) => String::from(f),
            None => String::from("/"),
        };
        let mut argv = Vec::new();
        for word in core::iter::once(program).chain(arguments.iter().map(String::as_str)) {
            argv.extend_from_slice(word.as_bytes());
            argv.push(0);
        }
        let mut applications = door_service_client::Applications::connect(self.registry_port)?;
        let child = applications.execute_detached(&native, &folder, &argv, self.directory)?;
        let _ = door_sys::close(child);
        let _ = door_sys::write(format!("DOOR_P16 SETTINGS START path={native}\n").as_bytes());
        Ok(())
    }
    pub fn reload_file_types(&mut self, storage: &mut Storage) {
        use door_workspace::default_apps as d;
        let apps = d::Apps::load(storage);
        match d::file_types(storage, &apps) {
            Ok(types) => {
                self.file_types = types;
                self.file_types_error = None;
            }
            Err(_) => {
                self.file_types.clear();
                self.file_types_error = Some("Unavailable: the registry didn't answer");
            }
        }
        self.default_apps = Some(apps);
    }
    /// The apps file type `index` can be opened with: (key, name).
    pub fn file_type_choices(&self, index: usize) -> Vec<(String, String)> {
        let (Some(apps), Some(t)) = (self.default_apps.as_ref(), self.file_types.get(index)) else {
            return Vec::new();
        };
        apps.choices(&t.ext, t.default.as_deref())
            .into_iter()
            .map(|a| (a.key.clone(), a.name.clone()))
            .collect()
    }
    /// Make `key` the default for file type `index`.
    pub fn set_file_type_default(
        &mut self,
        index: usize,
        key: &str,
        storage: &mut Storage,
    ) -> Result<String, &'static str> {
        use door_workspace::default_apps as d;
        let (Some(apps), Some(t)) = (self.default_apps.as_ref(), self.file_types.get(index)) else {
            return Err("Unavailable");
        };
        let ext = t.ext.clone();
        d::set_default(storage, apps, &ext, key).map_err(|_| "The default couldn't be saved")?;
        let name = apps.get(key).map_or_else(String::new, |a| a.name.clone());
        self.reload_file_types(storage);
        let _ = door_sys::write(
            format!("DOOR_P14 SETTINGS DEFAULT_APP ext={ext} app={key}\n").as_bytes(),
        );
        Ok(format!("Default apps: {ext} opens with {name}"))
    }
    fn discover_services(&mut self) {
        self.services = [None; 4];
        let Ok(mut init) = Init::connect(self.registry_port) else {
            return;
        };
        for (slot, (id, _)) in self.services.iter_mut().zip(SERVICES) {
            *slot = Some(match init.resolve(id) {
                Ok(port) => {
                    let _ = door_sys::close(port);
                    true
                }
                Err(_) => false,
            });
        }
    }
    /// Event Viewer: the newest Application and System events.
    fn reload_events(&mut self, storage: &mut Storage) {
        use door_service_protocol::eventlog::{Log, kind};
        let clock = Clock::read(storage);
        self.application_events.clear();
        self.system_events.clear();
        let Ok(mut client) = door_service_client::EventLog::connect(self.registry_port) else {
            return;
        };
        for (log, rows) in [
            (Log::Application, &mut self.application_events),
            (Log::System, &mut self.system_events),
        ] {
            let Ok((oldest, total)) = client.info(log) else {
                continue;
            };
            for back in 0..total.min(EVENT_ROWS) {
                let Ok(Some(e)) = client.read(log, oldest + total - 1 - back) else {
                    continue;
                };
                let level = match e.kind {
                    kind::ERROR => "Error",
                    kind::WARNING => "Warning",
                    kind::AUDIT_SUCCESS => "Audit Success",
                    kind::AUDIT_FAILURE => "Audit Failure",
                    _ => "Information",
                };
                rows.push((
                    format!("{level} - {}", e.source()),
                    format!("Event ID {}: {}", e.id, e.message()),
                    clock.text(e.time),
                ));
            }
        }
    }
    /// Scheduled tasks: the Task Scheduler's tasks this user can see.
    fn reload_tasks(&mut self, storage: &mut Storage) {
        use door_service_protocol::tasks::{Schedule, Status, TASKS};
        let clock = Clock::read(storage);
        self.scheduled.clear();
        let Ok(mut client) = door_service_client::Tasks::connect(self.registry_port) else {
            return;
        };
        for index in 0..TASKS as u16 {
            let Ok(Some(s)) = client.query(index) else {
                break;
            };
            let every = |n: u16, unit: &str| {
                if n == 1 {
                    format!("Every {unit}")
                } else {
                    format!("Every {n} {unit}s")
                }
            };
            let trigger = match s.task.schedule {
                Schedule::Once => format!("One time at {}", clock.text(s.task.start)),
                Schedule::Minutes(n) => every(n, "minute"),
                Schedule::Hours(n) => every(n, "hour"),
                Schedule::Days(n) => every(n, "day"),
                Schedule::OnLogon => String::from("At log on"),
            };
            let next = if s.next_run == 0 {
                String::from("N/A")
            } else {
                clock.text(s.next_run)
            };
            let last = if s.last_run == 0 {
                String::from("The task has not yet run. (0x41303)")
            } else {
                format!("Last run result: 0x{:X}", s.last_result)
            };
            self.scheduled.push((
                String::from(s.task.name()),
                format!("{trigger}. Next run: {next}. {last}"),
                String::from(match s.state {
                    Status::RUNNING => "Running",
                    Status::DISABLED => "Disabled",
                    _ => "Ready",
                }),
            ));
        }
    }
    /// Services: what the Service Control Manager knows, with each
    /// service's display name from its registry key.
    fn reload_scm(&mut self, storage: &mut Storage) {
        use door_service_protocol::scm::{BUILTIN, state};
        self.services_rows.clear();
        let Ok(mut scm) = door_service_client::Services::connect(self.registry_port) else {
            return;
        };
        let mut listed = Vec::new();
        let _ = scm.each(|e| listed.push((String::from(e.name()), e.state, e.pid)));
        for (name, status, pid) in listed {
            let display = match BUILTIN.iter().find(|b| b.0.eq_ignore_ascii_case(&name)) {
                Some(b) => String::from(b.1),
                None => registry::service_display(storage, &name).unwrap_or_else(|| name.clone()),
            };
            let detail = if pid == 0 {
                name
            } else {
                format!("{name} - PID {pid}")
            };
            self.services_rows.push((
                display,
                detail,
                String::from(match status {
                    state::RUNNING => "Running",
                    state::PAUSED => "Paused",
                    state::STOPPED => "Stopped",
                    2 => "Starting",
                    3 => "Stopping",
                    _ => "Pending",
                }),
            ));
        }
        let listed: Vec<String> = self
            .services_rows
            .iter()
            .map(|(name, _, state)| format!("{name}: {state}"))
            .collect();
        let _ = door_sys::write(
            format!(
                "DOOR_P16 SETTINGS SERVICES count={} [{}]\n",
                listed.len(),
                listed.join(", ")
            )
            .as_bytes(),
        );
    }
    /// Troubleshoot > System check: the native interface, the clock and a
    /// Storage round trip, each reported as it happened.
    pub fn run_checks(&mut self, storage: &mut Storage) {
        self.checks[0] = Some(match door_sys::query() {
            Ok(info) if info.tick_hz > 0 => (
                true,
                format!("Working, version {}.{}", door_abi::MAJOR, door_abi::MINOR),
            ),
            Ok(_) => (false, String::from("No timer rate reported")),
            Err(e) => (false, format!("Failed: {e:?}")),
        });
        self.checks[1] = Some(match (door_sys::time(), door_sys::time()) {
            (Ok(a), Ok(b)) if b >= a => (true, String::from("Working")),
            (Ok(_), Ok(_)) => (false, String::from("The clock went backwards")),
            (Err(e), _) | (_, Err(e)) => (false, format!("Failed: {e:?}")),
        });
        self.checks[2] = Some(
            match storage.get_setting(door_service_client::Setting::DesktopAppearance) {
                Ok(_) => (true, String::from("Working")),
                Err(e) => (false, String::from(reason(e))),
            },
        );
        let passed = self.checks.iter().flatten().filter(|c| c.0).count();
        let _ = door_sys::write(format!("DOOR_P14 SETTINGS CHECKS passed={passed}\n").as_bytes());
    }
    pub fn reload_registry(&mut self, storage: &mut Storage) {
        self.snapshot = None;
        self.registry_error = None;
        match registry::load(storage, &self.location) {
            Ok(snapshot) => self.snapshot = Some(snapshot),
            Err(error) => self.registry_error = Some(reason(error)),
        }
    }
    /// Registry Editor > Root key / Registry view.
    pub fn set_root(&mut self, root: u8, view: u8, storage: Option<&mut Storage>) {
        self.location = registry::Location {
            root: match root {
                1 => RegistryRoot::LocalMachine,
                2 => RegistryRoot::ClassesRoot,
                _ => RegistryRoot::CurrentUser,
            },
            view: if view == 1 {
                RegistryView::Bits32
            } else {
                RegistryView::Bits64
            },
            path: Vec::new(),
        };
        if let Some(storage) = storage {
            self.reload_registry(storage);
        }
    }
    pub fn root_index(&self) -> (u8, u8) {
        (
            match self.location.root {
                RegistryRoot::CurrentUser => 0,
                RegistryRoot::LocalMachine => 1,
                RegistryRoot::ClassesRoot => 2,
            },
            u8::from(self.location.view == RegistryView::Bits32),
        )
    }
    pub fn registry_up(&mut self, storage: Option<&mut Storage>) {
        if let Some(parent) = self.location.parent() {
            self.location = parent;
            if let Some(storage) = storage {
                self.reload_registry(storage);
            }
        }
    }
    pub fn open_subkey(&mut self, index: usize, storage: Option<&mut Storage>) {
        let Some(next) = self
            .snapshot
            .as_ref()
            .and_then(|s| s.children.get(index).map(|name| s.location.child(name)))
        else {
            return;
        };
        match next {
            Ok(location) => {
                self.location = location;
                if let Some(storage) = storage {
                    self.reload_registry(storage);
                }
            }
            Err(error) => self.registry_error = Some(reason(error)),
        }
    }
    /// Registry Editor > Go to a key: open `text`, creating it first when
    /// `create`. Errors are the dialog's to show.
    pub fn go_to(
        &mut self,
        text: &str,
        create: bool,
        storage: &mut Storage,
    ) -> Result<(), &'static str> {
        let location = registry::Location::from_ascii(text.trim(), self.location.view)
            .map_err(|_| "That isn't a registry path. Start with HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE or HKEY_CLASSES_ROOT.")?;
        if create {
            let key = storage
                .registry_open(location.root, location.view, &location.path, 3, true)
                .map_err(reason)?;
            let flushed = storage.flush();
            let closed = storage.registry_close(key);
            flushed.and(closed).map_err(reason)?;
        }
        let snapshot = registry::load(storage, &location).map_err(|e| match e {
            Error::Service(Code::NotFound) => "That key doesn't exist. Select Create to make it.",
            other => reason(other),
        })?;
        self.location = location;
        self.snapshot = Some(snapshot);
        self.registry_error = None;
        Ok(())
    }
    pub fn reload_variables(&mut self, storage: &mut Storage) {
        self.variables = None;
        self.variables_error = None;
        match registry::load(storage, &environment_key()) {
            Ok(snapshot) => self.variables = Some(snapshot),
            // No variables saved yet: the key is created by the first one.
            Err(Error::Service(Code::NotFound)) => (),
            Err(error) => self.variables_error = Some(reason(error)),
        }
        // Guest scenarios check that saved variables are read back.
        let names: Vec<String> = self.variables.as_ref().map_or_else(Vec::new, |s| {
            s.values
                .iter()
                .map(|v| registry::display_units(v.name.units()))
                .collect()
        });
        let _ = door_sys::write(
            format!("DOOR_P14 SETTINGS VARIABLES names={}\n", names.join(",")).as_bytes(),
        );
    }
    /// Environment variables: save `name` = `data` (a new variable when
    /// `new`) in HKEY_CURRENT_USER\Environment, creating the key if needed.
    /// Environment variables > Edit > Delete: remove a user variable.
    pub fn delete_variable(&mut self, name: &str, storage: &mut Storage) -> Result<(), String> {
        let snapshot = self
            .variables
            .clone()
            .ok_or_else(|| String::from("You haven't saved any variables."))?;
        let units: Vec<u16> = name.encode_utf16().collect();
        registry::delete_value(storage, &snapshot, &units).map_err(|e| String::from(reason(e)))?;
        self.reload_variables(storage);
        Ok(())
    }
    pub fn save_variable(
        &mut self,
        name: &str,
        data: &str,
        new: bool,
        storage: &mut Storage,
    ) -> Result<(), String> {
        if name.is_empty()
            || name.len() > 48
            || name.contains('=')
            || !name.bytes().all(|b| (33..=126).contains(&b))
        {
            return Err(String::from(
                "Use a name of up to 48 letters, digits or symbols, without spaces or =.",
            ));
        }
        let kind = self
            .variables
            .as_ref()
            .and_then(|s| {
                s.values.iter().find(|v| {
                    registry::same_name(v.name.units(), &name.encode_utf16().collect::<Vec<_>>())
                })
            })
            .map_or(1, |v| v.value.kind);
        let parsed = value::parse(kind, data).map_err(|e| match e {
            value::Invalid::TooLong => {
                String::from("The value is too long: use at most 31 characters.")
            }
            _ => format!("The value isn't valid ({e:?})."),
        })?;
        if self.variables.is_none() {
            let key = environment_key();
            let opened = storage
                .registry_open(key.root, key.view, &key.path, 3, true)
                .map_err(|e| String::from(reason(e)))?;
            let flushed = storage.flush();
            let closed = storage.registry_close(opened);
            flushed.and(closed).map_err(|e| String::from(reason(e)))?;
            self.reload_variables(storage);
        }
        let snapshot = self.variables.clone().ok_or_else(|| {
            String::from(
                self.variables_error
                    .unwrap_or("The variables couldn't be read."),
            )
        })?;
        let units: Vec<u16> = name.encode_utf16().collect();
        registry::save(storage, &snapshot, &units, parsed, new)
            .map_err(|e| String::from(reason(e)))?;
        self.reload_variables(storage);
        Ok(())
    }
    /// File system > Check access: what Storage reports for `path` under
    /// this session's permissions.
    pub fn check_access(&mut self, path: &str, storage: &mut Storage) {
        let result = storage.metadata(path.trim()).map_err(reason);
        self.checked = Some((String::from(path.trim()), result));
    }
    /// Event logs: a Settings change, newest first (bounded).
    pub fn log(&mut self, time: String, text: String) {
        let _ = door_sys::write(format!("DOOR_P14 SETTINGS EVENT {text}\n").as_bytes());
        self.events.insert(0, (time, text));
        self.events.truncate(64);
    }
    pub fn lists(&self) -> [usize; catalog::List::COUNT] {
        catalog::List::ALL.map(|list| self.count(list))
    }
    pub fn count(&self, list: catalog::List) -> usize {
        match list {
            catalog::List::RegistryKeys => self.snapshot.as_ref().map_or(0, |s| s.children.len()),
            catalog::List::RegistryValues => self.snapshot.as_ref().map_or(0, |s| s.values.len()),
            catalog::List::UserVariables => self.variables.as_ref().map_or(0, |s| s.values.len()),
            catalog::List::LaunchVariables => self.launch.len(),
            catalog::List::Events => self.events.len(),
            catalog::List::FileTypes => self.file_types.len(),
            catalog::List::MicrophoneApps => self.microphone.iter().filter(|m| m.2).count(),
            catalog::List::MicrophoneActivity => self.microphone.len(),
            catalog::List::VpnConnections => self.vpns.len(),
            catalog::List::OtherUsers => self.users.len(),
            catalog::List::ApplicationEvents => self.application_events.len(),
            catalog::List::SystemEvents => self.system_events.len(),
            catalog::List::ScheduledTasks => self.scheduled.len(),
            catalog::List::InstalledServices => self.services_rows.len(),
        }
    }
    /// What list entry `index` shows: (title, description, value).
    pub fn entry(&self, list: catalog::List, index: usize) -> Option<(String, String, String)> {
        match list {
            catalog::List::RegistryKeys => {
                let name = self.snapshot.as_ref()?.children.get(index)?;
                Some((
                    registry::display_units(name.units()),
                    String::from("Key"),
                    String::new(),
                ))
            }
            catalog::List::RegistryValues | catalog::List::UserVariables => {
                let snapshot = if list == catalog::List::RegistryValues {
                    self.snapshot.as_ref()?
                } else {
                    self.variables.as_ref()?
                };
                let entry = snapshot.values.get(index)?;
                let name = if entry.name.units().is_empty() {
                    String::from("(Default)")
                } else {
                    registry::display_units(entry.name.units())
                };
                Some((
                    name,
                    format!(
                        "{}  {}",
                        value::label(entry.value.kind),
                        value::format(entry.value)
                    ),
                    String::new(),
                ))
            }
            catalog::List::LaunchVariables => {
                let (name, data) = self.launch.get(index)?;
                Some((name.clone(), data.clone(), String::new()))
            }
            catalog::List::Events => {
                let (time, text) = self.events.get(index)?;
                Some((text.clone(), time.clone(), String::new()))
            }
            catalog::List::MicrophoneApps => {
                let (name, detail, _) = self.microphone.iter().filter(|m| m.2).nth(index)?;
                Some((name.clone(), detail.clone(), String::new()))
            }
            catalog::List::MicrophoneActivity => {
                let (name, detail, _) = self.microphone.get(index)?;
                Some((name.clone(), detail.clone(), String::new()))
            }
            catalog::List::VpnConnections => {
                let (name, status, _) = self.vpns.get(index)?;
                Some((name.clone(), status.clone(), String::new()))
            }
            catalog::List::ApplicationEvents
            | catalog::List::SystemEvents
            | catalog::List::ScheduledTasks
            | catalog::List::InstalledServices => {
                let rows = match list {
                    catalog::List::ApplicationEvents => &self.application_events,
                    catalog::List::SystemEvents => &self.system_events,
                    catalog::List::ScheduledTasks => &self.scheduled,
                    _ => &self.services_rows,
                };
                let (title, detail, value) = rows.get(index)?;
                Some((title.clone(), detail.clone(), value.clone()))
            }
            catalog::List::OtherUsers => {
                let (_, name, administrator) = self.users.get(index)?;
                Some((
                    name.clone(),
                    String::from(if *administrator {
                        "Administrator - Local account"
                    } else {
                        "Local account"
                    }),
                    String::new(),
                ))
            }
            catalog::List::FileTypes => {
                let t = self.file_types.get(index)?;
                let app = t
                    .default
                    .as_deref()
                    .and_then(|k| self.default_apps.as_ref()?.get(k))
                    .map_or_else(|| String::from("Choose a default"), |a| a.name.clone());
                Some((t.ext.clone(), app, String::new()))
            }
        }
    }
    /// The line a list shows when it is empty (or why it couldn't be read).
    pub fn empty_text(&self, list: catalog::List) -> &'static str {
        match list {
            catalog::List::RegistryKeys | catalog::List::RegistryValues => {
                self.registry_error.unwrap_or(list.empty())
            }
            catalog::List::UserVariables => self.variables_error.unwrap_or(list.empty()),
            catalog::List::FileTypes => self.file_types_error.unwrap_or(list.empty()),
            _ => list.empty(),
        }
    }
    #[cfg(test)]
    pub fn review(&mut self) {
        self.hardware = Some(door_abi::hardware::HardwareInfo {
            power_profile: 2,
            processors: 8,
        });
        self.disk = Some(DiskInfo {
            sectors: 262144,
            volume_first: 2048,
            volume_sectors: 65536,
            journal_first: 67584,
            journal_sectors: 4096,
            writable: true,
            flush: true,
        });
        self.services = [Some(true); 4];
        let name = |n: &str| {
            door_service_client::RegistryName::new(&n.encode_utf16().collect::<Vec<_>>()).unwrap()
        };
        self.snapshot = Some(registry::Snapshot {
            location: self.location.clone(),
            version: 1,
            children: ["Software", "Environment"].into_iter().map(name).collect(),
            values: [("ReviewLabel", 1, "Settings review"), ("Scale", 4, "100")]
                .into_iter()
                .map(|(n, kind, data)| door_service_client::RegistryEntry {
                    name: name(n),
                    value: value::parse(kind, data).unwrap(),
                })
                .collect(),
        });
        self.variables = Some(registry::Snapshot {
            location: environment_key(),
            version: 1,
            children: Vec::new(),
            values: [("EDITOR", 1, "notes")]
                .into_iter()
                .map(|(n, kind, data)| door_service_client::RegistryEntry {
                    name: name(n),
                    value: value::parse(kind, data).unwrap(),
                })
                .collect(),
        });
        self.launch = alloc::vec![(String::from("HOME"), String::from("/users/1001"))];
        self.events = alloc::vec![(String::from("This session"), String::from("Theme: Dark"))];
    }
    /// Registry review state: these subkeys and values at the current key.
    #[cfg(test)]
    pub fn review_registry(&mut self, children: &[&str], values: &[(&str, u8, &str)]) {
        let name = |n: &str| {
            door_service_client::RegistryName::new(&n.encode_utf16().collect::<Vec<_>>()).unwrap()
        };
        self.snapshot = Some(registry::Snapshot {
            location: self.location.clone(),
            version: 1,
            children: children.iter().map(|n| name(n)).collect(),
            values: values
                .iter()
                .map(|(n, kind, data)| door_service_client::RegistryEntry {
                    name: name(n),
                    value: value::parse(*kind, data).unwrap(),
                })
                .collect(),
        });
    }
}

/// A Windows path on C: as the native path of what is there, each name
/// matched case-insensitively.
fn native_path(storage: &mut Storage, windows: &str) -> Option<String> {
    let rest = windows
        .get(..3)
        .filter(|d| d.eq_ignore_ascii_case("C:\\"))
        .map(|_| &windows[3..])?;
    let mut parts = rest.split('\\').filter(|p| !p.is_empty());
    let first = parts.next()?;
    let mut path = if first.eq_ignore_ascii_case("Users") {
        String::from("/users")
    } else {
        // The rest of C: lives under /compat.
        let mut p = String::from("/compat");
        p = match_child(storage, &p, first)?;
        p
    };
    for part in parts {
        path = match_child(storage, &path, part)?;
    }
    Some(path)
}
fn match_child(storage: &mut Storage, folder: &str, name: &str) -> Option<String> {
    let mut after = 0;
    for _ in 0..4096 {
        let entry = storage.list(folder, after).ok()??;
        after = entry.cookie;
        let found = String::from_utf16_lossy(&entry.name[..entry.name_len]);
        if found.eq_ignore_ascii_case(name) {
            return Some(format!("{folder}/{found}"));
        }
    }
    None
}
