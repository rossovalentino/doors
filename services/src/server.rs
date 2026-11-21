use door_abi::{
    Handle, Status, Transfer,
    platform::{Identity, StartupV2, inherited},
    rights,
};
use door_service_protocol::{BODY_BYTES, Code, Frame, Message, manifest, opcode};
pub const CONNECTIONS: usize = manifest::MAX_CLIENT_CONNECTIONS;
pub struct Response {
    pub code: Code,
    pub body: [u8; BODY_BYTES],
    pub len: usize,
    pub transfers: [Transfer; 1],
    pub handles: usize,
}
impl Response {
    pub const fn empty() -> Self {
        Self {
            code: Code::Success,
            body: [0; BODY_BYTES],
            len: 0,
            transfers: [Transfer {
                handle: 0,
                rights: 0,
                mode: 0,
            }],
            handles: 0,
        }
    }
    pub fn error(code: Code) -> Self {
        Self {
            code,
            ..Self::empty()
        }
    }
    pub fn scalar(value: u64) -> Self {
        let mut r = Self::empty();
        r.body[..8].copy_from_slice(&value.to_le_bytes());
        r.len = 8;
        r
    }
    pub fn bytes(value: &[u8]) -> Self {
        let mut r = Self::empty();
        r.body[..value.len()].copy_from_slice(value);
        r.len = value.len();
        r
    }
    pub fn handle(handle: Handle, rights: u64, moving: bool) -> Self {
        Self {
            transfers: [Transfer {
                handle,
                rights,
                mode: moving as u64,
            }],
            handles: 1,
            ..Self::empty()
        }
    }
}
pub trait Handler {
    fn request(
        &mut self,
        channel: Handle,
        peer: Identity,
        opcode: u32,
        body: &[u8],
        transfers: &[Transfer],
    ) -> Response;
    fn disconnected(&mut self, _channel: Handle) {}
    fn shutdown(&mut self) -> Result<(), Status> {
        Ok(())
    }
}
#[derive(Clone, Copy)]
struct Connection {
    channel: Handle,
    last: u64,
    /// The peer process, and whether it is a boot service (session 0).
    pid: u64,
    system: bool,
}
/// Connections one session process may hold to a service, and the slots
/// kept for boot services (Init, the window server, the session service),
/// so that one program cannot take every connection a service has.
pub const PER_PROCESS: usize = 6;
pub const RESERVED: usize = 2;
/// Whether a new connection from `pid` (a boot service when `system`) is
/// taken, given the connections held now (`(pid, system)` per slot).
fn admits(held: &[Option<(u64, bool)>], pid: u64, system: bool) -> bool {
    if system {
        return true;
    }
    let free = held.iter().filter(|c| c.is_none()).count();
    let own = held.iter().flatten().filter(|(p, _)| *p == pid).count();
    free > RESERVED && own < PER_PROCESS
}
pub struct Server {
    pub port: Handle,
    instance: u64,
    control: Handle,
    connections: [Option<Connection>; CONNECTIONS],
    progressed: bool,
    idle: IdleBackoff,
}
#[derive(Default)]
struct IdleBackoff(u8);
impl IdleBackoff {
    fn sleep(&mut self, progressed: bool) -> bool {
        if progressed {
            self.0 = 0;
            return false;
        }
        self.0 = self.0.saturating_add(1).min(32);
        self.0 == 32
    }
}
impl Server {
    /// Keep active IPC bursts cooperative. Only repeated empty polls back off
    /// for one existing timer tick; new work resets the backoff immediately.
    /// This is bounded polling, not an interrupt-driven readiness wait.
    pub fn idle_backoff(&mut self) -> Result<(), Status> {
        if self.idle.sleep(core::mem::take(&mut self.progressed)) {
            door_sys::sleep(door_sys::time()?.saturating_add(1))
        } else {
            door_sys::yield_now()
        }
    }
    pub fn note_progress(&mut self) {
        self.progressed = true;
    }
    pub fn new(startup: &StartupV2) -> Result<Self, Status> {
        let own = door_sys::identity(0)?;
        let policy =
            door_abi::platform::manifest_policy(own.manifest).ok_or(Status::AccessDenied)?;
        if own != startup.identity
            || own.uid != policy.uid
            || own.capabilities != policy.caps
            || own.session != 0
        {
            return Err(Status::AccessDenied);
        }
        let port = door_sys::port_create()?;
        Ok(Self {
            port,
            instance: own.instance,
            control: startup.base.inherited[inherited::CONTROL].handle,
            connections: [None; CONNECTIONS],
            progressed: false,
            idle: IdleBackoff::default(),
        })
    }
    pub fn ready(&self, manifest: u64) -> Result<(), Status> {
        let packet = Message::new(
            opcode::READY,
            1,
            self.instance,
            Code::Success,
            &manifest.to_le_bytes(),
            1,
        )?;
        door_sys::channel_send(
            self.control,
            packet.bytes(),
            &[Transfer {
                handle: self.port,
                rights: manifest::PUBLISH_RIGHTS,
                mode: 0,
            }],
        )
    }
    pub fn poll<H: Handler>(&mut self, handler: &mut H) -> Result<bool, Status> {
        if self.control != 0 {
            let mut bytes = [0; door_abi::MAX_PACKET];
            let mut transfers = [Transfer::default(); door_abi::MAX_TRANSFERS];
            match door_sys::channel_receive(self.control, &mut bytes, &mut transfers) {
                Ok(metadata) => {
                    let frame = Frame::decode(&bytes[..metadata.packet_bytes as usize]);
                    let peer = door_sys::identity(self.control)?;
                    for transfer in &transfers[..metadata.handle_count as usize] {
                        let _ = door_sys::close(transfer.handle);
                    }
                    let frame = frame?;
                    if peer.manifest != manifest::INIT
                        || frame.header.opcode != opcode::STOP
                        || frame.header.request_id != 2
                        || frame.code != Code::Success
                        || !frame.body.is_empty()
                        || frame.instance != self.instance
                        || metadata.handle_count != 0
                    {
                        return Err(Status::AccessDenied);
                    }
                    self.disconnect_all(handler);
                    handler.shutdown()?;
                    return Ok(false);
                }
                Err(Status::WouldBlock) => (),
                Err(Status::PeerClosed) => {
                    self.disconnect_all(handler);
                    handler.shutdown()?;
                    return Ok(false);
                }
                Err(error) => return Err(error),
            }
        }
        if let Some(index) = self.connections.iter().position(Option::is_none) {
            match door_sys::port_accept(self.port) {
                Ok(channel) => {
                    self.progressed = true;
                    let held: [Option<(u64, bool)>; CONNECTIONS] =
                        self.connections.map(|c| c.map(|c| (c.pid, c.system)));
                    match door_sys::identity(channel) {
                        Ok(peer) if admits(&held, peer.pid, peer.session == 0) => {
                            self.connections[index] = Some(Connection {
                                channel,
                                last: 0,
                                pid: peer.pid,
                                system: peer.session == 0,
                            });
                        }
                        // Refused: the peer sees its channel closed.
                        _ => {
                            let _ = door_sys::close(channel);
                        }
                    }
                }
                Err(Status::WouldBlock) => (),
                Err(error) => return Err(error),
            }
        }
        for index in 0..CONNECTIONS {
            let Some(connection) = self.connections[index] else {
                continue;
            };
            let mut bytes = [0; door_abi::MAX_PACKET];
            let mut transfers = [Transfer::default(); door_abi::MAX_TRANSFERS];
            let metadata =
                match door_sys::channel_receive(connection.channel, &mut bytes, &mut transfers) {
                    Ok(metadata) => metadata,
                    Err(Status::WouldBlock) => continue,
                    Err(Status::PeerClosed | Status::AccessDenied) => {
                        self.disconnect(index, handler);
                        continue;
                    }
                    Err(error) => return Err(error),
                };
            let received = &transfers[..metadata.handle_count as usize];
            self.progressed = true;
            let frame = Frame::decode(&bytes[..metadata.packet_bytes as usize]);
            let peer = door_sys::identity(connection.channel);
            // A revoked session (or vanished peer) can never authenticate
            // again: answer Denied, then disconnect so the handler releases
            // everything the channel holds (open files, staging slots).
            let revoked = frame.is_ok() && unauthenticatable(&peer);
            let response = match (frame.as_ref(), peer) {
                (Ok(frame), Ok(peer)) => {
                    match frame.request(self.instance, connection.last, received.len()) {
                        Ok(()) => {
                            self.connections[index].as_mut().unwrap().last =
                                frame.header.request_id;
                            handler.request(
                                connection.channel,
                                peer,
                                frame.header.opcode,
                                frame.body,
                                received,
                            )
                        }
                        Err(code) => Response::error(code),
                    }
                }
                (Ok(_), Err(Status::AccessDenied | Status::PeerClosed)) => {
                    Response::error(Code::Denied)
                }
                _ => {
                    for transfer in received {
                        let _ = door_sys::close(transfer.handle);
                    }
                    self.disconnect(index, handler);
                    continue;
                }
            };
            for transfer in received {
                let _ = door_sys::close(transfer.handle);
            }
            let frame = frame.unwrap();
            let packet = Message::new(
                frame.header.opcode,
                frame.header.request_id,
                self.instance,
                response.code,
                &response.body[..response.len],
                response.handles,
            )?;
            if let Err(error) = door_sys::channel_send(
                connection.channel,
                packet.bytes(),
                &response.transfers[..response.handles],
            ) {
                for transfer in &response.transfers[..response.handles] {
                    if transfer.mode == 1 {
                        let _ = door_sys::close(transfer.handle);
                    }
                }
                self.disconnect(index, handler);
                if !matches!(
                    error,
                    Status::PeerClosed | Status::AccessDenied | Status::WouldBlock
                ) {
                    return Err(error);
                }
            } else if revoked {
                self.disconnect(index, handler);
            }
        }
        Ok(true)
    }
    fn disconnect<H: Handler>(&mut self, index: usize, handler: &mut H) {
        if let Some(connection) = self.connections[index].take() {
            self.progressed = true;
            handler.disconnected(connection.channel);
            let _ = door_sys::close(connection.channel);
        }
    }
    pub fn disconnect_all<H: Handler>(&mut self, handler: &mut H) {
        for i in 0..CONNECTIONS {
            self.disconnect(i, handler);
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        for connection in self.connections.iter().flatten() {
            let _ = door_sys::close(connection.channel);
        }
        let _ = door_sys::close(self.port);
    }
}
pub fn serve<H: Handler>(mut server: Server, mut handler: H) -> i64 {
    loop {
        match server.poll(&mut handler) {
            Ok(true) => {
                if server.idle_backoff().is_err() {
                    return -102;
                }
            }
            Ok(false) => return 0,
            Err(_) => return -103,
        }
    }
}
pub fn status(error: Status) -> Code {
    match error {
        Status::AccessDenied => Code::Denied,
        Status::BadHandle => Code::InvalidHandle,
        Status::NotFound => Code::NotFound,
        Status::AlreadyExists => Code::Exists,
        Status::ResourceExhausted => Code::Resources,
        Status::WouldBlock => Code::WouldBlock,
        Status::Unsupported => Code::Unsupported,
        Status::InvalidArgument | Status::VersionMismatch | Status::InvalidOperation => {
            Code::InvalidRequest
        }
        _ => Code::Io,
    }
}
/// The kernel no longer authenticates this channel's peer (revoked session
/// epoch or closed peer); it never will again.
fn unauthenticatable(peer: &Result<Identity, Status>) -> bool {
    matches!(peer, Err(Status::AccessDenied | Status::PeerClosed))
}
/// A process of a logon session: a local account's application, or a
/// process of the service account's session (`service_peer`).
pub fn session_peer(peer: Identity) -> bool {
    peer.session != 0
        && peer.epoch != 0
        && (peer.uid >= 1000 || peer.uid == door_abi::platform::SERVICE_ACCOUNT)
        && peer.capabilities == 0
}
/// A Windows service (or a program it started): a process of the service
/// account's logon session, which only Init's boot permit opens.
pub fn service_peer(peer: Identity) -> bool {
    session_peer(peer) && peer.uid == door_abi::platform::SERVICE_ACCOUNT
}
pub fn transferred_capability(handle: Handle, kind: door_abi::HandleKind) -> Result<(), Status> {
    let info = door_sys::inspect(handle)?;
    if info[0] != kind as u64 || info[1] & rights::QUERY == 0 {
        return Err(Status::AccessDenied);
    }
    Ok(())
}
#[cfg(test)]
mod idle_tests {
    use super::{
        CONNECTIONS, Identity, IdleBackoff, PER_PROCESS, RESERVED, Status, admits, service_peer,
        session_peer, unauthenticatable,
    };
    #[test]
    fn the_service_account_is_a_session_peer_only_in_a_logon_session() {
        let user = Identity {
            pid: 9,
            uid: 1001,
            session: 4,
            epoch: 4,
            ..Identity::default()
        };
        assert!(session_peer(user) && !service_peer(user));
        let service = Identity {
            uid: door_abi::platform::SERVICE_ACCOUNT,
            ..user
        };
        assert!(session_peer(service) && service_peer(service));
        // Without a logon session, with capabilities (a boot service), or
        // any other system uid: never.
        for peer in [
            Identity {
                session: 0,
                ..service
            },
            Identity {
                epoch: 0,
                ..service
            },
            Identity {
                capabilities: 1,
                ..service
            },
            Identity { uid: 17, ..service },
            Identity { uid: 999, ..user },
        ] {
            assert!(!session_peer(peer) && !service_peer(peer), "{peer:?}");
        }
    }
    #[test]
    fn one_process_cannot_take_every_connection() {
        let mut held = [None; CONNECTIONS];
        // A session process gets PER_PROCESS connections, no more.
        for slot in held.iter_mut().take(PER_PROCESS) {
            assert!(admits(&[None; CONNECTIONS], 7, false));
            *slot = Some((7, false));
        }
        assert!(!admits(&held, 7, false));
        // Others fill the rest but the last RESERVED slots.
        let mut next = 100;
        while admits(&held, next, false) {
            let free = held.iter().position(Option::is_none).unwrap();
            held[free] = Some((next, false));
            next += 1;
        }
        assert_eq!(held.iter().filter(|c| c.is_none()).count(), RESERVED);
        // Boot services still connect.
        assert!(admits(&held, 1, true));
    }
    #[test]
    fn revoked_or_closed_peers_are_disconnected_after_their_denied_reply() {
        assert!(unauthenticatable(&Err(Status::AccessDenied)));
        assert!(unauthenticatable(&Err(Status::PeerClosed)));
        assert!(!unauthenticatable(&Ok(Identity::default())));
        assert!(!unauthenticatable(&Err(Status::WouldBlock)));
    }
    #[test]
    fn idle_servers_stop_spinning_but_cooperative_request_bursts_do_not_sleep() {
        let mut backoff = IdleBackoff::default();
        for _ in 0..100 {
            assert!(!backoff.sleep(true));
            for _ in 0..8 {
                assert!(!backoff.sleep(false));
            }
        }
        for _ in 8..31 {
            assert!(!backoff.sleep(false));
        }
        for _ in 0..100 {
            assert!(backoff.sleep(false));
        }
        assert!(!backoff.sleep(true));
        for _ in 0..31 {
            assert!(!backoff.sleep(false));
        }
        assert!(backoff.sleep(false));
    }
}
