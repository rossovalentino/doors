use door_boot_protocol::{platform::*, *};

fn sum(b: &mut [u8], checksum: usize) {
    b[checksum] = 0;
    b[checksum] = 0u8.wrapping_sub(b.iter().fold(0u8, |a, b| a.wrapping_add(*b)));
}
fn table(signature: &[u8; 4], length: usize, revision: u8) -> Vec<u8> {
    let mut b = vec![0; length];
    b[..4].copy_from_slice(signature);
    b[4..8].copy_from_slice(&(length as u32).to_le_bytes());
    b[8] = revision;
    sum(&mut b, 9);
    b
}
fn madt(entries: &[u8]) -> Vec<u8> {
    let mut b = table(b"APIC", 44 + entries.len(), 5);
    b[36..40].copy_from_slice(&0xfee0_0000u32.to_le_bytes());
    b[40] = 1;
    b[44..].copy_from_slice(entries);
    sum(&mut b, 9);
    b
}
const CPU: [u8; 8] = [0, 8, 3, 2, 1, 0, 0, 0];
const IO: [u8; 12] = [1, 12, 0, 0, 0, 0, 0xc0, 0xfe, 0, 0, 0, 0];
fn fadt() -> Vec<u8> {
    let mut b = table(b"FACP", 244, 3);
    b[76..80].copy_from_slice(&0x608u32.to_le_bytes());
    b[91] = 4;
    b[109] = 1;
    sum(&mut b, 9);
    b
}
fn platform() -> Platform {
    let mut p = Platform::empty();
    let m = madt(&[&CPU[..], &IO[..]].concat());
    parse_madt(&m, &mut p).unwrap();
    parse_fadt(&fadt(), &mut p).unwrap();
    p.roots = [0x1000, 0x2000, 0x3000, 0x4000];
    p.root_bytes = [36, 52, m.len() as u32, 244];
    p
}
fn boot_blob(p: &Platform) -> Vec<u8> {
    let length = 128 + PLATFORM_BYTES + 80;
    let mut b = vec![0; length];
    Header {
        total_bytes: length as u64,
        table_count: 2,
        tables_offset: 64,
        self_phys: 0x100000,
    }
    .encode(&mut b)
    .unwrap();
    for (index, (kind, offset, bytes, entry)) in [
        (PLATFORM_KIND, 128, PLATFORM_BYTES, PLATFORM_BYTES),
        (MEMORY_REGIONS_KIND, 128 + PLATFORM_BYTES, 80, 40),
    ]
    .into_iter()
    .enumerate()
    {
        TableDescriptor {
            kind,
            revision: 1,
            flags: REQUIRED,
            offset: offset as u64,
            length: bytes as u64,
            entry_bytes: entry as u32,
        }
        .encode(&mut b[64 + index * 32..], length as u64)
        .unwrap();
    }
    p.encode(&mut b[128..]).unwrap();
    for (index, (start, bytes, ty, ownership)) in [
        (0x1000, 0x4000, 9, Ownership::PermanentlyReserved),
        (0x100000, 0x1000, 2, Ownership::BootData),
    ]
    .into_iter()
    .enumerate()
    {
        MemoryRegion {
            range: PhysicalRange::new(start, bytes, 4096).unwrap(),
            firmware_attributes: 0,
            firmware_type: ty,
            ownership,
        }
        .encode(&mut b[128 + PLATFORM_BYTES + index * 40..])
        .unwrap();
    }
    b
}
#[test]
fn rsdp_checks_both_checksums_and_lengths_before_root_use() {
    let mut b = vec![0; 36];
    b[..8].copy_from_slice(b"RSD PTR ");
    b[15] = 2;
    b[20..24].copy_from_slice(&36u32.to_le_bytes());
    b[24..32].copy_from_slice(&0x2000u64.to_le_bytes());
    sum(&mut b[..20], 8);
    sum(&mut b, 32);
    assert_eq!(parse_rsdp(&b), Ok((0x2000, 36)));
    for length in 0..36 {
        assert!(parse_rsdp(&b[..length]).is_err());
    }
    let mut bad = b.clone();
    bad[9] ^= 1;
    assert_eq!(parse_rsdp(&bad), Err(Error::BadChecksum));
    let mut bad = b.clone();
    bad[32] ^= 1;
    assert_eq!(parse_rsdp(&bad), Err(Error::BadChecksum));
    let mut bad = b.clone();
    bad[15] = 0;
    assert_eq!(parse_rsdp(&bad), Err(Error::UnsupportedVersion));
    for length in [0u32, 35, 4097, u32::MAX] {
        let mut bad = b.clone();
        bad[20..24].copy_from_slice(&length.to_le_bytes());
        assert!(parse_rsdp(&bad).is_err());
    }
}
#[test]
fn xsdt_rejects_partial_duplicates_null_high_and_excessive_pointers() {
    let mut b = table(b"XSDT", 52, 1);
    b[36..44].copy_from_slice(&0x3000u64.to_le_bytes());
    b[44..52].copy_from_slice(&0x4000u64.to_le_bytes());
    sum(&mut b, 9);
    assert_eq!(xsdt_addresses(&b).unwrap().len(), 16);
    for address in [0u64, 0x3000, 0x1_0000_0000] {
        let mut bad = b.clone();
        bad[44..52].copy_from_slice(&address.to_le_bytes());
        sum(&mut bad, 9);
        assert!(xsdt_addresses(&bad).is_err());
    }
    assert!(xsdt_addresses(&table(b"XSDT", 45, 1)).is_err());
    assert!(xsdt_addresses(&table(b"XSDT", 36 + 129 * 8, 1)).is_err());
    for length in 0..b.len() {
        assert!(xsdt_addresses(&b[..length]).is_err());
    }
    let mut bad = b;
    bad[9] ^= 1;
    assert_eq!(xsdt_addresses(&bad), Err(Error::BadChecksum));
}
#[test]
fn madt_copies_enabled_processors_and_skips_bounded_unknown_entries() {
    let disabled = [0, 8, 99, 44, 0, 0, 0, 0];
    let b = madt(&[&CPU[..], &disabled[..], &[0x80, 4, 1, 2], &IO[..]].concat());
    let mut p = Platform::empty();
    parse_madt(&b, &mut p).unwrap();
    assert_eq!(p.processor_count, 1);
    assert_eq!(
        p.processors[0],
        Processor {
            acpi_uid: 3,
            apic_id: 2
        }
    );
    assert_eq!(p.io_apic_count, 1);
    assert_eq!(p.io_apics[0].physical, 0xfec00000);
    assert!(p.pcat_compat);
    for length in 0..b.len() {
        assert!(parse_madt(&b[..length], &mut p).is_err());
    }
}
#[test]
fn malformed_or_duplicate_madt_records_do_not_change_output() {
    for entries in [
        [&CPU[..], &CPU[..], &IO[..]].concat(),
        [&CPU[..], &IO[..], &IO[..]].concat(),
        [&CPU[..], &IO[..], &[0x80, 0]].concat(),
        [&CPU[..], &IO[..], &[0x80, 1]].concat(),
        [&CPU[..], &IO[..], &[0x80, 12, 0]].concat(),
        [&CPU[..], &IO[..], &[0]].concat(),
    ] {
        let mut p = platform();
        let previous = p;
        assert!(parse_madt(&madt(&entries), &mut p).is_err());
        assert_eq!(p, previous);
    }
    let mut many = Vec::new();
    for uid in 0..65 {
        many.extend_from_slice(&[0, 8, uid, uid, 1, 0, 0, 0]);
    }
    many.extend_from_slice(&IO);
    assert_eq!(
        parse_madt(&madt(&many), &mut Platform::empty()),
        Err(Error::LimitExceeded)
    );
}
#[test]
fn lapic_override_and_x2apic_ids_obey_xapic_profile() {
    let override_entry = [5, 12, 0, 0, 0, 0, 0xe0, 0xfe, 0, 0, 0, 0];
    let mut p = Platform::empty();
    parse_madt(
        &madt(&[&CPU[..], &IO[..], &override_entry[..]].concat()),
        &mut p,
    )
    .unwrap();
    assert_eq!(p.lapic_physical, 0xfee00000);
    assert!(
        parse_madt(
            &madt(&[&CPU[..], &IO[..], &override_entry[..], &override_entry[..]].concat()),
            &mut p
        )
        .is_err()
    );
    let x2 = [9, 16, 0, 0, 7, 0, 0, 0, 1, 0, 0, 0, 9, 0, 0, 0];
    parse_madt(&madt(&[&x2[..], &IO[..]].concat()), &mut p).unwrap();
    assert_eq!(
        p.processors[0],
        Processor {
            acpi_uid: 9,
            apic_id: 7
        }
    );
    let mut bad = x2;
    bad[5] = 1;
    assert!(parse_madt(&madt(&[&bad[..], &IO[..]].concat()), &mut p).is_err());
}
#[test]
fn fadt_timer_width_extended_address_and_hardware_reduced_are_explicit() {
    let mut p = Platform::empty();
    let mut b = fadt();
    parse_fadt(&b, &mut p).unwrap();
    assert_eq!((p.pm_timer_port, p.pm_timer_bits), (0x608, 24));
    assert!(p.legacy_devices);
    b[113] |= 1;
    b[208] = 1;
    b[209] = 32;
    b[211] = 3;
    b[212..220].copy_from_slice(&0x408u64.to_le_bytes());
    sum(&mut b, 9);
    parse_fadt(&b, &mut p).unwrap();
    assert_eq!((p.pm_timer_port, p.pm_timer_bits), (0x408, 32));
    b[208] = 0;
    sum(&mut b, 9);
    parse_fadt(&b, &mut p).unwrap();
    assert_eq!(p.pm_timer_port, 0x608);
    b[114] |= 0x10;
    sum(&mut b, 9);
    parse_fadt(&b, &mut p).unwrap();
    assert_eq!((p.pm_timer_port, p.pm_timer_bits), (0, 0));
    let mut bad = fadt();
    bad[91] = 3;
    sum(&mut bad, 9);
    assert!(parse_fadt(&bad, &mut p).is_err());
    assert!(parse_fadt(&table(b"FACP", 116, 3), &mut p).is_err());
    for length in 0..244 {
        assert!(parse_fadt(&b[..length], &mut p).is_err());
    }
}
#[test]
fn platform_wire_and_final_memory_reservations_round_trip() {
    let p = platform();
    let mut encoded = [0; PLATFORM_BYTES];
    p.encode(&mut encoded).unwrap();
    assert_eq!(Platform::decode(&encoded), Ok(p));
    let blob = boot_blob(&p);
    assert_eq!(BootInfo::parse(&blob).unwrap().validate_platform(), Ok(p));
    let mut bad = blob.clone();
    let region = 128 + PLATFORM_BYTES;
    bad[region + 28..region + 32].copy_from_slice(&3u32.to_le_bytes());
    assert!(BootInfo::parse(&bad).unwrap().validate_platform().is_err());
    let mut bad = p;
    bad.lapic_physical = 0x100000;
    assert!(
        BootInfo::parse(&boot_blob(&bad))
            .unwrap()
            .validate_platform()
            .is_err()
    );
    for length in 0..PLATFORM_BYTES {
        assert!(Platform::decode(&encoded[..length]).is_err());
    }
}
#[test]
fn platform_reserved_duplicate_and_overflow_fields_reject_without_encoder_write() {
    let p = platform();
    for mutant in 0..7 {
        let mut p = p;
        match mutant {
            0 => p.processor_count = 65,
            1 => p.io_apic_count = 9,
            2 => p.roots[0] = u64::MAX - 10,
            3 => {
                p.processor_count = 2;
                p.processors[1] = p.processors[0];
            }
            4 => p.io_apics[0].physical = p.lapic_physical,
            5 => p.pm_timer_bits = 16,
            _ => p.processors[63].apic_id = 10,
        }
        let mut out = [0xa5; PLATFORM_BYTES];
        assert!(p.encode(&mut out).is_err());
        assert_eq!(out, [0xa5; PLATFORM_BYTES]);
    }
    let mut b = [0; PLATFORM_BYTES];
    p.encode(&mut b).unwrap();
    for index in [59, 67, 79, 720, 1023] {
        let mut bad = b;
        bad[index] = 0xff;
        assert!(Platform::decode(&bad).is_err());
    }
}

#[test]
fn firmware_profile_survives_revision_two_handoff_and_revision_one_stays_strict() {
    for profile in [0, 1, 2, 8, 255] {
        let mut f = fadt();
        f[45] = profile;
        sum(&mut f, 9);
        let mut p = platform();
        parse_fadt(&f, &mut p).unwrap();
        assert_eq!(p.power_profile, if profile <= 8 { profile } else { 0 });
        let mut blob = boot_blob(&p);
        blob[68..70].copy_from_slice(&2u16.to_le_bytes());
        let parsed = BootInfo::parse(&blob).unwrap();
        assert!(parsed.table(PLATFORM_KIND).is_some());
        assert_eq!(parsed.validate_platform(), Ok(p));
        blob[68..70].copy_from_slice(&1u16.to_le_bytes());
        if p.power_profile == 0 {
            assert_eq!(BootInfo::parse(&blob).unwrap().validate_platform(), Ok(p));
        } else {
            assert!(BootInfo::parse(&blob).is_err());
        }
    }
}
