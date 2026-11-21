use super::{
    Access, Acl, CATALOG_BYTES, CATALOG_NAME, Context, Error, HIVE_NAME, Name, Path, Result,
    Subject, codec, config, nodes, path,
};
use crate::storage::{FileId, Vfs};
use core::sync::atomic::{AtomicU64, Ordering};
pub(crate) const NODES: usize = 32;
pub(crate) const PRODUCTION_NODES: usize = 256;
pub(crate) const EMPTY_ID: FileId = FileId {
    volume: [0; 16],
    inode: 0,
    generation: 0,
};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Node {
    pub id: u64,
    pub parent: u64,
    pub kind: u8,
    pub pending: bool,
    pub name: Name,
    pub backing: FileId,
    pub acl: Acl,
    pub attributes: u32,
    pub version: u64,
    pub times: FileTimes,
}
impl Node {
    pub const EMPTY: Self = Self {
        id: 0,
        parent: 0,
        kind: 0,
        pending: false,
        name: Name::EMPTY,
        backing: EMPTY_ID,
        acl: Acl::empty(1, 0),
        attributes: 0,
        version: 0,
        times: FileTimes::NONE,
    };
}
/// Catalog timestamps as FILETIME (100 ns since 1601-01-01 UTC, the
/// `door_abi::clock` epoch/unit, 1-second RTC precision); 0 = not recorded
/// (created before catalog v4/v5, or no valid wall clock at that moment).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FileTimes {
    pub created: u64,
    /// Last content write (directories: last child binding change).
    pub modified: u64,
    /// Last access. Read-only access does not update it (see `ACCESS_UPDATES`).
    pub accessed: u64,
}
impl FileTimes {
    pub const NONE: Self = Self {
        created: 0,
        modified: 0,
        accessed: 0,
    };
    /// Largest stored value (Windows FILETIME is a signed 64-bit count).
    pub const MAX: u64 = i64::MAX as u64;
    pub const fn at(now: u64) -> Self {
        Self {
            created: now,
            modified: now,
            accessed: now,
        }
    }
    pub const fn valid(self) -> bool {
        self.created <= Self::MAX && self.modified <= Self::MAX && self.accessed <= Self::MAX
    }
}
/// Read-only access does not update `accessed`, like Windows with
/// NtfsDisableLastAccessUpdate (the default on current Windows): the access
/// time is set at creation and at every content modification only. This
/// avoids a catalog write (and journal transaction) per read.
pub const ACCESS_UPDATES: bool = false;
/// Entries kept for change subscribers (`changes_since`).
/// The session service's private Argon2id credential records (WS-C).
pub const CREDENTIALS_FOLDER: &str = "/state/credentials";
/// Immutable identity of the session service (its manifest policy UID).
pub const SESSION_UID: u32 = 2;
pub const CHANGE_RING: usize = 32;
/// One observed change, for directory-change and settings subscriptions.
/// Volatile notification state: it confers no authority and is not
/// persisted (a subscriber that missed entries sees an overflow and
/// rescans, like a directory-change buffer overflow on Windows).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Change {
    /// A child of this folder object was created, removed, renamed, or
    /// changed content or size.
    Folder(u64),
    /// A setting or registry value of this user changed (0 = machine).
    Settings(u32),
}
#[derive(Clone, Copy)]
struct ChangeLog {
    sequence: u64,
    ring: [(u64, Change); CHANGE_RING],
}
impl ChangeLog {
    const fn new() -> Self {
        Self {
            sequence: 0,
            ring: [(0, Change::Folder(0)); CHANGE_RING],
        }
    }
}
#[derive(Clone, Copy)]
struct Open {
    token: u64,
    node: u64,
    access: Access,
    share: Access,
}
impl Open {
    const EMPTY: Self = Self {
        token: 0,
        node: 0,
        access: Access(0),
        share: Access(0),
    };
}
pub struct State {
    /// Catalog v1-v5: every node. Catalog v6 (`tree`): a write-back cache
    /// of the nodes in `door.nodes`, `stored` naming what the tree has for
    /// each slot (its node's id and record digest; id 0: nothing), `used`
    /// the slots' last use.
    pub(crate) nodes: [Node; PRODUCTION_NODES],
    pub(crate) stored: [nodes::Stored; PRODUCTION_NODES],
    used: [u64; PRODUCTION_NODES],
    tick: u64,
    pub(crate) capacity: usize,
    /// Catalog v6: the nodes live in the `door.nodes` tree.
    pub(crate) tree: bool,
    pub(crate) work: crate::storage::btree::Work,
    scan: nodes::Scan,
    pub(crate) config: config::Store,
    pub(crate) next_object: u64,
    pub(crate) revision: u64,
    opens: [Open; OPENS],
    next_handle: u64,
    mount: u64,
}
/// Open instances the volume tracks at once.
pub(crate) const OPENS: usize = 256;
impl State {
    pub const fn new() -> Self {
        Self {
            nodes: [Node::EMPTY; PRODUCTION_NODES],
            stored: [nodes::Stored::NONE; PRODUCTION_NODES],
            used: [0; PRODUCTION_NODES],
            tick: 0,
            capacity: NODES,
            tree: false,
            work: crate::storage::btree::Work::new(),
            scan: nodes::Scan::new(),
            config: config::Store::new(),
            next_object: 1,
            revision: 0,
            opens: [Open::EMPTY; OPENS],
            next_handle: 1,
            mount: 0,
        }
    }
}
impl State {
    /// Construct only in static storage in protected services.
    pub const fn production() -> Self {
        Self {
            capacity: PRODUCTION_NODES,
            ..Self::new()
        }
    }
    /// Select on an unused static destination before initialization; no large
    /// temporary State or node array is constructed on the caller's stack.
    pub fn select_production(&mut self) -> Result<()> {
        if self.nodes.iter().any(|n| n.id != 0) {
            return Err(Error::InvalidParameter);
        }
        self.capacity = PRODUCTION_NODES;
        Ok(())
    }
    pub fn node_capacity(&self) -> usize {
        self.capacity
    }
    pub(crate) fn mount_identity(&self) -> u64 {
        self.mount
    }
    pub(crate) fn clear(&mut self) {
        self.nodes.fill(Node::EMPTY);
        self.stored.fill(nodes::Stored::NONE);
        self.used.fill(0);
        self.tick = 0;
        self.tree = false;
        self.config.keys.fill(config::Key::EMPTY);
        self.config.values.fill(config::Entry::EMPTY);
        self.config.settings.fill(config::Settings::EMPTY);
        self.config.next_key = 1;
        self.config.clear_handles();
        self.opens.fill(Open::EMPTY);
        self.next_object = 1;
        self.revision = 0;
        self.next_handle = 1;
        self.mount = 0;
    }
}
impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}
/// Tests: a boxed State built in its own frame (it is a third of a MiB; a
/// test holding several would overflow its thread's stack in debug).
#[cfg(test)]
impl State {
    #[inline(never)]
    pub(crate) fn boxed() -> alloc_box::boxed::Box<Self> {
        alloc_box::boxed::Box::new(Self::new())
    }
    #[inline(never)]
    pub(crate) fn boxed_production() -> alloc_box::boxed::Box<Self> {
        alloc_box::boxed::Box::new(Self::production())
    }
}
#[cfg(test)]
extern crate std as alloc_box;
static NEXT_MOUNT: AtomicU64 = AtomicU64::new(1);
/// An owned capability. It cannot be forged or cloned outside this module.
#[derive(Debug)]
pub struct FileHandle {
    mount: u64,
    token: u64,
}
#[derive(Clone, Copy, Debug)]
pub enum Disposition {
    CreateNew,
    OpenExisting,
    OpenAlways,
    CreateAlways,
    TruncateExisting,
}
#[derive(Clone, Copy, Debug)]
pub struct OpenOptions {
    pub access: Access,
    pub share: Access,
    pub disposition: Disposition,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileInfo {
    pub object: u64,
    pub backing: FileId,
    pub bytes: u64,
    pub directory: bool,
    pub delete_pending: bool,
    pub attributes: u32,
    pub version: u64,
    pub times: FileTimes,
}
/// A staged whole-file replacement (`stage_replace`): an owned capability for
/// one parentless, delete-pending catalog file node whose backing receives the
/// candidate bytes over many bounded transactions. No path, listing or object
/// lookup reaches it; abort or a validated remount reclaims it.
#[derive(Debug)]
pub struct Staged {
    mount: u64,
    node: u64,
    target: u64,
    version: u64,
    bytes: u64,
    written: u64,
    /// Blocks of the volume reservation this candidate has not written yet.
    reserved: u64,
}
impl Staged {
    /// Candidate bytes received so far (the next sequential offset).
    pub fn written(&self) -> u64 {
        self.written
    }
    /// Declared candidate length.
    pub fn len(&self) -> u64 {
        self.bytes
    }
    pub fn is_empty(&self) -> bool {
        self.bytes == 0
    }
}
/// Staged pages committed before each candidate write, so one write (at most
/// 4 KiB) always fits the journal working set.
const STAGE_BUDGET: usize = 24;
/// Blocks a candidate of `bytes` occupies: data blocks plus the
/// single-indirect block past the 12 direct blocks (monotonic in `bytes`).
const fn stage_blocks(bytes: u64) -> u64 {
    bytes.div_ceil(4096) + (bytes > 12 * 4096) as u64
}
#[derive(Clone, Copy, Debug)]
pub struct DirectoryEntry {
    pub name: Name,
    pub info: FileInfo,
}
pub struct PersistentVolume<'a, F> {
    pub(crate) fs: F,
    pub(crate) state: &'a mut State,
    buffer: &'a mut [u8],
    catalog: FileId,
    /// Catalog v6: the `door.nodes` tree file.
    pub(crate) nodes_file: Option<FileId>,
    /// The registry hive file, once a record needed it.
    hive: Option<FileId>,
    /// Header of the hive as last read or written (`codec::hive::identity`).
    hive_written: [u8; codec::hive::HEADER],
    pub(crate) poisoned: bool,
    pub(crate) dirty: bool,
    /// Wall time stamped by mutations (`set_clock`); 0 = unavailable.
    clock: u64,
    /// Blocks promised to live staged replacements and not yet written.
    staged_reserved: u64,
    changes: ChangeLog,
}
pub(crate) fn backing_name(id: u64, output: &mut [u8; 19]) -> &str {
    output[..3].copy_from_slice(b"obj");
    for i in 0..16 {
        output[3 + i] = b"0123456789abcdef"[((id >> ((15 - i) * 4)) & 15) as usize];
    }
    core::str::from_utf8(output).unwrap()
}
impl<'a, F: Vfs> PersistentVolume<'a, F> {
    /// Allocation usage of the backing volume as (total bytes, free bytes).
    pub fn usage(&self) -> Option<(u64, u64)> {
        self.fs.usage()
    }
    pub fn initialize<const B: usize>(
        mut fs: F,
        state: &'a mut State,
        buffer: &'a mut [u8; B],
        subject: Subject,
    ) -> Result<Self> {
        if !subject.system() {
            return Err(Error::AccessDenied);
        }
        if B < codec::catalog_bytes(state.capacity)? {
            return Err(Error::BufferTooSmall);
        }
        match fs.open(CATALOG_NAME) {
            Err(crate::storage::Error::NotFound) => (),
            Ok(_) => return Err(Error::AlreadyExists),
            Err(e) => return Err(e.into()),
        }
        state.clear();
        let mut protected = Acl::owner_full(1);
        protected.integrity = 1;
        protected.push(super::Ace {
            principal: super::Principal::Everyone,
            deny: false,
            access: Access::READ.union(Access::TRAVERSE),
            inherit: true,
        })?;
        for (index, (parent, name)) in [
            (1, "root"),
            (1, "system"),
            (1, "users"),
            (1, "state"),
            (1, "apps"),
            (1, "cache"),
            (1, "tmp"),
            (1, "volumes"),
            (1, "devices"),
            (1, "compat"),
            (10, "Windows"),
            (11, "System32"),
            (2, "versions"),
            (2, "current"),
            (4, "registry"),
            (4, "machine"),
        ]
        .into_iter()
        .enumerate()
        {
            state.nodes[index] = Node {
                id: index as u64 + 1,
                parent,
                kind: 2,
                name: Name::ascii(name)?,
                acl: protected,
                version: 1,
                ..Node::EMPTY
            };
        }
        state.next_object = 17;
        state.revision = 1;
        config::initialize(&mut state.config)?;
        let catalog = fs.create(CATALOG_NAME)?;
        // A volume at scale keeps its nodes in a tree (catalog v6), written
        // at the first flush; a constrained volume keeps catalog v4/v5.
        let nodes_file = if fs.scaled() {
            let file = fs.create(nodes::NODES_NAME)?;
            nodes::create(&mut fs, file, &mut state.work)?;
            state.tree = true;
            Some(file)
        } else {
            None
        };
        Self::attach(fs, state, buffer, catalog, None, true, nodes_file)
    }
    pub fn mount<const B: usize>(
        mut fs: F,
        state: &'a mut State,
        buffer: &'a mut [u8; B],
    ) -> Result<Self> {
        let catalog = fs.open(CATALOG_NAME)?;
        let length = usize::try_from(fs.metadata(catalog)?.bytes).map_err(|_| Error::Corrupt)?;
        if ![CATALOG_BYTES, super::PRODUCTION_CATALOG_BYTES].contains(&length)
            || length > buffer.len()
            || fs.read(catalog, 0, &mut buffer[..length])? != length
        {
            return Err(Error::Corrupt);
        }
        let migrated = codec::decode_catalog(&buffer[..length], state)?;
        // Registry records past the catalog's slots (a missing hive: none).
        let mut hive_written = [0; codec::hive::HEADER];
        let hive = match fs.open(HIVE_NAME) {
            Ok(hive) => {
                let length =
                    usize::try_from(fs.metadata(hive)?.bytes).map_err(|_| Error::Corrupt)?;
                if fs.read(hive, 0, &mut hive_written)? != codec::hive::HEADER {
                    return Err(Error::Corrupt);
                }
                let extent = codec::hive::header_extent(&hive_written, length)?;
                let mut crc = codec::hive::CRC_START;
                for (offset, bytes) in codec::hive::chunks(extent, buffer.len()) {
                    let at = (codec::hive::HEADER + offset) as u64;
                    if fs.read(hive, at, &mut buffer[..bytes])? != bytes {
                        return Err(Error::Corrupt);
                    }
                    crc = codec::hive::decode_chunk(state, extent, offset, &buffer[..bytes], crc)?;
                }
                if !codec::hive::crc_matches(&hive_written, crc) {
                    return Err(Error::Corrupt);
                }
                Some(hive)
            }
            Err(crate::storage::Error::NotFound) => None,
            Err(e) => return Err(e.into()),
        };
        codec::validate(state)?;
        for node in &state.nodes {
            if node.id != 0 && node.kind == 1 {
                let mut name = [0; 19];
                if fs.open(backing_name(node.id, &mut name))? != node.backing {
                    return Err(Error::Corrupt);
                }
                fs.metadata(node.backing)?;
            }
        }
        // Catalog v6 keeps its nodes in `door.nodes`; an older catalog's
        // nodes (all in the cache) move there at the first flush, written
        // with the v6 catalog in one transaction (a tree left by an
        // interrupted move is started again).
        let upgrading = !state.tree && fs.scaled();
        let nodes_file = if state.tree {
            Some(fs.open(nodes::NODES_NAME)?)
        } else if !upgrading {
            None
        } else {
            let file = match fs.open(nodes::NODES_NAME) {
                Ok(file) => {
                    fs.truncate(file, 0)?;
                    file
                }
                Err(crate::storage::Error::NotFound) => fs.create(nodes::NODES_NAME)?,
                Err(e) => return Err(e.into()),
            };
            nodes::create(&mut fs, file, &mut state.work)?;
            state.tree = true;
            Some(file)
        };
        let mut volume = Self::attach(
            fs,
            state,
            buffer,
            catalog,
            hive,
            migrated || upgrading,
            nodes_file,
        )?;
        volume.hive_written = hive_written;
        // Open instances disappear at reboot; a persisted delete disposition is
        // completed in the next enclosing journal transaction before publication.
        for i in 0..volume.state.capacity {
            if volume.state.nodes[i].pending {
                volume.reclaim(i)?;
            }
        }
        loop {
            let mut ids = [0u64; 32];
            let n = volume.pending_ids(&mut ids)?;
            for &id in &ids[..n] {
                let i = volume.index(id)?;
                if volume.state.nodes[i].pending {
                    volume.reclaim(i)?;
                }
            }
            volume.flush_nodes()?;
            if n < ids.len() {
                break;
            }
        }
        Ok(volume)
    }
    fn attach(
        fs: F,
        state: &'a mut State,
        buffer: &'a mut [u8],
        catalog: FileId,
        hive: Option<FileId>,
        dirty: bool,
        nodes_file: Option<FileId>,
    ) -> Result<Self> {
        state.mount = NEXT_MOUNT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Error::NoSpace)?;
        state.opens.fill(Open::EMPTY);
        state.next_handle = 1;
        state.config.clear_handles();
        Ok(Self {
            fs,
            state,
            buffer,
            catalog,
            nodes_file,
            hive,
            hive_written: [0; codec::hive::HEADER],
            poisoned: false,
            dirty,
            clock: 0,
            staged_reserved: 0,
            changes: ChangeLog::new(),
        })
    }
    /// Set the wall time (FILETIME, `door_abi::clock`) that subsequent
    /// mutations record; 0 when no valid clock exists. While it is 0, new
    /// nodes record "not recorded" and existing nodes keep the times they
    /// have: a last known time is never erased for want of a clock.
    pub fn set_clock(&mut self, now: u64) {
        self.clock = if now <= FileTimes::MAX { now } else { 0 };
    }
    /// The wall time mutations currently record (0 = unavailable).
    pub fn clock(&self) -> u64 {
        self.clock
    }
    /// A new binding or removal changed directory `parent`'s contents.
    fn touch_directory(&mut self, parent: u64) {
        self.note(Change::Folder(parent));
        if self.clock != 0
            && let Ok(p) = self.index(parent)
        {
            self.state.nodes[p].times.modified = self.clock;
            // The folder's own last-write time changed: its parent's
            // subscribers see it too (Windows FILE_NOTIFY_CHANGE_LAST_WRITE
            // on a subfolder), so folder-content icons refresh.
            let grandparent = self.state.nodes[p].parent;
            if grandparent != 0 && grandparent != parent {
                self.note(Change::Folder(grandparent));
            }
        }
    }
    /// Record a change for subscribers (never fails; the oldest drop).
    pub(crate) fn note(&mut self, change: Change) {
        let log = &mut self.changes;
        if log.sequence != 0
            && log.ring[log.sequence as usize % CHANGE_RING] == (log.sequence, change)
        {
            return;
        }
        log.sequence = log.sequence.wrapping_add(1).max(1);
        log.ring[log.sequence as usize % CHANGE_RING] = (log.sequence, change);
    }
    /// Sequence of the newest recorded change (0 = none yet).
    pub fn change_sequence(&self) -> u64 {
        self.changes.sequence
    }
    /// Visit every change after `seen`. Returns false when entries after
    /// `seen` were already dropped (the subscriber must rescan).
    pub fn changes_since(&self, seen: u64, mut visit: impl FnMut(Change)) -> bool {
        let log = &self.changes;
        if seen >= log.sequence {
            return true;
        }
        let complete = log.sequence - seen <= CHANGE_RING as u64;
        let first = log
            .sequence
            .saturating_sub(CHANGE_RING as u64 - 1)
            .max(seen + 1);
        for sequence in first..=log.sequence {
            let (stored, change) = log.ring[sequence as usize % CHANGE_RING];
            if stored == sequence {
                visit(change);
            }
        }
        complete
    }
    /// The namespace object still exists (not removed or delete-pending).
    pub fn object_alive(&mut self, object: u64) -> bool {
        self.index(object)
            .is_ok_and(|i| !self.state.nodes[i].pending)
    }
    /// Object ID of `path` for change subscriptions; requires the same
    /// READ|TRAVERSE right as listing the folder.
    pub fn watch_target(&mut self, subject: Subject, path: Path<'_>) -> Result<u64> {
        let info = self.metadata(subject, path, Context::root())?;
        if !info.directory {
            return Err(Error::InvalidParameter);
        }
        Ok(info.object)
    }
    /// Create the session service's private credential folder when it is
    /// missing (volume initialization and every later mount of an older
    /// volume). Owned by `SESSION_UID` with an ACL granting only that
    /// identity: no user, application or other service reaches it.
    /// Returns whether it was created (staged; the caller commits).
    pub fn ensure_credentials(&mut self, system: Subject) -> Result<bool> {
        self.ready()?;
        if !system.system() {
            return Err(Error::AccessDenied);
        }
        match self.resolve(system, Path::Native(CREDENTIALS_FOLDER), Context::root()) {
            Ok(i) if self.state.nodes[i].kind == 2 => Ok(false),
            Ok(_) => Err(Error::Corrupt),
            Err(Error::NotFound) => {
                self.mkdir(
                    system,
                    Path::Native(CREDENTIALS_FOLDER),
                    Context::root(),
                    Some(Acl::owner_full(SESSION_UID)),
                )?;
                Ok(true)
            }
            Err(e) => Err(e),
        }
    }
    /// The Windows C: layout on a new or older volume (every session mount):
    /// `Program Files`, `Program Files (x86)`, `ProgramData` and
    /// `Windows\Installer` readable by everyone and writable by
    /// Administrators (an elevated process), the
    /// shared `C:\Users\Public` folders, and each profile's known folders
    /// (`AppData\Local`, `LocalLow`, `Roaming`, `Local\Temp`, Desktop,
    /// Documents, Downloads, ...) owned by its user, AppData hidden as on
    /// Windows. A system folder created by an older build gets the current
    /// ACL. Returns how many folders it created or changed (staged; the
    /// caller commits).
    pub fn ensure_windows_layout(&mut self, system: Subject) -> Result<u32> {
        use super::security::{ADMINISTRATORS, Ace, Principal};
        self.ready()?;
        if !system.system() {
            return Err(Error::AccessDenied);
        }
        let mut changed = 0;
        let ace = |principal, access: Access| Ace {
            principal,
            deny: false,
            access,
            inherit: true,
        };
        let mut machine = Acl::owner_full(1);
        machine.integrity = 1;
        machine.push(ace(
            Principal::Everyone,
            Access::READ.union(Access::TRAVERSE),
        ))?;
        machine.push(ace(Principal::Group(ADMINISTRATORS), Access::ALL))?;
        let mut public = Acl::owner_full(1);
        public.push(ace(
            Principal::Everyone,
            Access(Access::ALL.0 & !Access::CHANGE_ACL.0),
        ))?;
        public.push(ace(Principal::Group(ADMINISTRATORS), Access::ALL))?;
        for (path, acl) in [
            ("/compat/Program Files", machine),
            ("/compat/Program Files (x86)", machine),
            // Common Files in both trees, as Windows ships them
            // (FOLDERID_ProgramFilesCommon and ProgramFilesCommonX86).
            ("/compat/Program Files/Common Files", machine),
            ("/compat/Program Files (x86)/Common Files", machine),
            ("/compat/ProgramData", machine),
            // Windows Installer's package cache (C:\Windows\Installer):
            // written by elevated installations only.
            ("/compat/Windows/Installer", machine),
            // System32 and its catalog store (CryptCATAdminAddCatalog's
            // CatRoot\{DRIVER_ACTION_VERIFY}), which Windows ships with
            // Administrators allowed to change them.
            ("/compat/Windows/System32", machine),
            ("/compat/Windows/System32/CatRoot", machine),
            (
                "/compat/Windows/System32/CatRoot/{F750E6C3-38EE-11D1-85E5-00C04FC295EE}",
                machine,
            ),
            // WOW64's 32-bit system folder (P17): a 32-bit installer's
            // System32 writes land here.
            ("/compat/Windows/SysWOW64", machine),
            // FOLDERID_Fonts, where Windows Installer's Font table and
            // installers copy fonts (Administrators may write it).
            ("/compat/Windows/Fonts", machine),
            ("/users/Public", public),
        ] {
            match self.resolve(system, Path::Native(path), Context::root()) {
                Ok(i) if self.state.nodes[i].kind == 2 => {
                    // Only a system-owned folder is brought up to date.
                    if self.state.nodes[i].acl.owner == 1 && self.state.nodes[i].acl != acl {
                        let version = self.state.nodes[i]
                            .version
                            .checked_add(1)
                            .ok_or(Error::NoSpace)?;
                        self.mutation()?;
                        self.state.nodes[i].acl = acl;
                        self.state.nodes[i].version = version;
                        changed += 1;
                    }
                }
                Ok(_) => return Err(Error::Corrupt),
                Err(Error::NotFound) => {
                    self.mkdir(system, Path::Native(path), Context::root(), Some(acl))?;
                    changed += 1;
                }
                Err(e) => return Err(e),
            }
        }
        // The catalog holds 256 objects: the shared folders and each profile
        // get the known folders programs write to, not Windows' full set
        // (Contacts, Favorites, Links, Saved Games, Searches).
        for folder in ["Desktop", "Documents"] {
            changed += self.ensure_folder(system, "/users/Public", folder, None, 0)?;
        }
        // Every profile folder (`/users/<uid>`), owned by its user.
        let mut profiles = [0u32; 16];
        let mut count = 0;
        let mut after = 0;
        while let Some(i) = self.next_child(3, after)? {
            let n = self.state.nodes[i];
            after = n.id;
            if n.kind != 2 || n.pending {
                continue;
            }
            let units = n.name.units();
            if units.is_empty()
                || units.len() > 9
                || !units.iter().all(|u| (0x30..=0x39).contains(u))
            {
                continue;
            }
            let uid = units.iter().try_fold(0u32, |v, u| {
                v.checked_mul(10)?.checked_add(u32::from(*u - 0x30))
            });
            if let Some(uid) = uid.filter(|uid| *uid >= 1000)
                && count < profiles.len()
            {
                profiles[count] = uid;
                count += 1;
            }
        }
        for &uid in &profiles[..count] {
            let mut home = PathText::new();
            let _ = core::fmt::Write::write_fmt(&mut home, format_args!("/users/{uid}"));
            for (parent, folder, attributes) in [
                ("", "AppData", 0x2),
                ("/AppData", "Local", 0),
                ("/AppData/Local", "Temp", 0),
                ("/AppData", "LocalLow", 0),
                ("/AppData", "Roaming", 0),
                ("", "Desktop", 0),
                ("", "Documents", 0),
                ("", "Downloads", 0),
                ("", "Music", 0),
                ("", "Pictures", 0),
                ("", "Videos", 0),
            ] {
                let mut at = PathText::new();
                let _ =
                    core::fmt::Write::write_fmt(&mut at, format_args!("{}{parent}", home.text()));
                changed += self.ensure_folder(system, at.text(), folder, Some(uid), attributes)?;
            }
        }
        Ok(changed)
    }
    /// `parent/name` as a folder inheriting `parent`'s ACL (owned by
    /// `owner`, else the parent's owner), with `attributes`; 1 if created.
    fn ensure_folder(
        &mut self,
        system: Subject,
        parent: &str,
        name: &str,
        owner: Option<u32>,
        attributes: u32,
    ) -> Result<u32> {
        let mut buffer = PathText::new();
        core::fmt::Write::write_fmt(&mut buffer, format_args!("{parent}/{name}"))
            .map_err(|_| Error::InvalidName)?;
        let path = buffer.text();
        // A profile's folders are found and created as its user would (the
        // system has no access inside a profile).
        let creator = match owner {
            // SAFETY: the system provisions this user's own profile; the
            // identity is the folder's owner, never a request's.
            Some(uid) => unsafe { Subject::authenticated(uid, 0, 1) }?,
            None => system,
        };
        match self.resolve(creator, Path::Native(path), Context::root()) {
            Ok(_) => Ok(0),
            Err(Error::NotFound) => {
                let p = self.resolve(creator, Path::Native(parent), Context::root())?;
                let parent_acl = self.state.nodes[p].acl;
                let acl = parent_acl.inherited(owner.unwrap_or(parent_acl.owner));
                self.mkdir(creator, Path::Native(path), Context::root(), Some(acl))?;
                if attributes != 0 {
                    let i = self.resolve(creator, Path::Native(path), Context::root())?;
                    self.state.nodes[i].attributes = attributes;
                }
                Ok(1)
            }
            Err(e) => Err(e),
        }
    }
    /// Re-check a subscription against the current object: every ancestor
    /// still grants TRAVERSE and the folder READ|TRAVERSE (rights can change
    /// after subscribing; a subscriber must not keep observing).
    pub fn watch_allowed(&mut self, subject: Subject, object: u64) -> Result<()> {
        self.ready()?;
        let i = self.current_parent(subject, object)?;
        self.state.nodes[i]
            .acl
            .check(subject, Access::READ.union(Access::TRAVERSE))
    }
    /// A content write: modification (and, by policy, access) time.
    fn touch_content(&mut self, index: usize) {
        self.note(Change::Folder(self.state.nodes[index].parent));
        if self.clock != 0 {
            let times = &mut self.state.nodes[index].times;
            times.modified = self.clock;
            times.accessed = self.clock;
        }
    }
    pub(crate) fn ready(&self) -> Result<()> {
        if self.poisoned {
            Err(Error::Poisoned)
        } else {
            Ok(())
        }
    }
    pub(crate) fn mutation(&mut self) -> Result<()> {
        self.ready()?;
        self.state.revision = self.state.revision.checked_add(1).ok_or(Error::NoSpace)?;
        self.dirty = true;
        Ok(())
    }
    fn io<T>(&mut self, result: crate::storage::Result<T>) -> Result<T> {
        match result {
            Ok(v) => Ok(v),
            Err(e) => {
                if e != crate::storage::Error::NoSpace || self.fs.mutation_uncertain() {
                    self.poisoned = true;
                }
                Err(e.into())
            }
        }
    }
    /// A constrained volume's catalog holds names of at most 48 units (a
    /// longer one never existed there: InvalidName, as before catalog v6).
    fn storable(&self, name: &Name) -> Result<()> {
        if !self.state.tree && usize::from(name.len) > path::CATALOG_NAME_UNITS {
            return Err(Error::InvalidName);
        }
        Ok(())
    }
    fn walk(&mut self, subject: Subject, start: u64, names: &[Name], fold: bool) -> Result<usize> {
        for name in names {
            self.storable(name)?;
        }
        let mut index = self.index(start)?;
        for name in names {
            let current = self.state.nodes[index];
            if current.kind != 2 {
                return Err(Error::InvalidParameter);
            }
            if current.pending {
                return Err(Error::DeletePending);
            }
            current.acl.check(subject, Access::TRAVERSE)?;
            if name.units() == [46] {
                continue;
            }
            if name.units() == [46, 46] {
                index = self.index(if (current.id == 3 || current.id == 10) && fold {
                    10
                } else {
                    current.parent
                })?;
                continue;
            }
            index = self.child(current.id, *name, fold)?;
        }
        if self.state.nodes[index].pending {
            return Err(Error::DeletePending);
        }
        Ok(index)
    }
    fn resolve(&mut self, subject: Subject, path: Path<'_>, context: Context) -> Result<usize> {
        self.check_context(context)?;
        let parsed = path::parse(path, context)?;
        self.walk(
            subject,
            parsed.start,
            &parsed.components[..parsed.len],
            parsed.fold,
        )
    }
    fn parent(
        &mut self,
        subject: Subject,
        path: Path<'_>,
        context: Context,
    ) -> Result<(usize, Name, bool)> {
        self.check_context(context)?;
        let p = path::parse(path, context)?;
        if p.len == 0 {
            return Err(Error::InvalidName);
        }
        let name = p.components[p.len - 1];
        if name.units() == [46] || name.units() == [46, 46] {
            return Err(Error::InvalidName);
        }
        self.storable(&name)?;
        let index = self.walk(subject, p.start, &p.components[..p.len - 1], p.fold)?;
        let parent = self.state.nodes[index];
        if parent.kind != 2 {
            return Err(Error::InvalidParameter);
        }
        parent.acl.check(subject, Access::TRAVERSE)?;
        Ok((index, name, p.fold))
    }
    pub fn change_directory(
        &mut self,
        subject: Subject,
        path: Path<'_>,
        context: Context,
    ) -> Result<Context> {
        self.ready()?;
        let windows = matches!(path, Path::Windows(_));
        let index = self.resolve(subject, path, context)?;
        let node = self.state.nodes[index];
        if node.kind != 2 {
            return Err(Error::InvalidParameter);
        }
        node.acl.check(subject, Access::TRAVERSE)?;
        let mut ancestor = node.id;
        loop {
            if ancestor == 3 || ancestor == 10 {
                break;
            }
            if ancestor == 1 {
                return Err(Error::Unsupported);
            }
            ancestor = self.state.nodes[self.index(ancestor)?].parent;
        }
        Ok(Context {
            cwd: node.id,
            drive_cwd: if windows { node.id } else { context.drive_cwd },
            mount: self.state.mount,
        })
    }
    fn check_context(&self, context: Context) -> Result<()> {
        if context.mount != 0 && context.mount != self.state.mount {
            Err(Error::InvalidHandle)
        } else {
            Ok(())
        }
    }
    pub fn mkdir(
        &mut self,
        subject: Subject,
        path: Path<'_>,
        context: Context,
        acl: Option<Acl>,
    ) -> Result<u64> {
        self.ready()?;
        let (parent, name, _) = self.parent(subject, path, context)?;
        let p = self.state.nodes[parent];
        p.acl.check(subject, Access::CREATE)?;
        if self.child(p.id, name, true).is_ok() {
            return Err(Error::AlreadyExists);
        }
        let slot = self.free_slot()?;
        let id = self.state.next_object;
        let next = id.checked_add(1).ok_or(Error::NoSpace)?;
        let acl = acl.unwrap_or_else(|| p.acl.inherited(subject.default_owner()));
        if !acl.valid() || !subject.system() && !subject.owns(acl.owner) {
            return Err(Error::AccessDenied);
        }
        self.mutation()?;
        self.state.next_object = next;
        self.state.nodes[slot] = Node {
            id,
            parent: p.id,
            kind: 2,
            name,
            acl,
            version: 1,
            times: FileTimes::at(self.clock),
            ..Node::EMPTY
        };
        self.touch_directory(p.id);
        Ok(id)
    }
    pub fn open(
        &mut self,
        subject: Subject,
        path: Path<'_>,
        context: Context,
        options: OpenOptions,
    ) -> Result<(FileHandle, bool)> {
        self.ready()?;
        if !options.access.valid() || options.share.0 & !7 != 0 {
            return Err(Error::InvalidParameter);
        }
        let handle_slot = self
            .state
            .opens
            .iter()
            .position(|o| o.token == 0)
            .ok_or(Error::NoSpace)?;
        let token = self.state.next_handle;
        let next_token = token.checked_add(1).ok_or(Error::NoSpace)?;
        let (parent, name, fold) = self.parent(subject, path, context)?;
        let p = self.state.nodes[parent];
        let found = self.child(p.id, name, fold).ok();
        let truncate = matches!(
            options.disposition,
            Disposition::CreateAlways | Disposition::TruncateExisting
        );
        if truncate && !options.access.contains(Access::WRITE) {
            return Err(Error::AccessDenied);
        }
        let (index, created) = if let Some(index) = found {
            let node = self.state.nodes[index];
            if node.pending {
                return Err(Error::DeletePending);
            }
            if node.kind != 1 {
                return Err(Error::InvalidParameter);
            }
            if matches!(options.disposition, Disposition::CreateNew) {
                return Err(Error::AlreadyExists);
            }
            node.acl.check(subject, options.access)?;
            self.sharing(node.id, options.access, options.share)?;
            (index, false)
        } else {
            if matches!(
                options.disposition,
                Disposition::OpenExisting | Disposition::TruncateExisting
            ) {
                return Err(Error::NotFound);
            }
            if self.child(p.id, name, true).is_ok() {
                return Err(Error::AlreadyExists);
            }
            p.acl.check(subject, Access::CREATE)?;
            let acl = p.acl.inherited(subject.default_owner());
            acl.check(subject, options.access)?;
            let index = self.free_slot()?;
            let id = self.state.next_object;
            let next = id.checked_add(1).ok_or(Error::NoSpace)?;
            self.mutation()?;
            let mut disk_name = [0; 19];
            let result = self.fs.create(backing_name(id, &mut disk_name));
            let backing = self.io(result)?;
            self.state.next_object = next;
            self.state.nodes[index] = Node {
                id,
                parent: p.id,
                kind: 1,
                name,
                backing,
                acl,
                version: 1,
                times: FileTimes::at(self.clock),
                ..Node::EMPTY
            };
            self.touch_directory(p.id);
            (index, true)
        };
        if truncate && !created {
            let version = self.state.nodes[index]
                .version
                .checked_add(1)
                .ok_or(Error::NoSpace)?;
            self.mutation()?;
            let result = self.fs.truncate(self.state.nodes[index].backing, 0);
            self.io(result)?;
            self.state.nodes[index].version = version;
            self.touch_content(index);
        }
        self.state.next_handle = next_token;
        self.state.opens[handle_slot] = Open {
            token,
            node: self.state.nodes[index].id,
            access: options.access,
            share: options.share,
        };
        Ok((
            FileHandle {
                mount: self.state.mount,
                token,
            },
            created,
        ))
    }
    fn sharing(&self, node: u64, access: Access, share: Access) -> Result<()> {
        for open in &self.state.opens {
            if open.token != 0
                && open.node == node
                && ((access.0 & 7) & !open.share.0 != 0 || (open.access.0 & 7) & !share.0 != 0)
            {
                return Err(Error::SharingViolation);
            }
        }
        Ok(())
    }
    fn handle(&mut self, handle: &FileHandle, access: Access) -> Result<(usize, usize)> {
        self.ready()?;
        if handle.mount != self.state.mount {
            return Err(Error::InvalidHandle);
        }
        let slot = self
            .state
            .opens
            .iter()
            .position(|o| o.token == handle.token && handle.token != 0)
            .ok_or(Error::InvalidHandle)?;
        let open = self.state.opens[slot];
        if !open.access.contains(access) {
            return Err(Error::AccessDenied);
        }
        Ok((slot, self.index(open.node)?))
    }
    pub fn info(&mut self, handle: &FileHandle) -> Result<FileInfo> {
        let (_, i) = self.handle(handle, Access(0))?;
        self.node_info(i)
    }
    /// Identity hints confer no authority. Existing handles retain their captured
    /// grants; a current write grant and unchanged object/version are required.
    /// This validates only: candidate bytes remain outside the volume until commit.
    pub fn check_replace(
        &mut self,
        handle: &FileHandle,
        object: u64,
        version: u64,
        bytes: usize,
    ) -> Result<FileInfo> {
        let (_, i) = self.handle(handle, Access::WRITE)?;
        if object == 0 || version == 0 || bytes > super::REPLACE_BYTES {
            return Err(Error::InvalidParameter);
        }
        let node = self.state.nodes[i];
        if node.id != object || node.version != version {
            return Err(Error::Changed);
        }
        if node.pending {
            return Err(Error::DeletePending);
        }
        if node.kind != 1 {
            return Err(Error::InvalidParameter);
        }
        node.version.checked_add(1).ok_or(Error::NoSpace)?;
        self.state.revision.checked_add(1).ok_or(Error::NoSpace)?;
        let info = self.node_info(i)?;
        if info.bytes > super::REPLACE_BYTES as u64 {
            return Err(Error::InvalidParameter);
        }
        Ok(info)
    }
    fn node_info(&mut self, i: usize) -> Result<FileInfo> {
        let n = self.state.nodes[i];
        let bytes = if n.kind == 2 {
            0
        } else {
            let result = self.fs.metadata(n.backing);
            self.io(result)?.bytes
        };
        Ok(FileInfo {
            object: n.id,
            backing: n.backing,
            bytes,
            directory: n.kind == 2,
            delete_pending: n.pending,
            attributes: n.attributes,
            version: n.version,
            times: n.times,
        })
    }
    pub fn metadata(
        &mut self,
        subject: Subject,
        path: Path<'_>,
        context: Context,
    ) -> Result<FileInfo> {
        self.ready()?;
        let i = self.resolve(subject, path, context)?;
        let required = if self.state.nodes[i].kind == 2 {
            Access::READ.union(Access::TRAVERSE)
        } else {
            Access::READ
        };
        self.state.nodes[i].acl.check(subject, required)?;
        self.node_info(i)
    }
    /// Object-ID cursor: deletions never repeat entries; creations after the
    /// cursor may appear, and moved entries reflect their current parent.
    pub fn list(
        &mut self,
        subject: Subject,
        path: Path<'_>,
        context: Context,
        after: u64,
    ) -> Result<Option<DirectoryEntry>> {
        self.ready()?;
        let i = self.resolve(subject, path, context)?;
        let directory = self.state.nodes[i];
        if directory.kind != 2 {
            return Err(Error::InvalidParameter);
        }
        directory
            .acl
            .check(subject, Access::READ.union(Access::TRAVERSE))?;
        let mut cursor = after;
        let next = loop {
            match self.next_child(directory.id, cursor)? {
                Some(i) if self.state.nodes[i].pending => cursor = self.state.nodes[i].id,
                found => break found,
            }
        };
        match next {
            Some(i) => Ok(Some(DirectoryEntry {
                name: self.state.nodes[i].name,
                info: self.node_info(i)?,
            })),
            None => Ok(None),
        }
    }
    pub fn truncate(&mut self, handle: &FileHandle, bytes: u64) -> Result<()> {
        let (_, i) = self.handle(handle, Access::WRITE)?;
        let version = self.state.nodes[i]
            .version
            .checked_add(1)
            .ok_or(Error::NoSpace)?;
        self.mutation()?;
        let result = self.fs.truncate(self.state.nodes[i].backing, bytes);
        self.io(result)?;
        self.state.nodes[i].version = version;
        self.touch_content(i);
        Ok(())
    }
    pub fn remove_path(
        &mut self,
        subject: Subject,
        path: Path<'_>,
        context: Context,
    ) -> Result<()> {
        self.ready()?;
        let i = self.resolve(subject, path, context)?;
        let n = self.state.nodes[i];
        if n.id <= 16 {
            return Err(Error::AccessDenied);
        }
        n.acl.check(subject, Access::DELETE)?;
        if self.has_children(n.id)? {
            return Err(Error::InvalidParameter);
        }
        self.can_delete(i)?;
        self.remove_index(i)
    }
    /// Delete one observed binding. Object IDs and versions are comparison hints;
    /// current ancestry and DELETE authority are checked on every call.
    pub fn check_remove_child(
        &mut self,
        subject: Subject,
        parent: u64,
        object: u64,
        observed_version: u64,
    ) -> Result<()> {
        self.removable_child(subject, parent, object, observed_version)
            .map(|_| ())
    }
    fn removable_child(
        &mut self,
        subject: Subject,
        parent: u64,
        object: u64,
        observed_version: u64,
    ) -> Result<usize> {
        self.ready()?;
        if parent == 0 || object == 0 || observed_version == 0 {
            return Err(Error::InvalidParameter);
        }
        self.current_parent(subject, parent)?;
        let i = self.index(object)?;
        let n = self.state.nodes[i];
        if n.id <= 16 {
            return Err(Error::AccessDenied);
        }
        if n.pending {
            return Err(Error::DeletePending);
        }
        if n.parent != parent {
            return Err(Error::Changed);
        }
        n.acl.check(subject, Access::DELETE)?;
        self.can_delete(i)?;
        if n.version != observed_version {
            return Err(Error::Changed);
        }
        // Pending children still own a binding/backing lifetime. Never orphan
        // one merely because the ordinary directory listing hides it.
        if self.has_children(n.id)? {
            return Err(Error::InvalidParameter);
        }
        self.state.revision.checked_add(1).ok_or(Error::NoSpace)?;
        Ok(i)
    }
    /// Staged until the serialized owner commits. SHARE_DELETE opens retain
    /// their backing stream until final close; mount also reclaims pending nodes.
    pub fn remove_child_checked(
        &mut self,
        subject: Subject,
        parent: u64,
        object: u64,
        observed_version: u64,
    ) -> Result<()> {
        let i = self.removable_child(subject, parent, object, observed_version)?;
        self.remove_index(i)
    }
    fn remove_index(&mut self, i: usize) -> Result<()> {
        let n = self.state.nodes[i];
        self.mutation()?;
        self.state.nodes[i].pending = true;
        self.touch_directory(n.parent);
        if !self
            .state
            .opens
            .iter()
            .any(|o| o.token != 0 && o.node == n.id)
        {
            self.reclaim_mutated(i)?;
        }
        Ok(())
    }
    /// Rename one observed child without treating object IDs as authority. This
    /// remains staged until the serialized owner successfully commits it.
    pub fn rename_child_checked(
        &mut self,
        subject: Subject,
        parent: u64,
        object: u64,
        observed_version: u64,
        units: &[u16],
    ) -> Result<()> {
        self.ready()?;
        if parent == 0 || object == 0 || observed_version == 0 {
            return Err(Error::InvalidParameter);
        }
        let name = Name::new(units)?;
        if matches!(name.units(), [46] | [46, 46]) {
            return Err(Error::InvalidName);
        }
        self.storable(&name)?;
        let parent_index = self.current_parent(subject, parent)?;
        let i = self.index(object)?;
        let n = self.state.nodes[i];
        if n.id <= 16 {
            return Err(Error::AccessDenied);
        }
        if n.pending {
            return Err(Error::DeletePending);
        }
        if n.parent != parent {
            return Err(Error::Changed);
        }
        n.acl.check(subject, Access::DELETE)?;
        self.state.nodes[parent_index]
            .acl
            .check(subject, Access::CREATE)?;
        self.can_delete(i)?;
        if n.version != observed_version {
            return Err(Error::Changed);
        }
        if self
            .child(parent, name, true)
            .is_ok_and(|target| target != i)
        {
            return Err(Error::AlreadyExists);
        }
        if n.name == name {
            return Ok(());
        }
        let version = n.version.checked_add(1).ok_or(Error::NoSpace)?;
        self.mutation()?;
        self.state.nodes[i].name = name;
        self.state.nodes[i].version = version;
        // NTFS: a rename keeps the item's own times; its directory changed.
        self.touch_directory(parent);
        Ok(())
    }
    /// Authenticate current ancestry even after a parent has moved since listing.
    fn current_parent(&mut self, subject: Subject, parent: u64) -> Result<usize> {
        let parent_index = self.index(parent)?;
        let mut ancestor = parent;
        for _ in 0..self.state.capacity {
            let node = self.state.nodes[self.index(ancestor).map_err(|_| Error::Corrupt)?];
            if node.pending {
                return Err(Error::DeletePending);
            }
            if node.kind != 2 {
                return Err(Error::InvalidParameter);
            }
            node.acl.check(subject, Access::TRAVERSE)?;
            if node.id == 1 {
                return Ok(parent_index);
            }
            ancestor = node.parent;
        }
        Err(Error::Corrupt)
    }
    pub fn rename_path(
        &mut self,
        subject: Subject,
        old: Path<'_>,
        new: Path<'_>,
        context: Context,
    ) -> Result<()> {
        self.ready()?;
        let i = self.resolve(subject, old, context)?;
        let n = self.state.nodes[i];
        if n.id <= 16 {
            return Err(Error::AccessDenied);
        }
        n.acl.check(subject, Access::DELETE)?;
        self.can_delete(i)?;
        let (parent, name, fold) = self.parent(subject, new, context)?;
        let p = self.state.nodes[parent];
        p.acl.check(subject, Access::CREATE)?;
        let mut ancestor = p.id;
        loop {
            if ancestor == n.id {
                return Err(Error::InvalidParameter);
            }
            if ancestor == 1 {
                break;
            }
            ancestor = self.state.nodes[self.index(ancestor)?].parent;
        }
        if let Ok(target) = self.child(p.id, name, fold)
            && target != i
        {
            return Err(Error::AlreadyExists);
        }
        if self.child(p.id, name, true).is_ok_and(|t| t != i) {
            return Err(Error::AlreadyExists);
        }
        if n.parent == p.id && n.name == name {
            return Ok(());
        }
        let version = n.version.checked_add(1).ok_or(Error::NoSpace)?;
        self.mutation()?;
        self.state.nodes[i].parent = p.id;
        self.state.nodes[i].name = name;
        self.state.nodes[i].version = version;
        self.touch_directory(n.parent);
        self.touch_directory(p.id);
        Ok(())
    }
    pub fn read(&mut self, handle: &FileHandle, offset: u64, out: &mut [u8]) -> Result<usize> {
        let (_, i) = self.handle(handle, Access::READ)?;
        let result = self.fs.read(self.state.nodes[i].backing, offset, out);
        self.io(result)
    }
    pub fn write(&mut self, handle: &FileHandle, offset: u64, input: &[u8]) -> Result<usize> {
        let (_, i) = self.handle(handle, Access::WRITE)?;
        if input.is_empty() {
            return Ok(0);
        }
        let version = self.state.nodes[i]
            .version
            .checked_add(1)
            .ok_or(Error::NoSpace)?;
        self.mutation()?;
        let result = self.fs.write(self.state.nodes[i].backing, offset, input);
        let n = self.io(result)?;
        self.state.nodes[i].version = version;
        self.touch_content(i);
        Ok(n)
    }
    pub fn set_acl(&mut self, subject: Subject, handle: &FileHandle, acl: Acl) -> Result<()> {
        let (_, i) = self.handle(handle, Access::CHANGE_ACL)?;
        if !acl.valid() || !subject.system() && acl.owner != self.state.nodes[i].acl.owner {
            return Err(Error::AccessDenied);
        }
        if self.state.nodes[i].acl == acl {
            return Ok(());
        }
        let version = self.state.nodes[i]
            .version
            .checked_add(1)
            .ok_or(Error::NoSpace)?;
        self.mutation()?;
        self.state.nodes[i].acl = acl;
        self.state.nodes[i].version = version;
        Ok(())
    }
    /// A file's or folder's ACL and whether it is a folder, as Windows'
    /// GetNamedSecurityInfo reads it: its owner (READ_CONTROL is implicit)
    /// or a subject the ACL grants read.
    pub fn security(
        &mut self,
        subject: Subject,
        path: Path<'_>,
        context: Context,
    ) -> Result<(Acl, bool)> {
        self.ready()?;
        let i = self.resolve(subject, path, context)?;
        self.node_security(subject, i)
    }
    /// `security` of an open file (GetSecurityInfo of a handle).
    pub fn security_of(&mut self, subject: Subject, handle: &FileHandle) -> Result<(Acl, bool)> {
        let (_, i) = self.handle(handle, Access(0))?;
        self.node_security(subject, i)
    }
    fn node_security(&self, subject: Subject, i: usize) -> Result<(Acl, bool)> {
        let node = &self.state.nodes[i];
        if !subject.owns(node.acl.owner) && !subject.system() {
            node.acl.check(subject, Access::READ)?;
        }
        Ok((node.acl, node.kind == 2))
    }
    /// SetNamedSecurityInfo: the owner (WRITE_DAC is implicit) or a subject
    /// the ACL grants CHANGE_ACL may replace it; the owner may become only
    /// the subject itself (taking ownership), and a label may not rise above
    /// the subject's integrity. The system may do anything.
    pub fn set_security(
        &mut self,
        subject: Subject,
        path: Path<'_>,
        context: Context,
        acl: Acl,
    ) -> Result<()> {
        self.ready()?;
        let i = self.resolve(subject, path, context)?;
        self.node_set_security(subject, i, acl)
    }
    /// `set_security` of an open file (SetSecurityInfo of a handle).
    pub fn set_security_of(
        &mut self,
        subject: Subject,
        handle: &FileHandle,
        acl: Acl,
    ) -> Result<()> {
        let (_, i) = self.handle(handle, Access(0))?;
        self.node_set_security(subject, i, acl)
    }
    fn node_set_security(&mut self, subject: Subject, i: usize, acl: Acl) -> Result<()> {
        if !acl.valid() {
            return Err(Error::InvalidParameter);
        }
        let current = self.state.nodes[i].acl;
        if !subject.system() {
            if !subject.owns(current.owner) {
                current.check(subject, Access::CHANGE_ACL)?;
            }
            if acl.owner != current.owner && !subject.owns(acl.owner)
                || acl.integrity > subject.integrity()
            {
                return Err(Error::AccessDenied);
            }
        }
        if current == acl {
            return Ok(());
        }
        let version = self.state.nodes[i]
            .version
            .checked_add(1)
            .ok_or(Error::NoSpace)?;
        self.mutation()?;
        self.state.nodes[i].acl = acl;
        self.state.nodes[i].version = version;
        Ok(())
    }
    pub fn acl(&mut self, handle: &FileHandle) -> Result<Acl> {
        let (_, i) = self.handle(handle, Access::READ)?;
        Ok(self.state.nodes[i].acl)
    }
    pub fn set_attributes(&mut self, handle: &FileHandle, attributes: u32) -> Result<()> {
        let (_, i) = self.handle(handle, Access::WRITE)?;
        if attributes & !0x22 != 0 {
            return Err(Error::Unsupported);
        }
        if self.state.nodes[i].attributes == attributes {
            return Ok(());
        }
        let version = self.state.nodes[i]
            .version
            .checked_add(1)
            .ok_or(Error::NoSpace)?;
        self.mutation()?;
        self.state.nodes[i].attributes = attributes;
        self.state.nodes[i].version = version;
        Ok(())
    }
    /// Attributes of a file or folder by path (folders cannot be opened as
    /// handles). Requires the current WRITE right on the object.
    pub fn set_attributes_path(
        &mut self,
        subject: Subject,
        path: Path<'_>,
        context: Context,
        attributes: u32,
    ) -> Result<()> {
        self.ready()?;
        let i = self.resolve(subject, path, context)?;
        if self.state.nodes[i].id <= 16 {
            return Err(Error::AccessDenied);
        }
        self.state.nodes[i].acl.check(subject, Access::WRITE)?;
        if attributes & !0x22 != 0 {
            return Err(Error::Unsupported);
        }
        if self.state.nodes[i].attributes == attributes {
            return Ok(());
        }
        let version = self.state.nodes[i]
            .version
            .checked_add(1)
            .ok_or(Error::NoSpace)?;
        self.mutation()?;
        self.state.nodes[i].attributes = attributes;
        self.state.nodes[i].version = version;
        let parent = self.state.nodes[i].parent;
        self.note(Change::Folder(parent));
        Ok(())
    }
    fn can_delete(&self, index: usize) -> Result<()> {
        let id = self.state.nodes[index].id;
        for o in &self.state.opens {
            if o.token != 0 && o.node == id && !o.share.contains(Access::DELETE) {
                return Err(Error::SharingViolation);
            }
        }
        Ok(())
    }
    fn reclaim(&mut self, index: usize) -> Result<()> {
        let node = self.state.nodes[index];
        if node.kind == 1 {
            let mut n = [0; 19];
            let result = self.fs.unlink(backing_name(node.id, &mut n), node.backing);
            self.io(result)?;
        }
        self.state.nodes[index] = Node::EMPTY;
        self.dirty = true;
        Ok(())
    }
    /// A removal/last close already changed namespace or open-instance state.
    /// No rejected reclaim may remain eligible for a later unrelated commit.
    fn reclaim_mutated(&mut self, index: usize) -> Result<()> {
        let result = self.reclaim(index);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    pub fn delete(&mut self, handle: &FileHandle) -> Result<()> {
        let (_, i) = self.handle(handle, Access::DELETE)?;
        self.can_delete(i)?;
        self.mutation()?;
        self.state.nodes[i].pending = true;
        self.touch_directory(self.state.nodes[i].parent);
        Ok(())
    }
    pub fn rename(
        &mut self,
        subject: Subject,
        handle: &FileHandle,
        new_path: Path<'_>,
        context: Context,
        replace: bool,
    ) -> Result<()> {
        let (_, index) = self.handle(handle, Access::DELETE)?;
        if self.state.nodes[index].pending {
            return Err(Error::DeletePending);
        }
        self.can_delete(index)?;
        let (parent, name, fold) = self.parent(subject, new_path, context)?;
        let p = self.state.nodes[parent];
        p.acl.check(subject, Access::CREATE)?;
        let target = self.child(p.id, name, fold).ok();
        if target == Some(index) {
            if self.state.nodes[index].name == name {
                return Ok(());
            }
            let version = self.state.nodes[index]
                .version
                .checked_add(1)
                .ok_or(Error::NoSpace)?;
            self.mutation()?;
            self.state.nodes[index].name = name;
            self.state.nodes[index].version = version;
            self.touch_directory(p.id);
            return Ok(());
        }
        if let Some(t) = target {
            let n = self.state.nodes[t];
            if !replace {
                return Err(Error::AlreadyExists);
            }
            if n.pending {
                return Err(Error::DeletePending);
            }
            if n.kind != 1 {
                return Err(Error::InvalidParameter);
            }
            n.acl.check(subject, Access::DELETE)?;
            self.can_delete(t)?;
        }
        if target.is_none()
            && self
                .child(p.id, name, true)
                .is_ok_and(|other| other != index)
        {
            return Err(Error::AlreadyExists);
        }
        let version = self.state.nodes[index]
            .version
            .checked_add(1)
            .ok_or(Error::NoSpace)?;
        self.mutation()?;
        if let Some(t) = target {
            let id = self.state.nodes[t].id;
            self.state.nodes[t].pending = true;
            self.state.nodes[t].parent = 0;
            if !self
                .state
                .opens
                .iter()
                .any(|o| o.token != 0 && o.node == id)
            {
                self.reclaim(t)?;
            }
        }
        let old_parent = self.state.nodes[index].parent;
        self.state.nodes[index].parent = p.id;
        self.state.nodes[index].name = name;
        self.state.nodes[index].version = version;
        self.touch_directory(old_parent);
        self.touch_directory(p.id);
        Ok(())
    }
    pub fn close(&mut self, handle: FileHandle) -> Result<()> {
        let (slot, index) = self.handle(&handle, Access(0))?;
        let node = self.state.nodes[index];
        self.state.opens[slot] = Open::EMPTY;
        if node.pending
            && !self
                .state
                .opens
                .iter()
                .any(|o| o.token != 0 && o.node == node.id)
        {
            if let Err(error) = self.mutation() {
                // The consumed open instance cannot be restored to its caller.
                // Retire even on counter exhaustion before reclaim starts.
                self.poisoned = true;
                return Err(error);
            }
            self.reclaim_mutated(index)?;
        }
        Ok(())
    }
    /// Stages coherent filesystem, catalog, registry and settings state. This is
    /// not a device-durability acknowledgement; use commit on the journal owner.
    pub fn flush(&mut self) -> Result<()> {
        self.ready()?;
        self.flush_nodes()?;
        if self.dirty {
            let length = codec::catalog_bytes(self.state.capacity)?;
            codec::encode(self.state, &mut self.buffer[..length])?;
            let result = self.fs.write(self.catalog, 0, &self.buffer[..length]);
            let count = self.io(result)?;
            if count != length {
                self.poisoned = true;
                return Err(Error::Corrupt);
            }
            self.flush_hive()?;
        }
        let result = self.fs.flush();
        self.io(result)?;
        self.dirty = false;
        Ok(())
    }
    /// Write the registry hive in the catalog's transaction when the records
    /// past the catalog's slots changed. The hive is created by the first
    /// such record and is never removed (an empty hive states no records).
    fn flush_hive(&mut self) -> Result<()> {
        let identity = codec::hive::identity(self.state, self.buffer)?;
        if identity == self.hive_written {
            return Ok(());
        }
        let hive = match self.hive {
            Some(hive) => hive,
            None if codec::hive::extent(self.state) == (0, 0) => return Ok(()),
            None => {
                let result = self.fs.create(HIVE_NAME);
                let hive = self.io(result)?;
                self.hive = Some(hive);
                hive
            }
        };
        let length = codec::hive::bytes(codec::hive::extent(self.state));
        let (fs, state, buffer) = (&mut self.fs, &*self.state, &mut *self.buffer);
        let mut failure = None;
        let written = codec::hive::encode(state, buffer, |offset, bytes| {
            match fs.write(hive, offset as u64, bytes) {
                Ok(n) if n == bytes.len() => Ok(()),
                Ok(_) => Err(Error::Corrupt),
                Err(e) => {
                    failure = Some(e);
                    Err(e.into())
                }
            }
        });
        if let Some(e) = failure {
            return self.io(Err(e));
        }
        let header = match written {
            Ok(header) => header,
            Err(e) => {
                self.poisoned = true;
                return Err(e);
            }
        };
        let result = self.fs.truncate(hive, length as u64);
        self.io(result)?;
        self.hive_written = header;
        Ok(())
    }
    pub fn into_inner(mut self) -> Result<F> {
        self.flush()?;
        Ok(self.fs)
    }
    pub fn revision(&self) -> Result<u64> {
        self.ready()?;
        Ok(self.state.revision)
    }
}
impl<D: crate::storage::BlockDevice>
    PersistentVolume<'_, crate::storage::Ext2<crate::storage::Journal<'_, D>>>
{
    /// Replace one bounded stream and acknowledge the complete durable payload,
    /// length and catalog version. No other request can checkpoint midway.
    pub fn replace_checked(
        &mut self,
        handle: &FileHandle,
        object: u64,
        version: u64,
        input: &[u8],
    ) -> Result<FileInfo> {
        self.replace_checked_with(handle, object, version, input, |_| {})
    }
    /// The callback observes only the replacement's final commit, for the same
    /// explicit crash-injection boundary as commit_with (not earlier staging).
    pub fn replace_checked_with(
        &mut self,
        handle: &FileHandle,
        object: u64,
        version: u64,
        input: &[u8],
        checkpoint: impl FnMut(crate::storage::Checkpoint),
    ) -> Result<FileInfo> {
        let mut info = self.check_replace(handle, object, version, input.len())?;
        // Isolate the candidate from all legacy staging and reserve the entire
        // journal working set. This can retire a final-generation owner.
        self.commit()?;
        let (_, i) = self.handle(handle, Access::WRITE)?;
        self.mutation()?;
        let result = (|| {
            if !input.is_empty() {
                let count = self.fs.write(info.backing, 0, input)?;
                if count != input.len() {
                    return Err(crate::storage::Error::Corrupt);
                }
            }
            self.fs.truncate(info.backing, input.len() as u64)
        })();
        if let Err(error) = result {
            // Even a capacity error must not expose partially staged replacement
            // state through later requests or an unrelated checkpoint.
            self.poisoned = true;
            return Err(error.into());
        }
        info.bytes = input.len() as u64;
        info.version += 1; // check_replace preflighted both monotonic counters.
        self.state.nodes[i].version = info.version;
        self.touch_content(i);
        info.times = self.state.nodes[i].times;
        self.commit_with(checkpoint)?;
        // Do not issue a fallible read after acknowledging the durable commit.
        Ok(info)
    }
    /// Begin a whole-file replacement of up to `LARGE_REPLACE_BYTES`. Checks
    /// the WRITE grant, exact object/version and a free node slot before any
    /// mutation (refusals leave the owner usable). The candidate's blocks are
    /// reserved against every other live staged replacement of this mount
    /// (released as they are written, or on commit/abort), so concurrent
    /// staged saves never overcommit free space. Ordinary writes do not
    /// consult the reservation: if they consume the space first, a later
    /// `stage_write` fails with NoSpace and only the candidate is lost. The
    /// new parentless pending node and its empty backing are staged in the
    /// current transaction; nothing reaches the namespace.
    pub fn stage_replace(
        &mut self,
        handle: &FileHandle,
        object: u64,
        version: u64,
        bytes: u64,
    ) -> Result<Staged> {
        let (_, i) = self.handle(handle, Access::WRITE)?;
        if object == 0 || version == 0 || bytes > super::LARGE_REPLACE_BYTES as u64 {
            return Err(Error::InvalidParameter);
        }
        let node = self.state.nodes[i];
        if node.id != object || node.version != version {
            return Err(Error::Changed);
        }
        if node.pending {
            return Err(Error::DeletePending);
        }
        if node.kind != 1 {
            return Err(Error::InvalidParameter);
        }
        node.version.checked_add(1).ok_or(Error::NoSpace)?;
        let slot = self.free_slot()?;
        let id = self.state.next_object;
        let next = id.checked_add(1).ok_or(Error::NoSpace)?;
        let blocks = stage_blocks(bytes);
        let promised = self.staged_reserved.saturating_add(blocks);
        if self
            .fs
            .usage()
            .is_some_and(|(_, free)| free / 4096 < promised)
        {
            return Err(Error::NoSpace);
        }
        let name = Name::ascii("staged")?;
        self.mutation()?;
        let mut disk_name = [0; 19];
        let result = self.fs.create(backing_name(id, &mut disk_name));
        let backing = self.io(result)?;
        self.state.next_object = next;
        self.state.nodes[slot] = Node {
            id,
            parent: 0,
            kind: 1,
            pending: true,
            name,
            backing,
            acl: node.acl,
            version: 1,
            ..Node::EMPTY
        };
        self.staged_reserved = promised;
        Ok(Staged {
            mount: self.state.mount,
            node: id,
            target: object,
            version,
            bytes,
            written: 0,
            reserved: blocks,
        })
    }
    fn staged_index(&mut self, staged: &Staged) -> Result<usize> {
        if staged.mount != self.state.mount {
            return Err(Error::InvalidHandle);
        }
        let i = self.index(staged.node).map_err(|_| Error::InvalidHandle)?;
        let n = self.state.nodes[i];
        if !n.pending || n.parent != 0 || n.kind != 1 {
            return Err(Error::InvalidHandle);
        }
        Ok(i)
    }
    /// The staged node still exists (no commit attempt or abort consumed it).
    pub fn staged_live(&mut self, staged: &Staged) -> bool {
        self.staged_index(staged).is_ok()
    }
    /// Append 1..=4096 candidate bytes at exactly `offset == written()`.
    /// Earlier staged pages are committed first (the candidate stays
    /// unreachable), so any length up to the limit fits the journal. A
    /// backend failure poisons the owner, as for any write.
    pub fn stage_write(&mut self, staged: &mut Staged, offset: u64, input: &[u8]) -> Result<()> {
        self.ready()?;
        let i = self.staged_index(staged)?;
        let end = staged
            .written
            .checked_add(input.len() as u64)
            .ok_or(Error::InvalidParameter)?;
        if input.is_empty() || input.len() > 4096 || offset != staged.written || end > staged.bytes
        {
            return Err(Error::InvalidParameter);
        }
        self.checkpoint_budget(STAGE_BUDGET)?;
        let result = self.fs.write(self.state.nodes[i].backing, offset, input);
        if self.io(result)? != input.len() {
            self.poisoned = true;
            return Err(Error::Corrupt);
        }
        // The blocks this write allocated leave the reservation.
        let used = (stage_blocks(end) - stage_blocks(staged.written)).min(staged.reserved);
        staged.reserved -= used;
        self.staged_reserved -= used;
        staged.written = end;
        Ok(())
    }
    /// Blocks reserved for live staged candidates and not yet written.
    pub fn staged_reserved_blocks(&self) -> u64 {
        self.staged_reserved
    }
    pub fn commit_staged(&mut self, handle: &FileHandle, staged: &Staged) -> Result<FileInfo> {
        self.commit_staged_with(handle, staged, |_| {})
    }
    /// Publish a complete candidate: one journal transaction unlinks the old
    /// payload, renames the candidate backing to the target's backing name,
    /// rebinds the node (same object ID, version + 1, modified/accessed
    /// times) and drops the staging node. All candidate pages and earlier
    /// staging are committed first, so recovery yields the complete old or
    /// complete new file. Preflight refusals leave the staging node (abort
    /// it); once the swap starts any failure poisons the owner. The callback
    /// observes only the swap's commit (crash-injection boundaries).
    pub fn commit_staged_with(
        &mut self,
        handle: &FileHandle,
        staged: &Staged,
        checkpoint: impl FnMut(crate::storage::Checkpoint),
    ) -> Result<FileInfo> {
        let (_, t) = self.handle(handle, Access::WRITE)?;
        let s = self.staged_index(staged)?;
        if staged.written != staged.bytes {
            return Err(Error::InvalidParameter);
        }
        let target = self.state.nodes[t];
        if target.id != staged.target || target.version != staged.version {
            return Err(Error::Changed);
        }
        if target.pending {
            return Err(Error::DeletePending);
        }
        let version = target.version.checked_add(1).ok_or(Error::NoSpace)?;
        self.state.revision.checked_add(2).ok_or(Error::NoSpace)?;
        let result = self.fs.metadata(self.state.nodes[s].backing);
        if self.io(result)?.bytes != staged.bytes {
            return Err(Error::InvalidParameter);
        }
        // Isolate the swap: candidate payload and unrelated staging first.
        self.commit()?;
        let stage = self.state.nodes[s];
        self.mutation()?;
        let result = (|| {
            let (mut a, mut b) = ([0; 19], [0; 19]);
            let target_name = backing_name(target.id, &mut a);
            let stage_name = backing_name(stage.id, &mut b);
            self.fs.unlink(target_name, target.backing)?;
            self.fs.rename(stage_name, target_name)
        })();
        if let Err(error) = result {
            self.poisoned = true;
            return Err(error.into());
        }
        self.state.nodes[t].backing = stage.backing;
        self.state.nodes[t].version = version;
        self.touch_content(t);
        self.state.nodes[s] = Node::EMPTY;
        self.staged_reserved -= staged.reserved.min(self.staged_reserved);
        let times = self.state.nodes[t].times;
        self.commit_with(checkpoint)?;
        Ok(FileInfo {
            object: target.id,
            backing: stage.backing,
            bytes: staged.bytes,
            directory: false,
            delete_pending: false,
            attributes: target.attributes,
            version,
            times,
        })
    }
    /// Discard a candidate: reclaim the staging node and its backing in the
    /// current transaction (a failure poisons the owner).
    pub fn abort_staged(&mut self, staged: Staged) -> Result<()> {
        self.ready()?;
        let i = self.staged_index(&staged)?;
        self.mutation()?;
        self.reclaim_mutated(i)?;
        self.staged_reserved -= staged.reserved.min(self.staged_reserved);
        Ok(())
    }
    pub fn commit(&mut self) -> Result<()> {
        self.commit_with(|_| {})
    }
    /// Reserve room for the catalog and one bounded service mutation before it
    /// can exhaust journal staging. Does not acknowledge the next operation.
    pub fn checkpoint_budget(&mut self, limit: usize) -> Result<()> {
        self.ready()?;
        if limit == 0 || limit > 24 {
            return Err(Error::InvalidParameter);
        }
        let result = self
            .fs
            .with_flushed_device(|journal| Ok(journal.staged_pages()));
        if self.io(result)? >= limit {
            self.commit()?;
        }
        Ok(())
    }
    pub fn commit_with(
        &mut self,
        checkpoint: impl FnMut(crate::storage::Checkpoint),
    ) -> Result<()> {
        self.flush()?;
        let result = self.fs.with_flushed_device(|journal| {
            journal.commit_with(checkpoint)?;
            match journal.begin() {
                Ok(()) => Ok(true),
                Err(crate::storage::Error::NoSpace) => Ok(false),
                Err(e) => Err(e),
            }
        });
        let reusable = self.io(result)?;
        // The final representable generation is durably committed. Report that
        // success, then retire this instance rather than aliasing a generation
        // or misleading the caller with a failed-commit status.
        if !reusable {
            self.poisoned = true;
        }
        Ok(())
    }
}

/// A short native path built without allocation (layout folders).
struct PathText {
    bytes: [u8; 128],
    len: usize,
}
impl PathText {
    fn new() -> Self {
        Self {
            bytes: [0; 128],
            len: 0,
        }
    }
    fn text(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}
impl core::fmt::Write for PathText {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let end = self.len + s.len();
        if end > self.bytes.len() {
            return Err(core::fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

// ---- catalog v6: the node cache over `door.nodes` ----------------------------------
impl<F: Vfs> PersistentVolume<'_, F> {
    fn touch_slot(&mut self, s: usize) {
        self.state.tick += 1;
        self.state.used[s] = self.state.tick;
    }
    /// Whether slot `s` holds what the tree has (nothing changed).
    fn clean(&self, s: usize) -> bool {
        let (n, stored) = (&self.state.nodes[s], self.state.stored[s]);
        n.id == stored.id && (n.id == 0 || nodes::Stored::of(n) == stored)
    }
    /// Whether a cache slot holds `id` now or held it (a removal not yet
    /// written): its cached copy decides, not the tree.
    fn cached_any(&self, id: u64) -> bool {
        (0..self.state.capacity)
            .any(|s| self.state.nodes[s].id == id || self.state.stored[s].id == id)
    }
    /// A slot for a node: a free one, else (catalog v6) the least recently
    /// used unchanged one past the 16 system folders, its node staying in
    /// the tree; when every slot holds a change, the changes are written to
    /// the tree first (staged, as every write before the commit).
    pub(crate) fn free_slot(&mut self) -> Result<usize> {
        for attempt in 0..2 {
            let cap = self.state.capacity;
            if let Some(s) =
                (0..cap).find(|&s| self.state.nodes[s].id == 0 && self.state.stored[s].id == 0)
            {
                self.touch_slot(s);
                return Ok(s);
            }
            if !self.state.tree {
                return Err(Error::NoSpace);
            }
            if let Some(s) = (0..cap)
                .filter(|&s| self.state.nodes[s].id > 16 && self.clean(s))
                .min_by_key(|&s| self.state.used[s])
            {
                self.state.nodes[s] = Node::EMPTY;
                self.state.stored[s] = nodes::Stored::NONE;
                self.touch_slot(s);
                return Ok(s);
            }
            if attempt == 0 {
                self.flush_nodes()?;
            }
        }
        Err(Error::NoSpace)
    }
    /// Reads node `id` from the tree.
    fn load(&mut self, id: u64) -> Result<Option<Node>> {
        let file = self.nodes_file.ok_or(Error::Corrupt)?;
        let state = &mut *self.state;
        let mut tree = nodes::open(&mut self.fs, file, &mut state.work)?;
        match tree.get(&nodes::node_key(id), &mut state.scan.value, &mut state.work)? {
            None => Ok(None),
            Some(n) => nodes::decode(id, &state.scan.value[..n]).map(Some),
        }
    }
    /// The first tree key after `from` (or at it, `inclusive`) into
    /// `state.scan.key`: its length.
    fn tree_next(&mut self, from: &[u8], inclusive: bool) -> Result<Option<usize>> {
        let file = self.nodes_file.ok_or(Error::Corrupt)?;
        let state = &mut *self.state;
        let mut tree = nodes::open(&mut self.fs, file, &mut state.work)?;
        Ok(tree
            .next(
                from,
                inclusive,
                &mut state.scan.key,
                &mut state.scan.value,
                &mut state.work,
            )?
            .map(|(k, _)| k))
    }
    /// The cache slot of node `id`, loading it from the tree (catalog v6).
    pub(crate) fn index(&mut self, id: u64) -> Result<usize> {
        if id == 0 {
            return Err(Error::NotFound);
        }
        if let Some(s) = (0..self.state.capacity).find(|&s| self.state.nodes[s].id == id) {
            self.touch_slot(s);
            return Ok(s);
        }
        if !self.state.tree || self.cached_any(id) {
            return Err(Error::NotFound);
        }
        let node = self.load(id)?.ok_or(Error::NotFound)?;
        let s = self.free_slot()?;
        self.state.nodes[s] = node;
        self.state.stored[s] = nodes::Stored::of(&node);
        Ok(s)
    }
    /// The child `name` of folder `parent` (folded: case-insensitive).
    fn child(&mut self, parent: u64, name: Name, fold: bool) -> Result<usize> {
        if parent == 10 && fold && name.same(Name::ascii("Users").unwrap(), true) {
            return self.index(3);
        }
        if let Some(s) = (0..self.state.capacity).find(|&s| {
            let n = &self.state.nodes[s];
            n.id != 0 && n.parent == parent && n.id != parent && n.name.same(name, fold)
        }) {
            self.touch_slot(s);
            return Ok(s);
        }
        if !self.state.tree {
            return Err(Error::NotFound);
        }
        let mut prefix = [0u8; nodes::KEY_BYTES];
        let length = nodes::child_key(parent, &name, None, &mut prefix);
        let mut cursor = prefix;
        let mut at = length;
        let mut inclusive = true;
        loop {
            let Some(k) = self.tree_next(&cursor[..at], inclusive)? else {
                return Err(Error::NotFound);
            };
            if k != length + 8 || self.state.scan.key[..length] != prefix[..length] {
                return Err(Error::NotFound);
            }
            let id = u64::from_be_bytes(self.state.scan.key[length..k].try_into().unwrap());
            cursor[..k].copy_from_slice(&self.state.scan.key[..k]);
            at = k;
            inclusive = false;
            // A cached node was compared above as it is now.
            if id == parent || self.cached_any(id) {
                continue;
            }
            let s = self.index(id)?;
            let n = self.state.nodes[s];
            if n.parent == parent && n.name.same(name, fold) {
                return Ok(s);
            }
        }
    }
    /// The child of `parent` with the smallest object ID above `after`
    /// (delete-pending ones included).
    pub(crate) fn next_child(&mut self, parent: u64, after: u64) -> Result<Option<usize>> {
        let mut best = (0..self.state.capacity)
            .filter(|&s| {
                let n = &self.state.nodes[s];
                n.id > after && n.parent == parent && n.id != parent
            })
            .map(|s| self.state.nodes[s].id)
            .min();
        if self.state.tree {
            let mut cursor = nodes::list_key(parent, after);
            loop {
                let Some(k) = self.tree_next(&cursor, false)? else {
                    break;
                };
                if k != 17 || self.state.scan.key[..9] != cursor[..9] {
                    break;
                }
                let id = u64::from_be_bytes(self.state.scan.key[9..17].try_into().unwrap());
                if best.is_some_and(|b| id >= b) {
                    break;
                }
                cursor.copy_from_slice(&self.state.scan.key[..17]);
                if id == parent || self.cached_any(id) {
                    continue;
                }
                best = Some(id);
                break;
            }
        }
        match best {
            None => Ok(None),
            Some(id) => self.index(id).map(Some),
        }
    }
    /// Whether folder `id` has any child (delete-pending ones included).
    fn has_children(&mut self, id: u64) -> Result<bool> {
        Ok(self.next_child(id, 0)?.is_some())
    }
    /// Writes every changed cache slot to the tree (staged until commit).
    pub(crate) fn flush_nodes(&mut self) -> Result<()> {
        if !self.state.tree {
            return Ok(());
        }
        let file = self.nodes_file.ok_or(Error::Corrupt)?;
        for s in 0..self.state.capacity {
            if self.clean(s) {
                continue;
            }
            let n = self.state.nodes[s];
            let stored = self.state.stored[s].id;
            let result = (|| {
                // The tree's copy names the index records to replace.
                let old = if stored == 0 {
                    Node::EMPTY
                } else {
                    self.load(stored)?.ok_or(Error::Corrupt)?
                };
                self.write_node(file, &n, &old)
            })();
            if let Err(e) = result {
                self.poisoned = true;
                return Err(e);
            }
            self.state.stored[s] = nodes::Stored::of(&n);
        }
        Ok(())
    }
    fn write_node(&mut self, file: FileId, n: &Node, old: &Node) -> Result<()> {
        let state = &mut *self.state;
        let mut tree = nodes::open(&mut self.fs, file, &mut state.work)?;
        let work = &mut state.work;
        let mut key = [0u8; nodes::KEY_BYTES];
        let moved = old.id != n.id || old.parent != n.parent || old.name != n.name;
        if old.id != 0 && moved {
            if old.id != n.id {
                tree.delete(&nodes::node_key(old.id), work)?;
                tree.delete(&nodes::pending_key(old.id), work)?;
            }
            let k = nodes::child_key(old.parent, &old.name, Some(old.id), &mut key);
            tree.delete(&key[..k], work)?;
            tree.delete(&nodes::list_key(old.parent, old.id), work)?;
        }
        if n.id != 0 {
            let length = nodes::encode(n, &mut state.scan.value);
            tree.put(&nodes::node_key(n.id), &state.scan.value[..length], work)?;
            if moved {
                let k = nodes::child_key(n.parent, &n.name, Some(n.id), &mut key);
                tree.put(&key[..k], &[], work)?;
                tree.put(&nodes::list_key(n.parent, n.id), &[], work)?;
            }
            let was_pending = old.id == n.id && old.pending;
            if n.pending && !was_pending {
                tree.put(&nodes::pending_key(n.id), &[], work)?;
            } else if !n.pending && was_pending {
                tree.delete(&nodes::pending_key(n.id), work)?;
            }
        }
        Ok(())
    }
    /// Catalog v6's delete-pending nodes (`D` records), up to `out.len()`.
    fn pending_ids(&mut self, out: &mut [u64]) -> Result<usize> {
        let mut n = 0;
        if !self.state.tree {
            return Ok(0);
        }
        let mut cursor = nodes::pending_key(0);
        let mut inclusive = true;
        while n < out.len() {
            let Some(k) = self.tree_next(&cursor, inclusive)? else {
                break;
            };
            if k != 9 || self.state.scan.key[0] != b'D' {
                break;
            }
            cursor.copy_from_slice(&self.state.scan.key[..9]);
            inclusive = false;
            out[n] = u64::from_be_bytes(cursor[1..9].try_into().unwrap());
            n += 1;
        }
        Ok(n)
    }
}
