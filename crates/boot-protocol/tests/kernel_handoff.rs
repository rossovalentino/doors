use door_boot_protocol::*;
fn fixture() -> Vec<u8> {
    let maps = [
        Mapping {
            virtual_start: 0xffff_ffff_8000_0000,
            physical: 0x120000,
            bytes: 4096,
            permissions: 5,
            role: 1,
        },
        Mapping {
            virtual_start: 0x130000,
            physical: 0x130000,
            bytes: 4096,
            permissions: 4,
            role: 2,
        },
        Mapping {
            virtual_start: 0x100000,
            physical: 0x100000,
            bytes: 4096,
            permissions: 4,
            role: 3,
        },
        Mapping {
            virtual_start: 0xffff_ffff_9000_1000,
            physical: 0x140000,
            bytes: 128 * 1024,
            permissions: 6,
            role: 4,
        },
        Mapping {
            virtual_start: 0x170000,
            physical: 0x170000,
            bytes: 64 * 4096,
            permissions: 6,
            role: 5,
        },
        Mapping {
            virtual_start: 0xc0000000,
            physical: 0xc0000000,
            bytes: 4096,
            permissions: 6,
            role: 7,
        },
    ];
    let modules = [
        Module {
            role: 1,
            physical: 0x110000,
            storage_bytes: 4096,
            file_bytes: 2048,
            virtual_start: 0xffff_ffff_8000_0000,
            virtual_bytes: 4096,
            sha256: [1; 32],
        },
        Module {
            role: 2,
            physical: 0x130000,
            storage_bytes: 4096,
            file_bytes: 2048,
            virtual_start: 0x130000,
            virtual_bytes: 4096,
            sha256: [2; 32],
        },
    ];
    let mut bytes = vec![0; 712];
    Header {
        total_bytes: 712,
        table_count: 4,
        tables_offset: 64,
        self_phys: 0x100000,
    }
    .encode(&mut bytes)
    .unwrap();
    for (i, (kind, offset, length, entry_bytes)) in [
        (1, 672, 40, 40),
        (2, 192, 160, 80),
        (3, 352, 280, 40),
        (4, 632, 40, 40),
    ]
    .into_iter()
    .enumerate()
    {
        TableDescriptor {
            kind,
            revision: 1,
            flags: REQUIRED,
            offset,
            length,
            entry_bytes,
        }
        .encode(&mut bytes[64 + i * 32..96 + i * 32], 712)
        .unwrap();
    }
    for (i, module) in modules.into_iter().enumerate() {
        module
            .encode(&mut bytes[192 + i * 80..272 + i * 80])
            .unwrap();
    }
    for (i, map) in maps.into_iter().enumerate() {
        map.encode(&mut bytes[352 + i * 40..392 + i * 40]).unwrap();
    }
    Mapping {
        virtual_start: 0x110000,
        physical: 0x110000,
        bytes: 4096,
        permissions: 4,
        role: 8,
    }
    .encode(&mut bytes[592..632])
    .unwrap();
    Framebuffer {
        physical: 0xc0000000,
        bytes: 4096,
        width: 16,
        height: 16,
        stride: 16,
        format: 1,
        virtual_start: 0xc0000000,
    }
    .encode(&mut bytes[632..672])
    .unwrap();
    MemoryRegion {
        range: PhysicalRange::new(0x100000, 0x100000, 4096).unwrap(),
        firmware_attributes: 0,
        firmware_type: 2,
        ownership: Ownership::BootData,
    }
    .encode(&mut bytes[672..712])
    .unwrap();
    bytes
}
#[test]
fn boot_requires_complete_reserved_modules_and_mappings() {
    let bytes = fixture();
    let info = BootInfo::parse(&bytes).unwrap();
    info.validate_boot().unwrap();
    assert_eq!(info.module(2).unwrap().file_bytes, 2048);
    let mut bad = bytes.clone();
    bad[272..276].copy_from_slice(&1u32.to_le_bytes());
    assert!(BootInfo::parse(&bad).unwrap().validate_boot().is_err());
    let mut bad = bytes;
    bad[352 + 40 + 24..352 + 40 + 28].copy_from_slice(&6u32.to_le_bytes());
    assert!(BootInfo::parse(&bad).unwrap().validate_boot().is_err());
}
#[test]
fn framebuffer_stride_permissions_and_aliases_are_checked() {
    let bytes = fixture();
    let mut bad = bytes.clone();
    bad[632 + 24..632 + 28].copy_from_slice(&1u32.to_le_bytes());
    assert!(BootInfo::parse(&bad).is_err());
    let mut bad = bytes.clone();
    bad[352 + 24..352 + 28].copy_from_slice(&7u32.to_le_bytes());
    assert!(BootInfo::parse(&bad).is_err());
    let mut bad = bytes;
    bad[352 + 3 * 40 + 8..352 + 3 * 40 + 16].copy_from_slice(&0x120000u64.to_le_bytes());
    assert!(BootInfo::parse(&bad).unwrap().validate_boot().is_err());
}
#[test]
fn invalid_extension_encoding_keeps_output_unchanged() {
    let mut bytes = [0xa5; 80];
    let module = Module {
        role: 2,
        physical: 0x100000,
        storage_bytes: 4096,
        file_bytes: 5000,
        virtual_start: 0x100000,
        virtual_bytes: 4096,
        sha256: [2; 32],
    };
    assert!(module.encode(&mut bytes).is_err());
    assert_eq!(bytes, [0xa5; 80]);
    let mut bytes = [0xa5; 40];
    assert!(
        Mapping {
            virtual_start: 0,
            physical: 0,
            bytes: 4096,
            permissions: 7,
            role: 1
        }
        .encode(&mut bytes)
        .is_err()
    );
    assert_eq!(bytes, [0xa5; 40]);
    assert!(
        Framebuffer {
            physical: 0xc0000000,
            bytes: 16,
            width: 10,
            height: 10,
            stride: 10,
            format: 1,
            virtual_start: 0xc0000000
        }
        .encode(&mut bytes)
        .is_err()
    );
    assert_eq!(bytes, [0xa5; 40]);
}
