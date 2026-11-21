//! The Task Scheduler (Init hosts it): tasks that run a program once, every
//! N minutes, hours or days from a start time, or at each sign-in, as the
//! user who created them. Tasks persist in `/state/tasks.db`. `schtasks`,
//! Settings > System > Advanced > Scheduled tasks and taskschd.dll (the
//! Task Scheduler 2.0 COM API, which also keeps each task's XML here) use
//! it. A name may hold task folders: `Vendor\Updater` is the task
//! `Updater` in the folder `\Vendor`.

/// Create a task (`Task::encode`); with `replace` (schtasks /F) an existing
/// task of that name is replaced. Reply empty.
pub const CREATE: u32 = 0x7020;
/// Delete (body: name). The task's creator or an administrator.
pub const DELETE: u32 = 0x7021;
/// Run now (body: name).
pub const RUN: u32 = 0x7022;
/// End the running instance (body: name).
pub const END: u32 = 0x7023;
/// The task at `[index u16]`: reply `Status::encode` (NotFound past the
/// last).
pub const QUERY: u32 = 0x7024;
/// Enable or disable (`[enabled u8][name]`).
pub const CHANGE: u32 = 0x7025;
/// Store part of a task's XML definition (`[offset u32][name_len u8][name]
/// [bytes]`, UTF-8; offset 0 starts it over). The task's creator or an
/// administrator.
pub const XML_PUT: u32 = 0x7026;
/// Read a task's XML from `[offset u32][name]`: at most `XML_CHUNK`
/// bytes, empty past the end (or when the task has none).
pub const XML_GET: u32 = 0x7027;
/// The largest piece of XML one XML_GET answers.
pub const XML_CHUNK: usize = 224;
/// The largest XML a task keeps.
pub const XML_BYTES: usize = 16 * 1024;
/// The first and last Task Scheduler operations (Init routes the range).
pub const FIRST: u32 = 0x7020;
pub const LAST: u32 = 0x702f;

/// Tasks the scheduler keeps.
pub const TASKS: usize = 16;
pub const NAME_BYTES: usize = 48;
pub const COMMAND_BYTES: usize = 140;
/// FILETIME ticks per minute.
pub const MINUTE: u64 = 600_000_000;

/// When a task runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Schedule {
    Once,
    /// Every `n` minutes, hours or days from the start time.
    Minutes(u16),
    Hours(u16),
    Days(u16),
    /// At each sign-in of its user.
    OnLogon,
}
impl Schedule {
    fn encode(self) -> (u8, u16) {
        match self {
            Self::Once => (0, 0),
            Self::Minutes(n) => (1, n),
            Self::Hours(n) => (2, n),
            Self::Days(n) => (3, n),
            Self::OnLogon => (4, 0),
        }
    }
    fn decode(kind: u8, n: u16) -> Option<Self> {
        Some(match (kind, n) {
            (0, 0) => Self::Once,
            (1, 1..) => Self::Minutes(n),
            (2, 1..) => Self::Hours(n),
            (3, 1..) => Self::Days(n),
            (4, 0) => Self::OnLogon,
            _ => return None,
        })
    }
    /// The period in FILETIME ticks (None: not repeating).
    pub fn period(self) -> Option<u64> {
        match self {
            Self::Minutes(n) => Some(u64::from(n) * MINUTE),
            Self::Hours(n) => Some(u64::from(n) * 60 * MINUTE),
            Self::Days(n) => Some(u64::from(n) * 1440 * MINUTE),
            Self::Once | Self::OnLogon => None,
        }
    }
    /// The first run at or after `now` (None: no more runs).
    pub fn next(self, start: u64, now: u64) -> Option<u64> {
        match self {
            Self::OnLogon => None,
            Self::Once => (start >= now).then_some(start),
            _ => {
                let period = self.period()?;
                if start >= now {
                    return Some(start);
                }
                let steps = (now - start).div_ceil(period);
                Some(start + steps * period)
            }
        }
    }
}

/// A task: `[name_len u8][command_len u8][schedule u8][enabled u8]
/// [interval u16][replace u8][0][start u64][name][command]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Task {
    name: [u8; NAME_BYTES],
    name_len: usize,
    /// The program (a Windows path, quoted when it has spaces) and its
    /// arguments, as schtasks /TR.
    command: [u8; COMMAND_BYTES],
    command_len: usize,
    pub schedule: Schedule,
    /// FILETIME (UTC) of the first run.
    pub start: u64,
    pub enabled: bool,
    /// CREATE: replace an existing task of the name.
    pub replace: bool,
}
pub const TASK_HEADER: usize = 16;
pub const TASK_BYTES: usize = TASK_HEADER + NAME_BYTES + COMMAND_BYTES;
/// A task name: 1..=NAME_BYTES bytes of UTF-8 without `/ : * ? " < > |`;
/// `\` separates folders (no empty folder, none leading or trailing).
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= NAME_BYTES
        && !name
            .chars()
            .any(|c| c.is_control() || "/:*?\"<>|".contains(c))
        && name.split('\\').all(|part| !part.is_empty())
}
impl Task {
    pub fn new(name: &str, command: &str, schedule: Schedule, start: u64) -> Option<Self> {
        if !valid_name(name) || command.is_empty() || command.len() > COMMAND_BYTES {
            return None;
        }
        let mut t = Self {
            name: [0; NAME_BYTES],
            name_len: name.len(),
            command: [0; COMMAND_BYTES],
            command_len: command.len(),
            schedule,
            start,
            enabled: true,
            replace: false,
        };
        t.name[..name.len()].copy_from_slice(name.as_bytes());
        t.command[..command.len()].copy_from_slice(command.as_bytes());
        Some(t)
    }
    pub fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }
    pub fn command(&self) -> &str {
        core::str::from_utf8(&self.command[..self.command_len]).unwrap_or("")
    }
    pub fn encode(&self, out: &mut [u8; TASK_BYTES]) -> usize {
        let (kind, n) = self.schedule.encode();
        out[0] = self.name_len as u8;
        out[1] = self.command_len as u8;
        out[2] = kind;
        out[3] = u8::from(self.enabled);
        out[4..6].copy_from_slice(&n.to_le_bytes());
        out[6] = u8::from(self.replace);
        out[7] = 0;
        out[8..16].copy_from_slice(&self.start.to_le_bytes());
        let mut at = TASK_HEADER;
        out[at..at + self.name_len].copy_from_slice(&self.name[..self.name_len]);
        at += self.name_len;
        out[at..at + self.command_len].copy_from_slice(&self.command[..self.command_len]);
        at + self.command_len
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < TASK_HEADER || b[3] > 1 || b[6] > 1 || b[7] != 0 {
            return None;
        }
        let (n, c) = (usize::from(b[0]), usize::from(b[1]));
        if b.len() != TASK_HEADER + n + c {
            return None;
        }
        let schedule = Schedule::decode(b[2], u16::from_le_bytes([b[4], b[5]]))?;
        let name = core::str::from_utf8(&b[TASK_HEADER..TASK_HEADER + n]).ok()?;
        let command = core::str::from_utf8(&b[TASK_HEADER + n..]).ok()?;
        let mut t = Self::new(
            name,
            command,
            schedule,
            u64::from_le_bytes(b[8..16].try_into().ok()?),
        )?;
        t.enabled = b[3] == 1;
        t.replace = b[6] == 1;
        Some(t)
    }
}
/// QUERY reply: `[state u8][0; 3][last result u32][next run u64][last run
/// u64][uid u32][0; 4]` then the task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub task: Task,
    /// 1 ready, 2 running, 3 disabled.
    pub state: u8,
    pub last_result: u32,
    /// FILETIME; 0: none ("N/A").
    pub next_run: u64,
    pub last_run: u64,
    pub uid: u32,
}
pub const STATUS_HEADER: usize = 32;
const _: () = assert!(STATUS_HEADER + TASK_BYTES <= 240);
impl Status {
    pub const READY: u8 = 1;
    pub const RUNNING: u8 = 2;
    pub const DISABLED: u8 = 3;
    pub fn encode(&self, out: &mut [u8; STATUS_HEADER + TASK_BYTES]) -> usize {
        out[..STATUS_HEADER].fill(0);
        out[0] = self.state;
        out[4..8].copy_from_slice(&self.last_result.to_le_bytes());
        out[8..16].copy_from_slice(&self.next_run.to_le_bytes());
        out[16..24].copy_from_slice(&self.last_run.to_le_bytes());
        out[24..28].copy_from_slice(&self.uid.to_le_bytes());
        let mut task = [0u8; TASK_BYTES];
        let n = self.task.encode(&mut task);
        out[STATUS_HEADER..STATUS_HEADER + n].copy_from_slice(&task[..n]);
        STATUS_HEADER + n
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < STATUS_HEADER || !(1..=3).contains(&b[0]) {
            return None;
        }
        let w = |at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
        let q = |at: usize| u64::from_le_bytes(b[at..at + 8].try_into().unwrap());
        Some(Self {
            task: Task::decode(&b[STATUS_HEADER..])?,
            state: b[0],
            last_result: w(4),
            next_run: q(8),
            last_run: q(16),
            uid: w(24),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tasks_and_schedules() {
        let mut t = Task::new(
            "Doors Probe",
            "\"C:\\Program Files\\p.exe\" /x",
            Schedule::Minutes(5),
            1000,
        )
        .unwrap();
        t.replace = true;
        let mut b = [0u8; TASK_BYTES];
        let n = t.encode(&mut b);
        assert_eq!(Task::decode(&b[..n]), Some(t));
        assert!(Task::new("a/b", "x", Schedule::Once, 0).is_none());
        assert!(Task::new("Vendor\\Updater", "x", Schedule::Once, 0).is_some());
        for bad in ["\\a", "a\\", "a\\\\b"] {
            assert!(!valid_name(bad), "{bad}");
        }
        let s = Status {
            task: t,
            state: Status::RUNNING,
            last_result: 0x41301,
            next_run: 5,
            last_run: 4,
            uid: 1001,
        };
        let mut sb = [0u8; STATUS_HEADER + TASK_BYTES];
        let n = s.encode(&mut sb);
        assert_eq!(Status::decode(&sb[..n]), Some(s));
        // Runs from the start, every period, never in the past.
        let m = MINUTE;
        assert_eq!(Schedule::Minutes(5).next(100 * m, 90 * m), Some(100 * m));
        assert_eq!(Schedule::Minutes(5).next(100 * m, 101 * m), Some(105 * m));
        assert_eq!(Schedule::Minutes(5).next(100 * m, 105 * m), Some(105 * m));
        assert_eq!(Schedule::Once.next(100 * m, 101 * m), None);
        assert_eq!(Schedule::Days(1).next(0, 1), Some(1440 * m));
        assert_eq!(Schedule::OnLogon.next(0, 0), None);
    }
}
