//! `schtasks` over the Task Scheduler (the host's `tasks` methods). Output
//! follows Windows' schtasks.exe.
use super::Cmd;
use super::builtin::{Outcome, unq};
use crate::host::{Host, HostTask};
use alloc::{format, string::String, vec::Vec};

fn fail(cmd: &mut Cmd, host: &mut dyn Host, text: &str) -> Outcome {
    cmd.err_line(host, text);
    cmd.errorlevel = 1;
    Outcome::Done(false)
}
fn ok(cmd: &mut Cmd, host: &mut dyn Host, text: &str) -> Outcome {
    cmd.out_line(host, text);
    cmd.errorlevel = 0;
    Outcome::Done(true)
}
/// `/key value` options (keys lower-cased); flags have an empty value.
fn options(args: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let key = args[i].to_ascii_lowercase();
        let takes = matches!(
            key.as_str(),
            "/tn"
                | "/tr"
                | "/sc"
                | "/mo"
                | "/st"
                | "/sd"
                | "/fo"
                | "/ru"
                | "/rl"
                | "/s"
                | "/u"
                | "/p"
        );
        let value = if takes {
            i += 1;
            args.get(i).map(|v| unq(v)).unwrap_or_default()
        } else {
            String::new()
        };
        out.push((key, value));
        i += 1;
    }
    out
}
/// "11/7/2026 10:05:00 AM" from local minutes since 1601, or "N/A".
fn when(local: Option<(i64, u32, u32, u32, u32, u32)>) -> String {
    match local {
        Some((y, mo, d, h, mi, s)) => {
            let (h12, ampm) = match h {
                0 => (12, "AM"),
                1..=11 => (h, "AM"),
                12 => (12, "PM"),
                _ => (h - 12, "PM"),
            };
            format!("{mo}/{d}/{y} {h12}:{mi:02}:{s:02} {ampm}")
        }
        None => String::from("N/A"),
    }
}
fn state(t: &HostTask) -> &'static str {
    match t.state {
        2 => "Running",
        3 => "Disabled",
        _ => "Ready",
    }
}
pub(crate) fn schtasks(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let o = options(args);
    let has = |k: &str| o.iter().any(|(key, _)| key == k);
    let get = |k: &str| o.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
    let verb = o.first().map(|(k, _)| k.clone()).unwrap_or_default();
    match verb.as_str() {
        "" | "/query" => {
            let tasks = match host.tasks() {
                Ok(t) => t,
                Err(_) => {
                    return fail(
                        cmd,
                        host,
                        "ERROR: The Task Scheduler service is not available.",
                    );
                }
            };
            let filtered: Vec<&HostTask> = match get("/tn") {
                Some(n) => tasks
                    .iter()
                    .filter(|t| t.name.eq_ignore_ascii_case(n.trim_start_matches('\\')))
                    .collect(),
                None => tasks.iter().collect(),
            };
            if get("/tn").is_some() && filtered.is_empty() {
                return fail(
                    cmd,
                    host,
                    "ERROR: The system cannot find the file specified.",
                );
            }
            let list = get("/fo").is_some_and(|f| f.eq_ignore_ascii_case("LIST"));
            let mut out = String::new();
            if list {
                for t in &filtered {
                    out.push_str(&format!(
                        "\nFolder: \\\nHostName:      {}\nTaskName:      \\{}\nNext Run Time: {}\nStatus:        {}\nLogon Mode:    Interactive only\n",
                        host.computer_name(),
                        t.name,
                        when(host.local_time(t.next_run)),
                        state(t)
                    ));
                }
            } else {
                out.push_str("\nFolder: \\\nTaskName                                 Next Run Time          Status\n======================================== ====================== ===============\n");
                if filtered.is_empty() {
                    out.push_str("INFO: There are no scheduled tasks presently available at your access level.\n");
                }
                for t in &filtered {
                    out.push_str(&format!(
                        "{:<40} {:<22} {}\n",
                        t.name,
                        when(host.local_time(t.next_run)),
                        state(t)
                    ));
                }
            }
            ok(cmd, host, &out)
        }
        "/create" => {
            let (Some(name), Some(command)) = (get("/tn"), get("/tr")) else {
                return fail(
                    cmd,
                    host,
                    "ERROR: Invalid syntax. Mandatory option '/tn' or '/tr' is missing.\nType \"SCHTASKS /CREATE /?\" for usage.",
                );
            };
            let Some(schedule) = get("/sc") else {
                return fail(
                    cmd,
                    host,
                    "ERROR: Invalid syntax. Mandatory option '/sc' is missing.\nType \"SCHTASKS /CREATE /?\" for usage.",
                );
            };
            let modifier = get("/mo").and_then(|m| m.parse::<u16>().ok()).unwrap_or(1);
            let kind = match schedule.to_ascii_uppercase().as_str() {
                "ONCE" => 0,
                "MINUTE" => 1,
                "HOURLY" => 2,
                "DAILY" => 3,
                "ONLOGON" => 4,
                _ => {
                    return fail(cmd, host, "ERROR: Invalid value for /SC option.");
                }
            };
            if kind == 0 && get("/st").is_none() {
                return fail(
                    cmd,
                    host,
                    "ERROR: Invalid Start Time. Start time must be specified for ONCE schedule.",
                );
            }
            let start = get("/st").map(|st| {
                let (h, m) = st.split_once(':').unwrap_or((&st, "0"));
                (h.parse::<u32>().unwrap_or(0), m.parse::<u32>().unwrap_or(0))
            });
            let name = String::from(name.trim_start_matches('\\'));
            match host.task_create(&name, &command, kind, modifier, start, has("/f")) {
                Ok(()) => ok(
                    cmd,
                    host,
                    &format!(
                        "SUCCESS: The scheduled task \"{name}\" has successfully been created."
                    ),
                ),
                Err(183) => fail(
                    cmd,
                    host,
                    "ERROR: Cannot create a file when that file already exists.",
                ),
                Err(5) => fail(cmd, host, "ERROR: Access is denied."),
                Err(_) => fail(cmd, host, "ERROR: The task could not be created."),
            }
        }
        "/delete" | "/run" | "/end" | "/change" => {
            let Some(name) = get("/tn") else {
                return fail(
                    cmd,
                    host,
                    &format!(
                        "ERROR: Invalid syntax. Mandatory option '/tn' is missing.\nType \"SCHTASKS {} /?\" for usage.",
                        verb.to_ascii_uppercase()
                    ),
                );
            };
            let name = String::from(name.trim_start_matches('\\'));
            let result = match verb.as_str() {
                "/delete" => host.task_delete(&name),
                "/run" => host.task_run(&name),
                "/end" => host.task_end(&name),
                _ if has("/disable") => host.task_enable(&name, false),
                _ if has("/enable") => host.task_enable(&name, true),
                _ => {
                    return fail(
                        cmd,
                        host,
                        "ERROR: Invalid syntax. Specify /ENABLE or /DISABLE.",
                    );
                }
            };
            match result {
                Ok(()) => ok(
                    cmd,
                    host,
                    &match verb.as_str() {
                        "/delete" => format!(
                            "SUCCESS: The scheduled task \"{name}\" was successfully deleted."
                        ),
                        "/run" => {
                            format!("SUCCESS: Attempted to run the scheduled task \"{name}\".")
                        }
                        "/end" => format!(
                            "SUCCESS: The scheduled task \"{name}\" has been terminated successfully."
                        ),
                        _ => format!(
                            "SUCCESS: The parameters of scheduled task \"{name}\" have been changed."
                        ),
                    },
                ),
                Err(2) => fail(
                    cmd,
                    host,
                    "ERROR: The system cannot find the file specified.",
                ),
                Err(5) => fail(cmd, host, "ERROR: Access is denied."),
                Err(_) => fail(cmd, host, "ERROR: The operation failed."),
            }
        }
        _ => fail(
            cmd,
            host,
            &format!(
                "ERROR: Invalid argument/option - '{}'.\nType \"SCHTASKS /?\" for usage.",
                args.first().map_or("", |a| a.as_str())
            ),
        ),
    }
}
