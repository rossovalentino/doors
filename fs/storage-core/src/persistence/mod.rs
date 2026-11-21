//! Serialized namespace, security and configuration owner above the ext2 backend.
//! Callers provide authenticated authority; all persistent updates share one redo transaction.
mod codec;
mod config;
pub mod fileops;
mod namespace;
mod nodes;
mod path;
mod security;
pub use config::{
    KeyHandle, MAX_VALUE_BYTES, RECORD_DATA, RegistryRoot, RegistryType, RegistryView, Setting,
    Value,
};
pub use namespace::{
    ACCESS_UPDATES, CHANGE_RING, CREDENTIALS_FOLDER, Change, DirectoryEntry, Disposition,
    FileHandle, FileInfo, FileTimes, OpenOptions, PersistentVolume, SESSION_UID, Staged, State,
};
pub use path::{Context, Name, Path, RegName};
pub use security::{ADMINISTRATORS, ADMINISTRATORS_OWNER, Access, Ace, Acl, Principal, Subject};
pub const CATALOG_BYTES: usize = 16384;
pub const PRODUCTION_CATALOG_BYTES: usize = 69632;
pub const CATALOG_NAME: &str = "door.catalog";
/// Registry records past the catalog's fixed slots (`config::KEYS`).
pub const HIVE_NAME: &str = "door.registry";
/// One compound document replacement, not the general filesystem size limit.
pub const REPLACE_BYTES: usize = 4096;
/// Largest staged whole-file replacement: the ext2 regular-file limit (12
/// direct and 1024 single-indirect 4 KiB blocks).
pub const LARGE_REPLACE_BYTES: usize = 4_243_456;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Storage(crate::storage::Error),
    AccessDenied,
    SharingViolation,
    DeletePending,
    KeyDeleted,
    NotFound,
    AlreadyExists,
    InvalidName,
    Unsupported,
    NoSpace,
    InvalidHandle,
    InvalidParameter,
    TypeMismatch,
    BufferTooSmall,
    Changed,
    Corrupt,
    Poisoned,
}
pub type Result<T> = core::result::Result<T, Error>;
impl From<crate::storage::Error> for Error {
    fn from(value: crate::storage::Error) -> Self {
        Self::Storage(value)
    }
}
#[cfg(test)]
mod tests;
