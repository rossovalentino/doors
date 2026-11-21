//! Port grants preserve creator identity while accepted channels acquire exactly
//! the client/server ownership required by the ordinary transport.
use super::*;
pub(super) const MAX_PORTS: usize = 8;
const PENDING: usize = 4;
#[derive(Clone, Copy)]
pub(super) struct Port {
    owner: ProcessId,
    live: bool,
    pending: [Option<u64>; PENDING],
}
impl Executive {
    pub fn create_port(&mut self, pid: ProcessId) -> Result<u64> {
        self.authority(pid)?;
        self.handle_capacity(pid, 1)?;
        self.object_capacity(1)?;
        let slot = self
            .ports
            .iter()
            .position(Option::is_none)
            .ok_or(Status::ResourceExhausted)?;
        self.ports[slot] = Some(Port {
            owner: pid,
            live: true,
            pending: [None; PENDING],
        });
        let object = self.allocate_object(Kind::Port { slot });
        Ok(self.install(
            pid,
            object,
            rights::QUERY | rights::DUPLICATE | rights::TRANSFER | rights::READ | rights::WRITE,
        ))
    }
    pub fn connect_port(&mut self, pid: ProcessId, handle: u64) -> Result<u64> {
        self.authority(pid)?;
        let grant = self.grant(pid, handle, rights::WRITE)?;
        let Kind::Port { slot } = self.object(grant.object).kind else {
            return Err(Status::InvalidArgument);
        };
        let port = self.ports[slot].as_ref().expect("live port object");
        if !port.live || self.authority(port.owner).is_err() {
            return Err(Status::PeerClosed);
        }
        let pending = port
            .pending
            .iter()
            .position(Option::is_none)
            .ok_or(Status::WouldBlock)?;
        let owner = port.owner;
        let pair = self.channel_pair(pid, owner)?;
        self.ports[slot].as_mut().unwrap().pending[pending] = Some(pair[1]);
        Ok(pair[0])
    }
    pub fn accept_port(&mut self, pid: ProcessId, handle: u64) -> Result<u64> {
        self.authority(pid)?;
        let grant = self.grant(pid, handle, rights::READ)?;
        let Kind::Port { slot } = self.object(grant.object).kind else {
            return Err(Status::InvalidArgument);
        };
        let port = self.ports[slot].as_mut().expect("live port object");
        if port.owner != pid || !port.live {
            return Err(Status::AccessDenied);
        }
        let result = port.pending[0].take().ok_or(Status::WouldBlock)?;
        port.pending.rotate_left(1);
        Ok(result)
    }
    pub(super) fn close_owned_ports(&mut self, pid: ProcessId) {
        for slot in 0..MAX_PORTS {
            if self.ports[slot]
                .as_ref()
                .is_some_and(|p| p.owner == pid && p.live)
            {
                self.close_port(slot);
            }
        }
    }
    fn close_port(&mut self, slot: usize) {
        let port = self.ports[slot].as_mut().expect("owned port");
        port.live = false;
        let owner = port.owner;
        let pending = core::mem::replace(&mut port.pending, [None; PENDING]);
        for handle in pending.into_iter().flatten() {
            // The owner may already have closed a pending endpoint explicitly.
            let _ = self.close_owned(owner, handle);
        }
    }
    pub(super) fn drop_port(&mut self, slot: usize) {
        self.close_port(slot);
        self.ports[slot] = None;
    }
}
