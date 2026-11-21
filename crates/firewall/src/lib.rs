//! Windows Defender Firewall as data: rules in the form Windows keeps them
//! (`HKLM\SYSTEM\CurrentControlSet\Services\SharedAccess\Parameters\
//! FirewallPolicy\FirewallRules`, one "v2.33|Action=Allow|Dir=In|..."
//! string per rule), the three profiles' on/off state and default actions,
//! and the decision for one connection: loopback always passes; a matching
//! block rule wins over an allow rule; inbound needs an allow rule (or the
//! profile's default), outbound passes unless blocked. door-network enforces
//! it; netsh, the firewall COM API and Settings edit it.
//!
//! A rule may carry conditions this model cannot evaluate (a service, an
//! interface, authentication, local addresses, port keywords such as
//! "RPC"). Such a rule fails closed: as an allow rule it never matches, as a
//! block rule it applies as if the condition held. Its fields are kept
//! verbatim, so editing the rule elsewhere does not lose them.
#![no_std]
extern crate alloc;
use alloc::{format, string::String, vec::Vec};

/// Profile bits (NET_FW_PROFILE2_*).
pub mod profile {
    pub const DOMAIN: u8 = 0x1;
    pub const PRIVATE: u8 = 0x2;
    pub const PUBLIC: u8 = 0x4;
    pub const ALL: u8 = 0x7;
}
/// IANA protocol numbers rules name (NET_FW_IP_PROTOCOL_*).
pub mod protocol {
    pub const ICMP: u8 = 1;
    pub const TCP: u8 = 6;
    pub const UDP: u8 = 17;
    pub const ICMPV6: u8 = 58;
}
/// The remote-address keywords of RA4/RA6 this model evaluates: what the
/// network stack knows the remote address to be.
pub mod keyword {
    /// "LocalSubnet": on the adapter's own subnet (IPv6: on-link).
    pub const LOCAL_SUBNET: u8 = 0x1;
    /// "DefaultGateway".
    pub const DEFAULT_GATEWAY: u8 = 0x2;
    /// "DNS": a configured DNS server.
    pub const DNS: u8 = 0x4;
    /// "DHCP": the DHCP server that gave the lease.
    pub const DHCP: u8 = 0x8;
}
/// The most rules the store keeps.
pub const MAX_RULES: usize = 1024;
/// The longest rule text accepted.
pub const MAX_RULE_TEXT: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Allow,
    Block,
}
/// A port range, inclusive.
pub type Ports = (u16, u16);
/// An IPv4 network: address and prefix length.
pub type Net = ([u8; 4], u8);
/// An IPv6 network: address and prefix length.
pub type Net6 = ([u8; 16], u8);

/// A remote address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Address {
    V4([u8; 4]),
    V6([u8; 16]),
}
impl Address {
    /// The loopback interface's: 127.0.0.0/8, ::1 and IPv4-mapped 127/8.
    pub fn is_loopback(&self) -> bool {
        match self {
            Address::V4(a) => a[0] == 127,
            Address::V6(a) => {
                *a == V6_LOOPBACK
                    || (a[..10] == [0; 10] && a[10..12] == [0xff, 0xff] && a[12] == 127)
            }
        }
    }
    /// As one 128-bit number (IPv4 as itself), for ranges.
    fn value(&self) -> u128 {
        match self {
            Address::V4(a) => u128::from(u32::from_be_bytes(*a)),
            Address::V6(a) => u128::from_be_bytes(*a),
        }
    }
}
const V6_LOOPBACK: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// The registry value name (a GUID-like id, or the netsh rule's own).
    pub id: String,
    pub name: String,
    pub description: String,
    /// EmbedCtxt: the group ("@FirewallAPI.dll,-28502" or a plain name).
    pub group: String,
    pub enabled: bool,
    pub direction: Direction,
    pub action: Action,
    /// None: any protocol.
    pub protocol: Option<u8>,
    /// Empty: any port.
    pub local_ports: Vec<Ports>,
    pub remote_ports: Vec<Ports>,
    /// Remote addresses (RA4, RA6): networks, ranges and keywords. All
    /// empty and no keyword: any address.
    pub remote_addresses: Vec<Net>,
    pub remote_addresses6: Vec<Net6>,
    pub remote_ranges: Vec<(Address, Address)>,
    pub remote_keywords: u8,
    /// ICMP4/ICMP6 "type:code" conditions (code None: "*").
    pub icmp: Vec<(u8, Option<u8>)>,
    /// The program, compared case-insensitively; None: any.
    pub program: Option<String>,
    /// Profile bits; ALL when the rule names none.
    pub profiles: u8,
    /// Fields kept verbatim ("Key=Value"): conditions the model cannot
    /// evaluate, and keys it does not know.
    pub extra: Vec<String>,
    /// The rule has a condition this model cannot evaluate (fails closed).
    pub unmodelled: bool,
}

/// Why a rule text was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Malformed;

/// Keys that describe the rule without restricting what it matches.
const DESCRIPTIVE: [&str; 6] = [
    "Edge",
    "Defer",
    "Platform",
    "Platform2",
    "RuleVersion",
    "TTK",
];

impl Rule {
    /// A rule from its registry form ("v2.xx|Key=Value|...|").
    pub fn parse(id: &str, text: &str) -> Result<Self, Malformed> {
        if text.len() > MAX_RULE_TEXT {
            return Err(Malformed);
        }
        let mut fields = text.split('|');
        let version = fields.next().ok_or(Malformed)?;
        if !version.starts_with("v2.") {
            return Err(Malformed);
        }
        let mut rule = Rule {
            id: String::from(id),
            name: String::new(),
            description: String::new(),
            group: String::new(),
            enabled: true,
            direction: Direction::In,
            action: Action::Allow,
            protocol: None,
            local_ports: Vec::new(),
            remote_ports: Vec::new(),
            remote_addresses: Vec::new(),
            remote_addresses6: Vec::new(),
            remote_ranges: Vec::new(),
            remote_keywords: 0,
            icmp: Vec::new(),
            program: None,
            profiles: 0,
            extra: Vec::new(),
            unmodelled: false,
        };
        let mut seen_direction = false;
        let mut seen_action = false;
        for field in fields.filter(|f| !f.is_empty()) {
            let (key, value) = field.split_once('=').ok_or(Malformed)?;
            match key {
                "Action" => {
                    rule.action = match value {
                        "Allow" => Action::Allow,
                        "Block" => Action::Block,
                        _ => return Err(Malformed),
                    };
                    seen_action = true;
                }
                "Active" => {
                    rule.enabled = match value {
                        "TRUE" => true,
                        "FALSE" => false,
                        _ => return Err(Malformed),
                    }
                }
                "Dir" => {
                    rule.direction = match value {
                        "In" => Direction::In,
                        "Out" => Direction::Out,
                        _ => return Err(Malformed),
                    };
                    seen_direction = true;
                }
                "Protocol" => rule.protocol = Some(value.parse().map_err(|_| Malformed)?),
                "Profile" => {
                    rule.profiles |= match value {
                        "Domain" => profile::DOMAIN,
                        "Private" => profile::PRIVATE,
                        "Public" => profile::PUBLIC,
                        _ => return Err(Malformed),
                    }
                }
                "LPort" | "RPort" | "LPort2_10" | "RPort2_10" => {
                    let (list, keywords) = ports(value)?;
                    if keywords {
                        // "RPC", "IPHTTPS", "Ply2Disc", ...: not modelled.
                        rule.unmodelled = true;
                        rule.extra.push(String::from(field));
                    } else if key.starts_with('L') {
                        rule.local_ports.extend(list);
                    } else {
                        rule.remote_ports.extend(list);
                    }
                }
                "RA4" | "RA6" => {
                    for part in value.split(',').map(str::trim) {
                        match remote(part)? {
                            Remote::Any => (),
                            Remote::Net4(n) => rule.remote_addresses.push(n),
                            Remote::Net6(n) => rule.remote_addresses6.push(n),
                            Remote::Range(a, b) => rule.remote_ranges.push((a, b)),
                            Remote::Keyword(k) => rule.remote_keywords |= k,
                            Remote::Unmodelled => {
                                rule.unmodelled = true;
                                rule.extra.push(format!("{key}={part}"));
                            }
                        }
                    }
                }
                "ICMP4" | "ICMP6" => rule.icmp.push(icmp(value)?),
                "App" => rule.program = Some(String::from(value)),
                "Name" => rule.name = String::from(value),
                "Desc" => rule.description = String::from(value),
                "EmbedCtxt" => rule.group = String::from(value),
                _ if DESCRIPTIVE.contains(&key) => rule.extra.push(String::from(field)),
                // Services, interfaces, local addresses, authentication,
                // packages and keys this model does not know.
                _ => {
                    rule.unmodelled = true;
                    rule.extra.push(String::from(field));
                }
            }
            if rule.local_ports.len() > 64
                || rule.remote_ports.len() > 64
                || rule.remote_addresses.len() + rule.remote_addresses6.len() > 64
                || rule.remote_ranges.len() > 64
                || rule.icmp.len() > 64
                || rule.extra.len() > 64
            {
                return Err(Malformed);
            }
        }
        if !seen_direction || !seen_action || rule.name.is_empty() {
            return Err(Malformed);
        }
        if rule.profiles == 0 {
            rule.profiles = profile::ALL;
        }
        Ok(rule)
    }
    /// The registry form, as netsh writes it.
    pub fn text(&self) -> String {
        let mut s = String::from("v2.33|");
        s.push_str(match self.action {
            Action::Allow => "Action=Allow|",
            Action::Block => "Action=Block|",
        });
        s.push_str(if self.enabled {
            "Active=TRUE|"
        } else {
            "Active=FALSE|"
        });
        s.push_str(match self.direction {
            Direction::In => "Dir=In|",
            Direction::Out => "Dir=Out|",
        });
        if let Some(p) = self.protocol {
            s.push_str(&format!("Protocol={p}|"));
        }
        if self.profiles != profile::ALL {
            for (bit, name) in [
                (profile::DOMAIN, "Domain"),
                (profile::PRIVATE, "Private"),
                (profile::PUBLIC, "Public"),
            ] {
                if self.profiles & bit != 0 {
                    s.push_str(&format!("Profile={name}|"));
                }
            }
        }
        for (key, list) in [("LPort", &self.local_ports), ("RPort", &self.remote_ports)] {
            for (a, b) in list {
                if a == b {
                    s.push_str(&format!("{key}={a}|"));
                } else {
                    s.push_str(&format!("{key}={a}-{b}|"));
                }
            }
        }
        for (a, prefix) in &self.remote_addresses {
            s.push_str(&format!(
                "RA4={}.{}.{}.{}/{}|",
                a[0], a[1], a[2], a[3], prefix
            ));
        }
        for (a, prefix) in &self.remote_addresses6 {
            s.push_str(&format!("RA6={}/{prefix}|", v6_text(a)));
        }
        for (a, b) in &self.remote_ranges {
            match (a, b) {
                (Address::V4(a), Address::V4(b)) => s.push_str(&format!(
                    "RA4={}.{}.{}.{}-{}.{}.{}.{}|",
                    a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]
                )),
                (Address::V6(a), Address::V6(b)) => {
                    s.push_str(&format!("RA6={}-{}|", v6_text(a), v6_text(b)))
                }
                _ => (),
            }
        }
        for (bit, name) in KEYWORDS {
            if self.remote_keywords & bit != 0 {
                s.push_str(&format!("RA4={name}|RA6={name}|"));
            }
        }
        for (t, code) in &self.icmp {
            let key = if self.protocol == Some(protocol::ICMPV6) {
                "ICMP6"
            } else {
                "ICMP4"
            };
            match code {
                Some(c) => s.push_str(&format!("{key}={t}:{c}|")),
                None => s.push_str(&format!("{key}={t}:*|")),
            }
        }
        if let Some(app) = &self.program {
            s.push_str(&format!("App={app}|"));
        }
        for field in &self.extra {
            s.push_str(field);
            s.push('|');
        }
        s.push_str(&format!("Name={}|", self.name));
        if !self.description.is_empty() {
            s.push_str(&format!("Desc={}|", self.description));
        }
        if !self.group.is_empty() {
            s.push_str(&format!("EmbedCtxt={}|", self.group));
        }
        s
    }
    /// The remote addresses as netsh shows them ("10.0.0.5/255.255.255.255",
    /// "10.0.0.0/8", "fe80::/64", "a-b", "LocalSubnet"); empty: any.
    pub fn remote_list(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .remote_addresses
            .iter()
            .map(|(a, p)| {
                if *p == 32 {
                    format!("{}.{}.{}.{}/255.255.255.255", a[0], a[1], a[2], a[3])
                } else {
                    format!("{}.{}.{}.{}/{p}", a[0], a[1], a[2], a[3])
                }
            })
            .collect();
        out.extend(
            self.remote_addresses6
                .iter()
                .map(|(a, p)| format!("{}/{p}", v6_text(a))),
        );
        for (a, b) in &self.remote_ranges {
            let text = |x: &Address| match x {
                Address::V4(v) => format!("{}.{}.{}.{}", v[0], v[1], v[2], v[3]),
                Address::V6(v) => v6_text(v),
            };
            out.push(format!("{}-{}", text(a), text(b)));
        }
        out.extend(
            KEYWORDS
                .iter()
                .filter(|(bit, _)| self.remote_keywords & bit != 0)
                .map(|(_, name)| String::from(*name)),
        );
        out
    }
    /// Whether the rule names remote addresses at all.
    fn restricts_remote(&self) -> bool {
        !self.remote_addresses.is_empty()
            || !self.remote_addresses6.is_empty()
            || !self.remote_ranges.is_empty()
            || self.remote_keywords != 0
    }
    fn remote_matches(&self, c: &Connection<'_>) -> bool {
        if !self.restricts_remote() {
            return true;
        }
        let address = c.remote_address;
        let network = match address {
            Address::V4(a) => self.remote_addresses.iter().any(|n| inside(a, *n)),
            Address::V6(a) => self.remote_addresses6.iter().any(|n| inside6(&a, n)),
        };
        network
            || self.remote_ranges.iter().any(|(a, b)| {
                core::mem::discriminant(a) == core::mem::discriminant(&address)
                    && (a.value()..=b.value()).contains(&address.value())
            })
            || self.remote_keywords & c.remote_kind != 0
    }
    fn matches(&self, c: &Connection<'_>, active: u8) -> bool {
        // A condition the model cannot evaluate: an allow rule never
        // matches; a block rule applies as if it held.
        if self.unmodelled && self.action == Action::Allow {
            return false;
        }
        self.enabled
            && self.direction == c.direction
            && self.profiles & active != 0
            && self.protocol.is_none_or(|p| p == c.protocol)
            && (self.local_ports.is_empty()
                || self
                    .local_ports
                    .iter()
                    .any(|(a, b)| (*a..=*b).contains(&c.local_port)))
            && (self.remote_ports.is_empty()
                || self
                    .remote_ports
                    .iter()
                    .any(|(a, b)| (*a..=*b).contains(&c.remote_port)))
            && self.remote_matches(c)
            && (self.icmp.is_empty()
                || c.icmp.is_some_and(|(t, code)| {
                    self.icmp
                        .iter()
                        .any(|(rt, rc)| *rt == t && rc.is_none_or(|rc| rc == code))
                }))
            && self
                .program
                .as_deref()
                .is_none_or(|p| c.program.is_some_and(|q| p.eq_ignore_ascii_case(q)))
    }
}
const KEYWORDS: [(u8, &str); 4] = [
    (keyword::LOCAL_SUBNET, "LocalSubnet"),
    (keyword::DEFAULT_GATEWAY, "DefaultGateway"),
    (keyword::DNS, "DNS"),
    (keyword::DHCP, "DHCP"),
];
/// "80", "1000-2000", "80,443" (netsh lists) in a rule field; true when a
/// part is a keyword ("RPC", "IPHTTPS", ...).
fn ports(value: &str) -> Result<(Vec<Ports>, bool), Malformed> {
    let mut out = Vec::new();
    let mut keywords = false;
    for part in value.split(',').map(str::trim) {
        if part.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            keywords = true;
            continue;
        }
        let (a, b) = part.split_once('-').unwrap_or((part, part));
        let a: u16 = a.trim().parse().map_err(|_| Malformed)?;
        let b: u16 = b.trim().parse().map_err(|_| Malformed)?;
        if a > b {
            return Err(Malformed);
        }
        out.push((a, b));
    }
    Ok((out, keywords))
}
enum Remote {
    Any,
    Net4(Net),
    Net6(Net6),
    Range(Address, Address),
    Keyword(u8),
    Unmodelled,
}
/// One RA4/RA6 entry: "*", "10.0.0.0/8", "192.168.1.5", "a-b", "fe80::/64",
/// or a keyword ("LocalSubnet", "Intranet", ...).
fn remote(part: &str) -> Result<Remote, Malformed> {
    if part == "*" {
        return Ok(Remote::Any);
    }
    if part.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && !part.contains(':') {
        return Ok(KEYWORDS
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(part))
            .map_or(Remote::Unmodelled, |(bit, _)| Remote::Keyword(*bit)));
    }
    if let Some((a, b)) = part.split_once('-') {
        let (a, b) = (address(a.trim())?, address(b.trim())?);
        if core::mem::discriminant(&a) != core::mem::discriminant(&b) || a.value() > b.value() {
            return Err(Malformed);
        }
        return Ok(Remote::Range(a, b));
    }
    let (text, prefix) = match part.split_once('/') {
        Some((a, p)) => (a, Some(p)),
        None => (part, None),
    };
    match address(text)? {
        Address::V4(a) => {
            let prefix = match prefix {
                None => 32,
                // "a.b.c.d/255.255.0.0" (netsh shows masks).
                Some(p) if p.contains('.') => mask_prefix(p)?,
                Some(p) => p.parse().map_err(|_| Malformed)?,
            };
            if prefix > 32 {
                return Err(Malformed);
            }
            Ok(Remote::Net4((a, prefix)))
        }
        Address::V6(a) => {
            let prefix: u8 = prefix.map_or(Ok(128), |p| p.parse().map_err(|_| Malformed))?;
            if prefix > 128 {
                return Err(Malformed);
            }
            Ok(Remote::Net6((a, prefix)))
        }
    }
}
fn mask_prefix(mask: &str) -> Result<u8, Malformed> {
    let Address::V4(m) = address(mask)? else {
        return Err(Malformed);
    };
    let m = u32::from_be_bytes(m);
    let ones = m.leading_ones();
    if m.checked_shl(ones).unwrap_or(0) != 0 {
        return Err(Malformed);
    }
    Ok(ones as u8)
}
/// A dotted IPv4 or a textual IPv6 address (RFC 4291 section 2.2).
pub fn address(text: &str) -> Result<Address, Malformed> {
    if !text.contains(':') {
        let mut octets = [0u8; 4];
        let mut fields = text.split('.');
        for o in &mut octets {
            *o = fields
                .next()
                .ok_or(Malformed)?
                .parse()
                .map_err(|_| Malformed)?;
        }
        if fields.next().is_some() {
            return Err(Malformed);
        }
        return Ok(Address::V4(octets));
    }
    let groups = |s: &str| -> Result<Vec<u16>, Malformed> {
        if s.is_empty() {
            return Ok(Vec::new());
        }
        s.split(':')
            .map(|g| {
                if g.is_empty() || g.len() > 4 {
                    return Err(Malformed);
                }
                u16::from_str_radix(g, 16).map_err(|_| Malformed)
            })
            .collect()
    };
    let (head, tail) = match text.split_once("::") {
        Some((h, t)) => {
            if t.contains("::") {
                return Err(Malformed);
            }
            (groups(h)?, Some(groups(t)?))
        }
        None => (groups(text)?, None),
    };
    let mut words = [0u16; 8];
    match tail {
        None if head.len() == 8 => words.copy_from_slice(&head),
        Some(tail) if head.len() + tail.len() < 8 => {
            words[..head.len()].copy_from_slice(&head);
            words[8 - tail.len()..].copy_from_slice(&tail);
        }
        _ => return Err(Malformed),
    }
    let mut out = [0u8; 16];
    for (i, w) in words.iter().enumerate() {
        out[2 * i..2 * i + 2].copy_from_slice(&w.to_be_bytes());
    }
    Ok(Address::V6(out))
}
fn v6_text(a: &[u8; 16]) -> String {
    let words: Vec<String> = a
        .chunks(2)
        .map(|w| format!("{:x}", u16::from_be_bytes([w[0], w[1]])))
        .collect();
    words.join(":")
}
/// "8:*", "8:0", "128:*" in ICMP4/ICMP6.
fn icmp(value: &str) -> Result<(u8, Option<u8>), Malformed> {
    let (t, code) = value.split_once(':').unwrap_or((value, "*"));
    let t = t.parse().map_err(|_| Malformed)?;
    let code = if code == "*" {
        None
    } else {
        Some(code.parse().map_err(|_| Malformed)?)
    };
    Ok((t, code))
}
fn inside(address: [u8; 4], (net, prefix): Net) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    u32::from_be_bytes(address) & mask == u32::from_be_bytes(net) & mask
}
fn inside6(address: &[u8; 16], (net, prefix): &Net6) -> bool {
    let mask = if *prefix == 0 {
        0
    } else {
        u128::MAX << (128 - u32::from(*prefix))
    };
    u128::from_be_bytes(*address) & mask == u128::from_be_bytes(*net) & mask
}

/// One profile's settings (Settings > Windows Security > Firewall).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileState {
    pub enabled: bool,
    /// The default for inbound connections no rule allows: Windows blocks.
    pub inbound_allow: bool,
    /// "Block all incoming connections, including those in the list of
    /// allowed apps".
    pub shields_up: bool,
    pub outbound_allow: bool,
}
impl Default for ProfileState {
    fn default() -> Self {
        Self {
            enabled: true,
            inbound_allow: false,
            shields_up: false,
            outbound_allow: true,
        }
    }
}

/// What door-network asks about one connection or datagram.
#[derive(Clone, Copy, Debug)]
pub struct Connection<'a> {
    pub direction: Direction,
    pub protocol: u8,
    pub local_port: u16,
    pub remote_address: Address,
    pub remote_port: u16,
    /// What the stack knows the remote address to be (`keyword` bits).
    pub remote_kind: u8,
    /// ICMP and ICMPv6: the message's type and code.
    pub icmp: Option<(u8, u8)>,
    /// The socket owner's program (its Windows path), if known.
    pub program: Option<&'a str>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Policy {
    /// Domain, private, public.
    pub profiles: [ProfileState; 3],
    pub rules: Vec<Rule>,
}
impl Policy {
    /// The settings of profile `active` (one of the profile bits).
    pub fn state(&self, active: u8) -> ProfileState {
        match active {
            profile::DOMAIN => self.profiles[0],
            profile::PRIVATE => self.profiles[1],
            _ => self.profiles[2],
        }
    }
    /// Whether the connection may proceed on a network of profile `active`
    /// (one of the profile bits).
    pub fn permits(&self, c: &Connection<'_>, active: u8) -> bool {
        // The loopback interface is never filtered.
        if c.remote_address.is_loopback() {
            return true;
        }
        let state = self.state(active);
        if !state.enabled {
            return true;
        }
        if self
            .rules
            .iter()
            .any(|r| r.action == Action::Block && r.matches(c, active))
        {
            return false;
        }
        match c.direction {
            Direction::In => {
                !state.shields_up
                    && (state.inbound_allow
                        || self
                            .rules
                            .iter()
                            .any(|r| r.action == Action::Allow && r.matches(c, active)))
            }
            Direction::Out => {
                state.outbound_allow
                    || self
                        .rules
                        .iter()
                        .any(|r| r.action == Action::Allow && r.matches(c, active))
            }
        }
    }
}

#[cfg(test)]
mod tests;
