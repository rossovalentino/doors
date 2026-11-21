//! Native per-user Recycle Bin.
//!
//! The bin is the hidden folder `/users/<uid>/$Recycle.Bin`, owned by the
//! user through the home folder's inheritable ACL, so no other user can list
//! or restore its items. Recycling one item is a single transaction: an
//! atomic rename of the item (a whole folder tree moves at once) to
//! `$R<id>[.ext]` plus the creation of its info record `$I<id>`:
//!
//! | Offset | Bytes | Field |
//! | ---: | ---: | --- |
//! | 0 | 8 | ASCII `DOORRCY1` |
//! | 8 | 4 | Version 1 |
//! | 12 | 4 | Record length (including CRC) |
//! | 16 | 8 | Recycle ID (nonzero, also in both names) |
//! | 24 | 8 | Deletion time (FILETIME, 0 = unknown) |
//! | 32 | 8 | Bytes (recursive) |
//! | 40 | 4 | Items (recursive, including the item) |
//! | 44 | 1 | Flags: bit0 folder |
//! | 45 | 1 | Reserved zero |
//! | 46 | 2 | Original path length |
//! | 48 | n | Original absolute path, UTF-8 |
//! | 48+n | 4 | CRC32 of the preceding bytes |
//!
//! An `$R` item without its `$I` record (an interrupted purge) is an orphan
//! that listing ignores and emptying removes; a `$I` without its item cannot
//! be restored (`NotFound`) and is removed by purge/empty.
use crate::{
    Error, NAME_BYTES, Name, PATH_BYTES, PathBuf, Result, Volume, crc32, file_name, join,
    parent_of, within,
};

pub const BIN_NAME: &str = "$Recycle.Bin";
pub const RECYCLE_INFO_BYTES: usize = 48 + PATH_BYTES + 4;
const MAGIC: &[u8; 8] = b"DOORRCY1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Recycled(u64),
    /// Larger than the bin's limit: Windows asks to delete permanently.
    TooBig,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Usage {
    pub items: u32,
    pub bytes: u64,
    pub limit: u64,
}
/// Windows' default maximum bin size is a share of the volume; Door uses
/// 10 % with a 1 MiB floor (the development volumes are small).
pub fn default_limit(volume_bytes: u64) -> u64 {
    (volume_bytes / 10).max(1 << 20)
}
pub fn bin_path(uid: u32) -> Result<PathBuf<64>> {
    let mut digits = [0u8; 10];
    let mut v = uid;
    let mut n = 0;
    loop {
        digits[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    let mut path = PathBuf::from("/users/")?;
    let mut text = [0u8; 10];
    for i in 0..n {
        text[i] = digits[n - 1 - i];
    }
    path.push(core::str::from_utf8(&text[..n]).map_err(|_| Error::Invalid)?)?;
    path.push(BIN_NAME)?;
    Ok(path)
}
fn hex_name(prefix: &str, id: u64, ext: &str) -> Result<Name> {
    let mut out = [0u8; NAME_BYTES];
    out[..2].copy_from_slice(prefix.as_bytes());
    for i in 0..12 {
        out[2 + i] = b"0123456789ABCDEF"[((id >> ((11 - i) * 4)) & 15) as usize];
    }
    let mut len = 14;
    if !ext.is_empty() && crate::utf16_len(ext) <= 34 {
        out[len..len + ext.len()].copy_from_slice(ext.as_bytes());
        len += ext.len();
    }
    Name::new(core::str::from_utf8(&out[..len]).map_err(|_| Error::Invalid)?)
}
/// `$R<id>` plus the original extension, like Windows' `$RXXXXXX.ext`.
pub fn item_name(id: u64, original: &str, directory: bool) -> Result<Name> {
    let name = file_name(original);
    let ext = match name.rfind('.') {
        Some(at) if at > 0 && !directory => &name[at..],
        _ => "",
    };
    hex_name("$R", id, ext)
}
pub fn info_name(id: u64) -> Result<Name> {
    hex_name("$I", id, "")
}
fn parse_id(name: &str, prefix: &str) -> Option<u64> {
    let digits = name.strip_prefix(prefix)?.get(..12)?;
    u64::from_str_radix(digits, 16).ok().filter(|id| *id != 0)
}

#[derive(Clone, Copy, Debug)]
pub struct InfoRecord {
    pub id: u64,
    pub deleted: u64,
    pub bytes: u64,
    pub items: u32,
    pub directory: bool,
    pub original: PathBuf<PATH_BYTES>,
}
impl InfoRecord {
    pub fn encode(&self, out: &mut [u8; RECYCLE_INFO_BYTES]) -> usize {
        let path = self.original.as_str().as_bytes();
        out.fill(0);
        out[..8].copy_from_slice(MAGIC);
        out[8..12].copy_from_slice(&1u32.to_le_bytes());
        let len = 48 + path.len() + 4;
        out[12..16].copy_from_slice(&(len as u32).to_le_bytes());
        out[16..24].copy_from_slice(&self.id.to_le_bytes());
        out[24..32].copy_from_slice(&self.deleted.to_le_bytes());
        out[32..40].copy_from_slice(&self.bytes.to_le_bytes());
        out[40..44].copy_from_slice(&self.items.to_le_bytes());
        out[44] = u8::from(self.directory);
        out[46..48].copy_from_slice(&(path.len() as u16).to_le_bytes());
        out[48..48 + path.len()].copy_from_slice(path);
        let crc = crc32(&out[..48 + path.len()]);
        out[48 + path.len()..len].copy_from_slice(&crc.to_le_bytes());
        len
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 52 || bytes.len() > RECYCLE_INFO_BYTES || &bytes[..8] != MAGIC {
            return Err(Error::Corrupt);
        }
        let u32at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let u64at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        let path_len = u16::from_le_bytes([bytes[46], bytes[47]]) as usize;
        if u32at(8) != 1
            || u32at(12) as usize != bytes.len()
            || 48 + path_len + 4 != bytes.len()
            || bytes[44] > 1
            || bytes[45] != 0
            || crc32(&bytes[..48 + path_len]) != u32at(48 + path_len)
        {
            return Err(Error::Corrupt);
        }
        let id = u64at(16);
        let path = core::str::from_utf8(&bytes[48..48 + path_len]).map_err(|_| Error::Corrupt)?;
        if id == 0 || !path.starts_with('/') || path == "/" {
            return Err(Error::Corrupt);
        }
        Ok(Self {
            id,
            deleted: u64at(24),
            bytes: u64at(32),
            items: u32at(40),
            directory: bytes[44] == 1,
            original: PathBuf::from(path).map_err(|_| Error::Corrupt)?,
        })
    }
}

/// Bin operations for one authenticated user (the owner's subject).
pub struct Bin {
    pub uid: u32,
    /// Maximum total bytes kept.
    pub limit: u64,
}
fn read_info<V: Volume>(vol: &mut V, path: &str) -> Result<InfoRecord> {
    let mut buf = [0u8; RECYCLE_INFO_BYTES];
    let file = vol.open_read(path)?;
    let mut len = 0;
    let result = loop {
        match vol.read(&file, len as u64, &mut buf[len..]) {
            Ok(0) => break Ok(()),
            Ok(n) => {
                len += n;
                if len == buf.len() {
                    break Ok(());
                }
            }
            Err(e) => break Err(e),
        }
    };
    vol.close(file)?;
    result?;
    InfoRecord::decode(&buf[..len])
}
impl Bin {
    pub fn path(&self) -> Result<PathBuf<64>> {
        bin_path(self.uid)
    }
    /// Every valid info record, in ascending ID order after `after`.
    pub fn next<V: Volume>(&self, vol: &mut V, after: u64) -> Result<Option<InfoRecord>> {
        let bin = self.path()?;
        let mut best: Option<(u64, Name)> = None;
        let mut cursor = 0;
        loop {
            let entry = match vol.child(bin.as_str(), cursor) {
                Ok(entry) => entry,
                Err(Error::NotFound) => return Ok(None),
                Err(e) => return Err(e),
            };
            let Some((name, stat)) = entry else { break };
            cursor = stat.object;
            if let Some(id) = parse_id(name.as_str(), "$I")
                && !stat.directory
                && id > after
                && best.is_none_or(|(b, _)| id < b)
            {
                best = Some((id, name));
            }
        }
        let Some((_, name)) = best else {
            return Ok(None);
        };
        let path = join(bin.as_str(), name.as_str())?;
        match read_info(vol, path.as_str()) {
            Ok(info) => Ok(Some(info)),
            // Skip a damaged record rather than hiding every later item.
            Err(Error::Corrupt) => {
                let id = parse_id(name.as_str(), "$I").unwrap_or(after.saturating_add(1));
                self.next(vol, id)
            }
            Err(e) => Err(e),
        }
    }
    pub fn usage<V: Volume>(&self, vol: &mut V) -> Result<Usage> {
        let mut usage = Usage {
            limit: self.limit,
            ..Usage::default()
        };
        let mut after = 0;
        while let Some(info) = self.next(vol, after)? {
            after = info.id;
            usage.items = usage.items.saturating_add(1);
            usage.bytes = usage.bytes.saturating_add(info.bytes);
        }
        Ok(usage)
    }
    fn next_id<V: Volume>(&self, vol: &mut V) -> Result<u64> {
        let bin = self.path()?;
        let mut max = 0;
        let mut cursor = 0;
        while let Some((name, stat)) = vol.child(bin.as_str(), cursor)? {
            cursor = stat.object;
            let id = parse_id(name.as_str(), "$I")
                .or_else(|| parse_id(name.as_str(), "$R"))
                .unwrap_or(0);
            max = max.max(id);
        }
        // Seconds since 1601 keep IDs increasing after the bin is emptied.
        max.checked_add(1)
            .map(|id| id.max(vol.now() / 10_000_000))
            .filter(|id| *id < 1 << 48)
            .ok_or(Error::NoSpace)
    }
    /// Move `path` into the bin with its original path and deletion time.
    /// One durable transaction; the oldest items are purged first when the
    /// bin would exceed its limit.
    pub fn recycle<V: Volume>(&self, vol: &mut V, path: &str) -> Result<Outcome> {
        vol.commit()?;
        let bin = self.path()?;
        let home = parent_of(bin.as_str());
        if !path.starts_with('/') || within(path, bin.as_str()) || within(home, path) {
            return Err(Error::Invalid);
        }
        let stat = vol.stat(path)?;
        let (items, bytes) = measure(vol, path)?;
        if bytes > self.limit {
            return Ok(Outcome::TooBig);
        }
        match vol.stat(bin.as_str()) {
            Ok(s) if s.directory => (),
            Ok(_) => return Err(Error::Corrupt),
            Err(Error::NotFound) => {
                vol.mkdir(bin.as_str())?;
                vol.set_hidden(bin.as_str(), true)?;
            }
            Err(e) => return Err(e),
        }
        let mut usage = self.usage(vol)?;
        while usage.bytes.saturating_add(bytes) > self.limit {
            let Some(oldest) = self.oldest(vol)? else {
                break;
            };
            self.purge_now(vol, oldest.id)?;
            usage.bytes = usage.bytes.saturating_sub(oldest.bytes);
        }
        let id = self.next_id(vol)?;
        let item = join(bin.as_str(), item_name(id, path, stat.directory)?.as_str())?;
        let info_path = join(bin.as_str(), info_name(id)?.as_str())?;
        let record = InfoRecord {
            id,
            deleted: vol.now(),
            bytes,
            items,
            directory: stat.directory,
            original: PathBuf::from(path)?,
        };
        let mut buf = [0u8; RECYCLE_INFO_BYTES];
        let len = record.encode(&mut buf);
        vol.rename(path, item.as_str(), false)?;
        let file = vol.create(info_path.as_str())?;
        let written = vol.write(&file, 0, &buf[..len]);
        vol.close(file)?;
        written?;
        vol.commit()?;
        Ok(Outcome::Recycled(id))
    }
    fn oldest<V: Volume>(&self, vol: &mut V) -> Result<Option<InfoRecord>> {
        let mut best: Option<InfoRecord> = None;
        let mut after = 0;
        while let Some(info) = self.next(vol, after)? {
            after = info.id;
            if best.is_none_or(|b| (info.deleted, info.id) < (b.deleted, b.id)) {
                best = Some(info);
            }
        }
        Ok(best)
    }
    /// Paths of `id`'s info record and item (for a purge operation).
    pub fn entry_paths<V: Volume>(
        &self,
        vol: &mut V,
        id: u64,
    ) -> Result<(PathBuf<PATH_BYTES>, Option<PathBuf<PATH_BYTES>>)> {
        let bin = self.path()?;
        let info_path = join(bin.as_str(), info_name(id)?.as_str())?;
        let mut item = None;
        let mut cursor = 0;
        while let Some((name, stat)) = vol.child(bin.as_str(), cursor)? {
            cursor = stat.object;
            if parse_id(name.as_str(), "$R") == Some(id) {
                item = Some(join(bin.as_str(), name.as_str())?);
            }
        }
        Ok((info_path, item))
    }
    /// Remove one entry permanently right away (bin limit enforcement):
    /// the record goes first, then its tree in bounded transactions.
    fn purge_now<V: Volume>(&self, vol: &mut V, id: u64) -> Result<()> {
        let (info, item) = self.entry_paths(vol, id)?;
        match vol.remove(info.as_str()) {
            Ok(()) | Err(Error::NotFound) => (),
            Err(e) => return Err(e),
        }
        vol.commit()?;
        if let Some(item) = item {
            delete_tree(vol, item.as_str())?;
        }
        Ok(())
    }
    /// Put item `id` back at its original path (recreating missing parent
    /// folders). `Exists` when the original name is taken.
    pub fn restore<V: Volume>(&self, vol: &mut V, id: u64) -> Result<()> {
        vol.commit()?;
        let (info_path, item) = self.entry_paths(vol, id)?;
        let info = read_info(vol, info_path.as_str())?;
        if info.id != id {
            return Err(Error::Corrupt);
        }
        let item = item.ok_or(Error::NotFound)?;
        let original = info.original.as_str();
        match vol.stat(original) {
            Ok(_) => return Err(Error::Exists),
            Err(Error::NotFound) => (),
            Err(e) => return Err(e),
        }
        let parent = parent_of(original);
        let mut built = PathBuf::<PATH_BYTES>::from("/")?;
        for component in parent.split('/').filter(|c| !c.is_empty()) {
            built.push(component)?;
            match vol.stat(built.as_str()) {
                Ok(s) if s.directory => (),
                Ok(_) => return Err(Error::Exists),
                Err(Error::NotFound) => vol.mkdir(built.as_str())?,
                Err(e) => return Err(e),
            }
        }
        vol.rename(item.as_str(), original, false)?;
        vol.remove(info_path.as_str())?;
        vol.commit()
    }
}
/// Count items and bytes of `path` (recursive).
pub(crate) fn measure<V: Volume>(vol: &mut V, path: &str) -> Result<(u32, u64)> {
    let root = vol.stat(path)?;
    if !root.directory {
        return Ok((1, root.bytes));
    }
    let mut items = 1u32;
    let mut bytes = 0u64;
    let mut dir = PathBuf::<PATH_BYTES>::from(path)?;
    let mut cursors = [0u64; crate::MAX_DEPTH + 2];
    let mut depth = 0usize;
    loop {
        match vol.child(dir.as_str(), cursors[depth])? {
            Some((name, stat)) => {
                cursors[depth] = stat.object;
                items = items.saturating_add(1);
                bytes = bytes.saturating_add(stat.bytes);
                if stat.directory && depth < crate::MAX_DEPTH {
                    dir.push(name.as_str())?;
                    depth += 1;
                    cursors[depth] = 0;
                }
            }
            None if depth == 0 => break,
            None => {
                dir.pop();
                depth -= 1;
            }
        }
    }
    Ok((items, bytes))
}
/// Permanently delete a tree, committing every few removals.
pub(crate) fn delete_tree<V: Volume>(vol: &mut V, path: &str) -> Result<()> {
    let mut dir = PathBuf::<PATH_BYTES>::from(path)?;
    if !vol.stat(path)?.directory {
        vol.remove(path)?;
        return vol.commit();
    }
    let mut depth = 0usize;
    let mut pending = 0;
    loop {
        match vol.child(dir.as_str(), 0)? {
            Some((name, stat)) if stat.directory && depth < crate::MAX_DEPTH => {
                dir.push(name.as_str())?;
                depth += 1;
            }
            Some((name, _)) => {
                let child = join(dir.as_str(), name.as_str())?;
                vol.remove(child.as_str())?;
                pending += 1;
            }
            None => {
                vol.remove(dir.as_str())?;
                pending += 1;
                if depth == 0 {
                    break;
                }
                dir.pop();
                depth -= 1;
            }
        }
        if pending >= 8 {
            vol.commit()?;
            pending = 0;
        }
    }
    vol.commit()
}
