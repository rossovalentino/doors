//! RASDIAL and WIREGUARD (P15 VPN) over the host's VPN calls: list, dial
//! and hang up the user's connections as rasdial.exe prints it, and
//! WireGuard for Windows' `/installtunnelservice` (import a wg-quick file
//! and bring it up, named after the file) and `/uninstalltunnelservice`.
use super::Cmd;
use super::builtin::{Outcome, Pending as BuiltinPending, unq};
use super::tools::Pending;
use crate::Step;
use crate::host::{FsError, Host, VpnState};
use alloc::{format, string::String, vec::Vec};

/// A dial in progress: the connection and the half seconds left (20 s).
pub(crate) struct Dial {
    name: String,
    remaining: u32,
}
const DIAL_TICKS: u32 = 40;
/// Bound on a configuration file (door.network's limit).
const MAX_TEXT: usize = 4096;
const COMPLETED: &str = "Command completed successfully.";

/// rasdial's report of a Remote Access error (and its exit code).
fn ras_error(cmd: &mut Cmd, host: &mut dyn Host, code: u32) -> Outcome {
    let text = match code {
        623 => "The system could not find the phone book entry for this connection.",
        809 => {
            "The network connection between your computer and the VPN server could not be established because the remote server is not responding. This could be because one of the network devices (e.g, firewalls, NAT, routers, etc) between your computer and the remote server is not configured to allow VPN connections. Please contact your Administrator or your service provider to determine which device may be causing the problem."
        }
        868 => {
            "The remote connection was not made because the name of the remote access server did not resolve."
        }
        _ => {
            "Unable to establish the VPN connection. The VPN server may be unreachable, or security parameters may not be configured properly for this connection."
        }
    };
    let code = if matches!(code, 623 | 809 | 868) {
        code
    } else {
        800
    };
    cmd.out_line(
        host,
        &format!("Remote Access error {code} - {text}\n\nFor more help on this error:\n        Type 'hh netcfg.chm'\n        In help, click Troubleshooting, then Error Messages, then {code}\n"),
    );
    cmd.errorlevel = code as i32;
    Outcome::Done(false)
}
fn usage(cmd: &mut Cmd, host: &mut dyn Host) -> Outcome {
    cmd.out_line(
        host,
        "USAGE:\n\trasdial entryname [username [password|*]] [/DOMAIN:domain]\n\t\t[/PHONE:phonenumber] [/CALLBACK:callbacknumber]\n\t\t[/PHONEBOOK:phonebookfile] [/PREFIXSUFFIX]\n\n\trasdial [entryname] /DISCONNECT\n\n\trasdial\n\n\tCommand completed successfully.",
    );
    Outcome::Done(true)
}

pub(crate) fn rasdial(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let args: Vec<String> = args.iter().map(|a| unq(a)).collect();
    if args.iter().any(|a| a == "/?") {
        return usage(cmd, host);
    }
    let disconnect = args
        .iter()
        .any(|a| a.eq_ignore_ascii_case("/disconnect") || a.eq_ignore_ascii_case("/d"));
    let name = args.iter().find(|a| !a.starts_with('/'));
    let list = host.vpn_connections();
    let Some(name) = name else {
        if disconnect {
            return usage(cmd, host);
        }
        let connected: Vec<&str> = list
            .iter()
            .filter(|c| c.state == VpnState::Connected)
            .map(|c| c.name.as_str())
            .collect();
        if connected.is_empty() {
            cmd.out_line(host, "No connections");
        } else {
            cmd.out_line(host, "Connected to");
            for n in connected {
                cmd.out_line(host, n);
            }
        }
        cmd.out_line(host, COMPLETED);
        return Outcome::Done(true);
    };
    let Some(entry) = list.iter().find(|c| c.name.eq_ignore_ascii_case(name)) else {
        return ras_error(cmd, host, 623);
    };
    let name = entry.name.clone();
    if disconnect {
        if !matches!(entry.state, VpnState::Connected | VpnState::Connecting) {
            cmd.out_line(host, &format!("You are not connected to {name}."));
        } else if host.vpn_disconnect(&name).is_err() {
            return ras_error(cmd, host, 800);
        }
        cmd.out_line(host, COMPLETED);
        return Outcome::Done(true);
    }
    if entry.state == VpnState::Connected {
        cmd.out_line(
            host,
            &format!("You are already connected to {name}.\n{COMPLETED}"),
        );
        return Outcome::Done(true);
    }
    cmd.out_line(host, &format!("Connecting to {name}..."));
    // A dial already in progress is waited for, not restarted.
    if entry.state != VpnState::Connecting && host.vpn_connect(&name).is_err() {
        return ras_error(cmd, host, 800);
    }
    Outcome::Wait(
        BuiltinPending::Tool(Pending::Dial(Dial {
            name,
            remaining: DIAL_TICKS,
        })),
        Step::Sleep {
            ms: 500,
            keys: false,
        },
    )
}
/// Half a second later: connected, failed, or still waiting.
pub(crate) fn dial_tick(cmd: &mut Cmd, host: &mut dyn Host, dial: Dial) -> Outcome {
    let entry = host
        .vpn_connections()
        .into_iter()
        .find(|c| c.name == dial.name);
    match entry.as_ref().map(|e| (e.state, e.error)) {
        Some((VpnState::Connected, _)) => {
            cmd.out_line(
                host,
                &format!("Successfully connected to {}.\n{COMPLETED}", dial.name),
            );
            Outcome::Done(true)
        }
        Some((VpnState::Failed, error)) => ras_error(cmd, host, error),
        Some((VpnState::Connecting, _)) if dial.remaining > 0 => Outcome::Wait(
            BuiltinPending::Tool(Pending::Dial(Dial {
                remaining: dial.remaining - 1,
                ..dial
            })),
            Step::Sleep {
                ms: 500,
                keys: false,
            },
        ),
        _ => {
            // Gave up waiting (or the connection went away): hang up.
            let _ = host.vpn_disconnect(&dial.name);
            ras_error(cmd, host, 809)
        }
    }
}

/// WireGuard's tunnel names: 1 to 32 of `A-Z a-z 0-9 _ = + . -`.
pub fn tunnel_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_=+.-".contains(&b))
}
fn wireguard_usage(cmd: &mut Cmd, host: &mut dyn Host) -> Outcome {
    cmd.err_line(host, "Usage: wireguard.exe [\n    /installtunnelservice CONFIG_PATH\n    /uninstalltunnelservice TUNNEL_NAME\n]");
    cmd.errorlevel = 1;
    Outcome::Done(false)
}
fn fail(cmd: &mut Cmd, host: &mut dyn Host, text: &str) -> Outcome {
    cmd.err_line(host, text);
    cmd.errorlevel = 1;
    Outcome::Done(false)
}
pub(crate) fn wireguard(cmd: &mut Cmd, host: &mut dyn Host, args: &[String]) -> Outcome {
    let args: Vec<String> = args.iter().map(|a| unq(a)).collect();
    let (Some(verb), Some(operand), None) = (args.first(), args.get(1), args.get(2)) else {
        return wireguard_usage(cmd, host);
    };
    match verb.to_ascii_lowercase().as_str() {
        "/installtunnelservice" => {
            let file = operand.rsplit(['\\', '/']).next().unwrap_or(operand);
            let lower = file.to_ascii_lowercase();
            let Some(stem) = lower.strip_suffix(".conf").map(|s| &file[..s.len()]) else {
                return fail(cmd, host, "Configuration files must end in .conf");
            };
            if !tunnel_name(stem) {
                return fail(cmd, host, "Tunnel name is not valid");
            }
            let bytes = match cmd
                .native(host, operand)
                .and_then(|n| host.read(&n, MAX_TEXT))
            {
                Ok(b) => b,
                Err(e) => return fail(cmd, host, e.message()),
            };
            let Ok(text) = core::str::from_utf8(&bytes) else {
                return fail(cmd, host, "Configuration file is not valid UTF-8");
            };
            let config = match door_wireguard::Config::parse(text) {
                Ok(c) => c,
                Err(e) => return fail(cmd, host, &format!("Unable to load configuration: {e}")),
            };
            if config.peer.endpoint.is_none() || config.addresses.is_empty() {
                return fail(
                    cmd,
                    host,
                    "A Doors VPN connection needs an Address and the peer's Endpoint",
                );
            }
            if let Err(e) = host
                .vpn_save(stem, text)
                .and_then(|()| host.vpn_connect(stem))
            {
                return fail(cmd, host, e.message());
            }
            Outcome::Done(true)
        }
        "/uninstalltunnelservice" => {
            let _ = host.vpn_disconnect(operand);
            match host.vpn_remove(operand) {
                Ok(()) => Outcome::Done(true),
                Err(FsError::NotFound) => fail(
                    cmd,
                    host,
                    "The specified service does not exist as an installed service.",
                ),
                Err(e) => fail(cmd, host, e.message()),
            }
        }
        _ => wireguard_usage(cmd, host),
    }
}
