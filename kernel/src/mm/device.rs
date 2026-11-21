//! Checks resource claims against RAM and already-owned boot/platform mappings.
use super::{Error, PAGE_BYTES, PageCount, PhysicalAddress};
use door_boot_protocol::{
    BootInfo, MAPPING_BYTES, MAPPINGS_KIND, Mapping, MemoryRegion, Ownership, PhysicalRange,
};

/// Validate an assigned, size-probed PCI memory BAR before mapping it uncached.
/// The caller must separately establish device identity and exclusive BAR ownership.
/// PCI apertures absent from the firmware memory map are permitted; overlapping
/// RAM, firmware runtime, framebuffer, boot mappings and interrupt controllers are not.
pub fn validate_device_extent(
    info: &BootInfo<'_>,
    physical: PhysicalAddress,
    pages: PageCount,
) -> Result<(), Error> {
    let extent = PhysicalRange::new(physical.get(), pages.bytes().get(), PAGE_BYTES)
        .map_err(|_| Error::Address)?;
    if extent.start() < 0x10_0000 || extent.end() > (1 << 52) {
        return Err(Error::Address);
    }
    for i in 0..info.memory_region_count() {
        let region = info.memory_region(i).map_err(|_| Error::InvalidMemoryMap)?;
        check_region(extent, region)?;
    }
    for bytes in info
        .table(MAPPINGS_KIND)
        .ok_or(Error::InvalidMemoryMap)?
        .chunks_exact(MAPPING_BYTES)
    {
        let mapping = Mapping::decode(bytes).map_err(|_| Error::InvalidMapping)?;
        let used = PhysicalRange::new(mapping.physical, mapping.bytes, PAGE_BYTES)
            .map_err(|_| Error::InvalidMapping)?;
        if extent.overlaps(used) {
            return Err(Error::Overlap);
        }
    }
    let platform = info
        .validate_platform()
        .map_err(|_| Error::InvalidMemoryMap)?;
    for address in core::iter::once(platform.lapic_physical).chain(
        platform.io_apics[..usize::from(platform.io_apic_count)]
            .iter()
            .map(|apic| apic.physical),
    ) {
        let used = PhysicalRange::new(address, PAGE_BYTES, PAGE_BYTES)
            .map_err(|_| Error::InvalidMemoryMap)?;
        if extent.overlaps(used) {
            return Err(Error::Overlap);
        }
    }
    Ok(())
}

fn check_region(extent: PhysicalRange, region: MemoryRegion) -> Result<(), Error> {
    if extent.overlaps(region.range)
        && (!matches!(region.firmware_type, 0 | 11 | 12)
            || region.ownership != Ownership::PermanentlyReserved
            || region.firmware_attributes & (1 << 63) != 0)
    {
        Err(Error::Overlap)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_claims_never_reclassify_ram_or_reserved_live_owners() {
        let extent = PhysicalRange::new(0x8000_0000, 8192, 4096).unwrap();
        for firmware_type in 0..=15 {
            for ownership in [
                Ownership::UsableAfterHandoff,
                Ownership::PermanentlyReserved,
                Ownership::ReclaimableAfterCopy,
                Ownership::KernelModule,
                Ownership::FramebufferMmio,
                Ownership::FirmwareRuntime,
                Ownership::BootData,
            ] {
                let region = MemoryRegion {
                    range: PhysicalRange::new(0x8000_1000, 8192, 4096).unwrap(),
                    firmware_type,
                    ownership,
                    firmware_attributes: 0,
                };
                assert_eq!(
                    check_region(extent, region).is_ok(),
                    matches!(firmware_type, 0 | 11 | 12)
                        && ownership == Ownership::PermanentlyReserved
                );
                assert_eq!(
                    check_region(
                        extent,
                        MemoryRegion {
                            firmware_attributes: 1 << 63,
                            ..region
                        }
                    ),
                    Err(Error::Overlap)
                );
                assert_eq!(
                    check_region(
                        extent,
                        MemoryRegion {
                            range: PhysicalRange::new(extent.end(), 4096, 4096).unwrap(),
                            ..region
                        }
                    ),
                    Ok(())
                );
            }
        }
    }
}
