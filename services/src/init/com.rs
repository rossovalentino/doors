//! The COM activation broker and ncalrpc transport
//! (`door_service_protocol::com`): named endpoints of this session's
//! processes, links between a client and an endpoint's owner whose packets
//! Init carries, and LocalServer32 launch coordination. Everything a
//! process holds is keyed by its connection to Init and goes when that
//! connection does (the process ended or gave up on Init).
//!
//! Endpoints are scoped to their owner's identity: a process finds, connects
//! to and activates only endpoints of its own user at its own elevation (an
//! unelevated program never reaches an elevated server, nor answers an
//! elevated client in place of the real one), registers its object exporter
//! (`OLE<pid>`) only under its own process ID, and holds a bounded share of
//! the session's endpoints, links and packets.
use crate::server::Response;
use door_abi::{Handle, platform::Identity};
use door_service_protocol::{
    Code,
    com::{self as wire, ENDPOINTS, LINKS, MAILBOX, NAME_BYTES, PAYLOAD_BYTES, event},
};

/// Packets in flight across every link.
const POOL: usize = 24;
/// What one connection may hold: endpoints, links it opened, packets it
/// sent that wait in the pool.
const ENDPOINTS_EACH: usize = 4;
const LINKS_EACH: usize = 4;
const PACKETS_EACH: usize = 8;

/// Who may see an endpoint: the session, the user and whether elevated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Scope {
    session: u64,
    uid: u32,
    elevated: bool,
}
impl Scope {
    fn of(peer: &Identity) -> Self {
        Self {
            session: peer.session,
            uid: peer.uid,
            elevated: peer.groups & door_abi::platform::group::ADMINISTRATORS != 0,
        }
    }
}

#[derive(Clone, Copy)]
struct Endpoint {
    name: [u8; NAME_BYTES],
    len: usize,
    scope: Scope,
    channel: Handle,
}
impl Endpoint {
    fn name(&self) -> &[u8] {
        &self.name[..self.len]
    }
}
/// One side of a link: 0 is the client that connected, 1 the owner.
#[derive(Clone, Copy)]
struct Side {
    channel: Handle,
    open: bool,
    /// Pool indices of the packets waiting for this side, oldest first.
    queue: [u8; MAILBOX],
    count: usize,
}
#[derive(Clone, Copy)]
struct Link {
    id: u32,
    session: u64,
    sides: [Side; 2],
    /// The owner has not been told about the link yet.
    fresh: bool,
    endpoint: [u8; NAME_BYTES],
    len: usize,
}
#[derive(Clone, Copy)]
struct Packet {
    len: usize,
    data: [u8; PAYLOAD_BYTES],
    /// The connection that sent it (its share of the pool).
    from: Handle,
}
#[derive(Clone, Copy)]
struct Launch {
    scope: Scope,
    clsid: [u8; 16],
    until: u64,
}
#[derive(Default)]
pub(super) struct Broker {
    endpoints: [Option<Endpoint>; ENDPOINTS],
    links: [Option<Link>; LINKS],
    pool: [Option<Packet>; POOL],
    launches: [Option<Launch>; 4],
    last_id: u32,
    /// Where POLL starts looking, so one busy link cannot starve others.
    cursor: usize,
}
fn link_id(body: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(body.get(..4)?.try_into().ok()?))
}
impl Broker {
    fn find(&self, scope: Scope, name: &[u8]) -> Option<usize> {
        self.endpoints.iter().position(|e| {
            e.as_ref()
                .is_some_and(|e| e.scope == scope && e.name() == name)
        })
    }
    /// The link `id` of `session` and the caller's open side of it.
    fn side(&self, session: u64, channel: Handle, id: u32) -> Option<(usize, usize)> {
        self.links.iter().enumerate().find_map(|(i, l)| {
            let l = l.as_ref()?;
            if l.id != id || l.session != session {
                return None;
            }
            let s = l
                .sides
                .iter()
                .position(|s| s.open && s.channel == channel)?;
            Some((i, s))
        })
    }
    /// Close side `s` of link `i`: drop what waits for it, and the link
    /// once neither side is open.
    fn close(&mut self, i: usize, s: usize) {
        let Some(link) = self.links[i].as_mut() else {
            return;
        };
        let side = &mut link.sides[s];
        for &slot in &side.queue[..side.count] {
            self.pool[slot as usize] = None;
        }
        side.count = 0;
        side.open = false;
        if link.sides.iter().all(|s| !s.open) {
            self.links[i] = None;
        }
    }
    /// The connection `channel` went: its endpoints and link sides go too.
    pub(super) fn disconnected(&mut self, channel: Handle) {
        for e in self.endpoints.iter_mut() {
            if e.is_some_and(|e| e.channel == channel) {
                *e = None;
            }
        }
        for i in 0..LINKS {
            for s in 0..2 {
                if self.links[i].is_some_and(|l| l.sides[s].open && l.sides[s].channel == channel) {
                    self.close(i, s);
                }
            }
        }
    }
    pub(super) fn request(
        &mut self,
        channel: Handle,
        peer: Identity,
        operation: u32,
        body: &[u8],
        now: u64,
    ) -> Response {
        let session = peer.session;
        let scope = Scope::of(&peer);
        match operation {
            wire::REGISTER | wire::UNREGISTER | wire::CONNECT => {
                if !wire::valid_name(body) {
                    return Response::error(Code::InvalidRequest);
                }
                let found = self.find(scope, body);
                match operation {
                    wire::REGISTER => {
                        // An object exporter is registered by its own
                        // process only.
                        if !body.starts_with(b"OLECLASS-")
                            && body != &wire::exporter_endpoint(peer.pid).0[..]
                        {
                            return Response::error(Code::Denied);
                        }
                        self.register(channel, scope, body, found)
                    }
                    wire::UNREGISTER => match found {
                        Some(i) if self.endpoints[i].is_some_and(|e| e.channel == channel) => {
                            self.endpoints[i] = None;
                            Response::empty()
                        }
                        Some(_) => Response::error(Code::Denied),
                        None => Response::error(Code::NotFound),
                    },
                    _ => self.connect(channel, session, found),
                }
            }
            wire::SEND => {
                let (Some(id), Some(packet)) = (link_id(body), body.get(4..)) else {
                    return Response::error(Code::InvalidRequest);
                };
                if packet.is_empty() || packet.len() > PAYLOAD_BYTES {
                    return Response::error(Code::InvalidRequest);
                }
                let Some((i, s)) = self.side(session, channel, id) else {
                    return Response::error(Code::NotFound);
                };
                if self
                    .pool
                    .iter()
                    .flatten()
                    .filter(|p| p.from == channel)
                    .count()
                    >= PACKETS_EACH
                {
                    return Response::error(Code::WouldBlock);
                }
                self.send(i, 1 - s, packet, channel)
            }
            wire::POLL => {
                if !body.is_empty() {
                    return Response::error(Code::InvalidRequest);
                }
                self.poll(channel, session)
            }
            wire::CLOSE => {
                let Some(id) = link_id(body).filter(|_| body.len() == 4) else {
                    return Response::error(Code::InvalidRequest);
                };
                let Some((i, s)) = self.side(session, channel, id) else {
                    return Response::error(Code::NotFound);
                };
                self.close(i, s);
                Response::empty()
            }
            wire::ACTIVATE | wire::FAILED => {
                let Ok(clsid) = <[u8; 16]>::try_from(body) else {
                    return Response::error(Code::InvalidRequest);
                };
                self.activate(operation, scope, clsid, now)
            }
            _ => Response::error(Code::Unsupported),
        }
    }
    fn register(
        &mut self,
        channel: Handle,
        scope: Scope,
        name: &[u8],
        found: Option<usize>,
    ) -> Response {
        if found.is_some() {
            return Response::error(Code::Exists);
        }
        if self
            .endpoints
            .iter()
            .flatten()
            .filter(|e| e.channel == channel)
            .count()
            >= ENDPOINTS_EACH
        {
            return Response::error(Code::Resources);
        }
        let Some(slot) = self.endpoints.iter().position(Option::is_none) else {
            return Response::error(Code::Resources);
        };
        let mut stored = [0u8; NAME_BYTES];
        stored[..name.len()].copy_from_slice(name);
        self.endpoints[slot] = Some(Endpoint {
            name: stored,
            len: name.len(),
            scope,
            channel,
        });
        // A class server is up: its launch is over.
        for l in self.launches.iter_mut() {
            if l.is_some_and(|l| l.scope == scope && wire::class_endpoint(&l.clsid)[..] == *name) {
                *l = None;
            }
        }
        Response::empty()
    }
    fn connect(&mut self, channel: Handle, session: u64, found: Option<usize>) -> Response {
        let Some(e) = found.and_then(|i| self.endpoints[i]) else {
            return Response::error(Code::NotFound);
        };
        // A process reaches its own objects directly.
        if e.channel == channel {
            return Response::error(Code::InvalidRequest);
        }
        if self
            .links
            .iter()
            .flatten()
            .filter(|l| l.sides[0].open && l.sides[0].channel == channel)
            .count()
            >= LINKS_EACH
        {
            return Response::error(Code::Resources);
        }
        let Some(slot) = self.links.iter().position(Option::is_none) else {
            return Response::error(Code::Resources);
        };
        let mut id = self.last_id.wrapping_add(1).max(1);
        while self.links.iter().flatten().any(|l| l.id == id) {
            id = id.wrapping_add(1).max(1);
        }
        self.last_id = id;
        let side = |channel| Side {
            channel,
            open: true,
            queue: [0; MAILBOX],
            count: 0,
        };
        self.links[slot] = Some(Link {
            id,
            session,
            sides: [side(channel), side(e.channel)],
            fresh: true,
            endpoint: e.name,
            len: e.len,
        });
        Response::bytes(&id.to_le_bytes())
    }
    fn send(&mut self, i: usize, to: usize, packet: &[u8], from: Handle) -> Response {
        let Some(link) = self.links[i].as_mut() else {
            return Response::error(Code::NotFound);
        };
        let side = &mut link.sides[to];
        if !side.open {
            return Response::error(Code::Closing);
        }
        if side.count == MAILBOX {
            return Response::error(Code::WouldBlock);
        }
        let Some(slot) = self.pool.iter().position(Option::is_none) else {
            return Response::error(Code::WouldBlock);
        };
        let mut data = [0u8; PAYLOAD_BYTES];
        data[..packet.len()].copy_from_slice(packet);
        self.pool[slot] = Some(Packet {
            len: packet.len(),
            data,
            from,
        });
        side.queue[side.count] = slot as u8;
        side.count += 1;
        Response::empty()
    }
    fn poll(&mut self, channel: Handle, session: u64) -> Response {
        for n in 0..LINKS {
            let i = (self.cursor + n) % LINKS;
            let Some(link) = self.links[i].as_mut() else {
                continue;
            };
            if link.session != session {
                continue;
            }
            let Some(s) = link
                .sides
                .iter()
                .position(|s| s.open && s.channel == channel)
            else {
                continue;
            };
            let mut reply = [0u8; 5 + PAYLOAD_BYTES];
            reply[..4].copy_from_slice(&link.id.to_le_bytes());
            let len;
            if s == 1 && link.fresh {
                link.fresh = false;
                reply[4] = event::NEW;
                reply[5..5 + link.len].copy_from_slice(&link.endpoint[..link.len]);
                len = 5 + link.len;
            } else if link.sides[s].count > 0 {
                let side = &mut link.sides[s];
                let slot = side.queue[0] as usize;
                side.queue.copy_within(1.., 0);
                side.count -= 1;
                let Some(packet) = self.pool[slot].take() else {
                    continue;
                };
                reply[4] = event::PACKET;
                reply[5..5 + packet.len].copy_from_slice(&packet.data[..packet.len]);
                len = 5 + packet.len;
            } else if !link.sides[1 - s].open {
                reply[4] = event::CLOSED;
                len = 5;
                self.close(i, s);
            } else {
                continue;
            }
            self.cursor = (i + 1) % LINKS;
            return Response::bytes(&reply[..len]);
        }
        Response::error(Code::WouldBlock)
    }
    fn activate(&mut self, operation: u32, scope: Scope, clsid: [u8; 16], now: u64) -> Response {
        let launch = self
            .launches
            .iter()
            .position(|l| l.is_some_and(|l| l.scope == scope && l.clsid == clsid));
        if operation == wire::FAILED {
            if let Some(i) = launch {
                self.launches[i] = None;
            }
            return Response::empty();
        }
        if self.find(scope, &wire::class_endpoint(&clsid)).is_some() {
            return Response::scalar(u64::from(wire::READY));
        }
        if let Some(i) = launch {
            if self.launches[i].is_some_and(|l| now < l.until) {
                return Response::scalar(u64::from(wire::PENDING));
            }
            self.launches[i] = None;
        }
        let Some(slot) = self.launches.iter().position(Option::is_none) else {
            return Response::error(Code::Resources);
        };
        self.launches[slot] = Some(Launch {
            scope,
            clsid,
            until: now + wire::LAUNCH_TICKS,
        });
        Response::scalar(u64::from(wire::LAUNCH))
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec::Vec;
    fn peer(session: u64) -> Identity {
        Identity {
            session,
            pid: 7,
            uid: 1001,
            ..Identity::default()
        }
    }
    fn as_peer(b: &mut Broker, channel: Handle, who: Identity, op: u32, body: &[u8]) -> Code {
        b.request(channel, who, op, body, 0).code
    }
    fn call(b: &mut Broker, channel: Handle, op: u32, body: &[u8]) -> (Code, Vec<u8>) {
        let r = b.request(channel, peer(1), op, body, 0);
        (r.code, r.body[..r.len].to_vec())
    }
    #[test]
    fn relays_packets_in_order_and_reports_close() {
        let mut b = Broker::default();
        let name = b"OLE0000000000000007";
        assert_eq!(call(&mut b, 10, wire::REGISTER, name).0, Code::Success);
        assert_eq!(call(&mut b, 11, wire::REGISTER, name).0, Code::Exists);
        assert_eq!(
            call(&mut b, 10, wire::CONNECT, name).0,
            Code::InvalidRequest
        );
        let (code, id) = call(&mut b, 11, wire::CONNECT, name);
        assert_eq!(code, Code::Success);
        let mut send = id.clone();
        send.extend_from_slice(b"one");
        assert_eq!(call(&mut b, 11, wire::SEND, &send).0, Code::Success);
        send.truncate(4);
        send.extend_from_slice(b"two");
        assert_eq!(call(&mut b, 11, wire::SEND, &send).0, Code::Success);
        // The owner hears of the link first, then the packets in order.
        let (_, new) = call(&mut b, 10, wire::POLL, &[]);
        assert_eq!((new[4], &new[5..]), (event::NEW, &name[..]));
        assert_eq!(&call(&mut b, 10, wire::POLL, &[]).1[4..], b"\0one");
        assert_eq!(&call(&mut b, 10, wire::POLL, &[]).1[4..], b"\0two");
        assert_eq!(call(&mut b, 10, wire::POLL, &[]).0, Code::WouldBlock);
        // A full mailbox pushes back.
        for _ in 0..MAILBOX {
            assert_eq!(call(&mut b, 11, wire::SEND, &send).0, Code::Success);
        }
        assert_eq!(call(&mut b, 11, wire::SEND, &send).0, Code::WouldBlock);
        // Another connection cannot use the link.
        assert_eq!(call(&mut b, 12, wire::SEND, &send).0, Code::NotFound);
        // The client goes: the owner drains, then learns of it.
        b.disconnected(11);
        for _ in 0..MAILBOX {
            assert_eq!(call(&mut b, 10, wire::POLL, &[]).1[4], event::PACKET);
        }
        assert_eq!(
            call(&mut b, 10, wire::POLL, &[]).1,
            [&id[..], &[event::CLOSED]].concat()
        );
        assert_eq!(call(&mut b, 10, wire::POLL, &[]).0, Code::WouldBlock);
        assert!(b.links.iter().all(Option::is_none));
        assert!(b.pool.iter().all(Option::is_none));
        // The owner goes: its endpoint goes too.
        b.disconnected(10);
        assert_eq!(call(&mut b, 11, wire::CONNECT, name).0, Code::NotFound);
    }
    #[test]
    fn endpoints_are_scoped_to_their_owners_identity() {
        let mut b = Broker::default();
        let user = peer(1);
        let elevated = Identity {
            groups: door_abi::platform::group::ADMINISTRATORS,
            pid: 9,
            ..user
        };
        let other = Identity {
            uid: 1002,
            pid: 11,
            ..user
        };
        // An object exporter only under the registering process's ID.
        let (seven, _) = wire::exporter_endpoint(7);
        assert_eq!(
            as_peer(&mut b, 20, other, wire::REGISTER, &seven),
            Code::Denied
        );
        assert_eq!(
            as_peer(&mut b, 10, user, wire::REGISTER, &seven),
            Code::Success
        );
        // An unelevated class server is not what an elevated client gets,
        // nor another user's; the same name may be served at each level.
        let class = wire::class_endpoint(&[3u8; 16]);
        assert_eq!(
            as_peer(&mut b, 10, user, wire::REGISTER, &class),
            Code::Success
        );
        assert_eq!(
            as_peer(&mut b, 30, elevated, wire::CONNECT, &class),
            Code::NotFound
        );
        assert_eq!(
            as_peer(&mut b, 40, other, wire::CONNECT, &class),
            Code::NotFound
        );
        assert_eq!(
            as_peer(&mut b, 40, other, wire::CONNECT, &seven),
            Code::NotFound
        );
        assert_eq!(
            b.request(30, elevated, wire::ACTIVATE, &[3u8; 16], 0).body[..8],
            u64::from(wire::LAUNCH).to_le_bytes()
        );
        assert_eq!(
            as_peer(&mut b, 30, elevated, wire::REGISTER, &class),
            Code::Success
        );
        assert_eq!(
            as_peer(&mut b, 31, elevated, wire::CONNECT, &class),
            Code::Success
        );
        assert_eq!(
            as_peer(&mut b, 11, user, wire::CONNECT, &class),
            Code::Success
        );
    }
    #[test]
    fn one_connection_holds_a_bounded_share() {
        let mut b = Broker::default();
        let who = peer(1);
        for i in 0..ENDPOINTS_EACH as u8 {
            let name = wire::class_endpoint(&[i; 16]);
            assert_eq!(
                as_peer(&mut b, 10, who, wire::REGISTER, &name),
                Code::Success
            );
        }
        let more = wire::class_endpoint(&[0xee; 16]);
        assert_eq!(
            as_peer(&mut b, 10, who, wire::REGISTER, &more),
            Code::Resources
        );
        let mut ids = Vec::new();
        for i in 0..LINKS_EACH as u8 {
            let r = b.request(11, who, wire::CONNECT, &wire::class_endpoint(&[i; 16]), 0);
            assert_eq!(r.code, Code::Success);
            ids.push(r.body[..4].to_vec());
        }
        assert_eq!(
            as_peer(
                &mut b,
                11,
                who,
                wire::CONNECT,
                &wire::class_endpoint(&[0; 16])
            ),
            Code::Resources
        );
        // Packets: MAILBOX on each link, PACKETS_EACH in all.
        let mut sent = 0;
        for id in &ids {
            for _ in 0..MAILBOX {
                let body = [&id[..], b"p"].concat();
                if as_peer(&mut b, 11, who, wire::SEND, &body) == Code::Success {
                    sent += 1;
                }
            }
        }
        assert_eq!(sent, PACKETS_EACH);
    }
    #[test]
    fn sends_to_a_closed_side_fail() {
        let mut b = Broker::default();
        let name = b"OLECLASS-X";
        call(&mut b, 10, wire::REGISTER, name);
        let (_, id) = call(&mut b, 11, wire::CONNECT, name);
        assert_eq!(call(&mut b, 10, wire::CLOSE, &id).0, Code::Success);
        let send = [&id[..], b"x"].concat();
        assert_eq!(call(&mut b, 11, wire::SEND, &send).0, Code::Closing);
        assert_eq!(call(&mut b, 11, wire::POLL, &[]).1[4], event::CLOSED);
        assert!(b.links.iter().all(Option::is_none));
    }
    #[test]
    fn activation_is_launched_once() {
        let mut b = Broker::default();
        let clsid = [7u8; 16];
        assert_eq!(
            call(&mut b, 11, wire::ACTIVATE, &clsid).1,
            2u64.to_le_bytes()
        );
        assert_eq!(
            call(&mut b, 12, wire::ACTIVATE, &clsid).1,
            1u64.to_le_bytes()
        );
        call(&mut b, 13, wire::REGISTER, &wire::class_endpoint(&clsid));
        assert_eq!(
            call(&mut b, 12, wire::ACTIVATE, &clsid).1,
            0u64.to_le_bytes()
        );
    }
}
