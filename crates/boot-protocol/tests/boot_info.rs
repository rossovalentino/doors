use door_boot_protocol::{
    BootInfo, Error, HEADER_BYTES, Header, MAX_BLOB_BYTES, MAX_MEMORY_REGIONS, MAX_TABLES,
    MEMORY_REGION_BYTES, MEMORY_REGIONS_KIND, MemoryRegion, Ownership, PAGE_BYTES, PhysicalRange,
    REQUIRED, TABLE_BYTES, TableDescriptor,
};

fn region(start: u64, length: u64, ownership: Ownership) -> MemoryRegion {
    MemoryRegion {
        range: PhysicalRange::new(start, length, PAGE_BYTES).unwrap(),
        firmware_attributes: 0,
        firmware_type: if ownership == Ownership::UsableAfterHandoff {
            7
        } else {
            2
        },
        ownership,
    }
}

fn fixture() -> Vec<u8> {
    let mut blob = vec![0; HEADER_BYTES + TABLE_BYTES + 2 * MEMORY_REGION_BYTES];
    let total = blob.len() as u64;
    Header {
        total_bytes: total,
        table_count: 1,
        tables_offset: 64,
        self_phys: 0x1000,
    }
    .encode(&mut blob)
    .unwrap();
    TableDescriptor {
        kind: MEMORY_REGIONS_KIND,
        revision: 1,
        flags: REQUIRED,
        offset: 96,
        length: 80,
        entry_bytes: 40,
    }
    .encode(&mut blob[64..], total)
    .unwrap();
    region(0x1000, PAGE_BYTES, Ownership::BootData)
        .encode(&mut blob[96..])
        .unwrap();
    region(0x2000, 2 * PAGE_BYTES, Ownership::UsableAfterHandoff)
        .encode(&mut blob[136..])
        .unwrap();
    blob
}

fn fails(blob: &[u8], expected: Error) {
    assert_eq!(BootInfo::parse(blob).unwrap_err(), expected);
}

#[test]
fn roundtrip_and_allocation_respect_live_ownership() {
    let blob = fixture();
    // Literal known bytes independently check the wire format and endianness.
    assert_eq!(&blob[..16], b"DOORBOOT\x01\x00\x00\x00\x40\x00\x00\x00");
    assert_eq!(&blob[64..72], &[1, 0, 0, 0, 1, 0, 1, 0]);
    assert_eq!(&blob[96..104], &[0, 16, 0, 0, 0, 0, 0, 0]);
    let info = BootInfo::parse(&blob).unwrap();
    assert_eq!(info.header().total_bytes, 176);
    assert_eq!(info.memory_region_count(), 2);
    assert_eq!(
        info.memory_region(0).unwrap(),
        region(0x1000, PAGE_BYTES, Ownership::BootData)
    );
    assert_eq!(info.memory_region(2), Err(Error::OutOfBounds));
    assert_eq!(info.memory_region(usize::MAX), Err(Error::OutOfBounds));
    assert!(!info.is_allocatable(PhysicalRange::new(0x1000, PAGE_BYTES, PAGE_BYTES).unwrap()));
    assert!(info.is_allocatable(PhysicalRange::new(0x2000, PAGE_BYTES, PAGE_BYTES).unwrap()));
    assert!(!info.is_allocatable(PhysicalRange::new(0x3000, 2 * PAGE_BYTES, PAGE_BYTES).unwrap()));
}

#[test]
fn every_truncated_prefix_is_rejected_without_panic() {
    let blob = fixture();
    for end in 0..blob.len() {
        assert!(BootInfo::parse(&blob[..end]).is_err(), "prefix {end}");
    }
    assert_eq!(Header::decode(&blob[..63]), Err(Error::ShortRecord));
    assert_eq!(
        TableDescriptor::decode(&blob[64..95]),
        Err(Error::ShortRecord)
    );
    assert_eq!(
        MemoryRegion::decode(&blob[96..135]),
        Err(Error::ShortRecord)
    );
}

#[test]
fn wrong_magic_endianness_and_versions_are_rejected() {
    let mut blob = fixture();
    blob[0] = b'X';
    fails(&blob, Error::BadMagic);
    blob = fixture();
    blob[8..10].copy_from_slice(&1_u16.to_be_bytes());
    fails(&blob, Error::UnsupportedVersion);
    blob = fixture();
    blob[10] = 1;
    fails(&blob, Error::UnsupportedVersion);
}

#[test]
fn reserved_fields_and_unknown_flags_are_rejected() {
    for offset in [24, 56, 70, 92, 128] {
        let mut blob = fixture();
        blob[offset] |= if offset == 70 { 2 } else { 1 };
        fails(&blob, Error::ReservedBits);
    }
}

#[test]
fn header_and_descriptor_sizes_are_checked() {
    for offset in [12, 36, 88] {
        let mut blob = fixture();
        blob[offset] = 0;
        fails(&blob, Error::InvalidRecordSize);
    }
    let mut blob = fixture();
    blob[72..80].copy_from_slice(&97_u64.to_le_bytes());
    fails(&blob, Error::Misaligned);
    let mut blob = fixture();
    blob[80..88].copy_from_slice(&0_u64.to_le_bytes());
    fails(&blob, Error::EmptyRange);
    let mut blob = fixture();
    blob[80..88].copy_from_slice(&79_u64.to_le_bytes());
    fails(&blob, Error::InvalidRecordSize);
}

#[test]
fn bounds_limits_and_overflow_are_checked() {
    let mut blob = fixture();
    blob[16..24].copy_from_slice(&((MAX_BLOB_BYTES + 1) as u64).to_le_bytes());
    fails(&blob, Error::LimitExceeded);
    let mut blob = fixture();
    blob[32..36].copy_from_slice(&(MAX_TABLES + 1).to_le_bytes());
    fails(&blob, Error::LimitExceeded);
    let mut blob = fixture();
    blob[40..48].copy_from_slice(&184_u64.to_le_bytes());
    fails(&blob, Error::OutOfBounds);
    let mut blob = fixture();
    blob[72..80].copy_from_slice(&(u64::MAX - 7).to_le_bytes());
    fails(&blob, Error::Overflow);
    let mut blob = fixture();
    blob[48..56].copy_from_slice(&(u64::MAX - 7).to_le_bytes());
    fails(&blob, Error::Overflow);
    let mut blob = fixture();
    blob[144..152].copy_from_slice(&0_u64.to_le_bytes());
    fails(&blob, Error::EmptyRange);
    let mut blob = fixture();
    blob[136..144].copy_from_slice(&(u64::MAX - 4095).to_le_bytes());
    fails(&blob, Error::Overflow);
}

#[test]
fn memory_alignment_order_and_overlap_are_checked() {
    let mut blob = fixture();
    blob[136..144].copy_from_slice(&0x2001_u64.to_le_bytes());
    fails(&blob, Error::Misaligned);
    let mut blob = fixture();
    blob[136..144].copy_from_slice(&0x1000_u64.to_le_bytes());
    fails(&blob, Error::Overlap);
    let mut blob = fixture();
    blob[136..144].copy_from_slice(&0_u64.to_le_bytes());
    fails(&blob, Error::UnsortedMemoryMap);
}

#[test]
fn table_payload_cannot_overlap_header_or_descriptors() {
    for offset in [0_u64, 64] {
        let mut blob = fixture();
        blob[72..80].copy_from_slice(&offset.to_le_bytes());
        fails(&blob, Error::Overlap);
    }
}

fn with_extra_table(kind: u32, required: bool) -> Vec<u8> {
    let old = fixture();
    let mut blob = vec![0; old.len() + TABLE_BYTES + 8];
    let total = blob.len() as u64;
    Header {
        total_bytes: total,
        table_count: 2,
        tables_offset: 64,
        self_phys: 0x1000,
    }
    .encode(&mut blob)
    .unwrap();
    TableDescriptor {
        kind: 1,
        revision: 1,
        flags: REQUIRED,
        offset: 128,
        length: 80,
        entry_bytes: 40,
    }
    .encode(&mut blob[64..], total)
    .unwrap();
    TableDescriptor {
        kind,
        revision: 77,
        flags: if required { REQUIRED } else { 0 },
        offset: 208,
        length: 8,
        entry_bytes: 1,
    }
    .encode(&mut blob[96..], total)
    .unwrap();
    blob[128..208].copy_from_slice(&old[96..176]);
    blob
}

#[test]
fn unknown_optional_tables_are_skipped_only_after_validation() {
    let blob = with_extra_table(99, false);
    assert!(BootInfo::parse(&blob).is_ok());
    fails(&with_extra_table(99, true), Error::UnsupportedRequiredTable);
    let mut blob = blob;
    blob[104..112].copy_from_slice(&128_u64.to_le_bytes());
    fails(&blob, Error::Overlap);
}

#[test]
fn duplicate_kinds_unsupported_revision_and_missing_map_are_rejected() {
    fails(&with_extra_table(1, false), Error::DuplicateTable);
    let mut blob = fixture();
    blob[68] = 2;
    fails(&blob, Error::UnsupportedRequiredTable);
    let mut blob = fixture();
    blob[64] = 99;
    blob[70] = 0;
    fails(&blob, Error::MissingMemoryMap);
}

#[test]
fn live_boot_data_must_be_fully_reserved() {
    for ownership in [
        Ownership::UsableAfterHandoff,
        Ownership::ReclaimableAfterCopy,
        Ownership::FramebufferMmio,
        Ownership::FirmwareRuntime,
    ] {
        let mut blob = fixture();
        let mut entry = region(0x1000, PAGE_BYTES, ownership);
        if ownership == Ownership::FirmwareRuntime {
            entry.firmware_type = 5;
        }
        entry.encode(&mut blob[96..]).unwrap();
        fails(&blob, Error::LiveBootDataNotReserved);
    }
    let mut blob = fixture();
    blob[48..56].copy_from_slice(&0x1ff8_u64.to_le_bytes());
    fails(&blob, Error::LiveBootDataNotReserved);
    let mut blob = fixture();
    blob[124..128].copy_from_slice(&99_u32.to_le_bytes());
    fails(&blob, Error::InvalidOwnership);
}

#[test]
fn runtime_and_reserved_firmware_cannot_be_marked_free() {
    for firmware_type in [0_u32, 5, 6, 11, 12, 99] {
        let mut blob = fixture();
        blob[160..164].copy_from_slice(&firmware_type.to_le_bytes());
        assert!(BootInfo::parse(&blob).is_err());
    }
    let mut blob = fixture();
    blob[152..160].copy_from_slice(&(1_u64 << 63).to_le_bytes());
    fails(&blob, Error::RuntimeMemoryOwnership);
}

#[test]
fn range_boundaries_are_half_open_and_checked() {
    assert_eq!(PhysicalRange::new(0, 0, 1), Err(Error::EmptyRange));
    assert_eq!(PhysicalRange::new(u64::MAX, 1, 1), Err(Error::Overflow));
    for alignment in [0, 3] {
        assert_eq!(PhysicalRange::new(0, 8, alignment), Err(Error::Misaligned));
    }
    let left = PhysicalRange::new(0, 4096, PAGE_BYTES).unwrap();
    let right = PhysicalRange::new(4096, 4096, PAGE_BYTES).unwrap();
    assert!(!left.overlaps(right));
    assert!(!left.contains(right));
    assert!(left.contains(left));
}

#[test]
fn encoders_reject_short_buffers_without_writes() {
    let blob = fixture();
    let header = Header::decode(&blob).unwrap();
    let descriptor = TableDescriptor::decode(&blob[64..]).unwrap();
    let entry = MemoryRegion::decode(&blob[96..]).unwrap();
    let mut short = [0xaa; 31];
    assert_eq!(header.encode(&mut short), Err(Error::ShortRecord));
    assert_eq!(descriptor.encode(&mut short, 176), Err(Error::ShortRecord));
    assert_eq!(entry.encode(&mut short), Err(Error::ShortRecord));
    assert_eq!(short, [0xaa; 31]);
}

#[test]
fn excessive_memory_region_count_is_rejected() {
    let length = (MAX_MEMORY_REGIONS + 1) * MEMORY_REGION_BYTES;
    let mut blob = vec![0; 96 + length];
    let total = blob.len() as u64;
    Header {
        total_bytes: total,
        table_count: 1,
        tables_offset: 64,
        self_phys: 0x1000,
    }
    .encode(&mut blob)
    .unwrap();
    TableDescriptor {
        kind: 1,
        revision: 1,
        flags: REQUIRED,
        offset: 96,
        length: length as u64,
        entry_bytes: 40,
    }
    .encode(&mut blob[64..], total)
    .unwrap();
    fails(&blob, Error::LimitExceeded);
}

#[test]
fn mutations_and_unaligned_host_slices_never_panic() {
    let blob = fixture();
    let mut prefixed = vec![0xff];
    prefixed.extend_from_slice(&blob);
    assert!(BootInfo::parse(&prefixed[1..]).is_ok());
    for index in 0..blob.len() {
        for value in [0, 1, 0x80, 0xff] {
            let mut mutated = blob.clone();
            mutated[index] = value;
            let _ = BootInfo::parse(&mutated);
        }
    }
}
