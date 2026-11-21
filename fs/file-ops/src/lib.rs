#![no_std]
//! Reusable, allocation-free file-operation engine (spec §61): operation IDs,
//! progress, pause/cancel/resume, conflict decisions, a native Recycle Bin and
//! durable recovery records, independent of the presentation layer.
//!
//! The engine runs against a [`Volume`]: a serialized owner whose staged
//! mutations become durable together at [`Volume::commit`]. Every step stages
//! its data changes *and* the updated recovery record, then commits once, so
//! after a crash the last committed record exactly describes the committed
//! data (P07: file bytes, namespace and record in one durable transaction).
//! Recovery resumes from that record; replaying a step is idempotent.
#[cfg(test)]
extern crate std;

mod engine;
#[cfg(test)]
mod memory;
mod naming;
mod plan;
mod recycle;
#[cfg(test)]
mod tests;

pub use engine::{Engine, Progress, STEP_BYTES, STEP_ITEMS};
pub use naming::{copy_name, numbered_name, utf16_len};
pub use plan::{MAX_SOURCES, Plan, RECORD_BYTES, crc32};
pub use recycle::{
    BIN_NAME, Bin, InfoRecord, Outcome as RecycleOutcome, RECYCLE_INFO_BYTES,
    Usage as RecycleUsage, bin_path, default_limit, info_name, item_name,
};

/// Longest absolute path the engine builds (the namespace path limit).
pub const PATH_BYTES: usize = 1024;
/// Longest component in UTF-8 (48 UTF-16 units, at most three bytes each).
pub const NAME_BYTES: usize = 144;
/// Deepest folder nesting below a source item (namespace: 16 components).
pub const MAX_DEPTH: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Mode {
    Copy = 0,
    Move = 1,
    /// Permanent deletion (Shift+Delete, Recycle Bin purge/empty).
    Delete = 2,
}
impl Mode {
    pub fn decode(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::Copy,
            1 => Self::Move,
            2 => Self::Delete,
            _ => return None,
        })
    }
}
/// What to do when a destination name exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Policy {
    Ask = 0,
    Replace = 1,
    Skip = 2,
    /// Keep both: the incoming item gets "name (2).ext".
    KeepBoth = 3,
}
impl Policy {
    pub fn decode(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::Ask,
            1 => Self::Replace,
            2 => Self::Skip,
            3 => Self::KeepBoth,
            _ => return None,
        })
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Decision {
    Replace = 1,
    Skip = 2,
    KeepBoth = 3,
}
impl Decision {
    pub fn decode(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::Replace,
            2 => Self::Skip,
            3 => Self::KeepBoth,
            _ => return None,
        })
    }
    pub fn policy(self) -> Policy {
        match self {
            Self::Replace => Policy::Replace,
            Self::Skip => Policy::Skip,
            Self::KeepBoth => Policy::KeepBoth,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum State {
    /// Sources accepted; totals not yet counted.
    Scanning = 0,
    Running = 1,
    Paused = 2,
    /// Waiting for a decision about the current item (see `Plan::conflict`).
    Conflict = 3,
    Done = 4,
    Cancelled = 5,
    Failed = 6,
}
impl State {
    pub fn decode(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::Scanning,
            1 => Self::Running,
            2 => Self::Paused,
            3 => Self::Conflict,
            4 => Self::Done,
            5 => Self::Cancelled,
            6 => Self::Failed,
            _ => return None,
        })
    }
    pub fn finished(self) -> bool {
        matches!(self, Self::Done | Self::Cancelled | Self::Failed)
    }
}
/// Backend failures, mapped by the owner from its own error type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Error {
    NotFound = 1,
    Exists = 2,
    Denied = 3,
    /// Volume, journal, node or handle capacity.
    NoSpace = 4,
    Invalid = 5,
    Sharing = 6,
    Changed = 7,
    Corrupt = 8,
    Io = 9,
    /// The owner is poisoned; nothing more can be staged until remount.
    Poisoned = 10,
    Busy = 11,
    /// A file is larger than the backend's per-file limit.
    TooLarge = 12,
}
impl Error {
    pub fn decode(value: u8) -> Option<Self> {
        Some(match value {
            1 => Self::NotFound,
            2 => Self::Exists,
            3 => Self::Denied,
            4 => Self::NoSpace,
            5 => Self::Invalid,
            6 => Self::Sharing,
            7 => Self::Changed,
            8 => Self::Corrupt,
            9 => Self::Io,
            10 => Self::Poisoned,
            11 => Self::Busy,
            12 => Self::TooLarge,
            _ => return None,
        })
    }
    /// Failures after which nothing more may be staged in this owner.
    pub fn fatal(self) -> bool {
        matches!(self, Self::Poisoned | Self::Io | Self::Corrupt)
    }
}
pub type Result<T> = core::result::Result<T, Error>;

/// Metadata of one namespace object.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Stat {
    pub object: u64,
    pub version: u64,
    pub bytes: u64,
    pub directory: bool,
    /// FILETIME; 0 = not recorded.
    pub modified: u64,
}
/// A bounded UTF-8 component.
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Name {
    bytes: [u8; NAME_BYTES],
    len: u8,
}
impl core::fmt::Debug for Name {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.as_str().fmt(f)
    }
}
impl Name {
    pub const EMPTY: Self = Self {
        bytes: [0; NAME_BYTES],
        len: 0,
    };
    /// A valid component: 1..=48 UTF-16 units, no separator, NUL or dot name.
    pub fn new(text: &str) -> Result<Self> {
        if text.is_empty()
            || text.len() > NAME_BYTES
            || utf16_len(text) > 48
            || text == "."
            || text == ".."
            || text.bytes().any(|b| b == b'/' || b == 0)
        {
            return Err(Error::Invalid);
        }
        let mut name = Self::EMPTY;
        name.bytes[..text.len()].copy_from_slice(text.as_bytes());
        name.len = text.len() as u8;
        Ok(name)
    }
    pub fn as_str(&self) -> &str {
        // Only constructed from &str.
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}
/// A bounded absolute path builder.
#[derive(Clone, Copy)]
pub struct PathBuf<const N: usize> {
    bytes: [u8; N],
    len: usize,
}
impl<const N: usize> core::fmt::Debug for PathBuf<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.as_str().fmt(f)
    }
}
impl<const N: usize> PathBuf<N> {
    pub const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
        }
    }
    pub fn from(text: &str) -> Result<Self> {
        let mut p = Self::new();
        p.set(text)?;
        Ok(p)
    }
    pub fn set(&mut self, text: &str) -> Result<()> {
        if text.len() > N {
            return Err(Error::Invalid);
        }
        self.bytes[..text.len()].copy_from_slice(text.as_bytes());
        self.len = text.len();
        Ok(())
    }
    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    /// Append `/component` (no separator is added to an empty buffer).
    pub fn push(&mut self, component: &str) -> Result<()> {
        let extra = component.len() + usize::from(self.len != 0 && !self.as_str().ends_with('/'));
        if self.len + extra > N {
            return Err(Error::Invalid);
        }
        if self.len != 0 && !self.as_str().ends_with('/') {
            self.bytes[self.len] = b'/';
            self.len += 1;
        }
        self.bytes[self.len..self.len + component.len()].copy_from_slice(component.as_bytes());
        self.len += component.len();
        Ok(())
    }
    /// Remove the last component (and its separator).
    pub fn pop(&mut self) {
        match self.as_str().rfind('/') {
            Some(at) => self.len = at,
            None => self.len = 0,
        }
    }
}
impl<const N: usize> Default for PathBuf<N> {
    fn default() -> Self {
        Self::new()
    }
}
/// `parent/child` (or `/child` under the root).
pub fn join(parent: &str, child: &str) -> Result<PathBuf<PATH_BYTES>> {
    let mut p = PathBuf::from(parent)?;
    p.push(child)?;
    Ok(p)
}
/// Last component of an absolute path.
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
/// Parent of an absolute path (`/` for top-level items).
pub fn parent_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(0) => "/",
        Some(at) => &path[..at],
        None => "",
    }
}
/// A Windows path on C: (`C:\Users\1001\x`, in any case) as the native
/// path storage keeps it (`/users/1001/x`): `C:\Users` is `/users`, the
/// rest of C: is `/compat`, and each component takes its stored spelling.
pub fn native_from_windows<V: Volume>(vol: &mut V, path: &str) -> Result<PathBuf<PATH_BYTES>> {
    let rest = path
        .strip_prefix("C:\\")
        .or_else(|| path.strip_prefix("c:\\"))
        .ok_or(Error::Invalid)?;
    let mut parts = rest.split('\\').filter(|p| !p.is_empty()).peekable();
    let mut native = PathBuf::<PATH_BYTES>::new();
    if parts
        .peek()
        .is_some_and(|p| p.eq_ignore_ascii_case("users"))
    {
        parts.next();
        native.set("/users")?;
    } else {
        native.set("/compat")?;
    }
    for part in parts {
        if part == "." || part == ".." {
            return Err(Error::Invalid);
        }
        let mut after = 0;
        let found = loop {
            match vol.child(native.as_str(), after)? {
                Some((name, _)) if name.as_str().eq_ignore_ascii_case(part) => break name,
                Some((_, stat)) => after = stat.object,
                None => return Err(Error::NotFound),
            }
        };
        native.push(found.as_str())?;
    }
    Ok(native)
}
/// `inner` equals `outer` or lies below it.
pub fn within(inner: &str, outer: &str) -> bool {
    inner == outer
        || inner.len() > outer.len()
            && inner.starts_with(outer)
            && (outer.ends_with('/') || inner.as_bytes()[outer.len()] == b'/')
}

/// The serialized storage owner the engine stages through.
///
/// Authorization is the owner's: every call re-checks the authenticated
/// subject against the current object, never against an earlier observation.
pub trait Volume {
    type File;
    fn stat(&mut self, path: &str) -> Result<Stat>;
    /// Next child of `folder` with an object ID greater than `after`
    /// (object-ID cursor: deletions never repeat entries).
    fn child(&mut self, folder: &str, after: u64) -> Result<Option<(Name, Stat)>>;
    fn mkdir(&mut self, path: &str) -> Result<()>;
    /// Create a new empty file for writing (fails `Exists`).
    fn create(&mut self, path: &str) -> Result<Self::File>;
    fn open_read(&mut self, path: &str) -> Result<Self::File>;
    fn open_write(&mut self, path: &str) -> Result<Self::File>;
    fn read(&mut self, file: &Self::File, offset: u64, out: &mut [u8]) -> Result<usize>;
    fn write(&mut self, file: &Self::File, offset: u64, data: &[u8]) -> Result<()>;
    fn truncate(&mut self, file: &Self::File, bytes: u64) -> Result<()>;
    fn close(&mut self, file: Self::File) -> Result<()>;
    /// Atomic same-volume rename/move. `replace` overwrites an existing
    /// regular file at `to` in the same staged mutation.
    fn rename(&mut self, from: &str, to: &str, replace: bool) -> Result<()>;
    /// Remove one file or empty folder.
    fn remove(&mut self, path: &str) -> Result<()>;
    /// Set (or clear) the hidden attribute.
    fn set_hidden(&mut self, path: &str, hidden: bool) -> Result<()>;
    /// Make everything staged so far durable (one redo transaction).
    fn commit(&mut self) -> Result<()>;
    /// Stage the operation record `op` (owner-private, tamper-resistant).
    fn save_record(&mut self, op: u64, bytes: &[u8]) -> Result<()>;
    fn load_record(&mut self, op: u64, out: &mut [u8]) -> Result<Option<usize>>;
    fn remove_record(&mut self, op: u64) -> Result<()>;
    /// Next stored record ID greater than `after`.
    fn next_record(&mut self, after: u64) -> Result<Option<u64>>;
    /// Largest regular file the backend can hold.
    fn file_limit(&self) -> u64;
    /// Wall time as FILETIME (0 = unavailable).
    fn now(&self) -> u64;
}
