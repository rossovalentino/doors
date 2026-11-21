#![no_std]
//! Task Manager. Every value comes from the authenticated kernel snapshot or
//! the window service; providers this system lacks are absent or disabled with
//! their reason, never simulated.
extern crate alloc;
mod graph;
mod model;
mod presentation;
use alloc::{
    collections::VecDeque,
    format,
    string::{String, ToString},
    vec::Vec,
};
use door_abi::telemetry::{Process, Snapshot};
use door_app_ui::{Application, View};
use door_graphics_text::FontAtlas;
use door_service_client::{Error, Storage, window::WindowClient};
use door_service_protocol::Code;
use door_workspace::line_edit::LineEdit;
pub use model::{Group, Key, Sort};

/// Destinations in navigation order; Settings is pinned at the bottom.
pub const PROCESSES: u32 = 0;
pub const PERFORMANCE: u32 = 1;
pub const APP_HISTORY: u32 = 2;
pub const STARTUP: u32 = 3;
pub const USERS: u32 = 4;
pub const DETAILS: u32 = 5;
pub const SERVICES: u32 = 6;
pub const SETTINGS: u32 = 7;
pub const PAGE_NAMES: [&str; 8] = [
    "Processes",
    "Performance",
    "App history",
    "Startup apps",
    "Users",
    "Details",
    "Services",
    "Settings",
];
/// Real time update speed, in 100 Hz timer ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed {
    High,
    Normal,
    Low,
    Paused,
}
impl Speed {
    pub const ALL: [Self; 4] = [Self::High, Self::Normal, Self::Low, Self::Paused];
    fn interval(self) -> Option<u64> {
        match self {
            Self::High => Some(50),
            Self::Normal => Some(100),
            Self::Low => Some(400),
            Self::Paused => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::High => "High",
            Self::Normal => "Normal",
            Self::Low => "Low",
            Self::Paused => "Paused",
        }
    }
}
/// One accepted observation, placed on the graphs by its own timer tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sample {
    ticks: u64,
    /// Whole-system CPU utilization in permille over the preceding interval.
    cpu: Option<u32>,
    /// Allocated pages in permille of managed pages.
    memory: u32,
    /// Device read+write requests per second over the preceding interval.
    io: Option<u32>,
}
/// Commands that resolve to a catalog application through the window service.
pub const RUN_TARGETS: [(&str, u32); 17] = [
    ("explorer", 1),
    ("explorer.exe", 1),
    ("file explorer", 1),
    ("notes", 2),
    ("notepad", 2),
    ("notepad.exe", 2),
    ("settings", 3),
    ("ms-settings:", 3),
    ("control", 3),
    ("terminal", 9),
    ("wt", 9),
    ("wt.exe", 9),
    ("preview", 10),
    ("photos", 10),
    ("taskmgr", 11),
    ("taskmgr.exe", 11),
    ("task manager", 11),
];
pub fn run_target(input: &str) -> Option<u32> {
    let wanted = input.trim();
    RUN_TARGETS
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
        .map(|(_, command)| *command)
}
/// The "Create new task" dialog.
#[derive(Default)]
struct RunDialog {
    input: LineEdit,
    error: String,
    /// Waiting for the window service to accept the launch.
    launching: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Menu {
    View,
    StartPage,
    Speed,
}
pub struct Monitor {
    width: u32,
    height: u32,
    page: u32,
    /// Performance resource: 0 CPU, 1 Memory, 2 Disk.
    resource: u32,
    current: Option<Snapshot>,
    previous: Option<Snapshot>,
    history: VecDeque<Sample>,
    accepted: u64,
    next: u64,
    refresh_requested: bool,
    speed: Speed,
    baseline: bool,
    focused: bool,
    caret: bool,
    search: LineEdit,
    searching: bool,
    sorts: [Sort; 8],
    selected: Option<(u64, u64)>,
    error: String,
    /// User choice for the navigation pane; `None` follows the window width.
    nav_expanded: Option<bool>,
    nav_scroll: u32,
    group_by_type: bool,
    /// Optional Isolation column on Processes (always shown on Details).
    show_isolation: bool,
    users_expanded: bool,
    /// Vertical list scroll in logical pixels.
    scroll: u32,
    menu: Option<Menu>,
    dialog: Option<RunDialog>,
    pending_launch: Option<u32>,
    default_page: u32,
    /// Label widths measured with the layout atlas; shared by paint and hits.
    widths: Vec<(&'static str, bool, u32)>,
    /// Display density seen by the last paint; bounds the caption region.
    scale: core::cell::Cell<u32>,
    /// The bootstrap registry endpoint (0 in tests: no services).
    registry: u64,
    /// The Service Control Manager's services, read with each sample on
    /// the Services page.
    scm: Vec<ScmService>,
    /// The last Start/Stop/Restart failure, shown under the table.
    service_notice: String,
    /// Restart: the service to start again once it has stopped.
    restart: Option<String>,
}
/// A service the Service Control Manager knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScmService {
    pub name: String,
    pub display: String,
    pub state: u32,
    pub pid: u32,
}
/// What one Services row shows.
pub(crate) struct ServiceFields {
    pub name: String,
    pub pid: Option<u64>,
    pub description: String,
    pub status: &'static str,
    pub group: &'static str,
}
impl Default for Monitor {
    fn default() -> Self {
        Self::new()
    }
}
impl Monitor {
    pub fn new() -> Self {
        Self {
            width: 1100,
            height: 720,
            page: PROCESSES,
            resource: 0,
            current: None,
            previous: None,
            history: VecDeque::new(),
            accepted: 0,
            next: 0,
            refresh_requested: false,
            speed: Speed::Normal,
            baseline: true,
            focused: true,
            caret: true,
            search: LineEdit::default(),
            searching: false,
            sorts: [Sort::NAME; 8],
            selected: None,
            error: String::new(),
            nav_expanded: None,
            nav_scroll: 0,
            group_by_type: true,
            show_isolation: false,
            users_expanded: true,
            scroll: 0,
            menu: None,
            dialog: None,
            pending_launch: None,
            default_page: PROCESSES,
            widths: Vec::new(),
            scale: core::cell::Cell::new(1),
            registry: 0,
            scm: Vec::new(),
            service_notice: String::new(),
            restart: None,
        }
    }
    pub fn with_registry(registry: u64) -> Self {
        Self {
            registry,
            ..Self::new()
        }
    }
    /// The Services page: the SCM's services with their display names
    /// (read from the registry once per service), and a pending restart.
    fn reload_scm(&mut self, storage: &mut Storage) {
        if self.registry == 0 {
            return;
        }
        let Ok(mut scm) = door_service_client::Services::connect(self.registry) else {
            self.scm.clear();
            return;
        };
        let mut listed = Vec::new();
        let _ = scm.each(|e| listed.push((String::from(e.name()), e.state, e.pid)));
        let mut next = Vec::with_capacity(listed.len());
        for (name, state, pid) in listed {
            let display = match self.scm.iter().find(|s| s.name == name) {
                Some(old) => old.display.clone(),
                None => door_service_protocol::scm::BUILTIN
                    .iter()
                    .find(|b| b.0.eq_ignore_ascii_case(&name))
                    .map(|b| String::from(b.1))
                    .or_else(|| door_workspace::registry_browser::service_display(storage, &name))
                    .unwrap_or_else(|| name.clone()),
            };
            next.push(ScmService {
                name,
                display,
                state,
                pid,
            });
        }
        self.scm = next;
        if let Some(name) = self.restart.clone()
            && let Some(s) = self.scm.iter().find(|s| s.name == name)
            && s.state == door_service_protocol::scm::state::STOPPED
        {
            self.restart = None;
            if let Err(code) = scm.start(&name, 0) {
                self.service_notice = service_failure("start", &name, code);
            }
        }
    }
    /// The selected Services row when it is an SCM service.
    pub(crate) fn selected_scm(&self) -> Option<&ScmService> {
        match self.selected {
            Some((k, i)) if k == u64::MAX - 2 => self.scm.get(i as usize),
            _ => None,
        }
    }
    pub(crate) fn service_fields(&self, row: &Row) -> Option<ServiceFields> {
        use door_service_protocol::scm::{BUILTIN, state};
        match row {
            Row::Service(i) => {
                let s = presentation::SERVICE_CATALOG.get(*i)?;
                let pid = self.service_pid(*i);
                Some(ServiceFields {
                    name: String::from(s.name),
                    pid,
                    description: String::from(s.description),
                    status: if pid.is_some() { "Running" } else { "Stopped" },
                    group: s.group,
                })
            }
            Row::Scm(i) => {
                let s = self.scm.get(*i)?;
                Some(ServiceFields {
                    name: s.name.clone(),
                    pid: (s.pid != 0).then_some(u64::from(s.pid)),
                    description: s.display.clone(),
                    status: match s.state {
                        state::RUNNING => "Running",
                        state::PAUSED => "Paused",
                        state::STOPPED => "Stopped",
                        2 => "Starting",
                        3 => "Stopping",
                        _ => "Pending",
                    },
                    group: if BUILTIN.iter().any(|b| b.0.eq_ignore_ascii_case(&s.name)) {
                        "Doors"
                    } else {
                        ""
                    },
                })
            }
            _ => None,
        }
    }
    fn delta(old: &Snapshot, new: &Snapshot) -> Option<(u32, u64)> {
        let n = new.total_samples.checked_sub(old.total_samples)?;
        if n == 0 || new.ticks <= old.ticks {
            return None;
        }
        let busy = new.busy_samples.checked_sub(old.busy_samples)?;
        if busy > n {
            return None;
        }
        let a = new
            .reads
            .checked_add(new.writes)?
            .checked_sub(old.reads.checked_add(old.writes)?)?;
        Some((
            ((busy as u128 * 1000 / n as u128) as u64).min(1000) as u32,
            (a as u128 * 100 / (new.ticks - old.ticks) as u128).min(u64::MAX as u128) as u64,
        ))
    }
    fn accept(&mut self, s: Snapshot) {
        self.previous = if self.baseline { None } else { self.current };
        self.baseline = false;
        let delta = self.previous.as_ref().and_then(|p| Self::delta(p, &s));
        let sample = Sample {
            ticks: s.ticks,
            cpu: delta.map(|d| d.0),
            memory: ((s.allocated_pages as u128 * 1000 / s.managed_pages.max(1) as u128) as u64)
                .min(1000) as u32,
            io: delta.map(|d| d.1.min(u32::MAX as u64) as u32),
        };
        // Keep a little more than the visible minute so the left edge is drawn.
        while self.history.len() >= 512
            || self
                .history
                .front()
                .is_some_and(|f| f.ticks + 6400 < s.ticks || f.ticks > s.ticks)
        {
            self.history.pop_front();
        }
        self.history.push_back(sample);
        self.accepted += 1;
        self.current = Some(s);
        self.error.clear();
        if let Some(sel) = self.selected
            && !s.processes[..s.count]
                .iter()
                .any(|p| (p.pid, p.instance) == sel)
        {
            self.selected = None;
        }
        self.clamp_scroll();
    }
    /// Accept one telemetry snapshot, as `tick` does with the sampled one
    /// (host reviews drive pages with fixed snapshots).
    #[doc(hidden)]
    pub fn ingest(&mut self, s: Snapshot) {
        self.accept(s);
    }
    /// Apply a hit-test target, as a click does (host reviews).
    #[doc(hidden)]
    pub fn press(&mut self, id: u32) -> bool {
        self.activate(id)
    }
    pub fn set_speed(&mut self, speed: Speed) {
        let was = self.speed == Speed::Paused;
        self.speed = speed;
        let paused = speed == Speed::Paused;
        if was != paused {
            if !paused {
                // A resumed interval would span the pause: start a new baseline.
                self.previous = None;
                self.baseline = true;
                self.next = 0;
            }
            door_rt::diagnostic(format_args!("DOOR_MONITOR PAUSED value={}\n", paused as u8));
        } else if !paused {
            self.next = 0;
        }
    }
    /// Whole-system CPU of one process over the latest interval, in permille.
    fn cpu(&self, p: &Process) -> Option<u32> {
        let old = self.previous.as_ref()?;
        let now = self.current.as_ref()?;
        let n = now.total_samples.checked_sub(old.total_samples)?;
        let previous = old.processes[..old.count]
            .iter()
            .find(|q| q.pid == p.pid && q.instance == p.instance)?;
        let samples = p.samples.checked_sub(previous.samples)?;
        (n != 0 && samples <= n).then(|| ((samples as u128 * 1000 / n as u128) as u64) as u32)
    }
    fn processes(&self) -> Vec<Process> {
        self.current
            .as_ref()
            .map_or(Vec::new(), |s| s.processes[..s.count].to_vec())
    }
    fn matches(&self, p: &Process) -> bool {
        let query = self.search.text().trim().to_ascii_lowercase();
        if query.is_empty() {
            return true;
        }
        let id = model::identity(p);
        id.name.to_ascii_lowercase().contains(&query)
            || id.image.contains(&query)
            || p.pid.to_string() == query
    }
    fn compare(&self, sort: Sort, a: &Process, b: &Process) -> core::cmp::Ordering {
        use core::cmp::Ordering;
        let (ia, ib) = (model::identity(a), model::identity(b));
        let primary = match sort.key {
            Key::Name if self.page == DETAILS => ia.image.cmp(ib.image),
            Key::Name => ia.name.cmp(ib.name),
            Key::Description => ia.name.cmp(ib.name),
            Key::Status => model::status(a).cmp(model::status(b)),
            Key::Cpu => self.cpu(a).cmp(&self.cpu(b)),
            Key::Memory => a.private_bytes.cmp(&b.private_bytes),
            Key::Pid => a.pid.cmp(&b.pid),
            Key::User => model::user_name(a.uid).cmp(model::user_name(b.uid)),
            Key::Threads => a.threads.cmp(&b.threads),
            Key::CpuTime => a.samples.cmp(&b.samples),
            Key::Isolation => model::isolation(a).cmp(model::isolation(b)),
        };
        let primary = if sort.descending {
            primary.reverse()
        } else {
            primary
        };
        if primary != Ordering::Equal {
            return primary;
        }
        // Stable secondary order: image name for Details, else friendly name, then PID.
        let secondary = if sort.key == Key::Pid {
            Ordering::Equal
        } else if self.page == DETAILS {
            ia.image.cmp(ib.image)
        } else {
            ia.name.cmp(ib.name)
        };
        secondary.then(a.pid.cmp(&b.pid))
    }
    fn sorted(&self, mut rows: Vec<Process>) -> Vec<Process> {
        let sort = self.sorts[self.page as usize];
        rows.sort_by(|a, b| self.compare(sort, a, b));
        rows
    }
    /// Processes page rows, optionally grouped by type.
    fn process_rows(&self) -> Vec<Row> {
        let rows = self.sorted(
            self.processes()
                .into_iter()
                .filter(|p| self.matches(p))
                .collect(),
        );
        if !self.group_by_type {
            // Grouping off: one category holding every process (Windows 26H2).
            if rows.is_empty() {
                return Vec::new();
            }
            let mut out = alloc::vec![Row::Group(Group::All, rows.len())];
            out.extend(rows.into_iter().map(Row::Process));
            return out;
        }
        let mut out = Vec::new();
        for group in [Group::Apps, Group::Background, Group::Windows] {
            let members: Vec<_> = rows
                .iter()
                .filter(|p| model::identity(p).group == group)
                .copied()
                .collect();
            if !members.is_empty() {
                out.push(Row::Group(group, members.len()));
                out.extend(members.into_iter().map(Row::Process));
            }
        }
        out
    }
    fn user_rows(&self) -> Vec<Row> {
        let mut uids: Vec<u32> = self
            .processes()
            .iter()
            .filter(|p| !model::system_account(p.uid))
            .map(|p| p.uid)
            .collect();
        uids.sort_unstable();
        uids.dedup();
        let mut out = Vec::new();
        for uid in uids {
            let members = self.sorted(
                self.processes()
                    .into_iter()
                    .filter(|p| p.uid == uid && self.matches(p))
                    .collect(),
            );
            out.push(Row::User(uid, members.len()));
            if self.users_expanded {
                out.extend(members.into_iter().map(Row::Process));
            }
        }
        out
    }
    fn history_rows(&self) -> Vec<Row> {
        self.sorted(
            self.processes()
                .into_iter()
                .filter(|p| model::identity(p).group == Group::Apps && self.matches(p))
                .collect(),
        )
        .into_iter()
        .map(Row::Process)
        .collect()
    }
    fn detail_rows(&self) -> Vec<Row> {
        self.sorted(
            self.processes()
                .into_iter()
                .filter(|p| self.matches(p))
                .collect(),
        )
        .into_iter()
        .map(Row::Process)
        .collect()
    }
    fn service_rows(&self) -> Vec<Row> {
        let query = self.search.text().trim().to_ascii_lowercase();
        let mut rows: Vec<(Row, ServiceFields)> = (0..presentation::SERVICE_CATALOG.len())
            .map(Row::Service)
            .chain((0..self.scm.len()).map(Row::Scm))
            .filter_map(|r| Some((r, self.service_fields(&r)?)))
            .filter(|(_, f)| {
                query.is_empty()
                    || f.name.to_ascii_lowercase().contains(&query)
                    || f.description.to_ascii_lowercase().contains(&query)
                    || f.pid.is_some_and(|p| p.to_string() == query)
            })
            .collect();
        let sort = self.sorts[SERVICES as usize];
        rows.sort_by(|(_, a), (_, b)| {
            let order = match sort.key {
                Key::Pid => a.pid.cmp(&b.pid),
                Key::Description => a.description.cmp(&b.description),
                Key::Status => a.status.cmp(b.status),
                Key::User => a.group.cmp(b.group),
                _ => a
                    .name
                    .to_ascii_lowercase()
                    .cmp(&b.name.to_ascii_lowercase()),
            };
            if sort.descending {
                order.reverse()
            } else {
                order
            }
        });
        rows.into_iter().map(|(r, _)| r).collect()
    }
    fn service_pid(&self, index: usize) -> Option<u64> {
        let manifest = presentation::SERVICE_CATALOG[index].manifest;
        self.processes()
            .iter()
            .find(|p| p.manifest as u64 == manifest && model::system_account(p.uid))
            .map(|p| p.pid)
    }
    fn rows(&self) -> Vec<Row> {
        match self.page {
            PROCESSES => self.process_rows(),
            APP_HISTORY => self.history_rows(),
            USERS => self.user_rows(),
            DETAILS => self.detail_rows(),
            SERVICES => self.service_rows(),
            _ => Vec::new(),
        }
    }
    fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.min(self.max_scroll());
    }
    fn sample_due(&mut self, now: u64) -> bool {
        if self.refresh_requested {
            self.refresh_requested = false;
            self.next = now.saturating_add(self.speed.interval().unwrap_or(100));
            return true;
        }
        let Some(interval) = self.speed.interval() else {
            return false;
        };
        if now < self.next {
            return false;
        }
        self.next = now.saturating_add(interval);
        true
    }
    fn navigate(&mut self, page: u32) {
        if page != self.page {
            self.scroll = 0;
            self.menu = None;
        }
        self.page = page;
        if !matches!(page, PROCESSES | APP_HISTORY | USERS | DETAILS | SERVICES) {
            self.searching = false;
        }
        door_rt::diagnostic(format_args!("DOOR_MONITOR PAGE id={page}\n"));
    }
    fn select_resource(&mut self, resource: u32) {
        self.resource = resource.min(2);
        door_rt::diagnostic(format_args!(
            "DOOR_MONITOR PAGE id={}\n",
            20 + self.resource
        ));
    }
    /// Run new task: resolve a catalog app or explain why nothing will open.
    fn submit_run(&mut self) {
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        if dialog.launching {
            return;
        }
        let text = String::from(dialog.input.text().trim());
        if text.is_empty() {
            dialog.error = "Type the name of an app to open.".into();
            return;
        }
        match run_target(&text) {
            Some(command) => {
                dialog.error.clear();
                dialog.launching = true;
                self.pending_launch = Some(command);
            }
            None => {
                dialog.error = format!(
                    "Doors cannot find '{}'. Make sure you typed the name correctly, and then try again.",
                    door_app_ui::ui::display_ascii(&text)
                );
            }
        }
    }
    fn open_run_dialog(&mut self) {
        self.menu = None;
        self.searching = false;
        let mut dialog = RunDialog::default();
        dialog.input.set(String::new(), false);
        self.dialog = Some(dialog);
    }
    fn move_selection(&mut self, forward: bool) {
        let rows = self.rows();
        let candidates: Vec<(usize, (u64, u64))> = rows
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.key().map(|k| (i, k)))
            .collect();
        if candidates.is_empty() {
            return;
        }
        let at = self
            .selected
            .and_then(|s| candidates.iter().position(|(_, k)| *k == s));
        let next = match at {
            None => 0,
            Some(i) if forward => (i + 1).min(candidates.len() - 1),
            Some(i) => i.saturating_sub(1),
        };
        self.selected = Some(candidates[next].1);
        self.reveal_row(candidates[next].0);
    }
}
/// One painted table row. Process rows carry their kernel identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Group(Group, usize),
    User(u32, usize),
    Process(Process),
    Service(usize),
    /// A service of the Service Control Manager (index into `scm`).
    Scm(usize),
}
impl Row {
    fn key(&self) -> Option<(u64, u64)> {
        match self {
            Row::Process(p) => Some((p.pid, p.instance)),
            Row::Service(i) => Some((u64::MAX, *i as u64)),
            Row::Scm(i) => Some((u64::MAX - 2, *i as u64)),
            Row::User(uid, _) => Some((u64::MAX - 1, *uid as u64)),
            Row::Group(..) => None,
        }
    }
}
fn edit(line: &mut LineEdit, key: u32, limit: usize) -> bool {
    let scan = key & 511;
    let control = (key >> 9) & door_abi::graphics::CONTROL != 0;
    let shift = (key >> 9) & door_abi::graphics::SHIFT != 0;
    match scan {
        14 => line.backspace(),
        0x153 => line.delete(),
        30 if control => line.select_all(),
        0x14b => line.horizontal(false, shift, control),
        0x14d => line.horizontal(true, shift, control),
        0x147 => line.place(0, shift),
        0x14f => line.place(line.text().len(), shift),
        _ => {
            let ch = (key >> 16) as u8;
            if control || !(32..=126).contains(&ch) {
                return false;
            }
            line.insert_bounded(ch as char, limit);
        }
    }
    true
}
impl Application for Monitor {
    fn new_window(&self, _s: &mut Storage) -> Result<Option<Self>, Error> {
        Ok(Some(self.spawn()))
    }
    fn title(&self) -> &str {
        "Task Manager"
    }
    fn options(&self) -> door_workspace::runtime::WindowOptions {
        let (left, _) = presentation::caption_region(1100, self.scale.get());
        door_workspace::runtime::WindowOptions {
            width: 1100,
            height: 720,
            caption_height: presentation::CAPTION as u32,
            caption_left: left,
            centered: false,
        }
    }
    fn caption(&self) -> (u32, u32) {
        presentation::caption_region(self.width, self.scale.get())
    }
    fn configure(&mut self, w: u32, h: u32) {
        self.width = w;
        self.height = h;
        self.clamp_scroll();
        self.clamp_nav();
    }
    fn configure_fonts(&mut self, w: u32, h: u32, fonts: &FontAtlas) {
        self.measure(fonts);
        self.configure(w, h);
    }
    fn semantics(&self, s: &mut door_workspace::semantics::Semantics, _f: &FontAtlas) {
        self.describe(s)
    }
    fn target(&self, x: i32, y: i32) -> Option<u32> {
        self.hits()
            .into_iter()
            .rev()
            .find(|(_, r)| r.hit(x, y))
            .map(|(id, _)| id)
    }
    fn click(&mut self, id: u32, _storage: &mut Storage) -> Result<bool, Error> {
        Ok(self.activate(id))
    }
    fn key(&mut self, key: u32, _storage: &mut Storage) -> Result<bool, Error> {
        Ok(self.handle_key(key))
    }
    fn focus(&mut self, focused: bool) {
        self.focused = focused;
    }
    fn modal(&self) -> bool {
        self.dialog.is_some()
    }
    fn menu_open(&self) -> bool {
        self.menu.is_some()
    }
    fn dismiss_menu(&mut self) {
        self.menu = None;
    }
    fn text_hit(&self, x: i32, y: i32) -> bool {
        self.text_field_at(x, y)
    }
    fn wheel(&mut self, x: i32, y: i32, notches: i32, _f: &FontAtlas) -> bool {
        self.scroll_at(x, y, notches)
    }
    fn tick(&mut self, storage: &mut Storage, now: u64) -> Result<bool, Error> {
        let editing = self.searching || self.dialog.is_some();
        let next = self.focused && (now / 50).is_multiple_of(2);
        let changed = editing && next != self.caret;
        self.caret = next;
        if !self.sample_due(now) {
            return Ok(changed);
        }
        match door_sys::system_snapshot() {
            Ok(snapshot) => {
                self.accept(snapshot);
                if matches!(self.accepted, 2 | 6) {
                    let s = self.current.as_ref().unwrap();
                    door_rt::diagnostic(format_args!(
                        "DOOR_MONITOR {} processes={} managed={} allocated={} observations={} busy={}\n",
                        if self.accepted == 2 {
                            "SAMPLE"
                        } else {
                            "SETTLED"
                        },
                        s.count,
                        s.managed_pages,
                        s.allocated_pages,
                        s.total_samples,
                        s.busy_samples
                    ));
                }
            }
            Err(e) => self.error = format!("System measurements unavailable: {e:?}"),
        }
        if self.page == SERVICES || self.restart.is_some() {
            self.reload_scm(storage);
        }
        Ok(true)
    }
    fn update(
        &mut self,
        _s: &mut Storage,
        c: &mut WindowClient,
        _window: u64,
    ) -> Result<bool, Error> {
        let Some(command) = self.pending_launch.take() else {
            return Ok(false);
        };
        // A new task is a new window when the app is already running; apps
        // without multi-window support are activated instead.
        let result = match c.launch_window(command) {
            Err(Error::Service(Code::Unsupported)) => c.launch_app(command),
            other => other,
        };
        door_rt::diagnostic(format_args!(
            "DOOR_MONITOR LAUNCH app={command} ok={}\n",
            result.is_ok() as u8
        ));
        match result {
            Ok(()) => self.dialog = None,
            Err(e) => {
                if let Some(d) = self.dialog.as_mut() {
                    d.launching = false;
                    d.error = match e {
                        Error::Service(Code::Closing) => {
                            "The desktop is shutting down. The task was not started.".into()
                        }
                        Error::Service(Code::WouldBlock) => {
                            "That app cannot open another window right now. Try again.".into()
                        }
                        e => format!("The window service did not start the task: {e:?}"),
                    };
                }
            }
        }
        Ok(true)
    }
    fn draw(&self, v: &mut View<'_>) {
        self.scale.set(v.scale);
        self.paint(v);
    }
}
impl Monitor {
    /// Keyboard input; Task Manager keys never need storage.
    pub(crate) fn handle_key(&mut self, key: u32) -> bool {
        let scan = key & 511;
        let control = (key >> 9) & door_abi::graphics::CONTROL != 0;
        let shift = (key >> 9) & door_abi::graphics::SHIFT != 0;
        if let Some(dialog) = self.dialog.as_mut() {
            match scan {
                1 => self.dialog = None,
                28 | 0x11c => self.submit_run(),
                _ => {
                    if !dialog.launching && edit(&mut dialog.input, key, 192) {
                        dialog.error.clear();
                    }
                }
            }
            return true;
        }
        if self.menu.is_some() && scan == 1 {
            self.menu = None;
            return true;
        }
        if control && scan == 33 && self.search().is_some() {
            // Ctrl+F: focus the title-bar search box.
            self.menu = None;
            self.searching = true;
            self.search.select_all();
            return true;
        }
        if control && scan == 15 {
            let next = if shift {
                (self.page + 7) % 8
            } else {
                (self.page + 1) % 8
            };
            self.navigate(next);
            return true;
        }
        if control && scan == 49 && !shift {
            self.open_run_dialog();
            return true;
        }
        if scan == 63 {
            self.refresh_requested = true;
            self.next = 0;
            return true;
        }
        if self.searching {
            match scan {
                1 => {
                    if self.search.text().is_empty() {
                        self.searching = false;
                    } else {
                        self.search.set(String::new(), false);
                    }
                }
                28 | 0x11c | 0x150 => self.searching = false,
                _ => {
                    if !edit(&mut self.search, key, 64) {
                        return false;
                    }
                    if matches!(self.page, PERFORMANCE | STARTUP | SETTINGS) {
                        self.navigate(PROCESSES);
                    }
                }
            }
            self.scroll = 0;
            self.clamp_scroll();
            return true;
        }
        match scan {
            1 => {
                self.selected = None;
                true
            }
            0x148 | 0x150 if self.page == PERFORMANCE => {
                self.select_resource(if scan == 0x150 {
                    (self.resource + 1).min(2)
                } else {
                    self.resource.saturating_sub(1)
                });
                true
            }
            0x148 | 0x150 => {
                self.move_selection(scan == 0x150);
                true
            }
            0x149 | 0x151 => {
                let page = self.list_viewport().h.saturating_sub(32);
                self.scroll = if scan == 0x151 {
                    self.scroll.saturating_add(page)
                } else {
                    self.scroll.saturating_sub(page)
                };
                self.clamp_scroll();
                true
            }
            0x147 => {
                self.scroll = 0;
                true
            }
            0x14f => {
                self.scroll = self.max_scroll();
                true
            }
            _ => false,
        }
    }
    /// A new Task Manager window inherits this window's preferences.
    fn spawn(&self) -> Self {
        let mut next = Self::new();
        next.speed = self.speed;
        next.group_by_type = self.group_by_type;
        next.show_isolation = self.show_isolation;
        next.default_page = self.default_page;
        next.page = self.default_page;
        next.nav_expanded = self.nav_expanded;
        next.registry = self.registry;
        next
    }
}
#[cfg(test)]
mod tests;
/// A Start/Stop failure as the notice under the Services table.
fn service_failure(verb: &str, name: &str, code: u32) -> String {
    match code {
        5 => format!(
            "Couldn't {verb} {name}: Access is denied. Open Task Manager as administrator to change services."
        ),
        1056 => format!("Couldn't {verb} {name}: An instance of the service is already running."),
        1062 => format!("Couldn't {verb} {name}: The service has not been started."),
        1063 | 1722 => {
            format!("Couldn't {verb} {name}: The Service Control Manager isn't available.")
        }
        _ => format!("Couldn't {verb} {name}: error {code}."),
    }
}
