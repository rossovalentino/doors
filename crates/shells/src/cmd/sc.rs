//! `sc` and `net start/stop` over the Service Control Manager (the host's
//! `services` methods). Output follows Windows' sc.exe and net.exe.
use super::Cmd;
use super::builtin::{Outcome, unq};
use crate::host::{Host, Service};
use alloc::{format, string::String, vec::Vec};

const STATES: [&str; 8] = [
    "",
    "STOPPED",
    "START_PENDING",
    "STOP_PENDING",
    "RUNNING",
    "CONTINUE_PENDING",
    "PAUSE_PENDING",
    "PAUSED",
];
fn kind_line(kind: u32) -> String {
    let name = match kind & 0x3f {
        0x1 => "KERNEL_DRIVER",
        0x2 => "FILE_SYSTEM_DRIVER",
        0x20 => "WIN32_SHARE_PROCESS",
        _ => "WIN32_OWN_PROCESS",
    };
    if kind & 0x100 != 0 {
        format!("{:x}  {name}  (interactive)", kind)
    } else {
        format!("{:x}  {name}", kind)
    }
}
fn start_name(start: u32) -> &'static str {
    match start {
        0 => "BOOT_START",
        1 => "SYSTEM_START",
        2 => "AUTO_START",
        4 => "DISABLED",
        _ => "DEMAND_START",
    }
}
/// The accepted-controls line under STATE.
fn accepts(s: &Service) -> String {
    let stoppable = if s.accepted & 1 != 0 {
        "STOPPABLE"
    } else {
        "NOT_STOPPABLE"
    };
    let pausable = if s.accepted & 2 != 0 {
        "PAUSABLE"
    } else {
        "NOT_PAUSABLE"
    };
    let shutdown = if s.accepted & 4 != 0 {
        "ACCEPTS_SHUTDOWN"
    } else {
        "IGNORES_SHUTDOWN"
    };
    format!("({stoppable}, {pausable}, {shutdown})")
}
/// sc's status block (`ex` adds PID and FLAGS).
fn status_block(s: &Service, display: bool, ex: bool) -> String {
    let mut out = format!("\nSERVICE_NAME: {}\n", s.name);
    if display {
        out.push_str(&format!("DISPLAY_NAME: {}\n", s.display));
    }
    out.push_str(&format!(
        "        TYPE               : {}\n        STATE              : {}  {}\n                                {}\n        WIN32_EXIT_CODE    : {}  (0x{:x})\n        SERVICE_EXIT_CODE  : 0  (0x0)\n        CHECKPOINT         : 0x0\n        WAIT_HINT          : 0x0\n",
        kind_line(s.kind),
        s.state,
        STATES.get(s.state as usize).copied().unwrap_or(""),
        accepts(s),
        s.exit_code,
        s.exit_code
    ));
    if ex {
        out.push_str(&format!(
            "        PID                : {}\n        FLAGS              :\n",
            s.pid
        ));
    }
    out
}
/// Win32 messages sc prints after "FAILED <code>:".
fn message(code: u32) -> &'static str {
    match code {
        2 => "The system cannot find the file specified.",
        5 => "Access is denied.",
        87 => "The parameter is incorrect.",
        1052 => "The requested control is not valid for this service.",
        1053 => "The service did not respond to the start or control request in a timely fashion.",
        1056 => "An instance of the service is already running.",
        1058 => {
            "The service cannot be started, either because it is disabled or because it has no enabled devices associated with it."
        }
        1060 => "The specified service does not exist as an installed service.",
        1061 => "The service cannot accept control messages at this time.",
        1062 => "The service has not been started.",
        1063 => "The service process could not connect to the service controller.",
        1072 => "The specified service has been marked for deletion.",
        1068 => "The dependency service or group failed to start.",
        1059 => "Circular service dependency was specified.",
        1073 => "The specified service already exists.",
        1075 => "The dependency service does not exist or has been marked for deletion.",
        _ => "The operation failed.",
    }
}
fn failed(cmd: &mut Cmd, host: &mut dyn Host, api: &str, code: u32) -> Outcome {
    cmd.out_line(
        host,
        &format!("[SC] {api} FAILED {code}:\n\n{}\n", message(code)),
    );
    cmd.errorlevel = code as i32;
    Outcome::Done(false)
}
/// `depend=`'s list: names separated by `/` (`/` alone clears it).
fn dependencies(value: &str) -> Vec<String> {
    value
        .split('/')
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(String::from)
        .collect()
}
/// sc's option keys (`binPath= <value>`).
const KEYS: [&str; 10] = [
    "binpath",
    "start",
    "displayname",
    "type",
    "error",
    "depend",
    "obj",
    "password",
    "group",
    "tag",
];
/// `key= value` options. sc spells the `=` as part of the key; cmd's
/// argument splitting may also hand it over as its own word or drop it.
fn options(args: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].to_ascii_lowercase();
        let key = a.strip_suffix('=').unwrap_or(&a);
        if KEYS.contains(&key) {
            let mut v = i + 1;
            if args.get(v).is_some_and(|x| x == "=") {
                v += 1;
            }
            out.push((
                String::from(key),
                args.get(v).map(|v| unq(v)).unwrap_or_default(),
            ));
            i = v + 1;
        } else {
            i += 1;
        }
    }
    out
}
pub(crate) fn sc(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let sub = args
        .first()
        .map(|a| a.to_ascii_lowercase())
        .unwrap_or_default();
    let name = args.get(1).map(|n| unq(n));
    let ok = |cmd: &mut Cmd| {
        cmd.errorlevel = 0;
        Outcome::Done(true)
    };
    match (sub.as_str(), name.as_deref()) {
        ("query" | "queryex", None) => match host.services() {
            Ok(list) => {
                for s in list.iter().filter(|s| s.state != 1) {
                    let block = status_block(s, true, sub == "queryex");
                    cmd.out(host, &block);
                }
                ok(cmd)
            }
            Err(code) => failed(cmd, host, "OpenSCManager", code),
        },
        ("query" | "queryex", Some(n)) => match host.service(n) {
            Ok(s) => {
                let block = status_block(&s, false, sub == "queryex");
                cmd.out(host, &block);
                ok(cmd)
            }
            Err(code) => failed(cmd, host, "EnumQueryServicesStatus:OpenService", code),
        },
        ("start", Some(n)) => {
            match host.service_start(n, &args[2..].iter().map(|a| unq(a)).collect::<Vec<_>>()) {
                Ok(s) => {
                    let block = status_block(&s, false, true);
                    cmd.out(host, &block);
                    ok(cmd)
                }
                Err(code) => failed(cmd, host, "StartService", code),
            }
        }
        ("stop" | "pause" | "continue" | "interrogate", Some(n)) => {
            let control = match sub.as_str() {
                "stop" => 1,
                "pause" => 2,
                "continue" => 3,
                _ => 4,
            };
            match host.service_control(n, control) {
                Ok(s) => {
                    let block = status_block(&s, false, false);
                    cmd.out(host, &block);
                    ok(cmd)
                }
                Err(code) => failed(cmd, host, "ControlService", code),
            }
        }
        ("qc", Some(n)) => match host.service(n) {
            Ok(s) => {
                cmd.out_line(
                    host,
                    &format!(
                        "[SC] QueryServiceConfig SUCCESS\n\nSERVICE_NAME: {}\n        TYPE               : {}\n        START_TYPE         : {}   {}\n        ERROR_CONTROL      : 1   NORMAL\n        BINARY_PATH_NAME   : {}\n        LOAD_ORDER_GROUP   : \n        TAG                : 0\n        DISPLAY_NAME       : {}\n        DEPENDENCIES       : {}\n        SERVICE_START_NAME : LocalSystem\n",
                        s.name,
                        kind_line(s.kind),
                        s.start,
                        start_name(s.start),
                        s.binary,
                        s.display,
                        // Windows prints each further dependency on its own
                        // line, aligned under the first.
                        s.dependencies.join("\n                           : ")
                    ),
                );
                ok(cmd)
            }
            Err(code) => failed(cmd, host, "OpenService", code),
        },
        ("qdescription", Some(n)) => match host.service(n) {
            Ok(s) => {
                cmd.out_line(
                    host,
                    &format!(
                        "[SC] QueryServiceConfig2 SUCCESS\n\nSERVICE_NAME: {}\nDESCRIPTION:  {}\n",
                        s.name, s.description
                    ),
                );
                ok(cmd)
            }
            Err(code) => failed(cmd, host, "OpenService", code),
        },
        ("create", Some(n)) => {
            let o = options(&args[2..]);
            let get = |k: &str| o.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
            let Some(binary) = get("binpath") else {
                return failed(cmd, host, "CreateService", 87);
            };
            let start = match get("start").as_deref() {
                None | Some("demand") => 3,
                Some("auto") | Some("delayed-auto") => 2,
                Some("disabled") => 4,
                Some("boot") => 0,
                Some("system") => 1,
                Some(_) => return failed(cmd, host, "CreateService", 87),
            };
            let service = Service {
                name: String::from(n),
                display: get("displayname").unwrap_or_else(|| String::from(n)),
                kind: 0x10,
                start,
                binary,
                state: 1,
                dependencies: get("depend").map(|d| dependencies(&d)).unwrap_or_default(),
                ..Service::default()
            };
            match host.service_create(&service) {
                Ok(()) => {
                    cmd.out_line(host, "[SC] CreateService SUCCESS");
                    ok(cmd)
                }
                Err(code) => failed(cmd, host, "CreateService", code),
            }
        }
        ("delete", Some(n)) => match host.service_delete(n) {
            Ok(()) => {
                cmd.out_line(host, "[SC] DeleteService SUCCESS");
                ok(cmd)
            }
            Err(code) => failed(cmd, host, "DeleteService", code),
        },
        ("config", Some(n)) => {
            let o = options(&args[2..]);
            let get = |k: &str| o.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
            let start = match get("start").as_deref() {
                None => None,
                Some("demand") => Some(3),
                Some("auto") | Some("delayed-auto") => Some(2),
                Some("disabled") => Some(4),
                Some(_) => return failed(cmd, host, "ChangeServiceConfig", 87),
            };
            // Dependencies first: a cycle (1059) changes nothing.
            let configured = match get("depend") {
                Some(d) => host.service_set_dependencies(n, &dependencies(&d)),
                None => Ok(()),
            }
            .and_then(|()| {
                host.service_configure(
                    n,
                    start,
                    get("binpath").as_deref(),
                    get("displayname").as_deref(),
                    None,
                )
            });
            match configured {
                Ok(()) => {
                    cmd.out_line(host, "[SC] ChangeServiceConfig SUCCESS");
                    ok(cmd)
                }
                Err(code) => failed(cmd, host, "ChangeServiceConfig", code),
            }
        }
        ("description", Some(n)) => {
            let text = args.get(2).map(|t| unq(t)).unwrap_or_default();
            match host.service_configure(n, None, None, None, Some(&text)) {
                Ok(()) => {
                    cmd.out_line(host, "[SC] ChangeServiceConfig2 SUCCESS");
                    ok(cmd)
                }
                Err(code) => failed(cmd, host, "ChangeServiceConfig2", code),
            }
        }
        ("", _) | ("/?", _) => {
            cmd.out(host, "DESCRIPTION:\n        SC is a command line program used for communicating with the\n        Service Control Manager and services.\nUSAGE:\n        sc <server> [command] [service name] <option1> <option2>...\n\n        Commands:\n          query-----------Queries the status for a service, or\n                          enumerates the status for types of services.\n          queryex---------Queries the extended status for a service, or\n                          enumerates the status for types of services.\n          start-----------Starts a service.\n          pause-----------Sends a PAUSE control request to a service.\n          interrogate-----Sends an INTERROGATE control request to a service.\n          continue--------Sends a CONTINUE control request to a service.\n          stop------------Sends a STOP request to a service.\n          config----------Changes the configuration of a service (persistent).\n          description-----Changes the description of a service.\n          qc--------------Queries the configuration information for a service.\n          qdescription----Queries the description for a service.\n          delete----------Deletes a service (from the registry).\n          create----------Creates a service. (adds it to the registry).\n");
            ok(cmd)
        }
        _ => failed(cmd, host, "OpenService", 1060),
    }
}
/// `net start` (no name: the started services by display name), `net start
/// <name>`, `net stop <name>`.
pub(crate) fn net_service(
    cmd: &mut Cmd,
    host: &mut dyn Host,
    stop: bool,
    name: Option<&str>,
) -> Outcome {
    let Some(name) = name else {
        return match host.services() {
            Ok(list) => {
                let mut out = String::from("These Windows services are started:\n\n");
                for s in list.iter().filter(|s| s.state == 4) {
                    out.push_str(&format!("   {}\n", s.display));
                }
                out.push_str("\nThe command completed successfully.\n");
                cmd.out_line(host, &out);
                cmd.errorlevel = 0;
                Outcome::Done(true)
            }
            Err(_) => {
                cmd.err_line(host, "System error 1063 has occurred.\n");
                cmd.errorlevel = 2;
                Outcome::Done(false)
            }
        };
    };
    // net accepts the display name too.
    let resolved = host.services().ok().and_then(|l| {
        l.into_iter()
            .find(|s| s.name.eq_ignore_ascii_case(name) || s.display.eq_ignore_ascii_case(name))
    });
    let Some(service) = resolved else {
        cmd.err_line(
            host,
            "The service name is invalid.\n\nMore help is available by typing NET HELPMSG 2185.\n",
        );
        cmd.errorlevel = 2;
        return Outcome::Done(false);
    };
    let display = service.display.clone();
    let result = if stop {
        cmd.out_line(host, &format!("The {display} service is stopping."));
        host.service_control(&service.name, 1)
            .and_then(|_| host.service_wait(&service.name, 1))
    } else {
        cmd.out_line(host, &format!("The {display} service is starting."));
        host.service_start(&service.name, &[])
            .and_then(|_| host.service_wait(&service.name, 4))
    };
    match result {
        Ok(()) => {
            cmd.out_line(
                host,
                &format!(
                    "The {display} service was {} successfully.\n",
                    if stop { "stopped" } else { "started" }
                ),
            );
            cmd.errorlevel = 0;
            Outcome::Done(true)
        }
        Err(1056) => {
            cmd.err_line(host, "The requested service has already been started.\n\nMore help is available by typing NET HELPMSG 2182.\n");
            cmd.errorlevel = 2;
            Outcome::Done(false)
        }
        Err(1062) => {
            cmd.err_line(host, &format!("The {display} service is not started.\n\nMore help is available by typing NET HELPMSG 3521.\n"));
            cmd.errorlevel = 2;
            Outcome::Done(false)
        }
        Err(5) => {
            cmd.err_line(host, "System error 5 has occurred.\n\nAccess is denied.\n");
            cmd.errorlevel = 2;
            Outcome::Done(false)
        }
        Err(code) => {
            cmd.err_line(
                host,
                &format!("System error {code} has occurred.\n\n{}\n", message(code)),
            );
            cmd.errorlevel = 2;
            Outcome::Done(false)
        }
    }
}
