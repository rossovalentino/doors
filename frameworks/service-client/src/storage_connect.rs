//! Pollable authenticated endpoint discovery. Never replays a Storage operation.
use super::*;

pub struct StorageConnect {
    init: Option<Init>,
}
impl StorageConnect {
    /// Connect/identity/send are non-waiting syscalls. Service discovery is polled
    /// by the caller while its normal input/event loop continues.
    pub fn begin(registry: Handle, now: u64) -> Result<Self> {
        let mut init = Init::connect(registry)?;
        init.connection
            .begin(opcode::RESOLVE, &manifest::STORAGE.to_le_bytes(), now)?;
        Ok(Self { init: Some(init) })
    }
    /// One terminal result per attempt. Dropping an attempt closes its Init
    /// channel, including on timeout or malformed transferred-port replies.
    pub fn poll(&mut self, now: u64) -> Result<Option<Storage>> {
        let Some(init) = self.init.as_mut() else {
            return Err(Error::Service(Code::Closing));
        };
        let reply = match init.connection.poll(now) {
            Ok(None) => return Ok(None),
            result => result,
        };
        self.init.take();
        let port = reply?.ok_or(Error::Malformed)?.one_handle()?;
        let storage = Storage::connect_port(port);
        let _ = door_sys::close(port);
        storage.map(Some)
    }
}
