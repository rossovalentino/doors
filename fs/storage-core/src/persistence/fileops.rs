//! `door_file_ops::Volume` over the journaled persistent namespace.
//!
//! Data operations use the authenticated user's subject, so every step
//! re-checks current ACLs, sharing and protected-node policy against the
//! actual objects. Operation records are owner-private files in
//! `/state/machine/fileops` (system ACL), staged in the same redo
//! transaction as the data they describe.
use super::{
    Access, Context, Disposition, Error, FileHandle, OpenOptions, Path, PersistentVolume, Subject,
};
use crate::storage::{BlockDevice, Ext2, Journal, Vfs};
use door_file_ops as ops;

pub const RECORD_FOLDER: &str = "/state/machine/fileops";

pub fn op_error(error: Error) -> ops::Error {
    use crate::storage::Error as S;
    match error {
        Error::AccessDenied => ops::Error::Denied,
        Error::SharingViolation => ops::Error::Sharing,
        Error::NotFound | Error::DeletePending | Error::KeyDeleted => ops::Error::NotFound,
        Error::Storage(S::NotFound) => ops::Error::NotFound,
        Error::AlreadyExists | Error::Storage(S::AlreadyExists) => ops::Error::Exists,
        Error::NoSpace | Error::Storage(S::NoSpace) => ops::Error::NoSpace,
        Error::Changed => ops::Error::Changed,
        Error::Corrupt | Error::Storage(S::Corrupt) => ops::Error::Corrupt,
        Error::Poisoned => ops::Error::Poisoned,
        Error::InvalidName
        | Error::InvalidParameter
        | Error::TypeMismatch
        | Error::BufferTooSmall
        | Error::InvalidHandle
        | Error::Unsupported
        | Error::Storage(S::Unsupported) => ops::Error::Invalid,
        Error::Storage(_) => ops::Error::Io,
    }
}
fn record_name(op: u64, out: &mut [u8; 18]) -> &str {
    out[..2].copy_from_slice(b"op");
    for i in 0..16 {
        out[2 + i] = b"0123456789abcdef"[((op >> ((15 - i) * 4)) & 15) as usize];
    }
    core::str::from_utf8(out).unwrap_or("op")
}
fn record_path(op: u64) -> ops::PathBuf<64> {
    let mut name = [0; 18];
    let mut p = ops::PathBuf::from(RECORD_FOLDER).unwrap_or_default();
    let _ = p.push(record_name(op, &mut name));
    p
}
/// Volumes whose staged state can be made durable (the journaled owner).
pub trait Durable {
    fn commit_durable(&mut self) -> Result<(), Error>;
}
impl<D: BlockDevice> Durable for PersistentVolume<'_, Ext2<Journal<'_, D>>> {
    fn commit_durable(&mut self) -> Result<(), Error> {
        self.commit()
    }
}
/// One serialized owner borrowed for an engine call.
pub struct OpVolume<'v, 'a, F: Vfs> {
    pub volume: &'v mut PersistentVolume<'a, F>,
    pub user: Subject,
    pub system: Subject,
}
impl<F: Vfs> OpVolume<'_, '_, F> {
    fn open(
        &mut self,
        subject: Subject,
        path: &str,
        access: Access,
        d: Disposition,
    ) -> ops::Result<FileHandle> {
        self.volume
            .open(
                subject,
                Path::Native(path),
                Context::root(),
                OpenOptions {
                    access,
                    share: Access::SHARE_ALL,
                    disposition: d,
                },
            )
            .map(|(file, _)| file)
            .map_err(op_error)
    }
    fn ensure_records(&mut self) -> ops::Result<()> {
        match self
            .volume
            .metadata(self.system, Path::Native(RECORD_FOLDER), Context::root())
        {
            Ok(info) if info.directory => Ok(()),
            Ok(_) => Err(ops::Error::Corrupt),
            Err(Error::NotFound) => self
                .volume
                .mkdir(
                    self.system,
                    Path::Native(RECORD_FOLDER),
                    Context::root(),
                    None,
                )
                .map(|_| ())
                .map_err(op_error),
            Err(e) => Err(op_error(e)),
        }
    }
}
fn utf8_name(units: &[u16]) -> ops::Result<ops::Name> {
    let mut out = [0u8; ops::NAME_BYTES];
    let mut len = 0;
    for c in char::decode_utf16(units.iter().copied()) {
        // Unpaired surrogates have no UTF-8 spelling; refuse rather than
        // address a different name.
        let c = c.map_err(|_| ops::Error::Invalid)?;
        let n = c.len_utf8();
        if len + n > out.len() {
            return Err(ops::Error::Invalid);
        }
        c.encode_utf8(&mut out[len..]);
        len += n;
    }
    ops::Name::new(core::str::from_utf8(&out[..len]).map_err(|_| ops::Error::Invalid)?)
}
fn stat(info: super::FileInfo) -> ops::Stat {
    ops::Stat {
        object: info.object,
        version: info.version,
        bytes: info.bytes,
        directory: info.directory,
        modified: info.times.modified,
    }
}
impl<'a, F: Vfs> ops::Volume for OpVolume<'_, 'a, F>
where
    PersistentVolume<'a, F>: Durable,
{
    type File = FileHandle;
    fn stat(&mut self, path: &str) -> ops::Result<ops::Stat> {
        self.volume
            .metadata(self.user, Path::Native(path), Context::root())
            .map(stat)
            .map_err(op_error)
    }
    fn child(&mut self, folder: &str, after: u64) -> ops::Result<Option<(ops::Name, ops::Stat)>> {
        match self
            .volume
            .list(self.user, Path::Native(folder), Context::root(), after)
            .map_err(op_error)?
        {
            Some(entry) => Ok(Some((utf8_name(entry.name.units())?, stat(entry.info)))),
            None => Ok(None),
        }
    }
    fn mkdir(&mut self, path: &str) -> ops::Result<()> {
        self.volume
            .mkdir(self.user, Path::Native(path), Context::root(), None)
            .map(|_| ())
            .map_err(op_error)
    }
    fn create(&mut self, path: &str) -> ops::Result<FileHandle> {
        let all = Access::READ.union(Access::WRITE).union(Access::DELETE);
        self.open(self.user, path, all, Disposition::CreateNew)
    }
    fn open_read(&mut self, path: &str) -> ops::Result<FileHandle> {
        self.open(self.user, path, Access::READ, Disposition::OpenExisting)
    }
    fn open_write(&mut self, path: &str) -> ops::Result<FileHandle> {
        self.open(
            self.user,
            path,
            Access::READ.union(Access::WRITE),
            Disposition::OpenExisting,
        )
    }
    fn read(&mut self, file: &FileHandle, offset: u64, out: &mut [u8]) -> ops::Result<usize> {
        self.volume.read(file, offset, out).map_err(op_error)
    }
    fn write(&mut self, file: &FileHandle, offset: u64, data: &[u8]) -> ops::Result<()> {
        let mut done = 0;
        while done < data.len() {
            let n = self
                .volume
                .write(file, offset + done as u64, &data[done..])
                .map_err(op_error)?;
            if n == 0 {
                return Err(ops::Error::Io);
            }
            done += n;
        }
        Ok(())
    }
    fn truncate(&mut self, file: &FileHandle, bytes: u64) -> ops::Result<()> {
        let current = self.volume.info(file).map_err(op_error)?.bytes;
        if current == bytes {
            return Ok(());
        }
        self.volume.truncate(file, bytes).map_err(op_error)
    }
    fn close(&mut self, file: FileHandle) -> ops::Result<()> {
        self.volume.close(file).map_err(op_error)
    }
    fn rename(&mut self, from: &str, to: &str, replace: bool) -> ops::Result<()> {
        if !replace {
            return self
                .volume
                .rename_path(
                    self.user,
                    Path::Native(from),
                    Path::Native(to),
                    Context::root(),
                )
                .map_err(op_error);
        }
        let file = self.open(self.user, from, Access::DELETE, Disposition::OpenExisting)?;
        let result = self
            .volume
            .rename(self.user, &file, Path::Native(to), Context::root(), true)
            .map_err(op_error);
        let closed = self.volume.close(file).map_err(op_error);
        result.and(closed)
    }
    fn remove(&mut self, path: &str) -> ops::Result<()> {
        self.volume
            .remove_path(self.user, Path::Native(path), Context::root())
            .map_err(op_error)
    }
    fn set_hidden(&mut self, path: &str, hidden: bool) -> ops::Result<()> {
        let info = self
            .volume
            .metadata(self.user, Path::Native(path), Context::root())
            .map_err(op_error)?;
        let attributes = if hidden {
            info.attributes | 2
        } else {
            info.attributes & !2
        };
        self.volume
            .set_attributes_path(self.user, Path::Native(path), Context::root(), attributes)
            .map_err(op_error)
    }
    fn commit(&mut self) -> ops::Result<()> {
        self.volume.commit_durable().map_err(op_error)
    }
    fn save_record(&mut self, op: u64, bytes: &[u8]) -> ops::Result<()> {
        self.ensure_records()?;
        let path = record_path(op);
        let file = self.open(
            self.system,
            path.as_str(),
            Access::READ.union(Access::WRITE),
            Disposition::OpenAlways,
        )?;
        // Rewrite in place (unchanged pages compact away in the journal),
        // then cut any longer old record.
        let mut result = Ok(());
        for (i, chunk) in bytes.chunks(4096).enumerate() {
            result = ops::Volume::write(self, &file, (i * 4096) as u64, chunk);
            if result.is_err() {
                break;
            }
        }
        if result.is_ok() {
            result = ops::Volume::truncate(self, &file, bytes.len() as u64);
        }
        let closed = self.volume.close(file).map_err(op_error);
        result.and(closed)
    }
    fn load_record(&mut self, op: u64, out: &mut [u8]) -> ops::Result<Option<usize>> {
        let path = record_path(op);
        let file = match self.open(
            self.system,
            path.as_str(),
            Access::READ,
            Disposition::OpenExisting,
        ) {
            Ok(file) => file,
            Err(ops::Error::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut len = 0;
        let result = loop {
            if len == out.len() {
                // Longer than any valid record.
                let mut probe = [0u8; 1];
                break match self.volume.read(&file, len as u64, &mut probe) {
                    Ok(0) => Ok(()),
                    Ok(_) => Err(ops::Error::Corrupt),
                    Err(e) => Err(op_error(e)),
                };
            }
            match self.volume.read(&file, len as u64, &mut out[len..]) {
                Ok(0) => break Ok(()),
                Ok(n) => len += n,
                Err(e) => break Err(op_error(e)),
            }
        };
        let closed = self.volume.close(file).map_err(op_error);
        result.and(closed).map(|()| Some(len))
    }
    fn remove_record(&mut self, op: u64) -> ops::Result<()> {
        let path = record_path(op);
        match self
            .volume
            .remove_path(self.system, Path::Native(path.as_str()), Context::root())
        {
            Ok(()) | Err(Error::NotFound) => Ok(()),
            Err(e) => Err(op_error(e)),
        }
    }
    fn next_record(&mut self, after: u64) -> ops::Result<Option<u64>> {
        let mut best: Option<u64> = None;
        let mut cursor = 0;
        loop {
            let entry = match self.volume.list(
                self.system,
                Path::Native(RECORD_FOLDER),
                Context::root(),
                cursor,
            ) {
                Ok(entry) => entry,
                Err(Error::NotFound) => return Ok(None),
                Err(e) => return Err(op_error(e)),
            };
            let Some(entry) = entry else { break };
            cursor = entry.info.object;
            let units = entry.name.units();
            if units.len() == 18 && units[..2] == [b'o' as u16, b'p' as u16] {
                let mut value = 0u64;
                let mut valid = true;
                for unit in &units[2..] {
                    match char::from_u32(u32::from(*unit)).and_then(|c| c.to_digit(16)) {
                        Some(d) => value = value << 4 | u64::from(d),
                        None => valid = false,
                    }
                }
                if valid && value > after && best.is_none_or(|b| value < b) {
                    best = Some(value);
                }
            }
        }
        Ok(best)
    }
    fn file_limit(&self) -> u64 {
        super::LARGE_REPLACE_BYTES as u64
    }
    fn now(&self) -> u64 {
        self.volume.clock()
    }
}
