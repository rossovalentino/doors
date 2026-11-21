//! Actual service outcomes are attributed to kernel-owned retired identities.
//! Numeric fields are canonical and unique; user text cannot forge a kernel row.
use crate::{emulator::Outcome, scenarios::Scenario};
use std::collections::BTreeSet;

pub const SAVE: &str = "services-save";
pub const REOPEN: &str = "services-reopen";
pub const RESTART: &str = "services-restart";
pub const FAILURE: &str = "services-failure";
const NORMAL: &str = "DOOR_P08 SUPERVISOR PASS services=3 apps=2 restart=0 sessions=2 shutdown=1";
const RESTARTED: &str =
    "DOOR_P08 SUPERVISOR PASS services=3 apps=2 restart=1 sessions=2 shutdown=1";
const FAILED: &str = "DOOR_P08 FAILURE PASS launches=4 backoff=1 refused=1 shutdown=1";
const READINESS: &str = "DOOR_P08 READINESS PASS timeout=1 backoff=1";
pub const CONSOLE_COMMANDS: [&str; 6] = ["help", "status", "file", "theme", "threads", "exit"];
pub const RENAME_COMMANDS: [&str; 6] = ["help", "status", "file", "rename", "threads", "exit"];
const RENAME_PASS: &str =
    "DOOR_P12 SDK RENAME PASS uid=1001 durable_commits=2 raw_utf16=1 identity=1 sharing=1 stale=1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Normal,
    Restart,
    Failure,
    Demo,
}
pub fn cases() -> Vec<Scenario> {
    [SAVE, REOPEN, RESTART, FAILURE]
        .into_iter()
        .map(|mode| {
            let mut markers = vec!["KERNEL_DIAGNOSTIC PASS", "DOOR_P08 CLEANUP PASS"];
            if mode == FAILURE {
                markers.push(FAILED);
            } else {
                markers.extend([
                    "DOOR_P08 REGISTRY PASS uid=1001 view=64 epoch=1001 denied=1",
                    "DOOR_P08 REGISTRY PASS uid=1002 view=64 epoch=1002 denied=1",
                    "DOOR_P08 APP PASS uid=1001 bytes=4096 threads=2 tls=1 heap=1 timer=1 denied=1",
                    "DOOR_P08 APP PASS uid=1002 bytes=4096 threads=2 tls=1 heap=1 timer=1 denied=1",
                    if mode == RESTART { RESTARTED } else { NORMAL },
                ]);
                if mode == RESTART {
                    markers.extend([READINESS, "DOOR_P08 RESTART PASS", "DOOR_P08 REVOKE PASS"]);
                }
            }
            Scenario {
                id: mode,
                mode,
                profile: super::PROFILE,
                expected: Outcome::Passed,
                markers,
            }
        })
        .collect()
}
pub fn serial_matches(markers: &[&str], serial: &str) -> bool {
    if !markers.iter().any(|marker| marker.starts_with("DOOR_P08 ")) {
        return true;
    }
    let mode = if markers.contains(&FAILED) {
        Mode::Failure
    } else if markers.contains(&RESTARTED) {
        Mode::Restart
    } else {
        Mode::Normal
    };
    validate(serial, mode).is_some()
}
#[derive(Clone, Copy, Debug)]
pub struct Process {
    pub manifest: u64,
    pub pid: u64,
    pub instance: u64,
    pub uid: u64,
    pub session: u64,
    pub epoch: u64,
    pub exit: i64,
}
fn unsigned(text: &str) -> Option<u64> {
    let value = text.parse::<u64>().ok()?;
    (value.to_string() == text).then_some(value)
}
fn fields(line: &str, prefix: &str, names: &[&str]) -> Option<Vec<u64>> {
    let words: Vec<_> = line.strip_prefix(prefix)?.split(' ').collect();
    if words.len() != names.len() {
        return None;
    }
    words
        .iter()
        .zip(names)
        .map(|(word, name)| unsigned(word.strip_prefix(name)?))
        .collect()
}
fn user(line: &str) -> Option<(u64, &str)> {
    let (pid, text) = line.strip_prefix("DOOR_USER pid=")?.split_once(' ')?;
    Some((unsigned(pid)?, text))
}
pub(crate) fn process(line: &str) -> Option<Process> {
    let (body, exit) = line.split_once(" exit=")?;
    let values = fields(
        body,
        "DOOR_P08 PROCESS ",
        &[
            "manifest=",
            "pid=",
            "instance=",
            "uid=",
            "session=",
            "epoch=",
        ],
    )?;
    let code = exit.parse::<i64>().ok()?;
    if code.to_string() != exit {
        return None;
    }
    Some(Process {
        manifest: values[0],
        pid: values[1],
        instance: values[2],
        uid: values[3],
        session: values[4],
        epoch: values[5],
        exit: code,
    })
}
pub fn validate(serial: &str, mode: Mode) -> Option<Vec<Process>> {
    validate_commands(serial, mode, &CONSOLE_COMMANDS)
}
pub fn validate_rename_console(serial: &str) -> Option<Vec<Process>> {
    let processes = validate_commands(serial, Mode::Demo, &RENAME_COMMANDS)?;
    let markers: Vec<_> = serial
        .lines()
        .filter(|line| line.contains("DOOR_P12 SDK RENAME"))
        .map(user)
        .collect::<Option<_>>()?;
    if markers.len() != 1
        || markers[0].1 != RENAME_PASS
        || !processes
            .iter()
            .any(|p| p.pid == markers[0].0 && p.manifest == 5 && p.uid == 1001 && p.exit == 71)
    {
        return None;
    }
    Some(processes)
}
/// Coarse native elapsed ticks for one fixed, read-only metadata sample. The
/// marker must come from the successfully retired authenticated SDK process.
pub fn rpc_latency(serial: &str) -> Option<u64> {
    let processes = validate_rename_console(serial)?;
    let markers: Vec<_> = serial
        .lines()
        .filter(|line| line.contains("DOOR_P12 SDK RPC"))
        .map(user)
        .collect::<Option<_>>()?;
    if markers.len() != 1
        || !processes
            .iter()
            .any(|p| p.pid == markers[0].0 && p.manifest == 5 && p.uid == 1001 && p.exit == 71)
    {
        return None;
    }
    let values = fields(
        markers[0].1,
        "DOOR_P12 SDK RPC PASS ",
        &["calls=", "ticks=", "identity="],
    )?;
    if values[0] != 128 || values[2] != 1 {
        return None;
    }
    values[1].checked_mul(door_abi::platform::MONOTONIC_TICK_MILLIS)?;
    Some(values[1])
}
pub fn rpc_latency_record(serial: &str) -> Option<serde_json::Value> {
    let ticks = rpc_latency(serial)?;
    Some(serde_json::json!({
        "calls": 128,
        "native_ticks": ticks,
        "tick_millis": door_abi::platform::MONOTONIC_TICK_MILLIS,
        "coarse_millis": ticks.checked_mul(door_abi::platform::MONOTONIC_TICK_MILLIS)?,
        "scope": "Sequential cached metadata RPC on this private single-CPU fixture; not whole-app latency or FPS"
    }))
}
pub fn rpc_latency_matches(serial: &str, record: &serde_json::Value) -> bool {
    rpc_latency_record(serial).as_ref() == Some(record)
}
pub(super) fn validate_commands(
    serial: &str,
    mode: Mode,
    expected_commands: &[&str],
) -> Option<Vec<Process>> {
    if serial
        .lines()
        .any(|line| line.starts_with("DOOR_P08 USER_FAULT "))
    {
        return None;
    }
    let processes: Vec<_> = serial
        .lines()
        .filter(|line| line.starts_with("DOOR_P08 PROCESS"))
        .map(process)
        .collect::<Option<_>>()?;
    let expected = match mode {
        Mode::Normal => 6,
        Mode::Restart => 11,
        Mode::Failure => 5,
        Mode::Demo => 5,
    };
    if processes.len() != expected {
        return None;
    }
    let mut pids = BTreeSet::new();
    let mut instances = BTreeSet::new();
    let mut sessions = BTreeSet::new();
    for process in &processes {
        if process.pid == 0
            || process.instance == 0
            || !pids.insert(process.pid)
            || !instances.insert(process.instance)
        {
            return None;
        }
        if process.manifest == 5 {
            if !matches!(process.uid, 1001 | 1002) || process.session == 0 || process.epoch == 0 {
                return None;
            }
            if process.exit == -13 {
                if mode != Mode::Restart || process.uid != 1002 {
                    return None;
                }
            } else if !sessions.insert(process.session)
                || process.exit != (process.uid - 930) as i64
            {
                return None;
            }
        } else {
            let uid = match process.manifest {
                1 | 2 => 1,
                3 => 2,
                4 => 3,
                _ => return None,
            };
            if process.uid != uid || process.session != 0 || process.epoch != 0 {
                return None;
            }
        }
    }
    let by_manifest = |manifest| {
        processes
            .iter()
            .filter(|p| p.manifest == manifest)
            .collect::<Vec<_>>()
    };
    let init = by_manifest(1);
    if init.len() != 1 || init[0].instance != 1 || init[0].exit != 80 {
        return None;
    }
    let init_pid = init[0].pid;
    let expected_counts = match mode {
        Mode::Normal => [1, 1, 1, 1, 2],
        Mode::Restart => [1, 2, 2, 3, 3],
        Mode::Failure => [1, 4, 0, 0, 0],
        Mode::Demo => [1, 1, 1, 1, 1],
    };
    for manifest in 1..=5 {
        if by_manifest(manifest).len() != expected_counts[manifest as usize - 1] {
            return None;
        }
    }
    for manifest in 2..=4 {
        let mut group = by_manifest(manifest);
        group.sort_by_key(|p| p.instance);
        for (index, process) in group.iter().enumerate() {
            let exit = if mode == Mode::Failure {
                -120
            } else if mode == Mode::Restart && index == 0 && manifest == 2 {
                -151
            } else if mode == Mode::Restart && index == 0 && manifest == 4 {
                -150
            } else {
                0
            };
            if process.exit != exit {
                return None;
            }
        }
    }
    if mode != Mode::Failure {
        let mut app_uids: Vec<_> = by_manifest(5)
            .iter()
            .filter(|p| p.exit != -13)
            .map(|p| p.uid)
            .collect();
        app_uids.sort();
        if app_uids
            != if mode == Mode::Demo {
                vec![1001]
            } else {
                vec![1001, 1002]
            }
        {
            return None;
        }
    }
    let revoked: Vec<_> = by_manifest(5)
        .into_iter()
        .filter(|p| p.exit == -13)
        .collect();
    if revoked.len() != usize::from(mode == Mode::Restart) {
        return None;
    }
    if let Some(probe) = revoked.first() {
        let prior = processes
            .iter()
            .find(|p| p.manifest == 5 && p.uid == 1002 && p.exit == 72)?;
        if probe.session != prior.session
            || probe.epoch != prior.epoch
            || probe.instance <= prior.instance
        {
            return None;
        }
    }
    let cleanup: Vec<_> = serial
        .lines()
        .filter(|line| line.starts_with("DOOR_P08 CLEANUP"))
        .collect();
    let threads = match mode {
        Mode::Normal => 8,
        Mode::Restart => 14,
        Mode::Failure => 5,
        Mode::Demo => 7,
    };
    if cleanup.len() != 1
        || fields(
            cleanup[0],
            "DOOR_P08 CLEANUP PASS ",
            &["processes=", "threads=", "leaked="],
        )? != [expected as u64, threads, 0]
    {
        return None;
    }
    let mut readiness = Vec::new();
    let mut apps = BTreeSet::new();
    let mut registry = BTreeSet::new();
    let mut refused = BTreeSet::new();
    let mut summary = 0;
    let mut deadline = 0;
    let mut restart = 0;
    let mut revocation = 0;
    let mut commands = Vec::new();
    let mut console_ready = 0;
    for line in serial
        .lines()
        .filter(|line| line.starts_with("DOOR_USER pid=") && line.contains("DOOR_P08 "))
    {
        let (pid, text) = user(line)?;
        let owner = processes.iter().find(|p| p.pid == pid)?;
        if text.starts_with("DOOR_P08 READY") {
            if pid != init_pid {
                return None;
            }
            let values = fields(text, "DOOR_P08 READY ", &["manifest=", "pid=", "instance="])?;
            let child = processes.iter().find(|p| {
                p.manifest == values[0] && p.pid == values[1] && p.instance == values[2]
            })?;
            if !(2..=4).contains(&child.manifest)
                || child.exit == -150
                || readiness.contains(&(child.manifest, child.pid))
            {
                return None;
            }
            readiness.push((child.manifest, child.pid));
        } else if text.starts_with("DOOR_P08 APP") {
            let expected = format!(
                "DOOR_P08 APP PASS uid={} bytes=4096 threads=2 tls=1 heap=1 timer=1 denied=1",
                owner.uid
            );
            if owner.manifest != 5 || owner.exit == -13 || text != expected || !apps.insert(pid) {
                return None;
            }
        } else if text.starts_with("DOOR_P08 REGISTRY") {
            let expected = format!(
                "DOOR_P08 REGISTRY PASS uid={} view=64 epoch={} denied=1",
                owner.uid, owner.uid
            );
            if owner.manifest != 5 || owner.exit == -13 || text != expected || !registry.insert(pid)
            {
                return None;
            }
        } else if text == "DOOR_P08 STORAGE REFUSED" {
            if mode != Mode::Failure || owner.manifest != 2 || !refused.insert(pid) {
                return None;
            }
        } else if text.starts_with("DOOR_P08 SUPERVISOR") || text.starts_with("DOOR_P08 FAILURE") {
            let expected = match mode {
                Mode::Normal => NORMAL,
                Mode::Restart => RESTARTED,
                Mode::Failure => FAILED,
                Mode::Demo => {
                    "DOOR_P08 SUPERVISOR PASS services=3 apps=1 restart=0 sessions=1 shutdown=1"
                }
            };
            if pid != init_pid || text != expected {
                return None;
            }
            summary += 1;
        } else if text.starts_with("DOOR_P08 READINESS") {
            if mode != Mode::Restart || pid != init_pid || text != READINESS {
                return None;
            }
            deadline += 1;
        } else if text.starts_with("DOOR_P08 RESTART") {
            if mode != Mode::Restart || pid != init_pid {
                return None;
            }
            let values = fields(text, "DOOR_P08 RESTART PASS ", &["old=", "new=", "stale="])?;
            let storage = by_manifest(2);
            if values[2] != 1
                || !storage
                    .iter()
                    .any(|p| p.instance == values[0] && p.exit == -151)
                || !storage
                    .iter()
                    .any(|p| p.instance == values[1] && p.exit == 0)
                || values[0] >= values[1]
            {
                return None;
            }
            restart += 1;
        } else if text.starts_with("DOOR_P08 REVOKE") {
            if mode != Mode::Restart || pid != init_pid {
                return None;
            }
            let values = fields(
                text,
                "DOOR_P08 REVOKE PASS ",
                &["session=", "threads=", "denied=", "foreign_mutex="],
            )?;
            if values != [revoked[0].session, 2, 1, 1] {
                return None;
            }
            revocation += 1;
        } else if mode == Mode::Demo && owner.manifest == 5 && text == "DOOR_P08 CONSOLE READY" {
            console_ready += 1;
        } else if mode == Mode::Demo
            && owner.manifest == 5
            && text.starts_with("DOOR_P08 CONSOLE COMMAND ")
        {
            commands.push(text.strip_prefix("DOOR_P08 CONSOLE COMMAND ")?);
        } else {
            return None;
        }
    }
    let ready_order: Vec<_> = readiness.iter().map(|(manifest, _)| *manifest).collect();
    if ready_order
        != match mode {
            Mode::Failure => vec![],
            Mode::Restart => vec![2, 3, 4, 2, 3, 4],
            _ => vec![2, 3, 4],
        }
    {
        return None;
    }
    if summary != 1
        || apps.len() != expected_counts[4] - revoked.len()
        || registry.len() != expected_counts[4] - revoked.len()
        || refused.len() != if mode == Mode::Failure { 4 } else { 0 }
        || deadline != usize::from(mode == Mode::Restart)
        || restart != usize::from(mode == Mode::Restart)
        || revocation != usize::from(mode == Mode::Restart)
    {
        return None;
    }
    if mode == Mode::Demo && (console_ready != 1 || commands != expected_commands) {
        return None;
    }
    Some(processes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rpc_measurement_requires_one_exact_sample_from_the_retired_sdk_owner() {
        let normal = fixture(Mode::Demo).replace("CONSOLE COMMAND theme", "CONSOLE COMMAND rename");
        let base = format!("{normal}\nDOOR_USER pid=205 {RENAME_PASS}");
        let marker = "DOOR_USER pid=205 DOOR_P12 SDK RPC PASS calls=128 ticks=128 identity=1";
        let valid = format!("{base}\n{marker}");
        assert_eq!(rpc_latency(&valid), Some(128));
        let record = rpc_latency_record(&valid).unwrap();
        assert!(rpc_latency_matches(&valid, &record));
        for key in [
            "calls",
            "native_ticks",
            "tick_millis",
            "coarse_millis",
            "scope",
        ] {
            let mut changed = record.clone();
            changed[key] = serde_json::json!(0);
            assert!(!rpc_latency_matches(&valid, &changed), "{key}");
            let mut missing = record.clone();
            missing.as_object_mut().unwrap().remove(key);
            assert!(!rpc_latency_matches(&valid, &missing), "missing {key}");
        }
        assert!(!rpc_latency_matches(&base, &record));
        assert!(!rpc_latency_matches(&valid, &serde_json::Value::Null));
        assert_eq!(rpc_latency(&valid.replace("ticks=128", "ticks=0")), Some(0));
        for bad in [
            base,
            valid.replace("pid=205 DOOR_P12 SDK RPC", "pid=202 DOOR_P12 SDK RPC"),
            valid.replace("calls=128", "calls=127"),
            valid.replace("ticks=128", "ticks=0128"),
            valid.replace("ticks=128", "ticks=18446744073709551615"),
            valid.replace("ticks=128", "ticks=-1"),
            valid.replace("SDK RPC PASS", "SDK RPC FAILED"),
            valid.replace("ticks=128 identity=1", "ticks=128 identity=0"),
            valid.replace("DOOR_USER pid=205 DOOR_P12 SDK RPC", "DOOR_P12 SDK RPC"),
            format!("{valid}\n{marker}"),
        ] {
            assert_eq!(rpc_latency(&bad), None, "{bad}");
        }
    }
    #[test]
    fn explicit_rename_console_requires_its_exact_commands_and_retired_sdk_owner() {
        let normal = fixture(Mode::Demo);
        assert!(validate_rename_console(&normal).is_none());
        let commands = normal.replace("CONSOLE COMMAND theme", "CONSOLE COMMAND rename");
        let marker = format!("DOOR_USER pid=205 {RENAME_PASS}");
        let valid = format!("{commands}\n{marker}");
        assert!(validate_rename_console(&valid).is_some());
        assert!(validate(&valid, Mode::Demo).is_none());
        for bad in [
            commands,
            valid.replace(&marker, RENAME_PASS),
            valid.replace("pid=205 DOOR_P12", "pid=202 DOOR_P12"),
            valid.replace("SDK RENAME PASS uid=1001", "SDK RENAME PASS uid=1002"),
            valid.replace("durable_commits=2", "durable_commits=1"),
            valid.replace("raw_utf16=1", "raw_utf16=0"),
            valid.replace("CONSOLE COMMAND rename", "CONSOLE COMMAND theme"),
            format!("{valid}\n{marker}"),
        ] {
            assert!(validate_rename_console(&bad).is_none(), "{bad}");
        }
    }
    fn fixture(mode: Mode) -> String {
        let rows: Vec<(u64, u64, i64)> = match mode {
            Mode::Normal => vec![
                (1, 1, 80),
                (2, 1, 0),
                (3, 2, 0),
                (4, 3, 0),
                (5, 1001, 71),
                (5, 1002, 72),
            ],
            Mode::Restart => vec![
                (1, 1, 80),
                (2, 1, -151),
                (3, 2, 0),
                (4, 3, -150),
                (4, 3, 0),
                (2, 1, 0),
                (3, 2, 0),
                (4, 3, 0),
                (5, 1001, 71),
                (5, 1002, 72),
                (5, 1002, -13),
            ],
            Mode::Failure => vec![
                (1, 1, 80),
                (2, 1, -120),
                (2, 1, -120),
                (2, 1, -120),
                (2, 1, -120),
            ],
            Mode::Demo => vec![(1, 1, 80), (2, 1, 0), (3, 2, 0), (4, 3, 0), (5, 1001, 71)],
        };
        let mut lines = Vec::new();
        for (i, (manifest, uid, exit)) in rows.iter().enumerate() {
            lines.push(format!("DOOR_P08 PROCESS manifest={manifest} pid={} instance={} uid={uid} session={} epoch={} exit={exit}",201+i,1+i,if *manifest==5{uid-990}else{0},u8::from(*manifest==5)));
            if *manifest == 5 && *exit != -13 {
                lines.push(format!("DOOR_USER pid={} DOOR_P08 REGISTRY PASS uid={uid} view=64 epoch={uid} denied=1",201+i));
                lines.push(format!("DOOR_USER pid={} DOOR_P08 APP PASS uid={uid} bytes=4096 threads=2 tls=1 heap=1 timer=1 denied=1",201+i));
            }
            if mode == Mode::Failure && *manifest == 2 {
                lines.push(format!(
                    "DOOR_USER pid={} DOOR_P08 STORAGE REFUSED",
                    201 + i
                ));
            }
        }
        let ready: Vec<usize> = match mode {
            Mode::Failure => vec![],
            Mode::Restart => vec![1, 2, 4, 5, 6, 7],
            _ => vec![1, 2, 3],
        };
        for i in ready {
            lines.push(format!(
                "DOOR_USER pid=201 DOOR_P08 READY manifest={} pid={} instance={}",
                rows[i].0,
                201 + i,
                1 + i
            ));
        }
        if mode == Mode::Restart {
            lines.extend([
                format!("DOOR_USER pid=201 {READINESS}"),
                "DOOR_USER pid=201 DOOR_P08 RESTART PASS old=2 new=6 stale=1".into(),
                "DOOR_USER pid=201 DOOR_P08 REVOKE PASS session=12 threads=2 denied=1 foreign_mutex=1".into(),
            ]);
        }
        let summary = match mode {
            Mode::Normal => NORMAL,
            Mode::Restart => RESTARTED,
            Mode::Failure => FAILED,
            Mode::Demo => {
                "DOOR_P08 SUPERVISOR PASS services=3 apps=1 restart=0 sessions=1 shutdown=1"
            }
        };
        lines.push(format!("DOOR_USER pid=201 {summary}"));
        if mode == Mode::Demo {
            lines.push("DOOR_USER pid=205 DOOR_P08 CONSOLE READY".into());
            for command in ["help", "status", "file", "theme", "threads", "exit"] {
                lines.push(format!(
                    "DOOR_USER pid=205 DOOR_P08 CONSOLE COMMAND {command}"
                ));
            }
        }
        let threads = match mode {
            Mode::Normal => 8,
            Mode::Restart => 14,
            Mode::Failure => 5,
            Mode::Demo => 7,
        };
        lines.push(format!(
            "DOOR_P08 CLEANUP PASS processes={} threads={threads} leaked=0",
            rows.len()
        ));
        lines.join("\n")
    }
    #[test]
    fn service_schedule_keeps_every_p07_case_and_requires_four_new_boots() {
        let old = crate::scenarios::cases("P07").unwrap();
        let new = crate::scenarios::cases("P08").unwrap();
        assert_eq!(old.len(), 35);
        assert_eq!(new.len(), 39);
        for (a, b) in old.iter().zip(&new) {
            assert_eq!(
                (a.id, a.mode, a.profile, &a.expected, &a.markers),
                (b.id, b.mode, b.profile, &b.expected, &b.markers)
            );
        }
        assert_eq!(
            new[35..].iter().map(|case| case.mode).collect::<Vec<_>>(),
            [SAVE, REOPEN, RESTART, FAILURE]
        );
    }
    #[test]
    fn services_require_authenticated_processes_effects_and_exact_retirement() {
        for mode in [Mode::Normal, Mode::Restart, Mode::Failure, Mode::Demo] {
            let serial = fixture(mode);
            assert!(validate(&serial, mode).is_some(), "{mode:?}");
            for bad in [
                serial.replace("pid=201 ", "pid=0201 "),
                serial.replace("instance=1 ", "instance=0 "),
                serial.replace("uid=1 ", "uid=1001 "),
                serial.replace("epoch=0 ", "epoch=1 "),
                serial.replace("exit=80", "exit=0"),
                serial.replace("leaked=0", "leaked=1"),
                serial.replace("DOOR_P08 CLEANUP PASS", "DOOR_P08 CLEANUP FAILED"),
                format!(
                    "{serial}\nDOOR_P08 PROCESS manifest=1 pid=201 instance=1 uid=1 session=0 epoch=0 exit=80"
                ),
                format!("{serial}\nDOOR_USER pid=201 DOOR_P08 INIT FAIL status=16"),
            ] {
                assert!(validate(&bad, mode).is_none(), "accepted {mode:?}: {bad}");
            }
            for line in serial.lines() {
                assert!(
                    validate(&serial.replace(line, ""), mode).is_none(),
                    "accepted missing {line}"
                );
            }
        }
        let normal = fixture(Mode::Normal);
        for bad in [
            normal.replace("pid=205 DOOR_P08 APP", "pid=202 DOOR_P08 APP"),
            normal.replace("session=11 ", "session=12 "),
            normal.replace("threads=8", "threads=6"),
            normal.replace("bytes=4096", "bytes=2048"),
            normal.replace("instance=2", "instance=3"),
        ] {
            assert!(validate(&bad, Mode::Normal).is_none());
        }
        let restart = fixture(Mode::Restart);
        for bad in [
            restart.replace("old=2 new=6", "old=6 new=2"),
            restart.replace("exit=-150", "exit=0"),
            restart.replace("stale=1", "stale=0"),
            restart.replace("backoff=1", "backoff=0"),
            restart.replace("REVOKE PASS session=12", "REVOKE PASS session=11"),
            restart.replace("foreign_mutex=1", "foreign_mutex=0"),
            restart.replace(" foreign_mutex=1", ""),
            restart.replace("pid=201 DOOR_P08 REVOKE", "pid=211 DOOR_P08 REVOKE"),
            restart.replace("exit=-13", "exit=72"),
            restart.replace("session=12 epoch=1 exit=-13", "session=13 epoch=1 exit=-13"),
            restart.replace("session=12 epoch=1 exit=-13", "session=12 epoch=2 exit=-13"),
            restart.replace("threads=14", "threads=12"),
        ] {
            assert!(validate(&bad, Mode::Restart).is_none());
        }
        let demo = fixture(Mode::Demo);
        assert!(validate(&demo.replace("COMMAND theme", "COMMAND file"), Mode::Demo).is_none());
        assert!(
            validate(
                &demo.replace("pid=205 DOOR_P08 CONSOLE", "pid=204 DOOR_P08 CONSOLE"),
                Mode::Demo
            )
            .is_none()
        );
    }
}
