use door_image_format::{ElfImage, Error, PeImage};
#[path = "../../../tests/support/images.rs"]
mod fixtures;
use fixtures::{elf, pe, put16, put32, put64};
#[test]
fn valid_formats_expose_real_layout() {
    let e = elf();
    let image = ElfImage::parse(&e).unwrap();
    assert_eq!(image.segment(0).unwrap().memory_bytes, 4096);
    assert_eq!(image.segment(usize::MAX), Err(Error::Bounds));
    let p = pe();
    let image = PeImage::parse(&p).unwrap();
    assert_eq!(image.subsystem, 10);
    assert_eq!(image.import_count().unwrap(), 0);
    assert_eq!(image.export_count().unwrap(), 0);
    assert_eq!(image.section(usize::MAX), Err(Error::Bounds));
    assert_eq!(image.rva_bytes(4096, 1).unwrap(), &[0]);
    assert_eq!(image.file_offset(4096, 1), Ok(512));
    assert_eq!(image.file_offset(0, 64), Ok(0));
    assert_eq!(image.headers_bytes(), 512);
    assert_eq!(image.file_offset(4095, 2), Err(Error::Bounds));
    assert_eq!(image.file_offset(u32::MAX, 8), Err(Error::Bounds));
}
#[test]
fn truncated_and_mutated_records_never_panic() {
    for b in [elf(), pe()] {
        for end in 0..b.len().min(1024) {
            let _ = ElfImage::parse(&b[..end]);
            let _ = PeImage::parse(&b[..end]);
        }
        for index in 0..512 {
            for value in [0, 1, 128, 255] {
                let mut m = b.clone();
                m[index] = value;
                let _ = ElfImage::parse(&m);
                let _ = PeImage::parse(&m);
            }
        }
    }
}
#[test]
fn elf_rejects_dynamic_wrong_machine_and_writable_code() {
    let mut b = elf();
    put32(&mut b, 64, 3);
    assert_eq!(ElfImage::parse(&b).unwrap_err(), Error::DynamicDependency);
    let mut b = elf();
    put16(&mut b, 18, 3);
    assert_eq!(ElfImage::parse(&b).unwrap_err(), Error::Unsupported);
    let mut b = elf();
    put32(&mut b, 68, 7);
    assert_eq!(ElfImage::parse(&b).unwrap_err(), Error::Permissions);
}
#[test]
fn elf_rejects_wrapping_mappings_bounds_and_entry() {
    let mut b = elf();
    put64(&mut b, 104, u64::MAX);
    assert_eq!(ElfImage::parse(&b).unwrap_err(), Error::Overflow);
    let mut b = elf();
    put64(&mut b, 80, 0x0000_8000_0000_0000);
    assert_eq!(ElfImage::parse(&b).unwrap_err(), Error::Bounds);
    let mut b = elf();
    put64(&mut b, 24, 0x1000);
    assert_eq!(ElfImage::parse(&b).unwrap_err(), Error::InvalidEntry);
    let mut b = elf();
    put64(&mut b, 96, 8192);
    assert_eq!(ElfImage::parse(&b).unwrap_err(), Error::Bounds);
}
#[test]
fn elf_rejects_overlaps_and_bad_alignment() {
    let mut b = elf();
    put16(&mut b, 56, 2);
    let first = b[64..120].to_vec();
    b[120..176].copy_from_slice(&first);
    assert_eq!(ElfImage::parse(&b).unwrap_err(), Error::Overlap);
    let mut b = elf();
    put64(&mut b, 112, 3);
    assert_eq!(ElfImage::parse(&b).unwrap_err(), Error::Alignment);
}
#[test]
fn pe_rejects_bad_headers_machine_entry_and_permissions() {
    let mut b = pe();
    put32(&mut b, 60, u32::MAX);
    assert!(PeImage::parse(&b).is_err());
    let mut b = pe();
    put16(&mut b, 68, 0x14c);
    assert_eq!(PeImage::parse(&b).unwrap_err(), Error::Unsupported);
    let mut b = pe();
    put32(&mut b, 104, 0);
    assert_eq!(PeImage::parse(&b).unwrap_err(), Error::InvalidEntry);
    let mut b = pe();
    put32(&mut b, 364, 0xe000_0020);
    assert_eq!(PeImage::parse(&b).unwrap_err(), Error::Permissions);
}
#[test]
fn pe_rejects_raw_and_virtual_bounds_overlaps_and_delay_imports() {
    let mut b = pe();
    put32(&mut b, 344, 1024);
    assert!(PeImage::parse(&b).is_err());
    let mut b = pe();
    put32(&mut b, 336, u32::MAX);
    assert!(PeImage::parse(&b).is_err());
    let mut b = pe();
    put16(&mut b, 70, 2);
    let first = b[328..368].to_vec();
    b[368..408].copy_from_slice(&first);
    assert_eq!(PeImage::parse(&b).unwrap_err(), Error::Overlap);
    let mut b = pe();
    put32(&mut b, 200 + 13 * 8, 4096);
    put32(&mut b, 204 + 13 * 8, 4);
    assert_eq!(PeImage::parse(&b).unwrap_err(), Error::Unsupported);
}
#[test]
fn pe_imports_are_bounded_and_names_validated() {
    let mut b = pe();
    put32(&mut b, 208, 4096);
    put32(&mut b, 212, 40);
    put32(&mut b, 512 + 12, 4160);
    b[576..589].copy_from_slice(b"kernel32.dll\0");
    let image = PeImage::parse(&b).unwrap();
    assert_eq!(image.import_count().unwrap(), 1);
    assert_eq!(image.import_name(0).unwrap(), "kernel32.dll");
    put32(&mut b, 524, u32::MAX);
    assert!(PeImage::parse(&b).is_err());
}

#[test]
fn elf_rejects_tls_relocations_and_entry_into_zero_fill() {
    let mut b = elf();
    put32(&mut b, 64, 7);
    assert_eq!(ElfImage::parse(&b).unwrap_err(), Error::Unsupported);
    let mut b = elf();
    put64(&mut b, 24, 0xffff_ffff_8000_0800);
    assert_eq!(ElfImage::parse(&b).unwrap_err(), Error::InvalidEntry);
    let mut b = elf();
    put64(&mut b, 40, 512);
    put16(&mut b, 58, 64);
    put16(&mut b, 60, 1);
    put32(&mut b, 516, 4);
    put64(&mut b, 536, 1024);
    put64(&mut b, 544, 24);
    assert_eq!(ElfImage::parse(&b).unwrap_err(), Error::Unsupported);
}

#[test]
fn elf_metadata_file_ranges_are_checked_even_when_not_loaded() {
    let mut b = elf();
    put16(&mut b, 56, 2);
    put32(&mut b, 120, 4);
    put64(&mut b, 128, u64::MAX);
    put64(&mut b, 152, 4096);
    assert_eq!(ElfImage::parse(&b).unwrap_err(), Error::Overflow);
    put64(&mut b, 128, 8192);
    assert_eq!(ElfImage::parse(&b).unwrap_err(), Error::Bounds);
    put64(&mut b, 128, 4096);
    assert!(ElfImage::parse(&b).is_ok());
}
#[test]
fn delay_import_directory_is_left_only_to_an_explicit_validating_loader() {
    let mut b = pe();
    put32(&mut b, 200 + 13 * 8, 4096);
    put32(&mut b, 204 + 13 * 8, 32);
    assert_eq!(PeImage::parse(&b).unwrap_err(), Error::Unsupported);
    let image = PeImage::parse_with_delay_imports(&b).unwrap();
    assert_eq!(image.directory(13).unwrap(), (4096, 32));
    // The extent itself remains a checked file-backed RVA range.
    put32(&mut b, 204 + 13 * 8, u32::MAX);
    assert!(PeImage::parse_with_delay_imports(&b).is_err());
}
#[test]
fn pe32_images_parse_only_where_x86_is_asked_for() {
    use door_image_format::Machine;
    let b = fixtures::pe32();
    // The x64 loaders' entry points keep refusing 32-bit images.
    assert_eq!(PeImage::parse(&b).unwrap_err(), Error::Unsupported);
    assert_eq!(
        PeImage::parse_with_delay_imports(&b).unwrap_err(),
        Error::Unsupported
    );
    let image = PeImage::parse_any(&b).unwrap();
    assert_eq!(image.machine, Machine::X86);
    assert_eq!(image.machine.pointer_bytes(), 4);
    assert_eq!(image.image_base, 0x40_0000);
    assert_eq!(image.subsystem, 3);
    assert_eq!(image.entry_rva, 4096);
    assert_eq!(image.directory(0), Ok((0, 0)));
    assert_eq!(
        PeImage::parse_any(&fixtures::pe()).unwrap().machine,
        Machine::X64
    );
    // A PE32+ optional header on an x86 machine, and the reverse.
    let mut m = b.clone();
    put16(&mut m, 88, 0x20b);
    assert_eq!(PeImage::parse_any(&m).unwrap_err(), Error::Unsupported);
    let mut m = fixtures::pe();
    put16(&mut m, 88, 0x10b);
    assert_eq!(PeImage::parse_any(&m).unwrap_err(), Error::Unsupported);
    // Directory count past the PE32 header, and the import directory read
    // at the PE32 offset (an import table with an unterminated name).
    let mut m = b.clone();
    put32(&mut m, 180, 17);
    assert_eq!(PeImage::parse_any(&m).unwrap_err(), Error::Limit);
    let mut m = b.clone();
    put32(&mut m, 192, 4096);
    put32(&mut m, 196, 40);
    put32(&mut m, 512 + 12, 4096 + 100);
    for byte in &mut m[612..1024] {
        *byte = b'a';
    }
    assert!(PeImage::parse_any(&m).is_err());
    for end in 0..b.len() {
        let _ = PeImage::parse_any(&b[..end]);
    }
}
