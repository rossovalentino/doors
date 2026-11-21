//! The only ACPI pointer adapter. All firmware reads finish before final exit.
use door_boot_protocol::{
    Platform,
    platform::{self, MAX_ACPI_TABLE},
};
use uefi::{
    boot::MemoryType,
    mem::memory_map::{MemoryMap, MemoryMapOwned},
    table::cfg::ConfigTableEntry,
};

struct FirmwareTables {
    memory: MemoryMapOwned,
}
impl FirmwareTables {
    fn bytes(&self, physical: u64, length: usize) -> &[u8] {
        assert!(
            physical != 0 && length != 0 && length <= MAX_ACPI_TABLE,
            "ACPI extent"
        );
        let end = physical
            .checked_add(length as u64)
            .expect("ACPI address overflow");
        assert!(end <= 0x1_0000_0000, "ACPI bootstrap physical limit");
        assert!(
            self.memory.entries().any(|descriptor| {
                let end_of_region = descriptor
                    .page_count
                    .checked_mul(4096)
                    .and_then(|bytes| descriptor.phys_start.checked_add(bytes));
                matches!(descriptor.ty.0, 9 | 10)
                    && descriptor.phys_start <= physical
                    && end_of_region.is_some_and(|limit| end <= limit)
            }),
            "ACPI bytes outside firmware ACPI reservation"
        );
        // SAFETY: before ExitBootServices, x64 UEFI identity maps firmware memory.
        // The entire bounded range is covered by the just-acquired ACPI memory map;
        // no allocation or firmware call changes these tables while this view lives.
        unsafe { core::slice::from_raw_parts(physical as *const u8, length) }
    }
    fn table(&self, physical: u64) -> &[u8] {
        let header = self.bytes(physical, 36);
        let length = platform::acpi_table_length(header).expect("ACPI table length");
        let all = self.bytes(physical, length);
        let signature = all[..4].try_into().expect("ACPI signature size");
        platform::acpi_table(all, signature).expect("ACPI table checksum")
    }
}
pub fn acquire() -> Platform {
    let rsdp = uefi::system::with_config_table(|entries| {
        let mut matches = entries
            .iter()
            .filter(|entry| entry.guid == ConfigTableEntry::ACPI2_GUID);
        let address = matches
            .next()
            .expect("ACPI2 configuration table required")
            .address as u64;
        assert!(
            matches.next().is_none(),
            "duplicate ACPI2 configuration table"
        );
        address
    });
    let tables = FirmwareTables {
        memory: uefi::boot::memory_map(MemoryType::LOADER_DATA).expect("ACPI reservation map"),
    };
    let prefix = tables.bytes(rsdp, 36);
    let rsdp_length = u32::from_le_bytes(prefix[20..24].try_into().expect("RSDP length field"));
    assert!((36..=4096).contains(&rsdp_length), "RSDP length limit");
    let (xsdt, rsdp_length) =
        platform::parse_rsdp(tables.bytes(rsdp, rsdp_length as usize)).expect("RSDP validation");
    let xsdt_table = tables.table(xsdt);
    let addresses = platform::xsdt_addresses(xsdt_table).expect("XSDT validation");
    let mut result = Platform::empty();
    result.roots[0] = rsdp;
    result.root_bytes[0] = rsdp_length;
    result.roots[1] = xsdt;
    result.root_bytes[1] = xsdt_table.len() as u32;
    for encoded in addresses.chunks_exact(8) {
        let physical = u64::from_le_bytes(encoded.try_into().expect("XSDT address size"));
        let table = tables.table(physical);
        match &table[..4] {
            b"APIC" => {
                assert_eq!(result.roots[2], 0, "duplicate MADT");
                platform::parse_madt(table, &mut result).expect("MADT validation");
                result.roots[2] = physical;
                result.root_bytes[2] = table.len() as u32;
            }
            b"FACP" => {
                assert_eq!(result.roots[3], 0, "duplicate FADT");
                platform::parse_fadt(table, &mut result).expect("FADT validation");
                result.roots[3] = physical;
                result.root_bytes[3] = table.len() as u32;
            }
            _ => {} // Unused checksummed tables stay firmware-reserved, never evaluated.
        }
    }
    let mut encoded = [0; door_boot_protocol::PLATFORM_BYTES];
    result
        .encode(&mut encoded)
        .expect("complete platform summary");
    // `tables` and its memory-map pool drop here, while boot services are live.
    result
}
