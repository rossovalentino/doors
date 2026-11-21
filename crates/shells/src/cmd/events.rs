//! `eventcreate` and `wevtutil` over the Event Log (the host's `events`
//! methods). Output follows Windows' eventcreate.exe and wevtutil.exe.
use super::Cmd;
use super::builtin::{Outcome, unq};
use crate::host::{Host, HostEvent};
use alloc::{format, string::String, vec::Vec};

/// EVENTLOG_*_TYPE by eventcreate's /T names.
const TYPES: [(&str, u16); 5] = [
    ("ERROR", 0x1),
    ("WARNING", 0x2),
    ("INFORMATION", 0x4),
    ("SUCCESSAUDIT", 0x8),
    ("FAILUREAUDIT", 0x10),
];
fn level_number(kind: u16) -> u32 {
    match kind {
        0x1 => 2,
        0x2 => 3,
        _ => 4,
    }
}
fn level_name(kind: u16) -> &'static str {
    match kind {
        0x1 => "Error",
        0x2 => "Warning",
        0x8 => "Audit Success",
        0x10 => "Audit Failure",
        _ => "Information",
    }
}
fn fail(cmd: &mut Cmd, host: &mut dyn Host, text: &str, code: i32) -> Outcome {
    cmd.err_line(host, text);
    cmd.errorlevel = code;
    Outcome::Done(false)
}
pub(crate) fn eventcreate(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let mut kind = None;
    let mut id = None;
    let mut log = String::from("APPLICATION");
    let mut source = String::from("EventCreate");
    let mut description = None;
    let mut i = 0;
    while i < args.len() {
        let key = args[i].to_ascii_uppercase();
        let value = args.get(i + 1).map(|v| unq(v));
        match (key.as_str(), value) {
            ("/?", _) => {
                cmd.out_line(host, "\nEVENTCREATE [/S system [/U username [/P [password]]]] /ID eventid\n            [/L logname] [/SO srcname] /T type /D description\n\nDescription:\n    This command line tool enables an administrator to create\n    a custom event ID and message in a specified event log.\n");
                cmd.errorlevel = 0;
                return Outcome::Done(true);
            }
            ("/T", Some(v)) => kind = Some(v),
            ("/ID", Some(v)) => id = Some(v),
            ("/L", Some(v)) => log = v.to_ascii_uppercase(),
            ("/SO", Some(v)) => source = v,
            ("/D", Some(v)) => description = Some(v),
            (other, _) => {
                return fail(
                    cmd,
                    host,
                    &format!(
                        "ERROR: Invalid argument/option - '{other}'.\nType \"EVENTCREATE /?\" for usage."
                    ),
                    1,
                );
            }
        }
        i += 2;
    }
    let Some(description) = description else {
        return fail(
            cmd,
            host,
            "ERROR: Mandatory option '/D' is missing.\nType \"EVENTCREATE /?\" for usage.",
            1,
        );
    };
    let Some(kind_name) = kind else {
        return fail(
            cmd,
            host,
            "ERROR: Mandatory option '/T' is missing.\nType \"EVENTCREATE /?\" for usage.",
            1,
        );
    };
    let Some(&(type_name, kind)) = TYPES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(&kind_name))
    else {
        return fail(
            cmd,
            host,
            "ERROR: Invalid Argument/Option - '/T'.\nType \"EVENTCREATE /?\" for usage.",
            1,
        );
    };
    let Some(id) = id
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| (1..=1000).contains(v))
    else {
        return fail(
            cmd,
            host,
            "ERROR: Invalid ID. ID should be in the range of 1 - 1000.",
            1,
        );
    };
    if log != "APPLICATION" && log != "SYSTEM" {
        return fail(
            cmd,
            host,
            "ERROR: Specified log does not exist. Cannot create the event.",
            1,
        );
    }
    match host.report_event(&log, kind, id, &source, &description) {
        Ok(()) => {
            cmd.out_line(
                host,
                &format!("SUCCESS: An event of type '{type_name}' was created in the '{log}' log with '{source}' as the source."),
            );
            cmd.errorlevel = 0;
            Outcome::Done(true)
        }
        Err(5) => fail(cmd, host, "ERROR: Access is denied.", 1),
        Err(_) => fail(
            cmd,
            host,
            "ERROR: The event log service is not available.",
            1,
        ),
    }
}
/// `2026-11-07T10:00:00.0000000Z` from a FILETIME.
fn iso(time: u64) -> String {
    let seconds = time / 10_000_000;
    let fraction = time % 10_000_000;
    let days = seconds / 86_400;
    let rest = seconds % 86_400;
    // Days since 1601-01-01 to a civil date.
    let (y, m, d) = crate::host::civil_from_days(days as i64 - 134_774);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{fraction:07}Z",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&apos;")
}
pub(crate) fn wevtutil(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let sub = args
        .first()
        .map(|a| a.to_ascii_lowercase())
        .unwrap_or_default();
    let log_arg = args.get(1).map(|l| unq(l));
    let option = |name: &str| -> Option<String> {
        args.iter().skip(2).find_map(|a| {
            let lower = a.to_ascii_lowercase();
            let rest = lower
                .strip_prefix(&format!("/{name}:"))
                .or_else(|| lower.strip_prefix(&format!("-{name}:")))?;
            let _ = rest;
            Some(String::from(&a[name.len() + 2..]))
        })
    };
    let computer = host.computer_name();
    match (sub.as_str(), log_arg.as_deref()) {
        ("el" | "enum-logs", _) => {
            for name in ["Application", "Security", "Setup", "System"] {
                cmd.out_line(host, name);
            }
            cmd.errorlevel = 0;
            Outcome::Done(true)
        }
        ("qe" | "query-events", Some(log)) => {
            let count = option("c").and_then(|v| v.parse::<usize>().ok());
            let newest = option("rd").is_some_and(|v| v.eq_ignore_ascii_case("true"));
            let text = option("f").is_some_and(|v| v.eq_ignore_ascii_case("text"));
            let events: Vec<HostEvent> = match host.events(log, newest, count.unwrap_or(usize::MAX))
            {
                Ok(e) => e,
                Err(5) => {
                    return fail(cmd, host, "Failed to read events. Access is denied.", 5);
                }
                Err(_) => {
                    return fail(
                        cmd,
                        host,
                        &format!(
                            "Failed to query log {log}. The specified channel could not be found."
                        ),
                        15007,
                    );
                }
            };
            let channel = crate::host::log_name(log).unwrap_or(log);
            for (i, e) in events.iter().enumerate() {
                if text {
                    cmd.out_line(
                        host,
                        &format!(
                            "Event[{i}]:\n  Log Name: {channel}\n  Source: {}\n  Date: {}\n  Event ID: {}\n  Task: N/A\n  Level: {}\n  Opcode: N/A\n  Keyword: Classic\n  User: N/A\n  User Name: N/A\n  Computer: {computer}\n  Description: \n{}\n",
                            e.source,
                            iso(e.time),
                            e.id,
                            level_name(e.kind),
                            e.message
                        ),
                    );
                } else {
                    cmd.out_line(
                        host,
                        &format!(
                            "<Event xmlns='http://schemas.microsoft.com/win/2004/08/events/event'><System><Provider Name='{}'/><EventID Qualifiers='0'>{}</EventID><Version>0</Version><Level>{}</Level><Task>0</Task><Opcode>0</Opcode><Keywords>0x80000000000000</Keywords><TimeCreated SystemTime='{}'/><EventRecordID>{}</EventRecordID><Correlation/><Execution ProcessID='0' ThreadID='0'/><Channel>{channel}</Channel><Computer>{computer}</Computer><Security/></System><EventData><Data>{}</Data></EventData></Event>",
                            xml_escape(&e.source),
                            e.id,
                            level_number(e.kind),
                            iso(e.time),
                            e.record,
                            xml_escape(&e.message)
                        ),
                    );
                }
            }
            cmd.errorlevel = 0;
            Outcome::Done(true)
        }
        ("gli" | "get-loginfo", Some(log)) => match host.events(log, false, usize::MAX) {
            Ok(events) => {
                let oldest = events.first().map_or(0, |e| e.record);
                cmd.out_line(
                    host,
                    &format!(
                        "creationTime: 2026-11-01T00:00:00.000Z\nlastAccessTime: 2026-11-01T00:00:00.000Z\nlastWriteTime: 2026-11-01T00:00:00.000Z\nfileSize: {}\nattributes: 32\nnumberOfLogRecords: {}\noldestRecordNumber: {oldest}",
                        16 + 256 * 512,
                        events.len()
                    ),
                );
                cmd.errorlevel = 0;
                Outcome::Done(true)
            }
            Err(_) => fail(
                cmd,
                host,
                "Failed to read log configuration. The specified channel could not be found.",
                15007,
            ),
        },
        ("cl" | "clear-log", Some(log)) => match host.clear_log(log) {
            Ok(()) => {
                cmd.errorlevel = 0;
                Outcome::Done(true)
            }
            Err(5) => fail(
                cmd,
                host,
                &format!("Failed to clear log {log}. Access is denied."),
                5,
            ),
            Err(_) => fail(
                cmd,
                host,
                &format!("Failed to clear log {log}. The specified channel could not be found."),
                15007,
            ),
        },
        _ => {
            cmd.out_line(host, "Windows Events Command Line Utility.\n\nEnables you to retrieve information about event logs and publishers,\ninstall and uninstall event manifests, run queries, and export, archive,\nand clear logs.\n\nUsage:\n\nYou can use either the short (for example, ep /uni) or long (for example,\nenum-publishers /unicode) version of the command and option names.\n\nwevtutil COMMAND [ARGUMENT [ARGUMENT] ...] [/OPTION:VALUE [/OPTION:VALUE] ...]\n\nCommands:\n\nel | enum-logs          List log names.\ngli | get-loginfo       Get log status information.\nqe | query-events       Query events from a log or log file.\ncl | clear-log          Clear a log.\n");
            cmd.errorlevel = 0;
            Outcome::Done(true)
        }
    }
}
