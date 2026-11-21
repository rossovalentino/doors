use crate::{BootInfo, Error, PhysicalRange, wire};

pub const MODULES_KIND: u32 = 2;
pub const MAPPINGS_KIND: u32 = 3;
pub const FRAMEBUFFER_KIND: u32 = 4;
pub const COMMAND_LINE_KIND: u32 = 6;
pub const MODULE_BYTES: usize = 80;
pub const MAPPING_BYTES: usize = 40;
pub const FRAMEBUFFER_BYTES: usize = 40;

#[derive(Clone, Copy, Debug)]
pub struct Module {
    pub role: u32,
    pub physical: u64,
    pub storage_bytes: u64,
    pub file_bytes: u64,
    pub virtual_start: u64,
    pub virtual_bytes: u64,
    pub sha256: [u8; 32],
}
impl Module {
    pub fn decode(b: &[u8]) -> Result<Self, Error> {
        let b = b.get(..MODULE_BYTES).ok_or(Error::ShortRecord)?;
        let role = u32::from_le_bytes(wire::read(b, 0)?);
        if !matches!(role, 1 | 2) || wire::read::<4>(b, 4)? != [0; 4] {
            return Err(Error::ReservedBits);
        }
        let value = Self {
            role,
            physical: u64::from_le_bytes(wire::read(b, 8)?),
            storage_bytes: u64::from_le_bytes(wire::read(b, 16)?),
            file_bytes: u64::from_le_bytes(wire::read(b, 24)?),
            virtual_start: u64::from_le_bytes(wire::read(b, 32)?),
            virtual_bytes: u64::from_le_bytes(wire::read(b, 40)?),
            sha256: wire::read(b, 48)?,
        };
        PhysicalRange::new(value.physical, value.storage_bytes, 4096)?;
        if value.file_bytes == 0
            || value.file_bytes > value.storage_bytes
            || value.sha256 == [0; 32]
        {
            return Err(Error::OutOfBounds);
        }
        PhysicalRange::new(value.virtual_start, value.virtual_bytes, 4096)?;
        Ok(value)
    }
    pub fn encode(self, b: &mut [u8]) -> Result<(), Error> {
        let output = b.get_mut(..MODULE_BYTES).ok_or(Error::ShortRecord)?;
        let mut encoded = [0; MODULE_BYTES];
        let b = &mut encoded;
        wire::write(b, 0, self.role.to_le_bytes());
        for (offset, value) in [
            (8, self.physical),
            (16, self.storage_bytes),
            (24, self.file_bytes),
            (32, self.virtual_start),
            (40, self.virtual_bytes),
        ] {
            wire::write(b, offset, value.to_le_bytes());
        }
        wire::write(b, 48, self.sha256);
        Self::decode(b)?;
        output.copy_from_slice(b);
        Ok(())
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Mapping {
    pub virtual_start: u64,
    pub physical: u64,
    pub bytes: u64,
    pub permissions: u32,
    pub role: u32,
}
impl Mapping {
    pub fn decode(b: &[u8]) -> Result<Self, Error> {
        let b = b.get(..MAPPING_BYTES).ok_or(Error::ShortRecord)?;
        if wire::read::<8>(b, 32)? != [0; 8] {
            return Err(Error::ReservedBits);
        }
        let v = Self {
            virtual_start: u64::from_le_bytes(wire::read(b, 0)?),
            physical: u64::from_le_bytes(wire::read(b, 8)?),
            bytes: u64::from_le_bytes(wire::read(b, 16)?),
            permissions: u32::from_le_bytes(wire::read(b, 24)?),
            role: u32::from_le_bytes(wire::read(b, 28)?),
        };
        let range = PhysicalRange::new(v.virtual_start, v.bytes, 4096)?;
        PhysicalRange::new(v.physical, v.bytes, 4096)?;
        if !matches!(v.permissions, 4..=6)
            || !(1..=10).contains(&v.role)
            || !canonical(range.start())
            || !canonical(range.end() - 1)
            || (range.start() < 0x8000_0000_0000 && range.end() > 0x8000_0000_0000)
        {
            return Err(Error::OutOfBounds);
        }
        Ok(v)
    }
    pub fn encode(self, b: &mut [u8]) -> Result<(), Error> {
        let output = b.get_mut(..MAPPING_BYTES).ok_or(Error::ShortRecord)?;
        let mut encoded = [0; MAPPING_BYTES];
        let b = &mut encoded;
        wire::write(b, 0, self.virtual_start.to_le_bytes());
        wire::write(b, 8, self.physical.to_le_bytes());
        wire::write(b, 16, self.bytes.to_le_bytes());
        wire::write(b, 24, self.permissions.to_le_bytes());
        wire::write(b, 28, self.role.to_le_bytes());
        Self::decode(b)?;
        output.copy_from_slice(b);
        Ok(())
    }
}
fn canonical(v: u64) -> bool {
    !(0x8000_0000_0000..0xffff_8000_0000_0000).contains(&v)
}

#[derive(Clone, Copy, Debug)]
pub struct Framebuffer {
    pub physical: u64,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: u32,
    pub virtual_start: u64,
}
impl Framebuffer {
    pub fn decode(b: &[u8]) -> Result<Self, Error> {
        let b = b.get(..FRAMEBUFFER_BYTES).ok_or(Error::ShortRecord)?;
        let v = Self {
            physical: u64::from_le_bytes(wire::read(b, 0)?),
            bytes: u64::from_le_bytes(wire::read(b, 8)?),
            width: u32::from_le_bytes(wire::read(b, 16)?),
            height: u32::from_le_bytes(wire::read(b, 20)?),
            stride: u32::from_le_bytes(wire::read(b, 24)?),
            format: u32::from_le_bytes(wire::read(b, 28)?),
            virtual_start: u64::from_le_bytes(wire::read(b, 32)?),
        };
        let needed = u64::from(v.stride)
            .checked_mul(u64::from(v.height))
            .and_then(|n| n.checked_mul(4))
            .ok_or(Error::Overflow)?;
        if v.width == 0
            || v.height == 0
            || v.width > 4096
            || v.height > 4096
            || v.stride < v.width
            || v.stride > 8192
            || needed > v.bytes
            || v.bytes > 512 * 1024 * 1024
            || !matches!(v.format, 0 | 1)
            || !v.physical.is_multiple_of(4096)
            || v.virtual_start != v.physical
        {
            return Err(Error::OutOfBounds);
        }
        v.physical.checked_add(v.bytes).ok_or(Error::Overflow)?;
        Ok(v)
    }
    pub fn encode(self, b: &mut [u8]) -> Result<(), Error> {
        let output = b.get_mut(..FRAMEBUFFER_BYTES).ok_or(Error::ShortRecord)?;
        let mut encoded = [0; FRAMEBUFFER_BYTES];
        let b = &mut encoded;
        wire::write(b, 0, self.physical.to_le_bytes());
        wire::write(b, 8, self.bytes.to_le_bytes());
        wire::write(b, 16, self.width.to_le_bytes());
        wire::write(b, 20, self.height.to_le_bytes());
        wire::write(b, 24, self.stride.to_le_bytes());
        wire::write(b, 28, self.format.to_le_bytes());
        wire::write(b, 32, self.virtual_start.to_le_bytes());
        Self::decode(b)?;
        output.copy_from_slice(b);
        Ok(())
    }
}

impl<'a> BootInfo<'a> {
    pub fn table(&self, kind: u32) -> Option<&'a [u8]> {
        (0..self.header.table_count).find_map(|index| {
            let descriptor = Self::descriptor(self.bytes, self.header, index).ok()?;
            (descriptor.kind == kind
                && (descriptor.revision == 1 || kind == 5 && descriptor.revision == 2))
                .then(|| {
                    &self.bytes[descriptor.offset as usize
                        ..(descriptor.offset + descriptor.length) as usize]
                })
        })
    }
    pub fn module(&self, role: u32) -> Result<Module, Error> {
        self.table(MODULES_KIND)
            .ok_or(Error::MissingBootTable)?
            .chunks_exact(MODULE_BYTES)
            .find_map(|b| Module::decode(b).ok().filter(|m| m.role == role))
            .ok_or(Error::MissingBootTable)
    }
    pub fn framebuffer(&self) -> Result<Framebuffer, Error> {
        Framebuffer::decode(
            self.table(FRAMEBUFFER_KIND)
                .ok_or(Error::MissingBootTable)?,
        )
    }
    pub fn validate_boot(&self) -> Result<(), Error> {
        for kind in [MODULES_KIND, MAPPINGS_KIND, FRAMEBUFFER_KIND] {
            if self.table(kind).is_none() {
                return Err(Error::MissingBootTable);
            }
        }
        let modules = self.table(MODULES_KIND).ok_or(Error::MissingBootTable)?;
        if modules.len() != 2 * MODULE_BYTES {
            return Err(Error::InvalidRecordSize);
        }
        let kernel = self.module(1)?;
        let initfs = self.module(2)?;
        if kernel.virtual_start < 0xffff_ffff_8000_0000
            || initfs.virtual_start != initfs.physical
            || initfs.virtual_bytes != initfs.storage_bytes
        {
            return Err(Error::OutOfBounds);
        }
        let maps = self.table(MAPPINGS_KIND).ok_or(Error::MissingBootTable)?;
        if maps.len() / MAPPING_BYTES > 128 {
            return Err(Error::LimitExceeded);
        }
        for (index, b) in maps.chunks_exact(MAPPING_BYTES).enumerate() {
            let m = Mapping::decode(b)?;
            let physical = PhysicalRange::new(m.physical, m.bytes, 4096)?;
            if m.role != 7
                && !(0..self.memory_region_count()).any(|i| {
                    self.memory_region(i).is_ok_and(|r| {
                        r.range.contains(physical) && r.ownership.protects_boot_data()
                    })
                })
            {
                return Err(Error::LiveBootDataNotReserved);
            }
            for prior in maps[..index * MAPPING_BYTES].chunks_exact(MAPPING_BYTES) {
                let p = Mapping::decode(prior)?;
                if PhysicalRange::new(m.virtual_start, m.bytes, 4096)?.overlaps(PhysicalRange::new(
                    p.virtual_start,
                    p.bytes,
                    4096,
                )?) {
                    return Err(Error::Overlap);
                }
                if physical.overlaps(PhysicalRange::new(p.physical, p.bytes, 4096)?)
                    && ((m.permissions == 5 && p.permissions == 6)
                        || (m.permissions == 6 && p.permissions == 5))
                {
                    return Err(Error::Overlap);
                }
            }
        }
        for module in [kernel, initfs] {
            let backing = PhysicalRange::new(module.physical, module.storage_bytes, 4096)?;
            if !(0..self.memory_region_count()).any(|i| {
                self.memory_region(i)
                    .is_ok_and(|r| r.range.contains(backing) && r.ownership.protects_boot_data())
            }) {
                return Err(Error::LiveBootDataNotReserved);
            }
        }
        let live = PhysicalRange::new(self.header.self_phys, self.header.total_bytes, 1)?;
        if !maps.chunks_exact(MAPPING_BYTES).any(|b| {
            Mapping::decode(b).is_ok_and(|m| {
                m.role == 8
                    && m.permissions == 4
                    && m.virtual_start == kernel.physical
                    && m.physical == kernel.physical
                    && m.bytes >= kernel.file_bytes
            })
        }) {
            return Err(Error::MissingBootTable);
        }
        if !maps.chunks_exact(MAPPING_BYTES).any(|b| {
            Mapping::decode(b).is_ok_and(|m| {
                m.role == 1
                    && m.permissions == 5
                    && m.virtual_start <= kernel.virtual_start
                    && m.virtual_start + m.bytes > kernel.virtual_start
            })
        }) {
            return Err(Error::MissingBootTable);
        }
        if !maps.chunks_exact(MAPPING_BYTES).any(|b| {
            Mapping::decode(b)
                .is_ok_and(|m| m.role == 4 && m.permissions == 6 && m.bytes >= 64 * 1024)
        }) {
            return Err(Error::MissingBootTable);
        }
        if !maps.chunks_exact(MAPPING_BYTES).any(|b| {
            Mapping::decode(b).is_ok_and(|m| {
                m.role == 3
                    && m.permissions == 4
                    && m.virtual_start == m.physical
                    && PhysicalRange::new(m.physical, m.bytes, 4096).is_ok_and(|r| r.contains(live))
            })
        }) {
            return Err(Error::MissingBootTable);
        }
        if !maps.chunks_exact(MAPPING_BYTES).any(|b| {
            Mapping::decode(b).is_ok_and(|m| {
                m.role == 2
                    && m.permissions == 4
                    && m.physical == initfs.physical
                    && m.virtual_start == initfs.virtual_start
                    && m.bytes >= initfs.file_bytes
            })
        }) {
            return Err(Error::MissingBootTable);
        }
        let fb = self.framebuffer()?;
        if !maps.chunks_exact(MAPPING_BYTES).any(|b| {
            Mapping::decode(b).is_ok_and(|m| {
                m.role == 7
                    && m.permissions == 6
                    && m.physical == fb.physical
                    && m.virtual_start == fb.virtual_start
                    && m.bytes >= fb.bytes
            })
        }) {
            return Err(Error::MissingBootTable);
        }
        if self.table(crate::STARTUP_KIND).is_some() {
            self.validate_startup()?;
        }
        Ok(())
    }
}
