//! Windows Defender Firewall in the stack (`door_firewall`'s policy). From
//! other hosts, a new TCP connection reaches a listener only when a rule
//! allows it (or the profile's default does), and a datagram reaches a
//! socket when it answers what the socket sent (the firewall is stateful) or
//! a rule allows it; refused ones are dropped silently, as Windows' stealth
//! mode does (no reset, no "port unreachable"). An outbound connection or
//! datagram a block rule names (or every one no rule allows, when outbound
//! is blocked) fails with Access denied. The loopback interface, and this
//! host's own addresses, are never filtered. Init keeps the policy (HKLM's
//! FirewallRules and the profile switches, loaded at boot and after every
//! change) and names the program each Windows process runs; a program that
//! listens while no rule names it is queued for the window server's
//! "Windows Security Alert".
use crate::{Network, net, word};
use alloc::{rc::Rc, string::String, vec::Vec};
use door_abi::{Handle, platform::Identity, platform::manifest};
use door_firewall::{Address, Connection, Direction, Policy, ProfileState, Rule, keyword};
use door_service_protocol::{Code, net::firewall as f};
use door_system_services::server::Response;
use smoltcp::wire::IpAddress;

/// Programs remembered, alerts waiting, peers a datagram socket sent to.
const PROGRAMS: usize = 128;
const ALERTS: usize = 16;
pub const CONTACTED: usize = 16;

#[derive(Default)]
pub struct Firewall {
    pub policy: Rc<Policy>,
    /// Init's rules being handed over.
    staging: Vec<u8>,
    /// The program of each process Init launched: (pid, Windows path).
    programs: Vec<(u64, String)>,
    /// Programs blocked from listening for the Security Alert, and those
    /// already asked about (once per program while the stack runs, as
    /// Windows asks once).
    alerts: Vec<(u64, String)>,
    asked: Vec<String>,
}

/// What the stack knows about the networks it is on: this host's own
/// addresses, and what RA4/RA6 keywords ("LocalSubnet", "DefaultGateway",
/// "DNS") stand for.
#[derive(Clone, Default)]
pub struct Locality {
    /// The adapter's addresses (not the loopback interface's).
    pub own4: Vec<[u8; 4]>,
    pub own6: Vec<[u8; 16]>,
    pub subnet4: Option<([u8; 4], u8)>,
    pub gateway4: Option<[u8; 4]>,
    pub prefix6: Option<([u8; 16], u8)>,
    pub router6: Option<[u8; 16]>,
    pub dns: Vec<Address>,
}
fn same_prefix(a: u128, b: u128, prefix: u8, width: u32) -> bool {
    let prefix = u32::from(prefix).min(width);
    if prefix == 0 {
        return true;
    }
    let shift = width - prefix;
    a >> shift == b >> shift
}
impl Locality {
    /// Whether `a` is one of the adapter's addresses.
    pub fn own(&self, a: &Address) -> bool {
        match a {
            Address::V4(v) => *v != [0; 4] && self.own4.contains(v),
            Address::V6(v) => *v != [0; 16] && self.own6.contains(v),
        }
    }
    /// The `door_firewall::keyword` bits `a` answers to.
    pub fn kind(&self, a: Address) -> u8 {
        let mut k = 0;
        match a {
            Address::V4(v) => {
                let value = u128::from(u32::from_be_bytes(v));
                if self.subnet4.is_some_and(|(n, p)| {
                    same_prefix(value, u128::from(u32::from_be_bytes(n)), p, 32)
                }) {
                    k |= keyword::LOCAL_SUBNET;
                }
                if self.gateway4 == Some(v) {
                    k |= keyword::DEFAULT_GATEWAY;
                }
            }
            Address::V6(v) => {
                let link_local = v[0] == 0xfe && v[1] & 0xc0 == 0x80;
                if link_local
                    || self.prefix6.is_some_and(|(n, p)| {
                        same_prefix(u128::from_be_bytes(v), u128::from_be_bytes(n), p, 128)
                    })
                {
                    k |= keyword::LOCAL_SUBNET;
                }
                if self.router6 == Some(v) {
                    k |= keyword::DEFAULT_GATEWAY;
                }
            }
        }
        if self.dns.contains(&a) {
            k |= keyword::DNS;
        }
        k
    }
}

/// What the frames in and out are checked against: the policy, the TCP
/// listeners and the stack's own addresses, kept beside the adapter
/// (`Nic`).
#[derive(Default)]
pub struct Gate {
    pub policy: Rc<Policy>,
    pub profile: u8,
    /// TCP listeners: port, bound address (None: any), program.
    pub listeners: Vec<(u16, Option<Address>, Option<String>)>,
    pub locality: Locality,
}
/// The IP fields of a frame the gate needs.
struct Packet<'a> {
    protocol: u8,
    source: Address,
    destination: Address,
    /// The transport header and data this frame carries.
    payload: &'a [u8],
    /// IPv4: the fragment offset in 8-byte units.
    offset: u16,
    /// IPv6: a Fragment header (the stack does not reassemble IPv6).
    fragment6: bool,
}
enum Parsed<'a> {
    /// ARP and other non-IP frames.
    Other,
    /// An IP frame too short or inconsistent to be read.
    Malformed,
    Ip(Packet<'a>),
}
const ETHERTYPE_IPV4: [u8; 2] = [0x08, 0x00];
const ETHERTYPE_IPV6: [u8; 2] = [0x86, 0xdd];
fn packet(frame: &[u8]) -> Parsed<'_> {
    if frame.len() < 14 {
        return Parsed::Other;
    }
    let ip = &frame[14..];
    if frame[12..14] == ETHERTYPE_IPV4 {
        if ip.len() < 20 || ip[0] >> 4 != 4 {
            return Parsed::Malformed;
        }
        let ihl = usize::from(ip[0] & 0x0f) * 4;
        let total = usize::from(u16::from_be_bytes([ip[2], ip[3]])).min(ip.len());
        if ihl < 20 || total < ihl {
            return Parsed::Malformed;
        }
        let fragment = u16::from_be_bytes([ip[6], ip[7]]);
        return Parsed::Ip(Packet {
            protocol: ip[9],
            source: Address::V4([ip[12], ip[13], ip[14], ip[15]]),
            destination: Address::V4([ip[16], ip[17], ip[18], ip[19]]),
            payload: &ip[ihl..total],
            offset: fragment & 0x1fff,
            fragment6: false,
        });
    }
    if frame[12..14] != ETHERTYPE_IPV6 {
        return Parsed::Other;
    }
    if ip.len() < 40 || ip[0] >> 4 != 6 {
        return Parsed::Malformed;
    }
    let end = (40 + usize::from(u16::from_be_bytes([ip[4], ip[5]]))).min(ip.len());
    let mut next = ip[6];
    let mut at = 40;
    let mut fragment6 = false;
    // Hop-by-hop, routing, fragment and destination options headers.
    for _ in 0..8 {
        match next {
            0 | 43 | 60 => {
                if at + 2 > end {
                    return Parsed::Malformed;
                }
                next = ip[at];
                at += (usize::from(ip[at + 1]) + 1) * 8;
            }
            44 => {
                if at + 8 > end {
                    return Parsed::Malformed;
                }
                next = ip[at];
                fragment6 = true;
                at += 8;
            }
            _ => break,
        }
        if at > end {
            return Parsed::Malformed;
        }
    }
    let address = |r: core::ops::Range<usize>| {
        let mut a = [0u8; 16];
        a.copy_from_slice(&ip[r]);
        Address::V6(a)
    };
    Parsed::Ip(Packet {
        protocol: next,
        source: address(8..24),
        destination: address(24..40),
        payload: &ip[at.min(end)..end],
        offset: 0,
        fragment6,
    })
}
/// The bytes of a transport header a first fragment must carry whole
/// (RFC 1858 against tiny fragments).
fn header_bytes(protocol: u8) -> usize {
    match protocol {
        door_firewall::protocol::TCP => 20,
        door_firewall::protocol::UDP
        | door_firewall::protocol::ICMP
        | door_firewall::protocol::ICMPV6 => 8,
        _ => 0,
    }
}
impl Gate {
    fn state(&self) -> ProfileState {
        self.policy.state(self.profile)
    }
    fn inbound(
        &self,
        p: &Packet<'_>,
        local_port: u16,
        remote_port: u16,
        program: Option<&str>,
    ) -> bool {
        let icmp = matches!(
            p.protocol,
            door_firewall::protocol::ICMP | door_firewall::protocol::ICMPV6
        )
        .then(|| (p.payload[0], p.payload.get(1).copied().unwrap_or(0)));
        self.policy.permits(
            &Connection {
                direction: Direction::In,
                protocol: p.protocol,
                local_port,
                remote_address: p.source,
                remote_port,
                remote_kind: self.locality.kind(p.source),
                icmp,
                program,
            },
            self.profile,
        )
    }
    /// A frame from the wire (or out of a VPN tunnel) for the stack.
    /// Whatever the firewall's state, another host never sends from or to
    /// the loopback interface's addresses or from this host's own, and
    /// IPv6 fragments (which the stack does not reassemble) go no further.
    /// With the firewall on, TCP connection requests and ICMP echo and
    /// timestamp requests need the policy's consent, and a fragmented
    /// datagram's first fragment must carry its whole transport header;
    /// everything else goes on (connections already made, replies;
    /// datagrams are checked when a socket takes them).
    pub fn admits(&self, frame: &[u8]) -> bool {
        let p = match packet(frame) {
            Parsed::Other => return true,
            Parsed::Malformed => return false,
            Parsed::Ip(p) => p,
        };
        if p.source.is_loopback()
            || p.destination.is_loopback()
            || self.locality.own(&p.source)
            || p.fragment6
        {
            return false;
        }
        if !self.state().enabled {
            return true;
        }
        let tcp = p.protocol == door_firewall::protocol::TCP;
        if p.offset != 0 {
            // A later fragment: an offset of 8 bytes would overwrite the
            // TCP flags of the first (RFC 1858).
            return !(tcp && p.offset == 1);
        }
        if p.payload.len() < header_bytes(p.protocol) {
            return false;
        }
        match p.protocol {
            door_firewall::protocol::TCP => {
                // SYN without ACK: a new connection.
                if p.payload[13] & 0x12 != 0x02 {
                    return true;
                }
                let remote_port = u16::from_be_bytes([p.payload[0], p.payload[1]]);
                let port = u16::from_be_bytes([p.payload[2], p.payload[3]]);
                // To a port nothing listens on: dropped without a reset.
                let Some((_, _, program)) = self
                    .listeners
                    .iter()
                    .find(|(lp, a, _)| *lp == port && a.is_none_or(|a| a == p.destination))
                else {
                    return false;
                };
                self.inbound(&p, port, remote_port, program.as_deref())
            }
            door_firewall::protocol::ICMP if matches!(p.payload[0], 8 | 13) => {
                self.inbound(&p, 0, 0, None)
            }
            door_firewall::protocol::ICMPV6 if p.payload[0] == 128 => self.inbound(&p, 0, 0, None),
            _ => true,
        }
    }
    /// A frame the stack sends to the wire: stealth mode keeps "port
    /// unreachable" answers in (ICMP 3/3, ICMPv6 1/4).
    pub fn sends(&self, frame: &[u8]) -> bool {
        if !self.state().enabled {
            return true;
        }
        let Parsed::Ip(p) = packet(frame) else {
            return true;
        };
        p.destination.is_loopback()
            || !matches!(
                (p.protocol, p.payload),
                (door_firewall::protocol::ICMP, [3, 3, ..])
                    | (door_firewall::protocol::ICMPV6, [1, 4, ..])
            )
    }
}
/// A socket address as the policy sees it (IPv4-mapped IPv6 as IPv4).
pub fn address_of(a: IpAddress) -> Address {
    match a {
        IpAddress::Ipv4(v) => Address::V4(v.octets()),
        IpAddress::Ipv6(v) => match net::v6::unmapped(&v.octets()) {
            Some(v4) => Address::V4(v4),
            None => Address::V6(v.octets()),
        },
    }
}
/// A profile's byte (`net::firewall::profile`).
pub fn state_byte(p: ProfileState) -> u8 {
    let mut b = 0;
    if p.enabled {
        b |= f::profile::ON;
    }
    if p.inbound_allow {
        b |= f::profile::INBOUND_ALLOW;
    }
    if p.shields_up {
        b |= f::profile::SHIELDS_UP;
    }
    if !p.outbound_allow {
        b |= f::profile::OUTBOUND_BLOCK;
    }
    b
}
pub fn profile_from(b: u8) -> ProfileState {
    ProfileState {
        enabled: b & f::profile::ON != 0,
        inbound_allow: b & f::profile::INBOUND_ALLOW != 0,
        shields_up: b & f::profile::SHIELDS_UP != 0,
        outbound_allow: b & f::profile::OUTBOUND_BLOCK == 0,
    }
}

impl Network {
    /// The active network's profile: Doors' adapter joins networks as
    /// public, Windows' default for a network it has not been told about.
    pub(crate) fn active_profile(&self) -> u8 {
        door_firewall::profile::PUBLIC
    }
    fn program_of(&self, pid: u64) -> Option<&str> {
        self.firewall
            .programs
            .iter()
            .find(|(p, _)| *p == pid)
            .map(|(_, path)| path.as_str())
    }
    /// Hand the policy and the listeners to the adapter's frame checks.
    pub(crate) fn refresh_gate(&mut self) {
        let listeners = self
            .clients
            .iter()
            .filter(|s| s.kind == net::TCP && !s.pool.is_empty())
            .map(|s| {
                (
                    s.local,
                    s.address.map(address_of),
                    self.program_of(s.pid).map(String::from),
                )
            })
            .collect();
        if let Some(nic) = self.nic.as_ref() {
            let mut gate = nic.gate.borrow_mut();
            gate.policy = self.firewall.policy.clone();
            gate.profile = self.active_profile();
            gate.listeners = listeners;
        }
    }
    /// Hand the stack's addresses (its own, the subnet, gateway, router
    /// and DNS servers) to the adapter's frame checks.
    pub(crate) fn refresh_locality(&mut self, own6: &[[u8; 16]]) {
        let locality = Locality {
            own4: self
                .address
                .map(|c| alloc::vec![c.address().octets()])
                .unwrap_or_default(),
            own6: own6.to_vec(),
            subnet4: self.address.map(|c| (c.address().octets(), c.prefix_len())),
            gateway4: self.gateway.map(|g| g.octets()),
            prefix6: self
                .slaac
                .as_ref()
                .and_then(|s| s.global)
                .map(|(c, _)| (c.address().octets(), c.prefix_len())),
            router6: self
                .slaac
                .as_ref()
                .and_then(|s| s.router)
                .map(|(r, _)| r.octets()),
            dns: self.servers().into_iter().map(address_of).collect(),
        };
        if let Some(nic) = self.nic.as_ref() {
            nic.gate.borrow_mut().locality = locality;
        }
    }
    /// Whether socket `index` may exchange traffic with `remote`:`port`.
    pub(crate) fn permitted(
        &self,
        index: usize,
        direction: Direction,
        remote: IpAddress,
        port: u16,
    ) -> bool {
        // This host's own addresses go by the loopback interface (frames
        // from the wire never carry them: `Gate::admits`).
        if self.is_own(remote) {
            return true;
        }
        let address = address_of(remote);
        let remote_kind = self
            .nic
            .as_ref()
            .map_or(0, |n| n.gate.borrow().locality.kind(address));
        let s = &self.clients[index];
        let protocol = match s.kind {
            net::TCP => door_firewall::protocol::TCP,
            net::UDP => door_firewall::protocol::UDP,
            _ => s.protocol,
        };
        let c = Connection {
            direction,
            protocol,
            local_port: s.local,
            remote_address: address,
            remote_port: port,
            remote_kind,
            icmp: None,
            program: self.program_of(s.pid),
        };
        self.firewall.policy.permits(&c, self.active_profile())
    }
    /// A datagram from `from` reaches socket `index`: replies to what it
    /// sent always do, others by the rules.
    pub(crate) fn datagram_permitted(&self, index: usize, from: IpAddress, port: u16) -> bool {
        self.clients[index].contacted.contains(&(from, port))
            || self.permitted(index, Direction::In, from, port)
    }
    /// Socket `index` sent to `to`: replies from there come in.
    pub(crate) fn contacted(&mut self, index: usize, to: IpAddress, port: u16) {
        let list = &mut self.clients[index].contacted;
        if !list.contains(&(to, port)) {
            if list.len() >= CONTACTED {
                list.remove(0);
            }
            list.push((to, port));
        }
    }
    /// Socket `index` started listening (TCP LISTEN, a UDP BIND to a port
    /// it names) where other hosts reach it: when the firewall is on and no
    /// inbound rule names its program, the window server is asked to show
    /// the Security Alert.
    pub(crate) fn listening(&mut self, index: usize) {
        self.refresh_gate();
        let s = &self.clients[index];
        // A socket bound to the loopback interface is out of other hosts'
        // reach.
        if s.address.is_some_and(|a| match a {
            IpAddress::Ipv4(v) => v.octets()[0] == 127,
            IpAddress::Ipv6(v) => v.is_loopback(),
        }) {
            return;
        }
        let pid = s.pid;
        let Some(program) = self.program_of(pid).map(String::from) else {
            return;
        };
        let policy = &self.firewall.policy;
        let named = policy.rules.iter().any(|r| {
            r.direction == Direction::In
                && r.program
                    .as_deref()
                    .is_some_and(|p| p.eq_ignore_ascii_case(&program))
        });
        if !policy.state(self.active_profile()).enabled
            || named
            || self
                .firewall
                .asked
                .iter()
                .any(|a| a.eq_ignore_ascii_case(&program))
            || self.firewall.alerts.len() >= ALERTS
        {
            return;
        }
        door_rt::diagnostic(format_args!(
            "DOOR_P15 FIREWALL BLOCKED program={program} pid={pid}\n"
        ));
        if self.firewall.asked.len() >= PROGRAMS {
            self.firewall.asked.remove(0);
        }
        self.firewall.asked.push(program.clone());
        self.firewall.alerts.push((pid, program));
    }
    /// door.network's firewall requests (`net::firewall`).
    pub(crate) fn firewall_request(
        &mut self,
        _channel: Handle,
        peer: Identity,
        opcode: u32,
        body: &[u8],
    ) -> Response {
        let init = peer.manifest == manifest::INIT;
        match opcode {
            f::STATE if body.is_empty() => {
                let mut out = [0u8; f::STATE_BYTES];
                for (i, p) in self.firewall.policy.profiles.iter().enumerate() {
                    out[i] = state_byte(*p);
                }
                out[3] = self.active_profile();
                let rules = self.firewall.policy.rules.len() as u32;
                out[8..12].copy_from_slice(&rules.to_le_bytes());
                let waiting = self.firewall.alerts.len() as u32;
                out[12..16].copy_from_slice(&waiting.to_le_bytes());
                Response::bytes(&out)
            }
            f::STAGE if init => {
                let Some(offset) = word(body, 0) else {
                    return Response::error(Code::InvalidRequest);
                };
                let data = &body[4..];
                if offset == 0 {
                    self.firewall.staging.clear();
                }
                if data.len() > f::PART
                    || offset as usize != self.firewall.staging.len()
                    || self.firewall.staging.len() + data.len() > f::MAX_POLICY
                {
                    return Response::error(Code::InvalidRequest);
                }
                self.firewall.staging.extend_from_slice(data);
                Response::scalar(self.firewall.staging.len() as u64)
            }
            f::APPLY if init => {
                if body.len() != 3 {
                    return Response::error(Code::InvalidRequest);
                }
                let staged = core::mem::take(&mut self.firewall.staging);
                let Ok(text) = core::str::from_utf8(&staged) else {
                    return Response::error(Code::InvalidRequest);
                };
                // One rule a line, "id\trule"; a damaged rule is skipped, as
                // Windows ignores one.
                let rules: Vec<Rule> = text
                    .lines()
                    .filter_map(|line| line.split_once('\t'))
                    .filter_map(|(id, rule)| Rule::parse(id, rule).ok())
                    .take(door_firewall::MAX_RULES)
                    .collect();
                let mut policy = Policy {
                    rules,
                    ..Policy::default()
                };
                for (i, b) in body.iter().enumerate() {
                    policy.profiles[i] = profile_from(*b);
                }
                door_rt::diagnostic(format_args!(
                    "DOOR_P15 FIREWALL POLICY rules={} public={:#x}\n",
                    policy.rules.len(),
                    body[2]
                ));
                // A program a rule now names is not asked about.
                self.firewall.alerts.retain(|(_, program)| {
                    !policy.rules.iter().any(|r| {
                        r.program
                            .as_deref()
                            .is_some_and(|p| p.eq_ignore_ascii_case(program))
                    })
                });
                self.firewall.policy = Rc::new(policy);
                self.refresh_gate();
                Response::empty()
            }
            f::PROGRAM if init => {
                let (Some(low), Some(high)) = (word(body, 0), word(body, 4)) else {
                    return Response::error(Code::InvalidRequest);
                };
                let pid = u64::from(low) | u64::from(high) << 32;
                let Ok(path) = core::str::from_utf8(&body[8..]) else {
                    return Response::error(Code::InvalidRequest);
                };
                if path.is_empty() || path.len() > 520 {
                    return Response::error(Code::InvalidRequest);
                }
                self.firewall.programs.retain(|(p, _)| *p != pid);
                if self.firewall.programs.len() >= PROGRAMS {
                    self.firewall.programs.remove(0);
                }
                self.firewall.programs.push((pid, String::from(path)));
                // A listener the program opened before Init named it.
                let listeners: Vec<usize> = (0..self.clients.len())
                    .filter(|&i| {
                        let s = &self.clients[i];
                        s.pid == pid && s.kind == net::TCP && !s.pool.is_empty()
                    })
                    .collect();
                for index in listeners {
                    self.listening(index);
                }
                self.refresh_gate();
                Response::empty()
            }
            f::ALERT if peer.manifest == manifest::WINDOW && body.is_empty() => {
                if self.firewall.alerts.is_empty() {
                    return Response::bytes(&[]);
                }
                let (pid, program) = self.firewall.alerts.remove(0);
                let mut out = Vec::with_capacity(8 + program.len());
                out.extend_from_slice(&pid.to_le_bytes());
                out.extend_from_slice(program.as_bytes());
                out.truncate(door_service_protocol::BODY_BYTES);
                Response::bytes(&out)
            }
            f::STAGE | f::APPLY | f::PROGRAM | f::ALERT => Response::error(Code::Denied),
            _ => Response::error(Code::InvalidRequest),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    /// An Ethernet + IPv4 + TCP frame with `flags`.
    fn tcp(source: [u8; 4], destination: [u8; 4], port: u16, flags: u8) -> Vec<u8> {
        let mut f = vec![0u8; 14 + 20 + 20];
        f[12] = 0x08;
        f[14] = 0x45;
        f[16..18].copy_from_slice(&40u16.to_be_bytes());
        f[23] = 6;
        f[26..30].copy_from_slice(&source);
        f[30..34].copy_from_slice(&destination);
        f[34..36].copy_from_slice(&50000u16.to_be_bytes());
        f[36..38].copy_from_slice(&port.to_be_bytes());
        f[47] = flags;
        f
    }
    fn gate(rules: &[&str]) -> Gate {
        let policy = Policy {
            rules: rules
                .iter()
                .enumerate()
                .map(|(i, r)| Rule::parse(&alloc::format!("r{i}"), r).unwrap())
                .collect(),
            ..Policy::default()
        };
        Gate {
            policy: Rc::new(policy),
            profile: door_firewall::profile::PUBLIC,
            listeners: vec![(8080, None, Some(String::from("C:\\app.exe")))],
            locality: Locality {
                own4: vec![[10, 0, 2, 15]],
                own6: vec![V6_ME],
                subnet4: Some(([10, 0, 2, 0], 24)),
                gateway4: Some([10, 0, 2, 2]),
                ..Locality::default()
            },
        }
    }
    const V6_ME: [u8; 16] = [0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x15];
    const V6_PEER: [u8; 16] = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2];
    /// An Ethernet + IPv6 frame: `header` (next header 6, 58 or 44), then
    /// `upper` (a TCP header with `flags`, an ICMPv6 message, ...).
    fn ip6(source: [u8; 16], destination: [u8; 16], next: u8, upper: &[u8]) -> Vec<u8> {
        let mut f = vec![0u8; 14 + 40];
        f[12..14].copy_from_slice(&[0x86, 0xdd]);
        f[14] = 0x60;
        f[18..20].copy_from_slice(&(upper.len() as u16).to_be_bytes());
        f[20] = next;
        f[22..38].copy_from_slice(&source);
        f[38..54].copy_from_slice(&destination);
        f.extend_from_slice(upper);
        f
    }
    fn tcp_header(port: u16, flags: u8) -> Vec<u8> {
        let mut t = vec![0u8; 20];
        t[0..2].copy_from_slice(&50000u16.to_be_bytes());
        t[2..4].copy_from_slice(&port.to_be_bytes());
        t[12] = 0x50;
        t[13] = flags;
        t
    }
    #[test]
    fn martians_ipv6_and_fragments() {
        let host = [10, 0, 2, 2];
        let me = [10, 0, 2, 15];
        let closed = gate(&[]);
        let mut off = gate(&[]);
        let mut policy = (*off.policy).clone();
        policy.profiles[2].enabled = false;
        off.policy = Rc::new(policy);
        // From another host: never from or to the loopback interface, never
        // from this host's own address, firewall on or off.
        for g in [&closed, &off] {
            assert!(!g.admits(&tcp([127, 0, 0, 1], me, 8080, 0x10)));
            assert!(!g.admits(&tcp(host, [127, 0, 0, 1], 8080, 0x10)));
            assert!(!g.admits(&tcp(me, me, 8080, 0x10)));
            let mut lo6 = [0u8; 16];
            lo6[15] = 1;
            assert!(!g.admits(&ip6(V6_PEER, lo6, 6, &tcp_header(8080, 0x10))));
            assert!(!g.admits(&ip6(V6_ME, V6_ME, 6, &tcp_header(8080, 0x10))));
            // IPv6 fragments are not reassembled.
            let mut fragment = vec![6, 0, 0, 1, 0, 0, 0, 7];
            fragment.extend_from_slice(&tcp_header(8080, 0x10));
            assert!(!g.admits(&ip6(V6_PEER, V6_ME, 44, &fragment)));
        }
        assert!(off.admits(&ip6(V6_PEER, V6_ME, 6, &tcp_header(8080, 0x02))));
        // IPv6 connection requests and echo requests need the policy too.
        assert!(!closed.admits(&ip6(V6_PEER, V6_ME, 6, &tcp_header(8080, 0x02))));
        assert!(closed.admits(&ip6(V6_PEER, V6_ME, 6, &tcp_header(8080, 0x10))));
        let open =
            gate(&["v2.33|Action=Allow|Active=TRUE|Dir=In|Protocol=6|App=c:\\APP.exe|Name=App|"]);
        assert!(open.admits(&ip6(V6_PEER, V6_ME, 6, &tcp_header(8080, 0x02))));
        let echo6 = [128u8, 0, 0, 0, 0, 1, 0, 1];
        assert!(!closed.admits(&ip6(V6_PEER, V6_ME, 58, &echo6)));
        assert!(
            gate(&["v2.33|Action=Allow|Active=TRUE|Dir=In|Protocol=58|ICMP6=128:*|Name=Echo6|"])
                .admits(&ip6(V6_PEER, V6_ME, 58, &echo6))
        );
        // Neighbour discovery and replies are not filtered.
        assert!(closed.admits(&ip6(V6_PEER, V6_ME, 58, &[135, 0, 0, 0, 0, 0, 0, 0])));
        let unreachable6 = ip6(V6_ME, V6_PEER, 58, &[1, 4, 0, 0, 0, 0, 0, 0]);
        assert!(!closed.sends(&unreachable6));
        assert!(off.sends(&unreachable6));
        // Allow rules by keyword: the gateway is on the local subnet.
        let subnet =
            gate(&["v2.33|Action=Allow|Active=TRUE|Dir=In|Protocol=6|RA4=LocalSubnet|Name=Lan|"]);
        assert!(subnet.admits(&tcp(host, me, 8080, 0x02)));
        assert!(!subnet.admits(&tcp([192, 0, 2, 9], me, 8080, 0x02)));
        // Tiny first fragments, and later fragments at offset 1 (8 bytes,
        // over the TCP flags), are dropped; other later fragments pass.
        let mut tiny = tcp(host, me, 8080, 0x10);
        tiny[20] = 0x20; // more fragments
        tiny.truncate(14 + 20 + 8);
        tiny[16..18].copy_from_slice(&28u16.to_be_bytes());
        assert!(!closed.admits(&tiny));
        let mut later = tcp(host, me, 8080, 0x10);
        later[21] = 1;
        assert!(!closed.admits(&later));
        later[21] = 3;
        assert!(closed.admits(&later));
        assert!(off.admits(&later));
        // Malformed IP headers are dropped.
        let mut short = tcp(host, me, 8080, 0x10);
        short[14] = 0x44;
        assert!(!closed.admits(&short));
    }
    #[test]
    fn stealth_and_rules() {
        let host = [10, 0, 2, 2];
        let me = [10, 0, 2, 15];
        let closed = gate(&[]);
        // No listener: dropped; no rule: dropped; established traffic and
        // replies go on.
        assert!(!closed.admits(&tcp(host, me, 9, 0x02)));
        assert!(!closed.admits(&tcp(host, me, 8080, 0x02)));
        assert!(closed.admits(&tcp(host, me, 8080, 0x10)));
        assert!(closed.admits(&tcp(host, me, 8080, 0x12)));
        assert!(!closed.admits(&tcp([127, 0, 0, 1], me, 9, 0x02)));
        let open =
            gate(&["v2.33|Action=Allow|Active=TRUE|Dir=In|Protocol=6|App=c:\\APP.exe|Name=App|"]);
        assert!(open.admits(&tcp(host, me, 8080, 0x02)));
        let other =
            gate(&["v2.33|Action=Allow|Active=TRUE|Dir=In|Protocol=6|App=C:\\other.exe|Name=O|"]);
        assert!(!other.admits(&tcp(host, me, 8080, 0x02)));
        // Echo requests need a rule; port unreachable never leaves.
        let mut echo = tcp(host, me, 0, 0);
        echo[23] = 1;
        echo[34] = 8;
        assert!(!closed.admits(&echo));
        assert!(
            gate(&["v2.33|Action=Allow|Active=TRUE|Dir=In|Protocol=1|Name=Echo|"]).admits(&echo)
        );
        let mut unreachable = tcp(me, host, 0, 0);
        unreachable[23] = 1;
        unreachable[34] = 3;
        unreachable[35] = 3;
        assert!(!closed.sends(&unreachable));
        unreachable[35] = 1;
        assert!(closed.sends(&unreachable));
        // The firewall off: everything.
        let mut off = gate(&[]);
        let mut policy = (*off.policy).clone();
        policy.profiles[2].enabled = false;
        off.policy = Rc::new(policy);
        assert!(off.admits(&tcp(host, me, 9, 0x02)));
        assert!(off.sends(&unreachable));
        assert_eq!(
            profile_from(state_byte(ProfileState::default())),
            ProfileState::default()
        );
        assert_eq!(state_byte(ProfileState::default()), f::profile::DEFAULT);
    }
    /// Hostile frames: mutations of real IPv4 and IPv6 frames (TCP, UDP,
    /// ICMP, extension headers, fragments, truncations) never panic the
    /// gate in either direction, and a frame from a loopback or this host's
    /// own source address is never admitted, whatever the bytes after it.
    #[test]
    fn hostile_frames_never_panic_or_pass_as_martians() {
        let closed = gate(&[]);
        let open = gate(&["v2.33|Action=Allow|Active=TRUE|Dir=In|Protocol=6|Name=All TCP|"]);
        let mut udp = tcp([10, 0, 2, 2], [10, 0, 2, 15], 53, 0);
        udp[23] = 17;
        let mut icmp = tcp([10, 0, 2, 2], [10, 0, 2, 15], 0, 0);
        icmp[23] = 1;
        icmp[34] = 8;
        let mut fragment = tcp([10, 0, 2, 2], [10, 0, 2, 15], 8080, 0x02);
        fragment[20] = 0x20;
        let mut hop = vec![6u8, 0, 0, 0, 0, 0, 0, 0];
        hop.extend_from_slice(&tcp_header(8080, 0x02));
        let mut fragment6 = vec![6u8, 0, 0, 1, 0, 0, 0, 7];
        fragment6.extend_from_slice(&tcp_header(8080, 0x02));
        let mut later = tcp([10, 0, 2, 2], [10, 0, 2, 15], 8080, 0x10);
        later[21] = 1;
        let seeds: [Vec<u8>; 9] = [
            ip6(V6_PEER, V6_ME, 44, &fragment6),
            later,
            tcp([10, 0, 2, 2], [10, 0, 2, 15], 8080, 0x02),
            udp,
            icmp,
            fragment,
            ip6(V6_PEER, V6_ME, 6, &tcp_header(8080, 0x02)),
            ip6(V6_PEER, V6_ME, 0, &hop),
            ip6(V6_PEER, V6_ME, 58, &[128, 0, 0, 0, 0, 1, 0, 1]),
        ];
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for round in 0..60_000 {
            let mut f = seeds[round % seeds.len()].clone();
            for _ in 0..1 + next() % 6 {
                let r = next();
                match r % 4 {
                    // A byte changed anywhere.
                    0 | 1 if !f.is_empty() => {
                        let at = (r >> 8) as usize % f.len();
                        f[at] = (r >> 32) as u8;
                    }
                    // Cut short.
                    2 => f.truncate((r >> 8) as usize % (f.len() + 1)),
                    // Bytes appended.
                    _ => f.extend_from_slice(&(r >> 8).to_le_bytes()[..(r % 8) as usize]),
                }
            }
            for g in [&closed, &open] {
                let _ = g.sends(&f);
                let admitted = g.admits(&f);
                // IPv4 with a loopback or own source.
                if admitted && f.len() >= 34 && f[12..14] == [0x08, 0x00] && f[14] >> 4 == 4 {
                    let source = [f[26], f[27], f[28], f[29]];
                    assert!(
                        source[0] != 127 && source != [10, 0, 2, 15],
                        "admitted a martian: {f:02x?}"
                    );
                    // A later fragment at offset 8 bytes over a TCP header.
                    let offset = u16::from_be_bytes([f[20] & 0x1f, f[21]]);
                    assert!(
                        !(f[23] == 6 && offset == 1),
                        "admitted an overlapping fragment: {f:02x?}"
                    );
                }
                // IPv6 from or to loopback, from this host, or carrying a
                // Fragment header first (never reassembled).
                if admitted && f.len() >= 54 && f[12..14] == [0x86, 0xdd] && f[14] >> 4 == 6 {
                    let mut loopback = [0u8; 16];
                    loopback[15] = 1;
                    let (source, destination) = (&f[22..38], &f[38..54]);
                    assert!(
                        source != loopback && destination != loopback && source != V6_ME,
                        "admitted an IPv6 martian: {f:02x?}"
                    );
                    assert_ne!(f[20], 44, "admitted an IPv6 fragment: {f:02x?}");
                }
            }
        }
    }
}
