//! Client of the COM activation broker and ncalrpc transport Init hosts
//! (`door_service_protocol::com`), for ole32.
use crate::rpc::{Connection, Error, Result};
use door_abi::Handle;
use door_service_protocol::{
    Code,
    com::{self as wire, PAYLOAD_BYTES},
    manifest,
};

/// What POLL reported on one of the caller's links (`wire::event`).
pub struct Event {
    pub link: u32,
    pub kind: u8,
    data: [u8; PAYLOAD_BYTES],
    len: usize,
}
impl Event {
    /// The packet, or the endpoint name of a NEW link.
    pub fn data(&self) -> &[u8] {
        &self.data[..self.len]
    }
}
pub struct ComBroker {
    connection: Connection,
}
fn would_block<T>(result: Result<T>) -> Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(Error::Service(Code::WouldBlock)) => Ok(None),
        Err(e) => Err(e),
    }
}
impl ComBroker {
    pub fn connect(registry: Handle) -> Result<Self> {
        Ok(Self {
            connection: Connection::connect(registry, manifest::INIT)?,
        })
    }
    fn named(&mut self, op: u32, name: &[u8]) -> Result<crate::rpc::Reply> {
        if !wire::valid_name(name) {
            return Err(Error::Service(Code::InvalidRequest));
        }
        self.connection.call(op, name, &[])
    }
    /// Whether the connection to Init broke (connect again: endpoints and
    /// links went with it).
    pub fn needs_reconnect(&self) -> bool {
        self.connection.needs_reconnect()
    }
    /// Own the endpoint `name`.
    pub fn register(&mut self, name: &[u8]) -> Result<()> {
        self.named(wire::REGISTER, name)?.empty()
    }
    pub fn unregister(&mut self, name: &[u8]) -> Result<()> {
        self.named(wire::UNREGISTER, name)?.empty()
    }
    /// A link to the endpoint's owner.
    pub fn connect_endpoint(&mut self, name: &[u8]) -> Result<u32> {
        let reply = self.named(wire::CONNECT, name)?;
        let data = reply.data()?;
        Ok(u32::from_le_bytes(
            data.try_into().map_err(|_| Error::Malformed)?,
        ))
    }
    /// Queue `packet` (1..=`PAYLOAD_BYTES`) for the link's other side;
    /// false while its mailbox is full.
    pub fn send(&mut self, link: u32, packet: &[u8]) -> Result<bool> {
        if packet.is_empty() || packet.len() > PAYLOAD_BYTES {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let mut body = [0u8; 4 + PAYLOAD_BYTES];
        body[..4].copy_from_slice(&link.to_le_bytes());
        body[4..4 + packet.len()].copy_from_slice(packet);
        let sent = self
            .connection
            .call(wire::SEND, &body[..4 + packet.len()], &[])
            .and_then(|reply| reply.empty());
        Ok(would_block(sent)?.is_some())
    }
    /// The next event on any of this connection's links.
    pub fn poll(&mut self) -> Result<Option<Event>> {
        let Some(reply) = would_block(self.connection.call(wire::POLL, &[], &[]))? else {
            return Ok(None);
        };
        let data = reply.data()?;
        if data.len() < 5 || data.len() > 5 + PAYLOAD_BYTES {
            return Err(Error::Malformed);
        }
        let mut event = Event {
            link: u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
            kind: data[4],
            data: [0; PAYLOAD_BYTES],
            len: data.len() - 5,
        };
        event.data[..event.len].copy_from_slice(&data[5..]);
        Ok(Some(event))
    }
    pub fn close(&mut self, link: u32) -> Result<()> {
        self.connection
            .call(wire::CLOSE, &link.to_le_bytes(), &[])?
            .empty()
    }
    /// `wire::READY`, `PENDING` or `LAUNCH` for a LocalServer32 class.
    pub fn activate(&mut self, clsid: &[u8; 16]) -> Result<u32> {
        Ok(self.connection.call(wire::ACTIVATE, clsid, &[])?.scalar()? as u32)
    }
    pub fn launch_failed(&mut self, clsid: &[u8; 16]) -> Result<()> {
        self.connection.call(wire::FAILED, clsid, &[])?.empty()
    }
}
