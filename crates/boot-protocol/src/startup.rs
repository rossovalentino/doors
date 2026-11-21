//! Firmware-owned low pages reserved for the kernel's secondary-CPU startup.
use crate::{
    BootInfo, Error, MAPPING_BYTES, MAPPINGS_KIND, Mapping, Ownership, PhysicalRange, wire,
};

pub const STARTUP_KIND: u32 = 9;
pub const STARTUP_BYTES: usize = 64;
pub const STARTUP_CODE_ROLE: u32 = 9;
pub const STARTUP_DATA_ROLE: u32 = 10;
pub const STARTUP_PAGE_BYTES: u64 = 4096;

/// Copied addresses, never firmware pointers. Initial mappings are both RW/NX;
/// the kernel writes the trampoline then transitions only the code page to RX.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Startup {
    pub code_physical: u64,
    pub data_physical: u64,
}
impl Startup {
    fn validate(self) -> Result<(), Error> {
        let code = PhysicalRange::new(self.code_physical, STARTUP_PAGE_BYTES, STARTUP_PAGE_BYTES)?;
        let data = PhysicalRange::new(self.data_physical, STARTUP_PAGE_BYTES, STARTUP_PAGE_BYTES)?;
        if code.start() == 0 || code.end() != data.start() || data.end() > 0x10_0000 {
            return Err(Error::OutOfBounds);
        }
        Ok(())
    }
    pub fn decode(input: &[u8]) -> Result<Self, Error> {
        let b = input.get(..STARTUP_BYTES).ok_or(Error::ShortRecord)?;
        let value = Self {
            code_physical: u64::from_le_bytes(wire::read(b, 0)?),
            data_physical: u64::from_le_bytes(wire::read(b, 8)?),
        };
        if u64::from_le_bytes(wire::read(b, 16)?) != STARTUP_PAGE_BYTES
            || u64::from_le_bytes(wire::read(b, 24)?) != STARTUP_PAGE_BYTES
        {
            return Err(Error::InvalidRecordSize);
        }
        if u64::from_le_bytes(wire::read(b, 32)?) != value.code_physical
            || u64::from_le_bytes(wire::read(b, 40)?) != value.data_physical
        {
            return Err(Error::OutOfBounds);
        }
        if b[48..].iter().any(|b| *b != 0) {
            return Err(Error::ReservedBits);
        }
        value.validate()?;
        Ok(value)
    }
    pub fn encode(self, output: &mut [u8]) -> Result<(), Error> {
        self.validate()?;
        let output = output.get_mut(..STARTUP_BYTES).ok_or(Error::ShortRecord)?;
        let mut b = [0; STARTUP_BYTES];
        for (offset, value) in [
            (0, self.code_physical),
            (8, self.data_physical),
            (16, STARTUP_PAGE_BYTES),
            (24, STARTUP_PAGE_BYTES),
            (32, self.code_physical),
            (40, self.data_physical),
        ] {
            wire::write(&mut b, offset, value.to_le_bytes());
        }
        output.copy_from_slice(&b);
        Ok(())
    }
}
impl BootInfo<'_> {
    /// Validate low-memory ownership and unique initial identity mappings.
    /// The mapped bytes themselves are owned by the kernel after firmware exit.
    pub fn validate_startup(&self) -> Result<Startup, Error> {
        let startup = Startup::decode(self.table(STARTUP_KIND).ok_or(Error::MissingBootTable)?)?;
        let maps = self.table(MAPPINGS_KIND).ok_or(Error::MissingBootTable)?;
        for (address, role) in [
            (startup.code_physical, STARTUP_CODE_ROLE),
            (startup.data_physical, STARTUP_DATA_ROLE),
        ] {
            let extent = PhysicalRange::new(address, STARTUP_PAGE_BYTES, STARTUP_PAGE_BYTES)?;
            if extent.overlaps(PhysicalRange::new(
                self.header().self_phys,
                self.header().total_bytes,
                1,
            )?) {
                return Err(Error::Overlap);
            }
            if !(0..self.memory_region_count()).any(|i| {
                self.memory_region(i).is_ok_and(|r| {
                    r.range.contains(extent)
                        && r.firmware_type == 2
                        && r.ownership == Ownership::BootData
                })
            }) {
                return Err(Error::LiveBootDataNotReserved);
            }
            let mut matches = 0;
            for encoded in maps.chunks_exact(MAPPING_BYTES) {
                let mapping = Mapping::decode(encoded)?;
                if mapping.role == role {
                    if mapping.physical != address
                        || mapping.virtual_start != address
                        || mapping.bytes != STARTUP_PAGE_BYTES
                        || mapping.permissions != 6
                    {
                        return Err(Error::OutOfBounds);
                    }
                    matches += 1;
                } else if extent.overlaps(PhysicalRange::new(
                    mapping.physical,
                    mapping.bytes,
                    STARTUP_PAGE_BYTES,
                )?) || extent.overlaps(PhysicalRange::new(
                    mapping.virtual_start,
                    mapping.bytes,
                    STARTUP_PAGE_BYTES,
                )?) {
                    // Do not allow an alias which can remain writable after code promotion.
                    return Err(Error::Overlap);
                }
            }
            if matches != 1 {
                return Err(Error::MissingBootTable);
            }
        }
        Ok(startup)
    }
}
