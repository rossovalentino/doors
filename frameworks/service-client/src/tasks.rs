//! Client of the Task Scheduler Init hosts (`door_service_protocol::tasks`).
use crate::rpc::{Connection, Error, Result};
use door_abi::Handle;
use door_service_protocol::{
    Code, manifest,
    tasks::{self as wire, Status, Task},
};

pub struct Tasks {
    connection: Connection,
}
impl Tasks {
    pub fn connect(registry: Handle) -> Result<Self> {
        Ok(Self {
            connection: Connection::connect(registry, manifest::INIT)?,
        })
    }
    /// Whether the connection to Init broke (connect again).
    pub fn needs_reconnect(&self) -> bool {
        self.connection.needs_reconnect()
    }
    /// Create `task` (`task.replace`: over one of the same name).
    pub fn create(&mut self, task: &Task) -> Result<()> {
        let mut body = [0u8; wire::TASK_BYTES];
        let n = task.encode(&mut body);
        self.connection.call(wire::CREATE, &body[..n], &[])?.empty()
    }
    fn named(&mut self, op: u32, prefix: &[u8], name: &str) -> Result<()> {
        if !wire::valid_name(name) {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let mut body = [0u8; 1 + wire::NAME_BYTES];
        body[..prefix.len()].copy_from_slice(prefix);
        body[prefix.len()..prefix.len() + name.len()].copy_from_slice(name.as_bytes());
        self.connection
            .call(op, &body[..prefix.len() + name.len()], &[])?
            .empty()
    }
    pub fn delete(&mut self, name: &str) -> Result<()> {
        self.named(wire::DELETE, &[], name)
    }
    pub fn run(&mut self, name: &str) -> Result<()> {
        self.named(wire::RUN, &[], name)
    }
    pub fn end(&mut self, name: &str) -> Result<()> {
        self.named(wire::END, &[], name)
    }
    pub fn enable(&mut self, name: &str, enabled: bool) -> Result<()> {
        self.named(wire::CHANGE, &[u8::from(enabled)], name)
    }
    /// Store `xml` (UTF-8) as the task's XML definition.
    pub fn put_xml(&mut self, name: &str, xml: &[u8]) -> Result<()> {
        if !wire::valid_name(name) || xml.len() > wire::XML_BYTES {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let room = door_service_protocol::BODY_BYTES - 5 - name.len();
        let mut offset = 0usize;
        loop {
            let n = (xml.len() - offset).min(room);
            let mut body = [0u8; door_service_protocol::BODY_BYTES];
            body[..4].copy_from_slice(&(offset as u32).to_le_bytes());
            body[4] = name.len() as u8;
            body[5..5 + name.len()].copy_from_slice(name.as_bytes());
            let at = 5 + name.len();
            body[at..at + n].copy_from_slice(&xml[offset..offset + n]);
            self.connection
                .call(wire::XML_PUT, &body[..at + n], &[])?
                .empty()?;
            offset += n;
            if offset == xml.len() {
                return Ok(());
            }
        }
    }
    /// The task's XML definition (empty: none kept), into `out`; its length.
    pub fn get_xml(&mut self, name: &str, out: &mut [u8]) -> Result<usize> {
        if !wire::valid_name(name) {
            return Err(Error::Service(Code::InvalidRequest));
        }
        let mut at = 0usize;
        loop {
            let mut body = [0u8; 4 + wire::NAME_BYTES];
            body[..4].copy_from_slice(&(at as u32).to_le_bytes());
            body[4..4 + name.len()].copy_from_slice(name.as_bytes());
            let reply = self
                .connection
                .call(wire::XML_GET, &body[..4 + name.len()], &[])?;
            let data = reply.data()?;
            if data.is_empty() {
                return Ok(at);
            }
            if at + data.len() > out.len() {
                return Err(Error::Malformed);
            }
            out[at..at + data.len()].copy_from_slice(data);
            at += data.len();
        }
    }
    /// The task at `index` among the caller's (an administrator's: all);
    /// `Ok(None)` past the last.
    pub fn query(&mut self, index: u16) -> Result<Option<Status>> {
        match self.connection.call(wire::QUERY, &index.to_le_bytes(), &[]) {
            Ok(reply) => Status::decode(reply.data()?)
                .map(Some)
                .ok_or(Error::Malformed),
            Err(Error::Service(Code::NotFound)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
