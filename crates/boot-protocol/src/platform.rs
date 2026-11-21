//! Bounded ACPI readers and the owned Door platform summary. No physical dereference.
use crate::{BootInfo, Error, Ownership, PhysicalRange, wire};

pub const PLATFORM_KIND: u32 = 5;
pub const PLATFORM_BYTES: usize = 1024;
pub const MAX_PROCESSORS: usize = 64;
pub const MAX_IO_APICS: usize = 8;
pub const MAX_ACPI_TABLE: usize = 64 * 1024;
pub const MAX_XSDT_ENTRIES: usize = 128;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Processor {
    pub acpi_uid: u32,
    pub apic_id: u32,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IoApic {
    pub id: u32,
    pub physical: u64,
    pub gsi_base: u32,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Platform {
    /// RSDP, XSDT, MADT and FADT in that order. Addresses are provenance, not pointers.
    pub roots: [u64; 4],
    pub root_bytes: [u32; 4],
    pub lapic_physical: u64,
    pub processor_count: u16,
    pub io_apic_count: u16,
    pub processors: [Processor; MAX_PROCESSORS],
    pub io_apics: [IoApic; MAX_IO_APICS],
    pub pm_timer_port: u16,
    pub pm_timer_bits: u8,
    pub pcat_compat: bool,
    pub legacy_devices: bool,
    /// OEM ACPI Preferred_PM_Profile (0 unspecified, 1..8 defined by ACPI).
    pub power_profile: u8,
}
impl Platform {
    pub const fn empty() -> Self {
        Self {
            roots: [0; 4],
            root_bytes: [0; 4],
            lapic_physical: 0,
            processor_count: 0,
            io_apic_count: 0,
            processors: [Processor {
                acpi_uid: 0,
                apic_id: 0,
            }; MAX_PROCESSORS],
            io_apics: [IoApic {
                id: 0,
                physical: 0,
                gsi_base: 0,
            }; MAX_IO_APICS],
            pm_timer_port: 0,
            pm_timer_bits: 0,
            pcat_compat: false,
            legacy_devices: false,
            power_profile: 0,
        }
    }
    fn validate(&self) -> Result<(), Error> {
        if self.power_profile > 8 {
            return Err(Error::ReservedBits);
        }
        if self.processor_count == 0
            || usize::from(self.processor_count) > MAX_PROCESSORS
            || self.io_apic_count == 0
            || usize::from(self.io_apic_count) > MAX_IO_APICS
        {
            return Err(Error::LimitExceeded);
        }
        mmio(self.lapic_physical)?;
        for index in 0..4 {
            let minimum = [36, 44, 44, 244][index];
            if self.root_bytes[index] < minimum || self.root_bytes[index] as usize > MAX_ACPI_TABLE
            {
                return Err(Error::InvalidRecordSize);
            }
            let range =
                PhysicalRange::new(self.roots[index], u64::from(self.root_bytes[index]), 1)?;
            if range.start() == 0 || range.end() > 0x1_0000_0000 {
                return Err(Error::OutOfBounds);
            }
            for other in 0..index {
                if range.overlaps(PhysicalRange::new(
                    self.roots[other],
                    u64::from(self.root_bytes[other]),
                    1,
                )?) {
                    return Err(Error::Overlap);
                }
            }
        }
        for (index, cpu) in self.processors.iter().enumerate() {
            if index >= usize::from(self.processor_count) {
                if *cpu != Processor::default() {
                    return Err(Error::ReservedBits);
                }
            } else if cpu.apic_id > 254
                || self.processors[..index]
                    .iter()
                    .any(|p| p.apic_id == cpu.apic_id || p.acpi_uid == cpu.acpi_uid)
            {
                return Err(Error::OutOfBounds);
            }
        }
        for (index, io) in self.io_apics.iter().enumerate() {
            if index >= usize::from(self.io_apic_count) {
                if *io != IoApic::default() {
                    return Err(Error::ReservedBits);
                }
            } else {
                mmio(io.physical)?;
                if io.id > 255
                    || io.physical == self.lapic_physical
                    || self.io_apics[..index].iter().any(|p| {
                        p.id == io.id || p.physical == io.physical || p.gsi_base == io.gsi_base
                    })
                {
                    return Err(Error::OutOfBounds);
                }
            }
        }
        if !matches!(
            (self.pm_timer_port, self.pm_timer_bits),
            (0, 0) | (1..=65532, 24 | 32)
        ) {
            return Err(Error::OutOfBounds);
        }
        Ok(())
    }
    pub fn decode(input: &[u8]) -> Result<Self, Error> {
        let b = input.get(..PLATFORM_BYTES).ok_or(Error::ShortRecord)?;
        let mut p = Self::empty();
        for i in 0..4 {
            p.roots[i] = u64::from_le_bytes(wire::read(b, i * 8)?);
            p.root_bytes[i] = u32::from_le_bytes(wire::read(b, 32 + i * 4)?);
        }
        p.lapic_physical = u64::from_le_bytes(wire::read(b, 48)?);
        let flags = u32::from_le_bytes(wire::read(b, 56)?);
        if flags & !3 != 0 || b[68..80].iter().any(|b| *b != 0) || b[720..].iter().any(|b| *b != 0)
        {
            return Err(Error::ReservedBits);
        }
        p.pcat_compat = flags & 1 != 0;
        p.legacy_devices = flags & 2 != 0;
        p.processor_count = u16::from_le_bytes(wire::read(b, 60)?);
        p.io_apic_count = u16::from_le_bytes(wire::read(b, 62)?);
        p.pm_timer_port = u16::from_le_bytes(wire::read(b, 64)?);
        p.pm_timer_bits = b[66];
        p.power_profile = b[67];
        for (i, cpu) in p.processors.iter_mut().enumerate() {
            cpu.acpi_uid = u32::from_le_bytes(wire::read(b, 80 + i * 8)?);
            cpu.apic_id = u32::from_le_bytes(wire::read(b, 84 + i * 8)?);
        }
        for (i, io) in p.io_apics.iter_mut().enumerate() {
            io.id = u32::from_le_bytes(wire::read(b, 592 + i * 16)?);
            io.gsi_base = u32::from_le_bytes(wire::read(b, 596 + i * 16)?);
            io.physical = u64::from_le_bytes(wire::read(b, 600 + i * 16)?);
        }
        p.validate()?;
        Ok(p)
    }
    pub fn encode(&self, output: &mut [u8]) -> Result<(), Error> {
        self.validate()?;
        let output = output.get_mut(..PLATFORM_BYTES).ok_or(Error::ShortRecord)?;
        let mut b = [0; PLATFORM_BYTES];
        for i in 0..4 {
            wire::write(&mut b, i * 8, self.roots[i].to_le_bytes());
            wire::write(&mut b, 32 + i * 4, self.root_bytes[i].to_le_bytes());
        }
        wire::write(&mut b, 48, self.lapic_physical.to_le_bytes());
        wire::write(
            &mut b,
            56,
            (u32::from(self.pcat_compat) | (u32::from(self.legacy_devices) << 1)).to_le_bytes(),
        );
        wire::write(&mut b, 60, self.processor_count.to_le_bytes());
        wire::write(&mut b, 62, self.io_apic_count.to_le_bytes());
        wire::write(&mut b, 64, self.pm_timer_port.to_le_bytes());
        b[66] = self.pm_timer_bits;
        b[67] = self.power_profile;
        for (i, cpu) in self.processors.iter().enumerate() {
            wire::write(&mut b, 80 + i * 8, cpu.acpi_uid.to_le_bytes());
            wire::write(&mut b, 84 + i * 8, cpu.apic_id.to_le_bytes());
        }
        for (i, io) in self.io_apics.iter().enumerate() {
            wire::write(&mut b, 592 + i * 16, io.id.to_le_bytes());
            wire::write(&mut b, 596 + i * 16, io.gsi_base.to_le_bytes());
            wire::write(&mut b, 600 + i * 16, io.physical.to_le_bytes());
        }
        output.copy_from_slice(&b);
        Ok(())
    }
}
impl BootInfo<'_> {
    /// Validates summary and retained firmware storage against the final memory map.
    /// Root addresses are never dereferenced and are not required to remain mapped.
    pub fn validate_platform(&self) -> Result<Platform, Error> {
        let p = Platform::decode(self.table(PLATFORM_KIND).ok_or(Error::MissingBootTable)?)?;
        for index in 0..4 {
            let range = PhysicalRange::new(p.roots[index], u64::from(p.root_bytes[index]), 1)?;
            if !(0..self.memory_region_count()).any(|i| {
                self.memory_region(i).is_ok_and(|r| {
                    r.range.contains(range)
                        && matches!(r.firmware_type, 9 | 10)
                        && matches!(
                            r.ownership,
                            Ownership::PermanentlyReserved | Ownership::FirmwareRuntime
                        )
                })
            }) {
                return Err(Error::LiveBootDataNotReserved);
            }
        }
        for physical in core::iter::once(p.lapic_physical).chain(
            p.io_apics[..usize::from(p.io_apic_count)]
                .iter()
                .map(|io| io.physical),
        ) {
            let device = PhysicalRange::new(physical, 4096, 4096)?;
            if (0..self.memory_region_count()).any(|i| {
                self.memory_region(i)
                    .is_ok_and(|r| device.overlaps(r.range) && matches!(r.firmware_type, 1..=10))
            }) {
                return Err(Error::Overlap);
            }
        }
        Ok(p)
    }
}
fn mmio(address: u64) -> Result<(), Error> {
    let r = PhysicalRange::new(address, 4096, 4096)?;
    if address == 0 || r.end() > 0x1_0000_0000 {
        return Err(Error::OutOfBounds);
    }
    Ok(())
}
fn checksum(bytes: &[u8]) -> Result<(), Error> {
    if bytes.iter().fold(0u8, |sum, b| sum.wrapping_add(*b)) != 0 {
        return Err(Error::BadChecksum);
    }
    Ok(())
}
/// Parses ACPI 2+ RSDP; the current UEFI profile deliberately has no RSDT fallback.
pub fn parse_rsdp(input: &[u8]) -> Result<(u64, u32), Error> {
    let b = input.get(..36).ok_or(Error::ShortRecord)?;
    if &b[..8] != b"RSD PTR " {
        return Err(Error::BadMagic);
    }
    if b[15] < 2 {
        return Err(Error::UnsupportedVersion);
    }
    checksum(&b[..20])?;
    let length = u32::from_le_bytes(wire::read(b, 20)?);
    if !(36..=4096).contains(&length) {
        return Err(Error::LimitExceeded);
    }
    let whole = input.get(..length as usize).ok_or(Error::ShortRecord)?;
    checksum(whole)?;
    if b[33..36] != [0; 3] {
        return Err(Error::ReservedBits);
    }
    let address = u64::from_le_bytes(wire::read(b, 24)?);
    if address == 0 || address >= 0x1_0000_0000 {
        return Err(Error::OutOfBounds);
    }
    Ok((address, length))
}
/// Header-only length bound, before the firmware adapter reads the complete table.
pub fn acpi_table_length(header: &[u8]) -> Result<usize, Error> {
    header.get(..36).ok_or(Error::ShortRecord)?;
    let length = u32::from_le_bytes(wire::read(header, 4)?) as usize;
    if !(36..=MAX_ACPI_TABLE).contains(&length) {
        return Err(Error::LimitExceeded);
    }
    Ok(length)
}
pub fn acpi_table<'a>(input: &'a [u8], signature: &[u8; 4]) -> Result<&'a [u8], Error> {
    let length = acpi_table_length(input)?;
    let table = input.get(..length).ok_or(Error::ShortRecord)?;
    if &table[..4] != signature {
        return Err(Error::BadMagic);
    }
    checksum(table)?;
    Ok(table)
}
pub fn xsdt_addresses(input: &[u8]) -> Result<&[u8], Error> {
    let b = acpi_table(input, b"XSDT")?;
    if b[8] != 1 || !(b.len() - 36).is_multiple_of(8) || b.len() == 36 {
        return Err(Error::InvalidRecordSize);
    }
    let entries = &b[36..];
    if entries.len() / 8 > MAX_XSDT_ENTRIES {
        return Err(Error::LimitExceeded);
    }
    for (i, entry) in entries.chunks_exact(8).enumerate() {
        let address = u64::from_le_bytes(entry.try_into().map_err(|_| Error::ShortRecord)?);
        if address == 0
            || address >= 0x1_0000_0000
            || entries[..i * 8].chunks_exact(8).any(|p| p == entry)
        {
            return Err(Error::OutOfBounds);
        }
    }
    Ok(entries)
}
/// Copies only enabled processor and I/O controller facts. ISA routes/NMIs/AML are not enabled here.
pub fn parse_madt(input: &[u8], platform: &mut Platform) -> Result<(), Error> {
    let b = acpi_table(input, b"APIC")?;
    b.get(..44).ok_or(Error::ShortRecord)?;
    let mut p = *platform;
    p.processor_count = 0;
    p.io_apic_count = 0;
    p.processors.fill(Processor::default());
    p.io_apics.fill(IoApic::default());
    p.lapic_physical = u64::from(u32::from_le_bytes(wire::read(b, 36)?));
    let flags = u32::from_le_bytes(wire::read(b, 40)?);
    if flags & !1 != 0 {
        return Err(Error::ReservedBits);
    }
    p.pcat_compat = flags & 1 != 0;
    let mut offset = 44;
    let mut override_seen = false;
    let mut known = [Processor::default(); MAX_PROCESSORS];
    let mut known_count = 0;
    while offset < b.len() {
        let header = b.get(offset..offset + 2).ok_or(Error::ShortRecord)?;
        let length = usize::from(header[1]);
        if length < 2 {
            return Err(Error::InvalidRecordSize);
        }
        let e = b.get(offset..offset + length).ok_or(Error::ShortRecord)?;
        match e[0] {
            0 | 9 => {
                let (cpu, flags) = if e[0] == 0 {
                    if length != 8 {
                        return Err(Error::InvalidRecordSize);
                    }
                    (
                        Processor {
                            acpi_uid: u32::from(e[2]),
                            apic_id: u32::from(e[3]),
                        },
                        u32::from_le_bytes(wire::read(e, 4)?),
                    )
                } else {
                    if length != 16 || e[2..4] != [0; 2] {
                        return Err(Error::InvalidRecordSize);
                    }
                    (
                        Processor {
                            acpi_uid: u32::from_le_bytes(wire::read(e, 12)?),
                            apic_id: u32::from_le_bytes(wire::read(e, 4)?),
                        },
                        u32::from_le_bytes(wire::read(e, 8)?),
                    )
                };
                if flags & !3 != 0 || flags == 3 {
                    return Err(Error::ReservedBits);
                }
                // ACPI declares entries with both bits clear unusable; ignore their identity.
                if flags != 0 {
                    if known_count == MAX_PROCESSORS {
                        return Err(Error::LimitExceeded);
                    }
                    if known[..known_count]
                        .iter()
                        .any(|c| c.apic_id == cpu.apic_id || c.acpi_uid == cpu.acpi_uid)
                    {
                        return Err(Error::Overlap);
                    }
                    known[known_count] = cpu;
                    known_count += 1;
                    if flags & 1 != 0 {
                        if cpu.apic_id > 254 {
                            return Err(Error::OutOfBounds);
                        }
                        p.processors[usize::from(p.processor_count)] = cpu;
                        p.processor_count += 1;
                    }
                }
            }
            1 => {
                if length != 12 || e[3] != 0 {
                    return Err(Error::InvalidRecordSize);
                }
                if usize::from(p.io_apic_count) == MAX_IO_APICS {
                    return Err(Error::LimitExceeded);
                }
                let io = IoApic {
                    id: u32::from(e[2]),
                    physical: u64::from(u32::from_le_bytes(wire::read(e, 4)?)),
                    gsi_base: u32::from_le_bytes(wire::read(e, 8)?),
                };
                mmio(io.physical)?;
                if p.io_apics[..usize::from(p.io_apic_count)]
                    .iter()
                    .any(|prior| {
                        prior.id == io.id
                            || prior.physical == io.physical
                            || prior.gsi_base == io.gsi_base
                    })
                {
                    return Err(Error::Overlap);
                }
                p.io_apics[usize::from(p.io_apic_count)] = io;
                p.io_apic_count += 1;
            }
            5 => {
                if length != 12 || e[2..4] != [0; 2] {
                    return Err(Error::InvalidRecordSize);
                }
                if override_seen {
                    return Err(Error::Overlap);
                }
                override_seen = true;
                p.lapic_physical = u64::from_le_bytes(wire::read(e, 4)?);
            }
            _ => {} // Unknown/unused types are skipped only after bounded length validation.
        }
        offset += length;
    }
    mmio(p.lapic_physical)?;
    if p.processor_count == 0 || p.io_apic_count == 0 {
        return Err(Error::MissingBootTable);
    }
    *platform = p;
    Ok(())
}
/// ACPI 2+ FADT. A supported system-I/O GAS takes precedence over the legacy port.
pub fn parse_fadt(input: &[u8], platform: &mut Platform) -> Result<(), Error> {
    let b = acpi_table(input, b"FACP")?;
    b.get(..116).ok_or(Error::ShortRecord)?;
    if b[8] < 3 {
        return Err(Error::UnsupportedVersion);
    }
    let minimum = match b[8] {
        3 | 4 => 244,
        5 => 268,
        _ => 276,
    };
    if b.len() < minimum {
        return Err(Error::ShortRecord);
    }
    let flags = u32::from_le_bytes(wire::read(b, 112)?);
    let mut port = u64::from(u32::from_le_bytes(wire::read(b, 76)?));
    if flags & (1 << 20) != 0 {
        port = 0;
    }
    // Hardware-reduced ACPI has no fixed timer.
    else if b.len() >= 220 {
        let gas = &b[208..220];
        let address = u64::from_le_bytes(wire::read(gas, 4)?);
        if address != 0
            && gas[0] == 1
            && matches!(gas[1], 24 | 32)
            && gas[2] == 0
            && matches!(gas[3], 0 | 3)
            && address <= 65532
        {
            port = address;
        }
    }
    let (port, bits) = if port == 0 {
        (0, 0)
    } else {
        if port > 65532 || b[91] != 4 {
            return Err(Error::OutOfBounds);
        }
        (port as u16, if flags & (1 << 8) != 0 { 32 } else { 24 })
    };
    platform.pm_timer_port = port;
    platform.pm_timer_bits = bits;
    platform.legacy_devices = u16::from_le_bytes(wire::read(b, 109)?) & 1 != 0;
    // Reserved future OEM profiles are unknown, never a guessed chassis.
    platform.power_profile = if b[45] <= 8 { b[45] } else { 0 };
    Ok(())
}
