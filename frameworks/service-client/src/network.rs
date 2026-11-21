//! door.network client (P15): the adapter's status and configuration,
//! host-name resolution and TCP/UDP sockets through door-network.
use crate::{Code, Error, Result, rpc::Connection};
use door_abi::Handle;
use door_service_protocol::net::{self, Config, SocketState, op};

/// An IPv4 address and port.
pub type Endpoint = ([u8; 4], u16);
pub struct Network {
    connection: Connection,
}
/// Addresses one RESOLVED answers (at most eight).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Addresses {
    pub list: [[u8; 4]; 8],
    pub count: usize,
}
impl Network {
    pub fn connect(registry: Handle) -> Result<Self> {
        Ok(Self {
            connection: crate::connect(registry, door_abi::platform::manifest::NETWORK)?,
        })
    }
    /// Connect through a port already resolved (`Init::begin_resolve`); the
    /// caller keeps and closes `port`.
    pub fn connect_port(port: Handle) -> Result<Self> {
        Ok(Self {
            connection: Connection::connect(port, door_abi::platform::manifest::NETWORK)?,
        })
    }
    /// A fatal transport error retired the channel: connect again.
    pub fn needs_reconnect(&self) -> bool {
        self.connection.needs_reconnect()
    }
    pub fn status(&mut self) -> Result<net::Status> {
        let reply = self.connection.call(op::STATUS, &[], &[])?;
        net::Status::decode(reply.data()?).ok_or(Error::Malformed)
    }
    /// Settings only (Network & internet).
    pub fn configure(&mut self, config: &Config) -> Result<()> {
        self.connection
            .call(op::CONFIGURE, &config.encode(), &[])?
            .empty()
    }
    /// Start resolving `name`: the query id, and the address at once when
    /// `name` is a dotted IPv4 address.
    pub fn resolve(&mut self, name: &str) -> Result<(u32, Option<[u8; 4]>)> {
        let reply = self.connection.call(op::RESOLVE, name.as_bytes(), &[])?;
        let data = reply.data()?;
        match data.len() {
            8 => Ok((u64::from_le_bytes(data.try_into().unwrap()) as u32, None)),
            12 => Ok((
                u64::from_le_bytes(data[..8].try_into().unwrap()) as u32,
                Some(data[8..12].try_into().unwrap()),
            )),
            _ => Err(Error::Malformed),
        }
    }
    /// Start an ICMP echo: its token (`ping_result`).
    pub fn ping_start(&mut self, request: &net::EchoRequest) -> Result<u32> {
        let mut bytes = [0; 12 + net::PATTERN_BYTES];
        let len = request.encode(&mut bytes);
        Ok(self
            .connection
            .call(net::diag::PING, &bytes[..len], &[])?
            .scalar()? as u32)
    }
    /// An echo's outcome; `Ok(None)` while it waits.
    pub fn ping_result(&mut self, token: u32) -> Result<Option<net::Echo>> {
        match self
            .connection
            .call(net::diag::PING_RESULT, &token.to_le_bytes(), &[])
        {
            Ok(reply) => net::Echo::decode(reply.data()?)
                .map(Some)
                .ok_or(Error::Malformed),
            Err(Error::Service(Code::WouldBlock)) => Ok(None),
            Err(e) => Err(e),
        }
    }
    /// One ICMP echo, waiting for its outcome (the service times it out).
    pub fn ping(&mut self, request: &net::EchoRequest) -> Result<net::Echo> {
        let token = self.ping_start(request)?;
        loop {
            if let Some(echo) = self.ping_result(token)? {
                return Ok(echo);
            }
            let _ = door_sys::time().and_then(|t| door_sys::sleep(t + 1));
        }
    }
    /// One page of the stack's TCP and UDP sockets from index `first`:
    /// (the total, how many were written into `out`).
    pub fn connections(
        &mut self,
        first: u16,
        out: &mut [net::Connection; net::diag::PAGE],
    ) -> Result<(usize, usize)> {
        let reply = self
            .connection
            .call(net::diag::CONNECTIONS, &first.to_le_bytes(), &[])?;
        let data = reply.data()?;
        if data.len() < 4 {
            return Err(Error::Malformed);
        }
        let total = usize::from(u16::from_le_bytes([data[0], data[1]]));
        let count = usize::from(u16::from_le_bytes([data[2], data[3]]));
        if count > net::diag::PAGE || data.len() != 4 + count * net::CONNECTION_BYTES {
            return Err(Error::Malformed);
        }
        for (slot, record) in out
            .iter_mut()
            .zip(data[4..].chunks_exact(net::CONNECTION_BYTES))
        {
            *slot = net::Connection::decode(record).ok_or(Error::Malformed)?;
        }
        Ok((total, count))
    }
    /// One page of the AF_INET6 sockets from index `first`: (total, count).
    pub fn connections6(
        &mut self,
        first: u16,
        out: &mut [net::Connection6; net::diag::PAGE6],
    ) -> Result<(usize, usize)> {
        let reply = self
            .connection
            .call(net::diag::CONNECTIONS6, &first.to_le_bytes(), &[])?;
        let data = reply.data()?;
        if data.len() < 4 {
            return Err(Error::Malformed);
        }
        let total = usize::from(u16::from_le_bytes([data[0], data[1]]));
        let count = usize::from(u16::from_le_bytes([data[2], data[3]]));
        if count > net::diag::PAGE6 || data.len() != 4 + count * net::CONNECTION6_BYTES {
            return Err(Error::Malformed);
        }
        for (slot, record) in out
            .iter_mut()
            .zip(data[4..].chunks_exact(net::CONNECTION6_BYTES))
        {
            *slot = net::Connection6::decode(record).ok_or(Error::Malformed)?;
        }
        Ok((total, count))
    }
    /// The IPv4 protocol counters (GetIpStatistics, netstat -s).
    pub fn statistics(&mut self) -> Result<net::stats::Counters> {
        use net::stats::{COUNTERS_BYTES, STATISTICS_CHUNK};
        let mut all = [0u8; COUNTERS_BYTES];
        let mut offset = 0;
        while offset < COUNTERS_BYTES {
            let reply = self.connection.call(
                net::stats::STATISTICS,
                &(offset as u16).to_le_bytes(),
                &[],
            )?;
            let data = reply.data()?;
            let want = STATISTICS_CHUNK.min(COUNTERS_BYTES - offset);
            if data.len() != want {
                return Err(Error::Malformed);
            }
            all[offset..offset + want].copy_from_slice(data);
            offset += want;
        }
        net::stats::Counters::decode(&all).ok_or(Error::Malformed)
    }
    /// One page of the ARP table from record `first`: (total, filled).
    pub fn neighbors(
        &mut self,
        first: u16,
        out: &mut [net::stats::Neighbor; net::stats::PAGE],
    ) -> Result<(usize, usize)> {
        let reply = self
            .connection
            .call(net::stats::NEIGHBORS, &first.to_le_bytes(), &[])?;
        let data = reply.data()?;
        if data.len() < 4 {
            return Err(Error::Malformed);
        }
        let total = usize::from(u16::from_le_bytes([data[0], data[1]]));
        let count = usize::from(u16::from_le_bytes([data[2], data[3]]));
        if count > net::stats::PAGE || data.len() != 4 + count * net::stats::NEIGHBOR_BYTES {
            return Err(Error::Malformed);
        }
        for (slot, record) in out
            .iter_mut()
            .zip(data[4..].chunks_exact(net::stats::NEIGHBOR_BYTES))
        {
            *slot = net::stats::Neighbor::decode(record).ok_or(Error::Malformed)?;
        }
        Ok((total, count))
    }
    /// A query's addresses; `Ok(None)` while it is pending,
    /// `Err(Service(NotFound))` when the name does not resolve.
    pub fn resolved(&mut self, id: u32) -> Result<Option<Addresses>> {
        match self.connection.call(op::RESOLVED, &id.to_le_bytes(), &[]) {
            Ok(reply) => {
                let data = reply.data()?;
                if data.len() % 4 != 0 || data.len() > 32 {
                    return Err(Error::Malformed);
                }
                let mut out = Addresses::default();
                for (slot, chunk) in out.list.iter_mut().zip(data.chunks_exact(4)) {
                    *slot = chunk.try_into().unwrap();
                    out.count += 1;
                }
                Ok(Some(out))
            }
            Err(Error::Service(door_service_protocol::Code::WouldBlock)) => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub fn socket(&mut self, kind: u8) -> Result<u32> {
        Ok(self.connection.call(op::SOCKET, &[kind], &[])?.scalar()? as u32)
    }
    /// An AF_INET6 (or dual-stack) socket: the IPv6 tables list it.
    pub fn socket6(&mut self, kind: u8) -> Result<u32> {
        Ok(self
            .connection
            .call(op::SOCKET, &[kind, op::FAMILY_INET6], &[])?
            .scalar()? as u32)
    }
    /// A raw socket of IP `protocol` (administrators only): IPv4, or IPv6
    /// when `v6`.
    pub fn socket_raw(&mut self, v6: bool, protocol: u8) -> Result<u32> {
        let family = if v6 { op::FAMILY_INET6 } else { 2 };
        Ok(self
            .connection
            .call(op::SOCKET, &[net::RAW, family, protocol], &[])?
            .scalar()? as u32)
    }
    /// Join (`op::MULTICAST_JOIN`) or leave a multicast group, `group` in
    /// the `net::v6` form.
    pub fn multicast(
        &mut self,
        id: u32,
        action: u8,
        group: [u8; 16],
        interface: u32,
    ) -> Result<()> {
        let mut body = [0u8; op::MULTICAST_BYTES];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4] = action;
        body[5..21].copy_from_slice(&group);
        body[21..25].copy_from_slice(&interface.to_le_bytes());
        self.connection.call(op::MULTICAST, &body, &[])?.empty()
    }
    pub fn connect_to(&mut self, id: u32, address: [u8; 4], port: u16) -> Result<()> {
        let mut body = [0u8; 10];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4..8].copy_from_slice(&address);
        body[8..10].copy_from_slice(&port.to_le_bytes());
        self.connection.call(op::CONNECT, &body, &[])?.empty()
    }
    pub fn listen(&mut self, id: u32, port: u16) -> Result<()> {
        self.listen_backlog(id, port, 1)
    }
    /// Listen with up to `backlog` connections waiting for ACCEPT (port 0:
    /// the bound one).
    pub fn listen_backlog(&mut self, id: u32, port: u16, backlog: u8) -> Result<()> {
        let mut body = [0u8; 7];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4..6].copy_from_slice(&port.to_le_bytes());
        body[6] = backlog;
        self.connection.call(op::LISTEN, &body, &[])?.empty()
    }
    /// Bind to `port` (0: an ephemeral one); the bound port.
    pub fn bind(&mut self, id: u32, port: u16) -> Result<u16> {
        let mut body = [0u8; 6];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4..6].copy_from_slice(&port.to_le_bytes());
        Ok(self.connection.call(op::BIND, &body, &[])?.scalar()? as u16)
    }
    /// `bind` to one of this host's addresses (`net::v6` form).
    pub fn bind_address(&mut self, id: u32, address: [u8; 16], port: u16) -> Result<u16> {
        let mut body = [0u8; 22];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4..6].copy_from_slice(&port.to_le_bytes());
        body[6..].copy_from_slice(&address);
        Ok(self.connection.call(op::BIND, &body, &[])?.scalar()? as u16)
    }
    /// A waiting connection of a listener: its socket id and the peer;
    /// `Err(Service(WouldBlock))` while none waits.
    pub fn accept(&mut self, id: u32) -> Result<(u32, [u8; 4], u16)> {
        let reply = self.connection.call(op::ACCEPT, &id.to_le_bytes(), &[])?;
        let data = reply.data()?;
        if data.len() != 10 {
            return Err(Error::Malformed);
        }
        Ok((
            u32::from_le_bytes(data[..4].try_into().unwrap()),
            data[4..8].try_into().unwrap(),
            u16::from_le_bytes([data[8], data[9]]),
        ))
    }
    /// UDP: send one datagram of up to `net::CHUNK` bytes.
    /// A datagram over `CHUNK` bytes goes in staged parts, the last one
    /// with SEND_TO.
    pub fn send_to(&mut self, id: u32, address: [u8; 4], port: u16, data: &[u8]) -> Result<usize> {
        self.send_to_with(id, address, port, data.len(), |offset, part| {
            part.copy_from_slice(&data[offset..offset + part.len()]);
            Ok(())
        })
    }
    /// `send_to` for a datagram of `length` bytes that `fill` produces
    /// part by part (`fill(offset, part)`), so no caller needs a buffer
    /// for the whole datagram.
    pub fn send_to_with(
        &mut self,
        id: u32,
        address: [u8; 4],
        port: u16,
        length: usize,
        mut fill: impl FnMut(usize, &mut [u8]) -> Result<()>,
    ) -> Result<usize> {
        if length > net::MAX_DATAGRAM {
            return Err(Error::Service(Code::InvalidRequest));
        }
        // Every part but the last is staged; the last one sends.
        let last = if length > net::CHUNK {
            (length - 1) / net::CHUNK * net::CHUNK
        } else {
            0
        };
        let mut offset = 0;
        while offset < last {
            let n = net::CHUNK;
            let mut body = [0u8; 8 + net::CHUNK];
            body[..4].copy_from_slice(&id.to_le_bytes());
            body[4..8].copy_from_slice(&(offset as u32).to_le_bytes());
            fill(offset, &mut body[8..8 + n])?;
            self.connection
                .call(op::SEND_PART, &body[..8 + n], &[])?
                .scalar()?;
            offset += n;
        }
        let n = length - last;
        let mut body = [0u8; 10 + net::CHUNK];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4..8].copy_from_slice(&address);
        body[8..10].copy_from_slice(&port.to_le_bytes());
        fill(last, &mut body[10..10 + n])?;
        Ok(self
            .connection
            .call(op::SEND_TO, &body[..10 + n], &[])?
            .scalar()? as usize)
    }
    /// UDP: one datagram into `out` with its sender: `(length, address,
    /// port)`, where `length` is the whole datagram (more than
    /// `out.len()` when it was cut to fit); None while none waits.
    pub fn receive_from(
        &mut self,
        id: u32,
        out: &mut [u8],
    ) -> Result<Option<(usize, [u8; 4], u16)>> {
        let max = out.len();
        self.receive_from_with(id, max, |offset, part| {
            out[offset..offset + part.len()].copy_from_slice(part);
            Ok(())
        })
    }
    /// `receive_from` delivering the first `max` bytes of the datagram
    /// part by part to `sink(offset, part)`.
    pub fn receive_from_with(
        &mut self,
        id: u32,
        max: usize,
        mut sink: impl FnMut(usize, &[u8]) -> Result<()>,
    ) -> Result<Option<(usize, [u8; 4], u16)>> {
        let mut body = [0u8; 8];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4..8].copy_from_slice(&(max.min(net::MAX_DATAGRAM) as u32).to_le_bytes());
        let reply = self.connection.call(op::RECEIVE_FROM, &body, &[])?;
        let data = reply.data()?;
        if data.is_empty() {
            return Ok(None);
        }
        if data.len() < net::DATAGRAM_HEADER {
            return Err(Error::Malformed);
        }
        let length = u32::from_le_bytes(data[6..10].try_into().unwrap()) as usize;
        let sender: [u8; 4] = data[..4].try_into().unwrap();
        let port = u16::from_le_bytes([data[4], data[5]]);
        let wanted = length.min(max);
        let first = (data.len() - net::DATAGRAM_HEADER).min(wanted);
        sink(0, &data[net::DATAGRAM_HEADER..net::DATAGRAM_HEADER + first])?;
        let mut copied = first;
        while copied < wanted {
            let mut more = [0u8; 8];
            more[..4].copy_from_slice(&id.to_le_bytes());
            more[4..8].copy_from_slice(&(copied as u32).to_le_bytes());
            let reply = self.connection.call(op::RECEIVE_MORE, &more, &[])?;
            let part = reply.data()?;
            if part.is_empty() {
                return Err(Error::Malformed);
            }
            let n = part.len().min(wanted - copied);
            sink(copied, &part[..n])?;
            copied += n;
        }
        Ok(Some((length, sender, port)))
    }
    /// The local and peer endpoints (zeros where not known yet).
    pub fn endpoints(&mut self, id: u32) -> Result<(Endpoint, Endpoint)> {
        let reply = self
            .connection
            .call(op::ENDPOINTS, &id.to_le_bytes(), &[])?;
        let d = reply.data()?;
        if d.len() != net::ENDPOINTS_BYTES {
            return Err(Error::Malformed);
        }
        Ok((
            (d[..4].try_into().unwrap(), u16::from_le_bytes([d[4], d[5]])),
            (
                d[6..10].try_into().unwrap(),
                u16::from_le_bytes([d[10], d[11]]),
            ),
        ))
    }
    /// Set a socket option (`net::option`).
    pub fn option(&mut self, id: u32, option: u8, value: u32) -> Result<()> {
        let mut body = [0u8; 9];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4] = option;
        body[5..9].copy_from_slice(&value.to_le_bytes());
        self.connection.call(op::OPTION, &body, &[])?.empty()
    }
    /// Queue up to `net::CHUNK` bytes; how many were taken.
    pub fn send(&mut self, id: u32, data: &[u8]) -> Result<usize> {
        let n = data.len().min(net::CHUNK);
        let mut body = [0u8; 4 + net::CHUNK];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4..4 + n].copy_from_slice(&data[..n]);
        Ok(self
            .connection
            .call(op::SEND, &body[..4 + n], &[])?
            .scalar()? as usize)
    }
    /// Received bytes into `out` (0 when none yet); `Err(Service(Closing))`
    /// once the peer finished and everything was read.
    pub fn receive(&mut self, id: u32, out: &mut [u8]) -> Result<usize> {
        let mut body = [0u8; 8];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4..8].copy_from_slice(&(out.len().min(net::CHUNK) as u32).to_le_bytes());
        let reply = self.connection.call(op::RECEIVE, &body, &[])?;
        let data = reply.data()?;
        let n = data.len().min(out.len());
        out[..n].copy_from_slice(&data[..n]);
        Ok(n)
    }
    pub fn shutdown(&mut self, id: u32) -> Result<()> {
        self.connection
            .call(op::SHUTDOWN, &id.to_le_bytes(), &[])?
            .empty()
    }
    pub fn close(&mut self, id: u32) -> Result<()> {
        self.connection
            .call(op::CLOSE, &id.to_le_bytes(), &[])?
            .empty()
    }
    /// The socket's state and its queued receive and send bytes.
    pub fn state(&mut self, id: u32) -> Result<(SocketState, u32, u32)> {
        let reply = self.connection.call(op::STATE, &id.to_le_bytes(), &[])?;
        let data = reply.data()?;
        if data.len() != 12 {
            return Err(Error::Malformed);
        }
        let state = match data[0] {
            0 => SocketState::Closed,
            1 => SocketState::Connecting,
            2 => SocketState::Established,
            3 => SocketState::PeerClosed,
            4 => SocketState::Closing,
            5 => SocketState::Listening,
            6 => SocketState::Failed,
            _ => return Err(Error::Malformed),
        };
        Ok((
            state,
            u32::from_le_bytes(data[4..8].try_into().unwrap()),
            u32::from_le_bytes(data[8..12].try_into().unwrap()),
        ))
    }
}

/// VPN connections (`net::vpn`): `uid` is the signed-in user (Settings'
/// own; the window server names its session's user).
impl Network {
    /// Save connection `name` from its wg-quick `text` (replacing one of
    /// that name). `Service(InvalidRequest)`: the text or name is invalid.
    pub fn vpn_save(&mut self, uid: u32, name: &str, text: &str) -> Result<()> {
        use net::vpn;
        if text.len() > vpn::MAX_TEXT {
            return Err(Error::Service(Code::InvalidRequest));
        }
        for (i, part) in text.as_bytes().chunks(vpn::PART).enumerate() {
            let mut rest = [0u8; 4 + vpn::PART];
            rest[..4].copy_from_slice(&((i * vpn::PART) as u32).to_le_bytes());
            rest[4..4 + part.len()].copy_from_slice(part);
            let (body, len) = vpn::body(uid, &rest[..4 + part.len()]);
            self.connection
                .call(vpn::STAGE, &body[..len], &[])?
                .scalar()?;
        }
        if text.is_empty() {
            let (body, len) = vpn::body(uid, &0u32.to_le_bytes());
            self.connection
                .call(vpn::STAGE, &body[..len], &[])?
                .scalar()?;
        }
        self.vpn_named(net::vpn::SAVE, uid, name)
    }
    pub fn vpn_remove(&mut self, uid: u32, name: &str) -> Result<()> {
        self.vpn_named(net::vpn::REMOVE, uid, name)
    }
    pub fn vpn_connect(&mut self, uid: u32, name: &str) -> Result<()> {
        self.vpn_named(net::vpn::CONNECT, uid, name)
    }
    pub fn vpn_disconnect(&mut self, uid: u32, name: &str) -> Result<()> {
        self.vpn_named(net::vpn::DISCONNECT, uid, name)
    }
    fn vpn_named(&mut self, opcode: u32, uid: u32, name: &str) -> Result<()> {
        if name.len() > net::vpn::MAX_NAME {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let (body, len) = net::vpn::body(uid, name.as_bytes());
        self.connection.call(opcode, &body[..len], &[])?.empty()
    }
    /// The tunnel adapter, while a connection is up (any process).
    pub fn vpn_tunnel(&mut self) -> Result<Option<net::vpn::Tunnel>> {
        let reply = self.connection.call(net::vpn::TUNNEL, &[], &[])?;
        let data = reply.data()?;
        if data.is_empty() {
            return Ok(None);
        }
        net::vpn::Tunnel::decode(data)
            .map(Some)
            .ok_or(Error::Malformed)
    }
    /// The user's connection `index`; `None` past the last one.
    pub fn vpn_entry(&mut self, uid: u32, index: u8) -> Result<Option<net::vpn::Entry>> {
        let (body, len) = net::vpn::body(uid, &[index]);
        match self.connection.call(net::vpn::ENTRY, &body[..len], &[]) {
            Ok(reply) => net::vpn::Entry::decode(reply.data()?)
                .map(Some)
                .ok_or(Error::Malformed),
            Err(Error::Service(Code::NotFound)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Windows Defender Firewall (`net::firewall`).
impl Network {
    /// `[domain, private, public]` profile bytes, the active profile bit,
    /// the rule count and the programs waiting for the Security Alert.
    pub fn firewall_state(&mut self) -> Result<([u8; 3], u8, u32, u32)> {
        let reply = self.connection.call(net::firewall::STATE, &[], &[])?;
        let d = reply.data()?;
        if d.len() != net::firewall::STATE_BYTES {
            return Err(Error::Malformed);
        }
        let word = |at: usize| u32::from_le_bytes(d[at..at + 4].try_into().unwrap());
        Ok(([d[0], d[1], d[2]], d[3], word(8), word(12)))
    }
    /// Init: rule text from `offset` (0 starts over), up to `PART` bytes.
    pub fn firewall_stage(&mut self, offset: u32, part: &[u8]) -> Result<()> {
        if part.len() > net::firewall::PART {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let mut body = [0u8; 4 + net::firewall::PART];
        body[..4].copy_from_slice(&offset.to_le_bytes());
        body[4..4 + part.len()].copy_from_slice(part);
        self.connection
            .call(net::firewall::STAGE, &body[..4 + part.len()], &[])?
            .scalar()
            .map(|_| ())
    }
    /// Init: the staged rules take effect with these profile bytes.
    pub fn firewall_apply(&mut self, profiles: [u8; 3]) -> Result<()> {
        self.connection
            .call(net::firewall::APPLY, &profiles, &[])?
            .empty()
    }
    /// Init: process `pid` runs the Windows program `path`.
    pub fn firewall_program(&mut self, pid: u64, path: &str) -> Result<()> {
        let mut body = [0u8; 8 + 520];
        if path.is_empty() || path.len() > 520 {
            return Err(Error::Service(Code::InvalidRequest));
        }
        body[..8].copy_from_slice(&pid.to_le_bytes());
        body[8..8 + path.len()].copy_from_slice(path.as_bytes());
        self.connection
            .call(net::firewall::PROGRAM, &body[..8 + path.len()], &[])?
            .empty()
    }
    /// The window server: the next program blocked from listening (its
    /// pid, and its path's length in `path`), None when none waits.
    pub fn firewall_alert(&mut self, path: &mut [u8]) -> Result<Option<(u64, usize)>> {
        let reply = self.connection.call(net::firewall::ALERT, &[], &[])?;
        let d = reply.data()?;
        if d.is_empty() {
            return Ok(None);
        }
        if d.len() < 9 {
            return Err(Error::Malformed);
        }
        let n = (d.len() - 8).min(path.len());
        path[..n].copy_from_slice(&d[8..8 + n]);
        Ok(Some((u64::from_le_bytes(d[..8].try_into().unwrap()), n)))
    }
}

/// IPv6 addresses one AAAA answer gives (at most eight).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Addresses6 {
    pub list: [[u8; 16]; 8],
    pub count: usize,
}
/// An address in `net::v6` form (IPv4 as `::ffff:a.b.c.d`) and a port.
pub type Endpoint6 = ([u8; 16], u16);
/// IPv6 and dual-stack requests (`net::v6`).
impl Network {
    pub fn status6(&mut self) -> Result<net::v6::Status6> {
        let reply = self.connection.call(net::v6::STATUS, &[], &[])?;
        net::v6::Status6::decode(reply.data()?).ok_or(Error::Malformed)
    }
    /// Start an AAAA query: its id for `resolved6`.
    pub fn resolve6(&mut self, name: &str) -> Result<u32> {
        Ok(self
            .connection
            .call(net::v6::RESOLVE, name.as_bytes(), &[])?
            .scalar()? as u32)
    }
    /// An AAAA query's addresses; `Ok(None)` while pending.
    pub fn resolved6(&mut self, id: u32) -> Result<Option<Addresses6>> {
        match self.connection.call(op::RESOLVED, &id.to_le_bytes(), &[]) {
            Ok(reply) => {
                let data = reply.data()?;
                if data.len() % 16 != 0 || data.len() > 16 * net::v6::MAX_ADDRESSES {
                    return Err(Error::Malformed);
                }
                let mut out = Addresses6::default();
                for (slot, chunk) in out.list.iter_mut().zip(data.chunks_exact(16)) {
                    *slot = chunk.try_into().unwrap();
                    out.count += 1;
                }
                Ok(Some(out))
            }
            Err(Error::Service(door_service_protocol::Code::WouldBlock)) => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub fn connect_to6(&mut self, id: u32, address: [u8; 16], port: u16) -> Result<()> {
        let mut body = [0u8; 4 + net::v6::ENDPOINT_BYTES];
        body[..4].copy_from_slice(&id.to_le_bytes());
        net::v6::encode_endpoint(address, port, &mut body[4..]);
        self.connection.call(net::v6::CONNECT, &body, &[])?.empty()
    }
    pub fn accept6(&mut self, id: u32) -> Result<(u32, [u8; 16], u16)> {
        let reply = self
            .connection
            .call(net::v6::ACCEPT, &id.to_le_bytes(), &[])?;
        let data = reply.data()?;
        if data.len() != 4 + net::v6::ENDPOINT_BYTES {
            return Err(Error::Malformed);
        }
        let (address, port) = net::v6::decode_endpoint(&data[4..]).ok_or(Error::Malformed)?;
        Ok((
            u32::from_le_bytes(data[..4].try_into().unwrap()),
            address,
            port,
        ))
    }
    pub fn endpoints6(&mut self, id: u32) -> Result<(Endpoint6, Endpoint6)> {
        let reply = self
            .connection
            .call(net::v6::ENDPOINTS, &id.to_le_bytes(), &[])?;
        let d = reply.data()?;
        if d.len() != 2 * net::v6::ENDPOINT_BYTES {
            return Err(Error::Malformed);
        }
        Ok((
            net::v6::decode_endpoint(&d[..18]).ok_or(Error::Malformed)?,
            net::v6::decode_endpoint(&d[18..]).ok_or(Error::Malformed)?,
        ))
    }
    /// `send_to_with` to an IPv6 (or IPv4-mapped) endpoint.
    pub fn send_to6_with(
        &mut self,
        id: u32,
        address: [u8; 16],
        port: u16,
        length: usize,
        mut fill: impl FnMut(usize, &mut [u8]) -> Result<()>,
    ) -> Result<usize> {
        if length > net::MAX_DATAGRAM {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let last = if length > net::CHUNK {
            (length - 1) / net::CHUNK * net::CHUNK
        } else {
            0
        };
        let mut offset = 0;
        while offset < last {
            let mut body = [0u8; 8 + net::CHUNK];
            body[..4].copy_from_slice(&id.to_le_bytes());
            body[4..8].copy_from_slice(&(offset as u32).to_le_bytes());
            fill(offset, &mut body[8..8 + net::CHUNK])?;
            self.connection.call(op::SEND_PART, &body, &[])?.scalar()?;
            offset += net::CHUNK;
        }
        let n = length - last;
        const HEADER: usize = 4 + net::v6::ENDPOINT_BYTES;
        let mut body = [0u8; HEADER + net::CHUNK];
        body[..4].copy_from_slice(&id.to_le_bytes());
        net::v6::encode_endpoint(address, port, &mut body[4..HEADER]);
        fill(last, &mut body[HEADER..HEADER + n])?;
        Ok(self
            .connection
            .call(net::v6::SEND_TO, &body[..HEADER + n], &[])?
            .scalar()? as usize)
    }
    /// `receive_from_with` with the sender in `net::v6` form.
    pub fn receive_from6_with(
        &mut self,
        id: u32,
        max: usize,
        mut sink: impl FnMut(usize, &[u8]) -> Result<()>,
    ) -> Result<Option<(usize, [u8; 16], u16)>> {
        let mut body = [0u8; 8];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4..8].copy_from_slice(&(max.min(net::MAX_DATAGRAM) as u32).to_le_bytes());
        let reply = self.connection.call(net::v6::RECEIVE_FROM, &body, &[])?;
        let data = reply.data()?;
        if data.is_empty() {
            return Ok(None);
        }
        const HEADER: usize = net::v6::DATAGRAM_HEADER;
        if data.len() < HEADER {
            return Err(Error::Malformed);
        }
        let (sender, port) = net::v6::decode_endpoint(data).ok_or(Error::Malformed)?;
        let length = u32::from_le_bytes(data[18..22].try_into().unwrap()) as usize;
        let wanted = length.min(max);
        let first = (data.len() - HEADER).min(wanted);
        sink(0, &data[HEADER..HEADER + first])?;
        let mut copied = first;
        while copied < wanted {
            let mut more = [0u8; 8];
            more[..4].copy_from_slice(&id.to_le_bytes());
            more[4..8].copy_from_slice(&(copied as u32).to_le_bytes());
            let reply = self.connection.call(op::RECEIVE_MORE, &more, &[])?;
            let part = reply.data()?;
            if part.is_empty() {
                return Err(Error::Malformed);
            }
            let n = part.len().min(wanted - copied);
            sink(copied, &part[..n])?;
            copied += n;
        }
        Ok(Some((length, sender, port)))
    }
}
