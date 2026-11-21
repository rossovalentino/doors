//! door.notifications client (`door_service_protocol::notifications`).
//!
//! Applications post toasts through Init; the desktop shell presents them.
//! Example (File Explorer, after a copy finished):
//! ```ignore
//! let mut n = Notifications::connect(registry)?;
//! let id = n.post(&Toast {
//!     icon: door_service_protocol::shell::icon::FILES,
//!     flags: 0,
//!     title: "Copy complete",
//!     body: "3 items copied to Documents",
//!     actions: ["Open folder", ""],
//! })?;
//! // Later, from the app's event loop:
//! if let Some((id, action)) = n.take_activation()? { /* 0 body, 1.. button */ }
//! ```
use super::*;
pub use door_service_protocol::notifications::{Identity, Post as Toast, Record, State, Status};
use door_service_protocol::{BODY_BYTES, notifications as n};

pub struct Notifications {
    connection: Connection,
}
impl Notifications {
    pub fn connect(registry: Handle) -> Result<Self> {
        Ok(Self {
            connection: Connection::connect(registry, manifest::INIT)?,
        })
    }
    /// Post a toast; Init attributes it to this process. Returns its id.
    pub fn post(&mut self, toast: &Toast<'_>) -> Result<u32> {
        let mut body = [0; BODY_BYTES];
        let len = toast.encode(&mut body).map_err(Error::Service)?;
        let reply = self.connection.call(n::POST, &body[..len], &[])?;
        let data = reply.data()?;
        if data.len() != 4 {
            return Err(Error::Malformed);
        }
        Ok(u32::from_le_bytes(data.try_into().unwrap()))
    }
    /// Remove one of this process's own notifications.
    pub fn withdraw(&mut self, id: u32) -> Result<()> {
        self.connection
            .call(n::WITHDRAW, &id.to_le_bytes(), &[])?
            .empty()
    }
    /// The next activation of one of this process's notifications:
    /// (id, 0 for the body or 1.. for a button).
    pub fn take_activation(&mut self) -> Result<Option<(u32, u32)>> {
        let reply = self.connection.call(n::TAKE_ACTIVATION, &[], &[])?;
        let data = reply.data()?;
        match data.len() {
            0 => Ok(None),
            8 => Ok(Some((
                u32::from_le_bytes(data[..4].try_into().unwrap()),
                u32::from_le_bytes(data[4..].try_into().unwrap()),
            ))),
            _ => Err(Error::Malformed),
        }
    }
    /// Desktop shell only.
    pub fn state(&mut self) -> Result<State> {
        let reply = self.connection.call(n::STATE, &[], &[])?;
        State::decode(reply.data()?).map_err(|_| Error::Malformed)
    }
    /// Desktop shell only: the record bytes of `id` (decode with
    /// `Record::decode(&out[..len])`).
    pub fn get(&mut self, id: u32, out: &mut [u8; BODY_BYTES]) -> Result<usize> {
        let reply = self.connection.call(n::GET, &id.to_le_bytes(), &[])?;
        let data = reply.data()?;
        Record::decode(data).map_err(|_| Error::Malformed)?;
        out[..data.len()].copy_from_slice(data);
        Ok(data.len())
    }
    /// Desktop shell only: dismiss one notification, or all (`id` 0).
    pub fn clear(&mut self, id: u32) -> Result<()> {
        self.connection
            .call(n::CLEAR, &id.to_le_bytes(), &[])?
            .empty()
    }
    /// Desktop shell or a workspace app (Settings): the manual Do not
    /// disturb choice; Init stores it in the NOTIFICATIONS word.
    pub fn set_dnd(&mut self, on: bool) -> Result<()> {
        self.connection
            .call(n::SET_DND, &u32::from(on).to_le_bytes(), &[])?
            .empty()
    }
    pub fn activate(&mut self, id: u32, action: u32) -> Result<()> {
        let mut body = [0; 8];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4..].copy_from_slice(&action.to_le_bytes());
        self.connection.call(n::ACTIVATE, &body, &[])?.empty()
    }
    pub fn mark_read(&mut self) -> Result<()> {
        self.connection.call(n::MARK_READ, &[], &[])?.empty()
    }
    /// Desktop shell only: Init's record of a session process (source code
    /// and admitted executable path), copied into `out`.
    pub fn app_identity(&mut self, pid: u64, out: &mut [u8; BODY_BYTES]) -> Result<usize> {
        let reply = self
            .connection
            .call(n::APP_IDENTITY, &pid.to_le_bytes(), &[])?;
        let data = reply.data()?;
        Identity::decode(data).map_err(|_| Error::Malformed)?;
        out[..data.len()].copy_from_slice(data);
        Ok(data.len())
    }
}
/// Settings > System > Notifications and the Focus footer (desktop shell and
/// the session's workspace apps). Settings stores a preference word with
/// `Storage::set_setting` first and then calls `apply_preference`, so Init
/// applies exactly what was stored.
impl Notifications {
    /// Do not disturb and Focus as they stand now.
    pub fn status(&mut self) -> Result<Status> {
        let reply = self.connection.call(n::STATUS, &[], &[])?;
        Status::decode(reply.data()?).map_err(|_| Error::Malformed)
    }
    /// Start a Focus session of `minutes` (0: the user's FOCUS length;
    /// otherwise one of `preferences::Focus::LENGTHS`). A running session
    /// restarts with the new length.
    pub fn focus_start(&mut self, minutes: u32) -> Result<()> {
        self.connection
            .call(n::FOCUS_START, &minutes.to_le_bytes(), &[])?
            .empty()
    }
    /// End the running Focus session early (no completion toast).
    pub fn focus_stop(&mut self) -> Result<()> {
        self.connection.call(n::FOCUS_STOP, &[], &[])?.empty()
    }
    /// The Focus footer's stepper: Init stores the new FOCUS length.
    pub fn focus_length(&mut self, minutes: u32) -> Result<()> {
        self.connection
            .call(n::FOCUS_LENGTH, &minutes.to_le_bytes(), &[])?
            .empty()
    }
    /// Make a stored preference word live (`preferences::NOTIFICATIONS`,
    /// `DND_SCHEDULE` or `FOCUS`).
    pub fn apply_preference(&mut self, id: u32, value: u32) -> Result<()> {
        let mut body = [0; 8];
        body[..4].copy_from_slice(&id.to_le_bytes());
        body[4..].copy_from_slice(&value.to_le_bytes());
        self.connection
            .call(n::APPLY_PREFERENCE, &body, &[])?
            .empty()
    }
}
