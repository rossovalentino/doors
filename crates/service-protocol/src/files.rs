//! Fixed little-endian metadata records. Directory names retain UTF-16 units.
use crate::Code;
pub const INFO_BYTES: usize = 32;
/// METADATA / FILE_INFO / METADATA_FOR replies: `FileInfo` then `FileTimes`.
pub const STAT_BYTES: usize = INFO_BYTES + TIMES_BYTES;
pub const TIMES_BYTES: usize = 24;
/// A file or folder name: up to 255 UTF-16 units (catalog v6 volumes; a
/// constrained volume's names stop at 48).
pub const NAME_UNITS: usize = 255;
/// RENAME_CHILD_CHECKED's one-packet record keeps 48-unit names.
pub const RENAME_NAME_UNITS: usize = 48;
pub const RENAME_CHILD_BYTES: usize = 32 + RENAME_NAME_UNITS * 2;
pub const REPLACE_BYTES: usize = 4096;
/// Staged whole-file replacement limit (STAGE_*): the per-file cap.
pub const LARGE_REPLACE_BYTES: usize = 4_243_456;
pub const REPLACE_BEGIN_BYTES: usize = 32;
pub const REMOVE_CHILD_BYTES: usize = 24;
/// Candidate bytes per STAGE_WRITE message: the whole body after its
/// 24-byte header (file token, staging token, offset).
pub const STAGE_CHUNK_BYTES: usize = crate::BODY_BYTES - 24;
/// Candidate bytes per STAGE_WRITE_SECTION request: one bounded section
/// snapshot, so a request holds the single-threaded service briefly.
pub const STAGE_SECTION_BYTES: usize = door_abi::graphics::SECTION_COPY_BYTES;
pub const STAGE_SECTION_REQUEST_BYTES: usize = 36;
/// Requests that upload a staged candidate of `bytes` in `per_request`
/// pieces (BEGIN and COMMIT excluded).
pub const fn stage_requests(bytes: usize, per_request: usize) -> usize {
    bytes.div_ceil(per_request)
}
/// STAGE_WRITE_SECTION body: `[file u64][token u64][offset u64]
/// [section_offset u64][bytes u32]`. Candidate bytes `offset..offset+bytes`
/// are read from the transferred section at `section_offset`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StageSection {
    pub file: u64,
    pub token: u64,
    pub offset: u64,
    pub section_offset: u64,
    pub bytes: u32,
}
impl StageSection {
    pub fn new(
        file: u64,
        token: u64,
        offset: u64,
        section_offset: u64,
        bytes: usize,
    ) -> Result<Self, Code> {
        if file == 0
            || token == 0
            || bytes == 0
            || bytes > STAGE_SECTION_BYTES
            || offset.checked_add(bytes as u64).is_none()
            || section_offset.checked_add(bytes as u64).is_none()
        {
            return Err(Code::InvalidRequest);
        }
        Ok(Self {
            file,
            token,
            offset,
            section_offset,
            bytes: bytes as u32,
        })
    }
    pub fn encode(self) -> [u8; STAGE_SECTION_REQUEST_BYTES] {
        let mut out = [0; STAGE_SECTION_REQUEST_BYTES];
        for (chunk, value) in out[..32].chunks_exact_mut(8).zip([
            self.file,
            self.token,
            self.offset,
            self.section_offset,
        ]) {
            chunk.copy_from_slice(&value.to_le_bytes());
        }
        out[32..].copy_from_slice(&self.bytes.to_le_bytes());
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Code> {
        if bytes.len() != STAGE_SECTION_REQUEST_BYTES {
            return Err(Code::InvalidRequest);
        }
        let field = |i: usize| u64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().unwrap());
        Self::new(
            field(0),
            field(1),
            field(2),
            field(3),
            u32::from_le_bytes(bytes[32..].try_into().unwrap()) as usize,
        )
    }
}
/// The authenticated owner reauthorizes the current binding before deletion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemoveChild {
    parent: u64,
    object: u64,
    version: u64,
}
impl RemoveChild {
    pub fn parent(self) -> u64 {
        self.parent
    }
    pub fn object(self) -> u64 {
        self.object
    }
    pub fn version(self) -> u64 {
        self.version
    }
    pub fn new(parent: u64, object: u64, version: u64) -> Result<Self, Code> {
        if parent == 0 || object == 0 || version == 0 {
            return Err(Code::InvalidRequest);
        }
        Ok(Self {
            parent,
            object,
            version,
        })
    }
    pub fn encode(self) -> [u8; REMOVE_CHILD_BYTES] {
        let mut out = [0; REMOVE_CHILD_BYTES];
        for (chunk, value) in out
            .chunks_exact_mut(8)
            .zip([self.parent, self.object, self.version])
        {
            chunk.copy_from_slice(&value.to_le_bytes());
        }
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Code> {
        if bytes.len() != REMOVE_CHILD_BYTES {
            return Err(Code::InvalidRequest);
        }
        Self::new(
            u64::from_le_bytes(bytes[..8].try_into().unwrap()),
            u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
        )
    }
}
/// A numeric file token is checked against the kernel-authenticated channel;
/// object/version are comparison hints, never authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BeginReplace {
    file: u64,
    object: u64,
    version: u64,
    bytes: usize,
}
impl BeginReplace {
    pub fn new(file: u64, object: u64, version: u64, bytes: usize) -> Result<Self, Code> {
        Self::bounded(file, object, version, bytes, REPLACE_BYTES)
    }
    /// STAGE_BEGIN: the same 32-byte request, up to `LARGE_REPLACE_BYTES`.
    pub fn large(file: u64, object: u64, version: u64, bytes: usize) -> Result<Self, Code> {
        Self::bounded(file, object, version, bytes, LARGE_REPLACE_BYTES)
    }
    fn bounded(
        file: u64,
        object: u64,
        version: u64,
        bytes: usize,
        limit: usize,
    ) -> Result<Self, Code> {
        if file == 0 || object == 0 || version == 0 || bytes > limit {
            return Err(Code::InvalidRequest);
        }
        Ok(Self {
            file,
            object,
            version,
            bytes,
        })
    }
    pub fn file(self) -> u64 {
        self.file
    }
    pub fn object(self) -> u64 {
        self.object
    }
    pub fn version(self) -> u64 {
        self.version
    }
    pub fn bytes(self) -> usize {
        self.bytes
    }
    pub fn encode(self) -> [u8; REPLACE_BEGIN_BYTES] {
        let mut out = [0; REPLACE_BEGIN_BYTES];
        for (chunk, value) in
            out.chunks_exact_mut(8)
                .zip([self.file, self.object, self.version, self.bytes as u64])
        {
            chunk.copy_from_slice(&value.to_le_bytes());
        }
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Code> {
        Self::decode_bounded(bytes, REPLACE_BYTES)
    }
    pub fn decode_large(bytes: &[u8]) -> Result<Self, Code> {
        Self::decode_bounded(bytes, LARGE_REPLACE_BYTES)
    }
    fn decode_bounded(bytes: &[u8], limit: usize) -> Result<Self, Code> {
        if bytes.len() != REPLACE_BEGIN_BYTES {
            return Err(Code::InvalidRequest);
        }
        let mut fields = [0u64; 4];
        for (out, chunk) in fields.iter_mut().zip(bytes.chunks_exact(8)) {
            *out = u64::from_le_bytes(chunk.try_into().unwrap());
        }
        Self::bounded(
            fields[0],
            fields[1],
            fields[2],
            usize::try_from(fields[3]).map_err(|_| Code::InvalidRequest)?,
            limit,
        )
    }
}
/// Object/version hints carry no authority. The service authenticates the peer
/// and the current source/parent ancestry before one durable same-parent rename.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenameChild {
    parent: u64,
    object: u64,
    version: u64,
    name: [u16; RENAME_NAME_UNITS],
    length: usize,
}
impl RenameChild {
    pub fn new(parent: u64, object: u64, version: u64, name: &[u16]) -> Result<Self, Code> {
        if parent == 0
            || object == 0
            || version == 0
            || name.is_empty()
            || name.len() > RENAME_NAME_UNITS
            || name.contains(&0)
        {
            return Err(Code::InvalidRequest);
        }
        let mut value = Self {
            parent,
            object,
            version,
            name: [0; RENAME_NAME_UNITS],
            length: name.len(),
        };
        value.name[..name.len()].copy_from_slice(name);
        Ok(value)
    }
    pub fn parent(&self) -> u64 {
        self.parent
    }
    pub fn object(&self) -> u64 {
        self.object
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    pub fn name(&self) -> &[u16] {
        &self.name[..self.length]
    }
    pub fn encode(&self) -> [u8; RENAME_CHILD_BYTES] {
        let mut out = [0; RENAME_CHILD_BYTES];
        out[..8].copy_from_slice(&self.parent.to_le_bytes());
        out[8..16].copy_from_slice(&self.object.to_le_bytes());
        out[16..24].copy_from_slice(&self.version.to_le_bytes());
        out[24..26].copy_from_slice(&(self.length as u16).to_le_bytes());
        for (bytes, unit) in out[32..].chunks_exact_mut(2).zip(self.name()) {
            bytes.copy_from_slice(&unit.to_le_bytes());
        }
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Code> {
        if bytes.len() != RENAME_CHILD_BYTES || bytes[26..32].iter().any(|v| *v != 0) {
            return Err(Code::InvalidRequest);
        }
        let length = usize::from(u16::from_le_bytes(bytes[24..26].try_into().unwrap()));
        if length == 0
            || length > RENAME_NAME_UNITS
            || bytes[32 + length * 2..].iter().any(|v| *v != 0)
        {
            return Err(Code::InvalidRequest);
        }
        let mut name = [0; RENAME_NAME_UNITS];
        for (unit, pair) in name.iter_mut().zip(bytes[32..].chunks_exact(2)) {
            *unit = u16::from_le_bytes(pair.try_into().unwrap());
        }
        Self::new(
            u64::from_le_bytes(bytes[..8].try_into().unwrap()),
            u64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            &name[..length],
        )
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileInfo {
    pub bytes: u64,
    pub object: u64,
    pub version: u64,
    pub directory: bool,
    /// Windows file attributes (FILE_ATTRIBUTE_HIDDEN 0x2 and so on), at
    /// bytes 28..32; senders before P17 wrote zeros there.
    pub attributes: u32,
}
impl FileInfo {
    pub fn encode(self) -> [u8; INFO_BYTES] {
        let mut b = [0; INFO_BYTES];
        b[..8].copy_from_slice(&self.bytes.to_le_bytes());
        b[8..16].copy_from_slice(&self.object.to_le_bytes());
        b[16..24].copy_from_slice(&self.version.to_le_bytes());
        b[24] = u8::from(self.directory);
        b[28..].copy_from_slice(&self.attributes.to_le_bytes());
        b
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() != INFO_BYTES || b[24] > 1 || b[25..28].iter().any(|v| *v != 0) {
            return Err(Code::InvalidRequest);
        }
        let info = Self {
            bytes: u64::from_le_bytes(b[..8].try_into().unwrap()),
            object: u64::from_le_bytes(b[8..16].try_into().unwrap()),
            version: u64::from_le_bytes(b[16..24].try_into().unwrap()),
            directory: b[24] == 1,
            attributes: u32::from_le_bytes(b[28..].try_into().unwrap()),
        };
        if info.object == 0 || info.version == 0 || (info.directory && info.bytes != 0) {
            return Err(Code::InvalidRequest);
        }
        Ok(info)
    }
}
/// SECURITY / FILE_SECURITY replies and SET_SECURITY / SET_FILE_SECURITY
/// records: `[owner u32][integrity u8][count u8][directory u8][0]` then
/// four 12-byte entries `[principal u8][deny u8][access u16][id u32]
/// [inherit u8][0; 3]` (unused entries zero).
pub const SECURITY_BYTES: usize = 8 + SECURITY_ENTRIES * 12;
pub const SECURITY_ENTRIES: usize = 4;
/// `SecurityEntry::principal`.
pub mod principal {
    pub const EVERYONE: u8 = 0;
    /// `id` is a uid (1 is the system).
    pub const USER: u8 = 1;
    /// `id` is a group bit (`door_abi::platform::group`: 1 Users,
    /// 2 services, 3 Administrators).
    pub const GROUP: u8 = 2;
}
/// The access bits storage grants (storage-core's `Access`).
pub mod access {
    pub const READ: u16 = 1;
    pub const WRITE: u16 = 2;
    pub const DELETE: u16 = 4;
    pub const TRAVERSE: u16 = 8;
    pub const CREATE: u16 = 16;
    pub const CHANGE_ACL: u16 = 32;
    pub const ALL: u16 = 63;
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SecurityEntry {
    pub principal: u8,
    pub id: u32,
    pub deny: bool,
    pub access: u16,
    pub inherit: bool,
}
/// A file's or folder's access control list as storage keeps it: an owner
/// uid, a mandatory label (0 none, 1 medium, 2 system) and at most four
/// ordered entries.
/// `Security::owner` of what an elevated program creates: BUILTIN\
/// Administrators (S-1-5-32-544), Windows' default owner for an elevated
/// token. No account has this ID, so the same user's unelevated programs
/// do not get the owner's implicit rights (READ_CONTROL, WRITE_DAC).
pub const ADMINISTRATORS_OWNER: u32 = 0x8000_0220;
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Security {
    pub owner: u32,
    pub integrity: u8,
    pub directory: bool,
    pub count: u8,
    pub entries: [SecurityEntry; SECURITY_ENTRIES],
}
impl Security {
    pub fn entries(&self) -> &[SecurityEntry] {
        &self.entries[..usize::from(self.count).min(SECURITY_ENTRIES)]
    }
    pub fn encode(&self) -> [u8; SECURITY_BYTES] {
        let mut b = [0; SECURITY_BYTES];
        b[..4].copy_from_slice(&self.owner.to_le_bytes());
        b[4] = self.integrity;
        b[5] = self.count;
        b[6] = u8::from(self.directory);
        for (i, e) in self.entries().iter().enumerate() {
            let at = 8 + i * 12;
            b[at] = e.principal;
            b[at + 1] = u8::from(e.deny);
            b[at + 2..at + 4].copy_from_slice(&e.access.to_le_bytes());
            b[at + 4..at + 8].copy_from_slice(&e.id.to_le_bytes());
            b[at + 8] = u8::from(e.inherit);
        }
        b
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() != SECURITY_BYTES
            || b[4] > 2
            || usize::from(b[5]) > SECURITY_ENTRIES
            || b[6] > 1
            || b[7] != 0
        {
            return Err(Code::InvalidRequest);
        }
        let mut value = Self {
            owner: u32::from_le_bytes(b[..4].try_into().unwrap()),
            integrity: b[4],
            directory: b[6] == 1,
            count: b[5],
            entries: [SecurityEntry::default(); SECURITY_ENTRIES],
        };
        if value.owner == 0 {
            return Err(Code::InvalidRequest);
        }
        for i in 0..SECURITY_ENTRIES {
            let e = &b[8 + i * 12..20 + i * 12];
            if i >= usize::from(value.count) {
                if e.iter().any(|v| *v != 0) {
                    return Err(Code::InvalidRequest);
                }
                continue;
            }
            let entry = SecurityEntry {
                principal: e[0],
                deny: e[1] == 1,
                access: u16::from_le_bytes([e[2], e[3]]),
                id: u32::from_le_bytes(e[4..8].try_into().unwrap()),
                inherit: e[8] == 1,
            };
            let valid_id = match entry.principal {
                principal::EVERYONE => entry.id == 0,
                principal::USER => entry.id != 0,
                principal::GROUP => entry.id < 32,
                _ => false,
            };
            if !valid_id
                || e[1] > 1
                || e[8] > 1
                || e[9..].iter().any(|v| *v != 0)
                || entry.access & !access::ALL != 0
            {
                return Err(Code::InvalidRequest);
            }
            value.entries[i] = entry;
        }
        Ok(value)
    }
}
/// Volume timestamps (FILETIME: 100 ns since 1601-01-01 UTC); 0 = not
/// recorded (Explorer shows blank). Accessed is not updated by reads.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FileTimes {
    pub created: u64,
    pub modified: u64,
    pub accessed: u64,
}
impl FileTimes {
    pub const NONE: Self = Self {
        created: 0,
        modified: 0,
        accessed: 0,
    };
    pub fn encode(self) -> [u8; TIMES_BYTES] {
        let mut b = [0; TIMES_BYTES];
        b[..8].copy_from_slice(&self.created.to_le_bytes());
        b[8..16].copy_from_slice(&self.modified.to_le_bytes());
        b[16..].copy_from_slice(&self.accessed.to_le_bytes());
        b
    }
    /// Exact length; every value within the signed FILETIME range.
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() != TIMES_BYTES {
            return Err(Code::InvalidRequest);
        }
        let t = |at: usize| u64::from_le_bytes(b[at..at + 8].try_into().unwrap());
        let times = Self {
            created: t(0),
            modified: t(8),
            accessed: t(16),
        };
        if [times.created, times.modified, times.accessed]
            .iter()
            .any(|v| *v > i64::MAX as u64)
        {
            return Err(Code::InvalidRequest);
        }
        Ok(times)
    }
}
/// A STAT_BYTES reply.
pub fn encode_stat(info: FileInfo, times: FileTimes) -> [u8; STAT_BYTES] {
    let mut b = [0; STAT_BYTES];
    b[..INFO_BYTES].copy_from_slice(&info.encode());
    b[INFO_BYTES..].copy_from_slice(&times.encode());
    b
}
pub fn decode_stat(b: &[u8]) -> Result<(FileInfo, FileTimes), Code> {
    if b.len() != STAT_BYTES {
        return Err(Code::InvalidRequest);
    }
    Ok((
        FileInfo::decode(&b[..INFO_BYTES])?,
        FileTimes::decode(&b[INFO_BYTES..])?,
    ))
}
/// LIST reply: `FileInfo`, `FileTimes`, u8 name length (1..=48), zero, then
/// the lossless UTF-16 name units.
pub const LIST_NAME_AT: usize = STAT_BYTES + 2;
/// Name units a LIST reply carries; byte STAT_BYTES holds the whole
/// name's length, and NAME_PART replies carry the rest.
pub const LIST_NAME_UNITS: usize = (crate::wire::BODY_BYTES - LIST_NAME_AT) / 2;
/// Name units a NAME_PART reply carries.
pub const NAME_PART_UNITS: usize = crate::wire::BODY_BYTES / 2;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirEntry {
    pub cookie: u64,
    pub name: [u16; NAME_UNITS],
    pub name_len: usize,
    pub info: FileInfo,
    pub times: FileTimes,
}

// ---- P14 file operations, Recycle Bin and change notification ----

/// What a file operation does (FILEOP_BEGIN byte 0).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FileOpMode {
    Copy = 0,
    Move = 1,
    /// Permanent deletion (Shift+Delete, Recycle Bin purge/empty).
    Delete = 2,
}
/// What to do when a destination name exists (FILEOP_BEGIN byte 1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ConflictPolicy {
    Ask = 0,
    Replace = 1,
    Skip = 2,
    /// The incoming item becomes "name (2).ext".
    KeepBoth = 3,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Decision {
    Replace = 1,
    Skip = 2,
    KeepBoth = 3,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum OpState {
    /// Counting items and bytes.
    Scanning = 0,
    Running = 1,
    Paused = 2,
    /// Waiting for `fileop_resolve` about the current item.
    Conflict = 3,
    Done = 4,
    Cancelled = 5,
    Failed = 6,
}
impl OpState {
    pub fn decode(value: u8) -> Result<Self, Code> {
        Ok(match value {
            0 => Self::Scanning,
            1 => Self::Running,
            2 => Self::Paused,
            3 => Self::Conflict,
            4 => Self::Done,
            5 => Self::Cancelled,
            6 => Self::Failed,
            _ => return Err(Code::InvalidRequest),
        })
    }
    pub fn finished(self) -> bool {
        matches!(self, Self::Done | Self::Cancelled | Self::Failed)
    }
}
/// FILEOP_CONTROL actions.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum OpControl {
    Pause = 0,
    Resume = 1,
    Cancel = 2,
    Dismiss = 3,
}
/// The pending conflict: the incoming item and the existing destination.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConflictInfo {
    pub source_bytes: u64,
    pub dest_bytes: u64,
    /// FILETIME; 0 = not recorded.
    pub source_modified: u64,
    pub dest_modified: u64,
    pub directory: bool,
}
pub const OP_NAME_BYTES: usize = 144;
pub const PROGRESS_BYTES: usize = 81 + OP_NAME_BYTES;
/// FILEOP_STEP/QUERY/CONTROL/LIST reply (`PROGRESS_BYTES` at most):
/// `[op u64][mode u8][state u8][last error Code u8 (0 none)][flags u8:
/// bit0 conflict, bit1 conflicting source is a folder][items total u32]
/// [items done u32][skipped u32][failures u32][reserved u32][bytes total
/// u64][bytes done u64][conflict source bytes, dest bytes, source modified,
/// dest modified: u64 ×4][name length u8][current name, UTF-8]`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpProgress {
    pub op: u64,
    pub mode: FileOpMode,
    pub state: OpState,
    pub items_total: u32,
    pub items_done: u32,
    pub skipped: u32,
    pub failures: u32,
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub last_error: Option<Code>,
    pub conflict: Option<ConflictInfo>,
    name: [u8; OP_NAME_BYTES],
    name_len: u8,
}
impl OpProgress {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        op: u64,
        mode: FileOpMode,
        state: OpState,
        items: (u32, u32),
        skipped: u32,
        failures: u32,
        bytes: (u64, u64),
        last_error: Option<Code>,
        conflict: Option<ConflictInfo>,
        current: &str,
    ) -> Self {
        let mut name = [0; OP_NAME_BYTES];
        let mut len = current.len().min(OP_NAME_BYTES);
        while !current.is_char_boundary(len) {
            len -= 1;
        }
        name[..len].copy_from_slice(&current.as_bytes()[..len]);
        Self {
            op,
            mode,
            state,
            items_total: items.0,
            items_done: items.1,
            skipped,
            failures,
            bytes_total: bytes.0,
            bytes_done: bytes.1,
            last_error,
            conflict,
            name,
            name_len: len as u8,
        }
    }
    /// The item being processed (or the conflicting item).
    pub fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len as usize]).unwrap_or("")
    }
    pub fn encode(&self, out: &mut [u8; PROGRESS_BYTES]) -> usize {
        out.fill(0);
        out[..8].copy_from_slice(&self.op.to_le_bytes());
        out[8] = self.mode as u8;
        out[9] = self.state as u8;
        out[10] = self.last_error.map_or(0, |c| c as u8);
        if let Some(c) = self.conflict {
            out[11] = 1 | (u8::from(c.directory) << 1);
            for (i, v) in [
                c.source_bytes,
                c.dest_bytes,
                c.source_modified,
                c.dest_modified,
            ]
            .into_iter()
            .enumerate()
            {
                out[48 + i * 8..56 + i * 8].copy_from_slice(&v.to_le_bytes());
            }
        }
        for (i, v) in [
            self.items_total,
            self.items_done,
            self.skipped,
            self.failures,
        ]
        .into_iter()
        .enumerate()
        {
            out[12 + i * 4..16 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        out[32..40].copy_from_slice(&self.bytes_total.to_le_bytes());
        out[40..48].copy_from_slice(&self.bytes_done.to_le_bytes());
        out[80] = self.name_len;
        out[81..81 + self.name_len as usize].copy_from_slice(&self.name[..self.name_len as usize]);
        81 + self.name_len as usize
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() < 81 || b.len() > PROGRESS_BYTES || b.len() != 81 + b[80] as usize {
            return Err(Code::InvalidRequest);
        }
        let u32at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        let u64at = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
        let mode = match b[8] {
            0 => FileOpMode::Copy,
            1 => FileOpMode::Move,
            2 => FileOpMode::Delete,
            _ => return Err(Code::InvalidRequest),
        };
        let state = OpState::decode(b[9])?;
        let last_error = match b[10] {
            0 => None,
            c => Some(Code::decode(u32::from(c)).map_err(|_| Code::InvalidRequest)?),
        };
        if b[11] & !3 != 0 || u32at(28) != 0 || (b[11] & 1 == 1) != (state == OpState::Conflict) {
            return Err(Code::InvalidRequest);
        }
        let conflict = (b[11] & 1 == 1).then(|| ConflictInfo {
            source_bytes: u64at(48),
            dest_bytes: u64at(56),
            source_modified: u64at(64),
            dest_modified: u64at(72),
            directory: b[11] & 2 != 0,
        });
        if conflict.is_none() && b[48..80].iter().any(|v| *v != 0) {
            return Err(Code::InvalidRequest);
        }
        let name = core::str::from_utf8(&b[81..]).map_err(|_| Code::InvalidRequest)?;
        let op = u64at(0);
        if op == 0 {
            return Err(Code::InvalidRequest);
        }
        Ok(Self::new(
            op,
            mode,
            state,
            (u32at(12), u32at(16)),
            u32at(20),
            u32at(24),
            (u64at(32), u64at(40)),
            last_error,
            conflict,
            name,
        ))
    }
}
/// RECYCLE reply: `[kind u8: 0 recycled, 1 too big][id u64]`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecycleOutcome {
    Recycled(u64),
    /// Larger than the bin's limit: ask before deleting permanently.
    TooBig,
}
impl RecycleOutcome {
    pub fn encode(self) -> [u8; 9] {
        let mut out = [0; 9];
        match self {
            Self::Recycled(id) => out[1..].copy_from_slice(&id.to_le_bytes()),
            Self::TooBig => out[0] = 1,
        }
        out
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        let id = u64::from_le_bytes(b.get(1..9).ok_or(Code::InvalidRequest)?.try_into().unwrap());
        match (b.len(), b[0], id) {
            (9, 0, id) if id != 0 => Ok(Self::Recycled(id)),
            (9, 1, 0) => Ok(Self::TooBig),
            _ => Err(Code::InvalidRequest),
        }
    }
}
pub const RECYCLE_PATH_BYTES: usize = crate::BODY_BYTES - 32;
/// RECYCLE_LIST reply: `[id u64][deleted FILETIME u64][bytes u64][items
/// u32][flags u8: bit0 folder, bit1 path truncated at the front][reserved
/// u8][path length u16][original path, UTF-8]`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecycleEntry {
    pub id: u64,
    pub deleted: u64,
    pub bytes: u64,
    pub items: u32,
    pub directory: bool,
    /// Only the end of a very long original path is carried.
    pub truncated: bool,
    path: [u8; RECYCLE_PATH_BYTES],
    path_len: u16,
}
impl RecycleEntry {
    pub fn new(id: u64, deleted: u64, bytes: u64, items: u32, directory: bool, path: &str) -> Self {
        let mut start = path.len().saturating_sub(RECYCLE_PATH_BYTES);
        while !path.is_char_boundary(start) {
            start += 1;
        }
        let tail = &path.as_bytes()[start..];
        let mut buf = [0; RECYCLE_PATH_BYTES];
        buf[..tail.len()].copy_from_slice(tail);
        Self {
            id,
            deleted,
            bytes,
            items,
            directory,
            truncated: start != 0,
            path: buf,
            path_len: tail.len() as u16,
        }
    }
    /// Original absolute path (its tail when `truncated`).
    pub fn original(&self) -> &str {
        core::str::from_utf8(&self.path[..self.path_len as usize]).unwrap_or("")
    }
    /// Original name.
    pub fn name(&self) -> &str {
        self.original().rsplit('/').next().unwrap_or("")
    }
    /// Original folder ("Original Location").
    pub fn folder(&self) -> &str {
        let p = self.original();
        match p.rfind('/') {
            Some(0) => "/",
            Some(at) => &p[..at],
            None => "",
        }
    }
    pub fn encode(&self, out: &mut [u8; crate::BODY_BYTES]) -> usize {
        out.fill(0);
        out[..8].copy_from_slice(&self.id.to_le_bytes());
        out[8..16].copy_from_slice(&self.deleted.to_le_bytes());
        out[16..24].copy_from_slice(&self.bytes.to_le_bytes());
        out[24..28].copy_from_slice(&self.items.to_le_bytes());
        out[28] = u8::from(self.directory) | (u8::from(self.truncated) << 1);
        out[30..32].copy_from_slice(&self.path_len.to_le_bytes());
        let n = self.path_len as usize;
        out[32..32 + n].copy_from_slice(&self.path[..n]);
        32 + n
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() < 33 || b.len() > crate::BODY_BYTES {
            return Err(Code::InvalidRequest);
        }
        let n = u16::from_le_bytes([b[30], b[31]]) as usize;
        let id = u64::from_le_bytes(b[..8].try_into().unwrap());
        if b.len() != 32 + n || b[28] & !3 != 0 || b[29] != 0 || id == 0 {
            return Err(Code::InvalidRequest);
        }
        let path = core::str::from_utf8(&b[32..]).map_err(|_| Code::InvalidRequest)?;
        let mut entry = Self::new(
            id,
            u64::from_le_bytes(b[8..16].try_into().unwrap()),
            u64::from_le_bytes(b[16..24].try_into().unwrap()),
            u32::from_le_bytes(b[24..28].try_into().unwrap()),
            b[28] & 1 != 0,
            path,
        );
        entry.truncated = b[28] & 2 != 0;
        Ok(entry)
    }
}
/// RECYCLE_USAGE reply: `[items u32][reserved u32][bytes u64][limit u64]`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RecycleUsage {
    pub items: u32,
    pub bytes: u64,
    pub limit: u64,
}
impl RecycleUsage {
    pub fn encode(self) -> [u8; 24] {
        let mut out = [0; 24];
        out[..4].copy_from_slice(&self.items.to_le_bytes());
        out[8..16].copy_from_slice(&self.bytes.to_le_bytes());
        out[16..].copy_from_slice(&self.limit.to_le_bytes());
        out
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() != 24 || b[4..8] != [0; 4] {
            return Err(Code::InvalidRequest);
        }
        Ok(Self {
            items: u32::from_le_bytes(b[..4].try_into().unwrap()),
            bytes: u64::from_le_bytes(b[8..16].try_into().unwrap()),
            limit: u64::from_le_bytes(b[16..].try_into().unwrap()),
        })
    }
}
/// WATCH body byte 0: what to observe.
pub const WATCH_FOLDER: u8 = 0;
pub const WATCH_SETTINGS: u8 = 1;
/// WATCH_POLL reply: `[sequence u64][flags u8: bit0 changed since the last
/// poll, bit1 overflow (rescan), bit2 the watched folder is gone]`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WatchState {
    pub sequence: u64,
    pub changed: bool,
    pub overflow: bool,
    pub gone: bool,
}
impl WatchState {
    pub fn encode(self) -> [u8; 9] {
        let mut out = [0; 9];
        out[..8].copy_from_slice(&self.sequence.to_le_bytes());
        out[8] = u8::from(self.changed) | u8::from(self.overflow) << 1 | u8::from(self.gone) << 2;
        out
    }
    pub fn decode(b: &[u8]) -> Result<Self, Code> {
        if b.len() != 9 || b[8] & !7 != 0 {
            return Err(Code::InvalidRequest);
        }
        Ok(Self {
            sequence: u64::from_le_bytes(b[..8].try_into().unwrap()),
            changed: b[8] & 1 != 0,
            overflow: b[8] & 2 != 0,
            gone: b[8] & 4 != 0,
        })
    }
}
/// Bytes per READ_SECTION / WRITE_SECTION request: a 64 KiB file read
/// takes one request instead of 342 READ messages, and a request holds the
/// single-threaded service briefly.
pub const SECTION_IO_BYTES: usize = 256 * 1024;
pub const SECTION_IO_REQUEST_BYTES: usize = 28;
/// READ_SECTION / WRITE_SECTION body: `[file u64][offset u64]
/// [section_offset u64][bytes u32]`: file bytes `offset..offset+bytes` and
/// section bytes `section_offset..` of the transferred section.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SectionIo {
    pub file: u64,
    pub offset: u64,
    pub section_offset: u64,
    pub bytes: u32,
}
impl SectionIo {
    pub fn new(file: u64, offset: u64, section_offset: u64, bytes: usize) -> Result<Self, Code> {
        if file == 0
            || bytes == 0
            || bytes > SECTION_IO_BYTES
            || offset.checked_add(bytes as u64).is_none()
            || section_offset.checked_add(bytes as u64).is_none()
        {
            return Err(Code::InvalidRequest);
        }
        Ok(Self {
            file,
            offset,
            section_offset,
            bytes: bytes as u32,
        })
    }
    pub fn encode(self) -> [u8; SECTION_IO_REQUEST_BYTES] {
        let mut out = [0; SECTION_IO_REQUEST_BYTES];
        out[..8].copy_from_slice(&self.file.to_le_bytes());
        out[8..16].copy_from_slice(&self.offset.to_le_bytes());
        out[16..24].copy_from_slice(&self.section_offset.to_le_bytes());
        out[24..].copy_from_slice(&self.bytes.to_le_bytes());
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Code> {
        if bytes.len() != SECTION_IO_REQUEST_BYTES {
            return Err(Code::InvalidRequest);
        }
        let field = |i: usize| u64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().unwrap());
        Self::new(
            field(0),
            field(1),
            field(2),
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()) as usize,
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn section_io_requests_round_trip_and_stay_bounded() {
        let r = SectionIo::new(7, 1 << 40, 4096, SECTION_IO_BYTES).unwrap();
        assert_eq!(SectionIo::decode(&r.encode()), Ok(r));
        assert!(SectionIo::new(0, 0, 0, 1).is_err());
        assert!(SectionIo::new(7, 0, 0, 0).is_err());
        assert!(SectionIo::new(7, 0, 0, SECTION_IO_BYTES + 1).is_err());
        assert!(SectionIo::new(7, u64::MAX, 0, 1).is_err());
        assert!(SectionIo::decode(&[0; 27]).is_err());
    }
    #[test]
    fn security_records_round_trip_and_reject_malformed_entries() {
        let mut s = Security {
            owner: 1001,
            integrity: 1,
            directory: true,
            count: 2,
            ..Security::default()
        };
        s.entries[0] = SecurityEntry {
            principal: principal::USER,
            id: 1001,
            deny: false,
            access: access::ALL,
            inherit: true,
        };
        s.entries[1] = SecurityEntry {
            principal: principal::GROUP,
            id: 3,
            deny: true,
            access: access::WRITE,
            inherit: false,
        };
        let b = s.encode();
        assert_eq!(Security::decode(&b), Ok(s));
        assert_eq!(s.entries().len(), 2);
        for (at, value) in [(4, 3), (5, 5), (6, 2), (8, 7), (8 + 12, 3), (8 + 24, 1)] {
            let mut bad = b;
            bad[at] = value;
            assert!(Security::decode(&bad).is_err(), "{at}");
        }
        let mut nobody = b;
        nobody[..4].fill(0);
        assert!(Security::decode(&nobody).is_err());
        assert!(Security::decode(&b[1..]).is_err());
    }
    #[test]
    fn file_operation_progress_round_trips_and_rejects_inconsistent_replies() {
        let conflict = ConflictInfo {
            source_bytes: 5,
            dest_bytes: 3,
            source_modified: 133_000_000_000_000_000,
            dest_modified: 1,
            directory: false,
        };
        let p = OpProgress::new(
            9,
            FileOpMode::Copy,
            OpState::Conflict,
            (5, 2),
            1,
            0,
            (150_010, 4096),
            None,
            Some(conflict),
            "a (2).txt",
        );
        let mut b = [0; PROGRESS_BYTES];
        let n = p.encode(&mut b);
        assert_eq!(OpProgress::decode(&b[..n]), Ok(p));
        assert_eq!(OpProgress::decode(&b[..n]).unwrap().name(), "a (2).txt");
        for cut in 0..n {
            assert!(OpProgress::decode(&b[..cut]).is_err());
        }
        // A conflict flag must agree with the state; unknown enums fail.
        for (at, value) in [(9, 1u8), (8, 3), (9, 7), (11, 4), (10, 99)] {
            let mut bad = b;
            bad[at] = value;
            assert!(OpProgress::decode(&bad[..n]).is_err(), "byte {at}");
        }
        let done = OpProgress::new(
            9,
            FileOpMode::Delete,
            OpState::Failed,
            (5, 5),
            0,
            1,
            (10, 10),
            Some(Code::Resources),
            None,
            "",
        );
        let n = done.encode(&mut b);
        assert_eq!(OpProgress::decode(&b[..n]), Ok(done));
        let mut stale = b;
        stale[60] = 1; // conflict bytes without a conflict
        assert!(OpProgress::decode(&stale[..n]).is_err());
        // Long names are cut on a character boundary.
        let long = "é".repeat(100);
        let p = OpProgress::new(
            1,
            FileOpMode::Move,
            OpState::Running,
            (0, 0),
            0,
            0,
            (0, 0),
            None,
            None,
            &long,
        );
        assert_eq!(p.name().len(), 144);
    }
    #[test]
    fn recycle_and_watch_replies_are_exact() {
        extern crate std;
        for o in [RecycleOutcome::Recycled(7), RecycleOutcome::TooBig] {
            assert_eq!(RecycleOutcome::decode(&o.encode()), Ok(o));
        }
        assert!(RecycleOutcome::decode(&[0; 9]).is_err());
        assert!(RecycleOutcome::decode(&[1, 1, 0, 0, 0, 0, 0, 0, 0]).is_err());
        let e = RecycleEntry::new(3, 133, 9005, 4, true, "/users/1001/src/tree");
        let mut b = [0; crate::BODY_BYTES];
        let n = e.encode(&mut b);
        let d = RecycleEntry::decode(&b[..n]).unwrap();
        assert_eq!(d, e);
        assert_eq!(
            (d.name(), d.folder(), d.truncated),
            ("tree", "/users/1001/src", false)
        );
        assert!(RecycleEntry::decode(&b[..n - 1]).is_err());
        let long = std::format!("/users/1001/{}", "d/".repeat(150));
        let t = RecycleEntry::new(3, 0, 0, 1, false, &long);
        assert!(t.truncated && long.ends_with(t.original()));
        let n = t.encode(&mut b);
        assert_eq!(RecycleEntry::decode(&b[..n]), Ok(t));
        let u = RecycleUsage {
            items: 2,
            bytes: 9,
            limit: 1 << 20,
        };
        assert_eq!(RecycleUsage::decode(&u.encode()), Ok(u));
        let w = WatchState {
            sequence: 42,
            changed: true,
            overflow: false,
            gone: true,
        };
        assert_eq!(WatchState::decode(&w.encode()), Ok(w));
        let mut bad = w.encode();
        bad[8] = 8;
        assert!(WatchState::decode(&bad).is_err());
    }
    #[test]
    fn permanent_remove_uses_exact_observed_binding_and_rejects_malformed_requests() {
        let r = RemoveChild::new(0x0807060504030201, 27, 42).unwrap();
        let bytes = r.encode();
        assert_eq!(&bytes[..8], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(RemoveChild::decode(&bytes), Ok(r));
        for n in 0..REMOVE_CHILD_BYTES {
            assert!(RemoveChild::decode(&bytes[..n]).is_err());
        }
        let mut extra = bytes.to_vec();
        extra.push(0);
        assert!(RemoveChild::decode(&extra).is_err());
        for offset in [0, 8, 16] {
            let mut bad = bytes;
            bad[offset..offset + 8].fill(0);
            assert!(RemoveChild::decode(&bad).is_err());
        }
    }
    #[test]
    fn replacement_begin_has_exact_bounded_little_endian_identity() {
        for length in [0, 1, REPLACE_BYTES] {
            let request = BeginReplace::new(0x0807060504030201, 27, 42, length).unwrap();
            let encoded = request.encode();
            assert_eq!(&encoded[..8], &[1, 2, 3, 4, 5, 6, 7, 8]);
            assert_eq!(BeginReplace::decode(&encoded), Ok(request));
            for n in 0..encoded.len() {
                assert!(BeginReplace::decode(&encoded[..n]).is_err());
            }
            let mut long = encoded.to_vec();
            long.push(0);
            assert!(BeginReplace::decode(&long).is_err());
            for offset in [0, 8, 16] {
                let mut bad = encoded;
                bad[offset..offset + 8].fill(0);
                assert!(BeginReplace::decode(&bad).is_err());
            }
            for len in [REPLACE_BYTES as u64 + 1, u64::MAX] {
                let mut bad = encoded;
                bad[24..].copy_from_slice(&len.to_le_bytes());
                assert!(BeginReplace::decode(&bad).is_err());
            }
        }
    }
    #[test]
    fn section_stage_writes_are_bounded_and_cut_round_trips() {
        let r = StageSection::new(9, 11, 4096, 0, STAGE_SECTION_BYTES).unwrap();
        let encoded = r.encode();
        assert_eq!(StageSection::decode(&encoded), Ok(r));
        assert_eq!(&encoded[32..], &(STAGE_SECTION_BYTES as u32).to_le_bytes());
        for n in 0..encoded.len() {
            assert!(StageSection::decode(&encoded[..n]).is_err());
        }
        for (file, token, offset, at, bytes) in [
            (0, 11, 0, 0, 1),
            (9, 0, 0, 0, 1),
            (9, 11, 0, 0, 0),
            (9, 11, 0, 0, STAGE_SECTION_BYTES + 1),
            (9, 11, u64::MAX, 0, 1),
            (9, 11, 0, u64::MAX, 1),
        ] {
            assert!(StageSection::new(file, token, offset, at, bytes).is_err());
        }
        // A whole message body carries candidate bytes after the header.
        assert_eq!(STAGE_CHUNK_BYTES, 216);
        // The largest staged save: ~22,100 STAGE_WRITE round trips at the old
        // 192-byte chunk, 19,646 at a full body, 65 through sections.
        assert_eq!(
            stage_requests(LARGE_REPLACE_BYTES, crate::CHUNK_BYTES),
            22_102
        );
        assert_eq!(
            stage_requests(LARGE_REPLACE_BYTES, STAGE_CHUNK_BYTES),
            19_646
        );
        assert_eq!(stage_requests(LARGE_REPLACE_BYTES, STAGE_SECTION_BYTES), 65);
        assert_eq!(stage_requests(4 << 20, STAGE_SECTION_BYTES), 64);
    }
    #[test]
    fn staged_begin_shares_the_wire_and_bounds_the_length() {
        let r = BeginReplace::large(9, 27, 3, LARGE_REPLACE_BYTES).unwrap();
        assert_eq!(BeginReplace::decode_large(&r.encode()), Ok(r));
        assert!(
            BeginReplace::decode(&r.encode()).is_err(),
            "over the 4 KiB checked limit"
        );
        assert!(BeginReplace::large(9, 27, 3, LARGE_REPLACE_BYTES + 1).is_err());
        let mut over = r.encode();
        over[24..].copy_from_slice(&(LARGE_REPLACE_BYTES as u64 + 1).to_le_bytes());
        assert!(BeginReplace::decode_large(&over).is_err());
        assert!(BeginReplace::large(0, 27, 3, 1).is_err());
    }
    #[test]
    fn checked_child_rename_has_one_lossless_fixed_wire_identity() {
        let units = [0xd800, 0xdc01, 0xdfff, 46, 120];
        let request = RenameChild::new(0x102030405060708, 27, 42, &units).unwrap();
        let encoded = request.encode();
        assert_eq!(&encoded[..8], &[8, 7, 6, 5, 4, 3, 2, 1]);
        assert_eq!(&encoded[8..16], &27u64.to_le_bytes());
        assert_eq!(&encoded[16..24], &42u64.to_le_bytes());
        assert_eq!(&encoded[24..32], &[5, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&encoded[32..42], &[0, 216, 1, 220, 255, 223, 46, 0, 120, 0]);
        assert_eq!(RenameChild::decode(&encoded), Ok(request));
        assert_eq!(request.name(), units);
        for length in 0..RENAME_CHILD_BYTES {
            assert!(RenameChild::decode(&encoded[..length]).is_err());
        }
        for at in [0, 8, 16] {
            let mut bad = encoded;
            bad[at..at + 8].fill(0);
            assert!(RenameChild::decode(&bad).is_err());
        }
        for at in [26, 31, 42, 127] {
            let mut bad = encoded;
            bad[at] = 1;
            assert!(RenameChild::decode(&bad).is_err());
        }
        for length in [0u16, 49, u16::MAX] {
            let mut bad = encoded;
            bad[24..26].copy_from_slice(&length.to_le_bytes());
            assert!(RenameChild::decode(&bad).is_err());
        }
        assert!(RenameChild::new(1, 27, 1, &[0]).is_err());
        assert!(RenameChild::new(1, 27, 1, &[b'x' as u16; RENAME_NAME_UNITS + 1]).is_err());
        let full = RenameChild::new(1, 27, 1, &[0xffff; RENAME_NAME_UNITS]).unwrap();
        assert_eq!(RenameChild::decode(&full.encode()), Ok(full));
    }
    #[test]
    fn stat_wire_carries_bounded_filetimes() {
        let info = FileInfo {
            attributes: 0,
            bytes: 5,
            object: 27,
            version: 3,
            directory: false,
        };
        let times = FileTimes {
            created: 1,
            modified: 133_000_000_000_000_000,
            accessed: 0,
        };
        let b = encode_stat(info, times);
        assert_eq!(decode_stat(&b), Ok((info, times)));
        assert_eq!(&b[40..48], &133_000_000_000_000_000u64.to_le_bytes());
        assert!(decode_stat(&b[..STAT_BYTES - 1]).is_err());
        assert!(decode_stat(&info.encode()).is_err());
        let mut bad = b;
        bad[STAT_BYTES - 1] = 0x80; // accessed beyond i64::MAX
        assert!(decode_stat(&bad).is_err());
        assert_eq!(
            FileTimes::decode(&FileTimes::NONE.encode()),
            Ok(FileTimes::NONE)
        );
    }
    #[test]
    fn metadata_wire_validates_length_flags_and_reserved_bytes() {
        let info = FileInfo {
            attributes: 0,
            bytes: 4_200_000,
            object: 27,
            version: 9,
            directory: false,
        };
        let mut bytes = info.encode();
        assert_eq!(FileInfo::decode(&bytes), Ok(info));
        for invalid in [
            FileInfo { object: 0, ..info },
            FileInfo { version: 0, ..info },
            FileInfo {
                directory: true,
                ..info
            },
        ] {
            assert!(FileInfo::decode(&invalid.encode()).is_err());
        }
        let folder = FileInfo {
            bytes: 0,
            directory: true,
            ..info
        };
        assert_eq!(FileInfo::decode(&folder.encode()), Ok(folder));
        assert!(FileInfo::decode(&bytes[..31]).is_err());
        bytes[24] = 2;
        assert!(FileInfo::decode(&bytes).is_err());
        bytes[24] = 0;
        bytes[27] = 1;
        assert!(FileInfo::decode(&bytes).is_err());
        let hidden = FileInfo {
            attributes: 0x2,
            ..info
        };
        assert_eq!(FileInfo::decode(&hidden.encode()), Ok(hidden));
    }
}
