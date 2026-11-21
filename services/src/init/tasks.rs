//! The Task Scheduler (`door_service_protocol::tasks`): Init runs each
//! task's program at its time, as its creator, with the session user's
//! working-folder authority the desktop shell handed over at sign-in.
//! Tasks persist in `/state/tasks.db`: `DOORTSK1`, then `TASKS` slots of
//! `SLOT` bytes `[used u8][0][length u16][uid u32][last run u64][last
//! result u32][0 u32][task]`. A task registered through taskschd.dll also
//! keeps its XML definition in `/state/tasks/<slot>.xml`; its Exec action's
//! WorkingDirectory is the folder the program starts in.
use super::log as trace;
use crate::server::Response;
use door_abi::{Handle, platform::Identity};
use door_service_client::{Disposition, READ, Storage, WRITE};
use door_service_protocol::{
    Code,
    tasks::{self as wire, Schedule, Status, Task},
};

const PATH: &str = "/state/tasks.db";
const MAGIC: &[u8; 8] = b"DOORTSK1";
const SLOT: usize = 256;
const XML_DIRECTORY: &str = "/state/tasks";
/// `/state/tasks/<slot>.xml`.
fn xml_path(slot: usize, out: &mut [u8; 32]) -> &str {
    let prefix = b"/state/tasks/";
    out[..prefix.len()].copy_from_slice(prefix);
    let mut at = prefix.len();
    if slot >= 10 {
        out[at] = b'0' + (slot / 10) as u8;
        at += 1;
    }
    out[at] = b'0' + (slot % 10) as u8;
    out[at + 1..at + 5].copy_from_slice(b".xml");
    core::str::from_utf8(&out[..at + 5]).unwrap_or("")
}
/// The text of the first `<WorkingDirectory>` in `xml` (entities decoded
/// into `out`).
fn working_directory<'a>(xml: &[u8], out: &'a mut [u8; 256]) -> Option<&'a str> {
    const OPEN: &[u8] = b"<WorkingDirectory>";
    let start = xml.windows(OPEN.len()).position(|w| w == OPEN)? + OPEN.len();
    let end = start + xml[start..].iter().position(|&b| b == b'<')?;
    let mut n = 0;
    let mut i = start;
    while i < end {
        let (byte, used) = match &xml[i..end] {
            [b'&', b'a', b'm', b'p', b';', ..] => (b'&', 5),
            [b'&', b'l', b't', b';', ..] => (b'<', 4),
            [b'&', b'g', b't', b';', ..] => (b'>', 4),
            [b'&', b'q', b'u', b'o', b't', b';', ..] => (b'"', 6),
            [b'&', b'a', b'p', b'o', b's', b';', ..] => (b'\'', 6),
            [b, ..] => (*b, 1),
            [] => break,
        };
        *out.get_mut(n)? = byte;
        n += 1;
        i += used;
    }
    core::str::from_utf8(&out[..n])
        .ok()
        .filter(|s| !s.is_empty())
}
/// SCHED_S_TASK_HAS_NOT_RUN, what schtasks shows as "Last Result" first.
pub(super) const NOT_RUN: u32 = 0x0004_1303;

#[derive(Clone, Copy)]
pub(super) struct Entry {
    task: Task,
    uid: u32,
    last_run: u64,
    last_result: u32,
    next_run: u64,
    running: Option<u64>,
}
#[derive(Default)]
pub(super) struct Scheduler {
    entries: [Option<Entry>; wire::TASKS],
    loaded: bool,
    /// OnLogon tasks already run this session.
    logon_done: bool,
}
fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}
impl Scheduler {
    fn find(&self, name: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.is_some_and(|e| same(e.task.name(), name)))
    }
    /// QUERY (body: index u16): the `index`th task `owned` lets the caller
    /// see (each user their own, administrators all).
    fn query(&self, body: &[u8], owned: impl Fn(&Entry) -> bool) -> Response {
        let index = usize::from(u16::from_le_bytes([body[0], body[1]]));
        let mut visible = self.entries.iter().flatten().filter(|e| owned(e));
        match visible.nth(index) {
            Some(e) => {
                let mut out = [0u8; wire::STATUS_HEADER + wire::TASK_BYTES];
                let n = self.status(e).encode(&mut out);
                Response::bytes(&out[..n])
            }
            None => Response::error(Code::NotFound),
        }
    }
    fn load(&mut self, storage: &mut Storage, now: u64) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let Ok(file) = storage.open(PATH, Disposition::OpenExisting, READ, READ) else {
            return;
        };
        let mut magic = [0u8; 8];
        if storage.read(&file, 0, &mut magic).ok() != Some(8) || &magic != MAGIC {
            let _ = storage.close(file);
            return;
        }
        for i in 0..wire::TASKS {
            let mut b = [0u8; SLOT];
            if storage.read_full(&file, 8 + (i * SLOT) as u64, &mut b).ok() != Some(SLOT)
                || b[0] != 1
            {
                continue;
            }
            let len = usize::from(u16::from_le_bytes([b[2], b[3]]));
            let Some(task) = b.get(24..24 + len).and_then(Task::decode) else {
                continue;
            };
            let w = |at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
            self.entries[i] = Some(Entry {
                task,
                uid: w(4),
                last_run: u64::from_le_bytes(b[8..16].try_into().unwrap()),
                last_result: w(16),
                next_run: task.schedule.next(task.start, now).unwrap_or(0),
                running: None,
            });
        }
        let _ = storage.close(file);
    }
    /// Writes slot `i` (and the file's magic).
    fn save(&self, storage: &mut Storage, i: usize) -> Result<(), Code> {
        let code = |e: door_service_client::Error| match e {
            door_service_client::Error::Service(c) => c,
            _ => Code::Io,
        };
        let file = match storage.open(PATH, Disposition::OpenExisting, READ | WRITE, 0) {
            Ok(f) => f,
            Err(door_service_client::Error::Service(Code::NotFound)) => {
                let f = storage
                    .open(PATH, Disposition::CreateNew, READ | WRITE, 0)
                    .map_err(code)?;
                let _ = storage.write_all(&f, 0, MAGIC);
                f
            }
            Err(e) => return Err(code(e)),
        };
        let mut b = [0u8; SLOT];
        if let Some(e) = self.entries[i] {
            let mut task = [0u8; wire::TASK_BYTES];
            let n = e.task.encode(&mut task);
            b[0] = 1;
            b[2..4].copy_from_slice(&(n as u16).to_le_bytes());
            b[4..8].copy_from_slice(&e.uid.to_le_bytes());
            b[8..16].copy_from_slice(&e.last_run.to_le_bytes());
            b[16..20].copy_from_slice(&e.last_result.to_le_bytes());
            b[24..24 + n].copy_from_slice(&task[..n]);
        }
        let written = storage.write_all(&file, 8 + (i * SLOT) as u64, &b);
        let _ = storage.close(file);
        written.map_err(code)?;
        let _ = storage.flush();
        Ok(())
    }
    fn drop_xml(storage: &mut Storage, slot: usize) {
        let mut path = [0u8; 32];
        let _ = storage.remove(xml_path(slot, &mut path));
    }
    fn status(&self, e: &Entry) -> Status {
        Status {
            task: e.task,
            state: if e.running.is_some() {
                Status::RUNNING
            } else if e.task.enabled {
                Status::READY
            } else {
                Status::DISABLED
            },
            last_result: e.last_result,
            next_run: if e.task.enabled { e.next_run } else { 0 },
            last_run: e.last_run,
            uid: e.uid,
        }
    }
}

/// The program and its arguments from a /TR command line: (program,
/// NUL-separated arguments with the program first).
pub(super) fn split(command: &str, out: &mut [u8; 256]) -> Option<(usize, usize)> {
    let mut at = 0;
    let mut first_end = 0;
    let mut chars = command.trim().chars().peekable();
    let mut index = 0;
    while chars.peek().is_some() {
        while chars.peek() == Some(&' ') {
            chars.next();
        }
        let quoted = chars.peek() == Some(&'"');
        if quoted {
            chars.next();
        }
        let start = at;
        for ch in chars.by_ref() {
            if (quoted && ch == '"') || (!quoted && ch == ' ') {
                break;
            }
            let mut b = [0u8; 4];
            let s = ch.encode_utf8(&mut b);
            if at + s.len() + 1 > out.len() {
                return None;
            }
            out[at..at + s.len()].copy_from_slice(s.as_bytes());
            at += s.len();
        }
        if at > start || quoted {
            out[at] = 0;
            at += 1;
            if index == 0 {
                first_end = at - 1;
            }
            index += 1;
        }
    }
    (index > 0).then_some((first_end, at))
}

impl super::Registry {
    /// The desktop shell's working-folder authority (sign-in): tasks and
    /// services launch with it.
    pub(super) fn bind_directory(&mut self, directory: Handle) {
        if self.session_directory != 0 {
            let _ = door_sys::close(self.session_directory);
        }
        self.session_directory = directory;
    }
    /// Due tasks of the signed-in user, and OnLogon tasks once per session.
    pub(super) fn tasks_tick(&mut self, uid: u32) {
        if self.session == 0 || self.session_directory == 0 {
            return;
        }
        let now = door_sys::wall_time().unwrap_or(0);
        // Storage only to load the store once and to run a due task, not
        // on every tick.
        let mut storage = None;
        if !self.tasks.loaded {
            let Ok(mut s) = Storage::connect_port(self.ports[0]) else {
                return;
            };
            self.tasks.load(&mut s, now);
            storage = Some(s);
        }
        let logon = !self.tasks.logon_done;
        self.tasks.logon_done = true;
        for i in 0..wire::TASKS {
            let Some(e) = self.tasks.entries[i] else {
                continue;
            };
            if e.uid != uid || !e.task.enabled || e.running.is_some() {
                continue;
            }
            let due = match e.task.schedule {
                Schedule::OnLogon => logon,
                _ => e.next_run != 0 && e.next_run <= now,
            };
            if !due {
                continue;
            }
            if storage.is_none() {
                storage = Storage::connect_port(self.ports[0]).ok();
            }
            if let Some(s) = storage.as_mut() {
                let _ = self.task_run(i, s, now);
            }
        }
    }
    fn task_run(&mut self, i: usize, storage: &mut Storage, now: u64) -> Result<(), Code> {
        let mut e = self.tasks.entries[i].ok_or(Code::NotFound)?;
        let mut arguments = [0u8; 256];
        let (program_end, len) =
            split(e.task.command(), &mut arguments).ok_or(Code::InvalidRequest)?;
        let program =
            core::str::from_utf8(&arguments[..program_end]).map_err(|_| Code::InvalidRequest)?;
        let path = super::scm::native(program).ok_or(Code::NotFound)?;
        let mut folder = &path.as_str()[..path.as_str().rfind('/').unwrap_or(0).max(1)];
        // The XML's Exec WorkingDirectory, when it has one.
        let mut name = [0u8; 32];
        let mut xml = [0u8; 1024];
        let mut directory = [0u8; 256];
        let custom = storage
            .open(
                xml_path(i, &mut name),
                Disposition::OpenExisting,
                READ,
                READ,
            )
            .ok()
            .and_then(|file| {
                let n = storage.read(&file, 0, &mut xml).ok();
                let _ = storage.close(file);
                n
            })
            .and_then(|n| working_directory(&xml[..n], &mut directory))
            .and_then(super::scm::native);
        if let Some(d) = custom.as_ref() {
            folder = d.as_str();
        }
        let request = door_service_protocol::application::Execute {
            path: path.as_str(),
            cwd: folder,
            arguments: &arguments[..len],
            staged: false,
            options: door_service_protocol::application::ExecuteOptions::NONE,
        };
        self.last_launched = None;
        let directory = self.session_directory;
        let _ = self.execute(0, e.uid, true, &request, directory, None, false);
        e.last_run = now;
        e.next_run = e.task.schedule.next(e.task.start, now + 1).unwrap_or(0);
        match self.last_launched.take() {
            Some(pid) => {
                e.running = Some(pid);
                trace(format_args!(
                    "DOOR_P16 TASK run name={} pid={pid}",
                    e.task.name()
                ));
            }
            // ERROR_FILE_NOT_FOUND as schtasks shows it.
            None => e.last_result = 0x8007_0002,
        }
        self.tasks.entries[i] = Some(e);
        self.tasks.save(storage, i)
    }
    /// A process ended: a task's run is over (its exit code is the task's
    /// Last Result).
    pub(super) fn task_exited(&mut self, pid: u64, code: u32) {
        let Some(i) = self
            .tasks
            .entries
            .iter()
            .position(|e| e.is_some_and(|e| e.running == Some(pid)))
        else {
            return;
        };
        if let Some(e) = self.tasks.entries[i].as_mut() {
            e.running = None;
            e.last_result = code;
            trace(format_args!(
                "DOOR_P16 TASK exit name={} code={code}",
                e.task.name()
            ));
        }
        if let Ok(mut storage) = Storage::connect_port(self.ports[0]) {
            let _ = self.tasks.save(&mut storage, i);
        }
    }
    /// Every Task Scheduler request.
    pub(super) fn tasks_request(
        &mut self,
        peer: Identity,
        operation: u32,
        body: &[u8],
    ) -> Response {
        if !self.admits(peer) {
            return Response::error(Code::Denied);
        }
        let admin = peer.groups & door_abi::platform::group::ADMINISTRATORS != 0;
        let now = door_sys::wall_time().unwrap_or(0);
        let owned = |e: &Entry| admin || e.uid == peer.uid;
        // A status query reads only what Init keeps: no storage connection
        // once the store is loaded (Task Scheduler clients poll it).
        if operation == wire::QUERY && body.len() == 2 && self.tasks.loaded {
            return self.tasks.query(body, owned);
        }
        let Ok(mut storage) = Storage::connect_port(self.ports[0]) else {
            return Response::error(Code::Io);
        };
        self.tasks.load(&mut storage, now);
        fn name(b: &[u8]) -> Option<&str> {
            core::str::from_utf8(b).ok().filter(|n| wire::valid_name(n))
        }
        match operation {
            wire::CREATE => {
                let Some(task) = Task::decode(body) else {
                    return Response::error(Code::InvalidRequest);
                };
                let slot = match self.tasks.find(task.name()) {
                    Some(_) if !task.replace => return Response::error(Code::Exists),
                    Some(i) if !owned(&self.tasks.entries[i].unwrap()) => {
                        return Response::error(Code::Denied);
                    }
                    Some(i) => i,
                    None => match self.tasks.entries.iter().position(Option::is_none) {
                        Some(i) => i,
                        None => return Response::error(Code::Resources),
                    },
                };
                let mut task = task;
                task.replace = false;
                // Its definition comes again (XML_PUT) if it has one.
                Scheduler::drop_xml(&mut storage, slot);
                self.tasks.entries[slot] = Some(Entry {
                    task,
                    uid: peer.uid,
                    last_run: 0,
                    last_result: NOT_RUN,
                    next_run: task.schedule.next(task.start, now).unwrap_or(0),
                    running: None,
                });
                trace(format_args!("DOOR_P16 TASK created name={}", task.name()));
                match self.tasks.save(&mut storage, slot) {
                    Ok(()) => Response::empty(),
                    Err(c) => Response::error(c),
                }
            }
            wire::QUERY if body.len() == 2 => self.tasks.query(body, owned),
            wire::DELETE | wire::RUN | wire::END => {
                let Some(n) = name(body) else {
                    return Response::error(Code::InvalidRequest);
                };
                let Some(i) = self.tasks.find(n) else {
                    return Response::error(Code::NotFound);
                };
                let e = self.tasks.entries[i].unwrap();
                if !owned(&e) {
                    return Response::error(Code::Denied);
                }
                match operation {
                    wire::DELETE => {
                        if let Some(pid) = e.running
                            && let Some(p) = self.job_process(pid)
                        {
                            let _ = door_sys::process_terminate(p, -1);
                        }
                        self.tasks.entries[i] = None;
                        Scheduler::drop_xml(&mut storage, i);
                        match self.tasks.save(&mut storage, i) {
                            Ok(()) => Response::empty(),
                            Err(c) => Response::error(c),
                        }
                    }
                    wire::RUN => {
                        if e.uid != self.session_uid || self.session_directory == 0 {
                            return Response::error(Code::Denied);
                        }
                        if e.running.is_some() {
                            return Response::empty();
                        }
                        match self.task_run(i, &mut storage, now) {
                            Ok(()) => Response::empty(),
                            Err(c) => Response::error(c),
                        }
                    }
                    _ => {
                        let Some(pid) = e.running else {
                            return Response::error(Code::NotFound);
                        };
                        if let Some(p) = self.job_process(pid) {
                            let _ = door_sys::process_terminate(p, 0x8007_013a_u32 as i32 as i64);
                        }
                        Response::empty()
                    }
                }
            }
            wire::CHANGE if body.len() > 1 && body[0] <= 1 => {
                let Some(n) = name(&body[1..]) else {
                    return Response::error(Code::InvalidRequest);
                };
                let Some(i) = self.tasks.find(n) else {
                    return Response::error(Code::NotFound);
                };
                let Some(e) = self.tasks.entries[i].as_mut() else {
                    return Response::error(Code::NotFound);
                };
                if !(admin || e.uid == peer.uid) {
                    return Response::error(Code::Denied);
                }
                e.task.enabled = body[0] == 1;
                e.next_run = e.task.schedule.next(e.task.start, now).unwrap_or(0);
                match self.tasks.save(&mut storage, i) {
                    Ok(()) => Response::empty(),
                    Err(c) => Response::error(c),
                }
            }
            wire::XML_PUT if body.len() > 5 => {
                let offset = u32::from_le_bytes([body[0], body[1], body[2], body[3]]) as usize;
                let n = usize::from(body[4]);
                let (Some(task), Some(data)) =
                    (body.get(5..5 + n).and_then(name), body.get(5 + n..))
                else {
                    return Response::error(Code::InvalidRequest);
                };
                if offset + data.len() > wire::XML_BYTES {
                    return Response::error(Code::Resources);
                }
                let Some(i) = self.tasks.find(task) else {
                    return Response::error(Code::NotFound);
                };
                if !owned(&self.tasks.entries[i].unwrap()) {
                    return Response::error(Code::Denied);
                }
                let _ = storage.mkdir(XML_DIRECTORY);
                let mut path = [0u8; 32];
                let path = xml_path(i, &mut path);
                let disposition = if offset == 0 {
                    Disposition::CreateAlways
                } else {
                    Disposition::OpenExisting
                };
                let file = match storage.open(path, disposition, READ | WRITE, 0) {
                    Ok(f) => f,
                    Err(door_service_client::Error::Service(c)) => return Response::error(c),
                    Err(_) => return Response::error(Code::Io),
                };
                let written = storage.write_all(&file, offset as u64, data);
                let _ = storage.close(file);
                match written {
                    Ok(()) => Response::empty(),
                    Err(_) => Response::error(Code::Io),
                }
            }
            wire::XML_GET if body.len() > 4 => {
                let offset = u32::from_le_bytes([body[0], body[1], body[2], body[3]]) as u64;
                let Some(task) = name(&body[4..]) else {
                    return Response::error(Code::InvalidRequest);
                };
                let Some(i) = self.tasks.find(task) else {
                    return Response::error(Code::NotFound);
                };
                if !owned(&self.tasks.entries[i].unwrap()) {
                    return Response::error(Code::Denied);
                }
                let mut path = [0u8; 32];
                let Ok(file) = storage.open(
                    xml_path(i, &mut path),
                    Disposition::OpenExisting,
                    READ,
                    READ,
                ) else {
                    return Response::empty();
                };
                let mut chunk = [0u8; wire::XML_CHUNK];
                let n = storage.read(&file, offset, &mut chunk).unwrap_or(0);
                let _ = storage.close(file);
                Response::bytes(&chunk[..n])
            }
            _ => Response::error(Code::InvalidRequest),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn task_files_and_working_directories() {
        let mut b = [0u8; 32];
        assert_eq!(xml_path(3, &mut b), "/state/tasks/3.xml");
        assert_eq!(xml_path(15, &mut b), "/state/tasks/15.xml");
        let mut d = [0u8; 256];
        let xml = b"<Exec><Command>a.exe</Command><WorkingDirectory>C:\\R&amp;D</WorkingDirectory></Exec>";
        assert_eq!(working_directory(xml, &mut d), Some("C:\\R&D"));
        assert_eq!(
            working_directory(b"<Exec><Command>a</Command></Exec>", &mut d),
            None
        );
    }
}
