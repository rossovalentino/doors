//! Microsoft FAT 1.03: read-only, 512-byte clusters and one flat root cluster.
use super::{BlockDevice, Error, FileId, Metadata, Result, Vfs, u16le, u32le};
#[derive(Clone, Copy)]
struct File {
    name: [u8; 11],
    cluster: u32,
    bytes: u32,
}
pub struct Fat32<D> {
    device: D,
    reserved: u32,
    fat_sectors: u32,
    data: u32,
    clusters: u32,
    volume: [u8; 16],
    files: [Option<File>; 16],
}
impl<D: BlockDevice> Fat32<D> {
    pub fn mount(mut device: D) -> Result<Self> {
        if device.geometry().sector_size != 512 {
            return Err(Error::Unsupported);
        }
        let mut boot = [0u8; 512];
        device.read_sectors(0, &mut boot)?;
        let reserved = u32::from(u16le(&boot, 14));
        let fats = u32le(&boot, 36);
        let total = u32le(&boot, 32);
        if boot[510..] != [0x55, 0xaa]
            || u16le(&boot, 11) != 512
            || boot[13] != 1
            || reserved != 32
            || boot[16] != 2
            || u16le(&boot, 17) != 0
            || u16le(&boot, 19) != 0
            || u16le(&boot, 22) != 0
            || boot[21] != 0xf8
            || u16le(&boot, 40) != 0
            || u16le(&boot, 42) != 0
            || u32le(&boot, 44) != 2
            || u16le(&boot, 48) != 1
            || u16le(&boot, 50) != 6
            || boot[66] != 0x29
            || fats == 0
        {
            return Err(Error::Unsupported);
        }
        let data = reserved
            .checked_add(fats.checked_mul(2).ok_or(Error::Overflow)?)
            .ok_or(Error::Overflow)?;
        let clusters = total.checked_sub(data).ok_or(Error::Corrupt)?;
        if !(65525..=131072).contains(&clusters)
            || u64::from(total) > device.geometry().sectors
            || u64::from(fats) * 128 < u64::from(clusters) + 2
        {
            return Err(Error::Corrupt);
        }
        let mut backup = [0u8; 512];
        device.read_sectors(6, &mut backup)?;
        if backup != boot {
            return Err(Error::Corrupt);
        }
        let mut info = [0u8; 512];
        device.read_sectors(1, &mut info)?;
        device.read_sectors(7, &mut backup)?;
        if backup != info
            || u32le(&info, 0) != 0x4161_5252
            || u32le(&info, 484) != 0x6141_7272
            || u32le(&info, 508) != 0xaa55_0000
            || (u32le(&info, 488) != u32::MAX && u32le(&info, 488) > clusters)
            || (u32le(&info, 492) != u32::MAX && !(2..clusters + 2).contains(&u32le(&info, 492)))
        {
            return Err(Error::Corrupt);
        }
        let mut volume = *b"DOOR-FAT32--\0\0\0\0";
        volume[12..].copy_from_slice(&boot[67..71]);
        let mut fs = Self {
            device,
            reserved,
            fat_sectors: fats,
            data,
            clusters,
            volume,
            files: [None; 16],
        };
        if fs.next(0)? != 0x0fff_fff8 || fs.next(1)? < 0x0fff_fff8 || fs.next(2)? < 0x0fff_fff8 {
            return Err(Error::Corrupt);
        }
        let mut directory = [0u8; 512];
        fs.device.read_sectors(u64::from(data), &mut directory)?;
        let mut used = [0u8; 16385];
        used[0] |= 1;
        let mut count = 0;
        for record in directory.chunks_exact(32) {
            if record[0] == 0 {
                break;
            }
            if record[0] == 0xe5 {
                continue;
            }
            let attr = record[11];
            if attr & 0xc0 != 0 || attr == 0x0f || attr & 0x10 != 0 {
                return Err(Error::Unsupported);
            }
            if attr & 8 != 0 {
                continue;
            }
            if count == 16 || record[..11].iter().any(|b| *b < 0x20 || *b >= 0x7f) {
                return Err(Error::Corrupt);
            }
            let file = File {
                name: record[..11].try_into().unwrap(),
                cluster: (u32::from(u16le(record, 20)) << 16) | u32::from(u16le(record, 26)),
                bytes: u32le(record, 28),
            };
            if fs.files.iter().flatten().any(|old| old.name == file.name)
                || file.bytes > 1024 * 1024
            {
                return Err(Error::Corrupt);
            }
            let pages = u64::from(file.bytes).div_ceil(512);
            let mut cluster = file.cluster;
            if pages == 0 && cluster != 0 {
                return Err(Error::Corrupt);
            }
            for page in 0..pages {
                fs.cluster(cluster)?;
                let index = (cluster - 2) as usize;
                if used[index / 8] & (1 << (index % 8)) != 0 {
                    return Err(Error::Corrupt);
                }
                used[index / 8] |= 1 << (index % 8);
                let next = fs.next(cluster)?;
                if page + 1 == pages {
                    if next < 0x0fff_fff8 {
                        return Err(Error::Corrupt);
                    }
                } else {
                    fs.cluster(next)?;
                }
                cluster = next;
            }
            fs.files[count] = Some(file);
            count += 1;
        }
        Ok(fs)
    }
    pub fn into_inner(self) -> D {
        self.device
    }
    fn cluster(&self, cluster: u32) -> Result<()> {
        if !(2..self.clusters + 2).contains(&cluster) {
            Err(Error::Corrupt)
        } else {
            Ok(())
        }
    }
    fn next(&mut self, cluster: u32) -> Result<u32> {
        if cluster >= self.clusters + 2 {
            return Err(Error::Corrupt);
        }
        let sector = cluster / 128;
        let offset = (cluster % 128) as usize * 4;
        let mut a = [0; 512];
        let mut b = [0; 512];
        self.device
            .read_sectors(u64::from(self.reserved + sector), &mut a)?;
        self.device
            .read_sectors(u64::from(self.reserved + self.fat_sectors + sector), &mut b)?;
        if a != b {
            return Err(Error::Corrupt);
        }
        Ok(u32le(&a, offset) & 0x0fff_ffff)
    }
    fn file(&self, id: FileId) -> Result<File> {
        if id.volume != self.volume || id.generation != 0 || id.inode == 0 {
            return Err(Error::NotFound);
        }
        self.files
            .get(id.inode as usize - 1)
            .copied()
            .flatten()
            .ok_or(Error::NotFound)
    }
}
fn short_name(name: &str) -> Result<[u8; 11]> {
    let mut parts = name.split('.');
    let base = parts.next().ok_or(Error::InvalidName)?;
    let ext = parts.next().unwrap_or("");
    if parts.next().is_some()
        || base.is_empty()
        || base.len() > 8
        || ext.len() > 3
        || !base
            .bytes()
            .chain(ext.bytes())
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err(Error::InvalidName);
    }
    let mut out = [b' '; 11];
    for (i, b) in base.bytes().enumerate() {
        out[i] = b.to_ascii_uppercase();
    }
    for (i, b) in ext.bytes().enumerate() {
        out[8 + i] = b.to_ascii_uppercase();
    }
    Ok(out)
}
impl<D: BlockDevice> Vfs for Fat32<D> {
    fn open(&mut self, name: &str) -> Result<FileId> {
        let short = short_name(name)?;
        let index = self
            .files
            .iter()
            .position(|f| f.is_some_and(|f| f.name == short))
            .ok_or(Error::NotFound)?;
        Ok(FileId {
            volume: self.volume,
            inode: index as u32 + 1,
            generation: 0,
        })
    }
    fn metadata(&mut self, id: FileId) -> Result<Metadata> {
        let f = self.file(id)?;
        Ok(Metadata {
            id,
            bytes: u64::from(f.bytes),
            directory: false,
            read_only: true,
        })
    }
    fn read(&mut self, id: FileId, offset: u64, out: &mut [u8]) -> Result<usize> {
        let file = self.file(id)?;
        if offset >= u64::from(file.bytes) {
            return Ok(0);
        }
        let count = out.len().min((u64::from(file.bytes) - offset) as usize);
        let mut cluster = file.cluster;
        for _ in 0..offset / 512 {
            cluster = self.next(cluster)?;
            self.cluster(cluster)?;
        }
        let mut done = 0;
        let mut within = offset as usize % 512;
        while done < count {
            self.cluster(cluster)?;
            let mut sector = [0; 512];
            self.device
                .read_sectors(u64::from(self.data + cluster - 2), &mut sector)?;
            let n = (512 - within).min(count - done);
            out[done..done + n].copy_from_slice(&sector[within..within + n]);
            done += n;
            within = 0;
            if done < count {
                cluster = self.next(cluster)?;
            }
        }
        Ok(done)
    }
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}
