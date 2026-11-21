//! The device name (`door_service_protocol::device`): read it from HKLM
//! (readable by every session process) and rename the PC through Init,
//! which holds the system access HKLM writes need.
use super::*;
use door_service_protocol::device::{self, Name};

/// Rename this PC: the Settings app of the signed-in session asks Init.
pub struct DeviceName {
    connection: Connection,
}
impl DeviceName {
    pub fn connect(registry: Handle) -> Result<Self> {
        Ok(Self {
            connection: Connection::connect(registry, manifest::INIT)?,
        })
    }
    /// Validate `name` (show `device::validate`'s message first) and store
    /// it machine-wide. Init refuses every caller but the session's Settings
    /// app (`Denied`) and any name `device::Name::parse` rejects.
    pub fn set(&mut self, name: &str) -> Result<()> {
        let name = Name::parse(name).map_err(|_| Error::Service(Code::InvalidRequest))?;
        self.connection
            .call(device::SET_NAME, name.as_str().as_bytes(), &[])?
            .empty()
    }
}
fn units<const N: usize>(text: &str) -> ([u16; N], usize) {
    let mut out = [0; N];
    let mut len = 0;
    for (slot, unit) in out.iter_mut().zip(text.encode_utf16()) {
        *slot = unit;
        len += 1;
    }
    (out, len)
}
/// The stored device name, or `None` when none was ever set (or the stored
/// value is not a valid name). Errors are transport/service failures.
pub fn stored_device_name(storage: &mut Storage) -> Result<Option<Name>> {
    let (path, path_len) = units::<48>(device::KEY_PATH);
    let key = match storage.registry_open(
        RegistryRoot::LocalMachine,
        RegistryView::Bits64,
        &path[..path_len],
        1,
        false,
    ) {
        Ok(key) => key,
        Err(Error::Service(Code::NotFound)) => return Ok(None),
        Err(error) => return Err(error),
    };
    let (value, value_len) = units::<16>(device::VALUE);
    let queried = storage.registry_query(&key, &value[..value_len]);
    let closed = storage.registry_close(key);
    let name = match queried {
        Ok(v) if v.kind == 1 => Name::from_registry(v.data()),
        Ok(_) | Err(Error::Service(Code::NotFound)) => None,
        Err(error) => return Err(error),
    };
    closed?;
    Ok(name)
}
/// The device name shown in About, Home and File Explorer's This PC:
/// the stored name, or `device::DEFAULT_NAME` ("DOORS-PC") when none is
/// stored or it cannot be read.
pub fn device_name(storage: &mut Storage) -> Name {
    stored_device_name(storage)
        .ok()
        .flatten()
        .unwrap_or_default()
}
