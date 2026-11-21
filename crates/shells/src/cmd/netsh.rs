//! `netsh advfirewall`: Windows Defender Firewall's profiles and rules,
//! stored where Windows keeps them (HKLM FirewallPolicy, rules in their
//! "v2.33|..." form) through the host's registry, then handed to the
//! network stack again (`Host::firewall_reload`). Changing anything takes
//! an administrator's (elevated) console, as on Windows. Output follows
//! Windows' netsh.exe.
use super::Cmd;
use super::builtin::Outcome;
use crate::host::{Host, RegValue};
use alloc::{format, string::String, vec::Vec};
use door_firewall::{Action, Direction, Rule, profile};
use door_service_protocol::firewall as fw;

const ELEVATION: &str = "The requested operation requires elevation (Run as administrator).\n";
const INVALID: &str = "A specified value is not valid.\n";
const NO_RULES: &str = "\nNo rules match the specified criteria.\n";
const LINE: &str = "----------------------------------------------------------------------";

fn key(sub: &str) -> String {
    format!("HKLM\\{}\\{sub}", fw::POLICY_KEY)
}
fn say(cmd: &mut Cmd, host: &mut dyn Host, text: &str, ok: bool) -> Outcome {
    cmd.out_line(host, text);
    cmd.errorlevel = if ok { 0 } else { 1 };
    Outcome::Done(ok)
}
/// `key=value` arguments, keys lower-cased, values unquoted.
fn pairs(args: &[String]) -> Vec<(String, String)> {
    args.iter()
        .map(|a| match a.split_once('=') {
            Some((k, v)) => (k.to_ascii_lowercase(), v.replace('"', "")),
            None => (a.to_ascii_lowercase().replace('"', ""), String::new()),
        })
        .collect()
}
fn get<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}
/// The rules in the registry: (value name, rule).
fn rules(host: &mut dyn Host) -> Vec<(String, Rule)> {
    host.reg_values(&key(fw::RULES))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(id, v)| match v {
            RegValue::Sz(text) => Rule::parse(&id, &text).ok().map(|r| (id, r)),
            _ => None,
        })
        .collect()
}
/// The three profile bytes (`net::firewall::profile`).
fn profiles(host: &mut dyn Host) -> [u8; 3] {
    fw::PROFILE_KEYS.map(|k| {
        let values = host.reg_values(&key(k)).unwrap_or_default();
        fw::profile_byte(fw::VALUES.map(|name| {
            values.iter().find_map(|(n, v)| match v {
                RegValue::Dword(d) if n.eq_ignore_ascii_case(name) => Some(*d),
                _ => None,
            })
        }))
    })
}
fn write_profile(host: &mut dyn Host, index: usize, byte: u8) -> bool {
    let k = key(fw::PROFILE_KEYS[index]);
    fw::VALUES
        .iter()
        .zip(fw::profile_values(byte))
        .all(|(name, value)| host.reg_set(&k, name, &RegValue::Dword(value)).is_ok())
}
/// Profiles a `show`/`set` names (indexes into the three).
fn which(word: &str) -> Option<&'static [usize]> {
    Some(match word {
        "allprofiles" => &[0, 1, 2],
        "domainprofile" => &[0],
        "privateprofile" => &[1],
        // Doors' network is public.
        "publicprofile" | "currentprofile" => &[2],
        _ => return None,
    })
}
const NAMES: [&str; 3] = ["Domain", "Private", "Public"];

/// netsh's own words: split at spaces outside quotes (`name="Web Server"`
/// and `blockinbound,allowoutbound` stay whole).
fn words(tail: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in tail.chars() {
        if c == '"' {
            quoted = !quoted;
            cur.push(c);
        } else if !quoted && c.is_whitespace() {
            if !cur.is_empty() {
                out.push(core::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}
pub(crate) fn netsh(cmd: &mut Cmd, host: &mut dyn Host, tail: &str) -> Outcome {
    let args = words(tail);
    let args = &args[..];
    let lower: Vec<String> = args.iter().map(|a| a.to_ascii_lowercase()).collect();
    match lower.first().map(String::as_str) {
        None | Some("?" | "help" | "/?") => say(
            cmd,
            host,
            "\nThe following commands are available:\n\nCommands in this context:\n?              - Displays a list of commands.\nadvfirewall    - Changes to the `netsh advfirewall' context.\ninterface      - Changes to the `netsh interface' context.\nwlan           - Changes to the `netsh wlan' context.\n",
            true,
        ),
        Some("advfirewall") => advfirewall(cmd, host, &args[1..], &lower[1..]),
        Some(other) => {
            let text = format!("The following command was not found: {other}.\n");
            say(cmd, host, &text, false)
        }
    }
}
fn advfirewall(cmd: &mut Cmd, host: &mut dyn Host, args: &[String], lower: &[String]) -> Outcome {
    match lower.first().map(String::as_str) {
        Some("show") => {
            let Some(list) = lower.get(1).and_then(|w| which(w)) else {
                return say(cmd, host, INVALID, false);
            };
            let only_state = lower.get(2).is_some_and(|w| w == "state");
            let bytes = profiles(host);
            let mut out = String::new();
            for &i in list {
                let b = bytes[i];
                let p = door_firewall::ProfileState {
                    enabled: b & 1 != 0,
                    inbound_allow: b & 2 != 0,
                    shields_up: b & 4 != 0,
                    outbound_allow: b & 8 == 0,
                };
                out.push_str(&format!("\n{} Profile Settings: \n{LINE}\n", NAMES[i]));
                out.push_str(&format!(
                    "State                                 {}\n",
                    if p.enabled { "ON" } else { "OFF" }
                ));
                if only_state {
                    continue;
                }
                let inbound = if p.shields_up {
                    "BlockInboundAlways"
                } else if p.inbound_allow {
                    "AllowInbound"
                } else {
                    "BlockInbound"
                };
                let outbound = if p.outbound_allow {
                    "AllowOutbound"
                } else {
                    "BlockOutbound"
                };
                out.push_str(&format!(
                    "Firewall Policy                       {inbound},{outbound}\n\
                     LocalFirewallRules                    N/A (GPO-store only)\n\
                     LocalConSecRules                      N/A (GPO-store only)\n\
                     InboundUserNotification               Enable\n\
                     RemoteManagement                      Disable\n\
                     UnicastResponseToMulticast            Enable\n\
                     \nLogging:\n\
                     LogAllowedConnections                 Disable\n\
                     LogDroppedConnections                 Disable\n\
                     FileName                              %systemroot%\\system32\\LogFiles\\Firewall\\pfirewall.log\n\
                     MaxFileSize                           4096\n\n"
                ));
            }
            out.push_str("Ok.\n");
            say(cmd, host, &out, true)
        }
        Some("set") => {
            if !host.elevated() {
                return say(cmd, host, ELEVATION, false);
            }
            let (Some(list), Some(setting)) = (lower.get(1).and_then(|w| which(w)), lower.get(2))
            else {
                return say(cmd, host, INVALID, false);
            };
            let value = lower.get(3).map(String::as_str).unwrap_or("");
            let bytes = profiles(host);
            for &i in list {
                let mut b = bytes[i];
                match (setting.as_str(), value) {
                    ("state", "on") => b |= 1,
                    ("state", "off") => b &= !1,
                    ("firewallpolicy", policy) => {
                        let Some((inbound, outbound)) = policy.split_once(',') else {
                            return say(cmd, host, INVALID, false);
                        };
                        b &= 1;
                        match inbound {
                            "blockinbound" => (),
                            "blockinboundalways" => b |= 4,
                            "allowinbound" => b |= 2,
                            _ => return say(cmd, host, INVALID, false),
                        }
                        match outbound {
                            "allowoutbound" => (),
                            "blockoutbound" => b |= 8,
                            _ => return say(cmd, host, INVALID, false),
                        }
                    }
                    _ => return say(cmd, host, INVALID, false),
                }
                if !write_profile(host, i, b) {
                    return say(cmd, host, ELEVATION, false);
                }
            }
            host.firewall_reload();
            say(cmd, host, "Ok.\n", true)
        }
        Some("reset") => {
            if !host.elevated() {
                return say(cmd, host, ELEVATION, false);
            }
            for (id, _) in rules(host) {
                let _ = host.reg_delete(&key(fw::RULES), Some(&id));
            }
            for i in 0..3 {
                write_profile(
                    host,
                    i,
                    door_service_protocol::net::firewall::profile::DEFAULT,
                );
            }
            host.firewall_reload();
            say(cmd, host, "Ok.\n", true)
        }
        Some("firewall") => firewall(cmd, host, &args[1..], &lower[1..]),
        _ => say(
            cmd,
            host,
            "\nThe following commands are available:\n\nCommands in this context:\n?              - Displays a list of commands.\nfirewall       - Changes to the `netsh advfirewall firewall' context.\nreset          - Resets the policy to the default out-of-box policy.\nset            - Sets the per-profile or global settings.\nshow           - Displays profile or global properties.\n",
            lower.is_empty(),
        ),
    }
}
/// A rule from `add rule` arguments (None: a value netsh refuses).
fn rule_from(pairs: &[(String, String)], id: &str) -> Option<Rule> {
    let name = get(pairs, "name").filter(|n| !n.is_empty() && *n != "all")?;
    let direction = match get(pairs, "dir")?.to_ascii_lowercase().as_str() {
        "in" => Direction::In,
        "out" => Direction::Out,
        _ => return None,
    };
    let action = match get(pairs, "action")?.to_ascii_lowercase().as_str() {
        "allow" => Action::Allow,
        "block" => Action::Block,
        _ => return None,
    };
    let protocol = match get(pairs, "protocol")
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("any") => None,
        Some("tcp") => Some(6),
        Some("udp") => Some(17),
        Some("icmpv4") => Some(1),
        Some("icmpv6") => Some(58),
        Some(n) => Some(n.parse::<u8>().ok()?),
    };
    let mut profiles = 0;
    if let Some(list) = get(pairs, "profile") {
        for p in list.split(',') {
            profiles |= match p.trim().to_ascii_lowercase().as_str() {
                "public" => profile::PUBLIC,
                "private" => profile::PRIVATE,
                "domain" => profile::DOMAIN,
                "any" => profile::ALL,
                _ => return None,
            };
        }
    }
    let enabled = match get(pairs, "enable").map(str::to_ascii_lowercase).as_deref() {
        None | Some("yes") => true,
        Some("no") => false,
        _ => return None,
    };
    // Ports only with TCP or UDP, as netsh requires.
    let ports = ["localport", "remoteport"].map(|k| get(pairs, k).filter(|p| *p != "any"));
    if ports.iter().any(Option::is_some) && !matches!(protocol, Some(6 | 17)) {
        return None;
    }
    let mut text = format!(
        "v2.33|Action={}|Active={}|Dir={}|",
        if action == Action::Allow {
            "Allow"
        } else {
            "Block"
        },
        if enabled { "TRUE" } else { "FALSE" },
        if direction == Direction::In {
            "In"
        } else {
            "Out"
        }
    );
    if let Some(p) = protocol {
        text.push_str(&format!("Protocol={p}|"));
    }
    if profiles != 0 && profiles != profile::ALL {
        for (bit, name) in [
            (profile::DOMAIN, "Domain"),
            (profile::PRIVATE, "Private"),
            (profile::PUBLIC, "Public"),
        ] {
            if profiles & bit != 0 {
                text.push_str(&format!("Profile={name}|"));
            }
        }
    }
    if let Some(p) = ports[0] {
        text.push_str(&format!("LPort={p}|"));
    }
    if let Some(p) = ports[1] {
        text.push_str(&format!("RPort={p}|"));
    }
    if let Some(list) = get(pairs, "remoteip").filter(|a| !a.eq_ignore_ascii_case("any")) {
        // IPv6 entries go to RA6, keywords ("LocalSubnet", ...) to both.
        for a in list.split(',').map(str::trim) {
            if a.contains(':') {
                text.push_str(&format!("RA6={a}|"));
            } else if a.starts_with(|c: char| c.is_ascii_alphabetic()) {
                text.push_str(&format!("RA4={a}|RA6={a}|"));
            } else {
                text.push_str(&format!("RA4={a}|"));
            }
        }
    }
    if let Some(app) = get(pairs, "program") {
        text.push_str(&format!("App={app}|"));
    }
    text.push_str(&format!("Name={name}|"));
    if let Some(d) = get(pairs, "description") {
        text.push_str(&format!("Desc={d}|"));
    }
    if text.contains("||") || name.contains('|') {
        return None;
    }
    Rule::parse(id, &text).ok()
}
fn show_rule(rule: &Rule, verbose: bool) -> String {
    let field = |label: &str, value: &str| format!("{:<38}{value}\n", format!("{label}:"));
    let mut out = format!("\n{}\n{LINE}\n", field("Rule Name", &rule.name).trim_end());
    if verbose {
        out.push_str(&field("Description", &rule.description));
    }
    out.push_str(&field("Enabled", if rule.enabled { "Yes" } else { "No" }));
    out.push_str(&field(
        "Direction",
        if rule.direction == Direction::In {
            "In"
        } else {
            "Out"
        },
    ));
    let profiles: Vec<&str> = [
        (profile::DOMAIN, "Domain"),
        (profile::PRIVATE, "Private"),
        (profile::PUBLIC, "Public"),
    ]
    .iter()
    .filter(|(b, _)| rule.profiles & b != 0)
    .map(|(_, n)| *n)
    .collect();
    out.push_str(&field("Profiles", &profiles.join(",")));
    out.push_str(&field("Grouping", &rule.group));
    out.push_str(&field("LocalIP", "Any"));
    let remote = rule.remote_list();
    let remote = if remote.is_empty() {
        String::from("Any")
    } else {
        remote.join(",")
    };
    out.push_str(&field("RemoteIP", &remote));
    let protocol = match rule.protocol {
        None => String::from("Any"),
        Some(6) => String::from("TCP"),
        Some(17) => String::from("UDP"),
        Some(1) => String::from("ICMPv4"),
        Some(58) => String::from("ICMPv6"),
        Some(n) => format!("{n}"),
    };
    out.push_str(&field("Protocol", &protocol));
    let ports = |list: &[(u16, u16)]| -> String {
        if list.is_empty() {
            return String::from("Any");
        }
        list.iter()
            .map(|(a, b)| {
                if a == b {
                    format!("{a}")
                } else {
                    format!("{a}-{b}")
                }
            })
            .collect::<Vec<_>>()
            .join(",")
    };
    if matches!(rule.protocol, Some(6 | 17)) {
        out.push_str(&field("LocalPort", &ports(&rule.local_ports)));
        out.push_str(&field("RemotePort", &ports(&rule.remote_ports)));
    }
    out.push_str(&field("Edge traversal", "No"));
    if verbose {
        if let Some(p) = &rule.program {
            out.push_str(&field("Program", p));
        }
        out.push_str(&field("InterfaceTypes", "Any"));
        out.push_str(&field("Security", "NotRequired"));
        out.push_str(&field("Rule source", "Local Setting"));
    }
    out.push_str(&field(
        "Action",
        if rule.action == Action::Allow {
            "Allow"
        } else {
            "Block"
        },
    ));
    out
}
fn firewall(cmd: &mut Cmd, host: &mut dyn Host, args: &[String], lower: &[String]) -> Outcome {
    let verb = lower.first().map(String::as_str);
    if lower.get(1).is_none_or(|w| w != "rule") {
        return say(
            cmd,
            host,
            "\nThe following commands are available:\n\nCommands in this context:\n?              - Displays a list of commands.\nadd            - Adds a new inbound or outbound firewall rule.\ndelete         - Deletes all matching firewall rules.\nset            - Sets new values for properties of a existing rule.\nshow           - Displays a specified firewall rule.\n",
            lower.is_empty(),
        );
    }
    let p = pairs(&args[2..]);
    let filter = |r: &Rule, p: &[(String, String)]| {
        let name = get(p, "name").unwrap_or("");
        (name.eq_ignore_ascii_case("all") || r.name.eq_ignore_ascii_case(name))
            && get(p, "dir").is_none_or(|d| {
                d.eq_ignore_ascii_case(if r.direction == Direction::In {
                    "in"
                } else {
                    "out"
                })
            })
    };
    match verb {
        Some("add") => {
            if !host.elevated() {
                return say(cmd, host, ELEVATION, false);
            }
            let id = format!(
                "{{{:08X}-{:04X}-{:04X}-{:04X}-{:04X}{:08X}}}",
                host.random(),
                host.random() & 0xffff,
                host.random() & 0xffff,
                host.random() & 0xffff,
                host.random() & 0xffff,
                host.random()
            );
            let Some(rule) = rule_from(&p, &id) else {
                return say(cmd, host, INVALID, false);
            };
            if host
                .reg_set(&key(fw::RULES), &id, &RegValue::Sz(rule.text()))
                .is_err()
            {
                return say(cmd, host, ELEVATION, false);
            }
            host.firewall_reload();
            say(cmd, host, "Ok.\n", true)
        }
        Some("delete") => {
            if !host.elevated() {
                return say(cmd, host, ELEVATION, false);
            }
            if get(&p, "name").is_none() {
                return say(cmd, host, INVALID, false);
            }
            let matched: Vec<String> = rules(host)
                .into_iter()
                .filter(|(_, r)| filter(r, &p))
                .map(|(id, _)| id)
                .collect();
            if matched.is_empty() {
                return say(cmd, host, NO_RULES, false);
            }
            for id in &matched {
                if host.reg_delete(&key(fw::RULES), Some(id)).is_err() {
                    return say(cmd, host, ELEVATION, false);
                }
            }
            host.firewall_reload();
            let text = format!("\nDeleted {} rule(s).\nOk.\n", matched.len());
            say(cmd, host, &text, true)
        }
        Some("show") => {
            if get(&p, "name").is_none() {
                return say(cmd, host, INVALID, false);
            }
            let verbose = p.iter().any(|(k, _)| k == "verbose");
            let shown: Vec<String> = rules(host)
                .iter()
                .filter(|(_, r)| filter(r, &p))
                .map(|(_, r)| show_rule(r, verbose))
                .collect();
            if shown.is_empty() {
                return say(cmd, host, NO_RULES, false);
            }
            let text = format!("{}Ok.\n", shown.concat());
            say(cmd, host, &text, true)
        }
        Some("set") => {
            if !host.elevated() {
                return say(cmd, host, ELEVATION, false);
            }
            // `set rule name=X new enable=no action=block`.
            let at = p.iter().position(|(k, _)| k == "new");
            let (Some(at), Some(_)) = (at, get(&p, "name")) else {
                return say(cmd, host, INVALID, false);
            };
            let (which, new) = p.split_at(at);
            let matched: Vec<(String, Rule)> = rules(host)
                .into_iter()
                .filter(|(_, r)| filter(r, which))
                .collect();
            if matched.is_empty() {
                return say(cmd, host, NO_RULES, false);
            }
            let count = matched.len();
            for (id, mut rule) in matched {
                for (k, v) in &new[1..] {
                    match (k.as_str(), v.to_ascii_lowercase().as_str()) {
                        ("enable", "yes") => rule.enabled = true,
                        ("enable", "no") => rule.enabled = false,
                        ("action", "allow") => rule.action = Action::Allow,
                        ("action", "block") => rule.action = Action::Block,
                        _ => return say(cmd, host, INVALID, false),
                    }
                }
                if host
                    .reg_set(&key(fw::RULES), &id, &RegValue::Sz(rule.text()))
                    .is_err()
                {
                    return say(cmd, host, ELEVATION, false);
                }
            }
            host.firewall_reload();
            let text = format!("\nUpdated {count} rule(s).\nOk.\n");
            say(cmd, host, &text, true)
        }
        _ => say(cmd, host, INVALID, false),
    }
}
