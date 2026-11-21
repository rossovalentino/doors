//! WireGuard VPN connections (P15; `door_service_protocol::net::vpn`).
//!
//! A connection is a wg-quick configuration with one peer, saved per user
//! by Settings. Connecting resolves the server, runs the handshake over a
//! UDP socket on the physical address and, once the first handshake
//! completes, adds the tunnel address to the interface. From then on the
//! NIC (`Divert`) takes every frame whose IPv4 source is the tunnel address
//! into the tunnel instead of onto the wire, answers ARP for the tunnel's
//! on-link peers, and hands decrypted packets back as Ethernet frames: to
//! smoltcp it is one interface with two addresses.
//!
//! New client sockets take the tunnel address as their source when the
//! peer's AllowedIPs route the destination (the longest prefix wins against
//! the on-link network; the server itself always stays outside). The
//! tunnel's DNS servers replace the adapter's while it is up. A
//! configuration routing 0.0.0.0/0 blocks untunneled traffic as WireGuard
//! for Windows' kill-switch does: only DHCP and the tunnel's own datagrams
//! leave on the physical address.
use super::{Network, ipv4, now, udp_socket, word};
use alloc::{
    collections::VecDeque,
    string::{String, ToString},
    vec::Vec,
};
use door_abi::{
    Handle,
    platform::{Identity, manifest},
};
use door_service_protocol::{
    Code,
    net::{
        self,
        vpn::{self as v, Entry, Failure, State},
    },
};
use door_system_services::server::Response;
use door_wireguard::{self as wg, Cidr, Io, Now, Random, Tunnel};
use smoltcp::{
    iface::SocketHandle,
    socket::{dns, udp},
    wire::{DnsQueryType, IpAddress, IpCidr, IpEndpoint, Ipv4Cidr, Ipv6Address, Ipv6Cidr},
};

/// Resolution plus the first handshake must finish within this.
const CONNECT_TIMEOUT: u64 = 15_000;
/// The hardware address the tunnel's on-link peers answer ARP with
/// (locally administered).
const TUNNEL_MAC: [u8; 6] = [0x02, 0x57, 0x47, 0x00, 0x00, 0x01];
const ETHERNET_IPV4: [u8; 2] = [0x08, 0x00];
const ETHERNET_ARP: [u8; 2] = [0x08, 0x06];
const ETHERNET_IPV6: [u8; 2] = [0x86, 0xdd];

/// Frames the NIC diverts into the tunnel and frames it delivers from it.
#[derive(Default)]
pub(crate) struct Divert {
    /// While up: the tunnel's address and prefix.
    tunnel: Option<Cidr>,
    /// The adapter's own network (its on-link hosts are not tunnel peers).
    physical: Option<Cidr>,
    mac: [u8; 6],
    mss: u16,
    /// The kill-switch: the server's address and port, and the tunnel's own
    /// port, are all the physical address may still talk to (besides DHCP).
    block: Option<([u8; 4], u16, u16)>,
    /// IPv4 packets for the tunnel.
    pub(crate) outbound: Vec<Vec<u8>>,
    /// Ethernet frames to deliver as received.
    pub(crate) inbound: VecDeque<Vec<u8>>,
    /// Physical frames the kill-switch dropped.
    blocked: u64,
    /// The tunnel's IPv6 address and prefix, when it has one.
    tunnel6: Option<wg::ip::Cidr6>,
}
pub(crate) enum Verdict {
    Send,
    Taken,
}
impl Divert {
    /// A frame smoltcp is about to transmit.
    pub(crate) fn outgoing(&mut self, frame: &mut [u8]) -> Verdict {
        let Some(tunnel) = self.tunnel else {
            return Verdict::Send;
        };
        if frame.len() < 14 {
            return Verdict::Send;
        }
        let ethertype = [frame[12], frame[13]];
        if ethertype == ETHERNET_IPV6 {
            return self.outgoing6(frame);
        }
        if ethertype == ETHERNET_ARP {
            // A request for an on-link tunnel peer: answer it here.
            let arp = &frame[14..];
            if arp.len() >= 28 && arp[6..8] == [0, 1] {
                let target: [u8; 4] = arp[24..28].try_into().unwrap();
                let on_wire = self.physical.is_some_and(|p| p.contains(target));
                if tunnel.contains(target) && target != tunnel.address && !on_wire {
                    let asker_mac: [u8; 6] = arp[8..14].try_into().unwrap();
                    let asker_ip: [u8; 4] = arp[14..18].try_into().unwrap();
                    self.inbound
                        .push_back(arp_reply(asker_mac, asker_ip, target));
                    return Verdict::Taken;
                }
            }
            return Verdict::Send;
        }
        if ethertype != ETHERNET_IPV4 {
            return Verdict::Send;
        }
        let packet = &mut frame[14..];
        let Some(h) = wg::ip::header(packet) else {
            return Verdict::Send;
        };
        if h.source == tunnel.address {
            let total = h.total;
            wg::ip::clamp_mss(&mut packet[..total], self.mss);
            self.outbound.push(packet[..total].to_vec());
            return Verdict::Taken;
        }
        if let Some((server, server_port, own_port)) = self.block {
            let ports = (h.protocol == 17 && packet.len() >= h.header + 4).then(|| {
                let u = &packet[h.header..];
                (
                    u16::from_be_bytes([u[0], u[1]]),
                    u16::from_be_bytes([u[2], u[3]]),
                )
            });
            let allowed = h.source == [0; 4]
                || ports == Some((68, 67))
                || (h.destination == server && ports == Some((own_port, server_port)));
            if !allowed {
                self.blocked += 1;
                return Verdict::Taken;
            }
        }
        Verdict::Send
    }
    /// An IPv6 frame: neighbour solicitations for the tunnel's on-link
    /// peers are answered here, packets from the tunnel's IPv6 address go
    /// into the tunnel, and the kill-switch keeps everything else but
    /// link-local traffic and neighbour discovery off the wire.
    fn outgoing6(&mut self, frame: &mut [u8]) -> Verdict {
        let asker: [u8; 6] = frame[6..12].try_into().unwrap();
        let packet = &mut frame[14..];
        let Some(h) = wg::ip::header6(packet) else {
            return Verdict::Send;
        };
        let discovery = h.next == 58 && packet.get(40).is_some_and(|t| (133..=137).contains(t));
        if let Some(t6) = self.tunnel6 {
            if discovery && packet[40] == 135 && packet.len() >= 64 {
                let target: [u8; 16] = packet[48..64].try_into().unwrap();
                if t6.contains(target) && target != t6.address {
                    self.inbound
                        .push_back(crate::loopback::neighbor_advertisement(
                            asker, TUNNEL_MAC, target, h.source,
                        ));
                    return Verdict::Taken;
                }
            }
            if h.source == t6.address {
                let total = h.total;
                wg::ip::clamp_mss6(&mut packet[..total], self.mss.saturating_sub(20));
                self.outbound.push(packet[..total].to_vec());
                return Verdict::Taken;
            }
        }
        let link_local = h.source[0] == 0xfe && h.source[1] & 0xc0 == 0x80;
        if self.block.is_some() && !discovery && !link_local && h.source != [0; 16] {
            self.blocked += 1;
            return Verdict::Taken;
        }
        Verdict::Send
    }
    /// A packet out of the tunnel, delivered as an Ethernet frame.
    fn deliver(&mut self, mut packet: Vec<u8>) {
        if packet.len() + 14 > door_abi::platform::NET_FRAME_BYTES || packet.is_empty() {
            return;
        }
        let v6 = packet[0] >> 4 == 6;
        if v6 {
            wg::ip::clamp_mss6(&mut packet, self.mss.saturating_sub(20));
        } else {
            wg::ip::clamp_mss(&mut packet, self.mss);
        }
        let mut frame = Vec::with_capacity(14 + packet.len());
        frame.extend_from_slice(&self.mac);
        frame.extend_from_slice(&TUNNEL_MAC);
        frame.extend_from_slice(if v6 { &ETHERNET_IPV6 } else { &ETHERNET_IPV4 });
        frame.extend_from_slice(&packet);
        self.inbound.push_back(frame);
    }
}
fn arp_reply(asker_mac: [u8; 6], asker_ip: [u8; 4], target: [u8; 4]) -> Vec<u8> {
    let mut f = Vec::with_capacity(42);
    f.extend_from_slice(&asker_mac);
    f.extend_from_slice(&TUNNEL_MAC);
    f.extend_from_slice(&ETHERNET_ARP);
    f.extend_from_slice(&[0, 1, 8, 0, 6, 4, 0, 2]);
    f.extend_from_slice(&TUNNEL_MAC);
    f.extend_from_slice(&target);
    f.extend_from_slice(&asker_mac);
    f.extend_from_slice(&asker_ip);
    f
}

/// Kernel entropy for ephemeral keys and indices (host tests: a fixed
/// generator). A failed read marks the connection failed before anything
/// derived from it leaves.
pub(crate) struct Entropy {
    pool: [u8; door_abi::entropy::READ_MAX],
    used: usize,
    failed: bool,
    #[cfg(test)]
    state: u64,
}
impl Default for Entropy {
    fn default() -> Self {
        Self {
            pool: [0; door_abi::entropy::READ_MAX],
            used: door_abi::entropy::READ_MAX,
            failed: false,
            #[cfg(test)]
            state: 0x9E37_79B9_7F4A_7C15,
        }
    }
}
impl Entropy {
    #[cfg(not(test))]
    fn refill(&mut self) {
        match door_sys::entropy_read(&mut self.pool) {
            Ok(n) if n == self.pool.len() => (),
            _ => {
                self.failed = true;
                self.pool.fill(0);
            }
        }
    }
    #[cfg(test)]
    fn refill(&mut self) {
        for b in &mut self.pool {
            self.state ^= self.state << 13;
            self.state ^= self.state >> 7;
            self.state ^= self.state << 17;
            *b = self.state as u8;
        }
    }
}
impl Random for Entropy {
    fn fill(&mut self, out: &mut [u8]) {
        for b in out {
            if self.used == self.pool.len() {
                self.refill();
                self.used = 0;
            }
            *b = self.pool[self.used];
            self.pool[self.used] = 0;
            self.used += 1;
        }
    }
}
fn clock() -> Now {
    // FILETIME (100 ns since 1601) to nanoseconds since 1970.
    let unix_nanos = door_sys::wall_time().ok().map_or(0, |ft| {
        ft.saturating_sub(116_444_736_000_000_000)
            .saturating_mul(100)
    });
    Now {
        millis: now().total_millis().max(0) as u64,
        unix_nanos,
    }
}

struct Profile {
    owner: u32,
    name: String,
    config: wg::Config,
    public: [u8; 32],
}
#[derive(Clone, Copy)]
enum Phase {
    Resolving(dns::QueryHandle),
    Handshaking,
    Connected,
}
struct Active {
    owner: u32,
    name: String,
    config: wg::Config,
    tunnel: Tunnel,
    socket: SocketHandle,
    port: u16,
    server: Option<([u8; 4], u16)>,
    phase: Phase,
    started: u64,
}
#[derive(Default)]
pub(crate) struct Vpn {
    profiles: Vec<Profile>,
    staging: Vec<(Handle, Vec<u8>)>,
    active: Option<Active>,
    /// The default IPv6 route through the tunnel was added (no router).
    route6: bool,
    /// The last failed attempt: owner, name, why.
    failure: Option<(u32, String, Failure)>,
    entropy: Entropy,
}
impl Vpn {
    /// The UDP port the tunnel holds (no client socket may take it).
    pub(crate) fn port(&self) -> Option<u16> {
        self.active.as_ref().map(|a| a.port)
    }
    pub(crate) fn forget_channel(&mut self, channel: Handle) {
        self.staging.retain(|(c, _)| *c != channel);
    }
    fn find(&self, owner: u32, name: &str) -> Option<usize> {
        self.profiles
            .iter()
            .position(|p| p.owner == owner && p.name.eq_ignore_ascii_case(name))
    }
    fn is_active(&self, owner: u32, name: &str) -> bool {
        self.active
            .as_ref()
            .is_some_and(|a| a.owner == owner && a.name.eq_ignore_ascii_case(name))
    }
}
fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= v::MAX_NAME && !name.chars().any(char::is_control)
}

impl Network {
    /// A SAVE `body` names a connection `owner` already has (the shell's
    /// restore never replaces one).
    fn vpn_holds(&self, owner: u32, body: &[u8]) -> bool {
        core::str::from_utf8(&body[4..]).is_ok_and(|name| self.vpn.find(owner, name).is_some())
    }
    /// door.network's VPN requests (`net::vpn`).
    pub(crate) fn vpn_request(
        &mut self,
        channel: Handle,
        peer: Identity,
        opcode: u32,
        body: &[u8],
    ) -> Response {
        // The tunnel adapter is machine-wide (ipconfig, iphlpapi).
        if opcode == v::TUNNEL {
            if !body.is_empty() {
                return Response::error(Code::InvalidRequest);
            }
            let Some(a) = self.vpn.active.as_ref() else {
                return Response::bytes(&[]);
            };
            let mut t = v::Tunnel::new(&a.name);
            t.connected = matches!(a.phase, Phase::Connected);
            if let Some(c) = a.config.addresses.first() {
                t.address = c.address;
                t.prefix = c.prefix;
            }
            for (slot, d) in t.dns.iter_mut().zip(&a.config.dns) {
                *slot = *d;
            }
            t.mtu = a.config.tunnel_mtu();
            t.rx_bytes = a.tunnel.stats.rx_bytes;
            t.tx_bytes = a.tunnel.stats.tx_bytes;
            return Response::bytes(&t.encode());
        }
        let window = peer.manifest == manifest::WINDOW;
        // The desktop shell hands Settings' saved connections back after a
        // restart (the user's phonebook): it may only add them.
        let shell = peer.manifest == manifest::DESKTOP_SHELL;
        if peer.manifest != manifest::WORKSPACE && !window && !shell {
            return Response::error(Code::Denied);
        }
        let Some(owner) = word(body, 0) else {
            return Response::error(Code::InvalidRequest);
        };
        // Settings acts for its own user only; only Settings edits.
        if (!window && owner != peer.uid)
            || (window && matches!(opcode, v::STAGE | v::SAVE | v::REMOVE))
            || (shell && !matches!(opcode, v::STAGE | v::SAVE | v::ENTRY))
            || (shell && opcode == v::SAVE && self.vpn_holds(owner, body))
        {
            return Response::error(Code::Denied);
        }
        let rest = &body[4..];
        let name = core::str::from_utf8(rest).ok().filter(|n| valid_name(n));
        match opcode {
            v::STAGE => {
                let Some(offset) = word(rest, 0) else {
                    return Response::error(Code::InvalidRequest);
                };
                let data = &rest[4..];
                let at = match self.vpn.staging.iter().position(|(c, _)| *c == channel) {
                    Some(at) => at,
                    None => {
                        self.vpn.staging.push((channel, Vec::new()));
                        self.vpn.staging.len() - 1
                    }
                };
                let staged = &mut self.vpn.staging[at].1;
                if offset == 0 {
                    staged.clear();
                }
                if data.len() > v::PART
                    || offset as usize != staged.len()
                    || staged.len() + data.len() > v::MAX_TEXT
                {
                    return Response::error(Code::InvalidRequest);
                }
                staged.extend_from_slice(data);
                Response::scalar(staged.len() as u64)
            }
            v::SAVE => {
                let Some(name) = name else {
                    return Response::error(Code::InvalidRequest);
                };
                let Some(at) = self.vpn.staging.iter().position(|(c, _)| *c == channel) else {
                    return Response::error(Code::InvalidRequest);
                };
                let (_, text) = self.vpn.staging.swap_remove(at);
                let Ok(config) = core::str::from_utf8(&text)
                    .map_err(|_| ())
                    .and_then(|t| wg::Config::parse(t).map_err(|_| ()))
                else {
                    return Response::error(Code::InvalidRequest);
                };
                // A client needs a server to reach and an address to use.
                if config.peer.endpoint.is_none() || config.addresses.is_empty() {
                    return Response::error(Code::InvalidRequest);
                }
                if self.vpn.is_active(owner, name) {
                    self.vpn_teardown(None);
                }
                let profile = Profile {
                    owner,
                    name: name.trim().to_string(),
                    public: wg::public_key(&config.private_key),
                    config,
                };
                match self.vpn.find(owner, name) {
                    Some(i) => self.vpn.profiles[i] = profile,
                    None => self.vpn.profiles.push(profile),
                }
                door_rt::diagnostic(format_args!("DOOR_P15 VPN SAVED name={}\n", name.trim()));
                Response::empty()
            }
            v::REMOVE => {
                let Some(i) = name.and_then(|n| self.vpn.find(owner, n)) else {
                    return Response::error(Code::NotFound);
                };
                let name = self.vpn.profiles[i].name.clone();
                if self.vpn.is_active(owner, &name) {
                    self.vpn_teardown(None);
                }
                self.vpn.profiles.remove(i);
                self.vpn.failure = self
                    .vpn
                    .failure
                    .take()
                    .filter(|(o, n, _)| !(*o == owner && *n == name));
                Response::empty()
            }
            v::CONNECT => match name.and_then(|n| self.vpn.find(owner, n)) {
                Some(i) => {
                    self.vpn_connect(i);
                    Response::empty()
                }
                None => Response::error(Code::NotFound),
            },
            v::DISCONNECT => {
                let Some(i) = name.and_then(|n| self.vpn.find(owner, n)) else {
                    return Response::error(Code::NotFound);
                };
                let name = self.vpn.profiles[i].name.clone();
                if self.vpn.is_active(owner, &name) {
                    self.vpn_teardown(None);
                }
                self.vpn.failure = self
                    .vpn
                    .failure
                    .take()
                    .filter(|(o, n, _)| !(*o == owner && *n == name));
                Response::empty()
            }
            v::ENTRY => {
                let Some(&index) = rest.first().filter(|_| rest.len() == 1) else {
                    return Response::error(Code::InvalidRequest);
                };
                match self
                    .vpn
                    .profiles
                    .iter()
                    .filter(|p| p.owner == owner)
                    .nth(usize::from(index))
                {
                    Some(p) => Response::bytes(&self.vpn_entry(p).encode()),
                    None => Response::error(Code::NotFound),
                }
            }
            _ => Response::error(Code::InvalidRequest),
        }
    }
    fn vpn_entry(&self, p: &Profile) -> Entry {
        let server = p
            .config
            .peer
            .endpoint
            .as_ref()
            .map(|e| alloc::format!("{e}"))
            .unwrap_or_default();
        let mut e = Entry::new(&p.name, &server);
        e.blocks_untunneled = p.config.blocks_untunneled();
        if let Some(a) = p.config.addresses.first() {
            e.address = a.address;
            e.prefix = a.prefix;
        }
        e.public_key = p.public;
        if let Some(a) = self
            .vpn
            .active
            .as_ref()
            .filter(|a| a.owner == p.owner && a.name == p.name)
        {
            e.state = if matches!(a.phase, Phase::Connected) {
                State::Connected
            } else {
                State::Connecting
            };
            let t = clock().millis;
            e.handshake_age = a.tunnel.stats.last_handshake.map_or(u32::MAX, |at| {
                (t.saturating_sub(at) / 1000).min(u64::from(u32::MAX - 1)) as u32
            });
            e.rx_bytes = a.tunnel.stats.rx_bytes;
            e.tx_bytes = a.tunnel.stats.tx_bytes;
        } else if let Some((_, _, why)) = self
            .vpn
            .failure
            .as_ref()
            .filter(|(o, n, _)| *o == p.owner && *n == p.name)
        {
            e.state = State::Failed;
            e.failure = *why;
        }
        e
    }

    /// Start connection `index` (another one goes down first).
    fn vpn_connect(&mut self, index: usize) {
        self.vpn_teardown(None);
        let p = &self.vpn.profiles[index];
        let (owner, name, config) = (p.owner, p.name.clone(), p.config.clone());
        self.vpn.failure = None;
        door_rt::diagnostic(format_args!("DOOR_P15 VPN CONNECTING name={name}\n"));
        let fail = |n: &mut Self, why| n.vpn.failure = Some((owner, name.clone(), why));
        if self.iface.is_none() || self.address.is_none() {
            fail(self, Failure::NoNetwork);
            door_rt::diagnostic(format_args!("DOOR_P15 VPN FAILED reason=no-network\n"));
            return;
        }
        let port = match self.free_port(net::UDP, config.listen_port.unwrap_or(0)) {
            Ok(p) => p,
            Err(_) => match self.free_port(net::UDP, 0) {
                Ok(p) => p,
                Err(_) => {
                    fail(self, Failure::NoNetwork);
                    return;
                }
            },
        };
        let mut socket = udp_socket();
        if socket.bind(port).is_err() {
            fail(self, Failure::NoNetwork);
            return;
        }
        let socket = self.sockets.add(socket);
        let endpoint = config
            .peer
            .endpoint
            .clone()
            .expect("saved connections have a server");
        let mut tunnel = Tunnel::new(
            config.private_key,
            config.peer.public_key,
            config.peer.preshared_key,
            config.peer.allowed_ips.clone(),
            config.peer.persistent_keepalive,
            config.tunnel_mtu(),
        );
        tunnel.allow6(config.peer.allowed_ips6.clone());
        let started = clock().millis;
        let (phase, server) = match endpoint.address() {
            Some(a) => (Phase::Handshaking, Some((a, endpoint.port))),
            None => {
                let query = match (self.iface.as_mut(), self.dns) {
                    (Some(iface), Some(h)) => self
                        .sockets
                        .get_mut::<dns::Socket>(h)
                        .start_query(iface.context(), &endpoint.host, DnsQueryType::A)
                        .ok(),
                    _ => None,
                };
                let Some(query) = query else {
                    self.sockets.remove(socket);
                    fail(self, Failure::Unresolved);
                    door_rt::diagnostic(format_args!("DOOR_P15 VPN FAILED reason=unresolved\n"));
                    return;
                };
                (Phase::Resolving(query), None)
            }
        };
        self.vpn.active = Some(Active {
            owner,
            name,
            config,
            tunnel,
            socket,
            port,
            server,
            phase,
            started,
        });
        if matches!(phase, Phase::Handshaking) {
            self.vpn_start_handshake();
        }
    }
    fn vpn_start_handshake(&mut self) {
        let mut io = Io::default();
        let Some(a) = self.vpn.active.as_mut() else {
            return;
        };
        a.tunnel.connect(clock(), &mut self.vpn.entropy, &mut io);
        self.vpn_flush(io);
    }
    /// Send what the tunnel produced: datagrams to the server from the
    /// physical address, packets to the interface.
    fn vpn_flush(&mut self, io: Io) {
        if self.vpn.entropy.failed {
            // Never send anything keyed from a failed entropy read.
            self.vpn_teardown(Some(Failure::NoNetwork));
            return;
        }
        let Some(a) = self.vpn.active.as_ref() else {
            return;
        };
        let (socket, server) = (a.socket, a.server);
        let local = self.address.map(|c| IpAddress::Ipv4(c.address()));
        if let Some((address, port)) = server {
            for d in io.network {
                let meta = udp::UdpMetadata {
                    endpoint: IpEndpoint::new(IpAddress::Ipv4(ipv4(address)), port),
                    local_address: local,
                    meta: Default::default(),
                };
                // A full buffer drops the datagram; WireGuard retransmits.
                let _ = self
                    .sockets
                    .get_mut::<udp::Socket>(socket)
                    .send_slice(&d, meta);
            }
        }
        if let Some(nic) = self.nic.as_mut() {
            for p in io.tunnel {
                nic.divert.get_mut().deliver(p);
            }
        }
    }
    /// End the active connection (`why`: it failed).
    pub(crate) fn vpn_teardown(&mut self, why: Option<Failure>) {
        let Some(a) = self.vpn.active.take() else {
            return;
        };
        self.sockets.remove(a.socket);
        self.vpn_uninstall(&a);
        match why {
            Some(f) => {
                door_rt::diagnostic(format_args!(
                    "DOOR_P15 VPN FAILED name={} reason={f:?}\n",
                    a.name
                ));
                self.vpn.failure = Some((a.owner, a.name, f));
            }
            None => {
                door_rt::diagnostic(format_args!("DOOR_P15 VPN DISCONNECTED name={}\n", a.name))
            }
        }
    }
    fn tunnel_cidr(a: &Active) -> Option<Ipv4Cidr> {
        a.config
            .addresses
            .first()
            .map(|c| Ipv4Cidr::new(ipv4(c.address), c.prefix))
    }
    fn tunnel_cidr6(a: &Active) -> Option<Ipv6Cidr> {
        a.config
            .addresses6
            .first()
            .map(|c| Ipv6Cidr::new(Ipv6Address::from(c.address), c.prefix))
    }
    /// A next hop inside the tunnel's IPv6 prefix that is not its own
    /// address: the default route when the adapter has no IPv6 router (the
    /// NIC answers its neighbour solicitation).
    fn tunnel_gateway6(c: &wg::ip::Cidr6) -> [u8; 16] {
        let mut g = c.address;
        let bits = u32::from(c.prefix.min(127));
        let network = u128::from_be_bytes(g) & !(u128::MAX.checked_shr(bits).unwrap_or(0));
        g = (network | 1).to_be_bytes();
        if g == c.address {
            g = (network | 2).to_be_bytes();
        }
        g
    }
    /// Add the tunnel address (first when its DNS servers are in effect, so
    /// smoltcp's queries leave from it) and start diverting.
    fn vpn_install(&mut self) {
        let Some(a) = self.vpn.active.as_ref() else {
            return;
        };
        let Some(cidr) = Self::tunnel_cidr(a) else {
            return;
        };
        let first = !a.config.dns.is_empty();
        let block = a
            .config
            .blocks_untunneled()
            .then(|| a.server.map(|(addr, port)| (addr, port, a.port)))
            .flatten();
        let mss = wg::mss_for(a.config.tunnel_mtu());
        let physical = self
            .address
            .map(|c| Cidr::new(c.address().octets(), c.prefix_len()));
        let cidr6 = Self::tunnel_cidr6(a);
        let tunnel6 = a.config.addresses6.first().copied();
        // Without an IPv6 router on the adapter, IPv6 leaves by the tunnel.
        let route6 = tunnel6
            .filter(|_| self.slaac.as_ref().is_none_or(|s| s.router.is_none()))
            .map(|t| Self::tunnel_gateway6(&t));
        if let Some(iface) = self.iface.as_mut() {
            iface.update_ip_addrs(|addrs| {
                addrs.retain(|c| *c != IpCidr::Ipv4(cidr));
                let _ = if first {
                    addrs.insert(0, IpCidr::Ipv4(cidr))
                } else {
                    addrs.push(IpCidr::Ipv4(cidr))
                };
                if let Some(c6) = cidr6 {
                    addrs.retain(|c| *c != IpCidr::Ipv6(c6));
                    let _ = addrs.push(IpCidr::Ipv6(c6));
                }
            });
            if let Some(gateway) = route6 {
                let _ = iface
                    .routes_mut()
                    .add_default_ipv6_route(Ipv6Address::from(gateway));
            }
            crate::loopback_last(iface);
        }
        self.vpn.route6 = route6.is_some();
        if let Some(nic) = self.nic.as_mut() {
            let d = nic.divert.get_mut();
            d.tunnel = Some(Cidr::new(cidr.address().octets(), cidr.prefix_len()));
            d.tunnel6 = tunnel6;
            d.physical = physical;
            d.mac = self.mac;
            d.mss = mss;
            d.block = block;
        }
        self.refresh_dns();
    }
    fn vpn_uninstall(&mut self, a: &Active) {
        if let (Some(iface), Some(cidr)) = (self.iface.as_mut(), Self::tunnel_cidr(a)) {
            let cidr6 = Self::tunnel_cidr6(a);
            iface.update_ip_addrs(|addrs| {
                addrs.retain(|c| *c != IpCidr::Ipv4(cidr) && Some(*c) != cidr6.map(IpCidr::Ipv6))
            });
            if core::mem::take(&mut self.vpn.route6) {
                iface.routes_mut().remove_default_ipv6_route();
            }
        }
        if let Some(nic) = self.nic.as_mut() {
            let blocked = nic.divert.get_mut().blocked;
            if blocked != 0 {
                door_rt::diagnostic(format_args!("DOOR_P15 VPN KILLSWITCH blocked={blocked}\n"));
            }
            *nic.divert.get_mut() = Divert::default();
        }
        self.refresh_dns();
    }
    /// The tunnel's DNS servers while it is up and has any.
    pub(crate) fn vpn_dns(&self) -> Option<Vec<IpAddress>> {
        let a = self
            .vpn
            .active
            .as_ref()
            .filter(|a| matches!(a.phase, Phase::Connected))?;
        (!a.config.dns.is_empty() || !a.config.dns6.is_empty()).then(|| {
            a.config
                .dns
                .iter()
                .map(|d| IpAddress::Ipv4(ipv4(*d)))
                .chain(
                    a.config
                        .dns6
                        .iter()
                        .map(|d| IpAddress::Ipv6(Ipv6Address::from(*d))),
                )
                .collect()
        })
    }
    /// The IPv6 form of `vpn_source`: the tunnel's IPv6 address when the
    /// peer's IPv6 AllowedIPs route `destination` (link-local and this
    /// host's own addresses never do); `None` otherwise, smoltcp choosing.
    pub(crate) fn vpn_source6(&self, destination: [u8; 16]) -> Option<IpAddress> {
        let a = self
            .vpn
            .active
            .as_ref()
            .filter(|a| matches!(a.phase, Phase::Connected))?;
        let tunnel = a.config.addresses6.first()?;
        let link_local = destination[0] == 0xfe && destination[1] & 0xc0 == 0x80;
        if link_local
            || destination[0] == 0xff
            || destination == crate::loopback::V6
            || wg::ip::longest6(&a.config.peer.allowed_ips6, destination).is_none()
        {
            return None;
        }
        Some(IpAddress::Ipv6(Ipv6Address::from(tunnel.address)))
    }
    /// The source address a new socket to `destination` takes while a VPN
    /// is up: the tunnel's when the peer's AllowedIPs route it (the longest
    /// prefix wins against the on-link network unless the kill-switch is
    /// on), else the adapter's, always explicitly, since smoltcp would
    /// otherwise pick the first address, which may be the tunnel's. Apps'
    /// traffic to the server's own address is routed like any other, as
    /// WireGuard does; only the tunnel's socket bypasses it. `None` while
    /// no VPN is up.
    pub(crate) fn vpn_source(&self, destination: [u8; 4]) -> Option<IpAddress> {
        let a = self
            .vpn
            .active
            .as_ref()
            .filter(|a| matches!(a.phase, Phase::Connected))?;
        let tunnel = a.config.addresses.first()?;
        let physical = self.address.map(|c| IpAddress::Ipv4(c.address()));
        let Some(routed) = wg::ip::longest(&a.config.peer.allowed_ips, destination) else {
            return physical;
        };
        if !a.config.blocks_untunneled()
            && self.address.is_some_and(|c| {
                Cidr::new(c.address().octets(), c.prefix_len()).contains(destination)
                    && c.prefix_len() > routed
            })
        {
            return physical;
        }
        Some(IpAddress::Ipv4(ipv4(tunnel.address)))
    }

    /// Where a client datagram to `address:port` goes and leaves from.
    pub(crate) fn udp_to(&self, address: IpAddress, port: u16) -> udp::UdpMetadata {
        udp::UdpMetadata {
            endpoint: IpEndpoint::new(address, port),
            local_address: match address {
                IpAddress::Ipv4(v4) => self.vpn_source(v4.octets()),
                IpAddress::Ipv6(v6) => self.vpn_source6(v6.octets()),
            },
            meta: Default::default(),
        }
    }

    /// One VPN step after the stack's: the connection's phase, datagrams
    /// from the server, packets the NIC diverted, timers. True when
    /// anything happened.
    pub(crate) fn vpn_poll(&mut self) -> bool {
        let Some((phase, established, cidr)) = self.vpn.active.as_ref().map(|a| {
            (
                a.phase,
                a.tunnel.established(clock().millis),
                Self::tunnel_cidr(a),
            )
        }) else {
            return false;
        };
        if self.iface.is_none() || self.address.is_none() {
            self.vpn_teardown(Some(Failure::NoNetwork));
            return true;
        }
        let t = clock();
        let mut changed = false;
        match phase {
            Phase::Resolving(query) => {
                let result = self.dns.map(|h| {
                    self.sockets
                        .get_mut::<dns::Socket>(h)
                        .get_query_result(query)
                        .map(|addresses| {
                            addresses.iter().find_map(|a| match a {
                                IpAddress::Ipv4(v4) => Some(*v4),
                                IpAddress::Ipv6(_) => None,
                            })
                        })
                });
                match result {
                    Some(Ok(Some(first))) => {
                        let a = self.vpn.active.as_mut().unwrap();
                        let port = a.config.peer.endpoint.as_ref().map_or(51820, |e| e.port);
                        a.server = Some((first.octets(), port));
                        a.phase = Phase::Handshaking;
                        self.vpn_start_handshake();
                        changed = true;
                    }
                    Some(Err(dns::GetQueryResultError::Pending)) => (),
                    _ => {
                        self.vpn_teardown(Some(Failure::Unresolved));
                        return true;
                    }
                }
            }
            Phase::Handshaking if established => {
                let a = self.vpn.active.as_mut().unwrap();
                a.phase = Phase::Connected;
                let ms = t.millis.saturating_sub(a.started);
                let (name, address) = (a.name.clone(), a.config.addresses[0]);
                self.vpn_install();
                door_rt::diagnostic(format_args!(
                    "DOOR_P15 VPN CONNECTED name={name} address={address} handshake_ms={ms}\n"
                ));
                changed = true;
            }
            Phase::Connected => {
                // A DHCP renewal or reconfiguration rebuilt the address list.
                let present = self
                    .iface
                    .as_ref()
                    .is_some_and(|i| cidr.is_some_and(|c| i.ip_addrs().contains(&IpCidr::Ipv4(c))));
                if !present {
                    self.vpn_install();
                }
            }
            Phase::Handshaking => (),
        }
        let late = self.vpn.active.as_ref().and_then(|a| match a.phase {
            _ if t.millis.saturating_sub(a.started) < CONNECT_TIMEOUT => None,
            Phase::Resolving(_) => Some(Failure::Unresolved),
            Phase::Handshaking => Some(Failure::NoAnswer),
            Phase::Connected => None,
        });
        if let Some(why) = late {
            self.vpn_teardown(Some(why));
            return true;
        }
        let Some(a) = self.vpn.active.as_mut() else {
            return changed;
        };
        let mut io = Io::default();
        // Datagrams from the server (anything else on the port is dropped).
        let mut buffer = [0u8; 2048];
        while let Ok((n, meta)) = self
            .sockets
            .get_mut::<udp::Socket>(a.socket)
            .recv_slice(&mut buffer)
        {
            let IpAddress::Ipv4(from) = meta.endpoint.addr else {
                continue;
            };
            if a.server == Some((from.octets(), meta.endpoint.port)) {
                let _ = a
                    .tunnel
                    .receive(&buffer[..n], t, &mut self.vpn.entropy, &mut io);
                changed = true;
            }
        }
        // Packets the NIC diverted into the tunnel.
        if let Some(nic) = self.nic.as_mut() {
            let outbound = core::mem::take(&mut nic.divert.get_mut().outbound);
            if matches!(a.phase, Phase::Connected) {
                // Cryptokey routing: only destinations the peer's
                // AllowedIPs hold go to it; anything else is dropped.
                for p in &outbound {
                    let routed = match p.first().map(|b| b >> 4) {
                        Some(6) => wg::ip::header6(p).is_some_and(|h| {
                            a.config
                                .peer
                                .allowed_ips6
                                .iter()
                                .any(|c| c.contains(h.destination))
                        }),
                        _ => wg::ip::header(p).is_some_and(|h| {
                            a.config
                                .peer
                                .allowed_ips
                                .iter()
                                .any(|c| c.contains(h.destination))
                        }),
                    };
                    if routed {
                        a.tunnel.send(p, t, &mut self.vpn.entropy, &mut io);
                    }
                }
            }
            changed |= !outbound.is_empty();
        }
        a.tunnel.tick(t, &mut self.vpn.entropy, &mut io);
        changed |= !io.network.is_empty() || !io.tunnel.is_empty();
        self.vpn_flush(io);
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn divert() -> Divert {
        Divert {
            tunnel: Some(Cidr::new([10, 8, 0, 2], 24)),
            physical: Some(Cidr::new([10, 0, 2, 15], 24)),
            mac: [0x52, 0x54, 0, 0x12, 0x34, 0x56],
            mss: 1380,
            ..Divert::default()
        }
    }
    fn frame(src: [u8; 4], dst: [u8; 4], protocol: u8, ports: (u16, u16)) -> Vec<u8> {
        let mut f = alloc::vec![0u8; 14 + 28];
        f[12..14].copy_from_slice(&ETHERNET_IPV4);
        let p = &mut f[14..];
        p[0] = 0x45;
        p[2..4].copy_from_slice(&28u16.to_be_bytes());
        p[9] = protocol;
        p[12..16].copy_from_slice(&src);
        p[16..20].copy_from_slice(&dst);
        p[20..22].copy_from_slice(&ports.0.to_be_bytes());
        p[22..24].copy_from_slice(&ports.1.to_be_bytes());
        f
    }

    fn frame6(src: [u8; 16], dst: [u8; 16], next: u8, payload: &[u8]) -> Vec<u8> {
        let mut f = alloc::vec![0u8; 14 + 40 + payload.len()];
        f[6..12].copy_from_slice(&[0x52, 0x54, 0, 0x12, 0x34, 0x56]);
        f[12..14].copy_from_slice(&ETHERNET_IPV6);
        let p = &mut f[14..];
        p[0] = 0x60;
        p[4..6].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        p[6] = next;
        p[7] = 255;
        p[8..24].copy_from_slice(&src);
        p[24..40].copy_from_slice(&dst);
        p[40..].copy_from_slice(payload);
        f
    }

    #[test]
    fn ipv6_through_the_tunnel() {
        let ours = wg::ip::Cidr6::parse("fd00::2/64").unwrap();
        let mut d = divert();
        d.tunnel6 = Some(ours);
        let gateway = Network::tunnel_gateway6(&ours);
        assert_eq!(gateway, wg::ip::Cidr6::parse("fd00::1").unwrap().address);
        // A neighbour solicitation for the gateway: advertised from the
        // tunnel's hardware address.
        let mut ns = alloc::vec![0u8; 24];
        ns[0] = 135;
        ns[8..24].copy_from_slice(&gateway);
        let mut f = frame6(ours.address, [0xff; 16], 58, &ns);
        assert!(matches!(d.outgoing(&mut f), Verdict::Taken));
        let na = d.inbound.pop_front().unwrap();
        assert_eq!(&na[6..12], &TUNNEL_MAC);
        assert_eq!(na[14 + 40], 136);
        assert_eq!(&na[14 + 48..14 + 64], &gateway);
        // A packet from the tunnel's IPv6 address goes in; one from
        // elsewhere passes (no kill-switch).
        let remote = wg::ip::Cidr6::parse("2001:db8::1").unwrap().address;
        let mut p = frame6(ours.address, remote, 17, &[0; 8]);
        assert!(matches!(d.outgoing(&mut p), Verdict::Taken));
        assert_eq!(d.outbound, [p[14..].to_vec()]);
        let global = wg::ip::Cidr6::parse("fec0::5054:ff:fe12:3456")
            .unwrap()
            .address;
        let mut q = frame6(global, remote, 17, &[0; 8]);
        assert!(matches!(d.outgoing(&mut q), Verdict::Send));
        // The kill-switch keeps it off the wire, but not link-local
        // neighbour discovery.
        d.block = Some(([10, 0, 2, 2], 51820, 50000));
        assert!(matches!(d.outgoing(&mut q), Verdict::Taken));
        let link = wg::ip::Cidr6::parse("fe80::1").unwrap().address;
        let mut r = frame6(link, [0xff; 16], 58, &[133, 0, 0, 0, 0, 0, 0, 0]);
        assert!(matches!(d.outgoing(&mut r), Verdict::Send));
        // From the tunnel: an IPv6 Ethernet frame.
        d.deliver(p[14..].to_vec());
        let back = d.inbound.pop_front().unwrap();
        assert_eq!(&back[12..14], &ETHERNET_IPV6);
        assert_eq!(&back[14..], &p[14..]);
    }

    #[test]
    fn tunnel_sourced_frames_are_taken_and_others_pass() {
        let mut d = divert();
        let mut f = frame([10, 8, 0, 2], [1, 1, 1, 1], 17, (5000, 53));
        assert!(matches!(d.outgoing(&mut f), Verdict::Taken));
        assert_eq!(d.outbound, [f[14..].to_vec()]);
        let mut g = frame([10, 0, 2, 15], [1, 1, 1, 1], 17, (5000, 53));
        assert!(matches!(d.outgoing(&mut g), Verdict::Send));
        // Nothing is diverted while the tunnel is down.
        let mut idle = Divert::default();
        assert!(matches!(idle.outgoing(&mut f), Verdict::Send));
    }

    #[test]
    fn the_kill_switch_lets_only_dhcp_and_the_tunnel_out() {
        let mut d = divert();
        d.block = Some(([10, 0, 2, 2], 51820, 49200));
        let mut cases = [
            (
                frame([10, 0, 2, 15], [10, 0, 2, 2], 17, (49200, 51820)),
                true,
            ),
            (
                frame([10, 0, 2, 15], [10, 0, 2, 2], 17, (49201, 51820)),
                false,
            ),
            (
                frame([10, 0, 2, 15], [10, 0, 2, 2], 6, (49200, 51820)),
                false,
            ),
            (
                frame([0, 0, 0, 0], [255, 255, 255, 255], 17, (68, 67)),
                true,
            ),
            (frame([10, 0, 2, 15], [10, 0, 2, 2], 17, (68, 67)), true),
            (frame([10, 0, 2, 15], [10, 0, 2, 3], 17, (5000, 53)), false),
        ];
        for (f, passes) in &mut cases {
            assert_eq!(matches!(d.outgoing(f), Verdict::Send), *passes);
        }
        assert_eq!(d.blocked, 3);
    }

    const SETTINGS: Identity = Identity {
        pid: 9,
        uid: 1000,
        integrity: 1,
        groups: 0,
        session: 3,
        epoch: 1,
        manifest: manifest::WORKSPACE,
        instance: 1,
        capabilities: 0,
    };
    const TEXT: &str = "[Interface]\nPrivateKey = yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=\nAddress = 10.8.0.2/24\nDNS = 10.8.0.1\n[Peer]\nPublicKey = xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=\nAllowedIPs = 0.0.0.0/0\nEndpoint = 10.0.2.2:51820\n";
    fn call(n: &mut Network, peer: Identity, opcode: u32, uid: u32, rest: &[u8]) -> Response {
        let (body, len) = v::body(uid, rest);
        n.vpn_request(0x10, peer, opcode, &body[..len])
    }
    fn stage(n: &mut Network, text: &str) {
        for (i, part) in text.as_bytes().chunks(v::PART).enumerate() {
            let mut rest = ((i * v::PART) as u32).to_le_bytes().to_vec();
            rest.extend_from_slice(part);
            assert_eq!(call(n, SETTINGS, v::STAGE, 1000, &rest).code, Code::Success);
        }
    }
    fn entry(n: &mut Network, peer: Identity, uid: u32, index: u8) -> Option<Entry> {
        let r = call(n, peer, v::ENTRY, uid, &[index]);
        (r.code == Code::Success).then(|| Entry::decode(&r.body[..r.len]).unwrap())
    }

    #[test]
    fn settings_saves_and_lists_its_users_connections() {
        let mut n = Network::new(None, [2, 0, 0, 0, 0, 1]);
        stage(&mut n, TEXT);
        assert_eq!(
            call(&mut n, SETTINGS, v::SAVE, 1000, b"Office").code,
            Code::Success
        );
        let e = entry(&mut n, SETTINGS, 1000, 0).unwrap();
        assert_eq!(
            (e.name(), e.server(), e.state),
            ("Office", "10.0.2.2:51820", State::Disconnected)
        );
        assert!(e.blocks_untunneled);
        assert_eq!((e.address, e.prefix), ([10, 8, 0, 2], 24));
        assert_eq!(
            e.public_key,
            wg::public_key(
                &wg::config::decode_key("yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=").unwrap()
            )
        );
        assert!(entry(&mut n, SETTINGS, 1000, 1).is_none());
        // Another user sees nothing; Settings cannot speak for them.
        assert_eq!(
            call(&mut n, SETTINGS, v::ENTRY, 1001, &[0]).code,
            Code::Denied
        );
        // The window server reads the user it names, but never edits.
        let window = Identity {
            manifest: manifest::WINDOW,
            uid: 1,
            session: 0,
            epoch: 0,
            ..SETTINGS
        };
        assert_eq!(entry(&mut n, window, 1000, 0).unwrap().name(), "Office");
        assert!(entry(&mut n, window, 1001, 0).is_none());
        assert_eq!(
            call(&mut n, window, v::REMOVE, 1000, b"Office").code,
            Code::Denied
        );
        // An ordinary app is refused outright.
        let app = Identity {
            manifest: manifest::WINDOWS_GUI,
            ..SETTINGS
        };
        assert_eq!(call(&mut n, app, v::ENTRY, 1000, &[0]).code, Code::Denied);
        // Saving the same name (any case) replaces it; remove deletes it.
        stage(&mut n, &TEXT.replace("0.0.0.0/0", "10.8.0.0/24"));
        assert_eq!(
            call(&mut n, SETTINGS, v::SAVE, 1000, b"office").code,
            Code::Success
        );
        assert!(!entry(&mut n, SETTINGS, 1000, 0).unwrap().blocks_untunneled);
        assert!(entry(&mut n, SETTINGS, 1000, 1).is_none());
        assert_eq!(
            call(&mut n, SETTINGS, v::REMOVE, 1000, b"OFFICE").code,
            Code::Success
        );
        assert!(entry(&mut n, SETTINGS, 1000, 0).is_none());
    }

    #[test]
    fn invalid_texts_names_and_stages_are_refused() {
        let mut n = Network::new(None, [2, 0, 0, 0, 0, 1]);
        // SAVE needs something staged first.
        assert_eq!(
            call(&mut n, SETTINGS, v::SAVE, 1000, b"A").code,
            Code::InvalidRequest
        );
        stage(&mut n, "[Interface]\nPrivateKey = nope\n");
        assert_eq!(
            call(&mut n, SETTINGS, v::SAVE, 1000, b"A").code,
            Code::InvalidRequest
        );
        // No endpoint: a client has nowhere to go.
        stage(&mut n, &TEXT.replace("Endpoint = 10.0.2.2:51820\n", ""));
        assert_eq!(
            call(&mut n, SETTINGS, v::SAVE, 1000, b"A").code,
            Code::InvalidRequest
        );
        stage(&mut n, TEXT);
        for bad in [&b""[..], b"   ", b"a\nb", &[b'x'; 65]] {
            assert_eq!(
                call(&mut n, SETTINGS, v::SAVE, 1000, bad).code,
                Code::InvalidRequest
            );
        }
        // Parts continue where the staging ends, within MAX_TEXT.
        let mut rest = 5u32.to_le_bytes().to_vec();
        rest.extend_from_slice(b"x");
        assert_eq!(
            call(&mut n, SETTINGS, v::STAGE, 1000, &rest).code,
            Code::InvalidRequest
        );
        let mut at = 0;
        while at + v::PART <= v::MAX_TEXT {
            let mut rest = (at as u32).to_le_bytes().to_vec();
            rest.extend_from_slice(&[b'#'; v::PART]);
            assert_eq!(
                call(&mut n, SETTINGS, v::STAGE, 1000, &rest).code,
                Code::Success
            );
            at += v::PART;
        }
        let mut rest = (at as u32).to_le_bytes().to_vec();
        rest.extend_from_slice(&[b'#'; v::PART]);
        assert_eq!(
            call(&mut n, SETTINGS, v::STAGE, 1000, &rest).code,
            Code::InvalidRequest
        );
    }

    #[test]
    fn connecting_without_an_adapter_fails_truthfully() {
        let mut n = Network::new(None, [2, 0, 0, 0, 0, 1]);
        stage(&mut n, TEXT);
        call(&mut n, SETTINGS, v::SAVE, 1000, b"Office");
        assert_eq!(
            call(&mut n, SETTINGS, v::CONNECT, 1000, b"Office").code,
            Code::Success
        );
        let e = entry(&mut n, SETTINGS, 1000, 0).unwrap();
        assert_eq!((e.state, e.failure), (State::Failed, Failure::NoNetwork));
        assert_eq!(
            call(&mut n, SETTINGS, v::CONNECT, 1000, b"Home").code,
            Code::NotFound
        );
        // Disconnect clears the failure.
        assert_eq!(
            call(&mut n, SETTINGS, v::DISCONNECT, 1000, b"Office").code,
            Code::Success
        );
        assert_eq!(
            entry(&mut n, SETTINGS, 1000, 0).unwrap().state,
            State::Disconnected
        );
        assert!(n.vpn_source([1, 1, 1, 1]).is_none());
    }

    #[test]
    fn sockets_take_the_tunnel_address_only_where_the_peer_routes() {
        let mut n = Network::new(None, [2, 0, 0, 0, 0, 1]);
        n.address = Some(Ipv4Cidr::new(ipv4([10, 0, 2, 15]), 24));
        let physical = Some(IpAddress::Ipv4(ipv4([10, 0, 2, 15])));
        let tunnel = Some(IpAddress::Ipv4(ipv4([10, 8, 0, 2])));
        let up = |n: &mut Network, allowed: &str| {
            let config = wg::Config::parse(&TEXT.replace("0.0.0.0/0", allowed)).unwrap();
            let socket = n.sockets.add(udp_socket());
            let t = Tunnel::new(
                config.private_key,
                config.peer.public_key,
                None,
                config.peer.allowed_ips.clone(),
                None,
                1420,
            );
            n.vpn.active = Some(Active {
                owner: 1000,
                name: "Office".into(),
                config,
                tunnel: t,
                socket,
                port: 49200,
                server: Some(([10, 0, 2, 2], 51820)),
                phase: Phase::Connected,
                started: 0,
            });
        };
        // No VPN: smoltcp chooses.
        assert_eq!(n.vpn_source([1, 1, 1, 1]), None);
        // Split tunnel: only its prefix, and the adapter's address (never
        // the implicit first one) for the rest.
        up(&mut n, "10.8.0.0/24, 192.168.50.0/24");
        assert_eq!(n.vpn_source([10, 8, 0, 1]), tunnel);
        assert_eq!(n.vpn_source([192, 168, 50, 9]), tunnel);
        assert_eq!(n.vpn_source([1, 1, 1, 1]), physical);
        // Everything, kill-switch on: even the LAN and the server's host.
        up(&mut n, "0.0.0.0/0");
        assert_eq!(n.vpn_source([10, 0, 2, 3]), tunnel);
        assert_eq!(n.vpn_source([10, 0, 2, 2]), tunnel);
        // Everything as two /1 routes (kill-switch off): the on-link /24 wins.
        up(&mut n, "0.0.0.0/1, 128.0.0.0/1");
        assert_eq!(n.vpn_source([10, 0, 2, 3]), physical);
        assert_eq!(n.vpn_source([8, 8, 8, 8]), tunnel);
        // While connecting nothing changes yet.
        n.vpn.active.as_mut().unwrap().phase = Phase::Handshaking;
        assert_eq!(n.vpn_source([8, 8, 8, 8]), None);
    }

    #[test]
    fn arp_for_tunnel_peers_is_answered_locally() {
        let mut d = divert();
        let mut request = alloc::vec![0u8; 42];
        request[..6].fill(0xFF);
        request[6..12].copy_from_slice(&d.mac);
        request[12..14].copy_from_slice(&ETHERNET_ARP);
        request[14..22].copy_from_slice(&[0, 1, 8, 0, 6, 4, 0, 1]);
        request[22..28].copy_from_slice(&d.mac);
        request[28..32].copy_from_slice(&[10, 0, 2, 15]);
        request[38..42].copy_from_slice(&[10, 8, 0, 1]);
        assert!(matches!(d.outgoing(&mut request), Verdict::Taken));
        let reply = d.inbound.pop_front().unwrap();
        assert_eq!(&reply[..6], &d.mac);
        assert_eq!(&reply[20..22], &[0, 2]);
        assert_eq!(&reply[22..28], &TUNNEL_MAC);
        assert_eq!(&reply[28..32], &[10, 8, 0, 1]);
        assert_eq!(&reply[38..42], &[10, 0, 2, 15]);
        // The gateway is on the wire: its ARP goes out.
        request[38..42].copy_from_slice(&[10, 0, 2, 2]);
        assert!(matches!(d.outgoing(&mut request), Verdict::Send));
    }
}
