use super::{Error, Partition, Result};
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Geometry {
    pub sector_size: u32,
    pub sectors: u64,
    pub read_only: bool,
    pub flush_supported: bool,
}
/// Completion means every requested byte transferred. An error may have changed
/// a prefix; consumers must never cache an errored read or acknowledge its write.
pub trait BlockDevice {
    fn geometry(&self) -> Geometry;
    fn read_sectors(&mut self, lba: u64, output: &mut [u8]) -> Result<()>;
    fn write_sectors(&mut self, lba: u64, input: &[u8]) -> Result<()>;
    fn flush(&mut self) -> Result<()>;
}
impl<T: BlockDevice + ?Sized> BlockDevice for &mut T {
    fn geometry(&self) -> Geometry {
        (**self).geometry()
    }
    fn read_sectors(&mut self, lba: u64, b: &mut [u8]) -> Result<()> {
        (**self).read_sectors(lba, b)
    }
    fn write_sectors(&mut self, lba: u64, b: &[u8]) -> Result<()> {
        (**self).write_sectors(lba, b)
    }
    fn flush(&mut self) -> Result<()> {
        (**self).flush()
    }
}
pub fn check_transfer(geometry: Geometry, lba: u64, bytes: usize) -> Result<u64> {
    if geometry.sector_size == 0
        || bytes == 0
        || !bytes.is_multiple_of(geometry.sector_size as usize)
    {
        return Err(Error::Alignment);
    }
    let count = (bytes / geometry.sector_size as usize) as u64;
    if lba.checked_add(count).ok_or(Error::Overflow)? > geometry.sectors {
        return Err(Error::Bounds);
    }
    Ok(count)
}
pub struct PartitionDevice<D> {
    device: D,
    partition: Partition,
}
impl<D: BlockDevice> PartitionDevice<D> {
    pub fn new(device: D, partition: Partition) -> Result<Self> {
        let g = device.geometry();
        if g.sector_size != 512
            || partition.sectors == 0
            || partition
                .first
                .checked_add(partition.sectors)
                .ok_or(Error::Overflow)?
                > g.sectors
        {
            return Err(Error::Bounds);
        }
        Ok(Self { device, partition })
    }
    pub fn into_inner(self) -> D {
        self.device
    }
}
impl<D: BlockDevice> BlockDevice for PartitionDevice<D> {
    fn geometry(&self) -> Geometry {
        Geometry {
            sectors: self.partition.sectors,
            read_only: self.device.geometry().read_only
                || self.partition.attributes & (1 << 60) != 0,
            ..self.device.geometry()
        }
    }
    fn read_sectors(&mut self, lba: u64, b: &mut [u8]) -> Result<()> {
        check_transfer(self.geometry(), lba, b.len())?;
        self.device.read_sectors(self.partition.first + lba, b)
    }
    fn write_sectors(&mut self, lba: u64, b: &[u8]) -> Result<()> {
        if self.geometry().read_only {
            return Err(Error::ReadOnly);
        }
        check_transfer(self.geometry(), lba, b.len())?;
        self.device.write_sectors(self.partition.first + lba, b)
    }
    fn flush(&mut self) -> Result<()> {
        if !self.geometry().flush_supported {
            return Err(Error::Unsupported);
        }
        self.device.flush()
    }
}
