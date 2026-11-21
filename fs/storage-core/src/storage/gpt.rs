//! UEFI 2.10 section 5, selected 512-byte/128-entry GPT profile.
use super::{BlockDevice, Error, Result, u32le, u64le};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Partition {
    pub type_guid: [u8; 16],
    pub id: [u8; 16],
    pub first: u64,
    pub sectors: u64,
    pub attributes: u64,
}
pub struct Gpt {
    pub disk_id: [u8; 16],
    entries: [Option<Partition>; 128],
    count: usize,
}
pub fn crc32(bytes: &[u8]) -> u32 {
    !crc_update(!0, bytes)
}
fn crc_update(mut crc: u32, bytes: &[u8]) -> u32 {
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = (crc >> 1) ^ ((0u32.wrapping_sub(crc & 1)) & 0xedb8_8320);
        }
    }
    crc
}
impl Gpt {
    pub fn read(device: &mut impl BlockDevice) -> Result<Self> {
        let geometry = device.geometry();
        if geometry.sector_size != 512 || geometry.sectors < 68 {
            return Err(Error::Unsupported);
        }
        let last = geometry.sectors - 1;
        let mut mbr = [0u8; 512];
        device.read_sectors(0, &mut mbr)?;
        if mbr[510..] != [0x55, 0xaa]
            || mbr[450] != 0xee
            || u32le(&mbr, 454) != 1
            || u64::from(u32le(&mbr, 458)) != last.min(u64::from(u32::MAX))
            || mbr[462..510].iter().any(|b| *b != 0)
        {
            return Err(Error::Corrupt);
        }
        let mut primary = [0u8; 512];
        let mut backup = [0u8; 512];
        device.read_sectors(1, &mut primary)?;
        device.read_sectors(last, &mut backup)?;
        header(&primary, 1, last, 2)?;
        header(&backup, last, 1, last - 32)?;
        if primary[40..72] != backup[40..72]
            || primary[80..92] != backup[80..92]
            || u64le(&primary, 40) < 34
            || u64le(&primary, 48) > last - 33
            || u64le(&primary, 40) > u64le(&primary, 48)
        {
            return Err(Error::Corrupt);
        }
        let mut result = Self {
            disk_id: primary[56..72].try_into().unwrap(),
            entries: [None; 128],
            count: 0,
        };
        if result.disk_id == [0; 16] {
            return Err(Error::Corrupt);
        }
        let mut crc = !0;
        for sector in 0..32 {
            let mut a = [0u8; 512];
            let mut b = [0u8; 512];
            device.read_sectors(2 + sector, &mut a)?;
            device.read_sectors(last - 32 + sector, &mut b)?;
            if a != b {
                return Err(Error::Corrupt);
            }
            crc = crc_update(crc, &a);
            for record in a.chunks_exact(128) {
                if record[..16] == [0; 16] {
                    if record.iter().any(|v| *v != 0) {
                        return Err(Error::Corrupt);
                    }
                    continue;
                }
                let first = u64le(record, 32);
                let end = u64le(record, 40);
                let id: [u8; 16] = record[16..32].try_into().unwrap();
                if id == [0; 16]
                    || first > end
                    || first < u64le(&primary, 40)
                    || end > u64le(&primary, 48)
                {
                    return Err(Error::Corrupt);
                }
                let p = Partition {
                    type_guid: record[..16].try_into().unwrap(),
                    id,
                    first,
                    sectors: end - first + 1,
                    attributes: u64le(record, 48),
                };
                if result.partitions().any(|old| {
                    old.id == id || (first < old.first + old.sectors && old.first <= end)
                }) {
                    return Err(Error::Corrupt);
                }
                result.entries[result.count] = Some(p);
                result.count += 1;
            }
        }
        if !crc != u32le(&primary, 88) {
            return Err(Error::Corrupt);
        }
        Ok(result)
    }
    pub fn partitions(&self) -> impl Iterator<Item = Partition> + '_ {
        self.entries[..self.count].iter().flatten().copied()
    }
    pub fn find(&self, type_guid: [u8; 16]) -> Result<Partition> {
        let mut found = self.partitions().filter(|p| p.type_guid == type_guid);
        let p = found.next().ok_or(Error::NotFound)?;
        if found.next().is_some() {
            return Err(Error::Corrupt);
        }
        Ok(p)
    }
}
fn header(bytes: &[u8; 512], current: u64, other: u64, table: u64) -> Result<()> {
    if &bytes[..8] != b"EFI PART"
        || u32le(bytes, 8) != 0x10000
        || u32le(bytes, 12) != 92
        || u32le(bytes, 20) != 0
        || u64le(bytes, 24) != current
        || u64le(bytes, 32) != other
        || u64le(bytes, 72) != table
        || u32le(bytes, 80) != 128
        || u32le(bytes, 84) != 128
        || bytes[92..].iter().any(|b| *b != 0)
    {
        return Err(Error::Corrupt);
    }
    let mut header = *bytes;
    header[16..20].fill(0);
    if crc32(&header[..92]) != u32le(bytes, 16) {
        return Err(Error::Corrupt);
    }
    Ok(())
}
