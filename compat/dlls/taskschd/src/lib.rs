#![no_std]
//! Original Task Scheduler 2.0 (taskschd.dll): the TaskScheduler class and
//! the object model it hands out (ITaskService, folders, registered and
//! running tasks, task definitions with their triggers, actions, settings and
//! principal), over the scheduler Init hosts
//! (`door_service_protocol::tasks`), which keeps each task's XML.
//!
//! This module is the host-testable part: the task definition, its XML in
//! the layout Windows 11 writes (`tests/reference/tschedprobe-raw.txt` is
//! what Windows printed for the same definitions) and reads, and what of a
//! definition Doors' scheduler can run. The COM objects are in `guest`.
//!
//! The same sources build the x64 and the x86 DLL (P17): Windows-facing
//! exports take pointer-width parameters, widened at entry; a pointer or
//! handle result is narrowed from the body's 64-bit value (an immediately
//! called closure, so the body's own `return`s keep their meaning).
#![allow(clippy::redundant_closure_call)]
extern crate alloc;
use alloc::{string::String, vec::Vec};

pub mod xml;

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

/// HRESULTs (winerror.h).
pub mod hr {
    pub const S_OK: u32 = 0;
    pub const S_FALSE: u32 = 1;
    pub const E_NOTIMPL: u32 = 0x8000_4001;
    pub const E_NOINTERFACE: u32 = 0x8000_4002;
    pub const E_POINTER: u32 = 0x8000_4003;
    pub const E_FAIL: u32 = 0x8000_4005;
    pub const E_ACCESSDENIED: u32 = 0x8007_0005;
    pub const E_OUTOFMEMORY: u32 = 0x8007_000E;
    pub const E_INVALIDARG: u32 = 0x8007_0057;
    pub const CLASS_E_NOAGGREGATION: u32 = 0x8004_0110;
    pub const CLASS_E_CLASSNOTAVAILABLE: u32 = 0x8004_0111;
    /// HRESULT_FROM_WIN32 of ERROR_FILE_NOT_FOUND, ERROR_NOT_SUPPORTED,
    /// ERROR_INVALID_NAME, ERROR_DIR_NOT_EMPTY, ERROR_ALREADY_EXISTS,
    /// ERROR_FILENAME_EXCED_RANGE, ERROR_ONLY_IF_CONNECTED, RPC_S_SERVER_UNAVAILABLE.
    pub const FILE_NOT_FOUND: u32 = 0x8007_0002;
    pub const NOT_SUPPORTED: u32 = 0x8007_0032;
    pub const INVALID_NAME: u32 = 0x8007_007B;
    pub const DIR_NOT_EMPTY: u32 = 0x8007_0091;
    pub const ALREADY_EXISTS: u32 = 0x8007_00B7;
    pub const NAME_TOO_LONG: u32 = 0x8007_00CE;
    pub const ONLY_IF_CONNECTED: u32 = 0x8007_04E3;
    pub const SERVER_UNAVAILABLE: u32 = 0x8007_06BA;
    pub const SCHED_S_TASK_RUNNING: u32 = 0x0004_1301;
    pub const SCHED_S_TASK_HAS_NOT_RUN: u32 = 0x0004_1303;
    pub const SCHED_E_TASK_NOT_RUNNING: u32 = 0x8004_130B;
    pub const SCHED_E_UNSUPPORTED_ACCOUNT_OPTION: u32 = 0x8004_1314;
    pub const SCHED_E_UNEXPECTEDNODE: u32 = 0x8004_1316;
    pub const SCHED_E_NAMESPACE: u32 = 0x8004_1317;
    pub const SCHED_E_INVALIDVALUE: u32 = 0x8004_1318;
    pub const SCHED_E_MISSINGNODE: u32 = 0x8004_1319;
    pub const SCHED_E_MALFORMEDXML: u32 = 0x8004_131A;
    pub const SCHED_E_TOO_MANY_NODES: u32 = 0x8004_131D;
    pub const SCHED_E_TASK_DISABLED: u32 = 0x8004_1326;
    pub const SCHED_E_START_ON_DEMAND: u32 = 0x8004_1328;
}

/// TASK_TRIGGER_TYPE2.
pub mod trigger {
    pub const EVENT: u32 = 0;
    pub const TIME: u32 = 1;
    pub const DAILY: u32 = 2;
    pub const WEEKLY: u32 = 3;
    pub const MONTHLY: u32 = 4;
    pub const MONTHLYDOW: u32 = 5;
    pub const IDLE: u32 = 6;
    pub const REGISTRATION: u32 = 7;
    pub const BOOT: u32 = 8;
    pub const LOGON: u32 = 9;
    pub const SESSION_STATE_CHANGE: u32 = 11;
    /// Types ITriggerCollection::Create makes (12, the custom trigger, it
    /// does not).
    pub fn creatable(kind: u32) -> bool {
        kind <= 9 || kind == 11
    }
}
/// TASK_ACTION_TYPE.
pub mod action {
    pub const EXEC: u32 = 0;
    pub const COM_HANDLER: u32 = 5;
    pub const SEND_EMAIL: u32 = 6;
    pub const SHOW_MESSAGE: u32 = 7;
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Registration {
    pub source: Option<String>,
    pub date: Option<String>,
    pub author: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub documentation: Option<String>,
    pub uri: Option<String>,
    pub security_descriptor: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Repetition {
    pub interval: Option<String>,
    pub duration: Option<String>,
    pub stop_at_end: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trigger {
    pub kind: u32,
    pub id: Option<String>,
    pub repetition: Repetition,
    pub start: Option<String>,
    pub end: Option<String>,
    pub limit: Option<String>,
    pub enabled: bool,
    pub random_delay: Option<String>,
    pub delay: Option<String>,
    pub user_id: Option<String>,
    pub days_interval: i16,
    pub weeks_interval: i16,
    pub days_of_week: i16,
    pub days_of_month: i32,
    pub months: i16,
    pub weeks_of_month: i16,
    pub last_day: bool,
    pub last_week: bool,
    pub state_change: u32,
    pub subscription: Option<String>,
    pub value_queries: Vec<(String, String)>,
}
impl Trigger {
    pub fn new(kind: u32) -> Self {
        Self {
            kind,
            id: None,
            repetition: Repetition::default(),
            start: None,
            end: None,
            limit: None,
            enabled: true,
            random_delay: None,
            delay: None,
            user_id: None,
            days_interval: 1,
            weeks_interval: 1,
            days_of_week: 0,
            days_of_month: 0,
            months: 0,
            weeks_of_month: 0,
            last_day: false,
            last_week: false,
            state_change: 0,
            subscription: None,
            value_queries: Vec::new(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub kind: u32,
    pub id: Option<String>,
    pub path: Option<String>,
    pub arguments: Option<String>,
    pub directory: Option<String>,
    pub hide_window: bool,
    pub class_id: Option<String>,
    pub data: Option<String>,
    pub server: Option<String>,
    pub subject: Option<String>,
    pub to: Option<String>,
    pub cc: Option<String>,
    pub bcc: Option<String>,
    pub reply_to: Option<String>,
    pub from: Option<String>,
    pub body: Option<String>,
    pub title: Option<String>,
    pub header_fields: Vec<(String, String)>,
}
impl Action {
    pub fn new(kind: u32) -> Self {
        Self {
            kind,
            id: None,
            path: None,
            arguments: None,
            directory: None,
            hide_window: false,
            class_id: None,
            data: None,
            server: None,
            subject: None,
            to: None,
            cc: None,
            bcc: None,
            reply_to: None,
            from: None,
            body: None,
            title: None,
            header_fields: Vec::new(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Idle {
    pub duration: Option<String>,
    pub wait: Option<String>,
    pub stop_on_end: bool,
    pub restart: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settings {
    pub demand_start: bool,
    pub restart_interval: Option<String>,
    pub restart_count: i32,
    pub instances: u32,
    pub stop_on_batteries: bool,
    pub disallow_on_batteries: bool,
    pub hard_terminate: bool,
    pub when_available: bool,
    pub network_only: bool,
    pub limit: Option<String>,
    pub enabled: bool,
    pub delete_after: Option<String>,
    pub priority: i32,
    pub compatibility: u32,
    pub hidden: bool,
    pub idle: Idle,
    pub idle_only: bool,
    pub wake: bool,
    pub network_name: Option<String>,
    pub network_id: Option<String>,
    pub remote_app: bool,
    pub unified: bool,
    pub volatile: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Principal {
    pub id: Option<String>,
    pub user_id: Option<String>,
    pub group_id: Option<String>,
    pub display_name: Option<String>,
    pub logon: u32,
    /// LogonType was set (or read): it is written.
    pub logon_set: bool,
    pub run_level: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    pub info: Registration,
    pub triggers: Vec<Trigger>,
    pub settings: Settings,
    pub principal: Principal,
    pub actions: Vec<Action>,
    pub context: Option<String>,
    pub data: Option<String>,
}
impl Definition {
    /// ITaskService::NewTask's definition (its idle settings carry the
    /// defaults Windows writes).
    pub fn new_task() -> Self {
        let mut d = Self::from_defaults();
        d.settings.idle.duration = Some("PT10M".into());
        d.settings.idle.wait = Some("PT1H".into());
        d
    }
    /// What an XML definition holds before its elements are read.
    pub fn from_defaults() -> Self {
        Self {
            info: Registration::default(),
            triggers: Vec::new(),
            settings: Settings {
                demand_start: true,
                restart_interval: None,
                restart_count: 0,
                instances: 2,
                stop_on_batteries: true,
                disallow_on_batteries: true,
                hard_terminate: true,
                when_available: false,
                network_only: false,
                limit: Some("PT72H".into()),
                enabled: true,
                delete_after: None,
                priority: 7,
                compatibility: 2,
                hidden: false,
                idle: Idle {
                    duration: None,
                    wait: None,
                    stop_on_end: true,
                    restart: false,
                },
                idle_only: false,
                wake: false,
                network_name: None,
                network_id: None,
                remote_app: false,
                unified: false,
                volatile: false,
            },
            principal: Principal {
                id: None,
                user_id: None,
                group_id: None,
                display_name: None,
                logon: 3,
                logon_set: false,
                run_level: 0,
            },
            actions: Vec::new(),
            context: None,
            data: None,
        }
    }
    /// What RegisterTaskDefinition checks before anything is registered.
    pub fn validate(&self) -> Result<(), u32> {
        if self.actions.is_empty() {
            return Err(hr::SCHED_E_MISSINGNODE);
        }
        if self.actions.len() > 32 || self.triggers.len() > 48 {
            return Err(hr::SCHED_E_TOO_MANY_NODES);
        }
        for a in &self.actions {
            let missing = match a.kind {
                action::EXEC => a.path.as_deref().is_none_or(str::is_empty),
                action::COM_HANDLER => a.class_id.is_none(),
                action::SHOW_MESSAGE => a.title.is_none() || a.body.is_none(),
                _ => a.server.is_none() || a.from.is_none(),
            };
            if missing {
                return Err(hr::SCHED_E_MISSINGNODE);
            }
        }
        if self.settings.instances > 3 || !(0..=10).contains(&self.settings.priority) {
            return Err(hr::SCHED_E_INVALIDVALUE);
        }
        for t in &self.triggers {
            for b in [&t.start, &t.end].into_iter().flatten() {
                if parse_boundary(b).is_none() {
                    return Err(hr::SCHED_E_INVALIDVALUE);
                }
            }
            if matches!(
                t.kind,
                trigger::TIME
                    | trigger::DAILY
                    | trigger::WEEKLY
                    | trigger::MONTHLY
                    | trigger::MONTHLYDOW
            ) && t.start.is_none()
            {
                return Err(hr::SCHED_E_MISSINGNODE);
            }
        }
        Ok(())
    }
}

/// A trigger boundary (`YYYY-MM-DDTHH:MM:SS`, optional fraction, then
/// optional `Z` or `+HH:MM`): the civil date and time, and the offset from
/// UTC in minutes when the text gives one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Boundary {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub offset: Option<i16>,
}
pub fn parse_boundary(text: &str) -> Option<Boundary> {
    let b = text.as_bytes();
    let digits = |r: core::ops::Range<usize>| -> Option<u32> {
        let s = b.get(r)?;
        s.iter().try_fold(0u32, |v, c| {
            c.is_ascii_digit().then(|| v * 10 + u32::from(c - b'0'))
        })
    };
    if b.len() < 19
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let mut at = 19;
    if b.get(at) == Some(&b'.') {
        at += 1;
        while b.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
    }
    let offset = match b.get(at) {
        None => None,
        Some(b'Z') if at + 1 == b.len() => Some(0),
        Some(&sign @ (b'+' | b'-')) if b.len() == at + 6 && b[at + 3] == b':' => {
            let m = digits(at + 1..at + 3)? * 60 + digits(at + 4..at + 6)?;
            Some(if sign == b'-' { -(m as i16) } else { m as i16 })
        }
        _ => return None,
    };
    let v = Boundary {
        year: digits(0..4)? as u16,
        month: digits(5..7)? as u8,
        day: digits(8..10)? as u8,
        hour: digits(11..13)? as u8,
        minute: digits(14..16)? as u8,
        second: digits(17..19)? as u8,
        offset,
    };
    let days = days_in_month(v.year, v.month);
    ((1..=12).contains(&v.month)
        && v.day >= 1
        && v.day <= days
        && v.hour < 24
        && v.minute < 60
        && v.second < 60)
        .then_some(v)
}
fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        2 if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}
/// An XML duration of whole minutes, hours or days (`PT5M`, `PT2H`,
/// `P3D`): the scheduler's repeating schedule.
pub fn period(text: &str) -> Option<door_service_protocol::tasks::Schedule> {
    use door_service_protocol::tasks::Schedule;
    let (number, unit) = if let Some(rest) = text.strip_prefix("PT") {
        (&rest[..rest.len().checked_sub(1)?], rest.as_bytes().last()?)
    } else {
        let rest = text.strip_prefix('P')?;
        (&rest[..rest.len().checked_sub(1)?], rest.as_bytes().last()?)
    };
    let n: u16 = number.parse().ok().filter(|n| *n > 0)?;
    match (text.starts_with("PT"), unit) {
        (true, b'M') => Some(Schedule::Minutes(n)),
        (true, b'H') => Some(Schedule::Hours(n)),
        (false, b'D') => Some(Schedule::Days(n)),
        _ => None,
    }
}
/// What Doors' scheduler runs for a definition: its schedule and start
/// (None: no trigger, on demand only), its command line and whether it is
/// enabled. Anything the scheduler cannot keep answers NOT_SUPPORTED
/// (recorded deviations: one trigger, one Exec action, no end, delay,
/// random delay, repetition duration or other trigger types).
pub struct Plan<'a> {
    pub schedule: door_service_protocol::tasks::Schedule,
    pub start: Option<Boundary>,
    pub command: String,
    pub enabled: bool,
    pub directory: Option<&'a str>,
}
pub fn plan(d: &Definition) -> Result<Plan<'_>, u32> {
    use door_service_protocol::tasks::{COMMAND_BYTES, Schedule};
    d.validate()?;
    let [a] = d.actions.as_slice() else {
        return Err(hr::NOT_SUPPORTED);
    };
    if a.kind != action::EXEC || a.hide_window {
        return Err(hr::NOT_SUPPORTED);
    }
    let path = a.path.as_deref().unwrap_or("");
    let mut command = String::new();
    if path.contains(' ') && !path.starts_with('"') {
        command.push('"');
        command.push_str(path);
        command.push('"');
    } else {
        command.push_str(path);
    }
    if let Some(args) = a.arguments.as_deref().filter(|s| !s.is_empty()) {
        command.push(' ');
        command.push_str(args);
    }
    if command.len() > COMMAND_BYTES {
        return Err(hr::NOT_SUPPORTED);
    }
    if d.principal.run_level != 0 {
        return Err(hr::NOT_SUPPORTED);
    }
    if matches!(d.principal.logon, 1 | 2 | 4 | 5 | 6) && d.principal.logon_set {
        return Err(hr::SCHED_E_UNSUPPORTED_ACCOUNT_OPTION);
    }
    let (schedule, start) = match d.triggers.as_slice() {
        [] => (Schedule::Once, None),
        [t] => {
            if t.end.is_some()
                || t.delay.is_some()
                || t.random_delay.is_some()
                || t.repetition.duration.is_some()
                || !t.enabled
            {
                return Err(hr::NOT_SUPPORTED);
            }
            let start = t.start.as_deref().and_then(parse_boundary);
            let schedule = match t.kind {
                trigger::TIME => match t.repetition.interval.as_deref() {
                    None => Schedule::Once,
                    Some(i) => period(i).ok_or(hr::NOT_SUPPORTED)?,
                },
                trigger::DAILY if t.repetition.interval.is_none() && t.days_interval > 0 => {
                    Schedule::Days(t.days_interval as u16)
                }
                trigger::LOGON if t.repetition.interval.is_none() => Schedule::OnLogon,
                _ => return Err(hr::NOT_SUPPORTED),
            };
            (schedule, start)
        }
        _ => return Err(hr::NOT_SUPPORTED),
    };
    Ok(Plan {
        schedule,
        start,
        command,
        enabled: d.settings.enabled,
        directory: a.directory.as_deref(),
    })
}

/// A GUID as `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`, lower case.
pub fn guid_text(g: &[u8; 16]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let order = [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15];
    let mut out = String::new();
    for (k, &i) in order.iter().enumerate() {
        if matches!(k, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        out.push(HEX[usize::from(g[i] >> 4)] as char);
        out.push(HEX[usize::from(g[i] & 15)] as char);
    }
    out
}

#[cfg(test)]
mod tests;
