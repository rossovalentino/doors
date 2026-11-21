use door_boot_protocol::*;

const CODE: u64 = 0x7000;
const DATA: u64 = 0x8000;
const SELF: u64 = 0x100000;

fn startup() -> Startup {
    Startup {
        code_physical: CODE,
        data_physical: DATA,
    }
}
fn mappings() -> [Mapping; 2] {
    [(CODE, STARTUP_CODE_ROLE), (DATA, STARTUP_DATA_ROLE)].map(|(physical, role)| Mapping {
        virtual_start: physical,
        physical,
        bytes: 4096,
        permissions: 6,
        role,
    })
}
fn fixture(mappings: &[Mapping]) -> Vec<u8> {
    let maps = 160 + STARTUP_BYTES;
    let memory = maps + mappings.len() * MAPPING_BYTES;
    let total = memory + 80;
    let mut b = vec![0; total];
    Header {
        total_bytes: total as u64,
        table_count: 3,
        tables_offset: 64,
        self_phys: SELF,
    }
    .encode(&mut b)
    .unwrap();
    for (i, (kind, offset, length, entry)) in [
        (STARTUP_KIND, 160, STARTUP_BYTES, STARTUP_BYTES),
        (
            MAPPINGS_KIND,
            maps,
            mappings.len() * MAPPING_BYTES,
            MAPPING_BYTES,
        ),
        (MEMORY_REGIONS_KIND, memory, 80, MEMORY_REGION_BYTES),
    ]
    .into_iter()
    .enumerate()
    {
        TableDescriptor {
            kind,
            revision: 1,
            flags: REQUIRED,
            offset: offset as u64,
            length: length as u64,
            entry_bytes: entry as u32,
        }
        .encode(&mut b[64 + i * 32..], total as u64)
        .unwrap();
    }
    startup().encode(&mut b[160..]).unwrap();
    for (index, map) in mappings.iter().enumerate() {
        map.encode(&mut b[maps + index * MAPPING_BYTES..]).unwrap();
    }
    for (index, (physical, bytes)) in [(CODE, 8192), (SELF, 4096)].into_iter().enumerate() {
        MemoryRegion {
            range: PhysicalRange::new(physical, bytes, 4096).unwrap(),
            firmware_attributes: 0,
            firmware_type: 2,
            ownership: Ownership::BootData,
        }
        .encode(&mut b[memory + index * MEMORY_REGION_BYTES..])
        .unwrap();
    }
    b
}

#[test]
fn reserved_identity_pages_round_trip_and_have_an_eight_bit_sipi_vector() {
    let b = fixture(&mappings());
    let info = BootInfo::parse(&b).unwrap();
    assert_eq!(info.validate_startup(), Ok(startup()));
    assert!(!info.is_allocatable(PhysicalRange::new(CODE, 8192, 4096).unwrap()));
    for code in [0x1000, 0xfe000] {
        let value = Startup {
            code_physical: code,
            data_physical: code + 4096,
        };
        let mut out = [0; STARTUP_BYTES];
        value.encode(&mut out).unwrap();
        assert_eq!(Startup::decode(&out), Ok(value));
        assert!(value.code_physical >> 12 <= u64::from(u8::MAX));
    }
}

#[test]
fn startup_wire_rejects_truncation_extents_identity_and_reserved_fields() {
    let mut valid = [0; STARTUP_BYTES];
    startup().encode(&mut valid).unwrap();
    for length in 0..STARTUP_BYTES {
        assert!(Startup::decode(&valid[..length]).is_err());
    }
    for offset in [16, 24, 32, 40, 48, 56, 63] {
        let mut invalid = valid;
        invalid[offset] ^= 1;
        assert!(
            Startup::decode(&invalid).is_err(),
            "accepted malformed offset {offset}"
        );
    }
    for value in [
        Startup {
            code_physical: 0,
            data_physical: 4096,
        },
        Startup {
            code_physical: 0x7001,
            data_physical: 0x8001,
        },
        Startup {
            code_physical: CODE,
            data_physical: CODE,
        },
        Startup {
            code_physical: CODE,
            data_physical: DATA + 4096,
        },
        Startup {
            code_physical: 0xff000,
            data_physical: 0x100000,
        },
        Startup {
            code_physical: 0x100000,
            data_physical: 0x101000,
        },
        Startup {
            code_physical: u64::MAX - 4095,
            data_physical: 0,
        },
    ] {
        let mut out = [0xa5; STARTUP_BYTES];
        assert!(value.encode(&mut out).is_err());
        assert_eq!(out, [0xa5; STARTUP_BYTES]);
    }
    let mut out = [0xa5; STARTUP_BYTES - 1];
    assert!(startup().encode(&mut out).is_err());
    assert_eq!(out, [0xa5; STARTUP_BYTES - 1]);
}

#[test]
fn startup_requires_complete_loader_data_reservation_and_no_bootinfo_overlap() {
    let valid = fixture(&mappings());
    let memory = valid.len() - 80;
    for (ty, ownership) in [
        (7u32, Ownership::UsableAfterHandoff),
        (3, Ownership::ReclaimableAfterCopy),
        (2, Ownership::PermanentlyReserved),
        (5, Ownership::FirmwareRuntime),
    ] {
        let mut b = valid.clone();
        b[memory + 24..memory + 28].copy_from_slice(&ty.to_le_bytes());
        b[memory + 28..memory + 32].copy_from_slice(&(ownership as u32).to_le_bytes());
        assert!(BootInfo::parse(&b).unwrap().validate_startup().is_err());
    }
    let mut b = valid.clone();
    b[memory + 8..memory + 16].copy_from_slice(&4096u64.to_le_bytes());
    assert!(BootInfo::parse(&b).unwrap().validate_startup().is_err());
    let mut b = valid;
    b[48..56].copy_from_slice(&CODE.to_le_bytes());
    assert_eq!(
        BootInfo::parse(&b).unwrap().validate_startup(),
        Err(Error::Overlap)
    );
}

#[test]
fn startup_rejects_missing_duplicate_oversized_or_executable_initial_mappings() {
    let mut cases = Vec::new();
    cases.push(vec![mappings()[0]]);
    cases.push(vec![mappings()[0], mappings()[1], mappings()[0]]);
    for (index, value) in [(0usize, 5u32), (1, 4)] {
        let mut maps = mappings();
        maps[index].permissions = value;
        cases.push(maps.to_vec());
    }
    let mut maps = mappings();
    maps[0].bytes = 8192;
    cases.push(maps.to_vec());
    let mut maps = mappings();
    maps[1].physical += 4096;
    cases.push(maps.to_vec());
    let mut maps = mappings();
    maps[0].virtual_start = 0xffff_ffff_c000_0000;
    cases.push(maps.to_vec());
    for maps in cases {
        assert!(
            BootInfo::parse(&fixture(&maps))
                .unwrap()
                .validate_startup()
                .is_err()
        );
    }
}

#[test]
fn startup_rejects_physical_and_virtual_aliases_regardless_of_permissions() {
    for (virtual_start, physical, permissions) in [
        (0xffff_ffff_c000_0000, CODE, 4),
        (0xffff_ffff_c000_0000, DATA, 6),
        (CODE, 0x200000, 5),
        (DATA, 0x200000, 4),
    ] {
        let mut maps = mappings().to_vec();
        maps.push(Mapping {
            virtual_start,
            physical,
            bytes: 4096,
            permissions,
            role: 1,
        });
        assert_eq!(
            BootInfo::parse(&fixture(&maps)).unwrap().validate_startup(),
            Err(Error::Overlap)
        );
    }
}

#[test]
fn startup_descriptor_revision_and_record_size_are_enforced_by_envelope() {
    let valid = fixture(&mappings());
    let mut b = valid.clone();
    b[68..70].copy_from_slice(&2u16.to_le_bytes());
    assert!(matches!(
        BootInfo::parse(&b),
        Err(Error::UnsupportedRequiredTable)
    ));
    let mut b = valid.clone();
    b[88..92].copy_from_slice(&32u32.to_le_bytes());
    assert!(matches!(BootInfo::parse(&b), Err(Error::InvalidRecordSize)));
    let mut b = valid;
    b[64..68].copy_from_slice(&0x80u32.to_le_bytes());
    assert!(matches!(
        BootInfo::parse(&b),
        Err(Error::UnsupportedRequiredTable)
    ));
}
