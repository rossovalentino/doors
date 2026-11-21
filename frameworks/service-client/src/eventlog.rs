//! Client of the Event Log Init hosts (`door_service_protocol::eventlog`).
use crate::rpc::{Connection, Error, Result};
use door_abi::Handle;
use door_service_protocol::{
    Code,
    eventlog::{self as wire, Event, Log, Partial},
    manifest,
};

pub struct EventLog {
    connection: Connection,
}
impl EventLog {
    pub fn connect(registry: Handle) -> Result<Self> {
        Ok(Self {
            connection: Connection::connect(registry, manifest::INIT)?,
        })
    }
    /// Report `event` to its log: the new record number. A message longer
    /// than one packet holds goes ahead in REPORT_TEXT pieces.
    pub fn report(&mut self, event: &Event) -> Result<u32> {
        let message = event.message_bytes();
        let mut at = event.head_message();
        while at < message.len() {
            let end = (at + wire::TEXT_CHUNK).min(message.len());
            let mut body = [0u8; 1 + wire::TEXT_CHUNK];
            body[0] = event.log as u8;
            body[1..1 + end - at].copy_from_slice(&message[at..end]);
            self.connection
                .call(wire::REPORT_TEXT, &body[..1 + end - at], &[])?
                .empty()?;
            at = end;
        }
        let mut body = [0u8; 1 + wire::HEAD_BYTES];
        body[0] = event.log as u8;
        let n = {
            let head: &mut [u8; wire::HEAD_BYTES] = (&mut body[1..]).try_into().unwrap();
            event.encode(head)
        };
        let reply = self.connection.call(wire::REPORT, &body[..1 + n], &[])?;
        let data = reply.data()?;
        <[u8; 4]>::try_from(data)
            .map(u32::from_le_bytes)
            .map_err(|_| Error::Malformed)
    }
    /// Event `record` of `log`; `Ok(None)` when it is not there (overwritten
    /// or not written yet).
    pub fn read(&mut self, log: Log, record: u32) -> Result<Option<Event>> {
        let mut body = [0u8; 5];
        body[0] = log as u8;
        body[1..].copy_from_slice(&record.to_le_bytes());
        let mut partial = match self.connection.call(wire::READ, &body, &[]) {
            Ok(reply) => Partial::decode(reply.data()?).ok_or(Error::Malformed)?,
            Err(Error::Service(Code::NotFound)) => return Ok(None),
            Err(e) => return Err(e),
        };
        while partial.received() < partial.total() {
            let mut body = [0u8; 7];
            body[0] = log as u8;
            body[1..5].copy_from_slice(&record.to_le_bytes());
            body[5..].copy_from_slice(&(partial.received() as u16).to_le_bytes());
            let part = match self.connection.call(wire::READ_TEXT, &body, &[]) {
                Ok(reply) => reply,
                // Overwritten while it was read.
                Err(Error::Service(Code::NotFound)) => return Ok(None),
                Err(e) => return Err(e),
            };
            let part = part.data()?;
            if part.is_empty() || !partial.push(part) {
                return Err(Error::Malformed);
            }
        }
        partial.finish().map(Some).ok_or(Error::Malformed)
    }
    /// `(oldest record, count)` of `log` (oldest 0 when empty).
    pub fn info(&mut self, log: Log) -> Result<(u32, u32)> {
        let reply = self.connection.call(wire::INFO, &[log as u8], &[])?;
        let data = reply.data()?;
        if data.len() != 8 {
            return Err(Error::Malformed);
        }
        Ok((
            u32::from_le_bytes(data[..4].try_into().unwrap()),
            u32::from_le_bytes(data[4..].try_into().unwrap()),
        ))
    }
    pub fn clear(&mut self, log: Log) -> Result<()> {
        self.connection
            .call(wire::CLEAR, &[log as u8], &[])?
            .empty()
    }
}
