//! Persistence and change subscription behind a registry, and the
//! implementation over the storage service client.
use crate::schema::Store;
use door_service_client::{
    Error as ClientError, RegistryRoot, RegistryValue, RegistryView, Storage,
};
use door_service_protocol::Code;
pub use door_service_protocol::files::WatchState;

/// What a backend reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendError {
    /// The value does not exist (it reads as its default).
    NotFound,
    /// The persisted value failed validation (storage `InvalidRequest`).
    Invalid,
    /// Storage reports its own state damaged (`Corrupt`): never overwrite.
    Corrupt,
    /// The caller lacks the authority for this operation.
    Denied,
    /// The connection or service instance is gone; reconnect and
    /// subscribe again.
    Disconnected,
    /// The storage write or flush failed.
    Io,
    /// Any other service code.
    Other(u32),
}
impl From<ClientError> for BackendError {
    fn from(error: ClientError) -> Self {
        match error {
            ClientError::Service(Code::NotFound) => Self::NotFound,
            ClientError::Service(Code::InvalidRequest) => Self::Invalid,
            ClientError::Service(Code::Corrupt) => Self::Corrupt,
            ClientError::Service(Code::Denied) => Self::Denied,
            ClientError::Service(Code::Io) => Self::Io,
            ClientError::Service(Code::InvalidHandle | Code::StaleInstance | Code::Closing)
            | ClientError::Transport(_)
            | ClientError::Malformed => Self::Disconnected,
            ClientError::Service(code) => Self::Other(code as u32),
        }
    }
}

/// Persistence for registry words. One process normally holds one backend
/// connection; every registry call takes it by reference, so the caller
/// keeps ownership (and replaces it on reconnect).
pub trait Backend {
    /// The persisted word (validated by the service where it validates).
    fn read(&mut self, store: Store) -> Result<u32, BackendError>;
    /// Persist the word durably; an error means the old value is kept.
    fn write(&mut self, store: Store, value: u32) -> Result<(), BackendError>;
    /// Subscribe to the caller's settings changes (and machine ones).
    fn watch(&mut self) -> Result<u64, BackendError>;
    /// The subscription's change sequence.
    fn poll(&mut self, token: u64) -> Result<WatchState, BackendError>;
}

const KEY_READ: u16 = 1;
const KEY_WRITE: u16 = 3;
fn units(text: &str, out: &mut [u16]) -> Result<usize, BackendError> {
    let mut length = 0;
    for unit in text.encode_utf16() {
        *out.get_mut(length)
            .ok_or(BackendError::Other(Code::InvalidRequest as u32))? = unit;
        length += 1;
    }
    Ok(length)
}
fn open(
    storage: &mut Storage,
    root: RegistryRoot,
    path: &str,
    write: bool,
) -> Result<door_service_client::Key, BackendError> {
    let mut buffer = [0u16; 96];
    let length = units(path, &mut buffer)?;
    Ok(storage.registry_open(
        root,
        RegistryView::Bits64,
        &buffer[..length],
        if write { KEY_WRITE } else { KEY_READ },
        write,
    )?)
}

/// The storage service: `Setting` words through GET/SET_SETTING (each set
/// flushed), registry dwords through the registry operations.
impl Backend for Storage {
    fn read(&mut self, store: Store) -> Result<u32, BackendError> {
        match store {
            Store::Setting(setting) => Ok(self.get_setting(setting)?),
            Store::Registry { root, path, name } => {
                let key = open(self, root, path, false)?;
                let mut buffer = [0u16; 48];
                let result = units(name, &mut buffer)
                    .and_then(|n| Ok(self.registry_query(&key, &buffer[..n])?));
                let _ = self.registry_close(key);
                let value = result?;
                if value.kind != 4 {
                    return Err(BackendError::Invalid);
                }
                Ok(u32::from_le_bytes(
                    value.data().try_into().map_err(|_| BackendError::Invalid)?,
                ))
            }
        }
    }
    fn write(&mut self, store: Store, value: u32) -> Result<(), BackendError> {
        match store {
            Store::Setting(setting) => self.set_setting(setting, value)?,
            Store::Registry { root, path, name } => {
                let key = open(self, root, path, true)?;
                let mut buffer = [0u16; 48];
                let result = units(name, &mut buffer).and_then(|n| {
                    Ok(self.registry_set(&key, &buffer[..n], RegistryValue::dword(value))?)
                });
                let _ = self.registry_close(key);
                result?;
            }
        }
        Ok(self.flush()?)
    }
    fn watch(&mut self) -> Result<u64, BackendError> {
        Ok(self.watch_settings()?)
    }
    fn poll(&mut self, token: u64) -> Result<WatchState, BackendError> {
        Ok(self.watch_poll(token)?)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn client_errors_map_to_registry_outcomes() {
        let map = |e| BackendError::from(e);
        assert_eq!(
            map(ClientError::Service(Code::NotFound)),
            BackendError::NotFound
        );
        assert_eq!(
            map(ClientError::Service(Code::InvalidRequest)),
            BackendError::Invalid
        );
        assert_eq!(
            map(ClientError::Service(Code::Corrupt)),
            BackendError::Corrupt
        );
        assert_eq!(
            map(ClientError::Service(Code::Denied)),
            BackendError::Denied
        );
        assert_eq!(map(ClientError::Service(Code::Io)), BackendError::Io);
        assert_eq!(map(ClientError::Malformed), BackendError::Disconnected);
        assert_eq!(
            map(ClientError::Service(Code::StaleInstance)),
            BackendError::Disconnected
        );
        assert_eq!(
            map(ClientError::Service(Code::Resources)),
            BackendError::Other(6)
        );
    }
    #[test]
    fn names_longer_than_the_wire_limit_are_refused() {
        let mut out = [0u16; 4];
        assert_eq!(units("abcd", &mut out), Ok(4));
        assert!(units("abcde", &mut out).is_err());
    }
}
