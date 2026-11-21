//! door.accessibility client calls on the window connection
//! (`door_service_protocol::accessibility`). Allocation-free: providers pass
//! encoded wire messages, assistive clients read into caller buffers.
//! Request the capabilities with `WindowClient::connect_with(registry,
//! CAP_ACCESSIBILITY)` (providers) or `CAP_ACCESSIBILITY | CAP_ASSISTIVE`
//! (assistive clients; granted only to authorized identities).
use crate::window::{Event, WindowClient};
use crate::{Error, Result};
use door_service_protocol::{BODY_BYTES, Code, accessibility as a};

/// Accessibility events for clients that negotiated a capability (used by
/// `WindowClient::poll`).
pub(crate) fn decode_event(b: &[u8], capabilities: u32) -> Option<Event> {
    if capabilities & (a::CAP_ACCESSIBILITY | a::CAP_ASSISTIVE) != 0 {
        return a::decode(b).ok();
    }
    None
}

/// An assistive action request delivered to a provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionRequest {
    pub window: u64,
    pub node: u64,
    pub action: u32,
    /// Argument bytes in `buffer[..len]` (see `accessibility::ActionRequest`).
    pub len: usize,
}

impl WindowClient {
    pub fn supports_accessibility(&self) -> bool {
        self.capabilities & a::CAP_ACCESSIBILITY != 0
    }
    pub fn supports_assistive(&self) -> bool {
        self.capabilities & a::CAP_ASSISTIVE != 0
    }
    fn require(&self, cap: u32) -> Result<()> {
        if self.capabilities & cap == 0 {
            Err(Error::Service(Code::Unsupported))
        } else {
            Ok(())
        }
    }
    /// Publish one encoded tree message (full or delta) for an owned window.
    /// `Service(Changed)` means a delta's base is not the published version:
    /// publish the full tree instead.
    pub fn accessibility_publish(&mut self, window: u64, message: &[u8]) -> Result<u32> {
        self.require(a::CAP_ACCESSIBILITY)?;
        let p = a::Publish::of_message(window, message).map_err(Error::Service)?;
        self.connection
            .call(a::PUBLISH_BEGIN, &p.encode(), &[])?
            .empty()?;
        let mut body = [0u8; BODY_BYTES];
        for (offset, len) in a::chunks(message.len()) {
            body[..4].copy_from_slice(&(offset as u32).to_le_bytes());
            body[4..4 + len].copy_from_slice(&message[offset..offset + len]);
            self.connection
                .call(a::PUBLISH_CHUNK, &body[..4 + len], &[])?
                .empty()?;
        }
        let r = self.connection.call(a::PUBLISH_COMMIT, &[], &[])?;
        let d = r.data()?;
        if d.len() != 4 {
            return Err(Error::Malformed);
        }
        Ok(u32::from_le_bytes(d.try_into().unwrap()))
    }
    /// Withdraw a window's tree (before destroying it).
    pub fn accessibility_withdraw(&mut self, window: u64) -> Result<()> {
        self.require(a::CAP_ACCESSIBILITY)?;
        self.connection
            .call(a::WITHDRAW, &window.to_le_bytes(), &[])?
            .empty()
    }
    /// The request behind an ACTION event; the argument goes to `buffer`.
    pub fn accessibility_action(
        &mut self,
        serial: u32,
        buffer: &mut [u8],
    ) -> Result<ActionRequest> {
        self.require(a::CAP_ACCESSIBILITY)?;
        let r = self
            .connection
            .call(a::ACTION_GET, &serial.to_le_bytes(), &[])?;
        let req = a::ActionRequest::decode(r.data()?).map_err(|_| Error::Malformed)?;
        if req.arg.len() > buffer.len() {
            return Err(Error::Service(Code::Resources));
        }
        buffer[..req.arg.len()].copy_from_slice(req.arg);
        Ok(ActionRequest {
            window: req.window,
            node: req.node,
            action: req.action,
            len: req.arg.len(),
        })
    }
    pub fn accessibility_action_done(&mut self, serial: u32, result: u32) -> Result<()> {
        self.require(a::CAP_ACCESSIBILITY)?;
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&serial.to_le_bytes());
        b[4..].copy_from_slice(&result.to_le_bytes());
        self.connection.call(a::ACTION_DONE, &b, &[])?.empty()
    }
    /// Assistive: the focused window and node.
    pub fn accessibility_focus(&mut self) -> Result<a::Focus> {
        self.require(a::CAP_ASSISTIVE)?;
        let r = self.connection.call(a::FOCUS_GET, &[], &[])?;
        a::Focus::decode(r.data()?).map_err(|_| Error::Malformed)
    }
    /// Assistive: read a redacted tree (or one node) into `out`; returns its
    /// length. `Service(Changed)` when the snapshot expired mid-read.
    pub fn accessibility_read(&mut self, request: a::Snapshot, out: &mut [u8]) -> Result<usize> {
        self.require(a::CAP_ASSISTIVE)?;
        let r = self.connection.call(a::SNAPSHOT, &request.encode(), &[])?;
        let d = r.data()?;
        if d.len() != 8 {
            return Err(Error::Malformed);
        }
        let serial = u32::from_le_bytes(d[..4].try_into().unwrap());
        let total = u32::from_le_bytes(d[4..].try_into().unwrap()) as usize;
        if total > out.len() {
            return Err(Error::Service(Code::Resources));
        }
        let mut done = 0;
        while done < total {
            let mut q = [0u8; 8];
            q[..4].copy_from_slice(&serial.to_le_bytes());
            q[4..].copy_from_slice(&(done as u32).to_le_bytes());
            let r = self.connection.call(a::READ, &q, &[])?;
            let chunk = r.data()?;
            if chunk.is_empty() || done + chunk.len() > total {
                return Err(Error::Malformed);
            }
            out[done..done + chunk.len()].copy_from_slice(chunk);
            done += chunk.len();
        }
        Ok(total)
    }
    /// Assistive: ask a provider to act; the result arrives as ACTION_RESULT.
    pub fn accessibility_invoke(
        &mut self,
        window: u64,
        node: u64,
        action: u32,
        arg: &[u8],
    ) -> Result<u32> {
        self.require(a::CAP_ASSISTIVE)?;
        let mut b = [0u8; BODY_BYTES];
        let n = a::ActionRequest {
            window,
            node,
            action,
            arg,
        }
        .encode(&mut b)
        .map_err(Error::Service)?;
        let r = self.connection.call(a::INVOKE, &b[..n], &[])?;
        let d = r.data()?;
        if d.len() != 4 {
            return Err(Error::Malformed);
        }
        Ok(u32::from_le_bytes(d.try_into().unwrap()))
    }
    /// Assistive: windows with published trees: (window, version, flags).
    pub fn accessibility_windows(&mut self, out: &mut [(u64, u32, u32)]) -> Result<usize> {
        self.require(a::CAP_ASSISTIVE)?;
        let r = self.connection.call(a::WINDOWS, &[], &[])?;
        let d = r.data()?;
        if d.len() < 4 {
            return Err(Error::Malformed);
        }
        let n = u32::from_le_bytes(d[..4].try_into().unwrap()) as usize;
        if d.len() != 4 + 16 * n {
            return Err(Error::Malformed);
        }
        let mut written = 0;
        for (slot, e) in out.iter_mut().zip(d[4..].chunks_exact(16)) {
            *slot = (
                u64::from_le_bytes(e[..8].try_into().unwrap()),
                u32::from_le_bytes(e[8..12].try_into().unwrap()),
                u32::from_le_bytes(e[12..].try_into().unwrap()),
            );
            written += 1;
        }
        Ok(written)
    }
}
