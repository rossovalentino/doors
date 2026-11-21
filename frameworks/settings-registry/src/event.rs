//! Runner diagnostics and the corrupt-settings notice.
use crate::schema::PropertySet;
use core::fmt;

/// Why a property was reset to its default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetReason {
    /// The persisted value failed validation.
    Corrupt,
    /// Storage reported its own state damaged; nothing was written.
    StorageCorrupt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// This registry committed the value.
    Commit,
    /// The value changed elsewhere (another process, a restart) and this
    /// registry observed it.
    Observed,
    /// A damaged value now reads as its default.
    Reset(ResetReason),
}

/// One runner diagnostic. `Display` gives the line, e.g.
/// `DOOR_P14 SETTINGS commit key=taskbar.position value=1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    pub kind: EventKind,
    pub key: &'static str,
    pub value: u32,
}
impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            EventKind::Commit => write!(
                f,
                "DOOR_P14 SETTINGS commit key={} value={}",
                self.key, self.value
            ),
            EventKind::Observed => write!(
                f,
                "DOOR_P14 SETTINGS observed key={} value={}",
                self.key, self.value
            ),
            EventKind::Reset(reason) => write!(
                f,
                "DOOR_P14 SETTINGS reset key={} reason={}",
                self.key,
                match reason {
                    ResetReason::Corrupt => "corrupt",
                    ResetReason::StorageCorrupt => "storage-corrupt",
                }
            ),
        }
    }
}

/// Shown once after damaged settings were replaced by defaults.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Notice {
    /// Properties now reading as their defaults.
    pub reset: PropertySet,
    /// Of those, the ones whose default was written back to storage.
    pub rewritten: PropertySet,
    /// Storage reported damage: defaults are in memory only and the
    /// affected properties are read-only until a clean read.
    pub storage_corrupt: bool,
}
impl Notice {
    pub const MESSAGE: &'static str =
        "Some settings were reset to their defaults because they were damaged";
    pub const fn message(&self) -> &'static str {
        Self::MESSAGE
    }
}
