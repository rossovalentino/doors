//! Single protected owner of the P07 volume. The kernel supplies only bounded
//! block transfers; parsing, namespace authorization and redo commit run here.
use crate::server::{self, Handler, Response, Server};
use core::{
    cell::UnsafeCell,
    sync::atomic::{AtomicBool, Ordering},
};
use door_abi::{
    Handle, Status, Transfer,
    platform::{Identity, StartupV2, inherited, mode},
};
use door_service_protocol::{CHUNK_BYTES, Code, LONG_BYTES, PATH_BYTES, manifest, opcode};
use door_storage_core::{
    persistence::{
        Access, Acl, Context, Disposition, FileHandle, OpenOptions, PRODUCTION_CATALOG_BYTES, Path,
        PersistentVolume, Setting, State, Subject,
    },
    storage::{self, BlockDevice, Ext2, Geometry, Gpt, Journal, Vfs, WorkingSet},
};
mod fileops;
mod files;
mod registry;
mod replacement;
use door_file_ops::Engine;
use registry::OpenKey;
use replacement::Staging;

struct Scratch {
    busy: AtomicBool,
    work: UnsafeCell<WorkingSet>,
    state: UnsafeCell<State>,
    catalog: UnsafeCell<[u8; PRODUCTION_CATALOG_BYTES]>,
    staging: UnsafeCell<Staging>,
    engine: UnsafeCell<Engine>,
}
// One initial thread owns the entire storage service; busy rejects reentry before
// any mutable borrow. The service never creates another thread or exposes state.
unsafe impl Sync for Scratch {}
static SCRATCH: Scratch = Scratch {
    busy: AtomicBool::new(false),
    work: UnsafeCell::new(WorkingSet::new()),
    state: UnsafeCell::new(State::new()),
    catalog: UnsafeCell::new([0; PRODUCTION_CATALOG_BYTES]),
    staging: UnsafeCell::new(Staging::new()),
    engine: UnsafeCell::new(Engine::new()),
};
struct Device {
    handle: Handle,
    geometry: Geometry,
}
impl Device {
    fn new(handle: Handle) -> Result<Self, Status> {
        let info = door_sys::block_info(handle)?;
        Ok(Self {
            handle,
            geometry: Geometry {
                sector_size: info.sector_bytes as u32,
                sectors: info.sectors,
                read_only: !info.writable,
                flush_supported: info.flush,
            },
        })
    }
}
fn disk_error(error: Status) -> storage::Error {
    match error {
        Status::DeviceLost => storage::Error::DeviceLost,
        Status::Timeout => storage::Error::Timeout,
        Status::AccessDenied => storage::Error::ReadOnly,
        Status::Unsupported => storage::Error::Unsupported,
        _ => storage::Error::Io,
    }
}
impl BlockDevice for Device {
    fn geometry(&self) -> Geometry {
        self.geometry
    }
    fn read_sectors(&mut self, lba: u64, output: &mut [u8]) -> storage::Result<()> {
        storage::check_transfer(self.geometry, lba, output.len())?;
        for (index, chunk) in output.chunks_mut(4096).enumerate() {
            door_sys::block_read(self.handle, lba + index as u64 * 8, chunk).map_err(disk_error)?;
        }
        Ok(())
    }
    fn write_sectors(&mut self, lba: u64, input: &[u8]) -> storage::Result<()> {
        storage::check_transfer(self.geometry, lba, input.len())?;
        for (index, chunk) in input.chunks(4096).enumerate() {
            door_sys::block_write(self.handle, lba + index as u64 * 8, chunk)
                .map_err(disk_error)?;
        }
        Ok(())
    }
    fn flush(&mut self) -> storage::Result<()> {
        door_sys::block_flush(self.handle).map_err(disk_error)
    }
}
type Volume<'a> = PersistentVolume<'a, Ext2<Journal<'a, Device>>>;
struct Open {
    channel: Handle,
    pid: u64,
    session: u64,
    epoch: u64,
    token: u64,
    file: FileHandle,
}
pub struct Broker<'a> {
    disk_info: door_service_protocol::administration::DiskInfo,
    volume: Volume<'a>,
    ordinary: bool,
    files: [Option<Open>; manifest::MAX_OPEN_FILES],
    keys: [Option<OpenKey>; manifest::MAX_OPEN_KEYS],
    /// A long registry value being staged (REGISTRY_STAGE_VALUE): one at a
    /// time; another key's staging waits (WouldBlock) until it is set.
    value_stage: registry::Stage,
    next: u64,
    staging: &'a mut Staging,
    /// The storage manifest's immutable system identity: operation records
    /// and protected folders, never user data.
    system: Subject,
    engine: &'a mut Engine,
    next_op: u64,
    watches: [Option<fileops::Watch>; fileops::WATCHES],
    /// Long requests being received (LONG_PART), per connection.
    long: [Option<Long>; LONGS],
}
/// A request body too long for one packet (a long path), as LONG_PART
/// pieces arrive.
struct Long {
    channel: Handle,
    len: usize,
    bytes: [u8; LONG_BYTES],
}
/// Connections that may send a long request at once.
const LONGS: usize = 4;
/// The operations a long request may carry: the ones that take paths.
const LONG_OPERATIONS: [u32; 17] = [
    opcode::OPEN,
    opcode::OPEN_WINDOWS,
    opcode::METADATA,
    opcode::METADATA_FOR,
    opcode::OPEN_FOR,
    opcode::LIST,
    opcode::NAME_PART,
    opcode::MKDIR,
    opcode::REMOVE,
    opcode::RENAME,
    opcode::SECURITY,
    opcode::SET_SECURITY,
    opcode::FILEOP_BEGIN,
    opcode::FILEOP_ADD,
    opcode::RECYCLE,
    opcode::WATCH,
    opcode::REGISTRY_OPEN,
];
fn persistence_error(error: door_storage_core::persistence::Error) -> Code {
    use door_storage_core::persistence::Error as E;
    match error {
        E::AccessDenied => Code::Denied,
        E::SharingViolation => Code::SharingViolation,
        E::DeletePending | E::KeyDeleted => Code::DeletePending,
        E::NotFound | E::Storage(storage::Error::NotFound) => Code::NotFound,
        E::AlreadyExists => Code::Exists,
        E::Changed => Code::Changed,
        E::NoSpace | E::Storage(storage::Error::NoSpace) => Code::Resources,
        E::InvalidHandle => Code::InvalidHandle,
        E::Corrupt | E::Storage(storage::Error::Corrupt) => Code::Corrupt,
        E::Unsupported | E::Storage(storage::Error::Unsupported) => Code::Unsupported,
        E::InvalidName | E::InvalidParameter | E::TypeMismatch | E::BufferTooSmall => {
            Code::InvalidRequest
        }
        _ => Code::Io,
    }
}
fn u64at(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}
fn subject(peer: Identity) -> Result<Subject, Code> {
    if !(server::session_peer(peer) || trusted_init(peer) || trusted_session(peer))
        || peer.groups > u64::from(u32::MAX)
        || peer.integrity > 2
    {
        return Err(Code::Denied);
    }
    // Only Server calls dispatch, after kernel identity(channel) validates the
    // immutable peer and its current session epoch, never request payload fields.
    unsafe { Subject::authenticated(peer.uid, peer.groups as u32, peer.integrity as u8) }
        .map_err(persistence_error)
}
/// The session service (uid 2): its private credential folder only, by ACL.
fn trusted_session(peer: Identity) -> bool {
    peer.manifest == manifest::SESSION
        && peer.uid == door_storage_core::persistence::SESSION_UID
        && peer.session == 0
        && peer.epoch == 0
        && peer.instance != 0
}
fn trusted_init(peer: Identity) -> bool {
    peer.manifest == manifest::INIT
        && peer.uid == 1
        && peer.integrity == 2
        && peer.session == 0
        && peer.epoch == 0
        && peer.instance != 0
}
impl<'a> Broker<'a> {
    /// READ_SECTION / WRITE_SECTION (P18 bulk file data): file bytes moved
    /// to or from the client's transferred section in 16 KiB steps (the
    /// kernel copies each step in or out of the section).
    fn section_io(
        &mut self,
        channel: Handle,
        peer: Identity,
        operation: u32,
        body: &[u8],
        section: Handle,
    ) -> Result<Response, Code> {
        use door_service_protocol::files::SectionIo;
        let request = SectionIo::decode(body)?;
        let index = self.file(channel, peer, request.file)?;
        server::transferred_capability(section, door_abi::HandleKind::Section)
            .map_err(server::status)?;
        let bytes = request.bytes as usize;
        let mut buffer = [0u8; 16 * 1024];
        let mut done = 0usize;
        if operation == opcode::READ_SECTION {
            while done < bytes {
                let want = (bytes - done).min(buffer.len());
                let n = self
                    .volume
                    .read(
                        &self.files[index].as_ref().unwrap().file,
                        request.offset + done as u64,
                        &mut buffer[..want],
                    )
                    .map_err(persistence_error)?;
                if n == 0 {
                    break;
                }
                door_sys::section_fill(section, request.section_offset + done as u64, &buffer[..n])
                    .map_err(server::status)?;
                done += n;
                if n < want {
                    break;
                }
            }
        } else {
            if self.ordinary {
                // As WRITE: growth stays contiguous (no huge sparse holes).
                let length = self
                    .volume
                    .info(&self.files[index].as_ref().unwrap().file)
                    .map_err(persistence_error)?
                    .bytes;
                if request.offset > length.saturating_add(4096) {
                    return Err(Code::Resources);
                }
            }
            while done < bytes {
                let want = (bytes - done).min(buffer.len());
                match door_sys::section_snapshot(
                    section,
                    request.section_offset + done as u64,
                    &mut buffer[..want],
                ) {
                    Ok(n) if n == want => {}
                    Ok(_) => return Err(Code::Io),
                    Err(error) => return Err(server::status(error)),
                }
                let n = self
                    .volume
                    .write(
                        &self.files[index].as_ref().unwrap().file,
                        request.offset + done as u64,
                        &buffer[..want],
                    )
                    .map_err(persistence_error)?;
                done += n;
                if n < want {
                    break;
                }
            }
        }
        Ok(Response::scalar(done as u64))
    }
    fn file(&self, channel: Handle, peer: Identity, token: u64) -> Result<usize, Code> {
        self.files
            .iter()
            .position(|entry| {
                entry.as_ref().is_some_and(|e| {
                    e.channel == channel
                        && e.pid == peer.pid
                        && e.session == peer.session
                        && e.epoch == peer.epoch
                        && e.token == token
                })
            })
            .ok_or(Code::InvalidHandle)
    }
    fn dispatch(
        &mut self,
        channel: Handle,
        peer: Identity,
        operation: u32,
        body: &[u8],
        section: Option<Handle>,
        now: u64,
    ) -> Result<Response, Code> {
        let subject = subject(peer)?;
        match operation {
            opcode::LONG_PART if !body.is_empty() => {
                let i = match self
                    .long
                    .iter()
                    .position(|l| l.as_ref().is_some_and(|l| l.channel == channel))
                {
                    Some(i) => i,
                    None => {
                        let i = self
                            .long
                            .iter()
                            .position(Option::is_none)
                            .ok_or(Code::Resources)?;
                        self.long[i] = Some(Long {
                            channel,
                            len: 0,
                            bytes: [0; LONG_BYTES],
                        });
                        i
                    }
                };
                let Some(l) = self.long[i].as_mut() else {
                    return Err(Code::Io);
                };
                if l.len + body.len() > LONG_BYTES {
                    self.long[i] = None;
                    return Err(Code::InvalidRequest);
                }
                l.bytes[l.len..l.len + body.len()].copy_from_slice(body);
                l.len += body.len();
                Ok(Response::empty())
            }
            opcode::LONG_COMMIT if body.len() == 4 => {
                let wanted = u32::from_le_bytes(body.try_into().unwrap());
                let l = self
                    .long
                    .iter_mut()
                    .find(|l| l.as_ref().is_some_and(|l| l.channel == channel))
                    .and_then(Option::take)
                    .ok_or(Code::InvalidRequest)?;
                if !LONG_OPERATIONS.contains(&wanted) {
                    return Err(Code::InvalidRequest);
                }
                self.dispatch(channel, peer, wanted, &l.bytes[..l.len], None, now)
            }
            opcode::DISK_INFO if body.is_empty() => Ok(Response::bytes(&self.disk_info.encode())),
            opcode::VOLUME_USAGE if body.is_empty() => {
                let (total_bytes, free_bytes) = self.volume.usage().ok_or(Code::Unsupported)?;
                let usage = door_service_protocol::administration::VolumeUsage {
                    total_bytes,
                    free_bytes: free_bytes.min(total_bytes),
                };
                Ok(Response::bytes(&usage.encode()))
            }
            opcode::GET_SETTING_FOR => {
                if !trusted_init(peer) {
                    return Err(Code::Denied);
                }
                if body.len() != 8 {
                    return Err(Code::InvalidRequest);
                }
                let uid = u32::from_le_bytes(body[..4].try_into().unwrap());
                let setting = Setting::from_id(u32::from_le_bytes(body[4..].try_into().unwrap()))
                    .ok_or(Code::InvalidRequest)?;
                Ok(Response::scalar(
                    self.volume
                        .setting_get(subject, uid, setting)
                        .map_err(persistence_error)? as u64,
                ))
            }
            opcode::SET_SETTING_FOR => {
                // Init persists display-service preferences (the shell layout,
                // controls changed from the desktop) for the session user.
                if !trusted_init(peer) {
                    return Err(Code::Denied);
                }
                if body.len() != 12 {
                    return Err(Code::InvalidRequest);
                }
                let uid = u32::from_le_bytes(body[..4].try_into().unwrap());
                let setting =
                    match Setting::from_id(u32::from_le_bytes(body[4..8].try_into().unwrap())) {
                        Some(
                            Setting::Theme | Setting::ScalePercent | Setting::DesktopAppearance,
                        )
                        | None => return Err(Code::InvalidRequest),
                        Some(setting) => setting,
                    };
                self.volume
                    .setting_set(
                        subject,
                        uid,
                        setting,
                        u32::from_le_bytes(body[8..12].try_into().unwrap()),
                    )
                    .map_err(persistence_error)?;
                self.volume.commit().map_err(persistence_error)?;
                Ok(Response::empty())
            }
            opcode::METADATA
            | opcode::FILE_INFO
            | opcode::LIST
            | opcode::NAME_PART
            | opcode::MKDIR
            | opcode::RENAME
            | opcode::RENAME_CHILD_CHECKED
            | opcode::REMOVE
            | opcode::REMOVE_CHILD_CHECKED
            | opcode::TRUNCATE
            | opcode::OPEN_FOR
            | opcode::METADATA_FOR
            | opcode::SECURITY
            | opcode::FILE_SECURITY
            | opcode::SET_SECURITY
            | opcode::SET_FILE_SECURITY => {
                self.file_operations(channel, peer, subject, operation, body)
            }
            opcode::REPLACE_BEGIN
            | opcode::REPLACE_CHUNK
            | opcode::REPLACE_COMMIT
            | opcode::REPLACE_ABORT
            | opcode::STAGE_BEGIN
            | opcode::STAGE_WRITE
            | opcode::STAGE_COMMIT
            | opcode::STAGE_ABORT => self.replacement(channel, peer, operation, body, now),
            opcode::STAGE_WRITE_SECTION => {
                let section = section.ok_or(Code::InvalidRequest)?;
                self.stage_section(channel, peer, body, section, now)
            }
            opcode::READ_SECTION | opcode::WRITE_SECTION => {
                let section = section.ok_or(Code::InvalidRequest)?;
                self.section_io(channel, peer, operation, body, section)
            }
            opcode::OPEN | opcode::OPEN_WINDOWS => {
                if body.len() < 8
                    || body[1] != 0
                    || body.len() != 8 + u16::from_le_bytes(body[6..8].try_into().unwrap()) as usize
                    || body.len() > 8 + PATH_BYTES
                {
                    return Err(Code::InvalidRequest);
                }
                let disposition = match body[0] {
                    0 => Disposition::CreateNew,
                    1 => Disposition::OpenExisting,
                    2 => Disposition::OpenAlways,
                    3 => Disposition::CreateAlways,
                    4 => Disposition::TruncateExisting,
                    _ => return Err(Code::InvalidRequest),
                };
                let access = Access(u16::from_le_bytes(body[2..4].try_into().unwrap()));
                let share = Access(u16::from_le_bytes(body[4..6].try_into().unwrap()));
                if access.0 & !7 != 0 || share.0 & !7 != 0 {
                    return Err(Code::InvalidRequest);
                }
                let mut wide = [0u16; PATH_BYTES];
                let (path, context) = if operation == opcode::OPEN_WINDOWS {
                    if !(body.len() - 8).is_multiple_of(2) {
                        return Err(Code::InvalidRequest);
                    }
                    for (out, bytes) in wide.iter_mut().zip(body[8..].chunks_exact(2)) {
                        *out = u16::from_le_bytes(bytes.try_into().unwrap());
                    }
                    let home: &[u8] = match peer.uid {
                        1001 => b"C:\\Users\\1001",
                        1002 => b"C:\\Users\\1002",
                        _ => return Err(Code::Denied),
                    };
                    let mut units = [0u16; 32];
                    for (out, byte) in units.iter_mut().zip(home) {
                        *out = u16::from(*byte);
                    }
                    let context = self
                        .volume
                        .change_directory(
                            subject,
                            Path::Windows(&units[..home.len()]),
                            Context::root(),
                        )
                        .map_err(persistence_error)?;
                    (Path::Windows(&wide[..(body.len() - 8) / 2]), context)
                } else {
                    // A native path, or a Windows path (`C:...`) resolved the
                    // Windows way.
                    (files::path(&body[8..], &mut wide)?, Context::root())
                };
                let index = self
                    .files
                    .iter()
                    .position(Option::is_none)
                    .ok_or(Code::Resources)?;
                let next = self.next.checked_add(1).ok_or(Code::Resources)?;
                let (file, created) = self
                    .volume
                    .open(
                        subject,
                        path,
                        context,
                        OpenOptions {
                            access,
                            share,
                            disposition,
                        },
                    )
                    .map_err(persistence_error)?;
                let token = self.next;
                self.next = next;
                self.files[index] = Some(Open {
                    channel,
                    pid: peer.pid,
                    session: peer.session,
                    epoch: peer.epoch,
                    token,
                    file,
                });
                if operation == opcode::OPEN_WINDOWS {
                    let mut bytes = [0u8; 16];
                    bytes[..8].copy_from_slice(&token.to_le_bytes());
                    bytes[8] = u8::from(created);
                    Ok(Response::bytes(&bytes))
                } else {
                    Ok(Response::scalar(token))
                }
            }
            opcode::READ if body.len() == 24 => {
                let index = self.file(channel, peer, u64at(body, 0))?;
                let length = usize::try_from(u64at(body, 16)).map_err(|_| Code::InvalidRequest)?;
                if length > CHUNK_BYTES {
                    return Err(Code::InvalidRequest);
                }
                let mut response = Response::empty();
                response.len = self
                    .volume
                    .read(
                        &self.files[index].as_ref().unwrap().file,
                        u64at(body, 8),
                        &mut response.body[..length],
                    )
                    .map_err(persistence_error)?;
                Ok(response)
            }
            opcode::WRITE if (16..=16 + CHUNK_BYTES).contains(&body.len()) => {
                let index = self.file(channel, peer, u64at(body, 0))?;
                if self.ordinary {
                    let length = self
                        .volume
                        .info(&self.files[index].as_ref().unwrap().file)
                        .map_err(persistence_error)?
                        .bytes;
                    let end = u64at(body, 8)
                        .checked_add((body.len() - 16) as u64)
                        .ok_or(Code::InvalidRequest)?;
                    if end > length.saturating_add(4096) {
                        return Err(Code::Resources);
                    }
                }
                let count = self
                    .volume
                    .write(
                        &self.files[index].as_ref().unwrap().file,
                        u64at(body, 8),
                        &body[16..],
                    )
                    .map_err(persistence_error)?;
                Ok(Response::scalar(count as u64))
            }
            opcode::CLOSE if body.len() == 8 => {
                let index = self.file(channel, peer, u64at(body, 0))?;
                self.staging.close(channel, peer, u64at(body, 0));
                self.abort_large_file(channel, peer, u64at(body, 0));
                self.volume
                    .close(self.files[index].take().unwrap().file)
                    .map_err(persistence_error)?;
                Ok(Response::empty())
            }
            opcode::FLUSH if body.is_empty() => {
                self.volume.commit().map_err(persistence_error)?;
                Ok(Response::empty())
            }
            opcode::GET_SETTING | opcode::SET_SETTING => {
                if body.len()
                    != if operation == opcode::GET_SETTING {
                        4
                    } else {
                        8
                    }
                {
                    return Err(Code::InvalidRequest);
                }
                let setting = Setting::from_id(u32::from_le_bytes(body[..4].try_into().unwrap()))
                    .ok_or(Code::InvalidRequest)?;
                if operation == opcode::GET_SETTING {
                    Ok(Response::scalar(
                        self.volume
                            .setting_get(subject, peer.uid, setting)
                            .map_err(persistence_error)? as u64,
                    ))
                } else {
                    self.volume
                        .setting_set(
                            subject,
                            peer.uid,
                            setting,
                            u32::from_le_bytes(body[4..8].try_into().unwrap()),
                        )
                        .map_err(persistence_error)?;
                    Ok(Response::empty())
                }
            }
            opcode::READ | opcode::WRITE | opcode::CLOSE | opcode::FLUSH => {
                Err(Code::InvalidRequest)
            }
            opcode::FILEOP_BEGIN
            | opcode::FILEOP_ADD
            | opcode::FILEOP_STEP
            | opcode::FILEOP_QUERY
            | opcode::FILEOP_RESOLVE
            | opcode::FILEOP_CONTROL
            | opcode::FILEOP_LIST
            | opcode::RECYCLE
            | opcode::RECYCLE_LIST
            | opcode::RECYCLE_RESTORE
            | opcode::RECYCLE_PURGE
            | opcode::RECYCLE_USAGE
            | opcode::WATCH
            | opcode::WATCH_POLL
            | opcode::UNWATCH => self.file_ops(channel, peer, subject, operation, body),
            opcode::REGISTRY_OPEN
            | opcode::REGISTRY_QUERY
            | opcode::REGISTRY_SET
            | opcode::REGISTRY_VERSION
            | opcode::REGISTRY_ENUM_KEY
            | opcode::REGISTRY_ENUM_VALUE
            | opcode::REGISTRY_SET_CHECKED
            | opcode::REGISTRY_DELETE_VALUE
            | opcode::REGISTRY_DELETE_KEY
            | opcode::REGISTRY_STAGE_VALUE
            | opcode::REGISTRY_CLOSE => self.registry(channel, peer, subject, operation, body),
            _ => Err(Code::Unsupported),
        }
    }
}
impl Handler for Broker<'_> {
    fn request(
        &mut self,
        channel: Handle,
        peer: Identity,
        operation: u32,
        body: &[u8],
        transfers: &[Transfer],
    ) -> Response {
        // Only the section requests carry a handle (their section).
        let section = match transfers {
            [] => None,
            [section]
                if matches!(
                    operation,
                    opcode::STAGE_WRITE_SECTION | opcode::READ_SECTION | opcode::WRITE_SECTION
                ) =>
            {
                Some(section.handle)
            }
            _ => return Response::error(Code::InvalidRequest),
        };
        if self.ordinary
            && let Err(error) = self.volume.checkpoint_budget(24)
        {
            return Response::error(persistence_error(error));
        }
        // Abandoned candidates (idle past IDLE_TICKS) release their slots and
        // reserved blocks before this request can need them.
        let now = door_sys::time().unwrap_or(0);
        self.expire_staging(now);
        // Timestamps for this request (catalog v4/v5). Without a valid RTC new
        // nodes record "not recorded" (0) and existing nodes keep their times;
        // a transient RTC update reuses no stale value.
        self.volume.set_clock(door_sys::wall_time().unwrap_or(0));
        self.dispatch(channel, peer, operation, body, section, now)
            .unwrap_or_else(Response::error)
    }
    fn disconnected(&mut self, channel: Handle) {
        for l in &mut self.long {
            if l.as_ref().is_some_and(|l| l.channel == channel) {
                *l = None;
            }
        }
        self.drop_watches(channel);
        self.staging.disconnected(channel);
        self.abort_large_channel(channel);
        for entry in &mut self.files {
            if entry.as_ref().is_some_and(|e| e.channel == channel) {
                let _ = self.volume.close(entry.take().unwrap().file);
            }
        }
        for entry in &mut self.keys {
            if entry.as_ref().is_some_and(|e| e.channel == channel) {
                let _ = self.volume.registry_close(entry.take().unwrap().key);
            }
        }
    }
    fn shutdown(&mut self) -> Result<(), Status> {
        self.volume.commit().map_err(|_| Status::IoFailure)
    }
}
fn mount(startup: &StartupV2) -> Result<Broker<'static>, Code> {
    let mut device =
        Device::new(startup.base.inherited[inherited::AUTHORITY].handle).map_err(server::status)?;
    let table = Gpt::read(&mut device).map_err(|e| persistence_error(e.into()))?;
    let home = table
        .partitions()
        .find(|p| p.id == *b"DoorP07ExtGUID00")
        .ok_or(Code::Corrupt)?;
    let log = table
        .partitions()
        .find(|p| p.type_guid == *b"DoorRedoJournal1")
        .ok_or(Code::Corrupt)?;
    let geometry = device.geometry();
    let disk_info = door_service_protocol::administration::DiskInfo {
        sectors: geometry.sectors,
        volume_first: home.first,
        volume_sectors: home.sectors,
        journal_first: log.first,
        journal_sectors: log.sectors,
        writable: !geometry.read_only,
        flush: geometry.flush_supported,
    };
    if SCRATCH.busy.swap(true, Ordering::AcqRel) {
        return Err(Code::Resources);
    }
    // This process owns one static working set until process exit; no service
    // restart reuses its address space. Failure never mounts an empty fallback.
    let (work, state, catalog, staging, engine) = unsafe {
        (
            &mut *SCRATCH.work.get(),
            &mut *SCRATCH.state.get(),
            &mut *SCRATCH.catalog.get(),
            &mut *SCRATCH.staging.get(),
            &mut *SCRATCH.engine.get(),
        )
    };
    let (mut journal, recovered) = Journal::open(device, home, log, table.disk_id, work)
        .map_err(|e| persistence_error(e.into()))?;
    journal.begin().map_err(|e| persistence_error(e.into()))?;
    let mut fs = Ext2::mount(journal).map_err(|e| persistence_error(e.into()))?;
    let initialize = match fs.open(door_storage_core::persistence::CATALOG_NAME) {
        Ok(_) => false,
        Err(storage::Error::NotFound)
            if (startup.mode == mode::SAVE || startup.mode == 9) && recovered.generation == 0 =>
        {
            true
        }
        Err(error) => return Err(persistence_error(error.into())),
    };
    let mut volume = if initialize {
        if startup.mode == 9 {
            state.select_production().map_err(persistence_error)?;
        }
        // The storage manifest's immutable system identity is the provisioning
        // authority. This is never inferred from a request or displayed name.
        let system = unsafe {
            Subject::authenticated(
                startup.identity.uid,
                startup.identity.groups as u32,
                startup.identity.integrity as u8,
            )
        }
        .map_err(persistence_error)?;
        let mut volume =
            PersistentVolume::initialize(fs, state, catalog, system).map_err(persistence_error)?;
        for (uid, path) in [(1001, "/users/1001"), (1002, "/users/1002")] {
            volume
                .mkdir(
                    system,
                    Path::Native(path),
                    Context::root(),
                    Some(Acl::owner_full(uid)),
                )
                .map_err(persistence_error)?;
        }
        if startup.mode == 9 {
            volume
                .mkdir(system, Path::Native("/system/bin"), Context::root(), None)
                .map_err(persistence_error)?;
        }
        volume
    } else {
        PersistentVolume::mount(fs, state, catalog).map_err(persistence_error)?
    };
    // The storage manifest's immutable identity owns operation records and
    // protected service folders (never inferred from a request).
    let system = unsafe {
        Subject::authenticated(
            startup.identity.uid,
            startup.identity.groups as u32,
            startup.identity.integrity as u8,
        )
    }
    .map_err(persistence_error)?;
    if startup.mode == mode::SESSION || startup.mode == 9 {
        // The session service's private credential folder (created on an
        // older volume at its first P14 mount; a no-op afterwards).
        volume
            .ensure_credentials(system)
            .map_err(persistence_error)?;
        // The Windows C: layout (Program Files and ProgramData writable by
        // Administrators, Users\Public, the profiles' known folders), also
        // on volumes an older build created.
        volume
            .ensure_windows_layout(system)
            .map_err(persistence_error)?;
        // Every profile's hive has HKEY_CURRENT_USER\Software, as Windows'
        // default user hive does (programs open it before writing under
        // it). The 128-key registry gets that one key per profile.
        let software: [u16; 8] = core::array::from_fn(|i| u16::from(b"Software"[i]));
        // The service account's too (its services read HKCU\Software).
        for uid in [1001, 1002, door_abi::platform::SERVICE_ACCOUNT] {
            let user = unsafe { Subject::authenticated(uid, 0, 1) }.map_err(persistence_error)?;
            let key = volume
                .registry_open(
                    user,
                    door_storage_core::persistence::RegistryRoot::CurrentUser,
                    door_storage_core::persistence::RegistryView::Bits64,
                    &software,
                    Access::READ,
                    true,
                )
                .map_err(persistence_error)?;
            volume.registry_close(key).map_err(persistence_error)?;
        }
    }
    volume.commit().map_err(persistence_error)?;
    // Operations a reboot interrupted come back Paused; unstarted ones are
    // dropped. Without records this stages nothing and performs no I/O.
    let next_op = {
        let mut vol = door_storage_core::persistence::fileops::OpVolume {
            volume: &mut volume,
            user: system,
            system,
        };
        engine.recover(&mut vol).map_err(fileops::op_code)?;
        let mut last = 0;
        while let Some(op) =
            door_file_ops::Volume::next_record(&mut vol, last).map_err(fileops::op_code)?
        {
            last = op;
        }
        // Milliseconds since 1601 keep IDs increasing across dismissals.
        (last + 1).max(door_sys::wall_time().unwrap_or(0) / 10_000)
    };
    Ok(Broker {
        disk_info,
        ordinary: startup.mode == mode::SESSION,
        volume,
        files: core::array::from_fn(|_| None),
        keys: core::array::from_fn(|_| None),
        value_stage: registry::Stage::EMPTY,
        next: 1,
        staging,
        system,
        engine,
        next_op,
        watches: [None; fileops::WATCHES],
        long: core::array::from_fn(|_| None),
    })
}
pub fn run(startup: StartupV2) -> i64 {
    let broker = match mount(&startup) {
        Ok(broker) => broker,
        Err(_) => {
            let _ = door_sys::write(b"DOOR_P08 STORAGE REFUSED\n");
            return -120;
        }
    };
    let server = match Server::new(&startup) {
        Ok(server) => server,
        Err(_) => return -121,
    };
    if server.ready(manifest::STORAGE).is_err() {
        return -122;
    }
    server::serve(server, broker)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_immutable_init_identity_can_delegate_and_provision() {
        let peer = Identity {
            pid: 2,
            uid: 1,
            integrity: 2,
            groups: 0,
            session: 0,
            epoch: 0,
            manifest: manifest::INIT,
            instance: 1,
            capabilities: 0,
        };
        assert!(trusted_init(peer));
        assert_eq!(subject(peer).unwrap().identity(), 1);
        for bad in [
            Identity { uid: 1001, ..peer },
            Identity {
                manifest: manifest::STORAGE,
                ..peer
            },
            Identity {
                session: 1,
                epoch: 1,
                ..peer
            },
            Identity {
                integrity: 1,
                ..peer
            },
            Identity {
                instance: 0,
                ..peer
            },
        ] {
            assert!(!trusted_init(bad));
            assert!(subject(bad).is_err());
        }
    }
    #[test]
    fn the_session_service_identity_is_admitted_only_with_its_immutable_manifest() {
        let peer = Identity {
            pid: 4,
            uid: 2,
            integrity: 2,
            groups: 0,
            session: 0,
            epoch: 0,
            manifest: manifest::SESSION,
            instance: 3,
            capabilities: door_abi::platform::capability::SESSION,
        };
        assert_eq!(subject(peer).unwrap().identity(), 2);
        for bad in [
            Identity { uid: 1001, ..peer },
            Identity {
                manifest: manifest::CONSOLE,
                ..peer
            },
            Identity {
                session: 1,
                epoch: 1,
                ..peer
            },
            Identity {
                instance: 0,
                ..peer
            },
        ] {
            assert!(!trusted_session(bad));
        }
    }
    #[test]
    fn subject_capture_rejects_non_session_services_and_unrepresentable_claims() {
        let peer = Identity {
            pid: 7,
            uid: 1001,
            integrity: 1,
            groups: 2,
            session: 3,
            epoch: 3,
            manifest: 5,
            instance: 9,
            capabilities: 0,
        };
        assert_eq!(subject(peer).unwrap().identity(), 1001);
        for invalid in [
            Identity { session: 0, ..peer },
            Identity { epoch: 0, ..peer },
            Identity {
                capabilities: 2,
                ..peer
            },
            Identity {
                groups: 1u64 << 40,
                ..peer
            },
            Identity {
                integrity: 3,
                ..peer
            },
            Identity { uid: 1, ..peer },
        ] {
            assert_eq!(subject(invalid).err(), Some(Code::Denied));
        }
    }
}
