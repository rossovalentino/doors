//! Derive the bounded shutdown document choice from sealed guest observations.
//! Power gesture counts are controller observations, never inferred guest facts.
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn required(wide: &Value, checks: &[Value], serial: &str) -> bool {
    wide["document_close_protocol"] == 2
        || checks
            .iter()
            .any(|c| c["id"] == "desktop-shutdown-save-dialog")
        || serial
            .lines()
            .any(|l| l.contains("DOOR_P12 NOTES PROMPT intent=Close"))
}
pub(super) fn validate_summary(saved: &Value, canonical: &Value) -> Result<(), String> {
    if saved["notes_pid"] != canonical["notes"]["pid"]
        || saved["decision"] != "Save"
        || saved["fresh_close_prompt"] != true
        || saved["fresh_client_exit"] != true
        || saved["power_requests"] != 2
        || saved["first_request_canceled"] != true
    {
        return Err("shutdown summary disagrees with canonical guest/controller contract".into());
    }
    Ok(())
}
fn fields(text: &str) -> Result<BTreeMap<&str, i64>, String> {
    let mut result = BTreeMap::new();
    for token in text.split_whitespace() {
        let (key, value) = token.split_once('=').ok_or("malformed shutdown field")?;
        let number: i64 = value.parse().map_err(|_| "nonnumeric shutdown field")?;
        if number.to_string() != value {
            return Err("noncanonical shutdown field".into());
        }
        if result.insert(key, number).is_some() {
            return Err("duplicate shutdown field".into());
        }
    }
    Ok(result)
}
fn user(line: &str) -> Option<(i64, &str)> {
    let (pid, payload) = line.strip_prefix("DOOR_USER pid=")?.split_once(' ')?;
    let number: i64 = pid.parse().ok()?;
    (number > 0 && number.to_string() == pid).then_some((number, payload))
}
/// Exit code Init gives the session's windowless Start search host.
pub(super) const SEARCH_HOST_EXIT: i64 = -150;
/// Manifest of the process in the search-host slot: since P14 the desktop
/// shell (`door-desktop-shell`), which hosts Start file search.
pub(super) const SEARCH_HOST_MANIFEST: i64 = door_abi::platform::manifest::DESKTOP_SHELL as i64;
/// Search host processes Init launched (`DOOR_P12 SEARCH_HOST LAUNCHED`).
/// Each is one more retired workspace process (manifest 12) at shutdown.
pub(super) fn search_hosts(text: &str) -> BTreeSet<i64> {
    text.lines()
        .filter_map(user)
        .filter_map(|(_, p)| p.strip_prefix("DOOR_P12 SEARCH_HOST LAUNCHED pid="))
        .filter_map(|v| v.trim().parse().ok())
        .collect()
}
/// Exit code Init gives the session IME (`door-ime`, manifest 16).
pub(super) const IME_EXIT: i64 = -151;
/// IME processes Init launched (`DOOR_P12 IME LAUNCHED`). Each is one more
/// retired session process (manifest 16) at shutdown.
pub(super) fn ime_hosts(text: &str) -> BTreeSet<i64> {
    text.lines()
        .filter_map(user)
        .filter_map(|(_, p)| p.strip_prefix("DOOR_P12 IME LAUNCHED pid="))
        .filter_map(|v| v.trim().parse().ok())
        .collect()
}
/// The P15 system services. pids, door-network and door-audio (Init
/// reports each incarnation ready): session-0 services (uid 4) that Init
/// stops with exit -1.
pub(super) fn network_hosts(text: &str) -> BTreeSet<i64> {
    text.lines()
        .filter_map(user)
        .filter_map(|(_, p)| {
            p.strip_prefix("DOOR_P15 NETWORK READY pid=")
                .or_else(|| p.strip_prefix("DOOR_P15 AUDIO READY pid="))
        })
        .filter_map(|v| v.trim().parse().ok())
        .collect()
}
/// door-audio's pids alone (manifest 30; `network_hosts` counts both).
pub(super) fn audio_hosts(text: &str) -> BTreeSet<i64> {
    text.lines()
        .filter_map(user)
        .filter_map(|(_, p)| p.strip_prefix("DOOR_P15 AUDIO READY pid="))
        .filter_map(|v| v.trim().parse().ok())
        .collect()
}
/// door-network and door-audio.
const SYSTEM_MANIFESTS: [i64; 2] = [29, 30];
fn unique(items: Vec<usize>) -> Result<usize, String> {
    if items.len() != 1 {
        return Err("missing or duplicate shutdown event".into());
    }
    Ok(items[0])
}
/// Hash only after the final kernel record including its newline is present.
/// CLEANUP is earlier; accepting its prefix can seal an actively growing file.
pub(super) fn device_complete(text: &str) -> bool {
    let Some(line) = text.strip_suffix('\n').and_then(|s| s.lines().last()) else {
        return false;
    };
    let Some(value) = line.strip_prefix("DOOR_P08 DEVICE PASS ") else {
        return false;
    };
    let Ok(f) = fields(value) else {
        return false;
    };
    f.keys().copied().collect::<BTreeSet<_>>()
        == BTreeSet::from([
            "reads",
            "writes",
            "flushes",
            "sampled_remaining",
            "reclaimed",
        ])
        && f["reclaimed"] == 8
        && f.values().all(|v| *v >= 0)
}
pub(super) fn canonical(text: &str) -> Result<Value, String> {
    canonical_for(text, 18)
}
/// A bounded Files fixture has no shutdown document prompt. Still require the
/// complete kernel-owned identities, ordered service retirement and DEVICE8.
pub(super) fn retirement(text: &str, apps: usize) -> Result<Value, String> {
    retirement_commands(text, &(1..=apps as i64).collect::<Vec<_>>())
}
pub(super) fn retirement_commands(text: &str, commands: &[i64]) -> Result<Value, String> {
    if commands.is_empty()
        || commands.iter().copied().collect::<BTreeSet<_>>().len() != commands.len()
    {
        return Err("invalid expected application identities".into());
    }
    let expected_manifest = |command: i64| match command {
        1..=3 | 9 => Some(12),
        10 => Some(13),
        11 => Some(14),
        _ => None,
    };
    if commands.iter().any(|c| expected_manifest(*c).is_none()) {
        return Err("unknown expected application command".into());
    }
    let apps = commands.len();
    let hosts = search_hosts(text);
    let imes = ime_hosts(text);
    let nets = network_hosts(text);
    let mut retired_hosts = 0;
    let mut retired_imes = 0;
    let mut retired_nets = 0;
    let lines: Vec<_> = text.lines().collect();
    if !device_complete(text)
        || lines.iter().any(|l| {
            user(l).is_some_and(|(_, p)| {
                p.starts_with("DOOR_P08 PROCESS ")
                    || p.starts_with("DOOR_P08 CLEANUP ")
                    || p.starts_with("DOOR_P08 DEVICE ")
            })
        })
    {
        return Err("Files retirement needs final authentic kernel records".into());
    }
    let mut pids = BTreeSet::new();
    let mut instances = BTreeSet::new();
    let mut processes = Vec::new();
    let mut service_lines = BTreeMap::new();
    let mut applications = 0;
    for (i, line) in lines.iter().enumerate() {
        if let Some(value) = line.strip_prefix("DOOR_P08 PROCESS ") {
            let f = fields(value)?;
            if f.keys().copied().collect::<BTreeSet<_>>()
                != BTreeSet::from([
                    "manifest", "pid", "instance", "uid", "session", "epoch", "exit",
                ])
                || f["pid"] <= 0
                || f["instance"] <= 0
                || !pids.insert(f["pid"])
                || !instances.insert(f["instance"])
            {
                return Err("Files retirement invalid/duplicate identity".into());
            }
            let host = f["manifest"] == SEARCH_HOST_MANIFEST && hosts.contains(&f["pid"]);
            let ime = f["manifest"] == 16 && imes.contains(&f["pid"]);
            if SYSTEM_MANIFESTS.contains(&f["manifest"]) && nets.contains(&f["pid"]) {
                if f["uid"] != 4 || f["exit"] != -1 || f["session"] != 0 || f["epoch"] != 0 {
                    return Err("Files retirement network service differs".into());
                }
                retired_nets += 1;
                processes.push((i, f));
                continue;
            }
            let (uid, exit) = if host {
                (1001, SEARCH_HOST_EXIT)
            } else if ime {
                (1001, IME_EXIT)
            } else {
                match f["manifest"] {
                    1 => (1, 80),
                    2 => (1, 0),
                    3 => (2, 0),
                    4 => (3, 0),
                    11 => (3, -1),
                    12 => (1001, 0),
                    13 if commands.contains(&10) => (1001, 0),
                    14 if commands.contains(&11) => (1001, 0),
                    _ => return Err("Files retirement unexpected manifest".into()),
                }
            };
            if f["uid"] != uid || f["exit"] != exit {
                return Err("Files retirement wrong UID/exit".into());
            }
            if matches!(f["manifest"], 12 | 13 | 14 | 16) || host {
                if f["session"] <= 0 || f["epoch"] <= 0 {
                    return Err("Files retirement app session missing".into());
                }
                if host {
                    retired_hosts += 1;
                } else if ime {
                    retired_imes += 1;
                } else {
                    applications += 1;
                }
            } else if f["session"] != 0
                || f["epoch"] != 0
                || service_lines.insert(f["manifest"], i).is_some()
            {
                return Err("Files retirement service authority differs".into());
            }
            processes.push((i, f));
        }
    }
    let cleanup = unique(
        lines
            .iter()
            .enumerate()
            .filter_map(|(i, l)| l.starts_with("DOOR_P08 CLEANUP ").then_some(i))
            .collect(),
    )?;
    let count = apps + 5 + hosts.len() + imes.len() + nets.len();
    let c = fields(
        lines[cleanup]
            .strip_prefix("DOOR_P08 CLEANUP PASS ")
            .ok_or("Files cleanup failure")?,
    )?;
    if c != BTreeMap::from([
        ("processes", count as i64),
        ("threads", count as i64),
        ("leaked", 0),
    ]) || processes.len() != count
        || applications != apps
        || retired_hosts != hosts.len()
        || retired_imes != imes.len()
        || retired_nets != nets.len()
        || service_lines.len() != 5
    {
        return Err("Files exact process/thread retirement differs".into());
    }
    let init = processes
        .iter()
        .find(|(_, f)| f["manifest"] == 1)
        .ok_or("Files Init identity")?
        .1["pid"];
    let mut launched = BTreeMap::new();
    let mut launch_lines = BTreeMap::new();
    for (i, line) in lines.iter().enumerate() {
        if let Some((actor, payload)) = user(line)
            && let Some(value) = payload.strip_prefix("DOOR_P12 APP_LAUNCHED ")
        {
            let f = fields(value)?;
            if actor != init
                || f.keys().copied().collect::<BTreeSet<_>>() != BTreeSet::from(["app", "pid"])
                || !commands.contains(&f["app"])
                || launched.insert(f["app"], f["pid"]).is_some()
                || !processes.iter().any(|(end, p)| {
                    *end > i
                        && Some(p["manifest"]) == expected_manifest(f["app"])
                        && p["pid"] == f["pid"]
                })
            {
                return Err("Files launch/retirement authority mismatch".into());
            }
            launch_lines.insert(f["app"], i);
        }
    }
    if launched.len() != apps || launched.values().copied().collect::<BTreeSet<_>>().len() != apps {
        return Err("Files unique app launches missing".into());
    }
    let files_retired = processes
        .iter()
        .find(|(_, f)| f["pid"] == launched[&commands[0]])
        .ok_or("Files retired identity missing")?
        .0;
    for (i, line) in lines.iter().enumerate() {
        if let Some((actor, payload)) = user(line)
            && payload.starts_with("DOOR_P12 FILES REMOVE")
            && (!commands.contains(&1)
                || actor != launched[&1]
                || i <= launch_lines[&1]
                || i >= files_retired)
        {
            return Err("Files removal marker from another process".into());
        }
    }
    let device = unique(
        lines
            .iter()
            .enumerate()
            .filter_map(|(i, l)| l.starts_with("DOOR_P08 DEVICE ").then_some(i))
            .collect(),
    )?;
    let ordered = [
        service_lines[&11],
        service_lines[&4],
        service_lines[&3],
        service_lines[&2],
        service_lines[&1],
        cleanup,
        device,
    ];
    if ordered.windows(2).any(|x| x[0] >= x[1])
        || processes.iter().any(|(i, f)| {
            *i >= cleanup
                || (matches!(f["manifest"], 12 | 13 | 14 | 16) && *i >= service_lines[&11])
        })
    {
        return Err("Files retirement kernel order differs".into());
    }
    Ok(
        json!({"processes":count,"threads":count,"leaked":0,"device_reclaimed":8,"applications":launched,"identities":processes.iter().map(|(i,f)|json!({"line":i,"fields":f})).collect::<Vec<_>>(),"cleanup_line":cleanup,"device_line":device}),
    )
}
pub(super) fn canonical_for(text: &str, count: i64) -> Result<Value, String> {
    let hosts = search_hosts(text);
    let imes = ime_hosts(text);
    let count = count + hosts.len() as i64 + imes.len() as i64 + network_hosts(text).len() as i64;
    let lines: Vec<_> = text.lines().collect();
    if lines.iter().any(|l| {
        user(l).is_some_and(|(_, p)| {
            p.starts_with("DOOR_P08 PROCESS ")
                || p.starts_with("DOOR_P08 CLEANUP ")
                || p.starts_with("DOOR_P08 DEVICE ")
        })
    }) {
        return Err("user output cannot attest kernel shutdown records".into());
    }
    let mut processes = Vec::new();
    let mut pids = BTreeSet::new();
    let mut instances = BTreeSet::new();
    for (line, value) in lines.iter().enumerate() {
        if let Some(value) = value.strip_prefix("DOOR_P08 PROCESS ") {
            let f = fields(value)?;
            let keys: BTreeSet<_> = f.keys().copied().collect();
            if keys
                != BTreeSet::from([
                    "manifest", "pid", "instance", "uid", "session", "epoch", "exit",
                ])
                || f["pid"] <= 0
                || f["instance"] <= 0
                || !pids.insert(f["pid"])
                || !instances.insert(f["instance"])
            {
                return Err("invalid/duplicate kernel shutdown identity".into());
            }
            processes.push((line, f));
        }
    }
    let process = |manifest, uid, exit| -> Result<_, String> {
        let found: Vec<_> = processes
            .iter()
            .filter(|(_, f)| f["manifest"] == manifest && f["uid"] == uid && f["exit"] == exit)
            .collect();
        if found.len() != 1 {
            return Err("shutdown service identity is not unique".into());
        }
        let (line, f) = found[0];
        if f["session"] != 0 || f["epoch"] != 0 {
            return Err("service shutdown session differs".into());
        }
        Ok((*line, f))
    };
    let (init_line, init) = process(1, 1, 80)?;
    let (window_line, window) = process(11, 3, -1)?;
    // The search host retires before the display service it answers.
    for host in &hosts {
        let retired = processes.iter().find(|(_, f)| f["pid"] == *host);
        if !retired.is_some_and(|(line, f)| {
            f["manifest"] == SEARCH_HOST_MANIFEST
                && f["uid"] == 1001
                && f["exit"] == SEARCH_HOST_EXIT
                && f["session"] > 0
                && f["epoch"] > 0
                && *line < window_line
        }) {
            return Err("search host retirement differs".into());
        }
    }
    // So does the session IME.
    for ime in &imes {
        let retired = processes.iter().find(|(_, f)| f["pid"] == *ime);
        if !retired.is_some_and(|(line, f)| {
            f["manifest"] == 16
                && f["uid"] == 1001
                && f["exit"] == IME_EXIT
                && f["session"] > 0
                && f["epoch"] > 0
                && *line < window_line
        }) {
            return Err("IME retirement differs".into());
        }
    }
    // Shutdown with a dirty document (Windows 11): the app asks; Save or
    // Don't save closes it and the shutdown finishes by itself; Cancel
    // cancels the shutdown and the session stays. Forms:
    // - "save": one request, one Close, prompt, Save, the shutdown finishes;
    // - "cancel-then-save": a cancelled request, then the "save" form;
    // - "legacy-two-requests": retained evidence from before this behaviour,
    //   where the prompt itself cancelled the request and the app's own
    //   close after Save was logged as a second Close request.
    let prompts: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            user(l)
                .filter(|(_, p)| *p == "DOOR_P12 NOTES PROMPT intent=Close")
                .map(|_| i)
        })
        .collect();
    let first_prompt = *prompts.first().ok_or("document Close prompt missing")?;
    let pid = user(lines[first_prompt]).ok_or("prompt actor")?.0;
    if prompts
        .iter()
        .any(|i| user(lines[*i]).map(|v| v.0) != Some(pid))
    {
        return Err("shutdown prompts come from different documents".into());
    }
    let (retired, notes) = processes
        .iter()
        .find(|(_, f)| f["pid"] == pid)
        .ok_or("document kernel retirement missing")?;
    if notes["manifest"] != 12
        || notes["uid"] != 1001
        || notes["exit"] != 0
        || notes["session"] <= 0
        || notes["epoch"] <= 0
    {
        return Err("document shutdown authenticated identity differs".into());
    }
    let all = |actor: i64, payload: &str| -> Vec<usize> {
        lines
            .iter()
            .enumerate()
            .filter_map(|(i, l)| (user(l) == Some((actor, payload))).then_some(i))
            .collect()
    };
    let event =
        |actor: i64, payload: &str| -> Result<usize, String> { unique(all(actor, payload)) };
    let launch = event(
        init["pid"],
        &format!("DOOR_P12 APP_LAUNCHED app=2 pid={pid}"),
    )?;
    let saved = event(pid, "DOOR_P11 NOTES SAVED")?;
    let done = event(pid, "DOOR_P12 NOTES PROMPT_DONE intent=Close choice=0")?;
    let exit = event(
        init["pid"],
        &format!("DOOR_P11 CLIENT_EXIT pid={pid} code=0"),
    )?;
    let mut closes = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if let Some((actor, payload)) = user(l)
            && let Some(value) = payload.strip_prefix("DOOR_P12 CLOSE REQUEST ")
        {
            let f = fields(value)?;
            if f.get("pid") == Some(&pid) {
                if actor != window["pid"] || f.len() != 2 || !f.contains_key("at") {
                    return Err("close requester differs from retired Window identity".into());
                }
                closes.push(i);
            }
        }
    }
    let cancels = all(pid, "DOOR_P12 NOTES PROMPT_CANCEL intent=Close");
    let cancelled = all(
        window["pid"],
        &format!("DOOR_P12 SHUTDOWN CANCELLED pid={pid}"),
    );
    let (form, mut ordered) = match (prompts.len(), cancels.len(), cancelled.len(), closes.len()) {
        (1, 0, 0, 1) => ("save", vec![launch, closes[0], prompts[0], saved, done]),
        (2, 1, 1, 2) => (
            "cancel-then-save",
            vec![
                launch,
                closes[0],
                prompts[0],
                cancels[0],
                cancelled[0],
                closes[1],
                prompts[1],
                saved,
                done,
            ],
        ),
        (1, 0, 0, 2) => (
            "legacy-two-requests",
            vec![launch, closes[0], prompts[0], saved, done, closes[1]],
        ),
        _ => return Err("document shutdown prompt/Close/cancel sequence differs".into()),
    };
    let cleanup = unique(
        lines
            .iter()
            .enumerate()
            .filter_map(|(i, l)| l.starts_with("DOOR_P08 CLEANUP ").then_some(i))
            .collect(),
    )?;
    let c = fields(
        lines[cleanup]
            .strip_prefix("DOOR_P08 CLEANUP PASS ")
            .ok_or("cleanup failure")?,
    )?;
    if c != BTreeMap::from([("processes", count), ("threads", count), ("leaked", 0)])
        || processes.len() != count as usize
    {
        return Err("wide shutdown exact retirement counts differ".into());
    }
    let device = unique(
        lines
            .iter()
            .enumerate()
            .filter_map(|(i, l)| l.starts_with("DOOR_P08 DEVICE ").then_some(i))
            .collect(),
    )?;
    let d = fields(
        lines[device]
            .strip_prefix("DOOR_P08 DEVICE PASS ")
            .ok_or("device failure")?,
    )?;
    if d.keys().copied().collect::<BTreeSet<_>>()
        != BTreeSet::from([
            "reads",
            "writes",
            "flushes",
            "sampled_remaining",
            "reclaimed",
        ])
        || d["reclaimed"] != 8
        || d.values().any(|v| *v < 0)
    {
        return Err("wide device reclamation differs".into());
    }
    ordered.extend([*retired, exit, window_line, init_line, cleanup, device]);
    if ordered.windows(2).any(|w| w[0] >= w[1]) || processes.iter().any(|(i, _)| *i >= cleanup) {
        return Err("shutdown Save/retirement events are out of order".into());
    }
    let identity = |f: &BTreeMap<&str, i64>| {
        json!({
            "pid":f["pid"],"instance":f["instance"],"manifest":f["manifest"],
            "uid":f["uid"],"session":f["session"],"epoch":f["epoch"],"exit":f["exit"]
        })
    };
    Ok(
        json!({"schema_version":1,"decision":"Save","form":form,"notes":identity(notes),
        "init":identity(init),"window":identity(window),
        "ordered_line_numbers":ordered.iter().map(|i| i + 1).collect::<Vec<_>>(),
        "client_exit_line":exit + 1,"window_exit_line":window_line + 1,
        "retired_processes":count,"retired_threads":count,"leaked":0,"device_reclaimed":8,
        "power_gestures":"controller-only; not derivable from guest serial"}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_session_ime_is_one_more_retired_process_before_the_window_service() {
        let mut s = String::from(
            "DOOR_USER pid=1 DOOR_P12 IME LAUNCHED pid=7\nDOOR_USER pid=1 DOOR_P12 APP_LAUNCHED app=9 pid=6\nDOOR_P08 PROCESS manifest=12 pid=6 instance=6 uid=1001 session=1 epoch=1 exit=0\nDOOR_P08 PROCESS manifest=16 pid=7 instance=7 uid=1001 session=1 epoch=1 exit=-151\n",
        );
        for (manifest, pid, uid, exit) in [
            (11, 5, 3, -1),
            (4, 4, 3, 0),
            (3, 3, 2, 0),
            (2, 2, 1, 0),
            (1, 1, 1, 80),
        ] {
            s.push_str(&format!("DOOR_P08 PROCESS manifest={manifest} pid={pid} instance={pid} uid={uid} session=0 epoch=0 exit={exit}\n"));
        }
        s.push_str("DOOR_P08 CLEANUP PASS processes=7 threads=7 leaked=0\nDOOR_P08 DEVICE PASS reads=1 writes=1 flushes=1 sampled_remaining=1 reclaimed=8\n");
        assert_eq!(ime_hosts(&s), BTreeSet::from([7]));
        assert!(retirement_commands(&s, &[9]).is_ok());
        assert!(
            retirement_commands(&s.replace("exit=-151", "exit=0"), &[9]).is_err(),
            "IME exit code"
        );
        assert!(
            retirement_commands(&s.replace("IME LAUNCHED pid=7", "IME LAUNCHED pid=8"), &[9])
                .is_err(),
            "unlaunched manifest 16"
        );
    }
    #[test]
    fn terminal_retirement_requires_explicit_command_nine_and_kernel_identity() {
        let mut s = String::from(
            "DOOR_USER pid=1 DOOR_P12 APP_LAUNCHED app=9 pid=6\nDOOR_P08 PROCESS manifest=12 pid=6 instance=6 uid=1001 session=1 epoch=1 exit=0\n",
        );
        for (manifest, pid, uid, exit) in [
            (11, 5, 3, -1),
            (4, 4, 3, 0),
            (3, 3, 2, 0),
            (2, 2, 1, 0),
            (1, 1, 1, 80),
        ] {
            s.push_str(&format!("DOOR_P08 PROCESS manifest={manifest} pid={pid} instance={pid} uid={uid} session=0 epoch=0 exit={exit}\n"));
        }
        s.push_str("DOOR_P08 CLEANUP PASS processes=6 threads=6 leaked=0\nDOOR_P08 DEVICE PASS reads=1 writes=1 flushes=1 sampled_remaining=1 reclaimed=8\n");
        assert!(retirement_commands(&s, &[9]).is_ok());
        assert!(retirement(&s, 1).is_err());
        assert!(retirement_commands(&s.replace("app=9", "app=1"), &[9]).is_err());
        assert!(
            retirement_commands(&s.replace("DOOR_USER pid=1", "DOOR_USER pid=2"), &[9]).is_err()
        );
        assert!(retirement_commands(&s.replace("instance=6", "instance=5"), &[9]).is_err());
        assert!(retirement_commands(&s, &[9, 9]).is_err());
        let preview = s
            .replace("app=9", "app=10")
            .replace("manifest=12", "manifest=13");
        assert!(retirement_commands(&preview, &[10]).is_ok());
        assert!(
            retirement_commands(&preview.replace("manifest=13", "manifest=12"), &[10]).is_err()
        );
        assert!(retirement_commands(&s.replace("manifest=12", "manifest=13"), &[9]).is_err());
        assert!(retirement_commands(&preview, &[12]).is_err());
        // Init's windowless search host: one more workspace process, retired
        // with its own exit code before the display service.
        let host = s
            .replace(
                "DOOR_USER pid=1 DOOR_P12 APP_LAUNCHED",
                "DOOR_USER pid=1 DOOR_P12 SEARCH_HOST LAUNCHED pid=7\nDOOR_USER pid=1 DOOR_P12 APP_LAUNCHED",
            )
            .replace(
                "DOOR_P08 PROCESS manifest=11",
                "DOOR_P08 PROCESS manifest=19 pid=7 instance=7 uid=1001 session=1 epoch=1 exit=-150\nDOOR_P08 PROCESS manifest=11",
            )
            .replace("processes=6 threads=6", "processes=7 threads=7");
        assert_eq!(search_hosts(&host), BTreeSet::from([7]));
        assert!(retirement_commands(&host, &[9]).is_ok());
        assert!(retirement_commands(&host.replace("exit=-150", "exit=0"), &[9]).is_err());
        assert!(
            retirement_commands(
                &host.replace("processes=7 threads=7", "processes=6 threads=6"),
                &[9]
            )
            .is_err()
        );
        // A marker from any process but Init's output is still a user line;
        // without the kernel record the counts no longer match.
        assert!(
            retirement_commands(
                &s.replace(
                    "processes=6 threads=6",
                    "processes=6 threads=6\nDOOR_USER pid=1 DOOR_P12 SEARCH_HOST LAUNCHED pid=9"
                ),
                &[9]
            )
            .is_err()
        );
    }
    #[test]
    fn files_retirement_requires_exact_authenticated_apps_services_and_order() {
        let good = "DOOR_USER pid=1 DOOR_P12 APP_LAUNCHED app=1 pid=6\nDOOR_USER pid=6 DOOR_P12 FILES REMOVED object=32 version=4\nDOOR_P08 PROCESS manifest=12 pid=6 instance=6 uid=1001 session=1 epoch=1 exit=0\nDOOR_P08 PROCESS manifest=11 pid=5 instance=5 uid=3 session=0 epoch=0 exit=-1\nDOOR_P08 PROCESS manifest=4 pid=4 instance=4 uid=3 session=0 epoch=0 exit=0\nDOOR_P08 PROCESS manifest=3 pid=3 instance=3 uid=2 session=0 epoch=0 exit=0\nDOOR_P08 PROCESS manifest=2 pid=2 instance=2 uid=1 session=0 epoch=0 exit=0\nDOOR_P08 PROCESS manifest=1 pid=1 instance=1 uid=1 session=0 epoch=0 exit=80\nDOOR_P08 CLEANUP PASS processes=6 threads=6 leaked=0\nDOOR_P08 DEVICE PASS reads=2 writes=3 flushes=4 sampled_remaining=500 reclaimed=8\n";
        assert!(retirement(good, 1).is_ok());
        assert!(retirement(good, 3).is_err());
        let marker = "DOOR_USER pid=6 DOOR_P12 FILES REMOVED object=32 version=4\n";
        let no_marker = good.replace(marker, "");
        assert!(retirement(&format!("{marker}{no_marker}"), 1).is_err());
        assert!(
            retirement(
                &no_marker.replace(
                    "DOOR_P08 PROCESS manifest=11",
                    &format!("{marker}DOOR_P08 PROCESS manifest=11")
                ),
                1
            )
            .is_err()
        );
        for (a, b) in [
            ("threads=6", "threads=7"),
            ("uid=1001", "uid=1002"),
            ("epoch=1", "epoch=01"),
            ("instance=6", "instance=5"),
            ("pid=1 DOOR_P12 APP_LAUNCHED", "pid=5 DOOR_P12 APP_LAUNCHED"),
            (
                "pid=6 DOOR_P12 FILES REMOVED",
                "pid=5 DOOR_P12 FILES REMOVED",
            ),
            ("app=1 pid=6", "app=1 pid=5"),
            ("leaked=0", "leaked=1"),
            ("reclaimed=8", "reclaimed=7"),
            (
                "DOOR_P08 PROCESS manifest=2",
                "DOOR_USER pid=6 DOOR_P08 PROCESS manifest=2",
            ),
        ] {
            assert!(retirement(&good.replace(a, b), 1).is_err(), "{b}");
        }
        let order = good.replace(
            "DOOR_P08 PROCESS manifest=2 pid=2 instance=2 uid=1 session=0 epoch=0 exit=0\n",
            "",
        );
        assert!(retirement(&format!("DOOR_P08 PROCESS manifest=2 pid=2 instance=2 uid=1 session=0 epoch=0 exit=0\n{order}"),1).is_err());
    }
    #[test]
    fn sealing_requires_complete_kernel_device_record_after_cleanup() {
        let good = fixture();
        assert!(device_complete(&good));
        assert!(device_complete(&good.replace('\n', "\r\n")));
        for bad in [
            good.trim_end().to_string(),
            good.replace("reclaimed=8", "reclaimed="),
            good.replace("reclaimed=8", "reclaimed=7"),
            good.replace(
                "DOOR_P08 DEVICE PASS",
                "DOOR_USER pid=1 DOOR_P08 DEVICE PASS",
            ),
            good.replace("reads=2", "reads=02"),
            format!("{good}later output\n"),
        ] {
            assert!(!device_complete(&bad), "{bad}");
        }
    }
    /// Wide shutdown (Windows 11): the first request is cancelled from the
    /// document's prompt, the second is answered Save and finishes alone.
    fn fixture() -> String {
        with_document(
            "DOOR_USER pid=1 DOOR_P12 APP_LAUNCHED app=2 pid=17\nDOOR_USER pid=5 DOOR_P12 CLOSE REQUEST pid=17 at=100\nDOOR_USER pid=17 DOOR_P12 NOTES PROMPT intent=Close\nDOOR_USER pid=17 DOOR_P12 NOTES PROMPT_CANCEL intent=Close\nDOOR_USER pid=5 DOOR_P12 SHUTDOWN CANCELLED pid=17\nDOOR_USER pid=5 DOOR_P12 CLOSE REQUEST pid=17 at=200\nDOOR_USER pid=17 DOOR_P12 NOTES PROMPT intent=Close\nDOOR_USER pid=17 DOOR_P11 NOTES SAVED\nDOOR_USER pid=17 DOOR_P12 NOTES PROMPT_DONE intent=Close choice=0\nDOOR_P08 PROCESS manifest=12 pid=17 instance=17 uid=1001 session=1 epoch=1 exit=0\nDOOR_USER pid=1 DOOR_P11 CLIENT_EXIT pid=17 code=0\n",
        )
    }
    /// One request, Save, and the shutdown finishes by itself.
    fn save_fixture() -> String {
        with_document(
            "DOOR_USER pid=1 DOOR_P12 APP_LAUNCHED app=2 pid=17\nDOOR_USER pid=5 DOOR_P12 CLOSE REQUEST pid=17 at=100\nDOOR_USER pid=17 DOOR_P12 NOTES PROMPT intent=Close\nDOOR_USER pid=17 DOOR_P11 NOTES SAVED\nDOOR_USER pid=17 DOOR_P12 NOTES PROMPT_DONE intent=Close choice=0\nDOOR_P08 PROCESS manifest=12 pid=17 instance=17 uid=1001 session=1 epoch=1 exit=0\nDOOR_USER pid=1 DOOR_P11 CLIENT_EXIT pid=17 code=0\n",
        )
    }
    /// Retained evidence from before: the prompt cancelled the request and
    /// the document's own close after Save was a second logged request.
    fn legacy_fixture() -> String {
        with_document(
            "DOOR_USER pid=1 DOOR_P12 APP_LAUNCHED app=2 pid=17\nDOOR_USER pid=5 DOOR_P12 CLOSE REQUEST pid=17 at=100\nDOOR_USER pid=17 DOOR_P12 NOTES PROMPT intent=Close\nDOOR_USER pid=17 DOOR_P11 NOTES SAVED\nDOOR_USER pid=17 DOOR_P12 NOTES PROMPT_DONE intent=Close choice=0\nDOOR_USER pid=5 DOOR_P12 CLOSE REQUEST pid=17 at=110\nDOOR_P08 PROCESS manifest=12 pid=17 instance=17 uid=1001 session=1 epoch=1 exit=0\nDOOR_USER pid=1 DOOR_P11 CLIENT_EXIT pid=17 code=0\n",
        )
    }
    fn with_document(document: &str) -> String {
        let mut s = String::from(document);
        for pid in 2..=18 {
            if pid != 5 && pid != 17 {
                s.push_str(&format!("DOOR_P08 PROCESS manifest=12 pid={pid} instance={pid} uid=1001 session=1 epoch=1 exit=0\n"));
            }
        }
        s.push_str("DOOR_P08 PROCESS manifest=11 pid=5 instance=5 uid=3 session=0 epoch=0 exit=-1\nDOOR_P08 PROCESS manifest=1 pid=1 instance=1 uid=1 session=0 epoch=0 exit=80\nDOOR_P08 CLEANUP PASS processes=18 threads=18 leaked=0\nDOOR_P08 DEVICE PASS reads=2 writes=3 flushes=4 sampled_remaining=500 reclaimed=8\n");
        s
    }
    #[test]
    fn shutdown_save_requires_authenticated_unique_ordered_events() {
        let s = fixture();
        let canonical = canonical(&s).unwrap();
        assert_eq!(canonical["notes"]["pid"], 17);
        for (from, to) in [
            ("pid=17", "pid=+17"),
            ("pid=17", "pid=017"),
            ("epoch=1", "epoch=01"),
            ("session=0", "session=-0"),
        ] {
            assert!(
                super::canonical(&s.replace(from, to)).is_err(),
                "noncanonical {to}"
            );
        }
        for (from, to) in [
            (
                "DOOR_USER pid=1 DOOR_P12 APP_LAUNCHED",
                "DOOR_USER pid=5 DOOR_P12 APP_LAUNCHED",
            ),
            (
                "DOOR_USER pid=5 DOOR_P12 CLOSE REQUEST",
                "DOOR_USER pid=1 DOOR_P12 CLOSE REQUEST",
            ),
            (
                "uid=1001 session=1 epoch=1 exit=0",
                "uid=1002 session=1 epoch=1 exit=0",
            ),
            ("pid=17 instance=17", "pid=17 instance=0"),
            (
                "pid=17 instance=17 uid=1001 session=1",
                "pid=17 instance=17 uid=1001 session=0",
            ),
            (
                "pid=17 instance=17 uid=1001 session=1 epoch=1",
                "pid=17 instance=17 uid=1001 session=1 epoch=0",
            ),
            (
                "DOOR_USER pid=17 DOOR_P11 NOTES SAVED",
                "DOOR_USER pid=99 DOOR_P11 NOTES SAVED",
            ),
            ("choice=0", "choice=1"),
            ("processes=18", "processes=17"),
            ("reclaimed=8", "reclaimed=7"),
            (
                "DOOR_P08 PROCESS manifest=12 pid=17",
                "DOOR_USER pid=17 DOOR_P08 PROCESS manifest=12 pid=17",
            ),
            (
                "DOOR_P08 DEVICE PASS",
                "DOOR_USER pid=1 DOOR_P08 DEVICE PASS",
            ),
        ] {
            assert!(super::canonical(&s.replace(from, to)).is_err(), "{from}");
        }
        for line in s
            .lines()
            .filter(|l| l.contains("pid=17") || l.contains("CLEANUP") || l.contains("DEVICE"))
        {
            assert!(
                super::canonical(&s.replace(&format!("{line}\n"), "")).is_err(),
                "missing {line}"
            );
            assert!(
                super::canonical(&format!("{s}{line}\n")).is_err(),
                "duplicate {line}"
            );
        }
        let a = "DOOR_USER pid=17 DOOR_P11 NOTES SAVED";
        let b = "DOOR_USER pid=17 DOOR_P12 NOTES PROMPT_DONE intent=Close choice=0";
        assert!(super::canonical(&s.replace(a, "TEMP").replace(b, a).replace("TEMP", b)).is_err());
    }
    #[test]
    fn shutdown_forms_save_cancel_then_save_and_retained_legacy() {
        assert_eq!(canonical(&fixture()).unwrap()["form"], "cancel-then-save");
        assert_eq!(canonical(&save_fixture()).unwrap()["form"], "save");
        assert_eq!(
            canonical(&legacy_fixture()).unwrap()["form"],
            "legacy-two-requests"
        );
        let c = canonical(&save_fixture()).unwrap();
        let text = save_fixture();
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            lines[c["client_exit_line"].as_u64().unwrap() as usize - 1]
                .contains("CLIENT_EXIT pid=17")
        );
        assert!(
            lines[c["window_exit_line"].as_u64().unwrap() as usize - 1].contains("manifest=11")
        );
        // A cancellation without the shell cancelling the shutdown, or a
        // shell cancellation without the user's Cancel, is not either form.
        let f = fixture();
        assert!(
            canonical(&f.replace("DOOR_USER pid=5 DOOR_P12 SHUTDOWN CANCELLED pid=17\n", ""))
                .is_err()
        );
        assert!(
            canonical(&f.replace(
                "DOOR_USER pid=17 DOOR_P12 NOTES PROMPT_CANCEL intent=Close\n",
                ""
            ))
            .is_err()
        );
        assert!(
            canonical(&f.replace("SHUTDOWN CANCELLED pid=17", "SHUTDOWN CANCELLED pid=18"))
                .is_err()
        );
        assert!(
            canonical(&f.replace(
                "DOOR_USER pid=5 DOOR_P12 SHUTDOWN",
                "DOOR_USER pid=17 DOOR_P12 SHUTDOWN"
            ))
            .is_err()
        );
        // The Save attempt has exactly one Close: a second request after the
        // cancel (the old forced re-request) is rejected.
        assert!(canonical(&f.replace(
            "DOOR_USER pid=17 DOOR_P12 NOTES PROMPT_DONE intent=Close choice=0\n",
            "DOOR_USER pid=17 DOOR_P12 NOTES PROMPT_DONE intent=Close choice=0\nDOOR_USER pid=5 DOOR_P12 CLOSE REQUEST pid=17 at=300\n",
        )).is_err());
        // Cancel must come before the second request.
        let swapped = f
            .replace("DOOR_USER pid=5 DOOR_P12 CLOSE REQUEST pid=17 at=200\n", "")
            .replace("DOOR_USER pid=17 DOOR_P12 NOTES PROMPT_CANCEL intent=Close\n", "DOOR_USER pid=5 DOOR_P12 CLOSE REQUEST pid=17 at=200\nDOOR_USER pid=17 DOOR_P12 NOTES PROMPT_CANCEL intent=Close\n");
        assert!(canonical(&swapped).is_err());
    }
    #[test]
    fn protocol_removal_cannot_bypass_choice_and_summary_is_compared_to_guest() {
        assert!(required(&json!({}), &[], &fixture()));
        assert!(required(
            &json!({}),
            &[json!({"id":"desktop-shutdown-save-dialog"})],
            ""
        ));
        assert!(required(&json!({"document_close_protocol":2}), &[], ""));
        assert!(!required(&json!({}), &[], "historical no prompt"));
        let c = canonical(&fixture()).unwrap();
        let summary = json!({"notes_pid":17,"decision":"Save","power_requests":2,
            "first_request_canceled":true,"fresh_close_prompt":true,"fresh_client_exit":true});
        validate_summary(&summary, &c).unwrap();
        for (key, value) in [
            ("notes_pid", json!(99)),
            ("decision", json!("Don't save")),
            ("fresh_close_prompt", json!(false)),
            ("fresh_client_exit", json!(false)),
        ] {
            let mut bad = summary.clone();
            bad[key] = value;
            assert!(validate_summary(&bad, &c).is_err());
        }
    }
}
