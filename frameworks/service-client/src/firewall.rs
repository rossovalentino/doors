//! Windows Defender Firewall's store through Init
//! (`door_service_protocol::firewall`): administrators write the rules in
//! HKLM themselves and ask Init to hand them to door.network again; the
//! window server stores a change once User Account Control approved it.
use super::*;
use door_service_protocol::firewall as fw;

pub struct Firewall {
    connection: Connection,
}
impl Firewall {
    pub fn connect(registry: Handle) -> Result<Self> {
        Ok(Self {
            connection: Connection::connect(registry, manifest::INIT)?,
        })
    }
    /// Init reads the registry again and hands the policy over.
    pub fn reload(&mut self) -> Result<()> {
        self.connection.call(fw::RELOAD, &[], &[])?.empty()
    }
    /// The window server: store an approved change (`fw::change`).
    pub fn change(&mut self, body: &[u8]) -> Result<()> {
        self.connection.call(fw::CHANGE, body, &[])?.empty()
    }
}
