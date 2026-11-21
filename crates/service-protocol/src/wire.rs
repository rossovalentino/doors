//! Service payloads ride the authenticated native channel envelope. Integers are
//! little endian; no wire bytes are cast to Rust structs or treated as authority.
use door_abi::{Header, Status};

pub const VERSION: u32 = 1;
pub const PREFIX_BYTES: usize = 16;
pub const BODY_BYTES: usize = door_abi::MAX_PAYLOAD - PREFIX_BYTES;
pub const CHUNK_BYTES: usize = 192;
/// The longest registry value (storage keeps it in chained records).
pub const REGISTRY_VALUE_BYTES: usize = 2048;
/// The longest path a storage request carries (UTF-8 bytes; a Windows path
/// of MAX_PATH units). A request longer than one packet goes as a long
/// request (`opcode::LONG_PART`, `LONG_COMMIT`).
pub const PATH_BYTES: usize = 1024;
/// The longest long request: two paths and their header (RENAME).
pub const LONG_BYTES: usize = 2 * PATH_BYTES + 64;
/// The longest registry key path (UTF-16 units) REGISTRY_OPEN takes (a long
/// request past one packet).
pub const REGISTRY_PATH_UNITS: usize = 512;
pub mod opcode {
    pub const READY: u32 = 0x1000;
    pub const STOP: u32 = 0x1001;
    pub const RESOLVE: u32 = 0x1100;
    pub const REVOCATION_READY: u32 = 0x1200;
    pub const OPEN: u32 = 0x2000;
    pub const READ: u32 = 0x2001;
    pub const WRITE: u32 = 0x2002;
    pub const FLUSH: u32 = 0x2003;
    pub const CLOSE: u32 = 0x2004;
    pub const OPEN_WINDOWS: u32 = 0x2005;
    pub const METADATA: u32 = 0x2006;
    pub const FILE_INFO: u32 = 0x2007;
    pub const LIST: u32 = 0x2008;
    pub const MKDIR: u32 = 0x2009;
    pub const RENAME: u32 = 0x200a;
    pub const REMOVE: u32 = 0x200b;
    pub const TRUNCATE: u32 = 0x200c;
    pub const OPEN_FOR: u32 = 0x200d;
    pub const METADATA_FOR: u32 = 0x200e;
    pub const RENAME_CHILD_CHECKED: u32 = 0x200f;
    pub const GET_SETTING: u32 = 0x2010;
    pub const SET_SETTING: u32 = 0x2011;
    pub const GET_SETTING_FOR: u32 = 0x2012;
    pub const REPLACE_BEGIN: u32 = 0x2013;
    pub const REPLACE_CHUNK: u32 = 0x2014;
    pub const REPLACE_COMMIT: u32 = 0x2015;
    pub const REPLACE_ABORT: u32 = 0x2016;
    pub const REMOVE_CHILD_CHECKED: u32 = 0x2017;
    pub const DISK_INFO: u32 = 0x2018;
    pub const VOLUME_USAGE: u32 = 0x2019;
    /// Trusted Init only: set a setting for a user (uid, setting, value).
    pub const SET_SETTING_FOR: u32 = 0x201a;
    /// Staged whole-file replacement (persistent-namespace-v1): BEGIN is a
    /// 32-byte `BeginReplace::large` and returns a token; WRITE is file
    /// token, staging token, exact offset, 1..=`files::STAGE_CHUNK_BYTES`
    /// bytes; COMMIT and ABORT are the two tokens. COMMIT replies with the
    /// 32-byte FileInfo.
    pub const STAGE_BEGIN: u32 = 0x201b;
    pub const STAGE_WRITE: u32 = 0x201c;
    pub const STAGE_COMMIT: u32 = 0x201d;
    pub const STAGE_ABORT: u32 = 0x201e;
    /// STAGE_WRITE from a section: a `files::StageSection` body plus one
    /// section handle with READ; the service snapshots
    /// 1..=`files::STAGE_SECTION_BYTES` bytes per request.
    pub const STAGE_WRITE_SECTION: u32 = 0x201f;
    pub const REGISTRY_OPEN: u32 = 0x2020;
    pub const REGISTRY_QUERY: u32 = 0x2021;
    pub const REGISTRY_SET: u32 = 0x2022;
    pub const REGISTRY_CLOSE: u32 = 0x2023;
    pub const REGISTRY_VERSION: u32 = 0x2024;
    pub const REGISTRY_ENUM_KEY: u32 = 0x2025;
    pub const REGISTRY_ENUM_VALUE: u32 = 0x2026;
    pub const REGISTRY_SET_CHECKED: u32 = 0x2027;
    /// Delete a value of an open key: key token u64, name units u16, 6 zero
    /// bytes, then the UTF-16 name (Registry Editor > Delete).
    pub const REGISTRY_DELETE_VALUE: u32 = 0x2037;
    /// Delete an open key (it must have no subkeys): key token u64.
    pub const REGISTRY_DELETE_KEY: u32 = 0x2038;
    /// Stage part of a value too long for one REGISTRY_SET body: key token
    /// u64, offset u16, whole length u16, 4 zero bytes, then the bytes. A
    /// REGISTRY_SET(_CHECKED) whose staged length field is non-zero then
    /// stores the staged bytes (all of them, or the request fails).
    pub const REGISTRY_STAGE_VALUE: u32 = 0x2039;
    /// P14 file-operation engine (`files::OpProgress`). BEGIN is mode u8,
    /// policy u8, destination folder path (empty for Delete) and returns the
    /// operation ID; ADD is op u64 + one source path; STEP/QUERY are op u64;
    /// RESOLVE is op u64, decision u8, apply-to-all u8; CONTROL is op u64,
    /// `files::OpControl` u8; LIST is the u64 operation cursor (empty reply
    /// at the end). Operations belong to the authenticated user, survive
    /// reconnects and reboots (interrupted ones come back Paused).
    pub const FILEOP_BEGIN: u32 = 0x2028;
    pub const FILEOP_ADD: u32 = 0x2029;
    pub const FILEOP_STEP: u32 = 0x202a;
    pub const FILEOP_QUERY: u32 = 0x202b;
    pub const FILEOP_RESOLVE: u32 = 0x202c;
    pub const FILEOP_CONTROL: u32 = 0x202d;
    pub const FILEOP_LIST: u32 = 0x202e;
    /// Recycle Bin: RECYCLE takes a path (`files::RecycleOutcome` reply);
    /// LIST the u64 ID cursor (`files::RecycleEntry`, empty at the end);
    /// RESTORE an ID; PURGE an ID (0 = empty the bin) and returns a Delete
    /// operation ID to step (0 = nothing to delete); USAGE `files::RecycleUsage`.
    pub const RECYCLE: u32 = 0x202f;
    pub const RECYCLE_LIST: u32 = 0x2030;
    pub const RECYCLE_RESTORE: u32 = 0x2031;
    pub const RECYCLE_PURGE: u32 = 0x2032;
    pub const RECYCLE_USAGE: u32 = 0x2033;
    /// Change subscriptions: WATCH is kind u8 (`files::WATCH_FOLDER` + folder
    /// path, or `files::WATCH_SETTINGS`) and returns a token bound to the
    /// channel; WATCH_POLL is the token (`files::WatchState` reply); UNWATCH
    /// releases it.
    pub const WATCH: u32 = 0x2034;
    pub const WATCH_POLL: u32 = 0x2035;
    pub const UNWATCH: u32 = 0x2036;
    /// A file's or folder's access control list (`files::Security`), as
    /// Windows' GetNamedSecurityInfo reads it: SECURITY is a path,
    /// FILE_SECURITY an open file's token. Its owner, or a caller the list
    /// grants read, may read it.
    pub const SECURITY: u32 = 0x203a;
    pub const FILE_SECURITY: u32 = 0x203c;
    /// Replace the list: SET_SECURITY is `[Security][path]`,
    /// SET_FILE_SECURITY `[token u64][Security]`. The owner or a caller the
    /// list grants change may; the owner may become only the caller itself.
    pub const SET_SECURITY: u32 = 0x203b;
    pub const SET_FILE_SECURITY: u32 = 0x203d;
    /// A storage request whose body (a long path) does not fit one packet:
    /// LONG_PART `[bytes]` appends to the connection's pending body (at
    /// most `LONG_BYTES`), LONG_COMMIT `[operation u32]` runs that operation
    /// on it as if it had come in one packet and drops it. Path operations
    /// only; the pending body goes with the connection.
    pub const LONG_PART: u32 = 0x203e;
    pub const LONG_COMMIT: u32 = 0x203f;
    /// Bulk file data through a transferred section (P18): a
    /// `files::SectionIo` body plus one section handle. READ_SECTION fills
    /// the section from the file (READ access on the file, WRITE on the
    /// section), WRITE_SECTION writes the file from it; both move up to
    /// `files::SECTION_IO_BYTES` and reply with the bytes moved (a READ
    /// stops early at the end of the file).
    pub const READ_SECTION: u32 = 0x2040;
    pub const WRITE_SECTION: u32 = 0x2041;
    /// The rest of a LIST entry's name (P18: 255-unit names).
    pub const NAME_PART: u32 = 0x2042;
    pub const CONSOLE_WRITE: u32 = 0x3000;
    pub const CONSOLE_READ: u32 = 0x3001;
    pub const CONSOLE_CONTROL: u32 = 0x3002;
    pub const CONSOLE_INTERRUPT: u32 = 0x3003;
    /// Session Terminal (Workspace, terminal mode) only: drain the text its
    /// session's console clients wrote since the last call, so output of a
    /// program launched from Terminal shows in the tab while the kernel still
    /// publishes it as `DOOR_CONSOLE` records.
    pub const CONSOLE_MIRROR: u32 = 0x3004;
    /// Session Terminal only: typed input for the console's programs (key
    /// events as win32-input-mode sequences, or text), queued whole or not
    /// at all; replies with the bytes queued.
    pub const CONSOLE_INPUT: u32 = 0x3005;
    /// The console screen's state (`console::State`); a `console::Command`
    /// body changes it first (the Terminal's grid size, the input mode,
    /// flushing typed input).
    pub const CONSOLE_STATE: u32 = 0x3006;
    /// Typed input not yet read, up to `CHUNK_BYTES` (WouldBlock when none).
    pub const CONSOLE_READ_INPUT: u32 = 0x3007;
    pub const LOGON: u32 = 0x4000;
    pub const LOGOFF: u32 = 0x4001;
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Code {
    Success = 0,
    InvalidRequest = 1,
    Unsupported = 2,
    Denied = 3,
    NotFound = 4,
    Exists = 5,
    Resources = 6,
    InvalidHandle = 7,
    SharingViolation = 8,
    DeletePending = 9,
    Corrupt = 10,
    Io = 11,
    WouldBlock = 12,
    StaleInstance = 13,
    Closing = 14,
    Changed = 15,
}
impl Code {
    pub fn decode(value: u32) -> Result<Self, Status> {
        Ok(match value {
            0 => Self::Success,
            1 => Self::InvalidRequest,
            2 => Self::Unsupported,
            3 => Self::Denied,
            4 => Self::NotFound,
            5 => Self::Exists,
            6 => Self::Resources,
            7 => Self::InvalidHandle,
            8 => Self::SharingViolation,
            9 => Self::DeletePending,
            10 => Self::Corrupt,
            11 => Self::Io,
            12 => Self::WouldBlock,
            13 => Self::StaleInstance,
            14 => Self::Closing,
            15 => Self::Changed,
            _ => return Err(Status::InvalidArgument),
        })
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Frame<'a> {
    pub header: Header,
    pub code: Code,
    pub instance: u64,
    pub body: &'a [u8],
}
impl<'a> Frame<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, Status> {
        let (header, payload) = Header::packet(bytes)?;
        if payload.len() < PREFIX_BYTES {
            return Err(Status::InvalidArgument);
        }
        if u32::from_le_bytes(payload[0..4].try_into().unwrap()) != VERSION {
            return Err(Status::VersionMismatch);
        }
        let code = Code::decode(u32::from_le_bytes(payload[4..8].try_into().unwrap()))?;
        let instance = u64::from_le_bytes(payload[8..16].try_into().unwrap());
        if instance == 0 {
            return Err(Status::InvalidArgument);
        }
        Ok(Self {
            header,
            code,
            instance,
            body: &payload[PREFIX_BYTES..],
        })
    }
    pub fn request(&self, instance: u64, last_request: u64, handles: usize) -> Result<(), Code> {
        if self.code != Code::Success || self.header.handle_count as usize != handles {
            return Err(Code::InvalidRequest);
        }
        if self.instance != instance {
            return Err(Code::StaleInstance);
        }
        if self.header.request_id <= last_request {
            return Err(Code::InvalidRequest);
        }
        Ok(())
    }
}
pub struct Message {
    bytes: [u8; door_abi::MAX_PACKET],
    len: usize,
}
impl Message {
    pub fn new(
        opcode: u32,
        request_id: u64,
        instance: u64,
        code: Code,
        body: &[u8],
        handles: usize,
    ) -> Result<Self, Status> {
        if body.len() > BODY_BYTES || handles > door_abi::MAX_TRANSFERS || instance == 0 {
            return Err(Status::InvalidArgument);
        }
        let mut message = Self {
            bytes: [0; door_abi::MAX_PACKET],
            len: door_abi::HEADER_BYTES + PREFIX_BYTES + body.len(),
        };
        Header {
            opcode,
            flags: 0,
            request_id,
            payload_bytes: (PREFIX_BYTES + body.len()) as u32,
            handle_count: handles as u16,
        }
        .encode(&mut message.bytes[..door_abi::HEADER_BYTES])?;
        let payload = &mut message.bytes[door_abi::HEADER_BYTES..message.len];
        payload[0..4].copy_from_slice(&VERSION.to_le_bytes());
        payload[4..8].copy_from_slice(&(code as u32).to_le_bytes());
        payload[8..16].copy_from_slice(&instance.to_le_bytes());
        payload[PREFIX_BYTES..].copy_from_slice(body);
        Ok(message)
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn version_length_status_and_instance_are_checked_before_dispatch() {
        let message =
            Message::new(opcode::WRITE, 7, 8, Code::Success, &[9; CHUNK_BYTES], 0).unwrap();
        let frame = Frame::decode(message.bytes()).unwrap();
        assert_eq!(frame.body, &[9; CHUNK_BYTES]);
        assert_eq!(frame.request(8, 6, 0), Ok(()));
        assert_eq!(frame.request(8, 7, 0), Err(Code::InvalidRequest));
        assert_eq!(frame.request(9, 6, 0), Err(Code::StaleInstance));
        assert_eq!(frame.request(8, 6, 1), Err(Code::InvalidRequest));
        for length in 0..message.bytes().len() {
            assert!(Frame::decode(&message.bytes()[..length]).is_err());
        }
        for offset in [0, 4, 6, 12, 30, 32, 36] {
            let mut bad = message.bytes;
            bad[offset] ^= 0x80;
            assert!(
                Frame::decode(&bad[..message.len]).is_err(),
                "offset {offset}"
            );
        }
        assert!(Message::new(1, 1, 0, Code::Success, &[], 0).is_err());
        assert!(Message::new(1, 1, 1, Code::Success, &[0; BODY_BYTES + 1], 0).is_err());
    }
}
