use super::{Error, Result};
use core::sync::atomic::{AtomicU64, Ordering};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileId {
    pub volume: [u8; 16],
    pub inode: u32,
    pub generation: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Metadata {
    pub id: FileId,
    pub bytes: u64,
    pub directory: bool,
    pub read_only: bool,
}
pub trait Vfs {
    /// Conservative default: a failed mutation may have changed backing bytes.
    /// Backends may clear this only when they track preflight versus I/O failure.
    fn mutation_uncertain(&self) -> bool {
        true
    }
    fn open(&mut self, name: &str) -> Result<FileId>;
    fn metadata(&mut self, id: FileId) -> Result<Metadata>;
    fn read(&mut self, id: FileId, offset: u64, output: &mut [u8]) -> Result<usize>;
    fn create(&mut self, _name: &str) -> Result<FileId> {
        Err(Error::ReadOnly)
    }
    fn write(&mut self, _id: FileId, _offset: u64, _input: &[u8]) -> Result<usize> {
        Err(Error::ReadOnly)
    }
    fn rename(&mut self, _old: &str, _new: &str) -> Result<()> {
        Err(Error::ReadOnly)
    }
    fn truncate(&mut self, _id: FileId, _bytes: u64) -> Result<()> {
        Err(Error::ReadOnly)
    }
    fn unlink(&mut self, _name: &str, _expected: FileId) -> Result<()> {
        Err(Error::ReadOnly)
    }
    /// Allocation usage as (total bytes, free bytes) when the backend tracks it.
    fn usage(&self) -> Option<(u64, u64)> {
        None
    }
    /// A volume at scale (many groups, large files): the namespace keeps
    /// its nodes in a tree there (catalog v6); a constrained volume keeps
    /// the original catalog, byte for byte.
    fn scaled(&self) -> bool {
        false
    }
    fn flush(&mut self) -> Result<()>;
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpenFile {
    mount: u64,
    pub id: FileId,
}
static NEXT_MOUNT: AtomicU64 = AtomicU64::new(1);
/// Native mount routing is explicit; handles carry mount incarnation and inode,
/// never a saved pathname. This is not a Windows path/permission implementation.
pub struct Mount<F> {
    prefix: &'static str,
    identity: u64,
    backend: F,
}
impl<F: Vfs> Mount<F> {
    pub fn new(prefix: &'static str, backend: F) -> Result<Self> {
        if !prefix.starts_with('/')
            || prefix.len() < 2
            || prefix.ends_with('/')
            || prefix[1..].contains('/')
        {
            return Err(Error::InvalidName);
        }
        let identity = NEXT_MOUNT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Error::NoSpace)?;
        Ok(Self {
            prefix,
            identity,
            backend,
        })
    }
    fn relative<'a>(&self, path: &'a str) -> Result<&'a str> {
        path.strip_prefix(self.prefix)
            .and_then(|s| s.strip_prefix('/'))
            .filter(|s| !s.is_empty())
            .ok_or(Error::NotFound)
    }
    fn check(&self, file: OpenFile) -> Result<FileId> {
        if file.mount != self.identity {
            Err(Error::NotFound)
        } else {
            Ok(file.id)
        }
    }
    pub fn open(&mut self, path: &str) -> Result<OpenFile> {
        let name = self.relative(path)?;
        let id = self.backend.open(name)?;
        Ok(OpenFile {
            mount: self.identity,
            id,
        })
    }
    pub fn create(&mut self, path: &str) -> Result<OpenFile> {
        let name = self.relative(path)?;
        let id = self.backend.create(name)?;
        Ok(OpenFile {
            mount: self.identity,
            id,
        })
    }
    pub fn read(&mut self, file: OpenFile, offset: u64, out: &mut [u8]) -> Result<usize> {
        let id = self.check(file)?;
        self.backend.read(id, offset, out)
    }
    pub fn write(&mut self, file: OpenFile, offset: u64, input: &[u8]) -> Result<usize> {
        let id = self.check(file)?;
        self.backend.write(id, offset, input)
    }
    pub fn metadata(&mut self, file: OpenFile) -> Result<Metadata> {
        let id = self.check(file)?;
        self.backend.metadata(id)
    }
    pub fn rename(&mut self, old: &str, new: &str) -> Result<()> {
        let old = self.relative(old)?;
        let new = self.relative(new)?;
        self.backend.rename(old, new)
    }
    pub fn flush(&mut self) -> Result<()> {
        self.backend.flush()
    }
    pub fn into_inner(self) -> F {
        self.backend
    }
}
pub struct InitFs<'a, 'b> {
    archive: &'a door_initfs::Archive<'b>,
}
impl<'a, 'b> InitFs<'a, 'b> {
    pub fn new(archive: &'a door_initfs::Archive<'b>) -> Self {
        Self { archive }
    }
}
const INITFS_VOLUME: [u8; 16] = *b"DOOR-INITFS-V1\0\0";
impl Vfs for InitFs<'_, '_> {
    fn open(&mut self, name: &str) -> Result<FileId> {
        for i in 0..self.archive.len() {
            let e = self.archive.entry(i).map_err(|_| Error::Corrupt)?;
            if e.path == name {
                return Ok(FileId {
                    volume: INITFS_VOLUME,
                    inode: i as u32 + 1,
                    generation: 0,
                });
            }
        }
        Err(Error::NotFound)
    }
    fn metadata(&mut self, id: FileId) -> Result<Metadata> {
        if id.volume != INITFS_VOLUME || id.inode == 0 || id.generation != 0 {
            return Err(Error::NotFound);
        }
        let e = self
            .archive
            .entry(id.inode as usize - 1)
            .map_err(|_| Error::NotFound)?;
        Ok(Metadata {
            id,
            bytes: e.data.len() as u64,
            directory: e.directory,
            read_only: true,
        })
    }
    fn read(&mut self, id: FileId, offset: u64, out: &mut [u8]) -> Result<usize> {
        let m = self.metadata(id)?;
        if m.directory {
            return Err(Error::IsDirectory);
        }
        if offset >= m.bytes {
            return Ok(0);
        }
        let e = self
            .archive
            .entry(id.inode as usize - 1)
            .map_err(|_| Error::Corrupt)?;
        let n = out.len().min(e.data.len() - offset as usize);
        out[..n].copy_from_slice(&e.data[offset as usize..offset as usize + n]);
        Ok(n)
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}
