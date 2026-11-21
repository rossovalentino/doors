//! door-network (P15): the TCP/IP stack over the NIC. It is the only process
//! holding the NETWORK role authority (frames in and out through the
//! kernel's polled VirtIO driver); smoltcp supplies Ethernet/ARP, IPv4,
//! ICMP, UDP, TCP, the DHCP client and DNS queries. Clients reach it over
//! the door.network protocol (`door_service_protocol::net`): status and
//! configuration (Settings), name resolution and sockets with a per-client
//! quota; a client's sockets close when its channel goes away. WireGuard
//! VPN connections run inside the same stack (`vpn`).
#![no_std]
extern crate alloc;
mod firewall;
mod icmp;
mod ipv6;
mod loopback;
mod stats;
mod vpn;
use alloc::{collections::VecDeque, vec, vec::Vec};
use core::cell::{Cell, RefCell};
use door_abi::{
    Handle, Status as SysStatus, Transfer,
    platform::{Identity, StartupV2, inherited, manifest},
};
use door_service_protocol::{
    Code,
    net::{self, Config, Dhcp, SocketState, op},
};
use door_system_services::server::{Handler, Response, Server};
use smoltcp::{
    iface::{Config as IfaceConfig, Interface, SocketHandle, SocketSet},
    phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken},
    socket::{dhcpv4, dns, raw, tcp, udp},
    time::Instant,
    wire::{
        DnsQueryType, EthernetAddress, HardwareAddress, IpAddress, IpCidr, IpEndpoint, IpProtocol,
        IpVersion, Ipv4Address, Ipv4Cidr, Ipv6Address,
    },
};

/// Heap for socket buffers and the stack's tables (the manifest allows 16 MiB).
pub const HEAP_BYTES: u64 = 12 * 1024 * 1024;
const TCP_BUFFER: usize = 32 * 1024;
const UDP_PACKETS: usize = 16;
const UDP_BUFFER: usize = 16 * 1024;
const DNS_QUERIES: usize = 8;
/// Groups one socket may join (Windows' IP_MAX_MEMBERSHIPS).
const MAX_MEMBERSHIPS: usize = 20;
/// Loopback frames waiting for the stack, and stack steps one poll takes
/// while they keep coming.
const LOOPED_FRAMES: usize = 64;
const LOOPBACK_ROUNDS: usize = 16;

struct Nic {
    /// The adapter (none: the loopback interface alone).
    handle: Option<Handle>,
    /// The adapter is on: frames go to and come from the wire.
    wire: Cell<bool>,
    mac: [u8; 6],
    rx: [u8; door_abi::platform::NET_FRAME_BYTES],
    frames_in: Cell<u64>,
    frames_out: Cell<u64>,
    bytes_in: Cell<u64>,
    bytes_out: Cell<u64>,
    /// VPN: frames taken into the tunnel and frames delivered from it.
    divert: RefCell<vpn::Divert>,
    /// Protocol counters and the ARP neighbour table, from the frames.
    stats: RefCell<stats::Stats>,
    /// Frames to this host itself (`loopback`), back to the stack.
    looped: RefCell<VecDeque<Vec<u8>>>,
    /// The firewall's checks on frames in and out.
    gate: RefCell<firewall::Gate>,
}
struct Rx<'a>(&'a [u8]);
struct Tx<'a> {
    nic: &'a Nic,
}
impl RxToken for Rx<'_> {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        f(self.0)
    }
}
impl TxToken for Tx<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut frame = [0u8; door_abi::platform::NET_FRAME_BYTES];
        let len = len.min(frame.len());
        let result = f(&mut frame[..len]);
        // This host's own addresses: back to the stack, never the wire
        // (nor the counters and neighbour table, as on Windows).
        if let Some(back) = loopback::route(&frame[..len], self.nic.mac) {
            let mut looped = self.nic.looped.borrow_mut();
            if looped.len() < LOOPED_FRAMES {
                looped.push_back(back);
            }
            return result;
        }
        let Some(handle) = self.nic.handle.filter(|_| self.nic.wire.get()) else {
            return result;
        };
        if !self.nic.gate.borrow().sends(&frame[..len]) {
            return result;
        }
        self.nic
            .stats
            .borrow_mut()
            .observe(&frame[..len], true, now().total_millis() as u64);
        if matches!(
            self.nic.divert.borrow_mut().outgoing(&mut frame[..len]),
            vpn::Verdict::Taken
        ) {
            return result;
        }
        // A full transmit ring drops the frame; TCP retransmits it.
        if door_sys::net_send(handle, &frame[..len]).is_ok() {
            self.nic.frames_out.set(self.nic.frames_out.get() + 1);
            self.nic
                .bytes_out
                .set(self.nic.bytes_out.get() + len as u64);
        }
        result
    }
}
impl Device for Nic {
    type RxToken<'a> = Rx<'a>;
    type TxToken<'a> = Tx<'a>;
    fn receive(&mut self, _: Instant) -> Option<(Rx<'_>, Tx<'_>)> {
        // What the stack sent itself comes first.
        if let Some(frame) = self.looped.get_mut().pop_front() {
            let n = frame.len().min(self.rx.len());
            self.rx[..n].copy_from_slice(&frame[..n]);
            let nic: &Nic = self;
            return Some((Rx(&nic.rx[..n]), Tx { nic }));
        }
        let handle = self.handle.filter(|_| self.wire.get())?;
        // Packets out of the VPN tunnel come next (not NIC frames); what
        // the firewall refuses is dropped here, before the stack answers.
        loop {
            let n = if let Some(frame) = self.divert.get_mut().inbound.pop_front() {
                let n = frame.len().min(self.rx.len());
                self.rx[..n].copy_from_slice(&frame[..n]);
                n
            } else {
                let n = door_sys::net_receive(handle, &mut self.rx).ok()?;
                if n == 0 {
                    return None;
                }
                self.frames_in.set(self.frames_in.get() + 1);
                self.bytes_in.set(self.bytes_in.get() + n as u64);
                n
            };
            self.stats
                .get_mut()
                .observe(&self.rx[..n], false, now().total_millis() as u64);
            if self.gate.get_mut().admits(&self.rx[..n]) {
                let nic: &Nic = self;
                return Some((Rx(&nic.rx[..n]), Tx { nic }));
            }
        }
    }
    fn transmit(&mut self, _: Instant) -> Option<Tx<'_>> {
        Some(Tx { nic: self })
    }
    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = door_abi::platform::NET_FRAME_BYTES;
        caps
    }
}

struct ClientSocket {
    channel: Handle,
    /// The owning process (netstat -o, GetExtendedTcpTable).
    pid: u64,
    id: u32,
    kind: u8,
    handle: SocketHandle,
    /// UDP peer set by CONNECT.
    peer: Option<(IpAddress, u16)>,
    /// Local port from BIND or the first send/connect (0: none yet).
    local: u16,
    /// A TCP listener's sockets on its port: smoltcp hands a listening
    /// socket the connection itself, so the listener keeps a backlog of
    /// them and ACCEPT gives an established one a socket id of its own.
    pool: Vec<SocketHandle>,
    /// UDP: the datagram RECEIVE_FROM took, for RECEIVE_MORE.
    inbound: Vec<u8>,
    /// UDP: parts SEND_PART staged for the next SEND_TO.
    outbound: Vec<u8>,
    /// TCP: the peer's FIN was seen, so a later close is orderly (else it
    /// was reset).
    fin_seen: bool,
    /// An AF_INET6 (or dual-stack) socket: the IPv6 tables list it, the
    /// IPv4 ones do not.
    v6: bool,
    /// RAW: the IP protocol, and whether IPv4 datagrams carry their header.
    protocol: u8,
    header_included: bool,
    /// UDP and raw: unicast and multicast TTL.
    ttl: u8,
    multicast_ttl: u8,
    /// UDP: whether this host's own datagrams to a joined group reach the
    /// socket (Winsock's receive-side IP_MULTICAST_LOOP), the groups
    /// joined, and those datagrams until read (the network never returns
    /// a host's own multicast).
    multicast_loop: bool,
    /// The local address BIND named (None: any), as on Windows: a socket
    /// bound to 127.0.0.1 is reachable from this host only.
    address: Option<IpAddress>,
    /// IP_MULTICAST_IF (`net::option::MULTICAST_INTERFACE`).
    multicast_interface: u32,
    groups: Vec<IpAddress>,
    looped: Vec<(IpEndpoint, Vec<u8>)>,
    /// UDP and raw: where it sent (the firewall lets the answers in).
    contacted: Vec<(IpAddress, u16)>,
}
impl ClientSocket {
    fn new(channel: Handle, pid: u64, id: u32, kind: u8, handle: SocketHandle, v6: bool) -> Self {
        Self {
            channel,
            pid,
            id,
            kind,
            handle,
            peer: None,
            local: 0,
            pool: Vec::new(),
            inbound: Vec::new(),
            outbound: Vec::new(),
            fin_seen: false,
            v6,
            protocol: 0,
            header_included: false,
            ttl: net::DEFAULT_TTL,
            multicast_ttl: 1,
            multicast_loop: true,
            multicast_interface: 0,
            address: None,
            groups: Vec::new(),
            looped: Vec::new(),
            contacted: Vec::new(),
        }
    }
    /// Sockets it holds against the client's quota.
    fn weight(&self) -> usize {
        1 + self.pool.len()
    }
}
fn tcp_socket() -> tcp::Socket<'static> {
    let mut socket = tcp::Socket::new(
        tcp::SocketBuffer::new(vec![0; TCP_BUFFER]),
        tcp::SocketBuffer::new(vec![0; TCP_BUFFER]),
    );
    // Windows' TTL, not smoltcp's 64.
    socket.set_hop_limit(Some(net::DEFAULT_TTL));
    socket
}
fn udp_socket() -> udp::Socket<'static> {
    let mut socket = udp::Socket::new(
        udp::PacketBuffer::new(
            vec![udp::PacketMetadata::EMPTY; UDP_PACKETS],
            vec![0; UDP_BUFFER],
        ),
        udp::PacketBuffer::new(
            vec![udp::PacketMetadata::EMPTY; UDP_PACKETS],
            vec![0; UDP_BUFFER],
        ),
    );
    socket.set_hop_limit(Some(net::DEFAULT_TTL));
    socket
}
/// A client's raw socket: every datagram of `protocol` the host receives.
fn raw_socket(v6: bool, protocol: u8) -> raw::Socket<'static> {
    raw::Socket::new(
        if v6 { IpVersion::Ipv6 } else { IpVersion::Ipv4 },
        IpProtocol::from(protocol),
        raw::PacketBuffer::new(
            vec![raw::PacketMetadata::EMPTY; UDP_PACKETS],
            vec![0; UDP_BUFFER],
        ),
        raw::PacketBuffer::new(
            vec![raw::PacketMetadata::EMPTY; UDP_PACKETS],
            vec![0; UDP_BUFFER],
        ),
    )
}
/// A pooled listener socket holds a connection (ACCEPT may take it).
fn connected(socket: &tcp::Socket<'_>) -> bool {
    matches!(
        socket.state(),
        tcp::State::Established | tcp::State::CloseWait
    )
}
/// An endpoint in the IPv4 requests' form (IPv6 ones have none: zeros).
fn endpoint(e: Option<IpEndpoint>) -> ([u8; 4], u16) {
    match e {
        Some(IpEndpoint {
            addr: IpAddress::Ipv4(a),
            port,
        }) => (octets(a), port),
        _ => ([0; 4], 0),
    }
}
/// An endpoint in the `net::v6` form (IPv4 as `::ffff:a.b.c.d`).
fn endpoint16(e: Option<IpEndpoint>) -> ([u8; 16], u16) {
    match e {
        Some(e) => (address16(e.addr), e.port),
        None => ([0; 16], 0),
    }
}
fn address16(a: IpAddress) -> [u8; 16] {
    match a {
        IpAddress::Ipv4(v4) => net::v6::mapped(v4.octets()),
        IpAddress::Ipv6(v6) => v6.octets(),
    }
}
/// A `net::v6` address as smoltcp's: an IPv4-mapped one is IPv4.
fn from16(a: [u8; 16]) -> IpAddress {
    match net::v6::unmapped(&a) {
        Some(v4) => IpAddress::Ipv4(ipv4(v4)),
        None => IpAddress::Ipv6(Ipv6Address::from(a)),
    }
}
struct Query {
    channel: Handle,
    id: u32,
    handle: dns::QueryHandle,
    /// An AAAA query (answered with 16-byte addresses).
    v6: bool,
}

pub struct Network {
    nic: Option<Nic>,
    mac: [u8; 6],
    iface: Option<Interface>,
    sockets: SocketSet<'static>,
    dhcp: Option<SocketHandle>,
    dns: Option<SocketHandle>,
    config: Config,
    state: Dhcp,
    address: Option<Ipv4Cidr>,
    gateway: Option<Ipv4Address>,
    dhcp_dns: [[u8; 4]; 2],
    clients: Vec<ClientSocket>,
    queries: Vec<Query>,
    next_id: u32,
    vpn: vpn::Vpn,
    pings: icmp::Pings,
    /// IPv6: link-local address, router discovery, autoconfiguration.
    slaac: Option<ipv6::Slaac>,
    /// The identification of the next raw IPv4 datagram.
    raw_ident: u16,
    /// When the stack last learned its own addresses (`loopback::seed`).
    loopback_seeded: Option<Instant>,
    firewall: firewall::Firewall,
}
/// Random bits from the kernel's generator (door-network may read it, for
/// WireGuard keys and the stack's sequence numbers, ports and DNS IDs);
/// when it is not ready yet, the clock mixed into a counter, so values
/// still differ.
fn random_u64() -> u64 {
    let mut b = [0u8; 8];
    if door_sys::entropy_read(&mut b) == Ok(8) {
        return u64::from_le_bytes(b);
    }
    static FALLBACK: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
    let mut z = FALLBACK.fetch_add(0x9e37_79b9_7f4a_7c15, core::sync::atomic::Ordering::Relaxed)
        ^ door_sys::time().unwrap_or(0).rotate_left(29);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}
fn now() -> Instant {
    let ticks = door_sys::time().unwrap_or(0);
    Instant::from_millis((ticks * door_abi::platform::MONOTONIC_TICK_MILLIS) as i64)
}
fn ipv4(a: [u8; 4]) -> Ipv4Address {
    Ipv4Address::new(a[0], a[1], a[2], a[3])
}
fn octets(a: Ipv4Address) -> [u8; 4] {
    a.octets()
}
impl Network {
    fn new(handle: Option<Handle>, mac: [u8; 6]) -> Self {
        let mut network = Self {
            // Without an adapter the stack still runs the loopback
            // interface, under a locally administered address.
            nic: Some(Nic {
                handle,
                wire: Cell::new(false),
                mac: if handle.is_some() {
                    mac
                } else {
                    [0x02, 0, 0, 0, 0, 1]
                },
                rx: [0; door_abi::platform::NET_FRAME_BYTES],
                frames_in: Cell::new(0),
                frames_out: Cell::new(0),
                bytes_in: Cell::new(0),
                bytes_out: Cell::new(0),
                divert: RefCell::new(vpn::Divert::default()),
                stats: RefCell::new(stats::Stats::default()),
                looped: RefCell::new(VecDeque::new()),
                gate: RefCell::new(firewall::Gate::default()),
            }),
            mac,
            iface: None,
            sockets: SocketSet::new(Vec::new()),
            dhcp: None,
            dns: None,
            config: Config::default(),
            state: Dhcp::Off,
            address: None,
            gateway: None,
            dhcp_dns: [[0; 4]; 2],
            clients: Vec::new(),
            queries: Vec::new(),
            next_id: 1,
            vpn: vpn::Vpn::default(),
            pings: icmp::Pings::default(),
            slaac: None,
            raw_ident: 0,
            loopback_seeded: None,
            firewall: firewall::Firewall::default(),
        };
        // Until Init hands over the stored policy: Windows' defaults.
        network.refresh_gate();
        network.apply();
        network
    }
    /// (Re)build the interface from the configuration.
    fn apply(&mut self) {
        // A VPN does not outlive the adapter it runs over.
        self.vpn_teardown(Some(net::vpn::Failure::NoNetwork));
        for handle in [self.dhcp.take(), self.dns.take()].into_iter().flatten() {
            self.sockets.remove(handle);
        }
        if let Some(slaac) = self.slaac.take() {
            slaac.remove(&mut self.sockets);
        }
        self.address = None;
        self.gateway = None;
        self.state = Dhcp::Off;
        let Some(nic) = self.nic.as_mut() else {
            self.iface = None;
            return;
        };
        // Without the adapter (absent, or off in airplane mode) only the
        // loopback interface runs.
        let wire = nic.handle.is_some() && self.config.enabled;
        nic.wire.set(wire);
        nic.looped.get_mut().clear();
        self.loopback_seeded = None;
        let mac = nic.mac;
        let mut cfg = IfaceConfig::new(HardwareAddress::Ethernet(EthernetAddress(mac)));
        // smoltcp draws TCP initial sequence numbers, its own ports and DNS
        // IDs from this seed.
        cfg.random_seed = random_u64();
        let mut iface = Interface::new(cfg, nic, now());
        if !wire {
            loopback_last(&mut iface);
            self.iface = Some(iface);
            self.rejoin_groups();
            return;
        }
        self.slaac = Some(ipv6::Slaac::new(
            &mut iface,
            &mut self.sockets,
            self.mac,
            now(),
        ));
        match self.config.fixed {
            Some((address, prefix, gateway)) => {
                let cidr = Ipv4Cidr::new(ipv4(address), prefix);
                iface.update_ip_addrs(|addrs| {
                    let _ = addrs.push(IpCidr::Ipv4(cidr));
                });
                if gateway != [0; 4] {
                    let _ = iface.routes_mut().add_default_ipv4_route(ipv4(gateway));
                    self.gateway = Some(ipv4(gateway));
                }
                self.address = Some(cidr);
                self.state = Dhcp::Static;
            }
            None => {
                self.dhcp = Some(self.sockets.add(dhcpv4::Socket::new()));
                self.state = Dhcp::Discovering;
            }
        }
        loopback_last(&mut iface);
        self.iface = Some(iface);
        self.refresh_dns();
        self.rejoin_groups();
        door_rt::diagnostic(format_args!(
            "DOOR_P15 NET CONFIGURE enabled={} mode={:?}\n",
            u8::from(self.config.enabled),
            self.state
        ));
    }
    /// DNS servers: a connected VPN's, else the manual ones, else DHCP's.
    fn servers(&self) -> Vec<IpAddress> {
        if let Some(tunnel) = self.vpn_dns() {
            return tunnel;
        }
        let manual: Vec<[u8; 4]> = self
            .config
            .dns
            .iter()
            .copied()
            .filter(|a| *a != [0; 4])
            .collect();
        let list = if manual.is_empty() {
            self.dhcp_dns
                .iter()
                .copied()
                .filter(|a| *a != [0; 4])
                .collect()
        } else {
            manual
        };
        list.into_iter().map(|a| IpAddress::Ipv4(ipv4(a))).collect()
    }
    fn refresh_dns(&mut self) {
        // The firewall's "DNS" keyword follows the servers.
        self.loopback_seeded = None;
        let servers = self.servers();
        if let Some(h) = self.dns {
            self.sockets
                .get_mut::<dns::Socket>(h)
                .update_servers(&servers);
        } else if !servers.is_empty() {
            let queries: Vec<Option<dns::DnsQuery>> = (0..DNS_QUERIES).map(|_| None).collect();
            self.dns = Some(self.sockets.add(dns::Socket::new(&servers, queries)));
        }
    }
    /// One stack step: frames in and out, timers, DHCP events. True when
    /// anything happened.
    pub fn poll(&mut self) -> bool {
        self.seed_loopback();
        let (Some(iface), Some(nic)) = (self.iface.as_mut(), self.nic.as_mut()) else {
            return false;
        };
        let before = nic.frames_in.get() + nic.frames_out.get();
        // Loopback frames the stack sent itself are taken in the same
        // step, round after round while they keep coming.
        let mut looped = false;
        for _ in 0..LOOPBACK_ROUNDS {
            iface.poll(now(), nic, &mut self.sockets);
            if nic.looped.get_mut().is_empty() {
                break;
            }
            looped = true;
        }
        let mut changed = looped || nic.frames_in.get() + nic.frames_out.get() != before;
        if let Some(h) = self.dhcp {
            let event = self.sockets.get_mut::<dhcpv4::Socket>(h).poll();
            match event {
                Some(dhcpv4::Event::Configured(lease)) => {
                    let cidr = lease.address;
                    iface.update_ip_addrs(|addrs| {
                        // IPv6 addresses stay; the lease replaces IPv4.
                        addrs.retain(|c| matches!(c, IpCidr::Ipv6(_)));
                        let _ = addrs.push(IpCidr::Ipv4(cidr));
                    });
                    loopback_last(iface);
                    if let Some(router) = lease.router {
                        let _ = iface.routes_mut().add_default_ipv4_route(router);
                    } else {
                        iface.routes_mut().remove_default_ipv4_route();
                    }
                    self.address = Some(cidr);
                    self.gateway = lease.router;
                    self.dhcp_dns = [[0; 4]; 2];
                    for (slot, server) in self.dhcp_dns.iter_mut().zip(lease.dns_servers.iter()) {
                        *slot = octets(*server);
                    }
                    self.state = Dhcp::Bound;
                    self.loopback_seeded = None;
                    door_rt::diagnostic(format_args!(
                        "DOOR_P15 NET LEASE ip={} gateway={} dns={}\n",
                        cidr,
                        lease.router.unwrap_or(Ipv4Address::UNSPECIFIED),
                        ipv4(self.dhcp_dns[0])
                    ));
                    self.refresh_dns();
                    changed = true;
                }
                Some(dhcpv4::Event::Deconfigured) => {
                    iface.update_ip_addrs(|addrs| addrs.retain(|c| matches!(c, IpCidr::Ipv6(_))));
                    loopback_last(iface);
                    iface.routes_mut().remove_default_ipv4_route();
                    self.address = None;
                    self.gateway = None;
                    self.state = Dhcp::Discovering;
                    door_rt::diagnostic(format_args!("DOOR_P15 NET DECONFIGURED\n"));
                    changed = true;
                }
                None => (),
            }
        }
        if let (Some(iface), Some(slaac)) = (self.iface.as_mut(), self.slaac.as_mut())
            && slaac.poll(iface, &mut self.sockets, now())
        {
            changed = true;
            self.loopback_seeded = None;
        }
        if self.pings.poll(&mut self.sockets, now()) {
            changed = true;
        }
        if self.vpn_poll() {
            // What the tunnel produced goes out (and in) in this step.
            if let (Some(iface), Some(nic)) = (self.iface.as_mut(), self.nic.as_mut()) {
                iface.poll(now(), nic, &mut self.sockets);
            }
            changed = true;
        }
        changed
    }
    /// Keep the stack's neighbour entries for its own addresses fresh, so
    /// loopback traffic never waits on address resolution.
    fn seed_loopback(&mut self) {
        const REFRESH_MILLIS: i64 = 20_000;
        let now = now();
        if self
            .loopback_seeded
            .is_some_and(|at| (now - at).total_millis() < REFRESH_MILLIS as u64)
        {
            return;
        }
        let v6: Vec<[u8; 16]> = core::iter::once(loopback::V6)
            .chain(self.slaac.as_ref().into_iter().flat_map(|s| {
                core::iter::once(s.link_local.octets())
                    .chain(s.global.map(|(c, _)| c.address().octets()))
            }))
            .collect();
        let v4 = [loopback::V4, self.local_address()];
        self.refresh_locality(&v6[1..]);
        let Some(nic) = self.nic.as_mut() else {
            return;
        };
        let looped = nic.looped.get_mut();
        for frame in loopback::seed(nic.mac, &v4, &v6) {
            if looped.len() < LOOPED_FRAMES {
                looped.push_back(frame);
            }
        }
        self.loopback_seeded = Some(now);
    }
    fn status(&self) -> net::Status {
        let link = self
            .nic
            .as_ref()
            .and_then(|n| n.handle)
            .and_then(|h| door_sys::net_info(h).ok())
            .is_some_and(|i| i.link);
        let (frames_in, frames_out, bytes_in, bytes_out) =
            self.nic.as_ref().map_or((0, 0, 0, 0), |n| {
                (
                    n.frames_in.get(),
                    n.frames_out.get(),
                    n.bytes_in.get(),
                    n.bytes_out.get(),
                )
            });
        let servers = self.servers();
        let mut dns = [[0u8; 4]; 2];
        let v4 = servers.iter().filter_map(|s| match s {
            IpAddress::Ipv4(a) => Some(octets(*a)),
            IpAddress::Ipv6(_) => None,
        });
        for (slot, server) in dns.iter_mut().zip(v4) {
            *slot = server;
        }
        net::Status {
            present: self.nic.as_ref().is_some_and(|n| n.handle.is_some()),
            link,
            enabled: self.config.enabled,
            mac: self.mac,
            dhcp: self.state,
            address: self.address.map(|c| octets(c.address())).unwrap_or([0; 4]),
            prefix: self.address.map(|c| c.prefix_len()).unwrap_or(0),
            gateway: self.gateway.map(octets).unwrap_or([0; 4]),
            dns,
            lease_seconds: 0,
            frames_in,
            frames_out,
            bytes_in,
            bytes_out,
            config: self.config,
        }
    }
    fn socket(&self, channel: Handle, id: u32) -> Option<usize> {
        self.clients
            .iter()
            .position(|s| s.channel == channel && s.id == id)
    }
    /// A port from the dynamic range (49152-65535), at random so an
    /// off-path host cannot guess it (RFC 6056).
    fn ephemeral(&mut self) -> u16 {
        49152 + (random_u64() % 16384) as u16
    }
    fn close_socket(&mut self, index: usize) {
        let s = self.clients.remove(index);
        for group in &s.groups {
            self.release_group(*group);
        }
        if s.kind == net::TCP {
            self.sockets.get_mut::<tcp::Socket>(s.handle).abort();
            for h in &s.pool {
                self.sockets.get_mut::<tcp::Socket>(*h).abort();
            }
        }
        self.sockets.remove(s.handle);
        let listener = !s.pool.is_empty();
        for h in s.pool {
            self.sockets.remove(h);
        }
        if listener {
            self.refresh_gate();
        }
    }
    /// A port no socket of `kind` holds (`port` itself, or an ephemeral one
    /// when 0); `Exists` when taken.
    fn free_port(&mut self, kind: u8, port: u16) -> Result<u16, Code> {
        // A VPN tunnel holds its UDP port too.
        let tunnel = if kind == net::UDP {
            self.vpn.port()
        } else {
            None
        };
        let taken = |clients: &[ClientSocket], p: u16| {
            tunnel == Some(p) || clients.iter().any(|s| s.kind == kind && s.local == p)
        };
        if port != 0 {
            return if taken(&self.clients, port) {
                Err(Code::Exists)
            } else {
                Ok(port)
            };
        }
        for _ in 0..16384 {
            let p = self.ephemeral();
            if !taken(&self.clients, p) {
                return Ok(p);
            }
        }
        Err(Code::Resources)
    }
    /// Bind UDP socket `index` (to `port`, ephemeral when 0) unless bound.
    fn bind_udp(&mut self, index: usize, port: u16) -> Result<u16, Code> {
        let bound = self.clients[index].local;
        if bound != 0 {
            return if port == 0 || port == bound {
                Ok(bound)
            } else {
                Err(Code::InvalidRequest)
            };
        }
        let port = self.free_port(net::UDP, port)?;
        let handle = self.clients[index].handle;
        self.sockets
            .get_mut::<udp::Socket>(handle)
            .bind(smoltcp::wire::IpListenEndpoint {
                addr: self.clients[index].address,
                port,
            })
            .map_err(|_| Code::Resources)?;
        self.clients[index].local = port;
        Ok(port)
    }
    /// Join or leave `group` for UDP socket `index`; the adapter listens
    /// for a group while any socket holds it.
    fn multicast(&mut self, index: usize, action: u8, group: IpAddress) -> Result<(), Code> {
        let s = &self.clients[index];
        // An AF_INET socket joins IPv4 groups; an AF_INET6 one either.
        if s.kind != net::UDP
            || !group.is_multicast()
            || (!s.v6 && matches!(group, IpAddress::Ipv6(_)))
        {
            return Err(Code::InvalidRequest);
        }
        match action {
            op::MULTICAST_JOIN => {
                if s.groups.contains(&group) {
                    return Err(Code::Exists);
                }
                if s.groups.len() >= MAX_MEMBERSHIPS {
                    return Err(Code::Resources);
                }
                let first = !self.clients.iter().any(|c| c.groups.contains(&group));
                if first && let Some(iface) = self.iface.as_mut() {
                    iface
                        .join_multicast_group(group)
                        .map_err(|_| Code::Resources)?;
                }
                self.clients[index].groups.push(group);
                Ok(())
            }
            op::MULTICAST_LEAVE => {
                let at = s
                    .groups
                    .iter()
                    .position(|g| *g == group)
                    .ok_or(Code::NotFound)?;
                self.clients[index].groups.remove(at);
                self.release_group(group);
                Ok(())
            }
            _ => Err(Code::InvalidRequest),
        }
    }
    /// The adapter stops listening for `group` once no socket holds it.
    fn release_group(&mut self, group: IpAddress) {
        if !self.clients.iter().any(|c| c.groups.contains(&group))
            && let Some(iface) = self.iface.as_mut()
        {
            let _ = iface.leave_multicast_group(group);
        }
    }
    /// After the interface is rebuilt: listen again for the groups held.
    fn rejoin_groups(&mut self) {
        let mut groups: Vec<IpAddress> = Vec::new();
        for g in self.clients.iter().flat_map(|c| c.groups.iter()) {
            if !groups.contains(g) {
                groups.push(*g);
            }
        }
        if let Some(iface) = self.iface.as_mut() {
            for g in groups {
                let _ = iface.join_multicast_group(g);
            }
        }
    }
    /// This host's own datagram to `group`:`port` for every socket on that
    /// port that joined the group and takes looped multicast (the network
    /// does not send a host its own multicast back).
    fn loop_back(&mut self, from: usize, group: IpAddress, port: u16, data: &[u8]) {
        // From the interface it left by: the loopback interface's address,
        // else the adapter's.
        let interface = self.clients[from].multicast_interface;
        let source = match group {
            IpAddress::Ipv4(_) if interface != 0 => IpAddress::Ipv4(ipv4(interface.to_le_bytes())),
            IpAddress::Ipv4(_) => IpAddress::Ipv4(ipv4(self.local_address())),
            IpAddress::Ipv6(_) if interface == net::LOOPBACK_INDEX => {
                IpAddress::Ipv6(Ipv6Address::from(loopback::V6))
            }
            IpAddress::Ipv6(g) => IpAddress::Ipv6(self.local_v6(g)),
        };
        let source = IpEndpoint::new(source, self.clients[from].local);
        for s in self.clients.iter_mut() {
            if s.kind == net::UDP
                && s.local == port
                && s.multicast_loop
                && s.groups.contains(&group)
                && s.looped.len() < net::LOOPED_DATAGRAMS
            {
                s.looped.push((source, data.to_vec()));
            }
        }
    }
    /// One of this host's addresses: the loopback interface's (127.0.0.0/8,
    /// ::1) or the adapter's.
    fn is_own(&self, a: IpAddress) -> bool {
        match a {
            IpAddress::Ipv4(v4) => v4.octets()[0] == 127 || v4.octets() == self.local_address(),
            IpAddress::Ipv6(v6) => {
                v6.octets() == loopback::V6
                    || self.slaac.as_ref().is_some_and(|s| {
                        s.link_local == v6 || s.global.is_some_and(|(c, _)| c.address() == v6)
                    })
            }
        }
    }
    /// The IPv6 source towards `to`: the link-local address for link
    /// scope, else the global one when there is one.
    fn local_v6(&self, to: Ipv6Address) -> Ipv6Address {
        let o = to.octets();
        let link = (o[0] == 0xfe && o[1] & 0xc0 == 0x80) || (o[0] == 0xff && o[1] & 0x0f <= 2);
        self.slaac
            .as_ref()
            .map(|v6| {
                if link {
                    v6.link_local
                } else {
                    v6.global.map_or(v6.link_local, |(c, _)| c.address())
                }
            })
            .unwrap_or(Ipv6Address::UNSPECIFIED)
    }
    /// The next datagram for UDP or raw socket `index` and its sender:
    /// what this host looped back first, then the network's. A UDP socket
    /// takes a group's datagrams only once it joined the group; a raw IPv4
    /// datagram keeps its header, an IPv6 one does not.
    fn next_datagram(&mut self, index: usize) -> Option<(IpEndpoint, Vec<u8>)> {
        let handle = self.clients[index].handle;
        if self.clients[index].kind == net::RAW {
            loop {
                let socket = self.sockets.get_mut::<raw::Socket>(handle);
                let packet = socket.recv().ok()?;
                let datagram = if let Ok(p) = smoltcp::wire::Ipv4Packet::new_checked(packet)
                    && p.version() == 4
                {
                    let from = IpAddress::Ipv4(p.src_addr());
                    (IpEndpoint::new(from, 0), packet.to_vec())
                } else if let Ok(p) = smoltcp::wire::Ipv6Packet::new_checked(packet)
                    && p.version() == 6
                {
                    let from = IpAddress::Ipv6(p.src_addr());
                    (IpEndpoint::new(from, 0), p.payload().to_vec())
                } else {
                    continue;
                };
                // The firewall: answers to what it sent, or by the rules.
                if self.datagram_permitted(index, datagram.0.addr, 0) {
                    return Some(datagram);
                }
            }
        }
        if !self.clients[index].looped.is_empty() {
            return Some(self.clients[index].looped.remove(0));
        }
        let mut whole = vec![0u8; UDP_BUFFER];
        loop {
            let (n, meta) = self
                .sockets
                .get_mut::<udp::Socket>(handle)
                .recv_slice(&mut whole)
                .ok()?;
            if let Some(to) = meta.local_address
                && to.is_multicast()
                && !self.clients[index].groups.contains(&to)
            {
                continue;
            }
            if !self.datagram_permitted(index, meta.endpoint.addr, meta.endpoint.port) {
                continue;
            }
            whole.truncate(n);
            return Some((meta.endpoint, whole));
        }
    }
    /// One datagram from raw socket `index` to `address`: the stack adds
    /// the IP header (IPv4 with IP_HDRINCL: completes the caller's) and an
    /// ICMPv6 checksum, as Windows does.
    fn send_raw(&mut self, index: usize, address: IpAddress, data: &[u8]) -> Response {
        let s = &self.clients[index];
        let (protocol, handle, v6, header_included) =
            (s.protocol, s.handle, s.v6, s.header_included);
        let hop = if address.is_multicast() {
            s.multicast_ttl
        } else {
            s.ttl
        }
        .max(1);
        // TCP segments never go out through a raw socket on Windows.
        if protocol == 6 {
            return Response::error(Code::InvalidRequest);
        }
        if !self.permitted(index, door_firewall::Direction::Out, address, 0) {
            return Response::error(Code::Denied);
        }
        self.contacted(index, address, 0);
        self.raw_ident = self.raw_ident.wrapping_add(1);
        let ident = self.raw_ident;
        let packet = match address {
            IpAddress::Ipv4(to) if !v6 => {
                let source = match self.vpn_source(to.octets()) {
                    Some(IpAddress::Ipv4(v4)) => v4.octets(),
                    _ => self.local_address(),
                };
                if source == [0; 4] {
                    return Response::error(Code::Unsupported);
                }
                if header_included {
                    match included_header(data, protocol, source, to.octets(), ident) {
                        Some(p) => p,
                        None => return Response::error(Code::InvalidRequest),
                    }
                } else {
                    let mut p = vec![0u8; 20 + data.len()];
                    p[0] = 0x45;
                    p[2..4].copy_from_slice(&((20 + data.len()) as u16).to_be_bytes());
                    p[4..6].copy_from_slice(&ident.to_be_bytes());
                    p[8] = hop;
                    p[9] = protocol;
                    p[12..16].copy_from_slice(&source);
                    p[16..20].copy_from_slice(&to.octets());
                    p[20..].copy_from_slice(data);
                    p
                }
            }
            IpAddress::Ipv6(to) if v6 => {
                let source = self.local_v6(to);
                if source.is_unspecified() {
                    return Response::error(Code::Unsupported);
                }
                let mut p = vec![0u8; 40 + data.len()];
                p[0] = 0x60;
                p[4..6].copy_from_slice(&(data.len() as u16).to_be_bytes());
                p[6] = protocol;
                p[7] = hop;
                p[8..24].copy_from_slice(&source.octets());
                p[24..40].copy_from_slice(&to.octets());
                p[40..].copy_from_slice(data);
                // ICMPv6: the stack computes the checksum (RFC 3542).
                if protocol == 58 && data.len() >= 4 {
                    p[42..44].fill(0);
                    let mut pseudo = [0u8; 40];
                    pseudo[..16].copy_from_slice(&source.octets());
                    pseudo[16..32].copy_from_slice(&to.octets());
                    pseudo[32..36].copy_from_slice(&(data.len() as u32).to_be_bytes());
                    pseudo[39] = 58;
                    let sum = internet_checksum(&[&pseudo, &p[40..]]);
                    p[42..44].copy_from_slice(&sum.to_be_bytes());
                }
                p
            }
            _ => return Response::error(Code::InvalidRequest),
        };
        match self
            .sockets
            .get_mut::<raw::Socket>(handle)
            .send_slice(&packet)
        {
            Ok(()) => Response::scalar(data.len() as u64),
            Err(_) => Response::error(Code::WouldBlock),
        }
    }
    fn quota(&self, channel: Handle) -> usize {
        self.clients
            .iter()
            .filter(|s| s.channel == channel)
            .map(ClientSocket::weight)
            .sum()
    }
    /// The frame counters plus what the sockets say now: established
    /// connections, TCP rows and bound UDP ports (MIB_TCPSTATS dwCurrEstab
    /// and dwNumConns, MIB_UDPSTATS dwNumAddrs).
    fn statistics(&self) -> net::stats::Counters {
        use net::tcp_state as t;
        let mut counters = self
            .nic
            .as_ref()
            .map(|nic| nic.stats.borrow().counters)
            .unwrap_or_default();
        for c in self.connections() {
            if c.protocol == 17 {
                counters.v4.udp_num_addrs += 1;
            } else {
                counters.v4.tcp_num_conns += 1;
                if matches!(c.state, t::ESTABLISHED | t::CLOSE_WAIT) {
                    counters.v4.tcp_curr_estab += 1;
                }
            }
        }
        // IPv6 connections (the IPv4 table above leaves them out).
        for c in self.clients.iter().filter(|c| c.kind == net::TCP) {
            for handle in core::iter::once(c.handle).chain(c.pool.iter().copied()) {
                let socket = self.sockets.get::<tcp::Socket>(handle);
                if !matches!(
                    socket.remote_endpoint().map(|e| e.addr),
                    Some(IpAddress::Ipv6(_))
                ) {
                    continue;
                }
                counters.v6.tcp_num_conns += 1;
                if matches!(
                    socket.state(),
                    tcp::State::Established | tcp::State::CloseWait
                ) {
                    counters.v6.tcp_curr_estab += 1;
                }
            }
        }
        counters
    }
    /// The ARP table as `arp -a` shows it: the neighbours ARP traffic
    /// taught, then the fixed broadcast entries of the adapter's subnet.
    fn neighbors(&self) -> Vec<net::stats::Neighbor> {
        use net::stats::{NEIGHBOR_STATIC, Neighbor};
        let mut all = self
            .nic
            .as_ref()
            .map(|nic| nic.stats.borrow().neighbors(now().total_millis() as u64))
            .unwrap_or_default();
        all.sort_by_key(|n| n.address);
        if let Some(cidr) = self.address {
            let address = u32::from_be_bytes(octets(cidr.address()));
            let mask = u32::MAX
                .checked_shl(32 - u32::from(cidr.prefix_len()))
                .unwrap_or(0);
            all.push(Neighbor::v4(
                (address | !mask).to_be_bytes(),
                [0xff; 6],
                NEIGHBOR_STATIC,
            ));
            all.push(Neighbor::v4([255; 4], [0xff; 6], NEIGHBOR_STATIC));
        }
        all
    }
    /// Every application socket (and a listener's pooled connections),
    /// as netstat lists them.
    fn connections(&self) -> Vec<net::Connection> {
        use net::tcp_state as t;
        let local = self.local_address();
        let mut out = Vec::new();
        for c in &self.clients {
            // AF_INET6 sockets are in the IPv6 table (`connections6`).
            if c.v6 {
                continue;
            }
            let pid = c.pid as u32;
            if c.kind == net::UDP {
                if c.local != 0 {
                    out.push(net::Connection {
                        protocol: 17,
                        state: 0,
                        local: [0; 4],
                        local_port: c.local,
                        remote: [0; 4],
                        remote_port: 0,
                        pid,
                    });
                }
                continue;
            }
            for (i, handle) in core::iter::once(c.handle)
                .chain(c.pool.iter().copied())
                .enumerate()
            {
                let socket = self.sockets.get::<tcp::Socket>(handle);
                // IPv6 connections belong to the IPv6 table, not this one.
                if matches!(
                    socket.remote_endpoint().map(|e| e.addr),
                    Some(IpAddress::Ipv6(_))
                ) {
                    continue;
                }
                use tcp::State as S;
                let state = match socket.state() {
                    // A pooled listener socket without a connection is the
                    // listener itself, reported once.
                    S::Listen if i == 0 || c.pool.is_empty() => t::LISTEN,
                    S::Listen => continue,
                    S::Closed if i == 0 && !c.pool.is_empty() => t::LISTEN,
                    S::Closed => continue,
                    S::SynSent => t::SYN_SENT,
                    S::SynReceived => t::SYN_RCVD,
                    S::Established => t::ESTABLISHED,
                    S::FinWait1 => t::FIN_WAIT1,
                    S::FinWait2 => t::FIN_WAIT2,
                    S::CloseWait => t::CLOSE_WAIT,
                    S::Closing => t::CLOSING,
                    S::LastAck => t::LAST_ACK,
                    S::TimeWait => t::TIME_WAIT,
                };
                if state == t::LISTEN
                    && out.iter().any(|o: &net::Connection| {
                        o.state == t::LISTEN && o.local_port == c.local && o.protocol == 6
                    })
                {
                    continue;
                }
                let (local_address, local_port) = match socket.local_endpoint() {
                    Some(e) => endpoint(Some(e)),
                    None => ([0; 4], c.local),
                };
                let (remote, remote_port) = endpoint(socket.remote_endpoint());
                out.push(net::Connection {
                    protocol: 6,
                    state,
                    local: if state == t::LISTEN {
                        [0; 4]
                    } else if local_address == [0; 4] {
                        local
                    } else {
                        local_address
                    },
                    local_port,
                    remote,
                    remote_port,
                    pid,
                });
            }
        }
        out
    }
    /// The AF_INET6 sockets, as the IPv6 tables and netstat list them: a
    /// listener on `[::]`, a dual-stack socket's IPv4 peer as
    /// `::ffff:a.b.c.d`, a link-local address with its interface's scope.
    fn connections6(&self) -> Vec<net::Connection6> {
        use net::tcp_state as t;
        let scope = |a: &[u8; 16]| -> u32 {
            if a[0] == 0xfe && a[1] & 0xc0 == 0x80 {
                ipv6::SCOPE
            } else {
                0
            }
        };
        let mut out: Vec<net::Connection6> = Vec::new();
        for c in self.clients.iter().filter(|c| c.v6) {
            let pid = c.pid as u32;
            if c.kind == net::UDP {
                if c.local != 0 {
                    out.push(net::Connection6 {
                        protocol: 17,
                        local_port: c.local,
                        pid,
                        ..Default::default()
                    });
                }
                continue;
            }
            for (i, handle) in core::iter::once(c.handle)
                .chain(c.pool.iter().copied())
                .enumerate()
            {
                let socket = self.sockets.get::<tcp::Socket>(handle);
                use tcp::State as S;
                let state = match socket.state() {
                    S::Listen if i == 0 || c.pool.is_empty() => t::LISTEN,
                    S::Listen => continue,
                    S::Closed if i == 0 && !c.pool.is_empty() => t::LISTEN,
                    S::Closed => continue,
                    S::SynSent => t::SYN_SENT,
                    S::SynReceived => t::SYN_RCVD,
                    S::Established => t::ESTABLISHED,
                    S::FinWait1 => t::FIN_WAIT1,
                    S::FinWait2 => t::FIN_WAIT2,
                    S::CloseWait => t::CLOSE_WAIT,
                    S::Closing => t::CLOSING,
                    S::LastAck => t::LAST_ACK,
                    S::TimeWait => t::TIME_WAIT,
                };
                if state == t::LISTEN
                    && out
                        .iter()
                        .any(|o| o.state == t::LISTEN && o.local_port == c.local && o.protocol == 6)
                {
                    continue;
                }
                let (local, local_port) = match socket.local_endpoint() {
                    Some(e) if state != t::LISTEN => endpoint16(Some(e)),
                    _ => ([0; 16], c.local),
                };
                let (remote, remote_port) = endpoint16(socket.remote_endpoint());
                out.push(net::Connection6 {
                    protocol: 6,
                    state,
                    local,
                    local_scope: scope(&local),
                    local_port,
                    remote,
                    remote_scope: scope(&remote),
                    remote_port,
                    pid,
                });
            }
        }
        out
    }
    fn local_address(&self) -> [u8; 4] {
        self.address.map(|c| octets(c.address())).unwrap_or([0; 4])
    }
}
/// The loopback addresses after every other one: smoltcp falls back to the
/// first IPv4 address for a source, which must be the adapter's.
fn loopback_last(iface: &mut Interface) {
    let v4 = IpCidr::Ipv4(Ipv4Cidr::new(ipv4(loopback::V4), loopback::V4_PREFIX));
    let v6 = IpCidr::Ipv6(smoltcp::wire::Ipv6Cidr::new(
        Ipv6Address::from(loopback::V6),
        128,
    ));
    iface.update_ip_addrs(|addrs| {
        addrs.retain(|c| *c != v4 && *c != v6);
        let _ = addrs.push(v4);
        let _ = addrs.push(v6);
    });
    // Every 127.0.0.0/8 address is this host's: the stack takes packets
    // to the ones it does not hold (AnyIP) when routed through 127.0.0.1.
    iface.set_any_ip(true);
    iface.routes_mut().update(|routes| {
        if !routes.iter().any(|r| r.cidr == v4) {
            let _ = routes.push(smoltcp::iface::Route {
                cidr: v4,
                via_router: IpAddress::Ipv4(ipv4(loopback::V4)),
                preferred_until: None,
                expires_at: None,
            });
        }
    });
}
/// The Internet checksum (RFC 1071) over `parts` in order, complemented.
fn internet_checksum(parts: &[&[u8]]) -> u16 {
    let mut sum = 0u32;
    let mut odd: Option<u8> = None;
    for byte in parts.iter().flat_map(|p| p.iter().copied()) {
        match odd.take() {
            Some(high) => sum += u32::from(u16::from_be_bytes([high, byte])),
            None => odd = Some(byte),
        }
    }
    if let Some(high) = odd {
        sum += u32::from(high) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}
/// A raw IPv4 datagram the caller headed (IP_HDRINCL), as Windows sends it:
/// the total length set, and a zero identification or source filled in.
/// The header must name the socket's protocol and carry no options; its
/// destination is the one the firewall checked (`to`, or zero for it) and
/// its source this host's (Windows drops datagrams whose source no
/// interface holds), so a raw socket cannot forge either.
fn included_header(
    data: &[u8],
    protocol: u8,
    source: [u8; 4],
    to: [u8; 4],
    ident: u16,
) -> Option<Vec<u8>> {
    if data.len() < 20 || data[0] != 0x45 || data[9] != protocol || data.len() > 0xffff {
        return None;
    }
    let mut p = data.to_vec();
    p[2..4].copy_from_slice(&(data.len() as u16).to_be_bytes());
    if p[4..6] == [0, 0] {
        p[4..6].copy_from_slice(&ident.to_be_bytes());
    }
    if p[12..16] == [0; 4] {
        p[12..16].copy_from_slice(&source);
    }
    if p[16..20] == [0; 4] {
        p[16..20].copy_from_slice(&to);
    }
    if p[12..16] != source || p[16..20] != to {
        return None;
    }
    if p[8] == 0 {
        return None;
    }
    Some(p)
}
fn word(body: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(body.get(at..at + 4)?.try_into().ok()?))
}
fn state_of(socket: &tcp::Socket<'_>) -> SocketState {
    use tcp::State as S;
    match socket.state() {
        S::Closed => SocketState::Closed,
        S::Listen => SocketState::Listening,
        S::SynSent | S::SynReceived => SocketState::Connecting,
        S::Established => SocketState::Established,
        S::CloseWait => SocketState::PeerClosed,
        S::FinWait1 | S::FinWait2 | S::Closing | S::LastAck | S::TimeWait => SocketState::Closing,
    }
}
impl Handler for Network {
    fn request(
        &mut self,
        channel: Handle,
        peer: Identity,
        opcode: u32,
        body: &[u8],
        _transfers: &[Transfer],
    ) -> Response {
        match opcode {
            op::STATUS if body.is_empty() => Response::bytes(&self.status().encode()),
            net::v6::STATUS if body.is_empty() => Response::bytes(
                &self
                    .slaac
                    .as_ref()
                    .map(ipv6::Slaac::status)
                    .unwrap_or_default()
                    .encode(),
            ),
            op::CONFIGURE => {
                // Only Settings changes the adapter (Network & internet).
                if peer.manifest != manifest::WORKSPACE {
                    return Response::error(Code::Denied);
                }
                let Some(config) = Config::decode(body) else {
                    return Response::error(Code::InvalidRequest);
                };
                if config != self.config {
                    self.config = config;
                    self.apply();
                }
                Response::empty()
            }
            op::RESOLVE | net::v6::RESOLVE => {
                let v6 = opcode == net::v6::RESOLVE;
                let Ok(name) = core::str::from_utf8(body) else {
                    return Response::error(Code::InvalidRequest);
                };
                if name.is_empty() || name.len() > 200 {
                    return Response::error(Code::InvalidRequest);
                }
                // A dotted address answers itself.
                if let (false, Ok(a)) = (v6, name.parse::<Ipv4Address>()) {
                    let id = self.next_id;
                    self.next_id += 1;
                    let mut r = Response::scalar(u64::from(id));
                    r.body[8..12].copy_from_slice(&octets(a));
                    r.len = 12;
                    return r;
                }
                let (Some(iface), Some(h)) = (self.iface.as_mut(), self.dns) else {
                    return Response::error(Code::Unsupported);
                };
                match self.sockets.get_mut::<dns::Socket>(h).start_query(
                    iface.context(),
                    name,
                    if v6 {
                        DnsQueryType::Aaaa
                    } else {
                        DnsQueryType::A
                    },
                ) {
                    Ok(handle) => {
                        let id = self.next_id;
                        self.next_id += 1;
                        self.queries.push(Query {
                            channel,
                            id,
                            handle,
                            v6,
                        });
                        Response::scalar(u64::from(id))
                    }
                    Err(_) => Response::error(Code::Resources),
                }
            }
            op::RESOLVED => {
                let Some(id) = word(body, 0) else {
                    return Response::error(Code::InvalidRequest);
                };
                let Some(at) = self
                    .queries
                    .iter()
                    .position(|q| q.channel == channel && q.id == id)
                else {
                    return Response::error(Code::NotFound);
                };
                let Some(h) = self.dns else {
                    return Response::error(Code::Unsupported);
                };
                match self
                    .sockets
                    .get_mut::<dns::Socket>(h)
                    .get_query_result(self.queries[at].handle)
                {
                    Ok(addresses) => {
                        let v6 = self.queries.remove(at).v6;
                        let mut out = Vec::new();
                        for a in addresses.iter() {
                            match (a, v6) {
                                (IpAddress::Ipv4(v4), false) => out.extend_from_slice(&octets(*v4)),
                                (IpAddress::Ipv6(a), true) => out.extend_from_slice(&a.octets()),
                                _ => (),
                            }
                        }
                        let most = if v6 { 16 * net::v6::MAX_ADDRESSES } else { 32 };
                        // A name without records of that kind does not
                        // resolve (WSANO_DATA / DNS_INFO_NO_RECORDS).
                        if out.is_empty() {
                            return Response::error(Code::NotFound);
                        }
                        Response::bytes(&out[..out.len().min(most)])
                    }
                    Err(dns::GetQueryResultError::Pending) => Response::error(Code::WouldBlock),
                    Err(dns::GetQueryResultError::Failed) => {
                        self.queries.remove(at);
                        Response::error(Code::NotFound)
                    }
                }
            }
            net::diag::PING => {
                let Some(request) = net::EchoRequest::decode(body) else {
                    return Response::error(Code::InvalidRequest);
                };
                let source = self.address.map(|c| c.address());
                match self
                    .pings
                    .start(&mut self.sockets, channel, source, &request, now())
                {
                    Ok(token) => Response::scalar(u64::from(token)),
                    Err(code) => Response::error(code),
                }
            }
            net::diag::PING_RESULT => {
                let Some(token) = word(body, 0).filter(|_| body.len() == 4) else {
                    return Response::error(Code::InvalidRequest);
                };
                match self.pings.take(channel, token) {
                    Ok(echo) => Response::bytes(&echo.encode()),
                    Err(code) => Response::error(code),
                }
            }
            net::diag::CONNECTIONS => {
                let Some(first) = body
                    .get(..2)
                    .filter(|_| body.len() == 2)
                    .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
                else {
                    return Response::error(Code::InvalidRequest);
                };
                let all = self.connections();
                let page: Vec<&net::Connection> =
                    all.iter().skip(first).take(net::diag::PAGE).collect();
                let mut b = [0u8; 4 + net::diag::PAGE * net::CONNECTION_BYTES];
                b[..2]
                    .copy_from_slice(&(all.len().min(usize::from(u16::MAX)) as u16).to_le_bytes());
                b[2..4].copy_from_slice(&(page.len() as u16).to_le_bytes());
                for (i, c) in page.iter().enumerate() {
                    let at = 4 + i * net::CONNECTION_BYTES;
                    c.encode(&mut b[at..at + net::CONNECTION_BYTES]);
                }
                Response::bytes(&b[..4 + page.len() * net::CONNECTION_BYTES])
            }
            net::diag::CONNECTIONS6 => {
                let Some(first) = body
                    .get(..2)
                    .filter(|_| body.len() == 2)
                    .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
                else {
                    return Response::error(Code::InvalidRequest);
                };
                let all = self.connections6();
                let page: Vec<&net::Connection6> =
                    all.iter().skip(first).take(net::diag::PAGE6).collect();
                let mut b = [0u8; 4 + net::diag::PAGE6 * net::CONNECTION6_BYTES];
                b[..2]
                    .copy_from_slice(&(all.len().min(usize::from(u16::MAX)) as u16).to_le_bytes());
                b[2..4].copy_from_slice(&(page.len() as u16).to_le_bytes());
                for (i, c) in page.iter().enumerate() {
                    let at = 4 + i * net::CONNECTION6_BYTES;
                    c.encode(&mut b[at..at + net::CONNECTION6_BYTES]);
                }
                Response::bytes(&b[..4 + page.len() * net::CONNECTION6_BYTES])
            }
            net::stats::STATISTICS => {
                let Some(offset) = body
                    .get(..2)
                    .filter(|_| body.len() == 2)
                    .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
                    .filter(|o| *o < net::stats::COUNTERS_BYTES)
                else {
                    return Response::error(Code::InvalidRequest);
                };
                let all = self.statistics().encode();
                let end = (offset + net::stats::STATISTICS_CHUNK).min(all.len());
                Response::bytes(&all[offset..end])
            }
            net::stats::NEIGHBORS => {
                let Some(first) = body
                    .get(..2)
                    .filter(|_| body.len() == 2)
                    .map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])))
                else {
                    return Response::error(Code::InvalidRequest);
                };
                let all = self.neighbors();
                let page: Vec<&net::stats::Neighbor> =
                    all.iter().skip(first).take(net::stats::PAGE).collect();
                let mut b = [0u8; 4 + net::stats::PAGE * net::stats::NEIGHBOR_BYTES];
                b[..2].copy_from_slice(&(all.len() as u16).to_le_bytes());
                b[2..4].copy_from_slice(&(page.len() as u16).to_le_bytes());
                for (i, n) in page.iter().enumerate() {
                    let at = 4 + i * net::stats::NEIGHBOR_BYTES;
                    n.encode(&mut b[at..at + net::stats::NEIGHBOR_BYTES]);
                }
                Response::bytes(&b[..4 + page.len() * net::stats::NEIGHBOR_BYTES])
            }
            op::SOCKET => {
                let kind = body.first().copied().unwrap_or(0);
                // The optional family byte, AF_INET (2) or AF_INET6, and a
                // raw socket's protocol.
                let length = if kind == net::RAW { 3 } else { 2 };
                let v6 = match body.get(1) {
                    None if kind != net::RAW && body.len() == 1 => false,
                    Some(2) if body.len() == length => false,
                    Some(&net::op::FAMILY_INET6) if body.len() == length => true,
                    _ => return Response::error(Code::InvalidRequest),
                };
                let protocol = body.get(2).copied().unwrap_or(0);
                // Raw sockets: administrators only, as on Windows, for one
                // protocol (IPPROTO_IP and IPPROTO_RAW, every protocol at
                // once, are not supported).
                if kind == net::RAW && matches!(protocol, 0 | 255) {
                    return Response::error(Code::Unsupported);
                }
                // Every user may open ICMP (IPv4) and ICMPv6 ones, as on
                // Windows; other protocols need an administrator.
                let icmp = matches!((v6, protocol), (false, 1) | (true, 58));
                if kind == net::RAW
                    && !icmp
                    && peer.groups & door_abi::platform::group::ADMINISTRATORS == 0
                {
                    return Response::error(Code::Denied);
                }
                if !matches!(kind, net::TCP | net::UDP | net::RAW) {
                    return Response::error(Code::InvalidRequest);
                }
                if self.quota(channel) >= net::SOCKETS_PER_CLIENT {
                    return Response::error(Code::Resources);
                }
                // UDP binds on BIND or its first send (Winsock's implicit bind).
                let handle = match kind {
                    net::TCP => self.sockets.add(tcp_socket()),
                    net::UDP => self.sockets.add(udp_socket()),
                    _ => self.sockets.add(raw_socket(v6, protocol)),
                };
                let id = self.next_id;
                self.next_id += 1;
                let mut socket = ClientSocket::new(channel, peer.pid, id, kind, handle, v6);
                socket.protocol = protocol;
                self.clients.push(socket);
                Response::scalar(u64::from(id))
            }
            op::CONNECT | net::v6::CONNECT => {
                let parsed = if opcode == op::CONNECT {
                    body.get(4..8).zip(body.get(8..10)).map(|(a, p)| {
                        (
                            IpAddress::Ipv4(ipv4(a.try_into().unwrap())),
                            u16::from_le_bytes([p[0], p[1]]),
                        )
                    })
                } else {
                    body.get(4..)
                        .filter(|b| b.len() == net::v6::ENDPOINT_BYTES)
                        .and_then(net::v6::decode_endpoint)
                        .map(|(a, p)| (from16(a), p))
                };
                let (Some(id), Some((address, port))) = (word(body, 0), parsed) else {
                    return Response::error(Code::InvalidRequest);
                };
                let Some(index) = self.socket(channel, id) else {
                    return Response::error(Code::NotFound);
                };
                // A raw socket's peer has no port.
                if self.clients[index].kind == net::RAW && !address.is_unspecified() {
                    self.clients[index].peer = Some((address, 0));
                    return Response::empty();
                }
                if port == 0 || address.is_unspecified() {
                    return Response::error(Code::InvalidRequest);
                }
                // An outbound connection the firewall blocks: Access denied
                // (WSAEACCES), before anything leaves.
                if self.clients[index].kind == net::TCP
                    && !self.permitted(index, door_firewall::Direction::Out, address, port)
                {
                    return Response::error(Code::Denied);
                }
                if self.clients[index].kind == net::UDP {
                    if let Err(code) = self.bind_udp(index, 0) {
                        return Response::error(code);
                    }
                    self.clients[index].peer = Some((address, port));
                    return Response::empty();
                }
                let local = match self.clients[index].local {
                    0 => match self.free_port(net::TCP, 0) {
                        Ok(p) => p,
                        Err(code) => return Response::error(code),
                    },
                    bound => bound,
                };
                self.clients[index].local = local;
                // The address BIND named, as on Windows; else the tunnel's
                // when a connected VPN routes it; otherwise IPv6 picks its
                // source by the destination.
                let source = self.clients[index].address.or(match address {
                    IpAddress::Ipv4(v4) => self.vpn_source(octets(v4)),
                    IpAddress::Ipv6(v6) => self.vpn_source6(v6.octets()),
                });
                let Some(iface) = self.iface.as_mut() else {
                    return Response::error(Code::Unsupported);
                };
                let handle = self.clients[index].handle;
                match self.sockets.get_mut::<tcp::Socket>(handle).connect(
                    iface.context(),
                    (address, port),
                    smoltcp::wire::IpListenEndpoint {
                        addr: source,
                        port: local,
                    },
                ) {
                    Ok(()) => Response::empty(),
                    Err(_) => Response::error(Code::InvalidRequest),
                }
            }
            op::LISTEN => {
                let (Some(id), Some(port)) = (
                    word(body, 0),
                    body.get(4..6).map(|p| u16::from_le_bytes([p[0], p[1]])),
                ) else {
                    return Response::error(Code::InvalidRequest);
                };
                if !matches!(body.len(), 6 | 7) {
                    return Response::error(Code::InvalidRequest);
                }
                let backlog =
                    usize::from(body.get(6).copied().unwrap_or(1)).clamp(1, net::MAX_BACKLOG);
                let Some(index) = self.socket(channel, id) else {
                    return Response::error(Code::NotFound);
                };
                let s = &self.clients[index];
                if s.kind != net::TCP || !s.pool.is_empty() {
                    return Response::error(Code::InvalidRequest);
                }
                // The bound port, or the one LISTEN names.
                let port = match (s.local, port) {
                    (0, 0) => return Response::error(Code::InvalidRequest),
                    (0, p) => match self.free_port(net::TCP, p) {
                        Ok(p) => p,
                        Err(code) => return Response::error(code),
                    },
                    (bound, _) => bound,
                };
                if self.quota(channel) + backlog > net::SOCKETS_PER_CLIENT {
                    return Response::error(Code::Resources);
                }
                let mut pool = Vec::new();
                for _ in 0..backlog {
                    let mut socket = tcp_socket();
                    let at = smoltcp::wire::IpListenEndpoint {
                        addr: self.clients[index].address,
                        port,
                    };
                    if socket.listen(at).is_err() {
                        for h in pool {
                            self.sockets.remove(h);
                        }
                        return Response::error(Code::InvalidRequest);
                    }
                    pool.push(self.sockets.add(socket));
                }
                let s = &mut self.clients[index];
                s.local = port;
                s.pool = pool;
                self.listening(index);
                Response::empty()
            }
            op::BIND => {
                let (Some(id), Some(port)) = (
                    word(body, 0),
                    body.get(4..6).map(|p| u16::from_le_bytes([p[0], p[1]])),
                ) else {
                    return Response::error(Code::InvalidRequest);
                };
                let Some(index) = self
                    .socket(channel, id)
                    .filter(|_| matches!(body.len(), 6 | 22))
                else {
                    return Response::error(Code::NotFound);
                };
                // The optional local address: one of this host's (the
                // loopback interface's, or the adapter's), or any.
                let address = body
                    .get(6..22)
                    .map(|a| from16(a.try_into().unwrap()))
                    .filter(|a| !a.is_unspecified());
                if let Some(a) = address {
                    if !self.is_own(a) || self.clients[index].local != 0 {
                        return Response::error(Code::InvalidRequest);
                    }
                    self.clients[index].address = Some(a);
                }
                if self.clients[index].kind == net::UDP {
                    return match self.bind_udp(index, port) {
                        Ok(p) => {
                            // A port it names: a UDP server.
                            if port != 0 {
                                self.listening(index);
                            }
                            Response::scalar(u64::from(p))
                        }
                        Err(code) => Response::error(code),
                    };
                }
                // Raw sockets have no ports.
                if self.clients[index].kind == net::RAW {
                    return Response::scalar(0);
                }
                if self.clients[index].local != 0 {
                    return Response::error(Code::InvalidRequest);
                }
                match self.free_port(net::TCP, port) {
                    Ok(p) => {
                        self.clients[index].local = p;
                        Response::scalar(u64::from(p))
                    }
                    Err(code) => Response::error(code),
                }
            }
            op::ACCEPT | net::v6::ACCEPT => {
                let Some(index) = word(body, 0)
                    .filter(|_| body.len() == 4)
                    .and_then(|id| self.socket(channel, id))
                else {
                    return Response::error(Code::NotFound);
                };
                let s = &self.clients[index];
                if s.pool.is_empty() {
                    return Response::error(Code::InvalidRequest);
                }
                let Some(slot) = s
                    .pool
                    .iter()
                    .position(|h| connected(self.sockets.get::<tcp::Socket>(*h)))
                else {
                    return Response::error(Code::WouldBlock);
                };
                let port = s.local;
                // A fresh listener takes the accepted socket's place.
                let mut fresh = tcp_socket();
                let at = smoltcp::wire::IpListenEndpoint {
                    addr: s.address,
                    port,
                };
                if fresh.listen(at).is_err() {
                    return Response::error(Code::Resources);
                }
                let fresh = self.sockets.add(fresh);
                let handle = core::mem::replace(&mut self.clients[index].pool[slot], fresh);
                let remote = self.sockets.get::<tcp::Socket>(handle).remote_endpoint();
                let id = self.next_id;
                self.next_id += 1;
                // An accepted connection has its listener's family and TTL.
                let mut accepted = ClientSocket::new(
                    channel,
                    self.clients[index].pid,
                    id,
                    net::TCP,
                    handle,
                    self.clients[index].v6,
                );
                accepted.local = port;
                accepted.ttl = self.clients[index].ttl;
                self.clients.push(accepted);
                let mut out = [0u8; 4 + net::v6::ENDPOINT_BYTES];
                out[..4].copy_from_slice(&id.to_le_bytes());
                if opcode == net::v6::ACCEPT {
                    let (peer, peer_port) = endpoint16(remote);
                    net::v6::encode_endpoint(peer, peer_port, &mut out[4..]);
                    return Response::bytes(&out);
                }
                let (peer, peer_port) = endpoint(remote);
                out[4..8].copy_from_slice(&peer);
                out[8..10].copy_from_slice(&peer_port.to_le_bytes());
                Response::bytes(&out[..10])
            }
            op::SEND_TO | net::v6::SEND_TO => {
                let header = if opcode == op::SEND_TO {
                    10
                } else {
                    4 + net::v6::ENDPOINT_BYTES
                };
                let parsed = if opcode == op::SEND_TO {
                    body.get(4..8).zip(body.get(8..10)).map(|(a, p)| {
                        (
                            IpAddress::Ipv4(ipv4(a.try_into().unwrap())),
                            u16::from_le_bytes([p[0], p[1]]),
                        )
                    })
                } else {
                    body.get(4..header)
                        .and_then(net::v6::decode_endpoint)
                        .map(|(a, p)| (from16(a), p))
                };
                let (Some(id), Some((address, port))) = (word(body, 0), parsed) else {
                    return Response::error(Code::InvalidRequest);
                };
                let data = &body[header..];
                let Some(index) = self.socket(channel, id) else {
                    return Response::error(Code::NotFound);
                };
                let kind = self.clients[index].kind;
                if kind == net::TCP
                    || data.len() > net::CHUNK
                    || self.clients[index].outbound.len() + data.len() > net::MAX_DATAGRAM
                    || (port == 0 && kind == net::UDP)
                    || address.is_unspecified()
                {
                    return Response::error(Code::InvalidRequest);
                }
                let mut datagram = core::mem::take(&mut self.clients[index].outbound);
                datagram.extend_from_slice(data);
                if kind == net::RAW {
                    return self.send_raw(index, address, &datagram);
                }
                if let Err(code) = self.bind_udp(index, 0) {
                    return Response::error(code);
                }
                if !self.permitted(index, door_firewall::Direction::Out, address, port) {
                    return Response::error(Code::Denied);
                }
                self.contacted(index, address, port);
                let handle = self.clients[index].handle;
                let to = self.udp_to(address, port);
                // Multicast goes out with the multicast TTL; 0 keeps it on
                // this host. The hop limit is the socket's, so what is
                // queued under the other one leaves first.
                let multicast = address.is_multicast();
                let s = &self.clients[index];
                // Multicast by the loopback interface stays on this host.
                let local_only = multicast
                    && match address {
                        IpAddress::Ipv4(_) => s.multicast_interface.to_le_bytes()[0] == 127,
                        IpAddress::Ipv6(_) => s.multicast_interface == net::LOOPBACK_INDEX,
                    };
                let hop = match (multicast, local_only) {
                    (_, true) => 0,
                    (true, false) => s.multicast_ttl,
                    (false, _) => s.ttl,
                };
                let (unicast_ttl, loops) = (s.ttl, multicast);
                let socket = self.sockets.get_mut::<udp::Socket>(handle);
                if socket.send_queue() != 0 && socket.hop_limit() != Some(hop.max(1)) {
                    self.poll();
                }
                let sent = if hop == 0 {
                    Ok(())
                } else {
                    let socket = self.sockets.get_mut::<udp::Socket>(handle);
                    socket.set_hop_limit(Some(hop));
                    socket.send_slice(&datagram, to)
                };
                if multicast && hop != 0 {
                    self.poll();
                    self.sockets
                        .get_mut::<udp::Socket>(handle)
                        .set_hop_limit(Some(unicast_ttl));
                }
                match sent {
                    Ok(()) => {
                        if loops {
                            self.loop_back(index, address, port, &datagram);
                        }
                        Response::scalar(datagram.len() as u64)
                    }
                    Err(_) => {
                        // The staged parts stay: the sender retries the
                        // final part.
                        datagram.truncate(datagram.len() - data.len());
                        self.clients[index].outbound = datagram;
                        Response::error(Code::WouldBlock)
                    }
                }
            }
            op::SEND_PART => {
                let (Some(id), Some(offset)) = (word(body, 0), word(body, 4)) else {
                    return Response::error(Code::InvalidRequest);
                };
                let data = &body[8..];
                let Some(index) = self.socket(channel, id) else {
                    return Response::error(Code::NotFound);
                };
                let s = &mut self.clients[index];
                if offset == 0 {
                    s.outbound.clear();
                }
                if s.kind == net::TCP
                    || data.len() > net::CHUNK
                    || offset as usize != s.outbound.len()
                    || s.outbound.len() + data.len() > net::MAX_DATAGRAM
                {
                    return Response::error(Code::InvalidRequest);
                }
                s.outbound.extend_from_slice(data);
                Response::scalar(s.outbound.len() as u64)
            }
            op::RECEIVE_FROM | net::v6::RECEIVE_FROM => {
                let v6 = opcode == net::v6::RECEIVE_FROM;
                let header = if v6 {
                    net::v6::DATAGRAM_HEADER
                } else {
                    net::DATAGRAM_HEADER
                };
                let (Some(id), Some(max)) = (word(body, 0), word(body, 4)) else {
                    return Response::error(Code::InvalidRequest);
                };
                let Some(index) = self.socket(channel, id) else {
                    return Response::error(Code::NotFound);
                };
                if self.clients[index].kind == net::TCP {
                    return Response::error(Code::InvalidRequest);
                }
                let max = (max as usize).min(net::MAX_DATAGRAM);
                let mut out = [0u8; net::v6::DATAGRAM_HEADER + net::CHUNK];
                match self.next_datagram(index) {
                    Some((from, mut whole)) => {
                        let n = whole.len();
                        if v6 {
                            let (address, port) = endpoint16(Some(from));
                            net::v6::encode_endpoint(address, port, &mut out);
                        } else {
                            let (address, port) = endpoint(Some(from));
                            out[..4].copy_from_slice(&address);
                            out[4..6].copy_from_slice(&port.to_le_bytes());
                        }
                        out[header - 4..header].copy_from_slice(&(n as u32).to_le_bytes());
                        // A datagram larger than the reader's buffer is cut,
                        // as Winsock's WSAEMSGSIZE truncation; the reader
                        // still learns the whole length.
                        whole.truncate(n.min(max));
                        let first = whole.len().min(net::CHUNK);
                        out[header..header + first].copy_from_slice(&whole[..first]);
                        self.clients[index].inbound = whole;
                        Response::bytes(&out[..header + first])
                    }
                    None => {
                        self.clients[index].inbound.clear();
                        Response::bytes(&[])
                    }
                }
            }
            op::RECEIVE_MORE => {
                let (Some(id), Some(offset)) = (word(body, 0), word(body, 4)) else {
                    return Response::error(Code::InvalidRequest);
                };
                let Some(index) = self.socket(channel, id) else {
                    return Response::error(Code::NotFound);
                };
                let staged = &self.clients[index].inbound;
                let offset = offset as usize;
                if offset > staged.len() {
                    return Response::error(Code::InvalidRequest);
                }
                Response::bytes(&staged[offset..staged.len().min(offset + net::CHUNK)])
            }
            op::ENDPOINTS | net::v6::ENDPOINTS => {
                let Some(index) = word(body, 0)
                    .filter(|_| body.len() == 4)
                    .and_then(|id| self.socket(channel, id))
                else {
                    return Response::error(Code::NotFound);
                };
                let s = &self.clients[index];
                let (local, peer) = if s.kind == net::TCP && s.pool.is_empty() {
                    let socket = self.sockets.get::<tcp::Socket>(s.handle);
                    (socket.local_endpoint(), socket.remote_endpoint())
                } else {
                    let peer = s.peer.map(|(a, p)| IpEndpoint::new(a, p));
                    // A bound UDP socket (or listener): the address BIND
                    // named; with a peer, the adapter's address of the
                    // peer's family; else the unspecified one of its own
                    // family, as Windows' getsockname answers.
                    let local = (s.local != 0).then(|| {
                        let address = s.address.unwrap_or_else(|| match peer.map(|p| p.addr) {
                            Some(IpAddress::Ipv6(_)) => IpAddress::Ipv6(
                                self.slaac
                                    .as_ref()
                                    .map(|v6| v6.global.map_or(v6.link_local, |(c, _)| c.address()))
                                    .unwrap_or(Ipv6Address::UNSPECIFIED),
                            ),
                            Some(IpAddress::Ipv4(_)) => IpAddress::Ipv4(ipv4(self.local_address())),
                            None if s.v6 => IpAddress::Ipv6(Ipv6Address::UNSPECIFIED),
                            None => IpAddress::Ipv4(Ipv4Address::UNSPECIFIED),
                        });
                        IpEndpoint::new(address, s.local)
                    });
                    (local, peer)
                };
                let fill_port = |e: Option<IpEndpoint>| {
                    e.map(|mut e| {
                        if e.port == 0 {
                            e.port = s.local;
                        }
                        e
                    })
                };
                let local = fill_port(local).or_else(|| {
                    (s.local != 0).then(|| {
                        IpEndpoint::new(IpAddress::Ipv4(Ipv4Address::UNSPECIFIED), s.local)
                    })
                });
                if opcode == net::v6::ENDPOINTS {
                    let mut out = [0u8; 2 * net::v6::ENDPOINT_BYTES];
                    let (a, p) = endpoint16(local);
                    net::v6::encode_endpoint(a, p, &mut out[..18]);
                    let (a, p) = endpoint16(peer);
                    net::v6::encode_endpoint(a, p, &mut out[18..]);
                    return Response::bytes(&out);
                }
                let (local, local_port) = endpoint(local);
                let local_port = if local_port == 0 { s.local } else { local_port };
                let (peer, peer_port) = endpoint(peer);
                let mut out = [0u8; net::ENDPOINTS_BYTES];
                out[..4].copy_from_slice(&local);
                out[4..6].copy_from_slice(&local_port.to_le_bytes());
                out[6..10].copy_from_slice(&peer);
                out[10..12].copy_from_slice(&peer_port.to_le_bytes());
                Response::bytes(&out)
            }
            op::OPTION => {
                let (Some(id), Some(option), Some(value)) =
                    (word(body, 0), body.get(4).copied(), word(body, 5))
                else {
                    return Response::error(Code::InvalidRequest);
                };
                let Some(index) = self.socket(channel, id).filter(|_| body.len() == 9) else {
                    return Response::error(Code::NotFound);
                };
                let s = &mut self.clients[index];
                match (s.kind, option) {
                    (_, net::option::TTL) => {
                        let Some(ttl) = u8::try_from(value).ok().filter(|t| *t != 0) else {
                            return Response::error(Code::InvalidRequest);
                        };
                        s.ttl = ttl;
                        let (kind, handles) = (s.kind, s.pool.clone());
                        let handle = s.handle;
                        match kind {
                            net::TCP => {
                                for h in core::iter::once(handle).chain(handles) {
                                    self.sockets
                                        .get_mut::<tcp::Socket>(h)
                                        .set_hop_limit(Some(ttl));
                                }
                            }
                            net::UDP => self
                                .sockets
                                .get_mut::<udp::Socket>(handle)
                                .set_hop_limit(Some(ttl)),
                            _ => (),
                        }
                        return Response::empty();
                    }
                    (net::UDP | net::RAW, net::option::MULTICAST_TTL) => {
                        let Ok(ttl) = u8::try_from(value) else {
                            return Response::error(Code::InvalidRequest);
                        };
                        s.multicast_ttl = ttl;
                        return Response::empty();
                    }
                    (net::UDP, net::option::MULTICAST_LOOP) => {
                        s.multicast_loop = value != 0;
                        return Response::empty();
                    }
                    (net::UDP, net::option::MULTICAST_INTERFACE) => {
                        s.multicast_interface = value;
                        return Response::empty();
                    }
                    (net::RAW, net::option::HEADER_INCLUDED) if !s.v6 => {
                        s.header_included = value != 0;
                        return Response::empty();
                    }
                    (
                        net::TCP,
                        net::option::NO_DELAY | net::option::KEEP_ALIVE | net::option::TIMEOUT,
                    ) => (),
                    _ => return Response::error(Code::Unsupported),
                }
                let s = &self.clients[index];
                let duration =
                    (value != 0).then(|| smoltcp::time::Duration::from_millis(u64::from(value)));
                let handles: Vec<SocketHandle> = core::iter::once(s.handle)
                    .chain(s.pool.iter().copied())
                    .collect();
                for h in handles {
                    let socket = self.sockets.get_mut::<tcp::Socket>(h);
                    match option {
                        net::option::NO_DELAY => socket.set_nagle_enabled(value == 0),
                        net::option::KEEP_ALIVE => socket.set_keep_alive(duration),
                        net::option::TIMEOUT => socket.set_timeout(duration),
                        _ => return Response::error(Code::InvalidRequest),
                    }
                }
                Response::empty()
            }
            op::SEND => {
                let Some(id) = word(body, 0) else {
                    return Response::error(Code::InvalidRequest);
                };
                let data = &body[4..];
                if data.len() > net::CHUNK {
                    return Response::error(Code::InvalidRequest);
                }
                let Some(index) = self.socket(channel, id) else {
                    return Response::error(Code::NotFound);
                };
                let s = &self.clients[index];
                if s.kind == net::TCP {
                    let socket = self.sockets.get_mut::<tcp::Socket>(s.handle);
                    if !socket.may_send() {
                        return Response::error(Code::Closing);
                    }
                    match socket.send_slice(data) {
                        Ok(n) => Response::scalar(n as u64),
                        Err(_) => Response::error(Code::Closing),
                    }
                } else {
                    let Some((address, port)) = s.peer else {
                        return Response::error(Code::InvalidRequest);
                    };
                    if s.local == 0 {
                        return Response::error(Code::InvalidRequest);
                    }
                    if !self.permitted(index, door_firewall::Direction::Out, address, port) {
                        return Response::error(Code::Denied);
                    }
                    self.contacted(index, address, port);
                    let s = &self.clients[index];
                    let (handle, to) = (s.handle, self.udp_to(address, port));
                    let socket = self.sockets.get_mut::<udp::Socket>(handle);
                    match socket.send_slice(data, to) {
                        Ok(()) => Response::scalar(data.len() as u64),
                        Err(_) => Response::error(Code::WouldBlock),
                    }
                }
            }
            op::RECEIVE => {
                let (Some(id), Some(max)) = (word(body, 0), word(body, 4)) else {
                    return Response::error(Code::InvalidRequest);
                };
                let Some(index) = self.socket(channel, id) else {
                    return Response::error(Code::NotFound);
                };
                let max = (max as usize).min(net::CHUNK);
                let mut buffer = [0u8; net::CHUNK];
                let s = &mut self.clients[index];
                if s.kind == net::TCP {
                    let socket = self.sockets.get_mut::<tcp::Socket>(s.handle);
                    if matches!(
                        socket.state(),
                        tcp::State::CloseWait
                            | tcp::State::LastAck
                            | tcp::State::Closing
                            | tcp::State::TimeWait
                    ) {
                        s.fin_seen = true;
                    }
                    if !socket.may_recv() && socket.recv_queue() == 0 {
                        // The peer finished; closed without its FIN, the
                        // connection was reset (Io).
                        let reset = socket.state() == tcp::State::Closed && !s.fin_seen;
                        return Response::error(if reset { Code::Io } else { Code::Closing });
                    }
                    match socket.recv_slice(&mut buffer[..max]) {
                        Ok(n) => Response::bytes(&buffer[..n]),
                        Err(_) => Response::error(Code::Closing),
                    }
                } else {
                    let handle = s.handle;
                    // What the firewall refuses is passed over.
                    loop {
                        let socket = self.sockets.get_mut::<udp::Socket>(handle);
                        match socket.recv_slice(&mut buffer[..max]) {
                            Ok((n, meta)) => {
                                let from = meta.endpoint;
                                if self.datagram_permitted(index, from.addr, from.port) {
                                    break Response::bytes(&buffer[..n]);
                                }
                            }
                            Err(_) => break Response::bytes(&[]),
                        }
                    }
                }
            }
            op::SHUTDOWN => {
                let Some(index) = word(body, 0).and_then(|id| self.socket(channel, id)) else {
                    return Response::error(Code::NotFound);
                };
                let s = &self.clients[index];
                if s.kind == net::TCP {
                    self.sockets.get_mut::<tcp::Socket>(s.handle).close();
                }
                Response::empty()
            }
            op::CLOSE => {
                let Some(index) = word(body, 0).and_then(|id| self.socket(channel, id)) else {
                    return Response::error(Code::NotFound);
                };
                self.close_socket(index);
                Response::empty()
            }
            op::STATE => {
                let Some(index) = word(body, 0).and_then(|id| self.socket(channel, id)) else {
                    return Response::error(Code::NotFound);
                };
                let s = &self.clients[index];
                let (state, queued_in, queued_out) = if !s.pool.is_empty() {
                    // A listener: readable while a connection waits for ACCEPT.
                    let waiting = s
                        .pool
                        .iter()
                        .filter(|h| connected(self.sockets.get::<tcp::Socket>(**h)))
                        .count();
                    (SocketState::Listening, waiting, 0)
                } else if s.kind == net::TCP {
                    let socket = self.sockets.get::<tcp::Socket>(s.handle);
                    (state_of(socket), socket.recv_queue(), socket.send_queue())
                } else if s.kind == net::RAW {
                    let socket = self.sockets.get::<raw::Socket>(s.handle);
                    (
                        SocketState::Established,
                        socket.recv_queue(),
                        socket.send_queue(),
                    )
                } else {
                    let socket = self.sockets.get::<udp::Socket>(s.handle);
                    let looped: usize = s.looped.iter().map(|(_, d)| d.len().max(1)).sum();
                    (
                        SocketState::Established,
                        socket.recv_queue() + looped,
                        socket.send_queue(),
                    )
                };
                let mut out = [0u8; 12];
                out[0] = state as u8;
                out[4..8].copy_from_slice(&(queued_in as u32).to_le_bytes());
                out[8..12].copy_from_slice(&(queued_out as u32).to_le_bytes());
                Response::bytes(&out)
            }
            op::MULTICAST => {
                let (Some(id), Some(action), Some(group)) = (
                    word(body, 0),
                    body.get(4).copied(),
                    body.get(5..21)
                        .and_then(|g| <[u8; 16]>::try_from(g).ok())
                        .map(from16),
                ) else {
                    return Response::error(Code::InvalidRequest);
                };
                let Some(index) = self
                    .socket(channel, id)
                    .filter(|_| body.len() == op::MULTICAST_BYTES)
                else {
                    return Response::error(Code::NotFound);
                };
                match self.multicast(index, action, group) {
                    Ok(()) => Response::empty(),
                    Err(code) => Response::error(code),
                }
            }
            o if net::vpn::is_vpn(o) => self.vpn_request(channel, peer, opcode, body),
            o if (net::firewall::STATE..=net::firewall::ALERT).contains(&o) => {
                self.firewall_request(channel, peer, opcode, body)
            }
            _ => Response::error(Code::InvalidRequest),
        }
    }
    fn disconnected(&mut self, channel: Handle) {
        while let Some(index) = self.clients.iter().position(|s| s.channel == channel) {
            self.close_socket(index);
        }
        self.queries.retain(|q| q.channel != channel);
        self.vpn.forget_channel(channel);
        self.pings.forget(channel);
    }
}

pub fn run(startup: StartupV2) -> i64 {
    let result = (|| -> Result<(), SysStatus> {
        let mut server = Server::new(&startup)?;
        let handle = startup.base.inherited[inherited::AUTHORITY].handle;
        let info = door_sys::net_info(handle);
        let (nic, mac) = match info {
            Ok(info) => (Some(handle), info.mac),
            Err(_) => (None, [0; 6]),
        };
        door_rt::diagnostic(format_args!(
            "DOOR_P15 NET ADAPTER present={} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x} link={}\n",
            u8::from(nic.is_some()),
            mac[0],
            mac[1],
            mac[2],
            mac[3],
            mac[4],
            mac[5],
            u8::from(info.is_ok_and(|i| i.link))
        ));
        let mut network = Network::new(nic, mac);
        server.ready(manifest::NETWORK)?;
        while server.poll(&mut network)? {
            if network.poll() {
                server.note_progress();
            }
            server.idle_backoff()?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(status) => {
            door_rt::diagnostic(format_args!(
                "DOOR_P15 NET FAILED status={status:?}
"
            ));
            -151
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const A: Handle = 0x10;
    const B: Handle = 0x20;
    fn network() -> Network {
        Network::new(None, [0x02, 0, 0, 0, 0, 1])
    }
    fn call(n: &mut Network, channel: Handle, opcode: u32, body: &[u8]) -> Response {
        n.request(channel, Identity::default(), opcode, body, &[])
    }
    fn scalar(r: &Response) -> u64 {
        assert_eq!(r.code, Code::Success);
        u64::from_le_bytes(r.body[..8].try_into().unwrap())
    }
    fn socket(n: &mut Network, channel: Handle, kind: u8) -> u32 {
        scalar(&call(n, channel, op::SOCKET, &[kind])) as u32
    }
    fn bind(n: &mut Network, channel: Handle, id: u32, port: u16) -> Response {
        let mut body = id.to_le_bytes().to_vec();
        body.extend(port.to_le_bytes());
        call(n, channel, op::BIND, &body)
    }

    #[test]
    fn sockets_respect_the_client_quota_and_close_with_their_channel() {
        let mut n = network();
        for _ in 0..net::SOCKETS_PER_CLIENT {
            socket(&mut n, A, net::UDP);
        }
        assert_eq!(
            call(&mut n, A, op::SOCKET, &[net::TCP]).code,
            Code::Resources
        );
        // Another client has its own quota.
        let other = socket(&mut n, B, net::TCP);
        assert_eq!(call(&mut n, A, op::SOCKET, &[3]).code, Code::InvalidRequest);
        n.disconnected(A);
        assert_eq!(n.quota(A), 0);
        socket(&mut n, A, net::TCP);
        assert!(n.socket(B, other).is_some());
    }

    #[test]
    fn a_client_reaches_only_its_own_sockets() {
        let mut n = network();
        let id = socket(&mut n, A, net::UDP);
        assert_eq!(
            call(&mut n, B, op::CLOSE, &id.to_le_bytes()).code,
            Code::NotFound
        );
        assert_eq!(
            call(&mut n, B, op::STATE, &id.to_le_bytes()).code,
            Code::NotFound
        );
        assert_eq!(
            call(&mut n, A, op::CLOSE, &id.to_le_bytes()).code,
            Code::Success
        );
        assert_eq!(
            call(&mut n, A, op::CLOSE, &id.to_le_bytes()).code,
            Code::NotFound
        );
    }

    #[test]
    fn binding_takes_free_ports_per_protocol() {
        let mut n = network();
        let (first, second, tcp) = (
            socket(&mut n, A, net::UDP),
            socket(&mut n, B, net::UDP),
            socket(&mut n, A, net::TCP),
        );
        assert_eq!(scalar(&bind(&mut n, A, first, 5000)), 5000);
        // The port is taken for UDP, by any client; TCP has its own ports.
        assert_eq!(bind(&mut n, B, second, 5000).code, Code::Exists);
        assert_eq!(scalar(&bind(&mut n, A, tcp, 5000)), 5000);
        // A bound UDP socket keeps its port; an ephemeral one is 49152 and up.
        assert_eq!(bind(&mut n, A, first, 5001).code, Code::InvalidRequest);
        assert_eq!(scalar(&bind(&mut n, A, first, 0)), 5000);
        assert!(scalar(&bind(&mut n, B, second, 0)) >= 49152);
        assert_eq!(bind(&mut n, A, tcp, 5002).code, Code::InvalidRequest);
        assert_eq!(
            call(&mut n, A, op::BIND, &first.to_le_bytes()).code,
            Code::InvalidRequest
        );
    }

    #[test]
    fn datagram_parts_are_staged_in_order_and_bounded() {
        let mut n = network();
        let udp = socket(&mut n, A, net::UDP);
        let tcp = socket(&mut n, A, net::TCP);
        let part = |id: u32, offset: u32, data: &[u8]| {
            let mut body = id.to_le_bytes().to_vec();
            body.extend(offset.to_le_bytes());
            body.extend(data);
            body
        };
        let chunk = [7u8; net::CHUNK];
        assert_eq!(
            scalar(&call(&mut n, A, op::SEND_PART, &part(udp, 0, &chunk))),
            net::CHUNK as u64
        );
        // Parts must continue where the staging ends.
        assert_eq!(
            call(&mut n, A, op::SEND_PART, &part(udp, 5, &chunk)).code,
            Code::InvalidRequest
        );
        assert_eq!(
            call(&mut n, A, op::SEND_PART, &part(tcp, 0, &chunk)).code,
            Code::InvalidRequest
        );
        let mut at = net::CHUNK;
        while at + net::CHUNK <= net::MAX_DATAGRAM {
            scalar(&call(
                &mut n,
                A,
                op::SEND_PART,
                &part(udp, at as u32, &chunk),
            ));
            at += net::CHUNK;
        }
        // No datagram grows past MAX_DATAGRAM.
        assert_eq!(
            call(&mut n, A, op::SEND_PART, &part(udp, at as u32, &chunk)).code,
            Code::InvalidRequest
        );
        // Offset 0 starts over.
        assert_eq!(
            scalar(&call(&mut n, A, op::SEND_PART, &part(udp, 0, &[1, 2, 3]))),
            3
        );
        // Nothing received: RECEIVE_FROM is empty and RECEIVE_MORE has nothing
        // past offset 0.
        let mut receive = udp.to_le_bytes().to_vec();
        receive.extend(4096u32.to_le_bytes());
        let r = call(&mut n, A, op::RECEIVE_FROM, &receive);
        assert_eq!((r.code, r.len), (Code::Success, 0));
        let mut more = udp.to_le_bytes().to_vec();
        more.extend(0u32.to_le_bytes());
        assert_eq!(call(&mut n, A, op::RECEIVE_MORE, &more).len, 0);
        more[4..].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(
            call(&mut n, A, op::RECEIVE_MORE, &more).code,
            Code::InvalidRequest
        );
        assert_eq!(
            call(&mut n, A, op::RECEIVE_FROM, &{
                let mut b = tcp.to_le_bytes().to_vec();
                b.extend(64u32.to_le_bytes());
                b
            })
            .code,
            Code::InvalidRequest
        );
    }

    #[test]
    fn the_connection_table_lists_sockets_with_their_owner_and_pings_need_an_address() {
        let mut n = network();
        let peer = Identity {
            pid: 42,
            ..Identity::default()
        };
        let tcp = scalar(&n.request(1, peer, op::SOCKET, &[net::TCP], &[])) as u32;
        let mut body = [0u8; 6];
        body[..4].copy_from_slice(&tcp.to_le_bytes());
        body[4..6].copy_from_slice(&8080u16.to_le_bytes());
        assert_eq!(
            n.request(1, peer, op::LISTEN, &body, &[]).code,
            Code::Success
        );
        let udp = scalar(&n.request(1, peer, op::SOCKET, &[net::UDP], &[])) as u32;
        assert_eq!(bind(&mut n, 1, udp, 5353).code, Code::Success);
        let r = call(&mut n, 2, net::diag::CONNECTIONS, &0u16.to_le_bytes());
        assert_eq!(r.code, Code::Success);
        let data = &r.body[..r.len];
        assert_eq!(u16::from_le_bytes([data[0], data[1]]), 2);
        let tcp_row = net::Connection::decode(&data[4..24]).unwrap();
        assert_eq!(
            (
                tcp_row.protocol,
                tcp_row.state,
                tcp_row.local_port,
                tcp_row.pid
            ),
            (6, net::tcp_state::LISTEN, 8080, 42)
        );
        let udp_row = net::Connection::decode(&data[24..44]).unwrap();
        assert_eq!((udp_row.protocol, udp_row.local_port), (17, 5353));
        // AF_INET6 sockets: only the IPv6 table lists them.
        let tcp6 =
            scalar(&n.request(1, peer, op::SOCKET, &[net::TCP, net::op::FAMILY_INET6], &[])) as u32;
        let mut body = [0u8; 6];
        body[..4].copy_from_slice(&tcp6.to_le_bytes());
        body[4..6].copy_from_slice(&8443u16.to_le_bytes());
        assert_eq!(
            n.request(1, peer, op::LISTEN, &body, &[]).code,
            Code::Success
        );
        assert_eq!(
            n.request(1, peer, op::SOCKET, &[net::UDP, 99], &[]).code,
            Code::InvalidRequest
        );
        let r = call(&mut n, 2, net::diag::CONNECTIONS, &0u16.to_le_bytes());
        assert_eq!(u16::from_le_bytes([r.body[0], r.body[1]]), 2);
        let r = call(&mut n, 2, net::diag::CONNECTIONS6, &0u16.to_le_bytes());
        assert_eq!(r.code, Code::Success);
        let data = &r.body[..r.len];
        assert_eq!(u16::from_le_bytes([data[0], data[1]]), 1);
        let row = net::Connection6::decode(&data[4..4 + net::CONNECTION6_BYTES]).unwrap();
        assert_eq!(
            (row.protocol, row.state, row.local, row.local_port, row.pid),
            (6, net::tcp_state::LISTEN, [0; 16], 8443, 42)
        );
        // Without an address the echo fails at once, as Windows' does.
        let request = net::EchoRequest {
            address: [10, 0, 2, 2],
            ttl: 64,
            dont_fragment: false,
            size: 32,
            timeout: 1000,
            pattern: [0; net::PATTERN_BYTES],
            pattern_len: 0,
        };
        let mut b = [0u8; 12 + net::PATTERN_BYTES];
        let len = request.encode(&mut b);
        let token = scalar(&call(&mut n, 3, net::diag::PING, &b[..len])) as u32;
        // Only its client may collect it.
        assert_eq!(
            call(&mut n, 4, net::diag::PING_RESULT, &token.to_le_bytes()).code,
            Code::NotFound
        );
        let r = call(&mut n, 3, net::diag::PING_RESULT, &token.to_le_bytes());
        let echo = net::Echo::decode(&r.body[..r.len]).unwrap();
        assert_eq!(echo.status, net::ip_status::GENERAL_FAILURE);
        assert!(net::EchoRequest::decode(&b[..11]).is_none());
    }
    #[test]
    fn only_settings_configures_the_adapter() {
        let mut n = network();
        let body = Config::default().encode();
        assert_eq!(call(&mut n, A, op::CONFIGURE, &body).code, Code::Denied);
        assert_eq!(call(&mut n, A, op::STATUS, &[1]).code, Code::InvalidRequest);
        assert_eq!(call(&mut n, A, 0x6FFF, &[]).code, Code::InvalidRequest);
    }

    #[test]
    fn statistics_and_neighbors_fit_the_reply_size() {
        use door_service_protocol::BODY_BYTES;
        let mut n = network();
        let mut all = Vec::new();
        let mut offset = 0;
        while offset < net::stats::COUNTERS_BYTES {
            let r = call(
                &mut n,
                A,
                net::stats::STATISTICS,
                &(offset as u16).to_le_bytes(),
            );
            assert_eq!(r.code, Code::Success);
            assert!(r.len <= BODY_BYTES && r.len > 0);
            all.extend_from_slice(&r.body[..r.len]);
            offset += r.len;
        }
        assert_eq!(
            net::stats::Counters::decode(&all),
            Some(net::stats::Counters::default())
        );
        let past = (net::stats::COUNTERS_BYTES as u16).to_le_bytes();
        assert_eq!(
            call(&mut n, A, net::stats::STATISTICS, &past).code,
            Code::InvalidRequest
        );
        assert_eq!(
            call(&mut n, A, net::stats::STATISTICS, &[]).code,
            Code::InvalidRequest
        );
        // The subnet's broadcast entries once the adapter has an address.
        n.address = Some(Ipv4Cidr::new(ipv4([10, 0, 2, 15]), 24));
        let r = call(&mut n, A, net::stats::NEIGHBORS, &0u16.to_le_bytes());
        assert_eq!(r.code, Code::Success);
        assert!(r.len <= BODY_BYTES);
        let data = &r.body[..r.len];
        assert_eq!(u16::from_le_bytes([data[0], data[1]]), 2);
        let first = net::stats::Neighbor::decode(&data[4..28]).unwrap();
        assert_eq!(first.ipv4(), Some([10, 0, 2, 255]));
        assert_eq!(first.kind, net::stats::NEIGHBOR_STATIC);
        const { assert!(4 + net::stats::PAGE * net::stats::NEIGHBOR_BYTES <= BODY_BYTES) };
    }

    fn administrator() -> Identity {
        Identity {
            groups: door_abi::platform::group::ADMINISTRATORS,
            ..Identity::default()
        }
    }
    fn membership(n: &mut Network, channel: Handle, id: u32, action: u8, group: [u8; 16]) -> Code {
        let mut body = id.to_le_bytes().to_vec();
        body.push(action);
        body.extend(group);
        body.extend(0u32.to_le_bytes());
        call(n, channel, op::MULTICAST, &body).code
    }
    fn option(n: &mut Network, channel: Handle, id: u32, option: u8, value: u32) -> Code {
        let mut body = id.to_le_bytes().to_vec();
        body.push(option);
        body.extend(value.to_le_bytes());
        call(n, channel, op::OPTION, &body).code
    }
    fn send_to(
        n: &mut Network,
        channel: Handle,
        id: u32,
        to: [u8; 4],
        port: u16,
        data: &[u8],
    ) -> Response {
        let mut body = id.to_le_bytes().to_vec();
        body.extend(to);
        body.extend(port.to_le_bytes());
        body.extend(data);
        call(n, channel, op::SEND_TO, &body)
    }
    /// One datagram: its sender's port and bytes.
    fn receive_from(n: &mut Network, channel: Handle, id: u32) -> Option<(u16, Vec<u8>)> {
        let mut body = id.to_le_bytes().to_vec();
        body.extend(64u32.to_le_bytes());
        let r = call(n, channel, op::RECEIVE_FROM, &body);
        assert_eq!(r.code, Code::Success);
        (r.len != 0).then(|| {
            (
                u16::from_le_bytes([r.body[4], r.body[5]]),
                r.body[net::DATAGRAM_HEADER..r.len].to_vec(),
            )
        })
    }

    #[test]
    fn raw_sockets_are_for_administrators_and_have_no_ports() {
        let mut n = network();
        // ICMP and ICMPv6 for every user (as measured on Windows), other
        // protocols for administrators.
        let igmp = [net::RAW, 2, 2];
        assert_eq!(call(&mut n, A, op::SOCKET, &igmp).code, Code::Denied);
        scalar(&n.request(A, administrator(), op::SOCKET, &igmp, &[]));
        scalar(&call(
            &mut n,
            A,
            op::SOCKET,
            &[net::RAW, op::FAMILY_INET6, 58],
        ));
        assert_eq!(
            call(&mut n, A, op::SOCKET, &[net::RAW, op::FAMILY_INET6, 1]).code,
            Code::Denied
        );
        let icmp = [net::RAW, 2, 1];
        let id = scalar(&call(&mut n, A, op::SOCKET, &icmp)) as u32;
        // IPPROTO_IP and IPPROTO_RAW, every protocol at once, are not
        // supported; the protocol byte is required.
        for body in [&[net::RAW, 2, 255][..], &[net::RAW, 2, 0], &[net::RAW, 2]] {
            assert_ne!(
                n.request(A, administrator(), op::SOCKET, body, &[]).code,
                Code::Success
            );
        }
        assert_eq!(scalar(&bind(&mut n, A, id, 0)), 0);
        // No address yet: nothing to send from.
        assert_eq!(
            send_to(&mut n, A, id, [10, 0, 2, 2], 0, &[8, 0, 0, 0]).code,
            Code::Unsupported
        );
        // Only UDP sockets join groups; only raw IPv4 takes IP_HDRINCL.
        let group = net::v6::mapped([239, 255, 77, 1]);
        assert_eq!(
            membership(&mut n, A, id, op::MULTICAST_JOIN, group),
            Code::InvalidRequest
        );
        assert_eq!(
            option(&mut n, A, id, net::option::HEADER_INCLUDED, 1),
            Code::Success
        );
        let udp = socket(&mut n, A, net::UDP);
        assert_eq!(
            option(&mut n, A, udp, net::option::HEADER_INCLUDED, 1),
            Code::Unsupported
        );
    }

    #[test]
    fn this_hosts_multicast_reaches_the_sockets_that_joined_the_group() {
        let mut n = network();
        let group = [239, 255, 77, 1];
        let mapped = net::v6::mapped(group);
        let joined = socket(&mut n, A, net::UDP);
        assert_eq!(scalar(&bind(&mut n, A, joined, 5000)), 5000);
        let elsewhere = socket(&mut n, B, net::UDP);
        assert_eq!(scalar(&bind(&mut n, B, elsewhere, 5001)), 5001);
        let sender = socket(&mut n, B, net::UDP);
        let join = op::MULTICAST_JOIN;
        assert_eq!(membership(&mut n, A, joined, join, mapped), Code::Success);
        assert_eq!(membership(&mut n, A, joined, join, mapped), Code::Exists);
        // Not a group; an AF_INET socket joins no IPv6 group.
        let unicast = net::v6::mapped([10, 0, 0, 1]);
        assert_eq!(
            membership(&mut n, A, joined, join, unicast),
            Code::InvalidRequest
        );
        let mut v6_group = [0u8; 16];
        v6_group[..2].copy_from_slice(&[0xff, 0x15]);
        v6_group[15] = 0x77;
        assert_eq!(
            membership(&mut n, A, joined, join, v6_group),
            Code::InvalidRequest
        );
        assert_eq!(
            membership(&mut n, B, elsewhere, join, mapped),
            Code::Success
        );
        // Sent here, delivered here (the network never returns it), to the
        // socket on the group's port only.
        assert_eq!(
            scalar(&send_to(&mut n, B, sender, group, 5000, b"hello")),
            5
        );
        let from = n.clients[n.socket(B, sender).unwrap()].local;
        assert_eq!(
            receive_from(&mut n, A, joined),
            Some((from, b"hello".to_vec()))
        );
        assert_eq!(receive_from(&mut n, A, joined), None);
        assert_eq!(receive_from(&mut n, B, elsewhere), None);
        // STATE reports the looped datagram as readable.
        scalar(&send_to(&mut n, B, sender, group, 5000, b"again"));
        let state = call(&mut n, A, op::STATE, &joined.to_le_bytes());
        assert_eq!(&state.body[4..8], &5u32.to_le_bytes());
        assert!(receive_from(&mut n, A, joined).is_some());
        // IP_MULTICAST_LOOP off on the receiver keeps this host's own out.
        let looping = net::option::MULTICAST_LOOP;
        assert_eq!(option(&mut n, A, joined, looping, 0), Code::Success);
        scalar(&send_to(&mut n, B, sender, group, 5000, b"quiet"));
        assert_eq!(receive_from(&mut n, A, joined), None);
        assert_eq!(option(&mut n, A, joined, looping, 1), Code::Success);
        // A group left (twice is NotFound) no longer arrives.
        let leave = op::MULTICAST_LEAVE;
        assert_eq!(membership(&mut n, A, joined, leave, mapped), Code::Success);
        assert_eq!(membership(&mut n, A, joined, leave, mapped), Code::NotFound);
        scalar(&send_to(&mut n, B, sender, group, 5000, b"gone"));
        assert_eq!(receive_from(&mut n, A, joined), None);
        // A socket holds a bounded number of them. Multicast TTL 0 keeps
        // them on this host (nothing queues for the network).
        let ttl = net::option::MULTICAST_TTL;
        assert_eq!(option(&mut n, B, sender, ttl, 0), Code::Success);
        for _ in 0..net::LOOPED_DATAGRAMS + 4 {
            scalar(&send_to(&mut n, B, sender, group, 5001, b"x"));
        }
        let index = n.socket(B, elsewhere).unwrap();
        assert_eq!(n.clients[index].looped.len(), net::LOOPED_DATAGRAMS);
    }

    #[test]
    fn ttls_default_to_windows_and_are_checked() {
        let mut n = network();
        let tcp = socket(&mut n, A, net::TCP);
        let udp = socket(&mut n, A, net::UDP);
        let handle = n.clients[n.socket(A, tcp).unwrap()].handle;
        assert_eq!(
            n.sockets.get::<tcp::Socket>(handle).hop_limit(),
            Some(net::DEFAULT_TTL)
        );
        let ttl = net::option::TTL;
        assert_eq!(option(&mut n, A, udp, ttl, 64), Code::Success);
        let s = &n.clients[n.socket(A, udp).unwrap()];
        assert_eq!((s.ttl, s.multicast_ttl, s.multicast_loop), (64, 1, true));
        assert_eq!(n.sockets.get::<udp::Socket>(s.handle).hop_limit(), Some(64));
        assert_eq!(option(&mut n, A, udp, ttl, 0), Code::InvalidRequest);
        assert_eq!(option(&mut n, A, udp, ttl, 256), Code::InvalidRequest);
        assert_eq!(option(&mut n, A, tcp, ttl, 32), Code::Success);
        assert_eq!(n.sockets.get::<tcp::Socket>(handle).hop_limit(), Some(32));
        // Multicast TTL 0 keeps datagrams on this host.
        let multicast_ttl = net::option::MULTICAST_TTL;
        assert_eq!(option(&mut n, A, udp, multicast_ttl, 0), Code::Success);
        assert_eq!(option(&mut n, A, tcp, multicast_ttl, 4), Code::Unsupported);
    }

    /// TCP through the stack to `to` (an IPv4 address, or IPv6 with v6
    /// requests): connect, accept, data both ways.
    fn tcp_round_trip(n: &mut Network, to: [u8; 16], port: u16) {
        let listener = socket(n, A, net::TCP);
        let mut body = listener.to_le_bytes().to_vec();
        body.extend(port.to_le_bytes());
        assert_eq!(call(n, A, op::LISTEN, &body).code, Code::Success);
        let client = socket(n, B, net::TCP);
        let mut body = client.to_le_bytes().to_vec();
        let opcode = match net::v6::unmapped(&to) {
            Some(v4) => {
                body.extend(v4);
                body.extend(port.to_le_bytes());
                op::CONNECT
            }
            None => {
                let mut endpoint = [0u8; net::v6::ENDPOINT_BYTES];
                net::v6::encode_endpoint(to, port, &mut endpoint);
                body.extend(endpoint);
                net::v6::CONNECT
            }
        };
        assert_eq!(call(n, B, opcode, &body).code, Code::Success);
        let mut accepted = None;
        for _ in 0..8 {
            n.poll();
            let r = call(n, A, op::ACCEPT, &listener.to_le_bytes());
            if r.code == Code::Success {
                accepted = Some(u32::from_le_bytes(r.body[..4].try_into().unwrap()));
                break;
            }
        }
        let accepted = accepted.unwrap_or_else(|| panic!("not accepted: {to:?}"));
        let mut body = client.to_le_bytes().to_vec();
        body.extend(b"ping");
        assert_eq!(scalar(&call(n, B, op::SEND, &body)), 4);
        let mut got: Vec<u8> = Vec::new();
        for _ in 0..8 {
            n.poll();
            let mut body = accepted.to_le_bytes().to_vec();
            body.extend(64u32.to_le_bytes());
            let r = call(n, A, op::RECEIVE, &body);
            got.extend(&r.body[..r.len]);
            if got.len() >= 4 {
                break;
            }
        }
        assert_eq!(got, b"ping");
    }

    #[test]
    fn the_loopback_interface_carries_tcp_and_udp_without_an_adapter() {
        let mut n = network();
        // No adapter: the interface runs with the loopback addresses only.
        assert!(!n.status().present);
        tcp_round_trip(&mut n, net::v6::mapped([127, 0, 0, 1]), 7000);
        // Any 127.0.0.0/8 address, and ::1.
        tcp_round_trip(&mut n, net::v6::mapped([127, 1, 2, 3]), 7001);
        let mut one = [0u8; 16];
        one[15] = 1;
        tcp_round_trip(&mut n, one, 7002);
        // UDP: a datagram to 127.0.0.1 arrives from 127.0.0.1.
        let receiver = socket(&mut n, A, net::UDP);
        assert_eq!(scalar(&bind(&mut n, A, receiver, 7003)), 7003);
        let sender = socket(&mut n, B, net::UDP);
        scalar(&send_to(
            &mut n,
            B,
            sender,
            [127, 0, 0, 1],
            7003,
            b"datagram",
        ));
        n.poll();
        let mut body = receiver.to_le_bytes().to_vec();
        body.extend(64u32.to_le_bytes());
        let r = call(&mut n, A, op::RECEIVE_FROM, &body);
        assert_eq!(&r.body[..4], &[127, 0, 0, 1]);
        assert_eq!(&r.body[net::DATAGRAM_HEADER..r.len], b"datagram");
        // Nothing of it reached the adapter's counters or neighbour table.
        let nic = n.nic.as_ref().unwrap();
        assert_eq!(nic.frames_out.get() + nic.frames_in.get(), 0);
        assert!(n.neighbors().is_empty());
    }

    #[test]
    fn a_socket_bound_to_an_address_answers_only_there() {
        let mut n = network();
        let listener = socket(&mut n, A, net::TCP);
        let mut body = listener.to_le_bytes().to_vec();
        body.extend(7100u16.to_le_bytes());
        body.extend(net::v6::mapped([127, 0, 0, 1]));
        assert_eq!(scalar(&call(&mut n, A, op::BIND, &body)), 7100);
        // Not one of this host's addresses.
        let other = socket(&mut n, A, net::UDP);
        let mut body = other.to_le_bytes().to_vec();
        body.extend(7101u16.to_le_bytes());
        body.extend(net::v6::mapped([10, 9, 9, 9]));
        assert_eq!(call(&mut n, A, op::BIND, &body).code, Code::InvalidRequest);
        let mut body = listener.to_le_bytes().to_vec();
        body.extend(0u16.to_le_bytes());
        assert_eq!(call(&mut n, A, op::LISTEN, &body).code, Code::Success);
        // getsockname: the bound address.
        let r = call(&mut n, A, op::ENDPOINTS, &listener.to_le_bytes());
        assert_eq!(&r.body[..6], &[127, 0, 0, 1, 0xbc, 0x1b]);
        // 127.0.0.2 is this host too, but not the listener's address.
        let client = socket(&mut n, B, net::TCP);
        let mut body = client.to_le_bytes().to_vec();
        body.extend([127, 0, 0, 2]);
        body.extend(7100u16.to_le_bytes());
        assert_eq!(call(&mut n, B, op::CONNECT, &body).code, Code::Success);
        for _ in 0..8 {
            n.poll();
        }
        assert_eq!(
            call(&mut n, A, op::ACCEPT, &listener.to_le_bytes()).code,
            Code::WouldBlock
        );
        let state = call(&mut n, B, op::STATE, &client.to_le_bytes());
        assert_ne!(state.body[0], SocketState::Established as u8);
        // IPv6: getsockname answers ::1 when bound there, :: when bound to
        // any, as Windows does.
        let mut one = [0u8; 16];
        one[15] = 1;
        for (address, want) in [(Some(one), one), (None, [0u8; 16])] {
            let id = scalar(&call(&mut n, A, op::SOCKET, &[net::UDP, op::FAMILY_INET6])) as u32;
            let mut body = id.to_le_bytes().to_vec();
            body.extend(0u16.to_le_bytes());
            if let Some(a) = address {
                body.extend(a);
            }
            scalar(&call(&mut n, A, op::BIND, &body));
            let r = call(&mut n, A, net::v6::ENDPOINTS, &id.to_le_bytes());
            assert_eq!(&r.body[..16], &want);
        }
    }

    #[test]
    fn the_internet_checksum_spans_parts_and_odd_lengths() {
        // RFC 1071's example words, split unevenly.
        let whole = [0x00, 0x01, 0xf2, 0x03, 0xf4, 0xf5, 0xf6, 0xf7];
        assert_eq!(internet_checksum(&[&whole]), !0xddf2);
        assert_eq!(internet_checksum(&[&whole[..3], &whole[3..]]), !0xddf2);
        assert_eq!(internet_checksum(&[&[0x01]]), !0x0100);
        // IP_HDRINCL: length, identification and source completed.
        let mut header = [0u8; 24];
        header[0] = 0x45;
        header[8] = 64;
        header[9] = 1;
        let (me, to) = ([10, 0, 2, 15], [10, 0, 2, 2]);
        let p = included_header(&header, 1, me, to, 7).unwrap();
        assert_eq!(&p[2..4], &[0, 24]);
        assert_eq!(&p[4..6], &[0, 7]);
        assert_eq!(&p[12..16], &me);
        assert_eq!(&p[16..20], &to);
        assert!(included_header(&header, 17, me, to, 7).is_none());
        // A forged source, or a destination other than the one sent to.
        let mut forged = header;
        forged[12..16].copy_from_slice(&[10, 0, 2, 99]);
        assert!(included_header(&forged, 1, me, to, 7).is_none());
        let mut elsewhere = header;
        elsewhere[16..20].copy_from_slice(&[192, 0, 2, 1]);
        assert!(included_header(&elsewhere, 1, me, to, 7).is_none());
        header[0] = 0x46;
        assert!(included_header(&header, 1, me, to, 7).is_none());
    }
}
