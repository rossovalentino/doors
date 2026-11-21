//! door.network (P15): the network service's status, configuration, name
//! resolution and socket requests. The original requests carry IPv4
//! addresses; `v6` has the IPv6 and dual-stack forms. All
//! integers are little endian.

/// Opcodes of the door.network family.
pub mod op {
    /// Adapter, address, DHCP and DNS state (`Status`).
    pub const STATUS: u32 = 0x6000;
    /// Set the configuration (`Config`): DHCP or a static address, DNS
    /// servers, the adapter on or off (airplane mode).
    pub const CONFIGURE: u32 = 0x6001;
    /// Start resolving a host name (body: UTF-8 name); answers a query id.
    pub const RESOLVE: u32 = 0x6002;
    /// A query's result (body: id): pending, the addresses, or the failure.
    pub const RESOLVED: u32 = 0x6003;
    /// Open a socket (body: kind u8, then optionally the Winsock family u8:
    /// `FAMILY_INET6` for an AF_INET6 socket, which netstat and the IPv6
    /// tables list; a `RAW` socket adds its IP protocol u8): answers its id.
    /// Raw sockets are for administrators (`Denied` otherwise).
    pub const SOCKET: u32 = 0x6010;
    /// SOCKET's family byte for an IPv6 (or dual-stack) socket (AF_INET6).
    pub const FAMILY_INET6: u8 = 23;
    /// Connect a TCP socket / set a UDP peer (id u32, address [4], port u16).
    pub const CONNECT: u32 = 0x6011;
    /// Queue bytes (id u32, then up to `CHUNK` bytes): answers how many.
    pub const SEND: u32 = 0x6012;
    /// Take received bytes (id u32, max u32): answers them (empty: none yet).
    pub const RECEIVE: u32 = 0x6013;
    /// Half-close: no more sends (id u32).
    pub const SHUTDOWN: u32 = 0x6014;
    pub const CLOSE: u32 = 0x6015;
    /// A socket's state (id u32): `SocketState` and buffered byte counts.
    pub const STATE: u32 = 0x6016;
    /// Listen on a TCP port (id u32, port u16, optional backlog u8; 0 or
    /// absent: 1, at most `MAX_BACKLOG`).
    pub const LISTEN: u32 = 0x6017;
    /// Bind to a local port (id u32, port u16; 0: an ephemeral one, then
    /// optionally the local address [16] in the `v6` form: one of this
    /// host's, which the socket then answers on alone): answers the port.
    /// A port another socket of the same kind holds is `Exists`.
    pub const BIND: u32 = 0x6018;
    /// Take a connection a listening socket accepted (id u32): answers
    /// `[socket id u32][peer address 4][peer port u16]`; `WouldBlock`
    /// while none is waiting. The listener keeps listening.
    pub const ACCEPT: u32 = 0x6019;
    /// UDP: send one datagram (id u32, address 4, port u16, then up to
    /// `CHUNK` bytes) made of the parts staged by `SEND_PART` and these
    /// bytes; binds an ephemeral port first if needed. Replies its length.
    pub const SEND_TO: u32 = 0x601A;
    /// UDP: take one datagram (id u32, max u32): `[address 4][port u16]
    /// [length u32]` then its first bytes (at most `CHUNK`); empty while
    /// none. `length` is the whole datagram; up to `max` of it stays
    /// staged for `RECEIVE_MORE` until the next RECEIVE_FROM.
    pub const RECEIVE_FROM: u32 = 0x601B;
    /// Both ends (id u32): `[local address 4][local port u16][peer address
    /// 4][peer port u16]` (zeros where not known yet).
    pub const ENDPOINTS: u32 = 0x601C;
    /// Set an option (id u32, option u8, value u32): `option`.
    pub const OPTION: u32 = 0x601D;
    /// UDP: more of the datagram RECEIVE_FROM staged (id u32, offset u32):
    /// up to `CHUNK` bytes from `offset`.
    pub const RECEIVE_MORE: u32 = 0x601E;
    /// UDP: stage part of an outgoing datagram (id u32, offset u32, then up
    /// to `CHUNK` bytes); offset 0 starts a new one, any other offset must
    /// equal what is staged. Replies the staged length.
    pub const SEND_PART: u32 = 0x601F;
    /// UDP: join or leave a multicast group (id u32, `MULTICAST_JOIN` or
    /// `MULTICAST_LEAVE` u8, group [16] in the `v6` form, IPv4-mapped for
    /// IPv4, interface u32: an IPv4 address or an IPv6 interface index, 0
    /// for the adapter). A joined socket receives the group's datagrams on
    /// its port, from the network and from this host. Joining twice is
    /// `Exists`, leaving a group not joined `NotFound`, an address that is
    /// not a group `InvalidRequest`.
    pub const MULTICAST: u32 = 0x6045;
    pub const MULTICAST_JOIN: u8 = 1;
    pub const MULTICAST_LEAVE: u8 = 2;
    /// MULTICAST's body length.
    pub const MULTICAST_BYTES: usize = 25;
}
/// Socket options (`op::OPTION`).
pub mod option {
    /// TCP: 1 sends small segments at once (no Nagle), 0 coalesces.
    pub const NO_DELAY: u8 = 1;
    /// TCP: keep-alive probe interval in milliseconds (0: off).
    pub const KEEP_ALIVE: u8 = 2;
    /// TCP: give up a connection silent for this many milliseconds (0: never).
    pub const TIMEOUT: u8 = 3;
    /// UDP and raw: the TTL (hop limit) of unicast datagrams, 1-255
    /// (`DEFAULT_TTL` until set).
    pub const TTL: u8 = 4;
    /// UDP and raw: the TTL of datagrams to a multicast group (default 1).
    pub const MULTICAST_TTL: u8 = 5;
    /// UDP: 1 (the default) delivers this host's own datagrams to a joined
    /// group to the socket, 0 does not (Winsock's IP_MULTICAST_LOOP applies
    /// to the receiving socket).
    pub const MULTICAST_LOOP: u8 = 6;
    /// Raw IPv4: 1 when the caller's datagrams carry their IP header
    /// (IP_HDRINCL).
    pub const HEADER_INCLUDED: u8 = 7;
    /// UDP: the interface multicast leaves by (IP_MULTICAST_IF): an IPv4
    /// address (its bytes as stored, little endian) or an IPv6 interface
    /// index; 0 is the adapter. The loopback interface (127.0.0.0/8, index
    /// `LOOPBACK_INDEX`) keeps the datagrams on this host.
    pub const MULTICAST_INTERFACE: u8 = 8;
}
/// The loopback interface's index (Windows' Loopback Pseudo-Interface 1).
pub const LOOPBACK_INDEX: u32 = 1;
/// The TTL Windows gives IPv4 datagrams, and its IPv6 hop limit.
pub const DEFAULT_TTL: u8 = 128;
/// Datagrams this host sent to a group a socket holds until it reads them.
pub const LOOPED_DATAGRAMS: usize = 16;
/// Bytes one SEND or RECEIVE carries.
pub const CHUNK: usize = 192;
/// The largest UDP datagram a socket sends or receives (its buffer).
pub const MAX_DATAGRAM: usize = 16 * 1024;
/// RECEIVE_FROM's reply header: address, port, datagram length.
pub const DATAGRAM_HEADER: usize = 10;
/// Sockets one client may hold (a listener's waiting connections count).
pub const SOCKETS_PER_CLIENT: usize = 32;
/// Connections a listening socket accepts before ACCEPT takes them.
pub const MAX_BACKLOG: usize = 8;
/// Bytes of an ENDPOINTS answer.
pub const ENDPOINTS_BYTES: usize = 12;
pub const TCP: u8 = 1;
pub const UDP: u8 = 2;
/// Raw IP: datagrams of one IP protocol. IPv4 ones arrive with their
/// header, as Winsock delivers them; IPv6 ones without.
pub const RAW: u8 = 3;

/// How the adapter gets its address.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dhcp {
    /// No adapter, or the adapter is off.
    #[default]
    Off = 0,
    /// Asking for a lease.
    Discovering = 1,
    /// Leased.
    Bound = 2,
    /// A static address.
    Static = 3,
}
impl Dhcp {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Off,
            1 => Self::Discovering,
            2 => Self::Bound,
            3 => Self::Static,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// A NIC exists.
    pub present: bool,
    pub link: bool,
    /// The adapter is on (off is airplane mode).
    pub enabled: bool,
    pub mac: [u8; 6],
    pub dhcp: Dhcp,
    pub address: [u8; 4],
    pub prefix: u8,
    pub gateway: [u8; 4],
    pub dns: [[u8; 4]; 2],
    pub lease_seconds: u32,
    pub frames_in: u64,
    pub frames_out: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    /// The configuration in force (Settings edits one part of it and
    /// writes the rest back unchanged).
    pub config: Config,
}
pub const STATUS_BYTES: usize = 88;
impl Status {
    pub fn encode(&self) -> [u8; STATUS_BYTES] {
        let mut b = [0u8; STATUS_BYTES];
        b[0] = u8::from(self.present) | u8::from(self.link) << 1 | u8::from(self.enabled) << 2;
        b[1] = self.dhcp as u8;
        b[2..8].copy_from_slice(&self.mac);
        b[8..12].copy_from_slice(&self.address);
        b[12] = self.prefix;
        b[13..17].copy_from_slice(&self.gateway);
        b[17..21].copy_from_slice(&self.dns[0]);
        b[21..25].copy_from_slice(&self.dns[1]);
        b[28..32].copy_from_slice(&self.lease_seconds.to_le_bytes());
        b[32..40].copy_from_slice(&self.frames_in.to_le_bytes());
        b[40..48].copy_from_slice(&self.frames_out.to_le_bytes());
        b[48..56].copy_from_slice(&self.bytes_in.to_le_bytes());
        b[56..64].copy_from_slice(&self.bytes_out.to_le_bytes());
        b[64..64 + CONFIG_BYTES].copy_from_slice(&self.config.encode());
        b
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != STATUS_BYTES || b[0] & !7 != 0 || b[12] > 32 {
            return None;
        }
        let a4 = |at: usize| -> [u8; 4] { b[at..at + 4].try_into().unwrap() };
        let u64at = |at: usize| u64::from_le_bytes(b[at..at + 8].try_into().unwrap());
        Some(Self {
            present: b[0] & 1 != 0,
            link: b[0] & 2 != 0,
            enabled: b[0] & 4 != 0,
            dhcp: Dhcp::from_u8(b[1])?,
            mac: b[2..8].try_into().unwrap(),
            address: a4(8),
            prefix: b[12],
            gateway: a4(13),
            dns: [a4(17), a4(21)],
            lease_seconds: u32::from_le_bytes(b[28..32].try_into().unwrap()),
            frames_in: u64at(32),
            frames_out: u64at(40),
            bytes_in: u64at(48),
            bytes_out: u64at(56),
            config: Config::decode(&b[64..64 + CONFIG_BYTES])?,
        })
    }
}

/// The configuration Settings writes (Network & internet).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub enabled: bool,
    /// None: DHCP.
    pub fixed: Option<([u8; 4], u8, [u8; 4])>,
    /// Manual DNS servers (0.0.0.0: from DHCP).
    pub dns: [[u8; 4]; 2],
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            fixed: None,
            dns: [[0; 4]; 2],
        }
    }
}
pub const CONFIG_BYTES: usize = 20;
impl Config {
    pub fn encode(&self) -> [u8; CONFIG_BYTES] {
        let mut b = [0u8; CONFIG_BYTES];
        b[0] = u8::from(self.enabled) | u8::from(self.fixed.is_some()) << 1;
        if let Some((address, prefix, gateway)) = self.fixed {
            b[1..5].copy_from_slice(&address);
            b[5] = prefix;
            b[6..10].copy_from_slice(&gateway);
        }
        b[10..14].copy_from_slice(&self.dns[0]);
        b[14..18].copy_from_slice(&self.dns[1]);
        b
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != CONFIG_BYTES || b[0] & !3 != 0 || b[18..].iter().any(|v| *v != 0) {
            return None;
        }
        let a4 = |at: usize| -> [u8; 4] { b[at..at + 4].try_into().unwrap() };
        let fixed = if b[0] & 2 != 0 {
            if b[5] == 0 || b[5] > 30 || a4(1) == [0; 4] {
                return None;
            }
            Some((a4(1), b[5], a4(6)))
        } else {
            None
        };
        Some(Self {
            enabled: b[0] & 1 != 0,
            fixed,
            dns: [a4(10), a4(14)],
        })
    }
}

/// A dotted IPv4 address ("192.168.1.20"); None for anything else.
pub fn parse_ipv4(text: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = text.trim().split('.');
    for slot in out.iter_mut() {
        let part = parts.next()?;
        if part.is_empty() || part.len() > 3 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    parts.next().is_none().then_some(out)
}

/// A socket's state as clients see it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketState {
    Closed = 0,
    Connecting = 1,
    Established = 2,
    /// The peer finished sending (half-closed from its side).
    PeerClosed = 3,
    /// This side shut down sending.
    Closing = 4,
    Listening = 5,
    /// Refused, reset or timed out.
    Failed = 6,
}

/// VPN connections (P15): WireGuard tunnels the network service runs.
/// Settings (Network & internet > VPN) saves and removes the signed-in
/// user's connections; Settings and the window server's Quick settings
/// connect, disconnect and list them. Every body starts with the user id
/// (u32): Settings' must be its own, the window server names the user of
/// the session it shows. One connection is up at a time.
pub mod vpn {
    /// Stage part of a connection's wg-quick text (uid, offset u32, then up
    /// to `PART` bytes); offset 0 starts over, any other must equal what is
    /// staged; at most `MAX_TEXT` bytes. Replies the staged length.
    pub const STAGE: u32 = 0x6020;
    /// Save the staged text as connection `name` (uid, then the UTF-8
    /// name): replaces a connection of that name, disconnecting it first.
    /// `InvalidRequest` when the text is not a valid single-peer
    /// configuration.
    pub const SAVE: u32 = 0x6021;
    /// Remove a connection (uid, name), disconnecting it first.
    pub const REMOVE: u32 = 0x6022;
    /// Bring a connection up (uid, name); another connection goes down.
    pub const CONNECT: u32 = 0x6023;
    pub const DISCONNECT: u32 = 0x6024;
    /// The user's connection `index` (uid, index u8): an `Entry`;
    /// `NotFound` past the last one.
    pub const ENTRY: u32 = 0x6025;
    pub const MAX_TEXT: usize = 4096;
    pub const MAX_NAME: usize = 64;
    pub const MAX_SERVER: usize = 64;
    /// Text bytes one STAGE carries.
    pub const PART: usize = 224;
    pub const ENTRY_BYTES: usize = 190;
    /// The opcodes of this family.
    pub const fn is_vpn(opcode: u32) -> bool {
        matches!(opcode, STAGE..=TUNNEL)
    }

    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum State {
        #[default]
        Disconnected = 0,
        /// Resolving the server or waiting for the first handshake.
        Connecting = 1,
        Connected = 2,
        /// The last attempt failed (`Failure` says why).
        Failed = 3,
    }
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum Failure {
        #[default]
        None = 0,
        /// No adapter, no address, or airplane mode.
        NoNetwork = 1,
        /// The server name did not resolve.
        Unresolved = 2,
        /// The server never answered the handshake.
        NoAnswer = 3,
    }
    /// One connection as Settings and Quick settings show it.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Entry {
        pub state: State,
        pub failure: Failure,
        /// The kill-switch: every route goes through the tunnel and nothing
        /// else leaves while it is up.
        pub blocks_untunneled: bool,
        name: [u8; MAX_NAME],
        name_len: u8,
        server: [u8; MAX_SERVER],
        server_len: u8,
        /// The tunnel's address.
        pub address: [u8; 4],
        pub prefix: u8,
        /// This connection's public key (what the server lists).
        pub public_key: [u8; 32],
        /// Seconds since the latest handshake; `u32::MAX` for none.
        pub handshake_age: u32,
        pub rx_bytes: u64,
        pub tx_bytes: u64,
    }
    impl Default for Entry {
        fn default() -> Self {
            Self {
                state: State::Disconnected,
                failure: Failure::None,
                blocks_untunneled: false,
                name: [0; MAX_NAME],
                name_len: 0,
                server: [0; MAX_SERVER],
                server_len: 0,
                address: [0; 4],
                prefix: 0,
                public_key: [0; 32],
                handshake_age: u32::MAX,
                rx_bytes: 0,
                tx_bytes: 0,
            }
        }
    }
    /// The longest prefix of `text` within `max` bytes on a char boundary.
    fn clip(text: &str, max: usize) -> &str {
        let mut end = text.len().min(max);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        &text[..end]
    }
    impl Entry {
        pub fn new(name: &str, server: &str) -> Self {
            let mut e = Self::default();
            let (n, s) = (clip(name, MAX_NAME), clip(server, MAX_SERVER));
            e.name[..n.len()].copy_from_slice(n.as_bytes());
            e.name_len = n.len() as u8;
            e.server[..s.len()].copy_from_slice(s.as_bytes());
            e.server_len = s.len() as u8;
            e
        }
        pub fn name(&self) -> &str {
            core::str::from_utf8(&self.name[..usize::from(self.name_len)]).unwrap_or("")
        }
        /// The server as configured ("vpn.example.com:51820").
        pub fn server(&self) -> &str {
            core::str::from_utf8(&self.server[..usize::from(self.server_len)]).unwrap_or("")
        }
        pub fn encode(&self) -> [u8; ENTRY_BYTES] {
            let mut b = [0u8; ENTRY_BYTES];
            b[0] = self.state as u8;
            b[1] = self.failure as u8;
            b[2] = u8::from(self.blocks_untunneled);
            b[3] = self.name_len;
            b[4..68].copy_from_slice(&self.name);
            b[68] = self.server_len;
            b[69..133].copy_from_slice(&self.server);
            b[133..137].copy_from_slice(&self.address);
            b[137] = self.prefix;
            b[138..170].copy_from_slice(&self.public_key);
            b[170..174].copy_from_slice(&self.handshake_age.to_le_bytes());
            b[174..182].copy_from_slice(&self.rx_bytes.to_le_bytes());
            b[182..190].copy_from_slice(&self.tx_bytes.to_le_bytes());
            b
        }
        pub fn decode(b: &[u8]) -> Option<Self> {
            if b.len() != ENTRY_BYTES
                || b[2] > 1
                || usize::from(b[3]) > MAX_NAME
                || usize::from(b[68]) > MAX_SERVER
                || b[137] > 32
            {
                return None;
            }
            let state = match b[0] {
                0 => State::Disconnected,
                1 => State::Connecting,
                2 => State::Connected,
                3 => State::Failed,
                _ => return None,
            };
            let failure = match b[1] {
                0 => Failure::None,
                1 => Failure::NoNetwork,
                2 => Failure::Unresolved,
                3 => Failure::NoAnswer,
                _ => return None,
            };
            let e = Self {
                state,
                failure,
                blocks_untunneled: b[2] == 1,
                name: b[4..68].try_into().ok()?,
                name_len: b[3],
                server: b[69..133].try_into().ok()?,
                server_len: b[68],
                address: b[133..137].try_into().ok()?,
                prefix: b[137],
                public_key: b[138..170].try_into().ok()?,
                handshake_age: u32::from_le_bytes(b[170..174].try_into().ok()?),
                rx_bytes: u64::from_le_bytes(b[174..182].try_into().ok()?),
                tx_bytes: u64::from_le_bytes(b[182..190].try_into().ok()?),
            };
            core::str::from_utf8(&e.name[..usize::from(e.name_len)]).ok()?;
            core::str::from_utf8(&e.server[..usize::from(e.server_len)]).ok()?;
            Some(e)
        }
    }
    /// The tunnel adapter while a connection is up (or coming up), as
    /// ipconfig and GetAdaptersAddresses show it: machine-wide, so any
    /// process may ask (empty body, no user id). Reply `Tunnel::encode`,
    /// empty when no tunnel is up.
    pub const TUNNEL: u32 = 0x6026;
    pub const TUNNEL_BYTES: usize = 97;
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Tunnel {
        /// The handshake completed (the adapter is up).
        pub connected: bool,
        pub address: [u8; 4],
        pub prefix: u8,
        pub dns: [[u8; 4]; 2],
        pub mtu: u16,
        pub rx_bytes: u64,
        pub tx_bytes: u64,
        name: [u8; MAX_NAME],
        name_len: u8,
    }
    impl Tunnel {
        pub fn new(name: &str) -> Self {
            let n = clip(name, MAX_NAME);
            let mut t = Self {
                connected: false,
                address: [0; 4],
                prefix: 0,
                dns: [[0; 4]; 2],
                mtu: 0,
                rx_bytes: 0,
                tx_bytes: 0,
                name: [0; MAX_NAME],
                name_len: n.len() as u8,
            };
            t.name[..n.len()].copy_from_slice(n.as_bytes());
            t
        }
        /// The connection's name: the adapter's (ipconfig's "Unknown
        /// adapter <name>").
        pub fn name(&self) -> &str {
            core::str::from_utf8(&self.name[..usize::from(self.name_len)]).unwrap_or("")
        }
        pub fn encode(&self) -> [u8; TUNNEL_BYTES] {
            let mut b = [0u8; TUNNEL_BYTES];
            b[0] = u8::from(self.connected);
            b[1] = self.prefix;
            b[2..6].copy_from_slice(&self.address);
            b[6..10].copy_from_slice(&self.dns[0]);
            b[10..14].copy_from_slice(&self.dns[1]);
            b[14..16].copy_from_slice(&self.mtu.to_le_bytes());
            b[16..24].copy_from_slice(&self.rx_bytes.to_le_bytes());
            b[24..32].copy_from_slice(&self.tx_bytes.to_le_bytes());
            b[32] = self.name_len;
            b[33..97].copy_from_slice(&self.name);
            b
        }
        pub fn decode(b: &[u8]) -> Option<Self> {
            if b.len() != TUNNEL_BYTES || b[0] > 1 || b[1] > 32 || usize::from(b[32]) > MAX_NAME {
                return None;
            }
            let four = |at: usize| -> [u8; 4] { b[at..at + 4].try_into().unwrap() };
            let t = Self {
                connected: b[0] == 1,
                prefix: b[1],
                address: four(2),
                dns: [four(6), four(10)],
                mtu: u16::from_le_bytes([b[14], b[15]]),
                rx_bytes: u64::from_le_bytes(b[16..24].try_into().ok()?),
                tx_bytes: u64::from_le_bytes(b[24..32].try_into().ok()?),
                name_len: b[32],
                name: b[33..97].try_into().ok()?,
            };
            core::str::from_utf8(&t.name[..usize::from(t.name_len)]).ok()?;
            Some(t)
        }
    }
    /// A body: the user id, then `rest`.
    pub fn body(uid: u32, rest: &[u8]) -> ([u8; crate::wire::BODY_BYTES], usize) {
        let mut b = [0u8; crate::wire::BODY_BYTES];
        let n = rest.len().min(b.len() - 4);
        b[..4].copy_from_slice(&uid.to_le_bytes());
        b[4..4 + n].copy_from_slice(&rest[..n]);
        (b, 4 + n)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn tunnels_round_trip() {
            let mut t = Tunnel::new("Office VPN");
            t.connected = true;
            t.address = [10, 8, 0, 2];
            t.prefix = 24;
            t.dns = [[10, 8, 0, 1], [0; 4]];
            t.mtu = 1420;
            t.rx_bytes = 5;
            let d = Tunnel::decode(&t.encode()).unwrap();
            assert_eq!(d, t);
            assert_eq!(d.name(), "Office VPN");
            let mut bad = t.encode();
            bad[1] = 33;
            assert!(Tunnel::decode(&bad).is_none());
            assert!(is_vpn(TUNNEL));
        }
        #[test]
        fn entries_round_trip_and_reject_malformed_bytes() {
            let mut e = Entry::new("Office", "vpn.example.com:51820");
            e.state = State::Connected;
            e.blocks_untunneled = true;
            e.address = [10, 8, 0, 2];
            e.prefix = 24;
            e.public_key = [7; 32];
            e.handshake_age = 12;
            e.rx_bytes = 1 << 40;
            e.tx_bytes = 99;
            assert_eq!(Entry::decode(&e.encode()), Some(e));
            assert_eq!((e.name(), e.server()), ("Office", "vpn.example.com:51820"));
            for (at, value) in [(0, 4), (1, 9), (2, 2), (3, 65), (137, 33)] {
                let mut bad = e.encode();
                bad[at] = value;
                assert_eq!(Entry::decode(&bad), None, "byte {at}");
            }
            // Names are clipped on a character boundary.
            let long = "é".repeat(40);
            assert_eq!(Entry::new(&long, "").name().len(), 64);
            const _: () = assert!(
                ENTRY_BYTES <= crate::wire::BODY_BYTES && PART + 8 <= crate::wire::BODY_BYTES
            );
            assert!(is_vpn(STAGE) && is_vpn(ENTRY) && !is_vpn(super::super::op::SEND_PART));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ipv6_connections_round_trip_and_reject_malformed_records() {
        let c = Connection6 {
            protocol: 6,
            state: tcp_state::ESTABLISHED,
            local: [0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
            local_scope: 2,
            local_port: 50_000,
            remote: v6::mapped([10, 0, 2, 2]),
            remote_scope: 0,
            remote_port: 7,
            pid: 42,
        };
        let mut b = [0u8; CONNECTION6_BYTES];
        c.encode(&mut b);
        assert_eq!(Connection6::decode(&b), Some(c));
        let mut bad = b;
        bad[0] = 1;
        assert_eq!(Connection6::decode(&bad), None);
        let mut bad = b;
        bad[26] = 1;
        assert_eq!(Connection6::decode(&bad), None);
        assert_eq!(Connection6::decode(&b[1..]), None);
    }
    #[test]
    fn status_and_config_round_trip_and_reject_malformed_words() {
        let s = Status {
            present: true,
            link: true,
            enabled: true,
            mac: [0x52, 0x54, 0, 0x12, 0x34, 0x56],
            dhcp: Dhcp::Bound,
            address: [10, 0, 2, 15],
            prefix: 24,
            gateway: [10, 0, 2, 2],
            dns: [[10, 0, 2, 3], [0; 4]],
            lease_seconds: 86400,
            frames_in: 7,
            frames_out: 9,
            bytes_in: 1000,
            bytes_out: 2000,
            config: Config {
                dns: [[1, 1, 1, 1], [0; 4]],
                ..Config::default()
            },
        };
        assert_eq!(Status::decode(&s.encode()), Some(s));
        let mut bad = s.encode();
        bad[1] = 9;
        assert_eq!(Status::decode(&bad), None);
        let c = Config {
            enabled: true,
            fixed: Some(([192, 168, 1, 20], 24, [192, 168, 1, 1])),
            dns: [[1, 1, 1, 1], [9, 9, 9, 9]],
        };
        assert_eq!(Config::decode(&c.encode()), Some(c));
        assert_eq!(
            Config::decode(&Config::default().encode()),
            Some(Config::default())
        );
        let mut bad = c.encode();
        bad[5] = 31;
        assert_eq!(Config::decode(&bad), None, "a /31 is not a host network");
        const _: () = assert!(
            STATUS_BYTES <= crate::wire::BODY_BYTES && CHUNK + 4 <= crate::wire::BODY_BYTES
        );
    }
    #[test]
    fn dotted_addresses_parse_strictly() {
        assert_eq!(parse_ipv4(" 10.0.2.15 "), Some([10, 0, 2, 15]));
        for bad in [
            "10.0.2",
            "10.0.2.15.1",
            "256.1.1.1",
            "1..1.1",
            "a.b.c.d",
            "1.1.1.+1",
        ] {
            assert_eq!(parse_ipv4(bad), None, "{bad}");
        }
    }
}

/// ICMP echo and the connection table (IcmpSendEcho, ping, tracert,
/// GetExtendedTcpTable, netstat).
/// Windows Defender Firewall in door.network (`door_firewall`): Init keeps
/// the policy (HKLM FirewallRules, the profile switches) and hands it over;
/// door.network enforces it and queues the programs it blocked for the
/// window server's Security Alert.
pub mod firewall {
    /// The state (empty body): `[domain, private, public: profile byte]
    /// [active profile bit][0; 3][rules u32][programs blocked waiting u32]`.
    pub const STATE: u32 = 0x6050;
    /// Init: stage the policy's rules (offset u32, then up to `PART` bytes
    /// of "id\trule\n" lines; offset 0 starts over).
    pub const STAGE: u32 = 0x6051;
    /// Init: apply the staged rules with the three profile bytes.
    pub const APPLY: u32 = 0x6052;
    /// Init: the Windows program a process runs (pid u64, then its
    /// Windows path, UTF-8), for program rules.
    pub const PROGRAM: u32 = 0x6053;
    /// The window server: the next program the firewall blocked from
    /// listening that no rule names: `[pid u64][path UTF-8]`, empty when
    /// none waits.
    pub const ALERT: u32 = 0x6054;
    pub const STATE_BYTES: usize = 16;
    /// Bytes one STAGE carries, and the most a policy may stage.
    pub const PART: usize = 200;
    pub const MAX_POLICY: usize = 512 * 1024;
    /// Profile byte bits.
    pub mod profile {
        pub const ON: u8 = 0x1;
        /// Inbound connections no rule allows are allowed (not Windows'
        /// default).
        pub const INBOUND_ALLOW: u8 = 0x2;
        /// "Block all incoming connections, including those in the list of
        /// allowed apps".
        pub const SHIELDS_UP: u8 = 0x4;
        /// Outbound connections no rule allows are blocked.
        pub const OUTBOUND_BLOCK: u8 = 0x8;
        /// Windows' default: on, inbound blocked, outbound allowed.
        pub const DEFAULT: u8 = ON;
    }
}
pub mod diag {
    /// Start an ICMP echo (`EchoRequest`): answers a token.
    pub const PING: u32 = 0x6030;
    /// A started echo's outcome (body: token u32): `Echo`, or `WouldBlock`
    /// while it waits. Taking it ends the request.
    pub const PING_RESULT: u32 = 0x6031;
    /// Every TCP and UDP socket of the stack (body: first index u16):
    /// `[total u16][count u16]` then `count` `Connection`s.
    pub const CONNECTIONS: u32 = 0x6032;
    /// Records one CONNECTIONS reply carries.
    pub const PAGE: usize = 11;
    /// The AF_INET6 sockets (body: first index u16): `[total u16][count
    /// u16]` then `count` `Connection6`s (GetTcp6Table, netstat's IPv6
    /// lines). CONNECTIONS lists only AF_INET sockets.
    pub const CONNECTIONS6: u32 = 0x6035;
    /// Records one CONNECTIONS6 reply carries.
    pub const PAGE6: usize = 4;
}
/// IP_STATUS values (Windows' IcmpSendEcho reply status).
pub mod ip_status {
    pub const SUCCESS: u32 = 0;
    pub const BUF_TOO_SMALL: u32 = 11001;
    pub const DEST_NET_UNREACHABLE: u32 = 11002;
    pub const DEST_HOST_UNREACHABLE: u32 = 11003;
    pub const DEST_PROT_UNREACHABLE: u32 = 11004;
    pub const DEST_PORT_UNREACHABLE: u32 = 11005;
    pub const NO_RESOURCES: u32 = 11006;
    pub const PACKET_TOO_BIG: u32 = 11009;
    pub const REQ_TIMED_OUT: u32 = 11010;
    pub const TTL_EXPIRED_TRANSIT: u32 = 11013;
    pub const GENERAL_FAILURE: u32 = 11050;
}
/// `diag::PING`: `[address 4][ttl u8][flags u8][size u16][timeout ms
/// u32]` then the data pattern (at most `PATTERN_BYTES`; none: Windows'
/// "abcdefghijklmnopqrstuvw"), repeated to `size` bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EchoRequest {
    pub address: [u8; 4],
    pub ttl: u8,
    pub dont_fragment: bool,
    /// Echo data bytes, at most `MAX_ECHO`.
    pub size: u16,
    pub timeout: u32,
    pub pattern: [u8; PATTERN_BYTES],
    pub pattern_len: u8,
}
pub const PATTERN_BYTES: usize = 64;
/// The largest echo data a request may carry.
pub const MAX_ECHO: u16 = 8192;
impl EchoRequest {
    /// The bytes of the request; `len` of `buffer`.
    pub fn encode(&self, b: &mut [u8; 12 + PATTERN_BYTES]) -> usize {
        b[..4].copy_from_slice(&self.address);
        b[4] = self.ttl;
        b[5] = u8::from(self.dont_fragment);
        b[6..8].copy_from_slice(&self.size.to_le_bytes());
        b[8..12].copy_from_slice(&self.timeout.to_le_bytes());
        let n = usize::from(self.pattern_len).min(PATTERN_BYTES);
        b[12..12 + n].copy_from_slice(&self.pattern[..n]);
        12 + n
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if !(12..=12 + PATTERN_BYTES).contains(&b.len()) || b[5] > 1 || b[4] == 0 {
            return None;
        }
        let mut pattern = [0; PATTERN_BYTES];
        pattern[..b.len() - 12].copy_from_slice(&b[12..]);
        let size = u16::from_le_bytes([b[6], b[7]]);
        let timeout = u32::from_le_bytes(b[8..12].try_into().ok()?);
        (size <= MAX_ECHO && (1..=600_000).contains(&timeout)).then(|| Self {
            address: b[..4].try_into().unwrap(),
            ttl: b[4],
            dont_fragment: b[5] == 1,
            size,
            timeout,
            pattern,
            pattern_len: (b.len() - 12) as u8,
        })
    }
    /// The echo data: the pattern repeated to `size` bytes.
    pub fn data_at(&self, index: usize) -> u8 {
        const WINDOWS: &[u8] = b"abcdefghijklmnopqrstuvw";
        match usize::from(self.pattern_len) {
            0 => WINDOWS[index % WINDOWS.len()],
            n => self.pattern[index % n],
        }
    }
}
/// `diag::PING_RESULT`: `[status u32][rtt ms u32][ttl u8][0; 3][from 4]
/// [size u16]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Echo {
    pub status: u32,
    pub rtt: u32,
    pub ttl: u8,
    pub from: [u8; 4],
    pub size: u16,
}
impl Echo {
    pub fn encode(&self) -> [u8; 18] {
        let mut b = [0; 18];
        b[..4].copy_from_slice(&self.status.to_le_bytes());
        b[4..8].copy_from_slice(&self.rtt.to_le_bytes());
        b[8] = self.ttl;
        b[12..16].copy_from_slice(&self.from);
        b[16..18].copy_from_slice(&self.size.to_le_bytes());
        b
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != 18 {
            return None;
        }
        Some(Self {
            status: u32::from_le_bytes(b[..4].try_into().ok()?),
            rtt: u32::from_le_bytes(b[4..8].try_into().ok()?),
            ttl: b[8],
            from: b[12..16].try_into().ok()?,
            size: u16::from_le_bytes([b[16], b[17]]),
        })
    }
}
/// One socket in `diag::CONNECTIONS` (20 bytes): `[protocol u8 (6 TCP, 17
/// UDP)][state u8 (MIB_TCP_STATE; 0 for UDP)][0; 2][local 4][local port
/// u16][remote port u16][remote 4][owning pid u32]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Connection {
    pub protocol: u8,
    pub state: u8,
    pub local: [u8; 4],
    pub local_port: u16,
    pub remote: [u8; 4],
    pub remote_port: u16,
    pub pid: u32,
}
pub const CONNECTION_BYTES: usize = 20;
impl Connection {
    pub fn encode(&self, b: &mut [u8]) {
        b[0] = self.protocol;
        b[1] = self.state;
        b[2..4].copy_from_slice(&[0, 0]);
        b[4..8].copy_from_slice(&self.local);
        b[8..10].copy_from_slice(&self.local_port.to_le_bytes());
        b[10..12].copy_from_slice(&self.remote_port.to_le_bytes());
        b[12..16].copy_from_slice(&self.remote);
        b[16..20].copy_from_slice(&self.pid.to_le_bytes());
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != CONNECTION_BYTES || !matches!(b[0], 6 | 17) || b[1] > 12 {
            return None;
        }
        Some(Self {
            protocol: b[0],
            state: b[1],
            local: b[4..8].try_into().ok()?,
            local_port: u16::from_le_bytes([b[8], b[9]]),
            remote_port: u16::from_le_bytes([b[10], b[11]]),
            remote: b[12..16].try_into().ok()?,
            pid: u32::from_le_bytes(b[16..20].try_into().ok()?),
        })
    }
}
/// One AF_INET6 socket for the IPv6 tables: addresses as 16 bytes (an
/// IPv4 peer of a dual-stack socket as `::ffff:a.b.c.d`) with their scope
/// ids (the interface index of a link-local address, else 0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Connection6 {
    pub protocol: u8,
    pub state: u8,
    pub local: [u8; 16],
    pub local_scope: u32,
    pub local_port: u16,
    pub remote: [u8; 16],
    pub remote_scope: u32,
    pub remote_port: u16,
    pub pid: u32,
}
pub const CONNECTION6_BYTES: usize = 52;
impl Connection6 {
    pub fn encode(&self, b: &mut [u8]) {
        b[0] = self.protocol;
        b[1] = self.state;
        b[2..4].copy_from_slice(&self.local_port.to_le_bytes());
        b[4..20].copy_from_slice(&self.local);
        b[20..24].copy_from_slice(&self.local_scope.to_le_bytes());
        b[24..26].copy_from_slice(&self.remote_port.to_le_bytes());
        b[26..28].copy_from_slice(&[0, 0]);
        b[28..44].copy_from_slice(&self.remote);
        b[44..48].copy_from_slice(&self.remote_scope.to_le_bytes());
        b[48..52].copy_from_slice(&self.pid.to_le_bytes());
    }
    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() != CONNECTION6_BYTES
            || !matches!(b[0], 6 | 17)
            || b[1] > 12
            || b[26..28] != [0, 0]
        {
            return None;
        }
        Some(Self {
            protocol: b[0],
            state: b[1],
            local_port: u16::from_le_bytes([b[2], b[3]]),
            local: b[4..20].try_into().ok()?,
            local_scope: u32::from_le_bytes(b[20..24].try_into().ok()?),
            remote_port: u16::from_le_bytes([b[24], b[25]]),
            remote: b[28..44].try_into().ok()?,
            remote_scope: u32::from_le_bytes(b[44..48].try_into().ok()?),
            pid: u32::from_le_bytes(b[48..52].try_into().ok()?),
        })
    }
}
const _: () = assert!(4 + diag::PAGE6 * CONNECTION6_BYTES <= 240);
/// MIB_TCP_STATE values.
pub mod tcp_state {
    pub const CLOSED: u8 = 1;
    pub const LISTEN: u8 = 2;
    pub const SYN_SENT: u8 = 3;
    pub const SYN_RCVD: u8 = 4;
    pub const ESTABLISHED: u8 = 5;
    pub const FIN_WAIT1: u8 = 6;
    pub const FIN_WAIT2: u8 = 7;
    pub const CLOSE_WAIT: u8 = 8;
    pub const CLOSING: u8 = 9;
    pub const LAST_ACK: u8 = 10;
    pub const TIME_WAIT: u8 = 11;
}

/// IPv6 (and dual-stack) forms of the socket and name requests. An
/// endpoint is 18 bytes, `[address 16][port u16]`; an IPv4 address travels
/// in its IPv4-mapped form (`::ffff:a.b.c.d`), so one socket API serves both
/// families.
pub mod v6 {
    /// Start resolving a host name to IPv6 addresses (AAAA): a query id
    /// for `op::RESOLVED`, whose answer is then 16-byte addresses.
    pub const RESOLVE: u32 = 0x6004;
    /// IPv6 addresses, router and DNS servers of the adapter (`Status6`).
    pub const STATUS: u32 = 0x6005;
    /// `op::CONNECT` with an endpoint: `[id u32][endpoint 18]`.
    pub const CONNECT: u32 = 0x6040;
    /// `op::SEND_TO` with an endpoint: `[id u32][endpoint 18]` then data.
    pub const SEND_TO: u32 = 0x6041;
    /// `op::RECEIVE_FROM`: `[endpoint 18][length u32]` then data.
    pub const RECEIVE_FROM: u32 = 0x6042;
    /// `op::ACCEPT`: `[socket id u32][peer endpoint 18]`.
    pub const ACCEPT: u32 = 0x6043;
    /// `op::ENDPOINTS`: `[local endpoint 18][peer endpoint 18]`.
    pub const ENDPOINTS: u32 = 0x6044;
    pub const ENDPOINT_BYTES: usize = 18;
    pub const DATAGRAM_HEADER: usize = 22;
    /// Addresses one AAAA answer carries.
    pub const MAX_ADDRESSES: usize = 8;

    /// `::ffff:a.b.c.d`.
    pub const fn mapped(v4: [u8; 4]) -> [u8; 16] {
        [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, v4[0], v4[1], v4[2], v4[3],
        ]
    }
    /// The IPv4 address an IPv4-mapped address names.
    pub fn unmapped(a: &[u8; 16]) -> Option<[u8; 4]> {
        (a[..10] == [0; 10] && a[10] == 0xff && a[11] == 0xff).then(|| [a[12], a[13], a[14], a[15]])
    }
    pub fn encode_endpoint(address: [u8; 16], port: u16, out: &mut [u8]) {
        out[..16].copy_from_slice(&address);
        out[16..18].copy_from_slice(&port.to_le_bytes());
    }
    pub fn decode_endpoint(b: &[u8]) -> Option<([u8; 16], u16)> {
        Some((
            b.get(..16)?.try_into().ok()?,
            u16::from_le_bytes([*b.get(16)?, *b.get(17)?]),
        ))
    }

    /// The IPv6 side of the adapter (RFC 4862 autoconfiguration).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Status6 {
        /// fe80::/64 from the MAC (unspecified while the adapter is off).
        pub link_local: [u8; 16],
        /// The address autoconfigured from a router's prefix (zeros: none).
        pub global: [u8; 16],
        pub prefix: u8,
        /// The default router (its link-local address; zeros: none).
        pub router: [u8; 16],
        /// RDNSS servers the router advertised.
        pub dns: [[u8; 16]; 2],
        /// The interface index Windows shows after `%` (scope id).
        pub scope: u32,
    }
    pub const STATUS_BYTES: usize = 16 * 5 + 1 + 4;
    impl Status6 {
        pub fn encode(&self) -> [u8; STATUS_BYTES] {
            let mut b = [0u8; STATUS_BYTES];
            b[..16].copy_from_slice(&self.link_local);
            b[16..32].copy_from_slice(&self.global);
            b[32..48].copy_from_slice(&self.router);
            b[48..64].copy_from_slice(&self.dns[0]);
            b[64..80].copy_from_slice(&self.dns[1]);
            b[80] = self.prefix;
            b[81..85].copy_from_slice(&self.scope.to_le_bytes());
            b
        }
        pub fn decode(b: &[u8]) -> Option<Self> {
            if b.len() != STATUS_BYTES || b[80] > 128 {
                return None;
            }
            let a = |at: usize| -> [u8; 16] { b[at..at + 16].try_into().unwrap() };
            Some(Self {
                link_local: a(0),
                global: a(16),
                router: a(32),
                dns: [a(48), a(64)],
                prefix: b[80],
                scope: u32::from_le_bytes(b[81..85].try_into().ok()?),
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn status_and_endpoints_round_trip() {
            let s = Status6 {
                link_local: [
                    0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0x50, 0x54, 0, 0xff, 0xfe, 0xd0, 5, 1,
                ],
                global: [
                    0xfe, 0xc0, 0, 0, 0, 0, 0, 0, 0x50, 0x54, 0, 0xff, 0xfe, 0xd0, 5, 1,
                ],
                prefix: 64,
                router: [0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2],
                dns: [[0; 16]; 2],
                scope: 4,
            };
            assert_eq!(Status6::decode(&s.encode()), Some(s));
            let mut b = [0u8; ENDPOINT_BYTES];
            encode_endpoint(mapped([10, 0, 2, 2]), 7, &mut b);
            let (a, p) = decode_endpoint(&b).unwrap();
            assert_eq!((unmapped(&a), p), (Some([10, 0, 2, 2]), 7));
            assert_eq!(unmapped(&s.global), None);
        }
    }
}
/// Protocol statistics and the neighbour (ARP / neighbour discovery) table
/// (GetIpStatisticsEx, GetTcpStatisticsEx, GetIcmpStatisticsEx,
/// GetIpNetTable2, netstat -s, arp -a).
pub mod stats {
    /// The encoded counters (`Counters`, `COUNTERS_BYTES`), a part at a
    /// time (a reply carries at most 240 bytes): the body is the byte
    /// offset (u16), the reply up to `STATISTICS_CHUNK` bytes from it.
    pub const STATISTICS: u32 = 0x6033;
    pub const STATISTICS_CHUNK: usize = 200;
    /// The neighbour table (body: first index u16): `[total u16][count u16]`
    /// then `count` `Neighbor`s.
    pub const NEIGHBORS: u32 = 0x6034;
    /// Records one NEIGHBORS reply carries (4 + 9 * 24 bytes).
    pub const PAGE: usize = 9;
    pub const NEIGHBOR_BYTES: usize = 24;
    /// MIB_IPNET_TYPE: learned from traffic, or fixed (broadcast, multicast).
    pub const NEIGHBOR_DYNAMIC: u8 = 3;
    pub const NEIGHBOR_STATIC: u8 = 4;
    /// ICMP message types counted one by one: ICMPv4 types 0..24 in their
    /// own slot; ICMPv6 errors 1..=4 likewise and informational types
    /// 128..=146 in slots 5..=23.
    pub const ICMP_SLOTS: usize = 24;
    pub const fn icmp_slot(v6: bool, kind: u8) -> Option<usize> {
        match (v6, kind) {
            (false, 0..=23) => Some(kind as usize),
            (true, 1..=4) => Some(kind as usize),
            (true, 128..=146) => Some(kind as usize - 123),
            _ => None,
        }
    }

    /// One address family's MIB-II counters, as Windows reports them.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Family {
        pub ip_in_receives: u32,
        pub ip_in_header_errors: u32,
        pub ip_in_unknown_protos: u32,
        pub ip_in_delivers: u32,
        pub ip_out_requests: u32,
        pub ip_reasm_reqds: u32,
        pub ip_frag_creates: u32,
        pub icmp_in_msgs: u32,
        pub icmp_in_errors: u32,
        pub icmp_out_msgs: u32,
        pub icmp_out_errors: u32,
        /// Messages by type (`icmp_slot`).
        pub icmp_in: [u32; ICMP_SLOTS],
        pub icmp_out: [u32; ICMP_SLOTS],
        pub tcp_active_opens: u32,
        pub tcp_passive_opens: u32,
        pub tcp_attempt_fails: u32,
        pub tcp_estab_resets: u32,
        pub tcp_curr_estab: u32,
        pub tcp_in_segs: u32,
        pub tcp_out_segs: u32,
        pub tcp_retrans_segs: u32,
        pub tcp_in_errs: u32,
        pub tcp_out_rsts: u32,
        pub tcp_num_conns: u32,
        pub udp_in_datagrams: u32,
        pub udp_no_ports: u32,
        pub udp_in_errors: u32,
        pub udp_out_datagrams: u32,
        pub udp_num_addrs: u32,
    }
    pub const FAMILY_WORDS: usize = 11 + 2 * ICMP_SLOTS + 16;
    pub const COUNTERS_BYTES: usize = 2 * 4 * FAMILY_WORDS;
    impl Family {
        fn words(&self) -> [u32; FAMILY_WORDS] {
            let mut w = [0u32; FAMILY_WORDS];
            w[..11].copy_from_slice(&[
                self.ip_in_receives,
                self.ip_in_header_errors,
                self.ip_in_unknown_protos,
                self.ip_in_delivers,
                self.ip_out_requests,
                self.ip_reasm_reqds,
                self.ip_frag_creates,
                self.icmp_in_msgs,
                self.icmp_in_errors,
                self.icmp_out_msgs,
                self.icmp_out_errors,
            ]);
            w[11..11 + ICMP_SLOTS].copy_from_slice(&self.icmp_in);
            w[11 + ICMP_SLOTS..11 + 2 * ICMP_SLOTS].copy_from_slice(&self.icmp_out);
            w[11 + 2 * ICMP_SLOTS..].copy_from_slice(&[
                self.tcp_active_opens,
                self.tcp_passive_opens,
                self.tcp_attempt_fails,
                self.tcp_estab_resets,
                self.tcp_curr_estab,
                self.tcp_in_segs,
                self.tcp_out_segs,
                self.tcp_retrans_segs,
                self.tcp_in_errs,
                self.tcp_out_rsts,
                self.tcp_num_conns,
                self.udp_in_datagrams,
                self.udp_no_ports,
                self.udp_in_errors,
                self.udp_out_datagrams,
                self.udp_num_addrs,
            ]);
            w
        }
        fn from_words(w: &[u32; FAMILY_WORDS]) -> Self {
            let t = 11 + 2 * ICMP_SLOTS;
            Self {
                ip_in_receives: w[0],
                ip_in_header_errors: w[1],
                ip_in_unknown_protos: w[2],
                ip_in_delivers: w[3],
                ip_out_requests: w[4],
                ip_reasm_reqds: w[5],
                ip_frag_creates: w[6],
                icmp_in_msgs: w[7],
                icmp_in_errors: w[8],
                icmp_out_msgs: w[9],
                icmp_out_errors: w[10],
                icmp_in: w[11..11 + ICMP_SLOTS].try_into().unwrap(),
                icmp_out: w[11 + ICMP_SLOTS..t].try_into().unwrap(),
                tcp_active_opens: w[t],
                tcp_passive_opens: w[t + 1],
                tcp_attempt_fails: w[t + 2],
                tcp_estab_resets: w[t + 3],
                tcp_curr_estab: w[t + 4],
                tcp_in_segs: w[t + 5],
                tcp_out_segs: w[t + 6],
                tcp_retrans_segs: w[t + 7],
                tcp_in_errs: w[t + 8],
                tcp_out_rsts: w[t + 9],
                tcp_num_conns: w[t + 10],
                udp_in_datagrams: w[t + 11],
                udp_no_ports: w[t + 12],
                udp_in_errors: w[t + 13],
                udp_out_datagrams: w[t + 14],
                udp_num_addrs: w[t + 15],
            }
        }
        /// A message type's count (`icmp_slot`), 0 for types not kept.
        pub fn icmp(&self, v6: bool, outgoing: bool, kind: u8) -> u32 {
            let side = if outgoing {
                &self.icmp_out
            } else {
                &self.icmp_in
            };
            icmp_slot(v6, kind).map_or(0, |slot| side[slot])
        }
    }
    /// IPv4 and IPv6 counters.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Counters {
        pub v4: Family,
        pub v6: Family,
    }
    impl Counters {
        pub fn encode(&self) -> [u8; COUNTERS_BYTES] {
            let mut b = [0u8; COUNTERS_BYTES];
            let words = self.v4.words().into_iter().chain(self.v6.words());
            for (chunk, word) in b.chunks_exact_mut(4).zip(words) {
                chunk.copy_from_slice(&word.to_le_bytes());
            }
            b
        }
        pub fn decode(b: &[u8]) -> Option<Self> {
            if b.len() != COUNTERS_BYTES {
                return None;
            }
            let word = |i: usize| u32::from_le_bytes(b[4 * i..4 * i + 4].try_into().unwrap());
            let v4: [u32; FAMILY_WORDS] = core::array::from_fn(word);
            let v6: [u32; FAMILY_WORDS] = core::array::from_fn(|i| word(FAMILY_WORDS + i));
            Some(Self {
                v4: Family::from_words(&v4),
                v6: Family::from_words(&v6),
            })
        }
    }
    /// One neighbour: its address (IPv4 in the mapped form `::ffff:a.b.c.d`),
    /// link-layer address, MIB_IPNET_TYPE and whether it is a router.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct Neighbor {
        pub address: [u8; 16],
        pub mac: [u8; 6],
        pub kind: u8,
        pub router: bool,
    }
    impl Neighbor {
        pub fn v4(address: [u8; 4], mac: [u8; 6], kind: u8) -> Self {
            Self {
                address: super::v6::mapped(address),
                mac,
                kind,
                router: false,
            }
        }
        /// The IPv4 address of an IPv4 neighbour.
        pub fn ipv4(&self) -> Option<[u8; 4]> {
            super::v6::unmapped(&self.address)
        }
        pub fn encode(&self, b: &mut [u8]) {
            b[..16].copy_from_slice(&self.address);
            b[16..22].copy_from_slice(&self.mac);
            b[22] = self.kind;
            b[23] = u8::from(self.router);
        }
        pub fn decode(b: &[u8]) -> Option<Self> {
            if b.len() != NEIGHBOR_BYTES
                || !matches!(b[22], NEIGHBOR_DYNAMIC | NEIGHBOR_STATIC)
                || b[23] > 1
            {
                return None;
            }
            Some(Self {
                address: b[..16].try_into().ok()?,
                mac: b[16..22].try_into().ok()?,
                kind: b[22],
                router: b[23] == 1,
            })
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn counters_and_neighbors_round_trip() {
            let mut c = Counters::default();
            c.v4.ip_in_receives = 1;
            c.v4.tcp_out_rsts = 9;
            c.v6.udp_num_addrs = 3;
            c.v4.icmp_in[8] = 4;
            c.v6.icmp_out[icmp_slot(true, 135).unwrap()] = 5;
            assert_eq!(Counters::decode(&c.encode()), Some(c));
            assert_eq!(c.v6.icmp(true, true, 135), 5);
            assert_eq!(c.v4.icmp(false, false, 8), 4);
            assert_eq!(icmp_slot(true, 147), None);
            let n = Neighbor::v4([10, 0, 2, 2], [0x52, 0x55, 10, 0, 2, 2], NEIGHBOR_DYNAMIC);
            let mut b = [0u8; NEIGHBOR_BYTES];
            n.encode(&mut b);
            assert_eq!(Neighbor::decode(&b), Some(n));
            assert_eq!(n.ipv4(), Some([10, 0, 2, 2]));
            b[23] = 2;
            assert_eq!(Neighbor::decode(&b), None);
        }
    }
}
