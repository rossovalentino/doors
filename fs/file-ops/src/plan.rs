//! The operation plan and its durable recovery record (`DOORFOP1`).
//!
//! Record layout (little endian, at most `RECORD_BYTES`):
//!
//! | Offset | Bytes | Field |
//! | ---: | ---: | --- |
//! | 0 | 8 | ASCII `DOORFOP1` |
//! | 8 | 4 | Version 1 |
//! | 12 | 4 | Record length in bytes (including the trailing CRC) |
//! | 16 | 8 | Operation ID (nonzero) |
//! | 24 | 4 | Owner user ID |
//! | 28 | 1 | Mode (0 copy, 1 move, 2 delete) |
//! | 29 | 1 | Policy (0 ask, 1 replace, 2 skip, 3 keep both) |
//! | 30 | 1 | State (0 scanning .. 6 failed) |
//! | 31 | 1 | Pending decision for the current item (0 none) |
//! | 32 | 4 ×4 | Items total, done, skipped, failed |
//! | 48 | 8 ×2 | Bytes total, done |
//! | 64 | 1 | Last error (0 none) |
//! | 65 | 1 | Depth (0..=16) |
//! | 66 | 1 | Flags: bit0 temp file live, bit1 current item started, bit2 same-folder copy |
//! | 67 | 1 | Source count (1..=32) |
//! | 68 | 2 | Current source index |
//! | 70 | 2 | Reserved zero |
//! | 72 | 8 | Created (FILETIME) |
//! | 80 | 8 | Current file bytes copied into the temporary file |
//! | 88 | 8 | Current item object ID (0 none) |
//! | 96 | 8 ×17 | Listing cursors per depth (object-ID cursor) |
//! | 232 | ... | Strings: source folder, destination folder, relative folder, top-level destination name, current name, current destination name, then each source name; each `u16` length + UTF-8 |
//! | len-4 | 4 | CRC32 of every preceding byte |
//!
//! Decoding validates every field before the engine acts on a record; a
//! record that fails validation is `Corrupt` and is never replayed.
use crate::{
    Decision, Error, MAX_DEPTH, Mode, NAME_BYTES, Name, PATH_BYTES, PathBuf, Policy, Result, State,
    within,
};

pub const RECORD_BYTES: usize = 8192;
pub const MAX_SOURCES: usize = 32;
const MAGIC: &[u8; 8] = b"DOORFOP1";
const VERSION: u32 = 1;
const STRINGS_AT: usize = 232;
/// Source and destination folder paths are bounded by the request size.
pub const FOLDER_BYTES: usize = 192;

pub const FLAG_TEMP: u8 = 1;
pub const FLAG_STARTED: u8 = 2;
pub const FLAG_SAME_FOLDER: u8 = 4;

#[derive(Clone)]
pub struct Plan {
    pub op: u64,
    pub owner: u32,
    pub mode: Mode,
    pub policy: Policy,
    pub state: State,
    pub decision: Option<Decision>,
    pub items_total: u32,
    pub items_done: u32,
    pub skipped: u32,
    pub failures: u32,
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub last_error: Option<Error>,
    pub depth: u8,
    pub flags: u8,
    pub source_index: u16,
    pub created: u64,
    pub file_offset: u64,
    pub current_object: u64,
    pub cursors: [u64; MAX_DEPTH + 1],
    pub source_root: PathBuf<FOLDER_BYTES>,
    pub dest_root: PathBuf<FOLDER_BYTES>,
    /// Folder being walked, relative to `source_root` (top-level name first).
    pub relative: PathBuf<PATH_BYTES>,
    /// Destination name chosen for the current top-level item.
    pub top_name: Name,
    /// Item being processed (display and resume identity).
    pub current: Name,
    /// Destination name chosen for the current item.
    pub target: Name,
    pub sources: [Name; MAX_SOURCES],
    pub source_count: u8,
}
impl Plan {
    pub const fn new(op: u64, owner: u32, mode: Mode, policy: Policy, created: u64) -> Self {
        Self {
            op,
            owner,
            mode,
            policy,
            state: State::Scanning,
            decision: None,
            items_total: 0,
            items_done: 0,
            skipped: 0,
            failures: 0,
            bytes_total: 0,
            bytes_done: 0,
            last_error: None,
            depth: 0,
            flags: 0,
            source_index: 0,
            created,
            file_offset: 0,
            current_object: 0,
            cursors: [0; MAX_DEPTH + 1],
            source_root: PathBuf::new(),
            dest_root: PathBuf::new(),
            relative: PathBuf::new(),
            top_name: Name::EMPTY,
            current: Name::EMPTY,
            target: Name::EMPTY,
            sources: [Name::EMPTY; MAX_SOURCES],
            source_count: 0,
        }
    }
    pub fn sources(&self) -> &[Name] {
        &self.sources[..self.source_count as usize]
    }
    pub fn flag(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }
    pub fn set_flag(&mut self, flag: u8, on: bool) {
        if on {
            self.flags |= flag;
        } else {
            self.flags &= !flag;
        }
    }
    pub fn encode(&self, out: &mut [u8; RECORD_BYTES]) -> Result<usize> {
        out.fill(0);
        out[..8].copy_from_slice(MAGIC);
        put32(out, 8, VERSION);
        put64(out, 16, self.op);
        put32(out, 24, self.owner);
        out[28] = self.mode as u8;
        out[29] = self.policy as u8;
        out[30] = self.state as u8;
        out[31] = self.decision.map_or(0, |d| d as u8);
        put32(out, 32, self.items_total);
        put32(out, 36, self.items_done);
        put32(out, 40, self.skipped);
        put32(out, 44, self.failures);
        put64(out, 48, self.bytes_total);
        put64(out, 56, self.bytes_done);
        out[64] = self.last_error.map_or(0, |e| e as u8);
        out[65] = self.depth;
        out[66] = self.flags;
        out[67] = self.source_count;
        out[68..70].copy_from_slice(&self.source_index.to_le_bytes());
        put64(out, 72, self.created);
        put64(out, 80, self.file_offset);
        put64(out, 88, self.current_object);
        for (i, c) in self.cursors.iter().enumerate() {
            put64(out, 96 + i * 8, *c);
        }
        let mut at = STRINGS_AT;
        let mut put = |text: &str| -> Result<()> {
            if at + 2 + text.len() + 4 > RECORD_BYTES {
                return Err(Error::Invalid);
            }
            out[at..at + 2].copy_from_slice(&(text.len() as u16).to_le_bytes());
            out[at + 2..at + 2 + text.len()].copy_from_slice(text.as_bytes());
            at += 2 + text.len();
            Ok(())
        };
        put(self.source_root.as_str())?;
        put(self.dest_root.as_str())?;
        put(self.relative.as_str())?;
        put(self.top_name.as_str())?;
        put(self.current.as_str())?;
        put(self.target.as_str())?;
        for s in self.sources() {
            put(s.as_str())?;
        }
        let len = at + 4;
        put32(out, 12, len as u32);
        let crc = crc32(&out[..at]);
        put32(out, at, crc);
        Ok(len)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < STRINGS_AT + 4 || bytes.len() > RECORD_BYTES || &bytes[..8] != MAGIC {
            return Err(Error::Corrupt);
        }
        if get32(bytes, 8) != VERSION || get32(bytes, 12) as usize != bytes.len() {
            return Err(Error::Corrupt);
        }
        let body = bytes.len() - 4;
        if crc32(&bytes[..body]) != get32(bytes, body) {
            return Err(Error::Corrupt);
        }
        let mode = Mode::decode(bytes[28]).ok_or(Error::Corrupt)?;
        let policy = Policy::decode(bytes[29]).ok_or(Error::Corrupt)?;
        let state = State::decode(bytes[30]).ok_or(Error::Corrupt)?;
        let decision = match bytes[31] {
            0 => None,
            d => Some(Decision::decode(d).ok_or(Error::Corrupt)?),
        };
        let last_error = match bytes[64] {
            0 => None,
            e => Some(Error::decode(e).ok_or(Error::Corrupt)?),
        };
        let op = get64(bytes, 16);
        let depth = bytes[65];
        let flags = bytes[66];
        let source_count = bytes[67];
        let source_index = u16::from_le_bytes([bytes[68], bytes[69]]);
        if op == 0
            || depth as usize > MAX_DEPTH
            || flags & !7 != 0
            || source_count == 0 && (state != State::Paused || flags & FLAG_STARTED != 0)
            || source_count as usize > MAX_SOURCES
            || source_index > u16::from(source_count)
            || bytes[70..72] != [0, 0]
        {
            return Err(Error::Corrupt);
        }
        let mut plan = Self::new(op, get32(bytes, 24), mode, policy, get64(bytes, 72));
        plan.state = state;
        plan.decision = decision;
        plan.items_total = get32(bytes, 32);
        plan.items_done = get32(bytes, 36);
        plan.skipped = get32(bytes, 40);
        plan.failures = get32(bytes, 44);
        plan.bytes_total = get64(bytes, 48);
        plan.bytes_done = get64(bytes, 56);
        plan.last_error = last_error;
        plan.depth = depth;
        plan.flags = flags;
        plan.source_index = source_index;
        plan.file_offset = get64(bytes, 80);
        plan.current_object = get64(bytes, 88);
        for (i, c) in plan.cursors.iter_mut().enumerate() {
            *c = get64(bytes, 96 + i * 8);
        }
        let mut at = STRINGS_AT;
        let mut take = || -> Result<&str> {
            if at + 2 > body {
                return Err(Error::Corrupt);
            }
            let len = u16::from_le_bytes([bytes[at], bytes[at + 1]]) as usize;
            if at + 2 + len > body {
                return Err(Error::Corrupt);
            }
            let text =
                core::str::from_utf8(&bytes[at + 2..at + 2 + len]).map_err(|_| Error::Corrupt)?;
            at += 2 + len;
            Ok(text)
        };
        plan.source_root.set(take()?).map_err(|_| Error::Corrupt)?;
        plan.dest_root.set(take()?).map_err(|_| Error::Corrupt)?;
        plan.relative.set(take()?).map_err(|_| Error::Corrupt)?;
        plan.top_name = optional_name(take()?)?;
        plan.current = optional_name(take()?)?;
        plan.target = optional_name(take()?)?;
        for i in 0..source_count as usize {
            plan.sources[i] = Name::new(take()?).map_err(|_| Error::Corrupt)?;
        }
        plan.source_count = source_count;
        if at != body
            || (source_count != 0 && !plan.source_root.as_str().starts_with('/'))
            || (mode != Mode::Delete && !plan.dest_root.as_str().starts_with('/'))
            || plan.items_done > plan.items_total && state != State::Scanning
        {
            return Err(Error::Corrupt);
        }
        // A copy/move never targets its own subtree (rechecked here so a
        // damaged record cannot start one).
        if mode != Mode::Delete {
            for s in plan.sources() {
                let src = crate::join(plan.source_root.as_str(), s.as_str())?;
                if within(plan.dest_root.as_str(), src.as_str()) {
                    return Err(Error::Corrupt);
                }
            }
        }
        Ok(plan)
    }
}
fn optional_name(text: &str) -> Result<Name> {
    if text.is_empty() {
        Ok(Name::EMPTY)
    } else {
        Name::new(text).map_err(|_| Error::Corrupt)
    }
}
fn put32(out: &mut [u8], at: usize, v: u32) {
    out[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn put64(out: &mut [u8], at: usize, v: u64) {
    out[at..at + 8].copy_from_slice(&v.to_le_bytes());
}
fn get32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn get64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
/// IEEE CRC32 (the journal's polynomial), bitwise and table-free.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
const _: () = assert!(NAME_BYTES <= u8::MAX as usize);
