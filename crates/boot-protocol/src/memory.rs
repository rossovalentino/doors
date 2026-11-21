use crate::{Error, MEMORY_REGION_BYTES, wire};

pub const PAGE_BYTES: u64 = 4096;

/// Half-open extent; construction prevents wrapping and invalid alignment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PhysicalRange {
    start: u64,
    end: u64,
}

impl PhysicalRange {
    pub fn new(start: u64, length: u64, alignment: u64) -> Result<Self, Error> {
        if length == 0 {
            return Err(Error::EmptyRange);
        }
        let end = start.checked_add(length).ok_or(Error::Overflow)?;
        if !alignment.is_power_of_two()
            || !start.is_multiple_of(alignment)
            || !length.is_multiple_of(alignment)
        {
            return Err(Error::Misaligned);
        }
        Ok(Self { start, end })
    }

    pub const fn start(self) -> u64 {
        self.start
    }
    pub const fn length(self) -> u64 {
        self.end - self.start
    }
    pub const fn end(self) -> u64 {
        self.end
    }
    pub const fn contains(self, other: Self) -> bool {
        self.start <= other.start && other.end <= self.end
    }
    pub const fn overlaps(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum Ownership {
    UsableAfterHandoff = 0,
    PermanentlyReserved = 1,
    ReclaimableAfterCopy = 2,
    KernelModule = 3,
    FramebufferMmio = 4,
    FirmwareRuntime = 5,
    BootData = 6,
}

impl Ownership {
    fn decode(value: u32) -> Result<Self, Error> {
        match value {
            0 => Ok(Self::UsableAfterHandoff),
            1 => Ok(Self::PermanentlyReserved),
            2 => Ok(Self::ReclaimableAfterCopy),
            3 => Ok(Self::KernelModule),
            4 => Ok(Self::FramebufferMmio),
            5 => Ok(Self::FirmwareRuntime),
            6 => Ok(Self::BootData),
            _ => Err(Error::InvalidOwnership),
        }
    }

    pub(crate) fn protects_boot_data(self) -> bool {
        matches!(
            self,
            Self::BootData | Self::KernelModule | Self::PermanentlyReserved
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryRegion {
    pub range: PhysicalRange,
    pub firmware_attributes: u64,
    pub firmware_type: u32,
    pub ownership: Ownership,
}

impl MemoryRegion {
    pub fn decode(input: &[u8]) -> Result<Self, Error> {
        let bytes = input.get(..MEMORY_REGION_BYTES).ok_or(Error::ShortRecord)?;
        if wire::read::<8>(bytes, 32)? != [0; 8] {
            return Err(Error::ReservedBits);
        }
        let region = Self {
            range: PhysicalRange::new(
                u64::from_le_bytes(wire::read(bytes, 0)?),
                u64::from_le_bytes(wire::read(bytes, 8)?),
                PAGE_BYTES,
            )?,
            firmware_attributes: u64::from_le_bytes(wire::read(bytes, 16)?),
            firmware_type: u32::from_le_bytes(wire::read(bytes, 24)?),
            ownership: Ownership::decode(u32::from_le_bytes(wire::read(bytes, 28)?))?,
        };
        region.validate()?;
        Ok(region)
    }

    pub fn encode(self, output: &mut [u8]) -> Result<(), Error> {
        self.validate()?;
        let bytes = output
            .get_mut(..MEMORY_REGION_BYTES)
            .ok_or(Error::ShortRecord)?;
        bytes.fill(0);
        wire::write(bytes, 0, self.range.start().to_le_bytes());
        wire::write(bytes, 8, self.range.length().to_le_bytes());
        wire::write(bytes, 16, self.firmware_attributes.to_le_bytes());
        wire::write(bytes, 24, self.firmware_type.to_le_bytes());
        wire::write(bytes, 28, (self.ownership as u32).to_le_bytes());
        Ok(())
    }

    fn validate(self) -> Result<(), Error> {
        PhysicalRange::new(self.range.start(), self.range.length(), PAGE_BYTES)?;
        // UEFI 2.11 §7.2.3: runtime code/data types 5/6 and EFI_MEMORY_RUNTIME
        // (bit 63) survive ExitBootServices and must not enter the free pool.
        if (matches!(self.firmware_type, 5 | 6) || self.firmware_attributes & (1 << 63) != 0)
            && self.ownership != Ownership::FirmwareRuntime
        {
            return Err(Error::RuntimeMemoryOwnership);
        }
        // UEFI conventional memory is the only initial free-memory source.
        // Explicit later reclaim of loader/boot-service memory needs a lifetime
        // transition, rather than changing its ownership here without evidence.
        if self.ownership == Ownership::UsableAfterHandoff && self.firmware_type != 7 {
            return Err(Error::InvalidOwnership);
        }
        Ok(())
    }
}
