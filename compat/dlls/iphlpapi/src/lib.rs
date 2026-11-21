#![no_std]
//! Original IP Helper (iphlpapi.dll): the adapter, interface, address and
//! route views Windows programs read, built from door.network's status
//! (one VirtIO Ethernet adapter) plus the software loopback interface, in
//! the exact x64 layouts of the Windows SDK. IPv4, and IPv6 addresses the
//! stack autoconfigured (link-local, router-advertised prefix, RDNSS).
//!
//! This module is the host-testable part: the interface model and the
//! byte builders. `guest` holds the exports.
//!
//! The same sources build the x64 and the x86 DLL (P17): Windows-facing
//! exports take pointer-width parameters, widened at entry; a pointer or
//! handle result is narrowed from the body's 64-bit value (an immediately
//! called closure, so the body's own `return`s keep their meaning).
#![allow(clippy::redundant_closure_call)]
extern crate alloc;
use alloc::{string::String, vec, vec::Vec};
use door_service_protocol::net::{Status, v6::Status6};
use door_windows_abi::layout::{self, Field, Field::*};

/// ::1
pub const LOOPBACK6: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];

#[cfg(all(feature = "guest-pe", target_os = "windows", not(test)))]
mod guest;

pub const ERROR_SUCCESS: u32 = 0;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
pub const ERROR_NOT_SUPPORTED: u32 = 50;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
pub const ERROR_BUFFER_OVERFLOW: u32 = 111;
pub const ERROR_NO_DATA: u32 = 232;
pub const ERROR_NETWORK_UNREACHABLE: u32 = 1231;
pub const ERROR_NOT_FOUND: u32 = 1168;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_GEN_FAILURE: u32 = 31;

pub const AF_UNSPEC: u16 = 0;
pub const AF_INET: u16 = 2;
pub const AF_INET6: u16 = 23;

/// IANA ifType values the model uses.
pub const IF_TYPE_ETHERNET_CSMACD: u32 = 6;
pub const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;

/// `GetAdaptersAddresses` flags.
pub mod gaa {
    pub const SKIP_UNICAST: u32 = 0x1;
    pub const SKIP_ANYCAST: u32 = 0x2;
    pub const SKIP_MULTICAST: u32 = 0x4;
    pub const SKIP_DNS_SERVER: u32 = 0x8;
    pub const INCLUDE_PREFIX: u32 = 0x10;
    pub const SKIP_FRIENDLY_NAME: u32 = 0x20;
    pub const INCLUDE_GATEWAYS: u32 = 0x80;
}

/// Traffic counters of one interface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub frames_in: u64,
    pub frames_out: u64,
}

/// One interface as every view reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    pub index: u32,
    pub if_type: u32,
    /// NET_LUID's NetLuidIndex (the `ethernet_32768` in an interface name).
    pub luid_index: u32,
    pub guid: [u8; 16],
    pub alias: String,
    pub description: String,
    pub mac: Option<[u8; 6]>,
    pub mtu: u32,
    /// Link speed in bits per second; `u64::MAX` while disconnected.
    pub speed: u64,
    /// Operationally up (link, enabled, and for Ethernet an address).
    pub up: bool,
    /// The cable/link is present (MediaConnectState).
    pub connected: bool,
    pub enabled: bool,
    pub dhcp: bool,
    pub ipv4: Option<([u8; 4], u8)>,
    pub gateway: Option<[u8; 4]>,
    pub dns: Vec<[u8; 4]>,
    /// IPv6 addresses: (address, prefix length, IpPrefixOrigin).
    pub ipv6: Vec<([u8; 16], u8, u32)>,
    /// The IPv6 default router (link-local) and RDNSS servers.
    pub gateway6: Option<[u8; 16]>,
    pub dns6: Vec<[u8; 16]>,
    pub dhcp_server: Option<[u8; 4]>,
    /// DHCP lease seconds remaining, when known.
    pub lease: Option<u32>,
    pub metric: u32,
    pub counters: Counters,
}

pub const ETHERNET_INDEX: u32 = 2;
/// A VPN tunnel's adapter (WireGuard's: IF_TYPE_PROP_VIRTUAL).
pub const TUNNEL_INDEX: u32 = 3;
pub const IF_TYPE_PROP_VIRTUAL: u32 = 53;

/// A running VPN tunnel as its own adapter, before the loopback interface
/// (WireGuard's: no physical address, no DHCP, its configured address and
/// DNS servers, routes on-link through it).
pub fn add_tunnel(list: &mut Vec<Interface>, t: &door_service_protocol::net::vpn::Tunnel) {
    let mut seed = [0u8; 6];
    for (i, b) in t.name().bytes().enumerate() {
        seed[i % 6] ^= b;
    }
    let at = list
        .iter()
        .position(Interface::loopback)
        .unwrap_or(list.len());
    list.insert(
        at,
        Interface {
            index: TUNNEL_INDEX,
            if_type: IF_TYPE_PROP_VIRTUAL,
            luid_index: 32768,
            guid: adapter_guid(seed, 0x57),
            alias: String::from(t.name()),
            description: String::from("WireGuard Tunnel"),
            mac: None,
            mtu: u32::from(t.mtu),
            speed: 10_000_000_000,
            up: true,
            connected: true,
            enabled: true,
            dhcp: false,
            ipv4: Some((t.address, t.prefix)),
            gateway: None,
            dns: t.dns.iter().copied().filter(|d| *d != [0; 4]).collect(),
            ipv6: Vec::new(),
            gateway6: None,
            dns6: Vec::new(),
            dhcp_server: None,
            lease: None,
            metric: 5,
            counters: Counters {
                bytes_in: t.rx_bytes,
                bytes_out: t.tx_bytes,
                frames_in: 0,
                frames_out: 0,
            },
        },
    );
}
pub const LOOPBACK_INDEX: u32 = 1;

impl Interface {
    /// NET_LUID: IfType in bits 48..63, NetLuidIndex in bits 24..47.
    pub fn luid(&self) -> u64 {
        (u64::from(self.if_type) << 48) | (u64::from(self.luid_index) << 24)
    }
    pub fn loopback(&self) -> bool {
        self.if_type == IF_TYPE_SOFTWARE_LOOPBACK
    }
    /// `ConvertInterfaceLuidToName`: `ethernet_32768`, `loopback_0`.
    pub fn name(&self) -> String {
        let kind = match self.if_type {
            IF_TYPE_ETHERNET_CSMACD => "ethernet",
            IF_TYPE_SOFTWARE_LOOPBACK => "loopback",
            _ => "other",
        };
        alloc::format!("{kind}_{}", self.luid_index)
    }
    /// `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}` (the AdapterName).
    pub fn guid_text(&self) -> String {
        guid_text(&self.guid)
    }
    pub fn mask(&self) -> [u8; 4] {
        mask(self.ipv4.map_or(0, |(_, p)| p))
    }
}

pub fn mask(prefix: u8) -> [u8; 4] {
    if prefix == 0 {
        [0; 4]
    } else {
        (u32::MAX << (32 - u32::from(prefix.min(32)))).to_be_bytes()
    }
}
/// GUID text from its in-memory bytes (Data1..3 little-endian).
pub fn guid_text(g: &[u8; 16]) -> String {
    let d1 = u32::from_le_bytes([g[0], g[1], g[2], g[3]]);
    let d2 = u16::from_le_bytes([g[4], g[5]]);
    let d3 = u16::from_le_bytes([g[6], g[7]]);
    alloc::format!(
        "{{{d1:08X}-{d2:04X}-{d3:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g[8],
        g[9],
        g[10],
        g[11],
        g[12],
        g[13],
        g[14],
        g[15]
    )
}
/// A stable adapter GUID: fixed Data1..3, the MAC in the node bytes.
fn adapter_guid(node: [u8; 6], seed: u8) -> [u8; 16] {
    let mut g = [
        0x2e,
        0x54,
        0xd0,
        0x6d,
        0x0d,
        0x0e,
        0x4d,
        0x4f,
        0x80 | seed,
        0x00,
        0,
        0,
        0,
        0,
        0,
        0,
    ];
    g[10..].copy_from_slice(&node);
    g
}

/// The interfaces Doors reports, in Windows' order: physical adapters
/// first, the software loopback last.
pub fn interfaces(status: Option<&Status>, status6: Option<&Status6>) -> Vec<Interface> {
    let mut list = Vec::new();
    if let Some(s) = status.filter(|s| s.present) {
        let connected = s.link && s.enabled;
        let ipv4 = (connected && s.address != [0; 4]).then_some((s.address, s.prefix));
        // IpPrefixOriginRouterAdvertisement (4) for the autoconfigured
        // address, WellKnown (2) for link-local.
        let v6 = status6.filter(|_| connected);
        let mut ipv6 = Vec::new();
        if let Some(v6) = v6 {
            if v6.global != [0; 16] {
                ipv6.push((v6.global, v6.prefix, 4));
            }
            if v6.link_local != [0; 16] {
                ipv6.push((v6.link_local, 64, 2));
            }
        }
        let dhcp = s.config.fixed.is_none();
        list.push(Interface {
            index: ETHERNET_INDEX,
            if_type: IF_TYPE_ETHERNET_CSMACD,
            luid_index: 32768,
            guid: adapter_guid(s.mac, 0x01),
            alias: String::from("Ethernet"),
            description: String::from("Red Hat VirtIO Ethernet Adapter"),
            mac: Some(s.mac),
            mtu: 1500,
            // NetKVM reports 10 Gbps; Windows reports a disconnected
            // adapter's speed as all ones.
            speed: if connected { 10_000_000_000 } else { u64::MAX },
            connected,
            enabled: s.enabled,
            dhcp,
            ipv4,
            gateway: (ipv4.is_some() && s.gateway != [0; 4]).then_some(s.gateway),
            dns: if ipv4.is_some() {
                s.dns.iter().copied().filter(|d| *d != [0; 4]).collect()
            } else {
                Vec::new()
            },
            gateway6: v6.map(|v| v.router).filter(|r| *r != [0; 16]),
            dns6: v6.map_or(Vec::new(), |v| {
                v.dns.iter().copied().filter(|d| *d != [0; 16]).collect()
            }),
            up: ipv4.is_some(),
            ipv6,
            dhcp_server: (dhcp && ipv4.is_some() && s.gateway != [0; 4]).then_some(s.gateway),
            lease: (dhcp && s.lease_seconds != 0).then_some(s.lease_seconds),
            // Automatic metric: 15 for 2-20 Gbps; 5 for a disconnected link.
            metric: if connected { 15 } else { 5 },
            counters: Counters {
                bytes_in: s.bytes_in,
                bytes_out: s.bytes_out,
                frames_in: s.frames_in,
                frames_out: s.frames_out,
            },
        });
    }
    list.push(Interface {
        index: LOOPBACK_INDEX,
        if_type: IF_TYPE_SOFTWARE_LOOPBACK,
        luid_index: 0,
        guid: adapter_guid(*b"\x80nonic", 0x3e),
        alias: String::from("Loopback Pseudo-Interface 1"),
        description: String::from("Software Loopback Interface 1"),
        mac: None,
        mtu: u32::MAX,
        speed: 1_073_741_824,
        up: true,
        connected: true,
        enabled: true,
        dhcp: false,
        ipv4: Some(([127, 0, 0, 1], 8)),
        gateway: None,
        dns: Vec::new(),
        ipv6: vec![(LOOPBACK6, 128, 2)],
        gateway6: None,
        dns6: Vec::new(),
        dhcp_server: None,
        lease: None,
        metric: 75,
        counters: Counters::default(),
    });
    list
}

// ---- byte building ----------------------------------------------------
/// A buffer that will live at application address `base`: offsets become
/// pointers with [`Blob::pointer`].
pub struct Blob {
    pub base: u64,
    pub bytes: Vec<u8>,
    /// The program's pointer width (P17): 8, or 4 for a 32-bit program.
    pub width: usize,
}
impl Blob {
    pub fn new(base: u64) -> Self {
        Self::at_width(base, layout::POINTER)
    }
    pub fn at_width(base: u64, width: usize) -> Self {
        Self {
            base,
            bytes: Vec::new(),
            width,
        }
    }
    /// Reserves `size` zeroed bytes aligned to `align`; their offset.
    pub fn reserve(&mut self, size: usize, align: usize) -> usize {
        let at = self.bytes.len().next_multiple_of(align);
        self.bytes.resize(at + size, 0);
        at
    }
    pub fn pointer(&self, offset: usize) -> u64 {
        self.base + offset as u64
    }
    pub fn u8(&mut self, at: usize, v: u8) {
        self.bytes[at] = v;
    }
    pub fn u16(&mut self, at: usize, v: u16) {
        self.bytes[at..at + 2].copy_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, at: usize, v: u32) {
        self.bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, at: usize, v: u64) {
        self.bytes[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
    pub fn put(&mut self, at: usize, data: &[u8]) {
        self.bytes[at..at + data.len()].copy_from_slice(data);
    }
    /// Stores a pointer to `target` at `at` (at the program's width).
    pub fn link(&mut self, at: usize, target: usize) {
        let p = self.pointer(target);
        let width = self.width;
        self.bytes[at..at + width].copy_from_slice(&p.to_le_bytes()[..width]);
    }
    /// A NUL-terminated UTF-16 string; its offset.
    pub fn wide(&mut self, text: &str) -> usize {
        let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
        let at = self.reserve(units.len() * 2, 2);
        for (i, u) in units.iter().enumerate() {
            self.u16(at + i * 2, *u);
        }
        at
    }
    /// A NUL-terminated byte string; its offset.
    pub fn narrow(&mut self, text: &str) -> usize {
        let at = self.reserve(text.len() + 1, 1);
        self.put(at, text.as_bytes());
        at
    }
    /// A 16-byte sockaddr_in; its offset.
    pub fn sockaddr(&mut self, address: [u8; 4]) -> usize {
        let at = self.reserve(16, 8);
        self.put(at, &sockaddr_in(address));
        at
    }
    /// SOCKET_ADDRESS { lpSockaddr, iSockaddrLength } at `at`.
    pub fn socket_address(&mut self, at: usize, address: [u8; 4]) {
        let sa = self.sockaddr(address);
        self.link(at, sa);
        self.u32(at + self.width, 16);
    }
    /// SOCKET_ADDRESS of a 28-byte SOCKADDR_IN6 (link-local addresses carry
    /// the interface as their scope id).
    pub fn socket_address6(&mut self, at: usize, address: [u8; 16], scope: u32) {
        let sa = self.reserve(28, 8);
        self.put(sa, &sockaddr_in6(address, scope));
        self.link(at, sa);
        self.u32(at + self.width, 28);
    }
}
/// SOCKADDR_IN6 (28 bytes); `scope` applies to link-local addresses.
pub fn sockaddr_in6(address: [u8; 16], scope: u32) -> [u8; 28] {
    let mut b = [0u8; 28];
    b[..2].copy_from_slice(&AF_INET6.to_le_bytes());
    b[8..24].copy_from_slice(&address);
    if address[0] == 0xfe && address[1] & 0xc0 == 0x80 {
        b[24..28].copy_from_slice(&scope.to_le_bytes());
    }
    b
}
pub fn sockaddr_in(address: [u8; 4]) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[..2].copy_from_slice(&AF_INET.to_le_bytes());
    b[4..8].copy_from_slice(&address);
    b
}
/// SOCKADDR_INET (28 bytes) holding an IPv4 address.
pub fn sockaddr_inet(address: [u8; 4]) -> [u8; 28] {
    let mut b = [0u8; 28];
    b[..16].copy_from_slice(&sockaddr_in(address));
    b
}
fn broadcast(address: [u8; 4], prefix: u8) -> [u8; 4] {
    let m = u32::from_be_bytes(mask(prefix));
    (u32::from_be_bytes(address) | !m).to_be_bytes()
}
fn network(address: [u8; 4], prefix: u8) -> [u8; 4] {
    (u32::from_be_bytes(address) & u32::from_be_bytes(mask(prefix))).to_be_bytes()
}
/// The on-link prefixes Windows lists for an IPv4 address: the network,
/// the host, the subnet broadcast, multicast and the limited broadcast.
pub fn prefixes(address: [u8; 4], prefix: u8) -> [([u8; 4], u8); 5] {
    [
        (network(address, prefix), prefix),
        (address, 32),
        (broadcast(address, prefix), 32),
        ([224, 0, 0, 0], 4),
        ([255, 255, 255, 255], 32),
    ]
}

/// IP_ADAPTER_ADDRESSES_LH (448 bytes on x64, 376 on x86). These lists
/// open with a union of their Length and Flags (or IfIndex) with a
/// ULONGLONG, spelled as that U64 (aligning them to 8 on x86 too) and an
/// empty slot that keeps the field numbers; Length and Flags are written
/// at +0 and +4. Each
/// SOCKET_ADDRESS member is spelled as two pointer-sized slots: lpSockaddr,
/// then iSockaddrLength with its padding.
pub const ADAPTER_ADDRESSES: [Field; 39] = [
    U64,
    Bytes(0),
    Pointer,
    Pointer,
    Pointer,
    Pointer,
    Pointer,
    Pointer,
    Pointer,
    Pointer,
    Pointer,
    Bytes(8),
    U32,
    U32,
    U32,
    U32,
    U32,
    U32,
    Bytes(64),
    Pointer,
    U64,
    U64,
    Pointer,
    Pointer,
    U32,
    U32,
    U64,
    Pointer,
    Pointer,
    U32,
    Bytes(16),
    U32,
    U32,
    Pointer,
    Pointer,
    Bytes(130),
    U32,
    U32,
    Pointer,
];
/// Field indexes of `ADAPTER_ADDRESSES`.
pub mod aa {
    pub const NEXT: usize = 2;
    pub const NAME: usize = 3;
    pub const UNICAST: usize = 4;
    pub const DNS: usize = 7;
    pub const SUFFIX: usize = 8;
    pub const DESCRIPTION: usize = 9;
    pub const FRIENDLY: usize = 10;
    pub const PHYSICAL: usize = 11;
    pub const PHYSICAL_LENGTH: usize = 12;
    pub const FLAGS: usize = 13;
    pub const MTU: usize = 14;
    pub const IF_TYPE: usize = 15;
    pub const OPER_STATUS: usize = 16;
    pub const IPV6_INDEX: usize = 17;
    pub const ZONES: usize = 18;
    pub const PREFIX: usize = 19;
    pub const TRANSMIT: usize = 20;
    pub const RECEIVE: usize = 21;
    pub const GATEWAY: usize = 23;
    pub const IPV4_METRIC: usize = 24;
    pub const LUID: usize = 26;
    pub const DHCPV4: usize = 27;
    pub const COMPARTMENT: usize = 29;
    pub const GUID: usize = 30;
    pub const CONNECTION: usize = 31;
}
/// IP_ADAPTER_UNICAST_ADDRESS_LH: Length, Flags, Next, Address (two
/// slots), PrefixOrigin, SuffixOrigin, DadState, ValidLifetime,
/// PreferredLifetime, LeaseLifetime, OnLinkPrefixLength (64 bytes on x64,
/// 48 on x86).
pub const UNICAST_ADDRESS: [Field; 12] = [
    U64,
    Bytes(0),
    Pointer,
    Pointer,
    Pointer,
    U32,
    U32,
    U32,
    U32,
    U32,
    U32,
    U8,
];
/// IP_ADAPTER_DNS_SERVER_ADDRESS_XP and IP_ADAPTER_GATEWAY_ADDRESS_LH:
/// Length, Reserved, Next, Address (32 bytes on x64, 24 on x86).
pub const SERVER_ADDRESS: [Field; 5] = [U64, Bytes(0), Pointer, Pointer, Pointer];
/// IP_ADAPTER_PREFIX_XP: Length, Flags, Next, Address, PrefixLength (40
/// bytes on x64, 24 on x86).
pub const ADAPTER_PREFIX: [Field; 6] = [U64, Bytes(0), Pointer, Pointer, Pointer, U32];

/// `IP_ADAPTER_ADDRESSES_LH` list for an application buffer at `base`;
/// `ERROR_NO_DATA` when no interface carries `family`.
pub fn adapters_addresses(
    base: u64,
    family: u16,
    flags: u32,
    list: &[Interface],
) -> Result<Vec<u8>, u32> {
    adapters_addresses_at(base, family, flags, list, layout::POINTER)
}
/// `adapters_addresses` for a program of pointer width `width` (P17).
pub fn adapters_addresses_at(
    base: u64,
    family: u16,
    flags: u32,
    list: &[Interface],
    width: usize,
) -> Result<Vec<u8>, u32> {
    if !matches!(family, AF_UNSPEC | AF_INET | AF_INET6) {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let want4 = family != AF_INET6;
    let want6 = family != AF_INET;
    if list.is_empty() {
        return Err(ERROR_NO_DATA);
    }
    let a = |at: usize, index: usize| at + layout::offset_at(&ADAPTER_ADDRESSES, index, width);
    let ua = |at: usize, index: usize| at + layout::offset_at(&UNICAST_ADDRESS, index, width);
    let sa = |at: usize, index: usize| at + layout::offset_at(&SERVER_ADDRESS, index, width);
    let pa = |at: usize, index: usize| at + layout::offset_at(&ADAPTER_PREFIX, index, width);
    let size = layout::size_at(&ADAPTER_ADDRESSES, width);
    let unicast_size = layout::size_at(&UNICAST_ADDRESS, width);
    let server_size = layout::size_at(&SERVER_ADDRESS, width);
    let prefix_size = layout::size_at(&ADAPTER_PREFIX, width);
    let mut b = Blob::at_width(base, width);
    let heads: Vec<usize> = list.iter().map(|_| b.reserve(size, 8)).collect();
    for (i, (interface, &at)) in list.iter().zip(&heads).enumerate() {
        b.u32(at, size as u32);
        b.u32(at + 4, interface.index);
        if let Some(next) = heads.get(i + 1) {
            b.link(a(at, aa::NEXT), *next);
        }
        let name = b.narrow(&interface.guid_text());
        b.link(a(at, aa::NAME), name);
        // Unicast addresses: IPv6 ones first, then IPv4, as Windows lists them.
        let mut previous_unicast: Option<usize> = None;
        if flags & gaa::SKIP_UNICAST == 0 && want6 {
            for &(address, prefix, origin) in &interface.ipv6 {
                let u = b.reserve(unicast_size, 8);
                b.u32(u, unicast_size as u32);
                b.u32(u + 4, u32::from(origin != 2 || interface.loopback()));
                b.socket_address6(ua(u, 3), address, interface.index);
                b.u32(ua(u, 5), origin);
                // IpSuffixOriginLinkLayerAddress (4) for EUI-64 identifiers.
                b.u32(ua(u, 6), if interface.loopback() { 2 } else { 4 });
                b.u32(ua(u, 7), 4); // IpDadStatePreferred
                b.u32(ua(u, 8), u32::MAX);
                b.u32(ua(u, 9), u32::MAX);
                b.u32(ua(u, 10), u32::MAX);
                b.u8(ua(u, 11), prefix);
                match previous_unicast {
                    Some(p) => b.link(ua(p, 2), u),
                    None => b.link(a(at, aa::UNICAST), u),
                }
                previous_unicast = Some(u);
            }
        }
        if flags & gaa::SKIP_UNICAST == 0
            && want4
            && let Some((address, prefix)) = interface.ipv4
        {
            let u = b.reserve(unicast_size, 8);
            b.u32(u, unicast_size as u32);
            let (origin, dns_eligible) = if interface.loopback() {
                (2, 0)
            } else if interface.dhcp {
                (3, 1)
            } else {
                (1, 1)
            };
            b.u32(u + 4, dns_eligible);
            b.socket_address(ua(u, 3), address);
            b.u32(ua(u, 5), origin);
            b.u32(ua(u, 6), origin);
            b.u32(ua(u, 7), 4); // IpDadStatePreferred
            let lifetime = interface.lease.unwrap_or(u32::MAX);
            b.u32(ua(u, 8), lifetime);
            b.u32(ua(u, 9), lifetime);
            b.u32(ua(u, 10), interface.lease.unwrap_or(u32::MAX));
            b.u8(ua(u, 11), prefix);
            match previous_unicast {
                Some(p) => b.link(ua(p, 2), u),
                None => b.link(a(at, aa::UNICAST), u),
            }
        }
        // DNS servers.
        if flags & gaa::SKIP_DNS_SERVER == 0 {
            let mut previous: Option<usize> = None;
            for server in interface.dns.iter().filter(|_| want4) {
                let d = b.reserve(server_size, 8);
                b.u32(d, server_size as u32);
                b.socket_address(sa(d, 3), *server);
                match previous {
                    Some(p) => b.link(sa(p, 2), d),
                    None => b.link(a(at, aa::DNS), d),
                }
                previous = Some(d);
            }
            for server in interface.dns6.iter().filter(|_| want6) {
                let d = b.reserve(server_size, 8);
                b.u32(d, server_size as u32);
                b.socket_address6(sa(d, 3), *server, interface.index);
                match previous {
                    Some(p) => b.link(sa(p, 2), d),
                    None => b.link(a(at, aa::DNS), d),
                }
                previous = Some(d);
            }
        }
        let suffix = b.wide("");
        b.link(a(at, aa::SUFFIX), suffix);
        let description = b.wide(&interface.description);
        b.link(a(at, aa::DESCRIPTION), description);
        let friendly = b.wide(if flags & gaa::SKIP_FRIENDLY_NAME == 0 {
            &interface.alias
        } else {
            ""
        });
        b.link(a(at, aa::FRIENDLY), friendly);
        if let Some(mac) = interface.mac {
            b.put(a(at, aa::PHYSICAL), &mac);
            b.u32(a(at, aa::PHYSICAL_LENGTH), 6);
        }
        // IP_ADAPTER_DDNS_ENABLED | DHCP_ENABLED | NETBIOS_OVER_TCPIP_ENABLED
        // | IPV4_ENABLED, as this PC's adapters report (loopback: DDNS | IPv4).
        let adapter_flags = if interface.loopback() {
            0x81
        } else {
            0xc1 | if interface.dhcp { 0x4 } else { 0 }
        };
        // IP_ADAPTER_IPV6_ENABLED (0x100) with IPv6 bound to the adapter (the
        // reference PC's loopback reports 0x81 beside its ::1).
        let adapter_flags = adapter_flags
            | if interface.ipv6.is_empty() || interface.loopback() {
                0
            } else {
                0x100
            };
        b.u32(a(at, aa::FLAGS), adapter_flags);
        b.u32(a(at, aa::MTU), interface.mtu);
        b.u32(a(at, aa::IF_TYPE), interface.if_type);
        // IfOperStatusUp / Down
        b.u32(a(at, aa::OPER_STATUS), if interface.up { 1 } else { 2 });
        if !interface.ipv6.is_empty() {
            b.u32(a(at, aa::IPV6_INDEX), interface.index);
        }
        // ZoneIndices: interface and link scopes are the interface, the
        // wider scopes the default zone 1, ScopeLevelGlobal's slot 0.
        for zone in 0..16 {
            let value = match zone {
                0..=3 => interface.index,
                14 => 0,
                _ => 1,
            };
            b.u32(a(at, aa::ZONES) + zone * 4, value);
        }
        if flags & gaa::INCLUDE_PREFIX != 0
            && let Some((address, prefix)) = interface.ipv4
        {
            let mut previous: Option<usize> = None;
            for (network, length) in prefixes(address, prefix) {
                let p = b.reserve(prefix_size, 8);
                b.u32(p, prefix_size as u32);
                b.socket_address(pa(p, 3), network);
                b.u32(pa(p, 5), u32::from(length));
                match previous {
                    Some(q) => b.link(pa(q, 2), p),
                    None => b.link(a(at, aa::PREFIX), p),
                }
                previous = Some(p);
            }
        }
        b.u64(a(at, aa::TRANSMIT), interface.speed);
        b.u64(a(at, aa::RECEIVE), interface.speed);
        if flags & gaa::INCLUDE_GATEWAYS != 0 {
            let mut previous: Option<usize> = None;
            if let (true, Some(gateway)) = (want6, interface.gateway6) {
                let g = b.reserve(server_size, 8);
                b.u32(g, server_size as u32);
                b.socket_address6(sa(g, 3), gateway, interface.index);
                b.link(a(at, aa::GATEWAY), g);
                previous = Some(g);
            }
            if let (true, Some(gateway)) = (want4, interface.gateway) {
                let g = b.reserve(server_size, 8);
                b.u32(g, server_size as u32);
                b.socket_address(sa(g, 3), gateway);
                match previous {
                    Some(p) => b.link(sa(p, 2), g),
                    None => b.link(a(at, aa::GATEWAY), g),
                }
            }
        }
        b.u32(a(at, aa::IPV4_METRIC), interface.metric);
        b.u64(a(at, aa::LUID), interface.luid());
        if let Some(server) = interface.dhcp_server {
            b.socket_address(a(at, aa::DHCPV4), server);
        }
        b.u32(a(at, aa::COMPARTMENT), 1); // CompartmentId
        if interface.up {
            b.put(a(at, aa::GUID), &interface.guid);
        }
        b.u32(a(at, aa::CONNECTION), 1); // NET_IF_CONNECTION_DEDICATED
    }
    Ok(b.bytes)
}

/// Dotted text in a 16-byte IP_ADDRESS_STRING.
fn address_string(address: [u8; 4]) -> [u8; 16] {
    let text = alloc::format!(
        "{}.{}.{}.{}",
        address[0],
        address[1],
        address[2],
        address[3]
    );
    let mut b = [0u8; 16];
    b[..text.len()].copy_from_slice(text.as_bytes());
    b
}
/// IP_ADDR_STRING: Next, IpAddress, IpMask, Context (48 bytes on x64, 40
/// on x86; the DWORD Context is spelled as a pointer-sized slot, its
/// padding included).
pub const ADDR_STRING: [Field; 4] = [Pointer, Bytes(16), Bytes(16), Pointer];
/// IP_ADDR_STRING { Next, IpAddress, IpMask, Context } at `at`.
fn addr_string(b: &mut Blob, at: usize, address: [u8; 4], mask: [u8; 4], context: u32) {
    let w = b.width;
    b.put(
        at + layout::offset_at(&ADDR_STRING, 1, w),
        &address_string(address),
    );
    b.put(
        at + layout::offset_at(&ADDR_STRING, 2, w),
        &address_string(mask),
    );
    b.u32(at + layout::offset_at(&ADDR_STRING, 3, w), context);
}
/// IP_ADAPTER_INFO: Next, ComboIndex, AdapterName[260], Description[132],
/// AddressLength, Address[8], Index, Type, DhcpEnabled, CurrentIpAddress,
/// IpAddressList, GatewayList, DhcpServer, HaveWins, PrimaryWinsServer,
/// SecondaryWinsServer, LeaseObtained, LeaseExpires (704 bytes on x64,
/// 648 on x86).
pub const ADAPTER_INFO: [Field; 33] = [
    Pointer,
    U32,
    Bytes(260),
    Bytes(132),
    U32,
    Bytes(8),
    U32,
    U32,
    U32,
    Pointer,
    // IpAddressList, GatewayList, DhcpServer
    Pointer,
    Bytes(16),
    Bytes(16),
    Pointer,
    Pointer,
    Bytes(16),
    Bytes(16),
    Pointer,
    Pointer,
    Bytes(16),
    Bytes(16),
    Pointer,
    U32,
    // PrimaryWinsServer, SecondaryWinsServer
    Pointer,
    Bytes(16),
    Bytes(16),
    Pointer,
    Pointer,
    Bytes(16),
    Bytes(16),
    Pointer,
    U64,
    U64,
];
/// Field indexes of `ADAPTER_INFO`.
pub mod ai {
    pub const COMBO_INDEX: usize = 1;
    pub const NAME: usize = 2;
    pub const DESCRIPTION: usize = 3;
    pub const ADDRESS_LENGTH: usize = 4;
    pub const ADDRESS: usize = 5;
    pub const INDEX: usize = 6;
    pub const TYPE: usize = 7;
    pub const DHCP: usize = 8;
    pub const ADDRESSES: usize = 10;
    pub const GATEWAYS: usize = 14;
    pub const DHCP_SERVER: usize = 18;
    pub const PRIMARY_WINS: usize = 23;
    pub const SECONDARY_WINS: usize = 27;
}
/// `IP_ADAPTER_INFO` list: every adapter except the loopback.
pub fn adapters_info(base: u64, list: &[Interface]) -> Result<Vec<u8>, u32> {
    adapters_info_at(base, list, layout::POINTER)
}
/// `adapters_info` for a program of pointer width `width` (P17).
pub fn adapters_info_at(base: u64, list: &[Interface], width: usize) -> Result<Vec<u8>, u32> {
    let adapters: Vec<&Interface> = list.iter().filter(|i| !i.loopback()).collect();
    if adapters.is_empty() {
        return Err(ERROR_NO_DATA);
    }
    let f = |at: usize, index: usize| at + layout::offset_at(&ADAPTER_INFO, index, width);
    let mut b = Blob::at_width(base, width);
    let size = layout::size_at(&ADAPTER_INFO, width);
    let heads: Vec<usize> = adapters.iter().map(|_| b.reserve(size, 8)).collect();
    for (i, (interface, &at)) in adapters.iter().zip(&heads).enumerate() {
        if let Some(next) = heads.get(i + 1) {
            b.link(at, *next);
        }
        b.u32(f(at, ai::COMBO_INDEX), interface.index);
        let name = interface.guid_text();
        b.put(f(at, ai::NAME), name.as_bytes());
        let description = interface.description.as_bytes();
        b.put(
            f(at, ai::DESCRIPTION),
            &description[..description.len().min(131)],
        );
        if let Some(mac) = interface.mac {
            b.u32(f(at, ai::ADDRESS_LENGTH), 6);
            b.put(f(at, ai::ADDRESS), &mac);
        }
        b.u32(f(at, ai::INDEX), interface.index);
        // MIB_IF_TYPE_ETHERNET (6), a tunnel 53
        b.u32(f(at, ai::TYPE), interface.if_type);
        b.u32(f(at, ai::DHCP), u32::from(interface.dhcp));
        let (address, mask) = interface
            .ipv4
            .map_or(([0; 4], [0; 4]), |(a, _)| (a, interface.mask()));
        addr_string(&mut b, f(at, ai::ADDRESSES), address, mask, interface.index);
        addr_string(
            &mut b,
            f(at, ai::GATEWAYS),
            interface.gateway.unwrap_or([0; 4]),
            [255; 4],
            0,
        );
        addr_string(
            &mut b,
            f(at, ai::DHCP_SERVER),
            interface.dhcp_server.unwrap_or([0; 4]),
            [255; 4],
            0,
        );
        addr_string(&mut b, f(at, ai::PRIMARY_WINS), [0; 4], [0; 4], 0);
        addr_string(&mut b, f(at, ai::SECONDARY_WINS), [0; 4], [0; 4], 0);
    }
    Ok(b.bytes)
}

/// FIXED_INFO: HostName[132], DomainName[132], CurrentDnsServer,
/// DnsServerList, NodeType, ScopeId[260], EnableRouting, EnableProxy,
/// EnableDns (600 bytes on x64, 584 on x86).
pub const FIXED_INFO: [Field; 12] = [
    Bytes(132),
    Bytes(132),
    Pointer,
    Pointer,
    Bytes(16),
    Bytes(16),
    Pointer,
    U32,
    Bytes(260),
    U32,
    U32,
    U32,
];
/// `FIXED_INFO` and the DNS server list after it.
pub fn fixed_info(base: u64, host: &str, dns: &[[u8; 4]]) -> Vec<u8> {
    fixed_info_at(base, host, dns, layout::POINTER)
}
/// `fixed_info` for a program of pointer width `width` (P17).
pub fn fixed_info_at(base: u64, host: &str, dns: &[[u8; 4]], width: usize) -> Vec<u8> {
    let f = |index: usize| layout::offset_at(&FIXED_INFO, index, width);
    let mut b = Blob::at_width(base, width);
    let at = b.reserve(layout::size_at(&FIXED_INFO, width), 8);
    let host = host.as_bytes();
    b.put(at, &host[..host.len().min(131)]);
    // The first server is inline; the rest follow the structure.
    let mut previous = at + f(3);
    if let Some(first) = dns.first() {
        addr_string(&mut b, previous, *first, [0; 4], 0);
        b.link(at + f(2), previous);
    }
    for server in dns.iter().skip(1) {
        let next = b.reserve(layout::size_at(&ADDR_STRING, width), 8);
        addr_string(&mut b, next, *server, [0; 4], 0);
        b.link(previous, next);
        previous = next;
    }
    b.u32(at + f(7), 8); // HYBRID_NODETYPE, as ipconfig /all reports
    b.u32(at + f(11), 1); // EnableDns
    b.bytes
}

/// MIB_IF_ROW2 (1352 bytes).
pub fn if_row2(interface: &Interface) -> Vec<u8> {
    let mut b = Blob::new(0);
    b.reserve(1352, 8);
    b.u64(0, interface.luid());
    b.u32(8, interface.index);
    b.put(12, &interface.guid);
    for (i, u) in interface.alias.encode_utf16().take(256).enumerate() {
        b.u16(28 + i * 2, u);
    }
    for (i, u) in interface.description.encode_utf16().take(256).enumerate() {
        b.u16(542 + i * 2, u);
    }
    if let Some(mac) = interface.mac {
        b.u32(1056, 6);
        b.put(1060, &mac);
        b.put(1092, &mac);
    }
    b.u32(1124, interface.mtu);
    b.u32(1128, interface.if_type);
    if interface.loopback() {
        b.u32(1144, 1); // NET_IF_ACCESS_LOOPBACK
    } else {
        b.u32(1140, 14); // NdisPhysicalMedium802_3
        b.u32(1144, 2); // NET_IF_ACCESS_BROADCAST
        // HardwareInterface | ConnectorPresent | NotMediaConnected.
        b.u8(1152, 0x1 | 0x4 | if interface.connected { 0 } else { 0x10 });
    }
    b.u32(1156, if interface.up { 1 } else { 2 });
    b.u32(1160, if interface.enabled { 1 } else { 2 });
    b.u32(1164, if interface.connected { 1 } else { 2 });
    if interface.up {
        b.put(1168, &interface.guid);
    }
    b.u32(1184, 1);
    b.u64(1192, interface.speed);
    b.u64(1200, interface.speed);
    let c = interface.counters;
    b.u64(1208, c.bytes_in);
    b.u64(1216, c.frames_in);
    b.u64(1256, c.bytes_in);
    b.u64(1280, c.bytes_out);
    b.u64(1288, c.frames_out);
    b.u64(1320, c.bytes_out);
    b.bytes
}
/// MIB_IF_TABLE2: NumEntries, padding, rows.
pub fn if_table2(list: &[Interface]) -> Vec<u8> {
    let mut b = vec![0u8; 8];
    b[..4].copy_from_slice(&(list.len() as u32).to_le_bytes());
    for interface in list {
        b.extend(if_row2(interface));
    }
    b
}
/// MIB_IFROW (860 bytes).
pub fn if_row(interface: &Interface) -> Vec<u8> {
    let mut b = Blob::new(0);
    b.reserve(860, 4);
    let name = alloc::format!("\\DEVICE\\TCPIP_{}", interface.guid_text());
    for (i, u) in name.encode_utf16().take(255).enumerate() {
        b.u16(i * 2, u);
    }
    b.u32(512, interface.index);
    b.u32(516, interface.if_type);
    b.u32(520, interface.mtu);
    // dwSpeed is the low 32 bits of the link speed, as Windows reports it.
    b.u32(524, interface.speed as u32);
    if let Some(mac) = interface.mac {
        b.u32(528, 6);
        b.put(532, &mac);
    }
    b.u32(540, 1);
    // IF_OPER_STATUS_OPERATIONAL / NON_OPERATIONAL.
    b.u32(544, if interface.up { 5 } else { 2 });
    let c = interface.counters;
    b.u32(552, c.bytes_in as u32);
    b.u32(556, c.frames_in as u32);
    b.u32(576, c.bytes_out as u32);
    b.u32(580, c.frames_out as u32);
    let description = interface.description.as_bytes();
    let n = description.len().min(255);
    b.u32(600, n as u32 + 1);
    b.put(604, &description[..n]);
    b.bytes
}
/// MIB_IFTABLE: dwNumEntries then rows.
pub fn if_table(list: &[Interface]) -> Vec<u8> {
    let mut b = (list.len() as u32).to_le_bytes().to_vec();
    for interface in list {
        b.extend(if_row(interface));
    }
    b
}
fn sorted_by_index(list: &[Interface]) -> Vec<&Interface> {
    let mut sorted: Vec<&Interface> = list.iter().collect();
    sorted.sort_by_key(|i| i.index);
    sorted
}
/// MIB_IPADDRTABLE, optionally sorted by address.
pub fn ip_addr_table(list: &[Interface], order: bool) -> Vec<u8> {
    let mut rows: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    for interface in list {
        let Some((address, prefix)) = interface.ipv4 else {
            continue;
        };
        let mut row = [0u8; 24];
        row[..4].copy_from_slice(&address);
        row[4..8].copy_from_slice(&interface.index.to_le_bytes());
        row[8..12].copy_from_slice(&mask(prefix));
        row[12..16].copy_from_slice(&1u32.to_le_bytes());
        row[16..20].copy_from_slice(&65535u32.to_le_bytes());
        // MIB_IPADDR_PRIMARY, plus MIB_IPADDR_DYNAMIC for a DHCP address.
        let kind: u16 = 0x1 | if interface.dhcp { 0x4 } else { 0 };
        row[22..24].copy_from_slice(&kind.to_le_bytes());
        rows.push((address, row.to_vec()));
    }
    if order {
        rows.sort_by_key(|(a, _)| u32::from_be_bytes(*a));
    }
    let mut b = (rows.len() as u32).to_le_bytes().to_vec();
    for (_, row) in rows {
        b.extend(row);
    }
    b
}

/// One IPv4 route as `route print` lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Route {
    pub destination: [u8; 4],
    pub prefix: u8,
    pub next_hop: [u8; 4],
    pub index: u32,
    /// Route metric (the interface metric is added by the views).
    pub metric: u32,
    pub direct: bool,
    pub loopback: bool,
}
/// The IPv4 routing table: the default route through the gateway, then
/// each interface's on-link routes.
pub fn routes(list: &[Interface]) -> Vec<Route> {
    let mut routes = Vec::new();
    for interface in list {
        let Some((address, prefix)) = interface.ipv4 else {
            continue;
        };
        if let Some(gateway) = interface.gateway {
            routes.push(Route {
                destination: [0; 4],
                prefix: 0,
                next_hop: gateway,
                index: interface.index,
                metric: 0,
                direct: false,
                loopback: false,
            });
        }
        for (destination, length) in prefixes(address, prefix) {
            routes.push(Route {
                destination,
                prefix: length,
                next_hop: [0; 4],
                index: interface.index,
                metric: 256,
                direct: true,
                loopback: interface.loopback(),
            });
        }
    }
    routes
}
/// MIB_IPFORWARDTABLE (56-byte rows), optionally sorted by destination.
pub fn ip_forward_table(list: &[Interface], order: bool) -> Vec<u8> {
    let mut all = routes(list);
    if order {
        all.sort_by_key(|r| (u32::from_be_bytes(r.destination), r.prefix, r.index));
    }
    let mut b = (all.len() as u32).to_le_bytes().to_vec();
    for route in all {
        let metric = route.metric
            + list
                .iter()
                .find(|i| i.index == route.index)
                .map_or(0, |i| i.metric);
        let mut row = [0u8; 56];
        row[..4].copy_from_slice(&route.destination);
        row[4..8].copy_from_slice(&mask(route.prefix));
        // A direct route's next hop is the interface's own address.
        let hop = if route.direct {
            list.iter()
                .find(|i| i.index == route.index)
                .and_then(|i| i.ipv4)
                .map_or([0; 4], |(a, _)| a)
        } else {
            route.next_hop
        };
        row[12..16].copy_from_slice(&hop);
        row[16..20].copy_from_slice(&route.index.to_le_bytes());
        // MIB_IPROUTE_TYPE_DIRECT 3 / INDIRECT 4; MIB_IPPROTO_LOCAL 2 /
        // NETMGMT 3.
        row[20..24].copy_from_slice(&(if route.direct { 3u32 } else { 4 }).to_le_bytes());
        row[24..28].copy_from_slice(&(if route.direct { 2u32 } else { 3 }).to_le_bytes());
        row[36..40].copy_from_slice(&metric.to_le_bytes());
        for slot in 1..5 {
            row[36 + slot * 4..40 + slot * 4].copy_from_slice(&u32::MAX.to_le_bytes());
        }
        b.extend(row);
    }
    b
}
/// MIB_IPFORWARD_ROW2 (104 bytes).
pub fn forward_row2(route: &Route, interface: &Interface) -> [u8; 104] {
    let mut row = [0u8; 104];
    row[..8].copy_from_slice(&interface.luid().to_le_bytes());
    row[8..12].copy_from_slice(&route.index.to_le_bytes());
    row[12..40].copy_from_slice(&sockaddr_inet(route.destination));
    row[40] = route.prefix;
    row[44..72].copy_from_slice(&sockaddr_inet(route.next_hop));
    row[76..80].copy_from_slice(&u32::MAX.to_le_bytes());
    row[80..84].copy_from_slice(&u32::MAX.to_le_bytes());
    row[84..88].copy_from_slice(&route.metric.to_le_bytes());
    // MIB_IPPROTO_LOCAL / NT_AUTOSTATIC (DHCP default route).
    row[88..92].copy_from_slice(&(if route.direct { 2u32 } else { 10002 }).to_le_bytes());
    row[92] = u8::from(route.loopback);
    row[93] = 1;
    row[94] = 1;
    row[95] = 1;
    // NlroWellKnown for on-link routes, NlroDHCP for the default route.
    row[100..104].copy_from_slice(&(if route.direct { 1u32 } else { 2 }).to_le_bytes());
    row
}
/// MIB_UNICASTIPADDRESS_ROW (80 bytes).
pub fn unicast_row(interface: &Interface) -> Option<[u8; 80]> {
    let (address, prefix) = interface.ipv4?;
    let mut row = [0u8; 80];
    row[..28].copy_from_slice(&sockaddr_inet(address));
    row[32..40].copy_from_slice(&interface.luid().to_le_bytes());
    row[40..44].copy_from_slice(&interface.index.to_le_bytes());
    let origin = if interface.loopback() {
        2u32
    } else if interface.dhcp {
        3
    } else {
        1
    };
    row[44..48].copy_from_slice(&origin.to_le_bytes());
    row[48..52].copy_from_slice(&origin.to_le_bytes());
    let lifetime = interface.lease.unwrap_or(u32::MAX);
    row[52..56].copy_from_slice(&lifetime.to_le_bytes());
    row[56..60].copy_from_slice(&lifetime.to_le_bytes());
    row[60] = prefix;
    row[64..68].copy_from_slice(&4u32.to_le_bytes());
    Some(row)
}
/// MIB_IPINTERFACE_ROW (168 bytes) for AF_INET.
pub fn ip_interface_row(interface: &Interface) -> [u8; 168] {
    let mut row = [0u8; 168];
    row[..2].copy_from_slice(&AF_INET.to_le_bytes());
    row[8..16].copy_from_slice(&interface.luid().to_le_bytes());
    row[16..20].copy_from_slice(&interface.index.to_le_bytes());
    row[20..24].copy_from_slice(&65535u32.to_le_bytes());
    row[32..36].copy_from_slice(&200u32.to_le_bytes());
    row[36..40].copy_from_slice(&600u32.to_le_bytes());
    row[44] = 1; // UseAutomaticMetric
    row[45] = 1;
    row[52..56].copy_from_slice(&2u32.to_le_bytes()); // RouterDiscoveryDhcp
    row[56..60].copy_from_slice(&1u32.to_le_bytes());
    row[60..64].copy_from_slice(&30000u32.to_le_bytes());
    row[64..68].copy_from_slice(&1000u32.to_le_bytes());
    row[68..72].copy_from_slice(&600000u32.to_le_bytes());
    row[72..76].copy_from_slice(&1u32.to_le_bytes()); // LinkLocalDelayed (APIPA after DHCP fails)
    for zone in 0..16 {
        let value = match zone {
            0..=3 => interface.index,
            14 => 0,
            _ => 1,
        };
        row[80 + zone * 4..84 + zone * 4].copy_from_slice(&value.to_le_bytes());
    }
    row[148..152].copy_from_slice(&interface.metric.to_le_bytes());
    row[152..156].copy_from_slice(
        &(if interface.loopback() {
            u32::MAX
        } else {
            interface.mtu
        })
        .to_le_bytes(),
    );
    row[156] = u8::from(interface.up);
    row[158] = u8::from(!interface.loopback());
    row[159] = u8::from(!interface.loopback());
    row[160..164].copy_from_slice(&30000u32.to_le_bytes());
    row
}

/// The interface a destination leaves through: the loopback for 127/8,
/// otherwise the interface whose subnet holds it, otherwise the one with a
/// default route.
pub fn best_interface(list: &[Interface], destination: [u8; 4]) -> Result<u32, u32> {
    if destination[0] == 127 {
        return Ok(LOOPBACK_INDEX);
    }
    for interface in list.iter().filter(|i| !i.loopback()) {
        if let Some((address, prefix)) = interface.ipv4
            && (network(address, prefix) == network(destination, prefix) || destination == address)
        {
            return Ok(interface.index);
        }
    }
    list.iter()
        .find(|i| i.gateway.is_some() && i.up)
        .map(|i| i.index)
        .ok_or(ERROR_NETWORK_UNREACHABLE)
}

/// Finds an interface by its LUID value.
pub fn by_luid(list: &[Interface], luid: u64) -> Option<&Interface> {
    list.iter().find(|i| i.luid() == luid)
}
pub fn by_index(list: &[Interface], index: u32) -> Option<&Interface> {
    list.iter().find(|i| i.index == index)
}
/// `ConvertInterfaceNameToLuid`: names compare case-insensitively.
pub fn by_name<'a>(list: &'a [Interface], name: &str) -> Option<&'a Interface> {
    list.iter().find(|i| i.name().eq_ignore_ascii_case(name))
}
pub fn by_alias<'a>(list: &'a [Interface], alias: &str) -> Option<&'a Interface> {
    list.iter().find(|i| i.alias.eq_ignore_ascii_case(alias))
}
pub fn by_guid<'a>(list: &'a [Interface], guid: &[u8; 16]) -> Option<&'a Interface> {
    list.iter().find(|i| &i.guid == guid)
}
/// Rows in index order, for the tables Windows sorts that way.
pub fn ordered(list: &[Interface]) -> Vec<&Interface> {
    sorted_by_index(list)
}

// ---- protocol statistics (GetIpStatisticsEx and the others) --------------
use door_service_protocol::net::stats::{Family, NEIGHBOR_STATIC, Neighbor};

fn words<const N: usize>(values: [u32; N]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}
/// MIB_IPSTATS (92 bytes) of one family: not forwarding, default TTL 128
/// and a 60-second reassembly timeout as Windows reports them, the
/// interface, address and route counts of that family.
pub fn ip_stats(f: &Family, list: &[Interface], v6: bool) -> Vec<u8> {
    let (interfaces, addresses, routes) = if v6 {
        let with: Vec<&Interface> = list
            .iter()
            .filter(|i| i.loopback() || !i.ipv6.is_empty())
            .collect();
        let addresses: usize = with
            .iter()
            .map(|i| if i.loopback() { 1 } else { i.ipv6.len() })
            .sum();
        let defaults = with.iter().filter(|i| i.gateway6.is_some()).count();
        (with.len(), addresses, addresses + defaults)
    } else {
        let with = list.iter().filter(|i| i.ipv4.is_some()).count();
        (with, with, routes(list).len())
    };
    words([
        2,   // dwForwarding: MIB_IP_NOT_FORWARDING
        128, // dwDefaultTTL
        f.ip_in_receives,
        f.ip_in_header_errors,
        0, // dwInAddrErrors
        0, // dwForwDatagrams
        f.ip_in_unknown_protos,
        0, // dwInDiscards
        f.ip_in_delivers,
        f.ip_out_requests,
        0,  // dwRoutingDiscards
        0,  // dwOutDiscards
        0,  // dwOutNoRoutes
        60, // dwReasmTimeout
        f.ip_reasm_reqds,
        0, // dwReasmOks
        0, // dwReasmFails
        0, // dwFragOks
        0, // dwFragFails
        f.ip_frag_creates,
        interfaces as u32,
        addresses as u32,
        routes as u32,
    ])
}
/// MIB_TCPSTATS (60 bytes): Van Jacobson RTO, the 5 ms floor and the
/// unbounded maximum and connection limit Windows reports.
pub fn tcp_stats(f: &Family) -> Vec<u8> {
    words([
        4, // MIB_TCP_RTO_VANJ
        5,
        u32::MAX,
        u32::MAX, // MIB_TCP_MAXCONN_DYNAMIC
        f.tcp_active_opens,
        f.tcp_passive_opens,
        f.tcp_attempt_fails,
        f.tcp_estab_resets,
        f.tcp_curr_estab,
        f.tcp_in_segs,
        f.tcp_out_segs,
        f.tcp_retrans_segs,
        f.tcp_in_errs,
        f.tcp_out_rsts,
        f.tcp_num_conns,
    ])
}
/// MIB_UDPSTATS (20 bytes).
pub fn udp_stats(f: &Family) -> Vec<u8> {
    words([
        f.udp_in_datagrams,
        f.udp_no_ports,
        f.udp_in_errors,
        f.udp_out_datagrams,
        f.udp_num_addrs,
    ])
}
/// MIB_ICMP (104 bytes): MIBICMPINFO's received then sent MIBICMPSTATS.
pub fn icmp_stats(f: &Family) -> Vec<u8> {
    let side = |outgoing: bool| {
        let (messages, errors) = if outgoing {
            (f.icmp_out_msgs, f.icmp_out_errors)
        } else {
            (f.icmp_in_msgs, f.icmp_in_errors)
        };
        let t = |kind| f.icmp(false, outgoing, kind);
        [
            messages,
            errors,
            t(3),
            t(11),
            t(12),
            t(4),
            t(5),
            t(8),
            t(0),
            t(13),
            t(14),
            t(17),
            t(18),
        ]
    };
    let mut b = words(side(false));
    b.extend_from_slice(&words(side(true)));
    b
}
/// MIB_ICMP_EX (2064 bytes): MIBICMPSTATS_EX received then sent, each
/// messages, errors and the count of every message type.
pub fn icmp_stats_ex(f: &Family, v6: bool) -> Vec<u8> {
    let mut b = Vec::with_capacity(2064);
    for outgoing in [false, true] {
        let (messages, errors) = if outgoing {
            (f.icmp_out_msgs, f.icmp_out_errors)
        } else {
            (f.icmp_in_msgs, f.icmp_in_errors)
        };
        b.extend_from_slice(&messages.to_le_bytes());
        b.extend_from_slice(&errors.to_le_bytes());
        for kind in 0..=255u8 {
            b.extend_from_slice(&f.icmp(v6, outgoing, kind).to_le_bytes());
        }
    }
    b
}
/// MIB_IPNETTABLE: the Ethernet adapter's IPv4 neighbours as MIB_IPNETROW
/// (index, MAC length, MAC in 8 bytes, address, MIB_IPNET_TYPE), by
/// address when `order`.
pub fn ip_net_table(list: &[Interface], neighbors: &[Neighbor], order: bool) -> Vec<u8> {
    let index = list
        .iter()
        .find(|i| !i.loopback())
        .map_or(ETHERNET_INDEX, |i| i.index);
    let mut rows: Vec<(&Neighbor, [u8; 4])> = neighbors
        .iter()
        .filter_map(|n| n.ipv4().map(|a| (n, a)))
        .collect();
    if order {
        rows.sort_by_key(|(_, a)| *a);
    }
    let mut b = vec![0u8; 4];
    b[..4].copy_from_slice(&(rows.len() as u32).to_le_bytes());
    for (n, address) in rows {
        let mut row = [0u8; 24];
        row[..4].copy_from_slice(&index.to_le_bytes());
        row[4..8].copy_from_slice(&6u32.to_le_bytes());
        row[8..14].copy_from_slice(&n.mac);
        row[16..20].copy_from_slice(&address);
        row[20..24].copy_from_slice(&u32::from(n.kind).to_le_bytes());
        b.extend_from_slice(&row);
    }
    b
}
/// MIB_IPNET_ROW2 (88 bytes): SOCKADDR_INET, interface index and LUID,
/// link-layer address, NL_NEIGHBOR_STATE (permanent for the fixed
/// entries, reachable for the learned ones) and the router flag.
pub fn ip_net_row2(n: &Neighbor, interface: &Interface) -> [u8; 88] {
    let mut row = [0u8; 88];
    match n.ipv4() {
        Some(address) => row[..16].copy_from_slice(&sockaddr_in(address)),
        None => {
            let scope = if n.address[0] == 0xfe && n.address[1] & 0xc0 == 0x80 {
                interface.index
            } else {
                0
            };
            row[..28].copy_from_slice(&sockaddr_in6(n.address, scope));
        }
    }
    row[28..32].copy_from_slice(&interface.index.to_le_bytes());
    row[32..40].copy_from_slice(&interface.luid().to_le_bytes());
    row[40..46].copy_from_slice(&n.mac);
    row[72..76].copy_from_slice(&6u32.to_le_bytes());
    let state: u32 = if n.kind == NEIGHBOR_STATIC { 6 } else { 5 };
    row[76..80].copy_from_slice(&state.to_le_bytes());
    row[80] = u8::from(n.router);
    row
}

#[cfg(test)]
mod tests;
